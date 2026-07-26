-- Derived feature register: expansion, invariants, and lookup.
--
-- The register is authored content (`data/research_features.lua` plus
-- `data/research_instruments.lua`), so structural violations assert. The
-- invariants that matter are the ones a model or a report could otherwise
-- violate quietly:
--
--   * only a `human_experience` feature may be cited as human-fun evidence, so
--     `metrics.fun_score` is registered as `soccer_shape_proxy_score` with
--     `human_fun_claim = false` and `human_fun_claim` in its prohibited uses;
--   * every instrument construct has exactly one feature and every
--     instrument-derived feature has a construct, so a construct cannot be
--     analysed without a definition; and
--   * a tick/decision/encounter-grain feature may never declare a participant
--     as its independent unit, which is the pseudo-replication trap.

local instruments = require("data.research_instruments")
local research_schema = require("sim.research_schema")
local source = require("data.research_features")

---@class ResearchFeaturesModule
local research_features = {}

research_features.VERSION = 1
research_features.SUPPORTED_VERSIONS = { [1] = true }
research_features.REGISTRY_LABEL = "research-feature-registry/v1"

research_features.GRAINS = research_schema.enum({
    "tick",
    "decision",
    "possession",
    "encounter",
    "match",
    "player_match",
    "condition_block",
    "session",
    "participant",
})

-- Grains whose rows are not independent participants. A feature at these
-- grains must cluster, never treat a row as a participant-level unit.
research_features.CLUSTERED_GRAINS = {
    tick = true,
    decision = true,
    possession = true,
    encounter = true,
    match = true,
    player_match = true,
}

research_features.SHAPE = research_schema.record("research_feature/v1", {
    { name = "id", kind = "id" },
    { name = "version", kind = "integer", min = 1 },
    { name = "description", kind = "string", max_length = 512 },
    { name = "grain", kind = "enum", values = research_features.GRAINS },
    {
        name = "source_schemas",
        kind = "array",
        min_length = 1,
        element = { name = "schema", kind = "string" },
    },
    {
        name = "source_fields",
        kind = "array",
        min_length = 1,
        element = { name = "field", kind = "string" },
    },
    { name = "extraction_module", kind = "string" },
    { name = "extraction_config_id", kind = "id" },
    { name = "numerator", kind = "string" },
    { name = "denominator", kind = "string", optional = true },
    { name = "unit", kind = "id" },
    {
        name = "exclusions",
        kind = "array",
        element = { name = "exclusion", kind = "string" },
    },
    {
        name = "missing_value_behavior",
        kind = "enum",
        values = research_schema.enum({
            "na_row",
            "drop_row",
            "zero_when_defined",
            "protocol_failure",
        }),
    },
    {
        name = "causal_window",
        kind = "record",
        fields = {
            {
                name = "kind",
                kind = "enum",
                values = research_schema.enum({
                    "same_tick",
                    "forward_ticks",
                    "backward_ticks",
                    "whole_match",
                    "post_condition",
                }),
            },
            { name = "ticks", kind = "integer", optional = true, min = 1 },
        },
    },
    { name = "normalization", kind = "id" },
    {
        name = "leakage_risk",
        kind = "enum",
        values = research_schema.enum({
            "none",
            "outcome_derived",
            "privileged_state",
            "future_information",
            "label_adjacent",
        }),
    },
    {
        name = "observability",
        kind = "enum",
        values = research_schema.enum({
            "player_observable",
            "privileged_diagnostic",
            "outcome_derived",
            "protected_sensitive",
            "prohibited",
        }),
    },
    {
        name = "evidence_tier",
        kind = "enum",
        values = research_schema.enum({
            "human_experience",
            "behavioral_proxy",
            "soccer_shape_proxy",
            "machine_diagnostic",
        }),
    },
    {
        name = "outcome_role",
        kind = "enum",
        values = research_schema.enum({
            "primary_outcome",
            "secondary_outcome",
            "guardrail",
            "diagnostic",
            "exploratory",
            "feature_only",
        }),
    },
    {
        name = "aggregation_levels",
        kind = "array",
        min_length = 1,
        element = { name = "level", kind = "enum", values = research_features.GRAINS },
    },
    {
        name = "pseudo_replication_guard",
        kind = "enum",
        values = research_schema.enum({
            "independent_unit_participant",
            "cluster_by_participant",
            "cluster_by_match",
            "not_an_analysis_unit",
        }),
    },
    {
        name = "confounds",
        kind = "array",
        min_length = 1,
        element = { name = "confound", kind = "string" },
    },
    { name = "goodhart_failure", kind = "string" },
    {
        name = "prohibited_uses",
        kind = "array",
        element = { name = "use", kind = "id" },
    },
    { name = "human_fun_claim", kind = "boolean" },
})

---@param instrument_id string
---@param construct string
---@return string
function research_features.feature_id(instrument_id, construct)
    return instrument_id .. "." .. construct
end

---@param defaults ResearchInstrumentFeatureDefaults
---@param instrument ResearchInstrumentData
---@param construct string
---@param note ResearchConstructNoteData
---@return ResearchFeatureData
local function expand_construct(defaults, instrument, construct, note)
    local item_ids = {}
    for _, item in ipairs(instrument.items) do
        if item.construct == construct then
            item_ids[#item_ids + 1] = "research_response_set.responses[" .. item.id .. "]"
        end
    end
    local mean_scored = instrument.score_aggregation == "mean_all_items"
    local prohibited = {}
    for index, use in ipairs(defaults.prohibited_uses) do
        prohibited[index] = use
    end
    if note.outcome_role ~= "primary_outcome" then
        prohibited[#prohibited + 1] = "primary_outcome"
    end
    return {
        id = research_features.feature_id(instrument.id, construct),
        version = source.version,
        description = ("Construct %s from %s (%s), scored by %s."):format(
            construct,
            instrument.name,
            instrument.instrument_version,
            instrument.scoring_key_version
        ),
        grain = defaults.grain,
        source_schemas = research_schema.copy(defaults.source_schemas),
        source_fields = item_ids,
        extraction_module = defaults.extraction_module,
        extraction_config_id = defaults.extraction_config_id,
        numerator = mean_scored and ("sum of answered " .. construct .. " item responses")
            or ("raw response for " .. construct),
        denominator = mean_scored
                and ("count of " .. construct .. " items; every item is required")
            or nil,
        unit = defaults.unit,
        exclusions = research_schema.copy(defaults.exclusions),
        missing_value_behavior = defaults.missing_value_behavior,
        causal_window = research_schema.copy(defaults.causal_window),
        normalization = defaults.normalization,
        leakage_risk = defaults.leakage_risk,
        observability = defaults.observability,
        evidence_tier = defaults.evidence_tier,
        outcome_role = note.outcome_role,
        aggregation_levels = research_schema.copy(defaults.aggregation_levels),
        pseudo_replication_guard = defaults.pseudo_replication_guard,
        confounds = research_schema.copy(note.confounds),
        goodhart_failure = note.goodhart_failure,
        prohibited_uses = prohibited,
        -- Only the primary human-experience endpoint may be cited as evidence
        -- about human fun. Everything else is a mechanism, guardrail, or proxy.
        human_fun_claim = defaults.evidence_tier == "human_experience"
            and note.outcome_role == "primary_outcome",
    }
end

---@param feature ResearchFeatureData
---@param label string
local function assert_feature_invariants(feature, label)
    local ok, err = research_schema.validate(research_features.SHAPE, feature)
    assert(ok, label .. ": " .. tostring(err))

    if feature.human_fun_claim then
        assert(
            feature.evidence_tier == "human_experience",
            label .. " may not claim human-fun evidence outside a human-experience instrument"
        )
        assert(
            feature.outcome_role == "primary_outcome",
            label .. " may only claim human-fun evidence as the primary outcome"
        )
    end
    if feature.outcome_role == "primary_outcome" then
        assert(
            feature.evidence_tier == "human_experience",
            label .. " cannot be a primary outcome without human-experience evidence"
        )
        assert(feature.leakage_risk == "none", label .. " primary outcomes must be leakage-free")
    end
    if feature.evidence_tier == "soccer_shape_proxy" then
        local forbids_fun_claim = false
        for _, use in ipairs(feature.prohibited_uses) do
            if use == "human_fun_claim" then
                forbids_fun_claim = true
            end
        end
        assert(
            forbids_fun_claim,
            label .. " is a soccer-shape proxy and must prohibit human_fun_claim"
        )
        assert(not feature.human_fun_claim, label .. " soccer-shape proxies are never human fun")
    end
    if feature.observability == "prohibited" then
        assert(
            feature.outcome_role == "feature_only" and #feature.prohibited_uses > 0,
            label .. " prohibited features must record why they are unusable"
        )
    end
    if
        feature.causal_window.kind == "forward_ticks"
        or feature.causal_window.kind == "backward_ticks"
    then
        assert(feature.causal_window.ticks, label .. " windowed features must record their window")
    else
        assert(
            feature.causal_window.ticks == nil,
            label .. " non-windowed features cannot declare a tick window"
        )
    end
    local includes_grain = false
    for _, level in ipairs(feature.aggregation_levels) do
        if level == feature.grain then
            includes_grain = true
        end
    end
    assert(includes_grain, label .. " aggregation levels must include its own grain")
    if research_features.CLUSTERED_GRAINS[feature.grain] then
        assert(
            feature.pseudo_replication_guard ~= "independent_unit_participant",
            label .. " rows at this grain are not independent participants"
        )
    end
    if feature.denominator == nil then
        assert(feature.normalization == "none", label .. " cannot normalize without a denominator")
    end
end

---@return table<string, ResearchFeatureData>
local function build_registry()
    assert(type(source) == "table", "research feature source data is required")
    assert(
        research_features.SUPPORTED_VERSIONS[source.version],
        "research feature source version is unsupported"
    )
    local registry = {}

    for instrument_id, defaults in pairs(source.instrument_defaults) do
        local instrument = assert(
            instruments[instrument_id],
            "research feature defaults name unknown instrument " .. instrument_id
        )
        for _, construct in ipairs(instrument.constructs) do
            local note = assert(
                source.construct_notes[construct],
                "research construct " .. construct .. " has no registered feature notes"
            )
            local feature = expand_construct(defaults, instrument, construct, note)
            assert(not registry[feature.id], "duplicate research feature " .. feature.id)
            assert_feature_invariants(feature, "research feature " .. feature.id)
            registry[feature.id] = feature
        end
    end

    for _, feature in ipairs(source.behavioral) do
        assert(not registry[feature.id], "duplicate research feature " .. feature.id)
        assert_feature_invariants(feature, "research feature " .. feature.id)
        registry[feature.id] = feature
    end

    -- Every instrument construct must be registered exactly once.
    for instrument_id, instrument in pairs(instruments) do
        assert(
            source.instrument_defaults[instrument_id],
            "instrument " .. instrument_id .. " has no feature defaults"
        )
        for _, construct in ipairs(instrument.constructs) do
            assert(
                registry[research_features.feature_id(instrument_id, construct)],
                "instrument construct " .. instrument_id .. "." .. construct .. " has no feature"
            )
        end
    end
    return registry
end

local REGISTRY = build_registry()

---@return boolean valid
function research_features.validate_registry()
    for id, feature in pairs(REGISTRY) do
        assert_feature_invariants(feature, "research feature " .. id)
    end
    return true
end

---@param feature any
---@param label string?
---@return boolean?, string?
function research_features.validate_feature(feature, label)
    if type(feature) ~= "table" then
        return nil, (label or "research feature") .. " must be a table"
    end
    local ok, err = research_schema.validate(research_features.SHAPE, feature)
    if not ok then
        return nil, err
    end
    local invariant_ok, invariant_err =
        pcall(assert_feature_invariants, feature, label or ("research feature " .. feature.id))
    if not invariant_ok then
        return nil, tostring(invariant_err)
    end
    return true
end

---@return string[]
function research_features.ids()
    local ids = {}
    for id in pairs(REGISTRY) do
        ids[#ids + 1] = id
    end
    table.sort(ids)
    return ids
end

---@param id string
---@return ResearchFeatureData?, string?
function research_features.feature(id)
    if type(id) ~= "string" then
        return nil, "research feature id must be a string"
    end
    local feature = REGISTRY[id]
    if not feature then
        return nil, "unknown research feature " .. id
    end
    return research_schema.copy(feature)
end

-- Content hash of the whole register at a set of versions. Dataset manifests
-- store this so a silent register edit invalidates lineage.
---@return string
function research_features.registry_hash()
    local parts = {}
    for _, id in ipairs(research_features.ids()) do
        local feature = REGISTRY[id]
        parts[#parts + 1] = id
        parts[#parts + 1] = feature.version
        parts[#parts + 1] = assert(research_schema.content_hash(research_features.SHAPE, feature))
    end
    return research_schema.tuple_hash(research_features.REGISTRY_LABEL, parts)
end

-- Guard against pseudo-replication and prohibited use at the point of use.
---@param id string
---@param level string
---@return boolean?, string?
function research_features.aggregation_allowed(id, level)
    local feature, err = research_features.feature(id)
    if not feature then
        return nil, err
    end
    for _, allowed in ipairs(feature.aggregation_levels) do
        if allowed == level then
            return true
        end
    end
    return nil,
        "research feature " .. id .. " is not defined at aggregation level " .. tostring(level)
end

---@param id string
---@param use string
---@return boolean?, string?
function research_features.use_allowed(id, use)
    local feature, err = research_features.feature(id)
    if not feature then
        return nil, err
    end
    for _, prohibited in ipairs(feature.prohibited_uses) do
        if prohibited == use then
            return nil, "research feature " .. id .. " prohibits use " .. use
        end
    end
    return true
end

return research_features
