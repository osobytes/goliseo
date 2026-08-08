//! Port of `sim/research_features.lua`.
//!
//! Derived feature register: expansion, invariants, and lookup.
//!
//! The register is authored content (`gc_data::research_features` plus
//! `gc_data::research_instruments`), so structural violations `assert!`. The
//! invariants that matter are the ones a model or a report could otherwise
//! violate quietly:
//!
//!   * only a `human_experience` feature may be cited as human-fun evidence,
//!     so `metrics.fun_score` is registered as `soccer_shape_proxy_score`
//!     with `human_fun_claim = false` and `human_fun_claim` in its
//!     prohibited uses;
//!   * every instrument construct has exactly one feature and every
//!     instrument-derived feature has a construct, so a construct cannot be
//!     analysed without a definition; and
//!   * a tick/decision/encounter-grain feature may never declare a
//!     participant as its independent unit, which is the pseudo-replication
//!     trap.
//!
//! ## `assert!` vs `Result` here
//!
//! [`build_registry`]/[`assert_feature_invariants`] `assert!`: the register
//! is authored content, so a broken registry entry is a code bug (AGENTS.md
//! §7). [`validate_feature`] is the one place the *same* invariant check
//! also needs to answer "is this externally supplied feature valid" as a
//! `Result` — mirroring the Lua original's `pcall(assert_feature_invariants,
//! ...)`, this wraps the assertion call in [`std::panic::catch_unwind`]
//! rather than duplicating every invariant into a second, parallel
//! `Result`-returning implementation.

use crate::research_schema::{
    self, ResearchField, ResearchFieldKind, ResearchShape, Result, TuplePart, Value,
};
use gc_data::research_features as data_features;
use gc_data::research_instruments as data_instruments;
use std::sync::OnceLock;

/// Reader version for the derived feature register wire shape.
pub const VERSION: i64 = 1;
/// Serialization versions this reader accepts.
pub const SUPPORTED_VERSIONS: &[i64] = &[1];
/// Label under which the whole register's content hash is computed.
pub const REGISTRY_LABEL: &str = "research-feature-registry/v1";

/// Grains whose rows are not independent participants. A feature at these
/// grains must cluster, never treat a row as a participant-level unit.
const CLUSTERED_GRAINS: &[&str] = &[
    "tick",
    "decision",
    "possession",
    "encounter",
    "match",
    "player_match",
];

/// Every declared grain, in the closed enum's canonical order.
#[must_use]
pub fn grains() -> Vec<String> {
    research_schema::enum_values(&[
        "tick",
        "decision",
        "possession",
        "encounter",
        "match",
        "player_match",
        "condition_block",
        "session",
        "participant",
    ])
}

/// Is `grain` one of the [`CLUSTERED_GRAINS`] (rows that are not
/// independent participants)?
#[must_use]
pub fn is_clustered_grain(grain: &str) -> bool {
    CLUSTERED_GRAINS.contains(&grain)
}

/// The research-feature record shape.
#[must_use]
pub fn shape() -> ResearchShape {
    let causal_window_fields = vec![
        ResearchField::new(ResearchFieldKind::Enum)
            .named("kind")
            .values(research_schema::enum_values(&[
                "same_tick",
                "forward_ticks",
                "backward_ticks",
                "whole_match",
                "post_condition",
            ])),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("ticks")
            .optional()
            .min(1.0),
    ];
    research_schema::record(
        "research_feature/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Id).named("id"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Str)
                .named("description")
                .max_length(512),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("grain")
                .values(grains()),
            ResearchField::new(ResearchFieldKind::Array)
                .named("source_schemas")
                .min_length(1)
                .element(ResearchField::new(ResearchFieldKind::Str).named("schema")),
            ResearchField::new(ResearchFieldKind::Array)
                .named("source_fields")
                .min_length(1)
                .element(ResearchField::new(ResearchFieldKind::Str).named("field")),
            ResearchField::new(ResearchFieldKind::Str).named("extraction_module"),
            ResearchField::new(ResearchFieldKind::Id).named("extraction_config_id"),
            ResearchField::new(ResearchFieldKind::Str).named("numerator"),
            ResearchField::new(ResearchFieldKind::Str)
                .named("denominator")
                .optional(),
            ResearchField::new(ResearchFieldKind::Id).named("unit"),
            ResearchField::new(ResearchFieldKind::Array)
                .named("exclusions")
                .element(ResearchField::new(ResearchFieldKind::Str).named("exclusion")),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("missing_value_behavior")
                .values(research_schema::enum_values(&[
                    "na_row",
                    "drop_row",
                    "zero_when_defined",
                    "protocol_failure",
                ])),
            ResearchField::new(ResearchFieldKind::Record)
                .named("causal_window")
                .fields(causal_window_fields),
            ResearchField::new(ResearchFieldKind::Id).named("normalization"),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("leakage_risk")
                .values(research_schema::enum_values(&[
                    "none",
                    "outcome_derived",
                    "privileged_state",
                    "future_information",
                    "label_adjacent",
                ])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("observability")
                .values(research_schema::enum_values(&[
                    "player_observable",
                    "privileged_diagnostic",
                    "outcome_derived",
                    "protected_sensitive",
                    "prohibited",
                ])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("evidence_tier")
                .values(research_schema::enum_values(&[
                    "human_experience",
                    "behavioral_proxy",
                    "soccer_shape_proxy",
                    "machine_diagnostic",
                ])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("outcome_role")
                .values(research_schema::enum_values(&[
                    "primary_outcome",
                    "secondary_outcome",
                    "guardrail",
                    "diagnostic",
                    "exploratory",
                    "feature_only",
                ])),
            ResearchField::new(ResearchFieldKind::Array)
                .named("aggregation_levels")
                .min_length(1)
                .element(
                    ResearchField::new(ResearchFieldKind::Enum)
                        .named("level")
                        .values(grains()),
                ),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("pseudo_replication_guard")
                .values(research_schema::enum_values(&[
                    "independent_unit_participant",
                    "cluster_by_participant",
                    "cluster_by_match",
                    "not_an_analysis_unit",
                ])),
            ResearchField::new(ResearchFieldKind::Array)
                .named("confounds")
                .min_length(1)
                .element(ResearchField::new(ResearchFieldKind::Str).named("confound")),
            ResearchField::new(ResearchFieldKind::Str).named("goodhart_failure"),
            ResearchField::new(ResearchFieldKind::Array)
                .named("prohibited_uses")
                .element(ResearchField::new(ResearchFieldKind::Id).named("use")),
            ResearchField::new(ResearchFieldKind::Boolean).named("human_fun_claim"),
        ],
    )
}

/// `instrument_id.construct`, this register's feature id grammar.
#[must_use]
pub fn feature_id(instrument_id: &str, construct: &str) -> String {
    format!("{instrument_id}.{construct}")
}

fn grain_wire(grain: data_features::ResearchFeatureGrain) -> &'static str {
    use data_features::ResearchFeatureGrain::{
        ConditionBlock, Decision, Encounter, Match, Participant, PlayerMatch, Possession, Session,
        Tick,
    };
    match grain {
        Tick => "tick",
        Decision => "decision",
        Possession => "possession",
        Encounter => "encounter",
        Match => "match",
        PlayerMatch => "player_match",
        ConditionBlock => "condition_block",
        Session => "session",
        Participant => "participant",
    }
}

fn leakage_wire(v: data_features::ResearchFeatureLeakage) -> &'static str {
    use data_features::ResearchFeatureLeakage::{
        FutureInformation, LabelAdjacent, None as LeakageNone, OutcomeDerived, PrivilegedState,
    };
    match v {
        LeakageNone => "none",
        OutcomeDerived => "outcome_derived",
        PrivilegedState => "privileged_state",
        FutureInformation => "future_information",
        LabelAdjacent => "label_adjacent",
    }
}

fn observability_wire(v: data_features::ResearchFeatureObservability) -> &'static str {
    use data_features::ResearchFeatureObservability::{
        OutcomeDerived, PlayerObservable, PrivilegedDiagnostic, Prohibited, ProtectedSensitive,
    };
    match v {
        PlayerObservable => "player_observable",
        PrivilegedDiagnostic => "privileged_diagnostic",
        OutcomeDerived => "outcome_derived",
        ProtectedSensitive => "protected_sensitive",
        Prohibited => "prohibited",
    }
}

fn evidence_tier_wire(v: data_features::ResearchEvidenceTier) -> &'static str {
    use data_features::ResearchEvidenceTier::{
        BehavioralProxy, HumanExperience, MachineDiagnostic, SoccerShapeProxy,
    };
    match v {
        HumanExperience => "human_experience",
        BehavioralProxy => "behavioral_proxy",
        SoccerShapeProxy => "soccer_shape_proxy",
        MachineDiagnostic => "machine_diagnostic",
    }
}

fn outcome_role_wire(v: data_features::ResearchOutcomeRole) -> &'static str {
    use data_features::ResearchOutcomeRole::{
        Diagnostic, Exploratory, FeatureOnly, Guardrail, PrimaryOutcome, SecondaryOutcome,
    };
    match v {
        PrimaryOutcome => "primary_outcome",
        SecondaryOutcome => "secondary_outcome",
        Guardrail => "guardrail",
        Diagnostic => "diagnostic",
        Exploratory => "exploratory",
        FeatureOnly => "feature_only",
    }
}

fn missing_value_behavior_wire(v: data_features::ResearchMissingValueBehavior) -> &'static str {
    use data_features::ResearchMissingValueBehavior::{
        DropRow, NaRow, ProtocolFailure, ZeroWhenDefined,
    };
    match v {
        NaRow => "na_row",
        DropRow => "drop_row",
        ZeroWhenDefined => "zero_when_defined",
        ProtocolFailure => "protocol_failure",
    }
}

fn pseudo_replication_guard_wire(v: data_features::ResearchPseudoReplicationGuard) -> &'static str {
    use data_features::ResearchPseudoReplicationGuard::{
        ClusterByMatch, ClusterByParticipant, IndependentUnitParticipant, NotAnAnalysisUnit,
    };
    match v {
        IndependentUnitParticipant => "independent_unit_participant",
        ClusterByParticipant => "cluster_by_participant",
        ClusterByMatch => "cluster_by_match",
        NotAnAnalysisUnit => "not_an_analysis_unit",
    }
}

fn causal_window_kind_wire(v: data_features::ResearchCausalWindowKind) -> &'static str {
    use data_features::ResearchCausalWindowKind::{
        BackwardTicks, ForwardTicks, PostCondition, SameTick, WholeMatch,
    };
    match v {
        SameTick => "same_tick",
        ForwardTicks => "forward_ticks",
        BackwardTicks => "backward_ticks",
        WholeMatch => "whole_match",
        PostCondition => "post_condition",
    }
}

fn causal_window_value(window: &data_features::ResearchCausalWindowData) -> Value {
    let mut fields = vec![(
        "kind".to_string(),
        Value::str(causal_window_kind_wire(window.kind)),
    )];
    if let Some(ticks) = window.ticks {
        fields.push(("ticks".to_string(), Value::Number(ticks as f64)));
    }
    Value::Record(fields)
}

/// A registered/authored feature, already fully typed, as a
/// [`research_schema::Value`] matching [`shape`].
fn feature_to_value(f: &data_features::ResearchFeatureData) -> Value {
    let mut fields: Vec<(String, Value)> = vec![
        ("id".to_string(), Value::str(f.id)),
        ("version".to_string(), Value::Number(f.version as f64)),
        ("description".to_string(), Value::str(f.description)),
        ("grain".to_string(), Value::str(grain_wire(f.grain))),
        (
            "source_schemas".to_string(),
            Value::Array(f.source_schemas.iter().map(|s| Value::str(*s)).collect()),
        ),
        (
            "source_fields".to_string(),
            Value::Array(f.source_fields.iter().map(|s| Value::str(*s)).collect()),
        ),
        (
            "extraction_module".to_string(),
            Value::str(f.extraction_module),
        ),
        (
            "extraction_config_id".to_string(),
            Value::str(f.extraction_config_id),
        ),
        ("numerator".to_string(), Value::str(f.numerator)),
    ];
    if let Some(denominator) = f.denominator {
        fields.push(("denominator".to_string(), Value::str(denominator)));
    }
    fields.push(("unit".to_string(), Value::str(f.unit)));
    fields.push((
        "exclusions".to_string(),
        Value::Array(f.exclusions.iter().map(|s| Value::str(*s)).collect()),
    ));
    fields.push((
        "missing_value_behavior".to_string(),
        Value::str(missing_value_behavior_wire(f.missing_value_behavior)),
    ));
    fields.push((
        "causal_window".to_string(),
        causal_window_value(&f.causal_window),
    ));
    fields.push(("normalization".to_string(), Value::str(f.normalization)));
    fields.push((
        "leakage_risk".to_string(),
        Value::str(leakage_wire(f.leakage_risk)),
    ));
    fields.push((
        "observability".to_string(),
        Value::str(observability_wire(f.observability)),
    ));
    fields.push((
        "evidence_tier".to_string(),
        Value::str(evidence_tier_wire(f.evidence_tier)),
    ));
    fields.push((
        "outcome_role".to_string(),
        Value::str(outcome_role_wire(f.outcome_role)),
    ));
    fields.push((
        "aggregation_levels".to_string(),
        Value::Array(
            f.aggregation_levels
                .iter()
                .map(|g| Value::str(grain_wire(*g)))
                .collect(),
        ),
    ));
    fields.push((
        "pseudo_replication_guard".to_string(),
        Value::str(pseudo_replication_guard_wire(f.pseudo_replication_guard)),
    ));
    fields.push((
        "confounds".to_string(),
        Value::Array(f.confounds.iter().map(|s| Value::str(*s)).collect()),
    ));
    fields.push((
        "goodhart_failure".to_string(),
        Value::str(f.goodhart_failure),
    ));
    fields.push((
        "prohibited_uses".to_string(),
        Value::Array(f.prohibited_uses.iter().map(|s| Value::str(*s)).collect()),
    ));
    fields.push((
        "human_fun_claim".to_string(),
        Value::Bool(f.human_fun_claim),
    ));
    Value::Record(fields)
}

/// Expand one instrument construct into its derived feature, using the
/// instrument's shared defaults and the construct's cross-instrument note.
fn expand_construct(
    defaults: &data_features::ResearchInstrumentFeatureDefaults,
    instrument: &data_instruments::ResearchInstrumentData,
    construct: &str,
    note: &data_features::ResearchConstructNoteData,
) -> Value {
    let mut source_fields = Vec::new();
    for item in instrument.items {
        if item.construct == construct {
            source_fields.push(Value::str(format!(
                "research_response_set.responses[{}]",
                item.id
            )));
        }
    }
    let mean_scored =
        instrument.score_aggregation == data_instruments::ResearchScoreAggregation::MeanAllItems;
    let mut prohibited: Vec<Value> = defaults
        .prohibited_uses
        .iter()
        .map(|u| Value::str(*u))
        .collect();
    if note.outcome_role != data_features::ResearchOutcomeRole::PrimaryOutcome {
        prohibited.push(Value::str("primary_outcome"));
    }
    let description = format!(
        "Construct {construct} from {} ({}), scored by {}.",
        instrument.name, instrument.instrument_version, instrument.scoring_key_version
    );
    let numerator = if mean_scored {
        format!("sum of answered {construct} item responses")
    } else {
        format!("raw response for {construct}")
    };
    // Only the primary human-experience endpoint may be cited as evidence
    // about human fun. Everything else is a mechanism, guardrail, or proxy.
    let human_fun_claim = defaults.evidence_tier
        == data_features::ResearchEvidenceTier::HumanExperience
        && note.outcome_role == data_features::ResearchOutcomeRole::PrimaryOutcome;

    let mut fields: Vec<(String, Value)> = vec![
        (
            "id".to_string(),
            Value::str(feature_id(instrument.id, construct)),
        ),
        (
            "version".to_string(),
            Value::Number(data_features::SOURCE.version as f64),
        ),
        ("description".to_string(), Value::str(description)),
        ("grain".to_string(), Value::str(grain_wire(defaults.grain))),
        (
            "source_schemas".to_string(),
            Value::Array(
                defaults
                    .source_schemas
                    .iter()
                    .map(|s| Value::str(*s))
                    .collect(),
            ),
        ),
        ("source_fields".to_string(), Value::Array(source_fields)),
        (
            "extraction_module".to_string(),
            Value::str(defaults.extraction_module),
        ),
        (
            "extraction_config_id".to_string(),
            Value::str(defaults.extraction_config_id),
        ),
        ("numerator".to_string(), Value::str(numerator)),
    ];
    if mean_scored {
        fields.push((
            "denominator".to_string(),
            Value::str(format!(
                "count of {construct} items; every item is required"
            )),
        ));
    }
    fields.push(("unit".to_string(), Value::str(defaults.unit)));
    fields.push((
        "exclusions".to_string(),
        Value::Array(defaults.exclusions.iter().map(|s| Value::str(*s)).collect()),
    ));
    fields.push((
        "missing_value_behavior".to_string(),
        Value::str(missing_value_behavior_wire(defaults.missing_value_behavior)),
    ));
    fields.push((
        "causal_window".to_string(),
        causal_window_value(&defaults.causal_window),
    ));
    fields.push((
        "normalization".to_string(),
        Value::str(defaults.normalization),
    ));
    fields.push((
        "leakage_risk".to_string(),
        Value::str(leakage_wire(defaults.leakage_risk)),
    ));
    fields.push((
        "observability".to_string(),
        Value::str(observability_wire(defaults.observability)),
    ));
    fields.push((
        "evidence_tier".to_string(),
        Value::str(evidence_tier_wire(defaults.evidence_tier)),
    ));
    fields.push((
        "outcome_role".to_string(),
        Value::str(outcome_role_wire(note.outcome_role)),
    ));
    fields.push((
        "aggregation_levels".to_string(),
        Value::Array(
            defaults
                .aggregation_levels
                .iter()
                .map(|g| Value::str(grain_wire(*g)))
                .collect(),
        ),
    ));
    fields.push((
        "pseudo_replication_guard".to_string(),
        Value::str(pseudo_replication_guard_wire(
            defaults.pseudo_replication_guard,
        )),
    ));
    fields.push((
        "confounds".to_string(),
        Value::Array(note.confounds.iter().map(|s| Value::str(*s)).collect()),
    ));
    fields.push((
        "goodhart_failure".to_string(),
        Value::str(note.goodhart_failure),
    ));
    fields.push(("prohibited_uses".to_string(), Value::Array(prohibited)));
    fields.push(("human_fun_claim".to_string(), Value::Bool(human_fun_claim)));
    Value::Record(fields)
}

fn text_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a str {
    Value::record_get(entries, name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("validated feature is missing text field {name}"))
}

fn bool_field(entries: &[(String, Value)], name: &str) -> bool {
    Value::record_get(entries, name)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("validated feature is missing boolean field {name}"))
}

fn array_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [Value] {
    Value::record_get(entries, name)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("validated feature is missing array field {name}"))
}

/// Check the cross-field invariants a lone schema shape cannot express.
///
/// # Panics
///
/// Panics (`assert!`) on any violation: `feature` here is always either
/// already-authored content (from [`build_registry`]) or a value that has
/// already passed [`research_schema::validate`] against [`shape`] (from
/// [`validate_feature`], which converts a panic here into a `Result` via
/// [`std::panic::catch_unwind`] — see the module doc comment).
fn assert_feature_invariants(feature: &Value, label: &str) {
    research_schema::validate(&shape(), feature).unwrap_or_else(|err| panic!("{label}: {err}"));
    let entries = feature.as_record().expect("validated record");
    let human_fun_claim = bool_field(entries, "human_fun_claim");
    let evidence_tier = text_field(entries, "evidence_tier");
    let outcome_role = text_field(entries, "outcome_role");
    let leakage_risk = text_field(entries, "leakage_risk");
    let observability = text_field(entries, "observability");
    let grain = text_field(entries, "grain");

    if human_fun_claim {
        assert!(
            evidence_tier == "human_experience",
            "{label} may not claim human-fun evidence outside a human-experience instrument"
        );
        assert!(
            outcome_role == "primary_outcome",
            "{label} may only claim human-fun evidence as the primary outcome"
        );
    }
    if outcome_role == "primary_outcome" {
        assert!(
            evidence_tier == "human_experience",
            "{label} cannot be a primary outcome without human-experience evidence"
        );
        assert!(
            leakage_risk == "none",
            "{label} primary outcomes must be leakage-free"
        );
    }
    if evidence_tier == "soccer_shape_proxy" {
        let forbids_fun_claim = array_field(entries, "prohibited_uses")
            .iter()
            .any(|v| v.as_str() == Some("human_fun_claim"));
        assert!(
            forbids_fun_claim,
            "{label} is a soccer-shape proxy and must prohibit human_fun_claim"
        );
        assert!(
            !human_fun_claim,
            "{label} soccer-shape proxies are never human fun"
        );
    }
    if observability == "prohibited" {
        assert!(
            outcome_role == "feature_only" && !array_field(entries, "prohibited_uses").is_empty(),
            "{label} prohibited features must record why they are unusable"
        );
    }
    let causal_window = Value::record_get(entries, "causal_window")
        .and_then(Value::as_record)
        .expect("validated causal_window");
    let causal_kind = text_field(causal_window, "kind");
    let has_ticks = Value::record_get(causal_window, "ticks").is_some();
    if causal_kind == "forward_ticks" || causal_kind == "backward_ticks" {
        assert!(
            has_ticks,
            "{label} windowed features must record their window"
        );
    } else {
        assert!(
            !has_ticks,
            "{label} non-windowed features cannot declare a tick window"
        );
    }
    let includes_grain = array_field(entries, "aggregation_levels")
        .iter()
        .any(|v| v.as_str() == Some(grain));
    assert!(
        includes_grain,
        "{label} aggregation levels must include its own grain"
    );
    if is_clustered_grain(grain) {
        let guard = text_field(entries, "pseudo_replication_guard");
        assert!(
            guard != "independent_unit_participant",
            "{label} rows at this grain are not independent participants"
        );
    }
    if Value::record_get(entries, "denominator").is_none() {
        assert!(
            text_field(entries, "normalization") == "none",
            "{label} cannot normalize without a denominator"
        );
    }
}

fn build_registry() -> Vec<(String, Value)> {
    assert!(
        SUPPORTED_VERSIONS.contains(&data_features::SOURCE.version),
        "research feature source version is unsupported"
    );
    let mut registry: Vec<(String, Value)> = Vec::new();

    for &(instrument_id, ref defaults) in data_features::SOURCE.instrument_defaults {
        let instrument = data_instruments::get(instrument_id).unwrap_or_else(|| {
            panic!("research feature defaults name unknown instrument {instrument_id}")
        });
        for &construct in instrument.constructs {
            let note = data_features::construct_note(construct).unwrap_or_else(|| {
                panic!("research construct {construct} has no registered feature notes")
            });
            let value = expand_construct(defaults, instrument, construct, note);
            let id = feature_id(instrument.id, construct);
            assert!(
                !registry.iter().any(|(existing, _)| existing == &id),
                "duplicate research feature {id}"
            );
            assert_feature_invariants(&value, &format!("research feature {id}"));
            registry.push((id, value));
        }
    }

    for feature in data_features::SOURCE.behavioral {
        let value = feature_to_value(feature);
        let id = feature.id.to_string();
        assert!(
            !registry.iter().any(|(existing, _)| existing == &id),
            "duplicate research feature {id}"
        );
        assert_feature_invariants(&value, &format!("research feature {id}"));
        registry.push((id, value));
    }

    // Every instrument construct must be registered exactly once.
    for instrument in data_instruments::ALL {
        assert!(
            data_features::instrument_defaults(instrument.id).is_some(),
            "instrument {} has no feature defaults",
            instrument.id
        );
        for &construct in instrument.constructs {
            let id = feature_id(instrument.id, construct);
            assert!(
                registry.iter().any(|(existing, _)| existing == &id),
                "instrument construct {}.{construct} has no feature",
                instrument.id
            );
        }
    }

    registry
}

static REGISTRY: OnceLock<Vec<(String, Value)>> = OnceLock::new();

fn registry() -> &'static [(String, Value)] {
    REGISTRY.get_or_init(build_registry)
}

/// Re-check every registered feature's invariants.
///
/// # Panics
///
/// Panics (via the internal invariant checker) if the authored register is
/// broken: a broken register is a code bug, not recoverable external input.
#[must_use]
pub fn validate_registry() -> bool {
    for (id, feature) in registry() {
        assert_feature_invariants(feature, &format!("research feature {id}"));
    }
    true
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "research feature invariant violated".to_string()
    }
}

/// Validate an externally supplied feature against [`shape`] and this
/// register's cross-field invariants.
pub fn validate_feature(feature: &Value, label: Option<&str>) -> Result<()> {
    research_schema::validate(&shape(), feature)?;
    let entries = feature
        .as_record()
        .expect("validate already confirmed a record");
    let owned_label = label
        .map(str::to_string)
        .unwrap_or_else(|| format!("research feature {}", text_field(entries, "id")));
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_feature_invariants(feature, &owned_label);
    }));
    std::panic::set_hook(previous_hook);
    outcome.map_err(|payload| panic_payload_message(payload.as_ref()))
}

/// Every registered feature id, sorted.
#[must_use]
pub fn ids() -> Vec<String> {
    let mut result: Vec<String> = registry().iter().map(|(id, _)| id.clone()).collect();
    result.sort();
    result
}

/// Look up one registered feature by id. Returns a clone so callers cannot
/// edit the register.
pub fn feature(id: &str) -> Result<Value> {
    registry()
        .iter()
        .find(|(existing, _)| existing == id)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| format!("unknown research feature {id}"))
}

/// Content hash of the whole register at a set of versions. Dataset
/// manifests store this so a silent register edit invalidates lineage.
#[must_use]
pub fn registry_hash() -> String {
    let mut parts = Vec::new();
    for id in ids() {
        let feature_value = feature(&id).expect("id came from the register");
        let entries = feature_value
            .as_record()
            .expect("registered feature is a record");
        let version = Value::record_get(entries, "version")
            .and_then(Value::as_number)
            .expect("version");
        let hash = research_schema::content_hash(&shape(), &feature_value)
            .expect("registered feature validates");
        parts.push(TuplePart::Text(id));
        parts.push(TuplePart::Number(version));
        parts.push(TuplePart::Text(hash));
    }
    research_schema::tuple_hash(REGISTRY_LABEL, &parts)
}

/// Guard against pseudo-replication and prohibited use at the point of use.
pub fn aggregation_allowed(id: &str, level: &str) -> Result<()> {
    let value = feature(id)?;
    let entries = value.as_record().expect("registered feature is a record");
    let allowed = array_field(entries, "aggregation_levels")
        .iter()
        .any(|v| v.as_str() == Some(level));
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "research feature {id} is not defined at aggregation level {level}"
        ))
    }
}

/// Guard against a prohibited use at the point of use.
pub fn use_allowed(id: &str, use_name: &str) -> Result<()> {
    let value = feature(id)?;
    let entries = value.as_record().expect("registered feature is a record");
    let prohibited = array_field(entries, "prohibited_uses")
        .iter()
        .any(|v| v.as_str() == Some(use_name));
    if prohibited {
        Err(format!("research feature {id} prohibits use {use_name}"))
    } else {
        Ok(())
    }
}
