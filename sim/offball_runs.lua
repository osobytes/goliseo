-- Pure role-gated off-ball run geometry and bounded team arbitration. Match
-- owns authoritative player clocks/state; this module derives candidates from
-- one world boundary and delegates deterministic ranking to sim.brain.

local Vec2 = require("core.vec2")
local ai = require("sim.ai")
local brain = require("sim.brain")
local outfield_decision = require("sim.outfield_decision")
local formations = require("data.formations")

---@class OffballRunPlayer
---@field player_index integer
---@field role FormationRole
---@field run_drive number
---@field pos Vec2
---@field anchor_y number  -- normalized authored formation width

---@class OffballRunTeammate
---@field player_index integer
---@field pos Vec2

---@class OffballRunOpponent
---@field pos Vec2
---@field is_keeper boolean

---@class OffballRunContext
---@field team "home"|"away"
---@field field { w: number, h: number }
---@field carrier_pos Vec2
---@field carrier_settled boolean
---@field carrier_pressure number
---@field pressure_distance number
---@field counterattack boolean?  -- inside a counter-attack window (sim.possession_transition)
---@field players OffballRunPlayer[]
---@field teammates OffballRunTeammate[]
---@field opponents OffballRunOpponent[]

---@class OffballRunModule
local offball_runs = {}

offball_runs.RUN_LIFETIME_SECONDS = outfield_decision.RUN_LIFETIME_SECONDS
offball_runs.TELEGRAPH_SECONDS = 0.2
offball_runs.MAX_ACTIVE_PER_TEAM = 2
offball_runs.RUN_DRIVE_THRESHOLD = 0.55
offball_runs.MIN_RUN_PROGRESS = 24
offball_runs.MIN_SUPPORT_DISTANCE = 80
offball_runs.MAX_SUPPORT_DISTANCE = 250

local FIELD_MARGIN = 24
local IN_BEHIND_GAP = 56
local IN_BEHIND_LANE_WIDTH = 24
local COME_SHORT_DISTANCE = 115
local COME_SHORT_MARKER_DISTANCE = 100
local COME_SHORT_MARKER_OFFSET = 24
local HOLD_WIDTH_DEPTH = 70
local HOLD_WIDTH_OCCUPIED_X = 130
local HOLD_WIDTH_OCCUPIED_Y = 72

local ROLES = {
    def = true,
    mid = true,
    wide = true,
    fwd = true,
}

---@param value number
---@param low number
---@param high number
---@return number
local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

---@param context OffballRunContext
---@return number
local function attack_direction(context)
    return context.team == "home" and 1 or -1
end

---@param context OffballRunContext
---@param point Vec2
---@return Vec2
local function clamp_target(context, point)
    return Vec2.new(
        clamp(point.x, FIELD_MARGIN, context.field.w - FIELD_MARGIN),
        clamp(point.y, FIELD_MARGIN, context.field.h - FIELD_MARGIN)
    )
end

---@param context OffballRunContext
---@param point Vec2
---@param minimum number
---@param maximum number
---@return Vec2?
local function bound_support_distance(context, point, minimum, maximum)
    local offset = point:sub(context.carrier_pos)
    local distance = offset:length()
    local direction = distance > 0 and offset:scale(1 / distance)
        or Vec2.new(attack_direction(context), 0)
    local target = clamp_target(
        context,
        context.carrier_pos:add(direction:scale(clamp(distance, minimum, maximum)))
    )
    local final_distance = context.carrier_pos:dist(target)
    if final_distance < minimum or final_distance > maximum then
        return nil
    end
    return target
end

---@param context OffballRunContext
---@return Vec2[]
local function nonkeeper_opponent_positions(context)
    local positions = {}
    for _, opponent in ipairs(context.opponents) do
        if not opponent.is_keeper then
            positions[#positions + 1] = opponent.pos
        end
    end
    return positions
end

---@param context OffballRunContext
---@param player OffballRunPlayer
---@return BrainRunTarget?
local function in_behind_target(context, player)
    -- A counter-attack window buys ONE immediate in-behind request from an
    -- unsettled carrier (`grant` keeps only the best). The role gate, lane
    -- rules, progress floor, team cap, and slot expiry are untouched.
    if
        player.role ~= "fwd"
        or player.run_drive < offball_runs.RUN_DRIVE_THRESHOLD
        or not (context.carrier_settled or context.counterattack)
    then
        return nil
    end

    local attack = attack_direction(context)
    local last_defender = nil
    for _, opponent in ipairs(context.opponents) do
        if not opponent.is_keeper then
            if last_defender == nil or (opponent.pos.x - assert(last_defender)) * attack > 0 then
                last_defender = opponent.pos.x
            end
        end
    end
    if last_defender == nil then
        return nil
    end

    local target =
        clamp_target(context, Vec2.new(last_defender + attack * IN_BEHIND_GAP, player.pos.y))
    if
        (target.x - last_defender) * attack <= 0
        or (target.x - context.field.w / 2) * attack <= 0
        or (target.x - player.pos.x) * attack < offball_runs.MIN_RUN_PROGRESS
        or ai.lane_blocker(
                context.carrier_pos,
                target,
                nonkeeper_opponent_positions(context),
                IN_BEHIND_LANE_WIDTH
            )
            ~= nil
    then
        return nil
    end

    local progress = math.max(0, (target.x - context.carrier_pos.x) * attack)
    return {
        score = 100 + player.run_drive * 20 + progress / context.field.w * 10,
        x = target.x,
        y = target.y,
        duration = offball_runs.RUN_LIFETIME_SECONDS,
    }
end

---@param context OffballRunContext
---@param player OffballRunPlayer
---@return BrainRunTarget?
local function come_short_target(context, player)
    if
        (player.role ~= "mid" and player.role ~= "wide")
        or not context.carrier_settled
        or context.carrier_pressure > context.pressure_distance
    then
        return nil
    end

    local to_runner = player.pos:sub(context.carrier_pos)
    local runner_distance = to_runner:length()
    local direction = runner_distance > 0 and to_runner:normalized()
        or Vec2.new(attack_direction(context), 0)
    local target = context.carrier_pos:add(direction:scale(COME_SHORT_DISTANCE))
    local marker = nil
    local marker_distance = nil
    for _, opponent in ipairs(context.opponents) do
        if not opponent.is_keeper then
            local distance = opponent.pos:dist(target)
            if marker_distance == nil or distance < marker_distance then
                marker = opponent.pos
                marker_distance = distance
            end
        end
    end
    if marker and assert(marker_distance) < COME_SHORT_MARKER_DISTANCE then
        local goal_side_x = marker.x + attack_direction(context) * COME_SHORT_MARKER_OFFSET
        if (goal_side_x - target.x) * attack_direction(context) > 0 then
            target = Vec2.new(goal_side_x, target.y)
        end
    end
    local bounded_target = bound_support_distance(
        context,
        target,
        offball_runs.MIN_SUPPORT_DISTANCE,
        COME_SHORT_DISTANCE + COME_SHORT_MARKER_OFFSET
    )
    if not bounded_target then
        return nil
    end
    target = bounded_target
    local target_distance = context.carrier_pos:dist(target)
    if runner_distance - target_distance < offball_runs.MIN_RUN_PROGRESS then
        return nil
    end
    local pressure_factor = 1
        - clamp(context.carrier_pressure / math.max(1, context.pressure_distance), 0, 1)
    return {
        score = 78 + pressure_factor * 14 + player.run_drive * 6,
        x = target.x,
        y = target.y,
        duration = offball_runs.RUN_LIFETIME_SECONDS,
    }
end

---@param context OffballRunContext
---@param player OffballRunPlayer
---@return BrainRunTarget?
local function hold_width_target(context, player)
    if player.role ~= "wide" or not context.carrier_settled then
        return nil
    end
    local upper_lane = player.anchor_y < 0.5
    local lane = Vec2.new(
        context.carrier_pos.x + attack_direction(context) * HOLD_WIDTH_DEPTH,
        upper_lane and context.field.h / 6 or context.field.h * 5 / 6
    )
    lane = clamp_target(context, lane)
    if
        math.abs(context.carrier_pos.x - lane.x) < HOLD_WIDTH_OCCUPIED_X
        and math.abs(context.carrier_pos.y - lane.y) < HOLD_WIDTH_OCCUPIED_Y
    then
        return nil
    end
    for _, teammate in ipairs(context.teammates) do
        if
            teammate.player_index ~= player.player_index
            and math.abs(teammate.pos.x - lane.x) < HOLD_WIDTH_OCCUPIED_X
            and math.abs(teammate.pos.y - lane.y) < HOLD_WIDTH_OCCUPIED_Y
        then
            return nil
        end
    end
    local target = bound_support_distance(
        context,
        lane,
        offball_runs.MIN_SUPPORT_DISTANCE,
        offball_runs.MAX_SUPPORT_DISTANCE
    )
    if not target then
        return nil
    end
    local remains_outer = upper_lane and target.y <= context.field.h / 3
        or not upper_lane and target.y >= context.field.h * 2 / 3
    if not remains_outer then
        return nil
    end
    local width_gain = math.abs(target.y - context.field.h / 2) / (context.field.h / 2)
    return {
        score = 64 + width_gain * 12 + player.run_drive * 5,
        x = target.x,
        y = target.y,
        duration = offball_runs.RUN_LIFETIME_SECONDS,
    }
end

---@param formation_id string
---@param outfield_ordinal integer
---@return FormationRole
function offball_runs.formation_role(formation_id, outfield_ordinal)
    local formation = assert(formations[formation_id], "unknown formation: " .. formation_id)
    local anchor =
        assert(formation.outfield[outfield_ordinal], "formation outfield ordinal is out of range")
    assert(ROLES[anchor.role], "formation has an unknown outfield role")
    return anchor.role
end

---@param formation_id string
---@param outfield_ordinal integer
---@return number
function offball_runs.formation_anchor_y(formation_id, outfield_ordinal)
    local formation = assert(formations[formation_id], "unknown formation: " .. formation_id)
    local anchor =
        assert(formation.outfield[outfield_ordinal], "formation outfield ordinal is out of range")
    return anchor.y
end

---@param context OffballRunContext
---@param active RunSlot[]
---@param now number
---@return RunSlot[]
function offball_runs.grant(context, active, now)
    local players = {}
    for _, player in ipairs(context.players) do
        players[#players + 1] = {
            player_index = player.player_index,
            in_behind = in_behind_target(context, player),
            come_short = come_short_target(context, player),
            hold_width = hold_width_target(context, player),
        }
    end
    if context.counterattack and not context.carrier_settled then
        -- Exactly one free in-behind request: the best-scoring option wins and
        -- the player index breaks ties, so mirrored fixtures mirror.
        local best = nil
        for _, player in ipairs(players) do
            if
                player.in_behind ~= nil
                and (best == nil or player.in_behind.score > assert(best.in_behind).score)
            then
                best = player
            end
        end
        for _, player in ipairs(players) do
            if player ~= best then
                player.in_behind = nil
            end
        end
    end
    local candidates = brain.run_candidates({ players = players })
    return brain.grant_runs(candidates, active, offball_runs.MAX_ACTIVE_PER_TEAM, now)
end

---@param now number
---@param expires_at number
---@return boolean
function offball_runs.telegraphing(now, expires_at)
    local granted_at = expires_at - offball_runs.RUN_LIFETIME_SECONDS
    local elapsed = now - granted_at
    local epsilon = 1e-12
    if math.abs(elapsed) <= epsilon then
        elapsed = 0
    elseif math.abs(elapsed - offball_runs.TELEGRAPH_SECONDS) <= epsilon then
        elapsed = offball_runs.TELEGRAPH_SECONDS
    end
    return elapsed >= 0 and elapsed < offball_runs.TELEGRAPH_SECONDS and now < expires_at
end

return offball_runs
