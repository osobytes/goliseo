-- A frozen online session built from this build's *content-derived* manifest,
-- shortened so a whole match runs inside a headless test.
--
-- `game.online.match_driver_fixture` pins the protocol fixture manifest, whose
-- team and player ids exist only in that fixture. The match flow has to rebuild
-- boundary zero from the manifest, so it needs a manifest that names real
-- content — which is exactly what `game.online.match_manifest` produces.

local coordinator = require("game.online.coordinator")
local match_manifest = require("game.online.match_manifest")
local protocol = require("game.online.protocol")
local transport = require("game.transport")
local transport_contract = require("game.transport.contract")

---@class OnlineMatchFixtureSession
---@field mode SessionMatchMode
---@field manifest SessionManifest
---@field freeze CoordinatorFreeze
---@field peer_ids string[]
---@field host_transport FakeStarTransport
---@field guest_transports table<string, FakeStarTransport>

---@class OnlineMatchFixtureModule
local fixture = {}

fixture.HOST_PEER_ID = transport_contract.HOST_PEER_ID
fixture.COUNTDOWN_ID = "countdown.1"
-- Long enough to reach full time through a correction, short enough that eight
-- peers of combat simulation stay inside a headless run.
fixture.DEFAULT_DURATION_TICKS = 90

---@param index integer
---@return string
function fixture.guest_peer_id(index)
    return "guest_" .. tostring(index)
end

---@param mode SessionMatchMode?
---@param duration_ticks integer?
---@return SessionManifest
function fixture.manifest(mode, duration_ticks)
    local manifest = match_manifest.template(mode)
    manifest.duration_ticks = duration_ticks or fixture.DEFAULT_DURATION_TICKS
    return manifest
end

---@param mode SessionMatchMode
---@param humans integer?
---@return string[]
function fixture.peer_ids(mode, humans)
    local shape = assert(protocol.match_mode(mode), "unknown match mode")
    local count = humans or shape.humans
    assert(count >= 1 and count <= shape.humans, "the mode does not seat that many humans")
    local ids = { fixture.HOST_PEER_ID }
    for index = 1, count - 1 do
        ids[#ids + 1] = fixture.guest_peer_id(index)
    end
    return ids
end

-- The freeze `coordinator.begin_countdown` produces, rebuilt from the same
-- public pieces so the fixture cannot drift from the real one.
---@param manifest SessionManifest
---@param peer_ids string[]
---@param first_input_tick integer?
---@return CoordinatorFreeze
function fixture.freeze(manifest, peer_ids, first_input_tick)
    local assignments = assert(coordinator.plan_assignments(manifest, peer_ids))
    local sources = assert(coordinator.slot_sources(manifest, assignments))
    ---@type table<string, InputSlotId[]>
    local owned = {}
    ---@type table<string, InputSlotId>
    local live = {}
    for _, producer in ipairs(assignments) do
        if producer.producer_kind == "peer" and not owned[producer.producer_id] then
            local slots = protocol.owned_slots(assignments, producer.producer_id)
            owned[producer.producer_id] = slots
            live[producer.producer_id] = slots[1]
        end
    end
    return {
        manifest_id = protocol.manifest_id(manifest),
        assignment_id = assert(protocol.assignment_id(assignments, 1)),
        countdown_id = fixture.COUNTDOWN_ID,
        first_input_tick = first_input_tick or 0,
        seed = manifest.seed,
        tick_rate = manifest.tick_rate,
        duration_ticks = manifest.duration_ticks,
        max_goals = manifest.max_goals,
        content_id = manifest.content_id,
        tuning_id = manifest.tuning_id,
        combat_rules_id = manifest.combat_rules_id,
        gameplay_ai_policy_id = manifest.gameplay_ai_policy_id,
        combat_status = manifest.combat_status,
        match_mode = manifest.match_mode,
        assignments = assignments,
        sources = sources,
        owned = owned,
        live = live,
    }
end

-- One connected in-process star: a host endpoint plus one guest endpoint per
-- seated guest, sharing a rendezvous so they can only reach each other.
---@param mode SessionMatchMode
---@param humans integer?
---@param duration_ticks integer?
---@return OnlineMatchFixtureSession
function fixture.session(mode, humans, duration_ticks)
    local manifest = fixture.manifest(mode, duration_ticks)
    local peer_ids = fixture.peer_ids(mode, humans)
    local freeze = fixture.freeze(manifest, peer_ids)
    local rendezvous = transport.fake_star_rendezvous()
    local host = transport.fake_star({ role = "host", rendezvous = rendezvous })
    assert(host:initialize())
    ---@type table<string, FakeStarTransport>
    local guest_transports = {}
    for index = 2, #peer_ids do
        local peer_id = peer_ids[index]
        local guest = transport.fake_star({
            role = "guest",
            peer_id = peer_id,
            rendezvous = rendezvous,
        })
        assert(guest:initialize())
        assert(host:open_peer(peer_id))
        assert(host:link(guest))
        guest_transports[peer_id] = guest
    end
    return {
        mode = mode,
        manifest = manifest,
        freeze = freeze,
        peer_ids = peer_ids,
        host_transport = host,
        guest_transports = guest_transports,
    }
end

return fixture
