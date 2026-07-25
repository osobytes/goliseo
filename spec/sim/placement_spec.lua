local t = require("spec.support.runner")
local placement = require("sim.placement")
local formations = require("data.formations")

local field = { w = 960, h = 540 }
local f = formations["2-1-1"]

t.describe("placement.anchors", function()
    t.it("produces a keeper plus four outfield anchors", function()
        t.eq(#placement.anchors(f, "home", field), 5)
    end)

    t.it("places the home keeper left of centre, away keeper right of centre", function()
        local home = placement.anchors(f, "home", field)
        local away = placement.anchors(f, "away", field)
        t.is_true(home[1].x < field.w / 2)
        t.is_true(away[1].x > field.w / 2)
    end)

    t.it("mirrors away anchors across the vertical centre line", function()
        local home = placement.anchors(f, "home", field)
        local away = placement.anchors(f, "away", field)
        for i = 1, #home do
            t.near(home[i].x + away[i].x, field.w)
            t.near(home[i].y, away[i].y)
        end
    end)

    t.it("tags every built-in outfield slot with the closed role contract", function()
        local expected = {
            ["2-1-1"] = { "def", "def", "mid", "fwd" },
            ["1-2-1"] = { "def", "wide", "wide", "fwd" },
            ["1-1-2"] = { "def", "mid", "fwd", "fwd" },
        }
        local allowed = { def = true, mid = true, wide = true, fwd = true }
        for formation_id, roles in pairs(expected) do
            local formation = assert(formations[formation_id])
            t.eq(#formation.outfield, 4)
            for ordinal, anchor in ipairs(formation.outfield) do
                t.is_true(allowed[anchor.role] == true)
                t.eq(anchor.role, roles[ordinal], formation_id .. " slot " .. ordinal)
            end
        end
    end)
end)
