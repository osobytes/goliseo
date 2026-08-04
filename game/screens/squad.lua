local focus = require("game.ui.focus")
local identity = require("render.identity")
local player_pool = require("data.players")
local teams = require("data.teams")

---@class SquadScreenContext
---@field starter_ids string[]

---@class SquadScreenState
---@field viewport { w: number, h: number }
---@field roster PlayerData[]
---@field selected_ids string[]
---@field focus string
---@field message string

---@class SquadScreenModule
local squad = {}

---@return table<string, PlayerData>
local function players_by_id()
    local result = {}
    for _, player in ipairs(player_pool) do
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

---@param ids string[]
---@param id string
---@return boolean
local function contains(ids, id)
    for _, value in ipairs(ids) do
        if value == id then
            return true
        end
    end
    return false
end

---@param ids string[]
---@param id string
---@return string[]
local function without(ids, id)
    local result = {}
    for _, value in ipairs(ids) do
        if value ~= id then
            result[#result + 1] = value
        end
    end
    return result
end

---@param viewport { w: number, h: number }
---@param context SquadScreenContext?
---@return SquadScreenState
function squad.new_state(viewport, context)
    local by_id = players_by_id()
    local roster = {}
    for _, id in ipairs(teams.nebula.squad or teams.nebula.roster) do
        roster[#roster + 1] = assert(by_id[id], "unknown squad player: " .. id)
    end
    local selected = context and context.starter_ids or teams.nebula.roster
    return {
        viewport = viewport,
        roster = roster,
        selected_ids = copy_ids(selected),
        focus = "player_" .. roster[1].id,
        message = "Select four outfielders. Ozzo is your locked keeper.",
    }
end

---@param state SquadScreenState
---@return Layout
function squad.layout(state)
    local layout = {
        {
            id = "title",
            kind = "title",
            text = "PICK THE FIVE",
            rect = { x = 0, y = 22, w = state.viewport.w, h = 34 },
            data = { align = "center" },
        },
        {
            id = "counter",
            kind = "eyebrow",
            text = ("%d / 5 STARTERS"):format(#state.selected_ids),
            rect = { x = 0, y = 60, w = state.viewport.w, h = 22 },
            data = { align = "center" },
        },
    }

    for i, player in ipairs(state.roster) do
        local presentation =
            assert(identity.for_player(player.id), "missing showcase identity: " .. player.id)
        local col = (i - 1) % 2
        local row = math.floor((i - 1) / 2)
        local id = "player_" .. player.id
        layout[#layout + 1] = {
            id = id,
            kind = "card",
            text = ("%s  •  %s %s\nPAC %d  STR %d  TEC %d  STA %d  MEN %d\n%s"):format(
                player.name,
                presentation.species_name,
                player.position:upper(),
                player.stats.pace,
                player.stats.strength,
                player.stats.technique,
                player.stats.stamina,
                player.stats.mental,
                presentation.tagline
            ),
            selected = contains(state.selected_ids, player.id),
            focused = state.focus == id,
            rect = { x = 62 + col * 424, y = 96 + row * 86, w = 390, h = 74 },
            data = {
                accent = presentation.palette,
                species_shape = presentation.shape,
                locked = player.position == "keeper",
            },
        }
    end

    layout[#layout + 1] = {
        id = "message",
        kind = "label",
        text = state.message,
        rect = { x = 120, y = 448, w = 720, h = 22 },
        data = { align = "center", tone = "muted" },
    }
    layout[#layout + 1] = {
        id = "back",
        kind = "button",
        text = "BACK",
        focused = state.focus == "back",
        rect = { x = 254, y = 482, w = 190, h = 40 },
    }
    layout[#layout + 1] = {
        id = "next",
        kind = "button",
        text = "SET THE SHAPE",
        focused = state.focus == "next",
        rect = { x = 516, y = 482, w = 190, h = 40 },
        data = { disabled = #state.selected_ids ~= 5 },
    }
    return layout
end

---@param state SquadScreenState
---@param event InputEvent
---@return SquadScreenState, table?
function squad.update(state, event)
    local layout = squad.layout(state)
    local next = {
        viewport = state.viewport,
        roster = state.roster,
        selected_ids = copy_ids(state.selected_ids),
        focus = focus.navigate(layout, state.focus, event) or state.focus,
        message = state.message,
    }
    if event.kind == "action" and event.action == "back" then
        return next, { go = "title" }
    end

    local id = focus.activated(layout, next.focus, event)
    if id then
        next.focus = id
    end
    if id == "back" then
        return next, { go = "title" }
    elseif id == "next" and #next.selected_ids == 5 then
        return next, { go = "formation", starter_ids = copy_ids(next.selected_ids) }
    end

    local player_id = id and id:match("^player_(.+)$")
    if player_id then
        local player = players_by_id()[player_id]
        if player.position == "keeper" then
            next.message = "Your team sheet must keep exactly one keeper."
        elseif contains(next.selected_ids, player_id) then
            next.selected_ids = without(next.selected_ids, player_id)
            next.message = "Choose " .. tostring(5 - #next.selected_ids) .. " more starter."
        elseif #next.selected_ids < 5 then
            next.selected_ids[#next.selected_ids + 1] = player_id
            next.message = #next.selected_ids == 5 and "Team sheet ready."
                or "Choose another starter."
        else
            next.message = "Deselect an outfielder before adding " .. player.name .. "."
        end
    end
    return next
end

return squad
