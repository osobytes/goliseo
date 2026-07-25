local t = require("spec.support.runner")
local stats = require("sim.stats")

---@param pace integer
---@param strength integer
---@param mental integer?
---@param technique integer?
---@param stamina integer?
---@return StatBlock
local function block(pace, strength, mental, technique, stamina)
    return {
        pace = pace,
        strength = strength,
        technique = technique or 5,
        stamina = stamina or 5,
        mental = mental or 5,
    }
end

t.describe("stats", function()
    t.it("move_speed increases with pace without changing the established mapping", function()
        local slow = stats.move_speed(block(2, 5))
        local fast = stats.move_speed(block(8, 5))
        t.is_true(fast > slow, "faster player should have higher move speed")
        t.eq(fast, 220, "pace 8 keeps the pre-migration derived speed")
    end)

    t.it("shot_speed increases with strength without changing the established mapping", function()
        local weak = stats.shot_speed(block(5, 2))
        local strong = stats.shot_speed(block(5, 8))
        t.is_true(strong > weak, "stronger player should shoot faster")
        t.eq(strong, 550, "strength 8 keeps the pre-migration derived shot speed")
    end)

    t.it("move_speed is positive at zero pace", function()
        t.is_true(stats.move_speed(block(0, 0)) > 0)
    end)

    t.it("maps outfield scan rate exactly and clamps it to the unit interval", function()
        t.eq(stats.scan_rate(block(5, 5, 0, 5, 0)), 0)
        t.eq(stats.scan_rate(block(5, 5, 5, 5, 5)), 0.5)
        t.eq(stats.scan_rate(block(5, 5, 10, 5, 10)), 1)
        t.eq(stats.scan_rate(block(5, 5, -2, 5, -2)), 0)
        t.eq(stats.scan_rate(block(5, 5, 12, 5, 12)), 1)
    end)

    t.it("weights mental and stamina independently in outfield scan rate", function()
        t.eq(stats.scan_rate(block(5, 5, 8, 5, 4)), 0.7)
        t.eq(stats.scan_rate(block(5, 5, 4, 5, 8)), 0.5)
    end)

    t.it("never lowers outfield scan rate as mental or stamina increases", function()
        local previous_mental = stats.scan_rate(block(5, 5, 0, 5, 5))
        local previous_stamina = stats.scan_rate(block(5, 5, 5, 5, 0))
        for value = 1, 10 do
            local current_mental = stats.scan_rate(block(5, 5, value, 5, 5))
            local current_stamina = stats.scan_rate(block(5, 5, 5, 5, value))
            t.is_true(current_mental >= previous_mental)
            t.is_true(current_stamina >= previous_stamina)
            previous_mental = current_mental
            previous_stamina = current_stamina
        end
    end)

    t.it("maps outfield composure and press discipline from mental only", function()
        local functions = { stats.composure, stats.press_discipline }
        for _, derive in ipairs(functions) do
            t.eq(derive(block(5, 5, 0)), 0)
            t.eq(derive(block(5, 5, 5)), 0.5)
            t.eq(derive(block(5, 5, 10)), 1)
            t.eq(derive(block(5, 5, -2)), 0)
            t.eq(derive(block(5, 5, 12)), 1)
            t.eq(derive(block(0, 0, 5, 0, 0)), 0.5)
            t.eq(derive(block(10, 10, 5, 10, 10)), 0.5)
        end
    end)

    t.it("never lowers outfield composure or press discipline as mental increases", function()
        local previous_composure = stats.composure(block(5, 5, 0))
        local previous_discipline = stats.press_discipline(block(5, 5, 0))
        for mental = 1, 10 do
            local current_composure = stats.composure(block(5, 5, mental))
            local current_discipline = stats.press_discipline(block(5, 5, mental))
            t.is_true(current_composure >= previous_composure)
            t.is_true(current_discipline >= previous_discipline)
            previous_composure = current_composure
            previous_discipline = current_discipline
        end
    end)

    t.it("maps outfield run drive exactly and clamps it to the unit interval", function()
        t.eq(stats.run_drive(block(0, 5, 0)), 0)
        t.eq(stats.run_drive(block(5, 5, 5)), 0.5)
        t.eq(stats.run_drive(block(10, 5, 10)), 1)
        t.eq(stats.run_drive(block(-2, 5, -2)), 0)
        t.eq(stats.run_drive(block(12, 5, 12)), 1)
        t.eq(stats.run_drive(block(8, 5, 4)), 0.64)
    end)

    t.it("never lowers outfield run drive as pace or mental increases", function()
        local previous_pace = stats.run_drive(block(0, 5, 5))
        local previous_mental = stats.run_drive(block(5, 5, 0))
        for value = 1, 10 do
            local current_pace = stats.run_drive(block(value, 5, 5))
            local current_mental = stats.run_drive(block(5, 5, value))
            t.is_true(current_pace >= previous_pace)
            t.is_true(current_mental >= previous_mental)
            previous_pace = current_pace
            previous_mental = current_mental
        end
    end)

    t.it("maps technique to a bounded maximum execution error in radians", function()
        local maximum = math.pi / 15
        t.eq(stats.execution_error(block(5, 5, 5, 0)), maximum)
        t.eq(stats.execution_error(block(5, 5, 5, 5)), maximum / 2)
        t.eq(stats.execution_error(block(5, 5, 5, 10)), 0)
        t.eq(stats.execution_error(block(5, 5, 5, -2)), maximum)
        t.eq(stats.execution_error(block(5, 5, 5, 12)), 0)
    end)

    t.it("never increases execution error as technique increases", function()
        local previous = stats.execution_error(block(5, 5, 5, 0))
        for technique = 1, 10 do
            local current = stats.execution_error(block(5, 5, 5, technique))
            t.is_true(current <= previous)
            previous = current
        end
    end)

    t.it("keeps unrelated stats out of outfield behavior derivations", function()
        local low = block(4, 0, 6, 7, 8)
        local high = block(4, 10, 6, 7, 8)
        t.eq(stats.scan_rate(low), stats.scan_rate(high))
        t.eq(stats.composure(low), stats.composure(high))
        t.eq(stats.press_discipline(low), stats.press_discipline(high))
        t.eq(stats.run_drive(low), stats.run_drive(high))
        t.eq(stats.execution_error(low), stats.execution_error(high))
    end)

    t.it("derives keeper reach from mental and pace", function()
        local composed = stats.keeper_reach(block(4, 5, 8))
        local unsettled = stats.keeper_reach(block(4, 5, 2))
        t.eq(composed, 78, "the migrated mental value keeps the existing reach mapping")
        t.is_true(composed > unsettled, "mental should improve derived defensive reach")
    end)

    t.it("maps keeper anticipation exactly and clamps it to the unit interval", function()
        t.eq(stats.keeper_anticipation(block(5, 5, 0)), 0)
        t.eq(stats.keeper_anticipation(block(5, 5, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 5, 10)), 1)
        t.eq(stats.keeper_anticipation(block(5, 5, -2)), 0)
        t.eq(stats.keeper_anticipation(block(5, 5, 12)), 1)
    end)

    t.it("never lowers keeper anticipation as mental increases", function()
        local previous = stats.keeper_anticipation(block(5, 5, 0))
        for mental = 1, 10 do
            local current = stats.keeper_anticipation(block(5, 5, mental))
            t.is_true(current >= previous)
            previous = current
        end
    end)

    t.it("uses only mental to derive keeper anticipation", function()
        t.eq(stats.keeper_anticipation(block(0, 5, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(10, 5, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 0, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 10, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 5, 5, 0, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 5, 5, 10, 5)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 5, 5, 5, 0)), 0.5)
        t.eq(stats.keeper_anticipation(block(5, 5, 5, 5, 10)), 0.5)
    end)

    t.it("maps keeper aggression to a conservative positive pixel distance", function()
        t.eq(stats.keeper_aggression(block(0, 5, 0)), 18)
        t.eq(stats.keeper_aggression(block(5, 5, 5)), 38)
        t.eq(stats.keeper_aggression(block(10, 5, 10)), 58)
    end)

    t.it("adds exact independent pace and mental contributions to keeper aggression", function()
        local pace_four = stats.keeper_aggression(block(4, 5, 7))
        local pace_five = stats.keeper_aggression(block(5, 5, 7))
        t.eq(pace_four, 40)
        t.eq(pace_five, 42)
        t.eq(pace_five - pace_four, 2)

        local mental_four = stats.keeper_aggression(block(7, 5, 4))
        local mental_five = stats.keeper_aggression(block(7, 5, 5))
        t.eq(mental_four, 40)
        t.eq(mental_five, 42)
        t.eq(mental_five - mental_four, 2)
    end)

    t.it("never lowers keeper aggression as pace increases", function()
        local previous = stats.keeper_aggression(block(0, 5, 5))
        for pace = 1, 10 do
            local current = stats.keeper_aggression(block(pace, 5, 5))
            t.is_true(current >= previous)
            previous = current
        end
    end)

    t.it("never lowers keeper aggression as mental increases", function()
        local previous = stats.keeper_aggression(block(5, 5, 0))
        for mental = 1, 10 do
            local current = stats.keeper_aggression(block(5, 5, mental))
            t.is_true(current >= previous)
            previous = current
        end
    end)

    t.it("uses only pace and mental to derive keeper aggression", function()
        t.eq(stats.keeper_aggression(block(5, 0, 5, 5, 5)), 38)
        t.eq(stats.keeper_aggression(block(5, 10, 5, 5, 5)), 38)
        t.eq(stats.keeper_aggression(block(5, 5, 5, 0, 5)), 38)
        t.eq(stats.keeper_aggression(block(5, 5, 5, 10, 5)), 38)
        t.eq(stats.keeper_aggression(block(5, 5, 5, 5, 0)), 38)
        t.eq(stats.keeper_aggression(block(5, 5, 5, 5, 10)), 38)
    end)

    t.it("maps keeper distribution accuracy exactly and clamps it to the unit interval", function()
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, 0)), 0)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, 10)), 1)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, -2)), 0)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, 12)), 1)
    end)

    t.it("never lowers keeper distribution accuracy as technique increases", function()
        local previous = stats.keeper_distribution_accuracy(block(5, 5, 5, 0))
        for technique = 1, 10 do
            local current = stats.keeper_distribution_accuracy(block(5, 5, 5, technique))
            t.is_true(current >= previous)
            previous = current
        end
    end)

    t.it("uses only technique to derive keeper distribution accuracy", function()
        t.eq(stats.keeper_distribution_accuracy(block(0, 5, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(10, 5, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 0, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 10, 5, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 0, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 10, 5, 5)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, 5, 0)), 0.5)
        t.eq(stats.keeper_distribution_accuracy(block(5, 5, 5, 5, 10)), 0.5)
    end)
end)
