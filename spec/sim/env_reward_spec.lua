local t = require("spec.support.runner")
local env_reward = require("sim.env_reward")

---@param overrides table<string, any>?
---@return EnvRewardTransition
local function transition(overrides)
    local base = {
        team = "home",
        score_before = { home = 0, away = 0 },
        score_after = { home = 0, away = 0 },
        owner_team_before = nil,
        owner_team_after = nil,
        events = {},
        terminated = false,
    }
    for key, value in pairs(overrides or {}) do
        base[key] = value
    end
    return base
end

---@param objectives EnvRewardChannelId[]
---@param shaping EnvRewardChannelId[]?
---@return EnvRewardSelection
local function selection(objectives, shaping)
    return { objectives = objectives, shaping = shaping or {} }
end

t.describe("env_reward registry", function()
    t.it("has no channel named fun and no optimizable diagnostic", function()
        for _, channel in ipairs(env_reward.CHANNELS) do
            t.is_true(channel.id ~= "fun", "no reward channel may be named fun")
            t.is_true(channel.id:find("fun") == nil, "no channel id may mention fun")
            if channel.role == "diagnostic" then
                t.eq(channel.optimizable, false)
            end
            t.is_true(channel.description ~= "", "every channel documents itself")
        end
        t.eq(env_reward.CHANNELS_BY_ID.experience_proxy_metrics.role, "diagnostic")
        t.eq(env_reward.DIAGNOSTIC_METRIC_CHANNEL, "experience_proxy_metrics")
    end)

    t.it("defaults to the sparse match outcome only", function()
        t.eq(#env_reward.DEFAULT_OBJECTIVES, 1)
        t.eq(env_reward.DEFAULT_OBJECTIVES[1], "match_outcome")
    end)
end)

t.describe("env_reward.validate_selection", function()
    t.it("accepts channels of the requested role", function()
        local objectives = assert(env_reward.validate_selection({ "goal_scored" }, "objective"))
        t.eq(objectives[1], "goal_scored")
        t.eq(#assert(env_reward.validate_selection({}, "shaping")), 0)
    end)

    t.it("rejects unknown, misplaced, duplicated, and malformed selections", function()
        for _, case in ipairs({
            { ids = { "score_more" }, role = "objective", code = "unknown_channel" },
            { ids = { "possession_gain" }, role = "objective", code = "wrong_role" },
            { ids = { "match_outcome" }, role = "shaping", code = "wrong_role" },
            {
                ids = { "experience_proxy_metrics" },
                role = "objective",
                code = "wrong_role",
            },
            {
                ids = { "goal_scored", "goal_scored" },
                role = "objective",
                code = "duplicate_channel",
            },
            { ids = { 7 }, role = "objective", code = "malformed" },
            { ids = "goal_scored", role = "objective", code = "malformed" },
        }) do
            local ids, err, code = env_reward.validate_selection(case.ids, case.role)
            t.eq(ids, nil)
            t.eq(code, case.code)
            t.is_true(type(err) == "string" and err ~= "")
        end
    end)

    t.it("rejects a selection table with unknown fields", function()
        local bad, _, code = env_reward.validate({ objectives = {}, rewards = {} })
        t.eq(bad, nil)
        t.eq(code, "malformed")
    end)
end)

t.describe("env_reward.evaluate", function()
    t.it("pays the sparse outcome only when the match ends", function()
        local running = env_reward.evaluate(
            transition({ score_after = { home = 2, away = 0 } }),
            selection({ "match_outcome" })
        )
        t.eq(running.objectives.match_outcome, 0)

        local won = env_reward.evaluate(
            transition({ score_after = { home = 2, away = 0 }, terminated = true }),
            selection({ "match_outcome" })
        )
        t.eq(won.objectives.match_outcome, 1)

        local lost = env_reward.evaluate(
            transition({ score_after = { home = 0, away = 2 }, terminated = true }),
            selection({ "match_outcome" })
        )
        t.eq(lost.objectives.match_outcome, -1)

        local drawn = env_reward.evaluate(
            transition({ score_after = { home = 1, away = 1 }, terminated = true }),
            selection({ "match_outcome" })
        )
        t.eq(drawn.objectives.match_outcome, 0)
    end)

    t.it("expresses goal channels from the configured perspective", function()
        local moved = transition({
            score_before = { home = 1, away = 1 },
            score_after = { home = 2, away = 1 },
        })
        local home = env_reward.evaluate(
            moved,
            selection({ "goal_scored", "goal_conceded", "goal_difference_delta" })
        )
        t.eq(home.objectives.goal_scored, 1)
        t.eq(home.objectives.goal_conceded, 0)
        t.eq(home.objectives.goal_difference_delta, 1)

        local away_view = transition({
            team = "away",
            score_before = { home = 1, away = 1 },
            score_after = { home = 2, away = 1 },
        })
        local away = env_reward.evaluate(
            away_view,
            selection({ "goal_scored", "goal_conceded", "goal_difference_delta" })
        )
        t.eq(away.objectives.goal_scored, 0)
        t.eq(away.objectives.goal_conceded, -1)
        t.eq(away.objectives.goal_difference_delta, -1)
        t.eq(away.team, "away")
    end)

    t.it("scores shaping channels from confirmed events and ownership", function()
        local gained = env_reward.evaluate(
            transition({ owner_team_before = "away", owner_team_after = "home" }),
            selection({}, { "possession_gain" })
        )
        t.eq(gained.shaping.possession_gain, 1)

        local lost = env_reward.evaluate(
            transition({ owner_team_before = "home", owner_team_after = nil }),
            selection({}, { "possession_gain" })
        )
        t.eq(lost.shaping.possession_gain, -1)

        local shots = env_reward.evaluate(
            transition({
                events = {
                    { kind = "shot", team = "home" },
                    { kind = "header", team = "home" },
                    { kind = "shot", team = "away" },
                    { kind = "pass", team = "home" },
                },
            }),
            selection({}, { "shot_attempt" })
        )
        t.eq(shots.shaping.shot_attempt, 2)

        local contacts = env_reward.evaluate(
            transition({
                events = {
                    { kind = "contact", team = "home", result = "hit" },
                    { kind = "contact", team = "home", result = "guarded" },
                    { kind = "contact", team = "away", result = "hit" },
                },
            }),
            selection({}, { "equipment_contact" })
        )
        t.eq(contacts.shaping.equipment_contact, 1)
    end)

    t.it("keeps shaping separable so an ablation is a subtraction", function()
        local moved = transition({
            score_after = { home = 1, away = 0 },
            owner_team_before = nil,
            owner_team_after = "home",
            events = { { kind = "shot", team = "home" } },
        })
        local both = env_reward.evaluate(
            moved,
            selection({ "goal_scored" }, { "possession_gain", "shot_attempt" })
        )
        t.eq(both.objective_total, 1)
        t.eq(both.shaping_total, 2)
        t.eq(both.total, 3)

        local ablated = env_reward.evaluate(moved, selection({ "goal_scored" }))
        t.eq(ablated.objective_total, both.objective_total)
        t.eq(ablated.shaping_total, 0)
        t.eq(ablated.total, 1)
        t.eq(next(ablated.shaping), nil, "an ablated run reports no shaping terms at all")
    end)

    t.it("never invents a channel the selection did not ask for", function()
        local result = env_reward.evaluate(
            transition({ score_after = { home = 3, away = 0 }, terminated = true }),
            selection({ "match_outcome" })
        )
        t.eq(result.objectives.goal_scored, nil)
        t.eq(result.objectives.experience_proxy_metrics, nil)
        t.eq(result.version, env_reward.VERSION)
    end)
end)
