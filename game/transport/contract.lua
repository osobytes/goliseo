---@alias TransportMessageType "input"|"event"|"state"
---@alias TransportState "new"|"connected"|"disconnected"|"closed"|"error"
---@alias TransportRole "host"|"guest"
---@alias TransportChannel "control"|"input"
---@alias TransportPeerState
---| "new"
---| "connecting"
---| "connected"
---| "disconnected"
---| "closed"
---| "error"
---@alias TransportErrorCode
---| "not_initialized"
---| "not_connected"
---| "closed"
---| "malformed"
---| "unsupported_version"
---| "payload_too_large"
---| "overflow"
---| "bridge_unavailable"
---| "bridge_error"
---| "disconnected"
---| "capacity"
---| "unknown_peer"
---| "duplicate_peer"
---| "role_forbidden"
---| "channel_mismatch"
---| "backpressure"
---| "signal_error"

---@class TransportMessage
---@field version integer
---@field type TransportMessageType
---@field seq integer -- Monotonic transport sequence, starting at zero.
---@field tick integer? -- Required for input messages; omitted for control messages.
---@field payload string -- UTF-8 payload owned by the next protocol layer.

---@class TransportMessageOptions
---@field version integer?
---@field type TransportMessageType
---@field seq integer
---@field tick integer?
---@field payload string

---@class TransportEvent
---@field kind "state"|"error"
---@field state TransportState?
---@field code TransportErrorCode?
---@field message string?

---@class TransportDiagnostics
---@field state TransportState
---@field queue_limit integer
---@field outbound_depth integer
---@field inbound_depth integer
---@field event_depth integer
---@field dropped_outbound integer
---@field dropped_inbound integer
---@field malformed integer
---@field unsupported_version integer
---@field overflow integer
---@field sent integer
---@field received integer
---@field last_error string?

---@class TransportChannelDiagnostics
---@field state TransportPeerState
---@field outbound_depth integer
---@field inbound_depth integer
---@field buffered_amount integer
---@field sent integer
---@field received integer
---@field dropped_outbound integer
---@field dropped_inbound integer

---@class TransportPeerDiagnostics
---@field peer_id string
---@field slot integer -- Stable star slot, 1..MAX_GUESTS on a host, always 1 on a guest.
---@field state TransportPeerState
---@field ice_state string
---@field control TransportChannelDiagnostics
---@field input TransportChannelDiagnostics
---@field sequence_gaps integer
---@field backpressure integer
---@field malformed integer
---@field last_error string?

---@class TransportStarDiagnostics
---@field role TransportRole
---@field state TransportState
---@field capacity integer
---@field peer_count integer
---@field queue_limit integer
---@field buffered_amount_limit integer
---@field event_depth integer
---@field sent integer
---@field received integer
---@field dropped_outbound integer
---@field dropped_inbound integer
---@field malformed integer
---@field unsupported_version integer
---@field overflow integer
---@field backpressure integer
---@field last_error string?
---@field peers TransportPeerDiagnostics[] -- Ordered by slot, never by arrival.

---@class TransportAddressedMessage
---@field peer_id string -- Link identity chosen by the star, never read from the payload.
---@field channel TransportChannel
---@field message TransportMessage

---@class TransportPeerMessage: TransportAddressedMessage
---@field arrival_seq integer -- Per-peer transport receive counter; not a simulation tick.

---@class TransportPeerEvent
---@field kind "peer_state"|"peer_error"|"star_state"|"star_error"
---@field peer_id string?
---@field channel TransportChannel?
---@field state TransportPeerState?
---@field code TransportErrorCode?
---@field message string?

---@class StarTransportAdapter
---@field initialize fun(self: StarTransportAdapter): boolean?, string?, TransportErrorCode?
---@field shutdown fun(self: StarTransportAdapter): boolean?, string?, TransportErrorCode?
---@field role fun(self: StarTransportAdapter): TransportRole
---@field capacity fun(self: StarTransportAdapter): integer
---@field open_peer fun(self: StarTransportAdapter, peer_id: string): integer?, string?, TransportErrorCode?
---@field close_peer fun(self: StarTransportAdapter, peer_id: string, reason: string?): boolean?, string?, TransportErrorCode?
---@field peer_ids fun(self: StarTransportAdapter): string[]
---@field peer_state fun(self: StarTransportAdapter, peer_id: string): TransportPeerState?
-- Manual signaling. Offer/answer creation is asynchronous in the browser, so
-- these only start the work; `take_signal` yields the blob on a later frame.
---@field request_offer fun(self: StarTransportAdapter, peer_id: string): boolean?, string?, TransportErrorCode?
---@field accept_offer fun(self: StarTransportAdapter, signal: string): boolean?, string?, TransportErrorCode?
---@field accept_answer fun(self: StarTransportAdapter, peer_id: string, signal: string): boolean?, string?, TransportErrorCode?
---@field take_signal fun(self: StarTransportAdapter, peer_id: string): string?, string?, TransportErrorCode?
---@field send fun(self: StarTransportAdapter, peer_id: string, channel: TransportChannel, message: TransportMessage): boolean?, string?, TransportErrorCode?
---@field broadcast fun(self: StarTransportAdapter, channel: TransportChannel, message: TransportMessage): integer?, string?, TransportErrorCode?
---@field poll fun(self: StarTransportAdapter): TransportPeerMessage?, string?, TransportErrorCode?
---@field poll_batch fun(self: StarTransportAdapter, limit: integer?): TransportPeerMessage[]
---@field poll_event fun(self: StarTransportAdapter): TransportPeerEvent?
---@field state fun(self: StarTransportAdapter): TransportState
---@field diagnostics fun(self: StarTransportAdapter): TransportStarDiagnostics

---@class TransportAdapter
---@field initialize fun(self: TransportAdapter): boolean?, string?, TransportErrorCode?
---@field shutdown fun(self: TransportAdapter): boolean?, string?, TransportErrorCode?
---@field enqueue fun(self: TransportAdapter, message: TransportMessage): boolean?, string?, TransportErrorCode?
---@field send fun(self: TransportAdapter, message: TransportMessage): boolean?, string?, TransportErrorCode?
---@field poll fun(self: TransportAdapter): TransportMessage?, string?, TransportErrorCode?
---@field poll_event fun(self: TransportAdapter): TransportEvent?
---@field state fun(self: TransportAdapter): TransportState
---@field diagnostics fun(self: TransportAdapter): TransportDiagnostics

---@class TransportContractModule
local contract = {}

contract.VERSION = 1
contract.DEFAULT_QUEUE_LIMIT = 64
contract.MAX_QUEUE_LIMIT = 256
contract.MAX_PAYLOAD_BYTES = 1024

-- Host-star bounds. One host endpoint addresses at most seven guest links.
contract.MAX_GUESTS = 7
contract.HOST_PEER_ID = "host"
contract.MAX_PEER_ID_BYTES = 128
contract.PEER_ID_PATTERN = "^[%w][%w_%-]*$"
contract.DEFAULT_BUFFERED_AMOUNT_LIMIT = 65536
contract.MAX_BUFFERED_AMOUNT_LIMIT = 1048576
contract.DEFAULT_POLL_BATCH = 32

-- Channel configuration is part of the contract: the control channel is
-- reliable and ordered, the input channel is unordered and never retransmits.
-- `rank` fixes the deterministic drain order at the Lua boundary.
contract.CHANNEL_CONFIG = {
    control = { label = "control", ordered = true, reliable = true, rank = 1 },
    input = { label = "input", ordered = false, max_retransmits = 0, rank = 2 },
}
contract.CHANNEL_ORDER = { "control", "input" }

-- Channel/message-type pairing. Control carries lifecycle and session control
-- payloads; input carries tick-numbered input packets and nothing else.
contract.CHANNEL_MESSAGE_TYPES = {
    control = { event = true, state = true },
    input = { input = true },
}

local MESSAGE_TYPES = {
    input = true,
    event = true,
    state = true,
}

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number" and value == math.floor(value)
end

---@param code TransportErrorCode
---@param message string
---@return nil, string, TransportErrorCode
local function failure(code, message)
    return nil, message, code
end

---@param message any
---@return boolean?, string?, TransportErrorCode?
function contract.validate(message)
    if type(message) ~= "table" then
        return failure("malformed", "transport message must be a table")
    end
    if not is_integer(message.version) then
        return failure("malformed", "transport message version must be an integer")
    end
    if message.version ~= contract.VERSION then
        return failure("unsupported_version", "unsupported transport message version")
    end
    if type(message.type) ~= "string" or not MESSAGE_TYPES[message.type] then
        return failure("malformed", "transport message type is invalid")
    end
    if not is_integer(message.seq) or message.seq < 0 then
        return failure("malformed", "transport message seq must be a non-negative integer")
    end
    if message.type == "input" then
        if not is_integer(message.tick) or message.tick < 0 then
            return failure("malformed", "input message tick must be a non-negative integer")
        end
    elseif message.tick ~= nil and (not is_integer(message.tick) or message.tick < 0) then
        return failure("malformed", "transport message tick must be a non-negative integer")
    end
    if type(message.payload) ~= "string" then
        return failure("malformed", "transport message payload must be a string")
    end
    if #message.payload > contract.MAX_PAYLOAD_BYTES then
        return failure("payload_too_large", "transport message payload exceeds the byte limit")
    end
    return true
end

---@param options TransportMessageOptions
---@return TransportMessage?, string?, TransportErrorCode?
function contract.new(options)
    local message = {
        version = options.version or contract.VERSION,
        type = options.type,
        seq = options.seq,
        tick = options.tick,
        payload = options.payload,
    }
    local ok, err, code = contract.validate(message)
    if not ok then
        return nil, err, code
    end
    return message
end

---@param message TransportMessage
---@return TransportMessage
function contract.copy(message)
    local ok, err = contract.validate(message)
    assert(ok, err)
    return {
        version = message.version,
        type = message.type,
        seq = message.seq,
        tick = message.tick,
        payload = message.payload,
    }
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

---@param value string
---@return string?, string?
local function unescape(value)
    local result = {}
    local index = 1
    while index <= #value do
        local character = value:sub(index, index)
        if character == "%" then
            local hex = value:sub(index + 1, index + 2)
            if #hex ~= 2 or not hex:match("^%x%x$") then
                return nil, "transport wire field has an invalid escape"
            end
            result[#result + 1] = string.char(tonumber(hex, 16))
            index = index + 3
        else
            result[#result + 1] = character
            index = index + 1
        end
    end
    return table.concat(result)
end

---@param value string
---@return integer
local function parse_integer(value)
    local number = assert(tonumber(value))
    ---@cast number integer
    return number
end

---@param message TransportMessage
---@return string?, string?, TransportErrorCode?
function contract.encode(message)
    local ok, err, code = contract.validate(message)
    if not ok then
        return nil, err, code
    end
    return table.concat({
        tostring(message.version),
        escape(message.type),
        tostring(message.seq),
        message.tick and tostring(message.tick) or "",
        escape(message.payload),
    }, "|")
end

---@param wire string
---@return TransportMessage?, string?, TransportErrorCode?
function contract.decode(wire)
    if type(wire) ~= "string" then
        return failure("malformed", "transport wire message must be a string")
    end
    local raw_version, raw_type, raw_seq, raw_tick, raw_payload =
        wire:match("^([^|]*)|([^|]*)|([^|]*)|([^|]*)|([^|]*)$")
    if not raw_version then
        return failure("malformed", "transport wire message has invalid fields")
    end
    if not raw_version:match("^%d+$") or not raw_seq:match("^%d+$") then
        return failure("malformed", "transport wire message numbers are invalid")
    end
    if raw_tick ~= "" and not raw_tick:match("^%d+$") then
        return failure("malformed", "transport wire message tick is invalid")
    end
    local message_type, type_err = unescape(raw_type)
    local payload, payload_err = unescape(raw_payload)
    if not message_type or not payload then
        return failure("malformed", type_err or payload_err or "transport wire escape is invalid")
    end
    -- `cond and nil or fallback` always takes the fallback, so an absent tick
    -- has to be handled with a statement rather than an expression.
    local tick = nil
    if raw_tick ~= "" then
        tick = parse_integer(raw_tick)
    end
    return contract.new({
        version = parse_integer(raw_version),
        type = message_type,
        seq = parse_integer(raw_seq),
        tick = tick,
        payload = payload,
    })
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

---@param peer_id any
---@return boolean?, string?, TransportErrorCode?
function contract.validate_peer_id(peer_id)
    if type(peer_id) ~= "string" or peer_id == "" then
        return failure("malformed", "transport peer id must be a non-empty string")
    end
    if #peer_id > contract.MAX_PEER_ID_BYTES then
        return failure("malformed", "transport peer id exceeds the byte limit")
    end
    if not peer_id:match(contract.PEER_ID_PATTERN) then
        return failure("malformed", "transport peer id contains unsupported characters")
    end
    return true
end

---@param channel any
---@return boolean?, string?, TransportErrorCode?
function contract.validate_channel(channel)
    if type(channel) ~= "string" or not contract.CHANNEL_CONFIG[channel] then
        return failure("channel_mismatch", "transport channel must be control or input")
    end
    return true
end

-- Enforces the channel/message-type pairing before anything reaches a data
-- channel, so an input packet can never take the reliable ordered path and a
-- lifecycle message can never take the lossy path.
---@param channel any
---@param message any
---@return boolean?, string?, TransportErrorCode?
function contract.validate_channel_message(channel, message)
    local ok, err, code = contract.validate_channel(channel)
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate(message)
    if not ok then
        return nil, err, code
    end
    if not contract.CHANNEL_MESSAGE_TYPES[channel][message.type] then
        return failure(
            "channel_mismatch",
            ("transport %s channel does not carry %s messages"):format(channel, message.type)
        )
    end
    return true
end

-- Peer-addressed wire framing: `peer_id|channel|<envelope wire>`. Peer ids and
-- channel names never contain a pipe, so the envelope tail is recovered by
-- splitting only the first two separators.
---@param peer_id string
---@param channel TransportChannel
---@param message TransportMessage
---@return string?, string?, TransportErrorCode?
function contract.encode_addressed(peer_id, channel, message)
    local ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate_channel_message(channel, message)
    if not ok then
        return nil, err, code
    end
    local wire, encode_err, encode_code = contract.encode(message)
    if not wire then
        return nil, encode_err, encode_code
    end
    return peer_id .. "|" .. channel .. "|" .. wire
end

---@param line any
---@return TransportAddressedMessage?, string?, TransportErrorCode?
function contract.decode_addressed(line)
    if type(line) ~= "string" then
        return failure("malformed", "transport addressed wire must be a string")
    end
    local peer_id, channel, wire = line:match("^([^|]*)|([^|]*)|(.*)$")
    if not peer_id then
        return failure("malformed", "transport addressed wire has invalid fields")
    end
    local ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate_channel(channel)
    if not ok then
        return nil, err, code
    end
    local message, decode_err, decode_code = contract.decode(wire)
    if not message then
        return nil, decode_err, decode_code
    end
    ---@cast channel TransportChannel
    if not contract.CHANNEL_MESSAGE_TYPES[channel][message.type] then
        return failure(
            "channel_mismatch",
            ("transport %s channel does not carry %s messages"):format(channel, message.type)
        )
    end
    return { peer_id = peer_id, channel = channel, message = message }
end

---@param diagnostics TransportChannelDiagnostics
---@return string[]
local function channel_fields(diagnostics)
    return {
        diagnostics.state,
        tostring(diagnostics.outbound_depth),
        tostring(diagnostics.inbound_depth),
        tostring(diagnostics.buffered_amount),
        tostring(diagnostics.sent),
        tostring(diagnostics.received),
        tostring(diagnostics.dropped_outbound),
        tostring(diagnostics.dropped_inbound),
    }
end

-- Line-oriented star diagnostics: one `star` record followed by one `peer`
-- record per slot. The JavaScript bridge emits the same shape.
---@param diagnostics TransportStarDiagnostics
---@return string
function contract.encode_star_diagnostics(diagnostics)
    local lines = {
        table.concat({
            "star",
            diagnostics.role,
            diagnostics.state,
            tostring(diagnostics.capacity),
            tostring(diagnostics.peer_count),
            tostring(diagnostics.queue_limit),
            tostring(diagnostics.buffered_amount_limit),
            tostring(diagnostics.event_depth),
            tostring(diagnostics.sent),
            tostring(diagnostics.received),
            tostring(diagnostics.dropped_outbound),
            tostring(diagnostics.dropped_inbound),
            tostring(diagnostics.malformed),
            tostring(diagnostics.unsupported_version),
            tostring(diagnostics.overflow),
            tostring(diagnostics.backpressure),
            escape(diagnostics.last_error or ""),
        }, "|"),
    }
    for _, peer in ipairs(diagnostics.peers) do
        local fields = {
            "peer",
            peer.peer_id,
            tostring(peer.slot),
            peer.state,
            escape(peer.ice_state),
        }
        for _, value in ipairs(channel_fields(peer.control)) do
            fields[#fields + 1] = value
        end
        for _, value in ipairs(channel_fields(peer.input)) do
            fields[#fields + 1] = value
        end
        fields[#fields + 1] = tostring(peer.sequence_gaps)
        fields[#fields + 1] = tostring(peer.backpressure)
        fields[#fields + 1] = tostring(peer.malformed)
        fields[#fields + 1] = escape(peer.last_error or "")
        lines[#lines + 1] = table.concat(fields, "|")
    end
    return table.concat(lines, "\n")
end

local STAR_FIELDS = 17
local PEER_FIELDS = 25

---@param fields string[]
---@param offset integer
---@return TransportChannelDiagnostics?
local function parse_channel(fields, offset)
    local numbers = {}
    for index = 1, 7 do
        local value = tonumber(fields[offset + index])
        if not value or value ~= math.floor(value) then
            return nil
        end
        numbers[index] = value
    end
    ---@type TransportPeerState
    local state = fields[offset]
    return {
        state = state,
        outbound_depth = numbers[1],
        inbound_depth = numbers[2],
        buffered_amount = numbers[3],
        sent = numbers[4],
        received = numbers[5],
        dropped_outbound = numbers[6],
        dropped_inbound = numbers[7],
    }
end

---@param text any
---@return TransportStarDiagnostics?, string?, TransportErrorCode?
function contract.decode_star_diagnostics(text)
    if type(text) ~= "string" or text == "" then
        return failure("bridge_error", "star diagnostics must be a non-empty string")
    end
    local lines = split(text, "\n")
    local star = split(lines[1], "|")
    if star[1] ~= "star" or #star ~= STAR_FIELDS then
        return failure("bridge_error", "star diagnostics header is invalid")
    end
    local numbers = {}
    for index = 4, 16 do
        local value = tonumber(star[index])
        if not value or value ~= math.floor(value) then
            return failure("bridge_error", "star diagnostics contain invalid numbers")
        end
        numbers[index] = value
    end
    local last_error = unescape(star[17])
    ---@type TransportRole
    local role = star[2]
    ---@type TransportState
    local state = star[3]
    ---@type TransportStarDiagnostics
    local diagnostics = {
        role = role,
        state = state,
        capacity = numbers[4],
        peer_count = numbers[5],
        queue_limit = numbers[6],
        buffered_amount_limit = numbers[7],
        event_depth = numbers[8],
        sent = numbers[9],
        received = numbers[10],
        dropped_outbound = numbers[11],
        dropped_inbound = numbers[12],
        malformed = numbers[13],
        unsupported_version = numbers[14],
        overflow = numbers[15],
        backpressure = numbers[16],
        last_error = (last_error ~= "" and last_error) or nil,
        peers = {},
    }
    for index = 2, #lines do
        local peer = split(lines[index], "|")
        if peer[1] ~= "peer" or #peer ~= PEER_FIELDS then
            return failure("bridge_error", "star diagnostics peer record is invalid")
        end
        local slot = tonumber(peer[3])
        local control = parse_channel(peer, 6)
        local input = parse_channel(peer, 14)
        local sequence_gaps = tonumber(peer[22])
        local backpressure = tonumber(peer[23])
        local malformed = tonumber(peer[24])
        if not slot or not control or not input then
            return failure("bridge_error", "star diagnostics peer channels are invalid")
        end
        if not sequence_gaps or not backpressure or not malformed then
            return failure("bridge_error", "star diagnostics peer counters are invalid")
        end
        ---@cast slot integer
        ---@cast sequence_gaps integer
        ---@cast backpressure integer
        ---@cast malformed integer
        local peer_error = unescape(peer[25])
        ---@type TransportPeerState
        local peer_state = peer[4]
        diagnostics.peers[#diagnostics.peers + 1] = {
            peer_id = peer[2],
            slot = slot,
            state = peer_state,
            ice_state = unescape(peer[5]) or "",
            control = control,
            input = input,
            sequence_gaps = sequence_gaps,
            backpressure = backpressure,
            malformed = malformed,
            last_error = (peer_error ~= "" and peer_error) or nil,
        }
    end
    return diagnostics
end

return contract
