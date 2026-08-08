-- Versioned learning-environment actions.
--
-- An action is the same abstract intent a human or a #112 bot expresses: a
-- continuous move vector plus named held actions and one-shot edges. It is
-- quantized into the canonical InputSample that sim.match already consumes, so a
-- policy cannot address resolver internals, teleport, set ownership or outcomes,
-- skip recovery or cooldowns, or act between fixed ticks. Anything a policy could
-- want that is not in this contract simply does not exist as an input.
--
-- Legality masks are derived from an observation, not from MatchState. A mask
-- built from a representative view therefore cannot contain privileged legality
-- by construction; a mask built from the privileged profile is tagged as such.

local input_frame = require("sim.input_frame")

---@alias EnvActionErrorCode
---| "malformed"
---| "move_out_of_range"
---| "unknown_held_action"
---| "unknown_edge_action"
---| "unavailable_action"

---@class EnvSlotAction
---@field version integer? -- Defaults to EnvAction.VERSION when omitted.
---@field move { x: number, y: number }? -- Continuous, inside the unit disc; nil means still.
---@field aim { x: number, y: number }? -- Continuous aim direction; nil means aim follows move.
---@field held table<InputHeldAction, boolean>? -- Continuously held intents for this tick.
---@field edges table<InputEdgeAction, boolean>? -- One-shot intents fired on this tick only.

---@class EnvActionMask
---@field version integer
---@field profile EnvObservationProfile
---@field privileged boolean -- True when the mask was derived from a privileged view.
---@field slot integer
---@field move boolean -- Movement is always legal; kept explicit for consumers.
---@field aim boolean -- Aiming is always legal; kept explicit for consumers.
---@field held table<InputHeldAction, boolean>
---@field edges table<InputEdgeAction, boolean>

---@class EnvActionModule
local env_action = {}

-- Bumped to 2 by #316: an action carries an independent aim direction, so a v1
-- action and a v2 action are not the same contract even when their fields agree.
env_action.VERSION = 2

---@type InputHeldAction[]
env_action.HELD_ACTIONS = {
    "shoot",
    "pass",
    "sprint",
    "jockey",
    "lob",
    "aerial_strike",
    "aerial_acrobatic",
    "equipment",
}

---@type InputEdgeAction[]
env_action.EDGE_ACTIONS = {
    "shoot",
    "pass",
    "switch",
    "dash",
    "dodge",
    "equipment_pressed",
    "equipment_released",
}

local ACTION_FIELDS = {
    version = true,
    move = true,
    aim = true,
    held = true,
    edges = true,
}

local MOVE_FIELDS = { x = true, y = true }
local AIM_FIELDS = { x = true, y = true }

-- Fixed-slot routing gives every slot exactly one player for the whole fixture,
-- so the legacy "switch to the outfielder nearest the ball" intent has no
-- meaning here. sim.match ignores it; the mask says so and the contract rejects
-- it with a reason instead of silently dropping it.
local UNAVAILABLE_EDGES = { switch = true }

local MOVE_EPSILON = 1e-9

---@param code EnvActionErrorCode
---@param message string
---@return nil, string, EnvActionErrorCode
local function reject(code, message)
    return nil, message, code
end

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value < math.huge and value > -math.huge
end

---@param value any
---@param allowed table<string, boolean>
---@return boolean
local function only_known_fields(value, allowed)
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

---@return EnvSlotAction
function env_action.neutral()
    return { version = env_action.VERSION, move = { x = 0, y = 0 }, held = {}, edges = {} }
end

-- Validate one slot action and return a normalized copy. Actions come from a
-- policy (external input), so an illegal action is a recoverable rejection with
-- a machine-readable reason, never an assert.
---@param action any
---@return EnvSlotAction?, string?, EnvActionErrorCode?
function env_action.validate(action)
    if not only_known_fields(action, ACTION_FIELDS) then
        return reject("malformed", "action must be a table of known fields")
    end
    if action.version ~= nil and action.version ~= env_action.VERSION then
        return reject("malformed", "unsupported env action version")
    end
    local move = { x = 0, y = 0 }
    if action.move ~= nil then
        if not only_known_fields(action.move, MOVE_FIELDS) then
            return reject("malformed", "action.move must be an { x, y } table")
        end
        local x = action.move.x or 0
        local y = action.move.y or 0
        if not is_finite(x) or not is_finite(y) then
            return reject("malformed", "action.move components must be finite numbers")
        end
        if x * x + y * y > 1 + MOVE_EPSILON then
            return reject("move_out_of_range", "action.move must lie inside the unit disc")
        end
        move = { x = x, y = y }
    end
    -- Aim is a direction, so unlike `move` it is not confined to the unit disc:
    -- only its angle survives quantization. A nil or zero-length aim is the
    -- absence of one, which `to_sample` encodes as AIM_NONE.
    local aim = nil
    if action.aim ~= nil then
        if not only_known_fields(action.aim, AIM_FIELDS) then
            return reject("malformed", "action.aim must be an { x, y } table")
        end
        local x = action.aim.x or 0
        local y = action.aim.y or 0
        if not is_finite(x) or not is_finite(y) then
            return reject("malformed", "action.aim components must be finite numbers")
        end
        if x ~= 0 or y ~= 0 then
            aim = { x = x, y = y }
        end
    end
    local held = {}
    if action.held ~= nil then
        if type(action.held) ~= "table" then
            return reject("malformed", "action.held must be a table")
        end
        for key, value in pairs(action.held) do
            if type(key) ~= "string" or input_frame.HELD_BITS[key] == nil then
                return reject("unknown_held_action", "unknown held action: " .. tostring(key))
            end
            if type(value) ~= "boolean" then
                return reject("malformed", "held action " .. key .. " must be a boolean")
            end
            held[key] = value
        end
    end
    local edges = {}
    if action.edges ~= nil then
        if type(action.edges) ~= "table" then
            return reject("malformed", "action.edges must be a table")
        end
        for key, value in pairs(action.edges) do
            if type(key) ~= "string" or input_frame.EDGE_BITS[key] == nil then
                return reject("unknown_edge_action", "unknown edge action: " .. tostring(key))
            end
            if type(value) ~= "boolean" then
                return reject("malformed", "edge action " .. key .. " must be a boolean")
            end
            edges[key] = value
        end
    end
    return { version = env_action.VERSION, move = move, aim = aim, held = held, edges = edges }
end

-- Drop the one-shot edges, keeping held intents. Used when one action is held
-- across several fixed ticks: an edge means "this tick only", so repeating it
-- would fabricate presses the policy never made.
---@param action EnvSlotAction
---@return EnvSlotAction
function env_action.without_edges(action)
    local held = {}
    for key, value in pairs(action.held or {}) do
        held[key] = value
    end
    local move = action.move or { x = 0, y = 0 }
    local aim = action.aim
    return {
        version = env_action.VERSION,
        move = { x = move.x, y = move.y },
        aim = aim and { x = aim.x, y = aim.y } or nil,
        held = held,
        edges = {},
    }
end

---@param action EnvSlotAction
---@return InputSample?, string?, EnvActionErrorCode?
function env_action.to_sample(action)
    local normalized, err, code = env_action.validate(action)
    if not normalized then
        return nil, err, code
    end
    local move = assert(normalized.move)
    local move_x, move_y, quantize_err = input_frame.quantize_move(move.x, move.y)
    if not move_x or not move_y then
        return reject("move_out_of_range", quantize_err or "action.move is not quantizable")
    end
    local held = 0
    local edges = 0
    for _, name in ipairs(env_action.HELD_ACTIONS) do
        if assert(normalized.held)[name] then
            held = held + input_frame.HELD_BITS[name]
        end
    end
    for _, name in ipairs(env_action.EDGE_ACTIONS) do
        if assert(normalized.edges)[name] then
            edges = edges + input_frame.EDGE_BITS[name]
        end
    end
    local aim = input_frame.AIM_NONE
    if normalized.aim ~= nil then
        local quantized, aim_err = input_frame.quantize_aim(normalized.aim.x, normalized.aim.y)
        if not quantized then
            return reject("malformed", aim_err or "action.aim is not quantizable")
        end
        aim = quantized
    end
    local sample, sample_err = input_frame.new_sample({
        move_x = move_x,
        move_y = move_y,
        held = held,
        edges = edges,
        aim = aim,
    })
    if not sample then
        return reject("malformed", sample_err or "action is not encodable as an InputSample")
    end
    return sample
end

---@param sample InputSample
---@return EnvSlotAction?, string?, EnvActionErrorCode?
function env_action.from_sample(sample)
    local move_x, move_y, move_err = input_frame.dequantize_move(sample)
    if not move_x or not move_y then
        return reject("malformed", move_err or "sample move is not decodable")
    end
    local held = {}
    for _, name in ipairs(env_action.HELD_ACTIONS) do
        held[name] = input_frame.is_held(sample, name) == true
    end
    local edges = {}
    for _, name in ipairs(env_action.EDGE_ACTIONS) do
        edges[name] = input_frame.has_edge(sample, name) == true
    end
    local aim = nil
    if sample.aim ~= input_frame.AIM_NONE then
        local aim_x, aim_y, aim_err = input_frame.dequantize_aim(sample.aim)
        if not aim_x or not aim_y then
            return reject("malformed", aim_err or "sample aim is not decodable")
        end
        aim = { x = aim_x, y = aim_y }
    end
    return {
        version = env_action.VERSION,
        move = { x = move_x, y = move_y },
        aim = aim,
        held = held,
        edges = edges,
    }
end

-- Legality a normal client can know, computed only from the fields present in
-- `view`. The representative and team profiles carry no private timers, so the
-- resulting mask cannot encode privileged legality; the privileged profile is
-- flagged so a report can never present it as a human-equivalent mask.
--
-- Only `own` and `ball` are read, so a narrow EnvActionView is enough: callers on
-- the per-step path should pass one instead of building a whole observation.
---@param view EnvSlotView|EnvActionView
---@return EnvActionMask?, string?, EnvActionErrorCode?
function env_action.mask(view)
    if type(view) ~= "table" or type(view.own) ~= "table" or type(view.ball) ~= "table" then
        return reject("malformed", "an action mask needs a slot view")
    end
    local own = view.own
    local equipment = own.equipment
    local equipped = equipment ~= nil
    local equipment_ready = equipment ~= nil and equipment.ready == true
    local aerial_available = own.header_ready and view.ball.airborne and not own.stunned
    return {
        version = env_action.VERSION,
        profile = view.profile,
        privileged = view.profile == "privileged",
        slot = view.slot,
        move = true,
        aim = true,
        held = {
            shoot = true,
            pass = true,
            sprint = true,
            jockey = true,
            lob = true,
            aerial_strike = aerial_available,
            aerial_acrobatic = aerial_available,
            equipment = equipped,
        },
        edges = {
            shoot = true,
            pass = true,
            switch = false,
            dash = own.tackle_ready and not own.stunned,
            dodge = own.dodge_ready and not own.stunned,
            equipment_pressed = equipment_ready,
            equipment_released = equipped,
        },
    }
end

-- Check a validated action against a mask and name the first violation. The
-- environment reports this reason back to the caller so a policy learns why an
-- intent was refused instead of silently losing it.
---@param action EnvSlotAction
---@param mask EnvActionMask
---@return boolean?, string?, EnvActionErrorCode?
function env_action.check_mask(action, mask)
    assert(type(mask) == "table", "an action mask is required")
    local normalized, err, code = env_action.validate(action)
    if not normalized then
        return nil, err, code
    end
    for _, name in ipairs(env_action.HELD_ACTIONS) do
        if assert(normalized.held)[name] and not mask.held[name] then
            return reject(
                "unavailable_action",
                "held action " .. name .. " is not available to slot " .. mask.slot .. " this tick"
            )
        end
    end
    for _, name in ipairs(env_action.EDGE_ACTIONS) do
        if assert(normalized.edges)[name] and not mask.edges[name] then
            local reason = UNAVAILABLE_EDGES[name]
                    and ("edge action " .. name .. " does not exist under fixed-slot routing; slot " .. mask.slot .. " controls one player for the whole fixture")
                or (
                    "edge action "
                    .. name
                    .. " is not available to slot "
                    .. mask.slot
                    .. " this tick"
                )
            return reject("unavailable_action", reason)
        end
    end
    return true
end

return env_action
