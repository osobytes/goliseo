-- Retained, serializable combat intent for one AI-driven player.
--
-- The AI has no direct simulation action channel: it can only materialize the
-- same three abstract equipment signals a human produces. Ranged needs a press
-- tick followed by a release edge, and guard needs a held run followed by a
-- release, so the producer must remember where in that materialization it is.
-- That memory lives here as plain versioned scalars, so it survives strict
-- snapshot capture, canonical hashing, and rollback resimulation instead of
-- hiding in a producer closure.
--
-- It is deliberately TINY. Retained state is charged against the bounded
-- rollback snapshot budget (`data/omp2_rollback_validation.lua`) at ten players
-- times thirty-one boundaries, so anything derivable is derived instead of
-- stored: the decision cadence is a stateless function of the canonical tick and
-- the player's scan rate, and the policy's selection seed is a pure function of
-- the tick and the player index. Only what genuinely cannot be recomputed --
-- where a multi-tick materialization stands, and the stable reason and target it
-- serves -- is kept here.
--
-- There is no per-record version field: this record is a child of the versioned
-- `CombatMatchState` companion, exactly like a windup shot or a combat event,
-- and `combat_snapshot.VERSION` is what moves when this shape changes.
--
-- It requires only leaf modules: `sim.combat_snapshot` requires it, so it must
-- not reach back into the snapshot or the match.

local rng = require("core.rng")
local brain = require("sim.brain")
local fixed_clock = require("sim.fixed_clock")

---@alias CombatIntentStage "idle"|"release"|"hold"

---@alias CombatDecisionReason
---|"none"
---|"decline"
---|"carrier_contest"
---|"carrier_protection"
---|"loose_ball_contest"
---|"passing_lane_or_shot_denial"
---|"recovery_punish"
---|"unattributed_off_ball"

---@class CombatIntentState
---@field stage CombatIntentStage
---@field hold_ticks integer -- Guard hold ticks still owed before the release edge.
---@field reason CombatDecisionReason
---@field target_player integer?

---@class CombatIntentModule
local combat_intent = {}

combat_intent.VERSION = 1
combat_intent.SLOW_REFRESH_SECONDS = 0.45
combat_intent.FAST_REFRESH_SECONDS = 0.15

-- Canonical field order. `sim.combat_snapshot` uses it as the allowlist and as
-- the serialization order, so a new field cannot be added without moving the
-- combat snapshot version with it.
combat_intent.FIELDS = {
    "stage",
    "hold_ticks",
    "reason",
    "target_player",
}

-- Stride for the per-decision seed. A large prime keeps consecutive ticks and
-- adjacent player indices far apart in the stream.
combat_intent.DECISION_SEED_STRIDE = 15485863

---@type table<CombatIntentStage, boolean>
local STAGES = {
    idle = true,
    release = true,
    hold = true,
}

---@type table<CombatDecisionReason, boolean>
combat_intent.REASONS = {
    none = true,
    decline = true,
    carrier_contest = true,
    carrier_protection = true,
    loose_ball_contest = true,
    passing_lane_or_shot_denial = true,
    recovery_punish = true,
    unattributed_off_ball = true,
}

-- The five soccer-purpose ids. `decline` is valid only when an episode closes
-- with no action and can never label a commit; `unattributed_off_ball` is the
-- closed outcome for a zero-context or infeasible commit.
---@type table<CombatDecisionReason, boolean>
combat_intent.COMMIT_REASONS = {
    carrier_contest = true,
    carrier_protection = true,
    loose_ball_contest = true,
    passing_lane_or_shot_denial = true,
    recovery_punish = true,
    unattributed_off_ball = true,
}

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

---@return CombatIntentState
function combat_intent.new_state()
    return {
        stage = "idle",
        hold_ticks = 0,
        reason = "none",
        target_player = nil,
    }
end

---@param state CombatIntentState
local function validate_state(state)
    assert(type(state) == "table", "combat intent state must be a table")
    assert(STAGES[state.stage], "unknown combat intent stage")
    assert(
        is_finite(state.hold_ticks)
            and state.hold_ticks == math.floor(state.hold_ticks)
            and state.hold_ticks >= 0,
        "combat intent hold ticks must be a non-negative integer"
    )
    assert(combat_intent.REASONS[state.reason], "unknown combat intent reason")
    if state.target_player ~= nil then
        assert(
            is_finite(state.target_player)
                and state.target_player == math.floor(state.target_player)
                and state.target_player >= 1,
            "combat intent target must be a positive integer"
        )
    end
    if state.stage == "idle" then
        assert(state.hold_ticks == 0, "an idle combat intent cannot owe guard hold ticks")
    else
        assert(
            combat_intent.COMMIT_REASONS[state.reason],
            "a materializing combat intent requires a commit reason"
        )
        assert(state.target_player ~= nil, "a materializing combat intent requires a target")
    end
    if state.stage ~= "hold" then
        assert(state.hold_ticks == 0, "only a guard hold owes hold ticks")
    end
end

---@param state CombatIntentState
---@return CombatIntentState
function combat_intent.copy_state(state)
    validate_state(state)
    return {
        stage = state.stage,
        hold_ticks = state.hold_ticks,
        reason = state.reason,
        target_player = state.target_player,
    }
end

-- Drop any in-flight materialization. Used when a player becomes combat
-- ineligible (forced, keeper, aerial, match over) so a stale release edge can
-- never be emitted into a later, unrelated situation.
---@param state CombatIntentState
---@return CombatIntentState
function combat_intent.reset(state)
    combat_intent.copy_state(state)
    return combat_intent.new_state()
end

---@param state CombatIntentState
---@return CombatIntentState
function combat_intent.decline(state)
    local next_state = combat_intent.copy_state(state)
    next_state.stage = "idle"
    next_state.hold_ticks = 0
    next_state.reason = "decline"
    next_state.target_player = nil
    return combat_intent.copy_state(next_state)
end

-- Record an accepted commit and the materialization it still owes. `hold_ticks`
-- is guard-only; every other family owes exactly one release edge.
---@param state CombatIntentState
---@param reason CombatDecisionReason
---@param target_player integer
---@param hold_ticks integer
---@return CombatIntentState
function combat_intent.commit(state, reason, target_player, hold_ticks)
    local next_state = combat_intent.copy_state(state)
    assert(combat_intent.COMMIT_REASONS[reason], "a commit requires a commit reason")
    next_state.reason = reason
    next_state.target_player = target_player
    if hold_ticks > 0 then
        next_state.stage = "hold"
        next_state.hold_ticks = hold_ticks
    else
        next_state.stage = "release"
        next_state.hold_ticks = 0
    end
    return combat_intent.copy_state(next_state)
end

-- The personal decision cadence, derived rather than retained: a stat-scaled
-- interval plus a stable per-player phase offset. Two players with the same scan
-- rate still decide on different ticks, and nobody re-decides every tick, which
-- is what stops targets and actions oscillating.
---@param scan_rate number
---@return integer ticks
function combat_intent.decision_period(scan_rate)
    local seconds = brain.refresh_interval(
        scan_rate,
        combat_intent.SLOW_REFRESH_SECONDS,
        combat_intent.FAST_REFRESH_SECONDS
    )
    return math.max(1, math.floor(seconds * fixed_clock.TICK_RATE + 0.5))
end

---@param tick integer
---@param player_index integer
---@param scan_rate number
---@return boolean
function combat_intent.should_decide(tick, player_index, scan_rate)
    return (tick + player_index) % combat_intent.decision_period(scan_rate) == 0
end

-- A dedicated canonical Park-Miller seed for one decision. It never touches the
-- match RNG and never persists: the same (tick, player) always yields the same
-- stream, which is exactly what a rollback resimulation of that boundary needs.
---@param tick integer
---@param player_index integer
---@return integer
function combat_intent.decision_seed(tick, player_index)
    return rng.seed(tick * combat_intent.DECISION_SEED_STRIDE + player_index)
end

---@class CombatIntentSignals
---@field equipment_held boolean
---@field equipment_pressed boolean
---@field equipment_released boolean

-- Advance one owed materialization tick. Returns the abstract signals to place
-- on this tick's MatchInput and the next retained state.
--
-- Every triple this returns satisfies `input_frame.validate_sample`'s legality
-- rule: a press always carries `held`, and a release never carries `held`.
---@param state CombatIntentState
---@return CombatIntentSignals?
---@return CombatIntentState
function combat_intent.materialize(state)
    local next_state = combat_intent.copy_state(state)
    if next_state.stage == "release" then
        next_state.stage = "idle"
        next_state.hold_ticks = 0
        return {
            equipment_held = false,
            equipment_pressed = false,
            equipment_released = true,
        },
            combat_intent.copy_state(next_state)
    end
    if next_state.stage == "hold" then
        if next_state.hold_ticks > 0 then
            next_state.hold_ticks = next_state.hold_ticks - 1
            if next_state.hold_ticks == 0 then
                next_state.stage = "release"
            end
            return {
                equipment_held = true,
                equipment_pressed = false,
                equipment_released = false,
            },
                combat_intent.copy_state(next_state)
        end
        next_state.stage = "idle"
        return {
            equipment_held = false,
            equipment_pressed = false,
            equipment_released = true,
        },
            combat_intent.copy_state(next_state)
    end
    return nil, combat_intent.copy_state(next_state)
end

-- The signals a fresh commit places on the tick it is decided.
---@return CombatIntentSignals
function combat_intent.commit_signals()
    return {
        equipment_held = true,
        equipment_pressed = true,
        equipment_released = false,
    }
end

return combat_intent
