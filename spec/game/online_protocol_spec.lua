local fnv1a64 = require("core.fnv1a64")
local conformance = require("game.online.protocol_conformance")
local fixture = require("game.online.protocol_fixture")
local protocol = require("game.online.protocol")
local input_frame = require("sim.input_frame")
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

---@param prefix string
---@param length integer
---@return string
local function bounded_id(prefix, length)
    assert(#prefix <= length)
    return prefix .. string.rep("x", length - #prefix)
end

---@param value any
---@return string
local function encode_raw_test_value(value)
    local kind = type(value)
    if kind == "boolean" then
        return value and "b1" or "b0"
    elseif kind == "number" then
        local text = tostring(value)
        return "i" .. tostring(#text) .. ":" .. text
    elseif kind == "string" then
        return "s" .. tostring(#value) .. ":" .. value
    end
    assert(kind == "table")
    local keys = {}
    for key in pairs(value) do
        keys[#keys + 1] = key
    end
    table.sort(keys, function(left, right)
        if type(left) ~= type(right) then
            return type(left) == "number"
        end
        return left < right
    end)
    local parts = { "t", tostring(#keys), ":" }
    for _, key in ipairs(keys) do
        parts[#parts + 1] = encode_raw_test_value(key)
        parts[#parts + 1] = encode_raw_test_value(value[key])
    end
    return table.concat(parts)
end

---@param message SessionControlMessage
---@return string
local function encode_raw_test_wire(message)
    return "GCOP;" .. tostring(protocol.VERSION) .. ";" .. encode_raw_test_value(message)
end

---@param manifest SessionManifest
---@param team_index integer
---@param roster_index integer
---@param player_id string
local function replace_manifest_player_id(manifest, team_index, roster_index, player_id)
    local player = manifest.teams[team_index].roster[roster_index]
    local previous = player.player_id
    player.player_id = player_id
    for _, slot in ipairs(manifest.slots) do
        if slot.player_id == previous then
            slot.player_id = player_id
            return
        end
    end
end

t.describe("OMP-3 online protocol", function()
    t.it("pins the accepted input, snapshot, tape, and combat schema versions", function()
        t.eq(protocol.CURRENT_VERSIONS.protocol, 1)
        t.eq(protocol.CURRENT_VERSIONS.input, 2)
        t.eq(protocol.CURRENT_VERSIONS.snapshot, 12)
        t.eq(protocol.CURRENT_VERSIONS.tape, 2)
        t.eq(protocol.CURRENT_VERSIONS.combat, 1)
    end)

    t.it("matches literal wire, manifest, transcript, and per-kind golden evidence", function()
        local report = conformance.verify()
        t.eq(report.manifest_id, "659947cdd13d7d68")
        t.eq(report.transcript_id, "5772846370555e41")
        t.eq(report.message_count, 13)
        t.eq(fnv1a64.hash(conformance.GOLDEN.complete_wire), "66c31c2ba4fe0898")
        t.eq(
            conformance.marker(report),
            "GC_PROTOCOL|golden|schema=1|manifest_id=659947cdd13d7d68"
                .. "|transcript_id=5772846370555e41|messages=13"
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

    t.it("rejects sparse arrays without relying on Lua's undefined length operator", function()
        local runtime = fixture.runtime()
        runtime.capabilities = {
            [1] = "combat_feedback.v1",
            [3] = "input_channel.v1",
        }
        local value, _, code = protocol.validate_runtime(runtime)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local sparse_manifest_cases = {
            {
                name = "teams",
                mutate = function(manifest)
                    manifest.teams = { [1] = manifest.teams[1], [3] = manifest.teams[2] }
                end,
            },
            {
                name = "roster",
                mutate = function(manifest)
                    local roster = manifest.teams[1].roster
                    manifest.teams[1].roster = {
                        [1] = roster[1],
                        [2] = roster[2],
                        [3] = roster[3],
                        [4] = roster[4],
                        [6] = roster[5],
                    }
                end,
            },
            {
                name = "slots",
                mutate = function(manifest)
                    local slots = manifest.slots
                    manifest.slots = {
                        [1] = slots[1],
                        [2] = slots[2],
                        [3] = slots[3],
                        [4] = slots[4],
                        [5] = slots[5],
                        [6] = slots[6],
                        [7] = slots[7],
                        [9] = slots[8],
                    }
                end,
            },
        }
        for _, case in ipairs(sparse_manifest_cases) do
            local manifest = fixture.manifest()
            case.mutate(manifest)
            value, _, code = protocol.validate_manifest(manifest)
            t.eq(value, nil, case.name)
            t.eq(code, "malformed", case.name)
        end

        local assignments = fixture.assignments()
        assignments = {
            [1] = assignments[1],
            [2] = assignments[2],
            [3] = assignments[3],
            [4] = assignments[4],
            [5] = assignments[5],
            [6] = assignments[6],
            [7] = assignments[7],
            [9] = assignments[8],
        }
        local assignment_message = deep_copy(fixture.messages()[5])
        assignment_message.body.assignments = assignments
        value, _, code = protocol.validate(assignment_message)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local messages = fixture.messages()
        local sparse_transcript = { [1] = messages[1], [3] = messages[2] }
        local ok, err = pcall(protocol.transcript_id, sparse_transcript)
        t.eq(ok, false)
        t.is_true(tostring(err):find("canonical message array", 1, true) ~= nil)
    end)

    t.it("rejects sparse numeric-key arrays parsed from independent raw wires", function()
        local cases = {
            {
                name = "runtime capabilities",
                message_index = 1,
                mutate = function(message)
                    local capabilities = message.body.runtime.capabilities
                    message.body.runtime.capabilities = {
                        [1] = capabilities[1],
                        [3] = capabilities[3],
                    }
                end,
            },
            {
                name = "manifest teams",
                message_index = 2,
                mutate = function(message)
                    local teams = message.body.manifest.teams
                    message.body.manifest.teams = { [1] = teams[1], [3] = teams[2] }
                end,
            },
            {
                name = "manifest roster",
                message_index = 2,
                mutate = function(message)
                    local roster = message.body.manifest.teams[1].roster
                    message.body.manifest.teams[1].roster = {
                        [1] = roster[1],
                        [2] = roster[2],
                        [3] = roster[3],
                        [4] = roster[4],
                        [6] = roster[5],
                    }
                end,
            },
            {
                name = "manifest slots",
                message_index = 2,
                mutate = function(message)
                    local slots = message.body.manifest.slots
                    message.body.manifest.slots = {
                        [1] = slots[1],
                        [2] = slots[2],
                        [3] = slots[3],
                        [4] = slots[4],
                        [5] = slots[5],
                        [6] = slots[6],
                        [7] = slots[7],
                        [9] = slots[8],
                    }
                end,
            },
            {
                name = "producer assignments",
                message_index = 5,
                mutate = function(message)
                    local assignments = message.body.assignments
                    message.body.assignments = {
                        [1] = assignments[1],
                        [2] = assignments[2],
                        [3] = assignments[3],
                        [4] = assignments[4],
                        [5] = assignments[5],
                        [6] = assignments[6],
                        [7] = assignments[7],
                        [9] = assignments[8],
                    }
                end,
            },
        }
        local messages = fixture.messages()
        for _, case in ipairs(cases) do
            local message = deep_copy(messages[case.message_index])
            case.mutate(message)
            local decoded, _, code = protocol.decode(encode_raw_test_wire(message))
            t.eq(decoded, nil, case.name)
            t.eq(code, "malformed", case.name)
        end
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

    t.it("rejects representative raw parser failures", function()
        local valid_wire = assert(protocol.encode(fixture.messages()[1]))
        local malformed_wires = {
            { name = "unknown value tag", wire = "GCOP;1;x" },
            { name = "truncated string", wire = "GCOP;1;s1:" },
            { name = "noncanonical integer", wire = "GCOP;1;i2:01" },
            { name = "invalid boolean", wire = "GCOP;1;b2" },
            {
                name = "duplicate table key",
                wire = "GCOP;1;t2:s1:as1:xs1:as1:y",
            },
            { name = "trailing bytes", wire = valid_wire .. "x" },
        }
        for _, case in ipairs(malformed_wires) do
            local value, _, code = protocol.decode(case.wire)
            t.eq(value, nil, case.name)
            t.eq(code, "malformed", case.name)
        end
    end)

    t.it("rejects malformed bodies for every control message kind", function()
        local cases = {
            {
                kind = "handshake",
                mutate = function(body)
                    body.role = "captain"
                end,
            },
            {
                kind = "manifest_proposal",
                mutate = function(body)
                    body.manifest_id = "not-a-hash"
                end,
            },
            {
                kind = "manifest_accept",
                mutate = function(body)
                    body.manifest_id = "not-a-hash"
                end,
            },
            {
                kind = "peer_assignment",
                mutate = function(body)
                    body.assigned_peer_id = ""
                end,
            },
            {
                kind = "slot_assignment",
                mutate = function(body)
                    body.assignments = {}
                end,
            },
            {
                kind = "ready",
                mutate = function(body)
                    body.ready = "yes"
                end,
            },
            {
                kind = "countdown",
                mutate = function(body)
                    body.remaining_ticks = -1
                end,
            },
            {
                kind = "start",
                mutate = function(body)
                    body.first_input_tick = -1
                end,
            },
            {
                kind = "match_phase",
                mutate = function(body)
                    body.phase = "countdown"
                end,
            },
            {
                kind = "hash_report",
                mutate = function(body)
                    body.boundary_hash = "not-a-hash"
                end,
            },
            {
                kind = "result_ack",
                mutate = function(body)
                    body.final_hash = "not-a-hash"
                end,
            },
            {
                kind = "abort",
                mutate = function(body)
                    body.code = "freeform_abort"
                end,
            },
            {
                kind = "disconnect",
                mutate = function(body)
                    body.code = "freeform_disconnect"
                end,
            },
        }
        local messages = fixture.messages()
        t.eq(#cases, #messages)
        for index, case in ipairs(cases) do
            local message = deep_copy(messages[index])
            t.eq(message.kind, case.kind)
            case.mutate(message.body)
            local value, _, code = protocol.validate(message)
            t.eq(value, nil, case.kind)
            t.eq(code, "malformed", case.kind)
        end

        for _, index in ipairs({ 12, 13 }) do
            local message = deep_copy(messages[index])
            message.body.detail = "peer-authored prose"
            local value, _, code = protocol.validate(message)
            t.eq(value, nil, message.kind .. " prose")
            t.eq(code, "malformed", message.kind .. " prose")
        end
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

        assignments = fixture.assignments()
        assignments[7].producer_id = assignments[1].producer_id
        invalid_message, _, code = protocol.new("slot_assignment", manifest.session_id, "host", 3, {
            manifest_id = protocol.manifest_id(manifest),
            assignments = assignments,
        })
        t.eq(invalid_message, nil)
        t.eq(code, "malformed")
    end)

    t.it("uses the InputFrame player-id bound for every player-id surface", function()
        local lengths = {
            { name = "minimum", length = 1, accepted = true },
            { name = "maximum", length = input_frame.MAX_PLAYER_ID_BYTES, accepted = true },
            { name = "over", length = input_frame.MAX_PLAYER_ID_BYTES + 1, accepted = false },
        }
        for _, case in ipairs(lengths) do
            local player_id = bounded_id("p", case.length)
            local manifest = fixture.manifest()
            replace_manifest_player_id(manifest, 1, 2, player_id)
            local value, _, code = protocol.validate_manifest(manifest)
            t.eq(value == true, case.accepted, "roster " .. case.name)
            if not case.accepted then
                t.eq(code, "malformed", "roster " .. case.name)
            end

            if not case.accepted then
                manifest = fixture.manifest()
                manifest.slots[1].player_id = player_id
                value, _, code = protocol.validate_manifest(manifest)
                t.eq(value, nil, "manifest slot " .. case.name)
                t.eq(code, "malformed", "manifest slot " .. case.name)
            end

            local assignments = fixture.assignments()
            assignments[1].player_id = player_id
            local message
            message, _, code =
                protocol.new("slot_assignment", "session_alpha", "host", case.length, {
                    manifest_id = protocol.manifest_id(fixture.manifest()),
                    assignments = assignments,
                })
            t.eq(message ~= nil, case.accepted, "producer assignment " .. case.name)
            if not case.accepted then
                t.eq(code, "malformed", "producer assignment " .. case.name)
            end
        end
    end)

    t.it("names the first deterministic identity mismatch before countdown", function()
        local expected = fixture.manifest()
        local cases = {
            {
                path = "manifest.build_id",
                mutate = function(actual)
                    actual.build_id = "build.other"
                end,
            },
            {
                path = "manifest.teams.2.team_id",
                mutate = function(actual)
                    actual.teams[2].team_id = "team_other"
                end,
            },
            {
                path = "manifest.teams.2.roster.3.family_id",
                mutate = function(actual)
                    actual.teams[2].roster[3].family_id = "ranged"
                end,
            },
            {
                path = "manifest.slots.1.player_id",
                mutate = function(actual)
                    actual.slots[1].player_id, actual.slots[2].player_id =
                        actual.slots[2].player_id, actual.slots[1].player_id
                end,
            },
        }
        for _, case in ipairs(cases) do
            local actual = deep_copy(expected)
            case.mutate(actual)
            local ok, _, code, path = protocol.compare_manifest(expected, actual)
            t.eq(ok, nil, case.path)
            t.eq(code, "identity_mismatch", case.path)
            t.eq(path, case.path, case.path)
        end

        local actual = deep_copy(expected)
        actual.presentation_id = "not-a-manifest-field"
        local valid, _, valid_code = protocol.validate_manifest(actual)
        t.eq(valid, nil)
        t.eq(valid_code, "malformed")
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

    t.it("rejects old and future control, runtime, and nested manifest versions", function()
        local version_fields = {
            "version",
            "protocol_version",
            "input_version",
            "snapshot_version",
            "tape_version",
            "combat_schema_version",
        }
        for _, field in ipairs(version_fields) do
            for _, delta in ipairs({ -1, 1 }) do
                local manifest = fixture.manifest()
                manifest[field] = manifest[field] + delta
                local value, _, code = protocol.validate_manifest(manifest)
                t.eq(value, nil, field .. " " .. tostring(delta))
                t.eq(code, "unsupported_version", field .. " " .. tostring(delta))
            end
        end

        for _, delta in ipairs({ -1, 1 }) do
            local runtime = fixture.runtime()
            runtime.version = runtime.version + delta
            local value, _, code = protocol.validate_runtime(runtime)
            t.eq(value, nil, "runtime " .. tostring(delta))
            t.eq(code, "unsupported_version", "runtime " .. tostring(delta))

            local message = deep_copy(fixture.messages()[1])
            message.version = message.version + delta
            value, _, code = protocol.validate(message)
            t.eq(value, nil, "message " .. tostring(delta))
            t.eq(code, "unsupported_version", "message " .. tostring(delta))

            local wire = assert(protocol.encode(fixture.messages()[1]))
            wire = wire:gsub("^GCOP;1;", "GCOP;" .. tostring(protocol.VERSION + delta) .. ";")
            local decoded, _, decode_code = protocol.decode(wire)
            t.eq(decoded, nil, "wire " .. tostring(delta))
            t.eq(decode_code, "unsupported_version", "wire " .. tostring(delta))
        end
    end)

    t.it("accepts exact scalar bounds and rejects every over-bound class", function()
        for _, case in ipairs({
            { name = "minimum generic id", length = 1, accepted = true },
            { name = "maximum generic id", length = protocol.MAX_ID_BYTES, accepted = true },
            { name = "over generic id", length = protocol.MAX_ID_BYTES + 1, accepted = false },
        }) do
            local manifest = fixture.manifest()
            manifest.build_id = bounded_id("b", case.length)
            local value, _, code = protocol.validate_manifest(manifest)
            t.eq(value == true, case.accepted, case.name)
            if not case.accepted then
                t.eq(code, "malformed", case.name)
            end
        end

        for _, case in ipairs({
            { name = "minimum session", session = "s", peer = "peer", accepted = true },
            {
                name = "maximum session",
                session = bounded_id("s", protocol.MAX_SESSION_ID_BYTES),
                peer = "peer",
                accepted = true,
            },
            {
                name = "over session",
                session = bounded_id("s", protocol.MAX_SESSION_ID_BYTES + 1),
                peer = "peer",
                accepted = false,
            },
            { name = "minimum peer", session = "session", peer = "p", accepted = true },
            {
                name = "maximum peer",
                session = "session",
                peer = bounded_id("p", protocol.MAX_PEER_ID_BYTES),
                accepted = true,
            },
            {
                name = "over peer",
                session = "session",
                peer = bounded_id("p", protocol.MAX_PEER_ID_BYTES + 1),
                accepted = false,
            },
        }) do
            local value, _, code = protocol.message_id(case.session, case.peer, 0)
            t.eq(value ~= nil, case.accepted, case.name)
            if not case.accepted then
                t.eq(code, "malformed", case.name)
            end
        end

        for _, case in ipairs({
            { name = "minimum sequence", value = 0, accepted = true },
            { name = "maximum sequence", value = protocol.MAX_SEQUENCE, accepted = true },
            { name = "over sequence", value = protocol.MAX_SEQUENCE + 1, accepted = false },
        }) do
            local value, _, code = protocol.message_id("session", "peer", case.value)
            t.eq(value ~= nil, case.accepted, case.name)
            if not case.accepted then
                t.eq(code, "malformed", case.name)
            end
        end

        local manifest_integer_cases = {
            {
                name = "seed",
                field = "seed",
                minimum = 0,
                maximum = protocol.MAX_SEED,
            },
            {
                name = "duration",
                field = "duration_ticks",
                minimum = 1,
                maximum = protocol.MAX_DURATION_TICKS,
            },
            {
                name = "goal limit",
                field = "max_goals",
                minimum = 1,
                maximum = protocol.MAX_GOALS,
            },
        }
        for _, case in ipairs(manifest_integer_cases) do
            for _, boundary in ipairs({
                { name = "minimum", value = case.minimum, accepted = true },
                { name = "maximum", value = case.maximum, accepted = true },
                { name = "over", value = case.maximum + 1, accepted = false },
            }) do
                local manifest = fixture.manifest()
                manifest[case.field] = boundary.value
                local value, _, code = protocol.validate_manifest(manifest)
                t.eq(value == true, boundary.accepted, case.name .. " " .. boundary.name)
                if not boundary.accepted then
                    t.eq(code, "malformed", case.name .. " " .. boundary.name)
                end
            end
        end

        for _, case in ipairs({
            { name = "minimum", value = 0, accepted = true },
            { name = "maximum", value = protocol.MAX_COUNTDOWN_TICKS, accepted = true },
            { name = "over", value = protocol.MAX_COUNTDOWN_TICKS + 1, accepted = false },
        }) do
            local message = deep_copy(fixture.messages()[7])
            message.body.remaining_ticks = case.value
            local value, _, code = protocol.validate(message)
            t.eq(value == true, case.accepted, "countdown " .. case.name)
            if not case.accepted then
                t.eq(code, "malformed", "countdown " .. case.name)
            end
        end

        for _, case in ipairs({
            { name = "minimum", value = 0, accepted = true },
            { name = "maximum", value = input_frame.MAX_TICK, accepted = true },
            { name = "over", value = input_frame.MAX_TICK + 1, accepted = false },
        }) do
            local message = deep_copy(fixture.messages()[10])
            message.body.tick = case.value
            local value, _, code = protocol.validate(message)
            t.eq(value == true, case.accepted, "tick " .. case.name)
            if not case.accepted then
                t.eq(code, "malformed", "tick " .. case.name)
            end
        end

        for _, case in ipairs({
            { name = "minimum", value = 0, accepted = true },
            { name = "maximum", value = protocol.MAX_GOALS, accepted = true },
            { name = "over", value = protocol.MAX_GOALS + 1, accepted = false },
        }) do
            local message = deep_copy(fixture.messages()[9])
            message.body.home_score = case.value
            local value, _, code = protocol.validate(message)
            t.eq(value == true, case.accepted, "score " .. case.name)
            if not case.accepted then
                t.eq(code, "malformed", "score " .. case.name)
            end
        end

        for _, case in ipairs({
            { name = "minimum", count = 0, accepted = true },
            { name = "maximum", count = protocol.MAX_CAPABILITIES, accepted = true },
            { name = "over", count = protocol.MAX_CAPABILITIES + 1, accepted = false },
        }) do
            local runtime = fixture.runtime()
            runtime.capabilities = {}
            for index = 1, case.count do
                runtime.capabilities[index] = ("capability_%03d"):format(index)
            end
            local value, _, code = protocol.validate_runtime(runtime)
            t.eq(value == true, case.accepted, "capabilities " .. case.name)
            if not case.accepted then
                t.eq(code, "malformed", "capabilities " .. case.name)
            end
        end

        local maximum_id = assert(
            protocol.message_id(
                bounded_id("s", protocol.MAX_SESSION_ID_BYTES),
                bounded_id("p", protocol.MAX_PEER_ID_BYTES),
                protocol.MAX_SEQUENCE
            )
        )
        t.eq(#maximum_id, protocol.MAX_MESSAGE_ID_BYTES)
        local message = deep_copy(fixture.messages()[1])
        message.message_id = string.rep("m", protocol.MAX_MESSAGE_ID_BYTES + 1)
        local value, _, code = protocol.validate(message)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local decoded, _, decode_code =
            protocol.decode(string.rep("x", protocol.MAX_WIRE_BYTES + 1))
        t.eq(decoded, nil)
        t.eq(decode_code, "wire_too_large")
    end)

    t.it("keeps an all-applicable-max proposal within the 8 KiB record bound", function()
        local manifest = fixture.manifest()
        manifest.session_id = bounded_id("session", protocol.MAX_SESSION_ID_BYTES)
        manifest.combat_status = "accepted_revision"
        manifest.seed = protocol.MAX_SEED
        manifest.duration_ticks = protocol.MAX_DURATION_TICKS
        manifest.max_goals = protocol.MAX_GOALS
        for _, field in ipairs({
            "build_id",
            "source_id",
            "content_id",
            "tuning_id",
            "match_config_id",
            "fixture_id",
            "arena_id",
            "combat_rules_id",
            "gameplay_ai_policy_id",
        }) do
            manifest[field] = bounded_id(field:sub(1, 1), protocol.MAX_ID_BYTES)
        end
        manifest.teams[1].team_id = bounded_id("home", protocol.MAX_ID_BYTES)
        manifest.teams[2].team_id = bounded_id("away", protocol.MAX_ID_BYTES)
        for team_index, team in ipairs(manifest.teams) do
            for roster_index, player in ipairs(team.roster) do
                replace_manifest_player_id(
                    manifest,
                    team_index,
                    roster_index,
                    bounded_id(
                        ("p%d%d"):format(team_index, roster_index),
                        input_frame.MAX_PLAYER_ID_BYTES
                    )
                )
                if player.position ~= "keeper" then
                    player.position = "midfielder"
                    player.loadout_id = bounded_id(
                        ("l%d%d"):format(team_index, roster_index),
                        protocol.MAX_ID_BYTES
                    )
                    player.family_id = bounded_id(
                        ("f%d%d"):format(team_index, roster_index),
                        protocol.MAX_ID_BYTES
                    )
                end
            end
        end
        t.is_true(assert(protocol.validate_manifest(manifest)))

        local maximal_proposal = assert(
            protocol.new(
                "manifest_proposal",
                manifest.session_id,
                bounded_id("peer", protocol.MAX_PEER_ID_BYTES),
                protocol.MAX_SEQUENCE,
                {
                    manifest_id = protocol.manifest_id(manifest),
                    manifest = manifest,
                }
            )
        )
        local maximal_wire = assert(protocol.encode(maximal_proposal))
        t.is_true(#maximal_wire <= protocol.MAX_WIRE_BYTES)
        t.eq(#maximal_wire, 7220)
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

        for _, match_phase in ipairs({
            "kickoff",
            "playing",
            "goal_stoppage",
            "full_time",
        }) do
            local message = deep_copy(messages[9])
            message.body.phase = match_phase
            t.is_true(
                assert(protocol.validate_phase(message, "running")),
                "running " .. match_phase
            )
            ok, _, code = protocol.validate_phase(message, "result")
            t.eq(ok, nil, "result " .. match_phase)
            t.eq(code, "invalid_phase", "result " .. match_phase)
        end

        local result_message = deep_copy(messages[9])
        result_message.body.phase = "result"
        t.is_true(assert(protocol.validate_phase(result_message, "result")))
        ok, _, code = protocol.validate_phase(result_message, "running")
        t.eq(ok, nil)
        t.eq(code, "invalid_phase")

        ok, _, code = protocol.validate_phase(messages[7], "result")
        t.eq(ok, nil)
        t.eq(code, "invalid_phase")

        local invalid_match_phase = deep_copy(messages[9])
        invalid_match_phase.body.phase = "countdown"
        ok, _, code = protocol.validate_phase(invalid_match_phase, "result")
        t.eq(ok, nil)
        t.eq(code, "malformed")
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
