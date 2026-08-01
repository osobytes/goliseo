local focus = require("game.ui.focus")
local bindings = require("game.input.bindings")

local CARD_LABEL_WIDTH = 18

-- The card is rendered from `bindings.reference`, never hand-written, so a
-- rebinding cannot leave the how-to-play screen lying to the player.
---@param heading string
---@param device "keyboard"|"gamepad"
---@return string
local function control_card(heading, device)
    local lines = { heading }
    for _, row in ipairs(bindings.reference("match")) do
        local label = row.label:upper() .. (row.footnote and "*" or "")
        local pad = math.max(1, CARD_LABEL_WIDTH - #label)
        lines[#lines + 1] = label .. (" "):rep(pad) .. row[device]
    end
    return table.concat(lines, "\n")
end

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
            text = control_card("MATCH CONTROLS · KEYBOARD", "keyboard"),
            rect = { x = 92, y = 112, w = 360, h = 292 },
            data = { focusable = false },
        },
        {
            id = "gamepad",
            kind = "card",
            text = control_card("MATCH CONTROLS · GAMEPAD", "gamepad"),
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
