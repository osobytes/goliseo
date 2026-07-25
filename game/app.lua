local controller = require("game.input.controller")
local match_adapter = require("game.match_adapter")
local session_model = require("game.session")
local settings_model = require("game.settings")
local viewport_model = require("game.ui.viewport")
local Credits = require("game.screens.credits")
local Formation = require("game.screens.formation")
local Help = require("game.screens.help")
local Menu = require("game.screens.menu")
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
    self:show_title()
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
    self:_replace("match", screen)
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

---@param focused boolean
function App:focus(focused)
    if not focused then
        self:pause_match()
    end
end

---@param event InputEvent|RawGamepadEvent
function App:event(event)
    local route = self:current_route()
    local normalized = controller.normalize(event, self.transform, route == "match")
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
