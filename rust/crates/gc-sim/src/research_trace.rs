//! Versioned gameplay trace manifest.
//!
//! The manifest *references* the authoritative gameplay spine in
//! [`crate::input_tape`]; it never duplicates the frame format and never adds
//! a participant or survey field to [`crate::input_tape::InputTapeIdentity`].
//! Two consequences are structural facts here rather than review promises:
//!
//!   1. `simulation` is the only manifest group that feeds
//!      [`simulation_identity_hash`], so research annotations and
//!      observational runtime diagnostics cannot move a simulation boundary
//!      hash (enforced by the `#[cfg(test)]` self-check at the bottom of
//!      this file); and
//!   2. `research_links` is an append-only list of opaque join keys, so one
//!      tape can carry many research annotations without the tape or its
//!      simulation identity changing.
//!
//! Like [`crate::research_session`], every manifest here is a
//! [`research_schema::Value`] built and read through small private field
//! helpers, never a bespoke Rust struct — see that module's doc comment for
//! the rationale this file follows.
//!
//! ## Slot wire strings are duplicated from `env_observation.rs`, not shared
//!
//! `input_frame.rs` exposes no public function mapping [`input_frame::SlotId`]
//! /[`input_frame::Team`] to their canonical wire strings (`"home_1"`..
//! `"away_4"`, `"home"`/`"away"`). `env_observation.rs` has private
//! `slot_id_wire`/`input_team_wire` helpers that do exactly this, but they
//! are private to that module, so [`slot_id_wire`] and [`input_team_wire`]
//! below are a second, private copy — the same precedent `headless.rs`'s
//! module doc documents for its own duplicate of `metrics.rs`'s band table:
//! this module does not own `env_observation.rs`, so it cannot make its
//! helpers `pub`, and the duplication only ever feeds wire-string
//! comparisons, never a determinism-path computation of its own (the strings
//! themselves are already part of the canonical wire format either way).

use crate::fixed_clock;
use crate::input_frame;
use crate::input_tape::{self, InputTape};
use crate::match_snapshot;
use crate::research_schema::{
    self, ResearchField, ResearchFieldKind, ResearchShape, Result, TuplePart, Value,
};

/// Reader version for the trace manifest wire shape.
pub const VERSION: i64 = 1;
/// Serialization versions this reader accepts.
pub const SUPPORTED_VERSIONS: &[i64] = &[1];
/// `manifest_kind` this module writes and reads.
pub const KIND: &str = "gameplay_trace_manifest";
/// Tuple-hash label for [`tape_content_hash`].
pub const TAPE_CONTENT_LABEL: &str = "input-tape-content/v1";
/// Tuple-hash label for [`derive_trace_id`].
pub const TRACE_ID_LABEL: &str = "gameplay-trace/v1";

const SIMULATION_GROUP: &[&str] = &["simulation"];
const ANNOTATION_GROUP: &[&str] = &["runtime", "research_links"];
const ENVELOPE_GROUP: &[&str] = &[
    "schema_version",
    "manifest_kind",
    "digest",
    "trace_id",
    "game_instance_id",
];

/// The manifest field partition. `simulation` alone determines simulation
/// identity; `annotation` and `envelope` groups are deliberately excluded.
#[must_use]
pub fn field_groups() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("simulation", SIMULATION_GROUP),
        ("annotation", ANNOTATION_GROUP),
        ("envelope", ENVELOPE_GROUP),
    ]
}

/// Declared producer kinds.
#[must_use]
pub fn producer_kinds() -> Vec<String> {
    research_schema::enum_values(&["human", "bot", "replay"])
}

/// Declared completion states.
#[must_use]
pub fn completions() -> Vec<String> {
    research_schema::enum_values(&[
        "completed",
        "incomplete_interrupted",
        "incomplete_abandoned",
        "incomplete_process_exit",
    ])
}

/// Declared research-link kinds.
#[must_use]
pub fn link_kinds() -> Vec<String> {
    research_schema::enum_values(&[
        "research_session",
        "annotation_set",
        "response_set",
        "derived_dataset",
    ])
}

fn divergence_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Integer)
            .named("boundary_tick")
            .min(0.0)
            .max(input_frame::MAX_TICK as f64),
        ResearchField::new(ResearchFieldKind::Hash).named("expected_hash"),
        ResearchField::new(ResearchFieldKind::Hash).named("actual_hash"),
        ResearchField::new(ResearchFieldKind::Str).named("state_path"),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("causal_input_tick")
            .optional()
            .min(0.0)
            .max(input_frame::MAX_TICK as f64),
    ]
}

fn producer_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("slot"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("team")
            .values(research_schema::enum_values(&["home", "away"])),
        ResearchField::new(ResearchFieldKind::Id).named("player_id"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("producer_kind")
            .values(producer_kinds()),
        ResearchField::new(ResearchFieldKind::Id)
            .named("producer_policy_id")
            .optional(),
    ]
}

/// Everything that describes deterministic simulation truth. This shape, and
/// only this shape, feeds [`simulation_identity_hash`].
#[must_use]
pub fn simulation_shape() -> ResearchShape {
    research_schema::record(
        "gameplay_trace_simulation/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Integer)
                .named("tape_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("input_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("snapshot_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("ruleset_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("event_schema_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Str)
                .named("combat_identity")
                .optional(),
            ResearchField::new(ResearchFieldKind::Str).named("build"),
            ResearchField::new(ResearchFieldKind::Str).named("source"),
            ResearchField::new(ResearchFieldKind::Str).named("content"),
            // Empty means "no active tuning override", exactly as
            // `tuning::serialize` reports it, so this is the one simulation
            // string allowed to be empty.
            ResearchField::new(ResearchFieldKind::Str)
                .named("tuning")
                .min_length(0),
            ResearchField::new(ResearchFieldKind::Str).named("config"),
            ResearchField::new(ResearchFieldKind::Str).named("fixture"),
            ResearchField::new(ResearchFieldKind::Integer).named("seed"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("tick_rate")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("first_boundary_tick")
                .min(0.0)
                .max(input_frame::MAX_TICK as f64),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("last_boundary_tick")
                .min(0.0)
                .max(input_frame::MAX_TICK as f64),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("frame_count")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Hash).named("tape_content_hash"),
            ResearchField::new(ResearchFieldKind::Hash).named("initial_boundary_hash"),
            ResearchField::new(ResearchFieldKind::Hash).named("final_boundary_hash"),
            ResearchField::new(ResearchFieldKind::Hash).named("confirmed_event_stream_hash"),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("completion")
                .values(completions()),
            ResearchField::new(ResearchFieldKind::Array)
                .named("producers")
                .min_length(input_frame::SLOT_COUNT as usize)
                .max_length(input_frame::SLOT_COUNT as usize)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("producer")
                        .fields(producer_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Record)
                .named("divergence")
                .optional()
                .fields(divergence_fields()),
        ],
    )
}

/// Observational runtime evidence (section 4.0 of the combat fun evidence
/// contract): protocol-repeatable, never byte-identical, never authoritative.
#[must_use]
pub fn runtime_shape() -> ResearchShape {
    research_schema::record(
        "gameplay_trace_runtime/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Id).named("platform"),
            ResearchField::new(ResearchFieldKind::Id).named("renderer"),
            ResearchField::new(ResearchFieldKind::Number)
                .named("render_hz")
                .min(1.0)
                .max(1000.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("render_hz_mode")
                .values(research_schema::enum_values(&["fixed", "variable"])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("input_device")
                .values(research_schema::enum_values(&[
                    "keyboard", "gamepad", "touch", "mixed",
                ])),
            ResearchField::new(ResearchFieldKind::Number)
                .named("mean_frame_ms")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Number)
                .named("p99_frame_ms")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("dropped_frame_count")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("pause_count")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("goal_replay_count")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("rollback_count")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("max_rollback_ticks")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("raw_device_event_policy")
                .values(research_schema::enum_values(&[
                    "not_collected",
                    "minimized_diagnostic",
                    "full_diagnostic",
                ])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("raw_device_event_clock")
                .values(research_schema::enum_values(&[
                    "none",
                    "wall_clock_monotonic",
                ])),
        ],
    )
}

fn link_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Enum)
            .named("link_kind")
            .values(link_kinds()),
        ResearchField::new(ResearchFieldKind::Id).named("target_id"),
        ResearchField::new(ResearchFieldKind::Hash).named("target_hash"),
    ]
}

/// The `gameplay_trace_manifest/v1` record shape.
#[must_use]
pub fn shape() -> ResearchShape {
    research_schema::record(
        "gameplay_trace_manifest/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Integer)
                .named("schema_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("manifest_kind")
                .values(research_schema::enum_values(&[KIND])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("digest")
                .values(research_schema::enum_values(&[research_schema::DIGEST])),
            ResearchField::new(ResearchFieldKind::Hash).named("trace_id"),
            ResearchField::new(ResearchFieldKind::Id).named("game_instance_id"),
            ResearchField::new(ResearchFieldKind::Record)
                .named("simulation")
                .fields(
                    simulation_shape()
                        .fields
                        .expect("simulation_shape always returns a record"),
                ),
            ResearchField::new(ResearchFieldKind::Record)
                .named("runtime")
                .fields(
                    runtime_shape()
                        .fields
                        .expect("runtime_shape always returns a record"),
                ),
            ResearchField::new(ResearchFieldKind::Array)
                .named("research_links")
                .max_length(1024)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("link")
                        .fields(link_fields()),
                ),
        ],
    )
}

fn slot_id_wire(id: input_frame::SlotId) -> &'static str {
    match id {
        input_frame::SlotId::Home1 => "home_1",
        input_frame::SlotId::Home2 => "home_2",
        input_frame::SlotId::Home3 => "home_3",
        input_frame::SlotId::Home4 => "home_4",
        input_frame::SlotId::Away1 => "away_1",
        input_frame::SlotId::Away2 => "away_2",
        input_frame::SlotId::Away3 => "away_3",
        input_frame::SlotId::Away4 => "away_4",
    }
}

fn input_team_wire(team: input_frame::Team) -> &'static str {
    match team {
        input_frame::Team::Home => "home",
        input_frame::Team::Away => "away",
    }
}

fn text_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a str {
    Value::record_get(entries, name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("validated field {name} missing or not text"))
}

fn opt_text_field<'a>(entries: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    Value::record_get(entries, name).and_then(Value::as_str)
}

fn number_field(entries: &[(String, Value)], name: &str) -> f64 {
    Value::record_get(entries, name)
        .and_then(Value::as_number)
        .unwrap_or_else(|| panic!("validated field {name} missing or not a number"))
}

fn array_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [Value] {
    Value::record_get(entries, name)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("validated field {name} missing or not an array"))
}

fn record_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    Value::record_get(entries, name)
        .and_then(Value::as_record)
        .unwrap_or_else(|| panic!("validated field {name} missing or not a record"))
}

/// Content hash of the authoritative tape, derived only from the tape's own
/// identity, canonical initial boundary, and canonical frame wires. This is a
/// reference to [`crate::input_tape`], not a second frame format.
pub fn tape_content_hash(tape: &InputTape) -> Result<String> {
    input_tape::validate_structure(tape)
        .map_err(|e| format!("gameplay trace tape is not a valid input tape: {e}"))?;
    let identity = input_tape::copy_identity(&tape.identity)
        .map_err(|e| format!("gameplay trace tape is not a valid input tape: {e}"))?;
    let mut parts: Vec<TuplePart> = vec![
        TuplePart::Number(identity.tape_version as f64),
        TuplePart::Number(identity.input_version as f64),
        TuplePart::Number(identity.snapshot_version as f64),
        TuplePart::Text(identity.build.clone()),
        TuplePart::Text(identity.source.clone()),
        TuplePart::Text(identity.content.clone()),
        TuplePart::Text(identity.tuning.clone()),
        TuplePart::Text(identity.config.clone()),
        TuplePart::Text(identity.fixture.clone()),
        TuplePart::Number(identity.seed),
        TuplePart::Number(identity.tick_rate as f64),
        TuplePart::Text(identity.combat.clone().unwrap_or_default()),
        TuplePart::Text(match_snapshot::encode(&tape.initial)),
        TuplePart::Number(tape.frames.len() as f64),
    ];
    for (index, frame) in tape.frames.iter().enumerate() {
        let wire = input_frame::encode(frame)
            .map_err(|e| format!("gameplay trace frame {} is unencodable: {e}", index + 1))?;
        parts.push(TuplePart::Text(wire));
    }
    for hash in &tape.boundary_hashes {
        parts.push(TuplePart::Text(hash.clone()));
    }
    Ok(research_schema::tuple_hash(TAPE_CONTENT_LABEL, &parts))
}

/// Hash of the simulation group only. Participant, session, annotation, and
/// runtime data are structurally outside the preimage.
pub fn simulation_identity_hash(manifest: &Value) -> Result<String> {
    let entries = manifest
        .as_record()
        .ok_or_else(|| "gameplay trace manifest must be a table".to_string())?;
    let simulation = Value::record_get(entries, "simulation")
        .ok_or_else(|| "gameplay_trace_manifest.simulation is required".to_string())?;
    research_schema::content_hash(&simulation_shape(), simulation)
}

/// Derive `trace_id` from a manifest's simulation identity and game instance.
pub fn derive_trace_id(manifest: &Value) -> Result<String> {
    let identity = simulation_identity_hash(manifest)?;
    let entries = manifest
        .as_record()
        .ok_or_else(|| "gameplay trace manifest must be a table".to_string())?;
    let game_instance_id = Value::record_get(entries, "game_instance_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "gameplay_trace_manifest.game_instance_id is required".to_string())?;
    Ok(research_schema::tuple_hash(
        TRACE_ID_LABEL,
        &[
            TuplePart::Text(identity),
            TuplePart::Text(game_instance_id.to_string()),
        ],
    ))
}

/// Validate a trace manifest against [`shape`] plus every cross-field
/// invariant.
pub fn validate(manifest: &Value) -> Result<()> {
    let entries_for_version = manifest
        .as_record()
        .ok_or_else(|| "gameplay trace manifest must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&shape(), manifest)?;
    let entries = manifest.as_record().expect("validated record");
    let simulation = record_field(entries, "simulation");

    if number_field(simulation, "tick_rate") != fixed_clock::TICK_RATE {
        return Err("gameplay_trace_manifest.simulation.tick_rate is unsupported".to_string());
    }
    let first_boundary = number_field(simulation, "first_boundary_tick");
    let last_boundary = number_field(simulation, "last_boundary_tick");
    let frame_count = number_field(simulation, "frame_count");
    if last_boundary != first_boundary + frame_count {
        return Err(
            "gameplay_trace_manifest.simulation boundary range disagrees with frame_count"
                .to_string(),
        );
    }
    let tape_version = number_field(simulation, "tape_version") as i64;
    let combat_identity = opt_text_field(simulation, "combat_identity");
    if tape_version == input_tape::COMBAT_VERSION {
        if combat_identity.is_none() {
            return Err(
                "gameplay_trace_manifest.simulation.combat_identity is required for a combat tape"
                    .to_string(),
            );
        }
    } else if combat_identity.is_some() {
        return Err(
            "gameplay_trace_manifest.simulation.combat_identity is only valid for a combat tape"
                .to_string(),
        );
    }
    let completion = text_field(simulation, "completion");
    if frame_count == 0.0 && completion == "completed" {
        return Err(
            "gameplay_trace_manifest.simulation cannot be completed with no frames".to_string(),
        );
    }
    let has_divergence = Value::record_get(simulation, "divergence").is_some();
    if has_divergence && completion == "completed" {
        return Err(
            "gameplay_trace_manifest.simulation cannot report divergence and completion"
                .to_string(),
        );
    }

    let mut seen_players: Vec<&str> = Vec::new();
    let producers = array_field(simulation, "producers");
    for (loop_index, producer) in producers.iter().enumerate() {
        let index = loop_index as i64 + 1;
        let producer_entries = producer.as_record().expect("validated producer record");
        let expected = input_frame::slot(index).map_err(|_| {
            "gameplay_trace_manifest.simulation.producers has too many slots".to_string()
        })?;
        let slot_text = text_field(producer_entries, "slot");
        let team_text = text_field(producer_entries, "team");
        if slot_text != slot_id_wire(expected.id) || team_text != input_team_wire(expected.team) {
            return Err(format!(
                "gameplay_trace_manifest.simulation.producers.{index} violates canonical slot order"
            ));
        }
        let player_id = text_field(producer_entries, "player_id");
        if seen_players.contains(&player_id) {
            return Err(
                "gameplay_trace_manifest.simulation.producers duplicate a player".to_string(),
            );
        }
        seen_players.push(player_id);
        let producer_kind = text_field(producer_entries, "producer_kind");
        let producer_policy_id = opt_text_field(producer_entries, "producer_policy_id");
        if producer_kind == "human" && producer_policy_id.is_some() {
            return Err(format!(
                "gameplay_trace_manifest.simulation.producers.{index} human slots cannot declare a bot policy"
            ));
        }
        if producer_kind != "human" && producer_policy_id.is_none() {
            return Err(format!(
                "gameplay_trace_manifest.simulation.producers.{index} machine slots must declare a policy id"
            ));
        }
    }

    let mut link_keys: Vec<String> = Vec::new();
    let research_links = array_field(entries, "research_links");
    for (loop_index, link) in research_links.iter().enumerate() {
        let link_entries = link.as_record().expect("validated link record");
        let key = format!(
            "{}/{}",
            text_field(link_entries, "link_kind"),
            text_field(link_entries, "target_id")
        );
        if link_keys.contains(&key) {
            return Err(format!(
                "gameplay_trace_manifest.research_links.{} is duplicated",
                loop_index + 1
            ));
        }
        link_keys.push(key);
    }

    let expected_id = derive_trace_id(manifest)?;
    if text_field(entries, "trace_id") != expected_id {
        return Err(
            "gameplay_trace_manifest.trace_id is not derived from its simulation identity"
                .to_string(),
        );
    }
    Ok(())
}

/// Content hash of a valid trace manifest.
pub fn content_hash(manifest: &Value) -> Result<String> {
    validate(manifest)?;
    research_schema::content_hash(&shape(), manifest)
}

/// Canonical bytes of a valid trace manifest.
pub fn encode(manifest: &Value) -> Result<Vec<u8>> {
    validate(manifest)?;
    research_schema::encode(&shape(), manifest)
}

/// Decode and re-validate a trace manifest.
pub fn decode(bytes: &[u8]) -> Result<Value> {
    let manifest = research_schema::decode(&shape(), bytes)?;
    validate(&manifest)?;
    Ok(manifest)
}

/// Input for one canonical slot's producer diagnostics, supplied to
/// [`from_tape`]. Not part of the wire shape — this is a plain convenience
/// struct, never itself serialized; the manifest [`from_tape`] builds is a
/// [`Value`], exactly like the rest of this module.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchTraceProducerInput {
    /// `"human"`, `"bot"`, or `"replay"`.
    pub producer_kind: String,
    /// Required for machine producers, forbidden for human ones.
    pub producer_policy_id: Option<String>,
}

/// Recorder-owned diagnostics supplied to [`from_tape`]. Not part of the wire
/// shape (see [`ResearchTraceProducerInput`]'s doc comment); `runtime`,
/// `divergence`, and `research_links` are already shape-conformant
/// [`Value`]s because [`from_tape`] copies them verbatim into the built
/// manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchTraceOptions {
    /// The manifest's `game_instance_id`.
    pub game_instance_id: String,
    /// The manifest's `simulation.ruleset_version`.
    pub ruleset_version: i64,
    /// The manifest's `simulation.event_schema_version`.
    pub event_schema_version: i64,
    /// The manifest's `simulation.confirmed_event_stream_hash`.
    pub confirmed_event_stream_hash: String,
    /// The manifest's `simulation.completion`.
    pub completion: String,
    /// Exactly [`input_frame::SLOT_COUNT`] entries, in canonical slot order.
    pub producers: Vec<ResearchTraceProducerInput>,
    /// The manifest's `runtime` record.
    pub runtime: Value,
    /// The manifest's `simulation.divergence` record, if any.
    pub divergence: Option<Value>,
    /// The manifest's `research_links`, if any (defaults to empty).
    pub research_links: Option<Vec<Value>>,
}

/// Build a manifest from an immutable tape plus the diagnostics the recorder
/// owns. The tape is only read: no field of `tape` is written, and no
/// research identifier reaches `tape.identity`.
///
/// There is no runtime "is this the right shape" guard on `options`:
/// `options` is a typed [`ResearchTraceOptions`], not a dynamically-typed
/// value, so such a check would be structurally redundant here (README
/// rule 9).
pub fn from_tape(tape: &InputTape, options: &ResearchTraceOptions) -> Result<Value> {
    let content_hash = tape_content_hash(tape)?;
    let identity = input_tape::copy_identity(&tape.identity)
        .map_err(|e| format!("gameplay trace tape is not a valid input tape: {e}"))?;
    let (initial_state, _initial_combat) = match_snapshot::restore(&tape.initial);
    let first_tick = initial_state.input_tick;

    let mut producers: Vec<Value> = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        let supplied = options
            .producers
            .get((index - 1) as usize)
            .ok_or_else(|| format!("gameplay trace producers.{index} is required"))?;
        let assignment = &identity.ownership.slots[(index - 1) as usize];
        let mut producer_entries: Vec<(String, Value)> = vec![
            (
                "slot".to_string(),
                Value::str(slot_id_wire(assignment.slot)),
            ),
            (
                "team".to_string(),
                Value::str(input_team_wire(assignment.team)),
            ),
            (
                "player_id".to_string(),
                Value::str(assignment.player_id.clone()),
            ),
            (
                "producer_kind".to_string(),
                Value::str(supplied.producer_kind.clone()),
            ),
        ];
        if let Some(policy_id) = &supplied.producer_policy_id {
            producer_entries.push((
                "producer_policy_id".to_string(),
                Value::str(policy_id.clone()),
            ));
        }
        producers.push(Value::Record(producer_entries));
    }

    let mut simulation_entries: Vec<(String, Value)> = vec![
        (
            "tape_version".to_string(),
            Value::Number(identity.tape_version as f64),
        ),
        (
            "input_version".to_string(),
            Value::Number(identity.input_version as f64),
        ),
        (
            "snapshot_version".to_string(),
            Value::Number(identity.snapshot_version as f64),
        ),
        (
            "ruleset_version".to_string(),
            Value::Number(options.ruleset_version as f64),
        ),
        (
            "event_schema_version".to_string(),
            Value::Number(options.event_schema_version as f64),
        ),
    ];
    if let Some(combat) = &identity.combat {
        simulation_entries.push(("combat_identity".to_string(), Value::str(combat.clone())));
    }
    simulation_entries.extend([
        ("build".to_string(), Value::str(identity.build.clone())),
        ("source".to_string(), Value::str(identity.source.clone())),
        ("content".to_string(), Value::str(identity.content.clone())),
        ("tuning".to_string(), Value::str(identity.tuning.clone())),
        ("config".to_string(), Value::str(identity.config.clone())),
        ("fixture".to_string(), Value::str(identity.fixture.clone())),
        ("seed".to_string(), Value::Number(identity.seed)),
        (
            "tick_rate".to_string(),
            Value::Number(identity.tick_rate as f64),
        ),
        (
            "first_boundary_tick".to_string(),
            Value::Number(first_tick as f64),
        ),
        (
            "last_boundary_tick".to_string(),
            Value::Number((first_tick + tape.frames.len() as i64) as f64),
        ),
        (
            "frame_count".to_string(),
            Value::Number(tape.frames.len() as f64),
        ),
        ("tape_content_hash".to_string(), Value::str(content_hash)),
        (
            "initial_boundary_hash".to_string(),
            Value::str(tape.boundary_hashes[0].clone()),
        ),
        (
            "final_boundary_hash".to_string(),
            Value::str(tape.boundary_hashes[tape.boundary_hashes.len() - 1].clone()),
        ),
        (
            "confirmed_event_stream_hash".to_string(),
            Value::str(options.confirmed_event_stream_hash.clone()),
        ),
        (
            "completion".to_string(),
            Value::str(options.completion.clone()),
        ),
        ("producers".to_string(), Value::Array(producers)),
    ]);
    if let Some(divergence) = &options.divergence {
        simulation_entries.push(("divergence".to_string(), divergence.clone()));
    }

    let manifest_entries: Vec<(String, Value)> = vec![
        ("schema_version".to_string(), Value::Number(VERSION as f64)),
        ("manifest_kind".to_string(), Value::str(KIND)),
        ("digest".to_string(), Value::str(research_schema::DIGEST)),
        ("trace_id".to_string(), Value::str("0000000000000000")),
        (
            "game_instance_id".to_string(),
            Value::str(options.game_instance_id.clone()),
        ),
        ("simulation".to_string(), Value::Record(simulation_entries)),
        ("runtime".to_string(), options.runtime.clone()),
        (
            "research_links".to_string(),
            Value::Array(options.research_links.clone().unwrap_or_default()),
        ),
    ];
    let manifest = Value::Record(manifest_entries);
    research_schema::validate(&shape(), &manifest)?;
    let trace_id = derive_trace_id(&manifest)?;
    let mut final_entries = manifest.as_record().expect("just built").to_vec();
    if let Some(entry) = final_entries.iter_mut().find(|(k, _)| k == "trace_id") {
        entry.1 = Value::str(trace_id);
    }
    let manifest = Value::Record(final_entries);
    validate(&manifest)?;
    Ok(manifest)
}

/// The confirmed event stream is scoped by tape content, not by manifest, so
/// it can be built before recorder diagnostics exist. This is the join that
/// proves a manifest and a stream describe the same run.
pub fn validate_against_stream(manifest: &Value, stream: &Value) -> Result<()> {
    validate(manifest)?;
    let entries = manifest.as_record().expect("validated record");
    let simulation = record_field(entries, "simulation");
    let stream_entries = stream
        .as_record()
        .ok_or_else(|| "research event stream must be a table".to_string())?;
    if text_field(stream_entries, "run_scope_id") != text_field(simulation, "tape_content_hash") {
        return Err("research event stream describes another tape".to_string());
    }
    if text_field(stream_entries, "game_instance_id") != text_field(entries, "game_instance_id") {
        return Err("research event stream describes another game instance".to_string());
    }
    if text_field(stream_entries, "stream_hash")
        != text_field(simulation, "confirmed_event_stream_hash")
    {
        return Err(
            "gameplay_trace_manifest.simulation.confirmed_event_stream_hash does not match the stream"
                .to_string(),
        );
    }
    let confirmed_boundary = number_field(stream_entries, "confirmed_boundary");
    if confirmed_boundary > number_field(simulation, "last_boundary_tick") {
        return Err("research event stream confirms past the recorded tape boundary".to_string());
    }
    if confirmed_boundary < number_field(simulation, "first_boundary_tick") {
        return Err("research event stream confirms before the recorded tape boundary".to_string());
    }
    Ok(())
}

/// Attach one more research annotation to an existing manifest. Returns a
/// new manifest: the input is never mutated, the simulation identity is
/// unchanged, and duplicate links fail closed.
pub fn with_research_link(manifest: &Value, link: &Value) -> Result<Value> {
    validate(manifest)?;
    let entries = manifest.as_record().expect("validated record");
    let mut next_entries: Vec<(String, Value)> = entries.to_vec();
    let links_index = next_entries
        .iter()
        .position(|(k, _)| k == "research_links")
        .expect("validated manifest has research_links");
    let mut links = match &next_entries[links_index].1 {
        Value::Array(items) => items.clone(),
        _ => unreachable!("validated array"),
    };
    links.push(link.clone());
    next_entries[links_index].1 = Value::Array(links);
    let next_manifest = Value::Record(next_entries);
    validate(&next_manifest)?;
    Ok(next_manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The self-check the module doc comment refers to: the field groups
    /// are disjoint and cover every declared manifest field. An outside-in
    /// version of this same invariant — "keeps every manifest field in
    /// exactly one identity group" — is asserted separately in
    /// `tests/research_trace.rs`.
    #[test]
    fn field_groups_are_disjoint_and_cover_every_manifest_field() {
        research_schema::assert_disjoint("gameplay_trace_manifest field groups", &field_groups())
            .expect("gameplay trace field groups must be disjoint");
        let shape = shape();
        let declared = shape.fields.expect("record shape has fields");
        for field in &declared {
            let covered = field_groups()
                .iter()
                .any(|(_, members)| members.contains(&field.name.as_str()));
            assert!(covered, "gameplay trace field {} has no group", field.name);
        }
    }
}
