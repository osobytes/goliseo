local actions = require("game.input.actions")
local bindings = require("game.input.bindings")
local help = require("game.screens.help")
local t = require("spec.support.runner")

---@param id ControlId
---@return ControlBinding
local function control(id)
    return bindings.control(id)
end

t.describe("input bindings", function()
    t.it("derives both action maps from the one table", function()
        t.eq(actions.from_key(control("juke").keys[1]).action, "juke")
        t.eq(actions.from_gamepad(control("juke").buttons[1]).action, "juke")
        t.eq(actions.from_key(control("sprint").keys[1]).action, "sprint")
        t.eq(actions.from_key(control("play").keys[1]).action, "pass_switch")
    end)

    t.it("renders the help card from the bindings rather than a literal", function()
        local card = nil
        for _, element in ipairs(help.layout(help.new_state({ w = 960, h = 540 }))) do
            if element.id == "keyboard" then
                card = element.text
            end
        end
        card = assert(card, "the help screen lost its keyboard card")
        t.is_true(card:find("MODIFIER", 1, true) ~= nil)
        t.is_true(
            card:find(bindings.key_label("modifier"), 1, true) ~= nil,
            "the card does not show the bound modifier key"
        )
        t.is_true(
            card:find(bindings.key_label("juke"), 1, true) ~= nil,
            "the card does not show the bound juke key"
        )
    end)

    t.it("labels every reference row for both devices", function()
        local rows = bindings.reference()
        t.is_true(#rows > 0)
        for _, row in ipairs(rows) do
            t.is_true(#row.label > 0)
            t.is_true(#row.keyboard > 0, row.label .. " has no keyboard binding")
        end
        for _, row in ipairs(bindings.reference("match")) do
            t.is_true(#row.gamepad > 0, row.label .. " has no gamepad binding")
        end
    end)
end)
