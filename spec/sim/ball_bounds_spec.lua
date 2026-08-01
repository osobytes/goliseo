local t = require("spec.support.runner")
local match = require("sim.match")
local teams = require("data.teams")
local Vec2 = require("core.vec2")

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

local FIELD = { w = 960, h = 540 }
local PLAYER_RADIUS = 12
local BALL_RADIUS = 6

local function neutral(s)
    local out = {}
    for index in ipairs(s.players) do
        out[index] = NO_INPUT
    end
    return out
end

local function scenario()
    local s = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = FIELD,
        human_controlled = false,
        seed = 11,
    })
    local carrier = 2
    local p = s.players[carrier]
    p.pos = Vec2.new(FIELD.w * 0.5, FIELD.h - 2)
    p.facing = Vec2.new(0, 1)
    s.owner = carrier
    s.ball_vel = Vec2.new(0, 0)
    s.ball_z, s.ball_vz = 0, 0
    -- Cancel place_kickoff's defensive freeze; these scenarios start mid-play.
    s.kickoff_hold = 0
    return s, carrier
end

-- Puts the carrier next to `bx, by` so the ball stays inside dribbling control
-- and the POSSESSION path keeps running. Without this the ball is hundreds of
-- pixels from the carrier, ownership drops on tick 1, and the pre-existing
-- loose-ball walls quietly do all the work -- a test written that way passes
-- with the possession clamp deleted, which is exactly the trap this helper
-- exists to avoid. Dribbling control is TUNE.DRIBBLE_CONTROL (34) plus up to
-- DRIBBLE_CONTROL_SKILL (26), so an offset well under 30 is always retained.
---@return MatchState, integer
local function carrier_at(bx, by, face_x, face_y, team)
    local s, carrier = scenario()
    if team then
        -- Home attacks right and away attacks left, and match AI will not walk
        -- the ball into its own net, so a goal-line case has to be carried by
        -- the side that actually attacks that goal.
        for index, player in ipairs(s.players) do
            if player.team == team and not player.is_keeper then
                carrier = index
                break
            end
        end
        s.owner = carrier
    end
    local p = s.players[carrier]
    -- Players are themselves clamped to the pitch, so the carrier is placed on
    -- the pitch and only the ball is allowed past the line.
    p.pos = Vec2.new(
        math.max(PLAYER_RADIUS, math.min(FIELD.w - PLAYER_RADIUS, bx - face_x * 14)),
        math.max(PLAYER_RADIUS, math.min(FIELD.h - PLAYER_RADIUS, by - face_y * 14))
    )
    p.facing = Vec2.new(face_x, face_y)
    p.vel = Vec2.new(0, 0)
    p.run_vel = Vec2.new(0, 0)
    s.ball = Vec2.new(bx, by)
    s.ball_vel = Vec2.new(0, 0)
    s.ball_z, s.ball_vz = 0, 0
    return s, carrier
end

t.describe("ball bounds", function()
    t.it("pulls a possessed ball back onto the pitch", function()
        -- The touchline clamp lives on the loose-ball path, and the possession
        -- branch returns before reaching it. A ball that ended up outside the
        -- pitch while owned was therefore never corrected: it sat there, the
        -- carrier could not walk out to it (players are clamped to the field),
        -- and the only way to recover it was to shoot it back in.
        local s = scenario()
        s.ball = Vec2.new(FIELD.w * 0.5, FIELD.h + 25)

        for _ = 1, 30 do
            match.step(s, 1 / 60, neutral(s))
        end

        t.is_true(
            s.ball.y <= FIELD.h,
            string.format("ball stayed outside the touchline at y=%.1f", s.ball.y)
        )
    end)

    t.it("keeps a possessed ball inside the arena on every side", function()
        -- Every case here keeps the carrier next to the ball and asserts
        -- possession survives, so the POSSESSION clamp is what is under test.
        -- Written the other way -- ball teleported far from the carrier -- all
        -- four cases pass with the clamp deleted, because ownership drops on
        -- tick 1 and the loose-ball walls handle it instead.
        --
        -- The four are off the goal-mouth band on purpose: there the arena is
        -- exactly the pitch. The mouths are the exception and are covered by
        -- the goal tests below.
        for _, case in ipairs({
            { x = -20, y = 60, fx = -1, fy = 0 },
            { x = FIELD.w + 20, y = FIELD.h - 60, fx = 1, fy = 0 },
            { x = FIELD.w / 2, y = -20, fx = 0, fy = -1 },
            { x = FIELD.w / 2, y = FIELD.h + 20, fx = 0, fy = 1 },
        }) do
            local s, carrier = carrier_at(case.x, case.y, case.fx, case.fy)
            for tick = 1, 12 do
                match.step(s, 1 / 60, neutral(s))
                t.eq(
                    s.owner,
                    carrier,
                    string.format(
                        "possession lost at tick %d, so this case stopped testing the clamp",
                        tick
                    )
                )
            end
            t.is_true(
                s.ball.x >= 0 and s.ball.x <= FIELD.w and s.ball.y >= 0 and s.ball.y <= FIELD.h,
                string.format("ball escaped at (%.1f, %.1f)", s.ball.x, s.ball.y)
            )
        end
    end)

    t.it("leaves a possessed ball exactly on a boundary alone", function()
        -- The side selection turns on strict `< 0` / `> field.w`, so the two
        -- exact boundary values are where an off-by-one would hide. Rescuing a
        -- ball that has not actually left must be a no-op.
        for _, x in ipairs({ 0, FIELD.w }) do
            local s = carrier_at(x, 60, x == 0 and -1 or 1, 0)
            match.step(s, 1 / 60, neutral(s))
            t.is_true(
                s.ball.x >= 0 and s.ball.x <= FIELD.w,
                string.format("boundary ball moved out to x=%.1f", s.ball.x)
            )
        end
    end)

    t.it("still lets a carrier dribble the ball over each goal line", function()
        -- The reason this clamp is to the arena and not to the pitch. A goal
        -- line sits at the pitch edge and check_goal needs the ball to CROSS
        -- it, so a possessed ball clamped to the pitch could never score.
        --
        -- Both goals, deliberately: `in_mouth` tests the y band only and both
        -- goals share it, so a side-selection bug makes exactly one goal line
        -- unreachable and a single-sided test cannot see it.
        for _, case in ipairs({
            { goal = "goal_away", face = 1, scorer = "home", team = "home" },
            { goal = "goal_home", face = -1, scorer = "away", team = "away" },
        }) do
            local probe = scenario()
            local goal = probe[case.goal]
            local mouth_y = goal.y + goal.h / 2
            -- Just inside the line, so the ball crosses it while still within
            -- dribbling control of the carrier: the clamp, not a loose ball,
            -- decides whether the crossing is allowed to happen.
            local start_x = (case.face > 0) and (FIELD.w - 4) or 4
            local s, carrier = carrier_at(start_x, mouth_y, case.face, 0, case.team)
            s.players[carrier].run_vel = Vec2.new(case.face * 200, 0)
            s.players[carrier].vel = s.players[carrier].run_vel

            local crossed_while_owned = false
            local before = s.score[case.scorer]
            for _ = 1, 40 do
                match.step(s, 1 / 60, neutral(s))
                local past = (case.face > 0) and (s.ball.x > FIELD.w) or (s.ball.x < 0)
                if past and s.owner == carrier then
                    crossed_while_owned = true
                end
                if s.score[case.scorer] > before then
                    break
                end
            end
            t.is_true(
                crossed_while_owned,
                case.goal
                    .. ": the ball never crossed the line while owned, so the "
                    .. "possession clamp was never the thing being tested"
            )
            t.is_true(
                s.score[case.scorer] > before,
                case.goal .. ": a ball dribbled over the line must still score"
            )
        end
    end)

    t.it("does not displace a ball a keeper is holding", function()
        -- The keeper hold branch sets the ball from keeper_hold_pos and then
        -- falls through to this same clamp. The hold position is already inside
        -- the arena, so the clamp has to be a no-op over it rather than an
        -- override that fights the keeper every tick.
        local s = scenario()
        local keeper
        for index, player in ipairs(s.players) do
            if player.is_keeper then
                keeper = index
                break
            end
        end
        keeper = assert(keeper, "fixture has no keeper")
        s.owner = keeper
        s.ball = Vec2.new(s.players[keeper].pos.x, s.players[keeper].pos.y)
        match.step(s, 1 / 60, neutral(s))
        local held = s.ball
        match.step(s, 1 / 60, neutral(s))
        t.near(s.ball.x, held.x, 12)
        t.near(s.ball.y, held.y, 12)
        t.is_true(
            s.ball.x >= s.goal_home.x and s.ball.x <= s.goal_away.x + s.goal_away.w,
            string.format("keeper's ball left the arena at x=%.1f", s.ball.x)
        )
    end)

    t.it("recovers a ball stranded behind the net", function()
        -- The arena's far wall is the back of the net, not infinity. This one
        -- resolves through ownership loss rather than the possession clamp --
        -- the carrier is clamped to the pitch, so a ball this far out is beyond
        -- dribbling control and goes loose the same tick. Pinned end-to-end
        -- rather than per-branch because "the ball came back" is the property
        -- the original bug violated, whichever path delivers it.
        local s = scenario()
        local goal = s.goal_away
        s.ball = Vec2.new(goal.x + goal.w + 200, goal.y + goal.h / 2)
        for _ = 1, 20 do
            match.step(s, 1 / 60, neutral(s))
        end
        t.is_true(
            s.ball.x <= goal.x + goal.w,
            string.format("ball stayed behind the net at x=%.1f", s.ball.x)
        )
    end)
end)
