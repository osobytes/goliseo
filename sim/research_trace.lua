-- Versioned gameplay trace manifest.
--
-- The manifest *references* the authoritative gameplay spine in
-- `sim/input_tape.lua`; it never duplicates the frame format and never adds a
-- participant or survey field to `InputTapeIdentity`. Two consequences are
-- load-time structural facts rather than review promises:
--
--   1. `simulation` is the only manifest group that feeds
--      `research_trace.simulation_identity_hash`, so research annotations and
--      observational runtime diagnostics cannot move a simulation boundary
--      hash; and
--   2. `research_links` is an append-only list of opaque join keys, so one tape
--      can carry many research annotations without the tape or its simulation
--      identity changing.

local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match_snapshot = require("sim.match_snapshot")
local research_schema = require("sim.research_schema")

---@alias ResearchProducerKind "human"|"bot"|"replay"

---@alias ResearchTraceCompletion
---| "completed" -- the match reached full time inside the tape
---| "incomplete_interrupted" -- operator or participant stopped the session
---| "incomplete_abandoned" -- participant left; no clean stop was recorded
---| "incomplete_process_exit" -- abrupt exit; the tail is whatever was flushed

---@alias ResearchRawDeviceEventPolicy
---| "not_collected"
---| "minimized_diagnostic"
---| "full_diagnostic"

---@class ResearchTraceSlotProducer
---@field slot string
---@field team InputTeam
---@field player_id string
---@field producer_kind ResearchProducerKind
---@field producer_policy_id string?

---@class ResearchTraceDivergence
---@field boundary_tick integer
---@field expected_hash string
---@field actual_hash string
---@field state_path string
---@field causal_input_tick integer?

---@class ResearchTraceSimulation
---@field tape_version integer
---@field input_version integer
---@field snapshot_version integer
---@field ruleset_version integer
---@field event_schema_version integer
---@field combat_identity string?
---@field build string
---@field source string
---@field content string
---@field tuning string
---@field config string
---@field fixture string
---@field seed integer
---@field tick_rate integer
---@field first_boundary_tick integer
---@field last_boundary_tick integer
---@field frame_count integer
---@field tape_content_hash string
---@field initial_boundary_hash string
---@field final_boundary_hash string
---@field confirmed_event_stream_hash string
---@field completion ResearchTraceCompletion
---@field producers ResearchTraceSlotProducer[]
---@field divergence ResearchTraceDivergence?

---@class ResearchTraceRuntime
---@field platform string
---@field renderer string
---@field render_hz number
---@field render_hz_mode "fixed"|"variable"
---@field input_device "keyboard"|"gamepad"|"touch"|"mixed"
---@field mean_frame_ms number
---@field p99_frame_ms number
---@field dropped_frame_count integer
---@field pause_count integer
---@field goal_replay_count integer
---@field rollback_count integer
---@field max_rollback_ticks integer
---@field raw_device_event_policy ResearchRawDeviceEventPolicy
---@field raw_device_event_clock "none"|"wall_clock_monotonic"

---@class ResearchTraceLink
---@field link_kind "research_session"|"annotation_set"|"response_set"|"derived_dataset"
---@field target_id string
---@field target_hash string

---@class ResearchTraceManifest
---@field schema_version integer
---@field manifest_kind "gameplay_trace_manifest"
---@field digest string
---@field trace_id string
---@field game_instance_id string
---@field simulation ResearchTraceSimulation
---@field runtime ResearchTraceRuntime
---@field research_links ResearchTraceLink[]

---@class ResearchTraceOptions
---@field game_instance_id string
---@field ruleset_version integer
---@field event_schema_version integer
---@field confirmed_event_stream_hash string
---@field completion ResearchTraceCompletion
---@field producers ResearchTraceSlotProducer[]
---@field runtime ResearchTraceRuntime
---@field divergence ResearchTraceDivergence?
---@field research_links ResearchTraceLink[]?

---@class ResearchTraceModule
local research_trace = {}

research_trace.VERSION = 1
research_trace.SUPPORTED_VERSIONS = { [1] = true }
research_trace.KIND = "gameplay_trace_manifest"
research_trace.TAPE_CONTENT_LABEL = "input-tape-content/v1"
research_trace.TRACE_ID_LABEL = "gameplay-trace/v1"

research_trace.PRODUCER_KINDS = research_schema.enum({ "human", "bot", "replay" })
research_trace.COMPLETIONS = research_schema.enum({
    "completed",
    "incomplete_interrupted",
    "incomplete_abandoned",
    "incomplete_process_exit",
})
research_trace.LINK_KINDS = research_schema.enum({
    "research_session",
    "annotation_set",
    "response_set",
    "derived_dataset",
})

local DIVERGENCE_FIELDS = {
    { name = "boundary_tick", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    { name = "expected_hash", kind = "hash" },
    { name = "actual_hash", kind = "hash" },
    { name = "state_path", kind = "string" },
    {
        name = "causal_input_tick",
        kind = "integer",
        optional = true,
        min = 0,
        max = input_frame.MAX_TICK,
    },
}

local PRODUCER_FIELDS = {
    { name = "slot", kind = "id" },
    { name = "team", kind = "enum", values = research_schema.enum({ "home", "away" }) },
    { name = "player_id", kind = "id" },
    { name = "producer_kind", kind = "enum", values = research_trace.PRODUCER_KINDS },
    { name = "producer_policy_id", kind = "id", optional = true },
}

-- Everything that describes deterministic simulation truth. This group, and
-- only this group, feeds `simulation_identity_hash`.
research_trace.SIMULATION_SHAPE = research_schema.record("gameplay_trace_simulation/v1", {
    { name = "tape_version", kind = "integer", min = 1 },
    { name = "input_version", kind = "integer", min = 1 },
    { name = "snapshot_version", kind = "integer", min = 1 },
    { name = "ruleset_version", kind = "integer", min = 1 },
    { name = "event_schema_version", kind = "integer", min = 1 },
    { name = "combat_identity", kind = "string", optional = true },
    { name = "build", kind = "string" },
    { name = "source", kind = "string" },
    { name = "content", kind = "string" },
    -- Empty means "no active tuning override", exactly as `tuning.serialize()`
    -- reports it, so this is the one simulation string allowed to be empty.
    { name = "tuning", kind = "string", min_length = 0 },
    { name = "config", kind = "string" },
    { name = "fixture", kind = "string" },
    { name = "seed", kind = "integer" },
    { name = "tick_rate", kind = "integer", min = 1 },
    { name = "first_boundary_tick", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    { name = "last_boundary_tick", kind = "integer", min = 0, max = input_frame.MAX_TICK },
    { name = "frame_count", kind = "integer", min = 0 },
    { name = "tape_content_hash", kind = "hash" },
    { name = "initial_boundary_hash", kind = "hash" },
    { name = "final_boundary_hash", kind = "hash" },
    { name = "confirmed_event_stream_hash", kind = "hash" },
    { name = "completion", kind = "enum", values = research_trace.COMPLETIONS },
    {
        name = "producers",
        kind = "array",
        min_length = input_frame.SLOT_COUNT,
        max_length = input_frame.SLOT_COUNT,
        element = { name = "producer", kind = "record", fields = PRODUCER_FIELDS },
    },
    { name = "divergence", kind = "record", optional = true, fields = DIVERGENCE_FIELDS },
})

-- Observational runtime evidence (section 4.0 of the combat fun evidence
-- contract): protocol-repeatable, never byte-identical, never authoritative.
research_trace.RUNTIME_SHAPE = research_schema.record("gameplay_trace_runtime/v1", {
    { name = "platform", kind = "id" },
    { name = "renderer", kind = "id" },
    { name = "render_hz", kind = "number", min = 1, max = 1000 },
    {
        name = "render_hz_mode",
        kind = "enum",
        values = research_schema.enum({ "fixed", "variable" }),
    },
    {
        name = "input_device",
        kind = "enum",
        values = research_schema.enum({ "keyboard", "gamepad", "touch", "mixed" }),
    },
    { name = "mean_frame_ms", kind = "number", min = 0 },
    { name = "p99_frame_ms", kind = "number", min = 0 },
    { name = "dropped_frame_count", kind = "integer", min = 0 },
    { name = "pause_count", kind = "integer", min = 0 },
    { name = "goal_replay_count", kind = "integer", min = 0 },
    { name = "rollback_count", kind = "integer", min = 0 },
    { name = "max_rollback_ticks", kind = "integer", min = 0 },
    {
        name = "raw_device_event_policy",
        kind = "enum",
        values = research_schema.enum({
            "not_collected",
            "minimized_diagnostic",
            "full_diagnostic",
        }),
    },
    {
        name = "raw_device_event_clock",
        kind = "enum",
        values = research_schema.enum({ "none", "wall_clock_monotonic" }),
    },
})

local LINK_FIELDS = {
    { name = "link_kind", kind = "enum", values = research_trace.LINK_KINDS },
    { name = "target_id", kind = "id" },
    { name = "target_hash", kind = "hash" },
}

research_trace.SHAPE = research_schema.record("gameplay_trace_manifest/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_trace.KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "trace_id", kind = "hash" },
    { name = "game_instance_id", kind = "id" },
    { name = "simulation", kind = "record", fields = research_trace.SIMULATION_SHAPE.fields },
    { name = "runtime", kind = "record", fields = research_trace.RUNTIME_SHAPE.fields },
    {
        name = "research_links",
        kind = "array",
        max_length = 1024,
        element = { name = "link", kind = "record", fields = LINK_FIELDS },
    },
})

-- The manifest field partition. `simulation` alone determines simulation
-- identity; `annotation` and `envelope` groups are deliberately excluded.
research_trace.FIELD_GROUPS = {
    simulation = { "simulation" },
    annotation = { "runtime", "research_links" },
    envelope = { "schema_version", "manifest_kind", "digest", "trace_id", "game_instance_id" },
}

do
    local disjoint, overlap_err = research_schema.assert_disjoint(
        "gameplay_trace_manifest field groups",
        research_trace.FIELD_GROUPS
    )
    assert(disjoint, overlap_err)
    local covered = {}
    for _, members in pairs(research_trace.FIELD_GROUPS) do
        for _, member in ipairs(members) do
            covered[member] = true
        end
    end
    for _, field in ipairs(research_trace.SHAPE.fields) do
        assert(covered[field.name], "gameplay trace field " .. field.name .. " has no group")
    end
end

-- Content hash of the authoritative tape, derived only from the tape's own
-- identity, canonical initial boundary, and canonical frame wires. This is a
-- reference to `sim/input_tape.lua`, not a second frame format.
---@param tape InputTape
---@return string?, string?
function research_trace.tape_content_hash(tape)
    local ok, err = pcall(input_tape.validate_structure, tape)
    if not ok then
        return nil, "gameplay trace tape is not a valid input tape: " .. tostring(err)
    end
    local identity = input_tape.copy_identity(tape.identity)
    local parts = {
        identity.tape_version,
        identity.input_version,
        identity.snapshot_version,
        identity.build,
        identity.source,
        identity.content,
        identity.tuning,
        identity.config,
        identity.fixture,
        identity.seed,
        identity.tick_rate,
        identity.combat or "",
        match_snapshot.encode(tape.initial),
        #tape.frames,
    }
    for index = 1, #tape.frames do
        local wire, wire_err = input_frame.encode(tape.frames[index])
        if not wire then
            return nil,
                "gameplay trace frame " .. index .. " is unencodable: " .. tostring(wire_err)
        end
        parts[#parts + 1] = wire
    end
    for index = 1, #tape.boundary_hashes do
        parts[#parts + 1] = tape.boundary_hashes[index]
    end
    return research_schema.tuple_hash(research_trace.TAPE_CONTENT_LABEL, parts)
end

-- Hash of the simulation group only. Participant, session, annotation, and
-- runtime data are structurally outside the preimage.
---@param manifest ResearchTraceManifest
---@return string?, string?
function research_trace.simulation_identity_hash(manifest)
    if type(manifest) ~= "table" then
        return nil, "gameplay trace manifest must be a table"
    end
    return research_schema.content_hash(research_trace.SIMULATION_SHAPE, manifest.simulation)
end

---@param manifest ResearchTraceManifest
---@return string?, string?
function research_trace.derive_trace_id(manifest)
    local identity, err = research_trace.simulation_identity_hash(manifest)
    if not identity then
        return nil, err
    end
    if type(manifest.game_instance_id) ~= "string" then
        return nil, "gameplay_trace_manifest.game_instance_id is required"
    end
    return research_schema.tuple_hash(
        research_trace.TRACE_ID_LABEL,
        { identity, manifest.game_instance_id }
    )
end

---@param manifest any
---@return boolean?, string?
function research_trace.validate(manifest)
    if type(manifest) ~= "table" then
        return nil, "gameplay trace manifest must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        "gameplay_trace_manifest",
        research_trace.SUPPORTED_VERSIONS,
        research_trace.VERSION,
        manifest.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_trace.SHAPE, manifest)
    if not ok then
        return nil, err
    end
    ---@cast manifest ResearchTraceManifest
    local simulation = manifest.simulation
    if simulation.tick_rate ~= fixed_clock.TICK_RATE then
        return nil, "gameplay_trace_manifest.simulation.tick_rate is unsupported"
    end
    if simulation.last_boundary_tick ~= simulation.first_boundary_tick + simulation.frame_count then
        return nil, "gameplay_trace_manifest.simulation boundary range disagrees with frame_count"
    end
    if simulation.tape_version == input_tape.COMBAT_VERSION then
        if simulation.combat_identity == nil then
            return nil,
                "gameplay_trace_manifest.simulation.combat_identity is required for a combat tape"
        end
    elseif simulation.combat_identity ~= nil then
        return nil,
            "gameplay_trace_manifest.simulation.combat_identity is only valid for a combat tape"
    end
    if simulation.frame_count == 0 and simulation.completion == "completed" then
        return nil, "gameplay_trace_manifest.simulation cannot be completed with no frames"
    end
    if simulation.divergence and simulation.completion == "completed" then
        return nil, "gameplay_trace_manifest.simulation cannot report divergence and completion"
    end
    local seen_slots = {}
    for index, producer in ipairs(simulation.producers) do
        local expected = input_frame.slot(index)
        if not expected then
            return nil, "gameplay_trace_manifest.simulation.producers has too many slots"
        end
        if producer.slot ~= expected.id or producer.team ~= expected.team then
            return nil,
                "gameplay_trace_manifest.simulation.producers."
                    .. index
                    .. " violates canonical slot order"
        end
        if seen_slots[producer.player_id] then
            return nil, "gameplay_trace_manifest.simulation.producers duplicate a player"
        end
        seen_slots[producer.player_id] = true
        if producer.producer_kind == "human" and producer.producer_policy_id ~= nil then
            return nil,
                "gameplay_trace_manifest.simulation.producers."
                    .. index
                    .. " human slots cannot declare a bot policy"
        end
        if producer.producer_kind ~= "human" and producer.producer_policy_id == nil then
            return nil,
                "gameplay_trace_manifest.simulation.producers."
                    .. index
                    .. " machine slots must declare a policy id"
        end
    end
    local link_keys = {}
    for index, link in ipairs(manifest.research_links) do
        local key = link.link_kind .. "/" .. link.target_id
        if link_keys[key] then
            return nil, "gameplay_trace_manifest.research_links." .. index .. " is duplicated"
        end
        link_keys[key] = true
    end
    local expected_id, id_err = research_trace.derive_trace_id(manifest)
    if not expected_id then
        return nil, id_err
    end
    if manifest.trace_id ~= expected_id then
        return nil, "gameplay_trace_manifest.trace_id is not derived from its simulation identity"
    end
    return true
end

---@param manifest ResearchTraceManifest
---@return string?, string?
function research_trace.content_hash(manifest)
    local ok, err = research_trace.validate(manifest)
    if not ok then
        return nil, err
    end
    return research_schema.content_hash(research_trace.SHAPE, manifest)
end

---@param manifest ResearchTraceManifest
---@return string?, string?
function research_trace.encode(manifest)
    local ok, err = research_trace.validate(manifest)
    if not ok then
        return nil, err
    end
    return research_schema.encode(research_trace.SHAPE, manifest)
end

---@param bytes string
---@return ResearchTraceManifest?, string?
function research_trace.decode(bytes)
    local manifest, err = research_schema.decode(research_trace.SHAPE, bytes)
    if not manifest then
        return nil, err
    end
    local ok, validate_err = research_trace.validate(manifest)
    if not ok then
        return nil, validate_err
    end
    return manifest
end

-- Build a manifest from an immutable tape plus the diagnostics the recorder
-- owns. The tape is only read: no field of `tape` is written, and no research
-- identifier reaches `tape.identity`.
---@param tape InputTape
---@param options ResearchTraceOptions
---@return ResearchTraceManifest?, string?
function research_trace.from_tape(tape, options)
    if type(options) ~= "table" then
        return nil, "gameplay trace options are required"
    end
    local content_hash, content_err = research_trace.tape_content_hash(tape)
    if not content_hash then
        return nil, content_err
    end
    local identity = input_tape.copy_identity(tape.identity)
    local initial_state = match_snapshot.restore(tape.initial)
    local first_tick = initial_state.input_tick
    local producers = {}
    for index = 1, input_frame.SLOT_COUNT do
        local supplied = (options.producers or {})[index]
        if type(supplied) ~= "table" then
            return nil, "gameplay trace producers." .. index .. " is required"
        end
        local assignment = identity.ownership.slots[index]
        producers[index] = {
            slot = assignment.slot,
            team = assignment.team,
            player_id = assignment.player_id,
            producer_kind = supplied.producer_kind,
            producer_policy_id = supplied.producer_policy_id,
        }
    end
    local manifest = {
        schema_version = research_trace.VERSION,
        manifest_kind = research_trace.KIND,
        digest = research_schema.DIGEST,
        trace_id = "0000000000000000",
        game_instance_id = options.game_instance_id,
        simulation = {
            tape_version = identity.tape_version,
            input_version = identity.input_version,
            snapshot_version = identity.snapshot_version,
            ruleset_version = options.ruleset_version,
            event_schema_version = options.event_schema_version,
            combat_identity = identity.combat,
            build = identity.build,
            source = identity.source,
            content = identity.content,
            tuning = identity.tuning,
            config = identity.config,
            fixture = identity.fixture,
            seed = identity.seed,
            tick_rate = identity.tick_rate,
            first_boundary_tick = first_tick,
            last_boundary_tick = first_tick + #tape.frames,
            frame_count = #tape.frames,
            tape_content_hash = content_hash,
            initial_boundary_hash = tape.boundary_hashes[1],
            final_boundary_hash = tape.boundary_hashes[#tape.boundary_hashes],
            confirmed_event_stream_hash = options.confirmed_event_stream_hash,
            completion = options.completion,
            producers = producers,
            divergence = research_schema.copy(options.divergence),
        },
        runtime = research_schema.copy(options.runtime),
        research_links = research_schema.copy(options.research_links) or {},
    }
    local shape_ok, shape_err = research_schema.validate(research_trace.SHAPE, manifest)
    if not shape_ok then
        return nil, shape_err
    end
    local trace_id, id_err = research_trace.derive_trace_id(manifest)
    if not trace_id then
        return nil, id_err
    end
    manifest.trace_id = trace_id
    local ok, err = research_trace.validate(manifest)
    if not ok then
        return nil, err
    end
    ---@cast manifest ResearchTraceManifest
    return manifest
end

-- The confirmed event stream is scoped by tape content, not by manifest, so it
-- can be built before recorder diagnostics exist. This is the join that proves
-- a manifest and a stream describe the same run.
---@param manifest ResearchTraceManifest
---@param stream ResearchEventStream
---@return boolean?, string?
function research_trace.validate_against_stream(manifest, stream)
    local ok, err = research_trace.validate(manifest)
    if not ok then
        return nil, err
    end
    if type(stream) ~= "table" then
        return nil, "research event stream must be a table"
    end
    if stream.run_scope_id ~= manifest.simulation.tape_content_hash then
        return nil, "research event stream describes another tape"
    end
    if stream.game_instance_id ~= manifest.game_instance_id then
        return nil, "research event stream describes another game instance"
    end
    if stream.stream_hash ~= manifest.simulation.confirmed_event_stream_hash then
        return nil,
            "gameplay_trace_manifest.simulation.confirmed_event_stream_hash does not match the stream"
    end
    if stream.confirmed_boundary > manifest.simulation.last_boundary_tick then
        return nil, "research event stream confirms past the recorded tape boundary"
    end
    if stream.confirmed_boundary < manifest.simulation.first_boundary_tick then
        return nil, "research event stream confirms before the recorded tape boundary"
    end
    return true
end

-- Attach one more research annotation to an existing manifest. Returns a new
-- manifest: the input is never mutated, the simulation identity is unchanged,
-- and duplicate links fail closed.
---@param manifest ResearchTraceManifest
---@param link ResearchTraceLink
---@return ResearchTraceManifest?, string?
function research_trace.with_research_link(manifest, link)
    local ok, err = research_trace.validate(manifest)
    if not ok then
        return nil, err
    end
    local next_manifest = research_schema.copy(manifest)
    ---@cast next_manifest ResearchTraceManifest
    local links = next_manifest.research_links
    links[#links + 1] = research_schema.copy(link)
    local valid, valid_err = research_trace.validate(next_manifest)
    if not valid then
        return nil, valid_err
    end
    return next_manifest
end

return research_trace
