local actions = require("game.input.actions")
local viewport = require("game.ui.viewport")

---@class RawGamepadEvent
---@field kind "gamepad"
---@field button string
---@field pressed boolean?

---@class InputControllerModule
local controller = {}

---@param event InputEvent|RawGamepadEvent
---@param transform ViewportTransform
---@return InputEvent?
function controller.normalize(event, transform)
    local normalized = nil
    if event.kind == "action" then
        normalized = event
    elseif event.kind == "key" then
        normalized = actions.from_key(event.key, event.pressed)
    elseif event.kind == "gamepad" then
        normalized = actions.from_gamepad(event.button, event.pressed)
    elseif event.kind == "click" then
        local x, y = viewport.to_virtual(transform, event.x, event.y)
        if x and y then
            return { kind = "click", x = x, y = y, button = event.button or 1 }
        end
    end
    if
        normalized
        and normalized.kind == "action"
        and normalized.pressed == false
        and normalized.action ~= "equipment"
    then
        return nil
    end
    return normalized
end

return controller
