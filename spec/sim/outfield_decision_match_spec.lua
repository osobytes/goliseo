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
