local t = require("spec.support.runner")
local combat = require("sim.combat")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_snapshot_history = require("sim.rollback_snapshot_history")
local teams = require("data.teams")

---@return MatchState
local function new_state()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 2,
        max_goals = 3,
        seed = 69,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
end

---@param state MatchState
---@param tick integer
---@return MatchSnapshot
local function boundary(state, tick)
    state.input_tick = tick
    return match_snapshot.capture(state)
end

---@return RollbackInputSource[]
local function remote_sources()
    local result = {}
    for index = 1, input_frame.SLOT_COUNT do
        result[index] = "remote"
    end
    return result
end

t.describe("bounded rollback snapshot history", function()
    t.it("retains the present boundary plus exactly thirty prior boundaries", function()
        local history = rollback_snapshot_history.new()
        local state = new_state()
        for tick = 0, 31 do
            assert(rollback_snapshot_history.store(history, boundary(state, tick)))
        end

        local diagnostics = rollback_snapshot_history.diagnostics(history)
        t.eq(diagnostics.max_rollback_ticks, 30)
        t.eq(diagnostics.capacity, 31)
        t.eq(diagnostics.retained_boundary_count, 31)
        t.eq(diagnostics.oldest_supported_tick, 1)
        t.eq(diagnostics.oldest_boundary_tick, 1)
        t.eq(diagnostics.latest_tick, 31)
        t.eq(rollback_snapshot_history.lookup(history, 31).status, "present")
        t.eq(rollback_snapshot_history.lookup(history, 1).status, "retained")
        t.eq(rollback_snapshot_history.lookup(history, 0).status, "outside_window")
    end)

    t.it("distinguishes missing boundaries inside the ring's supported range", function()
        local history = rollback_snapshot_history.new(3)
        local state = new_state()
        assert(rollback_snapshot_history.store(history, boundary(state, 10)))
        assert(rollback_snapshot_history.store(history, boundary(state, 12)))

        t.eq(rollback_snapshot_history.lookup(history, 12).status, "present")
        t.eq(rollback_snapshot_history.lookup(history, 10).status, "retained")
        t.eq(rollback_snapshot_history.lookup(history, 11).status, "missing")
        t.eq(rollback_snapshot_history.lookup(history, 8).status, "outside_window")
        local rejected, _, code = rollback_snapshot_history.store(history, boundary(state, 8))
        t.eq(rejected, nil)
        t.eq(code, "outside_window")
    end)

    t.it("owns independent stored and returned snapshots", function()
        local history = rollback_snapshot_history.new()
        local state = new_state()
        local supplied = boundary(state, 0)
        local original_x = supplied.state.players[1].pos.x
        assert(rollback_snapshot_history.store(history, supplied))
        supplied.state.players[1].pos.x = original_x + 100

        local first = assert(rollback_snapshot_history.lookup(history, 0).snapshot)
        t.eq(first.state.players[1].pos.x, original_x)
        first.state.players[1].pos.x = original_x - 100
        local second = assert(rollback_snapshot_history.lookup(history, 0).snapshot)
        t.eq(second.state.players[1].pos.x, original_x)

        local restored, status = rollback_snapshot_history.restore(history, 0)
        t.eq(status, "present")
        assert(restored).players[1].pos.x = original_x + 50
        local unchanged = assert(rollback_snapshot_history.restore(history, 0))
        t.eq(unchanged.players[1].pos.x, original_x)
        t.eq(rollback_snapshot_history.status(history, 0), "present")
        t.eq(rollback_snapshot_history.status(history, 1), "missing")
    end)

    t.it("stores, accounts, and restores the combat companion atomically", function()
        local history = rollback_snapshot_history.new()
        local state = new_state()
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local supplied = match_snapshot.capture(state, combat_state)
        local expected_bytes = match_snapshot.encoded_size_canonical(supplied)
        assert(rollback_snapshot_history.store(history, supplied))
        assert(supplied.combat).next_source_sequence = 99

        local lookup = rollback_snapshot_history.lookup(history, 0)
        t.eq(lookup.canonical_bytes, expected_bytes)
        t.eq(assert(lookup.snapshot).version, match_snapshot.COMBAT_VERSION)
        t.eq(assert(lookup.snapshot.combat).next_source_sequence, 1)

        local restored, restored_combat, status =
            rollback_snapshot_history.restore_simulation(history, 0)
        t.eq(status, "present")
        t.eq(assert(restored).input_tick, 0)
        t.eq(assert(restored_combat).next_source_sequence, 1)
        restored_combat.next_source_sequence = 55
        local _, unchanged = rollback_snapshot_history.restore_simulation(history, 0)
        t.eq(assert(unchanged).next_source_sequence, 1)
        t.eq(rollback_snapshot_history.diagnostics(history).canonical_bytes, expected_bytes)
    end)

    t.it("compares retained boundaries and materializes hashes only for divergence", function()
        local expected = rollback_snapshot_history.new(2)
        local actual = rollback_snapshot_history.new(2)
        local state = new_state()
        assert(rollback_snapshot_history.store(expected, boundary(state, 0)))
        assert(rollback_snapshot_history.store(actual, boundary(state, 0)))

        local matched = rollback_snapshot_history.compare(expected, actual, 0)
        t.is_true(matched.matched)
        t.eq(matched.expected_status, "present")
        t.eq(matched.actual_status, "present")
        t.eq(matched.expected_hash, nil)
        t.eq(matched.actual_hash, nil)
        t.eq(matched.first_difference, nil)

        state.score.home = 1
        assert(rollback_snapshot_history.store(actual, boundary(state, 0)))
        local diverged = rollback_snapshot_history.compare(expected, actual, 0)
        t.eq(diverged.matched, false)
        t.eq(assert(diverged.first_difference).path, "state.score.home")
        t.is_true(assert(diverged.expected_hash) ~= assert(diverged.actual_hash))

        assert(rollback_snapshot_history.store(expected, boundary(state, 3)))
        local missing = rollback_snapshot_history.compare(expected, actual, 3)
        t.eq(missing.matched, false)
        t.eq(missing.expected_status, "present")
        t.eq(missing.actual_status, "missing")
    end)

    t.it("rejects retained boundaries that differ only by windup shot type", function()
        local expected = rollback_snapshot_history.new(2)
        local actual = rollback_snapshot_history.new(2)
        local state = new_state()
        state.players[2].windup_shot = {
            dir = state.players[2].facing,
            speed = 456,
            vz = 123,
            spin = -8,
            shot_type = "chip",
        }
        assert(rollback_snapshot_history.store(expected, boundary(state, 0)))
        state.players[2].windup_shot.shot_type = "ground"
        assert(rollback_snapshot_history.store(actual, boundary(state, 0)))

        local comparison = rollback_snapshot_history.compare(expected, actual, 0)
        t.is_true(not comparison.matched)
        t.eq(assert(comparison.first_difference).path, "state.players.2.windup_shot.shot_type")
        t.is_true(assert(comparison.expected_hash) ~= assert(comparison.actual_hash))
    end)

    t.it("updates byte accounting and invalidates a replaced boundary's lazy hash", function()
        local history = rollback_snapshot_history.new(1)
        local state = new_state()
        local initial = boundary(state, 0)
        local initial_bytes = #match_snapshot.encode(initial)
        local first = assert(rollback_snapshot_history.store(history, initial))
        t.eq(first.replaced, false)
        t.eq(history._entries[1].canonical_wire, nil)
        t.eq(rollback_snapshot_history.diagnostics(history).canonical_bytes, initial_bytes)
        local first_hash = assert(rollback_snapshot_history.boundary_hash(history, 0))
        t.eq(#assert(history._entries[1].canonical_wire), initial_bytes)

        state.score.home = 1
        local replacement = boundary(state, 0)
        local replacement_bytes = #match_snapshot.encode(replacement)
        local replaced = assert(rollback_snapshot_history.store(history, replacement))
        t.eq(replaced.replaced, true)
        local diagnostics = rollback_snapshot_history.diagnostics(history)
        t.eq(diagnostics.retained_boundary_count, 1)
        t.eq(diagnostics.canonical_bytes, replacement_bytes)
        local replacement_hash = assert(rollback_snapshot_history.boundary_hash(history, 0))
        t.is_true(replacement_hash ~= first_hash)

        local next_boundary = boundary(state, 1)
        local next_bytes = #match_snapshot.encode(next_boundary)
        assert(rollback_snapshot_history.store(history, next_boundary))
        t.eq(
            rollback_snapshot_history.diagnostics(history).canonical_bytes,
            replacement_bytes + next_bytes
        )
        local third_boundary = boundary(state, 2)
        local third_bytes = #match_snapshot.encode(third_boundary)
        local advanced = assert(rollback_snapshot_history.store(history, third_boundary))
        t.eq(advanced.evicted, 1)
        t.eq(
            rollback_snapshot_history.diagnostics(history).canonical_bytes,
            next_bytes + third_bytes
        )
        t.eq(rollback_snapshot_history.lookup(history, 0).status, "outside_window")
    end)

    t.it("evicts deterministically across wraparound and sparse advances", function()
        local left = rollback_snapshot_history.new(3)
        local right = rollback_snapshot_history.new(3)
        local state = new_state()
        for _, tick in ipairs({ 0, 1, 4, 6, 7 }) do
            assert(rollback_snapshot_history.store(left, boundary(state, tick)))
            assert(rollback_snapshot_history.store(right, boundary(state, tick)))
        end
        local left_diagnostics = rollback_snapshot_history.diagnostics(left)
        local right_diagnostics = rollback_snapshot_history.diagnostics(right)
        t.eq(left_diagnostics.retained_boundary_count, right_diagnostics.retained_boundary_count)
        t.eq(left_diagnostics.canonical_bytes, right_diagnostics.canonical_bytes)
        for tick = 0, 7 do
            local left_lookup = rollback_snapshot_history.lookup(left, tick)
            local right_lookup = rollback_snapshot_history.lookup(right, tick)
            t.eq(left_lookup.status, right_lookup.status)
            if left_lookup.snapshot then
                t.eq(
                    assert(rollback_snapshot_history.boundary_hash(left, tick)),
                    assert(rollback_snapshot_history.boundary_hash(right, tick))
                )
            end
        end
    end)

    t.it("rejects malformed, missing, and outside-floor truncation without mutation", function()
        local history = rollback_snapshot_history.new(2)
        local state = new_state()
        assert(rollback_snapshot_history.store(history, boundary(state, 5)))
        assert(rollback_snapshot_history.store(history, boundary(state, 7)))
        local before = rollback_snapshot_history.diagnostics(history)

        local truncated, _, code = rollback_snapshot_history.truncate_after(history, 4)
        t.eq(truncated, nil)
        t.eq(code, "outside_window")
        truncated, _, code = rollback_snapshot_history.truncate_after(history, 6)
        t.eq(truncated, nil)
        t.eq(code, "missing")
        ---@type any
        local malformed_tick = 6.5
        truncated, _, code = rollback_snapshot_history.truncate_after(history, malformed_tick)
        t.eq(truncated, nil)
        t.eq(code, "malformed")

        local after = rollback_snapshot_history.diagnostics(history)
        t.eq(after.latest_tick, before.latest_tick)
        t.eq(after.oldest_supported_tick, before.oldest_supported_tick)
        t.eq(after.retained_boundary_count, before.retained_boundary_count)
        t.eq(after.canonical_bytes, before.canonical_bytes)
    end)

    t.it("discards a corrected full-time tail at exact causal boundaries", function()
        local snapshots = rollback_snapshot_history.new(10)
        local inputs = rollback_input_history.new(remote_sources())
        local state = new_state()
        for tick = 0, 5 do
            assert(rollback_snapshot_history.store(snapshots, boundary(state, tick)))
            if tick < 5 then
                rollback_input_history.materialize(inputs, tick)
            end
        end
        local obsolete_hash = assert(rollback_snapshot_history.boundary_hash(snapshots, 5))
        t.is_true(obsolete_hash ~= "")

        assert(
            rollback_input_history.add_authoritative(
                inputs,
                4,
                1,
                assert(input_frame.new_sample({ move_x = 70 }))
            )
        )
        t.eq(rollback_input_history.earliest_divergence(inputs), 4)

        -- The corrected timeline reaches full time at boundary 3: frames 3+
        -- and snapshots 4+ belong only to the obsolete predicted timeline.
        -- Issue #70 owns the restore/step integration; this pins its exact
        -- storage handoff without implementing the rollback session here.
        state.finished = true
        assert(rollback_snapshot_history.store(snapshots, boundary(state, 3)))
        local retained_bytes = 0
        for tick = 0, 3 do
            retained_bytes = retained_bytes
                + assert(rollback_snapshot_history.lookup(snapshots, tick).canonical_bytes)
        end
        local retained_hash = assert(rollback_snapshot_history.boundary_hash(snapshots, 3))
        local snapshot_tail = assert(rollback_snapshot_history.truncate_after(snapshots, 3))
        local input_tail = assert(rollback_input_history.truncate_from(inputs, 3))

        t.eq(snapshot_tail.removed, 2)
        t.eq(snapshot_tail.diagnostics.latest_tick, 3)
        t.eq(snapshot_tail.diagnostics.retained_boundary_count, 4)
        t.eq(snapshot_tail.diagnostics.canonical_bytes, retained_bytes)
        t.eq(snapshot_tail.diagnostics.peak_retained_boundary_count, 6)
        t.is_true(
            snapshot_tail.diagnostics.peak_canonical_bytes
                > snapshot_tail.diagnostics.canonical_bytes
        )
        t.eq(rollback_snapshot_history.lookup(snapshots, 3).status, "present")
        t.is_true(assert(rollback_snapshot_history.lookup(snapshots, 3).snapshot).state.finished)
        t.eq(rollback_snapshot_history.lookup(snapshots, 4).status, "missing")
        t.eq(assert(rollback_snapshot_history.boundary_hash(snapshots, 3)), retained_hash)
        local removed_hash, removed_status = rollback_snapshot_history.boundary_hash(snapshots, 5)
        t.eq(removed_hash, nil)
        t.eq(removed_status, "missing")

        t.eq(input_tail.effective_removed, 2)
        t.eq(input_tail.records_removed, 2)
        t.is_true(input_tail.cleared_divergence)
        t.eq(input_tail.diagnostics.effective_tick_count, 3)
        t.eq(input_tail.diagnostics.record_tick_count, 3)
        t.eq(rollback_input_history.record(inputs, 3), nil)
        t.eq(rollback_input_history.earliest_divergence(inputs), nil)
        t.is_true(rollback_input_history.authoritative_record(inputs, 4, 1) ~= nil)

        local later = assert(
            rollback_input_history.add_authoritative(
                inputs,
                4,
                2,
                assert(input_frame.new_sample({ move_x = -70 }))
            )
        )
        t.eq(later.earliest_divergence, nil, "discarded input was never used by this timeline")
    end)

    t.it("retains the final full-time boundary without inventing another input", function()
        local history = rollback_snapshot_history.new()
        local state = new_state()
        assert(rollback_snapshot_history.store(history, boundary(state, 6)))
        state.finished = true
        assert(rollback_snapshot_history.store(history, boundary(state, 7)))

        local final = rollback_snapshot_history.lookup(history, 7)
        t.eq(final.status, "present")
        t.is_true(assert(final.snapshot).state.finished)
        t.eq(rollback_snapshot_history.lookup(history, 8).status, "missing")
        t.eq(rollback_snapshot_history.diagnostics(history).latest_tick, 7)
    end)
end)
