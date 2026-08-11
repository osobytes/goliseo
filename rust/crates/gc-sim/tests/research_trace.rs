//! Tests for `gc_sim::research_trace`.
//!
//! One sub-case of "rejects participant fields in the manifest and in the
//! tape identity" has no Rust equivalent and is dropped with an inline
//! comment rather than the whole test: injecting an extra `participant_id`
//! field directly onto an `InputTapeIdentity` and checking that
//! `input_tape::copy_identity` rejects it. `InputTapeIdentity` here
//! (`gc_sim::input_tape`) is a statically typed Rust `struct` with a fixed
//! field set — there is no way to attach an extra field to a value of that
//! type, so the case that assertion exists to catch is a compile-time
//! impossibility rather than a runtime one. The other two assertions in that
//! same test (an unknown field on the manifest itself, and on
//! `manifest.simulation`) are exercised below, since a `gameplay_trace_manifest`
//! is a dynamically-typed `research_schema::Value` that accepts an injected
//! field at runtime.

mod research_fixtures;

use gc_sim::input_frame;
use gc_sim::input_tape::InputTape;
use gc_sim::match_snapshot;
use gc_sim::research_schema::{self, Value};
use gc_sim::research_trace::{self, ResearchTraceOptions, ResearchTraceProducerInput};

fn manifest_and_tape() -> (Value, InputTape) {
    let gameplay = research_fixtures::gameplay(None);
    (gameplay.manifest, gameplay.tape)
}

fn tape_signature(tape: &InputTape) -> String {
    let mut parts = vec![
        match_snapshot::hash(&tape.initial),
        tape.version.to_string(),
        tape.frames.len().to_string(),
    ];
    for frame in &tape.frames {
        parts.push(input_frame::encode(frame).expect("frame encodes"));
    }
    for hash in &tape.boundary_hashes {
        parts.push(hash.clone());
    }
    parts.join("|")
}

fn record_entries(value: &Value) -> Vec<(String, Value)> {
    value.as_record().expect("value is a record").to_vec()
}

fn get<'a>(entries: &'a [(String, Value)], name: &str) -> &'a Value {
    Value::record_get(entries, name).unwrap_or_else(|| panic!("missing field {name}"))
}

fn set(entries: &mut Vec<(String, Value)>, name: &str, value: Value) {
    if let Some(entry) = entries.iter_mut().find(|(k, _)| k == name) {
        entry.1 = value;
    } else {
        entries.push((name.to_string(), value));
    }
}

fn remove(entries: &mut Vec<(String, Value)>, name: &str) {
    entries.retain(|(k, _)| k != name);
}

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn with_top(manifest: &Value, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    let mut top = record_entries(manifest);
    f(&mut top);
    Value::Record(top)
}

fn with_simulation(manifest: &Value, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    with_top(manifest, |top| {
        let mut simulation = record_entries(get(top, "simulation"));
        f(&mut simulation);
        set(top, "simulation", Value::Record(simulation));
    })
}

fn with_runtime(manifest: &Value, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    with_top(manifest, |top| {
        let mut runtime = record_entries(get(top, "runtime"));
        f(&mut runtime);
        set(top, "runtime", Value::Record(runtime));
    })
}

fn with_producer(
    manifest: &Value,
    index: usize,
    f: impl FnOnce(&mut Vec<(String, Value)>),
) -> Value {
    with_simulation(manifest, |simulation| {
        let mut producers: Vec<Value> = get(simulation, "producers")
            .as_array()
            .expect("producers is an array")
            .to_vec();
        let mut producer = record_entries(&producers[index]);
        f(&mut producer);
        producers[index] = Value::Record(producer);
        set(simulation, "producers", Value::Array(producers));
    })
}

#[test]
fn gameplay_trace_manifest_derives_its_identity_from_the_tape_it_references() {
    let (manifest, tape) = manifest_and_tape();
    let simulation = record_entries(get(&record_entries(&manifest), "simulation"));
    assert_eq!(
        get(&simulation, "initial_boundary_hash").as_str(),
        Some(tape.boundary_hashes[0].as_str())
    );
    assert_eq!(
        get(&simulation, "final_boundary_hash").as_str(),
        Some(tape.boundary_hashes[tape.boundary_hashes.len() - 1].as_str())
    );
    assert_eq!(
        get(&simulation, "frame_count").as_number(),
        Some(tape.frames.len() as f64)
    );
    assert_eq!(
        get(&simulation, "last_boundary_tick").as_number(),
        Some(tape.frames.len() as f64)
    );
    assert_eq!(
        get(&simulation, "tape_content_hash").as_str(),
        research_trace::tape_content_hash(&tape).ok().as_deref()
    );
    let top = record_entries(&manifest);
    assert_eq!(
        get(&top, "trace_id").as_str(),
        research_trace::derive_trace_id(&manifest).ok().as_deref()
    );
}

#[test]
fn gameplay_trace_manifest_keeps_every_manifest_field_in_exactly_one_identity_group() {
    let groups = research_trace::field_groups();
    research_schema::assert_disjoint("groups", &groups).expect("groups are disjoint");
    let covered: usize = groups.iter().map(|(_, members)| members.len()).sum();
    let shape = research_trace::shape();
    let field_count = shape.fields.expect("shape has fields").len();
    assert_eq!(covered, field_count);
}

#[test]
fn research_metadata_cannot_move_a_simulation_boundary_hash_leaves_the_tape_and_its_simulation_identity_untouched_when_links_are_added()
 {
    let (manifest, tape) = manifest_and_tape();
    let before_tape = tape_signature(&tape);
    let before_identity =
        research_trace::simulation_identity_hash(&manifest).expect("identity hash");
    let before_content = research_trace::content_hash(&manifest).expect("content hash");

    let link_one = record(vec![
        ("link_kind", Value::str("research_session")),
        ("target_id", Value::str(research_fixtures::SESSION_ID)),
        ("target_hash", Value::str("1234567890abcdef")),
    ]);
    let linked = research_trace::with_research_link(&manifest, &link_one).expect("link added");
    let link_two = record(vec![
        ("link_kind", Value::str("annotation_set")),
        ("target_id", Value::str("as-participant-debrief-1")),
        ("target_hash", Value::str("fedcba0987654321")),
    ]);
    let twice = research_trace::with_research_link(&linked, &link_two).expect("second link added");

    assert_eq!(tape_signature(&tape), before_tape, "the tape changed");
    let manifest_top = record_entries(&manifest);
    assert_eq!(
        get(&manifest_top, "research_links")
            .as_array()
            .expect("array")
            .len(),
        0,
        "the source manifest was mutated"
    );
    let twice_top = record_entries(&twice);
    assert_eq!(
        get(&twice_top, "research_links")
            .as_array()
            .expect("array")
            .len(),
        2
    );
    assert_eq!(
        research_trace::simulation_identity_hash(&twice).expect("identity hash"),
        before_identity
    );
    assert_eq!(
        get(&twice_top, "trace_id").as_str(),
        get(&manifest_top, "trace_id").as_str()
    );
    // The whole-manifest hash *does* move: annotations are recorded, not hidden.
    assert_ne!(
        research_trace::content_hash(&twice).expect("content hash"),
        before_content
    );
}

#[test]
fn research_metadata_cannot_move_a_simulation_boundary_hash_rejects_participant_fields_in_the_manifest_and_in_the_tape_identity()
 {
    let (manifest, _tape) = manifest_and_tape();
    let injected = with_top(&manifest, |top| {
        set(top, "participant_id", Value::str("p-7f3a9c21b8e45d06"));
    });
    let err = research_trace::validate(&injected).expect_err("unknown field must be rejected");
    assert!(err.contains("unknown field"), "unexpected error: {err}");

    let simulation_injected = with_simulation(&manifest, |simulation| {
        set(
            simulation,
            "participant_id",
            Value::str("p-7f3a9c21b8e45d06"),
        );
    });
    assert!(research_trace::validate(&simulation_injected).is_err());

    // Injecting `participant_id` directly onto an `InputTapeIdentity` and
    // checking that `input_tape::copy_identity` rejects it has no Rust
    // equivalent here: see this file's module doc comment.
}

#[test]
fn research_metadata_cannot_move_a_simulation_boundary_hash_refuses_duplicate_research_links() {
    let (manifest, _tape) = manifest_and_tape();
    let link = record(vec![
        ("link_kind", Value::str("research_session")),
        ("target_id", Value::str(research_fixtures::SESSION_ID)),
        ("target_hash", Value::str("1234567890abcdef")),
    ]);
    let linked = research_trace::with_research_link(&manifest, &link).expect("link added");
    let err = research_trace::with_research_link(&linked, &link)
        .expect_err("duplicate link must be rejected");
    assert!(err.contains("duplicated"), "unexpected error: {err}");
}

#[test]
fn gameplay_trace_manifest_serialization_round_trips_byte_for_byte() {
    let (manifest, _tape) = manifest_and_tape();
    let bytes = research_trace::encode(&manifest).expect("manifest encodes");
    let decoded = research_trace::decode(&bytes).expect("manifest decodes");
    assert_eq!(
        research_trace::encode(&decoded).expect("decoded manifest encodes"),
        bytes
    );
    let manifest_top = record_entries(&manifest);
    let decoded_top = record_entries(&decoded);
    assert_eq!(
        get(&decoded_top, "trace_id").as_str(),
        get(&manifest_top, "trace_id").as_str()
    );
    let manifest_simulation = record_entries(get(&manifest_top, "simulation"));
    let decoded_simulation = record_entries(get(&decoded_top, "simulation"));
    assert_eq!(
        get(&decoded_simulation, "seed").as_number(),
        get(&manifest_simulation, "seed").as_number()
    );
    assert_eq!(
        get(&decoded_simulation, "producers")
            .as_array()
            .expect("array")
            .len(),
        input_frame::SLOT_COUNT as usize
    );
}

#[test]
fn gameplay_trace_manifest_serialization_fails_closed_on_malformed_unknown_and_cross_version_payloads()
 {
    let (manifest, _tape) = manifest_and_tape();
    let bytes = research_trace::encode(&manifest).expect("manifest encodes");
    let truncated = &bytes[..bytes.len() - 8];
    assert!(research_trace::decode(truncated).is_err());
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(b"n;");
    assert!(research_trace::decode(&trailing).is_err());

    let malformed_hash = with_simulation(&manifest, |simulation| {
        set(simulation, "tape_content_hash", Value::str("not-a-hash"));
    });
    assert!(research_trace::validate(&malformed_hash).is_err());

    let future = with_top(&manifest, |top| {
        set(
            top,
            "schema_version",
            Value::Number((research_trace::VERSION + 1) as f64),
        );
    });
    let err = research_trace::validate(&future).expect_err("future version must be rejected");
    assert!(err.contains("no migration"), "unexpected error: {err}");
}

#[test]
fn gameplay_trace_manifest_serialization_rejects_non_finite_numbers_in_this_family() {
    let (manifest, _tape) = manifest_and_tape();
    let nan = with_runtime(&manifest, |runtime| {
        set(runtime, "mean_frame_ms", Value::Number(f64::NAN));
    });
    assert!(research_trace::validate(&nan).is_err());

    let infinite = with_simulation(&manifest, |simulation| {
        set(simulation, "seed", Value::Number(f64::INFINITY));
    });
    assert!(research_trace::validate(&infinite).is_err());
}

#[test]
fn gameplay_trace_manifest_serialization_detects_a_hand_edited_trace_id_or_simulation_field() {
    let (manifest, _tape) = manifest_and_tape();
    let manifest_simulation = record_entries(get(&record_entries(&manifest), "simulation"));
    let seed = get(&manifest_simulation, "seed").as_number().expect("seed");
    let retagged = with_simulation(&manifest, |simulation| {
        set(simulation, "seed", Value::Number(seed + 1.0));
    });
    let err = research_trace::validate(&retagged).expect_err("retagged seed must be rejected");
    assert!(err.contains("trace_id"), "unexpected error: {err}");

    let relabelled = with_top(&manifest, |top| {
        set(top, "trace_id", Value::str("0000000000000000"));
    });
    assert!(research_trace::validate(&relabelled).is_err());
}

#[test]
fn gameplay_trace_manifest_invariants_requires_canonical_slot_order_and_honest_producer_kinds() {
    let (manifest, _tape) = manifest_and_tape();
    let simulation = record_entries(get(&record_entries(&manifest), "simulation"));
    let producers = get(&simulation, "producers")
        .as_array()
        .expect("array")
        .to_vec();
    let slot1 = get(&record_entries(&producers[1]), "slot").clone();
    let shuffled = with_producer(&manifest, 0, |entries| set(entries, "slot", slot1));
    assert!(research_trace::validate(&shuffled).is_err());

    let human_with_policy = with_producer(&manifest, 0, |entries| {
        set(entries, "producer_policy_id", Value::str("bot-baseline-v1"));
    });
    assert!(research_trace::validate(&human_with_policy).is_err());

    let bot_without_policy = with_producer(&manifest, 1, |entries| {
        remove(entries, "producer_policy_id")
    });
    assert!(research_trace::validate(&bot_without_policy).is_err());
}

#[test]
fn gameplay_trace_manifest_invariants_rejects_an_inconsistent_tick_range_completion_or_divergence()
{
    let (manifest, _tape) = manifest_and_tape();
    let simulation = record_entries(get(&record_entries(&manifest), "simulation"));
    let last_boundary_tick = get(&simulation, "last_boundary_tick")
        .as_number()
        .expect("last_boundary_tick");
    let bad_range = with_simulation(&manifest, |simulation| {
        set(
            simulation,
            "last_boundary_tick",
            Value::Number(last_boundary_tick + 1.0),
        );
    });
    assert!(research_trace::validate(&bad_range).is_err());

    let completed_with_divergence = with_simulation(&manifest, |simulation| {
        set(
            simulation,
            "divergence",
            record(vec![
                ("boundary_tick", Value::Number(2.0)),
                ("expected_hash", Value::str("1111111111111111")),
                ("actual_hash", Value::str("2222222222222222")),
                ("state_path", Value::str("state.ball.x")),
            ]),
        );
    });
    assert!(research_trace::validate(&completed_with_divergence).is_err());
}

#[test]
fn gameplay_trace_manifest_invariants_keeps_combat_identity_tied_to_the_combat_tape_version() {
    use gc_sim::input_tape;
    let (manifest, _tape) = manifest_and_tape();
    let simulation = record_entries(get(&record_entries(&manifest), "simulation"));
    assert_eq!(
        get(&simulation, "tape_version").as_number(),
        Some(input_tape::VERSION as f64)
    );
    let soccer_with_combat = with_simulation(&manifest, |simulation| {
        set(simulation, "combat_identity", Value::str("combat-identity"));
    });
    assert!(research_trace::validate(&soccer_with_combat).is_err());
}

#[test]
fn gameplay_trace_manifest_invariants_only_accepts_an_event_stream_that_describes_the_same_run() {
    let gameplay = research_fixtures::gameplay(None);
    research_trace::validate_against_stream(&gameplay.manifest, &gameplay.stream)
        .expect("gameplay stream validates against its own manifest");

    let other_scope = with_top(&gameplay.stream, |top| {
        set(top, "run_scope_id", Value::str("abcdefabcdefabcd"));
    });
    assert!(research_trace::validate_against_stream(&gameplay.manifest, &other_scope).is_err());

    let other_instance = with_top(&gameplay.stream, |top| {
        set(top, "game_instance_id", Value::str("gi-0002"));
    });
    assert!(research_trace::validate_against_stream(&gameplay.manifest, &other_instance).is_err());
}

#[test]
fn gameplay_trace_manifest_invariants_refuses_a_producer_set_that_is_not_one_row_per_canonical_slot()
 {
    let tape = research_fixtures::short_match_tape();
    let short_producers: Vec<ResearchTraceProducerInput> = (0..input_frame::SLOT_COUNT - 1)
        .map(|_| ResearchTraceProducerInput {
            producer_kind: "bot".to_string(),
            producer_policy_id: Some("bot-1".to_string()),
        })
        .collect();
    let runtime = record(vec![
        ("platform", Value::str("linux-native")),
        ("renderer", Value::str("opengl")),
        ("render_hz", Value::Number(60.0)),
        ("render_hz_mode", Value::str("fixed")),
        ("input_device", Value::str("keyboard")),
        ("mean_frame_ms", Value::Number(8.0)),
        ("p99_frame_ms", Value::Number(14.0)),
        ("dropped_frame_count", Value::Number(0.0)),
        ("pause_count", Value::Number(0.0)),
        ("goal_replay_count", Value::Number(0.0)),
        ("rollback_count", Value::Number(0.0)),
        ("max_rollback_ticks", Value::Number(0.0)),
        ("raw_device_event_policy", Value::str("not_collected")),
        ("raw_device_event_clock", Value::str("none")),
    ]);
    let options = ResearchTraceOptions {
        game_instance_id: "gi-0001".to_string(),
        ruleset_version: 1,
        event_schema_version: 1,
        confirmed_event_stream_hash: "1111111111111111".to_string(),
        completion: "completed".to_string(),
        producers: short_producers,
        runtime,
        divergence: None,
        research_links: None,
    };
    let result = research_trace::from_tape(&tape, &options);
    assert!(result.is_err());
}
