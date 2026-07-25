local contract = require("game.match_contract")
local t = require("spec.support.runner")
local teams = require("data.teams")

t.describe("match contracts", function()
    t.it("builds an immutable validated request", function()
        local source = { "ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex" }
        local request = assert(contract.new_request({
            home_team_id = "nebula",
            away_team_id = "orion",
            home_starter_ids = source,
            formation_id = "1-2-1",
            tactic_id = "counter",
            seed = 42,
        }))
        source[1] = "changed"
        t.eq(request.home_starter_ids[1], "ozzo")
        t.eq(request.arena_id, "helios_crown")
        t.eq(request.show_onboarding, false)
        t.eq(request.combat_enabled, false)
        t.eq(request.seed, 42)

        local combat_request = assert(contract.new_request({
            home_team_id = "nebula",
            away_team_id = "orion",
            home_starter_ids = { "ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex" },
            formation_id = "1-2-1",
            tactic_id = "counter",
            combat_enabled = true,
        }))
        t.eq(combat_request.combat_enabled, true)
    end)

    t.it("rejects unknown arena presentation records", function()
        local request, err = contract.new_request({
            home_team_id = "nebula",
            away_team_id = "orion",
            home_starter_ids = { "ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex" },
            formation_id = "1-2-1",
            tactic_id = "counter",
            arena_id = "unfinished_moon",
        })
        t.is_true(request == nil)
        t.is_true(assert(err):match("arena") ~= nil)
    end)

    t.it("rejects duplicate, ineligible, and keeperless team sheets", function()
        local ok, err = contract.validate_starters(
            { "ozzo", "brakka", "brakka", "rok_tann", "zyro_vex" },
            teams.nebula
        )
        t.is_true(ok == nil)
        t.is_true(assert(err):match("unique") ~= nil)

        ok, err = contract.validate_starters(
            { "gax_oru", "brakka", "veil_nyx", "rok_tann", "zyro_vex" },
            teams.nebula
        )
        t.is_true(ok == nil)
        t.is_true(assert(err):match("eligible") ~= nil)

        ok, err = contract.validate_starters(
            { "brakka", "veil_nyx", "rok_tann", "zyro_vex", "mika_olu" },
            teams.nebula
        )
        t.is_true(ok == nil)
        t.is_true(assert(err):match("keeper") ~= nil)
    end)

    t.it("derives winner and accepts unavailable optional metrics", function()
        local result = assert(contract.new_result({
            home_team_id = "nebula",
            away_team_id = "orion",
            home_score = 2,
            away_score = 1,
        }))
        t.eq(result.winner, "home")
        t.is_true(result.home_stats.shots == nil)
    end)

    t.it("rejects invalid result ratios", function()
        local result, err = contract.new_result({
            home_team_id = "nebula",
            away_team_id = "orion",
            home_score = 0,
            away_score = 0,
            home_stats = { possession = 1.2 },
        })
        t.is_true(result == nil)
        t.is_true(assert(err):match("possession") ~= nil)
    end)
end)
