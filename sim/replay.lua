-- Pure input-tape replay and first-divergence diagnostics.

local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local tuning = require("sim.tuning")

---@class ReplayBoundary
---@field tick integer
---@field hash string

---@class ReplayDivergence
---@field causal_input_tick integer?
---@field boundary_tick integer
---@field expected_hash string
---@field actual_hash string
---@field state_path string
---@field expected_state any
---@field actual_state any
---@field expected_input string?
---@field actual_input string?

---@class ReplayResult
---@field state MatchState
---@field combat_state CombatMatchState?
---@field boundaries ReplayBoundary[]
---@field divergence ReplayDivergence?

---@class ReplayComparison
---@field equal boolean
---@field expected ReplayResult
---@field actual ReplayResult
---@field divergence ReplayDivergence?

---@alias ReplayFailureCode "malformed"|"identity_mismatch"

---@class ReplayFailure
---@field code ReplayFailureCode
---@field message string
---@field path string?
---@field expected any
---@field actual any

---@class ReplayModule
local replay = {}

---@param message string
---@return nil, ReplayFailure
local function malformed(message)
    return nil, { code = "malformed", message = message }
end

---@param path string
---@param expected any
---@param actual any
---@return nil, ReplayFailure
local function identity_failure(path, expected, actual)
    return nil,
        {
            code = "identity_mismatch",
            message = "replay identity mismatch at " .. path,
            path = path,
            expected = expected,
            actual = actual,
        }
end

---@param tape InputTape
---@param expected_identity InputTapeIdentity
---@return boolean?, ReplayFailure?
local function validate_context(tape, expected_identity)
    if type(tape) ~= "table" then
        return malformed("input tape must be a table")
    end
    local expected_ok, copied_expected = pcall(input_tape.copy_identity, expected_identity)
    if not expected_ok then
        return malformed(tostring(copied_expected))
    end
    ---@cast copied_expected InputTapeIdentity
    if
        type(tape.identity) == "table"
        and type(tape.identity.input_version) == "number"
        and tape.identity.input_version ~= copied_expected.input_version
    then
        return identity_failure(
            "identity.input_version",
            copied_expected.input_version,
            tape.identity.input_version
        )
    end
    local tape_identity_ok, tape_identity = pcall(input_tape.copy_identity, tape.identity)
    if not tape_identity_ok then
        return malformed(tostring(tape_identity))
    end
    ---@cast tape_identity InputTapeIdentity
    local difference = input_tape.identity_difference(copied_expected, tape_identity)
    if difference then
        return identity_failure(difference.path, difference.expected, difference.actual)
    end
    local active_tuning = tuning.serialize()
    if active_tuning ~= tape_identity.tuning then
        return identity_failure("identity.tuning", tape_identity.tuning, active_tuning)
    end
    -- Full tape validation may step a restored state to prove that every frame
    -- is consumable. Identity and active tuning must be accepted before that
    -- simulation-aware path so configuration mismatches cannot be mislabeled.
    local tape_ok, tape_err = pcall(input_tape.validate, tape)
    if not tape_ok then
        return malformed(tostring(tape_err))
    end
    return true
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@return ReplayBoundary
local function boundary(state, combat_state)
    local snapshot = match_snapshot.capture(state, combat_state)
    return { tick = state.input_tick, hash = match_snapshot.hash(snapshot) }
end

---@param state MatchState
---@param expected_hash string
---@param actual_hash string
---@param causal_input_tick integer?
---@param actual_input string?
---@return ReplayDivergence
local function self_divergence(state, expected_hash, actual_hash, causal_input_tick, actual_input)
    return {
        causal_input_tick = causal_input_tick,
        boundary_tick = state.input_tick,
        expected_hash = expected_hash,
        actual_hash = actual_hash,
        state_path = "unavailable_without_reference_tape",
        expected_state = nil,
        actual_state = nil,
        expected_input = nil,
        actual_input = actual_input,
    }
end

---@param tape InputTape
---@param expected_identity InputTapeIdentity
---@return ReplayResult?, ReplayFailure?
function replay.run(tape, expected_identity)
    local valid, failure = validate_context(tape, expected_identity)
    if not valid then
        return nil, failure
    end
    local state, combat_state = match_snapshot.restore(tape.initial)
    local boundaries = { boundary(state, combat_state) }
    local divergence = nil
    if boundaries[1].hash ~= tape.boundary_hashes[1] then
        divergence = self_divergence(state, tape.boundary_hashes[1], boundaries[1].hash, nil, nil)
        return {
            state = state,
            combat_state = combat_state,
            boundaries = boundaries,
            divergence = divergence,
        }
    end
    for index = 1, #tape.frames do
        local frame = tape.frames[index]
        match.step(state, fixed_clock.TICK_SECONDS, frame, combat_state)
        boundaries[index + 1] = boundary(state, combat_state)
        if boundaries[index + 1].hash ~= tape.boundary_hashes[index + 1] then
            divergence = self_divergence(
                state,
                tape.boundary_hashes[index + 1],
                boundaries[index + 1].hash,
                frame.tick,
                assert(input_frame.encode(frame))
            )
            break
        end
    end
    return {
        state = state,
        combat_state = combat_state,
        boundaries = boundaries,
        divergence = divergence,
    }
end

---@param expected_state MatchState
---@param actual_state MatchState
---@param expected_combat CombatMatchState?
---@param actual_combat CombatMatchState?
---@param causal_input_tick integer?
---@param expected_input string?
---@param actual_input string?
---@return ReplayDivergence
local function compare_states(
    expected_state,
    actual_state,
    expected_combat,
    actual_combat,
    causal_input_tick,
    expected_input,
    actual_input
)
    local expected_snapshot = match_snapshot.capture(expected_state, expected_combat)
    local actual_snapshot = match_snapshot.capture(actual_state, actual_combat)
    local expected_hash = match_snapshot.hash(expected_snapshot)
    local actual_hash = match_snapshot.hash(actual_snapshot)
    local found = match_snapshot.first_difference(expected_snapshot, actual_snapshot)
    return {
        causal_input_tick = causal_input_tick,
        boundary_tick = actual_state.input_tick,
        expected_hash = expected_hash,
        actual_hash = actual_hash,
        state_path = found and found.path or "<canonical_hash>",
        expected_state = found and found.expected or nil,
        actual_state = found and found.actual or nil,
        expected_input = expected_input,
        actual_input = actual_input,
    }
end

---@param reference InputTape
---@param candidate InputTape
---@param expected_identity InputTapeIdentity
---@return ReplayComparison?, ReplayFailure?
function replay.compare(reference, candidate, expected_identity)
    local reference_valid, reference_failure = validate_context(reference, expected_identity)
    if not reference_valid then
        return nil, reference_failure
    end
    local candidate_valid, candidate_failure = validate_context(candidate, expected_identity)
    if not candidate_valid then
        return nil, candidate_failure
    end
    local identity_diff = input_tape.identity_difference(reference.identity, candidate.identity)
    if identity_diff then
        return identity_failure(identity_diff.path, identity_diff.expected, identity_diff.actual)
    end

    local expected_state, expected_combat = match_snapshot.restore(reference.initial)
    local actual_state, actual_combat = match_snapshot.restore(candidate.initial)
    local expected_boundaries = { boundary(expected_state, expected_combat) }
    local actual_boundaries = { boundary(actual_state, actual_combat) }
    local expected_result = {
        state = expected_state,
        combat_state = expected_combat,
        boundaries = expected_boundaries,
    }
    local actual_result = {
        state = actual_state,
        combat_state = actual_combat,
        boundaries = actual_boundaries,
    }
    if expected_boundaries[1].hash ~= actual_boundaries[1].hash then
        local divergence = compare_states(
            expected_state,
            actual_state,
            expected_combat,
            actual_combat,
            nil,
            nil,
            nil
        )
        expected_result.divergence = divergence
        actual_result.divergence = divergence
        return {
            equal = false,
            expected = expected_result,
            actual = actual_result,
            divergence = divergence,
        }
    end

    local count = math.min(#reference.frames, #candidate.frames)
    for index = 1, count do
        local expected_frame = reference.frames[index]
        local actual_frame = candidate.frames[index]
        local expected_wire = assert(input_frame.encode(expected_frame))
        local actual_wire = assert(input_frame.encode(actual_frame))
        match.step(expected_state, fixed_clock.TICK_SECONDS, expected_frame, expected_combat)
        match.step(actual_state, fixed_clock.TICK_SECONDS, actual_frame, actual_combat)
        expected_boundaries[index + 1] = boundary(expected_state, expected_combat)
        actual_boundaries[index + 1] = boundary(actual_state, actual_combat)
        if expected_boundaries[index + 1].hash ~= actual_boundaries[index + 1].hash then
            local divergence = compare_states(
                expected_state,
                actual_state,
                expected_combat,
                actual_combat,
                expected_frame.tick,
                expected_wire,
                actual_wire
            )
            expected_result.divergence = divergence
            actual_result.divergence = divergence
            return {
                equal = false,
                expected = expected_result,
                actual = actual_result,
                divergence = divergence,
            }
        end
    end

    if #reference.frames ~= #candidate.frames then
        local next_index = count + 1
        local expected_frame = reference.frames[next_index]
        local actual_frame = candidate.frames[next_index]
        local causal_tick = expected_frame and expected_frame.tick
            or (actual_frame and actual_frame.tick or expected_state.input_tick)
        local divergence = {
            causal_input_tick = causal_tick,
            boundary_tick = actual_state.input_tick,
            expected_hash = expected_boundaries[#expected_boundaries].hash,
            actual_hash = actual_boundaries[#actual_boundaries].hash,
            state_path = "frames.length",
            expected_state = #reference.frames,
            actual_state = #candidate.frames,
            expected_input = expected_frame and assert(input_frame.encode(expected_frame)) or nil,
            actual_input = actual_frame and assert(input_frame.encode(actual_frame)) or nil,
        }
        expected_result.divergence = divergence
        actual_result.divergence = divergence
        return {
            equal = false,
            expected = expected_result,
            actual = actual_result,
            divergence = divergence,
        }
    end

    return {
        equal = true,
        expected = expected_result,
        actual = actual_result,
        divergence = nil,
    }
end

return replay
