-- Human-proxy input driver for the controlled slot in headless matches.
-- Produces a MatchInput per frame the way a *predictable mediocre* player
-- would: decisions refresh only every reaction window (not every frame), aim
-- carries noise, and heuristics are deliberately simple. Balance results are
-- only comparable under the same bot — see docs/design/fun_metrics.md.
--
-- This is a SEPARATELY IDENTIFIED human-proxy population, not the gameplay AI.
-- It emits no equipment intent: every `equipment_*` field below is deliberately
-- false. `gameplay_ai/combat/v1` lives in `sim.combat_policy` and drives the
-- match AI and the declared fixed-slot bot fills; this driver may only gain
-- combat capability behind its own explicit policy id, and must never be
-- reported as the gameplay AI by accident.

local Vec2 = require("core.vec2")
local rng = require("core.rng")
local TUNE = require("sim.tuning").values

local bot = {}

local REACTION = 0.2 -- seconds between decisions (human-ish latency)
local AIM_NOISE = 0.15 -- radians of uniform aim wobble per decision
local SHOT_CHARGE_HOLD = 0.35 -- seconds of shoot_held before releasing
local PASS_CHARGE_HOLD = 0.22 -- short hold reaches a deliberate mid/long outlet
local SHOOT_EAGERNESS = 1.15 -- shoots a bit outside the AI's own range
local PRESSURE_PANIC = 1.3 -- passes when pressed at this x the AI's radius
local JUKE_REACT_DIST = 64 -- sidestep a defender who has committed inside this range
local LONG_OUTLET_DIST = 240 -- charge and sometimes loft passes beyond this distance
local AERIAL_STRIKE_Z = 18 -- first-time intent begins above the ground-control band
local AERIAL_STRIKE_DIST = 84 -- mirrors the match's readable aerial anticipation window
local DEFEND_JOCKEY_DIST = 70 -- shadow instead of chase inside this range
local POKE_DIST = 34 -- attempt the standing tackle inside this range
local SPRINT_MIN_METER = 0.5 -- only sprint on a half-full tank
local CHASE_LEAD = 0.15 -- seconds of ball-velocity lead when chasing

---@class BotState
---@field rng integer  -- own PRNG state: never touches the match's
---@field reaction number
---@field decide_t number  -- countdown to the next decision
---@field move Vec2  -- committed move direction (held between decisions)
---@field sprint boolean
---@field jockey boolean
---@field dash boolean  -- one-shot: consumed by the next frame
---@field dodge boolean  -- one-shot carrier juke
---@field pass boolean  -- one-shot
---@field lob boolean  -- modifier for the queued one-shot action
---@field charge_t number  -- >0: holding shoot, release when it expires
---@field pass_charge_t number  -- >0: holding pass, release when it expires
---@field pass_lob boolean
---@field shot_lob boolean
---@field aerial_strike boolean
---@field aerial_acrobatic boolean

---@param opts { seed: number, reaction: number? }?
---@return BotState
function bot.new(opts)
    opts = opts or {}
    return {
        rng = rng.seed((opts.seed or 1) * 7919 + 17),
        reaction = opts.reaction or REACTION,
        decide_t = 0,
        move = Vec2.new(0, 0),
        sprint = false,
        jockey = false,
        dash = false,
        dodge = false,
        pass = false,
        lob = false,
        charge_t = 0,
        pass_charge_t = 0,
        pass_lob = false,
        shot_lob = false,
        aerial_strike = false,
        aerial_acrobatic = false,
    }
end

-- Uniform sample in [0, 1) from the bot's own stream.
---@param b BotState
---@return number
local function roll(b)
    local s, v = rng.roll(b.rng)
    b.rng = s
    return v
end

-- `dir` rotated by up to +/- AIM_NOISE radians: the bot never aims perfectly.
---@param b BotState
---@param dir Vec2
---@return Vec2
local function noisy(b, dir)
    local a = (roll(b) * 2 - 1) * AIM_NOISE
    local c, s = math.cos(a), math.sin(a)
    return Vec2.new(dir.x * c - dir.y * s, dir.x * s + dir.y * c)
end

---@param s MatchState
---@param from Vec2
---@return number dist
---@return MatchPlayer? who
local function nearest_opponent(s, from)
    local best, who = math.huge, nil
    for _, p in ipairs(s.players) do
        if p.team == "away" and not p.is_keeper then
            local d = from:dist(p.pos)
            if d < best then
                best, who = d, p
            end
        end
    end
    return best, who
end

-- The teammate the bot would pick out: most advanced open outfielder in
-- passing range (crude — the sim's own aim cone does the real target pick).
---@param s MatchState
---@param me MatchPlayer
---@return MatchPlayer?
local function best_outlet(s, me)
    local best, who = -math.huge, nil
    for i, p in ipairs(s.players) do
        if p.team == "home" and not p.is_keeper and i ~= s.controlled then
            local d = me.pos:dist(p.pos)
            if d <= TUNE.PASS_RANGE_MAX then
                local space = nearest_opponent(s, p.pos)
                local value = p.pos.x + space * 0.5 -- advanced and unmarked
                if value > best then
                    best, who = value, p
                end
            end
        end
    end
    return who
end

-- Decide a fresh intent (called once per reaction window).
---@param b BotState
---@param s MatchState
local function decide(b, s)
    local me = s.players[s.controlled]
    local goal = Vec2.new(s.field.w, s.field.h / 2)
    b.sprint, b.jockey, b.dash, b.dodge, b.pass, b.lob = false, false, false, false, false, false
    b.aerial_strike, b.aerial_acrobatic = false, false
    b.shot_lob, b.pass_lob = false, false

    if s.owner == s.controlled then
        if me.is_keeper then
            b.pass = true -- distribute at the first opportunity
            b.move = Vec2.new(0, 0)
            return
        end
        local to_goal = goal:sub(me.pos)
        local pressure, defender = nearest_opponent(s, me.pos)
        local defender_committed = defender ~= nil
            and (defender.tackle_timer > 0 or defender.slide_timer > 0)
        if
            defender_committed
            and pressure <= JUKE_REACT_DIST
            and me.dodge_cd <= 0
            and roll(b) < 0.35
        then
            -- A tool-using human reacts to the defender, not a random timer:
            -- sidestep the committed poke, then reconsider on the next beat.
            b.move = noisy(b, to_goal:normalized())
            b.dodge = true
            return
        end
        if to_goal:length() < TUNE.AI_SHOOT_RANGE * SHOOT_EAGERNESS then
            -- Line up and charge: face the goal while holding the shot.
            b.move = noisy(b, to_goal:normalized())
            b.charge_t = SHOT_CHARGE_HOLD
            b.shot_lob = roll(b) < 0.15 -- occasional chip keeps the loft verb represented
        elseif pressure < TUNE.AI_PASS_PRESSURE * PRESSURE_PANIC then
            local outlet = best_outlet(s, me)
            if outlet then
                b.move = noisy(b, outlet.pos:sub(me.pos):normalized())
                local outlet_dist = me.pos:dist(outlet.pos)
                b.lob = outlet_dist >= LONG_OUTLET_DIST and roll(b) < 0.35
                if outlet_dist >= LONG_OUTLET_DIST then
                    b.pass_charge_t = PASS_CHARGE_HOLD
                    b.pass_lob = b.lob
                else
                    b.pass = true
                end
            else
                b.move = noisy(b, to_goal:normalized()) -- nowhere to go: push on
            end
        else
            b.move = noisy(b, to_goal:normalized())
            b.sprint = me.sprint_meter > SPRINT_MIN_METER
        end
        return
    end

    if s.owner == nil then
        -- Loose ball: run at where it is going, not where it is.
        local aim = s.ball:add(s.ball_vel:scale(CHASE_LEAD))
        local diff = aim:sub(me.pos)
        b.move = diff:length() > 1 and noisy(b, diff:normalized()) or Vec2.new(0, 0)
        b.sprint = diff:length() > 120 and me.sprint_meter > SPRINT_MIN_METER
        if
            s.ball_z >= AERIAL_STRIKE_Z
            and s.ball_vz < 0
            and diff:length() <= AERIAL_STRIKE_DIST
        then
            b.aerial_strike = true
            b.aerial_acrobatic = s.ball_z >= 45 and roll(b) < 0.08
        end
        return
    end

    local owner = s.players[s.owner]
    if owner.team == "home" then
        -- A teammate has it: offer a forward option ahead of the carrier.
        local spot =
            Vec2.new(math.min(owner.pos.x + 140, s.field.w - 60), me.pos.y * 0.7 + s.ball.y * 0.3)
        local diff = spot:sub(me.pos)
        b.move = diff:length() > 12 and noisy(b, diff:normalized()) or Vec2.new(0, 0)
        return
    end

    -- Opponent has it: get goal-side of the ball, contain, poke when close.
    local home_goal = Vec2.new(0, s.field.h / 2)
    local d = me.pos:dist(s.ball)
    if d < POKE_DIST and roll(b) < 0.5 then
        b.dash = true -- commit the tackle (only sometimes: humans hesitate)
    end
    local aim = d < DEFEND_JOCKEY_DIST and s.ball
        or s.ball:add(home_goal:sub(s.ball):normalized():scale(30))
    local diff = aim:sub(me.pos)
    b.move = diff:length() > 1 and noisy(b, diff:normalized()) or Vec2.new(0, 0)
    b.jockey = d < DEFEND_JOCKEY_DIST and not b.dash
    b.sprint = d > 150 and me.sprint_meter > SPRINT_MIN_METER
end

-- Produce this frame's input; call once per match.step with the same dt.
---@param b BotState
---@param s MatchState
---@param dt number
---@return MatchInput
function bot.input(b, s, dt)
    -- Finish an in-flight charged shot before considering anything else.
    if b.charge_t > 0 then
        b.charge_t = b.charge_t - dt
        if s.owner ~= s.controlled then
            b.charge_t = 0 -- lost the ball mid-charge: abort
        elseif b.charge_t > 0 then
            return {
                move = b.move,
                shoot = false,
                shoot_held = true,
                pass = false,
                pass_held = false,
                switch = false,
                dash = false,
                dodge = false,
                lob = b.shot_lob,
                sprint = false,
                jockey = false,
                aerial_strike = false,
                aerial_acrobatic = false,
                equipment_held = false,
                equipment_pressed = false,
                equipment_released = false,
            }
        else
            b.decide_t = b.reaction -- shot away: give it a beat before reacting
            return {
                move = b.move,
                shoot = true,
                shoot_held = false,
                pass = false,
                pass_held = false,
                switch = false,
                dash = false,
                dodge = false,
                lob = b.shot_lob,
                sprint = false,
                jockey = false,
                aerial_strike = false,
                aerial_acrobatic = false,
                equipment_held = false,
                equipment_pressed = false,
                equipment_released = false,
            }
        end
    end

    -- Deliberate long pass: hold to select range, then release with the same
    -- aim and optional loft modifier. Losing the ball aborts the action.
    if b.pass_charge_t > 0 then
        b.pass_charge_t = b.pass_charge_t - dt
        if s.owner ~= s.controlled then
            b.pass_charge_t = 0
            b.pass_lob = false
        elseif b.pass_charge_t > 0 then
            return {
                move = b.move,
                shoot = false,
                shoot_held = false,
                pass = false,
                pass_held = true,
                switch = false,
                dash = false,
                dodge = false,
                lob = b.pass_lob,
                sprint = false,
                jockey = false,
                aerial_strike = false,
                aerial_acrobatic = false,
                equipment_held = false,
                equipment_pressed = false,
                equipment_released = false,
            }
        else
            b.decide_t = b.reaction
            local lob = b.pass_lob
            b.pass_lob = false
            return {
                move = b.move,
                shoot = false,
                shoot_held = false,
                pass = true,
                pass_held = false,
                switch = false,
                dash = false,
                dodge = false,
                lob = lob,
                sprint = false,
                jockey = false,
                aerial_strike = false,
                aerial_acrobatic = false,
                equipment_held = false,
                equipment_pressed = false,
                equipment_released = false,
            }
        end
    end

    b.decide_t = b.decide_t - dt
    if b.decide_t <= 0 then
        b.decide_t = b.reaction * (0.75 + roll(b) * 0.5) -- jittered cadence
        decide(b, s)
    end

    local pass, dash, dodge, lob = b.pass, b.dash, b.dodge, b.lob
    b.pass, b.dash, b.dodge, b.lob = false, false, false, false -- one-frame actions
    return {
        move = b.move,
        shoot = false,
        shoot_held = false,
        pass = pass,
        pass_held = false,
        switch = false,
        dash = dash,
        dodge = dodge,
        lob = lob,
        sprint = b.sprint,
        jockey = b.jockey,
        aerial_strike = b.aerial_strike,
        aerial_acrobatic = b.aerial_acrobatic,
        equipment_held = false,
        equipment_pressed = false,
        equipment_released = false,
    }
end

return bot
