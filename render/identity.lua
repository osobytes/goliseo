local players = require("data.players")
local species = require("data.species")
local showcase_players = require("data.showcase_player_compatibility")

---@class PlayerPresentationIdentity
---@field player_id string
---@field name string
---@field position Position
---@field species_id string
---@field species_name string
---@field tagline string
---@field shape "round"|"broad"|"angular"|"cluster"
---@field palette number[]

---@class PresentationIdentityModule
local identity = {}

---@type table<string, PlayerData>
local player_by_id = {}
for _, player in ipairs(players) do
    player_by_id[player.id] = player
end

---@param player_id string
---@return PlayerPresentationIdentity?
function identity.for_player(player_id)
    local player = player_by_id[player_id]
    if not player then
        return nil
    end
    local showcase = showcase_players[player.id]
    if not showcase then
        return nil
    end
    local species_id = showcase.presentation_species or showcase.species
    local presentation = species[species_id]
    if not presentation then
        return nil
    end
    return {
        player_id = player.id,
        name = player.name,
        position = player.position,
        species_id = presentation.id,
        species_name = presentation.name,
        tagline = presentation.tagline or "",
        shape = presentation.shape or "round",
        palette = presentation.palette or { 0.55, 0.72, 0.92 },
    }
end

return identity
