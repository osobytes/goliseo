---@alias OnboardingPromptId "move"|"equipment"|"possession"|"defending"|"keeper"

---@class OnboardingPrompt
---@field id OnboardingPromptId
---@field title string
---@field body string

---@class OnboardingContext
---@field carrying boolean
---@field defending boolean
---@field keeper_holding boolean
---@field moved boolean
---@field shot boolean
---@field passed boolean
---@field defended boolean
---@field equipment_available boolean
---@field equipment_used boolean

---@class MatchOnboardingState
---@field enabled boolean
---@field current OnboardingPromptId?
---@field remaining number
---@field shown table<string, boolean>
---@field combat_enabled boolean

---@class MatchOnboardingModule
local onboarding = {}

local PROMPT_SECONDS = 6

---@type table<OnboardingPromptId, OnboardingPrompt>
local PROMPTS = {
    move = {
        id = "move",
        title = "YOU CONTROL THE DOUBLE-RINGED PLAYER",
        body = "MOVE [LS / WASD]     SPRINT [LB / SHIFT]",
    },
    equipment = {
        id = "equipment",
        title = "COMBAT PROTOTYPE · EQUIPMENT",
        body = "FACE YOUR TARGET     HOLD / TAP [B / J] TO USE EQUIPMENT",
    },
    possession = {
        id = "possession",
        title = "WITH THE BALL",
        body = "HOLD [A / SPACE] TO SHOOT     [X / K] TO PASS",
    },
    defending = {
        id = "defending",
        title = "WIN IT BACK",
        body = "HOLD [A / SPACE] TO JOCKEY     [X / K] SWITCHES",
    },
    keeper = {
        id = "keeper",
        title = "KEEPER IN POSSESSION",
        body = "[X / K] THROW     [A / SPACE] PUNT",
    },
}

---@param shown table<string, boolean>
---@return table<string, boolean>
local function copy_shown(shown)
    local result = {}
    for id, value in pairs(shown) do
        result[id] = value
    end
    return result
end

---@param enabled boolean
---@param combat_enabled boolean?
---@return MatchOnboardingState
function onboarding.new(enabled, combat_enabled)
    local combat = combat_enabled == true
    local active = enabled or combat
    local first = enabled and "move" or (combat and "equipment" or nil)
    return {
        enabled = active,
        current = first,
        remaining = active and PROMPT_SECONDS or 0,
        shown = first and { [first] = true } or {},
        combat_enabled = combat,
    }
end

---@param current OnboardingPromptId
---@param context OnboardingContext
---@return boolean
local function taught_action_used(current, context)
    if current == "move" then
        return context.moved
    elseif current == "equipment" then
        return context.equipment_used
    elseif current == "possession" then
        return context.shot or context.passed
    elseif current == "defending" then
        return context.defended
    elseif current == "keeper" then
        return context.shot or context.passed
    end
    return false
end

---@param state MatchOnboardingState
---@param context OnboardingContext
---@param dt number
---@return MatchOnboardingState
function onboarding.update(state, context, dt)
    if not state.enabled then
        return state
    end

    local next = {
        enabled = state.enabled,
        current = state.current,
        remaining = state.remaining,
        shown = copy_shown(state.shown),
        combat_enabled = state.combat_enabled,
    }
    if next.current then
        next.remaining = math.max(0, next.remaining - dt)
        if next.remaining == 0 or taught_action_used(next.current, context) then
            next.current = nil
        end
    end

    local candidate = nil
    if not next.current then
        if next.combat_enabled and context.equipment_available and not next.shown.equipment then
            candidate = "equipment"
        elseif context.keeper_holding and not next.shown.keeper then
            candidate = "keeper"
        elseif context.carrying and not next.shown.possession then
            candidate = "possession"
        elseif context.defending and not next.shown.defending then
            candidate = "defending"
        end
    end
    if candidate then
        next.current = candidate
        next.remaining = PROMPT_SECONDS
        next.shown[candidate] = true
    end
    return next
end

---@param state MatchOnboardingState
---@return OnboardingPrompt?
function onboarding.prompt(state)
    return state.current and PROMPTS[state.current] or nil
end

return onboarding
