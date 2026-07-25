-- Pure stable-event timeline for rollback presentation and confirmed consumers.
-- Identity is derived from simulation causality and never enters MatchState.

local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")

---@alias RollbackEventDomain
---| "lifecycle/goal"
---| "lifecycle/kickoff"
---| "lifecycle/full_time"
---| string

---@alias RollbackLifecycleKind "goal"|"kickoff"|"full_time"

---@class RollbackLifecyclePayload
---@field kind RollbackLifecycleKind
---@field team InputTeam?
---@field score { home: integer, away: integer }

---@class RollbackWrappedMatchEvent
---@field id string
---@field tick integer
---@field domain RollbackEventDomain
---@field ordinal integer
---@field payload MatchEvent

---@class RollbackWrappedLifecycleEvent
---@field id string
---@field tick integer
---@field domain RollbackEventDomain
---@field ordinal integer
---@field payload RollbackLifecyclePayload

---@class RollbackWrappedCombatEvent
---@field id string
---@field tick integer
---@field domain RollbackEventDomain
---@field ordinal integer
---@field payload CombatEvent

---@alias RollbackWrappedEvent
---| RollbackWrappedMatchEvent
---| RollbackWrappedCombatEvent
---| RollbackWrappedLifecycleEvent

---@class RollbackEventReplacement
---@field before RollbackWrappedEvent
---@field after RollbackWrappedEvent

---@class RollbackEventDiff
---@field added RollbackWrappedEvent[]
---@field revoked RollbackWrappedEvent[]
---@field replaced RollbackEventReplacement[]

---@alias RollbackEventsStatus "active"|"unconfirmed_window_exceeded"
---@alias RollbackEventsErrorCode "unconfirmed_window_exceeded"

---@class RollbackConfirmedStateView
---@field score { home: integer, away: integer }
---@field time_left number
---@field finished boolean
---@field owner_id string?
---@field owner_team InputTeam?

---@class RollbackEventPlayerIdentity
---@field id string
---@field team InputTeam

---@class RollbackEventStepInput
---@field output RollbackTickOutput
---@field snapshot MatchSnapshot -- Canonical post-step boundary.

---@class RollbackEventStep
---@field tick integer
---@field start_boundary integer
---@field end_boundary integer
---@field state RollbackConfirmedStateView
---@field match_events RollbackWrappedMatchEvent[]
---@field combat_events RollbackWrappedCombatEvent[]?
---@field lifecycle_events RollbackWrappedLifecycleEvent[]

---@class RollbackEventTimeline
---@field _status RollbackEventsStatus
---@field _max_unconfirmed_ticks integer
---@field _confirmed_tick integer
---@field _confirmed_boundary integer
---@field _confirmed_state RollbackConfirmedStateView
---@field _players table<integer, RollbackEventPlayerIdentity>
---@field _steps table<integer, RollbackEventStep>

---@class RollbackEventsDiagnostics
---@field status RollbackEventsStatus
---@field confirmed_tick integer
---@field confirmed_boundary integer
---@field max_unconfirmed_ticks integer
---@field retained_step_count integer
---@field retained_event_count integer
---@field oldest_tick integer?
---@field latest_tick integer?

---@class RollbackEventsAccounting
---@field retained_step_bytes integer
---@field total_bytes integer

---@class RollbackEventsModule
local rollback_events = {}

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

---@param left any
---@param right any
---@return boolean
local function equal_value(left, right)
    if type(left) ~= type(right) then
        return false
    end
    if type(left) == "number" then
        return match_snapshot.number_bytes(left) == match_snapshot.number_bytes(right)
    end
    if type(left) ~= "table" then
        return left == right
    end
    for key, value in pairs(left) do
        if not equal_value(value, right[key]) then
            return false
        end
    end
    for key in pairs(right) do
        if left[key] == nil then
            return false
        end
    end
    return true
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
    assert(false, "rollback event accounting cannot encode " .. kind)
    return ""
end

---@param timeline RollbackEventTimeline
local function assert_timeline(timeline)
    assert(type(timeline) == "table", "rollback event timeline is required")
    assert(
        timeline._status == "active" or timeline._status == "unconfirmed_window_exceeded",
        "rollback event timeline status is invalid"
    )
    assert(
        type(timeline._max_unconfirmed_ticks) == "number",
        "rollback event unconfirmed window is missing"
    )
    assert(type(timeline._confirmed_tick) == "number", "rollback event confirmed cursor is missing")
    assert(
        type(timeline._confirmed_boundary) == "number",
        "rollback event confirmed boundary is missing"
    )
    assert(type(timeline._confirmed_state) == "table", "rollback event confirmed state is missing")
    assert(type(timeline._players) == "table", "rollback event player identities are missing")
    assert(type(timeline._steps) == "table", "rollback event speculative steps are missing")
end

---@param tick integer
---@param domain RollbackEventDomain
---@param ordinal integer
---@return string
local function event_id(tick, domain, ordinal)
    assert(
        tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback event tick is outside the input range"
    )
    assert(#domain <= 999, "rollback event domain is too long")
    assert(ordinal >= 1 and ordinal <= 9999, "rollback event ordinal is outside the ID range")
    return ("%010d|%03d:%s|%04d"):format(tick, #domain, domain, ordinal)
end

---@param event MatchEvent
---@return MatchEvent
local function copy_match_event(event)
    local result = {}
    for key, value in pairs(event) do
        assert(type(value) ~= "table", "rollback match event payloads must contain scalars")
        result[key] = value
    end
    ---@cast result MatchEvent
    return result
end

---@param event RollbackWrappedEvent
---@return RollbackWrappedEvent
local function copy_wrapped_event(event)
    return {
        id = event.id,
        tick = event.tick,
        domain = event.domain,
        ordinal = event.ordinal,
        payload = copy_value(event.payload),
    }
end

---@param events RollbackWrappedMatchEvent[]
---@return RollbackWrappedMatchEvent[]
local function copy_match_wrappers(events)
    local result = {}
    for index, event in ipairs(events) do
        result[index] = copy_wrapped_event(event)
    end
    ---@cast result RollbackWrappedMatchEvent[]
    return result
end

---@param events RollbackWrappedCombatEvent[]
---@return RollbackWrappedCombatEvent[]
local function copy_combat_wrappers(events)
    local result = {}
    for index, event in ipairs(events) do
        result[index] = copy_wrapped_event(event)
    end
    ---@cast result RollbackWrappedCombatEvent[]
    return result
end

---@param events RollbackWrappedLifecycleEvent[]
---@return RollbackWrappedLifecycleEvent[]
local function copy_lifecycle_wrappers(events)
    local result = {}
    for index, event in ipairs(events) do
        result[index] = copy_wrapped_event(event)
    end
    ---@cast result RollbackWrappedLifecycleEvent[]
    return result
end

---@param state RollbackConfirmedStateView
---@return RollbackConfirmedStateView
local function copy_state_view(state)
    return {
        score = { home = state.score.home, away = state.score.away },
        time_left = state.time_left,
        finished = state.finished,
        owner_id = state.owner_id,
        owner_team = state.owner_team,
    }
end

---@param step RollbackEventStep
---@return RollbackEventStep
local function copy_step(step)
    local result = {
        tick = step.tick,
        start_boundary = step.start_boundary,
        end_boundary = step.end_boundary,
        state = copy_state_view(step.state),
        match_events = copy_match_wrappers(step.match_events),
        lifecycle_events = copy_lifecycle_wrappers(step.lifecycle_events),
    }
    if step.combat_events then
        result.combat_events = copy_combat_wrappers(step.combat_events)
    end
    return result
end

---@param tick integer
---@param events MatchEvent[]
---@return RollbackWrappedMatchEvent[]
local function wrap_match_events(tick, events)
    local counts = {}
    local wrapped = {}
    for index, event in ipairs(events) do
        local domain = "match/" .. event.kind
        local ordinal = (counts[domain] or 0) + 1
        counts[domain] = ordinal
        wrapped[index] = {
            id = event_id(tick, domain, ordinal),
            tick = tick,
            domain = domain,
            ordinal = ordinal,
            payload = copy_match_event(event),
        }
    end
    return wrapped
end

---@param tick integer
---@param events CombatEvent[]
---@return RollbackWrappedCombatEvent[]
local function wrap_combat_events(tick, events)
    local counts = {}
    local wrapped = {}
    for index, event in ipairs(events) do
        local sequence = assert(
            event.source_sequence,
            "rollback combat event is missing its stable source sequence"
        )
        local domain = "combat/" .. event.kind .. "/" .. tostring(sequence)
        local ordinal = (counts[domain] or 0) + 1
        counts[domain] = ordinal
        wrapped[index] = {
            id = event_id(tick, domain, ordinal),
            tick = tick,
            domain = domain,
            ordinal = ordinal,
            payload = copy_value(event),
        }
    end
    return wrapped
end

---@param tick integer
---@param domain RollbackEventDomain
---@param payload RollbackLifecyclePayload
---@return RollbackWrappedLifecycleEvent
local function lifecycle_event(tick, domain, payload)
    return {
        id = event_id(tick, domain, 1),
        tick = tick,
        domain = domain,
        ordinal = 1,
        payload = copy_value(payload),
    }
end

---@param tick integer
---@param pre RollbackConfirmedStateView
---@param post RollbackConfirmedStateView
---@return RollbackWrappedLifecycleEvent[]
local function derive_lifecycle_events(tick, pre, post)
    local score = { home = post.score.home, away = post.score.away }
    local events = {}
    local scoring_team = nil
    if post.score.home == pre.score.home + 1 and post.score.away == pre.score.away then
        scoring_team = "home"
    elseif post.score.away == pre.score.away + 1 and post.score.home == pre.score.home then
        scoring_team = "away"
    else
        assert(
            post.score.home == pre.score.home and post.score.away == pre.score.away,
            "rollback lifecycle score transition must contain at most one goal"
        )
    end

    if scoring_team then
        events[#events + 1] = lifecycle_event(tick, "lifecycle/goal", {
            kind = "goal",
            team = scoring_team,
            score = score,
        })
        if not post.finished then
            events[#events + 1] = lifecycle_event(tick, "lifecycle/kickoff", {
                kind = "kickoff",
                team = scoring_team == "home" and "away" or "home",
                score = score,
            })
        end
    end
    if not pre.finished and post.finished then
        events[#events + 1] = lifecycle_event(tick, "lifecycle/full_time", {
            kind = "full_time",
            score = score,
        })
    else
        assert(not pre.finished or post.finished, "rollback lifecycle cannot return from full time")
    end
    return events
end

---@param output RollbackTickOutput
---@param state MatchState
---@param combat_state CombatMatchState?
local function assert_step_coherent(output, state, combat_state)
    assert(type(output) == "table", "rollback event output is required")
    assert(output.tick == output.start_boundary, "rollback event output start boundary is invalid")
    assert(output.end_boundary == output.tick + 1, "rollback event output must advance one tick")
    assert(
        state.input_tick == output.end_boundary,
        "rollback event snapshot boundary does not match output"
    )
    assert(output.state.score.home == state.score.home, "rollback output home score differs")
    assert(output.state.score.away == state.score.away, "rollback output away score differs")
    assert(output.state.time_left == state.time_left, "rollback output time differs")
    assert(output.state.finished == state.finished, "rollback output finish differs")
    assert(output.finished == state.finished, "rollback output finish flag differs")
    assert(
        equal_value(output.events, state.events),
        "rollback output events differ from post-step snapshot"
    )
    if combat_state then
        assert(
            equal_value(output.combat_events, combat_state.events),
            "rollback output combat events differ from post-step snapshot"
        )
    else
        assert(output.combat_events == nil, "soccer output cannot contain combat events")
    end
end

---@param state MatchState
---@return table<integer, RollbackEventPlayerIdentity>
local function player_identities(state)
    local result = {}
    for index, player in ipairs(state.players) do
        assert(
            player.team == "home" or player.team == "away",
            "rollback event player team is invalid"
        )
        result[index] = { id = player.id, team = player.team }
    end
    return result
end

---@param state MatchState
---@param players table<integer, RollbackEventPlayerIdentity>
---@return RollbackConfirmedStateView
local function state_view(state, players)
    local owner = nil
    if state.owner ~= nil then
        owner = assert(players[state.owner], "rollback event owner is outside the initial roster")
        local current = assert(state.players[state.owner], "rollback event owner player is missing")
        assert(
            current.id == owner.id and current.team == owner.team,
            "rollback event owner identity differs from the initial roster"
        )
    end
    return {
        score = { home = state.score.home, away = state.score.away },
        time_left = state.time_left,
        finished = state.finished,
        owner_id = owner and owner.id or nil,
        owner_team = owner and owner.team or nil,
    }
end

---@param timeline RollbackEventTimeline
---@return integer
local function latest_speculative_tick(timeline)
    local latest = timeline._confirmed_tick
    for tick in pairs(timeline._steps) do
        latest = math.max(latest, tick)
    end
    return latest
end

---@param timeline RollbackEventTimeline
---@param tick integer
---@return RollbackConfirmedStateView
local function state_before(timeline, tick)
    if tick == timeline._confirmed_boundary then
        return timeline._confirmed_state
    end
    local previous = assert(
        timeline._steps[tick - 1],
        ("rollback event step %d is missing its previous boundary"):format(tick)
    )
    assert(
        previous.end_boundary == tick,
        ("rollback event step %d has a noncontiguous previous boundary"):format(tick)
    )
    return previous.state
end

---@param output RollbackTickOutput
---@param snapshot MatchSnapshot
---@param before RollbackConfirmedStateView
---@param players table<integer, RollbackEventPlayerIdentity>
---@return RollbackEventStep
local function make_step(output, snapshot, before, players)
    local canonical_state, canonical_combat = match_snapshot.restore(snapshot)
    assert_step_coherent(output, canonical_state, canonical_combat)
    local post = state_view(canonical_state, players)
    local result = {
        tick = output.tick,
        start_boundary = output.start_boundary,
        end_boundary = output.end_boundary,
        state = post,
        match_events = wrap_match_events(output.tick, output.events),
        lifecycle_events = derive_lifecycle_events(output.tick, before, post),
    }
    if output.combat_events then
        result.combat_events = wrap_combat_events(output.tick, output.combat_events)
    end
    return result
end

---@param steps table<integer, RollbackEventStep>
---@param from_tick integer
---@param through_tick integer
---@return RollbackWrappedEvent[]
local function ordered_events(steps, from_tick, through_tick)
    local events = {}
    for tick = from_tick, through_tick do
        local step = steps[tick]
        if step then
            for _, event in ipairs(step.match_events) do
                events[#events + 1] = event
            end
            for _, event in ipairs(step.combat_events or {}) do
                events[#events + 1] = event
            end
            for _, event in ipairs(step.lifecycle_events) do
                events[#events + 1] = event
            end
        end
    end
    return events
end

---@param old_events RollbackWrappedEvent[]
---@param new_events RollbackWrappedEvent[]
---@return RollbackEventDiff
local function diff_events(old_events, new_events)
    local old_by_id = {}
    local new_by_id = {}
    for _, event in ipairs(old_events) do
        assert(old_by_id[event.id] == nil, "rollback event identity collision in stale timeline")
        old_by_id[event.id] = event
    end
    for _, event in ipairs(new_events) do
        assert(
            new_by_id[event.id] == nil,
            "rollback event identity collision in corrected timeline"
        )
        new_by_id[event.id] = event
    end

    local diff = { added = {}, revoked = {}, replaced = {} }
    for _, event in ipairs(new_events) do
        local old = old_by_id[event.id]
        if old == nil then
            diff.added[#diff.added + 1] = copy_wrapped_event(event)
        elseif not equal_value(old.payload, event.payload) then
            diff.replaced[#diff.replaced + 1] = {
                before = copy_wrapped_event(old),
                after = copy_wrapped_event(event),
            }
        end
    end
    for _, event in ipairs(old_events) do
        if new_by_id[event.id] == nil then
            diff.revoked[#diff.revoked + 1] = copy_wrapped_event(event)
        end
    end
    return diff
end

---@param timeline RollbackEventTimeline
---@return integer
local function retained_step_count(timeline)
    local count = 0
    for _ in pairs(timeline._steps) do
        count = count + 1
    end
    return count
end

---@param steps RollbackEventStepInput[]
local function assert_dense_steps(steps)
    local count = 0
    for index in pairs(steps) do
        assert(
            type(index) == "number"
                and index == math.floor(index)
                and index >= 1
                and index <= #steps,
            "rollback event corrected steps must be a dense array"
        )
        count = count + 1
    end
    assert(count == #steps, "rollback event corrected steps must be a dense array")
end

---@param initial_snapshot MatchSnapshot Canonical boundary zero.
---@param max_unconfirmed_ticks integer?
---@return RollbackEventTimeline
function rollback_events.new(initial_snapshot, max_unconfirmed_ticks)
    max_unconfirmed_ticks = max_unconfirmed_ticks or rollback_input_history.ROLLBACK_WINDOW_TICKS
    assert(
        type(max_unconfirmed_ticks) == "number"
            and max_unconfirmed_ticks == math.floor(max_unconfirmed_ticks)
            and max_unconfirmed_ticks >= 1
            and max_unconfirmed_ticks <= rollback_input_history.ROLLBACK_WINDOW_TICKS,
        "rollback event unconfirmed window must be a positive bounded integer"
    )
    local initial_state = match_snapshot.restore(initial_snapshot)
    assert(initial_state.input_tick == 0, "rollback event timeline requires boundary zero")
    local players = player_identities(initial_state)
    return {
        _status = "active",
        _max_unconfirmed_ticks = max_unconfirmed_ticks,
        _confirmed_tick = -1,
        _confirmed_boundary = 0,
        _confirmed_state = state_view(initial_state, players),
        _players = players,
        _steps = {},
    }
end

---@param timeline RollbackEventTimeline
---@param replaced_from_tick integer
---@param replaced_through_tick integer
---@param steps RollbackEventStepInput[]
---@return RollbackEventDiff?, string?, RollbackEventsErrorCode?
function rollback_events.apply(timeline, replaced_from_tick, replaced_through_tick, steps)
    assert_timeline(timeline)
    if timeline._status == "unconfirmed_window_exceeded" then
        return nil,
            "rollback event timeline cannot accept steps after its unconfirmed window",
            "unconfirmed_window_exceeded"
    end
    assert(
        type(replaced_from_tick) == "number"
            and replaced_from_tick == math.floor(replaced_from_tick)
            and replaced_from_tick >= 0,
        "rollback event replacement start must be a non-negative integer"
    )
    assert(
        type(replaced_through_tick) == "number"
            and replaced_through_tick == math.floor(replaced_through_tick)
            and replaced_through_tick >= replaced_from_tick,
        "rollback event replacement end must not precede its start"
    )
    assert(
        replaced_from_tick > timeline._confirmed_tick,
        "rollback events cannot correct an already-confirmed tick"
    )
    assert(type(steps) == "table", "rollback event corrected steps are required")
    assert_dense_steps(steps)
    assert(#steps > 0, "rollback event replacement requires at least one corrected step")

    local latest = latest_speculative_tick(timeline)
    local replacing = timeline._steps[replaced_from_tick] ~= nil
    if replacing then
        assert(
            replaced_through_tick == latest,
            "rollback event correction must replace the complete speculative tail"
        )
        for tick = replaced_from_tick, replaced_through_tick do
            assert(
                timeline._steps[tick] ~= nil,
                ("rollback event correction is missing stale tick %d"):format(tick)
            )
        end
    else
        assert(
            replaced_from_tick == latest + 1
                and replaced_through_tick == replaced_from_tick
                and #steps == 1,
            "rollback event normal apply must append exactly one contiguous step"
        )
    end

    local previous_state = state_before(timeline, replaced_from_tick)
    local old_steps = {}
    for tick = replaced_from_tick, replaced_through_tick do
        old_steps[tick] = timeline._steps[tick]
    end

    local new_steps = {}
    for index, supplied in ipairs(steps) do
        assert(type(supplied) == "table", "rollback event step input is required")
        assert(
            not previous_state.finished,
            "rollback event timeline cannot apply a step after full time"
        )
        local output = supplied.output
        local expected_tick = replaced_from_tick + index - 1
        assert(
            output.tick == expected_tick,
            ("rollback event corrected step %d is noncontiguous"):format(output.tick)
        )
        assert(
            output.tick <= replaced_through_tick,
            "rollback event corrected steps extend beyond the replaced interval"
        )
        local step = make_step(output, supplied.snapshot, previous_state, timeline._players)
        new_steps[step.tick] = step
        previous_state = step.state
    end
    local replaced_count = replaced_through_tick - replaced_from_tick + 1
    if replacing and #steps < replaced_count then
        local final_step = assert(new_steps[replaced_from_tick + #steps - 1])
        assert(
            final_step.state.finished,
            "rollback event correction may shorten the stale tail only at full time"
        )
    end

    local retained_after = retained_step_count(timeline)
        - (replacing and replaced_count or 0)
        + #steps
    if retained_after > timeline._max_unconfirmed_ticks then
        timeline._status = "unconfirmed_window_exceeded"
        return nil,
            ("rollback event confirmation stalled beyond the %d-tick unconfirmed window"):format(
                timeline._max_unconfirmed_ticks
            ),
            "unconfirmed_window_exceeded"
    end

    local old_events = ordered_events(old_steps, replaced_from_tick, replaced_through_tick)
    local corrected_through = replaced_from_tick + #steps - 1
    local new_events = ordered_events(new_steps, replaced_from_tick, corrected_through)
    local diff = diff_events(old_events, new_events)

    for tick = replaced_from_tick, replaced_through_tick do
        timeline._steps[tick] = nil
    end
    for tick, step in pairs(new_steps) do
        timeline._steps[tick] = step
    end
    return diff
end

---@param timeline RollbackEventTimeline
---@param confirmed_output_tick integer
---@return RollbackEventStep[]
function rollback_events.confirm(timeline, confirmed_output_tick)
    assert_timeline(timeline)
    assert(
        timeline._status == "active",
        "rollback event timeline cannot confirm after its unconfirmed window is exceeded"
    )
    assert(
        type(confirmed_output_tick) == "number"
            and confirmed_output_tick == math.floor(confirmed_output_tick)
            and confirmed_output_tick >= -1,
        "rollback event confirmation must be an integer at or after -1"
    )
    assert(
        confirmed_output_tick >= timeline._confirmed_tick,
        "rollback event confirmation cannot move backward"
    )
    if confirmed_output_tick == timeline._confirmed_tick then
        return {}
    end

    local confirmed = {}
    local boundary = timeline._confirmed_boundary
    local final_state = timeline._confirmed_state
    for tick = timeline._confirmed_tick + 1, confirmed_output_tick do
        local step = assert(
            timeline._steps[tick],
            ("rollback event confirmation is missing tick %d"):format(tick)
        )
        assert(
            step.start_boundary == boundary,
            ("rollback event confirmation is noncontiguous at tick %d"):format(tick)
        )
        confirmed[#confirmed + 1] = copy_step(step)
        boundary = step.end_boundary
        final_state = step.state
    end

    timeline._confirmed_tick = confirmed_output_tick
    timeline._confirmed_boundary = boundary
    timeline._confirmed_state = copy_state_view(final_state)
    for tick = confirmed_output_tick - #confirmed + 1, confirmed_output_tick do
        timeline._steps[tick] = nil
    end
    return confirmed
end

---@param timeline RollbackEventTimeline
---@return RollbackEventsDiagnostics
function rollback_events.diagnostics(timeline)
    assert_timeline(timeline)
    local step_count = 0
    local event_count = 0
    local oldest = nil
    local latest = nil
    for tick, step in pairs(timeline._steps) do
        step_count = step_count + 1
        event_count = event_count
            + #step.match_events
            + #(step.combat_events or {})
            + #step.lifecycle_events
        oldest = oldest and math.min(oldest, tick) or tick
        latest = latest and math.max(latest, tick) or tick
    end
    return {
        status = timeline._status,
        confirmed_tick = timeline._confirmed_tick,
        confirmed_boundary = timeline._confirmed_boundary,
        max_unconfirmed_ticks = timeline._max_unconfirmed_ticks,
        retained_step_count = step_count,
        retained_event_count = event_count,
        oldest_tick = oldest,
        latest_tick = latest,
    }
end

-- Exact logical bytes retained in the speculative event window. Confirmed
-- presentation history lives in game-layer consumers and is not rollback
-- history.
---@param timeline RollbackEventTimeline
---@return RollbackEventsAccounting
function rollback_events.accounting(timeline)
    assert_timeline(timeline)
    local retained_step_bytes = 0
    for _, step in pairs(timeline._steps) do
        retained_step_bytes = retained_step_bytes + #canonical_payload(step)
    end
    return {
        retained_step_bytes = retained_step_bytes,
        total_bytes = retained_step_bytes,
    }
end

return rollback_events
