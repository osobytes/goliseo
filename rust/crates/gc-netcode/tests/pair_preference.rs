//! Pair-preference coordinator logic tests.
//!
//! Covers seven scenario groups: pair preference wire behavior, pair
//! preference rules, pair preference inertness, pair preference
//! generations, pair claims across a roster change, pair preference
//! sessions, and pair preference keeper protection.
//!
//! `tests/coordinator.rs`'s
//! `prefer_pair_is_inert_in_4v4_and_unchanged_for_the_slot_already_owned` is
//! adjacent to this file's inertness cases (same mode, same "nothing to
//! grant" property) but drives a `Driver` through a single 4v4/no-guest
//! event rather than sweeping the request space, and does not duplicate any
//! of the 34 cases below — none of them construct that exact zero-guest
//! scenario. All 34 of the original behavioral test cases are represented
//! here by name.

use gc_netcode::coordinator::{self, CoordinatorState, Event, PreferenceState, TerminalReason};
use gc_netcode::coordinator_driver::{self as driver, Driver};
use gc_netcode::coordinator_fixture as fixture;
use gc_netcode::protocol::{self, Value};
use gc_netcode::protocol_conformance;
use gc_netcode::protocol_fixture;
use gc_sim::input_frame::{self, SlotId};
use indexmap::IndexMap;

const HOST: &str = fixture::HOST_PEER_ID;

/// `fixture.manifest().session_id`: every match mode shares the one fixture
/// session id (see `protocol_fixture::manifest`), so this is safe to pin as
/// a single constant.
const SESSION: &str = "session_alpha";

fn message(
    kind: protocol::MessageKind,
    peer_id: &str,
    sequence: i64,
    body: Value,
) -> protocol::ControlMessage {
    protocol::new(kind, SESSION, peer_id, sequence, body).expect("fixture message must be valid")
}

fn deliver(
    state: &CoordinatorState,
    peer_id: &str,
    control: protocol::ControlMessage,
) -> CoordinatorState {
    coordinator::step(
        state,
        Event::Control {
            link_id: fixture::link_id(peer_id),
            message: Some(control),
            wire: None,
        },
    )
    .0
}

/// A host that has admitted `guest_count` guests, proposed the mode's
/// manifest, seen every acceptance, and published the planned block
/// ownership.
fn assigned_host(mode: protocol::MatchMode, guest_count: i64) -> CoordinatorState {
    let mut state = fixture::host(None);
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        state = deliver(
            &state,
            &peer_id,
            message(
                protocol::MessageKind::Handshake,
                &peer_id,
                0,
                Value::record(vec![
                    ("role", Value::str("guest")),
                    ("runtime", fixture::runtime()),
                ]),
            ),
        );
    }
    let (next, _) = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(Some(mode)),
        },
    );
    state = next;
    let manifest_id = state.manifest_id.clone().expect("manifest proposed");
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        state = deliver(
            &state,
            &peer_id,
            message(
                protocol::MessageKind::ManifestAccept,
                &peer_id,
                1,
                Value::record(vec![("manifest_id", Value::str(manifest_id.clone()))]),
            ),
        );
    }
    coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: fixture::assignments(guest_count, Some(mode)),
            preserve_claims: false,
        },
    )
    .0
}

fn owned_text(state: &CoordinatorState, producer_id: &str) -> String {
    coordinator::owned_slots(state, producer_id)
        .into_iter()
        .map(protocol::slot_wire_id)
        .collect::<Vec<_>>()
        .join(",")
}

fn joined_slots(slots: &[SlotId]) -> String {
    slots
        .iter()
        .map(|&slot| protocol::slot_wire_id(slot))
        .collect::<Vec<_>>()
        .join(",")
}

fn slots_value(slots: &[SlotId]) -> Value {
    Value::array(
        slots
            .iter()
            .map(|&slot| Value::str(protocol::slot_wire_id(slot)))
            .collect(),
    )
}

/// Every canonical slot has exactly one producer, and no human owns more or
/// fewer slots than the mode seats. Called after every interleaving so a
/// double ownership cannot hide behind a passing assertion elsewhere.
fn assert_partition(state: &CoordinatorState, mode: protocol::MatchMode) {
    let assignments = state
        .assignments
        .as_ref()
        .expect("ownership is unpublished");
    let shape = protocol::match_mode_shape(mode);
    let mut counts: IndexMap<String, i64> = IndexMap::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = assignments.get_index(index).expect("canonical producer");
        let slot_wire = producer
            .get("slot")
            .and_then(Value::as_str)
            .expect("producer names a slot");
        let expected =
            protocol::slot_wire_id(input_frame::slot(index).expect("canonical slot index").id);
        assert_eq!(slot_wire, expected, "canonical order broke");
        if producer.get("producer_kind").and_then(Value::as_str) == Some("peer") {
            let producer_id = producer
                .get("producer_id")
                .and_then(Value::as_str)
                .expect("peer producer names an id")
                .to_string();
            *counts.entry(producer_id).or_insert(0) += 1;
        }
    }
    for peer in &state.peers {
        let count = counts.get(&peer.peer_id).copied().unwrap_or(0);
        assert_eq!(
            count, shape.slots_per_human,
            "{} owns the wrong number of slots",
            peer.peer_id
        );
    }
    assert!(
        protocol::validate_assignment_manifest(
            state.manifest.as_ref().expect("manifest present"),
            assignments,
        )
        .is_ok(),
        "ownership stopped validating against the manifest"
    );
}

/// Every set of `size` canonical slots, so a mode's whole request space can
/// be swept instead of sampled.
fn slot_sets(size: i64) -> Vec<Vec<SlotId>> {
    fn walk(start: i64, size: i64, chosen: &mut Vec<SlotId>, sets: &mut Vec<Vec<SlotId>>) {
        if chosen.len() as i64 == size {
            sets.push(chosen.clone());
            return;
        }
        for index in start..=input_frame::SLOT_COUNT {
            chosen.push(input_frame::slot(index).expect("canonical slot index").id);
            walk(index + 1, size, chosen, sets);
            chosen.pop();
        }
    }
    let mut sets = Vec::new();
    walk(1, size, &mut Vec::new(), &mut sets);
    sets
}

// ---------------------------------------------------------------------------
// "pair preference wire"
// ---------------------------------------------------------------------------

/// The thirteen wire digests that shipped before pair preferences existed.
/// Two message kinds were appended to the conformance fixture, never
/// inserted, so appending must leave every one of these where it was.
const SHIPPED_DIGESTS: &[(protocol::MessageKind, &str)] = &[
    (protocol::MessageKind::Handshake, "2722abf054051350"),
    (protocol::MessageKind::ManifestProposal, "171c298f6eeb77e1"),
    (protocol::MessageKind::ManifestAccept, "363c57d949586608"),
    (protocol::MessageKind::PeerAssignment, "fa48b31571dfe543"),
    (protocol::MessageKind::SlotAssignment, "db929e7cd34eab60"),
    (protocol::MessageKind::Ready, "a89d1e1747464a51"),
    (protocol::MessageKind::Countdown, "c26f26e05519c2c8"),
    (protocol::MessageKind::Start, "3fdf9b6a442b6755"),
    (protocol::MessageKind::MatchPhase, "1671940891b78f1f"),
    (protocol::MessageKind::HashReport, "4405d9323b1e5b0f"),
    (protocol::MessageKind::ResultAck, "5f466e6740c6d4cf"),
    (protocol::MessageKind::Abort, "9db9c05e9728c4c1"),
    (protocol::MessageKind::Disconnect, "a7599b154bb86cec"),
];

#[test]
fn keeps_every_digest_that_shipped_before_it_and_the_manifest_id() {
    let report = protocol_conformance::verify();
    // Repinned by #268 (`max_goals` 5 -> 99). Pair preferences added no
    // manifest field, so this id only ever moves when the manifest does.
    assert_eq!(
        report.manifest_id, "eb59f113614c35b2",
        "the manifest id moved"
    );
    for (kind, digest) in SHIPPED_DIGESTS {
        let actual = protocol_conformance::GOLDEN
            .wire_digests
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, d)| *d)
            .unwrap_or_else(|| panic!("{} has no golden digest", kind.wire_str()));
        assert_eq!(actual, *digest, "{} wire digest moved", kind.wire_str());
    }
    let pair_preference = protocol_conformance::GOLDEN
        .wire_digests
        .iter()
        .find(|(k, _)| *k == protocol::MessageKind::PairPreference)
        .map(|(_, d)| *d)
        .expect("pair_preference has a golden digest");
    assert_eq!(pair_preference, "44cbe6dc14b4af77");
    let pair_preference_result = protocol_conformance::GOLDEN
        .wire_digests
        .iter()
        .find(|(k, _)| *k == protocol::MessageKind::PairPreferenceResult)
        .map(|(_, d)| *d)
        .expect("pair_preference_result has a golden digest");
    assert_eq!(pair_preference_result, "e9bc40f5818037f4");
}

#[test]
fn round_trips_a_request_and_a_verdict_through_the_canonical_codec() {
    let request = message(
        protocol::MessageKind::PairPreference,
        "guest.1",
        4,
        Value::record(vec![
            ("manifest_id", Value::str("a".repeat(16))),
            ("assignment_id", Value::str("b".repeat(16))),
            ("slots", slots_value(&[SlotId::Home1, SlotId::Home3])),
        ]),
    );
    let wire = protocol::encode(&request).expect("request encodes");
    let decoded = protocol::decode(&wire).expect("request decodes");
    assert_eq!(
        protocol::encode(&decoded).expect("decoded request re-encodes"),
        wire
    );

    let verdict = message(
        protocol::MessageKind::PairPreferenceResult,
        HOST,
        4,
        Value::record(vec![
            ("manifest_id", Value::str("a".repeat(16))),
            ("assignment_id", Value::str("b".repeat(16))),
            ("slots", slots_value(&[SlotId::Home1, SlotId::Home3])),
            ("status", Value::str("rejected")),
            ("reason", Value::str("already_taken")),
        ]),
    );
    let verdict_wire = protocol::encode(&verdict).expect("verdict encodes");
    let decoded_verdict = protocol::decode(&verdict_wire).expect("verdict decodes");
    assert_eq!(
        protocol::encode(&decoded_verdict).expect("decoded verdict re-encodes"),
        verdict_wire
    );
}

#[test]
fn refuses_a_slot_set_that_is_not_canonical_unique_and_bounded() {
    struct Case {
        name: &'static str,
        slots: Value,
    }
    let cases = vec![
        Case {
            name: "descending",
            slots: Value::array(vec![Value::str("home_2"), Value::str("home_1")]),
        },
        Case {
            name: "duplicate",
            slots: Value::array(vec![Value::str("home_1"), Value::str("home_1")]),
        },
        Case {
            name: "unknown slot",
            slots: Value::array(vec![Value::str("home_9")]),
        },
        // Keepers hold no canonical slot, so the vocabulary cannot name one.
        Case {
            name: "keeper player",
            slots: Value::array(vec![Value::str("ozzo")]),
        },
        Case {
            name: "empty",
            slots: Value::array(vec![]),
        },
        Case {
            name: "sparse",
            slots: Value::Table(vec![
                (Value::Int(1), Value::str("home_1")),
                (Value::Int(3), Value::str("home_3")),
            ]),
        },
        Case {
            name: "not strings",
            slots: Value::array(vec![Value::int(1), Value::int(2)]),
        },
        Case {
            name: "not an array",
            slots: Value::record(vec![("home_1", Value::bool(true))]),
        },
    ];
    for case in cases {
        let result = protocol::validate_slot_set(&case.slots);
        let err = result.expect_err(case.name);
        assert_eq!(err.code, protocol::ErrorCode::Malformed, "{}", case.name);
    }
    assert!(protocol::validate_slot_set(&slots_value(&[SlotId::Home1, SlotId::Away4])).is_ok());
}

#[test]
fn binds_the_typed_reason_to_the_rejected_status_and_to_nothing_else() {
    fn validate(status: &str, reason: Option<&str>) -> protocol::Result<()> {
        let body = Value::record(vec![
            ("manifest_id", Value::str("a".repeat(16))),
            ("assignment_id", Value::str("b".repeat(16))),
            ("slots", slots_value(&[SlotId::Home1])),
            ("status", Value::str(status)),
            ("reason", reason.map(Value::str).unwrap_or(Value::Nil)),
        ]);
        let control = protocol::ControlMessage {
            version: protocol::VERSION,
            kind: protocol::MessageKind::PairPreferenceResult,
            session_id: SESSION.to_string(),
            peer_id: HOST.to_string(),
            sequence: 0,
            message_id: protocol::message_id(SESSION, HOST, 0).expect("message id"),
            body,
        };
        protocol::validate(&control)
    }
    assert!(
        validate("rejected", None).is_err(),
        "a refusal must carry a reason"
    );
    assert!(
        validate("rejected", Some("no_thanks")).is_err(),
        "the reason vocabulary is closed"
    );
    // Silence is only observable at the end that waited, and a pair the
    // roster reseated away is only observable against the ownership that
    // took it, which every peer holds for itself. A host claiming either
    // would be reporting something it does not decide. Keeping the locally
    // minted reasons off the wire is what leaves the accepted message
    // vocabulary -- and both digests above -- exactly as they were.
    for reason in protocol::LOCAL_PREFERENCE_REJECTIONS {
        assert!(
            protocol::PREFERENCE_REJECTIONS.contains(reason),
            "{reason} is not a typed reason at all"
        );
        assert!(
            validate("rejected", Some(reason)).is_err(),
            "a host cannot report {reason}"
        );
    }
    assert!(protocol::LOCAL_PREFERENCE_REJECTIONS.contains(&"no_response"));
    assert!(protocol::LOCAL_PREFERENCE_REJECTIONS.contains(&"reseated"));
    assert!(
        validate("granted", Some("already_taken")).is_err(),
        "only a refusal carries a reason"
    );
    assert!(
        validate("maybe", None).is_err(),
        "the status vocabulary is closed"
    );
    assert!(validate("granted", None).is_ok());
    assert!(validate("unchanged", None).is_ok());
    assert!(validate("rejected", Some("wrong_team")).is_ok());
}

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

#[test]
fn is_legal_exactly_in_the_pre_start_phases_countdown_included() {
    let request = message(
        protocol::MessageKind::PairPreference,
        "guest.1",
        4,
        Value::record(vec![
            ("manifest_id", Value::str("a".repeat(16))),
            ("assignment_id", Value::str("b".repeat(16))),
            ("slots", slots_value(&[SlotId::Home1])),
        ]),
    );
    let result = message(
        protocol::MessageKind::PairPreferenceResult,
        HOST,
        4,
        Value::record(vec![
            ("manifest_id", Value::str("a".repeat(16))),
            ("assignment_id", Value::str("b".repeat(16))),
            ("slots", slots_value(&[SlotId::Home1])),
            ("status", Value::str("granted")),
        ]),
    );
    for &phase in coordinator::PHASES {
        let legal = matches!(
            phase,
            protocol::LifecyclePhase::Assigned
                | protocol::LifecyclePhase::Ready
                | protocol::LifecyclePhase::Countdown
        );
        for control in [&request, &result] {
            let outcome = protocol::validate_phase(control, phase);
            if legal {
                assert!(
                    outcome.is_ok(),
                    "{} must be legal in {}",
                    control.kind.wire_str(),
                    phase_wire(phase)
                );
            } else {
                let err = outcome.err().unwrap_or_else(|| {
                    panic!(
                        "{} must be illegal in {}",
                        control.kind.wire_str(),
                        phase_wire(phase)
                    )
                });
                assert_eq!(
                    err.code,
                    protocol::ErrorCode::InvalidPhase,
                    "{}",
                    phase_wire(phase)
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// "pair preference rules"
// ---------------------------------------------------------------------------

#[test]
fn grants_a_same_team_pair_and_pays_the_displaced_peer_in_vacated_slots() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    assert_eq!(owned_text(&state, HOST), "home_1,home_2");
    assert_eq!(owned_text(&state, "guest.1"), "home_3,home_4");

    let verdict = coordinator::evaluate_preference(&state, HOST, &[SlotId::Home1, SlotId::Home3]);
    assert_eq!(verdict.status, PreferenceState::Granted);
    let granted = verdict.assignments.expect("a grant publishes ownership");
    assert_eq!(
        protocol::owned_slots(&granted, HOST).join(","),
        "home_1,home_3"
    );
    assert_eq!(
        protocol::owned_slots(&granted, "guest.1").join(","),
        "home_2,home_4"
    );
    assert_eq!(
        protocol::owned_slots(&granted, "guest.2").join(","),
        "away_1,away_2"
    );
    assert_eq!(
        protocol::owned_slots(&granted, "guest.3").join(","),
        "away_3,away_4"
    );
}

#[test]
fn answers_the_set_a_peer_already_owns_with_unchanged() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let verdict = coordinator::evaluate_preference(&state, HOST, &[SlotId::Home1, SlotId::Home2]);
    assert_eq!(verdict.status, PreferenceState::Unchanged);
    assert_eq!(
        verdict.assignments, None,
        "an unchanged verdict publishes nothing"
    );
}

#[test]
fn refuses_each_ungrantable_request_with_its_own_typed_reason() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let refused = |slots: &[SlotId], reason: &str, peer_id: &str| {
        let verdict = coordinator::evaluate_preference(&state, peer_id, slots);
        assert_eq!(verdict.status, PreferenceState::Rejected, "{reason}");
        assert_eq!(verdict.reason.as_deref(), Some(reason));
        assert_eq!(verdict.assignments, None, "a refusal publishes nothing");
    };
    refused(&[SlotId::Home1, SlotId::Away1], "wrong_team", HOST);
    refused(&[SlotId::Home1], "invalid_slot", HOST);
    refused(
        &[SlotId::Home1, SlotId::Home2, SlotId::Home3],
        "invalid_slot",
        HOST,
    );
    refused(&[SlotId::Home2, SlotId::Home1], "invalid_slot", HOST);
    refused(&[SlotId::Home3, SlotId::Home4], "detached", HOST);
    refused(&[SlotId::Home1, SlotId::Home3], "not_seated", "nobody");
}

#[test]
fn protects_a_pair_its_owner_chose_and_only_that() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    // `guest.1` claims the pair the host's plan gave it.
    let state = deliver(
        &state,
        "guest.1",
        message(
            protocol::MessageKind::PairPreference,
            "guest.1",
            2,
            Value::record(vec![
                (
                    "manifest_id",
                    Value::str(state.manifest_id.clone().expect("manifest accepted")),
                ),
                (
                    "assignment_id",
                    Value::str(state.assignment_id.clone().expect("ownership published")),
                ),
                ("slots", slots_value(&[SlotId::Home3, SlotId::Home4])),
            ]),
        ),
    );
    let claimed = coordinator::evaluate_preference(&state, HOST, &[SlotId::Home1, SlotId::Home3]);
    assert_eq!(claimed.status, PreferenceState::Rejected);
    assert_eq!(claimed.reason.as_deref(), Some("already_taken"));
    // The away pair claimed nothing, so it is still exchangeable.
    let open = coordinator::evaluate_preference(&state, "guest.2", &[SlotId::Away1, SlotId::Away3]);
    assert_eq!(open.status, PreferenceState::Granted);
}

#[test]
fn refuses_a_preference_once_ownership_is_frozen() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(3),
        mode: Some(protocol::MatchMode::TwoVTwo),
        ..Default::default()
    });
    session.reach_start(None, None);
    let host = session.host().state.clone();
    assert!(coordinator::is_frozen(&host));
    let verdict =
        coordinator::evaluate_preference(&host, "guest.1", &[SlotId::Home3, SlotId::Home1]);
    assert_eq!(verdict.status, PreferenceState::Rejected);
    assert_eq!(verdict.reason.as_deref(), Some("after_freeze"));
}

// ---------------------------------------------------------------------------
// "pair preference inertness"
// ---------------------------------------------------------------------------

/// The whole request space of each mode, swept. In `1v1` and `4v4` nothing
/// is grantable; in `2v2` the same rule finds grants, which is what makes
/// the first two results a property of the rule and not of a mode branch.
fn sweep(
    mode: protocol::MatchMode,
    guest_count: i64,
    peer_id: &str,
) -> (i64, IndexMap<String, i64>) {
    let state = assigned_host(mode, guest_count);
    let shape = protocol::match_mode_shape(mode);
    let mut granted = 0i64;
    let mut answers: IndexMap<String, i64> = IndexMap::new();
    for slots in slot_sets(shape.slots_per_human) {
        let verdict = coordinator::evaluate_preference(&state, peer_id, &slots);
        if verdict.status == PreferenceState::Granted {
            granted += 1;
        } else {
            let key = match &verdict.reason {
                Some(reason) => reason.clone(),
                None => {
                    debug_assert_eq!(verdict.status, PreferenceState::Unchanged);
                    "unchanged".to_string()
                }
            };
            *answers.entry(key).or_insert(0) += 1;
        }
    }
    (granted, answers)
}

#[test]
fn cannot_move_a_1v1_human_whose_owned_set_is_the_whole_outfield_line() {
    let (granted, answers) = sweep(protocol::MatchMode::OneVOne, 1, HOST);
    assert_eq!(granted, 0, "1v1 must have nothing to choose");
    assert_eq!(
        answers.get("unchanged").copied().unwrap_or(0),
        1,
        "exactly one 1v1 request is the line already owned"
    );
}

#[test]
fn cannot_move_a_4v4_human_whose_owned_set_is_a_single_slot() {
    let (granted, answers) = sweep(protocol::MatchMode::FourVFour, 7, HOST);
    assert_eq!(granted, 0, "4v4 must have nothing to choose");
    assert_eq!(answers.get("unchanged").copied().unwrap_or(0), 1);
    assert_eq!(
        answers.get("detached").copied().unwrap_or(0),
        3,
        "the three same-team slots keep none of the owned set"
    );
    assert_eq!(answers.get("wrong_team").copied().unwrap_or(0), 4);
}

#[test]
fn is_equally_inert_in_a_4v4_that_ai_is_still_filling() {
    let (granted, _) = sweep(protocol::MatchMode::FourVFour, 2, HOST);
    assert_eq!(granted, 0, "a bot fill is not an opening in 4v4 either");
}

#[test]
fn finds_real_choices_in_2v2_under_the_same_rule() {
    let (granted, _) = sweep(protocol::MatchMode::TwoVTwo, 3, HOST);
    assert!(granted > 0, "2v2 must be able to choose a pair");
}

// ---------------------------------------------------------------------------
// "pair preference generations"
// ---------------------------------------------------------------------------

#[test]
fn mints_a_grant_through_the_existing_assignment_id_path_and_clears_readiness() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let (state, _) = coordinator::step(&state, Event::SetReady { ready: true });
    assert!(state.peers[0].ready);
    let epoch = state.assignment_epoch;
    let previous = state.assignment_id.clone().expect("ownership published");

    let (next_state, outcome) = coordinator::step(
        &state,
        Event::PreferPair {
            slots: vec![SlotId::Home1, SlotId::Home3],
        },
    );
    assert!(outcome.accepted);
    assert_eq!(next_state.assignment_epoch, epoch + 1);
    assert_eq!(
        next_state.assignment_id,
        Some(protocol::assignment_id(
            next_state.assignments.as_ref().expect("granted ownership"),
            epoch + 1,
        )),
        "a grant must mint its generation through the one existing path"
    );
    assert_ne!(next_state.assignment_id, Some(previous));
    assert_eq!(next_state.phase, protocol::LifecyclePhase::Assigned);
    assert!(
        !next_state.peers[0].ready,
        "a granted pair clears readiness"
    );
    assert_partition(&next_state, protocol::MatchMode::TwoVTwo);

    let mut published = false;
    for action in &outcome.actions {
        if let coordinator::Action::Send { message, .. } = action {
            published = published || message.kind == protocol::MessageKind::SlotAssignment;
        }
    }
    assert!(
        published,
        "a grant republishes ownership on the ordinary path"
    );
}

#[test]
fn mints_nothing_when_the_request_is_the_pair_already_owned() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let (state, _) = coordinator::step(&state, Event::SetReady { ready: true });
    let epoch = state.assignment_epoch;
    let previous = state.assignment_id.clone().expect("ownership published");

    let (next_state, outcome) = coordinator::step(
        &state,
        Event::PreferPair {
            slots: vec![SlotId::Home1, SlotId::Home2],
        },
    );
    assert!(outcome.accepted);
    assert_eq!(
        outcome.actions.len(),
        0,
        "an unchanged verdict publishes nothing"
    );
    assert_eq!(next_state.assignment_epoch, epoch);
    assert_eq!(next_state.assignment_id, Some(previous));
    assert!(
        next_state.peers[0].ready,
        "an unchanged verdict cannot clear readiness"
    );
    assert_eq!(
        next_state.preference.expect("preference recorded").status,
        PreferenceState::Unchanged
    );
}

#[test]
fn drops_every_claim_when_the_host_republishes_ownership_itself() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let (state, _) = coordinator::step(
        &state,
        Event::PreferPair {
            slots: vec![SlotId::Home1, SlotId::Home3],
        },
    );
    assert_eq!(
        joined_slots(state.peers[0].pair_choice.as_ref().expect("claim recorded")),
        "home_1,home_3"
    );
    let (state, _) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: fixture::assignments(3, Some(protocol::MatchMode::TwoVTwo)),
            preserve_claims: false,
        },
    );
    assert_eq!(
        state.peers[0].pair_choice, None,
        "the host overruled every choice"
    );
    assert_eq!(
        owned_text(&state, HOST),
        "home_1,home_2",
        "the host's own plan is back in force"
    );
}

// ---------------------------------------------------------------------------
// "pair claims across a roster change"
// ---------------------------------------------------------------------------

fn ask(state: &CoordinatorState, peer_id: &str, slots: &[SlotId]) -> CoordinatorState {
    if peer_id == HOST {
        coordinator::step(
            state,
            Event::PreferPair {
                slots: slots.to_vec(),
            },
        )
        .0
    } else {
        deliver(
            state,
            peer_id,
            message(
                protocol::MessageKind::PairPreference,
                peer_id,
                2,
                Value::record(vec![
                    (
                        "manifest_id",
                        Value::str(state.manifest_id.clone().expect("manifest accepted")),
                    ),
                    (
                        "assignment_id",
                        Value::str(state.assignment_id.clone().expect("ownership published")),
                    ),
                    ("slots", slots_value(slots)),
                ]),
            ),
        )
    }
}

fn depart(state: &CoordinatorState, peer_id: &str) -> CoordinatorState {
    deliver(
        state,
        peer_id,
        message(
            protocol::MessageKind::Disconnect,
            peer_id,
            3,
            Value::record(vec![
                ("target_peer_id", Value::str(peer_id)),
                ("code", Value::str("peer_left")),
            ]),
        ),
    )
}

/// Exactly what the lobby publishes after a roster change: the plan for the
/// roster that is left, offered to the coordinator to seat claims around.
fn reseat(state: &CoordinatorState, peer_ids: &[String]) -> CoordinatorState {
    let plan =
        coordinator::plan_assignments(state.manifest.as_ref().expect("manifest present"), peer_ids)
            .expect("reseat plan is valid");
    coordinator::step(
        state,
        Event::AssignSlots {
            assignments: plan,
            preserve_claims: true,
        },
    )
    .0
}

fn claim_text(state: &CoordinatorState, peer_id: &str) -> Option<String> {
    state
        .peers
        .iter()
        .find(|peer| peer.peer_id == peer_id)
        .and_then(|peer| peer.pair_choice.as_ref())
        .map(|slots| joined_slots(slots))
}

#[test]
fn keeps_a_claim_the_new_roster_still_fits_and_drops_the_one_it_cannot() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    // One claim inside the home line, which a smaller roster still seats
    // humans on, and one that reaches into the away line's far pair, which
    // it does not.
    let state = ask(&state, "guest.1", &[SlotId::Home2, SlotId::Home3]);
    let state = ask(&state, "guest.2", &[SlotId::Away1, SlotId::Away3]);
    assert_eq!(owned_text(&state, "guest.1"), "home_2,home_3");
    assert_eq!(owned_text(&state, "guest.2"), "away_1,away_3");
    assert_eq!(owned_text(&state, "guest.3"), "away_2,away_4");

    let state = depart(&state, "guest.3");
    assert_eq!(
        state.assignments, None,
        "ownership naming a departed peer is void"
    );
    assert_eq!(
        claim_text(&state, "guest.1"),
        Some("home_2,home_3".to_string()),
        "a claim outlives its generation"
    );
    let state = reseat(
        &state,
        &[
            HOST.to_string(),
            "guest.1".to_string(),
            "guest.2".to_string(),
        ],
    );

    assert_eq!(
        owned_text(&state, "guest.1"),
        "home_2,home_3",
        "a pair the roster fits is kept"
    );
    assert_eq!(
        claim_text(&state, "guest.1"),
        Some("home_2,home_3".to_string())
    );
    assert_eq!(
        owned_text(&state, "guest.2"),
        "away_1,away_2",
        "a pair it cannot fit is reseated"
    );
    assert_eq!(
        claim_text(&state, "guest.2"),
        None,
        "and the claim goes with it, deliberately"
    );
    assert_eq!(owned_text(&state, HOST), "home_1,home_4");
    assert_partition(&state, protocol::MatchMode::TwoVTwo);
}

#[test]
fn mints_the_reseat_through_the_existing_assignment_id_path() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let state = ask(&state, "guest.1", &[SlotId::Home2, SlotId::Home3]);
    let (state, _) = coordinator::step(&state, Event::SetReady { ready: true });
    assert!(state.peers[0].ready);
    let epoch = state.assignment_epoch;
    let previous = state.assignment_id.clone().expect("ownership published");

    let state = depart(&state, "guest.3");
    let state = reseat(
        &state,
        &[
            HOST.to_string(),
            "guest.1".to_string(),
            "guest.2".to_string(),
        ],
    );
    assert_eq!(
        state.assignment_epoch,
        epoch + 1,
        "a reseat is one generation, like any other"
    );
    assert_eq!(
        state.assignment_id,
        Some(protocol::assignment_id(
            state.assignments.as_ref().expect("reseated ownership"),
            state.assignment_epoch,
        )),
        "a reseat must mint its generation through the one existing path"
    );
    assert_ne!(state.assignment_id, Some(previous));
    assert_eq!(state.phase, protocol::LifecyclePhase::Assigned);
    for peer in &state.peers {
        assert!(
            !peer.ready,
            "{} kept readiness across a reseat",
            peer.peer_id
        );
    }
}

#[test]
fn still_lets_the_host_overrule_every_claim_by_reasserting_its_plan() {
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let state = ask(&state, "guest.1", &[SlotId::Home2, SlotId::Home3]);
    let state = depart(&state, "guest.3");
    // No `preserve_claims`: this is the host's own seating order, which is
    // the one thing that outranks a pair a guest was granted.
    let plan = coordinator::plan_assignments(
        state.manifest.as_ref().expect("manifest present"),
        &[
            HOST.to_string(),
            "guest.1".to_string(),
            "guest.2".to_string(),
        ],
    )
    .expect("plan is valid");
    let (state, _) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: plan,
            preserve_claims: false,
        },
    );
    assert_eq!(
        owned_text(&state, "guest.1"),
        "home_3,home_4",
        "the plan is back in force"
    );
    assert_eq!(claim_text(&state, "guest.1"), None);
    assert_partition(&state, protocol::MatchMode::TwoVTwo);
}

/// Swept rather than sampled: every grantable request by every remaining
/// peer is put through the same departure, so a double ownership or a claim
/// half-kept cannot hide behind the one case that was written by hand.
#[test]
fn ends_every_roster_change_on_a_valid_partition_claim_kept_whole_or_not_at_all() {
    let askers = [
        HOST.to_string(),
        "guest.1".to_string(),
        "guest.2".to_string(),
    ];
    let mut swept = 0i64;
    for peer_id in &askers {
        for slots in slot_sets(2) {
            let seeded = assigned_host(protocol::MatchMode::TwoVTwo, 3);
            if coordinator::evaluate_preference(&seeded, peer_id, &slots).status
                == PreferenceState::Granted
            {
                swept += 1;
                let wanted = joined_slots(&slots);
                let state = reseat(
                    &depart(&ask(&seeded, peer_id, &slots), "guest.3"),
                    &[
                        HOST.to_string(),
                        "guest.1".to_string(),
                        "guest.2".to_string(),
                    ],
                );
                assert_partition(&state, protocol::MatchMode::TwoVTwo);
                match claim_text(&state, peer_id) {
                    Some(claim) => {
                        assert_eq!(claim, wanted, "a kept claim is kept exactly");
                        assert_eq!(
                            owned_text(&state, peer_id),
                            wanted,
                            "a kept claim is the ownership"
                        );
                    }
                    None => {
                        assert_ne!(
                            owned_text(&state, peer_id),
                            wanted,
                            "a claim dropped while its pair survived would be a silent loss"
                        );
                    }
                }
            }
        }
    }
    assert!(swept > 0, "the sweep found no grantable request to reseat");
}

// ---------------------------------------------------------------------------
// "pair preference sessions"
// ---------------------------------------------------------------------------

fn settle(session: &mut Driver) {
    if session.latency_ticks > 0 {
        session.tick(Some(session.latency_ticks + 1));
    } else {
        session.pump();
    }
}

fn assigned_session(mode: protocol::MatchMode, guest_count: i64, latency: Option<i64>) -> Driver {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(guest_count),
        mode: Some(mode),
        latency_ticks: Some(latency.unwrap_or(0)),
        ..Default::default()
    });
    let manifest = fixture::manifest(Some(mode));
    session.connect_all();
    settle(&mut session);
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: manifest.clone(),
        },
    );
    settle(&mut session);
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: coordinator::plan_assignments(&manifest, &fixture::peer_ids(guest_count))
                .expect("assignment plan is valid"),
            preserve_claims: false,
        },
    );
    settle(&mut session);
    session
}

fn preference(session: &Driver, peer_id: &str) -> coordinator::Preference {
    session
        .node(peer_id)
        .expect("peer is admitted")
        .state
        .preference
        .clone()
        .expect("a preference was recorded")
}

#[test]
fn carries_a_guests_chosen_pair_to_the_host_and_the_grant_back() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Pending
    );
    session.pump();

    let host = session.host().state.clone();
    assert_eq!(owned_text(&host, "guest.2"), "away_1,away_3");
    assert_eq!(owned_text(&host, "guest.3"), "away_2,away_4");
    assert_partition(&host, protocol::MatchMode::TwoVTwo);
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Granted
    );
    assert_eq!(
        owned_text(
            &session.node("guest.2").expect("guest admitted").state,
            "guest.2"
        ),
        "away_1,away_3",
        "the requester holds the ownership before it reads the verdict"
    );
    assert_eq!(
        owned_text(
            &session.node("guest.3").expect("guest admitted").state,
            "guest.3"
        ),
        "away_2,away_4"
    );
    for node in &session.nodes {
        assert!(
            !coordinator::is_terminal(&node.state),
            "{} ended",
            node.peer_id
        );
    }
}

#[test]
fn answers_an_ungrantable_request_with_a_typed_reason_and_moves_nothing() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    let before = owned_text(&session.host().state, "guest.2");
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Home1, SlotId::Away1],
        },
    );
    session.pump();

    let answer = preference(&session, "guest.2");
    assert_eq!(answer.status, PreferenceState::Rejected);
    assert_eq!(answer.reason.as_deref(), Some("wrong_team"));
    assert_eq!(owned_text(&session.host().state, "guest.2"), before);
    assert_eq!(
        owned_text(
            &session.node("guest.2").expect("guest admitted").state,
            "guest.2"
        ),
        before
    );
    assert_eq!(
        session.host().state.assignment_epoch,
        1,
        "a refusal mints no generation"
    );
    assert!(!coordinator::is_terminal(&session.host().state));
}

#[test]
fn is_idempotent_when_a_reliable_transport_retransmits_the_request() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    session.pump();
    let epoch = session.host().state.assignment_epoch;
    let owned = owned_text(&session.host().state, "guest.2");

    let wire = session.first_wire(protocol::MessageKind::PairPreference);
    session.replay("guest.2", HOST, &wire);
    assert_eq!(
        session.host().state.assignment_epoch,
        epoch,
        "a retransmission cannot re-apply"
    );
    assert_eq!(owned_text(&session.host().state, "guest.2"), owned);
    assert_partition(&session.host().state, protocol::MatchMode::TwoVTwo);
}

#[test]
fn repeats_without_doubling_when_the_guest_asks_again_for_what_it_holds() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    for _ in 0..3 {
        session.send(
            "guest.2",
            Event::PreferPair {
                slots: vec![SlotId::Away1, SlotId::Away3],
            },
        );
        session.pump();
    }
    assert_eq!(
        session.host().state.assignment_epoch,
        2,
        "only the first request moved ownership"
    );
    assert_eq!(
        owned_text(&session.host().state, "guest.2"),
        "away_1,away_3"
    );
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Unchanged
    );
    assert_partition(&session.host().state, protocol::MatchMode::TwoVTwo);
}

#[test]
fn never_lets_two_guests_racing_for_one_slot_both_own_it() {
    // Both away humans want `away_3`, and both speak before either has seen
    // the other's ownership: the requests cross on the wire.
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, Some(1));
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    session.send(
        "guest.3",
        Event::PreferPair {
            slots: vec![SlotId::Away3, SlotId::Away4],
        },
    );
    assert_eq!(
        preference(&session, "guest.2").assignment_id,
        preference(&session, "guest.3").assignment_id
    );
    session.tick(Some(2));

    let host = session.host().state.clone();
    assert_partition(&host, protocol::MatchMode::TwoVTwo);
    assert_eq!(
        owned_text(&host, "guest.2"),
        "away_1,away_3",
        "the first request won"
    );
    assert_eq!(owned_text(&host, "guest.3"), "away_2,away_4");
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Granted
    );
    let loser = preference(&session, "guest.3");
    assert_eq!(loser.status, PreferenceState::Rejected);
    assert_eq!(loser.reason.as_deref(), Some("superseded"));

    // Asking again against the ownership now in force is refused too: the
    // winner claimed the slot, so it is no longer up for exchange.
    session.send(
        "guest.3",
        Event::PreferPair {
            slots: vec![SlotId::Away3, SlotId::Away4],
        },
    );
    session.tick(Some(2));
    assert_eq!(
        preference(&session, "guest.3").reason.as_deref(),
        Some("already_taken")
    );
    assert_partition(&session.host().state, protocol::MatchMode::TwoVTwo);
    assert_eq!(
        owned_text(&session.host().state, "guest.2"),
        "away_1,away_3"
    );
}

/// The freeze lands at the countdown, and a preference already in flight
/// when it does is an ordinary race. It is answered, not fatal.
fn counting_down_session() -> Driver {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    let peer_ids: Vec<String> = session.nodes.iter().map(|n| n.peer_id.clone()).collect();
    for peer_id in &peer_ids {
        session.send(peer_id, Event::SetReady { ready: true });
    }
    session.pump();
    session.send(
        HOST,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 30,
            first_input_tick: 0,
        },
    );
    session.pump();
    session
}

#[test]
fn refuses_a_preference_that_arrives_after_the_freeze_without_ending_the_session() {
    let mut session = counting_down_session();
    let host = session.host().state.clone();
    assert_eq!(host.phase, protocol::LifecyclePhase::Countdown);
    let freeze = host.freeze.clone().expect("countdown froze the session");
    session.inject(
        "guest.2",
        HOST,
        protocol::MessageKind::PairPreference,
        Value::record(vec![
            ("manifest_id", Value::str(freeze.manifest_id.clone())),
            ("assignment_id", Value::str(freeze.assignment_id.clone())),
            ("slots", slots_value(&[SlotId::Away1, SlotId::Away3])),
        ]),
        None,
    );

    let after = session.host().state.clone();
    assert_eq!(
        after.assignment_id,
        Some(freeze.assignment_id.clone()),
        "the frozen generation stands"
    );
    assert_eq!(
        after.freeze.as_ref().expect("still frozen").assignment_id,
        freeze.assignment_id
    );
    assert_eq!(
        joined_slots(
            after
                .freeze
                .as_ref()
                .unwrap()
                .owned
                .get("guest.2")
                .expect("guest.2 owns a pair")
        ),
        joined_slots(freeze.owned.get("guest.2").expect("guest.2 owns a pair")),
        "a frozen pair cannot move"
    );
    assert!(
        !coordinator::is_terminal(&after),
        "a late preference is not fatal"
    );
    let mut answered = false;
    for control in &session.transcript {
        if control.kind == protocol::MessageKind::PairPreferenceResult {
            answered = true;
            assert_eq!(
                control.body.get("status").and_then(Value::as_str),
                Some("rejected")
            );
            assert_eq!(
                control.body.get("reason").and_then(Value::as_str),
                Some("after_freeze")
            );
        }
    }
    assert!(answered, "the host must answer the late request");
}

/// Past the countdown every peer has seen `start`, and the phase table draws
/// the line there: a preference is then as much a violation as a late
/// `ready`, and ends the session the same way. Pinned because it is the
/// boundary the `countdown` allowance was chosen against.
#[test]
fn treats_a_preference_during_the_match_as_the_protocol_violation_it_is() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(3),
        mode: Some(protocol::MatchMode::TwoVTwo),
        ..Default::default()
    });
    session.reach_start(None, None);
    let freeze = session
        .host()
        .state
        .freeze
        .clone()
        .expect("session is frozen");
    session.inject(
        "guest.2",
        HOST,
        protocol::MessageKind::PairPreference,
        Value::record(vec![
            ("manifest_id", Value::str(freeze.manifest_id.clone())),
            ("assignment_id", Value::str(freeze.assignment_id.clone())),
            ("slots", slots_value(&[SlotId::Away1, SlotId::Away3])),
        ]),
        None,
    );
    let terminal = session
        .host()
        .state
        .terminal
        .clone()
        .expect("session terminated");
    assert_eq!(terminal.reason, TerminalReason::ProtocolViolation);
    assert_eq!(terminal.code.as_deref(), Some("invalid_phase"));
}

#[test]
fn keeps_a_full_2v2_session_reaching_its_start_boundary_after_a_granted_pair() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    session.pump();
    let peer_ids: Vec<String> = session.nodes.iter().map(|n| n.peer_id.clone()).collect();
    for peer_id in &peer_ids {
        session.send(peer_id, Event::SetReady { ready: true });
    }
    session.pump();
    session.send(
        HOST,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 3,
            first_input_tick: 0,
        },
    );
    session.pump();
    session.tick(Some(4));
    assert!(
        session.all_started(),
        "every peer must reach the start boundary"
    );
    let freeze = session.host().state.freeze.clone().expect("session froze");
    assert_eq!(
        joined_slots(freeze.owned.get("guest.2").expect("guest.2 owns a pair")),
        "away_1,away_3"
    );
    assert_eq!(freeze.live.get("guest.2").copied(), Some(SlotId::Away1));
}

/// A host that is up but silent. The guest's link is blackholed in the
/// guest->host direction only: neither end sees a `link_lost`, so neither
/// takes the `transport_lost` path that already terminates correctly, and
/// the request simply never lands. This is the case an unbounded `pending`
/// never escaped.
fn silence(session: &mut Driver, peer_id: &str) {
    let link_id = fixture::link_id(peer_id);
    let index = session
        .links
        .iter()
        .position(|link| link.id == link_id)
        .expect("known driver link");
    session.links[index].guest_open = false;
}

fn open_link(session: &mut Driver, peer_id: &str) {
    let link_id = fixture::link_id(peer_id);
    let index = session
        .links
        .iter()
        .position(|link| link.id == link_id)
        .expect("known driver link");
    session.links[index].guest_open = true;
}

#[test]
fn gives_up_on_a_request_a_live_host_never_answers() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    let before = owned_text(
        &session.node("guest.2").expect("guest admitted").state,
        "guest.2",
    );
    silence(&mut session, "guest.2");
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Pending
    );

    session.tick(Some(coordinator::PREFERENCE_TIMEOUT_TICKS));
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Pending,
        "the wait is not cut short"
    );
    session.tick(Some(1));

    let answer = preference(&session, "guest.2");
    assert_eq!(answer.status, PreferenceState::Rejected);
    assert_eq!(answer.reason.as_deref(), Some("no_response"));
    assert_eq!(
        answer.deadline, None,
        "an answered request waits on nothing"
    );
    assert_eq!(
        joined_slots(&answer.slots),
        "away_1,away_3",
        "the request stays legible"
    );
    assert_eq!(
        owned_text(
            &session.node("guest.2").expect("guest admitted").state,
            "guest.2"
        ),
        before
    );
    assert_eq!(owned_text(&session.host().state, "guest.2"), before);
    assert_eq!(
        session.host().state.assignment_epoch,
        1,
        "an expiry mints no generation"
    );
    assert_partition(&session.host().state, protocol::MatchMode::TwoVTwo);
    for node in &session.nodes {
        assert!(
            !coordinator::is_terminal(&node.state),
            "{} ended",
            node.peer_id
        );
    }
}

#[test]
fn cannot_be_moved_by_a_verdict_that_arrives_after_the_wait_expired() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    silence(&mut session, "guest.2");
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    session.tick(Some(coordinator::PREFERENCE_TIMEOUT_TICKS + 1));
    assert_eq!(
        preference(&session, "guest.2").reason.as_deref(),
        Some("no_response")
    );

    let guest = session
        .node("guest.2")
        .expect("guest admitted")
        .state
        .clone();
    let before = owned_text(&guest, "guest.2");
    let generation = guest.assignment_id.clone().expect("ownership published");
    let epoch = guest.assignment_epoch;
    // The host finally speaks, and grants exactly what was asked for. The
    // requester stopped waiting, so this answers no pending request and is
    // dropped: ownership only ever moves on `slot_assignment`.
    session.inject(
        HOST,
        "guest.2",
        protocol::MessageKind::PairPreferenceResult,
        Value::record(vec![
            (
                "manifest_id",
                Value::str(guest.manifest_id.clone().expect("manifest accepted")),
            ),
            ("assignment_id", Value::str(generation.clone())),
            ("slots", slots_value(&[SlotId::Away1, SlotId::Away3])),
            ("status", Value::str("granted")),
        ]),
        None,
    );

    let after = session
        .node("guest.2")
        .expect("guest admitted")
        .state
        .clone();
    assert_eq!(
        owned_text(&after, "guest.2"),
        before,
        "a late grant cannot move ownership"
    );
    assert_eq!(
        after.assignment_id,
        Some(generation),
        "a late grant mints no generation"
    );
    assert_eq!(after.assignment_epoch, epoch);
    assert_eq!(
        after
            .preference
            .as_ref()
            .expect("preference present")
            .status,
        PreferenceState::Rejected
    );
    assert_eq!(
        after.preference.as_ref().unwrap().reason.as_deref(),
        Some("no_response"),
        "a terminal outcome is not reopened"
    );
    assert!(
        !coordinator::is_terminal(&after),
        "a late grant is not fatal"
    );
}

#[test]
fn frees_the_guest_to_ask_again_once_the_wait_has_expired() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, None);
    silence(&mut session, "guest.2");
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    session.tick(Some(coordinator::PREFERENCE_TIMEOUT_TICKS + 1));
    assert_eq!(
        preference(&session, "guest.2").reason.as_deref(),
        Some("no_response")
    );

    open_link(&mut session, "guest.2");
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Pending
    );
    session.pump();
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Granted
    );
    assert_eq!(
        owned_text(&session.host().state, "guest.2"),
        "away_1,away_3"
    );
    assert_eq!(
        owned_text(&session.host().state, "guest.3"),
        "away_2,away_4"
    );
    assert_partition(&session.host().state, protocol::MatchMode::TwoVTwo);
}

/// `superseded` is what protects ownership when the answer is late rather
/// than absent, so it is re-proven now that a wait can also end by itself:
/// the host answers the loser well inside the deadline, and running the
/// clock past that deadline afterwards must not reach back and overwrite
/// the typed reason the host actually gave.
#[test]
fn still_refuses_a_request_that_outlived_its_ownership_generation() {
    let mut session = assigned_session(protocol::MatchMode::TwoVTwo, 3, Some(1));
    session.send(
        "guest.2",
        Event::PreferPair {
            slots: vec![SlotId::Away1, SlotId::Away3],
        },
    );
    session.send(
        "guest.3",
        Event::PreferPair {
            slots: vec![SlotId::Away3, SlotId::Away4],
        },
    );
    session.tick(Some(2));
    assert_eq!(
        preference(&session, "guest.2").status,
        PreferenceState::Granted
    );
    let loser = preference(&session, "guest.3");
    assert_eq!(loser.status, PreferenceState::Rejected);
    assert_eq!(loser.reason.as_deref(), Some("superseded"));
    assert_eq!(loser.deadline, None, "an answered request stops waiting");
    assert_eq!(
        owned_text(&session.host().state, "guest.2"),
        "away_1,away_3"
    );
    assert_partition(&session.host().state, protocol::MatchMode::TwoVTwo);

    session.tick(Some(coordinator::PREFERENCE_TIMEOUT_TICKS + 1));
    assert_eq!(
        preference(&session, "guest.3").reason.as_deref(),
        Some("superseded")
    );
    assert_eq!(
        owned_text(&session.host().state, "guest.2"),
        "away_1,away_3"
    );
}

// ---------------------------------------------------------------------------
// "pair preference keeper protection"
// ---------------------------------------------------------------------------

#[test]
fn cannot_name_a_keeper_because_a_keeper_holds_no_canonical_slot() {
    let manifest = protocol_fixture::manifest(Some(protocol::MatchMode::TwoVTwo));
    let mut keepers: Vec<String> = Vec::new();
    let teams = manifest.get("teams").expect("manifest has teams");
    for team_index in 1..=teams.len() as i64 {
        let team = teams.get_index(team_index).expect("canonical team");
        let roster = team.get("roster").expect("team has a roster");
        for player_index in 1..=roster.len() as i64 {
            let player = roster
                .get_index(player_index)
                .expect("canonical roster entry");
            if player.get("position").and_then(Value::as_str) == Some("keeper") {
                keepers.push(
                    player
                        .get("player_id")
                        .and_then(Value::as_str)
                        .expect("keeper has a player id")
                        .to_string(),
                );
            }
        }
    }
    for keeper in ["ozzo", "gax_oru"] {
        assert!(
            keepers.iter().any(|k| k == keeper),
            "{keeper} must be a fixture keeper"
        );
        assert_eq!(
            protocol::slot_index(keeper),
            None,
            "a keeper has no canonical slot"
        );
    }
    let state = assigned_host(protocol::MatchMode::TwoVTwo, 3);
    let assignments = state.assignments.as_ref().expect("ownership published");
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = assignments.get_index(index).expect("canonical producer");
        let player_id = producer
            .get("player_id")
            .and_then(Value::as_str)
            .expect("producer names a player id");
        assert!(
            !keepers.iter().any(|k| k == player_id),
            "a keeper reached a canonical slot"
        );
    }
}
