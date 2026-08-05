-- Whole-body action poses, as a sparse overlay on the rig root.
--
-- These are the poses the 2.5D renderer expressed as transforms of the whole
-- billboard rather than as limb animation: a keeper's dive, a bicycle kick, a
-- knockback, a stumble. It is worth being precise about why they are ported as
-- ROOT TRANSFORMS and not as clips.
--
-- Every one of them is continuous. A dive is not a fixed silhouette played
-- back; it is the body rotating through an angle as `dive` runs 0 -> 1 against
-- the simulation's own timer. The same is true of the aerial lift, the wind-up
-- and the tip. A keyframe clip would have to re-derive that ramp and would then
-- disagree with the timer driving it, whereas a parameterised transform reads
-- the timer directly -- which is what the 2.5D renderer did, and why its poses
-- stayed in step with the sim.
--
-- So the numbers below are not new. They are the constants the 2.5D renderer
-- already had, tuned against the real game, moved onto the rig. Authored clips
-- for these actions remain #101/#102's job; this is what keeps them legible in
-- the meantime.
--
-- ORIENTATION, since every sign here depends on it (skeleton.lua):
--   +Y is up, the character faces +Z, and because the frame is right-handed
--   their own RIGHT is at -X.
--   * rot.z rotates +X toward +Y, so a POSITIVE z tips the body toward their
--     own right (the head, at +Y, swings to -X).
--   * rot.x rotates +Y toward +Z, so a POSITIVE x tips the body FORWARD onto
--     its face, and a negative x tips it backward.
--   * move on `root` is a translation in the character's own frame before the
--     draw-time yaw, so move.x is sideways and move.z is along their facing.
--     Like every clip translation it rides the rig's motion_scale.
--
-- Distances are expressed in PLAYER RADII, the same unit the 2.5D constants
-- were written in, and converted once on the way out. That keeps the numbers
-- directly comparable to the renderer they came from.

local quat = require("core.quat")

local action_pose = {}

-- One player radius, in metres. The rig is HEIGHT_IN_RADII * 2 radii tall by
-- construction, so this is the conversion and it needs no camera state.
local RADII_PER_HEIGHT = 6.0

-- Keeper saves reuse one body under bounded transforms, so the families stay
-- distinguishable: spread stays compact, central corrects a short distance,
-- stretch holds the full lunge, and a one-shot tip reaches just past it.
---@type table<string, {angle: number, travel: number, floor: number?, fixed: number?}>
local SAVES = {
    keeper_spread = { angle = 28, travel = 0.65 },
    keeper_central = { angle = 48, travel = 0.95 },
    keeper_stretch = { angle = 78, travel = 1.9, floor = 0.82 },
    keeper_tip = { angle = 84, travel = 2.2, fixed = 1 },
    keeper_dive = { angle = 72, travel = 1.6 },
}

---@param x number
---@param lo number
---@param hi number
---@return number
local function clamp(x, lo, hi)
    return math.max(lo, math.min(hi, x))
end

---@return table
local function empty()
    return { rot = {}, move = {} }
end

-- Which way a dive leans, in the character's own frame.
--
-- The 2.5D renderer collapsed this to a screen-space sign, which is right often
-- enough for a keeper facing up the pitch and wrong when they are not. On the
-- rig the honest answer is available: project the dive direction onto the
-- character's own left, which is +X once the draw-time yaw is applied.
--
-- Returns +1 when the dive goes to the keeper's LEFT (toward +X), -1 to their
-- right, and 0 when there is nothing to lean along.
---@param dive_dir table|nil  -- world-space {x, y} on the pitch
---@param facing table|nil    -- world-space {x, y}
---@return number
local function lateralSign(dive_dir, facing)
    if not dive_dir then
        return 0
    end
    -- With no facing the pitch axes are the character's own, so their left is
    -- pitch +x. Matches the 2.5D fallback of leaning along the dominant axis.
    local fx, fy = 1, 0
    if facing then
        fx, fy = facing.x, facing.y
    end
    -- Character's local +X in pitch coordinates is (fy, -fx).
    local along_left = dive_dir.x * fy - dive_dir.y * fx
    if math.abs(along_left) < 1e-6 then
        return 0
    end
    return along_left > 0 and 1 or -1
end

-- The keeper save families, plus the un-posed dive the renderer falls back to
-- when no pose id is supplied.
---@param pose_id string|nil
---@param opts table
---@return table|nil
local function save(pose_id, opts)
    local spec = SAVES[pose_id or ""]
    if not spec and not (pose_id == nil and (opts.dive or 0) > 0) then
        return nil
    end
    spec = spec or SAVES.keeper_dive
    if not opts.dive_dir then
        return nil
    end

    local amount = spec.fixed or clamp(opts.dive or 0, 0, 1)
    if spec.floor then
        amount = math.max(amount, spec.floor)
    end
    local sign = lateralSign(opts.dive_dir, opts.facing)
    if sign == 0 then
        return nil
    end

    local pose = empty()
    -- Head toward the dive side: +X needs a negative z (see the note above).
    pose.rot.root = { 0, 0, -sign * spec.angle * amount }
    pose.move.root = { sign * spec.travel * amount / RADII_PER_HEIGHT, 0, 0 }
    return pose
end

-- Aerials use the ground point for sorting and the shadow but lift the body.
-- A bicycle also rotates it back into a readable overhead silhouette; the other
-- styles are a lift with the limbs posed by their own clip.
---@param pose_id string|nil
---@param opts table
---@return table|nil
local function aerial(pose_id, opts)
    local is_aerial = pose_id == "aerial_bicycle"
        or pose_id == "aerial_action"
        or (pose_id == nil and (opts.aerial or 0) > 0)
    if not (is_aerial and opts.aerial_style) then
        return nil
    end

    local amount = clamp(opts.aerial or 0, 0, 1)
    local lift = (0.35 + 1.65 * (opts.aerial_jump or 0)) * amount
    local pose = empty()
    pose.move.root = { 0, lift / RADII_PER_HEIGHT, 0 }
    if opts.aerial_style == "bicycle" then
        -- Over backwards, which is negative x: the head goes behind the hips.
        pose.rot.root = { -78 * amount, 0, 0 }
    end
    return pose
end

-- Reactions and recoveries, all of them a tip about the character's own
-- left-right axis. The angles are what separate them at a glance, so they are
-- listed together rather than spread through the file.
---@type table<string, {pitch: number, lift: number?, drop: number?}>
local TIPS = {
    -- Driven off their feet, away from whatever hit them.
    combat_knockback = { pitch = -68, lift = 0.45 },
    -- A rocked-back beat, deliberately shallow so it never reads as knockback.
    combat_stagger = { pitch = -8, drop = 0.28 },
    -- Back onto the feet after a save: shallower still, and to the dive side.
    keeper_get_up = { pitch = 0, drop = 0.18 },
}

---@param pose_id string|nil
---@param opts table
---@return table|nil
local function tip(pose_id, opts)
    -- A failed challenge tips the body away from the direction it committed to.
    -- Steeper than a combat stagger and pivoted off the trailing heel, so the
    -- two recoveries never read as the same thing.
    if pose_id == "stumble" then
        local pose = empty()
        pose.rot.root = { -24, 0, 0 }
        pose.move.root = { 0, 0.12 / RADII_PER_HEIGHT, -0.35 / RADII_PER_HEIGHT }
        return pose
    end

    local spec = TIPS[pose_id or ""]
    if not spec then
        return nil
    end

    local pose = empty()
    if pose_id == "keeper_get_up" then
        -- Still leaning on the hand they landed on, so this keeps the dive side.
        local sign = lateralSign(opts.dive_dir, opts.facing)
        pose.rot.root = { 0, 0, -sign * 16 }
    else
        pose.rot.root = { spec.pitch, 0, 0 }
    end
    local dy = (spec.lift or 0) - (spec.drop or 0)
    if dy ~= 0 then
        pose.move.root = { 0, dy / RADII_PER_HEIGHT, 0 }
    end
    return pose
end

-- The root overlay for one player's action, or nil when they are simply running
-- and the locomotion blend is the whole story.
--
-- Order matters: a keeper diving with the ball is diving first, and an aerial
-- beats a reaction, exactly as the 2.5D renderer's early returns had it.
---@param opts table
---@return table|nil  -- { rot = { root = {x,y,z} }, move = { root = {x,y,z} } }
function action_pose.forOptions(opts)
    local pose_id = opts.pose and opts.pose.id or nil
    return save(pose_id, opts) or aerial(pose_id, opts) or tip(pose_id, opts)
end

-- Merges the overlay into an already-resolved pose.
--
-- Kept separate from `forOptions` so the geometry stays pure and testable in
-- the authoring format (degrees, as clips.lua uses), while this side owns the
-- conversion into the quaternions skeleton.apply consumes.
--
-- The overlay only ever touches `root`, so it composes with the gait rather
-- than replacing it: a keeper who dives mid-stride keeps the stride.
---@param pose table
---@param opts table
---@return table pose  -- the same table, mutated
function action_pose.apply(pose, opts)
    local action = action_pose.forOptions(opts)
    if not action then
        return pose
    end
    for bone, r in pairs(action.rot) do
        pose.rot[bone] = quat.fromEuler(math.rad(r[1]), math.rad(r[2]), math.rad(r[3]))
    end
    for bone, m in pairs(action.move) do
        pose.move[bone] = m
    end
    return pose
end

return action_pose
