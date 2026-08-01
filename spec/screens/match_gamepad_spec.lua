local Match = require("game.screens.match")
local bindings = require("game.input.bindings")
local t = require("spec.support.runner")

-- Press the role, not the button: this follows a rebind instead of pinning one.
local ACTION_BUTTON = bindings.control("action").buttons[1]

-- Drive the match screen off a fake pad: no keyboard at all, `held` names the
-- gamepad buttons that are down, and `pull` names the trigger axes.
---@param held table<string, boolean>
---@param pull table<string, number>
---@param fn fun(match: MatchScreen)
local function with_pad(held, pull, fn)
    local saved_keyboard = love.keyboard
    local saved_joystick = love.joystick
    love.keyboard = {
        isDown = function()
            return false
        end,
    }
    local joystick = {
        isGamepadDown = function(_, button)
            return held[button] == true
        end,
        getGamepadAxis = function(_, axis)
            return pull[axis] or 0
        end,
    }
    love.joystick = {
        getJoysticks = function()
            return { joystick }
        end,
    }
    local ok, err = pcall(function()
        fn(Match.new())
    end)
    love.keyboard = saved_keyboard
    love.joystick = saved_joystick
    assert(ok, tostring(err))
end

t.describe("match screen gamepad input", function()
    t.it("polls the left stick and contextual action button", function()
        local saved_keyboard = love.keyboard
        local saved_joystick = love.joystick
        love.keyboard = {
            isDown = function()
                return false
            end,
        }
        local joystick = {
            getGamepadAxis = function(_, axis)
                return axis == "leftx" and 1 or 0
            end,
            isGamepadDown = function(_, button)
                return button == ACTION_BUTTON
            end,
        }
        love.joystick = {
            getJoysticks = function()
                return { joystick }
            end,
        }

        local ok, err = pcall(function()
            local match = Match.new()
            local player = match.state.players[match.state.controlled]
            match:update(1 / 60)
            t.is_true(player.run_vel.x > 0, "left stick drives the controlled player")
            t.is_true(player.charge > 0, "ACTION holds the contextual shot charge")
        end)
        love.keyboard = saved_keyboard
        love.joystick = saved_joystick
        assert(ok, tostring(err))
    end)

    t.it("accepts abstract gamepad actions for off-ball switching", function()
        local match = Match.new()
        match.state.owner = nil
        match:event({ kind = "action", action = "pass_switch", pressed = true })
        t.eq(match._switch, true)
    end)

    -- The whole point of moving the modifier onto a trigger. This chord used to
    -- be gamepad Y held with A, which one thumb cannot do, so the bicycle kick
    -- was documented, simulated, and unreachable on a controller. Assert the
    -- request survives the full render-frame path into the sampled input rather
    -- than trusting that the two halves compose.
    t.it("requests an acrobatic strike from MODIFIER + ACTION off the ball", function()
        local action_button = bindings.control("action").buttons[1]
        local modifier_axis = bindings.control("modifier").axes[1]

        -- ACTION alone off the ball is an ordinary first-time strike...
        with_pad({ [action_button] = true }, {}, function(match)
            match.state.owner = nil
            match:update(1 / 60)
            local held = match._input_adapter.held
            t.eq(held.aerial_strike, true, "ACTION off the ball should request a strike")
            t.eq(held.aerial_acrobatic, false, "without the modifier it is not acrobatic")
        end)

        -- ...and the same press with the trigger pulled is the acrobatic one.
        with_pad(
            { [action_button] = true },
            { [modifier_axis] = bindings.TRIGGER_THRESHOLD },
            function(match)
                match.state.owner = nil
                match:update(1 / 60)
                local held = match._input_adapter.held
                t.eq(held.aerial_acrobatic, true, "MODIFIER + ACTION should request a bicycle")
                t.eq(held.jockey, true, "and it is still an off-ball hold")
            end
        )

        -- A trigger short of the threshold must not arm it, or the chord would
        -- fire on an idle stick's resting noise.
        with_pad(
            { [action_button] = true },
            { [modifier_axis] = bindings.TRIGGER_THRESHOLD - 0.01 },
            function(match)
                match.state.owner = nil
                match:update(1 / 60)
                t.eq(
                    match._input_adapter.held.aerial_acrobatic,
                    false,
                    "a trigger below the threshold must not request a bicycle"
                )
            end
        )
    end)
end)
