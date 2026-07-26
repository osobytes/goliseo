local fnv1a64 = require("core.fnv1a64")
local protocol = require("game.online.protocol")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")

---@alias InputPacketKind "guest"|"host"
---@alias InputPacketErrorCode
---| "malformed"
---| "wire_too_large"
---| "unsupported_version"
---| "identity_mismatch"
---| "tick_mismatch"
---| "ownership_mismatch"
---| "duplicate"
---| "packet_conflict"
---| "authority_conflict"
---| "fairness_delay"
---| "backpressure_gap"
---@alias InputPacketDuplicateDisposition "idempotent"|"reject"

---@class InputAuthorityRow
---@field tick integer
---@field slot_index integer
---@field sample InputSample

---@class InputPacket
---@field version integer
---@field kind InputPacketKind
---@field input_version integer
---@field session_id string -- Context-bound; represented by manifest and packet ids on wire.
---@field manifest_id string
---@field sender_id string -- Context-bound to the transport peer or declared bot producer.
---@field sequence integer
---@field packet_id string
---@field transport_tick integer -- Sender transport clock, never an authority input tick.
---@field first_input_tick integer
---@field input_delay_ticks integer
---@field rows InputAuthorityRow[]

---@class InputPacketOptions
---@field session_id string
---@field manifest_id string
---@field sender_id string
---@field sequence integer
---@field transport_tick integer
---@field first_input_tick integer
---@field rows InputAuthorityRow[]

---@class InputPacketDecodeContext
---@field session_id string
---@field manifest_id string
---@field sender_id string

---@class InputPacketArrival
---@field packet InputPacket
---@field envelope TransportMessage
---@field arrival_tick integer -- Host transport clock when this envelope was polled.
---@field transport_peer_id string -- Authenticated/selected link identity, not payload authority.

---@class InputHostBatchOptions
---@field manifest SessionManifest
---@field assignments SessionSlotProducer[]
---@field host_peer_id string
---@field sequence integer
---@field transport_tick integer
---@field first_input_tick integer

---@class InputProtocolModule
local input_protocol = {}

input_protocol.VERSION = 1
input_protocol.HISTORY_ROWS = 6
input_protocol.RETAINED_ROWS = input_protocol.HISTORY_ROWS + 1
input_protocol.FAIRNESS_DELAY_TICKS = 3
input_protocol.MAX_GUEST_ROWS = input_protocol.RETAINED_ROWS
input_protocol.MAX_HOST_ROWS = input_frame.SLOT_COUNT * input_protocol.RETAINED_ROWS
input_protocol.RECORD_BYTES = 9
input_protocol.MAX_WIRE_BYTES = 1024
assert(
    transport_contract.MAX_PAYLOAD_BYTES >= input_protocol.MAX_WIRE_BYTES,
    "transport payload bound is smaller than the versioned input packet bound"
)

local HEADER = "GCIP"
local BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local PACKET_FIELDS = {
    version = true,
    kind = true,
    input_version = true,
    session_id = true,
    manifest_id = true,
    sender_id = true,
    sequence = true,
    packet_id = true,
    transport_tick = true,
    first_input_tick = true,
    input_delay_ticks = true,
    rows = true,
}
local ROW_FIELDS = {
    tick = true,
    slot_index = true,
    sample = true,
}

---@type table<string, integer>
local BASE64_VALUES = {}
for index = 1, #BASE64 do
    BASE64_VALUES[BASE64:sub(index, index)] = index - 1
end

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param value any
---@param allowed table<string, boolean>
---@return boolean
local function has_only_fields(value, allowed)
    if type(value) ~= "table" then
        return false
    end
    for key in pairs(value) do
        if type(key) ~= "string" or not allowed[key] then
            return false
        end
    end
    return true
end

---@param value any
---@param maximum integer?
---@return boolean
local function is_array(value, maximum)
    if type(value) ~= "table" then
        return false
    end
    local count = 0
    local greatest = 0
    for index in pairs(value) do
        if not is_integer(index) or index < 1 then
            return false
        end
        count = count + 1
        greatest = math.max(greatest, index)
    end
    return count == greatest and (maximum == nil or count <= maximum)
end

---@param code InputPacketErrorCode
---@param message string
---@return nil, string, InputPacketErrorCode
local function failure(code, message)
    return nil, message, code
end

---@param value any
---@param name string
---@param maximum integer
---@return boolean?, string?, InputPacketErrorCode?
local function validate_id(value, name, maximum)
    if
        type(value) ~= "string"
        or value == ""
        or #value > maximum
        or not value:match("^[A-Za-z0-9][A-Za-z0-9_%.%-]*$")
    then
        return failure("malformed", name .. " must be a bounded opaque ASCII id")
    end
    return true
end

---@param value any
---@param name string
---@param minimum integer
---@param maximum integer
---@return boolean?, string?, InputPacketErrorCode?
local function validate_integer(value, name, minimum, maximum)
    if not is_integer(value) or value < minimum or value > maximum then
        return failure("malformed", name .. " must be a bounded integer")
    end
    return true
end

---@param value any
---@param name string
---@return boolean?, string?, InputPacketErrorCode?
local function validate_hash(value, name)
    if type(value) ~= "string" or #value ~= 16 or not value:match("^[0-9a-f]+$") then
        return failure("malformed", name .. " must be a canonical 16-hex digest")
    end
    return true
end

---@param value string
---@return string
local function length_prefix(value)
    return tostring(#value) .. ":" .. value
end

---@param sample InputSample
---@return InputSample
local function copy_sample(sample)
    return assert(input_frame.new_sample(sample))
end

---@param row InputAuthorityRow
---@return InputAuthorityRow
local function copy_row(row)
    return {
        tick = row.tick,
        slot_index = row.slot_index,
        sample = copy_sample(row.sample),
    }
end

---@param rows InputAuthorityRow[]
---@return InputAuthorityRow[]
local function copy_rows(rows)
    local result = {}
    for index, row in ipairs(rows) do
        result[index] = copy_row(row)
    end
    return result
end

---@param left InputSample
---@param right InputSample
---@return boolean
local function samples_equal(left, right)
    return left.move_x == right.move_x
        and left.move_y == right.move_y
        and left.held == right.held
        and left.edges == right.edges
end

---@param left InputAuthorityRow
---@param right InputAuthorityRow
---@return boolean
local function row_less(left, right)
    return left.tick < right.tick
        or (left.tick == right.tick and left.slot_index < right.slot_index)
end

---@param value string
---@return string
local function base64_encode(value)
    local result = {}
    local index = 1
    while index <= #value do
        local first = assert(value:byte(index))
        local second = value:byte(index + 1)
        local third = value:byte(index + 2)
        local packed = first * 65536 + (second or 0) * 256 + (third or 0)
        local a = math.floor(packed / 262144) % 64
        local b = math.floor(packed / 4096) % 64
        local c = math.floor(packed / 64) % 64
        local d = packed % 64
        result[#result + 1] = BASE64:sub(a + 1, a + 1)
        result[#result + 1] = BASE64:sub(b + 1, b + 1)
        result[#result + 1] = second and BASE64:sub(c + 1, c + 1) or "="
        result[#result + 1] = third and BASE64:sub(d + 1, d + 1) or "="
        index = index + 3
    end
    return table.concat(result)
end

---@param value string
---@return string?
local function base64_decode(value)
    if #value == 0 or #value % 4 ~= 0 or not value:match("^[A-Za-z0-9+/=]+$") then
        return nil
    end
    local padding = value:match("(=*)$") or ""
    if #padding > 2 or value:sub(1, #value - #padding):find("=", 1, true) then
        return nil
    end
    local result = {}
    for index = 1, #value, 4 do
        local a = BASE64_VALUES[value:sub(index, index)]
        local b = BASE64_VALUES[value:sub(index + 1, index + 1)]
        local raw_c = value:sub(index + 2, index + 2)
        local raw_d = value:sub(index + 3, index + 3)
        local c = raw_c == "=" and 0 or BASE64_VALUES[raw_c]
        local d = raw_d == "=" and 0 or BASE64_VALUES[raw_d]
        if
            a == nil
            or b == nil
            or c == nil
            or d == nil
            or ((raw_c == "=" or raw_d == "=") and index ~= #value - 3)
            or (raw_c == "=" and raw_d ~= "=")
        then
            return nil
        end
        local packed = a * 262144 + b * 4096 + c * 64 + d
        result[#result + 1] = string.char(math.floor(packed / 65536) % 256)
        if raw_c ~= "=" then
            result[#result + 1] = string.char(math.floor(packed / 256) % 256)
        end
        if raw_d ~= "=" then
            result[#result + 1] = string.char(packed % 256)
        end
    end
    local decoded = table.concat(result)
    if base64_encode(decoded) ~= value then
        return nil
    end
    return decoded
end

---@param value integer
---@return string
local function encode_u32(value)
    return string.char(
        math.floor(value / 16777216) % 256,
        math.floor(value / 65536) % 256,
        math.floor(value / 256) % 256,
        value % 256
    )
end

---@param value string
---@param index integer
---@return integer
local function decode_u32(value, index)
    local a, b, c, d = value:byte(index, index + 3)
    return assert(a) * 16777216 + assert(b) * 65536 + assert(c) * 256 + assert(d)
end

---@param row InputAuthorityRow
---@return string
local function encode_row(row)
    local sample = assert(input_frame.encode_sample(row.sample))
    local bytes = {}
    for index = 1, input_frame.MAX_SAMPLE_WIRE_BYTES, 2 do
        bytes[#bytes + 1] = string.char(assert(tonumber(sample:sub(index, index + 1), 16)))
    end
    return encode_u32(row.tick) .. string.char(row.slot_index) .. table.concat(bytes)
end

---@param value string
---@param index integer
---@return InputAuthorityRow
local function decode_row(value, index)
    local slot_index = assert(value:byte(index + 4))
    local sample_wire = {}
    for offset = 5, 8 do
        sample_wire[#sample_wire + 1] = ("%02x"):format(assert(value:byte(index + offset)))
    end
    return {
        tick = decode_u32(value, index),
        slot_index = slot_index,
        sample = assert(input_frame.decode_sample(table.concat(sample_wire))),
    }
end

---@param kind InputPacketKind
---@param session_id string
---@param sender_id string
---@param sequence integer
---@return string?, string?, InputPacketErrorCode?
function input_protocol.packet_id(kind, session_id, sender_id, sequence)
    if kind ~= "guest" and kind ~= "host" then
        return failure("malformed", "input packet kind is invalid")
    end
    local ok, err, code = validate_id(session_id, "input session id", protocol.MAX_SESSION_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_id(sender_id, "input sender id", protocol.MAX_PEER_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_integer(sequence, "input sequence", 0, protocol.MAX_SEQUENCE)
    if not ok then
        return nil, err, code
    end
    local domain = kind == "guest" and "GCIG;1;" or "GCIH;1;"
    return fnv1a64.hash(
        domain
            .. length_prefix(session_id)
            .. length_prefix(sender_id)
            .. length_prefix(tostring(sequence))
    )
end

---@param packet any
---@return boolean?, string?, InputPacketErrorCode?
function input_protocol.validate(packet)
    if type(packet) ~= "table" or not has_only_fields(packet, PACKET_FIELDS) then
        return failure("malformed", "input packet must contain only canonical fields")
    end
    if not is_integer(packet.version) then
        return failure("malformed", "input packet version must be an integer")
    end
    if packet.version ~= input_protocol.VERSION then
        return failure("unsupported_version", "unsupported input packet version")
    end
    if packet.kind ~= "guest" and packet.kind ~= "host" then
        return failure("malformed", "input packet kind is invalid")
    end
    if not is_integer(packet.input_version) then
        return failure("malformed", "input sample version must be an integer")
    end
    if packet.input_version ~= input_frame.VERSION then
        return failure("unsupported_version", "unsupported input sample version")
    end
    local ok, err, code =
        validate_id(packet.session_id, "input session id", protocol.MAX_SESSION_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_id(packet.sender_id, "input sender id", protocol.MAX_PEER_ID_BYTES)
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_hash(packet.manifest_id, "input manifest id")
    if not ok then
        return nil, err, code
    end
    ok, err, code = validate_integer(packet.sequence, "input sequence", 0, protocol.MAX_SEQUENCE)
    if not ok then
        return nil, err, code
    end
    ok, err, code =
        validate_integer(packet.transport_tick, "input transport tick", 0, input_frame.MAX_TICK)
    if not ok then
        return nil, err, code
    end
    ok, err, code =
        validate_integer(packet.first_input_tick, "first input tick", 0, input_frame.MAX_TICK)
    if not ok then
        return nil, err, code
    end
    if packet.input_delay_ticks ~= input_protocol.FAIRNESS_DELAY_TICKS then
        return failure("unsupported_version", "unsupported input fairness delay")
    end
    local maximum = packet.kind == "guest" and input_protocol.MAX_GUEST_ROWS
        or input_protocol.MAX_HOST_ROWS
    if not is_array(packet.rows, maximum) or #packet.rows == 0 then
        return failure("malformed", "input packet rows must be a bounded canonical array")
    end
    for index, row in ipairs(packet.rows) do
        if type(row) ~= "table" or not has_only_fields(row, ROW_FIELDS) then
            return failure("malformed", "input packet row must contain only canonical fields")
        end
        if
            not is_integer(row.tick)
            or row.tick < packet.first_input_tick
            or row.tick > input_frame.MAX_TICK
        then
            return failure("malformed", "input packet row tick is outside the session range")
        end
        if
            not is_integer(row.slot_index)
            or row.slot_index < 1
            or row.slot_index > input_frame.SLOT_COUNT
        then
            return failure("malformed", "input packet row slot is invalid")
        end
        local sample_ok, sample_err = input_frame.validate_sample(row.sample)
        if not sample_ok then
            return failure("malformed", sample_err or "input packet sample is malformed")
        end
        if index > 1 and not row_less(packet.rows[index - 1], row) then
            return failure("malformed", "input packet rows are not in canonical tick-slot order")
        end
    end
    if packet.kind == "guest" then
        local slot_index = packet.rows[1].slot_index
        for index, row in ipairs(packet.rows) do
            if row.slot_index ~= slot_index then
                return failure("ownership_mismatch", "guest packet contains more than one slot")
            end
            if index > 1 and row.tick ~= packet.rows[index - 1].tick + 1 then
                return failure("malformed", "guest packet history is not contiguous")
            end
        end
        local current_tick = packet.rows[#packet.rows].tick
        local expected_first =
            math.max(packet.first_input_tick, current_tick - input_protocol.HISTORY_ROWS)
        if
            packet.rows[1].tick ~= expected_first
            or #packet.rows ~= current_tick - expected_first + 1
        then
            return failure(
                "malformed",
                "guest packet must contain current input plus exactly the retained prior rows"
            )
        end
    end
    local expected_id, id_err, id_code =
        input_protocol.packet_id(packet.kind, packet.session_id, packet.sender_id, packet.sequence)
    if expected_id == nil then
        return nil, id_err, id_code
    end
    if packet.packet_id ~= expected_id then
        return failure("identity_mismatch", "input packet id does not match its context")
    end
    return true
end

---@param packet InputPacket
---@return string?, string?, InputPacketErrorCode?
function input_protocol.encode(packet)
    local ok, err, code = input_protocol.validate(packet)
    if not ok then
        return nil, err, code
    end
    local raw_rows = {}
    for index, row in ipairs(packet.rows) do
        raw_rows[index] = encode_row(row)
    end
    local kind = packet.kind == "guest" and "G" or "H"
    local wire = table.concat({
        HEADER,
        tostring(input_protocol.VERSION),
        kind,
        tostring(input_frame.VERSION),
        packet.manifest_id,
        tostring(packet.sequence),
        packet.packet_id,
        tostring(packet.transport_tick),
        tostring(packet.first_input_tick),
        tostring(input_protocol.FAIRNESS_DELAY_TICKS),
        tostring(#packet.rows),
        base64_encode(table.concat(raw_rows)),
    }, ";")
    if #wire > input_protocol.MAX_WIRE_BYTES then
        return failure("wire_too_large", "input packet exceeds the transport payload bound")
    end
    return wire
end

---@param value string
---@return string[]
local function split_fields(value)
    local result = {}
    local start = 1
    while true do
        local separator = value:find(";", start, true)
        if not separator then
            result[#result + 1] = value:sub(start)
            return result
        end
        result[#result + 1] = value:sub(start, separator - 1)
        start = separator + 1
    end
end

---@param value string
---@return integer?
local function parse_unsigned(value)
    if value ~= "0" and not value:match("^[1-9]%d*$") then
        return nil
    end
    local result = tonumber(value)
    if not is_integer(result) then
        return nil
    end
    ---@cast result integer
    return result
end

---@param wire any
---@param context InputPacketDecodeContext
---@return InputPacket?, string?, InputPacketErrorCode?
function input_protocol.decode(wire, context)
    if type(wire) ~= "string" then
        return failure("malformed", "input packet wire must be a string")
    end
    if #wire > input_protocol.MAX_WIRE_BYTES then
        return failure("wire_too_large", "input packet exceeds the transport payload bound")
    end
    if type(context) ~= "table" then
        return failure("malformed", "input packet decode context is required")
    end
    local fields = split_fields(wire)
    if #fields ~= 12 or fields[1] ~= HEADER then
        return failure("malformed", "input packet wire has invalid fields")
    end
    local version = parse_unsigned(fields[2])
    local input_version = parse_unsigned(fields[4])
    if version == nil or input_version == nil then
        return failure("malformed", "input packet versions are not canonical integers")
    end
    if version ~= input_protocol.VERSION or input_version ~= input_frame.VERSION then
        return failure("unsupported_version", "unsupported input packet or sample version")
    end
    local kind = fields[3] == "G" and "guest" or fields[3] == "H" and "host" or nil
    if kind == nil then
        return failure("malformed", "input packet kind is invalid")
    end
    if fields[5] ~= context.manifest_id then
        return failure("identity_mismatch", "input manifest id does not match decode context")
    end
    local sequence = parse_unsigned(fields[6])
    local transport_tick = parse_unsigned(fields[8])
    local first_input_tick = parse_unsigned(fields[9])
    local input_delay_ticks = parse_unsigned(fields[10])
    local row_count = parse_unsigned(fields[11])
    if
        sequence == nil
        or transport_tick == nil
        or first_input_tick == nil
        or input_delay_ticks == nil
        or row_count == nil
    then
        return failure("malformed", "input packet integer is noncanonical")
    end
    local maximum = kind == "guest" and input_protocol.MAX_GUEST_ROWS
        or input_protocol.MAX_HOST_ROWS
    if row_count < 1 or row_count > maximum then
        return failure("malformed", "input packet row count is outside the kind bound")
    end
    local raw_rows = base64_decode(fields[12])
    if raw_rows == nil or #raw_rows ~= row_count * input_protocol.RECORD_BYTES then
        return failure("malformed", "input packet record block is invalid")
    end
    local rows = {}
    for index = 1, row_count do
        local offset = (index - 1) * input_protocol.RECORD_BYTES + 1
        local ok, row = pcall(decode_row, raw_rows, offset)
        if not ok then
            return failure("malformed", "input packet record is invalid")
        end
        rows[index] = row
    end
    local packet = {
        version = version,
        kind = kind,
        input_version = input_version,
        session_id = context.session_id,
        manifest_id = fields[5],
        sender_id = context.sender_id,
        sequence = sequence,
        packet_id = fields[7],
        transport_tick = transport_tick,
        first_input_tick = first_input_tick,
        input_delay_ticks = input_delay_ticks,
        rows = rows,
    }
    local ok, err, code = input_protocol.validate(packet)
    if not ok then
        return nil, err, code
    end
    if input_protocol.encode(packet) ~= wire then
        return failure("malformed", "input packet wire is not canonical")
    end
    return packet
end

---@param kind InputPacketKind
---@param options InputPacketOptions
---@return InputPacket?, string?, InputPacketErrorCode?
local function new_packet(kind, options)
    if type(options) ~= "table" then
        return failure("malformed", "input packet options are required")
    end
    local packet_id, id_err, id_code =
        input_protocol.packet_id(kind, options.session_id, options.sender_id, options.sequence)
    if packet_id == nil then
        return nil, id_err, id_code
    end
    local packet = {
        version = input_protocol.VERSION,
        kind = kind,
        input_version = input_frame.VERSION,
        session_id = options.session_id,
        manifest_id = options.manifest_id,
        sender_id = options.sender_id,
        sequence = options.sequence,
        packet_id = packet_id,
        transport_tick = options.transport_tick,
        first_input_tick = options.first_input_tick,
        input_delay_ticks = input_protocol.FAIRNESS_DELAY_TICKS,
        rows = options.rows,
    }
    local ok, validate_err, validate_code = input_protocol.validate(packet)
    if not ok then
        return nil, validate_err, validate_code
    end
    packet.rows = copy_rows(options.rows)
    local wire, err, code = input_protocol.encode(packet)
    if wire == nil then
        return nil, err, code
    end
    return input_protocol.decode(wire, {
        session_id = options.session_id,
        manifest_id = options.manifest_id,
        sender_id = options.sender_id,
    })
end

---@param options InputPacketOptions
---@return InputPacket?, string?, InputPacketErrorCode?
function input_protocol.new_guest(options)
    return new_packet("guest", options)
end

---@param options InputPacketOptions
---@return InputPacket?, string?, InputPacketErrorCode?
function input_protocol.new_host(options)
    return new_packet("host", options)
end

---@param packet InputPacket
---@return InputPacket?, string?, InputPacketErrorCode?
function input_protocol.copy(packet)
    local wire, err, code = input_protocol.encode(packet)
    if wire == nil then
        return nil, err, code
    end
    return input_protocol.decode(wire, {
        session_id = packet.session_id,
        manifest_id = packet.manifest_id,
        sender_id = packet.sender_id,
    })
end

---@param packet InputPacket
---@return InputAuthorityRow[]
function input_protocol.rows(packet)
    local ok, err = input_protocol.validate(packet)
    assert(ok, err)
    return copy_rows(packet.rows)
end

---@param previous InputPacket
---@param incoming InputPacket
---@return InputPacketDuplicateDisposition?, string?, InputPacketErrorCode?
function input_protocol.classify_duplicate(previous, incoming)
    local previous_wire, previous_err, previous_code = input_protocol.encode(previous)
    if previous_wire == nil then
        return nil, previous_err, previous_code
    end
    local incoming_wire, incoming_err, incoming_code = input_protocol.encode(incoming)
    if incoming_wire == nil then
        return nil, incoming_err, incoming_code
    end
    if
        previous.kind ~= incoming.kind
        or previous.session_id ~= incoming.session_id
        or previous.sender_id ~= incoming.sender_id
        or previous.sequence ~= incoming.sequence
    then
        return failure("duplicate", "input packets do not share a sender sequence identity")
    end
    if previous_wire == incoming_wire then
        return "idempotent"
    end
    return failure("packet_conflict", "input sender sequence was reused with different bytes")
end

---@param packet InputPacket
---@param envelope TransportMessage
---@return boolean?, string?, InputPacketErrorCode?
function input_protocol.validate_envelope(packet, envelope)
    local packet_ok, packet_err, packet_code = input_protocol.validate(packet)
    if not packet_ok then
        return nil, packet_err, packet_code
    end
    local envelope_ok, envelope_err = transport_contract.validate(envelope)
    if not envelope_ok then
        return failure("malformed", envelope_err or "input transport envelope is malformed")
    end
    if envelope.type ~= "input" then
        return failure("malformed", "input packet requires an input transport envelope")
    end
    if envelope.seq ~= packet.sequence or envelope.tick ~= packet.transport_tick then
        return failure(
            "tick_mismatch",
            "input packet sequence or transport tick mismatches envelope"
        )
    end
    if envelope.payload ~= input_protocol.encode(packet) then
        return failure("identity_mismatch", "input packet bytes mismatch the transport envelope")
    end
    return true
end

---@param older InputPacket
---@param newer InputPacket
---@return boolean
local function packet_covers(older, newer)
    if
        older.kind ~= newer.kind
        or older.session_id ~= newer.session_id
        or older.manifest_id ~= newer.manifest_id
        or older.sender_id ~= newer.sender_id
        or older.first_input_tick ~= newer.first_input_tick
        or newer.sequence < older.sequence
        or newer.transport_tick < older.transport_tick
    then
        return false
    end
    local newer_rows = {}
    for _, row in ipairs(newer.rows) do
        newer_rows[tostring(row.tick) .. ":" .. tostring(row.slot_index)] = row.sample
    end
    for _, row in ipairs(older.rows) do
        local sample = newer_rows[tostring(row.tick) .. ":" .. tostring(row.slot_index)]
        if sample == nil or not samples_equal(sample, row.sample) then
            return false
        end
    end
    return true
end

-- Queue replacement is safe only when the newer packet carries every byte of
-- authority from the unsent packet. Ordinary next-tick bundles do not cover
-- the row that just fell out of the six-row history and therefore fail closed.
---@param older InputPacket
---@param newer InputPacket
---@return InputPacket?, string?, InputPacketErrorCode?
function input_protocol.supersede_for_backpressure(older, newer)
    local older_ok, older_err, older_code = input_protocol.validate(older)
    if not older_ok then
        return nil, older_err, older_code
    end
    local newer_ok, newer_err, newer_code = input_protocol.validate(newer)
    if not newer_ok then
        return nil, newer_err, newer_code
    end
    if
        older.kind == newer.kind
        and older.session_id == newer.session_id
        and older.sender_id == newer.sender_id
        and older.sequence == newer.sequence
    then
        local disposition, duplicate_err, duplicate_code =
            input_protocol.classify_duplicate(older, newer)
        if disposition == nil then
            return nil, duplicate_err, duplicate_code
        end
        return input_protocol.copy(newer)
    end
    if not packet_covers(older, newer) then
        return failure(
            "backpressure_gap",
            "newer input packet does not cover every unsent authority row"
        )
    end
    return input_protocol.copy(newer)
end

---@param options InputHostBatchOptions
---@param arrivals InputPacketArrival[]
---@return InputPacket?, string?, InputPacketErrorCode?
function input_protocol.canonical_host_batch(options, arrivals)
    if type(options) ~= "table" then
        return failure("malformed", "host input batch options are required")
    end
    local manifest_ok, manifest_err = protocol.validate_manifest(options.manifest)
    if not manifest_ok then
        return failure("malformed", manifest_err or "host input manifest is invalid")
    end
    local manifest_id = protocol.manifest_id(options.manifest)
    local assignment_ok, assignment_err =
        protocol.validate_assignment_manifest(options.manifest, options.assignments)
    if not assignment_ok then
        return failure("ownership_mismatch", assignment_err or "input assignments are invalid")
    end
    local host_ok, host_err, host_code =
        validate_id(options.host_peer_id, "host peer id", protocol.MAX_PEER_ID_BYTES)
    if not host_ok then
        return nil, host_err, host_code
    end
    local sequence_ok, sequence_err, sequence_code =
        validate_integer(options.sequence, "host input sequence", 0, protocol.MAX_SEQUENCE)
    if not sequence_ok then
        return nil, sequence_err, sequence_code
    end
    local transport_ok, transport_err, transport_code = validate_integer(
        options.transport_tick,
        "host batch transport tick",
        0,
        input_frame.MAX_TICK
    )
    if not transport_ok then
        return nil, transport_err, transport_code
    end
    local first_ok, first_err, first_code =
        validate_integer(options.first_input_tick, "host first input tick", 0, input_frame.MAX_TICK)
    if not first_ok then
        return nil, first_err, first_code
    end
    if not is_array(arrivals, transport_contract.MAX_QUEUE_LIMIT) or #arrivals == 0 then
        return failure("malformed", "host input arrivals must be a bounded canonical array")
    end

    local packets = {}
    local authority = {}
    local rows = {}
    for _, arrival in ipairs(arrivals) do
        if
            type(arrival) ~= "table"
            or type(arrival.packet) ~= "table"
            or type(arrival.envelope) ~= "table"
        then
            return failure("malformed", "host input arrival is malformed")
        end
        local packet = arrival.packet
        local packet_ok, packet_err, packet_code = input_protocol.validate(packet)
        if not packet_ok then
            return nil, packet_err, packet_code
        end
        if packet.kind ~= "guest" then
            return failure("malformed", "host collector accepts only local-slot guest bundles")
        end
        if
            packet.session_id ~= options.manifest.session_id
            or packet.manifest_id ~= manifest_id
            or packet.first_input_tick ~= options.first_input_tick
        then
            return failure(
                "identity_mismatch",
                "guest input identity differs from the frozen match"
            )
        end
        local envelope_ok, envelope_err, envelope_code =
            input_protocol.validate_envelope(packet, arrival.envelope)
        if not envelope_ok then
            return nil, envelope_err, envelope_code
        end
        if
            not is_integer(arrival.arrival_tick)
            or arrival.arrival_tick ~= options.transport_tick
            or arrival.arrival_tick < packet.transport_tick
        then
            return failure("tick_mismatch", "guest input arrival is outside this transport batch")
        end
        local peer_ok, peer_err, peer_code = validate_id(
            arrival.transport_peer_id,
            "input transport peer id",
            protocol.MAX_PEER_ID_BYTES
        )
        if not peer_ok then
            return nil, peer_err, peer_code
        end
        local slot_index = packet.rows[1].slot_index
        local assignment = options.assignments[slot_index]
        if assignment.producer_id ~= packet.sender_id then
            return failure("ownership_mismatch", "input producer does not own the declared slot")
        end
        if
            (
                assignment.producer_kind == "peer"
                and arrival.transport_peer_id ~= assignment.producer_id
            )
            or (
                assignment.producer_kind == "bot"
                and arrival.transport_peer_id ~= options.host_peer_id
            )
        then
            return failure("ownership_mismatch", "transport peer cannot carry this input producer")
        end
        if
            arrival.transport_peer_id == options.host_peer_id
            and arrival.arrival_tick - packet.transport_tick
                < input_protocol.FAIRNESS_DELAY_TICKS
        then
            return failure("fairness_delay", "host-local input bypassed the fixed fairness delay")
        end

        local packet_key = packet.sender_id .. ":" .. tostring(packet.sequence)
        local previous = packets[packet_key]
        if previous ~= nil then
            local disposition, duplicate_err, duplicate_code =
                input_protocol.classify_duplicate(previous, packet)
            if disposition == nil then
                return nil, duplicate_err, duplicate_code
            end
        else
            packets[packet_key] = packet
            for _, row in ipairs(packet.rows) do
                local row_key = tostring(row.tick) .. ":" .. tostring(row.slot_index)
                local prior = authority[row_key]
                if prior ~= nil then
                    if not samples_equal(prior.sample, row.sample) then
                        return failure(
                            "authority_conflict",
                            "input authority conflicts within the host transport batch"
                        )
                    end
                else
                    local copied = copy_row(row)
                    authority[row_key] = copied
                    rows[#rows + 1] = copied
                end
            end
        end
    end
    table.sort(rows, row_less)
    if #rows > input_protocol.MAX_HOST_ROWS then
        return failure("wire_too_large", "host batch exceeds the bounded authority row count")
    end
    return input_protocol.new_host({
        session_id = options.manifest.session_id,
        manifest_id = manifest_id,
        sender_id = options.host_peer_id,
        sequence = options.sequence,
        transport_tick = options.transport_tick,
        first_input_tick = options.first_input_tick,
        rows = rows,
    })
end

return input_protocol
