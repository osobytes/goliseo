local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local outfield_press = require("sim.outfield_press")

---@param overrides table?
---@return OutfieldPressContext
local function press_context(overrides)
    local context = {
        heavy_touch = false,
        exposed_ball = false,
        cover_available = false,
        box_desperation = false,
        press_discipline = 0.8,
    }
    for key, value in pairs(overrides or {}) do
        context[key] = value
    end
    return context
end

t.describe("team-owned outfield press state", function()
    t.it("keeps an eligible presser at 86% cost and switches at the exact 85% boundary", function()
        local candidates = {
            { player_index = 2, distance_cost = 100 },
            { player_index = 3, distance_cost = 86 },
        }
        t.eq(outfield_press.assign_presser(candidates, 2), 2)
        candidates[2].distance_cost = 85
        t.eq(outfield_press.assign_presser(candidates, 2), 3)
    end)

    t.it("uses a conservative 0.35 low-discipline boundary", function()
        local boundary = outfield_press.resolve(2, press_context({ press_discipline = 0.35 }))
        t.eq(boundary.mode, "contain")
        t.eq(boundary.reason, "no_trigger")

        local fallback = outfield_press.resolve(2, press_context({ press_discipline = 0.349 }))
        t.eq(fallback.mode, "commit")
        t.eq(fallback.reason, "low_discipline")
    end)

    t.it("keeps heavy touch first in the stable multi-trigger precedence", function()
        local state = outfield_press.resolve(
            2,
            press_context({
                heavy_touch = true,
                exposed_ball = true,
                cover_available = true,
                box_desperation = true,
                press_discipline = 0,
            })
        )
        t.eq(state.mode, "commit")
        t.eq(state.reason, "heavy_touch")
    end)

    t.it("validates inactive, contain, and attributed commit relations", function()
        t.eq(outfield_press.new_state().mode, "inactive")
        t.eq(outfield_press.contain(2).reason, "no_trigger")
        local malformed = outfield_press.contain(2)
        malformed.reason = "cover"
        t.is_true(not pcall(function()
            outfield_press.copy_state(malformed)
        end))
    end)
end)

t.describe("outfield press geometry and tuning", function()
    t.it("holds goal-side while conceding eight pixels toward pitch centre", function()
        local carrier = Vec2.new(480, 100)
        local target = outfield_press.contain_target(carrier, Vec2.new(0, 270), 540, 32)
        t.is_true(target.x < carrier.x, "contain target remains goal-side")
        local unbiassed = carrier:add(Vec2.new(0, 270):sub(carrier):normalized():scale(32))
        t.near(target.y, unbiassed.y + 8)
    end)

    t.it("slows only inside the named 60-pixel contain radius", function()
        t.near(outfield_press.contain_speed(200, 61, 0.75), 200)
        t.near(outfield_press.contain_speed(200, 60, 0.75), 150)
    end)

    t.it("accepts goal-side cover through the named 140-pixel boundary", function()
        t.is_true(outfield_press.cover_available(140, true))
        t.is_true(not outfield_press.cover_available(140.001, true))
        t.is_true(not outfield_press.cover_available(100, false))
    end)

    t.it("shadows the highest-scored eligible lane with stable index ties", function()
        local carrier = Vec2.new(480, 270)
        local base = Vec2.new(336, 270)
        local candidates = {
            { player_index = 9, score = 10, pos = Vec2.new(360, 390) },
            { player_index = 8, score = 10, pos = Vec2.new(360, 150) },
            {
                player_index = 7,
                score = 99,
                pos = Vec2.new(300, 270),
                eligible = false,
            },
        }
        local best = assert(outfield_press.highest_scored_lane(candidates))
        t.eq(best.player_index, 8)
        local target = outfield_press.lane_shadow_target(base, carrier, candidates)
        t.is_true(target.y < base.y, "cover biases toward the selected upper lane")
        t.is_true(target.x > base.x, "lane bias preserves the interpose foundation")
    end)
end)
