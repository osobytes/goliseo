//! Port of `spec/game/online_fault_harness_spec.lua`.
//!
//! Three `t.describe` blocks, three different outcomes:
//!
//! - `"online fault harness"` (12 cases): every case runs a full
//!   `FaultHarness` end to end. Building one needs `game.online.coordinator`,
//!   `game.online.match_manifest`, `game.online.match_session`,
//!   `game.online.protocol`, `game.online.protocol_fixture`
//!   (`NOT YET PORTED` placeholders in this crate, owned by concurrent
//!   agents), and `game.online.lobby_link`, `game.online.net_diagnostics`,
//!   `game.online.match_presentation`, `game.screens.online_match_model`,
//!   `game.transport.fake_relay`, `game.transport.fake_star` (permanently
//!   TypeScript-owned, `v2/README.md` §2). None of these are ported; every
//!   case here is `#[ignore]`d naming the same blocker.
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
// "online fault harness" — blocked (see module doc)
// ---------------------------------------------------------------------------

const BLOCKED: &str = "blocked on building a live FaultHarness: needs crate::coordinator, \
    crate::match_manifest, crate::match_session, crate::protocol, crate::protocol_fixture \
    (NOT YET PORTED placeholders in this crate, owned by concurrent agents) and \
    game/online/lobby_link.lua, game/online/net_diagnostics.lua, \
    game/online/match_presentation.lua, game/screens/online_match_model.lua, \
    game/transport/fake_relay.lua, game/transport/fake_star.lua (permanently \
    TypeScript-owned, v2/README.md §2). See crate::fault_harness's module doc.";

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn runs_a_host_plus_seven_guests_from_handshake_to_an_agreed_result() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_1v1_and_2v2() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn declares_the_4v4_live_slot_comparison_inert_instead_of_claiming_it() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn drives_the_documented_profiles_through_sim_network_conditions() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn keeps_confirmed_boundaries_independent_of_arrival_release_order() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn publishes_each_confirmed_event_once_and_never_resurrects_a_revoked_one() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn reaches_the_declared_terminal_for_each_injected_fault() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn observes_the_backpressure_it_clamps_for_and_gates_on_a_real_peak() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn gates_channel_depth_on_a_peak_it_actually_observed() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn names_the_contingent_rows_instead_of_omitting_them() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn logs_the_smoke_subset_as_a_subset() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on a live FaultHarness; see BLOCKED"]
fn observes_this_processs_own_pairs_order_for_the_campaign_controller() {
    unreachable!("{BLOCKED}");
}
