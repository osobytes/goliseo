-- Accumulates flat-shaded triangles and bakes them into a LÖVE Mesh.
--
-- Every triangle gets a single face normal computed from its own winding, so
-- the whole slice is flat-shaded. That is deliberate: it is one line of code
-- instead of a smoothing-group pass, and it gives the faceted low-poly arcade
-- look the brief asks for.
--
-- Points are transformed by a mat4 as they are added ("baking"), so a part
-- generator can be written in convenient local coordinates and then placed
-- wherever it belongs on the rig.
--
-- Colour is NOT baked here (#337). Every vertex carries a palette SLOT INDEX
-- (see rig3d/themes.lua `SLOT_INDEX`) instead of a literal {r,g,b,a}; the
-- vertex shader resolves that index against a `u_palette` uniform sent once
-- per draw. That is what lets one mesh serve every team/theme colour variant.

local mat4 = require("core.mat4")

local meshbuilder = {}
local Builder = {}
Builder.__index = Builder

-- Position(3) + TexCoord(2) + Normal(3) + PaletteSlot(1).
-- TexCoord is never sampled by our shader, but LÖVE's generated vertex-shader
-- boilerplate always declares the VertexTexCoord attribute, so we supply it
-- rather than rely on an undefined attribute default.
meshbuilder.VERTEX_FORMAT = {
    { "VertexPosition", "float", 3 },
    { "VertexTexCoord", "float", 2 },
    { "VertexNormal", "float", 3 },
    { "VertexPaletteSlot", "float", 1 },
}

---@return table
function meshbuilder.new()
    return setmetatable({ verts = {} }, Builder)
end

local function normalize(x, y, z)
    local len = math.sqrt(x * x + y * y + z * z)
    if len < 1e-12 then
        return 0, 1, 0
    end
    return x / len, y / len, z / len
end

-- Adds one triangle. `tf` may be nil (identity). Vertices are {x, y, z} arrays
-- and are expected counter-clockwise as seen from outside the solid.
---@param tf number[]|nil
---@param a number[]
---@param b number[]
---@param c number[]
---@param slot integer  -- palette slot index, see rig3d/themes.lua SLOT_INDEX
function Builder:triangle(tf, a, b, c, slot)
    assert(
        type(slot) == "number",
        "triangle slot must be a palette slot index (themes.SLOT_INDEX.<name>), got " .. type(slot)
    )
    local ax, ay, az = a[1], a[2], a[3]
    local bx, by, bz = b[1], b[2], b[3]
    local cx, cy, cz = c[1], c[2], c[3]
    if tf then
        ax, ay, az = mat4.transformPoint(tf, ax, ay, az)
        bx, by, bz = mat4.transformPoint(tf, bx, by, bz)
        cx, cy, cz = mat4.transformPoint(tf, cx, cy, cz)
    end

    local ux, uy, uz = bx - ax, by - ay, bz - az
    local vx, vy, vz = cx - ax, cy - ay, cz - az
    local nx = uy * vz - uz * vy
    local ny = uz * vx - ux * vz
    local nz = ux * vy - uy * vx
    -- Degenerate triangles (sword tip, sphere poles, collapsed extrusion rings)
    -- would produce a NaN normal, so drop them instead.
    if nx * nx + ny * ny + nz * nz < 1e-18 then
        return
    end
    nx, ny, nz = normalize(nx, ny, nz)

    local verts = self.verts
    verts[#verts + 1] = { ax, ay, az, 0, 0, nx, ny, nz, slot }
    verts[#verts + 1] = { bx, by, bz, 0, 0, nx, ny, nz, slot }
    verts[#verts + 1] = { cx, cy, cz, 0, 0, nx, ny, nz, slot }
end

-- Adds a planar quad as two triangles, wound a -> b -> c -> d.
function Builder:quad(tf, a, b, c, d, slot)
    self:triangle(tf, a, b, c, slot)
    self:triangle(tf, a, c, d, slot)
end

---@return number
function Builder:triangleCount()
    return #self.verts / 3
end

-- Bakes the accumulated triangles into an immutable GPU mesh.
---@return love.Mesh
function Builder:build()
    assert(#self.verts > 0, "cannot build an empty mesh")
    return love.graphics.newMesh(meshbuilder.VERTEX_FORMAT, self.verts, "triangles", "static")
end

return meshbuilder
