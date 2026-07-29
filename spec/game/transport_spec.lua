local contract = require("game.transport.contract")
local t = require("spec.support.runner")
local transport = require("game.transport")

---@param seq integer
---@param tick integer?
---@param payload string?
---@return TransportMessage
local function message(seq, tick, payload)
    return assert(contract.new({
        type = tick and "input" or "event",
        seq = seq,
        tick = tick,
        payload = payload or "",
    }))
end

---@param fake FakeTransport
---@return fun(command: string): string
local function fake_browser_host(fake)
    return function(command)
        local name, argument = command:match("^window%.GoliseoTransportBridge%.([%w_]+)%((.*)%)$")
        assert(name, "unexpected browser command: " .. command)
        if name == "initialize" then
            assert(fake:initialize())
            return "state|connected"
        elseif name == "shutdown" then
            assert(fake:shutdown())
            return "state|closed"
        elseif name == "enqueue" then
            local wire = assert(argument:match("^'(.*)'$"))
            local decoded = assert(contract.decode(wire))
            local ok, err, code = fake:enqueue(decoded)
            if ok then
                return "ok"
            end
            return "error|" .. tostring(code) .. "|" .. tostring(err)
        elseif name == "poll" then
            local value = fake:poll()
            return value and assert(contract.encode(value)) or ""
        elseif name == "poll_event" then
            local event = fake:poll_event()
            if not event then
                return ""
            end
            if event.kind == "state" then
                return "state|" .. assert(event.state)
            end
            return "error|" .. assert(event.code)
        elseif name == "disconnect" then
            fake:disconnect()
            return "state|disconnected"
        elseif name == "diagnostics" then
            local d = fake:diagnostics()
            return table.concat({
                d.state,
                d.queue_limit,
                d.outbound_depth,
                d.inbound_depth,
                d.event_depth,
                d.dropped_outbound,
                d.dropped_inbound,
                d.malformed,
                d.unsupported_version,
                d.overflow,
                d.sent,
                d.received,
                d.last_error or "",
            }, "|")
        end
        error("unexpected browser method: " .. name)
    end
end

t.describe("transport envelope", function()
    t.it("validates and round-trips a tick-numbered input", function()
        local original = message(7, 42, "left|right\n%\255")
        local wire = assert(contract.encode(original))
        local decoded = assert(contract.decode(wire))
        t.eq(decoded.version, 1)
        t.eq(decoded.type, "input")
        t.eq(decoded.seq, 7)
        t.eq(decoded.tick, 42)
        t.eq(decoded.payload, original.payload)
    end)

    t.it("rejects malformed, unsupported, and oversized messages", function()
        local malformed, _, malformed_code = contract.decode("1|input|not-a-seq|2|payload")
        t.is_true(malformed == nil)
        t.eq(malformed_code, "malformed")

        local unsupported, _, unsupported_code = contract.new({
            version = 2,
            type = "input",
            seq = 1,
            tick = 1,
            payload = "payload",
        })
        t.is_true(unsupported == nil)
        t.eq(unsupported_code, "unsupported_version")

        local oversized, _, oversized_code = contract.new({
            type = "event",
            seq = 1,
            payload = string.rep("x", contract.MAX_PAYLOAD_BYTES + 1),
        })
        t.is_true(oversized == nil)
        t.eq(oversized_code, "payload_too_large")
    end)

    t.it("decodes a control envelope that carries no tick", function()
        local original = assert(contract.new({ type = "event", seq = 4, payload = "lifecycle" }))
        local decoded = assert(contract.decode(assert(contract.encode(original))))
        t.eq(decoded.type, "event")
        t.eq(decoded.seq, 4)
        t.eq(decoded.tick, nil)
        t.eq(decoded.payload, "lifecycle")
    end)

    t.it("requires ticks for input messages but permits control messages without one", function()
        local input, _, input_code = contract.new({
            type = "input",
            seq = 1,
            payload = "move",
        })
        t.is_true(input == nil)
        t.eq(input_code, "malformed")

        local event = assert(contract.new({
            type = "event",
            seq = 1,
            payload = "connected",
        }))
        t.eq(event.tick, nil)
    end)
end)

t.describe("fake loopback transport", function()
    t.it("initializes, loops back, and drains inbound messages in order", function()
        local fake = transport.fake()
        t.eq(fake:state(), "new")
        t.is_true(assert(fake:initialize()))
        t.eq(fake:state(), "connected")
        t.is_true(assert(fake:enqueue(message(1, 10, "a"))))
        t.is_true(assert(fake:enqueue(message(2, 11, "b"))))
        t.is_true(assert(fake:enqueue(message(3, 12, "c"))))

        t.eq(assert(fake:poll()).seq, 1)
        t.eq(assert(fake:poll()).seq, 2)
        t.eq(assert(fake:poll()).seq, 3)
        t.is_true(fake:poll() == nil)
        t.eq(fake:diagnostics().sent, 3)
        t.eq(fake:diagnostics().received, 3)
    end)

    t.it("reports bounded queue depth and overflow without blocking", function()
        local fake = transport.fake({ queue_limit = 2 })
        assert(fake:initialize())
        assert(fake:inject(message(1, 1, "inbound-a")))
        assert(fake:inject(message(2, 2, "inbound-b")))
        assert(fake:enqueue(message(3, 3, "outbound-a")))
        assert(fake:enqueue(message(4, 4, "outbound-b")))
        local ok, _, code = fake:enqueue(message(5, 5, "outbound-c"))
        t.is_true(ok == nil)
        t.eq(code, "overflow")

        local diagnostics = fake:diagnostics()
        t.eq(diagnostics.queue_limit, 2)
        t.eq(diagnostics.inbound_depth, 2)
        t.eq(diagnostics.outbound_depth, 2)
        t.eq(diagnostics.overflow, 1)
        t.eq(diagnostics.dropped_outbound, 1)

        t.eq(assert(fake:poll()).seq, 1)
        t.eq(assert(fake:poll()).seq, 2)
        t.eq(assert(fake:poll()).seq, 3)
        t.eq(assert(fake:poll()).seq, 4)
    end)

    t.it("exposes malformed, unsupported-version, disconnect, and shutdown events", function()
        local fake = transport.fake()
        assert(fake:initialize())
        ---@type any
        local malformed_message = {
            version = 1,
            type = "bogus",
            seq = 1,
            payload = "bad",
        }
        local malformed, _, malformed_code = fake:inject(malformed_message)
        t.is_true(malformed == nil)
        t.eq(malformed_code, "malformed")

        local unsupported, _, unsupported_code = fake:inject({
            version = 99,
            type = "event",
            seq = 2,
            payload = "old",
        })
        t.is_true(unsupported == nil)
        t.eq(unsupported_code, "unsupported_version")
        t.eq(assert(fake:poll_event()).state, "connected")
        t.eq(assert(fake:poll_event()).code, "malformed")
        t.eq(assert(fake:poll_event()).code, "unsupported_version")

        assert(fake:disconnect("peer closed"))
        t.eq(fake:state(), "disconnected")
        t.eq(assert(fake:poll_event()).state, "disconnected")
        t.eq(assert(fake:poll_event()).code, "disconnected")

        assert(fake:shutdown())
        t.eq(fake:state(), "closed")
        t.eq(assert(fake:poll_event()).state, "closed")
    end)

    t.it("bounds the observable event queue", function()
        local fake = transport.fake({ queue_limit = 2 })
        assert(fake:initialize())

        for seq = 1, 3 do
            ---@type any
            local malformed_message = {
                version = 1,
                type = "invalid",
                seq = seq,
                payload = "bad",
            }
            local ok, _, code = fake:inject(malformed_message)
            t.is_true(ok == nil)
            t.eq(code, "malformed")
        end

        local diagnostics = fake:diagnostics()
        t.eq(diagnostics.event_depth, 2)
        t.is_true(diagnostics.overflow >= 1)
        t.eq(assert(fake:poll_event()).code, "malformed")
        t.eq(assert(fake:poll_event()).code, "malformed")
        t.is_true(fake:poll_event() == nil)
    end)

    t.it("retains the disconnect state/error pair at the minimum queue limit", function()
        local fake = transport.fake({ queue_limit = 1 })
        assert(fake:initialize())

        assert(fake:disconnect("peer closed"))
        t.eq(fake:diagnostics().event_depth, 2)
        t.eq(assert(fake:poll_event()).state, "disconnected")
        t.eq(assert(fake:poll_event()).code, "disconnected")
        t.is_true(fake:poll_event() == nil)
    end)

    t.it("drops queued messages on disconnect before reconnecting", function()
        local fake = transport.fake({ queue_limit = 2 })
        assert(fake:initialize())
        assert(fake:poll_event())
        assert(fake:inject(message(1, 1, "inbound-a")))
        assert(fake:inject(message(2, 2, "inbound-b")))
        assert(fake:enqueue(message(3, 3, "outbound")))

        assert(fake:disconnect("peer closed"))
        local disconnected = fake:diagnostics()
        t.eq(disconnected.dropped_inbound, 2)
        t.eq(disconnected.dropped_outbound, 1)
        t.eq(disconnected.inbound_depth, 0)
        t.eq(disconnected.outbound_depth, 0)
        t.eq(assert(fake:poll_event()).state, "disconnected")
        t.eq(assert(fake:poll_event()).code, "disconnected")

        assert(fake:initialize())
        t.eq(assert(fake:poll_event()).state, "connected")
        t.is_true(fake:poll() == nil)
    end)
end)

t.describe("browser transport contract", function()
    t.it("uses the same enqueue/poll behavior through the host seam", function()
        local fake = transport.fake()
        local browser = transport.browser({ eval = fake_browser_host(fake) })
        t.is_true(assert(browser:initialize()))
        assert(browser:enqueue(message(8, 80, "first")))
        assert(browser:enqueue(message(9, 81, "second")))
        t.eq(assert(browser:poll()).seq, 8)
        t.eq(assert(browser:poll()).seq, 9)
        t.eq(browser:diagnostics().sent, 2)
        t.eq(browser:diagnostics().received, 2)
    end)

    t.it("keeps connection transitions and queue diagnostics observable", function()
        local fake = transport.fake({ queue_limit = 1 })
        local browser = transport.browser({
            queue_limit = 1,
            eval = fake_browser_host(fake),
        })
        assert(browser:initialize())
        t.eq(assert(browser:poll_event()).state, "connected")
        assert(browser:disconnect("test"))
        t.eq(browser:state(), "disconnected")
        t.eq(assert(browser:poll_event()).state, "disconnected")
        t.eq(assert(browser:poll_event()).code, "disconnected")
        assert(browser:shutdown())
        t.eq(browser:state(), "closed")
        t.eq(assert(browser:poll_event()).state, "closed")
    end)
end)
