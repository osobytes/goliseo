local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")
local tuning = require("sim.tuning")

---@class ShortMatchTapeFixture
local short_match_tape = {}

short_match_tape.EXPECTED_BOUNDARY_HASHES = {
    "445f4c782befc911",
    "beab813fee150eda",
    "addb27376b04012a",
    "29b04e232a262fff",
}

---@return InputTape tape
---@return InputTapeIdentity identity
function short_match_tape.make()
    local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = fixed_clock.TICK_SECONDS * 2.5,
        max_goals = 3,
        seed = 38,
        input_ownership = ownership,
    })
    local frames = {
        assert(input_frame.neutral(0)),
        assert(input_frame.neutral(1)),
        assert(input_frame.neutral(2)),
    }
    frames[2].slots[1] = assert(input_frame.new_sample({ move_x = input_frame.MOVE_SCALE }))
    local identity = {
        tape_version = input_tape.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = match_snapshot.VERSION,
        build = "spec-build",
        source = "197ce23264abff1d6ac0c71e9b313ee820814f7e",
        content = "showcase-content-v1",
        tuning = tuning.serialize(),
        config = "field=960x540;duration_ticks=2.5;max_goals=3",
        fixture = "nebula-v-orion;balanced-v-balanced",
        seed = 38,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = ownership,
    }
    local tape = input_tape.new(identity, match_snapshot.capture(state), frames)
    assert(#tape.boundary_hashes == #short_match_tape.EXPECTED_BOUNDARY_HASHES)
    for index, expected in ipairs(short_match_tape.EXPECTED_BOUNDARY_HASHES) do
        assert(
            tape.boundary_hashes[index] == expected,
            ("short tape boundary %d hash drifted: expected %s, got %s"):format(
                index - 1,
                expected,
                tape.boundary_hashes[index]
            )
                .. "; observed sequence="
                .. table.concat(tape.boundary_hashes, ",")
        )
    end
    return tape, identity
end

return short_match_tape
