local combat_feedback = require("game.presentation.combat_feedback")
local combat_presentation = require("game.presentation.combat")
local loadouts = require("data.loadouts")
local match_snapshot = require("sim.match_snapshot")
local combat = require("sim.combat")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

---@param kind CombatEventKind
---@param result CombatContactResult?
---@return CombatEvent
local function event(kind, result)
    return {
        kind = kind,
        tick = 12,
        family_id = kind == "guard_recoil" and "guard" or "light_melee",
        source_index = 2,
        target_index = kind == "commit" or kind == "projectile_spawn" and nil or 7,
        source_sequence = 4,
        result = result,
        x = 320,
        y = 240,
    }
end

t.describe("combat feedback presentation contract", function()
    t.it(
        "gives every supported authoritative result a deliberate multimodal disposition",
        function()
            local fixtures = {
                event("commit"),
                event("projectile_spawn"),
                event("projectile_expire"),
                event("contact", "hit"),
                event("contact", "extended"),
                event("contact", "guarded"),
                event("contact", "immune"),
                event("contact", "superseded"),
                event("ball_spill"),
                event("forced"),
                event("guard_recoil"),
            }
            local semantic_ids = {}
            for index, value in ipairs(fixtures) do
                local link = combat_feedback.accepted(value, "accepted/" .. index)
                t.eq(link.link_kind, "accepted_encounter")
                t.eq(link.source_sequence, 4)
                t.eq(link.feedback_event_id, "accepted/" .. index)
                t.is_true(link.disposition.vfx ~= "none")
                t.is_true(link.disposition.audio ~= nil)
                t.is_true(link.disposition.glyph ~= "")
                semantic_ids[link.semantic_id] = true
            end
            t.eq(#fixtures, 11)
            local semantic_count = 0
            for _ in pairs(semantic_ids) do
                semantic_count = semantic_count + 1
            end
            t.eq(semantic_count, 11)
        end
    )

    t.it("keeps accepted, rejected, and missing-feedback linkage disjoint", function()
        local accepted = combat_feedback.accepted(event("contact", "hit"), "accepted/contact")
        local rejected =
            combat_feedback.rejected_request("request/rejected", 14, 2, "soccer_commitment")
        local missing =
            combat_feedback.missing_feedback("request/missing", 15, 2, "missing_press_edge")

        t.eq(accepted.link_kind, "accepted_encounter")
        t.eq(accepted.source_sequence, 4)
        t.eq(rejected.link_kind, "rejected_request")
        t.is_true(rejected.source_sequence == nil)
        t.eq(rejected.reason, "soccer_commitment")
        t.eq(rejected.feedback_event_id, "request/rejected")
        t.eq(rejected.disposition.vfx, "rejected")
        t.eq(missing.link_kind, "missing_feedback")
        t.is_true(missing.source_sequence == nil)
        t.is_true(missing.feedback_event_id == nil)
        t.eq(missing.disposition.vfx, "none")
        t.is_true(missing.disposition.audio == nil)
    end)

    t.it("deduplicates confirmed HUD/camera feedback and bounds reduced-motion behavior", function()
        local state = combat_feedback.new()
        local link = combat_feedback.accepted(event("contact", "hit"), "confirmed/hit")
        t.is_true(combat_feedback.confirm(state, link))
        t.is_true(not combat_feedback.confirm(state, link))
        t.eq(combat_feedback.diagnostics(state).confirmed_count, 1)
        t.eq(assert(combat_feedback.notice(state, 2)).text, "CLEAN HIT")
        t.is_true(combat_feedback.notice(state, 5) == nil)
        local offset = combat_feedback.camera_offset(state)
        t.is_true(math.abs(offset.x) <= 4 and math.abs(offset.y) <= 4)

        local observation = combat_feedback.observation(state, link, "confirmed")
        t.is_true(observation.camera_enabled)
        t.eq(observation.cue_state, "confirmed")

        combat_feedback.configure(state, true, true)
        offset = combat_feedback.camera_offset(state)
        t.eq(offset.x, 0)
        t.eq(offset.y, 0)
        observation = combat_feedback.observation(state, link, "confirmed")
        t.is_true(not observation.camera_enabled)
        t.is_true(observation.reduced_motion)
        t.is_true(observation.reduced_flash)

        combat_feedback.tick(state, 1)
        t.is_true(combat_feedback.notice(state, 2) == nil)
    end)

    t.it("never mutates simulation truth while projecting feedback", function()
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
        })
        local combat_state = combat.new_state(state)
        local before = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        local link = combat_feedback.accepted(event("contact", "guarded"), "pure/guarded")
        local feedback_state = combat_feedback.new()
        combat_feedback.confirm(feedback_state, link)
        combat_feedback.tick(feedback_state, 1 / 60)
        combat_feedback.observation(feedback_state, link, "confirmed")
        local after = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        t.eq(before, after)
    end)

    t.it("keeps simulation identity stable across a same-family presentation swap", function()
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
        })
        local combat_state = combat.new_state(state)
        local player_index
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "light_melee" then
                player_index = index
                break
            end
        end
        local index = assert(player_index)
        local runtime = combat_state.players[index]
        local loadout = assert(loadouts[assert(runtime.loadout_id)])
        local original = loadout.equipment_presentation_id
        local swapped = original == "toy_foam_sword" and "scifi_energy_blade" or "toy_foam_sword"
        local before_hash = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        local before_name =
            assert(combat_presentation.model(state, combat_state).players[index].equipment_name)
        local ok, swapped_name = pcall(function()
            loadout.equipment_presentation_id = swapped
            return assert(
                combat_presentation.model(state, combat_state).players[index].equipment_name
            )
        end)
        loadout.equipment_presentation_id = original
        assert(ok, swapped_name)
        local after_hash = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        t.is_true(swapped_name ~= before_name)
        t.eq(after_hash, before_hash)
    end)
end)
