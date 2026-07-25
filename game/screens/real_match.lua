local contract = require("game.match_contract")
local match_observer = require("game.match_observer")
local Match = require("game.screens.match")
local teams = require("data.teams")

local FULL_TIME_HOLD = 0.9
local FULL_TIME_SKIP_DELAY = 0.25

---@class RealMatchScreen : Screen
---@field match MatchScreen
---@field request ProductMatchRequest
---@field callbacks MatchAdapterCallbacks
---@field observer MatchObserver
---@field completed boolean
---@field full_time_elapsed number
local RealMatch = {}
RealMatch.__index = RealMatch

---@param team TeamData
---@param roster string[]
---@return TeamData
local function with_roster(team, roster)
    return {
        id = team.id,
        name = team.name,
        color = team.color,
        formation = team.formation,
        roster = roster,
        squad = team.squad,
    }
end

---@param request ProductMatchRequest
---@param callbacks MatchAdapterCallbacks
---@return RealMatchScreen
function RealMatch.new(request, callbacks)
    local home = assert(teams[request.home_team_id], "unknown home team")
    local away = assert(teams[request.away_team_id], "unknown away team")
    local match = Match.new({
        formation = request.formation_id,
        tactic = request.tactic_id,
        home = with_roster(home, request.home_starter_ids),
        away = away,
        seed = request.seed,
        arena_id = request.arena_id,
        show_onboarding = request.show_onboarding,
        combat_enabled = request.combat_enabled,
        profile = "product",
    })
    return setmetatable({
        match = match,
        request = request,
        callbacks = callbacks,
        observer = match_observer.new(match.state),
        completed = false,
        full_time_elapsed = 0,
    }, RealMatch)
end

---@param self RealMatchScreen
local function complete(self)
    if self.completed then
        return
    end
    self.completed = true
    local observed = match_observer.finish(self.observer)
    local state = self.match.state
    local result = assert(contract.new_result({
        home_team_id = self.request.home_team_id,
        away_team_id = self.request.away_team_id,
        home_score = state.score.home,
        away_score = state.score.away,
        mvp_player_id = observed.mvp_player_id,
        mvp_summary = observed.mvp_summary,
        home_stats = observed.home_stats,
        away_stats = observed.away_stats,
        seed = self.request.seed,
    }))
    self.callbacks.on_finished(result)
end

---@param dt number
function RealMatch:update(dt)
    if self.match:full_time_confirmed() then
        local blocked = self.match:result_completion_blocked()
        self.match:update(dt)
        if blocked or self.match:result_completion_blocked() then
            return
        end
        self.full_time_elapsed = self.full_time_elapsed + dt
        if self.full_time_elapsed >= FULL_TIME_HOLD then
            complete(self)
        end
        return
    end
    local before = self.match.state.time_left
    self.match:update(dt)
    if self.match._rollback_lab then
        for _, step in ipairs(self.match._rollback_confirmed_steps) do
            match_observer.observe_confirmed(self.observer, step)
        end
    else
        local elapsed = before - self.match.state.time_left
        if elapsed > 0 then
            match_observer.observe(
                self.observer,
                self.match.state,
                elapsed,
                self.match._frame_events
            )
        end
    end
    if self.match:full_time_confirmed() then
        if self.match:result_completion_blocked() then
            return
        end
        self.full_time_elapsed = self.full_time_elapsed + dt
        if self.full_time_elapsed >= FULL_TIME_HOLD then
            complete(self)
        end
    end
end

---@param event InputEvent
function RealMatch:event(event)
    if self.match:full_time_confirmed() then
        if self.match:result_completion_blocked() then
            self.match:event(event)
            return
        end
        if
            self.full_time_elapsed >= FULL_TIME_SKIP_DELAY
            and event.kind == "action"
            and event.action == "confirm"
        then
            complete(self)
        end
        return
    end
    self.match:event(event)
end

function RealMatch:draw()
    self.match:draw()
end

function RealMatch:teardown()
    self.match:teardown()
end

return RealMatch
