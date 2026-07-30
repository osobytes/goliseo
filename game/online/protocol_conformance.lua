local fnv1a64 = require("core.fnv1a64")
local protocol = require("game.online.protocol")
local fixture = require("game.online.protocol_fixture")

---@class SessionProtocolGolden
---@field vocabulary_id string
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
    -- The vocabulary digest rides in `build_id`, so a kind, a body field, or a
    -- phase rule cannot change without changing which builds will play each
    -- other. Pinning it here makes that consequence part of the diff instead of
    -- something a reviewer has to notice. It is deliberately absent from
    -- `conformance.marker`: the browser evidence parser compares the marker's
    -- field set exactly, and this pin has no browser-specific meaning.
    vocabulary_id = "e13e3647001a0a7e",
    -- Repinned by #268: the fixture manifest's `max_goals` moved 5 -> 99 (no goal
    -- limit), so `manifest_id` moves and every wire that embeds it moves with it.
    -- The seven digests below that do not carry a manifest id are unchanged, which
    -- is the evidence nothing else moved.
    manifest_id = "eb59f113614c35b2",
    transcript_id = "653cba3b32c62ce9",
    complete_kind = "manifest_accept",
    complete_wire = "GCOP;1;t7:s4:bodyt1:s11:manifest_ids16:eb59f113614c35b2s4:kinds15:"
        .. "manifest_accepts10:message_ids32:GCMI;1;13:session_alpha4:host1:2s7:peer_ids4:"
        .. "hosts8:sequencei1:2s10:session_ids13:session_alphas7:versioni1:1",
    wire_digests = {
        handshake = "2722abf054051350",
        manifest_proposal = "171c298f6eeb77e1",
        manifest_accept = "363c57d949586608",
        peer_assignment = "fa48b31571dfe543",
        slot_assignment = "db929e7cd34eab60",
        ready = "a89d1e1747464a51",
        pair_preference = "44cbe6dc14b4af77",
        pair_preference_result = "e9bc40f5818037f4",
        countdown = "c26f26e05519c2c8",
        start = "3fdf9b6a442b6755",
        match_phase = "1671940891b78f1f",
        hash_report = "4405d9323b1e5b0f",
        result_ack = "5f466e6740c6d4cf",
        abort = "9db9c05e9728c4c1",
        disconnect = "a7599b154bb86cec",
    },
}

---@return SessionProtocolConformanceReport
function conformance.verify()
    local vocabulary_id = protocol.vocabulary_id()
    assert(
        vocabulary_id == conformance.GOLDEN.vocabulary_id,
        ("protocol vocabulary golden changed: expected %s, got %s"):format(
            conformance.GOLDEN.vocabulary_id,
            vocabulary_id
        )
    )

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
