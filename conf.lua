-- LÖVE configuration. Runs before any module loads, so this is where we detect
-- `--test` and disable the window/graphics modules for headless test runs.

---@param a string
---@return boolean
local function has_flag(a)
    for _, v in ipairs(arg or {}) do
        if v == a then
            return true
        end
    end
    return false
end

---@param t table  -- love config table
function love.conf(t)
    t.identity = "galactic_cup"
    t.version = "11.5"

    local headless_flags = {
        "--test",
        "--sim",
        "--sweep",
        "--search",
        "--eval",
        "--tripwire",
        "--snapshot-measure",
        "--rollback-lab",
        "--determinism-refresh",
        "--fault-harness",
    }
    if not has_flag("--browser-runtime") then
        headless_flags[#headless_flags + 1] = "--determinism"
        headless_flags[#headless_flags + 1] = "--rollback-validation"
    end
    local headless = false
    for _, f in ipairs(headless_flags) do
        headless = headless or has_flag(f)
    end
    if headless then
        -- Headless: no GL context, no display required.
        t.modules.window = false
        t.modules.graphics = false
        t.modules.audio = false
        t.modules.sound = false
        t.modules.joystick = false
        t.modules.physics = false
        t.modules.touch = false
    else
        t.window.title = "Galactic Cup"
        t.window.width = 960
        t.window.height = 540
        t.window.resizable = false
        t.window.vsync = 1
    end
end
