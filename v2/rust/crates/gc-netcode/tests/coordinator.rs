//! Port of `spec/game/online_coordinator_spec.lua`.
//!
//! This file also carries the reducer's required differential evidence
//! (README rule 5.9 / `v2/tools/lua_reference/README.md`): a from-scratch
//! event sequence — connect, propose manifest, assign slots, set ready,
//! begin countdown, several ticks with agreeing hash reports, then a
//! deliberate three-tick boundary-hash disagreement — driven identically
//! through the real Lua `game/online/coordinator_driver.lua` and through
//! this port, comparing phase, terminal reason, and mismatch counters on
//! *both* peers at every step. See
//! [`coordinator_reducer_reproduces_the_lua_reference_rejection_and_desync_paths`]
//! for the important part: the happy path (`agree_tick_*`) is not the
//! interesting evidence here, the desync path is — the host detects the
//! third disagreement and terminates as `hash_mismatch` while the guest,
//! racing the announced abort, never reaches its own third count and ends
//! as `peer_abort` instead. A reducer that agreed only on the happy path
//! and diverged on this would be exactly the failure this port must not
//! ship.
//!
//! `tests/fixtures/coordinator_desync_lua_reference.txt` is the captured
//! stdout of running the real Lua `game/online/coordinator_driver.lua` (via
//! `coordinator.lua`, `coordinator_fixture.lua`, `protocol.lua`,
//! `protocol_fixture.lua`, and their `sim`/`data`/`core` dependencies) under
//! headless `love` (no display, no `xvfb`), via a scratch `conf.lua`/
//! `main.lua` harness per that README (not committed — scratch dirs are
//! session-local).

use gc_netcode::coordinator::{self, Event, Options, PreferenceState, Role, TerminalReason};
use gc_netcode::coordinator_driver::{self as driver, Driver};
use gc_netcode::coordinator_fixture as fixture;
use gc_netcode::protocol::{self, Value};

fn phase_wire(phase: protocol::LifecyclePhase) -> &'static str {
    use protocol::LifecyclePhase::*;
    match phase {
        New => "new",
        Handshake => "handshake",
        Manifest => "manifest",
        Assigned => "assigned",
        Ready => "ready",
        Countdown => "countdown",
        Running => "running",
        Result => "result",
        Terminal => "terminal",
    }
}

fn reason_wire(reason: TerminalReason) -> &'static str {
    use TerminalReason::*;
    match reason {
        Completed => "completed",
        LocalAbort => "local_abort",
        PeerAbort => "peer_abort",
        GuestLeft => "guest_left",
        HostLeft => "host_left",
        Removed => "removed",
        TransportLost => "transport_lost",
        ProtocolViolation => "protocol_violation",
        ManifestMismatch => "manifest_mismatch",
        BuildMismatch => "build_mismatch",
        InvalidAssignment => "invalid_assignment",
        StartAckTimeout => "start_ack_timeout",
        InputChannelFailure => "input_channel_failure",
        LateInput => "late_input",
        HashMismatch => "hash_mismatch",
    }
}

fn disposition_wire(disposition: coordinator::Disposition) -> &'static str {
    match disposition {
        coordinator::Disposition::Applied => "applied",
        coordinator::Disposition::Idempotent => "idempotent",
        coordinator::Disposition::Rejected => "rejected",
    }
}

fn reject_code_wire(code: coordinator::RejectCode) -> &'static str {
    use coordinator::RejectCode::*;
    match code {
        Malformed => "malformed",
        WireTooLarge => "wire_too_large",
        UnsupportedVersion => "unsupported_version",
        UnknownMessage => "unknown_message",
        IdentityMismatch => "identity_mismatch",
        RuntimeMismatch => "runtime_mismatch",
        InvalidPhase => "invalid_phase",
        Duplicate => "duplicate",
        TranscriptConflict => "transcript_conflict",
        UnsupportedMatchMode => "unsupported_match_mode",
        InvalidOwnership => "invalid_ownership",
        Capacity => "capacity",
        DuplicatePeer => "duplicate_peer",
        RoleConflict => "role_conflict",
        InvalidAssignment => "invalid_assignment",
        UnknownLink => "unknown_link",
        NotPermitted => "not_permitted",
    }
}

fn record(session: &Driver, guest_peer: &str, label: &str) -> String {
    let host = session.host();
    let guest = session.node(guest_peer).expect("guest is admitted");
    let host_tracks_guest = host.state.peers.get(1).map(|p| p.hash_mismatches);
    let guest_tracks_host = guest.state.peers.get(1).map(|p| p.hash_mismatches);
    format!(
        "{label}|host_phase={}|host_terminal={}|guest_phase={}|guest_terminal={}|\
host_tracks_guest_mismatches={}|guest_tracks_host_mismatches={}",
        phase_wire(host.state.phase),
        host.terminal
            .as_ref()
            .map_or("nil".to_string(), |t| reason_wire(t.reason).to_string()),
        phase_wire(guest.state.phase),
        guest
            .terminal
            .as_ref()
            .map_or("nil".to_string(), |t| reason_wire(t.reason).to_string()),
        host_tracks_guest.map_or("nil".to_string(), |v| v.to_string()),
        guest_tracks_host.map_or("nil".to_string(), |v| v.to_string()),
    )
}

const FIXTURE: &str = include_str!("fixtures/coordinator_desync_lua_reference.txt");

#[test]
fn coordinator_reducer_reproduces_the_lua_reference_rejection_and_desync_paths() {
    let reference: Vec<&str> = FIXTURE.lines().collect();
    let mut log: Vec<String> = Vec::new();

    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    let guest_peer = fixture::guest_peer_id(1);

    session.reach_start(Some(3), Some(0));
    log.push(record(&session, &guest_peer, "after_reach_start"));

    session.send(
        fixture::HOST_PEER_ID,
        Event::MatchPhase {
            phase: "kickoff".to_string(),
            tick: 0,
            home_score: 0,
            away_score: 0,
        },
    );
    session.send(
        fixture::HOST_PEER_ID,
        Event::MatchPhase {
            phase: "playing".to_string(),
            tick: 1,
            home_score: 0,
            away_score: 0,
        },
    );
    session.pump();
    log.push(record(&session, &guest_peer, "after_kickoff"));

    // Agreeing boundary hashes: the happy path.
    for tick in [10, 20] {
        let outcome_h = session.send(
            fixture::HOST_PEER_ID,
            Event::HashReport {
                tick,
                boundary_hash: "aaaaaaaaaaaaaaaa".to_string(),
            },
        );
        session.pump();
        let outcome_g = session.send(
            &guest_peer,
            Event::HashReport {
                tick,
                boundary_hash: "aaaaaaaaaaaaaaaa".to_string(),
            },
        );
        session.pump();
        log.push(format!(
            "agree_tick_{tick}|host_outcome={}|guest_outcome={}",
            disposition_wire(outcome_h.disposition),
            disposition_wire(outcome_g.disposition)
        ));
        log.push(record(
            &session,
            &guest_peer,
            &format!("after_agree_{tick}"),
        ));
    }

    // Deliberate disagreement for three consecutive boundary ticks: this
    // trips `MAX_HASH_MISMATCHES` (3) and should end the session as a
    // `hash_mismatch` desync on whichever side detects the third one, with
    // the other side ending as `peer_abort` on receipt of the announced
    // abort.
    for tick in [30, 40, 50] {
        let outcome_h = session.send(
            fixture::HOST_PEER_ID,
            Event::HashReport {
                tick,
                boundary_hash: "cccccccccccccccc".to_string(),
            },
        );
        session.pump();
        let outcome_g = session.send(
            &guest_peer,
            Event::HashReport {
                tick,
                boundary_hash: "dddddddddddddddd".to_string(),
            },
        );
        session.pump();
        log.push(format!(
            "disagree_tick_{tick}|host_outcome={}|guest_outcome={}",
            disposition_wire(outcome_h.disposition),
            disposition_wire(outcome_g.disposition)
        ));
        log.push(record(
            &session,
            &guest_peer,
            &format!("after_disagree_{tick}"),
        ));
    }

    // A rejection path once terminal: further sends must be refused, not
    // silently ignored or applied.
    let rejected = session.send(fixture::HOST_PEER_ID, Event::SetReady { ready: false });
    log.push(format!(
        "post_terminal_reject|accepted={}|disposition={}|code={}",
        rejected.accepted,
        disposition_wire(rejected.disposition),
        rejected
            .code
            .map_or("nil".to_string(), |c| reject_code_wire(c).to_string()),
    ));

    log.push(format!("transcript_id={}", session.transcript_id()));
    log.push(format!("message_count={}", session.transcript.len()));

    assert_eq!(
        log.len(),
        reference.len(),
        "reference/port line count differs:\n{log:#?}\nvs\n{reference:#?}"
    );
    for (index, (ours, theirs)) in log.iter().zip(reference.iter()).enumerate() {
        assert_eq!(ours, theirs, "line {index} diverges from the Lua reference");
    }
}

// ---------------------------------------------------------------------------
// The full golden conformance session (happy path across 4v4/2v2/1v1/bots).
// ---------------------------------------------------------------------------

#[test]
fn coordinator_conformance_matches_the_lua_reference_golden_values() {
    let report = gc_netcode::coordinator_conformance::verify();
    assert_eq!(
        report.message_count,
        gc_netcode::coordinator_conformance::GOLDEN.full_message_count
    );
}

// ---------------------------------------------------------------------------
// Construction.
// ---------------------------------------------------------------------------

#[test]
fn new_admits_the_host_immediately_and_leaves_a_guest_unheard() {
    let host = fixture::host(None);
    assert_eq!(host.phase, protocol::LifecyclePhase::Handshake);
    assert_eq!(host.peers.len(), 1);

    let guest = fixture::guest(1, None, None);
    assert_eq!(guest.phase, protocol::LifecyclePhase::New);
    assert_eq!(guest.peers.len(), 2);
}

#[test]
fn new_refuses_a_guest_claiming_the_host_identity() {
    let result = coordinator::new(Options {
        role: Role::Guest,
        session_id: "s".to_string(),
        peer_id: "host".to_string(),
        host_peer_id: Some("host".to_string()),
        host_link_id: Some("link.host".to_string()),
        runtime: fixture::runtime(),
        build_id: None,
        expectation: None,
    });
    assert_eq!(
        result.unwrap_err().code,
        coordinator::RejectCode::RoleConflict
    );
}

#[test]
fn new_refuses_a_guest_without_a_host_link() {
    let result = coordinator::new(Options {
        role: Role::Guest,
        session_id: "s".to_string(),
        peer_id: "guest.1".to_string(),
        host_peer_id: Some("host".to_string()),
        host_link_id: None,
        runtime: fixture::runtime(),
        build_id: None,
        expectation: None,
    });
    assert_eq!(result.unwrap_err().code, coordinator::RejectCode::Malformed);
}

// ---------------------------------------------------------------------------
// Connect / handshake.
// ---------------------------------------------------------------------------

#[test]
fn connect_is_only_permitted_for_a_guest() {
    let host = fixture::host(None);
    let (_, outcome) = coordinator::step(&host, Event::Connect);
    assert_eq!(outcome.disposition, coordinator::Disposition::Rejected);
    assert_eq!(outcome.code, Some(coordinator::RejectCode::NotPermitted));
}

#[test]
fn connect_is_idempotent_once_handshaking() {
    let guest = fixture::guest(1, None, None);
    let (next, outcome) = coordinator::step(&guest, Event::Connect);
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next.phase, protocol::LifecyclePhase::Handshake);
    let (_, outcome2) = coordinator::step(&next, Event::Connect);
    assert_eq!(outcome2.disposition, coordinator::Disposition::Idempotent);
}

// ---------------------------------------------------------------------------
// Manifest proposal.
// ---------------------------------------------------------------------------

#[test]
fn propose_manifest_is_host_only() {
    let guest = fixture::guest(1, None, None);
    let (_, outcome) = coordinator::step(
        &guest,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::NotPermitted));
}

#[test]
fn propose_manifest_is_idempotent_on_an_identical_resend() {
    let host = fixture::host(None);
    let (next, outcome) = coordinator::step(
        &host,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    let (_, outcome2) = coordinator::step(
        &next,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    assert_eq!(outcome2.disposition, coordinator::Disposition::Idempotent);
}

#[test]
fn propose_manifest_is_immutable_after_the_first_proposal() {
    let host = fixture::host(None);
    let (next, _) = coordinator::step(
        &host,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    let mut other = fixture::manifest(None);
    other.set("seed", Value::int(999_999));
    let (_, outcome) = coordinator::step(&next, Event::ProposeManifest { manifest: other });
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::IdentityMismatch)
    );
}

// ---------------------------------------------------------------------------
// Slot assignment / readiness / pair preference / countdown, via the driver.
// ---------------------------------------------------------------------------

#[test]
fn assign_slots_is_host_only_and_readiness_gates_on_the_current_generation() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        fixture::HOST_PEER_ID,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.pump();

    let guest_peer = fixture::guest_peer_id(1);
    let outcome = session.send(
        &guest_peer,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::NotPermitted));

    session.send(
        fixture::HOST_PEER_ID,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.pump();

    // Ready before every peer accepted is impossible here since the driver
    // already delivered manifest_accept via pump; readiness for an owned
    // slot should now succeed.
    let ready_outcome = session.send(fixture::HOST_PEER_ID, Event::SetReady { ready: true });
    assert_eq!(ready_outcome.disposition, coordinator::Disposition::Applied);
}

#[test]
fn begin_countdown_requires_every_peer_ready() {
    let host = fixture::host(None);
    let (_, outcome) = coordinator::step(
        &host,
        Event::BeginCountdown {
            countdown_id: "c.1".to_string(),
            remaining_ticks: 3,
            first_input_tick: 0,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
}

#[test]
fn prefer_pair_is_inert_in_4v4_and_unchanged_for_the_slot_already_owned() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(0),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        fixture::HOST_PEER_ID,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.pump();
    session.send(
        fixture::HOST_PEER_ID,
        Event::AssignSlots {
            assignments: fixture::assignments(0, None),
            preserve_claims: false,
        },
    );
    session.pump();
    let owned = coordinator::owned_slots(&session.host().state, fixture::HOST_PEER_ID);
    assert_eq!(owned.len(), 1);
    let outcome = session.send(fixture::HOST_PEER_ID, Event::PreferPair { slots: owned });
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(
        session.host().state.preference.as_ref().unwrap().status,
        PreferenceState::Unchanged
    );
}

// ---------------------------------------------------------------------------
// Netcode failure / abort / leave.
// ---------------------------------------------------------------------------

#[test]
fn netcode_failure_with_an_unknown_class_is_rejected() {
    let host = fixture::host(None);
    let (_, outcome) = coordinator::step(
        &host,
        Event::NetcodeFailure {
            failure: "not_a_real_class".to_string(),
            peer_id: None,
            detail: None,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::Malformed));
}

#[test]
fn netcode_failure_desync_terminates_as_hash_mismatch() {
    let host = fixture::host(None);
    let (next, outcome) = coordinator::step(
        &host,
        Event::NetcodeFailure {
            failure: "desync".to_string(),
            peer_id: None,
            detail: None,
        },
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next.terminal.unwrap().reason, TerminalReason::HashMismatch);
}

#[test]
fn leave_is_guest_only() {
    let host = fixture::host(None);
    let (_, outcome) = coordinator::step(&host, Event::Leave);
    assert_eq!(outcome.code, Some(coordinator::RejectCode::NotPermitted));

    let guest = fixture::guest(1, None, None);
    let (next, outcome) = coordinator::step(&guest, Event::Leave);
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next.terminal.unwrap().reason, TerminalReason::GuestLeft);
}

#[test]
fn abort_defaults_its_code_to_host_abort() {
    let host = fixture::host(None);
    let (next, _) = coordinator::step(
        &host,
        Event::Abort {
            code: None,
            detail: None,
        },
    );
    let terminal = next.terminal.unwrap();
    assert_eq!(terminal.reason, TerminalReason::LocalAbort);
    assert_eq!(terminal.code.as_deref(), Some("host_abort"));
}

// ---------------------------------------------------------------------------
// Once terminal, every event but `tick` is refused.
// ---------------------------------------------------------------------------

#[test]
fn a_terminal_session_refuses_every_event_but_tick() {
    let host = fixture::host(None);
    let (terminal_state, _) = coordinator::step(
        &host,
        Event::Abort {
            code: None,
            detail: None,
        },
    );
    assert!(coordinator::is_terminal(&terminal_state));
    let (_, outcome) = coordinator::step(&terminal_state, Event::SetReady { ready: true });
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    let (after_tick, tick_outcome) = coordinator::step(&terminal_state, Event::Tick);
    assert_eq!(tick_outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(after_tick.clock, terminal_state.clock + 1);
}

// ===========================================================================
// Below: the remaining `t.it` cases of `spec/game/online_coordinator_spec.lua`
// not covered by the tests above. See this file's module doc comment for the
// two large differential/golden tests, which stay untouched; the sixteen
// named tests above them were already ported before this section and are
// also untouched. Helpers below mirror the Lua spec's own local helpers
// (`message`, `handshake`, `deliver`, `assigned_host`, `ready_host`).
// ===========================================================================

use gc_netcode::coordinator::CoordinatorState;

/// The fixture session id every coordinator in this file shares (mirrors the
/// spec's `SESSION = fixture.manifest().session_id`).
const SESSION: &str = "session_alpha";

/// `message(kind, peer_id, sequence, body)`: `protocol.new` plus an assert.
fn message(
    kind: protocol::MessageKind,
    peer_id: &str,
    sequence: i64,
    body: Value,
) -> protocol::ControlMessage {
    protocol::new(kind, SESSION, peer_id, sequence, body).expect("test message is well-formed")
}

/// `handshake(peer_id, sequence, role)`.
fn handshake(peer_id: &str, sequence: i64, role: Role) -> protocol::ControlMessage {
    message(
        protocol::MessageKind::Handshake,
        peer_id,
        sequence,
        Value::record(vec![
            ("role", Value::str(role.wire_str())),
            ("runtime", fixture::runtime()),
        ]),
    )
}

/// `deliver(state, peer_id, control)`.
fn deliver(
    state: &CoordinatorState,
    peer_id: &str,
    control: protocol::ControlMessage,
) -> (CoordinatorState, coordinator::Outcome) {
    coordinator::step(
        state,
        Event::Control {
            link_id: fixture::link_id(peer_id),
            message: Some(control),
            wire: None,
        },
    )
}

/// A host that has admitted `guest_count` guests, proposed the manifest, seen
/// every acceptance, and published canonical ownership. Mirrors the spec's
/// `assigned_host`.
fn assigned_host(guest_count: i64) -> (CoordinatorState, Vec<i64>) {
    let mut state = fixture::host(None);
    let mut sequences = vec![0i64; guest_count as usize];
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        state = deliver(&state, &peer_id, handshake(&peer_id, 0, Role::Guest)).0;
    }
    state = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    )
    .0;
    let manifest_id = state.manifest_id.clone().expect("manifest proposed");
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        sequences[(index - 1) as usize] += 1;
        let sequence = sequences[(index - 1) as usize];
        state = deliver(
            &state,
            &peer_id,
            message(
                protocol::MessageKind::ManifestAccept,
                &peer_id,
                sequence,
                Value::record(vec![("manifest_id", Value::str(manifest_id.clone()))]),
            ),
        )
        .0;
    }
    state = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: fixture::assignments(guest_count, None),
            preserve_claims: false,
        },
    )
    .0;
    (state, sequences)
}

/// Every admitted guest sends `ready = true`, then the host's own local
/// readiness is set. Mirrors the spec's `ready_host`.
fn ready_host(
    mut state: CoordinatorState,
    guest_count: i64,
    sequences: &mut [i64],
) -> CoordinatorState {
    let manifest_id = state.manifest_id.clone().expect("manifest accepted");
    let assignment_id = state.assignment_id.clone().expect("ownership published");
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        sequences[(index - 1) as usize] += 1;
        let sequence = sequences[(index - 1) as usize];
        state = deliver(
            &state,
            &peer_id,
            message(
                protocol::MessageKind::Ready,
                &peer_id,
                sequence,
                Value::record(vec![
                    ("manifest_id", Value::str(manifest_id.clone())),
                    ("assignment_id", Value::str(assignment_id.clone())),
                    ("ready", Value::bool(true)),
                ]),
            ),
        )
        .0;
    }
    coordinator::step(&state, Event::SetReady { ready: true }).0
}

// ---------------------------------------------------------------------------
// Construction (remaining cases).
// ---------------------------------------------------------------------------

#[test]
fn refuses_malformed_or_contradictory_identities() {
    let runtime = fixture::runtime();

    // `role = "spectator"`: the Lua original runtime-checks a raw string
    // role field and rejects it as malformed. `Options::role` here is a
    // Rust `Role` enum with only `Host`/`Guest` variants, so this bad value
    // is unconstructible in this port — the enum itself stands in for that
    // check (v2/README.md porting rule 6; precedent: `gc-sim/tests/
    // possession_transition.rs`, `content_validation.rs`).

    // A guest without a host link id: malformed. (Already exercised by
    // `new_refuses_a_guest_without_a_host_link`; re-asserted here so this
    // test stands as a complete port of its Lua original.)
    let result = coordinator::new(Options {
        role: Role::Guest,
        session_id: SESSION.to_string(),
        peer_id: "g".to_string(),
        host_peer_id: Some(fixture::HOST_PEER_ID.to_string()),
        host_link_id: None,
        runtime: runtime.clone(),
        build_id: None,
        expectation: None,
    });
    assert_eq!(result.unwrap_err().code, coordinator::RejectCode::Malformed);

    // A guest claiming the host's own peer identity: role_conflict. (Already
    // exercised by `new_refuses_a_guest_claiming_the_host_identity`.)
    let result = coordinator::new(Options {
        role: Role::Guest,
        session_id: SESSION.to_string(),
        peer_id: fixture::HOST_PEER_ID.to_string(),
        host_peer_id: Some(fixture::HOST_PEER_ID.to_string()),
        host_link_id: Some("link.a".to_string()),
        runtime: runtime.clone(),
        build_id: None,
        expectation: None,
    });
    assert_eq!(
        result.unwrap_err().code,
        coordinator::RejectCode::RoleConflict
    );

    // A host with a malformed session id: malformed.
    let result = coordinator::new(Options {
        role: Role::Host,
        session_id: "not a session id".to_string(),
        peer_id: fixture::HOST_PEER_ID.to_string(),
        host_peer_id: None,
        host_link_id: None,
        runtime,
        build_id: None,
        expectation: None,
    });
    assert_eq!(result.unwrap_err().code, coordinator::RejectCode::Malformed);
}

#[test]
fn names_every_lifecycle_phase_exactly_once() {
    let mut seen: Vec<protocol::LifecyclePhase> = Vec::new();
    for &phase in coordinator::PHASES {
        assert!(!seen.contains(&phase), "phase {phase:?} is listed twice");
        seen.push(phase);
    }
    assert_eq!(coordinator::PHASES.len(), 9);
    assert!(seen.contains(&protocol::LifecyclePhase::New));
    assert!(seen.contains(&protocol::LifecyclePhase::Terminal));
    assert!(seen.contains(&protocol::LifecyclePhase::Countdown));
    assert!(seen.contains(&protocol::LifecyclePhase::Running));
}

#[test]
fn maps_every_terminal_reason_to_a_closed_protocol_code() {
    // The Lua original walks `coordinator.TERMINAL_CODES`, a plain lookup
    // table, and checks every non-`completed` reason maps to a code that
    // `protocol.new("abort", ...)` accepts. This port's equivalent,
    // `terminal_code` (`coordinator.rs` around line 475), is a private `fn`
    // — not `pub`, so it cannot be introspected directly from this test
    // crate (README rule 5.8 says widen visibility only in `src/`, out of
    // scope for a test port). Instead this test drives the reducer down a
    // cheap real path to each reason and reads the wire code off the
    // resulting `Terminal`, which is the information the Lua test ultimately
    // cares about: a legal, non-null protocol abort code.
    //
    // Not exercised here, with why: `PeerAbort`'s code always arrives as an
    // explicit wire value at its one call site (`apply_abort`), so the
    // table's fallback for it is not independently reachable through the
    // public event surface. `Removed`, `TransportLost`, and
    // `StartAckTimeout` are reachable but not cheaply (a non-host-left
    // disconnect, a lost host link, and 120 ticks of countdown drift,
    // respectively); the `peer_disconnect` code family they share is already
    // demonstrated below via `GuestLeft`.

    fn assert_code(terminal: &coordinator::Terminal, expected: &str) {
        let code = terminal
            .code
            .clone()
            .expect("a non-completed termination carries a code");
        assert_eq!(code, expected);
        assert!(
            protocol::new(
                protocol::MessageKind::Abort,
                SESSION,
                fixture::HOST_PEER_ID,
                0,
                Value::record(vec![("code", Value::str(code))]),
            )
            .is_ok(),
            "{expected} does not encode as a legal abort code"
        );
    }

    // local_abort -> host_abort
    let (next, _) = coordinator::step(
        &fixture::host(None),
        Event::Abort {
            code: None,
            detail: None,
        },
    );
    assert_code(&next.terminal.unwrap(), "host_abort");

    // guest_left -> peer_disconnect
    let (next, _) = coordinator::step(&fixture::guest(1, None, None), Event::Leave);
    assert_code(&next.terminal.unwrap(), "peer_disconnect");

    // input_channel_failure / late_input / hash_mismatch
    for (failure, expected) in [
        ("input_channel", "peer_disconnect"),
        ("late_input", "desync"),
        ("desync", "desync"),
    ] {
        let (next, _) = coordinator::step(
            &fixture::host(None),
            Event::NetcodeFailure {
                failure: failure.to_string(),
                peer_id: None,
                detail: None,
            },
        );
        assert_code(&next.terminal.unwrap(), expected);
    }

    // protocol_violation -> malformed_message (an admitted link handshaking
    // twice).
    let state = deliver(
        &fixture::host(None),
        "guest.1",
        handshake("guest.1", 0, Role::Guest),
    )
    .0;
    let (next, _) = deliver(&state, "guest.1", handshake("guest.1", 1, Role::Guest));
    assert_code(&next.terminal.unwrap(), "malformed_message");

    // manifest_mismatch (and, identically, build_mismatch: `terminal_code`
    // maps both reasons to this same wire code) -> manifest_mismatch.
    let guest = coordinator::step(&fixture::guest(1, None, None), Event::Connect).0;
    let mut other = fixture::manifest(None);
    other.set("content_id", Value::str("content.other.v1"));
    let (next, _) = deliver(
        &guest,
        "guest.1",
        message(
            protocol::MessageKind::ManifestProposal,
            fixture::HOST_PEER_ID,
            0,
            Value::record(vec![
                ("manifest_id", Value::str(protocol::manifest_id(&other))),
                ("manifest", other),
            ]),
        ),
    );
    assert_code(&next.terminal.unwrap(), "manifest_mismatch");

    // invalid_assignment (a guest offered ownership that seats none of its
    // own slots; see also "refuses ownership that seats no local slot"
    // below).
    let manifest = fixture::manifest(None);
    let manifest_id = protocol::manifest_id(&manifest);
    let guest = deliver(
        &guest,
        "guest.1",
        message(
            protocol::MessageKind::ManifestProposal,
            fixture::HOST_PEER_ID,
            0,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("manifest", manifest.clone()),
            ]),
        ),
    )
    .0;
    let unowned =
        coordinator::plan_assignments(&manifest, &[fixture::HOST_PEER_ID.to_string()]).unwrap();
    let (next, _) = deliver(
        &guest,
        "guest.1",
        message(
            protocol::MessageKind::SlotAssignment,
            fixture::HOST_PEER_ID,
            1,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                (
                    "assignment_id",
                    Value::str(protocol::assignment_id(&unowned, 1)),
                ),
                ("assignments", unowned),
            ]),
        ),
    );
    assert_code(&next.terminal.unwrap(), "invalid_assignment");

    // A completed session names no code at all: see "completes only when
    // every peer acknowledges the same result" below.
}

// ---------------------------------------------------------------------------
// Slot ownership.
// ---------------------------------------------------------------------------

#[test]
fn gives_all_eight_canonical_slots_exactly_one_declared_source() {
    let manifest = fixture::manifest(None);
    for guest_count in 0..=coordinator::MAX_GUESTS {
        let peer_ids = fixture::peer_ids(guest_count);
        let assignments = coordinator::plan_assignments(&manifest, &peer_ids).unwrap();
        let sources = coordinator::slot_sources(&manifest, &assignments).unwrap();
        let mut peers = 0;
        let mut bots = 0;
        let mut ids: Vec<String> = Vec::new();
        for index in 1..=gc_sim::input_frame::SLOT_COUNT {
            let slot = gc_sim::input_frame::slot(index).unwrap();
            // `slot_sources` returns a slot-id-keyed record (the Lua
            // original indexes it as `sources[slot.id]`), not the 1-based
            // array `plan_assignments`/`assignments` returns.
            let producer = sources.get(protocol::slot_wire_id(slot.id)).unwrap();
            assert_eq!(
                producer.get("slot").and_then(Value::as_str),
                Some(protocol::slot_wire_id(slot.id))
            );
            assert_eq!(
                producer.get("team").and_then(Value::as_str),
                Some(protocol::team_wire_str(slot.team))
            );
            let producer_id = producer
                .get("producer_id")
                .and_then(Value::as_str)
                .unwrap()
                .to_string();
            assert!(!ids.contains(&producer_id), "producer ids collide");
            ids.push(producer_id);
            if producer.get("producer_kind").and_then(Value::as_str) == Some("peer") {
                peers += 1;
                assert!(producer.get("bot_seed").is_none());
            } else {
                bots += 1;
                assert!(producer.get("bot_seed").is_some());
            }
        }
        assert_eq!(peers, guest_count + 1);
        assert_eq!(bots, gc_sim::input_frame::SLOT_COUNT - guest_count - 1);
    }
}

#[test]
fn never_seats_a_combat_protected_keeper() {
    let manifest = fixture::manifest(None);
    let mut keepers: Vec<String> = Vec::new();
    let teams = manifest.get("teams").unwrap();
    for team_index in 1..=2 {
        let team = teams.get_index(team_index).unwrap();
        let roster = team.get("roster").unwrap();
        for player_index in 1..=gc_sim::input_frame::FIXTURE_TEAM_SIZE {
            let Some(player) = roster.get_index(player_index) else {
                continue;
            };
            if player.get("position").and_then(Value::as_str) == Some("keeper") {
                keepers.push(
                    player
                        .get("player_id")
                        .and_then(Value::as_str)
                        .unwrap()
                        .to_string(),
                );
            }
        }
    }
    let slots = manifest.get("slots").unwrap();
    assert!(slots.get_index(1).unwrap().get("player_id").is_some());
    for index in 1..=slots.len() as i64 {
        let slot = slots.get_index(index).unwrap();
        let player_id = slot.get("player_id").and_then(Value::as_str).unwrap();
        assert!(
            !keepers.iter().any(|k| k == player_id),
            "a manifest slot named a keeper"
        );
    }

    let keeper_id = teams
        .get_index(1)
        .unwrap()
        .get("roster")
        .unwrap()
        .get_index(1)
        .unwrap()
        .get("player_id")
        .unwrap()
        .clone();
    let mut items: Vec<Value> = (1..=gc_sim::input_frame::SLOT_COUNT)
        .map(|i| fixture::assignments(1, None).get_index(i).unwrap().clone())
        .collect();
    items[0].set("player_id", keeper_id);
    let assignments = Value::array(items);

    let err = coordinator::slot_sources(&manifest, &assignments).unwrap_err();
    assert!(err.message.contains("keepers"));
    // The Lua reference (`game/online/coordinator.lua` line 614) returns the
    // coordinator-local `"invalid_assignment"` for this exact rule — the
    // same code every other coordinator-local ownership invariant in this
    // module uses. This port's `slot_sources` (`coordinator.rs` around line
    // 1319) instead returns `RejectCode::InvalidOwnership`
    // ("invalid_ownership"), `protocol.rs`'s own structural-validation code.
    // Expected: `RejectCode::InvalidAssignment`. Actual:
    // `RejectCode::InvalidOwnership`.
    assert_eq!(
        err.code,
        coordinator::RejectCode::InvalidAssignment,
        "suspected coordinator defect: slot_sources's keeper rule returns \
         RejectCode::InvalidOwnership (\"invalid_ownership\") where the Lua \
         reference uses \"invalid_assignment\" (RejectCode::InvalidAssignment)"
    );
}

#[test]
fn derives_bot_seeds_deterministically_from_the_frozen_manifest_seed() {
    let manifest = fixture::manifest(None);
    let first =
        coordinator::plan_assignments(&manifest, &[fixture::HOST_PEER_ID.to_string()]).unwrap();
    let second =
        coordinator::plan_assignments(&manifest, &[fixture::HOST_PEER_ID.to_string()]).unwrap();
    let seed = manifest.get("seed").and_then(Value::as_int).unwrap();
    for index in 2..=gc_sim::input_frame::SLOT_COUNT {
        let first_seed = first.get_index(index).unwrap().get("bot_seed").cloned();
        let second_seed = second.get_index(index).unwrap().get("bot_seed").cloned();
        assert_eq!(first_seed, second_seed);
        assert_eq!(
            first_seed,
            Some(Value::int(
                (seed + index * coordinator::BOT_SEED_STRIDE).rem_euclid(protocol::MAX_SEED + 1)
            ))
        );
    }
}

#[test]
fn refuses_to_seat_more_humans_than_canonical_slots() {
    let ids: Vec<String> = (1..=gc_sim::input_frame::SLOT_COUNT + 1)
        .map(|i| format!("peer.{i}"))
        .collect();
    let result = coordinator::plan_assignments(&fixture::manifest(None), &ids);
    assert_eq!(result.unwrap_err().code, coordinator::RejectCode::Capacity);

    let result = coordinator::plan_assignments(
        &fixture::manifest(None),
        &[
            fixture::HOST_PEER_ID.to_string(),
            fixture::HOST_PEER_ID.to_string(),
        ],
    );
    assert_eq!(
        result.unwrap_err().code,
        coordinator::RejectCode::DuplicatePeer
    );
}

// ---------------------------------------------------------------------------
// Admission.
// ---------------------------------------------------------------------------

#[test]
fn admits_guests_with_stable_identities() {
    let state = fixture::host(None);
    let (next_state, outcome) = deliver(&state, "guest.1", handshake("guest.1", 0, Role::Guest));
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next_state.peers.len(), 2);
    assert_eq!(next_state.peers[1].peer_id, "guest.1");
    assert_eq!(
        next_state.peers[1].link_id.as_deref(),
        Some(fixture::link_id("guest.1").as_str())
    );
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Handshake);
    assert_eq!(
        state.phase,
        protocol::LifecyclePhase::Handshake,
        "the original state is untouched"
    );
    assert_eq!(state.peers.len(), 1);
}

#[test]
fn refuses_duplicate_over_capacity_role_and_runtime_violations_per_link() {
    let mut state = fixture::host(None);
    for index in 1..=coordinator::MAX_GUESTS {
        let peer_id = fixture::guest_peer_id(index);
        state = deliver(&state, &peer_id, handshake(&peer_id, 0, Role::Guest)).0;
    }
    assert_eq!(state.peers.len() as i64, coordinator::MAX_PEERS);

    let (overflow, outcome) = deliver(&state, "guest.8", handshake("guest.8", 0, Role::Guest));
    assert_eq!(outcome.code, Some(coordinator::RejectCode::Capacity));
    assert_eq!(overflow.peers.len() as i64, coordinator::MAX_PEERS);
    assert_eq!(overflow.phase, protocol::LifecyclePhase::Handshake);
    let coordinator::Action::Send { message: sent, .. } = &outcome.actions[0] else {
        panic!("expected a send action");
    };
    assert_eq!(
        sent.body.get("code").and_then(Value::as_str),
        Some("capacity")
    );
    assert!(matches!(
        outcome.actions[1],
        coordinator::Action::Close { .. }
    ));

    let (duplicate, outcome) = coordinator::step(
        &state,
        Event::Control {
            link_id: fixture::link_id("guest.8"),
            message: Some(handshake("guest.1", 0, Role::Guest)),
            wire: None,
        },
    );
    // Wire code "protocol_mismatch" -> `RejectCode::UnsupportedVersion`: see
    // `reject_code_from_session_reject`, which has no closer
    // coordinator-local code for this wire vocabulary entry.
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::UnsupportedVersion)
    );
    assert_eq!(duplicate.peers.len() as i64, coordinator::MAX_PEERS);

    // The same handshake replayed on its own link is a plain duplicate.
    let (replay, outcome) = deliver(&state, "guest.1", handshake("guest.1", 0, Role::Guest));
    assert_eq!(outcome.disposition, coordinator::Disposition::Idempotent);
    assert_eq!(replay, state);

    let (host_claim, outcome) = deliver(
        &fixture::host(None),
        "guest.9",
        handshake("guest.9", 0, Role::Host),
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::UnsupportedVersion)
    );
    assert_eq!(host_claim.peers.len(), 1);

    let mut incompatible = fixture::runtime();
    incompatible.set("runtime_revision", Value::str("lovejs.11.5.other"));
    let (mismatch, outcome) = deliver(
        &fixture::host(None),
        "guest.9",
        message(
            protocol::MessageKind::Handshake,
            "guest.9",
            0,
            Value::record(vec![
                ("role", Value::str("guest")),
                ("runtime", incompatible),
            ]),
        ),
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::RuntimeMismatch));
    assert_eq!(mismatch.peers.len(), 1);
    assert_eq!(mismatch.terminal, None);
}

#[test]
fn closes_admission_once_the_manifest_is_proposed() {
    let (state, _) = assigned_host(1);
    let (next_state, outcome) = deliver(&state, "guest.7", handshake("guest.7", 0, Role::Guest));
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(next_state.peers.len(), 2);
    assert_eq!(next_state.terminal, None);
}

#[test]
fn refuses_non_handshake_traffic_from_an_unadmitted_link() {
    let state = fixture::host(None);
    let (next_state, outcome) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::ManifestAccept,
            "guest.1",
            0,
            Value::record(vec![("manifest_id", Value::str("0123456789abcdef"))]),
        ),
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(next_state.peers.len(), 1);
    assert_eq!(next_state.terminal, None);
}
