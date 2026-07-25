local build_info = require("game.build_info")
local focus = require("game.ui.focus")

---@class CreditsScreenState
---@field viewport { w: number, h: number }
---@field focus string

---@class CreditsScreenModule
local credits = {}

---@param viewport { w: number, h: number }
---@return CreditsScreenState
function credits.new_state(viewport)
    return { viewport = viewport, focus = "back" }
end

---@return string[]
local function credit_lines()
    local lines = {
        "GALACTIC CUP",
        "Designed and developed by the Galactic Cup contributors",
        "",
        "Third-party runtime: LÖVE 11.5 and LuaJIT",
        "Third-party media: none bundled",
        "Code, documentation, and repository-owned assets:",
        "PolyForm Noncommercial License 1.0.0",
        "",
        "Version " .. build_info.version .. " • " .. build_info.channel,
    }
    if build_info.source_url then
        lines[#lines + 1] = "Source: " .. build_info.source_url
    end
    lines[#lines + 1] = ""
    lines[#lines + 1] = "No warranty. See LICENSE for complete terms."
    return lines
end

---@param state CreditsScreenState
---@return Layout
function credits.layout(state)
    return {
        {
            id = "title",
            kind = "title",
            text = "CREDITS & SOURCE",
            rect = { x = 64, y = 54, w = 832, h = 36 },
            data = { align = "center", focusable = false },
        },
        {
            id = "credits",
            kind = "card",
            text = table.concat(credit_lines(), "\n"),
            rect = { x = 180, y = 118, w = 600, h = 300 },
            data = { align = "center", focusable = false },
        },
        {
            id = "back",
            kind = "button",
            text = "BACK",
            focused = true,
            rect = { x = 380, y = 458, w = 200, h = 42 },
        },
    }
end

---@param state CreditsScreenState
---@param event InputEvent
---@return CreditsScreenState, table?
function credits.update(state, event)
    if event.kind == "action" and event.action == "back" then
        return state, { go = "back" }
    end
    local id = focus.activated(credits.layout(state), state.focus, event)
    return state, id == "back" and { go = "back" } or nil
end

return credits
