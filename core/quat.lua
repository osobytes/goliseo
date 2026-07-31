-- Quaternions, for interpolating and blending rotations.
--
-- WHY THIS EXISTS
--
-- Clips are authored as Euler angles because that is what a human can read and
-- edit. Euler angles are a fine *authoring* format and a bad *interpolation*
-- format, and the difference only shows up once you blend:
--
--   * Component-wise Euler lerp does not trace the shortest rotation. Blending
--     yaw -170 to +170 the naive way sweeps 340 degrees the wrong way round
--     instead of stepping 20 degrees across the seam.
--   * Two axes changing at once trace a path that is not a rotation about any
--     fixed axis, so a blend can dip through poses neither keyframe contains --
--     an arm can pass through the torso between two poses that both clear it.
--   * Near gimbal lock the mapping from angles to orientation is degenerate and
--     small angle changes produce large orientation changes.
--
-- Playing a single clip mostly hides all three. Layering a strike over a run,
-- or crossfading between clips (which #98 requires), does not.
--
-- THE FIX, AND WHY IT IS CHEAP
--
-- Author in Euler, bake to quaternions once at load, interpolate and blend as
-- quaternions, convert to a matrix at the end. Nothing in clips.lua has to
-- change for a human. This is the same split every production pipeline uses:
-- Blender authors Euler curves, glTF stores quaternions.
--
-- Storage is {x, y, z, w}.

local quat = {}

---@return number[]
function quat.identity()
    return { 0, 0, 0, 1 }
end

---@param angle number  -- radians
---@return number[]
local function axisAngle(ax, ay, az, angle)
    local h = angle * 0.5
    local s = math.sin(h)
    return { ax * s, ay * s, az * s, math.cos(h) }
end

-- Returns a * b: the rotation that applies b first, then a.
---@param a number[]
---@param b number[]
---@return number[]
function quat.multiply(a, b)
    local ax, ay, az, aw = a[1], a[2], a[3], a[4]
    local bx, by, bz, bw = b[1], b[2], b[3], b[4]
    return {
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    }
end

-- Euler in the slice's convention: Y * X * Z, so a limb is rolled (Z), then
-- pitched (X), then yawed (Y). Matches mat4.euler exactly, which is what lets
-- the switch to quaternions be visually invisible.
---@param rx number  -- radians
---@param ry number  -- radians
---@param rz number  -- radians
---@return number[]
function quat.fromEuler(rx, ry, rz)
    return quat.multiply(
        quat.multiply(axisAngle(0, 1, 0, ry), axisAngle(1, 0, 0, rx)),
        axisAngle(0, 0, 1, rz)
    )
end

---@param q number[]
---@return number[]
function quat.normalize(q)
    local len = math.sqrt(q[1] * q[1] + q[2] * q[2] + q[3] * q[3] + q[4] * q[4])
    if len < 1e-12 then
        return quat.identity()
    end
    return { q[1] / len, q[2] / len, q[3] / len, q[4] / len }
end

-- Spherical linear interpolation, always along the SHORT arc.
--
-- The sign flip is the part people forget: q and -q are the same orientation,
-- so without it a blend can take the 340-degree route. Below a very small angle
-- slerp's division becomes unstable, so it falls back to a normalized lerp --
-- indistinguishable at that scale.
---@param a number[]
---@param b number[]
---@param t number
---@return number[]
function quat.slerp(a, b, t)
    local bx, by, bz, bw = b[1], b[2], b[3], b[4]
    local dot = a[1] * bx + a[2] * by + a[3] * bz + a[4] * bw

    if dot < 0 then
        bx, by, bz, bw = -bx, -by, -bz, -bw
        dot = -dot
    end

    if dot > 0.9995 then
        return quat.normalize({
            a[1] + (bx - a[1]) * t,
            a[2] + (by - a[2]) * t,
            a[3] + (bz - a[3]) * t,
            a[4] + (bw - a[4]) * t,
        })
    end

    local theta = math.acos(dot)
    local sin_theta = math.sin(theta)
    local s0 = math.sin((1 - t) * theta) / sin_theta
    local s1 = math.sin(t * theta) / sin_theta
    return {
        a[1] * s0 + bx * s1,
        a[2] * s0 + by * s1,
        a[3] * s0 + bz * s1,
        a[4] * s0 + bw * s1,
    }
end

-- Builds a row-major 4x4 with this rotation and the given translation, matching
-- lib/mat4.lua's storage so the two are interchangeable downstream.
---@param q number[]
---@return number[]
function quat.toMat4(q, tx, ty, tz)
    local x, y, z, w = q[1], q[2], q[3], q[4]
    local x2, y2, z2 = x + x, y + y, z + z
    local xx, xy, xz = x * x2, x * y2, x * z2
    local yy, yz, zz = y * y2, y * z2, z * z2
    local wx, wy, wz = w * x2, w * y2, w * z2

    -- stylua: ignore
    return {
        1 - (yy + zz), xy - wz,       xz + wy,       tx,
        xy + wz,       1 - (xx + zz), yz - wx,       ty,
        xz - wy,       yz + wx,       1 - (xx + yy), tz,
        0,             0,             0,             1,
    }
end

return quat
