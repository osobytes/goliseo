local Credits = require("game.screens.credits")
local Formation = require("game.screens.formation")
local Help = require("game.screens.help")
local Pause = require("game.screens.pause")
local Result = require("game.screens.result")
local Settings = require("game.screens.settings")
local Squad = require("game.screens.squad")
local Tactic = require("game.screens.tactic")
local Title = require("game.screens.title")
local fake_result = require("game.fake_result")
local session = require("game.session")
local settings_model = require("game.settings")
local t = require("spec.support.runner")
local ui_draw = require("game.ui.draw")
local ui = require("game.ui.hit")

---@param width integer
---@param height integer
---@param fn fun()
---@return boolean, string?
local function with_graphics(width, height, fn)
    local saved = love.graphics
    local graphics = {}
    local noop = function() end
    for _, name in ipairs({
        "setColor",
        "setLineWidth",
        "rectangle",
        "polygon",
        "line",
        "circle",
        "ellipse",
        "push",
        "pop",
        "translate",
        "scale",
        "printf",
    }) do
        graphics[name] = noop
    end
    graphics.getDimensions = function()
        return width, height
    end
    love.graphics = graphics
    local ok, err = pcall(fn)
    love.graphics = saved
    return ok, err
end

---@param layout Layout
local function assert_within_virtual_canvas(layout)
    for _, widget in ipairs(layout) do
        t.is_true(widget.rect.x >= 0, widget.id .. " starts left of viewport")
        t.is_true(widget.rect.y >= 0, widget.id .. " starts above viewport")
        t.is_true(widget.rect.x + widget.rect.w <= 960, widget.id .. " exceeds viewport width")
        t.is_true(widget.rect.y + widget.rect.h <= 540, widget.id .. " exceeds viewport height")
    end
end

t.describe("product UI presentation", function()
    t.it("keeps keeper and combat-only equipment guidance together", function()
        local layout = Help.layout(Help.new_state({ w = 960, h = 540 }))
        local text = assert(assert(ui.find(layout, "hint")).text)
        t.is_true(text:find("KEEPER: PLAY THROWS · ACTION PUNTS", 1, true) ~= nil)
        t.is_true(text:find("*COMBAT PROTOTYPE ONLY", 1, true) ~= nil)
        t.is_true(text:find("EQUIPMENT: HOLD / TAP", 1, true) ~= nil)
    end)

    t.it("keeps every surrounding screen inside the virtual canvas", function()
        local viewport = { w = 960, h = 540 }
        local state = session.new()
        local request = assert(session.build_request(state, 1))
        local result = fake_result.for_request(request)
        local screens = {
            { Title, Title.new_state(viewport) },
            { Help, Help.new_state(viewport) },
            { Credits, Credits.new_state(viewport) },
            { Pause, Pause.new_state(viewport) },
            { Settings, Settings.new_state(viewport, { settings = settings_model.defaults() }) },
            { Squad, Squad.new_state(viewport) },
            {
                Formation,
                Formation.new_state(viewport, {
                    selected = state.formation_id,
                    starter_ids = state.starter_ids,
                }),
            },
            {
                Tactic,
                Tactic.new_state(viewport, {
                    selected = state.tactic_id,
                    formation_id = state.formation_id,
                }),
            },
            { Result, Result.new_state(viewport, { result = result }) },
        }
        for _, entry in ipairs(screens) do
            assert_within_virtual_canvas(entry[1].layout(entry[2]))
        end
    end)

    t.it("renders the product screens at all release resolutions", function()
        local viewport = { w = 960, h = 540 }
        local layout = Squad.layout(Squad.new_state(viewport))
        for _, size in ipairs({ { 960, 540 }, { 1280, 720 }, { 1920, 1080 }, { 1280, 800 } }) do
            local ok, err = with_graphics(size[1], size[2], function()
                ui_draw.layout(layout, viewport)
            end)
            t.is_true(ok, tostring(err))
        end
    end)
end)
