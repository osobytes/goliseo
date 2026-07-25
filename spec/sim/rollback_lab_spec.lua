local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_identity = require("sim.combat_identity")
local determinism_evidence = require("sim.determinism_evidence")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_lab = require("sim.rollback_lab")
local rollback_session = require("sim.rollback_session")
local teams = require("data.teams")
local tuning = require("sim.tuning")
local t = require("spec.support.runner")

---@param options table?
---@return NetworkProfile
local function profile(options)
    options = options or {}
    return {
        base_delay_ticks = options.base_delay_ticks or 0,
        jitter_min_ticks = options.jitter_min_ticks or 0,
        jitter_max_ticks = options.jitter_max_ticks or 0,
        independent_loss_rate = options.independent_loss_rate or 0,
        duplication_rate = options.duplication_rate or 0,
        burst_start_rate = options.burst_start_rate or 0,
        burst_length_ticks = options.burst_length_ticks or 0,
    }
end

---@param name string
---@param initial MatchSnapshot
---@return InputTapeIdentity
local function identity(name, initial)
    return {
        tape_version = input_tape.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = match_snapshot.VERSION,
        build = "rollback-lab-spec",
        source = "materialized-spec-fixture",
        content = "nebula-orion-spec-content",
        tuning = tuning.serialize(),
        config = "field=960x540;duration=20;max_goals=99;tick_rate=60",
        fixture = name,
        seed = 733,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = assert(initial.state.input_ownership),
    }
end

---@param duration number?
---@param max_goals integer?
---@return MatchState
local function new_state(duration, max_goals)
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = duration or 20,
        max_goals = max_goals or 99,
        seed = 733,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
end

---@param count integer
---@param name string?
---@return InputTape
local function varying_tape(count, name)
    local state = new_state()
    local initial = match_snapshot.capture(state)
    local frames = {}
    for tick = 0, count - 1 do
        local slots = {}
        for slot = 1, input_frame.SLOT_COUNT do
            slots[slot] = assert(input_frame.new_sample({
                move_x = ((tick * 29 + slot * 17) % 255) - 127,
                move_y = ((tick * 13 + slot * 31) % 255) - 127,
                held = tick % 3 == 0 and input_frame.HELD_BITS.sprint or 0,
                edges = tick % 7 == 0 and input_frame.EDGE_BITS.dash or 0,
            }))
        end
        frames[#frames + 1] = assert(input_frame.new(tick, slots))
    end
    return input_tape.new(identity(name or ("varying-" .. count), initial), initial, frames)
end

---@return InputTape
local function combat_tape()
    local state = new_state()
    state.kickoff_hold = 0
    local combat_state = combat.new_state(state)
    local initial = match_snapshot.capture(state, combat_state)
    local frames = {}
    for tick = 0, 79 do
        local frame = assert(input_frame.neutral(tick))
        for slot = 1, input_frame.SLOT_COUNT do
            if tick == 0 then
                frame.slots[slot] = assert(input_frame.new_sample({
                    held = input_frame.HELD_BITS.equipment,
                    edges = input_frame.EDGE_BITS.equipment_pressed,
                }))
            elseif tick < 20 then
                frame.slots[slot] = assert(input_frame.new_sample({
                    held = input_frame.HELD_BITS.equipment,
                }))
            elseif tick == 20 then
                frame.slots[slot] = assert(input_frame.new_sample({
                    edges = input_frame.EDGE_BITS.equipment_released,
                }))
            end
        end
        frames[#frames + 1] = frame
    end
    local tape_identity = {
        tape_version = input_tape.COMBAT_VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = match_snapshot.COMBAT_VERSION,
        build = "rollback-lab-combat-spec",
        source = "materialized-combat-spec-fixture",
        content = "nebula-orion-spec-content",
        tuning = tuning.serialize(),
        config = "field=960x540;duration=20;max_goals=99;tick_rate=60",
        fixture = "combat-rollback-80",
        seed = 733,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = assert(initial.state.input_ownership),
        combat = combat_identity.for_state(combat_state),
    }
    return input_tape.new(tape_identity, initial, frames)
end

---@param remote_slot integer?
---@return RollbackInputSource[]
local function one_remote(remote_slot)
    remote_slot = remote_slot or 1
    local sources = {}
    for slot = 1, input_frame.SLOT_COUNT do
        sources[slot] = slot == remote_slot and "remote" or "local"
    end
    return sources
end

---@return MatchSnapshot
local function preventable_goal_initial()
    local state = new_state(4, 1)
    local carrier_index = state.slot_players[1]
    local carrier = state.players[carrier_index]
    carrier.pos = Vec2.new(900, 270)
    carrier.vel = Vec2.new(0, 0)
    carrier.run_vel = Vec2.new(0, 0)
    carrier.facing = Vec2.new(1, 0)
    carrier.windup_timer = fixed_clock.TICK_SECONDS
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

---@return InputTape
local function early_finish_tape()
    local initial = preventable_goal_initial()
    local state = match_snapshot.restore(initial)
    local frames = {}
    while not state.finished do
        local tick = state.input_tick
        local slots = {}
        for slot = 1, input_frame.SLOT_COUNT do
            slots[slot] = input_frame.neutral_sample()
        end
        if tick == 0 then
            slots[5] = assert(input_frame.new_sample({ edges = input_frame.EDGE_BITS.dash }))
        end
        local frame = assert(input_frame.new(tick, slots))
        frames[#frames + 1] = frame
        match.step(state, fixed_clock.TICK_SECONDS, frame)
        assert(#frames < 400, "early-finish fixture did not reach full time")
    end
    return input_tape.new(identity("prevented-early-finish", initial), initial, frames)
end

t.describe("OMP-2 authoritative-reference rollback laboratory", function()
    t.it("pins the live soccer tape digest without a synthetic combat segment", function()
        t.eq(rollback_lab.tape_digest(determinism_evidence.fixture_tape()), "d89f7fc53d660ab7")
    end)

    t.it("converges combat state and confirmed events through delayed authority", function()
        local tape = combat_tape()
        local result = rollback_lab.run(tape, {
            profile_name = "combat-delay-spec",
            profile = profile({ base_delay_ticks = 3 }),
            network_seed = 2001,
            sources = one_remote(),
        })
        t.is_true(result.success)
        t.eq(result.status, "converged")
        t.is_true(result.metrics.rollback_count > 0)
        t.is_true(result.event_metrics.confirmed_combat_events > 0)
        t.eq(result.event_metrics.reference_digest, result.event_metrics.confirmed_digest)
        t.eq(assert(result.reference_final_snapshot.combat).tick, 80)
        t.eq(assert(result.client_final_snapshot.combat).tick, 80)
        t.is_true(result.metrics.peaks.snapshot_bytes < 768 * 1024)
        t.is_true(result.metrics.peaks.history_bytes < 1024 * 1024)
    end)

    t.it("matches every clean boundary without prediction or rollback", function()
        local tape = varying_tape(40)
        local result = rollback_lab.run(tape, {
            profile_name = "clean",
            network_seed = 1,
            sources = one_remote(),
        })

        t.is_true(result.success)
        t.eq(result.status, "converged")
        t.eq(result.metrics.predicted_slot_samples, 0)
        t.eq(result.metrics.predicted_ticks, 0)
        t.eq(result.metrics.rollback_count, 0)
        t.eq(result.metrics.correction_count, 0)
        t.eq(result.metrics.compared_boundaries, #tape.frames + 1)
        t.eq(result.reference_final_hash, result.client_final_hash)
        t.eq(result.confirmed_tick, #tape.frames - 1)
    end)

    t.it("rolls back and converges with OMP-0 delay and jitter reordering", function()
        local tape = varying_tape(48)
        local delayed = rollback_lab.run(tape, {
            profile_name = "omp0_parity",
            network_seed = 7302,
            sources = one_remote(),
        })
        t.is_true(delayed.success)
        t.is_true(delayed.metrics.rollback_count > 0)
        t.is_true(delayed.metrics.resimulated_ticks > 0)
        t.is_true(delayed.metrics.max_rollback_depth >= 3)

        local reordered = rollback_lab.run(tape, {
            profile_name = "jitter-spec",
            profile = profile({
                base_delay_ticks = 2,
                jitter_min_ticks = -2,
                jitter_max_ticks = 2,
            }),
            network_seed = 102223,
            sources = one_remote(),
        })
        t.is_true(reordered.success)
        t.is_true(reordered.network_counters.reordered > 0)
        t.is_true(reordered.metrics.rollback_count > 0)
    end)

    t.it("recovers independent loss from packet history and the final-row drain", function()
        local history = rollback_lab.run(varying_tape(5, "history-recovery"), {
            profile_name = "loss-history-spec",
            profile = profile({ independent_loss_rate = 0.5 }),
            network_seed = 85,
            sources = one_remote(),
        })
        t.is_true(history.success)
        t.is_true(history.network_counters.independent_lost > 0)
        t.is_true(history.network_counters.history_recovered > 0)

        local final = rollback_lab.run(varying_tape(6, "final-row-recovery"), {
            profile_name = "final-loss-spec",
            profile = profile({ base_delay_ticks = 4, independent_loss_rate = 0.5 }),
            network_seed = 85,
            sources = one_remote(),
        })
        t.is_true(final.success)
        t.is_true(final.drain.complete)
        t.eq(final.drain.recovered, 1)
        t.is_true(final.network_counters.sent > 6, "the lost final row must be resent")
        t.is_true(
            final.metrics.peaks.network_pending_envelopes
                > final.network_diagnostics.pending_envelopes
        )
        t.is_true(
            final.metrics.peaks.network_pending_record_references
                > final.network_diagnostics.pending_record_references
        )
    end)

    t.it("keeps impairment-created duplicates idempotent", function()
        local tape = varying_tape(12)
        local result = rollback_lab.run(tape, {
            profile_name = "duplicate-spec",
            profile = profile({ duplication_rate = 1 }),
            network_seed = 4,
            sources = one_remote(),
        })

        t.is_true(result.success)
        t.eq(result.network_counters.duplicated, result.network_counters.sent)
        t.eq(result.network_counters.delivered, result.network_counters.sent * 2)
        t.eq(result.metrics.correction_count, 0)
        t.eq(result.metrics.rollback_count, 0)
    end)

    t.it("reconciles multiple correction batches before final convergence", function()
        local result = rollback_lab.run(varying_tape(18), {
            profile_name = "batch-spec",
            profile = profile({ base_delay_ticks = 2 }),
            network_seed = 2,
            sources = one_remote(),
        })

        t.is_true(result.success)
        t.is_true(result.metrics.rollback_count > 2)
        t.is_true(#result.metrics.rollback_depths > 0)
        t.eq(
            result.metrics.rollback_count,
            result.metrics.correction_count,
            "one changing remote row arrives per correction batch"
        )
    end)

    t.it("reactivates a predicted early finish and later reaches reference full time", function()
        local tape = early_finish_tape()
        local result = rollback_lab.run(tape, {
            profile_name = "early-finish-spec",
            profile = profile({ base_delay_ticks = 6 }),
            network_seed = 6,
            sources = one_remote(5),
        })

        t.is_true(result.success)
        t.is_true(result.metrics.predicted_early_finish)
        t.is_true(result.metrics.reactivation_count > 0)
        t.eq(result.client_final_boundary, result.reference_final_boundary)
        t.eq(result.confirmed_output_tick, result.reference_final_boundary - 1)
    end)

    t.it("supports exactly thirty ticks and fails explicitly at thirty-one", function()
        local tape = varying_tape(40)
        local at_limit = rollback_lab.run(tape, {
            profile_name = "delay-30-spec",
            profile = profile({ base_delay_ticks = 30 }),
            network_seed = 30,
            sources = one_remote(),
        })
        t.is_true(at_limit.success)
        t.eq(at_limit.metrics.max_rollback_depth, rollback_input_history.ROLLBACK_WINDOW_TICKS)
        t.is_true(
            at_limit.metrics.confirmed_redundant_rows_skipped > 0,
            "confirmed tick-zero history is skipped while supported current rows reconcile"
        )
        local replay_boundaries = 0
        local replay_truncations = 0
        for _, row in ipairs(at_limit.event_trace) do
            if row.kind == "replay_boundary" then
                replay_boundaries = replay_boundaries + 1
                t.eq(assert(row.replay_sample).boundary, row.boundary)
            elseif row.kind == "replay_truncate" then
                replay_truncations = replay_truncations + 1
            end
        end
        t.is_true(replay_boundaries > at_limit.input_ticks)
        t.eq(replay_truncations, at_limit.metrics.rollback_count)

        local over_limit = rollback_lab.run(tape, {
            profile_name = "delay-31-spec",
            profile = profile({ base_delay_ticks = 31 }),
            network_seed = 31,
            sources = one_remote(),
        })
        t.is_true(not over_limit.success)
        t.eq(over_limit.status, "late_input_unrecoverable")
        t.eq(over_limit.late_input_tick, 0)
    end)

    t.it("derives over-window terminal stability from the blocked session seam", function()
        local tape = varying_tape(40)
        local campaign = rollback_lab.new_campaign(tape, {
            profile_name = "delay-31-spec",
            profile = profile({ base_delay_ticks = 31 }),
            network_seed = 31,
            sources = one_remote(),
        })
        local result = nil
        while result == nil do
            result = rollback_lab.step_campaign(campaign, #tape.frames)
        end
        t.eq(result.status, "late_input_unrecoverable")
        t.is_true(not rollback_lab.probe_terminal_stability(campaign))

        ---@type any
        local session_module = rollback_session
        local original_step = rollback_session.step
        session_module.step = function(session)
            session._state.input_tick = session._state.input_tick + 1
            return original_step(session)
        end
        local ok, hidden = pcall(rollback_lab.probe_terminal_stability, campaign)
        session_module.step = original_step
        assert(ok, hidden)
        t.is_true(hidden)
    end)

    t.it("reports causal hashes and the first state path for intentional corruption", function()
        local result = rollback_lab.run(varying_tape(16), {
            profile_name = "clean",
            network_seed = 9,
            sources = one_remote(),
            corruption = { tick = 2, slot = 1 },
        })

        t.is_true(not result.success)
        t.eq(result.status, "diverged")
        local divergence = assert(result.divergence)
        t.eq(divergence.causal_tick, 2)
        t.is_true(divergence.expected_hash ~= divergence.actual_hash)
        t.is_true(divergence.first_difference ~= nil)
        t.is_true(assert(divergence.first_difference).path ~= "")
        t.is_true(rollback_lab.summary(result):match("causal_tick=2") ~= nil)
    end)

    t.it("verifies every frozen authoritative boundary incrementally", function()
        local tape = varying_tape(4)
        tape.boundary_hashes[3] = "0000000000000000"
        local campaign = rollback_lab.new_campaign(tape, {
            profile_name = "clean",
            network_seed = 9,
            sources = one_remote(),
            prevalidated_tape = true,
        })

        local ok, err = pcall(rollback_lab.step_campaign, campaign, #tape.frames)
        t.is_true(not ok)
        t.is_true(tostring(err):match("frozen boundary 2 hash mismatch") ~= nil)
    end)

    t.it("keeps timing observers outside repeatable logical evidence", function()
        local tape = varying_tape(20)
        local calls_a = 0
        local calls_b = 1000
        local first = rollback_lab.run(tape, {
            profile_name = "repeat-spec",
            profile = profile({ base_delay_ticks = 3 }),
            network_seed = 77,
            sources = one_remote(),
            measure = function(_, operation)
                calls_a = calls_a + 1
                return operation()
            end,
        })
        local second = rollback_lab.run(tape, {
            profile_name = "repeat-spec",
            profile = profile({ base_delay_ticks = 3 }),
            network_seed = 77,
            sources = one_remote(),
            measure = function(_, operation)
                calls_b = calls_b + 7
                operation()
                return "fabricated observer result"
            end,
        })

        t.is_true(calls_a > 0)
        t.is_true(calls_b > calls_a)
        t.eq(rollback_lab.logical_marker(first), rollback_lab.logical_marker(second))
        t.is_true(rollback_lab.logical_marker(first):match("timing") == nil)

        local missing_call = pcall(rollback_lab.run, tape, {
            profile = profile(),
            sources = one_remote(),
            measure = function() end,
        })
        t.is_true(not missing_call, "an observer cannot skip the owned operation")
        local double_call = pcall(rollback_lab.run, tape, {
            profile = profile(),
            sources = one_remote(),
            measure = function(_, operation)
                operation()
                pcall(operation)
            end,
        })
        t.is_true(not double_call, "a swallowed second call still fails the once guard")
    end)

    t.it("uses collision-safe strings and exact tape/profile identity in markers", function()
        local injected = rollback_lab.run(varying_tape(2, "fixture|profile=forged"), {
            profile = profile({ independent_loss_rate = 0.1 }),
            network_seed = 2,
            sources = one_remote(),
        })
        local adjacent = rollback_lab.run(varying_tape(2, "fixture|profile=forged="), {
            profile_name = "custom|status=forged",
            profile = profile({ independent_loss_rate = 0.10000000000000002 }),
            network_seed = 2,
            sources = one_remote(),
        })
        local injected_marker = rollback_lab.logical_marker(injected)
        local adjacent_marker = rollback_lab.logical_marker(adjacent)

        t.eq(injected.profile, "custom")
        t.is_true(injected_marker:match("fixture|profile=forged") == nil)
        t.is_true(adjacent_marker:match("custom|status=forged") == nil)
        t.is_true(injected.profile_parameters ~= adjacent.profile_parameters)
        t.is_true(injected.tape_digest ~= adjacent.tape_digest)
        t.is_true(injected_marker ~= adjacent_marker)
    end)

    t.it("bounds retained resources and never passes with unconfirmed authority", function()
        local bounded = rollback_lab.run(varying_tape(80), {
            profile_name = "bounded-spec",
            profile = profile({ base_delay_ticks = 3 }),
            network_seed = 3,
            sources = one_remote(),
        })
        t.is_true(bounded.success)
        t.eq(bounded.metrics.peaks.snapshot_count, rollback_input_history.ROLLBACK_WINDOW_TICKS + 1)
        t.is_true(bounded.metrics.peaks.snapshot_bytes >= bounded.metrics.current_snapshot_bytes)
        t.is_true(bounded.metrics.peaks.input_authoritative_samples <= 32 * 8)
        t.is_true(bounded.metrics.peaks.network_authoritative_records <= 7)
        t.eq(bounded.network_diagnostics.pending_envelopes, 0)
        t.is_true(
            bounded.metrics.peaks.network_pending_envelopes
                > bounded.network_diagnostics.pending_envelopes
        )
        t.is_true(
            bounded.metrics.peaks.network_pending_record_references
                > bounded.network_diagnostics.pending_record_references
        )
        t.eq(rawget(bounded.drain, "deliveries"), nil)

        local unconfirmed = rollback_lab.run(varying_tape(10, "unconfirmed"), {
            profile_name = "all-loss-spec",
            profile = profile({ independent_loss_rate = 1 }),
            network_seed = 1,
            sources = one_remote(),
            drain_ticks = 16,
        })
        t.is_true(not unconfirmed.success)
        t.eq(unconfirmed.status, "unconfirmed_authority")
        t.eq(unconfirmed.unconfirmed_tick, 0)
        t.is_true(not unconfirmed.drain.complete)
    end)

    t.it("uses one logical result for incremental and synchronous execution", function()
        local tape = varying_tape(24, "incremental-equivalence")
        local options = {
            profile_name = "incremental-spec",
            profile = profile({ base_delay_ticks = 3 }),
            network_seed = 2001,
            sources = one_remote(),
        }
        local synchronous = rollback_lab.run(tape, options)
        local campaign = rollback_lab.new_campaign(tape, options)
        local incremental = nil
        while incremental == nil do
            incremental = rollback_lab.step_campaign(campaign, 1)
        end
        t.eq(rollback_lab.logical_marker(incremental), rollback_lab.logical_marker(synchronous))
        t.eq(incremental.event_metrics.reference_digest, incremental.event_metrics.confirmed_digest)
        t.eq(incremental.event_metrics.speculative_residue, 0)
        local accounting = incremental.history_accounting
        t.eq(accounting.input.authoritative_bytes, 3421)
        t.eq(accounting.input.predecessor_anchor_bytes, 0)
        t.eq(accounting.input.effective_frame_bytes, 2315)
        t.eq(accounting.input.input_record_bytes, 6131)
        t.eq(accounting.input.total_bytes, 11867)
        t.eq(accounting.output_bytes, 37008)
        t.eq(accounting.event_bytes, 0)
        t.eq(incremental.metrics.peaks.input_bytes, 11867)
        t.eq(incremental.metrics.peaks.output_bytes, 37026)
        t.eq(incremental.metrics.peaks.event_bytes, 833)
        t.is_true(incremental.history_accounting.total_bytes > 0)
        t.is_true(
            incremental.metrics.peaks.history_bytes >= incremental.history_accounting.total_bytes
        )
    end)
end)
