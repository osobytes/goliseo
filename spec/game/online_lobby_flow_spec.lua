-- Scripted multi-peer lobby flows over the in-process fake star. Every peer
-- here runs the same pure lobby model the screen runs, exchanges real canonical
-- control wires through the real framing, and completes the real manual
-- offer/answer handshake. No browser, no JavaScript, no display.

local coordinator = require("game.online.coordinator")
local lobby_link = require("game.online.lobby_link")
local lobby_model = require("game.screens.lobby_model")
local match_manifest = require("game.online.match_manifest")
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
---@param template (fun(mode: SessionMatchMode): SessionManifest)? -- Defaults to the model's.
---@return LobbyTestPeer
function Driver:add(role, peer_id, template)
    ---@type LobbyTestPeer
    local peer = {
        id = peer_id,
        model = lobby_model.new({ peer_id = peer_id, template = template }),
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

    -- The framing is delimiter-safe by construction: chunks are cut with
    -- `string.sub` on the wire and the header is matched with an anchored
    -- pattern, so nothing ever scans inside a payload for a separator. Pin it,
    -- because that is the invariant a future "optimization" would break.
    t.it("carries payloads full of its own delimiters unchanged", function()
        local wire = string.rep("GCLF;1;9;9;|a;b|\nc\r\n;;;|", 120)
        t.is_true(#wire > lobby_link.MAX_CHUNK_BYTES, "the case must span several frames")
        local frames = assert(lobby_link.frame(wire))
        t.is_true(#frames > 1)
        local buffer = lobby_link.new_buffer()
        local rebuilt
        for _, frame in ipairs(frames) do
            rebuilt = lobby_link.absorb(buffer, frame)
        end
        t.eq(rebuilt, wire)
    end)

    t.it("carries a real control wire whose body holds delimiters", function()
        local message = assert(protocol.new("abort", "session_alpha", "host", 1, {
            code = "host_abort",
        }))
        local wire = assert(protocol.encode(message))
        local frames = assert(lobby_link.frame(wire))
        local buffer = lobby_link.new_buffer()
        local rebuilt
        for _, frame in ipairs(frames) do
            rebuilt = lobby_link.absorb(buffer, frame)
        end
        t.eq(rebuilt, wire)
        t.eq(assert(protocol.decode(rebuilt)).kind, "abort")
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

t.describe("lobby pair selection", function()
    ---@param peer LobbyTestPeer
    ---@param slot InputSlotId
    ---@return boolean
    local function offers_pair(peer, slot)
        for _, row in ipairs(view(peer).slots) do
            if row.slot == slot then
                return row.can_prefer
            end
        end
        return false
    end

    ---@param mode SessionMatchMode
    ---@param guest_count integer
    ---@return LobbyTestDriver, LobbyTestPeer, LobbyTestPeer[]
    local function locked_lobby(mode, guest_count)
        local driver, host, guests = seated_lobby(mode, guest_count)
        driver:send(host, { kind = "lock" })
        driver:pump(8)
        return driver, host, guests
    end

    t.it("lets a guest choose its pair and shows the request through to the grant", function()
        local driver, host, guests = locked_lobby("2v2", 3)
        local chooser = guests[2]
        t.eq(table.concat(owned(chooser, "guest_2"), ","), "away_1,away_2")
        t.is_true(offers_pair(chooser, "away_3"), "a 2v2 guest must be able to ask")

        driver:send(chooser, { kind = "pair", slot = "away_3" })
        local pending = assert(view(chooser).preference, "the request must be visible at once")
        t.eq(pending.status, "pending")
        t.eq(table.concat(pending.slots, ","), "away_1,away_3")
        t.eq(pending.text, lobby_model.PREFERENCE_TEXT.pending)

        driver:pump(8)
        local granted = assert(view(chooser).preference)
        t.eq(granted.status, "granted")
        t.eq(granted.text, lobby_model.PREFERENCE_TEXT.granted)
        t.eq(table.concat(owned(chooser, "guest_2"), ","), "away_1,away_3")
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_1,away_3")
        t.eq(table.concat(owned(host, "guest_3"), ","), "away_2,away_4")
        t.is_true(not view(host).ready, "a granted pair clears readiness like any repartition")
    end)

    t.it("keeps a granted pair through the traffic that follows it", function()
        local driver, host, guests = locked_lobby("2v2", 3)
        driver:send(guests[2], { kind = "pair", slot = "away_3" })
        driver:pump(8)
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_1,away_3")

        -- Ordinary control traffic runs the host's publish path again; it must
        -- not quietly restore the seating plan over the granted pair.
        for _, peer in ipairs({ host, guests[1], guests[2], guests[3] }) do
            driver:send(peer, { kind = "ready", ready = true })
        end
        driver:pump(8)
        t.eq(view(host).phase, "ready")
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_1,away_3")
        t.eq(table.concat(owned(guests[2], "guest_2"), ","), "away_1,away_3")
    end)

    t.it("shows the typed reason when the host refuses", function()
        local driver, host, guests = locked_lobby("2v2", 3)
        driver:send(guests[2], { kind = "pair", slot = "away_3" })
        driver:pump(8)

        -- `away_3` is now part of a pair its owner chose, so it is not up for
        -- exchange any more.
        driver:send(guests[3], { kind = "pair", slot = "away_3" })
        driver:pump(8)
        local refused = assert(view(guests[3]).preference)
        t.eq(refused.status, "rejected")
        t.eq(refused.reason, "already_taken")
        t.eq(refused.text, lobby_model.PREFERENCE_TEXT.already_taken)
        t.eq(
            table.concat(owned(guests[3], "guest_3"), ","),
            "away_2,away_4",
            "a refusal leaves ownership exactly where it was"
        )
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_1,away_3")
    end)

    -- Every canonical slot has exactly one producer, and no human holds more or
    -- fewer slots than the mode seats. A roster change reseats humans, so this
    -- is asserted on the far side of one.
    ---@param peer LobbyTestPeer
    ---@param mode SessionMatchMode
    local function assert_partition(peer, mode)
        local shape = assert(protocol.MATCH_MODES[mode])
        ---@type table<string, integer>
        local counts = {}
        local rows = view(peer).slots
        t.eq(#rows, 8, "a partition covers every canonical slot")
        for _, row in ipairs(rows) do
            local owner = assert(row.owner, row.slot .. " has no producer")
            counts[owner] = (counts[owner] or 0) + 1
        end
        for _, seat in ipairs(view(peer).seats) do
            t.eq(
                counts[seat.peer_id] or 0,
                shape.slots_per_human,
                seat.peer_id .. " owns the wrong number of slots"
            )
        end
    end

    t.it("keeps the pair a roster change still fits and says why the other went", function()
        local driver, host, guests = locked_lobby("2v2", 3)
        -- `guest_1` refines its pair inside the home line; `guest_2` reaches
        -- into the far half of the away line for `away_3`.
        driver:send(guests[1], { kind = "pair", slot = "home_2" })
        driver:pump(8)
        driver:send(guests[2], { kind = "pair", slot = "away_3" })
        driver:pump(8)
        t.eq(table.concat(owned(host, "guest_1"), ","), "home_2,home_3")
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_1,away_3")

        -- The roster changes. A three-human 2v2 seats nobody on `away_3`, so
        -- one of the two granted pairs cannot come through this.
        driver:send(guests[3], { kind = "leave" })
        driver:pump(8)

        t.eq(table.concat(owned(host, "guest_1"), ","), "home_2,home_3")
        t.eq(table.concat(owned(guests[1], "guest_1"), ","), "home_2,home_3")
        t.eq(
            assert(view(guests[1]).preference).status,
            "granted",
            "a pair that survived is still the pair this guest was given"
        )

        local dropped = assert(view(guests[2]).preference, "a dropped pair must still be shown")
        t.eq(dropped.status, "rejected")
        t.eq(dropped.reason, "reseated")
        t.eq(dropped.text, lobby_model.PREFERENCE_TEXT.reseated)
        t.eq(table.concat(owned(guests[2], "guest_2"), ","), "away_1,away_2")
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_1,away_2")
        assert_partition(host, "2v2")
        assert_partition(guests[1], "2v2")
        assert_partition(guests[2], "2v2")
        t.is_true(not view(host).ready, "a reseat clears readiness like any repartition")
    end)

    -- `SWAP` deliberately outranks a guest's choice, and always has. What it
    -- must not do is leave that guest reading "the host gave you the pair you
    -- asked for" over an ownership that no longer seats it.
    t.it("tells a guest when the host's swap took its pair back", function()
        local driver, host, guests = locked_lobby("2v2", 3)
        driver:send(guests[2], { kind = "pair", slot = "away_3" })
        driver:pump(8)
        t.eq(assert(view(guests[2]).preference).status, "granted")
        t.eq(table.concat(owned(guests[2], "guest_2"), ","), "away_1,away_3")

        -- Seats 3 and 4 are the two away humans, so this moves `guest_2` off
        -- the pair it chose without the roster changing at all.
        driver:send(host, { kind = "swap", index = 3 })
        driver:pump(8)

        local taken = assert(view(guests[2]).preference, "a swapped-away pair must still be shown")
        t.eq(taken.status, "rejected")
        t.eq(taken.reason, "reseated", "the reason is read off the ownership, not off the cause")
        t.eq(taken.text, lobby_model.PREFERENCE_TEXT.reseated)
        t.eq(table.concat(owned(guests[2], "guest_2"), ","), "away_3,away_4")
        t.eq(table.concat(owned(host, "guest_2"), ","), "away_3,away_4")
        assert_partition(host, "2v2")
        assert_partition(guests[2], "2v2")
    end)

    t.it("offers nothing to choose in 1v1 or 4v4", function()
        for _, case in ipairs({ { mode = "1v1", guests = 1 }, { mode = "4v4", guests = 7 } }) do
            local driver, host = locked_lobby(case.mode, case.guests)
            for _, row in ipairs(view(host).slots) do
                t.is_true(
                    not row.can_prefer,
                    ("%s must offer no pair control on %s"):format(case.mode, row.slot)
                )
            end
            driver:send(host, { kind = "pair", slot = "home_4" })
            t.is_true(view(host).error ~= nil, case.mode .. " must refuse a pair request")
            t.eq(view(host).preference, nil, "nothing was asked, so nothing is shown")
        end
    end)

    t.it("stops waiting in plain language when the host never answers", function()
        local driver, host, guests = locked_lobby("2v2", 3)
        local chooser = guests[2]
        local before = table.concat(owned(chooser, "guest_2"), ",")
        -- The host is up and its link is open; it has simply stopped being
        -- serviced, so it never reads the request and never replies. Nothing is
        -- closed, so neither end takes the `transport_lost` path.
        for index, peer in ipairs(driver.peers) do
            if peer == host then
                table.remove(driver.peers, index)
                break
            end
        end

        driver:send(chooser, { kind = "pair", slot = "away_3" })
        t.eq(assert(view(chooser).preference).status, "pending")
        driver:tick(coordinator.PREFERENCE_TIMEOUT_TICKS + 1)

        local given_up = assert(view(chooser).preference)
        t.eq(given_up.status, "rejected")
        t.eq(given_up.reason, "no_response")
        t.eq(given_up.text, lobby_model.PREFERENCE_TEXT.no_response)
        t.eq(table.concat(given_up.slots, ","), "away_1,away_3", "the request stays legible")
        t.eq(table.concat(owned(chooser, "guest_2"), ","), before, "nothing moved")
        t.eq(view(chooser).terminal, nil, "a silent host does not end the session")
        t.eq(view(chooser).phase, "assigned")
    end)

    -- An outcome with no text renders as its bare enum name, which is a defect
    -- rather than a message. Derived from the protocol's own closed vocabulary
    -- so a reason added there without lobby text fails here.
    t.it("has plain language for every outcome a request can end on", function()
        ---@type table<string, boolean>
        local reachable = { pending = true }
        for status in pairs(protocol.PREFERENCE_STATUSES) do
            -- A refusal is shown by its typed reason, never by the bare status.
            if status ~= "rejected" then
                reachable[status] = true
            end
        end
        for reason in pairs(protocol.PREFERENCE_REJECTIONS) do
            reachable[reason] = true
        end
        for key in pairs(reachable) do
            local text = lobby_model.PREFERENCE_TEXT[key]
            t.is_true(type(text) == "string" and #text > 0, "no lobby text for " .. key)
        end
        for key in pairs(lobby_model.PREFERENCE_TEXT) do
            t.is_true(reachable[key] == true, "lobby text for an unreachable outcome: " .. key)
        end
    end)
end)

t.describe("lobby build skew", function()
    -- These flows use the content-derived template the shipped app injects,
    -- not the lobby's fixture default, because `build_id` is the identity
    -- under test and only that template computes one.

    -- The manifest a peer would propose if it had been built from a commit
    -- whose control vocabulary differs from this one's. Everything
    -- `game/build_info.lua` knows — name, version, channel — is identical, so
    -- this is exactly the pair of builds the old digest could not tell apart.
    ---@param mode SessionMatchMode
    ---@return SessionManifest
    local function foreign_template(mode)
        local original = protocol.vocabulary_id
        protocol.vocabulary_id = function()
            return original() .. "0"
        end
        local ok, manifest = pcall(match_manifest.template, mode)
        protocol.vocabulary_id = original
        assert(ok, tostring(manifest))
        return manifest
    end

    -- The lobby already prints `BUILD` beside the manifest, so the value under
    -- test is one a tester can read off two screens and compare.
    ---@param peer LobbyTestPeer
    ---@return string
    local function build_row(peer)
        for _, row in ipairs(view(peer).identity) do
            if row.label == "BUILD" then
                return row.value
            end
        end
        error("the lobby stopped showing a build identity")
    end

    t.it("ends a same-version, different-vocabulary session at the manifest check", function()
        local driver = new_driver()
        local host = driver:add("host", "host", foreign_template)
        driver:send(host, { kind = "mode", mode = "1v1" })
        local guest = driver:add("guest", "guest_1", match_manifest.template)
        t.is_true(
            build_row(host) ~= build_row(guest),
            "the two builds have to be distinguishable before either speaks"
        )
        driver:connect(host, guest)
        driver:send(host, { kind = "lock" })
        driver:pump(6)

        local state = assert(guest.model.coordinator)
        t.eq(state.phase, "terminal")
        local terminal = assert(state.terminal)
        t.eq(terminal.reason, "build_mismatch")
        t.eq(terminal.detail, "local identity differs at manifest.build_id")
        t.eq(
            view(guest).terminal_text,
            "The peers are running different builds. Install the same build on both."
        )

        -- At the manifest check means before anything a tester would read as
        -- progress: no ownership was ever published to this guest, so it never
        -- saw a seat, let alone a countdown.
        t.eq(state.assignment_id, nil)
        t.eq(guest.freeze, nil)

        -- The refusal is announced, not silent, and the host's answer to it is
        -- the one #163 already chose: before the freeze a guest's abort drops
        -- that guest and leaves the lobby standing. So the host stops waiting
        -- on a peer that will never ready, and one skewed guest cannot end the
        -- session for everyone else.
        driver:pump(8)
        local host_state = assert(host.model.coordinator)
        t.eq(host_state.terminal, nil, "one skewed guest must not end the host's lobby")
        t.eq(#host_state.peers, 1, "the host dropped the peer it could not play with")
        t.eq(host.freeze, nil)

        -- And now the host is told why. The lobby it is left holding cannot be
        -- filled from that device, and its own screen says the same thing the
        -- guest's already did -- which is what makes a two-device test
        -- diagnosable from either screen instead of only one.
        local departure = assert(host_state.departure, "the host was told nothing")
        t.eq(departure.reason, "build_mismatch")
        t.eq(departure.peer_id, "guest_1")
        t.eq(departure.code, "protocol_error", "the wire code is the one it always was")
        t.eq(
            view(host).departure_text,
            "A guest was dropped: it is running a different build. "
                .. "Install the same build on both."
        )
        t.eq(view(host).terminal_text, nil, "the host's lobby is not over")
    end)

    t.it("says only that a guest went when the two builds agree", function()
        -- The no-false-positive half. Both peers are this build; the guest is
        -- dropped for something that has nothing to do with builds, and the
        -- host must not name one.
        local driver = new_driver()
        local host = driver:add("host", "host", match_manifest.template)
        driver:send(host, { kind = "mode", mode = "1v1" })
        local guest = driver:add("guest", "guest_1", match_manifest.template)
        t.eq(build_row(host), build_row(guest))
        driver:connect(host, guest)
        driver:send(host, { kind = "lock" })
        driver:pump(6)
        t.eq(view(host).departure, nil, "a healthy lobby has no departure notice")

        driver:send(guest, { kind = "leave" })
        driver:pump(6)
        local host_state = assert(host.model.coordinator)
        local departure = assert(host_state.departure, "the host lost a guest and said nothing")
        t.eq(departure.reason, "guest_left")
        t.eq(view(host).departure_text, "A guest left the lobby.")
        t.eq(host_state.terminal, nil)
    end)

    t.it("has host-side language for every reason a drop can carry", function()
        -- A departure with no text renders as its bare enum name, which is a
        -- defect rather than a message.
        for reason, text in pairs(lobby_model.DEPARTURE_TEXT) do
            t.is_true(type(text) == "string" and #text > 0, "no departure text for " .. reason)
            t.is_true(
                lobby_model.TERMINAL_TEXT[reason] ~= nil,
                "a departure reason has to be a coordinator reason: " .. reason
            )
        end
        -- Every wire disconnect code a drop can carry lands on a reason, and
        -- every one of those reasons has a sentence: a code added to the
        -- protocol without host-side language fails here rather than rendering
        -- as its bare enum name on a tester's screen.
        for code, reason in pairs(coordinator.DISCONNECT_REASONS) do
            t.is_true(
                lobby_model.DEPARTURE_TEXT[reason] ~= nil,
                "no host-side sentence for " .. code .. " -> " .. reason
            )
        end
        t.is_true(
            lobby_model.DEPARTURE_TEXT.build_mismatch ~= nil,
            "the reason this table exists is the one that must be in it"
        )
    end)

    t.it("leaves peers on the same vocabulary exactly as compatible as before", function()
        local driver = new_driver()
        local host = driver:add("host", "host", match_manifest.template)
        driver:send(host, { kind = "mode", mode = "1v1" })
        local guest = driver:add("guest", "guest_1", match_manifest.template)
        t.eq(build_row(host), build_row(guest), "one vocabulary is one build identity")
        driver:connect(host, guest)
        driver:send(host, { kind = "lock" })
        driver:pump(6)

        t.eq(view(guest).phase, "assigned")
        driver:send(host, { kind = "ready", ready = true })
        driver:send(guest, { kind = "ready", ready = true })
        driver:pump(4)
        driver:send(host, { kind = "start" })
        driver:pump(2)
        driver:tick(lobby_model.COUNTDOWN_TICKS + 4)

        local freeze = assert(host.freeze, "the host never reached the start boundary")
        local guest_freeze = assert(guest.freeze, "the guest never reached the start boundary")
        t.eq(guest_freeze.manifest_id, freeze.manifest_id)
        t.eq(assert(host.model.coordinator).terminal, nil)
        t.eq(assert(guest.model.coordinator).terminal, nil)
        t.eq(view(host).departure, nil, "matching builds must produce no drop at all")
        t.eq(view(host).departure_text, nil)
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
