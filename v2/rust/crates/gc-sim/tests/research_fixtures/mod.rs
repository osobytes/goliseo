//! Shared test-support module: ports of `spec/fixtures/short_match_tape.lua`
//! and `spec/fixtures/research/example_package.lua`.
//!
//! Cargo does not compile `tests/<dir>/mod.rs` as its own test binary — only
//! direct `tests/*.rs` files — so this module is `mod research_fixtures;`
//! declared by each consuming test file (`tests/research_trace.rs`,
//! `tests/research_dataset.rs`, `tests/research_package.rs`,
//! `tests/research_canonical.rs`), which Rust resolves to this file
//! automatically as a sibling of `research_fixtures/`.
//!
//! Each consuming binary only exercises part of this module, so
//! `#![allow(dead_code)]` keeps an unused-in-this-binary `pub fn` from
//! failing `-D warnings` under `cargo clippy --workspace --all-targets`.
//!
//! ## `short_match_tape`'s pinned Lua boundary hashes are NOT reused here
//!
//! `spec/fixtures/short_match_tape.lua` asserts its tape's boundary hashes
//! against `EXPECTED_BOUNDARY_HASHES`, four hex strings computed by the
//! *Lua* `sim/match.lua` engine. `v2/tools/lua_reference/` has no vector
//! file for `match`/`input_tape` (only `diagnostics_schema_vectors.txt` and
//! `research_schema_vectors.txt` exist there), so there is no differential
//! harness proving the Rust port of `sim::r#match` produces byte-identical
//! boundary hashes for this fixture. Copying the Lua literals in would
//! therefore be asserting something nobody has verified. [`short_match_tape`]
//! instead builds the same *shape* (same teams, same three frames, same
//! identity fields) and lets the Rust engine compute its own hashes,
//! checked only for internal self-consistency (`boundary_hashes.len() ==
//! frames.len() + 1`) — see `tests/research_trace.rs` for the case that
//! exercises this.

#![allow(dead_code)]

use gc_data::teams;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputSampleOptions};
use gc_sim::input_tape::{self, InputTape, InputTapeIdentity};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{self, MatchEvent, MatchEventKind, MatchSnapshot, PitchSize};
use gc_sim::research_dataset;
use gc_sim::research_features;
use gc_sim::research_schema::{self, TuplePart, Value};
use gc_sim::research_timeline;
use gc_sim::research_trace::{self, ResearchTraceOptions, ResearchTraceProducerInput};
use gc_sim::rollback_events;
use gc_sim::tuning::Tuning;

// ---------------------------------------------------------------------
// spec/fixtures/short_match_tape.lua
// ---------------------------------------------------------------------

/// Build the checked-in short match tape: `nebula` vs `orion`, seed 38, a
/// 2.5-tick match duration, three frames (the second drives `home_1`
/// forward). Ported from `short_match_tape.make`.
#[must_use]
pub fn short_match_tape() -> InputTape {
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(fixed_clock::TICK_SECONDS * 2.5),
        max_goals: Some(3),
        seed: Some(38.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    // `sim::match::new` leaves `marks.home`/`.away` empty (they are sized
    // during `step`); `match_snapshot::capture`'s validation requires them
    // sized to `players.len()` immediately. See
    // `tests/input_tape_differential.rs`'s identical normalization and
    // `input_tape.rs`'s `normalize_marks` doc comment for the full report.
    let player_count = state.players.len();
    state.marks.home.resize(player_count, None);
    state.marks.away.resize(player_count, None);
    let initial = match_snapshot::capture(&state, None);

    let mut frames = vec![
        input_frame::neutral(0).expect("tick 0 frame"),
        input_frame::neutral(1).expect("tick 1 frame"),
        input_frame::neutral(2).expect("tick 2 frame"),
    ];
    frames[1].slots[0] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(input_frame::MOVE_SCALE),
        ..Default::default()
    })
    .expect("driven sample");

    let tune = Tuning::new();
    let identity = InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "spec-build".to_string(),
        source: "197ce23264abff1d6ac0c71e9b313ee820814f7e".to_string(),
        content: "showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: "field=960x540;duration_ticks=2.5;max_goals=3".to_string(),
        fixture: "nebula-v-orion;balanced-v-balanced".to_string(),
        seed: 38.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: None,
    };
    let tape = input_tape::new(&identity, &initial, &frames, &tune).expect("tape constructs");
    assert_eq!(
        tape.boundary_hashes.len(),
        tape.frames.len() + 1,
        "short match tape must carry one boundary hash per frame plus the initial boundary"
    );
    tape
}

// ---------------------------------------------------------------------
// spec/fixtures/research/example_package.lua
// ---------------------------------------------------------------------

/// `example_package.PARTICIPANT_ID`.
pub const PARTICIPANT_ID: &str = "p-7f3a9c21b8e45d06";
/// `example_package.SECOND_PARTICIPANT_ID`.
pub const SECOND_PARTICIPANT_ID: &str = "p-2b8e45d067f3a9c1";
/// `example_package.THIRD_PARTICIPANT_ID`.
pub const THIRD_PARTICIPANT_ID: &str = "p-45d067f3a9c12b8e";
/// `example_package.SESSION_ID`.
pub const SESSION_ID: &str = "s-1c9d4f7a3b6e2058";
/// `example_package.SECOND_SESSION_ID`.
pub const SECOND_SESSION_ID: &str = "s-6e2058f7a3b91c9d";
/// `example_package.THIRD_SESSION_ID`.
pub const THIRD_SESSION_ID: &str = "s-a3b91c9d6e2058f7";
/// `example_package.WITHDRAWN_SESSION_ID`.
pub const WITHDRAWN_SESSION_ID: &str = "s-58f7a3b6e20591c9";
/// `example_package.INTERRUPTED_SESSION_ID`.
pub const INTERRUPTED_SESSION_ID: &str = "s-91c9d4f7a3b6e205";
/// `example_package.GAME_INSTANCE_ID`.
pub const GAME_INSTANCE_ID: &str = "gi-0001";

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn get_record<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    Value::record_get(entries, name)
        .and_then(Value::as_record)
        .unwrap_or_else(|| panic!("fixture field {name} missing or not a record"))
}

fn set_field(entries: &[(String, Value)], name: &str, value: Value) -> Vec<(String, Value)> {
    let mut result: Vec<(String, Value)> = entries.to_vec();
    if let Some(entry) = result.iter_mut().find(|(k, _)| k == name) {
        entry.1 = value;
    } else {
        result.push((name.to_string(), value));
    }
    result
}

fn minimal_event(kind: MatchEventKind, x: f64, y: f64) -> MatchEvent {
    MatchEvent {
        kind,
        x,
        y,
        player: None,
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    }
}

/// Ported from `example_package`'s local `next_snapshot`: restore, advance
/// the input tick, replace the discrete events, and tick down the clock.
fn next_snapshot(before: &MatchSnapshot, events: Vec<MatchEvent>) -> MatchSnapshot {
    let (mut state, combat) = match_snapshot::restore(before);
    state.input_tick += 1;
    state.events = events;
    state.time_left = (state.time_left - 1.0 / 60.0).max(0.0);
    match_snapshot::capture(&state, combat.as_ref())
}

/// Ported from `example_package`'s local `step_input`. `RollbackEventTickOutput`
/// carries no `input` field in this port (see `rollback_events.rs`'s module
/// doc: it is part of the Lua shape but never read), so the Lua original's
/// `input_record` helper has no Rust equivalent here.
fn step_input(snapshot: &MatchSnapshot) -> rollback_events::RollbackEventStepInput {
    let (state, _combat) = match_snapshot::restore(snapshot);
    let tick = state.input_tick - 1;
    rollback_events::RollbackEventStepInput {
        output: rollback_events::RollbackEventTickOutput {
            tick,
            start_boundary: tick,
            end_boundary: tick + 1,
            events: state.events.clone(),
            combat_events: None,
            state: rollback_events::RollbackOutputStateView {
                score: state.score,
                time_left: state.time_left,
                finished: state.finished,
            },
            finished: state.finished,
        },
        snapshot: snapshot.clone(),
    }
}

fn convert_event(event: &rollback_events::RollbackWrappedEvent) -> research_timeline::RollbackEvent {
    research_timeline::RollbackEvent {
        id: event.id.clone(),
        domain: event.domain.clone(),
        tick: event.tick,
        ordinal: event.ordinal,
    }
}

fn convert_diff(diff: &rollback_events::RollbackEventDiff) -> research_timeline::RollbackEventDiff {
    research_timeline::RollbackEventDiff {
        added: diff.added.iter().map(convert_event).collect(),
        replaced: diff
            .replaced
            .iter()
            .map(|r| research_timeline::RollbackEventReplacement {
                before: convert_event(&r.before),
                after: convert_event(&r.after),
            })
            .collect(),
        revoked: diff.revoked.iter().map(convert_event).collect(),
    }
}

fn convert_step(step: &rollback_events::RollbackEventStep) -> research_timeline::RollbackEventStep {
    research_timeline::RollbackEventStep {
        tick: step.tick,
        start_boundary: step.start_boundary,
        end_boundary: step.end_boundary,
        match_events: step.match_events.iter().map(convert_event).collect(),
        combat_events: step
            .combat_events
            .as_ref()
            .map(|events| events.iter().map(convert_event).collect())
            .unwrap_or_default(),
        lifecycle_events: step.lifecycle_events.iter().map(convert_event).collect(),
    }
}

/// Ported from `example_package.rollback_sequence`: a real rollback
/// sequence over the short match tape — tick 0 confirmed as a touch, tick 1
/// first predicted as a tackle and then corrected to a pass. The tackle is
/// therefore an event that presentation saw and research must never see.
pub struct RollbackSequence {
    /// The tape the sequence replays.
    pub tape: InputTape,
    /// Diff from applying tick 0.
    pub first_diff: rollback_events::RollbackEventDiff,
    /// Diff from the first (tackle) prediction at tick 1.
    pub speculative_diff: rollback_events::RollbackEventDiff,
    /// Diff from correcting tick 1 to a pass.
    pub correction_diff: rollback_events::RollbackEventDiff,
    /// Steps confirmed through tick 1.
    pub confirmed: Vec<rollback_events::RollbackEventStep>,
    /// The rollback event id the correction revoked (the tackle).
    pub revoked_rollback_event_id: String,
}

/// Ported from `example_package.rollback_sequence`.
#[must_use]
pub fn rollback_sequence() -> RollbackSequence {
    let tape = short_match_tape();
    let initial = tape.initial.clone();
    let mut timeline = rollback_events::new(&initial, None);

    let first = next_snapshot(&initial, vec![minimal_event(MatchEventKind::Touch, 100.0, 100.0)]);
    let first_diff = rollback_events::apply(&mut timeline, 0, 0, &[step_input(&first)])
        .expect("tick 0 applies");

    let speculative =
        next_snapshot(&first, vec![minimal_event(MatchEventKind::Tackle, 120.0, 110.0)]);
    let speculative_diff =
        rollback_events::apply(&mut timeline, 1, 1, &[step_input(&speculative)])
            .expect("speculative tick 1 applies");

    let corrected = next_snapshot(&first, vec![minimal_event(MatchEventKind::Pass, 130.0, 115.0)]);
    let correction_diff = rollback_events::apply(&mut timeline, 1, 1, &[step_input(&corrected)])
        .expect("corrected tick 1 applies");
    assert_eq!(
        correction_diff.revoked.len(),
        1,
        "example package expects one revoked event"
    );

    let confirmed = rollback_events::confirm(&mut timeline, 1);
    let revoked_rollback_event_id = correction_diff.revoked[0].id.clone();

    RollbackSequence {
        tape,
        first_diff,
        speculative_diff,
        correction_diff,
        confirmed,
        revoked_rollback_event_id,
    }
}

/// Ported from `example_package.trace_manifest`.
pub fn trace_manifest(
    tape: &InputTape,
    confirmed_event_stream_hash: &str,
    completion: Option<&str>,
) -> research_schema::Result<Value> {
    let mut producers = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        if index == 1 {
            producers.push(ResearchTraceProducerInput {
                producer_kind: "human".to_string(),
                producer_policy_id: None,
            });
        } else {
            producers.push(ResearchTraceProducerInput {
                producer_kind: "bot".to_string(),
                producer_policy_id: Some("bot-baseline-v1".to_string()),
            });
        }
    }
    let runtime = record(vec![
        ("platform", Value::str("linux-native")),
        ("renderer", Value::str("opengl")),
        ("render_hz", Value::Number(60.0)),
        ("render_hz_mode", Value::str("fixed")),
        ("input_device", Value::str("keyboard")),
        ("mean_frame_ms", Value::Number(8.2)),
        ("p99_frame_ms", Value::Number(14.5)),
        ("dropped_frame_count", Value::Number(3.0)),
        ("pause_count", Value::Number(1.0)),
        ("goal_replay_count", Value::Number(0.0)),
        ("rollback_count", Value::Number(1.0)),
        ("max_rollback_ticks", Value::Number(2.0)),
        ("raw_device_event_policy", Value::str("minimized_diagnostic")),
        ("raw_device_event_clock", Value::str("wall_clock_monotonic")),
    ]);
    let options = ResearchTraceOptions {
        game_instance_id: GAME_INSTANCE_ID.to_string(),
        ruleset_version: 1,
        event_schema_version: 1,
        confirmed_event_stream_hash: confirmed_event_stream_hash.to_string(),
        completion: completion.unwrap_or("completed").to_string(),
        producers,
        runtime,
        divergence: None,
        research_links: None,
    };
    research_trace::from_tape(tape, &options)
}

/// The gameplay half of the package, ported from `example_package.gameplay`:
/// a real tape, a real confirmed event stream whose second tick was
/// corrected by rollback, and a trace manifest that references both.
pub struct Gameplay {
    /// The underlying tape.
    pub tape: InputTape,
    /// The confirmed `research_event_stream/v1` [`Value`].
    pub stream: Value,
    /// The `gameplay_trace_manifest/v1` [`Value`].
    pub manifest: Value,
    /// The rollback event id the correction revoked (the tackle).
    pub revoked_rollback_event_id: String,
}

/// Ported from `example_package.gameplay`.
#[must_use]
pub fn gameplay(completion: Option<&str>) -> Gameplay {
    let sequence = rollback_sequence();

    // The event stream is scoped by tape content, which is knowable before
    // any recorder diagnostics exist. That ordering is what keeps the trace
    // manifest (which references the stream hash) free of a circular
    // dependency.
    let run_scope_id =
        research_trace::tape_content_hash(&sequence.tape).expect("tape content hash");
    let mut stream_builder = research_timeline::new(run_scope_id, GAME_INSTANCE_ID)
        .expect("research timeline constructs");
    research_timeline::observe_diff(&mut stream_builder, &convert_diff(&sequence.speculative_diff))
        .expect("observe speculative diff");
    research_timeline::observe_diff(&mut stream_builder, &convert_diff(&sequence.correction_diff))
        .expect("observe correction diff");
    let confirmed_steps: Vec<research_timeline::RollbackEventStep> =
        sequence.confirmed.iter().map(convert_step).collect();
    research_timeline::confirm(&mut stream_builder, &confirmed_steps).expect("confirm steps");
    let stream = research_timeline::export(&stream_builder).expect("stream exports");
    let stream_entries = stream.as_record().expect("stream is a record");
    let stream_hash = Value::record_get(stream_entries, "stream_hash")
        .and_then(Value::as_str)
        .expect("stream carries a stream_hash")
        .to_string();

    let manifest = trace_manifest(&sequence.tape, &stream_hash, completion)
        .expect("trace manifest builds");
    research_trace::validate_against_stream(&manifest, &stream)
        .expect("manifest validates against its stream");

    Gameplay {
        tape: sequence.tape,
        stream,
        manifest,
        revoked_rollback_event_id: sequence.revoked_rollback_event_id,
    }
}

/// Optional field overrides for [`base_envelope`].
#[derive(Default)]
pub struct EnvelopeOverrides {
    /// Overrides `session_id`.
    pub session_id: Option<String>,
    /// Overrides `participant_id`.
    pub participant_id: Option<String>,
    /// Overrides `condition_id`.
    pub condition_id: Option<String>,
    /// Overrides `assignment.build_id`.
    pub build_id: Option<String>,
    /// Overrides `assignment.tuning_config_id`.
    pub tuning_config_id: Option<String>,
    /// Overrides `agreement.model_use_covered`.
    pub model_use_covered: Option<bool>,
}

/// Ported from `example_package`'s local `base_envelope`.
fn base_envelope(trace_id: &str, overrides: Option<&EnvelopeOverrides>) -> Value {
    let session_id = overrides
        .and_then(|o| o.session_id.clone())
        .unwrap_or_else(|| SESSION_ID.to_string());
    let participant_id = overrides
        .and_then(|o| o.participant_id.clone())
        .unwrap_or_else(|| PARTICIPANT_ID.to_string());
    let condition_id = overrides
        .and_then(|o| o.condition_id.clone())
        .unwrap_or_else(|| "condition-b-combat-on".to_string());
    let build_id = overrides
        .and_then(|o| o.build_id.clone())
        .unwrap_or_else(|| "spec-build".to_string());
    let tuning_config_id = overrides
        .and_then(|o| o.tuning_config_id.clone())
        .unwrap_or_else(|| "tuning-default-v1".to_string());
    let model_use_covered = overrides.and_then(|o| o.model_use_covered).unwrap_or(true);

    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_session_envelope")),
        ("digest", Value::str(research_schema::DIGEST)),
        ("session_id", Value::str(session_id)),
        ("participant_id", Value::str(participant_id)),
        ("study_id", Value::str("m11-combat-fun")),
        ("protocol_version", Value::str("m11-protocol-v3")),
        ("cohort_id", Value::str("confirmatory-wave-1")),
        ("recruitment_channel", Value::str("local-football-club")),
        ("block_id", Value::str("block-b")),
        ("trial_id", Value::str("trial-2")),
        ("condition_id", Value::str(condition_id)),
        ("condition_order", Value::Number(2.0)),
        ("sequence_label", Value::str("ab")),
        ("counterbalancing_cell", Value::str("cell-ab-home-seed38")),
        (
            "assignment",
            record(vec![
                ("build_id", Value::str(build_id)),
                ("tuning_config_id", Value::str(tuning_config_id)),
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
                ("model_use_covered", Value::Bool(model_use_covered)),
            ]),
        ),
        (
            "trace_links",
            Value::Array(vec![record(vec![
                ("trace_id", Value::str(trace_id)),
                ("game_instance_id", Value::str(GAME_INSTANCE_ID)),
                ("role", Value::str("condition_block")),
            ])]),
        ),
    ])
}

/// Optional field overrides for [`dataset_sessions`].
#[derive(Default)]
pub struct DatasetSessionsOverrides {
    /// Whether the third (candidate-build) session's agreement covers model
    /// use.
    pub third_model_use_covered: Option<bool>,
}

/// Ported from `example_package.dataset_sessions`: the session envelopes
/// that back [`dataset`]. Source rows are derived from these, so a dataset
/// can never claim an agreement or a model-use permission that no envelope
/// actually recorded.
#[must_use]
pub fn dataset_sessions(
    manifest: &Value,
    overrides: Option<&DatasetSessionsOverrides>,
) -> Vec<Value> {
    let manifest_entries = manifest.as_record().expect("manifest is a record");
    let trace_id = Value::record_get(manifest_entries, "trace_id")
        .and_then(Value::as_str)
        .expect("manifest has a trace_id");
    vec![
        base_envelope(trace_id, None),
        base_envelope(
            &research_schema::tuple_hash(
                "example-trace/v1",
                &[TuplePart::Text("second".to_string())],
            ),
            Some(&EnvelopeOverrides {
                session_id: Some(SECOND_SESSION_ID.to_string()),
                participant_id: Some(SECOND_PARTICIPANT_ID.to_string()),
                condition_id: Some("condition-a-combat-off".to_string()),
                ..Default::default()
            }),
        ),
        base_envelope(
            &research_schema::tuple_hash(
                "example-trace/v1",
                &[TuplePart::Text("third".to_string())],
            ),
            Some(&EnvelopeOverrides {
                session_id: Some(THIRD_SESSION_ID.to_string()),
                participant_id: Some(THIRD_PARTICIPANT_ID.to_string()),
                build_id: Some("spec-build-next".to_string()),
                tuning_config_id: Some("tuning-candidate-v2".to_string()),
                model_use_covered: overrides.and_then(|o| o.third_model_use_covered),
                ..Default::default()
            }),
        ),
    ]
}

/// Ported from `example_package`'s local `source_row`: one dataset source
/// row derived from the session envelope it came from.
fn source_row(envelope: &Value, trace_manifest_hash: &str) -> Value {
    let entries = envelope.as_record().expect("envelope is a record");
    let trace_links = Value::record_get(entries, "trace_links")
        .and_then(Value::as_array)
        .expect("envelope has trace_links");
    let link_entries = trace_links[0].as_record().expect("link is a record");
    let trace_id = Value::record_get(link_entries, "trace_id")
        .and_then(Value::as_str)
        .expect("link has a trace_id");
    let assignment = get_record(entries, "assignment");
    let agreement = get_record(entries, "agreement");

    record(vec![
        ("trace_id", Value::str(trace_id)),
        ("trace_manifest_hash", Value::str(trace_manifest_hash)),
        (
            "session_id",
            Value::str(
                Value::record_get(entries, "session_id")
                    .and_then(Value::as_str)
                    .expect("envelope has a session_id"),
            ),
        ),
        (
            "participant_id",
            Value::str(
                Value::record_get(entries, "participant_id")
                    .and_then(Value::as_str)
                    .expect("envelope has a participant_id"),
            ),
        ),
        (
            "condition_id",
            Value::str(
                Value::record_get(entries, "condition_id")
                    .and_then(Value::as_str)
                    .expect("envelope has a condition_id"),
            ),
        ),
        (
            "build_id",
            Value::str(
                Value::record_get(assignment, "build_id")
                    .and_then(Value::as_str)
                    .expect("assignment has a build_id"),
            ),
        ),
        (
            "tuning_config_id",
            Value::str(
                Value::record_get(assignment, "tuning_config_id")
                    .and_then(Value::as_str)
                    .expect("assignment has a tuning_config_id"),
            ),
        ),
        ("excluded", Value::Bool(false)),
        (
            "agreement_version",
            Value::str(
                Value::record_get(agreement, "agreement_version")
                    .and_then(Value::as_str)
                    .expect("agreement has an agreement_version"),
            ),
        ),
        (
            "model_use_covered",
            Value::Bool(
                Value::record_get(agreement, "model_use_covered")
                    .and_then(Value::as_bool)
                    .expect("agreement has model_use_covered"),
            ),
        ),
    ])
}

/// Ported from `example_package.completed_session`.
#[must_use]
pub fn completed_session(trace_id: &str) -> Value {
    base_envelope(trace_id, None)
}

/// Ported from `example_package.interrupted_session`.
#[must_use]
pub fn interrupted_session(trace_id: &str) -> Value {
    let base = base_envelope(trace_id, None);
    let mut entries = base.as_record().expect("record").to_vec();
    entries = set_field(&entries, "session_id", Value::str(INTERRUPTED_SESSION_ID));
    let mut lifecycle = get_record(&entries, "lifecycle").to_vec();
    lifecycle = set_field(&lifecycle, "status", Value::str("interrupted"));
    lifecycle = set_field(&lifecycle, "ended_wall_clock_ms", Value::Number(420_000.0));
    lifecycle = set_field(
        &lifecycle,
        "interruptions",
        Value::Array(vec![record(vec![
            ("kind", Value::str("technical_failure")),
            ("wall_clock_ms", Value::Number(410_000.0)),
            ("duration_ms", Value::Number(9000.0)),
            ("canonical_tick", Value::Number(2.0)),
            ("reason_code", Value::str("technical_failure")),
        ])]),
    );
    lifecycle = set_field(
        &lifecycle,
        "missingness",
        Value::Array(vec![record(vec![
            ("target_id", Value::str("pxi_enjoyment_addon")),
            ("target_kind", Value::str("instrument")),
            ("reason_code", Value::str("instrument_not_administered")),
        ])]),
    );
    entries = set_field(&entries, "lifecycle", Value::Record(lifecycle));
    Value::Record(entries)
}

/// Ported from `example_package.withdrawn_session`.
#[must_use]
pub fn withdrawn_session() -> Value {
    let base = base_envelope("0000000000000000", None);
    let mut entries = base.as_record().expect("record").to_vec();
    entries = set_field(&entries, "session_id", Value::str(WITHDRAWN_SESSION_ID));
    let mut lifecycle = get_record(&entries, "lifecycle").to_vec();
    lifecycle = set_field(&lifecycle, "status", Value::str("withdrawn"));
    lifecycle = set_field(&lifecycle, "ended_wall_clock_ms", Value::Number(500_000.0));
    lifecycle = set_field(
        &lifecycle,
        "missingness",
        Value::Array(vec![record(vec![
            ("target_id", Value::str("session-payload")),
            ("target_kind", Value::str("trace")),
            ("reason_code", Value::str("participant_withdrew")),
        ])]),
    );
    entries = set_field(&entries, "lifecycle", Value::Record(lifecycle));
    entries = set_field(&entries, "trace_links", Value::Array(vec![]));
    entries = set_field(&entries, "tombstone_id", Value::str("tombstone-4d7c1a9e"));
    Value::Record(entries)
}

/// Ported from `example_package.withdrawal_tombstone`.
#[must_use]
pub fn withdrawal_tombstone(revoked_payload_hashes: Option<Vec<String>>) -> Value {
    let hashes = revoked_payload_hashes.unwrap_or_else(|| vec!["1111111111111111".to_string()]);
    record(vec![
        ("schema_version", Value::Number(1.0)),
        (
            "manifest_kind",
            Value::str("research_withdrawal_tombstone"),
        ),
        ("digest", Value::str(research_schema::DIGEST)),
        ("tombstone_id", Value::str("tombstone-4d7c1a9e")),
        ("session_id", Value::str(WITHDRAWN_SESSION_ID)),
        ("participant_id", Value::str(PARTICIPANT_ID)),
        ("request_wall_clock_ms", Value::Number(1_200_000.0)),
        (
            "revoked_payload_hashes",
            Value::Array(hashes.into_iter().map(Value::str).collect()),
        ),
        ("rebuild_required", Value::Bool(true)),
    ])
}

/// Optional field overrides for [`enjoyment_responses`].
#[derive(Default)]
pub struct EnjoymentResponsesOverrides {
    /// Overrides `response_set_id`.
    pub response_set_id: Option<String>,
    /// Overrides `session_id`.
    pub session_id: Option<String>,
    /// Overrides `timing.trace_id`.
    pub trace_id: Option<String>,
    /// When set, the third item (`pxi_enjoy_3`) becomes missing with this
    /// reason instead of answered, and `scores` becomes empty.
    pub missing_item: Option<String>,
}

/// Ported from `example_package.enjoyment_responses`.
#[must_use]
pub fn enjoyment_responses(overrides: Option<&EnjoymentResponsesOverrides>) -> Value {
    let mut responses = vec![
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
    ];
    let mut scores = vec![record(vec![
        ("construct", Value::str("enjoyment")),
        ("score", Value::Number(2.0)),
        ("rule", Value::str("mean_all_items")),
        ("item_count", Value::Number(3.0)),
    ])];

    if let Some(missing_item) = overrides.and_then(|o| o.missing_item.clone()) {
        responses[2] = record(vec![
            ("item_id", Value::str("pxi_enjoy_3")),
            ("missing_reason", Value::str(missing_item)),
            ("response_ms", Value::Number(8000.0)),
        ]);
        scores = Vec::new();
    }

    let response_set_id = overrides
        .and_then(|o| o.response_set_id.clone())
        .unwrap_or_else(|| "rs-pxi-enjoyment-b-1".to_string());
    let session_id = overrides
        .and_then(|o| o.session_id.clone())
        .unwrap_or_else(|| SESSION_ID.to_string());
    let trace_id = overrides.and_then(|o| o.trace_id.clone());

    let mut timing_entries = vec![
        ("relative_to_play", Value::str("post_block")),
        ("offset_ms", Value::Number(4200.0)),
        ("canonical_boundary_tick", Value::Number(3.0)),
    ];
    if let Some(trace_id) = trace_id {
        timing_entries.push(("trace_id", Value::str(trace_id)));
    }

    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_response_set")),
        ("digest", Value::str(research_schema::DIGEST)),
        ("response_set_id", Value::str(response_set_id)),
        ("session_id", Value::str(session_id)),
        ("participant_id", Value::str(PARTICIPANT_ID)),
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
        ("timing", record(timing_entries)),
        (
            "presentation_order",
            Value::Array(vec![
                Value::str("pxi_enjoy_2"),
                Value::str("pxi_enjoy_1"),
                Value::str("pxi_enjoy_3"),
            ]),
        ),
        ("randomized_presentation", Value::Bool(true)),
        ("responses", Value::Array(responses)),
        ("scores", Value::Array(scores)),
    ])
}

/// Ported from `example_package.participant_annotations`.
#[must_use]
pub fn participant_annotations(run_scope_id: &str, event_id: Option<&str>) -> Value {
    let mut annotation_entries = vec![
        ("annotation_id", Value::str("an-1")),
        ("canonical_tick", Value::Number(1.0)),
        ("boundary_source", Value::str("wall_clock_mapped")),
        ("mapping_error_ms", Value::Number(4.5)),
        ("wall_clock_ms", Value::Number(1030.0)),
    ];
    if let Some(event_id) = event_id {
        annotation_entries.push(("event_id", Value::str(event_id)));
    }
    annotation_entries.extend([
        ("code_id", Value::str("felt-unfair")),
        ("confidence", Value::Number(0.6)),
        ("free_text", Value::str("I did not see the tackle coming")),
    ]);
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_annotation_set")),
        ("digest", Value::str(research_schema::DIGEST)),
        ("annotation_set_id", Value::str("as-participant-debrief-1")),
        ("run_scope_id", Value::str(run_scope_id)),
        ("session_id", Value::str(SESSION_ID)),
        ("author_role", Value::str("participant")),
        ("agreement_version", Value::str("playtest-agreement-v3")),
        ("coding_scheme_version", Value::str("debrief-codes-v2")),
        ("annotations", Value::Array(vec![record(annotation_entries)])),
    ])
}

/// Ported from `example_package.researcher_annotations`.
#[must_use]
pub fn researcher_annotations(run_scope_id: &str) -> Value {
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_annotation_set")),
        ("digest", Value::str(research_schema::DIGEST)),
        ("annotation_set_id", Value::str("as-researcher-coding-1")),
        ("run_scope_id", Value::str(run_scope_id)),
        ("session_id", Value::str(SESSION_ID)),
        ("author_role", Value::str("researcher")),
        ("agreement_version", Value::str("playtest-agreement-v3")),
        ("coding_scheme_version", Value::str("debrief-codes-v2")),
        (
            "annotations",
            Value::Array(vec![
                record(vec![
                    ("annotation_id", Value::str("an-r1")),
                    ("canonical_tick", Value::Number(1.0)),
                    ("boundary_source", Value::str("canonical_tick")),
                    ("mapping_error_ms", Value::Number(0.0)),
                    ("code_id", Value::str("counterplay-unreadable")),
                    ("confidence", Value::Number(0.4)),
                    ("disagreement_group", Value::str("dg-tackle-readability")),
                ]),
                record(vec![
                    ("annotation_id", Value::str("an-r2")),
                    ("canonical_tick", Value::Number(0.0)),
                    ("boundary_source", Value::str("render_frame_mapped")),
                    ("mapping_error_ms", Value::Number(-6.25)),
                    ("wall_clock_ms", Value::Number(1002.0)),
                    ("code_id", Value::str("cue-missed")),
                ]),
            ]),
        ),
    ])
}

fn default_feature_versions() -> Vec<Value> {
    vec![
        record(vec![
            ("feature_id", Value::str("pxi_enjoyment_addon.enjoyment")),
            ("version", Value::Number(1.0)),
            ("aggregation_level", Value::str("condition_block")),
        ]),
        record(vec![
            ("feature_id", Value::str("soccer_shape_proxy_score")),
            ("version", Value::Number(1.0)),
            ("aggregation_level", Value::str("match")),
        ]),
    ]
}

fn default_splits() -> Vec<Value> {
    vec![
        record(vec![
            ("split_id", Value::str("split-participant-holdout")),
            ("grouping", Value::str("participant")),
            (
                "folds",
                Value::Array(vec![
                    record(vec![
                        ("fold_id", Value::str("train")),
                        ("role", Value::str("train")),
                        (
                            "participant_ids",
                            Value::Array(vec![
                                Value::str(PARTICIPANT_ID),
                                Value::str(SECOND_PARTICIPANT_ID),
                            ]),
                        ),
                        ("build_ids", Value::Array(vec![])),
                    ]),
                    record(vec![
                        ("fold_id", Value::str("test")),
                        ("role", Value::str("test")),
                        (
                            "participant_ids",
                            Value::Array(vec![Value::str(THIRD_PARTICIPANT_ID)]),
                        ),
                        ("build_ids", Value::Array(vec![])),
                    ]),
                ]),
            ),
        ]),
        record(vec![
            ("split_id", Value::str("split-build-holdout")),
            ("grouping", Value::str("build")),
            (
                "folds",
                Value::Array(vec![
                    record(vec![
                        ("fold_id", Value::str("train")),
                        ("role", Value::str("train")),
                        ("participant_ids", Value::Array(vec![])),
                        ("build_ids", Value::Array(vec![Value::str("spec-build")])),
                    ]),
                    record(vec![
                        ("fold_id", Value::str("holdout")),
                        ("role", Value::str("holdout")),
                        ("participant_ids", Value::Array(vec![])),
                        (
                            "build_ids",
                            Value::Array(vec![Value::str("spec-build-next")]),
                        ),
                    ]),
                ]),
            ),
        ]),
    ]
}

/// Optional field overrides for [`dataset`].
#[derive(Default)]
pub struct DatasetOverrides {
    /// Overrides `dataset_id`.
    pub dataset_id: Option<String>,
    /// Overrides `parent_dataset_hash`.
    pub parent_dataset_hash: Option<String>,
    /// Overrides `purpose`.
    pub purpose: Option<String>,
    /// Overrides `usage.model_use_covered` (Lua: `overrides.model_use_covered
    /// ~= false`, i.e. anything but an explicit `false` means covered).
    pub model_use_covered: Option<bool>,
    /// Overrides `feature_versions`.
    pub feature_versions: Option<Vec<Value>>,
    /// Overrides `transformations`.
    pub transformations: Option<Vec<Value>>,
    /// Overrides `splits`.
    pub splits: Option<Vec<Value>>,
    /// Forwarded to [`dataset_sessions`].
    pub third_model_use_covered: Option<bool>,
}

/// Ported from `example_package.dataset`.
pub fn dataset(
    manifest: &Value,
    overrides: Option<&DatasetOverrides>,
) -> research_schema::Result<Value> {
    let manifest_hash = research_trace::content_hash(manifest)?;
    let session_overrides = overrides.map(|o| DatasetSessionsOverrides {
        third_model_use_covered: o.third_model_use_covered,
    });
    let envelopes = dataset_sessions(manifest, session_overrides.as_ref());
    let manifest_hashes = [
        manifest_hash,
        research_schema::tuple_hash(
            "example-manifest/v1",
            &[TuplePart::Text("second".to_string())],
        ),
        research_schema::tuple_hash(
            "example-manifest/v1",
            &[TuplePart::Text("third".to_string())],
        ),
    ];
    let sources: Vec<Value> = envelopes
        .iter()
        .zip(manifest_hashes.iter())
        .map(|(envelope, hash)| source_row(envelope, hash))
        .collect();

    let splits = overrides
        .and_then(|o| o.splits.clone())
        .unwrap_or_else(default_splits);
    let feature_versions = overrides
        .and_then(|o| o.feature_versions.clone())
        .unwrap_or_else(default_feature_versions);
    let transformations = overrides.and_then(|o| o.transformations.clone()).unwrap_or_default();
    let dataset_id = overrides
        .and_then(|o| o.dataset_id.clone())
        .unwrap_or_else(|| "ds-m11-enjoyment-v1".to_string());
    let purpose = overrides
        .and_then(|o| o.purpose.clone())
        .unwrap_or_else(|| "analysis".to_string());
    let model_use_covered = overrides.and_then(|o| o.model_use_covered).unwrap_or(true);
    let parent_dataset_hash = overrides.and_then(|o| o.parent_dataset_hash.clone());

    let mut manifest_entries: Vec<(String, Value)> = vec![
        ("schema_version".to_string(), Value::Number(1.0)),
        (
            "manifest_kind".to_string(),
            Value::str("research_dataset_manifest"),
        ),
        ("digest".to_string(), Value::str(research_schema::DIGEST)),
        ("dataset_id".to_string(), Value::str(dataset_id)),
        ("dataset_version".to_string(), Value::Number(1.0)),
    ];
    if let Some(parent) = parent_dataset_hash {
        manifest_entries.push(("parent_dataset_hash".to_string(), Value::str(parent)));
    }
    manifest_entries.extend([
        (
            "created_wall_clock_ms".to_string(),
            Value::Number(1_700_000_000_000.0),
        ),
        ("purpose".to_string(), Value::str(purpose)),
        (
            "usage".to_string(),
            record(vec![
                (
                    "agreement_versions",
                    Value::Array(vec![Value::str("playtest-agreement-v3")]),
                ),
                ("model_use_covered", Value::Bool(model_use_covered)),
            ]),
        ),
        (
            "extraction".to_string(),
            record(vec![
                (
                    "extraction_commit",
                    Value::str("0".repeat(40)),
                ),
                ("extraction_config_id", Value::str("extraction-config-v1")),
                (
                    "feature_registry_hash",
                    Value::str(research_features::registry_hash()),
                ),
            ]),
        ),
        (
            "feature_versions".to_string(),
            Value::Array(feature_versions),
        ),
        ("sources".to_string(), Value::Array(sources)),
        (
            "transformations".to_string(),
            Value::Array(transformations),
        ),
        ("splits".to_string(), Value::Array(splits)),
        ("dataset_hash".to_string(), Value::str("0000000000000000")),
    ]);
    research_dataset::seal(&Value::Record(manifest_entries))
}
