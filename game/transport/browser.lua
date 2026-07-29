local contract = require("game.transport.contract")

---@class BrowserTransportOptions
---@field queue_limit integer?
---@field eval fun(command: string): string? -- Test seam; defaults to love.js.eval.

---@class BrowserTransport: TransportAdapter
---@field _state TransportState
---@field _queue_limit integer
---@field _eval fun(command: string): string?
---@field _event_depth integer
---@field _dropped_outbound integer
---@field _dropped_inbound integer
---@field _malformed integer
---@field _unsupported_version integer
---@field _overflow integer
---@field _sent integer
---@field _received integer
---@field _last_error string?
local BrowserTransport = {}
BrowserTransport.__index = BrowserTransport

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
---@return string[]
local function split_fields(value)
    local fields = {}
    local start = 1
    while true do
        local separator = value:find("|", start, true)
        if not separator then
            fields[#fields + 1] = value:sub(start)
            return fields
        end
        fields[#fields + 1] = value:sub(start, separator - 1)
        start = separator + 1
    end
end

---@param options BrowserTransportOptions?
---@return BrowserTransport
function BrowserTransport.new(options)
    options = options or {}
    local queue_limit = options.queue_limit or contract.DEFAULT_QUEUE_LIMIT
    assert(
        queue_limit == math.floor(queue_limit)
            and queue_limit > 0
            and queue_limit <= contract.MAX_QUEUE_LIMIT,
        "browser transport queue_limit is outside the supported range"
    )
    return setmetatable({
        _state = "new",
        _queue_limit = queue_limit,
        _eval = options.eval or default_eval,
        _event_depth = 0,
        _dropped_outbound = 0,
        _dropped_inbound = 0,
        _malformed = 0,
        _unsupported_version = 0,
        _overflow = 0,
        _sent = 0,
        _received = 0,
        _last_error = nil,
    }, BrowserTransport)
end

---@param code TransportErrorCode
---@param message string
function BrowserTransport:_record_error(code, message)
    self._last_error = message
    if code == "malformed" or code == "payload_too_large" then
        self._malformed = self._malformed + 1
    elseif code == "unsupported_version" then
        self._unsupported_version = self._unsupported_version + 1
    elseif code == "overflow" then
        self._overflow = self._overflow + 1
    end
end

---@param name string
---@param argument string?
---@return string?, string?
function BrowserTransport:_call(name, argument)
    local command = "window.GoliseoTransportBridge." .. name .. "("
    if argument then
        command = command .. argument
    end
    command = command .. ")"
    local result, err = self._eval(command)
    if not result then
        self:_record_error("bridge_error", err or "browser bridge returned no result")
        return nil, err or "browser bridge returned no result"
    end
    return result
end

---@param result string
---@return TransportState?, string?
local function parse_state(result)
    local fields = split_fields(result)
    if fields[1] ~= "state" or not fields[2] then
        return nil, "browser bridge returned an invalid state response"
    end
    if
        fields[2] ~= "new"
        and fields[2] ~= "connected"
        and fields[2] ~= "disconnected"
        and fields[2] ~= "closed"
        and fields[2] ~= "error"
    then
        return nil, "browser bridge returned an unknown state"
    end
    ---@cast fields[2] TransportState
    return fields[2]
end

---@param result string
---@return string?, TransportErrorCode?, string?
local function parse_result(result)
    if result == "ok" then
        return "ok"
    end
    local fields = split_fields(result)
    if fields[1] ~= "error" or not fields[2] then
        return nil, "bridge_error", "browser bridge returned an invalid operation response"
    end
    local code = fields[2]
    ---@cast code TransportErrorCode
    return nil, code, fields[3] or code
end

---@return boolean?, string?, TransportErrorCode?
function BrowserTransport:initialize()
    if self._state == "connected" then
        return true
    end
    local result, err = self:_call("initialize", tostring(self._queue_limit))
    if not result then
        return nil, err, "bridge_error"
    end
    local state, state_err = parse_state(result)
    if not state then
        local error_message = state_err or "browser bridge returned an invalid state"
        self:_record_error("bridge_error", error_message)
        return nil, error_message, "bridge_error"
    end
    self._state = state
    if state ~= "connected" then
        return nil, "browser transport did not connect", "not_connected"
    end
    return true
end

---@return boolean?, string?, TransportErrorCode?
function BrowserTransport:shutdown()
    if self._state == "closed" then
        return true
    end
    local result, err = self:_call("shutdown")
    if not result then
        return nil, err, "bridge_error"
    end
    local state, state_err = parse_state(result)
    if not state then
        local error_message = state_err or "browser bridge returned an invalid state"
        self:_record_error("bridge_error", error_message)
        return nil, error_message, "bridge_error"
    end
    self._state = state
    if state ~= "closed" then
        return nil, "browser transport did not close", "bridge_error"
    end
    return true
end

---@return boolean?, string?, TransportErrorCode?
function BrowserTransport:_require_connected()
    if self._state == "new" then
        return nil, "transport is not initialized", "not_initialized"
    end
    if self._state == "closed" then
        return nil, "transport is closed", "closed"
    end
    if self._state ~= "connected" then
        return nil, "transport is not connected", "not_connected"
    end
    return true
end

---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function BrowserTransport:enqueue(message)
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    ok, err, code = contract.validate(message)
    if not ok then
        local error_code = code or "malformed"
        local error_message = err or "browser transport rejected a message"
        self:_record_error(error_code, error_message)
        return nil, error_message, error_code
    end
    local wire = assert(contract.encode(message))
    local result, call_err = self:_call("enqueue", "'" .. wire .. "'")
    if not result then
        return nil, call_err, "bridge_error"
    end
    local status, result_code, result_err = parse_result(result)
    if not status then
        local error_code = result_code or "bridge_error"
        local error_message = result_err or "browser bridge rejected a message"
        self:_record_error(error_code, error_message)
        if error_code == "overflow" then
            self._dropped_outbound = self._dropped_outbound + 1
        end
        return nil, error_message, error_code
    end
    self._sent = self._sent + 1
    return true
end

---@param message TransportMessage
---@return boolean?, string?, TransportErrorCode?
function BrowserTransport:send(message)
    return self:enqueue(message)
end

---@return TransportMessage?, string?, TransportErrorCode?
function BrowserTransport:poll()
    local ok, err, code = self:_require_connected()
    if not ok then
        return nil, err, code
    end
    local wire, call_err = self:_call("poll")
    if not wire then
        return nil, call_err, "bridge_error"
    end
    if wire == "" then
        return nil
    end
    local message, decode_err, decode_code = contract.decode(wire)
    if not message then
        local error_code = decode_code or "malformed"
        local error_message = decode_err or "browser bridge returned a malformed message"
        self:_record_error(error_code, error_message)
        self._dropped_inbound = self._dropped_inbound + 1
        return nil, error_message, error_code
    end
    self._received = self._received + 1
    return message
end

---@return TransportEvent?
function BrowserTransport:poll_event()
    local result = self:_call("poll_event")
    if not result or result == "" then
        return nil
    end
    local fields = split_fields(result)
    if fields[1] == "state" and fields[2] then
        self._state = fields[2]
        return { kind = "state", state = fields[2] }
    end
    if fields[1] == "error" and fields[2] then
        self._event_depth = math.max(0, self._event_depth - 1)
        return { kind = "error", code = fields[2] }
    end
    self:_record_error("bridge_error", "browser bridge returned an invalid event response")
    return { kind = "error", code = "bridge_error", message = result }
end

---@param reason string?
---@return boolean?, string?, TransportErrorCode?
function BrowserTransport:disconnect(reason)
    local argument = reason
            and ("'" .. reason:gsub("[^%w%-%._~]", function(character)
                return ("%%%02X"):format(string.byte(character))
            end) .. "'")
        or nil
    local result, err = self:_call("disconnect", argument)
    if not result then
        return nil, err, "bridge_error"
    end
    local state, state_err = parse_state(result)
    if not state then
        local error_message = state_err or "browser bridge returned an invalid state"
        self:_record_error("bridge_error", error_message)
        return nil, error_message, "bridge_error"
    end
    self._state = state
    return true
end

---@return TransportState
function BrowserTransport:state()
    return self._state
end

---@return TransportDiagnostics
function BrowserTransport:diagnostics()
    local result, err = self:_call("diagnostics")
    if not result then
        return {
            state = self._state,
            queue_limit = self._queue_limit,
            outbound_depth = 0,
            inbound_depth = 0,
            event_depth = self._event_depth,
            dropped_outbound = self._dropped_outbound,
            dropped_inbound = self._dropped_inbound,
            malformed = self._malformed,
            unsupported_version = self._unsupported_version,
            overflow = self._overflow,
            sent = self._sent,
            received = self._received,
            last_error = err,
        }
    end
    local fields = split_fields(result)
    if #fields ~= 13 then
        self:_record_error("bridge_error", "browser bridge returned invalid diagnostics")
        return self:diagnostics_fallback()
    end
    local numbers = {}
    for index = 2, 12 do
        numbers[index] = tonumber(fields[index])
        if not numbers[index] then
            self:_record_error("bridge_error", "browser bridge diagnostics contain invalid numbers")
            return self:diagnostics_fallback()
        end
    end
    self._state = fields[1]
    return {
        state = fields[1],
        queue_limit = numbers[2],
        outbound_depth = numbers[3],
        inbound_depth = numbers[4],
        event_depth = numbers[5],
        dropped_outbound = numbers[6],
        dropped_inbound = numbers[7],
        malformed = numbers[8],
        unsupported_version = numbers[9],
        overflow = numbers[10],
        sent = numbers[11],
        received = numbers[12],
        last_error = fields[13] ~= "" and fields[13] or nil,
    }
end

---@return TransportDiagnostics
function BrowserTransport:diagnostics_fallback()
    return {
        state = self._state,
        queue_limit = self._queue_limit,
        outbound_depth = 0,
        inbound_depth = 0,
        event_depth = self._event_depth,
        dropped_outbound = self._dropped_outbound,
        dropped_inbound = self._dropped_inbound,
        malformed = self._malformed,
        unsupported_version = self._unsupported_version,
        overflow = self._overflow,
        sent = self._sent,
        received = self._received,
        last_error = self._last_error,
    }
end

return BrowserTransport
