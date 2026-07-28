local fnv1a64 = require("core.fnv1a64")
local protocol = require("game.online.protocol")
local fixture = require("game.online.protocol_fixture")

---@class SessionProtocolGolden
---@field manifest_id string
---@field transcript_id string
---@field complete_kind SessionMessageKind
---@field complete_wire string
---@field wire_digests table<SessionMessageKind, string>

---@class SessionProtocolConformanceReport
---@field manifest_id string
---@field transcript_id string
---@field message_count integer

---@class SessionProtocolConformanceModule
local conformance = {}

---@type SessionProtocolGolden
conformance.GOLDEN = {
    manifest_id = "ea39ebe78423e0a0",
    transcript_id = "48162b614e650bd2",
    complete_kind = "manifest_accept",
    complete_wire = "GCOP;1;t7:s4:bodyt1:s11:manifest_ids16:ea39ebe78423e0a0s4:kinds15:"
        .. "manifest_accepts10:message_ids32:GCMI;1;13:session_alpha4:host1:2s7:peer_ids4:"
        .. "hosts8:sequencei1:2s10:session_ids13:session_alphas7:versioni1:1",
    wire_digests = {
        handshake = "839047279d13a02d",
        manifest_proposal = "6d3818a72c39032b",
        manifest_accept = "a292fa4a99393456",
        peer_assignment = "fa48b31571dfe543",
        slot_assignment = "aae4def8b6d64d92",
        ready = "d1a83352b259e76b",
        pair_preference = "a8f84f2616677df5",
        pair_preference_result = "df611e121f1ce5ba",
        countdown = "3c40b7699ce5861e",
        start = "d4fff9c7d4ba31b7",
        match_phase = "1671940891b78f1f",
        hash_report = "4405d9323b1e5b0f",
        result_ack = "5f466e6740c6d4cf",
        abort = "9db9c05e9728c4c1",
        disconnect = "a7599b154bb86cec",
    },
}

---@return SessionProtocolConformanceReport
function conformance.verify()
    local manifest_id = protocol.manifest_id(fixture.manifest())
    assert(manifest_id == conformance.GOLDEN.manifest_id, "protocol manifest golden changed")

    local messages = fixture.messages()
    local seen = {}
    for _, message in ipairs(messages) do
        assert(not seen[message.kind], "protocol fixture repeats a message kind")
        seen[message.kind] = true
        local expected = assert(
            conformance.GOLDEN.wire_digests[message.kind],
            "protocol fixture has an unpinned message kind"
        )
        local wire = assert(protocol.encode(message))
        local actual = fnv1a64.hash(wire)
        assert(
            actual == expected,
            ("%s protocol wire golden changed: expected %s, got %s"):format(
                message.kind,
                expected,
                actual
            )
        )
        if message.kind == conformance.GOLDEN.complete_kind then
            assert(
                wire == conformance.GOLDEN.complete_wire,
                "complete protocol wire golden changed"
            )
        end
    end
    local golden_count = 0
    for kind in pairs(conformance.GOLDEN.wire_digests) do
        assert(seen[kind], "protocol golden has no conformance message for " .. kind)
        golden_count = golden_count + 1
    end
    assert(golden_count == #messages, "protocol message/golden count differs")

    local decoded = assert(protocol.decode(conformance.GOLDEN.complete_wire))
    assert(decoded.kind == conformance.GOLDEN.complete_kind, "complete wire kind changed")
    assert(
        protocol.encode(decoded) == conformance.GOLDEN.complete_wire,
        "complete wire no longer decodes canonically"
    )

    local transcript_id = protocol.transcript_id(messages)
    assert(
        transcript_id == conformance.GOLDEN.transcript_id,
        ("protocol transcript golden changed: expected %s, got %s"):format(
            conformance.GOLDEN.transcript_id,
            transcript_id
        )
    )
    return {
        manifest_id = manifest_id,
        transcript_id = transcript_id,
        message_count = #messages,
    }
end

---@param report SessionProtocolConformanceReport
---@return string
function conformance.marker(report)
    return table.concat({
        "GC_PROTOCOL",
        "golden",
        "schema=1",
        "manifest_id=" .. report.manifest_id,
        "transcript_id=" .. report.transcript_id,
        "messages=" .. tostring(report.message_count),
    }, "|")
end

return conformance
