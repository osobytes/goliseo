-- #340: the gate that executes `game/render/player_renderer_3d.lua` itself.
--
-- WHY THIS IS NOT THE PALETTE GATE. `spec/support/rig3d_palette_snapshots.lua`
-- calls `body.build` directly, "mirroring the exact wiring
-- player_renderer_3d.build() uses". Mirroring is not exercising: the mirror can
-- stay correct while the real path rots, and until this file existed
-- `player_renderer_3d.build()`, `poseFor`, `clipFor`, `draw_player` and the
-- whole `pose -> skeleton.apply -> boneRows -> renderer.draw` integration were
-- executed by NOTHING in this repository. That is the finding #340 is about.
--
-- WHY THE BASELINE IS NUMBERS AND NOT AN IMAGE. The two committed rig3d PNGs
-- are pinned at EXACT pixel equality, which pins the GPU that produced them:
-- re-rendering them under Mesa llvmpipe (what a CI runner has) against
-- baselines made on an NVIDIA card gives 93% of pixels differing, max deviation
-- 39/255. A pixel baseline therefore cannot be a CI gate, and a gate nobody
-- runs is the exact failure #340 describes. So the pinned half of this gate is
-- the data that CROSSES THE SEAM into the GPU -- the resolved palette, the
-- character's model matrix, and every bone row -- captured at
-- `renderer.beginPass` / `renderer.draw`. That is pure Lua double arithmetic,
-- identical on every machine, and it is strictly closer to the logic under test
-- than the pixels are.
--
-- Pixels are still checked, just not pinned: two DRIVER-INDEPENDENT properties
-- that a numeric baseline cannot see (see COVERAGE below).
--
-- Four levels, all of which must pass:
--
--   A. INTEGRATION. Drive the real match path -- `bloom.draw(...)` wrapping
--      `pitch.draw(...)` with `pitch.rigged_players = true` -- and require that
--      the rigged branch was taken for every player. This is the only check
--      here that goes through pitch.lua's call site and through bloom's real
--      depth attachment, which is what `player_renderer_3d.available()`
--      actually demands.
--
--   B. BASELINE. Call `player_renderer_3d.draw` for a set of fixed scenarios
--      chosen to reach every branch of `poseFor`/`clipFor`, and compare the
--      captured palette/model/bone rows against the committed baseline.
--
--   C. COVERAGE. Read the rendered canvas back and require that (1) the
--      character actually rasterised -- a shader that links and draws nothing
--      passes every check in B -- and (2) the home and away renders differ,
--      which is the only evidence that the palette uniform reached the GPU
--      rather than merely being computed in Lua.
--
--   D. LOUDNESS. `player_renderer_3d.required` is set for the whole run, so a
--      build failure, a missing depth buffer or a draw failure raises here
--      instead of printing a line and falling back. A gate that measures the
--      procedural renderer while reporting on the rigged one is what #340 is.

local bloom = require("game.render.bloom")
local pitch = require("game.render.pitch")
local player_renderer_3d = require("game.render.player_renderer_3d")
local renderer = require("game.render.rig3d.renderer")
local render_payload = require("spec.support.render_payload")
local match = require("sim.match")
local teams = require("data.teams")

---@class Rig3dPlayerDrawModule
local snapshots = {}

local BASELINE = "spec/visual/baselines/rig3d_player_draw.txt"

-- Tolerance on every pinned number, and the reason for the exact value.
--
-- Everything pinned here is produced by pure Lua doubles, so on one machine the
-- reproduction is bit-exact. Across machines the only source of drift is libm:
-- `sin`/`cos`/`atan2` inside core/quat, core/mat4 and rig3d/clips may differ by
-- an ulp between glibc versions, which is ~1e-16 on values of order 1. A bone
-- row is a rotation entry (order 1) or a translation in METRES (order 1), so
-- 1e-6 is a micron -- five orders of magnitude below the smallest pose change
-- worth having, and ten orders above the noise it is there to absorb.
local TOLERANCE = 1e-6

-- Fixed clock. `poseFor` samples the idle clip and the keeper's gather against
-- `love.timer.getTime()`, so the pose it produces depends on when the harness
-- happened to run. Stubbing the clock is what makes B a baseline rather than a
-- lottery; the value is arbitrary but must never change, or every row moves.
local FIXED_TIME = 12.5

-- COVERAGE bounds for level C, as a fraction of the scanned box.
--
-- Deliberately loose. These exist to separate "a character was rasterised" from
-- "nothing came out", not to pin a silhouette -- the silhouette is pinned by
-- the bone rows in B, on hardware those bounds cannot see. Measured at 11.663%
-- covered and 3.400% team-differing on BOTH an NVIDIA RTX 2070 and Mesa
-- llvmpipe -- to three decimal places, which is more agreement than these
-- bounds need and a good deal more than the pixel baselines get (93% of their
-- pixels differ between those same two backends). Bounds sit roughly 4x either
-- side. The run prints both figures every time, so a drift toward an edge is
-- visible long before it fails.
local MIN_COVERED_FRACTION = 0.03
local MAX_COVERED_FRACTION = 0.40
-- How much of the box has to change when the team palette changes. Only the kit
-- slots differ between the two teams -- skin, hair and the metals are shared --
-- so this is well under the coverage figure, but a palette uniform that stopped
-- being sent would drop it to 0.
local MIN_TEAM_DIFFERENCE_FRACTION = 0.01

-- The colour the harness clears its canvas to. The scan does NOT compare
-- against these numbers: an 8-bit buffer stores 0.10 as 26/255 = 0.101961, so a
-- literal comparison would count every background pixel as covered. The
-- reference is read out of the image's own top-left corner instead, which is
-- also immune to a driver applying its own conversion on the way in.
local CLEAR = { 0.10, 0.10, 0.12, 1 }

-- The box the pixel scan looks at, around the drawn character. The frame is the
-- whole 960x540 window because the projection reads the window's dimensions,
-- but a character 22 px in radius covers a fraction of a percent of that, so
-- scanning all of it would put the interesting signal below the noise of the
-- bounds. Sized to hold the figure with room for a dive and its shadow.
local SCAN_HALF_WIDTH = 130
local SCAN_ABOVE = 190
local SCAN_BELOW = 40

-- Where the character's feet land, and its depth-scaled radius -- the same
-- three numbers pitch.lua's projection would hand the renderer. Fixed rather
-- than derived from a match so the baseline does not move when the sim does.
local FEET_X_FRACTION = 0.5
local FEET_Y_FRACTION = 0.86
local RADIUS = 22

---@class Rig3dPlayerDrawScenario
---@field id string
---@field team "home"|"away"
---@field view table
---@field opts table
---@field pixels boolean?  -- also read the canvas back (level C)

-- Chosen to reach every branch in `poseFor`/`clipFor`, because a baseline over
-- one pose would pin one path and leave the rest exactly as uncovered as before.
---@type Rig3dPlayerDrawScenario[]
local SCENARIOS = {
    -- Nothing selected: the idle clip alone, walk_mix 0, run_mix 0, and
    -- clipFor's `or "idle"` fallback. Also the home palette.
    {
        id = "idle_home",
        team = "home",
        view = { speed = 0, gait = 0 },
        opts = {},
        pixels = true,
    },
    -- The same pose and the same mesh with the away palette. Paired with
    -- idle_home this is the pixel evidence that colour is a uniform, through
    -- the real renderer rather than through body.build directly.
    {
        id = "idle_away",
        team = "away",
        view = { speed = 0, gait = 0 },
        opts = {},
        pixels = true,
    },
    -- Above WALK_SPEED and below RUN_SPEED: idle+walk layered, run_mix still 0.
    {
        id = "walk_home",
        team = "home",
        view = { speed = 40, gait = 0.37 },
        opts = { facing = { x = 0.6, y = -0.8 } },
    },
    -- Past RUN_SPEED: the second layer engages, so this is the only scenario
    -- that pins the run blend. Controlled, so draw_player's selection rings run.
    {
        id = "sprint_away",
        team = "away",
        view = { speed = 400, gait = 0.83 },
        opts = { controlled = true, facing = { x = -1, y = 0 } },
    },
    -- clipFor -> "guard": the UPPER_BODY stance layer.
    {
        id = "guard_home",
        team = "home",
        view = { speed = 20, gait = 0.21 },
        opts = { pose = { id = "combat_guard" }, facing = { x = 0, y = 1 } },
    },
    -- clipFor -> "charge": the other UPPER_BODY branch, phase-driven.
    {
        id = "charge_away",
        team = "away",
        view = { speed = 120, gait = 0.55 },
        opts = { pose = { id = "combat_active" }, windup = 0.4 },
    },
    -- Possession outranks the pose id: the keeper's gather.
    {
        id = "keeper_holding",
        team = "home",
        view = { speed = 0, gait = 0 },
        opts = { is_keeper = true, holding = true },
    },
    -- ...and the sling, which is the `throw` branch rather than `holding`.
    {
        id = "keeper_throw",
        team = "home",
        view = { speed = 0, gait = 0 },
        opts = { is_keeper = true, throw = 0.6 },
    },
    -- action_pose.apply on top of everything: a whole-body root transform, the
    -- one thing layered last so it rides the gait rather than competing.
    {
        id = "keeper_dive",
        team = "home",
        view = { speed = 30, gait = 0.11 },
        opts = {
            is_keeper = true,
            pose = { id = "keeper_stretch" },
            dive = 0.7,
            dive_dir = { x = 0, y = -1 },
            facing = { x = 1, y = 0 },
        },
    },
}

---@param values number[]
---@return number[]
local function copy(values)
    local out = {}
    for index, value in ipairs(values) do
        out[index] = value
    end
    return out
end

---@param rows number[][]
---@return number[][]
local function copyRows(rows)
    local out = {}
    for index, row in ipairs(rows) do
        out[index] = copy(row)
    end
    return out
end

-- Records everything `player_renderer_3d` hands the GPU, by standing in front of
-- the renderer module it already holds a reference to. `skeleton.boneRows`
-- writes into ONE reused buffer (see its own note), so the rows have to be
-- copied here or every scenario would end up holding the last one's pose.
---@param fn fun()
---@return table[]  -- one { palette, model, bones } per renderer.draw call
local function capture(fn)
    local real_begin, real_draw = renderer.beginPass, renderer.draw
    local calls, palette_now = {}, nil
    renderer.beginPass = function(camera, palette)
        palette_now = {}
        for index, slot in ipairs(palette) do
            palette_now[index] = copy(slot)
        end
        return real_begin(camera, palette)
    end
    renderer.draw = function(mesh, model, bone_rows)
        calls[#calls + 1] = {
            palette = palette_now,
            model = copy(model),
            bones = copyRows(bone_rows),
        }
        return real_draw(mesh, model, bone_rows)
    end

    local ok, err = pcall(fn)

    renderer.beginPass, renderer.draw = real_begin, real_draw
    if not ok then
        error(err, 0)
    end
    return calls
end

-- ---------------------------------------------------------------------------
-- Level A: the real match path
-- ---------------------------------------------------------------------------

-- Drives `bloom.draw(pitch.draw(...))` exactly as `game/render/benchmark.lua`
-- and the match itself do, with the rigged renderer switched on. Nothing here
-- is stubbed: bloom really allocates the depth attachment that
-- `player_renderer_3d.available()` requires, and pitch really chooses between
-- the two renderers at its one call site.
---@return boolean ok
---@return string report
local function integration()
    local width, height = love.graphics.getDimensions()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
    })
    local frame = render_payload.frame(state)
    local expected = frame.players.count

    local restore = pitch.rigged_players
    pitch.rigged_players = true
    local ok, calls = pcall(capture, function()
        bloom.draw(function()
            pitch.draw(frame, { w = width, h = height }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
            })
        end)
    end)
    pitch.rigged_players = restore

    if not ok then
        return false, "integration FAILED: the real match draw path raised: " .. tostring(calls)
    end
    if #calls ~= expected then
        return false,
            string.format(
                "integration FAILED: the real match path drew %d of %d players through the rigged "
                    .. "renderer -- pitch.lua fell back to the procedural one",
                #calls,
                expected
            )
    end
    return true,
        string.format(
            "integration OK: bloom.draw -> pitch.draw routed all %d players through "
                .. "player_renderer_3d (%d rigged draw calls, one per player)",
            expected,
            #calls
        )
end

-- ---------------------------------------------------------------------------
-- Level B / C: one scenario through player_renderer_3d.draw
-- ---------------------------------------------------------------------------

---@param scenario Rig3dPlayerDrawScenario
---@return table call            -- the captured { palette, model, bones }
---@return love.ImageData?       -- the canvas, when scenario.pixels
local function drawScenario(scenario)
    local width, height = love.graphics.getDimensions()
    -- The renderer reads love.graphics.getDimensions() for its projection, so
    -- the canvas has to be the window's size or the character lands off it.
    local canvas = love.graphics.newCanvas(width, height)

    local opts = {}
    for key, value in pairs(scenario.opts) do
        opts[key] = value
    end
    opts.team = scenario.team

    local real_time = love.timer.getTime
    love.timer.getTime = function()
        return FIXED_TIME
    end

    local ok, calls = pcall(capture, function()
        love.graphics.setCanvas({ canvas, depthstencil = true })
        love.graphics.clear(CLEAR[1], CLEAR[2], CLEAR[3], CLEAR[4])
        player_renderer_3d.draw(
            width * FEET_X_FRACTION,
            height * FEET_Y_FRACTION,
            RADIUS,
            { 1, 1, 1 },
            scenario.view,
            opts
        )
        love.graphics.setCanvas()
    end)

    love.timer.getTime = real_time
    if not ok then
        love.graphics.setCanvas()
        error(scenario.id .. ": " .. tostring(calls), 0)
    end
    assert(
        #calls == 1,
        string.format("%s: expected exactly 1 rigged draw call, got %d", scenario.id, #calls)
    )
    return calls[1], scenario.pixels and canvas:newImageData() or nil
end

-- The box around the drawn character, clamped to the image.
---@param image love.ImageData
---@return number x0, number y0, number x1, number y1
local function scanBox(image)
    local w, h = image:getDimensions()
    local cx = math.floor(w * FEET_X_FRACTION)
    local cy = math.floor(h * FEET_Y_FRACTION)
    return math.max(0, cx - SCAN_HALF_WIDTH),
        math.max(0, cy - SCAN_ABOVE),
        math.min(w - 1, cx + SCAN_HALF_WIDTH),
        math.min(h - 1, cy + SCAN_BELOW)
end

-- Fraction of the scan box that is not the background, i.e. that the character
-- (or its shadow) actually covered. The background reference is the image's own
-- top-left pixel, never the CLEAR constant -- see that constant's note.
---@param image love.ImageData
---@return number covered
local function coverage(image)
    local x0, y0, x1, y1 = scanBox(image)
    local kr, kg, kb = image:getPixel(0, 0)
    local covered, total = 0, 0
    for y = y0, y1 do
        for x = x0, x1 do
            local r, g, b = image:getPixel(x, y)
            total = total + 1
            if r ~= kr or g ~= kg or b ~= kb then
                covered = covered + 1
            end
        end
    end
    return covered / math.max(total, 1)
end

---@param a love.ImageData
---@param b love.ImageData
---@return number fraction differing
local function difference(a, b)
    local x0, y0, x1, y1 = scanBox(a)
    local differing, total = 0, 0
    for y = y0, y1 do
        for x = x0, x1 do
            local ar, ag, ab = a:getPixel(x, y)
            local br, bg, bb = b:getPixel(x, y)
            total = total + 1
            if ar ~= br or ag ~= bg or ab ~= bb then
                differing = differing + 1
            end
        end
    end
    return differing / math.max(total, 1)
end

-- ---------------------------------------------------------------------------
-- The committed baseline: a flat text file, on purpose
-- ---------------------------------------------------------------------------
--
-- Not a Lua table, because `write` would then produce a file that
-- `stylua --check` and `lua-language-server --check` both police, and a gate
-- whose refresh step breaks two other gates does not get refreshed. Not a hash,
-- because a hash can only say "different": these lines diff, and the report
-- below prints the measured deviation whether it passes or fails, so quietly
-- widening TOLERANCE until a regression fits would be visible in this gate's
-- own output.

---@param values number[]
---@return string
local function row(values)
    local parts = {}
    for index, value in ipairs(values) do
        parts[index] = string.format("%.9g", value)
    end
    return table.concat(parts, " ")
end

---@param calls table<string, table>
---@return string
local function serialize(calls)
    local out = {
        "# rig3d player draw baseline -- what game/render/player_renderer_3d.lua",
        "# hands the GPU, captured at renderer.beginPass / renderer.draw.",
        "# Generated by `love . --rig3d-player-draw write`; see",
        "# spec/support/rig3d_player_draw.lua for what each scenario pins.",
        "version 1",
        "clock " .. string.format("%.9g", FIXED_TIME),
    }
    for _, scenario in ipairs(SCENARIOS) do
        local call = calls[scenario.id]
        out[#out + 1] = "scenario " .. scenario.id
        for index, slot in ipairs(call.palette) do
            out[#out + 1] = "palette " .. (index - 1) .. " " .. row(slot)
        end
        out[#out + 1] = "model " .. row(call.model)
        for index, bone in ipairs(call.bones) do
            out[#out + 1] = "bone " .. (index - 1) .. " " .. row(bone)
        end
    end
    return table.concat(out, "\n") .. "\n"
end

-- Parses the baseline back into { [scenario] = { key -> number[] } }, where a
-- key is "palette 0", "model" or "bone 17". Flat on purpose: the comparison
-- below reports the key that moved, which is the whole point of not hashing.
---@param text string
---@return table<string, table<string, number[]>>
---@return table<string, string[]>  -- key order per scenario, so a report reads in file order
local function parse(text)
    local scenarios, order, current = {}, {}, nil
    for line in text:gmatch("[^\n]+") do
        if line:sub(1, 1) ~= "#" then
            local head, rest = line:match("^(%S+)%s*(.*)$")
            if head == "scenario" then
                current = rest
                scenarios[current] = {}
                order[current] = {}
            elseif head == "palette" or head == "bone" then
                local index, values = rest:match("^(%d+)%s+(.*)$")
                local key = head .. " " .. index
                local numbers = {}
                for value in values:gmatch("%S+") do
                    numbers[#numbers + 1] = tonumber(value)
                end
                scenarios[current][key] = numbers
                table.insert(order[current], key)
            elseif head == "model" then
                local numbers = {}
                for value in rest:gmatch("%S+") do
                    numbers[#numbers + 1] = tonumber(value)
                end
                scenarios[current]["model"] = numbers
                table.insert(order[current], "model")
            end
        end
    end
    return scenarios, order
end

---@param call table
---@return table<string, number[]>
local function flatten(call)
    local out = { model = call.model }
    for index, slot in ipairs(call.palette) do
        out["palette " .. (index - 1)] = slot
    end
    for index, bone in ipairs(call.bones) do
        out["bone " .. (index - 1)] = bone
    end
    return out
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

---@return string
local function baseline_path()
    return love.filesystem.getSource() .. "/" .. BASELINE
end

-- Compares one scenario against its committed rows.
---@param actual table<string, number[]>
---@param expected table<string, number[]>
---@param keys string[]
---@return boolean ok
---@return number max_deviation
---@return string? detail  -- the first key that moved, and by how much
local function compareScenario(actual, expected, keys)
    local max_deviation, detail = 0, nil
    for _, key in ipairs(keys) do
        local want, got = expected[key], actual[key]
        if not got then
            return false, math.huge, key .. " is missing from this run"
        end
        if #want ~= #got then
            return false,
                math.huge,
                string.format("%s has %d values, baseline has %d", key, #got, #want)
        end
        for index = 1, #want do
            local deviation = math.abs(got[index] - want[index])
            if deviation > max_deviation then
                max_deviation = deviation
                if deviation > TOLERANCE then
                    detail = string.format(
                        "%s[%d] is %.9g, baseline %.9g (off by %.3g)",
                        key,
                        index,
                        got[index],
                        want[index],
                        deviation
                    )
                end
            end
        end
    end
    -- A key the run produced that the baseline never heard of is drift too:
    -- more bones, a longer palette. Caught by count rather than by name.
    local actual_count = 0
    for _ in pairs(actual) do
        actual_count = actual_count + 1
    end
    if actual_count ~= #keys then
        return false,
            math.huge,
            string.format("this run produced %d rows, the baseline has %d", actual_count, #keys)
    end
    return max_deviation <= TOLERANCE, max_deviation, detail
end

-- ---------------------------------------------------------------------------
-- The self-test: prove each level can reject, without touching the renderer
-- ---------------------------------------------------------------------------
--
-- AGENTS.md #9 requires every gate ship with a demonstration it can go red, and
-- is explicit that this is NOT a substitute for running the real thing -- so
-- `check` remains the gate and this is only the proof its checks discriminate.
-- Each case perturbs the EVIDENCE rather than the renderer, which is what lets
-- it run in the same process without leaving the renderer in a broken state.
---@return boolean ok
---@return string report
local function selfTest()
    local reports = {}
    local problems = 0

    ---@param label string
    ---@param condition boolean
    ---@param note string
    local function expect(label, condition, note)
        if condition then
            reports[#reports + 1] = "self-test OK: " .. label
        else
            problems = problems + 1
            reports[#reports + 1] = "SELF-TEST FAILED: " .. label .. " -- " .. note
        end
    end

    local text = read_file(baseline_path())
    if not text then
        return false, "self-test needs the baseline; run `write` first (missing " .. BASELINE .. ")"
    end
    local expected, order = parse(text)

    -- 1. The control: a real render of the first scenario matches its own rows.
    --    If this fails, every rejection below is meaningless.
    local first = SCENARIOS[1]
    local call = drawScenario(first)
    local actual = flatten(call)
    local ok, deviation = compareScenario(actual, expected[first.id], order[first.id])
    expect(
        string.format("%s matches its own baseline (max deviation %.3g)", first.id, deviation),
        ok,
        "the fixture is not wired up, so nothing below proves anything"
    )

    -- 2. A single bone nudged by ten tolerances must be rejected. This is the
    --    shape of a real pose regression -- one joint moving -- and it is the
    --    case a hash would report as "different" with no idea what moved.
    local nudged = {}
    for key, values in pairs(expected[first.id]) do
        nudged[key] = copy(values)
    end
    nudged["bone 3"][4] = nudged["bone 3"][4] + TOLERANCE * 10
    local rejected, _, detail = compareScenario(actual, nudged, order[first.id])
    expect(
        "one bone row moved by 10x the tolerance is rejected (" .. tostring(detail) .. ")",
        not rejected,
        "the comparison cannot detect a single joint moving"
    )

    -- 3. Cross-scenario: idle must not pass against the sprint rows. A
    --    tolerance nobody has shown can reject a whole different pose is
    --    indistinguishable from no check at all.
    local sprint = "sprint_away"
    local crossed = compareScenario(actual, expected[sprint], order[sprint])
    expect(
        "idle_home is rejected against " .. sprint .. "'s baseline",
        not crossed,
        "the comparison does not discriminate between poses"
    )

    -- 4. Level C's floor must reject an empty frame. Rendered by clearing the
    --    canvas and drawing nothing at all -- the "shader linked, produced no
    --    pixels" case that every numeric check above passes happily.
    local width, height = love.graphics.getDimensions()
    local blank = love.graphics.newCanvas(width, height)
    love.graphics.setCanvas(blank)
    love.graphics.clear(CLEAR[1], CLEAR[2], CLEAR[3], CLEAR[4])
    love.graphics.setCanvas()
    local blank_covered = coverage(blank:newImageData())
    expect(
        string.format("an empty frame is rejected (covered %.4f%%)", blank_covered * 100),
        blank_covered < MIN_COVERED_FRACTION,
        "the coverage floor would accept a render that produced nothing"
    )

    -- 5. And the team-difference floor must reject two identical images, i.e.
    --    the case where the palette uniform stopped being sent.
    local same = difference(blank:newImageData(), blank:newImageData())
    expect(
        string.format("two identical renders are rejected as a palette swap (%.4f%%)", same * 100),
        same < MIN_TEAM_DIFFERENCE_FRACTION,
        "the palette check would accept two identical images"
    )

    -- 6. Level A's count: a run where pitch did NOT route to the rigged
    --    renderer must be caught. Driven for real, with rigged_players off.
    local restore = pitch.rigged_players
    pitch.rigged_players = false
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
    })
    local frame = render_payload.frame(state)
    local fallback = capture(function()
        bloom.draw(function()
            pitch.draw(frame, { w = width, h = height }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
            })
        end)
    end)
    pitch.rigged_players = restore
    expect(
        string.format(
            "a match drawn on the procedural path reports 0 rigged draws (got %d of %d players)",
            #fallback,
            frame.players.count
        ),
        #fallback == 0,
        "the integration check counts draws that did not happen"
    )

    if problems > 0 then
        return false, table.concat(reports, "\n")
    end
    return true, table.concat(reports, "\n")
end

-- ---------------------------------------------------------------------------

-- `reports` is owned by the caller so that a level which RAISES rather than
-- returning false still leaves everything measured before it on stdout. The
-- first refusal carries the root cause ("the build failed: ...shader..."); a
-- later one only says the renderer had already disabled itself, so losing the
-- earlier lines would cost the only diagnostic worth reading.
---@param mode "check"|"write"
---@param reports string[]
---@return boolean ok
local function execute(mode, reports)
    local ok = true

    local integration_ok, integration_report = integration()
    ok = ok and integration_ok
    reports[#reports + 1] = integration_report

    local calls, images = {}, {}
    for _, scenario in ipairs(SCENARIOS) do
        local call, image = drawScenario(scenario)
        calls[scenario.id] = call
        if image then
            images[scenario.id] = image
        end
    end

    if mode == "write" then
        write_file(baseline_path(), serialize(calls))
        reports[#reports + 1] = string.format(
            "wrote %s (%d scenarios, %d bone rows each)",
            BASELINE,
            #SCENARIOS,
            #calls[SCENARIOS[1].id].bones
        )
    else
        local text = read_file(baseline_path())
        if not text then
            ok = false
            reports[#reports + 1] = "missing baseline " .. BASELINE .. " -- run `write` first"
        else
            local expected, order = parse(text)
            for _, scenario in ipairs(SCENARIOS) do
                local want = expected[scenario.id]
                if not want then
                    ok = false
                    reports[#reports + 1] = "MISSING " .. scenario.id .. " in " .. BASELINE
                else
                    local matched, deviation, detail =
                        compareScenario(flatten(calls[scenario.id]), want, order[scenario.id])
                    ok = ok and matched
                    reports[#reports + 1] = string.format(
                        "%s %s: %d rows, max deviation %.3g (tolerance %.3g)%s",
                        matched and "matched" or "MISMATCH",
                        scenario.id,
                        #order[scenario.id],
                        deviation,
                        TOLERANCE,
                        detail and (" -- " .. detail) or ""
                    )
                end
            end
        end
    end

    -- Level C. Measured and printed on every run, pass or fail, so a slide
    -- toward either bound is visible before it becomes a failure.
    for _, scenario in ipairs(SCENARIOS) do
        local image = images[scenario.id]
        if image then
            local covered = coverage(image)
            local within = covered >= MIN_COVERED_FRACTION and covered <= MAX_COVERED_FRACTION
            ok = ok and within
            reports[#reports + 1] = string.format(
                "%s %s rasterised %.3f%% of the frame (bounds %.1f%%..%.1f%%)",
                within and "coverage OK:" or "COVERAGE FAILED:",
                scenario.id,
                covered * 100,
                MIN_COVERED_FRACTION * 100,
                MAX_COVERED_FRACTION * 100
            )
        end
    end

    local home, away = images["idle_home"], images["idle_away"]
    if home and away then
        local differing = difference(home, away)
        local within = differing >= MIN_TEAM_DIFFERENCE_FRACTION
        ok = ok and within
        reports[#reports + 1] = string.format(
            "%s the two team palettes differ over %.3f%% of the frame (floor %.1f%%)",
            within and "palette OK:" or "PALETTE FAILED:",
            differing * 100,
            MIN_TEAM_DIFFERENCE_FRACTION * 100
        )
    end

    return ok
end

---@param mode "check"|"write"|"self-test"
---@return boolean ok
---@return string report
function snapshots.run(mode)
    -- Level D, for the whole run: from here on a fallback raises rather than
    -- printing. Set before the first `available()` or `draw()` so nothing can
    -- latch `failed` quietly ahead of it.
    player_renderer_3d.required = true

    -- Level D is an `assert`, and an assert that escapes `love.load` hands the
    -- process to LÖVE's error screen -- which draws a window and loops forever
    -- rather than exiting. In CI that is not a red gate, it is a job that burns
    -- its timeout, so the loud failure is caught here and converted into the
    -- one thing every consumer of this harness reads: a `fail` verdict and a
    -- non-zero exit. The message is preserved verbatim.
    if mode == "self-test" then
        local ran, ok, report = pcall(selfTest)
        if not ran then
            return false, "ABORTED: " .. tostring(ok)
        end
        return ok, report
    end

    local reports = {}
    local ran, ok = pcall(execute, mode, reports)
    if not ran then
        reports[#reports + 1] = "ABORTED: " .. tostring(ok)
        ok = false
    end
    if mode == "write" then
        return ok, table.concat(reports, "\n")
    end
    reports[#reports + 1] = ok and "RIG3D-PLAYER-DRAW ok" or "RIG3D-PLAYER-DRAW fail"
    return ok, table.concat(reports, "\n")
end

return snapshots
