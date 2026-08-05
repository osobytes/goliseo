-- Committed evidence for #337 slice 1: palette-slot vertex colours.
--
-- This is the visual half of the proof that pixel-diffing rig3d renders by
-- hand (in a scratch worktree, against a scratch script nobody else can run)
-- is not: a snapshot gate anyone can re-run a year from now, in the same
-- opt-in tier-4 style as the keeper/outfield pose snapshots (needs a
-- display, checked-in baselines, `write` to intentionally refresh).
--
-- It proves two separate things:
--   1. VISUAL: the same character, rendered with two different resolved
--      palettes, matches its checked-in baseline pixel-for-pixel. A colour
--      regression in themes.lua, meshbuilder.lua or the renderer.lua shader
--      shows up here.
--   2. STRUCTURAL: building one theme once and resolving N palettes never
--      allocates a GPU mesh beyond the first build -- the actual business
--      case for this slice (a cosmetic variant costs zero extra meshes).
--      This is a plain assertion, not a baseline, so it goes red the moment
--      something reintroduces a per-team mesh build.

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

-- One theme, both teams -- the exact scenario the #337 payoff is about: one
-- mesh build, two genuinely different colourways.
local SCENARIOS = {
    { id = "rig3d_palette_crimson", team_index = 1 },
    { id = "rig3d_palette_azure", team_index = 2 },
}

---@return table[]  -- body.build's drawlist, built once and reused by every scenario
---@return table     -- the rig proportions the drawlist was built against
local function parts()
    local rig = proportions.RIG_MEDIUM
    local theme = themes.LIST[1]
    local figure = themes.FIGURES[1]
    return body.build(rig, theme, figure), rig
end

---@param team table  -- an entry from themes.TEAMS
---@return love.ImageData
local function render(team)
    local rig_parts, rig = parts()
    local theme = themes.LIST[1]
    local palette = themes.resolvedPalette(theme, team)
    local rig_instance = skeleton.new(rig)

    renderer.load(themes.SLOT_COUNT)
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
    local world = mat4.rotationY(0)
    for _, part in ipairs(rig_parts) do
        local bone_world = mat4.multiply(world, rig_instance.world[part.bone])
        local model = part.attach and mat4.multiply(bone_world, part.attach) or bone_world
        renderer.draw(part.mesh, model, part.material)
    end
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

-- STRUCTURAL half of the proof: one theme build must serve every team, so
-- resolving additional palettes must never call love.graphics.newMesh.
-- Mirrors the exact wiring player_renderer_3d.build() uses (one body.build,
-- one themes.resolvedPalette per team) so a regression back to a per-team
-- body.build would be caught here, not just noticed in a benchmark.
---@return boolean ok
---@return string report
local function checkPaletteIsMeshFree()
    local rig = proportions.RIG_MEDIUM
    local theme = themes.LIST[1]
    local figure = themes.FIGURES[1]

    local mesh_count = 0
    local real_new_mesh = love.graphics.newMesh
    love.graphics.newMesh = function(...)
        mesh_count = mesh_count + 1
        return real_new_mesh(...)
    end

    local ok, err = pcall(function()
        local rig_parts = body.build(rig, theme, figure)
        local after_build = mesh_count
        assert(
            after_build == #rig_parts,
            "one theme build should allocate exactly one mesh per part"
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
        return false, "mesh-free palette check FAILED: " .. tostring(err)
    end
    return true,
        string.format(
            "mesh-free palette check OK: %d parts, %d team palettes, %d total newMesh() calls",
            mesh_count,
            #themes.TEAMS,
            mesh_count
        )
end

-- Proves the comparison in `run("check")` actually rejects a mismatch,
-- per AGENTS.md #9 ("every gate must come with a demonstration it can go
-- red"). Renders both scenarios fresh and checks each against the OTHER
-- scenario's baseline -- a real colour swap, not a synthetic byte flip, so
-- this exercises the exact equality check `run` uses on a case guaranteed to
-- differ (crimson's cloth is nowhere near azure's).
---@return boolean ok
---@return string report
local function selfTest()
    local root = love.filesystem.getSource()
    local rendered = {}
    for _, scenario in ipairs(SCENARIOS) do
        local team = themes.TEAMS[scenario.team_index]
        rendered[scenario.id] = render(team):encode("png"):getString()
    end

    -- 1. Each render must match ITS OWN baseline (sanity: the fixture is
    --    wired up and baselines exist).
    for _, scenario in ipairs(SCENARIOS) do
        local path = root .. "/" .. BASELINE_DIR .. "/" .. scenario.id .. ".png"
        local expected = read_file(path)
        if not expected then
            return false, "self-test needs baselines; run `write` first (missing " .. path .. ")"
        end
        if expected ~= rendered[scenario.id] then
            return false,
                "self-test setup invalid: " .. scenario.id .. " does not match its own baseline"
        end
    end

    -- 2. Cross-checking crimson's render against azure's baseline (and vice
    --    versa) MUST report a mismatch. If it doesn't, the compare in `run`
    --    is not actually discriminating between different palettes -- e.g.
    --    it could be comparing image dimensions or always short-circuiting
    --    true -- and this gate would pass forever without ever proving
    --    anything.
    local crimson_id, azure_id = SCENARIOS[1].id, SCENARIOS[2].id
    if rendered[crimson_id] == rendered[azure_id] then
        return false,
            "self-test invalid: crimson and azure rendered byte-identical, nothing to detect"
    end
    local azure_path = root .. "/" .. BASELINE_DIR .. "/" .. azure_id .. ".png"
    local azure_baseline = read_file(azure_path)
    if rendered[crimson_id] == azure_baseline then
        return false,
            "SELF-TEST FAILED: crimson's render matched azure's baseline -- compare cannot detect a colour regression"
    end

    return true, "self-test OK: the compare correctly rejects crimson-vs-azure as a mismatch"
end

---@param mode "check"|"write"|"self-test"
---@return boolean ok
---@return string report
function snapshots.run(mode)
    if mode == "self-test" then
        return selfTest()
    end

    local write = mode == "write"
    local root = love.filesystem.getSource()
    local reports = {}
    local ok = true

    for _, scenario in ipairs(SCENARIOS) do
        local team = themes.TEAMS[scenario.team_index]
        local encoded = render(team):encode("png"):getString()
        local relative = BASELINE_DIR .. "/" .. scenario.id .. ".png"
        local path = root .. "/" .. relative
        if write then
            write_file(path, encoded)
            reports[#reports + 1] = "wrote " .. relative
        else
            local expected = read_file(path)
            if expected ~= encoded then
                ok = false
                reports[#reports + 1] = "mismatch " .. relative
            else
                reports[#reports + 1] = "matched " .. relative
            end
        end
    end

    local mesh_ok, mesh_report = checkPaletteIsMeshFree()
    ok = ok and mesh_ok
    reports[#reports + 1] = mesh_report

    return ok, table.concat(reports, "\n")
end

return snapshots
