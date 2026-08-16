//! Behavioral and differential tests for the coordinator reducer and driver:
//! peer admission, manifest proposal, slot assignment, readiness, countdown,
//! mid-match hash-agreement tracking, and the terminal-reason paths those
//! produce.
//!
//! This file also carries the reducer's required differential evidence
//! (ARCHITECTURE.md §3 rule 7 / `tools/lua_reference/README.md`): a from-scratch
//! event sequence — connect, propose manifest, assign slots, set ready,
//! begin countdown, several ticks with agreeing hash reports, then a
//! deliberate three-tick boundary-hash disagreement — driven identically
//! through the reference implementation this netcode's wire behaviour was
//! validated against and through this reducer, comparing phase, terminal
//! reason, and mismatch counters on *both* peers at every step. See
//! [`coordinator_reducer_reproduces_the_lua_reference_rejection_and_desync_paths`]
//! for the important part: the happy path (`agree_tick_*`) is not the
//! interesting evidence here, the desync path is — the host detects the
//! third disagreement and terminates as `hash_mismatch` while the guest,
//! racing the announced abort, never reaches its own third count and ends
//! as `peer_abort` instead. A reducer that agreed only on the happy path
//! and diverged on this would be exactly the failure this reducer must not
//! ship.
//!
//! `tests/fixtures/coordinator_desync_lua_reference.txt` is the frozen,
//! non-regenerable captured output of that reference run. See
//! `tools/lua_reference/README.md` for how it was captured and what
//! guarantees it as byte-exact evidence.

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

/// The `transcript_id=...` line's zero-based index in [`FIXTURE`] (and in
/// this test's own `log`) — the one line asserted against
/// [`TRANSCRIPT_ID_BASELINE`] instead of the frozen fixture. See that
/// constant's doc comment for why.
const TRANSCRIPT_ID_LINE: usize = 13;

/// `session.transcript_id()` for this test's desync scenario, recorded from
/// THIS build rather than read off [`FIXTURE`].
///
/// Self-recorded, NOT the retired Lua value — this is the same
/// schema-coupled situation `tools/lua_reference/README.md`'s third axis
/// documents (see the note on `coordinator_conformance::Golden` in
/// `gc-netcode/src/coordinator_conformance.rs`, retired for the identical
/// reason in the same PR): `protocol::validate_manifest` embeds
/// `match_snapshot::COMBAT_VERSION` in every manifest, #489 bumps that
/// constant 13 -> 14, and `transcript_id` digests the full wire bytes of
/// every message including the manifest. No other line in [`FIXTURE`]
/// depends on a version word — `message_count` counts messages regardless of
/// their content, and every phase/terminal/mismatch-counter line records
/// reducer *decisions*, not wire bytes — so only this one line is retired;
/// the fixture file itself, and every other line's comparison against it,
/// are unmodified and still gate.
///
/// Re-recorded by reading this assertion's own failure output — a single
/// value, so no separate `#[ignore]`d recorder is warranted (same reasoning
/// as `match_driver.rs`'s `BOUNDARY_ZERO_BASELINE_HASH`). Re-record only
/// when a deliberate, reviewed wire-schema change moves it — never to clear
/// a check that surprised you.
const TRANSCRIPT_ID_BASELINE: &str = "transcript_id=a44ebac2fc25d349";

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
        if index == TRANSCRIPT_ID_LINE {
            // #489: schema-coupled, retired to a Rust-recorded baseline.
            // See `TRANSCRIPT_ID_BASELINE`'s doc comment.
            assert_eq!(
                ours, TRANSCRIPT_ID_BASELINE,
                "transcript_id no longer matches its Rust-recorded baseline"
            );
            continue;
        }
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
// Below: further coverage of coordinator behavior beyond the two large
// differential/golden tests documented in this file's module doc comment.
// The tests and helpers below are grouped and named to track the reference
// implementation's own test structure and local helpers (`message`,
// `handshake`, `deliver`, `assigned_host`, `ready_host`).
// ===========================================================================

use gc_netcode::coordinator::CoordinatorState;

/// The fixture session id every coordinator in this file shares (mirrors the
/// reference implementation's `SESSION = fixture.manifest().session_id`).
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
/// every acceptance, and published canonical ownership. Mirrors the
/// reference implementation's `assigned_host`.
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
/// readiness is set. Mirrors the reference implementation's `ready_host`.
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

    // `role = "spectator"`: the reference implementation runtime-checks a
    // raw string role field and rejects it as malformed. `Options::role`
    // here is a Rust `Role` enum with only `Host`/`Guest` variants, so this
    // bad value is unconstructible in this codebase — the enum itself
    // stands in for that check (precedent:
    // `gc-sim/tests/possession_transition.rs`, `content_validation.rs`).

    // A guest without a host link id: malformed. (Already exercised by
    // `new_refuses_a_guest_without_a_host_link`; re-asserted here so this
    // test group fully covers what the reference implementation covers.)
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
            // `slot_sources` returns a slot-id-keyed record (the reference
            // implementation indexes it as `sources[slot.id]`), not the
            // 1-based array `plan_assignments`/`assignments` returns.
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
    // The reference implementation (see `tools/lua_reference/README.md`;
    // historically at `coordinator.lua` line 614) returns the
    // coordinator-local `"invalid_assignment"` for this exact rule — the
    // same code every other coordinator-local ownership invariant in this
    // module uses. This crate's `slot_sources` (`coordinator.rs` around
    // line 1319) instead returns `RejectCode::InvalidOwnership`
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

// ---------------------------------------------------------------------------
// Configuration.
// ---------------------------------------------------------------------------

#[test]
fn requires_manifest_acceptance_before_ownership_and_readiness() {
    let mut state = fixture::host(None);
    state = deliver(&state, "guest.1", handshake("guest.1", 0, Role::Guest)).0;
    state = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    )
    .0;
    assert_eq!(state.peers[1].accepted_manifest_id, None);

    let (blocked, outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(blocked, state, "ownership waits for every acceptance");

    let manifest_id = state.manifest_id.clone().unwrap();
    let (early, _) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Ready,
            "guest.1",
            1,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                // No ownership exists yet; the phase gate refuses this
                // before any generation check can apply.
                ("assignment_id", Value::str("0123456789abcdef")),
                ("ready", Value::bool(true)),
            ]),
        ),
    );
    assert_eq!(early.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        early.terminal.unwrap().code.as_deref(),
        Some("invalid_phase")
    );

    state = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::ManifestAccept,
            "guest.1",
            1,
            Value::record(vec![("manifest_id", Value::str(manifest_id.clone()))]),
        ),
    )
    .0;
    state = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    )
    .0;
    assert_eq!(state.phase, protocol::LifecyclePhase::Assigned);
    // Readiness that names any other ownership generation is refused.
    let bogus_assignment_id = protocol::assignment_id(&fixture::assignments(1, None), 99);
    let (superseded, outcome) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Ready,
            "guest.1",
            2,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("assignment_id", Value::str(bogus_assignment_id)),
                ("ready", Value::bool(true)),
            ]),
        ),
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::InvalidAssignment),
        "readiness must answer for this ownership"
    );
    assert_eq!(
        outcome.reason.as_deref(),
        Some(coordinator::STALE_GENERATION_REASON)
    );
    assert_eq!(superseded, state, "a superseded answer leaves no progress");

    let mut sequences = vec![2i64];
    state = ready_host(state, 1, &mut sequences);
    assert!(state.peers[1].ready);
    assert_eq!(state.phase, protocol::LifecyclePhase::Ready);
}

#[test]
fn holds_the_acceptance_invariant_even_if_ownership_was_published_early() {
    // White-box guard: the readiness barrier does not depend on the
    // assignment-time acceptance rule for its own correctness.
    let (mut state, _) = assigned_host(1);
    state.peers[1].accepted_manifest_id = None;
    let manifest_id = state.manifest_id.clone().unwrap();
    let assignment_id = state.assignment_id.clone().unwrap();
    let (next_state, outcome) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Ready,
            "guest.1",
            2,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("assignment_id", Value::str(assignment_id)),
                ("ready", Value::bool(true)),
            ]),
        ),
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        next_state.terminal.unwrap().detail.as_deref(),
        Some("readiness preceded manifest acceptance")
    );
}

#[test]
fn requires_slot_ownership_before_readiness() {
    let mut state = fixture::host(None);
    let (_, outcome) = coordinator::step(&state, Event::SetReady { ready: true });
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));

    state = deliver(&state, "guest.1", handshake("guest.1", 0, Role::Guest)).0;
    state = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    )
    .0;
    let (_, outcome) = coordinator::step(&state, Event::SetReady { ready: true });
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
}

#[test]
fn clears_readiness_whenever_ownership_changes() {
    let (state, mut sequences) = assigned_host(2);
    let state = ready_host(state, 2, &mut sequences);
    assert_eq!(state.phase, protocol::LifecyclePhase::Ready);

    let mut swapped_items: Vec<Value> = (1..=gc_sim::input_frame::SLOT_COUNT)
        .map(|i| fixture::assignments(2, None).get_index(i).unwrap().clone())
        .collect();
    swapped_items[0].set("producer_id", Value::str("guest.1"));
    swapped_items[1].set("producer_id", Value::str(fixture::HOST_PEER_ID));
    let swapped = Value::array(swapped_items);

    let (next_state, outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: swapped.clone(),
            preserve_claims: false,
        },
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Assigned);
    for peer in &next_state.peers {
        assert!(!peer.ready);
    }

    let (same, outcome) = coordinator::step(
        &next_state,
        Event::AssignSlots {
            assignments: swapped,
            preserve_claims: false,
        },
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Idempotent);
    assert_eq!(same, next_state);
}

#[test]
fn lets_a_peer_revoke_readiness_before_the_countdown() {
    let (state, mut sequences) = assigned_host(1);
    let state = ready_host(state, 1, &mut sequences);
    assert_eq!(state.phase, protocol::LifecyclePhase::Ready);

    sequences[0] += 1;
    let manifest_id = state.manifest_id.clone().unwrap();
    let assignment_id = state.assignment_id.clone().unwrap();
    let (next_state, outcome) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Ready,
            "guest.1",
            sequences[0],
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("assignment_id", Value::str(assignment_id)),
                ("ready", Value::bool(false)),
            ]),
        ),
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Assigned);
    assert!(!next_state.peers[1].ready);
}

#[test]
fn rejects_a_slot_assignment_that_leaves_a_peer_unseated() {
    let (state, _) = assigned_host(2);
    let orphaned = fixture::assignments(1, None);
    let (_, outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: orphaned,
            preserve_claims: false,
        },
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::InvalidAssignment)
    );

    let mut impostor_items: Vec<Value> = (1..=gc_sim::input_frame::SLOT_COUNT)
        .map(|i| fixture::assignments(2, None).get_index(i).unwrap().clone())
        .collect();
    impostor_items[2].set("producer_id", Value::str("guest.9"));
    let impostor = Value::array(impostor_items);
    let (_, outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: impostor,
            preserve_claims: false,
        },
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::InvalidAssignment)
    );
}

// ---------------------------------------------------------------------------
// Countdown and start.
// ---------------------------------------------------------------------------

#[test]
fn freezes_ownership_and_names_one_start_boundary() {
    let (state, mut sequences) = assigned_host(1);
    let state = ready_host(state, 1, &mut sequences);
    let (next_state, outcome) = coordinator::step(
        &state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 2,
            first_input_tick: 12,
        },
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Countdown);
    let freeze = next_state.freeze.clone().unwrap();
    assert_eq!(freeze.first_input_tick, 12);
    assert_eq!(
        freeze.seed,
        fixture::manifest(None)
            .get("seed")
            .and_then(Value::as_int)
            .unwrap()
    );
    assert_eq!(Some(freeze.manifest_id.clone()), next_state.manifest_id);
    assert_eq!(
        freeze.combat_rules_id,
        fixture::manifest(None)
            .get("combat_rules_id")
            .and_then(Value::as_str)
            .unwrap()
    );
    assert_eq!(
        freeze.assignments.len() as i64,
        gc_sim::input_frame::SLOT_COUNT
    );

    // Freezing takes a copy: Rust's ownership model makes the reference
    // implementation's "mutate the live table, prove the freeze is
    // untouched" check structurally impossible to fail here (there is no
    // aliasing to leak through) — the same reasoning applies here as for an
    // enum-unconstructible bad value elsewhere in this file. The meaningful
    // residual assertion is that the frozen slot still names its real
    // owner.
    assert_eq!(
        freeze
            .assignments
            .get_index(1)
            .unwrap()
            .get("producer_id")
            .and_then(Value::as_str),
        Some(fixture::HOST_PEER_ID)
    );

    let (blocked, outcome) = coordinator::step(
        &next_state,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(blocked, next_state);

    let (blocked, outcome) = coordinator::step(&next_state, Event::SetReady { ready: false });
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(blocked, next_state);
}

#[test]
fn publishes_start_only_when_the_fake_clock_drains_the_countdown() {
    let (state, mut sequences) = assigned_host(1);
    let mut state = ready_host(state, 1, &mut sequences);
    state = coordinator::step(
        &state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 2,
            first_input_tick: 0,
        },
    )
    .0;
    let (next_state, outcome) = coordinator::step(&state, Event::Tick);
    assert_eq!(outcome.actions.len(), 0);
    assert_eq!(next_state.countdown_remaining, Some(1));
    state = next_state;
    let (next_state, outcome) = coordinator::step(&state, Event::Tick);
    assert_eq!(next_state.countdown_remaining, Some(0));
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Countdown);
    let coordinator::Action::Send { message: sent, .. } = &outcome.actions[0] else {
        panic!("expected a send action");
    };
    assert_eq!(sent.kind, protocol::MessageKind::Start);
    assert_eq!(
        sent.body.get("first_input_tick").and_then(Value::as_int),
        Some(0)
    );
    state = next_state;

    sequences[0] += 1;
    let manifest_id = state.manifest_id.clone().unwrap();
    let (started, outcome) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Start,
            "guest.1",
            sequences[0],
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
    );
    assert_eq!(started.phase, protocol::LifecyclePhase::Running);
    let coordinator::Action::StartMatch { freeze } = &outcome.actions[0] else {
        panic!("expected a start_match action");
    };
    assert_eq!(freeze.first_input_tick, 0);
}

#[test]
fn does_not_double_start_on_a_duplicated_acknowledgement() {
    let (state, mut sequences) = assigned_host(1);
    let mut state = ready_host(state, 1, &mut sequences);
    state = coordinator::step(
        &state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 0,
            first_input_tick: 0,
        },
    )
    .0;
    sequences[0] += 1;
    let manifest_id = state.manifest_id.clone().unwrap();
    let ack = message(
        protocol::MessageKind::Start,
        "guest.1",
        sequences[0],
        Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
            ("first_input_tick", Value::int(0)),
        ]),
    );
    let (next_state, outcome) = deliver(&state, "guest.1", ack.clone());
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Running);
    assert!(matches!(
        outcome.actions[0],
        coordinator::Action::StartMatch { .. }
    ));

    let (repeated, outcome) = deliver(&next_state, "guest.1", ack);
    assert_eq!(outcome.disposition, coordinator::Disposition::Idempotent);
    assert_eq!(outcome.actions.len(), 0);
    assert_eq!(repeated, next_state);
}

#[test]
fn rejects_a_start_that_misnames_the_frozen_boundary() {
    let (state, mut sequences) = assigned_host(1);
    let mut state = ready_host(state, 1, &mut sequences);
    state = coordinator::step(
        &state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 0,
            first_input_tick: 0,
        },
    )
    .0;
    sequences[0] += 1;
    let manifest_id = state.manifest_id.clone().unwrap();
    let (next_state, _) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Start,
            "guest.1",
            sequences[0],
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("first_input_tick", Value::int(9)),
            ]),
        ),
    );
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        next_state.terminal.unwrap().reason,
        TerminalReason::ProtocolViolation
    );
}

// ---------------------------------------------------------------------------
// Match lifecycle.
// ---------------------------------------------------------------------------

/// A host that has reached the running phase with one guest. Mirrors the
/// reference implementation's local `running_host`.
fn running_host() -> (CoordinatorState, Vec<i64>) {
    let (state, mut sequences) = assigned_host(1);
    let mut state = ready_host(state, 1, &mut sequences);
    state = coordinator::step(
        &state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 0,
            first_input_tick: 0,
        },
    )
    .0;
    sequences[0] += 1;
    let manifest_id = state.manifest_id.clone().unwrap();
    state = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Start,
            "guest.1",
            sequences[0],
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
    )
    .0;
    (state, sequences)
}

#[test]
fn acknowledges_simulation_phases_without_authoring_them() {
    let (mut state, _) = running_host();
    let (next_state, outcome) = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "kickoff".to_string(),
            tick: 0,
            home_score: 0,
            away_score: 0,
        },
    );
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(next_state.progress.as_ref().unwrap().phase, "kickoff");
    state = next_state;

    let (rejected_state, outcome) = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "goal_stoppage".to_string(),
            tick: 30,
            home_score: 1,
            away_score: 0,
        },
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::InvalidPhase),
        "a goal cannot follow kickoff directly"
    );
    assert_eq!(rejected_state, state);

    state = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "playing".to_string(),
            tick: 1,
            home_score: 0,
            away_score: 0,
        },
    )
    .0;
    let (_, outcome) = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "goal_stoppage".to_string(),
            tick: 30,
            home_score: 0,
            away_score: 0,
        },
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::Malformed),
        "a stoppage must follow a scored goal"
    );

    let (_, outcome) = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "playing".to_string(),
            tick: 0,
            home_score: 0,
            away_score: 0,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));

    state = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "goal_stoppage".to_string(),
            tick: 30,
            home_score: 1,
            away_score: 0,
        },
    )
    .0;
    let (rejected_state, outcome) = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "kickoff".to_string(),
            tick: 31,
            home_score: 0,
            away_score: 0,
        },
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::Malformed),
        "scores never move backwards"
    );
    assert_eq!(rejected_state, state);
}

#[test]
fn never_restates_the_simulation_score_at_full_time() {
    let (state, _) = running_host();
    let state = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "kickoff".to_string(),
            tick: 0,
            home_score: 0,
            away_score: 0,
        },
    )
    .0;
    let (early, outcome) = coordinator::step(
        &state,
        Event::Finish {
            final_tick: 10,
            home_score: 0,
            away_score: 0,
            final_hash: "fedcba9876543210".to_string(),
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(early, state);

    let state = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "playing".to_string(),
            tick: 1,
            home_score: 0,
            away_score: 0,
        },
    )
    .0;
    let state = coordinator::step(
        &state,
        Event::MatchPhase {
            phase: "full_time".to_string(),
            tick: 7200,
            home_score: 2,
            away_score: 1,
        },
    )
    .0;

    let (lied, outcome) = coordinator::step(
        &state,
        Event::Finish {
            final_tick: 7200,
            home_score: 3,
            away_score: 1,
            final_hash: "fedcba9876543210".to_string(),
        },
    );
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::IdentityMismatch)
    );
    assert_eq!(lied, state);

    let (finished, outcome) = coordinator::step(
        &state,
        Event::Finish {
            final_tick: 7200,
            home_score: 2,
            away_score: 1,
            final_hash: "fedcba9876543210".to_string(),
        },
    );
    assert_eq!(finished.phase, protocol::LifecyclePhase::Result);
    assert_eq!(finished.result.as_ref().unwrap().home_score, 2);
    let coordinator::Action::Send { message: first, .. } = &outcome.actions[0] else {
        panic!("expected a send action");
    };
    assert_eq!(first.kind, protocol::MessageKind::MatchPhase);
    let coordinator::Action::Send {
        message: second, ..
    } = &outcome.actions[1]
    else {
        panic!("expected a send action");
    };
    assert_eq!(second.kind, protocol::MessageKind::ResultAck);
}

#[test]
fn completes_only_when_every_peer_acknowledges_the_same_result() {
    let (mut state, sequences) = running_host();
    for (phase, tick, home) in [("kickoff", 0, 0), ("playing", 1, 0), ("full_time", 7200, 1)] {
        state = coordinator::step(
            &state,
            Event::MatchPhase {
                phase: phase.to_string(),
                tick,
                home_score: home,
                away_score: 0,
            },
        )
        .0;
    }
    state = coordinator::step(
        &state,
        Event::Finish {
            final_tick: 7200,
            home_score: 1,
            away_score: 0,
            final_hash: "fedcba9876543210".to_string(),
        },
    )
    .0;
    assert_eq!(state.phase, protocol::LifecyclePhase::Result);

    let (wrong, _) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::ResultAck,
            "guest.1",
            sequences[0] + 1,
            Value::record(vec![
                ("final_tick", Value::int(7200)),
                ("home_score", Value::int(1)),
                ("away_score", Value::int(1)),
                ("final_hash", Value::str("fedcba9876543210")),
            ]),
        ),
    );
    assert_eq!(wrong.terminal.unwrap().reason, TerminalReason::HashMismatch);

    let (done, _) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::ResultAck,
            "guest.1",
            sequences[0] + 1,
            Value::record(vec![
                ("final_tick", Value::int(7200)),
                ("home_score", Value::int(1)),
                ("away_score", Value::int(0)),
                ("final_hash", Value::str("fedcba9876543210")),
            ]),
        ),
    );
    assert_eq!(done.phase, protocol::LifecyclePhase::Terminal);
    let terminal = done.terminal.unwrap();
    assert_eq!(terminal.reason, TerminalReason::Completed);
    assert_eq!(terminal.code, None);
}

// ---------------------------------------------------------------------------
// Duplicate and invalid traffic.
// ---------------------------------------------------------------------------

#[test]
fn treats_a_byte_identical_replay_as_a_no_op() {
    let state = fixture::host(None);
    let admission = handshake("guest.1", 0, Role::Guest);
    let (first, _) = deliver(&state, "guest.1", admission.clone());
    let (second, outcome) = deliver(&first, "guest.1", admission);
    assert_eq!(outcome.disposition, coordinator::Disposition::Idempotent);
    assert_eq!(outcome.actions.len(), 0);
    assert_eq!(second, first, "a duplicate never advances the session");
    assert_eq!(second.peers.len(), 2);
}

#[test]
fn treats_reused_transcript_identity_with_new_bytes_as_terminal() {
    let (state, sequences) = assigned_host(1);
    let manifest_id = state.manifest_id.clone().unwrap();
    let conflict = message(
        protocol::MessageKind::Ready,
        "guest.1",
        sequences[0],
        Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            (
                "assignment_id",
                Value::str(state.assignment_id.clone().unwrap()),
            ),
            ("ready", Value::bool(true)),
        ]),
    );
    let (next_state, _) = deliver(&state, "guest.1", conflict);
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Terminal);
    let terminal = next_state.terminal.unwrap();
    assert_eq!(terminal.reason, TerminalReason::ProtocolViolation);
    assert_eq!(terminal.code.as_deref(), Some("malformed_message"));
}

#[test]
fn treats_an_out_of_phase_message_as_terminal() {
    let mut state = fixture::host(None);
    state = deliver(&state, "guest.1", handshake("guest.1", 0, Role::Guest)).0;
    let (next_state, _) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::Ready,
            "guest.1",
            1,
            Value::record(vec![
                ("manifest_id", Value::str("0123456789abcdef")),
                ("assignment_id", Value::str("0123456789abcdef")),
                ("ready", Value::bool(true)),
            ]),
        ),
    );
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        next_state.terminal.unwrap().code.as_deref(),
        Some("invalid_phase")
    );
}

#[test]
fn treats_a_spoofed_peer_identity_or_wrong_direction_as_terminal() {
    let (state, _) = assigned_host(2);
    let manifest_id = state.manifest_id.clone().unwrap();
    let (spoofed, _) = coordinator::step(
        &state,
        Event::Control {
            link_id: fixture::link_id("guest.1"),
            message: Some(message(
                protocol::MessageKind::ManifestAccept,
                "guest.2",
                5,
                Value::record(vec![("manifest_id", Value::str(manifest_id.clone()))]),
            )),
            wire: None,
        },
    );
    assert_eq!(spoofed.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        spoofed.terminal.unwrap().detail.as_deref(),
        Some("control message claims another peer identity")
    );

    let (wrong_way, _) = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::ManifestProposal,
            "guest.1",
            5,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("manifest", fixture::manifest(None)),
            ]),
        ),
    );
    assert_eq!(wrong_way.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        wrong_way.terminal.unwrap().reason,
        TerminalReason::ProtocolViolation
    );
}

#[test]
fn treats_malformed_or_foreign_session_wire_as_a_per_link_refusal() {
    let state = fixture::host(None);
    let (next_state, outcome) =
        coordinator::receive(&state, &fixture::link_id("guest.1"), "GCOP;1;junk");
    assert_eq!(outcome.disposition, coordinator::Disposition::Rejected);
    assert_eq!(next_state.terminal, None);
    assert_eq!(next_state.peers.len(), 1);

    let foreign = protocol::new(
        protocol::MessageKind::Handshake,
        "other_session",
        "guest.1",
        0,
        Value::record(vec![
            ("role", Value::str("guest")),
            ("runtime", fixture::runtime()),
        ]),
    )
    .unwrap();
    let (refused, outcome) = deliver(&state, "guest.1", foreign);
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::UnsupportedVersion)
    );
    assert_eq!(refused.terminal, None);
}

#[test]
fn still_ends_the_session_mid_lobby_on_a_kind_this_build_never_heard_of() {
    // Folding the vocabulary into `build_id` moves this failure to the
    // manifest check for peers that compute one. It does not replace it: a
    // hand-written client, or anything that reaches an admitted peer with
    // traffic this build cannot read, still meets the announced termination
    // that has always been here.
    let (state, _) = assigned_host(1);
    let manifest_id = state.manifest_id.clone().unwrap();
    let assignment_id = state.assignment_id.clone().unwrap();
    let wire = protocol::encode(&message(
        protocol::MessageKind::Ready,
        "guest.1",
        2,
        Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("assignment_id", Value::str(assignment_id)),
            ("ready", Value::bool(true)),
        ]),
    ))
    .unwrap();
    // `relay` is exactly as long as `ready`, so every canonical length
    // prefix still holds and the wire is well formed in every way except
    // naming a kind this build has no rule for.
    let forged = wire.replacen("s4:kinds5:ready", "s4:kinds5:relay", 1);
    assert_ne!(forged, wire);

    let (next_state, outcome) = coordinator::receive(&state, &fixture::link_id("guest.1"), &forged);
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Terminal);
    let terminal = next_state.terminal.clone().unwrap();
    assert_eq!(terminal.reason, TerminalReason::ProtocolViolation);
    assert_eq!(terminal.code.as_deref(), Some("malformed_message"));
    assert_eq!(terminal.origin, coordinator::Origin::Remote);
    let mut announced = None;
    for action in &outcome.actions {
        if let coordinator::Action::Send { message, .. } = action
            && message.kind == protocol::MessageKind::Abort
        {
            announced = message.body.get("code").and_then(Value::as_str);
        }
    }
    assert_eq!(
        announced,
        Some("malformed_message"),
        "the termination is still announced"
    );
}

#[test]
fn refuses_unknown_events_and_post_terminal_traffic() {
    // The reference implementation also sends `{ kind = "teleport" }`
    // against a fresh host and expects `unknown_message`. `Event` here is a
    // closed Rust enum matched exhaustively in `step`, so an unrecognized
    // *local event kind* is unconstructible here (the same
    // enum-unconstructible situation noted elsewhere for a bad wire value;
    // see `gc-sim/tests/possession_transition.rs`,
    // `content_validation.rs`). The closest faithful equivalent reachable
    // through the public API is `receive`'s wire-level unknown *message*
    // kind path, already exercised by "still ends the session mid-lobby on
    // a kind this build never heard of" above. This test covers the rest
    // of that case: post-terminal traffic.
    let state = fixture::host(None);

    let (ended, _) = coordinator::step(
        &state,
        Event::Abort {
            code: Some("host_abort".to_string()),
            detail: None,
        },
    );
    assert_eq!(ended.phase, protocol::LifecyclePhase::Terminal);
    let (after, outcome) = coordinator::step(
        &ended,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(after, ended);

    let (ticked, outcome) = coordinator::step(&ended, Event::Tick);
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(ticked.clock, ended.clock + 1);
    assert_eq!(ticked.phase, protocol::LifecyclePhase::Terminal);
}

// ---------------------------------------------------------------------------
// Guest validation.
// ---------------------------------------------------------------------------

#[test]
fn accepts_a_matching_manifest_and_refuses_a_foreign_identity() {
    let guest = fixture::guest(1, None, None);
    let guest = coordinator::step(&guest, Event::Connect).0;
    assert_eq!(guest.phase, protocol::LifecyclePhase::Handshake);

    let manifest = fixture::manifest(None);
    let manifest_id = protocol::manifest_id(&manifest);
    let (accepted, outcome) = deliver(
        &guest,
        "guest.1",
        message(
            protocol::MessageKind::ManifestProposal,
            fixture::HOST_PEER_ID,
            0,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("manifest", manifest),
            ]),
        ),
    );
    assert_eq!(accepted.phase, protocol::LifecyclePhase::Manifest);
    assert_eq!(accepted.peers[0].accepted_manifest_id, Some(manifest_id));
    let coordinator::Action::Send { message: sent, .. } = &outcome.actions[0] else {
        panic!("expected a send action");
    };
    assert_eq!(sent.kind, protocol::MessageKind::ManifestAccept);

    let mut other = fixture::manifest(None);
    other.set("content_id", Value::str("content.other.v1"));
    let fresh_guest = coordinator::step(&fixture::guest(1, None, None), Event::Connect).0;
    let (refused, _) = deliver(
        &fresh_guest,
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
    assert_eq!(refused.phase, protocol::LifecyclePhase::Terminal);
    let terminal = refused.terminal.unwrap();
    assert_eq!(terminal.reason, TerminalReason::ManifestMismatch);
    assert_eq!(
        terminal.detail.as_deref(),
        Some("local identity differs at manifest.content_id")
    );
}

#[test]
fn reports_the_first_differing_expectation_field() {
    let manifest = fixture::manifest(None);
    assert_eq!(coordinator::expectation_difference(None, &manifest), None);
    assert_eq!(
        coordinator::expectation_difference(Some(&fixture::expectation()), &manifest),
        None
    );
    let mut expectation = fixture::expectation();
    expectation.build_id = Some("build.other".to_string());
    expectation.arena_id = Some("arena.other".to_string());
    let difference = coordinator::expectation_difference(Some(&expectation), &manifest).unwrap();
    assert_eq!(difference.path, "manifest.build_id");
    assert_eq!(difference.actual, *manifest.get("build_id").unwrap());
}

#[test]
fn names_a_build_disagreement_as_one_and_every_other_identity_as_a_manifest() {
    // `build_id` and `source_id` are the two expectation fields derived from
    // the build and its control vocabulary, so they get the reason whose fix
    // is "install the same build on both". The rest are content or
    // configuration disagreements between builds that could have played.
    let cases = [
        ("build_id", "build.other", TerminalReason::BuildMismatch),
        ("source_id", "source.other", TerminalReason::BuildMismatch),
        (
            "content_id",
            "content.other.v1",
            TerminalReason::ManifestMismatch,
        ),
        (
            "tuning_id",
            "tuning.other.v1",
            TerminalReason::ManifestMismatch,
        ),
        ("arena_id", "arena.other", TerminalReason::ManifestMismatch),
    ];
    for (field, value, reason) in cases {
        let mut other = fixture::manifest(None);
        other.set(field, Value::str(value));
        let guest = coordinator::step(&fixture::guest(1, None, None), Event::Connect).0;
        let (refused, outcome) = deliver(
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
        assert_eq!(
            refused.phase,
            protocol::LifecyclePhase::Terminal,
            "{field} has to end the session"
        );
        let terminal = refused.terminal.clone().unwrap();
        assert_eq!(terminal.reason, reason, "{field} reported the wrong reason");
        assert_eq!(
            terminal.detail.as_deref(),
            Some(format!("local identity differs at manifest.{field}").as_str())
        );
        // The wire vocabulary is untouched: both reasons announce the same
        // closed #161 code, so a peer on either side of the split reads the
        // session ending identically.
        assert_eq!(
            terminal.code.as_deref(),
            Some("manifest_mismatch"),
            "{field} reported the wrong wire code"
        );
        let mut announced = None;
        for action in &outcome.actions {
            if let coordinator::Action::Send { message, .. } = action
                && message.kind == protocol::MessageKind::Abort
            {
                announced = message.body.get("code").and_then(Value::as_str);
            }
        }
        assert_eq!(
            announced,
            Some("manifest_mismatch"),
            "{field} announced the wrong code"
        );
    }
}

#[test]
fn refuses_ownership_that_seats_no_local_slot() {
    let guest = coordinator::step(&fixture::guest(1, None, None), Event::Connect).0;
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
    let (next_state, _) = deliver(
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
                    Value::str(protocol::assignment_id(&fixture::assignments(1, None), 1)),
                ),
                ("assignments", unowned),
            ]),
        ),
    );
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Terminal);
    assert_eq!(
        next_state.terminal.unwrap().reason,
        TerminalReason::InvalidAssignment
    );
}

#[test]
fn revokes_readiness_when_the_host_republishes_ownership() {
    let guest = coordinator::step(&fixture::guest(1, None, None), Event::Connect).0;
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
    let guest = deliver(
        &guest,
        "guest.1",
        message(
            protocol::MessageKind::SlotAssignment,
            fixture::HOST_PEER_ID,
            1,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                (
                    "assignment_id",
                    Value::str(protocol::assignment_id(&fixture::assignments(1, None), 1)),
                ),
                ("assignments", fixture::assignments(1, None)),
            ]),
        ),
    )
    .0;
    assert_eq!(guest.phase, protocol::LifecyclePhase::Assigned);
    let first_generation = guest.assignment_id.clone().unwrap();
    let (guest, outcome) = coordinator::step(&guest, Event::SetReady { ready: true });
    assert_eq!(guest.phase, protocol::LifecyclePhase::Ready);
    let coordinator::Action::Send { message: sent, .. } = &outcome.actions[0] else {
        panic!("expected a send action");
    };
    assert_eq!(sent.body.get("ready").and_then(Value::as_bool), Some(true));
    assert_eq!(
        sent.body.get("assignment_id").and_then(Value::as_str),
        Some(first_generation.as_str()),
        "readiness names the generation the guest holds"
    );

    let mut swapped_items: Vec<Value> = (1..=gc_sim::input_frame::SLOT_COUNT)
        .map(|i| fixture::assignments(1, None).get_index(i).unwrap().clone())
        .collect();
    swapped_items[0].set("producer_id", Value::str("guest.1"));
    swapped_items[1].set("producer_id", Value::str(fixture::HOST_PEER_ID));
    let swapped = Value::array(swapped_items);
    let second_generation = protocol::assignment_id(&swapped, 2);
    let guest = deliver(
        &guest,
        "guest.1",
        message(
            protocol::MessageKind::SlotAssignment,
            fixture::HOST_PEER_ID,
            2,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("assignment_id", Value::str(second_generation.clone())),
                ("assignments", swapped),
            ]),
        ),
    )
    .0;
    assert_eq!(guest.phase, protocol::LifecyclePhase::Assigned);
    assert!(!guest.peers[0].ready);
    assert_eq!(guest.assignment_id, Some(second_generation.clone()));

    let (_, outcome) = coordinator::step(&guest, Event::SetReady { ready: true });
    let coordinator::Action::Send { message: sent, .. } = &outcome.actions[0] else {
        panic!("expected a send action");
    };
    assert_eq!(
        sent.body.get("assignment_id").and_then(Value::as_str),
        Some(second_generation.as_str())
    );
    assert_ne!(second_generation, first_generation);
}

// ---------------------------------------------------------------------------
// Transition matrix.
//
// Systematic coverage of the two transition dimensions the reducer defines:
// inbound (phase x control message kind) legality, and (phase x local event
// kind) totality. These are cross-products, not hand-picked scenarios, so a
// phase or kind added later cannot quietly escape coverage.
// ---------------------------------------------------------------------------

const GUEST: &str = "guest.1";

/// `guest_bodies(manifest_id)[kind]`.
fn guest_body(kind: protocol::MessageKind, manifest_id: &str) -> Value {
    use protocol::MessageKind::{
        Abort, Disconnect, Handshake, HashReport, ManifestAccept, Ready, ResultAck, Start,
    };
    match kind {
        Handshake => Value::record(vec![
            ("role", Value::str("guest")),
            ("runtime", fixture::runtime()),
        ]),
        ManifestAccept => Value::record(vec![("manifest_id", Value::str(manifest_id))]),
        Ready => Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("assignment_id", Value::str(manifest_id)),
            ("ready", Value::bool(true)),
        ]),
        Start => Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
            ("first_input_tick", Value::int(0)),
        ]),
        HashReport => Value::record(vec![
            ("tick", Value::int(60)),
            ("boundary_hash", Value::str("0123456789abcdef")),
        ]),
        ResultAck => Value::record(vec![
            ("final_tick", Value::int(7200)),
            ("home_score", Value::int(1)),
            ("away_score", Value::int(0)),
            ("final_hash", Value::str("fedcba9876543210")),
        ]),
        Abort => Value::record(vec![("code", Value::str("host_abort"))]),
        Disconnect => Value::record(vec![
            ("target_peer_id", Value::str(GUEST)),
            ("code", Value::str("peer_left")),
        ]),
        other => panic!("guest_body: unexpected kind {other:?}"),
    }
}

/// `host_bodies(manifest_id)[kind]`.
fn host_body(kind: protocol::MessageKind, manifest_id: &str) -> Value {
    use protocol::MessageKind::{
        Abort, Countdown, Disconnect, HashReport, ManifestProposal, MatchPhase, PeerAssignment,
        ResultAck, SlotAssignment, Start,
    };
    match kind {
        ManifestProposal => Value::record(vec![
            (
                "manifest_id",
                Value::str(protocol::manifest_id(&fixture::manifest(None))),
            ),
            ("manifest", fixture::manifest(None)),
        ]),
        PeerAssignment => Value::record(vec![
            ("assigned_peer_id", Value::str(GUEST)),
            ("role", Value::str("guest")),
        ]),
        SlotAssignment => Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("assignment_id", Value::str(manifest_id)),
            ("assignments", fixture::assignments(1, None)),
        ]),
        Countdown => Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
            ("remaining_ticks", Value::int(1)),
            ("first_input_tick", Value::int(0)),
        ]),
        MatchPhase => Value::record(vec![
            ("phase", Value::str("kickoff")),
            ("tick", Value::int(0)),
            ("home_score", Value::int(0)),
            ("away_score", Value::int(0)),
        ]),
        Disconnect => Value::record(vec![
            ("target_peer_id", Value::str(GUEST)),
            ("code", Value::str("host_left")),
        ]),
        Start | HashReport | ResultAck | Abort => guest_body(kind, manifest_id),
        other => panic!("host_body: unexpected kind {other:?}"),
    }
}

/// One host state per lifecycle phase, built through the real reducer.
/// Mirrors the reference implementation's local `host_states`. A `Vec` of
/// pairs, not a map (ARCHITECTURE.md §3 rule 4: never `HashMap`).
fn host_states() -> Vec<(protocol::LifecyclePhase, CoordinatorState)> {
    use protocol::LifecyclePhase::{
        Assigned, Countdown, Handshake, Manifest, Ready, Result, Running,
    };

    let mut states = Vec::new();
    let handshake_state = deliver(
        &fixture::host(None),
        GUEST,
        handshake(GUEST, 0, Role::Guest),
    )
    .0;
    states.push((Handshake, handshake_state));

    let (assigned_state, mut sequences) = assigned_host(1);
    let manifest_state = coordinator::step(
        &deliver(
            &fixture::host(None),
            GUEST,
            handshake(GUEST, 0, Role::Guest),
        )
        .0,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    )
    .0;
    states.push((Manifest, manifest_state));
    states.push((Assigned, assigned_state.clone()));

    let ready_state = ready_host(assigned_state, 1, &mut sequences);
    states.push((Ready, ready_state.clone()));

    let countdown_state = coordinator::step(
        &ready_state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 0,
            first_input_tick: 0,
        },
    )
    .0;
    states.push((Countdown, countdown_state.clone()));

    let manifest_id = countdown_state.manifest_id.clone().unwrap();
    let running_state = deliver(
        &countdown_state,
        GUEST,
        message(
            protocol::MessageKind::Start,
            GUEST,
            40,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
    )
    .0;
    states.push((Running, running_state.clone()));

    let mut state = running_state;
    for (phase, tick, home) in [("kickoff", 0, 0), ("playing", 1, 0), ("full_time", 7200, 1)] {
        state = coordinator::step(
            &state,
            Event::MatchPhase {
                phase: phase.to_string(),
                tick,
                home_score: home,
                away_score: 0,
            },
        )
        .0;
    }
    let result_state = coordinator::step(
        &state,
        Event::Finish {
            final_tick: 7200,
            home_score: 1,
            away_score: 0,
            final_hash: "fedcba9876543210".to_string(),
        },
    )
    .0;
    states.push((Result, result_state));

    states
}

/// One guest state per lifecycle phase, built through the real reducer.
/// Mirrors the reference implementation's local `guest_states`.
fn guest_states() -> Vec<(protocol::LifecyclePhase, CoordinatorState)> {
    use protocol::LifecyclePhase::{
        Assigned, Countdown, Handshake, Manifest, New, Ready, Result, Running,
    };

    let mut states = Vec::new();
    let new_state = fixture::guest(1, None, None);
    states.push((New, new_state.clone()));
    let handshake_state = coordinator::step(&new_state, Event::Connect).0;
    states.push((Handshake, handshake_state.clone()));

    let manifest = fixture::manifest(None);
    let manifest_id = protocol::manifest_id(&manifest);
    let manifest_state = deliver(
        &handshake_state,
        GUEST,
        message(
            protocol::MessageKind::ManifestProposal,
            fixture::HOST_PEER_ID,
            0,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("manifest", manifest),
            ]),
        ),
    )
    .0;
    states.push((Manifest, manifest_state.clone()));

    let assigned_state = deliver(
        &manifest_state,
        GUEST,
        message(
            protocol::MessageKind::SlotAssignment,
            fixture::HOST_PEER_ID,
            1,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                (
                    "assignment_id",
                    Value::str(protocol::assignment_id(&fixture::assignments(1, None), 1)),
                ),
                ("assignments", fixture::assignments(1, None)),
            ]),
        ),
    )
    .0;
    states.push((Assigned, assigned_state.clone()));

    let ready_state = coordinator::step(&assigned_state, Event::SetReady { ready: true }).0;
    states.push((Ready, ready_state.clone()));

    let countdown_state = deliver(
        &ready_state,
        GUEST,
        message(
            protocol::MessageKind::Countdown,
            fixture::HOST_PEER_ID,
            2,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("remaining_ticks", Value::int(0)),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
    )
    .0;
    states.push((Countdown, countdown_state.clone()));

    let running_state = deliver(
        &countdown_state,
        GUEST,
        message(
            protocol::MessageKind::Start,
            fixture::HOST_PEER_ID,
            3,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
    )
    .0;
    states.push((Running, running_state.clone()));

    let mut state = running_state;
    let mut sequence = 3;
    for (phase, tick, home) in [("kickoff", 0, 0), ("playing", 1, 0), ("full_time", 7200, 1)] {
        sequence += 1;
        state = deliver(
            &state,
            GUEST,
            message(
                protocol::MessageKind::MatchPhase,
                fixture::HOST_PEER_ID,
                sequence,
                Value::record(vec![
                    ("phase", Value::str(phase)),
                    ("tick", Value::int(tick)),
                    ("home_score", Value::int(home)),
                    ("away_score", Value::int(0)),
                ]),
            ),
        )
        .0;
    }
    states.push((Result, state));

    states
}

/// The coordinator validates a republished assignment against `assigned`
/// because ownership changes revoke readiness; the oracle mirrors that one
/// documented remap and nothing else. Mirrors the reference implementation's
/// `assert_phase_cell`.
fn assert_phase_cell(
    state: &CoordinatorState,
    sender: &str,
    kind: protocol::MessageKind,
    body: Value,
) {
    let control = message(kind, sender, 90, body);
    let phase = if kind == protocol::MessageKind::SlotAssignment
        && state.phase == protocol::LifecyclePhase::Ready
    {
        protocol::LifecyclePhase::Assigned
    } else {
        state.phase
    };
    let legal = protocol::validate_phase(&control, phase).is_ok();
    let (next_state, _) = deliver(state, GUEST, control);
    let label = format!("{} in {:?}", kind.wire_str(), state.phase);
    if legal {
        let ok = next_state
            .terminal
            .as_ref()
            .is_none_or(|t| t.code.as_deref() != Some("invalid_phase"));
        assert!(ok, "{label} is legal but was refused as out of phase");
    } else {
        assert_eq!(
            next_state.phase,
            protocol::LifecyclePhase::Terminal,
            "{label} must not be accepted"
        );
        assert_eq!(
            next_state.terminal.unwrap().code.as_deref(),
            Some("invalid_phase"),
            "{label}"
        );
    }
}

#[test]
fn agrees_with_the_protocol_phase_table_for_every_host_received_kind() {
    let states = host_states();
    let kinds = [
        protocol::MessageKind::Handshake,
        protocol::MessageKind::ManifestAccept,
        protocol::MessageKind::Ready,
        protocol::MessageKind::Start,
        protocol::MessageKind::HashReport,
        protocol::MessageKind::ResultAck,
        protocol::MessageKind::Abort,
        protocol::MessageKind::Disconnect,
    ];
    let mut checked = 0;
    for phase in coordinator::PHASES.iter().copied() {
        let Some((_, state)) = states.iter().find(|(p, _)| *p == phase) else {
            continue;
        };
        assert_eq!(state.phase, phase, "fixture for phase {phase:?}");
        let manifest_id = state
            .manifest_id
            .clone()
            .unwrap_or_else(|| "0123456789abcdef".to_string());
        for kind in kinds {
            assert_phase_cell(state, GUEST, kind, guest_body(kind, &manifest_id));
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        7 * 8,
        "every host phase must be crossed with every guest-sent kind"
    );
}

#[test]
fn agrees_with_the_protocol_phase_table_for_every_guest_received_kind() {
    let states = guest_states();
    let kinds = [
        protocol::MessageKind::ManifestProposal,
        protocol::MessageKind::PeerAssignment,
        protocol::MessageKind::SlotAssignment,
        protocol::MessageKind::Countdown,
        protocol::MessageKind::Start,
        protocol::MessageKind::MatchPhase,
        protocol::MessageKind::HashReport,
        protocol::MessageKind::ResultAck,
        protocol::MessageKind::Abort,
        protocol::MessageKind::Disconnect,
    ];
    let mut checked = 0;
    for phase in coordinator::PHASES.iter().copied() {
        let Some((_, state)) = states.iter().find(|(p, _)| *p == phase) else {
            continue;
        };
        assert_eq!(state.phase, phase, "fixture for phase {phase:?}");
        let manifest_id = state
            .manifest_id
            .clone()
            .unwrap_or_else(|| "0123456789abcdef".to_string());
        for kind in kinds {
            assert_phase_cell(
                state,
                fixture::HOST_PEER_ID,
                kind,
                host_body(kind, &manifest_id),
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        8 * 10,
        "every guest phase must be crossed with every host-sent kind"
    );
}

#[test]
fn is_a_total_function_over_every_phase_and_local_event_kind() {
    fn events() -> Vec<Event> {
        vec![
            Event::Connect,
            Event::Control {
                link_id: fixture::link_id(GUEST),
                message: None,
                wire: None,
            },
            Event::LinkLost {
                link_id: fixture::link_id(GUEST),
                code: Some("transport_lost".to_string()),
            },
            Event::ProposeManifest {
                manifest: fixture::manifest(None),
            },
            Event::AssignSlots {
                assignments: fixture::assignments(1, None),
                preserve_claims: false,
            },
            Event::SetReady { ready: true },
            Event::BeginCountdown {
                countdown_id: fixture::COUNTDOWN_ID.to_string(),
                remaining_ticks: 1,
                first_input_tick: 0,
            },
            Event::Tick,
            Event::MatchPhase {
                phase: "kickoff".to_string(),
                tick: 0,
                home_score: 0,
                away_score: 0,
            },
            Event::HashReport {
                tick: 60,
                boundary_hash: "0123456789abcdef".to_string(),
            },
            Event::Finish {
                final_tick: 7200,
                home_score: 1,
                away_score: 0,
                final_hash: "fedcba9876543210".to_string(),
            },
            Event::NetcodeFailure {
                failure: "late_input".to_string(),
                peer_id: None,
                detail: None,
            },
            Event::Leave,
            Event::Abort {
                code: Some("host_abort".to_string()),
                detail: None,
            },
        ]
    }

    let mut checked = 0;
    for states in [host_states(), guest_states()] {
        let mut terminal: Option<CoordinatorState> = None;
        for phase in coordinator::PHASES.iter().copied() {
            let Some((_, state)) = states.iter().find(|(p, _)| *p == phase) else {
                continue;
            };
            if terminal.is_none() {
                terminal = Some(
                    coordinator::step(
                        state,
                        Event::Abort {
                            code: Some("host_abort".to_string()),
                            detail: None,
                        },
                    )
                    .0,
                );
            }
            let terminal_state = terminal.clone().unwrap();
            for base in events() {
                for subject in [state.clone(), terminal_state.clone()] {
                    let (next_state, outcome) = coordinator::step(&subject, base.clone());
                    assert!(matches!(
                        outcome.disposition,
                        coordinator::Disposition::Applied
                            | coordinator::Disposition::Idempotent
                            | coordinator::Disposition::Rejected
                    ));
                    if outcome.disposition == coordinator::Disposition::Rejected {
                        assert_eq!(next_state, subject);
                        assert!(outcome.code.is_some());
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, (7 + 8) * events().len() * 2);
}

// ---------------------------------------------------------------------------
// Host-side departure reasons.
// ---------------------------------------------------------------------------

const HOST_BUILD: &str = "build.host_commit";
const GUEST_BUILD: &str = "build.guest_commit";

fn host_declaring(build_id: Option<&str>) -> CoordinatorState {
    coordinator::new(Options {
        role: Role::Host,
        session_id: SESSION.to_string(),
        peer_id: fixture::HOST_PEER_ID.to_string(),
        host_peer_id: None,
        host_link_id: None,
        runtime: fixture::runtime(),
        build_id: build_id.map(str::to_string),
        expectation: None,
    })
    .expect("host coordinator constructs")
}

fn peer_of<'a>(state: &'a CoordinatorState, peer_id: &str) -> Option<&'a coordinator::Peer> {
    state.peers.iter().find(|peer| peer.peer_id == peer_id)
}

fn declaring_handshake(
    peer_id: &str,
    sequence: i64,
    build_id: Option<&str>,
) -> protocol::ControlMessage {
    message(
        protocol::MessageKind::Handshake,
        peer_id,
        sequence,
        Value::record(vec![
            ("role", Value::str("guest")),
            ("runtime", fixture::runtime()),
            ("build_id", build_id.map(Value::str).unwrap_or(Value::Nil)),
        ]),
    )
}

/// A host that has admitted one guest and proposed its manifest, which is
/// the exact point a skewed guest refuses and goes. Mirrors the reference
/// implementation's local `proposed`.
fn proposed(host_build: Option<&str>, guest_build: Option<&str>) -> (CoordinatorState, String) {
    let peer_id = fixture::guest_peer_id(1);
    let mut state = host_declaring(host_build);
    state = deliver(
        &state,
        &peer_id,
        declaring_handshake(&peer_id, 0, guest_build),
    )
    .0;
    state = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    )
    .0;
    (state, peer_id)
}

fn guest_aborts(
    state: &CoordinatorState,
    peer_id: &str,
    code: &str,
) -> (CoordinatorState, coordinator::Outcome) {
    deliver(
        state,
        peer_id,
        message(
            protocol::MessageKind::Abort,
            peer_id,
            1,
            Value::record(vec![("code", Value::str(code))]),
        ),
    )
}

#[test]
fn records_the_build_a_guest_declared_in_its_handshake() {
    let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
    assert_eq!(state.build_id.as_deref(), Some(HOST_BUILD));
    assert_eq!(
        peer_of(&state, &peer_id).unwrap().build_id.as_deref(),
        Some(GUEST_BUILD)
    );
    assert_eq!(
        state.peers[0].build_id.as_deref(),
        Some(HOST_BUILD),
        "the host declares its own build too"
    );
    // Admission is never refused on it: the guest has to reach the manifest
    // check and mint its own reason, exactly as it did before.
    assert_eq!(state.terminal, None);
    assert_eq!(state.peers.len(), 2);
}

#[test]
fn names_the_build_when_it_drops_a_guest_that_is_running_a_different_one() {
    let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
    let (next_state, outcome) = guest_aborts(&state, &peer_id, "manifest_mismatch");
    assert_eq!(outcome.disposition, coordinator::Disposition::Applied);
    assert_eq!(
        next_state.terminal, None,
        "one skewed guest does not end the host's lobby"
    );
    assert_eq!(next_state.peers.len(), 1);

    let departure = next_state
        .departure
        .clone()
        .expect("the host recorded no reason at all");
    assert_eq!(departure.reason, TerminalReason::BuildMismatch);
    assert_eq!(departure.peer_id, peer_id);
    assert_eq!(
        departure.detail.as_deref(),
        Some("a guest aborted with manifest_mismatch")
    );

    // The wire is untouched: the announced disconnect still carries the
    // closed #161 code it always did, so the specific reason is local.
    assert_eq!(departure.code, "protocol_error");
    let mut announced = 0;
    for action in &outcome.actions {
        if let coordinator::Action::Send { message, .. } = action
            && message.kind == protocol::MessageKind::Disconnect
        {
            announced += 1;
            assert_eq!(
                message.body.get("code").and_then(Value::as_str),
                Some("protocol_error")
            );
            assert_eq!(
                message.body.get("target_peer_id").and_then(Value::as_str),
                Some(peer_id.as_str())
            );
        }
    }
    assert_eq!(announced, 1);
}

#[test]
fn blames_the_build_only_for_an_abort_over_session_identity() {
    // The width that is not bought. A guest can abort pre-freeze for reasons
    // that have nothing to do with builds, and on a mixed-build run it is
    // *always* also built differently -- so a rule keyed on the skew alone
    // would report every one of them as a build problem and send a tester to
    // reinstall instead of to the bug.
    let codes = [
        "invalid_assignment",
        "invalid_phase",
        "malformed_message",
        "unsupported_message",
        "capacity",
        "protocol_mismatch",
        "runtime_mismatch",
        "host_abort",
        "peer_disconnect",
        "desync",
    ];
    for code in codes {
        let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
        let (next_state, _) = guest_aborts(&state, &peer_id, code);
        let departure = next_state
            .departure
            .clone()
            .unwrap_or_else(|| panic!("{code} recorded no departure"));
        assert_eq!(
            departure.reason,
            TerminalReason::ProtocolViolation,
            "{code} must not blame the build"
        );
        assert_eq!(
            departure.detail.as_deref(),
            Some(format!("a guest aborted with {code}").as_str())
        );
    }
    // And the one that does, for contrast, against the same skew.
    let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
    let (next_state, _) = guest_aborts(&state, &peer_id, "manifest_mismatch");
    assert_eq!(
        next_state.departure.unwrap().reason,
        TerminalReason::BuildMismatch
    );
}

#[test]
fn keeps_a_generic_reason_when_the_two_peers_agree_on_the_build() {
    let (state, peer_id) = proposed(Some(HOST_BUILD), Some(HOST_BUILD));
    let (next_state, _) = guest_aborts(&state, &peer_id, "manifest_mismatch");
    let departure = next_state.departure.unwrap();
    assert_eq!(departure.reason, TerminalReason::ProtocolViolation);
    assert_eq!(departure.code, "protocol_error");
}

#[test]
fn claims_nothing_about_builds_when_neither_peer_declared_one() {
    // Every session built from `coordinator_fixture` is this case, which is
    // why no pinned coordinator transcript moved.
    let (state, peer_id) = proposed(None, None);
    let (next_state, _) = guest_aborts(&state, &peer_id, "manifest_mismatch");
    assert_eq!(
        next_state.departure.unwrap().reason,
        TerminalReason::ProtocolViolation
    );
}

#[test]
fn claims_nothing_about_builds_when_only_the_guest_declared_one() {
    let (state, peer_id) = proposed(None, Some(GUEST_BUILD));
    let (next_state, _) = guest_aborts(&state, &peer_id, "manifest_mismatch");
    assert_eq!(
        next_state.departure.unwrap().reason,
        TerminalReason::ProtocolViolation
    );
}

#[test]
fn names_the_build_when_a_guest_declares_none_against_a_host_that_does() {
    // A build from before the handshake carried an identity. It is a
    // different build, and saying so is the whole point.
    let (state, peer_id) = proposed(Some(HOST_BUILD), None);
    let (next_state, _) = guest_aborts(&state, &peer_id, "manifest_mismatch");
    assert_eq!(
        next_state.departure.unwrap().reason,
        TerminalReason::BuildMismatch
    );
}

#[test]
fn does_not_blame_the_build_for_a_link_that_simply_ended() {
    // `handle_link_lost`, the fourth drop site. Its code comes from the
    // local transport, and every value it accepts -- including
    // `protocol_error` -- keeps its own reason against a skewed peer.
    let cases = [
        ("peer_left", TerminalReason::GuestLeft),
        ("transport_lost", TerminalReason::TransportLost),
        ("host_left", TerminalReason::HostLeft),
        ("protocol_error", TerminalReason::ProtocolViolation),
    ];
    for (code, reason) in cases {
        let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
        let (next_state, _) = coordinator::step(
            &state,
            Event::LinkLost {
                link_id: fixture::link_id(&peer_id),
                code: Some(code.to_string()),
            },
        );
        let departure = next_state.departure.unwrap();
        assert_eq!(departure.reason, reason, "{code} must keep its own reason");
        assert_eq!(departure.code, code);
        assert_eq!(
            departure.detail.as_deref(),
            Some(format!("a guest's link ended as {code}").as_str())
        );
    }
}

#[test]
fn never_lets_a_guests_own_disconnect_code_name_a_build() {
    // `apply_disconnect`, the third drop site, and the only one whose code
    // arrives verbatim from the peer. A guest that announces its departure
    // as `protocol_error` while running a different build would otherwise
    // pick the sentence its own departure is reported under.
    let cases = [
        ("protocol_error", TerminalReason::ProtocolViolation),
        ("peer_left", TerminalReason::GuestLeft),
        ("transport_lost", TerminalReason::TransportLost),
        ("host_left", TerminalReason::HostLeft),
    ];
    for (code, reason) in cases {
        let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
        let (next_state, _) = deliver(
            &state,
            &peer_id,
            message(
                protocol::MessageKind::Disconnect,
                &peer_id,
                1,
                Value::record(vec![
                    ("target_peer_id", Value::str(peer_id.clone())),
                    ("code", Value::str(code)),
                ]),
            ),
        );
        let departure = next_state
            .departure
            .unwrap_or_else(|| panic!("{code} recorded no departure"));
        assert_eq!(
            departure.reason, reason,
            "{code} must not be able to name a build"
        );
        assert_eq!(departure.code, code);
        assert_eq!(
            departure.detail.as_deref(),
            Some(format!("a guest announced its own disconnect as {code}").as_str())
        );
    }
}

#[test]
fn names_the_build_on_a_drop_the_manifest_acceptance_caused() {
    // The second of the two host-judged drop sites: a guest accepting a
    // manifest this session never proposed. Its trigger is already a
    // specific identity disagreement, so it needs no further gate.
    let (state, peer_id) = proposed(Some(HOST_BUILD), Some(GUEST_BUILD));
    let (next_state, _) = deliver(
        &state,
        &peer_id,
        message(
            protocol::MessageKind::ManifestAccept,
            &peer_id,
            1,
            Value::record(vec![("manifest_id", Value::str("a".repeat(16)))]),
        ),
    );
    let departure = next_state.departure.unwrap();
    assert_eq!(departure.reason, TerminalReason::BuildMismatch);
    assert_eq!(
        departure.detail.as_deref(),
        Some("a guest accepted a manifest this session never proposed")
    );
}

#[test]
fn clears_the_reason_once_another_guest_takes_the_seat() {
    // A guest that gives up before the manifest is proposed leaves the host
    // still admitting, so the same lobby can be filled again -- which is
    // when the notice about the empty seat stops being true.
    let skewed = fixture::guest_peer_id(1);
    let mut state = host_declaring(Some(HOST_BUILD));
    state = deliver(
        &state,
        &skewed,
        declaring_handshake(&skewed, 0, Some(GUEST_BUILD)),
    )
    .0;
    state = guest_aborts(&state, &skewed, "manifest_mismatch").0;
    assert_eq!(
        state.departure.clone().unwrap().reason,
        TerminalReason::BuildMismatch
    );
    assert_eq!(
        state.phase,
        protocol::LifecyclePhase::Handshake,
        "a drop leaves the lobby open"
    );

    let replacement = fixture::guest_peer_id(2);
    state = deliver(
        &state,
        &replacement,
        declaring_handshake(&replacement, 0, Some(HOST_BUILD)),
    )
    .0;
    assert_eq!(state.departure, None, "a filled seat is no longer news");
    assert_eq!(state.peers.len(), 2);
}

/// The full `TERMINAL_CODES` table walk, which the previous pass could only
/// cover for 7 of 14 reasons because `terminal_code` was private. It is `pub`
/// now, so every reason is asserted directly against the reference mapping
/// (see `tools/lua_reference/README.md`; historically
/// `coordinator.lua:254-272`) rather than being reached through the handful
/// of events that happen to produce one.
#[test]
fn maps_every_terminal_reason_to_a_closed_protocol_code() {
    use coordinator::TerminalReason::*;
    let expected: &[(coordinator::TerminalReason, Option<&str>)] = &[
        (Completed, None),
        (LocalAbort, Some("host_abort")),
        (PeerAbort, Some("host_abort")),
        (GuestLeft, Some("peer_disconnect")),
        (HostLeft, Some("peer_disconnect")),
        (Removed, Some("peer_disconnect")),
        (TransportLost, Some("peer_disconnect")),
        (ProtocolViolation, Some("malformed_message")),
        (ManifestMismatch, Some("manifest_mismatch")),
        // A build disagreement is a manifest disagreement on the wire: the
        // closed rejection codes do not name builds, and inventing one would be
        // a protocol change to say locally what manifest_mismatch already says.
        (BuildMismatch, Some("manifest_mismatch")),
        (InvalidAssignment, Some("invalid_assignment")),
        (StartAckTimeout, Some("peer_disconnect")),
        (InputChannelFailure, Some("peer_disconnect")),
        (LateInput, Some("desync")),
        (HashMismatch, Some("desync")),
    ];
    for (reason, code) in expected {
        assert_eq!(
            coordinator::terminal_code(*reason),
            *code,
            "terminal code for {reason:?}"
        );
    }
}
