local arenas = require("data.arenas")
local tactics = require("data.tactics")
local teams = require("data.teams")
local combat_feedback = require("game.presentation.combat_feedback")
local combat_feedback_fixture = require("game.presentation.combat_feedback_fixture")
local combat_presentation = require("game.presentation.combat")
local bloom = require("game.render.bloom")
local effects = require("game.render.effects")
local match_hud_render = require("game.render.match_hud")
local pitch = require("game.render.pitch")
local view_state = require("game.render.view_state")
local release_follow = require("game.render.release_follow")
local match_hud = require("game.match_hud")

---@class CombatFeedbackFixtureScreen
---@field fixture CrowdedCombatFeedbackFixture
---@field feedback CombatFeedbackState
---@field refresh_seconds number
---@field capture_path string?
---@field captured boolean
---@field exit_seconds number?
local Fixture = {}
Fixture.__index = Fixture

---@param self CombatFeedbackFixtureScreen
local function refresh(self)
    effects.reset()
    effects.apply_event_diff({
        added = self.fixture.events,
        revoked = {},
        replaced = {},
    })
    local confirmed = self.fixture.events[3]
    effects.confirm_event(confirmed)
    combat_feedback.confirm(
        self.feedback,
        combat_feedback.accepted(confirmed.payload, confirmed.id)
    )
    self.refresh_seconds = 0.48
end

---@param capture_path string?
---@return CombatFeedbackFixtureScreen
function Fixture.new(capture_path)
    local self = setmetatable({
        fixture = combat_feedback_fixture.new(),
        feedback = combat_feedback.new(),
        refresh_seconds = 0,
        capture_path = capture_path,
        captured = false,
        exit_seconds = nil,
    }, Fixture)
    view_state.reset()
    release_follow.reset()
    view_state.update(self.fixture.state.players, 0)
    refresh(self)
    return self
end

---@param dt number
function Fixture:update(dt)
    effects.tick(dt)
    combat_feedback.tick(self.feedback, dt)
    self.refresh_seconds = self.refresh_seconds - dt
    if self.refresh_seconds <= 0 then
        refresh(self)
    end
    if self.exit_seconds then
        self.exit_seconds = self.exit_seconds - dt
        if self.exit_seconds <= 0 then
            love.event.quit()
        end
    end
end

function Fixture:draw()
    local viewport = { w = love.graphics.getWidth(), h = love.graphics.getHeight() }
    local model = combat_presentation.model(self.fixture.state, self.fixture.combat)
    bloom.draw(function()
        pitch.draw(self.fixture.state, viewport, {
            home_color = teams.nebula.color,
            away_color = teams.orion.color,
            arena = arenas.helios_crown,
            arena_pulse = 0.35,
            combat = model,
            camera_offset = combat_feedback.camera_offset(self.feedback),
        })
        match_hud_render.draw(
            match_hud.model(self.fixture.state, {
                home_name = teams.nebula.name,
                away_name = teams.orion.name,
                arena_name = arenas.helios_crown.name,
                arena_location = arenas.helios_crown.location,
                tactic_name = tactics.balanced.name,
                formation_name = teams.nebula.formation,
                combat_enabled = true,
                combat = model.players[self.fixture.state.controlled],
                combat_notice = combat_feedback.notice(
                    self.feedback,
                    self.fixture.state.controlled
                ),
            }),
            viewport
        )
    end)
    love.graphics.push("all")
    love.graphics.setColor(0.02, 0.03, 0.08, 0.92)
    love.graphics.rectangle("fill", 18, viewport.h - 34, viewport.w - 36, 22, 4, 4)
    love.graphics.setColor(1, 1, 1, 1)
    love.graphics.printf(
        "CROWDED FEEDBACK FIXTURE · VERIFY BALL, TEAM, SELECTION, THREAT GLYPHS · ESC TO EXIT",
        24,
        viewport.h - 30,
        viewport.w - 48,
        "center"
    )
    love.graphics.pop()
    if self.capture_path and not self.captured then
        love.graphics.captureScreenshot(self.capture_path)
        self.captured = true
        self.exit_seconds = 0.25
    end
end

return Fixture
