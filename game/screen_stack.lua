-- A minimal screen stack. The topmost screen receives update/event/draw.
-- Screens are duck-typed: any of :update(dt), :event(evt), :draw() may be absent.

---@class Screen
---@field update fun(self: Screen, dt: number)?
---@field event fun(self: Screen, evt: InputEvent)?
---@field draw fun(self: Screen)?
---@field teardown fun(self: Screen)?
---@field apply_settings fun(self: Screen, settings: GameSettings)?

---@alias InputEvent ActionEvent | { kind: "key", key: string, pressed: boolean? } | { kind: "click", x: number, y: number, button: number } | RawGamepadEvent

---@class ScreenStack
---@field screens Screen[]
local ScreenStack = {}
ScreenStack.__index = ScreenStack

---@return ScreenStack
function ScreenStack.new()
    return setmetatable({ screens = {} }, ScreenStack)
end

---@param screen Screen
function ScreenStack:push(screen)
    self.screens[#self.screens + 1] = screen
end

---@param screen Screen?
local function teardown(screen)
    if screen and screen.teardown then
        screen:teardown()
    end
end

---@param screen Screen
function ScreenStack:replace(screen)
    teardown(self.screens[#self.screens])
    self.screens[#self.screens] = screen
end

function ScreenStack:clear()
    for index = #self.screens, 1, -1 do
        teardown(self.screens[index])
    end
    self.screens = {}
end

---@return Screen?
function ScreenStack:pop()
    local screen = table.remove(self.screens)
    teardown(screen)
    return screen
end

---@return Screen?
function ScreenStack:current()
    return self.screens[#self.screens]
end

---@param dt number
function ScreenStack:update(dt)
    local s = self:current()
    if s and s.update then
        s:update(dt)
    end
end

---@param evt InputEvent
function ScreenStack:event(evt)
    local s = self:current()
    if s and s.event then
        s:event(evt)
    end
end

function ScreenStack:draw()
    local s = self:current()
    if s and s.draw then
        s:draw()
    end
end

return ScreenStack
