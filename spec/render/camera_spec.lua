local t = require("spec.support.runner")
local camera = require("game.render.camera")

local field = { w = 960, h = 540 }
local vp = { w = 1280, h = 720 }

t.describe("camera.project", function()
    t.it("places nearer points lower on screen than far points", function()
        local _, far_y = camera.project(480, 0, field, vp)
        local _, near_y = camera.project(480, 540, field, vp)
        t.is_true(near_y > far_y)
    end)

    t.it("scales nearer points up relative to far points", function()
        local _, _, far_scale = camera.project(480, 0, field, vp)
        local _, _, near_scale = camera.project(480, 540, field, vp)
        t.is_true(near_scale > far_scale)
    end)

    t.it("keeps the pitch centre line on the screen centre at any depth", function()
        local far_x = camera.project(480, 0, field, vp)
        local near_x = camera.project(480, 540, field, vp)
        t.near(far_x, vp.w / 2, 1e-6)
        t.near(near_x, vp.w / 2, 1e-6)
    end)

    t.it("spreads the near edge wider than the far edge (trapezoid)", function()
        local far_right = camera.project(960, 0, field, vp)
        local near_right = camera.project(960, 540, field, vp)
        t.is_true(near_right > far_right)
    end)
end)

t.describe("camera.view", function()
    t.it("zoom 1 leaves the projection untouched", function()
        local v = camera.view(120, 90, field, 1)
        local a, b, c = camera.project(300, 200, field, vp)
        local x, y, z = camera.project(300, 200, field, vp, nil, v)
        t.near(x, a, 0.001)
        t.near(y, b, 0.001)
        t.near(z, c, 0.001)
    end)

    t.it("clamps the focus so a zoomed frame stays over the pitch", function()
        local v = camera.view(0, 0, field, 2)
        t.eq(v.x, 240)
        t.eq(v.y, 135)
    end)

    t.it("puts the focus at the centre of the screen", function()
        local v = camera.view(300, 200, field, 2)
        local x, y = camera.project(v.x, v.y, field, vp, nil, v)
        t.near(x, vp.w / 2, 0.001)
        t.near(y, vp.h / 2, 0.001)
    end)

    t.it("magnifies without adding convergence", function()
        -- The whole point of a lens zoom: the ratio between a far span and a
        -- near span is a property of the perspective, so magnifying must not
        -- change it. The earlier window remap did, which is what made the pitch
        -- look like a funnel.
        local v = camera.view(480, 270, field, 2)
        local function span(wy, view)
            local a = camera.project(300, wy, field, vp, nil, view)
            local b = camera.project(660, wy, field, vp, nil, view)
            return math.abs(b - a)
        end
        t.near(span(60, v) / span(480, v), span(60, nil) / span(480, nil), 0.0001)
    end)
end)

t.describe("camera perspective mode", function()
    local function with_perspective(fn)
        local saved = camera.perspective_mode
        camera.perspective_mode = true
        local ok, err = pcall(fn)
        camera.perspective_mode = saved
        assert(ok, err)
    end

    t.it("centres the focus on screen", function()
        with_perspective(function()
            local v = camera.view(300, 200, field, 1)
            local x, y = camera.project(v.x, v.y, field, vp, nil, v)
            t.near(x, vp.w / 2, 0.5)
            t.near(y, vp.h / 2, 0.5)
        end)
    end)

    t.it("converges: a far span is narrower than an equal near span", function()
        with_perspective(function()
            local v = camera.view(480, 270, field, 1)
            local function span(wy)
                local a = camera.project(300, wy, field, vp, nil, v)
                local b = camera.project(660, wy, field, vp, nil, v)
                return math.abs(b - a)
            end
            -- +y runs toward the viewer, so a low y is the far touchline.
            t.is_true(span(60) < span(480), "far span " .. span(60) .. " vs near " .. span(480))
        end)
    end)

    t.it("scales a player with depth, larger when nearer", function()
        with_perspective(function()
            local v = camera.view(480, 270, field, 1)
            local _, _, far = camera.project(480, 60, field, vp, nil, v)
            local _, _, near = camera.project(480, 480, field, vp, nil, v)
            t.is_true(near > far, "near scale " .. near .. " not above far " .. far)
            t.is_true(far > 0, "far scale should stay positive, got " .. far)
        end)
    end)

    t.it("pushes a point behind the camera out of frame rather than mirroring it", function()
        with_perspective(function()
            -- Clamping a negative w to a small positive would place the point at
            -- a finite but wildly wrong spot on screen; it has to leave frame.
            local v = camera.view(480, 270, field, 1)
            -- +y runs toward the viewer and the eye sits beyond the focus on
            -- that side, so a large +y is behind the camera. (A large -y is
            -- merely very far away, and correctly converges on the vanishing
            -- point rather than leaving frame.)
            local x, y = camera.project(480, 100000, field, vp, nil, v)
            t.is_true(x < -1000 or x > vp.w + 1000, "x should be far off screen: " .. x)
            t.is_true(y < -1000 or y > vp.h + 1000, "y should be far off screen: " .. y)
        end)
    end)
end)
