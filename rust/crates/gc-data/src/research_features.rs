//! Derived feature register source data.
//!
//! Instrument-backed features are expanded from `research_instruments` by
//! `sim::research_features`, so a construct can never exist as an instrument
//! construct without a registered feature (or vice versa). Only the metadata
//! that actually differs per instrument or per construct is authored here;
//! nothing is defaulted silently by the expander.
//!
//! `extraction_commit` deliberately does not live here. The register declares
//! *which module and config* own an extraction; the commit that actually ran it
//! is recorded per feature version in the dataset manifest, where it is a fact
//! rather than a promise.

/// The grain at which a research feature is computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchFeatureGrain {
    /// One simulation tick.
    Tick,
    /// One decision point.
    Decision,
    /// One possession.
    Possession,
    /// One encounter.
    Encounter,
    /// One match.
    Match,
    /// One player within one match.
    PlayerMatch,
    /// One condition block.
    ConditionBlock,
    /// One session.
    Session,
    /// One participant.
    Participant,
}

/// How a feature could leak information it should not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchFeatureLeakage {
    /// No leakage risk.
    None,
    /// Derived from the outcome it is meant to help explain.
    OutcomeDerived,
    /// Derived from privileged, non-observable state.
    PrivilegedState,
    /// Derived from information not yet available at the causal window.
    FutureInformation,
    /// Adjacent enough to the label to bias analysis.
    LabelAdjacent,
}

/// Who or what may observe a feature's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchFeatureObservability {
    /// Observable by the player during play.
    PlayerObservable,
    /// Only visible via privileged diagnostics.
    PrivilegedDiagnostic,
    /// Derived from the match outcome.
    OutcomeDerived,
    /// Protected or sensitive; access-restricted.
    ProtectedSensitive,
    /// Never extracted, by design.
    Prohibited,
}

/// Evidentiary weight class for a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchEvidenceTier {
    /// Directly reported human experience.
    HumanExperience,
    /// A behavioral proxy for human experience.
    BehavioralProxy,
    /// A soccer-shape proxy over simulated match statistics.
    SoccerShapeProxy,
    /// A machine diagnostic with no human-experience claim.
    MachineDiagnostic,
}

/// The analytic role a feature plays in an evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchOutcomeRole {
    /// The primary outcome of an evaluation.
    PrimaryOutcome,
    /// A secondary outcome of an evaluation.
    SecondaryOutcome,
    /// A guardrail metric that must not regress.
    Guardrail,
    /// A diagnostic metric.
    Diagnostic,
    /// An exploratory metric.
    Exploratory,
    /// A feature that feeds another feature only; never analyzed on its own.
    FeatureOnly,
}

/// How a missing value is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchMissingValueBehavior {
    /// Row is retained with the value recorded as not-applicable.
    NaRow,
    /// Row is dropped entirely.
    DropRow,
    /// Value is defined as zero when otherwise absent.
    ZeroWhenDefined,
    /// Absence indicates a broken protocol; the run is invalid.
    ProtocolFailure,
}

/// Guard against treating clustered rows as independent samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchPseudoReplicationGuard {
    /// Each participant is an independent analysis unit.
    IndependentUnitParticipant,
    /// Rows must be clustered by participant.
    ClusterByParticipant,
    /// Rows must be clustered by match.
    ClusterByMatch,
    /// Not a valid unit of analysis at all.
    NotAnAnalysisUnit,
}

/// The kind of temporal window a feature's causal claim is scoped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchCausalWindowKind {
    /// Scoped to the same tick.
    SameTick,
    /// Scoped to a fixed number of ticks forward.
    ForwardTicks,
    /// Scoped to a fixed number of ticks backward.
    BackwardTicks,
    /// Scoped to the whole match.
    WholeMatch,
    /// Scoped to after a condition ends.
    PostCondition,
}

/// A causal window: a kind, plus an optional tick count for the windowed kinds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchCausalWindowData {
    /// Window kind.
    pub kind: ResearchCausalWindowKind,
    /// Tick count, for `ForwardTicks`/`BackwardTicks`.
    pub ticks: Option<i64>,
}

/// A single derived research feature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchFeatureData {
    /// Persistent identity.
    pub id: &'static str,
    /// Feature version.
    pub version: i64,
    /// Human-readable description.
    pub description: &'static str,
    /// Grain at which the feature is computed.
    pub grain: ResearchFeatureGrain,
    /// Schemas the feature is extracted from.
    pub source_schemas: &'static [&'static str],
    /// Fields the feature is extracted from.
    pub source_fields: &'static [&'static str],
    /// Module that owns the extraction.
    pub extraction_module: &'static str,
    /// Config id the extraction module uses.
    pub extraction_config_id: &'static str,
    /// Description of the numerator.
    pub numerator: &'static str,
    /// Description of the denominator, if the feature is a ratio.
    pub denominator: Option<&'static str>,
    /// Unit the feature is expressed in.
    pub unit: &'static str,
    /// Rows or matches excluded from the feature.
    pub exclusions: &'static [&'static str],
    /// How a missing value is handled.
    pub missing_value_behavior: ResearchMissingValueBehavior,
    /// Causal window the feature is scoped to.
    pub causal_window: ResearchCausalWindowData,
    /// Normalization applied to the raw value.
    pub normalization: &'static str,
    /// Leakage risk.
    pub leakage_risk: ResearchFeatureLeakage,
    /// Observability.
    pub observability: ResearchFeatureObservability,
    /// Evidentiary weight class.
    pub evidence_tier: ResearchEvidenceTier,
    /// Analytic role this feature plays.
    pub outcome_role: ResearchOutcomeRole,
    /// Grains this feature can be aggregated to.
    pub aggregation_levels: &'static [ResearchFeatureGrain],
    /// Guard against pseudo-replication.
    pub pseudo_replication_guard: ResearchPseudoReplicationGuard,
    /// Known confounds.
    pub confounds: &'static [&'static str],
    /// How optimizing this feature could fail Goodhart's law.
    pub goodhart_failure: &'static str,
    /// Uses this feature is prohibited from.
    pub prohibited_uses: &'static [&'static str],
    /// Whether this feature may be cited as evidence of human fun.
    pub human_fun_claim: bool,
}

/// Defaults shared by every feature an instrument backs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchInstrumentFeatureDefaults {
    /// Grain at which features are computed.
    pub grain: ResearchFeatureGrain,
    /// Unit the features are expressed in.
    pub unit: &'static str,
    /// Module that owns the extraction.
    pub extraction_module: &'static str,
    /// Config id the extraction module uses.
    pub extraction_config_id: &'static str,
    /// Schemas the features are extracted from.
    pub source_schemas: &'static [&'static str],
    /// Rows or matches excluded from the features.
    pub exclusions: &'static [&'static str],
    /// How a missing value is handled.
    pub missing_value_behavior: ResearchMissingValueBehavior,
    /// Causal window the features are scoped to.
    pub causal_window: ResearchCausalWindowData,
    /// Normalization applied to raw values.
    pub normalization: &'static str,
    /// Leakage risk.
    pub leakage_risk: ResearchFeatureLeakage,
    /// Observability.
    pub observability: ResearchFeatureObservability,
    /// Evidentiary weight class.
    pub evidence_tier: ResearchEvidenceTier,
    /// Grains the features can be aggregated to.
    pub aggregation_levels: &'static [ResearchFeatureGrain],
    /// Guard against pseudo-replication.
    pub pseudo_replication_guard: ResearchPseudoReplicationGuard,
    /// Uses these features are prohibited from.
    pub prohibited_uses: &'static [&'static str],
}

/// Analysis metadata for one construct, shared across the instruments that measure it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchConstructNoteData {
    /// Analytic role this construct plays.
    pub outcome_role: ResearchOutcomeRole,
    /// Known confounds.
    pub confounds: &'static [&'static str],
    /// How optimizing this construct could fail Goodhart's law.
    pub goodhart_failure: &'static str,
}

/// The derived feature register.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchFeatureSourceData {
    /// Register version.
    pub version: i64,
    /// Per-instrument feature defaults, keyed by instrument id.
    pub instrument_defaults: &'static [(&'static str, ResearchInstrumentFeatureDefaults)],
    /// Per-construct analysis notes, keyed by construct id.
    pub construct_notes: &'static [(&'static str, ResearchConstructNoteData)],
    /// Behavioral (non-instrument-backed) features.
    pub behavioral: &'static [ResearchFeatureData],
}

const POST_CONDITION: ResearchCausalWindowData = ResearchCausalWindowData {
    kind: ResearchCausalWindowKind::PostCondition,
    ticks: None,
};

/// The derived feature register.
pub const SOURCE: ResearchFeatureSourceData = ResearchFeatureSourceData {
    version: 1,
    instrument_defaults: &[
        (
            "pxi_enjoyment_addon",
            ResearchInstrumentFeatureDefaults {
                grain: ResearchFeatureGrain::ConditionBlock,
                unit: "pxi_points_minus3_to_plus3",
                extraction_module: "sim.research_response",
                extraction_config_id: "pxi-enjoyment-mean-1",
                source_schemas: &["research_response_set/v1", "research_session_envelope/v1"],
                exclusions: &["practice_block", "excluded_session", "withdrawn_session"],
                missing_value_behavior: ResearchMissingValueBehavior::NaRow,
                causal_window: POST_CONDITION,
                normalization: "none",
                leakage_risk: ResearchFeatureLeakage::None,
                observability: ResearchFeatureObservability::PlayerObservable,
                evidence_tier: ResearchEvidenceTier::HumanExperience,
                aggregation_levels: &[
                    ResearchFeatureGrain::ConditionBlock,
                    ResearchFeatureGrain::Session,
                    ResearchFeatureGrain::Participant,
                ],
                pseudo_replication_guard:
                    ResearchPseudoReplicationGuard::IndependentUnitParticipant,
                prohibited_uses: &["tick_row_analysis", "per_frame_weighting"],
            },
        ),
        (
            "pxi_partial_mechanisms",
            ResearchInstrumentFeatureDefaults {
                grain: ResearchFeatureGrain::ConditionBlock,
                unit: "pxi_points_minus3_to_plus3",
                extraction_module: "sim.research_response",
                extraction_config_id: "pxi-subscale-mean-1",
                source_schemas: &["research_response_set/v1", "research_session_envelope/v1"],
                exclusions: &["practice_block", "excluded_session", "withdrawn_session"],
                missing_value_behavior: ResearchMissingValueBehavior::NaRow,
                causal_window: POST_CONDITION,
                normalization: "none",
                leakage_risk: ResearchFeatureLeakage::None,
                observability: ResearchFeatureObservability::PlayerObservable,
                evidence_tier: ResearchEvidenceTier::HumanExperience,
                aggregation_levels: &[
                    ResearchFeatureGrain::ConditionBlock,
                    ResearchFeatureGrain::Session,
                    ResearchFeatureGrain::Participant,
                ],
                pseudo_replication_guard:
                    ResearchPseudoReplicationGuard::IndependentUnitParticipant,
                prohibited_uses: &[
                    "full_pxi_administration_claim",
                    "benchmark_comparison",
                    "tick_row_analysis",
                ],
            },
        ),
        (
            "bangs_session",
            ResearchInstrumentFeatureDefaults {
                grain: ResearchFeatureGrain::ConditionBlock,
                unit: "bangs_points_1_to_7",
                extraction_module: "sim.research_response",
                extraction_config_id: "bangs-subscale-mean-1",
                source_schemas: &["research_response_set/v1", "research_session_envelope/v1"],
                exclusions: &["practice_block", "excluded_session", "withdrawn_session"],
                missing_value_behavior: ResearchMissingValueBehavior::NaRow,
                causal_window: POST_CONDITION,
                normalization: "none",
                leakage_risk: ResearchFeatureLeakage::None,
                observability: ResearchFeatureObservability::PlayerObservable,
                evidence_tier: ResearchEvidenceTier::HumanExperience,
                aggregation_levels: &[
                    ResearchFeatureGrain::ConditionBlock,
                    ResearchFeatureGrain::Session,
                    ResearchFeatureGrain::Participant,
                ],
                pseudo_replication_guard:
                    ResearchPseudoReplicationGuard::IndependentUnitParticipant,
                prohibited_uses: &[
                    "confirmatory_gate",
                    "collapsing_frustration_into_satisfaction",
                    "tick_row_analysis",
                ],
            },
        ),
        (
            "affective_slider",
            ResearchInstrumentFeatureDefaults {
                grain: ResearchFeatureGrain::Session,
                unit: "normalized_0_to_1",
                extraction_module: "sim.research_response",
                extraction_config_id: "affective-raw-1",
                source_schemas: &["research_response_set/v1", "research_session_envelope/v1"],
                exclusions: &["practice_block", "excluded_session", "withdrawn_session"],
                missing_value_behavior: ResearchMissingValueBehavior::NaRow,
                causal_window: POST_CONDITION,
                normalization: "none",
                leakage_risk: ResearchFeatureLeakage::None,
                observability: ResearchFeatureObservability::PlayerObservable,
                evidence_tier: ResearchEvidenceTier::HumanExperience,
                aggregation_levels: &[
                    ResearchFeatureGrain::Session,
                    ResearchFeatureGrain::Participant,
                ],
                pseudo_replication_guard:
                    ResearchPseudoReplicationGuard::IndependentUnitParticipant,
                prohibited_uses: &[
                    "pooling_with_custom_affect_fallback",
                    "emotion_inference",
                    "tick_row_analysis",
                ],
            },
        ),
        (
            "custom_affect_fallback",
            ResearchInstrumentFeatureDefaults {
                grain: ResearchFeatureGrain::Session,
                unit: "likert_points_1_to_7",
                extraction_module: "sim.research_response",
                extraction_config_id: "goliseo-affect-fallback-1",
                source_schemas: &["research_response_set/v1", "research_session_envelope/v1"],
                exclusions: &["practice_block", "excluded_session", "withdrawn_session"],
                missing_value_behavior: ResearchMissingValueBehavior::NaRow,
                causal_window: POST_CONDITION,
                normalization: "none",
                leakage_risk: ResearchFeatureLeakage::None,
                observability: ResearchFeatureObservability::PlayerObservable,
                evidence_tier: ResearchEvidenceTier::HumanExperience,
                aggregation_levels: &[
                    ResearchFeatureGrain::Session,
                    ResearchFeatureGrain::Participant,
                ],
                pseudo_replication_guard:
                    ResearchPseudoReplicationGuard::IndependentUnitParticipant,
                prohibited_uses: &[
                    "pooling_with_affective_slider",
                    "validated_instrument_claim",
                    "emotion_inference",
                ],
            },
        ),
        (
            "custom_diagnostics",
            ResearchInstrumentFeatureDefaults {
                grain: ResearchFeatureGrain::ConditionBlock,
                unit: "likert_points_minus3_to_plus3",
                extraction_module: "sim.research_response",
                extraction_config_id: "goliseo-custom-items-1",
                source_schemas: &["research_response_set/v1", "research_session_envelope/v1"],
                exclusions: &["practice_block", "excluded_session", "withdrawn_session"],
                missing_value_behavior: ResearchMissingValueBehavior::NaRow,
                causal_window: POST_CONDITION,
                normalization: "none",
                leakage_risk: ResearchFeatureLeakage::None,
                observability: ResearchFeatureObservability::PlayerObservable,
                evidence_tier: ResearchEvidenceTier::HumanExperience,
                aggregation_levels: &[
                    ResearchFeatureGrain::ConditionBlock,
                    ResearchFeatureGrain::Session,
                    ResearchFeatureGrain::Participant,
                ],
                pseudo_replication_guard:
                    ResearchPseudoReplicationGuard::IndependentUnitParticipant,
                prohibited_uses: &[
                    "validated_scale_claim",
                    "scale_score_aggregation",
                    "confirmatory_gate",
                ],
            },
        ),
    ],
    construct_notes: &[
        (
            "enjoyment",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::PrimaryOutcome,
                confounds: &["novelty", "period_and_order", "win_or_loss", "fatigue"],
                goodhart_failure: "tuning for post-condition self-report while play quality falls",
            },
        ),
        (
            "autonomy",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Diagnostic,
                confounds: &["option_visibility", "role_assignment"],
                goodhart_failure: "adding options that raise perceived choice without agency",
            },
        ),
        (
            "mastery",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Diagnostic,
                confounds: &["learning_curve_position", "opponent_policy_strength"],
                goodhart_failure: "weakening opponents to manufacture felt mastery",
            },
        ),
        (
            "challenge",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Diagnostic,
                confounds: &["opponent_policy_strength", "experience_source_measures"],
                goodhart_failure: "treating mid-scale challenge as a target regardless of enjoyment",
            },
        ),
        (
            "ease_of_control",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["control_device", "input_latency", "control_mapping"],
                goodhart_failure: "removing depth to raise ease scores",
            },
        ),
        (
            "goals_and_rules",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["tutorial_exposure", "instruction_wording"],
                goodhart_failure: "over-explaining in instructions rather than in the game",
            },
        ),
        (
            "progress_feedback",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Diagnostic,
                confounds: &["score_state", "presentation_volume"],
                goodhart_failure: "adding feedback spectacle without informative content",
            },
        ),
        (
            "autonomy_satisfaction",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["session_length", "condition_order"],
                goodhart_failure: "reading exploratory subscale movement as a confirmatory win",
            },
        ),
        (
            "autonomy_frustration",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["session_length", "condition_order"],
                goodhart_failure: "collapsing frustration into a satisfaction composite",
            },
        ),
        (
            "competence_satisfaction",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["win_or_loss", "opponent_policy_strength"],
                goodhart_failure: "inflating win rate to raise competence scores",
            },
        ),
        (
            "competence_frustration",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["win_or_loss", "control_device"],
                goodhart_failure: "hiding failure feedback instead of making failure legible",
            },
        ),
        (
            "relatedness_satisfaction",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["observer_mode", "facilitator_presence"],
                goodhart_failure: "reading facilitator rapport as game-driven relatedness",
            },
        ),
        (
            "relatedness_frustration",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["observer_mode", "opponent_population"],
                goodhart_failure: "removing social framing rather than fixing the cause",
            },
        ),
        (
            "valence",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Diagnostic,
                confounds: &["break_timing", "instrument_modality"],
                goodhart_failure: "optimizing momentary affect at a natural break",
            },
        ),
        (
            "arousal",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Diagnostic,
                confounds: &["break_timing", "spectacle_volume"],
                goodhart_failure: "treating arousal as enjoyment",
            },
        ),
        (
            "soccer_primacy",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["participant_genre_preference"],
                goodhart_failure: "suppressing combat entirely to protect the item",
            },
        ),
        (
            "fairness",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["win_or_loss", "opponent_policy"],
                goodhart_failure: "removing counterplay variance to raise fairness ratings",
            },
        ),
        (
            "suspense",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["score_closeness", "match_length"],
                goodhart_failure: "engineering close scores to manufacture suspense",
            },
        ),
        (
            "counterplay_readability",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["cue_volume", "display_size", "readability_settings"],
                goodhart_failure: "adding cue noise that raises perceived readability",
            },
        ),
        (
            "overload",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["render_performance", "session_position"],
                goodhart_failure: "cutting simultaneous options to lower overload",
            },
        ),
        (
            "frustration",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Guardrail,
                confounds: &["win_or_loss", "disable_duration"],
                goodhart_failure: "removing all punishment to lower frustration",
            },
        ),
        (
            "desire_to_explore",
            ResearchConstructNoteData {
                outcome_role: ResearchOutcomeRole::Exploratory,
                confounds: &["novelty", "unrewarded_choice_framing"],
                goodhart_failure: "treating rematch clicks as intrinsic curiosity",
            },
        ),
    ],
    behavioral: &[
        ResearchFeatureData {
            id: "soccer_shape_proxy_score",
            version: 1,
            description: "Geometric mean of banded MatchMetrics desirabilities. A soccer-shape proxy over simulated match statistics; it is not a measurement of human fun and carries no participant evidence.",
            grain: ResearchFeatureGrain::Match,
            source_schemas: &["gameplay_trace_manifest/v1"],
            source_fields: &["sim.metrics.MatchMetrics", "sim.metrics.bands"],
            extraction_module: "sim.metrics",
            extraction_config_id: "metrics-fun-score-bands-1",
            numerator: "product of per-band desirabilities present in the match",
            denominator: Some("count of banded metrics present, as a geometric-mean exponent"),
            unit: "index_0_to_1",
            exclusions: &["matches with no banded metric present"],
            missing_value_behavior: ResearchMissingValueBehavior::NaRow,
            causal_window: ResearchCausalWindowData {
                kind: ResearchCausalWindowKind::WholeMatch,
                ticks: None,
            },
            normalization: "geometric_mean_over_present_bands",
            leakage_risk: ResearchFeatureLeakage::OutcomeDerived,
            observability: ResearchFeatureObservability::PrivilegedDiagnostic,
            evidence_tier: ResearchEvidenceTier::SoccerShapeProxy,
            outcome_role: ResearchOutcomeRole::Diagnostic,
            aggregation_levels: &[ResearchFeatureGrain::Match],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::ClusterByMatch,
            confounds: &[
                "band midpoints are authored targets rather than measured preferences",
                "bot policy strength drives most banded metrics",
            ],
            goodhart_failure: "tuning search maximizes band membership while human enjoyment falls",
            prohibited_uses: &[
                "human_fun_claim",
                "primary_outcome",
                "proceed_gate",
                "participant_facing_report",
            ],
            human_fun_claim: false,
        },
        ResearchFeatureData {
            id: "involuntary_disable_share",
            version: 1,
            description: "Share of eligible active-play ticks in which a player was forcibly unable to act.",
            grain: ResearchFeatureGrain::PlayerMatch,
            source_schemas: &["gameplay_trace_manifest/v1", "research_event_stream/v1"],
            source_fields: &["CombatPlayerState.forced_ticks", "MatchState.ball_in_play"],
            extraction_module: "sim.research_features",
            extraction_config_id: "combat-control-safety-1",
            numerator: "ticks with forced_ticks > 0",
            denominator: Some("eligible active-play ticks"),
            unit: "share_0_to_1",
            exclusions: &["non-active-play ticks", "keeper slots", "practice_block"],
            missing_value_behavior: ResearchMissingValueBehavior::NaRow,
            causal_window: ResearchCausalWindowData {
                kind: ResearchCausalWindowKind::WholeMatch,
                ticks: None,
            },
            normalization: "share_of_denominator",
            leakage_risk: ResearchFeatureLeakage::None,
            observability: ResearchFeatureObservability::PlayerObservable,
            evidence_tier: ResearchEvidenceTier::BehavioralProxy,
            outcome_role: ResearchOutcomeRole::Guardrail,
            aggregation_levels: &[
                ResearchFeatureGrain::PlayerMatch,
                ResearchFeatureGrain::Match,
                ResearchFeatureGrain::ConditionBlock,
            ],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::ClusterByMatch,
            confounds: &[
                "voluntary commitment mislabeled as disable",
                "disconnect or process exit truncating the denominator",
            ],
            goodhart_failure: "shortening matches to lower the disable count",
            prohibited_uses: &["human_fun_claim", "participant_capability_inference"],
            human_fun_claim: false,
        },
        ResearchFeatureData {
            id: "combat_to_soccer_conversion_rate",
            version: 1,
            description: "Share of confirmed accepted combat encounters that reach a soccer consequence inside the attribution window.",
            grain: ResearchFeatureGrain::Encounter,
            source_schemas: &["research_event_stream/v1"],
            source_fields: &[
                "ResearchEventRow.domain",
                "ResearchEventRow.event_kind",
                "ResearchEventRow.source_sequence",
            ],
            extraction_module: "sim.research_features",
            extraction_config_id: "combat-funnel-attribution-1",
            numerator: "confirmed encounters with an attributed soccer consequence",
            denominator: Some("confirmed accepted encounters"),
            unit: "share_0_to_1",
            exclusions: &[
                "rejected equipment requests",
                "revoked speculative encounters",
                "encounters whose window crosses full time",
            ],
            missing_value_behavior: ResearchMissingValueBehavior::NaRow,
            causal_window: ResearchCausalWindowData {
                kind: ResearchCausalWindowKind::ForwardTicks,
                ticks: Some(120),
            },
            normalization: "share_of_denominator",
            leakage_risk: ResearchFeatureLeakage::LabelAdjacent,
            observability: ResearchFeatureObservability::PlayerObservable,
            evidence_tier: ResearchEvidenceTier::BehavioralProxy,
            outcome_role: ResearchOutcomeRole::Diagnostic,
            aggregation_levels: &[
                ResearchFeatureGrain::Encounter,
                ResearchFeatureGrain::Match,
                ResearchFeatureGrain::ConditionBlock,
            ],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::ClusterByMatch,
            confounds: &[
                "attribution window length dominates the rate",
                "possession context differs by side and policy",
            ],
            goodhart_failure: "widening the attribution window until every encounter looks causal",
            prohibited_uses: &["human_fun_claim", "speculative_event_inclusion"],
            human_fun_claim: false,
        },
        ResearchFeatureData {
            id: "final_score_margin",
            version: 1,
            description: "Signed home-minus-away goal margin at full time. Outcome-derived: it is a label, not an input.",
            grain: ResearchFeatureGrain::Match,
            source_schemas: &["gameplay_trace_manifest/v1"],
            source_fields: &["sim.metrics.MatchMetrics.margin"],
            extraction_module: "sim.metrics",
            extraction_config_id: "metrics-final-margin-1",
            numerator: "home goals minus away goals at full time",
            denominator: None,
            unit: "goals",
            exclusions: &["incomplete matches", "practice_block"],
            missing_value_behavior: ResearchMissingValueBehavior::DropRow,
            causal_window: ResearchCausalWindowData {
                kind: ResearchCausalWindowKind::WholeMatch,
                ticks: None,
            },
            normalization: "none",
            leakage_risk: ResearchFeatureLeakage::OutcomeDerived,
            observability: ResearchFeatureObservability::OutcomeDerived,
            evidence_tier: ResearchEvidenceTier::BehavioralProxy,
            outcome_role: ResearchOutcomeRole::Diagnostic,
            aggregation_levels: &[
                ResearchFeatureGrain::Match,
                ResearchFeatureGrain::ConditionBlock,
            ],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::ClusterByMatch,
            confounds: &["side assignment", "opponent policy strength"],
            goodhart_failure: "treating a wider margin as a better match experience",
            prohibited_uses: &["human_fun_claim", "model_input"],
            human_fun_claim: false,
        },
        ResearchFeatureData {
            id: "declared_readability_settings",
            version: 1,
            description: "Readability settings the participant chose, recorded because they change what could be seen on screen.",
            grain: ResearchFeatureGrain::Session,
            source_schemas: &["research_session_envelope/v1"],
            source_fields: &["research_session_envelope.environment.readability_settings"],
            extraction_module: "sim.research_session",
            extraction_config_id: "session-readability-1",
            numerator: "the recorded readability setting map for the session",
            denominator: None,
            unit: "setting_map",
            exclusions: &["sessions with no recorded settings"],
            missing_value_behavior: ResearchMissingValueBehavior::NaRow,
            causal_window: POST_CONDITION,
            normalization: "none",
            leakage_risk: ResearchFeatureLeakage::None,
            observability: ResearchFeatureObservability::ProtectedSensitive,
            evidence_tier: ResearchEvidenceTier::MachineDiagnostic,
            outcome_role: ResearchOutcomeRole::FeatureOnly,
            aggregation_levels: &[ResearchFeatureGrain::Session],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::ClusterByParticipant,
            confounds: &["defaults left untouched are not a preference"],
            goodhart_failure: "reading a settings choice as a statement about the participant",
            prohibited_uses: &[
                "human_fun_claim",
                "trait_inference",
                "disability_inference",
                "model_input",
                "per_participant_report",
            ],
            human_fun_claim: false,
        },
        ResearchFeatureData {
            id: "inferred_participant_skill_trait",
            version: 1,
            description: "Placeholder for an inferred per-participant skill or trait score. Registered so that referencing it fails the use gate instead of quietly appearing in a pipeline.",
            grain: ResearchFeatureGrain::Participant,
            source_schemas: &["research_session_envelope/v1", "gameplay_trace_manifest/v1"],
            source_fields: &["any combination of behavioral and session fields"],
            extraction_module: "none",
            extraction_config_id: "prohibited-no-extraction",
            numerator: "not computed; this feature has no implementation by design",
            denominator: None,
            unit: "prohibited",
            exclusions: &["every row: this feature is never extracted"],
            missing_value_behavior: ResearchMissingValueBehavior::ProtocolFailure,
            causal_window: ResearchCausalWindowData {
                kind: ResearchCausalWindowKind::WholeMatch,
                ticks: None,
            },
            normalization: "none",
            leakage_risk: ResearchFeatureLeakage::LabelAdjacent,
            observability: ResearchFeatureObservability::Prohibited,
            evidence_tier: ResearchEvidenceTier::MachineDiagnostic,
            outcome_role: ResearchOutcomeRole::FeatureOnly,
            aggregation_levels: &[ResearchFeatureGrain::Participant],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::NotAnAnalysisUnit,
            confounds: &["skill is confounded with device, role, and opponent policy"],
            goodhart_failure: "ranking testers by an invented skill number",
            prohibited_uses: &[
                "any_analysis",
                "human_fun_claim",
                "model_input",
                "trait_inference",
                "per_participant_report",
            ],
            human_fun_claim: false,
        },
        ResearchFeatureData {
            id: "settled_turnovers_per_active_minute",
            version: 1,
            description: "Settled possession changes per active minute, as a soccer-integrity guardrail.",
            grain: ResearchFeatureGrain::Match,
            source_schemas: &["gameplay_trace_manifest/v1"],
            source_fields: &["sim.metrics.MatchMetrics.turnovers_per_min"],
            extraction_module: "sim.metrics",
            extraction_config_id: "metrics-settled-turnovers-1",
            numerator: "settled team possession changes",
            denominator: Some("active-play minutes"),
            unit: "count_per_minute",
            exclusions: &["ownership flicker below the settle hold", "practice_block"],
            missing_value_behavior: ResearchMissingValueBehavior::ProtocolFailure,
            causal_window: ResearchCausalWindowData {
                kind: ResearchCausalWindowKind::WholeMatch,
                ticks: None,
            },
            normalization: "per_active_minute",
            leakage_risk: ResearchFeatureLeakage::None,
            observability: ResearchFeatureObservability::PlayerObservable,
            evidence_tier: ResearchEvidenceTier::BehavioralProxy,
            outcome_role: ResearchOutcomeRole::Guardrail,
            aggregation_levels: &[
                ResearchFeatureGrain::Match,
                ResearchFeatureGrain::ConditionBlock,
            ],
            pseudo_replication_guard: ResearchPseudoReplicationGuard::ClusterByMatch,
            confounds: &["settle-hold threshold choice", "side and skill asymmetry"],
            goodhart_failure: "raising raw ownership churn and calling it dynamism",
            prohibited_uses: &["human_fun_claim"],
            human_fun_claim: false,
        },
    ],
};

/// Look up per-instrument feature defaults by instrument id.
pub fn instrument_defaults(id: &str) -> Option<&'static ResearchInstrumentFeatureDefaults> {
    SOURCE
        .instrument_defaults
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, value)| value)
}

/// Look up per-construct analysis notes by construct id.
pub fn construct_note(id: &str) -> Option<&'static ResearchConstructNoteData> {
    SOURCE
        .construct_notes
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, value)| value)
}

/// Look up a behavioral feature by id.
pub fn behavioral(id: &str) -> Option<&'static ResearchFeatureData> {
    SOURCE.behavioral.iter().find(|feature| feature.id == id)
}
