//! Differential and behavioral tests for the online match driver.
//!
//! ## `fixture.session` and a real connected session
//!
//! `mod online_match_driver` and the free-standing cases below it are built
//! through the `harness(mode, options)` helper defined in this file, which
//! calls `crate::match_driver_fixture::session` (backed by
//! `crate::coordinator`, `crate::protocol`, `crate::protocol_fixture`, and
//! `crate::fake_star`) to build a real connected host+guest session.
//!
//! `match_driver_differential_matches_the_lua_reference` is the highest-value
//! case in this file: it builds the exact `fixture.session("1v1")` scenario
//! the committed reference trace
//! (`tests/fixtures/match_driver_lua_reference.txt`) was captured from —
//! bursty delivery (the star pumps every 5th step only), 90 steps, neutral
//! samples — and asserts `diagnostics()`/`checkpoints()` agree, tick for
//! tick, with the DELIVERY PROTOCOL those frozen reference vectors record:
//! status and the confirmation arithmetic at all 90 steps, every checkpoint
//! tick, and the boundary-zero digest. It deliberately does not compare
//! correction counts or post-kickoff digests, which are simulated values
//! rather than wire facts — see that case's own doc and #520. That reference
//! trace cannot be regenerated — see `tools/lua_reference/README.md` for how
//! it was captured — so a failure here is a finding about this driver's
//! behavior, not a stale fixture to refresh. **One exception:** the
//! boundary-zero digest comparison itself was retired under #536 and now
//! reads a self-recorded baseline, not the frozen trace — see that case's
//! doc comment ("The tick-zero digest, retired under #536") for why and what
//! was lost. Every other comparison the case makes still reads the frozen
//! trace and is still exactly as load-bearing as before.
//!
//! `mod online_match_driver` and the cases after it cover the driver with
//! this file's own `harness`/`advance`/`run`/`assert_agreement`/
//! `assert_confirmed_state` helpers. Known coverage gaps: impaired-delivery/
//! fault-injection scenarios (`wrap_host_transport`/`wrap_guest_transport`),
//! the settle phase, and the combat-phase loop; see this crate's report for
//! what is and is not covered here yet.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

mod support;
use support::online_combat_phases;

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
use gc_sim::rollback_input_history::{RollbackAuthoritativeInput, RollbackInputSource};
use gc_sim::rollback_session;

// ---------------------------------------------------------------------------
// Shared `harness`/`advance`/`run`/`assert_agreement` helpers used by the
// driver tests below, built on `crate::match_driver_fixture::session`'s real
// connected session.
// ---------------------------------------------------------------------------

/// One harness-built match: the underlying session, every driver (host
/// first, then guests), and the current step count.
struct DriverHarness {
    session: MatchDriverFixtureSession,
    /// Host first, then guests in seating order — matches `session.guest_peer_ids`.
    drivers: Vec<match_driver::MatchDriver>,
    step: i64,
}

/// A caller-supplied decorator around the host's own transport endpoint.
type WrapHostTransport =
    Box<dyn Fn(Box<dyn StarTransportAdapter>) -> Box<dyn StarTransportAdapter>>;
/// A caller-supplied decorator around one guest's transport endpoint,
/// keyed by that driver's 1-based position in `DriverHarness.drivers`
/// (host is always `1`).
type WrapGuestTransport =
    Box<dyn Fn(i64, Box<dyn StarTransportAdapter>) -> Box<dyn StarTransportAdapter>>;

/// Options accepted by [`harness`]; only the subset the cases in this file
/// actually use.
#[derive(Default)]
struct DriverHarnessOptions {
    duration: Option<f64>,
    humans: Option<i64>,
    combat: bool,
    /// Boundary zero for every peer; overrides `combat` when set.
    initial_snapshot: Option<MatchSnapshot>,
    hash_interval_ticks: Option<i64>,
    max_rollback_ticks: Option<i64>,
    settle_timeout_ticks: Option<i64>,
    settle_timeout_seconds: Option<f64>,
    /// A shared monotonic-seconds counter. Every driver built from this
    /// harness gets its own closure over the *same* cell, so a case that
    /// bounds the settle phase in wall-clock time can share one clock across
    /// every driver's clock read, ticking the one counter forward.
    clock: Option<Rc<RefCell<f64>>>,
    /// 1-based driver index (host is `1`) whose boundary zero is seeded
    /// differently.
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

/// Builds a real connected session for `mode` and one [`match_driver::MatchDriver`]
/// per seated peer (host first, then guests), applying `options`.
fn harness(mode: MatchMode, options: DriverHarnessOptions) -> DriverHarness {
    let session = match_driver_fixture::session(mode, None, options.humans);
    let snapshot = options.initial_snapshot.clone().unwrap_or_else(|| {
        match_driver_fixture::initial_snapshot(options.duration, options.combat, None)
    });
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

/// Advances every driver in `h` by one step. `samples[i]` is the sample for
/// `drivers[i]`; `None`/short entries default to neutral.
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

/// Calls [`advance`] `steps` times with the same `samples`, returning the
/// last batch.
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

/// Asserts every checkpoint two drivers both hashed agrees between them,
/// boundary hash and live-slot map alike.
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

/// Asserts every peer agrees on state at the confirmed (not merely present)
/// boundary, since that is where authority is complete rather than
/// predicted.
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

/// Builds a valid quantized [`InputSample`] from `options`.
fn sample(options: input_frame::InputSampleOptions) -> InputSample {
    input_frame::new_sample(options).expect("a valid quantized sample")
}

/// Impaired delivery: the star only drains every `period` steps, so every
/// peer predicts the rows it has not received and corrects them in one
/// burst.
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
/// instead of one prediction repeats for free.
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
/// `deliver` allows, and stopping once no driver is still active.
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
/// `late_input`.
const SETTLE_DURATION: f64 = 24.0 / 60.0;

/// Which tick full time lands on is `sim.match`'s countdown to decide, not
/// arithmetic on the duration. Probed once, lazily, and cached.
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
/// reaches [`full_time_boundary_probe`].
fn full_time_step() -> i64 {
    full_time_boundary_probe() - 1
}

/// Asserts every peer completed through the settle phase, on the same final
/// boundary, with every tick of the match authoritative and the same hash
/// captured there.
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
        if let Some(hook) = &mut self.hooks.send_override
            && let Some(result) = hook(peer_id, channel, &message)
        {
            return result;
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
    // No `digest` field: the only comparison that used to read it (the
    // tick-zero boundary digest) was retired under #536 to
    // `BOUNDARY_ZERO_BASELINE_HASH` above, and nothing else in this file
    // reads a checkpoint digest off the fixture. The fixture's `hash` rows
    // still carry a fifth column — `parse_fixture` below just stops keeping
    // it. See `match_driver_differential_matches_the_lua_reference`'s doc.
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
            assert_eq!(
                fields.len(),
                5,
                "fixture hash row must keep its 5-column shape, digest column included: {line}"
            );
            hashes.push(ExpectedHash {
                step: fields[1].parse().expect("fixture hash step is an integer"),
                peer: fields[2].to_string(),
                tick: fields[3].parse().expect("fixture hash tick is an integer"),
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

fn canonical_digest(digest: &str) -> bool {
    digest.len() == 16
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `match_snapshot::hash` of the boundary-zero (kickoff) checkpoint this
/// file's `fixture.session("1v1")` scenario seeds every peer with, recorded
/// from THIS build.
///
/// Self-recorded, NOT the retired Lua digest —
/// `match_driver_differential_matches_the_lua_reference`'s own doc comment
/// ("The tick-zero digest, retired under #536") has the full account of why
/// and what was lost. `fixtures/match_driver_lua_reference.txt` still holds
/// the original Lua-captured value (`hash 10 host 0 4bec679c4f6b7769` /
/// `hash 10 guest 0 4bec679c4f6b7769`), kept unmodified as the historical
/// record; nothing reads it anymore.
///
/// Re-recorded by reading this assertion's own failure output — a single
/// value, so no separate `#[ignore]`d recorder is warranted the way a
/// multi-line baseline elsewhere in this workspace needs one. Re-record only
/// when a deliberate, reviewed change to `match_snapshot`'s canonical wire
/// schema or `match_driver_fixture::initial_snapshot`'s kickoff scenario
/// moves this value — never to clear a check that surprised you.
///
/// Re-recorded for #489: `match_snapshot::VERSION` bumps 12 -> 13 for the new
/// `action` field on every serialized `MatchPlayer`, which is on the
/// boundary-zero kickoff checkpoint like any other tick.
const BOUNDARY_ZERO_BASELINE_HASH: &str = "89b63e968ec22b9e";

/// See the module doc: reproduces the `fixture.session("1v1")` scenario
/// driven through `run_bursty(state, 90, 5)` with neutral samples, and
/// asserts the Rust driver reaches the identical DELIVERY PROTOCOL recorded
/// in the frozen reference vectors, tick for tick, at every one of the 90
/// steps and every one of the 9 checkpoints.
///
/// ## What "delivery protocol" excludes, and why (#520)
///
/// This case used to additionally assert the reference's `correction` COUNT
/// at every step, its `rollback` count, and its boundary DIGEST at every
/// checkpoint. Those are simulated values wearing a protocol test's clothes:
///
/// - `correction_count` counts authoritative input samples that differed
///   from the sample the peer had predicted, and the samples for the
///   non-human slots come from `slot_input::materialize` running the AI over
///   the current match state. Change the physics and the AI authors
///   different samples, so the count moves while nothing about the protocol
///   has. `rollback_count` moves with it, for the same reason.
/// - a checkpoint digest at tick 10 or later hashes stepped state. It is the
///   trajectory, in one word.
///
/// So a deliberate gameplay change turned this red with every claim it
/// exists to protect intact. What it asserts against the reference now is
/// what the reference is actually evidence of:
///
/// - `status`, `present_input_tick`, `confirmed_input_tick` and
///   `confirmed_output_tick` at all 90 steps for both peers — the
///   confirmation arithmetic of a star that pumps only every 5th step;
/// - the correction/rollback PROTOCOL, asserted on the reference's own rows
///   and on the live run alike: neither counter may move on a step where no
///   authority arrived (the star pumps every 5th step, so only steps
///   `6, 11, ... 86` may move them), neither may go backwards, a step
///   corrects if and only if it rolls back, and both must have moved by the
///   end. The magnitudes are not asserted, and the reference's own guest
///   column shows why they cannot be: it corrects at 6 and then not again
///   until 31, because for those five pumps the host's AI-authored samples
///   happened to be exactly what the guest had predicted — a fact about the
///   simulation, not about the wire;
/// - every checkpoint TICK, so the publication cadence stays pinned;
/// - the tick-ZERO checkpoint digest against [`BOUNDARY_ZERO_BASELINE_HASH`]
///   (see that constant's doc — **retired under #536, no longer Lua**);
/// - and for every later checkpoint: host and guest agree with EACH OTHER
///   (the desync-detection claim the digests are carried for), the digest is
///   canonical, and consecutive checkpoints differ.
///
/// ## The tick-zero digest, retired under #536 — schema-coupled, not
/// ## trajectory-coupled
///
/// Boundary zero is the kickoff snapshot every peer was seeded with, taken
/// before any tick steps — so, like the rest of this case, it was never
/// trajectory-coupled the way a stepped digest would be. But it still pins
/// `match_snapshot::hash`'s canonical wire *schema*, and #536 found that a
/// `match_snapshot::VERSION` bump reddens a zero-tick digest exactly as
/// readily as a stepped one: "not trajectory-coupled" was mistaken for
/// "immune to a schema change" here, the same gap `match_snapshot_differential.rs`
/// hit for the same reason — see that file's module doc for the fuller
/// account and `tools/lua_reference/README.md`'s new third category.
///
/// **Owner decision, 2026-08-14 (#536): retire per the documented
/// procedure.** Superseded by `0c94cee` (phase 1 of #531), which bumped
/// `match_snapshot::VERSION` 11 → 12 and added the `pass_intent` field to
/// every serialized `MatchPlayer`. Last commit at which this case's
/// boundary-zero comparison held: `2ce0ca0` (the direct parent of
/// `0c94cee`) — verified green there in a scratch worktree, not assumed.
///
/// `tests/fixtures/match_driver_lua_reference.txt` is kept, unmodified and
/// exactly as load-bearing as before for every OTHER claim this case makes
/// (status, confirmation arithmetic, the correction/rollback protocol,
/// checkpoint cadence, mutual host/guest agreement) — none of those read the
/// digest column at all, so none of them are affected by this retirement.
/// Only the single boundary-zero digest comparison moved, from the
/// fixture's `hash 10 host 0 …` / `hash 10 guest 0 …` rows to
/// [`BOUNDARY_ZERO_BASELINE_HASH`] below.
///
/// **The replacement is weaker.** The retired comparison was
/// cross-implementation evidence that `match_snapshot::hash`'s algorithm and
/// serialization order agreed with an independently written Lua encoder.
/// [`BOUNDARY_ZERO_BASELINE_HASH`] was recorded from this same build's
/// `match_driver_fixture::initial_snapshot`, so a pass now proves only that
/// this build agrees with a snapshot of itself — it detects change, not a
/// wire bug already present when the value was captured. No independent,
/// oracle-free alternative exists for a single hash's correctness (rule 5 of
/// the retirement procedure), for the same reason `match_snapshot_differential.rs`
/// has none: hashing IS the subject, and there is no second implementation
/// left to disagree with.
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

    // The star pumps on every 5th step, so a peer can only see new
    // authority on the step after one — hand-written from this case's own
    // `if step % 5 == 0 { pump() }` below rather than read off the fixture,
    // because the fixture's correction column is exactly the thing being
    // decoupled from.
    let may_correct = |step: i64| step > 5 && step % 5 == 1;

    // The same protocol asserted on the reference's own rows. If the
    // reference disagreed with the invariant the live run is held to, the
    // invariant would be this test's invention rather than the frozen
    // vector's evidence.
    for label in ["host", "guest"] {
        let mut previous: Option<&ExpectedStep> = None;
        let mut moved = 0;
        for expected in expected_steps.iter().filter(|e| e.peer == label) {
            let (was_rollback, was_correction) =
                previous.map_or((0, 0), |p| (p.rollback, p.correction));
            let rolled = expected.rollback != was_rollback;
            let corrected = expected.correction != was_correction;
            assert_eq!(
                rolled, corrected,
                "reference step {} {label}: rollback and correction must move together",
                expected.step
            );
            assert!(
                expected.rollback >= was_rollback && expected.correction >= was_correction,
                "reference step {} {label}: a counter went backwards",
                expected.step
            );
            if rolled {
                assert!(
                    may_correct(expected.step),
                    "reference step {} {label} corrects on a step no authority arrived on",
                    expected.step
                );
                moved += 1;
            }
            previous = Some(expected);
        }
        assert!(
            moved > 0,
            "the reference records no correction at all for {label}"
        );
    }

    // Per peer: the counters observed at the previous step, and the digests
    // already seen, so the burst shape and checkpoint progress can be
    // asserted across steps rather than at one.
    let mut previous_counters = [(0i64, 0i64), (0i64, 0i64)];
    let mut previous_digest: [Option<String>; 2] = [None, None];
    let mut corrections_observed = [0i64, 0i64];
    let mut checkpoints_compared = 0i64;

    for step in 1..=90i64 {
        let _ = match_driver::advance(&mut host, None);
        let _ = match_driver::advance(&mut guest, None);
        if step % 5 == 0 {
            host_transport.pump();
        }

        for (peer, (driver, label)) in [(&host, "host"), (&guest, "guest")].iter().enumerate() {
            let expected = expected_steps
                .iter()
                .find(|e| e.step == step && &e.peer == label)
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

            // The correction/rollback PROTOCOL, not the counts: see this
            // case's doc for why the magnitudes are the trajectory and this
            // is the wire.
            let (was_rollback, was_correction) = previous_counters[peer];
            let rolled = diagnostics.rollback_count != was_rollback;
            let corrected = diagnostics.correction_count != was_correction;
            assert_eq!(
                rolled, corrected,
                "step {step} {label}: rollback and correction must move together"
            );
            assert!(
                diagnostics.rollback_count >= was_rollback
                    && diagnostics.correction_count >= was_correction,
                "step {step} {label}: a counter went backwards"
            );
            if rolled {
                assert!(
                    may_correct(step),
                    "step {step} {label} corrects on a step no authority arrived on"
                );
                corrections_observed[peer] += 1;
            }
            previous_counters[peer] = (diagnostics.rollback_count, diagnostics.correction_count);
        }

        if step % 10 == 0 {
            let mut digests: Vec<String> = Vec::new();
            for (peer, (driver, label)) in [(&host, "host"), (&guest, "guest")].iter().enumerate() {
                let expected = expected_hashes
                    .iter()
                    .find(|h| h.step == step && &h.peer == label)
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
                assert!(
                    canonical_digest(&newest.hash),
                    "step {step} {label} checkpoint hash is not 16 lowercase hex characters: {}",
                    newest.hash
                );
                if newest.tick == 0 {
                    // Boundary zero is the kickoff snapshot every peer was
                    // seeded with, so this digest is content, not
                    // trajectory — but it is schema-coupled, and the Lua
                    // comparison it used to make (`expected.digest`) is
                    // retired under #536. See this test's own doc comment.
                    assert_eq!(
                        newest.hash, BOUNDARY_ZERO_BASELINE_HASH,
                        "step {step} {label} boundary-zero checkpoint hash"
                    );
                    checkpoints_compared += 1;
                }
                if let Some(previous) = &previous_digest[peer] {
                    assert_ne!(
                        &newest.hash, previous,
                        "step {step} {label} republished the previous checkpoint digest; \
                         the confirmed boundary did not advance"
                    );
                }
                previous_digest[peer] = Some(newest.hash.clone());
                digests.push(newest.hash.clone());
            }
            // The claim the later digests are carried for: both peers
            // simulated the same match and say so at the same tick.
            assert_eq!(
                digests[0], digests[1],
                "step {step}: host and guest disagree on the confirmed boundary"
            );
        }
    }

    // Non-vacuity, in the two places this case could quietly stop asserting:
    // a run that never corrected would satisfy every schedule assertion
    // above, and a boundary-zero digest that was never reached would leave
    // the only cross-language hash comparison unexecuted.
    assert!(
        corrections_observed[0] > 0 && corrections_observed[1] > 0,
        "both peers must have predicted and then corrected at least once; \
         host corrected on {} steps, guest on {}",
        corrections_observed[0],
        corrections_observed[1]
    );
    assert_eq!(
        checkpoints_compared, 2,
        "boundary zero must be compared to the reference once per peer"
    );
}

/// The cases below use the `harness`/`advance`/`run`/`assert_agreement`
/// helpers defined above `mod online_match_driver` (see this file's module
/// doc and `online_match_driver`'s coverage).
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

// The claim the seven scenarios in `converges_a_correction_taken_during_each_combat_phase`
// rest on: none of them starts already in the phase it is named for. A
// fixture that force-set `phase = "windup"` at boundary zero would pin the
// driver's restore path and nothing about the simulation reaching wind-up,
// and the difference is invisible from the scenario's own assertions.
#[test]
fn opens_every_combat_phase_scenario_from_a_ready_combat_state() {
    assert_eq!(online_combat_phases::PHASES.len(), 7);
    for &phase_id in &online_combat_phases::PHASES {
        let scenario = online_combat_phases::scenario(phase_id);
        assert!(matches!(
            scenario.route,
            online_combat_phases::OnlineCombatPhaseRoute::Policy
                | online_combat_phases::OnlineCombatPhaseRoute::CanonicalInput
        ));
        let snapshot = online_combat_phases::boundary_zero(phase_id, None);
        let companion = snapshot.combat.as_ref().expect(phase_id);
        assert_eq!(companion.projectiles.len(), 0, "{phase_id}");
        assert_eq!(companion.events.len(), 0, "{phase_id}");
        let mut equipped = 0;
        for runtime in &companion.players {
            assert_eq!(
                runtime.phase,
                gc_sim::combat_feasibility::CombatActionPhase::Ready,
                "{phase_id}"
            );
            assert_eq!(runtime.forced_state, None, "{phase_id}");
            assert_eq!(runtime.immunity_ticks, 0, "{phase_id}");
            assert_eq!(
                runtime.intent.stage,
                gc_sim::combat_intent::CombatIntentStage::Idle,
                "{phase_id}"
            );
            if let Some(family_id) = runtime.family_id {
                equipped += 1;
                assert!(
                    family_id == scenario.shape.home_family
                        || family_id == scenario.shape.away_family,
                    "{phase_id} equipped an unexpected family: {family_id:?}"
                );
            }
        }
        // Both keepers are protected and slotless, so every other body is
        // armed with the family the scenario declares.
        assert_eq!(equipped, input_frame::SLOT_COUNT, "{phase_id}");
    }
}

/// One case per named combat correction phase; `crate::fault_harness::declare_contingent`
/// names the same 7 phases. Written as 7 distinctly named cases rather than
/// one generic loop, so each is independently countable and independently
/// re-enableable.
///
/// Every one shares the same body: open on the phase's own boundary zero,
/// drive it 1v1 under impaired delivery, and require that at least one
/// correction on every peer resimulated a tick that genuinely ran through the
/// named phase -- and that at least one such tick was resimulated (and
/// agreed) by more than one peer.
///
/// `batch.outputs` carries two kinds of tick and does not label them:
/// `apply_rows` appends the reconciliation's corrected outputs first, then
/// `step_to` appends the ordinary forward tick of the same call. Only the
/// first kind answers the question, so a phase seen on a forward tick must
/// not count -- a corrected tick is always strictly below the present
/// boundary as it stood before the call, because reconciliation restores to
/// the divergence and resimulates back to the *same* present, while a
/// forward tick is the present itself.
mod converges_a_correction_taken_during_each_combat_phase {
    use super::*;

    fn converges_during(phase_id: &str) {
        let scenario = online_combat_phases::scenario(phase_id);
        let snapshot = online_combat_phases::boundary_zero(phase_id, None);
        assert_eq!(snapshot.version, gc_sim::match_snapshot::COMBAT_VERSION);
        // 1v1 is where the AI carries the most of the pitch: three of every
        // human's four owned slots are AI-driven at any instant, so six of
        // the eight slots fight without a human behind them.
        let mut state = harness(
            MatchMode::OneVOne,
            DriverHarnessOptions {
                initial_snapshot: Some(snapshot),
                ..Default::default()
            },
        );
        let first = state.session.freeze.first_input_tick;

        let mut observed = vec![0i64; state.drivers.len()];
        // Input tick -> the first peer to resimulate it in this phase and the
        // hash it landed on, recorded only once that tick is fully
        // authoritative. Below confirmation a tick still holds predicted
        // rows, so peers may legitimately differ there.
        let mut phase_hashes: Vec<(i64, usize, String)> = Vec::new();
        let mut shared = 0i64;

        for step in 0..scenario.steps {
            for (index, driver) in state.drivers.iter_mut().enumerate() {
                // Read before the call: this is the boundary a corrected tick
                // is strictly below and a forward tick starts at.
                let present_before = match_driver::diagnostics(driver).present_input_tick;
                let sample = online_combat_phases::live_sample(phase_id, step, index as i64 + 1);
                let batch = match_driver::advance(driver, Some(sample));
                if batch.rollbacks > 0 {
                    let confirmed = match_driver::diagnostics(driver).confirmed_input_tick;
                    let mut corrected = 0i64;
                    for output in &batch.outputs {
                        // `RollbackTickOutput.tick` is session space.
                        let tick = output.tick + first;
                        if tick >= present_before {
                            // `step_to`'s forward tick, not the correction's.
                            // It proves nothing here.
                            continue;
                        }
                        corrected += 1;
                        let before = match_driver::snapshot(driver, tick).snapshot;
                        let after = match_driver::snapshot(driver, tick + 1).snapshot;
                        if let (Some(before), Some(after)) = (&before, &after) {
                            let events = output.combat_events.as_deref().unwrap_or(&[]);
                            if online_combat_phases::observed(phase_id, before, after, events) {
                                observed[index] += 1;
                                if tick <= confirmed {
                                    let hash = gc_sim::match_snapshot::hash(before);
                                    if let Some(recorded) =
                                        phase_hashes.iter().find(|(t, _, _)| *t == tick)
                                    {
                                        assert_eq!(
                                            hash, recorded.2,
                                            "peers disagreed on the resimulated {phase_id} at tick {tick}"
                                        );
                                        // Only a genuinely cross-peer
                                        // comparison counts: one peer
                                        // resimulating the same tick twice
                                        // agrees with itself for free.
                                        if recorded.1 != index {
                                            shared += 1;
                                        }
                                    } else {
                                        phase_hashes.push((tick, index, hash));
                                    }
                                }
                            }
                        }
                    }
                    // The discriminator cannot quietly stop finding anything:
                    // a reconciliation that changed state re-derived at least
                    // one tick, by definition.
                    assert!(
                        corrected > 0,
                        "peer {} reported a rollback with no corrected tick below the present",
                        index + 1
                    );
                }
            }
            if (step + 1) % scenario.deliver_period == 0 {
                state.session.host_transport.pump();
            }
        }

        for (index, driver) in state.drivers.iter().enumerate() {
            // No terminal at all: a duplicate bundle that differed, or
            // authority from outside a frozen owned set, would have ended
            // this peer as `authority_conflict` / `ownership_violation`
            // rather than left it playing.
            assert_eq!(
                match_driver::status(driver),
                MatchDriverStatus::Active,
                "peer {} did not survive the {phase_id} run",
                index + 1
            );
            assert_eq!(match_driver::terminal(driver), None);
            let diagnostics = match_driver::diagnostics(driver);
            assert!(
                diagnostics.rollback_count > 0,
                "the {phase_id} burst never corrected peer {}",
                index + 1
            );
            assert_eq!(diagnostics.hash_mismatches, 0);
            assert!(
                observed[index] > 0,
                "no correction on peer {} ever resimulated a {phase_id} tick",
                index + 1
            );
            // One boundary is published once. A checkpoint republished under
            // a second hash is the duplicate-authority shape the coordinator
            // would have to arbitrate.
            let mut published: Vec<i64> = Vec::new();
            for checkpoint in match_driver::checkpoints(driver) {
                assert!(
                    !published.contains(&checkpoint.tick),
                    "peer {} published boundary {} twice",
                    index + 1,
                    checkpoint.tick
                );
                published.push(checkpoint.tick);
                assert_eq!(checkpoint.hash.len(), 16);
            }
        }
        assert!(
            shared > 0,
            "no confirmed {phase_id} tick was resimulated by more than one peer"
        );
        assert!(assert_agreement(&state) > 0);
        assert_confirmed_state(&state);
    }

    #[test]
    fn wind_up() {
        converges_during("windup");
    }

    #[test]
    fn guard() {
        converges_during("guard");
    }

    #[test]
    fn contact() {
        converges_during("contact");
    }

    #[test]
    fn projectile_flight() {
        converges_during("projectile_flight");
    }

    #[test]
    fn stagger() {
        converges_during("stagger");
    }

    #[test]
    fn ball_spill() {
        converges_during("ball_spill");
    }

    #[test]
    fn immunity_expiry() {
        converges_during("immunity_expiry");
    }
}

// The evidence behind the `guard` scenario's `CanonicalInput` route, and the
// tripwire that will tell us when it can be promoted to `Policy`.
//
// Every geometry arms the whole home side with `guard` and the whole away
// side with a family that publishes a readable threat, then lets
// `gameplay_ai/combat/v1` play with no human input at all -- and counts the
// same thing a phase scenario counts: corrected ticks a peer resimulated in
// the `guard` phase.
//
// The claim under test is about *rate*, not possibility. When a geometry does
// clear the bar this fails, and the fix is to move `guard` onto the `Policy`
// route in `tests/support/online_combat_phases.rs` -- not to raise the bar.
#[test]
fn finds_no_driver_level_geometry_where_the_policy_guards_often_enough() {
    for geometry in &online_combat_phases::GUARD_PROBE {
        let mut state = harness(
            MatchMode::OneVOne,
            DriverHarnessOptions {
                initial_snapshot: Some(online_combat_phases::guard_probe_boundary_zero(
                    geometry, None,
                )),
                ..Default::default()
            },
        );
        let first = state.session.freeze.first_input_tick;
        let mut observed = vec![0i64; state.drivers.len()];
        let mut threat_ticks = 0i64;
        for step in 1..=geometry.steps {
            for (index, driver) in state.drivers.iter_mut().enumerate() {
                let present_before = match_driver::diagnostics(driver).present_input_tick;
                let batch = match_driver::advance(driver, Some(input_frame::neutral_sample()));
                if batch.rollbacks > 0 {
                    for output in &batch.outputs {
                        let tick = output.tick + first;
                        if tick < present_before {
                            let before = match_driver::snapshot(driver, tick).snapshot;
                            let after = match_driver::snapshot(driver, tick + 1).snapshot;
                            if let (Some(before), Some(after)) = (&before, &after) {
                                let events = output.combat_events.as_deref().unwrap_or(&[]);
                                if online_combat_phases::observed("guard", before, after, events) {
                                    observed[index] += 1;
                                }
                            }
                        }
                    }
                }
            }
            if step % geometry.deliver_period == 0 {
                state.session.host_transport.pump();
            }
            // Away runtimes are indexes 7..10 (1-based); 6 is the protected
            // keeper, so this is `companion.players[6..]` zero-based.
            let snapshot = match_driver::current_snapshot(&state.drivers[0]);
            let companion = snapshot.combat.as_ref().expect("geometry needs combat");
            for runtime in &companion.players[6..] {
                if matches!(
                    runtime.phase,
                    gc_sim::combat_feasibility::CombatActionPhase::Windup
                        | gc_sim::combat_feasibility::CombatActionPhase::Active
                ) {
                    threat_ticks += 1;
                }
            }
            threat_ticks += companion.projectiles.len() as i64;
        }
        // Without a readable threat the policy could not guard even in
        // principle, and a low count here would mean nothing at all.
        assert!(
            threat_ticks > 0,
            "{} never telegraphed a threat, so it probes nothing",
            geometry.id
        );
        let weakest = observed.iter().copied().min().expect("at least one peer");
        assert!(
            weakest < online_combat_phases::GUARD_POLICY_ROUTE_MINIMUM,
            "{} now reaches {weakest} corrected guard ticks on every peer -- promote the guard \
             scenario to the policy route",
            geometry.id
        );
    }
}

// Retired driver-level case: `still_reconciles_if_a_local_insert_ever_reports_a_divergence`.
//
// That case originally forced this path by dynamically reassigning
// `rollback_session.reconcile`/`.add_authoritative_batch` at runtime to
// report a divergence unconditionally. Rust cannot reassign a free function,
// so the question worth answering first is whether `apply_rows(..., arrival
// = false)` in `match_driver.rs` — the guest's own-row insert — can ever see
// `earliest_divergence.is_some()` through the public driver API at all,
// rather than jumping straight to a mock seam.
//
// It cannot, for two independent structural reasons:
//
// 1. A guest's own rows are always authored `DELAY_TICKS` (3) ahead of the
//    tick `step_to` is about to simulate this call (`author_and_send` is
//    called with `input_tick + DELAY_TICKS`, then `step_to` only advances to
//    `input_tick`). `rollback_input_history::add_authoritative` only sets
//    `earliest_divergence` when `history.effective.get(&tick)` is already
//    populated — i.e. the tick was already simulated with some other sample.
//    A tick that far in the future has never been materialized, so the
//    local insert's own row can never trip the guard on itself.
// 2. Every *remote* arrival (`apply_rows(..., arrival = true)`, both the
//    host's canonical batch and a guest's received broadcast) goes through
//    `rollback_session::apply_authoritative_batch`, which calls
//    `add_authoritative_batch` and then unconditionally `reconcile` in the
//    same synchronous call — `reconcile` calls
//    `rollback_input_history::consume_earliest_divergence`, clearing the
//    flag before control ever returns to `apply_rows`. Nothing runs between
//    the two calls, so a divergence a remote arrival opens can never still
//    be pending by the time the next local insert (or anything else) runs.
//
// So `accepted.earliest_divergence.is_some()` on the local-insert branch is
// truly dead by construction, not merely untriggered by the scenarios this
// suite happens to drive — matching the retired case's own claim that real
// traffic never trips it. Forcing it would need a `RollbackOps` trait object
// in `match_driver`'s per-tick rollback path (production structure serving a
// test) purely to fake a state the public API cannot reach; that is the
// wrong trade for a branch this well-proven.
//
// What *can* be covered honestly, and is covered below: the substance the
// retired test protected, "if a batch ever reports a divergence, `reconcile`
// corrects it," tested directly against `rollback_session::reconcile` using
// only its own public API (no driver, no mock). This mirrors
// `applies_one_authoritative_packet_batch_through_exactly_one_reconciliation`
// in `gc-sim/tests/rollback_session.rs`, which already exercises the same
// substance in depth; the case below exists so the coverage this crate's
// test suite lost is visible here too, matched to the exact call the
// driver's local-insert branch makes (`add_authoritative_batch` then a
// conditional `reconcile`).
//
// The remaining piece — "match_driver's local-insert branch would call
// `reconcile` if `earliest_divergence` were ever `Some`" — is wiring around
// a branch that cannot be entered. A `Cell` counter (the pattern
// `MatchDriver::snapshot_captures` established for the sibling retired case
// below) cannot prove that either: it would just read zero forever. Adding
// one would document nothing a code reading doesn't already show, so this
// deliberately does not add one.
#[test]
fn reconcile_corrects_a_real_divergence_that_add_authoritative_batch_reports() {
    let snapshot = match_driver_fixture::initial_snapshot(None, false, None);
    let sources = [RollbackInputSource::Remote; 8];
    let mut session = rollback_session::new(&snapshot, sources, None, None);

    // Simulate a few ticks with no authority at all, so tick 0 is materialized
    // (predicted/neutral) and genuinely "used" before any authoritative row
    // for it exists.
    let _ = rollback_session::step(&mut session).expect("step succeeds");
    let _ = rollback_session::step(&mut session).expect("step succeeds");
    let _ = rollback_session::step(&mut session).expect("step succeeds");

    // Now submit an authoritative sample for the already-used tick 0 that
    // differs from the one the simulation actually consumed there — a real
    // divergence, reported by the same `add_authoritative_batch` call
    // `match_driver::apply_rows`'s local-insert branch makes.
    let moved = input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(90),
        ..Default::default()
    })
    .expect("valid sample");
    let arrivals = [RollbackAuthoritativeInput {
        tick: 0,
        slot_index: 1,
        sample: moved,
    }];
    let accepted = rollback_session::add_authoritative_batch(&mut session, &arrivals)
        .expect("an in-window row is accepted");
    assert_eq!(
        accepted.earliest_divergence,
        Some(0),
        "resubmitting an already-used tick with a different sample should report a divergence"
    );

    // This is exactly the guard `apply_rows` checks before deciding whether
    // to call `reconcile` at all.
    let old_present = rollback_session::diagnostics(&session).present_boundary;
    let result = rollback_session::reconcile(&mut session, false);
    assert!(
        result.changed,
        "reconcile did not correct a divergence it was told about"
    );
    assert_eq!(result.causal_tick, Some(0));
    assert_eq!(result.old_present_boundary, old_present);
    assert_eq!(
        result.new_present_boundary, old_present,
        "reconcile should resimulate back to the same present boundary"
    );
    assert_eq!(
        session.input_history.earliest_divergence, None,
        "reconcile should consume the divergence it corrected"
    );
}

#[test]
fn costs_no_extra_snapshot_work_when_a_peer_authors_only_its_control_slot() {
    // The original version of this assertion replaced
    // `rollback_session.current_snapshot` at runtime and counted calls. Rust
    // cannot reassign a free function, and the alternative — a trait object
    // in this module's per-tick path — would put
    // production structure in service of a test. `MatchDriver::snapshot_captures`
    // counts the same thing as a real diagnostic, which has the side benefit of
    // being readable on a live session, where a test mock never is.
    let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
    let guest = &state.drivers[1];
    assert_eq!(
        match_driver::diagnostics(guest).authored.len(),
        1,
        "4v4 guest should author only its own control slot"
    );

    let before = match_driver::diagnostics(guest).snapshot_captures;
    for _ in 0..8 {
        let _ = match_driver::advance(&mut state.drivers[1], Some(input_frame::neutral_sample()));
    }
    let after = match_driver::diagnostics(&state.drivers[1]).snapshot_captures;
    assert_eq!(
        after - before,
        0,
        "a singleton owned set still paid a capture-and-restore"
    );

    // And the peer that does author AI rows pays it once per step, not more.
    let mut multi = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
    let multi_before = match_driver::diagnostics(&multi.drivers[0]).snapshot_captures;
    for _ in 0..8 {
        let _ = match_driver::advance(&mut multi.drivers[0], Some(input_frame::neutral_sample()));
    }
    let multi_after = match_driver::diagnostics(&multi.drivers[0]).snapshot_captures;
    assert_eq!(
        multi_after - multi_before,
        8,
        "authoring AI rows should cost exactly one capture per step"
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

// Retired driver-level case:
// `maps_a_rejected_over_window_batch_onto_late_input_unreachable_by_design`.
//
// The original version of this case forced the branch by reassigning
// `rollback_session.apply_authoritative_batch` to return a fake
// `outside_window` rejection (case name: "maps a rejected over-window batch
// onto late_input (unreachable by design)"), and its own comment proved the
// branch is unreachable through legitimate traffic: "a row is only offered
// to the history when it is above this peer's confirmation, so a row below
// the floor implies `confirmed + 1 < floor`, which confirmation liveness
// terminates on at the end of the previous step."
//
// That proof carries over to Rust unchanged, and two already-passing cases
// in this file corroborate it directly instead of leaving it as an
// unverified claim:
//
// - `terminates_on_confirmation_liveness_before_a_below_floor_row_can_arrive`
//   drives a guest whose confirmation is pinned while its floor keeps
//   sliding — the only regime where the floor rule could bite — and asserts
//   `retained_floor_tick == confirmed_input_tick + 2` at the moment it
//   terminates: the *first* step the gap could ever open, caught before it
//   opens.
// - `reports_a_stalled_confirmation_at_the_step_it_becomes_permanent`'s own
//   comment states the same conclusion independently: "the reactive
//   `late_input` check cannot see it, because it fires on a row that
//   *arrives* below the floor and this is a row that never arrives at all."
//
// The code reason both empirical results hold: `apply_rows` only ever offers
// `rollback_session::apply_authoritative_batch` rows with `tick > confirmed`
// (see `match_driver.rs`'s `apply_rows`, the `row.tick > confirmed` filter),
// and `detect_confirmation_stall` — called at the end of every active step,
// before the next `apply_rows` call can run — terminates the driver with
// `ConfirmationStalled` the moment `confirmed + 1 < oldest_retained_tick`
// would ever let such a row exist. So by the time any row reaches
// `add_authoritative_batch`, `confirmed + 1 >= oldest_retained_tick` always
// holds and `tick >= confirmed + 1 >= oldest_retained_tick`: the row can
// never be below the floor, and `OutsideWindow` can never come back from a
// live arrival. (The driver's own local-insert rows are also always ahead of
// the floor, for the same DELAY_TICKS reason documented on the retired case
// above, so there is no second path in either that could reach it.)
//
// The mechanism itself — `rollback_input_history`/`rollback_session`
// rejecting a row below the retained floor with `OutsideWindow` — is
// thoroughly covered independently of the driver: see
// `fails_explicitly_at_thirty_one_ticks_late_without_hidden_progress` and
// `attributes_mixed_retained_and_over_window_batches_to_the_actual_late_tick`
// in `gc-sim/tests/rollback_session.rs`, and
// `omp2_rollback_input_history_preflights_complete_authority_batches_before_one_atomic_insertion`
// and its neighbors in `gc-sim/tests/rollback_input_history.rs`.
//
// What is left uncovered by a Rust test is only `apply_rows`'s few-line
// mapping from that error code onto `MatchDriverStatus::LateInput` plus the
// terminal detail/tick — code this proof says a real driver run can never
// reach. A `RollbackOps` trait-object seam could force it, at the cost of a
// trait object in the per-tick rollback path of a determinism-critical
// module, purely to simulate a state the public API cannot produce; that
// cost is not worth paying for a branch this thoroughly proven dead, so it
// is not added. Converting the branch itself to `unreachable!()`/
// `debug_assert!` was considered and rejected too: the original case's own
// comment is explicit that the mapping is "the fallback for a rule this
// driver does not own," kept intentionally so that if `rollback_input_history`'s window rule
// is ever reached by some future call path this driver doesn't yet have, the
// match still degrades gracefully to `late_input` instead of panicking.
// Asserting the branch away would trade that graceful degradation for a
// crash the very first time the proof above stops holding — the opposite of
// what the fallback is for.
#[test]
#[ignore = "retired: provably unreachable through the public driver API (see the doc comment above); the Lua forces it by reassigning rollback_session.apply_authoritative_batch, which Rust cannot do to a free function. The rejection mechanism itself is covered in gc-sim/tests; only the driver's error-code-to-status mapping is uncovered, and it cannot be reached without a mock past the public API."]
fn maps_a_rejected_over_window_batch_onto_late_input_unreachable_by_design() {
    unreachable!(
        "provably unreachable through the public driver API; see the doc comment and ignore reason above"
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
        for (index, driver) in state.drivers.iter().enumerate().skip(1) {
            assert_eq!(
                match_driver::status(driver),
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
