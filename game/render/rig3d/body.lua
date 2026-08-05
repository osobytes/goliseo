-- Builds one themed character presentation on the shared Rig_Medium skeleton.
--
-- Nothing in this file knows which theme it is building. It reads shape flags
-- and a resolved palette out of parts/themes.lua and proportions out of
-- rig/proportions.lua, so all three presentations come from this one code path.
-- If a theme ever needs a bespoke generator, that is a signal the theme contract
-- is under-specified, not that this file should grow a branch.
--
-- Kit rules -- what makes armour read as *equipped* rather than as body paint,
-- learned the hard way when v1 buried the greaves inside the shins:
--   1. every piece is authored as its own PART: its own builder, in its own
--      local space, riding its own bone, with its own material;
--   2. it is `gear.over` times wider than the limb it wraps;
--   3. it flares into a rim at each open end;
--   4. a dark strap ring sits in the gap where it meets the body.
--
-- Rule 1 used to read "every piece is its own mesh and its own draw call", and
-- #337 slice 2 overturned that MECHANISM: the parts are merged into one static
-- mesh and the whole character is one draw call. The RULE survives because the
-- draw call was never what did the work. What sells "equipped" is entirely
-- geometry -- the `gear.over` standoff of 2, the rim flare of 3, the dark strap
-- ring of 4 -- and geometry is authored per part whether or not each part is
-- submitted separately. Keeping a piece a distinct part is still load-bearing:
-- it is the unit that carries a bone, a material and an attach transform, and
-- the unit whose local space lets 2-4 be written in limb-relative numbers
-- instead of rig-absolute ones. Merge parts into one BUILDER, never into one
-- shape.

local mat4 = require("core.mat4")
local meshbuilder = require("game.render.rig3d.meshbuilder")
local shapes = require("game.render.rig3d.shapes")
local skeleton = require("game.render.rig3d.skeleton")
local themes = require("game.render.rig3d.themes")
local equipment = require("game.render.rig3d.equipment")
local headgear = require("game.render.rig3d.headgear")
local face = require("game.render.rig3d.face")

local body = {}

local SIDES = { { name = "R", sign = -1 }, { name = "L", sign = 1 } } -- his right is -X

-- Where each item hangs. The three light_melee items deliberately share one
-- entry: same bone, same transform, different geometry. That is the reskin test.
local SOCKETS = {
    tournament_sword = "hand_r",
    vector_blade = "hand_r",
    foam_champion = "hand_r",
    heater_shield = "forearm_l",
    spring_glove = "hand_l",
    pulse_blaster = "hip",
}

-- ---------------------------------------------------------------------------
-- Shared building blocks
-- ---------------------------------------------------------------------------

-- A limb: a tube from the joint down -Y with rounded ends.
local function limb(mb, f, r0, r1, length, color)
    local c0, c1 = r0 * f.cap * 0.42, r1 * f.cap * 0.42
    shapes.extrude(mb, nil, shapes.circleProfile(f.segments), {
        { y = c0 * 0.95, w = r0 * 0.52 },
        { y = c0 * 0.55, w = r0 * 0.88 },
        { y = 0, w = r0 },
        { y = -length, w = r1 },
        { y = -length - c1 * 0.55, w = r1 * 0.88 },
        { y = -length - c1 * 0.95, w = r1 * 0.52 },
    }, color)
end

-- An armour sleeve standing proud of a limb, flared into a lip at both ends,
-- with straps biting into the gap.
local function sleeve(mb, rig, limb_r, y0, y1, color, strap)
    local f = rig.form
    local r = limb_r * rig.gear.over
    local lip = math.min((y1 - y0) * 0.16, r * 0.30)
    local profile = shapes.circleProfile(f.segments)

    shapes.extrude(mb, nil, profile, {
        { y = y0, w = r * 1.13 },
        { y = y0 + lip, w = r },
        { y = y1 - lip, w = r },
        { y = y1, w = r * 1.13 },
    }, color)
    for _, y in ipairs({ y0 + lip * 1.25, y1 - lip * 1.25 }) do
        shapes.extrude(mb, nil, profile, {
            { y = y - lip * 0.22, w = r * 1.04 },
            { y = y + lip * 0.22, w = r * 1.04 },
        }, strap)
    end
end

-- A thin ring. Emissive seams and plate ridges are both this shape.
local function band(mb, f, r, y, half_h, color)
    shapes.extrude(mb, nil, shapes.circleProfile(f.segments), {
        { y = y - half_h, w = r },
        { y = y + half_h, w = r },
    }, color)
end

-- ---------------------------------------------------------------------------
-- Body
-- ---------------------------------------------------------------------------

-- Head silhouette per figure style. Returns the extrude sections, the profile
-- to sweep, and a function giving the front-surface Z at any height -- which is
-- what lets face.lua push features onto any head shape.
local function headForm(figure, hr, base_y)
    local hh = hr * figure.head_h
    local sections, profile, depth = {}, nil, 0.92

    if figure.head_shape == "cylinder" then
        -- Lego: a straight cylinder, very slightly barrelled at the rim.
        profile, depth = shapes.circleProfile(16), 1.0
        sections = {
            { y = base_y, w = hr * 0.96 },
            { y = base_y + hh * 0.06, w = hr },
            { y = base_y + hh * 0.94, w = hr },
            { y = base_y + hh, w = hr * 0.96 },
        }
    elseif figure.head_shape == "boxround" then
        -- Funko: a tall rounded box, flat-sided with domed ends.
        profile, depth = shapes.boxProfile(0.86, 0.80), 0.86
        for i = 0, 10 do
            local t = i / 10
            sections[#sections + 1] = {
                y = base_y + t * hh,
                w = hr * math.sin(math.pi * (0.13 + 0.74 * t)) ^ 0.30,
            }
        end
    else
        profile, depth = shapes.boxProfile(0.92, 0.55), 0.92
        for i = 0, 9 do
            local t = i / 9
            sections[#sections + 1] = {
                y = base_y + t * hh,
                w = hr * math.sin(math.pi * (0.08 + 0.84 * t)) ^ 0.62,
            }
        end
    end

    -- Linear search + lerp: the section list is a handful of entries.
    local function frontAt(y)
        for i = 1, #sections - 1 do
            local a, b = sections[i], sections[i + 1]
            if y >= a.y and y <= b.y then
                local u = (y - a.y) / math.max(b.y - a.y, 1e-6)
                return (a.w + (b.w - a.w) * u) * depth * 0.94
            end
        end
        return sections[#sections].w * depth * 0.94
    end

    return sections, profile, hh, frontAt
end

-- A Lego C-clamp hand: a broken ring whose gap faces forward, so a weapon shaft
-- passes through it.
local function clampHand(mb, r, color)
    local tube = r * 0.34
    local steps = 10
    for i = 0, steps do
        local a = math.pi * 0.5 + 0.62 + (i / steps) * (math.pi * 2 - 1.24)
        shapes.sphere(
            mb,
            mat4.translation(r * math.cos(a), -r * 1.1, r * math.sin(a)),
            tube,
            4,
            8,
            color
        )
    end
end

-- The skull's measurements, shared by the face and by whatever helm goes on it.
---@return table  -- { hr, base, hh, eye }
local function headGeometry(rig, figure)
    local f = rig.form
    local hr = f.head_r * figure.head_scale
    local base = 0.02 + figure.head_drop
    local hh = hr * figure.head_h
    return { hr = hr, base = base, hh = hh, eye = base + hh * f.eye_y }
end

local function buildBody(add, rig, theme, figure, c)
    local s, f, shape = rig.seg, rig.form, theme.shape

    -- Torso.
    local torso = meshbuilder.new()
    local bulge = f.torso_r * f.torso_bulge
    if figure.torso == "trapezoid" then
        -- Minifig: a flat-sided block, wider at the hem than the shoulders.
        shapes.extrude(torso, nil, shapes.boxProfile(0.62, 0.18), {
            { y = -s.spine - 0.02, w = bulge * 1.04 },
            { y = s.chest * 0.55, w = bulge * 0.94 },
            { y = s.chest + s.neck * 0.34, w = bulge * 0.86 },
            { y = s.chest + s.neck * 0.60, w = bulge * 0.62 },
        }, c.cloth)
    else
        shapes.extrude(torso, nil, shapes.boxProfile(0.66, shape.bevel), {
            { y = -s.spine - 0.02, w = f.torso_r * 0.86 },
            { y = 0, w = f.torso_r },
            { y = s.chest * 0.55, w = bulge },
            { y = s.chest + s.neck * 0.30, w = bulge * 0.94 },
            { y = s.chest + s.neck * 0.62, w = f.torso_r * 0.72 },
        }, c.cloth)
    end
    add("spine", torso)

    if figure.neck then
        local neck = meshbuilder.new()
        limb(neck, f, f.neck_r, f.neck_r * 0.94, s.neck * 0.55, c.skin)
        add("neck", neck)
    end

    -- Head + face.
    local head = meshbuilder.new()
    local g = headGeometry(rig, figure)
    local sections, profile, _, frontAt = headForm(figure, g.hr, g.base)
    shapes.extrude(head, nil, profile, sections, c.skin)
    if figure.stud then
        shapes.extrude(head, nil, shapes.circleProfile(12), {
            { y = g.base + g.hh, w = g.hr * 0.40 },
            { y = g.base + g.hh + g.hr * 0.16, w = g.hr * 0.40 },
        }, c.skin)
    end
    face.build(head, c, g.hr, figure.face, frontAt, g.eye)
    add("head", head)

    for _, side in ipairs(SIDES) do
        local n = side.name
        local tube = figure.limbs == "tube"
        local taper = tube and 1.0 or f.arm_taper
        local leg_taper = tube and 1.0 or f.leg_taper

        local upper = meshbuilder.new()
        limb(upper, f, f.arm_r, f.arm_r * taper, s.upperarm, c.limbs)
        add("upper_arm." .. n, upper)

        local lower = meshbuilder.new()
        limb(lower, f, f.arm_r * taper, f.arm_r * taper ^ 2, s.lowerarm, c.limbs)
        add("forearm." .. n, lower)

        local hand = meshbuilder.new()
        if figure.hands == "clamp" then
            clampHand(hand, f.hand_r * 1.05, c.limbs)
        else
            shapes.sphere(hand, mat4.translation(0, -f.hand_r * 0.95, 0), f.hand_r, 5, 10, c.limbs)
        end
        add("hand." .. n, hand)

        local thigh = meshbuilder.new()
        limb(thigh, f, f.leg_r, f.leg_r * leg_taper, s.upperleg, c.cloth)
        add("thigh." .. n, thigh)

        local shin = meshbuilder.new()
        limb(shin, f, f.leg_r * leg_taper, f.leg_r * leg_taper ^ 2, s.lowerleg, c.limbs)
        add("shin." .. n, shin)

        if shape.joint_balls then
            for _, joint in ipairs({
                { bone = "upper_arm." .. n, r = f.arm_r * 1.16 },
                { bone = "forearm." .. n, r = f.arm_r * taper * 1.18 },
                { bone = "thigh." .. n, r = f.leg_r * 1.12 },
                { bone = "shin." .. n, r = f.leg_r * leg_taper * 1.16 },
            }) do
                local ball = meshbuilder.new()
                shapes.sphere(ball, nil, joint.r, 6, f.segments, c.joint)
                add(joint.bone, ball)
            end
        end
    end
end

-- ---------------------------------------------------------------------------
-- Kit
-- ---------------------------------------------------------------------------

local function buildKit(add, rig, theme, c)
    local s, f, shape = rig.seg, rig.form, theme.shape
    local over = rig.gear.over

    local cuirass = meshbuilder.new()
    local cr = f.torso_r * f.torso_bulge * over
    shapes.extrude(cuirass, nil, shapes.boxProfile(0.66, shape.bevel), {
        { y = -s.spine * 0.35, w = cr * 1.10 },
        { y = -s.spine * 0.05, w = cr * 0.97 },
        { y = s.chest * 0.55, w = cr },
        { y = s.chest + s.neck * 0.26, w = cr * 0.93 },
        { y = s.chest + s.neck * 0.46, w = cr * 0.74 },
    }, c.plate)
    for i = 1, shape.plate_layers do
        local t = shape.plate_layers > 1 and (i - 1) / (shape.plate_layers - 1) or 0
        local y = s.chest * (0.08 + 0.30 * t)
        shapes.extrude(cuirass, nil, shapes.boxProfile(0.66, shape.bevel), {
            { y = y - s.chest * 0.045, w = cr * 1.05 },
            { y = y + s.chest * 0.045, w = cr * 1.05 },
        }, c.plate_dark)
    end
    if shape.heraldic_block then
        shapes.box(
            cuirass,
            mat4.translation(0, s.chest * 0.66, cr * 0.70),
            cr * 0.60,
            s.chest * 0.48,
            0.030,
            c.cloth
        )
        shapes.box(
            cuirass,
            mat4.translation(0, s.chest * 0.66, cr * 0.72),
            cr * 0.18,
            s.chest * 0.48,
            0.030,
            c.accent
        )
    end
    add("chest", cuirass, nil, "metal")

    -- Sci-Fi: luminous seams tracing the panel joins.
    if shape.seams then
        local seam = meshbuilder.new()
        shapes.extrude(seam, nil, shapes.boxProfile(0.66, shape.bevel), {
            { y = s.chest * 0.50, w = cr * 1.02 },
            { y = s.chest * 0.545, w = cr * 1.02 },
        }, c.seam)
        shapes.box(
            seam,
            mat4.translation(0, s.chest * 0.82, cr * 0.72),
            cr * 0.14,
            s.chest * 0.28,
            0.022,
            c.seam
        )
        add("chest", seam, nil, "emissive")
    end

    local belt = meshbuilder.new()
    local br = f.torso_r * over * 0.98
    shapes.extrude(belt, nil, shapes.boxProfile(0.66, shape.bevel), {
        { y = -0.012, w = br * 1.03 },
        { y = 0.048, w = br * 1.03 },
    }, c.strap)
    shapes.box(belt, mat4.translation(0, 0.018, br * 0.74), br * 0.30, 0.052, 0.030, c.accent)
    add("hips", belt, nil, "metal")

    for _, side in ipairs(SIDES) do
        local n = side.name

        local pauldron = meshbuilder.new()
        local pr = f.arm_r * over * 1.62
        shapes.sphere(pauldron, mat4.translation(0, -pr * 0.16, 0), pr, 5, f.segments, c.plate)
        band(pauldron, f, pr * 0.96, -pr * 0.46, pr * 0.13, c.plate_dark)
        add("shoulder." .. n, pauldron, nil, "metal")

        local bracer = meshbuilder.new()
        sleeve(
            bracer,
            rig,
            f.arm_r * f.arm_taper,
            -s.lowerarm * 0.88,
            -s.lowerarm * 0.30,
            c.accent,
            c.strap
        )
        add("forearm." .. n, bracer, nil, "metal")

        local greave = meshbuilder.new()
        sleeve(
            greave,
            rig,
            f.leg_r * f.leg_taper,
            -s.lowerleg * 0.86,
            -s.lowerleg * 0.16,
            c.plate,
            c.strap
        )
        add("shin." .. n, greave, nil, "metal")

        if shape.seams then
            local seam = meshbuilder.new()
            band(
                seam,
                f,
                f.leg_r * f.leg_taper * over * 1.02,
                -s.lowerleg * 0.42,
                s.lowerleg * 0.020,
                c.seam
            )
            add("shin." .. n, seam, nil, "emissive")
        end

        -- Heel and midfoot ride the ankle...
        local boot = meshbuilder.new()
        shapes.extrude(boot, nil, shapes.boxProfile(1.15, 0.30), {
            { y = -f.foot_w * 0.62, w = f.foot_w * 0.50, z = f.foot_len * 0.06 },
            { y = -f.foot_w * 0.16, w = f.foot_w * 0.48, z = f.foot_len * 0.05 },
            { y = f.foot_w * 0.22, w = f.foot_w * 0.44, z = 0 },
        }, c.strap)
        shapes.box(
            boot,
            mat4.translation(0, -f.foot_w * 0.72, f.foot_len * 0.06),
            f.foot_w,
            f.foot_w * 0.22,
            f.foot_len * 0.66,
            c.plate_dark
        )
        add("foot." .. n, boot)

        -- ...and the toe box rides the ball of the foot, so a plant, a push-off
        -- or a kick contact can actually roll.
        local toe = meshbuilder.new()
        shapes.extrude(toe, nil, shapes.boxProfile(0.86, 0.45), {
            { y = -f.foot_w * 0.26, w = f.foot_w * 0.46, z = f.foot_len * 0.09 },
            { y = f.foot_w * 0.12, w = f.foot_w * 0.42, z = f.foot_len * 0.05 },
        }, c.strap)
        shapes.box(
            toe,
            mat4.translation(0, -f.foot_w * 0.36, f.foot_len * 0.09),
            f.foot_w * 0.94,
            f.foot_w * 0.20,
            f.foot_len * 0.36,
            c.plate_dark
        )
        add("toe." .. n, toe)
    end
end

-- ---------------------------------------------------------------------------
-- Loadout
-- ---------------------------------------------------------------------------

-- Sockets are BONES now (see rig/skeleton.lua), so most of the old attachment
-- matrices are gone: the bone's rest offset and rest rotation carry the grip.
-- What is left here is the small per-item adjustment on top of its socket.
---@return string, number[]|nil  -- bone name, optional local transform
local function socketTransform(socket, rig)
    local s = rig.seg
    if socket == "hand_r" or socket == "hand_l" then
        return "socket_hand." .. (socket == "hand_r" and "R" or "L"), nil
    elseif socket == "forearm_l" then
        -- Slide the shell down its own axis so it covers hip to shoulder.
        return "socket_shield.L", mat4.translation(0, -0.10, 0)
    elseif socket == "hip" then
        -- Holstered on his left hip for a cross-body draw.
        return "hips",
            mat4.chain(
                mat4.translation(s.thigh_x * 1.5, -0.045, -0.020),
                mat4.rotationZ(math.rad(14)),
                mat4.rotationX(math.rad(-8))
            )
    end
    error("unknown socket: " .. tostring(socket))
end

local function buildLoadout(add, rig, theme, figure, c)
    local solid, glow = headgear.build(theme.head, c, headGeometry(rig, figure))
    add("head", solid, nil, "metal")
    if glow then
        add("head", glow, nil, "emissive")
    end

    for _, id in pairs(theme.loadout) do
        local socket = SOCKETS[id] or error("no socket for " .. id)
        local bone, attach = socketTransform(socket, rig)
        local item, item_glow = equipment.build(id, c)
        add(bone, item, attach, "metal")
        if item_glow then
            add(bone, item_glow, attach, "emissive")
        end
    end
end

-- ---------------------------------------------------------------------------

-- Generates every PART of one themed character and folds them into the single
-- builder that becomes one draw call.
--
-- Deliberately separate from `body.build`: everything here is pure Lua, so the
-- whole generation path -- bone assignment, material assignment, attach baking,
-- the merge itself -- is exercised by the headless tier. Only `body.build`'s
-- final `:build()` needs a GL context. #340 is open precisely because nothing
-- used to test the rigged renderer at all; this seam is what makes that
-- testable rather than hoping a green suite means something.
--
-- Colour is NOT baked in (#337 slice 1): every vertex carries a palette slot
-- index, resolved against a team's actual colours later by the shader via a
-- uniform (see themes.resolvedPalette and renderer.beginPass). That is what
-- makes this build reusable across every team -- and every future cosmetic
-- palette -- without rebuilding a single mesh.
---@param rig table    -- proportions.RIG_MEDIUM
---@param theme table  -- an entry from themes.LIST
---@param figure table -- an entry from themes.FIGURES
---@return table       -- the merged meshbuilder Builder
---@return table[]     -- the parts that went into it, for diagnostics and tests
function body.accumulate(rig, theme, figure)
    local c = themes.SLOT_INDEX
    local bone_index = skeleton.boneIndex(rig)
    local parts = {}
    local function add(bone, builder, attach, material)
        local index = bone_index[bone]
        -- A typo'd bone name used to place a part at the identity transform and
        -- render it at the character's feet; now it would index off the end of
        -- the bone uniform array, which is undefined rather than merely wrong.
        assert(index, "rig3d part refers to a bone the skeleton does not have: " .. tostring(bone))
        parts[#parts + 1] = {
            builder = builder,
            bone_name = bone,
            bone = index,
            attach = attach,
            material = material,
        }
    end

    buildBody(add, rig, theme, figure, c)
    buildKit(add, rig, theme, c)
    buildLoadout(add, rig, theme, figure, c)
    return meshbuilder.merge(parts), parts
end

-- Builds one theme's character as ONE static mesh (#337 slice 2).
---@param rig table    -- proportions.RIG_MEDIUM
---@param theme table  -- an entry from themes.LIST
---@param figure table -- an entry from themes.FIGURES
---@return table       -- { mesh = love.Mesh, part_count = integer, triangle_count = integer }
function body.build(rig, theme, figure)
    local merged, parts = body.accumulate(rig, theme, figure)
    return {
        mesh = merged:build(),
        part_count = #parts,
        triangle_count = merged:triangleCount(),
    }
end

return body
