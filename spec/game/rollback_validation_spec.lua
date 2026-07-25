local rollback_validation = require("game.rollback_validation")
local audio = require("game.audio")
local Match = require("game.screens.match")
local Vec2 = require("core.vec2")
local teams = require("data.teams")
local combat = require("sim.combat")
local sim_match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_events = require("sim.rollback_events")
local t = require("spec.support.runner")

---@param id string
---@param tick integer
---@param kind string
---@param player string?
---@return RollbackWrappedMatchEvent
local function match_event(id, tick, kind, player)
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
---@param home_score integer
---@return RollbackWrappedLifecycleEvent
local function lifecycle_event(id, tick, kind, home_score)
    return {
        id = id,
        tick = tick,
        domain = "lifecycle/" .. kind,
        ordinal = 1,
        payload = {
            kind = kind,
            team = kind == "goal" and "home" or nil,
            score = { home = home_score, away = 0 },
        },
    }
end

---@param tick integer
---@param score integer
---@param finished boolean
---@param owner_id string?
---@param owner_team InputTeam?
---@param match_events RollbackWrappedMatchEvent[]
---@param lifecycle_events RollbackWrappedLifecycleEvent[]
---@return RollbackEventStep
local function step(tick, score, finished, owner_id, owner_team, match_events, lifecycle_events)
    return {
        tick = tick,
        start_boundary = tick,
        end_boundary = tick + 1,
        state = {
            score = { home = score, away = 0 },
            time_left = math.max(0, 120 - (tick + 1) / 60),
            finished = finished,
            owner_id = owner_id,
            owner_team = owner_team,
        },
        match_events = match_events,
        lifecycle_events = lifecycle_events,
    }
end

---@param added RollbackWrappedEvent[]
---@param revoked RollbackWrappedEvent[]?
---@param replaced RollbackEventReplacement[]?
---@return RollbackEventDiff
local function diff(added, revoked, replaced)
    return {
        added = added,
        revoked = revoked or {},
        replaced = replaced or {},
    }
end

---@return MatchState
local function new_state()
    return sim_match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 77,
    })
end

---@param source MatchState
---@param boundary integer
---@param ball_x number
---@return MatchState
local function replay_state(source, boundary, ball_x)
    local state = new_state()
    state.input_tick = boundary
    state.ball = Vec2.new(ball_x, source.ball.y)
    state.score.home = source.score.home
    state.score.away = source.score.away
    state.finished = source.finished
    return state
end

t.describe("rollback validation", function()
    t.it("seeds its event audit from an atomic combat boundary", function()
        local state = new_state()
        local combat_state = combat.new_state(state)
        local initial, initial_combat =
            match_snapshot.restore(match_snapshot.capture(state, combat_state))
        local report = rollback_validation.run(initial, {}, {
            home_team_id = "nebula",
            away_team_id = "orion",
            initial_combat_state = assert(initial_combat),
            reference_final_state = initial,
            impaired_final_state = initial,
        })

        t.is_true(report.passed, table.concat(report.errors, "; "))
    end)

    t.it("audits corrected presentation consumers against independent authority", function()
        local initial = new_state()
        local home_player = initial.players[2].id
        local away_player = initial.players[7].id
        local away_keeper = initial.players[6].id
        local tackle = match_event("tackle-0", 0, "tackle", home_player)
        local shot = match_event("shot-1", 1, "shot", home_player)
        local header = match_event("header-2", 2, "header", away_player)
        local catch = match_event("catch-3", 3, "catch", away_keeper)
        local goal = lifecycle_event("goal-4", 4, "goal", 1)
        local kickoff = lifecycle_event("kickoff-4", 4, "kickoff", 1)
        local full_time = lifecycle_event("full-time-5", 5, "full_time", 1)
        local confirmed = {
            step(0, 0, false, home_player, "home", { tackle }, {}),
            step(1, 0, false, home_player, "home", { shot }, {}),
            step(2, 0, false, away_player, "away", { header }, {}),
            step(3, 0, false, away_keeper, "away", { catch }, {}),
            step(4, 1, false, home_player, "home", {}, { goal, kickoff }),
            step(5, 1, true, nil, nil, {}, { full_time }),
        }
        local stale_tackle = match_event("stale-tackle", 0, "touch", home_player)
        local revoked_shot = match_event("revoked-shot", 1, "shot", away_player)
        local trace = {}
        for _, value in ipairs(confirmed) do
            trace[#trace + 1] = { kind = "reference_confirmed", step = value }
        end
        trace[#trace + 1] = { kind = "impaired_diff", diff = diff({ stale_tackle }) }
        trace[#trace + 1] = {
            kind = "impaired_diff",
            diff = diff({}, nil, { { before = stale_tackle, after = tackle } }),
        }
        trace[#trace + 1] = { kind = "impaired_confirmed", step = confirmed[1] }
        trace[#trace + 1] = {
            kind = "impaired_confirmed",
            step = confirmed[1],
        }
        trace[#trace + 1] = {
            kind = "impaired_diff",
            diff = diff({ revoked_shot }),
        }
        trace[#trace + 1] = {
            kind = "impaired_diff",
            diff = diff({}, { revoked_shot }),
        }
        for index = 2, #confirmed do
            local value = confirmed[index]
            local added = {}
            for _, event in ipairs(value.match_events) do
                added[#added + 1] = event
            end
            for _, event in ipairs(value.lifecycle_events) do
                added[#added + 1] = event
            end
            trace[#trace + 1] = { kind = "impaired_diff", diff = diff(added) }
            trace[#trace + 1] = { kind = "confirmed", step = value }
        end

        trace[#trace + 1] = {
            kind = "replay_boundary",
            boundary = 0,
            snapshot = match_snapshot.capture(replay_state(initial, 0, 10)),
        }
        trace[#trace + 1] = {
            kind = "replay_boundary",
            boundary = 1,
            state = replay_state(initial, 1, 11),
        }
        trace[#trace + 1] = {
            kind = "replay_boundary",
            boundary = 2,
            state = replay_state(initial, 2, 12),
        }
        trace[#trace + 1] = { kind = "replay_truncate", boundary = 1 }
        trace[#trace + 1] = {
            kind = "replay_boundary",
            boundary = 1,
            state = replay_state(initial, 1, 101),
        }
        trace[#trace + 1] = {
            kind = "replay_boundary",
            boundary = 2,
            state = replay_state(initial, 2, 102),
        }

        local final_reference = new_state()
        final_reference.score.home = 1
        final_reference.finished = true
        local final_impaired = new_state()
        final_impaired.score.home = 1
        final_impaired.finished = true
        local report = rollback_validation.run(initial, trace, {
            home_team_id = "nebula",
            away_team_id = "orion",
            reference_final_state = final_reference,
            impaired_final_state = final_impaired,
            seed = 77,
            expected_replay_boundaries = { 0, 1, 2 },
            expected_replay_samples = {
                {
                    boundary = 1,
                    ball_x = 101,
                    ball_y = initial.ball.y,
                    score_home = 0,
                    score_away = 0,
                },
                {
                    boundary = 2,
                    ball_x = 102,
                    ball_y = initial.ball.y,
                    score_home = 0,
                    score_away = 0,
                },
            },
            expected_replay_truncate_count = 1,
            required_scenarios = {
                "possession",
                "tackle",
                "shot",
                "goal",
                "kickoff",
                "aerial",
                "keeper",
                "full_time",
            },
        })

        t.is_true(report.passed, table.concat(report.errors, "; "))
        t.eq(#report.errors, 0)
        t.eq(report.events.reference_unique, 7)
        t.eq(report.events.impaired_unique, 7)
        t.is_true(report.events.duplicate_confirmed > 0)
        t.eq(report.events.speculative_replaced, 1)
        t.eq(report.events.speculative_revoked, 1)
        t.eq(report.events.terminal_speculative_residue, 0)
        t.eq(report.events.consumer_speculative_residue, 0)
        t.eq(report.consumers.audio_cue_count, report.consumers.expected_audio_cue_count)
        t.is_true(report.observer_matched)
        t.is_true(report.result_matched)
        t.is_true(report.replay_matched)
        t.eq(report.consumers.replay_record_count, 5)
        t.eq(report.consumers.replay_truncate_count, 1)
        t.eq(report.replay_boundaries[1], 0)
        t.eq(report.replay_boundaries[3], 2)
    end)

    t.it("builds the authoritative bounded replay timeline", function()
        local boundaries = rollback_validation.expected_replay_boundaries(500)
        t.eq(#boundaries, 480)
        t.eq(boundaries[1], 21)
        t.eq(boundaries[#boundaries], 500)
    end)

    t.it("reports missing confirmation and terminal speculative residue", function()
        local initial = new_state()
        local player = initial.players[2].id
        local authority = match_event("authority-shot", 0, "shot", player)
        local residue = match_event("residue-shot", 1, "shot", player)
        local reference = step(0, 0, false, player, "home", { authority }, {})
        local trace = {
            { kind = "reference_confirmed", step = reference },
            { kind = "impaired_diff", diff = diff({ authority, residue }) },
        }

        local report = rollback_validation.run(initial, trace, {
            home_team_id = "nebula",
            away_team_id = "orion",
            reference_final_state = initial,
            impaired_final_state = initial,
            required_scenarios = { "shot", "full_time" },
        })

        t.is_true(not report.passed)
        t.eq(report.events.missing_confirmed, 1)
        t.eq(report.events.terminal_speculative_residue, 2)
        t.eq(report.events.consumer_speculative_residue, 2)
        t.is_true(#report.errors >= 4)
    end)

    t.it("derives reference identities from raw campaign step inputs", function()
        local initial = new_state()
        initial.owner = nil
        local final = match_snapshot.restore(match_snapshot.capture(initial))
        local player = final.players[2].id
        final.input_tick = 1
        final.owner = 2
        final.time_left = final.time_left - 1 / 60
        final.events = {
            { kind = "shot", x = final.ball.x, y = final.ball.y, player = player },
        }
        ---@type RollbackEventStepInput
        local supplied = {
            output = {
                tick = 0,
                start_boundary = 0,
                end_boundary = 1,
                input = { tick = 0, slots = {} },
                events = final.events,
                state = {
                    score = { home = 0, away = 0 },
                    time_left = final.time_left,
                    finished = false,
                },
                finished = false,
            },
            snapshot = match_snapshot.capture(final),
        }
        local timeline = rollback_events.new(match_snapshot.capture(initial))
        assert(rollback_events.apply(timeline, 0, 0, { supplied }))
        local derived = rollback_events.confirm(timeline, 0)[1]
        local added = {}
        for _, event in ipairs(derived.match_events) do
            added[#added + 1] = event
        end
        for _, event in ipairs(derived.lifecycle_events) do
            added[#added + 1] = event
        end

        local report = rollback_validation.run(initial, {
            { kind = "reference_step", step = supplied },
            { kind = "impaired_diff", diff = diff(added) },
            { kind = "confirmed", step = derived },
        }, {
            home_team_id = "nebula",
            away_team_id = "orion",
            seed = 77,
            required_scenarios = { "possession", "shot" },
        })

        t.is_true(report.passed, table.concat(report.errors, "; "))
        t.eq(report.events.reference_unique, 1)
        t.eq(report.events.impaired_unique, 1)
        t.eq(report.consumers.audio_cue_count, 1)
    end)

    t.it("keeps confirmed lifecycle presentation idempotent at the screen boundary", function()
        local screen = Match.new({
            rollback_lab = {
                profile_name = "clean",
                duration = 1 / 60,
            },
        })
        local goal = lifecycle_event("screen-goal", 10, "goal", 1)
        local kickoff = lifecycle_event("screen-kickoff", 10, "kickoff", 1)
        local full_time = lifecycle_event("screen-full-time", 11, "full_time", 1)

        t.is_true(screen:consume_confirmed_lifecycle(goal))
        t.is_true(not screen:consume_confirmed_lifecycle(goal))
        t.is_true(screen:consume_confirmed_lifecycle(kickoff))
        t.is_true(not screen:consume_confirmed_lifecycle(kickoff))
        t.is_true(screen:consume_confirmed_lifecycle(full_time))
        t.is_true(not screen:consume_confirmed_lifecycle(full_time))
        t.is_true(screen:full_time_confirmed())
        local cues = audio.confirmed_cue_counts()
        t.eq(cues.goal, 1)
        t.eq(cues.kickoff, 1)
        t.eq(cues.full_time, 1)
    end)
end)
