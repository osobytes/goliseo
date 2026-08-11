//! Tests for combat load fixtures via `gc_sim::rollback_validation`.
//!
//! `rollback_validation::combat_load_tape(fixture, tune)` is now `pub` (see
//! that module's doc comment), so every case below runs directly against the
//! real pinned fixtures instead of the `#[ignore]`d stubs this file used to
//! carry, via `rollback_validation::combat_load_covered_for(result,
//! scenario)`.

use gc_data::action_families::ActionFamilyId;
use gc_data::loadouts;
use gc_data::omp2_rollback_validation::{self, Omp2RollbackCombatLoadFixture};
use gc_sim::combat;
use gc_sim::combat_snapshot::{
    CombatContactResult, CombatEvent, CombatEventKind, CombatMatchState,
};
use gc_sim::fixed_clock;
use gc_sim::input_frame;
use gc_sim::input_tape::{self, InputTape};
use gc_sim::r#match::{self as sim_match, StepInput};
use gc_sim::match_snapshot;
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_lab::{self, RollbackLabOptions, RollbackLabResult, RollbackLabStatus};
use gc_sim::rollback_validation;
use gc_sim::tuning::Tuning;

fn fixture_for(scenario: &str) -> &'static Omp2RollbackCombatLoadFixture {
    omp2_rollback_validation::DATA
        .combat_load_fixtures
        .iter()
        .find(|fixture| fixture.scenario == scenario)
        .unwrap_or_else(|| panic!("unknown combat load scenario {scenario}"))
}

fn tape_for(scenario: &str, tune: &Tuning) -> InputTape {
    rollback_validation::combat_load_tape(fixture_for(scenario), tune)
}

/// Replay a tape through the plain simulation, returning every combat event
/// observed across the whole run alongside the final combat companion.
fn replay_events(tape: &InputTape, tune: &Tuning) -> (Vec<CombatEvent>, CombatMatchState) {
    let (mut state, mut combat_state) = match_snapshot::restore(&tape.initial);
    let mut events = Vec::new();
    for frame in &tape.frames {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(frame),
            combat_state.as_mut(),
            tune,
        );
        if let Some(combat_state) = &combat_state {
            events.extend(combat_state.events.iter().cloned());
        }
    }
    (
        events,
        combat_state.expect("a combat load fixture always carries a combat companion"),
    )
}

fn event_count(
    events: &[CombatEvent],
    kind: CombatEventKind,
    result: Option<CombatContactResult>,
) -> usize {
    events
        .iter()
        .filter(|event| event.kind == kind && event.result == result)
        .count()
}

/// One real rollback-lab campaign over `scenario`'s pinned tape, run to
/// completion under the stress profile.
fn lab_result(scenario: &str, tune: &Tuning) -> RollbackLabResult {
    let tape = tape_for(scenario, tune);
    let mut sources = [RollbackInputSource::Remote; 8];
    sources[0] = RollbackInputSource::Local;
    let mut campaign = rollback_lab::new_campaign(
        tape,
        RollbackLabOptions {
            profile_name: Some(omp2_rollback_validation::DATA.stress_profile.to_string()),
            network_seed: Some(omp2_rollback_validation::DATA.network_seeds[0]),
            sources: Some(sources),
            prevalidated_tape: true,
            ..Default::default()
        },
    );
    loop {
        if let Some(result) = rollback_lab::step_campaign(&mut campaign, 8, None) {
            return result.clone();
        }
    }
}

#[test]
fn omp_2_crowded_combat_load_fixtures_builds_every_pinned_fixture_at_its_recorded_identity() {
    let tune = Tuning::new();
    let fixtures = omp2_rollback_validation::DATA.combat_load_fixtures;
    assert_eq!(fixtures.len(), 4);
    for fixture in fixtures {
        // combat_load_tape asserts the pinned triple internally, so reaching
        // the assertions below already proves the artifact is unchanged.
        let tape = rollback_validation::combat_load_tape(fixture, &tune);
        assert_eq!(
            tape.frames.len() as i64,
            fixture.frame_count,
            "{} frame count",
            fixture.id
        );
        assert_eq!(tape.identity.fixture, fixture.id);
        assert!((tape.identity.seed - fixture.seed as f64).abs() < f64::EPSILON);
        assert_eq!(
            tape.boundary_hashes[0], fixture.initial_hash,
            "{} initial",
            fixture.id
        );
        assert_eq!(
            tape.boundary_hashes.last().expect("tape has boundaries"),
            fixture.final_hash,
            "{} final",
            fixture.id
        );
        assert_eq!(
            rollback_lab::tape_digest(&tape),
            fixture.tape_digest,
            "{} digest",
            fixture.id
        );
    }
}

#[test]
fn omp_2_crowded_combat_load_fixtures_declares_the_artifact_versions_its_combat_companion_implies()
{
    let tune = Tuning::new();
    for fixture in omp2_rollback_validation::DATA.combat_load_fixtures {
        let identity = rollback_validation::combat_load_tape(fixture, &tune).identity;
        if fixture.combat {
            assert_eq!(
                identity.tape_version,
                input_tape::COMBAT_VERSION,
                "{}",
                fixture.id
            );
            assert_eq!(
                identity.snapshot_version,
                match_snapshot::COMBAT_VERSION,
                "{}",
                fixture.id
            );
            let combat = identity
                .combat
                .as_deref()
                .unwrap_or_else(|| panic!("{} needs a combat identity", fixture.id));
            assert!(!combat.is_empty(), "{} needs a combat identity", fixture.id);
        } else {
            assert_eq!(identity.tape_version, input_tape::VERSION, "{}", fixture.id);
            assert_eq!(
                identity.snapshot_version,
                match_snapshot::VERSION,
                "{}",
                fixture.id
            );
            assert!(
                identity.combat.is_none(),
                "{} must not carry a combat identity",
                fixture.id
            );
        }
    }
}

// The twin only attributes cost to combat if it is the *same* workload. Comparing
// seeds is not enough: a diverged input plan would still share a seed and would
// quietly turn the paired measurement into a comparison of two different matches.
#[test]
fn omp_2_crowded_combat_load_fixtures_pairs_each_fixture_with_a_byte_identical_combat_disabled_twin()
 {
    let tune = Tuning::new();
    for scenario in ["combat_crowded", "combat_repeated_family"] {
        let active = fixture_for(scenario);
        let twin = fixture_for(&format!("{scenario}_disabled"));
        assert_eq!(twin.seed, active.seed, "{scenario} twin seed");
        assert_eq!(
            twin.frame_count, active.frame_count,
            "{scenario} twin length"
        );
        assert_eq!(twin.layout, active.layout, "{scenario} twin layout");
        assert!(!twin.combat, "{scenario} twin must disable combat");
        assert!(active.combat, "{scenario} must enable combat");

        let active_tape = rollback_validation::combat_load_tape(active, &tune);
        let twin_tape = rollback_validation::combat_load_tape(twin, &tune);
        assert_eq!(twin_tape.frames.len(), active_tape.frames.len());
        for index in 0..active_tape.frames.len() {
            assert_eq!(
                input_frame::encode(&twin_tape.frames[index]).expect("twin frame encodes"),
                input_frame::encode(&active_tape.frames[index]).expect("active frame encodes"),
                "{scenario} twin frame {index}"
            );
        }
    }
}

#[test]
fn omp_2_crowded_combat_load_fixtures_drives_all_four_action_families_in_the_crowded_fixture() {
    let tune = Tuning::new();
    let tape = tape_for("combat_crowded", &tune);
    let (events, combat_state) = replay_events(&tape, &tune);
    let mut seen = [false; 4];
    for runtime in &combat_state.players {
        if let Some(family_id) = runtime.family_id {
            seen[match family_id {
                ActionFamilyId::Unarmed => 0,
                ActionFamilyId::Guard => 1,
                ActionFamilyId::LightMelee => 2,
                ActionFamilyId::Ranged => 3,
            }] = true;
        }
    }
    for (index, family) in [
        ActionFamilyId::Unarmed,
        ActionFamilyId::Guard,
        ActionFamilyId::LightMelee,
        ActionFamilyId::Ranged,
    ]
    .iter()
    .enumerate()
    {
        assert!(seen[index], "crowded fixture is missing family {family:?}");
    }
    // A crowd that only commits proves nothing about crowded combat. Require the
    // resolution paths a lone attacker never reaches: a blocked hit, the recoil it
    // produces, an unguarded hit, the forced state it inflicts, and a projectile.
    assert!(
        event_count(&events, CombatEventKind::Commit, None) > 0,
        "crowded fixture never produced commit"
    );
    assert!(
        event_count(
            &events,
            CombatEventKind::Contact,
            Some(CombatContactResult::Guarded)
        ) > 0,
        "crowded fixture never produced contact/guarded"
    );
    assert!(
        event_count(
            &events,
            CombatEventKind::GuardRecoil,
            Some(CombatContactResult::Guarded)
        ) > 0,
        "crowded fixture never produced guard_recoil/guarded"
    );
    assert!(
        event_count(
            &events,
            CombatEventKind::Contact,
            Some(CombatContactResult::Hit)
        ) > 0,
        "crowded fixture never produced contact/hit"
    );
    assert!(
        event_count(
            &events,
            CombatEventKind::Forced,
            Some(CombatContactResult::Hit)
        ) > 0,
        "crowded fixture never produced forced/hit"
    );
    assert!(
        event_count(&events, CombatEventKind::ProjectileSpawn, None) > 0,
        "crowded fixture never produced projectile_spawn"
    );
}

#[test]
fn omp_2_crowded_combat_load_fixtures_puts_every_outfielder_on_one_family_in_the_repeated_family_fixture()
 {
    let tune = Tuning::new();
    let fixture = fixture_for("combat_repeated_family");
    let repeated_loadout_id = fixture
        .repeated_loadout_id
        .expect("repeated-family fixture names a loadout");
    let expected = loadouts::get(repeated_loadout_id)
        .expect("repeated loadout is authored")
        .family_id;
    let tape = tape_for("combat_repeated_family", &tune);
    let (events, combat_state) = replay_events(&tape, &tune);
    let mut outfielders = 0;
    for runtime in &combat_state.players {
        if let Some(family_id) = runtime.family_id {
            outfielders += 1;
            assert_eq!(
                family_id, expected,
                "repeated-family fixture mixed a family in"
            );
        }
    }
    assert_eq!(outfielders, input_frame::SLOT_COUNT);
    assert!(
        event_count(
            &events,
            CombatEventKind::Contact,
            Some(CombatContactResult::Hit)
        ) > 0,
        "repeated-family fixture landed no contact"
    );
}

#[test]
fn omp_2_crowded_combat_load_fixtures_keeps_the_combat_disabled_twins_free_of_combat_entirely() {
    let tune = Tuning::new();
    for scenario in ["combat_crowded_disabled", "combat_repeated_family_disabled"] {
        let tape = tape_for(scenario, &tune);
        let (state, combat_state) = match_snapshot::restore(&tape.initial);
        assert!(
            combat_state.is_none(),
            "{scenario} restored a combat companion"
        );
        assert_eq!(tape.initial.version, match_snapshot::VERSION, "{scenario}");
        assert!(!combat::blocks_actions(None, 1));
        assert_eq!(state.players.len(), 10);
    }
}

// The regression guard that matters most for these fixtures. `budgets.snapshot_bytes`
// has roughly nine kilobytes left across the whole 31-boundary window once a
// ten-player combat match is in it, so a few hundred extra bytes per snapshot
// anywhere in CombatMatchState pushes the real campaign over the gate. Failing here
// says so in seconds instead of in CI.
#[test]
fn omp_2_crowded_combat_load_fixtures_converges_inside_the_pinned_snapshot_and_history_budgets() {
    let tune = Tuning::new();
    let budgets = omp2_rollback_validation::DATA.budgets;
    for fixture in omp2_rollback_validation::DATA.combat_load_fixtures {
        let result = lab_result(fixture.scenario, &tune);
        let peaks = &result.metrics.peaks;
        assert!(result.success, "{} did not converge", fixture.id);
        assert_eq!(
            result.status,
            RollbackLabStatus::Converged,
            "{}",
            fixture.id
        );
        assert_eq!(result.input_ticks, fixture.frame_count, "{}", fixture.id);
        assert_eq!(
            result.reference_final_hash, fixture.final_hash,
            "{}",
            fixture.id
        );
        assert_eq!(
            result.client_final_hash, result.reference_final_hash,
            "{}",
            fixture.id
        );
        assert!(
            peaks.snapshot_count <= budgets.snapshot_count,
            "{} exceeded the snapshot-count budget",
            fixture.id
        );
        assert!(
            peaks.snapshot_bytes < budgets.snapshot_bytes,
            "{} peaked at {} snapshot bytes, budget is {}",
            fixture.id,
            peaks.snapshot_bytes,
            budgets.snapshot_bytes
        );
        assert!(
            peaks.history_bytes < budgets.history_bytes,
            "{} peaked at {} history bytes, budget is {}",
            fixture.id,
            peaks.history_bytes,
            budgets.history_bytes
        );
        let confirmed = result.event_metrics.confirmed_combat_events;
        if fixture.combat {
            assert!(confirmed > 0, "{} confirmed no combat event", fixture.id);
        } else {
            assert_eq!(
                confirmed, 0,
                "{} confirmed a combat event with combat off",
                fixture.id
            );
        }
    }
}

// A healthy campaign only ever reaches the two passing corners of this truth table,
// so the rejecting corners are unreachable from a normal run. They still decide
// `scenario_pass`, which becomes the `success=` marker CI enforces, so assert all
// four directly. The active/zero corner is the one that matters most: an operator
// precedence slip there let a fixture that measured no combat at all pass as
// covered, which is precisely the regression this fixture pair exists to catch.
#[test]
fn omp_2_crowded_combat_load_fixtures_rejects_a_case_whose_combat_presence_contradicts_its_fixture()
{
    let tune = Tuning::new();
    for scenario in ["combat_crowded", "combat_crowded_disabled"] {
        let mut result = lab_result(scenario, &tune);
        let active = fixture_for(scenario).combat;
        let observed = result.event_metrics.confirmed_combat_events;

        assert!(
            rollback_validation::combat_load_covered_for(&result, scenario),
            "{scenario} should cover itself as measured"
        );
        assert_eq!(
            observed > 0,
            active,
            "{scenario} measured the wrong combat presence"
        );

        result.event_metrics.confirmed_combat_events = 0;
        assert_eq!(
            rollback_validation::combat_load_covered_for(&result, scenario),
            !active,
            "{scenario} mishandled zero confirmed combat events"
        );

        result.event_metrics.confirmed_combat_events = 1;
        assert_eq!(
            rollback_validation::combat_load_covered_for(&result, scenario),
            active,
            "{scenario} mishandled a non-zero confirmed combat event count"
        );
    }
}
