-- One head presentation per theme.
--
-- Every helm is positioned from the ACTUAL head geometry it is going onto --
-- half-width, base, height and eye line are all passed in -- so the same helm
-- fits a natural head, a minifig cylinder and an oversized vinyl skull without
-- a per-figure special case.
--
-- All three are deliberately OPEN-FACED. The v1 great helm was a closed barrel
-- that hid the entire face, which is a large part of why the characters read as
-- robots. A helmet that frames a face beats a helmet that replaces one.
--
-- Each returns (solid, emissive). The crest / band / key is wired to the TEAM
-- trim colour, per the roster's team-ownership rule.

local mat4 = require("core.mat4")
local meshbuilder = require("game.render.rig3d.meshbuilder")
local shapes = require("game.render.rig3d.shapes")

local headgear = {}

-- A fin of quads following a half-circle silhouette: plumes and crests.
local function crest(mb, span, height, base_y, thickness, color)
    local segments = 14
    for i = 0, segments - 1 do
        local t0, t1 = i / segments, (i + 1) / segments
        local x0, x1 = (t0 - 0.5) * 2 * span, (t1 - 0.5) * 2 * span
        local curve = 0.30 / math.max(span, 1e-6)
        local y0, y1 = base_y - curve * x0 * x0, base_y - curve * x1 * x1
        local h0 = height * math.sin(t0 * math.pi) ^ 0.7
        local h1 = height * math.sin(t1 * math.pi) ^ 0.7
        local d = thickness
        mb:quad(
            nil,
            { x0, y0, -d },
            { x1, y1, -d },
            { x1, y1 + h1, -d },
            { x0, y0 + h0, -d },
            color
        )
        mb:quad(nil, { x1, y1, d }, { x0, y0, d }, { x0, y0 + h0, d }, { x1, y1 + h1, d }, color)
        mb:quad(
            nil,
            { x0, y0 + h0, -d },
            { x1, y1 + h1, -d },
            { x1, y1 + h1, d },
            { x0, y0 + h0, d },
            color
        )
    end
end

-- Medieval Fantasy: an open barbute. Bowl, brow band, nasal bar, cheek plates,
-- transverse plume -- and a clear opening for the eyes and mouth.
function headgear.great_helm(c, h)
    local mb = meshbuilder.new()
    local r = h.hr

    shapes.sphere(mb, mat4.translation(0, h.eye + r * 0.36, 0), r * 1.14, 6, 14, c.plate, 0.62)
    shapes.extrude(mb, nil, shapes.circleProfile(14), {
        { y = h.eye + r * 0.26, w = r * 1.10, d = r * 1.06 },
        { y = h.eye + r * 0.44, w = r * 1.18, d = r * 1.14 },
        { y = h.eye + r * 0.60, w = r * 1.12, d = r * 1.08 },
    }, c.accent)
    -- Nasal bar down the centre of the opening.
    shapes.box(
        mb,
        mat4.translation(0, h.eye - r * 0.10, r * 0.92),
        r * 0.15,
        r * 0.80,
        r * 0.16,
        c.plate
    )
    -- Cheek plates, leaving the middle of the face clear.
    for _, side in ipairs({ -1, 1 }) do
        shapes.extrude(
            mb,
            mat4.multiply(
                mat4.translation(side * r * 0.86, 0, r * 0.10),
                mat4.rotationZ(math.rad(-side * 6))
            ),
            shapes.boxProfile(0.80, 0.45),
            {
                { y = h.eye - r * 0.72, w = r * 0.26 },
                { y = h.eye - r * 0.10, w = r * 0.34 },
                { y = h.eye + r * 0.34, w = r * 0.32 },
            },
            c.plate
        )
    end
    local top = h.base + h.hh
    shapes.box(mb, mat4.translation(0, top * 0.99, 0), r * 2.0, r * 0.14, r * 0.34, c.accent)
    crest(mb, r * 0.94, r * 0.80, top * 1.02, r * 0.12, c.crest)
    return mb:build(), nil
end

-- Galactic Sci-Fi: a swept helm whose emissive band sits ABOVE the eyes, so the
-- visor reads as tech without blanking the face.
function headgear.visor_helm(c, h)
    local mb = meshbuilder.new()
    local r = h.hr

    shapes.sphere(
        mb,
        mat4.translation(0, h.eye + r * 0.42, -r * 0.04),
        r * 1.12,
        7,
        14,
        c.plate,
        0.60
    )
    shapes.extrude(mb, nil, shapes.circleProfile(14), {
        { y = h.eye + r * 0.30, w = r * 1.06, d = r * 1.02 },
        { y = h.eye + r * 0.46, w = r * 1.16, d = r * 1.12 },
    }, c.plate_dark)
    for _, side in ipairs({ -1, 1 }) do
        shapes.box(
            mb,
            mat4.multiply(
                mat4.translation(side * r * 1.02, h.eye + r * 0.30, -r * 0.22),
                mat4.rotationZ(math.rad(-side * 14))
            ),
            r * 0.18,
            r * 0.66,
            r * 0.92,
            c.accent
        )
        -- Jaw guard under the cheeks, still clear of the mouth.
        shapes.box(
            mb,
            mat4.translation(side * r * 0.84, h.eye - r * 0.42, r * 0.26),
            r * 0.24,
            r * 0.70,
            r * 0.62,
            c.accent
        )
    end

    local glow = meshbuilder.new()
    local segments = 12
    for i = 0, segments - 1 do
        local function ring(t, y, rad)
            local a = (t - 0.5) * 2.3
            return { rad * math.sin(a), y, rad * 0.98 * math.cos(a) }
        end
        local t0, t1 = i / segments, (i + 1) / segments
        local lo, hi = h.eye + r * 0.50, h.eye + r * 0.70
        glow:quad(
            nil,
            ring(t0, lo, r * 1.14),
            ring(t1, lo, r * 1.14),
            ring(t1, hi, r * 1.08),
            ring(t0, hi, r * 1.08),
            c.seam
        )
    end
    return mb:build(), glow:build()
end

-- Toybox: a moulded action-figure cap plus the wind-up key. Sits high, so the
-- whole face shows -- which is the point of the theme.
function headgear.figure_helm(c, h)
    local mb = meshbuilder.new()
    local r = h.hr

    shapes.sphere(
        mb,
        mat4.translation(0, h.eye + r * 0.50, -r * 0.05),
        r * 1.10,
        7,
        14,
        c.plate,
        0.56
    )
    shapes.extrude(mb, nil, shapes.circleProfile(14), {
        { y = h.eye + r * 0.34, w = r * 1.04, d = r * 1.02 },
        { y = h.eye + r * 0.52, w = r * 1.16, d = r * 1.12 },
        { y = h.eye + r * 0.68, w = r * 1.10, d = r * 1.06 },
    }, c.accent)
    for _, side in ipairs({ -1, 1 }) do
        shapes.sphere(
            mb,
            mat4.translation(side * r * 1.02, h.eye + r * 0.36, 0),
            r * 0.30,
            5,
            10,
            c.accent
        )
    end

    -- Wind-up key: the Toybox joke in one shape.
    local key_tf = mat4.multiply(
        mat4.translation(0, h.eye + r * 0.90, -r * 1.00),
        mat4.rotationX(math.rad(-18))
    )
    shapes.extrude(mb, key_tf, shapes.circleProfile(8), {
        { y = 0, w = r * 0.12 },
        { y = r * 0.50, w = r * 0.10 },
    }, c.joint)
    for _, side in ipairs({ -1, 1 }) do
        shapes.extrude(
            mb,
            mat4.multiply(key_tf, mat4.translation(side * r * 0.26, r * 0.52, 0)),
            shapes.boxProfile(0.30, 0.9),
            {
                { y = -r * 0.19, w = r * 0.27 },
                { y = r * 0.19, w = r * 0.27 },
            },
            c.joint
        )
    end
    crest(mb, r * 0.72, r * 0.42, h.base + h.hh * 1.02, r * 0.13, c.crest)
    return mb:build(), nil
end

-- `head` carries the geometry of the skull this helm has to fit:
--   { hr = half-width, base = base Y, hh = height, eye = eye-line Y }
---@param id string
---@param c table  -- themes.SLOT_INDEX: slot name -> palette slot index
---@param head table
---@return love.Mesh, love.Mesh|nil
function headgear.build(id, c, head)
    local fn = headgear[id]
    assert(fn, "unknown headgear id: " .. tostring(id))
    return fn(c, head)
end

return headgear
