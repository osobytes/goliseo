local App = require("game.app")
local match_adapter = require("game.match_adapter")

---@class BootstrapOptions
---@field apply_settings fun(settings: GameSettings)?
---@field request_quit fun()?
---@field settings_storage SettingsStorage?
---@field quick_match boolean?  -- playtest: boot straight into a match

---@class BootstrapModule
local bootstrap = {}

---@param width number
---@param height number
---@param opts BootstrapOptions?
---@return App
function bootstrap.new(width, height, opts)
    opts = opts or {}
    return App.new({
        actual_w = width,
        actual_h = height,
        match_adapter = match_adapter.real(),
        apply_settings = opts.apply_settings,
        request_quit = opts.request_quit,
        settings_storage = opts.settings_storage,
        quick_match = opts.quick_match,
    })
end

return bootstrap
