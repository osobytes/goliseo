local audio = require("game.audio")
local combat_feedback = require("game.presentation.combat_feedback")
local bloom = require("game.render.bloom")
local effects = require("game.render.effects")

---@class RuntimeSettingsModule
local runtime_settings = {}

---@param settings GameSettings
function runtime_settings.apply(settings)
    bloom.config.enabled = settings.bloom
    audio.configure(settings)
    combat_feedback.configure_defaults(settings)
    effects.configure(settings)

    if love.audio and love.audio.setVolume then
        love.audio.setVolume(settings.master_volume)
    end
    if love.window and love.window.getFullscreen and love.window.setFullscreen then
        local fullscreen = love.window.getFullscreen()
        if fullscreen ~= settings.fullscreen then
            love.window.setFullscreen(settings.fullscreen, "desktop")
        end
    end
end

return runtime_settings
