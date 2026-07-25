local Vec2 = require("core.vec2")
local combat_presentation = require("game.presentation.combat")
local player_pose = require("game.presentation.player_pose")
local combat = require("sim.combat")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")
local t = require("spec.support.runner")

---@return MatchState, CombatMatchState
local function fixture()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
    })
    return state, combat.new_state(state)
end

t.describe("combat presentation projection", function()
    t.it("projects fixed loadouts, phases, readiness, forced state, and projectiles", function()
        local state, combat_state = fixture()
        local family_indices = {}
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id and not family_indices[runtime.family_id] then
                family_indices[runtime.family_id] = index
            end
        end

        local unarmed = combat_state.players[assert(family_indices.unarmed)]
        unarmed.phase = "windup"
        unarmed.phase_ticks = 3
        unarmed.cooldown_ticks = 21
        local guard = combat_state.players[assert(family_indices.guard)]
        guard.phase = "guard"
        local melee = combat_state.players[assert(family_indices.light_melee)]
        melee.forced_state = "knockback"
        melee.forced_ticks = 7
        local ranged_index = assert(family_indices.ranged)
        local ranged = combat_state.players[ranged_index]
        ranged.phase = "aim"
        ranged.cooldown_ticks = 41
        combat_state.projectiles[1] = {
            family_id = "ranged",
            source_index = ranged_index,
            source_sequence = 12,
            pos = Vec2.new(400, 260),
            dir = Vec2.new(1, 0),
            remaining_ticks = 44,
        }

        local model = combat_presentation.model(state, combat_state)
        t.is_true(model.enabled)
        t.eq(model.tick, 0)
        t.eq(#model.players, 10)
        t.eq(#model.projectiles, 1)
        t.eq(model.players[family_indices.unarmed].telegraph_kind, "arc")
        t.eq(model.players[family_indices.guard].telegraph_kind, "guard_arc")
        t.eq(model.players[ranged_index].telegraph_kind, "line")
        t.eq(model.players[family_indices.light_melee].readiness, "forced")
        t.eq(model.players[family_indices.light_melee].forced_state, "knockback")
        t.eq(model.projectiles[1].source_sequence, 12)
        t.near(assert(model.players[ranged_index].projectile_range_px), 300, 1e-9)
        t.eq(model.players[1].readiness, "unavailable")
        t.is_true(model.players[1].equipment_presentation_id == nil)
    end)

    t.it("is pure and keeps presentation identity outside simulation hashes", function()
        local state, combat_state = fixture()
        local before = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        local first = combat_presentation.model(state, combat_state)
        local second = combat_presentation.model(state, combat_state)
        local after = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        t.eq(before, after)
        t.eq(
            first.players[2].equipment_presentation_id,
            second.players[2].equipment_presentation_id
        )
        t.is_true(combat_presentation.model(state, nil).enabled == false)
    end)

    t.it("maps authoritative event records to stable semantic presentation ids", function()
        local contact = combat_presentation.event({
            kind = "contact",
            tick = 8,
            family_id = "guard",
            source_index = 2,
            target_index = 7,
            source_sequence = 4,
            result = "guarded",
            x = 300,
            y = 240,
        }, "combat/8/4/contact")
        t.eq(contact.semantic_id, "combat.contact.guarded")
        t.eq(contact.stable_id, "combat/8/4/contact")

        local spawn = combat_presentation.event({
            kind = "projectile_spawn",
            tick = 9,
            family_id = "ranged",
            source_index = 4,
            source_sequence = 5,
            x = 320,
            y = 240,
        })
        t.eq(spawn.semantic_id, "combat.projectile.spawn")
    end)
end)

t.describe("shared player pose priority", function()
    t.it("keeps keeper, aerial, forced, and committed combat poses in one order", function()
        local state, combat_state = fixture()
        local index = 2
        local player = state.players[index]
        local runtime = combat_state.players[index]
        runtime.phase = "windup"
        runtime.phase_ticks = 2
        local sample = combat_presentation.model(state, combat_state).players[index]
        t.eq(player_pose.select(player, sample).id, "combat_windup")

        runtime.forced_state = "stagger"
        runtime.forced_ticks = 5
        sample = combat_presentation.model(state, combat_state).players[index]
        t.eq(player_pose.select(player, sample).id, "combat_stagger")

        player.aerial_timer = 0.2
        player.aerial_style = "header"
        t.eq(player_pose.select(player, sample).id, "aerial_action")

        player.aerial_timer = 0
        player.aerial_style = nil
        player.is_keeper = true
        player.dive_timer = 0.2
        player.dive_dir = Vec2.new(1, 0)
        t.eq(player_pose.select(player, sample).id, "keeper_dive")
        t.is_true(
            player_pose.PRIORITY.keeper_dive > player_pose.PRIORITY.aerial_action
                and player_pose.PRIORITY.aerial_action > player_pose.PRIORITY.combat_knockback
                and player_pose.PRIORITY.combat_knockback > player_pose.PRIORITY.combat_active
                and player_pose.PRIORITY.combat_active > player_pose.PRIORITY.soccer_windup,
            "the extensible priority contract is explicit"
        )
    end)

    t.it("chooses overlapping poses from declared priority with a stable tie rule", function()
        local state = fixture()
        local player = state.players[2]
        player.windup_timer = 0.2
        player.slide_timer = 0.2

        local original_slide = player_pose.PRIORITY.slide
        local ok, result = pcall(function()
            local selected = {
                default = player_pose.select(player, nil).id,
            }
            player_pose.PRIORITY.slide = player_pose.PRIORITY.soccer_windup + 1
            selected.raised = player_pose.select(player, nil).id
            player_pose.PRIORITY.slide = player_pose.PRIORITY.soccer_windup
            selected.tied = player_pose.select(player, nil).id
            return selected
        end)
        player_pose.PRIORITY.slide = original_slide
        assert(ok, result)
        ---@cast result table<string, PlayerPoseId>

        t.eq(result.default, "soccer_windup")
        t.eq(result.raised, "slide")
        t.eq(result.tied, "slide", "equal priorities choose the lexically smaller pose id")
    end)
end)
