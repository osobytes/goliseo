//! Port of `spec/game/online_match_driver_spec.lua`.
//!
//! ## `fixture.session` is unblocked
//!
//! Every one of the spec's 53 assertions (46 static `t.it` cases plus one
//! `t.it` inside a loop over `combat_phases.PHASES`'s 7 named phases) is
//! built through the spec's own `harness(mode, options)` helper
//! (`spec/game/online_match_driver_spec.lua:34-82`), which calls
//! `fixture.session(mode, ...)`. That, in turn, needed `coordinator.lua`
//! (`plan_assignments`, `slot_sources`), `protocol.lua` (`match_mode`,
//! `owned_slots`, `manifest_id`, `assignment_id`), `protocol_fixture.lua`,
//! and `game/transport/fake_star.lua` — all landed now
//! (`crate::coordinator`, `crate::protocol`, `crate::protocol_fixture`,
//! `crate::fake_star`), and `crate::match_driver_fixture::session` builds a
//! real connected host+guest session.
//!
//! `match_driver_differential_matches_the_lua_reference` is the highest-value
//! case this unblocks: it builds the exact `fixture.session("1v1")` scenario
//! the committed reference trace
//! (`tests/fixtures/match_driver_lua_reference.txt`) was captured from —
//! bursty delivery (the star pumps every 5th step only), 90 steps, neutral
//! samples — and asserts `diagnostics()`/`checkpoints()` agree with the real
//! Lua `match_driver.lua` tick for tick, including the boundary hash at every
//! one of the 9 checkpoints (10, 20, ..., 90).
//!
//! `mod online_match_driver` ports 15 more of the spec's static cases for
//! real, using this file's own `harness`/`advance`/`run`/`assert_agreement`/
//! `assert_confirmed_state` (mirroring the spec's own helpers at
//! `spec/game/online_match_driver_spec.lua:34-172`). The remaining ~38 cases
//! in `spec/game/online_match_driver_spec.lua` still need individual porting
//! from the spec file — mostly impaired-delivery/fault-injection scenarios
//! (`wrap_host_transport`/`wrap_guest_transport`), the settle phase, and the
//! combat-phase loop; see this crate's report for what is and is not covered
//! here yet.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use gc_netcode::coordinator;
use gc_netcode::fake_star;
use gc_netcode::fault_transport::{
    StarTransportAdapter, TransportChannel, TransportMessage, TransportMessageType,
    TransportPeerEvent, TransportPeerMessage, TransportPeerState, TransportResult, TransportRole,
    TransportStarDiagnostics, TransportState,
};
use gc_netcode::input_protocol;
use gc_netcode::match_driver::{
    self, CoordinatorNetcodeFailure, DriverRole, MatchDriverOptions, MatchDriverStatus,
};
use gc_netcode::match_driver_fixture::{self, DriverRules, MatchDriverFixtureSession};
use gc_netcode::protocol::{self, MatchMode, Value};
use gc_netcode::protocol_fixture;
use gc_sim::input_frame::{self, InputSample};
use gc_sim::match_snapshot::MatchSnapshot;

// ---------------------------------------------------------------------------
// Port of `spec/game/online_match_driver_spec.lua`'s own `harness`/`advance`/
// `run`/`assert_agreement` helpers (lines 34-172), now that
// `crate::match_driver_fixture::session` builds a real connected session.
// ---------------------------------------------------------------------------

/// Mirrors `DriverHarness`.
struct DriverHarness {
    session: MatchDriverFixtureSession,
    /// Host first, then guests in seating order — matches `session.guest_peer_ids`.
    drivers: Vec<match_driver::MatchDriver>,
    step: i64,
}

/// A caller-supplied decorator around the host's own transport endpoint.
/// Mirrors `DriverHarnessOptions.wrap_host_transport`.
type WrapHostTransport =
    Box<dyn Fn(Box<dyn StarTransportAdapter>) -> Box<dyn StarTransportAdapter>>;
/// A caller-supplied decorator around one guest's transport endpoint,
/// keyed by that driver's 1-based position in `DriverHarness.drivers`
/// (host is always `1`). Mirrors `DriverHarnessOptions.wrap_guest_transport`.
type WrapGuestTransport =
    Box<dyn Fn(i64, Box<dyn StarTransportAdapter>) -> Box<dyn StarTransportAdapter>>;

/// Mirrors `DriverHarnessOptions` (the subset this file's ported cases use).
#[derive(Default)]
struct DriverHarnessOptions {
    duration: Option<f64>,
    humans: Option<i64>,
    combat: bool,
    hash_interval_ticks: Option<i64>,
    max_rollback_ticks: Option<i64>,
    settle_timeout_ticks: Option<i64>,
    settle_timeout_seconds: Option<f64>,
    /// A shared monotonic-seconds counter. Every driver built from this
    /// harness gets its own closure over the *same* cell, mirroring the Lua
    /// spec's single shared `clock` upvalue (`spec/game/online_match_driver_spec.lua`'s
    /// `bounds the settle phase in wall clock...` case): every driver's
    /// clock read ticks the one counter forward.
    clock: Option<Rc<RefCell<f64>>>,
    /// 1-based driver index (host is `1`) whose boundary zero is seeded
    /// differently. Mirrors `DriverHarnessOptions.divergent_peer`.
    divergent_peer: Option<i64>,
    wrap_host_transport: Option<WrapHostTransport>,
    wrap_guest_transport: Option<WrapGuestTransport>,
}

/// Builds a [`Box<dyn FnMut() -> f64>`] that increments and reads a shared
/// counter, so every driver sharing one `Rc<RefCell<f64>>` observes the same
/// monotonically advancing clock — see [`DriverHarnessOptions::clock`].
fn shared_clock(counter: Rc<RefCell<f64>>) -> Box<dyn FnMut() -> f64> {
    Box::new(move || {
        let mut value = counter.borrow_mut();
        *value += 1.0;
        *value
    })
}

fn build_driver(
    session: &MatchDriverFixtureSession,
    role: DriverRole,
    peer_id: &str,
    transport: Box<dyn StarTransportAdapter>,
    snapshot: &MatchSnapshot,
    options: &DriverHarnessOptions,
) -> match_driver::MatchDriver {
    match_driver::new(MatchDriverOptions {
        role,
        peer_id: peer_id.to_string(),
        freeze: match_driver_fixture::to_driver_freeze(&session.freeze),
        manifest: match_driver_fixture::to_driver_manifest(&session.manifest),
        transport,
        initial_snapshot: snapshot.clone(),
        max_rollback_ticks: options.max_rollback_ticks,
        hash_interval_ticks: options.hash_interval_ticks,
        settle_timeout_ticks: options.settle_timeout_ticks,
        settle_timeout_seconds: options.settle_timeout_seconds,
        clock: options.clock.clone().map(shared_clock),
        rules: Box::new(DriverRules::new(
            session.manifest.clone(),
            session.freeze.clone(),
        )),
    })
}

/// Mirrors the spec's `harness(mode, options)`.
fn harness(mode: MatchMode, options: DriverHarnessOptions) -> DriverHarness {
    let session = match_driver_fixture::session(mode, None, options.humans);
    let snapshot = match_driver_fixture::initial_snapshot(options.duration, options.combat, None);
    // Every peer shares one boundary zero in a real session; a differing
    // seed for `divergent_peer` is the cheapest honest way to give one peer
    // a genuinely divergent simulation while every input row still agrees.
    let divergent_snapshot = options.divergent_peer.map(|_| {
        match_driver_fixture::initial_snapshot(
            options.duration,
            options.combat,
            Some(match_driver_fixture::DEFAULT_SEED + 1.0),
        )
    });
    let snapshot_for = |index: i64| -> MatchSnapshot {
        if Some(index) == options.divergent_peer {
            divergent_snapshot
                .clone()
                .expect("divergent snapshot built for the divergent peer")
        } else {
            snapshot.clone()
        }
    };

    let mut drivers = Vec::new();
    let host_transport: Box<dyn StarTransportAdapter> = {
        let inner: Box<dyn StarTransportAdapter> = Box::new(session.host_transport.clone());
        match &options.wrap_host_transport {
            Some(wrap) => wrap(inner),
            None => inner,
        }
    };
    drivers.push(build_driver(
        &session,
        DriverRole::Host,
        &session.host_peer_id,
        host_transport,
        &snapshot_for(1),
        &options,
    ));
    for (offset, peer_id) in session.guest_peer_ids.iter().enumerate() {
        let index = offset as i64 + 2;
        let transport = session
            .guest_transports
            .get(peer_id)
            .expect("session built a transport for every seated guest")
            .clone();
        let transport: Box<dyn StarTransportAdapter> = {
            let inner: Box<dyn StarTransportAdapter> = Box::new(transport);
            match &options.wrap_guest_transport {
                Some(wrap) => wrap(index, inner),
                None => inner,
            }
        };
        drivers.push(build_driver(
            &session,
            DriverRole::Guest,
            peer_id,
            transport,
            &snapshot_for(index),
            &options,
        ));
    }
    DriverHarness {
        session,
        drivers,
        step: 0,
    }
}

/// Mirrors the spec's `advance(harness_state, samples)`. `samples[i]` is the
/// sample for `drivers[i]`; `None`/short entries default to neutral, exactly
/// like the Lua `samples and samples[index] or input_frame.neutral_sample()`.
fn advance(
    h: &mut DriverHarness,
    samples: &[Option<InputSample>],
) -> Vec<match_driver::MatchDriverBatch> {
    let mut batches = Vec::with_capacity(h.drivers.len());
    for (index, driver) in h.drivers.iter_mut().enumerate() {
        let sample = samples.get(index).copied().flatten();
        batches.push(match_driver::advance(driver, sample));
    }
    h.session.host_transport.pump();
    h.step += 1;
    batches
}

/// Mirrors the spec's `run(harness_state, steps, samples)`.
fn run(
    h: &mut DriverHarness,
    steps: i64,
    samples: &[Option<InputSample>],
) -> Vec<match_driver::MatchDriverBatch> {
    let mut last = Vec::new();
    for _ in 0..steps {
        last = advance(h, samples);
    }
    last
}

/// Mirrors the spec's `assert_agreement(harness_state)`: every checkpoint two
/// drivers both hashed must agree, boundary hash and live-slot map alike.
fn assert_agreement(h: &DriverHarness) -> i64 {
    let reference = match_driver::checkpoints(&h.drivers[0]);
    let mut compared = 0i64;
    for checkpoint in &reference {
        for driver in &h.drivers[1..] {
            let mine = match_driver::checkpoints(driver)
                .into_iter()
                .find(|c| c.tick == checkpoint.tick);
            if let Some(mine) = mine {
                assert_eq!(
                    mine.hash, checkpoint.hash,
                    "boundary hash at {}",
                    checkpoint.tick
                );
                for (producer_id, slot) in &checkpoint.live {
                    assert_eq!(
                        mine.live.get(producer_id),
                        Some(slot),
                        "live slot for {producer_id} at {}",
                        checkpoint.tick
                    );
                }
                compared += 1;
            }
        }
    }
    compared
}

/// Mirrors the spec's `assert_confirmed_state(harness_state)`: the confirmed
/// (not merely present) boundary is where every peer must agree on state,
/// since that is where authority is complete rather than predicted.
fn assert_confirmed_state(h: &DriverHarness) {
    let mut boundary: Option<i64> = None;
    for driver in &h.drivers {
        let confirmed = match_driver::diagnostics(driver).confirmed_output_tick;
        boundary = Some(boundary.map_or(confirmed, |b| b.min(confirmed)));
    }
    let boundary = boundary.expect("at least one driver") + 1;
    assert!(boundary > 0, "no peer confirmed a single boundary");
    let mut reference: Option<String> = None;
    for driver in &h.drivers {
        use gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus as Status;
        let lookup = match_driver::snapshot(driver, boundary);
        assert!(matches!(lookup.status, Status::Present | Status::Retained));
        let snapshot = lookup
            .snapshot
            .expect("present/retained lookup carries a snapshot");
        let hash = gc_sim::match_snapshot::hash(&snapshot);
        if let Some(reference) = &reference {
            assert_eq!(&hash, reference, "confirmed boundary {boundary}");
        } else {
            reference = Some(hash);
        }
    }
}

fn wire(slot: input_frame::SlotId) -> &'static str {
    protocol::slot_wire_id(slot)
}

fn switch_sample() -> InputSample {
    input_frame::new_sample(input_frame::InputSampleOptions {
        edges: Some(input_frame::EDGE_SWITCH),
        ..Default::default()
    })
    .expect("a switch-only sample is always valid")
}

/// Mirrors the spec's local `sample(sample_options)`.
fn sample(options: input_frame::InputSampleOptions) -> InputSample {
    input_frame::new_sample(options).expect("a valid quantized sample")
}

/// Impaired delivery: the star only drains every `period` steps, so every
/// peer predicts the rows it has not received and corrects them in one
/// burst. Mirrors the spec's `run_bursty(harness_state, steps, period, samples)`.
fn run_bursty(h: &mut DriverHarness, steps: i64, period: i64, samples: &[Option<InputSample>]) {
    for step in 1..=steps {
        for (index, driver) in h.drivers.iter_mut().enumerate() {
            let sample = samples.get(index).copied().flatten();
            let _ = match_driver::advance(driver, sample);
        }
        if step % period == 0 {
            h.session.host_transport.pump();
        }
        h.step += 1;
    }
}

/// Input that changes every step, so a burst opens a real divergence
/// instead of one prediction repeats for free. Mirrors the spec's
/// `moving_sample(step, index)`.
fn moving_sample(step: i64, index: i64) -> InputSample {
    let phase = (step * 7 + index * 13) % 8;
    sample(input_frame::InputSampleOptions {
        move_x: Some(90 - phase * 24),
        move_y: Some(phase * 17 - 60),
        edges: Some(if phase == 3 {
            input_frame::EDGE_SWITCH
        } else {
            0
        }),
        ..Default::default()
    })
}

/// Advances every driver once per step, delivering only on the steps
/// `deliver` allows, and stopping once no driver is still active. Mirrors
/// the spec's `drive(harness_state, steps, deliver, sample_for)`.
fn drive(
    h: &mut DriverHarness,
    steps: i64,
    mut deliver: impl FnMut(i64) -> bool,
    sample_for: Option<fn(i64, i64) -> InputSample>,
) {
    for _ in 0..steps {
        let step = h.step;
        let mut active = false;
        for (index, driver) in h.drivers.iter_mut().enumerate() {
            let sample = Some(
                sample_for.map_or_else(input_frame::neutral_sample, |f| f(step, index as i64 + 1)),
            );
            let _ = match_driver::advance(driver, sample);
            active = active || match_driver::status(driver) == MatchDriverStatus::Active;
        }
        if deliver(step) {
            h.session.host_transport.pump();
        }
        h.step = step + 1;
        if !active {
            return;
        }
    }
}

/// A short match, so a burst can straddle full time without the hold itself
/// outrunning the 30-tick retained window and turning the run into
/// `late_input`. Mirrors the spec's `SETTLE_DURATION`.
const SETTLE_DURATION: f64 = 24.0 / 60.0;

/// Which tick full time lands on is `sim.match`'s countdown to decide, not
/// arithmetic on the duration. Probed once, lazily, exactly like the spec's
/// module-load-time `FULL_TIME_BOUNDARY` IIFE.
fn full_time_boundary_probe() -> i64 {
    static BOUNDARY: OnceLock<i64> = OnceLock::new();
    *BOUNDARY.get_or_init(|| {
        let mut probe = harness(
            MatchMode::OneVOne,
            DriverHarnessOptions {
                duration: Some(SETTLE_DURATION),
                ..Default::default()
            },
        );
        drive(&mut probe, 200, |_| true, None);
        match_driver::full_time_boundary(&probe.drivers[0]).expect("probe reaches full time")
    })
}

/// Driver step `T` simulates input tick `T`, so this is the step that
/// reaches [`full_time_boundary_probe`]. Mirrors the spec's `FULL_TIME_STEP`.
fn full_time_step() -> i64 {
    full_time_boundary_probe() - 1
}

/// Every peer completed through the settle phase, on the same final
/// boundary, with every tick of the match authoritative and the same hash
/// captured there. Mirrors the spec's `assert_settled(harness_state, label)`.
fn assert_settled(h: &DriverHarness, label: &str) -> i64 {
    let mut boundary: Option<i64> = None;
    for (index, driver) in h.drivers.iter().enumerate() {
        let diagnostics = match_driver::diagnostics(driver);
        assert_eq!(
            match_driver::status(driver),
            MatchDriverStatus::Completed,
            "peer {} did not complete in {label}",
            index + 1
        );
        assert_eq!(
            match_driver::terminal(driver)
                .expect("a non-active driver has a terminal")
                .failure,
            None
        );
        assert!(
            match_driver::settled(driver),
            "peer {} completed without settling in {label}",
            index + 1
        );
        assert!(!diagnostics.settling);
        let mine =
            match_driver::full_time_boundary(driver).expect("a completed driver reached full time");
        let boundary_value = *boundary.get_or_insert(mine);
        assert_eq!(
            mine,
            boundary_value,
            "final boundary on peer {} in {label}",
            index + 1
        );
        // The settle phase's whole contract: nothing in the match is left
        // unconfirmed when it reports the result.
        assert_eq!(
            diagnostics.confirmed_output_tick,
            boundary_value - 1,
            "peer {} completed with an unconfirmed tail in {label}",
            index + 1
        );
    }
    let boundary = boundary.expect("at least one driver");
    let mut reference: Option<String> = None;
    for (index, driver) in h.drivers.iter().enumerate() {
        use gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus as Status;
        let lookup = match_driver::snapshot(driver, boundary);
        assert!(matches!(lookup.status, Status::Present | Status::Retained));
        let hash = gc_sim::match_snapshot::hash(
            lookup
                .snapshot
                .as_ref()
                .expect("present/retained lookup carries a snapshot"),
        );
        if let Some(reference) = &reference {
            assert_eq!(
                &hash,
                reference,
                "final hash on peer {} in {label}",
                index + 1
            );
        } else {
            reference = Some(hash);
        }
    }
    boundary
}

// ---------------------------------------------------------------------------
// Transport decorators used by the wrapped-transport cases below. A single
// hookable wrapper covers every shape the spec's ad hoc `recorder`/`filter`
// tables need: forward everything, but let a test observe a broadcast or
// override what `poll_batch`/`send` return.
// ---------------------------------------------------------------------------

type BroadcastHook = Box<dyn FnMut(TransportChannel, &TransportMessage)>;
type PollBatchFilter = Box<dyn FnMut(Vec<TransportPeerMessage>) -> Vec<TransportPeerMessage>>;
type SendOverride =
    Box<dyn FnMut(&str, TransportChannel, &TransportMessage) -> Option<TransportResult<bool>>>;

#[derive(Default)]
struct TransportHooks {
    on_broadcast: Option<BroadcastHook>,
    poll_batch_filter: Option<PollBatchFilter>,
    send_override: Option<SendOverride>,
}

struct HookedTransport {
    inner: Box<dyn StarTransportAdapter>,
    hooks: TransportHooks,
}

impl StarTransportAdapter for HookedTransport {
    fn initialize(&mut self) -> TransportResult<bool> {
        self.inner.initialize()
    }
    fn shutdown(&mut self) -> TransportResult<bool> {
        self.inner.shutdown()
    }
    fn role(&self) -> TransportRole {
        self.inner.role()
    }
    fn capacity(&self) -> i64 {
        self.inner.capacity()
    }
    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        self.inner.open_peer(peer_id)
    }
    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        self.inner.close_peer(peer_id, reason)
    }
    fn peer_ids(&self) -> Vec<String> {
        self.inner.peer_ids()
    }
    fn peer_state(&self, peer_id: &str) -> Option<TransportPeerState> {
        self.inner.peer_state(peer_id)
    }
    fn request_offer(&mut self, peer_id: &str) -> TransportResult<bool> {
        self.inner.request_offer(peer_id)
    }
    fn accept_offer(&mut self, signal: &str) -> TransportResult<bool> {
        self.inner.accept_offer(signal)
    }
    fn accept_answer(&mut self, peer_id: &str, signal: &str) -> TransportResult<bool> {
        self.inner.accept_answer(peer_id, signal)
    }
    fn take_signal(&mut self, peer_id: &str) -> TransportResult<Option<String>> {
        self.inner.take_signal(peer_id)
    }
    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        if let Some(hook) = &mut self.hooks.send_override {
            if let Some(result) = hook(peer_id, channel, &message) {
                return result;
            }
        }
        self.inner.send(peer_id, channel, message)
    }
    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        if let Some(hook) = &mut self.hooks.on_broadcast {
            hook(channel, &message);
        }
        self.inner.broadcast(channel, message)
    }
    fn poll(&mut self) -> Option<TransportPeerMessage> {
        self.inner.poll()
    }
    fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage> {
        let messages = self.inner.poll_batch(limit);
        match &mut self.hooks.poll_batch_filter {
            Some(filter) => filter(messages),
            None => messages,
        }
    }
    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        self.inner.poll_event()
    }
    fn state(&self) -> TransportState {
        self.inner.state()
    }
    fn diagnostics(&self) -> TransportStarDiagnostics {
        self.inner.diagnostics()
    }
}

mod online_match_driver {
    use super::*;

    #[test]
    fn seats_a_4v4_host_on_one_slot_and_authors_every_bot_fill() {
        let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
        let host = match_driver::diagnostics(&state.drivers[0]);
        assert_eq!(host.role, DriverRole::Host);
        assert_eq!(host.owned.len(), 1);
        assert_eq!(wire(host.owned[0]), "home_1");
        assert_eq!(host.control_slot.map(wire), Some("home_1"));
        // Eight humans in 4v4, so the host authors only its own slot.
        assert_eq!(host.authored.len(), 1);
        let guest = match_driver::diagnostics(&state.drivers[1]);
        assert_eq!(guest.owned.len(), 1);
        assert_eq!(wire(guest.owned[0]), "home_2");
        let _ = &mut state;
    }

    #[test]
    fn gives_a_1v1_human_four_owned_slots_and_one_control_slot() {
        let state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        let host = match_driver::diagnostics(&state.drivers[0]);
        assert_eq!(host.owned.len(), 4);
        assert_eq!(
            host.owned
                .iter()
                .map(|&s| wire(s))
                .collect::<Vec<_>>()
                .join(","),
            "home_1,home_2,home_3,home_4"
        );
        assert_eq!(host.control_slot.map(wire), Some("home_1"));
        // No bot fills at all in 1v1: every slot belongs to one of the two
        // humans, so the host authors exactly its own four.
        assert_eq!(host.authored.len(), 4);
        let live = &state.drivers[0].live[&state.drivers[0].live_tick];
        let drivers = coordinator::slot_drivers(&state.session.freeze, Some(live));
        let index_of =
            |wire_id: &str| protocol::slot_index(wire_id).expect("canonical wire slot id");
        assert_eq!(
            drivers[(index_of("home_1") - 1) as usize],
            coordinator::SlotDriver::Human
        );
        assert_eq!(
            drivers[(index_of("home_2") - 1) as usize],
            coordinator::SlotDriver::Ai
        );
        assert_eq!(
            drivers[(index_of("away_1") - 1) as usize],
            coordinator::SlotDriver::Human
        );
        assert_eq!(
            drivers[(index_of("away_2") - 1) as usize],
            coordinator::SlotDriver::Ai
        );
    }

    #[test]
    fn makes_the_host_author_every_declared_bot_fill_in_a_short_lobby() {
        // A full lobby covers all eight slots in every mode, so a declared bot
        // fill only exists when fewer humans are seated than the mode allows.
        let mut state = harness(
            MatchMode::TwoVTwo,
            DriverHarnessOptions {
                humans: Some(2),
                ..Default::default()
            },
        );
        let mut bots = 0;
        for index in 1..=input_frame::SLOT_COUNT {
            let producer = state
                .session
                .freeze
                .assignments
                .get_index(index)
                .expect("canonical assignment slot");
            if producer
                .get("producer_kind")
                .and_then(protocol::Value::as_str)
                == Some("bot")
            {
                bots += 1;
            }
        }
        assert_eq!(bots, 4);
        let host = match_driver::diagnostics(&state.drivers[0]);
        assert_eq!(host.owned.len(), 2);
        // Its own two slots plus all four bots, every one of them carried by
        // the host's own delayed collector path.
        assert_eq!(host.authored.len(), 6);
        let guest = match_driver::diagnostics(&state.drivers[1]);
        assert_eq!(guest.authored.len(), 2);

        run(&mut state, 24, &[]);
        assert!(assert_agreement(&state) > 0);
        assert_confirmed_state(&state);
        // Declared fills are indistinguishable from a human's non-live owned
        // slots in the stream: every slot is authoritative at a confirmed tick.
        let live = &state.drivers[0].live[&state.drivers[0].live_tick];
        let drivers = coordinator::slot_drivers(&state.session.freeze, Some(live));
        let humans = drivers
            .iter()
            .filter(|&&d| d == coordinator::SlotDriver::Human)
            .count();
        assert_eq!(humans, 2);
    }

    #[test]
    fn converges_every_peer_on_the_same_confirmed_boundary_hashes_in_4v4() {
        let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
        run(&mut state, 40, &[]);
        assert!(assert_agreement(&state) > 0);
        for driver in &state.drivers {
            assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
            let diagnostics = match_driver::diagnostics(driver);
            assert_eq!(diagnostics.present_input_tick, 40);
            assert!(diagnostics.confirmed_input_tick >= 30);
        }
        assert_confirmed_state(&state);
    }

    #[test]
    fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_1v1() {
        let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        // The host presses switch continuously: liveness must move identically
        // on both peers, and 1v1 is where a divergence can actually appear.
        run(&mut state, 48, &[Some(switch_sample())]);
        assert!(assert_agreement(&state) > 0);
        for driver in &state.drivers {
            assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
        }
        assert_confirmed_state(&state);
    }

    #[test]
    fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_2v2() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(
            &mut state,
            48,
            &[Some(switch_sample()), None, Some(switch_sample())],
        );
        assert!(assert_agreement(&state) > 0);
        assert_confirmed_state(&state);
    }

    #[test]
    fn moves_the_control_slot_only_through_the_canonical_stream() {
        let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        let host_peer_id = state.session.host_peer_id.clone();
        assert_eq!(
            wire(match_driver::control_slot(
                &state.drivers[0],
                &host_peer_id,
                0
            )),
            "home_1"
        );
        run(&mut state, 24, &[Some(switch_sample())]);
        let mut moved = false;
        for tick in 1..=20 {
            if match_driver::control_slot(&state.drivers[0], &host_peer_id, tick)
                != input_frame::SlotId::Home1
            {
                moved = true;
            }
        }
        assert!(moved, "a held switch never moved the control slot");
        // The guest, which never saw the keypress, reaches the same conclusion
        // purely from the canonical rows it received.
        for tick in 0..=20 {
            assert_eq!(
                match_driver::control_slot(&state.drivers[1], &host_peer_id, tick),
                match_driver::control_slot(&state.drivers[0], &host_peer_id, tick)
            );
        }
    }

    #[test]
    fn applies_one_transport_tick_arrival_batch_as_one_reconciliation() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        for _ in 0..24 {
            let batches = advance(&mut state, &[]);
            for batch in &batches {
                assert!(
                    batch.reconciliations <= 1,
                    "more than one reconciliation in one step"
                );
            }
        }
    }

    #[test]
    fn is_insensitive_to_the_order_peers_are_polled_and_stepped() {
        let mut forward = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut forward, 32, &[]);
        let mut reversed = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        for _ in 0..32 {
            for driver in reversed.drivers.iter_mut().rev() {
                let _ = match_driver::advance(driver, None);
            }
            reversed.session.host_transport.pump();
        }
        for index in 0..forward.drivers.len() {
            let boundary = match_driver::diagnostics(&forward.drivers[index]).confirmed_output_tick;
            assert_eq!(
                match_driver::diagnostics(&reversed.drivers[index]).confirmed_output_tick,
                boundary
            );
            let reversed_snapshot = match_driver::snapshot(&reversed.drivers[index], boundary)
                .snapshot
                .expect("boundary is retained");
            let forward_snapshot = match_driver::snapshot(&forward.drivers[index], boundary)
                .snapshot
                .expect("boundary is retained");
            assert_eq!(
                gc_sim::match_snapshot::hash(&reversed_snapshot),
                gc_sim::match_snapshot::hash(&forward_snapshot)
            );
        }
    }

    #[test]
    fn keeps_protected_keepers_ai_only_and_slotless() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 8, &[]);
        let snapshot = match_driver::current_snapshot(&state.drivers[0]);
        for index in 1..=input_frame::SLOT_COUNT {
            let player_index = snapshot.state.slot_players[(index - 1) as usize]
                .expect("canonical slot is mapped");
            assert!(!snapshot.state.players[(player_index - 1) as usize].is_keeper);
        }
    }

    #[test]
    fn accepts_every_slot_inside_a_peers_frozen_owned_set() {
        let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        run(&mut state, 16, &[]);
        // A 1v1 guest legitimately authors four different slots every tick.
        let diagnostics = match_driver::diagnostics(&state.drivers[1]);
        assert_eq!(diagnostics.authored.len(), 4);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Active
        );
        assert_eq!(
            match_driver::status(&state.drivers[1]),
            MatchDriverStatus::Active
        );
    }

    #[test]
    fn stops_all_progress_after_a_terminal_status() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 6, &[]);
        let driver = &mut state.drivers[1];
        let before = match_driver::diagnostics(driver);
        for _ in 0..5 {
            assert!(!match_driver::observe_checkpoint(
                driver,
                0,
                "0000000000000000"
            ));
        }
        assert_eq!(
            match_driver::status(driver),
            MatchDriverStatus::HashMismatch
        );
        let batch = match_driver::advance(driver, None);
        assert_eq!(batch.outputs.len(), 0);
        assert_eq!(batch.sent_packets, 0);
        assert_eq!(batch.status, MatchDriverStatus::HashMismatch);
        let after = match_driver::diagnostics(driver);
        assert_eq!(after.present_input_tick, before.present_input_tick);
        assert_eq!(after.confirmed_input_tick, before.confirmed_input_tick);
    }

    #[test]
    fn hands_control_channel_traffic_back_instead_of_eating_it() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 4, &[]);
        let guest_id = state.session.guest_peer_ids[0].clone();
        let mut guest_transport = state
            .session
            .guest_transports
            .get(&guest_id)
            .expect("session built this guest's transport")
            .clone();
        let control = TransportMessage {
            version: 1,
            kind: TransportMessageType::State,
            seq: 3,
            tick: None,
            payload: b"GCOP;control".to_vec(),
        };
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Control, control)
            .expect("a canonical control envelope always sends");
        state.session.host_transport.pump();
        let batches = advance(&mut state, &[]);
        assert_eq!(batches[0].control.len(), 1);
        assert_eq!(batches[0].control[0].channel, TransportChannel::Control);
        assert_eq!(batches[0].control[0].message.payload, b"GCOP;control");
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Active
        );
    }

    #[test]
    fn refuses_authority_from_outside_a_peers_frozen_owned_set() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 4, &[]);
        let guest_id = state.session.guest_peer_ids[0].clone();
        let mut guest_transport = state
            .session
            .guest_transports
            .get(&guest_id)
            .expect("session built this guest's transport")
            .clone();
        // The guest owns home_3/home_4; away_1 (canonical slot index 5)
        // belongs to another human.
        let mut rows = Vec::new();
        for tick in 0..=6 {
            rows.push(input_protocol::AuthorityRow {
                tick,
                slot_index: 5,
                sample: input_frame::neutral_sample(),
            });
        }
        let session_id = state
            .session
            .manifest
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string();
        let forged = input_protocol::new_guest(input_protocol::PacketOptions {
            session_id,
            manifest_id: protocol::manifest_id(&state.session.manifest),
            sender_id: guest_id.clone(),
            sequence: 9000,
            transport_tick: 8,
            first_input_tick: 0,
            confirmed_span: None,
            rows,
        })
        .expect("a forged guest packet is well-formed");
        let wire = input_protocol::encode(&forged).expect("a well-formed packet encodes");
        let envelope = TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq: forged.sequence,
            tick: Some(forged.transport_tick),
            payload: wire,
        };
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Input, envelope)
            .expect("a canonical input envelope always sends");
        state.session.host_transport.pump();
        let _ = advance(&mut state, &[]);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::OwnershipViolation
        );
        let terminal =
            match_driver::terminal(&state.drivers[0]).expect("driver reached a terminal");
        assert_eq!(
            terminal.failure,
            Some(CoordinatorNetcodeFailure::InputChannel)
        );
    }

    #[test]
    fn is_idempotent_for_a_replayed_bundle_and_terminal_for_a_conflicting_one() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 6, &[]);
        let guest_id = state.session.guest_peer_ids[0].clone();
        let mut guest_transport = state
            .session
            .guest_transports
            .get(&guest_id)
            .expect("session built this guest's transport")
            .clone();
        let owned_slot = match_driver::slot_index_of(state.session.freeze.owned[&guest_id][0]);
        let session_id = state
            .session
            .manifest
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string();
        let manifest_id = protocol::manifest_id(&state.session.manifest);

        let bundle = |edges: i64| -> TransportMessage {
            let mut rows = Vec::new();
            for tick in 0..=6 {
                let sample = input_frame::new_sample(input_frame::InputSampleOptions {
                    edges: Some(if tick == 6 { edges } else { 0 }),
                    ..Default::default()
                })
                .expect("a neutral-plus-edges sample is always valid");
                rows.push(input_protocol::AuthorityRow {
                    tick,
                    slot_index: owned_slot,
                    sample,
                });
            }
            let packet = input_protocol::new_guest(input_protocol::PacketOptions {
                session_id: session_id.clone(),
                manifest_id: manifest_id.clone(),
                sender_id: guest_id.clone(),
                sequence: 4242,
                transport_tick: 9,
                first_input_tick: 0,
                confirmed_span: None,
                rows,
            })
            .expect("a well-formed guest packet");
            let wire = input_protocol::encode(&packet).expect("a well-formed packet encodes");
            TransportMessage {
                version: 1,
                kind: TransportMessageType::Input,
                seq: packet.sequence,
                tick: Some(packet.transport_tick),
                payload: wire,
            }
        };

        // The same sender sequence with byte-identical authority is a no-op.
        let repeated = bundle(0);
        guest_transport
            .send(
                fake_star::HOST_PEER_ID,
                TransportChannel::Input,
                repeated.clone(),
            )
            .expect("send");
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Input, repeated)
            .expect("send");
        state.session.host_transport.pump();
        let _ = advance(&mut state, &[]);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Active
        );

        // The same identity with different bytes is not.
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Input, bundle(0))
            .expect("send");
        guest_transport
            .send(
                fake_star::HOST_PEER_ID,
                TransportChannel::Input,
                bundle(input_frame::EDGE_DASH),
            )
            .expect("send");
        state.session.host_transport.pump();
        let _ = advance(&mut state, &[]);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::AuthorityConflict
        );
        assert_eq!(
            match_driver::terminal(&state.drivers[0])
                .expect("driver reached a terminal")
                .failure,
            Some(CoordinatorNetcodeFailure::InputChannel)
        );
    }
}

fn status_label(status: MatchDriverStatus) -> &'static str {
    match status {
        MatchDriverStatus::Active => "active",
        MatchDriverStatus::Completed => "completed",
        MatchDriverStatus::SettleTimeout => "settle_timeout",
        MatchDriverStatus::ConfirmationStalled => "confirmation_stalled",
        MatchDriverStatus::LateInput => "late_input",
        MatchDriverStatus::HashMismatch => "hash_mismatch",
        MatchDriverStatus::OwnershipViolation => "ownership_violation",
        MatchDriverStatus::AuthorityConflict => "authority_conflict",
        MatchDriverStatus::InputChannelFailure => "input_channel_failure",
        MatchDriverStatus::TransportLost => "transport_lost",
    }
}

struct ExpectedStep {
    step: i64,
    peer: String,
    status: String,
    present: i64,
    confirmed_in: i64,
    confirmed_out: i64,
    rollback: i64,
    correction: i64,
}

struct ExpectedHash {
    step: i64,
    peer: String,
    tick: i64,
    digest: String,
}

fn parse_fixture(text: &str) -> (Vec<ExpectedStep>, Vec<ExpectedHash>) {
    let mut steps = Vec::new();
    let mut hashes = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields[0] == "hash" {
            hashes.push(ExpectedHash {
                step: fields[1].parse().expect("fixture hash step is an integer"),
                peer: fields[2].to_string(),
                tick: fields[3].parse().expect("fixture hash tick is an integer"),
                digest: fields[4].to_string(),
            });
        } else {
            steps.push(ExpectedStep {
                step: fields[0].parse().expect("fixture step is an integer"),
                peer: fields[1].to_string(),
                status: fields[2].to_string(),
                present: fields[3]
                    .parse()
                    .expect("fixture present tick is an integer"),
                confirmed_in: fields[4]
                    .parse()
                    .expect("fixture confirmed_in tick is an integer"),
                confirmed_out: fields[5]
                    .parse()
                    .expect("fixture confirmed_out tick is an integer"),
                rollback: fields[6]
                    .parse()
                    .expect("fixture rollback count is an integer"),
                correction: fields[7]
                    .parse()
                    .expect("fixture correction count is an integer"),
            });
        }
    }
    (steps, hashes)
}

/// See the module doc: reproduces `fixture.session("1v1")` driven through
/// `run_bursty(state, 90, 5)` with neutral samples, and asserts the Rust
/// driver reaches the identical diagnostics and boundary hashes the real Lua
/// `match_driver.lua` reached, tick for tick, at every one of the 90 steps
/// and every one of the 9 checkpoints.
#[test]
fn match_driver_differential_matches_the_lua_reference() {
    const FIXTURE: &str = include_str!("fixtures/match_driver_lua_reference.txt");
    let (expected_steps, expected_hashes) = parse_fixture(FIXTURE);

    let session = match_driver_fixture::session(MatchMode::OneVOne, None, None);
    assert_eq!(
        session.guest_peer_ids.len(),
        1,
        "1v1 seats exactly one guest"
    );
    let guest_peer_id = session.guest_peer_ids[0].clone();
    let guest_transport = session
        .guest_transports
        .get(&guest_peer_id)
        .expect("the guest transport session built")
        .clone();
    let host_transport = session.host_transport.clone();

    let build = |role: DriverRole, peer_id: &str, transport: Box<dyn StarTransportAdapter>| {
        match_driver::new(MatchDriverOptions {
            role,
            peer_id: peer_id.to_string(),
            freeze: match_driver_fixture::to_driver_freeze(&session.freeze),
            manifest: match_driver_fixture::to_driver_manifest(&session.manifest),
            transport,
            initial_snapshot: match_driver_fixture::initial_snapshot(None, false, None),
            max_rollback_ticks: None,
            hash_interval_ticks: Some(10),
            settle_timeout_ticks: None,
            settle_timeout_seconds: None,
            clock: None,
            rules: Box::new(DriverRules::new(
                session.manifest.clone(),
                session.freeze.clone(),
            )),
        })
    };

    let mut host = build(
        DriverRole::Host,
        &session.host_peer_id,
        Box::new(host_transport.clone()),
    );
    let mut guest = build(
        DriverRole::Guest,
        &guest_peer_id,
        Box::new(guest_transport.clone()),
    );

    for step in 1..=90i64 {
        let _ = match_driver::advance(&mut host, None);
        let _ = match_driver::advance(&mut guest, None);
        if step % 5 == 0 {
            host_transport.pump();
        }

        for (driver, label) in [(&host, "host"), (&guest, "guest")] {
            let expected = expected_steps
                .iter()
                .find(|e| e.step == step && e.peer == label)
                .unwrap_or_else(|| panic!("fixture is missing step {step} for {label}"));
            let diagnostics = match_driver::diagnostics(driver);
            assert_eq!(
                status_label(diagnostics.status),
                expected.status,
                "step {step} {label} status"
            );
            assert_eq!(
                diagnostics.present_input_tick, expected.present,
                "step {step} {label} present"
            );
            assert_eq!(
                diagnostics.confirmed_input_tick, expected.confirmed_in,
                "step {step} {label} confirmed_in"
            );
            assert_eq!(
                diagnostics.confirmed_output_tick, expected.confirmed_out,
                "step {step} {label} confirmed_out"
            );
            assert_eq!(
                diagnostics.rollback_count, expected.rollback,
                "step {step} {label} rollback"
            );
            assert_eq!(
                diagnostics.correction_count, expected.correction,
                "step {step} {label} correction"
            );
        }

        if step % 10 == 0 {
            for (driver, label) in [(&host, "host"), (&guest, "guest")] {
                let expected = expected_hashes
                    .iter()
                    .find(|h| h.step == step && h.peer == label)
                    .unwrap_or_else(|| {
                        panic!("fixture is missing hash at step {step} for {label}")
                    });
                let checkpoints = match_driver::checkpoints(driver);
                let newest = checkpoints
                    .last()
                    .unwrap_or_else(|| panic!("{label} has no checkpoint by step {step}"));
                assert_eq!(
                    newest.tick, expected.tick,
                    "step {step} {label} checkpoint tick"
                );
                assert_eq!(
                    newest.hash, expected.digest,
                    "step {step} {label} checkpoint hash"
                );
            }
        }
    }
}

/// Named for the spec's own `harness(mode, options)` helper
/// (`spec/game/online_match_driver_spec.lua:34-172`, ported above as
/// `mod online_match_driver`'s free functions). Every case below still needs
/// individual porting from the spec file; `fixture.session` itself is no
/// longer the blocker (see this file's module doc and `online_match_driver`'s
/// growing coverage).
#[test]
fn tolerates_one_boundary_disagreement_and_clears_it_on_the_next_agreement() {
    let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
    run(&mut state, 34, &[]);
    let driver = &mut state.drivers[0];
    let checkpoints = match_driver::checkpoints(driver);
    assert!(checkpoints.len() >= 2);
    assert!(!match_driver::observe_checkpoint(
        driver,
        checkpoints[0].tick,
        "dead0000dead0000"
    ));
    assert!(match_driver::observe_checkpoint(
        driver,
        checkpoints[0].tick,
        &checkpoints[0].hash
    ));
    assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
    assert_eq!(match_driver::diagnostics(driver).hash_mismatches, 0);
}

#[test]
fn publishes_confirmed_boundary_hashes_on_the_documented_interval() {
    let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
    run(&mut state, 40, &[]);
    let checkpoints = match_driver::checkpoints(&state.drivers[0]);
    assert!(checkpoints.len() >= 2);
    assert_eq!(checkpoints[0].tick, 0);
    assert_eq!(
        checkpoints[1].tick,
        match_driver::DEFAULT_HASH_INTERVAL_TICKS
    );
    for checkpoint in &checkpoints {
        assert_eq!(checkpoint.hash.len(), 16);
    }
}

#[test]
fn ends_with_a_typed_completed_status_at_full_time() {
    let mut state = harness(
        MatchMode::FourVFour,
        DriverHarnessOptions {
            duration: Some(0.2),
            ..Default::default()
        },
    );
    for _ in 0..40 {
        advance(&mut state, &[]);
        if match_driver::status(&state.drivers[0]) != MatchDriverStatus::Active {
            break;
        }
    }
    assert_eq!(
        match_driver::status(&state.drivers[0]),
        MatchDriverStatus::Completed
    );
    let terminal = match_driver::terminal(&state.drivers[0]).expect("driver reached a terminal");
    assert_eq!(terminal.failure, None);
}

#[test]
fn converges_under_impaired_delivery_after_real_corrections() {
    let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
    run_bursty(
        &mut state,
        60,
        6,
        &[
            Some(sample(input_frame::InputSampleOptions {
                move_x: Some(90),
                ..Default::default()
            })),
            Some(sample(input_frame::InputSampleOptions {
                move_y: Some(-70),
                ..Default::default()
            })),
        ],
    );
    for driver in &state.drivers {
        assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
    }
    let mut rollbacks = 0;
    for driver in &state.drivers {
        rollbacks += match_driver::diagnostics(driver).rollback_count;
    }
    assert!(rollbacks > 0, "bursty delivery never produced a correction");
    assert!(assert_agreement(&state) > 0);
    assert_confirmed_state(&state);
}

#[test]
fn converges_in_1v1_under_impaired_delivery_with_live_slot_switching() {
    let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
    run_bursty(
        &mut state,
        60,
        5,
        &[Some(switch_sample()), Some(switch_sample())],
    );
    assert!(match_driver::diagnostics(&state.drivers[0]).rollback_count > 0);
    assert!(assert_agreement(&state) > 0);
    assert_confirmed_state(&state);
}

#[test]
fn terminates_explicitly_when_authority_falls_outside_the_retained_window() {
    let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
    // Nothing is delivered for well over the 30-tick floor, so confirmation
    // stops dead and the floor slides past the ticks it was waiting on.
    run_bursty(&mut state, 60, 50, &[]);
    let mut terminal_count = 0;
    for driver in &state.drivers {
        // Caught on confirmation liveness rather than on the arrival that
        // used to reveal it: the peer says so at the step the tick becomes
        // unconfirmable, not whenever the backlog lands.
        if match_driver::status(driver) == MatchDriverStatus::ConfirmationStalled {
            terminal_count += 1;
            let record = match_driver::terminal(driver).expect("driver reached a terminal");
            assert_eq!(record.failure, Some(CoordinatorNetcodeFailure::LateInput));
            assert!(record.tick.is_some());
        }
    }
    assert!(
        terminal_count > 0,
        "an over-window burst never terminated a peer"
    );
}

// FINDING 1 regression: `confirmed_tick` runs ahead of the simulated present
// by up to DELAY_TICKS even with zero loss and zero jitter, because a sample
// is authority before it is consumed. Keying a checkpoint's snapshot lookup
// off it aborted healthy matches whenever the interval landed on that race.
#[test]
fn publishes_checkpoints_on_the_simulated_ceiling_not_raw_confirmation() {
    for interval in [1, 2, 3, 4] {
        let mut state = harness(
            MatchMode::OneVOne,
            DriverHarnessOptions {
                hash_interval_ticks: Some(interval),
                ..Default::default()
            },
        );
        run(&mut state, 24, &[]);
        for driver in &state.drivers {
            assert_eq!(
                match_driver::status(driver),
                MatchDriverStatus::Active,
                "hash_interval_ticks={interval}"
            );
        }
        let checkpoints = match_driver::checkpoints(&state.drivers[0]);
        assert!(
            checkpoints.len() > 1,
            "interval {interval} published nothing"
        );
        for checkpoint in &checkpoints {
            // Never ahead of a boundary that was actually simulated.
            assert!(
                checkpoint.tick <= match_driver::diagnostics(&state.drivers[0]).present_input_tick
            );
        }
        assert!(assert_agreement(&state) > 0);
    }
}

#[test]
fn never_publishes_a_checkpoint_boundary_that_was_not_simulated() {
    // The invariant the fix rests on: the output-capped confirmation is
    // always at most one boundary behind the present, so `confirmed + 1`
    // always names a boundary the session actually captured.
    let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
    for _ in 0..24 {
        advance(&mut state, &[]);
        for driver in &state.drivers {
            use gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus as Status;
            let diagnostics = match_driver::diagnostics(driver);
            assert!(diagnostics.confirmed_output_tick <= diagnostics.confirmed_input_tick);
            assert!(diagnostics.confirmed_output_tick < diagnostics.present_input_tick);
            for checkpoint in match_driver::checkpoints(driver) {
                let lookup = match_driver::snapshot(driver, checkpoint.tick);
                assert!(matches!(lookup.status, Status::Present | Status::Retained));
            }
        }
    }
}

// FINDING 3 / retained-floor edge, revisited by #241. The driver still keeps
// no floor pre-check of its own -- `rollback_input_history` owns the floor.
// What changed is that the driver can no longer *reach* a below-floor
// arrival: a row is only offered to the history when it is above this peer's
// confirmation, so confirmation liveness terminates on `confirmed + 1 <
// floor` before any arrival is applied.
#[test]
fn terminates_on_confirmation_liveness_before_a_below_floor_row_can_arrive() {
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            max_rollback_ticks: Some(6),
            ..Default::default()
        },
    );
    let guest = &mut state.drivers[1];
    // Only the guest runs. The host is never advanced, so no canonical batch
    // is ever produced and the guest's seven remote slots never become
    // authoritative: its confirmation is pinned while its floor keeps
    // sliding, which is the only regime where the floor rule bites at all.
    for _ in 0..20 {
        let _ = match_driver::advance(guest, None);
    }
    assert_eq!(
        match_driver::status(guest),
        MatchDriverStatus::ConfirmationStalled
    );
    let diagnostics = match_driver::diagnostics(guest);
    let record = match_driver::terminal(guest).expect("driver reached a terminal");
    assert_eq!(record.failure, Some(CoordinatorNetcodeFailure::LateInput));
    assert_eq!(record.tick, Some(diagnostics.confirmed_input_tick + 1));
    assert_eq!(diagnostics.late_input_tick, record.tick);
    // The boundary itself, and the reason it is not off by one: the driver
    // terminates and makes no further progress, so the floor it froze at is
    // the first one that ever outran confirmation.
    assert_eq!(
        diagnostics.retained_floor_tick,
        diagnostics.confirmed_input_tick + 2,
        "confirmation liveness did not fire on the first step it could"
    );
}

// FINDING 4: the combination the 1v1 case covers, in the other mode that can
// exhibit live-slot divergence at all.
#[test]
fn agrees_on_the_live_slot_in_2v2_under_impaired_delivery_with_switching() {
    let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
    run_bursty(
        &mut state,
        60,
        5,
        &[
            Some(switch_sample()),
            Some(switch_sample()),
            Some(sample(input_frame::InputSampleOptions {
                move_x: Some(80),
                edges: Some(input_frame::EDGE_SWITCH),
                ..Default::default()
            })),
        ],
    );
    let mut rollbacks = 0;
    for driver in &state.drivers {
        rollbacks += match_driver::diagnostics(driver).rollback_count;
    }
    assert!(
        rollbacks > 0,
        "bursty 2v2 switching never produced a correction"
    );
    assert!(assert_agreement(&state) > 0);
    assert_confirmed_state(&state);
}

// FINDING 5: full time under impaired delivery, not only clean delivery.
#[test]
fn reaches_full_time_under_impaired_delivery() {
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(0.2),
            ..Default::default()
        },
    );
    run_bursty(&mut state, 60, 5, &[Some(switch_sample())]);
    for driver in &state.drivers {
        assert_eq!(match_driver::status(driver), MatchDriverStatus::Completed);
        assert_eq!(
            match_driver::terminal(driver)
                .expect("driver reached a terminal")
                .failure,
            None
        );
    }
    assert!(match_driver::diagnostics(&state.drivers[0]).rollback_count > 0);
}

// The mechanism: the companion is restored and resimulated at all. It says
// nothing about *what* the companion was doing.
#[test]
fn carries_the_combat_companion_through_correction_and_resimulation() {
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            combat: true,
            ..Default::default()
        },
    );
    let initial = match_driver::current_snapshot(&state.drivers[0]);
    assert_eq!(initial.version, gc_sim::match_snapshot::COMBAT_VERSION);
    assert!(initial.combat.is_some());
    run_bursty(
        &mut state,
        48,
        5,
        &[
            Some(switch_sample()),
            None,
            Some(sample(input_frame::InputSampleOptions {
                move_y: Some(60),
                ..Default::default()
            })),
        ],
    );
    assert!(match_driver::diagnostics(&state.drivers[0]).rollback_count > 0);
    for driver in &state.drivers {
        assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
        assert!(match_driver::current_snapshot(driver).combat.is_some());
    }
    assert!(assert_agreement(&state) > 0);
    assert_confirmed_state(&state);
}

#[test]
#[ignore = "needs a combat_phases fixture (spec/support/online_combat_phases.lua, 579 lines) that \
    has no Rust port yet -- fixture.session is not the blocker here; PHASES/scenario/boundary_zero \
    are"]
fn opens_every_combat_phase_scenario_from_a_ready_combat_state() {
    unreachable!("needs a combat_phases fixture port; see the ignore reason");
}

/// The spec's `for _, phase_id in ipairs(combat_phases.PHASES) do t.it(...) end`
/// loop (`spec/game/online_match_driver_spec.lua:867`) produces one case per
/// named combat correction phase; `crate::fault_harness::declare_contingent`
/// names the same 7 phases. Ported as 7 distinctly named cases rather than
/// one generic loop, so each is independently countable and independently
/// re-enableable.
mod converges_a_correction_taken_during_each_combat_phase {
    const REASON: &str = "needs a combat_phases fixture (spec/support/online_combat_phases.lua, \
        579 lines: PHASES/scenario/boundary_zero/live_sample/observed) that has no Rust port yet; \
        fixture.session is not the blocker -- see the parent module doc";

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn wind_up() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn guard() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn contact() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn projectile_flight() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn stagger() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn ball_spill() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "needs a combat_phases fixture port; see REASON"]
    fn immunity_expiry() {
        unreachable!("{REASON}");
    }
}

#[test]
#[ignore = "needs a combat_phases fixture (spec/support/online_combat_phases.lua, 579 lines: \
    GUARD_PROBE/guard_probe_boundary_zero) that has no Rust port yet"]
fn finds_no_driver_level_geometry_where_the_policy_guards_often_enough() {
    unreachable!("needs a combat_phases fixture port; see the ignore reason");
}

#[test]
#[ignore = "cannot be forced without a src/ change: the Lua case monkeypatches \
    rollback_session.reconcile/add_authoritative_batch to count calls and force a divergence \
    report; match_driver.rs (src, out of scope for this pass) calls \
    gc_sim::rollback_session::{reconcile, add_authoritative_batch} directly as free functions with \
    no injection seam, so there is no way to intercept or count those calls from a test without \
    widening match_driver's internals -- report as needing a pub seam, not a test-only workaround"]
fn still_reconciles_if_a_local_insert_ever_reports_a_divergence() {
    unreachable!(
        "needs a call-count/injection seam into gc_sim::rollback_session; see the ignore reason"
    );
}

#[test]
#[ignore = "cannot be forced without a src/ change: the Lua case monkeypatches \
    rollback_session.current_snapshot to count calls; match_driver.rs (src, out of scope for this \
    pass) calls gc_sim::rollback_session::current_snapshot directly as a free function with no \
    injection seam -- report as needing a pub seam, not a test-only workaround"]
fn costs_no_extra_snapshot_work_when_a_peer_authors_only_its_control_slot() {
    unreachable!(
        "needs a call-count/injection seam into gc_sim::rollback_session; see the ignore reason"
    );
}

// #237. The driver used to terminate at *present* full time, leaving up to
// DELAY_TICKS of the match unconfirmed at the moment it reported the result.
// These pin the settle phase that closes it.

#[test]
fn settles_the_final_boundary_before_completing_under_clean_delivery() {
    for mode in [MatchMode::OneVOne, MatchMode::TwoVTwo, MatchMode::FourVFour] {
        let mut state = harness(
            mode,
            DriverHarnessOptions {
                duration: Some(SETTLE_DURATION),
                ..Default::default()
            },
        );
        drive(
            &mut state,
            full_time_boundary_probe() + 20,
            |_| true,
            Some(moving_sample),
        );
        assert_eq!(
            assert_settled(&state, mode.wire_str()),
            full_time_boundary_probe()
        );
        for (index, driver) in state.drivers.iter().enumerate() {
            // Clean delivery confirms ahead of the present, so settling is
            // all but free for a guest: one step behind the fan-out that
            // carries the final row to it, plus one more for the host batch
            // that reports the host's own confirmation back.
            let allowed = 2;
            let steps = match_driver::diagnostics(driver).settle_steps;
            assert!(
                steps <= allowed,
                "peer {} settled slowly under clean delivery in {}: {} steps",
                index + 1,
                mode.wire_str(),
                steps
            );
        }
    }
}

// The regression test. The pre-existing full-time coverage runs bursty
// delivery *up to* full time, which is why this was missed: the burst has to
// straddle the final whistle for peers to stop at different confirmation
// depths.
#[test]
fn completes_with_an_agreed_final_hash_under_a_burst_across_full_time() {
    for mode in [MatchMode::OneVOne, MatchMode::TwoVTwo, MatchMode::FourVFour] {
        let mut state = harness(
            mode,
            DriverHarnessOptions {
                duration: Some(SETTLE_DURATION),
                ..Default::default()
            },
        );
        drive(
            &mut state,
            full_time_boundary_probe() + 90,
            |step| step < full_time_step() - 6 || step > full_time_step() + 4,
            Some(moving_sample),
        );
        assert_settled(&state, mode.wire_str());
        let mut rollbacks = 0;
        let mut settle_steps = 0;
        for driver in &state.drivers {
            let diagnostics = match_driver::diagnostics(driver);
            rollbacks += diagnostics.rollback_count;
            settle_steps += diagnostics.settle_steps;
        }
        assert!(
            rollbacks > 0,
            "the burst never corrected anything in {}",
            mode.wire_str()
        );
        // And the burst really did leave a tail to drain, so this is the
        // path under test rather than the clean one in disguise.
        assert!(
            settle_steps > 0,
            "nothing was left to settle in {}",
            mode.wire_str()
        );
    }
}

#[test]
fn does_not_swallow_a_boundary_disagreement_reported_while_settling() {
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            settle_timeout_ticks: Some(40),
            ..Default::default()
        },
    );
    drive(
        &mut state,
        full_time_step() + 1,
        |step| step < full_time_step() - 5,
        Some(moving_sample),
    );
    {
        let driver = &state.drivers[0];
        assert!(
            match_driver::diagnostics(driver).settling,
            "the peer was not settling"
        );
    }

    // A peer disagreeing about a boundary this driver hashed is exactly the
    // report the coordinator forwards. Settling must not make it wait it out.
    let checkpoint = match_driver::checkpoints(&state.drivers[0])
        .into_iter()
        .next()
        .expect("at least one checkpoint");
    {
        let driver = &mut state.drivers[0];
        for _ in 0..match_driver::MAX_HASH_MISMATCHES {
            assert!(!match_driver::observe_checkpoint(
                driver,
                checkpoint.tick,
                "dead0000dead0000"
            ));
        }
        assert_eq!(
            match_driver::status(driver),
            MatchDriverStatus::HashMismatch
        );
        assert_eq!(
            match_driver::terminal(driver)
                .expect("driver reached a terminal")
                .failure,
            Some(CoordinatorNetcodeFailure::Desync)
        );
        assert!(!match_driver::settled(driver));
    }

    // Delivery resumes and the tail would now confirm. A settle phase that
    // completed anyway would have converted a real divergence into a result.
    drive(&mut state, 40, |_| true, Some(moving_sample));
    let driver = &state.drivers[0];
    assert_eq!(
        match_driver::status(driver),
        MatchDriverStatus::HashMismatch
    );
    assert!(!match_driver::settled(driver));
}

#[test]
fn settles_a_genuinely_divergent_peer_without_hiding_the_divergence() {
    // Peer two simulates from a differently seeded boundary zero: every
    // input row still agrees, so every peer confirms every tick and settles,
    // but the states never do. Settling waits for *authority*, never for
    // agreement, so the disagreement survives into the final hash the
    // session acknowledges.
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            divergent_peer: Some(2),
            ..Default::default()
        },
    );
    drive(
        &mut state,
        full_time_boundary_probe() + 20,
        |_| true,
        Some(moving_sample),
    );
    let mut boundary: Option<i64> = None;
    for driver in &state.drivers {
        assert_eq!(match_driver::status(driver), MatchDriverStatus::Completed);
        assert!(match_driver::settled(driver));
        boundary = match_driver::full_time_boundary(driver);
    }
    let boundary = boundary.expect("some driver reached full time");
    let mut hashes = Vec::new();
    for driver in &state.drivers {
        let snapshot = match_driver::snapshot(driver, boundary)
            .snapshot
            .expect("boundary is retained");
        hashes.push(gc_sim::match_snapshot::hash(&snapshot));
    }
    assert_ne!(
        hashes[0], hashes[1],
        "a divergent peer settled onto an agreed final hash"
    );
    // The driver's own comparison would fire on the same evidence: the
    // boundaries it published during play already disagree.
    let divergent_checkpoint = match_driver::checkpoints(&state.drivers[1])
        .into_iter()
        .next()
        .expect("the divergent peer published at least one checkpoint");
    assert!(!match_driver::observe_checkpoint(
        &mut state.drivers[2],
        divergent_checkpoint.tick,
        &divergent_checkpoint.hash,
    ));
}

#[test]
fn ends_a_settle_nobody_can_finish_with_a_bounded_typed_reason() {
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            settle_timeout_ticks: Some(10),
            ..Default::default()
        },
    );
    // Delivery stops before full time and never resumes: every peer reaches
    // the final tick on predicted rows and none of them can ever confirm it.
    drive(
        &mut state,
        full_time_boundary_probe() + 60,
        |step| step < full_time_step() - 5,
        Some(moving_sample),
    );
    for (index, driver) in state.drivers.iter().enumerate() {
        let status = match_driver::status(driver);
        assert_eq!(
            status,
            MatchDriverStatus::SettleTimeout,
            "peer {} did not time out",
            index + 1
        );
        // Typed, and emphatically not the desync a healthy match used to get.
        assert_ne!(status, MatchDriverStatus::HashMismatch);
        let terminal = match_driver::terminal(driver).expect("driver reached a terminal");
        assert_eq!(
            terminal.failure,
            Some(CoordinatorNetcodeFailure::InputChannel)
        );
        assert_eq!(terminal.tick, match_driver::full_time_boundary(driver));
        assert!(!match_driver::settled(driver));
        assert_eq!(match_driver::diagnostics(driver).settle_steps, 10);
    }

    // No hidden progress after the settle phase ends, same as every other
    // terminal status.
    let driver = &mut state.drivers[0];
    let before = match_driver::diagnostics(driver);
    let batch = match_driver::advance(driver, None);
    assert_eq!(batch.outputs.len(), 0);
    assert_eq!(batch.sent_packets, 0);
    assert_eq!(batch.status, MatchDriverStatus::SettleTimeout);
    let after = match_driver::diagnostics(driver);
    assert_eq!(after.present_input_tick, before.present_input_tick);
    assert_eq!(after.confirmed_input_tick, before.confirmed_input_tick);
    assert_eq!(after.settle_steps, before.settle_steps);
}

#[test]
fn bounds_the_settle_phase_in_wall_clock_as_well_as_in_ticks() {
    // One second of monotonic time per reading, so a caller whose frames
    // have stopped arriving at 60 Hz cannot stretch a bounded number of
    // steps into an unbounded wait.
    let now = Rc::new(RefCell::new(0.0));
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            settle_timeout_ticks: Some(10000),
            settle_timeout_seconds: Some(2.0),
            clock: Some(now),
            ..Default::default()
        },
    );
    drive(
        &mut state,
        full_time_boundary_probe() + 60,
        |step| step < full_time_step() - 5,
        Some(moving_sample),
    );
    for (index, driver) in state.drivers.iter().enumerate() {
        assert_eq!(
            match_driver::status(driver),
            MatchDriverStatus::SettleTimeout,
            "peer {}",
            index + 1
        );
        let detail = match_driver::terminal(driver)
            .expect("driver reached a terminal")
            .detail;
        assert!(
            detail.contains("seconds"),
            "the wall-clock bound was not the one that fired: {detail}"
        );
        // Far short of the tick bound, which is the point.
        assert!(match_driver::diagnostics(driver).settle_steps < 10);
    }
}

#[test]
fn re_publishes_the_tail_while_settling_and_simulates_nothing() {
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            settle_timeout_ticks: Some(40),
            ..Default::default()
        },
    );
    drive(
        &mut state,
        full_time_step() + 1,
        |step| step < full_time_step() - 5,
        Some(moving_sample),
    );
    for (index, driver) in state.drivers.iter_mut().enumerate() {
        let before = match_driver::diagnostics(driver);
        assert!(before.settling, "peer {} was not settling", index + 1);
        assert_eq!(before.status, MatchDriverStatus::Active);
        let batch = match_driver::advance(driver, None);
        // Nothing is simulated after full time, ever.
        assert_eq!(
            batch.outputs.len(),
            0,
            "peer {} simulated after full time",
            index + 1
        );
        let after = match_driver::diagnostics(driver);
        assert_eq!(after.present_input_tick, before.present_input_tick);
        assert_eq!(
            after.present_input_tick,
            before
                .full_time_boundary
                .expect("settling peer has a full time boundary")
        );
        // But the last authored window keeps going out, which is how a peer
        // that lost the tail can still receive it.
        assert!(
            batch.sent_packets > 0 || index == 0,
            "settling peer {} stopped re-publishing its tail",
            index + 1
        );
    }
    // The host publishes through its own collector, so its re-sends leave on
    // the canonical batch a step later rather than immediately.
    let host_batch = match_driver::advance(&mut state.drivers[0], None);
    assert!(
        host_batch.sent_packets > 0,
        "a settling host stopped fanning out authority"
    );
}

// #241, the mid-match half. Confirmation could stop advancing permanently
// with nothing raised at the time: the retained floor slid past a tick that
// never got its eighth row, that tick's authority was deleted, and because
// confirmation only advances from `confirmed_tick + 1` it could never cross
// the hole again. The reactive `late_input` check cannot see it, because it
// fires on a row that *arrives* below the floor and this is a row that never
// arrives at all.
#[test]
fn reports_a_stalled_confirmation_at_the_step_it_becomes_permanent() {
    // Long enough that the retained floor can outrun a hole without full
    // time arriving first and turning this into a settle question.
    let mut state = harness(
        MatchMode::TwoVTwo,
        DriverHarnessOptions {
            duration: Some(2.0),
            ..Default::default()
        },
    );
    // Delivery stops for good well before full time. Every peer keeps
    // simulating on predicted rows, and thirty steps later the floor passes
    // the first tick that never became authoritative.
    drive(&mut state, 90, |step| step < 12, Some(moving_sample));
    for (index, driver) in state.drivers.iter().enumerate() {
        let status = match_driver::status(driver);
        assert_eq!(
            status,
            MatchDriverStatus::ConfirmationStalled,
            "peer {} did not report its stall",
            index + 1
        );
        // Distinct from both statuses that used to absorb this.
        assert_ne!(status, MatchDriverStatus::SettleTimeout);
        assert_ne!(status, MatchDriverStatus::HashMismatch);
        let terminal = match_driver::terminal(driver).expect("driver reached a terminal");
        assert_eq!(terminal.failure, Some(CoordinatorNetcodeFailure::LateInput));
        let diagnostics = match_driver::diagnostics(driver);
        // At the stall, not at the whistle: full time was never reached, so
        // the settle phase never even opened.
        assert_eq!(diagnostics.full_time_boundary, None);
        assert_eq!(diagnostics.settle_steps, 0);
        // And it names the tick that never became authoritative.
        assert_eq!(terminal.tick, Some(diagnostics.confirmed_input_tick + 1));
        assert!(
            diagnostics.retained_floor_tick > diagnostics.confirmed_input_tick + 1,
            "peer {} reported a stall it was not in",
            index + 1
        );
    }
}

#[test]
#[ignore = "cannot be forced without a src/ change: the Lua case monkeypatches \
    rollback_session.apply_authoritative_batch to force an `outside_window` rejection; \
    match_driver.rs (src, out of scope for this pass) calls \
    gc_sim::rollback_session::apply_authoritative_batch directly as a free function with no \
    injection seam -- report as needing a pub seam, not a test-only workaround"]
fn maps_a_rejected_over_window_batch_onto_late_input_unreachable_by_design() {
    unreachable!(
        "needs a call-count/injection seam into gc_sim::rollback_session; see the ignore reason"
    );
}

// #241, the fan-out half. One host batch is meant to carry the full seven-row
// redundancy window for every slot. Relaying only what arrived on this
// transport tick spent far less of it: a slot whose author's bundle was lost
// or delayed contributed nothing at all, so the guest-to-host leg's losses
// multiplied into the host-to-guest leg, which has no retransmission.
#[test]
fn fans_out_a_full_redundancy_window_for_a_slot_that_sent_nothing() {
    let sent: Rc<RefCell<Vec<TransportMessage>>> = Rc::new(RefCell::new(Vec::new()));
    let recorded = sent.clone();
    let wrap_host: WrapHostTransport = Box::new(move |inner| {
        let recorded = recorded.clone();
        Box::new(HookedTransport {
            inner,
            hooks: TransportHooks {
                on_broadcast: Some(Box::new(move |channel, message| {
                    if channel == TransportChannel::Input {
                        recorded.borrow_mut().push(message.clone());
                    }
                })),
                ..Default::default()
            },
        })
    });
    let mut state = harness(
        MatchMode::FourVFour,
        DriverHarnessOptions {
            wrap_host_transport: Some(wrap_host),
            ..Default::default()
        },
    );
    // Warm every slot: each guest has published a full window and the host
    // has accepted it.
    run(&mut state, 12, &[]);
    assert!(
        !sent.borrow().is_empty(),
        "the host never fanned anything out"
    );

    // Now only the host advances. The first step drains what the last pump
    // delivered; the second has no guest traffic at all, which is exactly
    // the shape a lost or delayed bundle produces -- here for seven slots at
    // once rather than one.
    let _ = match_driver::advance(&mut state.drivers[0], None);
    state.session.host_transport.pump();
    let before = sent.borrow().len();
    let _ = match_driver::advance(&mut state.drivers[0], None);
    assert_eq!(
        sent.borrow().len(),
        before + 1,
        "the host did not fan out a batch"
    );

    let fixture_manifest = protocol_fixture::manifest(Some(MatchMode::FourVFour));
    let context = input_protocol::DecodeContext {
        session_id: fixture_manifest
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string(),
        manifest_id: protocol::manifest_id(&fixture_manifest),
        sender_id: match_driver_fixture::HOST_PEER_ID.to_string(),
    };
    let last = sent.borrow().last().expect("a batch was recorded").clone();
    let packet =
        input_protocol::decode(&last.payload, &context).expect("a canonical host batch decodes");
    let mut per_slot: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for row in &packet.rows {
        *per_slot.entry(row.slot_index).or_insert(0) += 1;
    }
    for slot_index in 1..=input_frame::SLOT_COUNT {
        assert_eq!(
            per_slot.get(&slot_index).copied().unwrap_or(0),
            input_protocol::RETAINED_ROWS,
            "slot {slot_index} was not fanned out with a full window"
        );
    }
    // A full window for every slot, with the repair headroom left unspent:
    // no guest is behind here, so nothing has asked for a repair.
    assert_eq!(
        packet.rows.len() as i64,
        input_frame::SLOT_COUNT * input_protocol::RETAINED_ROWS
    );
    assert!((packet.rows.len() as i64) < input_protocol::MAX_HOST_ROWS);
}

// #241, the tail half. The host is the star's only relay and, as sequencer,
// structurally the first peer to confirm the final boundary -- so
// "confirmed, therefore done" made it leave first every time, and a guest
// still missing a tail row could never obtain it afterwards.
#[test]
fn keeps_the_host_relaying_until_its_guests_have_stopped_asking() {
    for mode in [MatchMode::TwoVTwo, MatchMode::FourVFour] {
        let mut state = harness(
            mode,
            DriverHarnessOptions {
                duration: Some(SETTLE_DURATION),
                ..Default::default()
            },
        );
        let mut left: Vec<Option<i64>> = vec![None; state.drivers.len()];
        for step in 1..=(full_time_boundary_probe() + 90) {
            let current = state.step;
            for (index, driver) in state.drivers.iter_mut().enumerate() {
                let sample = moving_sample(current, index as i64 + 1);
                let _ = match_driver::advance(driver, Some(sample));
                if left[index].is_none()
                    && match_driver::status(driver) != MatchDriverStatus::Active
                {
                    left[index] = Some(step);
                }
            }
            // A burst straddling full time, so there is a tail to drain and
            // the guests really are still asking when the host confirms.
            if current < full_time_step() - 6 || current > full_time_step() + 4 {
                state.session.host_transport.pump();
            }
            state.step = current + 1;
        }
        let host_left =
            left[0].unwrap_or_else(|| panic!("the host never left in {}", mode.wire_str()));
        for index in 1..state.drivers.len() {
            assert_eq!(
                match_driver::status(&state.drivers[index]),
                MatchDriverStatus::Completed,
                "guest {} did not complete in {}",
                index + 1,
                mode.wire_str()
            );
            let guest_left = left[index]
                .unwrap_or_else(|| panic!("guest {} never left in {}", index + 1, mode.wire_str()));
            assert!(
                host_left >= guest_left,
                "the host left the star before guest {} in {}",
                index + 1,
                mode.wire_str()
            );
        }
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Completed
        );
        assert!(match_driver::settled(&state.drivers[0]));
    }
}

// #255. The exact case the retired quiet count used to decide: a peer that
// reported itself behind and *then* fell silent. The host now keeps relaying
// for that peer until the settle deadline: this pins that the host waits
// *all* of it and then leaves with a typed terminal rather than hanging, and
// that a silent straggler still gets its own typed terminal too.
#[test]
fn keeps_relaying_for_a_peer_that_reported_behind_and_then_went_silent() {
    let settle_ticks = 20i64;
    // Blocked inbound authority a few steps before full time, so the guest
    // cannot confirm the tail and every bundle it sends says so.
    let blackout_from = full_time_step() - 8;
    // Silent from the first settle step onward, so the host's only evidence
    // about this peer is the stale report it already holds.
    let silent_from = full_time_step() + 1;
    let step = Rc::new(Cell::new(0i64));

    let filter_step = step.clone();
    let wrap_guest: WrapGuestTransport = Box::new(move |index, inner| {
        if index != 2 {
            return inner;
        }
        let poll_step = filter_step.clone();
        let send_step = filter_step.clone();
        Box::new(HookedTransport {
            inner,
            hooks: TransportHooks {
                poll_batch_filter: Some(Box::new(move |messages| {
                    if poll_step.get() < blackout_from {
                        return messages;
                    }
                    messages
                        .into_iter()
                        .filter(|entry| entry.message.kind != TransportMessageType::Input)
                        .collect()
                })),
                send_override: Some(Box::new(move |_, channel, _| {
                    if channel == TransportChannel::Input && send_step.get() >= silent_from {
                        // The wire accepted it and the network ate it: the
                        // host hears nothing further from this peer.
                        Some(Ok(true))
                    } else {
                        None
                    }
                })),
                ..Default::default()
            },
        })
    });

    let mut state = harness(
        MatchMode::OneVOne,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            settle_timeout_ticks: Some(settle_ticks),
            wrap_guest_transport: Some(wrap_guest),
            ..Default::default()
        },
    );

    let mut reported_at_silence: Option<i64> = None;
    let total_steps = full_time_boundary_probe() + settle_ticks + 40;
    for _ in 0..total_steps {
        let current = state.step;
        step.set(current);
        if current == silent_from {
            reported_at_silence =
                Some(match_driver::diagnostics(&state.drivers[1]).confirmed_input_tick);
        }
        for (index, driver) in state.drivers.iter_mut().enumerate() {
            let sample = moving_sample(current, index as i64 + 1);
            let _ = match_driver::advance(driver, Some(sample));
        }
        state.session.host_transport.pump();
        state.step = current + 1;
    }

    // The premise: the guest really was behind the final boundary at the
    // moment it stopped speaking, so the host is holding a report that says
    // so. Without that this test would pass for the wrong reason.
    let boundary =
        match_driver::full_time_boundary(&state.drivers[0]).expect("the host reached full time");
    let reported = reported_at_silence.expect("captured a report at the silence step");
    assert!(
        reported + 1 < boundary,
        "the guest was not behind when it went silent: {reported} vs {boundary}"
    );

    // The host waited the whole phase out rather than four silent steps, and
    // still left -- completed, because its own final boundary is confirmed.
    let host = match_driver::diagnostics(&state.drivers[0]);
    assert_eq!(host.status, MatchDriverStatus::Completed);
    assert!(match_driver::settled(&state.drivers[0]));
    assert_eq!(
        host.settle_steps, settle_ticks,
        "the host stopped relaying before the deadline"
    );

    // And the straggler is bounded by the same deadline, with the typed
    // terminal it would have had either way.
    let guest = match_driver::diagnostics(&state.drivers[1]);
    assert_eq!(guest.status, MatchDriverStatus::SettleTimeout);
    assert_eq!(
        guest.terminal.expect("guest reached a terminal").failure,
        Some(CoordinatorNetcodeFailure::InputChannel)
    );
    assert_eq!(guest.settle_steps, settle_ticks);
}

// #243, the repair half. Blind redundancy re-sends a row for seven transport
// ticks and then stops, so a guest that loses every one of those seven can
// never obtain that row from the ordinary fan-out again. The bundle it
// already sends every tick now reports where its confirmation actually is,
// so the host can aim a re-send at the gap instead of guessing.
#[test]
fn repairs_a_guest_whose_hole_has_aged_out_of_the_redundancy_window() {
    let blackout_from = 14i64;
    let blackout_through = 26i64;
    let step = Rc::new(Cell::new(0i64));
    let repaired_ticks: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(Vec::new()));

    let fixture_manifest = protocol_fixture::manifest(Some(MatchMode::FourVFour));
    let manifest_id = protocol::manifest_id(&fixture_manifest);
    let session_id = fixture_manifest
        .get("session_id")
        .and_then(Value::as_str)
        .expect("fixture manifest has a session id")
        .to_string();
    let host_peer_id = match_driver_fixture::HOST_PEER_ID.to_string();

    let guest_step = step.clone();
    let wrap_guest: WrapGuestTransport = Box::new(move |index, inner| {
        if index != 8 {
            return inner;
        }
        let guest_step = guest_step.clone();
        Box::new(HookedTransport {
            inner,
            hooks: TransportHooks {
                poll_batch_filter: Some(Box::new(move |messages| {
                    let current = guest_step.get();
                    if current < blackout_from || current > blackout_through {
                        return messages;
                    }
                    messages
                        .into_iter()
                        .filter(|entry| entry.message.kind != TransportMessageType::Input)
                        .collect()
                })),
                ..Default::default()
            },
        })
    });

    let repaired_for_wrap = repaired_ticks.clone();
    let wrap_host: WrapHostTransport = Box::new(move |inner| {
        let repaired = repaired_for_wrap.clone();
        let manifest_id = manifest_id.clone();
        let session_id = session_id.clone();
        let host_peer_id = host_peer_id.clone();
        Box::new(HookedTransport {
            inner,
            hooks: TransportHooks {
                on_broadcast: Some(Box::new(move |channel, message| {
                    if channel != TransportChannel::Input {
                        return;
                    }
                    let context = input_protocol::DecodeContext {
                        session_id: session_id.clone(),
                        manifest_id: manifest_id.clone(),
                        sender_id: host_peer_id.clone(),
                    };
                    if let Ok(packet) = input_protocol::decode(&message.payload, &context) {
                        let newest = packet
                            .rows
                            .last()
                            .expect("a canonical host batch is never empty")
                            .tick;
                        let oldest = packet.rows[0].tick;
                        if newest - oldest > input_protocol::HISTORY_ROWS {
                            repaired.borrow_mut().push(oldest);
                        }
                    }
                })),
                ..Default::default()
            },
        })
    });

    let mut state = harness(
        MatchMode::FourVFour,
        DriverHarnessOptions {
            wrap_guest_transport: Some(wrap_guest),
            wrap_host_transport: Some(wrap_host),
            ..Default::default()
        },
    );

    let mut stalled_during: Option<i64> = None;
    for _ in 0..110 {
        advance(&mut state, &[]);
        step.set(state.step);
        if state.step == blackout_through + 2 {
            stalled_during =
                Some(match_driver::diagnostics(&state.drivers[7]).confirmed_input_tick);
        }
    }

    // The blackout was wider than the window, so the guest is genuinely
    // behind the peers that kept receiving. Without that this test would
    // pass on a repair that never had anything to repair.
    let reference = match_driver::diagnostics(&state.drivers[1]).confirmed_input_tick;
    let stalled = stalled_during.expect("captured a reading during the blackout");
    assert!(
        stalled < reference,
        "the blackout did not put the guest behind its peers"
    );
    // The host spent rows on a tick older than blind redundancy would ever
    // re-send: that only happens when a guest reported where it was stuck.
    assert!(
        !repaired_ticks.borrow().is_empty(),
        "the host never fanned out a repaired tick"
    );
    // And the point of all of it: the guest confirms past the hole instead
    // of freezing at it, so no peer reaches `confirmation_stalled`.
    for (index, driver) in state.drivers.iter().enumerate() {
        assert_ne!(
            match_driver::status(driver),
            MatchDriverStatus::ConfirmationStalled,
            "peer {} stalled its confirmation",
            index + 1
        );
    }
    assert!(
        match_driver::diagnostics(&state.drivers[7]).confirmed_input_tick > stalled,
        "the repaired guest never confirmed past its hole"
    );
}

// #243, the settle half, and #241's tail stall in the other direction. A
// guest's authored tail exists nowhere else until the host has it, so a
// guest that leaves on "my own boundary is confirmed" can take rows with it
// that every other peer -- the host included -- is still missing.
#[test]
fn keeps_a_guest_re_publishing_until_the_host_has_confirmed_its_tail() {
    let mut state = harness(
        MatchMode::FourVFour,
        DriverHarnessOptions {
            duration: Some(SETTLE_DURATION),
            ..Default::default()
        },
    );
    let host_blind_until = full_time_step() + 12;
    for _ in 0..(full_time_boundary_probe() + 90) {
        let current = state.step;
        for (index, driver) in state.drivers.iter_mut().enumerate() {
            let sample = moving_sample(current, index as i64 + 1);
            let _ = match_driver::advance(driver, Some(sample));
        }
        // A burst on the guest-to-host leg straddling full time: the host
        // stops hearing one author exactly when its last rows are authored.
        if current < full_time_step() - 8 || current > host_blind_until {
            state.session.host_transport.pump();
        }
        state.step = current + 1;
    }
    for (index, driver) in state.drivers.iter().enumerate() {
        assert_eq!(
            match_driver::status(driver),
            MatchDriverStatus::Completed,
            "peer {} did not complete",
            index + 1
        );
        assert!(
            match_driver::settled(driver),
            "peer {} never settled",
            index + 1
        );
    }
}

#[test]
fn reports_a_lost_transport_as_a_typed_terminal_status() {
    let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
    run(&mut state, 4, &[]);
    let guest_id = state.session.guest_peer_ids[0].clone();
    state
        .session
        .host_transport
        .close_peer(&guest_id, Some("link failed"))
        .expect("closing a connected peer always succeeds");
    state.session.host_transport.pump();
    advance(&mut state, &[]);
    advance(&mut state, &[]);
    assert_ne!(
        match_driver::status(&state.drivers[1]),
        MatchDriverStatus::Active
    );
}
