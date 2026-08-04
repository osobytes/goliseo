local identity = require("render.identity")
local player_renderer = require("game.render.player_renderer")
local players = require("data.players")
local showcase_players = require("data.showcase_player_compatibility")
local t = require("spec.support.runner")

t.describe("pitch presentation identity", function()
    t.it("resolves every authored player without changing mechanical species", function()
        local seen = {}
        for _, player in ipairs(players) do
            local presentation = assert(identity.for_player(player.id))
            t.eq(assert(showcase_players[player.id]).species, "neutral")
            t.is_true(#presentation.palette == 3)
            seen[presentation.shape] = true
        end
        for _, shape in ipairs({ "round", "broad", "angular", "cluster" }) do
            t.is_true(seen[shape], "missing pitch silhouette " .. shape)
        end
    end)

    t.it("keeps the four silhouettes geometrically distinct", function()
        local terran = player_renderer.silhouette("round")
        local gravling = player_renderer.silhouette("broad")
        local voltari = player_renderer.silhouette("angular")
        local myceloid = player_renderer.silhouette("cluster")
        t.is_true(gravling.torso_scale > terran.torso_scale)
        t.is_true(terran.torso_scale > voltari.torso_scale)
        t.is_true(myceloid.torso_scale < voltari.torso_scale)
        t.eq(myceloid.head_kind, "cluster")
        t.eq(voltari.head_kind, "angular")
    end)
end)
