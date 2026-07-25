-- Pure aerial-contact geometry and outcome resolution. This module knows no
-- match state or rendering; sim.match gathers candidates and applies results.

local Vec2 = require("core.vec2")
local rng = require("core.rng")
local TUNE = require("sim.tuning").values
local species = require("sim.species")

---@alias AerialIntent "receive"|"strike"|"acrobatic"
---@alias AerialStyle "leg_control"|"chest_control"|"volley"|"header"|"bicycle"
---@alias AerialOutcome "clean"|"heavy"|"miss"

---@class AerialContext
---@field ball_pos Vec2
---@field ball_vel Vec2
---@field ball_z number
---@field ball_vz number
---@field player_pos Vec2
---@field player_vel Vec2
---@field facing Vec2
---@field move_speed number
---@field skill number
---@field strength number  -- normalized 0..1
---@field opponent_distance number
---@field anticipated boolean
---@field instability number  -- normalized 0..1
---@field extra_reach number
---@field extra_lift number

---@class AerialContact
---@field style AerialStyle
---@field jumping boolean
---@field jump_lift number
---@field jump_ratio number
---@field difficulty number
---@field reach number
---@field reach_ratio number

---@class AerialResolution
---@field contact AerialContact
---@field outcome AerialOutcome
---@field rng integer
---@field angle_error number
---@field weight_error number
---@field contact_probability number
---@field clean_probability number

---@class AerialStyleConfig
---@field style AerialStyle
---@field min_z number
---@field max_z number
---@field max_jump number
---@field reach number
---@field base_difficulty number

local ANTICIPATION_BONUS = 0.08
local PRESSURE_RADIUS = 64
local JUMP_POSE_THRESHOLD = 4
local BICYCLE_BEHIND_COS = 0.15
local BICYCLE_OVERHEAD_DIST = 8
local HUMAN_ASSIST_REACH = 12
local AERIAL_LOCK_TIME = 0.1
local AERIAL_RECEIVE_TIME = 0.75
local AERIAL_CD = 0.5
local RECOVERY_RECEIVE = 0.18
local RECOVERY_STAND = 0.22
local RECOVERY_JUMP = 0.35
local RECOVERY_BICYCLE = 0.6
local BICYCLE_SPEED = 1.4

---@type table<AerialStyle, AerialStyleConfig>
local STYLES = {
    leg_control = {
        style = "leg_control",
        min_z = 18,
        max_z = 42,
        max_jump = 22,
        reach = 28,
        base_difficulty = 0,
    },
    chest_control = {
        style = "chest_control",
        min_z = 34,
        max_z = 62,
        max_jump = 24,
        reach = 26,
        base_difficulty = 0.05,
    },
    volley = {
        style = "volley",
        min_z = 18,
        max_z = 38,
        max_jump = 16,
        reach = 30,
        base_difficulty = 0.12,
    },
    header = {
        style = "header",
        min_z = 44,
        max_z = 72,
        max_jump = 30,
        reach = 30,
        base_difficulty = 0.06,
    },
    bicycle = {
        style = "bicycle",
        min_z = 38,
        max_z = 68,
        max_jump = 24,
        reach = 24,
        base_difficulty = 0.28,
    },
}

---@class Aerial
local aerial = {}

-- The height above which NO style can touch the ball, even at a full jump —
-- a flight that stays above this over an opponent simply cannot be attacked.
-- (Species lift bonuses come on top; leave a margin when using this.)
aerial.MAX_TOUCH_Z = 0
for _, cfg in pairs(STYLES) do
    aerial.MAX_TOUCH_Z = math.max(aerial.MAX_TOUCH_Z, cfg.max_z + cfg.max_jump)
end

---@param value number
---@param low number
---@param high number
---@return number
local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

---@param value number
---@param easy number
---@param hard number
---@return number
local function normalize(value, easy, hard)
    return clamp((value - easy) / (hard - easy), 0, 1)
end

---@param a Vec2
---@param b Vec2
---@return number
local function dot(a, b)
    return a.x * b.x + a.y * b.y
end

---@param context AerialContext
---@return number cosine
local function ball_facing_cos(context)
    local to_ball = context.ball_pos:sub(context.player_pos)
    if to_ball:length() <= 1 then
        return 0
    end
    return dot(context.facing:normalized(), to_ball:normalized())
end

---@param context AerialContext
---@return boolean
local function bicycle_alignment_valid(context)
    local distance = context.player_pos:dist(context.ball_pos)
    return distance <= BICYCLE_OVERHEAD_DIST or ball_facing_cos(context) <= BICYCLE_BEHIND_COS
end

---@param context AerialContext
---@param config AerialStyleConfig
---@return AerialContact?
local function build_contact(context, config)
    if context.ball_z < config.min_z then
        return nil
    end
    if config.style == "bicycle" and not bicycle_alignment_valid(context) then
        return nil
    end

    local max_jump = config.max_jump + context.extra_lift
    local jump_lift = math.max(0, context.ball_z - config.max_z)
    if jump_lift > max_jump then
        return nil
    end

    local reach = config.reach + context.extra_reach
    local distance = context.player_pos:dist(context.ball_pos)
    if distance > reach then
        return nil
    end

    local jump_ratio = (max_jump > 0) and (jump_lift / max_jump) or 0
    local reach_ratio = (reach > 0) and (distance / reach) or 0
    local relative_pace = context.ball_vel:sub(context.player_vel):length()
    local horizontal_difficulty = normalize(relative_pace, 80, 600)
    local drop_difficulty = normalize(math.max(0, -context.ball_vz), 80, 500)
    local facing_cos = ball_facing_cos(context)
    local alignment_error
    if config.style == "bicycle" then
        -- Ideal contact is overhead to moderately behind, around cosine -0.5.
        alignment_error = clamp(math.abs(facing_cos + 0.5) / 1.5, 0, 1)
    else
        alignment_error = clamp((1 - facing_cos) * 0.5, 0, 1)
    end
    local pressure = clamp(1 - context.opponent_distance / PRESSURE_RADIUS, 0, 1)
    local anticipation = context.anticipated and ANTICIPATION_BONUS or 0
    local difficulty = clamp(
        config.base_difficulty
            + 0.24 * reach_ratio * reach_ratio
            + 0.18 * horizontal_difficulty
            + 0.12 * drop_difficulty
            + 0.16 * jump_ratio
            + 0.10 * alignment_error
            + 0.10 * clamp(context.instability, 0, 1)
            + 0.10 * pressure
            - anticipation,
        0,
        1
    )

    return {
        style = config.style,
        jumping = config.style == "bicycle" or jump_lift > JUMP_POSE_THRESHOLD,
        jump_lift = jump_lift,
        jump_ratio = jump_ratio,
        difficulty = difficulty,
        reach = reach,
        reach_ratio = reach_ratio,
    }
end

---@param contacts AerialContact[]
---@param contact AerialContact?
local function append(contacts, contact)
    if contact then
        contacts[#contacts + 1] = contact
    end
end

---@param context AerialContext
---@param intent AerialIntent
---@return AerialContact[]
function aerial.contacts(context, intent)
    local contacts = {}
    if intent == "receive" then
        append(contacts, build_contact(context, STYLES.leg_control))
        append(contacts, build_contact(context, STYLES.chest_control))
    elseif intent == "strike" then
        append(contacts, build_contact(context, STYLES.volley))
        append(contacts, build_contact(context, STYLES.header))
    else
        append(contacts, build_contact(context, STYLES.bicycle))
        if #contacts == 0 then
            append(contacts, build_contact(context, STYLES.volley))
            append(contacts, build_contact(context, STYLES.header))
        end
    end
    return contacts
end

---@param context AerialContext
---@param intent AerialIntent
---@return AerialContact?
function aerial.best_contact(context, intent)
    local contacts = aerial.contacts(context, intent)
    local best
    for _, contact in ipairs(contacts) do
        if not best or contact.difficulty < best.difficulty then
            best = contact
        end
    end
    return best
end

---@param context AerialContext
---@param contact AerialContact
---@param rng_state integer
---@return AerialResolution
function aerial.resolve(context, contact, rng_state)
    local roll_contact, roll_clean, roll_angle, roll_weight
    rng_state, roll_contact = rng.roll(rng_state)
    rng_state, roll_clean = rng.roll(rng_state)
    rng_state, roll_angle = rng.roll(rng_state)
    rng_state, roll_weight = rng.roll(rng_state)

    local margin = clamp(context.skill, 0, 1) - contact.difficulty
    local contact_probability = clamp(0.82 + 0.35 * margin, 0.30, 0.995)
    local clean_probability = clamp(0.58 + 0.60 * margin, 0.08, 0.97)
    local outcome ---@type AerialOutcome
    if roll_contact >= contact_probability then
        outcome = "miss"
    elseif roll_clean < clean_probability then
        outcome = "clean"
    else
        outcome = "heavy"
    end

    local acrobatic = contact.style == "bicycle"
    local max_angle
    local max_weight
    if outcome == "clean" then
        max_angle = acrobatic and 0.14 or 0.06
        max_weight = acrobatic and 0.12 or 0.08
    elseif outcome == "heavy" then
        max_angle = acrobatic and 0.9 or 0.55
        max_weight = acrobatic and 0.6 or 0.45
    else
        max_angle = 0
        max_weight = 0
    end

    return {
        contact = contact,
        outcome = outcome,
        rng = rng_state,
        angle_error = (roll_angle * 2 - 1) * max_angle,
        weight_error = (roll_weight * 2 - 1) * max_weight,
        contact_probability = contact_probability,
        clean_probability = clean_probability,
    }
end

---@param context AerialContext
---@param contact AerialContact
---@param intent_bonus number
---@param jump_edge number
---@param jitter number  -- normalized -1..1
---@return number
function aerial.claim_score(context, contact, intent_bonus, jump_edge, jitter)
    local position_quality = clamp(1 - contact.reach_ratio, 0, 1)
    return 0.45 * position_quality
        + 0.35 * clamp(context.skill, 0, 1)
        + 0.10 * clamp(context.strength, 0, 1)
        + intent_bonus
        + jump_edge
        + clamp(jitter, -1, 1) * 0.04
end

---@class AerialMatchConfig
---@field ground_grab_height number
---@field stick_ahead number
---@field gravity number
---@field release_cd number
---@field clear_header_speed number
---@field volley_speed number

---@class MatchAerialCandidate
---@field index integer
---@field context AerialContext
---@field contact AerialContact
---@field intent AerialIntent
---@field score number

---@param s MatchState
---@param player_idx integer
---@return boolean
local function is_human_player(s, player_idx)
    if s.slot_mode then
        return s.slot_for_player[player_idx] ~= nil
    end
    return s.human_controlled and player_idx == s.controlled
end

---@return MatchInput
local function neutral_input()
    return {
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
        aerial_strike = false,
        aerial_acrobatic = false,
        equipment_held = false,
        equipment_pressed = false,
        equipment_released = false,
    }
end

---@param input MatchInput
---@return boolean
function aerial.strike_requested(input)
    if input.aerial_strike ~= nil then
        return input.aerial_strike
    end
    return input.jockey or input.dash
end

---@param input MatchInput
---@return boolean
function aerial.acrobatic_requested(input)
    if input.aerial_acrobatic ~= nil then
        return input.aerial_acrobatic
    end
    return input.lob and aerial.strike_requested(input)
end

---@param s MatchState
---@param player MatchPlayer
---@return number pixels
local function nearest_opponent(s, player)
    local nearest = math.huge
    for _, opponent in ipairs(s.players) do
        if opponent.team ~= player.team then
            nearest = math.min(nearest, player.pos:dist(opponent.pos))
        end
    end
    return nearest
end

---@param player MatchPlayer
---@param style AerialStyle
---@return number
local function style_skill(player, style)
    if style == "leg_control" or style == "chest_control" then
        return player.first_touch
    elseif style == "header" then
        return player.header_skill
    elseif style == "volley" then
        return player.volley_skill
    end
    return player.bicycle_skill
end

---@param s MatchState
---@param player_idx integer
---@param skill number
---@return AerialContext
local function make_match_context(s, player_idx, skill)
    local player = s.players[player_idx]
    local human_assist = is_human_player(s, player_idx)
            and math.min(HUMAN_ASSIST_REACH, TUNE.AERIAL_ASSIST)
        or 0
    local instability = clamp(player.run_vel:length() / player.move_speed, 0, 1) * 0.6
    if player.sprinting then
        instability = math.min(1, instability + 0.25)
    end
    return {
        ball_pos = s.ball,
        ball_vel = s.ball_vel,
        ball_z = s.ball_z,
        ball_vz = s.ball_vz,
        player_pos = player.pos,
        player_vel = player.vel,
        facing = player.facing,
        move_speed = player.move_speed,
        skill = skill,
        strength = player.strength,
        opponent_distance = nearest_opponent(s, player),
        anticipated = player.receive_timer > 0,
        instability = instability,
        extra_reach = human_assist + species.jump_reach(player.owned_verb),
        extra_lift = species.jump_lift(player.owned_verb),
    }
end

---@param s MatchState
---@param player_idx integer
---@param intent AerialIntent
---@return AerialContext?
---@return AerialContact?
local function contact_for_intent(s, player_idx, intent)
    local player = s.players[player_idx]
    local context = make_match_context(s, player_idx, player.first_touch)
    local contact = aerial.best_contact(context, intent)
    if not contact then
        return nil, nil
    end
    context.skill = style_skill(player, contact.style)
    return context, contact
end

---@param s MatchState
---@param player_idx integer
---@param inputs table<integer, MatchInput>
---@return AerialIntent
local function match_intent(s, player_idx, inputs)
    local player = s.players[player_idx]
    if is_human_player(s, player_idx) then
        local input = inputs[player_idx] or neutral_input()
        if aerial.strike_requested(input) then
            return aerial.acrobatic_requested(input) and "acrobatic" or "strike"
        end
        return "receive"
    end

    local goal = player.team == "home" and s.goal_away or s.goal_home
    local goal_line =
        Vec2.new((player.team == "home") and goal.x or (goal.x + goal.w), goal.y + goal.h / 2)
    local own_third = (player.team == "home") and (player.pos.x < s.field.w * 0.33)
        or (player.team == "away" and player.pos.x > s.field.w * 0.67)
    if own_third or player.pos:dist(goal_line) <= TUNE.AI_HEADER_RANGE then
        return "strike"
    end
    return "receive"
end

---@param s MatchState
---@param inputs table<integer, MatchInput>
---@param ineligible table<integer, boolean>?
---@return MatchAerialCandidate?
local function choose_candidate(s, inputs, ineligible)
    local best ---@type MatchAerialCandidate?
    for i, player in ipairs(s.players) do
        if
            not (ineligible and ineligible[i])
            and not player.is_keeper
            and player.header_cd <= 0
            and player.aerial_recovery <= 0
            and player.stun_timer <= 0
            and player.slide_timer <= 0
            and player.dodge_timer <= 0
        then
            local intent = match_intent(s, i, inputs)
            local context, contact = contact_for_intent(s, i, intent)

            if intent == "strike" and not is_human_player(s, i) then
                local acro_context, acro_contact = contact_for_intent(s, i, "acrobatic")
                if acro_contact and acro_contact.style == "bicycle" and acro_context then
                    local acro_margin = acro_context.skill - acro_contact.difficulty
                    local normal_margin = context
                            and contact
                            and (context.skill - contact.difficulty)
                        or -math.huge
                    if acro_margin > normal_margin then
                        intent = "acrobatic"
                        context, contact = acro_context, acro_contact
                    end
                end
            end

            if context and contact then
                local intent_bonus = player.receive_timer > 0 and 0.12 or 0
                if is_human_player(s, i) and intent ~= "receive" then
                    intent_bonus = intent_bonus + 0.08
                end
                local score = aerial.claim_score(context, contact, intent_bonus, 0, 0)
                if
                    not best
                    or score > best.score
                    or (score == best.score and player.id < s.players[best.index].id)
                then
                    best = {
                        index = i,
                        context = context,
                        contact = contact,
                        intent = intent,
                        score = score,
                    }
                end
            end
        end
    end
    return best
end

---@param vector Vec2
---@param angle number
---@return Vec2
local function rotate_vector(vector, angle)
    local ca, sa = math.cos(angle), math.sin(angle)
    return Vec2.new(vector.x * ca - vector.y * sa, vector.x * sa + vector.y * ca)
end

---@param player MatchPlayer
---@param contact AerialContact
---@param outcome AerialOutcome
---@param intent AerialIntent
local function begin_action(player, contact, outcome, intent)
    local recovery
    if contact.style == "bicycle" then
        recovery = RECOVERY_BICYCLE
    elseif intent == "receive" then
        recovery = contact.jumping and RECOVERY_JUMP or RECOVERY_RECEIVE
    elseif contact.jumping then
        recovery = RECOVERY_JUMP
    else
        recovery = RECOVERY_STAND
    end
    player.header_cd = AERIAL_CD
    player.aerial_timer = recovery
    player.aerial_style = contact.style
    player.aerial_outcome = outcome
    player.aerial_jump = contact.jump_ratio
    player.aerial_recovery = recovery
end

---@param s MatchState
---@param candidate MatchAerialCandidate
---@param inputs table<integer, MatchInput>
---@param resolution AerialResolution
---@param config AerialMatchConfig
local function apply_reception(s, candidate, inputs, resolution, config)
    local player = s.players[candidate.index]
    local input = inputs[candidate.index] or neutral_input()
    s.events[#s.events + 1] = {
        kind = "reception",
        x = s.ball.x,
        y = s.ball.y,
        player = player.id,
        style = candidate.contact.style,
        outcome = resolution.outcome,
        jumping = candidate.contact.jumping,
        difficulty = candidate.contact.difficulty,
    }
    if resolution.outcome == "miss" then
        player.receive_timer = 0
        return
    end

    for i, other in ipairs(s.players) do
        if i ~= candidate.index then
            other.receive_timer = 0
        end
    end
    player.receive_timer = math.max(player.receive_timer, AERIAL_RECEIVE_TIME)

    local aim = player.facing
    if is_human_player(s, candidate.index) and (input.move.x ~= 0 or input.move.y ~= 0) then
        aim = input.move:normalized()
    end
    local duration = (candidate.contact.style == "chest_control") and 0.22 or 0.12
    if resolution.outcome == "heavy" then
        duration = duration * 1.35
    end
    local target = player.pos:add(player.vel:scale(duration)):add(aim:scale(config.stick_ahead))
    local travel = rotate_vector(target:sub(s.ball), resolution.angle_error)
    local weight = clamp(1 + resolution.weight_error, 0.45, 1.45)
    s.ball_vel = travel:scale(weight / duration)
    s.ball_vz = (0.5 * config.gravity * duration * duration - s.ball_z) / duration
    s.ball_spin = 0
end

---@param s MatchState
---@param candidate MatchAerialCandidate
---@param inputs table<integer, MatchInput>
---@param resolution AerialResolution
---@param config AerialMatchConfig
local function apply_strike(s, candidate, inputs, resolution, config)
    local player = s.players[candidate.index]
    local input = inputs[candidate.index] or neutral_input()
    local style = candidate.contact.style
    assert(style == "header" or style == "volley" or style == "bicycle", "strike style")
    ---@type "header"|"volley"|"bicycle"
    local kind
    if style == "header" then
        kind = "header"
    elseif style == "volley" then
        kind = "volley"
    else
        kind = "bicycle"
    end
    s.events[#s.events + 1] = {
        kind = kind,
        x = s.ball.x,
        y = s.ball.y,
        player = player.id,
        style = style,
        outcome = resolution.outcome,
        jumping = candidate.contact.jumping,
        difficulty = candidate.contact.difficulty,
    }
    for _, other in ipairs(s.players) do
        other.receive_timer = 0
    end
    if resolution.outcome == "miss" then
        return
    end

    local own_third = (player.team == "home") and (player.pos.x < s.field.w * 0.33)
        or (player.team == "away" and player.pos.x > s.field.w * 0.67)
    local target
    local defensive = own_third and not is_human_player(s, candidate.index)
    if defensive then
        target = player.pos:add(
            Vec2.new(
                (player.team == "home") and 240 or -240,
                (s.ball.y < s.field.h / 2) and -72 or 72
            )
        )
    elseif is_human_player(s, candidate.index) and (input.move.x ~= 0 or input.move.y ~= 0) then
        target = player.pos:add(input.move:normalized():scale(240))
    else
        local goal = player.team == "home" and s.goal_away or s.goal_home
        local keeper
        for _, other in ipairs(s.players) do
            if other.team ~= player.team and other.is_keeper then
                keeper = other
                break
            end
        end
        local vbias = 0.85
        if keeper then
            vbias = (keeper.pos.y < goal.y + goal.h / 2) and 0.85 or -0.85
        end
        local goal_x = (player.team == "home") and goal.x or (goal.x + goal.w)
        local half = goal.h / 2 - 8
        target = Vec2.new(goal_x, goal.y + goal.h / 2 + vbias * half)
    end

    local direction = rotate_vector(target:sub(player.pos):normalized(), resolution.angle_error)
    local speed
    local vz
    if defensive then
        speed = config.clear_header_speed
        vz = resolution.outcome == "clean" and 260 or 340
    elseif style == "header" then
        speed = player.shot_speed * TUNE.HEADER_SPEED
        vz = resolution.outcome == "clean" and -40 or 100
    elseif style == "volley" then
        speed = player.shot_speed * config.volley_speed
        vz = resolution.outcome == "clean" and 40 or (260 + math.abs(resolution.angle_error) * 300)
    else
        speed = player.shot_speed * BICYCLE_SPEED
        vz = resolution.outcome == "clean" and 30 or (360 + math.abs(resolution.angle_error) * 240)
    end
    if resolution.outcome == "heavy" then
        speed = speed * 0.65
    end
    speed = speed * clamp(1 + resolution.weight_error, 0.4, 1.35)
    s.ball_vel = direction:scale(speed)
    s.ball_vz = vz
    s.ball_spin = 0
    s.pickup_cd = config.release_cd * 0.6
end

---@param s MatchState
---@param inputs table<integer, MatchInput>
---@param config AerialMatchConfig
---@param ineligible table<integer, boolean>?
---@return boolean trajectory_changed
function aerial.resolve_play(s, inputs, config, ineligible)
    if
        s.ball_z <= config.ground_grab_height
        or s.ball_vz >= 0
        or s.pickup_cd > 0
        or s.aerial_lock > 0
    then
        return false
    end
    local candidate = choose_candidate(s, inputs, ineligible)
    if not candidate then
        return false
    end
    local resolution = aerial.resolve(candidate.context, candidate.contact, s.rng)
    s.rng = resolution.rng
    begin_action(
        s.players[candidate.index],
        candidate.contact,
        resolution.outcome,
        candidate.intent
    )
    s.aerial_lock = AERIAL_LOCK_TIME
    if candidate.intent == "receive" then
        apply_reception(s, candidate, inputs, resolution, config)
    else
        apply_strike(s, candidate, inputs, resolution, config)
    end
    return resolution.outcome ~= "miss"
end

return aerial
