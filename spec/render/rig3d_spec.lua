local t = require("spec.support.runner")
local mat4 = require("core.mat4")
local skeleton = require("game.render.rig3d.skeleton")
local clips = require("game.render.rig3d.clips")
local masks = require("game.render.rig3d.masks")
local proportions = require("game.render.rig3d.proportions")

-- None of these modules touch love.*, so they run in the headless tier.

local RIG = proportions.RIG_MEDIUM

t.describe("rig3d skeleton", function()
    t.it("lists every bone parent-before-child", function()
        local seen = {}
        for _, bone in ipairs(skeleton.bones(RIG)) do
            if bone.parent then
                t.is_true(seen[bone.parent] == true, bone.name .. " precedes its parent")
            end
            seen[bone.name] = true
        end
    end)

    t.it("stands the rig on the ground with feet below the hips", function()
        local rig = skeleton.new(RIG)
        local _, hips_y = skeleton.jointPosition(rig, "hips")
        local _, foot_y = skeleton.jointPosition(rig, "foot.R")
        t.is_true(foot_y < hips_y, "foot should sit below the hips")
        t.is_true(foot_y >= 0, "foot should not start below the ground, got " .. foot_y)
    end)

    t.it("mirrors left and right limbs about the centre line", function()
        local rig = skeleton.new(RIG)
        local rx = skeleton.jointPosition(rig, "hand.R")
        local lx = skeleton.jointPosition(rig, "hand.L")
        t.near(rx, -lx, 1e-9)
    end)

    t.it("applies a bone's rest rotation even with an empty pose", function()
        -- socket_hand carries a 145 degree rest roll; a weapon hangs off it, so
        -- losing the rest rotation would point every blade the wrong way.
        local rig = skeleton.new(RIG)
        local socket = rig.world["socket_hand.R"]
        local hand = rig.world["hand.R"]
        local sx, sy, sz = mat4.transformDirection(socket, 0, 1, 0)
        local hx, hy, hz = mat4.transformDirection(hand, 0, 1, 0)
        local dot = sx * hx + sy * hy + sz * hz
        t.is_true(dot < 0.5, "socket should be rolled well away from the hand axis")
    end)

    t.it("propagates a parent rotation to its children", function()
        local rig = skeleton.new(RIG)
        local before = select(2, skeleton.jointPosition(rig, "hand.R"))
        skeleton.apply(rig, {
            rot = { ["upper_arm.R"] = require("core.quat").fromEuler(math.rad(-90), 0, 0) },
            move = {},
        })
        local after = select(2, skeleton.jointPosition(rig, "hand.R"))
        t.is_true(after > before + 0.05, "raising the upper arm must lift the hand")
    end)
end)

t.describe("rig3d clips", function()
    t.it("every clip loops: the last keyframe matches the first", function()
        for _, clip in ipairs({ clips.ORDER[1], clips.ORDER[2], clips.RUN, clips.CHARGE }) do
            local first = clips.sample(clip, 0)
            local last = clips.sample(clip, clip.duration - 1e-6)
            for bone, q in pairs(first.rot) do
                local other = last.rot[bone]
                t.is_true(other ~= nil, clip.name .. ": " .. bone .. " missing at loop point")
                for i = 1, 4 do
                    t.near(q[i], other[i], 1e-3)
                end
            end
        end
    end)

    t.it("sampling wraps rather than running off the end", function()
        local walk = clips.ORDER[2]
        local a = clips.sample(walk, 0.1)
        local b = clips.sample(walk, walk.duration + 0.1)
        for bone, q in pairs(a.rot) do
            for i = 1, 4 do
                t.near(q[i], b.rot[bone][i], 1e-9)
            end
        end
    end)

    t.it("layer leaves bones outside the mask untouched", function()
        local base = clips.sample(clips.ORDER[2], 0.2)
        local overlay = clips.sample(clips.CHARGE, 0.1)
        local out = clips.layer(base, overlay, masks.UPPER_BODY, 1)
        -- Legs are not in UPPER_BODY, so they must survive verbatim.
        for _, bone in ipairs({ "thigh.R", "shin.R", "foot.L", "toe.L" }) do
            if base.rot[bone] then
                for i = 1, 4 do
                    t.near(out.rot[bone][i], base.rot[bone][i], 1e-12)
                end
            end
        end
    end)

    t.it("layer at weight 0 is the base pose", function()
        local base = clips.sample(clips.ORDER[2], 0.3)
        local overlay = clips.sample(clips.CHARGE, 0.2)
        local out = clips.layer(base, overlay, masks.UPPER_BODY, 0)
        for bone, q in pairs(base.rot) do
            for i = 1, 4 do
                t.near(out.rot[bone][i], q[i], 1e-9)
            end
        end
    end)

    t.it("layer at weight 1 takes the overlay on masked bones", function()
        local base = clips.sample(clips.ORDER[2], 0.3)
        local overlay = clips.sample(clips.CHARGE, 0.2)
        local out = clips.layer(base, overlay, masks.UPPER_BODY, 1)
        for bone, q in pairs(overlay.rot) do
            if masks.UPPER_BODY[bone] then
                for i = 1, 4 do
                    t.near(out.rot[bone][i], q[i], 1e-9)
                end
            end
        end
    end)

    t.it("masks include the sockets attached to the hands they cover", function()
        -- A socket left out of the mask keeps the base layer's transform while
        -- the arm follows the overlay, and the weapon detaches from the fist.
        for _, mask in ipairs({ masks.UPPER_BODY, masks.ARMS }) do
            t.is_true(mask["hand.R"] and mask["socket_hand.R"], "socket must accompany its hand")
            t.is_true(mask["hand.L"] and mask["socket_hand.L"], "socket must accompany its hand")
        end
    end)
end)
