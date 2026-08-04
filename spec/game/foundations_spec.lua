local actions = require("game.input.actions")
local controller = require("game.input.controller")
local focus = require("game.ui.focus")
local settings = require("game.settings")
local t = require("spec.support.runner")
local viewport = require("game.ui.viewport")

t.describe("input actions", function()
    -- The literal keys and buttons below are a deliberate exemption, not a
    -- leftover: everywhere else in `spec/` reads the role off
    -- `game.input.bindings` (#315). Do not derive these. The rule and the
    -- reasoning are recorded once, in `spec/game/input_bindings_spec.lua` above
    -- "keeps the modifier off the movement hand and off PLAY's finger".
    t.it("maps keyboard and gamepad to the same abstract actions", function()
        t.eq(actions.from_key("return").action, "confirm")
        t.eq(actions.from_gamepad("a").action, "confirm")
        t.eq(actions.from_key("escape").action, "back")
        t.eq(actions.from_gamepad("b").action, "back")
        t.eq(actions.from_key("u").action, "equipment")
        t.eq(actions.from_gamepad("rightshoulder").action, "equipment")
        t.is_true(actions.from_key("unknown") == nil)
    end)

    -- Equipment used to ride gamepad B and shadow Back inside a match. It has
    -- its own button now, so B means Back in every context.
    t.it("keeps Back unconditional on gamepad B", function()
        local transform = viewport.new(960, 540)
        t.eq(actions.from_gamepad("b").action, "back")
        t.eq(
            controller.normalize({ kind = "gamepad", button = "b", pressed = false }, transform),
            nil
        )
    end)

    t.it("forwards only equipment releases through normalized app input", function()
        local transform = viewport.new(960, 540)
        local equipment = assert(
            controller.normalize(
                { kind = "gamepad", button = "rightshoulder", pressed = false },
                transform
            )
        )
        t.eq(equipment.action, "equipment")
        t.eq(equipment.pressed, false)
    end)

    t.it("converts pointer input through a letterboxed viewport", function()
        local transform = viewport.new(1280, 720)
        local event = assert(
            controller.normalize({ kind = "click", x = 640, y = 360, button = 1 }, transform)
        )
        t.near(event.x, 480)
        t.near(event.y, 270)
    end)
end)

t.describe("viewport transform", function()
    t.it("round trips virtual coordinates", function()
        local transform = viewport.new(1000, 1000)
        local x, y = viewport.to_actual(transform, 200, 100)
        local vx, vy = viewport.to_virtual(transform, x, y)
        assert(vx and vy)
        t.near(vx, 200)
        t.near(vy, 100)
        local outside = viewport.to_virtual(transform, 10, 10)
        t.is_true(outside == nil)
    end)
end)

t.describe("menu focus", function()
    local layout = {
        { id = "a", kind = "button", rect = { x = 0, y = 0, w = 10, h = 10 } },
        { id = "label", kind = "label", rect = { x = 0, y = 20, w = 10, h = 10 } },
        { id = "b", kind = "button", rect = { x = 0, y = 40, w = 10, h = 10 } },
    }

    t.it("wraps focus and activates the focused widget", function()
        t.eq(focus.ensure(layout, nil), "a")
        t.eq(focus.move(layout, "a", -1), "b")
        t.eq(focus.activated(layout, "b", actions.event("confirm")), "b")
    end)
end)

t.describe("settings", function()
    t.it("clamps invalid numeric input and defaults invalid types", function()
        local value = settings.validate({
            master_volume = 3,
            sfx_volume = -1,
            muted = "no",
            fullscreen = true,
        })
        t.eq(value.master_volume, 1)
        t.eq(value.sfx_volume, 0)
        t.eq(value.muted, false)
        t.eq(value.fullscreen, true)
    end)

    t.it("preserves explicit false accessibility settings", function()
        local value = settings.validate({
            screen_shake = false,
            bloom = false,
        })
        t.eq(value.screen_shake, false)
        t.eq(value.bloom, false)
    end)

    t.it("round trips through deterministic storage", function()
        local contents = nil
        local storage = {
            read = function()
                return contents
            end,
            write = function(value)
                contents = value
                return true
            end,
        }
        local value = settings.defaults()
        value.master_volume = 0.42
        assert(settings.save(value, storage))
        t.eq(settings.load(storage).master_volume, 0.42)
        local saved = assert(contents)
        t.is_true(saved:match("^version=1") ~= nil)
    end)
end)
