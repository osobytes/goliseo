-- Derived feature register source data.
--
-- Instrument-backed features are expanded from `data/research_instruments.lua`
-- by `sim/research_features.lua`, so a construct can never exist as an
-- instrument construct without a registered feature (or vice versa). Only the
-- metadata that actually differs per instrument or per construct is authored
-- here; nothing is defaulted silently by the expander.
--
-- `extraction_commit` deliberately does not live here. The register declares
-- *which module and config* own an extraction; the commit that actually ran it
-- is recorded per feature version in the dataset manifest, where it is a fact
-- rather than a promise.

---@alias ResearchFeatureGrain
---| "tick"
---| "decision"
---| "possession"
---| "encounter"
---| "match"
---| "player_match"
---| "condition_block"
---| "session"
---| "participant"

---@alias ResearchFeatureLeakage
---| "none"
---| "outcome_derived"
---| "privileged_state"
---| "future_information"
---| "label_adjacent"

---@alias ResearchFeatureObservability
---| "player_observable"
---| "privileged_diagnostic"
---| "outcome_derived"
---| "protected_sensitive"
---| "prohibited"

---@alias ResearchEvidenceTier
---| "human_experience"
---| "behavioral_proxy"
---| "soccer_shape_proxy"
---| "machine_diagnostic"

---@alias ResearchOutcomeRole
---| "primary_outcome"
---| "secondary_outcome"
---| "guardrail"
---| "diagnostic"
---| "exploratory"
---| "feature_only"

---@alias ResearchMissingValueBehavior
---| "na_row"
---| "drop_row"
---| "zero_when_defined"
---| "protocol_failure"

---@alias ResearchPseudoReplicationGuard
---| "independent_unit_participant"
---| "cluster_by_participant"
---| "cluster_by_match"
---| "not_an_analysis_unit"

---@class ResearchCausalWindowData
---@field kind "same_tick"|"forward_ticks"|"backward_ticks"|"whole_match"|"post_condition"
---@field ticks integer?

---@class ResearchFeatureData
---@field id string
---@field version integer
---@field description string
---@field grain ResearchFeatureGrain
---@field source_schemas string[]
---@field source_fields string[]
---@field extraction_module string
---@field extraction_config_id string
---@field numerator string
---@field denominator string?
---@field unit string
---@field exclusions string[]
---@field missing_value_behavior ResearchMissingValueBehavior
---@field causal_window ResearchCausalWindowData
---@field normalization string
---@field leakage_risk ResearchFeatureLeakage
---@field observability ResearchFeatureObservability
---@field evidence_tier ResearchEvidenceTier
---@field outcome_role ResearchOutcomeRole
---@field aggregation_levels ResearchFeatureGrain[]
---@field pseudo_replication_guard ResearchPseudoReplicationGuard
---@field confounds string[]
---@field goodhart_failure string
---@field prohibited_uses string[]
---@field human_fun_claim boolean

---@class ResearchInstrumentFeatureDefaults
---@field grain ResearchFeatureGrain
---@field unit string
---@field extraction_module string
---@field extraction_config_id string
---@field source_schemas string[]
---@field exclusions string[]
---@field missing_value_behavior ResearchMissingValueBehavior
---@field causal_window ResearchCausalWindowData
---@field normalization string
---@field leakage_risk ResearchFeatureLeakage
---@field observability ResearchFeatureObservability
---@field evidence_tier ResearchEvidenceTier
---@field aggregation_levels ResearchFeatureGrain[]
---@field pseudo_replication_guard ResearchPseudoReplicationGuard
---@field prohibited_uses string[]

---@class ResearchConstructNoteData
---@field outcome_role ResearchOutcomeRole
---@field confounds string[]
---@field goodhart_failure string

---@class ResearchFeatureSourceData
---@field version integer
---@field instrument_defaults table<string, ResearchInstrumentFeatureDefaults>
---@field construct_notes table<string, ResearchConstructNoteData>
---@field behavioral ResearchFeatureData[]

---@type ResearchFeatureSourceData
return {
    version = 1,
    instrument_defaults = {
        pxi_enjoyment_addon = {
            grain = "condition_block",
            unit = "pxi_points_minus3_to_plus3",
            extraction_module = "sim.research_response",
            extraction_config_id = "pxi-enjoyment-mean-1",
            source_schemas = { "research_response_set/v1", "research_session_envelope/v1" },
            exclusions = { "practice_block", "excluded_session", "withdrawn_session" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "human_experience",
            aggregation_levels = { "condition_block", "session", "participant" },
            pseudo_replication_guard = "independent_unit_participant",
            prohibited_uses = { "tick_row_analysis", "per_frame_weighting" },
        },
        pxi_partial_mechanisms = {
            grain = "condition_block",
            unit = "pxi_points_minus3_to_plus3",
            extraction_module = "sim.research_response",
            extraction_config_id = "pxi-subscale-mean-1",
            source_schemas = { "research_response_set/v1", "research_session_envelope/v1" },
            exclusions = { "practice_block", "excluded_session", "withdrawn_session" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "human_experience",
            aggregation_levels = { "condition_block", "session", "participant" },
            pseudo_replication_guard = "independent_unit_participant",
            prohibited_uses = {
                "full_pxi_administration_claim",
                "benchmark_comparison",
                "tick_row_analysis",
            },
        },
        bangs_session = {
            grain = "condition_block",
            unit = "bangs_points_1_to_7",
            extraction_module = "sim.research_response",
            extraction_config_id = "bangs-subscale-mean-1",
            source_schemas = { "research_response_set/v1", "research_session_envelope/v1" },
            exclusions = { "practice_block", "excluded_session", "withdrawn_session" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "human_experience",
            aggregation_levels = { "condition_block", "session", "participant" },
            pseudo_replication_guard = "independent_unit_participant",
            prohibited_uses = {
                "confirmatory_gate",
                "collapsing_frustration_into_satisfaction",
                "tick_row_analysis",
            },
        },
        affective_slider = {
            grain = "session",
            unit = "normalized_0_to_1",
            extraction_module = "sim.research_response",
            extraction_config_id = "affective-raw-1",
            source_schemas = { "research_response_set/v1", "research_session_envelope/v1" },
            exclusions = { "practice_block", "excluded_session", "withdrawn_session" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "human_experience",
            aggregation_levels = { "session", "participant" },
            pseudo_replication_guard = "independent_unit_participant",
            prohibited_uses = {
                "pooling_with_custom_affect_fallback",
                "emotion_inference",
                "tick_row_analysis",
            },
        },
        custom_affect_fallback = {
            grain = "session",
            unit = "likert_points_1_to_7",
            extraction_module = "sim.research_response",
            extraction_config_id = "goliseo-affect-fallback-1",
            source_schemas = { "research_response_set/v1", "research_session_envelope/v1" },
            exclusions = { "practice_block", "excluded_session", "withdrawn_session" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "human_experience",
            aggregation_levels = { "session", "participant" },
            pseudo_replication_guard = "independent_unit_participant",
            prohibited_uses = {
                "pooling_with_affective_slider",
                "validated_instrument_claim",
                "emotion_inference",
            },
        },
        custom_diagnostics = {
            grain = "condition_block",
            unit = "likert_points_minus3_to_plus3",
            extraction_module = "sim.research_response",
            extraction_config_id = "goliseo-custom-items-1",
            source_schemas = { "research_response_set/v1", "research_session_envelope/v1" },
            exclusions = { "practice_block", "excluded_session", "withdrawn_session" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "human_experience",
            aggregation_levels = { "condition_block", "session", "participant" },
            pseudo_replication_guard = "independent_unit_participant",
            prohibited_uses = {
                "validated_scale_claim",
                "scale_score_aggregation",
                "confirmatory_gate",
            },
        },
    },
    construct_notes = {
        enjoyment = {
            outcome_role = "primary_outcome",
            confounds = { "novelty", "period_and_order", "win_or_loss", "fatigue" },
            goodhart_failure = "tuning for post-condition self-report while play quality falls",
        },
        autonomy = {
            outcome_role = "diagnostic",
            confounds = { "option_visibility", "role_assignment" },
            goodhart_failure = "adding options that raise perceived choice without agency",
        },
        mastery = {
            outcome_role = "diagnostic",
            confounds = { "learning_curve_position", "opponent_policy_strength" },
            goodhart_failure = "weakening opponents to manufacture felt mastery",
        },
        challenge = {
            outcome_role = "diagnostic",
            confounds = { "opponent_policy_strength", "experience_source_measures" },
            goodhart_failure = "treating mid-scale challenge as a target regardless of enjoyment",
        },
        ease_of_control = {
            outcome_role = "guardrail",
            confounds = { "control_device", "input_latency", "control_mapping" },
            goodhart_failure = "removing depth to raise ease scores",
        },
        goals_and_rules = {
            outcome_role = "guardrail",
            confounds = { "tutorial_exposure", "instruction_wording" },
            goodhart_failure = "over-explaining in instructions rather than in the game",
        },
        progress_feedback = {
            outcome_role = "diagnostic",
            confounds = { "score_state", "presentation_volume" },
            goodhart_failure = "adding feedback spectacle without informative content",
        },
        autonomy_satisfaction = {
            outcome_role = "exploratory",
            confounds = { "session_length", "condition_order" },
            goodhart_failure = "reading exploratory subscale movement as a confirmatory win",
        },
        autonomy_frustration = {
            outcome_role = "exploratory",
            confounds = { "session_length", "condition_order" },
            goodhart_failure = "collapsing frustration into a satisfaction composite",
        },
        competence_satisfaction = {
            outcome_role = "exploratory",
            confounds = { "win_or_loss", "opponent_policy_strength" },
            goodhart_failure = "inflating win rate to raise competence scores",
        },
        competence_frustration = {
            outcome_role = "exploratory",
            confounds = { "win_or_loss", "control_device" },
            goodhart_failure = "hiding failure feedback instead of making failure legible",
        },
        relatedness_satisfaction = {
            outcome_role = "exploratory",
            confounds = { "observer_mode", "facilitator_presence" },
            goodhart_failure = "reading facilitator rapport as game-driven relatedness",
        },
        relatedness_frustration = {
            outcome_role = "exploratory",
            confounds = { "observer_mode", "opponent_population" },
            goodhart_failure = "removing social framing rather than fixing the cause",
        },
        valence = {
            outcome_role = "diagnostic",
            confounds = { "break_timing", "instrument_modality" },
            goodhart_failure = "optimizing momentary affect at a natural break",
        },
        arousal = {
            outcome_role = "diagnostic",
            confounds = { "break_timing", "spectacle_volume" },
            goodhart_failure = "treating arousal as enjoyment",
        },
        soccer_primacy = {
            outcome_role = "guardrail",
            confounds = { "participant_genre_preference" },
            goodhart_failure = "suppressing combat entirely to protect the item",
        },
        fairness = {
            outcome_role = "guardrail",
            confounds = { "win_or_loss", "opponent_policy" },
            goodhart_failure = "removing counterplay variance to raise fairness ratings",
        },
        suspense = {
            outcome_role = "exploratory",
            confounds = { "score_closeness", "match_length" },
            goodhart_failure = "engineering close scores to manufacture suspense",
        },
        counterplay_readability = {
            outcome_role = "guardrail",
            confounds = { "cue_volume", "display_size", "readability_settings" },
            goodhart_failure = "adding cue noise that raises perceived readability",
        },
        overload = {
            outcome_role = "guardrail",
            confounds = { "render_performance", "session_position" },
            goodhart_failure = "cutting simultaneous options to lower overload",
        },
        frustration = {
            outcome_role = "guardrail",
            confounds = { "win_or_loss", "disable_duration" },
            goodhart_failure = "removing all punishment to lower frustration",
        },
        desire_to_explore = {
            outcome_role = "exploratory",
            confounds = { "novelty", "unrewarded_choice_framing" },
            goodhart_failure = "treating rematch clicks as intrinsic curiosity",
        },
    },
    behavioral = {
        {
            id = "soccer_shape_proxy_score",
            version = 1,
            description = "Geometric mean of banded MatchMetrics desirabilities. A soccer-shape proxy over simulated match statistics; it is not a measurement of human fun and carries no participant evidence.",
            grain = "match",
            source_schemas = { "gameplay_trace_manifest/v1" },
            source_fields = { "sim.metrics.MatchMetrics", "sim.metrics.bands" },
            extraction_module = "sim.metrics",
            extraction_config_id = "metrics-fun-score-bands-1",
            numerator = "product of per-band desirabilities present in the match",
            denominator = "count of banded metrics present, as a geometric-mean exponent",
            unit = "index_0_to_1",
            exclusions = { "matches with no banded metric present" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "whole_match" },
            normalization = "geometric_mean_over_present_bands",
            leakage_risk = "outcome_derived",
            observability = "privileged_diagnostic",
            evidence_tier = "soccer_shape_proxy",
            outcome_role = "diagnostic",
            aggregation_levels = { "match" },
            pseudo_replication_guard = "cluster_by_match",
            confounds = {
                "band midpoints are authored targets rather than measured preferences",
                "bot policy strength drives most banded metrics",
            },
            goodhart_failure = "tuning search maximizes band membership while human enjoyment falls",
            prohibited_uses = {
                "human_fun_claim",
                "primary_outcome",
                "proceed_gate",
                "participant_facing_report",
            },
            human_fun_claim = false,
        },
        {
            id = "involuntary_disable_share",
            version = 1,
            description = "Share of eligible active-play ticks in which a player was forcibly unable to act.",
            grain = "player_match",
            source_schemas = { "gameplay_trace_manifest/v1", "research_event_stream/v1" },
            source_fields = { "CombatPlayerState.forced_ticks", "MatchState.ball_in_play" },
            extraction_module = "sim.research_features",
            extraction_config_id = "combat-control-safety-1",
            numerator = "ticks with forced_ticks > 0",
            denominator = "eligible active-play ticks",
            unit = "share_0_to_1",
            exclusions = { "non-active-play ticks", "keeper slots", "practice_block" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "whole_match" },
            normalization = "share_of_denominator",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "behavioral_proxy",
            outcome_role = "guardrail",
            aggregation_levels = { "player_match", "match", "condition_block" },
            pseudo_replication_guard = "cluster_by_match",
            confounds = {
                "voluntary commitment mislabeled as disable",
                "disconnect or process exit truncating the denominator",
            },
            goodhart_failure = "shortening matches to lower the disable count",
            prohibited_uses = { "human_fun_claim", "participant_capability_inference" },
            human_fun_claim = false,
        },
        {
            id = "combat_to_soccer_conversion_rate",
            version = 1,
            description = "Share of confirmed accepted combat encounters that reach a soccer consequence inside the attribution window.",
            grain = "encounter",
            source_schemas = { "research_event_stream/v1" },
            source_fields = {
                "ResearchEventRow.domain",
                "ResearchEventRow.event_kind",
                "ResearchEventRow.source_sequence",
            },
            extraction_module = "sim.research_features",
            extraction_config_id = "combat-funnel-attribution-1",
            numerator = "confirmed encounters with an attributed soccer consequence",
            denominator = "confirmed accepted encounters",
            unit = "share_0_to_1",
            exclusions = {
                "rejected equipment requests",
                "revoked speculative encounters",
                "encounters whose window crosses full time",
            },
            missing_value_behavior = "na_row",
            causal_window = { kind = "forward_ticks", ticks = 120 },
            normalization = "share_of_denominator",
            leakage_risk = "label_adjacent",
            observability = "player_observable",
            evidence_tier = "behavioral_proxy",
            outcome_role = "diagnostic",
            aggregation_levels = { "encounter", "match", "condition_block" },
            pseudo_replication_guard = "cluster_by_match",
            confounds = {
                "attribution window length dominates the rate",
                "possession context differs by side and policy",
            },
            goodhart_failure = "widening the attribution window until every encounter looks causal",
            prohibited_uses = { "human_fun_claim", "speculative_event_inclusion" },
            human_fun_claim = false,
        },
        {
            id = "final_score_margin",
            version = 1,
            description = "Signed home-minus-away goal margin at full time. Outcome-derived: it is a label, not an input.",
            grain = "match",
            source_schemas = { "gameplay_trace_manifest/v1" },
            source_fields = { "sim.metrics.MatchMetrics.margin" },
            extraction_module = "sim.metrics",
            extraction_config_id = "metrics-final-margin-1",
            numerator = "home goals minus away goals at full time",
            unit = "goals",
            exclusions = { "incomplete matches", "practice_block" },
            missing_value_behavior = "drop_row",
            causal_window = { kind = "whole_match" },
            normalization = "none",
            leakage_risk = "outcome_derived",
            observability = "outcome_derived",
            evidence_tier = "behavioral_proxy",
            outcome_role = "diagnostic",
            aggregation_levels = { "match", "condition_block" },
            pseudo_replication_guard = "cluster_by_match",
            confounds = { "side assignment", "opponent policy strength" },
            goodhart_failure = "treating a wider margin as a better match experience",
            prohibited_uses = { "human_fun_claim", "model_input" },
            human_fun_claim = false,
        },
        {
            id = "declared_readability_settings",
            version = 1,
            description = "Readability settings the participant chose, recorded because they change what could be seen on screen.",
            grain = "session",
            source_schemas = { "research_session_envelope/v1" },
            source_fields = { "research_session_envelope.environment.readability_settings" },
            extraction_module = "sim.research_session",
            extraction_config_id = "session-readability-1",
            numerator = "the recorded readability setting map for the session",
            unit = "setting_map",
            exclusions = { "sessions with no recorded settings" },
            missing_value_behavior = "na_row",
            causal_window = { kind = "post_condition" },
            normalization = "none",
            leakage_risk = "none",
            observability = "protected_sensitive",
            evidence_tier = "machine_diagnostic",
            outcome_role = "feature_only",
            aggregation_levels = { "session" },
            pseudo_replication_guard = "cluster_by_participant",
            confounds = { "defaults left untouched are not a preference" },
            goodhart_failure = "reading a settings choice as a statement about the participant",
            prohibited_uses = {
                "human_fun_claim",
                "trait_inference",
                "disability_inference",
                "model_input",
                "per_participant_report",
            },
            human_fun_claim = false,
        },
        {
            id = "inferred_participant_skill_trait",
            version = 1,
            description = "Placeholder for an inferred per-participant skill or trait score. Registered so that referencing it fails the use gate instead of quietly appearing in a pipeline.",
            grain = "participant",
            source_schemas = { "research_session_envelope/v1", "gameplay_trace_manifest/v1" },
            source_fields = { "any combination of behavioral and session fields" },
            extraction_module = "none",
            extraction_config_id = "prohibited-no-extraction",
            numerator = "not computed; this feature has no implementation by design",
            unit = "prohibited",
            exclusions = { "every row: this feature is never extracted" },
            missing_value_behavior = "protocol_failure",
            causal_window = { kind = "whole_match" },
            normalization = "none",
            leakage_risk = "label_adjacent",
            observability = "prohibited",
            evidence_tier = "machine_diagnostic",
            outcome_role = "feature_only",
            aggregation_levels = { "participant" },
            pseudo_replication_guard = "not_an_analysis_unit",
            confounds = { "skill is confounded with device, role, and opponent policy" },
            goodhart_failure = "ranking testers by an invented skill number",
            prohibited_uses = {
                "any_analysis",
                "human_fun_claim",
                "model_input",
                "trait_inference",
                "per_participant_report",
            },
            human_fun_claim = false,
        },
        {
            id = "settled_turnovers_per_active_minute",
            version = 1,
            description = "Settled possession changes per active minute, as a soccer-integrity guardrail.",
            grain = "match",
            source_schemas = { "gameplay_trace_manifest/v1" },
            source_fields = { "sim.metrics.MatchMetrics.turnovers_per_min" },
            extraction_module = "sim.metrics",
            extraction_config_id = "metrics-settled-turnovers-1",
            numerator = "settled team possession changes",
            denominator = "active-play minutes",
            unit = "count_per_minute",
            exclusions = { "ownership flicker below the settle hold", "practice_block" },
            missing_value_behavior = "protocol_failure",
            causal_window = { kind = "whole_match" },
            normalization = "per_active_minute",
            leakage_risk = "none",
            observability = "player_observable",
            evidence_tier = "behavioral_proxy",
            outcome_role = "guardrail",
            aggregation_levels = { "match", "condition_block" },
            pseudo_replication_guard = "cluster_by_match",
            confounds = { "settle-hold threshold choice", "side and skill asymmetry" },
            goodhart_failure = "raising raw ownership churn and calling it dynamism",
            prohibited_uses = { "human_fun_claim" },
            human_fun_claim = false,
        },
    },
}
