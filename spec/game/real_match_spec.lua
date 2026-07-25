local bootstrap = require("game.bootstrap")
local contract = require("game.match_contract")
local match_adapter = require("game.match_adapter")
local match_observer = require("game.match_observer")
local Match = require("game.screens.match")
local RealMatch = require("game.screens.real_match")
local t = require("spec.support.runner")

local STARTERS = { "ozzo", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin" }

---@param seed integer?
---@return ProductMatchRequest
local function request(seed)
    return assert(contract.new_request({
        home_team_id = "nebula",
        away_team_id = "orion",
        home_starter_ids = STARTERS,
        formation_id = "1-2-1",
        tactic_id = "press_high",
        seed = seed,
    }))
end

t.describe("real match observer", function()
    t.it("derives per-team stats and an evidence-backed MVP from match events", function()
        local match = Match.new()
        local state = match.state
        local value = match_observer.new(state)
        local home_id = state.players[2].id
        local away_keeper = state.players[6].id

        state.events = {
            { kind = "pass", x = 0, y = 0, player = home_id },
            { kind = "shot", x = 0, y = 0, player = home_id },
            { kind = "parry", x = 0, y = 0, player = away_keeper },
        }
        state.owner = 3
        match_observer.observe(value, state, 1)
        state.score.home = 1
        state.events = {}
        match_observer.observe(value, state, 1)

        local summary = match_observer.finish(value)
        t.eq(summary.home_stats.shots, 1)
        t.eq(summary.home_stats.pass_completion, 1)
        t.eq(summary.away_stats.saves, 1)
        t.eq(summary.home_stats.possession, 1)
        t.eq(summary.mvp_player_id, home_id)
    end)

    t.it("can observe every event produced by a multi-tick render update", function()
        local match = Match.new()
        local state = match.state
        local value = match_observer.new(state)
        local home_id = state.players[2].id
        state.events = {}

        match_observer.observe(value, state, 2 / 60, {
            { kind = "pass", x = 0, y = 0, player = home_id },
            { kind = "shot", x = 0, y = 0, player = home_id },
        })

        local summary = match_observer.finish(value)
        t.eq(summary.home_stats.shots, 1)
        t.eq(summary.home_stats.pass_completion, 1)
    end)
end)

t.describe("real match adapter", function()
    t.it("applies request roster, formation, tactic, and seed", function()
        local completed = nil
        local screen = RealMatch.new(request(77), {
            on_finished = function(result)
                completed = result
            end,
            on_cancelled = function() end,
        })
        t.eq(screen.match.state.players[1].id, "ozzo")
        t.eq(screen.match.state.players[2].id, "veil_nyx")
        t.eq(screen.match.state.press.home, 2)

        screen.match.state.score.home = 2
        screen.match.state.score.away = 1
        screen.match.state.finished = true
        screen:update(0.89)
        t.is_true(completed == nil, "full time remains visible before routing to result")
        screen:update(0.02)
        local result = assert(completed)
        t.eq(result.home_score, 2)
        t.eq(result.away_score, 1)
        t.eq(result.seed, 77)
        screen:update(0)
        t.is_true(completed == result, "completion callback fires exactly once")
    end)

    t.it("is the adapter selected by the default bootstrap", function()
        local app = bootstrap.new(960, 540, {
            settings_storage = {
                read = function()
                    return nil
                end,
                write = function()
                    return true
                end,
            },
        })
        t.eq(app.adapter.kind, "real")
        t.eq(app:current_route(), "title")

        app:handle_action({ go = "play" })
        app:handle_action({ go = "formation", starter_ids = STARTERS })
        app:handle_action({ go = "tactic", formation_id = "1-2-1" })
        app:handle_action({ go = "match", tactic_id = "press_high" })
        t.eq(app:current_route(), "match")
        local screen = assert(app.stack:current())
        ---@cast screen RealMatchScreen
        screen.match.state.score.home = 3
        screen.match.state.finished = true
        app:update(0.9)
        t.eq(app:current_route(), "result")
        t.eq(assert(app.session.last_result).home_score, 3)
    end)

    t.it("keeps the fake adapter available for isolated product-flow tests", function()
        t.eq(match_adapter.fake().kind, "fake")
        t.eq(match_adapter.real().kind, "real")
    end)

    t.it("constructs combat only for the explicit post-showcase request", function()
        local showcase = RealMatch.new(request(92), {
            on_finished = function() end,
            on_cancelled = function() end,
        })
        t.is_true(showcase.match._combat_state == nil)

        local combat_request = request(93)
        combat_request.combat_enabled = true
        local prototype = RealMatch.new(combat_request, {
            on_finished = function() end,
            on_cancelled = function() end,
        })
        t.is_true(prototype.match._combat_state ~= nil)
        t.eq(#assert(prototype.match._combat_state).players, 10)
    end)

    t.it("allows confirmation to advance the full-time hold after its safety beat", function()
        local completed = false
        local screen = RealMatch.new(request(91), {
            on_finished = function()
                completed = true
            end,
            on_cancelled = function() end,
        })
        screen.match.state.finished = true
        screen:update(0.24)
        screen:event({ kind = "action", action = "confirm" })
        t.is_true(not completed, "confirmation cannot erase the full-time beat immediately")
        screen:update(0.02)
        screen:event({ kind = "action", action = "confirm" })
        t.is_true(completed)
    end)
end)
