-- Pure incremental rollback laboratory for the playable match screen. One
-- render-owned fixed clock supplies transport ticks; this controller owns the
-- independent reference, predicted client, network impairment, and stable
-- presentation-event timeline.

local network_profiles = require("data.network_profiles")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local network_conditions = require("sim.network_conditions")
local rollback_events = require("sim.rollback_events")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_session = require("sim.rollback_session")
local rollback_snapshot_history = require("sim.rollback_snapshot_history")
local slot_input = require("sim.slot_input")

---@alias RollbackPlayableLabStatus
---| "active"
---| "settling"
---| "converged"
---| "diverged"
---| "late_input_unrecoverable"
---| "unconfirmed_window_exceeded"
---| "comparison_history_missing"
---| "drain_incomplete"

---@alias RollbackPlayableConvergenceStatus "pending"|"matched"|"diverged"

---@class RollbackPlayableLabOptions
---@field local_slot integer?
---@field profile_name string?
---@field profile NetworkProfile?
---@field network_seed integer?
---@field bot_seed integer?
---@field max_rollback_ticks integer?
---@field settlement_ticks integer?

---@class RollbackPlayableCorrection
---@field causal_tick integer
---@field restored_boundary integer
---@field old_present_boundary integer
---@field new_present_boundary integer
---@field replaced_from_tick integer
---@field replaced_through_tick integer
---@field corrected_from_tick integer
---@field corrected_through_tick integer
---@field old_present_hash string?
---@field new_present_hash string?
---@field first_difference MatchSnapshotDifference?

---@class RollbackPlayableConvergence
---@field status RollbackPlayableConvergenceStatus
---@field boundary integer
---@field expected_hash string
---@field actual_hash string
---@field first_difference MatchSnapshotDifference?

---@class RollbackPlayableLabBatch
---@field outputs RollbackTickOutput[]
---@field event_diffs RollbackEventDiff[]
---@field confirmed_steps RollbackEventStep[]
---@field corrections RollbackPlayableCorrection[]
---@field status RollbackPlayableLabStatus

---@class RollbackPlayableLabDebugModel
---@field profile string
---@field local_slot integer
---@field status RollbackPlayableLabStatus
---@field reference_tick integer
---@field current_tick integer
---@field transport_tick integer
---@field confirmed_input_tick integer
---@field confirmed_output_tick integer
---@field predicted_slot_samples integer
---@field predicted_ticks integer
---@field correction_count integer
---@field rollback_count integer
---@field latest_rollback_depth integer
---@field max_rollback_depth integer
---@field resimulated_ticks integer
---@field retained_snapshot_count integer
---@field retained_snapshot_bytes integer
---@field network_pending integer
---@field network_counters NetworkConditionCounters
---@field network_high_water NetworkConditionDiagnostics
---@field convergence RollbackPlayableConvergence
---@field event_status RollbackEventsStatus
---@field predicted_early_finish boolean
---@field late_input_tick integer?
---@field settlement_ticks integer
---@field settlement_limit integer

---@class RollbackPlayableLab
---@field _reference MatchState
---@field _reference_combat CombatMatchState?
---@field _reference_history RollbackSnapshotHistory
---@field _session RollbackSession
---@field _events RollbackEventTimeline
---@field _network NetworkConditions
---@field _producer SlotInputProducerState
---@field _sources RollbackInputSource[]
---@field _local_slot integer
---@field _profile_name string
---@field _transport_tick integer
---@field _status RollbackPlayableLabStatus
---@field _last_compared_output integer
---@field _latest_convergence RollbackPlayableConvergence
---@field _predicted_early_finish boolean
---@field _late_input_tick integer?
---@field _settlement_ticks integer
---@field _settlement_limit integer
---@field _last_reference_input_tick integer?

---@class RollbackPlayableLabModule
local rollback_playable_lab = {}

rollback_playable_lab.DEFAULT_NETWORK_SEED = 7302
rollback_playable_lab.DEFAULT_BOT_SEED = 7400
rollback_playable_lab.DEFAULT_SETTLEMENT_TICKS = 256

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param value any
---@return any
local function copy_value(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, child in pairs(value) do
        result[copy_value(key)] = copy_value(child)
    end
    return result
end

---@param sample InputSample
---@return InputSample
local function copy_sample(sample)
    return assert(input_frame.new_sample(sample))
end

---@return RollbackPlayableLabBatch
local function new_batch()
    return {
        outputs = {},
        event_diffs = {},
        confirmed_steps = {},
        corrections = {},
        status = "active",
    }
end

---@param batch RollbackPlayableLabBatch
---@param values RollbackEventStep[]
local function append_confirmed(batch, values)
    for _, value in ipairs(values) do
        batch.confirmed_steps[#batch.confirmed_steps + 1] = copy_value(value)
    end
end

---@param initial_snapshot MatchSnapshot
---@param local_slot integer
---@return MatchSnapshot
local function controlled_initial_snapshot(initial_snapshot, local_slot)
    local state, combat_state = match_snapshot.restore(initial_snapshot)
    assert(state.slot_mode, "playable rollback lab requires a slot-mode match")
    assert(state.input_tick == 0, "playable rollback lab requires boundary zero")
    assert(not state.finished, "playable rollback lab requires an active initial match")
    state.controlled = assert(state.slot_players[local_slot], "local rollback slot is unmapped")
    return match_snapshot.capture(state, combat_state)
end

---@param local_slot integer
---@param bot_seed integer
---@return SlotInputProducerState
local function new_producer(local_slot, bot_seed)
    local sources = {}
    for slot = 1, input_frame.SLOT_COUNT do
        if slot == local_slot then
            sources[slot] = { kind = "frame" }
        else
            sources[slot] = { kind = "bot", seed = bot_seed + slot * 97 }
        end
    end
    return slot_input.new_producer(sources)
end

---@param local_slot integer
---@return RollbackInputSource[]
local function new_sources(local_slot)
    local sources = {}
    for slot = 1, input_frame.SLOT_COUNT do
        sources[slot] = slot == local_slot and "local" or "remote"
    end
    return sources
end

---@param options RollbackPlayableLabOptions
---@return NetworkProfile
---@return string
local function selected_profile(options)
    if options.profile then
        return options.profile, options.profile_name or "custom"
    end
    local name = options.profile_name or "playable"
    return assert(network_profiles[name], "unknown playable rollback profile: " .. name), name
end

---@param lab RollbackPlayableLab
---@param status RollbackPlayableLabStatus
local function terminate(lab, status)
    if lab._status == "active" or lab._status == "settling" then
        lab._status = status
    end
end

---@param lab RollbackPlayableLab
---@param tick integer
---@param slot integer
---@param sample InputSample
local function add_authority(lab, tick, slot, sample)
    local arrival, err, code = rollback_session.add_authoritative(lab._session, tick, slot, sample)
    if arrival then
        return
    end
    if code == "outside_window" then
        if lab._late_input_tick == nil or tick < lab._late_input_tick then
            lab._late_input_tick = tick
        end
        terminate(lab, "late_input_unrecoverable")
        return
    end
    assert(false, err or ("playable rollback lab rejected authority with " .. tostring(code)))
end

---@param lab RollbackPlayableLab
---@param output RollbackTickOutput
---@return RollbackEventStepInput
local function event_step(lab, output)
    local lookup = rollback_session.snapshot(lab._session, output.end_boundary)
    assert(
        lookup.status == "present" or lookup.status == "retained",
        "rollback event output boundary is not retained"
    )
    return {
        output = output,
        snapshot = assert(lookup.snapshot, "rollback event output snapshot is missing"),
    }
end

---@param lab RollbackPlayableLab
---@param batch RollbackPlayableLabBatch
---@param from_tick integer
---@param through_tick integer
---@param outputs RollbackTickOutput[]
---@return boolean
local function apply_outputs(lab, batch, from_tick, through_tick, outputs)
    local steps = {}
    for index, output in ipairs(outputs) do
        steps[index] = event_step(lab, output)
    end
    local diff, err, code = rollback_events.apply(lab._events, from_tick, through_tick, steps)
    if diff == nil then
        assert(
            code == "unconfirmed_window_exceeded",
            err or "playable rollback event application failed"
        )
        terminate(lab, "unconfirmed_window_exceeded")
        return false
    end
    batch.event_diffs[#batch.event_diffs + 1] = copy_value(diff)
    return true
end

---@param lab RollbackPlayableLab
---@param batch RollbackPlayableLabBatch
---@return boolean
local function publish_confirmation(lab, batch)
    if rollback_events.diagnostics(lab._events).status ~= "active" then
        terminate(lab, "unconfirmed_window_exceeded")
        return false
    end
    local confirmed_output = rollback_session.diagnostics(lab._session).confirmed_output_tick
    append_confirmed(batch, rollback_events.confirm(lab._events, confirmed_output))
    return true
end

---@param lab RollbackPlayableLab
---@param expected MatchSnapshot
---@param actual MatchSnapshot
---@param boundary integer
local function compare_boundary(lab, expected, actual, boundary)
    local expected_hash = match_snapshot.hash(expected)
    local actual_hash = match_snapshot.hash(actual)
    local matched = expected_hash == actual_hash
    lab._latest_convergence = {
        status = matched and "matched" or "diverged",
        boundary = boundary,
        expected_hash = expected_hash,
        actual_hash = actual_hash,
        first_difference = matched and nil or match_snapshot.first_difference(expected, actual),
    }
    if not matched then
        terminate(lab, "diverged")
    end
end

---@param lab RollbackPlayableLab
local function compare_newly_confirmed(lab)
    local confirmed = rollback_session.diagnostics(lab._session).confirmed_output_tick
    while lab._last_compared_output < confirmed do
        local output_tick = lab._last_compared_output + 1
        local boundary = output_tick + 1
        local expected = rollback_snapshot_history.lookup(lab._reference_history, boundary)
        local actual = rollback_session.snapshot(lab._session, boundary)
        if
            (expected.status ~= "present" and expected.status ~= "retained")
            or (actual.status ~= "present" and actual.status ~= "retained")
        then
            terminate(lab, "comparison_history_missing")
            return
        end
        compare_boundary(
            lab,
            assert(expected.snapshot, "reference comparison snapshot is missing"),
            assert(actual.snapshot, "client comparison snapshot is missing"),
            boundary
        )
        lab._last_compared_output = output_tick
        if lab._status == "diverged" then
            return
        end
    end
end

---@param result RollbackReconcileResult
---@return RollbackPlayableCorrection
local function correction_view(result)
    return {
        causal_tick = assert(result.causal_tick),
        restored_boundary = assert(result.restored_boundary),
        old_present_boundary = result.old_present_boundary,
        new_present_boundary = result.new_present_boundary,
        replaced_from_tick = assert(result.replaced_from_tick),
        replaced_through_tick = assert(result.replaced_through_tick),
        corrected_from_tick = assert(result.corrected_from_tick),
        corrected_through_tick = assert(result.corrected_through_tick),
        old_present_hash = result.old_present_hash,
        new_present_hash = result.new_present_hash,
        first_difference = copy_value(result.first_difference),
    }
end

---@param lab RollbackPlayableLab
---@param batch RollbackPlayableLabBatch
---@param deliveries NetworkDelivery[]
---@return boolean
local function process_deliveries(lab, batch, deliveries)
    for _, delivery in ipairs(deliveries) do
        for _, record in ipairs(network_conditions.records(delivery)) do
            local confirmed = rollback_session.diagnostics(lab._session).confirmed_tick
            if record.tick > confirmed then
                add_authority(lab, record.tick, delivery.source_slot, record.sample)
            end
        end
    end
    local reconciled = rollback_session.reconcile(lab._session)
    if reconciled.status == "late_input_unrecoverable" then
        if lab._late_input_tick == nil then
            lab._late_input_tick = reconciled.causal_tick
        end
        terminate(lab, "late_input_unrecoverable")
        return false
    end
    if not reconciled.changed then
        return lab._status == "active" or lab._status == "settling"
    end
    assert(#reconciled.corrected_outputs > 0, "changed rollback must return corrected outputs")
    if
        not apply_outputs(
            lab,
            batch,
            assert(reconciled.replaced_from_tick),
            assert(reconciled.replaced_through_tick),
            reconciled.corrected_outputs
        )
    then
        return false
    end
    batch.corrections[#batch.corrections + 1] = correction_view(reconciled)
    if reconciled.status == "active" and lab._predicted_early_finish then
        lab._predicted_early_finish = false
    end
    return true
end

---@param lab RollbackPlayableLab
---@param batch RollbackPlayableLabBatch
---@param target_boundary integer
---@return boolean
local function catch_up_client(lab, batch, target_boundary)
    while true do
        local diagnostics = rollback_session.diagnostics(lab._session)
        if diagnostics.present_boundary >= target_boundary then
            return true
        end
        if diagnostics.status == "late_input_unrecoverable" then
            terminate(lab, "late_input_unrecoverable")
            return false
        end
        if diagnostics.status == "finished" then
            lab._predicted_early_finish = true
            return true
        end
        local output, err, code = rollback_session.step(lab._session)
        if output == nil then
            if code == "match_finished" then
                lab._predicted_early_finish = true
                return true
            end
            assert(code == "late_input_unrecoverable", err or "rollback client step failed")
            terminate(lab, "late_input_unrecoverable")
            return false
        end
        batch.outputs[#batch.outputs + 1] = copy_value(output)
        if not apply_outputs(lab, batch, output.tick, output.tick, { output }) then
            return false
        end
    end
end

---@param lab RollbackPlayableLab
---@param batch RollbackPlayableLabBatch
---@return boolean
local function reconcile_and_publish(lab, batch)
    local deliveries = network_conditions.poll(lab._network, lab._transport_tick)
    if not process_deliveries(lab, batch, deliveries) then
        return false
    end
    -- Confirmation immediately after a delivery/correction frees the oldest
    -- event slot before the next predicted output is appended. At a 30-tick
    -- delay this is the difference between a supported correction and a false
    -- unconfirmed-window terminal.
    if not publish_confirmation(lab, batch) then
        return false
    end
    if not catch_up_client(lab, batch, lab._reference.input_tick) then
        return false
    end
    if not publish_confirmation(lab, batch) then
        return false
    end
    compare_newly_confirmed(lab)
    return lab._status == "active" or lab._status == "settling"
end

---@param lab RollbackPlayableLab
---@param local_sample InputSample
local function advance_reference(lab, local_sample)
    assert(
        lab._reference.input_tick == lab._transport_tick,
        "reference input tick and transport tick must stay aligned"
    )
    local slots = {}
    for slot = 1, input_frame.SLOT_COUNT do
        slots[slot] = input_frame.neutral_sample()
    end
    slots[lab._local_slot] = copy_sample(local_sample)
    local base = assert(input_frame.new(lab._transport_tick, slots))
    local frame = slot_input.materialize(lab._producer, lab._reference, base)
    match.step(lab._reference, fixed_clock.TICK_SECONDS, frame, lab._reference_combat)
    local boundary = match_snapshot.capture_owned(lab._reference, lab._reference_combat)
    assert(rollback_snapshot_history.store_owned(lab._reference_history, boundary))

    add_authority(lab, frame.tick, lab._local_slot, frame.slots[lab._local_slot])
    for slot = 1, input_frame.SLOT_COUNT do
        if slot ~= lab._local_slot then
            assert(
                network_conditions.send(
                    lab._network,
                    lab._transport_tick,
                    slot,
                    frame.tick,
                    frame.slots[slot]
                )
            )
        end
    end
    lab._last_reference_input_tick = frame.tick
end

---@param lab RollbackPlayableLab
local function resend_final_remote_rows(lab)
    local last_tick =
        assert(lab._last_reference_input_tick, "rollback settlement has no final reference input")
    if rollback_session.diagnostics(lab._session).confirmed_tick >= last_tick then
        return
    end
    for slot = 1, input_frame.SLOT_COUNT do
        if slot ~= lab._local_slot then
            assert(network_conditions.resend(lab._network, lab._transport_tick, slot, last_tick))
        end
    end
end

---@param lab RollbackPlayableLab
local function finish_settlement_if_ready(lab)
    local final_tick = assert(lab._last_reference_input_tick)
    local session = rollback_session.diagnostics(lab._session)
    if
        session.confirmed_tick < final_tick
        or session.confirmed_output_tick < final_tick
        or network_conditions.pending(lab._network) > 0
    then
        return
    end
    local expected = match_snapshot.capture_owned(lab._reference, lab._reference_combat)
    local comparison = rollback_session.compare(lab._session, expected, final_tick)
    lab._latest_convergence = {
        status = comparison.matched and "matched" or "diverged",
        boundary = comparison.expected_boundary,
        expected_hash = comparison.expected_hash,
        actual_hash = comparison.actual_hash,
        first_difference = copy_value(comparison.first_difference),
    }
    lab._status = comparison.matched and "converged" or "diverged"
end

---@param initial_snapshot MatchSnapshot Canonical slot-mode boundary zero.
---@param options RollbackPlayableLabOptions?
---@return RollbackPlayableLab
function rollback_playable_lab.new(initial_snapshot, options)
    options = options or {}
    local local_slot = options.local_slot or 1
    assert(
        is_integer(local_slot) and local_slot >= 1 and local_slot <= input_frame.SLOT_COUNT,
        "playable rollback local slot must be between one and eight"
    )
    local network_seed = options.network_seed or rollback_playable_lab.DEFAULT_NETWORK_SEED
    local bot_seed = options.bot_seed or rollback_playable_lab.DEFAULT_BOT_SEED
    assert(is_integer(network_seed), "playable rollback network seed must be an integer")
    assert(is_integer(bot_seed), "playable rollback bot seed must be an integer")
    local maximum = options.max_rollback_ticks or rollback_input_history.ROLLBACK_WINDOW_TICKS
    assert(
        is_integer(maximum)
            and maximum >= 1
            and maximum <= rollback_input_history.ROLLBACK_WINDOW_TICKS,
        "playable rollback window must be a positive bounded integer"
    )
    local settlement = options.settlement_ticks or rollback_playable_lab.DEFAULT_SETTLEMENT_TICKS
    assert(
        is_integer(settlement) and settlement >= 1,
        "playable rollback settlement must be a positive integer"
    )
    local profile, profile_name = selected_profile(options)
    local canonical = controlled_initial_snapshot(initial_snapshot, local_slot)
    local reference, reference_combat = match_snapshot.restore(canonical)
    local reference_history = rollback_snapshot_history.new(maximum)
    assert(rollback_snapshot_history.store_owned(reference_history, canonical))
    local sources = new_sources(local_slot)
    local session = rollback_session.new(canonical, sources, maximum)
    local initial_hash = match_snapshot.hash(canonical)
    return {
        _reference = reference,
        _reference_combat = reference_combat,
        _reference_history = reference_history,
        _session = session,
        _events = rollback_events.new(canonical, maximum),
        _network = network_conditions.new(profile, network_seed),
        _producer = new_producer(local_slot, bot_seed),
        _sources = sources,
        _local_slot = local_slot,
        _profile_name = profile_name,
        _transport_tick = 0,
        _status = "active",
        _last_compared_output = -1,
        _latest_convergence = {
            status = "matched",
            boundary = 0,
            expected_hash = initial_hash,
            actual_hash = initial_hash,
            first_difference = nil,
        },
        _predicted_early_finish = false,
        _late_input_tick = nil,
        _settlement_ticks = 0,
        _settlement_limit = settlement,
        _last_reference_input_tick = nil,
    }
end

---@param lab RollbackPlayableLab
---@return boolean
function rollback_playable_lab.needs_local_sample(lab)
    return lab._status == "active" and not lab._reference.finished
end

-- Advance exactly one transport tick. During active play this consumes one
-- local sample and one complete reference InputFrame. After reference full
-- time it advances only network settlement and never invents another frame.
---@param lab RollbackPlayableLab
---@param transport_tick integer
---@param local_sample InputSample?
---@return RollbackPlayableLabBatch
function rollback_playable_lab.advance(lab, transport_tick, local_sample)
    assert(
        is_integer(transport_tick) and transport_tick == lab._transport_tick,
        "playable rollback transport tick is not contiguous"
    )
    local batch = new_batch()
    if lab._status ~= "active" and lab._status ~= "settling" then
        batch.status = lab._status
        return batch
    end

    if lab._status == "active" then
        assert(local_sample ~= nil, "active playable rollback tick requires local input")
        assert(input_frame.validate_sample(local_sample))
        advance_reference(lab, local_sample)
        if lab._status == "active" then
            reconcile_and_publish(lab, batch)
        end
        if lab._status == "active" and lab._reference.finished then
            lab._status = "settling"
            finish_settlement_if_ready(lab)
        end
    else
        assert(local_sample == nil, "rollback settlement must not consume match input")
        lab._settlement_ticks = lab._settlement_ticks + 1
        resend_final_remote_rows(lab)
        reconcile_and_publish(lab, batch)
        if lab._status == "settling" then
            finish_settlement_if_ready(lab)
        end
        if lab._status == "settling" and lab._settlement_ticks >= lab._settlement_limit then
            lab._status = "drain_incomplete"
        end
    end

    lab._transport_tick = lab._transport_tick + 1
    batch.status = lab._status
    return copy_value(batch)
end

---@param lab RollbackPlayableLab
---@return MatchSnapshot
function rollback_playable_lab.current_snapshot(lab)
    return rollback_session.current_snapshot(lab._session)
end

---@param lab RollbackPlayableLab
---@return MatchSnapshot
function rollback_playable_lab.reference_snapshot(lab)
    return match_snapshot.capture_owned(lab._reference, lab._reference_combat)
end

---@param lab RollbackPlayableLab
---@param boundary_tick integer
---@return RollbackSnapshotLookup
function rollback_playable_lab.snapshot(lab, boundary_tick)
    return rollback_session.snapshot(lab._session, boundary_tick)
end

---@param lab RollbackPlayableLab
---@return RollbackPlayableLabDebugModel
function rollback_playable_lab.debug_model(lab)
    local session = rollback_session.diagnostics(lab._session)
    local network = network_conditions.diagnostics(lab._network)
    local event_diagnostics = rollback_events.diagnostics(lab._events)
    return copy_value({
        profile = lab._profile_name,
        local_slot = lab._local_slot,
        status = lab._status,
        reference_tick = lab._reference.input_tick,
        current_tick = session.present_boundary,
        transport_tick = lab._transport_tick,
        confirmed_input_tick = session.confirmed_tick,
        confirmed_output_tick = session.confirmed_output_tick,
        predicted_slot_samples = session.predicted_slot_samples,
        predicted_ticks = session.predicted_ticks,
        correction_count = session.correction_count,
        rollback_count = session.rollback_count,
        latest_rollback_depth = session.latest_rollback_depth,
        max_rollback_depth = session.max_rollback_depth,
        resimulated_ticks = session.resimulated_ticks,
        retained_snapshot_count = session.snapshot_history.retained_boundary_count,
        retained_snapshot_bytes = session.snapshot_history.canonical_bytes,
        network_pending = network.pending_envelopes,
        network_counters = network_conditions.counters(lab._network),
        network_high_water = network,
        convergence = copy_value(lab._latest_convergence),
        event_status = event_diagnostics.status,
        predicted_early_finish = lab._predicted_early_finish,
        late_input_tick = lab._late_input_tick,
        settlement_ticks = lab._settlement_ticks,
        settlement_limit = lab._settlement_limit,
    })
end

return rollback_playable_lab
