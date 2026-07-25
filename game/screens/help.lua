local focus = require("game.ui.focus")

---@class HelpScreenState
---@field viewport { w: number, h: number }
---@field focus string

---@class HelpScreenModule
local help = {}

---@param viewport { w: number, h: number }
---@return HelpScreenState
function help.new_state(viewport)
    return { viewport = viewport, focus = "back" }
end

---@param state HelpScreenState
---@return Layout
function help.layout(state)
    return {
        {
            id = "title",
            kind = "title",
            text = "HOW TO PLAY",
            rect = { x = 64, y = 44, w = 832, h = 36 },
            data = { align = "center" },
        },
        {
            id = "keyboard",
            kind = "card",
            text = "MATCH CONTROLS · KEYBOARD\nMove / Navigate   WASD or Arrows\nACTION              Space\nPLAY                   K\nSPRINT                Shift\nMODIFIER             L\nJUKE                    C\nEQUIPMENT*        J\nPAUSE                  P or Esc",
            rect = { x = 92, y = 112, w = 360, h = 292 },
            data = { focusable = false },
        },
        {
            id = "gamepad",
            kind = "card",
            text = "MATCH CONTROLS · GAMEPAD\nMove / Navigate   Left Stick or D-Pad\nACTION              A\nPLAY                   X\nSPRINT                LB\nMODIFIER             Y\nJUKE                    L3\nEQUIPMENT*        B\nPAUSE                  Start",
            rect = { x = 508, y = 112, w = 360, h = 292 },
            data = { focusable = false },
        },
        {
            id = "hint",
            kind = "label",
            text = "ACTION — shoot / tackle     PLAY — pass / switch     *COMBAT PROTOTYPE ONLY\nHOLD ACTION OR PLAY TO CHARGE · RELEASE TO COMMIT     EQUIPMENT: HOLD / TAP"
                .. "\nKEEPER: PLAY THROWS · ACTION PUNTS",
            rect = { x = 90, y = 414, w = 780, h = 54 },
            data = { align = "center", tone = "muted" },
        },
        {
            id = "back",
            kind = "button",
            text = "BACK",
            focused = state.focus == "back",
            rect = { x = 380, y = 486, w = 200, h = 42 },
        },
    }
end

---@param state HelpScreenState
---@param event InputEvent
---@return HelpScreenState, table?
function help.update(state, event)
    if event.kind == "action" and event.action == "back" then
        return state, { go = "back" }
    end
    local id = focus.activated(help.layout(state), state.focus, event)
    return state, id == "back" and { go = "back" } or nil
end

return help
