local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local brain = require("sim.brain")
local combat = require("sim.combat")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local outfield_decision = require("sim.outfield_decision")
local teams = require("data.teams")

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

---@param overrides table?
---@return MatchInput
local function input(overrides)
    local result = {}
    for key, value in pairs(NO_INPUT) do
        result[key] = value
    end
    for key, value in pairs(overrides or {}) do
        result[key] = value
    end
    ---@cast result MatchInput
    return result
end

---@return MatchState
local function settled_attack()
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
    local positions = {
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
        player.pos = positions[index]
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
        player.outfield_decision = outfield_decision.reset(player.outfield_decision)
    end
    for _, index in ipairs({ 4, 5 }) do
        state.players[index].move_speed = 260
        state.players[index].composure = 1
    end
    state.players[3].settle_timer = 0
    state.players[3].facing = Vec2.new(1, 0)
    state.ball = state.players[3].pos:add(Vec2.new(18, 0))
    state.ball_vel = Vec2.new(0, 0)
    return state
end

---@param state MatchState
---@param team "home"|"away"
---@return integer[]
local function active_runners(state, team)
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

---@param state MatchState
---@param max_goals integer
local function score_home_goal(state, max_goals)
    state.max_goals = max_goals
    local carrier = state.players[assert(state.owner)]
    carrier.pos = Vec2.new(941, 270)
    carrier.facing = Vec2.new(1, 0)
    carrier.run_vel = Vec2.new(carrier.move_speed, 0)
    carrier.vel = carrier.run_vel
    state.players[6].pos = Vec2.new(800, 50)
    state.ball = Vec2.new(965, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.ball_z = 0
    state.ball_vz = 0
    t.eq(#active_runners(state, "home"), 2, "goal setup must retain ordinary attacking runs")
    match.step(state, 1 / 60, input({ move = Vec2.new(1, 0) }))
end

t.describe("match role-gated off-ball runs", function()
    t.it("creates two forward options for a settled midfield carrier within one tick", function()
        local state = settled_attack()
        for _, option in ipairs(match._ai_pass_options(state, 3)) do
            t.is_true(option.forward_progress <= 0, "fixture already had a forward pass option")
        end
        match.step(state, 1 / 60, NO_INPUT)
        local runners = active_runners(state, "home")
        t.eq(#runners, 2)
        t.eq(runners[1], 4)
        t.eq(runners[2], 5)
        for _, index in ipairs(runners) do
            local decision = state.players[index].outfield_decision
            t.eq(decision.intent, "in_behind")
            t.is_true(assert(decision.target_x) > 650)
            t.near(assert(decision.run_expires_at) - -state.time_left, 1.8)
        end

        local elapsed = 1 / 60
        local gained = false
        while elapsed < 1.8 and not gained do
            for _, option in ipairs(match._ai_pass_options(state, 3)) do
                if
                    option.forward_progress > 0
                    and not option.lane_blocked
                    and not option.interception_risk
                then
                    gained = true
                    break
                end
            end
            if not gained then
                match.step(state, 1 / 60, NO_INPUT)
                elapsed = elapsed + 1 / 60
            end
        end
        t.is_true(gained, "run lifetime did not manufacture a clear forward pass option")
        t.is_true(elapsed <= 1.8)
    end)

    t.it("restores formation identity before deriving post-restore runner roles", function()
        local restored = match_snapshot.restore(match_snapshot.capture(settled_attack()))
        t.eq(restored.formation.home, "1-1-2")
        match.step(restored, 1 / 60, NO_INPUT)
        local runners = active_runners(restored, "home")
        t.eq(#runners, 2)
        t.eq(restored.players[4].outfield_decision.intent, "in_behind")
        t.eq(restored.players[5].outfield_decision.intent, "in_behind")
    end)

    t.it(
        "keeps stable run targets and cadence without re-running arbitration at the cap",
        function()
            local state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            local first = state.players[4].outfield_decision
            local expiry = assert(first.run_expires_at)
            local target_x = assert(first.target_x)
            local target_y = assert(first.target_y)
            local generation = first.generation
            first.remaining = 10
            state.players[5].outfield_decision.remaining = 10

            local original_candidates = brain.run_candidates
            local original_grants = brain.grant_runs
            brain.run_candidates = function()
                assert(false, "full stable run cap should skip candidate construction")
            end
            brain.grant_runs = function()
                assert(false, "full stable run cap should skip arbitration")
            end
            local ok, err = pcall(match.step, state, 1 / 60, NO_INPUT)
            brain.run_candidates = original_candidates
            brain.grant_runs = original_grants
            assert(ok, err)

            local retained = state.players[4].outfield_decision
            t.eq(retained.generation, generation)
            t.eq(retained.run_expires_at, expiry)
            t.eq(retained.target_x, target_x)
            t.eq(retained.target_y, target_y)
        end
    )

    t.it("ends at the exact expiry without same-tick regrant and refreshes support once", function()
        local state = settled_attack()
        match.step(state, 1 / 60, NO_INPUT)
        local runner = state.players[4]
        local generation = runner.outfield_decision.generation
        runner.outfield_decision.remaining = 0
        runner.outfield_decision.run_expires_at = -(state.time_left - 1 / 60)
        local expected = match._support_target(state, 4)

        match.step(state, 1 / 60, NO_INPUT)

        t.eq(runner.outfield_decision.intent, "move")
        t.eq(runner.outfield_decision.run_expires_at, nil)
        t.eq(runner.outfield_decision.generation, generation + 1)
        t.near(assert(runner.outfield_decision.target_x), expected.x)
        t.near(assert(runner.outfield_decision.target_y), expected.y)
    end)

    t.it("uses the scored support fallback on completion without changing cadence", function()
        local state = settled_attack()
        match.step(state, 1 / 60, NO_INPUT)
        local runner = state.players[4]
        runner.outfield_decision.remaining = 0.3
        runner.pos = Vec2.new(
            assert(runner.outfield_decision.target_x),
            assert(runner.outfield_decision.target_y)
        )
        local generation = runner.outfield_decision.generation
        local remaining = runner.outfield_decision.remaining
        local expected = match._support_target(state, 4)

        match._sanitize_run_states(state)

        t.eq(runner.outfield_decision.intent, "move")
        t.eq(runner.outfield_decision.run_expires_at, nil)
        t.eq(runner.outfield_decision.generation, generation)
        t.eq(runner.outfield_decision.remaining, remaining)
        t.near(assert(runner.outfield_decision.target_x), expected.x)
        t.near(assert(runner.outfield_decision.target_y), expected.y)
    end)

    t.it("keeps precomputed opponents out of teammate support separation", function()
        local state = settled_attack()
        local positions = {}
        local opponents = {}
        for index, player in ipairs(state.players) do
            positions[index] = player.pos
            if player.team == "away" then
                opponents[#opponents + 1] = player.pos
            end
        end

        local internally_built = match._support_target(state, 4, positions)
        local precomputed = match._support_target(state, 4, positions, opponents)

        t.near(precomputed.x, internally_built.x)
        t.near(precomputed.y, internally_built.y)
    end)

    t.it("waits for jockey and wind-up commitments before granting or retaining runs", function()
        local state = settled_attack()
        local jockeying = state.players[4]
        local winding_up = state.players[5]
        jockeying.jockey_timer = 0.2
        winding_up.windup_timer = 0.2
        winding_up.windup_shot = {
            dir = Vec2.new(1, 0),
            speed = 300,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }

        match.step(state, 1 / 60, NO_INPUT)

        t.eq(#active_runners(state, "home"), 0)
        t.is_true(not match._run_eligible(state, 4))
        t.is_true(not match._run_eligible(state, 5))

        state = settled_attack()
        match.step(state, 1 / 60, NO_INPUT)
        jockeying = state.players[4]
        winding_up = state.players[5]
        jockeying.jockey_timer = 0.2
        winding_up.windup_timer = 0.2
        winding_up.windup_shot = {
            dir = Vec2.new(1, 0),
            speed = 300,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }

        match._sanitize_run_states(state)

        t.eq(#active_runners(state, "home"), 0)
    end)

    t.it(
        "clears incompatible runs on reception, recovery, defense, loose play, and full-time",
        function()
            local state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            local runner = state.players[4]
            local generation = runner.outfield_decision.generation
            local remaining = runner.outfield_decision.remaining
            runner.receive_timer = 1
            match._sanitize_run_states(state)
            t.eq(runner.outfield_decision.intent, "move")
            t.eq(runner.outfield_decision.generation, generation)
            t.eq(runner.outfield_decision.remaining, remaining)
            t.near(assert(runner.outfield_decision.target_x), state.ball.x)
            t.near(assert(runner.outfield_decision.target_y), state.ball.y)

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            runner = state.players[4]
            runner.aerial_timer = 0.2
            match._sanitize_run_states(state)
            t.eq(runner.outfield_decision.intent, "move")

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            runner = state.players[4]
            local combat_state = combat.new_state(state)
            combat_state.players[4].forced_state = "stagger"
            combat_state.players[4].forced_ticks = 2
            match._sanitize_run_states(state, combat_state)
            t.eq(runner.outfield_decision.intent, "move")

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            state.owner = 8
            match._sanitize_run_states(state)
            t.eq(#active_runners(state, "home"), 0)

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            state.owner = nil
            match._sanitize_run_states(state)
            t.eq(#active_runners(state, "home"), 0)

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            score_home_goal(state, 3)
            t.eq(#active_runners(state, "home"), 0)
            t.is_true(state.kickoff_hold > 0)

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            state.time_left = 1 / 120
            match.step(state, 1 / 60, NO_INPUT)
            t.is_true(state.finished)
            t.eq(state.time_left, 0)
            t.eq(#active_runners(state, "home"), 0)

            state = settled_attack()
            match.step(state, 1 / 60, NO_INPUT)
            score_home_goal(state, 1)
            t.is_true(state.finished)
            t.eq(state.score.home, 1)
            t.eq(#active_runners(state, "home"), 0)
        end
    )

    t.it("excludes a human owner transfer and every fixed input slot", function()
        local state = settled_attack()
        match.step(state, 1 / 60, NO_INPUT)
        t.is_true(outfield_decision.is_run_intent(state.players[4].outfield_decision.intent))
        match._set_controlled_player(state, 4)
        t.eq(state.players[4].outfield_decision.context, "ineligible")

        local fixed = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
            seed = 56,
        })
        for index, player in ipairs(fixed.players) do
            if not player.is_keeper then
                t.is_true(not match._run_eligible(fixed, index))
            end
        end
    end)

    t.it("makes the passer eligible at its first post-release personal refresh", function()
        local state = settled_attack()
        local passer = state.players[4]
        state.owner = 4
        match._set_controlled_player(state, 4)
        passer.facing = state.players[3].pos:sub(passer.pos):normalized()
        state.ball = passer.pos:add(passer.facing:scale(18))
        passer.outfield_decision = outfield_decision.refresh(
            passer.outfield_decision,
            "carrier",
            "pass",
            passer.scan_rate,
            nil,
            nil,
            3
        )
        local generation = passer.outfield_decision.generation

        match.step(state, 1 / 60, input({ pass = true, move = passer.facing }))
        t.eq(state.owner, nil)
        t.eq(passer.outfield_decision.context, "ineligible")
        t.eq(passer.outfield_decision.generation, generation)

        state.owner = 3
        state.ball = state.players[3].pos:add(Vec2.new(18, 0))
        state.players[3].settle_timer = 0
        match._set_controlled_player(state, 3)
        match.step(state, 1 / 60, NO_INPUT)

        t.eq(passer.outfield_decision.intent, "in_behind")
        t.eq(passer.outfield_decision.generation, generation + 1)
    end)

    t.it("lets existing pass selection pick a moving runner and lead its velocity", function()
        local state = settled_attack()
        match.step(state, 1 / 60, NO_INPUT)
        local runner = state.players[4]
        t.is_true(runner.vel:length() > 0)
        local owner = state.players[3]
        local aim = runner.pos:sub(owner.pos):normalized()
        t.eq(match._select_pass_target(state, 3, false, aim), 4)

        match.step(state, 1 / 60, input({ pass = true, move = aim }))
        local direct = runner.pos:sub(owner.pos):normalized()
        local released = state.ball_vel:normalized()
        t.is_true(
            released.x > direct.x,
            "the established pass release did not aim ahead of the runner"
        )
        t.eq(runner.outfield_decision.context, "ineligible", "control transfer clears AI run state")
    end)
end)
