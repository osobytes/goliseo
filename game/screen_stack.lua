-- A minimal screen stack. The topmost screen receives update/event/draw.
-- Screens are duck-typed: any of :update(dt), :event(evt), :draw() may be absent.

---@class Screen
---@field update fun(self: Screen, dt: number)?
---@field event fun(self: Screen, evt: InputEvent)?
---@field draw fun(self: Screen)?
---@field teardown fun(self: Screen)?
---@field apply_settings fun(self: Screen, settings: GameSettings)?

-- Not every variant below reaches a screen by every route, and the difference
-- decides where a new handler belongs.
--
-- `App` forwards only *normalized* events: `controller.normalize` turns a key
-- into an `ActionEvent` via the binding table and returns `nil` when the key is
-- unbound, so a raw `key` event never travels through `App` to the stack. The
-- `key` variant reaches a screen only when something constructs that screen
-- directly and calls `:event(...)` on it — the specs and the playtest harness.
-- That is why the dev-harness keys (F1, R, B) live behind a directly-mounted
-- `playtest` profile and do nothing in a launched game; see `docs/controls.md`
-- and `docs/showcase_release.md`.
--
-- So: a handler for a bound control belongs in the `action` branch, or it will
-- never fire for a player. A raw `key` handler is reachable from a spec and
-- will pass its test while being dead through the shipped app.
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
