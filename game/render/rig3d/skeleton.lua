-- The rigged-player skeleton: a concrete proposal for the bone contract #93
-- has to pin down.
--
-- Orientation convention:
--   +Y up, the character faces +Z, and because the frame is right-handed his
--   own RIGHT side is at -X. Bone names use his anatomical left/right.
--
-- Bones are listed parent-before-child, so a single forward pass computes every
-- world transform: world = parent.world * local.
--
-- ---------------------------------------------------------------------------
-- DESIGN RULES (load-bearing decisions, not style preferences)
-- ---------------------------------------------------------------------------
--
-- The plan is to retarget ~131 external CC0 clips (KayKit Rig_Medium) onto this
-- skeleton. That single fact drives everything below:
--
-- 1. CHAIN TOPOLOGY MIRRORS A STANDARD HUMANOID. Inserting a bone into an
--    existing chain changes how rotation distributes along it, so every
--    imported clip would need remapping and name/proportion-based retargeters
--    produce hunched or corkscrewed results. This is the classic retargeting
--    failure mode.
--
--    Consequence: NO third spine segment and no in-chain twist bones. Two spine
--    segments plus neck is enough for bicycle kicks, dives and staggers when
--    the arch is spread across hips + spine + chest + neck. "More bones now,
--    for the future" is the wrong instinct HERE -- the source pack's skeleton
--    is the ceiling, and articulation no clip drives is dead weight.
--
-- 2. LEAF BONES ARE FREE. A bone with no children that no source clip targets
--    simply holds its rest pose. Toes and sockets cost nothing and can be added
--    now or later without touching a single imported clip. Be generous here and
--    nowhere else.
--
-- 3. NAMES ARE A RETARGETING INTERFACE. `hips` (not `pelvis`) because
--    name-matchers look for "hips". `shoulder.L` IS the clavicle -- having both
--    a `clavicle` and a `shoulder` double-matches. `.L`/`.R` suffixes are what
--    Blender symmetry, Rigify, Mixamo converters and Auto-Rig Pro key on. The
--    `socket_` prefix marks non-deform bones so no auto-weighter touches them.
--
-- REMOVED in this pass, and why:
--   * `hip_L/R` between pelvis and thigh -- every humanoid convention goes
--     hips -> thigh directly, AND the name collides with "hips" matching.
--     Double liability.
--   * `shoulder_L/R` as a THIRD joint between clavicle and upper arm -- nobody
--     animates it, so it either sits dead or catches misassigned rotation from
--     a fuzzy retargeter. The clavicle (now named `shoulder`) already gives the
--     shrug and reach that dives, spreads and guard raises need.
--
-- STILL UNVERIFIED: nobody has inspected the actual Rig_Medium GLB. Its real
-- chain -- clavicles or not, toes or not, two spine bones or three -- has to be
-- read out of the file and mirrored bone-for-bone before this is locked. #93
-- and #95 both say these assumptions are tests, not facts.
--
-- Parts and gear ride a bone's world transform. There are no per-vertex skin
-- WEIGHTS here -- one bone drives a vertex outright, which is the prototype's
-- rigid arcade look. Since #337 slice 2 that single influence is carried IN the
-- vertex (a bone index) and resolved on the GPU against `skeleton.boneRows`,
-- rather than by issuing a draw call per part. The production asset will be
-- smoothly skinned; the bone contract is what has to survive.
--
-- BONE INDEX CONTRACT: the order `skeleton.bones` returns IS the GPU bone
-- index order, 0-based. Vertices baked against one order and matrices uploaded
-- against another would silently render a scrambled character, so both sides
-- go through `skeleton.boneIndex` / `skeleton.boneRows` and never count for
-- themselves.

local mat4 = require("core.mat4")
local quat = require("core.quat")

local skeleton = {}

-- Rows uploaded per bone. Every bone transform in this rig is rotation +
-- translation + uniform scale, so the fourth row is always exactly (0, 0, 0, 1)
-- and carries no information -- sending it would burn a quarter of the bone
-- uniform budget on a constant. See rig3d/renderer.lua for the budget maths.
skeleton.ROWS_PER_BONE = 3

local RIGHT, LEFT = -1, 1 -- X sign for each side (see note above)

-- offset: rest translation in the parent's space, in metres.
-- rest:   rest rotation in degrees, added to whatever the animation asks for.
---@param rig table
---@return table[]
function skeleton.bones(rig)
    local s, f = rig.seg, rig.form

    -- Arm: shoulder (the clavicle) -> upper_arm -> forearm -> hand -> socket.
    local function arm(side, sign)
        return {
            {
                name = "shoulder." .. side,
                parent = "chest",
                offset = { sign * s.shoulder_x, s.shoulder_y, 0 },
            },
            {
                name = "upper_arm." .. side,
                parent = "shoulder." .. side,
                offset = { sign * s.arm_x, -s.upperarm * 0.11, 0 },
                rest = { 0, 0, sign * 8 },
            },
            {
                name = "forearm." .. side,
                parent = "upper_arm." .. side,
                offset = { 0, -s.upperarm, 0 },
            },
            { name = "hand." .. side, parent = "forearm." .. side, offset = { 0, -s.lowerarm, 0 } },
            -- Grip socket, non-deform. Everything held in a fist hangs off this,
            -- so the three light_melee items share one attachment id.
            {
                name = "socket_hand." .. side,
                parent = "hand." .. side,
                offset = { 0, -f.hand_r * 1.05, f.hand_r * 0.10 },
                rest = { 145, 0, 0 },
            },
        }
    end

    -- Leg: thigh -> shin -> foot -> toe, parented straight off `hips`.
    local function leg(side, sign)
        return {
            {
                name = "thigh." .. side,
                parent = "hips",
                offset = { sign * s.thigh_x, s.thigh_y, 0 },
                rest = { 0, 0, sign * 2 },
            },
            { name = "shin." .. side, parent = "thigh." .. side, offset = { 0, -s.upperleg, 0 } },
            { name = "foot." .. side, parent = "shin." .. side, offset = { 0, -s.lowerleg, 0 } },
            -- Ball of the foot. Without it the foot is a rigid block: no roll
            -- through a plant, no push-off on a keeper dive, no instep
            -- orientation for a kick contact. Highest-value addition for a
            -- football game, and free because it is a leaf.
            {
                name = "toe." .. side,
                parent = "foot." .. side,
                offset = { 0, -f.foot_w * 0.34, f.foot_len * 0.40 },
            },
        }
    end

    local bones = {
        { name = "root", parent = nil, offset = { 0, 0, 0 } },
        { name = "hips", parent = "root", offset = { 0, s.hips_y, 0 } },
        { name = "spine", parent = "hips", offset = { 0, s.spine, 0 } },
        { name = "chest", parent = "spine", offset = { 0, s.chest, 0 } },
        { name = "neck", parent = "chest", offset = { 0, s.neck, 0 } },
        { name = "head", parent = "neck", offset = { 0, s.head, 0 } },
    }
    for _, group in ipairs({ arm("R", RIGHT), arm("L", LEFT), leg("R", RIGHT), leg("L", LEFT) }) do
        for _, bone in ipairs(group) do
            bones[#bones + 1] = bone
        end
    end

    -- Appended last: these hang off bones the groups above just created.
    local trailing = {
        -- Strapped kit (shields) rides this rather than a matrix baked into the
        -- body builder. The 70 degree rest roll is the negation of the left
        -- forearm's guard-pose angle, which stands a shield upright.
        {
            name = "socket_shield.L",
            parent = "forearm.L",
            offset = { f.arm_r * 1.15, -s.lowerarm * 0.70, f.arm_r * 0.35 },
            rest = { 70, 0, 0 },
        },
        -- Held ball for keeper grab / carry / set. Parented to the chest so a
        -- torso-driven clip carries it. Throw and punt reparent it to
        -- socket_hand.R at runtime -- one socket cannot be in two hands.
        {
            name = "socket_ball",
            parent = "chest",
            offset = { 0, s.chest * 0.34, f.torso_r * 1.45 },
        },
    }
    for _, bone in ipairs(trailing) do
        bones[#bones + 1] = bone
    end
    return bones
end

-- Bone name -> 0-based GPU bone index, in the order `skeleton.bones` lists them.
-- Mesh builders bake this into a vertex; `skeleton.boneRows` uploads matrices in
-- the same order. Pure, and independent of any posed instance, so a mesh can be
-- built long before a skeleton is instantiated.
---@param rig table  -- proportions, e.g. proportions.RIG_MEDIUM
---@return table<string, integer>
function skeleton.boneIndex(rig)
    local out = {}
    for i, bone in ipairs(skeleton.bones(rig)) do
        out[bone.name] = i - 1
    end
    return out
end

---@param rig table  -- proportions, e.g. proportions.RIG_MEDIUM
---@return integer
function skeleton.boneCount(rig)
    return #skeleton.bones(rig)
end

local ZERO = { 0, 0, 0 }
local IDENTITY = quat.identity()

---@param rig table
---@return table
function skeleton.new(rig)
    local out = { style = rig, defs = skeleton.bones(rig), world = {}, byName = {}, order = {} }
    for i, def in ipairs(out.defs) do
        out.order[i] = def.name
    end
    for _, def in ipairs(out.defs) do
        assert(
            def.parent == nil or out.byName[def.parent],
            "parent must precede child: " .. def.name
        )
        out.byName[def.name] = def
    end
    -- Rest orientations are constant, so bake them to quaternions once.
    for _, def in ipairs(out.defs) do
        local r = def.rest or ZERO
        def.q_rest = quat.fromEuler(math.rad(r[1]), math.rad(r[2]), math.rad(r[3]))
    end
    skeleton.apply(out, { rot = {}, move = {} })
    return out
end

-- Evaluates every bone's world transform for one pose. `pose.rot` is in degrees
-- and `pose.move` is an additive translation on top of the rest offset; both are
-- sparse -- a bone the clip never mentions simply sits at rest.
--
-- Clip translations are authored in metres for a full-size figure and scaled by
-- the rig's motion_scale.
---@param rig table
---@param pose table  -- { rot = { [bone] = {x, y, z} }, move = { [bone] = {x, y, z} } }
function skeleton.apply(rig, pose)
    local k = (rig.style and rig.style.motion_scale) or 1
    for _, bone in ipairs(rig.defs) do
        local rot = pose.rot[bone.name] or IDENTITY
        local move = pose.move[bone.name] or ZERO
        local offset = bone.offset

        -- pose * rest, not a sum of angles. Every rest rotation in this rig is
        -- about a single axis, so this is numerically identical to the old
        -- summed-Euler form -- but it stays correct if a rest ever becomes
        -- multi-axis, which summing would not.
        local localTf = quat.toMat4(
            quat.multiply(rot, bone.q_rest),
            offset[1] + move[1] * k,
            offset[2] + move[2] * k,
            offset[3] + move[3] * k
        )

        rig.world[bone.name] = bone.parent and mat4.multiply(rig.world[bone.parent], localTf)
            or localTf
    end
end

-- Flattens the posed skeleton into the bone-matrix uniform payload: three vec4
-- rows per bone, in bone-index order, ready for `Shader:send("u_bones", ...)`.
--
-- `out` is reused across frames on purpose. This runs once per character per
-- frame and would otherwise allocate 3 * bone_count tables every time -- at ten
-- players that is ~47k short-lived tables per second handed straight to the GC,
-- which is exactly the kind of per-frame churn the frame-time gates notice.
-- Shader:send copies immediately, so one buffer can serve every character.
---@param rig table       -- a skeleton.new instance, already posed
---@param out table|nil   -- reusable row buffer
---@return table          -- `out`, filled: { {m11,m12,m13,m14}, {m21,...}, ... }
function skeleton.boneRows(rig, out)
    out = out or {}
    for i, name in ipairs(rig.order) do
        local m = rig.world[name]
        local base = (i - 1) * skeleton.ROWS_PER_BONE
        for r = 0, skeleton.ROWS_PER_BONE - 1 do
            local row = out[base + r + 1]
            if not row then
                row = { 0, 0, 0, 0 }
                out[base + r + 1] = row
            end
            local o = r * 4
            row[1], row[2], row[3], row[4] = m[o + 1], m[o + 2], m[o + 3], m[o + 4]
        end
    end
    return out
end

-- World-space position of a bone's origin. Used to place the drop shadow.
---@return number, number, number
function skeleton.jointPosition(rig, name)
    return mat4.transformPoint(rig.world[name], 0, 0, 0)
end

return skeleton
