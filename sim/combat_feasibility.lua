-- `intervention_candidate/v2` and `family_commit_feasibility/v1`.
--
-- Both are PURE temporal predicates over `combat_sim_observation/v1`. They read
-- no confirmed future input, no rollback-only or hidden state, no unresolved
-- outcome, no presentation, and no eventual contact. Everything they project is
-- a counterfactual, not a claim about what will happen.
--
-- Public motion projection (the frozen rule these predicates share with
-- `combat_observation.COLLISION_CATALOG_VERSION`): a body advances linearly at
-- the fixed tick from its observed position and velocity and is clamped to the
-- pitch by its radius. A committed or forced public phase scales that velocity
-- by the catalogued family movement multiplier (zero while forced), which is
-- the "catalogued public pose path anchored at the observed state" the contract
-- asks for. Nothing here searches a non-source trajectory.
--
-- The five purpose ids are exactly section 4.6's: carrier contest, carrier
-- protection, loose-ball contest, passing-lane/shot denial, and recovery
-- punish. `formation_risk_tradeoff` is a cost flag returned alongside them and
-- is never a sixth purpose.

local fixed_clock = require("sim.fixed_clock")
local action_families = require("data.action_families")

---@alias CombatPurposeId
---|"carrier_contest"
---|"carrier_protection"
---|"loose_ball_contest"
---|"passing_lane_or_shot_denial"
---|"recovery_punish"

---@class CombatFeasibilityWitness
---@field feasible boolean
---@field family_id ActionFamilyId
---@field target_player integer -- Frozen target, or hostile threat source for guard.
---@field commit_tick integer -- Relative tick the commit starts on.
---@field contact_tick integer -- Relative tick of the witnessed intersection; 0 when infeasible.
---@field horizon_ticks integer -- Last relative tick the proof searched.

---@class CombatInterventionPair
---@field target_player integer
---@field purpose CombatPurposeId
---@field commit_tick integer
---@field family_bitset table<ActionFamilyId, boolean>
---@field formation_risk boolean

---@class CombatFeasibilityModule
local combat_feasibility = {}

combat_feasibility.VERSION = 1
combat_feasibility.ENVELOPE_VERSION = 2
combat_feasibility.SEARCH_TICKS = 30

-- Section 4.6 constants. These are contract numbers, not tuning knobs.
combat_feasibility.CARRIER_PROTECTION_PX = 48
combat_feasibility.LOOSE_BALL_PX = 96
combat_feasibility.FORMATION_RISK_GAIN_PX = 36
combat_feasibility.FORMATION_RISK_ANCHOR_PX = 120

-- Lane geometry. A blocker "controls" a segment when its projected body is
-- within its own radius plus this corridor of the segment.
local LANE_CORRIDOR_PX = 18
local SHOT_RANGE_PX = 260
local EPSILON = 1e-9

-- Purpose priority when several predicates hold. #148 keeps the whole bitset and
-- reports `multi_context`; #112 must emit exactly one stable reason, so the most
-- specific true purpose wins and the order never depends on scores.
combat_feasibility.PURPOSE_PRIORITY = {
    "recovery_punish",
    "carrier_contest",
    "loose_ball_contest",
    "passing_lane_or_shot_denial",
    "carrier_protection",
}

---@type table<CombatPurposeId, boolean>
combat_feasibility.PURPOSES = {
    carrier_contest = true,
    carrier_protection = true,
    loose_ball_contest = true,
    passing_lane_or_shot_denial = true,
    recovery_punish = true,
}

---@param x number
---@param y number
---@return number
local function length(x, y)
    return math.sqrt(x * x + y * y)
end

---@param x number
---@param y number
---@param fallback_x number
---@param fallback_y number
---@return number, number
local function unit_or(x, y, fallback_x, fallback_y)
    local len = length(x, y)
    if len > EPSILON then
        return x / len, y / len
    end
    return fallback_x, fallback_y
end

---@param value number
---@param low number
---@param high number
---@return number
local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

---@class CombatProjectedBody
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field radius number

-- The frozen public no-response projection. A committed or forced public phase
-- scales the observed velocity by its catalogued multiplier; nothing else moves
-- the body but the pitch clamp.
---@param row CombatObservationPeer|CombatObservationSelf
---@param observation CombatObservation
---@param ticks integer
---@return CombatProjectedBody
function combat_feasibility.project(row, observation, ticks)
    local scale = 1
    if row.forced_ticks > 0 then
        scale = 0
    elseif row.phase ~= "ready" and row.family_id ~= "none" then
        scale = action_families[row.family_id].movement_multiplier
    end
    local vx = row.vx * scale
    local vy = row.vy * scale
    local seconds = ticks * fixed_clock.TICK_SECONDS
    return {
        x = clamp(row.x + vx * seconds, row.radius, observation.match.field_w - row.radius),
        y = clamp(row.y + vy * seconds, row.radius, observation.match.field_h - row.radius),
        vx = vx,
        vy = vy,
        radius = row.radius,
    }
end

---@param observation CombatObservation
---@param player_index integer
---@return CombatObservationPeer?
function combat_feasibility.row_for(observation, player_index)
    for _, row in ipairs(observation.opponents) do
        if row.player_index == player_index then
            return row
        end
    end
    for _, row in ipairs(observation.teammates) do
        if row.player_index == player_index then
            return row
        end
    end
    return nil
end

-- Melee contact geometry, mirroring `sim.combat`'s resolver exactly: inside the
-- reach plus the target radius, in front of the facing, and inside the arc.
---@param source_x number
---@param source_y number
---@param facing_x number
---@param facing_y number
---@param target CombatProjectedBody
---@param family ActionFamilyData
---@return boolean
local function melee_contact(source_x, source_y, facing_x, facing_y, target, family)
    local dx = target.x - source_x
    local dy = target.y - source_y
    local distance = length(dx, dy)
    if distance > assert(family.reach_px) + target.radius then
        return false
    end
    if distance <= EPSILON then
        return true
    end
    if dx * facing_x + dy * facing_y < 0 then
        return false
    end
    local half_arc = math.rad(family.front_arc_degrees / 2)
    return (dx / distance) * facing_x + (dy / distance) * facing_y + EPSILON >= math.cos(half_arc)
end

---@param start_x number
---@param start_y number
---@param end_x number
---@param end_y number
---@param center_x number
---@param center_y number
---@param radius number
---@return boolean
local function segment_hits_circle(start_x, start_y, end_x, end_y, center_x, center_y, radius)
    local sx = end_x - start_x
    local sy = end_y - start_y
    local length_sq = sx * sx + sy * sy
    if length_sq <= EPSILON then
        return length(center_x - start_x, center_y - start_y) <= radius
    end
    local t = clamp(((center_x - start_x) * sx + (center_y - start_y) * sy) / length_sq, 0, 1)
    local px = start_x + sx * t
    local py = start_y + sy * t
    return length(center_x - px, center_y - py) <= radius + EPSILON
end

---@param observation CombatObservation
---@return CombatObservationPeer[]
local function all_peers(observation)
    local rows = {}
    for _, row in ipairs(observation.teammates) do
        rows[#rows + 1] = row
    end
    for _, row in ipairs(observation.opponents) do
        rows[#rows + 1] = row
    end
    table.sort(rows, function(left, right)
        return left.player_index < right.player_index
    end)
    return rows
end

-- The source pose the witness tape leaves behind. `family_commit_feasibility`
-- carries the tape's last legal movement and facing input after the commit; an
-- empty tape means neutral movement and the observed facing.
---@class CombatWitnessTape
---@field move_x number
---@field move_y number
---@field ticks integer

---@param observation CombatObservation
---@param tape CombatWitnessTape?
---@return number, number, number, number
local function source_pose_after_tape(observation, tape)
    local own = observation.self
    if not tape or tape.ticks <= 0 or (tape.move_x == 0 and tape.move_y == 0) then
        local fx, fy = unit_or(own.facing_x, own.facing_y, own.team == "home" and 1 or -1, 0)
        return own.x, own.y, fx, fy
    end
    local mx, my = unit_or(tape.move_x, tape.move_y, 0, 0)
    local seconds = tape.ticks * fixed_clock.TICK_SECONDS
    local x = clamp(
        own.x + mx * own.move_speed * seconds,
        own.radius,
        observation.match.field_w - own.radius
    )
    local y = clamp(
        own.y + my * own.move_speed * seconds,
        own.radius,
        observation.match.field_h - own.radius
    )
    return x, y, mx, my
end

-- The source's committed pose at a relative tick after the commit: it repeats
-- the tape's last legal movement vector through the ordinary family movement
-- multiplier, the pitch, and the collision transition.
---@param observation CombatObservation
---@param tape CombatWitnessTape?
---@param family ActionFamilyData
---@param ticks integer
---@return number, number, number, number
local function committed_source_pose(observation, tape, family, ticks)
    local x, y, fx, fy = source_pose_after_tape(observation, tape)
    local own = observation.self
    local mx, my = 0, 0
    if tape and (tape.move_x ~= 0 or tape.move_y ~= 0) then
        mx, my = unit_or(tape.move_x, tape.move_y, 0, 0)
    end
    if mx == 0 and my == 0 then
        return x, y, fx, fy
    end
    local seconds = ticks * fixed_clock.TICK_SECONDS
    local speed = own.move_speed * family.movement_multiplier
    return clamp(x + mx * speed * seconds, own.radius, observation.match.field_w - own.radius),
        clamp(y + my * speed * seconds, own.radius, observation.match.field_h - own.radius),
        fx,
        fy
end

---@param observation CombatObservation
---@param target CombatObservationPeer
---@param family ActionFamilyData
---@param tape CombatWitnessTape?
---@return CombatFeasibilityWitness
local function melee_feasibility(observation, target, family, tape)
    local commit_tick = tape and tape.ticks or 0
    local active_ticks = assert(family.active_ticks)
    local horizon = commit_tick + family.windup_ticks + active_ticks
    for offset = 1, active_ticks do
        local relative = family.windup_ticks + offset
        local sx, sy, fx, fy = committed_source_pose(observation, tape, family, relative)
        local projected = combat_feasibility.project(target, observation, commit_tick + relative)
        if melee_contact(sx, sy, fx, fy, projected, family) then
            return {
                feasible = true,
                family_id = family.id,
                target_player = target.player_index,
                commit_tick = commit_tick,
                contact_tick = commit_tick + relative,
                horizon_ticks = horizon,
            }
        end
    end
    return {
        feasible = false,
        family_id = family.id,
        target_player = target.player_index,
        commit_tick = commit_tick,
        contact_tick = 0,
        horizon_ticks = horizon,
    }
end

-- The canonical ranged witness: commit with `pressed, held`, emit the early
-- release edge on the next tick, and let it latch through the rest of the
-- 18-tick windup. At the 1-tick spawn transition the aim must be legal and the
-- line clear against projected public blockers; the direction then freezes and
-- the projectile travels at the catalogued speed for at most its lifetime or
-- until it leaves the pitch.
---@param observation CombatObservation
---@param target CombatObservationPeer
---@param tape CombatWitnessTape?
---@return CombatFeasibilityWitness
local function ranged_feasibility(observation, target, tape)
    local family = action_families.ranged
    local commit_tick = tape and tape.ticks or 0
    local lifetime = assert(family.projectile_lifetime_ticks)
    local spawn_relative = family.windup_ticks + 1
    local horizon = commit_tick + spawn_relative + lifetime
    local sx, sy, fx, fy = committed_source_pose(observation, tape, family, spawn_relative)
    local step = assert(family.projectile_speed_px_per_second) * fixed_clock.TICK_SECONDS
    local blockers = all_peers(observation)
    local x, y = sx, sy
    for travel = 1, lifetime do
        local next_x = x + fx * step
        local next_y = y + fy * step
        local absolute = commit_tick + spawn_relative + travel
        local first ---@type CombatObservationPeer?
        local first_time = math.huge
        for _, row in ipairs(blockers) do
            if not row.is_keeper and row.team ~= observation.self.team then
                local projected = combat_feasibility.project(row, observation, absolute)
                if
                    segment_hits_circle(
                        x,
                        y,
                        next_x,
                        next_y,
                        projected.x,
                        projected.y,
                        projected.radius
                    )
                then
                    local time = length(projected.x - x, projected.y - y)
                    if time < first_time then
                        first_time = time
                        first = row
                    end
                end
            end
        end
        if first then
            return {
                feasible = first.player_index == target.player_index,
                family_id = "ranged",
                target_player = target.player_index,
                commit_tick = commit_tick,
                contact_tick = first.player_index == target.player_index and absolute or 0,
                horizon_ticks = horizon,
            }
        end
        x, y = next_x, next_y
        if x < 0 or x > observation.match.field_w or y < 0 or y > observation.match.field_h then
            break
        end
    end
    return {
        feasible = false,
        family_id = "ranged",
        target_player = target.player_index,
        commit_tick = commit_tick,
        contact_tick = 0,
        horizon_ticks = horizon,
    }
end

---@class CombatHostilePath
---@field source CombatObservationPeer?
---@field projectile CombatObservationProjectile?
---@field start_tick integer -- First relative tick of the path.
---@field last_tick integer -- Last relative tick of the path.

-- Guard's relevant threat set: only finite paths the public observation already
-- proves. A held or aimed ranged row without a public release latch never
-- participates.
---@param observation CombatObservation
---@param source_index integer?
---@return CombatHostilePath[]
function combat_feasibility.hostile_paths(observation, source_index)
    local paths = {}
    local tick = observation.observed_tick
    for _, row in ipairs(observation.opponents) do
        if not row.is_keeper and (source_index == nil or row.player_index == source_index) then
            if row.family_id == "unarmed" or row.family_id == "light_melee" then
                local family = action_families[row.family_id]
                if row.phase == "windup" then
                    paths[#paths + 1] = {
                        source = row,
                        projectile = nil,
                        start_tick = row.phase_ticks + 1,
                        last_tick = row.phase_ticks + assert(family.active_ticks),
                    }
                elseif row.phase == "active" then
                    paths[#paths + 1] = {
                        source = row,
                        projectile = nil,
                        start_tick = 1,
                        last_tick = row.phase_ticks,
                    }
                end
            elseif row.family_id == "ranged" and row.release_latched then
                local spawn = row.projected_spawn_tick - tick
                if spawn >= 0 then
                    paths[#paths + 1] = {
                        source = row,
                        projectile = nil,
                        start_tick = spawn + 1,
                        last_tick = spawn
                            + assert(action_families.ranged.projectile_lifetime_ticks),
                    }
                end
            end
        end
    end
    for _, row in ipairs(observation.projectiles) do
        if
            row.source_team ~= observation.self.team
            and (source_index == nil or row.source_player_index == source_index)
        then
            paths[#paths + 1] = {
                source = nil,
                projectile = row,
                start_tick = 1,
                last_tick = row.horizon_ticks,
            }
        end
    end
    return paths
end

---@param observation CombatObservation
---@param path CombatHostilePath
---@param tick integer
---@return number?, number?
local function hostile_contact_point(observation, path, tick)
    if path.projectile then
        local step = assert(action_families.ranged.projectile_speed_px_per_second)
            * fixed_clock.TICK_SECONDS
        return path.projectile.x + path.projectile.dir_x * step * tick,
            path.projectile.y + path.projectile.dir_y * step * tick
    end
    local row = assert(path.source)
    if row.family_id == "ranged" then
        local spawn = row.projected_spawn_tick - observation.observed_tick
        local travel = tick - spawn
        if travel < 1 then
            return nil, nil
        end
        local body = combat_feasibility.project(row, observation, spawn)
        local fx, fy = unit_or(row.facing_x, row.facing_y, row.team == "home" and 1 or -1, 0)
        local step = assert(action_families.ranged.projectile_speed_px_per_second)
            * fixed_clock.TICK_SECONDS
        return body.x + fx * step * travel, body.y + fy * step * travel
    end
    local body = combat_feasibility.project(row, observation, tick)
    return body.x, body.y
end

-- Guard is self-only. It must intersect the frozen hostile source's melee or
-- projectile contact path inside its catalogued arc on an active guard tick at
-- or before the cap, then release on the next tick. No relevant public path
-- makes guard infeasible; the release tick can never create an intersection.
---@param observation CombatObservation
---@param source_index integer
---@param tape CombatWitnessTape?
---@return CombatFeasibilityWitness
local function guard_feasibility(observation, source_index, tape)
    local family = action_families.guard
    local commit_tick = tape and tape.ticks or 0
    local paths = combat_feasibility.hostile_paths(observation, source_index)
    local cap = family.windup_ticks
    for _, path in ipairs(paths) do
        cap = math.max(cap, path.last_tick)
    end
    local witness = {
        feasible = false,
        family_id = "guard",
        target_player = source_index,
        commit_tick = commit_tick,
        contact_tick = 0,
        horizon_ticks = commit_tick + cap,
    }
    ---@cast witness CombatFeasibilityWitness
    if #paths == 0 then
        return witness
    end
    local guard_arc = math.cos(math.rad(family.front_arc_degrees / 2))
    for relative = family.windup_ticks, cap do
        local sx, sy, fx, fy = committed_source_pose(observation, tape, family, relative)
        for _, path in ipairs(paths) do
            if relative >= path.start_tick and relative <= path.last_tick then
                local hx, hy = hostile_contact_point(observation, path, relative)
                if hx and hy then
                    local dx = hx - sx
                    local dy = hy - sy
                    local distance = length(dx, dy)
                    local reach = observation.self.radius
                    if path.source and not path.projectile then
                        reach = reach + (path.source.projected_reach_px or 0)
                    end
                    if path.source and path.source.family_id == "ranged" then
                        reach = observation.self.radius
                    end
                    local inside_arc = distance <= EPSILON
                        or (dx / distance) * fx + (dy / distance) * fy + EPSILON >= guard_arc
                    if distance <= reach and inside_arc then
                        witness.feasible = true
                        witness.contact_tick = commit_tick + relative
                        return witness
                    end
                end
            end
        end
    end
    return witness
end

-- The soonest tick a public hostile path would reach this player's body, inside
-- `window_ticks`. This is the spacing/evade read: it asks only whether a
-- telegraphed threat lands, never whether it would hit, be guarded, or be
-- superseded.
-- Also returns where the threat IS, which is not the same thing as where its
-- source player stands. An in-flight projectile's shooter may be across the
-- pitch, or dead astern of it; a caller that wants to step out of the way has to
-- read the projectile, not the body that launched it.
---@param observation CombatObservation
---@param window_ticks integer
---@return integer? ticks_to_contact
---@return integer? source_player
---@return number? threat_x -- Current public position of the arriving threat.
---@return number? threat_y
function combat_feasibility.incoming_threat(observation, window_ticks)
    local own = observation.self
    local best_tick ---@type integer?
    local best_source ---@type integer?
    local best_x ---@type number?
    local best_y ---@type number?
    for _, path in ipairs(combat_feasibility.hostile_paths(observation, nil)) do
        local last = math.min(path.last_tick, window_ticks)
        for tick = path.start_tick, last do
            local hx, hy = hostile_contact_point(observation, path, tick)
            if hx and hy then
                local body = combat_feasibility.project(own, observation, tick)
                local reach = body.radius
                if path.source and not path.projectile then
                    reach = reach + (path.source.projected_reach_px or 0)
                end
                if path.source and path.source.family_id == "ranged" then
                    reach = body.radius
                end
                if length(hx - body.x, hy - body.y) <= reach then
                    local source = path.source and path.source.player_index
                        or assert(path.projectile).source_player_index
                    local origin = path.projectile or assert(path.source)
                    if
                        best_tick == nil
                        or tick < best_tick
                        or (tick == best_tick and source < assert(best_source))
                    then
                        best_tick = tick
                        best_source = source
                        best_x = origin.x
                        best_y = origin.y
                    end
                    break
                end
            end
        end
    end
    return best_tick, best_source, best_x, best_y
end

-- `family_commit_feasibility/v1`. Family feasibility ignores which family is
-- actually equipped and ignores actual cooldown, recovery, commitment, request
-- acceptance, and hit outcome; those stay separate policy inputs.
---@param observation CombatObservation
---@param target_player integer -- Frozen target, or the hostile source for guard.
---@param family_id ActionFamilyId
---@param tape CombatWitnessTape?
---@return CombatFeasibilityWitness
function combat_feasibility.family_commit(observation, target_player, family_id, tape)
    assert(action_families[family_id], "unknown action family: " .. tostring(family_id))
    if family_id == "guard" then
        return guard_feasibility(observation, target_player, tape)
    end
    local target = combat_feasibility.row_for(observation, target_player)
    if not target or target.is_keeper or target.team == observation.self.team then
        return {
            feasible = false,
            family_id = family_id,
            target_player = target_player,
            commit_tick = tape and tape.ticks or 0,
            contact_tick = 0,
            horizon_ticks = tape and tape.ticks or 0,
        }
    end
    if family_id == "ranged" then
        return ranged_feasibility(observation, target, tape)
    end
    return melee_feasibility(observation, target, action_families[family_id], tape)
end

---@param observation CombatObservation
---@return CombatObservationPeer?
local function ball_owner_row(observation)
    local owner = observation.ball.owner_player_index
    if owner == 0 or owner == observation.self.player_index then
        return nil
    end
    return combat_feasibility.row_for(observation, owner)
end

---@param observation CombatObservation
---@param target CombatObservationPeer
---@return boolean
local function carrier_protection_predicate(observation, target)
    if observation.ball.owner_team ~= observation.self.team then
        return false
    end
    local owner_index = observation.ball.owner_player_index
    if owner_index == 0 then
        return false
    end
    local owner_x, owner_y
    if owner_index == observation.self.player_index then
        owner_x, owner_y = observation.self.x, observation.self.y
    else
        local owner = combat_feasibility.row_for(observation, owner_index)
        if not owner then
            return false
        end
        owner_x, owner_y = owner.x, owner.y
    end
    if
        length(target.x - owner_x, target.y - owner_y) > combat_feasibility.CARRIER_PROTECTION_PX
    then
        return false
    end
    local best_index, best_distance = nil, math.huge
    for _, row in ipairs(observation.opponents) do
        if not row.is_keeper then
            local distance = length(row.x - owner_x, row.y - owner_y)
            if distance < best_distance - EPSILON then
                best_distance = distance
                best_index = row.player_index
            elseif math.abs(distance - best_distance) <= EPSILON and best_index then
                best_index = math.min(best_index, row.player_index)
            end
        end
    end
    return best_index == target.player_index
end

---@param observation CombatObservation
---@param target CombatObservationPeer
---@return boolean
local function loose_ball_predicate(observation, target)
    if observation.ball.owner_player_index ~= 0 then
        return false
    end
    local own = observation.self
    local radius = combat_feasibility.LOOSE_BALL_PX
    return length(own.x - observation.ball.x, own.y - observation.ball.y) <= radius
        and length(target.x - observation.ball.x, target.y - observation.ball.y) <= radius
end

---@param observation CombatObservation
---@param team InputTeam
---@return number, number
local function attack_goal_center(observation, team)
    local match = observation.match
    if team == "home" then
        return match.goal_away_x + match.goal_away_w, match.goal_away_y + match.goal_away_h / 2
    end
    return match.goal_home_x, match.goal_home_y + match.goal_home_h / 2
end

-- Sole blocker of a frozen candidate passing segment, the frozen likely
-- shooter, or the controller of an open shot lane.
---@param observation CombatObservation
---@param target CombatObservationPeer
---@return boolean
local function lane_or_shot_predicate(observation, target)
    if target.team == observation.self.team then
        return false
    end
    local own = observation.self
    if observation.ball.owner_team == own.team then
        -- Our possession: the target denies one of our frozen passing segments
        -- and is the only opponent standing in it.
        local owner_index = observation.ball.owner_player_index
        local ox, oy
        if owner_index == own.player_index then
            ox, oy = own.x, own.y
        else
            local owner = combat_feasibility.row_for(observation, owner_index)
            if not owner then
                return false
            end
            ox, oy = owner.x, owner.y
        end
        for _, mate in ipairs(observation.teammates) do
            if not mate.is_keeper and mate.player_index ~= owner_index then
                local blockers = 0
                local sole = 0
                for _, row in ipairs(observation.opponents) do
                    if
                        not row.is_keeper
                        and segment_hits_circle(
                            ox,
                            oy,
                            mate.x,
                            mate.y,
                            row.x,
                            row.y,
                            row.radius + LANE_CORRIDOR_PX
                        )
                    then
                        blockers = blockers + 1
                        sole = row.player_index
                    end
                end
                if blockers == 1 and sole == target.player_index then
                    return true
                end
            end
        end
        return false
    end

    -- Their possession or a loose ball: the target is the frozen likely shooter
    -- or controls an open shot lane at our goal.
    local gx, gy = attack_goal_center(observation, target.team)
    if length(target.x - gx, target.y - gy) > SHOT_RANGE_PX then
        return false
    end
    for _, row in ipairs(observation.teammates) do
        if
            not row.is_keeper
            and segment_hits_circle(
                target.x,
                target.y,
                gx,
                gy,
                row.x,
                row.y,
                row.radius + LANE_CORRIDOR_PX
            )
        then
            return false
        end
    end
    if observation.ball.owner_player_index == target.player_index then
        return true
    end
    -- Not the carrier: only the nearest opposing outfielder to our goal counts
    -- as the frozen likely shooter, so a whole team is never in this bucket.
    local best_index, best_distance = nil, math.huge
    for _, row in ipairs(observation.opponents) do
        if not row.is_keeper then
            local distance = length(row.x - gx, row.y - gy)
            if distance < best_distance - EPSILON then
                best_distance = distance
                best_index = row.player_index
            elseif math.abs(distance - best_distance) <= EPSILON and best_index then
                best_index = math.min(best_index, row.player_index)
            end
        end
    end
    return best_index == target.player_index
end

-- Every true purpose predicate for one (source, target) pair, as a bitset.
---@param observation CombatObservation
---@param target_player integer
---@return table<CombatPurposeId, boolean> bitset
---@return integer count
function combat_feasibility.purpose_bitset(observation, target_player)
    local bitset = {}
    local count = 0
    local target = combat_feasibility.row_for(observation, target_player)
    if not target or target.is_keeper or target.team == observation.self.team then
        return bitset, 0
    end
    local carrier = ball_owner_row(observation)
    if carrier and carrier.player_index == target.player_index and not carrier.is_keeper then
        bitset.carrier_contest = true
        count = count + 1
    end
    if carrier_protection_predicate(observation, target) then
        bitset.carrier_protection = true
        count = count + 1
    end
    if loose_ball_predicate(observation, target) then
        bitset.loose_ball_contest = true
        count = count + 1
    end
    if lane_or_shot_predicate(observation, target) then
        bitset.passing_lane_or_shot_denial = true
        count = count + 1
    end
    -- Recovery alone is diagnostic. It becomes a purpose only on top of one of
    -- the four ball/lane predicates.
    if target.phase == "recovery" and count > 0 then
        bitset.recovery_punish = true
        count = count + 1
    end
    return bitset, count
end

---@param bitset table<CombatPurposeId, boolean>
---@return CombatPurposeId?
function combat_feasibility.dominant_purpose(bitset)
    for _, purpose in ipairs(combat_feasibility.PURPOSE_PRIORITY) do
        if bitset[purpose] then
            return purpose
        end
    end
    return nil
end

-- `formation_risk_tradeoff`: a cost flag, never a purpose. Committing raises it
-- when the source is already far from its authored anchor or when the projected
-- committed motion adds at least the contract's gain.
---@param observation CombatObservation
---@param family_id ActionFamilyId
---@return boolean
function combat_feasibility.formation_risk(observation, family_id)
    local own = observation.self
    local anchor_x, anchor_y
    for _, row in ipairs(observation.anchors) do
        if row.player_index == own.player_index then
            anchor_x, anchor_y = row.anchor_x, row.anchor_y
            break
        end
    end
    if not anchor_x or not anchor_y then
        return false
    end
    local now = length(own.x - anchor_x, own.y - anchor_y)
    if now > combat_feasibility.FORMATION_RISK_ANCHOR_PX then
        return true
    end
    local family = action_families[family_id]
    local horizon = family.windup_ticks + (family.active_ticks or 0) + family.recovery_ticks
    local seconds = horizon * fixed_clock.TICK_SECONDS
    local scale = family.movement_multiplier
    local projected_x =
        clamp(own.x + own.vx * scale * seconds, own.radius, observation.match.field_w - own.radius)
    local projected_y =
        clamp(own.y + own.vy * scale * seconds, own.radius, observation.match.field_h - own.radius)
    local later = length(projected_x - anchor_x, projected_y - anchor_y)
    return later - now >= combat_feasibility.FORMATION_RISK_GAIN_PX
end

-- The canonical movement alphabet the envelope search varies. Order is frozen:
-- neutral first, then the eight compass directions clockwise from +x.
combat_feasibility.MOVE_ALPHABET = {
    { x = 0, y = 0 },
    { x = 1, y = 0 },
    { x = 1, y = 1 },
    { x = 0, y = 1 },
    { x = -1, y = 1 },
    { x = -1, y = 0 },
    { x = -1, y = -1 },
    { x = 0, y = -1 },
    { x = 1, y = -1 },
}

---@class CombatEnvelopeOptions
---@field search_ticks integer? -- Defaults to the contract's 30.
---@field families ActionFamilyId[]? -- Defaults to all four catalogued families.

-- `intervention_candidate/v2`: the family-neutral feasibility envelope. A
-- `(target, purpose)` pair enters only when its purpose predicate is true and at
-- least one searched source pose makes `family_commit_feasibility/v1` true for
-- at least one of the four catalogued families toward the same frozen identity.
--
-- The search varies only the source's canonical legal movement/facing inputs,
-- for at most `search_ticks`. Every non-source trajectory is the public
-- no-response projection. Search order is the frozen movement alphabet crossed
-- with the ascending commit tick, so the first feasible commit tick is stable.
---@param observation CombatObservation
---@param options CombatEnvelopeOptions?
---@return CombatInterventionPair[]
function combat_feasibility.intervention_candidates(observation, options)
    options = options or {}
    local search_ticks = options.search_ticks or combat_feasibility.SEARCH_TICKS
    local families = options.families or { "unarmed", "guard", "light_melee", "ranged" }
    local pairs_out = {}
    for _, target in ipairs(observation.opponents) do
        if not target.is_keeper then
            local bitset, count =
                combat_feasibility.purpose_bitset(observation, target.player_index)
            if count > 0 then
                local best_tick ---@type integer?
                local family_bitset = {}
                for commit_tick = 0, search_ticks do
                    for _, move in ipairs(combat_feasibility.MOVE_ALPHABET) do
                        local tape = commit_tick == 0 and nil
                            or { move_x = move.x, move_y = move.y, ticks = commit_tick }
                        for _, family_id in ipairs(families) do
                            if not family_bitset[family_id] then
                                local witness = combat_feasibility.family_commit(
                                    observation,
                                    target.player_index,
                                    family_id,
                                    tape
                                )
                                if witness.feasible then
                                    family_bitset[family_id] = true
                                    best_tick = best_tick or commit_tick
                                end
                            end
                        end
                        if commit_tick == 0 then
                            break
                        end
                    end
                end
                if best_tick then
                    for _, purpose in ipairs(combat_feasibility.PURPOSE_PRIORITY) do
                        if bitset[purpose] then
                            pairs_out[#pairs_out + 1] = {
                                target_player = target.player_index,
                                purpose = purpose,
                                commit_tick = best_tick,
                                family_bitset = family_bitset,
                                formation_risk = false,
                            }
                        end
                    end
                end
            end
        end
    end
    table.sort(pairs_out, function(left, right)
        if left.target_player ~= right.target_player then
            return left.target_player < right.target_player
        end
        return left.purpose < right.purpose
    end)
    return pairs_out
end

return combat_feasibility
