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
---| "unsupported_match_mode"
---| "invalid_ownership"
---@alias SessionMessageKind
---| "handshake"
---| "manifest_proposal"
---| "manifest_accept"
---| "peer_assignment"
---| "slot_assignment"
---| "ready"
---| "pair_preference"
---| "pair_preference_result"
---| "countdown"
---| "start"
---| "match_phase"
---| "hash_report"
---| "result_ack"
---| "abort"
---| "disconnect"
---@alias SessionRole "host"|"guest"
---@alias SessionTeam "home"|"away"
-- The pitch is always 4v4. A match mode says how many humans share those eight
-- canonical outfield slots, never how many players are on it.
---@alias SessionMatchMode "1v1"|"2v2"|"4v4"
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
-- The host's verdict on one pair preference. `unchanged` is a grant that moved
-- nothing: the peer already owned exactly the set it asked for, so no ownership
-- generation is minted and no readiness is cleared.
---@alias SessionPreferenceStatus "granted"|"unchanged"|"rejected"
-- Why a preference could not be granted. Every reason leaves the requesting
-- peer's ownership exactly as it was.
---@alias SessionPreferenceRejection
---| "already_taken" -- A requested slot belongs to another peer's chosen pair.
---| "wrong_team" -- A requested slot is not on the team this peer is seated on.
---| "invalid_slot" -- The set is not a legal owned set for the frozen match mode.
---| "detached" -- The set keeps none of the slots this peer already owns.
---| "not_seated" -- The peer owns nothing in the generation it answered.
---| "superseded" -- The preference answers an ownership generation no longer in force.
---| "after_freeze" -- Ownership is frozen and cannot move again this session.
---| "no_response" -- The request expired unanswered. Minted locally, never on the wire.
---| "reseated" -- A roster change needed the pair's slots. Minted locally, never on the wire.

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

---@class SessionMatchModeShape
---@field mode SessionMatchMode
---@field humans integer -- Total humans the mode seats across both teams.
---@field slots_per_human integer -- Canonical outfield slots one human owns.
---@field team_humans integer -- Humans the mode seats on one team.

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
---@field match_mode SessionMatchMode -- How the eight canonical slots are shared.
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
---@field assignment_id string -- Host-minted identity of this ownership generation.
---@field assignments SessionSlotProducer[]

---@class SessionReadyBody
---@field manifest_id string
---@field assignment_id string -- The ownership generation this readiness answers.
---@field ready boolean

-- A guest asking the host for the outfield slots it wants to control. It is a
-- request and never an assignment: the host answers with
-- `pair_preference_result` and, when it grants one, republishes ownership
-- through the ordinary `slot_assignment` path.
---@class SessionPairPreferenceBody
---@field manifest_id string
---@field assignment_id string -- The ownership generation this preference answers.
---@field slots InputSlotId[] -- The requested owned set, ascending canonical order.

---@class SessionPairPreferenceResultBody
---@field manifest_id string
---@field assignment_id string -- Echo of the generation the request named.
---@field slots InputSlotId[] -- Echo of the requested set, so a result is self-describing.
---@field status SessionPreferenceStatus
---@field reason SessionPreferenceRejection? -- Present exactly when `status` is `rejected`.

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
---| SessionPairPreferenceBody
---| SessionPairPreferenceResultBody
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

-- Every mode plays on the same eight canonical outfield slots and the same two
-- protected keepers; only the number of humans and the size of one human's
-- owned set changes. `humans * slots_per_human` is therefore always
-- `input_frame.SLOT_COUNT`, and 3v3 has no entry because four outfield slots per
-- team do not divide into three humans.
---@type table<string, SessionMatchModeShape>
protocol.MATCH_MODES = {
    ["1v1"] = { mode = "1v1", humans = 2, slots_per_human = 4, team_humans = 1 },
    ["2v2"] = { mode = "2v2", humans = 4, slots_per_human = 2, team_humans = 2 },
    ["4v4"] = { mode = "4v4", humans = 8, slots_per_human = 1, team_humans = 4 },
}

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
    match_mode = true,
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
    slot_assignment = { manifest_id = true, assignment_id = true, assignments = true },
    ready = { manifest_id = true, assignment_id = true, ready = true },
    pair_preference = { manifest_id = true, assignment_id = true, slots = true },
    pair_preference_result = {
        manifest_id = true,
        assignment_id = true,
        slots = true,
        status = true,
        reason = true,
    },
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
---@type table<SessionPreferenceStatus, boolean>
protocol.PREFERENCE_STATUSES = { granted = true, unchanged = true, rejected = true }

-- Every typed refusal a pair preference can end on, wire-borne or local. A
-- reader that shows preferences has to cover all of them, so the set is public.
---@type table<SessionPreferenceRejection, boolean>
protocol.PREFERENCE_REJECTIONS = {
    already_taken = true,
    wrong_team = true,
    invalid_slot = true,
    detached = true,
    not_seated = true,
    superseded = true,
    after_freeze = true,
    no_response = true,
    reseated = true,
}

-- The refusals no host ever sends, because neither is a verdict on a request.
-- Each is minted by the peer that observes it: silence is only observable at the
-- end that waited, and a pair the roster reseated away is only observable
-- against the ownership that took it, which every peer holds for itself. A host
-- claiming either is malformed.
---@type table<SessionPreferenceRejection, boolean>
protocol.LOCAL_PREFERENCE_REJECTIONS = {
    no_response = true,
    reseated = true,
}

-- The refusals a host may put on the wire: every typed reason except the ones a
-- peer mints for itself. The accepted message vocabulary is therefore exactly
-- what it was before either local reason existed.
---@type table<SessionPreferenceRejection, boolean>
local WIRE_PREFERENCE_REJECTIONS = {}
for reason in pairs(protocol.PREFERENCE_REJECTIONS) do
    if not protocol.LOCAL_PREFERENCE_REJECTIONS[reason] then
        WIRE_PREFERENCE_REJECTIONS[reason] = true
    end
end
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
    -- Configuration can still change in `assigned` and `ready`, so a preference
    -- is legal there. `countdown` is included on purpose: the freeze lands at
    -- the countdown, and a preference already in flight when it does is an
    -- ordinary race that deserves the typed `after_freeze` refusal the host
    -- gives it, not a terminated session. Past the countdown every peer has
    -- seen `start`, and a preference is as much a violation as a late `ready`.
    pair_preference = { assigned = true, ready = true, countdown = true },
    pair_preference_result = { assigned = true, ready = true, countdown = true },
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
    "match_mode",
}
local RUNTIME_COMPARE_FIELDS = {
    "version",
    "runtime_id",
    "runtime_revision",
    "presentation_id",
}
local MESSAGE_ID_PREFIX = "GCMI;1;"

-- Canonical slot id -> OMP-1 slot index, built once with a numeric loop so no
-- canonical-order query ever walks a hash table.
---@type table<string, integer>
local SLOT_INDEXES = {}
for index = 1, input_frame.SLOT_COUNT do
    SLOT_INDEXES[assert(input_frame.slot(index)).id] = index
end

---@param set table<string, any>
---@return string[]
local function sorted_keys(set)
    local keys = {}
    for key in pairs(set) do
        keys[#keys + 1] = key
    end
    table.sort(keys)
    return keys
end

-- Digest everything a peer has to agree with before a control message is
-- acceptable: which kinds exist, which fields each body carries, and which
-- lifecycle phases each kind is legal in. Disagreeing about any of the three is
-- unrecoverable and always was — an unknown kind is `unknown_message`, an
-- unfamiliar body field is `malformed`, and a kind sent where this build does
-- not allow it is `invalid_phase` — and each fires only when the offending
-- message is finally sent, which for a lobby kind means partway through the
-- lobby.
--
-- Pure and parameterised so the sensitivity is provable rather than asserted:
-- a test can put a vocabulary that differs by one kind, one field, or one phase
-- against this build's and require a different digest. Sorted throughout, so it
-- never depends on `pairs` order and two peers running the same code always
-- compute the same string.
---@param kinds table<string, table<string, any>> -- Message kind -> its body field set.
---@param phases table<string, table<string, any>> -- Message kind -> its legal phase set.
---@return string
function protocol.vocabulary_digest(kinds, phases)
    local ordered = sorted_keys(kinds)
    local ordered_phases = sorted_keys(phases)
    assert(#ordered == #ordered_phases, "every message kind needs exactly one phase rule")
    local parts = { "GCPV;1;", tostring(protocol.VERSION), ";" }
    for index, kind in ipairs(ordered) do
        assert(kind == ordered_phases[index], "message kind and phase tables disagree: " .. kind)
        parts[#parts + 1] = ("%s:%s@%s;"):format(
            kind,
            table.concat(sorted_keys(kinds[kind]), ","),
            table.concat(sorted_keys(phases[kind]), ",")
        )
    end
    return fnv1a64.hash(table.concat(parts))
end

-- This build's own vocabulary, digested once at load from the same two tables
-- `validate` and `validate_phase` read, so it cannot drift from the vocabulary
-- it claims to describe. `game.online.match_manifest` folds it into `build_id`,
-- which is what moves the disagreement above from mid-lobby to the manifest
-- check.
local VOCABULARY_ID = protocol.vocabulary_digest(BODY_FIELDS, ALLOWED_PHASES)

-- Opaque, stable identity of the control vocabulary this build speaks.
---@return string
function protocol.vocabulary_id()
    return VOCABULARY_ID
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

-- The one place an unsupported size is refused. `3v3` fails here, exactly like
-- `5v5` or a misspelling, because the mode table -- not a special case -- is the
-- closed set of supported shapes.
---@param mode any
---@return SessionMatchModeShape?, string?, SessionProtocolErrorCode?
function protocol.match_mode(mode)
    local shape = type(mode) == "string" and protocol.MATCH_MODES[mode] or nil
    if not shape then
        return failure(
            "unsupported_match_mode",
            ("match mode %s is not a supported size"):format(
                type(mode) == "string" and mode or type(mode)
            )
        )
    end
    return shape
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
    local mode_shape, mode_err, mode_code = protocol.match_mode(manifest.match_mode)
    if not mode_shape then
        return nil, mode_err, mode_code
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

---@class SessionProducerOwnership
---@field producer_kind SessionProducerKind
---@field team SessionTeam
---@field slots InputSlotId[] -- Canonical order, one or more entries.

-- Slot ownership is a *function* from the eight canonical slots onto producers:
-- every slot still names exactly one declared source, but one human source may
-- now cover several slots, which is what 1v1 and 2v2 are. The rules that keep
-- the source unambiguous survive that widening:
--
--   * a bot producer drives exactly one slot, because its `bot_seed` is per-slot
--     and a shared bot id would make that seed ambiguous;
--   * a producer id never crosses the peer and bot namespaces; and
--   * one human's owned slots all sit on one team, so an owned set is always a
--     subset of a single outfield line.
--
-- How *many* slots a human may own is a manifest question, not a wire-shape
-- one: it depends on the frozen match mode and is enforced by
-- `validate_assignment_manifest`.
---@param assignments any
---@return table<string, SessionProducerOwnership>?, string?, SessionProtocolErrorCode?
local function collect_slot_assignments(assignments)
    if not is_array(assignments, input_frame.SLOT_COUNT) then
        return failure("malformed", "slot assignment must contain exactly eight producers")
    end
    ---@type table<string, SessionProducerOwnership>
    local producers = {}
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
        local owned = producers[assignment.producer_id]
        if not owned then
            producers[assignment.producer_id] = {
                producer_kind = assignment.producer_kind,
                team = slot.team,
                slots = { slot.id },
            }
        elseif owned.producer_kind ~= assignment.producer_kind then
            return failure(
                "malformed",
                "slot producer ids must not cross the peer and bot namespaces"
            )
        elseif assignment.producer_kind ~= "peer" then
            return failure("malformed", "a bot producer drives exactly one canonical slot")
        elseif owned.team ~= slot.team then
            return failure("malformed", "a human producer owns slots on exactly one team")
        else
            owned.slots[#owned.slots + 1] = slot.id
        end
    end
    return producers
end

---@param assignments any
---@return boolean?, string?, SessionProtocolErrorCode?
local function validate_slot_assignments(assignments)
    local producers, err, code = collect_slot_assignments(assignments)
    if not producers then
        return nil, err, code
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
    local producers
    producers, err, code = collect_slot_assignments(assignments)
    if not producers then
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
    -- The frozen mode fixes the size of every human's owned set exactly. A
    -- 4v4 owned set has one member, so nothing here special-cases it. Slots are
    -- walked in canonical order rather than by hash order, so the first
    -- offending owned set is reported identically on every peer.
    local shape = assert(protocol.MATCH_MODES[manifest.match_mode])
    for index = 1, input_frame.SLOT_COUNT do
        local producer_id = assignments[index].producer_id
        local owned = producers[producer_id]
        if owned.producer_kind == "peer" and #owned.slots ~= shape.slots_per_human then
            return failure(
                "invalid_ownership",
                ("%s owns %d canonical slots but %s seats %d per human"):format(
                    producer_id,
                    #owned.slots,
                    shape.mode,
                    shape.slots_per_human
                )
            )
        end
    end
    return true
end

-- The owned set of one producer: every canonical slot that names it, in OMP-1
-- order. A human controls exactly one of these at a time; the rest run the
-- deterministic gameplay AI and are indistinguishable from declared bot fills.
-- The OMP-1 index of a canonical outfield slot id, or nil when the id names no
-- canonical slot. Keepers are slotless in every mode, so they answer nil here.
---@param slot any
---@return integer?
function protocol.slot_index(slot)
    if type(slot) ~= "string" then
        return nil
    end
    return SLOT_INDEXES[slot]
end

-- The wire shape of a requested owned set: canonical outfield slot ids in
-- strictly ascending canonical order, so one set has exactly one encoding and a
-- duplicate slot cannot hide inside it.
--
-- Keeper protection needs no check here. A keeper has no canonical slot id, so
-- the vocabulary this validates cannot express one.
--
-- *How many* slots a human may own is a manifest question, not a wire-shape
-- one: it depends on the frozen match mode and is enforced where the mode is
-- known, exactly as `validate_assignment_manifest` enforces published owned
-- sets.
---@param slots any
---@return boolean?, string?, SessionProtocolErrorCode?
function protocol.validate_slot_set(slots)
    if not is_array(slots, nil, input_frame.SLOT_COUNT) or #slots == 0 then
        return failure("malformed", "a slot set must hold one to eight canonical slots")
    end
    local previous = 0
    for _, slot in ipairs(slots) do
        local index = protocol.slot_index(slot)
        if not index then
            return failure("malformed", "slot set names an unknown canonical slot")
        end
        if index <= previous then
            return failure("malformed", "slot set is not in ascending canonical order")
        end
        previous = index
    end
    return true
end

---@param assignments SessionSlotProducer[]
---@param producer_id string
---@return InputSlotId[]
function protocol.owned_slots(assignments, producer_id)
    local ok, err = validate_slot_assignments(assignments)
    assert(ok, err)
    ---@type InputSlotId[]
    local owned = {}
    for index = 1, input_frame.SLOT_COUNT do
        local producer = assignments[index]
        if producer.producer_id == producer_id then
            owned[#owned + 1] = producer.slot
        end
    end
    return owned
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
        ok, err, code = validate_hash(body.assignment_id, "assignment id")
        if not ok then
            return nil, err, code
        end
        return validate_slot_assignments(body.assignments)
    elseif message.kind == "ready" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_hash(body.assignment_id, "assignment id")
        if not ok then
            return nil, err, code
        end
        if type(body.ready) ~= "boolean" then
            return failure("malformed", "ready state must be boolean")
        end
        return true
    elseif message.kind == "pair_preference" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_hash(body.assignment_id, "assignment id")
        if not ok then
            return nil, err, code
        end
        return protocol.validate_slot_set(body.slots)
    elseif message.kind == "pair_preference_result" then
        ok, err, code = validate_hash(body.manifest_id, "manifest id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = validate_hash(body.assignment_id, "assignment id")
        if not ok then
            return nil, err, code
        end
        ok, err, code = protocol.validate_slot_set(body.slots)
        if not ok then
            return nil, err, code
        end
        if not protocol.PREFERENCE_STATUSES[body.status] then
            return failure("malformed", "pair preference status is invalid")
        end
        if body.status == "rejected" then
            if not WIRE_PREFERENCE_REJECTIONS[body.reason] then
                return failure("malformed", "a refused pair preference needs a typed reason")
            end
        elseif body.reason ~= nil then
            return failure("malformed", "only a refused pair preference carries a reason")
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

-- Identity of one ownership generation. The epoch is part of the digest, so
-- republishing byte-identical ownership still mints a distinct generation and a
-- readiness answer can never be attributed to the wrong one. Peers treat the
-- result as an opaque token minted by the host and echo it back; only the host
-- can derive it, because only the host holds the epoch.
---@param assignments SessionSlotProducer[]
---@param epoch integer
---@return string
function protocol.assignment_id(assignments, epoch)
    local ok, err = validate_slot_assignments(assignments)
    assert(ok, err)
    assert(
        is_integer(epoch) and epoch >= 0 and epoch <= protocol.MAX_SEQUENCE,
        "assignment epoch must be a bounded non-negative integer"
    )
    return fnv1a64.hash("GCOA;" .. length_prefix(tostring(epoch)) .. encode_value(assignments, 0))
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
