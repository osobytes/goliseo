-- A `StarTransportAdapter` decorator that impairs *arrivals* with the OMP-2
-- probabilistic network model.
--
-- # Why the inbound side
--
-- Impairing the outbound side would mean intercepting `send`, which is where the
-- star reports `overflow`, `not_connected`, `role_forbidden`, and `malformed`
-- synchronously. Holding a message there would either delay those verdicts or
-- force this module to reimplement them, and a fault harness that reimplements
-- the failure paths it is meant to exercise proves nothing. So `send` and
-- `broadcast` are forwarded verbatim and every code the real star returns is the
-- code the driver sees. The impairment happens on the way *in*: the decorator
-- drains the wrapped endpoint, hands each envelope to `sim.network_conditions`,
-- and releases it to the driver on the arrival tick that module chose. That is
-- also the more faithful model — a real sender does transmit the packets the
-- network then drops.
--
-- # The profiles are the real ones
--
-- `data.network_profiles` and `sim.network_conditions` are used as-is, not
-- re-derived. Every envelope consumes one `send` on the conditions state, so the
-- jitter, independent-loss, duplication, and burst rolls are the documented four
-- per packet in the documented order, from the documented profile table, against
-- a private network seed. Arrival order is whatever `network_conditions.poll`
-- returns, so reordering and equal-arrival stability come from that module too.
--
-- The payload carried alongside is the real #161/#162 envelope. The conditions
-- state carries a synthetic `InputSample` purely as its packet identity: this
-- decorator never decodes, rewrites, or re-frames a real message, and the
-- redundancy that recovers a dropped row is the driver's own seven-row window,
-- not the laboratory's `history` array.
--
-- # What is impaired, and what deliberately is not
--
-- Only the `input` channel. A WebRTC control channel is reliable and ordered, so
-- silently losing session mail would model a transport this project does not
-- ship. Control-channel faults are therefore *declared* rather than
-- probabilistic: `duplicate_control_every` re-delivers every Nth control
-- envelope, which is the duplication a reliable channel can still produce
-- through a retransmit.
--
-- `withhold` is likewise declared, not probabilistic: a profile burst lands
-- where its RNG puts it, and pinning "a burst that straddles full time"
-- needs a window chosen on purpose. Both are recorded in `counters` so a report
-- can say which impairments were profile-driven and which were scripted.
--
-- This models *delivery conditions only*. It is not a claim about NAT
-- traversal, ICE, or real-device scheduling; see `docs/online/fault_harness.md`.

local contract = require("game.transport.contract")
local network_profiles = require("data.network_profiles")
local input_frame = require("sim.input_frame")
local network_conditions = require("sim.network_conditions")

---@alias FaultTransportPollOrder "arrival"|"reverse"

---@class FaultTransportOptions
---@field transport StarTransportAdapter -- The wrapped star endpoint.
---@field profile NetworkProfileName
---@field seed number -- Network-only seed; never the match seed.
---@field legs string[] -- Remote peer ids this endpoint receives from, canonical order.
---@field poll_order FaultTransportPollOrder? -- Release order handed to the driver.
---@field duplicate_control_every integer? -- Re-deliver every Nth control envelope; 0 disables.

---@class FaultTransportCounters
---@field profile NetworkProfileName
---@field scheduled integer -- Input envelopes handed to the impairment model.
---@field delivered integer -- Input envelopes released to the driver, duplicates included.
---@field dropped integer -- Input envelopes the model discarded.
---@field independent_lost integer
---@field burst_lost integer
---@field duplicated integer
---@field reordered integer
---@field withheld integer -- Input envelopes dropped by a scripted window, not the profile.
---@field held integer -- Input envelopes delayed by a scripted window and released after it.
---@field control_delivered integer
---@field control_duplicated integer -- Scripted control duplicates.
---@field unmapped integer -- Envelopes from a peer outside the declared legs.
---@field pending integer

---@class FaultTransport : StarTransportAdapter
---@field _transport StarTransportAdapter
---@field _conditions NetworkConditions
---@field _profile NetworkProfileName
---@field _slots table<string, integer> -- Remote peer id -> impairment source slot.
---@field _next_input_tick table<integer, integer>
---@field _queued table<integer, table<integer, TransportPeerMessage>>
---@field _ready TransportPeerMessage[]
---@field _tick integer
---@field _order FaultTransportPollOrder
---@field _control_every integer
---@field _control_seen integer
---@field _withhold { from: integer, through: integer }[]
---@field _hold { from: integer, through: integer }[]
---@field _held TransportPeerMessage[]
---@field _counters FaultTransportCounters
local FaultTransport = {}
FaultTransport.__index = FaultTransport

-- The impairment model addresses legs by canonical slot index, and a direct-host
-- star has at most seven guests plus the host link, so every leg of every
-- endpoint fits inside one slot axis.
FaultTransport.MAX_LEGS = input_frame.SLOT_COUNT

---@param options FaultTransportOptions
---@return FaultTransport
function FaultTransport.new(options)
    assert(type(options) == "table", "a fault transport needs options")
    local transport = assert(options.transport, "a fault transport needs a transport")
    local profile_name = assert(options.profile, "a fault transport needs a profile name")
    local profile = assert(network_profiles[profile_name], "unknown network profile")
    local legs = assert(options.legs, "a fault transport needs its declared legs")
    assert(#legs <= FaultTransport.MAX_LEGS, "a fault transport models at most eight legs")
    assert(type(options.seed) == "number", "a fault transport needs a network seed")
    ---@type table<string, integer>
    local slots = {}
    for index, peer_id in ipairs(legs) do
        assert(slots[peer_id] == nil, "a fault transport leg is declared twice")
        slots[peer_id] = index
    end
    return setmetatable({
        _transport = transport,
        _conditions = network_conditions.new(profile, options.seed),
        _profile = profile_name,
        _slots = slots,
        _next_input_tick = {},
        _queued = {},
        _ready = {},
        _tick = -1,
        _order = options.poll_order or "arrival",
        _control_every = options.duplicate_control_every or 0,
        _control_seen = 0,
        _withhold = {},
        _hold = {},
        _held = {},
        _counters = {
            profile = profile_name,
            scheduled = 0,
            delivered = 0,
            dropped = 0,
            independent_lost = 0,
            burst_lost = 0,
            duplicated = 0,
            reordered = 0,
            withheld = 0,
            held = 0,
            control_delivered = 0,
            control_duplicated = 0,
            unmapped = 0,
            pending = 0,
        },
    }, FaultTransport)
end

-- ---------------------------------------------------------------------------
-- Declared (non-probabilistic) faults
-- ---------------------------------------------------------------------------

-- Drop every input envelope arriving in `[from, through]`. Declared rather than
-- rolled, because a burst that has to straddle a named boundary cannot be left
-- to a profile's RNG.
---@param from integer
---@param through integer
function FaultTransport:withhold(from, through)
    assert(through >= from, "a withheld window ends at or after it starts")
    self._withhold[#self._withhold + 1] = { from = from, through = through }
end

-- Buffer every input envelope arriving in `[from, through]` and release the
-- whole backlog on the first tick after it. This is the withhold-then-resume
-- shape the driver's own specs use, and it is the only way to reach `late_input`
-- on purpose: a hold longer than the 30-tick retained floor delivers rows the
-- window can no longer accept, which no probabilistic profile can be made to do
-- reliably.
---@param from integer
---@param through integer
function FaultTransport:hold(from, through)
    assert(through >= from, "a held window ends at or after it starts")
    self._hold[#self._hold + 1] = { from = from, through = through }
end

---@param windows { from: integer, through: integer }[]
---@param tick integer
---@return boolean
local function inside(windows, tick)
    for _, window in ipairs(windows) do
        if tick >= window.from and tick <= window.through then
            return true
        end
    end
    return false
end

---@param transport FaultTransport
---@return boolean
local function withholding(transport)
    return inside(transport._withhold, transport._tick)
end

-- ---------------------------------------------------------------------------
-- The impairment pipeline
-- ---------------------------------------------------------------------------

---@param transport FaultTransport
---@param entry TransportPeerMessage
local function release(transport, entry)
    transport._ready[#transport._ready + 1] = entry
end

---@param transport FaultTransport
---@param entry TransportPeerMessage
local function schedule(transport, entry)
    if entry.channel ~= "input" then
        transport._counters.control_delivered = transport._counters.control_delivered + 1
        release(transport, entry)
        transport._control_seen = transport._control_seen + 1
        if
            transport._control_every > 0
            and transport._control_seen % transport._control_every == 0
        then
            transport._counters.control_duplicated = transport._counters.control_duplicated + 1
            release(transport, {
                peer_id = entry.peer_id,
                channel = entry.channel,
                message = contract.copy(entry.message),
                arrival_seq = entry.arrival_seq,
            })
        end
        return
    end
    local slot = transport._slots[entry.peer_id]
    if slot == nil then
        -- An envelope from a peer outside the declared legs has no impairment
        -- identity. Dropping it would be a silent fault the report could not
        -- explain, so it is delivered and counted.
        transport._counters.unmapped = transport._counters.unmapped + 1
        release(transport, entry)
        return
    end
    if withholding(transport) then
        transport._counters.withheld = transport._counters.withheld + 1
        return
    end
    if inside(transport._hold, transport._tick) then
        transport._counters.held = transport._counters.held + 1
        transport._held[#transport._held + 1] = entry
        return
    end
    local input_tick = (transport._next_input_tick[slot] or 0)
    transport._next_input_tick[slot] = input_tick + 1
    local receipt = network_conditions.send(
        transport._conditions,
        transport._tick,
        slot,
        input_tick,
        input_frame.neutral_sample()
    )
    if receipt == nil then
        -- The impairment model refused to schedule. That is a harness bug, not a
        -- transport fault, so it fails loudly rather than quietly delivering.
        error(
            "the impairment model refused a packet at transport tick " .. tostring(transport._tick)
        )
    end
    transport._counters.scheduled = transport._counters.scheduled + 1
    if receipt.dropped then
        transport._counters.dropped = transport._counters.dropped + 1
        return
    end
    local by_tick = transport._queued[slot]
    if by_tick == nil then
        by_tick = {}
        transport._queued[slot] = by_tick
    end
    by_tick[input_tick] = entry
end

---@param transport FaultTransport
local function ingest(transport)
    while true do
        local batch = transport._transport:poll_batch(contract.MAX_QUEUE_LIMIT)
        if #batch == 0 then
            return
        end
        for _, entry in ipairs(batch) do
            schedule(transport, entry)
        end
    end
end

---@param transport FaultTransport
local function deliver_due(transport)
    local deliveries = network_conditions.poll(transport._conditions, transport._tick)
    ---@type { slot: integer, tick: integer }[]
    local consumed = {}
    for _, delivery in ipairs(deliveries) do
        local by_tick = transport._queued[delivery.source_slot]
        local entry = by_tick and by_tick[delivery.current.tick] or nil
        if entry ~= nil then
            transport._counters.delivered = transport._counters.delivered + 1
            if delivery.duplicate_ordinal == 0 then
                release(transport, entry)
            else
                release(transport, {
                    peer_id = entry.peer_id,
                    channel = entry.channel,
                    message = contract.copy(entry.message),
                    arrival_seq = entry.arrival_seq,
                })
            end
            consumed[#consumed + 1] = { slot = delivery.source_slot, tick = delivery.current.tick }
        end
    end
    -- Both ordinals of a duplicated packet arrive on the same tick, so the entry
    -- is only released from the queue once every delivery for this tick has been
    -- handed out.
    for _, target in ipairs(consumed) do
        local by_tick = transport._queued[target.slot]
        if by_tick ~= nil then
            by_tick[target.tick] = nil
        end
    end
end

-- Advance this endpoint's transport clock by exactly one tick: drain the wrapped
-- endpoint, impair, and release everything the model says is due. The harness
-- calls this once per driver step, before the driver polls.
---@param transport_tick integer
function FaultTransport:tick(transport_tick)
    assert(transport_tick > self._tick, "a fault transport clock only moves forward")
    self._tick = transport_tick
    if #self._held > 0 and not inside(self._hold, transport_tick) then
        local backlog = self._held
        self._held = {}
        for _, entry in ipairs(backlog) do
            schedule(self, entry)
        end
    end
    ingest(self)
    deliver_due(self)
end

---@return FaultTransportCounters
function FaultTransport:counters()
    local model = network_conditions.counters(self._conditions)
    local counters = self._counters
    return {
        profile = counters.profile,
        scheduled = counters.scheduled,
        delivered = counters.delivered,
        dropped = counters.dropped,
        independent_lost = model.independent_lost,
        burst_lost = model.burst_lost,
        duplicated = model.duplicated,
        reordered = model.reordered,
        withheld = counters.withheld,
        held = counters.held,
        control_delivered = counters.control_delivered,
        control_duplicated = counters.control_duplicated,
        unmapped = counters.unmapped,
        pending = network_conditions.pending(self._conditions),
    }
end

---@return NetworkConditionDiagnostics
function FaultTransport:model_diagnostics()
    return network_conditions.diagnostics(self._conditions)
end

-- ---------------------------------------------------------------------------
-- The adapter surface
-- ---------------------------------------------------------------------------

-- `reverse` hands input arrivals over last-first. The driver documents that poll
-- order is recorded and never used as authority, so a reversed release order
-- must produce identical confirmed boundaries -- which is exactly the claim this
-- option exists to test.
--
-- The **control** channel keeps its order in both modes. A WebRTC control
-- channel is reliable *and ordered*, and the session's own framing splits a wire
-- across several envelopes that are reassembled in arrival order; reordering
-- them would break a guarantee the real transport provides and would test the
-- harness rather than the driver.
---@param limit integer?
---@return TransportPeerMessage[]
function FaultTransport:poll_batch(limit)
    local budget = limit or contract.DEFAULT_POLL_BATCH
    ---@type TransportPeerMessage[]
    local batch = {}
    if self._order == "arrival" then
        while #batch < budget and #self._ready > 0 do
            batch[#batch + 1] = table.remove(self._ready, 1)
        end
        return batch
    end
    ---@type TransportPeerMessage[], TransportPeerMessage[]
    local control, input = {}, {}
    for _, entry in ipairs(self._ready) do
        if entry.channel == "control" then
            control[#control + 1] = entry
        else
            input[#input + 1] = entry
        end
    end
    ---@type TransportPeerMessage[]
    local ordered = {}
    for _, entry in ipairs(control) do
        ordered[#ordered + 1] = entry
    end
    for index = #input, 1, -1 do
        ordered[#ordered + 1] = input[index]
    end
    self._ready = {}
    for index, entry in ipairs(ordered) do
        if index <= budget then
            batch[#batch + 1] = entry
        else
            self._ready[#self._ready + 1] = entry
        end
    end
    return batch
end

---@return TransportPeerMessage?
function FaultTransport:poll()
    return self:poll_batch(1)[1]
end

---@return boolean?, string?, TransportErrorCode?
function FaultTransport:initialize()
    return self._transport:initialize()
end

---@return boolean?, string?, TransportErrorCode?
function FaultTransport:shutdown()
    return self._transport:shutdown()
end

---@return TransportRole
function FaultTransport:role()
    return self._transport:role()
end

---@return integer
function FaultTransport:capacity()
    return self._transport:capacity()
end

---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function FaultTransport:open_peer(peer_id)
    return self._transport:open_peer(peer_id)
end

---@param peer_id string
---@param reason string?
---@return boolean?, string?, TransportErrorCode?
function FaultTransport:close_peer(peer_id, reason)
    return self._transport:close_peer(peer_id, reason)
end

---@return string[]
function FaultTransport:peer_ids()
    return self._transport:peer_ids()
end

---@param peer_id string
---@return TransportPeerState?
function FaultTransport:peer_state(peer_id)
    return self._transport:peer_state(peer_id)
end

---@return TransportState
function FaultTransport:state()
    return self._transport:state()
end

---@param peer_id string
---@return boolean?, string?, TransportErrorCode?
function FaultTransport:request_offer(peer_id)
    return self._transport:request_offer(peer_id)
end

---@param signal string
---@return boolean?, string?, TransportErrorCode?
function FaultTransport:accept_offer(signal)
    return self._transport:accept_offer(signal)
end

---@param peer_id string
---@param signal string
---@return boolean?, string?, TransportErrorCode?
function FaultTransport:accept_answer(peer_id, signal)
    return self._transport:accept_answer(peer_id, signal)
end

---@param peer_id string
---@return string?, string?, TransportErrorCode?
function FaultTransport:take_signal(peer_id)
    return self._transport:take_signal(peer_id)
end

---@param peer_id string
---@param channel TransportChannel
---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function FaultTransport:send(peer_id, channel, message)
    return self._transport:send(peer_id, channel, message)
end

---@param channel TransportChannel
---@param message TransportMessage
---@return integer?, string?, TransportErrorCode?
function FaultTransport:broadcast(channel, message)
    return self._transport:broadcast(channel, message)
end

---@return TransportPeerEvent?
function FaultTransport:poll_event()
    return self._transport:poll_event()
end

---@return TransportStarDiagnostics
function FaultTransport:diagnostics()
    return self._transport:diagnostics()
end

-- The decorator is only useful if it is substitutable for the endpoint it wraps.
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
    assert(type(FaultTransport[name]) == "function", "fault transport is missing " .. name)
end

return FaultTransport
