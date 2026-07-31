local t = require("spec.support.runner")
local quat = require("core.quat")
local mat4 = require("core.mat4")

local function max_diff(a, b)
    local worst = 0
    for i = 1, 16 do
        worst = math.max(worst, math.abs(a[i] - b[i]))
    end
    return worst
end

local function yaw_of(m)
    return math.deg(math.atan2(m[3], m[11]))
end

t.describe("quat", function()
    t.it("fromEuler agrees with mat4.euler on the same convention", function()
        -- The two must stay interchangeable: clips are authored in Euler and
        -- baked to quaternions, and any divergence in the Y*X*Z order would
        -- silently rotate every bone in the game.
        for _, e in ipairs({
            { -125, 0, 16 },
            { 20, -5, -2 },
            { 8, -24, 0 },
            { 46, 30, -12 },
            { -16, 12, 40 },
        }) do
            local rx, ry, rz = math.rad(e[1]), math.rad(e[2]), math.rad(e[3])
            local from_euler = mat4.transform(0, 0, 0, rx, ry, rz)
            local from_quat = quat.toMat4(quat.fromEuler(rx, ry, rz), 0, 0, 0)
            t.is_true(
                max_diff(from_euler, from_quat) < 1e-9,
                string.format(
                    "euler %d,%d,%d diverged by %g",
                    e[1],
                    e[2],
                    e[3],
                    max_diff(from_euler, from_quat)
                )
            )
        end
    end)

    t.it("slerps across the +/-180 seam the short way", function()
        -- Component-wise Euler interpolation takes the 340 degree route here.
        local a = quat.fromEuler(0, math.rad(-170), 0)
        local b = quat.fromEuler(0, math.rad(170), 0)
        local mid = yaw_of(quat.toMat4(quat.slerp(a, b, 0.5), 0, 0, 0))
        t.near(math.abs(mid), 180, 0.001)
    end)

    t.it("slerp endpoints are exact", function()
        local a = quat.fromEuler(math.rad(-40), 0, math.rad(12))
        local b = quat.fromEuler(math.rad(70), math.rad(30), 0)
        t.is_true(
            max_diff(quat.toMat4(quat.slerp(a, b, 0), 0, 0, 0), quat.toMat4(a, 0, 0, 0)) < 1e-9,
            "t=0 should be a"
        )
        t.is_true(
            max_diff(quat.toMat4(quat.slerp(a, b, 1), 0, 0, 0), quat.toMat4(b, 0, 0, 0)) < 1e-9,
            "t=1 should be b"
        )
    end)

    t.it("slerp stays on the unit sphere", function()
        local a = quat.fromEuler(math.rad(-125), 0, math.rad(16))
        local b = quat.fromEuler(math.rad(46), math.rad(30), 0)
        for i = 0, 10 do
            local q = quat.slerp(a, b, i / 10)
            local len = math.sqrt(q[1] ^ 2 + q[2] ^ 2 + q[3] ^ 2 + q[4] ^ 2)
            t.near(len, 1, 1e-9)
        end
    end)

    t.it("multiply composes: applying b then a", function()
        local a = quat.fromEuler(math.rad(30), 0, 0)
        local b = quat.fromEuler(0, math.rad(45), 0)
        local composed = quat.toMat4(quat.multiply(a, b), 0, 0, 0)
        local expected = mat4.multiply(quat.toMat4(a, 0, 0, 0), quat.toMat4(b, 0, 0, 0))
        t.is_true(max_diff(composed, expected) < 1e-9, "quat multiply must match matrix multiply")
    end)

    t.it("normalize survives a degenerate quaternion", function()
        local q = quat.normalize({ 0, 0, 0, 0 })
        t.near(q[4], 1, 1e-12)
    end)

    t.it("toMat4 carries the translation", function()
        local m = quat.toMat4(quat.identity(), 1.5, -2.5, 3.5)
        t.near(m[4], 1.5, 1e-12)
        t.near(m[8], -2.5, 1e-12)
        t.near(m[12], 3.5, 1e-12)
    end)
end)
