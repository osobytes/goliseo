-- Pure outfield decision and assignment helpers. Callers own all clocks, state,
-- candidate construction, and action execution; this module only resolves
-- serializable values.

local rng = require("core.rng")

---@alias TeamPhase "attack"|"defend"|"loose"|"counterpress"|"counterattack"
---@alias BrainPossession "team"|"opponent"|"loose"
---@alias BrainTransition "won"|"lost"
---@alias RunType "in_behind"|"come_short"|"hold_width"
---@alias PressMode "contain"|"commit"
---@alias PressReason
---|"heavy_touch"
---|"exposed_ball"
---|"cover"
---|"box_desperation"
---|"low_discipline"
---|"no_trigger"

---@class BrainPhaseContext
---@field possession BrainPossession
---@field transition BrainTransition?
---@field transition_elapsed number
---@field counterpress_window number
---@field counterattack_window number

---@class PresserCandidate
---@field player_index integer
---@field distance_cost number
---@field eligible boolean?

---@class BrainRunTarget
---@field score number
---@field x number
---@field y number
---@field duration number

---@class BrainRunPlayer
---@field player_index integer
---@field eligible boolean?
---@field in_behind BrainRunTarget?
---@field come_short BrainRunTarget?
---@field hold_width BrainRunTarget?

---@class BrainRunContext
---@field players BrainRunPlayer[]

---@class RunCandidate
---@field player_index integer
---@field run_type RunType
---@field score number
---@field target_x number
---@field target_y number
---@field duration number

---@class RunSlot
---@field player_index integer
---@field run_type RunType
---@field score number
---@field target_x number
---@field target_y number
---@field granted_at number
---@field expires_at number

---@class BrainPressContext
---@field heavy_touch boolean
---@field exposed_ball boolean
---@field cover_available boolean
---@field box_desperation boolean
---@field press_discipline number
---@field low_discipline_threshold number

---@alias BrainPayloadValue boolean|number|string

---@class BrainScoredOption
---@field id string
---@field kind string
---@field score number
---@field payload table<string, BrainPayloadValue>?
---@field reference string|integer?

---@alias CarrierOption BrainScoredOption

---@class BrainModule
local brain = {}

local RUN_TYPE_ORDER = {
    in_behind = 1,
    come_short = 2,
    hold_width = 3,
}

---@param value number
---@return boolean
local function is_finite(value)
    return value == value and value ~= math.huge and value ~= -math.huge
end

---@param value number
---@param label string
local function assert_finite(value, label)
    assert(is_finite(value), label .. " must be finite")
end

---@param value integer
---@param label string
local function assert_positive_index(value, label)
    assert_finite(value, label)
    assert(value == math.floor(value) and value >= 1, label .. " must be a positive integer")
end

---@param value number
---@return number
local function clamp_unit(value)
    if value ~= value or value == -math.huge then
        return 0
    end
    if value == math.huge then
        return 1
    end
    return math.max(0, math.min(1, value))
end

---@param context BrainPhaseContext
---@return TeamPhase
function brain.phase(context)
    assert(
        context.possession == "team"
            or context.possession == "opponent"
            or context.possession == "loose",
        "phase possession must be team, opponent, or loose"
    )
    assert(
        context.transition == nil or context.transition == "won" or context.transition == "lost",
        "phase transition must be won, lost, or nil"
    )
    assert_finite(context.transition_elapsed, "phase transition elapsed")
    assert(context.transition_elapsed >= 0, "phase transition elapsed must be non-negative")
    assert_finite(context.counterpress_window, "counterpress window")
    assert(context.counterpress_window >= 0, "counterpress window must be non-negative")
    assert_finite(context.counterattack_window, "counterattack window")
    assert(context.counterattack_window >= 0, "counterattack window must be non-negative")

    if
        context.transition == "lost"
        and context.transition_elapsed < context.counterpress_window
    then
        return "counterpress"
    end
    if
        context.transition == "won"
        and context.transition_elapsed < context.counterattack_window
    then
        return "counterattack"
    end
    if context.possession == "team" then
        return "attack"
    end
    if context.possession == "opponent" then
        return "defend"
    end
    return "loose"
end

-- Maps a normalized scan rate to a personal interval. Invalid scan-rate
-- scalars are saturated safely; interval endpoints remain caller-owned
-- programmer inputs.
---@param scan_rate number
---@param slow_seconds number
---@param fast_seconds number
---@return number
function brain.refresh_interval(scan_rate, slow_seconds, fast_seconds)
    assert_finite(slow_seconds, "slow refresh interval")
    assert(slow_seconds >= 0, "slow refresh interval must be non-negative")
    assert_finite(fast_seconds, "fast refresh interval")
    assert(fast_seconds >= 0, "fast refresh interval must be non-negative")
    assert(
        slow_seconds >= fast_seconds,
        "slow refresh interval must not be shorter than fast refresh interval"
    )
    local rate = clamp_unit(scan_rate)
    return slow_seconds + (fast_seconds - slow_seconds) * rate
end

---@param candidates PresserCandidate[]
---@param current integer?
---@param switch_ratio number
---@return integer?
function brain.assign_presser(candidates, current, switch_ratio)
    local eligible = {}
    local seen = {}
    for index, candidate in ipairs(candidates) do
        assert_positive_index(candidate.player_index, "presser candidate " .. index .. " player")
        assert(not seen[candidate.player_index], "presser candidate player indices must be unique")
        seen[candidate.player_index] = true
        assert_finite(candidate.distance_cost, "presser candidate " .. index .. " distance cost")
        assert(candidate.distance_cost >= 0, "presser distance cost must be non-negative")
        if candidate.eligible ~= false then
            eligible[#eligible + 1] = {
                player_index = candidate.player_index,
                distance_cost = candidate.distance_cost,
            }
        end
    end
    table.sort(eligible, function(a, b)
        if a.distance_cost ~= b.distance_cost then
            return a.distance_cost < b.distance_cost
        end
        return a.player_index < b.player_index
    end)
    local best = eligible[1]
    if not best then
        return nil
    end

    local current_candidate = nil
    if current ~= nil then
        assert_positive_index(current, "current presser")
        for _, candidate in ipairs(eligible) do
            if candidate.player_index == current then
                current_candidate = candidate
                break
            end
        end
    end
    if not current_candidate or best.player_index == current_candidate.player_index then
        return best.player_index
    end

    local ratio = clamp_unit(switch_ratio)
    local is_real_improvement = best.distance_cost < current_candidate.distance_cost
    local clears_hysteresis = best.distance_cost <= current_candidate.distance_cost * (1 - ratio)
    if is_real_improvement and clears_hysteresis then
        return best.player_index
    end
    return current_candidate.player_index
end

---@param candidates RunCandidate[]
local function sort_run_candidates(candidates)
    table.sort(candidates, function(a, b)
        if a.score ~= b.score then
            return a.score > b.score
        end
        if a.player_index ~= b.player_index then
            return a.player_index < b.player_index
        end
        return RUN_TYPE_ORDER[a.run_type] < RUN_TYPE_ORDER[b.run_type]
    end)
end

---@param candidate RunCandidate
---@return RunCandidate
local function copy_run_candidate(candidate)
    return {
        player_index = candidate.player_index,
        run_type = candidate.run_type,
        score = candidate.score,
        target_x = candidate.target_x,
        target_y = candidate.target_y,
        duration = candidate.duration,
    }
end

---@param candidates RunCandidate[]
---@param player_index integer
---@param run_type RunType
---@param target BrainRunTarget
---@param label string
local function append_run_candidate(candidates, player_index, run_type, target, label)
    assert_finite(target.score, label .. " score")
    assert_finite(target.x, label .. " target x")
    assert_finite(target.y, label .. " target y")
    assert_finite(target.duration, label .. " duration")
    assert(target.duration > 0, label .. " duration must be positive")
    candidates[#candidates + 1] = {
        player_index = player_index,
        run_type = run_type,
        score = target.score,
        target_x = target.x,
        target_y = target.y,
        duration = target.duration,
    }
end

---@param context BrainRunContext
---@return RunCandidate[]
function brain.run_candidates(context)
    local candidates = {}
    local seen = {}
    for index, player in ipairs(context.players) do
        assert_positive_index(player.player_index, "run player " .. index)
        assert(not seen[player.player_index], "run player indices must be unique")
        seen[player.player_index] = true
        if player.eligible ~= false then
            if player.in_behind then
                append_run_candidate(
                    candidates,
                    player.player_index,
                    "in_behind",
                    player.in_behind,
                    "in-behind run"
                )
            end
            if player.come_short then
                append_run_candidate(
                    candidates,
                    player.player_index,
                    "come_short",
                    player.come_short,
                    "come-short run"
                )
            end
            if player.hold_width then
                append_run_candidate(
                    candidates,
                    player.player_index,
                    "hold_width",
                    player.hold_width,
                    "hold-width run"
                )
            end
        end
    end
    sort_run_candidates(candidates)
    return candidates
end

---@param slot RunSlot
---@param index integer
local function validate_run_slot(slot, index)
    local label = "active run slot " .. index
    assert_positive_index(slot.player_index, label .. " player")
    assert(RUN_TYPE_ORDER[slot.run_type] ~= nil, label .. " has an unknown run type")
    assert_finite(slot.score, label .. " score")
    assert_finite(slot.target_x, label .. " target x")
    assert_finite(slot.target_y, label .. " target y")
    assert_finite(slot.granted_at, label .. " grant time")
    assert_finite(slot.expires_at, label .. " expiry")
    assert(slot.expires_at >= slot.granted_at, label .. " expires before it was granted")
end

---@param slot RunSlot
---@return RunSlot
local function copy_run_slot(slot)
    return {
        player_index = slot.player_index,
        run_type = slot.run_type,
        score = slot.score,
        target_x = slot.target_x,
        target_y = slot.target_y,
        granted_at = slot.granted_at,
        expires_at = slot.expires_at,
    }
end

---@param candidates RunCandidate[]
---@param active RunSlot[]
---@param maximum integer
---@param now number
---@return RunSlot[]
function brain.grant_runs(candidates, active, maximum, now)
    assert_finite(maximum, "maximum run slots")
    assert(
        maximum == math.floor(maximum) and maximum >= 0,
        "maximum run slots must be a non-negative integer"
    )
    assert_finite(now, "run grant time")

    local kept = {}
    local assigned = {}
    for index, slot in ipairs(active) do
        validate_run_slot(slot, index)
        assert(not assigned[slot.player_index], "active run slot players must be unique")
        assigned[slot.player_index] = true
        if slot.expires_at > now then
            kept[#kept + 1] = copy_run_slot(slot)
        end
    end
    table.sort(kept, function(a, b)
        if a.granted_at ~= b.granted_at then
            return a.granted_at < b.granted_at
        end
        if a.player_index ~= b.player_index then
            return a.player_index < b.player_index
        end
        return RUN_TYPE_ORDER[a.run_type] < RUN_TYPE_ORDER[b.run_type]
    end)
    -- A lowered cap cannot preserve every prior slot. Earlier grants remain
    -- authoritative and later grants yield deterministically.
    while #kept > maximum do
        kept[#kept] = nil
    end

    assigned = {}
    for _, slot in ipairs(kept) do
        assigned[slot.player_index] = true
    end

    local ranked = {}
    for index, candidate in ipairs(candidates) do
        assert_positive_index(candidate.player_index, "run candidate " .. index .. " player")
        assert(RUN_TYPE_ORDER[candidate.run_type] ~= nil, "run candidate has unknown run type")
        assert_finite(candidate.score, "run candidate score")
        assert_finite(candidate.target_x, "run candidate target x")
        assert_finite(candidate.target_y, "run candidate target y")
        assert_finite(candidate.duration, "run candidate duration")
        assert(candidate.duration > 0, "run candidate duration must be positive")
        ranked[#ranked + 1] = copy_run_candidate(candidate)
    end
    sort_run_candidates(ranked)

    for _, candidate in ipairs(ranked) do
        if #kept >= maximum then
            break
        end
        if not assigned[candidate.player_index] then
            kept[#kept + 1] = {
                player_index = candidate.player_index,
                run_type = candidate.run_type,
                score = candidate.score,
                target_x = candidate.target_x,
                target_y = candidate.target_y,
                granted_at = now,
                expires_at = now + candidate.duration,
            }
            assigned[candidate.player_index] = true
        end
    end
    return kept
end

---@param context BrainPressContext
---@return PressMode
---@return PressReason
function brain.press_mode(context)
    if context.heavy_touch then
        return "commit", "heavy_touch"
    end
    if context.exposed_ball then
        return "commit", "exposed_ball"
    end
    if context.cover_available then
        return "commit", "cover"
    end
    if context.box_desperation then
        return "commit", "box_desperation"
    end
    local discipline = clamp_unit(context.press_discipline)
    local threshold = clamp_unit(context.low_discipline_threshold)
    if discipline < threshold then
        return "commit", "low_discipline"
    end
    return "contain", "no_trigger"
end

---@param a BrainScoredOption
---@param b BrainScoredOption
---@return boolean
local function option_before(a, b)
    if a.kind ~= b.kind then
        return a.kind < b.kind
    end
    return a.id < b.id
end

---@param options BrainScoredOption[]
---@return BrainScoredOption[]
local function canonical_options(options)
    local ordered = {}
    ---@type table<string, table<string, boolean>>
    local ids_by_kind = {}
    for index, option in ipairs(options) do
        assert(
            type(option.id) == "string" and option.id ~= "",
            "option " .. index .. " needs an id"
        )
        assert(
            type(option.kind) == "string" and option.kind ~= "",
            "option " .. index .. " needs a kind"
        )
        assert_finite(option.score, "option " .. index .. " score")
        local kind_ids = ids_by_kind[option.kind]
        if not kind_ids then
            kind_ids = {}
            ids_by_kind[option.kind] = kind_ids
        end
        assert(not kind_ids[option.id], "option kind and id pairs must be unique")
        kind_ids[option.id] = true
        ordered[#ordered + 1] = option
    end
    assert(#ordered > 0, "scored option selection needs at least one option")
    table.sort(ordered, option_before)
    return ordered
end

---@param rng_state integer
local function assert_rng_state(rng_state)
    assert_finite(rng_state, "option RNG state")
    assert(
        rng_state == math.floor(rng_state) and rng.seed(rng_state) == rng_state,
        "option RNG state must be a canonical positive integer"
    )
end

-- Seeded softmax selection over a canonical option order. At zero temperature,
-- exact argmax is deterministic and consumes no RNG.
---@param options BrainScoredOption[]
---@param temperature number
---@param rng_state integer
---@return BrainScoredOption selected
---@return integer next_rng_state
function brain.select_scored_option(options, temperature, rng_state)
    local ordered = canonical_options(options)
    assert_rng_state(rng_state)
    local effective_temperature = temperature
    if temperature ~= temperature or temperature < 0 then
        effective_temperature = 0
    end

    local maximum = ordered[1].score
    for index = 2, #ordered do
        maximum = math.max(maximum, ordered[index].score)
    end
    if effective_temperature == 0 then
        for _, option in ipairs(ordered) do
            if option.score == maximum then
                return option, rng_state
            end
        end
    end

    local weights = {}
    local total = 0
    for index, option in ipairs(ordered) do
        local weight = effective_temperature == math.huge and 1
            or math.exp((option.score - maximum) / effective_temperature)
        weights[index] = weight
        total = total + weight
    end
    local next_state, sample = rng.roll(rng_state)
    local target = sample * total
    local cumulative = 0
    for index, option in ipairs(ordered) do
        cumulative = cumulative + weights[index]
        if target < cumulative then
            return option, next_state
        end
    end
    return ordered[#ordered], next_state
end

-- Composure sharpens ordinary selection while pressure increases temperature
-- only in proportion to missing composure.
---@param options CarrierOption[]
---@param composure number
---@param pressure number
---@param base_temperature number
---@param rng_state integer
---@return CarrierOption selected
---@return integer next_rng_state
function brain.decide_carrier(options, composure, pressure, base_temperature, rng_state)
    local calm = clamp_unit(composure)
    local danger = clamp_unit(pressure)
    local base = base_temperature
    if base_temperature ~= base_temperature or base_temperature < 0 then
        base = 0
    end
    local temperature = calm == 1 and 0 or base * (1 - calm) * (1 + danger)
    return brain.select_scored_option(options, temperature, rng_state)
end

return brain
