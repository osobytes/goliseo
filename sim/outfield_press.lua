-- Serializable team-owned presser state and pure pressing geometry.
-- Match owns world eligibility and trigger inputs; this module owns stable
-- arbitration, conservative tuning, and the compact versioned value contract.

local Vec2 = require("core.vec2")
local brain = require("sim.brain")

---@alias StablePressMode "inactive"|"contain"|"commit"

---@class OutfieldPressState
---@field version integer
---@field presser_index integer?
---@field mode StablePressMode
---@field reason PressReason

---@class OutfieldPressCandidate
---@field player_index integer
---@field distance_cost number
---@field eligible boolean?

---@class OutfieldLaneCandidate
---@field player_index integer
---@field score number
---@field pos Vec2
---@field eligible boolean?

---@class OutfieldPressContext
---@field heavy_touch boolean
---@field exposed_ball boolean
---@field cover_available boolean
---@field box_desperation boolean
---@field press_discipline number

---@class OutfieldPressModule
local outfield_press = {}

outfield_press.VERSION = 1

local PRESSER_SWITCH_RATIO = 0.15
local LOW_DISCIPLINE_THRESHOLD = 0.35
local COVER_SUPPORT_DISTANCE = 140
local CONTAIN_CENTER_BIAS = 8
local CONTAIN_SLOW_RADIUS = 60
local LANE_SHADOW_WEIGHT = 0.35
local LANE_POINT_FRACTION = 0.55

local MODES = {
    inactive = true,
    contain = true,
    commit = true,
}

local REASONS = {
    no_trigger = true,
    heavy_touch = true,
    exposed_ball = true,
    cover = true,
    box_desperation = true,
    low_discipline = true,
}

local COMMIT_REASONS = {
    heavy_touch = true,
    exposed_ball = true,
    cover = true,
    box_desperation = true,
    low_discipline = true,
}

---@param value number
---@return boolean
local function is_finite(value)
    return value == value and value ~= math.huge and value ~= -math.huge
end

---@param state OutfieldPressState
local function validate_state(state)
    assert(type(state) == "table", "outfield press state must be a table")
    assert(state.version == outfield_press.VERSION, "unsupported outfield press version")
    assert(MODES[state.mode], "unknown outfield press mode")
    assert(REASONS[state.reason], "unknown outfield press reason")
    if state.presser_index ~= nil then
        assert(
            type(state.presser_index) == "number"
                and is_finite(state.presser_index)
                and state.presser_index == math.floor(state.presser_index)
                and state.presser_index >= 1,
            "outfield presser index must be a positive integer"
        )
    end
    if state.mode == "inactive" then
        assert(state.presser_index == nil, "inactive outfield press cannot retain a presser")
        assert(
            state.reason == "no_trigger",
            "inactive outfield press cannot retain a commit reason"
        )
    elseif state.mode == "contain" then
        assert(state.presser_index ~= nil, "contain outfield press requires a presser")
        assert(state.reason == "no_trigger", "contain outfield press cannot retain a commit reason")
    else
        assert(state.presser_index ~= nil, "commit outfield press requires a presser")
        assert(COMMIT_REASONS[state.reason], "commit outfield press requires a commit reason")
    end
end

---@return OutfieldPressState
function outfield_press.new_state()
    return {
        version = outfield_press.VERSION,
        presser_index = nil,
        mode = "inactive",
        reason = "no_trigger",
    }
end

---@param state OutfieldPressState
---@return OutfieldPressState
function outfield_press.clear(state)
    validate_state(state)
    if state.mode == "inactive" then
        return state
    end
    return outfield_press.new_state()
end

---@param state OutfieldPressState
---@return OutfieldPressState
function outfield_press.copy_state(state)
    validate_state(state)
    return {
        version = state.version,
        presser_index = state.presser_index,
        mode = state.mode,
        reason = state.reason,
    }
end

---@param presser_index integer
---@param context OutfieldPressContext
---@return OutfieldPressState
function outfield_press.resolve(presser_index, context)
    local mode, reason = brain.press_mode({
        heavy_touch = context.heavy_touch,
        exposed_ball = context.exposed_ball,
        cover_available = context.cover_available,
        box_desperation = context.box_desperation,
        press_discipline = context.press_discipline,
        low_discipline_threshold = LOW_DISCIPLINE_THRESHOLD,
    })
    local state = {
        version = outfield_press.VERSION,
        presser_index = presser_index,
        mode = mode,
        reason = reason,
    }
    ---@cast state OutfieldPressState
    return outfield_press.copy_state(state)
end

---@param presser_index integer
---@return OutfieldPressState
function outfield_press.contain(presser_index)
    local state = {
        version = outfield_press.VERSION,
        presser_index = presser_index,
        mode = "contain",
        reason = "no_trigger",
    }
    ---@cast state OutfieldPressState
    return outfield_press.copy_state(state)
end

---@param candidates OutfieldPressCandidate[]
---@param current integer?
---@return integer?
function outfield_press.assign_presser(candidates, current)
    return brain.assign_presser(candidates, current, PRESSER_SWITCH_RATIO)
end

---@param distance number
---@param goal_side boolean
---@return boolean
function outfield_press.cover_available(distance, goal_side)
    assert(is_finite(distance) and distance >= 0, "cover support distance must be non-negative")
    return goal_side and distance <= COVER_SUPPORT_DISTANCE
end

---@param base_speed number
---@param target_distance number
---@param contain_factor number
---@return number
function outfield_press.contain_speed(base_speed, target_distance, contain_factor)
    assert(is_finite(base_speed) and base_speed >= 0, "contain base speed must be non-negative")
    assert(
        is_finite(target_distance) and target_distance >= 0,
        "contain target distance must be non-negative"
    )
    assert(is_finite(contain_factor), "contain speed factor must be finite")
    if target_distance <= CONTAIN_SLOW_RADIUS then
        return math.min(base_speed, base_speed * math.max(0, math.min(1, contain_factor)))
    end
    return base_speed
end

---@param carrier Vec2
---@param goal Vec2
---@param field_height number
---@param standoff number
---@return Vec2
function outfield_press.contain_target(carrier, goal, field_height, standoff)
    local target = carrier:add(goal:sub(carrier):normalized():scale(standoff))
    local center_delta = field_height / 2 - carrier.y
    if center_delta == 0 then
        return target
    end
    return Vec2.new(target.x, target.y + (center_delta > 0 and 1 or -1) * CONTAIN_CENTER_BIAS)
end

---@param candidates OutfieldLaneCandidate[]
---@return OutfieldLaneCandidate?
function outfield_press.highest_scored_lane(candidates)
    local best = nil
    for index, candidate in ipairs(candidates) do
        assert(
            type(candidate.player_index) == "number"
                and candidate.player_index == math.floor(candidate.player_index)
                and candidate.player_index >= 1,
            "passing lane candidate " .. index .. " player must be a positive integer"
        )
        assert(is_finite(candidate.score), "passing lane candidate score must be finite")
        if
            candidate.eligible ~= false
            and (
                best == nil
                or candidate.score > best.score
                or (candidate.score == best.score and candidate.player_index < best.player_index)
            )
        then
            best = candidate
        end
    end
    return best
end

---@param base Vec2
---@param carrier Vec2
---@param candidates OutfieldLaneCandidate[]
---@return Vec2
function outfield_press.lane_shadow_target(base, carrier, candidates)
    local receiver = outfield_press.highest_scored_lane(candidates)
    if not receiver then
        return base
    end
    local lane_point = carrier:add(receiver.pos:sub(carrier):scale(LANE_POINT_FRACTION))
    return base:add(lane_point:sub(base):scale(LANE_SHADOW_WEIGHT))
end

return outfield_press
