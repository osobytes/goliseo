-- Named learning-environment reward channels.
--
-- There is no implicit scalar reward. Every channel is registered with an
-- explicit role: `objective` channels are competition targets, `shaping`
-- channels must be preregistered and are always reported separately so an
-- ablation is a subtraction rather than a rerun, and `diagnostic` channels are
-- evaluation data that may never be optimized. The #128 fun-proxy metric family
-- is a diagnostic channel: selecting it as an objective or shaping term is a
-- validation error, and no channel may be named `fun`.

---@alias EnvRewardRole "objective"|"shaping"|"diagnostic"

---@alias EnvRewardChannelId
---| "match_outcome"
---| "goal_scored"
---| "goal_conceded"
---| "goal_difference_delta"
---| "possession_gain"
---| "shot_attempt"
---| "equipment_contact"
---| "experience_proxy_metrics"

---@alias EnvRewardErrorCode "malformed"|"unknown_channel"|"wrong_role"|"duplicate_channel"

---@class EnvRewardChannel
---@field id EnvRewardChannelId
---@field role EnvRewardRole
---@field optimizable boolean -- False for diagnostics: never a training target.
---@field preregistered boolean -- Shaping terms are declared here, not invented per run.
---@field description string

---@class EnvRewardEvent
---@field kind string -- MatchEventKind or CombatEventKind.
---@field team InputTeam? -- Acting side, when the event has an identified actor.
---@field result CombatContactResult? -- Confirmed combat contact verdict, when applicable.

---@class EnvRewardTransition
---@field team InputTeam -- Perspective the reward is expressed from.
---@field score_before { home: integer, away: integer }
---@field score_after { home: integer, away: integer }
---@field owner_team_before InputTeam?
---@field owner_team_after InputTeam?
---@field events EnvRewardEvent[] -- Confirmed events of the simulated ticks.
---@field terminated boolean -- True only on the tick the match itself ended.

---@class EnvRewardSelection
---@field objectives EnvRewardChannelId[]
---@field shaping EnvRewardChannelId[]

---@class EnvRewardResult
---@field version integer
---@field team InputTeam
---@field objectives table<EnvRewardChannelId, number>
---@field shaping table<EnvRewardChannelId, number>
---@field objective_total number
---@field shaping_total number
---@field total number -- objective_total + shaping_total, for callers that want one scalar.

---@class EnvRewardModule
local env_reward = {}

env_reward.VERSION = 1

-- The banned reward name from #138: predicted human experience is evaluation
-- evidence, never an optimization target.
env_reward.FORBIDDEN_CHANNEL_ID = "fun"

env_reward.DIAGNOSTIC_METRIC_CHANNEL = "experience_proxy_metrics"

local SHOT_EVENT_KINDS = {
    shot = true,
    header = true,
    volley = true,
    bicycle = true,
}

---@type EnvRewardChannel[]
env_reward.CHANNELS = {
    {
        id = "match_outcome",
        role = "objective",
        optimizable = true,
        preregistered = true,
        description = "Sparse +1 win / -1 loss / 0 draw, paid once when the match ends.",
    },
    {
        id = "goal_scored",
        role = "objective",
        optimizable = true,
        preregistered = true,
        description = "+1 per goal scored by the reward team during the step.",
    },
    {
        id = "goal_conceded",
        role = "objective",
        optimizable = true,
        preregistered = true,
        description = "-1 per goal conceded by the reward team during the step.",
    },
    {
        id = "goal_difference_delta",
        role = "objective",
        optimizable = true,
        preregistered = true,
        description = "Change in goal difference from the reward team's perspective.",
    },
    {
        id = "possession_gain",
        role = "shaping",
        optimizable = true,
        preregistered = true,
        description = "+1 when ball ownership transitions to the reward team, -1 when it is lost.",
    },
    {
        id = "shot_attempt",
        role = "shaping",
        optimizable = true,
        preregistered = true,
        description = "+1 per confirmed shot, header, volley, or bicycle strike by the reward team.",
    },
    {
        id = "equipment_contact",
        role = "shaping",
        optimizable = true,
        preregistered = true,
        description = "+1 per unguarded combat contact landed by the reward team.",
    },
    {
        id = "experience_proxy_metrics",
        role = "diagnostic",
        optimizable = false,
        preregistered = true,
        description = "The #128 fun-proxy metric family, returned as evaluation data only.",
    },
}

---@type table<EnvRewardChannelId, EnvRewardChannel>
env_reward.CHANNELS_BY_ID = {}
for _, channel in ipairs(env_reward.CHANNELS) do
    assert(
        channel.id ~= env_reward.FORBIDDEN_CHANNEL_ID,
        "no reward channel may be named " .. env_reward.FORBIDDEN_CHANNEL_ID
    )
    assert(not env_reward.CHANNELS_BY_ID[channel.id], "duplicate reward channel id")
    assert(
        channel.role ~= "diagnostic" or not channel.optimizable,
        "diagnostic reward channels must not be optimizable"
    )
    env_reward.CHANNELS_BY_ID[channel.id] = channel
end

---@type EnvRewardChannelId[]
env_reward.DEFAULT_OBJECTIVES = { "match_outcome" }

---@param code EnvRewardErrorCode
---@param message string
---@return nil, string, EnvRewardErrorCode
local function reject(code, message)
    return nil, message, code
end

---@param value any
---@return boolean
local function is_side(value)
    return value == "home" or value == "away"
end

-- Validate an authored channel selection for one role. Selections come from a
-- run config (external input), so failures are recoverable returns.
---@param ids any
---@param role EnvRewardRole
---@return EnvRewardChannelId[]?, string?, EnvRewardErrorCode?
function env_reward.validate_selection(ids, role)
    assert(role == "objective" or role == "shaping", "unknown reward role: " .. tostring(role))
    if type(ids) ~= "table" then
        return reject("malformed", role .. " channel selection must be an array")
    end
    local count = #ids
    for key in pairs(ids) do
        if type(key) ~= "number" or key ~= math.floor(key) or key < 1 or key > count then
            return reject("malformed", role .. " channel selection is not a canonical array")
        end
    end
    local seen = {}
    local copied = {}
    for index = 1, count do
        local id = ids[index]
        if type(id) ~= "string" then
            return reject("malformed", role .. " channel id must be a string")
        end
        local channel = env_reward.CHANNELS_BY_ID[id]
        if not channel then
            return reject("unknown_channel", "unknown reward channel: " .. id)
        end
        if channel.role ~= role then
            return reject(
                "wrong_role",
                "reward channel " .. id .. " is a " .. channel.role .. " channel, not " .. role
            )
        end
        if seen[id] then
            return reject("duplicate_channel", "reward channel " .. id .. " is selected twice")
        end
        seen[id] = true
        copied[index] = id
    end
    return copied
end

---@param selection any
---@return EnvRewardSelection?, string?, EnvRewardErrorCode?
function env_reward.validate(selection)
    if type(selection) ~= "table" then
        return reject("malformed", "reward selection must be a table")
    end
    for key in pairs(selection) do
        if key ~= "objectives" and key ~= "shaping" then
            return reject("malformed", "reward selection has unknown field " .. tostring(key))
        end
    end
    local objectives, objectives_err, objectives_code =
        env_reward.validate_selection(selection.objectives or {}, "objective")
    if not objectives then
        return nil, objectives_err, objectives_code
    end
    local shaping, shaping_err, shaping_code =
        env_reward.validate_selection(selection.shaping or {}, "shaping")
    if not shaping then
        return nil, shaping_err, shaping_code
    end
    return { objectives = objectives, shaping = shaping }
end

---@param transition EnvRewardTransition
---@return integer own_goals
---@return integer opponent_goals
local function goal_deltas(transition)
    local before, after = transition.score_before, transition.score_after
    local own_before = transition.team == "home" and before.home or before.away
    local own_after = transition.team == "home" and after.home or after.away
    local opponent_before = transition.team == "home" and before.away or before.home
    local opponent_after = transition.team == "home" and after.away or after.home
    return own_after - own_before, opponent_after - opponent_before
end

---@param transition EnvRewardTransition
---@return number
local function match_outcome(transition)
    if not transition.terminated then
        return 0
    end
    local after = transition.score_after
    local own = transition.team == "home" and after.home or after.away
    local opponent = transition.team == "home" and after.away or after.home
    if own > opponent then
        return 1
    end
    if own < opponent then
        return -1
    end
    return 0
end

---@param transition EnvRewardTransition
---@return number
local function possession_gain(transition)
    local before = transition.owner_team_before == transition.team
    local after = transition.owner_team_after == transition.team
    if after and not before then
        return 1
    end
    if before and not after then
        return -1
    end
    return 0
end

---@param transition EnvRewardTransition
---@return number
local function shot_attempt(transition)
    local total = 0
    for _, event in ipairs(transition.events) do
        if SHOT_EVENT_KINDS[event.kind] and event.team == transition.team then
            total = total + 1
        end
    end
    return total
end

---@param transition EnvRewardTransition
---@return number
local function equipment_contact(transition)
    local total = 0
    for _, event in ipairs(transition.events) do
        if event.kind == "contact" and event.team == transition.team and event.result == "hit" then
            total = total + 1
        end
    end
    return total
end

---@type table<EnvRewardChannelId, fun(transition: EnvRewardTransition): number>
local EVALUATORS = {
    match_outcome = match_outcome,
    goal_scored = function(transition)
        local own = goal_deltas(transition)
        return own
    end,
    goal_conceded = function(transition)
        local _, opponent = goal_deltas(transition)
        return -opponent
    end,
    goal_difference_delta = function(transition)
        local own, opponent = goal_deltas(transition)
        return own - opponent
    end,
    possession_gain = possession_gain,
    shot_attempt = shot_attempt,
    equipment_contact = equipment_contact,
}

---@param transition EnvRewardTransition
local function assert_transition(transition)
    assert(type(transition) == "table", "reward transition is required")
    assert(is_side(transition.team), "reward transition needs a home/away perspective")
    assert(type(transition.score_before) == "table", "reward transition needs score_before")
    assert(type(transition.score_after) == "table", "reward transition needs score_after")
    assert(type(transition.events) == "table", "reward transition needs an event array")
    assert(type(transition.terminated) == "boolean", "reward transition needs a terminated flag")
    assert(
        transition.owner_team_before == nil or is_side(transition.owner_team_before),
        "reward transition owner_team_before must be a side or nil"
    )
    assert(
        transition.owner_team_after == nil or is_side(transition.owner_team_after),
        "reward transition owner_team_after must be a side or nil"
    )
end

-- Score one already-simulated transition. Objectives and shaping are summed
-- separately so an ablation report can drop the shaping column without
-- re-running the episode.
---@param transition EnvRewardTransition
---@param selection EnvRewardSelection
---@return EnvRewardResult
function env_reward.evaluate(transition, selection)
    assert_transition(transition)
    assert(type(selection) == "table", "reward selection is required")
    local objectives = {}
    local shaping = {}
    local objective_total = 0
    local shaping_total = 0
    for _, id in ipairs(selection.objectives) do
        local channel = assert(env_reward.CHANNELS_BY_ID[id], "unknown objective channel: " .. id)
        assert(channel.role == "objective", "channel " .. id .. " is not an objective")
        local value =
            assert(EVALUATORS[id], "objective channel has no evaluator: " .. id)(transition)
        objectives[id] = value
        objective_total = objective_total + value
    end
    for _, id in ipairs(selection.shaping) do
        local channel = assert(env_reward.CHANNELS_BY_ID[id], "unknown shaping channel: " .. id)
        assert(channel.role == "shaping", "channel " .. id .. " is not shaping")
        local value = assert(EVALUATORS[id], "shaping channel has no evaluator: " .. id)(transition)
        shaping[id] = value
        shaping_total = shaping_total + value
    end
    return {
        version = env_reward.VERSION,
        team = transition.team,
        objectives = objectives,
        shaping = shaping,
        objective_total = objective_total,
        shaping_total = shaping_total,
        total = objective_total + shaping_total,
    }
end

return env_reward
