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
