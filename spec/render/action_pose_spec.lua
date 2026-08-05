-- The whole-body action overlay is pure geometry, so its sign conventions can
-- be pinned without a window. That matters more here than usual: rig3d/clips.lua
-- records that an early pose leaned backwards because a sign was assumed rather
-- than checked, and every value in action_pose.lua depends on the same
-- orientation rules (+Y up, facing +Z, their own right at -X).

local action_pose = require("game.render.rig3d.action_pose")
local t = require("spec.support.runner")

-- Facing up the pitch (+y). With that facing the character's own LEFT is +X,
-- which is pitch +x, so a dive toward +x is a dive to their left.
local FACING_UP = { x = 0, y = 1 }
local LEFT = { x = 1, y = 0 }
local RIGHT = { x = -1, y = 0 }

---@param id string|nil
---@param extra table|nil
---@return table
local function opts(id, extra)
    local out = { facing = FACING_UP }
    for k, v in pairs(extra or {}) do
        out[k] = v
    end
    if id then
        out.pose = { id = id }
    end
    return out
end

t.describe("rigged whole-body action poses", function()
    t.it("leaves an ordinary run to the locomotion blend", function()
        t.eq(action_pose.forOptions(opts("locomotion")), nil)
        t.eq(action_pose.forOptions(opts(nil)), nil)
    end)

    t.it("tips a keeper toward the side they dive to", function()
        local left = assert(action_pose.forOptions(opts("keeper_dive", {
            dive = 1,
            dive_dir = LEFT,
        })))
        -- Their left is +X, and the head (at +Y) only reaches +X on a NEGATIVE
        -- z rotation. Getting this backwards throws the keeper the wrong way.
        t.is_true(left.rot.root[3] < 0, "diving left must roll negative about z")
        t.is_true(left.move.root[1] > 0, "diving left must travel toward +X")
    end)

    t.it("mirrors the dive exactly", function()
        local left = assert(action_pose.forOptions(opts("keeper_dive", {
            dive = 1,
            dive_dir = LEFT,
        })))
        local right = assert(action_pose.forOptions(opts("keeper_dive", {
            dive = 1,
            dive_dir = RIGHT,
        })))
        t.eq(left.rot.root[3], -right.rot.root[3])
        t.eq(left.move.root[1], -right.move.root[1])
    end)

    t.it("keeps the save families distinguishable by commitment", function()
        local function reach(id, dive)
            local pose = assert(action_pose.forOptions(opts(id, {
                dive = dive or 1,
                dive_dir = LEFT,
            })))
            return math.abs(pose.rot.root[3])
        end
        -- Spread stays compact, central corrects, dive commits, stretch is the
        -- full lunge and a tip reaches just past it. If any two of these swap,
        -- two different keeper decisions start looking like the same save.
        t.is_true(reach("keeper_spread") < reach("keeper_central"))
        t.is_true(reach("keeper_central") < reach("keeper_dive"))
        t.is_true(reach("keeper_dive") < reach("keeper_stretch"))
        t.is_true(reach("keeper_stretch") < reach("keeper_tip"))
    end)

    t.it("holds the full-stretch silhouette even early in the dive", function()
        local early = assert(action_pose.forOptions(opts("keeper_stretch", {
            dive = 0.05,
            dive_dir = LEFT,
        })))
        local late = assert(action_pose.forOptions(opts("keeper_stretch", {
            dive = 1,
            dive_dir = LEFT,
        })))
        -- Floored at 0.82: a stretch that eased in from nothing would read as a
        -- spread for the first half of the save.
        t.is_true(math.abs(early.rot.root[3]) > 0.8 * math.abs(late.rot.root[3]))
    end)

    t.it("plays a tip at full reach regardless of the dive timer", function()
        local a = assert(action_pose.forOptions(opts("keeper_tip", { dive = 0, dive_dir = LEFT })))
        local b = assert(action_pose.forOptions(opts("keeper_tip", { dive = 1, dive_dir = LEFT })))
        t.eq(a.rot.root[3], b.rot.root[3])
    end)

    t.it("needs a direction before it will throw a keeper anywhere", function()
        t.eq(action_pose.forOptions(opts("keeper_dive", { dive = 1 })), nil)
    end)

    t.it("takes a bicycle kick over backwards and off the ground", function()
        local pose = assert(action_pose.forOptions(opts("aerial_bicycle", {
            aerial = 1,
            aerial_style = "bicycle",
            aerial_jump = 1,
        })))
        -- Negative x pitches the head behind the hips. Positive would be a dive
        -- onto the face, which is not a bicycle kick.
        t.is_true(pose.rot.root[1] < 0, "a bicycle must rotate backwards")
        t.is_true(pose.move.root[2] > 0, "a bicycle must leave the ground")
    end)

    t.it("lifts a non-bicycle aerial without rotating it", function()
        local pose = assert(action_pose.forOptions(opts("aerial_action", {
            aerial = 1,
            aerial_style = "chest_control",
            aerial_jump = 1,
        })))
        t.eq(pose.rot.root, nil)
        t.is_true(pose.move.root[2] > 0)
    end)

    t.it("lifts further off a jump than off a standing reach", function()
        local function lift(jump)
            local pose = assert(action_pose.forOptions(opts("aerial_action", {
                aerial = 1,
                aerial_style = "leg_control",
                aerial_jump = jump,
            })))
            return pose.move.root[2]
        end
        t.is_true(lift(1) > lift(0))
    end)

    t.it("separates a knockback from a stagger", function()
        local knock = assert(action_pose.forOptions(opts("combat_knockback")))
        local stagger = assert(action_pose.forOptions(opts("combat_stagger")))
        -- Both go backwards; the knockback is driven off the feet and the
        -- stagger is a rocked-back beat, so they must not read as one thing.
        t.is_true(knock.rot.root[1] < 0 and stagger.rot.root[1] < 0)
        t.is_true(math.abs(knock.rot.root[1]) > math.abs(stagger.rot.root[1]))
        t.is_true(knock.move.root[2] > 0, "a knockback leaves the ground")
        t.is_true(stagger.move.root[2] < 0, "a stagger settles into it")
    end)

    t.it("tips a stumble away from the committed direction", function()
        local pose = assert(action_pose.forOptions(opts("stumble")))
        t.is_true(pose.rot.root[1] < 0)
        t.is_true(pose.move.root[3] < 0, "a stumble falls behind the challenge")
    end)

    t.it("gets a keeper up on the side they landed on", function()
        local left = assert(action_pose.forOptions(opts("keeper_get_up", { dive_dir = LEFT })))
        local right = assert(action_pose.forOptions(opts("keeper_get_up", { dive_dir = RIGHT })))
        t.eq(left.rot.root[3], -right.rot.root[3])
    end)
end)
