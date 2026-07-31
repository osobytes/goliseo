local t = require("spec.support.runner")
local mat4 = require("core.mat4")

local function near_vec(ax, ay, az, bx, by, bz, tol)
    tol = tol or 1e-9
    t.near(ax, bx, tol)
    t.near(ay, by, tol)
    t.near(az, bz, tol)
end

t.describe("mat4", function()
    t.it("stores translation row-major, in m[4], m[8], m[12]", function()
        -- Row-major is not cosmetic: it is what LÖVE expects for a mat4 uniform.
        -- Getting it wrong sends a transposed matrix with no error.
        local m = mat4.translation(1, 2, 3)
        t.eq(m[4], 1)
        t.eq(m[8], 2)
        t.eq(m[12], 3)
    end)

    t.it("multiply applies the right-hand matrix first", function()
        local move = mat4.translation(10, 0, 0)
        local spin = mat4.rotationY(math.pi / 2)
        -- move * spin: rotate, then translate along world x.
        local x, y, z = mat4.transformPoint(mat4.multiply(move, spin), 0, 0, 1)
        near_vec(x, y, z, 11, 0, 0, 1e-9)
        -- spin * move: translate, then rotate the whole thing.
        local x2, y2, z2 = mat4.transformPoint(mat4.multiply(spin, move), 0, 0, 1)
        near_vec(x2, y2, z2, 1, 0, -10, 1e-9)
    end)

    t.it("rotations turn the expected way", function()
        local x, y, z = mat4.transformPoint(mat4.rotationY(math.pi / 2), 0, 0, 1)
        near_vec(x, y, z, 1, 0, 0, 1e-9)
        x, y, z = mat4.transformPoint(mat4.rotationX(math.pi / 2), 0, 1, 0)
        near_vec(x, y, z, 0, 0, 1, 1e-9)
        x, y, z = mat4.transformPoint(mat4.rotationZ(math.pi / 2), 1, 0, 0)
        near_vec(x, y, z, 0, 1, 0, 1e-9)
    end)

    t.it("transformDirection ignores translation", function()
        local m = mat4.multiply(mat4.translation(100, 100, 100), mat4.rotationY(math.pi / 2))
        local x, y, z = mat4.transformDirection(m, 0, 0, 1)
        near_vec(x, y, z, 1, 0, 0, 1e-9)
    end)

    t.it("lookAt puts the target in front of the eye", function()
        local view = mat4.lookAt({ 0, 0, 5 }, { 0, 0, 0 })
        -- The view basis has -z forward, so a target ahead lands at negative z.
        local _, _, z = mat4.transformPoint(view, 0, 0, 0)
        t.is_true(z < 0, "target should be down the negative z axis, got " .. z)
        -- The eye maps to the origin of view space.
        local ex, ey, ez = mat4.transformPoint(view, 0, 0, 5)
        near_vec(ex, ey, ez, 0, 0, 0, 1e-9)
    end)

    t.it("perspective maps the near and far planes to the clip range", function()
        local near, far = 1, 100
        local p = mat4.perspective(60, 16 / 9, near, far)
        local function ndc_z(depth)
            -- A point `depth` in front of the camera sits at z = -depth.
            local z = p[9] * 0 + p[11] * -depth + p[12]
            local w = p[13] * 0 + p[15] * -depth + p[16]
            return z / w
        end
        t.near(ndc_z(near), -1, 1e-6)
        t.near(ndc_z(far), 1, 1e-6)
    end)

    t.it("uniformScale keeps direction and scales length", function()
        local x, y, z = mat4.transformDirection(mat4.uniformScale(3), 0, 2, 0)
        near_vec(x, y, z, 0, 6, 0, 1e-9)
    end)

    t.it("chain composes left to right", function()
        local direct = mat4.multiply(mat4.translation(1, 0, 0), mat4.rotationY(math.pi))
        local chained = mat4.chain(mat4.translation(1, 0, 0), mat4.rotationY(math.pi))
        for i = 1, 16 do
            t.near(chained[i], direct[i], 1e-12)
        end
    end)
end)
