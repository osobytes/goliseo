local Vec2 = require("core.vec2")
local audio = require("game.audio")
local match_observer = require("game.match_observer")
local correction_smoothing = require("game.render.correction_smoothing")
local effects = require("game.render.effects")
local replay = require("game.render.replay")
local view_state = require("game.render.view_state")
local Match = require("game.screens.match")
local RealMatch = require("game.screens.real_match")
local fixed_clock = require("sim.fixed_clock")
local sim_match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

---@param id string
---@param tick integer
---@param kind string
---@param player string?
---@return RollbackWrappedMatchEvent
local function wrapped_match_event(id, tick, kind, player)
    return {
        id = id,
        tick = tick,
        domain = "match/" .. kind,
        ordinal = 1,
        payload = {
            kind = kind,
            x = 100 + tick,
            y = 200 + tick,
            player = player,
        },
    }
end

---@param id string
---@param tick integer
---@param kind RollbackLifecycleKind
---@return RollbackWrappedLifecycleEvent
local function wrapped_lifecycle_event(id, tick, kind)
    local domain = "lifecycle/" .. kind
    return {
        id = id,
        tick = tick,
        domain = domain,
        ordinal = 1,
        payload = {
            kind = kind,
            team = kind == "goal" and "home" or nil,
            score = { home = kind == "goal" and 1 or 0, away = 0 },
        },
    }
end

---@param id string
---@param tick integer
---@param sequence integer
---@return RollbackWrappedCombatEvent
local function wrapped_combat_event(id, tick, sequence)
    return {
        id = id,
        tick = tick,
        domain = "combat/contact/" .. sequence,
        ordinal = 1,
        payload = {
            kind = "contact",
            tick = tick,
            family_id = "light_melee",
            source_index = 1,
            target_index = 2,
            source_sequence = sequence,
            result = "hit",
            x = 100 + tick,
            y = 200 + tick,
            interruption_ticks = nil,
            displacement_px = nil,
        },
    }
end

---@param tick integer
---@param events RollbackWrappedMatchEvent[]
---@param owner_team InputTeam?
---@param home_score integer?
---@return RollbackEventStep
local function confirmed_step(tick, events, owner_team, home_score)
    return {
        tick = tick,
        start_boundary = tick,
        end_boundary = tick + 1,
        state = {
            score = { home = home_score or 0, away = 0 },
            time_left = 120 - (tick + 1) * fixed_clock.TICK_SECONDS,
            finished = false,
            owner_id = nil,
            owner_team = owner_team,
        },
        match_events = events,
        lifecycle_events = {},
    }
end

t.describe("rollback presentation consumers", function()
    t.it("deduplicates confirmed event and lifecycle audio by stable ID", function()
        audio.reset()
        local shot = wrapped_match_event("shot-id", 4, "shot", "striker")
        local goal = wrapped_lifecycle_event("goal-id", 4, "goal")
        local kickoff = wrapped_lifecycle_event("kickoff-id", 4, "kickoff")
        local full_time = wrapped_lifecycle_event("full-time-id", 5, "full_time")

        t.is_true(audio.consume_confirmed(shot))
        t.is_true(not audio.consume_confirmed(shot))
        audio.consume_confirmed(goal)
        audio.consume_confirmed(goal)
        audio.consume_confirmed(kickoff)
        audio.consume_confirmed(full_time)
        local counts = audio.confirmed_cue_counts()
        t.eq(counts.shot, 1)
        t.eq(counts.goal, 1)
        t.eq(counts.kickoff, 1)
        t.eq(counts.full_time, 1)
    end)

    t.it("publishes only the confirmed combat replacement exactly once", function()
        audio.reset()
        effects.reset()
        local screen = Match.new({
            rollback_lab = {
                profile_name = "clean",
                duration = 2,
            },
        })
        local predicted = wrapped_combat_event("predicted-combat", 8, 1)
        local authoritative = wrapped_combat_event("authoritative-combat", 8, 2)
        t.is_true(screen:consume_rollback_event_diff({
            added = { predicted },
            revoked = {},
            replaced = {},
        }))
        t.is_true(screen:consume_rollback_event_diff({
            added = {},
            revoked = {},
            replaced = { { before = predicted, after = authoritative } },
        }))

        local step = confirmed_step(8, {}, nil)
        step.combat_events = { authoritative }
        t.eq(screen:consume_confirmed_step(step), 1)
        t.eq(screen:consume_confirmed_step(step), 0)
        t.is_true(audio.consume_confirmed(predicted), "corrected-away combat ID was not published")
        t.is_true(
            not audio.consume_confirmed(authoritative),
            "confirmed combat replacement was already published"
        )
    end)

    t.it("adds, replaces, and revokes speculative effects by event ID", function()
        effects.reset()
        local shot = wrapped_match_event("action-id", 8, "shot", "striker")
        effects.apply_event_diff({ added = { shot }, revoked = {}, replaced = {} })
        local added = effects.diagnostics()
        t.eq(added.particle_count, 11)
        t.eq(added.speculative_ids[1], "action-id")

        effects.apply_event_diff({ added = { shot }, revoked = {}, replaced = {} })
        t.eq(effects.diagnostics().particle_count, 11, "duplicate add is idempotent")

        local pass = wrapped_match_event("action-id", 8, "pass", "striker")
        effects.apply_event_diff({
            added = {},
            revoked = {},
            replaced = { { before = shot, after = pass } },
        })
        local replaced = effects.diagnostics()
        t.eq(replaced.particle_count, 6)
        t.eq(replaced.speculative_ids[1], "action-id")

        effects.apply_event_diff({ added = {}, revoked = { pass }, replaced = {} })
        local revoked = effects.diagnostics()
        t.eq(revoked.particle_count, 0)
        t.eq(#revoked.speculative_ids, 0)
    end)

    t.it("keeps speculative goal lifecycle changes silent and reversible", function()
        audio.reset()
        effects.reset()
        replay.reset()
        local goal = wrapped_lifecycle_event("predicted-goal", 9, "goal")
        effects.apply_event_diff({ added = { goal }, revoked = {}, replaced = {} })
        t.eq(effects.diagnostics().particle_count, 0)
        t.is_true(not replay.active())
        t.is_true(audio.confirmed_cue_counts().goal == nil)

        effects.apply_event_diff({ added = {}, revoked = { goal }, replaced = {} })
        t.eq(effects.diagnostics().particle_count, 0)
        t.is_true(not replay.active())
        t.is_true(audio.confirmed_cue_counts().kickoff == nil)
    end)

    t.it("gates every confirmed lifecycle side effect by stable ID", function()
        local screen = Match.new({
            rollback_lab = {
                profile_name = "clean",
                duration = 2,
            },
        })
        for boundary = 1, 40 do
            replay.record_boundary(boundary, screen.state)
        end
        local goal = wrapped_lifecycle_event("confirmed-goal", 40, "goal")
        local kickoff = wrapped_lifecycle_event("confirmed-kickoff", 40, "kickoff")
        local full_time = wrapped_lifecycle_event("confirmed-full-time", 41, "full_time")

        t.is_true(screen:consume_confirmed_lifecycle(goal))
        t.is_true(replay.active())
        replay.step(0.5)
        local elapsed = replay.diagnostics().celebration_elapsed
        t.is_true(not screen:consume_confirmed_lifecycle(goal))
        t.eq(
            replay.diagnostics().celebration_elapsed,
            elapsed,
            "duplicate goal cannot restart replay"
        )

        t.is_true(screen:consume_confirmed_lifecycle(kickoff))
        t.is_true(screen._pending_confirmed_kickoff)
        t.is_true(not screen:consume_confirmed_lifecycle(kickoff))
        t.is_true(screen._pending_confirmed_kickoff)

        t.is_true(screen:consume_confirmed_lifecycle(full_time))
        t.is_true(screen:full_time_confirmed())
        t.is_true(
            not screen._pending_confirmed_kickoff,
            "full time supersedes a queued kickoff banner"
        )
        t.is_true(not screen:consume_confirmed_lifecycle(full_time))
        t.is_true(screen:full_time_confirmed())
        t.is_true(replay.active(), "duplicate full time cannot stop or restart confirmed replay")

        local counts = audio.confirmed_cue_counts()
        t.eq(counts.goal, 1)
        t.eq(counts.kickoff, 1)
        t.eq(counts.full_time, 1)
    end)

    t.it("isolates replay view state and live effects through entry and exit", function()
        local screen = Match.new({
            rollback_lab = {
                profile_name = "clean",
                duration = 2,
            },
        })
        local player = screen.state.players[1]
        local player_id = player.id
        view_state.reset()
        view_state.update(screen.state.players, 0)
        player.pos = player.pos:add(Vec2.new(100, 0))
        view_state.update(screen.state.players, 0.1)
        t.is_true(assert(view_state.get(player_id)).phase > 0)

        for boundary = 1, 40 do
            replay.record_boundary(boundary, screen.state)
        end
        local previous = sim_match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
        })
        previous.players[1].pos = screen.state.players[1].pos:add(Vec2.new(-40, 0))
        screen._render_smoothing = correction_smoothing.new(previous)
        screen._render_smoothing =
            correction_smoothing.correct(screen._render_smoothing, screen.state)
        screen._render_pose = correction_smoothing.pose(screen._render_smoothing)
        t.is_true(correction_smoothing.diagnostics(screen._render_smoothing).active_count > 0)

        local goal = wrapped_lifecycle_event("view-goal", 40, "goal")
        t.is_true(screen:consume_confirmed_lifecycle(goal))
        t.is_true(view_state.get(player_id) == nil, "replay entry drops live gait history")
        t.eq(correction_smoothing.diagnostics(screen._render_smoothing).active_count, 0)
        t.eq(assert(screen._rollback_debug).active_smoothing_count, 0)

        screen:update(0)
        local replay_view = assert(view_state.get(player_id))
        t.eq(replay_view.speed, 0)
        t.eq(replay_view.phase, 0)
        t.eq(replay_view.lean, 0)

        local shot = wrapped_match_event("future-shot", 41, "shot", player_id)
        t.is_true(not screen:consume_rollback_event_diff({
            added = { shot },
            revoked = {},
            replaced = {},
        }))
        local replay_effects = effects.diagnostics()
        t.eq(replay_effects.particle_count, 0, "live particles stay off past replay footage")
        t.eq(#replay_effects.speculative_ids, 0, "live effects are not retained as drawable IDs")

        -- A live-only position discontinuity must not contaminate the replay's
        -- derived speed/phase while replay footage is the drawn timeline.
        player.pos = player.pos:add(Vec2.new(10000, 0))
        screen:update(0.01)
        replay_view = assert(view_state.get(player_id))
        t.is_true(replay_view.speed < 1000, "replay gait ignores the hidden live timeline")
        t.is_true(replay_view.phase < 1, "replay gait remains continuous")

        screen:event({ kind = "action", action = "confirm" })
        t.is_true(not replay.active())
        local live_view = assert(view_state.get(player_id))
        t.eq(live_view.speed, 0)
        t.eq(live_view.phase, 0)
        t.eq(live_view.lean, 0)

        local pass = wrapped_match_event("future-shot", 41, "pass", player_id)
        t.is_true(screen:consume_rollback_event_diff({
            added = {},
            revoked = {},
            replaced = { { before = shot, after = pass } },
        }))
        t.eq(
            effects.diagnostics().particle_count,
            0,
            "a suppressed event replacement cannot surface after replay"
        )
    end)

    t.it("drops an invalid loose-ball trail on correction", function()
        effects.reset()
        local state =
            sim_match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        state.owner = nil
        state.ball = Vec2.new(300, 200)
        state.ball_vel = Vec2.new(500, 0)
        effects.sample_ball(state)
        t.eq(effects.diagnostics().trail_count, 1)
        effects.reset_trail()
        t.eq(effects.diagnostics().trail_count, 0)
    end)

    t.it("accounts newly confirmed observer steps exactly once", function()
        local state =
            sim_match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        local home_id = state.players[2].id
        local pass = wrapped_match_event("pass-0", 0, "pass", home_id)
        local shot = wrapped_match_event("shot-1", 1, "shot", home_id)
        local delayed = match_observer.new(state)
        local reference = match_observer.new(state)
        local first = confirmed_step(0, { pass }, "home")
        local second = confirmed_step(1, { shot }, "home", 1)

        match_observer.observe_confirmed(reference, first)
        match_observer.observe_confirmed(reference, second)
        match_observer.observe_confirmed(delayed, first)
        t.is_true(not match_observer.observe_confirmed(delayed, first))
        match_observer.observe_confirmed(delayed, second)
        t.is_true(not match_observer.observe_confirmed(delayed, second))

        local expected = match_observer.finish(reference)
        local actual = match_observer.finish(delayed)
        t.eq(actual.home_stats.shots, expected.home_stats.shots)
        t.eq(actual.home_stats.pass_completion, expected.home_stats.pass_completion)
        t.near(actual.home_stats.possession, assert(expected.home_stats.possession), 1e-12)
        t.eq(actual.mvp_player_id, expected.mvp_player_id)
    end)

    t.it("truncates and replaces replay footage by simulation boundary", function()
        replay.reset()
        local state =
            sim_match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        for boundary = 0, 40 do
            state.ball = Vec2.new(boundary, 100)
            replay.record_boundary(boundary, state)
        end
        replay.truncate_from(20)
        for boundary = 20, 40 do
            state.ball = Vec2.new(1000 + boundary, 200)
            replay.record_boundary(boundary, state)
        end

        local diagnostics = replay.diagnostics()
        t.eq(diagnostics.count, 41)
        t.eq(diagnostics.oldest_boundary, 0)
        t.eq(diagnostics.newest_boundary, 40)
        t.eq(assert(replay.boundary_sample(19)).ball_x, 19)
        t.eq(assert(replay.boundary_sample(20)).ball_x, 1020)
        t.eq(assert(replay.boundary_sample(40)).ball_y, 200)
    end)

    t.it("defers RealMatch completion until a confirmed replay is explicitly skipped", function()
        local lab_match = Match.new({
            rollback_lab = {
                profile_name = "clean",
                duration = fixed_clock.TICK_SECONDS,
            },
        })
        local completed = 0
        local request = {
            home_team_id = "nebula",
            away_team_id = "orion",
            home_starter_ids = { "ozzo", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin" },
            formation_id = "1-2-1",
            tactic_id = "balanced",
            arena_id = "helios_crown",
            seed = 75,
            show_onboarding = false,
        }
        ---@cast request ProductMatchRequest
        local screen = RealMatch.new(request, {
            on_finished = function()
                completed = completed + 1
            end,
            on_cancelled = function() end,
        })
        screen.match = lab_match
        screen.observer = match_observer.new(lab_match.state)

        lab_match.state.finished = true
        screen:update(0)
        t.eq(completed, 0, "speculative state.finished must not complete the result")

        for boundary = 1, 40 do
            replay.record_boundary(boundary, lab_match.state)
        end
        lab_match:consume_confirmed_lifecycle(wrapped_lifecycle_event("overlap-goal", 40, "goal"))
        lab_match:consume_confirmed_lifecycle(
            wrapped_lifecycle_event("overlap-full-time", 41, "full_time")
        )
        screen:update(1)
        t.eq(completed, 0, "full-time hold cannot navigate during confirmed replay")
        t.is_true(lab_match:result_completion_blocked())

        screen:event({ kind = "action", action = "confirm" })
        t.is_true(not lab_match:result_completion_blocked())
        screen:update(0.89)
        t.eq(completed, 0, "skipping replay still preserves the full-time hold")
        screen:update(0.02)
        t.eq(completed, 1)
        screen:update(1)
        t.eq(completed, 1)
    end)
end)
