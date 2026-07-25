---@class Omp2RollbackScenario
---@field id string
---@field kind "window"|"synthetic_goal"|"repeated"
---@field first_boundary integer?
---@field last_boundary integer?
---@field event_kind string?
---@field lifecycle_kind string?
---@field minimum_rollbacks integer?

---@class Omp2RollbackBudgets
---@field p95_work_ms number
---@field rollback_p999_ms number
---@field snapshot_count integer
---@field snapshot_bytes integer
---@field history_bytes integer
---@field memory_growth_ratio number

---@class Omp2RollbackCombatFixture
---@field id string
---@field seed integer
---@field frame_count integer
---@field initial_hash string
---@field final_hash string
---@field tape_digest string

---@class Omp2RollbackValidationData
---@field schema integer
---@field fixture_seed integer
---@field network_seeds integer[]
---@field source_pattern string
---@field full_profiles string[]
---@field browser_full_profiles string[]
---@field stress_profile string
---@field scenarios Omp2RollbackScenario[]
---@field combat_fixture Omp2RollbackCombatFixture
---@field budgets Omp2RollbackBudgets
---@field soak_network_seeds integer[]
---@field soak_samples string[]

---@type Omp2RollbackValidationData
return {
    schema = 1,
    fixture_seed = 19,
    network_seeds = { 2001, 2002, 2003 },
    source_pattern = "LRRRRRRR",
    full_profiles = { "clean", "omp0_parity", "playable", "stress" },
    browser_full_profiles = { "clean", "playable" },
    stress_profile = "stress",
    scenarios = {
        {
            id = "possession_change",
            kind = "window",
            first_boundary = 22,
            last_boundary = 27,
        },
        {
            id = "tackle",
            kind = "window",
            first_boundary = 23,
            last_boundary = 26,
            event_kind = "tackle",
        },
        {
            id = "shot",
            kind = "window",
            first_boundary = 1684,
            last_boundary = 1689,
            event_kind = "shot",
        },
        {
            id = "goal",
            kind = "synthetic_goal",
            lifecycle_kind = "goal",
        },
        {
            id = "kickoff",
            kind = "synthetic_goal",
            lifecycle_kind = "kickoff",
        },
        {
            id = "aerial",
            kind = "window",
            first_boundary = 1786,
            last_boundary = 1791,
            event_kind = "header",
        },
        {
            id = "keeper_action",
            kind = "window",
            first_boundary = 1690,
            last_boundary = 1695,
            event_kind = "catch",
        },
        {
            id = "repeated_rollback",
            kind = "repeated",
            first_boundary = 0,
            last_boundary = 48,
            minimum_rollbacks = 2,
        },
        {
            id = "full_time",
            kind = "window",
            first_boundary = 7198,
            last_boundary = 7201,
            lifecycle_kind = "full_time",
        },
    },
    combat_fixture = {
        id = "omp2-combat-rollback-v1",
        seed = 733,
        frame_count = 80,
        initial_hash = "28b7bc6393447100",
        final_hash = "49b2a6d0255a0825",
        tape_digest = "ef9d425396654839",
    },
    budgets = {
        p95_work_ms = 16.67,
        rollback_p999_ms = 33.3,
        snapshot_count = 31,
        snapshot_bytes = 768 * 1024,
        history_bytes = 1024 * 1024,
        memory_growth_ratio = 0.10,
    },
    soak_network_seeds = { 2001, 2002, 2003, 2001, 2002 },
    soak_samples = { "warmup", "120", "360", "600", "final" },
}
