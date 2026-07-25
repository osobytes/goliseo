local hud = require("game.match_hud")
local combat_feedback = require("game.presentation.combat_feedback")
local combat_presentation = require("game.presentation.combat")
local match_sim = require("sim.match")
local t = require("spec.support.runner")
local teams = require("data.teams")
local crowded_fixture = require("spec.fixtures.crowded_combat_feedback")

---@return MatchHudContext
local function context()
    return {
        home_name = "Nebula FC",
        away_name = "Orion Miners",
        arena_name = "Helios Crown",
        arena_location = "Kairon-9 Orbit",
        tactic_name = "Press High",
        formation_name = "1-2-1",
    }
end

---@return MatchState
local function state()
    return match_sim.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
    })
end

t.describe("match HUD model", function()
    t.it("formats time, identity, plan, possession, and clamped stamina", function()
        local value = state()
        value.time_left = 5.9
        value.players[value.controlled].sprint_meter = 1.4
        local model = hud.model(value, context())
        t.eq(model.clock, "0:05")
        t.eq(model.possession, "NEBULA FC POSSESSION")
        t.eq(model.possession_marker, "filled")
        t.is_true(model.player_name ~= "")
        t.is_true(model.player_detail:match("TERRAN") ~= nil)
        t.eq(model.plan, "PLAN · PRESS HIGH · 1-2-1")
        t.eq(model.stamina, 1)

        value.owner = nil
        model = hud.model(value, context())
        t.eq(model.possession, "LOOSE BALL")
        t.eq(model.possession_marker, "outline")

        value.owner = 6
        model = hud.model(value, context())
        t.eq(model.possession, "ORION MINERS POSSESSION")
        t.eq(model.player_state, "DEFENDING")
    end)

    t.it("authors kickoff, goal, replay, and full-time broadcast phases", function()
        local value = state()
        for _, phase in ipairs({ "kickoff", "goal", "replay", "full_time" }) do
            local ctx = context()
            ctx.phase = phase
            ctx.scoring_team = "home"
            local model = hud.model(value, ctx)
            t.is_true(model.announcement_title ~= nil, phase .. " needs a title")
            t.is_true(model.announcement_detail ~= nil, phase .. " needs supporting detail")
        end
    end)

    t.it("keeps every HUD region inside supported viewports", function()
        for _, size in ipairs({ { 960, 540 }, { 1280, 720 }, { 1920, 1080 }, { 1280, 800 } }) do
            local layout = hud.layout({ w = size[1], h = size[2] })
            for id, rect in pairs(layout) do
                if type(rect) == "table" then
                    t.is_true(rect.x >= 0 and rect.y >= 0, id .. " begins outside the viewport")
                    t.is_true(rect.x + rect.w <= size[1], id .. " exceeds viewport width")
                    t.is_true(rect.y + rect.h <= size[2], id .. " exceeds viewport height")
                end
            end
        end
    end)

    t.it("shows geometry-labeled equipment readiness and confirmed result feedback", function()
        local value = crowded_fixture.state
        value.controlled = 2
        local combat_model = combat_presentation.model(value, crowded_fixture.combat)
        local feedback_state = combat_feedback.new()
        local confirmed = crowded_fixture.events[1]
        combat_feedback.confirm(
            feedback_state,
            combat_feedback.accepted(confirmed.payload, confirmed.id)
        )
        local ctx = context()
        ctx.combat_enabled = true
        ctx.combat = combat_model.players[value.controlled]
        ctx.combat_notice = combat_feedback.notice(feedback_state, value.controlled)
        local model = hud.model(value, ctx)
        t.is_true(model.equipment_label ~= nil)
        t.is_true(model.equipment_state ~= nil)
        t.eq(model.feedback_text, "EQUIPMENT COMMITTED")
        t.eq(model.feedback_glyph, "chevron")
        t.is_true(model.equipment_progress >= 0 and model.equipment_progress <= 1)
    end)
end)
