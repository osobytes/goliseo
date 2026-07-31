-- Minimal 4x4 matrix math for the slice's 3D pipeline.
--
-- Storage is a flat 16-element table in ROW-MAJOR order:
--     m[1]  m[2]  m[3]  m[4]      <- row 0
--     m[5]  m[6]  m[7]  m[8]      <- row 1
--     m[9]  m[10] m[11] m[12]     <- row 2
--     m[13] m[14] m[15] m[16]     <- row 3
-- so a translation lives in m[4], m[8], m[12].
--
-- Row-major is not an arbitrary choice: it is what Shader:send expects for a
-- `mat4` uniform in LÖVE 11.5. Verified empirically before this file was
-- written -- a flat row-major table round-trips correctly, while passing the
-- "column" matrixlayout silently yields a garbage matrix.

local mat4 = {}

---@return number[]
function mat4.identity()
    return { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 }
end

-- Returns a * b, i.e. "apply b first, then a" when acting on a column vector.
---@param a number[]
---@param b number[]
---@return number[]
function mat4.multiply(a, b)
    local out = {}
    for row = 0, 3 do
        for col = 1, 4 do
            local sum = 0
            for k = 0, 3 do
                sum = sum + a[row * 4 + k + 1] * b[k * 4 + col]
            end
            out[row * 4 + col] = sum
        end
    end
    return out
end

-- Convenience for the common "parent * child * local" chains.
---@param ... number[]
---@return number[]
function mat4.chain(...)
    local out = select(1, ...)
    for i = 2, select("#", ...) do
        out = mat4.multiply(out, (select(i, ...)))
    end
    return out
end

---@return number[]
function mat4.translation(x, y, z)
    return { 1, 0, 0, x, 0, 1, 0, y, 0, 0, 1, z, 0, 0, 0, 1 }
end

---@param s number  -- uniform scale only: keeps transforms rigid-ish so that
---                    mat3(model) stays a valid normal matrix in the shader.
---@return number[]
function mat4.uniformScale(s)
    return { s, 0, 0, 0, 0, s, 0, 0, 0, 0, s, 0, 0, 0, 0, 1 }
end

---@param a number  -- radians
---@return number[]
function mat4.rotationX(a)
    local c, s = math.cos(a), math.sin(a)
    return { 1, 0, 0, 0, 0, c, -s, 0, 0, s, c, 0, 0, 0, 0, 1 }
end

---@param a number  -- radians
---@return number[]
function mat4.rotationY(a)
    local c, s = math.cos(a), math.sin(a)
    return { c, 0, s, 0, 0, 1, 0, 0, -s, 0, c, 0, 0, 0, 0, 1 }
end

---@param a number  -- radians
---@return number[]
function mat4.rotationZ(a)
    local c, s = math.cos(a), math.sin(a)
    return { c, -s, 0, 0, s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 }
end

-- Euler rotation used by every bone and every baked part transform.
-- Composition order is Y * X * Z, so a vector is rolled (Z), then pitched (X),
-- then yawed (Y). In practice: Z splays a limb outward, X swings it
-- forward/back, Y twists it.
---@param rx number  -- radians
---@param ry number  -- radians
---@param rz number  -- radians
---@return number[]
function mat4.euler(rx, ry, rz)
    return mat4.chain(mat4.rotationY(ry), mat4.rotationX(rx), mat4.rotationZ(rz))
end

-- A bone/part local transform: move to `offset`, then rotate in place.
---@return number[]
function mat4.transform(x, y, z, rx, ry, rz)
    return mat4.multiply(mat4.translation(x, y, z), mat4.euler(rx or 0, ry or 0, rz or 0))
end

---@param fov_deg number
---@param aspect number
---@param near number
---@param far number
---@return number[]
function mat4.perspective(fov_deg, aspect, near, far)
    local f = 1 / math.tan(math.rad(fov_deg) / 2)
    local nf = 1 / (near - far)
    -- stylua: ignore
    return {
        f / aspect, 0, 0,                 0,
        0,          f, 0,                 0,
        0,          0, (far + near) * nf, (2 * far * near) * nf,
        0,          0, -1,                0,
    }
end

local function normalize(x, y, z)
    local len = math.sqrt(x * x + y * y + z * z)
    if len < 1e-9 then
        return 0, 0, 0
    end
    return x / len, y / len, z / len
end

local function cross(ax, ay, az, bx, by, bz)
    return ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx
end

---@param eye number[]     -- {x, y, z}
---@param target number[]  -- {x, y, z}
---@return number[]
function mat4.lookAt(eye, target)
    -- z points *backwards* out of the screen, the usual right-handed view basis.
    local zx, zy, zz = normalize(eye[1] - target[1], eye[2] - target[2], eye[3] - target[3])
    local xx, xy, xz = normalize(cross(0, 1, 0, zx, zy, zz))
    local yx, yy, yz = cross(zx, zy, zz, xx, xy, xz)
    local function dot(ax, ay, az)
        return ax * eye[1] + ay * eye[2] + az * eye[3]
    end
    -- stylua: ignore
    return {
        xx, xy, xz, -dot(xx, xy, xz),
        yx, yy, yz, -dot(yx, yy, yz),
        zx, zy, zz, -dot(zx, zy, zz),
        0,  0,  0,  1,
    }
end

-- Transforms a point (w = 1). Used to place shadow blobs and debug markers.
---@param m number[]
---@return number, number, number
function mat4.transformPoint(m, x, y, z)
    return m[1] * x + m[2] * y + m[3] * z + m[4],
        m[5] * x + m[6] * y + m[7] * z + m[8],
        m[9] * x + m[10] * y + m[11] * z + m[12]
end

-- Transforms a direction (w = 0), ignoring translation.
---@param m number[]
---@return number, number, number
function mat4.transformDirection(m, x, y, z)
    return m[1] * x + m[2] * y + m[3] * z,
        m[5] * x + m[6] * y + m[7] * z,
        m[9] * x + m[10] * y + m[11] * z
end

return mat4
