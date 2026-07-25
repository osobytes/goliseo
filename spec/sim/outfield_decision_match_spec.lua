local t = require("spec.support.runner")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local outfield_decision = require("sim.outfield_decision")
local teams = require("data.teams")
local tuning = require("sim.tuning")
local Vec2 = require("core.vec2")

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
    overrides = overrides or {}
    local result = {}
    for key, value in pairs(NO_INPUT) do
        result[key] = value
    end
    for key, value in pairs(overrides) do
        result[key] = value
    end
    ---@cast result MatchInput
    return result
end

---@param options table?
---@return MatchState
local function new_match(options)
    options = options or {}
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        human_controlled = options.human_controlled,
        input_ownership = options.input_ownership,
        seed = 53,
    })
end

---@param player MatchPlayer
---@return integer rng_state
local function seed_retained_move(player)
    player.outfield_decision = outfield_decision.refresh(
        player.outfield_decision,
        "offball",
        "move",
        player.scan_rate,
        player.pos.x + 20,
        player.pos.y
    )
    player.outfield_decision.remaining = 10
    return player.outfield_decision.rng_state
end

---@param player MatchPlayer
---@param rng_state integer
local function assert_ai_state_cleared(player, rng_state)
    t.eq(player.outfield_decision.context, "ineligible")
    t.eq(player.outfield_decision.intent, "none")
    t.eq(player.outfield_decision.remaining, 0)
    t.eq(player.outfield_decision.target_x, nil)
    t.eq(player.outfield_decision.target_y, nil)
    t.eq(player.outfield_decision.target_player, nil)
    t.eq(player.outfield_decision.rng_state, rng_state)
end

t.describe("match outfield decision cadence", function()
    t.it("retains off-ball intent until refresh while excluding the human owner", function()
        local state = new_match({ human_controlled = true })
        local human = state.players[state.controlled]
        local ai_index = 8
        local player = state.players[ai_index]

        match.step(state, 1 / 60, NO_INPUT)
        t.eq(human.outfield_decision.context, "ineligible")
        t.eq(human.outfield_decision.generation, 0)
        t.eq(player.outfield_decision.context, "offball")
        local generation = player.outfield_decision.generation
        local target_x = assert(player.outfield_decision.target_x)
        local target_y = assert(player.outfield_decision.target_y)

        player.anchor = Vec2.new(900, 40)
        match.step(state, 1 / 60, NO_INPUT)
        t.eq(player.outfield_decision.generation, generation)
        t.eq(player.outfield_decision.target_x, target_x)
        t.eq(player.outfield_decision.target_y, target_y)

        player.outfield_decision.remaining = 0
        match.step(state, 1 / 60, NO_INPUT)
        t.eq(player.outfield_decision.generation, generation + 1)
        t.is_true(
            player.outfield_decision.target_x ~= target_x
                or player.outfield_decision.target_y ~= target_y,
            "expired cadence did not reconsider the changed role point"
        )
    end)

    t.it("lets an incoming reception bypass a retained off-ball cadence", function()
        local state = new_match({ human_controlled = true })
        local player = state.players[8]
        match.step(state, 1 / 60, NO_INPUT)
        local generation = player.outfield_decision.generation
        player.outfield_decision.remaining = 10
        player.receive_timer = 1
        state.owner = nil
        state.ball = player.pos:add(Vec2.new(-90, 0))
        state.ball_vel = Vec2.new(0, 0)

        local positions = {}
        for index, candidate in ipairs(state.players) do
            positions[index] = candidate.pos
        end
        local _, urgent = match._offball_targets(state, positions)
        ---@type MatchPlayer?
        local uninvolved = nil
        for index, candidate in ipairs(state.players) do
            local contest_reach = tuning.values.LOOSE_MAGNET
                + candidate.move_speed * outfield_decision.SLOW_REFRESH_SECONDS
            if
                index ~= 8
                and not candidate.is_keeper
                and index ~= state.controlled
                and not urgent[index]
                and candidate.pos:dist(state.ball) > contest_reach
            then
                uninvolved = candidate
                break
            end
        end
        uninvolved = assert(uninvolved, "fixture needs an uninvolved outfielder")
        uninvolved.outfield_decision.remaining = 10
        local uninvolved_generation = uninvolved.outfield_decision.generation
        local uninvolved_target_x = uninvolved.outfield_decision.target_x
        local uninvolved_target_y = uninvolved.outfield_decision.target_y

        match.step(state, 1 / 60, NO_INPUT)
        t.eq(player.outfield_decision.generation, generation + 1)
        t.eq(player.outfield_decision.context, "offball")
        t.near(assert(player.outfield_decision.target_x), state.ball.x)
        t.near(assert(player.outfield_decision.target_y), state.ball.y)
        t.eq(
            uninvolved.outfield_decision.generation,
            uninvolved_generation,
            "incoming loose ball should not refresh unrelated support movement"
        )
        t.eq(uninvolved.outfield_decision.target_x, uninvolved_target_x)
        t.eq(uninvolved.outfield_decision.target_y, uninvolved_target_y)
    end)

    t.it("keeps the existing live-pressure pursuit urgent across cadence", function()
        local state = new_match({ human_controlled = true })
        state.kickoff_hold = 0
        local carrier = state.players[assert(state.owner)]
        ---@type integer?
        local presser_index = nil
        ---@type number?
        local presser_distance = nil
        for index, player in ipairs(state.players) do
            if player.team ~= carrier.team and not player.is_keeper then
                local distance = player.pos:dist(carrier.pos)
                if not presser_distance or distance < presser_distance then
                    presser_index = index
                    presser_distance = distance
                end
            end
        end
        local presser = state.players[assert(presser_index)]

        match.step(state, 1 / 60, NO_INPUT)
        local generation = presser.outfield_decision.generation
        presser.outfield_decision.remaining = 10
        match.step(state, 1 / 60, NO_INPUT)

        t.eq(presser.outfield_decision.context, "offball")
        t.eq(presser.outfield_decision.generation, generation + 1)
    end)

    t.it("blocks stunned carrier choices, then refreshes once and retains on recovery", function()
        local state = new_match({ human_controlled = false })
        state.kickoff_hold = 0
        local owner_index = assert(state.owner)
        local owner = state.players[owner_index]
        owner.pos = Vec2.new(480, 270)
        owner.anchor = owner.pos
        state.ball = owner.pos:add(Vec2.new(18, 0))
        for index, player in ipairs(state.players) do
            if index ~= owner_index then
                if player.team == owner.team then
                    player.pos = Vec2.new(20, 20 + index * 18)
                else
                    player.pos = Vec2.new(900, 20 + index * 18)
                end
                player.anchor = player.pos
            end
        end

        owner.outfield_decision = outfield_decision.refresh(
            owner.outfield_decision,
            "carrier",
            "dribble",
            owner.scan_rate
        )
        local generation = owner.outfield_decision.generation
        local rng_state = owner.outfield_decision.rng_state
        owner.outfield_decision.remaining = 0
        owner.stun_timer = 0.1

        match.step(state, 1 / 60, NO_INPUT)
        t.eq(state.owner, owner_index)
        t.eq(owner.outfield_decision.context, "ineligible")
        t.eq(owner.outfield_decision.generation, generation)
        t.eq(owner.outfield_decision.rng_state, rng_state)
        t.eq(owner.windup_shot, nil)

        owner.stun_timer = 0
        match.step(state, 1 / 60, NO_INPUT)
        t.eq(state.owner, owner_index)
        t.eq(owner.outfield_decision.context, "carrier")
        t.eq(owner.outfield_decision.intent, "dribble")
        t.eq(owner.outfield_decision.generation, generation + 1)
        local recovered_rng = owner.outfield_decision.rng_state
        local recovered_remaining = owner.outfield_decision.remaining
        t.is_true(recovered_remaining > 0)

        match.step(state, 1 / 60, NO_INPUT)
        t.eq(owner.outfield_decision.context, "carrier")
        t.eq(owner.outfield_decision.intent, "dribble")
        t.eq(owner.outfield_decision.generation, generation + 1)
        t.eq(owner.outfield_decision.rng_state, recovered_rng)
        t.is_true(owner.outfield_decision.remaining < recovered_remaining)
    end)

    t.it(
        "scores matched opponent geometry from forward route space, not radial distance",
        function()
            ---@param opponent_forward number
            ---@return number
            local function route_space(opponent_forward)
                local state = new_match({ human_controlled = false })
                local owner_index = assert(state.owner)
                local owner = state.players[owner_index]
                owner.pos = Vec2.new(480, 270)
                local route_opponent
                for _, player in ipairs(state.players) do
                    if player.team ~= owner.team and not player.is_keeper then
                        if not route_opponent then
                            route_opponent = player
                        end
                        player.pos = Vec2.new(800, 40)
                    end
                end
                route_opponent = assert(route_opponent)
                route_opponent.pos = owner.pos:add(Vec2.new(opponent_forward, 0))
                return match._carrier_forward_space(state, owner_index)
            end

            local ahead = route_space(50)
            local behind = route_space(-50)
            t.is_true(ahead < behind, "a forward blocker must reduce usable dribble route space")
            t.near(behind, 1)
        end
    )

    t.it("scores multiple route blockers as an order-independent minimum", function()
        ---@class RouteBlocker
        ---@field forward number
        ---@field lateral number

        ---@param blockers RouteBlocker[]
        ---@return number
        local function route_space(blockers)
            local state = new_match({ human_controlled = false })
            local owner_index = assert(state.owner)
            local owner = state.players[owner_index]
            owner.pos = Vec2.new(480, 270)
            local blocker_index = 1
            for _, player in ipairs(state.players) do
                if player.team ~= owner.team and not player.is_keeper then
                    local blocker = blockers[blocker_index]
                    if blocker then
                        player.pos = owner.pos:add(Vec2.new(blocker.forward, blocker.lateral))
                    else
                        player.pos = Vec2.new(800, 40)
                    end
                    blocker_index = blocker_index + 1
                end
            end
            return match._carrier_forward_space(state, owner_index)
        end

        local centered = { forward = 35, lateral = 0 }
        local offset = { forward = 14, lateral = 22 }
        local centered_clearance = route_space({ centered })
        local offset_clearance = route_space({ offset })
        local independent_min = math.min(centered_clearance, offset_clearance)
        local centered_first = route_space({ centered, offset })
        local offset_first = route_space({ offset, centered })

        t.near(centered_clearance, 0.5)
        t.is_true(offset_clearance > centered_clearance)
        t.near(centered_first, independent_min)
        t.near(offset_first, independent_min)
        t.near(centered_first, offset_first)
    end)

    t.it("clears retained AI state on the exact human pass-receiver transfer", function()
        local state = new_match({ human_controlled = true })
        state.kickoff_hold = 0
        local passer_index = state.controlled
        local passer = state.players[passer_index]
        passer.pos = Vec2.new(300, 270)
        passer.facing = Vec2.new(1, 0)
        state.owner = passer_index
        state.ball = passer.pos:add(Vec2.new(18, 0))

        local receiver_index
        for index, player in ipairs(state.players) do
            if player.team == "home" and not player.is_keeper and index ~= passer_index then
                if not receiver_index then
                    receiver_index = index
                    player.pos = Vec2.new(430, 270)
                else
                    player.pos = Vec2.new(40, 30 + index * 30)
                end
            elseif player.team == "away" then
                player.pos = Vec2.new(850, 30 + index * 30)
            end
            player.anchor = player.pos
        end
        local receiver = state.players[assert(receiver_index)]
        local receiver_rng = seed_retained_move(receiver)
        passer.outfield_decision = outfield_decision.refresh(
            passer.outfield_decision,
            "carrier",
            "dribble",
            passer.scan_rate
        )
        local passer_rng = passer.outfield_decision.rng_state

        match.step(state, 1 / 60, input({ pass = true, move = Vec2.new(1, 0) }))
        t.eq(state.owner, nil)
        t.eq(state.controlled, receiver_index)
        assert_ai_state_cleared(passer, passer_rng)
        assert_ai_state_cleared(receiver, receiver_rng)
    end)

    t.it("clears retained AI state when a home outfielder wins the loose ball", function()
        local state = new_match({ human_controlled = true })
        state.owner = nil
        state.pickup_cd = 0
        local winner_index
        for index, player in ipairs(state.players) do
            if player.team == "home" and not player.is_keeper and index ~= state.controlled then
                winner_index = index
                break
            end
        end
        local winner = state.players[assert(winner_index)]
        winner.pos = Vec2.new(300, 270)
        winner.anchor = winner.pos
        state.ball = winner.pos
        state.ball_vel = Vec2.new(0, 0)
        local winner_rng = seed_retained_move(winner)

        match.step(state, 1 / 60, NO_INPUT)
        t.eq(state.owner, winner_index)
        t.eq(state.controlled, winner_index)
        assert_ai_state_cleared(winner, winner_rng)
    end)

    t.it("clears retained AI state on turnover and cross-aid control transfers", function()
        local turnover = new_match({ human_controlled = true })
        local away_index
        local defender_index
        for index, player in ipairs(turnover.players) do
            if player.team == "away" and not player.is_keeper and not away_index then
                away_index = index
            elseif
                player.team == "home"
                and not player.is_keeper
                and index ~= turnover.controlled
                and not defender_index
            then
                defender_index = index
            end
        end
        local defender = turnover.players[assert(defender_index)]
        local away = turnover.players[assert(away_index)]
        away.pos = Vec2.new(600, 270)
        defender.pos = Vec2.new(560, 270)
        turnover.players[turnover.controlled].pos = Vec2.new(100, 100)
        turnover.owner = nil
        turnover.pickup_cd = 0
        turnover.ball = away.pos
        turnover.ball_vel = Vec2.new(0, 0)
        local defender_rng = seed_retained_move(defender)

        match.step(turnover, 1 / 60, NO_INPUT)
        t.eq(turnover.owner, away_index)
        t.eq(turnover.controlled, defender_index)
        assert_ai_state_cleared(defender, defender_rng)

        local cross = new_match({ human_controlled = true })
        cross.owner = nil
        cross.pickup_cd = 60
        cross.ball = Vec2.new(800, 270)
        cross.ball_vel = Vec2.new(0, 0)
        cross.ball_z = 60
        cross.ball_vz = 0
        local attacker_index
        for index, player in ipairs(cross.players) do
            if player.team == "home" and not player.is_keeper and index ~= cross.controlled then
                attacker_index = index
                player.pos = Vec2.new(800, 270)
                break
            end
        end
        cross.players[cross.controlled].pos = Vec2.new(100, 100)
        local attacker = cross.players[assert(attacker_index)]
        local attacker_rng = seed_retained_move(attacker)

        match.step(cross, 1 / 60, NO_INPUT)
        t.eq(cross.controlled, attacker_index)
        assert_ai_state_cleared(attacker, attacker_rng)
    end)

    t.it("clears retained AI state when keeper control returns to an outfielder", function()
        local state = new_match({ human_controlled = true })
        state.controlled = 1
        state.owner = nil
        state.pickup_cd = 60
        state.ball = Vec2.new(400, 270)
        state.ball_vel = Vec2.new(0, 0)
        state.ball_z = 0
        local rng_by_index = {}
        for index, player in ipairs(state.players) do
            if player.team == "home" and not player.is_keeper then
                rng_by_index[index] = seed_retained_move(player)
            end
        end

        match.step(state, 1 / 60, NO_INPUT)
        t.is_true(not state.players[state.controlled].is_keeper)
        assert_ai_state_cleared(
            state.players[state.controlled],
            assert(rng_by_index[state.controlled])
        )
    end)

    t.it("never gives fixed-input-slot outfielders AI cadence or intent", function()
        local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
        local state = new_match({ input_ownership = ownership })
        for tick = 0, 2 do
            match.step(state, 1 / 60, assert(input_frame.neutral(tick)))
        end
        for _, player in ipairs(state.players) do
            if not player.is_keeper then
                t.eq(player.outfield_decision.context, "ineligible")
                t.eq(player.outfield_decision.intent, "none")
                t.eq(player.outfield_decision.generation, 0)
            end
        end
    end)
end)
