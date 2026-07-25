local fnv1a64 = require("core.fnv1a64")
local conformance = require("game.online.protocol_conformance")
local fixture = require("game.online.protocol_fixture")
local protocol = require("game.online.protocol")
local t = require("spec.support.runner")

---@param value any
---@return any
local function deep_copy(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, item in pairs(value) do
        result[key] = deep_copy(item)
    end
    return result
end

t.describe("OMP-3 online protocol", function()
    t.it("pins the accepted input, snapshot, tape, and combat schema versions", function()
        t.eq(protocol.CURRENT_VERSIONS.protocol, 1)
        t.eq(protocol.CURRENT_VERSIONS.input, 2)
        t.eq(protocol.CURRENT_VERSIONS.snapshot, 9)
        t.eq(protocol.CURRENT_VERSIONS.tape, 2)
        t.eq(protocol.CURRENT_VERSIONS.combat, 1)
    end)

    t.it("matches literal wire, manifest, transcript, and per-kind golden evidence", function()
        local report = conformance.verify()
        t.eq(report.manifest_id, "27c9d39785b1aaaf")
        t.eq(report.transcript_id, "c9a74fe23c6c46bc")
        t.eq(report.message_count, 13)
        t.eq(fnv1a64.hash(conformance.GOLDEN.complete_wire), "ae5be4cbc3ee95a7")
        t.eq(
            conformance.marker(report),
            "GC_PROTOCOL|golden|schema=1|manifest_id=27c9d39785b1aaaf"
                .. "|transcript_id=c9a74fe23c6c46bc|messages=13"
        )
    end)

    t.it("constructs owned runtime and deterministic manifest records", function()
        local runtime_source = fixture.runtime()
        local runtime = assert(protocol.new_runtime(runtime_source))
        runtime_source.capabilities[1] = "mutated"
        t.eq(runtime.capabilities[1], "combat_feedback.v1")

        local manifest_source = fixture.manifest()
        local manifest = assert(protocol.new_manifest(manifest_source))
        manifest_source.slots[1].player_id = "mutated"
        t.eq(manifest.slots[1].player_id, "zyro_vex")
    end)

    t.it("round-trips every control message through one canonical bounded codec", function()
        local messages = fixture.messages()
        local expected_kinds = {
            "handshake",
            "manifest_proposal",
            "manifest_accept",
            "peer_assignment",
            "slot_assignment",
            "ready",
            "countdown",
            "start",
            "match_phase",
            "hash_report",
            "result_ack",
            "abort",
            "disconnect",
        }
        t.eq(#messages, #expected_kinds)
        for index, message in ipairs(messages) do
            t.eq(message.kind, expected_kinds[index])
            local wire = assert(protocol.encode(message))
            t.is_true(#wire <= protocol.MAX_WIRE_BYTES)
            local decoded = assert(protocol.decode(wire))
            t.eq(assert(protocol.encode(decoded)), wire)
            local sequence = tostring(index - 1)
            t.eq(
                decoded.message_id,
                "GCMI;1;13:session_alpha4:host" .. tostring(#sequence) .. ":" .. sequence
            )
        end
    end)

    t.it("uses injective transcript ids through exact component maxima", function()
        local session_id = string.rep("s", protocol.MAX_SESSION_ID_BYTES)
        local peer_id = string.rep("p", protocol.MAX_PEER_ID_BYTES)
        local message_id = assert(protocol.message_id(session_id, peer_id, protocol.MAX_SEQUENCE))
        t.eq(#message_id, protocol.MAX_MESSAGE_ID_BYTES)

        local manifest_id = protocol.manifest_id(fixture.manifest())
        local message = assert(protocol.new("ready", session_id, peer_id, protocol.MAX_SEQUENCE, {
            manifest_id = manifest_id,
            ready = true,
        }))
        t.eq(message.message_id, message_id)
        t.eq(assert(protocol.decode(assert(protocol.encode(message)))).message_id, message_id)

        local first = assert(protocol.message_id("a.b", "c", 1))
        local second = assert(protocol.message_id("a", "b.c", 1))
        t.is_true(first ~= second)
        t.eq(first, "GCMI;1;3:a.b1:c1:1")
        t.eq(second, "GCMI;1;1:a3:b.c1:1")

        local too_long, _, code =
            protocol.message_id(string.rep("s", protocol.MAX_SESSION_ID_BYTES + 1), "peer", 0)
        t.eq(too_long, nil)
        t.eq(code, "malformed")
    end)

    t.it("rejects malformed, oversized, unsupported, unknown, and noncanonical wires", function()
        local message = fixture.messages()[1]
        local wire = assert(protocol.encode(message))

        local value, _, code = protocol.decode("not-a-protocol-message")
        t.eq(value, nil)
        t.eq(code, "malformed")

        value, _, code = protocol.decode(string.rep("x", protocol.MAX_WIRE_BYTES + 1))
        t.eq(value, nil)
        t.eq(code, "wire_too_large")

        value, _, code = protocol.decode(wire:gsub("^GCOP;1;", "GCOP;2;"))
        t.eq(value, nil)
        t.eq(code, "unsupported_version")

        value, _, code = protocol.decode(
            assert(protocol.encode(message)):gsub("s9:handshake", "s14:future_message")
        )
        t.eq(value, nil)
        t.eq(code, "unknown_message")

        value, _, code = protocol.decode(wire:gsub("t7:", "t07:", 1))
        t.eq(value, nil)
        t.eq(code, "malformed")

        local extra = deep_copy(message)
        extra.body.secret = "do-not-send"
        value, _, code =
            protocol.new(extra.kind, extra.session_id, extra.peer_id, extra.sequence, extra.body)
        t.eq(value, nil)
        t.eq(code, "malformed")
    end)

    t.it("validates canonical teams, slots, protected keepers, and bot fills", function()
        local manifest = fixture.manifest()
        t.is_true(assert(protocol.validate_manifest(manifest)))

        local keeper = deep_copy(manifest)
        keeper.teams[1].roster[1].loadout_id = "loadout_illegal"
        keeper.teams[1].roster[1].family_id = "unarmed"
        local ok, _, code = protocol.validate_manifest(keeper)
        t.eq(ok, nil)
        t.eq(code, "malformed")

        local reordered = deep_copy(manifest)
        reordered.slots[1], reordered.slots[2] = reordered.slots[2], reordered.slots[1]
        ok, _, code = protocol.validate_manifest(reordered)
        t.eq(ok, nil)
        t.eq(code, "malformed")

        local assignments = fixture.assignments()
        local message = assert(protocol.new("slot_assignment", manifest.session_id, "host", 1, {
            manifest_id = protocol.manifest_id(manifest),
            assignments = assignments,
        }))
        t.is_true(assert(protocol.validate(message)))
        t.is_true(assert(protocol.validate_assignment_manifest(manifest, assignments)))
        t.eq(assignments[7].producer_kind, "bot")
        t.eq(assignments[7].bot_seed, 21007)

        local wrong_player = deep_copy(assignments)
        wrong_player[1].player_id = manifest.slots[2].player_id
        ok, _, code = protocol.validate_assignment_manifest(manifest, wrong_player)
        t.eq(ok, nil)
        t.eq(code, "identity_mismatch")

        assignments[7].bot_seed = nil
        local invalid_message
        invalid_message, _, code = protocol.new("slot_assignment", manifest.session_id, "host", 2, {
            manifest_id = protocol.manifest_id(manifest),
            assignments = assignments,
        })
        t.eq(invalid_message, nil)
        t.eq(code, "malformed")
    end)

    t.it("names the first deterministic identity mismatch before countdown", function()
        local expected = fixture.manifest()
        local actual = deep_copy(expected)
        actual.teams[2].roster[3].family_id = "ranged"
        local ok, _, code, path = protocol.compare_manifest(expected, actual)
        t.eq(ok, nil)
        t.eq(code, "identity_mismatch")
        t.eq(path, "manifest.teams.2.roster.3.family_id")

        actual = deep_copy(expected)
        actual.presentation_id = "not-a-manifest-field"
        local valid, _, valid_code = protocol.validate_manifest(actual)
        t.eq(valid, nil)
        t.eq(valid_code, "malformed")

        actual = deep_copy(expected)
        actual.build_id = "build.other"
        ok, _, code, path = protocol.compare_manifest(expected, actual)
        t.eq(ok, nil)
        t.eq(code, "identity_mismatch")
        t.eq(path, "manifest.build_id")

        actual = deep_copy(expected)
        actual.slots[1].player_id, actual.slots[2].player_id =
            actual.slots[2].player_id, actual.slots[1].player_id
        ok, _, code, path = protocol.compare_manifest(expected, actual)
        t.eq(ok, nil)
        t.eq(code, "identity_mismatch")
        t.eq(path, "manifest.slots.1.player_id")
    end)

    t.it(
        "compares runtime and presentation compatibility outside deterministic identity",
        function()
            local manifest = fixture.manifest()
            local manifest_id = protocol.manifest_id(manifest)
            local expected = fixture.runtime()
            local actual = deep_copy(expected)
            actual.presentation_id = "presentation.other"

            local ok, _, code, path = protocol.compare_runtime(expected, actual)
            t.eq(ok, nil)
            t.eq(code, "runtime_mismatch")
            t.eq(path, "runtime.presentation_id")
            t.eq(protocol.manifest_id(manifest), manifest_id)

            actual = deep_copy(expected)
            actual.capabilities[2], actual.capabilities[3] =
                actual.capabilities[3], actual.capabilities[2]
            local valid, _, valid_code = protocol.validate_runtime(actual)
            t.eq(valid, nil)
            t.eq(valid_code, "malformed")

            actual = deep_copy(expected)
            actual.capabilities[3] = "voice.v1"
            ok, _, code, path = protocol.compare_runtime(expected, actual)
            t.eq(ok, nil)
            t.eq(code, "runtime_mismatch")
            t.eq(path, "runtime.capabilities.3")
        end
    )

    t.it("rejects every current schema version mismatch and bounded-field class", function()
        local version_fields = {
            "version",
            "protocol_version",
            "input_version",
            "snapshot_version",
            "tape_version",
            "combat_schema_version",
        }
        for _, field in ipairs(version_fields) do
            local manifest = fixture.manifest()
            manifest[field] = manifest[field] + 1
            local value, _, code = protocol.validate_manifest(manifest)
            t.eq(value, nil, field)
            t.eq(code, "unsupported_version", field)
        end

        local runtime = fixture.runtime()
        runtime.version = runtime.version + 1
        local value, _, code = protocol.validate_runtime(runtime)
        t.eq(value, nil)
        t.eq(code, "unsupported_version")

        local manifest = fixture.manifest()
        manifest.build_id = string.rep("x", protocol.MAX_ID_BYTES + 1)
        value, _, code = protocol.validate_manifest(manifest)
        t.eq(value, nil)
        t.eq(code, "malformed")

        manifest = fixture.manifest()
        manifest.seed = protocol.MAX_SEED + 1
        value, _, code = protocol.validate_manifest(manifest)
        t.eq(value, nil)
        t.eq(code, "malformed")

        manifest = fixture.manifest()
        manifest.duration_ticks = protocol.MAX_DURATION_TICKS + 1
        value, _, code = protocol.validate_manifest(manifest)
        t.eq(value, nil)
        t.eq(code, "malformed")

        runtime = fixture.runtime()
        for index = #runtime.capabilities + 1, protocol.MAX_CAPABILITIES + 1 do
            runtime.capabilities[index] = ("zz_capability_%02d"):format(index)
        end
        value, _, code = protocol.validate_runtime(runtime)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local abort = fixture.messages()[12]
        abort.body.detail = string.rep("x", protocol.MAX_DETAIL_BYTES + 1)
        value, _, code = protocol.validate(abort)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local oversized_sequence
        oversized_sequence, _, code = protocol.new(
            "ready",
            "session_alpha",
            "host",
            protocol.MAX_SEQUENCE + 1,
            { manifest_id = protocol.manifest_id(fixture.manifest()), ready = true }
        )
        t.eq(oversized_sequence, nil)
        t.eq(code, "malformed")
    end)

    t.it("rejects invalid phase use before callers mutate lifecycle state", function()
        local messages = fixture.messages()
        t.is_true(assert(protocol.validate_phase(messages[1], "new")))
        t.is_true(assert(protocol.validate_phase(messages[8], "countdown")))
        t.is_true(assert(protocol.validate_phase(messages[10], "running")))

        local ok, _, code = protocol.validate_phase(messages[8], "manifest")
        t.eq(ok, nil)
        t.eq(code, "invalid_phase")
        ok, _, code = protocol.validate_phase(messages[10], "terminal")
        t.eq(ok, nil)
        t.eq(code, "invalid_phase")
        ok, _, code = protocol.validate_phase(messages[12], "terminal")
        t.eq(ok, nil)
        t.eq(code, "invalid_phase")
    end)

    t.it("makes exact duplicates idempotent and conflicting reuse terminal", function()
        local previous = fixture.messages()[6]
        local duplicate = assert(protocol.decode(assert(protocol.encode(previous))))
        t.eq(assert(protocol.classify_duplicate(previous, duplicate)), "idempotent")

        local conflict = deep_copy(previous)
        conflict.body.ready = false
        local disposition, _, code = protocol.classify_duplicate(previous, conflict)
        t.eq(disposition, nil)
        t.eq(code, "transcript_conflict")

        local other = fixture.messages()[7]
        disposition, _, code = protocol.classify_duplicate(previous, other)
        t.eq(disposition, nil)
        t.eq(code, "duplicate")
    end)

    t.it("derives replay-safe transcript identity from canonical ordered messages", function()
        local messages = fixture.messages()
        local first = protocol.transcript_id(messages)
        local second = protocol.transcript_id(fixture.messages())
        t.eq(first, second)
        t.eq(#first, 16)

        messages[10].body.tick = 61
        t.is_true(protocol.transcript_id(messages) ~= first)
    end)
end)
