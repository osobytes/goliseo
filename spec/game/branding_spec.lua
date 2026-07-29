local Credits = require("game.screens.credits")
local Title = require("game.screens.title")
local build_info = require("game.build_info")
local hit = require("game.ui.hit")
local t = require("spec.support.runner")

local VIEWPORT = { w = 960, h = 540 }

t.describe("GOLISEO branding", function()
    t.it("uses the canonical name in metadata and player-facing shell screens", function()
        t.eq(build_info.name, "GOLISEO")

        local title = assert(hit.find(Title.layout(Title.new_state(VIEWPORT)), "title"))
        t.eq(title.text, "GOLISEO")

        local credits = assert(hit.find(Credits.layout(Credits.new_state(VIEWPORT)), "credits"))
        t.is_true(assert(credits.text):match("GOLISEO") ~= nil)
        t.is_true(credits.text:match("GOLISEO contributors") ~= nil)
    end)

    t.it("uses the corrected technical save identity", function()
        t.eq(love.filesystem.getIdentity(), "goliseo")
    end)

    t.it("retains no prototype product name in the player-facing shell", function()
        local title = assert(hit.find(Title.layout(Title.new_state(VIEWPORT)), "title"))
        local credits = assert(hit.find(Credits.layout(Credits.new_state(VIEWPORT)), "credits"))
        for _, text in ipairs({ build_info.name, title.text, assert(credits.text) }) do
            t.is_true(text:lower():match("galactic") == nil)
        end
    end)
end)
