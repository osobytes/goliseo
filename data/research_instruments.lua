-- Instrument register for the human playtest experience-response contract.
--
-- This table holds *structure and provenance only*. No instrument item text,
-- label wording, or asset is reproduced here: section 5.1 of
-- docs/design/combat_fun_evidence_contract.md forbids copying instrument text
-- into the repository until its exact reuse terms and version are recorded, so
-- every record carries `item_text_included = false` and points at the source
-- register key instead.
--
-- Constructs stay separately named. There is no field that could collapse PXI
-- enjoyment, BANGS satisfaction/frustration, Affective Slider valence/arousal,
-- and the custom exploratory items into one composite.

---@alias ResearchAnalysisRole "primary"|"diagnostic"|"exploratory"
---@alias ResearchScaleKind "likert"|"continuous"
---@alias ResearchScoreAggregation "mean_all_items"|"item_only"

---@class ResearchInstrumentItemData
---@field id string
---@field construct string
---@field position integer -- canonical position; presentation order is per-response
---@field reverse_scored boolean

---@class ResearchInstrumentData
---@field id string
---@field name string
---@field source_register_key string -- key in the combat fun evidence contract source register
---@field instrument_version string
---@field scoring_key_version string
---@field validated boolean
---@field partial_administration boolean
---@field analysis_role ResearchAnalysisRole
---@field scale_kind ResearchScaleKind
---@field scale_min number
---@field scale_max number
---@field scale_step number
---@field label_count integer?
---@field score_aggregation ResearchScoreAggregation
---@field requires_all_items boolean
---@field license string
---@field pooling_group string
---@field forbidden_pooling string[]
---@field substitute_for string? -- instrument this record stands in for, if any
---@field item_text_included boolean
---@field constructs string[]
---@field items ResearchInstrumentItemData[]

---@type table<string, ResearchInstrumentData>
return {
    pxi_enjoyment_addon = {
        id = "pxi_enjoyment_addon",
        name = "PXI enjoyment add-on",
        source_register_key = "pxi-independent",
        instrument_version = "pxi-2021.1-enjoyment-addon",
        scoring_key_version = "pxi-enjoyment-mean-1",
        validated = true,
        partial_administration = false,
        analysis_role = "primary",
        scale_kind = "likert",
        scale_min = -3,
        scale_max = 3,
        scale_step = 1,
        label_count = 7,
        score_aggregation = "mean_all_items",
        requires_all_items = true,
        license = "open-access-unchanged-items",
        pooling_group = "pxi_enjoyment",
        forbidden_pooling = { "pxi_partial_mechanisms", "bangs_session", "custom_diagnostics" },
        item_text_included = false,
        constructs = { "enjoyment" },
        items = {
            { id = "pxi_enjoy_1", construct = "enjoyment", position = 1, reverse_scored = false },
            { id = "pxi_enjoy_2", construct = "enjoyment", position = 2, reverse_scored = false },
            { id = "pxi_enjoy_3", construct = "enjoyment", position = 3, reverse_scored = false },
        },
    },
    pxi_partial_mechanisms = {
        id = "pxi_partial_mechanisms",
        name = "Partial-PXI mechanism subscales",
        source_register_key = "pxi",
        instrument_version = "pxi-2021.1-subset",
        scoring_key_version = "pxi-subscale-mean-1",
        validated = true,
        partial_administration = true,
        analysis_role = "diagnostic",
        scale_kind = "likert",
        scale_min = -3,
        scale_max = 3,
        scale_step = 1,
        label_count = 7,
        score_aggregation = "mean_all_items",
        requires_all_items = true,
        license = "open-access-unchanged-items",
        pooling_group = "pxi_partial",
        forbidden_pooling = { "pxi_enjoyment_addon", "bangs_session", "custom_diagnostics" },
        item_text_included = false,
        constructs = {
            "autonomy",
            "mastery",
            "challenge",
            "ease_of_control",
            "goals_and_rules",
            "progress_feedback",
        },
        items = {
            { id = "pxi_auto_1", construct = "autonomy", position = 1, reverse_scored = false },
            { id = "pxi_auto_2", construct = "autonomy", position = 2, reverse_scored = false },
            { id = "pxi_auto_3", construct = "autonomy", position = 3, reverse_scored = false },
            { id = "pxi_mast_1", construct = "mastery", position = 4, reverse_scored = false },
            { id = "pxi_mast_2", construct = "mastery", position = 5, reverse_scored = false },
            { id = "pxi_mast_3", construct = "mastery", position = 6, reverse_scored = false },
            { id = "pxi_chal_1", construct = "challenge", position = 7, reverse_scored = false },
            { id = "pxi_chal_2", construct = "challenge", position = 8, reverse_scored = false },
            { id = "pxi_chal_3", construct = "challenge", position = 9, reverse_scored = false },
            {
                id = "pxi_ease_1",
                construct = "ease_of_control",
                position = 10,
                reverse_scored = false,
            },
            {
                id = "pxi_ease_2",
                construct = "ease_of_control",
                position = 11,
                reverse_scored = false,
            },
            {
                id = "pxi_ease_3",
                construct = "ease_of_control",
                position = 12,
                reverse_scored = false,
            },
            {
                id = "pxi_goal_1",
                construct = "goals_and_rules",
                position = 13,
                reverse_scored = false,
            },
            {
                id = "pxi_goal_2",
                construct = "goals_and_rules",
                position = 14,
                reverse_scored = false,
            },
            {
                id = "pxi_goal_3",
                construct = "goals_and_rules",
                position = 15,
                reverse_scored = false,
            },
            {
                id = "pxi_prog_1",
                construct = "progress_feedback",
                position = 16,
                reverse_scored = false,
            },
            {
                id = "pxi_prog_2",
                construct = "progress_feedback",
                position = 17,
                reverse_scored = false,
            },
            {
                id = "pxi_prog_3",
                construct = "progress_feedback",
                position = 18,
                reverse_scored = false,
            },
        },
    },
    bangs_session = {
        id = "bangs_session",
        name = "BANGS particular-session variant",
        source_register_key = "bangs",
        instrument_version = "bangs-2024.1-session",
        scoring_key_version = "bangs-subscale-mean-1",
        validated = true,
        partial_administration = false,
        analysis_role = "exploratory",
        scale_kind = "likert",
        scale_min = 1,
        scale_max = 7,
        scale_step = 1,
        label_count = 7,
        score_aggregation = "mean_all_items",
        requires_all_items = true,
        license = "cc-by-4.0-article-cc-by-sa-4.0-guide",
        pooling_group = "bangs",
        forbidden_pooling = {
            "pxi_enjoyment_addon",
            "pxi_partial_mechanisms",
            "custom_diagnostics",
        },
        item_text_included = false,
        constructs = {
            "autonomy_satisfaction",
            "autonomy_frustration",
            "competence_satisfaction",
            "competence_frustration",
            "relatedness_satisfaction",
            "relatedness_frustration",
        },
        items = {
            {
                id = "bangs_auto_sat_1",
                construct = "autonomy_satisfaction",
                position = 1,
                reverse_scored = false,
            },
            {
                id = "bangs_auto_sat_2",
                construct = "autonomy_satisfaction",
                position = 2,
                reverse_scored = false,
            },
            {
                id = "bangs_auto_sat_3",
                construct = "autonomy_satisfaction",
                position = 3,
                reverse_scored = false,
            },
            {
                id = "bangs_auto_fru_1",
                construct = "autonomy_frustration",
                position = 4,
                reverse_scored = false,
            },
            {
                id = "bangs_auto_fru_2",
                construct = "autonomy_frustration",
                position = 5,
                reverse_scored = false,
            },
            {
                id = "bangs_auto_fru_3",
                construct = "autonomy_frustration",
                position = 6,
                reverse_scored = false,
            },
            {
                id = "bangs_comp_sat_1",
                construct = "competence_satisfaction",
                position = 7,
                reverse_scored = false,
            },
            {
                id = "bangs_comp_sat_2",
                construct = "competence_satisfaction",
                position = 8,
                reverse_scored = false,
            },
            {
                id = "bangs_comp_sat_3",
                construct = "competence_satisfaction",
                position = 9,
                reverse_scored = false,
            },
            {
                id = "bangs_comp_fru_1",
                construct = "competence_frustration",
                position = 10,
                reverse_scored = false,
            },
            {
                id = "bangs_comp_fru_2",
                construct = "competence_frustration",
                position = 11,
                reverse_scored = false,
            },
            {
                id = "bangs_comp_fru_3",
                construct = "competence_frustration",
                position = 12,
                reverse_scored = false,
            },
            {
                id = "bangs_rel_sat_1",
                construct = "relatedness_satisfaction",
                position = 13,
                reverse_scored = false,
            },
            {
                id = "bangs_rel_sat_2",
                construct = "relatedness_satisfaction",
                position = 14,
                reverse_scored = false,
            },
            {
                id = "bangs_rel_sat_3",
                construct = "relatedness_satisfaction",
                position = 15,
                reverse_scored = false,
            },
            {
                id = "bangs_rel_fru_1",
                construct = "relatedness_frustration",
                position = 16,
                reverse_scored = false,
            },
            {
                id = "bangs_rel_fru_2",
                construct = "relatedness_frustration",
                position = 17,
                reverse_scored = false,
            },
            {
                id = "bangs_rel_fru_3",
                construct = "relatedness_frustration",
                position = 18,
                reverse_scored = false,
            },
        },
    },
    affective_slider = {
        id = "affective_slider",
        name = "Affective Slider",
        source_register_key = "affective",
        instrument_version = "affective-slider-2016.1",
        scoring_key_version = "affective-raw-1",
        validated = true,
        partial_administration = false,
        analysis_role = "diagnostic",
        scale_kind = "continuous",
        scale_min = 0,
        scale_max = 1,
        scale_step = 0.01,
        label_count = nil,
        score_aggregation = "item_only",
        requires_all_items = false,
        license = "cc-by-sa-4.0",
        pooling_group = "affective_slider",
        forbidden_pooling = { "custom_affect_fallback" },
        item_text_included = false,
        constructs = { "valence", "arousal" },
        items = {
            { id = "as_valence", construct = "valence", position = 1, reverse_scored = false },
            { id = "as_arousal", construct = "arousal", position = 2, reverse_scored = false },
        },
    },
    custom_affect_fallback = {
        id = "custom_affect_fallback",
        name = "Custom accessible affect fallback",
        source_register_key = "project-custom",
        instrument_version = "goliseo-affect-fallback-1",
        scoring_key_version = "goliseo-affect-fallback-1",
        validated = false,
        partial_administration = false,
        analysis_role = "diagnostic",
        scale_kind = "likert",
        scale_min = 1,
        scale_max = 7,
        scale_step = 1,
        label_count = 7,
        score_aggregation = "item_only",
        requires_all_items = false,
        license = "project-internal",
        pooling_group = "custom_affect_fallback",
        forbidden_pooling = { "affective_slider" },
        substitute_for = "affective_slider",
        item_text_included = false,
        constructs = { "valence", "arousal" },
        items = {
            {
                id = "fallback_valence",
                construct = "valence",
                position = 1,
                reverse_scored = false,
            },
            {
                id = "fallback_arousal",
                construct = "arousal",
                position = 2,
                reverse_scored = false,
            },
        },
    },
    custom_diagnostics = {
        id = "custom_diagnostics",
        name = "Custom exploratory single items",
        source_register_key = "project-custom",
        instrument_version = "goliseo-custom-items-1",
        scoring_key_version = "goliseo-custom-items-1",
        validated = false,
        partial_administration = false,
        analysis_role = "exploratory",
        scale_kind = "likert",
        scale_min = -3,
        scale_max = 3,
        scale_step = 1,
        label_count = 7,
        score_aggregation = "item_only",
        requires_all_items = false,
        license = "project-internal",
        pooling_group = "custom_diagnostics",
        forbidden_pooling = {
            "pxi_enjoyment_addon",
            "pxi_partial_mechanisms",
            "bangs_session",
        },
        item_text_included = false,
        constructs = {
            "soccer_primacy",
            "fairness",
            "suspense",
            "counterplay_readability",
            "overload",
            "frustration",
            "desire_to_explore",
        },
        items = {
            {
                id = "custom_soccer_primacy",
                construct = "soccer_primacy",
                position = 1,
                reverse_scored = false,
            },
            {
                id = "custom_fairness",
                construct = "fairness",
                position = 2,
                reverse_scored = false,
            },
            {
                id = "custom_suspense",
                construct = "suspense",
                position = 3,
                reverse_scored = false,
            },
            {
                id = "custom_counterplay_readability",
                construct = "counterplay_readability",
                position = 4,
                reverse_scored = false,
            },
            {
                id = "custom_overload",
                construct = "overload",
                position = 5,
                reverse_scored = false,
            },
            {
                id = "custom_frustration",
                construct = "frustration",
                position = 6,
                reverse_scored = false,
            },
            {
                id = "custom_desire_to_explore",
                construct = "desire_to_explore",
                position = 7,
                reverse_scored = false,
            },
        },
    },
}
