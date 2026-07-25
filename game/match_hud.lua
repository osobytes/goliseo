local identity = require("game.presentation.identity")

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

---@class MatchHudLayout
---@field venue Rect
---@field scorebug Rect
---@field clock Rect
---@field status Rect
---@field identity Rect
---@field plan Rect
---@field prompt Rect
---@field announcement Rect
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

---@param state MatchState
---@param context MatchHudContext
---@return MatchHudModel
function hud.model(state, context)
    local controlled = state.players[state.controlled]
    local presentation = assert(
        identity.for_player(controlled.id),
        "missing presentation identity for " .. controlled.id
    )
    local owner = state.owner and state.players[state.owner] or nil
    local possession = "LOOSE BALL"
    if owner then
        possession = ((owner.team == "home") and context.home_name or context.away_name)
            .. " POSSESSION"
    end

    local player_state = "DEFENDING"
    if state.owner == state.controlled then
        player_state = controlled.is_keeper and "KEEPER BALL" or "ON BALL"
    elseif not owner then
        player_state = "CONTESTING"
    end

    local title, detail = nil, nil
    if context.phase == "kickoff" then
        title = context.combat_enabled and "COMBAT PROTOTYPE" or "SHOWCASE FIXTURE"
        detail = context.home_name .. "  ·  " .. context.away_name
    elseif context.phase == "goal" then
        local team_name = context.scoring_team == "away" and context.away_name or context.home_name
        title = "GOAL · " .. string.upper(team_name)
        detail = ("%d  —  %d"):format(state.score.home, state.score.away)
    elseif context.phase == "replay" then
        title = "REPLAY"
        detail = "[A / SPACE] SKIP"
    elseif context.phase == "full_time" then
        title = "FULL TIME"
        detail = ("%s  %d — %d  %s"):format(
            string.upper(context.home_name),
            state.score.home,
            state.score.away,
            string.upper(context.away_name)
        )
    end

    return {
        home_name = string.upper(context.home_name),
        away_name = string.upper(context.away_name),
        home_score = state.score.home,
        away_score = state.score.away,
        clock = hud.format_clock(state.time_left),
        venue = string.upper(context.arena_name .. " · " .. context.arena_location),
        possession = string.upper(possession),
        possession_marker = owner and owner.team == controlled.team and "filled" or "outline",
        player_name = string.upper(presentation.name),
        player_detail = string.upper(presentation.species_name .. " " .. presentation.position),
        player_state = player_state,
        species_shape = presentation.shape,
        species_color = presentation.palette,
        stamina = clamp01(controlled.sprint_meter),
        plan = string.upper("PLAN · " .. context.tactic_name .. " · " .. context.formation_name),
        prompt = context.prompt,
        announcement_title = title,
        announcement_detail = detail,
        announcement_kind = context.phase,
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
        scale = scale,
    }
end

return hud
