-- Experience-response contract: instrument register validation, response set
-- validation, and construct scoring that refuses to pool distinct constructs.
--
-- The register in `data/research_instruments.lua` is authored content, so a
-- broken register is a programmer error and asserts. Response sets arrive from
-- collection tooling, so every reader here returns `nil, err`.
--
-- Two rules exist to stop the silent-combination failure named in issue #132:
--   * an instrument whose aggregation is `item_only` may never carry a
--     construct score, so custom exploratory items and Affective Slider
--     readings cannot be averaged into anything; and
--   * a construct score is recomputed from the raw responses and compared, so a
--     hand-edited or partially-scored value fails closed instead of travelling
--     into a dataset.

local instruments = require("data.research_instruments")
local research_schema = require("sim.research_schema")

---@alias ResearchResponseTiming
---| "pre_block"
---| "post_block"
---| "natural_break"
---| "post_session"
---| "debrief"

---@class ResearchItemResponse
---@field item_id string
---@field raw_response number?
---@field missing_reason string?
---@field response_ms number?

---@class ResearchConstructScore
---@field construct string
---@field score number
---@field rule ResearchScoreAggregation
---@field item_count integer

---@class ResearchResponseSet
---@field schema_version integer
---@field manifest_kind "research_response_set"
---@field digest string
---@field response_set_id string
---@field session_id string
---@field participant_id string
---@field condition_id string
---@field instrument_id string
---@field instrument_version string
---@field scoring_key_version string
---@field analysis_role ResearchAnalysisRole
---@field validated_instrument boolean
---@field partial_administration boolean
---@field locale table
---@field timing table
---@field presentation_order string[]
---@field randomized_presentation boolean
---@field responses ResearchItemResponse[]
---@field scores ResearchConstructScore[]

---@class ResearchResponseModule
local research_response = {}

research_response.VERSION = 1
research_response.SUPPORTED_VERSIONS = { [1] = true }
research_response.KIND = "research_response_set"
research_response.SCALE_EPSILON = 1e-9

research_response.MISSING_REASONS = research_schema.enum({
    "participant_skipped",
    "participant_withdrew",
    "instrument_not_administered",
    "operator_error",
    "technical_failure",
    "time_limit",
})

local RESPONSE_FIELDS = {
    { name = "item_id", kind = "id" },
    { name = "raw_response", kind = "number", optional = true },
    {
        name = "missing_reason",
        kind = "enum",
        optional = true,
        values = research_response.MISSING_REASONS,
    },
    { name = "response_ms", kind = "number", optional = true, min = 0 },
}

local SCORE_FIELDS = {
    { name = "construct", kind = "id" },
    { name = "score", kind = "number" },
    {
        name = "rule",
        kind = "enum",
        values = research_schema.enum({ "mean_all_items", "item_only" }),
    },
    { name = "item_count", kind = "integer", min = 1 },
}

local LOCALE_FIELDS = {
    { name = "language_tag", kind = "id" },
    {
        name = "translation_provenance",
        kind = "enum",
        values = research_schema.enum({
            "original",
            "validated_translation",
            "project_adaptation",
        }),
    },
    { name = "translation_id", kind = "id", optional = true },
}

local TIMING_FIELDS = {
    {
        name = "relative_to_play",
        kind = "enum",
        values = research_schema.enum({
            "pre_block",
            "post_block",
            "natural_break",
            "post_session",
            "debrief",
        }),
    },
    { name = "offset_ms", kind = "number" },
    { name = "canonical_boundary_tick", kind = "integer", optional = true, min = 0 },
    { name = "trace_id", kind = "hash", optional = true },
}

research_response.SHAPE = research_schema.record("research_response_set/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_response.KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "response_set_id", kind = "id" },
    { name = "session_id", kind = "id", min_length = 16 },
    { name = "participant_id", kind = "id", min_length = 16 },
    { name = "condition_id", kind = "id" },
    { name = "instrument_id", kind = "id" },
    { name = "instrument_version", kind = "id" },
    { name = "scoring_key_version", kind = "id" },
    {
        name = "analysis_role",
        kind = "enum",
        values = research_schema.enum({ "primary", "diagnostic", "exploratory" }),
    },
    { name = "validated_instrument", kind = "boolean" },
    { name = "partial_administration", kind = "boolean" },
    { name = "locale", kind = "record", fields = LOCALE_FIELDS },
    { name = "timing", kind = "record", fields = TIMING_FIELDS },
    {
        name = "presentation_order",
        kind = "array",
        min_length = 1,
        max_length = 128,
        element = { name = "item_id", kind = "id" },
    },
    { name = "randomized_presentation", kind = "boolean" },
    {
        name = "responses",
        kind = "array",
        min_length = 1,
        max_length = 128,
        element = { name = "response", kind = "record", fields = RESPONSE_FIELDS },
    },
    {
        name = "scores",
        kind = "array",
        max_length = 32,
        element = { name = "score", kind = "record", fields = SCORE_FIELDS },
    },
})

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

-- Authored register validation. A broken register is a code/content bug.
---@param register table<string, ResearchInstrumentData>?
---@return boolean valid
function research_response.validate_register(register)
    register = register or instruments
    assert(type(register) == "table", "research instrument register is required")
    local count = 0
    for key, instrument in pairs(register) do
        local label = "research_instruments." .. tostring(key)
        assert(type(instrument) == "table", label .. " must be a table")
        assert(instrument.id == key, label .. " id must match its key")
        assert(
            type(instrument.name) == "string" and instrument.name ~= "",
            label .. " needs a name"
        )
        assert(
            type(instrument.source_register_key) == "string"
                and instrument.source_register_key ~= "",
            label .. " must name its source register key"
        )
        assert(
            instrument.item_text_included == false,
            label .. " must not embed instrument item text"
        )
        assert(
            type(instrument.license) == "string" and instrument.license ~= "",
            label .. " needs license terms"
        )
        assert(
            instrument.scale_kind == "likert" or instrument.scale_kind == "continuous",
            label .. " has an unknown scale kind"
        )
        assert(
            is_finite(instrument.scale_min)
                and is_finite(instrument.scale_max)
                and instrument.scale_min < instrument.scale_max,
            label .. " needs an ordered finite scale"
        )
        assert(
            is_finite(instrument.scale_step) and instrument.scale_step > 0,
            label .. " needs a positive scale step"
        )
        assert(
            instrument.score_aggregation == "mean_all_items"
                or instrument.score_aggregation == "item_only",
            label .. " has an unknown score aggregation"
        )
        assert(
            instrument.analysis_role == "primary"
                or instrument.analysis_role == "diagnostic"
                or instrument.analysis_role == "exploratory",
            label .. " has an unknown analysis role"
        )
        assert(
            not (instrument.analysis_role == "primary" and instrument.validated ~= true),
            label .. " cannot be a primary outcome without a validated instrument"
        )
        assert(
            instrument.score_aggregation ~= "mean_all_items"
                or instrument.requires_all_items == true,
            label .. " mean scoring requires the complete-item rule"
        )
        assert(
            type(instrument.pooling_group) == "string" and instrument.pooling_group ~= "",
            label .. " needs a pooling group"
        )
        assert(type(instrument.forbidden_pooling) == "table", label .. " needs forbidden pooling")
        if instrument.substitute_for ~= nil then
            local target = assert(
                register[instrument.substitute_for],
                label
                    .. " substitutes for unknown instrument "
                    .. tostring(instrument.substitute_for)
            )
            assert(
                target.validated and not instrument.validated,
                label
                    .. " may only substitute for a validated instrument it does not replace in kind"
            )
        end

        local constructs = {}
        for _, construct in ipairs(instrument.constructs) do
            assert(not constructs[construct], label .. " repeats construct " .. construct)
            constructs[construct] = 0
        end
        local positions = {}
        local item_ids = {}
        for index, item in ipairs(instrument.items) do
            local item_label = label .. ".items." .. index
            assert(type(item.id) == "string" and item.id ~= "", item_label .. " needs an id")
            assert(not item_ids[item.id], item_label .. " duplicates item id " .. item.id)
            item_ids[item.id] = true
            assert(
                constructs[item.construct] ~= nil,
                item_label .. " has unregistered construct " .. tostring(item.construct)
            )
            constructs[item.construct] = constructs[item.construct] + 1
            assert(
                type(item.position) == "number" and item.position == index,
                item_label .. " position must match canonical order"
            )
            assert(type(item.reverse_scored) == "boolean", item_label .. " needs reverse_scored")
            assert(not positions[item.position], item_label .. " duplicates a position")
            positions[item.position] = true
        end
        for construct, item_count in pairs(constructs) do
            assert(item_count > 0, label .. " construct " .. construct .. " has no items")
            if instrument.score_aggregation == "item_only" then
                assert(
                    item_count == 1,
                    label .. " item-only construct " .. construct .. " must be a single item"
                )
            end
        end
        count = count + 1
    end
    assert(count > 0, "research instrument register is empty")
    return true
end

---@param instrument_id string
---@return ResearchInstrumentData?, string?
function research_response.instrument(instrument_id)
    if type(instrument_id) ~= "string" then
        return nil, "instrument id must be a string"
    end
    local instrument = instruments[instrument_id]
    if not instrument then
        return nil, "unknown instrument " .. instrument_id
    end
    return instrument
end

---@param instrument ResearchInstrumentData
---@param value number
---@return boolean
local function on_scale(instrument, value)
    if value < instrument.scale_min - research_response.SCALE_EPSILON then
        return false
    end
    if value > instrument.scale_max + research_response.SCALE_EPSILON then
        return false
    end
    local steps = (value - instrument.scale_min) / instrument.scale_step
    return math.abs(steps - math.floor(steps + 0.5)) < 1e-6
end

-- Recompute construct scores from raw responses. Constructs with any missing
-- item get no score and are reported as incomplete; item-only instruments are
-- never aggregated.
---@param set ResearchResponseSet
---@return ResearchConstructScore[]?, string[]?, string?
function research_response.recompute_scores(set)
    if type(set) ~= "table" then
        return nil, nil, "research response set must be a table"
    end
    local instrument, instrument_err = research_response.instrument(set.instrument_id)
    if not instrument then
        return nil, nil, instrument_err
    end
    local construct_of = {}
    local expected_counts = {}
    for _, item in ipairs(instrument.items) do
        construct_of[item.id] = item.construct
        expected_counts[item.construct] = (expected_counts[item.construct] or 0) + 1
    end
    local sums = {}
    local answered = {}
    for index, response in ipairs(set.responses or {}) do
        local construct = construct_of[response.item_id]
        if not construct then
            return nil,
                nil,
                "research_response_set.responses." .. index .. " is not an instrument item"
        end
        if response.raw_response ~= nil then
            sums[construct] = (sums[construct] or 0) + response.raw_response
            answered[construct] = (answered[construct] or 0) + 1
        end
    end
    local scores = {}
    local incomplete = {}
    for _, construct in ipairs(instrument.constructs) do
        local count = answered[construct] or 0
        local expected = expected_counts[construct] or 0
        if instrument.score_aggregation == "item_only" then
            incomplete[#incomplete + 1] = construct
        elseif count == expected and expected > 0 then
            scores[#scores + 1] = {
                construct = construct,
                score = sums[construct] / count,
                rule = "mean_all_items",
                item_count = count,
            }
        else
            incomplete[#incomplete + 1] = construct
        end
    end
    table.sort(scores, function(left, right)
        return left.construct < right.construct
    end)
    table.sort(incomplete)
    return scores, incomplete
end

---@param set any
---@return boolean?, string?
function research_response.validate(set)
    if type(set) ~= "table" then
        return nil, "research response set must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        research_response.KIND,
        research_response.SUPPORTED_VERSIONS,
        research_response.VERSION,
        set.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_response.SHAPE, set)
    if not ok then
        return nil, err
    end
    ---@cast set ResearchResponseSet
    local instrument, instrument_err = research_response.instrument(set.instrument_id)
    if not instrument then
        return nil, instrument_err
    end
    -- Cross-version mismatches stop here rather than being reinterpreted.
    if set.instrument_version ~= instrument.instrument_version then
        return nil,
            "research_response_set.instrument_version "
                .. set.instrument_version
                .. " does not match registered "
                .. instrument.instrument_version
    end
    if set.scoring_key_version ~= instrument.scoring_key_version then
        return nil,
            "research_response_set.scoring_key_version "
                .. set.scoring_key_version
                .. " does not match registered "
                .. instrument.scoring_key_version
    end
    if set.analysis_role ~= instrument.analysis_role then
        return nil, "research_response_set.analysis_role contradicts the instrument register"
    end
    if set.validated_instrument ~= instrument.validated then
        return nil, "research_response_set.validated_instrument contradicts the register"
    end
    if set.partial_administration ~= instrument.partial_administration then
        return nil, "research_response_set.partial_administration contradicts the register"
    end
    if set.locale.translation_provenance ~= "original" and set.locale.translation_id == nil then
        return nil, "research_response_set.locale translations must name their provenance id"
    end
    if set.locale.translation_provenance == "original" and set.locale.translation_id ~= nil then
        return nil, "research_response_set.locale original administration has no translation id"
    end

    local registered_items = {}
    for _, item in ipairs(instrument.items) do
        registered_items[item.id] = true
    end
    local ordered = {}
    for index, item_id in ipairs(set.presentation_order) do
        if not registered_items[item_id] then
            return nil,
                "research_response_set.presentation_order." .. index .. " is not an instrument item"
        end
        if ordered[item_id] then
            return nil, "research_response_set.presentation_order." .. index .. " is duplicated"
        end
        ordered[item_id] = true
    end
    if #set.presentation_order ~= #instrument.items then
        return nil,
            "research_response_set.presentation_order must cover every administered instrument item"
    end

    local seen = {}
    for index, response in ipairs(set.responses) do
        local path = "research_response_set.responses." .. index
        if not registered_items[response.item_id] then
            return nil, path .. " is not an instrument item"
        end
        if seen[response.item_id] then
            return nil, path .. " duplicates item " .. response.item_id
        end
        seen[response.item_id] = true
        if response.raw_response == nil and response.missing_reason == nil then
            return nil, path .. " needs a raw response or a structured missingness reason"
        end
        if response.raw_response ~= nil and response.missing_reason ~= nil then
            return nil, path .. " cannot be both answered and missing"
        end
        if response.raw_response ~= nil and not on_scale(instrument, response.raw_response) then
            return nil, path .. " is off the registered response scale"
        end
    end
    if #set.responses ~= #instrument.items then
        return nil,
            "research_response_set.responses must carry one row per administered item, answered or missing"
    end

    local scores, incomplete, score_err = research_response.recompute_scores(set)
    if not scores then
        return nil, score_err
    end
    ---@cast incomplete string[]
    if instrument.score_aggregation == "item_only" and #set.scores > 0 then
        return nil,
            "research_response_set.scores is forbidden for item-only instrument " .. instrument.id
    end
    if #set.scores ~= #scores then
        return nil,
            "research_response_set.scores must contain exactly the scorable constructs ("
                .. tostring(#scores)
                .. " expected)"
    end
    local expected_by_construct = {}
    for _, score in ipairs(scores) do
        expected_by_construct[score.construct] = score
    end
    for index, score in ipairs(set.scores) do
        local expected = expected_by_construct[score.construct]
        if not expected then
            return nil,
                "research_response_set.scores."
                    .. index
                    .. " scores construct "
                    .. score.construct
                    .. " which is incomplete or not aggregatable"
        end
        if score.rule ~= expected.rule or score.item_count ~= expected.item_count then
            return nil, "research_response_set.scores." .. index .. " uses the wrong scoring rule"
        end
        if math.abs(score.score - expected.score) > 1e-9 then
            return nil,
                "research_response_set.scores." .. index .. " disagrees with its raw responses"
        end
    end
    for _, construct in ipairs(incomplete) do
        for _, score in ipairs(set.scores) do
            if score.construct == construct then
                return nil,
                    "research_response_set.scores must omit incomplete construct " .. construct
            end
        end
    end
    return true
end

-- Reject an administration set that pools instruments the register forbids
-- pooling, and reject an accessibility fallback presented alongside the
-- validated instrument it replaces.
---@param sets ResearchResponseSet[]
---@return boolean?, string?
function research_response.validate_administration(sets)
    if type(sets) ~= "table" then
        return nil, "research response sets must be an array"
    end
    local present = {}
    local ids = {}
    for index, set in ipairs(sets) do
        local ok, err = research_response.validate(set)
        if not ok then
            return nil, err
        end
        if ids[set.response_set_id] then
            return nil, "research response set " .. set.response_set_id .. " is duplicated"
        end
        ids[set.response_set_id] = true
        local key = set.instrument_id
            .. "/"
            .. set.condition_id
            .. "/"
            .. set.timing.relative_to_play
        if present[key] then
            return nil, "research response administration repeats " .. key
        end
        present[key] = { set = set, index = index }
    end
    -- A declared substitute replaces the instrument it stands in for; presenting
    -- both at one timepoint would invite pooling two non-equivalent measures.
    for key, entry in pairs(present) do
        local instrument = assert(research_response.instrument(entry.set.instrument_id))
        local substitute_for = instrument.substitute_for
        if substitute_for then
            local conflict = substitute_for
                .. "/"
                .. entry.set.condition_id
                .. "/"
                .. entry.set.timing.relative_to_play
            if present[conflict] then
                return nil,
                    "research response administration presents "
                        .. key
                        .. " together with its non-poolable substitute target "
                        .. conflict
            end
        end
    end
    return true
end

-- Orphan-join guard for the trace side. A response set that names a trace must
-- name one the caller actually holds. Mirrors
-- `research_trace.validate_against_stream`.
---@param set ResearchResponseSet
---@param manifest ResearchTraceManifest
---@return boolean?, string?
function research_response.validate_against_trace(set, manifest)
    local ok, err = research_response.validate(set)
    if not ok then
        return nil, err
    end
    if type(manifest) ~= "table" or type(manifest.trace_id) ~= "string" then
        return nil, "research trace manifest is required"
    end
    if set.timing.trace_id == nil then
        return nil, "research_response_set.timing.trace_id is required to join a trace"
    end
    if set.timing.trace_id ~= manifest.trace_id then
        return nil, "research_response_set.timing.trace_id is an orphan join"
    end
    if
        set.timing.canonical_boundary_tick ~= nil
        and set.timing.canonical_boundary_tick > manifest.simulation.last_boundary_tick
    then
        return nil, "research_response_set.timing.canonical_boundary_tick is past the trace"
    end
    return true
end

-- Orphan-join guard for the session side, plus the missingness contradiction:
-- a session that declares an instrument was not administered cannot also carry a
-- response set for that instrument.
---@param set ResearchResponseSet
---@param envelope ResearchSessionEnvelope
---@return boolean?, string?
function research_response.validate_against_session(set, envelope)
    local ok, err = research_response.validate(set)
    if not ok then
        return nil, err
    end
    if type(envelope) ~= "table" or type(envelope.session_id) ~= "string" then
        return nil, "research session envelope is required"
    end
    if set.session_id ~= envelope.session_id then
        return nil, "research_response_set.session_id is an orphan join"
    end
    if set.participant_id ~= envelope.participant_id then
        return nil, "research_response_set.participant_id disagrees with its session"
    end
    if envelope.lifecycle.status == "withdrawn" then
        return nil, "research_response_set belongs to a withdrawn session"
    end
    for index, missing in ipairs(envelope.lifecycle.missingness) do
        if
            missing.target_kind == "instrument"
            and missing.target_id == set.instrument_id
            and missing.reason_code == "instrument_not_administered"
        then
            return nil,
                "research_session_envelope.lifecycle.missingness."
                    .. index
                    .. " says "
                    .. set.instrument_id
                    .. " was not administered but a response set exists"
        end
    end
    return true
end

---@param set ResearchResponseSet
---@return string?, string?
function research_response.content_hash(set)
    local ok, err = research_response.validate(set)
    if not ok then
        return nil, err
    end
    return research_schema.content_hash(research_response.SHAPE, set)
end

---@param set ResearchResponseSet
---@return string?, string?
function research_response.encode(set)
    local ok, err = research_response.validate(set)
    if not ok then
        return nil, err
    end
    return research_schema.encode(research_response.SHAPE, set)
end

---@param bytes string
---@return ResearchResponseSet?, string?
function research_response.decode(bytes)
    local set, err = research_schema.decode(research_response.SHAPE, bytes)
    if not set then
        return nil, err
    end
    local ok, validate_err = research_response.validate(set)
    if not ok then
        return nil, validate_err
    end
    return set
end

return research_response
