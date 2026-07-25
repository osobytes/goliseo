local t = require("spec.support.runner")
local short_match_tape = require("spec.fixtures.short_match_tape")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_identity = require("sim.combat_identity")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local replay = require("sim.replay")
local teams = require("data.teams")
local tuning = require("sim.tuning")

---@param tape InputTape
---@return InputFrame[]
local function copy_frames(tape)
    local frames = {}
    for index, frame in ipairs(tape.frames) do
        frames[index] = assert(input_frame.copy(frame))
    end
    return frames
end

t.describe("input tape replay", function()
    t.it("pins combat mechanics in a v2 tape and replays every composite boundary", function()
        local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 2,
            max_goals = 3,
            seed = 811,
            input_ownership = ownership,
        })
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local source_player = nil
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "ranged" then
                source_player = index
                break
            end
        end
        source_player = assert(source_player, "fixture requires one ranged player")
        local source_slot = assert(state.slot_for_player[source_player])
        local frames = {}
        for tick = 0, 5 do
            frames[tick + 1] = assert(input_frame.neutral(tick))
        end
        frames[1].slots[source_slot] = assert(input_frame.new_sample({
            held = input_frame.HELD_BITS.equipment,
            edges = input_frame.EDGE_BITS.equipment_pressed,
        }))
        for index = 2, #frames do
            frames[index].slots[source_slot] = assert(input_frame.new_sample({
                held = input_frame.HELD_BITS.equipment,
            }))
        end
        local identity = {
            tape_version = input_tape.COMBAT_VERSION,
            input_version = input_frame.VERSION,
            snapshot_version = match_snapshot.COMBAT_VERSION,
            build = "combat-v2-spec",
            source = "combat-v2-spec",
            content = "nebula-orion-showcase-content-v1",
            tuning = tuning.serialize(),
            config = "field=960x540;duration=2;max_goals=3;tick_rate=60",
            fixture = "combat-v2-spec",
            seed = 811,
            tick_rate = fixed_clock.TICK_RATE,
            ownership = ownership,
            combat = combat_identity.for_state(combat_state),
        }
        local tape = input_tape.new(identity, match_snapshot.capture(state, combat_state), frames)
        local replayed = assert(replay.run(tape, identity))
        t.eq(tape.version, input_tape.COMBAT_VERSION)
        t.eq(#replayed.boundaries, #frames + 1)
        t.eq(assert(replayed.combat_state).players[source_player].phase, "windup")
        for index, row in ipairs(replayed.boundaries) do
            t.eq(row.hash, tape.boundary_hashes[index])
        end
        local mirror_state, mirror_combat = match_snapshot.restore(tape.initial)
        local mirror = input_tape.new(
            identity,
            match_snapshot.capture(mirror_state, mirror_combat),
            copy_frames(tape)
        )
        local compared = assert(replay.compare(tape, mirror, identity))
        t.is_true(compared.equal)
        for index, hash in ipairs(tape.boundary_hashes) do
            t.eq(mirror.boundary_hashes[index], hash)
        end

        local mismatched = input_tape.copy_identity(identity)
        mismatched.combat = mismatched.combat .. ";changed"
        local result, failure = replay.run(tape, mismatched)
        t.eq(result, nil)
        local mismatch = assert(failure)
        t.eq(mismatch.code, "identity_mismatch")
        t.eq(mismatch.path, "identity.combat")

        local soccer_identity = input_tape.copy_identity(identity)
        soccer_identity.tape_version = input_tape.VERSION
        soccer_identity.snapshot_version = match_snapshot.VERSION
        soccer_identity.combat = nil
        t.is_true(
            not pcall(
                input_tape.new,
                soccer_identity,
                match_snapshot.capture(state, combat_state),
                frames
            )
        )
    end)

    t.it("converges through a v5 goal, kickoff reset, and post-kickoff boundary", function()
        local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            duration = 2,
            max_goals = 3,
            seed = 83,
            input_ownership = ownership,
        })
        local away_keeper = state.players[6]
        away_keeper.keeper_state = "retreat"
        away_keeper.keeper_state_timer = 0.1
        away_keeper.keeper_release_state = "advance"
        away_keeper.keeper_release_motion = 0.5
        away_keeper.keeper_release_kind = "chip"
        away_keeper.keeper_release_depth = 42
        away_keeper.receive_timer = 1
        state.owner = nil
        state.ball = Vec2.new(965, 270)
        state.ball_vel = Vec2.new(600, 0)
        state.ball_z = 0
        state.ball_vz = 0
        state.pickup_cd = 1
        state.block_grace = 1

        local initial = match_snapshot.capture(state)
        local frames = {
            assert(input_frame.neutral(0)),
            assert(input_frame.neutral(1)),
            assert(input_frame.neutral(2)),
        }
        local identity = {
            tape_version = input_tape.VERSION,
            input_version = input_frame.VERSION,
            snapshot_version = match_snapshot.VERSION,
            build = "goal-kickoff-v5-spec",
            source = "synthetic-goal-kickoff-v1",
            content = "nebula-orion-showcase-content-v1",
            tuning = tuning.serialize(),
            config = "field=960x540;duration=2;max_goals=3;tick_rate=60",
            fixture = "goal-kickoff-v5-spec",
            seed = 83,
            tick_rate = fixed_clock.TICK_RATE,
            ownership = ownership,
        }
        local tape = input_tape.new(identity, initial, frames)
        local result, failure = replay.run(tape, identity)
        local replayed = assert(result, failure and failure.message)

        t.eq(#replayed.boundaries, 4)
        for index, boundary in ipairs(replayed.boundaries) do
            t.eq(
                boundary.hash,
                tape.boundary_hashes[index],
                "goal/kickoff boundary " .. (index - 1)
            )
        end
        t.eq(replayed.state.score.home, 1)
        t.is_true(replayed.state.kickoff_hold > 0)
        t.is_true(
            replayed.state.owner ~= nil
                and replayed.state.players[replayed.state.owner].team == "away"
        )
        t.eq(replayed.state.players[6].keeper_state, "base")
        t.eq(replayed.state.players[6].keeper_release_state, nil)
        t.eq(replayed.state.players[6].keeper_release_kind, nil)
        t.is_true(replayed.divergence == nil)

        local mirror = input_tape.new(
            identity,
            match_snapshot.capture(match_snapshot.restore(initial)),
            frames
        )
        local compared = assert(replay.compare(tape, mirror, identity))
        t.is_true(compared.equal)
        t.is_true(compared.divergence == nil)
    end)

    t.it("replays a short complete match with every boundary hash", function()
        local tape, identity = short_match_tape.make()
        local result, failure = replay.run(tape, identity)
        local replayed = assert(result, failure and failure.message)
        t.eq(#replayed.boundaries, #tape.frames + 1)
        for index = 1, #replayed.boundaries do
            local expected = short_match_tape.EXPECTED_BOUNDARY_HASHES[index]
            t.eq(tape.boundary_hashes[index], expected, "constructed tape boundary " .. (index - 1))
            t.eq(replayed.boundaries[index].hash, expected, "replay boundary " .. (index - 1))
        end
        t.is_true(replayed.state.finished, "the fixture reaches its match end")
        t.eq(replayed.state.input_tick, 3)
        t.is_true(replayed.divergence == nil)

        replayed.state.ball.x = -500
        local second = assert(replay.run(tape, identity))
        t.is_true(second.state.ball.x ~= -500, "each replay returns independent final state")
    end)

    t.it("deep-copies snapshot frames identity and ownership at construction", function()
        local tape, identity = short_match_tape.make()
        local frames = copy_frames(tape)
        local initial = match_snapshot.capture(match_snapshot.restore(tape.initial))
        local copied = input_tape.new(identity, initial, frames)
        frames[1].slots[1].move_x = -127
        initial.state.ball.x = -1
        identity.ownership.slots[1].player_id = "changed"
        identity.config = "changed"
        t.eq(copied.frames[1].slots[1].move_x, tape.frames[1].slots[1].move_x)
        t.is_true(copied.initial.state.ball.x ~= -1)
        t.is_true(copied.identity.ownership.slots[1].player_id ~= "changed")
        t.is_true(copied.identity.config ~= "changed")
    end)

    t.it("accepts and owns independently verified frozen boundary recordings", function()
        local tape, identity = short_match_tape.make()
        local frames = copy_frames(tape)
        local hashes = {}
        for index, hash in ipairs(tape.boundary_hashes) do
            hashes[index] = hash
        end
        local frozen = input_tape.from_frozen_recording(identity, tape.initial, frames, hashes)
        t.is_true(input_tape.validate_structure(frozen))
        t.eq(frozen.boundary_hashes[#frozen.boundary_hashes], hashes[#hashes])

        hashes[1] = "0000000000000000"
        frames[1].slots[1].move_x = frozen.frames[1].slots[1].move_x == -127 and 127 or -127
        t.is_true(frozen.boundary_hashes[1] ~= hashes[1])
        t.is_true(frozen.frames[1].slots[1].move_x ~= frames[1].slots[1].move_x)

        local malformed = {}
        for index, hash in ipairs(frozen.boundary_hashes) do
            malformed[index] = hash
        end
        malformed[1] = "0000000000000000"
        t.is_true(
            not pcall(
                input_tape.from_frozen_recording,
                identity,
                tape.initial,
                copy_frames(tape),
                malformed
            )
        )
    end)

    t.it("rejects identity and active tuning mismatches separately", function()
        local tape, identity = short_match_tape.make()
        local wrong = input_tape.copy_identity(identity)
        wrong.content = "different-content"
        local result, failure = replay.run(tape, wrong)
        t.eq(result, nil)
        local mismatch = assert(failure)
        t.eq(mismatch.code, "identity_mismatch")
        t.eq(mismatch.path, "identity.content")

        tuning.set("AI_SHOOT_RANGE", tuning.values.AI_SHOOT_RANGE + 1)
        local tuned, tune_failure = replay.run(tape, identity)
        tuning.reset()
        t.eq(tuned, nil)
        local tune_mismatch = assert(tune_failure)
        t.eq(tune_mismatch.code, "identity_mismatch")
        t.eq(tune_mismatch.path, "identity.tuning")
    end)

    t.it("rejects identity before simulation-aware tape validation", function()
        local tape, identity = short_match_tape.make()
        tape.frames[4] = assert(input_frame.neutral(3))
        tape.boundary_hashes[5] = tape.boundary_hashes[4]

        local wrong = input_tape.copy_identity(identity)
        wrong.content = "different-content"
        local mismatched, mismatch_failure = replay.run(tape, wrong)
        t.eq(mismatched, nil)
        local mismatch = assert(mismatch_failure)
        t.eq(mismatch.code, "identity_mismatch")
        t.eq(mismatch.path, "identity.content")

        tuning.set("AI_SHOOT_RANGE", tuning.values.AI_SHOOT_RANGE + 1)
        local tuned, tune_failure = replay.run(tape, identity)
        tuning.reset()
        t.eq(tuned, nil)
        local tune_mismatch = assert(tune_failure)
        t.eq(tune_mismatch.code, "identity_mismatch")
        t.eq(tune_mismatch.path, "identity.tuning")

        local missing_identity, valid_identity = short_match_tape.make()
        rawset(missing_identity, "identity", nil)
        local malformed_result, malformed_failure = replay.run(missing_identity, valid_identity)
        t.eq(malformed_result, nil)
        t.eq(assert(malformed_failure).code, "malformed")
    end)

    t.it("names the first causal input and differing state path", function()
        local reference, identity = short_match_tape.make()
        local changed_frames = copy_frames(reference)
        changed_frames[1].slots[1] = assert(input_frame.new_sample({ move_x = -127 }))
        local candidate = input_tape.new(identity, reference.initial, changed_frames)
        local comparison, failure = replay.compare(reference, candidate, identity)
        local compared = assert(comparison, failure and failure.message)
        t.is_true(not compared.equal)
        local divergence = assert(compared.divergence)
        t.eq(divergence.causal_input_tick, 0)
        t.eq(divergence.boundary_tick, 1)
        t.is_true(divergence.expected_hash ~= divergence.actual_hash)
        t.is_true(divergence.state_path:match("^state%.players%."))
        t.is_true(divergence.expected_input ~= divergence.actual_input)
    end)

    t.it("diagnoses a modified initial state before any causal input", function()
        local reference, identity = short_match_tape.make()
        local changed = match_snapshot.capture(match_snapshot.restore(reference.initial))
        changed.state.ball_z = 1
        local candidate = input_tape.new(identity, changed, copy_frames(reference))
        local comparison = assert(replay.compare(reference, candidate, identity))
        local divergence = assert(comparison.divergence)
        t.eq(divergence.causal_input_tick, nil)
        t.eq(divergence.boundary_tick, 0)
        t.eq(divergence.state_path, "state.ball_z")
        t.eq(divergence.expected_state, 0)
        t.eq(divergence.actual_state, 1)
    end)

    t.it("diagnoses a changed keeper behavior state before any causal input", function()
        local reference, identity = short_match_tape.make()
        local changed = match_snapshot.capture(match_snapshot.restore(reference.initial))
        changed.state.players[1].keeper_state = "advance"
        local candidate = input_tape.new(identity, changed, copy_frames(reference))
        local comparison = assert(replay.compare(reference, candidate, identity))
        local divergence = assert(comparison.divergence)
        t.eq(divergence.causal_input_tick, nil)
        t.eq(divergence.boundary_tick, 0)
        t.eq(divergence.state_path, "state.players.1.keeper_state")
    end)

    t.it("detects tampering against a tape's frozen boundary hashes", function()
        local tape, identity = short_match_tape.make()
        tape.frames[1].slots[1].move_x = -127
        local result = assert(replay.run(tape, identity))
        local divergence = assert(result.divergence)
        t.eq(divergence.causal_input_tick, 0)
        t.eq(divergence.boundary_tick, 1)
        t.eq(divergence.state_path, "unavailable_without_reference_tape")
    end)

    t.it("reports malformed and unsupported tape versions", function()
        local legacy, current_identity = short_match_tape.make()
        legacy.identity.input_version = 1
        legacy.identity.ownership.version = 1
        local legacy_result, legacy_failure = replay.run(legacy, current_identity)
        t.eq(legacy_result, nil)
        local incompatible = assert(legacy_failure)
        t.eq(incompatible.code, "identity_mismatch")
        t.eq(incompatible.path, "identity.input_version")
        t.eq(incompatible.expected, input_frame.VERSION)
        t.eq(incompatible.actual, 1)

        local tape, identity = short_match_tape.make()
        tape.version = 999
        local result, failure = replay.run(tape, identity)
        t.eq(result, nil)
        local malformed = assert(failure)
        t.eq(malformed.code, "malformed")
        t.is_true(malformed.message:match("unsupported input tape version") ~= nil)

        local snapshot_tape, snapshot_identity = short_match_tape.make()
        snapshot_tape.initial.version = match_snapshot.VERSION - 1
        local snapshot_result, snapshot_failure = replay.run(snapshot_tape, snapshot_identity)
        t.eq(snapshot_result, nil)
        local prior_schema = assert(snapshot_failure)
        t.eq(prior_schema.code, "malformed")
        t.is_true(prior_schema.message:match("unsupported match snapshot version") ~= nil)
    end)

    t.it("rejects ownership detached from the initial snapshot and post-match frames", function()
        local tape, identity = short_match_tape.make()
        tape.identity.ownership.slots[1].player_id = "detached-owner"
        local expected = input_tape.copy_identity(tape.identity)
        local detached, detached_failure = replay.run(tape, expected)
        t.eq(detached, nil)
        t.eq(assert(detached_failure).code, "malformed")

        local complete, complete_identity = short_match_tape.make()
        local frames = copy_frames(complete)
        frames[4] = assert(input_frame.neutral(3))
        t.is_true(
            not pcall(input_tape.new, complete_identity, complete.initial, frames),
            "a materialized frame cannot follow the finished boundary"
        )

        complete.frames[4] = assert(input_frame.neutral(3))
        complete.boundary_hashes[5] = complete.boundary_hashes[4]
        local appended, appended_failure = replay.run(complete, complete_identity)
        t.eq(appended, nil)
        local malformed = assert(appended_failure)
        t.eq(malformed.code, "malformed")
        t.is_true(malformed.message:match("frame after the match finished") ~= nil)
    end)

    t.it("rejects jointly malformed ownership and snapshot routing", function()
        local tape, identity = short_match_tape.make()
        local malformed_identity = input_tape.copy_identity(identity)
        local malformed_snapshot = match_snapshot.capture(match_snapshot.restore(tape.initial))
        malformed_identity.ownership.slots[1].team = "away"
        malformed_snapshot.state.input_ownership.slots[1].team = "away"
        t.is_true(
            not pcall(input_tape.new, malformed_identity, malformed_snapshot, copy_frames(tape)),
            "matching malformed slot semantics must still be rejected"
        )

        local duplicate_identity = input_tape.copy_identity(identity)
        local duplicate_snapshot = match_snapshot.capture(match_snapshot.restore(tape.initial))
        duplicate_identity.ownership.rosters.home[2] = duplicate_identity.ownership.rosters.home[1]
        duplicate_snapshot.state.input_ownership.rosters.home[2] =
            duplicate_snapshot.state.input_ownership.rosters.home[1]
        t.is_true(
            not pcall(input_tape.new, duplicate_identity, duplicate_snapshot, copy_frames(tape)),
            "matching duplicate roster membership must still be rejected"
        )

        local route_snapshot = match_snapshot.capture(match_snapshot.restore(tape.initial))
        route_snapshot.state.slot_players[1] = route_snapshot.state.slot_players[2]
        t.is_true(
            not pcall(input_tape.new, identity, route_snapshot, copy_frames(tape)),
            "slot_players must agree with immutable ownership"
        )

        local inverse_snapshot = match_snapshot.capture(match_snapshot.restore(tape.initial))
        local first_player = inverse_snapshot.state.slot_players[1]
        inverse_snapshot.state.slot_for_player[first_player] = 2
        t.is_true(
            not pcall(input_tape.new, identity, inverse_snapshot, copy_frames(tape)),
            "slot_for_player must agree with immutable ownership"
        )
    end)
end)
