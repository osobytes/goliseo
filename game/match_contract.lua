local arenas = require("data.arenas")
local formations = require("data.formations")
local players = require("data.players")
local tactics = require("data.tactics")
local teams = require("data.teams")

---@alias MatchWinner "home"|"away"|"draw"

---@class ProductMatchRequest
---@field home_team_id string
---@field away_team_id string
---@field home_starter_ids string[]
---@field formation_id string
---@field tactic_id string
---@field arena_id string
---@field show_onboarding boolean
---@field combat_enabled boolean
---@field seed integer?

---@class TeamResultStats
---@field shots integer?
---@field possession number?
---@field saves integer?
---@field pass_completion number?

---@class ProductMatchResult
---@field home_team_id string
---@field away_team_id string
---@field home_name string
---@field away_name string
---@field home_score integer
---@field away_score integer
---@field winner MatchWinner
---@field mvp_player_id string?
---@field mvp_summary string?
---@field home_stats TeamResultStats
---@field away_stats TeamResultStats
---@field seed integer?

---@class ProductMatchRequestOptions
---@field home_team_id string
---@field away_team_id string
---@field home_starter_ids string[]
---@field formation_id string
---@field tactic_id string
---@field arena_id string?
---@field show_onboarding boolean?
---@field combat_enabled boolean?
---@field seed integer?

---@class ProductMatchResultOptions
---@field home_team_id string
---@field away_team_id string
---@field home_score integer
---@field away_score integer
---@field mvp_player_id string?
---@field mvp_summary string?
---@field home_stats TeamResultStats?
---@field away_stats TeamResultStats?
---@field seed integer?

---@class MatchContractModule
local contract = {}

---@return table<string, PlayerData>
local function players_by_id()
    local result = {}
    for _, player in ipairs(players) do
        result[player.id] = player
    end
    return result
end

---@param ids string[]
---@return string[]
local function copy_ids(ids)
    local result = {}
    for i, id in ipairs(ids) do
        result[i] = id
    end
    return result
end

---@param team TeamData
---@return table<string, boolean>
local function eligible_ids(team)
    local result = {}
    local squad = team.squad or team.roster
    for _, id in ipairs(squad) do
        result[id] = true
    end
    return result
end

---@param ids string[]
---@param team TeamData
---@return boolean?, string?
function contract.validate_starters(ids, team)
    if #ids ~= 5 then
        return nil, "a team sheet needs exactly five starters"
    end

    local pool = players_by_id()
    local eligible = eligible_ids(team)
    local seen = {}
    local keepers = 0
    for _, id in ipairs(ids) do
        if seen[id] then
            return nil, "starter ids must be unique"
        end
        seen[id] = true
        if not eligible[id] then
            return nil, "player is not eligible for " .. team.name .. ": " .. id
        end
        local player = pool[id]
        if not player then
            return nil, "unknown player id: " .. id
        end
        if player.position == "keeper" then
            keepers = keepers + 1
        end
    end
    if keepers ~= 1 then
        return nil, "a team sheet needs exactly one keeper"
    end
    return true
end

---@param opts ProductMatchRequestOptions
---@return ProductMatchRequest?, string?
function contract.new_request(opts)
    local home = teams[opts.home_team_id]
    if not home then
        return nil, "unknown home team: " .. tostring(opts.home_team_id)
    end
    if not teams[opts.away_team_id] then
        return nil, "unknown away team: " .. tostring(opts.away_team_id)
    end
    if not formations[opts.formation_id] then
        return nil, "unknown formation: " .. tostring(opts.formation_id)
    end
    if not tactics[opts.tactic_id] then
        return nil, "unknown tactic: " .. tostring(opts.tactic_id)
    end
    local arena_id = opts.arena_id or "helios_crown"
    if not arenas[arena_id] then
        return nil, "unknown arena: " .. tostring(arena_id)
    end
    local ok, err = contract.validate_starters(opts.home_starter_ids, home)
    if not ok then
        return nil, err
    end

    return {
        home_team_id = opts.home_team_id,
        away_team_id = opts.away_team_id,
        home_starter_ids = copy_ids(opts.home_starter_ids),
        formation_id = opts.formation_id,
        tactic_id = opts.tactic_id,
        arena_id = arena_id,
        show_onboarding = opts.show_onboarding == true,
        combat_enabled = opts.combat_enabled == true,
        seed = opts.seed,
    }
end

---@param value number?
---@return boolean
local function valid_ratio(value)
    return value == nil or (value >= 0 and value <= 1)
end

---@param value integer?
---@return boolean
local function valid_count(value)
    return value == nil or (value >= 0 and value == math.floor(value))
end

---@param stats TeamResultStats
---@return boolean?, string?
local function validate_stats(stats)
    if not valid_count(stats.shots) then
        return nil, "shots must be a non-negative integer"
    end
    if not valid_count(stats.saves) then
        return nil, "saves must be a non-negative integer"
    end
    if not valid_ratio(stats.possession) then
        return nil, "possession must be between zero and one"
    end
    if not valid_ratio(stats.pass_completion) then
        return nil, "pass completion must be between zero and one"
    end
    return true
end

---@param opts ProductMatchResultOptions
---@return ProductMatchResult?, string?
function contract.new_result(opts)
    local home = teams[opts.home_team_id]
    local away = teams[opts.away_team_id]
    if not home or not away then
        return nil, "match result references an unknown team"
    end
    if
        not valid_count(opts.home_score)
        or opts.home_score == nil
        or not valid_count(opts.away_score)
        or opts.away_score == nil
    then
        return nil, "scores must be non-negative integers"
    end

    local home_stats = opts.home_stats or {}
    local away_stats = opts.away_stats or {}
    local ok, err = validate_stats(home_stats)
    if not ok then
        return nil, err
    end
    ok, err = validate_stats(away_stats)
    if not ok then
        return nil, err
    end

    local winner = "draw"
    if opts.home_score > opts.away_score then
        winner = "home"
    elseif opts.away_score > opts.home_score then
        winner = "away"
    end

    return {
        home_team_id = home.id,
        away_team_id = away.id,
        home_name = home.name,
        away_name = away.name,
        home_score = opts.home_score,
        away_score = opts.away_score,
        winner = winner,
        mvp_player_id = opts.mvp_player_id,
        mvp_summary = opts.mvp_summary,
        home_stats = home_stats,
        away_stats = away_stats,
        seed = opts.seed,
    }
end

return contract
