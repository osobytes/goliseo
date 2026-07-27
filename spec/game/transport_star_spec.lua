local contract = require("game.transport.contract")
local t = require("spec.support.runner")
local transport = require("game.transport")

---@param seq integer
---@param payload string?
---@return TransportMessage
local function control(seq, payload)
    return assert(contract.new({ type = "event", seq = seq, payload = payload or "" }))
end

---@param seq integer
---@param tick integer
---@param payload string?
---@return TransportMessage
local function input(seq, tick, payload)
    return assert(contract.new({ type = "input", seq = seq, tick = tick, payload = payload or "" }))
end

---@param count integer
---@param options FakeStarTransportOptions?
---@return FakeStarTransport, FakeStarTransport[]
local function build_star(count, options)
    options = options or {}
    local host = transport.fake_star({
        queue_limit = options.queue_limit,
        buffered_amount_limit = options.buffered_amount_limit,
    })
    assert(host:initialize())
    local guests = {}
    for index = 1, count do
        local peer_id = "guest_" .. index
        assert(host:open_peer(peer_id))
        local guest = transport.fake_star({
            role = "guest",
            peer_id = peer_id,
            queue_limit = options.queue_limit,
            buffered_amount_limit = options.buffered_amount_limit,
        })
        assert(guest:initialize())
        assert(host:link(guest))
        guests[index] = guest
    end
    return host, guests
end

---@param star FakeStarTransport
---@param peer_id string
---@return TransportPeerDiagnostics
local function peer_diagnostics(star, peer_id)
    for _, peer in ipairs(star:diagnostics().peers) do
        if peer.peer_id == peer_id then
            return peer
        end
    end
    error("no diagnostics for peer " .. peer_id)
end

t.describe("star transport contract", function()
    t.it("round-trips a peer-addressed envelope", function()
        local message = input(3, 90, "left|right\n%\255")
        local line = assert(contract.encode_addressed("guest_4", "input", message))
        local addressed = assert(contract.decode_addressed(line))
        t.eq(addressed.peer_id, "guest_4")
        t.eq(addressed.channel, "input")
        t.eq(addressed.message.tick, 90)
        t.eq(addressed.message.payload, message.payload)
    end)

    t.it("pins the control and input channel configuration", function()
        t.eq(contract.CHANNEL_CONFIG.control.ordered, true)
        t.eq(contract.CHANNEL_CONFIG.control.reliable, true)
        t.eq(contract.CHANNEL_CONFIG.input.ordered, false)
        t.eq(contract.CHANNEL_CONFIG.input.max_retransmits, 0)
        t.eq(contract.MAX_GUESTS, 7)
    end)

    t.it("rejects invalid peer ids, channels, and channel/type pairings", function()
        t.is_true(contract.validate_peer_id("guest_1") == true)
        local _, _, spaced = contract.validate_peer_id("guest 1")
        t.eq(spaced, "malformed")
        local _, _, piped = contract.validate_peer_id("guest|1")
        t.eq(piped, "malformed")
        local _, _, empty = contract.validate_peer_id("")
        t.eq(empty, "malformed")

        local _, _, unknown = contract.validate_channel("gossip")
        t.eq(unknown, "channel_mismatch")

        local _, _, lossy = contract.validate_channel_message("input", control(1))
        t.eq(lossy, "channel_mismatch")
        local _, _, reliable = contract.validate_channel_message("control", input(1, 2))
        t.eq(reliable, "channel_mismatch")
    end)

    t.it("round-trips the star diagnostics record", function()
        local host = build_star(2)
        assert(host:send("guest_1", "control", control(0, "hello")))
        local original = host:diagnostics()
        local decoded =
            assert(contract.decode_star_diagnostics(contract.encode_star_diagnostics(original)))
        t.eq(decoded.role, original.role)
        t.eq(decoded.capacity, original.capacity)
        t.eq(decoded.peer_count, 2)
        t.eq(#decoded.peers, 2)
        t.eq(decoded.peers[1].peer_id, "guest_1")
        t.eq(decoded.peers[1].slot, 1)
        t.eq(decoded.peers[1].control.outbound_depth, original.peers[1].control.outbound_depth)
        t.eq(decoded.peers[2].state, "connected")
    end)

    t.it("reports a malformed bridge diagnostics payload instead of throwing", function()
        local broken, _, code = contract.decode_star_diagnostics("star|host|connected")
        t.is_true(broken == nil)
        t.eq(code, "bridge_error")
    end)
end)

t.describe("fake host-star transport", function()
    t.it("holds one host and seven independently addressed guests", function()
        local host, guests = build_star(7)
        t.eq(host:capacity(), 7)
        t.eq(#host:peer_ids(), 7)
        for index = 1, 7 do
            t.eq(host:peer_state("guest_" .. index), "connected")
            t.eq(guests[index]:peer_state(contract.HOST_PEER_ID), "connected")
        end

        assert(host:send("guest_3", "control", control(0, "only-three")))
        host:pump()
        local addressed = assert(guests[3]:poll())
        t.eq(addressed.peer_id, contract.HOST_PEER_ID)
        t.eq(addressed.channel, "control")
        t.eq(addressed.message.payload, "only-three")
        for index = 1, 7 do
            if index ~= 3 then
                t.is_true(guests[index]:poll() == nil)
            end
        end
    end)

    t.it("rejects an eighth guest, a duplicate id, and the reserved host id", function()
        local host = build_star(7)
        local overflow, _, capacity = host:open_peer("guest_8")
        t.is_true(overflow == nil)
        t.eq(capacity, "capacity")
        local duplicate, _, duplicate_code = host:open_peer("guest_1")
        t.is_true(duplicate == nil)
        t.eq(duplicate_code, "duplicate_peer")
        local reserved, _, reserved_code = host:open_peer(contract.HOST_PEER_ID)
        t.is_true(reserved == nil)
        t.eq(reserved_code, "duplicate_peer")
        t.eq(#host:peer_ids(), 7)
    end)

    t.it("fans a canonical batch to every connected guest exactly once", function()
        local host, guests = build_star(7)
        t.eq(assert(host:broadcast("input", input(1, 120, "batch"))), 7)
        host:pump()
        for index = 1, 7 do
            local addressed = assert(guests[index]:poll())
            t.eq(addressed.message.tick, 120)
            t.is_true(guests[index]:poll() == nil)
        end
    end)

    t.it("attributes guest traffic by link identity, never by payload", function()
        local host, guests = build_star(3)
        for index = 1, 3 do
            -- Every guest claims to be guest_1 in its payload; the star must
            -- report the link it actually arrived on.
            assert(guests[index]:send(contract.HOST_PEER_ID, "input", input(0, 10, "guest_1")))
            guests[index]:pump()
        end
        local seen = {}
        for _ = 1, 3 do
            local addressed = assert(host:poll())
            t.is_true(seen[addressed.peer_id] == nil)
            seen[addressed.peer_id] = true
        end
        t.is_true(seen.guest_1 and seen.guest_2 and seen.guest_3)
        t.is_true(host:poll() == nil)
    end)

    t.it("lets only the host fan out and only guests address the host", function()
        local host, guests = build_star(2)
        local guest = guests[1]

        local fanned, _, fan_code = guest:broadcast("input", input(0, 1, "x"))
        t.is_true(fanned == nil)
        t.eq(fan_code, "role_forbidden")

        local crossed, _, crossed_code = guest:send("guest_2", "control", control(0))
        t.is_true(crossed == nil)
        t.eq(crossed_code, "role_forbidden")

        local opened, _, open_code = guest:open_peer("guest_9")
        t.is_true(opened == nil)
        t.eq(open_code, "role_forbidden")

        t.is_true(assert(host:send("guest_1", "control", control(0))))
    end)

    t.it("keeps input off the reliable channel and control off the lossy channel", function()
        local host = build_star(1)
        local reliable, _, reliable_code = host:send("guest_1", "control", input(0, 5))
        t.is_true(reliable == nil)
        t.eq(reliable_code, "channel_mismatch")

        local lossy, _, lossy_code = host:send("guest_1", "input", control(0))
        t.is_true(lossy == nil)
        t.eq(lossy_code, "channel_mismatch")

        t.eq(host:diagnostics().malformed, 2)
        t.eq(peer_diagnostics(host, "guest_1").malformed, 2)
    end)

    t.it("reports an unknown peer without throwing", function()
        local host = build_star(1)
        local sent, _, code = host:send("nobody", "control", control(0))
        t.is_true(sent == nil)
        t.eq(code, "unknown_peer")
    end)

    t.it("bounds the outbound queue and drops the overflow", function()
        local host = build_star(1, { queue_limit = 4 })
        for seq = 0, 3 do
            t.is_true(assert(host:send("guest_1", "control", control(seq))))
        end
        local overflowed, _, code = host:send("guest_1", "control", control(4))
        t.is_true(overflowed == nil)
        t.eq(code, "overflow")

        local diagnostics = host:diagnostics()
        t.eq(diagnostics.peers[1].control.outbound_depth, 4)
        t.eq(diagnostics.peers[1].control.dropped_outbound, 1)
        t.eq(diagnostics.dropped_outbound, 1)
        t.eq(diagnostics.overflow, 1)
    end)

    t.it("bounds the inbound queue when the peer never polls", function()
        local host, guests = build_star(1, { queue_limit = 3 })
        for seq = 0, 2 do
            assert(host:send("guest_1", "input", input(seq, 100 + seq)))
        end
        host:pump()
        assert(host:send("guest_1", "input", input(3, 103)))
        host:pump()

        local diagnostics = guests[1]:diagnostics()
        t.eq(diagnostics.peers[1].input.inbound_depth, 3)
        t.eq(diagnostics.peers[1].input.dropped_inbound, 1)
        t.is_true(diagnostics.overflow >= 1)
    end)

    t.it("queues instead of blocking when the send buffer is saturated", function()
        local host = build_star(1, { queue_limit = 8, buffered_amount_limit = 1 })
        t.is_true(assert(host:send("guest_1", "control", control(0, "payload"))))
        t.is_true(peer_diagnostics(host, "guest_1").control.buffered_amount > 0)

        host:pump()
        local diagnostics = host:diagnostics()
        t.eq(diagnostics.peers[1].control.outbound_depth, 1)
        t.eq(diagnostics.backpressure, 1)
        t.eq(diagnostics.peers[1].backpressure, 1)

        -- The saturated buffer costs one event, not one per drain pass.
        host:pump()
        host:pump()
        t.eq(host:diagnostics().backpressure, 1)

        local backpressure = nil
        local event = host:poll_event()
        while event do
            if event.code == "backpressure" then
                backpressure = event
            end
            event = host:poll_event()
        end
        t.eq(assert(backpressure).kind, "peer_error")
        t.eq(assert(backpressure).peer_id, "guest_1")
        t.eq(assert(backpressure).channel, "control")
    end)

    t.it("re-arms the backpressure latch after the channel drains", function()
        -- A budget that fits some but not all of a burst, so congestion can be
        -- relieved and then genuinely recur. A latch that never re-armed would
        -- hide the second congestion entirely.
        local host, guests = build_star(1, { queue_limit = 16, buffered_amount_limit = 40 })
        local function drain_events()
            local codes = {}
            local event = host:poll_event()
            while event do
                codes[#codes + 1] = event.code
                event = host:poll_event()
            end
            local count = 0
            for _, code in ipairs(codes) do
                if code == "backpressure" then
                    count = count + 1
                end
            end
            return count
        end
        drain_events()

        for seq = 0, 2 do
            assert(host:send("guest_1", "control", control(seq, "payload")))
        end
        host:pump()
        local congested = host:diagnostics()
        t.is_true(congested.peers[1].control.outbound_depth > 0)
        t.eq(congested.backpressure, 1)
        t.eq(drain_events(), 1)

        -- Relieve: the next pass has a fresh budget and empties the queue.
        host:pump()
        local relieved = host:diagnostics()
        t.eq(relieved.peers[1].control.outbound_depth, 0)
        t.eq(relieved.backpressure, 1)
        t.eq(drain_events(), 0)
        -- Everything the host queued actually arrived.
        t.eq(#guests[1]:poll_batch(8), 3)

        -- Re-saturate: a genuine second congestion is reported again.
        for seq = 3, 5 do
            assert(host:send("guest_1", "control", control(seq, "payload")))
        end
        host:pump()
        t.eq(host:diagnostics().backpressure, 2)
        t.eq(drain_events(), 1)
    end)

    t.it("drains peers and channels in a fixed order without starving a peer", function()
        local host, guests = build_star(3)
        for index = 1, 3 do
            assert(guests[index]:send(contract.HOST_PEER_ID, "control", control(0, "c")))
            assert(guests[index]:send(contract.HOST_PEER_ID, "input", input(0, 7, "i")))
            assert(guests[index]:send(contract.HOST_PEER_ID, "input", input(1, 8, "i")))
            guests[index]:pump()
        end

        local batch = host:poll_batch(6)
        t.eq(#batch, 6)
        local expected = {
            { "guest_1", "control" },
            { "guest_1", "input" },
            { "guest_2", "control" },
            { "guest_2", "input" },
            { "guest_3", "control" },
            { "guest_3", "input" },
        }
        for index, entry in ipairs(batch) do
            t.eq(entry.peer_id, expected[index][1])
            t.eq(entry.channel, expected[index][2])
        end
        t.eq(batch[1].arrival_seq, 1)

        -- The cursor persists, so the second pass resumes the same rotation.
        local rest = host:poll_batch(8)
        t.eq(#rest, 3)
        for _, entry in ipairs(rest) do
            t.eq(entry.channel, "input")
        end
    end)

    t.it("reports per-peer sequence gaps", function()
        local host, guests = build_star(1)
        assert(host:send("guest_1", "input", input(0, 10)))
        assert(host:send("guest_1", "input", input(5, 15)))
        host:pump()
        t.eq(peer_diagnostics(guests[1], contract.HOST_PEER_ID).sequence_gaps, 4)
    end)

    t.it("closes one link without disturbing the others", function()
        local host, guests = build_star(3)
        assert(host:send("guest_2", "control", control(0)))
        assert(host:close_peer("guest_2", "peer left"))

        t.eq(host:peer_state("guest_2"), "closed")
        t.eq(host:peer_state("guest_1"), "connected")
        t.eq(host:peer_state("guest_3"), "connected")
        t.eq(guests[2]:peer_state(contract.HOST_PEER_ID), "disconnected")

        local blocked, _, code = host:send("guest_2", "control", control(1))
        t.is_true(blocked == nil)
        t.eq(code, "not_connected")
        t.is_true(assert(host:send("guest_1", "control", control(1))))
        t.eq(assert(host:broadcast("input", input(2, 30))), 2)

        t.eq(peer_diagnostics(host, "guest_2").control.outbound_depth, 0)
        t.is_true(assert(host:close_peer("guest_2")))
    end)

    t.it("tears down every link and survives a repeated shutdown", function()
        local host, guests = build_star(7)
        assert(host:broadcast("input", input(0, 5)))
        assert(host:shutdown())
        assert(host:shutdown())

        local diagnostics = host:diagnostics()
        t.eq(diagnostics.state, "closed")
        t.eq(diagnostics.peer_count, 0)
        t.eq(#diagnostics.peers, 0)

        local sent, _, code = host:send("guest_1", "control", control(0))
        t.is_true(sent == nil)
        t.eq(code, "closed")
        t.eq(#host:poll_batch(8), 0)
        for index = 1, 7 do
            t.eq(guests[index]:peer_state(contract.HOST_PEER_ID), "disconnected")
        end
    end)

    t.it("refuses traffic before initialize", function()
        local host = transport.fake_star()
        local sent, _, code = host:send("guest_1", "control", control(0))
        t.is_true(sent == nil)
        t.eq(code, "not_initialized")
    end)

    t.it("completes a manual offer/answer handshake in process", function()
        local host = transport.fake_star()
        assert(host:initialize())
        assert(host:open_peer("guest_1"))
        local guest = transport.fake_star({ role = "guest", peer_id = "guest_1" })
        assert(guest:initialize())

        t.is_true(host:take_signal("guest_1") == nil)
        t.is_true(assert(host:request_offer("guest_1")))
        local offer = assert(host:take_signal("guest_1"))
        t.is_true(host:take_signal("guest_1") == nil, "a signal is handed out once")

        t.is_true(assert(guest:accept_offer(offer)))
        local answer = assert(guest:take_signal(contract.HOST_PEER_ID))
        t.is_true(assert(host:accept_answer("guest_1", answer)))

        t.eq(host:peer_state("guest_1"), "connected")
        t.eq(guest:peer_state(contract.HOST_PEER_ID), "connected")
        assert(host:send("guest_1", "control", control(0, "after-handshake")))
        host:pump()
        t.eq(assert(guest:poll()).message.payload, "after-handshake")
    end)

    t.it("rejects signaling misuse with typed errors", function()
        local host = transport.fake_star()
        assert(host:initialize())
        assert(host:open_peer("guest_1"))
        local guest = transport.fake_star({ role = "guest", peer_id = "guest_1" })
        assert(guest:initialize())

        local guest_offer, _, guest_code = guest:request_offer("guest_1")
        t.is_true(guest_offer == nil)
        t.eq(guest_code, "role_forbidden")

        local host_accept, _, host_code = host:accept_offer("anything")
        t.is_true(host_accept == nil)
        t.eq(host_code, "role_forbidden")

        local unknown, _, unknown_code = host:request_offer("nobody")
        t.is_true(unknown == nil)
        t.eq(unknown_code, "unknown_peer")

        local bogus, _, bogus_code = guest:accept_offer("not-a-real-offer")
        t.is_true(bogus == nil)
        t.eq(bogus_code, "signal_error")

        assert(host:request_offer("guest_1"))
        local stale, _, stale_code = host:accept_answer("guest_1", "answer:offer:nonexistent")
        t.is_true(stale == nil)
        t.eq(stale_code, "signal_error")
    end)
end)

---@param host FakeStarTransport
---@return fun(command: string): string?, string?
local function star_bridge(host)
    ---@param value string
    ---@return string
    local function unescape(value)
        return (
            value:gsub("%%(%x%x)", function(hex)
                return string.char(tonumber(hex, 16))
            end)
        )
    end

    -- The real bridge percent-decodes quoted arguments, except for the
    -- addressed and broadcast wires, which it forwards verbatim. Mirror both.
    ---@param argument string
    ---@return string[], string[]
    local function arguments(argument)
        local values, raw = {}, {}
        for field in (argument .. ","):gmatch("([^,]*),") do
            local quoted = field:match("^'(.*)'$")
            raw[#raw + 1] = quoted or field
            values[#values + 1] = quoted and unescape(quoted) or field
        end
        return values, raw
    end

    ---@param value string
    ---@return string
    local function escape(value)
        return (
            value:gsub("([^%w%-%._~])", function(character)
                return ("%%%02X"):format(string.byte(character))
            end)
        )
    end

    ---@param code TransportErrorCode?
    ---@param err string?
    ---@return string
    local function bridge_error(code, err)
        return "error|" .. tostring(code) .. "|" .. escape(err or "error")
    end

    return function(command)
        local name, argument = command:match("^window%.GalacticCupStarTransport%.([%w_]+)%((.*)%)$")
        assert(name, "unexpected browser star command: " .. command)
        local args, raw = arguments(argument)
        if name == "initialize" then
            assert(host:initialize())
            return "star|connected"
        elseif name == "shutdown" then
            assert(host:shutdown())
            return "star|closed"
        elseif name == "open_peer" then
            local slot, err, code = host:open_peer(args[1])
            if not slot then
                return bridge_error(code, err)
            end
            return "slot|" .. slot
        elseif name == "close_peer" then
            local ok, err, code = host:close_peer(args[1], args[2])
            if not ok then
                return bridge_error(code, err)
            end
            return "ok"
        elseif name == "request_offer" then
            local ok, err, code = host:request_offer(args[1])
            if not ok then
                return bridge_error(code, err)
            end
            return "ok"
        elseif name == "accept_offer" then
            local ok, err, code = host:accept_offer(args[1])
            if not ok then
                return bridge_error(code, err)
            end
            return "ok"
        elseif name == "accept_answer" then
            local ok, err, code = host:accept_answer(args[1], args[2])
            if not ok then
                return bridge_error(code, err)
            end
            return "ok"
        elseif name == "take_signal" then
            local signal, err, code = host:take_signal(args[1])
            if not signal then
                if code then
                    return bridge_error(code, err)
                end
                return ""
            end
            return "signal|" .. escape(signal)
        elseif name == "send" then
            local addressed, decode_err, decode_code = contract.decode_addressed(raw[1])
            if not addressed then
                return bridge_error(decode_code, decode_err)
            end
            local ok, err, code = host:send(addressed.peer_id, addressed.channel, addressed.message)
            if not ok then
                return bridge_error(code, err)
            end
            -- The real bridge drains the data channel inside the same call.
            host:pump()
            return "ok"
        elseif name == "broadcast" then
            local channel, wire = raw[1]:match("^([^|]*)|(.*)$")
            local message = assert(contract.decode(wire))
            ---@cast channel TransportChannel
            local delivered, err, code = host:broadcast(channel, message)
            if not delivered then
                return bridge_error(code, err)
            end
            host:pump()
            return "delivered|" .. delivered
        elseif name == "poll" then
            host:pump()
            local entry = host:poll()
            if not entry then
                return ""
            end
            return assert(contract.encode_addressed(entry.peer_id, entry.channel, entry.message))
        elseif name == "poll_event" then
            local event = host:poll_event()
            if not event then
                return ""
            end
            if event.kind == "star_state" then
                return "star_state|" .. tostring(event.state)
            elseif event.kind == "peer_state" then
                return "peer_state|" .. tostring(event.peer_id) .. "|" .. tostring(event.state)
            elseif event.kind == "star_error" then
                return "star_error|" .. tostring(event.code) .. "|"
            end
            return table.concat({
                "peer_error",
                tostring(event.peer_id),
                event.channel or "",
                tostring(event.code),
                "",
            }, "|")
        elseif name == "diagnostics" then
            host:pump()
            return contract.encode_star_diagnostics(host:diagnostics())
        end
        error("unexpected browser star method: " .. name)
    end
end

t.describe("browser host-star transport", function()
    t.it("mirrors the fake adapter through the JavaScript host seam", function()
        local reference, guests = build_star(3)
        local browser = transport.browser_star({ eval = star_bridge(reference) })
        t.is_true(assert(browser:initialize()))
        t.eq(browser:role(), "host")
        t.eq(browser:capacity(), 7)

        t.is_true(assert(browser:send("guest_2", "control", control(0, "addressed"))))
        local addressed = assert(guests[2]:poll())
        t.eq(addressed.message.payload, "addressed")

        t.eq(assert(browser:broadcast("input", input(1, 44, "batch"))), 3)
        for index = 1, 3 do
            t.eq(assert(guests[index]:poll()).message.tick, 44)
            assert(guests[index]:send(contract.HOST_PEER_ID, "input", input(0, 45, "reply")))
            guests[index]:pump()
        end

        local batch = browser:poll_batch(3)
        t.eq(#batch, 3)
        t.eq(batch[1].peer_id, "guest_1")
        t.eq(batch[1].channel, "input")
        t.eq(batch[1].arrival_seq, 1)
        t.eq(batch[3].peer_id, "guest_3")

        local diagnostics = browser:diagnostics()
        t.eq(diagnostics.role, "host")
        t.eq(diagnostics.peer_count, 3)
        t.eq(diagnostics.capacity, 7)
        t.eq(#diagnostics.peers, 3)
        t.eq(diagnostics.peers[1].state, "connected")
        t.eq(browser:peer_state("guest_1"), "connected")
    end)

    t.it("surfaces bridge-reported failures as typed transport errors", function()
        local reference = build_star(7)
        local browser = transport.browser_star({ eval = star_bridge(reference) })
        assert(browser:initialize())

        local slot, _, capacity = browser:open_peer("guest_8")
        t.is_true(slot == nil)
        t.eq(capacity, "capacity")

        local mismatch, _, mismatch_code = browser:send("guest_1", "control", input(0, 3))
        t.is_true(mismatch == nil)
        t.eq(mismatch_code, "channel_mismatch")

        local unknown, _, unknown_code = browser:send("nobody", "control", control(0))
        t.is_true(unknown == nil)
        t.eq(unknown_code, "unknown_peer")
    end)

    t.it("enforces role permissions before reaching the bridge", function()
        local reference = build_star(2)
        local guest = transport.browser_star({ role = "guest", eval = star_bridge(reference) })
        assert(guest:initialize())
        t.eq(guest:capacity(), 1)

        local fanned, _, fan_code = guest:broadcast("input", input(0, 1))
        t.is_true(fanned == nil)
        t.eq(fan_code, "role_forbidden")

        local crossed, _, crossed_code = guest:send("guest_2", "control", control(0))
        t.is_true(crossed == nil)
        t.eq(crossed_code, "role_forbidden")

        local opened, _, open_code = guest:open_peer("guest_9")
        t.is_true(opened == nil)
        t.eq(open_code, "role_forbidden")
    end)

    t.it("reports a missing bridge instead of throwing", function()
        local browser = transport.browser_star({
            eval = function()
                return nil, "love.js.eval is not available"
            end,
        })
        local ok, _, code = browser:initialize()
        t.is_true(ok == nil)
        t.eq(code, "bridge_error")

        local fallback = browser:diagnostics()
        t.eq(fallback.state, "new")
        t.eq(fallback.peer_count, 0)
        t.is_true(fallback.last_error ~= nil)
    end)

    t.it("drives the manual handshake through the bridge", function()
        local reference = transport.fake_star()
        local browser = transport.browser_star({ eval = star_bridge(reference) })
        assert(browser:initialize())
        assert(browser:open_peer("guest_1"))

        t.is_true(browser:take_signal("guest_1") == nil)
        t.is_true(assert(browser:request_offer("guest_1")))
        local offer = assert(browser:take_signal("guest_1"))
        t.is_true(browser:take_signal("guest_1") == nil)

        -- The guest side runs against its own endpoint and bridge.
        local guest_reference = transport.fake_star({ role = "guest", peer_id = "guest_1" })
        local guest = transport.browser_star({
            role = "guest",
            eval = star_bridge(guest_reference),
        })
        assert(guest:initialize())
        t.is_true(assert(guest:accept_offer(offer)))
        local answer = assert(guest:take_signal(contract.HOST_PEER_ID))
        t.is_true(assert(browser:accept_answer("guest_1", answer)))
        t.eq(reference:peer_state("guest_1"), "connected")
    end)

    t.it("round-trips a signal blob with SDP characters through the eval seam", function()
        -- A real SDP blob carries newlines, quotes, equals signs, and commas.
        -- Everything crossing the eval seam has to survive them intact.
        local blob = '{"type":"offer","sdp":"v=0\r\na=candidate:1 1 udp 2 10.0.0.1 5,6\'\\"}'
        local captured = nil
        local browser = transport.browser_star({
            eval = function(command)
                local name, argument =
                    command:match("^window%.GalacticCupStarTransport%.([%w_]+)%((.*)%)$")
                if name == "initialize" then
                    return "star|connected"
                elseif name == "open_peer" then
                    return "slot|1"
                elseif name == "accept_answer" then
                    captured = argument
                    return "ok"
                elseif name == "take_signal" then
                    return "signal|"
                        .. (
                            blob:gsub("([^%w%-%._~])", function(character)
                                return ("%%%02X"):format(string.byte(character))
                            end)
                        )
                end
                error("unexpected command: " .. command)
            end,
        })
        assert(browser:initialize())
        assert(browser:open_peer("guest_1"))
        t.eq(assert(browser:take_signal("guest_1")), blob)

        assert(browser:accept_answer("guest_1", blob))
        -- The command the bridge received must contain no raw quote, comma, or
        -- newline that could terminate or extend the JavaScript call.
        local quoted = assert(captured):match("^'guest_1','(.*)'$")
        t.is_true(quoted ~= nil, "accept_answer did not quote both arguments")
        t.is_true(assert(quoted):find("[',\r\n\\]") == nil, "a raw delimiter escaped the encoder")
    end)

    t.it("keeps a wire payload injection-safe across the eval seam", function()
        local hostile = "');window.owned=1;('"
        local captured = nil
        local browser = transport.browser_star({
            eval = function(command)
                local name, argument =
                    command:match("^window%.GalacticCupStarTransport%.([%w_]+)%((.*)%)$")
                if name == "initialize" then
                    return "star|connected"
                elseif name == "open_peer" then
                    return "slot|1"
                elseif name == "send" or name == "broadcast" then
                    captured = argument
                    return "ok"
                end
                error("unexpected command: " .. command)
            end,
        })
        assert(browser:initialize())
        assert(browser:open_peer("guest_1"))
        assert(browser:send("guest_1", "control", control(0, hostile)))
        local wire = assert(captured):match("^'(.*)'$")
        t.is_true(wire ~= nil, "send did not quote its argument")
        t.is_true(assert(wire):find("[']") == nil, "a quote survived into the eval command")
        -- The payload still round-trips through the addressed decoder.
        local addressed = assert(contract.decode_addressed(assert(wire)))
        t.eq(addressed.message.payload, hostile)
    end)

    t.it("makes a locally rejected call as observable as a bridge rejection", function()
        -- The fake records role violations through its error path; the browser
        -- adapter rejects them before the bridge is reached, so it has to queue
        -- an equivalent event and own last_error rather than leaving whatever
        -- the bridge last said.
        local reference = build_star(2)
        local guest_reference = transport.fake_star({ role = "guest", peer_id = "guest_1" })
        local guest = transport.browser_star({
            role = "guest",
            eval = star_bridge(guest_reference),
        })
        assert(guest:initialize())

        local fanned, _, fan_code = guest:broadcast("input", input(0, 1))
        t.is_true(fanned == nil)
        t.eq(fan_code, "role_forbidden")

        local event = assert(guest:poll_event())
        t.eq(event.kind, "star_error")
        t.eq(event.code, "role_forbidden")
        t.is_true(assert(guest:diagnostics().last_error):find("fans out") ~= nil)

        -- The fake reports the same fault the same way.
        local fake_guest = transport.fake_star({ role = "guest", peer_id = "guest_2" })
        assert(fake_guest:initialize())
        local _, _, fake_code = fake_guest:broadcast("input", input(0, 1))
        t.eq(fake_code, "role_forbidden")
        local fake_event = nil
        local next_event = fake_guest:poll_event()
        while next_event do
            if next_event.code == "role_forbidden" then
                fake_event = next_event
            end
            next_event = fake_guest:poll_event()
        end
        t.eq(assert(fake_event).kind, "star_error")
        t.is_true(assert(fake_guest:diagnostics().last_error):find("fans out") ~= nil)
        t.is_true(reference ~= nil)
    end)

    t.it("judges message shape before peer resolution on both adapters", function()
        -- A call that is BOTH addressed to an unknown peer AND carries a
        -- message the channel cannot take must report the same code either way.
        -- Shape wins: only the bridge can authoritatively resolve a peer, so an
        -- adapter must never answer `unknown_peer` from a cached peer table.
        local fake = build_star(1)
        local _, _, fake_code = fake:send("nobody", "control", input(0, 3))
        t.eq(fake_code, "channel_mismatch")

        local reference = build_star(1)
        local browser = transport.browser_star({ eval = star_bridge(reference) })
        assert(browser:initialize())
        local _, _, browser_code = browser:send("nobody", "control", input(0, 3))
        t.eq(browser_code, "channel_mismatch")
        t.eq(browser_code, fake_code)

        -- A well-shaped message to an unknown peer still reports unknown_peer.
        local _, _, fake_unknown = fake:send("nobody", "control", control(0))
        t.eq(fake_unknown, "unknown_peer")
        local _, _, browser_unknown = browser:send("nobody", "control", control(0))
        t.eq(browser_unknown, "unknown_peer")
    end)

    t.it("numbers arrival_seq per peer at poll time on both adapters", function()
        local fake, fake_guests = build_star(2)
        for index = 1, 2 do
            assert(fake_guests[index]:send(contract.HOST_PEER_ID, "control", control(0, "a")))
            assert(fake_guests[index]:send(contract.HOST_PEER_ID, "input", input(0, 1, "b")))
            fake_guests[index]:pump()
        end
        local fake_batch = fake:poll_batch(4)

        local reference, guests = build_star(2)
        local browser = transport.browser_star({ eval = star_bridge(reference) })
        assert(browser:initialize())
        for index = 1, 2 do
            assert(guests[index]:send(contract.HOST_PEER_ID, "control", control(0, "a")))
            assert(guests[index]:send(contract.HOST_PEER_ID, "input", input(0, 1, "b")))
            guests[index]:pump()
        end
        local browser_batch = browser:poll_batch(4)

        t.eq(#fake_batch, 4)
        t.eq(#browser_batch, 4)
        for index = 1, 4 do
            t.eq(fake_batch[index].peer_id, browser_batch[index].peer_id)
            t.eq(fake_batch[index].channel, browser_batch[index].channel)
            t.eq(fake_batch[index].arrival_seq, browser_batch[index].arrival_seq)
        end
        -- Two peers, two messages each, numbered 1 and 2 within each peer.
        t.eq(fake_batch[1].arrival_seq, 1)
        t.eq(fake_batch[2].arrival_seq, 2)
        t.eq(fake_batch[3].arrival_seq, 1)
        t.eq(fake_batch[4].arrival_seq, 2)
    end)

    t.it("closes peers and tears the star down through the bridge", function()
        local reference = build_star(3)
        local browser = transport.browser_star({ eval = star_bridge(reference) })
        assert(browser:initialize())
        assert(browser:close_peer("guest_2", "peer left"))
        t.eq(browser:peer_state("guest_2"), "closed")

        assert(browser:shutdown())
        t.eq(browser:state(), "closed")
        assert(browser:shutdown())
        t.eq(#browser:peer_ids(), 0)

        local sent, _, code = browser:send("guest_1", "control", control(0))
        t.is_true(sent == nil)
        t.eq(code, "closed")
    end)
end)

t.describe("transport layering", function()
    t.it("keeps browser and WebRTC APIs out of core, data, and sim", function()
        local forbidden = {
            "love%.js%.eval",
            "RTCPeerConnection",
            "GalacticCup%w+",
            "RTCDataChannel",
        }
        ---@param dir string
        local function scan(dir)
            for _, item in ipairs(love.filesystem.getDirectoryItems(dir)) do
                local path = dir .. "/" .. item
                local info = love.filesystem.getInfo(path)
                if info and info.type == "directory" then
                    scan(path)
                elseif item:match("%.lua$") then
                    local source = assert(love.filesystem.read(path))
                    for _, pattern in ipairs(forbidden) do
                        t.is_true(
                            source:find(pattern) == nil,
                            path .. " must not reference " .. pattern
                        )
                    end
                    t.is_true(
                        source:find('require%("game%.') == nil,
                        path .. " must not require a game module"
                    )
                end
            end
        end
        scan("core")
        scan("data")
        scan("sim")
    end)
end)
