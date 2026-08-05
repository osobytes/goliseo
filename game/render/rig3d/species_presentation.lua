-- Which rig presentation a species wears.
--
-- Two content vocabularies meet here, and this file is the whole of the
-- translation between them:
--
--   * The simulation describes a player's look as a species SHAPE
--     (round / broad / angular / cluster) plus a palette. That is what
--     data/species.lua carries and what game/presentation/identity.lua hands
--     the renderer.
--   * The rig describes it as a THEME (shape language, materials, loadout) and
--     a FIGURE (how stylised the body language is), in rig3d/themes.lua.
--
-- Neither vocabulary is derivable from the other, so the casting below is a
-- content decision, not a computation. Keeping it as one table means changing
-- who wears what is a one-line edit rather than a hunt through the renderer.
--
-- Proportions deliberately do NOT vary: docs/design/prototype_theme_roster.md
-- puts a second production rig out of scope and requires one validated skeleton
-- contract. Species read apart through shape language, figure style and accent
-- colour -- which is exactly the roster's claim, and the thing worth testing.
--
-- IDENTITY IS SPLIT IN TWO, and that split is the whole reason this file has
-- two key functions rather than one. Before #337 a vertex carried its literal
-- colour, so species accent, team and keeper-ness all changed the MESH and had
-- to appear in one cache key. Now a vertex carries a palette SLOT INDEX and
-- colour is a uniform, so:
--
--   * MESH identity is (theme, figure) and nothing else. Species colour, team
--     and keeper kit no longer touch geometry at all.
--   * COLOUR identity is (dressed theme, dressed team), resolved through
--     themes.resolvedPalette into the small array the shader takes as a
--     uniform. Twenty players share a handful of meshes and differ only here.
--
-- INTERIM: #115 owns the six conformed character presentations this will be
-- replaced by. Until then this keeps four species distinguishable on the pitch
-- instead of twenty identical bodies in two colours.

local themes = require("game.render.rig3d.themes")

local species_presentation = {}

-- Casting, one row per species shape. Each pairs a theme with a figure whose
-- silhouette carries that shape's read at match scale.
local CASTING = {
    -- The baseline athletic build: nothing exaggerated, so it anchors the set.
    round = { theme = "scifi", figure = "natural" },
    -- Plate over a natural frame is the heaviest silhouette the rig can make.
    broad = { theme = "medieval", figure = "natural" },
    -- Hard bevels plus the minifig's cylinder head and tube limbs read angular.
    angular = { theme = "scifi", figure = "minifig" },
    -- Molded blobby mass with an oversized head: the least humanoid outline.
    cluster = { theme = "toybox", figure = "vinyl" },
}

local DEFAULT_SHAPE = "round"

-- The theme/figure pair one species shape wears. An unknown or absent shape
-- casts as `round` rather than erroring: species content lands ahead of rig
-- content, and a new shape should read as the baseline build until it is cast.
---@param shape string|nil
---@return {theme: string, figure: string}
function species_presentation.casting(shape)
    return CASTING[shape or ""] or CASTING[DEFAULT_SHAPE]
end

-- Cache key for one built MESH. Only what changes geometry belongs here, and
-- since #337 that is the theme's shape flags and the figure's proportions --
-- not team, not keeper kit, not the species accent, all of which are now
-- palette entries resolved at draw time.
---@param shape string|nil
---@return string
function species_presentation.meshKey(shape)
    local cast = species_presentation.casting(shape)
    return cast.theme .. "/" .. cast.figure
end

-- Cache key for one resolved PALETTE. Everything that changes a colour belongs
-- here -- the species accent included, because two species can share a shape
-- and differ only by colour (neutral and terran are both `round`).
---@param shape string|nil
---@param palette number[]|nil
---@param team_key string
---@param is_keeper boolean
---@return string
function species_presentation.paletteKey(shape, palette, team_key, is_keeper)
    local cast = species_presentation.casting(shape)
    return table.concat({
        cast.theme,
        team_key,
        is_keeper and "gk" or "of",
        palette and string.format("%.3f,%.3f,%.3f", palette[1], palette[2], palette[3]) or "-",
    }, "/")
end

---@param key string
---@return table
local function figureByKey(key)
    for _, figure in ipairs(themes.FIGURES) do
        if figure.key == key then
            return figure
        end
    end
    error("unknown figure: " .. tostring(key))
end

-- The theme and figure tables one species shape is cast onto. This is the pair
-- a mesh is built from, and it is exactly what `meshKey` keys.
---@param shape string|nil
---@return table theme, table figure
function species_presentation.dressing(shape)
    local cast = species_presentation.casting(shape)
    return themes.byKey(cast.theme), figureByKey(cast.figure)
end

-- A shallow copy of `theme` with its colour slots adjusted for one player.
--
-- Two overrides, both deliberate:
--   * `accent` takes the species palette, so the secondary surface is the same
--     colour the 2.5D renderer used for that species and the HUD still agrees.
--   * A keeper swaps the team's main and trim, which is how a keeper strip
--     actually works -- same club, different shirt. Team ownership stays
--     readable because trim is already a team-owned colour.
--
-- The copy stays fully authored: every slot the source theme declared survives
-- it, which matters because themes.resolvedPalette now ASSERTS on a missing
-- slot rather than substituting a placeholder. A synthesized theme is exactly
-- the case that assert exists for, so this must not drop anything.
---@param theme table
---@param team table
---@param palette number[]|nil
---@param is_keeper boolean
---@return table theme, table team
local function dress(theme, team, palette, is_keeper)
    local color = {}
    for slot, value in pairs(theme.color) do
        color[slot] = value
    end
    if palette then
        color.accent = palette
    end
    local out = {}
    for k, v in pairs(theme) do
        out[k] = v
    end
    out.color = color
    if is_keeper then
        team = { key = team.key, label = team.label, main = team.trim, trim = team.main }
    end
    return out, team
end

-- The shader palette for one player. Callers cache on `paletteKey`.
--
-- `themes.resolve` only ever reads `team.main` / `team.trim`, so the keeper's
-- swapped strip needs nothing more than the swapped table `dress` returns.
---@param shape string|nil
---@param palette number[]|nil
---@param team table
---@param is_keeper boolean
---@return number[][]
function species_presentation.palette(shape, palette, team, is_keeper)
    local theme = themes.byKey(species_presentation.casting(shape).theme)
    local dressed_theme, dressed_team = dress(theme, team, palette, is_keeper)
    return themes.resolvedPalette(dressed_theme, dressed_team)
end

return species_presentation
