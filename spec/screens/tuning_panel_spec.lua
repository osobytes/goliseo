local t = require("spec.support.runner")
local bindings = require("game.input.bindings")
local panel = require("game.ui.tuning_panel")
local tuning = require("sim.tuning")
local Match = require("game.screens.match")

-- Press the role, not the key: this follows a rebind instead of pinning one.
-- The panel's own navigation keys stay literal below: they are a dev-harness
-- keymap that lives in `game/ui/tuning_panel.lua`, not in the binding table.
local PLAY_KEY = bindings.control("play").keys[1]

t.describe("tuning panel (tier 2)", function()
    t.it("navigates categories and rows, and adjusts the selected knob", function()
        tuning.reset()
        panel.open = true
        panel.cat, panel.row = 1, 1
        local first = tuning.in_category(tuning.categories()[1])[1]
        t.is_true(panel.key("right", false), "keys are consumed while open")
        t.eq(tuning.values[first.key], first.default + first.step)
        panel.key("left", false)
        t.eq(tuning.values[first.key], first.default)
        panel.key("right", true) -- big step
        t.near(tuning.values[first.key], first.default + first.step * 10, 1e-9)
        panel.key("backspace", false) -- reset selected knob
        t.eq(tuning.values[first.key], first.default)
        panel.key("tab", false)
        t.eq(panel.cat, 2, "tab cycles category")
        t.eq(panel.row, 1)
        panel.key("down", false)
        t.eq(panel.row, 2)
        panel.key("backspace", true) -- reset all
        panel.open = false
        tuning.reset()
    end)

    t.it("F1 pauses the match; closing resumes it", function()
        local m = Match.new()
        local t0 = m.state.time_left
        m:event({ kind = "key", key = "f1" })
        t.is_true(panel.open, "panel opened")
        m:update(1 / 60)
        t.eq(m.state.time_left, t0, "sim frozen while the panel is open")
        m:event({ kind = "key", key = "f1" })
        t.is_true(not panel.open, "panel closed")
        m:update(1 / 60)
        t.is_true(m.state.time_left < t0, "sim resumes")
    end)

    t.it("the overlay draws under a stubbed love.graphics", function()
        local saved = love.graphics
        local g, noop = {}, function() end
        for _, name in ipairs({ "setColor", "rectangle", "print", "printf", "line" }) do
            g[name] = noop
        end
        love.graphics = g
        panel.open = true
        local ok, err = pcall(panel.draw, { w = 1280, h = 720 })
        panel.open = false
        love.graphics = saved
        t.is_true(ok, "panel.draw error: " .. tostring(err))
    end)

    t.it("panel keys do not leak into match input", function()
        local m = Match.new()
        m.state.owner = nil
        m:event({ kind = "key", key = "f1" })
        m:event({ kind = "key", key = PLAY_KEY }) -- would be a switch if it leaked
        t.is_true(not m._switch, "PLAY is consumed by the panel, not the match")
        m:event({ kind = "key", key = "f1" })
        tuning.reset()
    end)
end)
