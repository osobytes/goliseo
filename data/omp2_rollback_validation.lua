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

--- A crowded combat *load* fixture and its same-seed combat-disabled twin.
--- `combat` false builds the identical match, layout, and input frames without a
--- CombatMatchState companion, so the pair differs only by combat being active and
--- the combat cost is attributable rather than merely asserted.
--- `repeated_loadout_id` forces one action family onto every outfielder, which is how
--- a fixture reaches the repeated-family load the authored mixed roster cannot produce.
---@class Omp2RollbackCombatLoadFixture
---@field id string
---@field scenario string
---@field layout "crowded"|"pocket"
---@field seed integer
---@field frame_count integer
---@field duration integer
---@field combat boolean
---@field repeated_loadout_id string?
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
---@field combat_load_fixtures Omp2RollbackCombatLoadFixture[]
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
        initial_hash = "6cf1f873f439f17e",
        final_hash = "40e0c1ab1984058c",
        tape_digest = "5172e0f60b87978e",
    },
    combat_load_fixtures = {
        {
            id = "omp2-combat-crowded-v1",
            scenario = "combat_crowded",
            layout = "crowded",
            seed = 941,
            frame_count = 160,
            duration = 20,
            combat = true,
            initial_hash = "627341cee2cca8e5",
            final_hash = "6a909c2bec42cd35",
            tape_digest = "ddf210a01e74c3b0",
        },
        {
            id = "omp2-combat-crowded-disabled-v1",
            scenario = "combat_crowded_disabled",
            layout = "crowded",
            seed = 941,
            frame_count = 160,
            duration = 20,
            combat = false,
            initial_hash = "cf44eafde0cfc459",
            final_hash = "a473429964f3e112",
            tape_digest = "1738a028e2fb1927",
        },
        {
            id = "omp2-combat-repeated-family-v1",
            scenario = "combat_repeated_family",
            layout = "pocket",
            seed = 977,
            frame_count = 160,
            duration = 20,
            combat = true,
            repeated_loadout_id = "loadout_spring_gloves",
            initial_hash = "e3a95013ce178fe4",
            final_hash = "502ba7eab6393518",
            tape_digest = "3736fc14e0487138",
        },
        {
            id = "omp2-combat-repeated-family-disabled-v1",
            scenario = "combat_repeated_family_disabled",
            layout = "pocket",
            seed = 977,
            frame_count = 160,
            duration = 20,
            combat = false,
            repeated_loadout_id = "loadout_spring_gloves",
            initial_hash = "3130c650a91d3e21",
            final_hash = "cbb297518f5862d3",
            tape_digest = "8e34403dfac85f7b",
        },
    },
    budgets = {
        p95_work_ms = 16.67,
        rollback_p999_ms = 33.3,
        snapshot_count = 31,
        -- Raised one 128-KiB step from 768 KiB / 1 MiB by #209. Both moved together
        -- so the 256-KiB gap between them is unchanged: the snapshot window stays the
        -- binding gate and history stays the backstop for non-snapshot retention.
        -- `scripts/rollback_validation.py` mirrors both; a stale mirror is a defect.
        snapshot_bytes = 896 * 1024,
        history_bytes = 1152 * 1024,
        memory_growth_ratio = 0.10,
    },
    soak_network_seeds = { 2001, 2002, 2003, 2001, 2002 },
    soak_samples = { "warmup", "120", "360", "600", "final" },
}
