-- Smoke tests for the impure renderers. We stub love.graphics (disabled in test
-- mode) so the draw code actually executes: this catches nil-field/arithmetic
-- bugs in projection, sorting and layout — everything except the pixels.

local t = require("spec.support.runner")
local pitch = require("game.render.pitch")
local render_payload = require("spec.support.render_payload")
local render_frame = require("render.frame")
local match_hud = require("game.match_hud")
local match_hud_render = require("game.render.match_hud")
local ui_draw = require("game.ui.draw")
local match_sim = require("sim.match")
local teams = require("data.teams")
local formations = require("data.formations")

local function stub_graphics()
    local g = {}
    local noop = function() end
    for _, name in ipairs({
        "setColor",
        "setLineWidth",
        "setBlendMode",
        "rectangle",
        "polygon",
        "line",
        "circle",
        "ellipse",
        "arc",
        "push",
        "pop",
        "translate",
        "rotate",
        "print",
        "printf",
    }) do
        g[name] = noop
    end
    g.getDimensions = function()
        return 1280, 720
    end
    g.getWidth = function()
        return 1280
    end
    g.getHeight = function()
        return 720
    end
    return g
end

---@param fn fun()
---@return boolean ok, string? err
local function with_stub(fn)
    local saved = love.graphics
    love.graphics = stub_graphics()
    local ok, err = pcall(fn)
    love.graphics = saved
    return ok, err
end

t.describe("renderer smoke", function()
    t.it("pitch.draw runs over a real match state", function()
        local ok, err = with_stub(function()
            local s = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            s.players[s.controlled].charge = 0.6 -- exercise the charge-meter path
            pitch.draw(render_payload.frame(s), { w = 1280, h = 720 }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
            })
        end)
        t.is_true(ok, "pitch.draw error: " .. tostring(err))
    end)

    t.it("rigged players fall back cleanly when 3D is unavailable", function()
        -- The headless runner stubs love.graphics without newShader/newMesh, so
        -- this is exactly the "runtime cannot host the 3D pass" case: the rigged
        -- renderer must report unavailable and the procedural one must still
        -- draw, rather than anything throwing.
        pitch.rigged_players = true
        local ok, err = with_stub(function()
            local s = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            pitch.draw(render_payload.frame(s), { w = 1280, h = 720 }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
            })
        end)
        pitch.rigged_players = false
        t.is_true(ok, "pitch.draw with rigged players error: " .. tostring(err))
    end)

    t.it("pitch.draw renders the pass-target marker without error", function()
        local ok, err = with_stub(function()
            local s = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            -- Point at player index 2 (a home outfielder) as the preview target.
            s.players[s.controlled].pass_target = 2
            pitch.draw(render_payload.frame(s), { w = 1280, h = 720 }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
            })
        end)
        t.is_true(ok, "pitch.draw with pass_target error: " .. tostring(err))
    end)

    t.it("ui_draw.layout runs over a mixed layout", function()
        local ok, err = with_stub(function()
            ---@type Layout
            local layout = {
                {
                    id = "t",
                    kind = "title",
                    text = "Title",
                    rect = { x = 0, y = 0, w = 100, h = 20 },
                    data = { align = "center" },
                },
                {
                    id = "b",
                    kind = "button",
                    text = "Go",
                    selected = true,
                    rect = { x = 0, y = 30, w = 80, h = 30 },
                },
                {
                    id = "c",
                    kind = "card",
                    text = "Card",
                    rect = { x = 0, y = 70, w = 200, h = 40 },
                },
                {
                    id = "p",
                    kind = "formation_preview",
                    selected = true,
                    rect = { x = 220, y = 70, w = 124, h = 52 },
                    data = {
                        keeper = formations["2-1-1"].keeper,
                        outfield = formations["2-1-1"].outfield,
                    },
                },
            }
            ui_draw.layout(layout)
        end)
        t.is_true(ok, "ui_draw.layout error: " .. tostring(err))
    end)

    t.it("draws the product match HUD over a real match state", function()
        local ok, err = with_stub(function()
            local s = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            local model = match_hud.model(render_frame.hud(s), {
                home_name = teams.nebula.name,
                away_name = teams.orion.name,
                arena_name = "Helios Crown",
                arena_location = "Kairon-9 Orbit",
                tactic_name = "Press High",
                formation_name = "1-2-1",
                phase = "full_time",
            })
            match_hud_render.draw(model, { w = 1280, h = 720 })
        end)
        t.is_true(ok, "match HUD draw error: " .. tostring(err))
    end)
end)
