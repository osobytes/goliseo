local t = require("spec.support.runner")
local keeper = require("sim.keeper")
local Vec2 = require("core.vec2")

local HOME_GOAL = { x = -30, y = 215, w = 30, h = 110 }
local AWAY_GOAL = { x = 960, y = 215, w = 30, h = 110 }

---@param ball_pos Vec2
---@param aggression number?
---@param in_1v1 boolean?
---@return KeeperPositionContext
local function home_context(ball_pos, aggression, in_1v1)
    return {
        keeper_pos = Vec2.new(12, 270),
        ball_pos = ball_pos,
        goal = HOME_GOAL,
        team = "home",
        aggression = aggression or 80,
        in_1v1 = in_1v1 or false,
    }
end

---@param ball_pos Vec2
---@param aggression number?
---@param in_1v1 boolean?
---@return KeeperPositionContext
local function away_context(ball_pos, aggression, in_1v1)
    return {
        keeper_pos = Vec2.new(948, 270),
        ball_pos = ball_pos,
        goal = AWAY_GOAL,
        team = "away",
        aggression = aggression or 80,
        in_1v1 = in_1v1 or false,
    }
end

t.describe("keeper.arc_target", function()
    t.it("moves monotonically off the home goal line as the ball approaches", function()
        local midfield = keeper.arc_target(home_context(Vec2.new(480, 270)))
        local approaching = keeper.arc_target(home_context(Vec2.new(320, 270)))
        local claim_edge = keeper.arc_target(home_context(Vec2.new(160, 270)))

        t.near(midfield.x, 0)
        t.is_true(approaching.x > midfield.x)
        t.is_true(claim_edge.x > approaching.x)
        t.near(claim_edge.x, 80)
    end)

    t.it("mirrors the approaching arc for the away goal", function()
        local midfield = keeper.arc_target(away_context(Vec2.new(480, 270)))
        local approaching = keeper.arc_target(away_context(Vec2.new(640, 270)))
        local claim_edge = keeper.arc_target(away_context(Vec2.new(800, 270)))

        t.near(midfield.x, 960)
        t.is_true(approaching.x < midfield.x)
        t.is_true(claim_edge.x < approaching.x)
        t.near(claim_edge.x, 880)
    end)

    t.it("holds deep and centred while the ball is beyond midfield", function()
        local home = keeper.arc_target(home_context(Vec2.new(700, 400)))
        local away = keeper.arc_target(away_context(Vec2.new(260, 140)))

        t.near(home.x, 0)
        t.near(home.y, 270)
        t.near(away.x, 960)
        t.near(away.y, 270)
    end)

    t.it("keeps one-on-one targets deep at and beyond midfield in both directions", function()
        local home_boundary = keeper.arc_target(home_context(Vec2.new(480, 400), 80, true))
        local home_beyond = keeper.arc_target(home_context(Vec2.new(700, 400), 80, true))
        local away_boundary = keeper.arc_target(away_context(Vec2.new(480, 140), 80, true))
        local away_beyond = keeper.arc_target(away_context(Vec2.new(260, 140), 80, true))

        t.near(home_boundary.x, 0)
        t.near(home_boundary.y, 270)
        t.near(home_beyond.x, 0)
        t.near(home_beyond.y, 270)
        t.near(away_boundary.x, 960)
        t.near(away_boundary.y, 270)
        t.near(away_beyond.x, 960)
        t.near(away_beyond.y, 270)
    end)

    t.it("clamps lateral targets to the 28-pixel guard on both sides", function()
        local low = keeper.arc_target(home_context(Vec2.new(160, 540), 120))
        local high = keeper.arc_target(away_context(Vec2.new(800, 0), 120))

        t.near(low.y, 298)
        t.near(high.y, 242)
    end)

    t.it("uses maximum aggression depth for a one-on-one", function()
        local normal_home = keeper.arc_target(home_context(Vec2.new(320, 270), 80))
        local one_on_one_home = keeper.arc_target(home_context(Vec2.new(320, 270), 80, true))
        local normal_away = keeper.arc_target(away_context(Vec2.new(640, 270), 80))
        local one_on_one_away = keeper.arc_target(away_context(Vec2.new(640, 270), 80, true))

        t.near(normal_home.x, 40)
        t.near(one_on_one_home.x, 80)
        t.near(normal_away.x, 920)
        t.near(one_on_one_away.x, 880)
    end)

    t.it("never exceeds aggression along the goal-to-ball ray", function()
        local home = home_context(Vec2.new(160, 500), 65, true)
        local away = away_context(Vec2.new(800, 40), 65, true)
        local home_target = keeper.arc_target(home)
        local away_target = keeper.arc_target(away)
        local home_center = Vec2.new(0, 270)
        local away_center = Vec2.new(960, 270)

        t.is_true(home_target:dist(home_center) <= 65)
        t.is_true(away_target:dist(away_center) <= 65)
    end)

    t.it("uses keeper position as a deterministic fallback for a degenerate ray", function()
        local context = home_context(Vec2.new(0, 270), 60, true)
        context.keeper_pos = Vec2.new(20, 270)
        local target = keeper.arc_target(context)

        t.near(target.x, 60)
        t.near(target.y, 270)
    end)
end)

t.describe("keeper.base_target", function()
    t.it("uses shallow mirrored dynamic depth as the ball approaches", function()
        local home_midfield = keeper.base_target(home_context(Vec2.new(480, 270), 80))
        local home_approaching = keeper.base_target(home_context(Vec2.new(320, 270), 80))
        local home_claim_edge = keeper.base_target(home_context(Vec2.new(160, 270), 80))
        local away_midfield = keeper.base_target(away_context(Vec2.new(480, 270), 80))
        local away_approaching = keeper.base_target(away_context(Vec2.new(640, 270), 80))
        local away_claim_edge = keeper.base_target(away_context(Vec2.new(800, 270), 80))

        t.near(home_midfield.x, 12)
        t.near(home_approaching.x, 15)
        t.near(home_claim_edge.x, 18)
        t.near(away_midfield.x, 948)
        t.near(away_approaching.x, 945)
        t.near(away_claim_edge.x, 942)
    end)

    t.it("stays deep and central beyond midfield", function()
        local home = keeper.base_target(home_context(Vec2.new(700, 500), 120))
        local away = keeper.base_target(away_context(Vec2.new(260, 40), 120))

        t.near(home.x, 12)
        t.near(home.y, 270)
        t.near(away.x, 948)
        t.near(away.y, 270)
    end)

    t.it("retains the deliberate lateral corner concession", function()
        local home = keeper.base_target(home_context(Vec2.new(160, 540), 120))
        local away = keeper.base_target(away_context(Vec2.new(800, 0), 120))

        t.near(home.y, 310)
        t.near(away.y, 230)
        t.is_true(home.x > 12 and home.x <= 18)
        t.is_true(away.x < 948 and away.x >= 942)
    end)
end)

t.describe("keeper.save_style", function()
    t.it("leaves the 26-pixel smother boundary to the claim branch", function()
        t.is_true(keeper.in_smother_range(26))
        t.is_true(not keeper.in_smother_range(26.000001))
        local ok = pcall(keeper.save_style, 26, 0, 100)
        t.is_true(not ok)
        t.eq(keeper.save_style(26.000001, 100, 100), "spread")
    end)

    t.it("keeps the 78-pixel boundary in the spread style", function()
        t.eq(keeper.save_style(78, 100, 100), "spread")
        t.eq(keeper.save_style(78.000001, 40, 100), "central")
    end)

    t.it("includes exactly 40 percent of reach in the central style", function()
        t.eq(keeper.save_style(100, 40, 100), "central")
        t.eq(keeper.save_style(100, 40.000001, 100), "stretch")
    end)
end)

t.describe("keeper.commit_lead", function()
    t.it("clamps anticipation and negative windups to safe bounds", function()
        t.near(keeper.commit_lead(-1, 2), 0)
        t.near(keeper.commit_lead(0.5, 2), 1)
        t.near(keeper.commit_lead(2, 2), 2)
        t.near(keeper.commit_lead(0.5, -2), 0)
    end)

    t.it("is monotonic in anticipation across the supported range", function()
        local previous = keeper.commit_lead(0, 0.3)
        for anticipation = 0.1, 1, 0.1 do
            local current = keeper.commit_lead(anticipation, 0.3)
            t.is_true(current >= previous)
            t.is_true(current >= 0 and current <= 0.3)
            previous = current
        end
    end)
end)

t.describe("keeper early-set eligibility", function()
    local home_goal = { x = -30, y = 215, w = 30, h = 110 }
    local away_goal = { x = 960, y = 215, w = 30, h = 110 }

    t.it("projects captured directions into either defending goal mouth", function()
        t.is_true(keeper.shot_targets_goal({
            defending_team = "away",
            shooter_team = "home",
            origin = Vec2.new(700, 250),
            direction = Vec2.new(260, 20),
            goal = away_goal,
        }))
        t.is_true(keeper.shot_targets_goal({
            defending_team = "home",
            shooter_team = "away",
            origin = Vec2.new(260, 290),
            direction = Vec2.new(-260, -20),
            goal = home_goal,
        }))
    end)

    t.it("rejects teammates, backwards shots, and projections outside the mouth", function()
        t.is_true(not keeper.shot_targets_goal({
            defending_team = "away",
            shooter_team = "away",
            origin = Vec2.new(700, 270),
            direction = Vec2.new(260, 0),
            goal = away_goal,
        }))
        t.is_true(not keeper.shot_targets_goal({
            defending_team = "away",
            shooter_team = "home",
            origin = Vec2.new(700, 270),
            direction = Vec2.new(-260, 0),
            goal = away_goal,
        }))
        t.is_true(not keeper.shot_targets_goal({
            defending_team = "away",
            shooter_team = "home",
            origin = Vec2.new(700, 270),
            direction = Vec2.new(260, 80),
            goal = away_goal,
        }))
    end)

    t.it("bounds set timing by the captured wind-up without making zero reactive early", function()
        local context = {
            defending_team = "away",
            shooter_team = "home",
            origin = Vec2.new(700, 270),
            direction = Vec2.new(260, 0),
            goal = away_goal,
            anticipation = 0,
            windup_duration = 0.15,
            windup_remaining = 0.000001,
        }
        t.is_true(not keeper.should_set(context))

        context.anticipation = 0.5
        context.windup_remaining = 0.075001
        t.is_true(not keeper.should_set(context))
        context.windup_remaining = 0.075
        t.is_true(keeper.should_set(context))

        context.anticipation = 1
        context.windup_remaining = 0.15
        t.is_true(keeper.should_set(context))
        context.windup_remaining = 0
        t.is_true(not keeper.should_set(context))
    end)
end)

t.describe("keeper advance eligibility", function()
    local eligible = {
        in_claim_zone = true,
        attacker_controlled = true,
        loose_touch = false,
        support_near = false,
        defender_engaged = false,
        threat_distance = 150,
    }

    t.it("uses control and visible support context instead of ball depth alone", function()
        t.is_true(keeper.should_advance(eligible))

        local supported = {}
        for key, value in pairs(eligible) do
            supported[key] = value
        end
        supported.support_near = true
        t.is_true(not keeper.should_advance(supported))
        t.is_true(keeper.should_contain(supported))

        local uncontrolled = {}
        for key, value in pairs(eligible) do
            uncontrolled[key] = value
        end
        uncontrolled.attacker_controlled = false
        t.is_true(not keeper.should_advance(uncontrolled))
    end)

    t.it("lets a loose touch create a smother chance despite an engaged defender", function()
        local context = {}
        for key, value in pairs(eligible) do
            context[key] = value
        end
        context.attacker_controlled = false
        context.loose_touch = true
        context.defender_engaged = true
        t.is_true(keeper.should_advance(context))

        context.loose_touch = false
        context.attacker_controlled = true
        t.is_true(not keeper.should_advance(context))
    end)
end)

t.describe("keeper behavior states", function()
    ---@param state KeeperBehaviorState
    ---@param overrides table?
    ---@return KeeperBehaviorContext
    local function context(state, overrides)
        local value = {
            current_state = state,
            state_timer = 0,
            keeper_pos = Vec2.new(12, 270),
            ball_pos = Vec2.new(150, 220),
            goal = HOME_GOAL,
            team = "home",
            aggression = 42,
            advance_eligible = false,
            contain_eligible = false,
            ground_cue = false,
            lob_cue = false,
            through_ball_cue = false,
            dt = 1 / 60,
        }
        for key, item in pairs(overrides or {}) do
            value[key] = item
        end
        ---@cast value KeeperBehaviorContext
        return value
    end

    t.it("advances and contains on a bounded centre-ray target", function()
        local advancing = keeper.behavior(context("base", { advance_eligible = true }))
        t.eq(advancing.state, "advance")
        t.is_true(advancing.target:dist(Vec2.new(0, 270)) <= 42)

        local containing = keeper.behavior(context("advance", {
            keeper_pos = advancing.target,
            advance_eligible = true,
        }))
        t.eq(containing.state, "contain")
        t.eq(containing.movement_scale, 0.45)
    end)

    t.it("sets for a ground cue and retreats for lob or through-ball preparation", function()
        local set_context = context("advance", { ground_cue = true })
        local set = keeper.behavior(set_context)
        t.eq(set.state, "set")
        t.eq(set.movement_scale, 0)
        t.eq(set.target, set_context.keeper_pos)

        local lob = keeper.behavior(context("advance", { lob_cue = true }))
        t.eq(lob.state, "retreat")
        t.near(lob.target.x, 0)
        t.near(lob.target.y, 270)

        t.eq(keeper.behavior(context("contain", { through_ball_cue = true })).state, "retreat")
        t.eq(keeper.behavior(context("base", { through_ball_cue = true })).state, "base")
    end)

    t.it("holds recover before retreating instead of snapping to base", function()
        local recover = keeper.behavior(context("advance"))
        t.eq(recover.state, "recover")
        t.eq(recover.movement_scale, 0)

        local holding = keeper.behavior(context("recover", {
            state_timer = recover.state_timer,
        }))
        t.eq(holding.state, "recover")
        t.eq(holding.movement_scale, 0)

        local retreat = keeper.behavior(context("recover", {
            state_timer = 0,
            keeper_pos = Vec2.new(40, 270),
        }))
        t.eq(retreat.state, "retreat")
        t.is_true(retreat.movement_scale > 0)
    end)
end)

t.describe("keeper chip counterplay", function()
    ---@param keeper_x number
    ---@return KeeperChipContext
    local function chip_context(keeper_x)
        return {
            origin = Vec2.new(700, 270),
            target = Vec2.new(960, 270),
            keeper_pos = Vec2.new(keeper_x, 270),
            defending_team = "away",
            goal = AWAY_GOAL,
            horizontal_speed = 500,
            friction = 0.3,
            gravity = 900,
            keeper_clearance = 60,
            crossbar = 70,
            desired_goal_height = 65,
        }
    end

    t.it("solves against the actual keeper plane and under the crossbar", function()
        local context = chip_context(900)
        local vz = assert(keeper.chip_launch(context))
        local direction = context.target:sub(context.origin):normalized()
        local height = assert(keeper.goal_line_height({
            origin = context.origin,
            direction = direction,
            horizontal_speed = context.horizontal_speed,
            vertical_speed = vz,
            defending_team = context.defending_team,
            goal = context.goal,
            friction = context.friction,
            gravity = context.gravity,
        }))
        t.is_true(height >= 0 and height < context.crossbar)

        local keeper_distance = (context.keeper_pos.x - context.origin.x) / direction.x
        local keeper_time =
            assert(keeper.travel_time(keeper_distance, context.horizontal_speed, context.friction))
        local keeper_height = vz * keeper_time - 0.5 * context.gravity * keeper_time * keeper_time
        t.is_true(keeper_height > context.keeper_clearance)
    end)

    t.it("makes a committed advance no harder to chip and rejects an empty path", function()
        local deep = assert(keeper.chip_launch(chip_context(948)))
        local advanced = assert(keeper.chip_launch(chip_context(880)))
        t.is_true(advanced <= deep)

        local impossible = chip_context(900)
        impossible.crossbar = 50
        t.eq(keeper.chip_launch(impossible), nil)
    end)

    t.it("keeps an infeasible human chip as an under-bar poor chip", function()
        local context = chip_context(900)
        context.keeper_clearance = 100
        t.eq(keeper.chip_launch(context), nil)

        local vz = keeper.committed_chip_launch(context)
        local direction = context.target:sub(context.origin):normalized()
        local keeper_time = assert(keeper.travel_time(200, 500, 0.3))
        local keeper_height = vz * keeper_time - 450 * keeper_time * keeper_time
        local goal_height = assert(keeper.goal_line_height({
            origin = context.origin,
            direction = direction,
            horizontal_speed = context.horizontal_speed,
            vertical_speed = vz,
            defending_team = context.defending_team,
            goal = context.goal,
            friction = context.friction,
            gravity = context.gravity,
        }))

        t.is_true(vz > 0)
        t.is_true(keeper_height < context.keeper_clearance)
        t.near(goal_height, context.desired_goal_height)
        t.is_true(goal_height < context.crossbar)
    end)

    t.it("uses a deterministic low lob when friction makes the goal unreachable", function()
        local context = chip_context(900)
        context.horizontal_speed = 50
        t.eq(keeper.chip_launch(context), nil)
        t.eq(keeper.travel_time(260, context.horizontal_speed, context.friction), nil)

        local first = keeper.committed_chip_launch(context)
        local second = keeper.committed_chip_launch(context)
        local apex = first * first / (2 * context.gravity)
        t.eq(first, second)
        t.is_true(first > 0)
        t.is_true(apex <= context.keeper_clearance * 0.5)
    end)

    t.it("exposes a visible high line and lets a deep keeper meet a poor chip", function()
        t.is_true(not keeper.chip_is_visible(Vec2.new(19.999999, 270), "home", HOME_GOAL))
        t.is_true(keeper.chip_is_visible(Vec2.new(20, 270), "home", HOME_GOAL))
        t.is_true(keeper.chip_is_visible(Vec2.new(80, 270), "home", HOME_GOAL))
        t.is_true(not keeper.chip_is_visible(Vec2.new(940.000001, 270), "away", AWAY_GOAL))
        t.is_true(keeper.chip_is_visible(Vec2.new(940, 270), "away", AWAY_GOAL))
        t.is_true(keeper.chip_is_visible(Vec2.new(880, 270), "away", AWAY_GOAL))

        local speed = 500
        local vz = 350
        local advanced_time = assert(keeper.travel_time(180, speed, 0.3))
        local deep_time = assert(keeper.travel_time(248, speed, 0.3))
        local goal_time = assert(keeper.travel_time(260, speed, 0.3))
        local advanced_height = vz * advanced_time - 450 * advanced_time * advanced_time
        local deep_height = vz * deep_time - 450 * deep_time * deep_time
        local goal_height = vz * goal_time - 450 * goal_time * goal_time
        t.is_true(advanced_height > 60, "the poor chip clears the committed keeper")
        t.is_true(deep_height <= 60, "the same chip reaches a deep keeper's hands")
        t.is_true(goal_height >= 0 and goal_height < 70, "the chip remains on target")
    end)

    t.it("never gives a moving keeper more reaction reach than a set keeper", function()
        local set = keeper.reaction_reach(100, 0, 0.32)
        local moving = keeper.reaction_reach(100, 1, 0.32)
        t.eq(set, 100)
        t.is_true(moving < set)
    end)
end)
