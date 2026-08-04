local focus = require("game.ui.focus")
local identity = require("render.identity")
local formations = require("data.formations")
local player_pool = require("data.players")

---@class FormationScreenContext
---@field selected string
---@field starter_ids string[]

---@class FormationScreenState
---@field viewport { w: number, h: number }
---@field selected string
---@field starter_ids string[]
---@field focus string

---@class FormationAction
---@field go "tactic"
---@field formation_id string
---@field formation string

---@class FormationScreenModule
local formation = {}

---@param ids string[]
---@return string[]
local function copy_ids(ids)
    local result = {}
    for i, id in ipairs(ids) do
        result[i] = id
    end
    return result
end

---@return table<string, PlayerData>
local function players_by_id()
    local result = {}
    for _, player in ipairs(player_pool) do
        result[player.id] = player
    end
    return result
end

---@return string[]
local function sorted_formation_ids()
    local ids = {}
    for id in pairs(formations) do
        ids[#ids + 1] = id
    end
    table.sort(ids)
    return ids
end

---@param viewport { w: number, h: number }
---@param context FormationScreenContext?
---@return FormationScreenState
function formation.new_state(viewport, context)
    local selected = context and context.selected or "2-1-1"
    assert(formations[selected], "missing formation: " .. selected)
    return {
        viewport = viewport,
        selected = selected,
        starter_ids = copy_ids(context and context.starter_ids or {}),
        focus = "formation_" .. selected,
    }
end

---@param state FormationScreenState
---@return Layout
function formation.layout(state)
    local layout = {
        {
            id = "title",
            kind = "title",
            text = "SET THE SHAPE",
            rect = { x = 0, y = 34, w = state.viewport.w, h = 34 },
            data = { align = "center" },
        },
    }
    local by_id = players_by_id()
    local markers = {}
    local lineup_names = {}
    for _, id in ipairs(state.starter_ids) do
        local player = by_id[id]
        local presentation = identity.for_player(id)
        if player then
            lineup_names[#lineup_names + 1] = player.name:upper()
        end
        markers[#markers + 1] = {
            color = presentation and presentation.palette or { 0.8, 0.9, 1 },
            shape = presentation and presentation.shape or "round",
            name = player and player.name or id,
        }
    end
    layout[#layout + 1] = {
        id = "lineup",
        kind = "label",
        text = table.concat(lineup_names, "  •  "),
        rect = { x = 80, y = 72, w = 800, h = 18 },
        data = { align = "center", tone = "muted" },
    }

    for i, id in ipairs(sorted_formation_ids()) do
        local data = formations[id]
        local y = 96 + (i - 1) * 112
        layout[#layout + 1] = {
            id = "formation_" .. id,
            kind = "button",
            text = id
                .. "  "
                .. data.name
                .. "\n+ "
                .. (data.strength or "A flexible small-sided shape.")
                .. "\n− "
                .. (data.risk or "Requires disciplined positioning."),
            selected = state.selected == id,
            focused = state.focus == "formation_" .. id,
            rect = { x = 150, y = y, w = 410, h = 94 },
            data = { align = "left" },
        }
        layout[#layout + 1] = {
            id = "preview_" .. id,
            kind = "formation_preview",
            selected = state.selected == id,
            rect = { x = 580, y = y, w = 230, h = 94 },
            data = {
                keeper = data.keeper,
                outfield = data.outfield,
                markers = markers,
            },
        }
    end
    layout[#layout + 1] = {
        id = "back",
        kind = "button",
        text = "BACK",
        focused = state.focus == "back",
        rect = { x = 254, y = 472, w = 190, h = 42 },
    }
    layout[#layout + 1] = {
        id = "next",
        kind = "button",
        text = "CHOOSE TACTIC",
        focused = state.focus == "next",
        rect = { x = 516, y = 472, w = 190, h = 42 },
    }
    return layout
end

---@param state FormationScreenState
---@param event InputEvent
---@return FormationScreenState, FormationAction|table?
function formation.update(state, event)
    local layout = formation.layout(state)
    local next = {
        viewport = state.viewport,
        selected = state.selected,
        starter_ids = copy_ids(state.starter_ids),
        focus = focus.navigate(layout, state.focus, event) or state.focus,
    }
    if event.kind == "action" and event.action == "back" then
        return next,
            {
                go = "squad",
                formation_id = next.selected,
                formation = next.selected,
            }
    end
    local id = focus.activated(layout, next.focus, event)
    if id then
        next.focus = id
        local formation_id = id:match("^formation_(.+)$") or id:match("^preview_(.+)$")
        if formation_id and formations[formation_id] then
            next.selected = formation_id
            next.focus = "formation_" .. formation_id
        elseif id == "back" then
            return next,
                {
                    go = "squad",
                    formation_id = next.selected,
                    formation = next.selected,
                }
        elseif id == "next" then
            return next,
                {
                    go = "tactic",
                    formation_id = next.selected,
                    formation = next.selected,
                }
        end
    end
    return next
end

return formation
