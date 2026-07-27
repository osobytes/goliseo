local coordinator = require("game.online.coordinator")
local protocol_fixture = require("game.online.protocol_fixture")

---@class SessionCoordinatorFixtureModule
local fixture = {}

fixture.HOST_PEER_ID = "host"
fixture.GUEST_PREFIX = "guest."
fixture.LINK_PREFIX = "link."
fixture.COUNTDOWN_ID = "countdown.1"

---@return SessionRuntimeIdentity
function fixture.runtime()
    return protocol_fixture.runtime()
end

---@return SessionManifest
function fixture.manifest()
    return protocol_fixture.manifest()
end

-- What a guest can verify locally before accepting a proposal: immutable build,
-- content, tuning, fixture, and combat-disposition identity. Session-scoped
-- values such as the seed are chosen by the host and are not pre-known.
---@return CoordinatorManifestExpectation
function fixture.expectation()
    local manifest = fixture.manifest()
    return {
        build_id = manifest.build_id,
        source_id = manifest.source_id,
        content_id = manifest.content_id,
        tuning_id = manifest.tuning_id,
        match_config_id = manifest.match_config_id,
        fixture_id = manifest.fixture_id,
        arena_id = manifest.arena_id,
        combat_rules_id = manifest.combat_rules_id,
        gameplay_ai_policy_id = manifest.gameplay_ai_policy_id,
        combat_status = manifest.combat_status,
    }
end

---@param index integer
---@return string
function fixture.guest_peer_id(index)
    return fixture.GUEST_PREFIX .. tostring(index)
end

---@param peer_id string
---@return string
function fixture.link_id(peer_id)
    return fixture.LINK_PREFIX .. peer_id
end

---@param guest_count integer
---@return string[]
function fixture.peer_ids(guest_count)
    local ids = { fixture.HOST_PEER_ID }
    for index = 1, guest_count do
        ids[#ids + 1] = fixture.guest_peer_id(index)
    end
    return ids
end

---@param session_id string?
---@return CoordinatorState
function fixture.host(session_id)
    return assert(coordinator.new({
        role = "host",
        session_id = session_id or fixture.manifest().session_id,
        peer_id = fixture.HOST_PEER_ID,
        runtime = fixture.runtime(),
    }))
end

---@param index integer
---@param session_id string?
---@param expectation CoordinatorManifestExpectation?
---@return CoordinatorState
function fixture.guest(index, session_id, expectation)
    local peer_id = fixture.guest_peer_id(index)
    return assert(coordinator.new({
        role = "guest",
        session_id = session_id or fixture.manifest().session_id,
        peer_id = peer_id,
        host_peer_id = fixture.HOST_PEER_ID,
        host_link_id = fixture.link_id(peer_id),
        runtime = fixture.runtime(),
        expectation = expectation or fixture.expectation(),
    }))
end

---@param guest_count integer
---@return SessionSlotProducer[]
function fixture.assignments(guest_count)
    return assert(coordinator.plan_assignments(fixture.manifest(), fixture.peer_ids(guest_count)))
end

return fixture
