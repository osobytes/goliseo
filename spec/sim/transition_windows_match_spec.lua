local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local outfield_decision = require("sim.outfield_decision")
local tactics = require("data.tactics")
local teams = require("data.teams")
local transitions = require("sim.possession_transition")

local TICK = 1 / 60

---@type MatchInput
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
    equipment_held = false,
    equipment_pressed = false,
    equipment_released = false,
}

---@param state MatchState
---@return Vec2[]
local function positions(state)
    local result = {}
    for index, player in ipairs(state.players) do
        result[index] = player.pos
    end
    return result
end

-- Put the possession memory exactly where a just-established turnover leaves it.
---@param state MatchState
---@param winner "home"|"away"
---@param elapsed number
local function arm_turnover(state, winner, elapsed)
    state.transition.last_team = winner
    state.transition.holding_team = winner
    state.transition.hold = transitions.ESTABLISH_SECONDS
    state.transition.turnover_team = winner
    state.transition.elapsed = elapsed
end

-- Both sides in 1-1-2 so home 2..5 and away 7..10 share outfield roles by
-- ordinal (def, mid, fwd, fwd) and a mirrored fixture mirrors exactly.
---@param home_tactic TacticData?
---@param away_tactic TacticData?
---@return MatchState
local function mirrored_match(home_tactic, away_tactic)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        home_formation = "1-1-2",
        tactic = home_tactic or tactics.balanced,
        away_tactic = away_tactic or tactics.balanced,
        duration = 60,
        seed = 57,
        human_controlled = false,
    })
    state.kickoff_hold = 0
    return state
end

-- The away side has just won the ball in midfield; home 3 and 4 are its two
-- nearest eligible hunters, home 2 (def) and 5 (fwd) are the rest.
---@param home_tactic TacticData?
---@param away_tactic TacticData?
---@return MatchState
local function home_lost_possession(home_tactic, away_tactic)
    local state = mirrored_match(home_tactic, away_tactic)
    local layout = {
        [1] = Vec2.new(60, 270),
        [2] = Vec2.new(250, 120),
        [3] = Vec2.new(440, 270),
        [4] = Vec2.new(400, 270),
        [5] = Vec2.new(250, 420),
        [6] = Vec2.new(900, 270),
        [7] = Vec2.new(480, 270),
        [8] = Vec2.new(600, 180),
        [9] = Vec2.new(620, 360),
        [10] = Vec2.new(700, 270),
    }
    for index, player in ipairs(state.players) do
        player.pos = layout[index]
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
        player.composure = 1
        player.dash_cd = 0
        player.settle_timer = 0
        player.outfield_decision = outfield_decision.reset(player.outfield_decision)
    end
    state.owner = 7
    state.ball = Vec2.new(480, 270)
    state.ball_vel = Vec2.new(0, 0)
    arm_turnover(state, "away", 0)
    return state
end

---@param state MatchState
---@param team "home"|"away"
---@param closing table<integer, boolean>
---@return integer[]
local function closers(state, team, closing)
    local result = {}
    for index, player in ipairs(state.players) do
        if player.team == team and closing[index] then
            result[#result + 1] = index
        end
    end
    return result
end

---@param state MatchState
---@param team "home"|"away"
---@return integer[]
local function runners(state, team)
    local result = {}
    for index, player in ipairs(state.players) do
        if
            player.team == team
            and outfield_decision.is_run_intent(player.outfield_decision.intent)
        then
            result[#result + 1] = index
        end
    end
    return result
end

-- The proven role-run fixture from #56, with the carrier deliberately UNSETTLED
-- so only a counter-attack window can buy an in-behind request.
---@return MatchState
local function unsettled_attack()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        home_formation = "1-1-2",
        human_controlled = true,
        seed = 56,
    })
    state.kickoff_hold = 0
    state.owner = 3
    match._set_controlled_player(state, 3)
    local layout = {
        [1] = Vec2.new(50, 270),
        [2] = Vec2.new(150, 80),
        [3] = Vec2.new(300, 270),
        [4] = Vec2.new(120, 180),
        [5] = Vec2.new(120, 360),
        [6] = Vec2.new(920, 270),
        [7] = Vec2.new(650, 50),
        [8] = Vec2.new(620, 140),
        [9] = Vec2.new(620, 400),
        [10] = Vec2.new(650, 490),
    }
    for index, player in ipairs(state.players) do
        player.pos = layout[index]
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
        player.outfield_decision = outfield_decision.reset(player.outfield_decision)
    end
    for _, index in ipairs({ 4, 5 }) do
        state.players[index].move_speed = 260
        state.players[index].composure = 1
    end
    state.players[3].facing = Vec2.new(1, 0)
    state.players[3].settle_timer = 0.2
    state.ball = state.players[3].pos:add(Vec2.new(18, 0))
    state.ball_vel = Vec2.new(0, 0)
    return state
end

t.describe("match possession-transition windows", function()
    t.it("commits exactly two counter-pressers and holds everyone else", function()
        local state = home_lost_possession()
        t.eq(match._team_phase(state, "home"), "counterpress")
        t.eq(match._team_phase(state, "away"), "counterattack")

        local held_before = { [2] = state.players[2].pos, [5] = state.players[5].pos }
        local targets, urgent, closing = match._offball_targets(state, positions(state))
        local hunters = closers(state, "home", closing)
        t.eq(#hunters, 2, "counter-press uses at most two eligible outfielders")
        t.eq(hunters[1], 3)
        t.eq(hunters[2], 4)
        for _, index in ipairs(hunters) do
            t.is_true(urgent[index], "a counter-presser closes at full urgency")
            t.near(targets[index].x, state.ball.x, 1e-9)
            t.near(targets[index].y, state.ball.y, 1e-9)
        end

        -- The forward stays on the ground the turnover left it on, shading the
        -- outlet rather than joining the chase or retreating; the defender
        -- recovers toward its anchor instead.
        t.eq(closing[5], nil, "a lane-cutting teammate is not a third hunter")
        t.eq(urgent[5], nil)
        t.is_true(
            targets[5]:dist(held_before[5]) < targets[5]:dist(state.ball),
            "a counter-pressing forward holds nearer its turnover position than the ball"
        )
        local anchor = state.players[2].anchor
        t.is_true(
            targets[2]:dist(anchor) < held_before[2]:dist(anchor),
            "defensive roles recover first when their team loses the ball"
        )
        t.eq(next(state.marks.home), nil, "the window suspends marking assignments")
    end)

    t.it("returns to one presser plus a standing-off cover once the window decays", function()
        local state = home_lost_possession()
        state.transition.elapsed = tactics.balanced.transition.counterpress
        t.eq(match._team_phase(state, "home"), "defend")

        local targets, urgent, closing = match._offball_targets(state, positions(state))
        t.eq(#closers(state, "home", closing), 0, "nobody keeps hunting after the decay")
        t.eq(state.outfield_press.home.presser_index, 3, "#55 keeps its single primary presser")
        t.is_true(urgent[3])
        t.eq(urgent[4], nil, "the ordinary cover is no longer an urgent presser")
        t.is_true(
            targets[4]:dist(state.ball) > 1,
            "the ordinary cover shadows the lane instead of closing the ball"
        )
    end)

    t.it("presses longer under Press High and not at all under Counter Attack", function()
        local press_high = home_lost_possession(tactics.press_high)
        press_high.transition.elapsed = 2.7
        t.eq(match._team_phase(press_high, "home"), "counterpress")
        local _, _, press_closing = match._offball_targets(press_high, positions(press_high))
        t.eq(#closers(press_high, "home", press_closing), 2)

        local balanced = home_lost_possession(tactics.balanced)
        balanced.transition.elapsed = 2.7
        t.eq(match._team_phase(balanced, "home"), "defend", "Balanced has already decayed by 2.7s")

        local counter = home_lost_possession(tactics.counter)
        counter.transition.elapsed = 0
        t.eq(match._team_phase(counter, "home"), "defend", "Counter Attack never counter-presses")
        local _, _, counter_closing = match._offball_targets(counter, positions(counter))
        t.eq(#closers(counter, "home", counter_closing), 0)
    end)

    t.it("counter-attacks longer under Counter Attack than under Balanced", function()
        local counter = home_lost_possession(tactics.balanced, tactics.counter)
        counter.transition.elapsed = 2.7
        t.eq(match._team_phase(counter, "away"), "counterattack")

        local balanced = home_lost_possession(tactics.balanced, tactics.balanced)
        balanced.transition.elapsed = 2.7
        t.eq(match._team_phase(balanced, "away"), "attack")
    end)

    t.it("mirrors phases and hunters for the mirrored team", function()
        local home_lost = home_lost_possession()
        local _, _, home_closing = match._offball_targets(home_lost, positions(home_lost))
        local home_hunters = closers(home_lost, "home", home_closing)

        local away_lost = mirrored_match()
        local layout = {
            [1] = Vec2.new(60, 270),
            [2] = Vec2.new(480, 270),
            [3] = Vec2.new(360, 180),
            [4] = Vec2.new(340, 360),
            [5] = Vec2.new(260, 270),
            [6] = Vec2.new(900, 270),
            [7] = Vec2.new(710, 420),
            [8] = Vec2.new(520, 270),
            [9] = Vec2.new(560, 270),
            [10] = Vec2.new(710, 120),
        }
        for index, player in ipairs(away_lost.players) do
            player.pos = layout[index]
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
            player.composure = 1
            player.dash_cd = 0
            player.settle_timer = 0
            player.outfield_decision = outfield_decision.reset(player.outfield_decision)
        end
        away_lost.owner = 2
        away_lost.ball = Vec2.new(480, 270)
        away_lost.ball_vel = Vec2.new(0, 0)
        arm_turnover(away_lost, "home", 0)

        t.eq(match._team_phase(away_lost, "away"), "counterpress")
        t.eq(match._team_phase(away_lost, "home"), "counterattack")
        local _, _, away_closing = match._offball_targets(away_lost, positions(away_lost))
        local away_hunters = closers(away_lost, "away", away_closing)
        t.eq(#away_hunters, #home_hunters)
        for index, home_index in ipairs(home_hunters) do
            t.eq(away_hunters[index], home_index + 5, "mirrored fixtures pick mirrored hunters")
        end
    end)

    t.it("buys one free in-behind run without the settled-carrier requirement", function()
        local ordinary = unsettled_attack()
        match._offball_targets(ordinary, positions(ordinary))
        t.eq(#runners(ordinary, "home"), 0, "an unsettled carrier grants no ordinary run")

        local counter_attacking = unsettled_attack()
        arm_turnover(counter_attacking, "home", 0)
        t.eq(match._team_phase(counter_attacking, "home"), "counterattack")
        match._offball_targets(counter_attacking, positions(counter_attacking))
        local granted = runners(counter_attacking, "home")
        t.eq(#granted, 1, "exactly one immediate in-behind request")
        t.eq(counter_attacking.players[granted[1]].outfield_decision.intent, "in_behind")
        t.is_true(granted[1] == 4 or granted[1] == 5)
    end)

    t.it("keeps the global two-run cap inside a counter-attack window", function()
        local state = unsettled_attack()
        state.players[3].settle_timer = 0
        arm_turnover(state, "home", 0)
        match._offball_targets(state, positions(state))
        t.eq(#runners(state, "home"), 2, "a settled counter-attack still caps at two runs")
    end)

    t.it("pushes counter-attacking support depth hardest for forward roles", function()
        local state = home_lost_possession()
        local ordinary = home_lost_possession()
        ordinary.transition.elapsed = tactics.balanced.transition.counterattack
        t.eq(match._team_phase(ordinary, "away"), "attack")

        local attack = 1
        for _, index in ipairs({ 8, 9, 10 }) do
            local pushed = match._support_target(
                state,
                index,
                nil,
                nil,
                transitions.support_push(match._formation_role(state, "away", index))
            )
            local plain = match._support_target(ordinary, index)
            local role = match._formation_role(state, "away", index)
            if role == "def" then
                t.near(pushed.x, plain.x, 1e-9, "defensive roles get no counter-attack push")
            else
                -- Away attacks -x, so a deeper push is a smaller x.
                t.is_true(
                    (pushed.x - plain.x) * -attack >= 0,
                    "counter-attacking support pushes toward the opponent goal"
                )
            end
        end
        t.is_true(transitions.support_push("fwd") > transitions.support_push("mid"))
    end)

    t.it("never assigns transition intents to human or fixed-slot players", function()
        local human = home_lost_possession()
        match._set_controlled_player(human, 3)
        human.human_controlled = true
        local _, urgent, closing = match._offball_targets(human, positions(human))
        t.eq(closing[3], nil, "the human player is never a counter-presser")
        t.eq(urgent[3], nil)
        local hunters = closers(human, "home", closing)
        t.eq(#hunters, 2)
        t.is_true(hunters[1] ~= 3 and hunters[2] ~= 3)

        local slots = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
            duration = 60,
            seed = 57,
        })
        slots.kickoff_hold = 0
        slots.owner = 7
        slots.ball = slots.players[7].pos
        arm_turnover(slots, "away", 0)
        t.eq(match._team_phase(slots, "home"), "counterpress")
        local _, slot_urgent, slot_closing = match._offball_targets(slots, positions(slots))
        t.eq(next(slot_closing), nil, "every outfielder owns a fixed input slot")
        t.eq(next(slot_urgent), nil)
        for index, player in ipairs(slots.players) do
            if not player.is_keeper then
                t.is_true(not match._run_eligible(slots, index))
            end
        end
    end)

    t.it("keeps combat-committed players out of the counter-press", function()
        local state = home_lost_possession()
        local combat_state = combat.new_state(state)
        combat_state.players[3].forced_state = "stagger"
        combat_state.players[3].forced_ticks = 4
        local _, _, closing = match._offball_targets(state, positions(state), combat_state)
        local hunters = closers(state, "home", closing)
        t.eq(#hunters, 2)
        t.is_true(hunters[1] ~= 3 and hunters[2] ~= 3, "a forced player cannot counter-press")
    end)

    t.it("hunts a loose ball with two eligible players and no ball magnet", function()
        local state = home_lost_possession()
        state.owner = nil
        state.ball = Vec2.new(470, 270)
        state.ball_vel = Vec2.new(0, 0)
        state.players[5].pos = Vec2.new(478, 262)
        t.eq(match._team_phase(state, "home"), "counterpress")

        local held = state.players[5].pos
        local targets, urgent, closing = match._offball_targets(state, positions(state))
        local hunters = closers(state, "home", closing)
        t.eq(#hunters, 2, "the ball magnet cannot add a third hunter inside the window")
        for _, index in ipairs(hunters) do
            t.is_true(urgent[index])
        end
        if not closing[5] then
            t.is_true(targets[5]:dist(held) < 60, "a non-hunter holds instead of chasing")
        end
    end)

    t.it("keeps kickoff hold and keeper protection ahead of the window", function()
        local held = home_lost_possession()
        held.kickoff_hold = 0.5
        t.eq(match._team_phase(held, "home"), "counterpress", "the phase is still a fact")
        local _, _, closing = match._offball_targets(held, positions(held))
        t.eq(next(closing), nil, "the kickoff hold suspends the counter-press")

        local keeper_ball = home_lost_possession()
        keeper_ball.owner = 6
        keeper_ball.ball = keeper_ball.players[6].pos
        keeper_ball.players[6].feet_ball = false
        local _, _, keeper_closing = match._offball_targets(keeper_ball, positions(keeper_ball))
        t.eq(next(keeper_closing), nil, "a keeper holding the ball cannot be counter-pressed")
    end)
end)

t.describe("match possession-transition lifecycle", function()
    t.it("constructs and kicks off without a turnover", function()
        local state = mirrored_match()
        t.eq(state.transition.last_team, nil)
        t.eq(state.transition.holding_team, nil)
        t.eq(state.transition.hold, 0)
        t.eq(state.transition.turnover_team, nil)
        t.eq(state.transition.elapsed, 0)

        for _ = 1, 90 do
            match.step(state, TICK, NO_INPUT)
        end
        t.eq(state.transition.turnover_team, nil, "a kickoff is not a turnover")
        for _, team in ipairs({ "home", "away" }) do
            local phase = match._team_phase(state, team)
            t.is_true(
                phase == "attack" or phase == "defend" or phase == "loose",
                "an untouched kickoff stays in an ordinary phase, got " .. phase
            )
        end
    end)

    t.it("clears a live window at a scoring restart", function()
        local state = mirrored_match()
        state.max_goals = 3
        arm_turnover(state, "home", 0.5)
        local carrier = state.players[5]
        state.owner = 5
        carrier.pos = Vec2.new(941, 270)
        carrier.facing = Vec2.new(1, 0)
        carrier.run_vel = Vec2.new(carrier.move_speed, 0)
        carrier.vel = carrier.run_vel
        state.ball = Vec2.new(965, 270)
        state.ball_vel = Vec2.new(0, 0)
        state.ball_z = 0
        state.ball_vz = 0

        match.step(state, TICK, NO_INPUT)
        t.eq(state.score.home, 1)
        t.is_true(state.kickoff_hold > 0)
        t.eq(state.transition.turnover_team, nil, "a restart cannot leak a window")
        t.eq(state.transition.last_team, nil)
        t.eq(state.transition.holding_team, nil)
        t.eq(state.transition.hold, 0)
        t.eq(state.transition.elapsed, 0)
    end)

    t.it("clears a live window at full time and afterwards", function()
        local state = mirrored_match()
        arm_turnover(state, "away", 1.0)
        state.time_left = TICK / 2
        match.step(state, TICK, NO_INPUT)
        t.is_true(state.finished)
        t.eq(state.transition.turnover_team, nil)
        t.eq(state.transition.last_team, nil)

        arm_turnover(state, "away", 1.0)
        match.step(state, TICK, NO_INPUT)
        t.eq(state.transition.turnover_team, nil, "a finished match keeps clearing the window")
        t.eq(state.transition.hold, 0)
    end)

    t.it("clears a live window when the goal ends the match", function()
        local state = mirrored_match()
        state.max_goals = 1
        arm_turnover(state, "home", 0.5)
        local carrier = state.players[5]
        state.owner = 5
        carrier.pos = Vec2.new(941, 270)
        carrier.facing = Vec2.new(1, 0)
        carrier.run_vel = Vec2.new(carrier.move_speed, 0)
        carrier.vel = carrier.run_vel
        state.ball = Vec2.new(965, 270)
        state.ball_vel = Vec2.new(0, 0)

        match.step(state, TICK, NO_INPUT)
        t.is_true(state.finished)
        t.eq(state.score.home, 1)
        t.eq(state.transition.turnover_team, nil)
        t.eq(state.transition.last_team, nil)
    end)

    t.it("opens exactly one window per settled turnover across a fixed-seed match", function()
        local state = mirrored_match()
        local opened = 0
        local seen = { counterpress = false, counterattack = false }
        local previous = nil
        for _ = 1, 3600 do
            if state.finished then
                break
            end
            match.step(state, TICK, NO_INPUT)
            local live = state.transition.turnover_team
            if live ~= nil and (previous == nil or state.transition.elapsed == 0) then
                opened = opened + 1
            end
            previous = live
            for _, team in ipairs({ "home", "away" }) do
                local phase = match._team_phase(state, team)
                if seen[phase] ~= nil then
                    seen[phase] = true
                end
            end
            if live ~= nil then
                t.is_true(
                    state.transition.elapsed
                        < transitions.window_limit(state.transition, state.transition_windows),
                    "a live window can never outlive both tactic windows"
                )
                t.eq(state.transition.last_team, live)
            end
        end
        t.is_true(opened > 0, "a real match must turn the ball over")
        t.is_true(seen.counterpress, "a turnover must produce a counter-press phase")
        t.is_true(seen.counterattack, "a turnover must produce a counter-attack phase")
    end)

    t.it("round-trips the transition block through capture and restore", function()
        local state = home_lost_possession(tactics.press_high, tactics.counter)
        state.transition.elapsed = 1.25
        local snapshot = match_snapshot.capture(state)
        local restored = match_snapshot.restore(snapshot)
        t.eq(restored.transition.version, transitions.VERSION)
        t.eq(restored.transition.last_team, "away")
        t.eq(restored.transition.holding_team, "away")
        t.eq(restored.transition.hold, transitions.ESTABLISH_SECONDS)
        t.eq(restored.transition.turnover_team, "away")
        t.eq(restored.transition.elapsed, 1.25)
        t.eq(restored.transition_windows.home.counterpress, 3.0)
        t.eq(restored.transition_windows.home.counterattack, 2.5)
        t.eq(restored.transition_windows.away.counterpress, 0.0)
        t.eq(restored.transition_windows.away.counterattack, 3.0)
        t.eq(match._team_phase(restored, "home"), "counterpress")
        t.eq(match._team_phase(restored, "away"), "counterattack")
        t.eq(
            match_snapshot.hash(match_snapshot.capture(restored)),
            match_snapshot.hash(snapshot),
            "the restored transition hashes identically"
        )

        -- The restored copy is independent of the live state.
        state.transition.elapsed = 0
        t.eq(restored.transition.elapsed, 1.25)
        t.eq(snapshot.state.transition.elapsed, 1.25)
    end)

    t.it("rejects a leaked window and an unknown transition field", function()
        local finished = home_lost_possession()
        finished.finished = true
        t.is_true(not pcall(match_snapshot.capture, finished))

        local outlived = home_lost_possession(tactics.balanced, tactics.balanced)
        outlived.transition.elapsed = 2.5
        t.is_true(not pcall(match_snapshot.capture, outlived))

        local unknown = home_lost_possession()
        rawset(unknown.transition, "future_field", 1)
        t.is_true(not pcall(match_snapshot.capture, unknown))

        local unknown_window = home_lost_possession()
        rawset(unknown_window.transition_windows.home, "future_window", 1)
        t.is_true(not pcall(match_snapshot.capture, unknown_window))
    end)
end)
