local t = require("spec.support.runner")
local input_frame = require("sim.input_frame")
local players = require("data.players")

---@return table<string, PlayerData>
local function players_by_id()
    local by_id = {}
    for _, player in ipairs(players) do
        by_id[player.id] = player
    end
    return by_id
end

---@return InputSlotAssignment[]
local function assignments()
    local player_ids = {
        "zyro_vex",
        "mika_olu",
        "rok_tann",
        "sela_dwin",
        "drell",
        "morv",
        "krag",
        "tox_vren",
    }
    local result = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        result[index] = {
            slot = slot.id,
            team = slot.team,
            player_id = player_ids[index],
        }
    end
    return result
end

---@return InputFixtureRosters
local function fixture_rosters()
    return {
        home = { "ozzo", "zyro_vex", "mika_olu", "rok_tann", "sela_dwin" },
        away = { "gax_oru", "drell", "morv", "krag", "tox_vren" },
    }
end

---@return InputSample[]
local function neutral_slots()
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        slots[index] = input_frame.neutral_sample()
    end
    return slots
end

t.describe("OMP-1 input frame", function()
    t.it("defines exactly four stable outfield slots per team", function()
        local expected = {
            { "home_1", "home", 1 },
            { "home_2", "home", 2 },
            { "home_3", "home", 3 },
            { "home_4", "home", 4 },
            { "away_1", "away", 1 },
            { "away_2", "away", 2 },
            { "away_3", "away", 3 },
            { "away_4", "away", 4 },
        }
        t.eq(input_frame.SLOT_COUNT, 8)
        t.eq(input_frame.FIXTURE_TEAM_SIZE, 5)
        for index = 1, input_frame.SLOT_COUNT do
            local slot = assert(input_frame.slot(index))
            t.eq(slot.index, index)
            t.eq(slot.id, expected[index][1])
            t.eq(slot.team, expected[index][2])
            t.eq(slot.outfield_index, expected[index][3])
        end
        t.eq(assert(input_frame.slot_index("home", 4)), 4)
        t.eq(assert(input_frame.slot_index("away", 1)), 5)
        local slot, _, code = input_frame.slot(9)
        t.eq(slot, nil)
        t.eq(code, "malformed")
    end)

    t.it("maps canonical slots to unique non-keeper roster players", function()
        local by_id = players_by_id()
        local rosters = fixture_rosters()
        local ownership = assert(input_frame.new_ownership(assignments(), rosters, by_id))
        t.eq(ownership.version, input_frame.VERSION)
        t.eq(ownership.rosters.home[1], "ozzo")
        t.eq(ownership.rosters.away[1], "gax_oru")
        t.eq(#ownership.slots, input_frame.SLOT_COUNT)
        t.eq(ownership.slots[1].slot, "home_1")
        t.eq(ownership.slots[5].team, "away")
        t.eq(ownership.slots[8].player_id, "tox_vren")

        local legacy = assert(input_frame.copy_ownership(ownership, by_id))
        legacy.version = 1
        local legacy_ok, _, legacy_code = input_frame.validate_ownership(legacy, by_id)
        t.eq(legacy_ok, nil)
        t.eq(legacy_code, "unsupported_version")

        local duplicate = assignments()
        duplicate[4].player_id = duplicate[1].player_id
        local value, _, code = input_frame.new_ownership(duplicate, rosters, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local keeper = assignments()
        keeper[1].player_id = "ozzo"
        value, _, code = input_frame.new_ownership(keeper, rosters, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local wrong_team = assignments()
        wrong_team[5].team = "home"
        value, _, code = input_frame.new_ownership(wrong_team, rosters, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local cross_side = assignments()
        cross_side[5].player_id = "zyro_vex"
        value, _, code = input_frame.new_ownership(cross_side, rosters, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local unknown_assignment = assignments()
        unknown_assignment[1].player_id = "missing_player"
        value, _, code = input_frame.new_ownership(unknown_assignment, rosters, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local unknown_roster = fixture_rosters()
        unknown_roster.away[5] = "missing_player"
        value, _, code = input_frame.new_ownership(assignments(), unknown_roster, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")

        local no_keeper = fixture_rosters()
        no_keeper.home[1] = "brakka"
        value, _, code = input_frame.new_ownership(assignments(), no_keeper, by_id)
        t.eq(value, nil)
        t.eq(code, "malformed")
    end)

    t.it("creates independent neutral samples for every tick and slot", function()
        local frame = assert(input_frame.neutral(120))
        t.eq(frame.version, input_frame.VERSION)
        t.eq(frame.tick, 120)
        t.eq(#frame.slots, input_frame.SLOT_COUNT)
        for index = 1, input_frame.SLOT_COUNT do
            local sample = frame.slots[index]
            t.eq(sample.move_x, 0)
            t.eq(sample.move_y, 0)
            t.eq(sample.held, 0)
            t.eq(sample.edges, 0)
            t.eq(
                sample.aim,
                input_frame.AIM_NONE,
                "a neutral sample aims nowhere; zero is a legal direction"
            )
        end
        frame.slots[1].move_x = 20
        t.eq(frame.slots[2].move_x, 0, "neutral slots do not share a table")
    end)

    t.it("quantizes movement with fixed saturation, rounding, and decode rules", function()
        t.eq(assert(input_frame.quantize_axis(-2)), -127)
        t.eq(assert(input_frame.quantize_axis(-1)), -127)
        t.eq(assert(input_frame.quantize_axis(-0.5)), -64)
        t.eq(assert(input_frame.quantize_axis(0)), 0)
        t.eq(tostring(assert(input_frame.quantize_axis(-0.0))), "0")
        t.eq(assert(input_frame.quantize_axis(0.5)), 64)
        t.eq(assert(input_frame.quantize_axis(1)), 127)
        t.eq(assert(input_frame.quantize_axis(2)), 127)
        local move_x, move_y = assert(input_frame.quantize_move(-0.5, 0.5))
        t.eq(move_x, -64)
        t.eq(move_y, 64)

        t.near(assert(input_frame.dequantize_axis(-127)), -1)
        t.near(assert(input_frame.dequantize_axis(64)), 64 / 127)
        local decoded_x, decoded_y = assert(input_frame.dequantize_move({
            move_x = -64,
            move_y = 64,
            held = 0,
            edges = 0,
            aim = input_frame.AIM_NONE,
        }))
        t.near(decoded_x, -64 / 127)
        t.near(decoded_y, 64 / 127)

        local value, _, code = input_frame.quantize_axis(0 / 0)
        t.eq(value, nil)
        t.eq(code, "malformed")
    end)

    t.it("keeps supplied holds and one-tick edges distinct", function()
        local sample = assert(input_frame.new_sample({
            held = input_frame.HELD_BITS.shoot
                + input_frame.HELD_BITS.sprint
                + input_frame.HELD_BITS.equipment,
            edges = input_frame.EDGE_BITS.shoot
                + input_frame.EDGE_BITS.dash
                + input_frame.EDGE_BITS.equipment_pressed,
        }))
        t.is_true(assert(input_frame.is_held(sample, "shoot")))
        t.is_true(assert(input_frame.is_held(sample, "sprint")))
        t.is_true(assert(input_frame.is_held(sample, "equipment")))
        t.eq(input_frame.is_held(sample, "pass"), false)
        t.is_true(assert(input_frame.has_edge(sample, "shoot")))
        t.is_true(assert(input_frame.has_edge(sample, "dash")))
        t.is_true(assert(input_frame.has_edge(sample, "equipment_pressed")))
        t.eq(input_frame.has_edge(sample, "pass"), false)

        local next_sample = assert(input_frame.new_sample({ held = sample.held, edges = 0 }))
        t.is_true(assert(input_frame.is_held(next_sample, "shoot")))
        t.eq(input_frame.has_edge(next_sample, "shoot"), false)
    end)

    t.it("round-trips the compact five-byte sample without omitting combat bits", function()
        local all_holds = assert(input_frame.new_sample({
            move_x = -127,
            move_y = 127,
            held = 255,
            edges = input_frame.EDGE_BITS.equipment_pressed,
        }))
        local hold_wire = assert(input_frame.encode_sample(all_holds))
        t.eq(hold_wire, "00feff20ff")
        t.eq(#hold_wire, input_frame.MAX_SAMPLE_WIRE_BYTES)
        local decoded_holds = assert(input_frame.decode_sample(hold_wire))
        t.eq(decoded_holds.move_x, -127)
        t.eq(decoded_holds.move_y, 127)
        t.eq(decoded_holds.held, 255)
        t.eq(decoded_holds.edges, input_frame.EDGE_BITS.equipment_pressed)
        t.eq(decoded_holds.aim, input_frame.AIM_NONE)

        local all_edges = assert(input_frame.new_sample({
            held = 0,
            edges = 127,
        }))
        local edge_wire = assert(input_frame.encode_sample(all_edges))
        t.eq(edge_wire, "7f7f007fff")
        t.eq(assert(input_frame.decode_sample(edge_wire)).edges, 127)

        local aimed = assert(input_frame.new_sample({ aim = 0 }))
        t.eq(assert(input_frame.encode_sample(aimed)), "7f7f000000")
        t.eq(assert(input_frame.decode_sample("7f7f000000")).aim, 0)

        for _, wire in ipairs({ "7F7f007fff", "7f7f007ff", "7f7f0080ff", "zzzzzzzzzz" }) do
            local value, _, code = input_frame.decode_sample(wire)
            t.eq(value, nil)
            t.eq(code, "malformed")
        end
    end)

    t.it("encodes and decodes one byte-for-byte canonical frame", function()
        local slots = neutral_slots()
        slots[1] = assert(input_frame.new_sample({
            move_x = -127,
            move_y = 64,
            held = input_frame.HELD_BITS.shoot + input_frame.HELD_BITS.lob,
            edges = input_frame.EDGE_BITS.shoot,
        }))
        slots[8] = assert(input_frame.new_sample({
            move_x = 127,
            move_y = -64,
            held = input_frame.HELD_BITS.equipment,
            edges = input_frame.EDGE_BITS.equipment_pressed,
        }))
        local frame = assert(input_frame.new(42, slots))
        local wire = assert(input_frame.encode(frame))
        t.eq(
            wire,
            "3|42|-127,64,17,1,255|0,0,0,0,255|0,0,0,0,255|0,0,0,0,255|0,0,0,0,255"
                .. "|0,0,0,0,255|0,0,0,0,255|127,-64,128,32,255"
        )

        local decoded = assert(input_frame.decode(wire))
        local reencoded = assert(input_frame.encode(decoded))
        t.eq(reencoded, wire)
        t.eq(decoded.tick, 42)
        t.eq(decoded.slots[1].move_x, -127)
        t.eq(decoded.slots[8].edges, input_frame.EDGE_BITS.equipment_pressed)
    end)

    t.it("rejects malformed, noncanonical, and oversized frame data", function()
        local frame = assert(input_frame.neutral(0))
        frame.slots[9] = input_frame.neutral_sample()
        local ok, _, code = input_frame.validate(frame)
        t.eq(ok, nil)
        t.eq(code, "malformed")

        local sample, _, sample_code = input_frame.new_sample({ held = 256 })
        t.eq(sample, nil)
        t.eq(sample_code, "malformed")
        sample, _, sample_code = input_frame.new_sample({ edges = 128 })
        t.eq(sample, nil)
        t.eq(sample_code, "malformed")
        sample, _, sample_code = input_frame.new_sample({ aim = 256 })
        t.eq(sample, nil)
        t.eq(sample_code, "malformed")
        sample, _, sample_code = input_frame.new_sample({ aim = -1 })
        t.eq(sample, nil)
        t.eq(sample_code, "malformed")

        local invalid_combinations = {
            {
                held = 0,
                edges = input_frame.EDGE_BITS.equipment_pressed,
            },
            {
                held = input_frame.HELD_BITS.equipment,
                edges = input_frame.EDGE_BITS.equipment_released,
            },
            {
                held = input_frame.HELD_BITS.equipment,
                edges = input_frame.EDGE_BITS.equipment_pressed
                    + input_frame.EDGE_BITS.equipment_released,
            },
        }
        for _, invalid in ipairs(invalid_combinations) do
            local rejected, _, rejected_code = input_frame.new_sample(invalid)
            t.eq(rejected, nil)
            t.eq(rejected_code, "malformed")
        end

        local wire = assert(input_frame.encode(assert(input_frame.neutral(0))))
        local old, _, old_code = input_frame.decode("1" .. wire:sub(2))
        t.eq(old, nil)
        t.eq(old_code, "unsupported_version")
        local value, _, decode_code = input_frame.decode("01" .. wire:sub(2))
        t.eq(value, nil)
        t.eq(decode_code, "malformed")
        value, _, decode_code = input_frame.decode(wire:gsub("0,0,0,0", "-0,0,0,0", 1))
        t.eq(value, nil)
        t.eq(decode_code, "malformed")
        value, _, decode_code = input_frame.decode(wire .. "x")
        t.eq(value, nil)
        t.eq(decode_code, "malformed")

        local maximum_slots = neutral_slots()
        for index = 1, input_frame.SLOT_COUNT do
            maximum_slots[index] = assert(input_frame.new_sample({
                move_x = -127,
                move_y = -127,
                held = 127,
                edges = 127,
            }))
        end
        local maximum_wire =
            assert(input_frame.encode(assert(input_frame.new(input_frame.MAX_TICK, maximum_slots))))
        t.eq(#maximum_wire, input_frame.MAX_WIRE_BYTES)
        value, _, decode_code = input_frame.decode(maximum_wire .. "x")
        t.eq(value, nil)
        t.eq(decode_code, "wire_too_large")
    end)
    -- A sign flip here mirrors every aimed pass across the halfway line and
    -- breaks nothing else, so the convention is pinned rather than inferred.
    t.it("pins the aim angle convention at zero and a quarter turn", function()
        local zero_x, zero_y = assert(input_frame.dequantize_aim(0))
        t.eq(zero_x, 1, "aim code zero points toward increasing x, the way home attacks")
        t.near(zero_y, 0)

        local quarter_x, quarter_y = assert(input_frame.dequantize_aim(64))
        t.near(quarter_x, 0, 0.02)
        t.near(quarter_y, 1, 0.02)
        t.is_true(quarter_y > 0, "increasing aim codes wind from +x toward +y, down the screen")

        t.eq(assert(input_frame.quantize_aim(1, 0)), 0)
        t.eq(assert(input_frame.quantize_aim(0, 1)), 64)
        t.eq(assert(input_frame.quantize_aim(-1, 0)), 128)
        t.eq(
            assert(input_frame.quantize_aim(7, 0)),
            0,
            "only the angle survives; magnitude is discarded"
        )
    end)

    t.it("round-trips every aim direction code and refuses the sentinel a direction", function()
        for code = 0, input_frame.AIM_STEPS - 1 do
            local x, y = assert(input_frame.dequantize_aim(code))
            t.eq(assert(input_frame.quantize_aim(x, y)), code, "aim code " .. code)
        end

        local none_x, _, _, none_code = input_frame.dequantize_aim(input_frame.AIM_NONE)
        t.eq(none_x, nil, "AIM_NONE names no direction and must not decode to one")
        t.eq(none_code, "malformed")

        t.eq(
            assert(input_frame.quantize_aim(0, 0)),
            input_frame.AIM_NONE,
            "a zero-length aim is the absence of a direction, not the zero direction"
        )
        local bad, _, bad_code = input_frame.quantize_aim(0 / 0, 1)
        t.eq(bad, nil)
        t.eq(bad_code, "malformed")
    end)

    t.it("carries aim across the full frame wire", function()
        local slots = neutral_slots()
        slots[3] = assert(input_frame.new_sample({ move_x = 127, aim = 64 }))
        local frame = assert(input_frame.new(9, slots))
        local wire = assert(input_frame.encode(frame))
        local decoded = assert(input_frame.decode(wire))
        t.eq(decoded.slots[3].aim, 64)
        t.eq(decoded.slots[3].move_x, 127, "aim and movement are independent channels")
        t.eq(decoded.slots[4].aim, input_frame.AIM_NONE)
        t.eq(assert(input_frame.encode(decoded)), wire)

        local copied = assert(input_frame.copy(frame))
        t.eq(copied.slots[3].aim, 64)
    end)
end)
