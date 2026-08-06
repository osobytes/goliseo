//! Port of `sim/research_response.lua`.
//!
//! Experience-response contract: instrument register validation, response
//! set validation, and construct scoring that refuses to pool distinct
//! constructs.
//!
//! The register in `gc_data::research_instruments` is authored content, so
//! a broken register is a programmer error and [`validate_register`]
//! `assert!`s. Response sets arrive from collection tooling, so every other
//! public reader here returns [`Result::Err`].
//!
//! Two rules exist to stop the silent-combination failure named in issue
//! #132:
//!   * an instrument whose aggregation is `item_only` may never carry a
//!     construct score, so custom exploratory items and Affective Slider
//!     readings cannot be averaged into anything; and
//!   * a construct score is recomputed from the raw responses and compared,
//!     so a hand-edited or partially-scored value fails closed instead of
//!     travelling into a dataset.

use crate::research_schema::{
    self, ResearchField, ResearchFieldKind, ResearchShape, Result, Value,
};
use gc_data::research_instruments::{self as data_instruments, ResearchInstrumentData};

/// Reader version for the response-set wire shape.
pub const VERSION: i64 = 1;
/// Serialization versions this reader accepts.
pub const SUPPORTED_VERSIONS: &[i64] = &[1];
/// `manifest_kind` this module writes and reads.
pub const KIND: &str = "research_response_set";
/// Tolerance for matching a raw response to a declared scale step.
pub const SCALE_EPSILON: f64 = 1e-9;

/// Declared structured-missingness reasons for a response item.
#[must_use]
pub fn missing_reasons() -> Vec<String> {
    research_schema::enum_values(&[
        "participant_skipped",
        "participant_withdrew",
        "instrument_not_administered",
        "operator_error",
        "technical_failure",
        "time_limit",
    ])
}

fn response_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("item_id"),
        ResearchField::new(ResearchFieldKind::Number)
            .named("raw_response")
            .optional(),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("missing_reason")
            .optional()
            .values(missing_reasons()),
        ResearchField::new(ResearchFieldKind::Number)
            .named("response_ms")
            .optional()
            .min(0.0),
    ]
}

fn score_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("construct"),
        ResearchField::new(ResearchFieldKind::Number).named("score"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("rule")
            .values(research_schema::enum_values(&[
                "mean_all_items",
                "item_only",
            ])),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("item_count")
            .min(1.0),
    ]
}

fn locale_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("language_tag"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("translation_provenance")
            .values(research_schema::enum_values(&[
                "original",
                "validated_translation",
                "project_adaptation",
            ])),
        ResearchField::new(ResearchFieldKind::Id)
            .named("translation_id")
            .optional(),
    ]
}

fn timing_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Enum)
            .named("relative_to_play")
            .values(research_schema::enum_values(&[
                "pre_block",
                "post_block",
                "natural_break",
                "post_session",
                "debrief",
            ])),
        ResearchField::new(ResearchFieldKind::Number).named("offset_ms"),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("canonical_boundary_tick")
            .optional()
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Hash)
            .named("trace_id")
            .optional(),
    ]
}

/// The `research_response_set/v1` record shape.
#[must_use]
pub fn shape() -> ResearchShape {
    research_schema::record(
        "research_response_set/v1",
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
            ResearchField::new(ResearchFieldKind::Id).named("response_set_id"),
            ResearchField::new(ResearchFieldKind::Id)
                .named("session_id")
                .min_length(16),
            ResearchField::new(ResearchFieldKind::Id)
                .named("participant_id")
                .min_length(16),
            ResearchField::new(ResearchFieldKind::Id).named("condition_id"),
            ResearchField::new(ResearchFieldKind::Id).named("instrument_id"),
            ResearchField::new(ResearchFieldKind::Id).named("instrument_version"),
            ResearchField::new(ResearchFieldKind::Id).named("scoring_key_version"),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("analysis_role")
                .values(research_schema::enum_values(&[
                    "primary",
                    "diagnostic",
                    "exploratory",
                ])),
            ResearchField::new(ResearchFieldKind::Boolean).named("validated_instrument"),
            ResearchField::new(ResearchFieldKind::Boolean).named("partial_administration"),
            ResearchField::new(ResearchFieldKind::Record)
                .named("locale")
                .fields(locale_fields()),
            ResearchField::new(ResearchFieldKind::Record)
                .named("timing")
                .fields(timing_fields()),
            ResearchField::new(ResearchFieldKind::Array)
                .named("presentation_order")
                .min_length(1)
                .max_length(128)
                .element(ResearchField::new(ResearchFieldKind::Id).named("item_id")),
            ResearchField::new(ResearchFieldKind::Boolean).named("randomized_presentation"),
            ResearchField::new(ResearchFieldKind::Array)
                .named("responses")
                .min_length(1)
                .max_length(128)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("response")
                        .fields(response_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Array)
                .named("scores")
                .max_length(32)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("score")
                        .fields(score_fields()),
                ),
        ],
    )
}

fn is_finite(value: f64) -> bool {
    value.is_finite()
}

/// Authored register validation. A broken register is a code/content bug.
///
/// # Panics
///
/// Panics (`assert!`) on any structural or cross-field violation.
///
/// Several of the Lua original's checks (`scale_kind` is `"likert"` or
/// `"continuous"`, `score_aggregation` is `"mean_all_items"` or
/// `"item_only"`, `analysis_role` is one of three roles, `reverse_scored`
/// is a boolean) exist because Lua fields are untyped strings/values; here
/// [`gc_data::research_instruments::ResearchScaleKind`] etc. are closed Rust
/// enums, so those specific checks are structurally impossible to violate
/// and are dropped rather than ported as dead code. The cross-field checks
/// they were adjacent to (mean scoring requires the complete-item rule, a
/// primary role requires a validated instrument, ...) are real and kept.
#[must_use]
pub fn validate_register(register: Option<&[ResearchInstrumentData]>) -> bool {
    let register = register.unwrap_or(data_instruments::ALL);
    let mut count = 0;
    for instrument in register {
        let label = format!("research_instruments.{}", instrument.id);
        assert!(!instrument.name.is_empty(), "{label} needs a name");
        assert!(
            !instrument.source_register_key.is_empty(),
            "{label} must name its source register key"
        );
        assert!(
            !instrument.item_text_included,
            "{label} must not embed instrument item text"
        );
        assert!(
            !instrument.license.is_empty(),
            "{label} needs license terms"
        );
        assert!(
            is_finite(instrument.scale_min)
                && is_finite(instrument.scale_max)
                && instrument.scale_min < instrument.scale_max,
            "{label} needs an ordered finite scale"
        );
        assert!(
            is_finite(instrument.scale_step) && instrument.scale_step > 0.0,
            "{label} needs a positive scale step"
        );
        assert!(
            instrument.analysis_role != data_instruments::ResearchAnalysisRole::Primary
                || instrument.validated,
            "{label} cannot be a primary outcome without a validated instrument"
        );
        assert!(
            instrument.score_aggregation
                != data_instruments::ResearchScoreAggregation::MeanAllItems
                || instrument.requires_all_items,
            "{label} mean scoring requires the complete-item rule"
        );
        assert!(
            !instrument.pooling_group.is_empty(),
            "{label} needs a pooling group"
        );
        if let Some(target_id) = instrument.substitute_for {
            let target = register
                .iter()
                .find(|i| i.id == target_id)
                .unwrap_or_else(|| {
                    panic!("{label} substitutes for unknown instrument {target_id}")
                });
            assert!(
                target.validated && !instrument.validated,
                "{label} may only substitute for a validated instrument it does not replace in kind"
            );
        }

        let mut constructs: Vec<(&str, i64)> = Vec::new();
        for &construct in instrument.constructs {
            assert!(
                !constructs.iter().any(|(c, _)| *c == construct),
                "{label} repeats construct {construct}"
            );
            constructs.push((construct, 0));
        }
        let mut positions: Vec<i64> = Vec::new();
        let mut item_ids: Vec<&str> = Vec::new();
        for (index, item) in instrument.items.iter().enumerate() {
            let item_label = format!("{label}.items.{}", index + 1);
            assert!(!item.id.is_empty(), "{item_label} needs an id");
            assert!(
                !item_ids.contains(&item.id),
                "{item_label} duplicates item id {}",
                item.id
            );
            item_ids.push(item.id);
            let entry = constructs
                .iter_mut()
                .find(|(c, _)| *c == item.construct)
                .unwrap_or_else(|| {
                    panic!("{item_label} has unregistered construct {}", item.construct)
                });
            entry.1 += 1;
            assert!(
                item.position == (index + 1) as i64,
                "{item_label} position must match canonical order"
            );
            assert!(
                !positions.contains(&item.position),
                "{item_label} duplicates a position"
            );
            positions.push(item.position);
        }
        for (construct, item_count) in &constructs {
            assert!(
                *item_count > 0,
                "{label} construct {construct} has no items"
            );
            if instrument.score_aggregation == data_instruments::ResearchScoreAggregation::ItemOnly
            {
                assert!(
                    *item_count == 1,
                    "{label} item-only construct {construct} must be a single item"
                );
            }
        }
        count += 1;
    }
    assert!(count > 0, "research instrument register is empty");
    true
}

/// Look up a registered instrument by id.
pub fn instrument(instrument_id: &str) -> Result<&'static ResearchInstrumentData> {
    data_instruments::get(instrument_id)
        .ok_or_else(|| format!("unknown instrument {instrument_id}"))
}

fn on_scale(instrument: &ResearchInstrumentData, value: f64) -> bool {
    if value < instrument.scale_min - SCALE_EPSILON {
        return false;
    }
    if value > instrument.scale_max + SCALE_EPSILON {
        return false;
    }
    let steps = (value - instrument.scale_min) / instrument.scale_step;
    (steps - (steps + 0.5).floor()).abs() < 1e-6
}

fn text_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a str {
    Value::record_get(entries, name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("validated field {name} missing or not text"))
}

fn opt_text_field<'a>(entries: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    Value::record_get(entries, name).and_then(Value::as_str)
}

fn opt_number_field(entries: &[(String, Value)], name: &str) -> Option<f64> {
    Value::record_get(entries, name).and_then(Value::as_number)
}

fn bool_field(entries: &[(String, Value)], name: &str) -> bool {
    Value::record_get(entries, name)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("validated field {name} missing or not boolean"))
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

/// Recompute construct scores from raw responses. Constructs with any
/// missing item get no score and are reported as incomplete; item-only
/// instruments are never aggregated. `(scores, incomplete)`.
pub fn recompute_scores(set: &Value) -> Result<(Vec<Value>, Vec<String>)> {
    let set = set
        .as_record()
        .ok_or_else(|| "research response set must be a table".to_string())?;
    let instrument_id = text_field(set, "instrument_id");
    let instrument = instrument(instrument_id)?;

    // construct_of[item_id] = construct, expected_counts[construct] = count
    let mut construct_of: Vec<(&str, &str)> = Vec::new();
    let mut expected_counts: Vec<(&str, i64)> = Vec::new();
    for item in instrument.items {
        construct_of.push((item.id, item.construct));
        if let Some(entry) = expected_counts
            .iter_mut()
            .find(|(c, _)| *c == item.construct)
        {
            entry.1 += 1;
        } else {
            expected_counts.push((item.construct, 1));
        }
    }

    let mut sums: Vec<(&str, f64)> = Vec::new();
    let mut answered: Vec<(&str, i64)> = Vec::new();
    let responses = array_field(set, "responses");
    for (index, response) in responses.iter().enumerate() {
        let response_entries = response.as_record().expect("validated response record");
        let item_id = text_field(response_entries, "item_id");
        let Some(&(_, construct)) = construct_of.iter().find(|(id, _)| *id == item_id) else {
            return Err(format!(
                "research_response_set.responses.{} is not an instrument item",
                index + 1
            ));
        };
        if let Some(raw) = opt_number_field(response_entries, "raw_response") {
            if let Some(entry) = sums.iter_mut().find(|(c, _)| *c == construct) {
                entry.1 += raw;
            } else {
                sums.push((construct, raw));
            }
            if let Some(entry) = answered.iter_mut().find(|(c, _)| *c == construct) {
                entry.1 += 1;
            } else {
                answered.push((construct, 1));
            }
        }
    }

    let mut scores: Vec<Value> = Vec::new();
    let mut incomplete: Vec<String> = Vec::new();
    for &construct in instrument.constructs {
        let count = answered
            .iter()
            .find(|(c, _)| *c == construct)
            .map_or(0, |(_, n)| *n);
        let expected = expected_counts
            .iter()
            .find(|(c, _)| *c == construct)
            .map_or(0, |(_, n)| *n);
        if instrument.score_aggregation == data_instruments::ResearchScoreAggregation::ItemOnly {
            incomplete.push(construct.to_string());
        } else if count == expected && expected > 0 {
            let sum = sums
                .iter()
                .find(|(c, _)| *c == construct)
                .map_or(0.0, |(_, s)| *s);
            scores.push(Value::Record(vec![
                ("construct".to_string(), Value::str(construct)),
                ("score".to_string(), Value::Number(sum / count as f64)),
                ("rule".to_string(), Value::str("mean_all_items")),
                ("item_count".to_string(), Value::Number(count as f64)),
            ]));
        } else {
            incomplete.push(construct.to_string());
        }
    }
    scores.sort_by(|a, b| {
        let ac = text_field(a.as_record().unwrap(), "construct");
        let bc = text_field(b.as_record().unwrap(), "construct");
        ac.cmp(bc)
    });
    incomplete.sort();
    Ok((scores, incomplete))
}

/// Validate a response set against [`shape`] plus every cross-field and
/// register-joined invariant.
pub fn validate(set: &Value) -> Result<()> {
    let entries_for_version = set
        .as_record()
        .ok_or_else(|| "research response set must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&shape(), set)?;
    let entries = set.as_record().expect("validated record");

    let instrument_id = text_field(entries, "instrument_id");
    let instrument = instrument(instrument_id)?;

    if text_field(entries, "instrument_version") != instrument.instrument_version {
        return Err(format!(
            "research_response_set.instrument_version {} does not match registered {}",
            text_field(entries, "instrument_version"),
            instrument.instrument_version
        ));
    }
    if text_field(entries, "scoring_key_version") != instrument.scoring_key_version {
        return Err(format!(
            "research_response_set.scoring_key_version {} does not match registered {}",
            text_field(entries, "scoring_key_version"),
            instrument.scoring_key_version
        ));
    }
    let analysis_role_wire = match instrument.analysis_role {
        data_instruments::ResearchAnalysisRole::Primary => "primary",
        data_instruments::ResearchAnalysisRole::Diagnostic => "diagnostic",
        data_instruments::ResearchAnalysisRole::Exploratory => "exploratory",
    };
    if text_field(entries, "analysis_role") != analysis_role_wire {
        return Err(
            "research_response_set.analysis_role contradicts the instrument register".to_string(),
        );
    }
    if bool_field(entries, "validated_instrument") != instrument.validated {
        return Err(
            "research_response_set.validated_instrument contradicts the register".to_string(),
        );
    }
    if bool_field(entries, "partial_administration") != instrument.partial_administration {
        return Err(
            "research_response_set.partial_administration contradicts the register".to_string(),
        );
    }

    let locale = record_field(entries, "locale");
    let translation_provenance = text_field(locale, "translation_provenance");
    let translation_id = opt_text_field(locale, "translation_id");
    if translation_provenance != "original" && translation_id.is_none() {
        return Err(
            "research_response_set.locale translations must name their provenance id".to_string(),
        );
    }
    if translation_provenance == "original" && translation_id.is_some() {
        return Err(
            "research_response_set.locale original administration has no translation id"
                .to_string(),
        );
    }

    let registered_items: Vec<&str> = instrument.items.iter().map(|i| i.id).collect();
    let presentation_order = array_field(entries, "presentation_order");
    let mut ordered: Vec<&str> = Vec::new();
    for (index, item_id) in presentation_order.iter().enumerate() {
        let item_id = item_id.as_str().expect("validated id");
        if !registered_items.contains(&item_id) {
            return Err(format!(
                "research_response_set.presentation_order.{} is not an instrument item",
                index + 1
            ));
        }
        if ordered.contains(&item_id) {
            return Err(format!(
                "research_response_set.presentation_order.{} is duplicated",
                index + 1
            ));
        }
        ordered.push(item_id);
    }
    if presentation_order.len() != instrument.items.len() {
        return Err(
            "research_response_set.presentation_order must cover every administered instrument item".to_string(),
        );
    }

    let mut seen: Vec<&str> = Vec::new();
    let responses = array_field(entries, "responses");
    for (index, response) in responses.iter().enumerate() {
        let path = format!("research_response_set.responses.{}", index + 1);
        let response_entries = response.as_record().expect("validated response");
        let item_id = text_field(response_entries, "item_id");
        if !registered_items.contains(&item_id) {
            return Err(format!("{path} is not an instrument item"));
        }
        if seen.contains(&item_id) {
            return Err(format!("{path} duplicates item {item_id}"));
        }
        seen.push(item_id);
        let raw_response = opt_number_field(response_entries, "raw_response");
        let missing_reason = opt_text_field(response_entries, "missing_reason");
        if raw_response.is_none() && missing_reason.is_none() {
            return Err(format!(
                "{path} needs a raw response or a structured missingness reason"
            ));
        }
        if raw_response.is_some() && missing_reason.is_some() {
            return Err(format!("{path} cannot be both answered and missing"));
        }
        if let Some(raw) = raw_response
            && !on_scale(instrument, raw)
        {
            return Err(format!("{path} is off the registered response scale"));
        }
    }
    if responses.len() != instrument.items.len() {
        return Err(
            "research_response_set.responses must carry one row per administered item, answered or missing"
                .to_string(),
        );
    }

    let (scores, incomplete) = recompute_scores(set)?;
    let set_scores = array_field(entries, "scores");
    if instrument.score_aggregation == data_instruments::ResearchScoreAggregation::ItemOnly
        && !set_scores.is_empty()
    {
        return Err(format!(
            "research_response_set.scores is forbidden for item-only instrument {}",
            instrument.id
        ));
    }
    if set_scores.len() != scores.len() {
        return Err(format!(
            "research_response_set.scores must contain exactly the scorable constructs ({} expected)",
            scores.len()
        ));
    }
    for (index, score) in set_scores.iter().enumerate() {
        let score_entries = score.as_record().expect("validated score");
        let construct = text_field(score_entries, "construct");
        let Some(expected) = scores
            .iter()
            .find(|expected| text_field(expected.as_record().unwrap(), "construct") == construct)
        else {
            return Err(format!(
                "research_response_set.scores.{} scores construct {construct} which is incomplete or not aggregatable",
                index + 1
            ));
        };
        let expected_entries = expected.as_record().unwrap();
        let rule = text_field(score_entries, "rule");
        let item_count = opt_number_field(score_entries, "item_count").expect("validated integer");
        let expected_rule = text_field(expected_entries, "rule");
        let expected_item_count =
            opt_number_field(expected_entries, "item_count").expect("validated integer");
        if rule != expected_rule || (item_count - expected_item_count).abs() > f64::EPSILON {
            return Err(format!(
                "research_response_set.scores.{} uses the wrong scoring rule",
                index + 1
            ));
        }
        let score_value = opt_number_field(score_entries, "score").expect("validated number");
        let expected_score = opt_number_field(expected_entries, "score").expect("validated number");
        if (score_value - expected_score).abs() > 1e-9 {
            return Err(format!(
                "research_response_set.scores.{} disagrees with its raw responses",
                index + 1
            ));
        }
    }
    for construct in &incomplete {
        if set_scores
            .iter()
            .any(|s| text_field(s.as_record().unwrap(), "construct") == construct)
        {
            return Err(format!(
                "research_response_set.scores must omit incomplete construct {construct}"
            ));
        }
    }
    Ok(())
}

/// Orphan-join guard for the trace side. A response set that names a trace
/// must name one the caller actually holds. Mirrors
/// `research_trace::validate_against_stream`. `manifest` is a
/// [`crate::research_trace`]-shaped [`Value`]; read the same way
/// `research_trace.rs` reads its own fields, so the field names/nesting here
/// (`manifest.trace_id`, `manifest.simulation.last_boundary_tick`) match
/// exactly what `research_trace::shape` declares.
pub fn validate_against_trace(set: &Value, manifest: &Value) -> Result<()> {
    validate(set)?;
    let manifest_entries = manifest
        .as_record()
        .ok_or_else(|| "research trace manifest is required".to_string())?;
    let trace_id = opt_text_field(manifest_entries, "trace_id")
        .ok_or_else(|| "research trace manifest is required".to_string())?;
    let entries = set.as_record().expect("validated record");
    let timing = record_field(entries, "timing");
    let Some(set_trace_id) = opt_text_field(timing, "trace_id") else {
        return Err(
            "research_response_set.timing.trace_id is required to join a trace".to_string(),
        );
    };
    if set_trace_id != trace_id {
        return Err("research_response_set.timing.trace_id is an orphan join".to_string());
    }
    if let Some(canonical_boundary_tick) = opt_number_field(timing, "canonical_boundary_tick") {
        let simulation = record_field(manifest_entries, "simulation");
        let last_boundary_tick =
            opt_number_field(simulation, "last_boundary_tick").expect("validated integer");
        if canonical_boundary_tick > last_boundary_tick {
            return Err(
                "research_response_set.timing.canonical_boundary_tick is past the trace"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// Reject an administration set that pools instruments the register
/// forbids pooling, and reject an accessibility fallback presented
/// alongside the validated instrument it replaces.
pub fn validate_administration(sets: &[Value]) -> Result<()> {
    let mut present: Vec<(String, &Value)> = Vec::new();
    let mut ids: Vec<&str> = Vec::new();
    for set in sets {
        validate(set)?;
        let entries = set.as_record().expect("validated record");
        let response_set_id = text_field(entries, "response_set_id");
        if ids.contains(&response_set_id) {
            return Err(format!(
                "research response set {response_set_id} is duplicated"
            ));
        }
        ids.push(response_set_id);
        let timing = record_field(entries, "timing");
        let key = format!(
            "{}/{}/{}",
            text_field(entries, "instrument_id"),
            text_field(entries, "condition_id"),
            text_field(timing, "relative_to_play")
        );
        if present.iter().any(|(k, _)| k == &key) {
            return Err(format!("research response administration repeats {key}"));
        }
        present.push((key, set));
    }
    // A declared substitute replaces the instrument it stands in for;
    // presenting both at one timepoint would invite pooling two
    // non-equivalent measures.
    for (key, set) in &present {
        let entries = set.as_record().expect("validated record");
        let instrument_id = text_field(entries, "instrument_id");
        let instrument = instrument(instrument_id).expect("already validated");
        if let Some(substitute_for) = instrument.substitute_for {
            let timing = record_field(entries, "timing");
            let condition_id = text_field(entries, "condition_id");
            let relative_to_play = text_field(timing, "relative_to_play");
            let conflict = format!("{substitute_for}/{condition_id}/{relative_to_play}");
            if present.iter().any(|(k, _)| k == &conflict) {
                return Err(format!(
                    "research response administration presents {key} together with its non-poolable substitute target {conflict}"
                ));
            }
        }
    }
    Ok(())
}

/// Orphan-join and missingness-contradiction guard for the session side. A
/// session that declares an instrument was not administered cannot also
/// carry a response set for that instrument.
pub fn validate_against_session(set: &Value, envelope: &Value) -> Result<()> {
    validate(set)?;
    let entries = set.as_record().expect("validated record");
    let envelope_entries = envelope
        .as_record()
        .ok_or_else(|| "research session envelope is required".to_string())?;
    let envelope_session_id = opt_text_field(envelope_entries, "session_id")
        .ok_or_else(|| "research session envelope is required".to_string())?;
    if text_field(entries, "session_id") != envelope_session_id {
        return Err("research_response_set.session_id is an orphan join".to_string());
    }
    let lifecycle = record_field(envelope_entries, "lifecycle");
    if text_field(lifecycle, "status") == "withdrawn" {
        return Err("research_response_set belongs to a withdrawn session".to_string());
    }
    let instrument_id = text_field(entries, "instrument_id");
    for (index, missing) in array_field(lifecycle, "missingness").iter().enumerate() {
        let missing_entries = missing.as_record().expect("validated missingness row");
        if text_field(missing_entries, "target_kind") == "instrument"
            && text_field(missing_entries, "target_id") == instrument_id
            && text_field(missing_entries, "reason_code") == "instrument_not_administered"
        {
            return Err(format!(
                "research_session_envelope.lifecycle.missingness.{} says {instrument_id} was not administered but a response set exists",
                index + 1
            ));
        }
    }
    Ok(())
}

/// Content hash of a valid response set.
pub fn content_hash(set: &Value) -> Result<String> {
    validate(set)?;
    research_schema::content_hash(&shape(), set)
}

/// Canonical bytes of a valid response set.
pub fn encode(set: &Value) -> Result<Vec<u8>> {
    validate(set)?;
    research_schema::encode(&shape(), set)
}

/// Decode and re-validate a response set.
pub fn decode(bytes: &[u8]) -> Result<Value> {
    let set = research_schema::decode(&shape(), bytes)?;
    validate(&set)?;
    Ok(set)
}
