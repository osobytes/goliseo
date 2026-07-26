-- Declarative shapes, strict validation, canonical serialization, and content
-- hashing for the versioned human-playtest research contracts.
--
-- Research payloads arrive from tooling, operators, and participants, so every
-- public reader here is an external-input validator: it returns `nil, err` and
-- never guesses. Programmer invariants (a malformed *shape*, not malformed
-- data) still assert, because an unusable shape is a code bug.
--
-- Serialization follows the length-prefixed `lp(value)` discipline recorded in
-- docs/design/combat_fun_evidence_contract.md section 4.0: every scalar emits
-- its byte length before its bytes, records emit declared field order, and maps
-- emit sorted keys. Delimiter-joined or table-order hashing is forbidden
-- because it is ambiguous.

local fnv1a64 = require("core.fnv1a64")
local match_snapshot = require("sim.match_snapshot")

---@alias ResearchFieldKind
---| "boolean"
---| "integer"
---| "number"
---| "string" -- opaque bounded machine string (hashes excluded, see "hash")
---| "id" -- join key: bounded charset, no direct identifiers
---| "text" -- bounded human free text, never a join key
---| "hash" -- lowercase fnv1a64 hex digest produced by this module
---| "enum"
---| "array"
---| "map"
---| "record"

---@class ResearchField
---@field name string
---@field kind ResearchFieldKind
---@field optional boolean?
---@field values table<string, boolean>? -- enum members
---@field element ResearchField? -- array/map element shape
---@field fields ResearchField[]? -- record fields, in canonical order
---@field min number? -- inclusive numeric lower bound
---@field max number? -- inclusive numeric upper bound
---@field min_length integer? -- inclusive string/array length lower bound
---@field max_length integer? -- inclusive string/array length upper bound

---@class ResearchShape : ResearchField
---@field kind "record"
---@field fields ResearchField[]

---@class ResearchSchemaModule
local research_schema = {}

-- Bumping this changes every canonical preimage in the research contracts and
-- therefore every content hash. It is a coordinated breaking change; see
-- docs/design/player_evidence_contracts.md for the bump rules.
research_schema.SERIALIZATION_VERSION = 1

-- Named so a future SHA-256 tooling digest can be introduced as a new value
-- instead of silently reinterpreting stored hashes.
research_schema.DIGEST = "fnv1a64/v1"
research_schema.HASH_LENGTH = 16

research_schema.MAX_STRING_LENGTH = 512
research_schema.MAX_TEXT_LENGTH = 4096
research_schema.MAX_ID_LENGTH = 96
research_schema.MAX_ARRAY_LENGTH = 100000
research_schema.MAX_SAFE_INTEGER = 9007199254740992

local HEADER = "GCRS"
-- Slug grammar for join keys: lowercase alphanumerics plus `_`, `-`, and `.`.
-- `/` and `:` are deliberately excluded so a pasted filesystem path or URL
-- cannot satisfy the grammar at all — `home/oscar/save.json` and
-- `c:/users/oscar/save.json` both fail on the separator, not on a substring
-- blacklist that a forward-slash path would slip past.
local ID_PATTERN = "^[a-z0-9][a-z0-9_%-%.]*$"
local HASH_PATTERN = "^[0-9a-f]+$"

-- Belt-and-braces on top of ID_PATTERN: these substrings mark a direct
-- identifier or a path even when every character is individually legal. `..`
-- is the one the charset alone cannot catch.
local FORBIDDEN_ID_SUBSTRINGS = {
    "@",
    "..",
}

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

---@param value any
---@return boolean
local function is_integer(value)
    return is_finite(value)
        and value == math.floor(value)
        and math.abs(value) <= research_schema.MAX_SAFE_INTEGER
end

---@param value any
---@return any
function research_schema.copy(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, child in pairs(value) do
        result[research_schema.copy(key)] = research_schema.copy(child)
    end
    return result
end

---@param field ResearchField
---@param path string
local function assert_field_shape(field, path)
    assert(type(field) == "table", path .. " shape must be a table")
    assert(type(field.kind) == "string", path .. " shape needs a kind")
    if field.kind == "enum" then
        assert(type(field.values) == "table", path .. " enum shape needs values")
        local count = 0
        for member, allowed in pairs(field.values) do
            assert(type(member) == "string" and allowed == true, path .. " enum member is invalid")
            count = count + 1
        end
        assert(count > 0, path .. " enum shape needs at least one member")
    elseif field.kind == "array" or field.kind == "map" then
        assert_field_shape(assert(field.element, path .. " needs an element shape"), path .. "[]")
    elseif field.kind == "record" then
        local fields = assert(field.fields, path .. " record shape needs fields")
        local seen = {}
        for index, child in ipairs(fields) do
            assert(type(child.name) == "string" and child.name ~= "", path .. " field needs a name")
            assert(not seen[child.name], path .. " field " .. child.name .. " is declared twice")
            seen[child.name] = true
            assert_field_shape(child, path .. "." .. child.name)
            assert(index <= #fields, path .. " record fields must be dense")
        end
    end
end

-- Build a validated record shape. A broken shape is a programmer error, so this
-- asserts rather than returning a diagnostic.
---@param name string
---@param fields ResearchField[]
---@return ResearchShape
function research_schema.record(name, fields)
    local shape = { name = name, kind = "record", fields = fields }
    assert(type(name) == "string" and name ~= "", "research shape needs a name")
    assert_field_shape(shape, name)
    ---@cast shape ResearchShape
    return shape
end

---@param members string[]
---@return table<string, boolean>
function research_schema.enum(members)
    local values = {}
    for _, member in ipairs(members) do
        assert(type(member) == "string" and member ~= "", "enum member must be a non-empty string")
        assert(not values[member], "enum member " .. member .. " is declared twice")
        values[member] = true
    end
    return values
end

---@param values table<string, boolean>
---@return string[]
local function sorted_members(values)
    local members = {}
    for member in pairs(values) do
        members[#members + 1] = member
    end
    table.sort(members)
    return members
end

---@param field ResearchField
---@param value any
---@param path string
---@return boolean?, string?
local function validate_string(field, value, path)
    if type(value) ~= "string" then
        return nil, path .. " must be a string"
    end
    local max = field.max_length
        or (field.kind == "text" and research_schema.MAX_TEXT_LENGTH)
        or (field.kind == "id" and research_schema.MAX_ID_LENGTH)
        or research_schema.MAX_STRING_LENGTH
    local min = field.min_length or (field.kind == "text" and 0 or 1)
    if #value < min then
        return nil, path .. " must be at least " .. tostring(min) .. " bytes"
    end
    if #value > max then
        return nil, path .. " must be at most " .. tostring(max) .. " bytes"
    end
    if field.kind == "hash" then
        if #value ~= research_schema.HASH_LENGTH or not value:match(HASH_PATTERN) then
            return nil, path .. " must be a " .. research_schema.DIGEST .. " hex digest"
        end
        return true
    end
    if field.kind == "id" then
        if not value:match(ID_PATTERN) then
            return nil, path .. " must be a lowercase bounded id"
        end
        for _, forbidden in ipairs(FORBIDDEN_ID_SUBSTRINGS) do
            if value:find(forbidden, 1, true) then
                return nil, path .. " must not contain a direct identifier or raw path"
            end
        end
        return true
    end
    if value:find("[%c]") then
        return nil, path .. " must not contain control characters"
    end
    return true
end

---@param field ResearchField
---@param value any
---@param path string
---@return boolean?, string?
local function validate_numeric(field, value, path)
    if field.kind == "integer" then
        if not is_integer(value) then
            return nil, path .. " must be a finite integer"
        end
    elseif not is_finite(value) then
        return nil, path .. " must be a finite number"
    end
    if field.min and value < field.min then
        return nil, path .. " must be at least " .. tostring(field.min)
    end
    if field.max and value > field.max then
        return nil, path .. " must be at most " .. tostring(field.max)
    end
    return true
end

---@param field ResearchField
---@param value any
---@param path string
---@return boolean?, string?
local function validate_value(field, value, path)
    local kind = field.kind
    if kind == "boolean" then
        if type(value) ~= "boolean" then
            return nil, path .. " must be a boolean"
        end
        return true
    elseif kind == "integer" or kind == "number" then
        return validate_numeric(field, value, path)
    elseif kind == "string" or kind == "id" or kind == "text" or kind == "hash" then
        return validate_string(field, value, path)
    elseif kind == "enum" then
        if type(value) ~= "string" then
            return nil, path .. " must be an enum string"
        end
        if not assert(field.values)[value] then
            return nil,
                path .. " must be one of " .. table.concat(
                    sorted_members(assert(field.values)),
                    "|"
                )
        end
        return true
    elseif kind == "array" then
        if type(value) ~= "table" then
            return nil, path .. " must be an array"
        end
        local count = #value
        local seen = 0
        for key in pairs(value) do
            if type(key) ~= "number" or key ~= math.floor(key) or key < 1 or key > count then
                return nil, path .. " must be a dense array"
            end
            seen = seen + 1
        end
        if seen ~= count then
            return nil, path .. " must be a dense array"
        end
        if field.min_length and count < field.min_length then
            return nil, path .. " needs at least " .. tostring(field.min_length) .. " entries"
        end
        local max = field.max_length or research_schema.MAX_ARRAY_LENGTH
        if count > max then
            return nil, path .. " allows at most " .. tostring(max) .. " entries"
        end
        local element = assert(field.element)
        for index = 1, count do
            local ok, err = validate_value(element, value[index], path .. "." .. index)
            if not ok then
                return nil, err
            end
        end
        return true
    elseif kind == "map" then
        if type(value) ~= "table" then
            return nil, path .. " must be a map"
        end
        local element = assert(field.element)
        local count = 0
        for key, child in pairs(value) do
            local key_ok, key_err =
                validate_string({ name = "key", kind = "id" }, key, path .. " key")
            if not key_ok then
                return nil, key_err
            end
            local ok, err = validate_value(element, child, path .. "." .. tostring(key))
            if not ok then
                return nil, err
            end
            count = count + 1
        end
        if field.min_length and count < field.min_length then
            return nil, path .. " needs at least " .. tostring(field.min_length) .. " entries"
        end
        return true
    elseif kind == "record" then
        if type(value) ~= "table" then
            return nil, path .. " must be a table"
        end
        local declared = {}
        for _, child in ipairs(assert(field.fields)) do
            declared[child.name] = child
        end
        -- Strict readers reject unknown fields instead of ignoring them: an
        -- unknown field means the writer knows something this reader does not.
        for key in pairs(value) do
            if type(key) ~= "string" or not declared[key] then
                return nil, path .. " has unknown field " .. tostring(key)
            end
        end
        for _, child in ipairs(assert(field.fields)) do
            local child_value = value[child.name]
            local child_path = path == "" and child.name or (path .. "." .. child.name)
            if child_value == nil then
                if not child.optional then
                    return nil, child_path .. " is required"
                end
            else
                local ok, err = validate_value(child, child_value, child_path)
                if not ok then
                    return nil, err
                end
            end
        end
        return true
    end
    return nil, path .. " has unsupported kind " .. tostring(kind)
end

---@param shape ResearchShape
---@param value any
---@return boolean?, string?
function research_schema.validate(shape, value)
    assert(type(shape) == "table" and shape.kind == "record", "research shape is required")
    return validate_value(shape, value, shape.name)
end

---@param parts string[]
---@param bytes string
local function emit_lp(parts, bytes)
    parts[#parts + 1] = tostring(#bytes)
    parts[#parts + 1] = ":"
    parts[#parts + 1] = bytes
    parts[#parts + 1] = ";"
end

---@param parts string[]
---@param field ResearchField
---@param value any
local function encode_value(parts, field, value)
    if value == nil then
        parts[#parts + 1] = "n;"
        return
    end
    local kind = field.kind
    if kind == "boolean" then
        parts[#parts + 1] = value and "b1;" or "b0;"
    elseif kind == "integer" then
        parts[#parts + 1] = "i"
        emit_lp(parts, ("%d"):format(value))
    elseif kind == "number" then
        parts[#parts + 1] = "d"
        emit_lp(parts, match_snapshot.number_bytes(value))
    elseif kind == "enum" then
        parts[#parts + 1] = "e"
        emit_lp(parts, value)
    elseif kind == "string" or kind == "id" or kind == "text" or kind == "hash" then
        parts[#parts + 1] = "s"
        emit_lp(parts, value)
    elseif kind == "array" then
        parts[#parts + 1] = "a"
        emit_lp(parts, tostring(#value))
        for index = 1, #value do
            encode_value(parts, assert(field.element), value[index])
        end
    elseif kind == "map" then
        local keys = {}
        for key in pairs(value) do
            keys[#keys + 1] = key
        end
        table.sort(keys)
        parts[#parts + 1] = "m"
        emit_lp(parts, tostring(#keys))
        for _, key in ipairs(keys) do
            parts[#parts + 1] = "k"
            emit_lp(parts, key)
            encode_value(parts, assert(field.element), value[key])
        end
    elseif kind == "record" then
        local fields = assert(field.fields)
        parts[#parts + 1] = "r"
        emit_lp(parts, tostring(#fields))
        for _, child in ipairs(fields) do
            parts[#parts + 1] = "k"
            emit_lp(parts, child.name)
            encode_value(parts, child, value[child.name])
        end
    else
        assert(false, "research serialization cannot encode " .. tostring(kind))
    end
end

-- Canonical bytes for a payload that already satisfies `shape`.
---@param shape ResearchShape
---@param value any
---@return string?, string?
function research_schema.encode(shape, value)
    local ok, err = research_schema.validate(shape, value)
    if not ok then
        return nil, err
    end
    local parts = { HEADER, tostring(research_schema.SERIALIZATION_VERSION), ";" }
    emit_lp(parts, shape.name)
    encode_value(parts, shape, value)
    return table.concat(parts)
end

---@class ResearchDecodeCursor
---@field bytes string
---@field at integer

---@param cursor ResearchDecodeCursor
---@param path string
---@return string?, string?
local function read_lp(cursor, path)
    local colon = cursor.bytes:find(":", cursor.at, true)
    if not colon then
        return nil, path .. " is missing a length prefix"
    end
    local length = tonumber(cursor.bytes:sub(cursor.at, colon - 1))
    if not length or not is_integer(length) or length < 0 then
        return nil, path .. " has an invalid length prefix"
    end
    local size = math.floor(length)
    local first = colon + 1
    local last = first + size - 1
    if last + 1 > #cursor.bytes or cursor.bytes:sub(last + 1, last + 1) ~= ";" then
        return nil, path .. " is truncated"
    end
    cursor.at = last + 2
    return cursor.bytes:sub(first, last)
end

---@param payload string
---@return number?
local function decode_number_bytes(payload)
    if payload == "z" then
        return 0
    elseif payload == "Z" then
        return -0.0
    end
    local sign, exponent, high, low = payload:match("^([pm]):(%-?%d+):(%d+):(%d+)$")
    if not sign then
        return nil
    end
    local mantissa = (tonumber(high) + tonumber(low) / 134217728) / 67108864
    local value = math.ldexp(mantissa, assert(tonumber(exponent)))
    return sign == "m" and -value or value
end

---@param cursor ResearchDecodeCursor
---@param field ResearchField
---@param path string
---@return any, string?
local function decode_value(cursor, field, path)
    local tag = cursor.bytes:sub(cursor.at, cursor.at)
    if tag == "" then
        return nil, path .. " is truncated"
    end
    if tag == "n" then
        if cursor.bytes:sub(cursor.at + 1, cursor.at + 1) ~= ";" then
            return nil, path .. " has a malformed absent marker"
        end
        cursor.at = cursor.at + 2
        return nil
    end
    local kind = field.kind
    if kind == "boolean" then
        local wire = cursor.bytes:sub(cursor.at, cursor.at + 2)
        if wire ~= "b1;" and wire ~= "b0;" then
            return nil, path .. " has a malformed boolean"
        end
        cursor.at = cursor.at + 3
        return wire == "b1;"
    end
    cursor.at = cursor.at + 1
    if kind == "integer" then
        if tag ~= "i" then
            return nil, path .. " expected an integer"
        end
        local payload, err = read_lp(cursor, path)
        if not payload then
            return nil, err
        end
        if not payload:match("^%-?%d+$") then
            return nil, path .. " has a non-canonical integer"
        end
        return tonumber(payload)
    elseif kind == "number" then
        if tag ~= "d" then
            return nil, path .. " expected a number"
        end
        local payload, err = read_lp(cursor, path)
        if not payload then
            return nil, err
        end
        local number = decode_number_bytes(payload)
        if number == nil then
            return nil, path .. " has a malformed number"
        end
        return number
    elseif
        kind == "enum"
        or kind == "string"
        or kind == "id"
        or kind == "text"
        or kind == "hash"
    then
        if (kind == "enum" and tag ~= "e") or (kind ~= "enum" and tag ~= "s") then
            return nil, path .. " expected a string"
        end
        return read_lp(cursor, path)
    elseif kind == "array" then
        if tag ~= "a" then
            return nil, path .. " expected an array"
        end
        local payload, err = read_lp(cursor, path)
        if not payload then
            return nil, err
        end
        local count = tonumber(payload)
        if not count or not is_integer(count) or count < 0 then
            return nil, path .. " has an invalid array length"
        end
        local result = {}
        for index = 1, count do
            local child, child_err =
                decode_value(cursor, assert(field.element), path .. "." .. index)
            if child == nil then
                return nil, child_err or (path .. "." .. index .. " is missing")
            end
            result[index] = child
        end
        return result
    elseif kind == "map" then
        if tag ~= "m" then
            return nil, path .. " expected a map"
        end
        local payload, err = read_lp(cursor, path)
        if not payload then
            return nil, err
        end
        local count = tonumber(payload)
        if not count or not is_integer(count) or count < 0 then
            return nil, path .. " has an invalid map length"
        end
        local result = {}
        local previous = nil
        for _ = 1, count do
            if cursor.bytes:sub(cursor.at, cursor.at) ~= "k" then
                return nil, path .. " map key is malformed"
            end
            cursor.at = cursor.at + 1
            local key, key_err = read_lp(cursor, path .. " key")
            if not key then
                return nil, key_err
            end
            if previous and key <= previous then
                return nil, path .. " map keys are not canonically sorted"
            end
            previous = key
            local child, child_err = decode_value(cursor, assert(field.element), path .. "." .. key)
            if child == nil then
                return nil, child_err or (path .. "." .. key .. " is missing")
            end
            result[key] = child
        end
        return result
    elseif kind == "record" then
        if tag ~= "r" then
            return nil, path .. " expected a record"
        end
        local payload, err = read_lp(cursor, path)
        if not payload then
            return nil, err
        end
        local fields = assert(field.fields)
        if tonumber(payload) ~= #fields then
            return nil, path .. " declares a different field count than this reader knows"
        end
        local result = {}
        for _, child in ipairs(fields) do
            if cursor.bytes:sub(cursor.at, cursor.at) ~= "k" then
                return nil, path .. " field key is malformed"
            end
            cursor.at = cursor.at + 1
            local name, name_err = read_lp(cursor, path .. " field name")
            if not name then
                return nil, name_err
            end
            if name ~= child.name then
                return nil, path .. " expected field " .. child.name .. " but found " .. name
            end
            local child_path = path == "" and child.name or (path .. "." .. child.name)
            local value, value_err = decode_value(cursor, child, child_path)
            if value == nil and value_err then
                return nil, value_err
            end
            result[child.name] = value
        end
        return result
    end
    return nil, path .. " has unsupported kind " .. tostring(kind)
end

-- Parse canonical bytes back into a payload and re-validate it. Round-tripping
-- is a contract requirement: encode(decode(bytes)) must reproduce `bytes`.
---@param shape ResearchShape
---@param bytes string
---@return any?, string?
function research_schema.decode(shape, bytes)
    assert(type(shape) == "table" and shape.kind == "record", "research shape is required")
    if type(bytes) ~= "string" then
        return nil, shape.name .. " wire payload must be a string"
    end
    local prefix = HEADER .. tostring(research_schema.SERIALIZATION_VERSION) .. ";"
    if bytes:sub(1, #prefix) ~= prefix then
        local found = bytes:match("^" .. HEADER .. "(%d+);")
        if found then
            return nil,
                shape.name
                    .. " was written by serialization version "
                    .. found
                    .. " and no migration to version "
                    .. tostring(research_schema.SERIALIZATION_VERSION)
                    .. " is registered"
        end
        return nil, shape.name .. " wire payload is not a research contract"
    end
    local cursor = { bytes = bytes, at = #prefix + 1 }
    local name, name_err = read_lp(cursor, shape.name .. " shape name")
    if not name then
        return nil, name_err
    end
    if name ~= shape.name then
        return nil, shape.name .. " wire payload declares shape " .. name
    end
    local value, value_err = decode_value(cursor, shape, shape.name)
    if value == nil then
        return nil, value_err or (shape.name .. " wire payload is empty")
    end
    if cursor.at <= #bytes then
        return nil, shape.name .. " wire payload has trailing bytes"
    end
    local ok, err = research_schema.validate(shape, value)
    if not ok then
        return nil, err
    end
    return value
end

---@param shape ResearchShape
---@param value any
---@return string?, string?
function research_schema.content_hash(shape, value)
    local bytes, err = research_schema.encode(shape, value)
    if not bytes then
        return nil, err
    end
    return fnv1a64.hash(bytes)
end

-- Hash an ordered tuple of already-canonical scalars. Used for compound ids
-- (`run_id`-style tuples) where there is no record shape to lean on.
---@param label string
---@param parts (string|number)[]
---@return string
function research_schema.tuple_hash(label, parts)
    assert(type(label) == "string" and label ~= "", "tuple hash needs a label")
    local buffer = { HEADER, tostring(research_schema.SERIALIZATION_VERSION), ";" }
    emit_lp(buffer, label)
    for index = 1, #parts do
        local part = parts[index]
        if type(part) == "number" then
            assert(is_finite(part), "tuple hash parts must be finite")
            buffer[#buffer + 1] = is_integer(part) and "i" or "d"
            emit_lp(
                buffer,
                is_integer(part) and ("%d"):format(part) or match_snapshot.number_bytes(part)
            )
        else
            assert(type(part) == "string", "tuple hash parts must be strings or numbers")
            buffer[#buffer + 1] = "s"
            emit_lp(buffer, part)
        end
    end
    return fnv1a64.hash(table.concat(buffer))
end

-- Version gate for a stored payload. Unsupported versions stop with a
-- diagnostic that names the missing migration instead of reinterpreting fields.
---@param label string
---@param supported table<integer, boolean>
---@param current integer
---@param version any
---@return boolean?, string?
function research_schema.accepts_version(label, supported, current, version)
    if not is_integer(version) then
        return nil, label .. " schema_version must be an integer"
    end
    if not supported[version] then
        return nil,
            label
                .. " schema_version "
                .. tostring(version)
                .. " is unsupported by reader version "
                .. tostring(current)
                .. " and no migration is registered"
    end
    return true
end

-- Reject a payload whose declared field sets overlap. Split manifests and the
-- simulation/research field partition both need this fail-closed check.
---@param label string
---@param groups table<string, string[]>
---@return boolean?, string?
function research_schema.assert_disjoint(label, groups)
    local owner = {}
    local names = {}
    for name in pairs(groups) do
        names[#names + 1] = name
    end
    table.sort(names)
    for _, name in ipairs(names) do
        for _, member in ipairs(groups[name]) do
            local previous = owner[member]
            if previous then
                return nil,
                    label
                        .. " member "
                        .. member
                        .. " appears in both "
                        .. previous
                        .. " and "
                        .. name
            end
            owner[member] = name
        end
    end
    return true
end

return research_schema
