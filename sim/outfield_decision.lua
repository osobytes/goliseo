-- Serializable personal decision cadence and carrier-option construction.
-- Callers own world geometry and execution; this module retains only value
-- state and resolves deterministic numeric contexts through sim.brain.

local brain = require("sim.brain")
local rng = require("core.rng")

---@alias OutfieldDecisionContext "ineligible"|"offball"|"carrier"
---@alias OutfieldIntent "none"|"move"|"in_behind"|"come_short"|"hold_width"|"shoot"|"cross"|"pass"|"dribble"

---@class OutfieldDecisionState
---@field version integer
---@field generation integer
---@field rng_state integer
---@field remaining number
---@field context OutfieldDecisionContext
---@field intent OutfieldIntent
---@field target_x number?
---@field target_y number?
---@field target_player integer?
---@field run_expires_at number?

---@class OutfieldPassOption
---@field player_index integer
---@field openness number
---@field forward_progress number
---@field distance number
---@field lane_blocked boolean
---@field interception_risk boolean
---@field lane_fraction number?

---@class OutfieldCarrierContext
---@field goal_distance number
---@field shoot_range number
---@field angle_quality number
---@field keeper_coverage number
---@field space number
---@field flank_depth number
---@field cross_target integer?
---@field box_targets integer
---@field cross_space number
---@field goal_progress number
---@field dribble_space number
---@field passes OutfieldPassOption[]

---@class OutfieldDecisionModule
local outfield_decision = {}

outfield_decision.VERSION = 2
outfield_decision.SLOW_REFRESH_SECONDS = 0.45
outfield_decision.FAST_REFRESH_SECONDS = 0.15
outfield_decision.BASE_TEMPERATURE = 3
outfield_decision.RUN_LIFETIME_SECONDS = 1.8

local SHARP_COMPOSURE = 0.8
local PASS_RISK_PENALTY = 5
local PASS_SPACE_BALANCE = 0.6
local PASS_SPACE_WEIGHT = 90

local CONTEXTS = {
    ineligible = true,
    offball = true,
    carrier = true,
}

local INTENTS = {
    none = true,
    move = true,
    in_behind = true,
    come_short = true,
    hold_width = true,
    shoot = true,
    cross = true,
    pass = true,
    dribble = true,
}

local RUN_INTENTS = {
    in_behind = true,
    come_short = true,
    hold_width = true,
}

local CARRIER_INTENTS = {
    shoot = true,
    cross = true,
    pass = true,
    dribble = true,
}

---@param value number
---@return boolean
local function is_finite(value)
    return value == value and value ~= math.huge and value ~= -math.huge
end

---@param value number
---@param label string
local function assert_finite(value, label)
    assert(type(value) == "number" and is_finite(value), label .. " must be finite")
end

---@param value number
---@return number
local function clamp_unit(value)
    if value ~= value or value == -math.huge then
        return 0
    end
    if value == math.huge then
        return 1
    end
    return math.max(0, math.min(1, value))
end

---@param rng_state integer?
---@return OutfieldDecisionState
function outfield_decision.new_state(rng_state)
    return {
        version = outfield_decision.VERSION,
        generation = 0,
        rng_state = rng.seed(rng_state or 1),
        remaining = 0,
        context = "ineligible",
        intent = "none",
        target_x = nil,
        target_y = nil,
        target_player = nil,
        run_expires_at = nil,
    }
end

---@param state OutfieldDecisionState
local function validate_state(state)
    assert(type(state) == "table", "outfield decision state must be a table")
    assert(state.version == outfield_decision.VERSION, "unsupported outfield decision version")
    assert(
        type(state.generation) == "number"
            and is_finite(state.generation)
            and state.generation == math.floor(state.generation)
            and state.generation >= 0,
        "outfield decision generation must be a non-negative integer"
    )
    assert(
        type(state.rng_state) == "number"
            and is_finite(state.rng_state)
            and state.rng_state == math.floor(state.rng_state)
            and rng.seed(state.rng_state) == state.rng_state,
        "outfield decision RNG state must be a canonical positive integer"
    )
    assert_finite(state.remaining, "outfield decision remaining time")
    assert(state.remaining >= 0, "outfield decision remaining time must be non-negative")
    assert(CONTEXTS[state.context], "unknown outfield decision context")
    assert(INTENTS[state.intent], "unknown outfield decision intent")
    if state.target_x ~= nil then
        assert_finite(state.target_x, "outfield decision target x")
    end
    if state.target_y ~= nil then
        assert_finite(state.target_y, "outfield decision target y")
    end
    assert(
        (state.target_x == nil) == (state.target_y == nil),
        "outfield decision target coordinates must be paired"
    )
    if state.target_player ~= nil then
        assert(
            type(state.target_player) == "number"
                and is_finite(state.target_player)
                and state.target_player == math.floor(state.target_player)
                and state.target_player >= 1,
            "outfield decision target player must be a positive integer"
        )
    end
    if state.run_expires_at ~= nil then
        assert_finite(state.run_expires_at, "outfield decision run expiry")
    end
    if state.context == "ineligible" then
        assert(state.intent == "none", "ineligible outfield decision state must have no intent")
        assert(state.remaining == 0, "ineligible outfield decision state cannot retain cadence")
        assert(
            state.target_x == nil and state.target_player == nil,
            "ineligible outfield decision state cannot retain a target"
        )
        assert(
            state.run_expires_at == nil,
            "ineligible outfield decision state cannot retain a run expiry"
        )
    elseif state.context == "offball" then
        assert(
            state.intent == "move" or RUN_INTENTS[state.intent],
            "offball outfield decision state must retain a movement intent"
        )
        assert(state.target_x ~= nil, "offball movement intent requires a target point")
        assert(state.target_player == nil, "offball movement intent cannot target a player")
        if RUN_INTENTS[state.intent] then
            assert(state.run_expires_at ~= nil, "offball run intent requires an absolute expiry")
        else
            assert(
                state.run_expires_at == nil,
                "ordinary offball movement cannot retain a run expiry"
            )
        end
    else
        assert(state.context == "carrier", "unknown eligible outfield decision context")
        assert(
            CARRIER_INTENTS[state.intent],
            "carrier outfield decision state requires a carrier intent"
        )
        if state.intent == "pass" or state.intent == "cross" then
            assert(state.target_player ~= nil, "pass and cross intents require a target player")
            assert(state.target_x == nil, "pass and cross intents cannot retain a target point")
        else
            assert(
                state.target_x == nil and state.target_player == nil,
                "shoot and dribble intents cannot retain a target"
            )
        end
        assert(
            state.run_expires_at == nil,
            "carrier outfield decision state cannot retain a run expiry"
        )
    end
end

---@param intent OutfieldIntent
---@return boolean
function outfield_decision.is_run_intent(intent)
    return RUN_INTENTS[intent] == true
end

---@param state OutfieldDecisionState
---@return OutfieldDecisionState
function outfield_decision.copy_state(state)
    validate_state(state)
    return {
        version = state.version,
        generation = state.generation,
        rng_state = state.rng_state,
        remaining = state.remaining,
        context = state.context,
        intent = state.intent,
        target_x = state.target_x,
        target_y = state.target_y,
        target_player = state.target_player,
        run_expires_at = state.run_expires_at,
    }
end

---@param state OutfieldDecisionState
---@param dt number
---@return OutfieldDecisionState
function outfield_decision.advance(state, dt)
    local next_state = outfield_decision.copy_state(state)
    assert_finite(dt, "outfield decision delta")
    assert(dt >= 0, "outfield decision delta must be non-negative")
    next_state.remaining = math.max(0, next_state.remaining - dt)
    return next_state
end

---@param state OutfieldDecisionState
---@return OutfieldDecisionState
function outfield_decision.reset(state)
    local current = outfield_decision.copy_state(state)
    local reset = outfield_decision.new_state(current.rng_state)
    reset.generation = current.generation
    return reset
end

---@param state OutfieldDecisionState
---@param rng_state integer
---@return OutfieldDecisionState
function outfield_decision.with_rng_state(state, rng_state)
    local next_state = outfield_decision.copy_state(state)
    next_state.rng_state = rng_state
    return outfield_decision.copy_state(next_state)
end

-- End an active run without inventing a cadence boundary. The caller supplies
-- the ordinary support fallback that takes over for the retained countdown.
---@param state OutfieldDecisionState
---@param target_x number
---@param target_y number
---@return OutfieldDecisionState
function outfield_decision.cancel_run(state, target_x, target_y)
    local next_state = outfield_decision.copy_state(state)
    assert(RUN_INTENTS[next_state.intent], "cancel_run requires an active run intent")
    next_state.intent = "move"
    next_state.target_x = target_x
    next_state.target_y = target_y
    next_state.target_player = nil
    next_state.run_expires_at = nil
    return outfield_decision.copy_state(next_state)
end

---@param state OutfieldDecisionState
---@param context OutfieldDecisionContext
---@param urgent boolean?
---@return boolean
function outfield_decision.should_refresh(state, context, urgent)
    validate_state(state)
    assert(CONTEXTS[context], "unknown requested outfield decision context")
    return context ~= "ineligible"
        and (urgent == true or state.context ~= context or state.remaining <= 0)
end

---@param state OutfieldDecisionState
---@param context OutfieldDecisionContext
---@param intent OutfieldIntent
---@param scan_rate number
---@param target_x number?
---@param target_y number?
---@param target_player integer?
---@param run_expires_at number?
---@return OutfieldDecisionState
function outfield_decision.refresh(
    state,
    context,
    intent,
    scan_rate,
    target_x,
    target_y,
    target_player,
    run_expires_at
)
    validate_state(state)
    assert(context ~= "ineligible" and CONTEXTS[context], "refresh requires an eligible context")
    local refreshed = {
        version = outfield_decision.VERSION,
        generation = state.generation + 1,
        rng_state = state.rng_state,
        remaining = brain.refresh_interval(
            scan_rate,
            outfield_decision.SLOW_REFRESH_SECONDS,
            outfield_decision.FAST_REFRESH_SECONDS
        ),
        context = context,
        intent = intent,
        target_x = target_x,
        target_y = target_y,
        target_player = target_player,
        run_expires_at = run_expires_at,
    }
    return outfield_decision.copy_state(refreshed)
end

---@param context OutfieldCarrierContext
---@return CarrierOption[]
function outfield_decision.carrier_options(context)
    assert_finite(context.goal_distance, "carrier goal distance")
    assert(context.goal_distance >= 0, "carrier goal distance must be non-negative")
    assert_finite(context.shoot_range, "carrier shoot range")
    assert(context.shoot_range > 0, "carrier shoot range must be positive")
    assert_finite(context.space, "carrier space")
    assert_finite(context.cross_space, "carrier cross space")
    assert_finite(context.flank_depth, "carrier flank depth")
    assert_finite(context.goal_progress, "carrier goal progress")
    assert_finite(context.dribble_space, "carrier dribble space")
    assert(
        type(context.box_targets) == "number"
            and context.box_targets == math.floor(context.box_targets)
            and context.box_targets >= 0,
        "carrier box target count must be a non-negative integer"
    )

    local distance_falloff = context.goal_distance / context.shoot_range
    local options = {
        {
            id = "shoot",
            kind = "shoot",
            score = 100 - distance_falloff * distance_falloff * 60 + clamp_unit(
                context.angle_quality
            ) * 25 - clamp_unit(context.keeper_coverage) * 12 + clamp_unit(
                context.space
            ) * 18,
        },
        {
            id = "dribble",
            kind = "dribble",
            score = 10
                + clamp_unit(context.dribble_space) * 35
                + clamp_unit(context.goal_progress) * 25,
        },
    }

    if context.cross_target ~= nil and context.box_targets > 0 then
        options[#options + 1] = {
            id = "cross",
            kind = "cross",
            score = 60
                + clamp_unit(context.flank_depth) * 45
                + math.min(3, context.box_targets) * 8
                + clamp_unit(context.cross_space) * 15,
            reference = context.cross_target,
        }
    end

    local seen = {}
    for index, pass in ipairs(context.passes) do
        assert(
            type(pass.player_index) == "number"
                and pass.player_index == math.floor(pass.player_index)
                and pass.player_index >= 1,
            "pass option " .. index .. " player must be a positive integer"
        )
        assert(not seen[pass.player_index], "pass option players must be unique")
        seen[pass.player_index] = true
        assert_finite(pass.openness, "pass option openness")
        assert_finite(pass.forward_progress, "pass option forward progress")
        assert_finite(pass.distance, "pass option distance")
        assert(pass.distance >= 0, "pass option distance must be non-negative")
        if pass.lane_fraction ~= nil then
            assert_finite(pass.lane_fraction, "pass option lane fraction")
        end
        options[#options + 1] = {
            id = "pass_" .. tostring(pass.player_index),
            kind = "pass",
            score = 18 + math.min(1, math.max(0, pass.openness) / 150) * 45 + math.max(
                -1,
                math.min(1, pass.forward_progress / 300)
            ) * 28 - math.min(1, pass.distance / 420) * 18 + (PASS_SPACE_BALANCE - clamp_unit(
                context.space
            )) * PASS_SPACE_WEIGHT - (pass.interception_risk and PASS_RISK_PENALTY or 0),
            payload = pass.lane_fraction and { lane_fraction = pass.lane_fraction } or nil,
            reference = pass.player_index,
        }
    end
    return options
end

---@param options CarrierOption[]
---@param composure number
---@param pressure number
---@param rng_state integer
---@return CarrierOption selected
---@return integer next_rng_state
function outfield_decision.decide_carrier(options, composure, pressure, rng_state)
    -- At the authored high-composure boundary the temperature is exactly zero:
    -- an effectively settled choice takes the argmax without needlessly
    -- advancing its dedicated decision RNG stream.
    local base_temperature = composure >= SHARP_COMPOSURE and 0
        or outfield_decision.BASE_TEMPERATURE
    return brain.decide_carrier(options, composure, pressure, base_temperature, rng_state)
end

return outfield_decision
