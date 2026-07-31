-- Edge layer: one key or button press becomes one ActionEvent. Menus and the
-- global toggles consume these. The match screen additionally POLLS the same
-- controls for held state -- see `game.screens.match`.
--
-- Both maps are derived from `game.input.bindings`. Never add a literal key
-- here; add the control there.

local bindings = require("game.input.bindings")

---@alias ActionName
---|"up"
---|"down"
---|"left"
---|"right"
---|"confirm"
---|"back"
---|"pause"
---|"pass_switch"
---|"sprint"
---|"lob"
---|"juke"
---|"equipment"
---|"toggle_fullscreen"
---|"toggle_mute"

---@class ActionEvent
---@field kind "action"
---@field action ActionName
---@field pressed boolean?
---@field source "keyboard"|"gamepad"?

---@class ActionsModule
local actions = {}

---@type table<string, ActionName>
local KEY_MAP = bindings.key_map()

---@type table<string, ActionName>
local GAMEPAD_MAP = bindings.gamepad_map()

---@param name ActionName
---@param pressed boolean?
---@param source "keyboard"|"gamepad"?
---@return ActionEvent
function actions.event(name, pressed, source)
    return {
        kind = "action",
        action = name,
        pressed = pressed == nil or pressed,
        source = source,
    }
end

---@param key string
---@param pressed boolean?
---@return ActionEvent?
function actions.from_key(key, pressed)
    local action = KEY_MAP[key]
    return action and actions.event(action, pressed, "keyboard") or nil
end

-- Equipment shares gamepad B with Back, so which one a press means depends on
-- whether a match is live. The binding table cannot express that; issue #311
-- gives equipment its own button and retires the special case.
---@param button string
---@param pressed boolean?
---@param in_match boolean?
---@return ActionEvent?
function actions.from_gamepad(button, pressed, in_match)
    local action = GAMEPAD_MAP[button]
    local equipment = bindings.control("equipment").buttons[1]
    if button == equipment and in_match then
        action = "equipment"
    end
    return action and actions.event(action, pressed, "gamepad") or nil
end

return actions
