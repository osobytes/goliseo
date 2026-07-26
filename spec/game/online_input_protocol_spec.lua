local t = require("spec.support.runner")
local input_protocol = require("game.online.input_protocol")
local input_conformance = require("game.online.input_protocol_conformance")
local input_fixture = require("game.online.input_protocol_fixture")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")
local rollback_input_history = require("sim.rollback_input_history")

---@param mask integer
---@param bit integer
---@return boolean
local function has_bit(mask, bit)
    return math.floor(mask / bit) % 2 == 1
end

---@param tick integer
---@param slot_index integer
---@param sample InputSample
---@return InputAuthorityRow
local function row(tick, slot_index, sample)
    return { tick = tick, slot_index = slot_index, sample = sample }
end

---@param value integer
---@return InputSample
local function fuzz_sample(value)
    local held = (value * 73) % 256
    local edges = (value * 41) % 128
    local equipment = input_frame.HELD_BITS.equipment
    local pressed = input_frame.EDGE_BITS.equipment_pressed
    local released = input_frame.EDGE_BITS.equipment_released
    if has_bit(edges, released) and has_bit(held, equipment) then
        held = held - equipment
    end
    if
        has_bit(edges, pressed)
        and not has_bit(edges, released)
        and not has_bit(held, equipment)
    then
        held = held + equipment
    end
    return assert(input_frame.new_sample({
        move_x = (value * 37) % 255 - 127,
        move_y = (value * 91) % 255 - 127,
        held = held,
        edges = edges,
    }))
end

---@param slot_index integer
---@param sender_id string
---@param sequence integer
---@param transport_tick integer
---@param current_tick integer
---@param first_input_tick integer?
---@param sample_offset integer?
---@return InputPacket
local function guest_packet(
    slot_index,
    sender_id,
    sequence,
    transport_tick,
    current_tick,
    first_input_tick,
    sample_offset
)
    local manifest = protocol_fixture.manifest()
    local first = first_input_tick or 0
    local rows = {}
    for tick = math.max(first, current_tick - input_protocol.HISTORY_ROWS), current_tick do
        rows[#rows + 1] = row(tick, slot_index, fuzz_sample(tick + (sample_offset or slot_index)))
    end
    return assert(input_protocol.new_guest({
        session_id = manifest.session_id,
        manifest_id = protocol.manifest_id(manifest),
        sender_id = sender_id,
        sequence = sequence,
        transport_tick = transport_tick,
        first_input_tick = first,
        rows = rows,
    }))
end

---@param packet InputPacket
---@return TransportMessage
local function envelope(packet)
    return assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = assert(input_protocol.encode(packet)),
    }))
end

---@param packet InputPacket
---@param arrival_tick integer
---@param transport_peer_id string
---@return InputPacketArrival
local function arrival(packet, arrival_tick, transport_peer_id)
    return {
        packet = packet,
        envelope = envelope(packet),
        arrival_tick = arrival_tick,
        transport_peer_id = transport_peer_id,
    }
end

---@return RollbackInputSource[]
local function remote_sources()
    local result = {}
    for index = 1, input_frame.SLOT_COUNT do
        result[index] = "remote"
    end
    return result
end

---@param sequence integer
---@param transport_tick integer
---@return InputHostBatchOptions
local function host_options(sequence, transport_tick)
    return {
        manifest = protocol_fixture.manifest(),
        assignments = protocol_fixture.assignments(),
        host_peer_id = "host",
        sequence = sequence,
        transport_tick = transport_tick,
        first_input_tick = 0,
    }
end

t.describe("OMP-3 input packet protocol", function()
    t.it("pins literal native and love.js conformance vectors", function()
        local report = input_conformance.verify()
        t.eq(report.guest_digest, "3332f9c19ea9ce34")
        t.eq(report.host_digest, "1e9b1ebfad823a44")
        t.eq(report.maximal_wire_bytes, 755)
        t.eq(
            input_conformance.marker(report),
            "GC_INPUT_PROTOCOL|golden|schema=1|input=2|history=6|delay=3|vectors=2"
                .. "|guest=3332f9c19ea9ce34|host=1e9b1ebfad823a44|max_bytes=755"
        )
    end)

    t.it("round-trips current plus exactly six prior guest rows with distinct clocks", function()
        local packet = input_fixture.guest()
        local wire = assert(input_protocol.encode(packet))
        t.is_true(#wire <= input_protocol.MAX_WIRE_BYTES)
        local decoded = assert(input_protocol.decode(wire, {
            session_id = packet.session_id,
            manifest_id = packet.manifest_id,
            sender_id = packet.sender_id,
        }))
        t.eq(assert(input_protocol.encode(decoded)), wire)
        t.eq(decoded.transport_tick, 12)
        t.eq(decoded.rows[1].tick, 0)
        t.eq(decoded.rows[7].tick, 6)
        t.eq(decoded.rows[1].slot_index, 2)
        t.eq(decoded.input_delay_ticks, 3)
        t.is_true(assert(input_protocol.validate_envelope(decoded, envelope(decoded))))

        local early = guest_packet(2, "guest_2", 8, 13, 2, 2)
        t.eq(#early.rows, 1, "the packet does not invent pre-start authority")
        local later = guest_packet(2, "guest_2", 9, 14, 8, 2)
        t.eq(#later.rows, 7)
        t.eq(later.rows[1].tick, 2)
        t.eq(later.rows[7].tick, 8)
    end)

    t.it(
        "covers every slot, soccer/combat bit, axis boundary, and generated valid sample",
        function()
            local sequence = 20
            for slot_index = 1, input_frame.SLOT_COUNT do
                for _, bit in pairs(input_frame.HELD_BITS) do
                    local edges = bit == input_frame.HELD_BITS.equipment
                            and input_frame.EDGE_BITS.equipment_pressed
                        or 0
                    local sample = assert(input_frame.new_sample({
                        move_x = slot_index % 2 == 0 and -127 or 127,
                        move_y = slot_index % 2 == 0 and 127 or -127,
                        held = bit,
                        edges = edges,
                    }))
                    local manifest = protocol_fixture.manifest()
                    local packet = assert(input_protocol.new_guest({
                        session_id = manifest.session_id,
                        manifest_id = protocol.manifest_id(manifest),
                        sender_id = "producer_" .. tostring(slot_index),
                        sequence = sequence,
                        transport_tick = sequence,
                        first_input_tick = 0,
                        rows = { row(0, slot_index, sample) },
                    }))
                    local decoded =
                        assert(input_protocol.decode(assert(input_protocol.encode(packet)), {
                            session_id = packet.session_id,
                            manifest_id = packet.manifest_id,
                            sender_id = packet.sender_id,
                        }))
                    t.eq(decoded.rows[1].sample.held, bit)
                    sequence = sequence + 1
                end
                for _, bit in pairs(input_frame.EDGE_BITS) do
                    local held = bit == input_frame.EDGE_BITS.equipment_pressed
                            and input_frame.HELD_BITS.equipment
                        or 0
                    local sample = assert(input_frame.new_sample({ held = held, edges = bit }))
                    local manifest = protocol_fixture.manifest()
                    local packet = assert(input_protocol.new_guest({
                        session_id = manifest.session_id,
                        manifest_id = protocol.manifest_id(manifest),
                        sender_id = "producer_" .. tostring(slot_index),
                        sequence = sequence,
                        transport_tick = sequence,
                        first_input_tick = 0,
                        rows = { row(0, slot_index, sample) },
                    }))
                    local decoded =
                        assert(input_protocol.decode(assert(input_protocol.encode(packet)), {
                            session_id = packet.session_id,
                            manifest_id = packet.manifest_id,
                            sender_id = packet.sender_id,
                        }))
                    t.eq(decoded.rows[1].sample.edges, bit)
                    sequence = sequence + 1
                end
            end

            for value = 0, 1023 do
                local sample = fuzz_sample(value)
                local packet = guest_packet(
                    value % input_frame.SLOT_COUNT + 1,
                    "fuzzer",
                    value,
                    value,
                    0,
                    0,
                    value
                )
                packet.rows[1].sample = sample
                local wire = assert(input_protocol.encode(packet))
                local decoded = assert(input_protocol.decode(wire, {
                    session_id = packet.session_id,
                    manifest_id = packet.manifest_id,
                    sender_id = packet.sender_id,
                }))
                t.eq(
                    assert(input_frame.encode_sample(decoded.rows[1].sample)),
                    assert(input_frame.encode_sample(sample))
                )
            end
        end
    )

    t.it("recovers six lost emissions without creating unsent authority", function()
        local recovered = guest_packet(2, "guest_2", 6, 6, 6)
        local history = rollback_input_history.new(remote_sources())
        local accepted = assert(
            rollback_input_history.add_authoritative_batch(history, input_protocol.rows(recovered))
        )
        t.eq(accepted.inserted, 7)
        t.is_true(rollback_input_history.authoritative_record(history, 0, 2) ~= nil)
        t.is_true(rollback_input_history.authoritative_record(history, 6, 2) ~= nil)
        t.eq(rollback_input_history.authoritative_record(history, 7, 2), nil)

        local after_seven_losses = guest_packet(2, "guest_2", 7, 7, 7)
        t.eq(after_seven_losses.rows[1].tick, 1)
        t.eq(
            rollback_input_history.authoritative_record(
                rollback_input_history.new(remote_sources()),
                0,
                2
            ),
            nil,
            "a later packet cannot invent the fallen-out tick"
        )
    end)

    t.it("rejects malformed, noncanonical, unsupported, mismatched, and oversized data", function()
        local packet = input_fixture.guest()
        local wire = assert(input_protocol.encode(packet))
        local decoded, _, code = input_protocol.decode(wire, {
            session_id = "other_session",
            manifest_id = packet.manifest_id,
            sender_id = packet.sender_id,
        })
        t.eq(decoded, nil)
        t.eq(code, "identity_mismatch")
        decoded, _, code = input_protocol.decode(wire, {
            session_id = packet.session_id,
            manifest_id = "fedcba9876543210",
            sender_id = packet.sender_id,
        })
        t.eq(decoded, nil)
        t.eq(code, "identity_mismatch")

        for _, case in ipairs({
            { wire = wire:gsub("^GCIP;1;", "GCIP;2;"), code = "unsupported_version" },
            { wire = wire:gsub("^GCIP;1;G;2;", "GCIP;1;G;3;"), code = "unsupported_version" },
            { wire = wire:gsub(";7;", ";07;", 1), code = "malformed" },
            { wire = wire:sub(1, -2) .. "*", code = "malformed" },
            { wire = string.rep("x", input_protocol.MAX_WIRE_BYTES + 1), code = "wire_too_large" },
        }) do
            decoded, _, code = input_protocol.decode(case.wire, {
                session_id = packet.session_id,
                manifest_id = packet.manifest_id,
                sender_id = packet.sender_id,
            })
            t.eq(decoded, nil)
            t.eq(code, case.code)
        end

        local invalid = assert(input_protocol.copy(packet))
        invalid.rows[1].sample.edges = 128
        local encoded
        encoded, _, code = input_protocol.encode(invalid)
        t.eq(encoded, nil)
        t.eq(code, "malformed")

        invalid = assert(input_protocol.copy(packet))
        invalid.rows[1], invalid.rows[2] = invalid.rows[2], invalid.rows[1]
        encoded, _, code = input_protocol.encode(invalid)
        t.eq(encoded, nil)
        t.eq(code, "malformed")

        invalid = assert(input_protocol.copy(packet))
        invalid.rows = { [1] = invalid.rows[1], [3] = invalid.rows[3] }
        encoded, _, code = input_protocol.encode(invalid)
        t.eq(encoded, nil)
        t.eq(code, "malformed")

        ---@type table<string, any>
        local extra = {}
        for key, child in pairs(assert(input_protocol.copy(packet))) do
            extra[key] = child
        end
        extra.extra = true
        local valid
        valid, _, code = input_protocol.validate(extra)
        t.eq(valid, nil)
        t.eq(code, "malformed")

        local sparse = input_protocol.rows(packet)
        sparse[2] = nil
        local constructed
        constructed, _, code = input_protocol.new_guest({
            session_id = packet.session_id,
            manifest_id = packet.manifest_id,
            sender_id = packet.sender_id,
            sequence = packet.sequence + 1,
            transport_tick = packet.transport_tick + 1,
            first_input_tick = packet.first_input_tick,
            rows = sparse,
        })
        t.eq(constructed, nil)
        t.eq(code, "malformed")

        local wrong_tick = assert(transport_contract.new({
            type = "input",
            seq = packet.sequence,
            tick = packet.transport_tick + 1,
            payload = wire,
        }))
        local ok
        ok, _, code = input_protocol.validate_envelope(packet, wrong_tick)
        t.eq(ok, nil)
        t.eq(code, "tick_mismatch")
    end)

    t.it("classifies packet and authority duplicates without first-arrival-wins", function()
        local original = input_fixture.guest()
        local duplicate = assert(input_protocol.copy(original))
        t.eq(assert(input_protocol.classify_duplicate(original, duplicate)), "idempotent")

        local conflict = assert(input_protocol.copy(original))
        conflict.rows[7].sample.move_x = conflict.rows[7].sample.move_x - 1
        local disposition, _, code = input_protocol.classify_duplicate(original, conflict)
        t.eq(disposition, nil)
        t.eq(code, "packet_conflict")

        local other = assert(input_protocol.new_guest({
            session_id = original.session_id,
            manifest_id = original.manifest_id,
            sender_id = original.sender_id,
            sequence = original.sequence + 1,
            transport_tick = 13,
            first_input_tick = original.first_input_tick,
            rows = input_protocol.rows(original),
        }))
        disposition, _, code = input_protocol.classify_duplicate(original, other)
        t.eq(disposition, nil)
        t.eq(code, "duplicate")

        local repeated = input_protocol.canonical_host_batch(host_options(40, 20), {
            arrival(original, 20, "guest_2"),
            arrival(duplicate, 20, "guest_2"),
            arrival(other, 20, "guest_2"),
        })
        t.eq(#assert(repeated).rows, 7)

        local changed = guest_packet(2, "guest_2", original.sequence + 2, 14, 6, 0, 100)
        local batch
        batch, _, code = input_protocol.canonical_host_batch(host_options(41, 20), {
            arrival(original, 20, "guest_2"),
            arrival(changed, 20, "guest_2"),
        })
        t.eq(batch, nil)
        t.eq(code, "authority_conflict")
    end)

    t.it("enforces frozen ownership and the host's three-tick local fairness path", function()
        local host_local = guest_packet(1, "host", 1, 7, 6)
        local batch, _, code = input_protocol.canonical_host_batch(host_options(50, 9), {
            arrival(host_local, 9, "host"),
        })
        t.eq(batch, nil)
        t.eq(code, "fairness_delay")

        batch = assert(input_protocol.canonical_host_batch(host_options(50, 10), {
            arrival(host_local, 10, "host"),
        }))
        t.eq(batch.rows[1].sample.move_x, host_local.rows[1].sample.move_x)
        t.eq(batch.rows[#batch.rows].sample.edges, host_local.rows[#host_local.rows].sample.edges)

        local false_claim = guest_packet(1, "guest_2", 2, 8, 6)
        batch, _, code = input_protocol.canonical_host_batch(host_options(51, 10), {
            arrival(false_claim, 10, "guest_2"),
        })
        t.eq(batch, nil)
        t.eq(code, "ownership_mismatch")

        local bot = guest_packet(7, "bot_away_3", 3, 7, 6)
        batch = assert(input_protocol.canonical_host_batch(host_options(52, 10), {
            arrival(bot, 10, "host"),
        }))
        t.eq(batch.rows[1].slot_index, 7)
    end)

    t.it("emits one byte-identical canonical host batch for every peer polling order", function()
        local assignments = protocol_fixture.assignments()
        local arrivals = {}
        for slot_index, assignment in ipairs(assignments) do
            local local_producer = assignment.producer_id == "host"
                or assignment.producer_kind == "bot"
            local packet = guest_packet(
                slot_index,
                assignment.producer_id,
                slot_index,
                local_producer and 17 or 19,
                6
            )
            arrivals[#arrivals + 1] = arrival(
                packet,
                20,
                assignment.producer_kind == "bot" and "host" or assignment.producer_id
            )
        end
        local reversed = {}
        for index = #arrivals, 1, -1 do
            reversed[#reversed + 1] = arrivals[index]
        end
        local first = assert(input_protocol.canonical_host_batch(host_options(60, 20), arrivals))
        local second = assert(input_protocol.canonical_host_batch(host_options(60, 20), reversed))
        t.eq(#first.rows, input_protocol.MAX_HOST_ROWS)
        t.eq(assert(input_protocol.encode(first)), assert(input_protocol.encode(second)))
        for index, authority in ipairs(first.rows) do
            if index > 1 then
                local previous = first.rows[index - 1]
                t.is_true(
                    previous.tick < authority.tick
                        or (
                            previous.tick == authority.tick
                            and previous.slot_index < authority.slot_index
                        )
                )
            end
        end
    end)

    t.it("fits the honest 56-row maximum and fails closed above it", function()
        local maximal = input_fixture.maximal()
        local wire = assert(input_protocol.encode(maximal))
        t.eq(#maximal.rows, 56)
        t.eq(#wire, 755)
        t.is_true(#wire <= input_protocol.MAX_WIRE_BYTES)
        local decoded = assert(input_protocol.decode(wire, {
            session_id = maximal.session_id,
            manifest_id = maximal.manifest_id,
            sender_id = maximal.sender_id,
        }))
        t.eq(#decoded.rows, 56)
        t.eq(decoded.rows[1].tick, input_frame.MAX_TICK - input_protocol.HISTORY_ROWS)
        t.eq(decoded.rows[56].tick, input_frame.MAX_TICK)

        local over = assert(input_protocol.copy(maximal))
        over.rows[57] = row(input_frame.MAX_TICK, 8, input_frame.neutral_sample())
        local value, _, code = input_protocol.encode(over)
        t.eq(value, nil)
        t.eq(code, "malformed")
    end)

    t.it("coalesces only when no unsent authority can fall through backpressure", function()
        local older = guest_packet(2, "guest_2", 70, 30, 6)
        local conflicting_reuse = assert(input_protocol.copy(older))
        conflicting_reuse.transport_tick = conflicting_reuse.transport_tick + 1
        local replacement, _, code =
            input_protocol.supersede_for_backpressure(older, conflicting_reuse)
        t.eq(replacement, nil)
        t.eq(code, "packet_conflict")

        local repeated = guest_packet(2, "guest_2", 71, 31, 6)
        t.is_true(input_protocol.supersede_for_backpressure(older, repeated) ~= nil)

        local next_tick = guest_packet(2, "guest_2", 72, 32, 7)
        replacement, _, code = input_protocol.supersede_for_backpressure(older, next_tick)
        t.eq(replacement, nil)
        t.eq(code, "backpressure_gap")
    end)

    t.it("does not coalesce across a different frozen first input tick", function()
        local older = guest_packet(2, "guest_2", 70, 30, 7, 0)
        local newer = guest_packet(2, "guest_2", 71, 31, 7, 1)
        local replacement, _, code = input_protocol.supersede_for_backpressure(older, newer)
        t.eq(replacement, nil)
        t.eq(code, "backpressure_gap")
    end)

    t.it("does not coalesce when transport time regresses", function()
        local older = guest_packet(2, "guest_2", 70, 30, 6)
        local newer = guest_packet(2, "guest_2", 71, 29, 6)
        local replacement, _, code = input_protocol.supersede_for_backpressure(older, newer)
        t.eq(replacement, nil)
        t.eq(code, "backpressure_gap")
    end)
end)
