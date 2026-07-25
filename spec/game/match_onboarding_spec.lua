local onboarding = require("game.match_onboarding")
local t = require("spec.support.runner")

---@param values table?
---@return OnboardingContext
local function context(values)
    values = values or {}
    return {
        carrying = values.carrying == true,
        defending = values.defending == true,
        keeper_holding = values.keeper_holding == true,
        moved = values.moved == true,
        shot = values.shot == true,
        passed = values.passed == true,
        defended = values.defended == true,
        equipment_available = values.equipment_available == true,
        equipment_used = values.equipment_used == true,
    }
end

t.describe("first-match onboarding", function()
    t.it("teaches one contextual action at a time and never repeats a lesson", function()
        local state = onboarding.new(true)
        t.eq(assert(onboarding.prompt(state)).id, "move")

        state = onboarding.update(state, context({ moved = true, carrying = true }), 0.1)
        t.eq(assert(onboarding.prompt(state)).id, "possession")
        state = onboarding.update(state, context({ passed = true, carrying = true }), 0.1)
        t.is_true(onboarding.prompt(state) == nil)

        state = onboarding.update(state, context({ defending = true }), 0.1)
        t.eq(assert(onboarding.prompt(state)).id, "defending")
        state = onboarding.update(state, context({ defending = true, defended = true }), 0.1)
        t.is_true(onboarding.prompt(state) == nil)
        state = onboarding.update(state, context({ defending = true }), 0.1)
        t.is_true(onboarding.prompt(state) == nil, "the defending lesson stays retired")
    end)

    t.it("prioritizes keeper distribution and retires prompts after six seconds", function()
        local state = onboarding.new(true)
        state = onboarding.update(state, context(), 6)
        t.is_true(onboarding.prompt(state) == nil)
        state = onboarding.update(state, context({ keeper_holding = true, carrying = true }), 0)
        t.eq(assert(onboarding.prompt(state)).id, "keeper")
        state = onboarding.update(
            state,
            context({ keeper_holding = true, carrying = true, shot = true }),
            0.1
        )
        t.eq(assert(onboarding.prompt(state)).id, "possession")
    end)

    t.it("is entirely suppressed after the first fixture", function()
        local state = onboarding.new(false)
        state =
            onboarding.update(state, context({ moved = true, carrying = true, passed = true }), 10)
        t.is_true(onboarding.prompt(state) == nil)
    end)

    t.it("teaches equipment only in the explicit combat prototype", function()
        local combat = onboarding.new(false, true)
        t.eq(assert(onboarding.prompt(combat)).id, "equipment")
        combat = onboarding.update(
            combat,
            context({ equipment_available = true, equipment_used = true }),
            0.1
        )
        t.is_true(onboarding.prompt(combat) == nil)

        local showcase = onboarding.new(false, false)
        showcase = onboarding.update(showcase, context({ equipment_available = true }), 0.1)
        t.is_true(onboarding.prompt(showcase) == nil)
    end)
end)
