-- The #100 ten-player render benchmark.
--
-- Answers one question with evidence instead of estimate: does the rigged
-- player renderer hold the omp0 frame gates with a full match on screen, on
-- every runtime we ship to?
--
-- WHAT THIS MEASURES, AND WHY IT IS NOT WHAT #100 ANTICIPATED
--
-- #100 was written against a Menori/glTF path doing GPU skinning, so its metric
-- list leads with GLB conversion, joint uploads and skeleton evaluation. That
-- path was never built. The renderer that exists (game/render/rig3d/) generates
-- meshes in code and draws one RIGID PART PER BONE with its own model matrix --
-- there is no skinning, no GLB, and no joint upload at all.
--
-- That inverts the expected bottleneck. Skinned characters cost 1-2 draw calls
-- each; this costs one per part, so ten players is several hundred draw calls
-- and several hundred uniform sends per frame. Draw-call count is therefore a
-- first-class metric here rather than a footnote.
--
-- What it is NOT is the browser's cost. This comment used to add "and it is the
-- number most likely to behave differently in love.js, where every call crosses
-- Lua -> JS -> WebGL", which was a reasonable guess and has been measured false:
-- the entire WebGL-internal cost of a rigged browser frame is ~0.28 ms of
-- 5.25 ms (~5%), and a prototype cutting GL calls 756 -> 429 per frame moved
-- browser draw by 2%. The browser gap is the wasm-hosted Lua interpreter running
-- draw-side code, not submission -- see docs/online/browser_rigged_3d.md. Keep
-- measuring draw calls, because the RENDERER's design makes them a real cost
-- natively; just do not read a browser number as a boundary cost.
--
-- FAIRNESS RULES, all of which exist so the two renderers are comparable:
--   * One fixture: same seed, same bot, same fixed timestep, same duration,
--     same viewport. The renderer choice is the ONLY difference.
--   * Warm-up is discarded. Shader compilation, mesh generation and the first
--     GC cycle are startup costs, and #100 wants them measured separately from
--     steady state.
--   * Update and draw are timed separately, because the gates are separate.
--   * The simulation hash is captured at the end. Presentation must not reach
--     the simulation, and a matching hash across renderer choices is what
--     proves it -- a fast frame that changed the game is not a pass.
--
-- Vsync is disabled by the caller for the measured run. With vsync on, frame
-- time is pinned to the refresh rate and reports 16.67 ms whether the renderer
-- has 90% headroom or none, which would make the whole exercise say nothing.

local bot = require("sim.bot")
local fixed_clock = require("sim.fixed_clock")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local metrics = require("sim.metrics")
local arenas = require("data.arenas")
local teams = require("data.teams")
local bloom = require("game.render.bloom")
local pitch = require("game.render.pitch")
local pitch_static = require("game.render.pitch_static")
local render_frame = require("render.frame")
local player_renderer_3d = require("game.render.player_renderer_3d")
local view_state = require("game.render.view_state")

local benchmark = {}
local Benchmark = {}
Benchmark.__index = Benchmark

local DT = fixed_clock.TICK_SECONDS

-- Thresholds from docs/online/omp0_acceptance.md. Named rather than inlined so
-- the report and the gate cannot drift apart.
benchmark.GATES = {
    update_p95_ms = 8,
    update_max_ms = 33,
    draw_p95_ms = 8,
    draw_max_ms = 33,
    slow_frame_ms = 33,
    slow_frames_per_60s = 3,
    stall_frame_ms = 250,
}

-- Feature-delta budget for the rigged renderer over the procedural baseline.
benchmark.DELTA_BUDGET = { update_p95_ms = 1, draw_p95_ms = 2 }

---@param values number[]
---@param q number  -- 0..1
---@return number
local function percentile(values, q)
    if #values == 0 then
        return 0
    end
    -- Nearest-rank on a sorted copy. The caller sorts once and reuses.
    local rank = math.max(1, math.ceil(q * #values))
    return values[math.min(rank, #values)]
end

---@param values number[]
---@param threshold number
---@return integer
local function count_above(values, threshold)
    local n = 0
    for _, v in ipairs(values) do
        if v > threshold then
            n = n + 1
        end
    end
    return n
end

---@param samples number[]  -- seconds
---@return table
local function summarise(samples)
    local ms = {}
    for i, v in ipairs(samples) do
        ms[i] = v * 1000
    end
    local sorted = {}
    for i, v in ipairs(ms) do
        sorted[i] = v
    end
    table.sort(sorted)
    local total = 0
    for _, v in ipairs(ms) do
        total = total + v
    end
    return {
        samples = #ms,
        mean_ms = #ms > 0 and (total / #ms) or 0,
        p50_ms = percentile(sorted, 0.50),
        p95_ms = percentile(sorted, 0.95),
        p99_ms = percentile(sorted, 0.99),
        max_ms = #sorted > 0 and sorted[#sorted] or 0,
        over_16_67 = count_above(ms, 1000 / 60),
        over_33 = count_above(ms, benchmark.GATES.slow_frame_ms),
        over_250 = count_above(ms, benchmark.GATES.stall_frame_ms),
    }
end

-- Builds the fixture. `rigged` selects the renderer under test; everything else
-- is identical between runs by construction.
---@param opts table  -- { rigged, seconds, warmup_seconds, seed, width, height }
---@return table
function benchmark.new(opts)
    local self = setmetatable({}, Benchmark)
    self.rigged = opts.rigged and true or false
    self.seed = opts.seed or 20260803
    -- Measured in FRAMES, not wall seconds, and one simulation tick runs per
    -- frame. This is a fairness requirement, not a convenience.
    --
    -- With vsync off a fast renderer produces frames far quicker than 60 Hz. If
    -- the fixture ran for a wall-clock window instead, the two renderers would
    -- execute different tick counts over that window and end up drawing
    -- different match situations -- so the comparison would be measuring two
    -- different scenes. Pinning frames pins the match content: both runs draw
    -- the identical sequence of game states, and the only thing that varies is
    -- how long each frame took.
    self.frames = opts.frames or 3600 -- 60 s at 60 Hz
    self.warmup_frames = opts.warmup_frames or 300 -- 5 s at 60 Hz
    self.viewport = { w = opts.width or 960, h = opts.height or 540 }
    -- #393: the static-scene cache under test. Default on (the shipped path);
    -- `nocache` runs measure the uncached path from the same build, which is
    -- what makes interleaved before/after passes comparable.
    self.static_cache = opts.static_cache ~= false
    -- #394: per-character sub-phase profiling. Opt-in rather than always-on
    -- because it costs ~10 clock reads per character per frame, which is real
    -- overhead under the wasm interpreter -- a profiling run accepts it, a
    -- before/after comparison run should not carry it.
    self.char_phases = opts.char_phases and true or false
    -- #394: the pose LOD under test. Default on (the shipped path); `nolod`
    -- runs measure full-rate pose evaluation from the same build, exactly as
    -- `nocache` serves the #393 cache.
    self.pose_lod = opts.pose_lod ~= false

    pitch.rigged_players = self.rigged
    pitch_static.enabled = self.static_cache
    pitch_static.reset()
    player_renderer_3d.pose_lod = self.pose_lod
    -- Also drops the #394 pose cache: its entries are weak-keyed on the
    -- PlayerViews this discards.
    view_state.reset()

    self.state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        human_controlled = true,
        seed = self.seed,
        -- Long enough that the measured window never runs into full time and
        -- starts sampling a result screen instead of a match.
        duration = (self.warmup_frames + self.frames) * DT + 60,
    })
    -- Match-constant identity for the payload build; derived once, exactly as
    -- the real match screen does.
    self.render_roster = render_frame.roster(self.state)
    self.bot = bot.new({ seed = self.seed })
    self.metrics = metrics.new(self.state)
    self.clock = fixed_clock.new()

    self.update_samples = {}
    self.draw_samples = {}
    self.frame_samples = {}
    -- #393: pitch.draw's own static/dynamic split, so "the scene is the cost"
    -- can be confirmed (or refuted) per runtime instead of estimated.
    self.scene_static_samples = {}
    self.scene_dynamic_samples = {}
    self.phase_sink = {}
    pitch.phase_sink = self.phase_sink
    -- #394: the per-character split of scene_dynamic. One sample per frame per
    -- phase, each the SUM across every character drawn that frame, so the rows
    -- read in the same unit as scene_dynamic and add up against it.
    self.char_phase_names = { "pose", "apply", "rows", "submit", "other" }
    self.char_samples = {}
    for _, name in ipairs(self.char_phase_names) do
        self.char_samples[name] = {}
    end
    self.char_counts = {}
    self.char_sink = self.char_phases and {} or nil
    player_renderer_3d.phase_sink = self.char_sink
    self.draw_calls = {}
    -- Which pose ids the fixture actually exercised. Reported rather than
    -- asserted: a benchmark that silently covered only locomotion would look
    -- like a pass while measuring a third of the work.
    self.poses_seen = {}
    self.frame_index = 0
    self.measured_frames = 0
    self.warmed = false
    self.done = false
    self.last_frame_at = nil
    return self
end

---@return boolean
function Benchmark:warming()
    return not self.warmed
end

-- Exactly one fixed-timestep simulation tick, timed. Rendering is timed
-- separately in draw(), because the omp0 gates budget them separately.
--
-- The caller's real `dt` is used ONLY to measure frame pacing; it never reaches
-- the simulation, which always advances by the fixed tick. That is what keeps
-- the two renderers drawing the same match.
function Benchmark:update()
    if self.done then
        return
    end
    local now = love.timer.getTime()
    if self.last_frame_at and self.warmed then
        self.frame_samples[#self.frame_samples + 1] = now - self.last_frame_at
    end
    self.last_frame_at = now

    local input = bot.input(self.bot, self.state, DT)
    local started = love.timer.getTime()
    fixed_clock.step(self.clock, input, function(_, tick_input)
        match.step(self.state, DT, tick_input)
        metrics.observe(self.metrics, self.state, DT)
        return not self.state.finished
    end)
    local update_seconds = love.timer.getTime() - started

    view_state.update(self.state.players, DT)

    self.frame_index = self.frame_index + 1
    if not self.warmed then
        if self.frame_index >= self.warmup_frames then
            self.warmed = true
            -- Start steady state from a known allocation floor so the first
            -- measured frames are not dominated by warm-up garbage.
            collectgarbage("collect")
            self.warm_memory_kb = collectgarbage("count")
        end
        return
    end

    self.update_samples[#self.update_samples + 1] = update_seconds
    self.measured_frames = self.measured_frames + 1
    if self.measured_frames >= self.frames then
        self:finish()
    end
end

function Benchmark:draw()
    if self.done then
        return
    end
    if love.graphics.getStats then
        love.graphics.getStats() -- reset-free read; sampled after the draw below
    end
    -- Through bloom.draw, exactly as game/screens/match.lua composes the real
    -- frame. This is not incidental fidelity -- it is load-bearing.
    --
    -- The rigged renderer depth-tests inside the bloom scene pass, so it checks
    -- bloom.hasDepth() before drawing and quietly defers to the procedural path
    -- when there is no depth attachment. Calling pitch.draw directly means no
    -- canvas is ever bound, hasDepth() is false, and the "rigged" run measures
    -- the procedural renderer while reporting a pass. The first version of this
    -- file did exactly that and produced two identical result rows.
    --
    -- Bloom's own post-processing cost is inside the measurement on purpose:
    -- it is part of what a shipped frame pays.
    local char_sink = self.char_sink
    if char_sink then
        -- The sink accumulates across characters; the frame owns zeroing it.
        char_sink.pose_s, char_sink.apply_s, char_sink.rows_s = 0, 0, 0
        char_sink.submit_s, char_sink.other_s, char_sink.characters = 0, 0, 0
    end
    local started = love.timer.getTime()
    bloom.draw(function()
        -- The payload build is inside the measurement on purpose: deriving the
        -- render frame is part of what a shipped frame pays.
        local frame = render_frame.build(self.state, { roster = self.render_roster })
        pitch.draw(frame, self.viewport, {
            home_color = teams.nebula.color,
            away_color = teams.orion.color,
            arena = arenas.helios_crown,
        })
    end)
    -- Outside the bloom pass, exactly as game/screens/match.lua does: a
    -- pending static-scene cache build must never bind canvases while bloom's
    -- depth-attached scene pass is active. Inside the timed window on purpose;
    -- the one-off build is part of what a shipped frame pays.
    pitch_static.flush()
    local draw_seconds = love.timer.getTime() - started

    -- Which renderer actually ran, sampled rather than assumed. The gate fails
    -- on a mismatch, so a silent fallback can never be reported as a pass.
    self.rigged_active = pitch.rigged_players and player_renderer_3d.available()
    -- Same discipline for the #393 cache: report whether a cached blit is
    -- actually in service, not just whether it was requested.
    self.static_cache_active = pitch_static.status().cached

    if self.warmed then
        self.draw_samples[#self.draw_samples + 1] = draw_seconds
        if self.phase_sink.scene_static_s then
            self.scene_static_samples[#self.scene_static_samples + 1] =
                self.phase_sink.scene_static_s
            self.scene_dynamic_samples[#self.scene_dynamic_samples + 1] =
                self.phase_sink.scene_dynamic_s
        end
        if char_sink then
            for _, name in ipairs(self.char_phase_names) do
                local samples = self.char_samples[name]
                samples[#samples + 1] = char_sink[name .. "_s"] or 0
            end
            self.char_counts[#self.char_counts + 1] = char_sink.characters or 0
        end
        local stats = love.graphics.getStats and love.graphics.getStats() or nil
        if stats then
            self.draw_calls[#self.draw_calls + 1] = stats.drawcalls or 0
            self.texture_memory = stats.texturememory
        end
        for _, p in ipairs(self.state.players) do
            local view = view_state.get(p.id)
            if view then
                self.poses_seen[p.is_keeper and "keeper" or "outfield"] = true
            end
        end
    end
end

function Benchmark:finish()
    if self.done then
        return
    end
    self.done = true
    self.final_hash = match_snapshot.hash(match_snapshot.capture(self.state))
    self.end_memory_kb = collectgarbage("count")
end

---@return boolean
function Benchmark:finished()
    return self.done
end

-- Machine-readable evidence. Deliberately flat and JSON-shaped: #100 requires
-- the metrics be reviewable without running Lua.
---@return table
function Benchmark:result()
    local draw_call_total, draw_call_max = 0, 0
    for _, v in ipairs(self.draw_calls) do
        draw_call_total = draw_call_total + v
        draw_call_max = math.max(draw_call_max, v)
    end
    -- #394: per-character sub-phase summaries, keyed by phase name. Empty
    -- tables summarise to zeros when profiling was off, and emit() skips them.
    local char = { raw = {} }
    for _, name in ipairs(self.char_phase_names) do
        char[name] = summarise(self.char_samples[name])
        char.raw[name] = self.char_samples[name]
    end
    local chars_total = 0
    for _, v in ipairs(self.char_counts) do
        chars_total = chars_total + v
    end
    return {
        renderer = self.rigged and "rigged" or "procedural",
        -- Requested vs actually used. These diverging is the failure this
        -- benchmark is most likely to hit and least likely to notice.
        rigged_requested = self.rigged,
        rigged_active = self.rigged_active and true or false,
        seed = self.seed,
        warmup_frames = self.warmup_frames,
        measured_frames = self.measured_frames,
        -- Simulated seconds the measured window covers, which is what the
        -- per-60-second pacing gate is written against.
        measured_seconds = self.measured_frames * DT,
        viewport = self.viewport,
        players = #self.state.players,
        update = summarise(self.update_samples),
        draw = summarise(self.draw_samples),
        frame = summarise(self.frame_samples),
        scene_static = summarise(self.scene_static_samples),
        scene_dynamic = summarise(self.scene_dynamic_samples),
        static_cache = self.static_cache,
        static_cache_active = self.static_cache_active and true or false,
        char_phases = self.char_phases,
        char = char,
        chars_per_frame_mean = #self.char_counts > 0 and (chars_total / #self.char_counts) or 0,
        pose_lod = self.pose_lod,
        draw_calls_mean = #self.draw_calls > 0 and (draw_call_total / #self.draw_calls) or 0,
        draw_calls_max = draw_call_max,
        texture_memory_bytes = self.texture_memory,
        lua_memory_warm_kb = self.warm_memory_kb,
        lua_memory_end_kb = self.end_memory_kb,
        final_state_hash = self.final_hash,
        love_version = string.format("%d.%d.%d", love.getVersion()),
        -- getRendererInfo returns (name, version, vendor, device).
        gpu_name = select(1, love.graphics.getRendererInfo()),
        gpu_vendor = select(3, love.graphics.getRendererInfo()),
        gpu_device = select(4, love.graphics.getRendererInfo()),
        raw_update_us = self.update_samples,
        raw_draw_us = self.draw_samples,
        raw_frame_us = self.frame_samples,
        raw_scene_static_us = self.scene_static_samples,
        raw_scene_dynamic_us = self.scene_dynamic_samples,
    }
end

-- Gate evaluation, kept next to the thresholds so a report can never claim a
-- pass the numbers do not support.
---@param result table
---@return boolean, string[]
function benchmark.evaluate(result)
    local g, failures = benchmark.GATES, {}
    local function check(ok, message)
        if not ok then
            failures[#failures + 1] = message
        end
    end
    check(
        result.update.p95_ms <= g.update_p95_ms,
        string.format("update p95 %.2f ms > %d ms", result.update.p95_ms, g.update_p95_ms)
    )
    check(
        result.update.max_ms <= g.update_max_ms,
        string.format("update max %.2f ms > %d ms", result.update.max_ms, g.update_max_ms)
    )
    check(
        result.draw.p95_ms <= g.draw_p95_ms,
        string.format("draw p95 %.2f ms > %d ms", result.draw.p95_ms, g.draw_p95_ms)
    )
    check(
        result.draw.max_ms <= g.draw_max_ms,
        string.format("draw max %.2f ms > %d ms", result.draw.max_ms, g.draw_max_ms)
    )
    -- The pacing gate is written per 60 seconds, so scale it to what we ran
    -- rather than comparing a 10-second run against a 60-second allowance.
    local allowance = g.slow_frames_per_60s * math.max(1, result.measured_seconds / 60)
    check(
        result.frame.over_33 <= allowance,
        string.format("%d frames over 33 ms > allowance %.1f", result.frame.over_33, allowance)
    )
    check(result.frame.over_250 == 0, string.format("%d frames over 250 ms", result.frame.over_250))
    -- Evidence integrity: a rigged run that silently fell back to the
    -- procedural renderer is not a fast rigged run, it is no measurement at all.
    check(
        not result.rigged_requested or result.rigged_active,
        "rigged renderer was requested but fell back to procedural (no depth buffer?)"
    )
    -- Same rule for the #393 cache: a "cached" run whose cache never engaged
    -- (canvas creation failed, build latched off) measured the uncached path
    -- and must say so rather than pass as a cache result.
    check(
        not result.static_cache or result.static_cache_active,
        "static-scene cache was requested but never engaged (canvas build failed?)"
    )
    return #failures == 0, failures
end

-- Evidence goes out as pipe-delimited `GC_BENCH_*` lines rather than JSON,
-- matching the rollback harness. That is not a style choice: this same fixture
-- has to run inside love.js, where the only channel out is the browser console,
-- and a line-oriented format survives that crossing while a serialised object
-- does not. scripts/render_benchmark.py folds these into the JSON artifact.
---@param values number[]  -- seconds
---@return string
local function microseconds(values)
    local out = {}
    for i, v in ipairs(values) do
        out[i] = string.format("%d", math.floor(v * 1e6 + 0.5))
    end
    return table.concat(out, ",")
end

---@param summary table
---@param prefix string
---@return string
local function summary_fields(summary, prefix)
    return string.format(
        "%s_p50=%.4f|%s_p95=%.4f|%s_p99=%.4f|%s_max=%.4f|%s_mean=%.4f|%s_over16=%d|%s_over33=%d|%s_over250=%d|%s_n=%d",
        prefix,
        summary.p50_ms,
        prefix,
        summary.p95_ms,
        prefix,
        summary.p99_ms,
        prefix,
        summary.max_ms,
        prefix,
        summary.mean_ms,
        prefix,
        summary.over_16_67,
        prefix,
        summary.over_33,
        prefix,
        summary.over_250,
        prefix,
        summary.samples
    )
end

---@param result table
function benchmark.emit(result)
    local pass, failures = benchmark.evaluate(result)
    print(
        string.format(
            "GC_BENCH_ENV|love=%s|gpu_name=%s|gpu_vendor=%s|gpu_device=%s|width=%d|height=%d|players=%d",
            result.love_version,
            result.gpu_name or "?",
            result.gpu_vendor or "?",
            result.gpu_device or "?",
            result.viewport.w,
            result.viewport.h,
            result.players
        )
    )
    -- The #393 phase fields ride at the END of the existing RESULT line: the
    -- browser harness's decoder reads pipe-delimited key=value pairs by name,
    -- so appended keys are backward-compatible where a reshaped line is not.
    print(
        string.format(
            "GC_BENCH_RESULT|renderer=%s|rigged_active=%s|seed=%d|warmup_frames=%d|measured_frames=%d|%s|%s|%s|draw_calls_mean=%.1f|draw_calls_max=%d|texture_memory=%d|lua_kb_warm=%.0f|lua_kb_end=%.0f|state_hash=%s|%s|%s",
            result.renderer,
            tostring(result.rigged_active),
            result.seed,
            result.warmup_frames,
            result.measured_frames,
            summary_fields(result.update, "update"),
            summary_fields(result.draw, "draw"),
            summary_fields(result.frame, "frame"),
            result.draw_calls_mean,
            result.draw_calls_max,
            result.texture_memory_bytes or 0,
            result.lua_memory_warm_kb or 0,
            result.lua_memory_end_kb or 0,
            result.final_state_hash or "?",
            summary_fields(result.scene_static, "scene_static"),
            summary_fields(result.scene_dynamic, "scene_dynamic")
        )
            .. string.format(
                "|static_cache=%s|static_cache_active=%s|pose_lod=%s",
                tostring(result.static_cache),
                tostring(result.static_cache_active),
                tostring(result.pose_lod)
            )
    )
    -- #394: the per-character split, on its own line rather than appended to
    -- RESULT -- the split only exists when profiling was requested, and the
    -- decoder reads pipe-delimited key=value pairs by name either way.
    if result.char_phases then
        print(
            string.format(
                "GC_BENCH_CHAR|renderer=%s|chars_per_frame=%.2f|%s|%s|%s|%s|%s",
                result.renderer,
                result.chars_per_frame_mean,
                summary_fields(result.char.pose, "char_pose"),
                summary_fields(result.char.apply, "char_apply"),
                summary_fields(result.char.rows, "char_rows"),
                summary_fields(result.char.submit, "char_submit"),
                summary_fields(result.char.other, "char_other")
            )
        )
    end
    -- Raw samples so a reviewer can recompute every percentile rather than
    -- trusting this file's arithmetic, which is what #100 means by reviewable.
    local sample_kinds = {
        update = result.raw_update_us,
        draw = result.raw_draw_us,
        frame = result.raw_frame_us,
        scene_static = result.raw_scene_static_us,
        scene_dynamic = result.raw_scene_dynamic_us,
    }
    if result.char_phases then
        for name, values in pairs(result.char.raw) do
            sample_kinds["char_" .. name] = values
        end
    end
    for kind, values in pairs(sample_kinds) do
        print(
            string.format(
                "GC_BENCH_SAMPLES|renderer=%s|kind=%s|unit=microseconds|samples=%s",
                result.renderer,
                kind,
                microseconds(values)
            )
        )
    end
    print(
        string.format(
            "GC_BENCH_GATE|renderer=%s|pass=%s|failures=%s",
            result.renderer,
            pass and "true" or "false",
            #failures > 0 and table.concat(failures, "; ") or ""
        )
    )
end

return benchmark
