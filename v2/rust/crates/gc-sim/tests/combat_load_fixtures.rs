//! Port of `spec/sim/combat_load_fixtures_spec.lua`.
//!
//! Every case in this spec drives `sim::match` (`match.new`/`match.step`,
//! not yet ported) through `sim.rollback_lab`, `sim.rollback_validation`,
//! and `sim.input_tape` — none of which are ported in this crate either
//! (they belong to the rollback layer built on top of this one). So none of
//! these eight cases can be expressed yet — each is stubbed here,
//! accurately named and `#[ignore]`d, so the assertion set is tracked
//! rather than silently dropped.

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::match (sim/match.lua), not yet ported"]
        fn $name() {
            unimplemented!(
                "needs sim::match (sim/match.lua), plus the unported sim::rollback_lab / \
                 sim::rollback_validation / sim::input_tape rollback layer built on top of it"
            )
        }
    };
}

blocked!(omp_2_crowded_combat_load_fixtures_builds_every_pinned_fixture_at_its_recorded_identity);
blocked!(
    omp_2_crowded_combat_load_fixtures_declares_the_artifact_versions_its_combat_companion_implies
);
blocked!(omp_2_crowded_combat_load_fixtures_pairs_each_fixture_with_a_byte_identical_combat_disabled_twin);
blocked!(omp_2_crowded_combat_load_fixtures_drives_all_four_action_families_in_the_crowded_fixture);
blocked!(omp_2_crowded_combat_load_fixtures_puts_every_outfielder_on_one_family_in_the_repeated_family_fixture);
blocked!(
    omp_2_crowded_combat_load_fixtures_keeps_the_combat_disabled_twins_free_of_combat_entirely
);
blocked!(
    omp_2_crowded_combat_load_fixtures_converges_inside_the_pinned_snapshot_and_history_budgets
);
blocked!(
    omp_2_crowded_combat_load_fixtures_rejects_a_case_whose_combat_presence_contradicts_its_fixture
);
