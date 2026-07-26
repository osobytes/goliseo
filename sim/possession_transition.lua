-- Serializable possession-transition state and the pure helpers that turn it
-- into per-team phases. Match owns the fixed clock, world eligibility, and the
-- resulting steering; this module owns the compact versioned value contract,
-- the one-turnover-per-flip rule, and the deterministic transition rankings.
--
-- A transition starts exactly once when authoritative possession moves from one
-- team to the other. The intervening loose-ball interval keeps `last_team`, so a
-- spill neither declares a new possession team nor restarts the window.
--
-- "Authoritative" means ESTABLISHED, not merely touched. Raw ownership flickers
-- roughly thirty times a minute in a poke-and-scramble, which would refresh a
-- multi-second window forever and erase the ordinary attack/defend phases
-- entirely. A team therefore has to HOLD the ball for ESTABLISH_SECONDS before
-- it counts: loose frames pause that streak (pass flights bridge) and only the
-- other team's touch resets it. sim.metrics measures settled possession the same
-- way; this module is the simulation-side owner of the rule.

local brain = require("sim.brain")

---@alias TransitionTeam "home"|"away"

---@class PossessionTransitionState
---@field version integer
---@field last_team TransitionTeam?  -- last team to ESTABLISH possession
---@field holding_team TransitionTeam?  -- team currently accruing a possession hold
---@field hold number  -- seconds holding_team has held, capped at ESTABLISH_SECONDS
---@field turnover_team TransitionTeam?  -- team that won the live turnover (nil = none)
---@field elapsed number  -- seconds since that turnover

---@class TransitionPresserCandidate
---@field player_index integer
---@field distance_cost number
---@field eligible boolean?

---@class PossessionTransitionModule
local possession_transition = {}

possession_transition.VERSION = 1

-- Seconds a team must hold the ball before its possession is authoritative.
-- Deliberately the same threshold sim.metrics uses for a settled possession
-- change, so one counted turnover is exactly one transition window (a spec locks
-- the two together). Raw ownership flips are ~30/min here; settled ones are ~3.
possession_transition.ESTABLISH_SECONDS = 0.7

-- Counter-press commits two hunters instead of possession defense's single
-- primary presser. Everyone else holds their turnover position.
possession_transition.MAX_PRESSERS = 2

-- Counter-attack support push per formation role, as a multiplier on the
-- tactic's ordinary attacking support depth. Forwards get the strongest
-- possession push; defenders get none so they recover first.
local SUPPORT_PUSH = {
    fwd = 1.5,
    wide = 1.3,
    mid = 1.2,
    def = 1.0,
}

local TEAMS = {
    home = true,
    away = true,
}

---@param value number
---@return boolean
local function is_finite(value)
    return value == value and value ~= math.huge and value ~= -math.huge
end

---@param state PossessionTransitionState
local function validate_state(state)
    assert(type(state) == "table", "possession transition state must be a table")
    assert(
        state.version == possession_transition.VERSION,
        "unsupported possession transition version"
    )
    assert(
        state.last_team == nil or TEAMS[state.last_team],
        "possession transition last team must be home, away, or nil"
    )
    assert(
        state.turnover_team == nil or TEAMS[state.turnover_team],
        "possession transition turnover team must be home, away, or nil"
    )
    assert(
        state.holding_team == nil or TEAMS[state.holding_team],
        "possession transition holding team must be home, away, or nil"
    )
    assert(
        type(state.hold) == "number" and is_finite(state.hold) and state.hold >= 0,
        "possession transition hold must be a non-negative finite number"
    )
    assert(
        state.holding_team ~= nil or state.hold == 0,
        "an unheld possession transition cannot retain hold time"
    )
    assert(
        state.hold <= possession_transition.ESTABLISH_SECONDS,
        "possession hold must saturate at the establish threshold"
    )
    assert(
        type(state.elapsed) == "number" and is_finite(state.elapsed) and state.elapsed >= 0,
        "possession transition elapsed must be a non-negative finite number"
    )
    assert(
        state.last_team == nil or state.holding_team ~= nil,
        "an established possession transition must retain its holder"
    )
    if state.turnover_team == nil then
        assert(state.elapsed == 0, "an idle possession transition cannot retain elapsed time")
    else
        assert(
            state.last_team == state.turnover_team,
            "a live possession transition must be owned by the last possession team"
        )
    end
end

---@return PossessionTransitionState
function possession_transition.new_state()
    return {
        version = possession_transition.VERSION,
        last_team = nil,
        holding_team = nil,
        hold = 0,
        turnover_team = nil,
        elapsed = 0,
    }
end

---@param state PossessionTransitionState
---@return PossessionTransitionState
function possession_transition.copy_state(state)
    validate_state(state)
    return {
        version = state.version,
        last_team = state.last_team,
        holding_team = state.holding_team,
        hold = state.hold,
        turnover_team = state.turnover_team,
        elapsed = state.elapsed,
    }
end

-- Forget every possession memory. Lifecycle boundaries (construction, kickoff
-- placement, full time) use this so a restart can never manufacture a turnover
-- out of the possession that preceded it.
---@param state PossessionTransitionState
---@return PossessionTransitionState state  -- same table, mutated
function possession_transition.clear(state)
    validate_state(state)
    state.last_team = nil
    state.holding_team = nil
    state.hold = 0
    state.turnover_team = nil
    state.elapsed = 0
    return state
end

---@param state PossessionTransitionState
---@return boolean
function possession_transition.active(state)
    validate_state(state)
    return state.turnover_team ~= nil
end

-- Seconds after which no team can still be inside a window, so the live
-- transition may be dropped. The loser's counter-press and the winner's
-- counter-attack are the only two windows a single turnover opens.
---@param state PossessionTransitionState
---@param windows table<TransitionTeam, TransitionConfig>
---@return number
function possession_transition.window_limit(state, windows)
    validate_state(state)
    local winner = state.turnover_team
    if winner == nil then
        return 0
    end
    local loser = winner == "home" and "away" or "home"
    return math.max(windows[winner].counterattack, windows[loser].counterpress)
end

-- Advance the live transition by one fixed tick and retire it once both windows
-- have decayed, so `elapsed` stays bounded and canonical.
---@param state PossessionTransitionState
---@param dt number
---@param windows table<TransitionTeam, TransitionConfig>
---@return PossessionTransitionState state  -- same table, mutated
function possession_transition.advance(state, dt, windows)
    validate_state(state)
    assert(type(dt) == "number" and is_finite(dt) and dt >= 0, "transition delta must be >= 0")
    if state.turnover_team == nil then
        return state
    end
    local limit = possession_transition.window_limit(state, windows)
    local elapsed = state.elapsed + dt
    if elapsed >= limit then
        state.turnover_team = nil
        state.elapsed = 0
        return state
    end
    state.elapsed = elapsed
    return state
end

-- Fold one tick of ownership into the possession memory. A loose ball
-- (`owner_team == nil`) is not a possession change: it pauses the hold streak and
-- keeps the prior team, so a spill can neither declare a new possession team nor
-- restart a live window. A turnover opens only when the ball's new holder has
-- held it for ESTABLISH_SECONDS, so scramble flicker cannot refresh the window.
---@param state PossessionTransitionState
---@param owner_team TransitionTeam?
---@param dt number
---@return PossessionTransitionState state  -- same table, mutated
function possession_transition.observe(state, owner_team, dt)
    validate_state(state)
    assert(
        owner_team == nil or TEAMS[owner_team],
        "observed possession team must be home, away, or nil"
    )
    assert(type(dt) == "number" and is_finite(dt) and dt >= 0, "observed delta must be >= 0")
    if owner_team == nil then
        return state
    end
    if owner_team ~= state.holding_team then
        state.holding_team = owner_team
        state.hold = 0
    end
    -- Saturate at the threshold: past it the streak carries no information, and a
    -- pinned value keeps the serialized state bounded instead of drifting upward
    -- for the rest of the match.
    state.hold = math.min(state.hold + dt, possession_transition.ESTABLISH_SECONDS)
    if state.hold < possession_transition.ESTABLISH_SECONDS or owner_team == state.last_team then
        return state
    end
    if state.last_team ~= nil then
        -- Possession has authoritatively changed hands: exactly one transition.
        state.turnover_team = owner_team
        state.elapsed = 0
    end
    -- First established possession after a lifecycle reset (a kickoff, a
    -- restart, a fresh match) is remembered, never a turnover.
    state.last_team = owner_team
    return state
end

-- Decay this team's transition memory into a phase. Ordinary attack, defend,
-- and loose phases are unchanged outside a live window.
---@param state PossessionTransitionState
---@param team TransitionTeam
---@param owner_team TransitionTeam?
---@param windows TransitionConfig
---@return TeamPhase
function possession_transition.phase(state, team, owner_team, windows)
    validate_state(state)
    assert(TEAMS[team], "transition phase needs a real team")
    local possession = "loose"
    if owner_team == team then
        possession = "team"
    elseif owner_team ~= nil then
        possession = "opponent"
    end
    local transition = nil
    if state.turnover_team ~= nil then
        transition = state.turnover_team == team and "won" or "lost"
    end
    return brain.phase({
        possession = possession,
        transition = transition,
        transition_elapsed = state.elapsed,
        counterpress_window = windows.counterpress,
        counterattack_window = windows.counterattack,
    })
end

---@param source TransitionConfig
---@return TransitionConfig
function possession_transition.copy_windows(source)
    assert(type(source) == "table", "transition windows must be a table")
    assert(
        type(source.counterpress) == "number"
            and is_finite(source.counterpress)
            and source.counterpress >= 0,
        "counterpress window must be a non-negative finite number"
    )
    assert(
        type(source.counterattack) == "number"
            and is_finite(source.counterattack)
            and source.counterattack >= 0,
        "counterattack window must be a non-negative finite number"
    )
    return { counterpress = source.counterpress, counterattack = source.counterattack }
end

-- The nearest eligible counter-pressers, capped at `limit`. Distance leads and
-- the player index breaks ties, so mirrored fixtures produce mirrored hunters.
---@param candidates TransitionPresserCandidate[]
---@param limit integer
---@return integer[]
function possession_transition.select_pressers(candidates, limit)
    assert(
        type(limit) == "number" and limit == math.floor(limit) and limit >= 0,
        "counter-press limit must be a non-negative integer"
    )
    local eligible = {}
    local seen = {}
    for index, candidate in ipairs(candidates) do
        local label = "counter-press candidate " .. index
        assert(
            type(candidate.player_index) == "number"
                and candidate.player_index == math.floor(candidate.player_index)
                and candidate.player_index >= 1,
            label .. " player must be a positive integer"
        )
        assert(not seen[candidate.player_index], "counter-press candidates must be unique")
        seen[candidate.player_index] = true
        assert(
            is_finite(candidate.distance_cost) and candidate.distance_cost >= 0,
            label .. " distance cost must be non-negative"
        )
        if candidate.eligible ~= false then
            eligible[#eligible + 1] = candidate
        end
    end
    table.sort(eligible, function(a, b)
        if a.distance_cost ~= b.distance_cost then
            return a.distance_cost < b.distance_cost
        end
        return a.player_index < b.player_index
    end)
    local chosen = {}
    for index = 1, math.min(limit, #eligible) do
        chosen[index] = eligible[index].player_index
    end
    return chosen
end

-- Role-shaped counter-attack support depth. Unknown roles keep the ordinary
-- push rather than inventing one.
---@param role FormationRole
---@return number
function possession_transition.support_push(role)
    return SUPPORT_PUSH[role] or 1
end

return possession_transition
