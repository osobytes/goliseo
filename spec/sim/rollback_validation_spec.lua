local rollback_validation = require("sim.rollback_validation")
local t = require("spec.support.runner")

t.describe("OMP-2 rollback validation campaign", function()
    t.it("pins the required scenario and runtime matrix", function()
        local config = rollback_validation.config()
        t.eq(config.fixture_seed, 19)
        t.eq(config.source_pattern, "LRRRRRRR")
        t.eq(#config.network_seeds, 3)
        t.eq(config.network_seeds[1], 2001)
        t.eq(config.network_seeds[3], 2003)
        t.eq(#config.full_profiles, 4)
        t.eq(#config.scenarios, 9)
        t.eq(#config.soak_network_seeds, 5)
        t.eq(config.soak_network_seeds[1], 2001)
        t.eq(config.soak_network_seeds[2], 2002)
        t.eq(config.soak_network_seeds[3], 2003)
        t.eq(config.soak_network_seeds[4], 2001)
        t.eq(config.soak_network_seeds[5], 2002)
        t.eq(#config.soak_samples, 5)
        t.eq(config.combat_fixture.id, "omp2-combat-rollback-v1")
        t.eq(config.combat_fixture.frame_count, 80)
        t.eq(config.soak_samples[1], "warmup")
        t.eq(config.soak_samples[2], "120")
        t.eq(config.soak_samples[3], "360")
        t.eq(config.soak_samples[4], "600")
        t.eq(config.soak_samples[5], "final")
        t.eq(config.budgets.snapshot_count, 31)
        t.eq(config.budgets.snapshot_bytes, 768 * 1024)
        t.eq(config.budgets.history_bytes, 1024 * 1024)
        t.eq(config.budgets.memory_growth_ratio, 0.10)
        t.eq(rollback_validation.profile_digest(), "5fbf1e0d51a6f4d5")

        local browser = rollback_validation.new_campaign("browser-stress", {
            profile_name = "stress",
            network_seed = 2001,
        })
        t.eq(#browser.cases, 10)
        local seen = {}
        for _, case in ipairs(browser.cases) do
            seen[case.scenario] = true
        end
        for _, scenario in ipairs(config.scenarios) do
            t.is_true(seen[scenario.id], "missing scenario " .. scenario.id)
        end
        t.is_true(seen.combat, "missing bounded combat scenario")
    end)

    t.it("accepts delay thirty and classifies delay thirty-one as the explicit terminal", function()
        local campaign = rollback_validation.new_campaign("late-window")
        local completed = {}
        local result = nil
        while result == nil do
            local row
            result, row = rollback_validation.step_campaign(campaign, 4)
            if row then
                completed[#completed + 1] = row
            end
        end

        t.is_true(result.success)
        t.eq(result.case_count, 2)
        t.eq(#completed, 2)
        t.eq(completed[1].id, "delay-30")
        t.is_true(completed[1].result.success)
        t.eq(completed[1].result.metrics.max_rollback_depth, 30)
        t.eq(completed[2].id, "delay-31")
        t.is_true(completed[2].accepted)
        t.is_true(completed[2].expected_failure)
        t.eq(completed[2].result.status, "late_input_unrecoverable")
        t.eq(completed[2].result.late_input_tick, 0)
        t.is_true(not completed[2].hidden_progress)
        t.is_true(rollback_validation.case_marker(completed[2]):match("expected_failure=1") ~= nil)
        t.is_true(
            rollback_validation.result_marker(result):match("^GC_ROLLBACK_VALIDATION|result|")
                ~= nil
        )
    end)
end)
