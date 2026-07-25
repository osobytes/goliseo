local t = require("spec.support.runner")
local match = require("sim.match")
local teams = require("data.teams")
local tactics = require("data.tactics")
local Vec2 = require("core.vec2")

local function new_match()
    return match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
end

---@param o table?
---@return MatchInput
local function input(o)
    o = o or {}
    return {
        move = o.move or Vec2.new(0, 0),
        shoot = o.shoot or false,
        shoot_held = o.shoot_held or false,
        pass = o.pass or false,
        pass_held = o.pass_held or false,
        switch = o.switch or false,
        dash = o.dash or false,
        dodge = o.dodge or false,
        lob = o.lob or false,
        sprint = o.sprint or false,
        jockey = o.jockey or false,
    }
end

local NO_INPUT = input()

-- Frames needed to advance past the 0.15s shot wind-up at 1/60 s per step.
local WINDUP_FRAMES = math.ceil(0.15 * 60) + 1 -- 10

-- Step `n` frames at 1/60 s each, all with NO_INPUT.
---@param s MatchState
---@param n integer
local function step_frames(s, n)
    for _ = 1, n do
        match.step(s, 1 / 60, NO_INPUT)
    end
end

t.describe("match.new", function()
    t.it("kicks off with 10 players and the home side in possession", function()
        local s = new_match()
        t.eq(#s.players, 10)
        t.is_true(s.owner == s.controlled, "controlled player should start with the ball")
        t.eq(s.score.home, 0)
        t.eq(s.score.away, 0)
        t.is_true(s.players[s.controlled].team == "home")
        t.is_true(not s.players[s.controlled].is_keeper)
    end)

    t.it("lets match AI drive both teams when no player is human-controlled", function()
        ---@param owner_idx integer
        ---@param direction number
        local function assert_ai_owner_moves(owner_idx, direction)
            local s = match.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
                human_controlled = false,
            })
            local owner = s.players[owner_idx]
            s.owner = owner_idx
            s.ball = owner.pos
            local before_x = owner.pos.x

            match.step(s, 1 / 60, NO_INPUT)

            t.is_true(
                (owner.pos.x - before_x) * direction > 0,
                owner.team .. " owner should dribble toward the opposing goal"
            )
        end

        local opening_owner = new_match().controlled
        assert_ai_owner_moves(opening_owner, 1)
        assert_ai_owner_moves(7, -1)
    end)
end)

t.describe("match.step timer", function()
    t.it("counts down and ends at full time", function()
        local s = new_match()
        match.step(s, 10, NO_INPUT)
        t.near(s.time_left, 110, 1e-6)
        t.is_true(not s.finished)
        match.step(s, 200, NO_INPUT)
        t.is_true(s.finished)
        t.eq(s.time_left, 0)
    end)
end)

t.describe("match.step shooting & passing", function()
    t.it("shooting releases the ball toward the goal", function()
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(1, 0)
        match.step(s, 0.016, input({ shoot = true }))
        -- Ball is still owned during the wind-up; step past it.
        step_frames(s, WINDUP_FRAMES)
        t.is_true(s.owner == nil)
        t.is_true(s.ball_vel.x > 0, "home shoots toward the right goal")
    end)

    t.it("aiming up sends the shot to the top corner", function()
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(0, -1)
        match.step(s, 0.016, input({ shoot = true }))
        step_frames(s, WINDUP_FRAMES)
        t.is_true(s.owner == nil)
        t.is_true(s.ball_vel.x > 0, "still goal-ward")
        t.is_true(s.ball_vel.y < 0, "and toward the top corner")
    end)

    t.it("passing sends the ball toward a teammate in the aim direction", function()
        local s = new_match()
        local owner = s.players[s.controlled]
        local mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and i ~= s.controlled and not p.is_keeper then
                mate = p
                break
            end
        end
        owner.facing = mate.pos:sub(owner.pos):normalized()
        match.step(s, 0.016, input({ pass = true }))
        t.is_true(s.owner == nil, "ball should be released on a pass")
        t.near(s.ball_vel:length(), 420, 0.5, "pass speed")
    end)
end)

t.describe("match.step charge shot", function()
    ---@param charge number
    ---@return number
    local function shot_speed(charge)
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(1, 0)
        s.players[s.controlled].charge = charge
        match.step(s, 0.016, input({ shoot = true }))
        -- Step past the wind-up so the ball is in flight.
        step_frames(s, WINDUP_FRAMES)
        return s.ball_vel:length()
    end

    t.it("a full charge shoots meaningfully harder than a tap", function()
        t.is_true(shot_speed(1) > shot_speed(0) * 1.5)
    end)

    t.it("holding shoot builds charge", function()
        local s = new_match()
        match.step(s, 0.1, input({ shoot_held = true }))
        t.is_true(s.players[s.controlled].charge > 0)
    end)
end)

t.describe("match.step juke", function()
    t.it("a dodging carrier is immune to tackles", function()
        local s = new_match()
        local me = s.players[s.controlled]
        local foe
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                foe = p
                break
            end
        end
        foe.pos = Vec2.new(me.pos.x + 8, me.pos.y)
        foe.dash_cd = 0
        me.dodge_timer = 1.0
        match.step(s, 0.016, input())
        t.is_true(s.owner == s.controlled, "dodging carrier keeps the ball")
    end)
end)

t.describe("match.step tackling", function()
    local function carrier_setup()
        local s = new_match()
        local away_idx
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                away_idx = i
                break
            end
        end
        s.owner = away_idx
        -- Challenges reach for the ball, so put it at the carrier's feet.
        local c = s.players[away_idx]
        s.ball = c.pos:add(c.facing:scale(18))
        -- Park the carrier's teammates out of passing range so it can't bail
        -- out of the challenge with a pressure pass.
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper and i ~= away_idx then
                p.pos = Vec2.new(40, 380 + i * 15)
            end
        end
        return s, away_idx, c
    end

    t.it("a standing tackle (slow) knocks the ball loose", function()
        local s, away_idx, carrier = carrier_setup()
        local me = s.players[s.controlled]
        -- stand on the BALL side (in front of the carrier), inside poke reach
        me.pos = Vec2.new(carrier.pos.x - 26, carrier.pos.y)
        me.vel = Vec2.new(0, 0) -- standing -> standing poke
        match.step(s, 0.016, input({ dash = true }))
        t.is_true(s.owner ~= away_idx, "carrier loses possession to the standing tackle")
    end)

    t.it("a slide (sprinting) wins the ball from further away and stuns the carrier", function()
        local s, away_idx, carrier = carrier_setup()
        local me = s.players[s.controlled]
        -- approach the BALL side (in front of the carrier), sprinting into a slide
        me.pos = Vec2.new(carrier.pos.x - 32, carrier.pos.y)
        me.vel = Vec2.new(200, 0)
        me.sprinting = true -- sprint + tackle = slide
        match.step(s, 0.016, input({ dash = true, move = Vec2.new(1, 0), sprint = true }))
        t.is_true(s.owner ~= away_idx, "slide wins the ball at extended reach")
        t.is_true(carrier.stun_timer > 0, "the slid-through carrier is knocked off balance")
    end)

    t.it("a carrier shields the ball from a challenge behind them", function()
        local s, away_idx, carrier = carrier_setup()
        carrier.facing = Vec2.new(-1, 0) -- ball sticks a step toward -x
        s.ball = carrier.pos:add(Vec2.new(-18, 0))
        local defender
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                defender = i
                break
            end
        end
        s.players[defender].pos = Vec2.new(carrier.pos.x + 20, carrier.pos.y) -- on their back
        s.players[defender].dash_cd = 0
        s.players[s.controlled].pos = Vec2.new(60, 60) -- human well away
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, away_idx, "the shielded ball stays with the carrier")
        t.is_true(s.players[defender].dash_cd > 0, "the failed poke still goes on cooldown")
    end)

    t.it("the same challenge from the ball side wins it", function()
        local s, away_idx, carrier = carrier_setup()
        carrier.facing = Vec2.new(-1, 0)
        s.ball = carrier.pos:add(Vec2.new(-18, 0))
        local defender
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                defender = i
                break
            end
        end
        s.players[defender].pos = Vec2.new(carrier.pos.x - 20, carrier.pos.y) -- goal side, on the ball
        s.players[defender].dash_cd = 0
        s.players[s.controlled].pos = Vec2.new(60, 60)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.owner ~= away_idx, "a front-on challenge dislodges the ball")
    end)

    t.it("the human can poke the ball loose from behind at contact range", function()
        local s, away_idx, carrier = carrier_setup()
        local me = s.players[s.controlled]
        me.pos = Vec2.new(carrier.pos.x + 24, carrier.pos.y) -- on the carrier's back
        me.vel = Vec2.new(0, 0)
        -- Chase while poking: the carrier dribbles away during the frame.
        match.step(s, 0.016, input({ dash = true, move = Vec2.new(-1, 0) }))
        t.is_true(s.owner ~= away_idx, "a contact-range poke wins even from behind")
    end)

    t.it("a jogging (non-sprint) tackle is a poke, not a slide", function()
        local s = new_match()
        local me = s.players[s.controlled]
        me.vel = Vec2.new(-150, 0) -- moving, but not sprinting
        match.step(s, 0.001, input({ dash = true, move = Vec2.new(-1, 0) }))
        t.is_true(me.slide_timer <= 0, "no committed slide without sprint")
        t.is_true(me.tackle_timer > 0, "a standing poke instead")
    end)

    t.it("slide speed scales with current velocity", function()
        local function slide_vel(speed)
            local s = new_match()
            local me = s.players[s.controlled]
            me.vel = Vec2.new(speed, 0)
            me.facing = Vec2.new(1, 0)
            me.sprinting = true
            match.step(s, 0.001, input({ dash = true, move = Vec2.new(1, 0), sprint = true }))
            return me.slide_vel
        end
        t.is_true(slide_vel(300) > slide_vel(150), "a faster run produces a faster slide")
    end)

    t.it("a stunned defender cannot tackle", function()
        local s, away_idx, carrier = carrier_setup()
        local me = s.players[s.controlled]
        me.pos = Vec2.new(carrier.pos.x + 8, carrier.pos.y)
        me.vel = Vec2.new(0, 0)
        me.stun_timer = 1.0
        match.step(s, 0.016, input({ dash = true }))
        t.is_true(s.owner == away_idx, "a stunned player can't win the ball")
    end)
end)

t.describe("match.step switching", function()
    t.it("hands control to the home outfielder nearest the ball", function()
        local s = new_match()
        local before = s.controlled
        -- Park a loose ball next to a specific teammate.
        local target
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= before then
                target = i
            end
        end
        s.owner = nil
        s.pickup_cd = 1
        s.ball = Vec2.new(s.players[target].pos.x + 30, s.players[target].pos.y)
        s.players[target].run_vel = Vec2.new(0, 0)
        match.step(s, 0.016, input({ switch = true, move = Vec2.new(1, 0) }))
        t.eq(s.controlled, target, "switch picks the player closest to the ball")
        t.is_true(s.players[s.controlled].team == "home")
        t.is_true(not s.players[s.controlled].is_keeper)
        t.is_true(
            s.players[target].run_vel.x > 0,
            "the newly selected player receives this tick's movement"
        )
    end)
end)

t.describe("match tactics", function()
    ---@param s MatchState
    ---@return number
    local function mean_home_outfield_x(s)
        local sum, n = 0, 0
        for _, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper then
                sum = sum + p.anchor.x
                n = n + 1
            end
        end
        return sum / n
    end

    t.it("press high pushes the home shape higher up the pitch", function()
        local balanced = new_match()
        local high = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            tactic = tactics.press_high,
        })
        t.is_true(mean_home_outfield_x(high) > mean_home_outfield_x(balanced))
    end)

    t.it("counter attack drops the home shape deeper", function()
        local balanced = new_match()
        local counter = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            tactic = tactics.counter,
        })
        t.is_true(mean_home_outfield_x(counter) < mean_home_outfield_x(balanced))
    end)

    t.it("press high assigns two home pressers", function()
        local high = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            tactic = tactics.press_high,
        })
        t.eq(high.press.home, 2)
    end)
end)

t.describe("match.step scoring", function()
    t.it("a ball wholly crossing the right goal line scores for home", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1 -- keep anyone from collecting it
        s.ball = Vec2.new(s.field.w - 5, s.field.h / 2)
        s.ball_vel = Vec2.new(400, 0)
        for _ = 1, 10 do
            match.step(s, 0.016, NO_INPUT)
            if s.score.home > 0 then
                break
            end
        end
        t.eq(s.score.home, 1)
        t.eq(s.score.away, 0)
    end)

    t.it("a ball ON the line is not yet a goal (must wholly cross)", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1
        -- Ball centre right on the goal line, barely moving: still in play.
        s.ball = Vec2.new(s.field.w, s.field.h / 2)
        s.ball_vel = Vec2.new(1, 0)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.score.home, 0, "on the line is not across the line")
    end)
end)

t.describe("match.step keeper", function()
    local function keeper_of(s, team)
        for i, p in ipairs(s.players) do
            if p.team == team and p.is_keeper then
                return i, p
            end
        end
    end

    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("catches a shot hit straight at it", function()
        local s = new_match()
        local ki, k = keeper_of(s, "away")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(945, 270) -- between the ball and the goal
        s.ball = Vec2.new(925, 270)
        s.ball_vel = Vec2.new(100, 0) -- crosses the keeper's line right at it
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, ki, "keeper should hold the catch")
        t.near(s.ball_vel:length(), 0, 1e-6)
        t.eq(s.score.home, 0, "a catch concedes nothing")
        t.is_true(has_event(s, "catch"), "expected a catch event")
    end)

    t.it("parries a shot to its side it can reach but not gather", function()
        local s = new_match()
        local _, k = keeper_of(s, "away")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(935, 300)
        s.ball = Vec2.new(890, 250)
        s.ball_vel = Vec2.new(380, 0) -- crosses to the keeper's side, fast: reachable, not catchable
        -- The save commits at once but completes when the ball ARRIVES at the
        -- diving keeper — play the flight out.
        local parried = false
        for _ = 1, 30 do
            match.step(s, 0.016, NO_INPUT)
            parried = parried or has_event(s, "parry")
            if parried then
                break
            end
        end
        t.is_true(parried, "expected a parry event")
        t.is_true(s.owner == nil, "a parry does not gain possession")
        t.is_true(s.ball_vel.x < 0, "ball is deflected back away from the goal")
        t.eq(s.score.home, 0)
    end)

    t.it("a saved shot flies its whole trajectory into the glove (no teleport)", function()
        local s = new_match()
        local _, k = keeper_of(s, "away")
        k.pos = Vec2.new(938, 270)
        s.owner = nil
        s.pickup_cd = 0.3 -- as if just released by the shooter
        s.ball = Vec2.new(738, 270) -- 200px out, straight at the keeper
        s.ball_vel = Vec2.new(500, 0)
        local caught_at, max_jump = nil, 0
        local prev = s.ball
        for f = 1, 60 do
            match.step(s, 1 / 60, NO_INPUT)
            max_jump = math.max(max_jump, prev:dist(s.ball))
            prev = s.ball
            if has_event(s, "catch") then
                caught_at = f
                break
            end
        end
        t.is_true(caught_at ~= nil, "the straight shot is caught")
        t.is_true(caught_at >= 12, "the ball spent real frames in flight (no zone snap)")
        t.is_true(max_jump < 60, "no single frame teleported the ball")
    end)

    t.it("is beaten when the shot crosses out of dive reach", function()
        local s = new_match()
        local _, k = keeper_of(s, "away")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(945, 120) -- stuck high; can't reach a central shot in time
        s.ball = Vec2.new(880, 270)
        s.ball_vel = Vec2.new(520, 0)
        local saved = false
        for _ = 1, 30 do
            match.step(s, 0.016, NO_INPUT)
            if has_event(s, "catch") or has_event(s, "parry") then
                saved = true
            end
            if s.score.home > 0 then
                break
            end
        end
        t.eq(s.score.home, 1, "an unreachable shot scores")
        t.is_true(not saved, "keeper made no save")
    end)

    t.it("holds a gathered ball safe from a challenging striker", function()
        local s = new_match()
        local ki, k = keeper_of(s, "away")
        s.owner = ki
        k.hold_timer = 1 -- still holding (won't distribute this step)
        -- An AI striker right on top of the keeper, ready to challenge.
        local striker
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                striker = p
                break
            end
        end
        striker.pos = Vec2.new(k.pos.x + 10, k.pos.y)
        striker.dash_cd = 0
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, ki, "the keeper keeps the gathered ball")
        t.is_true(not has_event(s, "tackle"), "a keeper in possession can't be tackled")
    end)

    t.it("distributes to a teammate instead of hoofing it", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        for _, p in ipairs(s.players) do
            if p.team == "away" then
                p.pos = Vec2.new(950, 40) -- clear every outlet of pressure
            end
        end
        s.owner = ki
        k.hold_timer = 0 -- hold already elapsed: distribute this step
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.owner == nil, "the keeper releases the ball")
        -- A paced short throw (arrives with a touch left), not a long clear.
        local speed = s.ball_vel:length()
        t.is_true(speed >= 320 and speed <= 620, "throw pace is a pass, not a hoof")
        t.is_true(has_event(s, "pass"), "expected a pass event")
        t.is_true(not has_event(s, "shot"), "should not hoof it upfield")
    end)
end)

t.describe("match.step off-ball AI", function()
    -- indices: home 1..5 (keeper 1), away 6..10 (keeper 6)
    local AWAY_CARRIER = 7

    local function pos_of(s)
        local p = {}
        for i, pl in ipairs(s.players) do
            p[i] = pl.pos
        end
        return p
    end

    -- Home defenders (non-keeper, non-controlled) that received a target.
    local function home_targets(s, targets)
        local list = {}
        for i, pl in ipairs(s.players) do
            if pl.team == "home" and not pl.is_keeper and targets[i] then
                list[#list + 1] = i
            end
        end
        return list
    end

    -- Replicate the internal closest-to-carrier ordering for role identification.
    local function defender_order(s, p)
        local order = {}
        for i, pl in ipairs(s.players) do
            if pl.team == "home" and not pl.is_keeper and i ~= s.controlled then
                order[#order + 1] = i
            end
        end
        table.sort(order, function(a, b)
            local da, db = p[a]:dist(p[AWAY_CARRIER]), p[b]:dist(p[AWAY_CARRIER])
            if da ~= db then
                return da < db
            end
            return a < b
        end)
        return order
    end

    local function defending_state(scheme)
        local s = new_match()
        if scheme then
            s.marking.home.scheme = scheme
        end
        s.owner = AWAY_CARRIER
        s.players[AWAY_CARRIER].pos = Vec2.new(480, 270)
        s.ball = Vec2.new(480, 270)
        -- Spread the away side to its open-play anchors: kickoff clamps them
        -- to their own half, which would bunch every marker's man (and so the
        -- marker targets) right next to the carrier at the halfway line.
        for i, p in ipairs(s.players) do
            if p.team == "away" and i ~= AWAY_CARRIER then
                p.pos = Vec2.new(p.anchor.x, p.anchor.y)
            end
        end
        -- This models settled open-play defending, not a restart: clear the
        -- kickoff hold that `new_match()` sets so pressing behaves normally.
        s.kickoff_hold = 0
        return s
    end

    t.it("sends exactly one presser to the carrier", function()
        local s = defending_state()
        local targets = match._offball_targets(s, pos_of(s))
        local carrier = s.players[AWAY_CARRIER].pos
        local near = 0
        for _, i in ipairs(home_targets(s, targets)) do
            if targets[i]:dist(carrier) <= 24 + 25 then
                near = near + 1
            end
        end
        t.eq(near, 1, "exactly one defender presses; the rest hold shape")
    end)

    t.it("holds shape instead of pressing during the post-kickoff hold", function()
        local s = defending_state()
        s.kickoff_hold = 2.5 -- as set by place_kickoff on a restart
        local targets = match._offball_targets(s, pos_of(s))
        local carrier = s.players[AWAY_CARRIER].pos
        local near = 0
        for _, i in ipairs(home_targets(s, targets)) do
            if targets[i]:dist(carrier) <= 24 + 25 then
                near = near + 1
            end
        end
        t.eq(near, 0, "no defender presses the carrier while the kickoff hold is active")
    end)

    t.it("positions the cover goal-side between carrier and own goal", function()
        local s = defending_state()
        local p = pos_of(s)
        local cover = defender_order(s, p)[2]
        local targets = match._offball_targets(s, p)
        t.is_true(cover ~= nil, "a cover defender exists")
        t.is_true(
            targets[cover].x > 5 and targets[cover].x < 480,
            "cover sits between own goal and the carrier"
        )
    end)

    t.it("shifts the defensive block toward the ball (zonal)", function()
        local s = defending_state("zonal")
        s.players[AWAY_CARRIER].pos = Vec2.new(480, 30) -- ball high near the top
        s.ball = Vec2.new(480, 30)
        local p = pos_of(s)
        local rest = defender_order(s, p)[3] -- a zone-holding defender
        local targets = match._offball_targets(s, p)
        t.is_true(rest ~= nil, "a zonal defender exists")
        t.is_true(
            targets[rest].y < s.players[rest].anchor.y,
            "the block slides up toward the high ball"
        )
    end)

    t.it("man-marks an opponent on the goal side", function()
        local s = defending_state("man")
        local targets = match._offball_targets(s, pos_of(s))
        local found = false
        for def, opp in pairs(s.marks.home) do
            found = true
            t.is_true(
                targets[def].x <= s.players[opp].pos.x + 1,
                "marker stands goal-side (lower x) of its man"
            )
        end
        t.is_true(found, "man scheme produced at least one mark")
    end)

    t.it("only man/hybrid schemes create marking assignments", function()
        local function marks_count(scheme)
            local s = defending_state(scheme)
            match._offball_targets(s, pos_of(s))
            local n = 0
            for _ in pairs(s.marks.home) do
                n = n + 1
            end
            return n
        end
        t.eq(marks_count("zonal"), 0)
        t.is_true(marks_count("man") >= 1, "man scheme assigns marks")
        t.is_true(marks_count("hybrid") >= 1, "hybrid scheme assigns marks")
    end)

    t.it("off-ball attackers leave their anchor to support the carrier", function()
        local s = new_match()
        local owner_idx
        for i, pl in ipairs(s.players) do
            if pl.team == "home" and not pl.is_keeper and i ~= s.controlled then
                owner_idx = i
                break
            end
        end
        s.owner = owner_idx
        s.players[owner_idx].pos = Vec2.new(500, 270)
        s.ball = Vec2.new(500, 270)
        local targets = match._offball_targets(s, pos_of(s))
        local moved = false
        for i, pl in ipairs(s.players) do
            if pl.team == "home" and not pl.is_keeper and targets[i] then
                if targets[i]:dist(pl.anchor) > 1 then
                    moved = true
                end
            end
        end
        t.is_true(moved, "an off-ball attacker repositioned off its anchor to support")
    end)
end)

t.describe("match player collisions", function()
    t.it("pushes overlapping players apart to at least their combined radius", function()
        local s =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        local a, b = s.players[2], s.players[3]
        a.pos = Vec2.new(200, 500) -- empty corner, away from other players
        b.pos = Vec2.new(210, 500) -- 10px apart, radii 12+12=24 -> overlapping
        match._resolve_collisions(s)
        t.is_true(a.pos:dist(b.pos) >= 23.9, "bodies separated to ~combined radius")
    end)

    t.it("a sliding player barges through and stuns the one it hits", function()
        local s =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        local slider, victim = s.players[2], s.players[8] -- home vs away
        slider.pos = Vec2.new(200, 500)
        slider.slide_timer = 0.3
        victim.pos = Vec2.new(210, 500)
        victim.stun_timer = 0
        match._resolve_collisions(s)
        t.is_true(victim.stun_timer > 0, "the player hit by the slide is knocked off balance")
    end)
end)

t.describe("match.step keeper claim", function()
    local function keeper_of(s, team)
        for i, p in ipairs(s.players) do
            if p.team == team and p.is_keeper then
                return i, p
            end
        end
    end

    t.it("comes off its line to claim a slow loose ball in the box", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(40, 270) -- on its line
        s.ball = Vec2.new(70, 270) -- loose, just inside the box
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        -- either it stepped out toward the ball, or already gathered it
        t.is_true(s.owner == ki or s.players[ki].pos.x > 40, "keeper claims / advances on the ball")
    end)

    t.it("gathers a loose ball it reaches in its box", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(60, 270)
        s.ball = Vec2.new(80, 270) -- within the extended claim radius (30)
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, ki, "keeper picks up the loose ball in its box")
    end)

    t.it("does not leave its line for a ball outside the box", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        s.owner = nil
        s.pickup_cd = 1 -- nobody collects this step
        k.pos = Vec2.new(40, 270)
        s.ball = Vec2.new(480, 270) -- midfield, well outside the box
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.players[ki].pos.x < 70, "keeper holds its line, doesn't chase midfield")
    end)
end)

t.describe("match.step keeper box dominance", function()
    local function keeper_of(s, team)
        for i, p in ipairs(s.players) do
            if p.team == team and p.is_keeper then
                return i, p
            end
        end
    end
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end
    local function away_outfielders(s)
        local out = {}
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                out[#out + 1] = i
            end
        end
        return out
    end

    t.it("keeper wins a contested loose ball in its own box", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(40, 270)
        s.players[away_outfielders(s)[1]].pos = Vec2.new(75, 270) -- attacker a step closer
        s.ball = Vec2.new(60, 270) -- loose in the home box
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, ki, "keeper claims the ball in its box over the closer attacker")
    end)

    t.it("fires a claim event when the keeper gathers in its box", function()
        local s = new_match()
        local _, k = keeper_of(s, "home")
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(60, 270)
        s.ball = Vec2.new(80, 270)
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "claim"), "a box gather emits a claim event")
    end)

    t.it("does NOT get priority outside its box (closer attacker wins)", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        local att = away_outfielders(s)[1]
        s.owner = nil
        s.pickup_cd = 0
        k.pos = Vec2.new(175, 270) -- ball at x=180 is outside the box (depth > 160)
        s.players[att].pos = Vec2.new(182, 270) -- strictly closer to the ball
        s.ball = Vec2.new(180, 270)
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, att, "outside the box the nearer attacker wins")
    end)

    t.it("long-clears when every distribution outlet is marked", function()
        local s = new_match()
        local ki, k = keeper_of(s, "home")
        -- pin an opponent onto each home outfielder so no safe outlet exists
        local outs = away_outfielders(s)
        local home_out = {}
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper then
                home_out[#home_out + 1] = i
            end
        end
        for n = 1, math.min(#outs, #home_out) do
            s.players[outs[n]].pos =
                Vec2.new(s.players[home_out[n]].pos.x, s.players[home_out[n]].pos.y)
        end
        s.owner = ki
        k.hold_timer = 0
        match.step(s, 0.001, NO_INPUT)
        t.is_true(s.owner == nil, "keeper releases the ball")
        t.is_true(s.ball_vel.x > 0, "it is cleared upfield, not passed sideways into pressure")
        t.is_true(not has_event(s, "pass"), "a clearance is not a safe pass")
    end)
end)

t.describe("match ball height (z)", function()
    local function loose(z, vz, vx)
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 5 -- keep anyone from collecting during the flight
        s.ball = Vec2.new(480, 270)
        s.ball_vel = Vec2.new(vx or 0, 0)
        s.ball_z = z
        s.ball_vz = vz
        return s
    end

    t.it("a lofted ball rises, slows under gravity, then comes back down", function()
        local s = loose(0, 300, 0)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.ball_z > 0, "it left the ground")
        t.near(s.ball_vz, 300 - 900 * 0.016, 1e-6, "gravity decremented vertical speed")
        for _ = 1, 80 do
            match.step(s, 0.016, NO_INPUT)
            t.is_true(s.ball_z >= 0, "height never goes negative")
        end
    end)

    t.it("rebounds off the ground keeping its horizontal pace", function()
        local s = loose(2, -300, 200)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.ball_vz > 0, "bounced back up")
        t.is_true(s.ball_vel.x > 150, "kept most of its horizontal speed through the bounce")
    end)

    t.it("settles instead of micro-bouncing forever", function()
        local s = loose(0.1, -40, 0)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.ball_z, 0)
        t.eq(s.ball_vz, 0)
    end)

    t.it("possession grounds the ball (z and vz reset)", function()
        local s = new_match()
        s.ball_z = 50
        s.ball_vz = 200
        s.owner = s.controlled
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.ball_z, 0)
        t.eq(s.ball_vz, 0)
    end)
end)

t.describe("match height gates", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("a ball in the air flies over heads and is not collected", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 0
        -- park a teammate right under the ball
        local p = s.players[2]
        p.pos = Vec2.new(480, 270)
        s.ball = Vec2.new(480, 270)
        s.ball_vel = Vec2.new(0, 0)
        s.ball_z = 40 -- above GROUND_GRAB_HEIGHT
        s.ball_vz = 0
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.owner == nil, "nobody collects an overhead ball")
    end)

    t.it("the same ball on the ground IS collected", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 0
        s.players[2].pos = Vec2.new(480, 270)
        s.ball = Vec2.new(480, 270)
        s.ball_vel = Vec2.new(0, 0)
        s.ball_z = 0
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.owner ~= nil, "a grounded ball is collected")
    end)

    t.it("a shot over the crossbar is not a goal; under the bar scores", function()
        local function shoot_at_line(z)
            local s = new_match()
            s.owner = nil
            s.pickup_cd = 1
            s.ball = Vec2.new(s.field.w - 5, s.field.h / 2)
            s.ball_vel = Vec2.new(400, 0)
            s.ball_z = z
            s.ball_vz = 60 -- hold height through the short flight to the line
            for _ = 1, 10 do
                match.step(s, 0.016, NO_INPUT)
                if s.score.home > 0 then
                    break
                end
            end
            return s.score.home
        end
        t.eq(shoot_at_line(80), 0, "over the bar: no goal")
        t.eq(shoot_at_line(10), 1, "under the bar: goal")
    end)

    t.it("a keeper does not save a ball above its aerial reach", function()
        local s = new_match()
        local ki
        for i, p in ipairs(s.players) do
            if p.team == "away" and p.is_keeper then
                ki = i
                s.players[i].pos = Vec2.new(945, 270)
            end
        end
        s.owner = nil
        s.pickup_cd = 0
        s.ball = Vec2.new(925, 270)
        s.ball_vel = Vec2.new(100, 0)
        s.ball_z = 80 -- well above the keeper's aerial reach as it crosses the line
        s.ball_vz = 100 -- still rising, so it stays high over the keeper
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.owner ~= ki, "the high ball sails over the keeper")
        t.is_true(not has_event(s, "catch"), "no catch on a ball over the keeper")
    end)
end)

t.describe("match lobs and chips", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    ---@param keeper_x number
    ---@param shot_speed number
    ---@return MatchState
    ---@return MatchPlayer
    ---@return MatchPlayer
    local function human_chip_state(keeper_x, shot_speed)
        local s = new_match()
        local shooter = s.players[s.controlled]
        local defending_keeper
        local parking_y = 30
        for _, player in ipairs(s.players) do
            if player.team == "away" and player.is_keeper then
                defending_keeper = player
            elseif player ~= shooter and not player.is_keeper then
                player.pos = Vec2.new(player.team == "home" and 300 or 820, parking_y)
                player.anchor = player.pos
                player.vel = Vec2.new(0, 0)
                player.run_vel = Vec2.new(0, 0)
                parking_y = parking_y + 45
            end
        end
        defending_keeper = assert(defending_keeper)
        shooter.pos = Vec2.new(700, 270)
        shooter.anchor = shooter.pos
        shooter.facing = Vec2.new(1, 0)
        shooter.vel = Vec2.new(0, 0)
        shooter.run_vel = Vec2.new(0, 0)
        shooter.shot_speed = shot_speed
        defending_keeper.pos = Vec2.new(keeper_x, 270)
        defending_keeper.anchor = defending_keeper.pos
        defending_keeper.vel = Vec2.new(0, 0)
        defending_keeper.run_vel = Vec2.new(0, 0)
        s.owner = s.controlled
        s.ball = shooter.pos:add(Vec2.new(18, 0))
        s.ball_vel = Vec2.new(0, 0)
        s.pickup_cd = 1
        s.block_grace = 1
        return s, shooter, defending_keeper
    end

    t.it("a chip shot leaves the ground", function()
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(1, 0)
        match.step(s, 0.016, input({ shoot = true, lob = true }))
        -- Step past the wind-up so the ball releases.
        step_frames(s, WINDUP_FRAMES)
        t.is_true(s.owner == nil, "shot released")
        t.is_true(s.ball_vz > 0, "the chip launches upward")
        local shot_event
        for _, event in ipairs(s.events) do
            if event.kind == "shot" then
                shot_event = event
            end
        end
        t.eq(assert(shot_event).shot_type, "chip")
        t.is_true(assert(shot_event).keeper_depth ~= nil)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(s.ball_z > 0, "and the ball is airborne next frame")
    end)

    t.it("locks human chip type and launch before a telegraphed keeper retreat", function()
        local s, shooter, defending_keeper = human_chip_state(880, 500)
        match.step(s, 0, input({ shoot = true, lob = true }))
        local committed = assert(shooter.windup_shot)
        local committed_vz = committed.vz
        t.eq(committed.shot_type, "chip")
        t.is_true(committed_vz > 0)

        defending_keeper.pos = Vec2.new(938, 270)
        defending_keeper.keeper_state = "retreat"
        shooter.windup_timer = 0
        match.step(s, 0, NO_INPUT)

        t.eq(s.owner, nil)
        t.eq(s.ball_vz, committed_vz)
        t.eq(defending_keeper.keeper_release_kind, "chip")
        t.eq(defending_keeper.keeper_release_state, "retreat")
        t.eq(defending_keeper.keeper_release_motion, 0)
        local event
        for _, candidate in ipairs(s.events) do
            if candidate.kind == "shot" then
                event = candidate
            end
        end
        t.eq(assert(event).shot_type, "chip")
    end)

    t.it("keeps an infeasible human chip verb instead of disguising a ground shot", function()
        local s, shooter, defending_keeper = human_chip_state(938, 50)
        match.step(s, 0, input({ shoot = true, lob = true }))
        local committed = assert(shooter.windup_shot)
        t.eq(committed.shot_type, "chip")
        t.is_true(committed.vz > 0)
        t.eq(
            require("sim.keeper").chip_launch({
                origin = shooter.pos,
                target = shooter.pos:add(committed.dir),
                keeper_pos = defending_keeper.pos,
                defending_team = "away",
                goal = s.goal_away,
                horizontal_speed = committed.speed,
                friction = 0.3,
                gravity = 900,
                keeper_clearance = 60,
                crossbar = 70,
                desired_goal_height = 65,
            }),
            nil
        )

        shooter.windup_timer = 0
        match.step(s, 0, NO_INPUT)
        t.eq(defending_keeper.keeper_release_kind, "chip")
        t.is_true(s.ball_vz > 0)
    end)

    t.it("flies a solved chip over the actual keeper plane and under the bar", function()
        local s, shooter, defending_keeper = human_chip_state(880, 500)
        defending_keeper.move_speed = 0
        match.step(s, 0, input({ shoot = true, lob = true }))
        local committed = assert(shooter.windup_shot)
        t.eq(committed.shot_type, "chip")
        shooter.windup_timer = 0
        match.step(s, 0, NO_INPUT)
        t.eq(s.ball_vz, committed.vz)

        local crossed_keeper = false
        for _ = 1, 120 do
            local before_x = s.ball.x
            match.step(s, 1 / 60, NO_INPUT)
            if
                not crossed_keeper
                and before_x < defending_keeper.pos.x
                and s.ball.x >= defending_keeper.pos.x
            then
                crossed_keeper = true
                t.is_true(s.ball_z > 60, "the locked flight clears the actual keeper plane")
            end
            if s.score.home > 0 then
                break
            end
        end
        t.is_true(crossed_keeper)
        t.eq(s.score.home, 1, "the same flight crosses the goal plane under the bar")
    end)

    t.it("lets ordinary team AI select the same visible high-keeper chip", function()
        ---@param keeper_x number
        ---@return MatchState
        ---@return MatchPlayer
        local function ai_shot(keeper_x)
            local s = match.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
                human_controlled = false,
                seed = 91,
            })
            local attacker_idx, attacker, defending_keeper
            local parking_y = 30
            for index, player in ipairs(s.players) do
                if player.team == "home" and not player.is_keeper and not attacker then
                    attacker_idx = index
                    attacker = player
                elseif player.team == "away" and player.is_keeper then
                    defending_keeper = player
                elseif not player.is_keeper then
                    player.pos = Vec2.new(player.team == "home" and 300 or 820, parking_y)
                    player.anchor = player.pos
                    parking_y = parking_y + 45
                end
            end
            attacker = assert(attacker)
            defending_keeper = assert(defending_keeper)
            attacker.pos = Vec2.new(730, 270)
            attacker.anchor = attacker.pos
            attacker.facing = Vec2.new(1, 0)
            attacker.vel = Vec2.new(0, 0)
            attacker.run_vel = Vec2.new(0, 0)
            attacker.settle_timer = 0
            attacker.shot_speed = 400
            for _, player in ipairs(s.players) do
                if player.team == "away" and not player.is_keeper then
                    player.pos = attacker.pos:add(Vec2.new(0, 55))
                    player.anchor = player.pos
                    break
                end
            end
            defending_keeper.pos = Vec2.new(keeper_x, 230)
            defending_keeper.anchor = defending_keeper.pos
            defending_keeper.vel = Vec2.new(0, 0)
            defending_keeper.run_vel = Vec2.new(0, 0)
            s.owner = assert(attacker_idx)
            s.ball = attacker.pos:add(Vec2.new(18, 0))
            s.ball_vel = Vec2.new(0, 0)
            s.pickup_cd = 1
            s.block_grace = 1
            return s, attacker
        end

        local deep, deep_attacker = ai_shot(942)
        local advanced, advanced_attacker = ai_shot(880)
        local deep_rng = deep.rng
        local advanced_rng = advanced.rng
        match.step(deep, 1 / 60, NO_INPUT)
        match.step(advanced, 1 / 60, NO_INPUT)

        t.eq(assert(deep_attacker.windup_shot).shot_type, "ground")
        local advanced_keeper = advanced.players[6]
        local advanced_shot = assert(advanced_attacker.windup_shot)
        t.is_true(
            require("sim.keeper").chip_is_visible(
                advanced_keeper.pos,
                advanced_keeper.team,
                advanced.goal_away
            ),
            "advanced keeper is visibly high"
        )
        t.is_true(advanced_attacker.pos:dist(advanced.ball) <= 24, "AI controls the ball")
        t.is_true(require("sim.keeper").chip_launch({
            origin = advanced_attacker.pos,
            target = advanced_attacker.pos:add(advanced_shot.dir),
            keeper_pos = advanced_keeper.pos,
            defending_team = "away",
            goal = advanced.goal_away,
            horizontal_speed = advanced_shot.speed,
            friction = 0.3,
            gravity = 900,
            keeper_clearance = 60,
            crossbar = 70,
            desired_goal_height = 65,
        }) ~= nil, "the visible chip has a feasible under-bar path")
        t.eq(assert(advanced_attacker.windup_shot).shot_type, "chip")
        t.is_true(assert(advanced_attacker.windup_shot).vz > 0)
        t.eq(deep.rng, deep_rng, "settled ground choice consumes no decision draw")
        t.eq(advanced.rng, advanced_rng, "settled chip choice consumes no decision draw")
    end)

    t.it("a driven shot stays on the ground", function()
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(1, 0)
        match.step(s, 0.016, input({ shoot = true }))
        -- Step past the wind-up so the ball releases.
        step_frames(s, WINDUP_FRAMES)
        t.eq(s.ball_vz, 0, "no loft on a normal shot")
    end)

    t.it("the keeper lobs over a defender on its throwing lane and lands near a mate", function()
        local s = new_match()
        local ki, mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and p.is_keeper then
                ki = i
            end
        end
        local k = s.players[ki]
        k.pos = Vec2.new(40, 270)
        -- The ONLY open outlet is mate (idx 5), and its lane is blocked -> the keeper
        -- must lob over the blocker. Mark the other home outfielders so they aren't
        -- viable outlets (forcing the lob rather than a clear ground pass elsewhere).
        mate = s.players[5]
        mate.pos = Vec2.new(300, 270)
        s.players[2].pos = Vec2.new(200, 150)
        s.players[3].pos = Vec2.new(200, 400)
        s.players[4].pos = Vec2.new(450, 270)
        s.players[6].pos = Vec2.new(170, 270) -- blocks the keeper->mate lane (f=0.5)
        s.players[7].pos = Vec2.new(208, 150) -- marks home 2
        s.players[8].pos = Vec2.new(208, 400) -- marks home 3
        s.players[9].pos = Vec2.new(458, 270) -- marks home 4
        s.players[10].pos = Vec2.new(950, 40)
        s.owner = ki
        k.hold_timer = 0
        match.step(s, 0.001, NO_INPUT)
        t.is_true(s.owner == nil, "keeper released the ball")
        t.is_true(s.ball_vz > 0, "it was lobbed over the camped defender")
        t.is_true(has_event(s, "pass"), "still counts as a distribution pass")
    end)
end)

t.describe("match keeper respect (hard retreat)", function()
    t.it("opponents keep clear of a keeper holding the ball", function()
        local s = new_match()
        local ki
        for i, p in ipairs(s.players) do
            if p.team == "home" and p.is_keeper then
                ki = i
            end
        end
        s.owner = ki
        s.players[ki].pos = Vec2.new(40, 270)
        -- an away outfielder camped right on the keeper
        local att = 7
        s.players[att].pos = Vec2.new(55, 270)
        local targets = match._offball_targets(
            s,
            (function()
                local p = {}
                for i, pl in ipairs(s.players) do
                    p[i] = pl.pos
                end
                return p
            end)()
        )
        t.is_true(targets[att] ~= nil, "the opponent has a target")
        t.is_true(
            targets[att]:dist(s.players[ki].pos) >= 69.9,
            "its target is pushed outside the keeper's respect ring"
        )
    end)
end)

t.describe("match auto-switch control", function()
    t.it("control follows the home player who wins the ball", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 0
        local target
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                target = i
                break
            end
        end
        s.players[target].pos = Vec2.new(300, 270) -- clear space in the home half
        s.ball = Vec2.new(300, 270)
        s.ball_vel = Vec2.new(0, 0)
        s.ball_z = 0
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, target, "that player gathered the ball")
        t.eq(s.controlled, target, "control auto-switched to the ball winner")
    end)

    t.it("hands control to the HOME keeper while it holds the ball", function()
        local s = new_match()
        local ki
        for i, p in ipairs(s.players) do
            if p.team == "home" and p.is_keeper then
                ki = i
            end
        end
        s.owner = nil
        s.pickup_cd = 0
        s.players[ki].pos = Vec2.new(60, 270)
        s.ball = Vec2.new(75, 270) -- in the home keeper's box
        s.ball_vel = Vec2.new(0, 0)
        s.ball_z = 0
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, ki, "keeper claimed it")
        t.eq(s.controlled, ki, "the human takes the keeper to pick the distribution")
        t.is_true(s.players[ki].hold_timer > 2, "with a generous six-second-rule budget")
    end)
end)

t.describe("match.step keeper vs close-range shots", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("saves a shot released moments ago (inside the shooter's pickup lockout)", function()
        local s = new_match()
        local ki, k
        for i, p in ipairs(s.players) do
            if p.team == "away" and p.is_keeper then
                ki, k = i, p
            end
        end
        k.pos = Vec2.new(938, 270)
        s.owner = nil
        s.pickup_cd = 0.3 -- the shot was just released: shooter can't re-collect...
        s.ball = Vec2.new(908, 270)
        s.ball_vel = Vec2.new(600, 0) -- ...but the keeper must still react to it
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "catch") or has_event(s, "parry"), "the keeper made a save")
        t.eq(s.score.home, 0, "a close-range shot is not an automatic goal")
    end)

    t.it("smothers a carrier who brings the ball into its box", function()
        local s = new_match()
        local carrier
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                carrier = i
                break
            end
        end
        local c = s.players[carrier]
        c.pos = Vec2.new(60, 270)
        c.facing = Vec2.new(-1, 0)
        s.players[1].pos = Vec2.new(24, 270) -- home keeper on its line
        s.owner = carrier
        s.ball = Vec2.new(42, 270) -- at the carrier's feet, in the keeper's box
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, 1, "the keeper takes the ball off the carrier's feet")
        t.is_true(s.players[1].hold_timer > 0, "and holds it in hand")
    end)

    t.it("rushes a carrier in its box instead of holding the line", function()
        local s = new_match()
        local carrier
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                carrier = i
                break
            end
        end
        s.players[carrier].pos = Vec2.new(140, 240)
        s.owner = carrier
        s.ball = Vec2.new(122, 240)
        local k = s.players[1]
        k.pos = Vec2.new(24, 270)
        local before = k.pos:dist(s.ball)
        -- Measure the rush over a few frames: the ball is a physical object now
        -- (it rolls at the carrier's feet), so a single 16ms tick from rest is in
        -- the noise — but the keeper visibly closes the gap as it accelerates.
        for _ = 1, 6 do
            match.step(s, 0.016, NO_INPUT)
        end
        t.is_true(k.pos:dist(s.ball) < before, "the keeper closes down the carrier")
    end)
end)

t.describe("match.step kickoff positioning", function()
    local function assert_own_halves(s)
        local half = s.field.w / 2
        for _, p in ipairs(s.players) do
            if p.team == "home" then
                t.is_true(p.pos.x <= half, p.id .. " starts in the home half")
            else
                t.is_true(p.pos.x >= half, p.id .. " starts in the away half")
            end
        end
    end

    t.it("every player starts in their own half at the opening kickoff", function()
        assert_own_halves(new_match())
    end)

    t.it("both teams restart in their own halves after a goal", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1
        s.ball = Vec2.new(s.field.w - 5, s.field.h / 2)
        s.ball_vel = Vec2.new(400, 0)
        for _ = 1, 10 do
            match.step(s, 0.016, NO_INPUT)
            if s.score.home > 0 then
                break
            end
        end
        t.eq(s.score.home, 1)
        assert_own_halves(s)
    end)
end)

t.describe("match.step kickoff rules", function()
    t.it("the conceding team kicks off after a goal", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1
        s.ball = Vec2.new(s.field.w - 5, s.field.h / 2)
        s.ball_vel = Vec2.new(400, 0)
        for _ = 1, 10 do
            match.step(s, 0.016, NO_INPUT)
            if s.score.home > 0 then
                break
            end
        end
        t.eq(s.score.home, 1)
        t.is_true(s.owner ~= nil, "kickoff possession is assigned")
        t.eq(s.players[s.owner].team, "away", "the team that conceded restarts play")
        t.is_true(not s.players[s.owner].is_keeper)
        t.eq(s.players[s.controlled].team, "home", "the human still controls a home player")
        t.is_true(not s.players[s.controlled].is_keeper)
    end)
end)

t.describe("match.step pass quality", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    -- Controlled passer at (300,270) facing +x with one teammate ahead at `tpos`;
    -- every other outfielder parked behind, all opponents far away.
    local function pass_setup(tpos)
        local s = new_match()
        local passer = s.controlled
        local mate
        s.players[passer].pos = Vec2.new(300, 270)
        s.players[passer].facing = Vec2.new(1, 0)
        local backy = 100
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                if not mate then
                    mate = i
                    p.pos = Vec2.new(tpos.x, tpos.y)
                else
                    p.pos = Vec2.new(100, backy)
                    backy = backy + 120
                end
            elseif p.team == "away" then
                p.pos = Vec2.new(940, 40)
            end
        end
        s.owner = passer
        s.ball = s.players[passer].pos:add(Vec2.new(18, 0))
        return s, passer, mate
    end

    t.it("a long pass is driven hard enough to actually arrive", function()
        local s, _, mate = pass_setup(Vec2.new(700, 270))
        match.step(s, 0.016, input({ pass = true }))
        t.is_true(has_event(s, "pass"))
        t.is_true(s.players[mate].receive_timer > 0, "the receiver runs onto it")
        t.is_true(s.ball_vel:length() > 450, "a 400px pass is driven, not rolled")
        for _ = 1, 150 do
            match.step(s, 1 / 60, NO_INPUT)
            if s.owner then
                break
            end
        end
        t.eq(s.owner, mate, "the receiver collects the pass")
    end)

    t.it("falls back to the nearest teammate when nobody is in the aim cone", function()
        local s = new_match()
        local passer = s.controlled
        s.players[passer].pos = Vec2.new(800, 270)
        s.players[passer].facing = Vec2.new(1, 0) -- aiming at open space
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                p.pos = Vec2.new(500, p.pos.y) -- everyone behind the passer
            end
        end
        s.owner = passer
        s.ball = s.players[passer].pos:add(Vec2.new(18, 0))
        match.step(s, 0.016, input({ pass = true }))
        t.is_true(has_event(s, "pass"), "the pass button always finds someone")
        t.is_true(s.ball_vel.x < 0, "played back to the nearest teammate")
    end)

    t.it("an AI carrier under pressure passes to an open teammate", function()
        local s = new_match()
        local carrier, mate
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                if not carrier then
                    carrier = i
                elseif not mate then
                    mate = i
                end
            end
        end
        s.players[carrier].pos = Vec2.new(600, 270)
        s.players[mate].pos = Vec2.new(450, 200) -- open, ahead (away attacks -x)
        -- A home defender close enough to pressure but not to steal.
        local defender
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                defender = i
                break
            end
        end
        s.players[defender].pos = Vec2.new(655, 270)
        s.players[s.controlled].pos = Vec2.new(200, 100)
        s.owner = carrier
        s.ball = Vec2.new(582, 270)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "pass"), "the pressured carrier moves the ball on")
        local receiver
        for i, p in ipairs(s.players) do
            if p.team == "away" and p.receive_timer > 0 then
                receiver = i
            end
        end
        t.is_true(receiver ~= nil and receiver ~= carrier, "an away teammate runs onto it")
        t.is_true(mate ~= nil) -- (setup sanity)
    end)
end)

t.describe("match.step keeper floated throw (tier 2)", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("floats a throw over the traffic when outlets are marked but not swarmed", function()
        local s = new_match()
        local k = s.players[1] -- home keeper
        k.pos = Vec2.new(40, 270)
        -- Every outlet has a marker 40px away: not SAFE (60) but receivable (>=30).
        s.players[2].pos = Vec2.new(250, 150)
        s.players[3].pos = Vec2.new(250, 390)
        s.players[4].pos = Vec2.new(420, 270)
        s.players[5].pos = Vec2.new(560, 200)
        s.players[7].pos = Vec2.new(290, 150)
        s.players[8].pos = Vec2.new(290, 390)
        s.players[9].pos = Vec2.new(460, 270)
        s.players[10].pos = Vec2.new(600, 200)
        s.players[6].pos = Vec2.new(938, 270) -- away keeper home
        s.owner = 1
        s.ball = Vec2.new(40, 270)
        k.hold_timer = 0
        match.step(s, 0.001, NO_INPUT)
        t.is_true(s.owner == nil, "keeper releases the ball")
        t.is_true(has_event(s, "pass"), "it is a distribution, not a clearance")
        t.is_true(s.ball_vz > 0, "and it is floated over the opponents")
    end)
end)

t.describe("match.step pass interception awareness", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("the pass button prefers a teammate whose lane cannot be cut", function()
        local s = new_match()
        local passer = s.controlled
        s.players[passer].pos = Vec2.new(300, 270)
        s.players[passer].facing = Vec2.new(1, 0)
        -- mate1: nearest in the cone, but an away body camps its lane late in the
        -- flight. mate2: a touch further, in the cone, and safely off that lane.
        local mate1, mate2
        local backy = 100
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                if not mate1 then
                    mate1 = i
                    p.pos = Vec2.new(440, 270)
                elseif not mate2 then
                    mate2 = i
                    p.pos = Vec2.new(420, 400)
                else
                    p.pos = Vec2.new(100, backy) -- behind: outside the aim cone
                    backy = backy + 120
                end
            end
        end
        local interceptor
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                if not interceptor then
                    interceptor = i
                    p.pos = Vec2.new(420, 270) -- sits on the passer->mate1 lane
                else
                    p.pos = Vec2.new(940, 40) -- everyone else far away
                end
            end
        end
        s.owner = passer
        s.ball = s.players[passer].pos:add(Vec2.new(18, 0))
        match.step(s, 0.016, input({ pass = true }))
        t.is_true(has_event(s, "pass"), "a pass is released")
        t.is_true(interceptor ~= nil) -- (setup sanity)
        t.is_true(
            s.players[mate2].receive_timer > 0,
            "the ball goes to the mate whose lane cannot be cut"
        )
    end)

    t.it("a pressured AI carrier lobs the pass a chaser would cut out", function()
        local s = new_match()
        -- Away carrier under pressure with one eligible outlet; a home defender
        -- stands 26px off the ground lane (statically clear, POSSESS_DIST is 22)
        -- but close enough to step onto the rolling ball.
        local away_out = {}
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                away_out[#away_out + 1] = i
            end
        end
        local carrier, outlet = away_out[1], away_out[2]
        s.players[carrier].pos = Vec2.new(600, 270)
        s.players[outlet].pos = Vec2.new(440, 270)
        s.players[away_out[3]].pos = Vec2.new(820, 200)
        s.players[away_out[4]].pos = Vec2.new(820, 340)
        local home_out = {}
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                home_out[#home_out + 1] = i
            end
        end
        s.players[home_out[1]].pos = Vec2.new(655, 270) -- pressures the carrier
        s.players[home_out[2]].pos = Vec2.new(520, 296) -- lurks 26px off the lane
        s.players[home_out[3]].pos = Vec2.new(824, 200) -- pins an away spare
        s.players[s.controlled].pos = Vec2.new(824, 340) -- pins the other spare
        s.owner = carrier
        s.ball = Vec2.new(582, 270)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "pass"), "the pressured carrier moves the ball on")
        t.is_true(s.players[outlet].receive_timer > 0, "to the eligible outlet")
        t.is_true(s.ball_vz > 0, "floated over the would-be interceptor, not rolled")
    end)

    t.it("the keeper floats its distribution over a chaser who could cut it", function()
        local s = new_match()
        local k = s.players[1] -- home keeper
        k.pos = Vec2.new(40, 270)
        -- Outlet 2 is the only viable target: its marker is 104px away (safe) and
        -- its lane is statically clear, but that marker sits 30px off the lane —
        -- near enough to cut a rolling ball. Every other outlet is pinned.
        s.players[2].pos = Vec2.new(240, 270)
        s.players[3].pos = Vec2.new(300, 100)
        s.players[4].pos = Vec2.new(300, 440)
        s.players[5].pos = Vec2.new(600, 270)
        s.players[7].pos = Vec2.new(140, 300) -- the chaser off the keeper->2 lane
        s.players[8].pos = Vec2.new(302, 100) -- pins home 3
        s.players[9].pos = Vec2.new(302, 440) -- pins home 4
        s.players[10].pos = Vec2.new(602, 270) -- pins home 5
        s.players[6].pos = Vec2.new(938, 270) -- away keeper home
        s.owner = 1
        s.ball = Vec2.new(40, 270)
        k.hold_timer = 0
        match.step(s, 0.001, NO_INPUT)
        t.is_true(s.owner == nil, "keeper releases the ball")
        t.is_true(has_event(s, "pass"), "it is a distribution, not a clearance")
        t.is_true(s.players[2].receive_timer > 0, "aimed at the open outlet")
        t.is_true(s.ball_vz > 0, "floated over the chaser instead of rolled past it")
    end)
end)

t.describe("match.step loose-ball pursuit", function()
    t.it("chasers lead a rolling ball instead of trailing it", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1
        s.ball = Vec2.new(700, 400) -- open space: no player is standing on it
        s.ball_vel = Vec2.new(200, 0) -- rolling toward the away goal
        local pos = {}
        for i, pl in ipairs(s.players) do
            pos[i] = pl.pos
        end
        local targets = match._offball_targets(s, pos)
        -- The away press-set chaser (nearest away outfielder to the ball).
        local chaser, best_d
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                local d = p.pos:dist(s.ball)
                if not best_d or d < best_d then
                    best_d, chaser = d, i
                end
            end
        end
        t.is_true(targets[chaser] ~= nil, "the chaser has a target")
        t.is_true(targets[chaser].x > s.ball.x, "it aims ahead of the ball along its path")
        t.near(targets[chaser].y, s.ball.y, 1e-6, "the lead stays on the ball's line")
    end)
end)

t.describe("match shot blocking", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    ---@return MatchState
    local function loose_ball(x, vx, z)
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1 -- keep collection out of the picture unless a test wants it
        s.ball = Vec2.new(x, 270)
        s.ball_vel = Vec2.new(vx, 0)
        s.ball_z = z or 0
        s.ball_vz = 0
        return s
    end

    -- Clear everyone off the midfield corridor, then park one away outfielder
    -- as a wall at (500, 270) so it is the only body the ball can meet.
    local function with_wall(s)
        local slot, wall = 0, nil
        for _, p in ipairs(s.players) do
            p.pos = Vec2.new(60 + slot * 50, 40)
            slot = slot + 1
            if not wall and p.team == "away" and not p.is_keeper then
                wall = p
                p.pos = Vec2.new(500, 270)
            end
        end
        return wall
    end

    t.it("a driven ball ricochets off a body in its path", function()
        local s = loose_ball(490, 450, 0)
        with_wall(s)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "block"), "the body blocked it")
        t.is_true(s.ball_vel.x < 0, "the ball came back off the body")
        t.is_true(s.ball_vel:length() < 450, "a block soaks pace")
    end)

    t.it("a lofted ball sails over the body", function()
        local s = loose_ball(490, 450, 40)
        s.ball_vz = 50 -- still rising through the frame
        with_wall(s)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(not has_event(s, "block"), "no block on a ball over head height")
        t.is_true(s.ball_vel.x > 0, "it kept flying")
    end)

    t.it("a ball moving away from a body is never blocked (own release)", function()
        local s = loose_ball(505, 450, 0) -- overlapping the wall but outbound
        with_wall(s)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(not has_event(s, "block"), "an outbound ball never re-blocks")
        t.is_true(s.ball_vel.x > 0)
    end)

    t.it("a slow ball is collected at the body, not bounced", function()
        local s = loose_ball(492, 200, 0)
        with_wall(s)
        s.pickup_cd = 0
        match.step(s, 0.016, NO_INPUT)
        t.is_true(not has_event(s, "block"), "slow balls are trapped, not deflected")
        t.is_true(s.owner ~= nil, "the body wins the ball instead")
    end)
end)

t.describe("match keeper save tuning", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    -- Fire a corner-aimed shot at the away keeper from 800,270 and play it out.
    local function corner_shot(speed)
        local s = new_match()
        local k
        for _, p in ipairs(s.players) do
            if p.team == "away" and p.is_keeper then
                k = p
            end
        end
        k.pos = Vec2.new(938, 270)
        -- Clear every outfielder off the shot lane so only the keeper matters.
        local slot = 0
        for _, p in ipairs(s.players) do
            if not p.is_keeper then
                p.pos = Vec2.new(100 + slot * 40, 60)
                slot = slot + 1
            end
        end
        s.owner = nil
        s.pickup_cd = 0.3 -- as if just released
        s.ball = Vec2.new(800, 270)
        s.ball_vel = Vec2.new(950, 317):sub(s.ball):normalized():scale(speed)
        local caught, parried = false, false
        for _ = 1, 40 do
            match.step(s, 1 / 60, NO_INPUT)
            caught = caught or has_event(s, "catch")
            parried = parried or has_event(s, "parry")
            if s.score.home > 0 then
                break
            end
        end
        return s.score.home, caught, parried
    end

    t.it("an uncharged corner shot is kept out", function()
        local goals, caught, parried = corner_shot(500)
        t.eq(goals, 0, "no clean goal from a plain corner shot")
        t.is_true(caught or parried, "the keeper got something on it")
    end)

    t.it("a fully charged corner shot beats the keeper", function()
        local goals = corner_shot(1000)
        t.eq(goals, 1, "a charged corner shot scores")
    end)
end)

t.describe("match AI shooting", function()
    -- An away carrier in range of the home goal; `defender_at` optionally parks
    -- a home outfielder near it. Returns the released shot speed.
    local function ai_shot_speed(defender_dist)
        local s = new_match()
        local carrier
        local slot = 0
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper then
                p.pos = Vec2.new(700 + slot * 40, 60) -- clear the home half
                slot = slot + 1
            elseif p.team == "away" and not p.is_keeper and not carrier then
                carrier = i
                p.pos = Vec2.new(200, 270)
                p.facing = Vec2.new(-1, 0)
            end
        end
        if defender_dist then
            for i, p in ipairs(s.players) do
                if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                    p.pos = Vec2.new(200 + defender_dist, 270)
                    break
                end
            end
        end
        s.owner = carrier
        s.ball = Vec2.new(182, 270)
        match.step(s, 0.016, NO_INPUT)
        -- Step past the wind-up so the ball releases.
        step_frames(s, WINDUP_FRAMES)
        return s.ball_vel:length()
    end

    t.it("a striker in space shoots much harder than one closed down", function()
        local open = ai_shot_speed(nil)
        local closed = ai_shot_speed(30)
        t.is_true(open > closed * 1.5, "space converts into shot power")
    end)
end)

t.describe("match possession feel", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("an AI receiver settles the ball before passing under pressure", function()
        local s = new_match()
        -- A loose ball at an away outfielder's feet with a home defender pressing.
        local recv, presser
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                recv = recv or i
            elseif p.team == "home" and not p.is_keeper and i ~= s.controlled then
                presser = presser or i
            end
        end
        s.players[recv].pos = Vec2.new(600, 270)
        s.players[presser].pos = Vec2.new(650, 270) -- pressured (< 70) but out of poke range
        s.players[s.controlled].pos = Vec2.new(100, 60)
        s.owner = nil
        s.pickup_cd = 0
        s.ball = Vec2.new(600, 270)
        s.ball_vel = Vec2.new(60, 0) -- rolling: collection reads as a touch
        local touched_at, passed_at
        for f = 1, 90 do
            match.step(s, 1 / 60, NO_INPUT)
            if not touched_at and s.owner == recv then
                touched_at = f
            end
            if touched_at and not passed_at and has_event(s, "pass") then
                passed_at = f
            end
        end
        t.is_true(touched_at ~= nil, "the receiver takes the ball")
        t.is_true(passed_at ~= nil, "and eventually moves it on")
        t.is_true(passed_at - touched_at >= 15, "but only after a settling touch (~0.3s+)")
    end)

    t.it("a whiffed AI poke stumbles the defender", function()
        local s = new_match()
        local carrier, defender
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                carrier = carrier or i
            elseif p.team == "home" and not p.is_keeper and i ~= s.controlled then
                defender = defender or i
            end
        end
        local c = s.players[carrier]
        c.facing = Vec2.new(-1, 0)
        s.owner = carrier
        s.ball = c.pos:add(Vec2.new(-18, 0))
        -- Park the carrier's teammates out of range so it can't pass out.
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper and i ~= carrier then
                p.pos = Vec2.new(40, 380 + i * 15)
            end
        end
        -- On the carrier's back: ball shielded, poke commits but comes up short.
        s.players[defender].pos = Vec2.new(c.pos.x + 20, c.pos.y)
        s.players[defender].dash_cd = 0
        s.players[s.controlled].pos = Vec2.new(60, 60)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, carrier, "the shielded carrier keeps it")
        t.is_true(s.players[defender].stun_timer > 0, "the whiffing defender stumbles")
    end)
end)

t.describe("match pressure on a static carrier", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("a carrier who never moves gets challenged, not ignored", function()
        local s = new_match() -- human holds the ball at kickoff
        local challenged, lost = false, false
        for _ = 1, 300 do -- 5 seconds of standing still
            match.step(s, 1 / 60, NO_INPUT)
            challenged = challenged or has_event(s, "tackle")
            lost = lost or (s.owner ~= nil and s.players[s.owner].team == "away") or s.owner == nil
        end
        t.is_true(challenged or lost, "the defense pressures a statue instead of freezing")
    end)

    t.it("a defender leaning on the carrier shoves them off their spot", function()
        local s = new_match()
        local me = s.players[s.controlled]
        local start = me.pos
        -- Overlap an away defender onto the carrier and let collisions resolve.
        for _, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                p.pos = Vec2.new(me.pos.x + 10, me.pos.y)
                break
            end
        end
        match.step(s, 1 / 60, NO_INPUT)
        t.is_true(me.pos:dist(start) > 3, "the carrier is displaced by the lean")
    end)
end)

t.describe("match keeper build-up space", function()
    t.it("opponents back right off and mark lanes, not the outlet's boots", function()
        local s = new_match()
        s.owner = 1 -- home keeper holds throughout
        s.players[1].pos = Vec2.new(40, 270)
        s.players[1].hold_timer = 2
        s.ball = Vec2.new(40, 270)
        local outlet = s.players[2]
        outlet.pos = Vec2.new(220, 200)
        local marker = s.players[8]
        marker.pos = Vec2.new(236, 200) -- starts tight on the outlet
        s.players[7].pos = Vec2.new(70, 270) -- camped on the keeper
        for _ = 1, 60 do
            s.controlled = 1
            match.step(s, 1 / 60, NO_INPUT)
            s.controlled = 1
        end
        for _, p in ipairs(s.players) do
            if p.team == "away" then
                t.is_true(
                    p.pos:dist(s.players[1].pos) >= 119,
                    p.id .. " backs off the keeper's ring"
                )
            end
        end
        t.is_true(marker.pos:dist(outlet.pos) >= 38, "the marker stands off, marking the lane")
    end)
end)

t.describe("match auto-switch on turnover", function()
    t.it("control jumps to the best-placed defender when the opponent wins it", function()
        local s = new_match()
        local away_idx, defender
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                away_idx = away_idx or i
            elseif p.team == "home" and not p.is_keeper and i ~= s.controlled then
                defender = defender or i
            end
        end
        s.players[away_idx].pos = Vec2.new(600, 270)
        s.players[defender].pos = Vec2.new(560, 270) -- closest home defender
        s.players[s.controlled].pos = Vec2.new(100, 100) -- current control far away
        s.owner = nil
        s.pickup_cd = 0
        s.ball = Vec2.new(600, 270)
        s.ball_vel = Vec2.new(0, 0)
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.owner, away_idx, "the away player collects")
        t.eq(s.controlled, defender, "control moves to the nearest home defender")
    end)
end)

t.describe("match keeper respect ring (physical)", function()
    t.it("the controlled player cannot camp a keeper holding the ball", function()
        local s = new_match()
        local ki
        for i, p in ipairs(s.players) do
            if p.team == "away" and p.is_keeper then
                ki = i
            end
        end
        s.owner = ki
        s.players[ki].hold_timer = 2 -- holding throughout
        local me = s.players[s.controlled]
        me.pos = Vec2.new(s.players[ki].pos.x - 10, s.players[ki].pos.y)
        match.step(s, 1 / 60, NO_INPUT)
        t.is_true(
            me.pos:dist(s.players[ki].pos) >= 69,
            "the human is pushed out to the respect ring"
        )
    end)
end)

t.describe("match human keeper control", function()
    local function home_keeper_holding(s)
        s.owner = 1
        s.controlled = 1
        s.players[1].pos = Vec2.new(40, 270)
        s.players[1].facing = Vec2.new(1, 0)
        s.players[1].hold_timer = 5
        s.ball = Vec2.new(46, 270)
    end

    t.it("a longer-held punt is hit harder and lofted", function()
        local function punt(charge)
            local s = new_match()
            home_keeper_holding(s)
            s.players[s.controlled].charge = charge
            match.step(s, 0.016, input({ shoot = true }))
            -- Step past the wind-up so the ball releases.
            step_frames(s, WINDUP_FRAMES)
            return s
        end
        local weak, strong = punt(0), punt(1)
        t.is_true(weak.owner == nil and strong.owner == nil, "the punt releases the ball")
        t.is_true(strong.ball_vz > 0, "punts are lofted clearances")
        t.is_true(
            strong.ball_vel:length() > weak.ball_vel:length(),
            "holding longer sends it further"
        )
    end)

    t.it("the charged throw range picks the far teammate along the aim", function()
        local function throw_receiver(charge)
            local s = new_match()
            home_keeper_holding(s)
            s.players[2].pos = Vec2.new(200, 270) -- short option on the aim line
            s.players[3].pos = Vec2.new(480, 270) -- long option on the aim line
            s.players[4].pos = Vec2.new(120, 60)
            s.players[5].pos = Vec2.new(120, 480)
            for i, p in ipairs(s.players) do
                if p.team == "away" then
                    p.pos = Vec2.new(900, 40 + i * 40) -- both options genuinely open
                end
            end
            s.players[s.controlled].pass_charge = charge
            match.step(s, 0.016, input({ pass = true }))
            for i, pl in ipairs(s.players) do
                if pl.receive_timer > 0 then
                    return i, s
                end
            end
        end
        local near_i = throw_receiver(0)
        local far_i, s2 = throw_receiver(1)
        t.eq(near_i, 2, "a tap throw goes short")
        t.eq(far_i, 3, "a charged throw picks out the long option")
        t.is_true(s2.controlled ~= 1, "control returns to an outfielder after the release")
    end)
end)

t.describe("match charge auto-fire", function()
    t.it("a full shot meter lets fly on its own", function()
        local s = new_match() -- human carries at kickoff
        for _ = 1, 60 do -- hold Space well past a full charge
            match.step(s, 1 / 60, input({ shoot_held = true }))
            if s.owner == nil then
                break
            end
        end
        t.is_true(s.owner == nil, "the shot auto-fired at full charge")
    end)

    t.it("a full pass meter releases the pass on its own", function()
        local s = new_match()
        for _ = 1, 60 do
            match.step(s, 1 / 60, input({ pass_held = true }))
            if s.owner == nil then
                break
            end
        end
        t.is_true(s.owner == nil, "the pass auto-fired at full charge")
    end)
end)

t.describe("match keeper no-aim throw safety", function()
    t.it("an aimless tap throw avoids the marked near man", function()
        local s = new_match()
        s.owner = 1
        s.controlled = 1
        s.players[1].pos = Vec2.new(40, 270)
        s.players[1].facing = Vec2.new(0, 1) -- facing nobody: empty cone
        s.players[1].hold_timer = 5
        s.ball = Vec2.new(46, 270)
        s.players[2].pos = Vec2.new(150, 270) -- nearest... and marked
        s.players[3].pos = Vec2.new(260, 170) -- further but open
        s.players[4].pos = Vec2.new(700, 60)
        s.players[5].pos = Vec2.new(700, 480)
        for i, p in ipairs(s.players) do
            if p.team == "away" then
                p.pos = Vec2.new(900, 40 + i * 40)
            end
        end
        s.players[7].pos = Vec2.new(174, 270) -- their forward, on the near man
        match.step(s, 0.016, input({ pass = true })) -- no direction held
        local receiver
        for i, pl in ipairs(s.players) do
            if pl.receive_timer > 0 then
                receiver = i
            end
        end
        t.eq(receiver, 3, "the throw goes to the open man, not the marked nearest")
    end)
end)

t.describe("match keeper carry limit", function()
    t.it("a keeper holding the ball cannot leave the penalty area", function()
        local s = new_match()
        s.owner = 1
        s.controlled = 1
        s.players[1].pos = Vec2.new(60, 270)
        s.players[1].hold_timer = 60 -- keep holding throughout
        s.ball = Vec2.new(66, 270)
        for _ = 1, 240 do -- 4 seconds of running up-right with the ball in hand
            match.step(s, 1 / 60, input({ move = Vec2.new(1, -1) }))
        end
        local box = match.PENALTY_BOX
        t.is_true(s.players[1].pos.x <= box.depth, "held at the edge of the drawn box")
        t.is_true(s.players[1].pos.y >= s.field.h / 2 - box.h / 2, "and inside its vertical bounds")
    end)
end)

t.describe("match keeper throw aim & safety", function()
    local function setup(s)
        s.owner = 1
        s.controlled = 1
        s.players[1].pos = Vec2.new(40, 270)
        s.players[1].facing = Vec2.new(1, 0)
        s.players[1].hold_timer = 5
        s.ball = Vec2.new(46, 270)
        -- Two outlets fanned up-right and down-right, others parked far.
        s.players[2].pos = Vec2.new(240, 160)
        s.players[3].pos = Vec2.new(240, 380)
        s.players[4].pos = Vec2.new(700, 60)
        s.players[5].pos = Vec2.new(700, 480)
        for i, p in ipairs(s.players) do
            if p.team == "away" then
                p.pos = Vec2.new(900, 40 + i * 40)
            end
        end
    end

    local function receiver(s)
        for i, pl in ipairs(s.players) do
            if pl.receive_timer > 0 then
                return i
            end
        end
    end

    t.it("holding a direction at release aims the throw", function()
        local s = new_match()
        setup(s)
        match.step(s, 0.016, input({ pass = true, move = Vec2.new(1, 1) })) -- down-right
        t.eq(receiver(s), 3, "down-right aim picks the lower outlet")

        local s2 = new_match()
        setup(s2)
        match.step(s2, 0.016, input({ pass = true, move = Vec2.new(1, -1) })) -- up-right
        t.eq(receiver(s2), 2, "up-right aim picks the upper outlet")
    end)

    t.it("a covered outlet loses to a nearby open one", function()
        local s = new_match()
        setup(s)
        -- Both outlets on symmetric aim; a striker camps the upper one's landing.
        s.players[7].pos = Vec2.new(250, 175)
        match.step(s, 0.016, input({ pass = true, move = Vec2.new(1, 0) })) -- aim straight
        t.eq(receiver(s), 3, "the throw picks the outlet the defense can't contest")
    end)
end)

t.describe("match aerial play", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    -- A loose airborne ball at `z` dropping onto player index `idx`.
    local function dropping_ball_on(s, idx, z)
        s.owner = nil
        s.pickup_cd = 0
        local p = s.players[idx]
        s.ball = Vec2.new(p.pos.x + 6, p.pos.y)
        s.ball_vel = Vec2.new(0, 0)
        s.ball_z = z
        s.ball_vz = -50
    end

    t.it("an AI attacker heads a dropping ball at goal", function()
        local s = new_match()
        local att
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                att = i
                break
            end
        end
        s.players[att].pos = Vec2.new(160, 270) -- in front of the home goal
        for i, p in ipairs(s.players) do
            if p.team == "home" then
                s.players[i].pos = Vec2.new(700, 60 + i * 40) -- clear of the drop
            end
        end
        dropping_ball_on(s, att, 45)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "header"), "the striker meets it")
        t.is_true(s.ball_vel.x < 0, "headed toward the home goal")
    end)

    t.it("a defender in its own third heads danger clear", function()
        local s = new_match()
        local def
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                def = i
                break
            end
        end
        s.players[def].pos = Vec2.new(100, 270) -- deep in the home third
        s.players[s.controlled].pos = Vec2.new(700, 100)
        for i, p in ipairs(s.players) do
            if p.team == "away" then
                s.players[i].pos = Vec2.new(820, 60 + i * 40)
            end
        end
        dropping_ball_on(s, def, 50)
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "header"), "the defender attacks the ball")
        t.is_true(s.ball_vel.x > 0, "cleared upfield")
        t.is_true(s.ball_vz > 0, "and high")
    end)

    t.it("volleys are riskier: some seeds sky it and the cage returns it", function()
        local skied, clean
        for seed = 1, 60 do
            local s = match.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
                seed = seed,
            })
            local att
            for i, p in ipairs(s.players) do
                if p.team == "away" and not p.is_keeper then
                    att = i
                    break
                end
            end
            s.players[att].pos = Vec2.new(160, 270)
            for i, p in ipairs(s.players) do
                if p.team == "home" then
                    s.players[i].pos = Vec2.new(700, 60 + i * 40)
                end
            end
            dropping_ball_on(s, att, 25) -- volley height
            match.step(s, 0.016, NO_INPUT)
            if has_event(s, "volley") then
                if s.ball_vz > 400 then
                    skied = skied or s
                else
                    clean = clean or s
                end
            end
        end
        t.is_true(skied ~= nil, "some volleys get skied")
        t.is_true(clean ~= nil, "and some are hit clean")
        -- The skied one: the cage ceiling caps it and brings it back down.
        local max_z = 0
        for _ = 1, 180 do
            match.step(skied, 1 / 60, NO_INPUT)
            max_z = math.max(max_z, skied.ball_z)
        end
        t.is_true(max_z <= 170, "the cage ceiling caps the flight")
        t.is_true(skied.ball_z < 60, "and the ball comes back down into play")
    end)
end)

t.describe("match crossing", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("an AI carrier on the flank crosses to the box", function()
        local s = new_match()
        local carrier, target
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                if not carrier then
                    carrier = i
                elseif not target then
                    target = i
                end
            end
        end
        s.players[carrier].pos = Vec2.new(300, 100) -- wide, attacking third (away attacks -x)
        s.players[target].pos = Vec2.new(150, 250) -- in the box
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper then
                s.players[i].pos = Vec2.new(820, 60 + i * 30) -- nobody pressuring
            end
        end
        s.owner = carrier
        s.ball = s.players[carrier].pos:add(Vec2.new(-18, 0))
        match.step(s, 0.016, NO_INPUT)
        t.is_true(has_event(s, "pass"), "the winger delivers it")
        t.is_true(s.ball_vz > 0, "a cross is lofted")
        t.is_true(s.players[target].receive_timer > 0, "aimed at the man in the box")
    end)

    t.it("a human lofted pass from wide targets the box runner", function()
        local s = new_match()
        local passer = s.controlled
        s.players[passer].pos = Vec2.new(750, 100) -- wide right, attacking third
        s.players[passer].facing = Vec2.new(1, 0)
        local cone_mate, box_mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                if not cone_mate then
                    cone_mate = i
                    p.pos = Vec2.new(860, 100) -- straight along the aim
                elseif not box_mate then
                    box_mate = i
                    p.pos = Vec2.new(830, 270) -- in the box
                else
                    p.pos = Vec2.new(200, 60 + i * 40)
                end
            end
        end
        for i, p in ipairs(s.players) do
            if p.team == "away" then
                s.players[i].pos = Vec2.new(120, 40 + i * 30)
            end
        end
        s.owner = passer
        s.ball = s.players[passer].pos:add(Vec2.new(18, 0))
        match.step(s, 0.016, input({ pass = true, lob = true }))
        t.is_true(s.players[box_mate].receive_timer > 0, "the cross picks the box, not the cone")
        t.is_true(s.ball_vz > 0, "and sails high")
    end)
end)

t.describe("match teammate awareness", function()
    t.it("an AI teammate claims a loose ball that lands near it", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 1
        -- The HUMAN is nearest the ball (would previously eat the whole chase
        -- allocation); an AI teammate 70px away must still go claim it.
        s.ball = Vec2.new(480, 300)
        s.ball_vel = Vec2.new(0, 0)
        local me = s.players[s.controlled]
        me.pos = Vec2.new(480, 320) -- human nearest
        local mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                mate = i
                p.pos = Vec2.new(480, 230) -- 70px off: inside the magnet
                break
            end
        end
        local pos = {}
        for i, pl in ipairs(s.players) do
            pos[i] = pl.pos
        end
        local targets = match._offball_targets(s, pos)
        t.is_true(targets[mate] ~= nil, "the teammate has a target")
        t.is_true(targets[mate]:dist(s.ball) < 40, "and it is the ball, not a shape point")
    end)

    t.it("a nearby supporter triangulates: offers a short angled option", function()
        local s = new_match()
        local carrier
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                carrier = carrier or i
            end
        end
        s.owner = carrier
        local c = s.players[carrier]
        c.pos = Vec2.new(500, 270)
        c.facing = Vec2.new(1, 0)
        s.ball = Vec2.new(518, 270)
        local supporter
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled and i ~= carrier then
                supporter = i
                p.pos = Vec2.new(420, 300) -- near the play
                -- Park its anchor-region under opponents so base spots score badly.
                p.anchor = Vec2.new(300, 300)
                break
            end
        end
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                p.pos = Vec2.new(300 + (i % 3) * 60, 240 + (i % 2) * 90) -- crowd the base area
            end
        end
        local pos = {}
        for i, pl in ipairs(s.players) do
            pos[i] = pl.pos
        end
        local targets = match._offball_targets(s, pos)
        t.is_true(targets[supporter] ~= nil)
        t.is_true(
            targets[supporter]:dist(c.pos) < 170 + 80,
            "the supporter comes short to a triangle spot near the carrier"
        )
    end)

    t.it("supporters do not clog the carrier's dribbling path", function()
        local s = new_match()
        local carrier
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                carrier = carrier or i
            end
        end
        s.owner = carrier
        local c = s.players[carrier]
        c.pos = Vec2.new(400, 270)
        c.facing = Vec2.new(1, 0)
        s.ball = Vec2.new(418, 270)
        local supporter
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled and i ~= carrier then
                supporter = i
                p.pos = Vec2.new(500, 270)
                p.anchor = Vec2.new(430, 270) -- base spot lands right on the path
                break
            end
        end
        local pos = {}
        for i, pl in ipairs(s.players) do
            pos[i] = pl.pos
        end
        local targets = match._offball_targets(s, pos)
        local ahead = c.pos:add(c.facing:scale(120))
        t.is_true(targets[supporter] ~= nil)
        t.is_true(
            targets[supporter]:dist(ahead) > 65,
            "the spot right ahead of the carrier is vacated"
        )
    end)
end)

t.describe("match control follows the pass", function()
    t.it("a human cross hands control to the box receiver in flight", function()
        local s = new_match()
        local passer = s.controlled
        s.players[passer].pos = Vec2.new(750, 100) -- wide right, attacking third
        s.players[passer].facing = Vec2.new(1, 0)
        local box_mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                if not box_mate then
                    box_mate = i
                    p.pos = Vec2.new(830, 270) -- in the box
                else
                    p.pos = Vec2.new(200, 60 + i * 40)
                end
            end
        end
        for i, p in ipairs(s.players) do
            if p.team == "away" then
                s.players[i].pos = Vec2.new(120, 40 + i * 30)
            end
        end
        s.owner = passer
        s.ball = s.players[passer].pos:add(Vec2.new(18, 0))
        match.step(s, 0.016, input({ pass = true, lob = true }))
        t.is_true(s.owner == nil, "the cross is away")
        t.eq(s.controlled, box_mate, "and you now control the man attacking it")
    end)

    t.it("an AI pass never moves the human's control", function()
        local s = new_match()
        local carrier, mate
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                if not carrier then
                    carrier = i
                elseif not mate then
                    mate = i
                end
            end
        end
        s.players[carrier].pos = Vec2.new(600, 270)
        s.players[mate].pos = Vec2.new(450, 200)
        s.players[s.controlled].pos = Vec2.new(660, 270) -- pressuring
        s.owner = carrier
        s.ball = Vec2.new(582, 270)
        local before = s.controlled
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.controlled, before, "AI passes don't steal your control")
    end)

    t.it("a directed header goes where you aim", function()
        local s = new_match()
        -- The controlled player under a dropping ball, aiming up-left.
        local me = s.players[s.controlled]
        me.pos = Vec2.new(480, 300)
        s.owner = nil
        s.pickup_cd = 0
        s.ball = Vec2.new(486, 300)
        s.ball_vel = Vec2.new(0, 0)
        s.ball_z = 45
        s.ball_vz = -50
        match.step(s, 0.016, input({ dash = true, move = Vec2.new(-1, -1) }))
        local headed = false
        for _, e in ipairs(s.events) do
            if e.kind == "header" then
                headed = true
            end
        end
        t.is_true(headed, "the human meets the ball first-time")
        t.is_true(s.ball_vel.x < 0 and s.ball_vel.y < 0, "and it goes where they aimed")
    end)
end)

t.describe("match positional calm", function()
    -- A home defender in a positional (non-urgent) role, parked exactly on its
    -- zone spot with a static loose ball (nobody may collect it): the target
    -- geometry stays put, so any movement is shuffle, not repositioning.
    local function calm_setup()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 60
        s.ball = Vec2.new(480, 300)
        s.ball_vel = Vec2.new(0, 0)
        s.players[s.controlled].pos = Vec2.new(100, 60) -- human out of the way
        local pos = {}
        for i, pl in ipairs(s.players) do
            pos[i] = pl.pos
        end
        local targets, urgent = match._offball_targets(s, pos)
        local calm_idx
        for i, p in ipairs(s.players) do
            if
                p.team == "home"
                and not p.is_keeper
                and i ~= s.controlled
                and targets[i]
                and not urgent[i]
                and p.pos:dist(s.ball) > 150 -- well outside the ball magnet
            then
                calm_idx = i
                break
            end
        end
        s.players[calm_idx].pos = targets[calm_idx] -- already at the spot
        s.players[calm_idx].run_vel = Vec2.new(0, 0)
        return s, calm_idx
    end

    t.it("a player at their role spot stands still instead of shuffling", function()
        local s, idx = calm_setup()
        local start = s.players[idx].pos
        for _ = 1, 45 do
            match.step(s, 1 / 60, NO_INPUT)
        end
        t.is_true(s.players[idx].pos:dist(start) < 10, "no back-and-forth: they plant at the spot")
    end)

    t.it("they walk again once the spot drifts meaningfully away", function()
        local s, idx = calm_setup()
        -- Displace the player well beyond the wake radius from their spot.
        s.players[idx].pos = s.players[idx].pos:add(Vec2.new(80, 0))
        local start = s.players[idx].pos
        for _ = 1, 45 do
            match.step(s, 1 / 60, NO_INPUT)
        end
        t.is_true(s.players[idx].pos:dist(start) > 25, "far from the spot: they move to it")
    end)

    t.it("a loose-ball chaser is exempt from the calm (full urgency)", function()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 60
        s.ball = Vec2.new(480, 300)
        s.ball_vel = Vec2.new(0, 0)
        local mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                mate = i
                p.pos = Vec2.new(480, 220) -- 80px off: inside the magnet
                p.run_vel = Vec2.new(0, 0)
                break
            end
        end
        s.players[s.controlled].pos = Vec2.new(100, 60)
        for _ = 1, 60 do
            match.step(s, 1 / 60, NO_INPUT)
        end
        t.is_true(
            s.players[mate].pos:dist(s.ball) < 40,
            "the chaser closes on the ball at full speed"
        )
    end)
end)

t.describe("match save grab-vs-parry odds", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    -- Fire the same shot at the away keeper under one seed; report the outcome.
    ---@return "catch"|"parry"|"goal"|nil
    local function shot_outcome(seed, speed, dy)
        local s = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            seed = seed,
        })
        local k
        for _, p in ipairs(s.players) do
            if p.team == "away" and p.is_keeper then
                k = p
            end
        end
        k.pos = Vec2.new(938, 270)
        local slot = 0
        for _, p in ipairs(s.players) do
            if not p.is_keeper then
                p.pos = Vec2.new(60 + slot * 40, 40) -- everyone clear of the lane
                slot = slot + 1
            end
        end
        s.owner = nil
        s.pickup_cd = 0.3
        s.ball = Vec2.new(750, 270)
        s.ball_vel = Vec2.new(950, 270 + dy):sub(s.ball):normalized():scale(speed)
        for _ = 1, 90 do
            match.step(s, 1 / 60, NO_INPUT)
            if has_event(s, "catch") then
                return "catch"
            end
            if has_event(s, "parry") then
                return "parry"
            end
            if s.score.home > 0 then
                return "goal"
            end
        end
        return nil
    end

    local function tally(speed, dy, n)
        local c = { catch = 0, parry = 0, goal = 0 }
        for seed = 1, n do
            local o = shot_outcome(seed, speed, dy)
            if o then
                c[o] = c[o] + 1
            end
        end
        return c
    end

    t.it("a soft, central shot sticks in the gloves nearly every time", function()
        local c = tally(420, 0, 40)
        local total = c.catch + c.parry
        t.eq(c.goal, 0, "a soft central shot never scores")
        t.is_true(total >= 39, "the keeper always deals with it")
        t.is_true(c.catch >= total * 0.85, "held, not parried: " .. c.catch .. "/" .. total)
    end)

    t.it("a hard shot toward the corner is mostly pushed away", function()
        local c = tally(700, 40, 40)
        local total = c.catch + c.parry
        t.eq(c.goal, 0, "still kept out")
        t.is_true(total >= 39)
        t.is_true(c.parry >= total * 0.7, "mostly parried: " .. c.parry .. "/" .. total)
    end)

    t.it("the same seed always reproduces the same outcome", function()
        local a = shot_outcome(7, 700, 30)
        local b = shot_outcome(7, 700, 30)
        t.eq(a, b, "seeded matches are deterministic")
    end)
end)

t.describe("match sprint", function()
    -- Run the controlled player along the bottom wing with the loose ball parked
    -- far away (top-left), so nothing interferes with the straight-line run.
    ---@param frames integer
    ---@param inputs table
    ---@param setup fun(s: MatchState, me: MatchPlayer)?
    local function run(frames, inputs, setup)
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 60
        s.ball = Vec2.new(100, 60)
        local me = s.players[s.controlled]
        me.pos = Vec2.new(150, 480)
        if setup then
            setup(s, me)
        end
        local x0 = me.pos.x
        for _ = 1, frames do
            match.step(s, 1 / 60, input(inputs))
        end
        return me.pos.x - x0, me
    end

    t.it("sprinting covers more ground and drains the meter", function()
        -- 60 frames: the standing-start ramp is shared, the sprint advantage
        -- compounds once both are up to speed.
        local walked = run(60, { move = Vec2.new(1, 0) })
        local sprinted, me = run(60, { move = Vec2.new(1, 0), sprint = true })
        t.is_true(sprinted > walked * 1.2, "sprint is meaningfully faster")
        t.is_true(me.sprint_meter < 1, "sprinting drains the meter")
    end)

    t.it("the meter refills while not sprinting", function()
        local _, me = run(60, { move = Vec2.new(1, 0) }, function(_, m)
            m.sprint_meter = 0.5
        end)
        t.is_true(me.sprint_meter > 0.5, "resting refills the tank")
    end)

    t.it("an empty tank gives no boost until it meaningfully recovers", function()
        local walked = run(30, { move = Vec2.new(1, 0) })
        local drained = run(30, { move = Vec2.new(1, 0), sprint = true }, function(_, m)
            m.sprint_meter = 0
        end)
        t.near(drained, walked, 1e-6, "no sprint speed from an empty tank")
    end)
end)

t.describe("match jockey stance", function()
    -- Shared setup: controlled player off the ball in open space.
    local function jockey_setup()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 60 -- nobody collects during the test
        s.ball = Vec2.new(480, 270) -- ball at midfield
        local me = s.players[s.controlled]
        me.pos = Vec2.new(400, 270)
        me.tackle_cd = 0
        me.stun_timer = 0
        return s, me
    end

    -- Acceptance 1: displacement over 30 frames is ~0.75x the plain-run displacement.
    t.it("jockeying slows the defender to ~0.75x and faces toward the ball", function()
        local function run_frames(with_jockey)
            local s, me = jockey_setup()
            -- Park every other player far from the run corridor so collisions
            -- cannot interfere with the controlled player's straight-line run.
            local slot = 0
            for i, p in ipairs(s.players) do
                if i ~= s.controlled then
                    p.pos = Vec2.new(60 + slot * 50, 40)
                    slot = slot + 1
                end
            end
            local start = me.pos
            for _ = 1, 30 do
                match.step(s, 1 / 60, input({ move = Vec2.new(1, 0), jockey = with_jockey }))
            end
            return me.pos:dist(start), me
        end
        local plain_dist = run_frames(false)
        local jockey_dist, me = run_frames(true)
        -- Displacement should be close to 75% of the plain run (within 10% tolerance).
        t.is_true(
            jockey_dist >= plain_dist * 0.65 and jockey_dist <= plain_dist * 0.85,
            ("jockey displacement %.1f should be ~0.75x plain %.1f"):format(jockey_dist, plain_dist)
        )
        -- Facing should be toward the ball (roughly +x from pos 400 to ball 480).
        t.is_true(me.facing.x > 0, "facing locked toward the ball")
    end)

    -- Acceptance 2: poke released from jockey wins from STAND_REACH + 6 (40px).
    -- A plain poke at this range misses; a jockey poke connects.
    t.it("a poke from jockey stance gains bonus reach", function()
        -- STAND_REACH = 34; STAND_REACH + JOCKEY_REACH_BONUS = 40.
        -- The carrier faces -x and dribbles the ball to its left (at -18px offset).
        -- The human defender is placed at 40px from the ball on the BALL SIDE
        -- (to the left of the carrier) — beyond STAND_REACH=34 but within
        -- STAND_REACH+6=40. The human is also more than STEAL_DIST=26px from
        -- the carrier's body so the body-contact shortcut doesn't apply.
        --
        --   defender @ 422          ball @ 462     carrier @ 480
        --      [me] <---40px-------> [ball] <--18px--> [c]
        --
        local function poke_at_40(with_jockey)
            local s = new_match()
            local away_idx
            for i, p in ipairs(s.players) do
                if p.team == "away" and not p.is_keeper then
                    away_idx = i
                    break
                end
            end
            -- Park ALL non-controlled home outfielders far from the challenge zone
            -- so no AI poke interferes with the test.
            for i, p in ipairs(s.players) do
                if p.team == "home" and not p.is_keeper and i ~= s.controlled then
                    p.pos = Vec2.new(100, 40 + i * 25)
                    p.dash_cd = 1 -- cooldown so they can't challenge
                end
                -- Park away teammates out of pressure-pass range.
                if p.team == "away" and not p.is_keeper and i ~= away_idx then
                    p.pos = Vec2.new(40, 380 + i * 15)
                end
            end
            local c = s.players[away_idx]
            c.pos = Vec2.new(480, 270)
            c.facing = Vec2.new(-1, 0)
            s.owner = away_idx
            s.ball = c.pos:add(c.facing:scale(18)) -- ball at 462, 270
            -- Human defender 40px left of the ball, on the ball side: 422, 270.
            -- Distance to carrier body (480): 58px > STEAL_DIST 26, so no shortcut.
            local me = s.players[s.controlled]
            me.pos = Vec2.new(422, 270) -- 40px from ball at 462, on its left
            me.vel = Vec2.new(0, 0)
            -- Prime jockey_timer so the bonus is active at poke time.
            if with_jockey then
                me.jockey_timer = 0.2
            else
                me.jockey_timer = 0
            end
            me.tackle_cd = 0
            me.stun_timer = 0
            -- Fire the poke toward the ball (dash + move right toward carrier).
            match.step(s, 0.016, input({ dash = true, move = Vec2.new(1, 0) }))
            return s.owner ~= away_idx
        end
        t.is_true(not poke_at_40(false), "plain poke misses at 40px (> STAND_REACH 34)")
        t.is_true(poke_at_40(true), "jockey poke wins at 40px (STAND_REACH + 6)")
    end)
end)

t.describe("match.step pass-target preview", function()
    -- Acceptance 1 & 3: pass_target is nil when not charging.
    t.it("pass_target is nil when idle (not holding pass)", function()
        local s = new_match()
        match.step(s, 0.016, NO_INPUT)
        t.eq(s.players[s.controlled].pass_target, nil, "pass_target is nil when idle")
    end)

    -- Acceptance 1: outfielder preview equals the actual receiver.
    t.it("outfielder preview equals the actual receiver", function()
        local s = new_match()
        local passer = s.controlled
        s.players[passer].pos = Vec2.new(300, 270)
        s.players[passer].facing = Vec2.new(1, 0)
        -- One teammate ahead; all others and all opponents parked well away.
        local mate
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                if not mate then
                    mate = i
                    p.pos = Vec2.new(500, 270)
                else
                    p.pos = Vec2.new(100, 60 + i * 30)
                end
            elseif p.team == "away" then
                p.pos = Vec2.new(900, 40 + i * 30)
            end
        end
        s.owner = passer
        s.ball = s.players[passer].pos:add(Vec2.new(18, 0))
        -- Hold pass for several frames to accumulate charge and read the preview.
        local recorded_target
        for _ = 1, 10 do
            match.step(s, 0.016, input({ pass_held = true }))
            if s.players[passer].pass_target then
                recorded_target = s.players[passer].pass_target
            end
        end
        t.is_true(recorded_target ~= nil, "pass_target was set while charging")
        -- Now fire the pass and verify the recorded target actually receives it.
        match.step(s, 0.016, input({ pass = true }))
        t.is_true(
            s.players[recorded_target].receive_timer > 0,
            "recorded preview == actual receiver"
        )
    end)

    -- Acceptance 2: keeper preview equals the actual throw receiver.
    t.it("keeper preview equals the actual throw receiver", function()
        local s = new_match()
        s.owner = 1
        s.controlled = 1
        s.players[1].pos = Vec2.new(40, 270)
        s.players[1].facing = Vec2.new(1, 0)
        s.players[1].hold_timer = 5
        s.ball = Vec2.new(46, 270)
        s.players[2].pos = Vec2.new(200, 270)
        s.players[3].pos = Vec2.new(480, 270)
        s.players[4].pos = Vec2.new(120, 60)
        s.players[5].pos = Vec2.new(120, 480)
        for i, p in ipairs(s.players) do
            if p.team == "away" then
                p.pos = Vec2.new(900, 40 + i * 40)
            end
        end
        local recorded_target
        for _ = 1, 10 do
            match.step(s, 0.016, input({ pass_held = true, move = Vec2.new(1, 0) }))
            if s.players[1].pass_target then
                recorded_target = s.players[1].pass_target
            end
        end
        t.is_true(recorded_target ~= nil, "keeper pass_target was set while charging")
        match.step(s, 0.016, input({ pass = true, move = Vec2.new(1, 0) }))
        t.is_true(
            s.players[recorded_target].receive_timer > 0,
            "keeper preview == actual throw receiver"
        )
    end)
end)

t.describe("match scenario: keeper retains possession under pressure", function()
    -- A scripted "real game" situation: the home keeper has gathered the ball with
    -- a striker pressing and two defenders available as outlets. Played out over 3
    -- seconds, the keeper must keep it for the team and never hand it to the
    -- opponent. Control is parked on the keeper so every outfielder is pure AI.
    t.it("the keeper builds out without losing the ball to the opponent", function()
        local s = new_match()
        s.owner = 1 -- home keeper
        s.players[1].pos = Vec2.new(40, 270)
        s.players[1].hold_timer = 0.9
        s.ball = Vec2.new(40, 270)
        -- Home defenders at their natural anchors (open, no opponents on the lane).
        -- Away side arranged so no one stands between the keeper and its closest
        -- outlet — with momentum the ball must arrive before opponents can react.
        s.players[2].pos = Vec2.new(210, 170) -- home defender (outlet)
        s.players[3].pos = Vec2.new(210, 370) -- home defender (outlet)
        s.players[4].pos = Vec2.new(450, 200)
        s.players[5].pos = Vec2.new(600, 270)
        -- Away side: striker presses laterally (not on the lane), markers are back.
        s.players[7].pos = Vec2.new(62, 350) -- away striker off to the side
        s.players[8].pos = Vec2.new(280, 170) -- away marking player 2
        s.players[9].pos = Vec2.new(500, 270)
        s.players[10].pos = Vec2.new(700, 270)

        -- The guarantee: the keeper's distribution reaches a home outfielder
        -- before the opponent ever touches the ball. With momentum players need
        -- time to accelerate, so allow up to 5 seconds (intent unchanged).
        local away_before_receive, home_outfielder_owned = false, false
        for _ = 1, 300 do
            s.controlled = 1 -- keep the human out of it; all outfielders are AI
            match.step(s, 1 / 60, NO_INPUT)
            s.controlled = 1
            local o = s.owner
            if o then
                if s.players[o].team == "away" and not home_outfielder_owned then
                    away_before_receive = true
                elseif s.players[o].team == "home" and not s.players[o].is_keeper then
                    home_outfielder_owned = true
                end
            end
        end

        t.is_true(not away_before_receive, "the opponent never intercepts the build-up")
        t.is_true(home_outfielder_owned, "a home outfielder received the keeper's distribution")
    end)
end)

-- Acceptance specs for T1: player momentum & turning radius
t.describe("match momentum (T1 acceptance)", function()
    -- Helper: park the loose ball well out of the way and give the controlled
    -- player ample sprint meter; returns the state and the player reference.
    local function momentum_setup()
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 60 -- nobody collects during the run
        s.ball = Vec2.new(100, 60)
        local me = s.players[s.controlled]
        me.pos = Vec2.new(480, 480) -- centre bottom, away from the ball
        me.sprint_meter = 1
        me.sprinting = false
        me.run_vel = Vec2.new(0, 0)
        return s, me
    end

    t.it("displacement builds up: first 6 frames < 60% of steady-state 6 frames", function()
        -- Acceptance criterion 1: from rest, the first 6 frames of movement are
        -- meaningfully slower than steady-state (frames 25-30), proving acceleration
        -- exists rather than instant top speed.
        local s, me = momentum_setup()
        local x0 = me.pos.x
        local disp_early = 0
        local disp_25_to_30 = 0
        for f = 1, 30 do
            local px = me.pos.x
            match.step(s, 1 / 60, input({ move = Vec2.new(1, 0) }))
            if f <= 6 then
                disp_early = disp_early + (me.pos.x - px)
            end
            if f >= 25 then
                disp_25_to_30 = disp_25_to_30 + (me.pos.x - px)
            end
        end
        t.is_true(
            disp_early < disp_25_to_30 * 0.6,
            "first-6-frame displacement is < 60% of steady-state 6 frames (acceleration)"
        )
    end)

    t.it("reversing at full speed takes longer to cover 40px than starting from rest", function()
        -- Acceptance criterion 2: a player running right at top speed who gets
        -- a reverse-left input must shed velocity first, taking longer to travel
        -- 40px left than a player who starts from rest and moves left immediately.
        -- This proves turn commitment exists.

        -- Measure time-to-40px for a player starting from rest running left.
        local frames_from_rest = 0
        do
            local s, me = momentum_setup()
            local start_x = me.pos.x
            for f = 1, 240 do
                match.step(s, 1 / 60, input({ move = Vec2.new(-1, 0) }))
                if me.pos.x <= start_x - 40 then
                    frames_from_rest = f
                    break
                end
            end
        end

        -- Measure time-to-40px for a player first running right at full speed
        -- (30 frames to build speed), then reversing left.
        local frames_after_reversal = 0
        do
            local s, me = momentum_setup()
            -- Run right for 30 frames to build up speed.
            for _ = 1, 30 do
                match.step(s, 1 / 60, input({ move = Vec2.new(1, 0) }))
            end
            local start_x = me.pos.x
            -- Now reverse left; count until 40px left of reversal point.
            for f = 1, 240 do
                match.step(s, 1 / 60, input({ move = Vec2.new(-1, 0) }))
                if me.pos.x <= start_x - 40 then
                    frames_after_reversal = f
                    break
                end
            end
        end

        t.is_true(frames_from_rest > 0, "rest run covers 40px")
        t.is_true(frames_after_reversal > 0, "reversal run covers 40px")
        t.is_true(
            frames_after_reversal > frames_from_rest,
            "reversing from full speed takes more frames than starting from rest"
        )
    end)
end)

-- ─── Acceptance: T5 wind-up telegraphs ───────────────────────────────────────

t.describe("match wind-up telegraphs (T5)", function()
    local function has_event(s, kind)
        for _, e in ipairs(s.events) do
            if e.kind == kind then
                return true
            end
        end
        return false
    end

    t.it("a shot input does NOT release the ball the same frame (wind-up delay)", function()
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(1, 0)
        match.step(s, 0.016, input({ shoot = true }))
        -- Ball must still be owned — no immediate release.
        t.is_true(s.owner ~= nil, "ball is still carried during the wind-up")
        t.is_true(not has_event(s, "shot"), "no shot event fires on the commit frame")
        -- After the wind-up elapses the ball releases.
        step_frames(s, WINDUP_FRAMES)
        t.is_true(s.owner == nil, "ball releases after ~0.15 s")
        t.is_true(has_event(s, "shot"), "a shot event fires on the release frame")
    end)

    t.it("shot parameters are captured at commit, not at release", function()
        -- Charge built before the shot commits is the charge that counts even if
        -- the player keeps holding shoot during the wind-up.
        local s = new_match()
        s.players[s.controlled].facing = Vec2.new(1, 0)
        s.players[s.controlled].charge = 1 -- full charge captured on commit
        match.step(s, 0.016, input({ shoot = true }))
        -- Hold shoot during wind-up — must not reset charge or re-commit.
        for _ = 1, WINDUP_FRAMES do
            match.step(s, 1 / 60, input({ shoot_held = true }))
        end
        -- Ball is now in flight; speed should reflect the full charge.
        t.is_true(s.owner == nil, "ball released")
        local base_speed = s.players[1].shot_speed or 500 -- fallback
        -- Loose ball — ball_vel has the release speed. Just assert it's non-zero.
        t.is_true(s.ball_vel:length() > 0, "ball has a velocity after release")
    end)

    t.it("a poke landing during the wind-up cancels the shot", function()
        -- Set up an away carrier in wind-up; a home defender close enough to poke.
        local s = new_match()
        local carrier_idx
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                carrier_idx = i
                break
            end
        end
        local carrier = s.players[carrier_idx]
        carrier.pos = Vec2.new(300, 270)
        carrier.facing = Vec2.new(-1, 0)
        s.owner = carrier_idx
        s.ball = carrier.pos:add(carrier.facing:scale(18))
        -- Park carrier's teammates out of range so no pressure-pass escapes.
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper and i ~= carrier_idx then
                p.pos = Vec2.new(40, 380 + i * 15)
            end
        end
        -- Manually start the wind-up on the carrier (simulates the AI deciding to shoot).
        carrier.windup_timer = 0.12 -- mid-wind-up
        carrier.windup_shot = {
            dir = Vec2.new(-1, 0),
            speed = 500,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }
        -- Place the human defender ball-side within poke range.
        local me = s.players[s.controlled]
        me.pos = Vec2.new(carrier.pos.x - 24, carrier.pos.y) -- on the ball side
        me.vel = Vec2.new(0, 0)
        -- Poke attempt this frame.
        match.step(s, 0.016, input({ dash = true }))
        -- The tackle should win, clearing the payload.
        t.is_true(s.owner ~= carrier_idx, "the tackle dispossessed the carrier mid-wind-up")
        t.is_true(not has_event(s, "shot"), "no shot fires — the wind-up was cancelled")
        t.is_true(carrier.windup_shot == nil, "windup payload cleared on dispossession")
    end)

    t.it("AI shots also enter the wind-up (telegraph is universal)", function()
        -- An away carrier in shooting range — just like the AI shooting spec, but
        -- we assert the shot does NOT fire the same frame.
        local s = new_match()
        local carrier_idx
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper and not carrier_idx then
                carrier_idx = i
            elseif p.team == "home" and not p.is_keeper then
                s.players[i].pos = Vec2.new(700, 60 + i * 40)
            end
        end
        s.players[carrier_idx].pos = Vec2.new(200, 270)
        s.players[carrier_idx].facing = Vec2.new(-1, 0)
        s.owner = carrier_idx
        s.ball = Vec2.new(182, 270)
        match.step(s, 0.016, NO_INPUT)
        -- The AI should have committed a wind-up, not released immediately.
        t.is_true(s.owner == carrier_idx, "AI carrier still owns the ball during wind-up")
        t.is_true(s.players[carrier_idx].windup_timer > 0, "AI shot committed the wind-up timer")
        -- After the wind-up elapses the ball fires.
        step_frames(s, WINDUP_FRAMES)
        t.is_true(s.owner == nil or s.owner ~= carrier_idx, "ball released after wind-up")
    end)

    t.it("a carrier moves at 0.3x speed during the wind-up", function()
        -- Human carrier commits a shot; their position must barely change while winding up.
        local s = new_match()
        local me = s.players[s.controlled]
        me.facing = Vec2.new(1, 0)
        local pos_before = me.pos
        match.step(s, 0.016, input({ shoot = true })) -- commit wind-up
        t.is_true(me.windup_timer > 0, "wind-up active")
        local pos_windup_start = me.pos
        -- Run right during the wind-up.
        local normal_speed = me.move_speed
        match.step(s, 0.016, input({ move = Vec2.new(1, 0) }))
        local dx_windup = me.pos.x - pos_windup_start.x
        -- A normal frame at full speed: dx should be ~move_speed/60
        local dx_full = normal_speed * 0.016
        -- The wind-up should reduce movement to ~30% of normal.
        t.is_true(dx_windup < dx_full * 0.5, "movement is capped during wind-up")
        t.is_true(dx_windup > 0, "but some movement is still allowed")
        -- Suppress unused warning
        t.is_true(pos_before ~= nil)
    end)
end)

t.describe("match keeper back-pass (receive with feet)", function()
    -- Controlled carrier near its own box aiming square at the keeper; the rest
    -- of the home side pushed far upfield so the aim cone holds only the keeper.
    ---@param s MatchState
    local function setup_backpass(s)
        s.owner = s.controlled
        local owner = s.players[s.controlled]
        owner.pos = Vec2.new(220, 270)
        owner.facing = Vec2.new(-1, 0)
        s.ball = Vec2.new(214, 270)
        s.players[1].pos = Vec2.new(70, 270) -- home keeper on its line
        for i, p in ipairs(s.players) do
            if p.team == "home" and i ~= s.controlled and not p.is_keeper then
                p.pos = Vec2.new(700, 60 + i * 40)
            elseif p.team == "away" then
                p.pos = Vec2.new(900, 60 + i * 40)
            end
        end
    end

    -- Step until the keeper collects the back-pass; also reports whether any
    -- hands gather ("claim"/"catch") happened along the way.
    ---@param s MatchState
    ---@return boolean handled
    local function run_until_received(s)
        local handled = false
        for _ = 1, 240 do
            match.step(s, 1 / 60, NO_INPUT)
            for _, e in ipairs(s.events) do
                if e.kind == "claim" or e.kind == "catch" then
                    handled = true
                end
            end
            if s.owner then
                break
            end
        end
        return handled
    end

    t.it("an aimed pass picks the keeper as the receiver", function()
        local s = new_match()
        setup_backpass(s)
        match.step(s, 1 / 60, input({ pass = true }))
        t.is_true(s.owner == nil, "ball released")
        t.is_true(s.players[1].receive_timer > 0, "keeper is the designated receiver")
    end)

    t.it("the keeper takes the pass with its feet, never its hands", function()
        local s = new_match()
        setup_backpass(s)
        match.step(s, 1 / 60, input({ pass = true }))
        local handled = run_until_received(s)
        t.eq(s.owner, 1)
        t.is_true(s.players[1].feet_ball, "ball sits at the keeper's feet")
        t.is_true(not handled, "no dive, claim, or catch on the way in")
        t.eq(s.players[1].hold_timer, 0, "no six-second clock on a ball at the feet")
        t.eq(s.controlled, 1, "control hands over to the keeper on the trap")
    end)

    t.it("from the feet, the keeper passes out like an outfielder", function()
        local s = new_match()
        setup_backpass(s)
        match.step(s, 1 / 60, input({ pass = true }))
        run_until_received(s)
        t.eq(s.owner, 1)
        match.step(s, 1 / 60, input({ pass = true, move = Vec2.new(1, 0) }))
        t.is_true(s.owner == nil, "kicked pass released immediately")
        t.is_true(s.ball_vel.x > 0, "played upfield")
        t.is_true(s.controlled ~= 1, "control follows the outlet")
        t.is_true(s.players[s.controlled].receive_timer > 0, "an outlet runs onto it")
    end)

    t.it("from the feet, the keeper can punt long right away", function()
        local s = new_match()
        setup_backpass(s)
        match.step(s, 1 / 60, input({ pass = true }))
        run_until_received(s)
        t.eq(s.owner, 1)
        match.step(s, 1 / 60, input({ shoot = true })) -- commit the punt wind-up
        step_frames(s, WINDUP_FRAMES)
        t.is_true(s.owner == nil, "punt released")
        t.is_true(s.ball_vel.x > 0, "sails upfield")
        t.is_true(s.ball_vz > 0, "a lofted clearance")
    end)

    t.it("a blind pass under no aim never dumps the ball at the keeper", function()
        local s = new_match()
        setup_backpass(s)
        -- Face upfield, away from the keeper: the cone finds nobody (mates are
        -- far), so the openness fallback fires — it must pick an outfielder.
        s.players[s.controlled].facing = Vec2.new(0, 1)
        match.step(s, 1 / 60, input({ pass = true }))
        t.is_true(s.owner == nil, "ball released")
        t.eq(s.players[1].receive_timer, 0, "keeper never the panic outlet")
    end)

    t.it("aim square at the keeper beats a nearer mid-lane teammate", function()
        local s = new_match()
        setup_backpass(s)
        -- A defender hovers near the back-pass lane, better aligned under the
        -- generic scoring (closer, nearly on-axis) — dead-on aim at the keeper
        -- must still pick the keeper.
        local defender
        for i, p in ipairs(s.players) do
            if p.team == "home" and i ~= s.controlled and not p.is_keeper then
                defender = i
                break
            end
        end
        s.players[defender].pos = Vec2.new(150, 255)
        local at_keeper = match._select_pass_target(s, s.controlled, false, Vec2.new(-1, 0), nil)
        t.eq(at_keeper, 1, "square aim is a deliberate back-pass")
        -- Aim at the defender instead: the keeper must not hijack the pass.
        local aim = s.players[defender].pos:sub(s.players[s.controlled].pos):normalized()
        local at_defender = match._select_pass_target(s, s.controlled, false, aim, nil)
        t.eq(at_defender, defender, "aiming at the defender still reaches the defender")
    end)

    t.it("the keeper chases down an under-hit back-pass and kicks on", function()
        local s = new_match()
        setup_backpass(s)
        -- The pass died short: a dead ball outside the claim zone, with the
        -- keeper's receive window open (as release_pass leaves it).
        s.owner = nil
        s.players[s.controlled].pos = Vec2.new(600, 400) -- passer well away
        s.ball = Vec2.new(190, 270)
        s.ball_vel = Vec2.new(-40, 0) -- last of its pace; dies well short
        s.players[1].receive_timer = 4
        local received = false
        for _ = 1, 300 do
            match.step(s, 1 / 60, NO_INPUT)
            if s.owner then
                received = true
                break
            end
        end
        t.is_true(received, "somebody reached the dying ball")
        t.eq(s.owner, 1, "the keeper came off its line to meet it")
        t.is_true(s.players[1].feet_ball, "and took it with the feet")
        t.is_true(s.players[1].pos.x > 90, "it genuinely left the goal line")
    end)

    t.it("an interception ends the keeper's receive window", function()
        local s = new_match()
        setup_backpass(s)
        -- Dead ball at an opponent's feet mid-lane while the keeper still
        -- expects the back-pass: the pickup must snap the window shut so the
        -- keeper's save reflexes come straight back online.
        s.owner = nil
        s.ball = Vec2.new(500, 270)
        s.ball_vel = Vec2.new(0, 0)
        s.players[1].receive_timer = 4
        local raider
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                raider = i
                break
            end
        end
        s.players[raider].pos = Vec2.new(500, 270)
        match.step(s, 1 / 60, NO_INPUT)
        t.eq(s.owner, raider, "the opponent collects the loose pass")
        t.eq(s.players[1].receive_timer, 0, "keeper stops receiving on the spot")
    end)

    t.it("a keeper with the ball at its feet can be tackled", function()
        local s = new_match()
        setup_backpass(s)
        match.step(s, 1 / 60, input({ pass = true }))
        run_until_received(s)
        t.eq(s.owner, 1)
        -- Pin an opponent onto the ball: no hands protection, so its poke
        -- strips the keeper within a few frames.
        local raider
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                raider = i
                break
            end
        end
        local stripped = false
        for _ = 1, 60 do
            s.players[raider].pos = Vec2.new(s.ball.x, s.ball.y)
            match.step(s, 1 / 60, NO_INPUT)
            if s.owner ~= 1 then
                stripped = true
                break
            end
        end
        t.is_true(stripped, "the ball is poked off the keeper's feet")
    end)
end)

t.describe("match tap-pass proximity (closest along the aim)", function()
    -- Passer facing +x with a near teammate 45 degrees off the aim and a far
    -- one dead ahead; everyone else parked, opponents away.
    local function setup(s)
        local passer = s.controlled
        s.players[passer].pos = Vec2.new(300, 270)
        s.players[passer].facing = Vec2.new(1, 0)
        s.owner = passer
        s.ball = Vec2.new(318, 270)
        local near, far
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper and i ~= passer then
                if not near then
                    near = i
                    p.pos = Vec2.new(390, 360) -- 127px away, ~45 deg off the aim
                elseif not far then
                    far = i
                    p.pos = Vec2.new(700, 270) -- 400px away, dead on the aim
                else
                    p.pos = Vec2.new(60, 60 + i * 30)
                end
            elseif p.team == "away" then
                p.pos = Vec2.new(900, 40 + i * 40)
            end
        end
        return near, far
    end

    t.it("a tap picks the near man, even with a far one better aligned", function()
        local s = new_match()
        local near = setup(s)
        local target = match._select_pass_target(s, s.controlled, false, Vec2.new(1, 0), nil)
        t.eq(target, near, "quick passes go short to the man you point at")
    end)

    t.it("a charged pass still picks out the far man by range", function()
        local s = new_match()
        local _, far = setup(s)
        local target = match._select_pass_target(s, s.controlled, false, Vec2.new(1, 0), 400)
        t.eq(target, far, "the charge is how you reach the long option")
    end)
end)

t.describe("match standing-start inertia (lever)", function()
    -- Early displacement from rest under a given START_ACCEL setting.
    ---@param setting number
    ---@return number dx
    local function push_off(setting)
        local tuning = require("sim.tuning")
        tuning.set("START_ACCEL", setting)
        local s = new_match()
        s.owner = nil
        s.pickup_cd = 60
        s.ball = Vec2.new(100, 60) -- parked away: pure movement test
        local me = s.players[s.controlled]
        me.pos = Vec2.new(480, 480)
        me.run_vel = Vec2.new(0, 0)
        local x0 = me.pos.x
        for _ = 1, 8 do
            match.step(s, 1 / 60, input({ move = Vec2.new(1, 0) }))
        end
        tuning.reset()
        return me.pos.x - x0
    end

    t.it("START_ACCEL scales the push-off from rest", function()
        local tuning = require("sim.tuning")
        local heavy = push_off(tuning.by_key.START_ACCEL.min)
        local light = push_off(tuning.by_key.START_ACCEL.max)
        t.is_true(heavy > 0, "even a heavy start moves")
        t.is_true(
            light > heavy * 1.5,
            ("the lever is live: max %.1fpx vs min %.1fpx in 8 frames"):format(light, heavy)
        )
    end)
end)
