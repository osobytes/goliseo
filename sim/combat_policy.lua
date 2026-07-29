-- `gameplay_ai/combat/v1`: the deterministic shipped gameplay combat policy.
--
-- It consumes only `combat_sim_observation/v1` and produces one scored choice on
-- the existing `BrainScoredOption` seam, so combat extends the milestone-9
-- decision contract instead of forming a second AI stack. It never calls a
-- resolver internal and never sets an outcome: an accepted decision becomes
-- three abstract equipment signals on an ordinary MatchInput, exactly like a
-- human producer.
--
-- Soccer value comes first. A candidate exists ONLY when one of the five
-- section 4.6 purpose predicates is true for the (source, target) pair AND
-- `family_commit_feasibility/v1` is true for the equipped family toward that
-- frozen identity. Purposeless off-ball harassment is therefore structurally
-- unreachable rather than merely discouraged, and every accepted commit is
-- inside `intervention_candidate/v2` by construction: the zero-tick witness
-- tape is one of the poses that envelope searches.
--
-- Exactly one stable reason leaves this module per decision: one of the five
-- purpose ids, or `decline` when the episode closes with no action. A commit
-- with no true predicate or an infeasible target is `unattributed_off_ball` and
-- additionally raises `representative_policy_context_violation`.

local brain = require("sim.brain")
local combat_feasibility = require("sim.combat_feasibility")
local combat_intent = require("sim.combat_intent")
local combat_observation = require("sim.combat_observation")
local fixed_clock = require("sim.fixed_clock")
local action_families = require("data.action_families")

---@alias CombatUnavailableReason
---|"no_loadout"
---|"soccer_commitment"
---|"aerial_or_recovery"
---|"forced"
---|"already_committed"
---|"cooldown"
---|"family_commit_feasibility"

---@class CombatPolicyCandidate
---@field target_player integer
---@field purpose CombatPurposeId
---@field family_id ActionFamilyId
---@field target_ball_distance number -- Target to ball, world px.
---@field source_ball_distance number -- Source to ball, world px.
---@field target_is_carrier boolean
---@field team_owns_ball boolean
---@field spills_ball boolean -- The equipped family's unguarded outcome spills the ball.
---@field teammate_coverage integer -- Same-team outfielders already contesting this target.
---@field formation_risk boolean
---@field commitment_ticks integer -- Windup + active + recovery + cooldown.
---@field guard_threat_ticks integer -- Ticks until the guarded threat lands; 0 when not guarding.

---@class CombatPolicyDecision
---@field action "commit"|"decline"|"unavailable"
---@field reason CombatDecisionReason
---@field family_id ActionFamilyId?
---@field target_player integer?
---@field formation_risk boolean
---@field unavailable_reason CombatUnavailableReason?
---@field context_violation boolean -- `representative_policy_context_violation`.
---@field rng_state integer
---@field digest string
---@field option_id string?

---@class CombatPolicyModule
local combat_policy = {}

combat_policy.POLICY_ID = "gameplay_ai/combat/v1"
-- The same identity rendered for the session manifest wire. `game.online.protocol`
-- restricts every manifest id to `[A-Za-z0-9][A-Za-z0-9_.-]*`, so the contract's
-- slashes become dots there. Widening the wire charset would be a protocol
-- change, and this policy does not need one to be named.
combat_policy.MANIFEST_POLICY_ID = "gameplay_ai.combat.v1"

-- Scoring constants. Every one of them is a soccer-value weight; none of them
-- reads a score line, a difficulty setting, or a hidden state.
combat_policy.DECLINE_BASELINE = 60
combat_policy.BALL_VALUE_RANGE = 220
combat_policy.BALL_VALUE_WEIGHT = 30
combat_policy.TURNOVER_BONUS = 16
combat_policy.COVERAGE_PENALTY = 13
combat_policy.COMMITMENT_WEIGHT = 26
combat_policy.FORMATION_RISK_PENALTY = 18
combat_policy.GUARD_URGENCY_TICKS = 24
combat_policy.GUARD_URGENCY_WEIGHT = 34
combat_policy.COVERAGE_RANGE_PX = 56
combat_policy.BASE_TEMPERATURE = 3
-- Composure at or above this takes the exact argmax and spends no RNG, mirroring
-- `outfield_decision.decide_carrier`'s authored boundary.
combat_policy.SHARP_COMPOSURE = 0.8
-- A telegraphed threat landing inside this window earns the existing sidestep
-- rather than a commit. The window matches the contract's 12-tick human-proxy
-- reaction budget, and the minimum lead keeps the AI honest: it reads a
-- telegraph it could plausibly have seen, and never snaps out of a swing that
-- is already landing. Without the floor, an active melee frame would be dodged
-- with zero reaction time and melee could never connect against the AI.
combat_policy.EVADE_WINDOW_TICKS = 12
combat_policy.EVADE_MIN_LEAD_TICKS = 4

---@type table<CombatPurposeId, number>
combat_policy.PURPOSE_BASE = {
    recovery_punish = 78,
    carrier_contest = 72,
    loose_ball_contest = 58,
    passing_lane_or_shot_denial = 46,
    carrier_protection = 44,
}

---@type table<CombatUnavailableReason, integer>
combat_policy.UNAVAILABLE_ORDER = {
    no_loadout = 1,
    soccer_commitment = 2,
    aerial_or_recovery = 3,
    forced = 4,
    already_committed = 5,
    cooldown = 6,
    family_commit_feasibility = 7,
}

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

---@param x number
---@param y number
---@return number
local function length(x, y)
    return math.sqrt(x * x + y * y)
end

---@param family_id ActionFamilyId
---@return integer
function combat_policy.commitment_ticks(family_id)
    local family = action_families[family_id]
    return family.windup_ticks
        + (family.active_ticks or 0)
        + family.recovery_ticks
        + family.cooldown_ticks
end

---@param candidate CombatPolicyCandidate
---@return string
function combat_policy.option_id(candidate)
    return candidate.purpose .. ":" .. tostring(candidate.target_player)
end

-- Pure scoring. Soccer value first: how close the contested body is to the ball,
-- whether winning it is a real turnover, whether a teammate already covers it,
-- what the commitment costs, and what it does to the formation.
---@param candidate CombatPolicyCandidate
---@return number
function combat_policy.score(candidate)
    local base = assert(
        combat_policy.PURPOSE_BASE[candidate.purpose],
        "unknown combat purpose: " .. tostring(candidate.purpose)
    )
    local score = base
    score = score
        + (1 - clamp_unit(candidate.target_ball_distance / combat_policy.BALL_VALUE_RANGE))
            * combat_policy.BALL_VALUE_WEIGHT
    if candidate.target_is_carrier and candidate.spills_ball then
        score = score + combat_policy.TURNOVER_BONUS
    end
    score = score - math.max(0, candidate.teammate_coverage) * combat_policy.COVERAGE_PENALTY
    score = score
        - (candidate.commitment_ticks / fixed_clock.TICK_RATE) * combat_policy.COMMITMENT_WEIGHT
    if candidate.formation_risk then
        score = score - combat_policy.FORMATION_RISK_PENALTY
    end
    if candidate.family_id == "guard" then
        local urgency = 1
            - clamp_unit(candidate.guard_threat_ticks / combat_policy.GUARD_URGENCY_TICKS)
        score = score + urgency * combat_policy.GUARD_URGENCY_WEIGHT
    end
    return score
end

-- Build the scored option set. The `decline` option is always present, so the
-- selector always has at least one option and a weak candidate loses to doing
-- nothing rather than being clamped away.
---@param candidates CombatPolicyCandidate[]
---@return BrainScoredOption[]
function combat_policy.options(candidates)
    local options = {
        {
            id = "decline",
            kind = "decline",
            score = combat_policy.DECLINE_BASELINE,
        },
    }
    for _, candidate in ipairs(candidates) do
        options[#options + 1] = {
            id = combat_policy.option_id(candidate),
            kind = candidate.family_id == "guard" and "guard" or "commit",
            score = combat_policy.score(candidate),
            payload = { target_player = candidate.target_player, purpose = candidate.purpose },
            reference = candidate.target_player,
        }
    end
    return options
end

---@param observation CombatObservation
---@return CombatUnavailableReason?
local function unavailable_reason(observation)
    local own = observation.self
    if own.is_keeper or own.family_id == "none" then
        return "no_loadout"
    end
    if own.forced_ticks > 0 then
        return "forced"
    end
    if own.phase ~= "ready" then
        return "already_committed"
    end
    if own.soccer_action == "aerial" then
        return "aerial_or_recovery"
    end
    if own.soccer_committed then
        return "soccer_commitment"
    end
    if own.cooldown_ticks > 0 then
        return "cooldown"
    end
    return nil
end

---@param observation CombatObservation
---@param target CombatObservationPeer
---@return integer
local function teammate_coverage(observation, target)
    local count = 0
    for _, mate in ipairs(observation.teammates) do
        if
            not mate.is_keeper
            and length(mate.x - target.x, mate.y - target.y) <= combat_policy.COVERAGE_RANGE_PX
        then
            count = count + 1
        end
    end
    return count
end

---@param observation CombatObservation
---@param source_index integer
---@return integer
local function guard_threat_ticks(observation, source_index)
    local best = math.huge
    for _, path in ipairs(combat_feasibility.hostile_paths(observation, source_index)) do
        best = math.min(best, path.start_tick)
    end
    if best == math.huge then
        return combat_policy.GUARD_URGENCY_TICKS
    end
    return best
end

-- Every loadout-aware candidate for this decision tick. A pair is a candidate
-- only when a purpose predicate holds and the EQUIPPED family can reach it from
-- the current pose; the zero-tick witness tape keeps the result inside
-- `intervention_candidate/v2` without re-running its search every tick.
---@param observation CombatObservation
---@return CombatPolicyCandidate[]
function combat_policy.candidates(observation)
    local own = observation.self
    local family_id = own.family_id
    if family_id == "none" then
        return {}
    end
    ---@cast family_id ActionFamilyId
    local family = action_families[family_id]
    local spills = family.unguarded_outcome ~= nil and family.unguarded_outcome.ball_spill == true
    local commitment = combat_policy.commitment_ticks(family_id)
    local formation_risk = combat_feasibility.formation_risk(observation, family_id)
    local candidates = {}
    for _, target in ipairs(observation.opponents) do
        if not target.is_keeper then
            local bitset, count =
                combat_feasibility.purpose_bitset(observation, target.player_index)
            if count > 0 then
                local witness = combat_feasibility.family_commit(
                    observation,
                    target.player_index,
                    family_id,
                    nil
                )
                if witness.feasible then
                    local purpose = assert(combat_feasibility.dominant_purpose(bitset))
                    candidates[#candidates + 1] = {
                        target_player = target.player_index,
                        purpose = purpose,
                        family_id = family_id,
                        target_ball_distance = length(
                            target.x - observation.ball.x,
                            target.y - observation.ball.y
                        ),
                        source_ball_distance = length(
                            own.x - observation.ball.x,
                            own.y - observation.ball.y
                        ),
                        target_is_carrier = observation.ball.owner_player_index
                            == target.player_index,
                        team_owns_ball = observation.ball.owner_team == own.team,
                        spills_ball = spills,
                        teammate_coverage = teammate_coverage(observation, target),
                        formation_risk = formation_risk,
                        commitment_ticks = commitment,
                        guard_threat_ticks = family_id == "guard"
                                and guard_threat_ticks(observation, target.player_index)
                            or 0,
                    }
                end
            end
        end
    end
    table.sort(candidates, function(left, right)
        return left.target_player < right.target_player
    end)
    return candidates
end

-- Resolve one decision. `temperature` is caller-owned: the gameplay caller
-- derives it from the deciding player's own composure, exactly as
-- `outfield_decision.decide_carrier` does, so a settled player takes the exact
-- argmax and consumes no RNG.
---@param observation CombatObservation
---@param rng_state integer
---@param temperature number?
---@return CombatPolicyDecision
function combat_policy.decide(observation, rng_state, temperature)
    -- The policy reads one schema and only that schema. The full allowlist pass
    -- lives in the specs; on the hot path the tag and version are what stop a
    -- foreign or privileged table from ever reaching the scoring code.
    assert(
        type(observation) == "table"
            and observation.schema == combat_observation.SCHEMA
            and observation.version == combat_observation.VERSION,
        "gameplay_ai/combat/v1 consumes only combat_sim_observation/v1"
    )
    local decision = {
        action = "decline",
        reason = "decline",
        family_id = nil,
        target_player = nil,
        formation_risk = false,
        unavailable_reason = nil,
        context_violation = false,
        rng_state = rng_state,
        digest = observation.digest,
        option_id = nil,
    }
    ---@cast decision CombatPolicyDecision

    local blocked = unavailable_reason(observation)
    if blocked then
        decision.action = "unavailable"
        decision.reason = "none"
        decision.unavailable_reason = blocked
        return decision
    end
    if observation.match.stoppage or observation.match.finished then
        decision.action = "unavailable"
        decision.reason = "none"
        decision.unavailable_reason = "soccer_commitment"
        return decision
    end

    local candidates = combat_policy.candidates(observation)
    if #candidates == 0 then
        decision.action = "unavailable"
        decision.reason = "none"
        decision.unavailable_reason = "family_commit_feasibility"
        return decision
    end

    local options = combat_policy.options(candidates)
    local selected, next_rng = brain.select_scored_option(options, temperature or 0, rng_state)
    decision.rng_state = next_rng
    decision.option_id = selected.id
    if selected.kind == "decline" then
        return decision
    end

    local target_player = assert(selected.reference)
    assert(type(target_player) == "number", "a combat commit needs a numeric target")
    ---@cast target_player integer
    local chosen ---@type CombatPolicyCandidate?
    for _, candidate in ipairs(candidates) do
        if combat_policy.option_id(candidate) == selected.id then
            chosen = candidate
            break
        end
    end
    chosen = assert(chosen, "the selected combat option left the candidate set")
    decision.action = "commit"
    decision.family_id = chosen.family_id
    decision.target_player = chosen.target_player
    decision.formation_risk = chosen.formation_risk
    decision.reason = chosen.purpose

    -- Defence in depth. A candidate cannot reach here without a true predicate
    -- and a feasible witness, so this branch is unreachable by construction; if
    -- it ever fires, the contract's closed outcome and the hard schema error
    -- are what must be reported, not a plausible-looking purpose.
    local bitset, count = combat_feasibility.purpose_bitset(observation, chosen.target_player)
    if count == 0 or not bitset[chosen.purpose] then
        decision.reason = "unattributed_off_ball"
        decision.context_violation = true
    end
    return decision
end

-- Guard is held: the policy owes a hold long enough to cover the threat it is
-- answering, then one release edge.
---@param observation CombatObservation
---@param decision CombatPolicyDecision
---@return integer
function combat_policy.hold_ticks(observation, decision)
    if decision.family_id ~= "guard" or decision.target_player == nil then
        return 0
    end
    local last = action_families.guard.windup_ticks
    for _, path in ipairs(combat_feasibility.hostile_paths(observation, decision.target_player)) do
        last = math.max(last, path.last_tick)
    end
    return last
end

---@param reason CombatDecisionReason
---@return boolean
function combat_policy.is_commit_reason(reason)
    return combat_intent.COMMIT_REASONS[reason] == true
end

return combat_policy
