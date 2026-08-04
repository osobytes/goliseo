local t = require("spec.support.runner")
local mat4 = require("core.mat4")
local skeleton = require("game.render.rig3d.skeleton")
local clips = require("game.render.rig3d.clips")
local masks = require("game.render.rig3d.masks")
local proportions = require("game.render.rig3d.proportions")
local themes = require("game.render.rig3d.themes")
local meshbuilder = require("game.render.rig3d.meshbuilder")

-- None of these modules touch love.* except meshbuilder's `Builder:build()`
-- (calls love.graphics.newMesh) -- and nothing below calls that, so the whole
-- file still runs in the headless tier.

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

-- #337 slice 1: colour is a palette slot index baked per vertex, resolved
-- against a team into a flat RGBA array at "palette-upload time" rather than
-- per vertex. themes.resolvedPalette and SLOT_INDEX are pure Lua -- no
-- love.graphics -- so this is real coverage of the changed logic in the
-- headless tier, not just the geometry around it.
t.describe("rig3d palette slots (#337)", function()
    t.it("has exactly twelve canonical slots", function()
        t.eq(#themes.SLOTS, 12)
        t.eq(themes.SLOT_COUNT, 12)
    end)

    t.it("SLOT_INDEX is a dense 0-based reindex of SLOTS", function()
        for i, name in ipairs(themes.SLOTS) do
            t.eq(themes.SLOT_INDEX[name], i - 1, name)
        end
    end)

    t.it('resolves the "team" sentinel to the team\'s main colour', function()
        -- Medieval's `cloth` slot is wired to "team" -- the surcoat is the
        -- dominant ownership surface.
        local theme = themes.byKey("medieval")
        t.eq(theme.color.cloth, "team")
        for _, team in ipairs(themes.TEAMS) do
            local palette = themes.resolvedPalette(theme, team)
            local cloth = palette[themes.SLOT_INDEX.cloth + 1]
            for i = 1, 3 do
                t.near(cloth[i], team.main[i], 1e-9, team.key .. " cloth[" .. i .. "]")
            end
        end
    end)

    t.it('resolves the "trim" sentinel to the team\'s trim colour', function()
        -- Every theme wires `crest` to "trim" -- the readable secondary used
        -- for crests, seams and edge accents.
        local theme = themes.byKey("medieval")
        t.eq(theme.color.crest, "trim")
        for _, team in ipairs(themes.TEAMS) do
            local palette = themes.resolvedPalette(theme, team)
            local crest = palette[themes.SLOT_INDEX.crest + 1]
            for i = 1, 3 do
                t.near(crest[i], team.trim[i], 1e-9, team.key .. " crest[" .. i .. "]")
            end
        end
    end)

    t.it("falls `limbs` back to `skin` when a theme leaves it unset", function()
        local theme = themes.byKey("medieval")
        t.is_true(theme.color.limbs == nil, "fixture assumption: medieval leaves limbs unset")
        local palette = themes.resolvedPalette(theme, themes.TEAMS[1])
        local limbs = palette[themes.SLOT_INDEX.limbs + 1]
        local skin = palette[themes.SLOT_INDEX.skin + 1]
        for i = 1, 4 do
            t.near(limbs[i], skin[i], 1e-9, "limbs[" .. i .. "]")
        end
    end)

    t.it("resolves a literal colour slot to exactly the authored value", function()
        local theme = themes.byKey("scifi")
        local expected = theme.color.plate_dark
        t.is_true(
            type(expected) == "table",
            "fixture assumption: plate_dark is a literal, not a sentinel"
        )
        local palette = themes.resolvedPalette(theme, themes.TEAMS[1])
        local plate_dark = palette[themes.SLOT_INDEX.plate_dark + 1]
        for i = 1, 3 do
            t.near(plate_dark[i], expected[i], 1e-9, "plate_dark[" .. i .. "]")
        end
    end)

    t.it("never varies the constant slots (ink, sclera) by theme or team", function()
        local a = themes.resolvedPalette(themes.byKey("medieval"), themes.TEAMS[1])
        local b = themes.resolvedPalette(themes.byKey("toybox"), themes.TEAMS[2])
        for _, name in ipairs({ "ink", "sclera" }) do
            local idx = themes.SLOT_INDEX[name] + 1
            for i = 1, 4 do
                t.near(a[idx][i], b[idx][i], 1e-9, name .. "[" .. i .. "]")
            end
        end
    end)

    t.it("resolves every theme x team pair to exactly SLOT_COUNT RGBA entries", function()
        for _, theme in ipairs(themes.LIST) do
            for _, team in ipairs(themes.TEAMS) do
                local palette = themes.resolvedPalette(theme, team)
                t.eq(#palette, themes.SLOT_COUNT, theme.key .. "/" .. team.key)
                for i, rgba in ipairs(palette) do
                    t.eq(#rgba, 4, theme.key .. "/" .. team.key .. " slot " .. i)
                end
            end
        end
    end)

    t.it(
        "fails loud instead of silently defaulting when a theme leaves a slot unauthored",
        function()
            -- A theme missing `crest` entirely, with no SLOT_FALLBACK entry for
            -- it, must not render an inert black placeholder -- #338 lands new
            -- theme content next, which is exactly the shape of mistake this
            -- guards against (AGENTS.md #7: assert on invariant violations).
            local broken_theme = {
                key = "broken-fixture",
                color = {
                    skin = { 0.5, 0.5, 0.5 },
                    cloth = { 0.5, 0.5, 0.5 },
                    plate = { 0.5, 0.5, 0.5 },
                    plate_dark = { 0.5, 0.5, 0.5 },
                    accent = { 0.5, 0.5, 0.5 },
                    strap = { 0.5, 0.5, 0.5 },
                    -- crest deliberately omitted
                    joint = { 0.5, 0.5, 0.5 },
                    limbs = { 0.5, 0.5, 0.5 },
                    seam = { 0.5, 0.5, 0.5 },
                },
            }
            local ok, err = pcall(themes.resolvedPalette, broken_theme, themes.TEAMS[1])
            t.is_true(not ok, "resolvedPalette must reject a theme missing an authored slot")
            t.is_true(
                tostring(err):find("crest", 1, true) ~= nil,
                "error should name the missing slot: " .. tostring(err)
            )
        end
    )
end)

-- meshbuilder no longer bakes a literal colour: every vertex carries a slot
-- index instead, and rejects anything that isn't one -- coverage for the
-- assertion the #337 review specifically asked to see exercised.
t.describe("rig3d meshbuilder palette-slot vertices (#337)", function()
    t.it("bakes a numeric slot index per vertex instead of a literal colour", function()
        local mb = meshbuilder.new()
        mb:triangle(nil, { 0, 0, 0 }, { 1, 0, 0 }, { 0, 1, 0 }, themes.SLOT_INDEX.skin)
        t.eq(#mb.verts, 3)
        for _, v in ipairs(mb.verts) do
            -- position(3) + texcoord(2) + normal(3) + slot(1) = 9.
            t.eq(#v, 9)
            t.eq(v[9], themes.SLOT_INDEX.skin)
        end
    end)

    t.it("quad() propagates the same slot to both triangles", function()
        local mb = meshbuilder.new()
        mb:quad(nil, { 0, 0, 0 }, { 1, 0, 0 }, { 1, 1, 0 }, { 0, 1, 0 }, themes.SLOT_INDEX.plate)
        t.eq(#mb.verts, 6)
        for _, v in ipairs(mb.verts) do
            t.eq(v[9], themes.SLOT_INDEX.plate)
        end
    end)

    t.it("rejects a literal colour table now that vertices carry a slot index", function()
        local mb = meshbuilder.new()
        local ok = pcall(
            mb.triangle,
            mb,
            nil,
            { 0, 0, 0 },
            { 1, 0, 0 },
            { 0, 1, 0 },
            { 1, 0, 0, 1 }
        )
        t.is_true(not ok, "triangle() must reject a colour table, not silently accept it as a slot")
    end)

    t.it("rejects a nil slot", function()
        local mb = meshbuilder.new()
        local ok = pcall(mb.triangle, mb, nil, { 0, 0, 0 }, { 1, 0, 0 }, { 0, 1, 0 }, nil)
        t.is_true(not ok, "triangle() must reject a nil slot")
    end)
end)
