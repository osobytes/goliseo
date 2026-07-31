local t = require("spec.support.runner")
local camera = require("game.render.camera")
local camera_follow = require("game.render.camera_follow")

local field = { w = 960, h = 540 }

---@return MatchState
local function fake_state(ball_x, ball_y)
    return {
        field = field,
        ball = { x = ball_x, y = ball_y },
        players = {},
        controlled = nil,
    }
end

t.describe("camera_follow", function()
    t.it("moves the focus toward the ball", function()
        -- Regression: deriving the clamp margin from field/(2*zoom) pinned the
        -- focus to the exact centre of the pitch at zoom 1, so the camera could
        -- not follow anything at all.
        camera_follow.reset()
        local s = fake_state(200, 150)
        camera_follow.update(s, 1 / 60)
        local fx, fy = camera_follow.focus()
        fx, fy = assert(fx), assert(fy)
        t.near(fx, 200, 1)
        t.near(fy, 150, 1)
    end)

    t.it("holds still while the ball drifts inside the deadzone", function()
        camera_follow.reset()
        camera_follow.update(fake_state(480, 270), 1 / 60)
        local before = assert((camera_follow.focus()))
        -- A slow drift of ~30 units/s. Jumping the ball instead would derive a
        -- huge velocity, and the lead would carry the target out of the box --
        -- correct behaviour, but not what this test is measuring.
        local x = 480
        for _ = 1, 30 do
            x = x + 0.5
            camera_follow.update(fake_state(x, 270), 1 / 60)
        end
        local after = assert((camera_follow.focus()))
        t.near(after, before, 2)
    end)

    t.it("starts moving once the ball leaves the deadzone", function()
        camera_follow.reset()
        camera_follow.update(fake_state(480, 270), 1 / 60)
        local before = assert((camera_follow.focus()))
        for _ = 1, 30 do
            camera_follow.update(fake_state(800, 270), 1 / 60)
        end
        local after = assert((camera_follow.focus()))
        t.is_true(after > before + 50, "focus should track a ball that has left the box")
    end)

    t.it("eases rather than snapping once established", function()
        camera_follow.reset()
        camera_follow.update(fake_state(480, 270), 1 / 60)
        camera_follow.update(fake_state(900, 500), 1 / 60)
        local fx = assert((camera_follow.focus()))
        t.is_true(fx > 480, "focus should move toward the ball")
        t.is_true(fx < 900, "focus should not snap to the ball")
    end)

    t.it("keeps the focus inside the pitch but still reaches toward goal", function()
        camera_follow.reset()
        for _ = 1, 400 do
            camera_follow.update(fake_state(960, 540), 1 / 30)
        end
        local view = camera_follow.view(field)
        view = assert(view, "view should exist after updates")
        t.is_true(view.x <= field.w, "focus stays on the pitch")
        -- The old clamp pinned this to 480; a usable camera gets much closer to
        -- the goal than the halfway line.
        t.is_true(view.x > 700, "focus should reach well into the attacking third")
    end)

    t.it("leads the ball along its travel", function()
        -- Measured with the deadzone off, because the deadzone deliberately lets
        -- the focus TRAIL the framing point -- the two features pull opposite
        -- ways and testing them together measures neither.
        local dz_x, dz_y = camera_follow.config.deadzone_x, camera_follow.config.deadzone_y
        camera_follow.config.deadzone_x, camera_follow.config.deadzone_y = 0, 0

        local function run_with_lead(lead_time)
            local saved = camera_follow.config.lead_time
            camera_follow.config.lead_time = lead_time
            camera_follow.reset()
            local x = 300
            for _ = 1, 40 do
                camera_follow.update(fake_state(x, 270), 1 / 60)
                x = x + 8
            end
            camera_follow.config.lead_time = saved
            return assert((camera_follow.focus()))
        end

        local led = run_with_lead(0.45)
        local unled = run_with_lead(0)
        camera_follow.config.deadzone_x, camera_follow.config.deadzone_y = dz_x, dz_y
        t.is_true(led > unled, "leading should put the focus further along the ball's travel")
    end)

    t.it("view zoom 1 still produces a usable projection", function()
        camera_follow.reset()
        camera_follow.update(fake_state(300, 200), 1 / 60)
        local view = camera_follow.view(field)
        local sx, sy = camera.project(300, 200, field, { w = 1280, h = 720 }, nil, view)
        t.is_true(sx == sx and sy == sy, "projection should be finite")
    end)
end)
