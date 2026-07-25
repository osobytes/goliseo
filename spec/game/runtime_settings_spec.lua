local runtime_settings = require("game.runtime_settings")
local combat_feedback = require("game.presentation.combat_feedback")
local settings = require("game.settings")
local t = require("spec.support.runner")

t.describe("runtime settings", function()
    t.it("applies master volume and fullscreen through thin LÖVE adapters", function()
        local saved_audio = love.audio
        local saved_window = love.window
        local volume = nil
        local fullscreen = false
        love.audio = {
            setVolume = function(value)
                volume = value
            end,
        }
        love.window = {
            getFullscreen = function()
                return false
            end,
            setFullscreen = function(value)
                fullscreen = value
                return true
            end,
        }

        local value = settings.defaults()
        value.master_volume = 0.35
        value.fullscreen = true
        local ok, err = pcall(runtime_settings.apply, value)
        love.audio = saved_audio
        love.window = saved_window

        assert(ok, tostring(err))
        t.eq(volume, 0.35)
        t.eq(fullscreen, true)
    end)

    t.it("routes existing shake and bloom settings to reduced combat feedback", function()
        local value = settings.defaults()
        value.screen_shake = false
        value.bloom = false
        runtime_settings.apply(value)
        local diagnostics = combat_feedback.diagnostics(combat_feedback.new())
        t.is_true(diagnostics.reduced_motion)
        t.is_true(diagnostics.reduced_flash)
        runtime_settings.apply(settings.defaults())
    end)
end)
