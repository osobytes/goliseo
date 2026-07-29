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
    combat.finish_tick(state, combat_state)
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

---@param events CombatEvent[]
---@param kind CombatEventKind
---@return CombatEvent[]
local function events_of(events, kind)
    local found = {}
    for _, event in ipairs(events) do
        if event.kind == kind then
            found[#found + 1] = event
        end
    end
    return found
end

-- Combat clears its event array every tick, so a lifecycle assertion has to keep
-- its own copy of the confirmed stream.
---@param sink CombatEvent[]
---@param combat_state CombatMatchState
local function drain(sink, combat_state)
    for _, event in ipairs(combat_state.events) do
        sink[#sink + 1] = event
    end
end

-- An opponent's action that is injected rather than simulated still opened a
-- sequence; the reconciliation below needs to know that before it sees its rows.
---@param sink CombatEvent[]
---@param source_sequence integer
local function declare_external_commit(sink, source_sequence)
    sink[#sink + 1] = {
        kind = "commit",
        tick = 0,
        family_id = "light_melee",
        source_index = nil,
        target_index = nil,
        source_sequence = source_sequence,
        result = nil,
        outcome = "accepted",
        reason = nil,
        terminal = nil,
        x = 0,
        y = 0,
        interruption_ticks = nil,
        displacement_px = nil,
    }
end

-- The closed reconciliation: every accepted sequence closes exactly once, and no
-- terminal names a sequence that was never opened.
---@param events CombatEvent[]
---@return table<integer, integer> closed_by_sequence
local function terminals_by_sequence(events)
    local opened = {}
    local closed = {}
    for _, event in ipairs(events) do
        if event.kind == "commit" then
            local sequence = assert(event.source_sequence)
            assert(not opened[sequence], "a source sequence was opened twice")
            opened[sequence] = true
            closed[sequence] = 0
        elseif event.terminal then
            local sequence = assert(event.source_sequence, "a terminal names its sequence")
            assert(opened[sequence], "a terminal closed an unopened sequence")
            closed[sequence] = closed[sequence] + 1
        end
    end
    return closed
end

t.describe("combat request lifecycle", function()
    t.it("types every blocked equipment press with its closed rejection reason", function()
        local press = { equipment_pressed = true, equipment_held = true }

        ---@param prepare fun(state: MatchState, combat_state: CombatMatchState): table<integer, boolean>?
        ---@param player_index integer
        ---@param values table<string, any>
        ---@return CombatEvent
        local function reject(prepare, player_index, values)
            local state = new_match()
            local combat_state = combat.new_state(state)
            local ineligible = prepare(state, combat_state)
            combat.prepare_inputs(
                state,
                combat_state,
                { [player_index] = input(values) },
                ineligible
            )
            local rejected = events_of(combat_state.events, "request_rejected")
            t.eq(#rejected, 1, "exactly one typed row per refused press")
            t.eq(combat_state.next_source_sequence, 1, "a rejection opens no encounter")
            t.eq(combat_state.players[player_index].phase, "ready")
            return rejected[1]
        end

        -- A protected keeper and a player with no equipment are the same physical
        -- refusal: nothing is equipped to request with.
        local keeper = reject(function() end, 1, press)
        t.eq(keeper.reason, "protected_keeper_or_no_loadout")
        t.eq(keeper.outcome, "rejected")
        t.eq(keeper.source_sequence, nil)
        t.eq(keeper.terminal, nil)

        local bare = reject(function(_, combat_state)
            combat_state.players[5].family_id = nil
            combat_state.players[5].loadout_id = nil
        end, 5, press)
        t.eq(bare.reason, "protected_keeper_or_no_loadout")

        local held = reject(function(state)
            state.kickoff_hold = 1
        end, 5, press)
        t.eq(held.reason, "kickoff_hold")

        local same_tick = reject(function() end, 5, { equipment_pressed = true, shoot = true })
        t.eq(same_tick.reason, "soccer_commitment")

        local sliding = reject(function(state)
            state.players[5].slide_timer = 0.4
        end, 5, press)
        t.eq(sliding.reason, "soccer_commitment")

        local airborne = reject(function(state)
            state.players[5].aerial_timer = 0.3
        end, 5, press)
        t.eq(airborne.reason, "aerial_state_or_recovery")

        local recovering = reject(function(state)
            state.players[5].aerial_recovery = 0.2
        end, 5, press)
        t.eq(recovering.reason, "aerial_state_or_recovery")

        local declared = reject(function()
            return { [5] = true }
        end, 5, press)
        t.eq(declared.reason, "aerial_state_or_recovery")

        local forced = reject(function(_, combat_state)
            combat_state.players[5].forced_ticks = 8
            combat_state.players[5].forced_state = "stagger"
        end, 5, press)
        t.eq(forced.reason, "forced_state")

        local cooling = reject(function(_, combat_state)
            combat_state.players[5].cooldown_ticks = 11
        end, 5, press)
        t.eq(cooling.reason, "cooldown")

        -- `already_committed` cannot use the shared helper: the player is mid
        -- action, so its phase is deliberately not `ready`.
        local state = new_match()
        local combat_state = combat.new_state(state)
        combat_state.players[5].phase = "windup"
        combat_state.players[5].phase_ticks = 3
        combat_state.players[5].source_sequence = 1
        combat_state.next_source_sequence = 2
        combat.prepare_inputs(state, combat_state, { [5] = input(press) })
        local committed = events_of(combat_state.events, "request_rejected")
        t.eq(#committed, 1)
        t.eq(committed[1].reason, "already_committed")
        t.eq(combat_state.next_source_sequence, 2, "a rejection opens no encounter")
    end)

    t.it("records nothing without a press edge and types an accepted press", function()
        local state = new_match()
        local combat_state = combat.new_state(state)

        -- No press is no request: a quiet tick owes no row, and neither does a
        -- refused tick that never carried an edge.
        combat_state.players[5].cooldown_ticks = 9
        combat.prepare_inputs(state, combat_state, { [5] = input({ equipment_held = true }) })
        t.eq(#combat_state.events, 0)

        combat_state.players[5].cooldown_ticks = 0
        combat.prepare_inputs(
            state,
            combat_state,
            { [5] = input({ equipment_pressed = true, equipment_held = true }) }
        )
        local commits = events_of(combat_state.events, "commit")
        t.eq(#commits, 1)
        t.eq(commits[1].outcome, "accepted")
        t.eq(commits[1].reason, nil)
        t.eq(commits[1].terminal, nil)
        t.eq(commits[1].source_sequence, 1)
        t.eq(#events_of(combat_state.events, "request_rejected"), 0)
    end)

    t.it("closes an accepted sequence exactly once for every family", function()
        ---@type table<ActionFamilyId, CombatEncounterTerminal>
        local expected = {
            unarmed = "miss",
            light_melee = "miss",
            guard = "cancelled",
            ranged = "expire",
        }
        for _, family_id in ipairs({ "unarmed", "light_melee", "guard", "ranged" }) do
            local state = new_match()
            local combat_state = combat.new_state(state)
            move_other_players_away(state, { 5 })
            state.players[5].pos = Vec2.new(100, 100)
            state.players[5].facing = Vec2.new(1, 0)
            combat_state.players[5].family_id = family_id
            local seen = {}

            -- Press, then release on the next tick so guard and ranged resolve
            -- their held windows instead of hanging in `guard`/`aim` forever.
            for tick_index = 1, 200 do
                tick(
                    state,
                    combat_state,
                    5,
                    input({
                        equipment_pressed = tick_index == 1,
                        equipment_held = tick_index == 1,
                        equipment_released = tick_index == 2,
                    })
                )
                drain(seen, combat_state)
            end

            local closed = terminals_by_sequence(seen)
            t.eq(closed[1], 1, family_id .. " closes its sequence exactly once")
            local terminal
            for _, event in ipairs(seen) do
                terminal = event.terminal or terminal
            end
            t.eq(terminal, expected[family_id], family_id .. " terminal")
        end
    end)

    t.it("closes a landed melee with its contact and never adds a second terminal", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 5, 10 })
        state.players[5].pos = Vec2.new(100, 100)
        state.players[5].facing = Vec2.new(1, 0)
        state.players[10].pos = Vec2.new(120, 100)
        state.players[10].facing = Vec2.new(-1, 0)
        combat_state.players[5].family_id = "unarmed"
        local seen = {}

        for tick_index = 1, 120 do
            tick(
                state,
                combat_state,
                5,
                input({
                    equipment_pressed = tick_index == 1,
                    equipment_held = tick_index == 1,
                })
            )
            drain(seen, combat_state)
        end

        local closed = terminals_by_sequence(seen)
        t.eq(closed[1], 1)
        local contacts = events_of(seen, "contact")
        t.eq(#contacts, 1)
        t.eq(contacts[1].result, "hit")
        t.eq(contacts[1].terminal, "hit")
        t.eq(#events_of(seen, "miss"), 0, "a landed swing never also misses")
        -- The consequence rows share the sequence but are not terminals.
        for _, kind in ipairs({ "forced", "ball_spill" }) do
            for _, event in ipairs(events_of(seen, kind)) do
                t.eq(event.terminal, nil, kind .. " is a consequence, not a terminal")
            end
        end
    end)

    t.it("interrupts a committed action once, and only when it still owes one", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 5, 10 })
        state.players[5].pos = Vec2.new(100, 100)
        state.players[5].facing = Vec2.new(1, 0)
        state.players[10].pos = Vec2.new(400, 400)
        combat_state.players[5].family_id = "light_melee"

        tick(state, combat_state, 5, input({ equipment_pressed = true, equipment_held = true }))
        t.eq(combat_state.players[5].phase, "windup")
        local seen = {}
        drain(seen, combat_state)
        declare_external_commit(seen, 99)

        -- A hit lands on the winding-up player: its own sequence closes as
        -- `interrupted` because it never reached a contact of its own.
        combat.clear_events(combat_state)
        combat.resolve_contacts(state, combat_state, {
            {
                family_id = "light_melee",
                source_index = 10,
                target_index = 5,
                source_sequence = 99,
                projectile_sequence = nil,
                guarded = false,
                immune = false,
                source_pos = state.players[10].pos,
            },
        })
        drain(seen, combat_state)

        local interrupted = events_of(seen, "interrupted")
        t.eq(#interrupted, 1)
        t.eq(interrupted[1].source_sequence, 1)
        t.eq(interrupted[1].terminal, "interrupted")
        t.eq(interrupted[1].source_index, 5)
        t.eq(terminals_by_sequence(seen)[1], 1)
        t.eq(combat_state.players[5].phase, "ready")
    end)

    t.it("hands the terminal to a spawned projectile instead of the cancelled action", function()
        local state = new_match()
        local combat_state = combat.new_state(state)
        move_other_players_away(state, { 4, 10 })
        state.players[4].pos = Vec2.new(100, 100)
        state.players[4].facing = Vec2.new(1, 0)
        state.players[10].pos = Vec2.new(600, 400)
        t.eq(combat_state.players[4].family_id, "ranged")
        local seen = {}

        for tick_index = 1, families.ranged.windup_ticks + 1 do
            tick(
                state,
                combat_state,
                4,
                input({
                    equipment_pressed = tick_index == 1,
                    equipment_released = tick_index == 1,
                })
            )
            drain(seen, combat_state)
        end
        t.eq(#combat_state.projectiles, 1)
        declare_external_commit(seen, 99)

        -- The shooter is knocked down while its shot is still flying. The action
        -- is cancelled, but the encounter belongs to the projectile now.
        combat.clear_events(combat_state)
        combat.resolve_contacts(state, combat_state, {
            {
                family_id = "light_melee",
                source_index = 10,
                target_index = 4,
                source_sequence = 99,
                projectile_sequence = nil,
                guarded = false,
                immune = false,
                source_pos = state.players[10].pos,
            },
        })
        drain(seen, combat_state)
        t.eq(#events_of(seen, "interrupted"), 0, "a flying projectile still owns the terminal")

        for _ = 1, assert(families.ranged.projectile_lifetime_ticks) + 2 do
            tick(state, combat_state, 4, input())
            drain(seen, combat_state)
        end
        t.eq(#combat_state.projectiles, 0)
        t.eq(terminals_by_sequence(seen)[1], 1)
        local expired = events_of(seen, "projectile_expire")
        t.eq(#expired, 1)
        t.eq(expired[1].terminal, "expire")
    end)

    t.it("round-trips the typed request and terminal rows through the snapshot", function()
        local combat_snapshot = require("sim.combat_snapshot")
        local state = new_match()
        local combat_state = combat.new_state(state)
        combat_state.players[5].cooldown_ticks = 4
        combat.prepare_inputs(
            state,
            combat_state,
            { [5] = input({ equipment_pressed = true, equipment_held = true }) }
        )
        combat_state.events[#combat_state.events + 1] = {
            kind = "miss",
            tick = combat_state.tick,
            family_id = "unarmed",
            source_index = 5,
            target_index = nil,
            source_sequence = 3,
            result = nil,
            outcome = nil,
            reason = nil,
            terminal = "miss",
            x = 12,
            y = 34,
            interruption_ticks = nil,
            displacement_px = nil,
        }

        local owned = combat.snapshot(combat_state)
        t.eq(owned.version, combat_snapshot.VERSION)
        t.eq(combat_snapshot.first_difference(owned, combat_state, "combat"), nil)
        t.eq(owned.events[1].reason, "cooldown")
        t.eq(owned.events[1].outcome, "rejected")
        t.eq(owned.events[2].terminal, "miss")

        -- The restore path is fail-closed on both new vocabularies. The rows are
        -- read as plain tables here precisely because the values under test are
        -- the ones the closed types forbid.
        combat_state.tick = 1
        t.is_true(pcall(combat_snapshot.copy, combat_state, "combat", state, false))
        local broken = combat.snapshot(combat_state)
        local rows = broken.events ---@type table<integer, table<string, any>>
        rows[1].reason = "unknown"
        t.is_true(not pcall(combat_snapshot.copy, broken, "combat", state, false))
        rows[1].reason = "cooldown"
        rows[2].terminal = "extended"
        t.is_true(not pcall(combat_snapshot.copy, broken, "combat", state, false))
        rows[2].terminal = "miss"
        rows[1].outcome = "accepted"
        t.is_true(
            not pcall(combat_snapshot.copy, broken, "combat", state, false),
            "a reason without a rejected outcome is malformed"
        )
    end)
end)

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
                combat.finish_tick(state, combat_state)
            end
            contact.family_id = "light_melee"
            contact.source_sequence = 2
            combat.resolve_contacts(state, combat_state, { contact })
            t.eq(runtime.forced_ticks, 18)
            t.eq(state.players[10].pos.x, displaced_x)

            while runtime.forced_ticks > 0 do
                combat.finish_tick(state, combat_state)
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
