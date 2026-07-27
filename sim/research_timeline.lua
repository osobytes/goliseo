-- Canonical confirmed-event stream and annotation timeline for research
-- exports.
--
-- Rollback makes "the events that happened" a function of confirmation, not of
-- presentation. `sim/rollback_events.lua` already separates a speculative
-- window from confirmed steps and reports revocations; this module is the
-- research-facing projection of that fact:
--
--   * only events delivered by `rollback_events.confirm` become export rows;
--   * an event id that a correction revoked can never appear in an export, and
--     a revocation of an already-confirmed id fails closed; and
--   * render-time and wall-clock cues are mapped onto the nearest canonical
--     boundary with the mapping error retained, never silently snapped.

local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local research_schema = require("sim.research_schema")

---@alias ResearchEventDomain "input"|"soccer"|"combat"|"lifecycle"
---@alias ResearchBoundarySource "canonical_tick"|"wall_clock_mapped"|"render_frame_mapped"
---@alias ResearchAnnotationAuthor "participant"|"researcher"|"tool"

---@class ResearchEventRow
---@field canonical_tick integer
---@field domain ResearchEventDomain
---@field domain_rank integer
---@field event_kind string
---@field source_sequence integer -- 0 when the source has no sequence
---@field same_kind_ordinal integer
---@field event_id string
---@field rollback_event_id string
---@field payload_hash string

---@class ResearchEventStream
---@field schema_version integer
---@field manifest_kind "research_event_stream"
---@field digest string
---@field run_scope_id string
---@field game_instance_id string
---@field confirmed_through_tick integer
---@field confirmed_boundary integer
---@field rows ResearchEventRow[]
---@field stream_hash string

---@class ResearchRevocation
---@field rollback_event_id string
---@field canonical_tick integer
---@field reason "rollback_revoked"

---@class ResearchTimeline
---@field run_scope_id string
---@field game_instance_id string
---@field _confirmed_tick integer
---@field _confirmed_boundary integer
---@field _rows ResearchEventRow[]
---@field _confirmed table<string, boolean>
---@field _speculative table<string, boolean>
---@field _revoked table<string, ResearchRevocation>

---@class ResearchBoundaryClock
---@field origin_wall_clock_ms number
---@field tick_rate integer
---@field first_boundary_tick integer
---@field last_boundary_tick integer
---@field non_simulated_ms number -- pause, goal replay, and menu wall time before the cue

---@class ResearchBoundaryMapping
---@field canonical_tick integer
---@field mapping_error_ms number -- signed cue minus boundary, in wall-clock ms
---@field boundary_source ResearchBoundarySource

---@class ResearchTimelineModule
local research_timeline = {}

research_timeline.VERSION = 1
research_timeline.SUPPORTED_VERSIONS = { [1] = true }
research_timeline.STREAM_KIND = "research_event_stream"
research_timeline.ANNOTATION_KIND = "research_annotation_set"
research_timeline.STREAM_LABEL = "confirmed-event-stream/v1"
research_timeline.EVENT_LABEL = "research-event/v1"

-- Closed schema enum. Ranks are part of the canonical sort key, so they may
-- never be renumbered without a serialization version bump.
research_timeline.DOMAIN_RANKS = {
    input = 1,
    soccer = 2,
    combat = 3,
    lifecycle = 4,
}

research_timeline.DOMAINS = research_schema.enum({ "input", "soccer", "combat", "lifecycle" })

local ROW_FIELDS = {
    { name = "canonical_tick", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    { name = "domain", kind = "enum", values = research_timeline.DOMAINS },
    { name = "domain_rank", kind = "integer", min = 1, max = 4 },
    { name = "event_kind", kind = "id" },
    { name = "source_sequence", kind = "integer", min = 0 },
    { name = "same_kind_ordinal", kind = "integer", min = 1 },
    { name = "event_id", kind = "hash" },
    { name = "rollback_event_id", kind = "string" },
    { name = "payload_hash", kind = "hash" },
}

research_timeline.ROWS_SHAPE = research_schema.record("research_event_rows/v1", {
    { name = "run_scope_id", kind = "hash" },
    { name = "game_instance_id", kind = "id" },
    { name = "confirmed_through_tick", kind = "integer", min = -1, max = input_frame.MAX_TICK },
    { name = "confirmed_boundary", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    {
        name = "rows",
        kind = "array",
        element = { name = "row", kind = "record", fields = ROW_FIELDS },
    },
})

research_timeline.STREAM_SHAPE = research_schema.record("research_event_stream/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_timeline.STREAM_KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "run_scope_id", kind = "hash" },
    { name = "game_instance_id", kind = "id" },
    { name = "confirmed_through_tick", kind = "integer", min = -1, max = input_frame.MAX_TICK },
    { name = "confirmed_boundary", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    {
        name = "rows",
        kind = "array",
        element = { name = "row", kind = "record", fields = ROW_FIELDS },
    },
    { name = "stream_hash", kind = "hash" },
})

local ANNOTATION_FIELDS = {
    { name = "annotation_id", kind = "id" },
    { name = "canonical_tick", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    {
        name = "boundary_source",
        kind = "enum",
        values = research_schema.enum({
            "canonical_tick",
            "wall_clock_mapped",
            "render_frame_mapped",
        }),
    },
    { name = "mapping_error_ms", kind = "number", min = -1000, max = 1000 },
    { name = "wall_clock_ms", kind = "number", optional = true, min = 0 },
    { name = "event_id", kind = "hash", optional = true },
    { name = "code_id", kind = "id" },
    { name = "confidence", kind = "number", optional = true, min = 0, max = 1 },
    { name = "disagreement_group", kind = "id", optional = true },
    { name = "free_text", kind = "text", optional = true },
}

research_timeline.ANNOTATION_SET_SHAPE = research_schema.record("research_annotation_set/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_timeline.ANNOTATION_KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "annotation_set_id", kind = "id" },
    { name = "run_scope_id", kind = "hash" },
    { name = "session_id", kind = "id" },
    {
        name = "author_role",
        kind = "enum",
        values = research_schema.enum({ "participant", "researcher", "tool" }),
    },
    -- Which in-game agreement version the participant accepted for this session.
    { name = "agreement_version", kind = "id" },
    { name = "coding_scheme_version", kind = "id" },
    {
        name = "annotations",
        kind = "array",
        element = { name = "annotation", kind = "record", fields = ANNOTATION_FIELDS },
    },
})

---@param domain string
---@return ResearchEventDomain?, string?, integer?
local function split_domain(domain)
    local group, remainder = domain:match("^([a-z_]+)/(.+)$")
    if not group then
        return nil, "rollback event domain " .. domain .. " is not a research domain"
    end
    if group == "match" then
        group = "soccer"
    end
    if not research_timeline.DOMAIN_RANKS[group] then
        return nil, "rollback event domain group " .. group .. " is unknown to this reader"
    end
    local kind, sequence = remainder:match("^([a-z_]+)/(%-?%d+)$")
    if kind then
        local parsed = tonumber(sequence)
        if not parsed then
            return nil, "rollback event domain " .. domain .. " has a malformed source sequence"
        end
        return group, kind, math.floor(parsed)
    end
    if not remainder:match("^[a-z_]+$") then
        return nil, "rollback event domain " .. domain .. " has a malformed kind"
    end
    return group, remainder, 0
end

---@param timeline ResearchTimeline
---@param event RollbackWrappedEvent
---@return ResearchEventRow?, string?
local function make_row(timeline, event)
    local group, kind, sequence = split_domain(event.domain)
    if not group then
        return nil, kind
    end
    ---@cast kind string
    ---@cast sequence integer
    if sequence < 0 then
        return nil, "rollback event " .. event.id .. " has a negative source sequence"
    end
    local payload_shape_hash = research_schema.tuple_hash("research-event-payload/v1", {
        event.id,
        event.domain,
        event.tick,
        event.ordinal,
    })
    local event_id = research_schema.tuple_hash(research_timeline.EVENT_LABEL, {
        timeline.run_scope_id,
        timeline.game_instance_id,
        event.tick,
        group,
        kind,
        sequence,
        event.ordinal,
    })
    return {
        canonical_tick = event.tick,
        domain = group,
        domain_rank = research_timeline.DOMAIN_RANKS[group],
        event_kind = kind,
        source_sequence = sequence,
        same_kind_ordinal = event.ordinal,
        event_id = event_id,
        rollback_event_id = event.id,
        payload_hash = payload_shape_hash,
    }
end

---@param run_scope_id string
---@param game_instance_id string
---@return ResearchTimeline?, string?
function research_timeline.new(run_scope_id, game_instance_id)
    local ok, err = research_schema.validate(
        research_schema.record("research_timeline_identity/v1", {
            { name = "run_scope_id", kind = "hash" },
            { name = "game_instance_id", kind = "id" },
        }),
        { run_scope_id = run_scope_id, game_instance_id = game_instance_id }
    )
    if not ok then
        return nil, err
    end
    return {
        run_scope_id = run_scope_id,
        game_instance_id = game_instance_id,
        _confirmed_tick = -1,
        _confirmed_boundary = 0,
        _rows = {},
        _confirmed = {},
        _speculative = {},
        _revoked = {},
    }
end

-- Record what a rollback correction did to the speculative window. Revoked
-- events are remembered so they can never be exported, and revoking an
-- already-confirmed event id is a contract violation rather than a correction.
---@param timeline ResearchTimeline
---@param diff RollbackEventDiff
---@return boolean?, string?
function research_timeline.observe_diff(timeline, diff)
    if type(timeline) ~= "table" or type(diff) ~= "table" then
        return nil, "research timeline and rollback diff are required"
    end
    for _, event in ipairs(diff.added or {}) do
        if timeline._confirmed[event.id] then
            return nil, "rollback re-added already-confirmed event " .. event.id
        end
        timeline._speculative[event.id] = true
        timeline._revoked[event.id] = nil
    end
    for _, replacement in ipairs(diff.replaced or {}) do
        local id = replacement.after.id
        if timeline._confirmed[id] then
            return nil, "rollback replaced already-confirmed event " .. id
        end
        timeline._speculative[id] = true
        timeline._revoked[id] = nil
    end
    for _, event in ipairs(diff.revoked or {}) do
        if timeline._confirmed[event.id] then
            return nil, "rollback revoked already-confirmed event " .. event.id
        end
        timeline._speculative[event.id] = nil
        timeline._revoked[event.id] = {
            rollback_event_id = event.id,
            canonical_tick = event.tick,
            reason = "rollback_revoked",
        }
    end
    return true
end

-- Promote confirmed rollback steps into export rows.
---@param timeline ResearchTimeline
---@param steps RollbackEventStep[]
---@return integer?, string?
function research_timeline.confirm(timeline, steps)
    if type(timeline) ~= "table" or type(steps) ~= "table" then
        return nil, "research timeline and confirmed steps are required"
    end
    local added = 0
    for index, step in ipairs(steps) do
        if step.tick ~= timeline._confirmed_tick + 1 then
            return nil, "confirmed step " .. index .. " is noncontiguous with the research timeline"
        end
        if step.start_boundary ~= timeline._confirmed_boundary then
            return nil, "confirmed step " .. index .. " does not start at the confirmed boundary"
        end
        local groups = { step.match_events, step.combat_events or {}, step.lifecycle_events }
        for _, events in ipairs(groups) do
            for _, event in ipairs(events) do
                if timeline._revoked[event.id] then
                    return nil, "confirmed step contains revoked event " .. event.id
                end
                if timeline._confirmed[event.id] then
                    return nil, "confirmed step repeats event " .. event.id
                end
                local row, row_err = make_row(timeline, event)
                if not row then
                    return nil, row_err
                end
                timeline._confirmed[event.id] = true
                timeline._speculative[event.id] = nil
                timeline._rows[#timeline._rows + 1] = row
                added = added + 1
            end
        end
        timeline._confirmed_tick = step.tick
        timeline._confirmed_boundary = step.end_boundary
    end
    return added
end

---@param left ResearchEventRow
---@param right ResearchEventRow
---@return boolean
local function row_precedes(left, right)
    if left.canonical_tick ~= right.canonical_tick then
        return left.canonical_tick < right.canonical_tick
    end
    if left.domain_rank ~= right.domain_rank then
        return left.domain_rank < right.domain_rank
    end
    if left.event_kind ~= right.event_kind then
        return left.event_kind < right.event_kind
    end
    if left.source_sequence ~= right.source_sequence then
        return left.source_sequence < right.source_sequence
    end
    if left.same_kind_ordinal ~= right.same_kind_ordinal then
        return left.same_kind_ordinal < right.same_kind_ordinal
    end
    return left.event_id < right.event_id
end

---@param stream ResearchEventStream
---@return string?, string?
function research_timeline.stream_hash(stream)
    return research_schema.content_hash(research_timeline.ROWS_SHAPE, {
        run_scope_id = stream.run_scope_id,
        game_instance_id = stream.game_instance_id,
        confirmed_through_tick = stream.confirmed_through_tick,
        confirmed_boundary = stream.confirmed_boundary,
        rows = stream.rows,
    })
end

-- Materialize the canonical confirmed stream. Fails closed if any revoked or
-- still-speculative event reached the rows, or if two rows share a total key.
---@param timeline ResearchTimeline
---@return ResearchEventStream?, string?
function research_timeline.export(timeline)
    if type(timeline) ~= "table" then
        return nil, "research timeline is required"
    end
    local rows = {}
    for index, row in ipairs(timeline._rows) do
        if timeline._revoked[row.rollback_event_id] then
            return nil, "research export contains revoked event " .. row.rollback_event_id
        end
        if timeline._speculative[row.rollback_event_id] then
            return nil, "research export contains speculative event " .. row.rollback_event_id
        end
        rows[index] = research_schema.copy(row)
    end
    table.sort(rows, row_precedes)
    local seen = {}
    for _, row in ipairs(rows) do
        local key = table.concat({
            tostring(row.canonical_tick),
            tostring(row.domain_rank),
            row.event_kind,
            tostring(row.source_sequence),
            tostring(row.same_kind_ordinal),
        }, "/")
        if seen[key] then
            return nil, "research export has a duplicate total key " .. key
        end
        seen[key] = true
    end
    local stream = {
        schema_version = research_timeline.VERSION,
        manifest_kind = research_timeline.STREAM_KIND,
        digest = research_schema.DIGEST,
        run_scope_id = timeline.run_scope_id,
        game_instance_id = timeline.game_instance_id,
        confirmed_through_tick = timeline._confirmed_tick,
        confirmed_boundary = timeline._confirmed_boundary,
        rows = rows,
        stream_hash = "0000000000000000",
    }
    local hash, hash_err = research_timeline.stream_hash(stream)
    if not hash then
        return nil, hash_err
    end
    stream.stream_hash = hash
    local ok, err = research_timeline.validate_stream(stream)
    if not ok then
        return nil, err
    end
    return stream
end

---@param stream any
---@return boolean?, string?
function research_timeline.validate_stream(stream)
    if type(stream) ~= "table" then
        return nil, "research event stream must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        research_timeline.STREAM_KIND,
        research_timeline.SUPPORTED_VERSIONS,
        research_timeline.VERSION,
        stream.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_timeline.STREAM_SHAPE, stream)
    if not ok then
        return nil, err
    end
    ---@cast stream ResearchEventStream
    if stream.confirmed_boundary ~= stream.confirmed_through_tick + 1 then
        return nil, "research_event_stream confirmed boundary must follow its confirmed tick"
    end
    local previous = nil
    for index, row in ipairs(stream.rows) do
        if row.domain_rank ~= research_timeline.DOMAIN_RANKS[row.domain] then
            return nil, "research_event_stream.rows." .. index .. " has a wrong domain rank"
        end
        if row.canonical_tick > stream.confirmed_through_tick then
            return nil, "research_event_stream.rows." .. index .. " is not confirmed"
        end
        if previous and not row_precedes(previous, row) then
            return nil, "research_event_stream.rows." .. index .. " breaks the canonical order"
        end
        previous = row
    end
    local expected, hash_err = research_timeline.stream_hash(stream)
    if not expected then
        return nil, hash_err
    end
    if expected ~= stream.stream_hash then
        return nil, "research_event_stream.stream_hash does not cover its rows"
    end
    return true
end

-- Map a wall-clock or render-time cue to the nearest canonical boundary and
-- keep the signed mapping error. A cue outside the trace window fails closed
-- rather than clamping silently.
---@param clock ResearchBoundaryClock
---@param wall_clock_ms number
---@param boundary_source ResearchBoundarySource?
---@return ResearchBoundaryMapping?, string?
function research_timeline.map_to_boundary(clock, wall_clock_ms, boundary_source)
    local ok, err = research_schema.validate(
        research_schema.record("research_boundary_clock/v1", {
            { name = "origin_wall_clock_ms", kind = "number", min = 0 },
            { name = "tick_rate", kind = "integer", min = 1 },
            { name = "first_boundary_tick", kind = "integer", min = 0 },
            { name = "last_boundary_tick", kind = "integer", min = 0 },
            { name = "non_simulated_ms", kind = "number", min = 0 },
        }),
        clock
    )
    if not ok then
        return nil, err
    end
    if type(wall_clock_ms) ~= "number" or wall_clock_ms ~= wall_clock_ms then
        return nil, "research boundary cue must be a finite wall-clock millisecond value"
    end
    if clock.last_boundary_tick < clock.first_boundary_tick then
        return nil, "research boundary clock window is empty"
    end
    if clock.tick_rate ~= fixed_clock.TICK_RATE then
        return nil, "research boundary clock tick rate is unsupported"
    end
    local ms_per_tick = 1000 / clock.tick_rate
    local simulated_ms = wall_clock_ms - clock.origin_wall_clock_ms - clock.non_simulated_ms
    local raw_tick = clock.first_boundary_tick + simulated_ms / ms_per_tick
    if raw_tick < clock.first_boundary_tick - 0.5 or raw_tick > clock.last_boundary_tick + 0.5 then
        return nil, "research boundary cue lies outside the trace window"
    end
    local nearest = math.floor(raw_tick + 0.5)
    if nearest < clock.first_boundary_tick then
        nearest = clock.first_boundary_tick
    elseif nearest > clock.last_boundary_tick then
        nearest = clock.last_boundary_tick
    end
    return {
        canonical_tick = nearest,
        mapping_error_ms = (raw_tick - nearest) * ms_per_tick,
        boundary_source = boundary_source or "wall_clock_mapped",
    }
end

---@param set any
---@return boolean?, string?
function research_timeline.validate_annotation_set(set)
    if type(set) ~= "table" then
        return nil, "research annotation set must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        research_timeline.ANNOTATION_KIND,
        research_timeline.SUPPORTED_VERSIONS,
        research_timeline.VERSION,
        set.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_timeline.ANNOTATION_SET_SHAPE, set)
    if not ok then
        return nil, err
    end
    local seen = {}
    for index, annotation in ipairs(set.annotations) do
        local path = "research_annotation_set.annotations." .. index
        if seen[annotation.annotation_id] then
            return nil, path .. " duplicates an annotation id"
        end
        seen[annotation.annotation_id] = true
        if annotation.boundary_source == "canonical_tick" then
            if annotation.mapping_error_ms ~= 0 then
                return nil, path .. " canonical cues cannot carry a mapping error"
            end
        elseif annotation.wall_clock_ms == nil then
            return nil, path .. " mapped cues must retain their wall-clock source"
        end
        if annotation.free_text ~= nil and set.author_role == "tool" then
            return nil, path .. " free text must come from a participant or a researcher"
        end
    end
    return true
end

-- Orphan-join and withdrawal guard for the annotation side, mirroring
-- `research_response.validate_against_session`. Annotation sets carry
-- `session_id` and optional free text, so they need a withdrawal path at least as
-- strong as the one structured survey answers have.
---@param set table
---@param envelope ResearchSessionEnvelope
---@return boolean?, string?
function research_timeline.validate_against_session(set, envelope)
    local ok, err = research_timeline.validate_annotation_set(set)
    if not ok then
        return nil, err
    end
    if type(envelope) ~= "table" or type(envelope.session_id) ~= "string" then
        return nil, "research session envelope is required"
    end
    if set.session_id ~= envelope.session_id then
        return nil, "research_annotation_set.session_id is an orphan join"
    end
    if envelope.lifecycle.status == "withdrawn" then
        return nil, "research_annotation_set belongs to a withdrawn session"
    end
    if set.agreement_version ~= envelope.agreement.agreement_version then
        return nil,
            "research_annotation_set.agreement_version disagrees with the accepted agreement"
    end
    return true
end

-- Join annotation sets onto one confirmed stream. Many sets may reference the
-- same trace; none of them may reference an event or boundary the stream does
-- not contain.
---@param stream ResearchEventStream
---@param sets table[]
---@return boolean?, string?
function research_timeline.join_annotations(stream, sets)
    local ok, err = research_timeline.validate_stream(stream)
    if not ok then
        return nil, err
    end
    if type(sets) ~= "table" then
        return nil, "research annotation sets must be an array"
    end
    local event_ids = {}
    for _, row in ipairs(stream.rows) do
        event_ids[row.event_id] = true
    end
    local set_ids = {}
    local annotation_ids = {}
    for index, set in ipairs(sets) do
        local set_ok, set_err = research_timeline.validate_annotation_set(set)
        if not set_ok then
            return nil, set_err
        end
        if set.run_scope_id ~= stream.run_scope_id then
            return nil, "research annotation set " .. index .. " references another trace"
        end
        if set_ids[set.annotation_set_id] then
            return nil, "research annotation set " .. set.annotation_set_id .. " is duplicated"
        end
        set_ids[set.annotation_set_id] = true
        for position, annotation in ipairs(set.annotations) do
            local key = set.annotation_set_id .. "/" .. annotation.annotation_id
            if annotation_ids[key] then
                return nil, "research annotation " .. key .. " is duplicated"
            end
            annotation_ids[key] = true
            if annotation.canonical_tick > stream.confirmed_through_tick then
                return nil,
                    "research annotation "
                        .. key
                        .. " points past the confirmed boundary at position "
                        .. position
            end
            if annotation.event_id ~= nil and not event_ids[annotation.event_id] then
                return nil, "research annotation " .. key .. " is an orphan join"
            end
        end
    end
    return true
end

return research_timeline
