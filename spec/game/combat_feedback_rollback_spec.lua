local audio = require("game.audio")
local combat_feedback = require("game.presentation.combat_feedback")
local effects = require("game.render.effects")
local replay = require("game.render.replay")
local Match = require("game.screens.match")
local fixture = require("spec.fixtures.crowded_combat_feedback")
local t = require("spec.support.runner")

---@param event RollbackWrappedCombatEvent
---@return RollbackEventStep
local function step(event)
    return {
        tick = event.tick,
        start_boundary = event.tick,
        end_boundary = event.tick + 1,
        state = {
            score = { home = 0, away = 0 },
            time_left = 100,
            finished = false,
            owner_id = nil,
            owner_team = nil,
        },
        match_events = {},
        combat_events = { event },
        lifecycle_events = {},
    }
end

---@param tick integer
---@param combat_events RollbackWrappedCombatEvent[]
---@param lifecycle_events RollbackWrappedLifecycleEvent[]
---@return RollbackEventStep
local function confirmed_step(tick, combat_events, lifecycle_events)
    return {
        tick = tick,
        start_boundary = tick,
        end_boundary = tick + 1,
        state = {
            score = { home = 0, away = 0 },
            time_left = 100,
            finished = false,
            owner_id = nil,
            owner_team = nil,
        },
        match_events = {},
        combat_events = combat_events,
        lifecycle_events = lifecycle_events,
    }
end

---@param id string
---@param tick integer
---@return RollbackWrappedLifecycleEvent
local function goal(id, tick)
    return {
        id = id,
        tick = tick,
        domain = "lifecycle/goal",
        ordinal = 1,
        payload = {
            kind = "goal",
            team = "home",
            score = { home = 1, away = 0 },
        },
    }
end

---@return MatchScreen
local function screen()
    return Match.new({
        rollback_lab = {
            profile_name = "clean",
            duration = 2,
        },
    })
end

t.describe("combat feedback rollback presentation", function()
    t.it("revokes corrected-away effects and publishes the survivor exactly once", function()
        effects.reset()
        audio.reset()
        local value = screen()
        local predicted = fixture.events[3]
        local corrected = fixture.events[2]

        t.is_true(value:consume_rollback_event_diff({
            added = { predicted },
            revoked = {},
            replaced = {},
        }))
        t.is_true(effects.diagnostics().particle_count > 0)
        t.is_true(value:consume_rollback_event_diff({
            added = {},
            revoked = {},
            replaced = { { before = predicted, after = corrected } },
        }))
        local corrected_count = effects.diagnostics().particle_count
        t.is_true(corrected_count > 0)

        t.eq(value:consume_confirmed_step(step(corrected)), 1)
        local confirmed_count = effects.diagnostics().particle_count
        t.eq(value:consume_confirmed_step(step(corrected)), 0)
        t.eq(effects.diagnostics().particle_count, confirmed_count)
        t.eq(audio.confirmed_cue_counts().combat_guarded, 1)
        t.eq(combat_feedback.diagnostics(value._combat_feedback).confirmed_count, 1)
        t.is_true(
            audio.confirmed_cue_counts().combat_hit == nil,
            "corrected-away result cannot publish audio"
        )
    end)

    t.it("suppresses live combat feedback while confirmed replay owns the screen", function()
        effects.reset()
        audio.reset()
        replay.reset()
        local value = screen()
        for boundary = 1, 40 do
            replay.record_boundary(boundary, value.state)
        end
        t.is_true(value:consume_confirmed_lifecycle(goal("fixture/goal", 40)))
        t.is_true(replay.active())

        local live = fixture.events[3]
        t.is_true(not value:consume_rollback_event_diff({
            added = { live },
            revoked = {},
            replaced = {},
        }))
        t.eq(effects.diagnostics().particle_count, 0)
        t.eq(value:consume_confirmed_step(step(live)), 1)
        t.eq(effects.diagnostics().particle_count, 0)
        t.is_true(combat_feedback.diagnostics(value._combat_feedback).notice_id == nil)
        t.is_true(combat_feedback.diagnostics(value._combat_feedback).impulse_id == nil)
        t.is_true(audio.confirmed_cue_counts().combat_hit == nil)
        t.eq(audio.suppressed_confirmed_cue_counts().combat_hit, 1)
    end)

    t.it("preserves speculative identity while replay clears live visuals", function()
        effects.reset()
        audio.reset()
        replay.reset()
        local value = screen()
        for boundary = 1, 40 do
            replay.record_boundary(boundary, value.state)
        end
        local live = fixture.events[3]
        t.is_true(value:consume_rollback_event_diff({
            added = { live },
            revoked = {},
            replaced = {},
        }))
        t.is_true(effects.diagnostics().particle_count > 0)
        t.eq(effects.diagnostics().speculative_ids[1], live.id)

        t.is_true(value:consume_confirmed_lifecycle(goal("ledger/goal", 40)))
        local entered = effects.diagnostics()
        t.is_true(replay.active())
        t.eq(entered.particle_count, 0)
        t.eq(entered.speculative_ids[1], live.id, "replay entry preserves the identity ledger")

        t.eq(value:consume_confirmed_step(step(live)), 1)
        local confirmed = effects.diagnostics()
        t.eq(confirmed.particle_count, 0)
        t.eq(#confirmed.speculative_ids, 0)
        t.eq(confirmed.confirmed_ids[1], live.id)
        t.eq(value:consume_confirmed_step(step(live)), 0)
        t.eq(effects.diagnostics().particle_count, 0, "duplicate confirmation stays suppressed")
    end)

    t.it("lets lifecycle replay ownership suppress same-batch combat feedback", function()
        effects.reset()
        audio.reset()
        replay.reset()
        local value = screen()
        for boundary = 1, 40 do
            replay.record_boundary(boundary, value.state)
        end
        local live = fixture.events[3]
        local batch = confirmed_step(40, { live }, { goal("same-batch/goal", 40) })

        t.eq(value:consume_confirmed_step(batch), 2)
        t.is_true(replay.active())
        t.eq(effects.diagnostics().particle_count, 0)
        t.is_true(combat_feedback.notice(value._combat_feedback, value.state.controlled) == nil)
        t.is_true(audio.confirmed_cue_counts().combat_hit == nil)
        t.eq(audio.suppressed_confirmed_cue_counts().combat_hit, 1)
        t.eq(audio.confirmed_cue_counts().goal, 1)
    end)

    t.it("retains a controlled-player notice behind unrelated same-step feedback", function()
        effects.reset()
        audio.reset()
        replay.reset()
        local value = screen()
        local controlled = fixture.events[1]
        local unrelated = fixture.events[3]
        t.eq(
            value:consume_confirmed_step(
                confirmed_step(controlled.tick, { controlled, unrelated }, {})
            ),
            2
        )

        local notice =
            assert(combat_feedback.notice(value._combat_feedback, value.state.controlled))
        t.eq(notice.id, controlled.id)
        t.eq(notice.text, "EQUIPMENT COMMITTED")
        t.eq(combat_feedback.diagnostics(value._combat_feedback).notice_count, 2)
    end)
end)
