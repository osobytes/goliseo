-- Entry point. Modes:
--   love .                    -> run the game (screen stack)
--   love . --test             -> run the headless test suite and exit with status code
--   love . --sim [n]          -> play n unattended matches, print fun-proxy metrics, exit
--   love . --capture-frames [frames] [warmup] [path] -> record a match as a file of
--                                RenderFrame payloads for an off-LOVE renderer (#341)
--   love . --snapshot-measure [n] -> measure canonical snapshot operations n times
--   love . --benchmark [frames] [warmup] [rigged|procedural] -> #100 ten-player
--                                render benchmark; windowed, needs a GL context
--   love . --gl-probe [canvas FORMAT READABLE | shader STEP] -> #360 GL capability
--                                readout; bare it surveys the runtime and creates
--                                nothing, otherwise it makes exactly one canvas or
--                                compiles exactly one shader rung and stops
--   love . --combat-feedback-fixture -> visually review the crowded #147 feedback fixture
--   love . --keeper-pose-snapshots [write] -> check or refresh #46 visual baselines
--   love . --outfield-pose-snapshots [write] -> check or refresh the #58 visual baseline
--   love . --rig3d-palette-snapshots [write|self-test] -> check/refresh the #337
--                                palette-slot visual baselines, or prove the compare
--                                itself can detect a colour regression
--   love . --rollback-lab [profile] [seed] [corrupt] -> run the OMP-2 lab
--   love . --rollback-validation SUITE [profile] [seed] -> gate OMP-2 evidence
--   love . --fault-harness [row|smoke|full] [net seed] [ticks] -> OMP-3 fault matrix
--                                (add `--topology relay` to run the same matrix over
--                                 the in-process relay instead of the direct-host star)
--   love . --determinism      -> verify the frozen OMP-1 complete-match evidence
--   love . --determinism-refresh -> deliberately replace the frozen OMP-1 recording
--   love . --rate-validate [n] -> validate frozen squad ratings over n paired seeds, exit
--   love . --levers [n]       -> paired liveness checks for built-in manager levers, exit
--   love . --sweep [n]        -> per-knob min/max sensitivity sweep over n seeds, exit
--   love . --search K1,K2 [n] [start] -> coordinate ascent over the named knobs
--                                (warm-started from tuning blob file `start`), exit
--   love . --eval FILE [n] [REF] -> tuning blob FILE vs REF blob (default: the
--                                defaults) on held-out seeds, exit
--   love . --tripwire [write] -> compare the fun signature against the checked-in
--                                baseline (exit 1 on drift); `write` refreshes it
--   love . --ai-baseline [write] -> verify the frozen combat-disabled Outfield AI
--                                common-seed baseline (exit 1 when it moved);
--                                `write` needs `--refreeze-ack` and re-freezes it

---@param a string
---@return boolean
local function has_flag(a)
    for _, v in ipairs(arg or {}) do
        if v == a then
            return true
        end
    end
    return false
end

-- Opt OUT of the rigged 3D player renderer, back to the procedural 2.5D one.
--
-- It used to be the other way round, because rigged players were waiting on the
-- ten-player benchmark. That benchmark has reported (see the note on
-- `pitch.rigged_players`), so rigged is the default and this flag is what keeps
-- the procedural renderer reachable -- which #100's before/after evidence needs,
-- and which anyone comparing the two by playing needs.
if has_flag("--procedural-players") then
    require("game.render.pitch").rigged_players = false
end

-- Opt into the broadcast-style following camera for a manual look. An optional
-- value tunes the zoom (`--follow-cam 2.0`) so framing can be judged by playing
-- rather than by editing a constant and restarting.
if has_flag("--follow-cam") then
    require("game.render.pitch").follow_camera = true
    -- A following camera only makes sense with real perspective: magnifying the
    -- fixed trapezoid flattens it the closer you get.
    require("game.render.camera").perspective_mode = true
    for index, value in ipairs(arg or {}) do
        if value == "--follow-cam" then
            local zoom = tonumber(arg[index + 1])
            if zoom then
                require("game.render.camera_follow").config.zoom = zoom
            end
        end
    end
end

-- Boot straight into a match. Purely a playtest convenience: menus are not what
-- anyone is trying to evaluate when looking at the renderer.
local quick_match = has_flag("--quick-match")

if has_flag("--test") then
    function love.load()
        local runner = require("spec.support.runner")
        runner.load_and_run("spec")
        os.exit(runner.summary() and 0 or 1)
    end
    return
end

if has_flag("--fault-harness") then
    function love.load()
        local runner = require("game.online.fault_campaign")
        ---@type string?, string?, string?
        local selector, seed, duration
        for index, value in ipairs(arg or {}) do
            if value == "--fault-harness" then
                selector, seed, duration = arg[index + 1], arg[index + 2], arg[index + 3]
                break
            end
        end
        ---@type string?
        local topology
        for index, value in ipairs(arg or {}) do
            if value == "--topology" then
                topology = arg[index + 1]
                break
            end
        end
        local ticks = tonumber(duration)
        local ok =
            runner.main(selector, tonumber(seed), ticks and math.floor(ticks) or nil, topology)
        os.exit(ok and 0 or 1)
    end
    return
end

-- Records a match as a file of `RenderFrame` payloads (#341). Headless: it
-- builds the payload and never draws it, which is the point -- the file is what
-- lets a renderer that does not exist yet be measured against this match.
--
--   love . --capture-frames [frames] [warmup] [path]
if has_flag("--capture-frames") then
    function love.load()
        local frames, warmup, path
        for index, value in ipairs(arg or {}) do
            if value == "--capture-frames" then
                frames, warmup, path = arg[index + 1], arg[index + 2], arg[index + 3]
                break
            end
        end
        local capture = require("scripts.capture_render_frames")
        os.exit(capture.main(frames, warmup, path) and 0 or 1)
    end
    return
end

-- Runs the Phase 0 portable-sim probe under LOVE's own Lua VM, so its hash can
-- be compared against the same script run on a bare interpreter. Identical
-- hashes are what prove the simulation is genuinely portable rather than merely
-- loadable elsewhere.
if has_flag("--phase0") then
    function love.load()
        dofile("scripts/phase0_sim_host.lua")
        os.exit(0)
    end
    return
end

if has_flag("--combat-feedback-fixture") then
    local fixture ---@type CombatFeedbackFixtureScreen
    local capture_path ---@type string?
    for index, value in ipairs(arg or {}) do
        if value == "--combat-feedback-fixture" then
            capture_path = arg[index + 1]
            break
        end
    end
    function love.load()
        fixture = require("game.screens.combat_feedback_fixture").new(capture_path)
    end
    function love.update(dt)
        fixture:update(dt)
    end
    function love.draw()
        fixture:draw()
    end
    function love.keypressed(key)
        if key == "escape" then
            love.event.quit()
        end
    end
    return
end

-- The #360 GL capability probe. Windowed for the same reason as the benchmark
-- below: it reads what the live GL context supports, which needs one.
--
--   love . --gl-probe                        -> survey; creates no canvas
--   love . --gl-probe canvas FORMAT READABLE -> create exactly that canvas, then stop
--   love . --gl-probe shader STEP            -> compile exactly that shader rung, then stop
--
-- Runs from love.draw rather than love.load on purpose: love.js initialises the
-- window later than the native runtime, and a probe that read the context before
-- it existed would report a capable runtime as incapable -- which is the exact
-- class of false negative this issue exists to correct.
if has_flag("--gl-probe") then
    local mode, first, second
    for index, value in ipairs(arg or {}) do
        if value == "--gl-probe" then
            mode, first, second = arg[index + 1], arg[index + 2], arg[index + 3]
            break
        end
    end
    function love.draw()
        local gl_probe = require("game.render.gl_probe")
        if mode == "canvas" then
            gl_probe.attemptOnce(first or "", second == "true")
        elseif mode == "shader" then
            gl_probe.shaderStep(first or "")
        else
            gl_probe.survey()
        end
        -- Exiting is right natively. Under love.js it is not: `os.exit` unwinds
        -- through emscripten and surfaces as `RuntimeError: unreachable` plus a
        -- page-level error event AFTER the terminator marker, so a probe that
        -- succeeded would leave a failed-looking page behind it. There the probe
        -- simply stops drawing and lets the controller close the browser.
        if love.system.getOS() == "Web" then
            function love.draw() end
            return
        end
        os.exit(0)
    end
    return
end

-- The #100 ten-player render benchmark. Windowed on purpose: it measures the
-- real match render path, so unlike every other harness above it needs a GL
-- context and cannot run through the headless path in conf.lua.
--
--   love . --benchmark [frames] [warmup] [rigged|procedural]
--
-- Vsync is turned off for the run. Left on, frame time is pinned to the refresh
-- rate and reports 16.67 ms whether the renderer has huge headroom or none.
if has_flag("--benchmark") then
    local fixture ---@type table
    local benchmark ---@type table
    local frames_arg, warmup, mode
    for index, value in ipairs(arg or {}) do
        if value == "--benchmark" then
            frames_arg, warmup, mode = arg[index + 1], arg[index + 2], arg[index + 3]
            break
        end
    end
    function love.load()
        -- Required here, not at file scope: love.js initialises the window
        -- later than the native runtime, and pulling in the render stack before
        -- it exists fails the boot before anything can report why.
        benchmark = require("game.render.benchmark")
        -- Best-effort: love.js does not always expose vsync control, and losing
        -- the setting costs measurement headroom, not correctness.
        pcall(function()
            love.window.setVSync(0)
        end)
        fixture = benchmark.new({
            frames = tonumber(frames_arg) or 3600,
            warmup_frames = tonumber(warmup) or 300,
            rigged = mode ~= "procedural",
        })
    end
    function love.update(dt)
        fixture:update(dt)
        if fixture:finished() then
            benchmark.emit(fixture:result())
            os.exit(0)
        end
    end
    function love.draw()
        fixture:draw()
    end
    function love.keypressed(key)
        if key == "escape" then
            love.event.quit()
        end
    end
    return
end

-- The (up to three) values following `flag`, e.g. `--search KNOBS [n] [start]`.
---@param flag string
---@return string?, string?, string?
local function args_after(flag)
    for i, v in ipairs(arg or {}) do
        if v == flag then
            return arg[i + 1], arg[i + 2], arg[i + 3]
        end
    end
    return nil, nil, nil
end

if has_flag("--keeper-pose-snapshots") then
    function love.load()
        local mode = args_after("--keeper-pose-snapshots")
        local ok, report = require("spec.support.keeper_pose_snapshots").run(mode == "write")
        print(report)
        os.exit(ok and 0 or 1)
    end
    return
end

if has_flag("--outfield-pose-snapshots") then
    function love.load()
        local mode = args_after("--outfield-pose-snapshots")
        local ok, report = require("spec.support.outfield_pose_snapshots").run(mode == "write")
        print(report)
        os.exit(ok and 0 or 1)
    end
    return
end

if has_flag("--rig3d-palette-snapshots") then
    function love.load()
        local mode = args_after("--rig3d-palette-snapshots") or "check"
        local ok, report = require("spec.support.rig3d_palette_snapshots").run(mode)
        print(report)
        os.exit(ok and 0 or 1)
    end
    return
end

if has_flag("--sim") then
    function love.load()
        local n_arg = args_after("--sim")
        local n = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 20
        local headless = require("sim.headless")
        print(headless.report(headless.run_batch({ n = n })))
        os.exit(0)
    end
    return
end

if has_flag("--snapshot-measure") then
    function love.load()
        local n_arg = args_after("--snapshot-measure")
        local iterations = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 1000
        assert(iterations > 0, "--snapshot-measure needs a positive iteration count")
        local fixed_clock = require("sim.fixed_clock")
        local input_frame = require("sim.input_frame")
        local sim_match = require("sim.match")
        local match_snapshot = require("sim.match_snapshot")
        local teams = require("data.teams")
        local ownership = sim_match.ownership_for_teams(teams.nebula, teams.orion)
        local state = sim_match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 120,
            max_goals = sim_match.NO_GOAL_LIMIT,
            seed = 38,
            input_ownership = ownership,
        })
        for tick = 0, 119 do
            sim_match.step(state, fixed_clock.TICK_SECONDS, assert(input_frame.neutral(tick)))
        end
        local snapshot = match_snapshot.capture(state)
        local encoded = match_snapshot.encode(snapshot)

        local started = os.clock()
        for _ = 1, iterations do
            encoded = match_snapshot.encode(snapshot)
        end
        local encode_ms = (os.clock() - started) * 1000

        local hash = ""
        started = os.clock()
        for _ = 1, iterations do
            hash = match_snapshot.hash(snapshot)
        end
        local hash_ms = (os.clock() - started) * 1000

        local restored = state
        started = os.clock()
        for _ = 1, iterations do
            restored = match_snapshot.restore(snapshot)
        end
        local restore_ms = (os.clock() - started) * 1000
        print(
            ("snapshot_measure version=%d tick=%d bytes=%d iterations=%d hash=%s"):format(
                snapshot.version,
                restored.input_tick,
                #encoded,
                iterations,
                hash
            )
        )
        print(
            ("snapshot_measure encode_ms_total=%.3f encode_us_each=%.3f"):format(
                encode_ms,
                encode_ms * 1000 / iterations
            )
        )
        print(
            ("snapshot_measure hash_with_encode_ms_total=%.3f hash_with_encode_us_each=%.3f"):format(
                hash_ms,
                hash_ms * 1000 / iterations
            )
        )
        print(
            ("snapshot_measure restore_ms_total=%.3f restore_us_each=%.3f"):format(
                restore_ms,
                restore_ms * 1000 / iterations
            )
        )
        os.exit(0)
    end
    return
end

if has_flag("--rollback-validation") then
    local rollback_validation = require("sim.rollback_validation")
    local game_validation = require("game.rollback_validation")
    local match_snapshot = require("sim.match_snapshot")
    local validation_config = rollback_validation.config()
    local suite_arg, profile_arg, seed_arg = args_after("--rollback-validation")
    local suite = suite_arg or "native"
    local browser_runtime = has_flag("--browser-runtime")
    local external_sample_ack = has_flag("--external-sample-ack")
    ---@cast suite RollbackValidationSuite
    local network_seed = nil
    if seed_arg ~= nil then
        network_seed = tonumber(seed_arg)
        assert(
            network_seed
                and network_seed == math.floor(network_seed)
                and network_seed == network_seed
                and network_seed ~= math.huge
                and network_seed ~= -math.huge,
            "--rollback-validation network seed must be a finite integer"
        )
        ---@cast network_seed integer
    end

    ---@class RuntimeTimingRow
    ---@field seconds number
    ---@field calls integer
    ---@field maximum number

    ---@class RuntimeCaseTiming
    ---@field tick RuntimeTimingRow
    ---@field capture RuntimeTimingRow
    ---@field restore RuntimeTimingRow
    ---@field resimulation RuntimeTimingRow
    ---@field rollback RuntimeTimingRow
    ---@field rollback_microseconds integer[]
    ---@field work_seconds number[]
    ---@field update_wall_seconds number[]

    ---@return RuntimeCaseTiming
    local function new_case_timing()
        return {
            tick = { seconds = 0, calls = 0, maximum = 0 },
            capture = { seconds = 0, calls = 0, maximum = 0 },
            restore = { seconds = 0, calls = 0, maximum = 0 },
            resimulation = { seconds = 0, calls = 0, maximum = 0 },
            rollback = { seconds = 0, calls = 0, maximum = 0 },
            rollback_microseconds = {},
            work_seconds = {},
            update_wall_seconds = {},
        }
    end

    local active_timing = new_case_timing()
    local current_work_seconds = 0

    ---@param label RollbackSessionMeasureLabel
    ---@param operation fun(): any
    ---@return any
    local function measure(label, operation)
        local started = love.timer.getTime()
        local value = operation()
        local elapsed = love.timer.getTime() - started
        local row = active_timing[label]
        row.seconds = row.seconds + elapsed
        row.calls = row.calls + 1
        row.maximum = math.max(row.maximum, elapsed)
        if label == "rollback" then
            active_timing.rollback_microseconds[#active_timing.rollback_microseconds + 1] =
                math.floor(elapsed * 1000000 + 0.5)
        end
        if label == "tick" or label == "rollback" then
            current_work_seconds = current_work_seconds + elapsed
        end
        return value
    end

    local campaign = rollback_validation.new_campaign(suite, {
        profile_name = profile_arg,
        network_seed = network_seed,
        measure = measure,
    })
    local runtime_failed = false

    ---@param values number[]
    ---@param percentile number
    ---@return number
    local function nearest_rank(values, percentile)
        if #values == 0 then
            return 0
        end
        local copied = {}
        for index, value in ipairs(values) do
            copied[index] = value
        end
        table.sort(copied)
        return copied[math.max(1, math.ceil(#copied * percentile))]
    end

    ---@param scenario string
    ---@return RollbackValidationScenario[]
    local function required_game_scenarios(scenario)
        if scenario == "complete_fixture" then
            return { "possession", "tackle", "shot", "aerial", "keeper", "full_time" }
        end
        local mapped = {
            possession_change = "possession",
            tackle = "tackle",
            shot = "shot",
            goal = "goal",
            kickoff = "kickoff",
            aerial = "aerial",
            keeper_action = "keeper",
            full_time = "full_time",
        }
        local value = mapped[scenario]
        return value and { value } or {}
    end

    ---@param completed RollbackValidationCompletedCase
    ---@return string
    ---@return string
    ---@return string?
    ---@return boolean
    local function complete_case(completed)
        local result = completed.result
        local game_pass = completed.expected_failure
        if not completed.expected_failure then
            local initial, initial_combat = match_snapshot.restore(completed.initial_snapshot)
            local reference_final_state = match_snapshot.restore(result.reference_final_snapshot)
            local impaired_final_state = match_snapshot.restore(result.client_final_snapshot)
            local report = game_validation.run(initial, result.event_trace, {
                home_team_id = "nebula",
                away_team_id = "orion",
                initial_combat_state = initial_combat,
                reference_final_state = reference_final_state,
                impaired_final_state = impaired_final_state,
                seed = result.fixture_seed,
                expected_replay_boundaries = game_validation.expected_replay_boundaries(
                    result.reference_final_boundary
                ),
                expected_replay_samples = {
                    {
                        boundary = result.reference_final_boundary,
                        ball_x = reference_final_state.ball.x,
                        ball_y = reference_final_state.ball.y,
                        score_home = reference_final_state.score.home,
                        score_away = reference_final_state.score.away,
                    },
                },
                expected_replay_truncate_count = result.metrics.rollback_count,
                required_scenarios = required_game_scenarios(completed.scenario),
            })
            game_pass = report.passed
        end

        local p95_work_ms = nearest_rank(active_timing.work_seconds, 0.95) * 1000
        local p95_update_wall_ms = nearest_rank(active_timing.update_wall_seconds, 0.95) * 1000
        local max_update_wall_ms = 0
        for _, value in ipairs(active_timing.update_wall_seconds) do
            max_update_wall_ms = math.max(max_update_wall_ms, value * 1000)
        end
        local rollback_p999_ms = nearest_rank(active_timing.rollback_microseconds, 0.999) / 1000
        local max_rollback_ms = nearest_rank(active_timing.rollback_microseconds, 1) / 1000
        local rollback_over_budget_count = 0
        for _, microseconds in ipairs(active_timing.rollback_microseconds) do
            if microseconds >= validation_config.budgets.rollback_p999_ms * 1000 then
                rollback_over_budget_count = rollback_over_budget_count + 1
            end
        end
        local cpu_gate_mode = "diagnostic"
        if result.profile == "playable" and suite ~= "soak" then
            cpu_gate_mode = browser_runtime and "normalized_deferred" or "absolute"
        end
        local cpu_gate_applied = cpu_gate_mode == "absolute"
        local cpu_gate = cpu_gate_applied
                and (p95_work_ms < validation_config.budgets.p95_work_ms and rollback_p999_ms < validation_config.budgets.rollback_p999_ms)
            or not cpu_gate_applied
        local snapshot_gate = result.metrics.peaks.snapshot_count
                <= validation_config.budgets.snapshot_count
            and result.metrics.peaks.snapshot_bytes < validation_config.budgets.snapshot_bytes
        local history_gate = result.metrics.peaks.history_bytes
            < validation_config.budgets.history_bytes
        local passed = completed.accepted
            and game_pass
            and cpu_gate
            and snapshot_gate
            and history_gate
        runtime_failed = runtime_failed or not passed

        local logical = rollback_validation.case_marker(completed)
            .. "|gate_contract=7"
            .. "|cpu_gate="
            .. (cpu_gate_applied and (cpu_gate and "1" or "0") or (cpu_gate_mode == "normalized_deferred" and "deferred" or "not_applied"))
            .. "|cpu_gate_applied="
            .. (cpu_gate_applied and "1" or "0")
            .. "|cpu_gate_mode="
            .. cpu_gate_mode
            .. "|snapshot_gate="
            .. (snapshot_gate and "1" or "0")
            .. "|history_gate="
            .. (history_gate and "1" or "0")
            .. "|game_gate="
            .. (game_pass and "1" or "0")
        local metrics = table.concat({
            "GC_ROLLBACK_METRICS",
            "case",
            "case=" .. completed.id,
            "profile=" .. result.profile,
            ("p95_work_ms=%.6f"):format(p95_work_ms),
            ("rollback_p999_ms=%.6f"):format(rollback_p999_ms),
            ("max_rollback_ms=%.6f"):format(max_rollback_ms),
            "rollback_sample_count=" .. #active_timing.rollback_microseconds,
            "rollback_over_33_3_count=" .. rollback_over_budget_count,
            "rollback_percentile=0.999",
            "rollback_percentile_method=nearest_rank",
            "rollback_timing_evidence=" .. (suite == "soak" and "aggregate_diagnostic" or "raw"),
            ("p95_update_wall_ms=%.6f"):format(p95_update_wall_ms),
            ("max_update_wall_ms=%.6f"):format(max_update_wall_ms),
            ("simulation_ms=%.6f"):format(active_timing.tick.seconds * 1000),
            ("capture_ms=%.6f"):format(active_timing.capture.seconds * 1000),
            ("restore_ms=%.6f"):format(active_timing.restore.seconds * 1000),
            ("resimulation_ms=%.6f"):format(active_timing.resimulation.seconds * 1000),
            ("rollback_ms=%.6f"):format(active_timing.rollback.seconds * 1000),
            "capture_calls=" .. active_timing.capture.calls,
            "simulation_calls=" .. active_timing.tick.calls,
            "restore_calls=" .. active_timing.restore.calls,
            "resimulation_calls=" .. active_timing.resimulation.calls,
            "rollback_calls=" .. active_timing.rollback.calls,
            "work_samples=" .. #active_timing.work_seconds,
            "peak_snapshot_bytes=" .. result.metrics.peaks.snapshot_bytes,
            "peak_history_bytes=" .. result.metrics.peaks.history_bytes,
        }, "|")
        local timings = nil
        if suite ~= "soak" and #active_timing.rollback_microseconds > 0 then
            local samples = {}
            for index, microseconds in ipairs(active_timing.rollback_microseconds) do
                samples[index] = tostring(microseconds)
            end
            timings = table.concat({
                "GC_ROLLBACK_TIMINGS",
                "case",
                "gate_contract=7",
                "case=" .. completed.id,
                "sample_count=" .. #samples,
                "unit=microseconds",
                "samples=" .. table.concat(samples, ","),
            }, "|")
        end
        return logical, metrics, timings, passed
    end

    local function flush_stdout()
        if io.stdout and io.stdout.flush then
            io.stdout:flush()
        end
    end

    local function runtime_marker()
        local major, minor, revision = love.getVersion()
        print(table.concat({
            "GC_ROLLBACK_METRICS",
            "runtime",
            "love=" .. major .. "." .. minor .. "." .. revision,
            "suite=" .. suite,
            "gate_contract=7",
            "profile_digest=" .. rollback_validation.profile_digest(),
            "input_version=2",
            "tape_versions=1,2",
            "snapshot_versions=" .. match_snapshot.VERSION .. "," .. match_snapshot.COMBAT_VERSION,
            "tick_rate=60",
        }, "|"))
        flush_stdout()
    end

    ---@return RollbackValidationResult?
    local function advance()
        current_work_seconds = 0
        local started = love.timer.getTime()
        local result, completed = rollback_validation.step_campaign(campaign, 1)
        local wall_seconds = love.timer.getTime() - started
        active_timing.work_seconds[#active_timing.work_seconds + 1] = current_work_seconds
        active_timing.update_wall_seconds[#active_timing.update_wall_seconds + 1] = wall_seconds
        if completed then
            local logical, metrics, timings = complete_case(completed)
            local sample = completed.sample
            local soak_digest = completed.result.event_metrics.confirmed_digest
            completed = nil
            active_timing = new_case_timing()
            if timings ~= nil then
                print(timings)
                timings = nil
            end
            if sample ~= nil then
                collectgarbage("collect")
                local heap_bytes = math.floor(collectgarbage("count") * 1024 + 0.5)
                logical = logical
                    .. "|forced_gc=1|lua_heap_bytes="
                    .. heap_bytes
                    .. "|logical_digest="
                    .. soak_digest
            end
            print(logical)
            print(metrics)
            flush_stdout()
            if sample == "final" and external_sample_ack then
                assert(
                    io.read("*l") == "GC_ROLLBACK_SAMPLE_ACK",
                    "rollback validation did not receive its external sample acknowledgement"
                )
            end
        end
        return result
    end

    ---@param result RollbackValidationResult
    local function finish(result)
        local marker = rollback_validation.result_marker({
            schema = result.schema,
            suite = result.suite,
            success = result.success and not runtime_failed,
            case_count = result.case_count,
            logical_digest = result.logical_digest,
        })
        print(marker)
        flush_stdout()
    end

    if browser_runtime then
        function love.load()
            runtime_marker()
        end

        function love.update()
            local ok, result = pcall(advance)
            if not ok then
                print("GC_ROLLBACK_VALIDATION|failure|message=" .. tostring(result):gsub("|", "/"))
                flush_stdout()
                love.event.quit(1)
            elseif result then
                finish(result)
                love.event.quit(result.success and not runtime_failed and 0 or 1)
            end
        end
    else
        function love.load()
            runtime_marker()
            local ok, result = pcall(function()
                local completed_result = nil
                while completed_result == nil do
                    completed_result = advance()
                end
                return completed_result
            end)
            if not ok then
                print("GC_ROLLBACK_VALIDATION|failure|message=" .. tostring(result):gsub("|", "/"))
                flush_stdout()
                os.exit(1)
            end
            ---@cast result RollbackValidationResult
            finish(result)
            os.exit(result.success and not runtime_failed and 0 or 1)
        end
    end
    return
end

if has_flag("--rollback-lab") then
    function love.load()
        local profile_arg, seed_arg, corruption_arg = args_after("--rollback-lab")
        local profile = profile_arg or "omp0_parity"
        local seed = 7302
        if seed_arg ~= nil then
            local parsed = tonumber(seed_arg)
            assert(
                parsed
                    and parsed == parsed
                    and parsed ~= math.huge
                    and parsed ~= -math.huge
                    and parsed == math.floor(parsed),
                "--rollback-lab seed must be a finite integer"
            )
            ---@cast parsed integer
            seed = parsed
        end
        assert(
            corruption_arg == nil or corruption_arg == "corrupt",
            "--rollback-lab third argument must be 'corrupt' when supplied"
        )
        local evidence = require("sim.determinism_evidence")
        local rollback_lab = require("sim.rollback_lab")
        local timing = {
            capture = { seconds = 0, calls = 0 },
            restore = { seconds = 0, calls = 0 },
            resimulation = { seconds = 0, calls = 0 },
            rollback = { seconds = 0, calls = 0 },
        }
        ---@param label RollbackSessionMeasureLabel
        ---@param operation fun(): any
        ---@return any
        local function measure(label, operation)
            local started = love.timer.getTime()
            local value = operation()
            local row = timing[label]
            row.seconds = row.seconds + love.timer.getTime() - started
            row.calls = row.calls + 1
            return value
        end

        local tape = evidence.fixture_tape()
        local started = love.timer.getTime()
        local result = rollback_lab.run(tape, {
            profile_name = profile,
            network_seed = seed,
            corruption = corruption_arg == "corrupt" and { tick = 24, slot = 5 } or nil,
            measure = measure,
        })
        local total_seconds = love.timer.getTime() - started
        print(rollback_lab.logical_marker(result))
        print(rollback_lab.summary(result))
        print(table.concat({
            "GC_ROLLBACK_LAB",
            "timing",
            "schema=1",
            ("capture_ms=%.3f"):format(timing.capture.seconds * 1000),
            "capture_calls=" .. timing.capture.calls,
            ("restore_ms=%.3f"):format(timing.restore.seconds * 1000),
            "restore_calls=" .. timing.restore.calls,
            ("resimulation_ms=%.3f"):format(timing.resimulation.seconds * 1000),
            "resimulation_calls=" .. timing.resimulation.calls,
            ("rollback_ms=%.3f"):format(timing.rollback.seconds * 1000),
            "rollback_calls=" .. timing.rollback.calls,
            ("total_ms=%.3f"):format(total_seconds * 1000),
        }, "|"))
        os.exit(result.success and 0 or 1)
    end
    return
end

if has_flag("--determinism-refresh") then
    function love.load()
        local evidence = require("sim.determinism_evidence")
        local recording = evidence.record()
        -- Validate and serialize before opening the checked-in fixture. A
        -- coverage assertion must never truncate the last known-good evidence.
        local payload = evidence.serialize_recording(recording)
        local path = "data/omp1_determinism.lua"
        local file = assert(io.open(path, "w"))
        file:write(payload)
        file:close()
        print(
            ("determinism fixture refreshed: %s (%d frames, %d boundaries)"):format(
                path,
                #recording.frame_wires,
                #recording.boundary_hashes
            )
        )
        os.exit(0)
    end
    return
end

if has_flag("--determinism") then
    local evidence = require("sim.determinism_evidence")
    local input_protocol_conformance = require("game.online.input_protocol_conformance")
    local protocol_conformance = require("game.online.protocol_conformance")
    ---@type DeterminismCampaign?
    local browser_campaign

    ---@param result DeterminismEvidenceResult
    ---@return string
    local function runtime_report(result)
        local major, minor, revision = love.getVersion()
        return evidence.report(result) .. ("|love=%d.%d.%d"):format(major, minor, revision)
    end

    ---@return boolean
    local function verify_protocol_goldens()
        local protocol_ok, protocol_result = pcall(protocol_conformance.verify)
        if not protocol_ok then
            print(
                "GC_DETERMINISM|failure|message=protocol_golden_"
                    .. tostring(protocol_result):gsub("|", "/")
            )
            return false
        end
        print(protocol_conformance.marker(protocol_result))
        local input_ok, input_result = pcall(input_protocol_conformance.verify)
        if not input_ok then
            print(
                "GC_DETERMINISM|failure|message=input_protocol_golden_"
                    .. tostring(input_result):gsub("|", "/")
            )
            return false
        end
        print(input_protocol_conformance.marker(input_result))
        return true
    end

    local function run_native_determinism()
        if not verify_protocol_goldens() then
            os.exit(1)
        end
        local ok, result = pcall(evidence.verify)
        print(
            ok and runtime_report(result)
                or ("GC_DETERMINISM|failure|message=" .. tostring(result):gsub("|", "/"))
        )
        os.exit(ok and 0 or 1)
    end

    function love.load()
        if has_flag("--browser-runtime") then
            if not verify_protocol_goldens() then
                love.event.quit(1)
                return
            end
            local ok, campaign = pcall(evidence.new_campaign, false)
            if ok then
                browser_campaign = campaign
            else
                print("GC_DETERMINISM|failure|message=" .. tostring(campaign):gsub("|", "/"))
                love.event.quit(1)
            end
        else
            run_native_determinism()
        end
    end

    function love.update()
        if browser_campaign then
            local ok, result = pcall(evidence.step_campaign, browser_campaign, 30)
            if not ok then
                print("GC_DETERMINISM|failure|message=" .. tostring(result):gsub("|", "/"))
                browser_campaign = nil
                love.event.quit(1)
            elseif result then
                print(runtime_report(result))
                browser_campaign = nil
                love.event.quit(0)
            end
        end
    end

    function love.draw() end

    return
end

if has_flag("--tripwire") then
    function love.load()
        local sub = args_after("--tripwire")
        local tripwire = require("sim.tripwire")
        if sub == "write" then
            local current, n = tripwire.measure()
            local f = assert(io.open("data/fun_baseline.lua", "w"))
            f:write(tripwire.serialize(current, n))
            f:close()
            print("fun baseline refreshed: data/fun_baseline.lua (" .. n .. " seeds)")
            os.exit(0)
        end
        local ok_load, baseline = pcall(require, "data.fun_baseline")
        if not ok_load or type(baseline) ~= "table" then
            print("no baseline: data/fun_baseline.lua missing — create it with:")
            print("    love . --tripwire write")
            os.exit(1)
        end
        local current, n = tripwire.measure(baseline.n)
        local ok, rows = tripwire.compare(baseline, current)
        print(tripwire.report(rows, ok, n))
        os.exit(ok and 0 or 1)
    end
    return
end

-- #59: the frozen combat-disabled Outfield AI control that #148/#149 cite.
-- Verification is the default and writing is deliberately awkward — the whole
-- point of the artifact is that it cannot be silently refreshed.
if has_flag("--ai-baseline") then
    function love.load()
        local sub = args_after("--ai-baseline")
        local baseline = require("sim.outfield_ai_baseline")
        local path = "data/outfield_ai_baseline.lua"
        local ok_load, frozen = pcall(require, "data.outfield_ai_baseline")
        local have_frozen = ok_load and type(frozen) == "table"
        if sub == "write" then
            if not has_flag("--refreeze-ack") then
                print("refusing to re-freeze " .. path .. " without an explicit acknowledgement.")
                print("#148/#149 cite this baseline; refreshing it deletes the evidence that")
                print("the control moved. If the change is intended, log it in the drift log")
                print("of docs/design/fun_metrics.md and re-run with:")
                print("    love . --ai-baseline write --refreeze-ack")
                os.exit(2)
            end
            local previous = have_frozen and frozen.baseline_version or 0
            local record = baseline.measure({ baseline_version = previous + 1 })
            local file = assert(io.open(path, "w"))
            file:write(baseline.serialize(record))
            file:close()
            print(
                ("outfield AI baseline re-frozen: %s (v%d, %d seeds, policy %s)"):format(
                    path,
                    record.baseline_version,
                    baseline.SEED_COUNT,
                    record.identity.policy_id
                )
            )
            os.exit(0)
        end
        if not have_frozen then
            print("no baseline: " .. path .. " missing — create it with:")
            print("    love . --ai-baseline write --refreeze-ack")
            os.exit(1)
        end
        local current = baseline.measure({ baseline_version = frozen.baseline_version })
        local comparison = baseline.compare(frozen, current)
        print(baseline.report(comparison, frozen, current))
        os.exit(comparison.ok and 0 or 1)
    end
    return
end

if has_flag("--rate-validate") then
    function love.load()
        local n_arg = args_after("--rate-validate")
        local n = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 20
        local validation = require("sim.rating_validation")
        print(validation.report(validation.run(n)))
        os.exit(0)
    end
    return
end

if has_flag("--levers") then
    function love.load()
        local n_arg = args_after("--levers")
        local n = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 30
        assert(n > 0, "--levers needs a positive seed count")
        local lever_metrics = require("sim.lever_metrics")
        local seeds = {}
        for i = 1, n do
            seeds[i] = i
        end
        local config_name, runs = lever_metrics.run_built_ins(seeds, print)
        print(lever_metrics.report(config_name, runs))
        os.exit(0)
    end
    return
end

if has_flag("--sweep") then
    function love.load()
        local n_arg = args_after("--sweep")
        local n = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 30
        local sweep = require("sim.sweep")
        local seeds = {}
        for i = 1, n do
            seeds[i] = i
        end
        local result = sweep.sensitivity({ seeds = seeds, log = print })
        print(sweep.sensitivity_report(result))
        os.exit(0)
    end
    return
end

if has_flag("--search") then
    function love.load()
        local keys_arg, n_arg, start_arg = args_after("--search")
        assert(keys_arg, "--search needs a comma-separated knob list")
        local keys = {}
        for key in keys_arg:gmatch("[%w_]+") do
            keys[#keys + 1] = key
        end
        local n = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 30
        local sweep = require("sim.sweep")
        local headless = require("sim.headless")
        local start = nil
        if start_arg then
            local f = assert(io.open(start_arg, "r"), "cannot open " .. start_arg)
            start = sweep.parse_blob(f:read("*a"))
            f:close()
        end
        local seeds = {}
        for i = 1, n do
            seeds[i] = i
        end
        local r = sweep.ascend({ keys = keys, seeds = seeds, start = start, log = print })
        print(("best blob (dFun %+.3f +/- %.3f on search seeds):"):format(r.delta.mean, r.delta.se))
        print(r.blob ~= "" and r.blob or "(defaults)")
        print(headless.report(headless.run_batch({ seeds = seeds, tuning_blob = r.blob })))
        os.exit(0)
    end
    return
end

if has_flag("--eval") then
    function love.load()
        local path, n_arg, ref_arg = args_after("--eval")
        assert(path, "--eval needs a tuning blob file path")
        local function slurp(p)
            local f = assert(io.open(p, "r"), "cannot open " .. tostring(p))
            local blob = f:read("*a")
            f:close()
            return blob
        end
        local blob = slurp(path)
        local ref = ref_arg and slurp(ref_arg) or ""
        local n = tonumber(n_arg) and math.floor(tonumber(n_arg) --[[@as number]]) or 60
        local sweep = require("sim.sweep")
        local headless = require("sim.headless")
        -- Held-out seeds, disjoint from the 1..N the sweep/search train on.
        local seeds = {}
        for i = 1, n do
            seeds[i] = 1000 + i
        end
        local base = sweep.evaluate(ref, seeds)
        local cand = sweep.evaluate(blob, seeds)
        local d = sweep.paired_delta(base.funs, cand.funs)
        print(
            ("held-out seeds %d..%d: dFun %+.3f +/- %.3f (paired, vs %s)"):format(
                seeds[1],
                seeds[n],
                d.mean,
                d.se,
                ref_arg or "defaults"
            )
        )
        print("--- reference ---")
        print(headless.report(headless.run_batch({ seeds = seeds, tuning_blob = ref })))
        print("--- candidate ---")
        print(headless.report(headless.run_batch({ seeds = seeds, tuning_blob = blob })))
        os.exit(0)
    end
    return
end

local bootstrap = require("game.bootstrap")
local compatibility_metrics = require("game.compatibility_metrics")
local runtime_settings = require("game.runtime_settings")
local CompatibilityFlow = require("game.compatibility_flow")

---@type App
local app
---@type CompatibilityMetrics
local metrics
local last_route
---@type CompatibilityFlow?
local compatibility_flow
local compatibility_audio_probe_next_at
local compatibility_audio_probe_observations = 0
local compatibility_audio_probe_started = false

local COMPATIBILITY_AUDIO_PROBE_INTERVAL_SECONDS = 0.5
local COMPATIBILITY_AUDIO_PROBE_MAX_OBSERVATIONS = 11

---@return number
local function clock()
    return love.timer.getTime()
end

---@param kind string
local function record_input(kind)
    if metrics then
        metrics:input(clock(), kind)
    end
end

---@param settings GameSettings
local function apply_settings(settings)
    runtime_settings.apply(settings)
    metrics:settings(clock(), settings)
end

---@param now number
local function record_audio(now)
    if love.audio and love.audio.getActiveSourceCount and love.audio.getVolume then
        metrics:audio(now, love.audio.getActiveSourceCount(), love.audio.getVolume())
    end
end

---@param now number
local function start_compatibility_audio_probe(now)
    if compatibility_audio_probe_started then
        return
    end
    compatibility_audio_probe_started = true
    compatibility_audio_probe_next_at = now
    compatibility_audio_probe_observations = 0
end

---@param now number
local function update_compatibility_audio_probe(now)
    if not compatibility_audio_probe_next_at or now < compatibility_audio_probe_next_at then
        return
    end
    record_audio(now)
    compatibility_audio_probe_observations = compatibility_audio_probe_observations + 1
    if compatibility_audio_probe_observations >= COMPATIBILITY_AUDIO_PROBE_MAX_OBSERVATIONS then
        compatibility_audio_probe_next_at = nil
        return
    end
    compatibility_audio_probe_next_at = compatibility_audio_probe_next_at
        + COMPATIBILITY_AUDIO_PROBE_INTERVAL_SECONDS
end

function love.load()
    metrics = compatibility_metrics.new(clock())
    local width, height = love.graphics.getDimensions()
    app = bootstrap.new(width, height, {
        quick_match = quick_match,
        apply_settings = apply_settings,
        request_quit = function()
            love.event.quit()
        end,
    })
    apply_settings(app.settings)
    app:resize(love.graphics.getDimensions())
    last_route = app:current_route()
    metrics:route(clock(), last_route)
    if has_flag("--compat-flow") then
        compatibility_flow = CompatibilityFlow.new(record_input)
    end
end

---@param dt number
function love.update(dt)
    metrics:begin_update(clock())
    if compatibility_flow then
        compatibility_flow:update(app, clock())
    end
    app:update(dt)
    local now = clock()
    metrics:finish_update(now)
    local route = app:current_route()
    if route ~= last_route then
        metrics:route(now, route)
        last_route = route
        if route == "match" and compatibility_flow then
            start_compatibility_audio_probe(now)
        elseif route == "result" then
            metrics:flow_complete(now, route)
        end
    end
    update_compatibility_audio_probe(now)
end

function love.draw()
    metrics:begin_draw(clock())
    app:draw()
    metrics:finish_draw(clock())
end

function love.quit()
    if metrics then
        metrics:finish(clock())
    end
end

---@param key string
function love.keypressed(key)
    record_input("key_" .. key)
    app:event({ kind = "key", key = key, pressed = true })
end

---@param key string
function love.keyreleased(key)
    app:event({ kind = "key", key = key, pressed = false })
end

---@param x number
---@param y number
---@param button number
function love.mousepressed(x, y, button)
    record_input("mouse_" .. button)
    app:event({ kind = "click", x = x, y = y, button = button })
end

---@param joystick love.Joystick
---@param button love.GamepadButton
function love.gamepadpressed(joystick, button)
    local _ = joystick
    record_input("gamepad_" .. button)
    app:event({ kind = "gamepad", button = button, pressed = true })
end

---@param joystick love.Joystick
---@param button love.GamepadButton
function love.gamepadreleased(joystick, button)
    local _ = joystick
    app:event({ kind = "gamepad", button = button, pressed = false })
end

---@param width number
---@param height number
function love.resize(width, height)
    metrics:lifecycle(clock(), "resize")
    app:resize(width, height)
end

---@param focused boolean
function love.focus(focused)
    metrics:lifecycle(clock(), focused and "focus" or "blur")
    app:focus(focused)
end

---@param joystick love.Joystick
function love.joystickremoved(joystick)
    local _ = joystick
    metrics:lifecycle(clock(), "joystick_removed")
    app:controller_removed()
end
