-- Versioned fixtures for the OMP-3 online match driver.
--
-- The driver's acceptance depends on combat-aware gameplay AI (#112) and on the
-- accepted default combat disposition (#114), neither of which has landed. The
-- mechanical plumbing is therefore developed against *these* fixtures: a pinned
-- slot-mode boundary zero, a pinned frozen session, and an in-process star. When
-- #114 accepts a default disposition, the manifest identity changes here and
-- nowhere else.
--
-- Producer ids double as transport link ids, because `canonical_host_batch`
-- requires a peer producer to arrive on its own selected link. They therefore
-- have to satisfy `game.transport.contract.PEER_ID_PATTERN`, which the session
-- coordinator's own test ids (`guest.1`) deliberately do not.

local coordinator = require("game.online.coordinator")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local FakeStarTransport = require("game.transport.fake_star")
local transport_contract = require("game.transport.contract")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")

---@class MatchDriverFixtureSession
---@field mode SessionMatchMode
---@field manifest SessionManifest
---@field freeze CoordinatorFreeze
---@field host_peer_id string
---@field guest_peer_ids string[]
---@field host_transport FakeStarTransport
---@field guest_transports table<string, FakeStarTransport>

---@class MatchDriverFixtureModule
local fixture = {}

fixture.HOST_PEER_ID = transport_contract.HOST_PEER_ID
fixture.COUNTDOWN_ID = "countdown.1"
fixture.DEFAULT_DURATION = 6
fixture.DEFAULT_SEED = 74

---@param index integer
---@return string
function fixture.guest_peer_id(index)
    return "guest_" .. tostring(index)
end

-- Boundary zero for the pinned combat fixture. Slot mode is mandatory: the
-- online driver never runs the legacy single-input path.
---@param duration number?
---@return MatchSnapshot
function fixture.initial_snapshot(duration)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = duration or fixture.DEFAULT_DURATION,
        max_goals = 99,
        seed = fixture.DEFAULT_SEED,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
    return match_snapshot.capture(state)
end

-- `humans` defaults to a full lobby. A short lobby is the only way a declared
-- bot fill exists at all: at full seating every mode covers all eight slots.
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
-- public pieces: `plan_assignments` seats the humans, `slot_sources` proves the
-- seating, `owned_slots` names each owned set, and the opening live slot is the
-- first owned slot in canonical order.
---@param mode SessionMatchMode
---@param first_input_tick integer?
---@param humans integer?
---@return CoordinatorFreeze
function fixture.freeze(mode, first_input_tick, humans)
    local manifest = protocol_fixture.manifest(mode)
    local assignments =
        assert(coordinator.plan_assignments(manifest, fixture.peer_ids(mode, humans)))
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
-- seated guest, all sharing a rendezvous so they can only reach each other.
---@param mode SessionMatchMode
---@param first_input_tick integer?
---@param humans integer?
---@return MatchDriverFixtureSession
function fixture.session(mode, first_input_tick, humans)
    local manifest = protocol_fixture.manifest(mode)
    local freeze = fixture.freeze(mode, first_input_tick, humans)
    local peer_ids = fixture.peer_ids(mode, humans)
    local rendezvous = FakeStarTransport.new_rendezvous()
    local host = FakeStarTransport.new({ role = "host", rendezvous = rendezvous })
    assert(host:initialize())
    ---@type string[]
    local guest_peer_ids = {}
    ---@type table<string, FakeStarTransport>
    local guest_transports = {}
    for index = 2, #peer_ids do
        local peer_id = peer_ids[index]
        guest_peer_ids[#guest_peer_ids + 1] = peer_id
        local guest =
            FakeStarTransport.new({ role = "guest", peer_id = peer_id, rendezvous = rendezvous })
        assert(guest:initialize())
        assert(host:open_peer(peer_id))
        assert(host:link(guest))
        guest_transports[peer_id] = guest
    end
    return {
        mode = mode,
        manifest = manifest,
        freeze = freeze,
        host_peer_id = fixture.HOST_PEER_ID,
        guest_peer_ids = guest_peer_ids,
        host_transport = host,
        guest_transports = guest_transports,
    }
end

return fixture
