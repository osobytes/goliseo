//! Instrument register for the human playtest experience-response contract.
//!
//! This table holds *structure and provenance only*. No instrument item text,
//! label wording, or asset is reproduced here: section 5.1 of
//! docs/design/combat_fun_evidence_contract.md forbids copying instrument text
//! into the repository until its exact reuse terms and version are recorded, so
//! every record carries `item_text_included = false` and points at the source
//! register key instead.
//!
//! Constructs stay separately named. There is no field that could collapse PXI
//! enjoyment, BANGS satisfaction/frustration, Affective Slider valence/arousal,
//! and the custom exploratory items into one composite.

/// The analytic role an instrument plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchAnalysisRole {
    /// The primary instrument for its construct.
    Primary,
    /// A diagnostic instrument.
    Diagnostic,
    /// An exploratory instrument.
    Exploratory,
}

/// The kind of response scale an instrument uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchScaleKind {
    /// A discrete Likert scale.
    Likert,
    /// A continuous scale.
    Continuous,
}

/// How an instrument's items are aggregated into a score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchScoreAggregation {
    /// Mean across all items.
    MeanAllItems,
    /// Each item stands alone; no aggregation.
    ItemOnly,
}

/// One item within an instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchInstrumentItemData {
    /// Persistent identity.
    pub id: &'static str,
    /// Construct this item measures.
    pub construct: &'static str,
    /// Canonical position; presentation order is per-response.
    pub position: i64,
    /// Whether this item's score is reverse-coded.
    pub reverse_scored: bool,
}

/// A registered research instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchInstrumentData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Key in the combat fun evidence contract source register.
    pub source_register_key: &'static str,
    /// Instrument version.
    pub instrument_version: &'static str,
    /// Scoring key version.
    pub scoring_key_version: &'static str,
    /// Whether the instrument is a validated psychometric scale.
    pub validated: bool,
    /// Whether only part of the validated instrument is administered.
    pub partial_administration: bool,
    /// Analytic role this instrument plays.
    pub analysis_role: ResearchAnalysisRole,
    /// Kind of response scale.
    pub scale_kind: ResearchScaleKind,
    /// Minimum scale value.
    pub scale_min: f64,
    /// Maximum scale value.
    pub scale_max: f64,
    /// Scale step size.
    pub scale_step: f64,
    /// Number of labeled scale points, for Likert scales.
    pub label_count: Option<i64>,
    /// How items aggregate into a score.
    pub score_aggregation: ResearchScoreAggregation,
    /// Whether every item must be answered for a valid score.
    pub requires_all_items: bool,
    /// License terms this instrument is used under.
    pub license: &'static str,
    /// Pooling group; instruments outside `forbidden_pooling` may share analysis.
    pub pooling_group: &'static str,
    /// Instrument ids this instrument's scores must never be pooled with.
    pub forbidden_pooling: &'static [&'static str],
    /// Instrument this record stands in for, if any.
    pub substitute_for: Option<&'static str>,
    /// Whether instrument item text is reproduced in this repository (always false).
    pub item_text_included: bool,
    /// Constructs this instrument measures.
    pub constructs: &'static [&'static str],
    /// Items making up this instrument.
    pub items: &'static [ResearchInstrumentItemData],
}

/// Every registered research instrument.
pub static ALL: &[ResearchInstrumentData] = &[
    ResearchInstrumentData {
        id: "pxi_enjoyment_addon",
        name: "PXI enjoyment add-on",
        source_register_key: "pxi-independent",
        instrument_version: "pxi-2021.1-enjoyment-addon",
        scoring_key_version: "pxi-enjoyment-mean-1",
        validated: true,
        partial_administration: false,
        analysis_role: ResearchAnalysisRole::Primary,
        scale_kind: ResearchScaleKind::Likert,
        scale_min: -3.0,
        scale_max: 3.0,
        scale_step: 1.0,
        label_count: Some(7),
        score_aggregation: ResearchScoreAggregation::MeanAllItems,
        requires_all_items: true,
        license: "open-access-unchanged-items",
        pooling_group: "pxi_enjoyment",
        forbidden_pooling: &[
            "pxi_partial_mechanisms",
            "bangs_session",
            "custom_diagnostics",
        ],
        substitute_for: None,
        item_text_included: false,
        constructs: &["enjoyment"],
        items: &[
            ResearchInstrumentItemData {
                id: "pxi_enjoy_1",
                construct: "enjoyment",
                position: 1,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_enjoy_2",
                construct: "enjoyment",
                position: 2,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_enjoy_3",
                construct: "enjoyment",
                position: 3,
                reverse_scored: false,
            },
        ],
    },
    ResearchInstrumentData {
        id: "pxi_partial_mechanisms",
        name: "Partial-PXI mechanism subscales",
        source_register_key: "pxi",
        instrument_version: "pxi-2021.1-subset",
        scoring_key_version: "pxi-subscale-mean-1",
        validated: true,
        partial_administration: true,
        analysis_role: ResearchAnalysisRole::Diagnostic,
        scale_kind: ResearchScaleKind::Likert,
        scale_min: -3.0,
        scale_max: 3.0,
        scale_step: 1.0,
        label_count: Some(7),
        score_aggregation: ResearchScoreAggregation::MeanAllItems,
        requires_all_items: true,
        license: "open-access-unchanged-items",
        pooling_group: "pxi_partial",
        forbidden_pooling: &["pxi_enjoyment_addon", "bangs_session", "custom_diagnostics"],
        substitute_for: None,
        item_text_included: false,
        constructs: &[
            "autonomy",
            "mastery",
            "challenge",
            "ease_of_control",
            "goals_and_rules",
            "progress_feedback",
        ],
        items: &[
            ResearchInstrumentItemData {
                id: "pxi_auto_1",
                construct: "autonomy",
                position: 1,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_auto_2",
                construct: "autonomy",
                position: 2,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_auto_3",
                construct: "autonomy",
                position: 3,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_mast_1",
                construct: "mastery",
                position: 4,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_mast_2",
                construct: "mastery",
                position: 5,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_mast_3",
                construct: "mastery",
                position: 6,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_chal_1",
                construct: "challenge",
                position: 7,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_chal_2",
                construct: "challenge",
                position: 8,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_chal_3",
                construct: "challenge",
                position: 9,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_ease_1",
                construct: "ease_of_control",
                position: 10,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_ease_2",
                construct: "ease_of_control",
                position: 11,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_ease_3",
                construct: "ease_of_control",
                position: 12,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_goal_1",
                construct: "goals_and_rules",
                position: 13,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_goal_2",
                construct: "goals_and_rules",
                position: 14,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_goal_3",
                construct: "goals_and_rules",
                position: 15,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_prog_1",
                construct: "progress_feedback",
                position: 16,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_prog_2",
                construct: "progress_feedback",
                position: 17,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "pxi_prog_3",
                construct: "progress_feedback",
                position: 18,
                reverse_scored: false,
            },
        ],
    },
    ResearchInstrumentData {
        id: "bangs_session",
        name: "BANGS particular-session variant",
        source_register_key: "bangs",
        instrument_version: "bangs-2024.1-session",
        scoring_key_version: "bangs-subscale-mean-1",
        validated: true,
        partial_administration: false,
        analysis_role: ResearchAnalysisRole::Exploratory,
        scale_kind: ResearchScaleKind::Likert,
        scale_min: 1.0,
        scale_max: 7.0,
        scale_step: 1.0,
        label_count: Some(7),
        score_aggregation: ResearchScoreAggregation::MeanAllItems,
        requires_all_items: true,
        license: "cc-by-4.0-article-cc-by-sa-4.0-guide",
        pooling_group: "bangs",
        forbidden_pooling: &[
            "pxi_enjoyment_addon",
            "pxi_partial_mechanisms",
            "custom_diagnostics",
        ],
        substitute_for: None,
        item_text_included: false,
        constructs: &[
            "autonomy_satisfaction",
            "autonomy_frustration",
            "competence_satisfaction",
            "competence_frustration",
            "relatedness_satisfaction",
            "relatedness_frustration",
        ],
        items: &[
            ResearchInstrumentItemData {
                id: "bangs_auto_sat_1",
                construct: "autonomy_satisfaction",
                position: 1,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_auto_sat_2",
                construct: "autonomy_satisfaction",
                position: 2,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_auto_sat_3",
                construct: "autonomy_satisfaction",
                position: 3,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_auto_fru_1",
                construct: "autonomy_frustration",
                position: 4,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_auto_fru_2",
                construct: "autonomy_frustration",
                position: 5,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_auto_fru_3",
                construct: "autonomy_frustration",
                position: 6,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_comp_sat_1",
                construct: "competence_satisfaction",
                position: 7,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_comp_sat_2",
                construct: "competence_satisfaction",
                position: 8,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_comp_sat_3",
                construct: "competence_satisfaction",
                position: 9,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_comp_fru_1",
                construct: "competence_frustration",
                position: 10,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_comp_fru_2",
                construct: "competence_frustration",
                position: 11,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_comp_fru_3",
                construct: "competence_frustration",
                position: 12,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_rel_sat_1",
                construct: "relatedness_satisfaction",
                position: 13,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_rel_sat_2",
                construct: "relatedness_satisfaction",
                position: 14,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_rel_sat_3",
                construct: "relatedness_satisfaction",
                position: 15,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_rel_fru_1",
                construct: "relatedness_frustration",
                position: 16,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_rel_fru_2",
                construct: "relatedness_frustration",
                position: 17,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "bangs_rel_fru_3",
                construct: "relatedness_frustration",
                position: 18,
                reverse_scored: false,
            },
        ],
    },
    ResearchInstrumentData {
        id: "affective_slider",
        name: "Affective Slider",
        source_register_key: "affective",
        instrument_version: "affective-slider-2016.1",
        scoring_key_version: "affective-raw-1",
        validated: true,
        partial_administration: false,
        analysis_role: ResearchAnalysisRole::Diagnostic,
        scale_kind: ResearchScaleKind::Continuous,
        scale_min: 0.0,
        scale_max: 1.0,
        scale_step: 0.01,
        label_count: None,
        score_aggregation: ResearchScoreAggregation::ItemOnly,
        requires_all_items: false,
        license: "cc-by-sa-4.0",
        pooling_group: "affective_slider",
        forbidden_pooling: &["custom_affect_fallback"],
        substitute_for: None,
        item_text_included: false,
        constructs: &["valence", "arousal"],
        items: &[
            ResearchInstrumentItemData {
                id: "as_valence",
                construct: "valence",
                position: 1,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "as_arousal",
                construct: "arousal",
                position: 2,
                reverse_scored: false,
            },
        ],
    },
    ResearchInstrumentData {
        id: "custom_affect_fallback",
        name: "Custom accessible affect fallback",
        source_register_key: "project-custom",
        instrument_version: "goliseo-affect-fallback-1",
        scoring_key_version: "goliseo-affect-fallback-1",
        validated: false,
        partial_administration: false,
        analysis_role: ResearchAnalysisRole::Diagnostic,
        scale_kind: ResearchScaleKind::Likert,
        scale_min: 1.0,
        scale_max: 7.0,
        scale_step: 1.0,
        label_count: Some(7),
        score_aggregation: ResearchScoreAggregation::ItemOnly,
        requires_all_items: false,
        license: "project-internal",
        pooling_group: "custom_affect_fallback",
        forbidden_pooling: &["affective_slider"],
        substitute_for: Some("affective_slider"),
        item_text_included: false,
        constructs: &["valence", "arousal"],
        items: &[
            ResearchInstrumentItemData {
                id: "fallback_valence",
                construct: "valence",
                position: 1,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "fallback_arousal",
                construct: "arousal",
                position: 2,
                reverse_scored: false,
            },
        ],
    },
    ResearchInstrumentData {
        id: "custom_diagnostics",
        name: "Custom exploratory single items",
        source_register_key: "project-custom",
        instrument_version: "goliseo-custom-items-1",
        scoring_key_version: "goliseo-custom-items-1",
        validated: false,
        partial_administration: false,
        analysis_role: ResearchAnalysisRole::Exploratory,
        scale_kind: ResearchScaleKind::Likert,
        scale_min: -3.0,
        scale_max: 3.0,
        scale_step: 1.0,
        label_count: Some(7),
        score_aggregation: ResearchScoreAggregation::ItemOnly,
        requires_all_items: false,
        license: "project-internal",
        pooling_group: "custom_diagnostics",
        forbidden_pooling: &[
            "pxi_enjoyment_addon",
            "pxi_partial_mechanisms",
            "bangs_session",
        ],
        substitute_for: None,
        item_text_included: false,
        constructs: &[
            "soccer_primacy",
            "fairness",
            "suspense",
            "counterplay_readability",
            "overload",
            "frustration",
            "desire_to_explore",
        ],
        items: &[
            ResearchInstrumentItemData {
                id: "custom_soccer_primacy",
                construct: "soccer_primacy",
                position: 1,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "custom_fairness",
                construct: "fairness",
                position: 2,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "custom_suspense",
                construct: "suspense",
                position: 3,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "custom_counterplay_readability",
                construct: "counterplay_readability",
                position: 4,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "custom_overload",
                construct: "overload",
                position: 5,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "custom_frustration",
                construct: "frustration",
                position: 6,
                reverse_scored: false,
            },
            ResearchInstrumentItemData {
                id: "custom_desire_to_explore",
                construct: "desire_to_explore",
                position: 7,
                reverse_scored: false,
            },
        ],
    },
];

/// Look up a research instrument by id.
pub fn get(id: &str) -> Option<&'static ResearchInstrumentData> {
    ALL.iter().find(|instrument| instrument.id == id)
}
