local BrowserStarTransport = require("game.transport.browser_star")
local BrowserTransport = require("game.transport.browser")
local FakeRelayTransport = require("game.transport.fake_relay")
local FakeStarTransport = require("game.transport.fake_star")
local FakeTransport = require("game.transport.fake")

---@class TransportModule
local transport = {}

---@param options FakeTransportOptions?
---@return FakeTransport
function transport.fake(options)
    return FakeTransport.new(options)
end

---@param options BrowserTransportOptions?
---@return BrowserTransport
function transport.browser(options)
    return BrowserTransport.new(options or {})
end

-- Host-star adapters: one host endpoint with up to seven independently
-- addressed guest links. The fake star is pure and in-process; the browser
-- star drives real peer connections behind the JavaScript bridge.
---@param options FakeStarTransportOptions?
---@return FakeStarTransport
function transport.fake_star(options)
    return FakeStarTransport.new(options)
end

-- One rendezvous per logical star. Pass the same one to a host and its guests
-- to let them complete a manual handshake in process; endpoints that were not
-- handed it cannot see each other's signals.
---@return FakeStarRendezvous
function transport.fake_star_rendezvous()
    return FakeStarTransport.new_rendezvous()
end

---@param options BrowserStarTransportOptions?
---@return BrowserStarTransport
function transport.browser_star(options)
    return BrowserStarTransport.new(options or {})
end

-- Relay adapter: every member holds one link to a room that forwards opaque
-- payloads and parses nothing. No member is privileged, so a `broadcast` costs
-- one upload rather than one per recipient. In-process only; it exists to
-- measure the OMP-4 relay topology in the fault harness.
---@param options FakeRelayTransportOptions
---@return FakeRelayTransport
function transport.fake_relay(options)
    return FakeRelayTransport.new(options)
end

-- One room per logical relay session. Pass the same one to every member; an
-- endpoint handed a different room cannot see them at all.
---@return FakeRelayRoom
function transport.fake_relay_room()
    return FakeRelayTransport.new_room()
end

return transport
