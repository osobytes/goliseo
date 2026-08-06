//! Port of `spec/game/online_fault_harness_spec.lua`.
//!
//! Three `t.describe` blocks:
//!
//! - `"online fault harness"` (12 cases): every case runs a full
//!   `FaultHarness` end to end, now real (`gc_netcode::fault_harness`'s
//!   module doc: construction, the pre-match lifecycle over a real star, the
//!   match itself, and teardown). 11 of the 12 are ported for real, driving
//!   [`gc_netcode::fault_scenarios::run`] exactly the way
//!   `spec/game/online_fault_harness_spec.lua`'s own `run` helper drives
//!   `fault_scenarios.run`. One case —
//!   `publishes_each_confirmed_event_once_and_never_resurrects_a_revoked_one`
//!   — is *not* a pass/fail port of the Lua original: `game.online.match_presentation`
//!   has no Rust port (TypeScript-owned, `v2/README.md` §2), so this port's
//!   `presentation.published_once`/`presentation.no_revoked_survivor`
//!   findings are declared *skipped*, not measured, and this case asserts
//!   exactly that — the same "declared contingent, not silently omitted"
//!   contract [`gc_netcode::fault_harness::declare_contingent`] uses. The
//!   remaining case,
//!   `observes_this_processs_own_pairs_order_for_the_campaign_controller`,
//!   is `#[ignore]`d for a reason that is not "blocked": `game.online.fault_campaign`
//!   is TypeScript-owned (`v2/README.md` §2, ~163 lines), and the
//!   per-process `pairs()` hash-order risk it probes for has no Rust analog
//!   at all — this crate's own rule (README rule 4: no `HashMap`/`HashSet`,
//!   `IndexMap` only) already eliminates the failure class that probe
//!   exists to catch. There is nothing for a Rust port of this case to
//!   assert.
//! - `"fault transport"` (6 cases) and `"fault harness input script"`
//!   (2 cases): neither needs a live `FaultHarness`, just a
//!   [`gc_netcode::fault_transport::StarTransportAdapter`] to wrap and
//!   [`gc_netcode::fault_harness::scripted_sample`]. The spec's own helper
//!   pairs two real `FakeStarTransport` endpoints; since that type does not
//!   exist here, [`StubHost`]/[`StubGuest`] below are a minimal substitute —
//!   a shared queue standing in for the star's actual routing — sufficient
//!   for every assertion these 8 cases make (delivery/drop/withhold counts,
//!   control-channel treatment, and delegation). These 8 are ported for
//!   real.

use gc_data::network_profiles::NetworkProfileName;
use gc_netcode::fault_harness;
use gc_netcode::fault_transport::{
    FaultTransport, FaultTransportOptions, StarTransportAdapter, TransportChannel, TransportError,
    TransportErrorCode, TransportMessage, TransportMessageType, TransportPeerEvent,
    TransportPeerMessage, TransportPeerState, TransportResult, TransportRole,
    TransportStarDiagnostics, TransportState,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// A minimal `StarTransportAdapter` pair, standing in for
// `game/transport/fake_star.lua`'s real routing (see module doc).
// ---------------------------------------------------------------------------

type Queue = Rc<RefCell<VecDeque<TransportPeerMessage>>>;

/// The host side: only ever used to enqueue envelopes the guest-side
/// [`FaultTransport`] under test will drain.
struct StubHost {
    to_guest: Queue,
    seq: i64,
}

impl StubHost {
    fn new() -> (Self, Queue) {
        let queue = Rc::new(RefCell::new(VecDeque::new()));
        (
            StubHost {
                to_guest: queue.clone(),
                seq: 0,
            },
            queue,
        )
    }

    fn send_input(&mut self, tick: i64) {
        self.seq += 1;
        self.to_guest.borrow_mut().push_back(TransportPeerMessage {
            peer_id: "host".to_string(),
            channel: TransportChannel::Input,
            message: TransportMessage {
                version: 1,
                kind: TransportMessageType::Input,
                seq: tick,
                tick: Some(tick),
                payload: format!("payload-{tick}").into_bytes(),
            },
            arrival_seq: self.seq,
        });
    }

    fn send_control(&mut self, seq: i64) {
        self.seq += 1;
        self.to_guest.borrow_mut().push_back(TransportPeerMessage {
            peer_id: "host".to_string(),
            channel: TransportChannel::Control,
            message: TransportMessage {
                version: 1,
                kind: TransportMessageType::Event,
                seq,
                tick: None,
                payload: format!("control-{seq}").into_bytes(),
            },
            arrival_seq: self.seq,
        });
    }
}

/// The guest side: the endpoint [`FaultTransport`] under test wraps.
struct StubGuest {
    inbound: Queue,
}

impl StubGuest {
    fn new(inbound: Queue) -> Self {
        StubGuest { inbound }
    }
}

impl StarTransportAdapter for StubGuest {
    fn initialize(&mut self) -> TransportResult<bool> {
        Ok(true)
    }
    fn shutdown(&mut self) -> TransportResult<bool> {
        Ok(true)
    }
    fn role(&self) -> TransportRole {
        TransportRole::Guest
    }
    fn capacity(&self) -> i64 {
        1
    }
    fn open_peer(&mut self, _peer_id: &str) -> TransportResult<i64> {
        Ok(1)
    }
    fn close_peer(&mut self, _peer_id: &str, _reason: Option<&str>) -> TransportResult<bool> {
        Ok(true)
    }
    fn peer_ids(&self) -> Vec<String> {
        vec!["host".to_string()]
    }
    fn peer_state(&self, _peer_id: &str) -> Option<TransportPeerState> {
        Some(TransportPeerState::Connected)
    }
    fn request_offer(&mut self, _peer_id: &str) -> TransportResult<bool> {
        Ok(true)
    }
    fn accept_offer(&mut self, _signal: &str) -> TransportResult<bool> {
        Ok(true)
    }
    fn accept_answer(&mut self, _peer_id: &str, _signal: &str) -> TransportResult<bool> {
        Ok(true)
    }
    fn take_signal(&mut self, _peer_id: &str) -> TransportResult<Option<String>> {
        Ok(None)
    }
    fn send(
        &mut self,
        _peer_id: &str,
        _channel: TransportChannel,
        _message: TransportMessage,
    ) -> TransportResult<bool> {
        Ok(true)
    }
    fn broadcast(
        &mut self,
        _channel: TransportChannel,
        _message: TransportMessage,
    ) -> TransportResult<i64> {
        Ok(0)
    }
    fn poll(&mut self) -> Option<TransportPeerMessage> {
        self.inbound.borrow_mut().pop_front()
    }
    fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage> {
        let budget = limit.unwrap_or(32).max(0) as usize;
        let mut batch = Vec::new();
        let mut queue = self.inbound.borrow_mut();
        while batch.len() < budget {
            match queue.pop_front() {
                Some(entry) => batch.push(entry),
                None => break,
            }
        }
        batch
    }
    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        None
    }
    fn state(&self) -> TransportState {
        TransportState::Connected
    }
    fn diagnostics(&self) -> TransportStarDiagnostics {
        TransportStarDiagnostics {
            role: Some(TransportRole::Guest),
            state: Some(TransportState::Connected),
            capacity: 1,
            peer_count: 1,
            ..Default::default()
        }
    }
}

fn pair(profile: NetworkProfileName) -> (FaultTransport, StubHost) {
    let (host, queue) = StubHost::new();
    let guest = StubGuest::new(queue);
    let fault = FaultTransport::new(FaultTransportOptions {
        transport: Box::new(guest),
        profile,
        seed: 17.0,
        legs: vec!["host".to_string()],
        poll_order: None,
        duplicate_control_every: None,
    });
    (fault, host)
}

// ---------------------------------------------------------------------------
// "fault transport" — ported for real
// ---------------------------------------------------------------------------

#[test]
fn delivers_everything_under_the_clean_profile() {
    let (mut fault, mut host) = pair(NetworkProfileName::Clean);
    let mut delivered = 0;
    for tick in 1..=40 {
        host.send_input(tick);
        fault.tick(tick);
        delivered += fault.poll_batch(Some(64)).len();
    }
    assert_eq!(delivered, 40);
    let counters = fault.counters();
    assert_eq!(counters.dropped, 0);
    assert_eq!(counters.pending, 0);
    assert_eq!(counters.profile, NetworkProfileName::Clean);
}

#[test]
fn drops_what_the_profile_says_and_nothing_else() {
    let (mut fault, mut host) = pair(NetworkProfileName::Stress);
    for tick in 1..=120 {
        host.send_input(tick);
        fault.tick(tick);
        fault.poll_batch(Some(64));
    }
    let counters = fault.counters();
    assert_eq!(counters.scheduled, 120);
    assert!(counters.dropped > 0, "the stress profile must lose packets");
    assert_eq!(
        counters.dropped,
        counters.independent_lost + counters.burst_lost
    );
    assert_eq!(counters.withheld, 0);
}

#[test]
fn withholds_a_declared_window_and_only_that_window() {
    let (mut fault, mut host) = pair(NetworkProfileName::Clean);
    fault.withhold(10, 14);
    let mut delivered = 0;
    for tick in 1..=30 {
        host.send_input(tick);
        fault.tick(tick);
        delivered += fault.poll_batch(Some(64)).len();
    }
    assert_eq!(fault.counters().withheld, 5);
    assert_eq!(delivered, 25);
}

#[test]
fn never_impairs_the_reliable_control_channel() {
    let (mut fault, mut host) = pair(NetworkProfileName::Stress);
    for tick in 1..=60 {
        host.send_control(tick);
        fault.tick(tick);
        fault.poll_batch(Some(64));
    }
    let counters = fault.counters();
    assert_eq!(counters.control_delivered, 60);
    assert_eq!(counters.scheduled, 0);
}

#[test]
fn duplicates_control_traffic_only_when_a_scenario_declares_it() {
    let (host_stub, queue) = StubHost::new();
    let mut host = host_stub;
    let guest = StubGuest::new(queue);
    let mut fault = FaultTransport::new(FaultTransportOptions {
        transport: Box::new(guest),
        profile: NetworkProfileName::Clean,
        seed: 3.0,
        legs: vec!["host".to_string()],
        poll_order: None,
        duplicate_control_every: Some(2),
    });
    let mut delivered = 0;
    for tick in 1..=10 {
        host.send_control(tick);
        fault.tick(tick);
        delivered += fault.poll_batch(Some(64)).len();
    }
    assert_eq!(fault.counters().control_duplicated, 5);
    assert_eq!(delivered, 15);
}

#[test]
fn substitutes_for_the_endpoint_it_wraps() {
    let (host, queue) = StubHost::new();
    let _ = host;
    let guest = StubGuest::new(queue.clone());
    let guest_role = guest.role();
    let guest_state = guest.state();
    let guest_peer_count = guest.peer_ids().len();
    let guest_capacity = guest.capacity();
    let mut fault = FaultTransport::new(FaultTransportOptions {
        transport: Box::new(guest),
        profile: NetworkProfileName::Clean,
        seed: 17.0,
        legs: vec!["host".to_string()],
        poll_order: None,
        duplicate_control_every: None,
    });
    assert_eq!(fault.role(), guest_role);
    assert_eq!(fault.state(), guest_state);
    assert_eq!(fault.peer_ids().len(), guest_peer_count);
    assert_eq!(fault.capacity(), guest_capacity);
    assert_eq!(fault.diagnostics().role, Some(TransportRole::Guest));
    let _: TransportResult<bool> = fault.send(
        "host",
        TransportChannel::Control,
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Event,
            seq: 0,
            tick: None,
            payload: Vec::new(),
        },
    );
    let _err: Option<TransportError> = None;
    let _ = TransportErrorCode::Backpressure;
}

// ---------------------------------------------------------------------------
// "fault harness input script" — ported for real
// ---------------------------------------------------------------------------

#[test]
fn is_a_pure_function_of_the_client_index_and_the_step() {
    for index in 1..=8 {
        for step in 0..=90 {
            let left = fault_harness::scripted_sample(index, step);
            let right = fault_harness::scripted_sample(index, step);
            assert_eq!(left.move_x, right.move_x);
            assert_eq!(left.move_y, right.move_y);
            assert_eq!(left.edges, right.edges);
            assert_eq!(left.held, right.held);
        }
    }
}

#[test]
fn moves_the_live_slot_by_putting_a_real_switch_edge_on_the_stream() {
    let mut switches = 0;
    for step in 0..=200 {
        if fault_harness::scripted_sample(1, step).edges != 0 {
            switches += 1;
        }
    }
    assert!(switches > 0, "the script must exercise switching");
    assert_eq!(gc_netcode::match_driver::DELAY_TICKS, 3);
}

// ---------------------------------------------------------------------------
// "online fault harness" — real, driving `fault_scenarios::run` the way
// `spec/game/online_fault_harness_spec.lua`'s own `run` helper drives
// `fault_scenarios.run`. See the module doc for the one narrowed case and
// the one still-ignored case.
// ---------------------------------------------------------------------------

use gc_netcode::fault_harness::FaultHarnessReport;
use gc_netcode::fault_scenarios::{self, FaultInjectionKind, RunOptions};

/// Short enough to keep the suite quick, long enough to cross several
/// confirmed checkpoints and at least one correction on every peer. Mirrors
/// the spec's own `SPEC_DURATION_TICKS`.
const SPEC_DURATION_TICKS: i64 = 60;

fn run(id: &str) -> gc_netcode::fault_harness::FaultHarnessReport {
    run_with(id, |_scenario| {})
}

/// Runs a declared scenario at [`SPEC_DURATION_TICKS`], with `edit` applied
/// to a mutable copy first. Mirrors the spec's own `run(id, overrides)`
/// helper: [`fault_scenarios::FaultScenario`] is `Copy`, so "override a
/// field" is exactly `edit`.
fn run_with(
    id: &str,
    edit: impl FnOnce(&mut fault_scenarios::FaultScenario),
) -> gc_netcode::fault_harness::FaultHarnessReport {
    let mut scenario =
        *fault_scenarios::find(id).unwrap_or_else(|| panic!("unknown scenario {id}"));
    edit(&mut scenario);
    fault_scenarios::run(
        &scenario,
        RunOptions {
            duration_ticks: Some(SPEC_DURATION_TICKS),
            network_seed: None,
        },
    )
}

fn finding<'a>(
    report: &'a FaultHarnessReport,
    id: &str,
) -> &'a gc_netcode::fault_harness::FaultHarnessFinding {
    report
        .findings
        .iter()
        .find(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("no finding named {id}"))
}

fn assert_all_ok(report: &FaultHarnessReport) {
    for entry in &report.findings {
        assert!(entry.ok, "{:?} {}: {}", report.mode, entry.id, entry.detail);
    }
}

/// `"checkpoint "`-prefixed markers only, mirrors the spec's own
/// `checkpoint_markers`.
fn checkpoint_markers(report: &FaultHarnessReport) -> Vec<&str> {
    report
        .markers
        .iter()
        .map(String::as_str)
        .filter(|marker| marker.starts_with("checkpoint "))
        .collect()
}

#[test]
fn runs_a_host_plus_seven_guests_from_handshake_to_an_agreed_result() {
    let report = run("4v4.clean");
    assert_eq!(report.clients, 8);
    assert_all_ok(&report);
    let hash = finding(&report, "converge.checkpoint_hash");
    assert!(
        hash.detail.contains("0 disagreed"),
        "every shared checkpoint must agree: {}",
        hash.detail
    );
}

#[test]
fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_1v1_and_2v2() {
    for id in ["1v1.clean", "2v2.clean"] {
        let report = run(id);
        let live = finding(&report, "converge.live_slot");
        assert!(!live.skipped, "{id} must actually compare live slots");
        assert!(live.ok, "{id} live slot: {}", live.detail);
        assert_all_ok(&report);
    }
}

/// 4v4 owned sets are singletons, so `next_live_slot` returns the slot
/// already live on every path. The row still runs; it is declared inert
/// rather than counted as coverage it does not provide.
#[test]
fn declares_the_4v4_live_slot_comparison_inert_instead_of_claiming_it() {
    let report = run("4v4.clean");
    let live = finding(&report, "converge.live_slot");
    assert!(live.skipped, "4v4 cannot exhibit a live-slot divergence");
    assert!(live.detail.contains("singleton"), "{}", live.detail);
}

#[test]
fn drives_the_documented_profiles_through_sim_network_conditions() {
    let report = run("1v1.stress");
    let mut impaired = false;
    for marker in &report.markers {
        if let Some(rest) = marker.strip_prefix("impairment ") {
            assert!(rest.contains("profile=stress"), "{marker}");
            let dropped: i64 = marker
                .split("dropped=")
                .nth(1)
                .and_then(|tail| tail.split_whitespace().next())
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            impaired = impaired || dropped > 0;
        }
    }
    assert!(impaired, "the stress profile must actually drop packets");
    // Impairment is not a licence to disagree.
    assert!(
        finding(&report, "converge.checkpoint_hash").ok,
        "stress must still converge"
    );
    assert!(
        finding(&report, "converge.live_slot").ok,
        "stress must agree on the live slot"
    );
}

/// Mechanism coverage, not the declared `2v2.poll_reversed` row: that row
/// runs the `stress` profile, whose losses would make the two runs differ
/// for a reason unrelated to release order. Holding the profile at `clean`
/// isolates the one variable. The matrix row itself still runs in
/// `fault_scenarios::SCENARIOS` and in `every_declared_row_reaches_its_declared_outcome`.
#[test]
fn keeps_confirmed_boundaries_independent_of_arrival_release_order() {
    let forward = run("2v2.clean");
    let reversed = run_with("2v2.clean", |scenario| {
        scenario.injection = FaultInjectionKind::PollReversed;
    });
    assert_eq!(
        checkpoint_markers(&reversed),
        checkpoint_markers(&forward),
        "reversing the release order must not move a confirmed boundary"
    );
}

/// `game.online.match_presentation` has no Rust port (TypeScript-owned,
/// `v2/README.md` §2), so — unlike the Lua original, which asserts these
/// findings `ok` — this port's `presentation.published_once`/
/// `presentation.no_revoked_survivor` are declared *skipped*, with an
/// accurate reason, rather than either measured (impossible: there is no
/// presentation timeline to fold) or silently omitted (indistinguishable
/// from "covered"). This case asserts that declaration is present and
/// honest, which is the meaningful claim this port can make about it.
#[test]
fn publishes_each_confirmed_event_once_and_never_resurrects_a_revoked_one() {
    let report = run("2v2.clean");
    for id in [
        "presentation.published_once",
        "presentation.no_revoked_survivor",
    ] {
        let entry = finding(&report, id);
        assert!(entry.skipped, "{id} must be declared skipped in this port");
        assert!(
            entry.detail.contains("TypeScript-owned"),
            "{id} must say why: {}",
            entry.detail
        );
    }
}

#[test]
fn reaches_the_declared_terminal_for_each_injected_fault() {
    for id in [
        "2v2.ownership_violation",
        "2v2.authority_conflict",
        "2v2.malformed_input",
        "2v2.peer_disconnect",
        "2v2.hash_divergence",
    ] {
        let declared = fault_scenarios::find(id).expect("declared scenario");
        let expected = declared.expect_status.expect("row declares a terminal");
        let report = run(id);
        let want = format!(
            "terminal.{}",
            gc_netcode::fault_harness::status_label(Some(expected))
        );
        let entry = finding(&report, &want);
        assert!(entry.ok, "{id}: {}", entry.detail);
    }
}

/// The mirror of "the stress profile must actually drop packets", one row
/// over: a clamped send buffer that never latched is a scenario that turned
/// itself off, and the gate it feeds would then be unfalsifiable.
#[test]
fn observes_the_backpressure_it_clamps_for_and_gates_on_a_real_peak() {
    let report = run("2v2.backpressure");
    let latched = finding(&report, "faults.backpressure_observed");
    assert!(!latched.skipped, "this row clamps the send buffer");
    assert!(
        latched.ok,
        "the clamped send buffer never latched backpressure: {}",
        latched.detail
    );
    let peak = finding(&report, "resources.channel_depth_observed");
    assert!(
        peak.ok,
        "the depth gate never observed a non-zero queue: {}",
        peak.detail
    );
    assert_all_ok(&report);
}

/// The depth gate reads a peak sampled every driver step
/// ([`gc_netcode::fault_harness::FaultHarness::advance`]), never a
/// quiescent final snapshot. This is the regression guard for exactly the
/// bug `docs/online/fault_harness.md` names.
#[test]
fn gates_channel_depth_on_a_peak_it_actually_observed() {
    for id in ["1v1.clean", "4v4.clean"] {
        let report = run(id);
        let observed = finding(&report, "resources.channel_depth_observed");
        assert!(observed.ok, "{id}: {}", observed.detail);
        assert!(
            finding(&report, "resources.channel_depth").ok,
            "{id} exceeded the gate"
        );
        assert!(
            finding(&report, "resources.no_overflow").ok,
            "{id} refused a send"
        );
    }
}

#[test]
fn names_the_contingent_rows_instead_of_omitting_them() {
    let report = run("1v1.clean");
    for id in [
        "combat.correction.wind_up",
        "combat.correction.contact",
        "combat.correction.immunity_expiry",
        "combat.default_disposition",
        "browser.multi_context",
    ] {
        let entry = finding(&report, id);
        assert!(entry.skipped, "{id} must be skipped, not silently passed");
        assert!(!entry.detail.is_empty(), "{id} must carry a reason");
    }
}

#[test]
fn logs_the_smoke_subset_as_a_subset() {
    let smoke = fault_scenarios::select(true);
    let full = fault_scenarios::select(false);
    assert!(!smoke.is_empty(), "the CI subset must not be empty");
    assert!(
        smoke.len() < full.len(),
        "the CI subset must be a strict subset"
    );
    let mut ids: Vec<&str> = full.iter().map(|scenario| scenario.id).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "scenario ids must be unique");
    for scenario in &smoke {
        assert!(
            scenario.smoke,
            "the subset must only contain rows marked smoke"
        );
    }
}

/// `game.online.fault_campaign` (the module `hash_order_probe`/`PROBE_KEYS`
/// belong to) is TypeScript-owned (`v2/README.md` §2) and has no Rust port —
/// see the module doc. Left `#[ignore]`d for that reason, not a blocker:
/// this crate's own no-`HashMap`/`HashSet` rule (README rule 4, `IndexMap`
/// only) already eliminates the per-process hash-order-randomization risk
/// this probe exists to catch, so there is no Rust behaviour for a port of
/// this case to exercise.
#[test]
#[ignore = "not applicable: game.online.fault_campaign is TypeScript-owned, and this crate's \
    IndexMap-only rule already rules out the pairs()-order risk the probe checks for -- see \
    the module doc"]
fn observes_this_processs_own_pairs_order_for_the_campaign_controller() {
    unreachable!(
        "no Rust equivalent of a per-process pairs() hash-order probe exists or is needed"
    );
}
