-- #268: there is no goal limit. A match is decided on score at full time, in
-- every mode. The decision and its reasoning are recorded in
-- `docs/online/match_flow.md`.
--
-- This spec lives in `spec/game/` rather than `spec/sim/` because the invariant
-- it pins is a cross-layer one: the simulation default, the learning
-- environment's default, the content-derived online manifest, and the protocol
-- fixture all have to say the same thing. Online said 5 and offline said 3 for
-- as long as both existed, and nothing failed when they diverged. Now something
-- does.

local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local match = require("sim.match")
local env_config = require("sim.env_config")
local match_manifest = require("game.online.match_manifest")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local teams = require("data.teams")

local TICK = 1 / 60

---@type MatchInput
local NO_INPUT = {
    move = Vec2.new(0, 0),
    shoot = false,
    shoot_held = false,
    pass = false,
    pass_held = false,
    switch = false,
    dash = false,
    dodge = false,
    lob = false,
    sprint = false,
    jockey = false,
    equipment_held = false,
    equipment_pressed = false,
    equipment_released = false,
}

---@return MatchState
local function new_match()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 268,
    })
end

-- Walk the ball over the away goal line with a home outfielder, the same way
-- `spec/sim/transition_windows_match_spec.lua` does. One step, one goal.
---@param state MatchState
local function score_home_goal(state)
    state.kickoff_hold = 0
    state.owner = 5
    local carrier = state.players[5]
    carrier.pos = Vec2.new(941, 270)
    carrier.facing = Vec2.new(1, 0)
    carrier.run_vel = Vec2.new(carrier.move_speed, 0)
    carrier.vel = carrier.run_vel
    state.ball = Vec2.new(965, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.ball_z = 0
    state.ball_vz = 0
    match.step(state, TICK, NO_INPUT)
end

t.describe("the goal limit", function()
    t.it("is the same value in every source that states one", function()
        -- 99 is `protocol.MAX_GOALS`, the largest a frozen manifest may carry.
        -- Unreachable in 7,200 ticks, which is what makes it mean "no limit"
        -- without removing the field from the wire.
        t.eq(match.NO_GOAL_LIMIT, 99)
        t.eq(protocol.MAX_GOALS, match.NO_GOAL_LIMIT, "the wire ceiling is the no-limit value")
        t.eq(new_match().max_goals, match.NO_GOAL_LIMIT, "the simulation default")
        t.eq(env_config.DEFAULT_MAX_GOALS, match.NO_GOAL_LIMIT, "the learning environment default")
        t.eq(
            match_manifest.DEFAULT_MAX_GOALS,
            match.NO_GOAL_LIMIT,
            "the content-derived online manifest"
        )
        t.eq(
            protocol_fixture.manifest().max_goals,
            match.NO_GOAL_LIMIT,
            "the protocol conformance fixture"
        )
    end)

    t.it("is carried by the online manifest in every match mode", function()
        for _, mode in ipairs({ "1v1", "2v2", "4v4" }) do
            t.eq(
                match_manifest.template(mode).max_goals,
                match.NO_GOAL_LIMIT,
                mode .. " must not end on a different goal count"
            )
        end
    end)

    t.it("does not end a match at the counts online and offline used to stop at", function()
        local state = new_match()
        -- Three was the offline default and five the online one. Both are now
        -- ordinary scorelines a match plays straight through.
        for goals = 1, 6 do
            score_home_goal(state)
            t.eq(state.score.home, goals, "goal " .. goals .. " must count")
            t.is_true(not state.finished, "the match must survive a scoreline of " .. goals)
        end
    end)

    t.it("ends on the clock, with the score deciding it", function()
        local state = new_match()
        score_home_goal(state)
        state.time_left = TICK / 2
        match.step(state, TICK, NO_INPUT)
        t.is_true(state.finished, "full time ends the match")
        t.eq(state.score.home, 1)
        t.eq(state.score.away, 0)
    end)

    t.it("still terminates a match early when a caller asks for a limit", function()
        -- The mechanism stays: `max_goals` is a live rule the evidence fixtures,
        -- rollback laboratories, and short-match specs depend on. Only the
        -- default changed.
        local state = new_match()
        state.max_goals = 2
        score_home_goal(state)
        t.is_true(not state.finished)
        score_home_goal(state)
        t.is_true(state.finished, "an explicit limit must still end the match")
    end)
end)
