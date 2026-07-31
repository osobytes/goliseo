-- Procedural primitive generators. Everything visible in the slice -- sword,
-- shield, helmet, every limb, the arena floor -- is built from these four
-- functions. No external model files are loaded anywhere in this prototype.
--
-- The workhorse is `shapes.extrude`: a closed 2D cross-section swept through a
-- stack of rings, each ring free to move and rescale. A tapered blade, a grip
-- cylinder, a pommel sphere and a tapering limb are all the same operation with
-- different inputs.

local shapes = {}

-- ---------------------------------------------------------------------------
-- Cross-section profiles (closed 2D polygons in the XZ plane, CCW from +Y)
-- ---------------------------------------------------------------------------

-- A circle, for grips, limbs and spheres.
---@param segments number
---@return number[][]
function shapes.circleProfile(segments)
    local profile = {}
    for i = 0, segments - 1 do
        local a = (i / segments) * math.pi * 2
        profile[#profile + 1] = { math.cos(a), math.sin(a) }
    end
    return profile
end

-- A flattened diamond: the classic edged-blade cross-section, with a central
-- ridge running down each face. `thickness` is the half-depth relative to the
-- half-width, so 0.2 is a broad, thin blade.
---@param thickness number
---@return number[][]
function shapes.diamondProfile(thickness)
    return { { 1, 0 }, { 0, thickness }, { -1, 0 }, { 0, -thickness } }
end

-- A rectangle, optionally with the corners cut off, for torsos and limbs that
-- should read as blocky rather than round.
---@param depth number    -- half-depth relative to half-width
---@param bevel number    -- 0 = square corners, 0.4 = strongly chamfered
---@return number[][]
function shapes.boxProfile(depth, bevel)
    local b = bevel or 0
    if b <= 0 then
        return { { 1, -depth }, { 1, depth }, { -1, depth }, { -1, -depth } }
    end
    local d = depth
    -- stylua: ignore
    return {
        {  1, -d + b * d }, {  1,  d - b * d }, {  1 - b,  d }, { -1 + b,  d },
        { -1,  d - b * d }, { -1, -d + b * d }, { -1 + b, -d }, {  1 - b, -d },
    }
end

-- ---------------------------------------------------------------------------
-- Solids
-- ---------------------------------------------------------------------------

-- Sweeps `profile` through `sections`, bottom to top, and caps both ends.
--
-- Each section is { y = <height>, w = <half-width scale>, d = <half-depth
-- scale, defaults to w>, x = <lateral offset>, z = <forward offset> }. A
-- section with w = 0 collapses to a point, which is how the sword tip and the
-- sphere poles are produced -- the resulting zero-area triangles are dropped by
-- the mesh builder.
---@param mb table
---@param tf number[]|nil
---@param profile number[][]
---@param sections table[]
---@param color number[]|fun(section_index: number, point_index: number): number[]
function shapes.extrude(mb, tf, profile, sections, color)
    local n = #profile
    local function colorFor(i, j)
        return type(color) == "function" and color(i, j) or color
    end

    local function point(section, j)
        local p = profile[j]
        local w = section.w
        local d = section.d or w
        return { (section.x or 0) + p[1] * w, section.y, (section.z or 0) + p[2] * d }
    end

    for i = 1, #sections - 1 do
        local lower, upper = sections[i], sections[i + 1]
        for j = 1, n do
            local k = (j % n) + 1
            mb:quad(
                tf,
                point(lower, j),
                point(lower, k),
                point(upper, k),
                point(upper, j),
                colorFor(i, j)
            )
        end
    end

    -- Triangle-fan caps. Skipped automatically when the end ring is collapsed.
    local first, last = sections[1], sections[#sections]
    for j = 2, n - 1 do
        mb:triangle(tf, point(first, 1), point(first, j + 1), point(first, j), colorFor(0, j))
        mb:triangle(tf, point(last, 1), point(last, j), point(last, j + 1), colorFor(#sections, j))
    end
end

-- An axis-aligned box centred on the local origin.
---@param mb table
---@param tf number[]|nil
---@param w number
---@param h number
---@param d number
---@param color number[]
function shapes.box(mb, tf, w, h, d, color)
    local x, y, z = w / 2, h / 2, d / 2
    -- stylua: ignore
    local p = {
        { -x, -y,  z }, { x, -y,  z }, { x, y,  z }, { -x, y,  z },  -- front (+Z)
        { -x, -y, -z }, { x, -y, -z }, { x, y, -z }, { -x, y, -z },  -- back  (-Z)
    }
    mb:quad(tf, p[1], p[2], p[3], p[4], color) -- +Z
    mb:quad(tf, p[6], p[5], p[8], p[7], color) -- -Z
    mb:quad(tf, p[2], p[6], p[7], p[3], color) -- +X
    mb:quad(tf, p[5], p[1], p[4], p[8], color) -- -X
    mb:quad(tf, p[4], p[3], p[7], p[8], color) -- +Y
    mb:quad(tf, p[5], p[6], p[2], p[1], color) -- -Y
end

-- A sphere, expressed as a circle profile swept along a sine silhouette. Used
-- for sword pommels, the shoulder pauldrons and shield bosses.
---@param mb table
---@param tf number[]|nil
---@param radius number
---@param rings number
---@param segments number
---@param color number[]
---@param arc number?  -- 1 (default) = full sphere, 0.5 = hemisphere
function shapes.sphere(mb, tf, radius, rings, segments, color, arc)
    local span = arc or 1
    local sections = {}
    for i = 0, rings do
        -- phi runs from the south pole (or the equator, for a dome) to the north.
        local phi = math.pi * (1 - span) + (i / rings) * math.pi * span
        sections[#sections + 1] = { y = -radius * math.cos(phi), w = radius * math.sin(phi) }
    end
    shapes.extrude(mb, tf, shapes.circleProfile(segments), sections, color)
end

-- A rectangle bent around the vertical axis, for curved shield shells.
-- Kept separate from `extrude` because the interesting
-- surface here is the *face*, which needs per-quad colouring to carry a device.
--
-- Local space: the panel is centred on the origin, curving away from +Z, so the
-- painted face looks toward +Z.
---@param colorAt fun(u: number, v: number): number[]  -- u, v in [0, 1] across the face
function shapes.curvedPanel(mb, tf, w, h, thickness, curve, segments, colorAt, edge)
    local radius = w / curve
    local half_h = h / 2

    -- front[i] / back[i] are the two surfaces at column i.
    local front, back = {}, {}
    for i = 0, segments do
        local u = i / segments
        local theta = (u - 0.5) * curve
        local sin, cos = math.sin(theta), math.cos(theta)
        front[i] = { radius * sin, radius * cos - radius + thickness }
        back[i] = { (radius - thickness) * sin, (radius - thickness) * cos - radius + thickness }
    end

    for i = 0, segments - 1 do
        local u0, u1 = i / segments, (i + 1) / segments
        local f0, f1, b0, b1 = front[i], front[i + 1], back[i], back[i + 1]

        -- Painted face, split vertically so the device can vary along v too.
        local rows = 10
        for r = 0, rows - 1 do
            local y0 = -half_h + h * (r / rows)
            local y1 = -half_h + h * ((r + 1) / rows)
            local v = (r + 0.5) / rows
            mb:quad(
                tf,
                { f0[1], y0, f0[2] },
                { f1[1], y0, f1[2] },
                { f1[1], y1, f1[2] },
                { f0[1], y1, f0[2] },
                colorAt((u0 + u1) / 2, v)
            )
        end

        -- Inside face and the top/bottom rims.
        mb:quad(
            tf,
            { b1[1], -half_h, b1[2] },
            { b0[1], -half_h, b0[2] },
            { b0[1], half_h, b0[2] },
            { b1[1], half_h, b1[2] },
            edge
        )
        mb:quad(
            tf,
            { f0[1], half_h, f0[2] },
            { f1[1], half_h, f1[2] },
            { b1[1], half_h, b1[2] },
            { b0[1], half_h, b0[2] },
            edge
        )
        mb:quad(
            tf,
            { b0[1], -half_h, b0[2] },
            { b1[1], -half_h, b1[2] },
            { f1[1], -half_h, f1[2] },
            { f0[1], -half_h, f0[2] },
            edge
        )
    end

    -- Left and right vertical rims.
    for _, side in ipairs({ 0, segments }) do
        local f, b = front[side], back[side]
        mb:quad(
            tf,
            { f[1], -half_h, f[2] },
            { b[1], -half_h, b[2] },
            { b[1], half_h, b[2] },
            { f[1], half_h, f[2] },
            edge
        )
    end
end

-- A flat horizontal disc on the XZ plane. The arena floor and the drop shadow.
---@param colorAt fun(ring: number, angle: number): number[]
function shapes.disc(mb, tf, radius, rings, segments, colorAt)
    for r = 0, rings - 1 do
        local r0 = radius * (r / rings)
        local r1 = radius * ((r + 1) / rings)
        for s = 0, segments - 1 do
            local a0 = (s / segments) * math.pi * 2
            local a1 = ((s + 1) / segments) * math.pi * 2
            local color = colorAt((r + 0.5) / rings, (s + 0.5) / segments)
            mb:quad(
                tf,
                { r0 * math.cos(a0), 0, r0 * math.sin(a0) },
                { r1 * math.cos(a0), 0, r1 * math.sin(a0) },
                { r1 * math.cos(a1), 0, r1 * math.sin(a1) },
                { r0 * math.cos(a1), 0, r0 * math.sin(a1) },
                color
            )
        end
    end
end

return shapes
