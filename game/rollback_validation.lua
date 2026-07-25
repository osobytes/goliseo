local Vec2 = require("core.vec2")
local match_contract = require("game.match_contract")
local match_observer = require("game.match_observer")
local audio = require("game.audio")
local effects = require("game.render.effects")
local replay = require("game.render.replay")
local match_snapshot = require("sim.match_snapshot")
local rollback_events = require("sim.rollback_events")

---@alias RollbackValidationScenario
---| "possession"
---| "tackle"
---| "shot"
---| "goal"
---| "kickoff"
---| "aerial"
---| "keeper"
---| "full_time"

---@class RollbackValidationTraceRow
---@field kind "reference_step"|"reference_confirmed"|"impaired_diff"|"confirmed"|"impaired_confirmed"|"replay_boundary"|"replay_truncate"
---@field step RollbackEventStepInput|RollbackEventStep?
---@field diff RollbackEventDiff?
---@field boundary integer?
---@field state MatchState?
---@field snapshot MatchSnapshot?
---@field replay_sample RollbackValidationReplaySample?

---@class RollbackValidationFinishOptions
---@field home_team_id string
---@field away_team_id string
---@field initial_combat_state CombatMatchState?
---@field reference_final_state MatchState?
---@field impaired_final_state MatchState?
---@field reference_final_score { home: integer, away: integer }?
---@field impaired_final_score { home: integer, away: integer }?
---@field seed integer?
---@field expected_replay_boundaries integer[]?
---@field expected_replay_samples RollbackValidationReplaySample[]?
---@field expected_replay_truncate_count integer?
---@field required_scenarios RollbackValidationScenario[]?

---@class RollbackValidationReplaySample
---@field boundary integer
---@field ball_x number
---@field ball_y number
---@field score_home integer
---@field score_away integer

---@class RollbackValidationEventMetrics
---@field reference_unique integer
---@field impaired_unique integer
---@field duplicate_confirmed integer
---@field confirmed_without_speculation integer
---@field speculative_added integer
---@field speculative_duplicate_added integer
---@field speculative_revoked integer
---@field speculative_unknown_revoked integer
---@field speculative_replaced integer
---@field speculative_invalid_replaced integer
---@field missing_confirmed integer
---@field unexpected_confirmed integer
---@field mismatched_confirmed integer
---@field terminal_speculative_residue integer
---@field consumer_speculative_residue integer

---@class RollbackValidationConsumerMetrics
---@field audio_cue_count integer
---@field expected_audio_cue_count integer
---@field audio_cues table<string, integer>
---@field expected_audio_cues table<string, integer>
---@field reference_observer_steps integer
---@field impaired_observer_steps integer
---@field replay_record_count integer
---@field replay_truncate_count integer
---@field replay_boundary_count integer
---@field result_count integer

---@class RollbackValidationReport
---@field passed boolean
---@field errors string[]
---@field events RollbackValidationEventMetrics
---@field consumers RollbackValidationConsumerMetrics
---@field scenario_counts table<RollbackValidationScenario, integer>
---@field observer_matched boolean
---@field observer_reference_digest string
---@field observer_impaired_digest string
---@field result_matched boolean
---@field result_reference_digest string
---@field result_impaired_digest string
---@field replay_matched boolean
---@field replay_boundaries integer[]

---@class RollbackValidationAudit
---@field reference_observer MatchObserver
---@field impaired_observer MatchObserver
---@field reference_timeline RollbackEventTimeline
---@field reference_ids table<string, string>
---@field reference_events table<string, RollbackWrappedEvent>
---@field impaired_ids table<string, string>
---@field speculative_ids table<string, string>
---@field scenario_counts table<RollbackValidationScenario, integer>
---@field reference_last_owner InputTeam?
---@field impaired_last_owner InputTeam?
---@field reference_score { home: integer, away: integer }
---@field impaired_score { home: integer, away: integer }
---@field events RollbackValidationEventMetrics
---@field reference_observer_steps integer
---@field impaired_observer_steps integer
---@field replay_record_count integer
---@field replay_truncate_count integer
---@field replay_state MatchState

---@class RollbackValidationModule
local rollback_validation = {}

local SCENARIOS = {
    "possession",
    "tackle",
    "shot",
    "goal",
    "kickoff",
    "aerial",
    "keeper",
    "full_time",
}

local AUDIO_KINDS = {
    touch = true,
    reception = true,
    pass = true,
    block = true,
    catch = true,
    claim = true,
    parry = true,
    tackle = true,
    header = true,
    shot = true,
    volley = true,
    bicycle = true,
}

local AERIAL_KINDS = {
    header = true,
    volley = true,
    bicycle = true,
}

local KEEPER_KINDS = {
    catch = true,
    parry = true,
    tip = true,
    claim = true,
}

---@param value any
---@return string
local function stable_value(value)
    local kind = type(value)
    if kind == "nil" then
        return "nil"
    elseif kind == "number" then
        return "n:" .. match_snapshot.number_bytes(value)
    elseif kind == "boolean" then
        return value and "b:1" or "b:0"
    elseif kind == "string" then
        return ("s:%d:%s"):format(#value, value)
    end
    assert(kind == "table", "rollback validation values must contain only serializable data")
    local keys = {}
    for key in pairs(value) do
        assert(type(key) == "string", "rollback validation table keys must be strings")
        keys[#keys + 1] = key
    end
    table.sort(keys)
    local parts = { "{" }
    for _, key in ipairs(keys) do
        parts[#parts + 1] = stable_value(key)
        parts[#parts + 1] = stable_value(value[key])
    end
    parts[#parts + 1] = "}"
    return table.concat(parts)
end

---@param event RollbackWrappedEvent
---@return string
local function event_signature(event)
    return stable_value({
        id = event.id,
        tick = event.tick,
        domain = event.domain,
        ordinal = event.ordinal,
        payload = event.payload,
    })
end

---@param counts table<RollbackValidationScenario, integer>
---@param scenario RollbackValidationScenario
local function count_scenario(counts, scenario)
    counts[scenario] = counts[scenario] + 1
end

---@param counts table<RollbackValidationScenario, integer>
---@param event RollbackWrappedEvent
local function classify_event(counts, event)
    if event.domain:sub(1, 6) == "match/" then
        local kind = event.payload.kind
        if kind == "tackle" then
            count_scenario(counts, "tackle")
        elseif kind == "shot" then
            count_scenario(counts, "shot")
        end
        if AERIAL_KINDS[kind] then
            count_scenario(counts, "aerial")
        end
        if KEEPER_KINDS[kind] then
            count_scenario(counts, "keeper")
        end
    elseif event.domain == "lifecycle/goal" then
        count_scenario(counts, "goal")
    elseif event.domain == "lifecycle/kickoff" then
        count_scenario(counts, "kickoff")
    elseif event.domain == "lifecycle/full_time" then
        count_scenario(counts, "full_time")
    end
end

---@param audit RollbackValidationAudit
---@param step RollbackEventStep
---@param reference boolean
local function classify_step(audit, step, reference)
    local previous = reference and audit.reference_last_owner or audit.impaired_last_owner
    if step.state.owner_team ~= previous then
        count_scenario(audit.scenario_counts, "possession")
    end
    if reference then
        audit.reference_last_owner = step.state.owner_team
    else
        audit.impaired_last_owner = step.state.owner_team
    end
end

---@param event RollbackWrappedEvent
---@return string?
local function audio_cue(event)
    if event.domain:sub(1, 6) == "match/" then
        return AUDIO_KINDS[event.payload.kind] and event.payload.kind or nil
    elseif event.domain == "lifecycle/goal" then
        return "goal"
    elseif event.domain == "lifecycle/kickoff" then
        return "kickoff"
    elseif event.domain == "lifecycle/full_time" then
        return "full_time"
    end
    return nil
end

---@param state MatchState
---@return InputTeam?
local function owner_team(state)
    local owner = state.owner and state.players[state.owner] or nil
    return owner and owner.team or nil
end

---@param initial_state MatchState
---@param initial_combat_state CombatMatchState?
---@return RollbackValidationAudit
function rollback_validation.new(initial_state, initial_combat_state)
    audio.reset()
    effects.reset()
    replay.reset()
    local scenarios = {}
    for _, name in ipairs(SCENARIOS) do
        scenarios[name] = 0
    end
    ---@cast scenarios table<RollbackValidationScenario, integer>
    local initial_owner = owner_team(initial_state)
    return {
        reference_observer = match_observer.new(initial_state),
        impaired_observer = match_observer.new(initial_state),
        reference_timeline = rollback_events.new(
            match_snapshot.capture(initial_state, initial_combat_state)
        ),
        reference_ids = {},
        reference_events = {},
        impaired_ids = {},
        speculative_ids = {},
        scenario_counts = scenarios,
        reference_last_owner = initial_owner,
        impaired_last_owner = initial_owner,
        reference_score = {
            home = initial_state.score.home,
            away = initial_state.score.away,
        },
        impaired_score = {
            home = initial_state.score.home,
            away = initial_state.score.away,
        },
        events = {
            reference_unique = 0,
            impaired_unique = 0,
            duplicate_confirmed = 0,
            confirmed_without_speculation = 0,
            speculative_added = 0,
            speculative_duplicate_added = 0,
            speculative_revoked = 0,
            speculative_unknown_revoked = 0,
            speculative_replaced = 0,
            speculative_invalid_replaced = 0,
            missing_confirmed = 0,
            unexpected_confirmed = 0,
            mismatched_confirmed = 0,
            terminal_speculative_residue = 0,
            consumer_speculative_residue = 0,
        },
        reference_observer_steps = 0,
        impaired_observer_steps = 0,
        replay_record_count = 0,
        replay_truncate_count = 0,
        replay_state = match_snapshot.restore(
            match_snapshot.capture(initial_state, initial_combat_state)
        ),
    }
end

---@param audit RollbackValidationAudit
---@param step RollbackEventStep
function rollback_validation.observe_reference_step(audit, step)
    local observed = match_observer.observe_confirmed(audit.reference_observer, step)
    if observed then
        audit.reference_observer_steps = audit.reference_observer_steps + 1
        classify_step(audit, step, true)
        audit.reference_score.home = step.state.score.home
        audit.reference_score.away = step.state.score.away
    end
    for _, events in ipairs({
        step.match_events,
        step.combat_events or {},
        step.lifecycle_events,
    }) do
        for _, event in ipairs(events) do
            local signature = event_signature(event)
            local previous = audit.reference_ids[event.id]
            if previous == nil then
                audit.reference_ids[event.id] = signature
                audit.reference_events[event.id] = event
                audit.events.reference_unique = audit.events.reference_unique + 1
                classify_event(audit.scenario_counts, event)
            elseif previous ~= signature then
                audit.events.mismatched_confirmed = audit.events.mismatched_confirmed + 1
            end
        end
    end
end

---@param audit RollbackValidationAudit
---@param diff RollbackEventDiff
function rollback_validation.apply_impaired_diff(audit, diff)
    for _, event in ipairs(diff.revoked) do
        audit.events.speculative_revoked = audit.events.speculative_revoked + 1
        if audit.speculative_ids[event.id] == nil then
            audit.events.speculative_unknown_revoked = audit.events.speculative_unknown_revoked + 1
        else
            audit.speculative_ids[event.id] = nil
        end
    end
    for _, replacement in ipairs(diff.replaced) do
        audit.events.speculative_replaced = audit.events.speculative_replaced + 1
        local before = audit.speculative_ids[replacement.before.id]
        if before == nil or before ~= event_signature(replacement.before) then
            audit.events.speculative_invalid_replaced = audit.events.speculative_invalid_replaced
                + 1
        end
        audit.speculative_ids[replacement.before.id] = nil
        audit.speculative_ids[replacement.after.id] = event_signature(replacement.after)
    end
    for _, event in ipairs(diff.added) do
        audit.events.speculative_added = audit.events.speculative_added + 1
        local signature = event_signature(event)
        if audit.speculative_ids[event.id] ~= nil then
            audit.events.speculative_duplicate_added = audit.events.speculative_duplicate_added + 1
            if audit.speculative_ids[event.id] ~= signature then
                audit.events.speculative_invalid_replaced = audit.events.speculative_invalid_replaced
                    + 1
            end
        else
            audit.speculative_ids[event.id] = signature
        end
    end
    effects.apply_event_diff(diff)
end

---@param audit RollbackValidationAudit
---@param step RollbackEventStep
function rollback_validation.observe_impaired_step(audit, step)
    local observed = match_observer.observe_confirmed(audit.impaired_observer, step)
    if observed then
        audit.impaired_observer_steps = audit.impaired_observer_steps + 1
        classify_step(audit, step, false)
        audit.impaired_score.home = step.state.score.home
        audit.impaired_score.away = step.state.score.away
    else
        audit.events.duplicate_confirmed = audit.events.duplicate_confirmed + 1
    end
    for _, events in ipairs({
        step.match_events,
        step.combat_events or {},
        step.lifecycle_events,
    }) do
        for _, event in ipairs(events) do
            local signature = event_signature(event)
            local previous = audit.impaired_ids[event.id]
            if previous == nil then
                audit.impaired_ids[event.id] = signature
                audit.events.impaired_unique = audit.events.impaired_unique + 1
            elseif previous == signature then
                audit.events.duplicate_confirmed = audit.events.duplicate_confirmed + 1
            else
                audit.events.mismatched_confirmed = audit.events.mismatched_confirmed + 1
            end
            if audit.speculative_ids[event.id] == nil and previous == nil then
                audit.events.confirmed_without_speculation = audit.events.confirmed_without_speculation
                    + 1
            end
            audit.speculative_ids[event.id] = nil
            effects.confirm_event(event.id)
            audio.consume_confirmed(event)
        end
    end
end

---@param audit RollbackValidationAudit
---@param boundary integer
function rollback_validation.truncate_replay(audit, boundary)
    replay.truncate_from(boundary)
    audit.replay_truncate_count = audit.replay_truncate_count + 1
end

---@param audit RollbackValidationAudit
---@param boundary integer
---@param state MatchState
function rollback_validation.record_replay_boundary(audit, boundary, state)
    replay.record_boundary(boundary, state)
    audit.replay_record_count = audit.replay_record_count + 1
end

---@param audit RollbackValidationAudit
---@param boundary integer
---@param sample RollbackValidationReplaySample
function rollback_validation.record_replay_sample(audit, boundary, sample)
    assert(sample.boundary == boundary, "replay sample boundary does not match its trace row")
    local state = audit.replay_state
    state.input_tick = boundary
    state.ball = Vec2.new(sample.ball_x, sample.ball_y)
    state.score.home = sample.score_home
    state.score.away = sample.score_away
    rollback_validation.record_replay_boundary(audit, boundary, state)
end

---@param left integer[]
---@param right integer[]
---@return boolean
local function equal_boundaries(left, right)
    if #left ~= #right then
        return false
    end
    for index, value in ipairs(left) do
        if right[index] ~= value then
            return false
        end
    end
    return true
end

---@param final_boundary integer
---@return integer[]
function rollback_validation.expected_replay_boundaries(final_boundary)
    assert(
        type(final_boundary) == "number"
            and final_boundary == math.floor(final_boundary)
            and final_boundary >= 0,
        "expected replay final boundary must be a non-negative integer"
    )
    local first = math.max(0, final_boundary - replay.capacity() + 1)
    local boundaries = {}
    for boundary = first, final_boundary do
        boundaries[#boundaries + 1] = boundary
    end
    return boundaries
end

---@param expected RollbackValidationReplaySample[]?
---@return boolean
local function replay_samples_match(expected)
    for _, sample in ipairs(expected or {}) do
        local actual = replay.boundary_sample(sample.boundary)
        if
            actual == nil
            or actual.ball_x ~= sample.ball_x
            or actual.ball_y ~= sample.ball_y
            or actual.score_home ~= sample.score_home
            or actual.score_away ~= sample.score_away
        then
            return false
        end
    end
    return true
end

---@param values table<string, integer>
---@return integer
local function sum_counts(values)
    local result = 0
    for _, value in pairs(values) do
        result = result + value
    end
    return result
end

---@param summary ObservedMatchSummary
---@return string
local function observer_digest(summary)
    return stable_value(summary)
end

---@param result ProductMatchResult
---@return string
local function result_digest(result)
    return stable_value(result)
end

---@param supplied { home: integer, away: integer }?
---@param state MatchState?
---@param fallback { home: integer, away: integer }
---@return { home: integer, away: integer }
local function final_score(supplied, state, fallback)
    if supplied then
        return supplied
    elseif state then
        return state.score
    end
    return fallback
end

---@param audit RollbackValidationAudit
---@return table<string, integer>
local function expected_audio_cues(audit)
    local counts = {}
    for id in pairs(audit.reference_ids) do
        local cue = audio_cue(assert(audit.reference_events[id]))
        if cue then
            counts[cue] = (counts[cue] or 0) + 1
        end
    end
    return counts
end

---@param errors string[]
---@param condition boolean
---@param message string
local function require_condition(errors, condition, message)
    if not condition then
        errors[#errors + 1] = message
    end
end

---@param audit RollbackValidationAudit
---@param options RollbackValidationFinishOptions
---@return RollbackValidationReport
function rollback_validation.finish(audit, options)
    local errors = {}
    for id, signature in pairs(audit.reference_ids) do
        local impaired = audit.impaired_ids[id]
        if impaired == nil then
            audit.events.missing_confirmed = audit.events.missing_confirmed + 1
        elseif impaired ~= signature then
            audit.events.mismatched_confirmed = audit.events.mismatched_confirmed + 1
        end
    end
    for id in pairs(audit.impaired_ids) do
        if audit.reference_ids[id] == nil then
            audit.events.unexpected_confirmed = audit.events.unexpected_confirmed + 1
        end
    end
    for _ in pairs(audit.speculative_ids) do
        audit.events.terminal_speculative_residue = audit.events.terminal_speculative_residue + 1
    end
    local effect_diagnostics = effects.diagnostics()
    audit.events.consumer_speculative_residue = #effect_diagnostics.speculative_ids

    local reference_summary = match_observer.finish(audit.reference_observer)
    local impaired_summary = match_observer.finish(audit.impaired_observer)
    local reference_observer_digest = observer_digest(reference_summary)
    local impaired_observer_digest = observer_digest(impaired_summary)
    local observers_match = reference_observer_digest == impaired_observer_digest
    local reference_score = final_score(
        options.reference_final_score,
        options.reference_final_state,
        audit.reference_score
    )
    local impaired_score = final_score(
        options.impaired_final_score,
        options.impaired_final_state,
        audit.impaired_score
    )

    local reference_result = assert(match_contract.new_result({
        home_team_id = options.home_team_id,
        away_team_id = options.away_team_id,
        home_score = reference_score.home,
        away_score = reference_score.away,
        mvp_player_id = reference_summary.mvp_player_id,
        mvp_summary = reference_summary.mvp_summary,
        home_stats = reference_summary.home_stats,
        away_stats = reference_summary.away_stats,
        seed = options.seed,
    }))
    local impaired_result = assert(match_contract.new_result({
        home_team_id = options.home_team_id,
        away_team_id = options.away_team_id,
        home_score = impaired_score.home,
        away_score = impaired_score.away,
        mvp_player_id = impaired_summary.mvp_player_id,
        mvp_summary = impaired_summary.mvp_summary,
        home_stats = impaired_summary.home_stats,
        away_stats = impaired_summary.away_stats,
        seed = options.seed,
    }))
    local reference_result_digest = result_digest(reference_result)
    local impaired_result_digest = result_digest(impaired_result)
    local results_match = reference_result_digest == impaired_result_digest

    local replay_diagnostics = replay.diagnostics()
    local replay_matched = (
        options.expected_replay_boundaries == nil
        or equal_boundaries(replay_diagnostics.boundaries, options.expected_replay_boundaries)
    )
        and replay_samples_match(options.expected_replay_samples)
        and (
            options.expected_replay_truncate_count == nil
            or audit.replay_truncate_count == options.expected_replay_truncate_count
        )
    local actual_audio_cues = audio.confirmed_cue_counts()
    local expected_cues = expected_audio_cues(audit)

    require_condition(errors, audit.events.missing_confirmed == 0, "confirmed events are missing")
    require_condition(
        errors,
        audit.events.unexpected_confirmed == 0,
        "unexpected confirmed events were presented"
    )
    require_condition(
        errors,
        audit.events.mismatched_confirmed == 0,
        "stable confirmed event IDs changed payload"
    )
    require_condition(
        errors,
        audit.events.confirmed_without_speculation == 0,
        "confirmed events were missing from the speculative presentation timeline"
    )
    require_condition(
        errors,
        audit.events.speculative_unknown_revoked == 0,
        "speculative revocation referenced an unknown event"
    )
    require_condition(
        errors,
        audit.events.speculative_invalid_replaced == 0,
        "speculative replacement did not match the active event"
    )
    require_condition(
        errors,
        audit.events.terminal_speculative_residue == 0,
        "speculative event ledger retained terminal residue"
    )
    require_condition(
        errors,
        audit.events.consumer_speculative_residue == 0,
        "effects consumer retained terminal speculative residue"
    )
    require_condition(
        errors,
        stable_value(actual_audio_cues) == stable_value(expected_cues),
        "confirmed audio cue counts differ from authority"
    )
    require_condition(
        errors,
        audit.reference_observer_steps == audit.impaired_observer_steps,
        "confirmed observer step count differs from authority"
    )
    require_condition(errors, observers_match, "observer statistics differ from authority")
    require_condition(errors, results_match, "product result differs from authority")
    require_condition(errors, replay_matched, "replay boundaries differ from the expected timeline")
    for _, scenario in ipairs(options.required_scenarios or {}) do
        require_condition(
            errors,
            audit.scenario_counts[scenario] > 0,
            "required rollback scenario is missing: " .. scenario
        )
    end

    return {
        passed = #errors == 0,
        errors = errors,
        events = audit.events,
        consumers = {
            audio_cue_count = sum_counts(actual_audio_cues),
            expected_audio_cue_count = sum_counts(expected_cues),
            audio_cues = actual_audio_cues,
            expected_audio_cues = expected_cues,
            reference_observer_steps = audit.reference_observer_steps,
            impaired_observer_steps = audit.impaired_observer_steps,
            replay_record_count = audit.replay_record_count,
            replay_truncate_count = audit.replay_truncate_count,
            replay_boundary_count = replay_diagnostics.count,
            result_count = 2,
        },
        scenario_counts = audit.scenario_counts,
        observer_matched = observers_match,
        observer_reference_digest = reference_observer_digest,
        observer_impaired_digest = impaired_observer_digest,
        result_matched = results_match,
        result_reference_digest = reference_result_digest,
        result_impaired_digest = impaired_result_digest,
        replay_matched = replay_matched,
        replay_boundaries = replay_diagnostics.boundaries,
    }
end

---@param audit RollbackValidationAudit
---@param supplied RollbackEventStepInput
local function observe_raw_reference_step(audit, supplied)
    local tick = supplied.output.tick
    assert(rollback_events.apply(audit.reference_timeline, tick, tick, { supplied }))
    local confirmed = rollback_events.confirm(audit.reference_timeline, tick)
    assert(#confirmed == 1, "reference rollback validation step did not confirm exactly once")
    rollback_validation.observe_reference_step(audit, confirmed[1])
end

---@param initial_state MatchState
---@param trace RollbackValidationTraceRow[]
---@param finish_options RollbackValidationFinishOptions
---@return RollbackValidationReport
function rollback_validation.run(initial_state, trace, finish_options)
    local audit = rollback_validation.new(initial_state, finish_options.initial_combat_state)
    for _, row in ipairs(trace) do
        if row.kind == "reference_step" then
            observe_raw_reference_step(audit, row.step --[[@as RollbackEventStepInput]])
        elseif row.kind == "reference_confirmed" then
            rollback_validation.observe_reference_step(audit, row.step --[[@as RollbackEventStep]])
        elseif row.kind == "impaired_diff" then
            rollback_validation.apply_impaired_diff(audit, assert(row.diff))
        elseif row.kind == "confirmed" or row.kind == "impaired_confirmed" then
            rollback_validation.observe_impaired_step(audit, row.step --[[@as RollbackEventStep]])
        elseif row.kind == "replay_truncate" then
            rollback_validation.truncate_replay(audit, assert(row.boundary))
        elseif row.kind == "replay_boundary" then
            local boundary = assert(row.boundary)
            if row.replay_sample then
                rollback_validation.record_replay_sample(audit, boundary, row.replay_sample)
            else
                local state = row.state
                    or match_snapshot.restore(
                        assert(row.snapshot, "replay validation boundary is missing its snapshot")
                    )
                rollback_validation.record_replay_boundary(audit, boundary, state)
            end
        else
            assert(false, "unknown rollback validation trace row: " .. tostring(row.kind))
        end
    end
    return rollback_validation.finish(audit, finish_options)
end

return rollback_validation
