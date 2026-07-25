-- Immutable-by-construction recorded input tapes.
--
-- A tape owns deep copies of its initial start-of-tick snapshot, materialized
-- effective InputFrames, identity, and boundary hashes. It never records bot
-- policy or producer RNG: those have already been materialized into frames.

local fixed_clock = require("sim.fixed_clock")
local combat_identity = require("sim.combat_identity")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local tuning = require("sim.tuning")

---@class InputTapeIdentity
---@field tape_version integer
---@field input_version integer
---@field snapshot_version integer
---@field build string
---@field source string
---@field content string
---@field tuning string
---@field config string
---@field fixture string
---@field seed integer
---@field tick_rate integer
---@field ownership InputOwnership
---@field combat string?

---@class InputTape
---@field version integer
---@field identity InputTapeIdentity
---@field initial MatchSnapshot
---@field frames InputFrame[]
---@field boundary_hashes string[]

---@class InputTapeIdentityDifference
---@field path string
---@field expected any
---@field actual any

---@class InputTapeModule
local input_tape = {}

input_tape.VERSION = 1
input_tape.COMBAT_VERSION = 2

input_tape.IDENTITY_FIELDS = {
    "tape_version",
    "input_version",
    "snapshot_version",
    "build",
    "source",
    "content",
    "tuning",
    "config",
    "fixture",
    "seed",
    "tick_rate",
    "ownership",
    "combat",
}

local IDENTITY_FIELD_SET = {}
for _, field in ipairs(input_tape.IDENTITY_FIELDS) do
    IDENTITY_FIELD_SET[field] = true
end

local TAPE_FIELD_SET = {
    version = true,
    identity = true,
    initial = true,
    frames = true,
    boundary_hashes = true,
}

local OWNERSHIP_FIELD_SET = {
    version = true,
    rosters = true,
    slots = true,
}

local TEAM_FIELD_SET = {
    home = true,
    away = true,
}

local ASSIGNMENT_FIELD_SET = {
    slot = true,
    team = true,
    player_id = true,
}

---@param value any
---@param allowed table<string, boolean>
---@param path string
local function assert_fields(value, allowed, path)
    assert(type(value) == "table", path .. " must be a table")
    for key in pairs(value) do
        assert(
            type(key) == "string" and allowed[key],
            path .. " has unknown field " .. tostring(key)
        )
    end
end

---@param value any
---@param path string
---@param expected integer?
local function assert_array(value, path, expected)
    assert(type(value) == "table", path .. " must be an array")
    local count = #value
    if expected then
        assert(count == expected, path .. " has the wrong length")
    end
    for key in pairs(value) do
        assert(
            type(key) == "number" and key == math.floor(key) and key >= 1 and key <= count,
            path .. " is not a canonical array"
        )
    end
end

---@param ownership any
---@param path string
---@return InputOwnership
local function copy_ownership(ownership, path)
    assert_fields(ownership, OWNERSHIP_FIELD_SET, path)
    assert(ownership.version == input_frame.VERSION, path .. " version is unsupported")
    assert_fields(ownership.rosters, TEAM_FIELD_SET, path .. ".rosters")
    assert_array(ownership.rosters.home, path .. ".rosters.home", input_frame.FIXTURE_TEAM_SIZE)
    assert_array(ownership.rosters.away, path .. ".rosters.away", input_frame.FIXTURE_TEAM_SIZE)
    assert_array(ownership.slots, path .. ".slots", input_frame.SLOT_COUNT)
    local result = {
        version = ownership.version,
        rosters = { home = {}, away = {} },
        slots = {},
    }
    for _, team in ipairs({ "home", "away" }) do
        for index = 1, input_frame.FIXTURE_TEAM_SIZE do
            local player_id = ownership.rosters[team][index]
            assert(type(player_id) == "string", path .. " roster id must be a string")
            result.rosters[team][index] = player_id
        end
    end
    for index = 1, input_frame.SLOT_COUNT do
        local assignment = ownership.slots[index]
        assert_fields(assignment, ASSIGNMENT_FIELD_SET, path .. ".slots." .. index)
        result.slots[index] = {
            slot = assignment.slot,
            team = assignment.team,
            player_id = assignment.player_id,
        }
    end
    return result
end

---@param ownership InputOwnership
---@param state MatchState
---@param path string
local function assert_ownership_routes(ownership, state, path)
    local player_index_by_id = {}
    for index, player in ipairs(state.players) do
        assert(not player_index_by_id[player.id], path .. " match player ids are not unique")
        player_index_by_id[player.id] = index
    end

    local fixture_players = {}
    local roster_members = { home = {}, away = {} }
    local roster_count = 0
    for _, team in ipairs({ "home", "away" }) do
        local keeper_count = 0
        for index = 1, input_frame.FIXTURE_TEAM_SIZE do
            local player_id = ownership.rosters[team][index]
            local player_index =
                assert(player_index_by_id[player_id], path .. " roster player is not in the match")
            local player = state.players[player_index]
            assert(player.team == team, path .. " roster player belongs to the other match side")
            assert(not fixture_players[player_id], path .. " fixture roster player is duplicated")
            fixture_players[player_id] = true
            roster_members[team][player_id] = true
            roster_count = roster_count + 1
            if player.is_keeper then
                keeper_count = keeper_count + 1
            end
        end
        assert(keeper_count == 1, path .. " fixture side must contain exactly one keeper")
    end
    assert(roster_count == #state.players, path .. " fixture rosters do not cover the match")

    local assigned_players = {}
    for index = 1, input_frame.SLOT_COUNT do
        local expected_slot = assert(input_frame.slot(index))
        local assignment = ownership.slots[index]
        assert(
            assignment.slot == expected_slot.id and assignment.team == expected_slot.team,
            path .. " assignments violate canonical slot order"
        )
        assert(
            roster_members[expected_slot.team][assignment.player_id],
            path .. " assignment is not a member of its fixture side"
        )
        local player_index = assert(
            player_index_by_id[assignment.player_id],
            path .. " assignment player is not in the match"
        )
        local player = state.players[player_index]
        assert(not player.is_keeper, path .. " keeper cannot own an input slot")
        assert(not assigned_players[player_index], path .. " player owns multiple input slots")
        assigned_players[player_index] = true
        assert(
            state.slot_players[index] == player_index,
            path .. " assignment disagrees with slot_players routing"
        )
        assert(
            state.slot_for_player[player_index] == index,
            path .. " assignment disagrees with slot_for_player routing"
        )
    end
    for index, player in ipairs(state.players) do
        if player.is_keeper then
            assert(
                state.slot_for_player[index] == nil,
                path .. " keeper appears in inverse slot routing"
            )
        else
            assert(assigned_players[index], path .. " outfielder has no input assignment")
        end
    end
end

---@param identity any
---@return InputTapeIdentity
function input_tape.copy_identity(identity)
    assert_fields(identity, IDENTITY_FIELD_SET, "identity")
    assert(
        identity.tape_version == input_tape.VERSION
            or identity.tape_version == input_tape.COMBAT_VERSION,
        "unsupported input tape identity version"
    )
    assert(identity.input_version == input_frame.VERSION, "unsupported identity input version")
    assert(
        identity.snapshot_version == match_snapshot.VERSION
            or identity.snapshot_version == match_snapshot.COMBAT_VERSION,
        "unsupported identity snapshot version"
    )
    for _, field in ipairs({ "build", "source", "content", "tuning", "config", "fixture" }) do
        assert(type(identity[field]) == "string", "identity." .. field .. " must be a string")
    end
    for _, field in ipairs({ "build", "source", "content", "config", "fixture" }) do
        assert(identity[field] ~= "", "identity." .. field .. " must not be empty")
    end
    assert(
        type(identity.seed) == "number"
            and identity.seed == math.floor(identity.seed)
            and identity.seed == identity.seed
            and identity.seed ~= math.huge
            and identity.seed ~= -math.huge,
        "identity.seed must be a finite integer"
    )
    assert(identity.tick_rate == fixed_clock.TICK_RATE, "identity tick rate is unsupported")
    if identity.tape_version == input_tape.VERSION then
        assert(
            identity.snapshot_version == match_snapshot.VERSION,
            "soccer tape identity requires the soccer snapshot version"
        )
        assert(identity.combat == nil, "soccer tape identity cannot contain combat identity")
    else
        assert(
            identity.snapshot_version == match_snapshot.COMBAT_VERSION,
            "combat tape identity requires the combat snapshot version"
        )
        assert(
            type(identity.combat) == "string" and identity.combat ~= "",
            "combat tape identity is required"
        )
    end
    return {
        tape_version = identity.tape_version,
        input_version = identity.input_version,
        snapshot_version = identity.snapshot_version,
        build = identity.build,
        source = identity.source,
        content = identity.content,
        tuning = identity.tuning,
        config = identity.config,
        fixture = identity.fixture,
        seed = identity.seed,
        tick_rate = identity.tick_rate,
        ownership = copy_ownership(identity.ownership, "identity.ownership"),
        combat = identity.combat,
    }
end

---@param path string
---@param expected any
---@param actual any
---@return InputTapeIdentityDifference
local function identity_difference(path, expected, actual)
    return { path = path, expected = expected, actual = actual }
end

---@param expected InputTapeIdentity
---@param actual InputTapeIdentity
---@return InputTapeIdentityDifference?
function input_tape.identity_difference(expected, actual)
    local left = input_tape.copy_identity(expected)
    local right = input_tape.copy_identity(actual)
    for _, field in ipairs(input_tape.IDENTITY_FIELDS) do
        if field ~= "ownership" and left[field] ~= right[field] then
            return identity_difference("identity." .. field, left[field], right[field])
        end
    end
    local a, b = left.ownership, right.ownership
    if a.version ~= b.version then
        return identity_difference("identity.ownership.version", a.version, b.version)
    end
    for _, team in ipairs({ "home", "away" }) do
        for index = 1, input_frame.FIXTURE_TEAM_SIZE do
            if a.rosters[team][index] ~= b.rosters[team][index] then
                return identity_difference(
                    "identity.ownership.rosters." .. team .. "." .. index,
                    a.rosters[team][index],
                    b.rosters[team][index]
                )
            end
        end
    end
    for index = 1, input_frame.SLOT_COUNT do
        for _, field in ipairs({ "slot", "team", "player_id" }) do
            if a.slots[index][field] ~= b.slots[index][field] then
                return identity_difference(
                    "identity.ownership.slots." .. index .. "." .. field,
                    a.slots[index][field],
                    b.slots[index][field]
                )
            end
        end
    end
    return nil
end

---@param identity InputTapeIdentity
---@param initial MatchSnapshot
---@param frames InputFrame[]
---@return InputTapeIdentity
---@return MatchState
---@return CombatMatchState?
---@return MatchSnapshot
---@return InputFrame[]
local function copy_components(identity, initial, frames)
    local copied_identity = input_tape.copy_identity(identity)
    assert(
        copied_identity.tuning == tuning.serialize(),
        "identity tuning does not match active simulation tuning"
    )
    local initial_state, initial_combat = match_snapshot.restore(initial)
    assert(initial_state.slot_mode, "input tapes require a fixed-slot match")
    assert(initial_state.input_ownership, "input tape snapshot has no ownership")
    assert_ownership_routes(copied_identity.ownership, initial_state, "identity.ownership")
    local snapshot_identity = {
        tape_version = initial_combat and input_tape.COMBAT_VERSION or input_tape.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = initial.version,
        build = copied_identity.build,
        source = copied_identity.source,
        content = copied_identity.content,
        tuning = copied_identity.tuning,
        config = copied_identity.config,
        fixture = copied_identity.fixture,
        seed = copied_identity.seed,
        tick_rate = copied_identity.tick_rate,
        ownership = initial_state.input_ownership,
        combat = initial_combat and combat_identity.for_state(initial_combat) or nil,
    }
    local ownership_diff = input_tape.identity_difference(copied_identity, snapshot_identity)
    assert(
        not ownership_diff,
        ownership_diff and ("tape " .. ownership_diff.path .. " differs from snapshot") or ""
    )
    assert_array(frames, "frames")
    local copied_frames = {}
    for index = 1, #frames do
        local frame = assert(input_frame.copy(frames[index]))
        assert(
            frame.tick == initial_state.input_tick + index - 1,
            "input tape frames must be contiguous from the snapshot boundary"
        )
        copied_frames[index] = frame
    end
    local normalized_initial = match_snapshot.capture(initial_state, initial_combat)
    return copied_identity, initial_state, initial_combat, normalized_initial, copied_frames
end

---@param identity InputTapeIdentity
---@param initial MatchSnapshot
---@param frames InputFrame[]
---@return InputTape
function input_tape.new(identity, initial, frames)
    local copied_identity, _, _, normalized_initial, copied_frames =
        copy_components(identity, initial, frames)
    local boundary_hashes = { match_snapshot.hash(normalized_initial) }
    local replay_state, replay_combat = match_snapshot.restore(normalized_initial)
    for index = 1, #copied_frames do
        assert(not replay_state.finished, "input tape contains a frame after the match finished")
        match.step(replay_state, fixed_clock.TICK_SECONDS, copied_frames[index], replay_combat)
        boundary_hashes[index + 1] =
            match_snapshot.hash(match_snapshot.capture(replay_state, replay_combat))
    end
    return {
        version = copied_identity.tape_version,
        identity = copied_identity,
        initial = normalized_initial,
        frames = copied_frames,
        boundary_hashes = boundary_hashes,
    }
end

-- Construct a tape from a checked-in recording whose complete boundary-hash
-- sequence is verified independently. This validates/copies every structural
-- input but deliberately avoids replaying the full match during startup.
---@param identity InputTapeIdentity
---@param initial MatchSnapshot
---@param frames InputFrame[]
---@param boundary_hashes string[]
---@return InputTape
function input_tape.from_frozen_recording(identity, initial, frames, boundary_hashes)
    local copied_identity, _, _, normalized_initial, copied_frames =
        copy_components(identity, initial, frames)
    assert_array(boundary_hashes, "boundary_hashes", #copied_frames + 1)
    local copied_hashes = {}
    for index = 1, #boundary_hashes do
        local hash = boundary_hashes[index]
        assert(
            type(hash) == "string" and hash:match("^[0-9a-f]+$") and #hash == 16,
            "frozen input tape boundary hash is malformed"
        )
        copied_hashes[index] = hash
    end
    assert(
        copied_hashes[1] == match_snapshot.hash(normalized_initial),
        "frozen input tape initial boundary hash disagrees with its snapshot"
    )
    return {
        version = copied_identity.tape_version,
        identity = copied_identity,
        initial = normalized_initial,
        frames = copied_frames,
        boundary_hashes = copied_hashes,
    }
end

---@param tape InputTape
---@return boolean
function input_tape.validate_structure(tape)
    assert_fields(tape, TAPE_FIELD_SET, "tape")
    assert(
        tape.version == input_tape.VERSION or tape.version == input_tape.COMBAT_VERSION,
        "unsupported input tape version"
    )
    local identity = input_tape.copy_identity(tape.identity)
    assert(tape.version == identity.tape_version, "tape and identity versions differ")
    local state, combat_state = match_snapshot.restore(tape.initial)
    assert(state.slot_mode, "input tape snapshot is not a fixed-slot match")
    assert(state.input_ownership, "input tape snapshot has no ownership")
    local snapshot_identity = input_tape.copy_identity(tape.identity)
    snapshot_identity.ownership =
        copy_ownership(state.input_ownership, "tape.initial.state.input_ownership")
    snapshot_identity.snapshot_version = tape.initial.version
    snapshot_identity.tape_version = combat_state and input_tape.COMBAT_VERSION
        or input_tape.VERSION
    snapshot_identity.combat = combat_state and combat_identity.for_state(combat_state) or nil
    local ownership_diff = input_tape.identity_difference(tape.identity, snapshot_identity)
    assert(
        not ownership_diff,
        ownership_diff and ("tape " .. ownership_diff.path .. " differs from snapshot") or ""
    )
    assert_ownership_routes(tape.identity.ownership, state, "tape.identity.ownership")
    assert_array(tape.frames, "tape.frames")
    assert_array(tape.boundary_hashes, "tape.boundary_hashes", #tape.frames + 1)
    for index = 1, #tape.frames do
        assert(input_frame.validate(tape.frames[index]))
        assert(
            tape.frames[index].tick == state.input_tick + index - 1,
            "input tape frames are not contiguous"
        )
    end
    for index = 1, #tape.boundary_hashes do
        assert(
            type(tape.boundary_hashes[index]) == "string"
                and tape.boundary_hashes[index]:match("^[0-9a-f]+$")
                and #tape.boundary_hashes[index] == 16,
            "input tape boundary hash is malformed"
        )
    end
    assert(
        tape.boundary_hashes[1] == match_snapshot.hash(tape.initial),
        "input tape initial boundary hash disagrees with its snapshot"
    )
    return true
end

---@param tape InputTape
---@return boolean
function input_tape.validate(tape)
    assert(input_tape.validate_structure(tape))
    local state, combat_state = match_snapshot.restore(tape.initial)
    for index = 1, #tape.frames do
        assert(not state.finished, "input tape contains a frame after the match finished")
        match.step(state, fixed_clock.TICK_SECONDS, tape.frames[index], combat_state)
    end
    return true
end

return input_tape
