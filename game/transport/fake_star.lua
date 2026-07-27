local contract = require("game.transport.contract")

-- Pure in-process host-star transport. It carries the reference semantics for
-- the browser adapter: one host endpoint, up to seven independently addressed
-- guest links, a reliable ordered control channel and a lossy unordered input
-- channel per link, bounded queues, simulated `bufferedAmount` backpressure,
-- typed events, and deterministic poll batching. It owns no simulation
-- authority and never inspects a payload.

---@class FakeStarTransportOptions
---@field role TransportRole?
---@field peer_id string? -- Guest link identity; ignored for a host.
---@field queue_limit integer?
---@field max_guests integer?
---@field buffered_amount_limit integer?

---@class FakeStarChannel
---@field state TransportPeerState
---@field outbound TransportAddressedMessage[]
---@field inbound TransportPeerMessage[]
---@field backpressured boolean
---@field buffered_amount integer
---@field sent integer
---@field received integer
---@field dropped_outbound integer
---@field dropped_inbound integer
---@field last_seq integer?

---@class FakeStarPeer
---@field peer_id string
---@field slot integer
---@field state TransportPeerState
---@field ice_state string
---@field channels table<TransportChannel, FakeStarChannel>
---@field arrival_seq integer
---@field sequence_gaps integer
---@field backpressure integer
---@field malformed integer
---@field last_error string?
---@field link FakeStarTransport?
---@field link_peer_id string?

---@class FakeStarTransport: StarTransportAdapter
---@field _role TransportRole
---@field _peer_id string
---@field _state TransportState
---@field _queue_limit integer
---@field _event_limit integer
---@field _max_guests integer
---@field _buffered_amount_limit integer
---@field _peers table<string, FakeStarPeer>
---@field _order string[]
---@field _events TransportPeerEvent[]
---@field _cursor integer
---@field _sent integer
---@field _received integer
---@field _dropped_outbound integer
---@field _dropped_inbound integer
---@field _malformed integer
---@field _unsupported_version integer
---@field _overflow integer
---@field _backpressure integer
---@field _last_error string?
local FakeStarTransport = {}
FakeStarTransport.__index = FakeStarTransport

---@param code TransportErrorCode
---@param message string
---@return nil, string, TransportErrorCode
local function failure(code, message)
    return nil, message, code
end

---@return FakeStarChannel
local function new_channel()
    return {
        state = "connecting",
        outbound = {},
        inbound = {},
        backpressured = false,
        buffered_amount = 0,
        sent = 0,
        received = 0,
        dropped_outbound = 0,
        dropped_inbound = 0,
        last_seq = nil,
    }
end

---@param options FakeStarTransportOptions?
---@return FakeStarTransport
function FakeStarTransport.new(options)
    options = options or {}
    local role = options.role or "host"
    assert(role == "host" or role == "guest", "fake star transport role must be host or guest")
    local queue_limit = options.queue_limit or contract.DEFAULT_QUEUE_LIMIT
    assert(
        queue_limit == math.floor(queue_limit)
            and queue_limit > 0
            and queue_limit <= contract.MAX_QUEUE_LIMIT,
        "fake star transport queue_limit is outside the supported range"
    )
    local max_guests = options.max_guests or contract.MAX_GUESTS
    assert(
        max_guests == math.floor(max_guests)
            and max_guests > 0
            and max_guests <= contract.MAX_GUESTS,
        "fake star transport max_guests is outside the supported range"
    )
    local buffered_amount_limit = options.buffered_amount_limit
        or contract.DEFAULT_BUFFERED_AMOUNT_LIMIT
    assert(
        buffered_amount_limit == math.floor(buffered_amount_limit)
            and buffered_amount_limit > 0
            and buffered_amount_limit <= contract.MAX_BUFFERED_AMOUNT_LIMIT,
        "fake star transport buffered_amount_limit is outside the supported range"
    )
    local peer_id = options.peer_id or "guest"
    assert(contract.validate_peer_id(peer_id))
    return setmetatable({
        _role = role,
        _peer_id = peer_id,
        _state = "new",
        _queue_limit = queue_limit,
        _event_limit = math.max(2, queue_limit),
        _max_guests = role == "host" and max_guests or 1,
        _buffered_amount_limit = buffered_amount_limit,
        _peers = {},
        _order = {},
        _events = {},
        _cursor = 0,
        _sent = 0,
        _received = 0,
        _dropped_outbound = 0,
        _dropped_inbound = 0,
        _malformed = 0,
        _unsupported_version = 0,
        _overflow = 0,
        _backpressure = 0,
        _last_error = nil,
    }, FakeStarTransport)
end

---@param event TransportPeerEvent
function FakeStarTransport:_push_event(event)
    if #self._events >= self._event_limit then
        table.remove(self._events, 1)
        self._overflow = self._overflow + 1
        self._last_error = "fake star transport event queue is full"
    end
    self._events[#self._events + 1] = event
end

---@param code TransportErrorCode
---@param message string
---@param peer FakeStarPeer?
---@param channel TransportChannel?
function FakeStarTransport:_record_error(code, message, peer, channel)
    self._last_error = message
    if code == "malformed" or code == "payload_too_large" or code == "channel_mismatch" then
        self._malformed = self._malformed + 1
        if peer then
            peer.malformed = peer.malformed + 1
        end
    elseif code == "unsupported_version" then
        self._unsupported_version = self._unsupported_version + 1
    elseif code == "overflow" then
        self._overflow = self._overflow + 1
    elseif code == "backpressure" then
        self._backpressure = self._backpressure + 1
        if peer then
            peer.backpressure = peer.backpressure + 1
        end
    end
    if peer then
        peer.last_error = message
        self:_push_event({
            kind = "peer_error",
            peer_id = peer.peer_id,
            channel = channel,
            code = code,
            message = message,
        })
    else
        self:_push_event({ kind = "star_error", code = code, message = message })
    end
end

---@param peer FakeStarPeer
---@param state TransportPeerState
function FakeStarTransport:_set_peer_state(peer, state)
    peer.state = state
    for _, channel in ipairs(contract.CHANNEL_ORDER) do
        peer.channels[channel].state = state
    end
    self:_push_event({ kind = "peer_state", peer_id = peer.peer_id, state = state })
end

---@param state TransportState
function FakeStarTransport:_set_state(state)
    self._state = state
    self:_push_event({ kind = "star_state", state = state })
end

---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:_require_connected()
    if self._state == "new" then
        return failure("not_initialized", "star transport is not initialized")
    end
    if self._state == "closed" then
        return failure("closed", "star transport is closed")
    end
    if self._state ~= "connected" then
        return failure("not_connected", "star transport is not connected")
    end
    return true
end

---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:initialize()
    if self._state == "connected" then
        return true
    end
    self:_set_state("connected")
    if self._role == "guest" and not self._peers[contract.HOST_PEER_ID] then
        assert(self:_add_peer(contract.HOST_PEER_ID))
    end
    return true
end

---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function FakeStarTransport:_add_peer(peer_id)
    local slot = #self._order + 1
    ---@type FakeStarPeer
    local peer = {
        peer_id = peer_id,
        slot = slot,
        state = "connecting",
        ice_state = "new",
        channels = { control = new_channel(), input = new_channel() },
        arrival_seq = 0,
        sequence_gaps = 0,
        backpressure = 0,
        malformed = 0,
        last_error = nil,
        link = nil,
        link_peer_id = nil,
    }
    self._peers[peer_id] = peer
    self._order[slot] = peer_id
    self:_push_event({ kind = "peer_state", peer_id = peer_id, state = "connecting" })
    return slot
end

-- Host-only. Allocates one guest slot with its own control and input channels.
---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function FakeStarTransport:open_peer(peer_id)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "host" then
        return failure("role_forbidden", "only a host opens guest links")
    end
    ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        self:_record_error(code or "malformed", err or "invalid peer id")
        return nil, err, code
    end
    if peer_id == contract.HOST_PEER_ID then
        self:_record_error("duplicate_peer", "the host peer id is reserved")
        return failure("duplicate_peer", "the host peer id is reserved")
    end
    if self._peers[peer_id] then
        self:_record_error("duplicate_peer", "star peer is already open")
        return failure("duplicate_peer", "star peer is already open")
    end
    if #self._order >= self._max_guests then
        self:_record_error("capacity", "star transport is at guest capacity")
        return failure("capacity", "star transport is at guest capacity")
    end
    return self:_add_peer(peer_id)
end

-- Test seam: joins a guest endpoint to an already-opened host slot. The browser
-- adapter reaches the same state through manual offer/answer exchange.
---@param guest FakeStarTransport
---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:link(guest)
    if self._role ~= "host" or guest._role ~= "guest" then
        return failure("role_forbidden", "link joins one host endpoint to one guest endpoint")
    end
    local peer = self._peers[guest._peer_id]
    if not peer then
        return failure("unknown_peer", "star host has no slot for that guest")
    end
    local host_peer = guest._peers[contract.HOST_PEER_ID]
    if not host_peer then
        return failure("not_connected", "guest endpoint is not initialized")
    end
    peer.link = guest
    peer.link_peer_id = contract.HOST_PEER_ID
    host_peer.link = self
    host_peer.link_peer_id = guest._peer_id
    peer.ice_state = "connected"
    host_peer.ice_state = "connected"
    self:_set_peer_state(peer, "connected")
    guest:_set_peer_state(host_peer, "connected")
    return true
end

---@param peer_id string
---@return FakeStarPeer?, string?, TransportErrorCode?
function FakeStarTransport:_peer(peer_id)
    local peer = self._peers[peer_id]
    if not peer then
        return failure("unknown_peer", "star transport has no peer with that id")
    end
    return peer
end

---@param peer FakeStarPeer
function FakeStarTransport:_drop_peer_queues(peer)
    for _, channel_name in ipairs(contract.CHANNEL_ORDER) do
        local channel = peer.channels[channel_name]
        self._dropped_outbound = self._dropped_outbound + #channel.outbound
        self._dropped_inbound = self._dropped_inbound + #channel.inbound
        channel.dropped_outbound = channel.dropped_outbound + #channel.outbound
        channel.dropped_inbound = channel.dropped_inbound + #channel.inbound
        channel.outbound = {}
        channel.inbound = {}
        channel.buffered_amount = 0
    end
end

-- Closes one link without disturbing the other guests. Repeated calls are
-- idempotent once the peer reports `closed`.
---@param peer_id string
---@param reason string?
---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:close_peer(peer_id, reason)
    local peer, err, code = self:_peer(peer_id)
    if not peer then
        return nil, err, code
    end
    if peer.state == "closed" then
        return true
    end
    local detail = reason or "star peer closed"
    self:_drop_peer_queues(peer)
    peer.ice_state = "closed"
    local link, link_peer_id = peer.link, peer.link_peer_id
    peer.link = nil
    peer.link_peer_id = nil
    self:_set_peer_state(peer, "closed")
    self:_record_error("disconnected", detail, peer)
    if link and link_peer_id then
        local remote = link._peers[link_peer_id]
        if remote and remote.state ~= "closed" then
            remote.link = nil
            remote.link_peer_id = nil
            link:_drop_peer_queues(remote)
            remote.ice_state = "closed"
            link:_set_peer_state(remote, "disconnected")
            link:_record_error("disconnected", detail, remote)
        end
    end
    return true
end

---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:shutdown()
    if self._state == "closed" then
        return true
    end
    for _, peer_id in ipairs(self._order) do
        self:close_peer(peer_id, "star transport shutdown")
    end
    -- Teardown releases every slot, so a repeated shutdown and any later
    -- initialize start from an empty star instead of orphan links.
    self._peers = {}
    self._order = {}
    self._cursor = 0
    self:_set_state("closed")
    return true
end

---@param peer FakeStarPeer
---@param channel_name TransportChannel
---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:_enqueue(peer, channel_name, message)
    local ok, err, code = contract.validate_channel_message(channel_name, message)
    if not ok then
        self:_record_error(code or "malformed", err or "star transport rejected a message", peer)
        return nil, err, code
    end
    if peer.state ~= "connected" then
        self:_record_error("not_connected", "star peer link is not connected", peer, channel_name)
        return failure("not_connected", "star peer link is not connected")
    end
    local channel = peer.channels[channel_name]
    local wire = assert(contract.encode(message))
    -- The queue bound is the only place a send is refused. A full send buffer
    -- is backpressure, not a rejection: it leaves messages queued until the
    -- channel drains or the bounded queue overflows.
    if #channel.outbound >= self._queue_limit then
        channel.dropped_outbound = channel.dropped_outbound + 1
        self._dropped_outbound = self._dropped_outbound + 1
        self:_record_error("overflow", "star peer outbound queue is full", peer, channel_name)
        return failure("overflow", "star peer outbound queue is full")
    end
    channel.outbound[#channel.outbound + 1] = {
        peer_id = peer.link_peer_id or peer.peer_id,
        channel = channel_name,
        message = contract.copy(message),
    }
    channel.buffered_amount = channel.buffered_amount + #wire
    channel.sent = channel.sent + 1
    self._sent = self._sent + 1
    return true
end

-- Host: address one guest. Guest: the only legal target is the host link.
---@param peer_id string
---@param channel TransportChannel
---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function FakeStarTransport:send(peer_id, channel, message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role == "guest" and peer_id ~= contract.HOST_PEER_ID then
        self:_record_error("role_forbidden", "a guest may only address the host")
        return failure("role_forbidden", "a guest may only address the host")
    end
    local peer, peer_err, peer_code = self:_peer(peer_id)
    if not peer then
        self:_record_error(peer_code or "unknown_peer", peer_err or "unknown peer")
        return nil, peer_err, peer_code
    end
    return self:_enqueue(peer, channel, message)
end

-- Host-only fan-out. Returns how many links accepted the message; per-peer
-- failures stay visible through peer events and diagnostics.
---@param channel TransportChannel
---@param message TransportMessage
---@return integer?, string?, TransportErrorCode?
function FakeStarTransport:broadcast(channel, message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "host" then
        self:_record_error("role_forbidden", "only a host fans out canonical batches")
        return failure("role_forbidden", "only a host fans out canonical batches")
    end
    ok, err, code = contract.validate_channel_message(channel, message)
    if not ok then
        self:_record_error(code or "malformed", err or "star transport rejected a message")
        return nil, err, code
    end
    local delivered = 0
    for _, peer_id in ipairs(self._order) do
        local peer = self._peers[peer_id]
        if peer.state == "connected" then
            if self:_enqueue(peer, channel, message) then
                delivered = delivered + 1
            end
        end
    end
    return delivered
end

---@param peer FakeStarPeer
---@param channel_name TransportChannel
---@param addressed TransportAddressedMessage
function FakeStarTransport:_receive(peer, channel_name, addressed)
    local channel = peer.channels[channel_name]
    if #channel.inbound >= self._queue_limit then
        channel.dropped_inbound = channel.dropped_inbound + 1
        self._dropped_inbound = self._dropped_inbound + 1
        self:_record_error("overflow", "star peer inbound queue is full", peer, channel_name)
        return
    end
    local seq = addressed.message.seq
    if channel.last_seq and seq > channel.last_seq + 1 then
        peer.sequence_gaps = peer.sequence_gaps + seq - channel.last_seq - 1
    end
    if not channel.last_seq or seq > channel.last_seq then
        channel.last_seq = seq
    end
    peer.arrival_seq = peer.arrival_seq + 1
    channel.inbound[#channel.inbound + 1] = {
        peer_id = peer.peer_id,
        channel = channel_name,
        arrival_seq = peer.arrival_seq,
        message = addressed.message,
    }
end

-- Test seam: delivers everything currently buffered on this endpoint and on
-- every endpoint linked to it, then clears the simulated `bufferedAmount`.
-- The browser adapter has no equivalent because the browser drains its own
-- data channels asynchronously.
function FakeStarTransport:pump()
    local endpoints = { self }
    local seen = { [self] = true }
    for _, peer_id in ipairs(self._order) do
        local link = self._peers[peer_id].link
        if link and not seen[link] then
            seen[link] = true
            endpoints[#endpoints + 1] = link
        end
    end
    for _, endpoint in ipairs(endpoints) do
        endpoint:_flush()
    end
end

-- Drains each channel until its send budget is exhausted. `buffered_amount_limit`
-- stands in for a data channel's `bufferedAmount` ceiling: once a pass reaches
-- it, the remainder stays queued and the peer reports backpressure instead of
-- blocking.
function FakeStarTransport:_flush()
    for _, peer_id in ipairs(self._order) do
        local peer = self._peers[peer_id]
        for _, channel_name in ipairs(contract.CHANNEL_ORDER) do
            local channel = peer.channels[channel_name]
            local link, link_peer_id = peer.link, peer.link_peer_id
            local budget = self._buffered_amount_limit
            while #channel.outbound > 0 do
                local addressed = channel.outbound[1]
                local size = #assert(contract.encode(addressed.message))
                if size > budget then
                    -- Latch the report so a saturated buffer costs one event,
                    -- not one per drain pass. It clears once the channel moves.
                    if not channel.backpressured then
                        channel.backpressured = true
                        self:_record_error(
                            "backpressure",
                            "star peer channel send buffer is full",
                            peer,
                            channel_name
                        )
                    end
                    break
                end
                table.remove(channel.outbound, 1)
                channel.backpressured = false
                budget = budget - size
                channel.buffered_amount = math.max(0, channel.buffered_amount - size)
                local remote = link and link_peer_id and link._peers[link_peer_id] or nil
                if link and remote and peer.state == "connected" then
                    link:_receive(remote, channel_name, addressed)
                else
                    channel.dropped_outbound = channel.dropped_outbound + 1
                    self._dropped_outbound = self._dropped_outbound + 1
                end
            end
        end
    end
end

-- Deterministic drain: a persistent cursor walks (slot, channel-rank) pairs and
-- takes at most one message per pair before moving on, so no peer can starve
-- another. The JavaScript bridge advances the same cursor. This is an ordering
-- rule at the Lua boundary, not a claim that browser callback arrival order is
-- simulation order; the session layer still reorders by tick.
---@return TransportPeerMessage?
function FakeStarTransport:_take()
    local slots = #self._order
    local channels = #contract.CHANNEL_ORDER
    if slots == 0 then
        return nil
    end
    for _ = 1, slots * channels do
        local peer_index = math.floor(self._cursor / channels) % slots
        local channel_index = self._cursor % channels
        self._cursor = (self._cursor + 1) % (slots * channels)
        local peer = self._peers[self._order[peer_index + 1]]
        local channel = peer.channels[contract.CHANNEL_ORDER[channel_index + 1]]
        if #channel.inbound > 0 then
            local entry = table.remove(channel.inbound, 1)
            channel.received = channel.received + 1
            self._received = self._received + 1
            return entry
        end
    end
    return nil
end

---@param limit integer?
---@return TransportPeerMessage[]
function FakeStarTransport:poll_batch(limit)
    local budget = limit or contract.DEFAULT_POLL_BATCH
    local batch = {}
    if self._state ~= "connected" then
        return batch
    end
    while #batch < budget do
        local entry = self:_take()
        if not entry then
            return batch
        end
        batch[#batch + 1] = entry
    end
    return batch
end

---@return TransportPeerMessage?, string?, TransportErrorCode?
function FakeStarTransport:poll()
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    return self:poll_batch(1)[1]
end

---@return TransportPeerEvent?
function FakeStarTransport:poll_event()
    return table.remove(self._events, 1)
end

---@return TransportRole
function FakeStarTransport:role()
    return self._role
end

---@return integer
function FakeStarTransport:capacity()
    return self._max_guests
end

---@return string[]
function FakeStarTransport:peer_ids()
    local ids = {}
    for index, peer_id in ipairs(self._order) do
        ids[index] = peer_id
    end
    return ids
end

---@param peer_id string
---@return TransportPeerState?
function FakeStarTransport:peer_state(peer_id)
    local peer = self._peers[peer_id]
    return peer and peer.state or nil
end

---@return TransportState
function FakeStarTransport:state()
    return self._state
end

---@param channel FakeStarChannel
---@return TransportChannelDiagnostics
local function channel_diagnostics(channel)
    return {
        state = channel.state,
        outbound_depth = #channel.outbound,
        inbound_depth = #channel.inbound,
        buffered_amount = channel.buffered_amount,
        sent = channel.sent,
        received = channel.received,
        dropped_outbound = channel.dropped_outbound,
        dropped_inbound = channel.dropped_inbound,
    }
end

---@return TransportStarDiagnostics
function FakeStarTransport:diagnostics()
    local peers = {}
    for index, peer_id in ipairs(self._order) do
        local peer = self._peers[peer_id]
        peers[index] = {
            peer_id = peer.peer_id,
            slot = peer.slot,
            state = peer.state,
            ice_state = peer.ice_state,
            control = channel_diagnostics(peer.channels.control),
            input = channel_diagnostics(peer.channels.input),
            sequence_gaps = peer.sequence_gaps,
            backpressure = peer.backpressure,
            malformed = peer.malformed,
            last_error = peer.last_error,
        }
    end
    return {
        role = self._role,
        state = self._state,
        capacity = self._max_guests,
        peer_count = #self._order,
        queue_limit = self._queue_limit,
        buffered_amount_limit = self._buffered_amount_limit,
        event_depth = #self._events,
        sent = self._sent,
        received = self._received,
        dropped_outbound = self._dropped_outbound,
        dropped_inbound = self._dropped_inbound,
        malformed = self._malformed,
        unsupported_version = self._unsupported_version,
        overflow = self._overflow,
        backpressure = self._backpressure,
        last_error = self._last_error,
        peers = peers,
    }
end

return FakeStarTransport
