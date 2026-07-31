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

    t.it("looks ahead along travel once the ball is moving", function()
        -- Look-ahead is applied to the eased result, not to the tracking target,
        -- so it survives the deadzone instead of being swallowed by it.
        camera_follow.reset()
        local x = 300
        for _ = 1, 60 do
            camera_follow.update(fake_state(x, 270), 1 / 60)
            x = x + 4 -- 240 units/s, a real running pace
        end
        local fx = assert((camera_follow.focus()))
        t.is_true(fx > x, "focus should sit ahead of a moving ball")
    end)

    t.it("leads a running ball by a useful distance, not a token one", function()
        -- Regression: an exponential ease chasing a target moving at v settles
        -- v/ease behind it, which at a running pace is the same order as the
        -- lead itself. Both mechanisms were present and they cancelled, so the
        -- carrier still could not see who was in front of them. Asserting only
        -- "ahead by something" let that through -- this pins the magnitude.
        camera_follow.reset()
        local x = 200
        for _ = 1, 180 do
            camera_follow.update(fake_state(x, 270), 1 / 60)
            x = x + 4 -- 240 units/s
        end
        local fx = assert((camera_follow.focus()))
        t.is_true(fx - x > 60, "lead was only " .. (fx - x))
    end)

    t.it("does not let ball-keeping clamp away the lead during fast play", function()
        -- KEEP is a backstop for when play outruns the ease. If the lead can ask
        -- for more than KEEP allows, KEEP binds every frame of a fast break and
        -- the look-ahead silently does nothing.
        camera_follow.reset()
        -- Kept clear of the touchlines: view() also applies the margin clamp,
        -- which is a separate concern from KEEP and would mask it.
        local x, y = 150, 120
        for _ = 1, 120 do
            camera_follow.update(fake_state(x, y), 1 / 60)
            x, y = x + 4, y + 2 -- ~268 units/s diagonally, a genuine counter
        end
        local fx, fy = camera_follow.focus()
        local view = assert(camera_follow.view(field))
        -- view() applies KEEP to the raw focus; if KEEP did not bind, the two
        -- agree exactly.
        t.near(view.x, assert(fx), 0.001)
        t.near(view.y, assert(fy), 0.001)
    end)

    t.it("does not look ahead for a slow drift", function()
        camera_follow.reset()
        local x = 480
        for _ = 1, 60 do
            camera_follow.update(fake_state(x, 270), 1 / 60)
            x = x + 0.4 -- 24 units/s, below the look-ahead threshold
        end
        local fx = assert((camera_follow.focus()))
        t.is_true(fx < x + 5, "a drifting ball should not push the camera ahead")
    end)

    t.it("keeps the ball near the focus however far tracking lags", function()
        -- Teleport the ball: tracking and look-ahead both lag badly, and the
        -- hard cap is the only thing keeping the ball on screen.
        camera_follow.reset()
        camera_follow.update(fake_state(120, 120), 1 / 60)
        camera_follow.update(fake_state(840, 420), 1 / 60)
        local view = assert(camera_follow.view(field))
        local cfg = camera_follow.config
        t.is_true(
            math.abs(view.x - 840) <= field.w * cfg.ball_keep_x + 1,
            "ball must stay within the keep radius of the focus"
        )
    end)

    t.it("view zoom 1 still produces a usable projection", function()
        camera_follow.reset()
        camera_follow.update(fake_state(300, 200), 1 / 60)
        local view = camera_follow.view(field)
        local sx, sy = camera.project(300, 200, field, { w = 1280, h = 720 }, nil, view)
        t.is_true(sx == sx and sy == sy, "projection should be finite")
    end)
end)

local view_state = require("game.render.view_state")

t.describe("view_state gait phase", function()
    local function player(x)
        return { id = "p1", pos = { x = x, y = 100 } }
    end

    t.it("advances smoothly when speed changes", function()
        -- Regression: the phase used to be derived as cumulative_distance /
        -- current_stride. Because the stride lengthens with speed, changing it
        -- retroactively rescaled every unit already travelled, so the phase
        -- jumped most of a cycle whenever speed wobbled -- the animation
        -- appeared to flick between two poses.
        local x, prev, worst = 0, nil, 0
        for i = 1, 400 do
            -- Accelerate through the walk/run blend, where the stride changes
            -- fastest and the old formulation was worst.
            local step = 1.5 + i * 0.012
            x = x + step
            view_state.update({ player(x) }, 1 / 60)
            local g = assert(view_state.get("p1")).gait
            if prev then
                local delta = (g - prev) % 1
                worst = math.max(worst, delta)
            end
            prev = g
        end
        -- One frame can never advance more than a small slice of a cycle: at the
        -- fastest stride a 60 Hz frame is well under a tenth of a cycle.
        t.is_true(worst < 0.1, "per-frame gait advance was " .. worst)
    end)

    t.it("stays within [0, 1)", function()
        local x = 0
        for _ = 1, 600 do
            x = x + 6
            view_state.update({ player(x) }, 1 / 60)
        end
        local g = assert(view_state.get("p1")).gait
        t.is_true(g >= 0 and g < 1, "gait out of range: " .. g)
    end)
end)
