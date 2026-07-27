local BrowserStarTransport = require("game.transport.browser_star")
local BrowserTransport = require("game.transport.browser")
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

---@param options BrowserStarTransportOptions?
---@return BrowserStarTransport
function transport.browser_star(options)
    return BrowserStarTransport.new(options or {})
end

return transport
