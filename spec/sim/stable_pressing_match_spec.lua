local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local match = require("sim.match")
local outfield_press = require("sim.outfield_press")
local teams = require("data.teams")

---@return MatchInput
local function neutral_input()
    return {
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
end

---@param state MatchState
---@return Vec2[]
local function positions(state)
    local result = {}
    for index, player in ipairs(state.players) do
        result[index] = player.pos
    end
    return result
end

---@param defending_team "home"|"away"
---@return MatchState
local function defended_state(defending_team)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 10,
        seed = 551,
        human_controlled = false,
    })
    state.kickoff_hold = 0
    local carrier_index = defending_team == "home" and 7 or 2
    state.owner = carrier_index
    state.players[carrier_index].pos = Vec2.new(480, 270)
    state.players[carrier_index].vel = Vec2.new(0, 0)
    state.players[carrier_index].settle_timer = 0
    state.ball = Vec2.new(480, 270)
    state.ball_vel = Vec2.new(0, 0)

    local defenders = defending_team == "home" and { 2, 3, 4, 5 } or { 7, 8, 9, 10 }
    local side = defending_team == "home" and -1 or 1
    state.players[defenders[1]].pos = Vec2.new(480 + side * 40, 270)
    state.players[defenders[2]].pos = Vec2.new(480 - side * 120, 270)
    state.players[defenders[3]].pos = Vec2.new(480 - side * 240, 100)
    state.players[defenders[4]].pos = Vec2.new(480 - side * 300, 440)
    for _, index in ipairs(defenders) do
        local player = state.players[index]
        player.composure = 1
        player.dash_cd = 0
        player.stun_timer = 0
        player.aerial_recovery = 0
        player.run_vel = Vec2.new(0, 0)
        player.jockey_timer = 0
    end
    return state
end

---@param state MatchState
---@param team "home"|"away"
---@return OutfieldPressState
local function resolve_press(state, team)
    match._offball_targets(state, positions(state))
    return state.outfield_press[team]
end

---@param state MatchState
---@param kind MatchEventKind
---@return boolean
local function has_event(state, kind)
    for _, event in ipairs(state.events) do
        if event.kind == kind then
            return true
        end
    end
    return false
end

t.describe("stable pressing assignment and contain geometry", function()
    t.it("retains the primary through 86% cost and switches at 85%", function()
        local state = defended_state("home")
        state.players[2].pos = Vec2.new(400, 270)
        state.players[3].pos = Vec2.new(380, 270)
        t.eq(resolve_press(state, "home").presser_index, 2)

        state.players[2].pos = Vec2.new(380, 270)
        state.players[3].pos = Vec2.new(394, 270)
        t.eq(resolve_press(state, "home").presser_index, 2, "86% challenger must wait")

        state.players[3].pos = Vec2.new(395, 270)
        t.eq(resolve_press(state, "home").presser_index, 3, "85% challenger takes over")
    end)

    t.it("contains goal-side toward centre and faces the ball without jockey state", function()
        local state = defended_state("home")
        local carrier = state.players[7]
        carrier.pos = Vec2.new(480, 100)
        state.ball = carrier.pos
        state.players[2].pos = Vec2.new(440, 100)
        local targets = match._offball_targets(state, positions(state))
        local press = state.outfield_press.home
        t.eq(press.presser_index, 2)
        t.eq(press.mode, "contain")
        t.is_true(targets[2].x < carrier.pos.x)
        t.is_true(targets[2].y > carrier.pos.y, "outside lane is conceded toward centre")

        local before = state.players[2].pos
        local faced_ball = state.ball
        match.step(state, 1 / 60, neutral_input())
        local presser = state.players[2]
        t.is_true(before:dist(presser.pos) <= presser.move_speed / 60 + 1e-6)
        t.eq(presser.jockey_timer, 0, "AI contain never simulates held human jockey input")
        local to_ball = faced_ball:sub(presser.pos):normalized()
        t.near(presser.facing.x, to_ball.x, 1e-6)
        t.near(presser.facing.y, to_ball.y, 1e-6)
    end)

    t.it("clamps inherited sprint velocity to the stat-derived movement speed", function()
        local state = defended_state("home")
        local presser = state.players[2]
        presser.run_vel = Vec2.new(presser.move_speed * 2, 0)
        local before = presser.pos
        match.step(state, 1 / 60, neutral_input())
        t.is_true(
            before:dist(presser.pos) <= presser.move_speed / 60 + 1e-6,
            "press role cannot retain a former human sprint boost"
        )
    end)

    t.it("mirrors the goal-side contain contract for the away team", function()
        local state = defended_state("away")
        local targets = match._offball_targets(state, positions(state))
        local press = state.outfield_press.away
        t.eq(press.presser_index, 7)
        t.eq(press.mode, "contain")
        t.is_true(targets[7].x > state.players[2].pos.x)
    end)
end)

t.describe("stable pressing commit triggers", function()
    ---@param configure fun(state: MatchState, presser: MatchPlayer, carrier: MatchPlayer)
    local function reason_for(configure)
        local state = defended_state("home")
        local presser = state.players[2]
        local carrier = state.players[7]
        configure(state, presser, carrier)
        local press = resolve_press(state, "home")
        t.eq(press.mode, "commit")
        return press.reason
    end

    t.it("attributes heavy touch, exposed ball, cover, box, and discipline distinctly", function()
        t.eq(
            reason_for(function(_, _, carrier)
                carrier.settle_timer = 0.2
            end),
            "heavy_touch"
        )
        t.eq(
            reason_for(function(state, presser, carrier)
                presser.pos = Vec2.new(470, 270)
                state.ball = Vec2.new(510, 270)
                carrier.settle_timer = 0
            end),
            "exposed_ball"
        )
        t.eq(
            reason_for(function(state)
                state.players[3].pos = Vec2.new(360, 270)
            end),
            "cover"
        )
        t.eq(
            reason_for(function(state, presser, carrier)
                carrier.pos = Vec2.new(90, 270)
                state.ball = carrier.pos
                presser.pos = Vec2.new(50, 270)
                state.players[3].pos = Vec2.new(300, 270)
            end),
            "box_desperation"
        )
        t.eq(
            reason_for(function(_, presser)
                presser.composure = 0.349
            end),
            "low_discipline"
        )
    end)

    t.it("stays in contain until the cooldown-and-reach gate is open", function()
        local state = defended_state("home")
        state.players[7].settle_timer = 0.2
        state.players[2].pos = Vec2.new(300, 270)
        local press = resolve_press(state, "home")
        t.eq(press.mode, "contain")
        t.eq(press.reason, "no_trigger")

        state.players[2].pos = Vec2.new(440, 270)
        state.players[2].dash_cd = 0.1
        press = resolve_press(state, "home")
        t.eq(press.mode, "contain")
        t.eq(press.reason, "no_trigger")
    end)

    t.it("lets only the assigned committing presser challenge", function()
        local state = defended_state("home")
        state.players[7].settle_timer = 0.2
        state.players[2].pos = Vec2.new(460, 270)
        state.players[3].pos = Vec2.new(455, 270)
        match.step(state, 1 / 60, neutral_input())
        t.is_true(state.players[2].dash_cd > 0, "assigned presser commits")
        t.eq(state.players[3].dash_cd, 0, "nearby non-presser never auto-challenges")
    end)

    t.it("emits one soccer reason and clears it on the same-tick possession loss", function()
        local state = defended_state("home")
        state.players[7].settle_timer = 0.2
        state.players[2].pos = Vec2.new(460, 270)
        match.step(state, 1 / 60, neutral_input())
        t.is_true(has_event(state, "press_commit_heavy_touch"))
        t.eq(state.owner, nil, "reachable press won the ball loose")
        t.eq(state.outfield_press.home.mode, "inactive")
        t.eq(state.outfield_press.home.reason, "no_trigger")
    end)
end)

t.describe("stable pressing cover, exclusions, and resets", function()
    t.it("shadows only an actual pass-eligible highest-scored lane", function()
        local state = defended_state("home")
        state.players[8].pos = Vec2.new(360, 150)
        state.players[9].pos = Vec2.new(455, 270) -- too close: not a legitimate AI pass
        state.players[10].pos = Vec2.new(940, 400) -- too far
        local candidates = match._passing_lane_candidates(state, "home", 7, positions(state))
        t.eq(#candidates, 1)
        t.eq(candidates[1].player_index, 8)

        local base = Vec2.new(336, 270)
        local shadow = match._lane_shadow_target(state, "home", 7, base, positions(state))
        t.is_true(shadow.y < base.y)
        t.is_true(shadow.x > base.x)
    end)

    t.it("hands off stunned, aerial-recovering, and combat-blocked pressers", function()
        local state = defended_state("home")
        t.eq(resolve_press(state, "home").presser_index, 2)
        state.players[2].stun_timer = 0.2
        t.eq(resolve_press(state, "home").presser_index, 3)

        state.players[3].aerial_recovery = 0.2
        t.eq(resolve_press(state, "home").presser_index, 4)

        local combat_state = combat.new_state(state)
        combat_state.players[4].phase = "recovery"
        combat_state.players[4].phase_ticks = 5
        match._offball_targets(state, positions(state), combat_state)
        t.eq(state.outfield_press.home.presser_index, 5)
    end)

    t.it("never assigns the controlled player, a keeper, or a fixed input slot", function()
        local state = defended_state("home")
        state.human_controlled = true
        state.controlled = 2
        t.is_true(resolve_press(state, "home").presser_index ~= 2)
        t.is_true(resolve_press(state, "home").presser_index ~= 1)

        local fixed = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            human_controlled = false,
            input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
        })
        fixed.kickoff_hold = 0
        fixed.owner = 7
        fixed.ball = fixed.players[7].pos
        match._offball_targets(fixed, positions(fixed))
        t.eq(fixed.outfield_press.home.mode, "inactive")
    end)

    t.it("clears on loose/own possession, kickoff hold, and full time", function()
        local state = defended_state("home")
        t.eq(resolve_press(state, "home").mode, "contain")
        state.owner = nil
        match._offball_targets(state, positions(state))
        t.eq(state.outfield_press.home.mode, "inactive")

        state = defended_state("home")
        resolve_press(state, "home")
        state.kickoff_hold = 2
        match._offball_targets(state, positions(state))
        t.eq(state.outfield_press.home.mode, "inactive")

        state = defended_state("home")
        resolve_press(state, "home")
        state.time_left = 0.001
        match.step(state, 1 / 60, neutral_input())
        t.eq(state.outfield_press.home.mode, "inactive")
    end)

    t.it("clears both team branches without replacing inactive state", function()
        local state = defended_state("home")
        state.outfield_press.home = outfield_press.contain(2)
        state.outfield_press.away = outfield_press.contain(7)
        state.owner = nil
        match._sanitize_press_states(state)
        t.eq(state.outfield_press.home.mode, "inactive")
        t.eq(state.outfield_press.away.mode, "inactive")

        local home = state.outfield_press.home
        local away = state.outfield_press.away
        match._sanitize_press_states(state)
        t.eq(state.outfield_press.home, home)
        t.eq(state.outfield_press.away, away)
    end)

    t.it("sanitizes inactive states without per-tick table churn", function()
        local state = defended_state("home")
        state.owner = nil
        match._reset_press_states(state)
        for _ = 1, 200 do
            match._sanitize_press_states(state)
        end

        collectgarbage("collect")
        collectgarbage("stop")
        local before_kib = collectgarbage("count")
        local ok, failure = pcall(function()
            for _ = 1, 10000 do
                match._sanitize_press_states(state)
            end
        end)
        local allocated_bytes = (collectgarbage("count") - before_kib) * 1024
        collectgarbage("restart")
        collectgarbage("collect")

        t.is_true(ok, tostring(failure))
        t.is_true(
            allocated_bytes < 4096,
            ("inactive press sanitation allocated %.0f bytes"):format(allocated_bytes)
        )
    end)
end)
