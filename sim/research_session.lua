-- Versioned research session envelope and withdrawal tombstone.
--
-- The envelope is the *only* place participant, protocol, and condition
-- metadata lives. It links to gameplay traces by content-addressed id, so a
-- session can reference many traces and a trace can be referenced by many
-- sessions without either side rewriting the other.
--
-- Deliberate exclusions (see docs/design/player_evidence_contracts.md):
--   * no direct identifiers and no agreement prose — the envelope records which
--     agreement version the participant accepted and when, never its text; and
--   * no hand-entered experience label. Experience is stored as continuous and
--     ordinal source measures, and any "beginner/intermediate/experienced"
--     label must name the versioned calibration that derived it.

local research_schema = require("sim.research_schema")

---@alias ResearchSessionStatus
---| "in_progress"
---| "completed"
---| "interrupted"
---| "abandoned"
---| "withdrawn"
---| "excluded"

---@alias ResearchMissingnessReason
---| "participant_skipped"
---| "participant_withdrew"
---| "instrument_not_administered"
---| "operator_error"
---| "technical_failure"
---| "time_limit"

---@alias ResearchExclusionReason
---| "protocol_violation"
---| "technical_failure"
---| "practice_block"
---| "operator_intervention"
---| "participant_request"
---| "identity_mismatch"

-- The recorded agreement: which in-game disclaimer/agreement version the
-- participant accepted, when they accepted it, and whether that version covered
-- training and shipping a model. Permission cannot be granted retroactively, so
-- `model_use_covered` is a property of the accepted version, not of a later
-- decision.
---@class ResearchAgreement
---@field agreement_version string
---@field accepted boolean
---@field accepted_wall_clock_ms number
---@field model_use_covered boolean

---@class ResearchInterruption
---@field kind "pause"|"technical_failure"|"participant_break"|"operator_stop"|"process_exit"
---@field wall_clock_ms number
---@field duration_ms number
---@field canonical_tick integer?
---@field reason_code ResearchMissingnessReason?

---@class ResearchExperienceSource
---@field play_hours_per_week number
---@field years_playing number
---@field football_games_ordinal integer
---@field action_games_ordinal integer
---@field self_reported_familiarity_ordinal integer

---@class ResearchExperienceLabel
---@field label "beginner"|"intermediate"|"experienced"
---@field calibration_id string
---@field calibration_version integer

---@class ResearchSessionEnvelope
---@field schema_version integer
---@field manifest_kind "research_session_envelope"
---@field digest string
---@field session_id string
---@field participant_id string
---@field study_id string
---@field protocol_version string
---@field cohort_id string
---@field recruitment_channel string
---@field block_id string
---@field trial_id string
---@field condition_id string
---@field condition_order integer
---@field sequence_label string
---@field counterbalancing_cell string
---@field assignment table
---@field environment table
---@field lifecycle table
---@field experience table
---@field agreement ResearchAgreement
---@field trace_links table[]
---@field tombstone_id string?

---@class ResearchWithdrawalTombstone
---@field schema_version integer
---@field manifest_kind "research_withdrawal_tombstone"
---@field digest string
---@field tombstone_id string
---@field session_id string
---@field participant_id string
---@field request_wall_clock_ms number
---@field revoked_payload_hashes string[]
---@field rebuild_required boolean

---@class ResearchSessionModule
local research_session = {}

research_session.VERSION = 1
research_session.SUPPORTED_VERSIONS = { [1] = true }
research_session.KIND = "research_session_envelope"
research_session.TOMBSTONE_KIND = "research_withdrawal_tombstone"

-- What this validator actually enforces for participant and session ids: the
-- bounded `id` charset, this minimum length, uniqueness inside a payload, and
-- that a session id differs from its participant id. It does **not** measure
-- entropy or guessability — generating unguessable ids is the recorder's job.
-- These ids are join keys, not secrets.
research_session.MIN_PSEUDONYM_LENGTH = 16

research_session.STATUSES = research_schema.enum({
    "in_progress",
    "completed",
    "interrupted",
    "abandoned",
    "withdrawn",
    "excluded",
})

research_session.MISSINGNESS_REASONS = research_schema.enum({
    "participant_skipped",
    "participant_withdrew",
    "instrument_not_administered",
    "operator_error",
    "technical_failure",
    "time_limit",
})

research_session.EXCLUSION_REASONS = research_schema.enum({
    "protocol_violation",
    "technical_failure",
    "practice_block",
    "operator_intervention",
    "participant_request",
    "identity_mismatch",
})

local PSEUDONYM_FIELD = {
    kind = "id",
    min_length = research_session.MIN_PSEUDONYM_LENGTH,
}

local ASSIGNMENT_FIELDS = {
    { name = "build_id", kind = "id" },
    { name = "tuning_config_id", kind = "id" },
    { name = "seed", kind = "integer" },
    { name = "side", kind = "enum", values = research_schema.enum({ "home", "away" }) },
    { name = "role_id", kind = "id" },
    { name = "loadout_id", kind = "id", optional = true },
    {
        name = "opponent_population",
        kind = "enum",
        values = research_schema.enum({ "bot", "human_proxy", "human", "adversarial_policy" }),
    },
    { name = "opponent_policy_id", kind = "id" },
    { name = "practice_block", kind = "boolean" },
}

local ENVIRONMENT_FIELDS = {
    {
        name = "device_class",
        kind = "enum",
        values = research_schema.enum({ "desktop", "laptop", "browser", "handheld" }),
    },
    {
        name = "control_device",
        kind = "enum",
        values = research_schema.enum({ "keyboard", "gamepad", "touch", "mixed" }),
    },
    { name = "control_mapping_id", kind = "id" },
    { name = "viewport_w", kind = "integer", min = 1 },
    { name = "viewport_h", kind = "integer", min = 1 },
    { name = "render_hz", kind = "number", min = 1, max = 1000 },
    { name = "input_latency_ms", kind = "number", min = 0, max = 1000 },
    { name = "language_tag", kind = "id" },
    -- Readability/accessibility settings are recorded because they change what a
    -- participant could see, which is a validity concern for cue readability.
    {
        name = "readability_settings",
        kind = "map",
        optional = true,
        element = { name = "setting", kind = "string" },
    },
}

local INTERRUPTION_FIELDS = {
    {
        name = "kind",
        kind = "enum",
        values = research_schema.enum({
            "pause",
            "technical_failure",
            "participant_break",
            "operator_stop",
            "process_exit",
        }),
    },
    { name = "wall_clock_ms", kind = "number", min = 0 },
    { name = "duration_ms", kind = "number", min = 0 },
    { name = "canonical_tick", kind = "integer", optional = true, min = 0 },
    {
        name = "reason_code",
        kind = "enum",
        optional = true,
        values = research_session.MISSINGNESS_REASONS,
    },
}

local MISSINGNESS_FIELDS = {
    { name = "target_id", kind = "id" },
    {
        name = "target_kind",
        kind = "enum",
        values = research_schema.enum({ "instrument", "item", "block", "trace", "annotation" }),
    },
    { name = "reason_code", kind = "enum", values = research_session.MISSINGNESS_REASONS },
}

local EXCLUSION_FIELDS = {
    { name = "exclusion_id", kind = "id" },
    {
        name = "scope",
        kind = "enum",
        values = research_schema.enum({ "session", "block", "trial", "trace", "instrument" }),
    },
    { name = "reason_code", kind = "enum", values = research_session.EXCLUSION_REASONS },
    { name = "preregistered", kind = "boolean" },
}

local LIFECYCLE_FIELDS = {
    { name = "status", kind = "enum", values = research_session.STATUSES },
    { name = "started_wall_clock_ms", kind = "number", min = 0 },
    { name = "ended_wall_clock_ms", kind = "number", optional = true, min = 0 },
    {
        name = "observer_mode",
        kind = "enum",
        values = research_schema.enum({ "none", "same_room", "remote", "recorded" }),
    },
    {
        name = "assistance",
        kind = "enum",
        values = research_schema.enum({ "none", "verbal_prompt", "operator_input", "restart" }),
    },
    {
        name = "interruptions",
        kind = "array",
        max_length = 256,
        element = { name = "interruption", kind = "record", fields = INTERRUPTION_FIELDS },
    },
    {
        name = "exclusions",
        kind = "array",
        max_length = 256,
        element = { name = "exclusion", kind = "record", fields = EXCLUSION_FIELDS },
    },
    {
        name = "missingness",
        kind = "array",
        max_length = 1024,
        element = { name = "missing", kind = "record", fields = MISSINGNESS_FIELDS },
    },
}

local EXPERIENCE_FIELDS = {
    { name = "play_hours_per_week", kind = "number", min = 0, max = 168 },
    { name = "years_playing", kind = "number", min = 0, max = 100 },
    { name = "football_games_ordinal", kind = "integer", min = 1, max = 5 },
    { name = "action_games_ordinal", kind = "integer", min = 1, max = 5 },
    { name = "self_reported_familiarity_ordinal", kind = "integer", min = 1, max = 5 },
    {
        name = "derived_label",
        kind = "record",
        optional = true,
        fields = {
            {
                name = "label",
                kind = "enum",
                values = research_schema.enum({ "beginner", "intermediate", "experienced" }),
            },
            { name = "calibration_id", kind = "id" },
            { name = "calibration_version", kind = "integer", min = 1 },
        },
    },
}

local AGREEMENT_FIELDS = {
    { name = "agreement_version", kind = "id" },
    { name = "accepted", kind = "boolean" },
    { name = "accepted_wall_clock_ms", kind = "number", min = 0 },
    { name = "model_use_covered", kind = "boolean" },
}

local TRACE_LINK_FIELDS = {
    { name = "trace_id", kind = "hash" },
    { name = "game_instance_id", kind = "id" },
    {
        name = "role",
        kind = "enum",
        values = research_schema.enum({ "condition_block", "practice", "replay_probe" }),
    },
}

research_session.SHAPE = research_schema.record("research_session_envelope/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_session.KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "session_id", kind = PSEUDONYM_FIELD.kind, min_length = PSEUDONYM_FIELD.min_length },
    {
        name = "participant_id",
        kind = PSEUDONYM_FIELD.kind,
        min_length = PSEUDONYM_FIELD.min_length,
    },
    { name = "study_id", kind = "id" },
    { name = "protocol_version", kind = "id" },
    { name = "cohort_id", kind = "id" },
    { name = "recruitment_channel", kind = "id" },
    { name = "block_id", kind = "id" },
    { name = "trial_id", kind = "id" },
    { name = "condition_id", kind = "id" },
    { name = "condition_order", kind = "integer", min = 1, max = 32 },
    { name = "sequence_label", kind = "id" },
    { name = "counterbalancing_cell", kind = "id" },
    { name = "assignment", kind = "record", fields = ASSIGNMENT_FIELDS },
    { name = "environment", kind = "record", fields = ENVIRONMENT_FIELDS },
    { name = "lifecycle", kind = "record", fields = LIFECYCLE_FIELDS },
    { name = "experience", kind = "record", fields = EXPERIENCE_FIELDS },
    { name = "agreement", kind = "record", fields = AGREEMENT_FIELDS },
    {
        name = "trace_links",
        kind = "array",
        max_length = 256,
        element = { name = "trace_link", kind = "record", fields = TRACE_LINK_FIELDS },
    },
    { name = "tombstone_id", kind = "id", optional = true },
})

research_session.TOMBSTONE_SHAPE = research_schema.record("research_withdrawal_tombstone/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_session.TOMBSTONE_KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "tombstone_id", kind = "id" },
    { name = "session_id", kind = "id", min_length = research_session.MIN_PSEUDONYM_LENGTH },
    { name = "participant_id", kind = "id", min_length = research_session.MIN_PSEUDONYM_LENGTH },
    { name = "request_wall_clock_ms", kind = "number", min = 0 },
    -- There is exactly one withdrawal path: delete the session and regenerate
    -- anything derived from it. Partial scopes were removed because nothing
    -- enforced them, and an unenforced promise is worse than no promise.
    {
        name = "revoked_payload_hashes",
        kind = "array",
        max_length = 4096,
        element = { name = "payload_hash", kind = "hash" },
    },
    { name = "rebuild_required", kind = "boolean" },
})

---@param envelope any
---@return boolean?, string?
function research_session.validate(envelope)
    if type(envelope) ~= "table" then
        return nil, "research session envelope must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        research_session.KIND,
        research_session.SUPPORTED_VERSIONS,
        research_session.VERSION,
        envelope.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_session.SHAPE, envelope)
    if not ok then
        return nil, err
    end
    ---@cast envelope ResearchSessionEnvelope
    local lifecycle = envelope.lifecycle

    -- Recording only happens after the participant accepts the agreement, so an
    -- envelope that was not accepted, or was accepted after recording started,
    -- is a contract violation rather than a preference to honour later.
    local agreement = envelope.agreement
    if not agreement.accepted then
        return nil, "research_session_envelope.agreement must be accepted before recording"
    end
    if agreement.accepted_wall_clock_ms > lifecycle.started_wall_clock_ms then
        return nil, "research_session_envelope.agreement was accepted after the session started"
    end

    if lifecycle.ended_wall_clock_ms ~= nil then
        if lifecycle.ended_wall_clock_ms < lifecycle.started_wall_clock_ms then
            return nil, "research_session_envelope.lifecycle ends before it starts"
        end
    elseif lifecycle.status ~= "in_progress" then
        return nil,
            "research_session_envelope.lifecycle."
                .. lifecycle.status
                .. " requires an end timestamp"
    end

    if lifecycle.status == "completed" then
        for index, interruption in ipairs(lifecycle.interruptions) do
            if interruption.kind == "process_exit" or interruption.kind == "operator_stop" then
                return nil,
                    "research_session_envelope.lifecycle.interruptions."
                        .. index
                        .. " contradicts a completed session"
            end
        end
    elseif lifecycle.status == "interrupted" and #lifecycle.interruptions == 0 then
        return nil, "research_session_envelope.lifecycle interrupted sessions must record why"
    elseif lifecycle.status == "excluded" and #lifecycle.exclusions == 0 then
        return nil, "research_session_envelope.lifecycle excluded sessions must record why"
    end

    if lifecycle.status == "withdrawn" then
        if envelope.tombstone_id == nil then
            return nil, "research_session_envelope withdrawn sessions require a tombstone_id"
        end
        if #envelope.trace_links > 0 then
            return nil, "research_session_envelope withdrawn sessions must not retain payload links"
        end
        local withdrawal_recorded = false
        for _, missing in ipairs(lifecycle.missingness) do
            if missing.reason_code == "participant_withdrew" then
                withdrawal_recorded = true
            end
        end
        if not withdrawal_recorded then
            return nil,
                "research_session_envelope withdrawn sessions must record participant_withdrew missingness"
        end
    elseif envelope.tombstone_id ~= nil then
        return nil, "research_session_envelope.tombstone_id requires a withdrawn session"
    end

    local seen_traces = {}
    for index, link in ipairs(envelope.trace_links) do
        local key = link.trace_id .. "/" .. link.game_instance_id
        if seen_traces[key] then
            return nil, "research_session_envelope.trace_links." .. index .. " is duplicated"
        end
        seen_traces[key] = true
        if link.role == "practice" and not envelope.assignment.practice_block then
            return nil,
                "research_session_envelope.trace_links."
                    .. index
                    .. " is a practice trace outside a practice block"
        end
    end

    local seen_missing = {}
    for index, missing in ipairs(lifecycle.missingness) do
        local key = missing.target_kind .. "/" .. missing.target_id
        if seen_missing[key] then
            return nil,
                "research_session_envelope.lifecycle.missingness." .. index .. " repeats " .. key
        end
        seen_missing[key] = true
    end

    local seen_exclusions = {}
    for index, exclusion in ipairs(lifecycle.exclusions) do
        if seen_exclusions[exclusion.exclusion_id] then
            return nil,
                "research_session_envelope.lifecycle.exclusions." .. index .. " is duplicated"
        end
        seen_exclusions[exclusion.exclusion_id] = true
    end

    if envelope.session_id == envelope.participant_id then
        return nil, "research_session_envelope.session_id must differ from participant_id"
    end
    return true
end

---@param tombstone any
---@return boolean?, string?
function research_session.validate_tombstone(tombstone)
    if type(tombstone) ~= "table" then
        return nil, "research withdrawal tombstone must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        research_session.TOMBSTONE_KIND,
        research_session.SUPPORTED_VERSIONS,
        research_session.VERSION,
        tombstone.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_session.TOMBSTONE_SHAPE, tombstone)
    if not ok then
        return nil, err
    end
    ---@cast tombstone ResearchWithdrawalTombstone
    local seen = {}
    for index, hash in ipairs(tombstone.revoked_payload_hashes) do
        if seen[hash] then
            return nil,
                "research_withdrawal_tombstone.revoked_payload_hashes." .. index .. " is duplicated"
        end
        seen[hash] = true
    end
    if #tombstone.revoked_payload_hashes == 0 then
        return nil, "research_withdrawal_tombstone must name the payloads it revokes"
    end
    if not tombstone.rebuild_required then
        return nil,
            "research_withdrawal_tombstone must require derived rebuilds; derived tables are never hand-patched"
    end
    return true
end

-- A tombstoned session and its tombstone must agree, and the tombstone must
-- cover every payload the session still names.
---@param envelope ResearchSessionEnvelope
---@param tombstone ResearchWithdrawalTombstone
---@return boolean?, string?
function research_session.validate_withdrawal(envelope, tombstone)
    local ok, err = research_session.validate(envelope)
    if not ok then
        return nil, err
    end
    local tombstone_ok, tombstone_err = research_session.validate_tombstone(tombstone)
    if not tombstone_ok then
        return nil, tombstone_err
    end
    if envelope.lifecycle.status ~= "withdrawn" then
        return nil, "research withdrawal requires a withdrawn session envelope"
    end
    if envelope.tombstone_id ~= tombstone.tombstone_id then
        return nil, "research withdrawal tombstone id does not match the session envelope"
    end
    if envelope.session_id ~= tombstone.session_id then
        return nil, "research withdrawal tombstone session id does not match the envelope"
    end
    if envelope.participant_id ~= tombstone.participant_id then
        return nil, "research withdrawal tombstone participant id does not match the envelope"
    end
    return true
end

---@param envelope ResearchSessionEnvelope
---@return string?, string?
function research_session.content_hash(envelope)
    local ok, err = research_session.validate(envelope)
    if not ok then
        return nil, err
    end
    return research_schema.content_hash(research_session.SHAPE, envelope)
end

---@param envelope ResearchSessionEnvelope
---@return string?, string?
function research_session.encode(envelope)
    local ok, err = research_session.validate(envelope)
    if not ok then
        return nil, err
    end
    return research_schema.encode(research_session.SHAPE, envelope)
end

---@param bytes string
---@return ResearchSessionEnvelope?, string?
function research_session.decode(bytes)
    local envelope, err = research_schema.decode(research_session.SHAPE, bytes)
    if not envelope then
        return nil, err
    end
    local ok, validate_err = research_session.validate(envelope)
    if not ok then
        return nil, validate_err
    end
    return envelope
end

-- A model trained on this session may ship or be shared only if the agreement
-- version the participant accepted covered that use. Permission is never granted
-- retroactively, so this reads the recorded acceptance and nothing else.
---@param envelope ResearchSessionEnvelope
---@return boolean?, string?
function research_session.allows_model_use(envelope)
    local ok, err = research_session.validate(envelope)
    if not ok then
        return nil, err
    end
    if not envelope.agreement.model_use_covered then
        return nil,
            "agreement version "
                .. envelope.agreement.agreement_version
                .. " did not cover training or shipping a model"
    end
    return true
end

-- Orphan-join guard for the session side. Every trace link must resolve to a
-- manifest the caller actually holds, and the resolved manifest must describe
-- the same run the envelope claims. Mirrors
-- `research_trace.validate_against_stream`.
---@param envelope ResearchSessionEnvelope
---@param manifests ResearchTraceManifest[]
---@return boolean?, string?
function research_session.validate_against_traces(envelope, manifests)
    local ok, err = research_session.validate(envelope)
    if not ok then
        return nil, err
    end
    if type(manifests) ~= "table" then
        return nil, "research trace manifests must be an array"
    end
    local by_key = {}
    for index, manifest in ipairs(manifests) do
        if type(manifest) ~= "table" or type(manifest.trace_id) ~= "string" then
            return nil, "research trace manifest " .. index .. " is malformed"
        end
        local key = manifest.trace_id .. "/" .. tostring(manifest.game_instance_id)
        if by_key[key] then
            return nil, "research trace manifest " .. key .. " is supplied twice"
        end
        by_key[key] = manifest
    end
    for index, link in ipairs(envelope.trace_links) do
        local path = "research_session_envelope.trace_links." .. index
        local manifest = by_key[link.trace_id .. "/" .. link.game_instance_id]
        if not manifest then
            return nil, path .. " is an orphan join"
        end
        if manifest.simulation.seed ~= envelope.assignment.seed then
            return nil, path .. " resolves to a trace with another seed"
        end
    end
    return true
end

return research_session
