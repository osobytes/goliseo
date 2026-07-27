-- A `StarTransportAdapter` that records what passed through it.
--
-- The match driver takes its transport by injection, so a diagnostic tap can sit
-- between the two without the driver knowing: every call is forwarded verbatim
-- and every return value is passed back untouched. Nothing here retries,
-- reorders, buffers, or drops, so the tap cannot change the behaviour it is
-- there to explain.
--
-- ## Why this decodes nothing
--
-- A packet's newest authority tick is a *function of the delay policy*, not of
-- its contents. The driver documents that a sample taken during driver step `T`
-- is authority for input tick `first + T + DELAY`, and that the transport tick
-- for step `T` is `T + DELAY`. Substituting gives
--
--     authority_input_tick = first_input_tick + transport_tick
--     sample_step          = transport_tick - DELAY
--
-- so the envelope's own tick is enough. The tap therefore never re-decodes an
-- input packet: the payload stays opaque, the hot path pays one table write, and
-- the recorded lifecycle is derived from the stated policy rather than from a
-- second parse that could disagree with the first. `row_count` is left unset for
-- exactly this reason -- it is the one field that would need the parse.
--
-- ## Why signalling is special-cased
--
-- `request_offer`, `accept_offer`, `accept_answer`, and `take_signal` carry SDP.
-- SDP carries ICE ufrag/pwd and candidate lines with raw host and
-- server-reflexive addresses. Those blobs are handed to `record_signal`, which
-- keeps a direction and a byte length and stores the content as an explicit
-- redaction marker. The blob is not truncated into the record, not digested, and
-- not logged.

local net_diagnostics = require("game.online.net_diagnostics")

---@class DiagnosticTransportOptions
---@field transport StarTransportAdapter
---@field recorder NetDiagnostics
---@field peer_id string -- The local peer, used as the sender id on outbound records.
---@field first_input_tick integer
---@field input_delay_ticks integer
---@field clock (fun(): number)? -- Monotonic milliseconds; defaults to a zero clock.
---@field transport_tick (fun(): integer)? -- The receiver's own transport clock.
---@field retain_wires integer? -- Canonical outbound input wires to keep; 0 (default) keeps none.

---@class DiagnosticTransport : StarTransportAdapter
---@field _transport StarTransportAdapter
---@field _recorder NetDiagnostics
---@field _peer_id string
---@field _first integer
---@field _delay integer
---@field _clock fun(): number
---@field _transport_tick (fun(): integer)?
---@field _retain_wires integer
---@field _wires string[] -- Bounded FIFO of canonical outbound input wires.
---@field _wires_dropped integer
---@field _closed_peers integer
local DiagnosticTransport = {}
DiagnosticTransport.__index = DiagnosticTransport

---@return number
local function zero_clock()
    return 0
end

---@param options DiagnosticTransportOptions
---@return DiagnosticTransport
function DiagnosticTransport.new(options)
    assert(type(options) == "table", "a diagnostic transport needs options")
    return setmetatable({
        _transport = assert(options.transport, "a diagnostic transport needs a transport"),
        _recorder = assert(options.recorder, "a diagnostic transport needs a recorder"),
        _peer_id = assert(options.peer_id, "a diagnostic transport needs a local peer id"),
        _first = assert(options.first_input_tick, "a diagnostic transport needs a first tick"),
        _delay = assert(options.input_delay_ticks, "a diagnostic transport needs the input delay"),
        _clock = options.clock or zero_clock,
        _transport_tick = options.transport_tick,
        _retain_wires = options.retain_wires or 0,
        _wires = {},
        _wires_dropped = 0,
        _closed_peers = 0,
    }, DiagnosticTransport)
end

-- Canonical input wires this endpoint published, oldest first. Retention is off
-- unless asked for, because a wire ring is the only part of this tap that costs
-- real memory: at the protocol's 1 KiB envelope bound, keeping 192 of them is
-- 192 KiB and keeping a whole match is not bounded at all.
--
-- These are the *authored* wires, not decoded rows. They are exactly what a
-- desync package embeds, so the capture and the wire agree by construction
-- rather than by a second encoding that could drift.
---@param self DiagnosticTransport
---@param wire string
local function retain_wire(self, wire)
    if self._retain_wires <= 0 then
        return
    end
    if #self._wires >= self._retain_wires then
        table.remove(self._wires, 1)
        self._wires_dropped = self._wires_dropped + 1
    end
    self._wires[#self._wires + 1] = wire
end

---@return string[] wires, integer dropped
function DiagnosticTransport:wires()
    local out = {}
    for index, wire in ipairs(self._wires) do
        out[index] = wire
    end
    return out, self._wires_dropped
end

-- The delay policy, applied to an envelope tick. Control messages carry no tick,
-- so they are never routed through here.
---@param self DiagnosticTransport
---@param transport_tick integer
---@return integer authority_input_tick, integer sample_step
local function policy_ticks(self, transport_tick)
    return self._first + transport_tick, transport_tick - self._delay
end

-- `send_transport_tick` always comes from the envelope, because that is the
-- *sender's* clock and it travels with the packet. `arrival_transport_tick` is
-- the *receiver's* clock and is not in the envelope, so it is recorded only when
-- the caller supplies one. Reusing the sender's tick for both would fabricate a
-- zero-latency delivery on every link that is not in-process.
---@param self DiagnosticTransport
---@param sender_id string
---@param message TransportMessage
---@param disposition NetDiagnosticsPacketDisposition
---@param inbound boolean
local function record_envelope(self, sender_id, message, disposition, inbound)
    local tick = message.tick
    if type(tick) ~= "number" then
        return
    end
    local authority, step = policy_ticks(self, tick)
    local arrival = nil
    if inbound and self._transport_tick ~= nil then
        arrival = self._transport_tick()
    end
    net_diagnostics.record_packet(self._recorder, {
        sender_id = sender_id,
        disposition = disposition,
        sequence = message.seq,
        payload_bytes = #message.payload,
        sample_step = step,
        send_transport_tick = tick,
        authority_input_tick = authority,
        arrival_transport_tick = arrival,
    })
end

-- `state`, `code`, and `channel` are closed vocabularies from the transport
-- contract. `detail` is not: it carries `err`, a caller's close reason, or a
-- bridge's raw `String(error)`. It is redacted by `record_event`, which is the
-- single boundary every free-text field in this system passes through -- doing
-- it here as well would put two owners on one rule.
---@param self DiagnosticTransport
---@param event NetDiagnosticsEvent
local function note(self, event)
    net_diagnostics.record_event(self._recorder, event)
end

-- ---------------------------------------------------------------------------
-- Forwarded surface
-- ---------------------------------------------------------------------------

---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:initialize()
    return self._transport:initialize()
end

-- Shutdown is where teardown completeness is decided, so the tap reads the
-- star's own view back afterwards: any link the star still reports, and any
-- queue depth it still holds, is an orphan rather than a clean close.
---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:shutdown()
    local ok, err, code = self._transport:shutdown()
    local diagnostics = self._transport:diagnostics()
    ---@type string[]
    local orphaned = {}
    local residual_outbound, residual_inbound = 0, 0
    for _, peer in ipairs(diagnostics.peers) do
        if peer.state ~= "closed" then
            orphaned[#orphaned + 1] = peer.peer_id
        end
        residual_outbound = residual_outbound
            + peer.control.outbound_depth
            + peer.input.outbound_depth
        residual_inbound = residual_inbound + peer.control.inbound_depth + peer.input.inbound_depth
    end
    net_diagnostics.record_teardown(self._recorder, {
        requested = true,
        closed_peers = self._closed_peers,
        orphaned_peers = orphaned,
        residual_outbound = residual_outbound,
        residual_inbound = residual_inbound,
    })
    note(self, {
        kind = "teardown",
        monotonic_ms = self._clock(),
        state = ok and "closed" or "failed",
        code = code,
        detail = err,
    })
    return ok, err, code
end

---@return TransportRole
function DiagnosticTransport:role()
    return self._transport:role()
end

---@return integer
function DiagnosticTransport:capacity()
    return self._transport:capacity()
end

---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function DiagnosticTransport:open_peer(peer_id)
    local slot, err, code = self._transport:open_peer(peer_id)
    note(self, {
        kind = "peer_state",
        monotonic_ms = self._clock(),
        peer_id = peer_id,
        state = slot ~= nil and "opened" or "rejected",
        code = code,
        detail = err,
    })
    return slot, err, code
end

---@param peer_id string
---@param reason string?
---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:close_peer(peer_id, reason)
    local ok, err, code = self._transport:close_peer(peer_id, reason)
    if ok then
        self._closed_peers = self._closed_peers + 1
    end
    note(self, {
        kind = "peer_state",
        monotonic_ms = self._clock(),
        peer_id = peer_id,
        state = ok and "closed" or "close_failed",
        code = code,
        detail = reason or err,
    })
    return ok, err, code
end

---@return string[]
function DiagnosticTransport:peer_ids()
    return self._transport:peer_ids()
end

---@param peer_id string
---@return TransportPeerState?
function DiagnosticTransport:peer_state(peer_id)
    return self._transport:peer_state(peer_id)
end

-- ---------------------------------------------------------------------------
-- Signalling: recorded, never retained
-- ---------------------------------------------------------------------------

---@param peer_id string
---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:request_offer(peer_id)
    local ok, err, code = self._transport:request_offer(peer_id)
    note(self, {
        kind = "signal",
        monotonic_ms = self._clock(),
        peer_id = peer_id,
        state = "offer_requested",
        code = code,
        detail = err,
    })
    return ok, err, code
end

---@param signal string
---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:accept_offer(signal)
    net_diagnostics.record_signal(self._recorder, self._peer_id, "inbound", signal)
    return self._transport:accept_offer(signal)
end

---@param peer_id string
---@param signal string
---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:accept_answer(peer_id, signal)
    net_diagnostics.record_signal(self._recorder, peer_id, "inbound", signal)
    return self._transport:accept_answer(peer_id, signal)
end

---@param peer_id string
---@return string?, string?, TransportErrorCode?
function DiagnosticTransport:take_signal(peer_id)
    local signal, err, code = self._transport:take_signal(peer_id)
    if type(signal) == "string" then
        net_diagnostics.record_signal(self._recorder, peer_id, "outbound", signal)
    end
    return signal, err, code
end

-- ---------------------------------------------------------------------------
-- Traffic
-- ---------------------------------------------------------------------------

---@param peer_id string
---@param channel TransportChannel
---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function DiagnosticTransport:send(peer_id, channel, message)
    local ok, err, code = self._transport:send(peer_id, channel, message)
    if channel == "input" then
        record_envelope(self, self._peer_id, message, ok and "sent" or "rejected", false)
        if ok then
            retain_wire(self, message.payload)
        end
    end
    return ok, err, code
end

---@param channel TransportChannel
---@param message TransportMessage
---@return integer?, string?, TransportErrorCode?
function DiagnosticTransport:broadcast(channel, message)
    local delivered, err, code = self._transport:broadcast(channel, message)
    if channel == "input" then
        record_envelope(
            self,
            self._peer_id,
            message,
            delivered ~= nil and "sent" or "rejected",
            false
        )
        if delivered ~= nil then
            retain_wire(self, message.payload)
        end
    end
    return delivered, err, code
end

---@return TransportPeerMessage?, string?, TransportErrorCode?
function DiagnosticTransport:poll()
    local message, err, code = self._transport:poll()
    if message ~= nil and message.channel == "input" then
        record_envelope(self, message.peer_id, message.message, "arrived", true)
    end
    return message, err, code
end

---@param limit integer?
---@return TransportPeerMessage[]
function DiagnosticTransport:poll_batch(limit)
    local messages = self._transport:poll_batch(limit)
    for _, addressed in ipairs(messages) do
        if addressed.channel == "input" then
            record_envelope(self, addressed.peer_id, addressed.message, "arrived", true)
        end
    end
    return messages
end

---@return TransportPeerEvent?
function DiagnosticTransport:poll_event()
    local event = self._transport:poll_event()
    if event ~= nil then
        note(self, {
            kind = event.kind,
            monotonic_ms = self._clock(),
            peer_id = event.peer_id,
            channel = event.channel,
            state = event.state,
            code = event.code,
            detail = event.message,
        })
    end
    return event
end

---@return TransportState
function DiagnosticTransport:state()
    return self._transport:state()
end

-- Reading transport diagnostics is also the moment to fold them into the
-- recorder: the star's counters are cumulative, so the newest read is the whole
-- truth and no accumulation of our own is correct on top of it.
---@return TransportStarDiagnostics
function DiagnosticTransport:diagnostics()
    local star = self._transport:diagnostics()
    net_diagnostics.record_transport(self._recorder, star)
    return star
end

-- The wrapped adapter, for callers that legitimately need the real thing (the
-- fake star's `pump`, for instance, is a test affordance and not part of the
-- adapter contract).
---@return StarTransportAdapter
function DiagnosticTransport:inner()
    return self._transport
end

-- Every method the star adapter contract names is implemented above. Asserted
-- here rather than trusted, so a future addition to the contract fails loudly at
-- load time instead of at the first call a driver happens to make.
for _, name in ipairs({
    "initialize",
    "shutdown",
    "role",
    "capacity",
    "open_peer",
    "close_peer",
    "peer_ids",
    "peer_state",
    "request_offer",
    "accept_offer",
    "accept_answer",
    "take_signal",
    "send",
    "broadcast",
    "poll",
    "poll_batch",
    "poll_event",
    "state",
    "diagnostics",
}) do
    assert(
        type(DiagnosticTransport[name]) == "function",
        "diagnostic transport is missing " .. name
    )
end

return DiagnosticTransport
