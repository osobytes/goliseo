local contract = require("game.transport.contract")

-- Pure in-process **relay** transport: a third `StarTransportAdapter`
-- implementation alongside `fake_star` and `browser_star`, built to measure the
-- OMP-4 relay topology in the #169 fault harness before any server exists.
--
-- # What makes it a relay rather than a star
--
-- In `fake_star` one endpoint *is* the hub: a host `broadcast` enqueues one copy
-- per guest link on the host's own uplink, and a guest may address nobody but
-- the host. Here every endpoint holds exactly **one** physical link, to the
-- room, and the room is not a player:
--
--  * `broadcast` costs **one** uplink copy no matter how many members receive
--    it. That single difference is the whole bandwidth claim under test.
--  * Any member may address any other member. No role is privileged, and
--    `role()` is carried only because the adapter contract and the session layer
--    above still name one. The room enforces nothing on it.
--  * There is no manual signaling. `request_offer`, `accept_offer`,
--    `accept_answer` and `take_signal` cannot mean anything when the far end is
--    a server the client dials, so they refuse rather than pretend. The room is
--    the rendezvous.
--
-- # The room never parses a game packet
--
-- This is the property the topology decision rests on, so it is structural here
-- rather than promised. An endpoint encodes its own outbound message into the
-- contract's peer-addressed wire (`origin|channel|<envelope>`) *before* handing
-- it up. `FakeRelayRoom` only ever appends those opaque strings to a
-- per-destination list, concatenates the list with a newline once per forward
-- pass, and hands the frame down. It calls no decoder, reads no field, and knows
-- nothing about slots, ticks, ownership, or canonicalisation. The receiving
-- endpoint splits the frame and decodes each line.
--
-- The only thing the room reads is the **destination set** attached to each unit
-- by the sending adapter, which is link identity — the same thing a real relay
-- reads from the data channel a packet arrived on plus the room it belongs to.
--
-- A member never receives its own line back: `forward` skips the origin.
--
-- # Byte accounting
--
-- `uplink_bytes` and `downlink_bytes` count encoded envelope wires, one count
-- per copy that actually crosses the link, which is exactly how `fake_star`
-- counts. On a star a seven-guest `broadcast` is seven uplink copies; here it is
-- one. `frame_overhead_bytes` is kept separately so a comparison never has to
-- guess whether framing was folded into the payload figure.

---@class FakeRelayRoomCounters
---@field frames integer -- Framed downlink messages the room emitted.
---@field lines integer -- Opaque lines forwarded, counted once per destination.
---@field dropped integer -- Lines with no connected destination left.

---@class FakeRelayRoom
---@field members string[] -- Join order; the only ordering the room has.
---@field endpoints table<string, FakeRelayTransport>
---@field inbox table<string, string[]> -- Destination peer id -> opaque lines this pass.
---@field counters FakeRelayRoomCounters

---@class FakeRelayTransportOptions
---@field role TransportRole? -- Carried for the session layer; the room ignores it.
---@field peer_id string -- Room-unique member identity.
---@field room FakeRelayRoom? -- Share one across every member of a single room.
---@field queue_limit integer?
---@field max_peers integer?
---@field buffered_amount_limit integer?

---@class FakeRelayChannel
---@field state TransportPeerState
---@field inbound TransportPeerMessage[]
---@field received integer
---@field dropped_inbound integer
---@field last_seq integer?

---@class FakeRelayPeer
---@field peer_id string
---@field slot integer
---@field state TransportPeerState
---@field ice_state string
---@field channels table<TransportChannel, FakeRelayChannel>
---@field arrival_seq integer
---@field sequence_gaps integer
---@field backpressure integer
---@field malformed integer
---@field sent integer -- Uplink units addressed to this member and released.
---@field dropped_outbound integer
---@field last_error string?

---@class FakeRelayUnit
---@field channel TransportChannel
---@field targets string[] -- Member ids resolved at send time; canonical order.
---@field target_set table<string, boolean>
---@field line string -- `origin|channel|<envelope wire>`; opaque to the room.
---@field bytes integer -- Encoded envelope wire length, excluding the frame header.

---@class FakeRelayUplink
---@field units FakeRelayUnit[]
---@field buffered_amount integer
---@field backpressured boolean
---@field sent integer
---@field dropped_outbound integer

---@class FakeRelayTransport: StarTransportAdapter
---@field _role TransportRole
---@field _peer_id string
---@field _state TransportState
---@field _queue_limit integer
---@field _event_limit integer
---@field _max_peers integer
---@field _buffered_amount_limit integer
---@field _room FakeRelayRoom
---@field _peers table<string, FakeRelayPeer>
---@field _order string[]
---@field _uplink table<TransportChannel, FakeRelayUplink>
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
---@field _uplink_bytes integer
---@field _downlink_bytes integer
---@field _input_uplink_bytes integer
---@field _input_downlink_bytes integer
---@field _uplink_units integer
---@field _downlink_frames integer
---@field _frame_overhead integer
---@field _last_error string?
local FakeRelayTransport = {}
FakeRelayTransport.__index = FakeRelayTransport

-- One room. Members reach each other only through a room they were all handed,
-- so two rooms in one process cannot cross-talk — the same explicitness
-- `fake_star.new_rendezvous` buys for a single logical star.
---@return FakeRelayRoom
function FakeRelayTransport.new_room()
    return {
        members = {},
        endpoints = {},
        inbox = {},
        counters = { frames = 0, lines = 0, dropped = 0 },
    }
end

---@param code TransportErrorCode
---@param message string
---@return nil, string, TransportErrorCode
local function failure(code, message)
    return nil, message, code
end

---@return FakeRelayChannel
local function new_channel()
    return {
        state = "connecting",
        inbound = {},
        received = 0,
        dropped_inbound = 0,
        last_seq = nil,
    }
end

---@return FakeRelayUplink
local function new_uplink()
    return {
        units = {},
        buffered_amount = 0,
        backpressured = false,
        sent = 0,
        dropped_outbound = 0,
    }
end

---@param value string
---@param separator string
---@return string[]
local function split(value, separator)
    local fields = {}
    local start = 1
    while true do
        local index = value:find(separator, start, true)
        if not index then
            fields[#fields + 1] = value:sub(start)
            return fields
        end
        fields[#fields + 1] = value:sub(start, index - 1)
        start = index + #separator
    end
end

---@param options FakeRelayTransportOptions
---@return FakeRelayTransport
function FakeRelayTransport.new(options)
    assert(type(options) == "table", "a fake relay transport needs options")
    local role = options.role or "guest"
    assert(role == "host" or role == "guest", "fake relay transport role must be host or guest")
    local queue_limit = options.queue_limit or contract.DEFAULT_QUEUE_LIMIT
    assert(
        queue_limit == math.floor(queue_limit)
            and queue_limit > 0
            and queue_limit <= contract.MAX_QUEUE_LIMIT,
        "fake relay transport queue_limit is outside the supported range"
    )
    local max_peers = options.max_peers or contract.MAX_GUESTS
    assert(
        max_peers == math.floor(max_peers) and max_peers > 0 and max_peers <= contract.MAX_GUESTS,
        "fake relay transport max_peers is outside the supported range"
    )
    local buffered_amount_limit = options.buffered_amount_limit
        or contract.DEFAULT_BUFFERED_AMOUNT_LIMIT
    assert(
        buffered_amount_limit == math.floor(buffered_amount_limit)
            and buffered_amount_limit > 0
            and buffered_amount_limit <= contract.MAX_BUFFERED_AMOUNT_LIMIT,
        "fake relay transport buffered_amount_limit is outside the supported range"
    )
    local peer_id = assert(options.peer_id, "a fake relay member needs its own peer id")
    assert(contract.validate_peer_id(peer_id))
    return setmetatable({
        _role = role,
        _peer_id = peer_id,
        _state = "new",
        _queue_limit = queue_limit,
        _event_limit = math.max(2, queue_limit),
        _max_peers = max_peers,
        _buffered_amount_limit = buffered_amount_limit,
        _room = options.room or FakeRelayTransport.new_room(),
        _peers = {},
        _order = {},
        _uplink = { control = new_uplink(), input = new_uplink() },
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
        _uplink_bytes = 0,
        _downlink_bytes = 0,
        _input_uplink_bytes = 0,
        _input_downlink_bytes = 0,
        _uplink_units = 0,
        _downlink_frames = 0,
        _frame_overhead = 0,
        _last_error = nil,
    }, FakeRelayTransport)
end

---@param event TransportPeerEvent
function FakeRelayTransport:_push_event(event)
    if #self._events >= self._event_limit then
        table.remove(self._events, 1)
        self._overflow = self._overflow + 1
        self._last_error = "fake relay transport event queue is full"
    end
    self._events[#self._events + 1] = event
end

---@param code TransportErrorCode
---@param message string
---@param peer FakeRelayPeer?
---@param channel TransportChannel?
function FakeRelayTransport:_record_error(code, message, peer, channel)
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

---@param peer FakeRelayPeer
---@param state TransportPeerState
function FakeRelayTransport:_set_peer_state(peer, state)
    peer.state = state
    for _, channel in ipairs(contract.CHANNEL_ORDER) do
        peer.channels[channel].state = state
    end
    self:_push_event({ kind = "peer_state", peer_id = peer.peer_id, state = state })
end

---@param state TransportState
function FakeRelayTransport:_set_state(state)
    self._state = state
    self:_push_event({ kind = "star_state", state = state })
end

---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:_require_connected()
    if self._state == "new" then
        return failure("not_initialized", "relay transport is not initialized")
    end
    if self._state == "closed" then
        return failure("closed", "relay transport is closed")
    end
    if self._state ~= "connected" then
        return failure("not_connected", "relay transport is not connected")
    end
    return true
end

---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function FakeRelayTransport:_add_peer(peer_id)
    local slot = #self._order + 1
    ---@type FakeRelayPeer
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
        sent = 0,
        dropped_outbound = 0,
        last_error = nil,
    }
    self._peers[peer_id] = peer
    self._order[slot] = peer_id
    self:_push_event({ kind = "peer_state", peer_id = peer_id, state = "connecting" })
    return slot
end

-- Joining the room is the whole handshake. Every member already in the room
-- gains a link to the newcomer and the newcomer gains one to each of them, in
-- join order, so slot numbering is a deterministic function of arrival and not
-- of any hash order.
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:initialize()
    if self._state == "connected" then
        return true
    end
    local room = self._room
    if room.endpoints[self._peer_id] ~= nil and room.endpoints[self._peer_id] ~= self then
        return failure("duplicate_peer", "the relay room already holds that member id")
    end
    self:_set_state("connected")
    room.endpoints[self._peer_id] = self
    room.inbox[self._peer_id] = room.inbox[self._peer_id] or {}
    local joined = false
    for _, member_id in ipairs(room.members) do
        if member_id == self._peer_id then
            joined = true
        end
    end
    if not joined then
        room.members[#room.members + 1] = self._peer_id
    end
    for _, member_id in ipairs(room.members) do
        if member_id ~= self._peer_id then
            local other = room.endpoints[member_id]
            if other ~= nil and other._state == "connected" then
                self:_connect_to(other)
                other:_connect_to(self)
            end
        end
    end
    return true
end

-- Bring one directed link up. Both endpoints call this on each other, which is
-- what makes a member's peer table symmetric without either side being "the"
-- opener.
---@param other FakeRelayTransport
function FakeRelayTransport:_connect_to(other)
    local peer = self._peers[other._peer_id]
    if peer == nil then
        if #self._order >= self._max_peers then
            self:_record_error("capacity", "relay room membership is at capacity")
            return
        end
        assert(self:_add_peer(other._peer_id))
        peer = assert(self._peers[other._peer_id])
    end
    if peer.state == "connected" then
        return
    end
    peer.ice_state = "connected"
    self:_set_peer_state(peer, "connected")
end

---@param peer_id string
---@return FakeRelayPeer?, string?, TransportErrorCode?
function FakeRelayTransport:_peer(peer_id)
    local peer = self._peers[peer_id]
    if not peer then
        return failure("unknown_peer", "relay transport has no member with that id")
    end
    return peer
end

-- Declaring a member explicitly. A relay has no privileged opener, so unlike the
-- star this is not host-only: it is here because the adapter contract names it,
-- and because a caller may want a link before the far member has joined.
---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function FakeRelayTransport:open_peer(peer_id)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        self:_record_error(code or "malformed", err or "invalid peer id")
        return nil, err, code
    end
    if peer_id == self._peer_id then
        self:_record_error("duplicate_peer", "a relay member cannot address itself")
        return failure("duplicate_peer", "a relay member cannot address itself")
    end
    if self._peers[peer_id] then
        self:_record_error("duplicate_peer", "relay member is already open")
        return failure("duplicate_peer", "relay member is already open")
    end
    if #self._order >= self._max_peers then
        self:_record_error("capacity", "relay room membership is at capacity")
        return failure("capacity", "relay room membership is at capacity")
    end
    return self:_add_peer(peer_id)
end

---@param peer FakeRelayPeer
function FakeRelayTransport:_drop_peer_queues(peer)
    for _, channel_name in ipairs(contract.CHANNEL_ORDER) do
        local channel = peer.channels[channel_name]
        self._dropped_inbound = self._dropped_inbound + #channel.inbound
        channel.dropped_inbound = channel.dropped_inbound + #channel.inbound
        channel.inbound = {}
    end
    -- Uplink units addressed only to this member have nowhere left to go. They
    -- are dropped here rather than at flush so teardown drains, exactly as the
    -- star drops a closed link's outbound queue.
    for _, channel_name in ipairs(contract.CHANNEL_ORDER) do
        local uplink = self._uplink[channel_name]
        ---@type FakeRelayUnit[]
        local kept = {}
        for _, unit in ipairs(uplink.units) do
            if unit.target_set[peer.peer_id] then
                unit.target_set[peer.peer_id] = nil
                ---@type string[]
                local targets = {}
                for _, target in ipairs(unit.targets) do
                    if target ~= peer.peer_id then
                        targets[#targets + 1] = target
                    end
                end
                unit.targets = targets
            end
            if #unit.targets > 0 then
                kept[#kept + 1] = unit
            else
                uplink.buffered_amount = math.max(0, uplink.buffered_amount - unit.bytes)
                uplink.dropped_outbound = uplink.dropped_outbound + 1
                peer.dropped_outbound = peer.dropped_outbound + 1
                self._dropped_outbound = self._dropped_outbound + 1
            end
        end
        uplink.units = kept
    end
end

-- Closes one member link without disturbing the rest of the room. The remote
-- member is told, because a relay does know when a client's link to it drops.
---@param peer_id string
---@param reason string?
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:close_peer(peer_id, reason)
    local peer, err, code = self:_peer(peer_id)
    if not peer then
        return nil, err, code
    end
    if peer.state == "closed" then
        return true
    end
    local detail = reason or "relay member closed"
    self:_drop_peer_queues(peer)
    peer.ice_state = "closed"
    self:_set_peer_state(peer, "closed")
    self:_record_error("disconnected", detail, peer)
    local other = self._room.endpoints[peer_id]
    if other ~= nil and other ~= self then
        local remote = other._peers[self._peer_id]
        if remote ~= nil and remote.state ~= "closed" then
            other:_drop_peer_queues(remote)
            remote.ice_state = "closed"
            other:_set_peer_state(remote, "disconnected")
            other:_record_error("disconnected", detail, remote)
        end
    end
    return true
end

---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:shutdown()
    if self._state == "closed" then
        return true
    end
    for _, peer_id in ipairs(self._order) do
        self:close_peer(peer_id, "relay transport shutdown")
    end
    self._peers = {}
    self._order = {}
    self._cursor = 0
    self._uplink = { control = new_uplink(), input = new_uplink() }
    local room = self._room
    room.endpoints[self._peer_id] = nil
    room.inbox[self._peer_id] = nil
    ---@type string[]
    local members = {}
    for _, member_id in ipairs(room.members) do
        if member_id ~= self._peer_id then
            members[#members + 1] = member_id
        end
    end
    room.members = members
    self:_set_state("closed")
    return true
end

-- ---------------------------------------------------------------------------
-- The uplink
-- ---------------------------------------------------------------------------

-- One unit on the single link to the room, whatever its destination set. This is
-- the whole topological difference from `fake_star`, where the same call costs
-- one queued copy per destination.
---@param channel_name TransportChannel
---@param message TransportMessage
---@param targets string[]
---@param attributed FakeRelayPeer?
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:_enqueue(channel_name, message, targets, attributed)
    local uplink = self._uplink[channel_name]
    if #uplink.units >= self._queue_limit then
        uplink.dropped_outbound = uplink.dropped_outbound + 1
        self._dropped_outbound = self._dropped_outbound + 1
        self:_record_error("overflow", "relay uplink queue is full", attributed, channel_name)
        return failure("overflow", "relay uplink queue is full")
    end
    local wire = assert(contract.encode(message))
    local line = assert(contract.encode_addressed(self._peer_id, channel_name, message))
    ---@type table<string, boolean>
    local target_set = {}
    for _, target in ipairs(targets) do
        target_set[target] = true
    end
    uplink.units[#uplink.units + 1] = {
        channel = channel_name,
        targets = targets,
        target_set = target_set,
        line = line,
        bytes = #wire,
    }
    uplink.buffered_amount = uplink.buffered_amount + #wire
    uplink.sent = uplink.sent + 1
    self._sent = self._sent + 1
    self._uplink_units = self._uplink_units + 1
    self._uplink_bytes = self._uplink_bytes + #wire
    if channel_name == "input" then
        self._input_uplink_bytes = self._input_uplink_bytes + #wire
    end
    self._frame_overhead = self._frame_overhead + (#line - #wire)
    return true
end

-- Address one member. Any member may address any other: the relay has no
-- privileged direction, which is precisely what removes the sequencer.
---@param peer_id string
---@param channel TransportChannel
---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:send(peer_id, channel, message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    local peer = self._peers[peer_id]
    ok, err, code = contract.validate_channel_message(channel, message)
    if not ok then
        self:_record_error(code or "malformed", err or "relay transport rejected a message", peer)
        return nil, err, code
    end
    if not peer then
        self:_record_error("unknown_peer", "relay transport has no member with that id")
        return failure("unknown_peer", "relay transport has no member with that id")
    end
    if peer.state ~= "connected" then
        self:_record_error("not_connected", "relay member link is not connected", peer, channel)
        return failure("not_connected", "relay member link is not connected")
    end
    return self:_enqueue(channel, message, { peer_id }, peer)
end

-- Fan-out for the price of one upload. Returns how many members the room will
-- frame this to; per-member failures stay visible through events and
-- diagnostics, exactly as on the star.
---@param channel TransportChannel
---@param message TransportMessage
---@return integer?, string?, TransportErrorCode?
function FakeRelayTransport:broadcast(channel, message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate_channel_message(channel, message)
    if not ok then
        self:_record_error(code or "malformed", err or "relay transport rejected a message")
        return nil, err, code
    end
    ---@type string[]
    local targets = {}
    for _, peer_id in ipairs(self._order) do
        if self._peers[peer_id].state == "connected" then
            targets[#targets + 1] = peer_id
        end
    end
    if #targets == 0 then
        return 0
    end
    local enqueued, enqueue_err, enqueue_code = self:_enqueue(channel, message, targets, nil)
    if not enqueued then
        return nil, enqueue_err, enqueue_code
    end
    return #targets
end

-- ---------------------------------------------------------------------------
-- The room
-- ---------------------------------------------------------------------------

-- Test seam, mirroring `fake_star:pump`. Flush every member's uplink into the
-- room in join order, then let the room frame one message per destination.
---@param room FakeRelayRoom
function FakeRelayTransport.pump_room(room)
    for _, member_id in ipairs(room.members) do
        local endpoint = room.endpoints[member_id]
        if endpoint ~= nil then
            endpoint:_flush()
        end
    end
    FakeRelayTransport.forward_room(room)
end

function FakeRelayTransport:pump()
    FakeRelayTransport.pump_room(self._room)
end

-- The relay itself. It concatenates the opaque lines it received for each
-- destination this pass and hands the destination one frame. It decodes nothing
-- and it never returns a line to its own origin — origin is decided by which
-- member handed the line up, never by reading it.
---@param room FakeRelayRoom
function FakeRelayTransport.forward_room(room)
    for _, member_id in ipairs(room.members) do
        local lines = room.inbox[member_id]
        if lines ~= nil and #lines > 0 then
            local frame = table.concat(lines, "\n")
            room.inbox[member_id] = {}
            room.counters.frames = room.counters.frames + 1
            local endpoint = room.endpoints[member_id]
            if endpoint ~= nil then
                endpoint:_receive_frame(frame)
            else
                room.counters.dropped = room.counters.dropped + #lines
            end
        end
    end
end

-- Drain the single uplink under the same `bufferedAmount` model the star uses,
-- with one difference that is the point: the budget is per *link*, and this
-- endpoint has one link however many members are listening.
function FakeRelayTransport:_flush()
    local room = self._room
    for _, channel_name in ipairs(contract.CHANNEL_ORDER) do
        local uplink = self._uplink[channel_name]
        local budget = self._buffered_amount_limit
        while #uplink.units > 0 do
            local unit = uplink.units[1]
            if unit.bytes > budget then
                if not uplink.backpressured then
                    uplink.backpressured = true
                    self._backpressure = self._backpressure + 1
                    self._last_error = "relay uplink send buffer is full"
                    for _, target in ipairs(unit.targets) do
                        local peer = self._peers[target]
                        if peer ~= nil then
                            peer.backpressure = peer.backpressure + 1
                            peer.last_error = self._last_error
                        end
                    end
                    self:_push_event({
                        kind = "peer_error",
                        peer_id = unit.targets[1],
                        channel = channel_name,
                        code = "backpressure",
                        message = self._last_error,
                    })
                end
                break
            end
            table.remove(uplink.units, 1)
            uplink.backpressured = false
            budget = budget - unit.bytes
            uplink.buffered_amount = math.max(0, uplink.buffered_amount - unit.bytes)
            local delivered = 0
            for _, target in ipairs(unit.targets) do
                local peer = self._peers[target]
                local remote = room.endpoints[target]
                if peer ~= nil and peer.state == "connected" and remote ~= nil then
                    local inbox = room.inbox[target]
                    if inbox ~= nil then
                        inbox[#inbox + 1] = unit.line
                        room.counters.lines = room.counters.lines + 1
                        peer.sent = peer.sent + 1
                        delivered = delivered + 1
                    end
                end
            end
            if delivered == 0 then
                uplink.dropped_outbound = uplink.dropped_outbound + 1
                self._dropped_outbound = self._dropped_outbound + 1
                room.counters.dropped = room.counters.dropped + 1
            end
        end
    end
end

-- Split the frame the room handed down and decode each line back into an
-- addressed envelope. The origin comes from the line the *sender* wrote, and is
-- checked against this endpoint's own member table: a line from a member this
-- endpoint does not hold a link to is counted, not delivered.
---@param frame string
function FakeRelayTransport:_receive_frame(frame)
    self._downlink_frames = self._downlink_frames + 1
    for _, line in ipairs(split(frame, "\n")) do
        local addressed, err, code = contract.decode_addressed(line)
        if addressed == nil then
            self:_record_error(code or "malformed", err or "relay frame line is invalid")
        else
            local peer = self._peers[addressed.peer_id]
            if peer == nil or peer.state ~= "connected" then
                self._dropped_inbound = self._dropped_inbound + 1
            else
                local wire_bytes = #line - #addressed.peer_id - #addressed.channel - 2
                self._downlink_bytes = self._downlink_bytes + wire_bytes
                if addressed.channel == "input" then
                    self._input_downlink_bytes = self._input_downlink_bytes + wire_bytes
                end
                self:_receive(peer, addressed.channel, addressed)
            end
        end
    end
end

---@param peer FakeRelayPeer
---@param channel_name TransportChannel
---@param addressed TransportAddressedMessage
function FakeRelayTransport:_receive(peer, channel_name, addressed)
    local channel = peer.channels[channel_name]
    if #channel.inbound >= self._queue_limit then
        channel.dropped_inbound = channel.dropped_inbound + 1
        self._dropped_inbound = self._dropped_inbound + 1
        self:_record_error("overflow", "relay member inbound queue is full", peer, channel_name)
        return
    end
    local seq = addressed.message.seq
    if channel.last_seq and seq > channel.last_seq + 1 then
        peer.sequence_gaps = peer.sequence_gaps + seq - channel.last_seq - 1
    end
    if not channel.last_seq or seq > channel.last_seq then
        channel.last_seq = seq
    end
    -- `arrival_seq` is stamped at poll time, for the same reason the star does
    -- it there: it must mean the same thing on every adapter.
    channel.inbound[#channel.inbound + 1] = {
        peer_id = peer.peer_id,
        channel = channel_name,
        arrival_seq = 0,
        message = addressed.message,
    }
end

-- ---------------------------------------------------------------------------
-- Draining
-- ---------------------------------------------------------------------------

-- The same persistent (slot, channel-rank) cursor the star uses, so release
-- order is a property of the contract rather than of the topology and the two
-- adapters can be compared without that as a variable.
---@return TransportPeerMessage?
function FakeRelayTransport:_take()
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
            peer.arrival_seq = peer.arrival_seq + 1
            entry.arrival_seq = peer.arrival_seq
            return entry
        end
    end
    return nil
end

---@param limit integer?
---@return TransportPeerMessage[]
function FakeRelayTransport:poll_batch(limit)
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
function FakeRelayTransport:poll()
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    return self:poll_batch(1)[1]
end

---@return TransportPeerEvent?
function FakeRelayTransport:poll_event()
    return table.remove(self._events, 1)
end

-- ---------------------------------------------------------------------------
-- Signaling: refused, not faked
-- ---------------------------------------------------------------------------

-- A relay client dials a server with a known address. There is no offer to hand
-- to a human, no answer to paste back, and no ICE state worth reporting on an
-- ICE-lite endpoint. Returning a plausible token would let a caller believe the
-- manual handshake still exists; refusing makes the four methods that the
-- topology deletes visible at the seam instead.
---@return nil, string, TransportErrorCode
local function no_signaling()
    return failure(
        "signal_error",
        "a relay endpoint has no manual signaling; the room is the rendezvous"
    )
end

---@param _peer_id string
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:request_offer(_peer_id)
    self:_record_error("signal_error", "relay endpoints do not create offers")
    return no_signaling()
end

---@param _signal string
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:accept_offer(_signal)
    self:_record_error("signal_error", "relay endpoints do not accept offers")
    return no_signaling()
end

---@param _peer_id string
---@param _signal string
---@return boolean?, string?, TransportErrorCode?
function FakeRelayTransport:accept_answer(_peer_id, _signal)
    self:_record_error("signal_error", "relay endpoints do not accept answers")
    return no_signaling()
end

---@param _peer_id string
---@return string?, string?, TransportErrorCode?
function FakeRelayTransport:take_signal(_peer_id)
    return nil
end

-- ---------------------------------------------------------------------------
-- Reporting
-- ---------------------------------------------------------------------------

---@return TransportRole
function FakeRelayTransport:role()
    return self._role
end

---@return integer
function FakeRelayTransport:capacity()
    return self._max_peers
end

---@return string[]
function FakeRelayTransport:peer_ids()
    local ids = {}
    for index, peer_id in ipairs(self._order) do
        ids[index] = peer_id
    end
    return ids
end

---@param peer_id string
---@return TransportPeerState?
function FakeRelayTransport:peer_state(peer_id)
    local peer = self._peers[peer_id]
    return peer and peer.state or nil
end

---@return TransportState
function FakeRelayTransport:state()
    return self._state
end

-- Encoded envelope wires this endpoint put on its uplink and took off its
-- downlink, one count per copy that crossed the link. On a star a `broadcast`
-- to seven guests is seven uplink copies; here it is one, and that difference is
-- the measurement the relay topology decision turns on.
---@return integer uplink_bytes, integer downlink_bytes
function FakeRelayTransport:wire_bytes()
    return self._uplink_bytes, self._downlink_bytes
end

---@class FakeRelayWireCounters
---@field uplink_bytes integer
---@field downlink_bytes integer
---@field input_uplink_bytes integer -- The `input` channel alone, which is the per-tick match cost.
---@field input_downlink_bytes integer
---@field uplink_units integer
---@field downlink_frames integer
---@field frame_overhead_bytes integer

---@return FakeRelayWireCounters
function FakeRelayTransport:wire_counters()
    return {
        uplink_bytes = self._uplink_bytes,
        downlink_bytes = self._downlink_bytes,
        input_uplink_bytes = self._input_uplink_bytes,
        input_downlink_bytes = self._input_downlink_bytes,
        uplink_units = self._uplink_units,
        downlink_frames = self._downlink_frames,
        frame_overhead_bytes = self._frame_overhead,
    }
end

---@param self FakeRelayTransport
---@param peer FakeRelayPeer
---@param channel_name TransportChannel
---@return TransportChannelDiagnostics
local function channel_diagnostics(self, peer, channel_name)
    local channel = peer.channels[channel_name]
    local uplink = self._uplink[channel_name]
    -- The uplink is shared, so a member's outbound view is the units still
    -- queued that are addressed to it. Byte figures follow the same rule, which
    -- keeps the depth gate meaningful without pretending the copies are real.
    local depth, buffered = 0, 0
    for _, unit in ipairs(uplink.units) do
        if unit.target_set[peer.peer_id] then
            depth = depth + 1
            buffered = buffered + unit.bytes
        end
    end
    return {
        state = channel.state,
        outbound_depth = depth,
        inbound_depth = #channel.inbound,
        buffered_amount = buffered,
        sent = peer.sent,
        received = channel.received,
        dropped_outbound = peer.dropped_outbound,
        dropped_inbound = channel.dropped_inbound,
    }
end

---@return TransportStarDiagnostics
function FakeRelayTransport:diagnostics()
    local peers = {}
    for index, peer_id in ipairs(self._order) do
        local peer = self._peers[peer_id]
        peers[index] = {
            peer_id = peer.peer_id,
            slot = peer.slot,
            state = peer.state,
            ice_state = peer.ice_state,
            control = channel_diagnostics(self, peer, "control"),
            input = channel_diagnostics(self, peer, "input"),
            sequence_gaps = peer.sequence_gaps,
            backpressure = peer.backpressure,
            malformed = peer.malformed,
            last_error = peer.last_error,
        }
    end
    return {
        role = self._role,
        state = self._state,
        capacity = self._max_peers,
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

-- Every method the star adapter contract names is implemented above. Asserted at
-- load time so a contract addition fails loudly rather than at the first call.
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
    assert(type(FakeRelayTransport[name]) == "function", "fake relay transport is missing " .. name)
end

return FakeRelayTransport
