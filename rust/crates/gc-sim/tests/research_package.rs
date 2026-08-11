//! Tests for `gc_sim::research_dataset`, `gc_sim::research_trace`,
//! `gc_sim::research_response`, `gc_sim::research_session`, and
//! `gc_sim::research_timeline` working together over one packaged research
//! run.
//!
//! Every case here builds its fixtures through
//! `research_fixtures::gameplay()` (a real rollback timeline and trace
//! manifest over the checked-in short match tape).
//!
//! One test,
//! `research_example_package_rejects_a_session_or_response_set_that_names_a_trace_nobody_holds`,
//! does not exercise `research_session::validate_against_traces` (the
//! envelope-vs-manifest orphan join, the empty-manifest-list case, and the
//! seed-mismatch branch): that function does not exist in this crate and is
//! explicitly out of scope — see `research_session.rs`'s own doc comment.
//! The `research_response::validate_against_trace` assertions in that same
//! test are fully exercised below.

mod research_fixtures;

use gc_sim::research_dataset;
use gc_sim::research_response;
use gc_sim::research_schema::Value;
use gc_sim::research_session;
use gc_sim::research_timeline;
use gc_sim::research_trace;
use research_fixtures::EnjoymentResponsesOverrides;

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
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

fn with_top(value: &Value, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    let mut top = record_entries(value);
    f(&mut top);
    Value::Record(top)
}

fn trace_id_of(manifest: &Value) -> String {
    get(&record_entries(manifest), "trace_id")
        .as_str()
        .expect("manifest has a trace_id")
        .to_string()
}

#[test]
fn research_example_package_assembles_a_completed_session_across_every_contract() {
    let gameplay = research_fixtures::gameplay(None);
    research_trace::validate(&gameplay.manifest).expect("manifest validates");
    research_timeline::validate_stream(&gameplay.stream).expect("stream validates");
    research_trace::validate_against_stream(&gameplay.manifest, &gameplay.stream)
        .expect("manifest validates against its stream");

    let trace_id = trace_id_of(&gameplay.manifest);
    let envelope = research_fixtures::completed_session(&trace_id);
    research_session::validate(&envelope).expect("envelope validates");

    let responses = research_fixtures::enjoyment_responses(Some(&EnjoymentResponsesOverrides {
        trace_id: Some(trace_id.clone()),
        ..Default::default()
    }));
    research_response::validate(&responses).expect("responses validate");
    let responses_top = record_entries(&responses);
    let scores = get(&responses_top, "scores").as_array().expect("array");
    assert_eq!(scores.len(), 1);
    let score_entries = record_entries(&scores[0]);
    let score = get(&score_entries, "score").as_number().expect("score");
    assert!((score - 2.0).abs() < 1e-9);

    let dataset = research_fixtures::dataset(&gameplay.manifest, None).expect("dataset seals");
    research_dataset::validate(&dataset).expect("dataset validates");
}

#[test]
fn research_example_package_keeps_corrected_away_rollback_events_out_of_the_confirmed_export() {
    let gameplay = research_fixtures::gameplay(None);
    let stream_top = record_entries(&gameplay.stream);
    let rows = get(&stream_top, "rows").as_array().expect("array");
    let mut kinds: Vec<String> = Vec::new();
    for row in rows {
        let row_entries = record_entries(row);
        let rollback_event_id = get(&row_entries, "rollback_event_id").as_str().expect("id");
        assert_ne!(
            rollback_event_id, gameplay.revoked_rollback_event_id,
            "revoked event leaked into the export"
        );
        let event_kind = get(&row_entries, "event_kind").as_str().expect("kind");
        assert_ne!(
            event_kind, "tackle",
            "the corrected-away tackle leaked into the export"
        );
        kinds.push(event_kind.to_string());
    }
    assert!(
        kinds.iter().any(|k| k == "touch"),
        "the confirmed touch is missing"
    );
    assert!(
        kinds.iter().any(|k| k == "pass"),
        "the corrected pass is missing"
    );
}

#[test]
fn research_example_package_rejects_a_session_or_response_set_that_names_a_trace_nobody_holds() {
    let gameplay = research_fixtures::gameplay(None);
    let trace_id = trace_id_of(&gameplay.manifest);
    let envelope = research_fixtures::completed_session(&trace_id);

    // `research_session::validate_against_traces` (the envelope-vs-manifest
    // orphan join, the empty-manifest-list case, and the seed-mismatch
    // branch) is not exercised here — see this file's module doc comment.

    let responses = research_fixtures::enjoyment_responses(Some(&EnjoymentResponsesOverrides {
        trace_id: Some(trace_id.clone()),
        ..Default::default()
    }));
    research_response::validate_against_trace(&responses, &gameplay.manifest)
        .expect("responses validate against their trace");
    research_response::validate_against_session(&responses, &envelope)
        .expect("responses validate against their session");

    let orphan_responses =
        research_fixtures::enjoyment_responses(Some(&EnjoymentResponsesOverrides {
            trace_id: Some("1111111111111111".to_string()),
            ..Default::default()
        }));
    let err = research_response::validate_against_trace(&orphan_responses, &gameplay.manifest)
        .expect_err("orphan trace id must be rejected");
    assert!(err.contains("orphan join"), "unexpected error: {err}");
}

#[test]
fn research_example_package_refuses_a_response_set_for_an_instrument_the_session_says_was_skipped()
{
    let gameplay = research_fixtures::gameplay(None);
    let trace_id = trace_id_of(&gameplay.manifest);
    let interrupted = research_fixtures::interrupted_session(&trace_id);
    let responses = research_fixtures::enjoyment_responses(Some(&EnjoymentResponsesOverrides {
        session_id: Some(research_fixtures::INTERRUPTED_SESSION_ID.to_string()),
        ..Default::default()
    }));
    let err = research_response::validate_against_session(&responses, &interrupted)
        .expect_err("must fail");
    assert!(err.contains("not administered"), "unexpected error: {err}");
}

#[test]
fn research_example_package_keeps_model_use_tied_to_the_accepted_agreement_version() {
    let gameplay = research_fixtures::gameplay(None);
    let trace_id = trace_id_of(&gameplay.manifest);
    let envelope = research_fixtures::completed_session(&trace_id);
    research_session::allows_model_use(&envelope).expect("model use allowed");

    let uncovered = with_top(&research_fixtures::completed_session(&trace_id), |top| {
        let mut agreement = record_entries(get(top, "agreement"));
        set(&mut agreement, "model_use_covered", Value::Bool(false));
        set(top, "agreement", Value::Record(agreement));
    });
    let err = research_session::allows_model_use(&uncovered).expect_err("must fail");
    assert!(err.contains("did not cover"), "unexpected error: {err}");
}

#[test]
fn research_example_package_joins_two_independent_annotation_sets_to_one_gameplay_run() {
    let gameplay = research_fixtures::gameplay(None);
    let stream_top = record_entries(&gameplay.stream);
    let rows = get(&stream_top, "rows").as_array().expect("array");
    let row0 = record_entries(&rows[0]);
    let event_id = get(&row0, "event_id").as_str().expect("id").to_string();
    let run_scope_id = get(&stream_top, "run_scope_id")
        .as_str()
        .expect("id")
        .to_string();

    let participant = research_fixtures::participant_annotations(&run_scope_id, Some(&event_id));
    let researcher = research_fixtures::researcher_annotations(&run_scope_id);
    research_timeline::join_annotations(&gameplay.stream, &[participant, researcher])
        .expect("annotation sets join");

    let orphan = with_top(
        &research_fixtures::participant_annotations(&run_scope_id, Some("abcdefabcdefabcd")),
        |top| {
            set(top, "annotation_set_id", Value::str("as-orphan"));
        },
    );
    let err =
        research_timeline::join_annotations(&gameplay.stream, &[orphan]).expect_err("must fail");
    assert!(err.contains("orphan"), "unexpected error: {err}");
}

#[test]
fn research_example_package_records_an_interrupted_session_with_structured_missingness() {
    let gameplay = research_fixtures::gameplay(Some("incomplete_interrupted"));
    let manifest_top = record_entries(&gameplay.manifest);
    let simulation = record_entries(get(&manifest_top, "simulation"));
    assert_eq!(
        get(&simulation, "completion").as_str(),
        Some("incomplete_interrupted")
    );
    let trace_id = trace_id_of(&gameplay.manifest);
    let envelope = research_fixtures::interrupted_session(&trace_id);
    research_session::validate(&envelope).expect("envelope validates");
    let envelope_top = record_entries(&envelope);
    let lifecycle = record_entries(get(&envelope_top, "lifecycle"));
    assert_eq!(
        get(&lifecycle, "interruptions")
            .as_array()
            .expect("array")
            .len(),
        1
    );
    let missingness = get(&lifecycle, "missingness").as_array().expect("array");
    let first_missing = record_entries(&missingness[0]);
    assert_eq!(
        get(&first_missing, "reason_code").as_str(),
        Some("instrument_not_administered")
    );

    let without_reason = with_top(&research_fixtures::interrupted_session(&trace_id), |top| {
        let mut lifecycle = record_entries(get(top, "lifecycle"));
        set(&mut lifecycle, "interruptions", Value::Array(vec![]));
        set(top, "lifecycle", Value::Record(lifecycle));
    });
    assert!(research_session::validate(&without_reason).is_err());
}

#[test]
fn research_example_package_scores_nothing_when_a_survey_item_is_missing() {
    let complete = research_fixtures::enjoyment_responses(None);
    let complete_top = record_entries(&complete);
    assert_eq!(
        get(&complete_top, "scores")
            .as_array()
            .expect("array")
            .len(),
        1
    );

    let partial = research_fixtures::enjoyment_responses(Some(&EnjoymentResponsesOverrides {
        missing_item: Some("participant_skipped".to_string()),
        response_set_id: Some("rs-pxi-enjoyment-b-partial".to_string()),
        ..Default::default()
    }));
    research_response::validate(&partial).expect("partial responses validate");
    let partial_top = record_entries(&partial);
    assert_eq!(
        get(&partial_top, "scores").as_array().expect("array").len(),
        0
    );

    let scored_anyway = with_top(
        &research_fixtures::enjoyment_responses(Some(&EnjoymentResponsesOverrides {
            missing_item: Some("participant_skipped".to_string()),
            response_set_id: Some("rs-pxi-enjoyment-b-partial".to_string()),
            ..Default::default()
        })),
        |top| {
            set(
                top,
                "scores",
                Value::Array(vec![record(vec![
                    ("construct", Value::str("enjoyment")),
                    ("score", Value::Number(1.5)),
                    ("rule", Value::str("mean_all_items")),
                    ("item_count", Value::Number(2.0)),
                ])]),
            );
        },
    );
    assert!(research_response::validate(&scored_anyway).is_err());
}

#[test]
fn research_example_package_pairs_a_withdrawal_tombstone_with_its_emptied_session_envelope() {
    let envelope = research_fixtures::withdrawn_session();
    let tombstone = research_fixtures::withdrawal_tombstone(None);
    research_session::validate(&envelope).expect("envelope validates");
    research_session::validate_tombstone(&tombstone).expect("tombstone validates");
    research_session::validate_withdrawal(&envelope, &tombstone).expect("withdrawal validates");

    let retained = with_top(&research_fixtures::withdrawn_session(), |top| {
        set(
            top,
            "trace_links",
            Value::Array(vec![record(vec![
                ("trace_id", Value::str("1111111111111111")),
                ("game_instance_id", Value::str("gi-0001")),
                ("role", Value::str("condition_block")),
            ])]),
        );
    });
    assert!(research_session::validate(&retained).is_err());

    let mismatched = with_top(&research_fixtures::withdrawal_tombstone(None), |top| {
        set(top, "tombstone_id", Value::str("tombstone-other"));
    });
    assert!(research_session::validate_withdrawal(&envelope, &mismatched).is_err());
}

#[test]
fn research_example_package_refuses_a_dataset_that_retains_a_withdrawn_participant() {
    let gameplay = research_fixtures::gameplay(None);
    let dataset = research_fixtures::dataset(&gameplay.manifest, None).expect("dataset seals");
    let tombstone = research_fixtures::withdrawal_tombstone(None);
    let err = research_dataset::validate_against_tombstones(&dataset, &[tombstone])
        .expect_err("must fail");
    assert!(err.contains("withdrawn"), "unexpected error: {err}");

    let other = with_top(&research_fixtures::withdrawal_tombstone(None), |top| {
        set(top, "participant_id", Value::str("p-000000000000000a"));
        set(top, "session_id", Value::str("s-000000000000000a"));
    });
    research_dataset::validate_against_tombstones(&dataset, &[other])
        .expect("no overlap, must pass");
}
