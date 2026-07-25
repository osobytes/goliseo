local effects = require("game.render.effects")
local camera = require("game.render.camera")
local match_hud = require("game.match_hud")
local settings = require("game.settings")
local fixture = require("spec.fixtures.crowded_combat_feedback")
local t = require("spec.support.runner")

---@param event RollbackWrappedCombatEvent
---@param result CombatContactResult
---@return RollbackWrappedCombatEvent
local function replacement(event, result)
    local payload = {}
    for key, value in pairs(event.payload) do
        payload[key] = value
    end
    payload.result = result
    return {
        id = event.id .. "/" .. result,
        tick = event.tick,
        domain = event.domain,
        ordinal = event.ordinal,
        payload = payload,
    }
end

t.describe("combat feedback effects", function()
    t.it("reconciles crowded add, replace, revoke, and confirm by stable ID", function()
        effects.configure(settings.defaults())
        effects.reset()
        effects.apply_event_diff({
            added = fixture.events,
            revoked = {},
            replaced = {},
        })
        local added = effects.diagnostics()
        t.eq(#added.speculative_ids, #fixture.events)
        local particle_count = added.particle_count
        t.is_true(particle_count > #fixture.events)

        effects.apply_event_diff({
            added = fixture.events,
            revoked = {},
            replaced = {},
        })
        t.eq(effects.diagnostics().particle_count, particle_count, "duplicate add is idempotent")

        local guarded = fixture.events[2]
        local immune = replacement(guarded, "immune")
        effects.apply_event_diff({
            added = {},
            revoked = {},
            replaced = { { before = guarded, after = immune } },
        })
        local replaced = effects.diagnostics()
        t.eq(#replaced.speculative_ids, #fixture.events)
        t.is_true(replaced.particle_count < particle_count)

        t.is_true(effects.confirm_event(immune))
        local confirmed_count = effects.diagnostics().particle_count
        t.is_true(not effects.confirm_event(immune))
        t.eq(effects.diagnostics().particle_count, confirmed_count)

        effects.apply_event_diff({
            added = {},
            revoked = { fixture.events[1], fixture.events[3], fixture.events[4], fixture.events[5] },
            replaced = {},
        })
        t.eq(#effects.diagnostics().speculative_ids, 0)
    end)

    t.it("reports crowded geometry without masking the ball or HUD", function()
        effects.configure(settings.defaults())
        effects.reset()
        effects.apply_event_diff({
            added = fixture.events,
            revoked = {},
            replaced = {},
        })
        local observation = effects.readability_observation(function(x, y)
            return x, y, 1
        end, fixture.ball, fixture.hud_rects, fixture.occluders)
        t.eq(observation.concurrent_event_count, #fixture.events)
        t.eq(observation.active_feedback_count, #fixture.events)
        t.is_true(observation.overlap_pair_count > 0)
        t.is_true(not observation.ball_masked)
        t.is_true(not observation.hud_masked)
        t.eq(observation.occluded_count, 0)
        t.is_true(observation.non_color_only)
    end)

    t.it("preserves ball and HUD clearance at every supported fixture size", function()
        effects.configure(settings.defaults())
        effects.reset()
        effects.apply_event_diff({
            added = fixture.events,
            revoked = {},
            replaced = {},
        })
        for _, viewport in ipairs({
            { w = 960, h = 540 },
            { w = 1280, h = 720 },
            { w = 1920, h = 1080 },
            { w = 1280, h = 800 },
        }) do
            local layout = match_hud.layout(viewport)
            local observation = effects.readability_observation(
                function(x, y)
                    return camera.project(x, y, fixture.state.field, viewport)
                end,
                fixture.ball,
                {
                    layout.combat,
                    layout.plan,
                    layout.identity,
                    layout.scorebug,
                },
                {}
            )
            t.is_true(not observation.ball_masked, "ball masked at " .. viewport.w)
            t.is_true(not observation.hud_masked, "HUD masked at " .. viewport.w)
            t.is_true(observation.non_color_only)
        end
    end)

    t.it("reduces flash density and particle travel through existing settings", function()
        local reduced = settings.defaults()
        reduced.screen_shake = false
        reduced.bloom = false
        effects.configure(reduced)
        effects.reset()
        effects.apply_event_diff({
            added = { fixture.events[3] },
            revoked = {},
            replaced = {},
        })
        t.is_true(effects.diagnostics().particle_count <= 4)
        local observation = effects.readability_observation(function(x, y)
            return x, y, 1
        end, fixture.ball, {}, {})
        t.eq(observation.active_feedback_count, 1)
        t.is_true(observation.non_color_only)
        effects.configure(settings.defaults())
    end)
end)
