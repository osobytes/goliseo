local contract = require("game.transport.contract")

-- Browser host-star adapter. Every WebRTC object stays behind
-- `window.GalacticCupStarTransport`; this module only exchanges bounded ASCII
-- strings through the pinned runtime's `love.js.eval` hook, so no JavaScript
-- value ever escapes `game/`. Nothing here is a simulation authority.

---@class BrowserStarTransportOptions
---@field role TransportRole?
---@field queue_limit integer?
---@field max_guests integer?
---@field buffered_amount_limit integer?
---@field eval fun(command: string): string?, string? -- Test seam; defaults to love.js.eval.

---@class BrowserStarTransport: StarTransportAdapter
---@field _role TransportRole
---@field _state TransportState
---@field _queue_limit integer
---@field _max_guests integer
---@field _buffered_amount_limit integer
---@field _eval fun(command: string): string?, string?
---@field _arrivals table<string, integer>
---@field _peer_states table<string, TransportPeerState>
---@field _order string[]
---@field _events TransportPeerEvent[]
---@field _event_limit integer
---@field _local_error boolean
---@field _last_error string?
local BrowserStarTransport = {}
BrowserStarTransport.__index = BrowserStarTransport

local BRIDGE = "window.GalacticCupStarTransport."

---@param code TransportErrorCode
---@param message string
---@return nil, string, TransportErrorCode
local function failure(code, message)
    return nil, message, code
end

---@param command string
---@return string?, string?
local function default_eval(command)
    local love_api = rawget(_G, "love")
    local js = love_api and rawget(love_api, "js")
    local eval = js and rawget(js, "eval")
    if type(eval) ~= "function" then
        return nil, "love.js.eval is not available"
    end
    local ok, result = pcall(eval, command)
    if not ok then
        return nil, tostring(result)
    end
    return result == nil and "" or tostring(result)
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
---@return string?
local function unescape(value)
    local result = {}
    local index = 1
    while index <= #value do
        local character = value:sub(index, index)
        if character == "%" then
            local hex = value:sub(index + 1, index + 2)
            if #hex ~= 2 or not hex:match("^%x%x$") then
                return nil
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
---@return string
local function quote(value)
    return "'" .. escape(value) .. "'"
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

---@param value integer
---@param name string
---@param maximum integer
---@return integer
local function bounded(value, name, maximum)
    assert(
        value == math.floor(value) and value > 0 and value <= maximum,
        "browser star transport " .. name .. " is outside the supported range"
    )
    return value
end

---@param options BrowserStarTransportOptions?
---@return BrowserStarTransport
function BrowserStarTransport.new(options)
    options = options or {}
    local role = options.role or "host"
    assert(role == "host" or role == "guest", "browser star transport role must be host or guest")
    return setmetatable({
        _role = role,
        _state = "new",
        _queue_limit = bounded(
            options.queue_limit or contract.DEFAULT_QUEUE_LIMIT,
            "queue_limit",
            contract.MAX_QUEUE_LIMIT
        ),
        _max_guests = role == "host" and bounded(
            options.max_guests or contract.MAX_GUESTS,
            "max_guests",
            contract.MAX_GUESTS
        ) or 1,
        _buffered_amount_limit = bounded(
            options.buffered_amount_limit or contract.DEFAULT_BUFFERED_AMOUNT_LIMIT,
            "buffered_amount_limit",
            contract.MAX_BUFFERED_AMOUNT_LIMIT
        ),
        _eval = options.eval or default_eval,
        _arrivals = {},
        _peer_states = {},
        _order = {},
        _events = {},
        _event_limit = math.max(2, options.queue_limit or contract.DEFAULT_QUEUE_LIMIT),
        _local_error = false,
        _last_error = nil,
    }, BrowserStarTransport)
end

-- A fault this adapter rejects locally (role misuse, lifecycle misuse, a
-- message the bridge would never see) has to be as observable as one the
-- bridge reports, or the two adapters disagree about what happened.
---@param code TransportErrorCode
---@param message string
---@param peer_id string?
---@param channel TransportChannel?
---@return nil, string, TransportErrorCode
function BrowserStarTransport:_record_error(code, message, peer_id, channel)
    self._last_error = message
    self._local_error = true
    if #self._events >= self._event_limit then
        table.remove(self._events, 1)
    end
    if peer_id then
        self._events[#self._events + 1] = {
            kind = "peer_error",
            peer_id = peer_id,
            channel = channel,
            code = code,
            message = message,
        }
    else
        self._events[#self._events + 1] = { kind = "star_error", code = code, message = message }
    end
    return nil, message, code
end

---@param name string
---@param ... string
---@return string?, string?
function BrowserStarTransport:_call(name, ...)
    local command = BRIDGE .. name .. "(" .. table.concat({ ... }, ",") .. ")"
    local result, err = self._eval(command)
    if not result then
        self._last_error = err or "browser star bridge returned no result"
        return nil, self._last_error
    end
    return result
end

-- Bridge results are either `ok`, `<tag>|<value>`, or `error|<code>|<detail>`.
---@param result string
---@return string[]?, string?, TransportErrorCode?
function BrowserStarTransport:_parse(result)
    local fields = split(result, "|")
    if fields[1] ~= "error" then
        return fields
    end
    local code = fields[2]
    local detail = unescape(fields[3] or "") or code
    if type(code) ~= "string" or code == "" then
        self._last_error = "browser star bridge returned an invalid error response"
        return failure("bridge_error", self._last_error)
    end
    ---@cast code TransportErrorCode
    self._last_error = detail
    return nil, detail, code
end

---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:initialize()
    if self._state == "connected" then
        return true
    end
    local result, err = self:_call(
        "initialize",
        quote(self._role),
        tostring(self._queue_limit),
        tostring(self._max_guests),
        tostring(self._buffered_amount_limit)
    )
    if not result then
        return failure("bridge_error", err or "browser star bridge is unavailable")
    end
    local fields, parse_err, code = self:_parse(result)
    if not fields then
        return nil, parse_err, code
    end
    if fields[1] ~= "star" or fields[2] ~= "connected" then
        return failure("not_connected", "browser star transport did not connect")
    end
    self._state = "connected"
    if self._role == "guest" then
        self._order = { contract.HOST_PEER_ID }
        self._peer_states[contract.HOST_PEER_ID] = "connecting"
    end
    return true
end

---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:shutdown()
    if self._state == "closed" then
        return true
    end
    local result, err = self:_call("shutdown")
    if not result then
        return failure("bridge_error", err or "browser star bridge is unavailable")
    end
    local fields, parse_err, code = self:_parse(result)
    if not fields then
        return nil, parse_err, code
    end
    if fields[1] ~= "star" or fields[2] ~= "closed" then
        return failure("bridge_error", "browser star transport did not close")
    end
    self._state = "closed"
    self._order = {}
    self._peer_states = {}
    self._arrivals = {}
    return true
end

---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:_require_connected()
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

---@param peer_id string
---@return integer?, string?, TransportErrorCode?
function BrowserStarTransport:open_peer(peer_id)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "host" then
        return self:_record_error("role_forbidden", "only a host opens guest links")
    end
    ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        return nil, err, code
    end
    local result, call_err = self:_call("open_peer", quote(peer_id))
    if not result then
        return failure("bridge_error", call_err or "browser star bridge is unavailable")
    end
    local fields, parse_err, parse_code = self:_parse(result)
    if not fields then
        return nil, parse_err, parse_code
    end
    local slot = fields[1] == "slot" and tonumber(fields[2]) or nil
    if not slot then
        return failure("bridge_error", "browser star bridge returned an invalid slot")
    end
    ---@cast slot integer
    self._order[#self._order + 1] = peer_id
    self._peer_states[peer_id] = "connecting"
    return slot
end

---@param peer_id string
---@param reason string?
---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:close_peer(peer_id, reason)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        return self:_record_error(code or "malformed", err or "invalid peer id")
    end
    local result, call_err = self:_call("close_peer", quote(peer_id), quote(reason or "closed"))
    if not result then
        return failure("bridge_error", call_err or "browser star bridge is unavailable")
    end
    local fields, parse_err, parse_code = self:_parse(result)
    if not fields then
        return nil, parse_err, parse_code
    end
    self._peer_states[peer_id] = "closed"
    return true
end

-- Manual signaling. Offer and answer creation are asynchronous in the browser,
-- so these calls only start a job; `take_signal` returns the SDP blob on a
-- later frame. Nothing here waits on the network inside a LÖVE update.
---@param peer_id string
---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:request_offer(peer_id)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "host" then
        return self:_record_error("role_forbidden", "only a host creates offers")
    end
    ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        return self:_record_error(code or "malformed", err or "invalid peer id")
    end
    return self:_command("request_offer", quote(peer_id))
end

---@param signal string
---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:accept_offer(signal)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "guest" then
        return self:_record_error("role_forbidden", "only a guest accepts an offer")
    end
    return self:_command("accept_offer", quote(signal))
end

---@param peer_id string
---@param signal string
---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:accept_answer(peer_id, signal)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "host" then
        return self:_record_error("role_forbidden", "only a host accepts an answer")
    end
    ok, err, code = contract.validate_peer_id(peer_id)
    if not ok then
        return self:_record_error(code or "malformed", err or "invalid peer id")
    end
    return self:_command("accept_answer", quote(peer_id), quote(signal))
end

---@param name string
---@param ... string
---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:_command(name, ...)
    local result, call_err = self:_call(name, ...)
    if not result then
        return failure("bridge_error", call_err or "browser star bridge is unavailable")
    end
    local fields, parse_err, parse_code = self:_parse(result)
    if not fields then
        return nil, parse_err, parse_code
    end
    return true
end

-- Returns the locally generated SDP blob for a peer once the browser has
-- finished gathering, or nil while the job is still pending.
---@param peer_id string
---@return string?, string?, TransportErrorCode?
function BrowserStarTransport:take_signal(peer_id)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    local result, call_err = self:_call("take_signal", quote(peer_id))
    if not result then
        return failure("bridge_error", call_err or "browser star bridge is unavailable")
    end
    if result == "" then
        return nil
    end
    local fields, parse_err, parse_code = self:_parse(result)
    if not fields then
        return nil, parse_err, parse_code
    end
    if fields[1] ~= "signal" then
        return failure("bridge_error", "browser star bridge returned an invalid signal")
    end
    local signal = unescape(fields[2] or "")
    if not signal then
        return failure("bridge_error", "browser star bridge signal escape is invalid")
    end
    return signal
end

---@param peer_id string
---@param channel TransportChannel
---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function BrowserStarTransport:send(peer_id, channel, message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role == "guest" and peer_id ~= contract.HOST_PEER_ID then
        return self:_record_error("role_forbidden", "a guest may only address the host")
    end
    -- Validation order is part of the contract: message shape first, peer
    -- resolution second. Only the bridge knows which links exist, so an
    -- adapter must never claim `unknown_peer` from its own cached peer table;
    -- shape is the one fault every layer can judge identically.
    local line, encode_err, encode_code = contract.encode_addressed(peer_id, channel, message)
    if not line then
        return self:_record_error(
            encode_code or "malformed",
            encode_err or "browser star transport rejected a message",
            peer_id,
            contract.CHANNEL_CONFIG[channel] and channel or nil
        )
    end
    return self:_command("send", "'" .. line .. "'")
end

---@param channel TransportChannel
---@param message TransportMessage
---@return integer?, string?, TransportErrorCode?
function BrowserStarTransport:broadcast(channel, message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    if self._role ~= "host" then
        return self:_record_error("role_forbidden", "only a host fans out canonical batches")
    end
    ok, err, code = contract.validate_channel_message(channel, message)
    if not ok then
        return self:_record_error(
            code or "malformed",
            err or "browser star transport rejected a message"
        )
    end
    local wire = assert(contract.encode(message))
    local result, call_err = self:_call("broadcast", "'" .. channel .. "|" .. wire .. "'")
    if not result then
        return failure("bridge_error", call_err or "browser star bridge is unavailable")
    end
    local fields, parse_err, parse_code = self:_parse(result)
    if not fields then
        return nil, parse_err, parse_code
    end
    local delivered = fields[1] == "delivered" and tonumber(fields[2]) or nil
    if not delivered then
        return failure("bridge_error", "browser star bridge returned an invalid fan-out count")
    end
    ---@cast delivered integer
    return delivered
end

---@return TransportPeerMessage?, string?, TransportErrorCode?
function BrowserStarTransport:poll()
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    local line, call_err = self:_call("poll")
    if not line then
        return failure("bridge_error", call_err or "browser star bridge is unavailable")
    end
    if line == "" then
        return nil
    end
    local addressed, decode_err, decode_code = contract.decode_addressed(line)
    if not addressed then
        self._last_error = decode_err
        return nil, decode_err, decode_code
    end
    local arrival = (self._arrivals[addressed.peer_id] or 0) + 1
    self._arrivals[addressed.peer_id] = arrival
    return {
        peer_id = addressed.peer_id,
        channel = addressed.channel,
        arrival_seq = arrival,
        message = addressed.message,
    }
end

-- The bridge drains its per-peer queues in slot order, control before input,
-- one message per queue per pass. This relays that fixed order; it is not a
-- claim that browser callback arrival order is simulation order.
---@param limit integer?
---@return TransportPeerMessage[]
function BrowserStarTransport:poll_batch(limit)
    local budget = limit or contract.DEFAULT_POLL_BATCH
    local batch = {}
    while #batch < budget do
        local entry = self:poll()
        if not entry then
            return batch
        end
        batch[#batch + 1] = entry
    end
    return batch
end

---@return TransportPeerEvent?
function BrowserStarTransport:poll_event()
    -- Locally rejected calls are queued here; drain them before the bridge's
    -- own events so the caller sees every fault this adapter produced.
    if #self._events > 0 then
        return table.remove(self._events, 1)
    end
    local result = self:_call("poll_event")
    if not result or result == "" then
        return nil
    end
    local fields = split(result, "|")
    if fields[1] == "star_state" and fields[2] then
        ---@type TransportState
        local state = fields[2]
        self._state = state
        return { kind = "star_state", state = state }
    end
    if fields[1] == "peer_state" and fields[2] and fields[3] then
        ---@type TransportPeerState
        local state = fields[3]
        self._peer_states[fields[2]] = state
        return { kind = "peer_state", peer_id = fields[2], state = state }
    end
    if fields[1] == "star_error" and fields[2] then
        ---@type TransportErrorCode
        local code = fields[2]
        local detail = unescape(fields[3] or "")
        self._last_error = detail
        return { kind = "star_error", code = code, message = detail }
    end
    if fields[1] == "peer_error" and fields[2] and fields[4] then
        ---@type TransportErrorCode
        local code = fields[4]
        ---@type TransportChannel?
        local channel = fields[3] ~= "" and fields[3] or nil
        local detail = unescape(fields[5] or "")
        self._last_error = detail
        return {
            kind = "peer_error",
            peer_id = fields[2],
            channel = channel,
            code = code,
            message = detail,
        }
    end
    self._last_error = "browser star bridge returned an invalid event response"
    return { kind = "star_error", code = "bridge_error", message = self._last_error }
end

---@return TransportRole
function BrowserStarTransport:role()
    return self._role
end

---@return integer
function BrowserStarTransport:capacity()
    return self._max_guests
end

---@return string[]
function BrowserStarTransport:peer_ids()
    local ids = {}
    for index, peer_id in ipairs(self._order) do
        ids[index] = peer_id
    end
    return ids
end

---@param peer_id string
---@return TransportPeerState?
function BrowserStarTransport:peer_state(peer_id)
    return self._peer_states[peer_id]
end

---@return TransportState
function BrowserStarTransport:state()
    return self._state
end

---@return TransportStarDiagnostics
function BrowserStarTransport:_fallback_diagnostics()
    return {
        role = self._role,
        state = self._state,
        capacity = self._max_guests,
        peer_count = #self._order,
        queue_limit = self._queue_limit,
        buffered_amount_limit = self._buffered_amount_limit,
        event_depth = 0,
        sent = 0,
        received = 0,
        dropped_outbound = 0,
        dropped_inbound = 0,
        malformed = 0,
        unsupported_version = 0,
        overflow = 0,
        backpressure = 0,
        last_error = self._last_error,
        peers = {},
    }
end

---@return TransportStarDiagnostics
function BrowserStarTransport:diagnostics()
    local result, err = self:_call("diagnostics")
    if not result then
        self._last_error = err or "browser star bridge is unavailable"
        return self:_fallback_diagnostics()
    end
    local diagnostics, decode_err = contract.decode_star_diagnostics(result)
    if not diagnostics then
        self._last_error = decode_err or "browser star diagnostics are invalid"
        return self:_fallback_diagnostics()
    end
    self._state = diagnostics.state
    self._order = {}
    for index, peer in ipairs(diagnostics.peers) do
        self._order[index] = peer.peer_id
        self._peer_states[peer.peer_id] = peer.state
    end
    -- A fault this adapter rejected locally never reached the bridge, so the
    -- bridge's `last_error` would silently hide it. The local one is newer.
    if self._local_error then
        diagnostics.last_error = self._last_error
        self._local_error = false
    end
    diagnostics.event_depth = diagnostics.event_depth + #self._events
    return diagnostics
end

return BrowserStarTransport
