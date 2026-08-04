local identity = require("render.identity")

---@alias BroadcastPhase "kickoff"|"goal"|"replay"|"full_time"

---@class MatchHudContext
---@field home_name string
---@field away_name string
---@field arena_name string
---@field arena_location string
---@field tactic_name string
---@field formation_name string
---@field prompt OnboardingPrompt?
---@field phase BroadcastPhase?
---@field scoring_team "home"|"away"?
---@field combat_enabled boolean?
---@field combat CombatPlayerPresentation?
---@field combat_notice CombatFeedbackNotice?

---@class MatchHudModel
---@field home_name string
---@field away_name string
---@field home_score integer
---@field away_score integer
---@field clock string
---@field venue string
---@field possession string
---@field possession_marker "filled"|"outline"
---@field player_name string
---@field player_detail string
---@field player_state string
---@field species_shape "round"|"broad"|"angular"|"cluster"
---@field species_color number[]
---@field stamina number
---@field plan string
---@field prompt OnboardingPrompt?
---@field announcement_title string?
---@field announcement_detail string?
---@field announcement_kind BroadcastPhase?
---@field equipment_label string?
---@field equipment_state string?
---@field equipment_progress number
---@field feedback_text string?
---@field feedback_glyph string?

---@class MatchHudLayout
---@field venue Rect
---@field scorebug Rect
---@field clock Rect
---@field status Rect
---@field identity Rect
---@field plan Rect
---@field prompt Rect
---@field announcement Rect
---@field combat Rect
---@field scale number

---@class MatchHudModule
local hud = {}

---@param value number
---@return number
local function clamp01(value)
    return math.max(0, math.min(1, value))
end

---@param seconds number
---@return string
function hud.format_clock(seconds)
    local whole = math.max(0, math.floor(seconds))
    return ("%d:%02d"):format(math.floor(whole / 60), whole % 60)
end

-- The HUD reads the scoreboard section of the same versioned render payload the
-- pitch draws from, never a `MatchState`. Only authored match metadata (team
-- names, arena, tactic, onboarding) comes in as context: none of it is
-- simulation state.
---@param scoreboard RenderFrameHud
---@param context MatchHudContext
---@return MatchHudModel
function hud.model(scoreboard, context)
    local presentation = assert(
        identity.for_player(scoreboard.controlled_id),
        "missing presentation identity for " .. scoreboard.controlled_id
    )
    local owner_team = scoreboard.possession_team
    local possession = "LOOSE BALL"
    if owner_team then
        possession = ((owner_team == "home") and context.home_name or context.away_name)
            .. " POSSESSION"
    end

    local player_state = "DEFENDING"
    if scoreboard.controlled_owns_ball then
        player_state = scoreboard.controlled_is_keeper and "KEEPER BALL" or "ON BALL"
    elseif not owner_team then
        player_state = "CONTESTING"
    end

    local title, detail = nil, nil
    if context.phase == "kickoff" then
        title = context.combat_enabled and "COMBAT PROTOTYPE" or "SHOWCASE FIXTURE"
        detail = context.home_name .. "  ·  " .. context.away_name
    elseif context.phase == "goal" then
        local team_name = context.scoring_team == "away" and context.away_name or context.home_name
        title = "GOAL · " .. string.upper(team_name)
        detail = ("%d  —  %d"):format(scoreboard.home_score, scoreboard.away_score)
    elseif context.phase == "replay" then
        title = "REPLAY"
        detail = "[A / SPACE] SKIP"
    elseif context.phase == "full_time" then
        title = "FULL TIME"
        detail = ("%s  %d — %d  %s"):format(
            string.upper(context.home_name),
            scoreboard.home_score,
            scoreboard.away_score,
            string.upper(context.away_name)
        )
    end

    local equipment_label, equipment_state = nil, nil
    local equipment_progress = 0
    if context.combat and context.combat.family_name then
        local combat = assert(context.combat)
        equipment_label = string.upper(combat.family_name)
        if combat.readiness == "ready" then
            equipment_state = "READY"
            equipment_progress = 1
        elseif combat.readiness == "cooldown" then
            equipment_state = "COOLDOWN"
            equipment_progress = 1 - combat.cooldown_fraction
        elseif combat.readiness == "forced" then
            equipment_state = "INTERRUPTED"
            equipment_progress = 0
        elseif combat.readiness == "committed" then
            equipment_state = string.upper(combat.phase)
            equipment_progress = combat.phase_progress
        else
            equipment_state = "UNAVAILABLE"
            equipment_progress = 0
        end
    end
    local notice = context.combat_notice

    return {
        home_name = string.upper(context.home_name),
        away_name = string.upper(context.away_name),
        home_score = scoreboard.home_score,
        away_score = scoreboard.away_score,
        clock = hud.format_clock(scoreboard.time_left),
        venue = string.upper(context.arena_name .. " · " .. context.arena_location),
        possession = string.upper(possession),
        possession_marker = (owner_team ~= nil and owner_team == scoreboard.controlled_team)
                and "filled"
            or "outline",
        player_name = string.upper(presentation.name),
        player_detail = string.upper(presentation.species_name .. " " .. presentation.position),
        player_state = player_state,
        species_shape = scoreboard.species_shape,
        species_color = scoreboard.species_color,
        stamina = clamp01(scoreboard.controlled_stamina),
        plan = string.upper("PLAN · " .. context.tactic_name .. " · " .. context.formation_name),
        prompt = context.prompt,
        announcement_title = title,
        announcement_detail = detail,
        announcement_kind = context.phase,
        equipment_label = equipment_label,
        equipment_state = equipment_state,
        equipment_progress = clamp01(equipment_progress),
        feedback_text = notice and notice.text or nil,
        feedback_glyph = notice and notice.glyph or nil,
    }
end

---@param viewport { w: number, h: number }
---@return MatchHudLayout
function hud.layout(viewport)
    local scale = math.min(viewport.w / 960, viewport.h / 540)
    local ox = (viewport.w - 960 * scale) / 2
    local oy = (viewport.h - 540 * scale) / 2
    ---@param x number
    ---@param y number
    ---@param w number
    ---@param h number
    ---@return Rect
    local function rect(x, y, w, h)
        return {
            x = ox + x * scale,
            y = oy + y * scale,
            w = w * scale,
            h = h * scale,
        }
    end
    return {
        venue = rect(230, 7, 500, 14),
        scorebug = rect(230, 24, 500, 48),
        clock = rect(744, 32, 86, 32),
        status = rect(300, 76, 360, 20),
        identity = rect(24, 452, 340, 64),
        plan = rect(696, 468, 240, 44),
        prompt = rect(270, 402, 420, 52),
        announcement = rect(180, 214, 600, 104),
        combat = rect(696, 436, 240, 28),
        scale = scale,
    }
end

return hud
