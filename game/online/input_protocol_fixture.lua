local input_protocol = require("game.online.input_protocol")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local input_frame = require("sim.input_frame")

---@class InputProtocolFixtureModule
local fixture = {}

---@param tick integer
---@param slot_index integer
---@param sample InputSample
---@return InputAuthorityRow
local function row(tick, slot_index, sample)
    return { tick = tick, slot_index = slot_index, sample = sample }
end

---@return InputPacket
function fixture.guest()
    local manifest = protocol_fixture.manifest()
    return assert(input_protocol.new_guest({
        session_id = manifest.session_id,
        manifest_id = protocol.manifest_id(manifest),
        sender_id = "guest_2",
        sequence = 7,
        transport_tick = 12,
        first_input_tick = 0,
        rows = {
            row(0, 2, input_frame.neutral_sample()),
            row(
                1,
                2,
                assert(input_frame.new_sample({
                    move_x = -127,
                    move_y = 127,
                    held = input_frame.HELD_BITS.shoot + input_frame.HELD_BITS.sprint,
                    edges = input_frame.EDGE_BITS.shoot + input_frame.EDGE_BITS.dash,
                }))
            ),
            row(
                2,
                2,
                assert(input_frame.new_sample({
                    held = input_frame.HELD_BITS.equipment,
                    edges = input_frame.EDGE_BITS.equipment_pressed,
                }))
            ),
            row(
                3,
                2,
                assert(input_frame.new_sample({
                    held = input_frame.HELD_BITS.equipment,
                }))
            ),
            row(
                4,
                2,
                assert(input_frame.new_sample({
                    edges = input_frame.EDGE_BITS.equipment_released,
                }))
            ),
            row(
                5,
                2,
                assert(input_frame.new_sample({
                    held = 127,
                    edges = 31,
                }))
            ),
            row(
                6,
                2,
                assert(input_frame.new_sample({
                    move_x = 127,
                    move_y = -127,
                    held = input_frame.HELD_BITS.pass + input_frame.HELD_BITS.lob,
                    edges = input_frame.EDGE_BITS.pass
                        + input_frame.EDGE_BITS.switch
                        + input_frame.EDGE_BITS.dodge,
                }))
            ),
        },
    }))
end

---@return InputPacket
function fixture.host()
    local manifest = protocol_fixture.manifest()
    local rows = {}
    for tick = 5, 6 do
        for slot_index = 1, input_frame.SLOT_COUNT do
            local held = slot_index == 8 and input_frame.HELD_BITS.equipment or (slot_index - 1)
            local edges = slot_index == 8 and input_frame.EDGE_BITS.equipment_pressed
                or (slot_index - 1)
            rows[#rows + 1] = row(
                tick,
                slot_index,
                assert(input_frame.new_sample({
                    move_x = tick * 10 - slot_index,
                    move_y = slot_index - tick * 10,
                    held = held,
                    edges = edges,
                }))
            )
        end
    end
    return assert(input_protocol.new_host({
        session_id = manifest.session_id,
        manifest_id = protocol.manifest_id(manifest),
        sender_id = "host",
        sequence = 13,
        transport_tick = 15,
        first_input_tick = 0,
        rows = rows,
    }))
end

---@return InputPacket
function fixture.maximal()
    local rows = {}
    for tick = input_frame.MAX_TICK - input_protocol.HISTORY_ROWS, input_frame.MAX_TICK do
        for slot_index = 1, input_frame.SLOT_COUNT do
            rows[#rows + 1] = row(
                tick,
                slot_index,
                assert(input_frame.new_sample({
                    move_x = 127,
                    move_y = -127,
                    held = 255,
                    edges = input_frame.EDGE_BITS.equipment_pressed,
                }))
            )
        end
    end
    return assert(input_protocol.new_host({
        session_id = string.rep("s", 128),
        manifest_id = "0123456789abcdef",
        sender_id = string.rep("h", 128),
        sequence = 2147483647,
        transport_tick = input_frame.MAX_TICK,
        first_input_tick = input_frame.MAX_TICK - input_protocol.HISTORY_ROWS,
        rows = rows,
    }))
end

return fixture
