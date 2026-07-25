local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_snapshot = require("sim.combat_snapshot")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")

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

    t.it("captures and restores every nested payload as independent state", function()
        local state = new_state()
        state.players[2].dive_target = Vec2.new(10, 20)
        state.players[1].keeper_state = "retreat"
        state.players[1].keeper_state_timer = 0.15
        state.players[1].keeper_release_state = "advance"
        state.players[1].keeper_release_motion = 0.75
        state.players[1].keeper_release_kind = "chip"
        state.players[1].keeper_release_depth = 40
        state.players[2].windup_shot = {
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
        state.players[1].keeper_state = "base"
        state.players[1].keeper_release_depth = -101
        state.players[2].windup_shot.dir.y = 99
        state.events[1].x = 999
        state.events[2].save_style = "central"
        t.is_true(snapshot.state.players[2].pos.x ~= -100)
        t.eq(snapshot.state.players[1].keeper_state, "retreat")
        t.eq(snapshot.state.players[1].keeper_release_state, "advance")
        t.eq(snapshot.state.players[1].keeper_release_motion, 0.75)
        t.eq(snapshot.state.players[1].keeper_release_kind, "chip")
        t.eq(snapshot.state.players[1].keeper_release_depth, 40)
        t.eq(snapshot.state.players[2].windup_shot.dir.y, -0.75)
        t.eq(snapshot.state.players[2].save_style, "stretch")
        t.is_true(snapshot.state.players[2].save_tip_emitted)
        t.eq(snapshot.state.players[2].keeper_anticipation, 0.75)
        t.eq(snapshot.state.players[2].keeper_set, 0.125)
        t.eq(snapshot.state.events[1].x, 44)
        t.eq(snapshot.state.events[2].save_style, "spread")

        snapshot.state.players[2].pos.y = -200
        snapshot.state.players[1].keeper_state = "recover"
        snapshot.state.players[2].windup_shot.speed = 1
        t.is_true(restored.players[2].pos.y ~= -200)
        t.eq(restored.players[1].keeper_state, "retreat")
        t.eq(restored.players[1].keeper_state_timer, 0.15)
        t.eq(restored.players[1].keeper_release_state, "advance")
        t.eq(restored.players[1].keeper_release_motion, 0.75)
        t.eq(restored.players[1].keeper_release_kind, "chip")
        t.eq(restored.players[1].keeper_release_depth, 40)
        t.near(restored.players[1].keeper_aggression, state.players[1].keeper_aggression)
        t.near(restored.players[1].keeper_anticipation, state.players[1].keeper_anticipation)
        t.eq(restored.players[2].windup_shot.speed, 456)
        t.eq(restored.players[2].windup_shot.shot_type, "chip")
        t.near(restored.players[2].windup_shot.dir:length(), math.sqrt(0.625))
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

    t.it("canonically restores a v5 keeper state through goal and kickoff", function()
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
        state.players[2].windup_shot = {
            dir = Vec2.new(0.25, -0.75),
            speed = 456,
            vz = 123,
            spin = -8,
            shot_type = "chip",
        }
        local left = match_snapshot.capture(state)
        local right = match_snapshot.capture(state)
        assert(right.state.players[2].windup_shot).shot_type = "ground"

        local found = assert(match_snapshot.first_difference_canonical(left, right))
        t.eq(found.path, "state.players.2.windup_shot.shot_type")
        t.eq(found.expected, "chip")
        t.eq(found.actual, "ground")
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
