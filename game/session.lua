local contract = require("game.match_contract")
local teams = require("data.teams")

---@class GameSession
---@field starter_ids string[]
---@field formation_id string
---@field tactic_id string
---@field setup_step "squad"|"formation"|"tactic"
---@field last_result ProductMatchResult?
---@field first_match boolean
---@field match_number integer
---@field combat_enabled boolean

---@alias ResultAction "rematch"|"change_plan"|"change_lineup"|"main_menu"

---@class SessionModule
local session = {}

---@param ids string[]
---@return string[]
local function copy_ids(ids)
    local result = {}
    for i, id in ipairs(ids) do
        result[i] = id
    end
    return result
end

---@return GameSession
function session.new()
    return {
        starter_ids = copy_ids(teams.nebula.roster),
        formation_id = teams.nebula.formation,
        tactic_id = "balanced",
        setup_step = "squad",
        last_result = nil,
        first_match = true,
        match_number = 0,
        combat_enabled = false,
    }
end

---@param state GameSession
---@param ids string[]
---@return boolean?, string?
function session.set_starters(state, ids)
    local ok, err = contract.validate_starters(ids, teams.nebula)
    if not ok then
        return nil, err
    end
    state.starter_ids = copy_ids(ids)
    return true
end

---@param state GameSession
---@param formation_id string
function session.set_formation(state, formation_id)
    state.formation_id = formation_id
end

---@param state GameSession
---@param tactic_id string
function session.set_tactic(state, tactic_id)
    state.tactic_id = tactic_id
end

---@param state GameSession
---@param enabled boolean
function session.set_combat_enabled(state, enabled)
    state.combat_enabled = enabled
end

---@param state GameSession
---@param seed integer?
---@return ProductMatchRequest?, string?
function session.build_request(state, seed)
    return contract.new_request({
        home_team_id = "nebula",
        away_team_id = "orion",
        home_starter_ids = state.starter_ids,
        formation_id = state.formation_id,
        tactic_id = state.tactic_id,
        arena_id = "helios_crown",
        show_onboarding = state.first_match,
        combat_enabled = state.combat_enabled,
        seed = seed,
    })
end

---@param state GameSession
---@param result ProductMatchResult
function session.record_result(state, result)
    state.last_result = result
    state.first_match = false
    state.match_number = state.match_number + 1
end

---@param action ResultAction
---@return "match"|"formation"|"squad"|"title"
function session.route_for_result(action)
    if action == "rematch" then
        return "match"
    elseif action == "change_plan" then
        return "formation"
    elseif action == "change_lineup" then
        return "squad"
    end
    return "title"
end

return session
