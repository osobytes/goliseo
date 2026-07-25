-- Pure authoritative-reference rollback laboratory. The reference consumes
-- only already-materialized tape frames; the client receives those same rows
-- immediately for local slots or through deterministic network conditions for
-- remote slots.

local fnv1a64 = require("core.fnv1a64")
local network_profiles = require("data.network_profiles")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local network_conditions = require("sim.network_conditions")
local rollback_events = require("sim.rollback_events")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_session = require("sim.rollback_session")
local rollback_snapshot_history = require("sim.rollback_snapshot_history")

---@alias RollbackLabStatus
---| "converged"
---| "diverged"
---| "late_input_unrecoverable"
---| "unconfirmed_authority"
---| "drain_incomplete"
---| "incomplete_client"
---| "comparison_history_missing"
---| "event_diverged"

---@class RollbackLabCorruption
---@field tick integer
---@field slot integer

---@class RollbackLabOptions
---@field profile_name string?
---@field profile NetworkProfile?
---@field network_seed integer?
---@field sources RollbackInputSource[]?
---@field max_rollback_ticks integer?
---@field drain_ticks integer?
---@field corruption RollbackLabCorruption?
---@field measure RollbackSessionMeasure? Optional runner-owned wall-time observer.
---@field prevalidated_tape boolean? Skip full replay after structural validation.

---@class RollbackLabDepth
---@field depth integer
---@field count integer

---@class RollbackLabDivergence
---@field causal_tick integer
---@field boundary integer
---@field expected_hash string
---@field actual_hash string
---@field first_difference MatchSnapshotDifference?

---@class RollbackLabPeaks
---@field snapshot_count integer
---@field snapshot_bytes integer
---@field input_authoritative_samples integer
---@field input_effective_ticks integer
---@field input_record_ticks integer
---@field network_pending_envelopes integer
---@field network_pending_record_references integer
---@field network_delivered_ledger_entries integer
---@field network_authoritative_records integer
---@field input_bytes integer
---@field output_bytes integer
---@field event_bytes integer
---@field history_bytes integer

---@class RollbackLabHistoryAccounting
---@field input RollbackInputHistoryAccounting
---@field output_bytes integer
---@field snapshot_bytes integer
---@field event_bytes integer
---@field total_bytes integer

---@class RollbackLabMetrics
---@field predicted_slot_samples integer
---@field predicted_ticks integer
---@field correction_count integer
---@field rollback_count integer
---@field resimulated_ticks integer
---@field latest_rollback_depth integer
---@field max_rollback_depth integer
---@field rollback_depths RollbackLabDepth[]
---@field predicted_early_finish boolean
---@field reactivation_count integer
---@field confirmed_redundant_rows_skipped integer
---@field compared_boundaries integer
---@field expected_boundaries integer
---@field current_snapshot_count integer
---@field current_snapshot_bytes integer
---@field peaks RollbackLabPeaks

---@class RollbackLabEventMetrics
---@field added integer
---@field revoked integer
---@field replaced integer
---@field confirmed_steps integer
---@field confirmed_match_events integer
---@field confirmed_combat_events integer
---@field confirmed_lifecycle_events integer
---@field speculative_residue integer
---@field reference_digest string
---@field confirmed_digest string
---@field matched boolean

---@class RollbackLabEventTraceRow
---@field kind "reference_confirmed"|"impaired_diff"|"impaired_confirmed"|"replay_boundary"|"replay_truncate"
---@field step RollbackEventStep?
---@field diff RollbackEventDiff?
---@field boundary integer?
---@field replay_sample RollbackLabReplaySample?

---@class RollbackLabReplaySample
---@field boundary integer
---@field ball_x number
---@field ball_y number
---@field score_home integer
---@field score_away integer

---@class RollbackLabDrainSummary
---@field final_tick integer
---@field complete boolean
---@field pending integer
---@field recovered integer
---@field requested integer

---@class RollbackLabResult
---@field schema integer
---@field success boolean
---@field status RollbackLabStatus
---@field fixture string
---@field fixture_seed integer
---@field profile string
---@field profile_parameters string
---@field network_seed integer
---@field source_pattern string
---@field input_ticks integer
---@field tape_digest string
---@field initial_hash string
---@field reference_final_boundary integer
---@field client_final_boundary integer
---@field reference_final_hash string
---@field client_final_hash string
---@field reference_final_snapshot MatchSnapshot
---@field client_final_snapshot MatchSnapshot
---@field confirmed_tick integer
---@field confirmed_output_tick integer
---@field late_input_tick integer?
---@field unconfirmed_tick integer?
---@field divergence RollbackLabDivergence?
---@field metrics RollbackLabMetrics
---@field input_diagnostics RollbackInputHistoryDiagnostics
---@field snapshot_diagnostics RollbackSnapshotHistoryDiagnostics
---@field network_counters NetworkConditionCounters
---@field network_diagnostics NetworkConditionDiagnostics
---@field event_metrics RollbackLabEventMetrics
---@field event_diagnostics RollbackEventsDiagnostics
---@field event_trace RollbackLabEventTraceRow[]
---@field history_accounting RollbackLabHistoryAccounting
---@field drain RollbackLabDrainSummary

---@class RollbackLabRunState
---@field session RollbackSession
---@field network NetworkConditions
---@field reference MatchState
---@field reference_combat CombatMatchState?
---@field reference_history RollbackSnapshotHistory
---@field reference_events RollbackEventTimeline
---@field events RollbackEventTimeline
---@field event_trace RollbackLabEventTraceRow[]
---@field event_window_exceeded boolean
---@field sources RollbackInputSource[]
---@field depth_counts table<integer, integer>
---@field last_compared_output integer
---@field compared_boundaries integer
---@field divergence RollbackLabDivergence?
---@field late_input_tick integer?
---@field unconfirmed_tick integer?
---@field comparison_history_missing boolean
---@field predicted_early_finish boolean
---@field reactivation_count integer
---@field confirmed_redundant_rows_skipped integer
---@field peaks RollbackLabPeaks

---@class RollbackLabCampaign
---@field tape InputTape
---@field options RollbackLabOptions
---@field state RollbackLabRunState
---@field profile NetworkProfile
---@field profile_name string
---@field network_seed integer
---@field drain_ticks integer
---@field next_frame_index integer
---@field result RollbackLabResult?

---@class RollbackLabModule
local rollback_lab = {}

rollback_lab.SCHEMA = 1
rollback_lab.DEFAULT_NETWORK_SEED = 7302
rollback_lab.DEFAULT_DRAIN_TICKS = 256

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param value boolean
---@return string
local function bool_marker(value)
    return value and "1" or "0"
end

---@param value any
---@return string
local function optional_marker(value)
    return value == nil and "none" or tostring(value)
end

local HEX = "0123456789abcdef"

---@param value string
---@return string
local function marker_string(value)
    local encoded = {}
    for index = 1, #value do
        local byte = value:byte(index)
        local high = math.floor(byte / 16)
        local low = byte % 16
        encoded[#encoded + 1] = HEX:sub(high + 1, high + 1)
        encoded[#encoded + 1] = HEX:sub(low + 1, low + 1)
    end
    return #value .. ":" .. table.concat(encoded)
end

---@param value string?
---@return string
local function optional_marker_string(value)
    if value == nil then
        return "none"
    end
    return "some:" .. marker_string(value)
end

---@param state Fnv1a64State
---@param value string
local function digest_segment(state, value)
    fnv1a64.update(state, tostring(#value) .. ":" .. value)
end

---@param key any
---@return string
local function canonical_key(key)
    return type(key) .. ":" .. tostring(key)
end

---@param state Fnv1a64State
---@param value any
local function digest_value(state, value)
    local kind = type(value)
    digest_segment(state, kind)
    if kind == "number" then
        digest_segment(state, match_snapshot.number_bytes(value))
    elseif kind == "boolean" then
        digest_segment(state, value and "1" or "0")
    elseif kind == "string" then
        digest_segment(state, value)
    elseif kind == "nil" then
        digest_segment(state, "")
    elseif kind == "table" then
        local keys = {}
        for key in pairs(value) do
            keys[#keys + 1] = key
        end
        table.sort(keys, function(left, right)
            return canonical_key(left) < canonical_key(right)
        end)
        digest_segment(state, tostring(#keys))
        for _, key in ipairs(keys) do
            digest_value(state, key)
            digest_value(state, value[key])
        end
    else
        assert(false, "rollback lab digest cannot encode " .. kind)
    end
end

---@param state RollbackLabRunState
---@return RollbackLabEventMetrics
local function event_metrics(state)
    local reference = fnv1a64.new()
    local confirmed = fnv1a64.new()
    local metrics = {
        added = 0,
        revoked = 0,
        replaced = 0,
        confirmed_steps = 0,
        confirmed_match_events = 0,
        confirmed_combat_events = 0,
        confirmed_lifecycle_events = 0,
        speculative_residue = rollback_events.diagnostics(state.events).retained_step_count,
        reference_digest = "",
        confirmed_digest = "",
        matched = false,
    }
    for _, row in ipairs(state.event_trace) do
        if row.kind == "impaired_diff" then
            local diff = assert(row.diff, "rollback lab impaired diff trace row is empty")
            metrics.added = metrics.added + #diff.added
            metrics.revoked = metrics.revoked + #diff.revoked
            metrics.replaced = metrics.replaced + #diff.replaced
        elseif row.kind == "reference_confirmed" then
            digest_value(reference, assert(row.step, "rollback lab reference trace row is empty"))
        elseif row.kind == "impaired_confirmed" then
            local step = assert(row.step, "rollback lab confirmed trace row is empty")
            metrics.confirmed_steps = metrics.confirmed_steps + 1
            metrics.confirmed_match_events = metrics.confirmed_match_events + #step.match_events
            metrics.confirmed_combat_events = metrics.confirmed_combat_events
                + #(step.combat_events or {})
            metrics.confirmed_lifecycle_events = metrics.confirmed_lifecycle_events
                + #step.lifecycle_events
            digest_value(confirmed, step)
        end
    end
    metrics.reference_digest = fnv1a64.hex(reference)
    metrics.confirmed_digest = fnv1a64.hex(confirmed)
    metrics.matched = metrics.reference_digest == metrics.confirmed_digest
        and metrics.confirmed_steps == state.reference.input_tick
        and metrics.speculative_residue == 0
    return metrics
end

---@param tape InputTape
---@return string
local function tape_digest(tape)
    local state = fnv1a64.new()
    digest_segment(state, match_snapshot.number_bytes(tape.version))
    local identity = tape.identity
    for _, field in ipairs({
        "tape_version",
        "input_version",
        "snapshot_version",
        "seed",
        "tick_rate",
    }) do
        digest_segment(state, match_snapshot.number_bytes(identity[field]))
    end
    for _, field in ipairs({
        "build",
        "source",
        "content",
        "tuning",
        "config",
        "fixture",
    }) do
        digest_segment(state, identity[field])
    end
    if identity.combat then
        digest_segment(state, identity.combat)
    end
    digest_segment(state, match_snapshot.encode(tape.initial))
    for _, frame in ipairs(tape.frames) do
        digest_segment(state, assert(input_frame.encode(frame)))
    end
    for _, hash in ipairs(tape.boundary_hashes) do
        digest_segment(state, hash)
    end
    return fnv1a64.hex(state)
end

---@param tape InputTape
---@return string
function rollback_lab.tape_digest(tape)
    assert(input_tape.validate_structure(tape))
    return tape_digest(tape)
end

---@param measure RollbackSessionMeasure?
---@param operation fun(): any
---@return any
local function capture(measure, operation)
    if measure == nil then
        return operation()
    end
    local calls = 0
    local result = nil
    local operation_succeeded = false
    local operation_error = nil
    measure("capture", function()
        calls = calls + 1
        assert(calls == 1, "rollback lab measurement operation must run exactly once")
        local ok, value = pcall(operation)
        operation_succeeded = ok
        if not ok then
            operation_error = value
            error(value, 0)
        end
        result = value
        return result
    end)
    assert(calls == 1, "rollback lab measurement observer must run its operation exactly once")
    if not operation_succeeded then
        error(operation_error, 0)
    end
    return result
end

---@param sample InputSample
---@return InputSample
local function copy_sample(sample)
    return assert(input_frame.new_sample(sample))
end

---@param event MatchEvent
---@return MatchEvent
local function copy_match_event(event)
    local copied = {}
    for key, value in pairs(event) do
        assert(type(value) ~= "table", "rollback lab match events must contain canonical scalars")
        copied[key] = value
    end
    ---@cast copied MatchEvent
    return copied
end

---@param frame InputFrame
---@param sources RollbackInputSource[]
---@param snapshot MatchSnapshot
---@return RollbackTickOutput
local function reference_output(frame, sources, snapshot)
    local records = {}
    for slot = 1, input_frame.SLOT_COUNT do
        records[slot] = {
            source = sources[slot],
            status = "authoritative",
            sample = copy_sample(frame.slots[slot]),
        }
    end
    local events = {}
    for index, event in ipairs(snapshot.state.events) do
        events[index] = copy_match_event(event)
    end
    local output = {
        tick = frame.tick,
        start_boundary = frame.tick,
        end_boundary = frame.tick + 1,
        input = { tick = frame.tick, slots = records },
        events = events,
        state = {
            score = {
                home = snapshot.state.score.home,
                away = snapshot.state.score.away,
            },
            time_left = snapshot.state.time_left,
            finished = snapshot.state.finished,
        },
        finished = snapshot.state.finished,
    }
    if snapshot.combat then
        local combat_events = {}
        for index, event in ipairs(snapshot.combat.events) do
            local copied = {}
            for key, value in pairs(event) do
                assert(
                    type(value) ~= "table",
                    "rollback lab combat events must contain canonical scalars"
                )
                copied[key] = value
            end
            combat_events[index] = copied
        end
        output.combat_events = combat_events
    end
    return output
end

---@param state RollbackLabRunState
---@param output RollbackTickOutput
---@return RollbackEventStepInput
local function event_step(state, output)
    local lookup = rollback_session.snapshot(state.session, output.end_boundary)
    assert(
        lookup.status == "present" or lookup.status == "retained",
        "rollback lab event boundary is not retained"
    )
    return {
        output = output,
        snapshot = assert(lookup.snapshot, "rollback lab event boundary snapshot is missing"),
    }
end

---@param state RollbackLabRunState
---@param boundary integer
---@param supplied MatchSnapshot?
local function trace_replay_boundary(state, boundary, supplied)
    local snapshot = supplied
    if snapshot == nil then
        local lookup = rollback_session.snapshot(state.session, boundary)
        assert(
            lookup.status == "present" or lookup.status == "retained",
            "rollback lab replay boundary is not retained"
        )
        snapshot = assert(lookup.snapshot, "rollback lab replay boundary snapshot is missing")
    end
    state.event_trace[#state.event_trace + 1] = {
        kind = "replay_boundary",
        boundary = boundary,
        replay_sample = {
            boundary = boundary,
            ball_x = snapshot.state.ball.x,
            ball_y = snapshot.state.ball.y,
            score_home = snapshot.state.score.home,
            score_away = snapshot.state.score.away,
        },
    }
end

---@param state RollbackLabRunState
---@param diff RollbackEventDiff
local function trace_diff(state, diff)
    if #diff.added > 0 or #diff.revoked > 0 or #diff.replaced > 0 then
        state.event_trace[#state.event_trace + 1] = {
            kind = "impaired_diff",
            diff = diff,
        }
    end
end

---@param state RollbackLabRunState
---@param from_tick integer
---@param through_tick integer
---@param outputs RollbackTickOutput[]
---@return boolean
local function apply_client_outputs(state, from_tick, through_tick, outputs)
    if state.event_window_exceeded then
        return false
    end
    local steps = {}
    for index, output in ipairs(outputs) do
        steps[index] = event_step(state, output)
    end
    local diff, err = rollback_events.apply(state.events, from_tick, through_tick, steps)
    if diff == nil then
        assert(
            rollback_events.diagnostics(state.events).status == "unconfirmed_window_exceeded",
            err or "rollback lab event application failed"
        )
        state.event_window_exceeded = true
        return false
    end
    trace_diff(state, diff)
    for index, output in ipairs(outputs) do
        trace_replay_boundary(state, output.end_boundary, steps[index].snapshot)
    end
    return true
end

---@param state RollbackLabRunState
local function publish_confirmation(state)
    if state.event_window_exceeded then
        return
    end
    local confirmed_output = rollback_session.diagnostics(state.session).confirmed_output_tick
    for _, step in ipairs(rollback_events.confirm(state.events, confirmed_output)) do
        state.event_trace[#state.event_trace + 1] = {
            kind = "impaired_confirmed",
            step = step,
        }
    end
end

---@param state RollbackLabRunState
---@param frame InputFrame
---@param snapshot MatchSnapshot
local function publish_reference(state, frame, snapshot)
    local output = reference_output(frame, state.sources, snapshot)
    local diff, err = rollback_events.apply(state.reference_events, frame.tick, frame.tick, {
        { output = output, snapshot = snapshot },
    })
    assert(diff, err or "rollback lab reference event application failed")
    local confirmed = rollback_events.confirm(state.reference_events, frame.tick)
    assert(#confirmed == 1, "rollback lab reference must confirm exactly one step")
    state.event_trace[#state.event_trace + 1] = {
        kind = "reference_confirmed",
        step = confirmed[1],
    }
end

---@return RollbackInputSource[]
local function default_sources()
    local sources = {}
    for slot = 1, input_frame.SLOT_COUNT do
        sources[slot] = slot <= input_frame.HOME_SLOT_COUNT and "local" or "remote"
    end
    return sources
end

---@param supplied RollbackInputSource[]?
---@return RollbackInputSource[]
local function copy_sources(supplied)
    supplied = supplied or default_sources()
    assert(type(supplied) == "table", "rollback lab sources must be an array")
    assert(#supplied == input_frame.SLOT_COUNT, "rollback lab requires eight input sources")
    local sources = {}
    for slot = 1, input_frame.SLOT_COUNT do
        local source = supplied[slot]
        assert(
            source == "local" or source == "remote",
            "rollback lab sources must be local or remote"
        )
        sources[slot] = source
    end
    for key in pairs(supplied) do
        assert(
            type(key) == "number"
                and key == math.floor(key)
                and key >= 1
                and key <= input_frame.SLOT_COUNT,
            "rollback lab sources must be a canonical array"
        )
    end
    return sources
end

---@param sources RollbackInputSource[]
---@return string
local function source_pattern(sources)
    local parts = {}
    for index, source in ipairs(sources) do
        parts[index] = source == "local" and "L" or "R"
    end
    return table.concat(parts)
end

---@param profile NetworkProfile
---@return string
local function profile_parameters(profile)
    return table.concat({
        match_snapshot.number_bytes(profile.base_delay_ticks),
        match_snapshot.number_bytes(profile.jitter_min_ticks),
        match_snapshot.number_bytes(profile.jitter_max_ticks),
        match_snapshot.number_bytes(profile.independent_loss_rate),
        match_snapshot.number_bytes(profile.duplication_rate),
        match_snapshot.number_bytes(profile.burst_start_rate),
        match_snapshot.number_bytes(profile.burst_length_ticks),
    }, ",")
end

---@param options RollbackLabOptions
---@return NetworkProfile
---@return string
local function selected_profile(options)
    if options.profile then
        return options.profile, options.profile_name or "custom"
    end
    local name = options.profile_name or "clean"
    local profile = network_profiles[name]
    assert(profile ~= nil, "unknown rollback lab profile: " .. name)
    return profile, name
end

---@param corruption RollbackLabCorruption?
---@param tick integer
---@param slot integer
---@param sample InputSample
---@return InputSample
local function client_sample(corruption, tick, slot, sample)
    local copied = copy_sample(sample)
    if corruption == nil or corruption.tick ~= tick or corruption.slot ~= slot then
        return copied
    end
    copied.move_x = copied.move_x == input_frame.MOVE_SCALE and -input_frame.MOVE_SCALE
        or input_frame.MOVE_SCALE
    copied.edges = copied.edges == input_frame.EDGE_BITS.dash and 0 or input_frame.EDGE_BITS.dash
    return copied
end

---@param state RollbackLabRunState
local function update_peaks(state)
    local session = rollback_session.diagnostics(state.session)
    local snapshots = session.snapshot_history
    local inputs = session.input_history
    local network = network_conditions.diagnostics(state.network)
    local accounting = rollback_session.accounting(state.session)
    local event_accounting = rollback_events.accounting(state.events)
    local peaks = state.peaks
    peaks.snapshot_count = math.max(peaks.snapshot_count, snapshots.peak_retained_boundary_count)
    peaks.snapshot_bytes = math.max(peaks.snapshot_bytes, snapshots.peak_canonical_bytes)
    peaks.input_authoritative_samples =
        math.max(peaks.input_authoritative_samples, inputs.authoritative_sample_count)
    peaks.input_effective_ticks = math.max(peaks.input_effective_ticks, inputs.effective_tick_count)
    peaks.input_record_ticks = math.max(peaks.input_record_ticks, inputs.record_tick_count)
    peaks.network_pending_envelopes =
        math.max(peaks.network_pending_envelopes, network.peak_pending_envelopes)
    peaks.network_pending_record_references =
        math.max(peaks.network_pending_record_references, network.peak_pending_record_references)
    peaks.network_delivered_ledger_entries =
        math.max(peaks.network_delivered_ledger_entries, network.peak_delivered_ledger_entries)
    peaks.network_authoritative_records =
        math.max(peaks.network_authoritative_records, network.peak_retained_authoritative_records)
    peaks.input_bytes = math.max(peaks.input_bytes, accounting.input.total_bytes)
    peaks.output_bytes = math.max(peaks.output_bytes, accounting.output_bytes)
    peaks.event_bytes = math.max(peaks.event_bytes, event_accounting.total_bytes)
    peaks.history_bytes =
        math.max(peaks.history_bytes, accounting.total_bytes + event_accounting.total_bytes)
end

---@param state RollbackLabRunState
local function detect_unconfirmed_floor(state)
    local diagnostics = rollback_session.diagnostics(state.session)
    local input = diagnostics.input_history
    if input.oldest_retained_tick > diagnostics.confirmed_tick + 1 then
        local missing = diagnostics.confirmed_tick + 1
        if state.unconfirmed_tick == nil or missing < state.unconfirmed_tick then
            state.unconfirmed_tick = missing
        end
    end
end

---@param state RollbackLabRunState
---@param expected MatchSnapshot
---@param actual MatchSnapshot
---@param boundary integer
local function compare_snapshots(state, expected, actual, boundary)
    local expected_hash = match_snapshot.hash(expected)
    local actual_hash = match_snapshot.hash(actual)
    state.compared_boundaries = state.compared_boundaries + 1
    if expected_hash ~= actual_hash and state.divergence == nil then
        state.divergence = {
            causal_tick = math.max(0, boundary - 1),
            boundary = boundary,
            expected_hash = expected_hash,
            actual_hash = actual_hash,
            first_difference = match_snapshot.first_difference(expected, actual),
        }
    end
end

---@param state RollbackLabRunState
local function compare_newly_confirmed(state)
    local diagnostics = rollback_session.diagnostics(state.session)
    while state.last_compared_output < diagnostics.confirmed_output_tick do
        local output_tick = state.last_compared_output + 1
        local boundary = output_tick + 1
        local comparison =
            rollback_session.compare_retained(state.session, state.reference_history, boundary)
        if
            (comparison.expected_status ~= "present" and comparison.expected_status ~= "retained")
            or (comparison.actual_status ~= "present" and comparison.actual_status ~= "retained")
        then
            state.comparison_history_missing = true
            return
        end
        state.compared_boundaries = state.compared_boundaries + 1
        if not comparison.matched and state.divergence == nil then
            state.divergence = {
                causal_tick = math.max(0, boundary - 1),
                boundary = boundary,
                expected_hash = assert(comparison.expected_hash),
                actual_hash = assert(comparison.actual_hash),
                first_difference = comparison.first_difference,
            }
        end
        state.last_compared_output = output_tick
    end
end

---@param state RollbackLabRunState
---@param tick integer
---@param slot integer
---@param sample InputSample
local function add_client_authority(state, tick, slot, sample)
    local arrival, err, code = rollback_session.add_authoritative(state.session, tick, slot, sample)
    if arrival then
        return
    end
    if code == "outside_window" then
        if state.late_input_tick == nil or tick < state.late_input_tick then
            state.late_input_tick = tick
        end
        return
    end
    assert(false, err or ("rollback lab rejected authority with " .. tostring(code)))
end

---@param state RollbackLabRunState
---@param deliveries NetworkDelivery[]
local function process_delivery_batch(state, deliveries)
    if #deliveries == 0 then
        return
    end
    local before = rollback_session.diagnostics(state.session).status
    for _, delivery in ipairs(deliveries) do
        for _, record in ipairs(network_conditions.records(delivery)) do
            local confirmed_tick = rollback_session.diagnostics(state.session).confirmed_tick
            if record.tick <= confirmed_tick then
                state.confirmed_redundant_rows_skipped = state.confirmed_redundant_rows_skipped + 1
            else
                add_client_authority(state, record.tick, delivery.source_slot, record.sample)
            end
        end
    end
    local reconciled = rollback_session.reconcile(state.session)
    if reconciled.changed then
        local depth = reconciled.old_present_boundary - assert(reconciled.causal_tick)
        state.depth_counts[depth] = (state.depth_counts[depth] or 0) + 1
        local replay_boundary = assert(reconciled.replaced_from_tick)
        state.event_trace[#state.event_trace + 1] = {
            kind = "replay_truncate",
            boundary = replay_boundary,
        }
        trace_replay_boundary(state, replay_boundary)
        apply_client_outputs(
            state,
            replay_boundary,
            assert(reconciled.replaced_through_tick),
            reconciled.corrected_outputs
        )
    end
    if before == "finished" and reconciled.status == "active" then
        state.reactivation_count = state.reactivation_count + 1
    end
    update_peaks(state)
    detect_unconfirmed_floor(state)
end

---@param state RollbackLabRunState
---@param target_boundary integer
local function catch_up_client(state, target_boundary)
    while true do
        local diagnostics = rollback_session.diagnostics(state.session)
        if diagnostics.present_boundary >= target_boundary then
            break
        end
        if diagnostics.status == "late_input_unrecoverable" then
            break
        end
        if diagnostics.status == "finished" then
            state.predicted_early_finish = true
            break
        end
        local output, err, code = rollback_session.step(state.session)
        if output == nil then
            assert(
                code == "match_finished" or code == "late_input_unrecoverable",
                err or "rollback lab client step failed"
            )
            if code == "match_finished" then
                state.predicted_early_finish = true
            end
            break
        end
        apply_client_outputs(state, output.tick, output.tick, { output })
        update_peaks(state)
        detect_unconfirmed_floor(state)
    end
end

---@param state RollbackLabRunState
---@param deliveries NetworkDelivery[]
local function process_drain_deliveries(state, deliveries)
    local group = {}
    local arrival_tick = nil
    for _, delivery in ipairs(deliveries) do
        if arrival_tick ~= nil and delivery.arrival_tick ~= arrival_tick then
            process_delivery_batch(state, group)
            publish_confirmation(state)
            catch_up_client(state, state.reference.input_tick)
            publish_confirmation(state)
            compare_newly_confirmed(state)
            group = {}
        end
        arrival_tick = delivery.arrival_tick
        group[#group + 1] = delivery
    end
    if #group > 0 then
        process_delivery_batch(state, group)
        publish_confirmation(state)
        catch_up_client(state, state.reference.input_tick)
        publish_confirmation(state)
        compare_newly_confirmed(state)
    end
end

---@param counts table<integer, integer>
---@return RollbackLabDepth[]
local function sorted_depths(counts)
    local depths = {}
    for depth in pairs(counts) do
        depths[#depths + 1] = depth
    end
    table.sort(depths)
    local result = {}
    for index, depth in ipairs(depths) do
        result[index] = { depth = depth, count = assert(counts[depth]) }
    end
    return result
end

---@param sources RollbackInputSource[]
---@param last_tick integer
---@return NetworkResendRequest[]
local function drain_requests(sources, last_tick)
    local requests = {}
    for slot, source in ipairs(sources) do
        if source == "remote" then
            requests[#requests + 1] = { source_slot = slot, input_tick = last_tick }
        end
    end
    return requests
end

---@param state RollbackLabRunState
---@param tape InputTape
---@param profile_name string
---@param profile NetworkProfile
---@param network_seed integer
---@param drain NetworkDrainResult
---@return RollbackLabResult
local function finish_result(state, tape, profile_name, profile, network_seed, drain)
    local session = rollback_session.diagnostics(state.session)
    local reference_snapshot = match_snapshot.capture_owned(state.reference, state.reference_combat)
    local client_snapshot = rollback_session.current_snapshot(state.session)
    local final_comparison =
        rollback_session.compare(state.session, reference_snapshot, state.late_input_tick)
    local events = event_metrics(state)
    local event_diagnostics = rollback_events.diagnostics(state.events)
    local session_accounting = rollback_session.accounting(state.session)
    local event_accounting = rollback_events.accounting(state.events)
    ---@type RollbackLabHistoryAccounting
    local history_accounting = {
        input = session_accounting.input,
        output_bytes = session_accounting.output_bytes,
        snapshot_bytes = session_accounting.snapshot_bytes,
        event_bytes = event_accounting.total_bytes,
        total_bytes = session_accounting.total_bytes + event_accounting.total_bytes,
    }
    local last_tick = #tape.frames - 1
    if session.confirmed_tick < last_tick and state.unconfirmed_tick == nil then
        state.unconfirmed_tick = session.confirmed_tick + 1
    end

    local status = "converged"
    if state.late_input_tick ~= nil or session.status == "late_input_unrecoverable" then
        status = "late_input_unrecoverable"
    elseif state.unconfirmed_tick ~= nil then
        status = "unconfirmed_authority"
    elseif not drain.complete or drain.pending ~= 0 then
        status = "drain_incomplete"
    elseif state.comparison_history_missing then
        status = "comparison_history_missing"
    elseif state.divergence ~= nil or not final_comparison.matched then
        status = "diverged"
    elseif not events.matched then
        status = "event_diverged"
    elseif session.present_boundary ~= state.reference.input_tick then
        status = "incomplete_client"
    end
    ---@cast status RollbackLabStatus

    local expected_boundaries = #tape.frames + 1
    local success = status == "converged"
        and session.confirmed_tick == last_tick
        and session.confirmed_output_tick == state.reference.input_tick - 1
        and state.compared_boundaries == expected_boundaries
        and network_conditions.pending(state.network) == 0
        and events.matched
    if not success and status == "converged" then
        status = "incomplete_client"
    end

    return {
        schema = rollback_lab.SCHEMA,
        success = success,
        status = status,
        fixture = tape.identity.fixture,
        fixture_seed = tape.identity.seed,
        profile = profile_name,
        profile_parameters = profile_parameters(profile),
        network_seed = network_seed,
        source_pattern = source_pattern(state.sources),
        input_ticks = #tape.frames,
        tape_digest = tape_digest(tape),
        initial_hash = match_snapshot.hash(tape.initial),
        reference_final_boundary = state.reference.input_tick,
        client_final_boundary = session.present_boundary,
        reference_final_hash = match_snapshot.hash(reference_snapshot),
        client_final_hash = match_snapshot.hash(client_snapshot),
        reference_final_snapshot = reference_snapshot,
        client_final_snapshot = client_snapshot,
        confirmed_tick = session.confirmed_tick,
        confirmed_output_tick = session.confirmed_output_tick,
        late_input_tick = state.late_input_tick,
        unconfirmed_tick = state.unconfirmed_tick,
        divergence = state.divergence,
        metrics = {
            predicted_slot_samples = session.predicted_slot_samples,
            predicted_ticks = session.predicted_ticks,
            correction_count = session.correction_count,
            rollback_count = session.rollback_count,
            resimulated_ticks = session.resimulated_ticks,
            latest_rollback_depth = session.latest_rollback_depth,
            max_rollback_depth = session.max_rollback_depth,
            rollback_depths = sorted_depths(state.depth_counts),
            predicted_early_finish = state.predicted_early_finish,
            reactivation_count = state.reactivation_count,
            confirmed_redundant_rows_skipped = state.confirmed_redundant_rows_skipped,
            compared_boundaries = state.compared_boundaries,
            expected_boundaries = expected_boundaries,
            current_snapshot_count = session.snapshot_history.retained_boundary_count,
            current_snapshot_bytes = session.snapshot_history.canonical_bytes,
            peaks = state.peaks,
        },
        input_diagnostics = session.input_history,
        snapshot_diagnostics = session.snapshot_history,
        network_counters = network_conditions.counters(state.network),
        network_diagnostics = network_conditions.diagnostics(state.network),
        event_metrics = events,
        event_diagnostics = event_diagnostics,
        event_trace = state.event_trace,
        history_accounting = history_accounting,
        drain = {
            final_tick = drain.final_tick,
            complete = drain.complete,
            pending = drain.pending,
            recovered = drain.recovered,
            requested = drain.requested,
        },
    }
end

-- Create an incremental authoritative-reference campaign. Runtime entrypoints
-- use this seam to yield between logical ticks in browsers; synchronous callers
-- keep using run(), which delegates to the same state machine.
---@param tape InputTape
---@param options RollbackLabOptions?
---@return RollbackLabCampaign
function rollback_lab.new_campaign(tape, options)
    options = options or {}
    assert(
        options.prevalidated_tape and input_tape.validate_structure(tape)
            or input_tape.validate(tape)
    )
    assert(#tape.frames > 0, "rollback lab tape must contain at least one input frame")
    local sources = copy_sources(options.sources)
    local profile, profile_name = selected_profile(options)
    local network_seed = options.network_seed or rollback_lab.DEFAULT_NETWORK_SEED
    assert(is_integer(network_seed), "rollback lab network seed must be an integer")
    local max_rollback_ticks = options.max_rollback_ticks
        or rollback_input_history.ROLLBACK_WINDOW_TICKS
    assert(
        is_integer(max_rollback_ticks)
            and max_rollback_ticks >= 0
            and max_rollback_ticks <= rollback_input_history.ROLLBACK_WINDOW_TICKS,
        "rollback lab rollback window must be a bounded non-negative integer"
    )
    local drain_ticks = options.drain_ticks or rollback_lab.DEFAULT_DRAIN_TICKS
    assert(is_integer(drain_ticks) and drain_ticks > 0, "rollback lab drain must be positive")
    if options.corruption then
        assert(
            is_integer(options.corruption.tick)
                and options.corruption.tick >= 0
                and options.corruption.tick < #tape.frames,
            "rollback lab corruption tick is outside the tape"
        )
        assert(
            is_integer(options.corruption.slot)
                and options.corruption.slot >= 1
                and options.corruption.slot <= input_frame.SLOT_COUNT,
            "rollback lab corruption slot is outside the input frame"
        )
    end

    local reference, reference_combat = match_snapshot.restore(tape.initial)
    local reference_history = rollback_snapshot_history.new(max_rollback_ticks)
    assert(rollback_snapshot_history.store(reference_history, tape.initial))
    local session = rollback_session.new(tape.initial, sources, max_rollback_ticks, options.measure)
    local network = network_conditions.new(profile, network_seed)
    ---@type RollbackLabRunState
    local state = {
        session = session,
        network = network,
        reference = reference,
        reference_combat = reference_combat,
        reference_history = reference_history,
        reference_events = rollback_events.new(tape.initial, math.max(1, max_rollback_ticks)),
        events = rollback_events.new(tape.initial, math.max(1, max_rollback_ticks)),
        event_trace = {},
        event_window_exceeded = false,
        sources = sources,
        depth_counts = {},
        last_compared_output = -1,
        compared_boundaries = 0,
        divergence = nil,
        late_input_tick = nil,
        unconfirmed_tick = nil,
        comparison_history_missing = false,
        predicted_early_finish = false,
        reactivation_count = 0,
        confirmed_redundant_rows_skipped = 0,
        peaks = {
            snapshot_count = 0,
            snapshot_bytes = 0,
            input_authoritative_samples = 0,
            input_effective_ticks = 0,
            input_record_ticks = 0,
            network_pending_envelopes = 0,
            network_pending_record_references = 0,
            network_delivered_ledger_entries = 0,
            network_authoritative_records = 0,
            input_bytes = 0,
            output_bytes = 0,
            event_bytes = 0,
            history_bytes = 0,
        },
    }
    compare_snapshots(state, tape.initial, rollback_session.current_snapshot(session), 0)
    trace_replay_boundary(state, 0)
    update_peaks(state)

    return {
        tape = tape,
        options = options,
        state = state,
        profile = profile,
        profile_name = profile_name,
        network_seed = network_seed,
        drain_ticks = drain_ticks,
        next_frame_index = 1,
        result = nil,
    }
end

---@param campaign RollbackLabCampaign
---@param frame InputFrame
local function advance_frame(campaign, frame)
    local state = campaign.state
    local tape = campaign.tape
    local options = campaign.options
    assert(
        frame.tick == state.reference.input_tick,
        "rollback lab tape and reference boundary disagree"
    )
    match.step(state.reference, 1 / tape.identity.tick_rate, frame, state.reference_combat)
    local reference_boundary = capture(options.measure, function()
        return match_snapshot.capture_owned(state.reference, state.reference_combat)
    end)
    ---@cast reference_boundary MatchSnapshot
    local reference_hash = match_snapshot.hash_canonical(reference_boundary)
    local expected_hash = assert(
        tape.boundary_hashes[frame.tick + 2],
        "rollback lab frozen recording is missing an authoritative boundary hash"
    )
    assert(
        reference_hash == expected_hash,
        ("rollback lab frozen boundary %d hash mismatch: expected %s, got %s"):format(
            frame.tick + 1,
            expected_hash,
            reference_hash
        )
    )
    assert(rollback_snapshot_history.store_owned(state.reference_history, reference_boundary))
    publish_reference(state, frame, reference_boundary)

    for slot, source in ipairs(state.sources) do
        local sample = client_sample(options.corruption, frame.tick, slot, frame.slots[slot])
        if source == "local" then
            add_client_authority(state, frame.tick, slot, sample)
        else
            assert(network_conditions.send(state.network, frame.tick, slot, frame.tick, sample))
        end
    end
    update_peaks(state)

    local deliveries = network_conditions.poll(state.network, frame.tick)
    process_delivery_batch(state, deliveries)
    publish_confirmation(state)
    catch_up_client(state, state.reference.input_tick)
    publish_confirmation(state)
    compare_newly_confirmed(state)
    update_peaks(state)
    detect_unconfirmed_floor(state)
end

---@param campaign RollbackLabCampaign
---@return RollbackLabResult
local function finish_campaign(campaign)
    local state = campaign.state
    local tape = campaign.tape
    local last_tick = #tape.frames - 1
    local drain = assert(
        network_conditions.drain(
            state.network,
            last_tick + 1,
            campaign.drain_ticks,
            drain_requests(state.sources, last_tick)
        )
    )
    update_peaks(state)
    process_drain_deliveries(state, drain.deliveries)
    catch_up_client(state, state.reference.input_tick)
    publish_confirmation(state)
    compare_newly_confirmed(state)
    update_peaks(state)
    detect_unconfirmed_floor(state)

    return finish_result(
        state,
        tape,
        campaign.profile_name,
        campaign.profile,
        campaign.network_seed,
        drain
    )
end

-- Advance at most max_ticks authoritative input rows. The final bounded drain
-- is performed only after the final row and returns the immutable result.
---@param campaign RollbackLabCampaign
---@param max_ticks integer
---@return RollbackLabResult?
function rollback_lab.step_campaign(campaign, max_ticks)
    assert(
        is_integer(max_ticks) and max_ticks > 0,
        "rollback lab campaign step must be a positive integer"
    )
    if campaign.result then
        return campaign.result
    end
    local last_index = math.min(#campaign.tape.frames, campaign.next_frame_index + max_ticks - 1)
    for index = campaign.next_frame_index, last_index do
        advance_frame(campaign, campaign.tape.frames[index])
    end
    campaign.next_frame_index = last_index + 1
    if campaign.next_frame_index > #campaign.tape.frames then
        campaign.result = finish_campaign(campaign)
    end
    return campaign.result
end

-- Exercise the terminal session seams and prove that an over-window failure
-- cannot advance state or logical counters after it has been reported.
---@param campaign RollbackLabCampaign
---@return boolean hidden_progress
function rollback_lab.probe_terminal_stability(campaign)
    local result = assert(campaign.result, "rollback lab terminal probe requires a result")
    assert(
        result.status == "late_input_unrecoverable",
        "rollback lab terminal probe requires an over-window result"
    )
    local session = campaign.state.session
    local before = rollback_session.diagnostics(session)
    local before_snapshot = rollback_session.current_snapshot(session)
    local output, _, code = rollback_session.step(session)
    assert(output == nil and code == "late_input_unrecoverable")
    local reconciled = rollback_session.reconcile(session)
    assert(not reconciled.changed and reconciled.status == "late_input_unrecoverable")
    local after = rollback_session.diagnostics(session)
    local after_snapshot = rollback_session.current_snapshot(session)
    return match_snapshot.first_difference_canonical(before_snapshot, after_snapshot) ~= nil
        or before.status ~= after.status
        or before.present_boundary ~= after.present_boundary
        or before.confirmed_tick ~= after.confirmed_tick
        or before.confirmed_output_tick ~= after.confirmed_output_tick
        or before.rollback_count ~= after.rollback_count
        or before.correction_count ~= after.correction_count
        or before.predicted_slot_samples ~= after.predicted_slot_samples
        or before.predicted_ticks ~= after.predicted_ticks
        or before.resimulated_ticks ~= after.resimulated_ticks
        or before.late_window_failures ~= after.late_window_failures
end

-- Run a validated, already-materialized authoritative tape against an
-- independent reference and one impaired rollback client. The optional
-- measurement callback is observational: clock values are neither read nor
-- retained here and cannot enter the logical result.
---@param tape InputTape
---@param options RollbackLabOptions?
---@return RollbackLabResult
function rollback_lab.run(tape, options)
    local campaign = rollback_lab.new_campaign(tape, options)
    local result = nil
    while result == nil do
        result = rollback_lab.step_campaign(campaign, #tape.frames)
    end
    return result
end

---@param depths RollbackLabDepth[]
---@return string
local function depth_marker(depths)
    local parts = {}
    for index, row in ipairs(depths) do
        parts[index] = row.depth .. ":" .. row.count
    end
    return #parts > 0 and table.concat(parts, ",") or "none"
end

-- Stable fixed-order logical evidence. Wall-time observations are deliberately
-- absent so repeated fresh processes can compare this line byte for byte.
---@param result RollbackLabResult
---@return string
function rollback_lab.logical_marker(result)
    local metrics = result.metrics
    local peaks = metrics.peaks
    local input = result.input_diagnostics
    local network = result.network_counters
    local network_diagnostics = result.network_diagnostics
    local events = result.event_metrics
    local history = result.history_accounting
    local drain = result.drain
    local difference = result.divergence and result.divergence.first_difference or nil
    return table.concat({
        "GC_ROLLBACK_LAB",
        "result",
        "schema=" .. result.schema,
        "fixture=" .. marker_string(result.fixture),
        "fixture_seed=" .. result.fixture_seed,
        "profile=" .. marker_string(result.profile),
        "profile_parameters=" .. marker_string(result.profile_parameters),
        "network_seed=" .. result.network_seed,
        "sources=" .. marker_string(result.source_pattern),
        "success=" .. bool_marker(result.success),
        "status=" .. marker_string(result.status),
        "ticks=" .. result.input_ticks,
        "tape_digest=" .. result.tape_digest,
        "reference_boundary=" .. result.reference_final_boundary,
        "client_boundary=" .. result.client_final_boundary,
        "initial_hash=" .. result.initial_hash,
        "reference_hash=" .. result.reference_final_hash,
        "client_hash=" .. result.client_final_hash,
        "confirmed_tick=" .. result.confirmed_tick,
        "confirmed_output_tick=" .. result.confirmed_output_tick,
        "late_tick=" .. optional_marker(result.late_input_tick),
        "unconfirmed_tick=" .. optional_marker(result.unconfirmed_tick),
        "divergence_tick="
            .. optional_marker(result.divergence and result.divergence.causal_tick or nil),
        "divergence_boundary="
            .. optional_marker(result.divergence and result.divergence.boundary or nil),
        "divergence_expected="
            .. optional_marker(result.divergence and result.divergence.expected_hash or nil),
        "divergence_actual="
            .. optional_marker(result.divergence and result.divergence.actual_hash or nil),
        "divergence_path=" .. optional_marker_string(difference and difference.path or nil),
        "compared=" .. metrics.compared_boundaries,
        "expected_compared=" .. metrics.expected_boundaries,
        "predicted_samples=" .. metrics.predicted_slot_samples,
        "predicted_ticks=" .. metrics.predicted_ticks,
        "corrections=" .. metrics.correction_count,
        "rollbacks=" .. metrics.rollback_count,
        "resimulated=" .. metrics.resimulated_ticks,
        "latest_depth=" .. metrics.latest_rollback_depth,
        "max_depth=" .. metrics.max_rollback_depth,
        "depths=" .. depth_marker(metrics.rollback_depths),
        "predicted_early_finish=" .. bool_marker(metrics.predicted_early_finish),
        "reactivations=" .. metrics.reactivation_count,
        "confirmed_redundant_skipped=" .. metrics.confirmed_redundant_rows_skipped,
        "snapshots=" .. metrics.current_snapshot_count,
        "snapshot_bytes=" .. metrics.current_snapshot_bytes,
        "peak_snapshots=" .. peaks.snapshot_count,
        "peak_snapshot_bytes=" .. peaks.snapshot_bytes,
        "input_floor=" .. input.oldest_retained_tick,
        "input_authority=" .. input.authoritative_sample_count,
        "input_effective=" .. input.effective_tick_count,
        "input_records=" .. input.record_tick_count,
        "input_anchors=" .. input.predecessor_anchor_count,
        "peak_input_authority=" .. peaks.input_authoritative_samples,
        "peak_input_effective=" .. peaks.input_effective_ticks,
        "peak_input_records=" .. peaks.input_record_ticks,
        "history_input_bytes=" .. history.input.total_bytes,
        "history_output_bytes=" .. history.output_bytes,
        "history_event_bytes=" .. history.event_bytes,
        "history_total_bytes=" .. history.total_bytes,
        "peak_history_input_bytes=" .. peaks.input_bytes,
        "peak_history_output_bytes=" .. peaks.output_bytes,
        "peak_history_event_bytes=" .. peaks.event_bytes,
        "peak_history_total_bytes=" .. peaks.history_bytes,
        "network_sent=" .. network.sent,
        "network_delivered=" .. network.delivered,
        "network_lost=" .. network.independent_lost,
        "network_burst_lost=" .. network.burst_lost,
        "network_duplicates=" .. network.duplicated,
        "network_reordered=" .. network.reordered,
        "network_history_recovered=" .. network.history_recovered,
        "network_retained=" .. network_diagnostics.retained_authoritative_records,
        "network_ledger=" .. network_diagnostics.delivered_ledger_entries,
        "network_pending=" .. network_diagnostics.pending_envelopes,
        "peak_network_pending=" .. peaks.network_pending_envelopes,
        "peak_network_references=" .. peaks.network_pending_record_references,
        "peak_network_ledger=" .. peaks.network_delivered_ledger_entries,
        "peak_network_retained=" .. peaks.network_authoritative_records,
        "event_added=" .. events.added,
        "event_revoked=" .. events.revoked,
        "event_replaced=" .. events.replaced,
        "event_confirmed_steps=" .. events.confirmed_steps,
        "event_confirmed_match=" .. events.confirmed_match_events,
        "event_confirmed_lifecycle=" .. events.confirmed_lifecycle_events,
        "event_residue=" .. events.speculative_residue,
        "event_reference_digest=" .. events.reference_digest,
        "event_confirmed_digest=" .. events.confirmed_digest,
        "event_matched=" .. bool_marker(events.matched),
        "drain_complete=" .. bool_marker(drain.complete),
        "drain_pending=" .. drain.pending,
        "drain_recovered=" .. drain.recovered,
        "drain_requested=" .. drain.requested,
        "drain_final_tick=" .. drain.final_tick,
    }, "|")
end

---@param result RollbackLabResult
---@return string
function rollback_lab.summary(result)
    local metrics = result.metrics
    local network = result.network_counters
    local lines = {
        ("Rollback laboratory: %s (%s)"):format(result.status, result.success and "pass" or "fail"),
        ("Fixture %s, profile %s, network seed %d, sources %s"):format(
            result.fixture,
            result.profile,
            result.network_seed,
            result.source_pattern
        ),
        ("Boundary %d/%d, confirmed input %d, confirmed output %d, compared %d/%d"):format(
            result.client_final_boundary,
            result.reference_final_boundary,
            result.confirmed_tick,
            result.confirmed_output_tick,
            metrics.compared_boundaries,
            metrics.expected_boundaries
        ),
        ("Hash reference=%s client=%s"):format(
            result.reference_final_hash,
            result.client_final_hash
        ),
        ("Prediction samples=%d ticks=%d; corrections=%d rollbacks=%d resimulated=%d max_depth=%d"):format(
            metrics.predicted_slot_samples,
            metrics.predicted_ticks,
            metrics.correction_count,
            metrics.rollback_count,
            metrics.resimulated_ticks,
            metrics.max_rollback_depth
        ),
        ("Network sent=%d delivered=%d loss=%d burst=%d duplicate=%d reordered=%d recovered=%d"):format(
            network.sent,
            network.delivered,
            network.independent_lost,
            network.burst_lost,
            network.duplicated,
            network.reordered,
            network.history_recovered
        ),
        ("Snapshots current=%d/%dB peak=%d/%dB; drain complete=%s pending=%d"):format(
            metrics.current_snapshot_count,
            metrics.current_snapshot_bytes,
            metrics.peaks.snapshot_count,
            metrics.peaks.snapshot_bytes,
            tostring(result.drain.complete),
            result.drain.pending
        ),
    }
    if result.divergence then
        lines[#lines + 1] = ("Divergence causal_tick=%d boundary=%d expected=%s actual=%s path=%s"):format(
            result.divergence.causal_tick,
            result.divergence.boundary,
            result.divergence.expected_hash,
            result.divergence.actual_hash,
            result.divergence.first_difference and result.divergence.first_difference.path or "none"
        )
    end
    if result.late_input_tick ~= nil then
        lines[#lines + 1] = "Late input tick: " .. result.late_input_tick
    end
    if result.unconfirmed_tick ~= nil then
        lines[#lines + 1] = "Unconfirmed authority begins at tick: " .. result.unconfirmed_tick
    end
    return table.concat(lines, "\n")
end

return rollback_lab
