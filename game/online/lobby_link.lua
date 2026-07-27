-- Carries lobby control traffic over a star transport.
--
-- The session protocol bounds a control wire at 8,192 bytes while the transport
-- envelope bounds one payload at 1,024, so a manifest proposal (about 2.5 kB)
-- cannot travel as a single message. This module adds the bounded framing the
-- transport bridge document leaves to the lobby: a control wire is split into
-- at most `MAX_FRAMES` ordered chunks over the reliable, ordered control
-- channel and reassembled on arrival. Framing is deliberately dumb — no
-- retransmission, no interleaving, no session state — because the control
-- channel already guarantees order and delivery.
--
-- It also drives the manual signaling handshake and translates transport
-- traffic into the events the pure lobby model consumes. Nothing here decides
-- lobby policy: it executes effects and reports facts.

local contract = require("game.transport.contract")
local protocol = require("game.online.protocol")

---@class LobbyFrameBuffer
---@field count integer -- Frames expected for the wire in progress, 0 when idle.
---@field next integer -- Next frame index expected.
---@field bytes integer -- Accumulated payload bytes, bounded by MAX_WIRE_BYTES.
---@field chunks string[]

---@alias LobbyLinkEvent
---| { kind: "signal", peer_id: string, signal: string }
---| { kind: "peer_connected", peer_id: string }
---| { kind: "link_lost", link_id: string, detail: string? }
---| { kind: "control", link_id: string, wire: string }
---| { kind: "link_error", link_id: string?, code: TransportErrorCode?, detail: string? }

---@class LobbyLink
---@field star StarTransportAdapter
---@field _buffers table<string, LobbyFrameBuffer>
---@field _sequence table<string, integer>
---@field _pending table<string, boolean> -- Peers whose local signal blob is awaited.
---@field _closing { link_id: string, detail: string? }[] -- Links closed on the next pump.
local LobbyLink = {}
LobbyLink.__index = LobbyLink

---@class LobbyLinkModule
local lobby_link = {}

lobby_link.FRAME_PREFIX = "GCLF;1;"
lobby_link.MAX_CHUNK_BYTES = 960
lobby_link.MAX_FRAMES = 9

---@param wire string
---@return string[]?, string?
function lobby_link.frame(wire)
    if type(wire) ~= "string" or #wire == 0 then
        return nil, "a control wire must be a non-empty string"
    end
    if #wire > protocol.MAX_WIRE_BYTES then
        return nil, "control wire exceeds the protocol bound"
    end
    local count = math.ceil(#wire / lobby_link.MAX_CHUNK_BYTES)
    if count > lobby_link.MAX_FRAMES then
        return nil, "control wire needs more frames than the bound allows"
    end
    ---@type string[]
    local frames = {}
    for index = 1, count do
        local first = (index - 1) * lobby_link.MAX_CHUNK_BYTES + 1
        local last = math.min(#wire, first + lobby_link.MAX_CHUNK_BYTES - 1)
        frames[index] = ("%s%d;%d;%s"):format(
            lobby_link.FRAME_PREFIX,
            index,
            count,
            wire:sub(first, last)
        )
    end
    return frames
end

---@return LobbyFrameBuffer
function lobby_link.new_buffer()
    return { count = 0, next = 1, bytes = 0, chunks = {} }
end

-- Feeds one payload into a reassembly buffer. Returns the complete wire once
-- the final frame lands, `nil` while more are expected, and `nil, err` when the
-- stream is malformed — which the caller must treat as a protocol violation
-- rather than silently resynchronising.
---@param buffer LobbyFrameBuffer
---@param payload string
---@return string?, string?
function lobby_link.absorb(buffer, payload)
    if type(payload) ~= "string" then
        return nil, "a control frame must be a string"
    end
    local raw_index, raw_count, chunk = payload:match("^GCLF;1;(%d+);(%d+);(.*)$")
    if not raw_index then
        return nil, "control frame is not canonical"
    end
    local index = math.floor(tonumber(raw_index) or -1)
    local count = math.floor(tonumber(raw_count) or -1)
    if count < 1 or count > lobby_link.MAX_FRAMES then
        return nil, "control frame count is outside the bound"
    end
    if index < 1 or index > count then
        return nil, "control frame index is outside its own stream"
    end
    if #chunk > lobby_link.MAX_CHUNK_BYTES then
        return nil, "control frame chunk exceeds the bound"
    end
    if buffer.count == 0 then
        if index ~= 1 then
            return nil, "control stream started mid-wire"
        end
        buffer.count = count
    elseif count ~= buffer.count or index ~= buffer.next then
        return nil, "control frame arrived out of order"
    end
    buffer.bytes = buffer.bytes + #chunk
    if buffer.bytes > protocol.MAX_WIRE_BYTES then
        return nil, "reassembled control wire exceeds the protocol bound"
    end
    buffer.chunks[#buffer.chunks + 1] = chunk
    if index < count then
        buffer.next = index + 1
        return nil
    end
    local wire = table.concat(buffer.chunks)
    buffer.count = 0
    buffer.next = 1
    buffer.bytes = 0
    buffer.chunks = {}
    return wire
end

---@param star StarTransportAdapter
---@return LobbyLink
function lobby_link.new(star)
    return setmetatable({
        star = star,
        _buffers = {},
        _sequence = {},
        _pending = {},
        _closing = {},
    }, LobbyLink)
end

---@param peer_id string
---@return LobbyFrameBuffer
function LobbyLink:_buffer(peer_id)
    local buffer = self._buffers[peer_id]
    if not buffer then
        buffer = lobby_link.new_buffer()
        self._buffers[peer_id] = buffer
    end
    return buffer
end

---@param peer_id string
---@return integer
function LobbyLink:_next_sequence(peer_id)
    local sequence = self._sequence[peer_id] or 0
    self._sequence[peer_id] = sequence + 1
    return sequence
end

---@param peer_id string
---@param wire string
---@return boolean?, string?
function LobbyLink:send(peer_id, wire)
    local frames, err = lobby_link.frame(wire)
    if not frames then
        return nil, err
    end
    for _, payload in ipairs(frames) do
        local message, message_err = contract.new({
            type = "event",
            seq = self:_next_sequence(peer_id),
            payload = payload,
        })
        if not message then
            return nil, message_err
        end
        local ok, send_err = self.star:send(peer_id, "control", message)
        if not ok then
            return nil, send_err
        end
    end
    return true
end

-- Executes one lobby effect. Transport failures are returned rather than
-- raised: a rejected paste is an ordinary lobby outcome, not a crash.
---@param effect table
---@return boolean?, string?
function LobbyLink:apply(effect)
    local kind = effect.kind
    if kind == "open_peer" then
        local slot, open_err = self.star:open_peer(effect.peer_id)
        if not slot then
            return nil, open_err
        end
        return true
    elseif kind == "request_offer" then
        self._pending[effect.peer_id] = true
        return self.star:request_offer(effect.peer_id)
    elseif kind == "accept_offer" then
        self._pending[contract.HOST_PEER_ID] = true
        return self.star:accept_offer(effect.signal)
    elseif kind == "accept_answer" then
        return self.star:accept_answer(effect.peer_id, effect.signal)
    elseif kind == "send" then
        return self:send(effect.link_id, effect.wire)
    elseif kind == "close" then
        -- A coordinator announces on a link and closes it in the same batch.
        -- An adapter is only required to *queue* the announcement, so closing
        -- immediately can discard it. The close is therefore deferred by one
        -- pump, which every adapter has to complete anyway.
        self._closing[#self._closing + 1] = { link_id = effect.link_id, detail = effect.detail }
        return true
    elseif kind == "shutdown" then
        return self.star:shutdown()
    end
    return true
end

---@param events LobbyLinkEvent[]
function LobbyLink:_drain_signals(events)
    for _, peer_id in ipairs(self.star:peer_ids()) do
        if self._pending[peer_id] then
            local signal = self.star:take_signal(peer_id)
            if signal then
                self._pending[peer_id] = nil
                events[#events + 1] = { kind = "signal", peer_id = peer_id, signal = signal }
            end
        end
    end
end

---@param events LobbyLinkEvent[]
function LobbyLink:_drain_events(events)
    local event = self.star:poll_event()
    while event do
        if event.kind == "peer_state" and event.peer_id then
            if event.state == "connected" then
                events[#events + 1] = { kind = "peer_connected", peer_id = event.peer_id }
            elseif
                event.state == "error"
                or event.state == "closed"
                or event.state == "disconnected"
            then
                events[#events + 1] = {
                    kind = "link_lost",
                    link_id = event.peer_id,
                    detail = event.message,
                }
            end
        elseif event.kind == "peer_error" or event.kind == "star_error" then
            events[#events + 1] = {
                kind = "link_error",
                link_id = event.peer_id,
                code = event.code,
                detail = event.message,
            }
        end
        event = self.star:poll_event()
    end
end

---@param events LobbyLinkEvent[]
function LobbyLink:_drain_messages(events)
    for _, received in ipairs(self.star:poll_batch()) do
        if received.channel == "control" then
            local wire, err =
                lobby_link.absorb(self:_buffer(received.peer_id), received.message.payload)
            if wire then
                events[#events + 1] = { kind = "control", link_id = received.peer_id, wire = wire }
            elseif err then
                self._buffers[received.peer_id] = lobby_link.new_buffer()
                events[#events + 1] = {
                    kind = "link_error",
                    link_id = received.peer_id,
                    code = "malformed",
                    detail = err,
                }
            end
        end
    end
end

-- One pump of the transport: locally produced signaling blobs first, then
-- connection events, then reassembled control wires. The order is fixed so a
-- scripted lobby flow is reproducible.
---@return LobbyLinkEvent[]
function LobbyLink:poll()
    ---@type LobbyLinkEvent[]
    local events = {}
    for _, closing in ipairs(self._closing) do
        self.star:close_peer(closing.link_id, closing.detail)
        self._buffers[closing.link_id] = nil
    end
    self._closing = {}
    self:_drain_signals(events)
    self:_drain_events(events)
    self:_drain_messages(events)
    return events
end

return lobby_link
