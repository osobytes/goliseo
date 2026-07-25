local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_session = require("sim.rollback_session")
local teams = require("data.teams")

---@param options { duration: number?, max_goals: integer?, seed: integer? }?
---@return MatchState
local function new_state(options)
    options = options or {}
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = options.duration or 4,
        max_goals = options.max_goals or 3,
        seed = options.seed or 70,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
end

---@return MatchSnapshot
local function initial_snapshot()
    return match_snapshot.capture(new_state())
end

---@param local_first boolean?
---@return RollbackInputSource[]
local function sources(local_first)
    local result = {}
    for index = 1, input_frame.SLOT_COUNT do
        result[index] = local_first and index == 1 and "local" or "remote"
    end
    return result
end

---@param options InputSampleOptions?
---@return InputSample
local function sample(options)
    return assert(input_frame.new_sample(options))
end

---@param session RollbackSession
---@param count integer
---@return RollbackTickOutput[]
local function step_many(session, count)
    local outputs = {}
    for _ = 1, count do
        outputs[#outputs + 1] = assert(rollback_session.step(session))
    end
    return outputs
end

---@param session RollbackSession
---@param tick integer
---@param rows table<integer, InputSample>
local function add_rows(session, tick, rows)
    for slot_index, row in pairs(rows) do
        assert(rollback_session.add_authoritative(session, tick, slot_index, row))
    end
end

---@param session RollbackSession
---@param tick integer
local function add_neutral_tick(session, tick)
    for slot_index = 1, input_frame.SLOT_COUNT do
        assert(
            rollback_session.add_authoritative(
                session,
                tick,
                slot_index,
                input_frame.neutral_sample()
            )
        )
    end
end

---@param max_goals integer?
---@return MatchSnapshot
local function shot_fixture(max_goals)
    local state = new_state({ duration = 4, max_goals = max_goals or 1, seed = 700 })
    local carrier_index = state.slot_players[1]
    local carrier = state.players[carrier_index]
    carrier.pos = Vec2.new(900, 270)
    carrier.vel = Vec2.new(0, 0)
    carrier.run_vel = Vec2.new(0, 0)
    carrier.facing = Vec2.new(1, 0)
    carrier.charge = 1
    state.owner = carrier_index
    state.ball = Vec2.new(918, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.ball_z = 0
    state.ball_vz = 0
    state.pickup_cd = 2
    state.block_grace = 2
    for index, player in ipairs(state.players) do
        if index ~= carrier_index then
            player.pos = Vec2.new(index < 6 and 200 or 100, 40 + index)
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
        end
    end
    return match_snapshot.capture(state)
end

---@return MatchSnapshot
local function preventable_goal_fixture()
    local state = new_state({ duration = 4, max_goals = 1, seed = 701 })
    local carrier_index = state.slot_players[1]
    local carrier = state.players[carrier_index]
    carrier.pos = Vec2.new(900, 270)
    carrier.vel = Vec2.new(0, 0)
    carrier.run_vel = Vec2.new(0, 0)
    carrier.facing = Vec2.new(1, 0)
    carrier.windup_timer = 1 / 60
    carrier.windup_shot = {
        dir = Vec2.new(1, 0),
        speed = 900,
        vz = 0,
        spin = 0,
        shot_type = "ground",
    }
    state.owner = carrier_index
    state.ball = Vec2.new(918, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.ball_z = 0
    state.ball_vz = 0
    state.pickup_cd = 2
    state.block_grace = 2
    for index, player in ipairs(state.players) do
        if index ~= carrier_index then
            player.pos = Vec2.new(index < 6 and 200 or 100, 40 + index)
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
        end
    end
    local defender = state.players[state.slot_players[5]]
    defender.pos = Vec2.new(910, 240)
    defender.facing = Vec2.new(-1, 0)
    return match_snapshot.capture(state)
end

t.describe("rollback session", function()
    t.it("restores and resimulates combat state atomically from the earliest edge", function()
        local state = new_state({ duration = 4, seed = 707 })
        state.kickoff_hold = 0
        local combat_state = combat.new_state(state)
        local source_player = nil
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "light_melee" then
                source_player = index
                break
            end
        end
        source_player = assert(source_player, "fixture requires one melee player")
        local target_player = nil
        for index, player in ipairs(state.players) do
            if player.team ~= state.players[source_player].team and not player.is_keeper then
                target_player = index
                break
            end
        end
        target_player = assert(target_player, "fixture requires an opposing target")
        for index, player in ipairs(state.players) do
            player.pos = Vec2.new(700 + index, 400 + index)
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
        end
        state.players[source_player].pos = Vec2.new(300, 270)
        state.players[source_player].facing = Vec2.new(1, 0)
        state.players[target_player].pos = Vec2.new(335, 270)
        state.players[target_player].facing = Vec2.new(-1, 0)
        state.owner = target_player
        state.ball = state.players[target_player].pos
        state.ball_vel = Vec2.new(0, 0)
        state.pickup_cd = 1
        local source_slot =
            assert(state.slot_for_player[source_player], "melee player requires an input slot")
        local initial = match_snapshot.capture(state, combat_state)
        local predicted = rollback_session.new(initial, sources())
        local predicted_output = assert(rollback_session.step(predicted))
        t.eq(predicted_output.combat_events and #predicted_output.combat_events or 0, 0)

        local pressed = sample({
            held = input_frame.HELD_BITS.equipment,
            edges = input_frame.EDGE_BITS.equipment_pressed,
        })
        for slot_index = 1, input_frame.SLOT_COUNT do
            assert(
                rollback_session.add_authoritative(
                    predicted,
                    0,
                    slot_index,
                    slot_index == source_slot and pressed or sample()
                )
            )
        end
        local reconciled = rollback_session.reconcile(predicted, true)
        t.is_true(reconciled.changed)
        t.eq(reconciled.causal_tick, 0)
        t.eq(#reconciled.corrected_outputs, 1)
        local corrected_events =
            assert(reconciled.corrected_outputs[1].combat_events, "combat events are missing")
        t.eq(corrected_events[1].kind, "commit")

        local expected = rollback_session.new(initial, sources())
        for slot_index = 1, input_frame.SLOT_COUNT do
            assert(
                rollback_session.add_authoritative(
                    expected,
                    0,
                    slot_index,
                    slot_index == source_slot and pressed or sample()
                )
            )
        end
        assert(rollback_session.step(expected))
        local saw_contact = false
        local saw_spill = false
        for tick = 1, 15 do
            add_neutral_tick(predicted, tick)
            add_neutral_tick(expected, tick)
            local output = assert(rollback_session.step(predicted))
            assert(rollback_session.step(expected))
            for _, event in ipairs(output.combat_events or {}) do
                saw_contact = saw_contact or event.kind == "contact"
                saw_spill = saw_spill or event.kind == "ball_spill"
            end
        end
        t.is_true(saw_contact)
        t.is_true(saw_spill)
        local comparison =
            rollback_session.compare(predicted, rollback_session.current_snapshot(expected), 0)
        t.is_true(comparison.matched)
        local final = rollback_session.current_snapshot(predicted)
        t.is_true(assert(final.combat).players[target_player].forced_ticks > 0)
        t.eq(final.combat.next_source_sequence, 2)
        t.eq(final.state.owner, nil)
    end)

    t.it("keeps measured operations under session ownership", function()
        local fabricated = rollback_session.new(
            initial_snapshot(),
            sources(),
            nil,
            function(_, operation)
                operation()
                return "not a snapshot"
            end
        )
        local output = assert(rollback_session.step(fabricated))
        t.eq(output.tick, 0)
        t.eq(rollback_session.diagnostics(fabricated).present_boundary, 1)

        local skipped = rollback_session.new(initial_snapshot(), sources(), nil, function() end)
        t.is_true(not pcall(rollback_session.step, skipped))

        local doubled = rollback_session.new(
            initial_snapshot(),
            sources(),
            nil,
            function(_, operation)
                operation()
                pcall(operation)
            end
        )
        t.is_true(not pcall(rollback_session.step, doubled))
    end)

    t.it("owns boundary zero and exposes only copied state, history, and outputs", function()
        local supplied = initial_snapshot()
        local original_x = supplied.state.players[1].pos.x
        local session = rollback_session.new(supplied, sources(true))
        supplied.state.players[1].pos.x = original_x + 1000

        local present = rollback_session.current_snapshot(session)
        t.eq(present.state.players[1].pos.x, original_x)
        present.state.players[1].pos.x = original_x - 1000
        t.eq(rollback_session.current_snapshot(session).state.players[1].pos.x, original_x)
        t.eq(rollback_session.snapshot(session, 0).status, "present")

        assert(rollback_session.add_authoritative(session, 0, 1, sample({ move_x = 30 })))
        local output = assert(rollback_session.step(session))
        t.eq(output.tick, 0)
        t.eq(output.start_boundary, 0)
        t.eq(output.end_boundary, 1)
        t.eq(output.input.slots[1].status, "authoritative")
        t.eq(output.input.slots[2].status, "predicted")
        t.eq(rollback_session.snapshot(session, 0).status, "retained")
        t.eq(rollback_session.snapshot(session, 1).status, "present")

        output.input.slots[1].sample.move_x = -30
        output.state.score.home = 99
        local retained = assert(rollback_session.output(session, 0))
        t.eq(retained.input.slots[1].sample.move_x, 30)
        t.eq(retained.state.score.home, 0)
        local diagnostics = rollback_session.diagnostics(session)
        t.eq(diagnostics.predicted_slot_samples, 7)
        t.eq(diagnostics.predicted_ticks, 1)
    end)

    t.it("treats an authoritative sample equal to prediction as a true no-op", function()
        local session = rollback_session.new(initial_snapshot(), sources())
        step_many(session, 3)
        local before = rollback_session.current_snapshot(session)
        local arrival =
            assert(rollback_session.add_authoritative(session, 1, 1, input_frame.neutral_sample()))
        t.is_true(not arrival.correction)
        t.eq(arrival.earliest_divergence, nil)

        local result = rollback_session.reconcile(session)
        t.is_true(not result.changed)
        t.eq(result.old_present_hash, nil)
        t.eq(result.new_present_hash, nil)
        t.eq(rollback_session.diagnostics(session).rollback_count, 0)
        t.eq(
            match_snapshot.hash(rollback_session.current_snapshot(session)),
            match_snapshot.hash(before)
        )
    end)

    t.it("converges one correction and replaces the affected snapshots and output", function()
        local initial = initial_snapshot()
        local corrected = sample({ move_x = 127 })
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        assert(rollback_session.add_authoritative(reference, 0, 1, corrected))
        step_many(reference, 5)

        local stale = step_many(delayed, 5)[1]
        local stale_hash = assert(rollback_session.snapshot(delayed, 1).snapshot)
        local arrival = assert(rollback_session.add_authoritative(delayed, 0, 1, corrected))
        t.is_true(arrival.correction)
        local result = rollback_session.reconcile(delayed, true)

        t.is_true(result.changed)
        t.eq(result.causal_tick, 0)
        t.eq(result.restore_status, "retained")
        t.eq(result.old_present_boundary, 5)
        t.eq(result.new_present_boundary, 5)
        t.eq(#result.corrected_outputs, 5)
        t.eq(result.corrected_outputs[1].input.slots[1].status, "authoritative")
        t.eq(stale.input.slots[1].sample.move_x, 0, "returned stale output remains immutable")
        t.eq(assert(rollback_session.output(delayed, 0)).input.slots[1].sample.move_x, 127)
        t.is_true(
            match_snapshot.hash(assert(rollback_session.snapshot(delayed, 1).snapshot))
                ~= match_snapshot.hash(stale_hash)
        )
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference), 0).matched
        )
        local first_difference = assert(result.first_difference)
        local original_path = first_difference.path
        first_difference.path = "mutated"
        local rollback_diagnostics = rollback_session.diagnostics(delayed)
        t.eq(
            assert(assert(rollback_diagnostics.last_rollback).first_difference).path,
            original_path
        )
        assert(assert(rollback_diagnostics.last_rollback).first_difference).path = "mutated again"
        t.eq(
            assert(assert(rollback_session.diagnostics(delayed).last_rollback).first_difference).path,
            original_path
        )
    end)

    t.it("batches several corrections into one earliest restore with ordered outputs", function()
        local initial = initial_snapshot()
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        local at_one = sample({ move_y = -127 })
        local at_three = sample({ move_x = 80, held = input_frame.HELD_BITS.sprint })
        add_rows(reference, 1, { [1] = at_one })
        add_rows(reference, 3, { [2] = at_three })
        step_many(reference, 6)
        step_many(delayed, 6)

        add_rows(delayed, 3, { [2] = at_three })
        add_rows(delayed, 1, { [1] = at_one })
        local result = rollback_session.reconcile(delayed)
        t.eq(result.causal_tick, 1)
        t.eq(#result.corrected_outputs, 5)
        t.eq(result.old_present_hash, nil)
        t.eq(result.new_present_hash, nil)
        t.eq(result.first_difference, nil)
        t.eq(assert(rollback_session.diagnostics(delayed).last_rollback).first_difference, nil)
        for index, output in ipairs(result.corrected_outputs) do
            t.eq(output.tick, index)
        end
        t.eq(result.corrected_outputs[1].input.slots[1].status, "authoritative")
        t.eq(result.corrected_outputs[3].input.slots[2].status, "authoritative")
        t.eq(rollback_session.diagnostics(delayed).rollback_count, 1)
        t.eq(rollback_session.diagnostics(delayed).correction_count, 2)
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference), 1).matched
        )
    end)

    t.it("remains deterministic across repeated rollback of corrected intervals", function()
        local initial = initial_snapshot()
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        local first = sample({ move_x = 100 })
        local second = sample({ move_y = 90 })
        add_rows(reference, 1, { [1] = first })
        add_rows(reference, 2, { [2] = second })
        step_many(reference, 7)
        step_many(delayed, 7)

        add_rows(delayed, 2, { [2] = second })
        rollback_session.reconcile(delayed)
        add_rows(delayed, 1, { [1] = first })
        rollback_session.reconcile(delayed)
        local diagnostics = rollback_session.diagnostics(delayed)
        t.eq(diagnostics.rollback_count, 2)
        t.eq(diagnostics.latest_rollback_depth, 6)
        t.eq(diagnostics.max_rollback_depth, 6)
        t.eq(diagnostics.resimulated_ticks, 11)
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference)).matched
        )
    end)

    t.it("handles a correction exactly thirty ticks deep", function()
        local initial = initial_snapshot()
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        local corrected = sample({ move_x = -127 })
        assert(rollback_session.add_authoritative(reference, 1, 1, corrected))
        step_many(reference, 31)
        step_many(delayed, 31)

        assert(rollback_session.add_authoritative(delayed, 1, 1, corrected))
        local result = rollback_session.reconcile(delayed)
        t.eq(result.causal_tick, 1)
        t.eq(result.restore_status, "retained")
        t.eq(#result.corrected_outputs, 30)
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference)).matched
        )
    end)

    t.it("fails explicitly at thirty-one ticks late without hidden progress", function()
        local session = rollback_session.new(initial_snapshot(), sources())
        step_many(session, 31)
        local before = rollback_session.current_snapshot(session)
        local before_hash = match_snapshot.hash(before)
        local arrival, _, code =
            rollback_session.add_authoritative(session, 0, 1, sample({ move_x = 127 }))
        t.eq(arrival, nil)
        t.eq(code, "outside_window")

        local result = rollback_session.reconcile(session)
        t.is_true(not result.changed)
        t.eq(result.status, "late_input_unrecoverable")
        t.eq(result.causal_tick, 0)
        t.eq(result.restore_status, "outside_window")
        t.eq(result.old_present_boundary, 31)
        t.eq(result.new_present_boundary, 31)
        t.eq(result.old_present_hash, nil)
        t.eq(result.new_present_hash, nil)
        t.eq(match_snapshot.hash(rollback_session.current_snapshot(session)), before_hash)
        local output, _, step_code = rollback_session.step(session)
        t.eq(output, nil)
        t.eq(step_code, "late_input_unrecoverable")
        t.eq(match_snapshot.hash(rollback_session.current_snapshot(session)), before_hash)
        t.eq(rollback_session.diagnostics(session).late_window_failures, 1)
    end)

    t.it("attributes mixed retained and over-window batches to the actual late tick", function()
        for _, order in ipairs({ "retained_first", "late_first" }) do
            local session = rollback_session.new(initial_snapshot(), sources())
            step_many(session, 31)
            local before = match_snapshot.hash(rollback_session.current_snapshot(session))
            local retained = function()
                return rollback_session.add_authoritative(session, 1, 1, sample({ move_x = 10 }))
            end
            local late = function()
                local arrival, _, code =
                    rollback_session.add_authoritative(session, 0, 2, sample({ move_y = 10 }))
                t.eq(arrival, nil)
                t.eq(code, "outside_window")
            end
            if order == "retained_first" then
                assert(retained())
                late()
            else
                late()
                assert(retained())
            end

            local result = rollback_session.reconcile(session)
            t.is_true(not result.changed)
            t.eq(result.causal_tick, 0, order)
            t.eq(result.restore_status, "outside_window", order)
            t.eq(result.old_present_boundary, 31, order)
            t.eq(result.new_present_boundary, 31, order)
            t.eq(result.old_present_hash, nil, order)
            t.eq(result.new_present_hash, nil, order)
            t.eq(match_snapshot.hash(rollback_session.current_snapshot(session)), before, order)
            t.eq(rollback_session.diagnostics(session).late_window_failures, 1, order)
            local output, _, code = rollback_session.step(session)
            t.eq(output, nil, order)
            t.eq(code, "late_input_unrecoverable", order)
        end
    end)

    t.it("asserts a retained-range snapshot gap before consuming or restoring", function()
        local session = rollback_session.new(initial_snapshot(), sources(), 5)
        step_many(session, 3)
        assert(rollback_session.add_authoritative(session, 1, 1, sample({ move_x = 10 })))
        local before = match_snapshot.hash(rollback_session.current_snapshot(session))

        -- Deliberate invariant corruption: remove boundary one from its ring
        -- while leaving the input divergence intact.
        local snapshots = session._snapshot_history
        snapshots._entries[(1 % snapshots._capacity) + 1] = nil
        local ok, err = pcall(rollback_session.reconcile, session)
        t.is_true(not ok)
        t.is_true(tostring(err):match("rollback snapshot invariant: boundary 1 is missing") ~= nil)
        t.eq(match_snapshot.hash(rollback_session.current_snapshot(session)), before)
        t.eq(session._input_history._earliest_divergence, 1)
    end)

    t.it("keeps no-op reconciliation and matching comparison diagnostics lazy", function()
        local session = rollback_session.new(initial_snapshot(), sources())
        step_many(session, 2)
        assert(rollback_session.add_authoritative(session, 1, 1, input_frame.neutral_sample()))
        local expected = rollback_session.current_snapshot(session)
        local terminal = rollback_session.new(initial_snapshot(), sources())
        step_many(terminal, 31)
        local rejected = rollback_session.add_authoritative(terminal, 0, 1, sample({ move_x = 1 }))
        t.eq(rejected, nil)
        local capture_calls, hash_calls, difference_calls = 0, 0, 0
        ---@type any
        local snapshot_module = match_snapshot
        local original_capture = match_snapshot.capture
        local original_hash = match_snapshot.hash
        local original_difference = match_snapshot.first_difference
        snapshot_module.capture = function(...)
            capture_calls = capture_calls + 1
            return original_capture(...)
        end
        snapshot_module.hash = function(...)
            hash_calls = hash_calls + 1
            return original_hash(...)
        end
        snapshot_module.first_difference = function(...)
            difference_calls = difference_calls + 1
            return original_difference(...)
        end

        local ok, err = pcall(function()
            local result = rollback_session.reconcile(session)
            t.is_true(not result.changed)
            t.eq(capture_calls, 0)
            t.eq(hash_calls, 0)
            t.eq(difference_calls, 0)

            local terminal_result = rollback_session.reconcile(terminal)
            t.is_true(not terminal_result.changed)
            t.eq(terminal_result.status, "late_input_unrecoverable")
            t.eq(capture_calls, 0)
            t.eq(hash_calls, 0)
            t.eq(difference_calls, 0)

            local comparison = rollback_session.compare(session, expected)
            t.is_true(comparison.matched)
            t.eq(hash_calls, 2)
            t.eq(difference_calls, 0)
        end)
        snapshot_module.capture = original_capture
        snapshot_module.hash = original_hash
        snapshot_module.first_difference = original_difference
        assert(ok, err)
    end)

    t.it("corrects a real shot into earlier full time and discards every stale tail", function()
        local initial = shot_fixture()
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        local shot = sample({ edges = input_frame.EDGE_BITS.shoot })
        assert(rollback_session.add_authoritative(reference, 0, 1, shot))
        while rollback_session.diagnostics(reference).status ~= "finished" do
            assert(rollback_session.step(reference))
        end
        local final_boundary = rollback_session.diagnostics(reference).present_boundary
        t.is_true(final_boundary < 30, "the causal shot fixture must finish promptly")

        step_many(delayed, 30)
        t.eq(rollback_session.diagnostics(delayed).status, "active")
        assert(rollback_session.add_authoritative(delayed, 0, 1, shot))
        local result = rollback_session.reconcile(delayed)
        t.eq(result.status, "finished")
        t.eq(result.new_present_boundary, final_boundary)
        t.is_true(result.new_present_boundary < result.old_present_boundary)
        t.eq(result.corrected_from_tick, 0)
        t.eq(result.corrected_through_tick, final_boundary - 1)
        t.eq(result.replaced_from_tick, 0)
        t.eq(result.replaced_through_tick, 29)
        t.eq(#result.corrected_outputs, final_boundary)
        t.is_true(assert(rollback_session.output(delayed, final_boundary - 1)).finished)
        t.eq(rollback_session.output(delayed, final_boundary), nil)
        t.eq(rollback_session.snapshot(delayed, final_boundary).status, "present")
        t.eq(rollback_session.snapshot(delayed, final_boundary + 1).status, "missing")
        local stopped, _, stopped_code = rollback_session.step(delayed)
        t.eq(stopped, nil)
        t.eq(stopped_code, "match_finished")
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference), 0).matched
        )
        local retained = rollback_session.diagnostics(delayed).snapshot_history
        t.is_true(retained.peak_retained_boundary_count > retained.retained_boundary_count)
        t.is_true(retained.peak_canonical_bytes > retained.canonical_bytes)

        local later = nil
        for tick = 0, final_boundary + 1 do
            for slot_index = 1, input_frame.SLOT_COUNT do
                local authoritative = tick == 0 and slot_index == 1 and shot
                    or input_frame.neutral_sample()
                later = assert(
                    rollback_session.add_authoritative(delayed, tick, slot_index, authoritative)
                )
            end
        end
        t.eq(later.earliest_divergence, nil, "discarded frames cannot create false divergence")
        local diagnostics = rollback_session.diagnostics(delayed)
        t.eq(diagnostics.confirmed_tick, final_boundary + 1)
        t.eq(diagnostics.confirmed_output_tick, final_boundary - 1)
    end)

    t.it("replaces predicted play with the corrected goal and kickoff timeline", function()
        local initial = shot_fixture(3)
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        local shot = sample({ edges = input_frame.EDGE_BITS.shoot })
        assert(rollback_session.add_authoritative(reference, 0, 1, shot))
        step_many(reference, 30)
        step_many(delayed, 30)
        t.eq(rollback_session.current_snapshot(delayed).state.score.home, 0)

        assert(rollback_session.add_authoritative(delayed, 0, 1, shot))
        local result = rollback_session.reconcile(delayed)
        t.eq(result.status, "active")
        t.eq(result.new_present_boundary, 30)
        t.eq(#result.corrected_outputs, 30)
        local corrected_goal_tick = nil
        local shot_event_tick = nil
        for _, output in ipairs(result.corrected_outputs) do
            if output.state.score.home == 1 then
                corrected_goal_tick = corrected_goal_tick or output.tick
            end
            for _, event in ipairs(output.events) do
                if event.kind == "shot" then
                    shot_event_tick = shot_event_tick or output.tick
                end
            end
        end
        t.is_true(shot_event_tick ~= nil, "corrected outputs retain per-tick event ordering")
        t.is_true(corrected_goal_tick ~= nil, "corrected outputs expose the goal transition")
        t.is_true(assert(shot_event_tick) < assert(corrected_goal_tick))
        local corrected = rollback_session.current_snapshot(delayed)
        t.eq(corrected.state.score.home, 1)
        t.is_true(corrected.state.kickoff_hold > 0, "corrected play retains the kickoff lifecycle")
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference), 0).matched
        )
    end)

    t.it("allows a finished predicted timeline to reactivate after correction", function()
        local initial = preventable_goal_fixture()
        local delayed = rollback_session.new(initial, sources())
        local reference = rollback_session.new(initial, sources())
        local tackle = sample({ edges = input_frame.EDGE_BITS.dash })
        assert(rollback_session.add_authoritative(reference, 0, 5, tackle))

        while rollback_session.diagnostics(delayed).status ~= "finished" do
            assert(rollback_session.step(delayed))
        end
        local predicted_finish = rollback_session.diagnostics(delayed).present_boundary
        t.eq(predicted_finish, 5)
        t.eq(rollback_session.current_snapshot(delayed).state.score.home, 1)
        t.eq(rollback_session.output(delayed, predicted_finish), nil)

        step_many(reference, predicted_finish)
        t.eq(rollback_session.diagnostics(reference).status, "active")
        assert(rollback_session.add_authoritative(delayed, 0, 5, tackle))
        local result = rollback_session.reconcile(delayed)
        t.is_true(result.changed)
        t.eq(result.old_present_boundary, predicted_finish)
        t.eq(result.new_present_boundary, predicted_finish)
        t.eq(result.status, "active")
        t.eq(rollback_session.current_snapshot(delayed).state.score.home, 0)
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference), 0).matched
        )

        while rollback_session.diagnostics(reference).status ~= "finished" do
            assert(rollback_session.step(reference))
            assert(rollback_session.step(delayed))
        end
        t.eq(rollback_session.diagnostics(delayed).status, "finished")
        t.eq(
            rollback_session.diagnostics(delayed).present_boundary,
            rollback_session.diagnostics(reference).present_boundary
        )
        t.is_true(
            rollback_session.compare(delayed, rollback_session.current_snapshot(reference), 0).matched
        )
        local final_boundary = rollback_session.diagnostics(delayed).present_boundary
        t.eq(rollback_session.output(delayed, final_boundary), nil)
    end)

    t.it("truncates successfully after a monotonic floor has advanced", function()
        local initial = shot_fixture()
        local session = rollback_session.new(initial, sources())
        step_many(session, 40)
        local before = rollback_session.diagnostics(session)
        t.eq(before.input_history.oldest_retained_tick, 10)
        local shot = sample({ edges = input_frame.EDGE_BITS.shoot })
        assert(rollback_session.add_authoritative(session, 15, 1, shot))
        local result = rollback_session.reconcile(session)
        t.is_true(result.new_present_boundary < 40)
        t.is_true(result.new_present_boundary >= 15)
        local after = rollback_session.diagnostics(session)
        t.eq(after.input_history.oldest_retained_tick, 10)
        t.eq(after.snapshot_history.oldest_supported_tick, 10)
        t.eq(after.snapshot_history.latest_tick, result.new_present_boundary)
        t.eq(rollback_session.output(session, result.new_present_boundary), nil)
        local later =
            assert(rollback_session.add_authoritative(session, 35, 2, sample({ move_y = 20 })))
        t.eq(later.earliest_divergence, nil)
    end)

    t.it("reports confirmation and deterministic boundary diagnostics", function()
        local session = rollback_session.new(initial_snapshot(), sources())
        add_neutral_tick(session, 0)
        assert(rollback_session.step(session))
        local diagnostics = rollback_session.diagnostics(session)
        t.eq(diagnostics.confirmed_tick, 0)
        t.eq(diagnostics.confirmed_output_tick, 0)

        local expected = rollback_session.current_snapshot(session)
        expected.state.players[1].dive_target = { x = 12, y = 34 }
        local comparison = rollback_session.compare(session, expected, 0)
        t.is_true(not comparison.matched)
        t.is_true(not comparison.boundary_mismatch)
        t.eq(comparison.causal_tick, 0)
        local difference = assert(comparison.first_difference)
        t.eq(difference.path, "state.players.1.dive_target")
        t.eq(assert(difference.expected).x, 12)
        assert(difference.expected).x = 99
        local stored =
            assert(assert(rollback_session.diagnostics(session).last_comparison).first_difference)
        t.eq(assert(stored.expected).x, 12)
        assert(stored.expected).x = 88
        local reread =
            assert(assert(rollback_session.diagnostics(session).last_comparison).first_difference)
        t.eq(assert(reread.expected).x, 12)
        expected.state.input_tick = 2
        local boundary = rollback_session.compare(session, expected)
        t.is_true(boundary.boundary_mismatch)
        t.eq(boundary.actual_boundary, 1)
        t.eq(boundary.expected_boundary, 2)

        local copied = rollback_session.diagnostics(session)
        assert(copied.last_comparison).actual_hash = "mutated"
        t.is_true(
            assert(rollback_session.diagnostics(session).last_comparison).actual_hash ~= "mutated"
        )
    end)
end)
