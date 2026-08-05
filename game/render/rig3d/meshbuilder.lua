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
-- Colour is NOT baked here (#337 slice 1). Every vertex carries a palette SLOT
-- INDEX (see rig3d/themes.lua `SLOT_INDEX`) instead of a literal {r,g,b,a}; the
-- vertex shader resolves that index against a `u_palette` uniform sent once
-- per draw. That is what lets one mesh serve every team/theme colour variant.
--
-- TWO KINDS OF BUILDER (#337 slice 2)
--
-- A PART builder holds one part's triangles in that part's own local space.
-- Its `bone` and `material` columns sit at zero: a part does not know where it
-- rides or how it shades, and it is no longer a mesh or a draw call on its own.
--
-- `meshbuilder.merge` folds a list of part builders into ONE builder whose
-- vertices carry, per vertex:
--   * the bone index that drives them (resolved on the GPU against a bone-matrix
--     uniform array -- see rig3d/renderer.lua), and
--   * the material id that shades them (metal / emissive / plain), so material
--     stops being a per-draw uniform and stops forcing a draw call per material
--     group.
-- Only a merged builder can `:build()`, because only a merged builder is a draw
-- call. That is the whole of slice 2's ~28-draws-per-character -> 1 collapse.

local mat4 = require("core.mat4")

local meshbuilder = {}
local Builder = {}
Builder.__index = Builder

-- Position(3) + TexCoord(2) + Normal(3) + PaletteSlot(1) + BoneIndex(1) +
-- Material(1) = 11 floats.
--
-- TexCoord is never sampled by our shader, but LÖVE's generated vertex-shader
-- boilerplate always declares the VertexTexCoord attribute, so we supply it
-- rather than rely on an undefined attribute default.
--
-- BoneIndex and Material are `float`, not an integer type, for the same reason
-- PaletteSlot is: GLSL ES 1.00 (WebGL 1) has no integer vertex attributes at
-- all -- `vertexAttribIPointer` is WebGL 2 -- so an index crosses the boundary
-- as a float and is rounded back to an int in the vertex stage.
meshbuilder.VERTEX_FORMAT = {
    { "VertexPosition", "float", 3 },
    { "VertexTexCoord", "float", 2 },
    { "VertexNormal", "float", 3 },
    { "VertexPaletteSlot", "float", 1 },
    { "VertexBoneIndex", "float", 1 },
    { "VertexMaterial", "float", 1 },
}

-- Shading family, baked per vertex instead of sent per draw. Kept as small
-- exact integers so the vertex->fragment varying compares safely against
-- half-way thresholds.
---@type table<string, integer>
meshbuilder.MATERIAL = {
    plain = 0,
    metal = 1,
    emissive = 2,
}

-- `nil` is the common case (skin, cloth, straps), so it maps to `plain` rather
-- than being an error.
---@param name string|nil
---@return integer
function meshbuilder.materialIndex(name)
    if name == nil then
        return meshbuilder.MATERIAL.plain
    end
    local index = meshbuilder.MATERIAL[name]
    assert(index, "unknown rig3d material: " .. tostring(name))
    return index
end

---@return table
function meshbuilder.new()
    return setmetatable({ verts = {}, merged = false }, Builder)
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

    -- Bone and material are the trailing zeros: a part builder does not know
    -- them, `meshbuilder.merge` stamps them in.
    local verts = self.verts
    verts[#verts + 1] = { ax, ay, az, 0, 0, nx, ny, nz, slot, 0, 0 }
    verts[#verts + 1] = { bx, by, bz, 0, 0, nx, ny, nz, slot, 0, 0 }
    verts[#verts + 1] = { cx, cy, cz, 0, 0, nx, ny, nz, slot, 0, 0 }
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

-- Folds many part builders into the single builder that becomes one draw call.
--
-- Each part contributes its vertices with:
--   * `attach` baked in. It is a STATIC per-part offset from its bone (a shield
--     slid down its socket, a holster rotated onto the hip), so once it lives in
--     the vertex the bone matrix alone drives the vertex -- which is precisely
--     what removes the per-part model matrix and therefore the per-part draw.
--   * `bone` written to every vertex, as the 0-based index into the bone-matrix
--     uniform array (skeleton.boneIndex).
--   * `material` written to every vertex, so armour and energy blades shade
--     differently inside ONE draw instead of splitting it into three.
--
-- Part order is preserved verbatim, so the triangles rasterise in exactly the
-- order the separate draw calls used to submit them.
---@param parts table[]  -- { { builder = table, bone = integer, attach = number[]|nil, material = string|nil }, ... }
---@return table  -- a merged Builder
function meshbuilder.merge(parts)
    local out = meshbuilder.new()
    out.merged = true
    local verts = out.verts
    for _, part in ipairs(parts) do
        assert(
            type(part.bone) == "number" and part.bone >= 0,
            "merge needs a 0-based bone index per part, got " .. tostring(part.bone)
        )
        local bone = part.bone
        local material = meshbuilder.materialIndex(part.material)
        local tf = part.attach
        for _, v in ipairs(part.builder.verts) do
            local x, y, z = v[1], v[2], v[3]
            local nx, ny, nz = v[6], v[7], v[8]
            if tf then
                x, y, z = mat4.transformPoint(tf, x, y, z)
                -- Every attach is rotation + translation today, so this keeps
                -- unit length already; renormalising costs one build-time
                -- square root per vertex and keeps the shader's `normalize`-free
                -- assumptions true if an attach ever gains a scale.
                nx, ny, nz = normalize(mat4.transformDirection(tf, nx, ny, nz))
            end
            verts[#verts + 1] = { x, y, z, 0, 0, nx, ny, nz, v[9], bone, material }
        end
    end
    return out
end

-- Bakes the accumulated triangles into an immutable GPU mesh.
---@return love.Mesh
function Builder:build()
    assert(
        self.merged,
        "only a merged builder is a mesh (#337 slice 2): a part is not a draw call, "
            .. "pass it through meshbuilder.merge"
    )
    assert(#self.verts > 0, "cannot build an empty mesh")
    return love.graphics.newMesh(meshbuilder.VERTEX_FORMAT, self.verts, "triangles", "static")
end

return meshbuilder
