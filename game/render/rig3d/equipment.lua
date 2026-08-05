-- The six prototype equipment presentations, one loadout per theme.
--
-- The interesting one is `light_melee`: Tournament Sword, Vector Blade and Foam
-- Champion are three completely different objects that share one simulation
-- family and one semantic animation verb. The roster calls this "the deliberate
-- cross-theme reskin test", so all three are built to hang off the same right
-- hand socket with the same attachment transform -- swapping them changes only
-- what you see.
--
-- Local space for every hand item: grip centred on the origin, business end
-- running up +Y. The hand attachment in body.lua assumes exactly that.

local mat4 = require("core.mat4")
local meshbuilder = require("game.render.rig3d.meshbuilder")
local shapes = require("game.render.rig3d.shapes")

local equipment = {}

-- A hexagonal blade cross-section: two cutting edges at x = +/-1 with flat
-- faces meeting at a central ridge.
-- stylua: ignore
local BLADE_PROFILE = {
    { 1, 0 }, { 0.45, 1 }, { -0.45, 1 }, { -1, 0 }, { -0.45, -1 }, { 0.45, -1 },
}

-- Grip + pommel shared by every melee item, so they all sit in the fist the
-- same way regardless of theme.
local function hilt(mb, c, grip_r, grip_top, pommel_r, grip_color, metal)
    shapes.sphere(mb, mat4.translation(0, -grip_top - pommel_r * 0.6, 0), pommel_r, 6, 10, metal)
    shapes.extrude(mb, nil, shapes.circleProfile(10), {
        { y = -grip_top, w = grip_r * 1.06 },
        { y = -grip_top * 0.4, w = grip_r },
        { y = grip_top * 0.5, w = grip_r },
        { y = grip_top, w = grip_r * 1.08 },
    }, grip_color)
end

-- ---------------------------------------------------------------------------
-- Medieval Fantasy
-- ---------------------------------------------------------------------------

-- Tournament Sword: straight arming sword, cruciform guard, wide fuller.
function equipment.tournament_sword(c)
    local mb = meshbuilder.new()
    hilt(mb, c, 0.021, 0.085, 0.034, c.strap, c.accent)

    -- Cruciform quillons: one long bar across, plus a central block.
    shapes.extrude(mb, mat4.rotationZ(math.pi / 2), shapes.circleProfile(8), {
        { y = -0.115, w = 0.016 },
        { y = -0.085, w = 0.021 },
        { y = 0.085, w = 0.021 },
        { y = 0.115, w = 0.016 },
    }, c.accent)
    shapes.box(mb, mat4.translation(0, 0.095, 0), 0.055, 0.045, 0.040, c.accent)

    shapes.extrude(mb, nil, BLADE_PROFILE, {
        { y = 0.115, w = 0.040, d = 0.0095 },
        { y = 0.200, w = 0.038, d = 0.0090 },
        { y = 0.560, w = 0.031, d = 0.0070 },
        { y = 0.680, w = 0.024, d = 0.0055 },
        { y = 0.840, w = 0.002, d = 0.0006 },
    }, c.plate)
    return mb, nil
end

-- Emberguard Shield: a heater shield -- wide flat top, tapering to a point.
-- Built as a stack of curved rings whose width shrinks, so it curves in both
-- axes rather than being a flat plate.
function equipment.heater_shield(c)
    local mb = meshbuilder.new()
    local rows = 12
    local height, top_w, thickness = 0.86, 0.62, 0.030

    local function widthAt(t) -- t: 0 at the point, 1 at the top edge
        return top_w * (0.16 + 0.84 * math.sin(t * math.pi * 0.62) ^ 0.55)
    end

    local prev
    for i = 0, rows do
        local t = i / rows
        local w = widthAt(t)
        local y = -height * 0.5 + height * t
        local ring, radius = {}, w / 1.30
        for j = 0, 6 do
            local a = (j / 6 - 0.5) * 1.30
            ring[j] = { radius * math.sin(a), y, radius * math.cos(a) - radius + thickness }
        end
        if prev then
            for j = 0, 5 do
                -- Face: team-coloured field with a bronze border band.
                local edge = (i <= 1 or i >= rows - 1 or j == 0 or j == 5)
                mb:quad(
                    nil,
                    prev[j],
                    prev[j + 1],
                    ring[j + 1],
                    ring[j],
                    edge and c.accent or c.cloth
                )
                -- Back, pushed in by the thickness.
                local function back(p)
                    return { p[1] * 0.94, p[2], p[3] - thickness }
                end
                mb:quad(
                    nil,
                    back(ring[j]),
                    back(ring[j + 1]),
                    back(prev[j + 1]),
                    back(prev[j]),
                    c.strap
                )
            end
        end
        prev = ring
    end

    -- Boss and the enarmes on the back.
    shapes.sphere(
        mb,
        mat4.multiply(mat4.translation(0, 0.02, thickness), mat4.rotationX(math.pi / 2)),
        0.070,
        5,
        12,
        c.accent,
        0.55
    )
    for _, y in ipairs({ -0.07, 0.09 }) do
        shapes.box(mb, mat4.translation(0, y, -0.045), 0.20, 0.035, 0.030, c.strap)
    end
    return mb, nil
end

-- ---------------------------------------------------------------------------
-- Galactic Sci-Fi
-- ---------------------------------------------------------------------------

-- Vector Blade: a machined hilt projecting a hard-light edge. The blade is
-- returned as a SECOND mesh tagged emissive, so it ignores the lighting model
-- and reads as energy rather than painted metal.
function equipment.vector_blade(c)
    local mb = meshbuilder.new()
    hilt(mb, c, 0.022, 0.090, 0.028, c.plate_dark, c.plate)

    -- Emitter housing.
    shapes.extrude(mb, nil, shapes.boxProfile(0.72, 0.4), {
        { y = 0.090, w = 0.042 },
        { y = 0.128, w = 0.046 },
        { y = 0.165, w = 0.034 },
    }, c.plate)
    shapes.box(mb, mat4.translation(0, 0.106, 0.040), 0.050, 0.030, 0.014, c.plate_dark)

    local glow = meshbuilder.new()
    -- Vent slots in the housing, then the blade itself.
    for _, y in ipairs({ 0.100, 0.122 }) do
        shapes.box(glow, mat4.translation(0, y, 0.047), 0.036, 0.008, 0.006, c.seam)
    end
    shapes.extrude(glow, nil, BLADE_PROFILE, {
        { y = 0.168, w = 0.030, d = 0.0075 },
        { y = 0.300, w = 0.032, d = 0.0080 },
        { y = 0.660, w = 0.027, d = 0.0065 },
        { y = 0.820, w = 0.002, d = 0.0006 },
    }, c.seam)
    return mb, glow
end

-- Pulse Blaster: a compact sidearm. Carried holstered on the hip, which also
-- proves a non-hand attachment socket.
function equipment.pulse_blaster(c)
    local mb = meshbuilder.new()
    shapes.extrude(mb, nil, shapes.boxProfile(0.55, 0.35), { -- grip
        { y = -0.075, w = 0.026 },
        { y = 0.000, w = 0.030 },
    }, c.plate_dark)
    shapes.box(mb, mat4.translation(0, 0.032, 0.020), 0.056, 0.070, 0.150, c.plate) -- receiver
    shapes.extrude(
        mb,
        mat4.multiply(mat4.translation(0, 0.040, 0.115), mat4.rotationX(math.pi / 2)),
        shapes.circleProfile(10),
        { { y = 0, w = 0.026 }, { y = 0.070, w = 0.021 } },
        c.plate_dark
    )

    local glow = meshbuilder.new()
    shapes.box(glow, mat4.translation(0, 0.070, 0.010), 0.030, 0.010, 0.090, c.seam)
    shapes.extrude(
        glow,
        mat4.multiply(mat4.translation(0, 0.040, 0.186), mat4.rotationX(math.pi / 2)),
        shapes.circleProfile(10),
        { { y = 0, w = 0.017 }, { y = 0.010, w = 0.017 } },
        c.seam
    )
    return mb, glow
end

-- ---------------------------------------------------------------------------
-- Toybox
-- ---------------------------------------------------------------------------

-- Foam Champion: a fat, soft, obviously harmless sword. Same socket and same
-- semantic verb as the Tournament Sword and Vector Blade.
function equipment.foam_champion(c)
    local mb = meshbuilder.new()
    hilt(mb, c, 0.026, 0.080, 0.040, c.plate_dark, c.accent)

    -- Chunky moulded guard.
    shapes.extrude(mb, nil, shapes.boxProfile(0.80, 0.85), {
        { y = 0.076, w = 0.052 },
        { y = 0.104, w = 0.078 },
        { y = 0.132, w = 0.050 },
    }, c.accent)

    -- The foam blade: rounded rectangle, barely tapered, domed tip.
    shapes.extrude(mb, nil, shapes.boxProfile(0.44, 0.92), {
        { y = 0.130, w = 0.062 },
        { y = 0.165, w = 0.078 },
        { y = 0.430, w = 0.076 },
        { y = 0.520, w = 0.066 },
        { y = 0.565, w = 0.040 },
        { y = 0.585, w = 0.014 },
    }, c.plate)
    -- A cream stripe so the foam reads as moulded plastic, not a slab.
    shapes.extrude(mb, nil, shapes.boxProfile(0.44, 0.92), {
        { y = 0.300, w = 0.080 },
        { y = 0.330, w = 0.080 },
    }, c.plate_dark)
    return mb, nil
end

-- Spring Glove: an oversized moulded gauntlet with a visible coil.
function equipment.spring_glove(c)
    local mb = meshbuilder.new()
    shapes.sphere(mb, mat4.translation(0, -0.030, 0), 0.088, 6, 12, c.plate)
    -- Cuff.
    shapes.extrude(mb, nil, shapes.circleProfile(12), {
        { y = 0.030, w = 0.086 },
        { y = 0.062, w = 0.094 },
        { y = 0.080, w = 0.080 },
    }, c.accent)
    -- The spring: a stack of rings stepping forward off the knuckles.
    for i = 0, 3 do
        shapes.extrude(
            mb,
            mat4.translation(0, -0.040, 0.078 + i * 0.026),
            shapes.circleProfile(10),
            {
                { y = -0.010, w = 0.050 - i * 0.004 },
                { y = 0.010, w = 0.050 - i * 0.004 },
            },
            i % 2 == 0 and c.plate_dark or c.accent
        )
    end
    return mb, nil
end

-- ---------------------------------------------------------------------------

-- Builds one item by id. Returns the solid PART BUILDER plus an optional
-- emissive one (nil for everything that does not glow). Neither is a mesh: both
-- are folded into the character's single merged mesh by body.build (#337 slice
-- 2), which is what lets an emissive blade cost zero extra draw calls.
---@param id string
---@param c table  -- themes.SLOT_INDEX: slot name -> palette slot index
---@return table, table|nil  -- meshbuilder part builders
function equipment.build(id, c)
    local fn = equipment[id]
    assert(fn, "unknown equipment id: " .. tostring(id))
    return fn(c)
end

return equipment
