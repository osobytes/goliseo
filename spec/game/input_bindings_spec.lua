local actions = require("game.input.actions")
local bindings = require("game.input.bindings")
local help = require("game.screens.help")
local t = require("spec.support.runner")

---@param id ControlId
---@return ControlBinding
local function control(id)
    return bindings.control(id)
end

---@param list string[]
---@param value string
---@return boolean
local function contains(list, value)
    for _, entry in ipairs(list) do
        if entry == value then
            return true
        end
    end
    return false
end

t.describe("input bindings", function()
    -- Rule 4 of the layout: LÖVE reports triggers as axes, so `love.gamepadpressed`
    -- never fires for one. A control that carries an edge cannot live there, and a
    -- future rebind must not quietly move one.
    t.it("keeps every edge action off the gamepad triggers", function()
        local edge_actions = {
            confirm = true,
            back = true,
            pause = true,
            pass_switch = true,
            juke = true,
            equipment = true,
            toggle_mute = true,
            toggle_fullscreen = true,
            up = true,
            down = true,
            left = true,
            right = true,
        }
        for _, entry in ipairs(bindings.CONTROLS) do
            if #entry.axes > 0 then
                t.is_true(
                    not edge_actions[entry.action],
                    entry.id .. " dispatches an edge action but is bound to a trigger"
                )
            end
        end
    end)

    t.it("binds no physical input to two different roles", function()
        local keys, buttons = {}, {}
        for _, entry in ipairs(bindings.CONTROLS) do
            for _, key in ipairs(entry.keys) do
                t.is_true(keys[key] == nil, "key " .. key .. " is bound twice")
                keys[key] = entry.id
            end
            for _, button in ipairs(entry.buttons) do
                t.is_true(buttons[button] == nil, "button " .. button .. " is bound twice")
                buttons[button] = entry.id
            end
        end
    end)

    -- The ergonomic core of the keyboard layout: the modifier is the right index
    -- and PLAY the right middle, the most independent same-hand pair. The left
    -- hand keeps WASD plus its pinky and thumb, and nothing else.
    t.it("keeps the modifier off the movement hand and off PLAY's finger", function()
        t.eq(control("modifier").keys[1], "j")
        t.eq(control("play").keys[1], "k")
        t.is_true(#control("modifier").buttons == 0, "the gamepad modifier must not be a button")
        t.is_true(contains(control("modifier").axes, "triggerright"))
    end)

    t.it("keeps juke off the movement hand", function()
        for _, key in ipairs(control("juke").keys) do
            local movement = { "w", "a", "s", "d", "lshift", "rshift", "space" }
            t.is_true(not contains(movement, key), "juke on " .. key .. " sits on the left hand")
        end
        t.is_true(not contains(control("juke").buttons, "leftstick"), "juke must leave L3 alone")
    end)

    t.it("derives both action maps from the one table", function()
        t.eq(actions.from_key(control("juke").keys[1]).action, "juke")
        t.eq(actions.from_gamepad(control("juke").buttons[1]).action, "juke")
        t.eq(actions.from_key(control("sprint").keys[1]).action, "sprint")
    end)

    t.it("reads a trigger past its threshold as held", function()
        local pull = 0
        local joystick = {
            isGamepadDown = function()
                return false
            end,
            getGamepadAxis = function()
                return pull
            end,
        }
        pull = bindings.TRIGGER_THRESHOLD - 0.01
        t.eq(bindings.gamepad_down("modifier", joystick), false)
        pull = bindings.TRIGGER_THRESHOLD
        t.eq(bindings.gamepad_down("modifier", joystick), true)
    end)

    t.it("renders the help card from the bindings rather than a literal", function()
        local card = nil
        for _, element in ipairs(help.layout(help.new_state({ w = 960, h = 540 }))) do
            if element.id == "keyboard" then
                card = element.text
            end
        end
        card = assert(card, "the help screen lost its keyboard card")
        t.is_true(card:find("MODIFIER", 1, true) ~= nil)
        t.is_true(
            card:find(bindings.key_label("modifier"), 1, true) ~= nil,
            "the card does not show the bound modifier key"
        )
        t.is_true(
            card:find(bindings.key_label("juke"), 1, true) ~= nil,
            "the card does not show the bound juke key"
        )
    end)

    t.it("labels every reference row for both devices", function()
        local rows = bindings.reference()
        t.is_true(#rows > 0)
        for _, row in ipairs(rows) do
            t.is_true(#row.label > 0)
            t.is_true(#row.keyboard > 0, row.label .. " has no keyboard binding")
        end
        for _, row in ipairs(bindings.reference("match")) do
            t.is_true(#row.gamepad > 0, row.label .. " has no gamepad binding")
        end
    end)
end)
