//! Tests for `gc_sim::research_features`.

use gc_sim::research_features;
use gc_sim::research_schema::{self, Value};

fn set_field(value: &Value, name: &str, new_value: Value) -> Value {
    let entries = value.as_record().expect("record value").to_vec();
    let mut updated: Vec<(String, Value)> =
        entries.into_iter().filter(|(key, _)| key != name).collect();
    updated.push((name.to_string(), new_value));
    Value::Record(updated)
}

fn field_str<'a>(value: &'a Value, name: &str) -> &'a str {
    let entries = value.as_record().expect("record value");
    Value::record_get(entries, name)
        .and_then(Value::as_str)
        .expect("field present")
}

fn field_bool(value: &Value, name: &str) -> bool {
    let entries = value.as_record().expect("record value");
    Value::record_get(entries, name)
        .and_then(Value::as_bool)
        .expect("field present")
}

#[test]
fn research_feature_register_accepts_the_authored_register() {
    assert!(research_features::validate_registry());
    assert!(research_features::ids().len() > 20);
}

#[test]
fn research_feature_register_registers_every_instrument_construct_exactly_once() {
    let mut expected = 0;
    for instrument in gc_data::research_instruments::ALL {
        for &construct in instrument.constructs {
            expected += 1;
            let id = research_features::feature_id(instrument.id, construct);
            let feature = research_features::feature(&id).expect("registered feature");
            assert_eq!(field_str(&feature, "evidence_tier"), "human_experience");
        }
    }
    assert_eq!(expected, 24, "the instrument register changed shape");
    assert_eq!(research_features::ids().len(), expected + 7);
}

#[test]
fn research_feature_register_returns_copies_so_callers_cannot_edit_the_register() {
    let feature = research_features::feature("soccer_shape_proxy_score").unwrap();
    let _mutated = set_field(&feature, "human_fun_claim", Value::Bool(true));
    let refetched = research_features::feature("soccer_shape_proxy_score").unwrap();
    assert!(!field_bool(&refetched, "human_fun_claim"));
}

#[test]
fn fun_score_tagging_registers_metrics_fun_score_as_a_soccer_shape_proxy_never_human_fun() {
    let feature = research_features::feature("soccer_shape_proxy_score").unwrap();
    assert_eq!(field_str(&feature, "evidence_tier"), "soccer_shape_proxy");
    assert!(!field_bool(&feature, "human_fun_claim"));
    assert_eq!(field_str(&feature, "outcome_role"), "diagnostic");
    assert_eq!(field_str(&feature, "leakage_risk"), "outcome_derived");
    assert_eq!(
        field_str(&feature, "observability"),
        "privileged_diagnostic"
    );
    assert!(field_str(&feature, "description").contains("not a measurement of human fun"));

    let err =
        research_features::use_allowed("soccer_shape_proxy_score", "human_fun_claim").unwrap_err();
    assert!(err.contains("prohibits use"), "{err}");
    assert!(research_features::use_allowed("soccer_shape_proxy_score", "primary_outcome").is_err());
    assert!(research_features::use_allowed("soccer_shape_proxy_score", "model_input").is_ok());
}

#[test]
fn fun_score_tagging_keeps_the_human_fun_claim_on_the_primary_enjoyment_endpoint_only() {
    let mut claims = Vec::new();
    for id in research_features::ids() {
        let feature = research_features::feature(&id).unwrap();
        if field_bool(&feature, "human_fun_claim") {
            claims.push(id);
        }
    }
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0], "pxi_enjoyment_addon.enjoyment");

    let enjoyment = research_features::feature("pxi_enjoyment_addon.enjoyment").unwrap();
    assert_eq!(field_str(&enjoyment, "outcome_role"), "primary_outcome");
    assert_eq!(field_str(&enjoyment, "leakage_risk"), "none");
    assert_eq!(field_str(&enjoyment, "evidence_tier"), "human_experience");
}

#[test]
fn fun_score_tagging_prohibits_promoting_a_diagnostic_or_exploratory_construct() {
    for id in [
        "pxi_partial_mechanisms.autonomy",
        "bangs_session.competence_frustration",
        "custom_diagnostics.fairness",
        "affective_slider.valence",
    ] {
        let err = research_features::use_allowed(id, "primary_outcome");
        assert!(err.is_err(), "{id} must prohibit primary_outcome use");
    }
}

#[test]
fn research_feature_observability_categories_has_a_worked_example_for_every_observability_category()
{
    let mut seen: Vec<String> = Vec::new();
    for id in research_features::ids() {
        let feature = research_features::feature(&id).unwrap();
        let observability = field_str(&feature, "observability").to_string();
        if !seen.contains(&observability) {
            seen.push(observability);
        }
    }
    for category in [
        "player_observable",
        "privileged_diagnostic",
        "outcome_derived",
        "protected_sensitive",
        "prohibited",
    ] {
        assert!(
            seen.iter().any(|s| s == category),
            "no registered feature exercises {category}"
        );
    }
}

#[test]
fn research_feature_observability_categories_keeps_prohibited_and_sensitive_features_out_of_every_use_it_names()
 {
    let prohibited = research_features::feature("inferred_participant_skill_trait").unwrap();
    assert_eq!(field_str(&prohibited, "observability"), "prohibited");
    assert_eq!(field_str(&prohibited, "outcome_role"), "feature_only");
    assert!(
        research_features::use_allowed("inferred_participant_skill_trait", "any_analysis").is_err()
    );
    assert!(
        research_features::use_allowed("inferred_participant_skill_trait", "model_input").is_err()
    );

    let sensitive = research_features::feature("declared_readability_settings").unwrap();
    assert_eq!(
        field_str(&sensitive, "observability"),
        "protected_sensitive"
    );
    assert!(
        research_features::use_allowed("declared_readability_settings", "disability_inference")
            .is_err()
    );

    let outcome = research_features::feature("final_score_margin").unwrap();
    assert_eq!(field_str(&outcome, "observability"), "outcome_derived");
    assert_eq!(field_str(&outcome, "leakage_risk"), "outcome_derived");
    assert!(research_features::use_allowed("final_score_margin", "model_input").is_err());
}

#[test]
fn research_feature_grains_never_treats_a_tick_match_or_encounter_row_as_a_participant() {
    for id in research_features::ids() {
        let feature = research_features::feature(&id).unwrap();
        let grain = field_str(&feature, "grain");
        if research_features::is_clustered_grain(grain) {
            assert_ne!(
                field_str(&feature, "pseudo_replication_guard"),
                "independent_unit_participant",
                "{id} claims participant independence at grain {grain}"
            );
        }
    }
}

#[test]
fn research_feature_grains_only_allows_the_aggregation_levels_it_defines() {
    assert!(
        research_features::aggregation_allowed("pxi_enjoyment_addon.enjoyment", "condition_block")
            .is_ok()
    );
    let err = research_features::aggregation_allowed("pxi_enjoyment_addon.enjoyment", "tick")
        .unwrap_err();
    assert!(err.contains("aggregation level"), "{err}");
    assert!(research_features::aggregation_allowed("soccer_shape_proxy_score", "session").is_err());
    assert!(research_features::aggregation_allowed("no_such_feature", "match").is_err());
}

#[test]
fn research_feature_grains_rejects_a_feature_that_violates_the_register_invariants() {
    let base = research_features::feature("involuntary_disable_share").unwrap();
    assert!(research_features::validate_feature(&base, None).is_ok());

    let pseudo_replicating = set_field(
        &base,
        "pseudo_replication_guard",
        Value::str("independent_unit_participant"),
    );
    let err = research_features::validate_feature(&pseudo_replicating, None).unwrap_err();
    assert!(err.contains("independent participants"), "{err}");

    let proxy_claiming_fun = set_field(
        &research_features::feature("soccer_shape_proxy_score").unwrap(),
        "human_fun_claim",
        Value::Bool(true),
    );
    assert!(research_features::validate_feature(&proxy_claiming_fun, None).is_err());

    let leaky_primary = set_field(&base, "outcome_role", Value::str("primary_outcome"));
    assert!(research_features::validate_feature(&leaky_primary, None).is_err());

    let missing_window = set_field(
        &research_features::feature("combat_to_soccer_conversion_rate").unwrap(),
        "causal_window",
        Value::Record(vec![("kind".to_string(), Value::str("forward_ticks"))]),
    );
    assert!(research_features::validate_feature(&missing_window, None).is_err());

    let unregistered_grain = set_field(
        &base,
        "aggregation_levels",
        Value::Array(vec![Value::str("session")]),
    );
    assert!(research_features::validate_feature(&unregistered_grain, None).is_err());
}

#[test]
fn research_feature_register_hash_is_stable_and_covers_every_feature_definition() {
    let hash = research_features::registry_hash();
    assert_eq!(hash.len(), research_schema::HASH_LENGTH);
    assert_eq!(hash, research_features::registry_hash());

    let feature = research_features::feature("involuntary_disable_share").unwrap();
    let before = research_schema::content_hash(&research_features::shape(), &feature).unwrap();
    let changed = set_field(
        &feature,
        "causal_window",
        Value::Record(vec![
            ("kind".to_string(), Value::str("forward_ticks")),
            ("ticks".to_string(), Value::Number(60.0)),
        ]),
    );
    let after = research_schema::content_hash(&research_features::shape(), &changed).unwrap();
    assert_ne!(before, after);
}
