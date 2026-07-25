local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local teams = require("data.teams")
local audio = require("game.audio")
local contract = require("game.match_contract")
local match_observer = require("game.match_observer")
local input_frame = require("sim.input_frame")
local fixed_clock = require("sim.fixed_clock")
local combat = require("sim.combat")
local sim_match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_playable_lab = require("sim.rollback_playable_lab")
local Match = require("game.screens.match")
local RealMatch = require("game.screens.real_match")
local ScreenStack = require("game.screen_stack")
local bloom = require("game.render.bloom")
local correction_smoothing = require("game.render.correction_smoothing")
local replay = require("game.render.replay")
local view_state = require("game.render.view_state")
local tuning_panel = require("game.ui.tuning_panel")

---@param delay integer
---@return NetworkProfile
local function fixed_profile(delay)
    return {
        base_delay_ticks = delay,
        jitter_min_ticks = 0,
        jitter_max_ticks = 0,
        independent_loss_rate = 0,
        duplication_rate = 0,
        burst_start_rate = 0,
        burst_length_ticks = 0,
    }
end

---@param fn fun(down: table<string, boolean>)
local function with_keyboard(fn)
    local saved = love.keyboard
    local down = {}
    love.keyboard = {
        isDown = function(...)
            for _, key in ipairs({ ... }) do
                if down[key] then
                    return true
                end
            end
            return false
        end,
    }
    local ok, err = pcall(fn, down)
    love.keyboard = saved
    assert(ok, err)
end

---@return MatchSnapshot
local function rollback_goal_fixture()
    local state = sim_match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 2,
        max_goals = 2,
        input_ownership = sim_match.ownership_for_teams(teams.nebula, teams.orion),
    })
    for _, player in ipairs(state.players) do
        player.pos = Vec2.new(180, 40)
        player.run_vel = Vec2.new(0, 0)
    end
    state.owner = nil
    state.ball = Vec2.new(350, 270)
    state.ball_vel = Vec2.new(1200, 0)
    state.ball_z = 10
    state.ball_vz = 200
    state.pickup_cd = 999
    state.block_grace = 999
    return match_snapshot.capture(state)
end

---@param screen MatchScreen
local function seed_render_correction(screen)
    local previous = match_snapshot.restore(match_snapshot.capture(screen.state))
    local player = previous.players[1]
    player.pos = Vec2.new(player.pos.x - 40, player.pos.y)
    previous.ball = Vec2.new(previous.ball.x - 20, previous.ball.y)
    screen._render_smoothing = correction_smoothing.new(previous)
    screen._render_smoothing = correction_smoothing.correct(screen._render_smoothing, screen.state)
    screen._render_pose = correction_smoothing.pose(screen._render_smoothing)
    t.is_true(correction_smoothing.diagnostics(screen._render_smoothing).active_count > 0)
end

---@param screen MatchScreen
local function prepare_forced_home_goal(screen)
    for _, player in ipairs(screen.state.players) do
        player.pos = Vec2.new(player.pos.x, 50)
    end
    screen.state.owner = nil
    screen.state.pickup_cd = 1
    screen.state.block_grace = 1
    screen.state.ball = Vec2.new(screen.state.field.w - 7, screen.state.field.h / 2)
    screen.state.ball_vel = Vec2.new(1000, 0)
    screen.state.ball_z, screen.state.ball_vz = 0, 0
end

---@param screen MatchScreen
local function start_actual_goal_replay(screen)
    for _ = 1, 40 do
        screen:update(1 / 60)
    end
    prepare_forced_home_goal(screen)
    screen:update(fixed_clock.TICK_SECONDS)
    t.eq(screen.state.score.home, 1)
    t.is_true(screen.state.kickoff_hold > 0)
    t.is_true(replay.active())
    t.is_true(screen._replay_state == nil)
end

t.describe("match screen rollback laboratory (tier 2)", function()
    t.it("constructs the combat companion for an explicit rollback playtest", function()
        local screen = Match.new({
            combat_enabled = true,
            rollback_lab = {
                local_slot = 2,
                profile_name = "clean",
            },
        })
        t.is_true(screen._combat_state ~= nil)
        local snapshot = rollback_playable_lab.current_snapshot(assert(screen._rollback_lab))
        t.is_true(snapshot.combat ~= nil)
        t.eq(assert(snapshot.combat).player_ids[2], screen.state.players[2].id)
    end)

    t.it("requires rollback snapshot combat presence to match the explicit opt-in", function()
        local state = sim_match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            input_ownership = sim_match.ownership_for_teams(teams.nebula, teams.orion),
        })
        local combat_snapshot = match_snapshot.capture(state, combat.new_state(state))
        local without_opt_in, without_opt_in_error = pcall(function()
            Match.new({
                rollback_lab = {
                    profile_name = "clean",
                    initial_snapshot = combat_snapshot,
                },
            })
        end)
        t.is_true(not without_opt_in)
        t.is_true(
            tostring(without_opt_in_error):find(
                "combat-bearing rollback snapshots require combat_enabled = true",
                1,
                true
            ) ~= nil
        )

        local opted_in = Match.new({
            combat_enabled = true,
            rollback_lab = {
                profile_name = "clean",
                initial_snapshot = combat_snapshot,
            },
        })
        t.is_true(opted_in._combat_state ~= nil)

        local soccer_state = sim_match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            input_ownership = sim_match.ownership_for_teams(teams.nebula, teams.orion),
        })
        local soccer_snapshot = match_snapshot.capture(soccer_state)
        local missing_companion, missing_companion_error = pcall(function()
            Match.new({
                combat_enabled = true,
                rollback_lab = {
                    profile_name = "clean",
                    initial_snapshot = soccer_snapshot,
                },
            })
        end)
        t.is_true(not missing_companion)
        t.is_true(
            tostring(missing_companion_error):find(
                "combat-enabled matches require a CombatMatchState companion",
                1,
                true
            ) ~= nil
        )
    end)

    t.it("is an explicit development-only slot-mode option", function()
        local ok = pcall(function()
            Match.new({
                profile = "product",
                rollback_lab = { profile_name = "clean" },
            })
        end)
        t.is_true(not ok, "product matches must reject the laboratory option")

        local screen = Match.new({
            rollback_lab = {
                local_slot = 6,
                profile_name = "clean",
            },
        })
        t.is_true(screen.state.slot_mode)
        t.eq(screen.state.controlled, screen.state.slot_players[6])
        t.eq(assert(screen._rollback_debug).local_slot, 6)
        t.eq(assert(screen._rollback_debug).status, "active")
    end)

    t.it("retains a zero-tick edge and consumes it exactly once", function()
        with_keyboard(function()
            local screen = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "clean",
                },
            })
            screen.state.owner = nil
            screen:event({ kind = "key", key = "k" })
            screen:update(fixed_clock.TICK_SECONDS / 2)
            t.eq(#screen._rollback_outputs, 0)
            t.is_true(screen._input_adapter.pending.switch)

            screen:update(fixed_clock.TICK_SECONDS / 2)
            t.eq(#screen._rollback_outputs, 1)
            local first = screen._rollback_outputs[1].input.slots[1].sample
            t.is_true(input_frame.has_edge(first, "switch") == true)
            t.is_true(not screen._input_adapter.pending.switch)

            screen:update(fixed_clock.TICK_SECONDS)
            t.eq(#screen._rollback_outputs, 1)
            local second = screen._rollback_outputs[1].input.slots[1].sample
            t.is_true(input_frame.has_edge(second, "switch") == false)
        end)
    end)

    t.it("captures a complete equipment tap before the next render update", function()
        with_keyboard(function()
            local screen = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "clean",
                },
            })
            screen:event({
                kind = "action",
                action = "equipment",
                pressed = true,
                source = "gamepad",
            })
            screen:event({
                kind = "action",
                action = "equipment",
                pressed = false,
                source = "gamepad",
            })
            screen:update(fixed_clock.TICK_SECONDS)

            local sample = screen._rollback_outputs[1].input.slots[1].sample
            t.is_true(input_frame.is_held(sample, "equipment") == false)
            t.is_true(input_frame.has_edge(sample, "equipment_pressed") == true)
            t.is_true(input_frame.has_edge(sample, "equipment_released") == true)
        end)
    end)

    t.it("uses one fixed clock and aggregates multi-tick edges, holds, and corrections", function()
        with_keyboard(function(down)
            local screen = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    network_profile = fixed_profile(2),
                    profile_name = "two_tick",
                },
            })
            screen.state.owner = nil
            screen:event({ kind = "key", key = "k" })
            down.lshift = true
            screen:update(3 * fixed_clock.TICK_SECONDS)

            t.eq(screen._clock.tick, 3)
            t.eq(assert(screen._rollback_debug).transport_tick, 3)
            t.eq(assert(screen._rollback_debug).reference_tick, 3)
            t.eq(#screen._rollback_outputs, 3)
            for index, output in ipairs(screen._rollback_outputs) do
                local sample = output.input.slots[1].sample
                t.is_true(input_frame.is_held(sample, "sprint") == true)
                t.eq(input_frame.has_edge(sample, "switch") == true, index == 1)
            end
            t.is_true(
                #screen._rollback_corrections > 0,
                "the render update must retain corrections from every simulated tick"
            )
            t.is_true(#screen._rollback_event_diffs >= #screen._rollback_outputs)
            t.eq(#screen._frame_events, 0, "legacy speculative consumers receive no events")
            t.is_true(not replay.active(), "rollback mode never starts the legacy goal replay")
        end)
    end)

    t.it("updates live player view state from the displayed rollback client", function()
        with_keyboard(function(down)
            local screen = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "clean",
                },
            })
            down.right = true
            down.lshift = true
            screen:update(fixed_clock.TICK_SECONDS)
            screen:update(fixed_clock.TICK_SECONDS)
            local player = screen.state.players[screen.state.slot_players[1]]
            local view = assert(view_state.get(player.id))
            t.is_true(view.speed > 0, "a moving lab player must produce live gait speed")
            t.is_true(view.phase > 0, "a moving lab player must advance gait phase")
        end)
    end)

    t.it("clears rollback handoff batches before paused and terminal early returns", function()
        with_keyboard(function()
            local paused = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    network_profile = fixed_profile(2),
                    profile_name = "pause",
                },
            })
            paused:update(3 * fixed_clock.TICK_SECONDS)
            t.is_true(#paused._rollback_outputs > 0)
            t.is_true(#paused._rollback_event_diffs > 0)
            tuning_panel.open = true
            paused:update(0)
            tuning_panel.open = false
            t.eq(#paused._rollback_outputs, 0)
            t.eq(#paused._rollback_event_diffs, 0)
            t.eq(#paused._rollback_confirmed_steps, 0)
            t.eq(#paused._rollback_corrections, 0)

            local terminal = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    network_profile = fixed_profile(2),
                    profile_name = "terminal",
                    max_rollback_ticks = 1,
                },
            })
            seed_render_correction(terminal)
            terminal:update(2 * fixed_clock.TICK_SECONDS)
            t.eq(assert(terminal._rollback_debug).status, "unconfirmed_window_exceeded")
            t.is_true(#terminal._rollback_outputs > 0)
            t.eq(assert(terminal._rollback_debug).active_smoothing_count, 0)
            t.near(assert(terminal._rollback_debug).correction_magnitude, 0)
            terminal._kickoff_banner = 0
            t.is_true(
                terminal:broadcast_phase() == nil,
                "synchronization failure must not masquerade as full time"
            )
            terminal:update(0)
            t.eq(#terminal._rollback_outputs, 0)
            t.eq(#terminal._rollback_event_diffs, 0)
            t.eq(#terminal._rollback_confirmed_steps, 0)
            t.eq(#terminal._rollback_corrections, 0)
        end)
    end)

    t.it("preserves fixed-clock overload dropping and contiguous transport ticks", function()
        with_keyboard(function()
            local screen = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "clean",
                },
            })
            screen:update((fixed_clock.MAX_TICKS_PER_UPDATE + 3.5) * fixed_clock.TICK_SECONDS)
            t.eq(#screen._rollback_outputs, fixed_clock.MAX_TICKS_PER_UPDATE)
            t.eq(screen._clock.tick, fixed_clock.MAX_TICKS_PER_UPDATE)
            t.eq(screen._clock.dropped_ticks, 3)
            t.eq(screen._clock.overloads, 1)
            t.eq(assert(screen._rollback_debug).transport_tick, fixed_clock.MAX_TICKS_PER_UPDATE)
            t.eq(assert(screen._rollback_debug).reference_tick, fixed_clock.MAX_TICKS_PER_UPDATE)
            t.near(screen._clock.accumulator, fixed_clock.TICK_SECONDS / 2, 1e-9)

            screen:update(fixed_clock.TICK_SECONDS / 2)
            t.eq(#screen._rollback_outputs, 1)
            t.eq(screen._rollback_outputs[1].tick, fixed_clock.MAX_TICKS_PER_UPDATE)
            t.eq(screen._clock.tick, fixed_clock.MAX_TICKS_PER_UPDATE + 1)
            t.eq(
                assert(screen._rollback_debug).transport_tick,
                fixed_clock.MAX_TICKS_PER_UPDATE + 1
            )
            t.eq(
                assert(screen._rollback_debug).reference_tick,
                fixed_clock.MAX_TICKS_PER_UPDATE + 1
            )
        end)
    end)

    t.it("live R replaces all rollback and presentation-owned state", function()
        with_keyboard(function()
            local screen = Match.new({
                formation = "2-1-1",
                rollback_lab = {
                    local_slot = 3,
                    network_profile = fixed_profile(2),
                    profile_name = "restart_profile",
                    network_seed = 22,
                },
            })
            local old_lab = assert(screen._rollback_lab)
            screen:update(3 * fixed_clock.TICK_SECONDS)
            view_state.update(screen.state.players, fixed_clock.TICK_SECONDS)
            t.is_true(view_state.get(screen.state.players[1].id) ~= nil)
            seed_render_correction(screen)
            screen._last_score = 8
            screen._last_home = 4
            screen._last_scoring_team = "home"
            screen._kickoff_banner = 0
            screen._replay_state = screen.state --[[@as ReplayFrame]]
            screen:event({ kind = "key", key = "k" })

            screen:event({ kind = "key", key = "r" })
            t.is_true(assert(screen._rollback_lab) ~= old_lab)
            local debug = assert(screen._rollback_debug)
            t.eq(debug.profile, "restart_profile")
            t.eq(debug.local_slot, 3)
            t.eq(debug.transport_tick, 0)
            t.eq(debug.reference_tick, 0)
            t.eq(debug.current_tick, 0)
            t.eq(debug.rollback_count, 0)
            t.eq(debug.network_pending, 0)
            t.eq(debug.event_status, "active")
            t.eq(debug.active_smoothing_count, 0)
            t.near(debug.correction_magnitude, 0)
            t.eq(screen._clock.tick, 0)
            t.eq(screen._clock.accumulator, 0)
            t.eq(#screen._frame_events, 0)
            t.eq(#screen._rollback_outputs, 0)
            t.eq(#screen._rollback_event_diffs, 0)
            t.eq(#screen._rollback_confirmed_steps, 0)
            t.eq(#screen._rollback_corrections, 0)
            t.eq(screen._last_score, 0)
            t.eq(screen._last_home, 0)
            t.is_true(screen._last_scoring_team == nil)
            t.is_true(screen._kickoff_banner > 0)
            t.is_true(screen._replay_state == nil)
            t.is_true(not replay.active())
            t.is_true(view_state.get(screen.state.players[1].id) == nil)
            t.is_true(not screen._input_adapter.pending.switch)
            t.eq(screen.state.controlled, screen.state.slot_players[3])
        end)
    end)

    t.it("clears smoothing at kickoff, full time, and stack teardown", function()
        with_keyboard(function()
            local kickoff = Match.new()
            prepare_forced_home_goal(kickoff)
            seed_render_correction(kickoff)
            kickoff:update(fixed_clock.TICK_SECONDS)
            t.eq(kickoff.state.score.home, 1)
            t.is_true(kickoff.state.kickoff_hold > 0)
            t.eq(correction_smoothing.diagnostics(kickoff._render_smoothing).active_count, 0)
            replay.reset()

            local full_time = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "clean",
                },
            })
            seed_render_correction(full_time)
            full_time.state.finished = true
            full_time:update(0)
            t.eq(assert(full_time._rollback_debug).active_smoothing_count, 0)

            local stack = ScreenStack.new()
            local teardown = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "clean",
                },
            })
            seed_render_correction(teardown)
            stack:push(teardown)
            t.is_true(stack:pop() == teardown)
            t.eq(correction_smoothing.diagnostics(teardown._render_smoothing).active_count, 0)
            t.eq(assert(teardown._rollback_debug).active_smoothing_count, 0)
        end)
    end)

    t.it("keeps actual goal replay gait coherent and clears smoothing on both exits", function()
        with_keyboard(function()
            local natural = Match.new()
            start_actual_goal_replay(natural)
            t.is_true(
                view_state.get(natural.state.players[1].id) == nil,
                "successful replay entry must discard the post-goal kickoff pose"
            )

            natural:update(1 / 60)
            local replay_state = assert(natural._replay_state)
            for _, player in ipairs(replay_state.players) do
                local view = assert(view_state.get(player.id))
                t.near(view.speed, 0)
                t.near(view.phase, 0)
                t.near(view.lean, 0)
            end
            for _ = 1, 12 do
                natural:update(1 / 60)
            end
            local saw_gait = false
            local saw_lean = false
            for _, player in ipairs(assert(natural._replay_state).players) do
                local view = assert(view_state.get(player.id))
                saw_gait = saw_gait or (view.speed > 0 and view.phase > 0)
                saw_lean = saw_lean or math.abs(view.lean) > 0
            end
            t.is_true(saw_gait, "active replay frames must preserve gait progression")
            t.is_true(saw_lean, "active replay frames must preserve lean progression")
            seed_render_correction(natural)

            natural:update(2)
            natural:update(100)
            t.is_true(not replay.active())
            t.is_true(natural._replay_state == nil)
            local natural_live = assert(view_state.get(natural.state.players[1].id))
            t.near(natural_live.speed, 0)
            t.near(natural_live.phase, 0)
            t.near(natural_live.lean, 0)
            t.eq(correction_smoothing.diagnostics(natural._render_smoothing).active_count, 0)

            local skipped = Match.new()
            start_actual_goal_replay(skipped)
            skipped:update(1 / 60)
            seed_render_correction(skipped)
            skipped:event({ kind = "key", key = "space" })
            t.is_true(not replay.active())
            t.is_true(skipped._replay_state == nil)
            local skipped_live = assert(view_state.get(skipped.state.players[1].id))
            t.near(skipped_live.speed, 0)
            t.near(skipped_live.phase, 0)
            t.near(skipped_live.lean, 0)
            t.eq(correction_smoothing.diagnostics(skipped._render_smoothing).active_count, 0)
        end)
    end)

    t.it("draws only from the cached debug model without mutating either match", function()
        local screen = Match.new({
            rollback_lab = {
                local_slot = 1,
                profile_name = "clean",
            },
        })
        local client_before = match_snapshot.hash(
            rollback_playable_lab.current_snapshot(assert(screen._rollback_lab))
        )
        local reference_before = match_snapshot.hash(
            rollback_playable_lab.reference_snapshot(assert(screen._rollback_lab))
        )
        local debug_before = assert(screen._rollback_debug).transport_tick
        local saved = love.graphics
        local saved_bloom = bloom.config.enabled
        local graphics = {}
        local noop = function() end
        for _, name in ipairs({
            "setColor",
            "setLineWidth",
            "setBlendMode",
            "rectangle",
            "polygon",
            "line",
            "circle",
            "ellipse",
            "arc",
            "push",
            "pop",
            "translate",
            "rotate",
            "print",
            "printf",
        }) do
            graphics[name] = noop
        end
        graphics.getDimensions = function()
            return 960, 540
        end
        graphics.getWidth = function()
            return 960
        end
        graphics.getHeight = function()
            return 540
        end
        love.graphics = graphics
        bloom.config.enabled = false
        local ok, err = pcall(function()
            screen:draw()
        end)
        love.graphics = saved
        bloom.config.enabled = saved_bloom
        assert(ok, err)

        t.eq(
            match_snapshot.hash(
                rollback_playable_lab.current_snapshot(assert(screen._rollback_lab))
            ),
            client_before
        )
        t.eq(
            match_snapshot.hash(
                rollback_playable_lab.reference_snapshot(assert(screen._rollback_lab))
            ),
            reference_before
        )
        t.eq(assert(screen._rollback_debug).transport_tick, debug_before)
    end)
end)

t.describe("playable rollback ScreenStack flow (tier 3)", function()
    t.it("converges under the checked-in playable profile with pinned seeds", function()
        with_keyboard(function()
            local stack = ScreenStack.new()
            local screen = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    profile_name = "playable",
                    network_seed = 7302,
                    bot_seed = 7400,
                    duration = 12 * fixed_clock.TICK_SECONDS,
                    settlement_ticks = 128,
                },
            })
            stack:push(screen)
            local saw_correction = false
            local saw_smoothing = false
            local saw_settling = false
            for _ = 1, 160 do
                stack:update(fixed_clock.TICK_SECONDS)
                saw_correction = saw_correction or #screen._rollback_corrections > 0
                local frame_debug = assert(screen._rollback_debug)
                if
                    not saw_smoothing
                    and #screen._rollback_corrections > 0
                    and frame_debug.active_smoothing_count > 0
                then
                    saw_smoothing = true
                    local magnitude_before = frame_debug.correction_magnitude
                    local hash_before = match_snapshot.hash(
                        rollback_playable_lab.current_snapshot(assert(screen._rollback_lab))
                    )
                    stack:update(fixed_clock.TICK_SECONDS / 4)
                    local settled_debug = assert(screen._rollback_debug)
                    t.eq(#screen._rollback_corrections, 0)
                    t.is_true(settled_debug.correction_magnitude < magnitude_before)
                    t.eq(
                        match_snapshot.hash(
                            rollback_playable_lab.current_snapshot(assert(screen._rollback_lab))
                        ),
                        hash_before,
                        "render-only settling must not change the client snapshot hash"
                    )
                    saw_settling = true
                end
                local status = assert(screen._rollback_debug).status
                if status ~= "active" and status ~= "settling" then
                    break
                end
            end
            local debug = assert(screen._rollback_debug)
            t.is_true(saw_correction)
            t.is_true(saw_smoothing)
            t.is_true(saw_settling)
            t.is_true(debug.rollback_count > 0)
            t.is_true(debug.resimulated_ticks > 0)
            t.eq(debug.status, "converged")
            t.eq(debug.convergence.status, "matched")
            t.eq(debug.reference_tick, debug.current_tick)
            local current = rollback_playable_lab.current_snapshot(assert(screen._rollback_lab))
            local reference = rollback_playable_lab.reference_snapshot(assert(screen._rollback_lab))
            t.eq(current.state.input_tick, reference.state.input_tick)
            t.eq(match_snapshot.hash(current), match_snapshot.hash(reference))
            t.eq(debug.convergence.actual_hash, debug.convergence.expected_hash)
            t.eq(debug.confirmed_input_tick, debug.reference_tick - 1)
            t.eq(debug.confirmed_output_tick, debug.reference_tick - 1)
            t.eq(debug.network_pending, 0)
            t.eq(debug.active_smoothing_count, 0)
            t.near(debug.correction_magnitude, 0)
        end)
    end)

    t.it("reconciles a rollback goal through confirmed replay and result completion", function()
        with_keyboard(function()
            local request = assert(contract.new_request({
                home_team_id = "nebula",
                away_team_id = "orion",
                home_starter_ids = {
                    "ozzo",
                    "veil_nyx",
                    "rok_tann",
                    "mika_olu",
                    "sela_dwin",
                },
                formation_id = "1-2-1",
                tactic_id = "balanced",
                seed = 75,
            }))
            local match = Match.new({
                rollback_lab = {
                    local_slot = 1,
                    network_profile = fixed_profile(2),
                    profile_name = "goal_flow",
                    network_seed = 7501,
                    bot_seed = 7502,
                    settlement_ticks = 128,
                    initial_snapshot = rollback_goal_fixture(),
                },
            })
            local completed = 0
            local real = RealMatch.new(request, {
                on_finished = function()
                    completed = completed + 1
                end,
                on_cancelled = function() end,
            })
            real.match = match
            real.observer = match_observer.new(match.state)
            local stack = ScreenStack.new()
            stack:push(real)
            local saw_correction = false
            local saw_goal_presentation = false
            local saw_terminal_replay = false
            local replay_finished_at = nil
            local completed_at = nil
            for iteration = 1, 600 do
                stack:update(fixed_clock.TICK_SECONDS)
                saw_correction = saw_correction or #match._rollback_corrections > 0
                saw_goal_presentation = saw_goal_presentation
                    or match:broadcast_phase() == "goal"
                    or replay.active()
                local status = assert(match._rollback_debug).status
                if status == "converged" and replay.active() then
                    saw_terminal_replay = true
                    t.eq(completed, 0, "terminal replay must block result navigation")
                elseif
                    saw_terminal_replay
                    and replay_finished_at == nil
                    and not replay.active()
                then
                    replay_finished_at = iteration
                end
                if completed > 0 and completed_at == nil then
                    completed_at = iteration
                end
                if completed > 0 and assert(match._rollback_debug).status == "converged" then
                    break
                end
            end

            local debug = assert(match._rollback_debug)
            local cues = audio.confirmed_cue_counts()
            t.is_true(saw_correction)
            t.is_true(saw_goal_presentation, "confirmed goal starts celebration/replay")
            t.is_true(saw_terminal_replay, "confirmed replay overlaps terminal convergence")
            t.is_true(replay_finished_at ~= nil, "terminal renderer replay finishes naturally")
            t.is_true(
                assert(completed_at) > assert(replay_finished_at),
                "result completion follows replay and full-time hold"
            )
            t.eq(match.state.score.home, 1)
            t.is_true(match:full_time_confirmed())
            t.eq(debug.status, "converged")
            t.is_true(not replay.active(), "result begins only after replay becomes inactive")
            t.is_true(
                not match._pending_confirmed_kickoff,
                "full time clears the post-goal kickoff beat"
            )
            t.eq(completed, 1)
            t.eq(cues.goal, 1)
            t.eq(cues.kickoff, 1)
            t.eq(cues.full_time, 1)
            stack:update(1)
            t.eq(completed, 1, "confirmed result completion remains exactly once")
        end)
    end)
end)
