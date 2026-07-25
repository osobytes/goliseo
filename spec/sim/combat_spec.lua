local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local match = require("sim.match")
local slot_input = require("sim.slot_input")
local teams = require("data.teams")
local families = require("data.action_families")
local t = require("spec.support.runner")

---@return MatchState
local function new_match()
    local state =
        match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
    state.kickoff_hold = 0
    return state
end

---@param values table<string, any>?
---@return MatchInput
local function input(values)
    local result = slot_input.neutral_match_input()
    for key, value in pairs(values or {}) do
        result[key] = value
    end
    return result
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param player_index integer
---@param player_input MatchInput
---@return CombatContact[]
local function tick(state, combat_state, player_index, player_input)
    combat.prepare_inputs(state, combat_state, { [player_index] = player_input })
    local contacts = combat.collect_contacts(state, combat_state)
    combat.resolve_contacts(state, combat_state, contacts)
    combat.finish_tick(combat_state)
    return contacts
end

---@param state MatchState
---@param keep integer[]
local function move_other_players_away(state, keep)
    local kept = {}
    for _, index in ipairs(keep) do
        kept[index] = true
    end
    for index, player in ipairs(state.players) do
        if not kept[index] then
            player.pos = Vec2.new(800 + index, 400 + index)
        end
    end
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

t.describe("deterministic combat resolver", function()
    t.it("uses exact integer phase windows and pays recovery and cooldown on misses", function()
        for _, family_id in ipairs({ "unarmed", "light_melee" }) do
            local state = new_match()
            local combat_state = combat.new_state(state)
            local runtime = combat_state.players[5]
            runtime.family_id = family_id
            local family = families[family_id]

            for tick_index = 1, family.windup_ticks do
                tick(
                    state,
                    combat_state,
                    5,
                    input({
                        equipment_pressed = tick_index == 1,
                        equipment_held = tick_index == 1,
                    })
                )
                if tick_index < family.windup_ticks then
                    t.eq(runtime.phase, "windup", family_id .. " windup")
                end
            end
            t.eq(runtime.phase, "active", family_id .. " active starts after windup")
            t.eq(combat_state.tick, family.windup_ticks)

            for _ = 1, assert(family.active_ticks) do
                tick(state, combat_state, 5, input())
            end
            t.eq(runtime.phase, "recovery", family_id .. " miss enters recovery")

            for _ = 1, family.recovery_ticks do
                tick(state, combat_state, 5, input())
            end
            t.eq(runtime.phase, "ready", family_id .. " recovery completes")
            local commitment_ticks = family.windup_ticks
                + assert(family.active_ticks)
                + family.recovery_ticks
            t.eq(
                runtime.cooldown_ticks,
                math.max(0, family.cooldown_ticks - commitment_ticks),
                family_id .. " cooldown starts at commit"
            )
            while runtime.cooldown_ticks > 0 do
                tick(state, combat_state, 5, input())
            end
            t.eq(runtime.cooldown_ticks, 0)
        end
    end)

    t.it("raises and lowers guard without inventing a held interval for a fast tap", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        local runtime = combat_state.players[2]
        t.eq(runtime.family_id, "guard")

        for tick_index = 1, families.guard.windup_ticks do
            tick(
                state,
                combat_state,
                2,
                input({
                    equipment_pressed = tick_index == 1,
                    equipment_released = tick_index == 1,
                    equipment_held = false,
                })
            )
        end
        t.eq(runtime.phase, "recovery")

        combat.reset(combat_state)
        runtime = combat_state.players[2]
        for tick_index = 1, families.guard.windup_ticks do
            tick(
                state,
                combat_state,
                2,
                input({
                    equipment_pressed = tick_index == 1,
                    equipment_held = true,
                })
            )
        end
        t.eq(runtime.phase, "guard")
        tick(state, combat_state, 2, input({ equipment_released = true, equipment_held = false }))
        t.eq(runtime.phase, "recovery")
    end)

    t.it("latches an early ranged release and never auto-fires a held aim", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        local runtime = combat_state.players[4]
        t.eq(runtime.family_id, "ranged")

        for tick_index = 1, families.ranged.windup_ticks do
            tick(
                state,
                combat_state,
                4,
                input({
                    equipment_pressed = tick_index == 1,
                    equipment_released = tick_index == 1,
                    equipment_held = false,
                })
            )
        end
        t.eq(runtime.phase, "active")
        tick(state, combat_state, 4, input())
        t.eq(runtime.phase, "recovery")
        t.eq(#combat_state.projectiles, 1)

        combat.reset(combat_state)
        runtime = combat_state.players[4]
        for tick_index = 1, families.ranged.windup_ticks do
            tick(
                state,
                combat_state,
                4,
                input({
                    equipment_pressed = tick_index == 1,
                    equipment_held = true,
                })
            )
        end
        t.eq(runtime.phase, "aim")
        for _ = 1, 20 do
            tick(state, combat_state, 4, input({ equipment_held = true }))
        end
        t.eq(runtime.phase, "aim")
        t.eq(#combat_state.projectiles, 0)
        tick(state, combat_state, 4, input({ equipment_released = true, equipment_held = false }))
        t.eq(runtime.phase, "recovery")
        t.eq(#combat_state.projectiles, 1)
    end)

    t.it("selects one legal melee target by forward projection and stable index", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 3, 4, 6, 7, 8 })
        local source = state.players[3]
        source.pos = Vec2.new(100, 100)
        source.facing = Vec2.new(1, 0)
        state.players[4].pos = Vec2.new(105, 100) -- friendly: ignored
        state.players[6].pos = Vec2.new(110, 100) -- keeper: ignored
        state.players[7].pos = Vec2.new(130, 106)
        state.players[8].pos = Vec2.new(130, 94)

        local runtime = combat_state.players[3]
        runtime.phase = "active"
        runtime.phase_ticks = 5
        runtime.source_sequence = 1
        local contacts = combat.collect_contacts(state, combat_state)
        t.eq(#contacts, 1)
        t.eq(contacts[1].target_index, 7)
    end)

    t.it("lets a projectile pass a dodge and uses swept contact on a later tick", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 4, 7 })
        state.players[4].pos = Vec2.new(100, 100)
        state.players[4].facing = Vec2.new(1, 0)
        state.players[7].pos = Vec2.new(104, 100)
        state.players[7].dodge_timer = 1
        local runtime = combat_state.players[4]
        runtime.phase = "active"
        runtime.phase_ticks = 1
        runtime.source_sequence = 11

        local contacts = combat.collect_contacts(state, combat_state)
        t.eq(#contacts, 0)
        t.eq(#combat_state.projectiles, 1)
        t.near(combat_state.projectiles[1].pos.x, 105)

        state.players[7].dodge_timer = 0
        state.players[7].pos = Vec2.new(109, 100)
        contacts = combat.collect_contacts(state, combat_state)
        t.eq(#contacts, 1)
        t.eq(contacts[1].target_index, 7)
        t.eq(#combat_state.projectiles, 0)
    end)

    t.it("expires a missed projectile after exactly sixty deterministic travel ticks", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 4 })
        state.players[4].pos = Vec2.new(100, 100)
        state.players[4].facing = Vec2.new(1, 0)
        local runtime = combat_state.players[4]
        runtime.phase = "active"
        runtime.phase_ticks = 1
        runtime.source_sequence = 7

        combat.collect_contacts(state, combat_state)
        t.eq(#combat_state.projectiles, 1)
        t.near(combat_state.projectiles[1].pos.x, 105)
        for _ = 2, assert(families.ranged.projectile_lifetime_ticks) do
            combat.collect_contacts(state, combat_state)
        end
        t.eq(#combat_state.projectiles, 0)
        t.is_true(event_of(combat_state.events, "projectile_expire") ~= nil)
    end)

    t.it("blocks a frontal hit with one bounded recoil and leaves the ball owned", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 3, 7 })
        state.players[3].pos = Vec2.new(100, 100)
        state.players[3].facing = Vec2.new(1, 0)
        state.players[7].pos = Vec2.new(130, 100)
        state.players[7].facing = Vec2.new(-1, 0)
        state.owner = 7
        combat_state.players[3].phase = "active"
        combat_state.players[3].phase_ticks = 5
        combat_state.players[3].source_sequence = 1
        combat_state.players[7].phase = "guard"

        local contacts = combat.collect_contacts(state, combat_state)
        t.is_true(contacts[1].guarded)
        combat.resolve_contacts(state, combat_state, contacts)
        t.eq(combat_state.players[7].forced_ticks, 0)
        t.eq(state.owner, 7)
        t.near(state.players[7].pos.x, 136)
        t.is_true(event_of(combat_state.events, "guard_recoil") ~= nil)
    end)

    t.it("resolves simultaneous trades and one dominant outcome per target", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 3, 5, 10 })
        state.players[3].pos = Vec2.new(100, 100)
        state.players[3].facing = Vec2.new(1, 0)
        state.players[5].pos = Vec2.new(100, 100)
        state.players[5].facing = Vec2.new(1, 0)
        state.players[10].pos = Vec2.new(130, 100)
        state.players[10].facing = Vec2.new(-1, 0)
        combat_state.players[3].phase = "active"
        combat_state.players[3].phase_ticks = 5
        combat_state.players[3].source_sequence = 1
        combat_state.players[5].phase = "active"
        combat_state.players[5].phase_ticks = 4
        combat_state.players[5].source_sequence = 2
        combat_state.players[10].phase = "active"
        combat_state.players[10].phase_ticks = 4
        combat_state.players[10].source_sequence = 3

        local contacts = combat.collect_contacts(state, combat_state)
        t.eq(#contacts, 3)
        combat.resolve_contacts(state, combat_state, contacts)
        t.eq(combat_state.players[10].forced_ticks, 18)
        t.eq(combat_state.players[3].forced_ticks, 10)
        t.eq(combat_state.players[5].forced_ticks, 0)
        local superseded = false
        for _, event in ipairs(combat_state.events) do
            if event.kind == "contact" and event.source_index == 5 then
                superseded = event.result == "superseded"
            end
        end
        t.is_true(superseded)
    end)

    t.it(
        "caps a hit chain and grants a full immunity window without repeat displacement",
        function()
            local state = new_match()
            local combat_state = combat.new_state(state)
            state.players[3].pos = Vec2.new(100, 100)
            state.players[10].pos = Vec2.new(130, 100)
            local contact = {
                family_id = "unarmed",
                source_index = 3,
                target_index = 10,
                source_sequence = 1,
                projectile_sequence = nil,
                guarded = false,
                immune = false,
                source_pos = state.players[3].pos,
            }

            combat.resolve_contacts(state, combat_state, { contact })
            local runtime = combat_state.players[10]
            local displaced_x = state.players[10].pos.x
            t.eq(runtime.forced_ticks, 10)
            for _ = 1, 5 do
                combat.finish_tick(combat_state)
            end
            contact.family_id = "light_melee"
            contact.source_sequence = 2
            combat.resolve_contacts(state, combat_state, { contact })
            t.eq(runtime.forced_ticks, 18)
            t.eq(state.players[10].pos.x, displaced_x)

            while runtime.forced_ticks > 0 do
                combat.finish_tick(combat_state)
            end
            t.eq(runtime.immunity_ticks, combat.IMMUNITY_TICKS)
            contact.immune = true
            combat.resolve_contacts(state, combat_state, { contact })
            t.eq(runtime.forced_ticks, 0)
            t.eq(runtime.immunity_ticks, combat.IMMUNITY_TICKS)
        end
    )

    t.it("does not invent a spill when an unguarded contact finds a loose ball", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        state.owner = nil
        state.players[3].pos = Vec2.new(100, 100)
        state.players[10].pos = Vec2.new(130, 100)
        combat.resolve_contacts(state, combat_state, {
            {
                family_id = "light_melee",
                source_index = 3,
                target_index = 10,
                source_sequence = 1,
                projectile_sequence = nil,
                guarded = false,
                immune = false,
                source_pos = state.players[3].pos,
            },
        })
        t.eq(state.owner, nil)
        t.eq(event_of(combat_state.events, "ball_spill"), nil)
        t.eq(combat_state.players[10].forced_state, "knockback")
    end)

    t.it("keeps combat presentation-free, neutral, fixture-bound, and snapshot-deferred", function()
        local state = new_match()
        local players_by_id = {}
        for _, player in ipairs(require("data.players")) do
            local copy = {}
            for key, value in pairs(player) do
                copy[key] = value
            end
            players_by_id[copy.id] = copy
        end
        players_by_id.veil_nyx.loadout_id = "loadout_vector_blade"
        local combat_state = combat.new_state(state, players_by_id)
        t.eq(combat_state.players[3].family_id, "light_melee")

        players_by_id.zyro_vex.loadout_id = nil
        local neutral_state = combat.new_state(state, players_by_id)
        t.eq(neutral_state.players[5].family_id, nil)
        combat.prepare_inputs(state, neutral_state, {
            [5] = input({ equipment_pressed = true, equipment_held = true }),
        })
        t.eq(neutral_state.players[5].phase, "ready")

        neutral_state.player_ids[5] = "wrong_player"
        t.is_true(not pcall(combat.prepare_inputs, state, neutral_state, {}))

        local snapshot = combat.snapshot(combat_state)
        t.eq(snapshot.version, combat_state.version)
        t.eq(snapshot.players[3].loadout_id, "loadout_vector_blade")
        t.eq(snapshot.players[3].family_id, "light_melee")
        snapshot.players[3].phase = "active"
        t.eq(combat_state.players[3].phase, "ready")
    end)
end)
