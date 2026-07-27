-- The impure half of the online match: it owns the OMP-3 driver, borrows the
-- lobby's star link, drives the shipped combat match renderer through the
-- renderer's own drive seam, and draws one extra online overlay.
--
-- Every decision it makes is delegated: the driver owns simulation and authority,
-- `game.online.match_presentation` owns the stable event timeline, and
-- `game.screens.online_match_model` owns session policy. This file translates
-- input, executes effects, and reports facts.
--
-- # What the local human can and cannot do
--
-- One `InputSample` per fixed tick goes into `match_driver.advance`, which puts
-- it on this peer's *control slot* and nowhere else. There is no path from local
-- input to a slot outside the frozen owned set, to another peer's slot, or to a
-- keeper: the driver validates authorship as set membership and keepers are
-- slotless in every mode. Switching inside the owned set is the ordinary
-- `switch` bit, evaluated by every peer from the canonical input stream — see
-- `game.online.match_session` for why that stays mode-agnostic.

local lobby_link = require("game.online.lobby_link")
local match_driver = require("game.online.match_driver")
local match_presentation = require("game.online.match_presentation")
local match_session = require("game.online.match_session")
local online_match_model = require("game.screens.online_match_model")
local Match = require("game.screens.match")
local contract = require("game.match_contract")
local match_observer = require("game.match_observer")
local combat_presentation = require("game.presentation.combat")
local theme = require("game.ui.theme")
local match_snapshot = require("sim.match_snapshot")

---@class OnlineMatchOptions
---@field request OnlineMatchRequest
---@field coordinator CoordinatorState
---@field link LobbyLink -- The lobby's framed control link; its star carries both channels.
---@field on_action fun(action: table)?

---@class OnlineMatchScreen : Screen
---@field request OnlineMatchRequest
---@field driver MatchDriver
---@field presentation OnlineMatchPresentation
---@field model OnlineMatchModel
---@field match MatchScreen
---@field link LobbyLink
---@field observer MatchObserver
---@field on_action fun(action: table)?
---@field live table<string, InputSlotId>
---@field _buffers table<string, LobbyFrameBuffer>
---@field _control TransportPeerMessage[]
---@field _checkpoints MatchDriverCheckpoint[]
---@field _confirmed RollbackEventStep[]
---@field _accumulator number
---@field _reported_terminal boolean
---@field _routed boolean
local OnlineMatch = {}
OnlineMatch.__index = OnlineMatch

OnlineMatch.TICK_SECONDS = 1 / 60

---@param self OnlineMatchScreen
---@return MatchRollbackSource
local function driver_source(self)
    ---@type MatchRollbackSource
    return {
        needs_local_sample = function()
            return match_driver.status(self.driver) == "active"
        end,
        advance = function(_, _, sample)
            local batch = match_driver.advance(self.driver, sample)
            for _, entry in ipairs(batch.control) do
                self._control[#self._control + 1] = entry
            end
            for _, checkpoint in ipairs(batch.checkpoints) do
                self._checkpoints[#self._checkpoints + 1] = checkpoint
            end
            self.live = batch.live
            local presented = match_presentation.consume(self.presentation, self.driver, batch)
            for _, step in ipairs(presented.confirmed_steps) do
                self._confirmed[#self._confirmed + 1] = step
            end
            return presented
        end,
        current_snapshot = function()
            return match_driver.current_snapshot(self.driver)
        end,
        snapshot = function(_, boundary)
            return match_driver.snapshot(self.driver, boundary + self.request.first_input_tick)
        end,
        terminal = function()
            return match_driver.status(self.driver) ~= "active"
        end,
        failed = function()
            local status = match_driver.status(self.driver)
            return status ~= "active" and status ~= "completed"
        end,
        full_time = function()
            return match_driver.status(self.driver) == "completed"
        end,
        debug_model = function()
            -- The online overlay below is this screen's own, drawn from real
            -- driver diagnostics. Borrowing the laboratory's debug model would
            -- mean inventing a dozen numbers the driver does not measure.
            return nil
        end,
        controlled_player = function(_, state)
            local live = self.live[self.request.peer_id] or self.request.live
            if live == nil then
                return nil
            end
            return match_session.player_index(state, live)
        end,
    }
end

---@param options OnlineMatchOptions
---@return OnlineMatchScreen
function OnlineMatch.new(options)
    local request = assert(options.request, "an online match needs its request")
    local link = assert(options.link, "an online match needs the session link")
    local self = setmetatable({
        request = request,
        link = link,
        on_action = options.on_action,
        live = { [request.peer_id] = request.live },
        _buffers = {},
        _control = {},
        _checkpoints = {},
        _confirmed = {},
        _accumulator = 0,
        _reported_terminal = false,
        _routed = false,
    }, OnlineMatch)
    self.driver = match_driver.new({
        role = request.role,
        peer_id = request.peer_id,
        freeze = request.freeze,
        manifest = request.manifest,
        transport = link.star,
        initial_snapshot = request.initial_snapshot,
    })
    self.presentation = match_presentation.new(request.initial_snapshot, request.first_input_tick)
    self.model = online_match_model.new(request, assert(options.coordinator))
    self.match = Match.new({
        home = request.home,
        away = request.away,
        arena_id = request.arena.id,
        seed = request.seed,
        combat_enabled = request.combat_enabled,
        profile = "online",
        initial_snapshot = request.initial_snapshot,
        rollback_source = function()
            return driver_source(self)
        end,
    })
    self.observer = match_observer.new(self.match.state)
    return self
end

---@param event table
function OnlineMatch:dispatch(event)
    local model, effects = online_match_model.command(self.model, event)
    self.model = model
    for _, effect in ipairs(effects) do
        if effect.kind == "send" then
            self.link:send(effect.link_id, effect.wire)
        elseif effect.kind == "close" then
            self.link:apply({ kind = "close", link_id = effect.link_id })
        elseif effect.kind == "observe_hash" then
            match_driver.observe_checkpoint(self.driver, effect.tick, effect.hash)
        end
    end
end

-- Control traffic shares the transport with input. The driver keeps the input
-- envelopes and hands everything else back untouched, so the session's own mail
-- is reassembled here through the same framing the lobby used.
--
-- Once the driver is terminal it stops polling entirely — that is its "no hidden
-- progress" contract — but the session is not finished: full time is exactly
-- when the result acknowledgement crosses the wire. So the link is pumped
-- directly from that point on, which is also how a link lost after the final
-- tick still reaches the coordinator.
function OnlineMatch:_drain_control()
    if match_driver.status(self.driver) ~= "active" then
        -- Straight off the star rather than through `LobbyLink:poll`, so both
        -- phases reassemble into the same buffers: a control wire whose frames
        -- straddle the final tick must not land in two different buffer sets.
        local star = self.link.star
        for _, entry in ipairs(star:poll_batch()) do
            if entry.channel == "control" then
                self._control[#self._control + 1] = entry
            end
        end
        local event = star:poll_event()
        while event do
            if
                event.kind == "peer_state"
                and event.peer_id
                and (
                    event.state == "closed"
                    or event.state == "error"
                    or event.state == "disconnected"
                )
            then
                self:dispatch({ kind = "link_lost", link_id = event.peer_id })
            end
            event = star:poll_event()
        end
    end
    local entries = self._control
    self._control = {}
    for _, entry in ipairs(entries) do
        if entry.channel == "control" then
            local buffer = self._buffers[entry.peer_id] or lobby_link.new_buffer()
            self._buffers[entry.peer_id] = buffer
            local wire, err = lobby_link.absorb(buffer, entry.message.payload)
            if wire then
                self:dispatch({ kind = "control", link_id = entry.peer_id, wire = wire })
            elseif err then
                self._buffers[entry.peer_id] = lobby_link.new_buffer()
                self.model.error = err
            end
        end
    end
end

function OnlineMatch:_drain_simulation()
    local checkpoints = self._checkpoints
    self._checkpoints = {}
    if #checkpoints > 0 then
        self:dispatch({ kind = "checkpoints", checkpoints = checkpoints })
    end
    local confirmed = self._confirmed
    self._confirmed = {}
    if #confirmed > 0 then
        for _, step in ipairs(confirmed) do
            match_observer.observe_confirmed(self.observer, step)
        end
        self:dispatch({ kind = "confirmed", steps = confirmed })
    end
end

function OnlineMatch:_report_driver()
    if self._reported_terminal then
        return
    end
    local terminal = match_driver.terminal(self.driver)
    if terminal == nil then
        return
    end
    self._reported_terminal = true
    if terminal.status == "completed" then
        local state = self.match.state
        self:dispatch({
            kind = "full_time",
            final_tick = state.input_tick + self.request.first_input_tick,
            home_score = state.score.home,
            away_score = state.score.away,
            final_hash = match_snapshot.hash(match_driver.current_snapshot(self.driver)),
        })
        return
    end
    self:dispatch({
        kind = "failure",
        status = terminal.status,
        failure = terminal.failure,
        detail = terminal.detail,
    })
end

---@return ProductMatchResult?
function OnlineMatch:_result()
    local result = self.model.result
    if result == nil then
        return nil
    end
    local observed = match_observer.finish(self.observer)
    -- The score is the coordinator's acknowledged result, never this peer's
    -- prediction. Statistics stay local: they are presentation, and OMP-3 does
    -- not put them on the wire.
    return (
        contract.new_result({
            home_team_id = self.request.home.id,
            away_team_id = self.request.away.id,
            home_score = result.home_score,
            away_score = result.away_score,
            mvp_player_id = observed.mvp_player_id,
            mvp_summary = observed.mvp_summary,
            home_stats = observed.home_stats,
            away_stats = observed.away_stats,
            seed = self.request.seed,
        })
    )
end

function OnlineMatch:_route()
    if self._routed or not online_match_model.ended(self.model) then
        return
    end
    -- A confirmed goal replay owns the screen until it finishes; the session has
    -- already ended underneath, so nothing is lost by letting it play out.
    if self.match:result_completion_blocked() then
        return
    end
    self._routed = true
    if not self.on_action then
        return
    end
    if online_match_model.exit_route(self.model) == "result" then
        local result = self:_result()
        if result then
            self.on_action({ go = "online_result", result = result })
            return
        end
    end
    self.on_action({
        go = "online_ended",
        terminal = self.model.terminal,
        detail = self.model.terminal and self.model.terminal.detail or self.model.error,
    })
end

---@param dt number
function OnlineMatch:update(dt)
    -- The match screen owns the fixed clock and calls back into the driver
    -- source. Local focus, pause requests, and a lost controller deliberately do
    -- not reach this call: an online peer that stops advancing stalls every peer.
    self.match:update(dt)
    self:_drain_control()
    self:_drain_simulation()
    self:_report_driver()
    self._accumulator = self._accumulator + dt
    while self._accumulator >= OnlineMatch.TICK_SECONDS do
        self._accumulator = self._accumulator - OnlineMatch.TICK_SECONDS
        self:dispatch({ kind = "tick", dt = OnlineMatch.TICK_SECONDS })
    end
    self:_route()
end

---@param event InputEvent
function OnlineMatch:event(event)
    if event.kind == "action" and (event.action == "pause" or event.action == "back") then
        self:dispatch({ kind = "pause_request" })
        return
    end
    if self.model.abort_prompt then
        self:dispatch({ kind = "resume_request" })
    end
    self.match:event(event)
end

function OnlineMatch:focus_lost()
    self:dispatch({ kind = "focus_lost" })
end

function OnlineMatch:controller_lost()
    self:dispatch({ kind = "controller_lost" })
end

---@param settings GameSettings
function OnlineMatch:apply_settings(settings)
    self.match:apply_settings(settings)
end

function OnlineMatch:teardown()
    self.match:teardown()
end

-- The online status panel. It never hides the ball or a combat telegraph: it is
-- a corner strip, drawn after the match, over the HUD's own margin.
---@return string[]
function OnlineMatch:overlay_lines()
    local diagnostics = match_driver.diagnostics(self.driver)
    local state = self.match.state
    local live = self.live[self.request.peer_id] or self.request.live
    local combat = combat_presentation.model(state, self.match._combat_state)
    local controlled = combat.players[state.controlled]
    local player = state.players[state.controlled]
    local lines = {
        ("%s  %s  %s"):format(
            self.request.mode:upper(),
            self.request.role:upper(),
            self.request.peer_id
        ),
        ("control %s  owned %s"):format(tostring(live), table.concat(self.request.owned, ",")),
        ("player %s  family %s"):format(
            player and player.id or "-",
            controlled and (controlled.family_name or controlled.family_id or "none") or "none"
        ),
        ("equipment %s  %s"):format(
            controlled and (controlled.equipment_name or "none") or "none",
            controlled and controlled.readiness or "unavailable"
        ),
        ("possession %s"):format(state.owner and state.players[state.owner].team or "loose"),
        ("net tick %d  present %d  confirmed %d"):format(
            diagnostics.transport_tick,
            diagnostics.present_input_tick,
            diagnostics.confirmed_output_tick
        ),
        ("rollbacks %d  corrections %d  predicted %d"):format(
            diagnostics.rollback_count,
            diagnostics.correction_count,
            diagnostics.predicted_slot_samples
        ),
        ("session %s  driver %s"):format(self.model.coordinator.phase, diagnostics.status),
    }
    if self.model.phase == "full_time" then
        lines[#lines + 1] = "full time — waiting for every peer to acknowledge the result"
    end
    local terminal = self.model.terminal
    if terminal then
        lines[#lines + 1] = ("session ended: %s"):format(terminal.reason)
    end
    if self.model.error then
        lines[#lines + 1] = self.model.error
    end
    if self.model.abort_prompt then
        lines[#lines + 1] = online_match_model.ABORT_PROMPT
    elseif self.model.notice then
        lines[#lines + 1] = self.model.notice.text
    end
    return lines
end

function OnlineMatch:draw()
    self.match:draw()
    local lines = self:overlay_lines()
    local width = math.min(420, love.graphics.getWidth() - 24)
    local height = #lines * 16 + 16
    love.graphics.push("all")
    love.graphics.setColor(0.015, 0.025, 0.06, 0.85)
    love.graphics.rectangle("fill", 12, 12, width, height, 6, 6)
    love.graphics.setColor(theme.colors.cyan)
    for index, line in ipairs(lines) do
        love.graphics.print(line, 22, 16 + index * 16)
    end
    love.graphics.pop()
end

return OnlineMatch
