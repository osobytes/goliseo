local fnv1a64 = require("core.fnv1a64")
local input_protocol = require("game.online.input_protocol")
local fixture = require("game.online.input_protocol_fixture")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")

-- The wire literals below embed `manifest-id`, a hash over the session manifest,
-- which carries the snapshot and combat schema versions. Bumping either version
-- invalidates every literal here, so the versions they were generated against are
-- pinned too and checked first: a stale golden then reports which version it was
-- built for instead of an opaque byte mismatch.
---@class InputProtocolGolden
---@field snapshot_version integer
---@field combat_version integer
---@field guest_wire string
---@field guest_digest string
---@field host_wire string
---@field host_digest string
---@field maximal_wire_bytes integer
---@field maximal_wire_margin integer

---@class InputProtocolConformanceReport
---@field guest_digest string
---@field host_digest string
---@field maximal_wire_bytes integer
---@field maximal_wire_margin integer
---@field vector_count integer

---@class InputProtocolConformanceModule
local conformance = {}

---@type InputProtocolGolden
conformance.GOLDEN = {
    snapshot_version = 11,
    combat_version = 12,
    guest_wire = "GCIP;1;G;2;ea39ebe78423e0a0;7;f6f6f9dbe278dccb;12;0;4;3;7;"
        .. "AAAAAAJ/fwAAAAAAAQIA/gUJAAAAAgJ/f4AgAAAAAwJ/f4AAAAAABAJ/fwBAAAAABQJ/f38f"
        .. "AAAABgL+ABIW",
    guest_digest = "cc1e2be6d59472ee",
    host_wire = "GCIP;1;H;2;ea39ebe78423e0a0;13;65c65955c65cc80a;15;0;5;3;16;"
        .. "AAAABQGwTgAAAAAABQKvTwEBAAAABQOuUAICAAAABQStUQMDAAAABQWsUgQEAAAABQarUwUF"
        .. "AAAABQeqVAYGAAAABQipVYAgAAAABgG6RAAAAAAABgK5RQEBAAAABgO4RgICAAAABgS3RwMD"
        .. "AAAABgW2SAQEAAAABga1SQUFAAAABge0SgYGAAAABgizS4Ag",
    host_digest = "9f5892e524d62b33",
    maximal_wire_bytes = 958,
    maximal_wire_margin = 66,
}

---@return InputProtocolConformanceReport
function conformance.verify()
    local versions = ("snapshot %d / combat %d"):format(
        match_snapshot.VERSION,
        match_snapshot.COMBAT_VERSION
    )
    assert(
        match_snapshot.VERSION == conformance.GOLDEN.snapshot_version
            and match_snapshot.COMBAT_VERSION == conformance.GOLDEN.combat_version,
        ("input packet goldens are stale for %s; pinned for snapshot %d / combat %d"):format(
            versions,
            conformance.GOLDEN.snapshot_version,
            conformance.GOLDEN.combat_version
        )
    )
    local guest = fixture.guest()
    local guest_wire = assert(input_protocol.encode(guest))
    assert(guest_wire == conformance.GOLDEN.guest_wire, "guest input packet golden changed")
    local guest_digest = fnv1a64.hash(guest_wire)
    assert(guest_digest == conformance.GOLDEN.guest_digest, "guest input packet digest changed")
    local decoded_guest = assert(input_protocol.decode(conformance.GOLDEN.guest_wire, {
        session_id = guest.session_id,
        manifest_id = guest.manifest_id,
        sender_id = guest.sender_id,
    }))
    assert(
        input_protocol.encode(decoded_guest) == conformance.GOLDEN.guest_wire,
        "guest input literal no longer round-trips"
    )

    local host = fixture.host()
    local host_wire = assert(input_protocol.encode(host))
    assert(host_wire == conformance.GOLDEN.host_wire, "host input packet golden changed")
    local host_digest = fnv1a64.hash(host_wire)
    assert(host_digest == conformance.GOLDEN.host_digest, "host input packet digest changed")
    local decoded_host = assert(input_protocol.decode(conformance.GOLDEN.host_wire, {
        session_id = host.session_id,
        manifest_id = host.manifest_id,
        sender_id = host.sender_id,
    }))
    assert(
        input_protocol.encode(decoded_host) == conformance.GOLDEN.host_wire,
        "host input literal no longer round-trips"
    )

    local maximal = fixture.maximal()
    local maximal_wire = assert(input_protocol.encode(maximal))
    assert(#maximal.rows == input_protocol.MAX_HOST_ROWS, "maximal input row count changed")
    assert(
        #maximal_wire == conformance.GOLDEN.maximal_wire_bytes,
        "maximal input packet size changed"
    )
    assert(
        #maximal_wire <= input_protocol.MAX_WIRE_BYTES,
        "maximal input packet exceeds its transport bound"
    )
    -- The margin is pinned, not merely non-negative. #243 sized `MAX_HOST_ROWS`
    -- against this budget, and a bound sized to the exact edge is a bound the
    -- next additive header field silently breaks. This says how much slack the
    -- sizing deliberately left, and fails with the number if it is spent.
    local margin = input_protocol.MAX_WIRE_BYTES - #maximal_wire
    assert(
        margin >= input_protocol.MIN_WIRE_MARGIN_BYTES,
        ("maximal input packet leaves %d spare bytes, below the declared %d-byte margin"):format(
            margin,
            input_protocol.MIN_WIRE_MARGIN_BYTES
        )
    )
    assert(
        margin == conformance.GOLDEN.maximal_wire_margin,
        "maximal input packet wire margin changed"
    )
    assert(
        maximal.input_version == input_frame.VERSION,
        "input conformance fixture uses the wrong sample version"
    )
    return {
        guest_digest = guest_digest,
        host_digest = host_digest,
        maximal_wire_bytes = #maximal_wire,
        maximal_wire_margin = margin,
        vector_count = 2,
    }
end

---@param report InputProtocolConformanceReport
---@return string
function conformance.marker(report)
    return table.concat({
        "GC_INPUT_PROTOCOL",
        "golden",
        "schema=" .. tostring(input_protocol.VERSION),
        "input=" .. tostring(input_frame.VERSION),
        "history=" .. tostring(input_protocol.HISTORY_ROWS),
        "delay=" .. tostring(input_protocol.FAIRNESS_DELAY_TICKS),
        "vectors=" .. tostring(report.vector_count),
        "guest=" .. report.guest_digest,
        "host=" .. report.host_digest,
        "host_rows=" .. tostring(input_protocol.MAX_HOST_ROWS),
        "max_bytes=" .. tostring(report.maximal_wire_bytes),
        "margin=" .. tostring(report.maximal_wire_margin),
    }, "|")
end

return conformance
