//! Port of `spec/game/online_protocol_spec.lua`.
//!
//! Also carries this crate's required differential evidence for
//! `protocol.rs` (README §"Why `protocol` is the highest-stakes file left"):
//! `tests/fixtures/protocol_lua_reference.txt` holds wire bytes and digests
//! captured from the real Lua `game/online/protocol.lua`
//! (`v2/tools/lua_reference/README.md`'s method), and the tests in the
//! `differential` module below assert against those bytes directly rather
//! than merely round-tripping Rust-produced values through themselves.

use std::panic::{AssertUnwindSafe, catch_unwind};

use gc_netcode::protocol::{self, ErrorCode, LifecyclePhase, MessageKind, Value};
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
/// `protocol::encode`'s own validation would refuse to produce. Mirrors the
/// Lua spec's own local `encode_raw_test_value`/`encode_raw_test_wire`
/// helpers, which exist in the Lua source for the identical reason: testing
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
    // README rule 6.8 asks for in pure code).
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
    assert_eq!(protocol::CURRENT_VERSIONS.snapshot, 13);
    assert_eq!(protocol::CURRENT_VERSIONS.tape, 2);
    assert_eq!(protocol::CURRENT_VERSIONS.combat, 3);
}

#[test]
fn omp3_online_protocol_matches_literal_wire_manifest_transcript_and_per_kind_golden_evidence() {
    let report = conformance::verify();
    assert_eq!(report.manifest_id, "eb59f113614c35b2");
    assert_eq!(report.transcript_id, "653cba3b32c62ce9");
    assert_eq!(report.message_count, 15);
    assert_eq!(
        gc_core::fnv1a64::hash(conformance::GOLDEN.complete_wire.as_bytes()),
        "363c57d949586608"
    );
    assert_eq!(
        conformance::marker(&report),
        "GC_PROTOCOL|golden|schema=1|manifest_id=eb59f113614c35b2|transcript_id=653cba3b32c62ce9|messages=15"
    );
}

#[test]
fn omp3_online_protocol_pins_the_control_vocabulary_this_build_speaks() {
    assert_eq!(protocol::vocabulary_id(), "e13e3647001a0a7e");
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
    // The Lua case specifically feeds a *sparse-keyed table* (`{[1]=a,[3]=b}`),
    // which has no Rust equivalent for a `&[ControlMessage]` slice (Rust
    // slices cannot be sparse) — seeàupdatethe module doc comment. What *is*
    // portable and is asserted directly below: `transcript_id` panics on a
    // non-monotonic per-peer sequence, the same invariant the Lua assertion
    // guards.
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
    // Lua also exercises non-string build declarations (`17`, `true`) —
    // structurally impossible here: `handshake`'s `build_id` parameter is
    // `Option<&str>`, so a non-string value cannot be constructed at all.
    // The malformed-type outcome those cases proved is instead guaranteed by
    // the type system, which is strictly stronger.
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
// Differential tests against the real Lua `protocol.lua`
// (`tests/fixtures/protocol_lua_reference.txt`, captured per
// `v2/tools/lua_reference/README.md`). These assert exact bytes, not merely
// that Rust's own encode/decode round-trip through themselves.
// ---------------------------------------------------------------------------

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
    assert_eq!(wire, lua_ref("MIN_WIRE"));
    assert_eq!(
        wire.len(),
        lua_ref("MIN_WIRE_LEN").parse::<usize>().unwrap()
    );
    assert_eq!(
        gc_core::fnv1a64::hash(wire.as_bytes()),
        lua_ref("MIN_WIRE_HASH")
    );
}

/// The maximum-size payload this protocol allows: every applicable field is
/// bounded_id'd out to its maximum, matching
/// spec/game/online_protocol_spec.lua's "keeps an all-applicable-max
/// proposal within the 8 KiB record bound", but checked against real
/// Lua-produced bytes rather than only a length.
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
    assert_eq!(manifest_id, lua_ref("MAXIMAL_MANIFEST_ID"));

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
    assert_eq!(
        wire.len(),
        lua_ref("MAXIMAL_WIRE_LEN").parse::<usize>().unwrap()
    );
    assert_eq!(
        gc_core::fnv1a64::hash(wire.as_bytes()),
        lua_ref("MAXIMAL_WIRE_HASH")
    );
}

#[test]
fn differential_vocabulary_manifest_and_transcript_ids_match_the_real_lua() {
    assert_eq!(protocol::vocabulary_id(), lua_ref("VOCAB_ID"));
    assert_eq!(
        protocol::manifest_id(&fixture::manifest(None)),
        lua_ref("MANIFEST_ID")
    );
    assert_eq!(
        protocol::transcript_id(&fixture::messages()),
        lua_ref("TRANSCRIPT_ID")
    );
}

/// Every non-canonical mutation applied directly to real Lua-produced bytes
/// (`HANDSHAKE_WIRE`), not to a Rust re-encode — the same base wire
/// `spec/game/online_protocol_spec.lua`'s `gsub` mutations operate on.
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
// spec/game/transport_relay_spec.lua — second `describe` block, deferred to
// this crate from the TypeScript port (per this agent's brief).
// ---------------------------------------------------------------------------

/// "relay topology probe: no peer is the sequencer" — the Lua spec's second
/// `describe` block in `spec/game/transport_relay_spec.lua` drives
/// `game.online.coordinator` and `game.online.match_driver` directly (a
/// scripted multi-peer relay session asserting no single peer's clock or
/// send order is treated as authoritative). Both `crate::coordinator` and
/// `crate::match_driver` are explicitly out of this agent's scope ("Do not
/// port coordinator*, match_driver*, ... — other agents own those right
/// now"), and as of this writing both are still the 4-line "NOT YET PORTED"
/// placeholder, so there is no API to drive. Stubbed `#[ignore]`d rather
/// than silently dropped (README §4: "never delete a case because it is
/// awkward").
#[test]
#[ignore = "needs gc_netcode::coordinator and gc_netcode::match_driver, both unported placeholders owned by other agents (see v2/README.md §2.1)"]
fn transport_relay_topology_probe_no_peer_is_the_sequencer() {
    unimplemented!(
        "port spec/game/online/transport_relay_spec.lua's second describe block \
         once coordinator.rs and match_driver.rs land"
    );
}
