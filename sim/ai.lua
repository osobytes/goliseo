-- Pure steering/selection helpers used by the match AI.

local Vec2 = require("core.vec2")

local ai = {}

-- Bump when this module's steering/selection behaviour changes without moving
-- one of the public constants below — `sim.outfield_ai_policy` hashes it, so
-- this is where a deliberate policy change to the file-local intercept
-- sampling constants is recorded.
ai.VERSION = 1

---@param point Vec2
---@param positions Vec2[]
---@param exclude integer?  -- index to skip (e.g. self)
---@return integer? index  -- index into positions of the closest, or nil if none
function ai.closest(point, positions, exclude)
    local best, best_dist
    for i, p in ipairs(positions) do
        if i ~= exclude then
            local d = point:dist(p)
            if not best_dist or d < best_dist then
                best_dist = d
                best = i
            end
        end
    end
    return best
end

-- Move `pos` toward `target`, covering at most `max_dist`. Returns the new
-- position and the unit direction travelled (zero direction if already there).
---@param pos Vec2
---@param target Vec2
---@param max_dist number
---@return Vec2 new_pos
---@return Vec2 dir
function ai.steer(pos, target, max_dist)
    local to = target:sub(pos)
    local d = to:length()
    if d == 0 then
        return Vec2.new(pos.x, pos.y), Vec2.new(0, 0)
    end
    local dir = to:normalized()
    if d <= max_dist then
        return Vec2.new(target.x, target.y), dir
    end
    return pos:add(dir:scale(max_dist)), dir
end

-- Predict where a moving target will be and aim there (Reynolds "pursuit"): the
-- lead horizon grows with distance, so far targets are led more. Returns the
-- point to chase; callers still pipe it through `steer` for the speed clamp.
---@param pos Vec2
---@param target_pos Vec2
---@param target_vel Vec2
---@param lead number  -- prediction coefficient (seconds per unit distance)
---@return Vec2 point
function ai.pursue(pos, target_pos, target_vel, lead)
    local horizon = lead * pos:dist(target_pos)
    return target_pos:add(target_vel:scale(horizon))
end

-- Point `frac` of the way from `a` to `b`. The marking/cover primitive: stand
-- goal-side of a man (interpose between opponent and goal) or behind the presser.
---@param a Vec2
---@param b Vec2
---@param frac number  -- 0 = a, 1 = b
---@return Vec2 point
function ai.interpose(a, b, frac)
    return a:add(b:sub(a):scale(frac))
end

-- Summed repulsion from neighbours within `radius`, with linear falloff, so
-- players don't collapse onto the same spot. Returns an offset to add to a
-- steering target (zero if nothing is close). Coincident neighbours are skipped.
---@param pos Vec2
---@param others Vec2[]
---@param radius number
---@return Vec2 offset
function ai.separation(pos, others, radius)
    local off = Vec2.new(0, 0)
    for _, o in ipairs(others) do
        local away = pos:sub(o)
        local d = away:length()
        if d > 0 and d < radius then
            off = off:add(away:normalized():scale((radius - d) / radius))
        end
    end
    return off
end

local function sigmoid(x)
    return 1 / (1 + math.exp(-x))
end

-- Shortest distance from point `p` to segment `a`-`b`.
---@param p Vec2
---@param a Vec2
---@param b Vec2
---@return number
local function point_seg_dist(p, a, b)
    local ab = b:sub(a)
    local len2 = ab.x * ab.x + ab.y * ab.y
    if len2 == 0 then
        return p:dist(a)
    end
    local tt = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2
    tt = math.max(0, math.min(1, tt))
    return p:dist(a:add(ab:scale(tt)))
end

-- Off-ball support scoring weights. Public because they ARE gameplay-AI
-- policy: `sim.outfield_ai_policy` hashes them into the frozen policy id, so a
-- change here is visible to the evidence that cites it (#59). The locals below
-- are load-time copies, kept only so the scoring loop stays upvalue-cheap.
ai.IMPORTANCE_K = 4 -- sigmoid steepness over normalized attacking depth
ai.CENTER_SIGMA = 0.28 -- gaussian width toward vertical centre (fraction of field height)
ai.LANE_WIDTH = 26 -- an opponent within this of the pass line blocks the lane
ai.LANE_BLOCK = 0.25 -- score multiplier when the passing lane is blocked

local IMPORTANCE_K = ai.IMPORTANCE_K
local CENTER_SIGMA = ai.CENTER_SIGMA
local LANE_WIDTH = ai.LANE_WIDTH
local LANE_BLOCK = ai.LANE_BLOCK

-- Pick the best off-ball support point: the candidate that is most open (far from
-- opponents), in a valuable area (upfield toward the attacking goal x central),
-- and reachable by a clear straight pass from the carrier. Deterministic; ties
-- resolve to the lowest candidate index. `attack_dir` is +1 (attack +x) or -1.
---@param carrier_pos Vec2
---@param candidates Vec2[]
---@param opponents Vec2[]
---@param attack_dir number
---@param field { w: number, h: number }
---@return Vec2 best  -- the carrier's own position if there are no candidates
function ai.support_spot(carrier_pos, candidates, opponents, attack_dir, field)
    local best, best_score
    for _, c in ipairs(candidates) do
        local open = field.w
        for _, o in ipairs(opponents) do
            open = math.min(open, c:dist(o))
        end
        local depth = (attack_dir >= 0) and (c.x / field.w) or (1 - c.x / field.w)
        local imp_x = sigmoid(IMPORTANCE_K * (depth - 0.5))
        local cy = (c.y - field.h / 2) / (field.h * CENTER_SIGMA)
        local imp_y = math.exp(-cy * cy)
        local lane = 1
        for _, o in ipairs(opponents) do
            if point_seg_dist(o, carrier_pos, c) < LANE_WIDTH then
                lane = LANE_BLOCK
                break
            end
        end
        local score = open * imp_x * imp_y * lane
        if not best_score or score > best_score then
            best_score = score
            best = c
        end
    end
    return best or carrier_pos
end

-- If any of `points` lies within `width` of the segment from->to (excluding the
-- very ends), return the lane-fraction (0..1) of the closest such blocker, else
-- nil. Used to decide whether a pass lane is blocked and where to lob over.
---@param from Vec2
---@param to Vec2
---@param points Vec2[]
---@param width number
---@return number? fraction
function ai.lane_blocker(from, to, points, width)
    local ab = to:sub(from)
    local len2 = ab.x * ab.x + ab.y * ab.y
    if len2 < 1 then
        return nil
    end
    local best_f, best_d
    for _, p in ipairs(points) do
        local f = ((p.x - from.x) * ab.x + (p.y - from.y) * ab.y) / len2
        if f > 0.1 and f < 0.95 then
            local d = p:dist(from:add(ab:scale(f)))
            if d < width and (not best_d or d < best_d) then
                best_d, best_f = d, f
            end
        end
    end
    return best_f
end

-- Interception model for a driven ground pass. Friction sheds a fraction of the
-- ball's speed per second (dv/dt = -friction * v), which makes the decay linear
-- in distance: after covering d the ball moves at launch - friction * d, and it
-- took ln(launch / (launch - friction * d)) / friction seconds to get there.
-- An opponent cuts the pass out if it can reach some point of the flight before
-- the ball does — but only where the ball has slowed below the collection cap
-- (a faster ball rolls straight past everyone), and only after a fixed reaction
-- delay (a chaser must read the pass and turn before it runs flat out).
local INTERCEPT_REACT = 0.1 -- seconds before a threat is at full chase
local INTERCEPT_F0 = 0.1 -- sample window start (lane fraction)
local INTERCEPT_F1 = 0.7 -- window end: past this the receiver meets the ball
local INTERCEPT_STEP = 0.05

---@class Threat
---@field pos Vec2
---@field speed number  -- px/s

-- Earliest point of a friction-decayed ground pass that some threat reaches
-- before the ball. Returns its lane fraction (0..1) — a ready-made lob-over
-- point — or nil when the pass outruns every threat. Closed-form and sampled on
-- a fixed grid: deterministic.
---@param from Vec2
---@param to Vec2
---@param launch_speed number  -- px/s at release
---@param friction number  -- fraction of ball speed shed per second
---@param threats Threat[]
---@param reach number  -- a threat collects the ball within this radius
---@param max_collect_speed number  -- a ball at/above this speed can't be collected
---@return number? fraction
function ai.pass_intercept(from, to, launch_speed, friction, threats, reach, max_collect_speed)
    local total = from:dist(to)
    if total < 1 or #threats == 0 then
        return nil
    end
    local dir = to:sub(from):normalized()
    local steps = math.floor((INTERCEPT_F1 - INTERCEPT_F0) / INTERCEPT_STEP + 0.5)
    for i = 0, steps do
        local f = INTERCEPT_F0 + i * INTERCEPT_STEP
        local d = f * total
        local v = launch_speed - friction * d
        if v <= 1 then
            return f -- the ball dies on the lane: anyone can walk onto it
        end
        if v < max_collect_speed then
            local t_ball = math.log(launch_speed / v) / friction
            local point = from:add(dir:scale(d))
            for _, th in ipairs(threats) do
                local t_threat = INTERCEPT_REACT
                    + math.max(0, point:dist(th.pos) - reach) / th.speed
                if t_threat <= t_ball then
                    return f
                end
            end
        end
    end
    return nil
end

-- Assign defenders to opponents (man-marking) with a stable greedy matching.
-- Pairs are ranked by distance with (defender, opponent) index tiebreaks, making
-- the sort a total order -> fully deterministic. A `stick_bonus` discount on the
-- previous tick's pair adds hysteresis so two defenders don't swap the same mark
-- every frame. Returns a `defender_index -> opponent_index` map (partial if the
-- counts differ).
---@param defenders Vec2[]
---@param opponents Vec2[]
---@param prev_map table<integer, integer>?  -- last tick's assignment
---@param stick_bonus number?  -- cost discount to keep a prior pair (px)
---@return table<integer, integer>
function ai.assign_marks(defenders, opponents, prev_map, stick_bonus)
    prev_map = prev_map or {}
    stick_bonus = stick_bonus or 0
    local list = {}
    for di, dp in ipairs(defenders) do
        for oi, op in ipairs(opponents) do
            local cost = dp:dist(op)
            if prev_map[di] == oi then
                cost = cost - stick_bonus
            end
            list[#list + 1] = { d = di, o = oi, cost = cost }
        end
    end
    table.sort(list, function(a, b)
        if a.cost ~= b.cost then
            return a.cost < b.cost
        end
        if a.d ~= b.d then
            return a.d < b.d
        end
        return a.o < b.o
    end)
    local result, d_taken, o_taken = {}, {}, {}
    for _, pr in ipairs(list) do
        if not d_taken[pr.d] and not o_taken[pr.o] then
            result[pr.d] = pr.o
            d_taken[pr.d] = true
            o_taken[pr.o] = true
        end
    end
    return result
end

return ai
