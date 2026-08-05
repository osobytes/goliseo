-- Committed evidence for #337: palette-slot vertex colours (slice 1) and rigid
-- GPU skinning (slice 2).
--
-- This is the visual half of the proof that pixel-diffing rig3d renders by
-- hand (in a scratch worktree, against a scratch script nobody else can run)
-- is not: a snapshot gate anyone can re-run a year from now, in the same
-- opt-in tier-4 style as the keeper/outfield pose snapshots (needs a
-- display, checked-in baselines, `write` to intentionally refresh).
--
-- It proves two separate things:
--   1. VISUAL: the same character, rendered with two different resolved
--      palettes, matches its checked-in baseline. A colour regression in
--      themes.lua, meshbuilder.lua or the renderer.lua shader shows up here.
--
--      Slice 2 leans on this harder than slice 1 did, and this is the
--      strongest check available for it: moving skinning onto the GPU must
--      change HOW pixels are submitted, never WHICH pixels come out. The
--      baselines were committed by slice 1 and are deliberately NOT
--      regenerated -- an unchanged image against an unchanged baseline is the
--      claim.
--
--   2. STRUCTURAL: building one theme allocates exactly ONE GPU mesh, and
--      resolving N team palettes allocates none. Both halves of the #337
--      payoff -- one draw call per character, and a cosmetic variant that
--      costs zero extra meshes -- are plain assertions rather than baselines,
--      so they go red the moment something reintroduces a per-part mesh or a
--      per-team build.

local mat4 = require("core.mat4")
local proportions = require("game.render.rig3d.proportions")
local skeleton = require("game.render.rig3d.skeleton")
local themes = require("game.render.rig3d.themes")
local body = require("game.render.rig3d.body")
local renderer = require("game.render.rig3d.renderer")

---@class Rig3dPaletteSnapshotsModule
local snapshots = {}

local WIDTH = 480
local HEIGHT = 480
local BASELINE_DIR = "spec/visual/baselines"

-- Slice 2 moved two multiplications that used to happen in Lua doubles into the
-- shader in float32: a part's `attach` is now baked into its vertices at build
-- time instead of being folded into a per-part model matrix, and the character
-- yaw is now applied after the bone matrix instead of before it. Same geometry,
-- different order of operations, so a last-bit float difference on the parts
-- that carry an attach (the shield, the holstered blaster) was expected, and
-- with it a silhouette pixel landing one level either side of an edge.
--
-- It did not happen. The measured maximum over both committed baselines is
-- 0/255 across all four channels and 0 of 230400 pixels differ, so this stays
-- pinned at EXACT equality rather than being widened for a deviation that does
-- not exist. The comparison is per-channel rather than PNG-byte equality only
-- because it can then report the measured deviation on every run, pass or fail
-- -- which is what makes a future widening visible rather than quiet.
local MAX_CHANNEL_DEVIATION = 0
local MAX_DIFFERING_FRACTION = 0

-- One theme, both teams -- the exact scenario the #337 payoff is about: one
-- mesh build, two genuinely different colourways.
local SCENARIOS = {
    { id = "rig3d_palette_crimson", team_index = 1 },
    { id = "rig3d_palette_azure", team_index = 2 },
}

---@return table  -- body.build's { mesh, part_count, triangle_count }
---@return table  -- the rig proportions it was built against
local function character()
    local rig = proportions.RIG_MEDIUM
    local theme = themes.LIST[1]
    local figure = themes.FIGURES[1]
    return body.build(rig, theme, figure), rig
end

---@param team table  -- an entry from themes.TEAMS
---@return love.ImageData
local function render(team)
    local built, rig = character()
    local theme = themes.LIST[1]
    local palette = themes.resolvedPalette(theme, team)
    local rig_instance = skeleton.new(rig)

    renderer.load(themes.SLOT_COUNT, skeleton.boneCount(rig))
    local cam = renderer.characterCamera(240, 420, 150, WIDTH, HEIGHT, math.rad(17))

    -- renderer.characterCamera bakes its projection assuming the scene lands
    -- on a Canvas that gets composited onto the backbuffer afterwards (see
    -- its own comment: LÖVE auto-corrects a Canvas's Y-flip when the Canvas
    -- itself is later drawn, but this shader bypasses LÖVE's transform, so it
    -- pre-flips for that later step). The match always draws through
    -- bloom.draw, which does exactly that composite; this harness has no
    -- bloom pass, so it does the same composite by hand -- draw the scene
    -- canvas onto a second canvas with a vertical flip -- otherwise the
    -- checked-in baseline would show the character upside down.
    local scene = love.graphics.newCanvas(WIDTH, HEIGHT)
    love.graphics.setCanvas({ scene, depthstencil = true })
    love.graphics.clear(0.10, 0.10, 0.12, 1)
    renderer.beginPass(cam, palette)
    renderer.draw(built.mesh, mat4.rotationY(0), skeleton.boneRows(rig_instance))
    renderer.endPass()
    love.graphics.setCanvas()

    local composed = love.graphics.newCanvas(WIDTH, HEIGHT)
    love.graphics.setCanvas(composed)
    love.graphics.clear(0.10, 0.10, 0.12, 1)
    love.graphics.setColor(1, 1, 1, 1)
    love.graphics.draw(scene, 0, HEIGHT, 0, 1, -1)
    love.graphics.setCanvas()
    return composed:newImageData()
end

---@param path string
---@return string?
local function read_file(path)
    local file = io.open(path, "rb")
    if not file then
        return nil
    end
    local value = file:read("*a")
    file:close()
    return value
end

---@param path string
---@param value string
local function write_file(path, value)
    local file = assert(io.open(path, "wb"))
    assert(file:write(value))
    assert(file:close())
end

-- Decodes a committed PNG into ImageData without going near the filesystem
-- sandbox: baselines live in the source tree, which love.filesystem cannot
-- read, so the bytes come in through io and are wrapped as FileData.
---@param bytes string
---@return love.ImageData
local function decode_png(bytes)
    return love.image.newImageData(love.filesystem.newFileData(bytes, "baseline.png"))
end

-- Per-channel comparison of two renders.
---@param actual love.ImageData
---@param expected love.ImageData
---@return number max_deviation   -- 0..255, the largest single-channel difference
---@return number differing       -- pixels differing in any channel at all
---@return number total
local function compare(actual, expected)
    local w, h = actual:getDimensions()
    local ew, eh = expected:getDimensions()
    if w ~= ew or h ~= eh then
        return 255, w * h, w * h
    end
    local abs, max = math.abs, math.max
    local max_deviation, differing = 0, 0
    for y = 0, h - 1 do
        for x = 0, w - 1 do
            local ar, ag, ab, aa = actual:getPixel(x, y)
            local er, eg, eb, ea = expected:getPixel(x, y)
            local worst = max(abs(ar - er), abs(ag - eg), abs(ab - eb), abs(aa - ea)) * 255
            if worst > 0 then
                differing = differing + 1
                if worst > max_deviation then
                    max_deviation = worst
                end
            end
        end
    end
    -- The buffers are 8-bit, so any real difference is an integer number of
    -- levels; rounding here only undoes float division by 255.
    return math.floor(max_deviation + 0.5), differing, w * h
end

-- What a whole character must cost on the GPU. This is the result under test,
-- so it is pinned rather than merely observed: slice 1 allocated one mesh per
-- part (28 of them, 28 draw calls) and slice 2 collapses that to exactly one
-- mesh drawn once, with the parts folded in and the bone each one rides carried
-- per vertex instead of per draw.
local EXPECTED_MESHES_PER_CHARACTER = 1

-- STRUCTURAL half of the proof, in two parts:
--   * one theme build allocates exactly EXPECTED_MESHES_PER_CHARACTER mesh --
--     the slice 2 collapse, which is the quantity the whole issue is about; and
--   * resolving additional team palettes allocates none -- the slice 1 payoff,
--     which slice 2 must not regress.
-- Mirrors the exact wiring player_renderer_3d.build() uses (one body.build, one
-- themes.resolvedPalette per team), so a regression back to a per-part mesh or a
-- per-team build is caught here rather than merely noticed in a benchmark.
---@return boolean ok
---@return string report
local function checkCharacterMeshBudget()
    local rig = proportions.RIG_MEDIUM
    local theme = themes.LIST[1]
    local figure = themes.FIGURES[1]

    local mesh_count = 0
    local real_new_mesh = love.graphics.newMesh
    love.graphics.newMesh = function(...)
        mesh_count = mesh_count + 1
        return real_new_mesh(...)
    end

    local part_count = 0
    local ok, err = pcall(function()
        local built = body.build(rig, theme, figure)
        part_count = built.part_count
        local after_build = mesh_count
        assert(
            after_build == EXPECTED_MESHES_PER_CHARACTER,
            string.format(
                "one character must be %d mesh(es), got %d for %d parts -- a part is not a draw "
                    .. "call any more (#337 slice 2)",
                EXPECTED_MESHES_PER_CHARACTER,
                after_build,
                part_count
            )
        )
        for _, team in ipairs(themes.TEAMS) do
            themes.resolvedPalette(theme, team)
        end
        assert(
            mesh_count == after_build,
            string.format(
                "resolving %d team palettes allocated %d extra mesh(es) -- palette must not touch the GPU",
                #themes.TEAMS,
                mesh_count - after_build
            )
        )
    end)
    love.graphics.newMesh = real_new_mesh

    if not ok then
        return false, "character mesh budget check FAILED: " .. tostring(err)
    end
    return true,
        string.format(
            "character mesh budget OK: %d parts merged, %d team palettes, %d total newMesh() calls",
            part_count,
            #themes.TEAMS,
            mesh_count
        )
end

---@param id string
---@return string  -- absolute path to a scenario's committed baseline
local function baseline_path(id)
    return love.filesystem.getSource() .. "/" .. BASELINE_DIR .. "/" .. id .. ".png"
end

-- Proves the comparison in `run("check")` actually rejects a mismatch, per
-- AGENTS.md #9 ("every gate must come with a demonstration it can go red").
-- Renders both scenarios fresh and checks each against the OTHER scenario's
-- baseline -- a real colour swap, not a synthetic byte flip, so this exercises
-- the exact tolerance check `run` uses on a case guaranteed to differ (crimson's
-- cloth is nowhere near azure's).
--
-- The tolerance makes this MORE necessary, not less: a bound nobody has shown
-- can reject anything is indistinguishable from no check at all.
---@return boolean ok
---@return string report
local function selfTest()
    local rendered, baselines = {}, {}
    for _, scenario in ipairs(SCENARIOS) do
        local team = themes.TEAMS[scenario.team_index]
        rendered[scenario.id] = render(team)
        local bytes = read_file(baseline_path(scenario.id))
        if not bytes then
            return false,
                "self-test needs baselines; run `write` first (missing " .. baseline_path(
                    scenario.id
                ) .. ")"
        end
        baselines[scenario.id] = decode_png(bytes)
    end

    local reports = {}

    -- 1. Each render must match ITS OWN baseline within tolerance (sanity: the
    --    fixture is wired up and the baselines are the ones under test).
    for _, scenario in ipairs(SCENARIOS) do
        local deviation, differing, total = compare(rendered[scenario.id], baselines[scenario.id])
        if deviation > MAX_CHANNEL_DEVIATION or differing > total * MAX_DIFFERING_FRACTION then
            return false,
                string.format(
                    "self-test setup invalid: %s does not match its own baseline "
                        .. "(max deviation %d/255, %d/%d pixels differ)",
                    scenario.id,
                    deviation,
                    differing,
                    total
                )
        end
        reports[#reports + 1] = string.format(
            "self-test control: %s vs own baseline, max deviation %d/255, %d/%d pixels differ",
            scenario.id,
            deviation,
            differing,
            total
        )
    end

    -- 2. Cross-checking crimson's render against azure's baseline MUST be
    --    rejected. If it isn't, the compare in `run` is not actually
    --    discriminating -- e.g. it could be comparing dimensions only, or the
    --    tolerance could have been widened until nothing fails -- and this gate
    --    would pass forever without proving anything.
    local crimson_id, azure_id = SCENARIOS[1].id, SCENARIOS[2].id
    local deviation, differing, total = compare(rendered[crimson_id], baselines[azure_id])
    if deviation <= MAX_CHANNEL_DEVIATION and differing <= total * MAX_DIFFERING_FRACTION then
        return false,
            string.format(
                "SELF-TEST FAILED: crimson's render passed against azure's baseline "
                    .. "(max deviation %d/255, %d/%d pixels differ) -- the tolerance cannot "
                    .. "detect a colour regression",
                deviation,
                differing,
                total
            )
    end
    reports[#reports + 1] = string.format(
        "self-test OK: crimson vs azure baseline rejected, max deviation %d/255 "
            .. "(tolerance %d/255), %d/%d pixels differ",
        deviation,
        MAX_CHANNEL_DEVIATION,
        differing,
        total
    )

    return true, table.concat(reports, "\n")
end

---@param mode "check"|"write"|"self-test"
---@return boolean ok
---@return string report
function snapshots.run(mode)
    if mode == "self-test" then
        return selfTest()
    end

    local write = mode == "write"
    local reports = {}
    local ok = true

    for _, scenario in ipairs(SCENARIOS) do
        local team = themes.TEAMS[scenario.team_index]
        local image = render(team)
        local relative = BASELINE_DIR .. "/" .. scenario.id .. ".png"
        local path = baseline_path(scenario.id)
        if write then
            write_file(path, image:encode("png"):getString())
            reports[#reports + 1] = "wrote " .. relative
        else
            local expected = read_file(path)
            if not expected then
                ok = false
                reports[#reports + 1] = "missing baseline " .. relative
            else
                local deviation, differing, total = compare(image, decode_png(expected))
                local within = deviation <= MAX_CHANNEL_DEVIATION
                    and differing <= total * MAX_DIFFERING_FRACTION
                ok = ok and within
                -- The measured deviation is printed whether it passes or fails,
                -- so quietly widening the bound until it passes would be visible
                -- in the history of this gate's own output.
                reports[#reports + 1] = string.format(
                    "%s %s: max deviation %d/255 (tolerance %d), %d/%d pixels differ (%.3f%%, cap %.1f%%)",
                    within and "matched" or "MISMATCH",
                    relative,
                    deviation,
                    MAX_CHANNEL_DEVIATION,
                    differing,
                    total,
                    100 * differing / math.max(total, 1),
                    100 * MAX_DIFFERING_FRACTION
                )
            end
        end
    end

    local mesh_ok, mesh_report = checkCharacterMeshBudget()
    ok = ok and mesh_ok
    reports[#reports + 1] = mesh_report

    return ok, table.concat(reports, "\n")
end

return snapshots
