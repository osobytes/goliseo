-- Bounded, privacy-safe OMP-3 network diagnostics.
--
-- This module is a *recorder*, not an instrument. It never reaches into the
-- match driver, the coordinator, or a transport: callers hand it the values
-- those modules already publish, and it shapes, bounds, validates, and exports
-- them. That is deliberate -- the driver and the star are settled, and a
-- diagnostic that can perturb the thing it measures is not a diagnostic.
--
-- ## The one distinction this module exists to keep
--
-- Canonical simulation evidence and wall-clock runtime observation are stored in
-- separate sections of the schema, and `game.online.diagnostics_schema` refuses
-- at shape-construction time to let either borrow the other's vocabulary. In
-- prose:
--
-- * **Canonical** is what the simulation says: input ticks, boundaries, hashes,
--   rollback depth, resimulated ticks, the retained floor, the first differing
--   path. Two peers that agree here agree about the match. It is reproducible.
-- * **Runtime** is what the machine and the wire did while that happened: RTT,
--   jitter, queue depth, `bufferedAmount`, ICE transitions, drops. It is a
--   measurement. It is *not* reproducible and is never simulation truth.
-- * **Anchors** are the only bridge, and each one carries its own mapping error.
--
-- Within canonical, `simulation` is simulation truth and `delivery` is observed
-- delivery order recorded in tick space. Delivery order is an *input* to a
-- reproduction (replay the same arrivals and you get the same divergence); it is
-- never evidence about what the simulation should have computed.
--
-- ## Privacy
--
-- Nothing here stores SDP, ICE credentials, raw addresses, clipboard contents,
-- direct identifiers, or any playtest payload. Signalling blobs are recorded as
-- a byte length and an explicit redaction marker and never as content -- see
-- `record_signal`. Every id is validated against a grammar whose charset
-- excludes `@`, `:`, `/`, and `\`, so an email address, a URL, a path, and an
-- IPv6 literal cannot be stored as an id even by mistake, and a dotted quad is
-- rejected by an explicit check on top of that.
--
-- Collection is always bounded and in memory. Export is opt-in and off by
-- default: holding diagnostics is cheap and local, writing them somewhere a
-- human might paste them is a decision.

local schema = require("game.online.diagnostics_schema")
local contract = require("game.transport.contract")
local protocol = require("game.online.protocol")

---@alias NetDiagnosticsRetention "complete"|"truncated"
---@alias NetDiagnosticsKeep "newest"|"oldest"
---@alias NetDiagnosticsSignalDirection "outbound"|"inbound"
---@alias NetDiagnosticsPacketDisposition
---| "authored"
---| "sent"
---| "arrived"
---| "deferred"
---| "duplicate"
---| "rejected"

---@class NetDiagnosticsRing
---@field items any[]
---@field limit integer
---@field head integer -- Next write position, 1-based.
---@field count integer
---@field dropped integer
---@field keep NetDiagnosticsKeep

---@class NetDiagnosticsLimits
---@field checkpoints integer
---@field packets integer
---@field control integer
---@field events integer
---@field signals integer
---@field mismatches integer
---@field anchors integer
---@field latency_peers integer

---@class NetDiagnosticsOptions
---@field role SessionRole
---@field peer_id string
---@field manifest SessionManifest
---@field freeze CoordinatorFreeze
---@field input_delay_ticks integer?
---@field hash_interval_ticks integer?
---@field max_rollback_ticks integer?
---@field network_profile_digest string?
---@field limits NetDiagnosticsLimits?
---@field export_opt_in boolean?

---@class NetDiagnosticsRuntimeSample
---@field peer_id string
---@field monotonic_ms number
---@field rtt_ms number?
---@field jitter_ms number?

---@class NetDiagnosticsAnchor
---@field input_tick integer
---@field monotonic_ms number
---@field mapping_error_ms number

---@class NetDiagnosticsPacket
---@field sender_id string
---@field sequence integer
---@field payload_bytes integer
---@field sample_step integer -- Driver step whose sample this packet's newest row is authority for.
---@field send_transport_tick integer
---@field authority_input_tick integer -- Newest input tick this packet carries authority for.
---@field arrival_transport_tick integer?
---@field apply_input_tick integer? -- Local input tick reached when the authority landed.
---@field row_count integer? -- Only when the observer decoded the packet.
---@field disposition NetDiagnosticsPacketDisposition

---@class NetDiagnosticsEvent
---@field kind "peer_state"|"peer_error"|"star_state"|"star_error"|"signal"|"lifecycle"|"teardown"
---@field monotonic_ms number
---@field peer_id string?
---@field channel TransportChannel?
---@field state string?
---@field code string?
---@field detail string?

---@class NetDiagnosticsMismatch
---@field tick integer
---@field local_hash string
---@field remote_hash string
---@field peer_id string
---@field first_difference_path string?

---@class NetDiagnosticsPressure
---@field samples integer -- Transport snapshots folded; a peak of one sample is not a peak.
---@field peak_outbound_depth integer
---@field peak_inbound_depth integer
---@field peak_buffered_amount integer
---@field peak_event_depth integer
---@field peak_peer_count integer
---@field backpressure integer -- Star-level cumulative latches, highest seen.
---@field peer_backpressure integer -- Highest cumulative latch count on any one link.
---@field overflow integer
---@field dropped_outbound integer
---@field dropped_inbound integer

---@class NetDiagnosticsTeardown
---@field requested boolean
---@field closed_peers integer
---@field orphaned_peers string[]
---@field residual_outbound integer
---@field residual_inbound integer

---@class NetDiagnosticsLatency
---@field peer_id string
---@field sample_count integer
---@field rtt_ms_min number?
---@field rtt_ms_max number?
---@field rtt_ms_last number?
---@field jitter_ms_max number?
---@field jitter_ms_last number?
---@field monotonic_ms_first number?
---@field monotonic_ms_last number?

---@class NetDiagnostics
---@field _session table
---@field _limits NetDiagnosticsLimits
---@field _export_opt_in boolean
---@field _rejected integer
---@field _checkpoints NetDiagnosticsRing
---@field _packets NetDiagnosticsRing
---@field _control NetDiagnosticsRing
---@field _events NetDiagnosticsRing
---@field _signals NetDiagnosticsRing
---@field _mismatches NetDiagnosticsRing
---@field _anchors NetDiagnosticsRing
---@field _latency table<string, NetDiagnosticsLatency>
---@field _latency_order string[]
---@field _simulation table
---@field _delivery table
---@field _events_counts table
---@field _worst table?
---@field _star table?
---@field _peers table[]
---@field _pressure table?
---@field _teardown table?
---@field _tick_digests table<integer, string>
---@field _tick_digest_floor integer
---@field _pending_apply table[]
---@field _ordinal integer

---@class NetDiagnosticsModule
local net_diagnostics = {}

net_diagnostics.SCHEMA_VERSION = 1

-- Caps are deliberately small. A diagnostic ring exists so a failure that
-- happened seconds ago is still explainable; it is not a log, and a diagnostic
-- that grows with match length would be the first thing to blame for a stall.
---@type NetDiagnosticsLimits
net_diagnostics.DEFAULT_LIMITS = {
    checkpoints = 64,
    packets = 256,
    control = 64,
    events = 128,
    signals = 32,
    mismatches = 16,
    anchors = 32,
    latency_peers = contract.MAX_GUESTS + 1,
}

net_diagnostics.MAX_DETAIL_BYTES = 240

local ROLES = schema.enum({ "host", "guest" })
local MATCH_MODES = schema.enum({ "1v1", "2v2", "4v4" })
local COMBAT_STATUS = schema.enum({ "provisional_114", "accepted_proceed", "accepted_revision" })
local DRIVER_STATUS = schema.enum({
    "active",
    "completed",
    "settle_timeout",
    "confirmation_stalled",
    "late_input",
    "hash_mismatch",
    "ownership_violation",
    "authority_conflict",
    "input_channel_failure",
    "transport_lost",
})
local NETCODE_FAILURE = schema.enum({ "input_channel", "late_input", "desync" })
local RETENTION = schema.enum({ schema.COMPLETE, schema.TRUNCATED })
local TRANSPORT_STATE = schema.enum({ "new", "connected", "disconnected", "closed", "error" })
local PEER_STATE =
    schema.enum({ "new", "connecting", "connected", "disconnected", "closed", "error" })
local CHANNELS = schema.enum({ "control", "input" })
local SIGNAL_DIRECTION = schema.enum({ "outbound", "inbound" })
local EVENT_KIND = schema.enum({
    "peer_state",
    "peer_error",
    "star_state",
    "star_error",
    "signal",
    "lifecycle",
    "teardown",
})
local CONTROL_KIND = schema.enum({
    "handshake",
    "manifest_proposal",
    "manifest_accept",
    "peer_assignment",
    "slot_assignment",
    "ready",
    "pair_preference",
    "pair_preference_result",
    "countdown",
    "start",
    "match_phase",
    "hash_report",
    "result_ack",
    "abort",
    "disconnect",
})
local PACKET_DISPOSITION = schema.enum({
    "authored",
    "sent",
    "arrived",
    "deferred",
    "duplicate",
    "rejected",
})

---@param name string
---@return DiagnosticsField
local function integer_field(name)
    return { name = name, kind = "integer", min = 0 }
end

---@param name string
---@return DiagnosticsField
local function signed_field(name)
    return { name = name, kind = "integer" }
end

---@param name string
---@return DiagnosticsField
local function optional_integer(name)
    return { name = name, kind = "integer", optional = true }
end

---@param name string
---@return DiagnosticsField
local function optional_number(name)
    return { name = name, kind = "number", optional = true }
end

local CHANNEL_FIELDS = {
    { name = "state", kind = "enum", values = PEER_STATE },
    integer_field("outbound_depth"),
    integer_field("inbound_depth"),
    integer_field("buffered_amount"),
    integer_field("sent"),
    integer_field("received"),
    integer_field("dropped_outbound"),
    integer_field("dropped_inbound"),
}

net_diagnostics.SESSION_SHAPE = {
    { name = "session_id", kind = "id" },
    { name = "peer_id", kind = "id" },
    { name = "role", kind = "enum", values = ROLES },
    { name = "match_mode", kind = "enum", values = MATCH_MODES },
    { name = "combat_status", kind = "enum", values = COMBAT_STATUS },
    { name = "manifest_id", kind = "hash" },
    { name = "assignment_id", kind = "hash" },
    { name = "countdown_id", kind = "id" },
    { name = "build_id", kind = "id" },
    { name = "source_id", kind = "id" },
    { name = "content_id", kind = "id" },
    { name = "tuning_id", kind = "id" },
    { name = "match_config_id", kind = "id" },
    { name = "fixture_id", kind = "id" },
    { name = "arena_id", kind = "id" },
    { name = "combat_rules_id", kind = "id" },
    { name = "gameplay_ai_policy_id", kind = "id" },
    { name = "network_profile_digest", kind = "hash", optional = true },
    integer_field("protocol_version"),
    integer_field("input_version"),
    integer_field("snapshot_version"),
    integer_field("tape_version"),
    integer_field("combat_schema_version"),
    integer_field("seed"),
    integer_field("tick_rate"),
    integer_field("duration_ticks"),
    integer_field("max_goals"),
}

-- The exported artifact. Section order here is the canonical export order, and
-- `diagnostics_schema.encode` follows declared order, so two exports of the same
-- values are byte-identical regardless of table iteration.
net_diagnostics.EXPORT = schema.record("net_diagnostics", nil, {
    { name = "schema_version", kind = "integer", domain = "identity", min = 1 },
    { name = "digest_algorithm", kind = "string", domain = "identity" },
    {
        name = "session",
        kind = "record",
        domain = "identity",
        fields = net_diagnostics.SESSION_SHAPE,
    },
    {
        name = "collection",
        kind = "record",
        domain = "identity",
        fields = {
            { name = "export_opt_in", kind = "boolean" },
            integer_field("rejected_values"),
            schema.nested("caps", {
                integer_field("checkpoints"),
                integer_field("packets"),
                integer_field("control"),
                integer_field("events"),
                integer_field("signals"),
                integer_field("mismatches"),
                integer_field("anchors"),
                integer_field("latency_peers"),
            }),
            schema.nested("dropped", {
                integer_field("checkpoints"),
                integer_field("packets"),
                integer_field("control"),
                integer_field("events"),
                integer_field("signals"),
                integer_field("mismatches"),
                integer_field("anchors"),
            }),
            schema.nested("retention", {
                { name = "checkpoints", kind = "enum", values = RETENTION },
                { name = "packets", kind = "enum", values = RETENTION },
                { name = "control", kind = "enum", values = RETENTION },
                { name = "events", kind = "enum", values = RETENTION },
                { name = "signals", kind = "enum", values = RETENTION },
                { name = "mismatches", kind = "enum", values = RETENTION },
                { name = "anchors", kind = "enum", values = RETENTION },
            }),
        },
    },
    {
        name = "canonical",
        kind = "record",
        domain = "canonical",
        fields = {
            schema.nested("simulation", {
                { name = "status", kind = "enum", values = DRIVER_STATUS },
                {
                    name = "terminal_status",
                    kind = "enum",
                    values = DRIVER_STATUS,
                    optional = true,
                },
                {
                    name = "terminal_failure",
                    kind = "enum",
                    values = NETCODE_FAILURE,
                    optional = true,
                },
                { name = "terminal_detail", kind = "text", optional = true },
                optional_integer("terminal_tick"),
                integer_field("step_count"),
                signed_field("first_input_tick"),
                signed_field("present_input_tick"),
                signed_field("confirmed_input_tick"),
                signed_field("confirmed_output_tick"),
                signed_field("retained_floor_tick"),
                integer_field("input_delay_ticks"),
                integer_field("hash_interval_ticks"),
                integer_field("max_rollback_ticks"),
                integer_field("rollback_count"),
                integer_field("correction_count"),
                integer_field("predicted_slot_samples"),
                integer_field("resimulated_ticks"),
                integer_field("max_rollback_depth"),
                integer_field("hash_mismatch_streak"),
                integer_field("checkpoint_count"),
                optional_integer("late_input_tick"),
                { name = "control_slot", kind = "id", optional = true },
                { name = "owned", kind = "array", element = { kind = "id" } },
                { name = "authored", kind = "array", element = { kind = "id" } },
            }),
            {
                name = "worst_correction",
                kind = "record",
                optional = true,
                fields = {
                    signed_field("causal_tick"),
                    signed_field("through_tick"),
                    integer_field("depth"),
                    integer_field("resimulated_ticks"),
                },
            },
            schema.nested("events", {
                integer_field("emitted"),
                integer_field("added"),
                integer_field("revoked"),
                integer_field("replaced"),
                integer_field("unchanged"),
                integer_field("resimulated_tick_count"),
            }),
            schema.nested("delivery", {
                integer_field("authored"),
                integer_field("published"),
                integer_field("sent"),
                integer_field("arrived"),
                integer_field("applied_rows"),
                integer_field("deferred"),
                integer_field("duplicate"),
                integer_field("rejected"),
                integer_field("reconciliations"),
                {
                    name = "packets",
                    kind = "array",
                    element = {
                        kind = "record",
                        fields = {
                            { name = "sender_id", kind = "id" },
                            {
                                name = "disposition",
                                kind = "enum",
                                values = PACKET_DISPOSITION,
                            },
                            integer_field("sequence"),
                            integer_field("payload_bytes"),
                            signed_field("sample_step"),
                            signed_field("send_transport_tick"),
                            signed_field("authority_input_tick"),
                            optional_integer("arrival_transport_tick"),
                            optional_integer("apply_input_tick"),
                            optional_integer("row_count"),
                        },
                    },
                },
            }),
            {
                name = "checkpoints",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        signed_field("tick"),
                        { name = "hash", kind = "hash" },
                        { name = "live", kind = "map", element = { kind = "id" } },
                    },
                },
            },
            {
                name = "mismatches",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        signed_field("tick"),
                        { name = "peer_id", kind = "id" },
                        { name = "local_hash", kind = "hash" },
                        { name = "remote_hash", kind = "hash" },
                        { name = "first_difference_path", kind = "string", optional = true },
                    },
                },
            },
            -- Control traffic is canonical, not runtime: a session control
            -- message is identified by a sender-local sequence and a derived
            -- message id, both reproducible, and the order they were accepted in
            -- is part of what an offline reproduction has to replay.
            {
                name = "control",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        integer_field("ordinal"),
                        { name = "kind", kind = "enum", values = CONTROL_KIND },
                        { name = "peer_id", kind = "id" },
                        integer_field("sequence"),
                        -- Not an `id`: the derived message id is length-prefixed
                        -- and contains `:`, which the join-key charset excludes
                        -- on purpose. It stays an opaque bounded string.
                        { name = "message_id", kind = "string", max_length = 320 },
                    },
                },
            },
        },
    },
    {
        name = "runtime",
        kind = "record",
        domain = "runtime",
        fields = {
            {
                name = "star",
                kind = "record",
                optional = true,
                fields = {
                    { name = "role", kind = "enum", values = ROLES },
                    { name = "state", kind = "enum", values = TRANSPORT_STATE },
                    integer_field("capacity"),
                    integer_field("peer_count"),
                    integer_field("queue_limit"),
                    integer_field("buffered_amount_limit"),
                    integer_field("event_depth"),
                    integer_field("sent"),
                    integer_field("received"),
                    integer_field("dropped_outbound"),
                    integer_field("dropped_inbound"),
                    integer_field("malformed"),
                    integer_field("unsupported_version"),
                    integer_field("overflow"),
                    integer_field("backpressure"),
                    { name = "last_error", kind = "text", optional = true },
                },
            },
            {
                name = "peers",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        { name = "peer_id", kind = "id" },
                        integer_field("slot"),
                        { name = "state", kind = "enum", values = PEER_STATE },
                        { name = "ice_state", kind = "string" },
                        { name = "control", kind = "record", fields = CHANNEL_FIELDS },
                        { name = "input", kind = "record", fields = CHANNEL_FIELDS },
                        integer_field("sequence_gaps"),
                        integer_field("backpressure"),
                        integer_field("malformed"),
                        { name = "last_error", kind = "text", optional = true },
                    },
                },
            },
            {
                name = "pressure",
                kind = "record",
                optional = true,
                fields = {
                    integer_field("samples"),
                    integer_field("peak_outbound_depth"),
                    integer_field("peak_inbound_depth"),
                    integer_field("peak_buffered_amount"),
                    integer_field("peak_event_depth"),
                    integer_field("peak_peer_count"),
                    integer_field("backpressure"),
                    integer_field("peer_backpressure"),
                    integer_field("overflow"),
                    integer_field("dropped_outbound"),
                    integer_field("dropped_inbound"),
                },
            },
            {
                name = "latency",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        { name = "peer_id", kind = "id" },
                        integer_field("sample_count"),
                        optional_number("rtt_ms_min"),
                        optional_number("rtt_ms_max"),
                        optional_number("rtt_ms_last"),
                        optional_number("jitter_ms_max"),
                        optional_number("jitter_ms_last"),
                        optional_number("monotonic_ms_first"),
                        optional_number("monotonic_ms_last"),
                    },
                },
            },
            {
                name = "events",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        integer_field("ordinal"),
                        { name = "kind", kind = "enum", values = EVENT_KIND },
                        { name = "monotonic_ms", kind = "number" },
                        { name = "peer_id", kind = "id", optional = true },
                        { name = "channel", kind = "enum", values = CHANNELS, optional = true },
                        { name = "state", kind = "string", optional = true },
                        { name = "code", kind = "string", optional = true },
                        { name = "detail", kind = "text", optional = true },
                    },
                },
            },
            {
                name = "signals",
                kind = "array",
                element = {
                    kind = "record",
                    fields = {
                        integer_field("ordinal"),
                        { name = "peer_id", kind = "id" },
                        { name = "direction", kind = "enum", values = SIGNAL_DIRECTION },
                        integer_field("byte_length"),
                        { name = "content", kind = "string" },
                    },
                },
            },
            {
                name = "teardown",
                kind = "record",
                optional = true,
                fields = {
                    { name = "requested", kind = "boolean" },
                    { name = "complete", kind = "boolean" },
                    integer_field("closed_peers"),
                    integer_field("residual_outbound"),
                    integer_field("residual_inbound"),
                    { name = "orphaned_peers", kind = "array", element = { kind = "id" } },
                },
            },
        },
    },
    {
        name = "anchors",
        kind = "array",
        domain = "anchor",
        element = {
            kind = "record",
            fields = {
                signed_field("input_tick"),
                { name = "monotonic_ms", kind = "number" },
                { name = "mapping_error_ms", kind = "number" },
            },
        },
    },
})

-- ---------------------------------------------------------------------------
-- Bounded rings
-- ---------------------------------------------------------------------------

---@param limit integer
---@param keep NetDiagnosticsKeep
---@return NetDiagnosticsRing
local function new_ring(limit, keep)
    return { items = {}, limit = limit, head = 1, count = 0, dropped = 0, keep = keep }
end

-- `newest` keeps the tail: what just happened explains a failure that just
-- happened. `oldest` keeps the head and is used where the *first* occurrence is
-- the causal one -- a hash mismatch stream, where later entries are echoes.
---@param ring NetDiagnosticsRing
---@param item any
local function ring_push(ring, item)
    if ring.limit <= 0 then
        ring.dropped = ring.dropped + 1
        return
    end
    if ring.keep == "oldest" then
        if ring.count >= ring.limit then
            ring.dropped = ring.dropped + 1
            return
        end
        ring.count = ring.count + 1
        ring.items[ring.count] = item
        return
    end
    if ring.count >= ring.limit then
        ring.dropped = ring.dropped + 1
    else
        ring.count = ring.count + 1
    end
    ring.items[ring.head] = item
    ring.head = ring.head % ring.limit + 1
end

---@param ring NetDiagnosticsRing
---@return any[]
local function ring_items(ring)
    if ring.keep == "oldest" or ring.count < ring.limit then
        local out = {}
        for index = 1, ring.count do
            out[index] = ring.items[index]
        end
        return out
    end
    local out = {}
    for offset = 0, ring.count - 1 do
        out[offset + 1] = ring.items[(ring.head + offset - 1) % ring.limit + 1]
    end
    return out
end

---@param ring NetDiagnosticsRing
---@return NetDiagnosticsRetention
local function ring_retention(ring)
    return ring.dropped > 0 and schema.TRUNCATED or schema.COMPLETE
end

-- ---------------------------------------------------------------------------
-- Value guards
-- ---------------------------------------------------------------------------

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

---@param value any
---@return boolean
local function is_integer(value)
    return is_finite(value) and value == math.floor(value)
end

-- Every free-text field in this module goes through here. Length is the lesser
-- half of the job: the text originates from a transport, a browser bridge, or a
-- raw DOM exception, so it is checked for network and identity material and
-- replaced whole when it looks like it carries any. See
-- `diagnostics_schema.is_sensitive_text`.
---@param text any
---@return string?
local function bounded_detail(text)
    return schema.redact_free_text(text, net_diagnostics.MAX_DETAIL_BYTES)
end

---@param recorder NetDiagnostics
---@param message string
---@return nil, string
local function reject(recorder, message)
    recorder._rejected = recorder._rejected + 1
    return nil, message
end

-- ---------------------------------------------------------------------------
-- Construction
-- ---------------------------------------------------------------------------

---@param options NetDiagnosticsOptions
---@return NetDiagnostics
function net_diagnostics.new(options)
    assert(type(options) == "table", "net diagnostics needs options")
    local manifest = assert(options.manifest, "net diagnostics needs a manifest")
    local freeze = assert(options.freeze, "net diagnostics needs a freeze")
    local limits = options.limits or net_diagnostics.DEFAULT_LIMITS
    ---@type NetDiagnostics
    local recorder = {
        _session = {
            session_id = manifest.session_id,
            peer_id = assert(options.peer_id, "net diagnostics needs a peer id"),
            role = assert(options.role, "net diagnostics needs a role"),
            match_mode = manifest.match_mode,
            combat_status = manifest.combat_status,
            manifest_id = freeze.manifest_id,
            assignment_id = freeze.assignment_id,
            countdown_id = freeze.countdown_id,
            build_id = manifest.build_id,
            source_id = manifest.source_id,
            content_id = manifest.content_id,
            tuning_id = manifest.tuning_id,
            match_config_id = manifest.match_config_id,
            fixture_id = manifest.fixture_id,
            arena_id = manifest.arena_id,
            combat_rules_id = manifest.combat_rules_id,
            gameplay_ai_policy_id = manifest.gameplay_ai_policy_id,
            network_profile_digest = options.network_profile_digest,
            protocol_version = manifest.protocol_version,
            input_version = manifest.input_version,
            snapshot_version = manifest.snapshot_version,
            tape_version = manifest.tape_version,
            combat_schema_version = manifest.combat_schema_version,
            seed = manifest.seed,
            tick_rate = manifest.tick_rate,
            duration_ticks = manifest.duration_ticks,
            max_goals = manifest.max_goals,
        },
        _limits = {
            checkpoints = limits.checkpoints,
            packets = limits.packets,
            control = limits.control,
            events = limits.events,
            signals = limits.signals,
            mismatches = limits.mismatches,
            anchors = limits.anchors,
            latency_peers = limits.latency_peers,
        },
        _export_opt_in = options.export_opt_in == true,
        _rejected = 0,
        _checkpoints = new_ring(limits.checkpoints, "newest"),
        _packets = new_ring(limits.packets, "newest"),
        _control = new_ring(limits.control, "newest"),
        _events = new_ring(limits.events, "newest"),
        _signals = new_ring(limits.signals, "newest"),
        _mismatches = new_ring(limits.mismatches, "oldest"),
        _anchors = new_ring(limits.anchors, "newest"),
        _latency = {},
        _latency_order = {},
        _simulation = {
            status = "active",
            step_count = 0,
            first_input_tick = freeze.first_input_tick,
            present_input_tick = freeze.first_input_tick,
            confirmed_input_tick = freeze.first_input_tick - 1,
            confirmed_output_tick = freeze.first_input_tick - 1,
            retained_floor_tick = freeze.first_input_tick,
            input_delay_ticks = options.input_delay_ticks or 0,
            hash_interval_ticks = options.hash_interval_ticks or 0,
            max_rollback_ticks = options.max_rollback_ticks or 0,
            rollback_count = 0,
            correction_count = 0,
            predicted_slot_samples = 0,
            resimulated_ticks = 0,
            max_rollback_depth = 0,
            hash_mismatch_streak = 0,
            checkpoint_count = 0,
            owned = {},
            authored = {},
        },
        _delivery = {
            authored = 0,
            published = 0,
            sent = 0,
            arrived = 0,
            applied_rows = 0,
            deferred = 0,
            duplicate = 0,
            rejected = 0,
            reconciliations = 0,
        },
        _events_counts = {
            emitted = 0,
            added = 0,
            revoked = 0,
            replaced = 0,
            unchanged = 0,
            resimulated_tick_count = 0,
        },
        _worst = nil,
        _star = nil,
        _peers = {},
        _pressure = nil,
        _teardown = nil,
        _tick_digests = {},
        _tick_digest_floor = freeze.first_input_tick,
        _pending_apply = {},
        _ordinal = 0,
    }
    return recorder
end

-- Export is a decision, not a default. Collection is always on and always
-- bounded; producing an artifact a human can paste is separately opted into.
---@param recorder NetDiagnostics
function net_diagnostics.opt_in_export(recorder)
    recorder._export_opt_in = true
end

---@param recorder NetDiagnostics
function net_diagnostics.opt_out_export(recorder)
    recorder._export_opt_in = false
end

-- ---------------------------------------------------------------------------
-- Canonical recording
-- ---------------------------------------------------------------------------

local MATCH_EVENT_FIELDS = {
    "kind",
    "x",
    "y",
    "player",
    "save_style",
    "style",
    "outcome",
    "jumping",
    "difficulty",
    "shot_type",
    "keeper_state",
    "keeper_depth",
    "on_target",
}

local COMBAT_EVENT_FIELDS = {
    "kind",
    "tick",
    "family_id",
    "source_index",
    "target_index",
    "source_sequence",
    "result",
    "x",
    "y",
    "interruption_ticks",
    "displacement_px",
}

---@param parts string[]
---@param event table
---@param fields string[]
local function push_event_fields(parts, event, fields)
    for _, name in ipairs(fields) do
        local value = event[name]
        if value == nil then
            parts[#parts + 1] = "-"
        elseif type(value) == "number" then
            parts[#parts + 1] = ("%.17g"):format(value)
        else
            parts[#parts + 1] = tostring(value)
        end
    end
end

-- The identity of one tick's event set. A rollback that replaces a tick and
-- produces the same events is `unchanged`; one that produces different events is
-- `replaced`. Both are resimulation, and only the second is a reconciliation a
-- consumer has to care about.
---@param output RollbackTickOutput
---@return string
local function event_digest(output)
    ---@type string[]
    local parts = { tostring(#output.events) }
    for _, event in ipairs(output.events) do
        push_event_fields(parts, event, MATCH_EVENT_FIELDS)
    end
    local combat = output.combat_events or {}
    parts[#parts + 1] = tostring(#combat)
    for _, event in ipairs(combat) do
        push_event_fields(parts, event, COMBAT_EVENT_FIELDS)
    end
    return schema.tuple_digest("tick_events", parts)
end

---@param recorder NetDiagnostics
---@param floor integer
local function prune_tick_digests(recorder, floor)
    if floor <= recorder._tick_digest_floor then
        return
    end
    for tick = recorder._tick_digest_floor, floor - 1 do
        recorder._tick_digests[tick] = nil
    end
    recorder._tick_digest_floor = floor
end

-- Fold one driver step. `diagnostics` is `match_driver.diagnostics(driver)` and
-- `batch` is the `MatchDriverBatch` the same `advance` returned; nothing here
-- reaches into the driver.
---@param recorder NetDiagnostics
---@param diagnostics MatchDriverDiagnostics
---@param batch MatchDriverBatch
---@return boolean?, string?
function net_diagnostics.record_step(recorder, diagnostics, batch)
    if type(diagnostics) ~= "table" or type(batch) ~= "table" then
        return reject(recorder, "a driver step needs diagnostics and a batch")
    end
    if not is_integer(diagnostics.present_input_tick) or not is_integer(batch.input_tick) then
        return reject(recorder, "a driver step must carry finite integer ticks")
    end
    local simulation = recorder._simulation
    simulation.status = diagnostics.status
    simulation.step_count = simulation.step_count + 1
    simulation.present_input_tick = diagnostics.present_input_tick
    simulation.confirmed_input_tick = diagnostics.confirmed_input_tick
    simulation.confirmed_output_tick = diagnostics.confirmed_output_tick
    simulation.retained_floor_tick = diagnostics.retained_floor_tick
    simulation.rollback_count = diagnostics.rollback_count
    simulation.correction_count = diagnostics.correction_count
    simulation.predicted_slot_samples = diagnostics.predicted_slot_samples
    simulation.max_rollback_depth = diagnostics.max_rollback_depth
    simulation.hash_mismatch_streak = diagnostics.hash_mismatches
    simulation.checkpoint_count = diagnostics.checkpoint_count
    simulation.late_input_tick = diagnostics.late_input_tick
    simulation.control_slot = diagnostics.control_slot
    simulation.owned = {}
    for index, slot in ipairs(diagnostics.owned) do
        simulation.owned[index] = slot
    end
    simulation.authored = {}
    for index, slot in ipairs(diagnostics.authored) do
        simulation.authored[index] = slot
    end
    local terminal = diagnostics.terminal
    if terminal ~= nil then
        simulation.terminal_status = terminal.status
        simulation.terminal_failure = terminal.failure
        simulation.terminal_detail = bounded_detail(terminal.detail)
        simulation.terminal_tick = terminal.tick
    end

    local delivery = recorder._delivery
    delivery.applied_rows = delivery.applied_rows + (batch.applied_rows or 0)
    delivery.reconciliations = delivery.reconciliations + (batch.reconciliations or 0)
    -- `published` is the driver's own count of the packets it handed to the
    -- transport; `sent` is what an observer actually saw leave. They are kept
    -- apart rather than summed: adding them would double-count whenever a tap is
    -- installed, and a disagreement between them is itself a finding.
    delivery.published = delivery.published + (batch.sent_packets or 0)

    -- Resimulation and event reconciliation are read off the outputs the batch
    -- already carries: a rollback re-emits every corrected tick before this
    -- step's own output, so a tick seen twice is a resimulated tick by
    -- definition rather than by a counter the driver would have to keep.
    local counts = recorder._events_counts
    local resimulated = 0
    local first_corrected = math.huge
    local last_corrected = -math.huge
    ---@type RollbackTickOutput[]
    local outputs = batch.outputs or {}
    for _, output in ipairs(outputs) do
        local tick = output.tick
        local digest = event_digest(output)
        local previous = recorder._tick_digests[tick]
        local event_count = #output.events + #(output.combat_events or {})
        counts.emitted = counts.emitted + event_count
        if previous == nil then
            counts.added = counts.added + 1
        else
            resimulated = resimulated + 1
            first_corrected = math.min(first_corrected, tick)
            last_corrected = math.max(last_corrected, tick)
            if previous == digest then
                counts.unchanged = counts.unchanged + 1
            elseif event_count == 0 then
                counts.revoked = counts.revoked + 1
            else
                counts.replaced = counts.replaced + 1
            end
        end
        recorder._tick_digests[tick] = digest
    end
    if resimulated > 0 then
        simulation.resimulated_ticks = simulation.resimulated_ticks + resimulated
        counts.resimulated_tick_count = counts.resimulated_tick_count + resimulated
        local depth = last_corrected - first_corrected + 1
        if recorder._worst == nil or depth > recorder._worst.depth then
            recorder._worst = {
                causal_tick = first_corrected,
                through_tick = last_corrected,
                depth = depth,
                resimulated_ticks = resimulated,
            }
        end
    end
    prune_tick_digests(recorder, diagnostics.retained_floor_tick)

    local pending = recorder._pending_apply
    recorder._pending_apply = {}
    for _, record in ipairs(pending) do
        record.apply_input_tick = batch.input_tick
    end

    for _, checkpoint in ipairs(batch.checkpoints or {}) do
        net_diagnostics.record_checkpoint(recorder, checkpoint)
    end
    return true
end

---@param recorder NetDiagnostics
---@param checkpoint MatchDriverCheckpoint
---@return boolean?, string?
function net_diagnostics.record_checkpoint(recorder, checkpoint)
    if type(checkpoint) ~= "table" or not is_integer(checkpoint.tick) then
        return reject(recorder, "a checkpoint needs a finite integer tick")
    end
    if type(checkpoint.hash) ~= "string" then
        return reject(recorder, "a checkpoint needs a hash")
    end
    ---@type table<string, InputSlotId>
    local live = {}
    for producer_id, slot in pairs(checkpoint.live or {}) do
        live[producer_id] = slot
    end
    ring_push(
        recorder._checkpoints,
        { tick = checkpoint.tick, hash = checkpoint.hash, live = live }
    )
    return true
end

-- A boundary where a peer's published hash disagreed with ours. The first
-- difference path is optional and is only ever present when the capture holds
-- both snapshots -- in a real session it does not, and claiming otherwise would
-- be the kind of false precision this schema exists to avoid.
---@param recorder NetDiagnostics
---@param mismatch NetDiagnosticsMismatch
---@return boolean?, string?
function net_diagnostics.record_mismatch(recorder, mismatch)
    if type(mismatch) ~= "table" or not is_integer(mismatch.tick) then
        return reject(recorder, "a mismatch needs a finite integer tick")
    end
    ring_push(recorder._mismatches, {
        tick = mismatch.tick,
        peer_id = mismatch.peer_id,
        local_hash = mismatch.local_hash,
        remote_hash = mismatch.remote_hash,
        first_difference_path = mismatch.first_difference_path,
    })
    return true
end

---@param recorder NetDiagnostics
---@param packet NetDiagnosticsPacket
---@return boolean?, string?
function net_diagnostics.record_packet(recorder, packet)
    if type(packet) ~= "table" then
        return reject(recorder, "a packet record must be a table")
    end
    local required = {
        packet.sequence,
        packet.payload_bytes,
        packet.sample_step,
        packet.send_transport_tick,
        packet.authority_input_tick,
    }
    for _, value in ipairs(required) do
        if not is_integer(value) then
            return reject(recorder, "a packet record needs finite integer ticks and counts")
        end
    end
    -- Spelled out rather than iterated: a nil in the middle of an array literal
    -- would silently end an `ipairs` walk and skip the checks behind it.
    if packet.arrival_transport_tick ~= nil and not is_integer(packet.arrival_transport_tick) then
        return reject(recorder, "a packet arrival tick must be a finite integer")
    end
    if packet.apply_input_tick ~= nil and not is_integer(packet.apply_input_tick) then
        return reject(recorder, "a packet apply tick must be a finite integer")
    end
    if packet.row_count ~= nil and not is_integer(packet.row_count) then
        return reject(recorder, "a packet row count must be a finite integer")
    end
    local delivery = recorder._delivery
    local disposition = packet.disposition
    if disposition == "authored" then
        delivery.authored = delivery.authored + 1
    elseif disposition == "sent" then
        delivery.sent = delivery.sent + 1
    elseif disposition == "arrived" then
        delivery.arrived = delivery.arrived + 1
    elseif disposition == "deferred" then
        delivery.deferred = delivery.deferred + 1
    elseif disposition == "duplicate" then
        delivery.duplicate = delivery.duplicate + 1
    elseif disposition == "rejected" then
        delivery.rejected = delivery.rejected + 1
    end
    local record = {
        sender_id = packet.sender_id,
        disposition = disposition,
        sequence = packet.sequence,
        payload_bytes = packet.payload_bytes,
        row_count = packet.row_count,
        sample_step = packet.sample_step,
        send_transport_tick = packet.send_transport_tick,
        authority_input_tick = packet.authority_input_tick,
        arrival_transport_tick = packet.arrival_transport_tick,
        apply_input_tick = packet.apply_input_tick,
    }
    ring_push(recorder._packets, record)
    -- An arrival is applied by the same `advance` that polled it, so the apply
    -- tick is stamped by the next `record_step` rather than guessed here. The
    -- lead it exposes -- authority tick minus apply tick -- is the number a
    -- prediction burst is actually about.
    if disposition == "arrived" and record.apply_input_tick == nil then
        recorder._pending_apply[#recorder._pending_apply + 1] = record
    end
    return true
end

-- One accepted session control message, in the order it was handed back. The
-- driver returns control traffic on the batch rather than consuming it, so this
-- takes the same `TransportPeerMessage` the coordinator will read -- it observes
-- the mail, it does not open it on anyone's behalf.
---@param recorder NetDiagnostics
---@param addressed TransportPeerMessage
---@return boolean?, string?
function net_diagnostics.record_control(recorder, addressed)
    if type(addressed) ~= "table" or type(addressed.message) ~= "table" then
        return reject(recorder, "a control record needs an addressed transport message")
    end
    local message = protocol.decode(addressed.message.payload)
    if message == nil then
        return reject(recorder, "control payload did not decode as a session message")
    end
    recorder._ordinal = recorder._ordinal + 1
    ring_push(recorder._control, {
        ordinal = recorder._ordinal,
        kind = message.kind,
        peer_id = addressed.peer_id,
        sequence = message.sequence,
        message_id = message.message_id,
    })
    return true
end

-- ---------------------------------------------------------------------------
-- Runtime observation
-- ---------------------------------------------------------------------------

---@param channel TransportChannelDiagnostics
---@return table
local function copy_channel(channel)
    return {
        state = channel.state,
        outbound_depth = channel.outbound_depth,
        inbound_depth = channel.inbound_depth,
        buffered_amount = channel.buffered_amount,
        sent = channel.sent,
        received = channel.received,
        dropped_outbound = channel.dropped_outbound,
        dropped_inbound = channel.dropped_inbound,
    }
end

-- Transport diagnostics are a *snapshot of counters*, so the latest replaces the
-- previous rather than accumulating: the star already counts cumulatively and
-- adding our own sum on top would double-count.
--
-- Queue **depths** are the exception, and they are why `runtime.pressure` exists.
-- A depth is an instantaneous level, not a counter, and the last snapshot a run
-- takes is almost always a quiescent one -- taken after the final pump, or after
-- teardown drained everything. Reading `peers[*].control.outbound_depth` out of
-- the export therefore answers "how deep was the queue when we stopped looking",
-- which is nearly always zero, and a resource gate written against it cannot
-- fail. `pressure` folds a genuine running peak across every snapshot, alongside
-- the cumulative backpressure and overflow latches that a depth sample can miss
-- entirely: a channel that hit its `bufferedAmount` ceiling and drained again
-- between two observations leaves no depth behind, only a latch.
---@param recorder NetDiagnostics
---@param star TransportStarDiagnostics
---@return boolean?, string?
function net_diagnostics.record_transport(recorder, star)
    if type(star) ~= "table" or type(star.peers) ~= "table" then
        return reject(recorder, "transport diagnostics need a peer list")
    end
    recorder._star = {
        role = star.role,
        state = star.state,
        capacity = star.capacity,
        peer_count = star.peer_count,
        queue_limit = star.queue_limit,
        buffered_amount_limit = star.buffered_amount_limit,
        event_depth = star.event_depth,
        sent = star.sent,
        received = star.received,
        dropped_outbound = star.dropped_outbound,
        dropped_inbound = star.dropped_inbound,
        malformed = star.malformed,
        unsupported_version = star.unsupported_version,
        overflow = star.overflow,
        backpressure = star.backpressure,
        last_error = bounded_detail(star.last_error),
    }
    ---@type table[]
    local peers = {}
    for index, peer in ipairs(star.peers) do
        peers[index] = {
            peer_id = peer.peer_id,
            slot = peer.slot,
            state = peer.state,
            ice_state = peer.ice_state,
            control = copy_channel(peer.control),
            input = copy_channel(peer.input),
            sequence_gaps = peer.sequence_gaps,
            backpressure = peer.backpressure,
            malformed = peer.malformed,
            last_error = bounded_detail(peer.last_error),
        }
    end
    recorder._peers = peers

    local pressure = recorder._pressure
        or {
            samples = 0,
            peak_outbound_depth = 0,
            peak_inbound_depth = 0,
            peak_buffered_amount = 0,
            peak_event_depth = 0,
            peak_peer_count = 0,
            backpressure = 0,
            peer_backpressure = 0,
            overflow = 0,
            dropped_outbound = 0,
            dropped_inbound = 0,
        }
    pressure.samples = pressure.samples + 1
    pressure.peak_event_depth = math.max(pressure.peak_event_depth, star.event_depth or 0)
    pressure.peak_peer_count = math.max(pressure.peak_peer_count, star.peer_count or 0)
    -- The star's own counters are cumulative and monotonic, so a running maximum
    -- is the same value while the star lives and survives a re-initialized one.
    pressure.backpressure = math.max(pressure.backpressure, star.backpressure or 0)
    pressure.overflow = math.max(pressure.overflow, star.overflow or 0)
    pressure.dropped_outbound = math.max(pressure.dropped_outbound, star.dropped_outbound or 0)
    pressure.dropped_inbound = math.max(pressure.dropped_inbound, star.dropped_inbound or 0)
    for _, peer in ipairs(peers) do
        pressure.peer_backpressure = math.max(pressure.peer_backpressure, peer.backpressure or 0)
        for _, channel in ipairs({ peer.control, peer.input }) do
            pressure.peak_outbound_depth =
                math.max(pressure.peak_outbound_depth, channel.outbound_depth)
            pressure.peak_inbound_depth =
                math.max(pressure.peak_inbound_depth, channel.inbound_depth)
            pressure.peak_buffered_amount =
                math.max(pressure.peak_buffered_amount, channel.buffered_amount)
        end
    end
    recorder._pressure = pressure
    return true
end

-- A wall-clock quality sample. Nothing here is ever consulted as simulation
-- truth; it lives in the runtime section and the schema will not let it borrow
-- a tick's vocabulary.
---@param recorder NetDiagnostics
---@param sample NetDiagnosticsRuntimeSample
---@return boolean?, string?
function net_diagnostics.record_runtime_sample(recorder, sample)
    if type(sample) ~= "table" or type(sample.peer_id) ~= "string" then
        return reject(recorder, "a runtime sample needs a peer id")
    end
    if not is_finite(sample.monotonic_ms) or sample.monotonic_ms < 0 then
        return reject(recorder, "a runtime sample needs a finite monotonic observation")
    end
    if sample.rtt_ms ~= nil and (not is_finite(sample.rtt_ms) or sample.rtt_ms < 0) then
        return reject(recorder, "an rtt observation must be finite and non-negative")
    end
    if sample.jitter_ms ~= nil and (not is_finite(sample.jitter_ms) or sample.jitter_ms < 0) then
        return reject(recorder, "a jitter observation must be finite and non-negative")
    end
    local entry = recorder._latency[sample.peer_id]
    if entry == nil then
        if #recorder._latency_order >= recorder._limits.latency_peers then
            return reject(recorder, "runtime samples exceeded the tracked peer cap")
        end
        entry = { peer_id = sample.peer_id, sample_count = 0 }
        recorder._latency[sample.peer_id] = entry
        recorder._latency_order[#recorder._latency_order + 1] = sample.peer_id
    end
    entry.sample_count = entry.sample_count + 1
    entry.monotonic_ms_first = entry.monotonic_ms_first or sample.monotonic_ms
    entry.monotonic_ms_last = sample.monotonic_ms
    if sample.rtt_ms ~= nil then
        entry.rtt_ms_last = sample.rtt_ms
        entry.rtt_ms_min = entry.rtt_ms_min and math.min(entry.rtt_ms_min, sample.rtt_ms)
            or sample.rtt_ms
        entry.rtt_ms_max = entry.rtt_ms_max and math.max(entry.rtt_ms_max, sample.rtt_ms)
            or sample.rtt_ms
    end
    if sample.jitter_ms ~= nil then
        entry.jitter_ms_last = sample.jitter_ms
        entry.jitter_ms_max = entry.jitter_ms_max
                and math.max(entry.jitter_ms_max, sample.jitter_ms)
            or sample.jitter_ms
    end
    return true
end

-- The only place the two clocks are allowed to meet, and it has to say how
-- badly they might disagree.
---@param recorder NetDiagnostics
---@param anchor NetDiagnosticsAnchor
---@return boolean?, string?
function net_diagnostics.record_anchor(recorder, anchor)
    if type(anchor) ~= "table" or not is_integer(anchor.input_tick) then
        return reject(recorder, "an anchor needs a finite integer input tick")
    end
    if not is_finite(anchor.monotonic_ms) or anchor.monotonic_ms < 0 then
        return reject(recorder, "an anchor needs a finite monotonic observation")
    end
    if not is_finite(anchor.mapping_error_ms) or anchor.mapping_error_ms < 0 then
        return reject(recorder, "an anchor must declare a finite non-negative mapping error")
    end
    ring_push(recorder._anchors, {
        input_tick = anchor.input_tick,
        monotonic_ms = anchor.monotonic_ms,
        mapping_error_ms = anchor.mapping_error_ms,
    })
    return true
end

---@param recorder NetDiagnostics
---@param event NetDiagnosticsEvent
---@return boolean?, string?
function net_diagnostics.record_event(recorder, event)
    if type(event) ~= "table" or type(event.kind) ~= "string" then
        return reject(recorder, "a runtime event needs a kind")
    end
    if not is_finite(event.monotonic_ms) or event.monotonic_ms < 0 then
        return reject(recorder, "a runtime event needs a finite monotonic observation")
    end
    recorder._ordinal = recorder._ordinal + 1
    ring_push(recorder._events, {
        ordinal = recorder._ordinal,
        kind = event.kind,
        monotonic_ms = event.monotonic_ms,
        peer_id = event.peer_id,
        channel = event.channel,
        state = event.state,
        code = event.code,
        detail = bounded_detail(event.detail),
    })
    return true
end

-- Signalling blobs carry SDP and ICE credentials, and SDP candidate lines carry
-- raw host and server-reflexive addresses. The blob itself is therefore never
-- stored -- not truncated, not digested, not hashed. What is useful for
-- debugging is that an exchange happened, in which direction, and how large it
-- was, and that is exactly what this keeps.
---@param recorder NetDiagnostics
---@param peer_id string
---@param direction NetDiagnosticsSignalDirection
---@param signal string
---@return boolean?, string?
function net_diagnostics.record_signal(recorder, peer_id, direction, signal)
    if type(peer_id) ~= "string" or type(signal) ~= "string" then
        return reject(recorder, "a signal record needs a peer id and a blob")
    end
    if direction ~= "outbound" and direction ~= "inbound" then
        return reject(recorder, "a signal record needs a direction")
    end
    recorder._ordinal = recorder._ordinal + 1
    ring_push(recorder._signals, {
        ordinal = recorder._ordinal,
        peer_id = peer_id,
        direction = direction,
        byte_length = #signal,
        content = schema.REDACTED,
    })
    return true
end

-- Teardown completeness. `orphaned_peers` are links the star still reports after
-- shutdown was requested; residual queue depth is what never drained.
---@param recorder NetDiagnostics
---@param teardown NetDiagnosticsTeardown
---@return boolean?, string?
function net_diagnostics.record_teardown(recorder, teardown)
    if type(teardown) ~= "table" then
        return reject(recorder, "a teardown record must be a table")
    end
    if not is_integer(teardown.closed_peers) then
        return reject(recorder, "teardown needs a finite closed peer count")
    end
    ---@type string[]
    local orphaned = {}
    for index, peer_id in ipairs(teardown.orphaned_peers or {}) do
        orphaned[index] = peer_id
    end
    local residual_outbound = teardown.residual_outbound or 0
    local residual_inbound = teardown.residual_inbound or 0
    recorder._teardown = {
        requested = teardown.requested == true,
        complete = #orphaned == 0 and residual_outbound == 0 and residual_inbound == 0,
        closed_peers = teardown.closed_peers,
        residual_outbound = residual_outbound,
        residual_inbound = residual_inbound,
        orphaned_peers = orphaned,
    }
    return true
end

-- ---------------------------------------------------------------------------
-- Reading back
-- ---------------------------------------------------------------------------

---@param recorder NetDiagnostics
---@return table
local function copy_session(recorder)
    local out = {}
    for key, value in pairs(recorder._session) do
        out[key] = value
    end
    return out
end

---@param recorder NetDiagnostics
---@return table[]
local function latency_rows(recorder)
    ---@type string[]
    local ids = {}
    for index, peer_id in ipairs(recorder._latency_order) do
        ids[index] = peer_id
    end
    -- Sorted, never insertion-ordered: an export whose row order depends on
    -- which peer happened to answer first is not stable across runs.
    table.sort(ids)
    ---@type table[]
    local rows = {}
    for index, peer_id in ipairs(ids) do
        local entry = recorder._latency[peer_id]
        rows[index] = {
            peer_id = entry.peer_id,
            sample_count = entry.sample_count,
            rtt_ms_min = entry.rtt_ms_min,
            rtt_ms_max = entry.rtt_ms_max,
            rtt_ms_last = entry.rtt_ms_last,
            jitter_ms_max = entry.jitter_ms_max,
            jitter_ms_last = entry.jitter_ms_last,
            monotonic_ms_first = entry.monotonic_ms_first,
            monotonic_ms_last = entry.monotonic_ms_last,
        }
    end
    return rows
end

---@param source table
---@return table
local function shallow_copy(source)
    local out = {}
    for key, value in pairs(source) do
        if type(value) == "table" then
            local nested = {}
            for nested_key, nested_value in pairs(value) do
                nested[nested_key] = nested_value
            end
            out[key] = nested
        else
            out[key] = value
        end
    end
    return out
end

-- The full artifact. Returns `nil, err` unless export was opted into: this is
-- the seam between "we are holding diagnostics" and "we are producing something
-- that can leave the machine".
---@param recorder NetDiagnostics
---@return table?, string?
function net_diagnostics.export(recorder)
    if not recorder._export_opt_in then
        return nil, "diagnostic export was not opted into"
    end
    local artifact = {
        schema_version = net_diagnostics.SCHEMA_VERSION,
        digest_algorithm = schema.DIGEST,
        session = copy_session(recorder),
        collection = {
            export_opt_in = true,
            rejected_values = recorder._rejected,
            caps = {
                checkpoints = recorder._limits.checkpoints,
                packets = recorder._limits.packets,
                control = recorder._limits.control,
                events = recorder._limits.events,
                signals = recorder._limits.signals,
                mismatches = recorder._limits.mismatches,
                anchors = recorder._limits.anchors,
                latency_peers = recorder._limits.latency_peers,
            },
            dropped = {
                checkpoints = recorder._checkpoints.dropped,
                packets = recorder._packets.dropped,
                control = recorder._control.dropped,
                events = recorder._events.dropped,
                signals = recorder._signals.dropped,
                mismatches = recorder._mismatches.dropped,
                anchors = recorder._anchors.dropped,
            },
            retention = {
                checkpoints = ring_retention(recorder._checkpoints),
                packets = ring_retention(recorder._packets),
                control = ring_retention(recorder._control),
                events = ring_retention(recorder._events),
                signals = ring_retention(recorder._signals),
                mismatches = ring_retention(recorder._mismatches),
                anchors = ring_retention(recorder._anchors),
            },
        },
        canonical = {
            simulation = shallow_copy(recorder._simulation),
            worst_correction = recorder._worst and shallow_copy(recorder._worst) or nil,
            events = shallow_copy(recorder._events_counts),
            delivery = shallow_copy(recorder._delivery),
            checkpoints = ring_items(recorder._checkpoints),
            mismatches = ring_items(recorder._mismatches),
            control = ring_items(recorder._control),
        },
        runtime = {
            star = recorder._star and shallow_copy(recorder._star) or nil,
            peers = recorder._peers,
            pressure = recorder._pressure and shallow_copy(recorder._pressure) or nil,
            latency = latency_rows(recorder),
            events = ring_items(recorder._events),
            signals = ring_items(recorder._signals),
            teardown = recorder._teardown and shallow_copy(recorder._teardown) or nil,
        },
        anchors = ring_items(recorder._anchors),
    }
    artifact.canonical.delivery.packets = ring_items(recorder._packets)
    local ok, err = schema.validate(net_diagnostics.EXPORT, artifact)
    if not ok then
        return nil, err
    end
    return artifact
end

---@param recorder NetDiagnostics
---@return string?, string?
function net_diagnostics.encode(recorder)
    local artifact, err = net_diagnostics.export(recorder)
    if artifact == nil then
        return nil, err
    end
    return schema.encode(net_diagnostics.EXPORT, artifact)
end

---@param recorder NetDiagnostics
---@return string?, string?
function net_diagnostics.digest(recorder)
    local artifact, err = net_diagnostics.export(recorder)
    if artifact == nil then
        return nil, err
    end
    return schema.digest(net_diagnostics.EXPORT, artifact)
end

-- The deterministic half of the artifact: identity plus canonical evidence, with
-- runtime observation and clock anchors projected out.
--
-- This exists because the two halves deserve different claims. Two runs of the
-- same fixture over the same delivery schedule produce byte-identical canonical
-- evidence, and that is worth pinning. The same two runs produce *different*
-- RTT, jitter, and monotonic timestamps on any real machine, and pinning those
-- would either be a lie or a test that only passes because the harness froze the
-- clock. So: byte-identity here, invariants over there.
net_diagnostics.CANONICAL_PROJECTION =
    schema.project(net_diagnostics.EXPORT, { identity = true, canonical = true })

---@param recorder NetDiagnostics
---@return string?, string?
function net_diagnostics.canonical_bytes(recorder)
    local artifact, err = net_diagnostics.export(recorder)
    if artifact == nil then
        return nil, err
    end
    return schema.encode(net_diagnostics.CANONICAL_PROJECTION, {
        schema_version = artifact.schema_version,
        digest_algorithm = artifact.digest_algorithm,
        session = artifact.session,
        collection = artifact.collection,
        canonical = artifact.canonical,
    })
end

---@param recorder NetDiagnostics
---@return string?, string?
function net_diagnostics.canonical_digest(recorder)
    local bytes, err = net_diagnostics.canonical_bytes(recorder)
    if bytes == nil then
        return nil, err
    end
    return schema.tuple_digest("net_diagnostics_canonical", { bytes })
end

-- A short human-readable read-out for an in-game debug overlay. It reads only
-- fields the export already carries, it never needs the export opt-in (nothing
-- leaves the machine), and it keeps the two clocks in separate lines with
-- separate units so a screenshot cannot blur them either.
---@param recorder NetDiagnostics
---@return string[]
function net_diagnostics.summary(recorder)
    local simulation = recorder._simulation
    local delivery = recorder._delivery
    ---@type string[]
    local lines = {
        ("session %s  peer %s  %s  %s"):format(
            recorder._session.manifest_id,
            recorder._session.peer_id,
            recorder._session.role,
            recorder._session.match_mode
        ),
        ("sim  tick %d  confirmed %d  floor %d  delay %d"):format(
            simulation.present_input_tick,
            simulation.confirmed_output_tick,
            simulation.retained_floor_tick,
            simulation.input_delay_ticks
        ),
        ("sim  rollbacks %d  worst depth %d  resimulated %d  corrections %d"):format(
            simulation.rollback_count,
            simulation.max_rollback_depth,
            simulation.resimulated_ticks,
            simulation.correction_count
        ),
        ("sim  checkpoints %d  mismatch streak %d  status %s"):format(
            simulation.checkpoint_count,
            simulation.hash_mismatch_streak,
            simulation.status
        ),
        ("wire  sent %d  arrived %d  applied rows %d  reconciliations %d"):format(
            delivery.sent,
            delivery.arrived,
            delivery.applied_rows,
            delivery.reconciliations
        ),
    }
    local star = recorder._star
    if star ~= nil then
        lines[#lines + 1] = ("wire  %s  peers %d  dropped %d/%d  backpressure %d"):format(
            star.state,
            star.peer_count,
            star.dropped_outbound,
            star.dropped_inbound,
            star.backpressure
        )
    end
    for _, row in ipairs(latency_rows(recorder)) do
        lines[#lines + 1] = ("clock  %s  rtt %s ms  jitter %s ms  (observed, not simulation)"):format(
            row.peer_id,
            row.rtt_ms_last and ("%.1f"):format(row.rtt_ms_last) or "n/a",
            row.jitter_ms_last and ("%.1f"):format(row.jitter_ms_last) or "n/a"
        )
    end
    local truncated = {}
    for _, entry in ipairs({
        { "checkpoints", recorder._checkpoints },
        { "packets", recorder._packets },
        { "control", recorder._control },
        { "events", recorder._events },
        { "signals", recorder._signals },
        { "mismatches", recorder._mismatches },
        { "anchors", recorder._anchors },
    }) do
        if entry[2].dropped > 0 then
            truncated[#truncated + 1] = ("%s -%d"):format(entry[1], entry[2].dropped)
        end
    end
    if #truncated > 0 then
        lines[#lines + 1] = "truncated  " .. table.concat(truncated, "  ")
    end
    if recorder._rejected > 0 then
        lines[#lines + 1] = ("rejected  %d invalid values"):format(recorder._rejected)
    end
    return lines
end

return net_diagnostics
