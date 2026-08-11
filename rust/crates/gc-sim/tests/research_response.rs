//! Tests for `gc_sim::research_response`.
//!
//! The `enjoyment_responses` fixture is reproduced locally as [`enjoyment`]
//! rather than pulled from `research_fixtures` (this crate's shared
//! trace/rollback fixture module): this file only ever needs the
//! response/session-envelope-shaped helpers (`enjoyment_responses`, plus the
//! constant participant/session ids), never the trace/rollback-derived ones
//! — see `research_package.rs` for the fixtures that do need those and are
//! `#[ignore]`d.

use gc_data::research_instruments as data_instruments;
use gc_sim::research_response;
use gc_sim::research_schema::{self, Value};

const SESSION_ID: &str = "s-1c9d4f7a3b6e2058";
const PARTICIPANT_ID: &str = "p-7f3a9c21b8e45d06";

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

fn remove_field(value: &Value, name: &str) -> Value {
    let entries = value.as_record().expect("record value").to_vec();
    Value::Record(entries.into_iter().filter(|(key, _)| key != name).collect())
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    Value::record_get(value.as_record().expect("record value"), name).expect("field present")
}

#[derive(Default)]
struct EnjoymentOverrides {
    missing_item: Option<&'static str>,
    response_set_id: Option<&'static str>,
    session_id: Option<&'static str>,
}

/// Builds the enjoyment response-set fixture used across this file's tests,
/// with optional overrides.
fn enjoyment_with(overrides: EnjoymentOverrides) -> Value {
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
    if let Some(missing_reason) = overrides.missing_item {
        responses[2] = record(vec![
            ("item_id", Value::str("pxi_enjoy_3")),
            ("missing_reason", Value::str(missing_reason)),
            ("response_ms", Value::Number(8000.0)),
        ]);
        scores = vec![];
    }
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_response_set")),
        ("digest", Value::str(research_schema::DIGEST)),
        (
            "response_set_id",
            Value::str(overrides.response_set_id.unwrap_or("rs-pxi-enjoyment-b-1")),
        ),
        (
            "session_id",
            Value::str(overrides.session_id.unwrap_or(SESSION_ID)),
        ),
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
        ("responses", Value::Array(responses)),
        ("scores", Value::Array(scores)),
    ])
}

fn enjoyment() -> Value {
    enjoyment_with(EnjoymentOverrides::default())
}

#[derive(Default)]
struct ResponseSetOverrides {
    response_set_id: Option<&'static str>,
    condition_id: Option<&'static str>,
    relative_to_play: Option<&'static str>,
    scores: Option<Vec<Value>>,
}

fn response_set(
    instrument_id: &str,
    responses: Vec<Value>,
    overrides: ResponseSetOverrides,
) -> Value {
    let instrument = data_instruments::get(instrument_id).expect("known instrument");
    let order: Vec<Value> = instrument
        .items
        .iter()
        .map(|item| Value::str(item.id))
        .collect();
    let analysis_role = match instrument.analysis_role {
        data_instruments::ResearchAnalysisRole::Primary => "primary",
        data_instruments::ResearchAnalysisRole::Diagnostic => "diagnostic",
        data_instruments::ResearchAnalysisRole::Exploratory => "exploratory",
    };
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_response_set")),
        ("digest", Value::str(research_schema::DIGEST)),
        (
            "response_set_id",
            Value::str(
                overrides
                    .response_set_id
                    .map_or_else(|| format!("rs-{instrument_id}"), str::to_string),
            ),
        ),
        ("session_id", Value::str(SESSION_ID)),
        ("participant_id", Value::str(PARTICIPANT_ID)),
        (
            "condition_id",
            Value::str(overrides.condition_id.unwrap_or("condition-b-combat-on")),
        ),
        ("instrument_id", Value::str(instrument_id)),
        (
            "instrument_version",
            Value::str(instrument.instrument_version),
        ),
        (
            "scoring_key_version",
            Value::str(instrument.scoring_key_version),
        ),
        ("analysis_role", Value::str(analysis_role)),
        ("validated_instrument", Value::Bool(instrument.validated)),
        (
            "partial_administration",
            Value::Bool(instrument.partial_administration),
        ),
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
                (
                    "relative_to_play",
                    Value::str(overrides.relative_to_play.unwrap_or("natural_break")),
                ),
                ("offset_ms", Value::Number(1200.0)),
            ]),
        ),
        ("presentation_order", Value::Array(order)),
        ("randomized_presentation", Value::Bool(false)),
        ("responses", Value::Array(responses)),
        ("scores", Value::Array(overrides.scores.unwrap_or_default())),
    ])
}

fn slider_set(overrides: ResponseSetOverrides) -> Value {
    response_set(
        "affective_slider",
        vec![
            record(vec![
                ("item_id", Value::str("as_valence")),
                ("raw_response", Value::Number(0.72)),
            ]),
            record(vec![
                ("item_id", Value::str("as_arousal")),
                ("raw_response", Value::Number(0.35)),
            ]),
        ],
        overrides,
    )
}

fn fallback_set(overrides: ResponseSetOverrides) -> Value {
    response_set(
        "custom_affect_fallback",
        vec![
            record(vec![
                ("item_id", Value::str("fallback_valence")),
                ("raw_response", Value::Number(5.0)),
            ]),
            record(vec![
                ("item_id", Value::str("fallback_arousal")),
                ("raw_response", Value::Number(3.0)),
            ]),
        ],
        overrides,
    )
}

#[test]
fn research_instrument_register_accepts_the_authored_register_and_embeds_no_item_text() {
    assert!(research_response::validate_register(None));
    for instrument in data_instruments::ALL {
        assert!(
            !instrument.item_text_included,
            "{} embeds item text",
            instrument.id
        );
        assert!(!instrument.constructs.is_empty());
        assert!(!instrument.items.is_empty());
    }
}

#[test]
fn research_instrument_register_keeps_separately_named_constructs_separate() {
    let pxi = data_instruments::get("pxi_enjoyment_addon").unwrap();
    let bangs = data_instruments::get("bangs_session").unwrap();
    assert_eq!(pxi.constructs.len(), 1);
    assert_eq!(bangs.constructs.len(), 6);
    assert_ne!(pxi.pooling_group, bangs.pooling_group);
    assert!(
        pxi.forbidden_pooling.contains(&"bangs_session"),
        "PXI enjoyment must not be poolable with BANGS"
    );
}

#[test]
fn research_instrument_register_rejects_a_register_that_claims_a_primary_unvalidated_instrument() {
    // Each case mutates one authored instrument in a local copy of the
    // register and expects validate_register to panic;
    // `std::panic::catch_unwind` catches that panic so the assertion can
    // inspect the result.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let run = |mutate: &dyn Fn(&mut Vec<data_instruments::ResearchInstrumentData>)| {
        let mut copy: Vec<data_instruments::ResearchInstrumentData> =
            data_instruments::ALL.to_vec();
        mutate(&mut copy);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            research_response::validate_register(Some(&copy))
        }))
    };

    let broken = run(&|copy| {
        let entry = copy
            .iter_mut()
            .find(|i| i.id == "custom_diagnostics")
            .unwrap();
        entry.analysis_role = data_instruments::ResearchAnalysisRole::Primary;
    });
    assert!(broken.is_err());

    let text_leak = run(&|copy| {
        let entry = copy
            .iter_mut()
            .find(|i| i.id == "pxi_enjoyment_addon")
            .unwrap();
        entry.item_text_included = true;
    });
    assert!(text_leak.is_err());

    let aggregated_custom = run(&|copy| {
        let entry = copy
            .iter_mut()
            .find(|i| i.id == "custom_diagnostics")
            .unwrap();
        entry.score_aggregation = data_instruments::ResearchScoreAggregation::MeanAllItems;
    });
    assert!(aggregated_custom.is_err());

    let unknown_substitute = run(&|copy| {
        let entry = copy
            .iter_mut()
            .find(|i| i.id == "custom_affect_fallback")
            .unwrap();
        entry.substitute_for = Some("no_such_instrument");
    });
    assert!(unknown_substitute.is_err());

    std::panic::set_hook(previous_hook);
}

#[test]
fn research_response_set_validation_accepts_the_complete_enjoyment_administration_and_round_trips()
{
    let set = enjoyment();
    assert!(research_response::validate(&set).is_ok());
    let bytes = research_response::encode(&set).unwrap();
    let decoded = research_response::decode(&bytes).unwrap();
    assert_eq!(research_response::encode(&decoded).unwrap(), bytes);
    let scores = field(&decoded, "scores").as_array().unwrap();
    let first_score = scores[0].as_record().unwrap();
    assert_eq!(
        Value::record_get(first_score, "construct").and_then(Value::as_str),
        Some("enjoyment")
    );
}

#[test]
fn research_response_set_validation_rejects_non_finite_responses_in_this_family() {
    let mut set = enjoyment();
    set = set_field(&set, "scores", Value::Array(vec![]));
    let responses = field(&set, "responses").as_array().unwrap().to_vec();
    let mut responses = responses;
    responses[0] = set_field(&responses[0], "raw_response", Value::Number(f64::NAN));
    set = set_field(&set, "responses", Value::Array(responses));
    assert!(research_response::validate(&set).is_err());
}

#[test]
fn research_response_set_validation_rejects_responses_off_the_registered_scale() {
    let base = enjoyment();
    let responses = field(&base, "responses").as_array().unwrap().to_vec();

    let mut out_of_range_responses = responses.clone();
    out_of_range_responses[0] = set_field(
        &out_of_range_responses[0],
        "raw_response",
        Value::Number(4.0),
    );
    let out_of_range = set_field(&base, "responses", Value::Array(out_of_range_responses));
    assert!(research_response::validate(&out_of_range).is_err());

    let mut off_step_responses = responses.clone();
    off_step_responses[0] = set_field(&off_step_responses[0], "raw_response", Value::Number(1.5));
    let off_step = set_field(&base, "responses", Value::Array(off_step_responses));
    assert!(research_response::validate(&off_step).is_err());

    let slider_off_step_base = slider_set(ResponseSetOverrides::default());
    let mut slider_responses = field(&slider_off_step_base, "responses")
        .as_array()
        .unwrap()
        .to_vec();
    slider_responses[0] = set_field(&slider_responses[0], "raw_response", Value::Number(0.725));
    let slider_off_step = set_field(
        &slider_off_step_base,
        "responses",
        Value::Array(slider_responses),
    );
    assert!(research_response::validate(&slider_off_step).is_err());
}

#[test]
fn research_response_set_validation_requires_exactly_one_row_per_item_answered_or_structurally_missing()
 {
    let base = enjoyment();
    let responses = field(&base, "responses").as_array().unwrap().to_vec();

    let mut both_responses = responses.clone();
    both_responses[0] = set_field(
        &both_responses[0],
        "missing_reason",
        Value::str("participant_skipped"),
    );
    let both = set_field(&base, "responses", Value::Array(both_responses));
    assert!(research_response::validate(&both).is_err());

    let mut neither_responses = responses.clone();
    neither_responses[0] = remove_field(&neither_responses[0], "raw_response");
    let neither = set_field(&base, "responses", Value::Array(neither_responses));
    assert!(research_response::validate(&neither).is_err());

    let mut dropped_responses = responses.clone();
    dropped_responses.remove(2);
    let mut dropped = set_field(&base, "responses", Value::Array(dropped_responses));
    dropped = set_field(&dropped, "scores", Value::Array(vec![]));
    let err = research_response::validate(&dropped).unwrap_err();
    assert!(err.contains("one row per"), "{err}");

    let mut duplicated_responses = responses.clone();
    duplicated_responses[2] = duplicated_responses[0].clone();
    let duplicated = set_field(&base, "responses", Value::Array(duplicated_responses));
    assert!(research_response::validate(&duplicated).is_err());
}

#[test]
fn research_response_set_validation_requires_the_presentation_order_to_cover_the_instrument_exactly()
 {
    let base = enjoyment();
    let order = field(&base, "presentation_order")
        .as_array()
        .unwrap()
        .to_vec();

    let mut short_order = order.clone();
    short_order.remove(0);
    let short = set_field(&base, "presentation_order", Value::Array(short_order));
    assert!(research_response::validate(&short).is_err());

    let mut duplicated_order = order.clone();
    duplicated_order[0] = duplicated_order[1].clone();
    let duplicated = set_field(&base, "presentation_order", Value::Array(duplicated_order));
    assert!(research_response::validate(&duplicated).is_err());

    let mut foreign_order = order.clone();
    foreign_order[0] = Value::str("bangs_auto_sat_1");
    let foreign = set_field(&base, "presentation_order", Value::Array(foreign_order));
    assert!(research_response::validate(&foreign).is_err());
}

#[test]
fn research_response_set_validation_stops_on_instrument_scoring_key_and_role_drift() {
    let base = enjoyment();

    let wrong_instrument_version = set_field(
        &base,
        "instrument_version",
        Value::str("pxi-2019.0-enjoyment-addon"),
    );
    let err = research_response::validate(&wrong_instrument_version).unwrap_err();
    assert!(err.contains("does not match registered"), "{err}");

    let wrong_key = set_field(
        &base,
        "scoring_key_version",
        Value::str("pxi-enjoyment-sum-1"),
    );
    assert!(research_response::validate(&wrong_key).is_err());

    let relabelled = set_field(&base, "analysis_role", Value::str("exploratory"));
    assert!(research_response::validate(&relabelled).is_err());

    let unvalidated_claim = set_field(&base, "validated_instrument", Value::Bool(false));
    assert!(research_response::validate(&unvalidated_claim).is_err());

    let unknown = set_field(&base, "instrument_id", Value::str("not-an-instrument"));
    assert!(research_response::validate(&unknown).is_err());
}

#[test]
fn research_response_set_validation_requires_translation_provenance_to_be_internally_consistent() {
    let base = enjoyment();
    let locale = field(&base, "locale").clone();

    let translated_locale = set_field(
        &locale,
        "translation_provenance",
        Value::str("validated_translation"),
    );
    let translated = set_field(&base, "locale", translated_locale.clone());
    assert!(research_response::validate(&translated).is_err());

    let translated_with_id = set_field(
        &translated_locale,
        "translation_id",
        Value::str("pxi-es-2022"),
    );
    let translated_ok = set_field(&base, "locale", translated_with_id);
    assert!(research_response::validate(&translated_ok).is_ok());

    let original_with_id = set_field(&locale, "translation_id", Value::str("pxi-es-2022"));
    let original_with_id_set = set_field(&base, "locale", original_with_id);
    assert!(research_response::validate(&original_with_id_set).is_err());
}

#[test]
fn research_construct_scoring_scores_a_mean_only_when_every_construct_item_is_answered() {
    let (scores, incomplete) = research_response::recompute_scores(&enjoyment()).unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(incomplete.len(), 0);
    let score = Value::record_get(scores[0].as_record().unwrap(), "score")
        .and_then(Value::as_number)
        .unwrap();
    assert!((score - 2.0).abs() < 1e-9);

    let partial = enjoyment_with(EnjoymentOverrides {
        missing_item: Some("time_limit"),
        ..Default::default()
    });
    let (partial_scores, partial_incomplete) =
        research_response::recompute_scores(&partial).unwrap();
    assert_eq!(partial_scores.len(), 0);
    assert_eq!(partial_incomplete.len(), 1);
    assert_eq!(partial_incomplete[0], "enjoyment");
    assert!(research_response::validate(&partial).is_ok());
}

#[test]
fn research_construct_scoring_never_aggregates_an_item_only_instrument() {
    let slider = slider_set(ResponseSetOverrides::default());
    assert!(research_response::validate(&slider).is_ok());
    let (scores, incomplete) = research_response::recompute_scores(&slider).unwrap();
    assert_eq!(scores.len(), 0);
    assert_eq!(incomplete.len(), 2);

    let aggregated = slider_set(ResponseSetOverrides {
        scores: Some(vec![record(vec![
            ("construct", Value::str("valence")),
            ("score", Value::Number(0.72)),
            ("rule", Value::str("item_only")),
            ("item_count", Value::Number(1.0)),
        ])]),
        ..Default::default()
    });
    let err = research_response::validate(&aggregated).unwrap_err();
    assert!(err.contains("item-only"), "{err}");
}

#[test]
fn research_construct_scoring_rejects_a_hand_edited_or_partially_scored_construct() {
    let base = enjoyment();
    let scores = field(&base, "scores").as_array().unwrap().to_vec();

    let mut edited_scores = scores.clone();
    edited_scores[0] = set_field(&edited_scores[0], "score", Value::Number(2.5));
    let edited = set_field(&base, "scores", Value::Array(edited_scores));
    assert!(research_response::validate(&edited).is_err());

    let mut wrong_count_scores = scores.clone();
    wrong_count_scores[0] = set_field(&wrong_count_scores[0], "item_count", Value::Number(2.0));
    let wrong_count = set_field(&base, "scores", Value::Array(wrong_count_scores));
    assert!(research_response::validate(&wrong_count).is_err());

    let mut scored_partial = enjoyment_with(EnjoymentOverrides {
        missing_item: Some("participant_skipped"),
        ..Default::default()
    });
    scored_partial = set_field(
        &scored_partial,
        "scores",
        Value::Array(vec![record(vec![
            ("construct", Value::str("enjoyment")),
            ("score", Value::Number(1.5)),
            ("rule", Value::str("mean_all_items")),
            ("item_count", Value::Number(2.0)),
        ])]),
    );
    assert!(research_response::validate(&scored_partial).is_err());
}

#[test]
fn research_response_administration_accepts_distinct_instruments_at_one_timepoint() {
    let sets = vec![enjoyment(), slider_set(ResponseSetOverrides::default())];
    assert!(research_response::validate_administration(&sets).is_ok());
}

#[test]
fn research_response_administration_refuses_the_accessibility_fallback_beside_the_instrument_it_replaces()
 {
    let sets = vec![
        slider_set(ResponseSetOverrides {
            relative_to_play: Some("natural_break"),
            ..Default::default()
        }),
        fallback_set(ResponseSetOverrides {
            relative_to_play: Some("natural_break"),
            ..Default::default()
        }),
    ];
    let err = research_response::validate_administration(&sets).unwrap_err();
    assert!(err.contains("non-poolable substitute target"), "{err}");

    let alone = vec![fallback_set(ResponseSetOverrides::default())];
    assert!(research_response::validate_administration(&alone).is_ok());
}

#[test]
fn research_response_administration_refuses_a_repeated_administration_of_the_same_instrument_and_timepoint()
 {
    let sets = vec![
        enjoyment(),
        enjoyment_with(EnjoymentOverrides {
            response_set_id: Some("rs-second"),
            ..Default::default()
        }),
    ];
    assert!(research_response::validate_administration(&sets).is_err());

    let duplicate_ids = vec![enjoyment(), enjoyment()];
    assert!(research_response::validate_administration(&duplicate_ids).is_err());
}
