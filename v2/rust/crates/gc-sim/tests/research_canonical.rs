//! Port of `spec/sim/research_canonical_spec.lua`.
//!
//! `spec/fixtures/research/canonical_vectors.lua`'s pinned values are
//! reproduced here as Rust constants (`SAMPLE_BYTES`/`SAMPLE_HASH`,
//! `SNAPSHOT_VERSION`/`COMBAT_VERSION`, and the register/session/response
//! hashes) rather than a second shared vector file: they are hand-checked
//! literals already committed to the Lua tree, not something this port
//! needs to regenerate from a fresh `love` run.
//!
//! Two of the five example-package assertions in "pins the example package
//! content hashes" — `TAPE_CONTENT_HASH`/`TRACE_ID`/
//! `SIMULATION_IDENTITY_HASH`/`TRACE_MANIFEST_HASH`/`EVENT_STREAM_HASH` —
//! only exist because `example_package.gameplay()` derives them from a real
//! rollback timeline and trace manifest (`sim::research_trace`,
//! `sim::rollback_events`). Both are now ported, but the five values are
//! still not pinned against the Lua literals in their own case,
//! `pins_the_example_package_content_hashes_trace_derived`: they are derived
//! from `sim::r#match`/`sim::input_tape`, and `v2/tools/lua_reference/` has
//! no vector file proving this port's match engine reproduces the *Lua*
//! engine's snapshot/boundary-hash bytes for this fixture (only
//! `diagnostics_schema_vectors.txt` and `research_schema_vectors.txt` exist
//! there). Copying the Lua literals in would assert something nobody has
//! verified, so that case asserts internal self-consistency instead — see
//! its own doc comment, and `tests/research_fixtures/mod.rs`'s module doc
//! for the identical reasoning applied to `short_match_tape`'s boundary
//! hashes. The other three (`SESSION_ENVELOPE_HASH`, `RESPONSE_SET_HASH`,
//! `FEATURE_REGISTRY_HASH`) need only a bare `trace_id` string and are fully
//! portable — see `pins_the_example_package_content_hashes_hand_built`.

mod research_fixtures;

use gc_sim::match_snapshot;
use gc_sim::research_features;
use gc_sim::research_response;
use gc_sim::research_schema::{self, ResearchField, ResearchFieldKind, Value};
use gc_sim::research_session;
use gc_sim::research_timeline;
use gc_sim::research_trace;

// spec/fixtures/research/canonical_vectors.lua
const SERIALIZATION_VERSION: i64 = 1;
const DIGEST: &str = "fnv1a64/v1";
const SAMPLE_SHAPE_NAME: &str = "research_canonical_sample/v1";
const SAMPLE_HASH: &str = "2c43b30a590f0da8";
const SNAPSHOT_VERSION: i64 = 11;
const COMBAT_VERSION: i64 = 13;
const SESSION_ENVELOPE_HASH: &str = "0da43aba0805a72b";
const RESPONSE_SET_HASH: &str = "04b559ff59cea90e";
const FEATURE_REGISTRY_HASH: &str = "7a42fe98b1bc784c";
// canonical_vectors.TRACE_ID: the trace_id `example_package.gameplay()`
// actually derives. Not computable without `sim::research_trace`, but the
// session/response hashes below were computed against it, so it is used
// verbatim as a bare `trace_id` string (not re-derived).
const TRACE_ID: &str = "d7491ed5cc4cd10b";

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn sample_shape() -> research_schema::ResearchShape {
    research_schema::record(
        SAMPLE_SHAPE_NAME,
        vec![
            ResearchField::new(ResearchFieldKind::Id).named("id"),
            ResearchField::new(ResearchFieldKind::Integer).named("count"),
            ResearchField::new(ResearchFieldKind::Number).named("share"),
            ResearchField::new(ResearchFieldKind::Array)
                .named("tags")
                .element(
                    ResearchField::new(ResearchFieldKind::Enum)
                        .named("tag")
                        .values(research_schema::enum_values(&["alpha", "beta"])),
                ),
        ],
    )
}

fn sample() -> Value {
    record(vec![
        ("id", Value::str("sample")),
        ("count", Value::Number(7.0)),
        ("share", Value::Number(0.5)),
        ("tags", Value::Array(vec![Value::str("alpha")])),
    ])
}

#[test]
fn research_canonical_serialization_vectors_pins_the_serialization_version_and_digest_name() {
    assert_eq!(
        research_schema::SERIALIZATION_VERSION,
        SERIALIZATION_VERSION
    );
    assert_eq!(research_schema::DIGEST, DIGEST);
}

#[test]
fn research_canonical_serialization_vectors_pins_the_simulation_versions_the_vectors_were_computed_against()
 {
    assert_eq!(
        match_snapshot::VERSION,
        SNAPSHOT_VERSION,
        "snapshot version moved: regenerate the canonical vectors"
    );
    assert_eq!(
        match_snapshot::COMBAT_VERSION,
        COMBAT_VERSION,
        "combat version moved: regenerate the canonical vectors"
    );
}

#[test]
fn research_canonical_serialization_vectors_pins_the_hand_written_sample_wire_bytes_and_digest() {
    let shape = sample_shape();
    let bytes = research_schema::encode(&shape, &sample()).unwrap();
    let expected_bytes: &[u8] = concat!(
        "GCRS1;",
        "28:research_canonical_sample/v1;",
        "r1:4;",
        "k2:id;s6:sample;",
        "k5:count;i1:7;",
        "k5:share;d14:p:0:33554432:0;",
        "k4:tags;a1:1;e5:alpha;",
    )
    .as_bytes();
    assert_eq!(bytes, expected_bytes);
    assert_eq!(
        research_schema::content_hash(&shape, &sample()).unwrap(),
        SAMPLE_HASH
    );
    let decoded = research_schema::decode(&shape, &bytes).unwrap();
    assert_eq!(research_schema::encode(&shape, &decoded).unwrap(), bytes);
}

/// The three example-package hashes that need only a bare `trace_id`
/// string, not a real trace manifest. See the module doc comment.
#[test]
fn pins_the_example_package_content_hashes_hand_built() {
    let envelope = record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_session_envelope")),
        ("digest", Value::str(DIGEST)),
        ("session_id", Value::str("s-1c9d4f7a3b6e2058")),
        ("participant_id", Value::str("p-7f3a9c21b8e45d06")),
        ("study_id", Value::str("m11-combat-fun")),
        ("protocol_version", Value::str("m11-protocol-v3")),
        ("cohort_id", Value::str("confirmatory-wave-1")),
        ("recruitment_channel", Value::str("local-football-club")),
        ("block_id", Value::str("block-b")),
        ("trial_id", Value::str("trial-2")),
        ("condition_id", Value::str("condition-b-combat-on")),
        ("condition_order", Value::Number(2.0)),
        ("sequence_label", Value::str("ab")),
        ("counterbalancing_cell", Value::str("cell-ab-home-seed38")),
        (
            "assignment",
            record(vec![
                ("build_id", Value::str("spec-build")),
                ("tuning_config_id", Value::str("tuning-default-v1")),
                ("seed", Value::Number(38.0)),
                ("side", Value::str("home")),
                ("role_id", Value::str("outfield-slot-1")),
                ("loadout_id", Value::str("loadout-spring-gloves")),
                ("opponent_population", Value::str("bot")),
                ("opponent_policy_id", Value::str("bot-baseline-v1")),
                ("practice_block", Value::Bool(false)),
            ]),
        ),
        (
            "environment",
            record(vec![
                ("device_class", Value::str("desktop")),
                ("control_device", Value::str("keyboard")),
                ("control_mapping_id", Value::str("kb-default-v2")),
                ("viewport_w", Value::Number(1280.0)),
                ("viewport_h", Value::Number(720.0)),
                ("render_hz", Value::Number(60.0)),
                ("input_latency_ms", Value::Number(24.5)),
                ("language_tag", Value::str("en-gb")),
                (
                    "readability_settings",
                    Value::Map(vec![
                        ("high_contrast_cues".to_string(), Value::str("on")),
                        ("cue_scale".to_string(), Value::str("1.25")),
                    ]),
                ),
            ]),
        ),
        (
            "lifecycle",
            record(vec![
                ("status", Value::str("completed")),
                ("started_wall_clock_ms", Value::Number(1000.0)),
                ("ended_wall_clock_ms", Value::Number(900_000.0)),
                ("observer_mode", Value::str("same_room")),
                ("assistance", Value::str("none")),
                ("interruptions", Value::Array(vec![])),
                ("exclusions", Value::Array(vec![])),
                ("missingness", Value::Array(vec![])),
            ]),
        ),
        (
            "experience",
            record(vec![
                ("play_hours_per_week", Value::Number(6.5)),
                ("years_playing", Value::Number(12.0)),
                ("football_games_ordinal", Value::Number(4.0)),
                ("action_games_ordinal", Value::Number(2.0)),
                ("self_reported_familiarity_ordinal", Value::Number(3.0)),
                (
                    "derived_label",
                    record(vec![
                        ("label", Value::str("intermediate")),
                        ("calibration_id", Value::str("experience-calibration-v1")),
                        ("calibration_version", Value::Number(1.0)),
                    ]),
                ),
            ]),
        ),
        (
            "agreement",
            record(vec![
                ("agreement_version", Value::str("playtest-agreement-v3")),
                ("accepted", Value::Bool(true)),
                ("accepted_wall_clock_ms", Value::Number(500.0)),
                ("model_use_covered", Value::Bool(true)),
            ]),
        ),
        (
            "trace_links",
            Value::Array(vec![record(vec![
                ("trace_id", Value::str(TRACE_ID)),
                ("game_instance_id", Value::str("gi-0001")),
                ("role", Value::str("condition_block")),
            ])]),
        ),
    ]);
    assert_eq!(
        research_session::content_hash(&envelope).unwrap(),
        SESSION_ENVELOPE_HASH
    );

    let responses = record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_response_set")),
        ("digest", Value::str(DIGEST)),
        ("response_set_id", Value::str("rs-pxi-enjoyment-b-1")),
        ("session_id", Value::str("s-1c9d4f7a3b6e2058")),
        ("participant_id", Value::str("p-7f3a9c21b8e45d06")),
        ("condition_id", Value::str("condition-b-combat-on")),
        ("instrument_id", Value::str("pxi_enjoyment_addon")),
        (
            "instrument_version",
            Value::str("pxi-2021.1-enjoyment-addon"),
        ),
        ("scoring_key_version", Value::str("pxi-enjoyment-mean-1")),
        ("analysis_role", Value::str("primary")),
        ("validated_instrument", Value::Bool(true)),
        ("partial_administration", Value::Bool(false)),
        (
            "locale",
            record(vec![
                ("language_tag", Value::str("en-gb")),
                ("translation_provenance", Value::str("original")),
            ]),
        ),
        (
            "timing",
            record(vec![
                ("relative_to_play", Value::str("post_block")),
                ("offset_ms", Value::Number(4200.0)),
                ("canonical_boundary_tick", Value::Number(3.0)),
            ]),
        ),
        (
            "presentation_order",
            Value::Array(vec![
                Value::str("pxi_enjoy_2"),
                Value::str("pxi_enjoy_1"),
                Value::str("pxi_enjoy_3"),
            ]),
        ),
        ("randomized_presentation", Value::Bool(true)),
        (
            "responses",
            Value::Array(vec![
                record(vec![
                    ("item_id", Value::str("pxi_enjoy_1")),
                    ("raw_response", Value::Number(2.0)),
                    ("response_ms", Value::Number(3400.0)),
                ]),
                record(vec![
                    ("item_id", Value::str("pxi_enjoy_2")),
                    ("raw_response", Value::Number(1.0)),
                    ("response_ms", Value::Number(2900.0)),
                ]),
                record(vec![
                    ("item_id", Value::str("pxi_enjoy_3")),
                    ("raw_response", Value::Number(3.0)),
                    ("response_ms", Value::Number(4100.0)),
                ]),
            ]),
        ),
        (
            "scores",
            Value::Array(vec![record(vec![
                ("construct", Value::str("enjoyment")),
                ("score", Value::Number(2.0)),
                ("rule", Value::str("mean_all_items")),
                ("item_count", Value::Number(3.0)),
            ])]),
        ),
    ]);
    assert_eq!(
        research_response::content_hash(&responses).unwrap(),
        RESPONSE_SET_HASH
    );

    assert_eq!(research_features::registry_hash(), FEATURE_REGISTRY_HASH);
}

/// `canonical_vectors.TAPE_CONTENT_HASH` / `.TRACE_ID` /
/// `.SIMULATION_IDENTITY_HASH` / `.TRACE_MANIFEST_HASH` / `.EVENT_STREAM_HASH`
/// are NOT asserted against the Lua literals here — see this file's module
/// doc comment for why. Instead this proves the five values are internally
/// coherent: the manifest's stored `tape_content_hash`/`trace_id` match
/// what `research_trace::tape_content_hash`/`derive_trace_id` recompute from
/// the same tape/manifest, `simulation_identity_hash`/`content_hash` produce
/// canonical `fnv1a64/v1` digests, and the stream's stored `stream_hash`
/// matches what `research_timeline::stream_hash` recomputes from its rows.
#[test]
fn pins_the_example_package_content_hashes_trace_derived() {
    let gameplay = research_fixtures::gameplay(None);
    let manifest_entries = gameplay.manifest.as_record().expect("manifest is a record");
    let simulation = Value::record_get(manifest_entries, "simulation")
        .and_then(Value::as_record)
        .expect("manifest has a simulation record");

    let tape_content_hash = Value::record_get(simulation, "tape_content_hash")
        .and_then(Value::as_str)
        .expect("simulation has a tape_content_hash");
    assert_eq!(
        tape_content_hash,
        research_trace::tape_content_hash(&gameplay.tape).expect("tape content hash recomputes")
    );

    let trace_id = Value::record_get(manifest_entries, "trace_id")
        .and_then(Value::as_str)
        .expect("manifest has a trace_id");
    assert_eq!(
        trace_id,
        research_trace::derive_trace_id(&gameplay.manifest).expect("trace id recomputes")
    );

    let simulation_identity_hash = research_trace::simulation_identity_hash(&gameplay.manifest)
        .expect("simulation identity hash computes");
    assert_eq!(simulation_identity_hash.len(), research_schema::HASH_LENGTH);

    let trace_manifest_hash = research_trace::content_hash(&gameplay.manifest)
        .expect("trace manifest content hash computes");
    assert_eq!(trace_manifest_hash.len(), research_schema::HASH_LENGTH);

    let stream_entries = gameplay.stream.as_record().expect("stream is a record");
    let stream_hash = Value::record_get(stream_entries, "stream_hash")
        .and_then(Value::as_str)
        .expect("stream has a stream_hash");
    assert_eq!(
        stream_hash,
        research_timeline::stream_hash(&gameplay.stream).expect("stream hash recomputes")
    );
}
