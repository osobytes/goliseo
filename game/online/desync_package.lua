-- A deterministic desync capture: the smallest artifact that lets someone else
-- reproduce a divergence offline.
--
-- The rule it is built around is that a reproduction needs *inputs and identity*,
-- not a transcript. Given the exact manifest, protocol, and fixture identity, the
-- canonical input wires over a bounded window, and the boundary the peers last
-- agreed on, an offline reproducer can re-derive the divergence. None of that
-- requires a full log, and a full log would be both unbounded and the most likely
-- place for something private to end up.
--
-- ## What "sufficient" means here, exactly
--
-- `reproducible_from` is a required field and it is the package's own honest
-- claim about itself:
--
-- * `fixture_boundary_zero` -- the wires start at the session's first input tick,
--   so a reproducer that can build boundary zero from the pinned fixture has
--   everything. This is the strongest claim and the one the specs exercise.
-- * `tape_reference` -- the wires do not reach boundary zero, but a recorded
--   input tape does, and its id and digest are carried here.
-- * `retained_window` -- neither of the above. The package reproduces the window
--   from the named boundary onward *if* the reproducer can independently produce
--   the state at that boundary. Weakest claim, and it says so.
--
-- A package never silently degrades between these. `build` derives the claim from
-- the evidence it was actually given.
--
-- ## What is deliberately not in here
--
-- No snapshot bytes. A canonical `MatchSnapshot` runs to hundreds of kilobytes,
-- and embedding one would turn a bounded capture into a file nobody attaches to
-- an issue. The agreed boundary is carried as a tick and a hash, which is what a
-- reproducer needs to *check* it rebuilt the right state -- and a hash is
-- evidence, whereas a snapshot is a copy of the match.
--
-- No SDP, no ICE, no addresses, no participant payloads; see
-- `game.online.net_diagnostics` for the redaction boundary this shares.

local schema = require("game.online.diagnostics_schema")
local net_diagnostics = require("game.online.net_diagnostics")
local input_protocol = require("game.online.input_protocol")

---@alias DesyncReproducibleFrom "fixture_boundary_zero"|"tape_reference"|"retained_window"

---@class DesyncTapeReference
---@field tape_id string
---@field tape_digest string
---@field tape_version integer

---@class DesyncPackageOptions
---@field recorder NetDiagnostics
---@field manifest SessionManifest
---@field freeze CoordinatorFreeze
---@field peer_id string
---@field remote_peer_id string
---@field agreed_boundary_tick integer -- Last boundary both peers hashed identically.
---@field agreed_boundary_hash string
---@field divergence_tick integer -- First boundary they disagreed on.
---@field local_hash string
---@field remote_hash string
---@field input_wires string[] -- Canonical input packet wires, sender order preserved.
---@field first_difference MatchSnapshotDifference?
---@field tape DesyncTapeReference?

---@class DesyncPackageModule
local desync_package = {}

desync_package.VERSION = 1

-- 192 wires at the protocol's 1 KiB envelope bound is a 192 KiB ceiling. That is
-- an artifact a person can attach to an issue; a full match of wires is not.
desync_package.MAX_WIRES = 192

local REPRODUCIBLE_FROM = schema.enum({
    "fixture_boundary_zero",
    "tape_reference",
    "retained_window",
})

desync_package.SHAPE = schema.record("desync_package", nil, {
    { name = "package_version", kind = "integer", domain = "identity", min = 1 },
    { name = "digest_algorithm", kind = "string", domain = "identity" },
    {
        name = "session",
        kind = "record",
        domain = "identity",
        fields = net_diagnostics.SESSION_SHAPE,
    },
    {
        name = "reproduction",
        kind = "record",
        domain = "identity",
        fields = {
            { name = "reproducible_from", kind = "enum", values = REPRODUCIBLE_FROM },
            { name = "local_peer_id", kind = "id" },
            { name = "remote_peer_id", kind = "id" },
            {
                name = "tape",
                kind = "record",
                optional = true,
                fields = {
                    { name = "tape_id", kind = "id" },
                    { name = "tape_digest", kind = "hash" },
                    { name = "tape_version", kind = "integer", min = 0 },
                },
            },
        },
    },
    {
        name = "divergence",
        kind = "record",
        domain = "canonical",
        fields = {
            { name = "agreed_boundary_tick", kind = "integer" },
            { name = "agreed_boundary_hash", kind = "hash" },
            { name = "divergence_tick", kind = "integer" },
            { name = "local_hash", kind = "hash" },
            { name = "remote_hash", kind = "hash" },
            { name = "first_difference_path", kind = "string", optional = true },
            { name = "first_difference_expected", kind = "text", optional = true },
            { name = "first_difference_actual", kind = "text", optional = true },
        },
    },
    {
        name = "inputs",
        kind = "record",
        domain = "canonical",
        fields = {
            { name = "input_delay_ticks", kind = "integer", min = 0 },
            { name = "from_input_tick", kind = "integer" },
            { name = "through_input_tick", kind = "integer" },
            { name = "wire_count", kind = "integer", min = 0 },
            { name = "wire_digest", kind = "hash" },
            {
                name = "retention",
                kind = "enum",
                values = schema.enum({ schema.COMPLETE, schema.TRUNCATED }),
            },
            {
                name = "wires",
                kind = "array",
                max_length = desync_package.MAX_WIRES,
                element = { kind = "string", max_length = 1024 },
            },
        },
    },
    {
        name = "checkpoints",
        kind = "array",
        domain = "canonical",
        element = {
            kind = "record",
            fields = {
                { name = "tick", kind = "integer" },
                { name = "hash", kind = "hash" },
                { name = "live", kind = "map", element = { kind = "id" } },
            },
        },
    },
    {
        name = "control",
        kind = "array",
        domain = "canonical",
        element = {
            kind = "record",
            fields = {
                { name = "ordinal", kind = "integer", min = 0 },
                { name = "kind", kind = "id" },
                { name = "peer_id", kind = "id" },
                { name = "sequence", kind = "integer", min = 0 },
                { name = "message_id", kind = "string", max_length = 320 },
            },
        },
    },
    {
        name = "runtime_events",
        kind = "array",
        domain = "runtime",
        element = {
            kind = "record",
            fields = {
                { name = "ordinal", kind = "integer", min = 0 },
                { name = "kind", kind = "id" },
                { name = "monotonic_ms", kind = "number" },
                { name = "peer_id", kind = "id", optional = true },
                { name = "channel", kind = "id", optional = true },
                { name = "state", kind = "string", optional = true },
                { name = "code", kind = "string", optional = true },
                { name = "detail", kind = "text", optional = true },
            },
        },
    },
})

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number" and value == math.floor(value) and value == value
end

---@param text any
---@return string?
local function bounded_text(text)
    if text == nil then
        return nil
    end
    local rendered = type(text) == "number" and ("%.17g"):format(text) or tostring(text)
    if #rendered > schema.MAX_TEXT_LENGTH then
        return rendered:sub(1, schema.MAX_TEXT_LENGTH)
    end
    return rendered
end

-- ---------------------------------------------------------------------------
-- Build
-- ---------------------------------------------------------------------------

-- Assemble a package. Returns `nil, err` on anything malformed: a capture is
-- produced at the worst moment of a session and must never be the thing that
-- raises.
---@param options DesyncPackageOptions
---@return table?, string?
function desync_package.build(options)
    if type(options) ~= "table" then
        return nil, "a desync package needs options"
    end
    local recorder = options.recorder
    local manifest = options.manifest
    local freeze = options.freeze
    if type(recorder) ~= "table" or type(manifest) ~= "table" or type(freeze) ~= "table" then
        return nil, "a desync package needs a recorder, a manifest, and a freeze"
    end
    if not is_integer(options.agreed_boundary_tick) or not is_integer(options.divergence_tick) then
        return nil, "a desync package needs finite integer boundary ticks"
    end
    if options.divergence_tick <= options.agreed_boundary_tick then
        return nil, "the divergence must be later than the boundary the peers agreed on"
    end

    local wires = {}
    local truncated = false
    for _, wire in ipairs(options.input_wires or {}) do
        if type(wire) ~= "string" then
            return nil, "an input wire must be a string"
        end
        if #wires >= desync_package.MAX_WIRES then
            truncated = true
            break
        end
        wires[#wires + 1] = wire
    end

    -- Coverage is read from the wires themselves rather than assumed, because
    -- `reproducible_from` is a claim and an unchecked claim is worse than none.
    local from_tick = math.huge
    local through_tick = -math.huge
    for _, wire in ipairs(wires) do
        local packet = input_protocol.decode(wire, {
            session_id = manifest.session_id,
            manifest_id = freeze.manifest_id,
            sender_id = options.peer_id,
        })
        if packet == nil then
            return nil, "an input wire did not decode against this manifest"
        end
        for _, row in ipairs(packet.rows) do
            from_tick = math.min(from_tick, row.tick)
            through_tick = math.max(through_tick, row.tick)
        end
    end
    if #wires == 0 then
        from_tick = options.agreed_boundary_tick
        through_tick = options.agreed_boundary_tick
    end

    ---@type DesyncReproducibleFrom
    local reproducible_from = "retained_window"
    if not truncated and from_tick <= freeze.first_input_tick then
        reproducible_from = "fixture_boundary_zero"
    elseif options.tape ~= nil then
        reproducible_from = "tape_reference"
    end

    local diagnostics = net_diagnostics.export(recorder)
    if diagnostics == nil then
        return nil, "a desync package needs an opted-in diagnostic export"
    end

    local difference = options.first_difference
    local package = {
        package_version = desync_package.VERSION,
        digest_algorithm = schema.DIGEST,
        session = diagnostics.session,
        reproduction = {
            reproducible_from = reproducible_from,
            local_peer_id = options.peer_id,
            remote_peer_id = options.remote_peer_id,
            tape = options.tape and {
                tape_id = options.tape.tape_id,
                tape_digest = options.tape.tape_digest,
                tape_version = options.tape.tape_version,
            } or nil,
        },
        divergence = {
            agreed_boundary_tick = options.agreed_boundary_tick,
            agreed_boundary_hash = options.agreed_boundary_hash,
            divergence_tick = options.divergence_tick,
            local_hash = options.local_hash,
            remote_hash = options.remote_hash,
            first_difference_path = difference and difference.path or nil,
            first_difference_expected = difference and bounded_text(difference.expected) or nil,
            first_difference_actual = difference and bounded_text(difference.actual) or nil,
        },
        inputs = {
            input_delay_ticks = input_protocol.FAIRNESS_DELAY_TICKS,
            from_input_tick = from_tick,
            through_input_tick = through_tick,
            wire_count = #wires,
            wire_digest = schema.tuple_digest("desync_input_wires", wires),
            retention = truncated and schema.TRUNCATED or schema.COMPLETE,
            wires = wires,
        },
        checkpoints = diagnostics.canonical.checkpoints,
        control = diagnostics.canonical.control,
        runtime_events = diagnostics.runtime.events,
    }
    local ok, err = schema.validate(desync_package.SHAPE, package)
    if not ok then
        return nil, err
    end
    return package
end

---@param package table
---@return string?, string?
function desync_package.encode(package)
    return schema.encode(desync_package.SHAPE, package)
end

---@param package table
---@return string?, string?
function desync_package.digest(package)
    return schema.digest(desync_package.SHAPE, package)
end

-- Decode the package's wires back into canonical `(tick, slot)` order. This is
-- the entry point an offline reproducer uses: feed these rows to a fresh
-- rollback session built from the identity in `session`, step to
-- `divergence.divergence_tick`, and compare.
---@param package table
---@param manifest SessionManifest
---@param sender_id string
---@return InputAuthorityRow[]?, string?
function desync_package.rows(package, manifest, sender_id)
    if type(package) ~= "table" or type(package.inputs) ~= "table" then
        return nil, "a desync package needs an inputs section"
    end
    ---@type InputAuthorityRow[]
    local rows = {}
    ---@type table<string, boolean>
    local seen = {}
    for _, wire in ipairs(package.inputs.wires) do
        local packet, err = input_protocol.decode(wire, {
            session_id = manifest.session_id,
            manifest_id = package.session.manifest_id,
            sender_id = sender_id,
        })
        if packet == nil then
            return nil, err or "an input wire did not decode"
        end
        for _, row in ipairs(packet.rows) do
            -- The redundancy window re-sends the same row on consecutive ticks,
            -- so a naive concatenation would replay each row up to seven times.
            -- Authorship is a partition, so `(tick, slot)` is unique authority
            -- and de-duplicating on it is lossless.
            local key = tostring(row.tick) .. ":" .. tostring(row.slot_index)
            if not seen[key] then
                seen[key] = true
                rows[#rows + 1] = row
            end
        end
    end
    table.sort(rows, function(left, right)
        if left.tick ~= right.tick then
            return left.tick < right.tick
        end
        return left.slot_index < right.slot_index
    end)
    return rows
end

-- A short read-out for an issue comment. Everything here is already in the
-- package and none of it is sensitive.
---@param package table
---@return string[]
function desync_package.summary(package)
    local divergence = package.divergence
    local inputs = package.inputs
    return {
        ("desync  manifest %s  fixture %s  %s"):format(
            package.session.manifest_id,
            package.session.fixture_id,
            package.session.match_mode
        ),
        ("agreed boundary %d  hash %s"):format(
            divergence.agreed_boundary_tick,
            divergence.agreed_boundary_hash
        ),
        ("diverged at %d  local %s  remote %s"):format(
            divergence.divergence_tick,
            divergence.local_hash,
            divergence.remote_hash
        ),
        ("first difference  %s"):format(divergence.first_difference_path or "not resolved locally"),
        ("inputs  %d wires  ticks %d..%d  digest %s  %s"):format(
            inputs.wire_count,
            inputs.from_input_tick,
            inputs.through_input_tick,
            inputs.wire_digest,
            inputs.retention
        ),
        ("reproducible from  %s"):format(package.reproduction.reproducible_from),
    }
end

return desync_package
