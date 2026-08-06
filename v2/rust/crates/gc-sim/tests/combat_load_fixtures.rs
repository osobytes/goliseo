//! Port of `spec/sim/combat_load_fixtures_spec.lua`.
//!
//! Every case in this spec depends on `rollback_validation.combat_load_tape
//! (scenario)`, the Lua module's scenario-by-name pinned-tape builder.
//! `sim::match`, `sim::rollback_lab`, and `sim::input_tape` are all fully
//! ported now (see `crates/gc-sim/src/rollback_lab.rs` and
//! `crates/gc-sim/src/rollback_validation.rs`), so the blocker this file's
//! stubs originally named ("needs sim::match, not yet ported") is stale —
//! but the blocker moved rather than closed. `rollback_validation.rs`'s
//! `combat_load_tape(fixture: &Omp2RollbackCombatLoadFixture, tune:
//! &Tuning)` — along with every helper it calls (`layout_positions`,
//! `repeated_family_roster`, `combat_load_slot_plans`,
//! `combat_load_frames`) — is a private `fn`, not `pub fn`, and there is no
//! scenario-by-name public wrapper around it (only
//! `rollback_validation::combat_load_covered_for(result, scenario)` is
//! `pub`, and it still needs a `RollbackLabResult` built from this same
//! tape to call). Every one of the eight cases below reads `tape.frames`,
//! `tape.identity`, or `tape.boundary_hashes` directly, or goes through
//! `lab_result(scenario)` (`rollback_lab::run` over this same tape), so all
//! eight are blocked on the identical missing `pub` accessor. Reimplementing
//! `combat_load_tape`'s ~150 lines of fixture construction independently in
//! this test file was rejected: these fixtures' whole purpose is a
//! bit-exact pinned identity (`initial_hash`/`final_hash`/`tape_digest`
//! asserted in the first case alone), so a second, independently-written
//! construction path risks silently diverging from the pinned one and
//! testing a fixture that only resembles the real one — worse than an
//! honest gap. Report this precisely: it needs a `pub` change in
//! `rollback_validation.rs` (out of scope for this pass — test files only),
//! not more test-file work.

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs a pub accessor: rollback_validation::combat_load_tape(fixture, tune) \
                     and the helpers it calls are private, with no scenario-by-name pub wrapper; \
                     see this file's module doc comment"]
        fn $name() {
            unimplemented!(
                "needs rollback_validation::combat_load_tape (or an equivalent scenario-by-name \
                 pub wrapper) to be made pub; reimplementing its fixture construction here would \
                 risk silently diverging from the pinned tape these tests exist to guard"
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
