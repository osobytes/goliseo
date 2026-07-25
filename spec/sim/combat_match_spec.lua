local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local slot_input = require("sim.slot_input")
local teams = require("data.teams")
local t = require("spec.support.runner")

---@param values table<string, any>?
---@return MatchInput
local function input(values)
    local result = slot_input.neutral_match_input()
    for key, value in pairs(values or {}) do
        result[key] = value
    end
    return result
end

---@param seed integer?
---@return MatchState
local function new_match(seed)
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = seed or 19,
    })
end

---@param events CombatEvent[]
---@param kind CombatEventKind
---@return CombatEvent?
local function event_of(events, kind)
    for _, event in ipairs(events) do
        if event.kind == kind then
            return event
        end
    end
    return nil
end

t.describe("combat match integration", function()
    t.it(
        "keeps the default soccer path byte-identical and rejects partial combat capture",
        function()
            local legacy = new_match()
            local default_path = new_match()
            local neutral = input()

            match.step(legacy, fixed_clock.TICK_SECONDS, neutral)
            match.step(default_path, fixed_clock.TICK_SECONDS, neutral)

            t.eq(
                match_snapshot.hash(match_snapshot.capture(default_path)),
                match_snapshot.hash(match_snapshot.capture(legacy))
            )

            local opted_in = new_match()
            local combat_state = combat.new_state(opted_in)
            match.step(opted_in, fixed_clock.TICK_SECONDS, neutral, combat_state)
            t.is_true(not pcall(match_snapshot.capture, opted_in))
            t.is_true(not pcall(match_snapshot.capture_owned, opted_in))
            t.eq(combat_state.tick, 1)
            t.eq(#combat_state.events, 0)
        end
    )

    t.it("gives existing soccer intent priority over a same-tick equipment press", function()
        local state = new_match()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local controlled = state.controlled
        t.eq(state.owner, controlled)

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({
                shoot_held = true,
                equipment_pressed = true,
                equipment_held = true,
            }),
            combat_state
        )

        t.eq(combat_state.players[controlled].phase, "ready")
        t.is_true(state.players[controlled].charge > 0)
    end)

    t.it("keeps a boundary-tick committed shot ahead of equipment", function()
        local state = new_match()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local controlled = state.controlled
        local player = state.players[controlled]
        player.windup_timer = fixed_clock.TICK_SECONDS
        player.windup_shot = {
            dir = Vec2.new(1, 0),
            speed = 500,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({ equipment_pressed = true, equipment_held = true }),
            combat_state
        )

        t.eq(combat_state.players[controlled].phase, "ready")
        t.eq(player.windup_shot, nil)
        t.eq(state.owner, nil)
        local shot_seen = false
        for _, event in ipairs(state.events) do
            shot_seen = shot_seen or event.kind == "shot"
        end
        t.is_true(shot_seen)
    end)

    t.it(
        "lets switch win before commit and blocks switching during equipment commitment",
        function()
            local state = new_match()
            state.kickoff_hold = 0
            local combat_state = combat.new_state(state)
            local first = state.controlled
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                input({
                    switch = true,
                    equipment_pressed = true,
                    equipment_held = true,
                }),
                combat_state
            )
            local switched = state.controlled
            t.is_true(switched ~= first)
            t.eq(combat_state.players[first].phase, "ready")
            t.eq(combat_state.players[switched].phase, "ready")

            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                input({ equipment_pressed = true, equipment_held = true }),
                combat_state
            )
            t.eq(combat_state.players[switched].phase, "windup")
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                input({ switch = true, equipment_held = true }),
                combat_state
            )
            t.eq(state.controlled, switched)
        end
    )

    t.it("commits equipment only after kickoff and suppresses hidden soccer charge", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        local controlled = state.controlled

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({ equipment_pressed = true, equipment_held = true }),
            combat_state
        )
        t.eq(combat_state.players[controlled].phase, "ready")

        state.kickoff_hold = 0
        state.players[controlled].charge = 0.75
        state.players[controlled].pass_charge = 0.5
        state.players[controlled].pass_target = 2
        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({ equipment_pressed = true, equipment_held = true, sprint = true }),
            combat_state
        )
        t.eq(combat_state.players[controlled].phase, "windup")
        t.eq(state.players[controlled].charge, 0)
        t.eq(state.players[controlled].pass_charge, 0)
        t.eq(state.players[controlled].pass_target, nil)
        t.eq(state.players[controlled].sprinting, false)
    end)

    t.it("gives only an eligible aerial contest priority over equipment", function()
        local state = new_match()
        state.kickoff_hold = 0
        state.controlled = 2
        local combat_state = combat.new_state(state)
        local player = state.players[2]
        state.ball = player.pos
        state.ball_z = 40
        state.ball_vz = -10

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({
                aerial_strike = true,
                equipment_pressed = true,
                equipment_held = true,
            }),
            combat_state
        )
        t.eq(combat_state.players[2].phase, "ready")

        state.ball_z = 0
        state.ball_vz = 0
        player.aerial_timer = 0
        player.aerial_recovery = 0
        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({
                aerial_strike = true,
                equipment_pressed = true,
                equipment_held = true,
            }),
            combat_state
        )
        t.eq(combat_state.players[2].phase, "windup")
    end)

    t.it("ignores every equipment signal for a protected goalkeeper", function()
        local state = new_match()
        state.kickoff_hold = 0
        state.controlled = 1
        local combat_state = combat.new_state(state)

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({
                equipment_pressed = true,
                equipment_released = true,
                equipment_held = false,
            }),
            combat_state
        )

        t.eq(combat_state.players[1].family_id, nil)
        t.eq(combat_state.players[1].phase, "ready")
        t.eq(#combat_state.events, 0)
    end)

    t.it("spills an interrupted carrier before loose-ball collection", function()
        local state = new_match()
        state.kickoff_hold = 0
        state.controlled = 5
        local combat_state = combat.new_state(state)

        for index, player in ipairs(state.players) do
            player.pos = Vec2.new(800 + index, 400 + index)
        end
        local source = state.players[5]
        local target = state.players[10]
        source.pos = Vec2.new(100, 100)
        source.facing = Vec2.new(1, 0)
        target.pos = Vec2.new(130, 100)
        target.facing = Vec2.new(-1, 0)
        state.owner = 10
        state.ball = target.pos
        state.ball_vel = Vec2.new(17, -3)
        combat_state.players[5].phase = "active"
        combat_state.players[5].phase_ticks = 4
        combat_state.players[5].source_sequence = 1

        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)

        t.eq(state.owner, nil)
        t.is_true(state.pickup_cd > 0)
        t.is_true(combat_state.players[10].forced_ticks > 0)
        t.is_true(event_of(combat_state.events, "ball_spill") ~= nil)

        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)
        t.eq(state.owner, nil)
        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)
        t.eq(state.owner, nil)
    end)

    t.it("cancels a struck receiver before aerial resolution and collection", function()
        local state = new_match()
        state.kickoff_hold = 0
        state.controlled = 5
        local combat_state = combat.new_state(state)

        for index, player in ipairs(state.players) do
            player.pos = Vec2.new(800 + index, 400 + index)
        end
        local source = state.players[10]
        local target = state.players[5]
        source.pos = Vec2.new(100, 100)
        source.facing = Vec2.new(1, 0)
        target.pos = Vec2.new(112, 100)
        target.facing = Vec2.new(-1, 0)
        target.receive_timer = 1
        state.owner = nil
        state.ball = target.pos
        state.ball_vel = Vec2.new(0, 0)
        state.ball_z = 30
        state.ball_vz = -30
        combat_state.players[10].family_id = "unarmed"
        combat_state.players[10].phase = "active"
        combat_state.players[10].phase_ticks = 4
        combat_state.players[10].source_sequence = 1

        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)

        t.eq(target.receive_timer, 0)
        t.eq(target.aerial_timer, 0)
        t.eq(target.aerial_outcome, nil)
        t.eq(state.owner, nil)
        t.is_true(combat_state.players[5].forced_ticks > 0)

        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)
        t.eq(target.aerial_timer, 0)
        t.eq(target.aerial_outcome, nil)
        t.eq(state.owner, nil)
    end)

    t.it("rejects a pass assigned to a teammate after that teammate is struck", function()
        local state = new_match()
        state.kickoff_hold = 0
        state.controlled = 5
        local combat_state = combat.new_state(state)

        for index, player in ipairs(state.players) do
            player.pos = Vec2.new(800 + index, 400 + index)
        end
        local owner = state.players[5]
        local target = state.players[2]
        local source = state.players[7]
        owner.pos = Vec2.new(300, 100)
        owner.facing = Vec2.new(1, 0)
        target.pos = Vec2.new(400, 100)
        target.facing = Vec2.new(1, 0)
        source.pos = Vec2.new(370, 100)
        source.facing = Vec2.new(1, 0)
        state.owner = 5
        state.ball = owner.pos:add(owner.facing:scale(18))
        state.ball_vel = Vec2.new(0, 0)
        combat_state.players[7].family_id = "unarmed"
        combat_state.players[7].phase = "active"
        combat_state.players[7].phase_ticks = 4
        combat_state.players[7].source_sequence = 1

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({ pass = true, move = Vec2.new(1, 0) }),
            combat_state
        )

        t.eq(state.owner, nil)
        t.is_true(combat_state.players[2].forced_ticks > 0)
        t.eq(target.receive_timer, 0)
        local pass_seen = false
        for _, event in ipairs(state.events) do
            pass_seen = pass_seen or event.kind == "pass"
        end
        t.is_true(pass_seen)

        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)
        t.eq(target.receive_timer, 0)
        t.eq(target.aerial_timer, 0)
        t.eq(target.aerial_outcome, nil)
        t.eq(state.owner, nil)
    end)

    t.it("preserves scoring-tick events and match-lifetime source sequences", function()
        local state = new_match()
        state.kickoff_hold = 0
        state.controlled = 5
        local combat_state = combat.new_state(state)

        for index, player in ipairs(state.players) do
            player.pos = Vec2.new(800 + index, 400 + index)
        end
        local source = state.players[5]
        local target = state.players[10]
        source.pos = Vec2.new(100, 100)
        source.facing = Vec2.new(1, 0)
        target.pos = Vec2.new(130, 100)
        target.facing = Vec2.new(-1, 0)
        state.owner = nil
        state.ball = Vec2.new(state.field.w - 5, state.field.h / 2)
        state.ball_vel = Vec2.new(1000, 0)
        state.pickup_cd = 1
        combat_state.players[5].phase = "active"
        combat_state.players[5].phase_ticks = 4
        combat_state.players[5].source_sequence = 7
        combat_state.next_source_sequence = 8

        match.step(state, fixed_clock.TICK_SECONDS, input(), combat_state)

        t.eq(state.score.home, 1)
        local contact = event_of(combat_state.events, "contact")
        t.eq(assert(contact).source_sequence, 7)
        t.eq(combat_state.next_source_sequence, 8)
        t.eq(#combat_state.projectiles, 0)
        t.eq(combat_state.players[10].forced_ticks, 0)

        state.kickoff_hold = 0
        state.controlled = 5
        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({ equipment_pressed = true, equipment_held = true }),
            combat_state
        )

        t.eq(combat_state.players[5].source_sequence, 8)
        t.eq(combat_state.next_source_sequence, 9)
        local commit = event_of(combat_state.events, "commit")
        t.eq(assert(commit).source_sequence, 8)
    end)

    t.it("applies family movement commitment without draining held sprint", function()
        local state = new_match()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local controlled = state.controlled
        local player = state.players[controlled]
        player.sprint_meter = 1
        local start = player.pos

        match.step(
            state,
            fixed_clock.TICK_SECONDS,
            input({
                move = Vec2.new(1, 0),
                sprint = true,
                equipment_pressed = true,
                equipment_held = true,
            }),
            combat_state
        )

        t.is_true(player.pos.x > start.x)
        t.eq(player.sprint_meter, 1)
        t.eq(player.sprinting, false)
    end)

    t.it("requires the canonical fixed tick only for the explicit combat path", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        t.is_true(not pcall(match.step, state, 1 / 30, input(), combat_state))

        local legacy = new_match()
        t.is_true(pcall(match.step, legacy, 1 / 30, input()))
    end)

    t.it("keeps the combat boundary synchronized when full time consumes a frame", function()
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = fixed_clock.TICK_SECONDS / 2,
            seed = 20,
            input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
        })
        local combat_state = combat.new_state(state)
        match.step(state, fixed_clock.TICK_SECONDS, assert(input_frame.neutral(0)), combat_state)
        t.is_true(state.finished)
        t.eq(state.input_tick, 1)
        t.eq(combat_state.tick, 1)
        t.eq(match_snapshot.capture(state, combat_state).version, match_snapshot.COMBAT_VERSION)
    end)
end)
