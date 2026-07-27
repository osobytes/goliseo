local protocol = require("game.online.protocol")
local input_frame = require("sim.input_frame")

---@alias CoordinatorOrigin "local"|"remote"|"timeout"
---@alias CoordinatorDisposition "applied"|"idempotent"|"rejected"
---@alias CoordinatorActionKind "send"|"close"|"start_match"|"terminate"
---@alias CoordinatorNetcodeFailure "input_channel"|"late_input"|"desync"
---@alias CoordinatorTerminalReason
---| "completed"
---| "local_abort"
---| "peer_abort"
---| "guest_left"
---| "host_left"
---| "removed"
---| "transport_lost"
---| "protocol_violation"
---| "manifest_mismatch"
---| "invalid_assignment"
---| "start_ack_timeout"
---| "input_channel_failure"
---| "late_input"
---| "hash_mismatch"
---@alias CoordinatorSlotDriver "human"|"ai"
---@alias CoordinatorRejectCode
---| SessionProtocolErrorCode
---| "capacity"
---| "duplicate_peer"
---| "role_conflict"
---| "invalid_assignment"
---| "unknown_link"
---| "not_permitted"
---@alias CoordinatorEventKind
---| "connect"
---| "control"
---| "link_lost"
---| "propose_manifest"
---| "assign_slots"
---| "set_ready"
---| "begin_countdown"
---| "tick"
---| "match_phase"
---| "hash_report"
---| "finish"
---| "netcode_failure"
---| "leave"
---| "abort"

---@class CoordinatorWireRecord
---@field sequence integer
---@field wire string

---@class CoordinatorHashRecord
---@field tick integer
---@field hash string

---@class CoordinatorPeer
---@field peer_id string
---@field link_id string? -- nil for the local peer; a transport link id otherwise.
---@field role SessionRole
---@field runtime SessionRuntimeIdentity
---@field accepted_manifest_id string?
---@field assigned boolean
---@field ready boolean
---@field started boolean
---@field result_acked boolean
---@field hash_mismatches integer
---@field last_sequence integer -- -1 until the peer has been heard from.
---@field window CoordinatorWireRecord[] -- Bounded recent inbound wires, oldest first.

---@class CoordinatorManifestExpectation
---@field build_id string?
---@field source_id string?
---@field content_id string?
---@field tuning_id string?
---@field match_config_id string?
---@field fixture_id string?
---@field arena_id string?
---@field combat_rules_id string?
---@field gameplay_ai_policy_id string?
---@field combat_status SessionCombatStatus?

---@class CoordinatorFreeze
---@field manifest_id string
---@field assignment_id string -- The exact ownership generation that was frozen.
---@field countdown_id string
---@field first_input_tick integer
---@field seed integer
---@field tick_rate integer
---@field duration_ticks integer
---@field max_goals integer
---@field content_id string
---@field tuning_id string
---@field combat_rules_id string
---@field gameplay_ai_policy_id string
---@field combat_status SessionCombatStatus
---@field match_mode SessionMatchMode -- Frozen with everything else; never varies mid-match.
---@field assignments SessionSlotProducer[]
---@field sources table<InputSlotId, SessionSlotProducer>
---@field owned table<string, InputSlotId[]> -- Human producer id -> owned slots, canonical order.
---@field live table<string, InputSlotId> -- Human producer id -> the slot live at the first tick.

---@class CoordinatorMatchState
---@field phase SessionMatchPhase
---@field tick integer
---@field home_score integer
---@field away_score integer

---@class CoordinatorResult
---@field final_tick integer
---@field home_score integer
---@field away_score integer
---@field final_hash string

---@class CoordinatorTerminal
---@field reason CoordinatorTerminalReason
---@field code SessionRejectCode? -- nil only for a completed session.
---@field origin CoordinatorOrigin
---@field peer_id string?
---@field detail string?

---@class CoordinatorAction
---@field kind CoordinatorActionKind
---@field targets string[]? -- send: ordered link ids.
---@field message SessionControlMessage?
---@field link_id string? -- close: the link to tear down.
---@field freeze CoordinatorFreeze? -- start_match: the frozen session identity.
---@field terminal CoordinatorTerminal? -- terminate: the stable session reason.

---@class CoordinatorOutcome
---@field accepted boolean
---@field disposition CoordinatorDisposition
---@field code CoordinatorRejectCode?
---@field reason string?
---@field actions CoordinatorAction[]

---@class CoordinatorState
---@field version integer
---@field role SessionRole
---@field session_id string
---@field peer_id string
---@field host_peer_id string
---@field host_link_id string? -- Guest only: the single link back to the host.
---@field runtime SessionRuntimeIdentity
---@field expectation CoordinatorManifestExpectation?
---@field phase SessionLifecyclePhase
---@field clock integer
---@field sequence integer -- Next outbound sender-local sequence.
---@field peers CoordinatorPeer[] -- Local peer first, then admission order.
---@field manifest SessionManifest?
---@field manifest_id string?
---@field assignments SessionSlotProducer[]?
---@field assignment_id string? -- Identity of the ownership generation in force.
---@field assignment_epoch integer -- Monotonic count of ownership publications.
---@field freeze CoordinatorFreeze?
---@field countdown_remaining integer?
---@field start_deadline integer?
---@field match CoordinatorMatchState?
---@field result CoordinatorResult?
---@field local_result CoordinatorResult?
---@field hashes CoordinatorHashRecord[]
---@field terminal CoordinatorTerminal?

---@class CoordinatorOptions
---@field role SessionRole
---@field session_id string
---@field peer_id string
---@field host_peer_id string?
---@field host_link_id string?
---@field runtime SessionRuntimeIdentity
---@field expectation CoordinatorManifestExpectation?

---@class CoordinatorSummary
---@field role SessionRole
---@field phase SessionLifecyclePhase
---@field peer_count integer
---@field ready_count integer
---@field manifest_id string?
---@field frozen boolean
---@field terminal CoordinatorTerminal?

---@class SessionCoordinatorModule
local coordinator = {}

coordinator.VERSION = 1
coordinator.MAX_PEERS = input_frame.SLOT_COUNT
coordinator.MAX_GUESTS = coordinator.MAX_PEERS - 1
coordinator.DUPLICATE_WINDOW = 8
coordinator.HASH_WINDOW = 8
coordinator.MAX_HASH_MISMATCHES = 3
coordinator.START_ACK_TIMEOUT_TICKS = 120
coordinator.BOT_SEED_STRIDE = 7919
coordinator.BOT_PRODUCER_PREFIX = "bot."
coordinator.STALE_DUPLICATE_REASON = "duplicate fell outside the retained transcript window"
coordinator.STALE_GENERATION_REASON = "readiness names a superseded ownership generation"

---@type SessionLifecyclePhase[]
coordinator.PHASES = {
    "new",
    "handshake",
    "manifest",
    "assigned",
    "ready",
    "countdown",
    "running",
    "result",
    "terminal",
}

-- Every terminal reason maps to exactly one closed #161 rejection code so the
-- wire stays inside the accepted vocabulary while local reasons stay specific.
---@type table<CoordinatorTerminalReason, SessionRejectCode>
coordinator.TERMINAL_CODES = {
    local_abort = "host_abort",
    peer_abort = "host_abort",
    guest_left = "peer_disconnect",
    host_left = "peer_disconnect",
    removed = "peer_disconnect",
    transport_lost = "peer_disconnect",
    protocol_violation = "malformed_message",
    manifest_mismatch = "manifest_mismatch",
    invalid_assignment = "invalid_assignment",
    start_ack_timeout = "peer_disconnect",
    input_channel_failure = "peer_disconnect",
    late_input = "desync",
    hash_mismatch = "desync",
}

---@type table<CoordinatorNetcodeFailure, CoordinatorTerminalReason>
coordinator.NETCODE_REASONS = {
    input_channel = "input_channel_failure",
    late_input = "late_input",
    desync = "hash_mismatch",
}

---@type table<SessionDisconnectCode, CoordinatorTerminalReason>
local DISCONNECT_REASONS = {
    peer_left = "guest_left",
    transport_lost = "transport_lost",
    host_left = "host_left",
    protocol_error = "protocol_violation",
}

-- Fixed comparison order keeps guest-side manifest rejection deterministic.
---@type string[]
local EXPECTATION_FIELDS = {
    "build_id",
    "source_id",
    "content_id",
    "tuning_id",
    "match_config_id",
    "fixture_id",
    "arena_id",
    "combat_rules_id",
    "gameplay_ai_policy_id",
    "combat_status",
}

---@type table<SessionMatchPhase, table<SessionMatchPhase, boolean>>
local MATCH_PHASE_NEXT = {
    kickoff = { playing = true },
    playing = { goal_stoppage = true, full_time = true },
    goal_stoppage = { kickoff = true },
    full_time = {},
    result = {},
}

local PHASE_SET = {}
for _, phase in ipairs(coordinator.PHASES) do
    PHASE_SET[phase] = true
end

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param code CoordinatorRejectCode
---@param reason string
---@return CoordinatorOutcome
local function rejected(code, reason)
    return { accepted = false, disposition = "rejected", code = code, reason = reason, actions = {} }
end

---@param actions CoordinatorAction[]?
---@return CoordinatorOutcome
local function applied(actions)
    return { accepted = true, disposition = "applied", actions = actions or {} }
end

---@return CoordinatorOutcome
local function idempotent()
    return { accepted = true, disposition = "idempotent", actions = {} }
end

-- An accepted no-op that carries why it was dropped, for diagnostics.
---@param reason string
---@return CoordinatorOutcome
local function stale(reason)
    return { accepted = true, disposition = "idempotent", reason = reason, actions = {} }
end

---@param value any
---@return any
local function copy_value(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, item in pairs(value) do
        result[key] = copy_value(item)
    end
    return result
end

---@param peer CoordinatorPeer
---@return CoordinatorPeer
local function copy_peer(peer)
    local window = {}
    for index, record in ipairs(peer.window) do
        window[index] = { sequence = record.sequence, wire = record.wire }
    end
    return {
        peer_id = peer.peer_id,
        link_id = peer.link_id,
        role = peer.role,
        runtime = peer.runtime,
        accepted_manifest_id = peer.accepted_manifest_id,
        assigned = peer.assigned,
        ready = peer.ready,
        started = peer.started,
        result_acked = peer.result_acked,
        hash_mismatches = peer.hash_mismatches,
        last_sequence = peer.last_sequence,
        window = window,
    }
end

-- Copy-on-write: rejected events return the untouched original state, so a
-- refused message can never leave partial lifecycle progress behind.
---@param state CoordinatorState
---@return CoordinatorState
local function copy_state(state)
    local peers = {}
    for index, peer in ipairs(state.peers) do
        peers[index] = copy_peer(peer)
    end
    local hashes = {}
    for index, record in ipairs(state.hashes) do
        hashes[index] = { tick = record.tick, hash = record.hash }
    end
    return {
        version = state.version,
        role = state.role,
        session_id = state.session_id,
        peer_id = state.peer_id,
        host_peer_id = state.host_peer_id,
        host_link_id = state.host_link_id,
        runtime = state.runtime,
        expectation = state.expectation,
        phase = state.phase,
        clock = state.clock,
        sequence = state.sequence,
        peers = peers,
        manifest = state.manifest,
        manifest_id = state.manifest_id,
        assignments = state.assignments,
        assignment_id = state.assignment_id,
        assignment_epoch = state.assignment_epoch,
        freeze = state.freeze,
        countdown_remaining = state.countdown_remaining,
        start_deadline = state.start_deadline,
        match = state.match,
        result = state.result,
        local_result = state.local_result,
        hashes = hashes,
        terminal = state.terminal,
    }
end

---@param state CoordinatorState
---@return CoordinatorPeer
local function local_peer(state)
    return state.peers[1]
end

---@param state CoordinatorState
---@param link_id string
---@return CoordinatorPeer?
local function peer_by_link(state, link_id)
    for _, peer in ipairs(state.peers) do
        if peer.link_id == link_id then
            return peer
        end
    end
    return nil
end

---@param state CoordinatorState
---@param peer_id string
---@return CoordinatorPeer?
local function peer_by_id(state, peer_id)
    for _, peer in ipairs(state.peers) do
        if peer.peer_id == peer_id then
            return peer
        end
    end
    return nil
end

---@param state CoordinatorState
---@param exclude_link string?
---@return string[]
local function link_targets(state, exclude_link)
    local targets = {}
    for _, peer in ipairs(state.peers) do
        if peer.link_id and peer.link_id ~= exclude_link then
            targets[#targets + 1] = peer.link_id
        end
    end
    return targets
end

-- Emitting is the single place a coordinator mints outbound control traffic:
-- every message is protocol-valid and phase-valid for the phase it leaves in.
---@param next_state CoordinatorState
---@param kind SessionMessageKind
---@param body SessionMessageBody
---@param targets string[]
---@param actions CoordinatorAction[]
---@return boolean?, string?, CoordinatorRejectCode?
local function try_emit(next_state, kind, body, targets, actions)
    local message, err, code =
        protocol.new(kind, next_state.session_id, next_state.peer_id, next_state.sequence, body)
    if not message then
        return nil, err, code
    end
    local phase_ok, phase_err, phase_code = protocol.validate_phase(message, next_state.phase)
    if not phase_ok then
        return nil, phase_err, phase_code
    end
    next_state.sequence = next_state.sequence + 1
    if #targets > 0 then
        actions[#actions + 1] = { kind = "send", targets = targets, message = message }
    end
    return true
end

---@param next_state CoordinatorState
---@param kind SessionMessageKind
---@param body SessionMessageBody
---@param targets string[]
---@param actions CoordinatorAction[]
local function emit(next_state, kind, body, targets, actions)
    local ok, err = try_emit(next_state, kind, body, targets, actions)
    assert(ok, err)
end

---@class CoordinatorTerminationOptions
---@field reason CoordinatorTerminalReason
---@field code SessionRejectCode?
---@field origin CoordinatorOrigin
---@field peer_id string?
---@field detail string?
---@field announce boolean?
---@field exclude_link string?

---@param next_state CoordinatorState
---@param options CoordinatorTerminationOptions
---@param actions CoordinatorAction[]
---@return CoordinatorState
local function terminate_session(next_state, options, actions)
    local code = options.code or coordinator.TERMINAL_CODES[options.reason]
    if options.announce then
        local targets = link_targets(next_state, options.exclude_link)
        emit(next_state, "abort", { code = assert(code) }, targets, actions)
    end
    for _, peer in ipairs(next_state.peers) do
        if peer.link_id then
            actions[#actions + 1] = { kind = "close", link_id = peer.link_id }
        end
    end
    next_state.phase = "terminal"
    next_state.terminal = {
        reason = options.reason,
        code = code,
        origin = options.origin,
        peer_id = options.peer_id,
        detail = options.detail,
    }
    actions[#actions + 1] = { kind = "terminate", terminal = next_state.terminal }
    return next_state
end

---@param state CoordinatorState
---@param options CoordinatorTerminationOptions
---@return CoordinatorState, CoordinatorOutcome
local function terminate_from(state, options)
    local actions = {}
    local next_state = terminate_session(copy_state(state), options, actions)
    return next_state, applied(actions)
end

---@param expectation CoordinatorManifestExpectation?
---@param manifest SessionManifest
---@return SessionIdentityDifference?
function coordinator.expectation_difference(expectation, manifest)
    if not expectation then
        return nil
    end
    for _, field in ipairs(EXPECTATION_FIELDS) do
        local expected = expectation[field]
        if expected ~= nil and expected ~= manifest[field] then
            return { path = "manifest." .. field, expected = expected, actual = manifest[field] }
        end
    end
    return nil
end

---@param manifest SessionManifest
---@param assignments SessionSlotProducer[]
---@return table<InputSlotId, SessionSlotProducer>?, string?, CoordinatorRejectCode?
function coordinator.slot_sources(manifest, assignments)
    local ok, err, code = protocol.validate_manifest(manifest)
    if not ok then
        return nil, err, code
    end
    ---@type table<string, boolean>
    local keepers = {}
    for _, team in ipairs(manifest.teams) do
        for _, player in ipairs(team.roster) do
            if player.position == "keeper" then
                keepers[player.player_id] = true
            end
        end
    end
    if type(assignments) == "table" then
        for _, producer in ipairs(assignments) do
            if type(producer) == "table" and keepers[producer.player_id] then
                return nil,
                    "combat-protected keepers cannot own a canonical outfield slot",
                    "invalid_assignment"
            end
        end
    end
    ok, err, code = protocol.validate_assignment_manifest(manifest, assignments)
    if not ok then
        return nil, err, code
    end
    -- `validate_assignment_manifest` has already proven that the array holds
    -- exactly the eight canonical slots in OMP-1 order, so indexing it by slot
    -- id cannot collide; this only re-shapes it for lookup.
    ---@type table<InputSlotId, SessionSlotProducer>
    local sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        sources[assert(input_frame.slot(index)).id] = assignments[index]
    end
    return sources
end

-- Humans are seated in contiguous canonical blocks of `slots_per_human`, so the
-- Nth human owns slots `(N-1)*k+1 .. N*k`. Four outfield slots per team divide
-- evenly by every supported `k`, so a block never straddles the halfway line and
-- an owned set is always a subset of one team's line. At k = 1 this is exactly
-- the one-slot-per-peer seating OMP-3 already shipped.
---@param manifest SessionManifest
---@param peer_ids string[]
---@return SessionSlotProducer[]?, string?, CoordinatorRejectCode?
function coordinator.plan_assignments(manifest, peer_ids)
    local manifest_ok, manifest_err, manifest_code = protocol.validate_manifest(manifest)
    if not manifest_ok then
        return nil, manifest_err, manifest_code
    end
    local shape = assert(protocol.MATCH_MODES[manifest.match_mode])
    if type(peer_ids) ~= "table" or #peer_ids > shape.humans then
        return nil,
            ("a %s session seats at most %d humans"):format(shape.mode, shape.humans),
            "capacity"
    end
    ---@type table<string, boolean>
    local seen = {}
    ---@type table<integer, string>
    local peer_by_slot = {}
    for order, peer_id in ipairs(peer_ids) do
        if type(peer_id) ~= "string" or seen[peer_id] then
            return nil, "human slot sources must be unique peer ids", "duplicate_peer"
        end
        seen[peer_id] = true
        for offset = 1, shape.slots_per_human do
            peer_by_slot[(order - 1) * shape.slots_per_human + offset] = peer_id
        end
    end
    ---@type SessionSlotProducer[]
    local assignments = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local entry = manifest.slots[index]
        local peer_id = peer_by_slot[index]
        if peer_id then
            assignments[index] = {
                slot = slot.id,
                team = slot.team,
                player_id = entry.player_id,
                producer_kind = "peer",
                producer_id = peer_id,
            }
        else
            assignments[index] = {
                slot = slot.id,
                team = slot.team,
                player_id = entry.player_id,
                producer_kind = "bot",
                producer_id = coordinator.BOT_PRODUCER_PREFIX .. slot.id,
                bot_seed = (manifest.seed + index * coordinator.BOT_SEED_STRIDE)
                    % (protocol.MAX_SEED + 1),
            }
        end
    end
    local sources, err, code = coordinator.slot_sources(manifest, assignments)
    if not sources then
        return nil, err, code
    end
    return assignments
end

---@param state CoordinatorState
---@param assignments SessionSlotProducer[]
---@return boolean?, string?, CoordinatorRejectCode?
local function validate_local_assignments(state, assignments)
    local manifest = state.manifest
    if not manifest or not state.manifest_id then
        return nil, "slot assignment requires an accepted manifest", "invalid_phase"
    end
    local sources, err, code = coordinator.slot_sources(manifest, assignments)
    if not sources then
        return nil, err, code
    end
    ---@type table<string, integer>
    local peer_slots = {}
    for _, producer in ipairs(assignments) do
        if producer.producer_kind == "peer" then
            if not peer_by_id(state, producer.producer_id) then
                return nil,
                    "slot producer " .. producer.producer_id .. " is not an admitted peer",
                    "invalid_assignment"
            end
            peer_slots[producer.producer_id] = (peer_slots[producer.producer_id] or 0) + 1
        elseif peer_by_id(state, producer.producer_id) then
            return nil, "a bot producer cannot reuse an admitted peer id", "invalid_assignment"
        end
    end
    -- `slot_sources` has already proven every owned set matches the frozen
    -- mode; this is the host-only half the guests cannot see, that the owned
    -- sets map onto genuinely admitted peers and leave nobody unseated.
    local shape = assert(protocol.MATCH_MODES[manifest.match_mode])
    for _, peer in ipairs(state.peers) do
        if (peer_slots[peer.peer_id] or 0) ~= shape.slots_per_human then
            return nil,
                ("admitted peer %s must own exactly %d canonical slot(s) in %s"):format(
                    peer.peer_id,
                    shape.slots_per_human,
                    shape.mode
                ),
                "invalid_assignment"
        end
    end
    return true
end

---@param assignments SessionSlotProducer[]?
---@param other SessionSlotProducer[]
---@return boolean
local function assignments_equal(assignments, other)
    if not assignments or #assignments ~= #other then
        return false
    end
    for index, producer in ipairs(assignments) do
        local candidate = other[index]
        if
            producer.slot ~= candidate.slot
            or producer.team ~= candidate.team
            or producer.player_id ~= candidate.player_id
            or producer.producer_kind ~= candidate.producer_kind
            or producer.producer_id ~= candidate.producer_id
            or producer.bot_seed ~= candidate.bot_seed
        then
            return false
        end
    end
    return true
end

---@param state CoordinatorState
---@param peer_id string
---@return boolean
local function owns_slot(state, peer_id)
    if not state.assignments then
        return false
    end
    for _, producer in ipairs(state.assignments) do
        if producer.producer_kind == "peer" and producer.producer_id == peer_id then
            return true
        end
    end
    return false
end

---@param next_state CoordinatorState
local function clear_readiness(next_state)
    for _, peer in ipairs(next_state.peers) do
        peer.ready = false
    end
    if next_state.phase == "ready" then
        next_state.phase = "assigned"
    end
end

-- Only the host observes the whole readiness barrier; a guest knows nothing
-- about its fellow guests and is `ready` exactly when it says so itself.
---@param state CoordinatorState
---@return boolean
local function all_ready(state)
    if state.role == "guest" then
        return local_peer(state).ready
    end
    for _, peer in ipairs(state.peers) do
        if not peer.ready then
            return false
        end
    end
    return true
end

---@param next_state CoordinatorState
local function refresh_ready_phase(next_state)
    if next_state.phase ~= "assigned" and next_state.phase ~= "ready" then
        return
    end
    next_state.phase = all_ready(next_state) and "ready" or "assigned"
end

---@param next_state CoordinatorState
---@param manifest SessionManifest
---@param countdown_id string
---@param first_input_tick integer
---@return CoordinatorFreeze
local function freeze_session(next_state, manifest, countdown_id, first_input_tick)
    local assignments = copy_value(assert(next_state.assignments))
    local sources = assert(coordinator.slot_sources(manifest, assignments))
    ---@type table<string, InputSlotId[]>
    local owned = {}
    ---@type table<string, InputSlotId>
    local live = {}
    for _, producer in ipairs(assignments) do
        if producer.producer_kind == "peer" and not owned[producer.producer_id] then
            local slots = protocol.owned_slots(assignments, producer.producer_id)
            owned[producer.producer_id] = slots
            -- The first owned slot in canonical order is live at the first tick.
            -- It is derived from the frozen assignments, so every peer computes
            -- the same opening live slot without exchanging anything further.
            live[producer.producer_id] = slots[1]
        end
    end
    ---@type CoordinatorFreeze
    local freeze = {
        manifest_id = assert(next_state.manifest_id),
        assignment_id = assert(next_state.assignment_id),
        countdown_id = countdown_id,
        first_input_tick = first_input_tick,
        seed = manifest.seed,
        tick_rate = manifest.tick_rate,
        duration_ticks = manifest.duration_ticks,
        max_goals = manifest.max_goals,
        content_id = manifest.content_id,
        tuning_id = manifest.tuning_id,
        combat_rules_id = manifest.combat_rules_id,
        gameplay_ai_policy_id = manifest.gameplay_ai_policy_id,
        combat_status = manifest.combat_status,
        match_mode = manifest.match_mode,
        assignments = assignments,
        sources = sources,
        owned = owned,
        live = live,
    }
    next_state.freeze = freeze
    return freeze
end

---@param next_state CoordinatorState
---@param tick integer
---@param hash string
local function record_hash(next_state, tick, hash)
    local hashes = next_state.hashes
    hashes[#hashes + 1] = { tick = tick, hash = hash }
    while #hashes > coordinator.HASH_WINDOW do
        table.remove(hashes, 1)
    end
end

---@param state CoordinatorState
---@param tick integer
---@return string?
local function local_hash(state, tick)
    for index = #state.hashes, 1, -1 do
        if state.hashes[index].tick == tick then
            return state.hashes[index].hash
        end
    end
    return nil
end

---@param options CoordinatorOptions
---@return CoordinatorState?, string?, CoordinatorRejectCode?
function coordinator.new(options)
    if type(options) ~= "table" then
        return nil, "coordinator options must be a table", "malformed"
    end
    if options.role ~= "host" and options.role ~= "guest" then
        return nil, "coordinator role must be host or guest", "malformed"
    end
    local host_peer_id = options.host_peer_id or options.peer_id
    if options.role == "guest" then
        if type(options.host_link_id) ~= "string" or options.host_link_id == "" then
            return nil, "a guest coordinator requires the host link id", "malformed"
        end
        if type(options.host_peer_id) ~= "string" then
            return nil, "a guest coordinator requires the host peer id", "malformed"
        end
        if options.host_peer_id == options.peer_id then
            return nil, "a guest cannot claim the host peer identity", "role_conflict"
        end
    elseif options.host_peer_id and options.host_peer_id ~= options.peer_id then
        return nil, "a host coordinator owns the host peer identity", "role_conflict"
    end
    local runtime, runtime_err, runtime_code = protocol.new_runtime(options.runtime)
    if not runtime then
        return nil, runtime_err, runtime_code
    end
    -- Probe the identity bounds by minting the peer's first message id.
    local _, id_err, id_code = protocol.message_id(options.session_id, options.peer_id, 0)
    if id_err then
        return nil, id_err, id_code
    end
    ---@type CoordinatorPeer
    local self_peer = {
        peer_id = options.peer_id,
        link_id = nil,
        role = options.role,
        runtime = runtime,
        accepted_manifest_id = nil,
        assigned = options.role == "host",
        ready = false,
        started = false,
        result_acked = false,
        hash_mismatches = 0,
        last_sequence = -1,
        window = {},
    }
    ---@type CoordinatorPeer[]
    local peers = { self_peer }
    if options.role == "guest" then
        peers[2] = {
            peer_id = host_peer_id,
            link_id = options.host_link_id,
            role = "host",
            runtime = runtime,
            accepted_manifest_id = nil,
            assigned = true,
            ready = false,
            started = false,
            result_acked = false,
            hash_mismatches = 0,
            last_sequence = -1,
            window = {},
        }
    end
    ---@type CoordinatorState
    local state = {
        version = coordinator.VERSION,
        role = options.role,
        session_id = options.session_id,
        peer_id = options.peer_id,
        host_peer_id = host_peer_id,
        host_link_id = options.host_link_id,
        runtime = runtime,
        expectation = options.expectation and copy_value(options.expectation) or nil,
        -- The host admits itself at construction; a guest is unheard until it
        -- sends its handshake, so only guests observe the `new` phase.
        phase = options.role == "host" and "handshake" or "new",
        clock = 0,
        sequence = 0,
        peers = peers,
        manifest = nil,
        manifest_id = nil,
        assignments = nil,
        assignment_id = nil,
        assignment_epoch = 0,
        freeze = nil,
        countdown_remaining = nil,
        start_deadline = nil,
        match = nil,
        result = nil,
        local_result = nil,
        hashes = {},
        terminal = nil,
    }
    return state
end

---@param state CoordinatorState
---@return boolean
function coordinator.is_terminal(state)
    return state.phase == "terminal"
end

---@param state CoordinatorState
---@return boolean
function coordinator.is_frozen(state)
    return state.freeze ~= nil
end

---@param state CoordinatorState
---@return CoordinatorSummary
function coordinator.summary(state)
    local ready_count = 0
    for _, peer in ipairs(state.peers) do
        if peer.ready then
            ready_count = ready_count + 1
        end
    end
    return {
        role = state.role,
        phase = state.phase,
        peer_count = #state.peers,
        ready_count = ready_count,
        manifest_id = state.manifest_id,
        frozen = state.freeze ~= nil,
        terminal = state.terminal,
    }
end

---@param state CoordinatorState
---@param slot InputSlotId
---@return SessionSlotProducer?
function coordinator.slot_owner(state, slot)
    local assignments = state.freeze and state.freeze.assignments or state.assignments
    if not assignments then
        return nil
    end
    for _, producer in ipairs(assignments) do
        if producer.slot == slot then
            return producer
        end
    end
    return nil
end

-- The slots one producer owns, in canonical order: four in 1v1, two in 2v2, one
-- in 4v4. Empty when ownership is unpublished or the producer is not seated.
---@param state CoordinatorState
---@param producer_id string
---@return InputSlotId[]
function coordinator.owned_slots(state, producer_id)
    local assignments = state.freeze and state.freeze.assignments or state.assignments
    if not assignments then
        return {}
    end
    return protocol.owned_slots(assignments, producer_id)
end

-- One human's control moves within their owned set and nowhere else. The rule
-- itself is the shipped single-player one (`docs/controls.md`): winning the ball
-- switches control to the winner, and the `switch` edge without the ball hands
-- control to the outfielder nearest the ball. Both are intersected with the
-- owned set here, which is the whole of the online generalization.
--
-- `switch` must be read from the canonical input row of the currently live slot
-- and `carrier`/`winner`/`ranked` from the deterministic simulation, never from
-- local presentation or local input timing: every peer evaluates the same inputs
-- at the same tick and therefore reaches the same live slot.
--
-- In 4v4 the owned set has one member, so every branch returns it and switching
-- is inert. That is a consequence of the general rule, not a special case.
---@class CoordinatorLiveTransition
---@field switch boolean -- The canonical `switch` edge bit for this tick.
---@field carrier InputSlotId? -- The slot holding the ball, if any.
---@field winner InputSlotId? -- The slot that won the ball on this tick, if any.
---@field ranked InputSlotId[]? -- Outfield slots ordered by deterministic distance to the ball.

---@param owned InputSlotId[]
---@param live InputSlotId
---@param transition CoordinatorLiveTransition
---@return InputSlotId
function coordinator.next_live_slot(owned, live, transition)
    ---@type table<InputSlotId, boolean>
    local set = {}
    for _, slot in ipairs(owned) do
        set[slot] = true
    end
    assert(set[live], "the live slot must belong to the owned set")
    if transition.winner and set[transition.winner] then
        return transition.winner
    end
    if not transition.switch or transition.carrier == live then
        return live
    end
    for _, slot in ipairs(transition.ranked or {}) do
        if set[slot] then
            return slot
        end
    end
    return live
end

-- Who materializes each canonical slot's input row for one tick. A slot is
-- human-driven only while it is that human's live slot; every other owned slot
-- is `ai`, exactly like a declared bot fill, so the input stream cannot tell
-- them apart.
---@param freeze CoordinatorFreeze
---@param live table<string, InputSlotId>? -- Defaults to the frozen opening live slots.
---@return table<InputSlotId, CoordinatorSlotDriver>
function coordinator.slot_drivers(freeze, live)
    live = live or freeze.live
    ---@type table<InputSlotId, CoordinatorSlotDriver>
    local drivers = {}
    for _, producer in ipairs(freeze.assignments) do
        local is_live = producer.producer_kind == "peer"
            and live[producer.producer_id] == producer.slot
        drivers[producer.slot] = is_live and "human" or "ai"
    end
    return drivers
end

-- ---------------------------------------------------------------------------
-- Local command handlers
-- ---------------------------------------------------------------------------

---@param state CoordinatorState
---@return CoordinatorState, CoordinatorOutcome
local function handle_connect(state)
    if state.role ~= "guest" then
        return state, rejected("not_permitted", "only a guest opens a manual connection")
    end
    if state.phase ~= "new" then
        return state, idempotent()
    end
    local actions = {}
    local next_state = copy_state(state)
    emit(
        next_state,
        "handshake",
        { role = "guest", runtime = next_state.runtime },
        link_targets(next_state),
        actions
    )
    next_state.phase = "handshake"
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_propose_manifest(state, event)
    if state.role ~= "host" then
        return state, rejected("not_permitted", "only the host proposes the session manifest")
    end
    local manifest, err, code = protocol.new_manifest(event.manifest)
    if not manifest then
        return state, rejected(code or "malformed", err or "invalid manifest")
    end
    if manifest.session_id ~= state.session_id then
        return state, rejected("identity_mismatch", "manifest names a different session")
    end
    local manifest_id = protocol.manifest_id(manifest)
    if state.manifest_id then
        if state.manifest_id == manifest_id then
            return state, idempotent()
        end
        return state, rejected("identity_mismatch", "the manifest is immutable after proposal")
    end
    if state.phase ~= "handshake" and state.phase ~= "manifest" then
        return state, rejected("invalid_phase", "manifest proposal is closed in " .. state.phase)
    end
    -- Admission closes when the manifest is proposed and the mode is immutable
    -- afterwards, so a lobby that seats more humans than the mode can own is
    -- refused here rather than deadlocking at assignment time.
    local shape = assert(protocol.MATCH_MODES[manifest.match_mode])
    if #state.peers > shape.humans then
        return state,
            rejected(
                "capacity",
                ("%s seats %d humans but %d are admitted"):format(
                    shape.mode,
                    shape.humans,
                    #state.peers
                )
            )
    end
    local actions = {}
    local next_state = copy_state(state)
    emit(
        next_state,
        "manifest_proposal",
        { manifest_id = manifest_id, manifest = manifest },
        link_targets(next_state),
        actions
    )
    next_state.manifest = manifest
    next_state.manifest_id = manifest_id
    next_state.phase = "manifest"
    local_peer(next_state).accepted_manifest_id = manifest_id
    for index = 2, #next_state.peers do
        local peer = next_state.peers[index]
        peer.assigned = true
        emit(
            next_state,
            "peer_assignment",
            { assigned_peer_id = peer.peer_id, role = peer.role },
            { assert(peer.link_id) },
            actions
        )
    end
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_assign_slots(state, event)
    if state.role ~= "host" then
        return state, rejected("not_permitted", "only the host publishes slot assignments")
    end
    if state.phase ~= "manifest" and state.phase ~= "assigned" and state.phase ~= "ready" then
        return state, rejected("invalid_phase", "slot assignment is closed in " .. state.phase)
    end
    -- Publishing ownership only after every peer has accepted the manifest
    -- keeps a slow acceptance from arriving in a phase that forbids it.
    for _, peer in ipairs(state.peers) do
        if peer.accepted_manifest_id ~= state.manifest_id then
            return state,
                rejected("invalid_phase", "every peer must accept the manifest before assignment")
        end
    end
    local ok, err, code = validate_local_assignments(state, event.assignments)
    if not ok then
        return state, rejected(code or "invalid_assignment", err or "invalid slot assignment")
    end
    if assignments_equal(state.assignments, event.assignments) then
        return state, idempotent()
    end
    local actions = {}
    local next_state = copy_state(state)
    local assignments = copy_value(event.assignments)
    -- Every publication is its own generation, even one that restores byte-
    -- identical ownership, because the epoch is part of the identity. Readiness
    -- for any earlier generation can no longer be mistaken for readiness now.
    next_state.assignment_epoch = state.assignment_epoch + 1
    next_state.assignment_id = protocol.assignment_id(assignments, next_state.assignment_epoch)
    next_state.assignments = assignments
    next_state.phase = "assigned"
    clear_readiness(next_state)
    emit(next_state, "slot_assignment", {
        manifest_id = assert(next_state.manifest_id),
        assignment_id = next_state.assignment_id,
        assignments = assignments,
    }, link_targets(next_state), actions)
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_set_ready(state, event)
    if type(event.ready) ~= "boolean" then
        return state, rejected("malformed", "readiness must be a boolean")
    end
    if state.phase ~= "assigned" and state.phase ~= "ready" then
        return state, rejected("invalid_phase", "readiness is closed in " .. state.phase)
    end
    local peer = local_peer(state)
    if event.ready then
        if peer.accepted_manifest_id ~= state.manifest_id then
            return state,
                rejected("identity_mismatch", "readiness requires accepting the exact manifest")
        end
        if not owns_slot(state, peer.peer_id) then
            return state,
                rejected("invalid_assignment", "readiness requires an owned set in this generation")
        end
    end
    if peer.ready == event.ready then
        return state, idempotent()
    end
    local actions = {}
    local next_state = copy_state(state)
    if next_state.role == "guest" then
        emit(next_state, "ready", {
            manifest_id = assert(next_state.manifest_id),
            -- Answer the generation this peer actually holds. If the host has
            -- since republished, the host refuses this answer rather than
            -- crediting it to ownership the peer never saw.
            assignment_id = assert(next_state.assignment_id),
            ready = event.ready,
        }, link_targets(next_state), actions)
    end
    local_peer(next_state).ready = event.ready
    refresh_ready_phase(next_state)
    return next_state, applied(actions)
end

---@param next_state CoordinatorState
---@param actions CoordinatorAction[]
local function emit_start(next_state, actions)
    local freeze = assert(next_state.freeze)
    emit(next_state, "start", {
        manifest_id = freeze.manifest_id,
        countdown_id = freeze.countdown_id,
        first_input_tick = freeze.first_input_tick,
    }, link_targets(next_state), actions)
    next_state.countdown_remaining = 0
    next_state.start_deadline = next_state.clock + coordinator.START_ACK_TIMEOUT_TICKS
    local pending = false
    for index = 2, #next_state.peers do
        if not next_state.peers[index].started then
            pending = true
        end
    end
    if not pending then
        next_state.phase = "running"
        next_state.start_deadline = nil
        actions[#actions + 1] = { kind = "start_match", freeze = freeze }
    end
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_begin_countdown(state, event)
    if state.role ~= "host" then
        return state, rejected("not_permitted", "only the host starts the countdown")
    end
    if state.phase ~= "ready" then
        return state, rejected("invalid_phase", "countdown requires every peer to be ready")
    end
    if
        not is_integer(event.remaining_ticks)
        or event.remaining_ticks < 0
        or event.remaining_ticks > protocol.MAX_COUNTDOWN_TICKS
    then
        return state, rejected("malformed", "countdown length is outside the protocol bound")
    end
    if
        not is_integer(event.first_input_tick)
        or event.first_input_tick < 0
        or event.first_input_tick > input_frame.MAX_TICK
    then
        return state, rejected("malformed", "the first input tick is outside the input bound")
    end
    local manifest = assert(state.manifest)
    local ownership_ok, err, code = validate_local_assignments(state, state.assignments)
    if not ownership_ok then
        return state, rejected(code or "invalid_assignment", err or "slot sources are incomplete")
    end
    local actions = {}
    local next_state = copy_state(state)
    local freeze = freeze_session(next_state, manifest, event.countdown_id, event.first_input_tick)
    next_state.countdown_remaining = event.remaining_ticks
    local ok, emit_err, emit_code = try_emit(next_state, "countdown", {
        manifest_id = freeze.manifest_id,
        countdown_id = freeze.countdown_id,
        remaining_ticks = event.remaining_ticks,
        first_input_tick = freeze.first_input_tick,
    }, link_targets(next_state), actions)
    if not ok then
        return state, rejected(emit_code or "malformed", emit_err or "invalid countdown")
    end
    next_state.phase = "countdown"
    if event.remaining_ticks == 0 then
        emit_start(next_state, actions)
    end
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@return CoordinatorState, CoordinatorOutcome
local function handle_tick(state)
    local next_state = copy_state(state)
    next_state.clock = state.clock + 1
    if state.phase == "terminal" then
        return next_state, applied()
    end
    local actions = {}
    if state.phase == "countdown" then
        if (next_state.countdown_remaining or 0) > 0 then
            next_state.countdown_remaining = next_state.countdown_remaining - 1
            if next_state.countdown_remaining == 0 and next_state.role == "host" then
                emit_start(next_state, actions)
            end
        elseif
            next_state.role == "host"
            and next_state.start_deadline
            and next_state.clock > next_state.start_deadline
        then
            local missing = nil
            for index = 2, #next_state.peers do
                if not next_state.peers[index].started then
                    missing = next_state.peers[index].peer_id
                    break
                end
            end
            return terminate_session(next_state, {
                reason = "start_ack_timeout",
                origin = "timeout",
                peer_id = missing,
                detail = "a peer never acknowledged the canonical start boundary",
                announce = true,
            }, actions),
                applied(actions)
        end
    end
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_match_phase(state, event)
    if state.role ~= "host" then
        return state, rejected("not_permitted", "only the host publishes match phases")
    end
    if state.phase ~= "running" then
        return state, rejected("invalid_phase", "match phases require a running session")
    end
    local body = {
        phase = event.phase,
        tick = event.tick,
        home_score = event.home_score,
        away_score = event.away_score,
    }
    local match = state.match
    if match then
        if
            match.phase == body.phase
            and match.tick == body.tick
            and match.home_score == body.home_score
            and match.away_score == body.away_score
        then
            return state, idempotent()
        end
        if not MATCH_PHASE_NEXT[match.phase][body.phase] then
            return state,
                rejected(
                    "invalid_phase",
                    ("match phase %s cannot follow %s"):format(
                        tostring(body.phase),
                        tostring(match.phase)
                    )
                )
        end
        if not is_integer(body.tick) or body.tick < match.tick then
            return state, rejected("malformed", "simulation ticks never move backwards")
        end
        if
            not is_integer(body.home_score)
            or not is_integer(body.away_score)
            or body.home_score < match.home_score
            or body.away_score < match.away_score
        then
            return state, rejected("malformed", "simulation scores never move backwards")
        end
        if
            body.phase == "goal_stoppage"
            and body.home_score + body.away_score <= match.home_score + match.away_score
        then
            return state, rejected("malformed", "a goal stoppage must follow a scored goal")
        end
    elseif body.phase ~= "kickoff" then
        return state, rejected("invalid_phase", "a running match opens with kickoff")
    end
    local actions = {}
    local next_state = copy_state(state)
    local ok, err, code =
        try_emit(next_state, "match_phase", body, link_targets(next_state), actions)
    if not ok then
        return state, rejected(code or "malformed", err or "invalid match phase report")
    end
    next_state.match = {
        phase = body.phase,
        tick = body.tick,
        home_score = body.home_score,
        away_score = body.away_score,
    }
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_hash_report(state, event)
    if state.phase ~= "running" then
        return state, rejected("invalid_phase", "boundary hashes require a running session")
    end
    local existing = local_hash(state, event.tick)
    if existing then
        if existing == event.boundary_hash then
            return state, idempotent()
        end
        return state, rejected("duplicate", "a local boundary tick reported two different hashes")
    end
    local actions = {}
    local next_state = copy_state(state)
    local ok, err, code = try_emit(next_state, "hash_report", {
        tick = event.tick,
        boundary_hash = event.boundary_hash,
    }, link_targets(next_state), actions)
    if not ok then
        return state, rejected(code or "malformed", err or "invalid boundary hash report")
    end
    record_hash(next_state, event.tick, event.boundary_hash)
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_finish(state, event)
    ---@type CoordinatorResult
    local result = {
        final_tick = event.final_tick,
        home_score = event.home_score,
        away_score = event.away_score,
        final_hash = event.final_hash,
    }
    if state.role == "guest" then
        if state.phase ~= "running" and state.phase ~= "result" then
            return state, rejected("invalid_phase", "a result requires a running session")
        end
        local next_state = copy_state(state)
        next_state.local_result = result
        return next_state, applied()
    end
    if state.phase ~= "running" then
        return state, rejected("invalid_phase", "a result requires a running session")
    end
    local match = state.match
    if not match or match.phase ~= "full_time" then
        return state, rejected("invalid_phase", "the simulation must reach full time first")
    end
    if result.home_score ~= match.home_score or result.away_score ~= match.away_score then
        return state, rejected("identity_mismatch", "the coordinator never restates the score")
    end
    if not is_integer(result.final_tick) or result.final_tick < match.tick then
        return state, rejected("malformed", "the final tick precedes full time")
    end
    local actions = {}
    local next_state = copy_state(state)
    next_state.phase = "result"
    next_state.result = result
    next_state.local_result = result
    local targets = link_targets(next_state)
    local ok, err, code = try_emit(next_state, "match_phase", {
        phase = "result",
        tick = result.final_tick,
        home_score = result.home_score,
        away_score = result.away_score,
    }, targets, actions)
    if ok then
        ok, err, code = try_emit(next_state, "result_ack", {
            final_tick = result.final_tick,
            home_score = result.home_score,
            away_score = result.away_score,
            final_hash = result.final_hash,
        }, targets, actions)
    end
    if not ok then
        return state, rejected(code or "malformed", err or "invalid simulation result")
    end
    next_state.match = {
        phase = "result",
        tick = result.final_tick,
        home_score = result.home_score,
        away_score = result.away_score,
    }
    local_peer(next_state).result_acked = true
    if #targets == 0 then
        return terminate_session(next_state, { reason = "completed", origin = "local" }, actions),
            applied(actions)
    end
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_netcode_failure(state, event)
    local reason = coordinator.NETCODE_REASONS[event.failure]
    if not reason then
        return state, rejected("malformed", "unknown netcode failure class")
    end
    return terminate_from(state, {
        reason = reason,
        origin = "local",
        peer_id = event.peer_id,
        detail = event.detail,
        announce = true,
    })
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_abort(state, event)
    local code = event.code or "host_abort"
    return terminate_from(state, {
        reason = "local_abort",
        code = code,
        origin = "local",
        detail = event.detail,
        announce = true,
    })
end

---@param state CoordinatorState
---@return CoordinatorState, CoordinatorOutcome
local function handle_leave(state)
    if state.role ~= "guest" then
        return state, rejected("not_permitted", "the host ends a session with abort")
    end
    local actions = {}
    local next_state = copy_state(state)
    if state.phase ~= "new" then
        emit(next_state, "disconnect", {
            target_peer_id = next_state.peer_id,
            code = "peer_left",
        }, link_targets(next_state), actions)
    end
    return terminate_session(next_state, {
        reason = "guest_left",
        origin = "local",
        peer_id = next_state.peer_id,
    }, actions),
        applied(actions)
end

---@param next_state CoordinatorState
---@param peer_id string
local function remove_peer(next_state, peer_id)
    for index = #next_state.peers, 2, -1 do
        if next_state.peers[index].peer_id == peer_id then
            table.remove(next_state.peers, index)
        end
    end
end

-- A pre-countdown departure invalidates any published ownership, so the host
-- drops back to the manifest phase and republishes rather than silently
-- running a slot with no declared source.
---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param code SessionDisconnectCode
---@return CoordinatorState, CoordinatorOutcome
local function drop_guest(state, peer, code)
    local actions = {}
    local next_state = copy_state(state)
    local departed = peer.peer_id
    -- Announce to every current link, including the departing one, so a peer
    -- that is being removed learns its own stable reason before teardown.
    local targets = link_targets(next_state)
    emit(next_state, "disconnect", {
        target_peer_id = departed,
        code = code,
    }, targets, actions)
    remove_peer(next_state, departed)
    clear_readiness(next_state)
    if owns_slot(state, departed) then
        -- The published ownership named a peer that is gone, so it is void
        -- until the host republishes; the phase stays configurable. Remaining
        -- peers may still have readiness in flight for it, which `apply_ready`
        -- refuses on the ownership check.
        next_state.assignments = nil
        next_state.assignment_id = nil
    end
    actions[#actions + 1] = { kind = "close", link_id = assert(peer.link_id) }
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_link_lost(state, event)
    local code = event.code or "transport_lost"
    if not DISCONNECT_REASONS[code] then
        return state, rejected("malformed", "unknown transport disconnect code")
    end
    local peer = peer_by_link(state, event.link_id)
    if not peer then
        return state, rejected("unknown_link", "no admitted peer owns that link")
    end
    if peer.role == "host" then
        return terminate_from(state, {
            reason = code == "transport_lost" and "transport_lost" or "host_left",
            origin = "remote",
            peer_id = peer.peer_id,
        })
    end
    if state.freeze then
        return terminate_from(state, {
            reason = code == "transport_lost" and "transport_lost" or "guest_left",
            origin = "remote",
            peer_id = peer.peer_id,
            announce = true,
            exclude_link = peer.link_id,
        })
    end
    return drop_guest(state, peer, code)
end

-- ---------------------------------------------------------------------------
-- Control message handlers
-- ---------------------------------------------------------------------------

-- A link that never became a peer is refused on its own: the rest of the
-- session keeps its lifecycle, roster, readiness, and manifest untouched.
---@param state CoordinatorState
---@param link_id string
---@param code SessionRejectCode
---@param reason string
---@param actions CoordinatorAction[]
---@return CoordinatorState, CoordinatorOutcome
local function reject_link(state, link_id, code, reason, actions)
    local next_state = copy_state(state)
    emit(next_state, "abort", { code = code }, { link_id }, actions)
    actions[#actions + 1] = { kind = "close", link_id = link_id }
    return next_state,
        {
            accepted = false,
            disposition = "rejected",
            code = code,
            reason = reason,
            actions = actions,
        }
end

---@param state CoordinatorState
---@param link_id string
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function admit_guest(state, link_id, message)
    local body = message.body
    ---@cast body SessionHandshakeBody
    if state.role ~= "host" then
        return reject_link(state, link_id, "protocol_mismatch", "a guest never admits peers", {})
    end
    if state.phase ~= "new" and state.phase ~= "handshake" then
        return reject_link(
            state,
            link_id,
            "invalid_phase",
            "admission closes when the manifest is proposed",
            {}
        )
    end
    if body.role ~= "guest" then
        return reject_link(state, link_id, "protocol_mismatch", "a session has one host", {})
    end
    if peer_by_id(state, message.peer_id) then
        return reject_link(state, link_id, "protocol_mismatch", "peer id is already admitted", {})
    end
    if #state.peers >= coordinator.MAX_PEERS then
        return reject_link(state, link_id, "capacity", "the session is full", {})
    end
    local compatible, compat_err = protocol.compare_runtime(state.runtime, body.runtime)
    if not compatible then
        return reject_link(state, link_id, "runtime_mismatch", compat_err or "runtime mismatch", {})
    end
    local next_state = copy_state(state)
    next_state.peers[#next_state.peers + 1] = {
        peer_id = message.peer_id,
        link_id = link_id,
        role = "guest",
        runtime = copy_value(body.runtime),
        accepted_manifest_id = nil,
        assigned = false,
        ready = false,
        started = false,
        result_acked = false,
        hash_mismatches = 0,
        last_sequence = message.sequence,
        window = { { sequence = message.sequence, wire = assert(protocol.encode(message)) } },
    }
    next_state.phase = "handshake"
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_manifest_proposal(state, peer, message)
    local body = message.body
    ---@cast body SessionManifestProposalBody
    if state.manifest_id then
        if state.manifest_id == body.manifest_id then
            return state, idempotent()
        end
        return terminate_from(state, {
            reason = "manifest_mismatch",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "the manifest is immutable after proposal",
            announce = true,
        })
    end
    local difference = coordinator.expectation_difference(state.expectation, body.manifest)
    if difference then
        return terminate_from(state, {
            reason = "manifest_mismatch",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "local identity differs at " .. difference.path,
            announce = true,
        })
    end
    local actions = {}
    local next_state = copy_state(state)
    next_state.manifest = copy_value(body.manifest)
    next_state.manifest_id = body.manifest_id
    next_state.phase = "manifest"
    local_peer(next_state).accepted_manifest_id = body.manifest_id
    emit(
        next_state,
        "manifest_accept",
        { manifest_id = body.manifest_id },
        link_targets(next_state),
        actions
    )
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_manifest_accept(state, peer, message)
    local body = message.body
    ---@cast body SessionManifestAcceptBody
    if body.manifest_id ~= state.manifest_id then
        return drop_guest(state, peer, "protocol_error")
    end
    if peer.accepted_manifest_id == body.manifest_id then
        return state, idempotent()
    end
    local next_state = copy_state(state)
    assert(peer_by_id(next_state, peer.peer_id)).accepted_manifest_id = body.manifest_id
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_peer_assignment(state, peer, message)
    local body = message.body
    ---@cast body SessionPeerAssignmentBody
    if body.assigned_peer_id ~= state.peer_id or body.role ~= state.role then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "the host named a different peer identity",
            announce = true,
        })
    end
    if local_peer(state).assigned then
        return state, idempotent()
    end
    local next_state = copy_state(state)
    local_peer(next_state).assigned = true
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_slot_assignment(state, peer, message)
    local body = message.body
    ---@cast body SessionSlotAssignmentBody
    local manifest = state.manifest
    if not manifest or body.manifest_id ~= state.manifest_id then
        return terminate_from(state, {
            reason = "manifest_mismatch",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "slot assignment names a different manifest",
            announce = true,
        })
    end
    local sources, source_err = coordinator.slot_sources(manifest, body.assignments)
    if not sources then
        return terminate_from(state, {
            reason = "invalid_assignment",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = source_err,
            announce = true,
        })
    end
    local owned = false
    for _, producer in ipairs(body.assignments) do
        if producer.producer_kind == "peer" and producer.producer_id == state.peer_id then
            owned = true
        end
    end
    if not owned then
        return terminate_from(state, {
            reason = "invalid_assignment",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "the published ownership seats no slot for this peer",
            announce = true,
        })
    end
    if
        body.assignment_id == state.assignment_id
        and assignments_equal(state.assignments, body.assignments)
    then
        return state, idempotent()
    end
    local next_state = copy_state(state)
    next_state.assignments = copy_value(body.assignments)
    -- The generation token is opaque here: only the host holds the epoch that
    -- produced it. The guest stores it and names it in every readiness answer,
    -- which is what lets the host bind that answer to this exact ownership.
    next_state.assignment_id = body.assignment_id
    next_state.phase = "assigned"
    clear_readiness(next_state)
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_ready(state, peer, message)
    local body = message.body
    ---@cast body SessionReadyBody
    if body.manifest_id ~= state.manifest_id then
        return terminate_from(state, {
            reason = "manifest_mismatch",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "readiness names a different manifest",
            announce = true,
        })
    end
    if body.ready and peer.accepted_manifest_id ~= state.manifest_id then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "readiness preceded manifest acceptance",
            announce = true,
        })
    end
    -- Live, and load-bearing: `drop_guest` voids ownership for every remaining
    -- peer when a *different* peer departs before the countdown, so a peer that
    -- legitimately owned a slot a moment ago can own none now, and its
    -- in-flight readiness arrives here. Do not prune this as unreachable.
    if body.ready and not owns_slot(state, peer.peer_id) then
        return state, rejected("invalid_assignment", "readiness for voided slot ownership")
    end
    -- The ownership generation is named on the wire, never inferred from
    -- arrival order or from the polarity of the last message seen. Inference
    -- cannot survive two republishes racing one readiness answer; an exact
    -- identity match can, and it is equally exact for a negative answer.
    if body.assignment_id ~= state.assignment_id then
        return state, rejected("invalid_assignment", coordinator.STALE_GENERATION_REASON)
    end
    if peer.ready == body.ready then
        return state, idempotent()
    end
    local next_state = copy_state(state)
    assert(peer_by_id(next_state, peer.peer_id)).ready = body.ready
    refresh_ready_phase(next_state)
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_countdown(state, peer, message)
    local body = message.body
    ---@cast body SessionCountdownBody
    if body.manifest_id ~= state.manifest_id then
        return terminate_from(state, {
            reason = "manifest_mismatch",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "countdown names a different manifest",
            announce = true,
        })
    end
    if state.freeze then
        if state.freeze.countdown_id == body.countdown_id then
            return state, idempotent()
        end
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "a frozen countdown cannot be restarted",
            announce = true,
        })
    end
    -- A countdown that precedes local readiness needs no check here: a guest is
    -- in the `ready` phase exactly when it is ready, and #161 admits `countdown`
    -- only during `ready` or `countdown`, so the phase gate has already refused
    -- it before this handler runs.
    local next_state = copy_state(state)
    freeze_session(
        next_state,
        assert(next_state.manifest),
        body.countdown_id,
        body.first_input_tick
    )
    next_state.countdown_remaining = body.remaining_ticks
    next_state.phase = "countdown"
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_start(state, peer, message)
    local body = message.body
    ---@cast body SessionStartBody
    local freeze = state.freeze
    if
        not freeze
        or body.manifest_id ~= freeze.manifest_id
        or body.countdown_id ~= freeze.countdown_id
        or body.first_input_tick ~= freeze.first_input_tick
    then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "start does not name the frozen countdown boundary",
            announce = true,
        })
    end
    if state.role == "guest" then
        if local_peer(state).started then
            return state, idempotent()
        end
        local actions = {}
        local next_state = copy_state(state)
        emit(next_state, "start", {
            manifest_id = freeze.manifest_id,
            countdown_id = freeze.countdown_id,
            first_input_tick = freeze.first_input_tick,
        }, link_targets(next_state), actions)
        local_peer(next_state).started = true
        next_state.countdown_remaining = 0
        next_state.phase = "running"
        actions[#actions + 1] = { kind = "start_match", freeze = freeze }
        return next_state, applied(actions)
    end
    if peer.started then
        return state, idempotent()
    end
    local actions = {}
    local next_state = copy_state(state)
    assert(peer_by_id(next_state, peer.peer_id)).started = true
    local pending = false
    for index = 2, #next_state.peers do
        if not next_state.peers[index].started then
            pending = true
        end
    end
    if not pending and next_state.countdown_remaining == 0 then
        next_state.phase = "running"
        next_state.start_deadline = nil
        actions[#actions + 1] = { kind = "start_match", freeze = freeze }
    end
    return next_state, applied(actions)
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_match_phase(state, peer, message)
    local body = message.body
    ---@cast body SessionMatchPhaseBody
    local match = state.match
    if match then
        if
            match.phase == body.phase
            and match.tick == body.tick
            and match.home_score == body.home_score
            and match.away_score == body.away_score
        then
            return state, idempotent()
        end
        local allowed = MATCH_PHASE_NEXT[match.phase]
        local ordered = allowed[body.phase]
            or (body.phase == "result" and match.phase == "full_time")
        if not ordered or body.tick < match.tick then
            return terminate_from(state, {
                reason = "protocol_violation",
                origin = "remote",
                peer_id = peer.peer_id,
                detail = "match phase ordering regressed",
                announce = true,
            })
        end
    elseif body.phase ~= "kickoff" then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "a running match opens with kickoff",
            announce = true,
        })
    end
    local next_state = copy_state(state)
    next_state.match = {
        phase = body.phase,
        tick = body.tick,
        home_score = body.home_score,
        away_score = body.away_score,
    }
    -- Full time is the canonical running -> result boundary for a guest, so the
    -- host's later `result` body and `result_ack` land in a legal phase.
    if body.phase == "full_time" or body.phase == "result" then
        next_state.phase = "result"
    end
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_hash_report(state, peer, message)
    local body = message.body
    ---@cast body SessionHashReportBody
    local expected = local_hash(state, body.tick)
    if not expected then
        return state, applied()
    end
    local next_state = copy_state(state)
    local tracked = assert(peer_by_id(next_state, peer.peer_id))
    if expected == body.boundary_hash then
        tracked.hash_mismatches = 0
        return next_state, applied()
    end
    tracked.hash_mismatches = tracked.hash_mismatches + 1
    if tracked.hash_mismatches < coordinator.MAX_HASH_MISMATCHES then
        return next_state, applied()
    end
    local actions = {}
    return terminate_session(next_state, {
        reason = "hash_mismatch",
        origin = "remote",
        peer_id = peer.peer_id,
        detail = ("%d consecutive boundary hashes disagreed"):format(tracked.hash_mismatches),
        announce = true,
    }, actions),
        applied(actions)
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_result_ack(state, peer, message)
    local body = message.body
    ---@cast body SessionResultAckBody
    ---@type CoordinatorResult
    local reported = {
        final_tick = body.final_tick,
        home_score = body.home_score,
        away_score = body.away_score,
        final_hash = body.final_hash,
    }
    local known = state.result or state.local_result
    if known then
        if
            known.final_tick ~= reported.final_tick
            or known.home_score ~= reported.home_score
            or known.away_score ~= reported.away_score
            or known.final_hash ~= reported.final_hash
        then
            return terminate_from(state, {
                reason = "hash_mismatch",
                origin = "remote",
                peer_id = peer.peer_id,
                detail = "the acknowledged result differs from the simulation result",
                announce = true,
            })
        end
    end
    if state.role == "guest" then
        if local_peer(state).result_acked then
            return state, idempotent()
        end
        local actions = {}
        local next_state = copy_state(state)
        next_state.result = reported
        next_state.phase = "result"
        emit(next_state, "result_ack", {
            final_tick = reported.final_tick,
            home_score = reported.home_score,
            away_score = reported.away_score,
            final_hash = reported.final_hash,
        }, link_targets(next_state), actions)
        local_peer(next_state).result_acked = true
        return terminate_session(next_state, { reason = "completed", origin = "remote" }, actions),
            applied(actions)
    end
    if peer.result_acked then
        return state, idempotent()
    end
    local next_state = copy_state(state)
    assert(peer_by_id(next_state, peer.peer_id)).result_acked = true
    local pending = false
    for _, tracked in ipairs(next_state.peers) do
        if not tracked.result_acked then
            pending = true
        end
    end
    if not pending then
        local actions = {}
        return terminate_session(next_state, { reason = "completed", origin = "local" }, actions),
            applied(actions)
    end
    return next_state, applied()
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_abort(state, peer, message)
    local body = message.body
    ---@cast body SessionAbortBody
    if state.role == "host" and not state.freeze then
        return drop_guest(state, peer, "protocol_error")
    end
    return terminate_from(state, {
        reason = "peer_abort",
        code = body.code,
        origin = "remote",
        peer_id = peer.peer_id,
        announce = true,
        exclude_link = peer.link_id,
    })
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_disconnect(state, peer, message)
    local body = message.body
    ---@cast body SessionDisconnectBody
    local reason = DISCONNECT_REASONS[body.code]
    if state.role == "guest" then
        if body.target_peer_id ~= state.peer_id and body.code ~= "host_left" then
            -- Another peer left: the roster changed, so local readiness lapses
            -- exactly as it does on the host.
            if not local_peer(state).ready then
                return state, applied()
            end
            local next_state = copy_state(state)
            clear_readiness(next_state)
            return next_state, applied()
        end
        return terminate_from(state, {
            reason = body.code == "host_left" and "host_left" or "removed",
            origin = "remote",
            peer_id = peer.peer_id,
        })
    end
    if body.target_peer_id ~= peer.peer_id then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "a guest cannot disconnect another peer",
            announce = true,
        })
    end
    if state.freeze then
        return terminate_from(state, {
            reason = reason,
            origin = "remote",
            peer_id = peer.peer_id,
            announce = true,
            exclude_link = peer.link_id,
        })
    end
    return drop_guest(state, peer, body.code)
end

---@param state CoordinatorState
---@param peer CoordinatorPeer
---@param _message SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function apply_repeat_handshake(state, peer, _message)
    return terminate_from(state, {
        reason = "protocol_violation",
        origin = "remote",
        peer_id = peer.peer_id,
        detail = "an admitted link cannot handshake again",
        announce = true,
    })
end

---@type table<SessionMessageKind, fun(state: CoordinatorState, peer: CoordinatorPeer, message: SessionControlMessage): CoordinatorState, CoordinatorOutcome>
local MESSAGE_HANDLERS = {
    handshake = apply_repeat_handshake,
    manifest_proposal = apply_manifest_proposal,
    manifest_accept = apply_manifest_accept,
    peer_assignment = apply_peer_assignment,
    slot_assignment = apply_slot_assignment,
    ready = apply_ready,
    countdown = apply_countdown,
    start = apply_start,
    match_phase = apply_match_phase,
    hash_report = apply_hash_report,
    result_ack = apply_result_ack,
    abort = apply_abort,
    disconnect = apply_disconnect,
}

-- `start`, `hash_report`, and `result_ack` are deliberately absent: the host
-- publishes them and every peer echoes the identical body back as its
-- acknowledgement, so both directions are legal for those three kinds.
---@type table<SessionMessageKind, SessionRole>
local SENDER_ROLE = {
    handshake = "guest",
    manifest_proposal = "host",
    manifest_accept = "guest",
    peer_assignment = "host",
    slot_assignment = "host",
    ready = "guest",
    countdown = "host",
    match_phase = "host",
}

-- Transcript classification is a *conflict detector*, not a liveness gate.
--
-- The retained window bounds how far back a byte-for-byte comparison can be
-- made. Nothing in #161 bounds how late a reliable transport may retransmit, so
-- a genuine retransmission can always age out of any finite window. Ending the
-- session in that case would kill a healthy match over an ordinary loss-and-
-- retry, so an unprovable duplicate fails *open*: it is dropped without being
-- applied and without advancing anything. Dropping is strictly safer than
-- applying, and the case it might have caught -- a conflicting reuse of an old
-- identity -- is refused either way, because the message is never applied.
---@param peer CoordinatorPeer
---@param message SessionControlMessage
---@return "applied"|"idempotent"|"stale"?, string?
local function classify_transcript(peer, message)
    if message.sequence > peer.last_sequence then
        return "applied"
    end
    for index = #peer.window, 1, -1 do
        local record = peer.window[index]
        if record.sequence == message.sequence then
            local wire = protocol.encode(message)
            if wire == record.wire then
                return "idempotent"
            end
            return nil, "message id was reused with different canonical bytes"
        end
    end
    return "stale"
end

---@param next_state CoordinatorState
---@param peer_id string
---@param message SessionControlMessage
local function record_transcript(next_state, peer_id, message)
    local peer = peer_by_id(next_state, peer_id)
    if not peer then
        return
    end
    peer.last_sequence = message.sequence
    peer.window[#peer.window + 1] = {
        sequence = message.sequence,
        wire = assert(protocol.encode(message)),
    }
    while #peer.window > coordinator.DUPLICATE_WINDOW do
        table.remove(peer.window, 1)
    end
end

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
local function handle_control(state, event)
    if type(event.link_id) ~= "string" then
        return state, rejected("malformed", "a control message needs its transport link id")
    end
    local peer = peer_by_link(state, event.link_id)
    local message = event.message
    if message == nil and type(event.wire) == "string" then
        local decoded, decode_err, decode_code = protocol.decode(event.wire)
        if not decoded then
            if peer then
                return terminate_from(state, {
                    reason = "protocol_violation",
                    code = decode_code == "unsupported_version" and "protocol_mismatch"
                        or "malformed_message",
                    origin = "remote",
                    peer_id = peer.peer_id,
                    detail = decode_err,
                    announce = true,
                })
            end
            return reject_link(state, event.link_id, "malformed_message", decode_err or "", {})
        end
        message = decoded
    end
    local valid, valid_err, valid_code = protocol.validate(message)
    if not valid then
        if peer then
            return terminate_from(state, {
                reason = "protocol_violation",
                code = valid_code == "unsupported_version" and "protocol_mismatch"
                    or "malformed_message",
                origin = "remote",
                peer_id = peer.peer_id,
                detail = valid_err,
                announce = true,
            })
        end
        return reject_link(state, event.link_id, "malformed_message", valid_err or "", {})
    end
    ---@cast message SessionControlMessage
    if message.session_id ~= state.session_id then
        if peer then
            return terminate_from(state, {
                reason = "protocol_violation",
                origin = "remote",
                peer_id = peer.peer_id,
                detail = "control message names a different session",
                announce = true,
            })
        end
        return reject_link(state, event.link_id, "protocol_mismatch", "unknown session", {})
    end
    if not peer then
        if message.kind ~= "handshake" then
            return reject_link(
                state,
                event.link_id,
                "invalid_phase",
                "an unadmitted link may only handshake",
                {}
            )
        end
        return admit_guest(state, event.link_id, message)
    end
    if message.peer_id ~= peer.peer_id then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "control message claims another peer identity",
            announce = true,
        })
    end
    local expected_sender = SENDER_ROLE[message.kind]
    if expected_sender and expected_sender ~= peer.role then
        return terminate_from(state, {
            reason = "protocol_violation",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = message.kind .. " may only be sent by the " .. expected_sender,
            announce = true,
        })
    end
    local disposition, transcript_err = classify_transcript(peer, message)
    if not disposition then
        return terminate_from(state, {
            reason = "protocol_violation",
            code = "malformed_message",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = transcript_err,
            announce = true,
        })
    end
    if disposition == "idempotent" then
        return state, idempotent()
    end
    if disposition == "stale" then
        return state, stale(coordinator.STALE_DUPLICATE_REASON)
    end
    -- Republished ownership implicitly revokes readiness, so it is validated
    -- against `assigned`: #161 forbids slot assignment during `ready`, and the
    -- coordinator's answer to a configuration change is to leave `ready`.
    local phase = state.phase
    if message.kind == "slot_assignment" and phase == "ready" then
        phase = "assigned"
    end
    local phase_ok, phase_err = protocol.validate_phase(message, phase)
    if not phase_ok then
        return terminate_from(state, {
            reason = "protocol_violation",
            code = "invalid_phase",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = phase_err,
            announce = true,
        })
    end
    local handler = MESSAGE_HANDLERS[message.kind]
    if not handler then
        return terminate_from(state, {
            reason = "protocol_violation",
            code = "unsupported_message",
            origin = "remote",
            peer_id = peer.peer_id,
            detail = "no coordinator handler for " .. message.kind,
            announce = true,
        })
    end
    local next_state, outcome = handler(state, peer, message)
    if outcome.disposition == "rejected" then
        return next_state, outcome
    end
    if next_state == state then
        next_state = copy_state(state)
    end
    record_transcript(next_state, peer.peer_id, message)
    return next_state, outcome
end

---@type table<CoordinatorEventKind, fun(state: CoordinatorState, event: table): CoordinatorState, CoordinatorOutcome>
local EVENT_HANDLERS = {
    connect = handle_connect,
    control = handle_control,
    link_lost = handle_link_lost,
    propose_manifest = handle_propose_manifest,
    assign_slots = handle_assign_slots,
    set_ready = handle_set_ready,
    begin_countdown = handle_begin_countdown,
    tick = handle_tick,
    match_phase = handle_match_phase,
    hash_report = handle_hash_report,
    finish = handle_finish,
    netcode_failure = handle_netcode_failure,
    leave = handle_leave,
    abort = handle_abort,
}

---@param state CoordinatorState
---@param event table
---@return CoordinatorState, CoordinatorOutcome
function coordinator.step(state, event)
    assert(type(state) == "table" and PHASE_SET[state.phase], "coordinator state is invalid")
    if type(event) ~= "table" or type(event.kind) ~= "string" then
        return state, rejected("malformed", "a coordinator event needs a kind")
    end
    local handler = EVENT_HANDLERS[event.kind]
    if not handler then
        return state, rejected("unknown_message", "unknown coordinator event " .. event.kind)
    end
    if state.phase == "terminal" and event.kind ~= "tick" then
        return state, rejected("invalid_phase", "the session already ended")
    end
    return handler(state, event)
end

---@param state CoordinatorState
---@param link_id string
---@param wire string
---@return CoordinatorState, CoordinatorOutcome
function coordinator.receive(state, link_id, wire)
    return coordinator.step(state, { kind = "control", link_id = link_id, wire = wire })
end

return coordinator
