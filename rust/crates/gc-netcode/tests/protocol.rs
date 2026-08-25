//! Unit and differential tests for the wire protocol (`protocol.rs`):
//! encode/decode round-tripping, malformed-input rejection, and exact-byte
//! validation against a frozen reference.
//!
//! Also carries this crate's required differential evidence for
//! `protocol.rs`: `tests/fixtures/protocol_lua_reference.txt` holds wire bytes and digests
//! captured from the original protocol implementation before it was retired
//! (see `tools/lua_reference/README.md` for capture provenance), and the
//! tests in the `differential` module below assert against those bytes
//! directly rather than merely round-tripping Rust-produced values through
//! themselves.

use std::panic::{AssertUnwindSafe, catch_unwind};

use gc_data::network_profiles::NetworkProfileName;
use gc_netcode::coordinator::{self, Event};
use gc_netcode::coordinator_fixture;
use gc_netcode::fault_harness::{FaultHarness, FaultHarnessOptions, FaultHarnessTopology};
use gc_netcode::fault_transport::{TransportChannel, TransportMessage, TransportMessageType};
use gc_netcode::input_protocol;
use gc_netcode::live_slot;
use gc_netcode::match_driver::{
    self, HostBatchErrorCode, HostBatchRequest, InputPacketArrival, MatchDriverRules,
    MatchDriverStatus, ProducerKind, SessionManifest as DriverSessionManifest, SlotAssignment,
};
use gc_netcode::match_driver_fixture::DriverRules;
use gc_netcode::protocol::{self, ErrorCode, LifecyclePhase, MatchMode, MessageKind, Value};
use gc_netcode::protocol_conformance as conformance;
use gc_netcode::protocol_fixture as fixture;
use gc_sim::input_frame;

const LUA_REFERENCE: &str = include_str!("fixtures/protocol_lua_reference.txt");

/// Looks up `KEY=value` from the differential fixture file.
fn lua_ref(key: &str) -> &'static str {
    for line in LUA_REFERENCE.lines() {
        if let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            return rest;
        }
    }
    panic!("missing lua reference key: {key}");
}

fn bounded_id(prefix: &str, length: usize) -> String {
    assert!(prefix.len() <= length);
    format!("{prefix}{}", "x".repeat(length - prefix.len()))
}

/// A raw, non-validating canonical-ish encoder used only by tests to build
/// deliberately non-canonical (e.g. sparse-keyed) wires that
/// `protocol::encode`'s own validation would refuse to produce. Named after,
/// and serving the same purpose as, the reference test suite's own local
/// `encode_raw_test_value`/`encode_raw_test_wire` helpers: testing
/// `decode`'s defenses needs wires `encode` itself cannot be asked to emit.
fn encode_raw_value(value: &Value) -> String {
    match value {
        Value::Nil => "z".to_string(),
        Value::Bool(b) => if *b { "b1" } else { "b0" }.to_string(),
        Value::Int(n) => {
            let text = n.to_string();
            format!("i{}:{}", text.len(), text)
        }
        Value::Str(s) => format!("s{}:{}", s.len(), s),
        Value::Table(entries) => {
            let mut keys: Vec<&Value> = entries.iter().map(|(k, _)| k).collect();
            keys.sort_by(|a, b| match (a, b) {
                (Value::Int(x), Value::Int(y)) => x.cmp(y),
                (Value::Str(x), Value::Str(y)) => x.cmp(y),
                (Value::Int(_), Value::Str(_)) => std::cmp::Ordering::Less,
                (Value::Str(_), Value::Int(_)) => std::cmp::Ordering::Greater,
                _ => unreachable!(),
            });
            let mut out = format!("t{}:", keys.len());
            for key in keys {
                let item = entries.iter().find(|(k, _)| k == key).unwrap().1.clone();
                out.push_str(&encode_raw_value(key));
                out.push_str(&encode_raw_value(&item));
            }
            out
        }
    }
}

fn encode_raw_wire(message: &protocol::ControlMessage) -> String {
    format!(
        "GCOP;{};{}",
        protocol::VERSION,
        encode_raw_value(&message.to_value())
    )
}

fn sparse_array(pairs: Vec<(i64, Value)>) -> Value {
    Value::Table(pairs.into_iter().map(|(i, v)| (Value::int(i), v)).collect())
}

fn expect_malformed<T: std::fmt::Debug>(result: protocol::Result<T>) {
    let err = result.expect_err("expected a malformed result");
    assert_eq!(err.code, ErrorCode::Malformed);
}

/// One malformed-body mutation case: which kind, and how to break its body.
type MutationCase = (MessageKind, fn(&mut Value));

fn replace_manifest_player_id(
    manifest: &mut Value,
    team_index: i64,
    roster_index: i64,
    player_id: &str,
) {
    let teams = manifest.get("teams").unwrap().clone();
    let mut team = teams.get_index(team_index).unwrap().clone();
    let roster = team.get("roster").unwrap().clone();
    let previous = roster
        .get_index(roster_index)
        .unwrap()
        .get("player_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    // `Value::set` only supports string keys, so the roster array is rebuilt
    // rather than mutated in place (mirrors the immutable-update style
    // ARCHITECTURE.md §4 rule 7 asks for in pure code).
    let mut new_roster_items = Vec::new();
    for index in 1..=roster.len() as i64 {
        if index == roster_index {
            let mut p = roster.get_index(index).unwrap().clone();
            p.set("player_id", Value::str(player_id));
            new_roster_items.push(p);
        } else {
            new_roster_items.push(roster.get_index(index).unwrap().clone());
        }
    }
    let roster = Value::array(new_roster_items);
    team.set("roster", roster);
    let mut new_teams = Vec::new();
    for index in 1..=teams.len() as i64 {
        if index == team_index {
            new_teams.push(team.clone());
        } else {
            new_teams.push(teams.get_index(index).unwrap().clone());
        }
    }
    let teams = Value::array(new_teams);
    manifest.set("teams", teams);

    let slots = manifest.get("slots").unwrap().clone();
    let mut new_slots = Vec::new();
    for index in 1..=slots.len() as i64 {
        let mut slot = slots.get_index(index).unwrap().clone();
        if slot.get("player_id").and_then(Value::as_str) == Some(previous.as_str()) {
            slot.set("player_id", Value::str(player_id));
        }
        new_slots.push(slot);
    }
    let slots = Value::array(new_slots);
    manifest.set("slots", slots);
}

// ---------------------------------------------------------------------------
// describe "OMP-3 online protocol"
// ---------------------------------------------------------------------------

#[test]
fn omp3_online_protocol_pins_the_accepted_input_snapshot_tape_and_combat_schema_versions() {
    assert_eq!(protocol::CURRENT_VERSIONS.protocol, 1);
    assert_eq!(protocol::CURRENT_VERSIONS.input, 2);
    assert_eq!(protocol::CURRENT_VERSIONS.snapshot, 15);
    assert_eq!(protocol::CURRENT_VERSIONS.tape, 2);
    assert_eq!(protocol::CURRENT_VERSIONS.combat, 3);
}

#[test]
fn omp3_online_protocol_matches_literal_wire_manifest_transcript_and_per_kind_golden_evidence() {
    let report = conformance::verify();
    // #489: repinned alongside `protocol_conformance::GOLDEN` -- see that
    // constant's doc comment. `match_snapshot::COMBAT_VERSION` moved
    // 13 -> 14 under #489 and 14 -> 15 under #490.
    assert_eq!(report.manifest_id, "90b90970080d7978");
    assert_eq!(report.transcript_id, "1b8407df3614a2cb");
    assert_eq!(report.message_count, 15);
    assert_eq!(
        gc_core::fnv1a64::hash(conformance::GOLDEN.complete_wire.as_bytes()),
        "d0907dd1786309f5"
    );
    assert_eq!(
        conformance::marker(&report),
        "GC_PROTOCOL|golden|schema=1|manifest_id=90b90970080d7978|transcript_id=1b8407df3614a2cb|messages=15"
    );
}

#[test]
fn omp3_online_protocol_pins_the_control_vocabulary_this_build_speaks() {
    // #612: repinned alongside `protocol_conformance::GOLDEN` -- see that
    // constant's doc comment. `Start`'s allowed phases widen to admit a
    // resend/duplicate-echo no-op, moving the vocabulary digest itself.
    assert_eq!(protocol::vocabulary_id(), "93f9c16ad1674b97");
    assert_eq!(protocol::vocabulary_id(), conformance::GOLDEN.vocabulary_id);
    assert_eq!(protocol::vocabulary_id(), protocol::vocabulary_id());
    assert!(
        protocol::vocabulary_id()
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    );
    assert!(!conformance::marker(&conformance::verify()).contains("vocabulary"));
}

#[test]
fn omp3_online_protocol_digests_every_part_of_the_vocabulary_a_peer_has_to_agree_with() {
    let kinds: protocol::Vocabulary = vec![
        ("alpha".to_string(), vec!["one".to_string()]),
        ("beta".to_string(), vec!["two".to_string()]),
    ];
    let phases: protocol::Vocabulary = vec![
        ("alpha".to_string(), vec!["ready".to_string()]),
        ("beta".to_string(), vec!["ready".to_string()]),
    ];
    let baseline = protocol::vocabulary_digest(&kinds, &phases);
    assert_eq!(baseline, protocol::vocabulary_digest(&kinds, &phases));

    // an added message kind
    let mut k = kinds.clone();
    let mut p = phases.clone();
    k.push(("gamma".to_string(), vec!["three".to_string()]));
    p.push(("gamma".to_string(), vec!["ready".to_string()]));
    assert_ne!(
        protocol::vocabulary_digest(&k, &p),
        baseline,
        "an added message kind"
    );

    // an added body field
    let mut k = kinds.clone();
    k[0].1.push("extra".to_string());
    assert_ne!(
        protocol::vocabulary_digest(&k, &phases),
        baseline,
        "an added body field"
    );

    // an added allowed phase
    let mut p = phases.clone();
    p[1].1.push("countdown".to_string());
    assert_ne!(
        protocol::vocabulary_digest(&kinds, &p),
        baseline,
        "an added allowed phase"
    );

    // a renamed message kind
    let mut k = kinds.clone();
    let mut p = phases.clone();
    k[1] = ("delta".to_string(), vec!["two".to_string()]);
    p[1] = ("delta".to_string(), vec!["ready".to_string()]);
    assert_ne!(
        protocol::vocabulary_digest(&k, &p),
        baseline,
        "a renamed message kind"
    );

    // insertion order never matters
    let shuffled_kinds: protocol::Vocabulary = vec![
        ("beta".to_string(), vec!["two".to_string()]),
        ("alpha".to_string(), vec!["one".to_string()]),
    ];
    let shuffled_phases: protocol::Vocabulary = vec![
        ("beta".to_string(), vec!["ready".to_string()]),
        ("alpha".to_string(), vec!["ready".to_string()]),
    ];
    assert_eq!(
        protocol::vocabulary_digest(&shuffled_kinds, &shuffled_phases),
        baseline
    );
}

#[test]
fn omp3_online_protocol_constructs_owned_runtime_and_deterministic_manifest_records() {
    let mut runtime_source = fixture::runtime();
    let runtime = protocol::new_runtime(&runtime_source).unwrap();
    let mut capabilities = runtime_source.get("capabilities").unwrap().clone();
    let mut items: Vec<Value> = (1..=capabilities.len() as i64)
        .map(|i| capabilities.get_index(i).unwrap().clone())
        .collect();
    items[0] = Value::str("mutated");
    capabilities = Value::array(items);
    runtime_source.set("capabilities", capabilities);
    assert_eq!(
        runtime
            .get("capabilities")
            .unwrap()
            .get_index(1)
            .unwrap()
            .as_str(),
        Some("combat_feedback.v1")
    );

    let mut manifest_source = fixture::manifest(None);
    let manifest = protocol::new_manifest(&manifest_source).unwrap();
    let mut slots = manifest_source.get("slots").unwrap().clone();
    let mut slot1 = slots.get_index(1).unwrap().clone();
    slot1.set("player_id", Value::str("mutated"));
    let mut new_slots: Vec<Value> = vec![slot1];
    for index in 2..=slots.len() as i64 {
        new_slots.push(slots.get_index(index).unwrap().clone());
    }
    slots = Value::array(new_slots);
    manifest_source.set("slots", slots);
    assert_eq!(
        manifest
            .get("slots")
            .unwrap()
            .get_index(1)
            .unwrap()
            .get("player_id")
            .and_then(Value::as_str),
        Some("zyro_vex")
    );
}

#[test]
fn omp3_online_protocol_rejects_sparse_arrays_without_relying_on_luas_undefined_length_operator() {
    let mut runtime = fixture::runtime();
    let capabilities = runtime.get("capabilities").unwrap().clone();
    let sparse = sparse_array(vec![
        (1, capabilities.get_index(1).unwrap().clone()),
        (3, capabilities.get_index(3).unwrap().clone()),
    ]);
    runtime.set("capabilities", sparse);
    expect_malformed(protocol::validate_runtime(&runtime));

    // sparse teams
    {
        let mut manifest = fixture::manifest(None);
        let teams = manifest.get("teams").unwrap().clone();
        let sparse = sparse_array(vec![
            (1, teams.get_index(1).unwrap().clone()),
            (3, teams.get_index(2).unwrap().clone()),
        ]);
        manifest.set("teams", sparse);
        expect_malformed(protocol::validate_manifest(&manifest));
    }
    // sparse roster
    {
        let mut manifest = fixture::manifest(None);
        let mut teams = manifest.get("teams").unwrap().clone();
        let mut team1 = teams.get_index(1).unwrap().clone();
        let roster = team1.get("roster").unwrap().clone();
        let sparse = sparse_array(vec![
            (1, roster.get_index(1).unwrap().clone()),
            (2, roster.get_index(2).unwrap().clone()),
            (3, roster.get_index(3).unwrap().clone()),
            (4, roster.get_index(4).unwrap().clone()),
            (6, roster.get_index(5).unwrap().clone()),
        ]);
        team1.set("roster", sparse);
        let mut new_teams = vec![team1];
        for index in 2..=teams.len() as i64 {
            new_teams.push(teams.get_index(index).unwrap().clone());
        }
        teams = Value::array(new_teams);
        manifest.set("teams", teams);
        expect_malformed(protocol::validate_manifest(&manifest));
    }
    // sparse slots
    {
        let mut manifest = fixture::manifest(None);
        let slots = manifest.get("slots").unwrap().clone();
        let mut pairs: Vec<(i64, Value)> = (1..=7)
            .map(|i| (i, slots.get_index(i).unwrap().clone()))
            .collect();
        pairs.push((9, slots.get_index(8).unwrap().clone()));
        manifest.set("slots", sparse_array(pairs));
        expect_malformed(protocol::validate_manifest(&manifest));
    }

    // sparse producer assignments inside a `slot_assignment` message
    {
        let assignments = fixture::assignments();
        let mut pairs: Vec<(i64, Value)> = (1..=7)
            .map(|i| (i, assignments.get_index(i).unwrap().clone()))
            .collect();
        pairs.push((9, assignments.get_index(8).unwrap().clone()));
        let mut message = fixture::messages()[4].clone();
        message.body.set("assignments", sparse_array(pairs));
        expect_malformed(protocol::validate(&message));
    }

    // a sparse transcript panics rather than silently under-counting
    let messages = fixture::messages();
    let sparse_transcript = vec![messages[0].clone(), messages[1].clone()];
    let result = catch_unwind(AssertUnwindSafe(|| {
        protocol::transcript_id(&sparse_transcript)
    }));
    assert!(
        result.is_ok(),
        "a two-message transcript with contiguous sequences 0,1 is valid"
    );
    // The original test suite fed a *sparse-keyed table* (`{[1]=a,[3]=b}`)
    // for this case, which has no Rust equivalent for a `&[ControlMessage]`
    // slice (Rust slices cannot be sparse) — see the module doc comment.
    // What *is* portable and is asserted directly below: `transcript_id`
    // panics on a non-monotonic per-peer sequence, the same invariant the
    // original assertion guards.
    let mut non_monotonic = vec![messages[0].clone(), messages[0].clone()];
    non_monotonic[1].sequence = 0;
    non_monotonic[1].message_id =
        protocol::message_id(&non_monotonic[1].session_id, &non_monotonic[1].peer_id, 0).unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| protocol::transcript_id(&non_monotonic)));
    assert!(
        result.is_err(),
        "transcript_id must panic on a non-monotonic sequence"
    );
}

#[test]
fn omp3_online_protocol_rejects_sparse_numeric_key_arrays_parsed_from_independent_raw_wires() {
    let messages = fixture::messages();

    // runtime capabilities
    {
        let mut message = messages[0].clone();
        let mut runtime = message.body.get("runtime").unwrap().clone();
        let capabilities = runtime.get("capabilities").unwrap().clone();
        runtime.set(
            "capabilities",
            sparse_array(vec![
                (1, capabilities.get_index(1).unwrap().clone()),
                (3, capabilities.get_index(3).unwrap().clone()),
            ]),
        );
        message.body.set("runtime", runtime);
        expect_malformed(protocol::decode(&encode_raw_wire(&message)));
    }
    // manifest teams
    {
        let mut message = messages[1].clone();
        let mut manifest = message.body.get("manifest").unwrap().clone();
        let teams = manifest.get("teams").unwrap().clone();
        manifest.set(
            "teams",
            sparse_array(vec![
                (1, teams.get_index(1).unwrap().clone()),
                (3, teams.get_index(2).unwrap().clone()),
            ]),
        );
        message.body.set("manifest", manifest);
        expect_malformed(protocol::decode(&encode_raw_wire(&message)));
    }
    // manifest roster
    {
        let mut message = messages[1].clone();
        let mut manifest = message.body.get("manifest").unwrap().clone();
        let mut teams = manifest.get("teams").unwrap().clone();
        let mut team1 = teams.get_index(1).unwrap().clone();
        let roster = team1.get("roster").unwrap().clone();
        let sparse = sparse_array(vec![
            (1, roster.get_index(1).unwrap().clone()),
            (2, roster.get_index(2).unwrap().clone()),
            (3, roster.get_index(3).unwrap().clone()),
            (4, roster.get_index(4).unwrap().clone()),
            (6, roster.get_index(5).unwrap().clone()),
        ]);
        team1.set("roster", sparse);
        let mut new_teams = vec![team1];
        for index in 2..=teams.len() as i64 {
            new_teams.push(teams.get_index(index).unwrap().clone());
        }
        teams = Value::array(new_teams);
        manifest.set("teams", teams);
        message.body.set("manifest", manifest);
        expect_malformed(protocol::decode(&encode_raw_wire(&message)));
    }
    // manifest slots
    {
        let mut message = messages[1].clone();
        let mut manifest = message.body.get("manifest").unwrap().clone();
        let slots = manifest.get("slots").unwrap().clone();
        let mut pairs: Vec<(i64, Value)> = (1..=7)
            .map(|i| (i, slots.get_index(i).unwrap().clone()))
            .collect();
        pairs.push((9, slots.get_index(8).unwrap().clone()));
        manifest.set("slots", sparse_array(pairs));
        message.body.set("manifest", manifest);
        expect_malformed(protocol::decode(&encode_raw_wire(&message)));
    }
    // producer assignments
    {
        let mut message = messages[4].clone();
        let assignments = message.body.get("assignments").unwrap().clone();
        let mut pairs: Vec<(i64, Value)> = (1..=7)
            .map(|i| (i, assignments.get_index(i).unwrap().clone()))
            .collect();
        pairs.push((9, assignments.get_index(8).unwrap().clone()));
        message.body.set("assignments", sparse_array(pairs));
        expect_malformed(protocol::decode(&encode_raw_wire(&message)));
    }
}

#[test]
fn omp3_online_protocol_round_trips_every_control_message_through_one_canonical_bounded_codec() {
    let messages = fixture::messages();
    let expected_kinds = [
        MessageKind::Handshake,
        MessageKind::ManifestProposal,
        MessageKind::ManifestAccept,
        MessageKind::PeerAssignment,
        MessageKind::SlotAssignment,
        MessageKind::Ready,
        MessageKind::Countdown,
        MessageKind::Start,
        MessageKind::MatchPhase,
        MessageKind::HashReport,
        MessageKind::ResultAck,
        MessageKind::Abort,
        MessageKind::Disconnect,
        MessageKind::PairPreference,
        MessageKind::PairPreferenceResult,
    ];
    assert_eq!(messages.len(), expected_kinds.len());
    for (index, message) in messages.iter().enumerate() {
        assert_eq!(message.kind, expected_kinds[index]);
        let wire = protocol::encode(message).unwrap();
        assert!(wire.len() <= protocol::MAX_WIRE_BYTES);
        let decoded = protocol::decode(&wire).unwrap();
        assert_eq!(protocol::encode(&decoded).unwrap(), wire);
        let sequence = index.to_string();
        assert_eq!(
            decoded.message_id,
            format!("GCMI;1;13:session_alpha4:host{}:{sequence}", sequence.len())
        );
    }
}

#[test]
fn omp3_online_protocol_uses_injective_transcript_ids_through_exact_component_maxima() {
    let session_id = "s".repeat(protocol::MAX_SESSION_ID_BYTES);
    let peer_id = "p".repeat(protocol::MAX_PEER_ID_BYTES);
    let message_id = protocol::message_id(&session_id, &peer_id, protocol::MAX_SEQUENCE).unwrap();
    assert_eq!(message_id.len(), protocol::MAX_MESSAGE_ID_BYTES);

    let manifest_id = protocol::manifest_id(&fixture::manifest(None));
    let body = Value::record(vec![
        ("manifest_id", Value::str(manifest_id.clone())),
        ("assignment_id", Value::str(manifest_id.clone())),
        ("ready", Value::bool(true)),
    ]);
    let message = protocol::new(
        MessageKind::Ready,
        &session_id,
        &peer_id,
        protocol::MAX_SEQUENCE,
        body,
    )
    .unwrap();
    assert_eq!(message.message_id, message_id);
    assert_eq!(
        protocol::decode(&protocol::encode(&message).unwrap())
            .unwrap()
            .message_id,
        message_id
    );

    let first = protocol::message_id("a.b", "c", 1).unwrap();
    let second = protocol::message_id("a", "b.c", 1).unwrap();
    assert_ne!(first, second);
    assert_eq!(first, "GCMI;1;3:a.b1:c1:1");
    assert_eq!(second, "GCMI;1;1:a3:b.c1:1");

    let too_long = protocol::message_id(&"s".repeat(protocol::MAX_SESSION_ID_BYTES + 1), "peer", 0);
    expect_malformed(too_long);
}

#[test]
fn omp3_online_protocol_rejects_malformed_oversized_unsupported_unknown_and_noncanonical_wires() {
    let message = &fixture::messages()[0];
    let wire = protocol::encode(message).unwrap();

    let err = protocol::decode("not-a-protocol-message").unwrap_err();
    assert_eq!(err.code, ErrorCode::Malformed);

    let err = protocol::decode(&"x".repeat(protocol::MAX_WIRE_BYTES + 1)).unwrap_err();
    assert_eq!(err.code, ErrorCode::WireTooLarge);

    let bumped = wire.replacen("GCOP;1;", "GCOP;2;", 1);
    let err = protocol::decode(&bumped).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedVersion);

    let renamed = wire.replace("s9:handshake", "s14:future_message");
    let err = protocol::decode(&renamed).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownMessage);

    let leading_zero = wire.replacen("t7:", "t07:", 1);
    let err = protocol::decode(&leading_zero).unwrap_err();
    assert_eq!(err.code, ErrorCode::Malformed);

    let mut extra = message.clone();
    let mut body = extra.body.clone();
    body.set("secret", Value::str("do-not-send"));
    extra.body = body;
    let err = protocol::new(
        extra.kind,
        &extra.session_id,
        &extra.peer_id,
        extra.sequence,
        extra.body,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Malformed);
}

#[test]
fn omp3_online_protocol_rejects_representative_raw_parser_failures() {
    let valid_wire = protocol::encode(&fixture::messages()[0]).unwrap();
    let cases: &[(&str, String)] = &[
        ("unknown value tag", "GCOP;1;x".to_string()),
        ("truncated string", "GCOP;1;s1:".to_string()),
        ("noncanonical integer", "GCOP;1;i2:01".to_string()),
        ("invalid boolean", "GCOP;1;b2".to_string()),
        (
            "duplicate table key",
            "GCOP;1;t2:s1:as1:xs1:as1:y".to_string(),
        ),
        ("trailing bytes", format!("{valid_wire}x")),
    ];
    for (name, wire) in cases {
        let err = protocol::decode(wire).unwrap_err();
        assert_eq!(err.code, ErrorCode::Malformed, "{name}");
    }
}

#[test]
fn omp3_online_protocol_rejects_malformed_bodies_for_every_control_message_kind() {
    let messages = fixture::messages();
    let mutations: &[MutationCase] = &[
        (MessageKind::Handshake, |b| {
            b.set("role", Value::str("captain"))
        }),
        (MessageKind::ManifestProposal, |b| {
            b.set("manifest_id", Value::str("not-a-hash"))
        }),
        (MessageKind::ManifestAccept, |b| {
            b.set("manifest_id", Value::str("not-a-hash"))
        }),
        (MessageKind::PeerAssignment, |b| {
            b.set("assigned_peer_id", Value::str(""))
        }),
        (MessageKind::SlotAssignment, |b| {
            b.set("assignments", Value::array(vec![]))
        }),
        (MessageKind::Ready, |b| b.set("ready", Value::str("yes"))),
        (MessageKind::Countdown, |b| {
            b.set("remaining_ticks", Value::int(-1))
        }),
        (MessageKind::Start, |b| {
            b.set("first_input_tick", Value::int(-1))
        }),
        (MessageKind::MatchPhase, |b| {
            b.set("phase", Value::str("countdown"))
        }),
        (MessageKind::HashReport, |b| {
            b.set("boundary_hash", Value::str("not-a-hash"))
        }),
        (MessageKind::ResultAck, |b| {
            b.set("final_hash", Value::str("not-a-hash"))
        }),
        (MessageKind::Abort, |b| {
            b.set("code", Value::str("freeform_abort"))
        }),
        (MessageKind::Disconnect, |b| {
            b.set("code", Value::str("freeform_disconnect"))
        }),
        (MessageKind::PairPreference, |b| {
            b.set(
                "slots",
                Value::array(vec![Value::str("home_2"), Value::str("home_1")]),
            )
        }),
        (MessageKind::PairPreferenceResult, |b| {
            b.set("status", Value::str("maybe"))
        }),
    ];
    assert_eq!(mutations.len(), messages.len());
    for (index, (kind, mutate)) in mutations.iter().enumerate() {
        let mut message = messages[index].clone();
        assert_eq!(message.kind, *kind);
        mutate(&mut message.body);
        expect_malformed(protocol::validate(&message));
    }

    for index in [11usize, 12] {
        let mut message = messages[index].clone();
        message
            .body
            .set("detail", Value::str("peer-authored prose"));
        expect_malformed(protocol::validate(&message));
    }
}

#[test]
fn omp3_online_protocol_validates_canonical_teams_slots_protected_keepers_and_bot_fills() {
    let manifest = fixture::manifest(None);
    protocol::validate_manifest(&manifest).unwrap();

    let mut keeper = manifest.clone();
    let mut teams = keeper.get("teams").unwrap().clone();
    let mut team1 = teams.get_index(1).unwrap().clone();
    let mut roster = team1.get("roster").unwrap().clone();
    let mut player1 = roster.get_index(1).unwrap().clone();
    player1.set("loadout_id", Value::str("loadout_illegal"));
    player1.set("family_id", Value::str("unarmed"));
    let mut new_roster = vec![player1];
    for index in 2..=roster.len() as i64 {
        new_roster.push(roster.get_index(index).unwrap().clone());
    }
    roster = Value::array(new_roster);
    team1.set("roster", roster);
    let mut new_teams = vec![team1];
    for index in 2..=teams.len() as i64 {
        new_teams.push(teams.get_index(index).unwrap().clone());
    }
    teams = Value::array(new_teams);
    keeper.set("teams", teams);
    expect_malformed(protocol::validate_manifest(&keeper));

    let mut reordered = manifest.clone();
    let slots = reordered.get("slots").unwrap().clone();
    let mut new_slots = vec![
        slots.get_index(2).unwrap().clone(),
        slots.get_index(1).unwrap().clone(),
    ];
    for index in 3..=slots.len() as i64 {
        new_slots.push(slots.get_index(index).unwrap().clone());
    }
    reordered.set("slots", Value::array(new_slots));
    expect_malformed(protocol::validate_manifest(&reordered));

    let assignments = fixture::assignments();
    let body = Value::record(vec![
        ("manifest_id", Value::str(protocol::manifest_id(&manifest))),
        (
            "assignment_id",
            Value::str(protocol::assignment_id(&assignments, 1)),
        ),
        ("assignments", assignments.clone()),
    ]);
    let session_id = manifest.get("session_id").unwrap().as_str().unwrap();
    let message = protocol::new(MessageKind::SlotAssignment, session_id, "host", 1, body).unwrap();
    protocol::validate(&message).unwrap();
    protocol::validate_assignment_manifest(&manifest, &assignments).unwrap();
    let a7 = assignments.get_index(7).unwrap();
    assert_eq!(a7.get("producer_kind").and_then(Value::as_str), Some("bot"));
    assert_eq!(a7.get("bot_seed").and_then(Value::as_int), Some(21007));

    let mut wrong_player = assignments.clone();
    let mut a1 = wrong_player.get_index(1).unwrap().clone();
    let manifest_slot2_player = manifest
        .get("slots")
        .unwrap()
        .get_index(2)
        .unwrap()
        .get("player_id")
        .unwrap()
        .clone();
    a1.set("player_id", manifest_slot2_player);
    let mut items = vec![a1];
    for index in 2..=wrong_player.len() as i64 {
        items.push(wrong_player.get_index(index).unwrap().clone());
    }
    wrong_player = Value::array(items);
    let err = protocol::validate_assignment_manifest(&manifest, &wrong_player).unwrap_err();
    assert_eq!(err.code, ErrorCode::IdentityMismatch);
}

#[test]
fn omp3_online_protocol_uses_the_input_frame_player_id_bound_for_every_player_id_surface() {
    let cases: &[(&str, usize, bool)] = &[
        ("minimum", 1, true),
        ("maximum", input_frame::MAX_PLAYER_ID_BYTES, true),
        ("over", input_frame::MAX_PLAYER_ID_BYTES + 1, false),
    ];
    for (name, length, accepted) in cases {
        let player_id = bounded_id("p", *length);
        let mut manifest = fixture::manifest(None);
        replace_manifest_player_id(&mut manifest, 1, 2, &player_id);
        let result = protocol::validate_manifest(&manifest);
        assert_eq!(result.is_ok(), *accepted, "roster {name}");
        if !accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "roster {name}"
            );

            let mut manifest = fixture::manifest(None);
            let mut slots = manifest.get("slots").unwrap().clone();
            let mut slot1 = slots.get_index(1).unwrap().clone();
            slot1.set("player_id", Value::str(player_id.clone()));
            let mut items = vec![slot1];
            for index in 2..=slots.len() as i64 {
                items.push(slots.get_index(index).unwrap().clone());
            }
            slots = Value::array(items);
            manifest.set("slots", slots);
            expect_malformed(protocol::validate_manifest(&manifest));
        }

        let mut assignments = fixture::assignments();
        let mut a1 = assignments.get_index(1).unwrap().clone();
        a1.set("player_id", Value::str(player_id));
        let mut items = vec![a1];
        for index in 2..=assignments.len() as i64 {
            items.push(assignments.get_index(index).unwrap().clone());
        }
        assignments = Value::array(items);
        let manifest = fixture::manifest(None);
        let manifest_id = protocol::manifest_id(&manifest);
        let body = Value::record(vec![
            ("manifest_id", Value::str(manifest_id.clone())),
            ("assignment_id", Value::str(manifest_id)),
            ("assignments", assignments),
        ]);
        let result = protocol::new(
            MessageKind::SlotAssignment,
            "session_alpha",
            "host",
            *length as i64,
            body,
        );
        assert_eq!(result.is_ok(), *accepted, "producer assignment {name}");
        if !accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "producer assignment {name}"
            );
        }
    }
}

#[test]
fn omp3_online_protocol_names_the_first_deterministic_identity_mismatch_before_countdown() {
    let expected = fixture::manifest(None);
    struct Case {
        path: &'static str,
        mutate: fn(&mut Value),
    }
    let cases = [
        Case {
            path: "manifest.build_id",
            mutate: |m| m.set("build_id", Value::str("build.other")),
        },
        Case {
            path: "manifest.teams.2.team_id",
            mutate: |m| {
                let teams = m.get("teams").unwrap().clone();
                let mut t2 = teams.get_index(2).unwrap().clone();
                t2.set("team_id", Value::str("team_other"));
                let items = vec![teams.get_index(1).unwrap().clone(), t2];
                m.set("teams", Value::array(items));
            },
        },
        Case {
            path: "manifest.teams.2.roster.3.family_id",
            mutate: |m| {
                let teams = m.get("teams").unwrap().clone();
                let mut t2 = teams.get_index(2).unwrap().clone();
                let mut roster = t2.get("roster").unwrap().clone();
                let mut p3 = roster.get_index(3).unwrap().clone();
                p3.set("family_id", Value::str("ranged"));
                let mut items = Vec::new();
                for index in 1..=roster.len() as i64 {
                    items.push(if index == 3 {
                        p3.clone()
                    } else {
                        roster.get_index(index).unwrap().clone()
                    });
                }
                roster = Value::array(items);
                t2.set("roster", roster);
                m.set(
                    "teams",
                    Value::array(vec![teams.get_index(1).unwrap().clone(), t2]),
                );
            },
        },
        Case {
            path: "manifest.slots.1.player_id",
            mutate: |m| {
                let slots = m.get("slots").unwrap().clone();
                let mut s1 = slots.get_index(1).unwrap().clone();
                let mut s2 = slots.get_index(2).unwrap().clone();
                let p1 = s1.get("player_id").unwrap().clone();
                let p2 = s2.get("player_id").unwrap().clone();
                s1.set("player_id", p2);
                s2.set("player_id", p1);
                let mut items = vec![s1, s2];
                for index in 3..=slots.len() as i64 {
                    items.push(slots.get_index(index).unwrap().clone());
                }
                m.set("slots", Value::array(items));
            },
        },
    ];
    for case in cases {
        let mut actual = expected.clone();
        (case.mutate)(&mut actual);
        let err = protocol::compare_manifest(&expected, &actual).unwrap_err();
        assert_eq!(err.code, ErrorCode::IdentityMismatch, "{}", case.path);
        assert_eq!(err.path.as_deref(), Some(case.path), "{}", case.path);
    }

    let mut actual = expected.clone();
    actual.set("presentation_id", Value::str("not-a-manifest-field"));
    expect_malformed(protocol::validate_manifest(&actual));
}

#[test]
fn omp3_online_protocol_compares_runtime_and_presentation_compatibility_outside_deterministic_identity()
 {
    let manifest = fixture::manifest(None);
    let manifest_id = protocol::manifest_id(&manifest);
    let expected = fixture::runtime();

    let mut actual = expected.clone();
    actual.set("presentation_id", Value::str("presentation.other"));
    let err = protocol::compare_runtime(&expected, &actual).unwrap_err();
    assert_eq!(err.code, ErrorCode::RuntimeMismatch);
    assert_eq!(err.path.as_deref(), Some("runtime.presentation_id"));
    assert_eq!(protocol::manifest_id(&manifest), manifest_id);

    let mut actual = expected.clone();
    let capabilities = actual.get("capabilities").unwrap().clone();
    let mut items: Vec<Value> = (1..=capabilities.len() as i64)
        .map(|i| capabilities.get_index(i).unwrap().clone())
        .collect();
    items.swap(1, 2);
    actual.set("capabilities", Value::array(items));
    expect_malformed(protocol::validate_runtime(&actual));

    let mut actual = expected.clone();
    let capabilities = actual.get("capabilities").unwrap().clone();
    let mut items: Vec<Value> = (1..=capabilities.len() as i64)
        .map(|i| capabilities.get_index(i).unwrap().clone())
        .collect();
    items[2] = Value::str("voice.v1");
    actual.set("capabilities", Value::array(items));
    let err = protocol::compare_runtime(&expected, &actual).unwrap_err();
    assert_eq!(err.code, ErrorCode::RuntimeMismatch);
    assert_eq!(err.path.as_deref(), Some("runtime.capabilities.3"));
}

#[test]
fn omp3_online_protocol_rejects_old_and_future_control_runtime_and_nested_manifest_versions() {
    let version_fields = [
        "version",
        "protocol_version",
        "input_version",
        "snapshot_version",
        "tape_version",
        "combat_schema_version",
    ];
    for field in version_fields {
        for delta in [-1i64, 1] {
            let mut manifest = fixture::manifest(None);
            let current = manifest.get(field).and_then(Value::as_int).unwrap();
            manifest.set(field, Value::int(current + delta));
            let err = protocol::validate_manifest(&manifest).unwrap_err();
            assert_eq!(err.code, ErrorCode::UnsupportedVersion, "{field} {delta}");
        }
    }

    for delta in [-1i64, 1] {
        let mut runtime = fixture::runtime();
        let current = runtime.get("version").and_then(Value::as_int).unwrap();
        runtime.set("version", Value::int(current + delta));
        let err = protocol::validate_runtime(&runtime).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedVersion, "runtime {delta}");

        let mut message = fixture::messages()[0].clone();
        message.version += delta;
        let err = protocol::validate(&message).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedVersion, "message {delta}");

        let wire = protocol::encode(&fixture::messages()[0]).unwrap();
        let bumped = wire.replacen(
            "GCOP;1;",
            &format!("GCOP;{};", protocol::VERSION + delta),
            1,
        );
        let err = protocol::decode(&bumped).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedVersion, "wire {delta}");
    }
}

#[test]
fn omp3_online_protocol_accepts_exact_scalar_bounds_and_rejects_every_over_bound_class() {
    for (name, length, accepted) in [
        ("minimum generic id", 1usize, true),
        ("maximum generic id", protocol::MAX_ID_BYTES, true),
        ("over generic id", protocol::MAX_ID_BYTES + 1, false),
    ] {
        let mut manifest = fixture::manifest(None);
        manifest.set("build_id", Value::str(bounded_id("b", length)));
        let result = protocol::validate_manifest(&manifest);
        assert_eq!(result.is_ok(), accepted, "{name}");
        if !accepted {
            assert_eq!(result.unwrap_err().code, ErrorCode::Malformed, "{name}");
        }
    }

    struct IdCase {
        name: &'static str,
        session: String,
        peer: String,
        accepted: bool,
    }
    let id_cases = [
        IdCase {
            name: "minimum session",
            session: "s".to_string(),
            peer: "peer".to_string(),
            accepted: true,
        },
        IdCase {
            name: "maximum session",
            session: bounded_id("s", protocol::MAX_SESSION_ID_BYTES),
            peer: "peer".to_string(),
            accepted: true,
        },
        IdCase {
            name: "over session",
            session: bounded_id("s", protocol::MAX_SESSION_ID_BYTES + 1),
            peer: "peer".to_string(),
            accepted: false,
        },
        IdCase {
            name: "minimum peer",
            session: "session".to_string(),
            peer: "p".to_string(),
            accepted: true,
        },
        IdCase {
            name: "maximum peer",
            session: "session".to_string(),
            peer: bounded_id("p", protocol::MAX_PEER_ID_BYTES),
            accepted: true,
        },
        IdCase {
            name: "over peer",
            session: "session".to_string(),
            peer: bounded_id("p", protocol::MAX_PEER_ID_BYTES + 1),
            accepted: false,
        },
    ];
    for case in id_cases {
        let result = protocol::message_id(&case.session, &case.peer, 0);
        assert_eq!(result.is_ok(), case.accepted, "{}", case.name);
        if !case.accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "{}",
                case.name
            );
        }
    }

    for (name, value, accepted) in [
        ("minimum sequence", 0i64, true),
        ("maximum sequence", protocol::MAX_SEQUENCE, true),
        ("over sequence", protocol::MAX_SEQUENCE + 1, false),
    ] {
        let result = protocol::message_id("session", "peer", value);
        assert_eq!(result.is_ok(), accepted, "{name}");
        if !accepted {
            assert_eq!(result.unwrap_err().code, ErrorCode::Malformed, "{name}");
        }
    }

    let manifest_integer_cases: &[(&str, &str, i64, i64)] = &[
        ("seed", "seed", 0, protocol::MAX_SEED),
        (
            "duration",
            "duration_ticks",
            1,
            protocol::MAX_DURATION_TICKS,
        ),
        ("goal limit", "max_goals", 1, protocol::MAX_GOALS),
    ];
    for (name, field, minimum, maximum) in manifest_integer_cases {
        for (boundary_name, value, accepted) in [
            ("minimum", *minimum, true),
            ("maximum", *maximum, true),
            ("over", *maximum + 1, false),
        ] {
            let mut manifest = fixture::manifest(None);
            manifest.set(field, Value::int(value));
            let result = protocol::validate_manifest(&manifest);
            assert_eq!(result.is_ok(), accepted, "{name} {boundary_name}");
            if !accepted {
                assert_eq!(
                    result.unwrap_err().code,
                    ErrorCode::Malformed,
                    "{name} {boundary_name}"
                );
            }
        }
    }

    for (name, value, accepted) in [
        ("minimum", 0i64, true),
        ("maximum", protocol::MAX_COUNTDOWN_TICKS, true),
        ("over", protocol::MAX_COUNTDOWN_TICKS + 1, false),
    ] {
        let mut message = fixture::messages()[6].clone();
        message.body.set("remaining_ticks", Value::int(value));
        let result = protocol::validate(&message);
        assert_eq!(result.is_ok(), accepted, "countdown {name}");
        if !accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "countdown {name}"
            );
        }
    }

    for (name, value, accepted) in [
        ("minimum", 0i64, true),
        ("maximum", input_frame::MAX_TICK, true),
        ("over", input_frame::MAX_TICK + 1, false),
    ] {
        let mut message = fixture::messages()[9].clone();
        message.body.set("tick", Value::int(value));
        let result = protocol::validate(&message);
        assert_eq!(result.is_ok(), accepted, "tick {name}");
        if !accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "tick {name}"
            );
        }
    }

    for (name, value, accepted) in [
        ("minimum", 0i64, true),
        ("maximum", protocol::MAX_GOALS, true),
        ("over", protocol::MAX_GOALS + 1, false),
    ] {
        let mut message = fixture::messages()[8].clone();
        message.body.set("home_score", Value::int(value));
        let result = protocol::validate(&message);
        assert_eq!(result.is_ok(), accepted, "score {name}");
        if !accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "score {name}"
            );
        }
    }

    for (name, count, accepted) in [
        ("minimum", 0i64, true),
        ("maximum", protocol::MAX_CAPABILITIES, true),
        ("over", protocol::MAX_CAPABILITIES + 1, false),
    ] {
        let mut runtime = fixture::runtime();
        let items: Vec<Value> = (1..=count)
            .map(|i| Value::str(format!("capability_{i:03}")))
            .collect();
        runtime.set("capabilities", Value::array(items));
        let result = protocol::validate_runtime(&runtime);
        assert_eq!(result.is_ok(), accepted, "capabilities {name}");
        if !accepted {
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::Malformed,
                "capabilities {name}"
            );
        }
    }

    let maximum_id = protocol::message_id(
        &bounded_id("s", protocol::MAX_SESSION_ID_BYTES),
        &bounded_id("p", protocol::MAX_PEER_ID_BYTES),
        protocol::MAX_SEQUENCE,
    )
    .unwrap();
    assert_eq!(maximum_id.len(), protocol::MAX_MESSAGE_ID_BYTES);
    let mut message = fixture::messages()[0].clone();
    message.message_id = "m".repeat(protocol::MAX_MESSAGE_ID_BYTES + 1);
    expect_malformed(protocol::validate(&message));

    let err = protocol::decode(&"x".repeat(protocol::MAX_WIRE_BYTES + 1)).unwrap_err();
    assert_eq!(err.code, ErrorCode::WireTooLarge);
}

#[test]
fn omp3_online_protocol_rejects_invalid_phase_use_before_callers_mutate_lifecycle_state() {
    let messages = fixture::messages();
    protocol::validate_phase(&messages[0], LifecyclePhase::New).unwrap();
    protocol::validate_phase(&messages[7], LifecyclePhase::Countdown).unwrap();
    protocol::validate_phase(&messages[9], LifecyclePhase::Running).unwrap();

    let err = protocol::validate_phase(&messages[7], LifecyclePhase::Manifest).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPhase);
    let err = protocol::validate_phase(&messages[9], LifecyclePhase::Terminal).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPhase);
    let err = protocol::validate_phase(&messages[11], LifecyclePhase::Terminal).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPhase);

    for match_phase in ["kickoff", "playing", "goal_stoppage", "full_time"] {
        let mut message = messages[8].clone();
        message.body.set("phase", Value::str(match_phase));
        protocol::validate_phase(&message, LifecyclePhase::Running).unwrap();
        let err = protocol::validate_phase(&message, LifecyclePhase::Result).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidPhase, "result {match_phase}");
    }

    let mut result_message = messages[8].clone();
    result_message.body.set("phase", Value::str("result"));
    protocol::validate_phase(&result_message, LifecyclePhase::Result).unwrap();
    let err = protocol::validate_phase(&result_message, LifecyclePhase::Running).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPhase);

    let err = protocol::validate_phase(&messages[6], LifecyclePhase::Result).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPhase);

    let mut invalid_match_phase = messages[8].clone();
    invalid_match_phase
        .body
        .set("phase", Value::str("countdown"));
    let err = protocol::validate_phase(&invalid_match_phase, LifecyclePhase::Result).unwrap_err();
    assert_eq!(err.code, ErrorCode::Malformed);
}

#[test]
fn omp3_online_protocol_makes_exact_duplicates_idempotent_and_conflicting_reuse_terminal() {
    let previous = fixture::messages()[5].clone();
    let duplicate = protocol::decode(&protocol::encode(&previous).unwrap()).unwrap();
    assert_eq!(
        protocol::classify_duplicate(&previous, &duplicate).unwrap(),
        protocol::DuplicateDisposition::Idempotent
    );

    let mut conflict = previous.clone();
    conflict.body.set("ready", Value::bool(false));
    let err = protocol::classify_duplicate(&previous, &conflict).unwrap_err();
    assert_eq!(err.code, ErrorCode::TranscriptConflict);

    let other = fixture::messages()[6].clone();
    let err = protocol::classify_duplicate(&previous, &other).unwrap_err();
    assert_eq!(err.code, ErrorCode::Duplicate);
}

#[test]
fn omp3_online_protocol_derives_replay_safe_transcript_identity_from_canonical_ordered_messages() {
    let mut messages = fixture::messages();
    let first = protocol::transcript_id(&messages);
    let second = protocol::transcript_id(&fixture::messages());
    assert_eq!(first, second);
    assert_eq!(first.len(), 16);

    messages[9].body.set("tick", Value::int(61));
    assert_ne!(protocol::transcript_id(&messages), first);
}

// ---------------------------------------------------------------------------
// describe "handshake build declaration"
// ---------------------------------------------------------------------------

fn handshake(build_id: Option<&str>) -> protocol::Result<protocol::ControlMessage> {
    let mut fields = vec![
        ("role", Value::str("guest")),
        ("runtime", fixture::runtime()),
    ];
    fields.push(("build_id", build_id.map_or(Value::Nil, Value::str)));
    protocol::new(
        MessageKind::Handshake,
        "session_alpha",
        "guest_1",
        0,
        Value::record(fields),
    )
}

#[test]
fn handshake_build_declaration_carries_a_declared_build_through_the_canonical_codec() {
    let message = handshake(Some("build.abc123")).unwrap();
    let wire = protocol::encode(&message).unwrap();
    let decoded = protocol::decode(&wire).unwrap();
    assert_eq!(
        decoded.body.get("build_id").and_then(Value::as_str),
        Some("build.abc123")
    );
    assert_eq!(protocol::encode(&decoded).unwrap(), wire);
}

#[test]
fn handshake_build_declaration_accepts_a_handshake_that_declares_no_build_at_all() {
    let message = handshake(None).unwrap();
    assert!(message.body.get("build_id").is_none_or(Value::is_nil));
    let decoded = protocol::decode(&protocol::encode(&message).unwrap()).unwrap();
    assert!(decoded.body.get("build_id").is_none_or(Value::is_nil));
}

#[test]
fn handshake_build_declaration_refuses_a_build_declaration_that_is_not_an_opaque_bounded_id() {
    let bad_ids = [
        String::new(),
        "build id with spaces".to_string(),
        "b".repeat(protocol::MAX_ID_BYTES + 1),
    ];
    for bad in bad_ids {
        let err = handshake(Some(&bad)).unwrap_err();
        assert_eq!(err.code, ErrorCode::Malformed, "{bad}");
    }
    // The reference test suite also exercised non-string build declarations
    // (`17`, `true`) — structurally impossible here: `handshake`'s `build_id`
    // parameter is `Option<&str>`, so a non-string value cannot be
    // constructed at all. The malformed-type outcome those cases proved is
    // instead guaranteed by the type system, which is strictly stronger.
}

#[test]
fn handshake_build_declaration_counts_the_declaration_as_part_of_the_vocabulary_this_build_speaks()
{
    let kinds: protocol::Vocabulary = vec![
        (
            "handshake".to_string(),
            vec!["role".to_string(), "runtime".to_string()],
        ),
        ("abort".to_string(), vec!["code".to_string()]),
    ];
    let phases: protocol::Vocabulary = vec![
        (
            "handshake".to_string(),
            vec!["new".to_string(), "handshake".to_string()],
        ),
        ("abort".to_string(), vec!["new".to_string()]),
    ];
    let without = protocol::vocabulary_digest(&kinds, &phases);
    let mut with_build_id = kinds.clone();
    with_build_id[0].1.push("build_id".to_string());
    assert_ne!(
        protocol::vocabulary_digest(&with_build_id, &phases),
        without
    );
}

// ---------------------------------------------------------------------------
// Differential tests against the pinned reference vectors captured from the
// implementation this netcode's wire behaviour was validated against
// (`tests/fixtures/protocol_lua_reference.txt`; see
// `tools/lua_reference/README.md` for capture provenance). These assert
// exact bytes, not merely that Rust's own encode/decode round-trip through
// themselves.
//
// #489, schema-coupled, same root cause and same PR as
// `coordinator_conformance::Golden`, `protocol_conformance::GOLDEN`, and
// `desync_package_identity_vector.txt`'s `manifest_id`/`snapshot_version`
// (see those constants' doc comments): `match_snapshot::COMBAT_VERSION`
// bumps 13 -> 14, and every value below derived from `manifest_id` moves
// with it. `HANDSHAKE_WIRE` and `VOCAB_ID` carry no manifest content and
// still read the frozen fixture unmodified — only `MIN_WIRE(_LEN/_HASH)`,
// `MAXIMAL_MANIFEST_ID`, `MAXIMAL_WIRE(_LEN/_HASH)`, `MANIFEST_ID` and
// `TRANSCRIPT_ID` are retired to the `*_BASELINE_489` constants below,
// following `tools/lua_reference/README.md`'s partial-retirement procedure
// (as used for `match_snapshot_case_a/b_lua_reference.txt` et al. under
// #536). The frozen fixture file itself, and every other assertion reading
// it, are unmodified. Recorded the same way as those: reading each
// assertion's own failure output (`MIN_WIRE`/`MAXIMAL_WIRE` via a temporary
// `eprintln!` added, run, then removed, since both need intermediate values
// a panic alone would not surface past the first divergence).
// #490 RE-RECORDS THE SAME SET, and the `_489` suffixes stay as they are: they
// name the PR that RETIRED these values from the frozen Lua fixture, not the
// last one to re-derive them. `match_snapshot::COMBAT_VERSION` bumps 14 -> 15
// (`MatchPlayer::keeper_fatigue`), so `manifest_id` and every value derived
// from it moves exactly as it did under #489, by exactly the same mechanism,
// and nothing outside that set moves. Re-derived the same way: a throwaway
// probe calling `protocol::manifest_id`/`transcript_id`/`encode` against the
// same fixture, plus temporary `eprintln!`s for the two intermediate wires.
// ---------------------------------------------------------------------------

/// See the module-section doc comment above.
const MIN_WIRE_BASELINE_489: &str = "GCOP;1;t7:s4:bodyt1:s11:manifest_ids16:90b90970080d7978s4:kinds15:\
manifest_accepts10:message_ids16:GCMI;1;1:s1:p1:0s7:peer_ids1:ps8:sequencei1:0s10:session_ids1:ss7:versioni1:1";
/// See the module-section doc comment above. Unchanged from the frozen
/// fixture's `MIN_WIRE_LEN` — the new and old manifest ids are both 16-byte
/// hex hashes, so wire length does not move even though the bytes do.
const MIN_WIRE_LEN_BASELINE_489: usize = 176;
/// See the module-section doc comment above.
const MIN_WIRE_HASH_BASELINE_489: &str = "b398e49a73452b3b";
/// See the module-section doc comment above.
const MAXIMAL_MANIFEST_ID_BASELINE_489: &str = "9fbc7b2c02f50b0b";
/// See the module-section doc comment above. Unchanged from the frozen
/// fixture's `MAXIMAL_WIRE_LEN`, same reasoning as `MIN_WIRE_LEN_BASELINE_489`.
const MAXIMAL_WIRE_LEN_BASELINE_489: usize = 7240;
/// See the module-section doc comment above.
const MAXIMAL_WIRE_HASH_BASELINE_489: &str = "1f436d15827652d2";
/// See the module-section doc comment above.
const MANIFEST_ID_BASELINE_489: &str = "90b90970080d7978";
/// See the module-section doc comment above.
const TRANSCRIPT_ID_BASELINE_489: &str = "1b8407df3614a2cb";

/// #612 (repository owner's call, tracked issue): retires `VOCAB_ID`'s
/// comparison against `protocol_lua_reference.txt` under
/// `tools/lua_reference/README.md` §2's procedure. Superseding change:
/// `Start`'s allowed phases widen from `[Countdown]` to
/// `[Countdown, Running]` (`protocol::allowed_phases`) so the coordinator can
/// resend the canonical start boundary and accept a duplicate echo of it as
/// a no-op instead of a protocol violation — the original Lua implementation
/// had no resend at all (a one-shot, 2-second handshake with no recovery),
/// so this is a deliberate capability past what the frozen fixture can ever
/// describe, not a divergence to chase back into agreement. `MANIFEST_ID`
/// and `TRANSCRIPT_ID` above are unaffected — they, and the fixture's other
/// entries, do not read the phase table — so only this one value moves to a
/// self-recorded baseline; everything else in this file keeps reading
/// `protocol_lua_reference.txt` unmodified. Last commit the fixture
/// comparison held: `b7ab896` (this branch's own merge base, verified
/// green). Recorded by calling `protocol::vocabulary_id()` directly against
/// this build — see `protocol_conformance::GOLDEN`'s doc comment for the
/// same value pinned the same way.
const VOCAB_ID_BASELINE_612: &str = "93f9c16ad1674b97";

#[test]
fn differential_handshake_wire_matches_the_real_lua_byte_for_byte() {
    let message = &fixture::messages()[0];
    let wire = protocol::encode(message).unwrap();
    assert_eq!(wire, lua_ref("HANDSHAKE_WIRE"));
    assert_eq!(
        wire.len(),
        lua_ref("HANDSHAKE_WIRE_LEN").parse::<usize>().unwrap()
    );
    assert_eq!(
        gc_core::fnv1a64::hash(wire.as_bytes()),
        lua_ref("HANDSHAKE_WIRE_HASH")
    );
}

#[test]
fn differential_minimum_size_message_matches_the_real_lua_byte_for_byte() {
    let manifest = fixture::manifest(None);
    let manifest_id = protocol::manifest_id(&manifest);
    let body = Value::record(vec![("manifest_id", Value::str(manifest_id))]);
    let message = protocol::new(MessageKind::ManifestAccept, "s", "p", 0, body).unwrap();
    let wire = protocol::encode(&message).unwrap();
    assert_eq!(wire, MIN_WIRE_BASELINE_489);
    assert_eq!(wire.len(), MIN_WIRE_LEN_BASELINE_489);
    assert_eq!(
        gc_core::fnv1a64::hash(wire.as_bytes()),
        MIN_WIRE_HASH_BASELINE_489
    );
}

/// The maximum-size payload this protocol allows: every applicable field is
/// bounded_id'd out to its maximum, matching the reference test suite's
/// "keeps an all-applicable-max proposal within the 8 KiB record bound",
/// but checked against the pinned reference bytes rather than only a
/// length.
#[test]
fn differential_maximum_size_payload_matches_the_real_lua_byte_for_byte() {
    let mut manifest = fixture::manifest(None);
    manifest.set(
        "session_id",
        Value::str(bounded_id("session", protocol::MAX_SESSION_ID_BYTES)),
    );
    manifest.set("combat_status", Value::str("accepted_revision"));
    manifest.set("seed", Value::int(protocol::MAX_SEED));
    manifest.set("duration_ticks", Value::int(protocol::MAX_DURATION_TICKS));
    manifest.set("max_goals", Value::int(protocol::MAX_GOALS));
    for field in [
        "build_id",
        "source_id",
        "content_id",
        "tuning_id",
        "match_config_id",
        "fixture_id",
        "arena_id",
        "combat_rules_id",
        "gameplay_ai_policy_id",
    ] {
        manifest.set(
            field,
            Value::str(bounded_id(&field[..1], protocol::MAX_ID_BYTES)),
        );
    }
    let mut teams = manifest.get("teams").unwrap().clone();
    let mut t1 = teams.get_index(1).unwrap().clone();
    t1.set(
        "team_id",
        Value::str(bounded_id("home", protocol::MAX_ID_BYTES)),
    );
    let mut t2 = teams.get_index(2).unwrap().clone();
    t2.set(
        "team_id",
        Value::str(bounded_id("away", protocol::MAX_ID_BYTES)),
    );
    teams = Value::array(vec![t1, t2]);
    manifest.set("teams", teams);

    for team_index in 1..=2i64 {
        let teams = manifest.get("teams").unwrap().clone();
        let mut team = teams.get_index(team_index).unwrap().clone();
        let roster = team.get("roster").unwrap().clone();
        let mut new_roster = Vec::new();
        for roster_index in 1..=roster.len() as i64 {
            let mut player = roster.get_index(roster_index).unwrap().clone();
            let player_id = bounded_id(
                &format!("p{team_index}{roster_index}"),
                input_frame::MAX_PLAYER_ID_BYTES,
            );
            let is_keeper = player.get("position").and_then(Value::as_str) == Some("keeper");
            player.set("player_id", Value::str(player_id));
            if !is_keeper {
                player.set("position", Value::str("midfielder"));
                player.set(
                    "loadout_id",
                    Value::str(bounded_id(
                        &format!("l{team_index}{roster_index}"),
                        protocol::MAX_ID_BYTES,
                    )),
                );
                player.set(
                    "family_id",
                    Value::str(bounded_id(
                        &format!("f{team_index}{roster_index}"),
                        protocol::MAX_ID_BYTES,
                    )),
                );
            }
            new_roster.push(player);
        }
        team.set("roster", Value::array(new_roster));
        let mut items = Vec::new();
        for index in 1..=teams.len() as i64 {
            items.push(if index == team_index {
                team.clone()
            } else {
                teams.get_index(index).unwrap().clone()
            });
        }
        manifest.set("teams", Value::array(items));
    }
    // Player ids changed; re-point every canonical slot at its (renamed)
    // outfielder the same way `replace_manifest_player_id` does, driven off
    // the fixture's original slot -> player mapping (positions are stable).
    let original = fixture::manifest(None);
    let original_slots = original.get("slots").unwrap().clone();
    let mut new_slots = Vec::new();
    for index in 1..=original_slots.len() as i64 {
        let original_slot = original_slots.get_index(index).unwrap();
        let team_index = if original_slot.get("team").and_then(Value::as_str) == Some("home") {
            1
        } else {
            2
        };
        let original_player_id = original_slot
            .get("player_id")
            .and_then(Value::as_str)
            .unwrap();
        let team_roster_original = original
            .get("teams")
            .unwrap()
            .get_index(team_index)
            .unwrap()
            .get("roster")
            .unwrap();
        let roster_index = (1..=team_roster_original.len() as i64)
            .find(|&i| {
                team_roster_original
                    .get_index(i)
                    .unwrap()
                    .get("player_id")
                    .and_then(Value::as_str)
                    == Some(original_player_id)
            })
            .unwrap();
        let new_player_id = bounded_id(
            &format!("p{team_index}{roster_index}"),
            input_frame::MAX_PLAYER_ID_BYTES,
        );
        let mut slot = original_slot.clone();
        slot.set("player_id", Value::str(new_player_id));
        new_slots.push(slot);
    }
    manifest.set("slots", Value::array(new_slots));

    protocol::validate_manifest(&manifest).expect("maximal manifest must validate");
    let manifest_id = protocol::manifest_id(&manifest);
    assert_eq!(manifest_id, MAXIMAL_MANIFEST_ID_BASELINE_489);

    let body = Value::record(vec![
        ("manifest_id", Value::str(manifest_id.clone())),
        ("manifest", manifest.clone()),
    ]);
    let session_id = manifest
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let peer_id = bounded_id("peer", protocol::MAX_PEER_ID_BYTES);
    let message = protocol::new(
        MessageKind::ManifestProposal,
        &session_id,
        &peer_id,
        protocol::MAX_SEQUENCE,
        body,
    )
    .unwrap();
    let wire = protocol::encode(&message).unwrap();
    assert!(wire.len() <= protocol::MAX_WIRE_BYTES);
    assert_eq!(wire.len(), MAXIMAL_WIRE_LEN_BASELINE_489);
    assert_eq!(
        gc_core::fnv1a64::hash(wire.as_bytes()),
        MAXIMAL_WIRE_HASH_BASELINE_489
    );
}

#[test]
fn differential_vocabulary_manifest_and_transcript_ids_match_the_real_lua() {
    // `vocabulary_id` no longer reads `lua_ref` — see `VOCAB_ID_BASELINE_612`'s
    // doc comment for the retirement. This still proves the value is stable
    // and matches the golden pinned elsewhere; it no longer proves agreement
    // with the (superseded, gone) Lua implementation's vocabulary.
    assert_eq!(protocol::vocabulary_id(), VOCAB_ID_BASELINE_612);
    assert_eq!(
        protocol::manifest_id(&fixture::manifest(None)),
        MANIFEST_ID_BASELINE_489
    );
    assert_eq!(
        protocol::transcript_id(&fixture::messages()),
        TRANSCRIPT_ID_BASELINE_489
    );
}

/// Every non-canonical mutation applied directly to the pinned reference
/// bytes (`HANDSHAKE_WIRE`), not to a Rust re-encode — the same base wire
/// the reference test suite's own `gsub` mutations operated on.
#[test]
fn differential_non_canonical_mutations_of_the_real_lua_wire_are_rejected() {
    let wire = lua_ref("HANDSHAKE_WIRE");
    assert_eq!(
        protocol::encode(&fixture::messages()[0]).unwrap(),
        wire,
        "sanity: Rust reproduces the pinned Lua wire"
    );

    let bumped = wire.replacen("GCOP;1;", "GCOP;2;", 1);
    assert_eq!(
        protocol::decode(&bumped).unwrap_err().code,
        ErrorCode::UnsupportedVersion
    );

    let renamed = wire.replace("s9:handshake", "s14:future_message");
    assert_eq!(
        protocol::decode(&renamed).unwrap_err().code,
        ErrorCode::UnknownMessage
    );

    let leading_zero = wire.replacen("t7:", "t07:", 1);
    assert_eq!(
        protocol::decode(&leading_zero).unwrap_err().code,
        ErrorCode::Malformed
    );

    let truncated = &wire[..wire.len() - 10];
    assert_eq!(
        protocol::decode(truncated).unwrap_err().code,
        ErrorCode::Malformed
    );

    let oversize = format!("{wire}{}", "x".repeat(protocol::MAX_WIRE_BYTES));
    let err = protocol::decode(&oversize).unwrap_err();
    assert_eq!(err.code, ErrorCode::WireTooLarge);

    let trailing = format!("{wire}x");
    assert_eq!(
        protocol::decode(&trailing).unwrap_err().code,
        ErrorCode::Malformed
    );
}

// ---------------------------------------------------------------------------
// The relay-topology probe scenario ("relay topology probe: no peer is the
// sequencer", originally the second `describe` block of the transport relay
// test suite), deferred to this crate from the TypeScript port (per this
// agent's brief). `coordinator.rs` and `match_driver.rs` landed first, so
// the two findings that never touched `game.transport.fake_relay` were
// ported for real early. `gc_netcode::fake_relay` (a Rust implementation of
// the fake relay transport) and a `relay` topology option on
// `gc_netcode::fault_harness::FaultHarnessOptions` have since landed too,
// so the remaining four findings below are now real, running tests rather
// than `#[ignore]`d stubs.
// ---------------------------------------------------------------------------

/// The 8 peer ids an 8-human `4v4` fixture manifest seats: the host plus
/// seven guests, admission order.
fn eight_peer_ids() -> Vec<String> {
    let mut peer_ids = vec!["host".to_string()];
    for index in 1..=7 {
        peer_ids.push(format!("guest_{index}"));
    }
    peer_ids
}

/// `coordinator::plan_assignments`'s wire-shaped `Value`, converted into
/// `match_driver::SlotAssignment`, one per canonical slot. Reads the
/// assignment `Value` through its public field accessors rather than
/// `coordinator`'s own (`pub(crate)`) `assignment_at`/`producer_kind`/
/// `producer_id` helpers, which this integration-test crate cannot reach.
fn slot_assignments(assignments_value: &Value) -> [SlotAssignment; 8] {
    std::array::from_fn(|i| {
        let producer = assignments_value
            .get_index(i as i64 + 1)
            .expect("eight canonical slots are always assigned");
        let kind = producer
            .get("producer_kind")
            .and_then(Value::as_str)
            .expect("every assignment names a producer kind");
        SlotAssignment {
            producer_kind: if kind == "peer" {
                ProducerKind::Peer
            } else {
                ProducerKind::Bot
            },
            producer_id: producer
                .get("producer_id")
                .and_then(Value::as_str)
                .expect("every assignment names a producer id")
                .to_string(),
            bot_seed: producer.get("bot_seed").and_then(Value::as_int),
        }
    })
}

/// Finding 2. `input_protocol.canonical_host_batch`'s ownership check is
/// bound to the *transport's* attributed origin, not merely to the packet's
/// self-declared `sender_id` — the property a relay's per-line origin
/// tagging depends on staying true (see the original test suite's own
/// comment: "a relay that concatenated blobs without keeping origin ...
/// takes this check with it"). `input_protocol.canonical_host_batch` itself
/// was never carried over as a free function (needs the protocol module's
/// `SessionManifest`/`SessionSlotProducer`, see `input_protocol.rs`'s module
/// doc), but the real equivalent lives on `match_driver_fixture::DriverRules`
/// — driven directly here, exactly as the original test suite drives
/// `input_protocol.canonical_host_batch` directly rather than through a live
/// driver.
#[test]
fn transport_relay_topology_probe_keeps_ownership_validation_bound_to_the_transport_origin() {
    let manifest = fixture::manifest(Some(MatchMode::FourVFour));
    let peer_ids = eight_peer_ids();
    let assignments_value = coordinator::plan_assignments(&manifest, &peer_ids)
        .expect("an 8-human 4v4 manifest plans without bot fills");
    let assignments = slot_assignments(&assignments_value);
    let slot_index = 2i64;
    let producer = assignments[(slot_index - 1) as usize].producer_id.clone();

    let mut rows = Vec::new();
    for tick in 0..=input_protocol::HISTORY_ROWS {
        rows.push(input_protocol::AuthorityRow {
            tick,
            slot_index,
            sample: input_frame::neutral_sample(),
        });
    }
    let session_id = manifest
        .get("session_id")
        .and_then(Value::as_str)
        .expect("fixture manifest has a session id")
        .to_string();
    let manifest_id = protocol::manifest_id(&manifest);
    let packet = input_protocol::new_guest(input_protocol::PacketOptions {
        session_id: session_id.clone(),
        manifest_id: manifest_id.clone(),
        sender_id: producer.clone(),
        sequence: 1,
        transport_tick: 0,
        first_input_tick: 0,
        confirmed_span: None,
        rows,
    })
    .expect("a well-formed guest packet");
    let wire = input_protocol::encode(&packet).expect("a well-formed packet encodes");
    let envelope = TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq: packet.sequence,
        tick: Some(packet.transport_tick),
        payload: wire,
    };

    let driver_manifest = DriverSessionManifest {
        session_id: session_id.clone(),
    };
    let freeze = coordinator::Freeze {
        manifest_id: manifest_id.clone(),
        assignment_id: "assignment.probe".to_string(),
        countdown_id: "countdown.1".to_string(),
        first_input_tick: 0,
        seed: manifest.get("seed").and_then(Value::as_int).unwrap(),
        tick_rate: manifest.get("tick_rate").and_then(Value::as_int).unwrap(),
        duration_ticks: manifest
            .get("duration_ticks")
            .and_then(Value::as_int)
            .unwrap(),
        max_goals: manifest.get("max_goals").and_then(Value::as_int).unwrap(),
        content_id: manifest
            .get("content_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        tuning_id: manifest
            .get("tuning_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        combat_rules_id: manifest
            .get("combat_rules_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        gameplay_ai_policy_id: manifest
            .get("gameplay_ai_policy_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        combat_status: manifest
            .get("combat_status")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        match_mode: MatchMode::FourVFour,
        assignments: assignments_value.clone(),
        owned: indexmap::IndexMap::new(),
        live: indexmap::IndexMap::new(),
    };
    let rules = DriverRules::new(manifest.clone(), freeze);

    let batch_with_origin = |transport_peer_id: &str| -> Option<HostBatchErrorCode> {
        let arrival = InputPacketArrival {
            packet: packet.clone(),
            envelope: envelope.clone(),
            arrival_tick: 0,
            transport_peer_id: transport_peer_id.to_string(),
        };
        let request = HostBatchRequest {
            manifest: &driver_manifest,
            assignments: &assignments,
            host_peer_id: "host",
            sequence: 1,
            transport_tick: 0,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: std::slice::from_ref(&arrival),
        };
        rules
            .canonical_host_batch(request)
            .err()
            .map(|err| err.code)
    };

    assert_eq!(
        batch_with_origin(&producer),
        None,
        "the true origin canonicalises"
    );
    assert_eq!(
        batch_with_origin("guest_7"),
        Some(HostBatchErrorCode::OwnershipMismatch),
        "a lost origin is an ownership violation, not a silent accept"
    );
}

/// Finding 4. The session lifecycle is host-authoritative in `coordinator`,
/// independently of the wire — moving input distribution to a relay does not
/// touch any of these role checks, so this exercises `coordinator::step`
/// directly exactly as the original test suite does, with no transport at
/// all.
#[test]
fn transport_relay_topology_probe_keeps_the_session_lifecycle_host_authoritative() {
    let manifest = coordinator_fixture::manifest(Some(MatchMode::TwoVTwo));
    let state = coordinator_fixture::guest(1, None, None);

    let (_, proposed) = coordinator::step(
        &state,
        Event::ProposeManifest {
            manifest: manifest.clone(),
        },
    );
    assert!(!proposed.accepted);
    assert_eq!(
        proposed.reason.as_deref(),
        Some("only the host proposes the session manifest")
    );

    let (_, assigned) = coordinator::step(
        &state,
        Event::AssignSlots {
            assignments: Value::array(Vec::new()),
            preserve_claims: false,
        },
    );
    assert_eq!(
        assigned.reason.as_deref(),
        Some("only the host publishes slot assignments")
    );

    let (_, counted) = coordinator::step(
        &state,
        Event::BeginCountdown {
            countdown_id: "countdown.1".to_string(),
            remaining_ticks: 2,
            first_input_tick: 0,
        },
    );
    assert_eq!(
        counted.reason.as_deref(),
        Some("only the host starts the countdown")
    );
}

/// Finding 1. `match_driver`'s guest authority path accepts host batches and
/// nothing else, so a client that receives another client's own bundle —
/// which is all a framing relay can ever deliver — kills the match. Drives
/// `fault_harness::FaultHarness::new` with `topology:
/// Some(FaultHarnessTopology::Relay)`, exactly as the original test suite's
/// analogous case drives `fault_harness.new({ topology = "relay", ... })`.
#[test]
fn transport_relay_topology_probe_terminates_a_guest_that_receives_a_peers_own_bundle() {
    let mut harness = FaultHarness::new(FaultHarnessOptions {
        topology: Some(FaultHarnessTopology::Relay),
        mode: Some(MatchMode::TwoVTwo),
        profile: Some(NetworkProfileName::Clean),
        duration_ticks: Some(60),
        ..Default::default()
    });
    assert!(
        harness.reach_start(None, None),
        "the relay harness never reached start"
    );
    harness.start_match();
    for _ in 0..12 {
        harness.advance();
    }

    let sender_peer_id = harness.client(2).peer_id.clone();
    let target_peer_id = harness.client(3).peer_id.clone();
    let freeze = harness
        .client(2)
        .coordinator
        .freeze
        .clone()
        .expect("the sender has frozen a session");
    let owned_slot = freeze
        .owned
        .get(&sender_peer_id)
        .expect("the sender owns at least one slot")[0];
    let slot_index = live_slot::slot_index(owned_slot);

    let mut rows = Vec::new();
    for tick in 0..=input_protocol::HISTORY_ROWS {
        rows.push(input_protocol::AuthorityRow {
            tick: freeze.first_input_tick + tick,
            slot_index,
            sample: input_frame::neutral_sample(),
        });
    }
    let packet = input_protocol::new_guest(input_protocol::PacketOptions {
        session_id: harness
            .manifest
            .get("session_id")
            .and_then(Value::as_str)
            .expect("the harness manifest has a session id")
            .to_string(),
        manifest_id: freeze.manifest_id.clone(),
        sender_id: sender_peer_id.clone(),
        sequence: 9100,
        transport_tick: harness.step + match_driver::DELAY_TICKS,
        first_input_tick: freeze.first_input_tick,
        confirmed_span: None,
        rows,
    })
    .expect("a well-formed guest packet");
    let wire = input_protocol::encode(&packet).expect("a well-formed packet encodes");
    let envelope = TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq: packet.sequence,
        tick: Some(packet.transport_tick),
        payload: wire,
    };
    // The relay accepts it. That is the point: the wire imposes nothing.
    assert!(
        harness
            .client(2)
            .send_raw(&target_peer_id, TransportChannel::Input, envelope)
            .expect("the relay accepts an addressed send between two non-host members")
    );

    for _ in 0..6 {
        harness.advance();
    }

    let target_driver = harness
        .client(3)
        .driver
        .as_ref()
        .expect("the target has a driver");
    let terminal = match_driver::terminal(target_driver).expect("the target went terminal");
    assert_eq!(terminal.status, MatchDriverStatus::OwnershipViolation);
    assert_eq!(
        terminal.detail,
        "a guest received authority that was not a host batch"
    );
    harness.teardown();
}

/// Finding 3. Declared bot fills are authored by the host and by nobody else
/// (`match_driver::new`: `role == Host && producer.producer_kind == Bot`).
/// Remove the host and those slots have no author at all.
#[test]
fn transport_relay_topology_probe_gives_declared_bot_fills_no_author_but_the_host() {
    let mut harness = FaultHarness::new(FaultHarnessOptions {
        topology: Some(FaultHarnessTopology::Relay),
        mode: Some(MatchMode::FourVFour),
        humans: Some(4),
        profile: Some(NetworkProfileName::Clean),
        duration_ticks: Some(60),
        ..Default::default()
    });
    assert!(
        harness.reach_start(None, None),
        "the relay harness never reached start"
    );
    harness.start_match();
    let host = match_driver::diagnostics(
        harness
            .client(1)
            .driver
            .as_ref()
            .expect("the host has a driver"),
    );
    let guest = match_driver::diagnostics(
        harness
            .client(2)
            .driver
            .as_ref()
            .expect("the guest has a driver"),
    );
    assert_eq!(host.owned.len(), 1, "the host owns one slot");
    assert_eq!(
        host.authored.len(),
        5,
        "and authors its own plus all four bot fills"
    );
    assert_eq!(guest.owned.len(), 1);
    assert_eq!(guest.authored.len(), 1, "a guest authors only what it owns");
    harness.teardown();
}

/// Finding 5. The settle phase's host relay wait is host-only by
/// construction. It exists because a player-host that stops relaying strands
/// everyone else's tail; a relay that is not a player cannot leave, so this
/// is one piece of complexity the topology genuinely deletes. See the
/// original test suite's own comment (`#243`/`#255`): guests report their
/// own confirmation in the bundles they already re-publish, so under clean
/// delivery the host leaves within two settle steps rather than the
/// four-plus a quiet-count heuristic used to cost.
#[test]
fn transport_relay_topology_probe_scopes_the_settle_relay_wait_to_the_host_alone() {
    let mut harness = FaultHarness::new(FaultHarnessOptions {
        topology: Some(FaultHarnessTopology::Relay),
        mode: Some(MatchMode::OneVOne),
        profile: Some(NetworkProfileName::Clean),
        duration_ticks: Some(40),
        ..Default::default()
    });
    assert!(
        harness.reach_start(None, None),
        "the relay harness never reached start"
    );
    harness.start_match();
    for _ in 0..200 {
        harness.advance();
        if harness.finished() {
            break;
        }
    }
    let host_driver = harness
        .client(1)
        .driver
        .as_ref()
        .expect("the host has a driver");
    let guest_driver = harness
        .client(2)
        .driver
        .as_ref()
        .expect("the guest has a driver");
    let host = match_driver::diagnostics(host_driver);
    let guest = match_driver::diagnostics(guest_driver);
    assert_eq!(
        match_driver::status(host_driver),
        MatchDriverStatus::Completed
    );
    assert_eq!(
        match_driver::status(guest_driver),
        MatchDriverStatus::Completed
    );
    assert!(
        host.settle_steps <= 2,
        "the host settled slowly on a clean 1v1: {} steps",
        host.settle_steps
    );
    assert!(
        guest.settle_steps <= host.settle_steps,
        "a guest cannot outlast the relay it depends on"
    );
    harness.teardown();
}

/// Finding 6. The per-node wire cost of the shape the decision actually
/// proposes: every member publishes only its own bundle and receives the
/// other seven, concatenated into one frame. Drives
/// `gc_netcode::fake_relay::FakeRelayTransport` directly (mirrors the
/// original test suite's own `build_room`/`broadcast`/`wire_counters`), not
/// through `fault_harness`: this finding measures the adapter's own byte
/// accounting, which does not need a running match.
#[test]
fn transport_relay_topology_probe_measures_sequencer_less_per_node_wire_cost() {
    use gc_netcode::fake_relay::{FakeRelayTransport, FakeRelayTransportOptions};
    use gc_netcode::fault_transport::{StarTransportAdapter, TransportRole};

    fn member_id(index: i64) -> String {
        if index == 1 {
            "host".to_string()
        } else {
            format!("guest_{}", index - 1)
        }
    }

    fn guest_bundle(peer_id: &str, slot_index: i64, transport_tick: i64) -> TransportMessage {
        let mut rows = Vec::new();
        for tick in 0..=input_protocol::HISTORY_ROWS {
            rows.push(input_protocol::AuthorityRow {
                tick: transport_tick + tick,
                slot_index,
                sample: input_frame::neutral_sample(),
            });
        }
        let packet = input_protocol::new_guest(input_protocol::PacketOptions {
            session_id: "session_relay_probe01".to_string(),
            manifest_id: "a".repeat(16),
            sender_id: peer_id.to_string(),
            sequence: transport_tick,
            transport_tick,
            first_input_tick: 0,
            confirmed_span: None,
            rows,
        })
        .expect("a well-formed guest packet");
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq: packet.sequence,
            tick: Some(packet.transport_tick),
            payload: input_protocol::encode(&packet).expect("a well-formed packet encodes"),
        }
    }

    let room = FakeRelayTransport::new_room();
    let mut members: Vec<FakeRelayTransport> = Vec::new();
    for index in 1..=8 {
        let mut endpoint = FakeRelayTransport::new(FakeRelayTransportOptions {
            role: if index == 1 {
                TransportRole::Host
            } else {
                TransportRole::Guest
            },
            peer_id: Some(member_id(index)),
            room: Some(room.clone()),
            ..Default::default()
        });
        endpoint.initialize().expect("member initializes");
        members.push(endpoint);
    }

    let ticks = 60i64;
    for tick in 1..=ticks {
        for (offset, endpoint) in members.iter_mut().enumerate() {
            let index = offset as i64 + 1;
            endpoint
                .broadcast(
                    TransportChannel::Input,
                    guest_bundle(&member_id(index), index, tick),
                )
                .expect("a connected member's broadcast is accepted");
        }
        members[0].pump();
        for endpoint in members.iter_mut() {
            endpoint.poll_batch(Some(256));
        }
    }

    for (offset, endpoint) in members.iter().enumerate() {
        let index = offset as i64 + 1;
        let counters = endpoint.wire_counters();
        let up = counters.input_uplink_bytes as f64 / ticks as f64;
        let down = counters.input_downlink_bytes as f64 / ticks as f64;
        let framed = counters.downlink_framed_bytes as f64 / ticks as f64;
        assert_eq!(
            counters.uplink_units,
            ticks,
            "{} uploads once per tick",
            member_id(index)
        );
        assert_eq!(
            counters.downlink_frames, ticks,
            "and receives one framed message per tick"
        );
        // One own bundle up; seven other bundles down. The uplink is an
        // order of magnitude under the decision's predicted 1,190 B/tick,
        // and the downlink is roughly double its predicted ~650 B/tick.
        assert!(up > 180.0 && up < 200.0, "uplink {up:.1} B/tick");
        assert!(down > 1300.0 && down < 1400.0, "downlink {down:.1} B/tick");
        assert!(
            (down / up - 7.0).abs() <= 0.05,
            "a member receives exactly the other seven bundles: {}",
            down / up
        );
        // `input_downlink_bytes` counts envelopes only, so that it compares
        // with the star's figure. The wire also carries the per-line origin
        // that finding 2 makes mandatory, plus the separators between
        // lines, so the true downlink is strictly higher and the envelope
        // figure is a floor.
        assert!(
            framed > down,
            "framed {framed:.1} must exceed the envelope figure {down:.1}"
        );
        // Carries one `confirmed_span` header field per input packet, same
        // as the original test suite's pinned bracket.
        assert!(
            framed > 1450.0 && framed < 1475.0,
            "framed downlink {framed:.1} B/tick"
        );
    }
}
