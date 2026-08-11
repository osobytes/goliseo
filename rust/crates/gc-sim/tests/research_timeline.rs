//! Tests for `gc_sim::research_timeline`.
//!
//! [`fresh_timeline`] hand-builds a synthetic
//! [`gc_sim::research_timeline::RollbackEventDiff`]/[`RollbackEventStep`]
//! sequence — tick 0 confirmed as a `touch`, tick 1 first predicted as a
//! `tackle` and then corrected to a `pass` — rather than deriving it from a
//! real rollback timeline over the checked-in short match tape. None of
//! this file's assertions pin a hash or byte value computed by the real
//! tape; that pinning lives in `research_canonical.rs`, so every case here
//! runs as a real test with no `#[ignore]`.

use gc_sim::research_schema::Value;
use gc_sim::research_timeline::{
    self, ResearchBoundaryClock, RollbackEvent, RollbackEventDiff, RollbackEventStep,
};

const RUN_SCOPE: &str = "0123456789abcdef";
const GAME_INSTANCE: &str = "gi-0001";

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn set_field(value: &Value, name: &str, new_value: Value) -> Value {
    let entries = value.as_record().expect("record value").to_vec();
    let mut updated: Vec<(String, Value)> =
        entries.into_iter().filter(|(key, _)| key != name).collect();
    updated.push((name.to_string(), new_value));
    Value::Record(updated)
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    Value::record_get(value.as_record().expect("record value"), name).expect("field present")
}

fn event(id: &str, domain: &str, tick: i64, ordinal: i64) -> RollbackEvent {
    RollbackEvent {
        id: id.to_string(),
        domain: domain.to_string(),
        tick,
        ordinal,
    }
}

struct Sequence {
    speculative_diff: RollbackEventDiff,
    correction_diff: RollbackEventDiff,
    confirmed: Vec<RollbackEventStep>,
    revoked_rollback_event_id: String,
}

/// Builds a fresh `ResearchTimeline` plus the synthetic
/// speculative/correction diff and confirmed-step sequence most tests below
/// share.
fn fresh_timeline() -> (research_timeline::ResearchTimeline, Sequence) {
    let touch = event("e-touch", "soccer/touch", 0, 1);
    let tackle = event("e-tackle", "soccer/tackle", 1, 1);
    let pass = event("e-pass", "soccer/pass", 1, 1);

    let speculative_diff = RollbackEventDiff {
        added: vec![tackle.clone()],
        ..Default::default()
    };
    let correction_diff = RollbackEventDiff {
        added: vec![pass.clone()],
        revoked: vec![tackle.clone()],
        ..Default::default()
    };
    let confirmed = vec![
        RollbackEventStep {
            tick: 0,
            start_boundary: 0,
            end_boundary: 1,
            match_events: vec![touch],
            combat_events: vec![],
            lifecycle_events: vec![],
        },
        RollbackEventStep {
            tick: 1,
            start_boundary: 1,
            end_boundary: 2,
            match_events: vec![pass],
            combat_events: vec![],
            lifecycle_events: vec![],
        },
    ];

    let timeline = research_timeline::new(RUN_SCOPE, GAME_INSTANCE).unwrap();
    (
        timeline,
        Sequence {
            speculative_diff,
            correction_diff,
            confirmed,
            revoked_rollback_event_id: tackle_id(),
        },
    )
}

fn tackle_id() -> String {
    "e-tackle".to_string()
}

fn clock() -> ResearchBoundaryClock {
    ResearchBoundaryClock {
        origin_wall_clock_ms: 1000.0,
        tick_rate: gc_sim::fixed_clock::TICK_RATE,
        first_boundary_tick: 0,
        last_boundary_tick: 3,
        non_simulated_ms: 0.0,
    }
}

fn base_envelope(trace_id: &str) -> Value {
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_session_envelope")),
        ("digest", Value::str("fnv1a64/v1")),
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
                ("trace_id", Value::str(trace_id)),
                ("game_instance_id", Value::str(GAME_INSTANCE)),
                ("role", Value::str("condition_block")),
            ])]),
        ),
    ])
}

fn withdrawn_session() -> Value {
    let mut envelope = base_envelope("0000000000000000");
    envelope = set_field(&envelope, "session_id", Value::str("s-58f7a3b6e20591c9"));
    envelope = set_field(
        &envelope,
        "lifecycle",
        record(vec![
            ("status", Value::str("withdrawn")),
            ("started_wall_clock_ms", Value::Number(1000.0)),
            ("ended_wall_clock_ms", Value::Number(500_000.0)),
            ("observer_mode", Value::str("same_room")),
            ("assistance", Value::str("none")),
            ("interruptions", Value::Array(vec![])),
            ("exclusions", Value::Array(vec![])),
            (
                "missingness",
                Value::Array(vec![record(vec![
                    ("target_id", Value::str("session-payload")),
                    ("target_kind", Value::str("trace")),
                    ("reason_code", Value::str("participant_withdrew")),
                ])]),
            ),
        ]),
    );
    envelope = set_field(&envelope, "trace_links", Value::Array(vec![]));
    envelope = set_field(&envelope, "tombstone_id", Value::str("tombstone-4d7c1a9e"));
    envelope
}

fn participant_annotations(run_scope_id: &str, event_id: Option<&str>) -> Value {
    let mut annotation = record(vec![
        ("annotation_id", Value::str("an-1")),
        ("canonical_tick", Value::Number(1.0)),
        ("boundary_source", Value::str("wall_clock_mapped")),
        ("mapping_error_ms", Value::Number(4.5)),
        ("wall_clock_ms", Value::Number(1030.0)),
        ("code_id", Value::str("felt-unfair")),
        ("confidence", Value::Number(0.6)),
        ("free_text", Value::str("I did not see the tackle coming")),
    ]);
    if let Some(event_id) = event_id {
        annotation = set_field(&annotation, "event_id", Value::str(event_id));
    }
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_annotation_set")),
        ("digest", Value::str("fnv1a64/v1")),
        ("annotation_set_id", Value::str("as-participant-debrief-1")),
        ("run_scope_id", Value::str(run_scope_id)),
        ("session_id", Value::str("s-1c9d4f7a3b6e2058")),
        ("author_role", Value::str("participant")),
        ("agreement_version", Value::str("playtest-agreement-v3")),
        ("coding_scheme_version", Value::str("debrief-codes-v2")),
        ("annotations", Value::Array(vec![annotation])),
    ])
}

fn researcher_annotations(run_scope_id: &str) -> Value {
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_annotation_set")),
        ("digest", Value::str("fnv1a64/v1")),
        ("annotation_set_id", Value::str("as-researcher-coding-1")),
        ("run_scope_id", Value::str(run_scope_id)),
        ("session_id", Value::str("s-1c9d4f7a3b6e2058")),
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

fn gameplay() -> Value {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::observe_diff(&mut timeline, &sequence.speculative_diff).unwrap();
    research_timeline::observe_diff(&mut timeline, &sequence.correction_diff).unwrap();
    research_timeline::confirm(&mut timeline, &sequence.confirmed).unwrap();
    research_timeline::export(&timeline).unwrap()
}

#[test]
fn research_timeline_construction_requires_a_hash_shaped_run_scope_and_a_bounded_instance_id() {
    assert!(research_timeline::new(RUN_SCOPE, "gi-0001").is_ok());
    assert!(research_timeline::new("nope", "gi-0001").is_err());
    assert!(research_timeline::new(RUN_SCOPE, "Game Instance").is_err());
}

#[test]
fn research_timeline_construction_exports_an_empty_stream_before_anything_is_confirmed() {
    let timeline = research_timeline::new(RUN_SCOPE, "gi-0001").unwrap();
    let stream = research_timeline::export(&timeline).unwrap();
    assert_eq!(field(&stream, "rows").as_array().unwrap().len(), 0);
    assert_eq!(
        field(&stream, "confirmed_through_tick").as_number(),
        Some(-1.0)
    );
    assert_eq!(field(&stream, "confirmed_boundary").as_number(), Some(0.0));
    assert!(research_timeline::validate_stream(&stream).is_ok());
}

#[test]
fn confirmed_versus_speculative_exports_only_confirmed_events_and_drops_the_corrected_away_one() {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::observe_diff(&mut timeline, &sequence.speculative_diff).unwrap();
    research_timeline::observe_diff(&mut timeline, &sequence.correction_diff).unwrap();
    let added = research_timeline::confirm(&mut timeline, &sequence.confirmed).unwrap();
    assert_eq!(added, 2);
    let stream = research_timeline::export(&timeline).unwrap();
    let rows = field(&stream, "rows").as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(field(&rows[0], "event_kind").as_str(), Some("touch"));
    assert_eq!(field(&rows[0], "canonical_tick").as_number(), Some(0.0));
    assert_eq!(field(&rows[1], "event_kind").as_str(), Some("pass"));
    assert_eq!(field(&rows[1], "canonical_tick").as_number(), Some(1.0));
    assert_eq!(
        field(&stream, "confirmed_through_tick").as_number(),
        Some(1.0)
    );
    assert_eq!(field(&stream, "confirmed_boundary").as_number(), Some(2.0));
    for row in rows {
        assert_ne!(
            field(row, "rollback_event_id").as_str(),
            Some(sequence.revoked_rollback_event_id.as_str())
        );
    }
}

#[test]
fn confirmed_versus_speculative_refuses_to_confirm_a_step_that_still_carries_a_revoked_event() {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::observe_diff(&mut timeline, &sequence.speculative_diff).unwrap();
    research_timeline::observe_diff(&mut timeline, &sequence.correction_diff).unwrap();
    let mut stale = sequence.confirmed.clone();
    stale[1].match_events[0] = sequence.correction_diff.revoked[0].clone();
    let err = research_timeline::confirm(&mut timeline, &stale).unwrap_err();
    assert!(err.contains("revoked"), "{err}");
}

#[test]
fn confirmed_versus_speculative_refuses_to_revoke_or_replace_an_already_confirmed_event() {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::confirm(&mut timeline, &sequence.confirmed).unwrap();
    let confirmed_event = sequence.confirmed[0].match_events[0].clone();

    let revoke_attempt = RollbackEventDiff {
        revoked: vec![confirmed_event.clone()],
        ..Default::default()
    };
    let err = research_timeline::observe_diff(&mut timeline, &revoke_attempt).unwrap_err();
    assert!(err.contains("already-confirmed"), "{err}");

    let replace_attempt = RollbackEventDiff {
        replaced: vec![gc_sim::research_timeline::RollbackEventReplacement {
            before: confirmed_event.clone(),
            after: confirmed_event,
        }],
        ..Default::default()
    };
    assert!(research_timeline::observe_diff(&mut timeline, &replace_attempt).is_err());
}

#[test]
fn confirmed_versus_speculative_requires_contiguous_confirmation_from_the_research_boundary() {
    let (mut timeline, sequence) = fresh_timeline();
    let skipped = vec![sequence.confirmed[1].clone()];
    let err = research_timeline::confirm(&mut timeline, &skipped).unwrap_err();
    assert!(err.contains("noncontiguous"), "{err}");
}

#[test]
fn confirmed_versus_speculative_refuses_to_confirm_the_same_event_twice() {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::confirm(&mut timeline, &sequence.confirmed).unwrap();
    assert!(research_timeline::confirm(&mut timeline, &sequence.confirmed).is_err());
}

#[test]
fn confirmed_event_stream_canonicalization_orders_rows_by_tick_then_domain_rank_and_hashes_them() {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::observe_diff(&mut timeline, &sequence.correction_diff).unwrap();
    research_timeline::confirm(&mut timeline, &sequence.confirmed).unwrap();
    let stream = research_timeline::export(&timeline).unwrap();
    let rows = field(&stream, "rows").as_array().unwrap();
    let mut previous_tick: Option<f64> = None;
    for row in rows {
        assert_eq!(field(row, "domain").as_str(), Some("soccer"));
        assert_eq!(
            field(row, "domain_rank").as_number(),
            Some(research_timeline::domain_rank("soccer").unwrap() as f64)
        );
        if let Some(previous) = previous_tick {
            assert!(previous <= field(row, "canonical_tick").as_number().unwrap());
        }
        previous_tick = field(row, "canonical_tick").as_number();
    }
    assert_eq!(
        field(&stream, "stream_hash").as_str(),
        Some(research_timeline::stream_hash(&stream).unwrap().as_str())
    );
}

#[test]
fn confirmed_event_stream_canonicalization_fails_closed_when_a_row_or_hash_is_tampered_with() {
    let stream = gameplay();

    let mut rows = field(&stream, "rows").as_array().unwrap().to_vec();
    rows[0] = set_field(&rows[0], "event_kind", Value::str("goal"));
    let tampered = set_field(&stream, "rows", Value::Array(rows.clone()));
    assert!(research_timeline::validate_stream(&tampered).is_err());

    let rehashed = set_field(&stream, "stream_hash", Value::str("0000000000000000"));
    assert!(research_timeline::validate_stream(&rehashed).is_err());

    let mut unconfirmed = set_field(&stream, "confirmed_through_tick", Value::Number(0.0));
    unconfirmed = set_field(&unconfirmed, "confirmed_boundary", Value::Number(1.0));
    assert!(research_timeline::validate_stream(&unconfirmed).is_err());

    let original_rows = field(&stream, "rows").as_array().unwrap().to_vec();
    let mut reordered_rows = original_rows.clone();
    reordered_rows.swap(0, 1);
    let reordered = set_field(&stream, "rows", Value::Array(reordered_rows));
    assert!(research_timeline::validate_stream(&reordered).is_err());
}

#[test]
fn confirmed_event_stream_canonicalization_detects_a_duplicate_canonical_key_before_it_reaches_an_export()
 {
    let (mut timeline, sequence) = fresh_timeline();
    research_timeline::confirm(&mut timeline, &sequence.confirmed).unwrap();
    let first_row = timeline.rows[0].clone();
    timeline.rows.push(first_row);
    let err = research_timeline::export(&timeline).unwrap_err();
    assert!(err.contains("duplicate total key"), "{err}");
}

#[test]
fn wall_clock_cue_mapping_maps_a_cue_to_the_nearest_boundary_and_keeps_the_signed_error() {
    let exact = research_timeline::map_to_boundary(&clock(), 1000.0, None).unwrap();
    assert_eq!(exact.canonical_tick, 0);
    assert!(exact.mapping_error_ms.abs() < 1e-9);

    let mid =
        research_timeline::map_to_boundary(&clock(), 1000.0 + 1000.0 / 60.0 * 1.4, None).unwrap();
    assert_eq!(mid.canonical_tick, 1);
    assert!((mid.mapping_error_ms - 0.4 * 1000.0 / 60.0).abs() < 1e-6);

    let behind =
        research_timeline::map_to_boundary(&clock(), 1000.0 + 1000.0 / 60.0 * 1.6, None).unwrap();
    assert_eq!(behind.canonical_tick, 2);
    assert!(behind.mapping_error_ms < 0.0);
}

#[test]
fn wall_clock_cue_mapping_subtracts_pause_and_replay_wall_time_before_mapping() {
    let mut paused = clock();
    paused.non_simulated_ms = 5000.0;
    let mapping = research_timeline::map_to_boundary(&paused, 6000.0, None).unwrap();
    assert_eq!(mapping.canonical_tick, 0);
    assert!(mapping.mapping_error_ms.abs() < 1e-9);
}

#[test]
fn wall_clock_cue_mapping_fails_closed_outside_the_trace_window_and_on_an_unsupported_tick_rate() {
    let err = research_timeline::map_to_boundary(&clock(), 5000.0, None).unwrap_err();
    assert!(err.contains("outside"), "{err}");
    assert!(research_timeline::map_to_boundary(&clock(), 0.0, None).is_err());

    let mut wrong_rate = clock();
    wrong_rate.tick_rate += 1.0;
    assert!(research_timeline::map_to_boundary(&wrong_rate, 1000.0, None).is_err());
}

#[test]
fn research_annotation_sets_requires_cue_provenance_and_participant_authored_free_text() {
    let stream = gameplay();
    let run_scope_id = field(&stream, "run_scope_id").as_str().unwrap().to_string();
    let set = participant_annotations(&run_scope_id, None);
    assert!(research_timeline::validate_annotation_set(&set).is_ok());

    let annotations = field(&set, "annotations").as_array().unwrap().to_vec();
    let canonical_with_error_annotation = set_field(
        &annotations[0],
        "boundary_source",
        Value::str("canonical_tick"),
    );
    let canonical_with_error = set_field(
        &set,
        "annotations",
        Value::Array(vec![canonical_with_error_annotation]),
    );
    assert!(research_timeline::validate_annotation_set(&canonical_with_error).is_err());

    let mapped_without_clock_annotation = {
        let entries = annotations[0].as_record().unwrap().to_vec();
        Value::Record(
            entries
                .into_iter()
                .filter(|(k, _)| k != "wall_clock_ms")
                .collect(),
        )
    };
    let mapped_without_clock = set_field(
        &set,
        "annotations",
        Value::Array(vec![mapped_without_clock_annotation]),
    );
    assert!(research_timeline::validate_annotation_set(&mapped_without_clock).is_err());

    let tool_authored = set_field(&set, "author_role", Value::str("tool"));
    assert!(research_timeline::validate_annotation_set(&tool_authored).is_err());

    let duplicate = set_field(
        &set,
        "annotations",
        Value::Array(vec![annotations[0].clone(), annotations[0].clone()]),
    );
    assert!(research_timeline::validate_annotation_set(&duplicate).is_err());
}

#[test]
fn research_annotation_sets_rejects_non_finite_mapping_errors() {
    let stream = gameplay();
    let run_scope_id = field(&stream, "run_scope_id").as_str().unwrap().to_string();
    let set = participant_annotations(&run_scope_id, None);
    let annotation = field(&set, "annotations").as_array().unwrap()[0].clone();
    let nan_annotation = set_field(&annotation, "mapping_error_ms", Value::Number(f64::NAN));
    let nan = set_field(&set, "annotations", Value::Array(vec![nan_annotation]));
    assert!(research_timeline::validate_annotation_set(&nan).is_err());
}

#[test]
fn research_annotation_sets_gives_free_text_annotations_the_same_withdrawal_path_as_survey_answers()
{
    let stream = gameplay();
    let trace_id = "d7491ed5cc4cd10b";
    let set = participant_annotations(field(&stream, "run_scope_id").as_str().unwrap(), None);
    let envelope = base_envelope(trace_id);
    assert!(research_timeline::validate_against_session(&set, &envelope).is_ok());
    let first_annotation = field(&set, "annotations").as_array().unwrap()[0].clone();
    assert!(
        field(&first_annotation, "free_text").as_str().is_some(),
        "this fixture must carry free text"
    );

    let mut withdrawn = withdrawn_session();
    withdrawn = set_field(&withdrawn, "session_id", field(&set, "session_id").clone());
    let err = research_timeline::validate_against_session(&set, &withdrawn).unwrap_err();
    assert!(err.contains("withdrawn session"), "{err}");

    let mut other_session = base_envelope(trace_id);
    other_session = set_field(
        &other_session,
        "session_id",
        Value::str("s-000000000000000a"),
    );
    let err = research_timeline::validate_against_session(&set, &other_session).unwrap_err();
    assert!(err.contains("orphan join"), "{err}");

    let mut drifted = base_envelope(trace_id);
    let agreement = field(&drifted, "agreement").clone();
    let drifted_agreement = set_field(
        &agreement,
        "agreement_version",
        Value::str("playtest-agreement-v2"),
    );
    drifted = set_field(&drifted, "agreement", drifted_agreement);
    assert!(research_timeline::validate_against_session(&set, &drifted).is_err());
}

#[test]
fn research_annotation_sets_rejects_sets_from_another_run_duplicates_and_unconfirmed_cues() {
    let stream = gameplay();
    let run_scope_id = field(&stream, "run_scope_id").as_str().unwrap().to_string();
    let set = participant_annotations(&run_scope_id, None);
    let other = researcher_annotations("abcdefabcdefabcd");
    assert!(research_timeline::join_annotations(&stream, &[set.clone(), other]).is_err());

    assert!(research_timeline::join_annotations(&stream, &[set.clone(), set.clone()]).is_err());

    let annotations = field(&set, "annotations").as_array().unwrap().to_vec();
    let confirmed_through_tick = field(&stream, "confirmed_through_tick")
        .as_number()
        .unwrap();
    let late_annotation = set_field(
        &annotations[0],
        "canonical_tick",
        Value::Number(confirmed_through_tick + 5.0),
    );
    let late = set_field(&set, "annotations", Value::Array(vec![late_annotation]));
    let err = research_timeline::join_annotations(&stream, &[late]).unwrap_err();
    assert!(err.contains("confirmed boundary"), "{err}");
}
