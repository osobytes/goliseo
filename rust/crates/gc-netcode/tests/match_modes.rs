//! Port of `spec/game/online_match_modes_spec.lua`.
//!
//! Covers all four `describe` blocks: `"OMP-3 match modes"`, `"OMP-3
//! multi-slot ownership"`, `"OMP-3 mode-aware coordinator ownership"`, and
//! `"OMP-3 live slot selection"`.
//!
//! `protocol::MATCH_MODES` (a Lua runtime table the spec iterates with
//! `pairs`) has no Rust counterpart: `protocol::MatchMode` is a closed
//! three-variant enum instead, so "exactly three sizes" is a compile-time
//! invariant here rather than something a loop additionally has to prove.
//! [`MODES`] below is the fixed three-element stand-in the Lua spec's local
//! `MODES` table becomes.
//!
//! Two cases are ported faithfully but `#[ignore]`d, because they exercise a
//! behavior `coordinator::slot_sources` (`src/coordinator.rs`) does not
//! currently reproduce from the Lua reference
//! (`game/online/coordinator.lua`'s `coordinator.slot_sources`,
//! lines 595-631): see
//! [`lets_one_human_cover_several_slots_while_every_slot_keeps_one_source`]
//! and [`keeps_both_keepers_unassignable_in_every_mode`] for the specific
//! expected/actual values.

use gc_netcode::coordinator::{self, CoordinatorState, Event, Origin, SlotDriver, TerminalReason};
use gc_netcode::coordinator_driver::{self as driver, Driver};
use gc_netcode::coordinator_fixture as fixture;
use gc_netcode::live_slot;
use gc_netcode::protocol::{self, Value};
use gc_sim::input_frame::{self, SlotId};

const HOST: &str = fixture::HOST_PEER_ID;

/// Stand-in for the Lua spec's local `MODES = { "1v1", "2v2", "4v4" }` — see
/// the module doc comment for why this is a fixed array rather than an
/// iterated `protocol.MATCH_MODES` table.
const MODES: [protocol::MatchMode; 3] = [
    protocol::MatchMode::OneVOne,
    protocol::MatchMode::TwoVTwo,
    protocol::MatchMode::FourVFour,
];

/// The fixture session id, mirroring the Lua spec's
/// `local SESSION = fixture.manifest().session_id`.
fn session_id() -> String {
    fixture::manifest(None)
        .get("session_id")
        .and_then(Value::as_str)
        .expect("fixture manifest has a session id")
        .to_string()
}

/// Ownership shaped by hand rather than by `coordinator::plan_assignments`,
/// so the validators are exercised against inputs the planner would never
/// produce. Mirrors the Lua spec's local `seated` helper.
fn seated(mode: protocol::MatchMode, humans: i64) -> (Value, Value) {
    let manifest = fixture::manifest(Some(mode));
    let shape = protocol::match_mode_shape(mode);
    let slots = manifest.get("slots").unwrap().clone();
    let mut assignments = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).unwrap();
        let order = (index - 1) / shape.slots_per_human + 1;
        let player_id = slots
            .get_index(index)
            .unwrap()
            .get("player_id")
            .unwrap()
            .clone();
        let record = if order <= humans {
            Value::record(vec![
                ("slot", Value::str(protocol::slot_wire_id(slot.id))),
                ("team", Value::str(protocol::team_wire_str(slot.team))),
                ("player_id", player_id),
                ("producer_kind", Value::str("peer")),
                ("producer_id", Value::str(format!("peer.{order}"))),
            ])
        } else {
            Value::record(vec![
                ("slot", Value::str(protocol::slot_wire_id(slot.id))),
                ("team", Value::str(protocol::team_wire_str(slot.team))),
                ("player_id", player_id),
                ("producer_kind", Value::str("bot")),
                (
                    "producer_id",
                    Value::str(format!("bot.{}", protocol::slot_wire_id(slot.id))),
                ),
                ("bot_seed", Value::int(1000 + index)),
            ])
        };
        assignments.push(record);
    }
    (manifest, Value::array(assignments))
}

/// The owned set of `producer_id`, joined for comparison. Mirrors the Lua
/// spec's local `owned_text` helper.
fn owned_text(assignments: &Value, producer_id: &str) -> String {
    protocol::owned_slots(assignments, producer_id).join(",")
}

/// `slots`, joined by their wire ids for comparison.
fn joined_slots(slots: &[SlotId]) -> String {
    slots
        .iter()
        .map(|slot| protocol::slot_wire_id(*slot))
        .collect::<Vec<_>>()
        .join(",")
}

/// Overwrite one field of the producer at 1-based canonical `index` in a
/// slot-assignment array, in place. Mirrors the Lua spec's direct table
/// field mutation (`assignments[index].field = value`).
fn set_field(assignments: &mut Value, index: i64, field: &str, value: Value) {
    if let Value::Table(entries) = assignments {
        for (key, entry) in entries.iter_mut() {
            if key.as_int() == Some(index) {
                entry.set(field, value);
                return;
            }
        }
    }
    panic!("no producer at canonical index {index}");
}

/// Deliver an already-built control message over `peer_id`'s fixture link.
/// Mirrors the Lua spec's local `deliver` helper.
fn deliver(
    state: &CoordinatorState,
    peer_id: &str,
    message: protocol::ControlMessage,
) -> (CoordinatorState, coordinator::Outcome) {
    coordinator::step(
        state,
        Event::Control {
            link_id: fixture::link_id(peer_id),
            message: Some(message),
            wire: None,
        },
    )
}

/// A host that has admitted `guest_count` guests and seen every acceptance
/// of a manifest in `mode`, ready for ownership to be published. Mirrors
/// the Lua spec's local `accepted_host` helper.
fn accepted_host(guest_count: i64, mode: protocol::MatchMode) -> CoordinatorState {
    let mut state = fixture::host(None);
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        let message = protocol::new(
            protocol::MessageKind::Handshake,
            &session_id(),
            &peer_id,
            0,
            Value::record(vec![
                ("role", Value::str("guest")),
                ("runtime", fixture::runtime()),
            ]),
        )
        .unwrap();
        state = deliver(&state, &peer_id, message).0;
    }
    state = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(Some(mode)),
        },
    )
    .0;
    let manifest_id = state.manifest_id.clone().expect("manifest proposed");
    for index in 1..=guest_count {
        let peer_id = fixture::guest_peer_id(index);
        let message = protocol::new(
            protocol::MessageKind::ManifestAccept,
            &session_id(),
            &peer_id,
            1,
            Value::record(vec![("manifest_id", Value::str(manifest_id.clone()))]),
        )
        .unwrap();
        state = deliver(&state, &peer_id, message).0;
    }
    state
}

// ---------------------------------------------------------------------------
// "OMP-3 match modes"
// ---------------------------------------------------------------------------

#[test]
fn supports_exactly_three_sizes_that_share_the_same_eight_canonical_slots() {
    let mut count = 0;
    for mode in MODES {
        count += 1;
        let shape = protocol::match_mode_shape(mode);
        assert_eq!(shape.mode, mode);
        assert_eq!(
            shape.humans * shape.slots_per_human,
            input_frame::SLOT_COUNT
        );
        assert_eq!(
            shape.team_humans * shape.slots_per_human,
            input_frame::HOME_SLOT_COUNT
        );
        assert_eq!(shape.team_humans * 2, shape.humans);
    }
    assert_eq!(count, 3);
    assert_eq!(
        protocol::match_mode(&Value::str("1v1"))
            .unwrap()
            .slots_per_human,
        4
    );
    assert_eq!(
        protocol::match_mode(&Value::str("2v2"))
            .unwrap()
            .slots_per_human,
        2
    );
    assert_eq!(
        protocol::match_mode(&Value::str("4v4"))
            .unwrap()
            .slots_per_human,
        1
    );
}

#[test]
fn rejects_3v3_and_every_other_unsupported_size_with_a_typed_reason() {
    for unsupported in ["3v3", "5v5", "2v3", "4V4", "", "1v1 "] {
        let err = protocol::match_mode(&Value::str(unsupported)).unwrap_err();
        assert_eq!(
            err.code,
            protocol::ErrorCode::UnsupportedMatchMode,
            "match mode {unsupported} must be refused"
        );
        assert!(!err.message.is_empty());
    }
    for malformed in [Value::int(3), Value::bool(true), Value::Table(Vec::new())] {
        let err = protocol::match_mode(&malformed).unwrap_err();
        assert_eq!(err.code, protocol::ErrorCode::UnsupportedMatchMode);
    }
    let err = protocol::match_mode(&Value::Nil).unwrap_err();
    assert_eq!(err.code, protocol::ErrorCode::UnsupportedMatchMode);
}

#[test]
fn refuses_an_unsupported_mode_at_manifest_validation_before_readiness() {
    for mode in MODES {
        assert!(protocol::validate_manifest(&fixture::manifest(Some(mode))).is_ok());
    }
    let mut three = fixture::manifest(None);
    three.set("match_mode", Value::str("3v3"));
    let err = protocol::validate_manifest(&three).unwrap_err();
    assert_eq!(err.code, protocol::ErrorCode::UnsupportedMatchMode);

    three.set("match_mode", Value::Nil);
    let err = protocol::validate_manifest(&three).unwrap_err();
    assert_eq!(err.code, protocol::ErrorCode::UnsupportedMatchMode);
}

#[test]
fn carries_the_mode_in_the_deterministic_manifest_identity() {
    let mut ids: Vec<(String, protocol::MatchMode)> = Vec::new();
    for mode in MODES {
        let id = protocol::manifest_id(&fixture::manifest(Some(mode)));
        assert!(
            !ids.iter().any(|(existing, _)| *existing == id),
            "two modes share a manifest digest"
        );
        ids.push((id, mode));
    }
    let difference = protocol::manifest_difference(
        &fixture::manifest(Some(protocol::MatchMode::FourVFour)),
        &fixture::manifest(Some(protocol::MatchMode::TwoVTwo)),
    )
    .unwrap();
    assert_eq!(difference.path, "manifest.match_mode");
    assert_eq!(difference.expected, Value::str("4v4"));
    assert_eq!(difference.actual, Value::str("2v2"));
}

#[test]
fn refuses_a_3v3_proposal_on_the_wire_and_at_the_coordinator() {
    let manifest = fixture::manifest(None);
    let valid_id = protocol::manifest_id(&manifest);
    let mut three = manifest;
    three.set("match_mode", Value::str("3v3"));
    let err = protocol::new(
        protocol::MessageKind::ManifestProposal,
        &session_id(),
        HOST,
        0,
        Value::record(vec![
            ("manifest_id", Value::str(valid_id)),
            ("manifest", three.clone()),
        ]),
    )
    .unwrap_err();
    assert_eq!(err.code, protocol::ErrorCode::UnsupportedMatchMode);

    let (_, outcome) = coordinator::step(
        &fixture::host(None),
        Event::ProposeManifest { manifest: three },
    );
    assert!(!outcome.accepted);
    assert_eq!(
        outcome.code,
        Some(coordinator::RejectCode::UnsupportedMatchMode)
    );
}

// ---------------------------------------------------------------------------
// "OMP-3 multi-slot ownership"
// ---------------------------------------------------------------------------

#[test]
fn lets_one_human_cover_several_slots_while_every_slot_keeps_one_source() {
    for mode in MODES {
        let shape = protocol::match_mode_shape(mode);
        let (manifest, assignments) = seated(mode, shape.humans);
        assert!(
            protocol::validate_assignment_manifest(&manifest, &assignments).is_ok(),
            "{} ownership must validate",
            mode.wire_str()
        );
        let sources = coordinator::slot_sources(&manifest, &assignments).unwrap();
        let mut seen = 0;
        for index in 1..=input_frame::SLOT_COUNT {
            let slot = input_frame::slot(index).unwrap();
            let wire_id = protocol::slot_wire_id(slot.id);
            let source = sources
                .get(wire_id)
                .unwrap_or_else(|| panic!("slot_sources must be keyed by slot id {wire_id}"));
            assert_eq!(source.get("slot").and_then(Value::as_str), Some(wire_id));
            seen += 1;
        }
        assert_eq!(
            seen,
            input_frame::SLOT_COUNT,
            "{} must declare every canonical slot once",
            mode.wire_str()
        );
        for order in 1..=shape.humans {
            assert_eq!(
                protocol::owned_slots(&assignments, &format!("peer.{order}")).len() as i64,
                shape.slots_per_human
            );
        }
    }
}

#[test]
fn pins_the_owned_sets_each_mode_produces_in_canonical_order() {
    let (_, duo) = seated(protocol::MatchMode::OneVOne, 2);
    assert_eq!(owned_text(&duo, "peer.1"), "home_1,home_2,home_3,home_4");
    assert_eq!(owned_text(&duo, "peer.2"), "away_1,away_2,away_3,away_4");

    let (_, pair) = seated(protocol::MatchMode::TwoVTwo, 4);
    assert_eq!(owned_text(&pair, "peer.1"), "home_1,home_2");
    assert_eq!(owned_text(&pair, "peer.2"), "home_3,home_4");
    assert_eq!(owned_text(&pair, "peer.3"), "away_1,away_2");
    assert_eq!(owned_text(&pair, "peer.4"), "away_3,away_4");

    let (_, quad) = seated(protocol::MatchMode::FourVFour, 8);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).unwrap();
        assert_eq!(
            owned_text(&quad, &format!("peer.{index}")),
            protocol::slot_wire_id(slot.id)
        );
    }
    assert_eq!(owned_text(&quad, "nobody"), "");
}

#[test]
fn rejects_owned_set_sizes_that_disagree_with_the_frozen_mode() {
    // The identical assignments array is legal under one mode and refused
    // under another: the mode, not the array alone, decides.
    let pairings = [
        (protocol::MatchMode::OneVOne, protocol::MatchMode::FourVFour),
        (protocol::MatchMode::OneVOne, protocol::MatchMode::TwoVTwo),
        (protocol::MatchMode::TwoVTwo, protocol::MatchMode::FourVFour),
        (protocol::MatchMode::FourVFour, protocol::MatchMode::OneVOne),
        (protocol::MatchMode::FourVFour, protocol::MatchMode::TwoVTwo),
        (protocol::MatchMode::TwoVTwo, protocol::MatchMode::OneVOne),
    ];
    for (shaped_for, claimed) in pairings {
        let shape = protocol::match_mode_shape(shaped_for);
        let (_, assignments) = seated(shaped_for, shape.humans);
        let manifest = fixture::manifest(Some(claimed));
        let err = protocol::validate_assignment_manifest(&manifest, &assignments).unwrap_err();
        assert_eq!(
            err.code,
            protocol::ErrorCode::InvalidOwnership,
            "{} ownership must not pass as {}",
            shaped_for.wire_str(),
            claimed.wire_str()
        );
        assert!(
            err.message.contains(claimed.wire_str()),
            "reason must name {}, got {:?}",
            claimed.wire_str(),
            err.message
        );
    }
}

#[test]
fn rejects_a_partially_seated_owned_set_inside_one_mode() {
    let (manifest, mut assignments) = seated(protocol::MatchMode::TwoVTwo, 4);
    // Hand one of peer.1's two slots to peer.2, leaving 1 and 3.
    set_field(&mut assignments, 2, "producer_id", Value::str("peer.2"));
    let err = protocol::validate_assignment_manifest(&manifest, &assignments).unwrap_err();
    assert_eq!(err.code, protocol::ErrorCode::InvalidOwnership);
}

#[test]
fn keeps_every_declared_source_unambiguous_while_humans_span_slots() {
    let (manifest, assignments) = seated(protocol::MatchMode::FourVFour, 8);

    let mut shared_bot = assignments.clone();
    set_field(&mut shared_bot, 7, "producer_kind", Value::str("bot"));
    set_field(&mut shared_bot, 7, "producer_id", Value::str("bot.away_4"));
    set_field(&mut shared_bot, 7, "bot_seed", Value::int(5));
    set_field(&mut shared_bot, 8, "producer_kind", Value::str("bot"));
    set_field(&mut shared_bot, 8, "producer_id", Value::str("bot.away_4"));
    set_field(&mut shared_bot, 8, "bot_seed", Value::int(6));
    let err = protocol::validate_assignment_manifest(&manifest, &shared_bot)
        .expect_err("a bot producer may drive only one slot");
    assert_eq!(err.code, protocol::ErrorCode::Malformed);

    let mut crossed = assignments.clone();
    set_field(&mut crossed, 8, "producer_kind", Value::str("bot"));
    set_field(&mut crossed, 8, "producer_id", Value::str("peer.1"));
    set_field(&mut crossed, 8, "bot_seed", Value::int(7));
    let err = protocol::validate_assignment_manifest(&manifest, &crossed)
        .expect_err("a producer id may not be both peer and bot");
    assert_eq!(err.code, protocol::ErrorCode::Malformed);

    let straddling = fixture::manifest(Some(protocol::MatchMode::TwoVTwo));
    let (_, mut pair) = seated(protocol::MatchMode::TwoVTwo, 4);
    set_field(&mut pair, 4, "producer_id", Value::str("peer.3"));
    set_field(&mut pair, 5, "producer_id", Value::str("peer.2"));
    let err = protocol::validate_assignment_manifest(&straddling, &pair)
        .expect_err("one human's owned slots must sit on one team");
    assert_eq!(err.code, protocol::ErrorCode::Malformed);
}

#[test]
fn carries_multi_slot_ownership_across_the_canonical_wire() {
    let (manifest, assignments) = seated(protocol::MatchMode::OneVOne, 2);
    let message = protocol::new(
        protocol::MessageKind::SlotAssignment,
        &session_id(),
        HOST,
        0,
        Value::record(vec![
            ("manifest_id", Value::str(protocol::manifest_id(&manifest))),
            (
                "assignment_id",
                Value::str(protocol::assignment_id(&assignments, 1)),
            ),
            ("assignments", assignments.clone()),
        ]),
    )
    .unwrap();
    let wire = protocol::encode(&message).unwrap();
    let decoded = protocol::decode(&wire).unwrap();
    let body_assignments = decoded.body.get("assignments").unwrap();
    assert_eq!(protocol::owned_slots(body_assignments, "peer.1").len(), 4);
    assert!(protocol::validate_assignment_manifest(&manifest, body_assignments).is_ok());
}

#[test]
fn mints_a_new_ownership_generation_when_a_pair_is_repartitioned() {
    let (_, assignments) = seated(protocol::MatchMode::TwoVTwo, 4);
    let before = protocol::assignment_id(&assignments, 1);
    // Swap which pair each home human owns without changing the mode, the
    // roster, or how many slots anybody owns.
    let mut repartitioned = assignments.clone();
    set_field(&mut repartitioned, 1, "producer_id", Value::str("peer.2"));
    set_field(&mut repartitioned, 2, "producer_id", Value::str("peer.2"));
    set_field(&mut repartitioned, 3, "producer_id", Value::str("peer.1"));
    set_field(&mut repartitioned, 4, "producer_id", Value::str("peer.1"));
    assert_ne!(before, protocol::assignment_id(&repartitioned, 1));
    // Even byte-identical ownership republished is a distinct generation.
    assert_ne!(before, protocol::assignment_id(&assignments, 2));
}

#[test]
fn keeps_both_keepers_unassignable_in_every_mode() {
    for mode in MODES {
        let (manifest, assignments) = seated(mode, 1);
        for keeper in ["ozzo", "gax_oru"] {
            let mut attempt = assignments.clone();
            set_field(&mut attempt, 1, "player_id", Value::str(keeper));
            let err = coordinator::slot_sources(&manifest, &attempt).unwrap_err();
            assert_eq!(
                err.code,
                coordinator::RejectCode::InvalidAssignment,
                "{} must refuse a keeper in a canonical slot",
                mode.wire_str()
            );
        }
        // The manifest itself never names a keeper in a slot either.
        let mut named = manifest.clone();
        let mut slots = named.get("slots").unwrap().clone();
        set_field(&mut slots, 1, "player_id", Value::str("ozzo"));
        named.set("slots", slots);
        let err = protocol::validate_manifest(&named).unwrap_err();
        assert_eq!(err.code, protocol::ErrorCode::Malformed);
        // Keepers stay slotless: eight slots for ten roster players.
        assert_eq!(
            manifest.get("slots").unwrap().len() as i64,
            input_frame::SLOT_COUNT
        );
        assert_eq!(
            manifest
                .get("teams")
                .unwrap()
                .get_index(1)
                .unwrap()
                .get("roster")
                .unwrap()
                .len() as i64,
            input_frame::FIXTURE_TEAM_SIZE
        );
    }
}

// ---------------------------------------------------------------------------
// "OMP-3 mode-aware coordinator ownership"
// ---------------------------------------------------------------------------

#[test]
fn seats_humans_in_contiguous_owned_blocks_per_mode() {
    let duo = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::OneVOne)),
        &["a".to_string(), "b".to_string()],
    )
    .unwrap();
    assert_eq!(owned_text(&duo, "a"), "home_1,home_2,home_3,home_4");
    assert_eq!(owned_text(&duo, "b"), "away_1,away_2,away_3,away_4");

    let pair = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::TwoVTwo)),
        &[
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ],
    )
    .unwrap();
    assert_eq!(owned_text(&pair, "a"), "home_1,home_2");
    assert_eq!(owned_text(&pair, "d"), "away_3,away_4");

    // Bots fill the unseated blocks, exactly as they always did.
    let lonely = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::OneVOne)),
        &["a".to_string()],
    )
    .unwrap();
    assert_eq!(owned_text(&lonely, "a"), "home_1,home_2,home_3,home_4");
    for index in 5..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).unwrap();
        let producer = lonely.get_index(index).unwrap();
        assert_eq!(
            producer.get("producer_kind").and_then(Value::as_str),
            Some("bot")
        );
        let expected_id = format!("bot.{}", protocol::slot_wire_id(slot.id));
        assert_eq!(
            producer.get("producer_id").and_then(Value::as_str),
            Some(expected_id.as_str())
        );
    }
}

#[test]
fn keeps_4v4_seating_byte_identical_to_one_slot_per_human() {
    let manifest = fixture::manifest(None);
    let plan = coordinator::plan_assignments(&manifest, &fixture::peer_ids(7)).unwrap();
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).unwrap();
        let producer = plan.get_index(index).unwrap();
        assert_eq!(
            producer.get("slot").and_then(Value::as_str),
            Some(protocol::slot_wire_id(slot.id))
        );
        assert_eq!(
            producer.get("producer_kind").and_then(Value::as_str),
            Some("peer")
        );
        let producer_id = producer.get("producer_id").and_then(Value::as_str).unwrap();
        assert_eq!(protocol::owned_slots(&plan, producer_id).len(), 1);
    }
    assert_eq!(
        plan.get_index(1)
            .unwrap()
            .get("producer_id")
            .and_then(Value::as_str),
        Some(HOST)
    );
    assert_eq!(
        plan.get_index(8)
            .unwrap()
            .get("producer_id")
            .and_then(Value::as_str),
        Some(fixture::guest_peer_id(7).as_str())
    );
}

#[test]
fn refuses_more_humans_than_a_mode_seats() {
    let err = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::OneVOne)),
        &["a".to_string(), "b".to_string(), "c".to_string()],
    )
    .unwrap_err();
    assert_eq!(err.code, coordinator::RejectCode::Capacity);
    assert!(err.message.contains("1v1"));

    let err = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::TwoVTwo)),
        &[
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
        ],
    )
    .unwrap_err();
    assert_eq!(err.code, coordinator::RejectCode::Capacity);

    assert!(
        coordinator::plan_assignments(
            &fixture::manifest(Some(protocol::MatchMode::FourVFour)),
            &fixture::peer_ids(7)
        )
        .is_ok()
    );
}

#[test]
fn refuses_a_proposal_whose_mode_cannot_seat_the_admitted_lobby() {
    let mut state = fixture::host(None);
    for index in 1..=3 {
        let peer_id = fixture::guest_peer_id(index);
        let message = protocol::new(
            protocol::MessageKind::Handshake,
            &session_id(),
            &peer_id,
            0,
            Value::record(vec![
                ("role", Value::str("guest")),
                ("runtime", fixture::runtime()),
            ]),
        )
        .unwrap();
        state = deliver(&state, &peer_id, message).0;
    }
    let (next_state, outcome) = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(Some(protocol::MatchMode::OneVOne)),
        },
    );
    assert!(!outcome.accepted);
    assert_eq!(outcome.code, Some(coordinator::RejectCode::Capacity));
    assert_eq!(
        next_state, state,
        "a refused proposal leaves no progress behind"
    );

    // The same lobby is fine at a mode that seats it.
    let (_, ok_outcome) = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: fixture::manifest(Some(protocol::MatchMode::TwoVTwo)),
        },
    );
    assert!(ok_outcome.accepted);
}

#[test]
fn refuses_ownership_that_disagrees_with_the_admitted_lobby_or_the_mode() {
    let state = accepted_host(1, protocol::MatchMode::OneVOne);
    let good = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::OneVOne)),
        &[HOST.to_string(), fixture::guest_peer_id(1)],
    )
    .unwrap();
    let (_, outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: good,
            preserve_claims: false,
        },
    );
    assert!(outcome.accepted);

    // 4v4-shaped ownership under a 1v1 manifest.
    let (_, mut wrong_shape) = seated(protocol::MatchMode::FourVFour, 2);
    set_field(&mut wrong_shape, 1, "producer_id", Value::str(HOST));
    set_field(
        &mut wrong_shape,
        2,
        "producer_id",
        Value::str(fixture::guest_peer_id(1)),
    );
    let (_, shape_outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: wrong_shape,
            preserve_claims: false,
        },
    );
    assert!(!shape_outcome.accepted);
    assert_eq!(
        shape_outcome.code,
        Some(coordinator::RejectCode::InvalidOwnership)
    );

    // Correctly shaped for the mode, but leaving an admitted peer unseated.
    let unseated = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::OneVOne)),
        &[HOST.to_string()],
    )
    .unwrap();
    let (_, unseated_outcome) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: unseated,
            preserve_claims: false,
        },
    );
    assert!(!unseated_outcome.accepted);
    assert_eq!(
        unseated_outcome.code,
        Some(coordinator::RejectCode::InvalidAssignment)
    );
}

#[test]
fn freezes_the_mode_the_owned_sets_and_the_opening_live_slot() {
    let cases = [
        (protocol::MatchMode::OneVOne, 1),
        (protocol::MatchMode::TwoVTwo, 3),
        (protocol::MatchMode::FourVFour, 7),
    ];
    for (mode, guest_count) in cases {
        let shape = protocol::match_mode_shape(mode);
        let mut session = Driver::new(driver::Options {
            guest_count: Some(guest_count),
            mode: Some(mode),
            ..Default::default()
        });
        session.reach_start(Some(2), Some(0));
        assert!(
            session.all_started(),
            "{} never reached its start boundary",
            mode.wire_str()
        );
        let freeze = session
            .host()
            .state
            .freeze
            .clone()
            .expect("host session is frozen");
        assert_eq!(freeze.match_mode, mode);
        let mut humans = 0;
        for node in &session.nodes {
            let owned = freeze
                .owned
                .get(&node.peer_id)
                .expect("a human has no owned set");
            humans += 1;
            assert_eq!(owned.len() as i64, shape.slots_per_human);
            assert_eq!(freeze.live.get(&node.peer_id), Some(&owned[0]));
        }
        assert_eq!(humans, guest_count + 1);

        // Every peer froze the same ownership model, not merely its own row.
        for node in &session.nodes {
            let peer_freeze = node.state.freeze.clone().expect("peer session is frozen");
            assert_eq!(peer_freeze.match_mode, freeze.match_mode);
            assert_eq!(peer_freeze.assignment_id, freeze.assignment_id);
            for other in &session.nodes {
                let left = peer_freeze
                    .owned
                    .get(&other.peer_id)
                    .cloned()
                    .unwrap_or_default();
                let right = freeze
                    .owned
                    .get(&other.peer_id)
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(joined_slots(&left), joined_slots(&right));
                assert_eq!(
                    peer_freeze.live.get(&other.peer_id),
                    freeze.live.get(&other.peer_id)
                );
            }
        }
    }
}

#[test]
fn reports_owned_sets_from_published_and_frozen_ownership_alike() {
    let mut state = accepted_host(1, protocol::MatchMode::TwoVTwo);
    assert_eq!(
        coordinator::owned_slots(&state, HOST).len(),
        0,
        "unpublished ownership owns nothing"
    );
    let plan = coordinator::plan_assignments(
        &fixture::manifest(Some(protocol::MatchMode::TwoVTwo)),
        &[HOST.to_string(), fixture::guest_peer_id(1)],
    )
    .unwrap();
    state = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: plan,
            preserve_claims: false,
        },
    )
    .0;
    assert_eq!(
        joined_slots(&coordinator::owned_slots(&state, HOST)),
        "home_1,home_2"
    );
    assert_eq!(coordinator::owned_slots(&state, "stranger").len(), 0);
    assert_eq!(
        coordinator::slot_owner(&state, SlotId::Home2)
            .unwrap()
            .get("producer_id")
            .and_then(Value::as_str),
        Some(HOST)
    );
}

#[test]
fn plays_a_full_1v1_and_a_full_2v2_session_through_to_a_result() {
    let cases = [
        (protocol::MatchMode::OneVOne, 1),
        (protocol::MatchMode::TwoVTwo, 3),
    ];
    for (mode, guest_count) in cases {
        let mut session = Driver::new(driver::Options {
            guest_count: Some(guest_count),
            mode: Some(mode),
            ..Default::default()
        });
        session.reach_start(Some(3), Some(0));
        assert!(session.all_started(), "{} never started", mode.wire_str());
        session.play_out(Some(2), Some(1));
        assert!(
            session.all_terminal(Some(TerminalReason::Completed)),
            "{} never completed",
            mode.wire_str()
        );
    }
}

// The phase gate that refuses this is mode-blind and shared with every other
// post-freeze configuration change, but #225 names the 2v2 pair explicitly,
// so it gets a test that names it explicitly rather than one that has to be
// reasoned about from the general case.
#[test]
fn freezes_a_2v2_humans_chosen_pair_at_countdown_and_mid_match() {
    let guest = fixture::guest_peer_id(1);
    let manifest = fixture::manifest(Some(protocol::MatchMode::TwoVTwo));
    let mut session = Driver::new(driver::Options {
        guest_count: Some(3),
        mode: Some(protocol::MatchMode::TwoVTwo),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: manifest.clone(),
        },
    );
    session.pump();
    let plan = coordinator::plan_assignments(&manifest, &fixture::peer_ids(3)).unwrap();
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: plan.clone(),
            preserve_claims: false,
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
            remaining_ticks: 4,
            first_input_tick: 0,
        },
    );
    session.pump();

    {
        let freeze = session.host().state.freeze.clone().unwrap();
        assert_eq!(freeze.match_mode, protocol::MatchMode::TwoVTwo);
        assert_eq!(
            joined_slots(freeze.owned.get(HOST).unwrap()),
            "home_1,home_2"
        );
        assert_eq!(
            joined_slots(freeze.owned.get(&guest).unwrap()),
            "home_3,home_4"
        );
    }

    // Swap which pair the two home humans hold. Same mode, same owned-set
    // sizes, same roster, same eight declared sources: only the partition
    // moves. It is perfectly legal ownership — it is the freeze, not the
    // ownership rules, that has to refuse it.
    let mut repartitioned = plan.clone();
    set_field(
        &mut repartitioned,
        1,
        "producer_id",
        Value::str(guest.as_str()),
    );
    set_field(
        &mut repartitioned,
        2,
        "producer_id",
        Value::str(guest.as_str()),
    );
    set_field(&mut repartitioned, 3, "producer_id", Value::str(HOST));
    set_field(&mut repartitioned, 4, "producer_id", Value::str(HOST));
    assert!(protocol::validate_assignment_manifest(&manifest, &repartitioned).is_ok());

    let stages = [
        (false, protocol::LifecyclePhase::Countdown),
        (true, protocol::LifecyclePhase::Running),
    ];
    for (needs_tick, phase) in stages {
        if needs_tick {
            session.tick(Some(5));
            assert!(session.all_started(), "the 2v2 session never started");
        }
        assert_eq!(session.host().state.phase, phase);
        let before = session.host().state.clone();
        let outcome = session.send(
            HOST,
            Event::AssignSlots {
                assignments: repartitioned.clone(),
                preserve_claims: false,
            },
        );
        assert!(
            !outcome.accepted,
            "a pair may not be repartitioned in {phase:?}"
        );
        assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
        assert_eq!(
            session.host().state,
            before,
            "a refused repartition leaves no progress behind"
        );
    }

    // Every peer's frozen pair, and its opening live slot, is untouched.
    for node in &session.nodes {
        let freeze = node.state.freeze.clone().unwrap();
        assert_eq!(
            joined_slots(freeze.owned.get(HOST).unwrap()),
            "home_1,home_2"
        );
        assert_eq!(
            joined_slots(freeze.owned.get(&guest).unwrap()),
            "home_3,home_4"
        );
        assert_eq!(freeze.live.get(HOST), Some(&SlotId::Home1));
        assert_eq!(freeze.live.get(&guest), Some(&SlotId::Home3));
    }
}

// The owned-set/mode check also has to hold on the receiving end of a real
// link, not only in `plan_assignments` and the host's local command path.
// `inject` sends a message the sender's own coordinator would never emit,
// over the sender's link and canonical wire, so this exercises
// `apply_slot_assignment` exactly as production would.
#[test]
fn terminates_a_guest_sent_ownership_that_disagrees_with_the_frozen_mode() {
    let guest = fixture::guest_peer_id(1);
    let manifest = fixture::manifest(Some(protocol::MatchMode::TwoVTwo));
    let mut session = Driver::new(driver::Options {
        guest_count: Some(3),
        mode: Some(protocol::MatchMode::TwoVTwo),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: manifest.clone(),
        },
    );
    session.pump();
    assert_eq!(
        session.node(&guest).unwrap().state.phase,
        protocol::LifecyclePhase::Manifest
    );

    // Wire-valid ownership: eight canonical slots in order, one declared
    // source each, no keeper, and the guest genuinely seated — but shaped
    // for 4v4 while the accepted manifest says 2v2. Nothing about the bytes
    // is wrong, so only the manifest-aware check can catch it.
    let four_v_four_manifest = fixture::manifest(Some(protocol::MatchMode::FourVFour));
    let one_slot_each =
        coordinator::plan_assignments(&four_v_four_manifest, &fixture::peer_ids(3)).unwrap();
    assert!(protocol::validate_assignment_manifest(&four_v_four_manifest, &one_slot_each).is_ok());
    assert_eq!(
        protocol::owned_slots(&one_slot_each, &guest).len(),
        1,
        "the guest is seated, just wrongly"
    );

    session.inject(
        HOST,
        &guest,
        protocol::MessageKind::SlotAssignment,
        Value::record(vec![
            ("manifest_id", Value::str(protocol::manifest_id(&manifest))),
            (
                "assignment_id",
                Value::str(protocol::assignment_id(&one_slot_each, 1)),
            ),
            ("assignments", one_slot_each),
        ]),
        None,
    );

    let node = session.node(&guest).unwrap();
    let terminal = node
        .terminal
        .clone()
        .expect("the guest accepted ownership it should refuse");
    assert_eq!(terminal.reason, TerminalReason::InvalidAssignment);
    assert_eq!(terminal.code.as_deref(), Some("invalid_assignment"));
    assert_eq!(terminal.origin, Origin::Remote);
    let detail = terminal.detail.clone().unwrap_or_default();
    assert!(
        detail.contains("2v2 seats 2 per human"),
        "the guest must name the mode disagreement, got {detail:?}"
    );
    assert_eq!(
        node.state.assignments, None,
        "refused ownership is never adopted"
    );
    // The blast radius is that one link: the host drops the guest and lives.
    assert_eq!(session.host().terminal, None);
}

// ---------------------------------------------------------------------------
// "OMP-3 live slot selection"
// ---------------------------------------------------------------------------

#[test]
fn auto_switches_to_an_owned_slot_that_wins_the_ball() {
    let owned = [SlotId::Home1, SlotId::Home2, SlotId::Home3, SlotId::Home4];
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: false,
                carrier: None,
                winner: Some(SlotId::Home3),
                ranked: vec![SlotId::Home2],
            },
        ),
        SlotId::Home3
    );
    // Winning the ball outranks a simultaneous switch request.
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: true,
                carrier: None,
                winner: Some(SlotId::Home4),
                ranked: vec![SlotId::Home2, SlotId::Home3],
            },
        ),
        SlotId::Home4
    );
    // A slot outside the owned set winning the ball changes nothing.
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: false,
                carrier: None,
                winner: Some(SlotId::Away2),
                ranked: vec![SlotId::Home3],
            },
        ),
        SlotId::Home1
    );
}

#[test]
fn switches_to_the_owned_slot_nearest_the_ball_and_only_when_eligible() {
    let owned = [SlotId::Home1, SlotId::Home2, SlotId::Home3, SlotId::Home4];
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: true,
                carrier: None,
                winner: None,
                ranked: vec![SlotId::Away1, SlotId::Home3, SlotId::Home2],
            },
        ),
        SlotId::Home3,
        "the nearest owned slot wins, skipping unowned ones"
    );
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: false,
                carrier: None,
                winner: None,
                ranked: vec![SlotId::Home4],
            },
        ),
        SlotId::Home1,
        "no switch edge, no transition"
    );
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: true,
                carrier: Some(SlotId::Home1),
                winner: None,
                ranked: vec![SlotId::Home4],
            },
        ),
        SlotId::Home1,
        "switching while carrying the ball is refused, as in solo play"
    );
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home1,
            &live_slot::LiveTransition {
                switch: true,
                carrier: None,
                winner: None,
                ranked: vec![SlotId::Away3],
            },
        ),
        SlotId::Home1,
        "an entirely unowned ranking leaves the live slot alone"
    );
    assert_eq!(
        coordinator::next_live_slot(
            &owned,
            SlotId::Home2,
            &live_slot::LiveTransition {
                switch: true,
                carrier: None,
                winner: None,
                ranked: Vec::new(),
            },
        ),
        SlotId::Home2,
        "an absent ranking leaves the live slot alone"
    );
}

#[test]
fn is_inert_in_4v4_without_any_mode_special_case() {
    let owned = [SlotId::Home2];
    for switch in [true, false] {
        for winner in [SlotId::Home2, SlotId::Home1, SlotId::Away4] {
            for carrier in [SlotId::Home2, SlotId::Away1] {
                assert_eq!(
                    coordinator::next_live_slot(
                        &owned,
                        SlotId::Home2,
                        &live_slot::LiveTransition {
                            switch,
                            carrier: Some(carrier),
                            winner: Some(winner),
                            ranked: vec![
                                SlotId::Away1,
                                SlotId::Home1,
                                SlotId::Home2,
                                SlotId::Home3
                            ],
                        },
                    ),
                    SlotId::Home2,
                    "a one-slot owned set can never change who is live"
                );
            }
        }
    }
}

#[test]
fn keeps_exactly_one_live_slot_per_human_and_drives_the_rest_with_ai() {
    let cases = [
        (protocol::MatchMode::OneVOne, 1),
        (protocol::MatchMode::TwoVTwo, 3),
        (protocol::MatchMode::FourVFour, 7),
    ];
    for (mode, guest_count) in cases {
        let shape = protocol::match_mode_shape(mode);
        let mut session = Driver::new(driver::Options {
            guest_count: Some(guest_count),
            mode: Some(mode),
            ..Default::default()
        });
        session.reach_start(Some(2), Some(0));
        let freeze = session.host().state.freeze.clone().unwrap();
        let drivers = coordinator::slot_drivers(&freeze, None);
        let mut human_slots: i64 = 0;
        for index in 1..=input_frame::SLOT_COUNT {
            let kind = drivers[(index - 1) as usize];
            if kind == SlotDriver::Human {
                human_slots += 1;
            }
        }
        assert_eq!(
            human_slots,
            guest_count + 1,
            "{} must expose one live slot per human",
            mode.wire_str()
        );

        // Moving a human's live slot moves exactly one human row with it.
        if shape.slots_per_human > 1 {
            let owned = freeze.owned.get(HOST).unwrap().clone();
            let mut moved = freeze.live.clone();
            moved.insert(HOST.to_string(), *owned.last().unwrap());
            let after = coordinator::slot_drivers(&freeze, Some(&moved));
            let vacated_index = (live_slot::slot_index(owned[0]) - 1) as usize;
            assert_eq!(
                after[vacated_index],
                SlotDriver::Ai,
                "the vacated owned slot falls back to AI"
            );
            let moved_index = (live_slot::slot_index(*owned.last().unwrap()) - 1) as usize;
            assert_eq!(after[moved_index], SlotDriver::Human);
            let still_human = after
                .iter()
                .filter(|&&kind| kind == SlotDriver::Human)
                .count() as i64;
            assert_eq!(still_human, human_slots);
        }
    }
}
