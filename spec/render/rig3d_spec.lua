local t = require("spec.support.runner")
local mat4 = require("core.mat4")
local skeleton = require("game.render.rig3d.skeleton")
local clips = require("game.render.rig3d.clips")
local masks = require("game.render.rig3d.masks")
local proportions = require("game.render.rig3d.proportions")
local themes = require("game.render.rig3d.themes")
local meshbuilder = require("game.render.rig3d.meshbuilder")
local body = require("game.render.rig3d.body")

-- These modules are required DIRECTLY, never through player_renderer_3d. That
-- is deliberate and it is the whole point of #340: player_renderer_3d.build()
-- calls love.graphics.newShader, which does not exist headless, so the rigged
-- renderer disables itself and every assertion made through it passes
-- vacuously. Requiring the modules themselves is what makes this real coverage.
--
-- None of them touch love.* except meshbuilder's `Builder:build()` (calls
-- love.graphics.newMesh) -- and nothing below calls that. `body.accumulate`
-- exists precisely so the entire generation path up to that one call is
-- reachable here.

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
        -- Looping clips only. KEEPER_SLING is a one-shot with a `fallback`, so
        -- its last keyframe is a follow-through and must NOT match its first.
        local looping =
            { clips.ORDER[1], clips.ORDER[2], clips.RUN, clips.CHARGE, clips.KEEPER_GATHER }
        for _, clip in ipairs(looping) do
            t.is_true(clip.loop == true, clip.name .. " is listed here but does not declare loop")
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
            -- position(3) + texcoord(2) + normal(3) + slot(1) + bone(1)
            -- + material(1) = 11 since #337 slice 2; the slot stays at 9.
            t.eq(#v, 11)
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

-- #337 slice 2: rigid GPU skinning. Every part of a character is folded into
-- ONE static mesh; the bone that drives a vertex and the material that shades
-- it travel IN the vertex instead of in a per-part uniform, so ~28 draw calls
-- per character become 1.
--
-- All of this is pure Lua up to the single love.graphics.newMesh call at the
-- very end, so the accumulation -- bone assignment, material assignment,
-- attach baking, the merge itself -- is fully covered headless.
t.describe("rig3d vertex format (#337 slice 2)", function()
    t.it("carries a bone index and a material alongside the palette slot", function()
        local names = {}
        for i, attribute in ipairs(meshbuilder.VERTEX_FORMAT) do
            names[i] = attribute[1]
            -- Floats, not integers: GLSL ES 1.00 (WebGL 1) has no integer
            -- vertex attributes, so every index crosses as a float.
            t.eq(attribute[2], "float", attribute[1] .. " must be a float attribute")
        end
        t.eq(
            table.concat(names, ","),
            "VertexPosition,VertexTexCoord,VertexNormal,"
                .. "VertexPaletteSlot,VertexBoneIndex,VertexMaterial"
        )
    end)

    t.it("emits one value per format component, in format order", function()
        local components = 0
        for _, attribute in ipairs(meshbuilder.VERTEX_FORMAT) do
            components = components + attribute[3]
        end
        t.eq(components, 11)

        local mb = meshbuilder.new()
        mb:triangle(nil, { 0, 0, 0 }, { 1, 0, 0 }, { 0, 1, 0 }, themes.SLOT_INDEX.skin)
        for _, v in ipairs(mb.verts) do
            t.eq(#v, components)
        end
    end)

    t.it("maps material names to the ids the shader branches on", function()
        t.eq(meshbuilder.materialIndex(nil), meshbuilder.MATERIAL.plain)
        t.eq(meshbuilder.materialIndex("metal"), meshbuilder.MATERIAL.metal)
        t.eq(meshbuilder.materialIndex("emissive"), meshbuilder.MATERIAL.emissive)
        -- The shader tests `v_material > 1.5` for emissive and `> 0.5` for
        -- metal, so the ordering of these three ids is load-bearing.
        t.is_true(
            meshbuilder.MATERIAL.plain < meshbuilder.MATERIAL.metal
                and meshbuilder.MATERIAL.metal < meshbuilder.MATERIAL.emissive,
            "material ids must stay ordered plain < metal < emissive"
        )
    end)

    t.it("rejects an unknown material rather than silently shading it plain", function()
        local ok = pcall(meshbuilder.materialIndex, "chrome")
        t.is_true(not ok, "an unknown material name must fail loud")
    end)
end)

t.describe("rig3d bone index contract (#337 slice 2)", function()
    t.it("indexes every bone densely from 0", function()
        local bones = skeleton.bones(RIG)
        local index = skeleton.boneIndex(RIG)
        t.eq(skeleton.boneCount(RIG), #bones)
        local seen = {}
        for i, bone in ipairs(bones) do
            t.eq(index[bone.name], i - 1, bone.name)
            t.is_true(not seen[index[bone.name]], "bone index must be unique: " .. bone.name)
            seen[index[bone.name]] = true
        end
    end)

    t.it("flattens the posed skeleton into three rows per bone, in index order", function()
        local rig = skeleton.new(RIG)
        local rows = skeleton.boneRows(rig)
        t.eq(#rows, skeleton.boneCount(RIG) * skeleton.ROWS_PER_BONE)
        t.eq(skeleton.ROWS_PER_BONE, 3)

        local index = skeleton.boneIndex(RIG)
        for name, i in pairs(index) do
            local m = rig.world[name]
            for r = 0, 2 do
                local row = rows[i * 3 + r + 1]
                t.eq(#row, 4, name .. " row " .. r)
                for c = 1, 4 do
                    t.near(row[c], m[r * 4 + c], 1e-12, name .. " row " .. r .. " col " .. c)
                end
            end
        end
    end)

    t.it("drops only the fourth row, which is always exactly (0, 0, 0, 1)", function()
        -- This is what buys the uniform budget: 3 rows per bone instead of 4.
        -- If a bone transform ever gained a projective or sheared component the
        -- shader would silently drop it, so the assumption is asserted here
        -- against a genuinely posed rig, not just the rest pose.
        local rig = skeleton.new(RIG)
        skeleton.apply(rig, {
            rot = {
                ["upper_arm.R"] = require("core.quat").fromEuler(math.rad(-70), 0.4, 0.2),
                hips = require("core.quat").fromEuler(0.3, -0.5, 0.1),
            },
            move = { hips = { 0.1, 0.2, -0.3 } },
        })
        for _, name in ipairs(rig.order) do
            local m = rig.world[name]
            for c = 1, 3 do
                t.near(m[12 + c], 0, 1e-12, name .. " fourth row must be (0,0,0,1)")
            end
            t.near(m[16], 1, 1e-12, name .. " fourth row must be (0,0,0,1)")
        end
    end)

    t.it("refills a reused row buffer rather than reallocating it", function()
        -- boneRows is called once per character per frame; the buffer is reused
        -- on purpose (see its own note on GC pressure at ten players).
        local rig = skeleton.new(RIG)
        local buffer = {}
        t.is_true(skeleton.boneRows(rig, buffer) == buffer, "must return the buffer it was given")
        local first_row = buffer[1]
        skeleton.apply(rig, {
            rot = { hips = require("core.quat").fromEuler(0.5, 0, 0) },
            move = {},
        })
        skeleton.boneRows(rig, buffer)
        t.is_true(buffer[1] == first_row, "row tables must be reused, not replaced")
        t.eq(#buffer, skeleton.boneCount(RIG) * skeleton.ROWS_PER_BONE)
    end)
end)

t.describe("rig3d vertex uniform budget (#337 slice 2)", function()
    t.it("fits the GLSL ES 1.00 guaranteed floor of 128 vertex uniform vectors", function()
        -- 26 bones as mat4 would be 104 vec4 + 12 palette + 12 matrix = 128,
        -- i.e. AT the floor and therefore unlinkable on a conforming minimum
        -- implementation. Three rows per bone is what makes it fit.
        local rows = skeleton.boneCount(RIG) * skeleton.ROWS_PER_BONE
        local used = rows + themes.SLOT_COUNT + 12
        t.is_true(used <= 128, "vertex uniform vectors used: " .. used)
        t.is_true(
            skeleton.boneCount(RIG) * 4 + themes.SLOT_COUNT + 12 > 128 - 1,
            "the mat4 form really is at or over the floor, so 3 rows is load-bearing"
        )
    end)
end)

t.describe("rig3d part merge (#337 slice 2)", function()
    local function tri(mb, slot, y)
        mb:triangle(nil, { 0, y, 0 }, { 1, y, 0 }, { 0, y + 1, 0 }, slot)
    end

    t.it("stamps each part's bone index and material onto every one of its vertices", function()
        local a, b = meshbuilder.new(), meshbuilder.new()
        tri(a, themes.SLOT_INDEX.skin, 0)
        tri(b, themes.SLOT_INDEX.plate, 0)
        local merged = meshbuilder.merge({
            { builder = a, bone = 4 },
            { builder = b, bone = 17, material = "emissive" },
        })
        t.eq(#merged.verts, 6)
        for i = 1, 3 do
            t.eq(merged.verts[i][10], 4, "bone index")
            t.eq(merged.verts[i][11], meshbuilder.MATERIAL.plain, "material")
            t.eq(merged.verts[i][9], themes.SLOT_INDEX.skin, "palette slot survives the merge")
        end
        for i = 4, 6 do
            t.eq(merged.verts[i][10], 17, "bone index")
            t.eq(merged.verts[i][11], meshbuilder.MATERIAL.emissive, "material")
            t.eq(merged.verts[i][9], themes.SLOT_INDEX.plate, "palette slot survives the merge")
        end
    end)

    t.it("preserves part order, so triangles rasterise in submission order", function()
        local a, b = meshbuilder.new(), meshbuilder.new()
        tri(a, themes.SLOT_INDEX.skin, 10)
        tri(b, themes.SLOT_INDEX.skin, 20)
        local merged = meshbuilder.merge({ { builder = a, bone = 0 }, { builder = b, bone = 1 } })
        t.near(merged.verts[1][2], 10, 1e-12)
        t.near(merged.verts[4][2], 20, 1e-12)
    end)

    t.it("bakes a part's attach transform into its vertices", function()
        -- The static per-part offset is what used to force a per-part model
        -- matrix. Once it lives in the vertex the bone matrix alone drives it.
        local plain, attached = meshbuilder.new(), meshbuilder.new()
        tri(plain, themes.SLOT_INDEX.skin, 0)
        tri(attached, themes.SLOT_INDEX.skin, 0)
        local shift = mat4.translation(5, -2, 3)
        local merged = meshbuilder.merge({
            { builder = plain, bone = 0 },
            { builder = attached, bone = 0, attach = shift },
        })
        for i = 1, 3 do
            local before, after = merged.verts[i], merged.verts[i + 3]
            t.near(after[1], before[1] + 5, 1e-12)
            t.near(after[2], before[2] - 2, 1e-12)
            t.near(after[3], before[3] + 3, 1e-12)
            -- A pure translation must leave the normal alone.
            for c = 6, 8 do
                t.near(after[c], before[c], 1e-12)
            end
        end
    end)

    t.it("rotates normals with an attach and keeps them unit length", function()
        local part = meshbuilder.new()
        tri(part, themes.SLOT_INDEX.skin, 0)
        local merged = meshbuilder.merge({
            { builder = part, bone = 0, attach = mat4.rotationX(math.rad(37)) },
        })
        for _, v in ipairs(merged.verts) do
            local len = math.sqrt(v[6] * v[6] + v[7] * v[7] + v[8] * v[8])
            t.near(len, 1, 1e-9, "normal must stay unit length through an attach")
        end
    end)

    t.it("rejects a part with no bone index", function()
        local part = meshbuilder.new()
        tri(part, themes.SLOT_INDEX.skin, 0)
        local ok = pcall(meshbuilder.merge, { { builder = part } })
        t.is_true(not ok, "merge must reject a part that names no bone")
    end)

    t.it("refuses to build a mesh from an unmerged part builder", function()
        -- A part is not a draw call any more. Enforced rather than documented,
        -- because a part builder's bone and material columns are zeros until
        -- merge stamps them -- building one directly would render every part
        -- on the root bone.
        local part = meshbuilder.new()
        tri(part, themes.SLOT_INDEX.skin, 0)
        local ok, err = pcall(part.build, part)
        t.is_true(not ok, "an unmerged builder must not build")
        t.is_true(
            tostring(err):find("merge", 1, true) ~= nil,
            "the error should point at merge: " .. tostring(err)
        )
    end)
end)

t.describe("rig3d character accumulation (#337 slice 2)", function()
    local THEME, FIGURE = themes.LIST[1], themes.FIGURES[1]

    t.it("merges every part into one builder without losing a triangle", function()
        local merged, parts = body.accumulate(RIG, THEME, FIGURE)
        t.is_true(#parts > 1, "the character really is built from many parts: " .. #parts)
        local sum = 0
        for _, part in ipairs(parts) do
            sum = sum + part.builder:triangleCount()
        end
        t.eq(merged:triangleCount(), sum, "the merge must lose nothing and invent nothing")
        t.is_true(sum > 0, "the character must have geometry")
    end)

    t.it("gives every emitted vertex a bone index inside the skeleton", function()
        local merged = body.accumulate(RIG, THEME, FIGURE)
        local count = skeleton.boneCount(RIG)
        -- An out-of-range index reads past the end of the bone uniform array,
        -- which is undefined behaviour rather than a visible mistake.
        local lowest, highest = math.huge, -math.huge
        for _, v in ipairs(merged.verts) do
            local bone = v[10]
            t.is_true(
                bone == math.floor(bone) and bone >= 0 and bone < count,
                "bone index out of [0, " .. (count - 1) .. "]: " .. tostring(bone)
            )
            lowest, highest = math.min(lowest, bone), math.max(highest, bone)
        end
        t.is_true(highest > lowest, "the character must actually use more than one bone")
    end)

    t.it("keeps every part on a bone the skeleton declares", function()
        local _, parts = body.accumulate(RIG, THEME, FIGURE)
        local index = skeleton.boneIndex(RIG)
        for _, part in ipairs(parts) do
            t.eq(index[part.bone_name], part.bone, part.bone_name)
        end
    end)

    t.it("resolves each part's material onto its vertices", function()
        local merged, parts = body.accumulate(RIG, THEME, FIGURE)
        local at, seen = 1, {}
        for _, part in ipairs(parts) do
            local expected = meshbuilder.materialIndex(part.material)
            seen[expected] = (seen[expected] or 0) + 1
            for _ = 1, #part.builder.verts do
                t.eq(merged.verts[at][11], expected, part.bone_name .. " material")
                at = at + 1
            end
        end
        t.eq(at - 1, #merged.verts, "every merged vertex must belong to a part")
        -- Medieval Fantasy is plate armour over bare skin: it must produce both
        -- families, or "material is per vertex" would be untested in practice.
        t.is_true((seen[meshbuilder.MATERIAL.plain] or 0) > 0, "expected plain parts")
        t.is_true((seen[meshbuilder.MATERIAL.metal] or 0) > 0, "expected metal parts")
    end)

    t.it("mixes materials inside one mesh for a theme that glows", function()
        -- Galactic Sci-Fi has emissive seams and an energy blade. Before slice 2
        -- those forced their own draw calls; now all three families share one
        -- mesh, which is the difference between ~3 draws per character and 1.
        local scifi = themes.byKey("scifi")
        local merged = body.accumulate(RIG, scifi, FIGURE)
        local seen = {}
        for _, v in ipairs(merged.verts) do
            seen[v[11]] = true
        end
        for _, name in ipairs({ "plain", "metal", "emissive" }) do
            t.is_true(seen[meshbuilder.MATERIAL[name]], "expected " .. name .. " vertices")
        end
    end)

    t.it("builds every theme x figure pair without a bad bone or material", function()
        for _, theme in ipairs(themes.LIST) do
            for _, figure in ipairs(themes.FIGURES) do
                local merged, parts = body.accumulate(RIG, theme, figure)
                t.is_true(
                    merged:triangleCount() > 0,
                    theme.key .. "/" .. figure.key .. " produced no geometry"
                )
                t.is_true(#parts > 0, theme.key .. "/" .. figure.key .. " produced no parts")
            end
        end
    end)
end)

-- #391: the shader SOURCE is testable headless even though the shader is not.
-- `renderer.load` calls love.graphics.newShader, which does not exist in this
-- runner, so the stage placement below can only be checked by reading the GLSL.
-- It has to be checked somewhere: getting it wrong costs nothing on desktop GL
-- or in Chrome and costs every Firefox player the entire rigged renderer.
local renderer = require("game.render.rig3d.renderer")

---@class Rig3dGlslDeclaration
---@field name string
---@field stage string?  -- "VERTEX", "PIXEL", or nil for outside every stage block

---Walk the GLSL and record which stage block each declaration sits in.
---@param source string
---@return table<string, string?> uniforms
---@return table<string, string?> varyings
local function declarations_by_stage(source)
    local stack = {}
    local uniforms = {}
    local varyings = {}
    for line in (source .. "\n"):gmatch("(.-)\n") do
        local opened = line:match("^%s*#ifdef%s+([%u_]+)")
        if opened then
            stack[#stack + 1] = opened
        elseif line:match("^%s*#endif") then
            assert(#stack > 0, "unbalanced #endif in the rig3d shader")
            stack[#stack] = nil
        else
            local stage = stack[#stack]
            local uniform = line:match("^%s*uniform%s+[%w_]+%s+([%w_]+)")
            if uniform then
                uniforms[uniform] = stage or false
            end
            local varying = line:match("^%s*varying%s+[%w_]+%s+([%w_]+)")
            if varying then
                varyings[varying] = stage or false
            end
        end
    end
    assert(#stack == 0, "unbalanced #ifdef in the rig3d shader")
    return uniforms, varyings
end

t.describe("rig3d shader source", function()
    local SOURCE = renderer.shaderSource(12, 78)

    t.it("bakes the array sizes it is given into the GLSL", function()
        t.is_true(SOURCE:find("u_palette%[12%]") ~= nil, "palette size was not substituted")
        t.is_true(SOURCE:find("u_bones%[78%]") ~= nil, "bone row count was not substituted")
    end)

    t.it("declares every uniform inside a stage block", function()
        -- A uniform outside `#ifdef VERTEX` / `#ifdef PIXEL` is compiled into
        -- BOTH stages, where LÖVE's headers give it different default float
        -- precision. GLSL ES 1.00 requires cross-stage uniform precision to
        -- match, and Firefox refuses to link when it does not:
        -- `Uniform 'u_palette' is not linkable between attached shaders`.
        local uniforms = declarations_by_stage(SOURCE)
        local names = {}
        for name in pairs(uniforms) do
            names[#names + 1] = name
        end
        table.sort(names)
        t.is_true(#names >= 8, "expected the shader's uniforms to be found, got " .. #names)
        for _, name in ipairs(names) do
            t.is_true(
                uniforms[name] == "VERTEX" or uniforms[name] == "PIXEL",
                name .. " is declared outside every stage block, so it compiles into both"
            )
        end
    end)

    t.it("puts each uniform in the one stage that reads it", function()
        local uniforms = declarations_by_stage(SOURCE)
        for _, name in ipairs({ "u_model", "u_view", "u_proj", "u_palette", "u_bones" }) do
            t.eq(uniforms[name], "VERTEX", name .. " is read by position(), so it is vertex-stage")
        end
        for _, name in ipairs({ "u_light_dir", "u_cam_pos", "u_unlit" }) do
            t.eq(uniforms[name], "PIXEL", name .. " is read by effect(), so it is pixel-stage")
        end
    end)

    t.it("keeps every varying outside the stage blocks", function()
        -- The mirror image of the rule above: a varying has to be visible to
        -- both stages to carry anything between them, and varyings are exempt
        -- from the cross-stage precision rule that moved the uniforms.
        local _, varyings = declarations_by_stage(SOURCE)
        for _, name in ipairs({ "v_normal", "v_world", "v_slot_color", "v_material" }) do
            t.eq(varyings[name], false, name .. " must be declared for both stages")
        end
    end)
end)

t.describe("rig3d pose LOD policy (#394)", function()
    local pose_lod = require("game.render.rig3d.pose_lod")

    -- A ten-player whole-pitch frame: nobody controlled, plain gaits, small on
    -- screen. This is the case the LOD exists for.
    local SMALL = pose_lod.FULL_RATE_HEIGHT_PX - 1
    ---@param extra table|nil
    ---@return table
    local function opts_with(extra)
        local opts = { pose = { id = "locomotion" } }
        for key, value in pairs(extra or {}) do
            opts[key] = value
        end
        return opts
    end

    t.it("holds a small character in the plain gait family", function()
        t.eq(pose_lod.interval(opts_with(), SMALL), pose_lod.REDUCED_INTERVAL)
        for _, id in ipairs({ "contain", "run_telegraph", "fatigue", "keeper_shuffle" }) do
            t.eq(
                pose_lod.interval({ pose = { id = id } }, SMALL),
                pose_lod.REDUCED_INTERVAL,
                id .. " is a plain gait and may be held"
            )
        end
        -- No pose id at all is the plain gait fallback, not an unknown.
        t.eq(pose_lod.interval({}, SMALL), pose_lod.REDUCED_INTERVAL)
    end)

    t.it("keeps a close-up character at full rate whatever it is doing", function()
        t.eq(pose_lod.interval(opts_with(), pose_lod.FULL_RATE_HEIGHT_PX + 1), 1)
    end)

    t.it("keeps the controlled player at full rate", function()
        t.eq(pose_lod.interval(opts_with({ controlled = true }), SMALL), 1)
    end)

    t.it("keeps every simulation-timer-driven action at full rate", function()
        for _, active in ipairs({
            { dive = 0.4 },
            { aerial = 0.2 },
            { throw = 0.9 },
            { windup = 0.5 },
            { holding = true },
        }) do
            local opts = opts_with(active)
            t.eq(pose_lod.interval(opts, SMALL), 1, "active option must force full rate")
        end
    end)

    t.it("treats any pose id outside the gait family as full rate", function()
        -- Includes ids that do not exist yet: an unlisted id must never be
        -- degraded by accident, so the whitelist fails toward full rate.
        for _, id in ipairs({ "keeper_stretch", "aerial_bicycle", "combat_windup", "no_such" }) do
            t.eq(pose_lod.interval({ pose = { id = id } }, SMALL), 1, id)
        end
    end)

    t.it("schedules refreshes deterministically from tick and stagger alone", function()
        -- Interval 1 is always due; interval 2 alternates per character and
        -- adjacent staggers land on opposite frames, so ten held characters
        -- split five/five instead of ten/zero.
        for tick = 0, 5 do
            t.is_true(pose_lod.due(tick, 0, 1), "full rate is always due")
            t.eq(pose_lod.due(tick, 0, 2), tick % 2 == 0)
            t.eq(
                pose_lod.due(tick, 0, 2),
                not pose_lod.due(tick, 1, 2),
                "adjacent staggers must refresh on opposite frames"
            )
            -- Pure: the same inputs answer the same way twice.
            t.eq(pose_lod.due(tick, 3, 2), pose_lod.due(tick, 3, 2))
        end
    end)
end)
