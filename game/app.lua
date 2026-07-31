local controller = require("game.input.controller")
local match_adapter = require("game.match_adapter")
local match_manifest = require("game.online.match_manifest")
local match_session = require("game.online.match_session")
local session_model = require("game.session")
local settings_model = require("game.settings")
local viewport_model = require("game.ui.viewport")
local Credits = require("game.screens.credits")
local Formation = require("game.screens.formation")
local Help = require("game.screens.help")
local Menu = require("game.screens.menu")
local OnlineLobby = require("game.screens.online_lobby")
local OnlineMatch = require("game.screens.online_match")
local Pause = require("game.screens.pause")
local Result = require("game.screens.result")
local Settings = require("game.screens.settings")
local Squad = require("game.screens.squad")
local Tactic = require("game.screens.tactic")
local Title = require("game.screens.title")
local ScreenStack = require("game.screen_stack")

---@class AppOptions
---@field actual_w number?
---@field actual_h number?
---@field settings GameSettings?
---@field settings_storage SettingsStorage?
---@field match_adapter MatchAdapter?
---@field apply_settings fun(settings: GameSettings)?
---@field request_quit fun()?
---@field quick_match boolean?  -- playtest: boot straight into a match

---@class App
---@field stack ScreenStack
---@field session GameSession
---@field settings GameSettings
---@field settings_storage SettingsStorage?
---@field viewport { w: number, h: number }
---@field transform ViewportTransform
---@field adapter MatchAdapter
---@field routes string[]
---@field quit_requested boolean
---@field apply_settings fun(settings: GameSettings)?
---@field request_quit fun()?
---@field online_error string? -- Why the last online session ended, if it did not complete.
local App = {}
App.__index = App

---@param opts AppOptions?
---@return App
function App.new(opts)
    opts = opts or {}
    local self = setmetatable({}, App)
    self.stack = ScreenStack.new()
    self.session = session_model.new()
    self.settings_storage = opts.settings_storage
    self.settings = opts.settings and settings_model.validate(opts.settings)
        or settings_model.load(opts.settings_storage)
    self.viewport = { w = 960, h = 540 }
    self.transform = viewport_model.new(opts.actual_w or 960, opts.actual_h or 540)
    self.adapter = opts.match_adapter or match_adapter.fake()
    self.apply_settings = opts.apply_settings
    self.request_quit = opts.request_quit
    self.routes = {}
    self.quit_requested = false
    self.online_error = nil
    -- Playtest convenience: boot straight into a match. Menus are not what
    -- anyone is evaluating when they are looking at the renderer.
    if opts and opts.quick_match then
        self:start_match()
    else
        self:show_title()
    end
    return self
end

---@param route string
---@param screen Screen
function App:_replace(route, screen)
    self.stack:clear()
    self.stack:push(screen)
    self.routes = { route }
end

---@param route string
---@param screen Screen
function App:_push(route, screen)
    self.stack:push(screen)
    self.routes[#self.routes + 1] = route
end

function App:_pop()
    if #self.routes > 1 then
        self.stack:pop()
        table.remove(self.routes)
    end
end

---@return string
function App:current_route()
    return self.routes[#self.routes]
end

---@param def any
---@param context any?
---@return Menu
function App:_menu(def, context)
    return Menu.new(def, self.viewport, function(action)
        self:handle_action(action)
    end, context)
end

function App:show_title()
    self:_replace("title", self:_menu(Title))
end

function App:show_squad()
    self.session.setup_step = "squad"
    self:_replace("squad", self:_menu(Squad, { starter_ids = self.session.starter_ids }))
end

function App:show_formation()
    self.session.setup_step = "formation"
    self:_replace(
        "formation",
        self:_menu(Formation, {
            selected = self.session.formation_id,
            starter_ids = self.session.starter_ids,
        })
    )
end

function App:show_tactic()
    self.session.setup_step = "tactic"
    self:_replace(
        "tactic",
        self:_menu(Tactic, {
            selected = self.session.tactic_id,
            formation_id = self.session.formation_id,
        })
    )
end

function App:show_result()
    assert(self.session.last_result, "result route needs a match result")
    self:_replace("result", self:_menu(Result, { result = self.session.last_result }))
end

-- The online result screen is the same product screen, on its own route so the
-- offline rematch (which replays the local session) cannot be reached from a
-- session that no longer exists.
---@param result ProductMatchResult
function App:show_online_result(result)
    self.online_error = nil
    self:_replace("online_result", self:_menu(Result, { result = result }))
end

function App:start_match()
    local request = assert(session_model.build_request(self.session, self.session.match_number + 1))
    local screen = self.adapter.new(request, {
        on_finished = function(result)
            session_model.record_result(self.session, result)
            self:show_result()
        end,
        on_cancelled = function()
            self:show_tactic()
        end,
    }, self.viewport)
    if screen.apply_settings then
        screen:apply_settings(self.settings)
    end
    self:_replace("match", screen)
end

-- The developer online route. It sits beside the showcase and combat-prototype
-- paths rather than replacing either, and it is only reachable on request.
--
-- The lobby proposes this build's *content-derived* manifest rather than the
-- protocol fixture, because the frozen manifest has to be enough to rebuild
-- boundary zero on every peer. A caller may still inject its own template; only
-- the default changes here.
---@param options OnlineLobbyOptions?
function App:show_lobby(options)
    options = options or {}
    local model_options = options.model_options or {}
    ---@type OnlineLobbyOptions
    local resolved = {
        star_factory = options.star_factory,
        clipboard = options.clipboard,
        model_options = {
            peer_id = model_options.peer_id,
            session_id = model_options.session_id,
            seed = model_options.seed,
            template = model_options.template or match_manifest.template,
        },
    }
    self:_push(
        "lobby",
        OnlineLobby.new(self.viewport, function(action)
            self:handle_action(action)
        end, resolved)
    )
end

-- Route the lobby's synchronized start into the real online match. The lobby
-- keeps its link: the match borrows the same star, because the session's control
-- channel and the match's input channel are the same transport.
---@param freeze CoordinatorFreeze
function App:start_online_match(freeze)
    local lobby = self.stack:current()
    ---@cast lobby OnlineLobby
    local model = lobby.state.model
    local state = model.coordinator
    local link = lobby.link
    if not state or not state.manifest or not link then
        self.online_error = "the lobby has no frozen session to play"
        return
    end
    local request, err = match_session.request({
        role = state.role,
        peer_id = state.peer_id,
        manifest = state.manifest,
        freeze = freeze,
    })
    if not request then
        self.online_error = err
        return
    end
    self.online_error = nil
    local screen = OnlineMatch.new({
        request = request,
        coordinator = state,
        link = link,
        on_action = function(action)
            self:handle_action(action)
        end,
    })
    screen:apply_settings(self.settings)
    self:_push("online_match", screen)
end

function App:show_pause()
    self:_push("pause", self:_menu(Pause))
end

---@param value GameSettings
---@param persist boolean?
function App:_set_settings(value, persist)
    self.settings = settings_model.validate(value)
    if self.apply_settings then
        self.apply_settings(self.settings)
    end
    for _, screen in ipairs(self.stack.screens) do
        if screen.apply_settings then
            screen:apply_settings(self.settings)
        end
    end
    if persist then
        settings_model.save(self.settings, self.settings_storage)
    end
end

---@param action table
function App:handle_action(action)
    local route = self:current_route()
    if action.go == "quit" then
        self.quit_requested = true
        if self.request_quit then
            self.request_quit()
        end
    elseif route == "title" and action.go == "play" then
        session_model.set_combat_enabled(self.session, false)
        self:show_squad()
    elseif route == "title" and action.go == "combat_prototype" then
        session_model.set_combat_enabled(self.session, true)
        self:show_squad()
    elseif route == "title" and action.go == "online_lobby" then
        self:show_lobby()
    elseif route == "lobby" and action.go == "online_match" then
        self:start_online_match(assert(action.freeze, "an online start needs its freeze"))
    elseif route == "online_match" and action.go == "online_result" then
        self:show_online_result(assert(action.result, "an online result needs its record"))
    elseif route == "online_match" and action.go == "online_ended" then
        -- The session is terminal, so its lobby is unusable: nothing about it can
        -- be reused, and pretending otherwise would put a stale manifest and a
        -- dead transport behind the next match. Back to the title, with why.
        self.online_error = action.detail
            or (action.terminal and action.terminal.reason)
            or "the online session ended"
        self:show_title()
    elseif
        route == "online_result"
        and (action.go == "rematch" or action.go == "change_lineup" or action.go == "change_plan")
    then
        -- A rematch is a *new* session. The manifest is immutable per session and
        -- the freeze is spent, so the honest rematch is a fresh lobby rather than
        -- a silent reuse of an incompatible manifest. Lineup and plan land in the
        -- same place for the same reason: online seating and the match mode are
        -- lobby choices, not local ones, so there is nowhere else honest to go.
        self:show_title()
        self:show_lobby()
    elseif route == "title" and action.go == "help" then
        self:_push("help", self:_menu(Help))
    elseif route == "title" and action.go == "credits" then
        self:_push("credits", self:_menu(Credits))
    elseif action.go == "settings" then
        self:_push("settings", self:_menu(Settings, { settings = self.settings }))
    elseif action.go == "settings_changed" then
        self:_set_settings(action.settings)
    elseif action.go == "back" then
        if action.settings then
            self:_set_settings(action.settings, true)
        end
        self:_pop()
    elseif route == "squad" and action.go == "formation" then
        assert(session_model.set_starters(self.session, action.starter_ids))
        self:show_formation()
    elseif route == "squad" and action.go == "title" then
        self:show_title()
    elseif route == "formation" and action.go == "squad" then
        if action.formation_id then
            session_model.set_formation(self.session, action.formation_id)
        end
        self:show_squad()
    elseif route == "formation" and action.go == "tactic" then
        session_model.set_formation(self.session, action.formation_id)
        self:show_tactic()
    elseif route == "tactic" and action.go == "formation" then
        if action.tactic_id then
            session_model.set_tactic(self.session, action.tactic_id)
        end
        self:show_formation()
    elseif route == "tactic" and action.go == "match" then
        session_model.set_tactic(self.session, action.tactic_id)
        self:start_match()
    elseif route == "result" and action.go == "rematch" then
        self:start_match()
    elseif route == "result" and action.go == "change_plan" then
        self:show_formation()
    elseif route == "result" and action.go == "change_lineup" then
        self:show_squad()
    elseif action.go == "main_menu" then
        self:show_title()
    elseif route == "pause" and action.go == "resume" then
        self:_pop()
    elseif route == "pause" and action.go == "controls" then
        self:_push("help", self:_menu(Help))
    elseif route == "pause" and action.go == "restart" then
        self:start_match()
    end
end

---@param width number
---@param height number
function App:resize(width, height)
    self.transform = viewport_model.new(width, height)
end

function App:pause_match()
    if self:current_route() == "match" then
        self:show_pause()
    end
end

---@return OnlineMatchScreen?
function App:_online_match()
    if self:current_route() ~= "online_match" then
        return nil
    end
    local screen = self.stack:current()
    ---@cast screen OnlineMatchScreen
    return screen
end

-- Focus loss pauses an offline match and deliberately does *not* pause an online
-- one. A peer that stops advancing stops publishing authority, and every other
-- peer stalls behind it; OMP-3 has no host-arbitrated pause, so the honest
-- answer is to keep simulating and say so on screen.
---@param focused boolean
function App:focus(focused)
    if focused then
        return
    end
    local online = self:_online_match()
    if online then
        online:focus_lost()
        return
    end
    self:pause_match()
end

-- A lost controller pauses an offline match and only notifies an online one, for
-- the same reason: keyboard input still reaches the live slot, and stalling the
-- session would punish every other peer for this peer's unplugged pad.
function App:controller_removed()
    local online = self:_online_match()
    if online then
        online:controller_lost()
        return
    end
    self:pause_match()
end

---@param event InputEvent|RawGamepadEvent
function App:event(event)
    local route = self:current_route()
    local normalized = controller.normalize(event, self.transform)
    if not normalized then
        return
    end
    if normalized.kind == "action" and normalized.action == "toggle_mute" then
        local settings = settings_model.validate(self.settings)
        settings.muted = not settings.muted
        self:_set_settings(settings, true)
        return
    elseif normalized.kind == "action" and normalized.action == "toggle_fullscreen" then
        local settings = settings_model.validate(self.settings)
        settings.fullscreen = not settings.fullscreen
        self:_set_settings(settings, true)
        return
    end
    if
        normalized.kind == "action"
        and route == "match"
        and (normalized.action == "pause" or normalized.action == "back")
    then
        self:show_pause()
        return
    end
    -- The online match owns pause and back itself: there is no pause screen to
    -- push, because pushing one would stop this peer's driver.
    self.stack:event(normalized)
end

---@param dt number
function App:update(dt)
    self.stack:update(dt)
end

function App:draw()
    self.stack:draw()
end

return App
