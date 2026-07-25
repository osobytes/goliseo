-- Pure rollback coordinator. It owns the live MatchState and composes bounded
-- input/snapshot histories into deterministic restore and resimulation.

local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_snapshot_history = require("sim.rollback_snapshot_history")

---@alias RollbackSessionStatus "active"|"finished"|"late_input_unrecoverable"
---@alias RollbackSessionErrorCode "match_finished"|"late_input_unrecoverable"

---@class RollbackOutputStateView
---@field score { home: integer, away: integer }
---@field time_left number
---@field finished boolean

---@class RollbackTickOutput
---@field tick integer -- Causal input tick.
---@field start_boundary integer
---@field end_boundary integer
---@field input RollbackInputTickRecord
---@field events MatchEvent[]
---@field combat_events CombatEvent[]?
---@field state RollbackOutputStateView
---@field finished boolean

---@class RollbackSessionArrival: RollbackAuthoritativeArrival
---@field correction boolean -- Accepted authority differs from a previously consumed sample.

---@class RollbackComparison
---@field matched boolean
---@field boundary_mismatch boolean
---@field actual_boundary integer
---@field expected_boundary integer
---@field actual_hash string
---@field expected_hash string
---@field causal_tick integer?
---@field first_difference MatchSnapshotDifference?

---@class RollbackReconcileResult
---@field changed boolean
---@field status RollbackSessionStatus
---@field causal_tick integer?
---@field restored_boundary integer?
---@field restore_status RollbackSnapshotLookupStatus?
---@field old_present_boundary integer
---@field new_present_boundary integer
---@field corrected_from_tick integer?
---@field corrected_through_tick integer?
---@field replaced_from_tick integer?
---@field replaced_through_tick integer?
---@field corrected_outputs RollbackTickOutput[]
---@field old_present_hash string? -- Present only when a rollback changed simulation state.
---@field new_present_hash string? -- Present only when a rollback changed simulation state.
---@field first_difference MatchSnapshotDifference?

---@class RollbackSessionLastRollback
---@field causal_tick integer
---@field restored_boundary integer
---@field old_present_boundary integer
---@field new_present_boundary integer
---@field old_present_hash string?
---@field new_present_hash string?
---@field first_difference MatchSnapshotDifference?

---@class RollbackSessionDiagnostics
---@field status RollbackSessionStatus
---@field present_boundary integer
---@field confirmed_tick integer -- Monotonic input-authority boundary.
---@field confirmed_output_tick integer -- Confirmed input capped to outputs that exist.
---@field rollback_count integer
---@field correction_count integer
---@field predicted_slot_samples integer -- Cumulative executions, including resimulation.
---@field predicted_ticks integer -- Cumulative executions with at least one prediction.
---@field latest_rollback_depth integer
---@field max_rollback_depth integer
---@field resimulated_ticks integer
---@field late_window_failures integer
---@field last_rollback RollbackSessionLastRollback?
---@field last_comparison RollbackComparison?
---@field input_history RollbackInputHistoryDiagnostics
---@field snapshot_history RollbackSnapshotHistoryDiagnostics

---@class RollbackSessionAccounting
---@field input RollbackInputHistoryAccounting
---@field output_bytes integer
---@field snapshot_bytes integer
---@field total_bytes integer

---@alias RollbackSessionMeasureLabel "tick"|"capture"|"restore"|"resimulation"|"rollback"
---@alias RollbackSessionMeasure fun(label: RollbackSessionMeasureLabel, operation: fun(): any): any

---@class RollbackSession
---@field _state MatchState
---@field _combat_state CombatMatchState?
---@field _input_history RollbackInputHistory
---@field _snapshot_history RollbackSnapshotHistory
---@field _outputs table<integer, RollbackTickOutput>
---@field _status RollbackSessionStatus
---@field _rollback_count integer
---@field _correction_count integer
---@field _predicted_slot_samples integer
---@field _predicted_ticks integer
---@field _latest_rollback_depth integer
---@field _max_rollback_depth integer
---@field _resimulated_ticks integer
---@field _late_window_failures integer
---@field _late_input_tick integer?
---@field _last_rollback RollbackSessionLastRollback?
---@field _last_comparison RollbackComparison?
---@field _measure RollbackSessionMeasure?

---@class RollbackSessionModule
local rollback_session = {}

---@param session RollbackSession
---@param label RollbackSessionMeasureLabel
---@param operation fun(): any
---@return any
local function measured(session, label, operation)
    if session._measure == nil then
        return operation()
    end
    local calls = 0
    local result = nil
    local operation_succeeded = false
    local operation_error = nil
    session._measure(label, function()
        calls = calls + 1
        assert(calls == 1, "rollback measurement operation must run exactly once")
        local ok, value = pcall(operation)
        operation_succeeded = ok
        if not ok then
            operation_error = value
            error(value, 0)
        end
        result = value
        return result
    end)
    assert(calls == 1, "rollback measurement observer must run its operation exactly once")
    if not operation_succeeded then
        error(operation_error, 0)
    end
    return result
end

---@param value any
---@return any
local function copy_diagnostic_value(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, child in pairs(value) do
        result[copy_diagnostic_value(key)] = copy_diagnostic_value(child)
    end
    return result
end

---@param difference MatchSnapshotDifference?
---@return MatchSnapshotDifference?
local function copy_difference(difference)
    if difference == nil then
        return nil
    end
    return {
        path = difference.path,
        expected = copy_diagnostic_value(difference.expected),
        actual = copy_diagnostic_value(difference.actual),
    }
end

---@param session RollbackSession
local function assert_session(session)
    assert(type(session) == "table", "rollback session is required")
    assert(type(session._state) == "table", "rollback session state is missing")
    assert(type(session._input_history) == "table", "rollback session input history is missing")
    assert(
        type(session._snapshot_history) == "table",
        "rollback session snapshot history is missing"
    )
end

---@param sample InputSample
---@param record RollbackInputSlotRecord
---@return boolean
local function sample_differs(sample, record)
    local used = record.sample
    return sample.move_x ~= used.move_x
        or sample.move_y ~= used.move_y
        or sample.held ~= used.held
        or sample.edges ~= used.edges
end

---@param event MatchEvent
---@return MatchEvent
local function copy_event(event)
    local result = {}
    for key, value in pairs(event) do
        assert(type(value) ~= "table", "rollback output events must contain canonical scalars")
        result[key] = value
    end
    ---@cast result MatchEvent
    return result
end

---@param events MatchEvent[]
---@return MatchEvent[]
local function copy_events(events)
    local result = {}
    for index, event in ipairs(events) do
        result[index] = copy_event(event)
    end
    return result
end

---@param events CombatEvent[]
---@return CombatEvent[]
local function copy_combat_events(events)
    local result = {}
    for index, event in ipairs(events) do
        local copied = {}
        for key, value in pairs(event) do
            assert(type(value) ~= "table", "rollback combat events must contain canonical scalars")
            copied[key] = value
        end
        result[index] = copied
    end
    ---@cast result CombatEvent[]
    return result
end

---@param value any
---@return string
local function canonical_payload(value)
    local kind = type(value)
    if kind == "nil" then
        return "n"
    elseif kind == "boolean" then
        return value and "b1" or "b0"
    elseif kind == "number" then
        local payload = match_snapshot.number_bytes(value)
        return "d" .. #payload .. ":" .. payload
    elseif kind == "string" then
        return "s" .. #value .. ":" .. value
    elseif kind == "table" then
        local keys = {}
        for key in pairs(value) do
            keys[#keys + 1] = key
        end
        table.sort(keys, function(left, right)
            return type(left) .. ":" .. tostring(left) < type(right) .. ":" .. tostring(right)
        end)
        local parts = { "t", tostring(#keys), ":" }
        for _, key in ipairs(keys) do
            parts[#parts + 1] = canonical_payload(key)
            parts[#parts + 1] = canonical_payload(value[key])
        end
        return table.concat(parts)
    end
    assert(false, "rollback session accounting cannot encode " .. kind)
    return ""
end

---@param record RollbackInputTickRecord
---@return RollbackInputTickRecord
local function copy_input_record(record)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = record.slots[index]
        slots[index] = {
            source = slot.source,
            status = slot.status,
            sample = assert(input_frame.new_sample(slot.sample)),
        }
    end
    return { tick = record.tick, slots = slots }
end

---@param output RollbackTickOutput
---@return RollbackTickOutput
local function copy_output(output)
    local result = {
        tick = output.tick,
        start_boundary = output.start_boundary,
        end_boundary = output.end_boundary,
        input = copy_input_record(output.input),
        events = copy_events(output.events),
        state = {
            score = { home = output.state.score.home, away = output.state.score.away },
            time_left = output.state.time_left,
            finished = output.state.finished,
        },
        finished = output.finished,
    }
    if output.combat_events then
        result.combat_events = copy_combat_events(output.combat_events)
    end
    return result
end

---@param comparison RollbackComparison
---@return RollbackComparison
local function copy_comparison(comparison)
    return {
        matched = comparison.matched,
        boundary_mismatch = comparison.boundary_mismatch,
        actual_boundary = comparison.actual_boundary,
        expected_boundary = comparison.expected_boundary,
        actual_hash = comparison.actual_hash,
        expected_hash = comparison.expected_hash,
        causal_tick = comparison.causal_tick,
        first_difference = copy_difference(comparison.first_difference),
    }
end

---@param rollback RollbackSessionLastRollback
---@return RollbackSessionLastRollback
local function copy_last_rollback(rollback)
    return {
        causal_tick = rollback.causal_tick,
        restored_boundary = rollback.restored_boundary,
        old_present_boundary = rollback.old_present_boundary,
        new_present_boundary = rollback.new_present_boundary,
        old_present_hash = rollback.old_present_hash,
        new_present_hash = rollback.new_present_hash,
        first_difference = copy_difference(rollback.first_difference),
    }
end

---@param session RollbackSession
---@param record RollbackInputTickRecord
local function count_predictions(session, record)
    local predicted = 0
    for index = 1, input_frame.SLOT_COUNT do
        if record.slots[index].status == "predicted" then
            predicted = predicted + 1
        end
    end
    session._predicted_slot_samples = session._predicted_slot_samples + predicted
    if predicted > 0 then
        session._predicted_ticks = session._predicted_ticks + 1
    end
end

---@param tick integer
---@param record RollbackInputTickRecord
---@param snapshot MatchSnapshot
---@return RollbackTickOutput
local function make_output(tick, record, snapshot)
    local state = snapshot.state
    local result = {
        tick = tick,
        start_boundary = tick,
        end_boundary = tick + 1,
        input = copy_input_record(record),
        events = copy_events(state.events),
        state = {
            score = { home = state.score.home, away = state.score.away },
            time_left = state.time_left,
            finished = state.finished,
        },
        finished = state.finished,
    }
    if snapshot.combat then
        result.combat_events = copy_combat_events(snapshot.combat.events)
    end
    return result
end

---@param session RollbackSession
---@param tick integer
---@return RollbackTickOutput
local function execute_tick(session, tick)
    assert(not session._state.finished, "rollback session cannot simulate after full time")
    assert(session._state.input_tick == tick, "rollback session boundary is inconsistent")
    local frame, record = rollback_input_history.materialize(session._input_history, tick)
    count_predictions(session, record)
    match.step(session._state, fixed_clock.TICK_SECONDS, frame, session._combat_state)
    local boundary = measured(session, "capture", function()
        return match_snapshot.capture_owned(session._state, session._combat_state)
    end)
    ---@cast boundary MatchSnapshot
    assert(
        boundary.state.input_tick == tick + 1,
        "rollback session step did not advance one boundary"
    )
    assert(rollback_snapshot_history.store_owned(session._snapshot_history, boundary))
    local output = make_output(tick, record, boundary)
    session._outputs[tick] = output
    return output
end

---@param session RollbackSession
local function prune_retained_outputs(session)
    local snapshot_diagnostics = rollback_snapshot_history.diagnostics(session._snapshot_history)
    local floor = assert(
        snapshot_diagnostics.oldest_supported_tick,
        "rollback snapshot history has no supported floor"
    )
    assert(rollback_input_history.prune_before(session._input_history, floor))
    for tick in pairs(session._outputs) do
        if tick < floor then
            session._outputs[tick] = nil
        end
    end
end

---@param initial_snapshot MatchSnapshot Canonical slot-mode boundary zero.
---@param sources RollbackInputSource[] Canonical eight-slot local/remote ownership.
---@param max_rollback_ticks integer?
---@param measure RollbackSessionMeasure? Optional observer; it cannot change logical results.
---@return RollbackSession
function rollback_session.new(initial_snapshot, sources, max_rollback_ticks, measure)
    local state, combat_state = match_snapshot.restore(initial_snapshot)
    assert(state.slot_mode, "rollback session requires a slot-mode match snapshot")
    assert(state.input_tick == 0, "rollback session requires the tick-zero boundary")
    assert(not state.finished, "rollback session tick-zero boundary must be active")
    local canonical = match_snapshot.capture_owned(state, combat_state)
    local snapshots = rollback_snapshot_history.new(max_rollback_ticks)
    assert(rollback_snapshot_history.store_owned(snapshots, canonical))
    return {
        _state = state,
        _combat_state = combat_state,
        _input_history = rollback_input_history.new(sources),
        _snapshot_history = snapshots,
        _outputs = {},
        _status = "active",
        _rollback_count = 0,
        _correction_count = 0,
        _predicted_slot_samples = 0,
        _predicted_ticks = 0,
        _latest_rollback_depth = 0,
        _max_rollback_depth = 0,
        _resimulated_ticks = 0,
        _late_window_failures = 0,
        _late_input_tick = nil,
        _last_rollback = nil,
        _last_comparison = nil,
        _measure = measure,
    }
end

---@param session RollbackSession
---@param tick integer
---@param slot_index integer
---@param sample InputSample
---@return RollbackSessionArrival?, string?, RollbackInputHistoryErrorCode?
function rollback_session.add_authoritative(session, tick, slot_index, sample)
    assert_session(session)
    local used = rollback_input_history.record(session._input_history, tick)
    local arrival, err, code =
        rollback_input_history.add_authoritative(session._input_history, tick, slot_index, sample)
    if not arrival then
        if code == "outside_window" and session._status ~= "late_input_unrecoverable" then
            session._status = "late_input_unrecoverable"
            session._late_window_failures = session._late_window_failures + 1
            session._late_input_tick = tick
        end
        return nil, err, code
    end
    local correction = used ~= nil
            and used.slots[slot_index] ~= nil
            and sample_differs(sample, used.slots[slot_index])
        or false
    if correction and not arrival.duplicate then
        session._correction_count = session._correction_count + 1
    end
    return {
        duplicate = arrival.duplicate,
        confirmed_tick = arrival.confirmed_tick,
        earliest_divergence = arrival.earliest_divergence,
        correction = correction and not arrival.duplicate,
    }
end

---@param session RollbackSession
---@return RollbackTickOutput?, string?, RollbackSessionErrorCode?
function rollback_session.step(session)
    assert_session(session)
    if session._status == "late_input_unrecoverable" then
        return nil,
            "rollback session cannot progress after an over-window correction",
            "late_input_unrecoverable"
    end
    if session._state.finished then
        session._status = "finished"
        return nil, "rollback session cannot simulate after full time", "match_finished"
    end
    local output = measured(session, "tick", function()
        return execute_tick(session, session._state.input_tick)
    end)
    ---@cast output RollbackTickOutput
    session._status = session._state.finished and "finished" or "active"
    prune_retained_outputs(session)
    return copy_output(output)
end

---@param session RollbackSession
---@param causal_tick integer?
---@param restore_status RollbackSnapshotLookupStatus?
---@return RollbackReconcileResult
local function unchanged_reconcile_result(session, causal_tick, restore_status)
    local present = session._state.input_tick
    return {
        changed = false,
        status = session._status,
        causal_tick = causal_tick,
        restored_boundary = nil,
        restore_status = restore_status,
        old_present_boundary = present,
        new_present_boundary = present,
        corrected_from_tick = nil,
        corrected_through_tick = nil,
        replaced_from_tick = nil,
        replaced_through_tick = nil,
        corrected_outputs = {},
        old_present_hash = nil,
        new_present_hash = nil,
        first_difference = nil,
    }
end

---@param session RollbackSession
---@param causal_tick integer
---@param detailed_diagnostics boolean
---@return RollbackReconcileResult
local function reconcile_changed(session, causal_tick, detailed_diagnostics)
    local old_present = session._state.input_tick
    local restore_status = rollback_snapshot_history.status(session._snapshot_history, causal_tick)
    if restore_status == "outside_window" then
        session._status = "late_input_unrecoverable"
        session._late_window_failures = session._late_window_failures + 1
        session._late_input_tick = causal_tick
        return unchanged_reconcile_result(session, causal_tick, restore_status)
    end
    assert(
        restore_status ~= "missing",
        ("rollback snapshot invariant: boundary %d is missing before correction of present %d"):format(
            causal_tick,
            old_present
        )
    )
    ---@type MatchSnapshot?
    local old_snapshot = nil
    local old_hash = nil
    if detailed_diagnostics then
        old_snapshot = measured(session, "capture", function()
            return match_snapshot.capture_owned(session._state, session._combat_state)
        end)
        old_hash =
            assert(rollback_snapshot_history.boundary_hash(session._snapshot_history, old_present))
    end
    assert(
        rollback_input_history.consume_earliest_divergence(session._input_history) == causal_tick,
        "rollback divergence changed before restore"
    )
    local restored = measured(session, "restore", function()
        local restored_state, restored_combat =
            rollback_snapshot_history.restore_simulation(session._snapshot_history, causal_tick)
        return { state = assert(restored_state), combat = restored_combat }
    end)
    ---@cast restored { state: MatchState, combat: CombatMatchState? }
    session._state = restored.state
    session._combat_state = restored.combat

    local corrected_outputs = {}
    measured(session, "resimulation", function()
        local tick = causal_tick
        while tick < old_present and not session._state.finished do
            local output = execute_tick(session, tick)
            corrected_outputs[#corrected_outputs + 1] = copy_output(output)
            session._resimulated_ticks = session._resimulated_ticks + 1
            tick = tick + 1
        end
    end)

    local new_present = session._state.input_tick
    if new_present < old_present then
        assert(rollback_snapshot_history.truncate_after(session._snapshot_history, new_present))
        assert(rollback_input_history.truncate_from(session._input_history, new_present))
        for output_tick in pairs(session._outputs) do
            if output_tick >= new_present then
                session._outputs[output_tick] = nil
            end
        end
    end
    prune_retained_outputs(session)

    local new_hash = nil
    local first_difference = nil
    if detailed_diagnostics then
        new_hash =
            assert(rollback_snapshot_history.boundary_hash(session._snapshot_history, new_present))
        if old_hash ~= new_hash then
            first_difference = rollback_snapshot_history.first_difference(
                session._snapshot_history,
                new_present,
                assert(old_snapshot)
            )
        end
    end
    local depth = old_present - causal_tick
    session._rollback_count = session._rollback_count + 1
    session._latest_rollback_depth = depth
    session._max_rollback_depth = math.max(session._max_rollback_depth, depth)
    session._status = session._state.finished and "finished" or "active"
    session._last_rollback = {
        causal_tick = causal_tick,
        restored_boundary = causal_tick,
        old_present_boundary = old_present,
        new_present_boundary = new_present,
        old_present_hash = old_hash,
        new_present_hash = new_hash,
        first_difference = copy_difference(first_difference),
    }
    return {
        changed = true,
        status = session._status,
        causal_tick = causal_tick,
        restored_boundary = causal_tick,
        restore_status = restore_status,
        old_present_boundary = old_present,
        new_present_boundary = new_present,
        corrected_from_tick = #corrected_outputs > 0 and causal_tick or nil,
        corrected_through_tick = #corrected_outputs > 0 and new_present - 1 or nil,
        replaced_from_tick = causal_tick,
        replaced_through_tick = old_present > causal_tick and old_present - 1 or nil,
        corrected_outputs = corrected_outputs,
        old_present_hash = old_hash,
        new_present_hash = new_hash,
        first_difference = copy_difference(first_difference),
    }
end

---@param session RollbackSession
---@param detailed_diagnostics boolean? Compute predicted-versus-corrected hashes and difference.
---@return RollbackReconcileResult
function rollback_session.reconcile(session, detailed_diagnostics)
    assert_session(session)
    assert(
        detailed_diagnostics == nil or type(detailed_diagnostics) == "boolean",
        "rollback detailed diagnostics flag must be boolean"
    )
    if session._status == "late_input_unrecoverable" then
        local late_tick = assert(
            session._late_input_tick,
            "unrecoverable rollback session is missing its causal late input tick"
        )
        return unchanged_reconcile_result(session, late_tick, "outside_window")
    end
    local causal_tick = rollback_input_history.earliest_divergence(session._input_history)
    if causal_tick == nil then
        return unchanged_reconcile_result(session, nil, nil)
    end
    local result = measured(session, "rollback", function()
        return reconcile_changed(session, causal_tick, detailed_diagnostics == true)
    end)
    ---@cast result RollbackReconcileResult
    return result
end

---@param session RollbackSession
---@return MatchSnapshot
function rollback_session.current_snapshot(session)
    assert_session(session)
    return match_snapshot.capture_owned(session._state, session._combat_state)
end

---@param session RollbackSession
---@param boundary_tick integer
---@return RollbackSnapshotLookup
function rollback_session.snapshot(session, boundary_tick)
    assert_session(session)
    return rollback_snapshot_history.lookup(session._snapshot_history, boundary_tick)
end

---@param session RollbackSession
---@param expected RollbackSnapshotHistory
---@param boundary_tick integer
---@return RollbackSnapshotHistoryComparison
function rollback_session.compare_retained(session, expected, boundary_tick)
    assert_session(session)
    return rollback_snapshot_history.compare(expected, session._snapshot_history, boundary_tick)
end

---@param session RollbackSession
---@param input_tick integer
---@return RollbackTickOutput?
function rollback_session.output(session, input_tick)
    assert_session(session)
    local output = session._outputs[input_tick]
    return output and copy_output(output) or nil
end

---@param session RollbackSession
---@param expected MatchSnapshot
---@param causal_tick integer?
---@return RollbackComparison
function rollback_session.compare(session, expected, causal_tick)
    assert_session(session)
    local actual = match_snapshot.capture_owned(session._state, session._combat_state)
    local actual_hash = match_snapshot.hash(actual)
    local expected_hash = match_snapshot.hash(expected)
    local matched = actual_hash == expected_hash
    local first_difference = nil
    if not matched then
        first_difference = match_snapshot.first_difference(expected, actual)
    end
    local comparison = {
        matched = matched,
        boundary_mismatch = actual.state.input_tick ~= expected.state.input_tick,
        actual_boundary = actual.state.input_tick,
        expected_boundary = expected.state.input_tick,
        actual_hash = actual_hash,
        expected_hash = expected_hash,
        causal_tick = causal_tick,
        first_difference = first_difference,
    }
    session._last_comparison = copy_comparison(comparison)
    return copy_comparison(comparison)
end

---@param session RollbackSession
---@return RollbackSessionDiagnostics
function rollback_session.diagnostics(session)
    assert_session(session)
    local confirmed = rollback_input_history.confirmed_tick(session._input_history)
    local output_ceiling = session._state.input_tick - 1
    return {
        status = session._status,
        present_boundary = session._state.input_tick,
        confirmed_tick = confirmed,
        confirmed_output_tick = math.min(confirmed, output_ceiling),
        rollback_count = session._rollback_count,
        correction_count = session._correction_count,
        predicted_slot_samples = session._predicted_slot_samples,
        predicted_ticks = session._predicted_ticks,
        latest_rollback_depth = session._latest_rollback_depth,
        max_rollback_depth = session._max_rollback_depth,
        resimulated_ticks = session._resimulated_ticks,
        late_window_failures = session._late_window_failures,
        last_rollback = session._last_rollback and copy_last_rollback(session._last_rollback)
            or nil,
        last_comparison = session._last_comparison and copy_comparison(session._last_comparison)
            or nil,
        input_history = rollback_input_history.diagnostics(session._input_history),
        snapshot_history = rollback_snapshot_history.diagnostics(session._snapshot_history),
    }
end

-- Exact logical payload retained by the rollback coordinator. Snapshot bytes
-- use the MatchSnapshot versioned canonical encoding; input/output bytes use their versioned
-- canonical encodings owned by their respective modules.
---@param session RollbackSession
---@return RollbackSessionAccounting
function rollback_session.accounting(session)
    assert_session(session)
    local input = rollback_input_history.accounting(session._input_history)
    local output_bytes = 0
    for _, output in pairs(session._outputs) do
        output_bytes = output_bytes + #canonical_payload(output)
    end
    local snapshot_bytes =
        rollback_snapshot_history.diagnostics(session._snapshot_history).canonical_bytes
    return {
        input = input,
        output_bytes = output_bytes,
        snapshot_bytes = snapshot_bytes,
        total_bytes = input.total_bytes + output_bytes + snapshot_bytes,
    }
end

return rollback_session
