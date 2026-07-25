local focus = require("game.ui.focus")

---@class TitleScreenState
---@field viewport { w: number, h: number }
---@field focus string

---@class TitleScreenModule
local title = {}

---@param viewport { w: number, h: number }
---@return TitleScreenState
function title.new_state(viewport)
    return { viewport = viewport, focus = "play" }
end

---@param state TitleScreenState
---@return Layout
function title.layout(state)
    local layout = {
        {
            id = "brand",
            kind = "eyebrow",
            text = "INTERGALACTIC 5v5",
            rect = { x = 0, y = 72, w = state.viewport.w, h = 22 },
            data = { align = "center" },
        },
        {
            id = "title",
            kind = "hero_title",
            text = "GALACTIC CUP",
            rect = { x = 0, y = 104, w = state.viewport.w, h = 58 },
            data = { align = "center" },
        },
        {
            id = "tagline",
            kind = "label",
            text = "PICK THE FIVE  •  SET THE SHAPE  •  PLAY THE PLAN",
            rect = { x = 0, y = 174, w = state.viewport.w, h = 24 },
            data = { align = "center", tone = "muted" },
        },
    }

    local labels = {
        { "play", "PLAY SHOWCASE" },
        { "combat_prototype", "COMBAT PROTOTYPE" },
        { "help", "HOW TO PLAY" },
        { "settings", "SETTINGS" },
        { "credits", "CREDITS" },
        { "quit", "QUIT" },
    }
    for i, item in ipairs(labels) do
        layout[#layout + 1] = {
            id = item[1],
            kind = "button",
            text = item[2],
            focused = state.focus == item[1],
            rect = { x = 350, y = 216 + (i - 1) * 48, w = 260, h = 40 },
        }
    end
    return layout
end

---@param state TitleScreenState
---@param event InputEvent
---@return TitleScreenState, table?
function title.update(state, event)
    local layout = title.layout(state)
    local next_focus = focus.navigate(layout, state.focus, event) or state.focus
    if event.kind == "action" and event.action == "back" then
        return { viewport = state.viewport, focus = next_focus }, { go = "quit" }
    end
    local id = focus.activated(layout, next_focus, event)
    local next_state = { viewport = state.viewport, focus = id or next_focus }
    return next_state, id and { go = id } or nil
end

return title
