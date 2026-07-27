-- Declarative shapes, strict validation, canonical serialization, and content
-- hashing for the OMP-3 network diagnostic contracts.
--
-- Serialization follows the length-prefixed discipline already used by the
-- research contracts in `sim/research_schema.lua`: every scalar emits its byte
-- length before its bytes, records emit declared field order, and maps emit
-- sorted keys. Delimiter-joined or table-order hashing is ambiguous and is
-- forbidden. This module keeps its own header and version rather than reusing
-- the research serializer, because it adds one concept that serializer has no
-- room for -- the field `domain` below -- and because a diagnostic export must
-- never share a version bump, a preimage, or a vocabulary with a participant
-- payload.
--
-- ## Domains: the separation this schema exists to enforce
--
-- Every field belongs to exactly one domain, and the domain decides which
-- vocabulary the field name is allowed to use:
--
-- | Domain      | Means                                   | Forbidden vocabulary |
-- | ----------- | --------------------------------------- | -------------------- |
-- | `identity`  | Build/protocol/manifest identity.       | wall clock           |
-- | `canonical` | Simulation and tick-space evidence.     | wall clock           |
-- | `runtime`   | Wall-clock and transport observation.   | simulation           |
-- | `anchor`    | The one binding between the two clocks. | neither, both needed |
--
-- So a reader cannot mistake an RTT sample for a tick, or a boundary hash for a
-- frame time, by reading the schema alone: the names that would allow the
-- confusion are rejected at shape-construction time. `anchor` is the sole
-- exception and it earns it -- an anchor exists only to state "input tick X was
-- observed at monotonic time Y, +/- this mapping error", so it must name both
-- and must declare its own error term.
--
-- Shape errors assert: a malformed *shape* is a code bug. Value errors return
-- `nil, err`: diagnostic values arrive from transports, drivers, and operators
-- and are external input.

local fnv1a64 = require("core.fnv1a64")

---@alias DiagnosticsDomain "identity"|"canonical"|"runtime"|"anchor"

---@alias DiagnosticsFieldKind
---| "boolean"
---| "integer"
---| "number"
---| "id" -- bounded join key: no direct identifiers, no paths, no addresses
---| "string" -- bounded opaque machine string
---| "text" -- bounded human-readable text, never a join key
---| "hash" -- lowercase fnv1a64 hex digest
---| "enum"
---| "array"
---| "map"
---| "record"

---@class DiagnosticsField
---@field name string?
---@field kind DiagnosticsFieldKind
---@field domain DiagnosticsDomain? -- Inherited from the enclosing record when absent.
---@field optional boolean?
---@field values table<string, boolean>? -- enum members
---@field element DiagnosticsField? -- array/map element shape
---@field fields DiagnosticsField[]? -- record fields, in canonical order
---@field min number?
---@field max number?
---@field min_length integer?
---@field max_length integer?

---@class DiagnosticsShape : DiagnosticsField
---@field kind "record"
---@field fields DiagnosticsField[]

---@class DiagnosticsSchemaModule
local diagnostics_schema = {}

-- Bumping this changes every canonical preimage and therefore every export
-- digest. It is a coordinated breaking change; see docs/online/net_diagnostics.md.
diagnostics_schema.SERIALIZATION_VERSION = 1

-- Named so a future SHA-256 digest can be introduced as a new value instead of
-- silently reinterpreting stored digests.
diagnostics_schema.DIGEST = "fnv1a64/v1"
diagnostics_schema.HASH_LENGTH = 16

diagnostics_schema.MAX_ID_LENGTH = 128
diagnostics_schema.MAX_STRING_LENGTH = 512
diagnostics_schema.MAX_TEXT_LENGTH = 2048
diagnostics_schema.MAX_ARRAY_LENGTH = 4096
diagnostics_schema.MAX_SAFE_INTEGER = 9007199254740992

-- The marker a bounded collection carries once it has dropped anything. It is a
-- field, never a silently shorter array: a truncated export that looks complete
-- is worse than no export.
diagnostics_schema.TRUNCATED = "truncated"
diagnostics_schema.COMPLETE = "complete"

-- What a redacted field reads as. It is a constant so a spec can assert the
-- absence of the original *and* the presence of the marker: a field that simply
-- vanished cannot be told apart from a field nobody recorded.
diagnostics_schema.REDACTED = "[redacted]"

local HEADER = "GCND"

-- Join-key grammar. Alphanumeric start, then alphanumerics plus `_`, `-`, `.`.
-- `@`, `:`, `/`, and `\` are excluded from the charset itself, so an email
-- address, a URL, a filesystem path, and a bare IPv6 literal all fail on a
-- character rather than on a substring blacklist that could be slipped past.
local ID_PATTERN = "^[%w][%w_%-%.]*$"
local HASH_PATTERN = "^[0-9a-f]+$"

-- Belt and braces over the charset. `..` is the one a charset alone cannot
-- catch, and a dotted quad is the one shape that satisfies the charset while
-- being exactly the direct identifier this schema must never carry.
local FORBIDDEN_ID_SUBSTRINGS = { "..", "@" }
local IPV4_PATTERN = "^%d+%.%d+%.%d+%.%d+$"

-- Free text arriving from a transport is not a controlled vocabulary. A star's
-- `last_error` and a peer event's message are produced by `String(error)` on a
-- WebRTC or DOM exception and by the browser bridge's own prose, and neither
-- this schema nor the transport contract constrains a single byte of it. Once
-- STUN/TURN is configured there is no reason to assume such a string will not
-- contain a candidate line, a relay URL, or a peer address verbatim.
--
-- So free text is *checked*, not merely shortened. A string matching any
-- sensitive shape is replaced whole rather than scrubbed in place: partial
-- scrubbing of a grammar nobody controls is the same half-measure as digesting
-- an SDP instead of dropping it, and it fails the moment the text is shaped
-- slightly differently than expected. A benign "outbound queue reached its
-- limit" survives untouched.
--
-- This deliberately over-rejects. Two or more colons is treated as address-shaped
-- even though a wall-clock time like `12:34:56` also matches, because losing one
-- timestamp from a diagnostic detail is cheap and leaking an IPv6 address is not.
local SENSITIVE_TEXT_SUBSTRINGS = {
    "ice-",
    "candidate",
    "fingerprint",
    "sdp",
    "stun:",
    "turn:",
    "turns:",
    "://",
    "@",
}

-- SDP body lines are `<key>=<value>` at a line start. Matched at line starts
-- only, so ordinary prose such as "data=17 rejected" is not swept up.
local SDP_LINE_KEYS = { "v", "o", "s", "c", "t", "m", "a", "b", "i", "u", "k", "r", "z" }

-- Vocabulary guards. These are matched against lowercase field names.
local WALL_CLOCK_WORDS = {
    "_ms$",
    "^ms_",
    "_ms_",
    "rtt",
    "jitter",
    "wall",
    "monotonic",
    "elapsed",
    "timestamp",
    "_at$",
    "second",
    "millis",
    "realtime",
    "frame_time",
}

local SIMULATION_WORDS = {
    "tick",
    "boundary",
    "hash",
    "checkpoint",
    "confirmed",
    "rollback",
    "resimulat",
    "snapshot",
}

local DOMAINS = {
    identity = true,
    canonical = true,
    runtime = true,
    anchor = true,
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
        and math.abs(value) <= diagnostics_schema.MAX_SAFE_INTEGER
end

---@param name string
---@param words string[]
---@return string? matched
local function matches_vocabulary(name, words)
    local lowered = name:lower()
    for _, word in ipairs(words) do
        if lowered:match(word) then
            return word
        end
    end
    return nil
end

-- Does this free text look like it carries network or identity material?
---@param text string
---@return boolean
function diagnostics_schema.is_sensitive_text(text)
    if type(text) ~= "string" then
        return false
    end
    local lowered = text:lower()
    for _, needle in ipairs(SENSITIVE_TEXT_SUBSTRINGS) do
        if lowered:find(needle, 1, true) then
            return true
        end
    end
    -- A dotted quad, anywhere in the string.
    if lowered:match("%d+%.%d+%.%d+%.%d+") then
        return true
    end
    -- Address-shaped: two or more colons covers every IPv6 form, compressed or
    -- not, without needing to parse one.
    local colons = 0
    for _ in lowered:gmatch(":") do
        colons = colons + 1
        if colons >= 2 then
            return true
        end
    end
    -- An SDP body line at a line start.
    local probe = "\n" .. lowered:gsub("\r", "\n")
    for _, key in ipairs(SDP_LINE_KEYS) do
        if probe:find("\n" .. key .. "=", 1, true) then
            return true
        end
    end
    return false
end

-- Bound and redact one free-text field. Returns the redaction marker when the
-- text looks sensitive, the truncated text when it is merely long, and the text
-- unchanged otherwise.
---@param text any
---@param max_bytes integer
---@return string?
function diagnostics_schema.redact_free_text(text, max_bytes)
    if type(text) ~= "string" then
        return nil
    end
    if diagnostics_schema.is_sensitive_text(text) then
        return diagnostics_schema.REDACTED
    end
    if #text <= max_bytes then
        return text
    end
    return text:sub(1, max_bytes - #diagnostics_schema.TRUNCATED - 1)
        .. " "
        .. diagnostics_schema.TRUNCATED
end

---@param name string
---@return boolean
function diagnostics_schema.is_wall_clock_name(name)
    return matches_vocabulary(name, WALL_CLOCK_WORDS) ~= nil
end

---@param name string
---@return boolean
function diagnostics_schema.is_simulation_name(name)
    return matches_vocabulary(name, SIMULATION_WORDS) ~= nil
end

---@param field DiagnosticsField
---@param domain DiagnosticsDomain
---@param path string
---@param seen { wall: boolean, simulation: boolean, mapping_error: boolean }
local function assert_domain(field, domain, path, seen)
    local name = field.name
    if name == nil then
        return
    end
    local wall = matches_vocabulary(name, WALL_CLOCK_WORDS)
    local simulation = matches_vocabulary(name, SIMULATION_WORDS)
    if wall then
        seen.wall = true
    end
    if simulation then
        seen.simulation = true
    end
    if name == "mapping_error_ms" then
        seen.mapping_error = true
    end
    if domain == "identity" or domain == "canonical" then
        assert(
            wall == nil,
            path
                .. " is "
                .. domain
                .. " and may not use wall-clock vocabulary ("
                .. tostring(wall)
                .. ")"
        )
    elseif domain == "runtime" then
        assert(
            simulation == nil,
            path
                .. " is runtime observation and may not use simulation vocabulary ("
                .. tostring(simulation)
                .. ")"
        )
    end
end

---@param field DiagnosticsField
---@param inherited DiagnosticsDomain?
---@param path string
---@param seen { wall: boolean, simulation: boolean, mapping_error: boolean }?
local function assert_field_shape(field, inherited, path, seen)
    assert(type(field) == "table", path .. " shape must be a table")
    assert(type(field.kind) == "string", path .. " shape needs a kind")
    local domain = field.domain or inherited
    if field.domain ~= nil then
        assert(DOMAINS[field.domain], path .. " declares an unknown domain")
        assert(
            inherited == nil or inherited == field.domain,
            path .. " may not change domain from " .. tostring(inherited)
        )
    end
    -- One scope per anchor subtree, keyed on the *domain transition* into
    -- `anchor` rather than on being the outermost call.
    --
    -- Keying it on `seen == nil` was wrong twice over, and both ways mattered.
    -- The shape that actually ships (`net_diagnostics.EXPORT`) is a domainless
    -- container, so a scope was created at the root and threaded into every
    -- descendant; by the time the nested `anchors` field was reached `seen` was
    -- never nil, and the completeness assertion below never ran for the shipped
    -- schema at all. It fired only for an anchor that happened to be top-level,
    -- which is to say only in this module's own unit tests. And because that one
    -- root scope accumulated flags from every unrelated identity, canonical, and
    -- runtime field in the tree, an anchor could have satisfied "names both
    -- vocabularies" using its siblings' field names rather than its own.
    local entering_anchor = domain == "anchor" and inherited ~= "anchor"
    local scope = seen
    if entering_anchor or scope == nil then
        scope = { wall = false, simulation = false, mapping_error = false }
    end
    if domain ~= nil then
        assert_domain(field, domain, path, scope)
    else
        -- Only a container may be domainless, and only so its children can each
        -- declare one. A leaf with no domain would escape the vocabulary guard.
        assert(
            field.kind == "record" or field.kind == "array" or field.kind == "map",
            path .. " has no domain and none is inherited"
        )
    end
    if field.kind == "enum" then
        assert(type(field.values) == "table", path .. " enum shape needs values")
        local count = 0
        for member, allowed in pairs(field.values) do
            assert(type(member) == "string" and allowed == true, path .. " enum member is invalid")
            count = count + 1
        end
        assert(count > 0, path .. " enum shape needs at least one member")
    elseif field.kind == "array" or field.kind == "map" then
        local element = assert(field.element, path .. " needs an element shape")
        assert_field_shape(element, domain, path .. "[]", scope)
    elseif field.kind == "record" then
        local fields = assert(field.fields, path .. " record shape needs fields")
        local names = {}
        for _, child in ipairs(fields) do
            assert(type(child.name) == "string" and child.name ~= "", path .. " field needs a name")
            assert(not names[child.name], path .. " field " .. child.name .. " is declared twice")
            names[child.name] = true
            assert_field_shape(child, domain, path .. "." .. child.name, scope)
        end
    end
    -- An anchor is the only place both vocabularies may meet, and it only earns
    -- that by actually binding them and by naming its own error term.
    if entering_anchor then
        assert(scope.wall, path .. " anchor must name a wall-clock observation")
        assert(scope.simulation, path .. " anchor must name a simulation tick")
        assert(scope.mapping_error, path .. " anchor must declare mapping_error_ms")
    end
end

-- Build a validated record shape. A broken shape is a programmer error, so this
-- asserts rather than returning a diagnostic.
---@param name string
---@param domain DiagnosticsDomain?
---@param fields DiagnosticsField[]
---@return DiagnosticsShape
function diagnostics_schema.record(name, domain, fields)
    assert(type(name) == "string" and name ~= "", "diagnostic shape needs a name")
    local shape = { name = name, kind = "record", domain = domain, fields = fields }
    assert_field_shape(shape, nil, name, nil)
    ---@cast shape DiagnosticsShape
    return shape
end

-- A record shape nested inside another: it inherits its parent's domain, so it
-- is checked when the parent is built rather than standalone.
---@param name string
---@param fields DiagnosticsField[]
---@return DiagnosticsField
function diagnostics_schema.nested(name, fields)
    return { name = name, kind = "record", fields = fields }
end

---@param members string[]
---@return table<string, boolean>
function diagnostics_schema.enum(members)
    local values = {}
    for _, member in ipairs(members) do
        assert(type(member) == "string" and member ~= "", "enum member must be a non-empty string")
        assert(not values[member], "enum member " .. member .. " is declared twice")
        values[member] = true
    end
    return values
end

-- ---------------------------------------------------------------------------
-- Validation
-- ---------------------------------------------------------------------------

---@param field DiagnosticsField
---@param value any
---@param path string
---@return boolean?, string?
local function validate_string(field, value, path)
    if type(value) ~= "string" then
        return nil, path .. " must be a string"
    end
    local max = field.max_length
        or (field.kind == "text" and diagnostics_schema.MAX_TEXT_LENGTH)
        or (field.kind == "id" and diagnostics_schema.MAX_ID_LENGTH)
        or diagnostics_schema.MAX_STRING_LENGTH
    local min = field.min_length or (field.kind == "text" and 0 or 1)
    if #value < min then
        return nil, path .. " must be at least " .. tostring(min) .. " bytes"
    end
    if #value > max then
        return nil, path .. " must be at most " .. tostring(max) .. " bytes"
    end
    if field.kind == "hash" then
        if #value ~= diagnostics_schema.HASH_LENGTH or not value:match(HASH_PATTERN) then
            return nil, path .. " must be a " .. diagnostics_schema.DIGEST .. " hex digest"
        end
        return true
    end
    if field.kind == "id" then
        if not value:match(ID_PATTERN) then
            return nil, path .. " is not a bounded pseudonymous id"
        end
        for _, forbidden in ipairs(FORBIDDEN_ID_SUBSTRINGS) do
            if value:find(forbidden, 1, true) then
                return nil, path .. " may not contain " .. forbidden
            end
        end
        if value:match(IPV4_PATTERN) then
            return nil, path .. " looks like a raw address"
        end
    end
    return true
end

---@param field DiagnosticsField
---@param value any
---@param path string
---@return boolean?, string?
local function validate_field(field, value, path)
    local kind = field.kind
    if kind == "boolean" then
        if type(value) ~= "boolean" then
            return nil, path .. " must be a boolean"
        end
        return true
    end
    if kind == "integer" then
        if not is_integer(value) then
            return nil, path .. " must be a finite integer"
        end
    elseif kind == "number" then
        if not is_finite(value) then
            return nil, path .. " must be a finite number"
        end
    end
    if kind == "integer" or kind == "number" then
        if field.min ~= nil and value < field.min then
            return nil, path .. " must be at least " .. tostring(field.min)
        end
        if field.max ~= nil and value > field.max then
            return nil, path .. " must be at most " .. tostring(field.max)
        end
        return true
    end
    if kind == "enum" then
        if type(value) ~= "string" then
            return nil, path .. " must be a string"
        end
        if not assert(field.values)[value] then
            return nil, path .. " is not a declared member"
        end
        return true
    end
    if kind == "id" or kind == "string" or kind == "text" or kind == "hash" then
        return validate_string(field, value, path)
    end
    if kind == "array" then
        if type(value) ~= "table" then
            return nil, path .. " must be an array"
        end
        local max = field.max_length or diagnostics_schema.MAX_ARRAY_LENGTH
        local count = #value
        if count > max then
            return nil, path .. " holds more than " .. tostring(max) .. " entries"
        end
        for index = 1, count do
            local ok, err =
                validate_field(assert(field.element), value[index], path .. "[" .. index .. "]")
            if not ok then
                return nil, err
            end
        end
        return true
    end
    if kind == "map" then
        if type(value) ~= "table" then
            return nil, path .. " must be a map"
        end
        local count = 0
        for key, child in pairs(value) do
            if type(key) ~= "string" then
                return nil, path .. " map keys must be strings"
            end
            local ok, err = validate_string({ kind = "id" }, key, path .. " key")
            if not ok then
                return nil, err
            end
            count = count + 1
            local child_ok, child_err =
                validate_field(assert(field.element), child, path .. "." .. key)
            if not child_ok then
                return nil, child_err
            end
        end
        local max = field.max_length or diagnostics_schema.MAX_ARRAY_LENGTH
        if count > max then
            return nil, path .. " holds more than " .. tostring(max) .. " entries"
        end
        return true
    end
    if kind == "record" then
        if type(value) ~= "table" then
            return nil, path .. " must be a record"
        end
        local declared = {}
        for _, child in ipairs(assert(field.fields)) do
            declared[child.name] = true
            local child_value = value[child.name]
            if child_value == nil then
                if not child.optional then
                    return nil, path .. "." .. tostring(child.name) .. " is required"
                end
            else
                local ok, err =
                    validate_field(child, child_value, path .. "." .. tostring(child.name))
                if not ok then
                    return nil, err
                end
            end
        end
        for key in pairs(value) do
            if not declared[key] then
                return nil, path .. "." .. tostring(key) .. " is not declared by this schema"
            end
        end
        return true
    end
    return nil, path .. " has an unsupported kind"
end

-- Validate a value against a shape. Returns `nil, err` rather than asserting:
-- diagnostic values come from transports, drivers, and operators.
---@param shape DiagnosticsShape
---@param value any
---@return boolean?, string?
function diagnostics_schema.validate(shape, value)
    return validate_field(shape, value, assert(shape.name))
end

-- ---------------------------------------------------------------------------
-- Canonical serialization
-- ---------------------------------------------------------------------------

---@param text string
---@return string
local function lp(text)
    return tostring(#text) .. ":" .. text
end

-- Integers print exactly; other numbers print with a fixed 17-significant-digit
-- format so the preimage cannot depend on `tostring`'s locale or precision.
---@param value number
---@return string
local function number_bytes(value)
    if value == math.floor(value) and math.abs(value) <= diagnostics_schema.MAX_SAFE_INTEGER then
        return ("%d"):format(value)
    end
    return ("%.17g"):format(value)
end

---@param field DiagnosticsField
---@param value any
---@param out string[]
local function encode_field(field, value, out)
    local kind = field.kind
    if kind == "boolean" then
        out[#out + 1] = value and "T" or "F"
        return
    end
    if kind == "integer" or kind == "number" then
        out[#out + 1] = lp(number_bytes(value))
        return
    end
    if kind == "array" then
        out[#out + 1] = lp(tostring(#value))
        for index = 1, #value do
            encode_field(assert(field.element), value[index], out)
        end
        return
    end
    if kind == "map" then
        local keys = {}
        for key in pairs(value) do
            keys[#keys + 1] = key
        end
        table.sort(keys)
        out[#out + 1] = lp(tostring(#keys))
        for _, key in ipairs(keys) do
            out[#out + 1] = lp(key)
            encode_field(assert(field.element), value[key], out)
        end
        return
    end
    if kind == "record" then
        for _, child in ipairs(assert(field.fields)) do
            local child_value = value[child.name]
            if child_value == nil then
                out[#out + 1] = "-"
            else
                out[#out + 1] = "+"
                encode_field(child, child_value, out)
            end
        end
        return
    end
    out[#out + 1] = lp(value)
end

-- Canonical bytes for a validated value. Byte-identical for byte-identical
-- input, on every platform, independent of table order.
---@param shape DiagnosticsShape
---@param value any
---@return string?, string?
function diagnostics_schema.encode(shape, value)
    local ok, err = diagnostics_schema.validate(shape, value)
    if not ok then
        return nil, err
    end
    local out = { HEADER, ";", tostring(diagnostics_schema.SERIALIZATION_VERSION), ";" }
    out[#out + 1] = lp(assert(shape.name))
    encode_field(shape, value, out)
    return table.concat(out)
end

---@param shape DiagnosticsShape
---@param value any
---@return string?, string?
function diagnostics_schema.digest(shape, value)
    local bytes, err = diagnostics_schema.encode(shape, value)
    if bytes == nil then
        return nil, err
    end
    return fnv1a64.hash(bytes)
end

---@param label string
---@param parts string[]
---@return string
function diagnostics_schema.tuple_digest(label, parts)
    local buffer =
        { HEADER, ";", tostring(diagnostics_schema.SERIALIZATION_VERSION), ";", lp(label) }
    buffer[#buffer + 1] = lp(tostring(#parts))
    for _, part in ipairs(parts) do
        buffer[#buffer + 1] = lp(part)
    end
    return fnv1a64.hash(table.concat(buffer))
end

-- A sub-shape holding only the top-level sections in `allowed`. The point is a
-- digest over *just* the deterministic half of an artifact: canonical evidence
-- can honestly be claimed byte-stable across runs, and runtime observation
-- cannot, so the two need separate preimages rather than one preimage and an
-- optimistic assertion.
---@param shape DiagnosticsShape
---@param allowed table<DiagnosticsDomain, boolean>
---@return DiagnosticsShape
function diagnostics_schema.project(shape, allowed)
    ---@type DiagnosticsField[]
    local kept = {}
    ---@type string[]
    local names = {}
    for _, field in ipairs(shape.fields) do
        if field.domain ~= nil and allowed[field.domain] then
            kept[#kept + 1] = field
            names[#names + 1] = field.domain
        end
    end
    assert(#kept > 0, "a projection must keep at least one section")
    table.sort(names)
    return diagnostics_schema.record(
        assert(shape.name) .. "/" .. table.concat(names, "+"),
        nil,
        kept
    )
end

-- Walk a shape and report every declared field path in its domain. Specs use
-- this to assert the separation holds across the *whole* schema rather than at
-- the handful of fields a hand-written test happens to name.
---@param shape DiagnosticsShape
---@return table<string, DiagnosticsDomain>
function diagnostics_schema.domains(shape)
    ---@type table<string, DiagnosticsDomain>
    local result = {}
    ---@param field DiagnosticsField
    ---@param inherited DiagnosticsDomain?
    ---@param path string
    local function walk(field, inherited, path)
        local domain = field.domain or inherited
        if field.name ~= nil then
            result[path] = domain
        end
        if field.kind == "record" then
            for _, child in ipairs(assert(field.fields)) do
                walk(child, domain, path .. "." .. tostring(child.name))
            end
        elseif field.kind == "array" or field.kind == "map" then
            walk(assert(field.element), domain, path .. "[]")
        end
    end
    walk(shape, nil, assert(shape.name))
    return result
end

return diagnostics_schema
