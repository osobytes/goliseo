-- Scripted multi-peer lobby flows over the in-process fake star. Every peer
-- here runs the same pure lobby model the screen runs, exchanges real canonical
-- control wires through the real framing, and completes the real manual
-- offer/answer handshake. No browser, no JavaScript, no display.

local lobby_link = require("game.online.lobby_link")
local lobby_model = require("game.screens.lobby_model")
local protocol = require("game.online.protocol")
local t = require("spec.support.runner")
local transport = require("game.transport")

---@class LobbyTestPeer
---@field id string
---@field model LobbyModel
---@field star FakeStarTransport?
---@field link LobbyLink?
---@field clipboard string?
---@field freeze CoordinatorFreeze?
---@field left boolean

---@class LobbyTestDriver
---@field rendezvous FakeStarRendezvous
---@field peers LobbyTestPeer[]
local Driver = {}
Driver.__index = Driver

---@return LobbyTestDriver
local function new_driver()
    return setmetatable({
        rendezvous = transport.fake_star_rendezvous(),
        peers = {},
    }, Driver)
end

---@param peer LobbyTestPeer
---@param effects LobbyEffect[]
function Driver:_run(peer, effects)
    for _, effect in ipairs(effects) do
        if effect.kind == "open_star" then
            local star = transport.fake_star({
                role = effect.role,
                peer_id = effect.role == "guest" and effect.peer_id or nil,
                rendezvous = self.rendezvous,
            })
            assert(star:initialize())
            peer.star = star
            peer.link = lobby_link.new(star)
        elseif effect.kind == "clipboard" then
            peer.clipboard = effect.text
        elseif effect.kind == "start_match" then
            peer.freeze = effect.freeze
        elseif effect.kind == "leave" then
            peer.left = true
        elseif effect.kind ~= "paste_request" then
            if peer.link then
                peer.link:apply(effect)
            end
        end
    end
end

---@param peer LobbyTestPeer
---@param command table
function Driver:send(peer, command)
    local model, effects = lobby_model.command(peer.model, command)
    peer.model = model
    self:_run(peer, effects)
end

---@param role LobbyRole
---@param peer_id string
---@return LobbyTestPeer
function Driver:add(role, peer_id)
    ---@type LobbyTestPeer
    local peer = {
        id = peer_id,
        model = lobby_model.new({ peer_id = peer_id }),
        left = false,
    }
    self.peers[#self.peers + 1] = peer
    self:send(peer, { kind = "role", role = role })
    return peer
end

---@param rounds integer?
function Driver:pump(rounds)
    for _ = 1, rounds or 6 do
        for _, peer in ipairs(self.peers) do
            if peer.star then
                peer.star:pump()
            end
        end
        for _, peer in ipairs(self.peers) do
            if peer.link then
                for _, event in ipairs(peer.link:poll()) do
                    self:send(peer, event)
                end
            end
        end
    end
end

---@param count integer
function Driver:tick(count)
    for _ = 1, count do
        for _, peer in ipairs(self.peers) do
            self:send(peer, { kind = "tick" })
        end
        self:pump(2)
    end
end

-- One complete manual handshake: the host invites, both sides copy and paste,
-- and the guest's coordinator handshake reaches the host.
---@param host LobbyTestPeer
---@param guest LobbyTestPeer
function Driver:connect(host, guest)
    self:send(host, { kind = "invite" })
    self:pump(2)
    self:send(host, { kind = "copy" })
    local offer = assert(host.clipboard, "the host produced no offer")
    self:send(guest, { kind = "paste", text = offer })
    self:pump(2)
    self:send(guest, { kind = "copy" })
    local answer = assert(guest.clipboard, "the guest produced no answer")
    self:send(host, { kind = "paste", text = answer })
    self:pump(4)
end

---@param mode SessionMatchMode
---@param guest_count integer
---@return LobbyTestDriver, LobbyTestPeer, LobbyTestPeer[]
local function seated_lobby(mode, guest_count)
    local driver = new_driver()
    local host = driver:add("host", "host")
    driver:send(host, { kind = "mode", mode = mode })
    ---@type LobbyTestPeer[]
    local guests = {}
    for index = 1, guest_count do
        local guest = driver:add("guest", "guest_" .. index)
        driver:connect(host, guest)
        guests[index] = guest
    end
    return driver, host, guests
end

---@param peer LobbyTestPeer
---@return LobbyView
local function view(peer)
    return lobby_model.view(peer.model)
end

---@param peer LobbyTestPeer
---@param producer_id string
---@return string[]
local function owned(peer, producer_id)
    local slots = {}
    for _, slot in ipairs(view(peer).slots) do
        if slot.owner == producer_id then
            slots[#slots + 1] = slot.slot
        end
    end
    return slots
end

t.describe("lobby control framing", function()
    t.it("splits and rebuilds a wire that exceeds one transport payload", function()
        local wire = string.rep("x", 2500)
        local frames = assert(lobby_link.frame(wire))
        t.eq(#frames, 3)
        local buffer = lobby_link.new_buffer()
        t.eq(lobby_link.absorb(buffer, frames[1]), nil)
        t.eq(lobby_link.absorb(buffer, frames[2]), nil)
        t.eq(lobby_link.absorb(buffer, frames[3]), wire)
    end)

    t.it("keeps every frame inside the transport payload bound", function()
        local frames = assert(lobby_link.frame(string.rep("y", protocol.MAX_WIRE_BYTES)))
        for _, frame in ipairs(frames) do
            t.is_true(#frame <= 1024, "frame exceeds the transport payload bound")
        end
    end)

    t.it("refuses a stream that starts mid-wire or reorders", function()
        local frames = assert(lobby_link.frame(string.rep("z", 2000)))
        local _, err = lobby_link.absorb(lobby_link.new_buffer(), frames[2])
        t.is_true(err ~= nil)
        local buffer = lobby_link.new_buffer()
        lobby_link.absorb(buffer, frames[1])
        local _, out_of_order = lobby_link.absorb(buffer, frames[1])
        t.is_true(out_of_order ~= nil)
    end)

    t.it("refuses a wire beyond the protocol bound", function()
        local _, err = lobby_link.frame(string.rep("q", protocol.MAX_WIRE_BYTES + 1))
        t.is_true(err ~= nil)
    end)
end)

t.describe("manual lobby handshake", function()
    t.it("completes offer and answer exchange without console commands", function()
        local driver, host, guests = seated_lobby("1v1", 1)
        t.eq(view(host).connected, 2)
        t.eq(view(guests[1]).connected, 2)
        t.eq(host.model.pending_link, nil)
        driver:pump(1)
    end)

    t.it("never retains the pasted blob after it is used", function()
        local _, host = seated_lobby("1v1", 1)
        t.eq(host.model.outgoing, nil)
        local record = assert(view(host).imported)
        t.eq(record.direction, "answer")
        t.is_true(record.bytes > 0)
        t.eq(#record.fingerprint, 8)
    end)

    t.it("reports a malformed paste without ending the session", function()
        local driver = new_driver()
        local host = driver:add("host", "host")
        driver:send(host, { kind = "paste", text = "" })
        t.is_true(view(host).error ~= nil)
        t.eq(view(host).phase, "handshake")
        driver:send(host, { kind = "invite" })
        t.eq(view(host).error, nil)
    end)
end)

t.describe("lobby match modes", function()
    t.it("seats a 1v1 human on a whole outfield line", function()
        local driver, host, guests = seated_lobby("1v1", 1)
        driver:send(host, { kind = "lock" })
        driver:pump(6)

        local host_slots = owned(host, "host")
        t.eq(#host_slots, 4)
        t.eq(host_slots[1], "home_1")
        t.eq(host_slots[4], "home_4")
        t.eq(#owned(host, "guest_1"), 4)
        t.eq(#owned(guests[1], "guest_1"), 4)
    end)

    t.it("shows AI-driven slots inside a human's owned set", function()
        local driver, host = seated_lobby("1v1", 1)
        driver:send(host, { kind = "lock" })
        driver:pump(6)

        local human, ai = 0, 0
        for _, slot in ipairs(view(host).slots) do
            if slot.owner == "host" then
                if slot.driver == "human" then
                    human = human + 1
                else
                    ai = ai + 1
                    t.eq(slot.owner_kind, "peer")
                end
            end
        end
        t.eq(human, 1)
        t.eq(ai, 3)
    end)

    t.it("seats a 2v2 human on a chosen pair and repartitions on a swap", function()
        local driver, host, guests = seated_lobby("2v2", 3)
        driver:send(host, { kind = "lock" })
        driver:pump(8)

        t.eq(#owned(host, "host"), 2)
        t.eq(table.concat(owned(host, "host"), ","), "home_1,home_2")
        t.eq(table.concat(owned(host, "guest_1"), ","), "home_3,home_4")

        driver:send(host, { kind = "ready", ready = true })
        driver:pump(2)
        t.is_true(view(host).ready)

        driver:send(host, { kind = "swap", index = 1 })
        driver:pump(4)
        t.eq(table.concat(owned(host, "host"), ","), "home_3,home_4")
        t.eq(table.concat(owned(host, "guest_1"), ","), "home_1,home_2")
        t.is_true(not view(host).ready, "readiness must clear on a pair change")
        t.eq(#owned(guests[1], "guest_1"), 2)
    end)

    t.it("gates the required peer count on the mode", function()
        local driver = new_driver()
        local host = driver:add("host", "host")
        driver:send(host, { kind = "mode", mode = "2v2" })
        t.eq(view(host).required, 4)
        driver:send(host, { kind = "lock" })
        t.is_true(view(host).error ~= nil)
        t.eq(view(host).phase, "handshake")

        driver:send(host, { kind = "mode", mode = "1v1" })
        t.eq(view(host).required, 2)
    end)

    t.it("fills empty seats with AI only when the host approves it", function()
        local driver = new_driver()
        local host = driver:add("host", "host")
        driver:send(host, { kind = "mode", mode = "4v4" })
        driver:send(host, { kind = "bot_fill" })
        driver:send(host, { kind = "lock" })
        driver:pump(4)

        t.eq(#owned(host, "host"), 1)
        local bots = 0
        for _, slot in ipairs(view(host).slots) do
            if slot.owner_kind == "bot" then
                bots = bots + 1
                t.eq(slot.driver, "ai")
            end
        end
        t.eq(bots, 7)
    end)

    t.it("keeps both keepers protected and slotless in every mode", function()
        for _, mode in ipairs({ "1v1", "2v2", "4v4" }) do
            local driver = new_driver()
            local host = driver:add("host", "host")
            driver:send(host, { kind = "mode", mode = mode })
            local keepers = view(host).keepers
            t.eq(#keepers, 2)
            for _, slot in ipairs(view(host).slots) do
                for _, keeper in ipairs(keepers) do
                    t.is_true(slot.player_id ~= keeper.player_id, "a keeper owns a canonical slot")
                end
            end
        end
    end)

    t.it("locks the mode once the manifest is proposed", function()
        local driver, host = seated_lobby("1v1", 1)
        driver:send(host, { kind = "lock" })
        driver:pump(4)
        t.is_true(view(host).mode_locked)
        driver:send(host, { kind = "mode", mode = "4v4" })
        t.is_true(view(host).error ~= nil)
        t.eq(view(host).mode, "1v1")
    end)
end)

t.describe("lobby readiness and countdown", function()
    t.it("reaches a synchronized start only after every peer is ready", function()
        local driver, host, guests = seated_lobby("1v1", 1)
        driver:send(host, { kind = "lock" })
        driver:pump(6)

        driver:send(host, { kind = "start" })
        t.is_true(view(host).error ~= nil, "countdown must require readiness")

        driver:send(host, { kind = "ready", ready = true })
        driver:send(guests[1], { kind = "ready", ready = true })
        driver:pump(4)
        t.eq(view(host).phase, "ready")
        t.is_true(view(host).can_start)

        driver:send(host, { kind = "start" })
        driver:pump(2)
        t.eq(view(host).phase, "countdown")
        t.is_true(view(guests[1]).countdown ~= nil)

        driver:tick(lobby_model.COUNTDOWN_TICKS + 4)
        local freeze = assert(host.freeze, "the host never reached the start boundary")
        t.eq(freeze.match_mode, "1v1")
        local guest_freeze = assert(guests[1].freeze)
        t.eq(guest_freeze.manifest_id, freeze.manifest_id)
        t.eq(#assert(freeze.owned["host"]), 4)
        t.eq(freeze.live["host"], "home_1")
    end)

    t.it("clears readiness when ownership is republished", function()
        local driver, host, guests = seated_lobby("2v2", 3)
        driver:send(host, { kind = "lock" })
        driver:pump(8)
        for _, peer in ipairs({ host, guests[1], guests[2], guests[3] }) do
            driver:send(peer, { kind = "ready", ready = true })
        end
        driver:pump(4)
        t.eq(view(host).phase, "ready")

        driver:send(host, { kind = "swap", index = 2 })
        driver:pump(4)
        t.eq(view(host).phase, "assigned")
        t.eq(view(host).ready_count, 0)
    end)
end)

t.describe("online lobby screen shell", function()
    -- Drives the impure screen the product actually mounts: it owns the star,
    -- the clipboard, and the fixed-rate clock, and reaches the same start
    -- boundary the pure flows above do.
    t.it("carries a full 1v1 session through two mounted screens", function()
        local OnlineLobby = require("game.screens.online_lobby")
        local rendezvous = transport.fake_star_rendezvous()
        ---@type table<string, string?>
        local clipboards = {}
        ---@type FakeStarTransport[]
        local stars = {}

        ---@param name string
        ---@return OnlineLobby
        local function mount(name)
            return OnlineLobby.new({ w = 960, h = 540 }, nil, {
                star_factory = function(role, peer_id)
                    local star = transport.fake_star({
                        role = role,
                        peer_id = role == "guest" and peer_id or nil,
                        rendezvous = rendezvous,
                    })
                    assert(star:initialize())
                    stars[#stars + 1] = star
                    return star
                end,
                clipboard = {
                    read = function()
                        return clipboards.shared
                    end,
                    write = function(text)
                        clipboards.shared = text
                    end,
                },
                model_options = { peer_id = name },
            })
        end

        local host = mount("host")
        local guest = mount("guest_1")
        local function pump(rounds)
            for _ = 1, rounds or 4 do
                for _, star in ipairs(stars) do
                    star:pump()
                end
                host:update(0)
                guest:update(0)
            end
        end

        host:dispatch({ kind = "role", role = "host" })
        host:dispatch({ kind = "mode", mode = "1v1" })
        guest:dispatch({ kind = "role", role = "guest" })
        host:dispatch({ kind = "invite" })
        pump(2)
        host:dispatch({ kind = "copy" })
        guest:dispatch({ kind = "paste_request" })
        pump(2)
        guest:dispatch({ kind = "copy" })
        host:dispatch({ kind = "paste_request" })
        pump(4)

        t.eq(lobby_model.view(host.state.model).connected, 2)
        host:dispatch({ kind = "lock" })
        pump(4)
        host:dispatch({ kind = "ready", ready = true })
        guest:dispatch({ kind = "ready", ready = true })
        pump(2)
        host:dispatch({ kind = "start" })
        for _ = 1, lobby_model.COUNTDOWN_TICKS + 10 do
            for _, star in ipairs(stars) do
                star:pump()
            end
            host:update(1 / 60)
            guest:update(1 / 60)
        end
        t.is_true(lobby_model.view(host.state.model).started, "the host never started")
        t.is_true(lobby_model.view(guest.state.model).started, "the guest never started")
        host:teardown()
        guest:teardown()
    end)

    t.it("is reachable from the title and returns to it", function()
        local App = require("game.app")
        local hit = require("game.ui.hit")
        local viewport = require("game.ui.viewport")
        local app = App.new()
        local menu = assert(app.stack:current())
        ---@cast menu Menu
        local widget = assert(hit.find(menu.def.layout(menu.state), "online_lobby"))
        local x, y = viewport.to_actual(
            app.transform,
            widget.rect.x + widget.rect.w / 2,
            widget.rect.y + widget.rect.h / 2
        )
        app:event({ kind = "click", x = x, y = y, button = 1 })
        t.eq(app:current_route(), "lobby")
        app:event({ kind = "key", key = "escape" })
        t.eq(app:current_route(), "title")
    end)
end)

t.describe("lobby failure paths", function()
    t.it("ends a guest session with a stable reason when the host aborts", function()
        local driver, host, guests = seated_lobby("1v1", 1)
        driver:send(host, { kind = "leave" })
        driver:pump(4)
        t.is_true(host.left)
        local guest_view = view(guests[1])
        t.eq(guest_view.phase, "terminal")
        t.is_true(guest_view.terminal_text ~= nil)
    end)

    t.it("drops a departed guest and voids the ownership that named it", function()
        local driver, host, guests = seated_lobby("2v2", 3)
        driver:send(host, { kind = "lock" })
        driver:pump(8)
        t.eq(view(host).connected, 4)

        driver:send(guests[3], { kind = "leave" })
        driver:pump(4)
        t.eq(view(host).connected, 3)
        t.eq(view(host).phase, "assigned")
    end)

    t.it("terminates a guest whose local identity differs from the manifest", function()
        local driver = new_driver()
        local host = driver:add("host", "host")
        driver:send(host, { kind = "mode", mode = "1v1" })
        ---@type LobbyTestPeer
        local guest = {
            id = "guest_1",
            model = lobby_model.new({
                peer_id = "guest_1",
                template = function(mode)
                    local manifest = require("game.online.protocol_fixture").manifest(mode)
                    manifest.content_id = "content.other.v1"
                    return manifest
                end,
            }),
            left = false,
        }
        driver.peers[#driver.peers + 1] = guest
        driver:send(guest, { kind = "role", role = "guest" })
        driver:connect(host, guest)
        driver:send(host, { kind = "lock" })
        driver:pump(6)

        local guest_view = view(guest)
        t.eq(guest_view.phase, "terminal")
        t.eq(assert(guest_view.terminal).reason, "manifest_mismatch")
        t.is_true(assert(guest_view.terminal).detail:find("content_id", 1, true) ~= nil)
    end)
end)
