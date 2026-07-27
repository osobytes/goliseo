-- The online match request: what a peer derives from the frozen manifest and the
-- frozen slot assignment, and what it refuses to derive.

local t = require("spec.support.runner")
local fixture = require("spec.fixtures.online_match_session")
local live_slot = require("game.online.live_slot")
local match_manifest = require("game.online.match_manifest")
local match_session = require("game.online.match_session")
local protocol = require("game.online.protocol")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match_snapshot = require("sim.match_snapshot")

---@param mode SessionMatchMode
---@param peer_index integer?
---@return OnlineMatchRequest
local function request_for(mode, peer_index)
    local manifest = fixture.manifest(mode)
    local peer_ids = fixture.peer_ids(mode)
    local freeze = fixture.freeze(manifest, peer_ids)
    local index = peer_index or 1
    return (
        assert(match_session.request({
            role = index == 1 and "host" or "guest",
            peer_id = peer_ids[index],
            manifest = manifest,
            freeze = freeze,
        }))
    )
end

t.describe("content-derived online manifest", function()
    t.it("resolves back to the content the simulation needs", function()
        local manifest = match_manifest.template("4v4")
        t.is_true(protocol.validate_manifest(manifest))
        local content = assert(match_manifest.resolve(manifest))
        t.eq(content.home.id, match_manifest.HOME_TEAM_ID)
        t.eq(content.away.id, match_manifest.AWAY_TEAM_ID)
        t.eq(content.arena.id, match_manifest.ARENA_ID)
        t.eq(#content.ownership.slots, input_frame.SLOT_COUNT)
    end)

    t.it("is identical across modes apart from the mode itself", function()
        local one = match_manifest.template("1v1")
        local four = match_manifest.template("4v4")
        t.eq(one.content_id, four.content_id)
        t.eq(one.tuning_id, four.tuning_id)
        one.match_mode = "4v4"
        t.eq(protocol.manifest_id(one), protocol.manifest_id(four))
    end)

    t.it("mints deterministic, protocol-shaped identities", function()
        for _, id in ipairs({
            match_manifest.content_id(),
            match_manifest.tuning_id(),
            match_manifest.build_id(),
        }) do
            t.is_true(
                id:match("^[A-Za-z0-9][A-Za-z0-9_%.%-]*$") ~= nil,
                "identity is not a bounded opaque ASCII id: " .. id
            )
            t.is_true(#id <= 128)
        end
        -- Two peers on the same build must agree, so the digest can depend on
        -- nothing but the content itself — no clock, no counter, no table order.
        t.eq(match_manifest.content_id(), match_manifest.content_id())
        t.eq(match_manifest.tuning_id(), match_manifest.tuning_id())
        t.eq(match_manifest.build_id(), match_manifest.build_id())
        t.eq(
            protocol.manifest_id(match_manifest.template("2v2")),
            protocol.manifest_id(match_manifest.template("2v2"))
        )
    end)

    t.it("describes rosters the protocol and the simulation both accept", function()
        local manifest = match_manifest.template("4v4")
        for _, team in ipairs(manifest.teams) do
            t.eq(#team.roster, 5)
            t.eq(team.roster[1].position, "keeper", "a manifest roster places its keeper first")
            t.eq(team.roster[1].loadout_id, nil, "keepers carry no combat loadout")
            t.eq(team.roster[1].family_id, nil)
            for index = 2, #team.roster do
                local player = team.roster[index]
                t.is_true(player.position ~= "keeper", "only one keeper per roster")
                t.is_true(player.loadout_id ~= nil, "an online outfielder needs a fixed loadout")
                t.is_true(player.family_id ~= nil, "an online outfielder needs its family")
            end
        end
    end)

    t.it("refuses a manifest naming content this build does not have", function()
        for _, mutate in ipairs({
            function(manifest)
                manifest.teams[1].team_id = "team_that_does_not_exist"
            end,
            function(manifest)
                manifest.arena_id = "arena.that.does.not.exist"
            end,
            function(manifest)
                manifest.teams[1].roster[2].player_id = "player_that_does_not_exist"
            end,
        }) do
            local manifest = match_manifest.template("4v4")
            mutate(manifest)
            local content, err = match_manifest.resolve(manifest)
            t.eq(content, nil)
            t.is_true(err ~= nil)
        end
    end)

    t.it("refuses a manifest that disagrees about a player this build has", function()
        -- The dangerous case: every id resolves, but the manifest describes the
        -- player differently, which would build a different match on this peer.
        local manifest = match_manifest.template("4v4")
        manifest.teams[1].roster[2].position = "defender" == manifest.teams[1].roster[2].position
                and "midfielder"
            or "defender"
        local content, err = match_manifest.resolve(manifest)
        t.eq(content, nil)
        t.is_true(err ~= nil)

        manifest = match_manifest.template("4v4")
        manifest.teams[1].roster[2].loadout_id = "loadout_that_does_not_exist"
        content, err = match_manifest.resolve(manifest)
        t.eq(content, nil)
        t.is_true(err ~= nil)
    end)

    t.it("refuses a slot table that disagrees with locally computed ownership", function()
        local manifest = match_manifest.template("4v4")
        manifest.slots[1].player_id, manifest.slots[2].player_id =
            manifest.slots[2].player_id, manifest.slots[1].player_id
        local content, err = match_manifest.resolve(manifest)
        t.eq(content, nil)
        t.is_true(err ~= nil)
    end)
end)

t.describe("online match request", function()
    t.it("selects the combat-bearing snapshot and tape contracts explicitly", function()
        local request = request_for("4v4")
        t.is_true(request.combat_enabled)
        t.eq(request.snapshot_version, match_snapshot.COMBAT_VERSION)
        t.eq(request.tape_version, input_tape.COMBAT_VERSION)
        local state, combat = match_snapshot.restore(request.initial_snapshot)
        t.is_true(state.slot_mode, "an online match is always slot mode")
        t.eq(state.input_tick, 0)
        t.is_true(combat ~= nil, "the online request carries a combat companion")
    end)

    t.it("gives every peer a byte-identical boundary zero", function()
        local manifest = fixture.manifest("2v2")
        local peer_ids = fixture.peer_ids("2v2")
        local freeze = fixture.freeze(manifest, peer_ids)
        local hashes = {}
        for index, peer_id in ipairs(peer_ids) do
            local request = assert(match_session.request({
                role = index == 1 and "host" or "guest",
                peer_id = peer_id,
                manifest = manifest,
                freeze = freeze,
            }))
            hashes[index] = match_snapshot.hash(request.initial_snapshot)
        end
        for index = 2, #hashes do
            t.eq(hashes[index], hashes[1], "peers disagree about boundary zero")
        end
    end)

    -- The owned-set size is the *only* thing the mode changes. Switching is
    -- routed identically in all three; 4v4 is inert because its set is a
    -- singleton, not because anything branches on the mode.
    t.it("sizes the owned set from the frozen mode alone", function()
        for _, case in ipairs({
            { mode = "1v1", slots = 4 },
            { mode = "2v2", slots = 2 },
            { mode = "4v4", slots = 1 },
        }) do
            local request = request_for(case.mode)
            t.eq(#request.owned, case.slots, case.mode .. " owned set size")
            t.eq(request.owned[1], request.live, case.mode .. " opens live on its first slot")
        end
    end)

    t.it("never seats a keeper in any owned set, in any mode", function()
        for _, mode in ipairs({ "1v1", "2v2", "4v4" }) do
            local request = request_for(mode)
            local state = match_snapshot.restore(request.initial_snapshot)
            for _, slot in ipairs(request.owned) do
                local index = assert(state.slot_players[live_slot.slot_index(slot)])
                t.is_true(
                    not state.players[index].is_keeper,
                    mode .. " owned a keeper, which no mode may do"
                )
            end
            -- Keeper control is a documented, deliberate divergence from solo
            -- play. Pin the decision so it is not "fixed" as a bug later.
            t.eq(match_session.KEEPER_CONTROL, false)
        end
    end)

    t.it("marks exactly one live human slot and drives every other slot with AI", function()
        local request = request_for("1v1")
        local humans = 0
        for _, driver in pairs(request.slot_drivers) do
            if driver == "human" then
                humans = humans + 1
            end
        end
        t.eq(humans, 2, "a 1v1 has two humans, each live on exactly one slot")
        t.eq(request.slot_drivers[request.live], "human")
        for index = 2, #request.owned do
            t.eq(
                request.slot_drivers[request.owned[index]],
                "ai",
                "a non-live owned slot is AI-driven, exactly as in solo play"
            )
        end
    end)

    t.it("refuses a freeze whose digest does not match the manifest", function()
        local manifest = fixture.manifest("4v4")
        local freeze = fixture.freeze(manifest, fixture.peer_ids("4v4"))
        freeze.manifest_id = "manifest.something.else"
        local request, err = match_session.request({
            role = "host",
            peer_id = fixture.HOST_PEER_ID,
            manifest = manifest,
            freeze = freeze,
        })
        t.eq(request, nil)
        t.is_true(err ~= nil)
    end)

    t.it("refuses a freeze that disagrees with the manifest about the match", function()
        local manifest = fixture.manifest("4v4")
        local freeze = fixture.freeze(manifest, fixture.peer_ids("4v4"))
        freeze.duration_ticks = freeze.duration_ticks + 1
        local request, err = match_session.request({
            role = "host",
            peer_id = fixture.HOST_PEER_ID,
            manifest = manifest,
            freeze = freeze,
        })
        t.eq(request, nil)
        t.is_true(err ~= nil)
    end)

    t.it("refuses a peer the freeze does not seat", function()
        local manifest = fixture.manifest("4v4")
        local freeze = fixture.freeze(manifest, fixture.peer_ids("4v4"))
        local request, err = match_session.request({
            role = "guest",
            peer_id = "guest_9",
            manifest = manifest,
            freeze = freeze,
        })
        t.eq(request, nil)
        t.is_true(err ~= nil)
    end)

    t.it("refuses a non-combat snapshot contract", function()
        local manifest = fixture.manifest("4v4")
        local freeze = fixture.freeze(manifest, fixture.peer_ids("4v4"))
        manifest.snapshot_version = match_snapshot.COMBAT_VERSION - 1
        local request, err = match_session.request({
            role = "host",
            peer_id = fixture.HOST_PEER_ID,
            manifest = manifest,
            freeze = freeze,
        })
        t.eq(request, nil)
        t.is_true(err ~= nil)
    end)
end)
