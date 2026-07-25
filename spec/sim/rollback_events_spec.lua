local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_events = require("sim.rollback_events")
local rollback_session = require("sim.rollback_session")
local teams = require("data.teams")

---@param options { duration: number?, max_goals: integer?, seed: integer? }?
---@return MatchState
local function new_state(options)
    options = options or {}
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = options.duration or 4,
        max_goals = options.max_goals or 3,
        seed = options.seed or 720,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
end

---@return MatchSnapshot
local function initial_snapshot()
    return match_snapshot.capture(new_state())
end

---@param source MatchEvent
---@return MatchEvent
local function copy_event(source)
    local event = {}
    for key, value in pairs(source) do
        event[key] = value
    end
    ---@cast event MatchEvent
    return event
end

---@param before MatchSnapshot
---@param options { events: MatchEvent[]?, home_score: integer?, away_score: integer?, time_left: number?, finished: boolean? }?
---@return MatchSnapshot
local function next_snapshot(before, options)
    options = options or {}
    local state = match_snapshot.restore(before)
    state.input_tick = state.input_tick + 1
    state.events = {}
    for index, event in ipairs(options.events or {}) do
        state.events[index] = copy_event(event)
    end
    state.score.home = options.home_score or state.score.home
    state.score.away = options.away_score or state.score.away
    if options.time_left ~= nil then
        state.time_left = options.time_left
    else
        state.time_left = math.max(0, state.time_left - 1 / 60)
    end
    if options.finished ~= nil then
        state.finished = options.finished
    end
    return match_snapshot.capture(state)
end

---@param tick integer
---@return RollbackInputTickRecord
local function input_record(tick)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        slots[index] = {
            source = "remote",
            status = "authoritative",
            sample = input_frame.neutral_sample(),
        }
    end
    return { tick = tick, slots = slots }
end

---@param snapshot MatchSnapshot
---@return RollbackTickOutput
local function output_for(snapshot)
    local tick = snapshot.state.input_tick - 1
    local events = {}
    for index, event in ipairs(snapshot.state.events) do
        events[index] = copy_event(event)
    end
    return {
        tick = tick,
        start_boundary = tick,
        end_boundary = tick + 1,
        input = input_record(tick),
        events = events,
        state = {
            score = {
                home = snapshot.state.score.home,
                away = snapshot.state.score.away,
            },
            time_left = snapshot.state.time_left,
            finished = snapshot.state.finished,
        },
        finished = snapshot.state.finished,
    }
end

---@param snapshot MatchSnapshot
---@return RollbackEventStepInput
local function supplied(snapshot)
    return { output = output_for(snapshot), snapshot = snapshot }
end

---@param value any
---@return boolean
local function fails(value)
    return not value
end

---@return RollbackInputSource[]
local function sources()
    local result = {}
    for index = 1, input_frame.SLOT_COUNT do
        result[index] = "remote"
    end
    return result
end

---@param max_goals integer
---@return MatchSnapshot
local function shot_fixture(max_goals)
    local state = new_state({ max_goals = max_goals, seed = 721 })
    local carrier_index = state.slot_players[1]
    local carrier = state.players[carrier_index]
    carrier.pos = Vec2.new(900, 270)
    carrier.vel = Vec2.new(0, 0)
    carrier.run_vel = Vec2.new(0, 0)
    carrier.facing = Vec2.new(1, 0)
    carrier.charge = 1
    state.owner = carrier_index
    state.ball = Vec2.new(918, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.ball_z = 0
    state.ball_vz = 0
    state.pickup_cd = 2
    state.block_grace = 2
    for index, player in ipairs(state.players) do
        if index ~= carrier_index then
            player.pos = Vec2.new(index < 6 and 200 or 100, 40 + index)
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
        end
    end
    return match_snapshot.capture(state)
end

---@param initial MatchSnapshot
---@param stop fun(snapshot: MatchSnapshot): boolean
---@return RollbackEventTimeline, RollbackEventDiff[]
local function run_real_match(initial, stop)
    local session = rollback_session.new(initial, sources())
    local timeline = rollback_events.new(initial)
    assert(
        rollback_session.add_authoritative(
            session,
            0,
            1,
            assert(input_frame.new_sample({ edges = input_frame.EDGE_BITS.shoot }))
        )
    )
    local diffs = {}
    for _ = 1, 40 do
        local output = assert(rollback_session.step(session))
        local post = assert(rollback_session.snapshot(session, output.end_boundary).snapshot)
        diffs[#diffs + 1] = assert(rollback_events.apply(timeline, output.tick, output.tick, {
            { output = output, snapshot = post },
        }))
        if stop(post) then
            return timeline, diffs
        end
    end
    assert(false, "real goal fixture did not reach its lifecycle transition")
    return timeline, diffs
end

---@param diffs RollbackEventDiff[]
---@return RollbackWrappedLifecycleEvent[]
local function lifecycle_additions(diffs)
    local result = {}
    for _, diff in ipairs(diffs) do
        for _, event in ipairs(diff.added) do
            if event.domain:match("^lifecycle/") then
                result[#result + 1] = event
            end
        end
    end
    ---@cast result RollbackWrappedLifecycleEvent[]
    return result
end

t.describe("rollback events", function()
    t.it("revokes corrected-away combat events and confirms the replacement once", function()
        local state = new_state()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local source_player = nil
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "light_melee" then
                source_player = index
                break
            end
        end
        source_player = assert(source_player, "fixture requires one melee player")
        local source_slot = assert(state.slot_for_player[source_player])
        local initial = match_snapshot.capture(state, combat_state)

        local function run(attack)
            local session = rollback_session.new(initial, sources())
            for slot = 1, input_frame.SLOT_COUNT do
                local row = input_frame.neutral_sample()
                if attack and slot == source_slot then
                    row = assert(input_frame.new_sample({
                        held = input_frame.HELD_BITS.equipment,
                        edges = input_frame.EDGE_BITS.equipment_pressed,
                    }))
                end
                assert(rollback_session.add_authoritative(session, 0, slot, row))
            end
            return assert(rollback_session.step(session)),
                rollback_session.current_snapshot(session)
        end

        local attack_output, attack_snapshot = run(true)
        local neutral_output, neutral_snapshot = run(false)
        local timeline = rollback_events.new(initial)
        local added = assert(rollback_events.apply(timeline, 0, 0, {
            { output = attack_output, snapshot = attack_snapshot },
        }))
        t.eq(#added.added, 1)
        t.is_true(added.added[1].domain:match("^combat/commit/1$") ~= nil)

        local corrected = assert(rollback_events.apply(timeline, 0, 0, {
            { output = neutral_output, snapshot = neutral_snapshot },
        }))
        t.eq(#corrected.revoked, 1)
        t.eq(corrected.revoked[1].id, added.added[1].id)
        t.eq(#corrected.added, 0)
        t.eq(#rollback_events.confirm(timeline, 0), 1)
        t.eq(#rollback_events.confirm(timeline, 0), 0)
    end)

    t.it("keeps per-domain ordinals stable and identical reapplication silent", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local post = next_snapshot(initial, {
            events = {
                { kind = "shot", x = 10, y = 20, player = "a" },
                { kind = "tackle", x = 30, y = 40, player = "b" },
                { kind = "shot", x = 50, y = 60, player = "c" },
            },
        })
        local first = assert(rollback_events.apply(timeline, 0, 0, { supplied(post) }))
        t.eq(#first.added, 3)
        t.eq(first.added[1].ordinal, 1)
        t.eq(first.added[2].ordinal, 1)
        t.eq(first.added[3].ordinal, 2)
        t.eq(first.added[1].id, "0000000000|010:match/shot|0001")
        t.eq(first.added[3].id, "0000000000|010:match/shot|0002")

        local again = assert(rollback_events.apply(timeline, 0, 0, { supplied(post) }))
        t.eq(#again.added, 0)
        t.eq(#again.revoked, 0)
        t.eq(#again.replaced, 0)
    end)

    t.it(
        "reports changed shot payload as replacement and shot-to-pass as revoke plus add",
        function()
            local initial = initial_snapshot()
            local timeline = rollback_events.new(initial)
            local shot = next_snapshot(initial, {
                events = {
                    {
                        kind = "shot",
                        x = 10,
                        y = 20,
                        player = "a",
                        shot_type = "ground",
                    },
                },
            })
            assert(rollback_events.apply(timeline, 0, 0, { supplied(shot) }))
            local changed = next_snapshot(initial, {
                events = {
                    {
                        kind = "shot",
                        x = 12,
                        y = 22,
                        player = "a",
                        shot_type = "chip",
                    },
                },
            })
            local replaced = assert(rollback_events.apply(timeline, 0, 0, { supplied(changed) }))
            t.eq(#replaced.replaced, 1)
            t.eq(#replaced.added, 0)
            t.eq(#replaced.revoked, 0)
            t.eq(replaced.replaced[1].before.id, replaced.replaced[1].after.id)
            t.eq(replaced.replaced[1].before.payload.x, 10)
            t.eq(replaced.replaced[1].after.payload.shot_type, "chip")

            local pass = next_snapshot(initial, {
                events = { { kind = "pass", x = 12, y = 22, player = "a" } },
            })
            local changed_kind = assert(rollback_events.apply(timeline, 0, 0, { supplied(pass) }))
            t.eq(#changed_kind.replaced, 0)
            t.eq(changed_kind.revoked[1].domain, "match/shot")
            t.eq(changed_kind.added[1].domain, "match/pass")
        end
    )

    t.it("uses canonical signed-zero equality for payload replacement", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local positive = next_snapshot(initial, {
            events = { { kind = "shot", x = 0, y = 1, player = "a" } },
        })
        assert(rollback_events.apply(timeline, 0, 0, { supplied(positive) }))
        local negative = next_snapshot(initial, {
            events = { { kind = "shot", x = -0.0, y = 1, player = "a" } },
        })
        local diff = assert(rollback_events.apply(timeline, 0, 0, { supplied(negative) }))
        t.eq(#diff.replaced, 1)
        t.eq(1 / diff.replaced[1].before.payload.x, math.huge)
        t.eq(1 / diff.replaced[1].after.payload.x, -math.huge)
    end)

    t.it("revokes a predicted goal and kickoff once and never confirms them", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local goal = next_snapshot(initial, { home_score = 1 })
        local predicted = assert(rollback_events.apply(timeline, 0, 0, { supplied(goal) }))
        t.eq(predicted.added[1].domain, "lifecycle/goal")
        t.eq(predicted.added[2].domain, "lifecycle/kickoff")

        local corrected = next_snapshot(initial)
        local revoked = assert(rollback_events.apply(timeline, 0, 0, { supplied(corrected) }))
        t.eq(#revoked.revoked, 2)
        t.eq(revoked.revoked[1].domain, "lifecycle/goal")
        t.eq(revoked.revoked[2].domain, "lifecycle/kickoff")
        local repeated = assert(rollback_events.apply(timeline, 0, 0, { supplied(corrected) }))
        t.eq(#repeated.revoked, 0)
        local confirmed = rollback_events.confirm(timeline, 0)
        t.eq(#confirmed[1].lifecycle_events, 0)
    end)

    t.it("removes a tackle repeatedly, then confirms a different event at the same tick", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local tackle = next_snapshot(initial, {
            events = { { kind = "tackle", x = 1, y = 2, player = "defender" } },
        })
        assert(rollback_events.apply(timeline, 0, 0, { supplied(tackle) }))
        local empty = next_snapshot(initial)
        local removed = assert(rollback_events.apply(timeline, 0, 0, { supplied(empty) }))
        t.eq(#removed.revoked, 1)
        local repeated = assert(rollback_events.apply(timeline, 0, 0, { supplied(empty) }))
        t.eq(#repeated.revoked, 0)

        local pass = next_snapshot(initial, {
            events = { { kind = "pass", x = 3, y = 4, player = "attacker" } },
        })
        local added = assert(rollback_events.apply(timeline, 0, 0, { supplied(pass) }))
        t.eq(added.added[1].domain, "match/pass")
        local confirmed = rollback_events.confirm(timeline, 0)
        t.eq(#confirmed[1].match_events, 1)
        t.eq(confirmed[1].match_events[1].domain, "match/pass")
    end)

    t.it("confirms catch, parry, tip, and claim vocabulary exactly once with stable IDs", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local post = next_snapshot(initial, {
            events = {
                {
                    kind = "catch",
                    x = 1,
                    y = 2,
                    player = "keeper",
                    save_style = "central",
                },
                {
                    kind = "parry",
                    x = 3,
                    y = 4,
                    player = "keeper",
                    save_style = "stretch",
                },
                { kind = "tip", x = 5, y = 6, player = "keeper" },
                { kind = "claim", x = 7, y = 8, player = "keeper" },
            },
        })
        assert(rollback_events.apply(timeline, 0, 0, { supplied(post) }))
        local confirmed = rollback_events.confirm(timeline, 0)
        t.eq(#confirmed[1].match_events, 4)
        for index, kind in ipairs({ "catch", "parry", "tip", "claim" }) do
            local event = confirmed[1].match_events[index]
            t.eq(event.domain, "match/" .. kind)
            t.eq(event.ordinal, 1)
        end
        t.eq(#rollback_events.confirm(timeline, 0), 0)
    end)

    t.it("derives real goal plus kickoff once from match snapshots", function()
        local timeline, diffs = run_real_match(shot_fixture(3), function(snapshot)
            return snapshot.state.score.home == 1
        end)
        local lifecycle = lifecycle_additions(diffs)
        t.eq(#lifecycle, 2)
        t.eq(lifecycle[1].domain, "lifecycle/goal")
        t.eq(lifecycle[1].payload.team, "home")
        t.eq(lifecycle[1].payload.score.home, 1)
        t.eq(lifecycle[2].domain, "lifecycle/kickoff")
        t.eq(lifecycle[2].payload.team, "away")
        local confirmed = rollback_events.confirm(timeline, lifecycle[1].tick)
        local confirmed_lifecycle = confirmed[#confirmed].lifecycle_events
        t.eq(confirmed_lifecycle[1].domain, "lifecycle/goal")
        t.eq(confirmed_lifecycle[1].payload.team, "home")
        t.eq(confirmed_lifecycle[2].domain, "lifecycle/kickoff")
        t.eq(confirmed_lifecycle[2].payload.team, "away")
        t.eq(#rollback_events.confirm(timeline, lifecycle[1].tick), 0)
    end)

    t.it("derives real max-goal full time without kickoff", function()
        local timeline, diffs = run_real_match(shot_fixture(1), function(snapshot)
            return snapshot.state.finished
        end)
        local lifecycle = lifecycle_additions(diffs)
        t.eq(#lifecycle, 2)
        t.eq(lifecycle[1].domain, "lifecycle/goal")
        t.eq(lifecycle[2].domain, "lifecycle/full_time")
        local confirmed = rollback_events.confirm(timeline, lifecycle[1].tick)
        local confirmed_lifecycle = confirmed[#confirmed].lifecycle_events
        t.eq(confirmed_lifecycle[1].domain, "lifecycle/goal")
        t.eq(confirmed_lifecycle[1].payload.score.home, 1)
        t.eq(confirmed_lifecycle[2].domain, "lifecycle/full_time")
        t.eq(confirmed_lifecycle[2].payload.score.home, 1)
        t.eq(#rollback_events.confirm(timeline, lifecycle[1].tick), 0)
    end)

    t.it("derives timer full time alone and no opening kickoff", function()
        local state = new_state({ duration = 1 / 60, max_goals = 3 })
        local initial = match_snapshot.capture(state)
        local session = rollback_session.new(initial, sources())
        local output = assert(rollback_session.step(session))
        local post = assert(rollback_session.snapshot(session, 1).snapshot)
        local timeline = rollback_events.new(initial)
        local diff = assert(rollback_events.apply(timeline, 0, 0, {
            { output = output, snapshot = post },
        }))
        t.eq(#diff.added, 1)
        t.eq(diff.added[1].domain, "lifecycle/full_time")
        local confirmed = rollback_events.confirm(timeline, 0)
        t.eq(confirmed[1].lifecycle_events[1].domain, "lifecycle/full_time")
        t.eq(confirmed[1].lifecycle_events[1].payload.score.home, 0)
        t.eq(#rollback_events.confirm(timeline, 0), 0)
    end)

    t.it("lets earlier corrected full time remove every stale later-tick event", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local zero = next_snapshot(initial, {
            events = { { kind = "shot", x = 1, y = 1, player = "a" } },
        })
        local one = next_snapshot(zero, {
            events = { { kind = "tackle", x = 2, y = 2, player = "b" } },
        })
        local two = next_snapshot(one, {
            events = { { kind = "pass", x = 3, y = 3, player = "c" } },
        })
        assert(rollback_events.apply(timeline, 0, 0, { supplied(zero) }))
        assert(rollback_events.apply(timeline, 1, 1, { supplied(one) }))
        assert(rollback_events.apply(timeline, 2, 2, { supplied(two) }))

        local earlier_finish = next_snapshot(initial, { finished = true, time_left = 0 })
        local corrected =
            assert(rollback_events.apply(timeline, 0, 2, { supplied(earlier_finish) }))
        t.eq(#corrected.revoked, 3)
        t.eq(corrected.added[1].domain, "lifecycle/full_time")
        t.eq(#rollback_events.confirm(timeline, 0), 1)
    end)

    t.it("rejects empty or active short corrections before mutating the stale tail", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local zero = next_snapshot(initial, {
            events = { { kind = "shot", x = 1, y = 1, player = "a" } },
        })
        local one = next_snapshot(zero, {
            events = { { kind = "tackle", x = 2, y = 2, player = "b" } },
        })
        local two = next_snapshot(one, {
            events = { { kind = "pass", x = 3, y = 3, player = "c" } },
        })
        assert(rollback_events.apply(timeline, 0, 0, { supplied(zero) }))
        assert(rollback_events.apply(timeline, 1, 1, { supplied(one) }))
        assert(rollback_events.apply(timeline, 2, 2, { supplied(two) }))

        local active_short = next_snapshot(initial)
        t.is_true(fails(pcall(rollback_events.apply, timeline, 0, 2, { supplied(active_short) })))
        t.is_true(fails(pcall(rollback_events.apply, timeline, 0, 2, {})))
        local diagnostics = rollback_events.diagnostics(timeline)
        t.eq(diagnostics.status, "active")
        t.eq(diagnostics.confirmed_tick, -1)
        t.eq(diagnostics.retained_step_count, 3)
        t.eq(diagnostics.retained_event_count, 3)
        local confirmed = rollback_events.confirm(timeline, 2)
        t.eq(confirmed[1].match_events[1].domain, "match/shot")
        t.eq(confirmed[2].match_events[1].domain, "match/tackle")
        t.eq(confirmed[3].match_events[1].domain, "match/pass")
    end)

    t.it(
        "bounds stalled confirmation to thirty compact step records and fails explicitly",
        function()
            local state = new_state()
            state.owner = state.slot_players[1]
            local initial = match_snapshot.capture(state)
            local timeline = rollback_events.new(initial)
            local post = initial
            for tick = 0, 29 do
                post = next_snapshot(post, {
                    events = { { kind = "touch", x = tick, y = tick, player = "a" } },
                })
                assert(rollback_events.apply(timeline, tick, tick, { supplied(post) }))
            end

            local healthy = rollback_events.diagnostics(timeline)
            t.eq(healthy.status, "active")
            t.eq(healthy.max_unconfirmed_ticks, 30)
            t.eq(healthy.retained_step_count, 30)
            t.eq(healthy.retained_event_count, 30)
            t.eq(healthy.oldest_tick, 0)
            t.eq(healthy.latest_tick, 29)
            ---@type any
            local retained = timeline._steps[0]
            t.eq(retained.snapshot, nil)
            local owner_index = assert(initial.state.owner)
            t.eq(retained.state.owner_id, initial.state.players[owner_index].id)
            t.eq(retained.state.owner_team, initial.state.players[owner_index].team)

            local over = next_snapshot(post)
            local diff, err, code = rollback_events.apply(timeline, 30, 30, { supplied(over) })
            t.eq(diff, nil)
            t.is_true(type(err) == "string")
            t.eq(code, "unconfirmed_window_exceeded")
            local terminal = rollback_events.diagnostics(timeline)
            t.eq(terminal.status, "unconfirmed_window_exceeded")
            t.eq(terminal.retained_step_count, 30)
            t.eq(terminal.oldest_tick, 0)
            t.eq(terminal.latest_tick, 29)
        end
    )

    t.it("confirms monotonically across calls and rejects gaps or confirmed correction", function()
        local initial = initial_snapshot()
        local timeline = rollback_events.new(initial)
        local zero = next_snapshot(initial)
        local one = next_snapshot(zero)
        local two = next_snapshot(one)
        assert(rollback_events.apply(timeline, 0, 0, { supplied(zero) }))
        assert(rollback_events.apply(timeline, 1, 1, { supplied(one) }))
        assert(rollback_events.apply(timeline, 2, 2, { supplied(two) }))
        t.eq(#rollback_events.confirm(timeline, 0), 1)
        t.eq(#rollback_events.confirm(timeline, 2), 2)
        t.eq(#rollback_events.confirm(timeline, 2), 0)
        t.is_true(fails(pcall(rollback_events.confirm, timeline, 1)))
        t.is_true(fails(pcall(rollback_events.apply, timeline, 0, 0, { supplied(zero) })))

        local missing = rollback_events.new(initial)
        assert(rollback_events.apply(missing, 0, 0, { supplied(zero) }))
        t.is_true(fails(pcall(rollback_events.confirm, missing, 1)))
        local after_failed_confirm = rollback_events.diagnostics(missing)
        t.eq(after_failed_confirm.confirmed_tick, -1)
        t.eq(after_failed_confirm.confirmed_boundary, 0)
        t.eq(after_failed_confirm.retained_step_count, 1)
        t.eq(#rollback_events.confirm(missing, 0), 1)

        local noncontiguous = rollback_events.new(initial)
        local bad = supplied(one)
        t.is_true(fails(pcall(rollback_events.apply, noncontiguous, 0, 0, { bad })))
    end)

    t.it("defensively copies inputs, diffs, confirmed records, and compact state views", function()
        local initial = initial_snapshot()
        local initial_hash = match_snapshot.hash(initial)
        local timeline = rollback_events.new(initial)
        local post = next_snapshot(initial, {
            events = { { kind = "shot", x = 10, y = 20, player = "a" } },
        })
        local input = supplied(post)
        local diff = assert(rollback_events.apply(timeline, 0, 0, { input }))
        input.output.events[1].x = 999
        input.snapshot.state.events[1].x = 999
        diff.added[1].payload.x = 888

        local confirmed = rollback_events.confirm(timeline, 0)
        t.eq(confirmed[1].match_events[1].payload.x, 10)
        t.eq(confirmed[1].state.score.home, 0)
        confirmed[1].match_events[1].payload.x = 777
        confirmed[1].state.score.home = 99

        local next = next_snapshot(post)
        local next_diff = assert(rollback_events.apply(timeline, 1, 1, { supplied(next) }))
        t.eq(#next_diff.added, 0)
        t.eq(match_snapshot.hash(initial), initial_hash)
        t.eq(match_snapshot.VERSION, 5)
    end)
end)
