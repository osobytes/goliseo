local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_snapshot = require("sim.combat_snapshot")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local outfield_decision = require("sim.outfield_decision")
local outfield_press = require("sim.outfield_press")
local possession_transition = require("sim.possession_transition")
local slot_input = require("sim.slot_input")
local action_families = require("data.action_families")
local teams = require("data.teams")
local validation_config = require("data.omp2_rollback_validation")

---@return MatchState
local function new_state()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 2,
        max_goals = 3,
        seed = 38,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
end

---@return MatchState
local function new_ai_state()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 2,
        max_goals = 3,
        seed = 38,
        human_controlled = false,
    })
end

---@return MatchState
local function new_attacking_ai_state()
    local state = new_ai_state()
    state.kickoff_hold = 0
    return state
end

---@param class_name string
---@param end_marker string
---@param source_path string?
---@return string[]
local function declared_fields(class_name, end_marker, source_path)
    local source = assert(love.filesystem.read(source_path or "sim/match.lua"))
    local start_at = assert(source:find("---@class " .. class_name, 1, true))
    local end_at = assert(source:find(end_marker, start_at, true))
    local fields = {}
    for field in source:sub(start_at, end_at - 1):gmatch("%-%-%-@field%s+([%w_]+)") do
        fields[#fields + 1] = field
    end
    return fields
end

---@param actual string[]
---@param expected string[]
local function same_fields(actual, expected)
    t.eq(#actual, #expected, "snapshot schema field count")
    for index = 1, #expected do
        t.eq(actual[index], expected[index], "snapshot schema field " .. index)
    end
end

t.describe("canonical match snapshots", function()
    t.it("pins MatchState and MatchPlayer additions to explicit versioned allowlists", function()
        same_fields(
            declared_fields("MatchPlayer", "---@class MatchInput"),
            match_snapshot.PLAYER_FIELDS
        )
        same_fields(declared_fields("MatchState", "local match = {}"), match_snapshot.MATCH_FIELDS)
        same_fields(
            declared_fields("CombatMatchState", "---@class CombatContact", "sim/combat.lua"),
            combat_snapshot.STATE_FIELDS
        )
        same_fields(
            declared_fields("CombatPlayerState", "---@class CombatProjectile", "sim/combat.lua"),
            combat_snapshot.PLAYER_FIELDS
        )
        same_fields(
            declared_fields("CombatProjectile", "---@class CombatEvent", "sim/combat.lua"),
            combat_snapshot.PROJECTILE_FIELDS
        )
        same_fields(
            declared_fields("CombatEvent", "---@class CombatMatchState", "sim/combat.lua"),
            combat_snapshot.EVENT_FIELDS
        )
    end)

    t.it("captures combat as one owned, canonical, versioned boundary", function()
        local state = new_state()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local source_index = nil
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "ranged" then
                source_index = index
                break
            end
        end
        source_index = assert(source_index, "fixture requires one ranged loadout")
        local runtime = combat_state.players[source_index]
        runtime.phase = "windup"
        runtime.phase_ticks = 7
        runtime.cooldown_ticks = 42
        runtime.source_sequence = 7
        runtime.control_held = true
        combat_state.projectiles[1] = {
            family_id = "ranged",
            source_index = source_index,
            source_sequence = 7,
            pos = Vec2.new(111, 222),
            dir = Vec2.new(0.5, -0.5),
            remaining_ticks = 12,
        }
        combat_state.events[1] = {
            kind = "projectile_spawn",
            tick = 0,
            family_id = "ranged",
            source_index = source_index,
            target_index = nil,
            source_sequence = 7,
            result = nil,
            x = 111,
            y = 222,
            interruption_ticks = nil,
            displacement_px = nil,
        }
        combat_state.next_source_sequence = 8
        state.input_tick = 1
        combat_state.tick = 1

        local snapshot = match_snapshot.capture(state, combat_state)
        t.eq(snapshot.version, match_snapshot.COMBAT_VERSION)
        t.eq(assert(snapshot.combat).version, combat_snapshot.VERSION)
        t.eq(snapshot.combat.players[source_index].loadout_id, runtime.loadout_id)
        local restored, restored_combat = match_snapshot.restore(snapshot)
        restored_combat = assert(restored_combat)
        t.eq(restored_combat.players[source_index].phase_ticks, 7)
        t.eq(restored_combat.projectiles[1].pos.x, 111)
        t.eq(restored_combat.events[1].source_sequence, 7)
        t.eq(
            match_snapshot.hash(match_snapshot.capture(restored, restored_combat)),
            match_snapshot.hash(snapshot)
        )

        state.players[source_index].pos.x = -1
        combat_state.players[source_index].phase_ticks = 1
        combat_state.projectiles[1].pos.x = -2
        combat_state.events[1].x = -3
        t.eq(snapshot.state.players[source_index].pos.x ~= -1, true)
        t.eq(snapshot.combat.players[source_index].phase_ticks, 7)
        t.eq(snapshot.combat.projectiles[1].pos.x, 111)
        t.eq(snapshot.combat.events[1].x, 111)

        snapshot.combat.players[source_index].phase_ticks = 6
        local changed = match_snapshot.capture(restored, restored_combat)
        local found = assert(match_snapshot.first_difference(snapshot, changed))
        t.eq(found.path, "combat.players." .. source_index .. ".phase_ticks")

        local malformed = match_snapshot.capture(restored, restored_combat)
        ---@type table<string, any>
        local malformed_player = assert(malformed.combat).players[source_index]
        malformed_player.unknown = true
        t.is_true(not pcall(match_snapshot.restore, malformed))
        t.is_true(not pcall(match_snapshot.capture, restored))
    end)

    t.it("rejects holes in authoritative combat projectile and event arrays", function()
        local state = new_state()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local source_index = nil
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "ranged" then
                source_index = index
                break
            end
        end
        source_index = assert(source_index, "fixture requires one ranged loadout")
        combat_state.projectiles[1] = {
            family_id = "ranged",
            source_index = source_index,
            source_sequence = 1,
            pos = Vec2.new(111, 222),
            dir = Vec2.new(1, 0),
            remaining_ticks = 12,
        }
        combat_state.events[1] = {
            kind = "projectile_spawn",
            tick = 0,
            family_id = "ranged",
            source_index = source_index,
            target_index = nil,
            source_sequence = 1,
            result = nil,
            x = 111,
            y = 222,
            interruption_ticks = nil,
            displacement_px = nil,
        }
        combat_state.next_source_sequence = 2
        state.input_tick = 1
        combat_state.tick = 1

        for _, field in ipairs({ "projectiles", "events" }) do
            local malformed = match_snapshot.capture(state, combat_state)
            local values = assert(malformed.combat)[field]
            values[2] = values[1]
            values[3] = values[1]
            values[4] = values[1]
            values[2] = nil
            t.eq(#values, 4, "hole fixture must retain its authoritative tail")
            t.is_true(not pcall(match_snapshot.restore, malformed), field .. " hole was accepted")
        end
    end)

    t.it("replays combat phase boundaries exactly after restore", function()
        ---@type { name: string, configure: fun(combat_state: CombatMatchState, family: table<string, integer>, state: MatchState) }[]
        local cases = {
            {
                name = "windup",
                configure = function(combat_state, family, _)
                    local runtime = combat_state.players[family.light_melee]
                    runtime.phase = "windup"
                    runtime.phase_ticks = 3
                    runtime.cooldown_ticks = 20
                    runtime.source_sequence = 1
                end,
            },
            {
                name = "active",
                configure = function(combat_state, family, _)
                    local runtime = combat_state.players[family.light_melee]
                    runtime.phase = "active"
                    runtime.phase_ticks = 2
                    runtime.cooldown_ticks = 20
                    runtime.source_sequence = 1
                end,
            },
            {
                name = "guard",
                configure = function(combat_state, family, _)
                    local runtime = combat_state.players[family.guard]
                    runtime.phase = "guard"
                    runtime.control_held = true
                    runtime.source_sequence = 1
                end,
            },
            {
                name = "projectile",
                configure = function(combat_state, family, state)
                    combat_state.projectiles[1] = {
                        family_id = "ranged",
                        source_index = family.ranged,
                        source_sequence = 1,
                        pos = state.players[family.ranged].pos,
                        dir = Vec2.new(1, 0),
                        remaining_ticks = 2,
                    }
                    combat_state.next_source_sequence = 2
                end,
            },
            {
                name = "stagger",
                configure = function(combat_state, family, _)
                    local runtime = combat_state.players[family.unarmed]
                    runtime.forced_state = "stagger"
                    runtime.forced_ticks = 2
                    runtime.chain_ticks = 4
                end,
            },
            {
                name = "knockback",
                configure = function(combat_state, family, _)
                    local runtime = combat_state.players[family.light_melee]
                    runtime.forced_state = "knockback"
                    runtime.forced_ticks = 2
                    runtime.chain_ticks = 4
                end,
            },
            {
                name = "immunity",
                configure = function(combat_state, family, _)
                    combat_state.players[family.ranged].immunity_ticks = 2
                end,
            },
        }
        for _, case in ipairs(cases) do
            local state = new_state()
            state.kickoff_hold = 0
            local combat_state = combat.new_state(state)
            local family = {}
            for index, runtime in ipairs(combat_state.players) do
                if runtime.family_id and family[runtime.family_id] == nil then
                    family[runtime.family_id] = index
                end
            end
            case.configure(combat_state, family, state)
            local start = match_snapshot.capture(state, combat_state)
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                assert(input_frame.neutral(0)),
                combat_state
            )
            local expected = match_snapshot.capture(state, combat_state)
            local restored, restored_combat = match_snapshot.restore(start)
            match.step(
                restored,
                fixed_clock.TICK_SECONDS,
                assert(input_frame.neutral(0)),
                assert(restored_combat)
            )
            t.eq(
                match_snapshot.hash(match_snapshot.capture(restored, restored_combat)),
                match_snapshot.hash(expected),
                case.name
            )
        end
    end)

    t.it("persists active AI runs through a v10 soccer boundary and continuation", function()
        local state = new_attacking_ai_state()
        local owner = assert(state.owner)
        local owner_team = state.players[owner].team
        local runner_indices = {}
        for index, player in ipairs(state.players) do
            if
                player.team == owner_team
                and not player.is_keeper
                and index ~= owner
                and #runner_indices < 2
            then
                runner_indices[#runner_indices + 1] = index
            end
        end
        t.eq(#runner_indices, 2)
        for ordinal, runner_index in ipairs(runner_indices) do
            local player = state.players[runner_index]
            player.outfield_decision = outfield_decision.refresh(
                player.outfield_decision,
                "offball",
                ordinal == 1 and "come_short" or "in_behind",
                player.scan_rate,
                ordinal == 1 and 420 or 700,
                ordinal == 1 and 180 or 300,
                nil,
                -0.2
            )
        end

        local boundary = match_snapshot.capture(state)
        t.eq(boundary.version, match_snapshot.VERSION)
        local restored, restored_combat = match_snapshot.restore(boundary)
        t.eq(restored_combat, nil)
        for _, runner_index in ipairs(runner_indices) do
            local expected = state.players[runner_index].outfield_decision
            local actual = restored.players[runner_index].outfield_decision
            t.eq(actual.intent, expected.intent)
            t.eq(actual.run_expires_at, expected.run_expires_at)
            t.eq(actual.remaining, expected.remaining)
            t.eq(actual.generation, expected.generation)
        end
        t.eq(match_snapshot.hash(match_snapshot.capture(restored)), match_snapshot.hash(boundary))

        for _ = 1, 2 do
            local live_input = slot_input.neutral_match_input()
            local restored_input = slot_input.neutral_match_input()
            match.step(state, fixed_clock.TICK_SECONDS, live_input)
            match.step(restored, fixed_clock.TICK_SECONDS, restored_input)
            t.eq(
                match_snapshot.hash(match_snapshot.capture(restored)),
                match_snapshot.hash(match_snapshot.capture(state))
            )
        end
    end)

    t.it("persists active AI runs through a v11 combat boundary and continuation", function()
        local state = new_attacking_ai_state()
        local owner = assert(state.owner)
        local owner_team = state.players[owner].team
        local runner_indices = {}
        for index, player in ipairs(state.players) do
            if
                player.team == owner_team
                and not player.is_keeper
                and index ~= owner
                and #runner_indices < 2
            then
                runner_indices[#runner_indices + 1] = index
            end
        end
        t.eq(#runner_indices, 2)
        for ordinal, runner_index in ipairs(runner_indices) do
            local player = state.players[runner_index]
            player.outfield_decision = outfield_decision.refresh(
                player.outfield_decision,
                "offball",
                ordinal == 1 and "come_short" or "in_behind",
                player.scan_rate,
                ordinal == 1 and 420 or 700,
                ordinal == 1 and 180 or 300,
                nil,
                -0.2
            )
        end

        local combat_state = combat.new_state(state)
        local boundary = match_snapshot.capture(state, combat_state)
        t.eq(boundary.version, match_snapshot.COMBAT_VERSION)
        local restored, restored_combat = match_snapshot.restore(boundary)
        restored_combat = assert(restored_combat)
        for _, runner_index in ipairs(runner_indices) do
            local expected = state.players[runner_index].outfield_decision
            local actual = restored.players[runner_index].outfield_decision
            t.eq(actual.intent, expected.intent)
            t.eq(actual.run_expires_at, expected.run_expires_at)
            t.eq(actual.remaining, expected.remaining)
            t.eq(actual.generation, expected.generation)
        end
        t.eq(
            match_snapshot.hash(match_snapshot.capture(restored, restored_combat)),
            match_snapshot.hash(boundary)
        )

        for _ = 1, 2 do
            local live_input = slot_input.neutral_match_input()
            local restored_input = slot_input.neutral_match_input()
            match.step(state, fixed_clock.TICK_SECONDS, live_input, combat_state)
            match.step(restored, fixed_clock.TICK_SECONDS, restored_input, restored_combat)
            t.eq(
                match_snapshot.hash(match_snapshot.capture(restored, restored_combat)),
                match_snapshot.hash(match_snapshot.capture(state, combat_state))
            )
        end
    end)

    t.it("rejects a combat-blocked active run as a malformed v11 boundary", function()
        local state = new_attacking_ai_state()
        local owner = assert(state.owner)
        local runner_index = nil
        for index, player in ipairs(state.players) do
            if
                player.team == state.players[owner].team
                and not player.is_keeper
                and index ~= owner
            then
                runner_index = index
                break
            end
        end
        runner_index = assert(runner_index)
        local runner = state.players[runner_index]
        runner.outfield_decision = outfield_decision.refresh(
            runner.outfield_decision,
            "offball",
            "in_behind",
            runner.scan_rate,
            700,
            240,
            nil,
            -0.2
        )
        local combat_state = combat.new_state(state)
        local runtime = combat_state.players[runner_index]

        local valid_boundary = match_snapshot.capture(state, combat_state)
        runtime.forced_state = "stagger"
        runtime.forced_ticks = 2
        runtime.chain_ticks = 4
        t.is_true(
            not pcall(match_snapshot.capture, state, combat_state),
            "capture accepted a forced player retaining a run"
        )

        valid_boundary.combat.players[runner_index].forced_state = "stagger"
        valid_boundary.combat.players[runner_index].forced_ticks = 2
        valid_boundary.combat.players[runner_index].chain_ticks = 4
        t.is_true(
            not pcall(match_snapshot.restore, valid_boundary),
            "restore accepted a forced player retaining a run"
        )
        local malformed_runtime = valid_boundary.combat.players[runner_index]
        malformed_runtime.forced_state = nil
        malformed_runtime.forced_ticks = 0
        malformed_runtime.chain_ticks = 0
        malformed_runtime.phase = "recovery"
        malformed_runtime.phase_ticks = 1
        t.is_true(
            not pcall(match_snapshot.restore, valid_boundary),
            "restore accepted an action-committed player retaining a run"
        )

        match._sanitize_run_states(state, combat_state)
        local cleared = state.players[runner_index].outfield_decision
        t.is_true(not outfield_decision.is_run_intent(cleared.intent))
        t.eq(cleared.run_expires_at, nil)
        local cleared_boundary = match_snapshot.capture(state, combat_state)
        local cleared_state, cleared_combat = match_snapshot.restore(cleared_boundary)
        t.eq(
            match_snapshot.hash(match_snapshot.capture(cleared_state, assert(cleared_combat))),
            match_snapshot.hash(cleared_boundary)
        )
    end)

    t.it("captures and restores every nested payload as independent state", function()
        local state = new_attacking_ai_state()
        state.players[2].outfield_decision = {
            version = outfield_decision.VERSION,
            generation = 7,
            rng_state = 53,
            remaining = 0.25,
            context = "offball",
            intent = "in_behind",
            target_x = 410,
            target_y = 220,
            target_player = nil,
            run_expires_at = -0.2,
        }
        state.players[2].dive_target = Vec2.new(10, 20)
        state.players[1].keeper_state = "retreat"
        state.players[1].keeper_state_timer = 0.15
        state.players[1].keeper_release_state = "advance"
        state.players[1].keeper_release_motion = 0.75
        state.players[1].keeper_release_kind = "chip"
        state.players[1].keeper_release_depth = 40
        state.players[3].windup_shot = {
            dir = Vec2.new(0.25, -0.75),
            speed = 456,
            vz = 123,
            spin = -8,
            shot_type = "chip",
        }
        state.players[2].save_style = "stretch"
        state.players[2].save_tip_emitted = true
        state.players[2].keeper_anticipation = 0.75
        state.players[2].keeper_set = 0.125
        state.events[1] = {
            kind = "header",
            x = 44,
            y = 55,
            player = state.players[2].id,
            style = "header",
            outcome = "clean",
            jumping = true,
            difficulty = 0.4,
        }
        state.events[2] = {
            kind = "catch",
            x = 66,
            y = 77,
            player = state.players[1].id,
            save_style = "spread",
            keeper_state = "set",
            keeper_depth = 12,
        }
        local snapshot = match_snapshot.capture(state)
        local restored = match_snapshot.restore(snapshot)

        state.players[2].pos.x = -100
        state.players[2].outfield_decision.generation = 8
        state.players[2].outfield_decision.target_x = -101
        state.players[1].keeper_state = "base"
        state.players[1].keeper_release_depth = -101
        state.players[3].windup_shot.dir.y = 99
        state.events[1].x = 999
        state.events[2].save_style = "central"
        t.is_true(snapshot.state.players[2].pos.x ~= -100)
        t.eq(snapshot.state.players[2].outfield_decision.generation, 7)
        t.eq(snapshot.state.players[2].outfield_decision.target_x, 410)
        t.eq(snapshot.state.players[2].outfield_decision.run_expires_at, -0.2)
        t.eq(snapshot.state.players[1].keeper_state, "retreat")
        t.eq(snapshot.state.players[1].keeper_release_state, "advance")
        t.eq(snapshot.state.players[1].keeper_release_motion, 0.75)
        t.eq(snapshot.state.players[1].keeper_release_kind, "chip")
        t.eq(snapshot.state.players[1].keeper_release_depth, 40)
        t.eq(snapshot.state.players[3].windup_shot.dir.y, -0.75)
        t.eq(snapshot.state.players[2].save_style, "stretch")
        t.is_true(snapshot.state.players[2].save_tip_emitted)
        t.eq(snapshot.state.players[2].keeper_anticipation, 0.75)
        t.eq(snapshot.state.players[2].keeper_set, 0.125)
        t.eq(snapshot.state.events[1].x, 44)
        t.eq(snapshot.state.events[2].save_style, "spread")

        snapshot.state.players[2].pos.y = -200
        snapshot.state.players[2].outfield_decision.target_y = -201
        snapshot.state.players[1].keeper_state = "recover"
        snapshot.state.players[3].windup_shot.speed = 1
        t.is_true(restored.players[2].pos.y ~= -200)
        t.eq(restored.players[2].outfield_decision.generation, 7)
        t.eq(restored.players[2].outfield_decision.target_y, 220)
        t.eq(restored.players[2].outfield_decision.run_expires_at, -0.2)
        t.eq(restored.players[1].keeper_state, "retreat")
        t.eq(restored.players[1].keeper_state_timer, 0.15)
        t.eq(restored.players[1].keeper_release_state, "advance")
        t.eq(restored.players[1].keeper_release_motion, 0.75)
        t.eq(restored.players[1].keeper_release_kind, "chip")
        t.eq(restored.players[1].keeper_release_depth, 40)
        t.near(restored.players[1].keeper_aggression, state.players[1].keeper_aggression)
        t.near(restored.players[1].keeper_anticipation, state.players[1].keeper_anticipation)
        t.eq(restored.players[3].windup_shot.speed, 456)
        t.eq(restored.players[3].windup_shot.shot_type, "chip")
        t.near(restored.players[3].windup_shot.dir:length(), math.sqrt(0.625))
        t.eq(restored.players[2].keeper_anticipation, 0.75)
        t.eq(restored.players[2].keeper_set, 0.125)
        t.eq(restored.events[2].save_style, "spread")
        t.eq(restored.events[2].keeper_state, "set")
        t.eq(restored.events[2].keeper_depth, 12)
    end)

    t.it("keeps trusted rollback copies exact and independently owned", function()
        local state = new_state()
        state.marking.home = {
            scheme = state.marking.home.scheme,
            man_marks = state.marking.home.man_marks,
            standoff = state.marking.home.standoff,
            compactness = state.marking.home.compactness,
            support = state.marking.home.support,
        }
        state.ball_z = -0.0
        state.players[2].dash_cd = -0.0
        state.players[2].dive_target = Vec2.new(10, 20)
        state.players[2].windup_shot = {
            dir = Vec2.new(0.25, -0.75),
            speed = 456,
            vz = 123,
            spin = -8,
            shot_type = "chip",
        }
        state.events[1] = {
            kind = "header",
            x = 44,
            y = 55,
            player = state.players[2].id,
        }
        local validated = match_snapshot.capture(state)
        local owned = match_snapshot.capture_owned(state)

        t.eq(match_snapshot.encode_canonical(owned), match_snapshot.encode(validated))
        t.eq(match_snapshot.hash(owned), match_snapshot.hash(validated))
        t.eq(
            match_snapshot.encoded_size_canonical(owned),
            match_snapshot.encoded_size_canonical(validated)
        )

        state.players[2].pos.x = -100
        state.players[2].dive_target.y = -200
        state.players[2].windup_shot.dir.x = -300
        state.events[1].x = -400
        state.field.w = -500
        state.goal_home.x = -600
        state.score.home = 7
        state.press.away = 99
        state.marking.home.standoff = -123
        state.marks.home[1] = 9
        state.input_ownership.rosters.home[1] = "mutated"
        state.input_ownership.slots[1].player_id = "mutated"
        state.slot_players[1] = 9
        state.slot_for_player[1] = 9
        t.is_true(owned.state.players[2].pos.x ~= -100)
        t.is_true(owned.state.players[2].dive_target.y ~= -200)
        t.is_true(owned.state.players[2].windup_shot.dir.x ~= -300)
        t.is_true(owned.state.events[1].x ~= -400)
        t.is_true(owned.state.field.w ~= -500)
        t.is_true(owned.state.goal_home.x ~= -600)
        t.is_true(owned.state.score.home ~= 7)
        t.is_true(owned.state.press.away ~= 99)
        t.is_true(owned.state.marking.home.standoff ~= -123)
        t.is_true(owned.state.marks.home[1] ~= 9)
        t.is_true(owned.state.input_ownership.rosters.home[1] ~= "mutated")
        t.is_true(owned.state.input_ownership.slots[1].player_id ~= "mutated")
        t.is_true(owned.state.slot_players[1] ~= 9)
        t.is_true(owned.state.slot_for_player[1] ~= 9)

        local public_restored = match_snapshot.restore(owned)
        local owned_restored = match_snapshot.restore_owned(owned)
        t.eq(
            match_snapshot.hash(match_snapshot.capture_owned(owned_restored)),
            match_snapshot.hash(match_snapshot.capture(public_restored))
        )
        owned.state.players[2].pos.y = -500
        owned.state.players[2].dive_target.x = -600
        owned.state.players[2].windup_shot.speed = -700
        owned.state.events[1].y = -800
        t.is_true(owned_restored.players[2].pos.y ~= -500)
        t.is_true(owned_restored.players[2].dive_target.x ~= -600)
        t.is_true(owned_restored.players[2].windup_shot.speed ~= -700)
        t.is_true(owned_restored.events[1].y ~= -800)
        t.near(owned_restored.players[2].pos:length(), public_restored.players[2].pos:length())
    end)

    t.it("guards the shallow trusted-copy ownership contract", function()
        t.is_true(not pcall(match_snapshot.capture_owned, nil))
        t.is_true(not pcall(match_snapshot.restore_owned, nil))
        ---@type any
        local wrong_version = { version = match_snapshot.VERSION - 1, state = {} }
        ---@type any
        local missing_state = { version = match_snapshot.VERSION, state = nil }
        t.is_true(not pcall(match_snapshot.restore_owned, wrong_version))
        t.is_true(not pcall(match_snapshot.restore_owned, missing_state))
    end)

    t.it("canonically restores a v10 keeper state through goal and kickoff", function()
        local live = new_state()
        local away_keeper = live.players[6]
        away_keeper.keeper_state = "retreat"
        away_keeper.keeper_state_timer = 0.1
        away_keeper.keeper_release_state = "advance"
        away_keeper.keeper_release_motion = 0.5
        away_keeper.keeper_release_kind = "chip"
        away_keeper.keeper_release_depth = 42
        away_keeper.receive_timer = 1
        live.owner = nil
        live.ball = Vec2.new(965, 270)
        live.ball_vel = Vec2.new(600, 0)
        live.ball_z = 0
        live.ball_vz = 0
        live.pickup_cd = 1
        live.block_grace = 1

        local boundary = match_snapshot.capture(live)
        local restored = match_snapshot.restore(boundary)
        t.eq(restored.players[6].keeper_state, "retreat")
        t.eq(restored.players[6].keeper_release_kind, "chip")
        t.eq(match_snapshot.hash(match_snapshot.capture(restored)), match_snapshot.hash(boundary))

        local frame = assert(input_frame.neutral(live.input_tick))
        match.step(live, fixed_clock.TICK_SECONDS, frame)
        match.step(restored, fixed_clock.TICK_SECONDS, frame)

        t.eq(live.score.home, 1)
        t.is_true(live.kickoff_hold > 0)
        t.is_true(live.owner ~= nil and live.players[live.owner].team == "away")
        t.eq(live.players[6].keeper_state, "base")
        t.eq(live.players[6].keeper_release_kind, nil)
        t.eq(
            match_snapshot.hash(match_snapshot.capture(restored)),
            match_snapshot.hash(match_snapshot.capture(live))
        )
    end)

    t.it("converges snapshot advance restore and replay at every boundary", function()
        local live = new_state()
        local initial = match_snapshot.capture(live)
        local frames = {
            assert(input_frame.neutral(0)),
            assert(input_frame.neutral(1)),
            assert(input_frame.neutral(2)),
        }
        frames[1].slots[1] = assert(input_frame.new_sample({ move_x = 127 }))
        frames[2].slots[5] = assert(input_frame.new_sample({ move_y = -127 }))
        local hashes = { match_snapshot.hash(initial) }
        for index, frame in ipairs(frames) do
            match.step(live, fixed_clock.TICK_SECONDS, frame)
            hashes[index + 1] = match_snapshot.hash(match_snapshot.capture(live))
        end

        local restored = match_snapshot.restore(initial)
        t.is_true(restored ~= live)
        for index, frame in ipairs(frames) do
            match.step(restored, fixed_clock.TICK_SECONDS, frame)
            t.eq(
                match_snapshot.hash(match_snapshot.capture(restored)),
                hashes[index + 1],
                "restored boundary " .. index
            )
        end
        t.is_true(
            match_snapshot.first_difference(
                match_snapshot.capture(live),
                match_snapshot.capture(restored)
            ) == nil
        )
    end)

    t.it("serializes independent of table insertion order", function()
        local snapshot = match_snapshot.capture(new_state())
        local reordered_state = {}
        for index = #match_snapshot.MATCH_FIELDS, 1, -1 do
            local field = match_snapshot.MATCH_FIELDS[index]
            reordered_state[field] = snapshot.state[field]
        end
        local reordered = { state = reordered_state, version = snapshot.version }
        t.eq(match_snapshot.encode(reordered), match_snapshot.encode(snapshot))
        t.eq(match_snapshot.encode_canonical(snapshot), match_snapshot.encode(snapshot))
        t.eq(match_snapshot.encoded_size_canonical(snapshot), #match_snapshot.encode(snapshot))
        t.eq(match_snapshot.encoded_size_canonical(reordered), #match_snapshot.encode(reordered))
        t.eq(match_snapshot.hash(reordered), match_snapshot.hash(snapshot))
        t.eq(match_snapshot.hash_canonical(snapshot), match_snapshot.hash(snapshot))
    end)

    t.it("compares owned canonical snapshots without normalizing them again", function()
        local left = match_snapshot.capture(new_state())
        local right = match_snapshot.capture(new_state())
        t.eq(match_snapshot.first_difference_canonical(left, right), nil)
        right.state.score.home = 1

        local expected = assert(match_snapshot.first_difference(left, right))
        ---@type any
        local snapshot_module = match_snapshot
        local original_capture = match_snapshot.capture
        local original_restore = match_snapshot.restore
        snapshot_module.capture = function()
            error("canonical comparison must not capture")
        end
        snapshot_module.restore = function()
            error("canonical comparison must not restore")
        end
        local ok, actual = pcall(match_snapshot.first_difference_canonical, left, right)
        snapshot_module.capture = original_capture
        snapshot_module.restore = original_restore

        assert(ok, actual)
        local found = assert(actual)
        t.eq(found.path, expected.path)
        t.eq(found.expected, expected.expected)
        t.eq(found.actual, expected.actual)
    end)

    t.it("compares every canonical windup-shot field", function()
        local state = new_state()
        state.players[3].windup_shot = {
            dir = Vec2.new(0.25, -0.75),
            speed = 456,
            vz = 123,
            spin = -8,
            shot_type = "chip",
        }
        local left = match_snapshot.capture(state)
        local right = match_snapshot.capture(state)
        assert(right.state.players[3].windup_shot).shot_type = "ground"

        local found = assert(match_snapshot.first_difference_canonical(left, right))
        t.eq(found.path, "state.players.3.windup_shot.shot_type")
        t.eq(found.expected, "chip")
        t.eq(found.actual, "ground")
    end)

    t.it("compares every canonical outfield decision field", function()
        local state = new_ai_state()
        local left = match_snapshot.capture(state)
        local right = match_snapshot.capture(state)
        right.state.players[2].outfield_decision = outfield_decision.refresh(
            right.state.players[2].outfield_decision,
            "offball",
            "hold_width",
            0.5,
            450,
            20,
            nil,
            -0.3
        )
        local found = assert(match_snapshot.first_difference_canonical(left, right))
        t.eq(found.path, "state.players.2.outfield_decision.generation")

        left = match_snapshot.capture(state)
        right = match_snapshot.capture(state)
        left.state.players[2].outfield_decision = outfield_decision.refresh(
            left.state.players[2].outfield_decision,
            "offball",
            "hold_width",
            0.5,
            450,
            20,
            nil,
            -0.3
        )
        right.state.players[2].outfield_decision = outfield_decision.refresh(
            right.state.players[2].outfield_decision,
            "offball",
            "hold_width",
            0.5,
            450,
            20,
            nil,
            -0.2
        )
        local found = assert(match_snapshot.first_difference_canonical(left, right))
        t.eq(found.path, "state.players.2.outfield_decision.run_expires_at")
    end)

    t.it("restores and diffs both authoritative formation identities", function()
        local state = new_state()
        local snapshot = match_snapshot.capture(state)
        local restored = match_snapshot.restore(snapshot)
        t.eq(restored.formation.home, "2-1-1")
        t.eq(restored.formation.away, "1-1-2")

        local changed = match_snapshot.capture(state)
        changed.state.formation.home = "1-2-1"
        local found = assert(match_snapshot.first_difference_canonical(snapshot, changed))
        t.eq(found.path, "state.formation.home")

        rawset(changed.state.formation, "extra", "2-1-1")
        t.is_true(not pcall(match_snapshot.restore, changed))

        local unknown = match_snapshot.capture(state)
        unknown.state.formation.away = "future-shape"
        t.is_true(not pcall(match_snapshot.restore, unknown))
    end)

    t.it("restores and hashes team press state across soccer and combat boundaries", function()
        for _, combat_active in ipairs({ false, true }) do
            local state = new_ai_state()
            state.outfield_press.home = outfield_press.resolve(2, {
                heavy_touch = true,
                exposed_ball = false,
                cover_available = false,
                box_desperation = false,
                press_discipline = 1,
            })
            local companion = combat_active and combat.new_state(state) or nil
            local snapshot = match_snapshot.capture(state, companion)
            local restored, restored_combat = match_snapshot.restore(snapshot)
            t.eq(restored.outfield_press.home.presser_index, 2)
            t.eq(restored.outfield_press.home.mode, "commit")
            t.eq(restored.outfield_press.home.reason, "heavy_touch")
            t.eq(
                match_snapshot.hash(match_snapshot.capture(restored, restored_combat)),
                match_snapshot.hash(snapshot)
            )
        end
    end)

    t.it("diffs and strictly validates nested press state relations", function()
        local state = new_ai_state()
        state.outfield_press.home = outfield_press.contain(2)
        local left = match_snapshot.capture(state)
        local right = match_snapshot.capture(state)
        right.state.outfield_press.home = outfield_press.resolve(2, {
            heavy_touch = false,
            exposed_ball = false,
            cover_available = true,
            box_desperation = false,
            press_discipline = 1,
        })
        local found = assert(match_snapshot.first_difference_canonical(left, right))
        t.eq(found.path, "state.outfield_press.home.mode")

        local mutations = {
            function(press)
                press.version = outfield_press.VERSION + 1
            end,
            function(press)
                press.presser_index = nil
            end,
            function(press)
                press.presser_index = 99
            end,
            function(press)
                press.presser_index = 1
            end,
            function(press)
                press.presser_index = 7
            end,
            function(press)
                press.unknown = true
            end,
        }
        for index, mutate in ipairs(mutations) do
            local malformed = match_snapshot.capture(state)
            mutate(malformed.state.outfield_press.home)
            t.is_true(
                not pcall(match_snapshot.restore, malformed),
                "malformed press state " .. index .. " was accepted"
            )
        end

        local fixed = new_state()
        local fixed_snapshot = match_snapshot.capture(fixed)
        fixed_snapshot.state.outfield_press.home = outfield_press.contain(2)
        t.is_true(
            not pcall(match_snapshot.restore, fixed_snapshot),
            "fixed-slot presser relation was accepted"
        )

        local human = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 2,
            max_goals = 3,
            seed = 38,
        })
        local human_snapshot = match_snapshot.capture(human)
        human_snapshot.state.outfield_press.home = outfield_press.contain(human.controlled)
        t.is_true(
            not pcall(match_snapshot.restore, human_snapshot),
            "human-controlled presser relation was accepted"
        )
    end)

    t.it("rejects malformed v10 and v11 decision contracts during restore", function()
        local state = new_state()
        local soccer = match_snapshot.capture(state)
        local soccer_decision = soccer.state.players[2].outfield_decision
        soccer_decision.context = "offball"
        soccer_decision.intent = "shoot"
        soccer_decision.remaining = 0.2
        t.is_true(not pcall(match_snapshot.restore, soccer))

        local combat_boundary = match_snapshot.capture(state, combat.new_state(state))
        local combat_decision = combat_boundary.state.players[2].outfield_decision
        combat_decision.context = "carrier"
        combat_decision.intent = "move"
        combat_decision.remaining = 0.2
        combat_decision.target_x = 100
        combat_decision.target_y = 200
        t.is_true(not pcall(match_snapshot.restore, combat_boundary))

        state = new_attacking_ai_state()
        local valid_run = match_snapshot.capture(state)
        valid_run.state.players[2].outfield_decision = outfield_decision.refresh(
            valid_run.state.players[2].outfield_decision,
            "offball",
            "in_behind",
            0.5,
            500,
            220,
            nil,
            -0.2
        )
        t.is_true(pcall(match_snapshot.restore, valid_run))
        local active = valid_run.state.players[2].outfield_decision
        local cancelled = outfield_decision.cancel_run(active, 420, 220)
        t.eq(cancelled.generation, active.generation)
        t.eq(cancelled.remaining, active.remaining)
        t.eq(cancelled.intent, "move")
        t.eq(cancelled.run_expires_at, nil)
        valid_run.state.players[2].outfield_decision = cancelled
        t.is_true(pcall(match_snapshot.restore, valid_run))

        local mutations = {
            function(decision)
                decision.version = outfield_decision.VERSION - 1
            end,
            function(decision)
                decision.run_expires_at = nil
            end,
            function(decision)
                decision.run_expires_at = 0 / 0
            end,
            function(decision)
                decision.run_expires_at = -2
            end,
            function(decision)
                decision.run_expires_at = 0
            end,
            function(decision)
                decision.target_x = nil
                decision.target_y = nil
            end,
            function(decision)
                decision.target_player = 3
            end,
            function(decision)
                decision.context = "carrier"
            end,
            function(decision)
                decision.intent = "move"
            end,
            function(decision)
                decision.unknown_run_field = true
            end,
        }
        for index, mutate in ipairs(mutations) do
            local malformed = match_snapshot.capture(state)
            malformed.state.players[2].outfield_decision = outfield_decision.refresh(
                malformed.state.players[2].outfield_decision,
                "offball",
                "come_short",
                0.5,
                300,
                220,
                nil,
                -0.2
            )
            mutate(malformed.state.players[2].outfield_decision)
            t.is_true(
                not pcall(match_snapshot.restore, malformed),
                "malformed v9 run decision " .. index .. " was accepted"
            )
        end

        local malformed_combat = match_snapshot.capture(state, combat.new_state(state))
        malformed_combat.state.players[7].outfield_decision = outfield_decision.refresh(
            malformed_combat.state.players[7].outfield_decision,
            "offball",
            "hold_width",
            0.5,
            500,
            30,
            nil,
            -0.2
        )
        malformed_combat.state.players[7].outfield_decision.run_expires_at = math.huge
        t.is_true(not pcall(match_snapshot.restore, malformed_combat))

        local too_many = new_attacking_ai_state()
        local too_many_snapshot = match_snapshot.capture(too_many)
        for _, index in ipairs({ 2, 3, 4 }) do
            local player = too_many_snapshot.state.players[index]
            player.outfield_decision = outfield_decision.refresh(
                player.outfield_decision,
                "offball",
                "in_behind",
                player.scan_rate,
                500 + index,
                200 + index,
                nil,
                -0.2
            )
        end
        t.is_true(
            not pcall(match_snapshot.restore, too_many_snapshot),
            "third same-team run was accepted"
        )

        ---@param relation_state MatchState
        ---@param runner_index integer
        ---@return integer owner_index
        local function set_ordinary_teammate_owner(relation_state, runner_index)
            local runner = relation_state.players[runner_index]
            for index, player in ipairs(relation_state.players) do
                if
                    index ~= runner_index
                    and player.team == runner.team
                    and not player.is_keeper
                then
                    relation_state.owner = index
                    relation_state.kickoff_hold = 0
                    relation_state.finished = false
                    return index
                end
            end
            assert(false, "relation fixture requires an ordinary teammate owner")
            return 0
        end

        local invalid_relations = {
            {
                name = "keeper",
                state = new_attacking_ai_state(),
                player_index = 1,
                target_x = 500,
                target_y = 200,
            },
            {
                name = "fixed slot",
                state = new_state(),
                player_index = 2,
                target_x = 500,
                target_y = 200,
            },
            {
                name = "outside field",
                state = new_attacking_ai_state(),
                player_index = 2,
                target_x = -1,
                target_y = 200,
            },
        }
        local human = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 2,
            max_goals = 3,
            seed = 38,
        })
        local fixed_case = invalid_relations[2]
        fixed_case.state.human_controlled = false
        set_ordinary_teammate_owner(fixed_case.state, fixed_case.player_index)
        set_ordinary_teammate_owner(human, human.controlled)
        invalid_relations[#invalid_relations + 1] = {
            name = "human control",
            state = human,
            player_index = human.controlled,
            target_x = 500,
            target_y = 200,
        }
        for _, case in ipairs(invalid_relations) do
            if case.name == "fixed slot" or case.name == "human control" then
                local relation_state = case.state
                local relation_player = relation_state.players[case.player_index]
                local relation_owner = relation_state.players[assert(relation_state.owner)]
                t.is_true(not relation_player.is_keeper, case.name .. " runner is a keeper")
                t.is_true(
                    relation_state.owner ~= case.player_index,
                    case.name .. " runner owns the ball"
                )
                t.eq(
                    relation_owner.team,
                    relation_player.team,
                    case.name .. " owner belongs to the wrong team"
                )
                t.is_true(not relation_owner.is_keeper, case.name .. " owner is a keeper")
                t.is_true(
                    relation_state.kickoff_hold <= 0 and not relation_state.finished,
                    case.name .. " fixture is outside ordinary attack"
                )
                if case.name == "fixed slot" then
                    t.is_true(
                        relation_state.slot_for_player[case.player_index] ~= nil,
                        "fixed-slot fixture runner has no slot"
                    )
                    t.is_true(
                        not relation_state.human_controlled
                            or relation_state.controlled ~= case.player_index,
                        "fixed-slot fixture also violates human control"
                    )
                else
                    t.eq(
                        relation_state.slot_for_player[case.player_index],
                        nil,
                        "human fixture runner owns a fixed slot"
                    )
                    t.is_true(
                        relation_state.human_controlled
                            and relation_state.controlled == case.player_index,
                        "human fixture runner is not human-controlled"
                    )
                end
            end
            local snapshot = match_snapshot.capture(case.state)
            local player = snapshot.state.players[case.player_index]
            player.outfield_decision = outfield_decision.refresh(
                player.outfield_decision,
                "offball",
                "in_behind",
                player.scan_rate,
                case.target_x,
                case.target_y,
                nil,
                -0.2
            )
            t.is_true(
                not pcall(match_snapshot.restore, snapshot),
                case.name .. " run relation was accepted"
            )
        end

        ---@return MatchSnapshot
        local function ordinary_run_boundary()
            local snapshot = match_snapshot.capture(new_attacking_ai_state())
            local player = snapshot.state.players[2]
            player.outfield_decision = outfield_decision.refresh(
                player.outfield_decision,
                "offball",
                "come_short",
                player.scan_rate,
                360,
                220,
                nil,
                -0.2
            )
            return snapshot
        end

        local possession_mutations = {
            {
                name = "loose ball",
                mutate = function(snapshot)
                    snapshot.state.owner = nil
                end,
            },
            {
                name = "opponent possession",
                mutate = function(snapshot)
                    snapshot.state.owner = 7
                end,
            },
            {
                name = "keeper build-up",
                mutate = function(snapshot)
                    snapshot.state.owner = 1
                end,
            },
            {
                name = "kickoff hold",
                mutate = function(snapshot)
                    snapshot.state.kickoff_hold = 0.2
                end,
            },
            {
                name = "finished match",
                mutate = function(snapshot)
                    snapshot.state.finished = true
                end,
            },
        }
        for _, case in ipairs(possession_mutations) do
            local malformed = ordinary_run_boundary()
            case.mutate(malformed)
            t.is_true(
                not pcall(match_snapshot.restore, malformed),
                case.name .. " retained an active run"
            )
        end

        local commitment_mutations = {
            {
                name = "stun",
                mutate = function(player)
                    player.stun_timer = 0.2
                end,
            },
            {
                name = "slide",
                mutate = function(player)
                    player.slide_timer = 0.2
                end,
            },
            {
                name = "tackle",
                mutate = function(player)
                    player.tackle_timer = 0.2
                end,
            },
            {
                name = "dodge",
                mutate = function(player)
                    player.dodge_timer = 0.2
                end,
            },
            {
                name = "jockey",
                mutate = function(player)
                    player.jockey_timer = 0.2
                end,
            },
            {
                name = "wind-up timer",
                mutate = function(player)
                    player.windup_timer = 0.2
                end,
            },
            {
                name = "wind-up payload",
                mutate = function(player)
                    player.windup_shot = {
                        dir = Vec2.new(1, 0),
                        speed = 300,
                        vz = 0,
                        spin = 0,
                        shot_type = "ground",
                    }
                end,
            },
            {
                name = "aerial action",
                mutate = function(player)
                    player.aerial_timer = 0.2
                end,
            },
            {
                name = "aerial recovery",
                mutate = function(player)
                    player.aerial_recovery = 0.2
                end,
            },
            {
                name = "reception",
                mutate = function(player)
                    player.receive_timer = 0.2
                end,
            },
        }
        for _, case in ipairs(commitment_mutations) do
            local malformed = ordinary_run_boundary()
            case.mutate(malformed.state.players[2])
            t.is_true(
                not pcall(match_snapshot.restore, malformed),
                case.name .. " retained an active run"
            )
        end
    end)

    t.it("encodes decision children positionally with exact no-run v11 arithmetic", function()
        local legacy_fields = {
            "version",
            "generation",
            "rng_state",
            "remaining",
            "context",
            "intent",
            "target_x",
            "target_y",
            "target_player",
        }
        local legacy_key_bytes = 0
        for _, field in ipairs(legacy_fields) do
            legacy_key_bytes = legacy_key_bytes + #("k" .. tostring(#field) .. ":" .. field .. ";")
        end
        t.eq(legacy_key_bytes, 115)

        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 120,
            max_goals = 3,
            seed = 38,
            input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
        })
        for tick = 0, 119 do
            match.step(state, fixed_clock.TICK_SECONDS, assert(input_frame.neutral(tick)))
        end
        -- A slot-mode neutral fixture never turns the ball over, so the
        -- transition block prices its established-but-idle spelling exactly.
        t.eq(state.transition.last_team, "home")
        t.eq(state.transition.holding_team, "home")
        t.eq(state.transition.hold, possession_transition.ESTABLISH_SECONDS)
        t.eq(state.transition.turnover_team, nil)
        t.eq(state.transition.elapsed, 0)
        local balanced_window = #("n" .. match_snapshot.number_bytes(2.5) .. ";")
        local transition_bytes = #"k18:transition_windows;"
            + 2 * (#"k12:counterpress;" + #"k13:counterattack;" + 2 * balanced_window)
            + #"k10:transition;"
            + #"k7:version;"
            + #("n" .. match_snapshot.number_bytes(1) .. ";")
            + #"k9:last_team;"
            + #"s4:home;"
            + #"k12:holding_team;"
            + #"s4:home;"
            + #"k4:hold;"
            + #("n" .. match_snapshot.number_bytes(possession_transition.ESTABLISH_SECONDS) .. ";")
            + #"k13:turnover_team;"
            + #"z;"
            + #"k7:elapsed;"
            + #"nz;"
        t.eq(transition_bytes, 311)

        local encoded = match_snapshot.encode(match_snapshot.capture(state))
        local expected = 21343
            - 10 * legacy_key_bytes
            + 10 * #"z;"
            + #"k9:formation;"
            + #"s5:2-1-1;"
            + #"s5:1-1-2;"
            + 10 * #"k19:keeper_get_up_timer;nz;"
            + transition_bytes
        t.eq(#encoded, expected)
        t.eq(#encoded, 20825)

        local decision_marker = "k17:outfield_decision;d;"
        local next_field_marker = "k9:is_keeper;"
        local search_at = 1
        local count = 0
        while true do
            local start_at = encoded:find(decision_marker, search_at, true)
            if not start_at then
                break
            end
            local decision_start = start_at + #decision_marker
            local decision_end = assert(encoded:find(next_field_marker, decision_start, true))
            local decision_wire = encoded:sub(decision_start, decision_end - 1)
            t.is_true(decision_wire:find("k", 1, true) == nil, "decision child emitted a named key")
            count = count + 1
            search_at = decision_end + #next_field_marker
        end
        t.eq(count, 10)
    end)

    t.it("prices four hypothetical runs on the valid high-overhead slot-mode base", function()
        local state = new_state()
        t.is_true(state.slot_mode)
        t.is_true(state.input_ownership ~= nil)
        for slot_index = 1, input_frame.SLOT_COUNT do
            local player_index = assert(state.slot_players[slot_index])
            t.eq(state.slot_for_player[player_index], slot_index)
        end

        local soccer = match_snapshot.capture(state)
        local combat_state = combat.new_state(state)
        local combat_boundary = match_snapshot.capture(state, combat_state)
        local soccer_bytes = match_snapshot.encoded_size_canonical(soccer)
        local combat_bytes = match_snapshot.encoded_size_canonical(combat_boundary)
        t.eq(soccer.state.outfield_press.home.version, outfield_press.VERSION)
        t.eq(soccer.state.outfield_press.away.version, outfield_press.VERSION)

        ---@param value number|string|boolean?
        ---@return integer
        local function scalar_bytes(value)
            local kind = type(value)
            if value == nil then
                return #"z;"
            elseif kind == "number" then
                ---@cast value number
                return #match_snapshot.number_bytes(value) + 2
            elseif kind == "string" then
                ---@cast value string
                return #("s" .. tostring(#value) .. ":" .. value .. ";")
            end
            return #"b0;"
        end

        ---@type string[]
        local decision_fields = {
            "version",
            "generation",
            "rng_state",
            "remaining",
            "context",
            "intent",
            "target_x",
            "target_y",
            "target_player",
            "run_expires_at",
        }
        ---@type { player_index: integer, intent: OutfieldIntent, x: number, y: number }[]
        local run_records = {
            { player_index = 2, intent = "come_short", x = 350, y = 120 },
            { player_index = 3, intent = "in_behind", x = 700, y = 180 },
            { player_index = 7, intent = "hold_width", x = 610, y = 510 },
            { player_index = 8, intent = "in_behind", x = 220, y = 300 },
        }
        local four_run_delta = 0
        for _, run in ipairs(run_records) do
            local player = soccer.state.players[run.player_index]
            local base_decision = player.outfield_decision
            local active_decision = outfield_decision.refresh(
                base_decision,
                "offball",
                run.intent,
                player.scan_rate,
                run.x,
                run.y,
                nil,
                -0.2
            )
            t.eq(active_decision.generation, base_decision.generation + 1)
            t.is_true(active_decision.remaining > 0)
            for _, field in ipairs(decision_fields) do
                four_run_delta = four_run_delta
                    + scalar_bytes(active_decision[field])
                    - scalar_bytes(base_decision[field])
            end
        end

        ---@type string[]
        local press_fields = { "version", "presser_index", "mode", "reason" }
        local press_delta = 0
        for _, team in ipairs({ "home", "away" }) do
            local base_press = soccer.state.outfield_press[team]
            local active_press = outfield_press.contain(team == "home" and 2 or 7)
            for _, field in ipairs(press_fields) do
                press_delta = press_delta
                    + scalar_bytes(active_press[field])
                    - scalar_bytes(base_press[field])
            end
        end
        local combined_delta = four_run_delta + press_delta
        -- Budget and boundary count come from the gated table rather than literals, so
        -- this diagnostic and `main.lua`'s gate can never describe different ceilings.
        -- spec/sim/rollback_validation_spec.lua pins the table's own values.
        local budget = validation_config.budgets.snapshot_bytes
        local boundaries = validation_config.budgets.snapshot_count
        local soccer_window = (soccer_bytes + combined_delta) * boundaries
        local combat_window = (combat_bytes + combined_delta) * boundaries
        -- `budget=` is part of the marker because scripts/check_snapshot_headroom.sh
        -- gates on this line and must not carry its own copy of the ceiling.
        print(
            ("snapshot_active_ai_budget budget=%d soccer_base=%d combat_base=%d run_delta=%d press_delta=%d combined_delta=%d soccer_window=%d combat_window=%d soccer_headroom=%d combat_headroom=%d"):format(
                budget,
                soccer_bytes,
                combat_bytes,
                four_run_delta,
                press_delta,
                combined_delta,
                soccer_window,
                combat_window,
                budget - soccer_window,
                budget - combat_window
            )
        )
        t.eq(boundaries, 31)
        t.eq(soccer_bytes, 20697)
        t.eq(combat_bytes, 24373)
        t.eq(four_run_delta, 346)
        t.eq(press_delta, 26)
        t.eq(combined_delta, 372)
        t.eq(soccer_window, 653139)
        t.eq(combat_window, 767095)
        t.eq(budget - soccer_window, 264365)
        t.eq(budget - combat_window, 150409)
        t.is_true(soccer_window < budget)
        t.is_true(combat_window < budget)
    end)

    -- The budget measurement above prices a `combat.new_state`, whose `events` array is
    -- empty, so until now the per-event footprint was unpriced -- a gap a reviewer found
    -- on #279, the change that added `outcome`, `reason` and `terminal` to every row.
    -- `combat_snapshot.copy` requires every retained event to carry `tick ==
    -- combat_state.tick - 1`, so a snapshot holds exactly one tick's events; the
    -- quantity that matters is therefore bytes per row times rows per tick.
    t.it("prices the worst-case combat event row against the retained window", function()
        local state = new_state()
        local combat_state = combat.new_state(state)
        local snapshot = match_snapshot.capture(state, combat_state)
        t.eq(#snapshot.combat.events, 0)

        local budget = validation_config.budgets.snapshot_bytes
        local boundaries = validation_config.budgets.snapshot_count
        local combat_bytes = match_snapshot.encoded_size_canonical(snapshot)
        t.eq(combat_bytes, 24373)
        -- The active-AI delta priced by the measurement above.
        local combined_delta = 372
        local combat_window = (combat_bytes + combined_delta) * boundaries
        t.eq(combat_window, 767095)

        ---@param values string[]
        ---@return string
        local function longest(values)
            local best = values[1]
            for _, value in ipairs(values) do
                -- Ties break lexicographically so the choice does not depend on
                -- table order.
                if #value > #best or (#value == #best and value < best) then
                    best = value
                end
            end
            return best
        end

        local family_ids = {}
        for id in pairs(action_families) do
            family_ids[#family_ids + 1] = id
        end
        table.sort(family_ids)

        -- Every closed vocabulary is spelled verbatim into the row, so the widest row
        -- the encoder can be asked to write uses the longest member of each. Numbers
        -- take the widest canonical spelling the simulation actually produces: pitch
        -- coordinates with a full fractional mantissa, a late-match tick, and a
        -- four-digit sequence.
        local worst_row = {
            kind = longest(combat_snapshot.EVENT_KINDS),
            tick = 3599,
            family_id = longest(family_ids),
            source_index = 10,
            target_index = 10,
            source_sequence = 9999,
            result = longest(combat_snapshot.CONTACT_RESULTS),
            outcome = longest(combat_snapshot.REQUEST_OUTCOMES),
            reason = longest(combat_snapshot.REJECTION_REASONS),
            terminal = longest(combat_snapshot.ENCOUNTER_TERMINALS),
            x = 959.9999999999999,
            y = 539.9999999999999,
            interruption_ticks = 30,
            displacement_px = 123.45678901234567,
        }
        t.eq(worst_row.kind, "projectile_expire")
        t.eq(worst_row.reason, "protected_keeper_or_no_loadout")
        t.eq(worst_row.terminal, "match_terminated")
        t.eq(worst_row.result, "superseded")
        t.eq(worst_row.family_id, "light_melee")

        -- The marginal cost of a row, measured rather than derived: going from one row
        -- to two isolates it from the array-length scalar, which changes width on its
        -- own. Mutating the captured snapshot bypasses `combat_snapshot.copy`'s
        -- validators on purpose -- this measures the encoder, not the simulator, and a
        -- valid row of this exact width is not constructible from one tick of play.
        snapshot.combat.events[1] = worst_row
        local one_row = match_snapshot.encoded_size_canonical(snapshot)
        snapshot.combat.events[2] = worst_row
        local two_rows = match_snapshot.encoded_size_canonical(snapshot)
        local row_bytes = two_rows - one_row

        -- Field names are re-emitted for all fourteen fields on every row, including
        -- the ones the row leaves nil. That share is the concrete target for narrowing
        -- the canonical encoding.
        local key_bytes = 0
        for _, field in ipairs(combat_snapshot.EVENT_FIELDS) do
            key_bytes = key_bytes + #("k" .. tostring(#field) .. ":" .. field .. ";")
        end

        -- Worst-case rows in one tick, from the emitter analysis recorded in
        -- docs/online/omp2_rollback_validation.md:
        --   P + 2*O + 1 + (1 + J) * R
        -- with P = 10 players, O = 8 non-keeper outfielders, R <= O ranged players and
        -- J = 2 concurrent projectiles per ranged player. It sums independently
        -- maximised terms, so it is a ceiling rather than a constructed witness.
        local worst_rows_per_tick = 10 + 2 * 8 + 1 + 3 * 8
        t.eq(worst_rows_per_tick, 51)

        local headroom = budget - combat_window
        local rows_per_boundary = math.floor(headroom / boundaries / row_bytes)
        local worst_tick_window = (combat_bytes + combined_delta + worst_rows_per_tick * row_bytes)
            * boundaries

        print(
            ("snapshot_combat_event_cost row_bytes=%d key_bytes=%d key_share_percent=%.1f worst_rows_per_tick=%d worst_tick_window=%d rows_per_boundary=%d"):format(
                row_bytes,
                key_bytes,
                key_bytes / row_bytes * 100,
                worst_rows_per_tick,
                worst_tick_window,
                rows_per_boundary
            )
        )

        t.eq(key_bytes, 179)
        t.eq(row_bytes, 456)

        -- The margin, pinned in rows rather than assumed. Each retained boundary can
        -- absorb this many worst-case event rows before the 896-KiB gate; #279's three
        -- added fields cost 33 key bytes plus their values out of the 456 above, so a
        -- further schema addition of that shape moves this number.
        t.eq(rows_per_boundary, 10)

        -- A realistic sustained ceiling -- one own-encounter row per outfielder on every
        -- one of the 31 retained boundaries -- fits with room to spare. The committed
        -- crowded fixtures average well below one event per tick.
        local sustained_window = (combat_bytes + combined_delta + 8 * row_bytes) * boundaries
        t.is_true(sustained_window < budget)
        t.eq(sustained_window, 880183)

        -- A recorded limit, not an aspiration: the adversarial 51-row tick sustained
        -- across all 31 boundaries does *not* fit, and no raise of this size would make
        -- it fit -- it needs 1,488,031 bytes. Narrowing the canonical encoding is the
        -- lever for that, and this assertion is what will tell us when it has moved.
        t.is_true(worst_tick_window > budget)
        t.eq(worst_tick_window, 1488031)
    end)

    t.it("rejects unhandled state and player fields", function()
        local state = new_state()
        rawset(state, "future_match_field", 1)
        t.is_true(not pcall(match_snapshot.capture, state))
        rawset(state, "future_match_field", nil)
        rawset(state.players[1], "future_player_field", true)
        t.is_true(not pcall(match_snapshot.capture, state))
    end)

    t.it("rejects the prior snapshot schema instead of inventing keeper state", function()
        local snapshot = match_snapshot.capture(new_state())
        snapshot.version = match_snapshot.VERSION - 1
        t.is_true(not pcall(match_snapshot.restore, snapshot))
    end)

    t.it("uses exact canonical finite-number spelling", function()
        t.eq(match_snapshot.number_bytes(0), "z")
        t.eq(match_snapshot.number_bytes(-0.0), "Z")
        t.eq(match_snapshot.number_bytes(0.5), "p:0:33554432:0")
        t.eq(match_snapshot.number_bytes(-1), "m:1:33554432:0")
        t.is_true(
            match_snapshot.number_bytes(0.1) ~= match_snapshot.number_bytes(0.10000000000000002)
        )
        t.is_true(not pcall(match_snapshot.number_bytes, 0 / 0))
        t.is_true(not pcall(match_snapshot.number_bytes, math.huge))
    end)
end)
