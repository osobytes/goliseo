-- Canonical start-of-tick snapshots for sim.match.
--
-- Capture is valid only between match.step calls. In slot mode `input_tick`
-- names the next InputFrame to consume. The explicit allowlists below are both
-- the copy contract and canonical serialization order: adding simulation state
-- must fail capture/specs until this module is consciously versioned.

local Vec2 = require("core.vec2")
local fnv1a64 = require("core.fnv1a64")
local input_frame = require("sim.input_frame")
local combat_snapshot = require("sim.combat_snapshot")

---@class MatchSnapshot
---@field version integer
---@field state MatchState
---@field combat CombatMatchState?

---@class MatchSnapshotDifference
---@field path string
---@field expected any
---@field actual any

---@class MatchSnapshotModule
local match_snapshot = {}

match_snapshot.VERSION = 5
match_snapshot.COMBAT_VERSION = 6

---@type table<MatchState, string>
local unsupported_states = setmetatable({}, { __mode = "k" })

---@param state MatchState
---@param reason string
function match_snapshot.mark_unsupported(state, reason)
    assert(type(state) == "table", "match state is required")
    assert(type(reason) == "string" and reason ~= "", "snapshot rejection reason is required")
    unsupported_states[state] = reason
end

---@param state MatchState
---@param combat_state CombatMatchState?
local function assert_snapshot_supported(state, combat_state)
    local reason = unsupported_states[state]
    assert(reason == nil or combat_state ~= nil, reason)
end

match_snapshot.MATCH_FIELDS = {
    "field",
    "goal_home",
    "goal_away",
    "players",
    "ball",
    "ball_vel",
    "ball_z",
    "ball_vz",
    "owner",
    "controlled",
    "human_controlled",
    "score",
    "time_left",
    "max_goals",
    "finished",
    "pickup_cd",
    "press",
    "marking",
    "marks",
    "ball_spin",
    "rng",
    "block_grace",
    "aerial_lock",
    "kickoff_hold",
    "events",
    "slot_mode",
    "input_ownership",
    "slot_players",
    "slot_for_player",
    "input_tick",
}

match_snapshot.PLAYER_FIELDS = {
    "id",
    "name",
    "team",
    "pos",
    "vel",
    "run_vel",
    "facing",
    "anchor",
    "species_id",
    "owned_verb",
    "move_speed",
    "shot_speed",
    "dribble",
    "strength",
    "first_touch",
    "header_skill",
    "volley_skill",
    "bicycle_skill",
    "is_keeper",
    "radius",
    "dash_cd",
    "dodge_cd",
    "dodge_timer",
    "dodge_dir",
    "reach",
    "handling",
    "keeper_aggression",
    "keeper_anticipation",
    "keeper_state",
    "keeper_state_timer",
    "keeper_release_state",
    "keeper_release_motion",
    "keeper_release_kind",
    "keeper_release_depth",
    "keeper_set",
    "dive_timer",
    "dive_dir",
    "dive_delay",
    "dive_target",
    "hold_timer",
    "feet_ball",
    "slide_timer",
    "slide_dir",
    "slide_vel",
    "tackle_timer",
    "tackle_cd",
    "stun_timer",
    "grab_timer",
    "throw_timer",
    "receive_timer",
    "sprint_meter",
    "sprint_dur",
    "sprinting",
    "save_pending",
    "save_timer",
    "save_vx",
    "save_style",
    "save_tip_emitted",
    "settle_timer",
    "header_cd",
    "aerial_timer",
    "aerial_style",
    "aerial_outcome",
    "aerial_jump",
    "aerial_recovery",
    "charge",
    "pass_charge",
    "pass_target",
    "windup_timer",
    "windup_shot",
    "jockey_timer",
}

local VECTOR_FIELDS = {
    pos = true,
    vel = true,
    run_vel = true,
    facing = true,
    anchor = true,
    dodge_dir = true,
    dive_dir = true,
    slide_dir = true,
}

local OPTIONAL_VECTOR_FIELDS = {
    dive_target = true,
}

local EVENT_FIELDS = {
    "kind",
    "x",
    "y",
    "player",
    "save_style",
    "style",
    "outcome",
    "jumping",
    "difficulty",
    "shot_type",
    "keeper_state",
    "keeper_depth",
    "on_target",
}

local MARKING_FIELDS = {
    "scheme",
    "man_marks",
    "standoff",
    "compactness",
    "support",
}

local WINDUP_FIELDS = {
    "dir",
    "speed",
    "vz",
    "spin",
    "shot_type",
}

local ASSIGNMENT_FIELDS = {
    "slot",
    "team",
    "player_id",
}

local MATCH_FIELD_SET = {}
for _, field in ipairs(match_snapshot.MATCH_FIELDS) do
    MATCH_FIELD_SET[field] = true
end

local PLAYER_FIELD_SET = {}
for _, field in ipairs(match_snapshot.PLAYER_FIELDS) do
    PLAYER_FIELD_SET[field] = true
end

---@param fields string[]
---@return table<string, boolean>
local function field_set(fields)
    local result = {}
    for _, field in ipairs(fields) do
        result[field] = true
    end
    return result
end

local VECTOR_FIELD_SET = field_set({ "x", "y" })
local FIELD_FIELD_SET = field_set({ "w", "h" })
local RECT_FIELD_SET = field_set({ "x", "y", "w", "h" })
local TEAM_FIELD_SET = field_set({ "home", "away" })
local MARKING_FIELD_SET = field_set(MARKING_FIELDS)
local EVENT_FIELD_SET = field_set(EVENT_FIELDS)
local WINDUP_FIELD_SET = field_set(WINDUP_FIELDS)
local OWNERSHIP_FIELD_SET = field_set({ "version", "rosters", "slots" })
local ASSIGNMENT_FIELD_SET = field_set(ASSIGNMENT_FIELDS)
local SNAPSHOT_FIELD_SET = field_set({ "version", "state", "combat" })

---@param value any
---@return boolean
local function is_finite_number(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

---@param value any
---@param allowed table<string, boolean>
---@param path string
local function assert_fields(value, allowed, path)
    assert(type(value) == "table", path .. " must be a table")
    for key in pairs(value) do
        assert(
            type(key) == "string" and allowed[key],
            path .. " has unknown field " .. tostring(key)
        )
    end
end

---@param value any
---@param path string
---@param expected integer?
local function assert_array(value, path, expected)
    assert(type(value) == "table", path .. " must be an array")
    local count = #value
    if expected then
        assert(count == expected, path .. " has the wrong length")
    end
    for key in pairs(value) do
        assert(
            type(key) == "number" and key == math.floor(key) and key >= 1 and key <= count,
            path .. " is not a canonical array"
        )
    end
    for index = 1, count do
        assert(value[index] ~= nil, path .. " has a hole")
    end
end

---@param value any
---@param path string
---@param make_vec boolean
---@return Vec2
local function copy_vec(value, path, make_vec)
    assert_fields(value, VECTOR_FIELD_SET, path)
    assert(is_finite_number(value.x), path .. ".x must be finite")
    assert(is_finite_number(value.y), path .. ".y must be finite")
    if make_vec then
        return Vec2.new(value.x, value.y)
    end
    return { x = value.x, y = value.y }
end

---@param value any
---@param path string
---@return any
local function copy_scalar(value, path)
    local kind = type(value)
    if kind == "number" then
        assert(is_finite_number(value), path .. " must be finite")
    else
        assert(
            value == nil or kind == "string" or kind == "boolean",
            path .. " has unsupported scalar type"
        )
    end
    return value
end

---@param source any
---@param path string
---@param make_vec boolean
---@return MatchPlayer
local function copy_player(source, path, make_vec)
    assert_fields(source, PLAYER_FIELD_SET, path)
    local result = {}
    for _, field in ipairs(match_snapshot.PLAYER_FIELDS) do
        local field_path = path .. "." .. field
        if VECTOR_FIELDS[field] then
            result[field] = copy_vec(source[field], field_path, make_vec)
        elseif OPTIONAL_VECTOR_FIELDS[field] then
            result[field] = source[field] and copy_vec(source[field], field_path, make_vec) or nil
        elseif field == "windup_shot" then
            local shot = source.windup_shot
            if shot then
                assert_fields(shot, WINDUP_FIELD_SET, field_path)
                result.windup_shot = {
                    dir = copy_vec(shot.dir, field_path .. ".dir", make_vec),
                    speed = copy_scalar(shot.speed, field_path .. ".speed"),
                    vz = copy_scalar(shot.vz, field_path .. ".vz"),
                    spin = copy_scalar(shot.spin, field_path .. ".spin"),
                    shot_type = copy_scalar(shot.shot_type, field_path .. ".shot_type"),
                }
            end
        else
            result[field] = copy_scalar(source[field], field_path)
        end
    end
    ---@cast result MatchPlayer
    return result
end

---@param source any
---@param path string
---@return MatchEvent
local function copy_event(source, path)
    assert_fields(source, EVENT_FIELD_SET, path)
    local result = {}
    for _, field in ipairs(EVENT_FIELDS) do
        result[field] = copy_scalar(source[field], path .. "." .. field)
    end
    ---@cast result MatchEvent
    return result
end

---@param source any
---@param path string
---@return MarkingConfig
local function copy_marking(source, path)
    assert_fields(source, MARKING_FIELD_SET, path)
    local result = {}
    for _, field in ipairs(MARKING_FIELDS) do
        result[field] = copy_scalar(source[field], path .. "." .. field)
    end
    ---@cast result MarkingConfig
    return result
end

---@param source any
---@param path string
---@return InputOwnership
local function copy_ownership(source, path)
    assert_fields(source, OWNERSHIP_FIELD_SET, path)
    assert(source.version == input_frame.VERSION, path .. " version is unsupported")
    assert_fields(source.rosters, TEAM_FIELD_SET, path .. ".rosters")
    assert_array(source.rosters.home, path .. ".rosters.home", input_frame.FIXTURE_TEAM_SIZE)
    assert_array(source.rosters.away, path .. ".rosters.away", input_frame.FIXTURE_TEAM_SIZE)
    assert_array(source.slots, path .. ".slots", input_frame.SLOT_COUNT)
    local rosters = { home = {}, away = {} }
    for _, team in ipairs({ "home", "away" }) do
        for index = 1, input_frame.FIXTURE_TEAM_SIZE do
            rosters[team][index] =
                copy_scalar(source.rosters[team][index], path .. ".rosters." .. team)
        end
    end
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local assignment = source.slots[index]
        assert_fields(assignment, ASSIGNMENT_FIELD_SET, path .. ".slots")
        slots[index] = {}
        for _, field in ipairs(ASSIGNMENT_FIELDS) do
            slots[index][field] =
                copy_scalar(assignment[field], path .. ".slots." .. index .. "." .. field)
        end
    end
    return { version = source.version, rosters = rosters, slots = slots }
end

---@param source any
---@param path string
---@param count integer
---@return table<integer, integer>
local function copy_sparse_indices(source, path, count)
    assert(type(source) == "table", path .. " must be a table")
    local result = {}
    for key, value in pairs(source) do
        assert(
            type(key) == "number" and key == math.floor(key) and key >= 1 and key <= count,
            path .. " has an invalid index"
        )
        assert(
            type(value) == "number" and value == math.floor(value),
            path .. " has a non-integer value"
        )
        result[key] = value
    end
    return result
end

---@param source any
---@param path string
---@param make_vec boolean
---@return MatchState
local function copy_state(source, path, make_vec)
    assert_fields(source, MATCH_FIELD_SET, path)
    assert_array(source.players, path .. ".players", 10)
    assert_array(source.events, path .. ".events")
    assert_fields(source.field, FIELD_FIELD_SET, path .. ".field")
    assert_fields(source.goal_home, RECT_FIELD_SET, path .. ".goal_home")
    assert_fields(source.goal_away, RECT_FIELD_SET, path .. ".goal_away")
    assert_fields(source.score, TEAM_FIELD_SET, path .. ".score")
    assert_fields(source.press, TEAM_FIELD_SET, path .. ".press")
    assert_fields(source.marking, TEAM_FIELD_SET, path .. ".marking")
    assert_fields(source.marks, TEAM_FIELD_SET, path .. ".marks")

    local result = {
        field = {
            w = copy_scalar(source.field.w, path .. ".field.w"),
            h = copy_scalar(source.field.h, path .. ".field.h"),
        },
        goal_home = {},
        goal_away = {},
        players = {},
        ball = copy_vec(source.ball, path .. ".ball", make_vec),
        ball_vel = copy_vec(source.ball_vel, path .. ".ball_vel", make_vec),
        score = {},
        press = {},
        marking = {},
        marks = {},
        events = {},
        slot_players = {},
        slot_for_player = {},
    }
    for _, goal_field in ipairs({ "x", "y", "w", "h" }) do
        result.goal_home[goal_field] =
            copy_scalar(source.goal_home[goal_field], path .. ".goal_home." .. goal_field)
        result.goal_away[goal_field] =
            copy_scalar(source.goal_away[goal_field], path .. ".goal_away." .. goal_field)
    end
    for index = 1, #source.players do
        result.players[index] =
            copy_player(source.players[index], path .. ".players." .. index, make_vec)
    end
    for _, field in ipairs({
        "ball_z",
        "ball_vz",
        "owner",
        "controlled",
        "human_controlled",
        "time_left",
        "max_goals",
        "finished",
        "pickup_cd",
        "ball_spin",
        "rng",
        "block_grace",
        "aerial_lock",
        "kickoff_hold",
        "slot_mode",
        "input_tick",
    }) do
        result[field] = copy_scalar(source[field], path .. "." .. field)
    end
    for _, team in ipairs({ "home", "away" }) do
        result.score[team] = copy_scalar(source.score[team], path .. ".score." .. team)
        result.press[team] = copy_scalar(source.press[team], path .. ".press." .. team)
        result.marking[team] = copy_marking(source.marking[team], path .. ".marking." .. team)
        result.marks[team] =
            copy_sparse_indices(source.marks[team], path .. ".marks." .. team, #source.players)
    end
    for index = 1, #source.events do
        result.events[index] = copy_event(source.events[index], path .. ".events." .. index)
    end
    result.input_ownership = source.input_ownership
            and copy_ownership(source.input_ownership, path .. ".input_ownership")
        or nil
    result.slot_players =
        copy_sparse_indices(source.slot_players, path .. ".slot_players", input_frame.SLOT_COUNT)
    result.slot_for_player =
        copy_sparse_indices(source.slot_for_player, path .. ".slot_for_player", #source.players)
    ---@cast result MatchState
    return result
end

-- Rollback owns states that have already crossed capture/restore's validating
-- boundary. These copies keep the same explicit schema and deep-ownership
-- contract without repeating allowlist checks or constructing diagnostic paths
-- at every predicted and resimulated boundary.
---@param value Vec2
---@param make_vec boolean
---@return Vec2
local function copy_owned_vec(value, make_vec)
    if make_vec then
        return Vec2.new(value.x, value.y)
    end
    return { x = value.x, y = value.y }
end

---@param source MatchPlayer
---@param make_vec boolean
---@return MatchPlayer
local function copy_owned_player(source, make_vec)
    local result = {}
    for _, field in ipairs(match_snapshot.PLAYER_FIELDS) do
        local value = source[field]
        if VECTOR_FIELDS[field] then
            result[field] = copy_owned_vec(value, make_vec)
        elseif OPTIONAL_VECTOR_FIELDS[field] then
            result[field] = value and copy_owned_vec(value, make_vec) or nil
        elseif field == "windup_shot" then
            if value then
                result.windup_shot = {
                    dir = copy_owned_vec(value.dir, make_vec),
                    speed = value.speed,
                    vz = value.vz,
                    spin = value.spin,
                    shot_type = value.shot_type,
                }
            end
        else
            result[field] = value
        end
    end
    ---@cast result MatchPlayer
    return result
end

---@param source MatchEvent
---@return MatchEvent
local function copy_owned_event(source)
    local result = {}
    for _, field in ipairs(EVENT_FIELDS) do
        result[field] = source[field]
    end
    ---@cast result MatchEvent
    return result
end

---@param source MarkingConfig
---@return MarkingConfig
local function copy_owned_marking(source)
    local result = {}
    for _, field in ipairs(MARKING_FIELDS) do
        result[field] = source[field]
    end
    ---@cast result MarkingConfig
    return result
end

---@param source InputOwnership
---@return InputOwnership
local function copy_owned_ownership(source)
    local rosters = { home = {}, away = {} }
    for _, team in ipairs({ "home", "away" }) do
        for index = 1, input_frame.FIXTURE_TEAM_SIZE do
            rosters[team][index] = source.rosters[team][index]
        end
    end
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local assignment = source.slots[index]
        slots[index] = {
            slot = assignment.slot,
            team = assignment.team,
            player_id = assignment.player_id,
        }
    end
    return { version = source.version, rosters = rosters, slots = slots }
end

---@param source table<integer, integer>
---@return table<integer, integer>
local function copy_owned_sparse_indices(source)
    local result = {}
    for key, value in pairs(source) do
        result[key] = value
    end
    return result
end

---@param source MatchState
---@param make_vec boolean
---@return MatchState
local function copy_owned_state(source, make_vec)
    local result = {
        field = { w = source.field.w, h = source.field.h },
        goal_home = {
            x = source.goal_home.x,
            y = source.goal_home.y,
            w = source.goal_home.w,
            h = source.goal_home.h,
        },
        goal_away = {
            x = source.goal_away.x,
            y = source.goal_away.y,
            w = source.goal_away.w,
            h = source.goal_away.h,
        },
        players = {},
        ball = copy_owned_vec(source.ball, make_vec),
        ball_vel = copy_owned_vec(source.ball_vel, make_vec),
        ball_z = source.ball_z,
        ball_vz = source.ball_vz,
        owner = source.owner,
        controlled = source.controlled,
        human_controlled = source.human_controlled,
        score = { home = source.score.home, away = source.score.away },
        time_left = source.time_left,
        max_goals = source.max_goals,
        finished = source.finished,
        pickup_cd = source.pickup_cd,
        press = { home = source.press.home, away = source.press.away },
        marking = {
            home = copy_owned_marking(source.marking.home),
            away = copy_owned_marking(source.marking.away),
        },
        marks = {
            home = copy_owned_sparse_indices(source.marks.home),
            away = copy_owned_sparse_indices(source.marks.away),
        },
        ball_spin = source.ball_spin,
        rng = source.rng,
        block_grace = source.block_grace,
        aerial_lock = source.aerial_lock,
        kickoff_hold = source.kickoff_hold,
        events = {},
        slot_mode = source.slot_mode,
        input_ownership = source.input_ownership and copy_owned_ownership(source.input_ownership)
            or nil,
        slot_players = copy_owned_sparse_indices(source.slot_players),
        slot_for_player = copy_owned_sparse_indices(source.slot_for_player),
        input_tick = source.input_tick,
    }
    for index = 1, #source.players do
        result.players[index] = copy_owned_player(source.players[index], make_vec)
    end
    for index = 1, #source.events do
        result.events[index] = copy_owned_event(source.events[index])
    end
    ---@cast result MatchState
    return result
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@return MatchSnapshot
function match_snapshot.capture(state, combat_state)
    assert_snapshot_supported(state, combat_state)
    local copied_state = copy_state(state, "state", false)
    if combat_state then
        return {
            version = match_snapshot.COMBAT_VERSION,
            state = copied_state,
            combat = combat_snapshot.copy(combat_state, "combat", copied_state, false),
        }
    end
    return { version = match_snapshot.VERSION, state = copied_state }
end

-- Internal ownership-transfer copy for a MatchState previously produced by a
-- validated snapshot or by sim.match. Arbitrary/external states must use
-- capture() so schema drift and malformed values still fail loudly.
---@param state MatchState
---@param combat_state CombatMatchState?
---@return MatchSnapshot
function match_snapshot.capture_owned(state, combat_state)
    assert(type(state) == "table", "owned match state is required")
    assert_snapshot_supported(state, combat_state)
    local copied_state = copy_owned_state(state, false)
    if combat_state then
        if copied_state.slot_mode then
            assert(
                combat_state.tick == copied_state.input_tick,
                "combat tick must match input tick"
            )
        end
        return {
            version = match_snapshot.COMBAT_VERSION,
            state = copied_state,
            combat = combat_snapshot.copy_owned(combat_state, false),
        }
    end
    return { version = match_snapshot.VERSION, state = copied_state }
end

---@param snapshot MatchSnapshot
---@return MatchState
---@return CombatMatchState?
function match_snapshot.restore(snapshot)
    assert_fields(snapshot, SNAPSHOT_FIELD_SET, "snapshot")
    assert(
        snapshot.version == match_snapshot.VERSION
            or snapshot.version == match_snapshot.COMBAT_VERSION,
        "unsupported match snapshot version"
    )
    local state = copy_state(snapshot.state, "snapshot.state", true)
    if snapshot.version == match_snapshot.VERSION then
        assert(snapshot.combat == nil, "soccer-only snapshots cannot contain combat state")
        return state, nil
    end
    assert(type(snapshot.combat) == "table", "combat snapshot companion is required")
    local combat_state = combat_snapshot.copy(snapshot.combat, "snapshot.combat", state, true)
    match_snapshot.mark_unsupported(
        state,
        "combat-active match snapshots require their CombatMatchState companion"
    )
    return state, combat_state
end

-- Internal restore for a canonical snapshot already owned by rollback. Public
-- and external snapshots must use restore() for full validation.
---@param snapshot MatchSnapshot
---@return MatchState
---@return CombatMatchState?
function match_snapshot.restore_owned(snapshot)
    assert(type(snapshot) == "table", "owned match snapshot is required")
    assert(
        snapshot.version == match_snapshot.VERSION
            or snapshot.version == match_snapshot.COMBAT_VERSION,
        "unsupported owned match snapshot version"
    )
    assert(type(snapshot.state) == "table", "owned match snapshot state is required")
    local state = copy_owned_state(snapshot.state, true)
    if snapshot.version == match_snapshot.VERSION then
        assert(snapshot.combat == nil, "owned soccer-only snapshot cannot contain combat state")
        return state, nil
    end
    assert(type(snapshot.combat) == "table", "owned combat snapshot companion is required")
    local combat_state = combat_snapshot.copy_owned(snapshot.combat, true)
    match_snapshot.mark_unsupported(
        state,
        "combat-active match snapshots require their CombatMatchState companion"
    )
    return state, combat_state
end

---@param number number
---@return string
function match_snapshot.number_bytes(number)
    assert(is_finite_number(number), "canonical numbers must be finite")
    if number == 0 then
        return (1 / number == -math.huge) and "Z" or "z"
    end
    local sign = number < 0 and "m" or "p"
    local mantissa, exponent = math.frexp(math.abs(number))
    local high = math.floor(mantissa * 67108864)
    local low = math.floor((mantissa * 67108864 - high) * 134217728 + 0.5)
    return sign .. ":" .. tostring(exponent) .. ":" .. tostring(high) .. ":" .. tostring(low)
end

---@param value integer
---@return integer
local function unsigned_decimal_bytes(value)
    if value < 10 then
        return 1
    elseif value < 100 then
        return 2
    elseif value < 1000 then
        return 3
    elseif value < 10000 then
        return 4
    elseif value < 100000 then
        return 5
    elseif value < 1000000 then
        return 6
    elseif value < 10000000 then
        return 7
    elseif value < 100000000 then
        return 8
    elseif value < 1000000000 then
        return 9
    elseif value < 10000000000 then
        return 10
    elseif value < 100000000000 then
        return 11
    elseif value < 1000000000000 then
        return 12
    elseif value < 10000000000000 then
        return 13
    elseif value < 100000000000000 then
        return 14
    elseif value < 1000000000000000 then
        return 15
    end
    return 16
end

---@param value integer
---@return integer
local function integer_decimal_bytes(value)
    if value < 0 then
        return unsigned_decimal_bytes(-value) + 1
    end
    return unsigned_decimal_bytes(value)
end

---@param number number
---@return integer
local function number_payload_bytes(number)
    assert(is_finite_number(number), "canonical numbers must be finite")
    if number == 0 then
        return 1
    end
    local mantissa, exponent = math.frexp(math.abs(number))
    ---@cast exponent integer
    local high = math.floor(mantissa * 67108864)
    local low = math.floor((mantissa * 67108864 - high) * 134217728 + 0.5)
    return 4
        + integer_decimal_bytes(exponent)
        + unsigned_decimal_bytes(high)
        + unsigned_decimal_bytes(low)
end

---@class MatchSnapshotEncoder
---@field parts string[]?
---@field bytes integer

---@type table<string, string>
local NAME_WIRES = {}

---@param encoder MatchSnapshotEncoder
---@param value string
local function append_literal(encoder, value)
    encoder.bytes = encoder.bytes + #value
    local parts = encoder.parts
    if parts then
        parts[#parts + 1] = value
    end
end

---@param encoder MatchSnapshotEncoder
---@param value any
local function append_scalar(encoder, value)
    local kind = type(value)
    if value == nil then
        append_literal(encoder, "z;")
    elseif kind == "boolean" then
        append_literal(encoder, value and "b1;" or "b0;")
    elseif kind == "number" then
        local parts = encoder.parts
        if parts then
            local payload = match_snapshot.number_bytes(value)
            encoder.bytes = encoder.bytes + #payload + 2
            parts[#parts + 1] = "n" .. payload .. ";"
        else
            encoder.bytes = encoder.bytes + number_payload_bytes(value) + 2
        end
    elseif kind == "string" then
        local parts = encoder.parts
        if parts then
            local length = tostring(#value)
            encoder.bytes = encoder.bytes + #length + #value + 3
            parts[#parts + 1] = "s" .. length .. ":" .. value .. ";"
        else
            encoder.bytes = encoder.bytes + unsigned_decimal_bytes(#value) + #value + 3
        end
    else
        assert(false, "unsupported canonical scalar")
    end
end

---@param encoder MatchSnapshotEncoder
---@param name string
local function append_name(encoder, name)
    local wire = NAME_WIRES[name]
    if wire == nil then
        wire = "k" .. tostring(#name) .. ":" .. name .. ";"
        NAME_WIRES[name] = wire
    end
    append_literal(encoder, wire)
end

---@param encoder MatchSnapshotEncoder
---@param value Vec2
local function append_vec(encoder, value)
    append_scalar(encoder, value.x)
    append_scalar(encoder, value.y)
end

---@param encoder MatchSnapshotEncoder
---@param value MarkingConfig
local function append_marking(encoder, value)
    for _, field in ipairs(MARKING_FIELDS) do
        append_name(encoder, field)
        append_scalar(encoder, value[field])
    end
end

---@param encoder MatchSnapshotEncoder
---@param value InputOwnership?
local function append_ownership(encoder, value)
    if not value then
        append_scalar(encoder, nil)
        return
    end
    append_scalar(encoder, value.version)
    for _, team in ipairs({ "home", "away" }) do
        append_name(encoder, team)
        for index = 1, input_frame.FIXTURE_TEAM_SIZE do
            append_scalar(encoder, value.rosters[team][index])
        end
    end
    for index = 1, input_frame.SLOT_COUNT do
        for _, field in ipairs(ASSIGNMENT_FIELDS) do
            append_name(encoder, field)
            append_scalar(encoder, value.slots[index][field])
        end
    end
end

---@param encoder MatchSnapshotEncoder
---@param player MatchPlayer
local function append_player(encoder, player)
    for _, field in ipairs(match_snapshot.PLAYER_FIELDS) do
        append_name(encoder, field)
        local value = player[field]
        if VECTOR_FIELDS[field] then
            append_vec(encoder, value)
        elseif OPTIONAL_VECTOR_FIELDS[field] then
            if value then
                append_literal(encoder, "v;")
                append_vec(encoder, value)
            else
                append_scalar(encoder, nil)
            end
        elseif field == "windup_shot" then
            if value then
                append_literal(encoder, "w;")
                for _, windup_field in ipairs(WINDUP_FIELDS) do
                    append_name(encoder, windup_field)
                    if windup_field == "dir" then
                        append_vec(encoder, value.dir)
                    else
                        append_scalar(encoder, value[windup_field])
                    end
                end
            else
                append_scalar(encoder, nil)
            end
        else
            append_scalar(encoder, value)
        end
    end
end

---@param encoder MatchSnapshotEncoder
---@param values table<integer, integer>
---@param count integer
local function append_sparse_indices(encoder, values, count)
    for index = 1, count do
        append_scalar(encoder, values[index])
    end
end

---@param encoder MatchSnapshotEncoder
---@param version integer
---@param state MatchState
local function append_state(encoder, version, state)
    append_literal(encoder, "GCMS;")
    append_scalar(encoder, version)
    for _, field in ipairs(match_snapshot.MATCH_FIELDS) do
        append_name(encoder, field)
        local value = state[field]
        if field == "field" then
            append_scalar(encoder, value.w)
            append_scalar(encoder, value.h)
        elseif field == "goal_home" or field == "goal_away" then
            for _, rect_field in ipairs({ "x", "y", "w", "h" }) do
                append_scalar(encoder, value[rect_field])
            end
        elseif field == "players" then
            append_scalar(encoder, #value)
            for index = 1, #value do
                append_player(encoder, value[index])
            end
        elseif field == "ball" or field == "ball_vel" then
            append_vec(encoder, value)
        elseif field == "score" or field == "press" then
            append_scalar(encoder, value.home)
            append_scalar(encoder, value.away)
        elseif field == "marking" then
            append_marking(encoder, value.home)
            append_marking(encoder, value.away)
        elseif field == "marks" then
            append_sparse_indices(encoder, value.home, #state.players)
            append_sparse_indices(encoder, value.away, #state.players)
        elseif field == "events" then
            append_scalar(encoder, #value)
            for index = 1, #value do
                for _, event_field in ipairs(EVENT_FIELDS) do
                    append_name(encoder, event_field)
                    append_scalar(encoder, value[index][event_field])
                end
            end
        elseif field == "input_ownership" then
            append_ownership(encoder, value)
        elseif field == "slot_players" then
            append_sparse_indices(encoder, value, input_frame.SLOT_COUNT)
        elseif field == "slot_for_player" then
            append_sparse_indices(encoder, value, #state.players)
        else
            append_scalar(encoder, value)
        end
    end
end

---@param version integer
---@param state MatchState
---@param combat_state CombatMatchState?
---@param encoder MatchSnapshotEncoder
local function append_snapshot(encoder, version, state, combat_state)
    append_state(encoder, version, state)
    if version == match_snapshot.COMBAT_VERSION then
        append_name(encoder, "combat")
        append_literal(encoder, "c;")
        local writer = {
            literal = function(value)
                append_literal(encoder, value)
            end,
            name = function(value)
                append_name(encoder, value)
            end,
            scalar = function(value)
                append_scalar(encoder, value)
            end,
            vec = function(value)
                append_vec(encoder, value)
            end,
        }
        combat_snapshot.append(writer, assert(combat_state))
    end
end

---@param version integer
---@param state MatchState
---@param combat_state CombatMatchState?
---@return string
local function encode_snapshot(version, state, combat_state)
    local encoder = { parts = {}, bytes = 0 }
    append_snapshot(encoder, version, state, combat_state)
    return table.concat(assert(encoder.parts))
end

---@param version integer
---@param state MatchState
---@param combat_state CombatMatchState?
---@return integer
local function encoded_snapshot_size(version, state, combat_state)
    local encoder = { parts = nil, bytes = 0 }
    append_snapshot(encoder, version, state, combat_state)
    return encoder.bytes
end

---@param snapshot MatchSnapshot
---@return string
function match_snapshot.encode(snapshot)
    -- Restore performs the full explicit allowlist/type validation and gives
    -- serialization a normalized independent source.
    local state, combat_state = match_snapshot.restore(snapshot)
    return encode_snapshot(snapshot.version, state, combat_state)
end

-- Encode a snapshot just returned by capture(), or an equally canonical
-- owned copy, without repeating its full restore/validation pass.
---@param snapshot MatchSnapshot
---@return string
function match_snapshot.encode_canonical(snapshot)
    assert(type(snapshot) == "table", "canonical match snapshot is required")
    assert(
        snapshot.version == match_snapshot.VERSION
            or snapshot.version == match_snapshot.COMBAT_VERSION,
        "unsupported canonical snapshot version"
    )
    assert(type(snapshot.state) == "table", "canonical match snapshot state is required")
    return encode_snapshot(snapshot.version, snapshot.state, snapshot.combat)
end

-- Count the exact canonical wire bytes for an owned snapshot without
-- materializing its wire string.
---@param snapshot MatchSnapshot
---@return integer
function match_snapshot.encoded_size_canonical(snapshot)
    assert(type(snapshot) == "table", "canonical match snapshot is required")
    assert(
        snapshot.version == match_snapshot.VERSION
            or snapshot.version == match_snapshot.COMBAT_VERSION,
        "unsupported canonical snapshot version"
    )
    assert(type(snapshot.state) == "table", "canonical match snapshot state is required")
    return encoded_snapshot_size(snapshot.version, snapshot.state, snapshot.combat)
end

---@param snapshot MatchSnapshot
---@return string
function match_snapshot.hash(snapshot)
    return fnv1a64.hash(match_snapshot.encode(snapshot))
end

-- Hash a snapshot already returned by capture(), or an equally canonical
-- owned copy, without repeating its restore/validation pass.
---@param snapshot MatchSnapshot
---@return string
function match_snapshot.hash_canonical(snapshot)
    return fnv1a64.hash(match_snapshot.encode_canonical(snapshot))
end

---@param left any
---@param right any
---@return boolean
local function same_scalar(left, right)
    if left ~= right then
        return false
    end
    if left == 0 and right == 0 then
        return 1 / left == 1 / right
    end
    return true
end

---@param path string
---@param expected any
---@param actual any
---@return MatchSnapshotDifference
local function difference(path, expected, actual)
    return { path = path, expected = expected, actual = actual }
end

---@param path string
---@param left Vec2
---@param right Vec2
---@return MatchSnapshotDifference?
local function compare_vec(path, left, right)
    if not same_scalar(left.x, right.x) then
        return difference(path .. ".x", left.x, right.x)
    end
    if not same_scalar(left.y, right.y) then
        return difference(path .. ".y", left.y, right.y)
    end
    return nil
end

---@param a MatchSnapshot
---@param b MatchSnapshot
---@return MatchSnapshotDifference?
local function first_difference_canonical(a, b)
    if a.version ~= b.version then
        return difference("version", a.version, b.version)
    end
    local sa, sb = a.state, b.state
    for _, field in ipairs(match_snapshot.MATCH_FIELDS) do
        local path = "state." .. field
        local av, bv = sa[field], sb[field]
        if field == "field" then
            for _, child in ipairs({ "w", "h" }) do
                if not same_scalar(av[child], bv[child]) then
                    return difference(path .. "." .. child, av[child], bv[child])
                end
            end
        elseif field == "goal_home" or field == "goal_away" then
            for _, child in ipairs({ "x", "y", "w", "h" }) do
                if not same_scalar(av[child], bv[child]) then
                    return difference(path .. "." .. child, av[child], bv[child])
                end
            end
        elseif field == "players" then
            if #av ~= #bv then
                return difference(path .. ".length", #av, #bv)
            end
            for index = 1, #av do
                for _, child in ipairs(match_snapshot.PLAYER_FIELDS) do
                    local child_path = path .. "." .. index .. "." .. child
                    local ac, bc = av[index][child], bv[index][child]
                    if VECTOR_FIELDS[child] then
                        local found = compare_vec(child_path, ac, bc)
                        if found then
                            return found
                        end
                    elseif OPTIONAL_VECTOR_FIELDS[child] then
                        if (ac == nil) ~= (bc == nil) then
                            return difference(child_path, ac, bc)
                        elseif ac then
                            local found = compare_vec(child_path, ac, bc)
                            if found then
                                return found
                            end
                        end
                    elseif child == "windup_shot" then
                        if (ac == nil) ~= (bc == nil) then
                            return difference(child_path, ac, bc)
                        elseif ac then
                            for _, shot_field in ipairs(WINDUP_FIELDS) do
                                if shot_field == "dir" then
                                    local found = compare_vec(
                                        child_path .. "." .. shot_field,
                                        ac[shot_field],
                                        bc[shot_field]
                                    )
                                    if found then
                                        return found
                                    end
                                elseif not same_scalar(ac[shot_field], bc[shot_field]) then
                                    return difference(
                                        child_path .. "." .. shot_field,
                                        ac[shot_field],
                                        bc[shot_field]
                                    )
                                end
                            end
                        end
                    elseif not same_scalar(ac, bc) then
                        return difference(child_path, ac, bc)
                    end
                end
            end
        elseif field == "ball" or field == "ball_vel" then
            local found = compare_vec(path, av, bv)
            if found then
                return found
            end
        elseif field == "score" or field == "press" then
            for _, team in ipairs({ "home", "away" }) do
                if not same_scalar(av[team], bv[team]) then
                    return difference(path .. "." .. team, av[team], bv[team])
                end
            end
        elseif field == "marking" then
            for _, team in ipairs({ "home", "away" }) do
                for _, child in ipairs(MARKING_FIELDS) do
                    if not same_scalar(av[team][child], bv[team][child]) then
                        return difference(
                            path .. "." .. team .. "." .. child,
                            av[team][child],
                            bv[team][child]
                        )
                    end
                end
            end
        elseif field == "marks" then
            for _, team in ipairs({ "home", "away" }) do
                for index = 1, #sa.players do
                    if not same_scalar(av[team][index], bv[team][index]) then
                        return difference(
                            path .. "." .. team .. "." .. index,
                            av[team][index],
                            bv[team][index]
                        )
                    end
                end
            end
        elseif field == "events" then
            if #av ~= #bv then
                return difference(path .. ".length", #av, #bv)
            end
            for index = 1, #av do
                for _, child in ipairs(EVENT_FIELDS) do
                    if not same_scalar(av[index][child], bv[index][child]) then
                        return difference(
                            path .. "." .. index .. "." .. child,
                            av[index][child],
                            bv[index][child]
                        )
                    end
                end
            end
        elseif field == "input_ownership" then
            if (av == nil) ~= (bv == nil) then
                return difference(path, av, bv)
            elseif av then
                if av.version ~= bv.version then
                    return difference(path .. ".version", av.version, bv.version)
                end
                for _, team in ipairs({ "home", "away" }) do
                    for index = 1, input_frame.FIXTURE_TEAM_SIZE do
                        if av.rosters[team][index] ~= bv.rosters[team][index] then
                            return difference(
                                path .. ".rosters." .. team .. "." .. index,
                                av.rosters[team][index],
                                bv.rosters[team][index]
                            )
                        end
                    end
                end
                for index = 1, input_frame.SLOT_COUNT do
                    for _, child in ipairs(ASSIGNMENT_FIELDS) do
                        if av.slots[index][child] ~= bv.slots[index][child] then
                            return difference(
                                path .. ".slots." .. index .. "." .. child,
                                av.slots[index][child],
                                bv.slots[index][child]
                            )
                        end
                    end
                end
            end
        elseif field == "slot_players" or field == "slot_for_player" then
            local count = field == "slot_players" and input_frame.SLOT_COUNT or #sa.players
            for index = 1, count do
                if not same_scalar(av[index], bv[index]) then
                    return difference(path .. "." .. index, av[index], bv[index])
                end
            end
        elseif not same_scalar(av, bv) then
            return difference(path, av, bv)
        end
    end
    return combat_snapshot.first_difference(a.combat, b.combat, "combat")
end

---@param left MatchSnapshot
---@param right MatchSnapshot
---@return MatchSnapshotDifference?
function match_snapshot.first_difference(left, right)
    local left_state, left_combat = match_snapshot.restore(left)
    local right_state, right_combat = match_snapshot.restore(right)
    local a = match_snapshot.capture(left_state, left_combat)
    local b = match_snapshot.capture(right_state, right_combat)
    return first_difference_canonical(a, b)
end

-- Compare snapshots already returned by capture(), or equally canonical owned
-- copies, without repeating their full restore/capture validation pass.
---@param left MatchSnapshot
---@param right MatchSnapshot
---@return MatchSnapshotDifference?
function match_snapshot.first_difference_canonical(left, right)
    assert(type(left) == "table", "left canonical match snapshot is required")
    assert(type(right) == "table", "right canonical match snapshot is required")
    assert(type(left.state) == "table", "left canonical match snapshot state is required")
    assert(type(right.state) == "table", "right canonical match snapshot state is required")
    return first_difference_canonical(left, right)
end

return match_snapshot
