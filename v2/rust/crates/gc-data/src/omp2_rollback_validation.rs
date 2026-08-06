//! OMP-2 rollback validation fixture and budget data.

/// The kind of rollback validation scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Omp2RollbackScenarioKind {
    /// A boundary window scenario.
    Window,
    /// A synthetic-goal lifecycle scenario.
    SyntheticGoal,
    /// A repeated-rollback scenario.
    Repeated,
}

/// A single rollback validation scenario.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Omp2RollbackScenario {
    /// Persistent identity.
    pub id: &'static str,
    /// Scenario kind.
    pub kind: Omp2RollbackScenarioKind,
    /// First boundary tick, for window/repeated scenarios.
    pub first_boundary: Option<i64>,
    /// Last boundary tick, for window/repeated scenarios.
    pub last_boundary: Option<i64>,
    /// Event kind the window is scoped around, if any.
    pub event_kind: Option<&'static str>,
    /// Match lifecycle kind, for synthetic-goal scenarios.
    pub lifecycle_kind: Option<&'static str>,
    /// Minimum rollback count required, for repeated scenarios.
    pub minimum_rollbacks: Option<i64>,
}

/// Performance and footprint budgets the rollback system must stay within.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Omp2RollbackBudgets {
    /// 95th percentile per-frame work budget, in milliseconds.
    pub p95_work_ms: f64,
    /// 99.9th percentile rollback budget, in milliseconds.
    pub rollback_p999_ms: f64,
    /// Retained snapshot count.
    pub snapshot_count: i64,
    /// Snapshot window budget, in bytes.
    pub snapshot_bytes: i64,
    /// History retention budget, in bytes.
    pub history_bytes: i64,
    /// Maximum allowed memory growth ratio over a soak run.
    pub memory_growth_ratio: f64,
}

/// A combat rollback determinism fixture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Omp2RollbackCombatFixture {
    /// Persistent identity.
    pub id: &'static str,
    /// RNG seed.
    pub seed: i64,
    /// Frame count.
    pub frame_count: i64,
    /// Hash of the initial state.
    pub initial_hash: &'static str,
    /// Hash of the final state.
    pub final_hash: &'static str,
    /// Digest over the recorded input tape.
    pub tape_digest: &'static str,
}

/// A crowded/pocket layout for a combat load fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Omp2RollbackLayout {
    /// A crowded layout, stressing many simultaneous encounters.
    Crowded,
    /// A pocket layout, isolating a small encounter.
    Pocket,
}

/// A crowded combat *load* fixture and its same-seed combat-disabled twin.
/// `combat` false builds the identical match, layout, and input frames without a
/// CombatMatchState companion, so the pair differs only by combat being active and
/// the combat cost is attributable rather than merely asserted.
/// `repeated_loadout_id` forces one action family onto every outfielder, which is how
/// a fixture reaches the repeated-family load the authored mixed roster cannot produce.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Omp2RollbackCombatLoadFixture {
    /// Persistent identity.
    pub id: &'static str,
    /// Scenario name.
    pub scenario: &'static str,
    /// Layout family.
    pub layout: Omp2RollbackLayout,
    /// RNG seed.
    pub seed: i64,
    /// Frame count.
    pub frame_count: i64,
    /// Duration, in seconds.
    pub duration: i64,
    /// Whether combat is active for this fixture.
    pub combat: bool,
    /// Loadout id forced onto every outfielder, if this fixture repeats one family.
    pub repeated_loadout_id: Option<&'static str>,
    /// Hash of the initial state.
    pub initial_hash: &'static str,
    /// Hash of the final state.
    pub final_hash: &'static str,
    /// Digest over the recorded input tape.
    pub tape_digest: &'static str,
}

/// The OMP-2 rollback validation fixture and budget register.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Omp2RollbackValidationData {
    /// Schema version.
    pub schema: i64,
    /// RNG seed used to build the fixture match.
    pub fixture_seed: i64,
    /// Network condition seeds used across the validation suite.
    pub network_seeds: &'static [i64],
    /// Source input pattern (L = local, R = remote).
    pub source_pattern: &'static str,
    /// Network profiles exercised by the full validation suite.
    pub full_profiles: &'static [&'static str],
    /// Network profiles exercised by the browser validation suite.
    pub browser_full_profiles: &'static [&'static str],
    /// Network profile used for stress runs.
    pub stress_profile: &'static str,
    /// Rollback validation scenarios.
    pub scenarios: &'static [Omp2RollbackScenario],
    /// The combat rollback determinism fixture.
    pub combat_fixture: Omp2RollbackCombatFixture,
    /// Combat load fixtures.
    pub combat_load_fixtures: &'static [Omp2RollbackCombatLoadFixture],
    /// Performance and footprint budgets.
    pub budgets: Omp2RollbackBudgets,
    /// Network condition seeds used across the soak suite.
    pub soak_network_seeds: &'static [i64],
    /// Named sample points taken during a soak run.
    pub soak_samples: &'static [&'static str],
}

/// The OMP-2 rollback validation fixture and budget register.
pub const DATA: Omp2RollbackValidationData = Omp2RollbackValidationData {
    schema: 1,
    fixture_seed: 19,
    network_seeds: &[2001, 2002, 2003],
    source_pattern: "LRRRRRRR",
    full_profiles: &["clean", "omp0_parity", "playable", "stress"],
    browser_full_profiles: &["clean", "playable"],
    stress_profile: "stress",
    scenarios: &[
        Omp2RollbackScenario {
            id: "possession_change",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(22),
            last_boundary: Some(27),
            event_kind: None,
            lifecycle_kind: None,
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "tackle",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(23),
            last_boundary: Some(26),
            event_kind: Some("tackle"),
            lifecycle_kind: None,
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "shot",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(1684),
            last_boundary: Some(1689),
            event_kind: Some("shot"),
            lifecycle_kind: None,
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "goal",
            kind: Omp2RollbackScenarioKind::SyntheticGoal,
            first_boundary: None,
            last_boundary: None,
            event_kind: None,
            lifecycle_kind: Some("goal"),
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "kickoff",
            kind: Omp2RollbackScenarioKind::SyntheticGoal,
            first_boundary: None,
            last_boundary: None,
            event_kind: None,
            lifecycle_kind: Some("kickoff"),
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "aerial",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(1786),
            last_boundary: Some(1791),
            event_kind: Some("header"),
            lifecycle_kind: None,
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "keeper_action",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(1690),
            last_boundary: Some(1695),
            event_kind: Some("catch"),
            lifecycle_kind: None,
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "repeated_rollback",
            kind: Omp2RollbackScenarioKind::Repeated,
            first_boundary: Some(0),
            last_boundary: Some(48),
            event_kind: None,
            lifecycle_kind: None,
            minimum_rollbacks: Some(2),
        },
        Omp2RollbackScenario {
            id: "full_time",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(7198),
            last_boundary: Some(7201),
            event_kind: None,
            lifecycle_kind: Some("full_time"),
            minimum_rollbacks: None,
        },
    ],
    combat_fixture: Omp2RollbackCombatFixture {
        id: "omp2-combat-rollback-v1",
        seed: 733,
        frame_count: 80,
        initial_hash: "6edfabacb5ecc6cd",
        final_hash: "822ca5cf529e725b",
        tape_digest: "da9d009342add99a",
    },
    combat_load_fixtures: &[
        Omp2RollbackCombatLoadFixture {
            id: "omp2-combat-crowded-v1",
            scenario: "combat_crowded",
            layout: Omp2RollbackLayout::Crowded,
            seed: 941,
            frame_count: 160,
            duration: 20,
            combat: true,
            repeated_loadout_id: None,
            initial_hash: "ba131fc89cabd89a",
            final_hash: "93206851a4a6455a",
            tape_digest: "570e79d1e5ac32fd",
        },
        Omp2RollbackCombatLoadFixture {
            id: "omp2-combat-crowded-disabled-v1",
            scenario: "combat_crowded_disabled",
            layout: Omp2RollbackLayout::Crowded,
            seed: 941,
            frame_count: 160,
            duration: 20,
            combat: false,
            repeated_loadout_id: None,
            initial_hash: "0c6f04fe7cdbdcb6",
            final_hash: "307cff049c8ea93f",
            tape_digest: "452e841205b6f510",
        },
        Omp2RollbackCombatLoadFixture {
            id: "omp2-combat-repeated-family-v1",
            scenario: "combat_repeated_family",
            layout: Omp2RollbackLayout::Pocket,
            seed: 977,
            frame_count: 160,
            duration: 20,
            combat: true,
            repeated_loadout_id: Some("loadout_spring_gloves"),
            initial_hash: "bfc22640819ecd1b",
            final_hash: "70c04d10f1e7ebf3",
            tape_digest: "fbb700e6b65acf3d",
        },
        Omp2RollbackCombatLoadFixture {
            id: "omp2-combat-repeated-family-disabled-v1",
            scenario: "combat_repeated_family_disabled",
            layout: Omp2RollbackLayout::Pocket,
            seed: 977,
            frame_count: 160,
            duration: 20,
            combat: false,
            repeated_loadout_id: Some("loadout_spring_gloves"),
            initial_hash: "11e9080994725ece",
            final_hash: "37b90de104d12a42",
            tape_digest: "f75bb3356ab89f8e",
        },
    ],
    budgets: Omp2RollbackBudgets {
        p95_work_ms: 16.67,
        rollback_p999_ms: 33.3,
        snapshot_count: 31,
        // Raised one 128-KiB step from 768 KiB / 1 MiB by #209. Both moved together
        // so the 256-KiB gap between them is unchanged: the snapshot window stays the
        // binding gate and history stays the backstop for non-snapshot retention.
        // `scripts/rollback_validation.py` mirrors both; a stale mirror is a defect.
        snapshot_bytes: 896 * 1024,
        history_bytes: 1152 * 1024,
        memory_growth_ratio: 0.10,
    },
    soak_network_seeds: &[2001, 2002, 2003, 2001, 2002],
    soak_samples: &["warmup", "120", "360", "600", "final"],
};
