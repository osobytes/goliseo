local Vec2 = require("core.vec2")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")
local t = require("spec.support.runner")

local TICK = fixed_clock.TICK_SECONDS
local KEEPER_HANDS = 30

local NO_INPUT = {
    move = Vec2.new(0, 0),
    shoot = false,
    shoot_held = false,
    pass = false,
    pass_held = false,
    switch = false,
    dash = false,
    dodge = false,
    lob = false,
    sprint = false,
    jockey = false,
}

local SHOT_INPUT = {
    move = Vec2.new(0, 0),
    shoot = true,
    shoot_held = false,
    pass = false,
    pass_held = false,
    switch = false,
    dash = false,
    dodge = false,
    lob = false,
    sprint = false,
    jockey = false,
}

---@param state MatchState
---@param team "home"|"away"
---@return MatchPlayer
local function team_keeper(state, team)
    for _, player in ipairs(state.players) do
        if player.team == team and player.is_keeper then
            return player
        end
    end
    error("missing " .. team .. " keeper")
end

---@param state MatchState
---@param team "home"|"away"
---@return integer
---@return MatchPlayer
local function team_shooter(state, team)
    for index, player in ipairs(state.players) do
        if player.team == team and not player.is_keeper then
            return index, player
        end
    end
    error("missing " .. team .. " shooter")
end

---@param state MatchState
---@param keep MatchPlayer
---@param shooter MatchPlayer
local function park_bystanders(state, keep, shooter)
    local y = 30
    for _, player in ipairs(state.players) do
        if player ~= keep and player ~= shooter then
            player.pos = Vec2.new(80, y)
            player.anchor = player.pos
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
            y = y + 42
        end
    end
end

---@param anticipation number
---@return MatchState
---@return MatchPlayer
---@return MatchPlayer
local function new_human_commit(anticipation)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 10,
        seed = 73,
    })
    local shooter = state.players[state.controlled]
    local keep = team_keeper(state, "away")
    park_bystanders(state, keep, shooter)
    shooter.pos = Vec2.new(700, 270)
    shooter.anchor = shooter.pos
    shooter.facing = Vec2.new(1, 0)
    shooter.vel = Vec2.new(0, 0)
    shooter.run_vel = Vec2.new(0, 0)
    keep.pos = Vec2.new(938, 270)
    keep.anchor = keep.pos
    keep.keeper_anticipation = anticipation
    state.owner = state.controlled
    state.ball = shooter.pos:add(Vec2.new(18, 0))
    state.ball_vel = Vec2.new(0, 0)
    state.pickup_cd = 1
    state.block_grace = 1
    return state, keep, shooter
end

---@param defending_team "home"|"away"
---@return MatchState
---@return MatchPlayer
local function new_mirrored_pending(defending_team)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 10,
        seed = 73,
    })
    local shooter_team = defending_team == "home" and "away" or "home"
    local shooter_index, shooter = team_shooter(state, shooter_team)
    local keep = team_keeper(state, defending_team)
    park_bystanders(state, keep, shooter)
    shooter.pos = Vec2.new(defending_team == "home" and 260 or 700, 270)
    shooter.anchor = shooter.pos
    shooter.facing = Vec2.new(defending_team == "home" and -1 or 1, 0)
    shooter.vel = Vec2.new(0, 0)
    shooter.run_vel = Vec2.new(0, 0)
    shooter.windup_timer = 0.15
    shooter.windup_shot = {
        dir = Vec2.new(defending_team == "home" and -260 or 260, 0),
        speed = 400,
        vz = 0,
        spin = 0,
        shot_type = "ground",
    }
    keep.pos = Vec2.new(defending_team == "home" and 22 or 938, 270)
    keep.anchor = keep.pos
    keep.keeper_anticipation = 1
    state.owner = shooter_index
    state.ball = shooter.pos:add(shooter.facing:scale(18))
    state.ball_vel = Vec2.new(0, 0)
    state.pickup_cd = 1
    state.block_grace = 1
    return state, keep
end

---@param state MatchState
---@param kind string
---@return MatchEvent?
local function event_of(state, kind)
    for _, event in ipairs(state.events) do
        if event.kind == kind then
            return event
        end
    end
    return nil
end

---@param set_duration number
---@param anticipation number
---@param dive_distance number
---@param speed number
---@param handling number
---@return MatchState
---@return MatchPlayer
local function verdict_state(set_duration, anticipation, dive_distance, speed, handling)
    local state, keep, shooter = new_human_commit(anticipation)
    park_bystanders(state, keep, shooter)
    keep.reach = 100
    keep.handling = handling
    keep.keeper_set = set_duration
    state.owner = nil
    state.ball = Vec2.new(keep.pos.x - 120, keep.pos.y)
    state.ball_vel = Vec2.new(speed, dive_distance * speed / 120)
    state.ball_z = 0
    state.ball_vz = 0
    state.pickup_cd = 1
    state.block_grace = 1
    return state, keep
end

---@param first MatchState
---@param first_keeper MatchPlayer
---@param second MatchState
---@param second_keeper MatchPlayer
local function same_save_state(first, first_keeper, second, second_keeper)
    t.eq(first.rng, second.rng)
    t.eq(first_keeper.save_pending, second_keeper.save_pending)
    t.eq(first_keeper.dive_timer, second_keeper.dive_timer)
    t.eq(first_keeper.dive_delay, second_keeper.dive_delay)
    t.eq(first_keeper.save_timer, second_keeper.save_timer)
    t.eq(first_keeper.save_vx, second_keeper.save_vx)
    t.eq(first.ball_vel.x, second.ball_vel.x)
    t.eq(first.ball_vel.y, second.ball_vel.y)
    if first_keeper.dive_target or second_keeper.dive_target then
        local first_target = assert(first_keeper.dive_target)
        local second_target = assert(second_keeper.dive_target)
        t.eq(first_target.x, second_target.x)
        t.eq(first_target.y, second_target.y)
    end
end

t.describe("keeper anticipation commit timing", function()
    t.it("sets full anticipation on the first readable windup tick and half later", function()
        local zero, zero_keeper = new_human_commit(0)
        local half, half_keeper = new_human_commit(0.5)
        local full, full_keeper = new_human_commit(1)

        match.step(zero, TICK, SHOT_INPUT)
        match.step(half, TICK, SHOT_INPUT)
        match.step(full, TICK, SHOT_INPUT)

        t.eq(zero_keeper.keeper_set, 0)
        t.eq(half_keeper.keeper_set, 0)
        t.eq(full_keeper.keeper_set, 0, "movement runs before the new windup is captured")

        local half_tick
        for tick = 1, 12 do
            match.step(zero, TICK, NO_INPUT)
            match.step(half, TICK, NO_INPUT)
            match.step(full, TICK, NO_INPUT)
            if not half_tick and half_keeper.keeper_set > 0 then
                half_tick = tick
            end
            if tick == 1 then
                t.is_true(full_keeper.keeper_set > 0, "full anticipation reads the windup")
            end
            if zero.owner then
                t.eq(zero_keeper.keeper_set, 0, "zero anticipation never sets before release")
            end
        end
        t.eq(half_tick, 5)
    end)

    t.it("applies the same fixed-tick projection rule at both goals", function()
        for _, team in ipairs({ "home", "away" }) do
            local state, keep = new_mirrored_pending(team)
            match.step(state, TICK, NO_INPUT)
            t.is_true(keep.keeper_set > 0, team .. " keeper reads the mirrored shot")
            t.eq(keep.dive_timer, 0)
            t.eq(keep.dive_delay, 0)
            t.eq(keep.save_pending, nil)

            local wide, wide_keeper = new_mirrored_pending(team)
            local wide_shooter = assert(wide.owner)
            assert(wide.players[wide_shooter].windup_shot).dir.y = 200
            match.step(wide, TICK, NO_INPUT)
            t.eq(wide_keeper.keeper_set, 0, team .. " keeper ignores an off-mouth shot")
        end
    end)

    t.it(
        "clears cancellation, possession loss, and released reversal without a ghost dive",
        function()
            local changed, changed_keeper, changed_shooter = new_human_commit(1)
            match.step(changed, TICK, SHOT_INPUT)
            changed_shooter.windup_shot = nil
            match.step(changed, TICK, NO_INPUT)
            t.eq(changed_keeper.keeper_set, 0)
            t.eq(changed_keeper.dive_timer, 0)
            t.eq(changed_keeper.dive_delay, 0)

            local lost, lost_keeper = new_human_commit(1)
            match.step(lost, TICK, SHOT_INPUT)
            lost.owner = nil
            lost.ball_vel = Vec2.new(0, 0)
            match.step(lost, TICK, NO_INPUT)
            t.eq(lost_keeper.keeper_set, 0)
            t.eq(lost_keeper.dive_timer, 0)
            t.eq(lost_keeper.dive_delay, 0)

            local reversed, reversed_keeper = new_human_commit(1)
            reversed.owner = nil
            reversed_keeper.keeper_set = 0.05
            reversed.ball = Vec2.new(700, 270)
            reversed.ball_vel = Vec2.new(-300, 0)
            match.step(reversed, TICK, NO_INPUT)
            t.eq(reversed_keeper.keeper_set, 0)
            t.eq(reversed_keeper.dive_timer, 0)
            t.eq(reversed_keeper.dive_delay, 0)

            local blocked, blocked_keeper = new_human_commit(1)
            blocked.owner = nil
            blocked_keeper.keeper_set = 0.05
            blocked.ball = Vec2.new(700, 270)
            blocked.ball_vel = Vec2.new(500, 0)
            blocked.block_grace = 0
            local blocker = blocked.players[7]
            blocker.pos = Vec2.new(720, 270)
            blocker.anchor = blocker.pos
            blocker.vel = Vec2.new(0, 0)
            blocker.run_vel = Vec2.new(0, 0)
            match.step(blocked, TICK, NO_INPUT)
            t.is_true(event_of(blocked, "block") ~= nil)
            t.is_true(blocked.ball_vel.x < 0)
            t.eq(blocked_keeper.keeper_set, 0)
            t.eq(blocked_keeper.dive_timer, 0)
            t.eq(blocked_keeper.dive_delay, 0)

            local bounced, bounced_keeper = new_human_commit(1)
            bounced.owner = nil
            bounced_keeper.keeper_set = 0.05
            bounced.ball = Vec2.new(700, 5)
            bounced.ball_vel = Vec2.new(300, -100)
            match.step(bounced, TICK, NO_INPUT)
            t.is_true(bounced.ball_vel.y > 0, "the cage wall redirects the released ball")
            t.eq(bounced_keeper.keeper_set, 0)
            t.eq(bounced_keeper.dive_timer, 0)
            t.eq(bounced_keeper.dive_delay, 0)
        end
    )

    t.it("does not reinterpret a goal-directed tackle pop as the cancelled shot", function()
        local state, keep, shooter = new_human_commit(1)
        match.step(state, TICK, SHOT_INPUT)
        match.step(state, TICK, NO_INPUT)
        t.is_true(keep.keeper_set > 0)

        local tackler = state.players[7]
        tackler.pos = shooter.pos:add(Vec2.new(20, 0))
        tackler.anchor = tackler.pos
        tackler.vel = Vec2.new(0, 0)
        tackler.run_vel = Vec2.new(0, 0)
        tackler.dash_cd = 0
        tackler.composure = 0 -- preserve the explicit legacy dive-in fixture
        state.kickoff_hold = 0
        match.step(state, TICK, NO_INPUT)

        t.is_true(event_of(state, "tackle") ~= nil, "the real tackle transition fires")
        t.eq(state.owner, nil)
        t.is_true(state.ball_vel.x > 0, "the tackle pop still travels toward the away goal")
        t.eq(keep.keeper_set, 0)
        for _ = 1, 90 do
            match.step(state, TICK, NO_INPUT)
            t.eq(keep.keeper_set, 0)
            t.eq(keep.dive_timer, 0)
            t.eq(keep.dive_delay, 0)
            t.eq(keep.save_pending, nil)
        end
    end)

    t.it("does not reinterpret a goal-directed heavy touch as the cancelled shot", function()
        local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 10,
            seed = 73,
            input_ownership = ownership,
        })
        local shooter_index = assert(state.slot_players[1])
        local shooter = state.players[shooter_index]
        local keep = team_keeper(state, "away")
        park_bystanders(state, keep, shooter)
        shooter.pos = Vec2.new(650, 270)
        shooter.anchor = shooter.pos
        shooter.facing = Vec2.new(1, 0)
        shooter.vel = Vec2.new(0, 0)
        shooter.run_vel = Vec2.new(0, 0)
        shooter.windup_timer = 0.15
        shooter.windup_shot = {
            dir = Vec2.new(310, 0),
            speed = 400,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }
        keep.pos = Vec2.new(938, 270)
        keep.anchor = keep.pos
        keep.keeper_anticipation = 1
        state.owner = shooter_index
        state.ball = shooter.pos:add(Vec2.new(18, 0))
        state.ball_vel = Vec2.new(0, 0)
        state.pickup_cd = 1

        match.step(state, TICK, assert(input_frame.neutral(state.input_tick)))
        t.is_true(keep.keeper_set > 0)

        state.ball = shooter.pos:add(Vec2.new(50, 0))
        state.ball_vel = Vec2.new(150, 0)
        match.step(state, TICK, assert(input_frame.neutral(state.input_tick)))

        t.eq(state.owner, nil)
        t.eq(shooter.windup_timer, 0)
        t.eq(shooter.windup_shot, nil)
        t.is_true(state.ball_vel.x > 0, "the heavy touch stays directed toward the away goal")
        t.eq(keep.keeper_set, 0)
        for _ = 1, 90 do
            match.step(state, TICK, assert(input_frame.neutral(state.input_tick)))
            t.eq(keep.keeper_set, 0)
            t.eq(keep.dive_timer, 0)
            t.eq(keep.dive_delay, 0)
            t.eq(keep.save_pending, nil)
        end
    end)

    t.it("clears a same-direction aerial redirection without changing legacy dive state", function()
        local state, keep, striker = new_human_commit(1)
        state.owner = nil
        state.pickup_cd = 0
        state.aerial_lock = 0
        striker.pos = Vec2.new(500, 270)
        striker.anchor = striker.pos
        striker.facing = Vec2.new(1, 0)
        striker.header_cd = 0
        striker.aerial_recovery = 0
        state.ball = striker.pos:add(Vec2.new(6, 0))
        state.ball_vel = Vec2.new(120, 0)
        state.ball_z = 56
        state.ball_vz = -50
        keep.keeper_set = 0.05
        local strike_input = {
            move = Vec2.new(0, 0),
            shoot = false,
            shoot_held = false,
            pass = false,
            pass_held = false,
            switch = false,
            dash = false,
            dodge = false,
            lob = false,
            sprint = false,
            jockey = false,
            aerial_strike = true,
        }

        match.step(state, TICK, strike_input)

        local strike = event_of(state, "header") or event_of(state, "volley")
        t.is_true(strike ~= nil and strike.outcome ~= "miss")
        t.is_true(state.ball_vel.x > 0, "the redirection keeps its x direction")
        t.eq(keep.keeper_set, 0)
        t.eq(keep.dive_timer, 0)
        t.eq(keep.dive_delay, 0)
        t.eq(keep.save_pending, nil)
    end)

    t.it("round-trips an active set through snapshot restore and replayed ticks", function()
        local state, keep = new_human_commit(1)
        match.step(state, TICK, SHOT_INPUT)
        match.step(state, TICK, NO_INPUT)
        t.is_true(keep.keeper_set > 0)
        local snapshot = match_snapshot.capture(state)
        local restored = match_snapshot.restore(snapshot)
        t.eq(restored.players[6].keeper_anticipation, keep.keeper_anticipation)
        t.eq(restored.players[6].keeper_set, keep.keeper_set)

        for tick = 1, 12 do
            match.step(state, TICK, NO_INPUT)
            match.step(restored, TICK, NO_INPUT)
            t.eq(
                match_snapshot.hash(match_snapshot.capture(restored)),
                match_snapshot.hash(match_snapshot.capture(state)),
                "restored early-set boundary " .. tick
            )
        end
    end)

    t.it("keeps a slow shot set until save evaluation without launching the dive early", function()
        local state, keep, shooter = new_human_commit(1)
        shooter.pos = Vec2.new(680, 270)
        shooter.anchor = shooter.pos
        shooter.shot_speed = 400
        state.ball = shooter.pos:add(Vec2.new(18, 0))
        match.step(state, TICK, SHOT_INPUT)

        local set_flight_frames = 0
        local dive_frame
        local resolution_frame
        local resolution_distance
        for frame = 1, 180 do
            match.step(state, TICK, NO_INPUT)
            if not state.owner and keep.keeper_set > 0 then
                set_flight_frames = set_flight_frames + 1
                t.eq(keep.dive_timer, 0, "set/lean never spends the contact-timed dive")
                t.eq(keep.dive_delay, 0, "set/lean never queues the contact-timed dive")
                t.eq(keep.save_pending, nil)
            end
            if not dive_frame and keep.dive_timer > 0 then
                dive_frame = frame
            end
            local resolution = event_of(state, "catch") or event_of(state, "parry")
            if resolution then
                resolution_frame = frame
                resolution_distance = keep.pos:dist(Vec2.new(resolution.x, resolution.y))
                break
            end
        end

        t.is_true(set_flight_frames > 6, "the set survives a materially slow released flight")
        t.is_true(dive_frame ~= nil and resolution_frame ~= nil)
        t.is_true(resolution_frame - dive_frame <= math.ceil(0.32 / TICK) + 10)
        t.is_true(resolution_distance <= KEEPER_HANDS + 1)
        t.eq(keep.keeper_set, 0)
    end)

    t.it("keeps catch, parry, and beaten twins invariant across anticipation alone", function()
        local scenarios = {
            { name = "catch", dive = 0, speed = 200, handling = 1, expected = "catch" },
            { name = "parry", dive = 60, speed = 200, handling = 0, expected = "parry" },
            { name = "beaten", dive = 70, speed = 500, handling = 0, expected = nil },
        }
        for _, scenario in ipairs(scenarios) do
            local reactive, reactive_keeper =
                verdict_state(0, 0, scenario.dive, scenario.speed, scenario.handling)
            local anticipatory, anticipatory_keeper =
                verdict_state(0.1, 1, scenario.dive, scenario.speed, scenario.handling)
            match.step(reactive, 0, NO_INPUT)
            match.step(anticipatory, 0, NO_INPUT)

            t.eq(reactive_keeper.save_pending, scenario.expected, scenario.name)
            t.eq(anticipatory_keeper.save_pending, scenario.expected, scenario.name)
            same_save_state(reactive, reactive_keeper, anticipatory, anticipatory_keeper)
            t.eq(reactive_keeper.keeper_set, 0)
            t.eq(anticipatory_keeper.keeper_set, 0)
        end
    end)

    t.it("clears set presentation at goal restart and full time", function()
        local collected, collected_keeper, collector = new_human_commit(1)
        collected.owner = nil
        collected_keeper.keeper_set = 0.05
        collected.pickup_cd = 0
        collected.ball = collector.pos
        collected.ball_vel = Vec2.new(0, 0)
        match.step(collected, TICK, NO_INPUT)
        t.is_true(collected.owner ~= nil)
        t.eq(collected_keeper.keeper_set, 0)

        local goal, goal_keeper = new_human_commit(1)
        goal.owner = nil
        goal_keeper.keeper_set = 0.05
        goal_keeper.receive_timer = 1
        goal.ball = Vec2.new(965, 270)
        goal.ball_vel = Vec2.new(600, 0)
        match.step(goal, 0.01, NO_INPUT)
        t.eq(goal.score.home, 1)
        t.eq(goal_keeper.keeper_set, 0)

        local finished, finished_keeper = new_human_commit(1)
        finished_keeper.keeper_set = 0.05
        finished.time_left = TICK / 2
        match.step(finished, TICK, NO_INPUT)
        t.is_true(finished.finished)
        t.eq(finished_keeper.keeper_set, 0)
    end)
end)
