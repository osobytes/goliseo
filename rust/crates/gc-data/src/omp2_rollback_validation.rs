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
    // #522 removed three scenarios -- `shot`, `aerial` and `keeper_action`.
    // This is a DELIBERATE NARROWING of the matrix, which is exactly what
    // `scripts/check_rollback_native.sh`'s `MIN_NATIVE_CASES` floor exists to
    // catch, so that floor moves in the same commit and for this reason.
    //
    // Each of the three scoped a window around an event the OMP-1 tape no
    // longer contains ANYWHERE: #488's locomotion rework drives the frozen
    // button presses to different places, and the shot, the header and the
    // catch stop happening. Unlike the tackle -- which merely moved, tick 24
    // to tick 31, and so is re-authored above -- there is no tick to re-point
    // these at. Re-pointing them at whatever event happens to be nearby would
    // be a window that covers something it is not named for.
    //
    // The coverage they were standing in for moves to #518, which asserts
    // behavior over a SEED SET rather than one frozen trajectory. That is the
    // durable form: a scenario pinned to one seed rots on any gameplay
    // change, which is the whole reason these three are being removed rather
    // than nudged.
    scenarios: &[
        // #522: shifted +7 boundaries (24 -> 31) for #488. #489 shifts them
        // again: a standing-poke tackle now charges and executes instead of
        // resolving instantly (`gc_sim::action_slot`), which moves the OMP-1
        // tape's first tackle again, tick 31 -> 40
        // (`gc_sim::determinism_evidence::record`'s `event_ticks["tackle"]`,
        // read directly off this build rather than assumed). Widened by a
        // couple of boundaries either side of the #488 precedent's offsets
        // to absorb the ambiguity between an event tick and the boundary
        // index it lands on, verified by running these two scenarios rather
        // than trusted from the arithmetic alone.
        Omp2RollbackScenario {
            id: "possession_change",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(37),
            last_boundary: Some(44),
            event_kind: None,
            lifecycle_kind: None,
            minimum_rollbacks: None,
        },
        Omp2RollbackScenario {
            id: "tackle",
            kind: Omp2RollbackScenarioKind::Window,
            first_boundary: Some(38),
            last_boundary: Some(43),
            event_kind: Some("tackle"),
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
    // NOTE on the `final_hash` / `tape_digest` fields below. Every
    // `initial_hash` here is the CONSTRUCTED fixture state and is frozen: it
    // is what makes "the fixture is the one that was recorded" a real claim.
    // The other two are what this build produces after stepping that state,
    // so a deliberate gameplay change moves them and only them. #488 moved
    // all eight while leaving all five `initial_hash` values untouched, which
    // is the check that the fixtures themselves did not drift. Re-derived by
    // running the failing assertion and reading the values it prints -- there
    // is no recorder for these the way `record_omp1_derived_baseline` is one
    // for OMP-1, and there should be.
    //
    // EXCEPTION, #531 phase 2: this is the first `match_snapshot::VERSION`
    // bump since the port (11 -> 12, the new `pass_intent` field on every
    // `MatchPlayer`). That changes the canonical byte encoding of EVERY
    // captured snapshot, including the CONSTRUCTED, zero-tick initial state
    // -- the same reason `gc_data::omp1_determinism::boundary_hash_lines()[0]`
    // moves too despite being described elsewhere as "the captured initial
    // state". So this one time, all four `initial_hash` values below moved
    // alongside `final_hash`/`tape_digest`, for the same schema reason and no
    // other -- not because any of these fixtures' constructed positions,
    // rosters or seeds changed. Re-derived the same way: run the failing
    // assertion and read the values it prints.
    // #489: match_snapshot::VERSION 12 -> 13 (MatchPlayer::action), the
    // second such bump since the #531 phase-2 note above and the same
    // schema-only reason -- all five `initial_hash` values below moved
    // alongside every `final_hash`/`tape_digest`, re-derived by running the
    // failing assertion and reading the values it printed.
    // #490: match_snapshot::VERSION 13 -> 14 (MatchPlayer::keeper_fatigue),
    // the third such bump and the same schema-only reason for the
    // `initial_hash` values -- a new `MatchPlayer` field changes the canonical
    // encoding of the CONSTRUCTED zero-tick state as much as of any other. The
    // `final_hash`/`tape_digest` values additionally reflect a real gameplay
    // change (the keeper's catch band resolves some saves as parries), which is
    // exactly what those two fields exist to move. Re-derived the same way:
    // run the failing assertion and read the values it printed.
    combat_fixture: Omp2RollbackCombatFixture {
        id: "omp2-combat-rollback-v1",
        seed: 733,
        frame_count: 80,
        initial_hash: "b21f3119c6903f71",
        final_hash: "30302128462635f7",
        tape_digest: "5f3000e8ff7a354c",
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
            initial_hash: "ccb4905c85efed14",
            final_hash: "800c2853618dff7a",
            tape_digest: "5d83af130b6b423e",
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
            initial_hash: "723c39b0d95a8cf5",
            final_hash: "ffebbb340f35575f",
            tape_digest: "4917379888a2076a",
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
            initial_hash: "eda58fe42b611b39",
            final_hash: "9032c4c1dad2d923",
            tape_digest: "e5d5c4ff43eeeb78",
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
            initial_hash: "9d69c2670bb33f6d",
            final_hash: "ca4d2f4b62599ba5",
            tape_digest: "9d8edc6cebfe36e8",
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
