//! The queue/drain seam every network-facing bridge in this crate is built
//! on: [`TickQueue`].
//!
//! ## The constraint this module exists to satisfy
//!
//! The browser delivers network bytes on a callback (a WebRTC data channel's
//! `onmessage`, a `WebSocket`'s `onmessage`) that can fire at any point in
//! the event loop — including, from the simulation's point of view, "in the
//! middle of nowhere": between rendered frames, between fixed ticks, at a
//! moment the simulation never asked to be interrupted at. `gc_netcode`'s
//! reducers ([`crate::coordinator_bridge`]'s coordinator, and
//! [`crate::match_driver_bridge`]'s match driver, via
//! [`crate::wasm_transport`]) are synchronous and tick-quantised: every
//! state transition happens inside one `step`/`advance` call, driven by the
//! caller, never by an event arriving asynchronously mid-call.
//!
//! Bridging those two worlds wrongly — for instance, by handing JS a
//! callback pointer that Rust invokes to mutate coordinator or match-driver
//! state directly from the network callback — would let a message mutate
//! simulation state at an arbitrary moment relative to the tick loop. On a
//! rollback netcode, "arbitrary moment" is exactly the failure mode: two
//! peers whose network callbacks happened to interleave differently with
//! their tick loops would derive different states from the identical
//! message stream, and every differential test in this repository stops
//! meaning anything (see this crate's brief).
//!
//! [`TickQueue`] is the discipline that prevents it, and it is deliberately
//! almost too simple to get wrong:
//!
//! - **`enqueue` is the only thing a network callback may call.** It is a
//!   plain, ordinary, synchronous function call (not a stored callback Rust
//!   invokes later) that appends to an internal `VecDeque` and returns
//!   immediately. It never reads or touches simulation state — there is
//!   none reachable from here to touch.
//! - **`drain` is the only thing that reads the queue**, and it is called
//!   from exactly one place: the start of a tick-quantised step (the
//!   coordinator bridge's `tick()`, or — via [`crate::wasm_transport`]'s
//!   `StarTransportAdapter` implementation — a driver's `advance()`, which
//!   internally polls its transport once per driver step, exactly the same
//!   contract `gc_netcode::fake_star::FakeStarTransport`/
//!   `gc_netcode::fake_relay::FakeRelayTransport` already model for their
//!   in-process test doubles — see those modules' docs). `drain` takes
//!   everything currently queued, in arrival order, and empties the queue;
//!   it never blocks, never invokes JS, and the `Vec` it returns is an owned
//!   snapshot nothing can retroactively mutate.
//!
//! One consequence falls out for free: a message enqueued *after* a `drain`
//! call returns cannot be part of that call's result (the `Vec` was already
//! built and handed back), and it cannot be part of *any* call's result
//! until the *next* `drain`. Arrival becomes a tick event: whichever tick's
//! `drain` happens to run next is the tick that sees it, never the tick
//! already in progress. [`tests::a_message_enqueued_after_drain_is_invisible_until_the_next_drain`]
//! is the test that proves exactly this property, the single most important
//! thing this crate proves this wave (see the brief).

use std::collections::VecDeque;

/// A FIFO queue enforcing tick-quantised delivery. See the module doc.
///
/// Generic over the queued item so both [`crate::coordinator_bridge`]
/// (queuing decoded control-wire arrivals) and [`crate::wasm_transport`]
/// (queuing `gc_netcode::fault_transport::TransportPeerMessage` arrivals)
/// share one implementation and one proof of correctness rather than two
/// copies that could drift.
#[derive(Debug)]
pub struct TickQueue<T> {
    queue: VecDeque<T>,
}

impl<T> TickQueue<T> {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        TickQueue {
            queue: VecDeque::new(),
        }
    }

    /// Appends `item`. Safe to call at any time, from any caller — this is
    /// the only operation a network callback may perform (see the module
    /// doc). Never touches, and cannot touch, anything but this queue.
    pub fn enqueue(&mut self, item: T) {
        self.queue.push_back(item);
    }

    /// Removes and returns every item currently queued, oldest first,
    /// leaving the queue empty. This is the tick boundary: call it from
    /// exactly one place, at the start of one tick-quantised step. See the
    /// module doc for why this is the entire discipline.
    pub fn drain(&mut self) -> Vec<T> {
        self.queue.drain(..).collect()
    }

    /// [`TickQueue::drain`], capped at `limit` items; any remainder stays
    /// queued (still oldest-first) for the next `drain`/`drain_up_to` call.
    /// Mirrors `gc_netcode::fault_transport::StarTransportAdapter::poll_batch`'s
    /// own bounded-batch contract, which [`crate::wasm_transport`] delegates
    /// to this method to satisfy.
    pub fn drain_up_to(&mut self, limit: usize) -> Vec<T> {
        let take = limit.min(self.queue.len());
        self.queue.drain(..take).collect()
    }

    /// How many items are currently queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// True when nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl<T> Default for TickQueue<T> {
    fn default() -> Self {
        TickQueue::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_returns_items_in_arrival_order_and_empties_the_queue() {
        let mut queue = TickQueue::new();
        queue.enqueue("a");
        queue.enqueue("b");
        queue.enqueue("c");
        assert_eq!(queue.len(), 3);

        let drained = queue.drain();
        assert_eq!(drained, vec!["a", "b", "c"]);
        assert!(queue.is_empty());
    }

    #[test]
    fn a_drain_with_nothing_queued_returns_empty_and_never_panics() {
        let mut queue: TickQueue<i32> = TickQueue::new();
        assert_eq!(queue.drain(), Vec::<i32>::new());
    }

    /// The core property: a message enqueued *after* one `drain` call
    /// returns cannot be observed by that call — trivially true, since the
    /// `Vec` it returned already exists — and, more importantly, is not
    /// observed by any means *before* the next `drain` call: nothing but
    /// `drain`/`drain_up_to` can ever read the queue's contents. This
    /// directly models the browser scenario in the module doc: `enqueue_1`
    /// stands for network bytes arriving during tick N's processing (after
    /// tick N already called `drain`), and `drain_2` stands for tick N+1's
    /// boundary — the first, and only, point that data becomes visible.
    #[test]
    fn a_message_enqueued_after_drain_is_invisible_until_the_next_drain() {
        let mut queue = TickQueue::new();

        // Tick N: one message has arrived before the boundary.
        queue.enqueue("tick_n_arrival");
        let tick_n = queue.drain();
        assert_eq!(tick_n, vec!["tick_n_arrival"]);

        // Simulates the network callback firing while tick N's own
        // processing (everything downstream of `drain`, e.g. folding the
        // batch through the coordinator reducer) is still running — from
        // this queue's point of view that is indistinguishable from firing
        // any time before the next `drain` call, which is exactly the
        // property under test: it must not retroactively appear in `tick_n`
        // (already an owned `Vec`, structurally impossible) and it must not
        // be readable by any query before the next boundary.
        queue.enqueue("tick_n_plus_1_arrival");
        assert_eq!(
            tick_n,
            vec!["tick_n_arrival"],
            "the already-returned tick batch must not gain a member"
        );

        // A second arrival before the next boundary — both must be held,
        // in order, until the next `drain`.
        queue.enqueue("also_tick_n_plus_1");

        // Tick N+1's boundary: exactly the two messages enqueued after tick
        // N's drain appear now, in arrival order, and nothing from tick N
        // is redelivered.
        let tick_n_plus_1 = queue.drain();
        assert_eq!(
            tick_n_plus_1,
            vec!["tick_n_plus_1_arrival", "also_tick_n_plus_1"]
        );

        // And the queue is quiescent again: a third drain with nothing new
        // enqueued sees nothing.
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn drain_up_to_caps_the_batch_and_retains_the_remainder_in_order() {
        let mut queue = TickQueue::new();
        for i in 0..5 {
            queue.enqueue(i);
        }

        let first = queue.drain_up_to(2);
        assert_eq!(first, vec![0, 1]);
        assert_eq!(queue.len(), 3);

        // The remainder is still oldest-first and still invisible to
        // anything but the next drain call.
        let rest = queue.drain_up_to(10);
        assert_eq!(rest, vec![2, 3, 4]);
        assert!(queue.is_empty());
    }

    #[test]
    fn many_interleaved_enqueue_and_drain_cycles_never_lose_or_duplicate_an_item() {
        let mut queue = TickQueue::new();
        let mut next_id = 0u64;
        let mut expected_total = 0u64;
        let mut observed_total = 0u64;

        for cycle in 0..50 {
            // A variable number of "network arrivals" per simulated tick.
            for _ in 0..(cycle % 4) {
                queue.enqueue(next_id);
                expected_total += 1;
                next_id += 1;
            }
            for item in queue.drain() {
                observed_total += 1;
                assert!(item < next_id, "drained an id that was never enqueued");
            }
        }
        // A final drain after the loop must not conjure extra items.
        assert!(queue.drain().is_empty());
        assert_eq!(observed_total, expected_total);
    }
}
