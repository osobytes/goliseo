-- The impure half of the online lobby: it owns the star transport, the
-- clipboard, and the fixed-rate lobby clock, and it draws. Every decision it
-- makes is delegated to the pure screen in `game.screens.lobby`; this file only
-- translates input, executes effects, and feeds transport facts back in.

local lobby = require("game.screens.lobby")
local lobby_link = require("game.online.lobby_link")
local transport = require("game.transport")
local ui_draw = require("game.ui.draw")

---@class LobbyClipboard
---@field read fun(): string?
---@field write fun(text: string)

---@class OnlineLobbyOptions
---@field star_factory fun(role: LobbyRole, peer_id: string): StarTransportAdapter?
---@field clipboard LobbyClipboard?
---@field model_options LobbyModelOptions?

---@class OnlineLobby : Screen
---@field state LobbyScreenState
---@field link LobbyLink?
---@field on_action fun(action: table)?
---@field clipboard LobbyClipboard
---@field star_factory fun(role: LobbyRole, peer_id: string): StarTransportAdapter?
---@field transition number
---@field accumulator number
local OnlineLobby = {}
OnlineLobby.__index = OnlineLobby

OnlineLobby.TICK_SECONDS = 1 / 60

---@return LobbyClipboard
local function love_clipboard()
    return {
        read = function()
            if love.system and love.system.getClipboardText then
                return love.system.getClipboardText()
            end
            return nil
        end,
        write = function(text)
            if love.system and love.system.setClipboardText then
                love.system.setClipboardText(text)
            end
        end,
    }
end

-- Mirrors the browser star's own default hook. It is repeated here only because
-- the adapter's option record declares `eval` as required; the behaviour is
-- identical, and a native build simply fails to find the hook.
---@param command string
---@return string?, string?
local function love_js_eval(command)
    local love_api = rawget(_G, "love")
    local js = love_api and rawget(love_api, "js")
    local eval = js and rawget(js, "eval")
    if type(eval) ~= "function" then
        return nil, "love.js.eval is not available"
    end
    local ok, result = pcall(eval, command)
    if not ok then
        return nil, tostring(result)
    end
    return result == nil and "" or tostring(result)
end

-- Real signaling needs the browser bridge, so a native build reaches the lobby
-- with no transport and says so rather than pretending to connect.
---@param role LobbyRole
---@param peer_id string
---@return StarTransportAdapter?
local function default_star(role, peer_id)
    local _ = peer_id
    local star = transport.browser_star({ role = role, eval = love_js_eval })
    if not star:initialize() then
        return nil
    end
    return star
end

---@param viewport { w: number, h: number }
---@param on_action fun(action: table)?
---@param options OnlineLobbyOptions?
---@return OnlineLobby
function OnlineLobby.new(viewport, on_action, options)
    options = options or {}
    return setmetatable({
        state = lobby.new_state(viewport, { options = options.model_options }),
        link = nil,
        on_action = on_action,
        clipboard = options.clipboard or love_clipboard(),
        star_factory = options.star_factory or default_star,
        transition = 0,
        accumulator = 0,
    }, OnlineLobby)
end

---@param command table
function OnlineLobby:dispatch(command)
    local state, action = lobby.update(self.state, { kind = "lobby", command = command })
    self.state = state
    self:_run(state.effects)
    if action and self.on_action then
        self.on_action(action)
    end
end

---@param effects LobbyEffect[]
function OnlineLobby:_run(effects)
    for _, effect in ipairs(effects) do
        if effect.kind == "open_star" then
            local star = self.star_factory(effect.role, effect.peer_id)
            if star then
                self.link = lobby_link.new(star)
            end
        elseif effect.kind == "clipboard" then
            self.clipboard.write(effect.text)
        elseif effect.kind == "paste_request" then
            local text = self.clipboard.read()
            -- Straight back into the pure model, which keeps only a digest.
            self:dispatch({ kind = "paste", text = text or "" })
        elseif effect.kind == "shutdown" then
            if self.link then
                self.link:apply(effect)
                self.link = nil
            end
        elseif effect.kind ~= "leave" and effect.kind ~= "start_match" then
            if self.link then
                local ok, err = self.link:apply(effect)
                if not ok and err then
                    self:dispatch({ kind = "link_error", detail = err })
                end
            end
        end
    end
end

---@param dt number
function OnlineLobby:update(dt)
    local motion = require("game.ui.motion")
    self.transition = motion.advance(self.transition, dt)
    if self.link then
        for _, event in ipairs(self.link:poll()) do
            self:dispatch(event)
        end
    end
    self.accumulator = self.accumulator + dt
    while self.accumulator >= OnlineLobby.TICK_SECONDS do
        self.accumulator = self.accumulator - OnlineLobby.TICK_SECONDS
        self:dispatch({ kind = "tick" })
    end
end

---@param evt InputEvent
function OnlineLobby:event(evt)
    local state, action = lobby.update(self.state, evt)
    self.state = state
    self:_run(state.effects)
    if action and self.on_action then
        self.on_action(action)
    end
end

function OnlineLobby:draw()
    ui_draw.layout(lobby.layout(self.state), self.state.viewport, self.transition)
end

function OnlineLobby:teardown()
    if self.link then
        self.link:apply({ kind = "shutdown" })
        self.link = nil
    end
end

return OnlineLobby
