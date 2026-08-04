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
    -- Rule 4 of the layout, plus the no-double-binding check. Both are derived
    -- from the table itself rather than from a list of today's control names --
    -- a hand-copied list would read `nil` for a control added later and pass,
    -- which is exactly the regression these are here to stop.
    t.it("declares a sound layout", function()
        t.eq(table.concat(bindings.layout_problems(), "; "), "")
    end)

    -- The guard above is only worth having if it actually fires, so drive it
    -- with each way a future rebind could break the layout.
    t.it("rejects an edge control that only an axis could deliver", function()
        local function with_extra(entry, fn)
            bindings.CONTROLS[#bindings.CONTROLS + 1] = entry
            local ok, result = pcall(fn)
            bindings.CONTROLS[#bindings.CONTROLS] = nil
            assert(ok, result)
            return result
        end

        -- An edge bound to nothing but a trigger: unreachable on both devices.
        local trigger_only = with_extra({
            id = "probe",
            action = "juke",
            keys = {},
            buttons = {},
            axes = { "triggerleft" },
            edge = true,
        }, function()
            return table.concat(bindings.layout_problems(), "; ")
        end)
        t.is_true(
            trigger_only:find("cannot fire one", 1, true) ~= nil,
            "an axis-only edge control passed the layout check: " .. trigger_only
        )

        -- An edge with a keyboard binding but only a trigger on the pad: works
        -- on a keyboard and is silently dead on a controller.
        local pad_dead = with_extra({
            id = "probe",
            action = "juke",
            keys = { "z" },
            buttons = {},
            axes = { "triggerleft" },
            edge = true,
        }, function()
            return table.concat(bindings.layout_problems(), "; ")
        end)
        t.is_true(
            pad_dead:find("only gamepad binding is an axis", 1, true) ~= nil,
            "a pad-undeliverable edge passed the layout check: " .. pad_dead
        )

        -- And a straightforward double binding. The clashing key is read off a
        -- real control rather than written out, so it stays a genuine clash
        -- after a rebind. Pinned to a literal it probes a key nobody holds once
        -- PLAY moves, and this test then fails saying the detector let a double
        -- binding through -- a false alarm aimed at `layout_problems` when the
        -- only stale thing is this line.
        local clash = with_extra({
            id = "probe",
            action = "juke",
            keys = { control("play").keys[1] },
            buttons = {},
            axes = {},
            edge = true,
        }, function()
            return table.concat(bindings.layout_problems(), "; ")
        end)
        t.is_true(
            clash:find("bound to both", 1, true) ~= nil,
            "a double-bound key passed the layout check: " .. clash
        )
    end)

    -- The ergonomic core of the keyboard layout: the modifier is the right index
    -- and PLAY the right middle, the most independent same-hand pair. The left
    -- hand keeps WASD plus its pinky and thumb, and nothing else.
    --
    -- CANONICAL RULE for literal keys in `spec/`: a spec asserts the role read
    -- off the binding table, never the key (#315). There are exactly two
    -- exemptions -- the two literals just below, and
    -- `spec/game/foundations_spec.lua`. Both pin what the bindings actually
    -- ARE, so deriving them would assert that the table equals itself: which
    -- finger a key falls under is a fact about hands, not about the table. A
    -- rebind is MEANT to fail this test and make a human re-check the
    -- ergonomics, which is why nothing else anywhere pins them.
    t.it("keeps the modifier off the movement hand and off PLAY's finger", function()
        t.eq(control("modifier").keys[1], "j")
        t.eq(control("play").keys[1], "k")
        t.is_true(#control("modifier").buttons == 0, "the gamepad modifier must not be a button")
        t.is_true(contains(control("modifier").axes, "triggerright"))
    end)

    -- The movement hand is derived, not copied: it is whatever the movement
    -- roles, sprint and ACTION are bound to today. A hardcoded list would keep
    -- naming keys none of those roles held after a rebind, and this check would
    -- go genuinely vacuous -- juke could land on the NEW action key and still
    -- pass, because a stale list cannot contain it.
    --
    -- Deriving widens the set: it now picks up the arrow-key alternates, which
    -- the old hand-written list never covered. That is the reading rule 3 in
    -- `bindings.lua` asks for. Rule 1 is about the LEFT hand specifically, but
    -- rule 3 is about "the movement hand", and with arrows bound that hand can
    -- be either one -- so juke has to stay off both sets, not just WASD.
    t.it("keeps juke off the movement hand", function()
        local movement = {}
        for _, id in ipairs({
            "move_up",
            "move_down",
            "move_left",
            "move_right",
            "sprint",
            "action",
        }) do
            for _, key in ipairs(control(id).keys) do
                movement[#movement + 1] = key
            end
        end
        for _, key in ipairs(control("juke").keys) do
            t.is_true(
                not contains(movement, key),
                "juke on " .. key .. " sits on the movement hand"
            )
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
