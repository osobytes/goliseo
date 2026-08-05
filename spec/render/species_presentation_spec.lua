-- Species -> rig casting is content, and content is exactly what a cache key
-- gets wrong quietly: two looks sharing one key means one of them silently
-- wears the other's colours, which no amount of staring at the pitch resolves.
--
-- Everything here is pure table work over rig3d/themes.lua, so it runs in the
-- headless tier -- unlike the renderer that calls it (#340).

local species_presentation = require("game.render.rig3d.species_presentation")
local themes = require("game.render.rig3d.themes")
local t = require("spec.support.runner")

local HOME = themes.TEAMS[1]
local RED = { 1, 0, 0 }
local BLUE = { 0, 0, 1 }

t.describe("rig3d species casting", function()
    t.it("casts every authored species shape onto a real theme and figure", function()
        for _, shape in ipairs({ "round", "broad", "angular", "cluster" }) do
            local cast = species_presentation.casting(shape)
            t.is_true(themes.byKey(cast.theme) ~= nil, shape .. " must cast to a known theme")
            local found = false
            for _, figure in ipairs(themes.FIGURES) do
                found = found or figure.key == cast.figure
            end
            t.is_true(found, shape .. " must cast to a known figure: " .. tostring(cast.figure))
        end
    end)

    t.it("falls back to the round build for an unknown or absent shape", function()
        local round = species_presentation.casting("round")
        t.eq(species_presentation.casting(nil).theme, round.theme)
        t.eq(species_presentation.casting(nil).figure, round.figure)
        t.eq(species_presentation.casting("tesseract").theme, round.theme)
        t.eq(species_presentation.casting("tesseract").figure, round.figure)
    end)
end)

t.describe("rig3d presentation cache keys", function()
    t.it("keys a mesh on geometry alone", function()
        -- Since #337 a vertex carries a palette slot, not a colour. If team,
        -- keeper kit or species accent crept back into this key the renderer
        -- would rebuild the same geometry once per team for nothing.
        t.eq(species_presentation.meshKey("round"), species_presentation.meshKey("round"))
        t.is_true(species_presentation.meshKey("round") ~= species_presentation.meshKey("broad"))
        -- round and angular share the scifi theme but not the figure.
        t.is_true(species_presentation.meshKey("round") ~= species_presentation.meshKey("angular"))
    end)

    t.it("keys a palette on everything that can change a colour", function()
        local away = themes.TEAMS[2]
        local base = species_presentation.paletteKey("round", RED, HOME.key, false)
        t.eq(base, species_presentation.paletteKey("round", RED, HOME.key, false))
        t.is_true(
            base ~= species_presentation.paletteKey("round", BLUE, HOME.key, false),
            "two species that share a shape must not share a palette key"
        )
        t.is_true(base ~= species_presentation.paletteKey("round", RED, away.key, false))
        t.is_true(base ~= species_presentation.paletteKey("round", RED, HOME.key, true))
        t.is_true(base ~= species_presentation.paletteKey("broad", RED, HOME.key, false))
        t.is_true(base ~= species_presentation.paletteKey("round", nil, HOME.key, false))
    end)
end)

t.describe("rig3d species palette", function()
    ---@param name string
    ---@param palette number[][]
    ---@return number[]
    local function slot(name, palette)
        for i, key in ipairs(themes.SLOTS) do
            if key == name then
                return palette[i]
            end
        end
        error("unknown slot: " .. name)
    end

    t.it("resolves a full palette for every cast species", function()
        -- themes.resolvedPalette asserts on an unauthored slot, and a dressed
        -- theme is a synthesized one -- exactly the case that assert guards.
        for _, shape in ipairs({ "round", "broad", "angular", "cluster" }) do
            local palette = species_presentation.palette(shape, RED, HOME, false)
            t.eq(#palette, themes.SLOT_COUNT, shape .. ": every slot must resolve")
        end
        local fallback = species_presentation.palette(nil, RED, HOME, false)
        t.eq(#fallback, themes.SLOT_COUNT, "an uncast shape must still resolve every slot")
    end)

    t.it("paints the species colour onto the accent surface", function()
        local palette = species_presentation.palette("round", RED, HOME, false)
        local accent = slot("accent", palette)
        t.eq(accent[1], RED[1])
        t.eq(accent[2], RED[2])
        t.eq(accent[3], RED[3])
    end)

    t.it("leaves the theme's own accent alone when a species has no colour", function()
        local themed = themes.byKey(species_presentation.casting("round").theme)
        local palette = species_presentation.palette("round", nil, HOME, false)
        local accent = slot("accent", palette)
        t.eq(accent[1], themes.resolve(themed.color.accent, HOME)[1])
    end)

    t.it("swaps main and trim for a keeper's strip", function()
        -- Same club, different shirt: the keeper's ownership surface takes the
        -- team's trim and the secondary takes its main.
        local outfield = species_presentation.palette("round", RED, HOME, false)
        local keeper = species_presentation.palette("round", RED, HOME, true)
        -- scifi wires `plate` to "team" and `crest` to "trim".
        t.eq(slot("plate", keeper)[1], slot("crest", outfield)[1])
        t.eq(slot("crest", keeper)[1], slot("plate", outfield)[1])
    end)

    t.it("does not mutate the shared theme it dresses", function()
        local themed = themes.byKey(species_presentation.casting("round").theme)
        local before = themed.color.accent
        species_presentation.palette("round", RED, HOME, false)
        t.eq(themed.color.accent, before, "dressing must copy, never edit themes.LIST")
    end)
end)
