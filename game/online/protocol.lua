local fnv1a64 = require("core.fnv1a64")
local combat_snapshot = require("sim.combat_snapshot")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match_snapshot = require("sim.match_snapshot")

---@alias SessionProtocolErrorCode
---| "malformed"
---| "wire_too_large"
---| "unsupported_version"
---| "unknown_message"
---| "identity_mismatch"
---| "runtime_mismatch"
---| "invalid_phase"
---| "duplicate"
---| "transcript_conflict"
---@alias SessionMessageKind
---| "handshake"
---| "manifest_proposal"
---| "manifest_accept"
---| "peer_assignment"
---| "slot_assignment"
---| "ready"
---| "countdown"
---| "start"
---| "match_phase"
---| "hash_report"
---| "result_ack"
---| "abort"
---| "disconnect"
---@alias SessionRole "host"|"guest"
---@alias SessionTeam "home"|"away"
---@alias SessionProducerKind "peer"|"bot"
---@alias SessionLifecyclePhase
---| "new"
---| "handshake"
---| "manifest"
---| "assigned"
---| "ready"
---| "countdown"
---| "running"
---| "result"
---| "terminal"
---@alias SessionMatchPhase "kickoff"|"playing"|"goal_stoppage"|"full_time"|"result"
---@alias SessionCombatStatus "provisional_114"|"accepted_proceed"|"accepted_revision"
---@alias SessionRejectCode
---| "protocol_mismatch"
---| "runtime_mismatch"
---| "manifest_mismatch"
---| "capacity"
---| "invalid_assignment"
---| "invalid_phase"
---| "malformed_message"
---| "unsupported_message"
---| "host_abort"
---| "peer_disconnect"
---| "desync"
---@alias SessionDisconnectCode "peer_left"|"transport_lost"|"host_left"|"protocol_error"
---@alias SessionDuplicateDisposition "idempotent"|"reject"

---@class SessionRuntimeIdentity
---@field version integer
---@field runtime_id string
---@field runtime_revision string
---@field presentation_id string
---@field capabilities string[] -- Sorted, unique compatibility capability ids.

---@class SessionRosterPlayer
---@field player_id string
---@field position Position
---@field loadout_id string?
---@field family_id string?

---@class SessionTeamManifest
---@field team SessionTeam
---@field team_id string
---@field roster SessionRosterPlayer[] -- Five ordered fixture players, including one keeper.

---@class SessionManifestSlot
---@field slot InputSlotId
---@field team SessionTeam
---@field player_id string

---@class SessionManifest
---@field version integer
---@field session_id string
---@field protocol_version integer
---@field input_version integer
---@field snapshot_version integer
---@field tape_version integer
---@field combat_schema_version integer
---@field build_id string
---@field source_id string
---@field content_id string
---@field tuning_id string
---@field match_config_id string
---@field fixture_id string
---@field arena_id string
---@field combat_rules_id string
---@field gameplay_ai_policy_id string
---@field combat_status SessionCombatStatus
---@field seed integer
---@field tick_rate integer
---@field duration_ticks integer
---@field max_goals integer
---@field teams SessionTeamManifest[] -- Home, then away.
---@field slots SessionManifestSlot[] -- OMP-1 canonical slot order.

---@class SessionSlotProducer
---@field slot InputSlotId
---@field team SessionTeam
---@field player_id string
---@field producer_kind SessionProducerKind
---@field producer_id string -- Stable peer id or deterministic bot id.
---@field bot_seed integer?

---@class SessionHandshakeBody
---@field role SessionRole
---@field runtime SessionRuntimeIdentity

---@class SessionManifestProposalBody
---@field manifest_id string
---@field manifest SessionManifest

---@class SessionManifestAcceptBody
---@field manifest_id string

---@class SessionPeerAssignmentBody
---@field assigned_peer_id string
---@field role SessionRole

---@class SessionSlotAssignmentBody
---@field manifest_id string
---@field assignments SessionSlotProducer[]

---@class SessionReadyBody
---@field manifest_id string
---@field ready boolean

---@class SessionCountdownBody
---@field manifest_id string
---@field countdown_id string
---@field remaining_ticks integer
---@field first_input_tick integer

---@class SessionStartBody
---@field manifest_id string
---@field countdown_id string
---@field first_input_tick integer

---@class SessionMatchPhaseBody
---@field phase SessionMatchPhase
---@field tick integer
---@field home_score integer
---@field away_score integer

---@class SessionHashReportBody
---@field tick integer
---@field boundary_hash string

---@class SessionResultAckBody
---@field final_tick integer
---@field home_score integer
---@field away_score integer
---@field final_hash string

---@class SessionAbortBody
---@field code SessionRejectCode

---@class SessionDisconnectBody
---@field target_peer_id string
---@field code SessionDisconnectCode

---@alias SessionMessageBody
---| SessionHandshakeBody
---| SessionManifestProposalBody
---| SessionManifestAcceptBody
---| SessionPeerAssignmentBody
---| SessionSlotAssignmentBody
---| SessionReadyBody
---| SessionCountdownBody
---| SessionStartBody
---| SessionMatchPhaseBody
---| SessionHashReportBody
---| SessionResultAckBody
---| SessionAbortBody
---| SessionDisconnectBody

---@class SessionControlMessage
---@field version integer
---@field kind SessionMessageKind
---@field session_id string
---@field peer_id string
---@field sequence integer
---@field message_id string
---@field body SessionMessageBody

---@class SessionIdentityDifference
---@field path string
---@field expected any
---@field actual any

---@class SessionProtocolModule
local protocol = {}

protocol.VERSION = 1
protocol.MANIFEST_VERSION = 1
protocol.RUNTIME_IDENTITY_VERSION = 1
protocol.COMBAT_SCHEMA_VERSION = combat_snapshot.VERSION
protocol.MAX_WIRE_BYTES = 8192
protocol.MAX_ID_BYTES = 128
protocol.MAX_SESSION_ID_BYTES = 128
protocol.MAX_PEER_ID_BYTES = 128
protocol.MAX_MESSAGE_ID_BYTES = 284
protocol.MAX_SEQUENCE = 2147483647
protocol.MAX_SEED = 2147483647
protocol.MAX_DURATION_TICKS = 216000
protocol.MAX_GOALS = 99
protocol.MAX_COUNTDOWN_TICKS = 600
protocol.MAX_CAPABILITIES = 16

protocol.CURRENT_VERSIONS = {
    protocol = protocol.VERSION,
    input = input_frame.VERSION,
    snapshot = match_snapshot.COMBAT_VERSION,
    tape = input_tape.COMBAT_VERSION,
    combat = protocol.COMBAT_SCHEMA_VERSION,
}

local MESSAGE_FIELDS = {
    version = true,
    kind = true,
    session_id = true,
    peer_id = true,
    sequence = true,
    message_id = true,
    body = true,
}
local RUNTIME_FIELDS = {
    version = true,
    runtime_id = true,
    runtime_revision = true,
    presentation_id = true,
    capabilities = true,
}
local MANIFEST_FIELDS = {
    version = true,
    session_id = true,
    protocol_version = true,
    input_version = true,
    snapshot_version = true,
    tape_version = true,
    combat_schema_version = true,
    build_id = true,
    source_id = true,
    content_id = true,
    tuning_id = true,
    match_config_id = true,
    fixture_id = true,
    arena_id = true,
    combat_rules_id = true,
    gameplay_ai_policy_id = true,
    combat_status = true,
    seed = true,
    tick_rate = true,
    duration_ticks = true,
    max_goals = true,
    teams = true,
    slots = true,
}
local TEAM_FIELDS = { team = true, team_id = true, roster = true }
local ROSTER_PLAYER_FIELDS = {
    player_id = true,
    position = true,
    loadout_id = true,
    family_id = true,
}
local MANIFEST_SLOT_FIELDS = { slot = true, team = true, player_id = true }
local SLOT_PRODUCER_FIELDS = {
    slot = true,
    team = true,
    player_id = true,
    producer_kind = true,
    producer_id = true,
    bot_seed = true,
}
local BODY_FIELDS = {
    handshake = { role = true, runtime = true },
    manifest_proposal = { manifest_id = true, manifest = true },
    manifest_accept = { manifest_id = true },
    peer_assignment = { assigned_peer_id = true, role = true },
    slot_assignment = { manifest_id = true, assignments = true },
    ready = { manifest_id = true, ready = true },
    countdown = {
        manifest_id = true,
        countdown_id = true,
        remaining_ticks = true,
        first_input_tick = true,
    },
    start = { manifest_id = true, countdown_id = true, first_input_tick = true },
    match_phase = { phase = true, tick = true, home_score = true, away_score = true },
    hash_report = { tick = true, boundary_hash = true },
    result_ack = {
        final_tick = true,
        home_score = true,
        away_score = true,
        final_hash = true,
    },
    abort = { code = true },
    disconnect = { target_peer_id = true, code = true },
}
local ROLES = { host = true, guest = true }
local POSITIONS = { keeper = true, defender = true, midfielder = true, forward = true }
local COMBAT_STATUSES = {
    provisional_114 = true,
    accepted_proceed = true,
    accepted_revision = true,
}
local MATCH_PHASES = {
    kickoff = true,
    playing = true,
    goal_stoppage = true,
    full_time = true,
    result = true,
}
local REJECT_CODES = {
    protocol_mismatch = true,
    runtime_mismatch = true,
    manifest_mismatch = true,
    capacity = true,
    invalid_assignment = true,
    invalid_phase = true,
    malformed_message = true,
    unsupported_message = true,
    host_abort = true,
    peer_disconnect = true,
    desync = true,
}
local DISCONNECT_CODES = {
    peer_left = true,
    transport_lost = true,
    host_left = true,
    protocol_error = true,
}
local ALLOWED_PHASES = {
    handshake = { new = true, handshake = true },
    manifest_proposal = { handshake = true, manifest = true },
    manifest_accept = { manifest = true },
    peer_assignment = { manifest = true, assigned = true },
    slot_assignment = { manifest = true, assigned = true },
    ready = { assigned = true, ready = true },
    countdown = { ready = true, countdown = true },
    start = { countdown = true },
    match_phase = { running = true, result = true },
    hash_report = { running = true },
    result_ack = { result = true },
    abort = {
        new = true,
        handshake = true,
        manifest = true,
        assigned = true,
        ready = true,
        countdown = true,
        running = true,
        result = true,
    },
    disconnect = {
        new = true,
        handshake = true,
        manifest = true,
        assigned = true,
        ready = true,
        countdown = true,
        running = true,
        result = true,
    },
}
local MATCH_PHASES_BY_LIFECYCLE = {
    running = {
        kickoff = true,
        playing = true,
        goal_stoppage = true,
        full_time = true,
    },
    result = { result = true },
}
local MANIFEST_COMPARE_FIELDS = {
    "version",
    "session_id",
    "protocol_version",
    "input_version",
    "snapshot_version",
    "tape_version",
    "combat_schema_version",
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
    "seed",
    "tick_rate",
    "duration_ticks",
    "max_goals",
}
local RUNTIME_COMPARE_FIELDS = {
    "version",
    "runtime_id",
    "runtime_revision",
    "presentation_id",
}
local MESSAGE_ID_PREFIX = "GCMI;1;"

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param value string
---@return boolean
local function is_canonical_unsigned(value)
    return value == "0" or value:match("^[1-9]%d*$") ~= nil
end

---@param value string
---@return boolean
local function is_canonical_integer(value)
    return is_canonical_unsigned(value) or value:match("^%-[1-9]%d*$") ~= nil
end

---@param value any
---@param allowed table<string, boolean>
---@return boolean
local function has_only_fields(value, allowed)
    if type(value) ~= "table" then
        return false
    end
    for key in pairs(value) do
        if type(key) ~= "string" or not allowed[key] then
            return false
        end
    end
    return true
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

---@param value any
---@param count integer?
---@param maximum integer?
---@return boolean
local function is_array(value, count, maximum)
    if type(value) ~= "table" then
        return false
    end
    local length = 0
    local maximum_index = 0
    for key in pairs(value) do
        if not is_integer(key) or key < 1 then
            return false
        end
        length = length + 1
        maximum_index = math.max(maximum_index, key)
    end
    if maximum_index ~= length then
        return false
    end
    if (count and length ~= count) or (maximum and length > maximum) then
        return false
    end
    return true
end

---@param code SessionProtocolErrorCode
---@param message string
---@return nil, string, SessionProtocolErrorCode
local function failure(code, message)
    return nil, message, code
end

---@param value any
---@param name string
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_id(value, name)
    if
        type(value) ~= "string"
        or value == ""
        or #value > protocol.MAX_ID_BYTES
        or not value:match("^[A-Za-z0-9][A-Za-z0-9_%.%-]*$")
    then
        return failure("malformed", name .. " must be a bounded opaque ASCII id")
    end
    return true
end

---@param value any
---@param name string
---@param maximum integer
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_identity_component(value, name, maximum)
    if
        type(value) ~= "string"
        or value == ""
        or #value > maximum
        or not value:match("^[A-Za-z0-9][A-Za-z0-9_%.%-]*$")
    then
        return failure("malformed", name .. " must be a bounded opaque ASCII component")
    end
    return true
end

---@param value any
---@param name string
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_player_id(value, name)
    return validate_identity_component(value, name, input_frame.MAX_PLAYER_ID_BYTES)
end

---@param value any
---@param name string
---@param minimum integer
---@param maximum integer
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_integer(value, name, minimum, maximum)
    if not is_integer(value) or value < minimum or value > maximum then
        return failure("malformed", name .. " must be a bounded integer")
    end
    return true
end

---@param value string
---@return string
local function length_prefix(value)
    return tostring(#value) .. ":" .. value
end

---@param session_id string
---@param peer_id string
---@param sequence integer
---@return string?, string?, SessionProtocolErrorCode?
function protocol.message_id(session_id, peer_id, sequence)
    local ok, err, code =
        validate_identity_component(session_id, "message.session_id", protocol.MAX_SESSION_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code =
        validate_identity_component(peer_id, "message.peer_id", protocol.MAX_PEER_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_integer(sequence, "message.sequence", 0, protocol.MAX_SEQUENCE)
    if not ok then
        return nil, err, code
    end
    local sequence_text = tostring(sequence)
    local message_id = MESSAGE_ID_PREFIX
        .. length_prefix(session_id)
        .. length_prefix(peer_id)
        .. length_prefix(sequence_text)
    assert(#message_id <= protocol.MAX_MESSAGE_ID_BYTES, "message id bound is inconsistent")
    return message_id
end

---@param value any
---@param name string
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_hash(value, name)
    if type(value) ~= "string" or #value ~= 16 or not value:match("^[0-9a-f]+$") then
        return failure("malformed", name .. " must be a 16-character lowercase hash")
    end
    return true
end

---@param runtime any
---@return boolean?, string?, SessionProtocolErrorCode?
function protocol.validate_runtime(runtime)
    if not has_only_fields(runtime, RUNTIME_FIELDS) then
        return failure("malformed", "runtime identity must contain only canonical fields")
    end
    if not is_integer(runtime.version) then
        return failure("malformed", "runtime identity version must be an integer")
    end
    if runtime.version ~= protocol.RUNTIME_IDENTITY_VERSION then
        return failure("unsupported_version", "unsupported runtime identity version")
    end
    for _, field in ipairs({ "runtime_id", "runtime_revision", "presentation_id" }) do
        local ok, err, code = validate_id(runtime[field], "runtime." .. field)
        if not ok then
            return nil, err, code
        end
    end
    if not is_array(runtime.capabilities, nil, protocol.MAX_CAPABILITIES) then
        return failure("malformed", "runtime capabilities must be a bounded canonical array")
    end
    local previous = nil
    for index, capability in ipairs(runtime.capabilities) do
        local ok, err, code = validate_id(capability, "runtime capability")
        if not ok then
            return nil, err, code
        end
        if previous and capability <= previous then
            return failure("malformed", "runtime capabilities must be sorted and unique")
        end
        previous = capability
    end
    return true
end

---@param runtime SessionRuntimeIdentity
---@return SessionRuntimeIdentity?, string?, SessionProtocolErrorCode?
function protocol.new_runtime(runtime)
    local ok, err, code = protocol.validate_runtime(runtime)
    if not ok then
        return nil, err, code
    end
    return copy_value(runtime)
end

---@param roster any
---@param team SessionTeam
---@param seen_players table<string, boolean>
---@return table<string, SessionRosterPlayer>?, string?, SessionProtocolErrorCode?
local function validate_roster(roster, team, seen_players)
    if not is_array(roster, input_frame.FIXTURE_TEAM_SIZE) then
        return failure("malformed", team .. " roster must contain exactly five players")
    end
    local players = {}
    local keeper_count = 0
    for index, player in ipairs(roster) do
        if not has_only_fields(player, ROSTER_PLAYER_FIELDS) then
            return failure("malformed", team .. " roster player has unknown fields")
        end
        local ok, err, code = validate_player_id(player.player_id, team .. " roster player id")
        if not ok then
            return nil, err, code
        end
        if not POSITIONS[player.position] then
            return failure("malformed", team .. " roster player position is invalid")
        end
        if seen_players[player.player_id] then
            return failure("malformed", "manifest player ids must be unique")
        end
        seen_players[player.player_id] = true
        players[player.player_id] = player
        if player.position == "keeper" then
            keeper_count = keeper_count + 1
            if player.loadout_id ~= nil or player.family_id ~= nil then
                return failure("malformed", "keepers cannot have combat loadouts")
            end
        else
            for _, field in ipairs({ "loadout_id", "family_id" }) do
                ok, err, code = validate_id(player[field], team .. " roster " .. field)
                if not ok then
                    return nil, err, code
                end
            end
        end
        if index == 1 and player.position ~= "keeper" then
            return failure("malformed", team .. " roster must place its keeper first")
        end
    end
    if keeper_count ~= 1 then
        return failure("malformed", team .. " roster must contain exactly one keeper")
    end
    return players
end

---@param manifest any
---@return boolean?, string?, SessionProtocolErrorCode?
function protocol.validate_manifest(manifest)
    if not has_only_fields(manifest, MANIFEST_FIELDS) then
        return failure("malformed", "session manifest must contain only canonical fields")
    end
    local versions = {
        { "version", protocol.MANIFEST_VERSION },
        { "protocol_version", protocol.VERSION },
        { "input_version", input_frame.VERSION },
        { "snapshot_version", match_snapshot.COMBAT_VERSION },
        { "tape_version", input_tape.COMBAT_VERSION },
        { "combat_schema_version", protocol.COMBAT_SCHEMA_VERSION },
    }
    for _, pair in ipairs(versions) do
        if not is_integer(manifest[pair[1]]) then
            return failure("malformed", "manifest " .. pair[1] .. " must be an integer")
        end
        if manifest[pair[1]] ~= pair[2] then
            return failure("unsupported_version", "unsupported manifest " .. pair[1])
        end
    end
    for _, field in ipairs({
        "session_id",
        "build_id",
        "source_id",
        "content_id",
        "tuning_id",
        "match_config_id",
        "fixture_id",
        "arena_id",
        "combat_rules_id",
        "gameplay_ai_policy_id",
    }) do
        local ok, err, code = validate_id(manifest[field], "manifest." .. field)
        if not ok then
            return nil, err, code
        end
    end
    if not COMBAT_STATUSES[manifest.combat_status] then
        return failure("malformed", "manifest combat status is invalid")
    end
    local integer_fields = {
        { "seed", 0, protocol.MAX_SEED },
        { "tick_rate", fixed_clock.TICK_RATE, fixed_clock.TICK_RATE },
        { "duration_ticks", 1, protocol.MAX_DURATION_TICKS },
        { "max_goals", 1, protocol.MAX_GOALS },
    }
    for _, definition in ipairs(integer_fields) do
        local ok, err, code = validate_integer(
            manifest[definition[1]],
            "manifest." .. definition[1],
            definition[2],
            definition[3]
        )
        if not ok then
            return nil, err, code
        end
    end
    if not is_array(manifest.teams, 2) then
        return failure("malformed", "manifest teams must contain home then away")
    end
    local all_players = {}
    local players_by_team = { home = {}, away = {} }
    for index, expected_team in ipairs({ "home", "away" }) do
        local team = manifest.teams[index]
        if not has_only_fields(team, TEAM_FIELDS) or team.team ~= expected_team then
            return failure("malformed", "manifest teams violate canonical home/away order")
        end
        local ok, err, code = validate_id(team.team_id, expected_team .. " team id")
        if not ok then
            return nil, err, code
        end
        local roster
        roster, err, code = validate_roster(team.roster, expected_team, all_players)
        if not roster then
            return nil, err, code
        end
        players_by_team[expected_team] = roster
    end
    if manifest.teams[1].team_id == manifest.teams[2].team_id then
        return failure("malformed", "manifest teams must have distinct ids")
    end
    if not is_array(manifest.slots, input_frame.SLOT_COUNT) then
        return failure("malformed", "manifest must contain exactly eight canonical slots")
    end
    local assigned_players = {}
    for index, assignment in ipairs(manifest.slots) do
        local slot = assert(input_frame.slot(index))
        if
            not has_only_fields(assignment, MANIFEST_SLOT_FIELDS)
            or assignment.slot ~= slot.id
            or assignment.team ~= slot.team
        then
            return failure("malformed", "manifest slots violate OMP-1 canonical order")
        end
        local ok, err, code = validate_player_id(assignment.player_id, "manifest slot player id")
        if not ok then
            return nil, err, code
        end
        local player = players_by_team[slot.team][assignment.player_id]
        if not player or player.position == "keeper" then
            return failure("malformed", "manifest slot must name an outfielder on its team")
        end
        if assigned_players[assignment.player_id] then
            return failure("malformed", "manifest outfielder owns multiple slots")
        end
        assigned_players[assignment.player_id] = true
    end
    for _, team in ipairs({ "home", "away" }) do
        for _, player in pairs(players_by_team[team]) do
            if player.position ~= "keeper" and not assigned_players[player.player_id] then
                return failure("malformed", "manifest outfielder is missing a canonical slot")
            end
        end
    end
    return true
end

---@param manifest SessionManifest
---@return SessionManifest?, string?, SessionProtocolErrorCode?
function protocol.new_manifest(manifest)
    local ok, err, code = protocol.validate_manifest(manifest)
    if not ok then
        return nil, err, code
    end
    return copy_value(manifest)
end

---@param path string
---@param expected any
---@param actual any
---@return SessionIdentityDifference
local function difference(path, expected, actual)
    return { path = path, expected = expected, actual = actual }
end

---@param expected SessionRuntimeIdentity
---@param actual SessionRuntimeIdentity
---@return SessionIdentityDifference?
function protocol.runtime_difference(expected, actual)
    local left_ok, left_err = protocol.validate_runtime(expected)
    assert(left_ok, left_err)
    local right_ok, right_err = protocol.validate_runtime(actual)
    assert(right_ok, right_err)
    for _, field in ipairs(RUNTIME_COMPARE_FIELDS) do
        if expected[field] ~= actual[field] then
            return difference("runtime." .. field, expected[field], actual[field])
        end
    end
    if #expected.capabilities ~= #actual.capabilities then
        return difference(
            "runtime.capabilities.length",
            #expected.capabilities,
            #actual.capabilities
        )
    end
    for index = 1, #expected.capabilities do
        if expected.capabilities[index] ~= actual.capabilities[index] then
            return difference(
                "runtime.capabilities." .. index,
                expected.capabilities[index],
                actual.capabilities[index]
            )
        end
    end
    return nil
end

---@param expected SessionManifest
---@param actual SessionManifest
---@return SessionIdentityDifference?
function protocol.manifest_difference(expected, actual)
    local left_ok, left_err = protocol.validate_manifest(expected)
    assert(left_ok, left_err)
    local right_ok, right_err = protocol.validate_manifest(actual)
    assert(right_ok, right_err)
    for _, field in ipairs(MANIFEST_COMPARE_FIELDS) do
        if expected[field] ~= actual[field] then
            return difference("manifest." .. field, expected[field], actual[field])
        end
    end
    for team_index = 1, 2 do
        local left_team = expected.teams[team_index]
        local right_team = actual.teams[team_index]
        for _, field in ipairs({ "team", "team_id" }) do
            if left_team[field] ~= right_team[field] then
                return difference(
                    ("manifest.teams.%d.%s"):format(team_index, field),
                    left_team[field],
                    right_team[field]
                )
            end
        end
        for player_index = 1, input_frame.FIXTURE_TEAM_SIZE do
            local left_player = left_team.roster[player_index]
            local right_player = right_team.roster[player_index]
            for _, field in ipairs({ "player_id", "position", "loadout_id", "family_id" }) do
                if left_player[field] ~= right_player[field] then
                    return difference(
                        ("manifest.teams.%d.roster.%d.%s"):format(team_index, player_index, field),
                        left_player[field],
                        right_player[field]
                    )
                end
            end
        end
    end
    for index = 1, input_frame.SLOT_COUNT do
        for _, field in ipairs({ "slot", "team", "player_id" }) do
            if expected.slots[index][field] ~= actual.slots[index][field] then
                return difference(
                    ("manifest.slots.%d.%s"):format(index, field),
                    expected.slots[index][field],
                    actual.slots[index][field]
                )
            end
        end
    end
    return nil
end

---@param expected SessionManifest
---@param actual SessionManifest
---@return boolean?, string?, SessionProtocolErrorCode?, string?
function protocol.compare_manifest(expected, actual)
    local diff = protocol.manifest_difference(expected, actual)
    if diff then
        return nil,
            "deterministic manifest mismatch at " .. diff.path,
            "identity_mismatch",
            diff.path
    end
    return true
end

---@param expected SessionRuntimeIdentity
---@param actual SessionRuntimeIdentity
---@return boolean?, string?, SessionProtocolErrorCode?, string?
function protocol.compare_runtime(expected, actual)
    local diff = protocol.runtime_difference(expected, actual)
    if diff then
        return nil, "runtime compatibility mismatch at " .. diff.path, "runtime_mismatch", diff.path
    end
    return true
end

---@param assignments any
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_slot_assignments(assignments)
    if not is_array(assignments, input_frame.SLOT_COUNT) then
        return failure("malformed", "slot assignment must contain exactly eight producers")
    end
    local producer_ids = {}
    for index, assignment in ipairs(assignments) do
        local slot = assert(input_frame.slot(index))
        if
            not has_only_fields(assignment, SLOT_PRODUCER_FIELDS)
            or assignment.slot ~= slot.id
            or assignment.team ~= slot.team
        then
            return failure("malformed", "slot producers violate OMP-1 canonical order")
        end
        local ok, err, code = validate_player_id(assignment.player_id, "slot assignment player id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_id(assignment.producer_id, "slot assignment producer id")
        if not ok then
            return nil, err, code
        end
        if producer_ids[assignment.producer_id] then
            return failure("malformed", "slot producer ids must be unique across peers and bots")
        end
        producer_ids[assignment.producer_id] = true
        if assignment.producer_kind == "peer" then
            if assignment.bot_seed ~= nil then
                return failure("malformed", "peer slot producer cannot carry a bot seed")
            end
        elseif assignment.producer_kind == "bot" then
            ok, err, code = validate_integer(assignment.bot_seed, "bot seed", 0, protocol.MAX_SEED)
            if not ok then
                return nil, err, code
            end
        else
            return failure("malformed", "slot producer kind is invalid")
        end
    end
    return true
end

---@param manifest SessionManifest
---@param assignments SessionSlotProducer[]
---@return boolean?, string?, SessionProtocolErrorCode?
function protocol.validate_assignment_manifest(manifest, assignments)
    local ok, err, code = protocol.validate_manifest(manifest)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_slot_assignments(assignments)
    if not ok then
        return nil, err, code
    end
    for index = 1, input_frame.SLOT_COUNT do
        if manifest.slots[index].player_id ~= assignments[index].player_id then
            return failure(
                "identity_mismatch",
                ("slot assignment player differs at manifest.slots.%d.player_id"):format(index)
            )
        end
    end
    return true
end

---@param message any
---@return boolean?, string?, SessionProtocolErrorCode?
function protocol.validate(message)
    if not has_only_fields(message, MESSAGE_FIELDS) then
        return failure("malformed", "control message must contain only canonical fields")
    end
    if not is_integer(message.version) then
        return failure("malformed", "control message version must be an integer")
    end
    if message.version ~= protocol.VERSION then
        return failure("unsupported_version", "unsupported control protocol version")
    end
    if type(message.kind) ~= "string" or not BODY_FIELDS[message.kind] then
        return failure("unknown_message", "unknown control message kind")
    end
    local ok, err, code = validate_identity_component(
        message.session_id,
        "message.session_id",
        protocol.MAX_SESSION_ID_BYTES
    )
    if not ok then
        return nil, err, code
    end
    ok, err, code =
        validate_identity_component(message.peer_id, "message.peer_id", protocol.MAX_PEER_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_integer(message.sequence, "message.sequence", 0, protocol.MAX_SEQUENCE)
    if not ok then
        return nil, err, code
    end
    if
        type(message.message_id) ~= "string"
        or #message.message_id > protocol.MAX_MESSAGE_ID_BYTES
    then
        return failure("malformed", "message id must be a bounded canonical transcript id")
    end
    if
        message.message_id
        ~= protocol.message_id(message.session_id, message.peer_id, message.sequence)
    then
        return failure("malformed", "message id does not match its transcript identity")
    end
    local allowed_fields = BODY_FIELDS[message.kind]
    if not has_only_fields(message.body, allowed_fields) then
        return failure("malformed", message.kind .. " body contains unknown fields")
    end
    local body = message.body
    if message.kind == "handshake" then
        if not ROLES[body.role] then
            return failure("malformed", "handshake role is invalid")
        end
        return protocol.validate_runtime(body.runtime)
    elseif message.kind == "manifest_proposal" then
        ok, err, code = protocol.validate_manifest(body.manifest)
        if not ok then
            return nil, err, code
        end
        if body.manifest.session_id ~= message.session_id then
            return failure("malformed", "manifest session id differs from message session")
        end
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        if body.manifest_id ~= protocol.manifest_id(body.manifest) then
            return failure("malformed", "manifest id does not match canonical manifest")
        end
        return true
    elseif message.kind == "manifest_accept" then
        return validate_hash(body.manifest_id, "manifest id")
    elseif message.kind == "peer_assignment" then
        if not ROLES[body.role] then
            return failure("malformed", "peer assignment role is invalid")
        end
        return validate_id(body.assigned_peer_id, "assigned peer id")
    elseif message.kind == "slot_assignment" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        return validate_slot_assignments(body.assignments)
    elseif message.kind == "ready" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        if type(body.ready) ~= "boolean" then
            return failure("malformed", "ready state must be boolean")
        end
        return true
    elseif message.kind == "countdown" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_id(body.countdown_id, "countdown id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_integer(
            body.remaining_ticks,
            "remaining countdown ticks",
            0,
            protocol.MAX_COUNTDOWN_TICKS
        )
        if not ok then
            return nil, err, code
        end
        return validate_integer(
            body.first_input_tick,
            "countdown first input tick",
            0,
            input_frame.MAX_TICK
        )
    elseif message.kind == "start" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_id(body.countdown_id, "countdown id")
        if not ok then
            return nil, err, code
        end
        return validate_integer(
            body.first_input_tick,
            "start first input tick",
            0,
            input_frame.MAX_TICK
        )
    elseif message.kind == "match_phase" then
        if not MATCH_PHASES[body.phase] then
            return failure("malformed", "match phase is invalid")
        end
        for _, definition in ipairs({
            { "tick", 0, input_frame.MAX_TICK },
            { "home_score", 0, protocol.MAX_GOALS },
            { "away_score", 0, protocol.MAX_GOALS },
        }) do
            ok, err, code = validate_integer(
                body[definition[1]],
                "match phase " .. definition[1],
                definition[2],
                definition[3]
            )
            if not ok then
                return nil, err, code
            end
        end
        return true
    elseif message.kind == "hash_report" then
        ok, err, code = validate_integer(body.tick, "hash report tick", 0, input_frame.MAX_TICK)
        if not ok then
            return nil, err, code
        end
        return validate_hash(body.boundary_hash, "boundary hash")
    elseif message.kind == "result_ack" then
        for _, definition in ipairs({
            { "final_tick", 0, input_frame.MAX_TICK },
            { "home_score", 0, protocol.MAX_GOALS },
            { "away_score", 0, protocol.MAX_GOALS },
        }) do
            ok, err, code = validate_integer(
                body[definition[1]],
                "result " .. definition[1],
                definition[2],
                definition[3]
            )
            if not ok then
                return nil, err, code
            end
        end
        return validate_hash(body.final_hash, "final hash")
    elseif message.kind == "abort" then
        if not REJECT_CODES[body.code] then
            return failure("malformed", "abort code is invalid")
        end
        return true
    end
    if not DISCONNECT_CODES[body.code] then
        return failure("malformed", "disconnect code is invalid")
    end
    ok, err, code = validate_id(body.target_peer_id, "disconnect target peer id")
    if not ok then
        return nil, err, code
    end
    return true
end

---@param value any
---@param depth integer
---@return string
local function encode_value(value, depth)
    assert(depth <= 12, "protocol value nesting is too deep")
    local kind = type(value)
    if value == nil then
        return "z"
    elseif kind == "boolean" then
        return value and "b1" or "b0"
    elseif kind == "number" then
        assert(is_integer(value), "protocol numbers must be finite integers")
        local text = tostring(value)
        return "i" .. tostring(#text) .. ":" .. text
    elseif kind == "string" then
        return "s" .. tostring(#value) .. ":" .. value
    end
    assert(kind == "table", "protocol values must be canonical scalars or tables")
    local keys = {}
    for key in pairs(value) do
        assert(
            type(key) == "string" or (is_integer(key) and key >= 1),
            "protocol table key is invalid"
        )
        keys[#keys + 1] = key
    end
    table.sort(keys, function(left, right)
        if type(left) ~= type(right) then
            return type(left) == "number"
        end
        return left < right
    end)
    local parts = { "t", tostring(#keys), ":" }
    for _, key in ipairs(keys) do
        parts[#parts + 1] = encode_value(key, depth + 1)
        parts[#parts + 1] = encode_value(value[key], depth + 1)
    end
    return table.concat(parts)
end

---@class ProtocolDecoder
---@field wire string
---@field index integer

---@param decoder ProtocolDecoder
---@return string?, string?
local function decode_length(decoder)
    local colon = decoder.wire:find(":", decoder.index, true)
    if not colon then
        return nil, "protocol value length is missing"
    end
    local raw = decoder.wire:sub(decoder.index, colon - 1)
    if not is_canonical_unsigned(raw) then
        return nil, "protocol value length is noncanonical"
    end
    decoder.index = colon + 1
    return raw
end

---@param decoder ProtocolDecoder
---@param depth integer
---@return any, string?
local function decode_value(decoder, depth)
    if depth > 12 or decoder.index > #decoder.wire then
        return nil, "protocol value is truncated or too deeply nested"
    end
    local tag = decoder.wire:sub(decoder.index, decoder.index)
    decoder.index = decoder.index + 1
    if tag == "z" then
        return nil
    elseif tag == "b" then
        local value = decoder.wire:sub(decoder.index, decoder.index)
        decoder.index = decoder.index + 1
        if value == "0" then
            return false
        elseif value == "1" then
            return true
        end
        return nil, "protocol boolean is invalid"
    elseif tag == "i" or tag == "s" then
        local raw_length, length_err = decode_length(decoder)
        if not raw_length then
            return nil, length_err
        end
        local length = tonumber(raw_length)
        if not length or length > protocol.MAX_WIRE_BYTES then
            return nil, "protocol value length exceeds the wire bound"
        end
        ---@cast length integer
        local finish = decoder.index + length - 1
        if finish > #decoder.wire then
            return nil, "protocol value is truncated"
        end
        local raw = decoder.wire:sub(decoder.index, finish)
        decoder.index = finish + 1
        if tag == "s" then
            return raw
        end
        if not is_canonical_integer(raw) then
            return nil, "protocol integer is noncanonical"
        end
        local value = tonumber(raw)
        if not is_integer(value) then
            return nil, "protocol integer is outside the supported range"
        end
        return value
    elseif tag ~= "t" then
        return nil, "protocol value tag is unknown"
    end
    local raw_count, count_err = decode_length(decoder)
    if not raw_count then
        return nil, count_err
    end
    local count = tonumber(raw_count)
    if not count or count > 256 then
        return nil, "protocol table exceeds the item bound"
    end
    local result = {}
    for _ = 1, count do
        local key, key_err = decode_value(decoder, depth + 1)
        if key_err then
            return nil, key_err
        end
        if (type(key) ~= "string" and not (is_integer(key) and key >= 1)) or result[key] ~= nil then
            return nil, "protocol table key is invalid or duplicated"
        end
        local item, item_err = decode_value(decoder, depth + 1)
        if item_err then
            return nil, item_err
        end
        if item == nil then
            return nil, "protocol tables cannot encode present nil fields"
        end
        result[key] = item
    end
    return result
end

---@param message SessionControlMessage
---@return string?, string?, SessionProtocolErrorCode?
function protocol.encode(message)
    local ok, err, code = protocol.validate(message)
    if not ok then
        return nil, err, code
    end
    local wire = "GCOP;" .. tostring(protocol.VERSION) .. ";" .. encode_value(message, 0)
    if #wire > protocol.MAX_WIRE_BYTES then
        return failure("wire_too_large", "control message exceeds the canonical wire bound")
    end
    return wire
end

---@param wire any
---@return SessionControlMessage?, string?, SessionProtocolErrorCode?
function protocol.decode(wire)
    if type(wire) ~= "string" then
        return failure("malformed", "control wire must be a string")
    end
    if #wire > protocol.MAX_WIRE_BYTES then
        return failure("wire_too_large", "control wire exceeds the canonical wire bound")
    end
    local raw_version, body = wire:match("^GCOP;([^;]+);(.*)$")
    if not raw_version or not is_canonical_unsigned(raw_version) then
        return failure("malformed", "control wire header is malformed")
    end
    local version = tonumber(raw_version)
    if version ~= protocol.VERSION then
        return failure("unsupported_version", "unsupported control wire version")
    end
    local decoder = { wire = body, index = 1 }
    local decoded, decode_err = decode_value(decoder, 0)
    if decode_err or type(decoded) ~= "table" or decoder.index ~= #body + 1 then
        return failure("malformed", decode_err or "control wire has trailing bytes")
    end
    local ok, err, code = protocol.validate(decoded)
    if not ok then
        return nil, err, code
    end
    ---@cast decoded SessionControlMessage
    local canonical = protocol.encode(decoded)
    if canonical ~= wire then
        return failure("malformed", "control wire is not canonical")
    end
    return decoded
end

---@param kind SessionMessageKind
---@param session_id string
---@param peer_id string
---@param sequence integer
---@param body SessionMessageBody
---@return SessionControlMessage?, string?, SessionProtocolErrorCode?
function protocol.new(kind, session_id, peer_id, sequence, body)
    local message_id, id_err, id_code = protocol.message_id(session_id, peer_id, sequence)
    if not message_id then
        return nil, id_err, id_code
    end
    local message = {
        version = protocol.VERSION,
        kind = kind,
        session_id = session_id,
        peer_id = peer_id,
        sequence = sequence,
        message_id = message_id,
        body = body,
    }
    local wire, err, code = protocol.encode(message)
    if not wire then
        return nil, err, code
    end
    return protocol.decode(wire)
end

---@param manifest SessionManifest
---@return string
function protocol.manifest_id(manifest)
    local ok, err = protocol.validate_manifest(manifest)
    assert(ok, err)
    return fnv1a64.hash("GCOM;" .. encode_value(manifest, 0))
end

---@param message SessionControlMessage
---@param phase SessionLifecyclePhase
---@return boolean?, string?, SessionProtocolErrorCode?
function protocol.validate_phase(message, phase)
    local ok, err, code = protocol.validate(message)
    if not ok then
        return nil, err, code
    end
    local allowed = ALLOWED_PHASES[message.kind]
    if not allowed or not allowed[phase] then
        return failure(
            "invalid_phase",
            ("%s is invalid during session phase %s"):format(message.kind, tostring(phase))
        )
    end
    if
        message.kind == "match_phase"
        and not MATCH_PHASES_BY_LIFECYCLE[phase][message.body.phase]
    then
        return failure(
            "invalid_phase",
            ("match phase %s is invalid during session phase %s"):format(
                tostring(message.body.phase),
                tostring(phase)
            )
        )
    end
    return true
end

---@param previous SessionControlMessage
---@param incoming SessionControlMessage
---@return SessionDuplicateDisposition?, string?, SessionProtocolErrorCode?
function protocol.classify_duplicate(previous, incoming)
    local previous_wire, previous_err, previous_code = protocol.encode(previous)
    if not previous_wire then
        return nil, previous_err, previous_code
    end
    local incoming_wire, incoming_err, incoming_code = protocol.encode(incoming)
    if not incoming_wire then
        return nil, incoming_err, incoming_code
    end
    if previous.message_id ~= incoming.message_id then
        return failure("duplicate", "messages do not share a transcript identity")
    end
    if previous_wire == incoming_wire then
        return "idempotent"
    end
    return failure("transcript_conflict", "message id was reused with different canonical bytes")
end

---@param messages SessionControlMessage[]
---@return string
function protocol.transcript_id(messages)
    assert(is_array(messages), "transcript must be a canonical message array")
    local state = fnv1a64.new()
    fnv1a64.update(state, "GCOT;1;")
    local last_by_peer = {}
    for _, message in ipairs(messages) do
        local wire = assert(protocol.encode(message))
        local previous = last_by_peer[message.peer_id]
        assert(
            previous == nil or message.sequence > previous,
            "transcript sequence is not monotonic"
        )
        last_by_peer[message.peer_id] = message.sequence
        fnv1a64.update(state, tostring(#wire) .. ":" .. wire)
    end
    return fnv1a64.hex(state)
end

return protocol
