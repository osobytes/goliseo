local Vec2 = require("core.vec2")
local deterministic_math = require("core.deterministic_math")

---@alias SaveStyle "spread"|"central"|"stretch"
---@alias KeeperBehaviorState "base"|"advance"|"contain"|"set"|"retreat"|"recover"
---@alias KeeperShotType "ground"|"chip"

---@class KeeperPositionContext
---@field keeper_pos Vec2
---@field ball_pos Vec2
---@field goal Rect
---@field team "home"|"away"
---@field aggression number
---@field in_1v1 boolean

---@class KeeperShotContext
---@field defending_team "home"|"away"
---@field shooter_team "home"|"away"
---@field origin Vec2
---@field direction Vec2
---@field goal Rect

---@class KeeperSetContext: KeeperShotContext
---@field anticipation number
---@field windup_duration number
---@field windup_remaining number

---@class KeeperAdvanceContext
---@field in_claim_zone boolean
---@field attacker_controlled boolean
---@field loose_touch boolean
---@field support_near boolean
---@field defender_engaged boolean
---@field threat_distance number

---@class KeeperBehaviorContext
---@field current_state KeeperBehaviorState
---@field state_timer number
---@field keeper_pos Vec2
---@field ball_pos Vec2
---@field goal Rect
---@field team "home"|"away"
---@field aggression number
---@field advance_eligible boolean
---@field contain_eligible boolean
---@field ground_cue boolean
---@field lob_cue boolean
---@field through_ball_cue boolean
---@field dt number

---@class KeeperBehaviorDecision
---@field state KeeperBehaviorState
---@field state_timer number
---@field target Vec2
---@field movement_scale number

---@class KeeperChipContext
---@field origin Vec2
---@field target Vec2
---@field keeper_pos Vec2
---@field defending_team "home"|"away"
---@field goal Rect
---@field horizontal_speed number
---@field friction number
---@field gravity number
---@field keeper_clearance number
---@field crossbar number
---@field desired_goal_height number

---@class KeeperTrajectoryContext
---@field origin Vec2
---@field direction Vec2
---@field horizontal_speed number
---@field vertical_speed number
---@field defending_team "home"|"away"
---@field goal Rect
---@field friction number
---@field gravity number

local CLAIM_DEPTH = 160
-- The issue's fixed context deliberately omits field dimensions. GOLISEO's canonical
-- 960px pitch therefore supplies the 480px goal-line-to-midfield half-depth.
local MIDFIELD_DEPTH = 480
local CONTEXT_LATERAL_GUARD = 28
local BASE_LATERAL_GUARD = 40
local BASE_MIN_DEPTH = 12
local BASE_ARC_EXTRA_FRACTION = 0.15
local BASE_ARC_MAX_EXTRA = 6
local CONTAIN_DISTANCE = 4
local RECOVER_DURATION = 0.18
local ADVANCE_THREAT_DISTANCE = 200
local DEFENDER_HANDOFF_DISTANCE = 120
local CONTAIN_DEPTH_FRACTION = 0.8
local CHIP_VISIBLE_MIN_DEPTH = 20
local CHIP_CLEARANCE_PAD = 2
local CROSSBAR_PAD = 2
local CHIP_FALLBACK_HEIGHT_FRACTION = 0.5
local MIN_CHIP_LAUNCH_SPEED = 1
local MOVEMENT_SETTLE_TIME = 0.12
local SMOTHER_DISTANCE = 26
local SPREAD_DISTANCE = 78
local CENTRAL_REACH_FRACTION = 0.4

---@class KeeperResolver
local keeper = {}

---@param distance number
---@return boolean
function keeper.in_smother_range(distance)
    return distance <= SMOTHER_DISTANCE
end

---@param value number
---@param minimum number
---@param maximum number
---@return number
local function clamp(value, minimum, maximum)
    return math.max(minimum, math.min(maximum, value))
end

---@param team "home"|"away"
---@param goal Rect
---@return number goal_line_x
---@return number infield_direction
local function goal_axis(team, goal)
    if team == "home" then
        return goal.x + goal.w, 1
    end
    return goal.x, -1
end

---@param context KeeperPositionContext
---@param depth number
---@return Vec2
function keeper.depth_target(context, depth)
    local goal_line_x, infield_direction = goal_axis(context.team, context.goal)
    local goal_center = Vec2.new(goal_line_x, context.goal.y + context.goal.h / 2)
    if depth <= 0 then
        return goal_center
    end

    local ray = context.ball_pos:sub(goal_center)
    if ray:length() == 0 then
        ray = context.keeper_pos:sub(goal_center)
    end
    if ray:length() == 0 or ray.x * infield_direction <= 0 then
        ray = Vec2.new(infield_direction, 0)
    end

    local target = goal_center:add(ray:normalized():scale(depth))
    return Vec2.new(
        target.x,
        clamp(
            target.y,
            goal_center.y - CONTEXT_LATERAL_GUARD,
            goal_center.y + CONTEXT_LATERAL_GUARD
        )
    )
end

---Choose a shallow neutral arc from the physical one-radius goal-line inset.
---Depth and the deliberately exaggerated lateral corner concession grow as an
---attack approaches, without becoming a continuously optimized save surface.
---@param context KeeperPositionContext
---@return Vec2
function keeper.base_target(context)
    local goal_line_x, infield_direction = goal_axis(context.team, context.goal)
    local ball_depth = (context.ball_pos.x - goal_line_x) * infield_direction
    local goal_center_y = context.goal.y + context.goal.h / 2
    if ball_depth >= MIDFIELD_DEPTH then
        return Vec2.new(goal_line_x + infield_direction * BASE_MIN_DEPTH, goal_center_y)
    end

    local approach = clamp((MIDFIELD_DEPTH - ball_depth) / (MIDFIELD_DEPTH - CLAIM_DEPTH), 0, 1)
    local extra_cap =
        math.min(math.max(context.aggression, 0) * BASE_ARC_EXTRA_FRACTION, BASE_ARC_MAX_EXTRA)
    local target_depth = BASE_MIN_DEPTH + extra_cap * approach
    local lateral =
        clamp(context.ball_pos.y - goal_center_y, -BASE_LATERAL_GUARD, BASE_LATERAL_GUARD)
    return Vec2.new(
        goal_line_x + infield_direction * target_depth,
        goal_center_y + lateral * approach
    )
end

---@param context KeeperPositionContext
---@return Vec2
function keeper.arc_target(context)
    local goal_line_x, infield_direction = goal_axis(context.team, context.goal)

    local goal_center = Vec2.new(goal_line_x, context.goal.y + context.goal.h / 2)
    local ball_depth = (context.ball_pos.x - goal_line_x) * infield_direction
    if ball_depth >= MIDFIELD_DEPTH then
        return goal_center
    end

    local approach = clamp((MIDFIELD_DEPTH - ball_depth) / (MIDFIELD_DEPTH - CLAIM_DEPTH), 0, 1)
    local target_depth = math.max(context.aggression, 0) * (context.in_1v1 and 1 or approach)
    if target_depth == 0 then
        return goal_center
    end

    return keeper.depth_target(context, target_depth)
end

---@param context KeeperAdvanceContext
---@return boolean
function keeper.should_advance(context)
    return context.in_claim_zone
        and context.threat_distance <= ADVANCE_THREAT_DISTANCE
        and (context.attacker_controlled or context.loose_touch)
        and not context.support_near
        and (
            context.loose_touch
            or not context.defender_engaged
            or context.threat_distance <= DEFENDER_HANDOFF_DISTANCE
        )
end

---@param context KeeperAdvanceContext
---@return boolean
function keeper.should_contain(context)
    return context.in_claim_zone
        and context.threat_distance <= ADVANCE_THREAT_DISTANCE
        and (context.attacker_controlled or context.loose_touch)
end

---@param context KeeperBehaviorContext
---@return KeeperBehaviorDecision
function keeper.behavior(context)
    local position_context = {
        keeper_pos = context.keeper_pos,
        ball_pos = context.ball_pos,
        goal = context.goal,
        team = context.team,
        aggression = context.aggression,
        in_1v1 = true,
    }
    local base_target = keeper.base_target(position_context)
    local retreat_target = base_target
    local deep_target = keeper.depth_target(position_context, 0)
    local advance_target = keeper.depth_target(position_context, math.max(context.aggression, 0))
    local contain_target = keeper.depth_target(
        position_context,
        math.max(context.aggression, 0) * CONTAIN_DEPTH_FRACTION
    )
    local state = context.current_state
    local state_timer = math.max(context.state_timer - context.dt, 0)
    local target = base_target
    local movement_scale = 1

    if context.lob_cue then
        state = "retreat"
        state_timer = 0
        target = deep_target
        movement_scale = 0.85
    elseif context.through_ball_cue and state ~= "base" then
        state = "retreat"
        state_timer = 0
        target = retreat_target
        movement_scale = 0.85
    elseif context.ground_cue then
        state = "set"
        state_timer = 0
        target = context.keeper_pos
        movement_scale = 0
    elseif context.advance_eligible then
        target = advance_target
        if context.keeper_pos:dist(advance_target) <= CONTAIN_DISTANCE then
            state = "contain"
            movement_scale = 0.45
        else
            state = "advance"
        end
        state_timer = 0
    elseif context.contain_eligible then
        state = "contain"
        state_timer = 0
        target = contain_target
        movement_scale = 0.6
    elseif state == "advance" or state == "contain" or state == "set" then
        state = "recover"
        state_timer = RECOVER_DURATION
        target = context.keeper_pos
        movement_scale = 0
    elseif state == "recover" and state_timer > 0 then
        target = context.keeper_pos
        movement_scale = 0
    elseif state == "retreat" or state == "recover" then
        target = retreat_target
        if context.keeper_pos:dist(retreat_target) <= CONTAIN_DISTANCE then
            state = "base"
            movement_scale = 1
        else
            state = "retreat"
            movement_scale = 0.85
        end
    else
        state = "base"
    end

    return {
        state = state,
        state_timer = state_timer,
        target = target,
        movement_scale = movement_scale,
    }
end

---@param keeper_pos Vec2
---@param team "home"|"away"
---@param goal Rect
---@return boolean
function keeper.chip_is_visible(keeper_pos, team, goal)
    local goal_line_x, infield_direction = goal_axis(team, goal)
    local depth = (keeper_pos.x - goal_line_x) * infield_direction
    return depth >= CHIP_VISIBLE_MIN_DEPTH
end

---@param distance number
---@param speed number
---@param friction number
---@return number?
function keeper.travel_time(distance, speed, friction)
    if distance <= 0 then
        return 0
    end
    if speed <= 0 then
        return nil
    end
    if friction <= 0 then
        return distance / speed
    end
    local ratio = distance * friction / speed
    if ratio >= 0.95 then
        return nil
    end
    return deterministic_math.negative_log_one_minus(ratio) / friction
end

---A moving keeper spends part of the fixed dive budget planting before pushing off.
---This is release-time state debt, not a positional reach bonus or penalty.
---@param reach number
---@param normalized_motion number
---@param dive_duration number
---@return number
function keeper.reaction_reach(reach, normalized_motion, dive_duration)
    if dive_duration <= 0 then
        return 0
    end
    local settle_time = MOVEMENT_SETTLE_TIME * clamp(normalized_motion, 0, 1)
    local available_time = math.max(0, dive_duration - settle_time)
    return math.max(reach, 0) * available_time / dive_duration
end

---@param context KeeperChipContext
---@return number? vertical_speed
function keeper.chip_launch(context)
    local direction = context.target:sub(context.origin)
    local distance = direction:length()
    if distance <= 0 or context.horizontal_speed <= 0 then
        return nil
    end
    direction = direction:scale(1 / distance)

    local goal_line_x = context.defending_team == "home" and (context.goal.x + context.goal.w)
        or context.goal.x
    if direction.x == 0 then
        return nil
    end
    local goal_distance = (goal_line_x - context.origin.x) / direction.x
    local keeper_distance = (context.keeper_pos.x - context.origin.x) / direction.x
    if goal_distance <= 0 or keeper_distance <= 0 or keeper_distance >= goal_distance then
        return nil
    end

    local keeper_time =
        keeper.travel_time(keeper_distance, context.horizontal_speed, context.friction)
    local goal_time = keeper.travel_time(goal_distance, context.horizontal_speed, context.friction)
    if not keeper_time or not goal_time then
        return nil
    end

    local lower_keeper = (context.keeper_clearance + CHIP_CLEARANCE_PAD) / keeper_time
        + 0.5 * context.gravity * keeper_time
    local lower_goal = 0.5 * context.gravity * goal_time
    local desired_goal = context.desired_goal_height / goal_time + 0.5 * context.gravity * goal_time
    local upper_goal = (context.crossbar - CROSSBAR_PAD) / goal_time
        + 0.5 * context.gravity * goal_time
    local vertical_speed = math.max(lower_keeper, lower_goal, desired_goal)
    if vertical_speed >= upper_goal then
        return nil
    end
    return vertical_speed
end

---Lock a human-selected chip verb at commit time. Prefer the fully feasible
---keeper-clearing solution; when that interval is empty, keep the chip intent
---with an under-bar goal-height arc. If friction prevents the ball reaching
---the goal at all, use a deterministic low lob that will land short.
---@param context KeeperChipContext
---@return number vertical_speed
function keeper.committed_chip_launch(context)
    local feasible = keeper.chip_launch(context)
    if feasible then
        return feasible
    end

    local direction = context.target:sub(context.origin)
    if direction:length() > 0 and direction.x ~= 0 then
        direction = direction:normalized()
        local goal_line_x = context.defending_team == "home" and (context.goal.x + context.goal.w)
            or context.goal.x
        local goal_distance = (goal_line_x - context.origin.x) / direction.x
        local goal_time =
            keeper.travel_time(goal_distance, context.horizontal_speed, context.friction)
        if goal_time then
            local desired_height =
                clamp(context.desired_goal_height, 0, math.max(context.crossbar - CROSSBAR_PAD, 0))
            return math.max(
                MIN_CHIP_LAUNCH_SPEED,
                desired_height / goal_time + 0.5 * context.gravity * goal_time
            )
        end
    end

    local low_height = math.max(
        CHIP_CLEARANCE_PAD,
        math.min(
            context.desired_goal_height,
            context.keeper_clearance,
            math.max(context.crossbar - CROSSBAR_PAD, 0)
        ) * CHIP_FALLBACK_HEIGHT_FRACTION
    )
    return math.max(MIN_CHIP_LAUNCH_SPEED, math.sqrt(math.max(2 * context.gravity * low_height, 0)))
end

---@param context KeeperTrajectoryContext
---@return number? height
function keeper.goal_line_height(context)
    if context.direction.x == 0 then
        return nil
    end
    local goal_line_x = context.defending_team == "home" and (context.goal.x + context.goal.w)
        or context.goal.x
    local distance = (goal_line_x - context.origin.x) / context.direction.x
    if distance < 0 then
        return nil
    end
    local eta = keeper.travel_time(distance, context.horizontal_speed, context.friction)
    if not eta then
        return nil
    end
    return context.vertical_speed * eta - 0.5 * context.gravity * eta * eta
end

---@param dist_to_keeper number
---@param dive_dist number
---@param reach number
---@return SaveStyle
function keeper.save_style(dist_to_keeper, dive_dist, reach)
    assert(
        not keeper.in_smother_range(dist_to_keeper),
        "save_style only classifies saves beyond the smother distance"
    )
    if dist_to_keeper <= SPREAD_DISTANCE then
        return "spread"
    end
    if dive_dist <= reach * CENTRAL_REACH_FRACTION then
        return "central"
    end
    return "stretch"
end

---@param anticipation number
---@param windup_duration number
---@return number seconds
function keeper.commit_lead(anticipation, windup_duration)
    return clamp(anticipation, 0, 1) * math.max(windup_duration, 0)
end

---@param context KeeperShotContext
---@return boolean
function keeper.shot_targets_goal(context)
    if context.shooter_team == context.defending_team or context.direction.x == 0 then
        return false
    end

    local goal_line_x = context.defending_team == "home" and (context.goal.x + context.goal.w)
        or context.goal.x
    local flight = (goal_line_x - context.origin.x) / context.direction.x
    if flight < 0 then
        return false
    end

    local goal_y = context.origin.y + context.direction.y * flight
    return goal_y >= context.goal.y and goal_y <= context.goal.y + context.goal.h
end

---@param context KeeperSetContext
---@return boolean
function keeper.should_set(context)
    local lead = keeper.commit_lead(context.anticipation, context.windup_duration)
    return lead > 0
        and context.windup_remaining > 0
        and context.windup_remaining <= lead
        and keeper.shot_targets_goal(context)
end

return keeper
