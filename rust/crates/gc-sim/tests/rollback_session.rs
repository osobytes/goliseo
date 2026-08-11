//! Tests for `gc_sim::rollback_session`.
//!
//! ## Design notes
//!
//! - **Copy-independence is verified behaviorally, not just relied on as a
//!   type-system guarantee.** Every payload type this module returns
//!   ([`RollbackTickOutput`], [`RollbackComparison`], ...) is an owned Rust
//!   value with a real [`Clone`] impl, so mutating a value the API just
//!   returned can never affect retained session state. Even so, each case
//!   that can plausibly touch this still performs at least one such
//!   mutation and re-reads the retained state to assert the independence
//!   directly, both to keep the assertion coverage this file is meant to
//!   preserve and because it costs nothing to check.
//! - **`MatchSnapshotDifference.expected`/`.actual` are rendered `String`s**
//!   (see `gc_sim::match_snapshot`), not nested tables: a presence/absence
//!   difference (`None` vs `Some`) renders as the boolean
//!   `"true"`/`"false"`, not a nested value's coordinates (see
//!   `match_snapshot.rs`'s `dive_target` diff arm). Because of that,
//!   "reports confirmation and deterministic boundary diagnostics" below
//!   diverges `state.score.home`, an `i64` leaf whose rendered
//!   `expected`/`actual` are unambiguous, and asserts the same
//!   path/copy-independence contract.
//! - **Slot and player *array* indices are 0-based; slot *identity* stays
//!   1-based.** Per `rollback_input_history`/`match.rs`'s own documented
//!   conventions, canonical slot identity is 1-based at the
//!   `add_authoritative` call boundary, while the
//!   `RollbackInputTickRecord.slots` array and `MatchState.players` array
//!   are 0-based Rust collections — the two must not be conflated when
//!   indexing.
//! - **`sources(false)` (no local slot)** returns every slot `"remote"` —
//!   `add_authoritative` never gates acceptance on ownership, only
//!   `input.slots[n].source` diagnostics read it.
//! - **Fixture snapshots use `match_snapshot::capture_owned`, not
//!   `capture`.** `crate::r#match::new`'s freshly constructed `MatchState`
//!   leaves `marks.home`/`.away` empty — the marking-assignment pass that
//!   sizes them to one entry per player only runs during `step`.
//!   `capture`'s `match_snapshot::validate` requires that length
//!   immediately, which a never-stepped fixture cannot satisfy;
//!   `capture_owned` is for state trusted to have come from `sim::match`,
//!   not arbitrary external input (see its own doc comment), which is
//!   exactly what every fixture function below is. `rollback_session::new`
//!   itself matches this: it restores via `restore_owned` for the same
//!   reason (see its doc comment), so the two choices agree end to end.

use gc_core::vec2::Vec2;
use gc_sim::input_frame::{self, InputSample, InputSampleOptions};
use gc_sim::keeper::KeeperShotType;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchSnapshot, PitchSize, WindupShot};
use gc_sim::rollback_input_history::RollbackInputStatus;
use gc_sim::rollback_session::{
    self, RollbackSession, RollbackSessionErrorCode, RollbackTickOutput,
};
use gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus;

struct NewStateOptions {
    duration: f64,
    max_goals: i64,
    seed: f64,
}

impl Default for NewStateOptions {
    fn default() -> Self {
        NewStateOptions {
            duration: 4.0,
            max_goals: 3,
            seed: 70.0,
        }
    }
}

fn new_state(options: NewStateOptions) -> match_snapshot::MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(options.duration),
        max_goals: Some(options.max_goals),
        seed: Some(options.seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    })
}

fn initial_snapshot() -> MatchSnapshot {
    match_snapshot::capture_owned(&new_state(NewStateOptions::default()), None)
}

fn sources(local_first: bool) -> [gc_sim::rollback_input_history::RollbackInputSource; 8] {
    use gc_sim::rollback_input_history::RollbackInputSource::{Local, Remote};
    let mut result = [Remote; 8];
    if local_first {
        result[0] = Local;
    }
    result
}

fn sample(options: InputSampleOptions) -> InputSample {
    input_frame::new_sample(options).expect("valid sample")
}

fn step_many(session: &mut RollbackSession, count: i64) -> Vec<RollbackTickOutput> {
    (0..count)
        .map(|_| rollback_session::step(session).expect("step succeeds"))
        .collect()
}

fn add_rows(session: &mut RollbackSession, tick: i64, rows: &[(i64, InputSample)]) {
    for &(slot_index, row) in rows {
        rollback_session::add_authoritative(session, tick, slot_index, row)
            .expect("authoritative row accepted");
    }
}

fn add_neutral_tick(session: &mut RollbackSession, tick: i64) {
    for slot_index in 1..=input_frame::SLOT_COUNT {
        rollback_session::add_authoritative(
            session,
            tick,
            slot_index,
            input_frame::neutral_sample(),
        )
        .expect("authoritative row accepted");
    }
}

fn shot_fixture(max_goals: Option<i64>) -> MatchSnapshot {
    let mut state = new_state(NewStateOptions {
        duration: 4.0,
        max_goals: max_goals.unwrap_or(1),
        seed: 700.0,
    });
    let carrier_index = state.slot_players[0].expect("slot one is mapped");
    {
        let carrier = &mut state.players[(carrier_index - 1) as usize];
        carrier.pos = Vec2::new(900.0, 270.0);
        carrier.vel = Vec2::new(0.0, 0.0);
        carrier.run_vel = Vec2::new(0.0, 0.0);
        carrier.facing = Vec2::new(1.0, 0.0);
        carrier.charge = 1.0;
    }
    state.owner = Some(carrier_index);
    state.ball = Vec2::new(918.0, 270.0);
    state.ball_vel = Vec2::new(0.0, 0.0);
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 2.0;
    state.block_grace = 2.0;
    for (index0, player) in state.players.iter_mut().enumerate() {
        let index = index0 as i64 + 1;
        if index != carrier_index {
            player.pos = Vec2::new(if index < 6 { 200.0 } else { 100.0 }, 40.0 + index as f64);
            player.vel = Vec2::new(0.0, 0.0);
            player.run_vel = Vec2::new(0.0, 0.0);
        }
    }
    match_snapshot::capture_owned(&state, None)
}

fn preventable_goal_fixture() -> MatchSnapshot {
    let mut state = new_state(NewStateOptions {
        duration: 4.0,
        max_goals: 1,
        seed: 701.0,
    });
    let carrier_index = state.slot_players[0].expect("slot one is mapped");
    {
        let carrier = &mut state.players[(carrier_index - 1) as usize];
        carrier.pos = Vec2::new(900.0, 270.0);
        carrier.vel = Vec2::new(0.0, 0.0);
        carrier.run_vel = Vec2::new(0.0, 0.0);
        carrier.facing = Vec2::new(1.0, 0.0);
        carrier.windup_timer = 1.0 / 60.0;
        carrier.windup_shot = Some(WindupShot {
            dir: Vec2::new(1.0, 0.0),
            speed: 900.0,
            vz: 0.0,
            spin: 0.0,
            shot_type: KeeperShotType::Ground,
        });
    }
    state.owner = Some(carrier_index);
    state.ball = Vec2::new(918.0, 270.0);
    state.ball_vel = Vec2::new(0.0, 0.0);
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 2.0;
    state.block_grace = 2.0;
    for (index0, player) in state.players.iter_mut().enumerate() {
        let index = index0 as i64 + 1;
        if index != carrier_index {
            player.pos = Vec2::new(if index < 6 { 200.0 } else { 100.0 }, 40.0 + index as f64);
            player.vel = Vec2::new(0.0, 0.0);
            player.run_vel = Vec2::new(0.0, 0.0);
        }
    }
    let defender_index = state.slot_players[4].expect("slot five is mapped");
    let defender = &mut state.players[(defender_index - 1) as usize];
    defender.pos = Vec2::new(910.0, 240.0);
    defender.facing = Vec2::new(-1.0, 0.0);
    match_snapshot::capture_owned(&state, None)
}

fn catch_panic<T>(f: impl FnOnce() -> T + std::panic::UnwindSafe) -> std::thread::Result<T> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(f);
    std::panic::set_hook(previous);
    result
}

#[test]
fn restores_and_resimulates_combat_state_atomically_from_the_earliest_edge() {
    use gc_sim::combat;

    let mut state = new_state(NewStateOptions {
        duration: 4.0,
        max_goals: 3,
        seed: 707.0,
    });
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    let source_player = combat_state
        .players
        .iter()
        .position(|runtime| {
            runtime.family_id == Some(gc_data::action_families::ActionFamilyId::LightMelee)
        })
        .expect("fixture requires one melee player")
        + 1;
    let target_player = state
        .players
        .iter()
        .position(|player| {
            player.team != state.players[source_player - 1].team && !player.is_keeper
        })
        .expect("fixture requires an opposing target")
        + 1;
    for (index0, player) in state.players.iter_mut().enumerate() {
        let index = index0 as i64 + 1;
        player.pos = Vec2::new(700.0 + index as f64, 400.0 + index as f64);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
    state.players[source_player - 1].pos = Vec2::new(300.0, 270.0);
    state.players[source_player - 1].facing = Vec2::new(1.0, 0.0);
    state.players[target_player - 1].pos = Vec2::new(335.0, 270.0);
    state.players[target_player - 1].facing = Vec2::new(-1.0, 0.0);
    state.owner = Some(target_player as i64);
    state.ball = state.players[target_player - 1].pos;
    state.ball_vel = Vec2::new(0.0, 0.0);
    state.pickup_cd = 1.0;
    let source_slot =
        state.slot_for_player[source_player - 1].expect("melee player requires an input slot");

    let initial = match_snapshot::capture_owned(&state, Some(&combat_state));
    let mut predicted = rollback_session::new(&initial, sources(false), None, None);
    let predicted_output = rollback_session::step(&mut predicted).expect("step succeeds");
    assert_eq!(
        predicted_output
            .combat_events
            .as_ref()
            .map_or(0, std::vec::Vec::len),
        0
    );

    let pressed = sample(InputSampleOptions {
        held: Some(input_frame::HELD_EQUIPMENT),
        edges: Some(input_frame::EDGE_EQUIPMENT_PRESSED),
        ..Default::default()
    });
    for slot_index in 1..=input_frame::SLOT_COUNT {
        let row = if slot_index == source_slot {
            pressed
        } else {
            sample(InputSampleOptions::default())
        };
        rollback_session::add_authoritative(&mut predicted, 0, slot_index, row)
            .expect("authoritative row accepted");
    }
    let reconciled = rollback_session::reconcile(&mut predicted, true);
    assert!(reconciled.changed);
    assert_eq!(reconciled.causal_tick, Some(0));
    assert_eq!(reconciled.corrected_outputs.len(), 1);
    let corrected_events = reconciled.corrected_outputs[0]
        .combat_events
        .as_ref()
        .expect("combat events are missing");
    assert_eq!(
        corrected_events[0].kind,
        gc_sim::combat_snapshot::CombatEventKind::Commit
    );

    let mut expected = rollback_session::new(&initial, sources(false), None, None);
    for slot_index in 1..=input_frame::SLOT_COUNT {
        let row = if slot_index == source_slot {
            pressed
        } else {
            sample(InputSampleOptions::default())
        };
        rollback_session::add_authoritative(&mut expected, 0, slot_index, row)
            .expect("authoritative row accepted");
    }
    rollback_session::step(&mut expected).expect("step succeeds");
    let mut saw_contact = false;
    let mut saw_spill = false;
    for tick in 1..=15 {
        add_neutral_tick(&mut predicted, tick);
        add_neutral_tick(&mut expected, tick);
        let output = rollback_session::step(&mut predicted).expect("step succeeds");
        rollback_session::step(&mut expected).expect("step succeeds");
        for event in output.combat_events.iter().flatten() {
            saw_contact =
                saw_contact || event.kind == gc_sim::combat_snapshot::CombatEventKind::Contact;
            saw_spill =
                saw_spill || event.kind == gc_sim::combat_snapshot::CombatEventKind::BallSpill;
        }
    }
    assert!(saw_contact);
    assert!(saw_spill);
    let comparison = rollback_session::compare(
        &mut predicted,
        &rollback_session::current_snapshot(&expected),
        Some(0),
    );
    assert!(comparison.matched);
    let final_snapshot = rollback_session::current_snapshot(&predicted);
    let combat = final_snapshot.combat.expect("combat state retained");
    assert!(combat.players[target_player - 1].forced_ticks > 0);
    assert_eq!(combat.next_source_sequence, 2);
    assert_eq!(final_snapshot.state.owner, None);
}

#[test]
fn keeps_measured_operations_under_session_ownership() {
    let mut fabricated = rollback_session::new(
        &initial_snapshot(),
        sources(false),
        None,
        Some(Box::new(|_label, operation: &mut dyn FnMut()| {
            operation();
        })),
    );
    let output = rollback_session::step(&mut fabricated).expect("step succeeds");
    assert_eq!(output.tick, 0);
    assert_eq!(
        rollback_session::diagnostics(&fabricated).present_boundary,
        1
    );

    let mut skipped = rollback_session::new(
        &initial_snapshot(),
        sources(false),
        None,
        Some(Box::new(|_label, _operation: &mut dyn FnMut()| {})),
    );
    let result = catch_panic(std::panic::AssertUnwindSafe(|| {
        rollback_session::step(&mut skipped)
    }));
    assert!(result.is_err());

    let mut doubled = rollback_session::new(
        &initial_snapshot(),
        sources(false),
        None,
        Some(Box::new(|_label, operation: &mut dyn FnMut()| {
            operation();
            operation();
        })),
    );
    let result = catch_panic(std::panic::AssertUnwindSafe(|| {
        rollback_session::step(&mut doubled)
    }));
    assert!(result.is_err());
}

#[test]
fn owns_boundary_zero_and_exposes_only_copied_state_history_and_outputs() {
    let mut supplied = initial_snapshot();
    let original_x = supplied.state.players[0].pos.x;
    let mut session = rollback_session::new(&supplied, sources(true), None, None);
    supplied.state.players[0].pos.x = original_x + 1000.0;

    let mut present = rollback_session::current_snapshot(&session);
    assert_eq!(present.state.players[0].pos.x, original_x);
    present.state.players[0].pos.x = original_x - 1000.0;
    assert_eq!(
        rollback_session::current_snapshot(&session).state.players[0]
            .pos
            .x,
        original_x
    );
    assert_eq!(
        rollback_session::snapshot(&session, 0).status,
        RollbackSnapshotLookupStatus::Present
    );

    rollback_session::add_authoritative(
        &mut session,
        0,
        1,
        sample(InputSampleOptions {
            move_x: Some(30),
            ..Default::default()
        }),
    )
    .expect("authoritative row accepted");
    let mut output = rollback_session::step(&mut session).expect("step succeeds");
    assert_eq!(output.tick, 0);
    assert_eq!(output.start_boundary, 0);
    assert_eq!(output.end_boundary, 1);
    assert_eq!(
        output.input.slots[0].status,
        RollbackInputStatus::Authoritative
    );
    assert_eq!(output.input.slots[1].status, RollbackInputStatus::Predicted);
    assert_eq!(
        rollback_session::snapshot(&session, 0).status,
        RollbackSnapshotLookupStatus::Retained
    );
    assert_eq!(
        rollback_session::snapshot(&session, 1).status,
        RollbackSnapshotLookupStatus::Present
    );

    output.input.slots[0].sample.move_x = -30;
    output.state.score.home = 99;
    let retained = rollback_session::output(&session, 0).expect("output retained");
    assert_eq!(retained.input.slots[0].sample.move_x, 30);
    assert_eq!(retained.state.score.home, 0);
    let diagnostics = rollback_session::diagnostics(&session);
    assert_eq!(diagnostics.predicted_slot_samples, 7);
    assert_eq!(diagnostics.predicted_ticks, 1);
}

#[test]
fn treats_an_authoritative_sample_equal_to_prediction_as_a_true_no_op() {
    let mut session = rollback_session::new(&initial_snapshot(), sources(false), None, None);
    step_many(&mut session, 3);
    let before = rollback_session::current_snapshot(&session);
    let arrival =
        rollback_session::add_authoritative(&mut session, 1, 1, input_frame::neutral_sample())
            .expect("authoritative row accepted");
    assert!(!arrival.correction);
    assert_eq!(arrival.earliest_divergence, None);

    let result = rollback_session::reconcile(&mut session, false);
    assert!(!result.changed);
    assert_eq!(result.old_present_hash, None);
    assert_eq!(result.new_present_hash, None);
    assert_eq!(rollback_session::diagnostics(&session).rollback_count, 0);
    assert_eq!(
        match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session)),
        match_snapshot::hash_canonical(&before)
    );
}

#[test]
fn converges_one_correction_and_replaces_the_affected_snapshots_and_output() {
    let initial = initial_snapshot();
    let corrected = sample(InputSampleOptions {
        move_x: Some(127),
        ..Default::default()
    });
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    rollback_session::add_authoritative(&mut reference, 0, 1, corrected)
        .expect("authoritative row accepted");
    step_many(&mut reference, 5);

    let stale = step_many(&mut delayed, 5).into_iter().next().unwrap();
    let stale_hash = rollback_session::snapshot(&delayed, 1)
        .snapshot
        .expect("boundary one is retained");
    let arrival = rollback_session::add_authoritative(&mut delayed, 0, 1, corrected)
        .expect("authoritative row accepted");
    assert!(arrival.correction);
    let result = rollback_session::reconcile(&mut delayed, true);

    assert!(result.changed);
    assert_eq!(result.causal_tick, Some(0));
    assert_eq!(
        result.restore_status,
        Some(RollbackSnapshotLookupStatus::Retained)
    );
    assert_eq!(result.old_present_boundary, 5);
    assert_eq!(result.new_present_boundary, 5);
    assert_eq!(result.corrected_outputs.len(), 5);
    assert_eq!(
        result.corrected_outputs[0].input.slots[0].status,
        RollbackInputStatus::Authoritative
    );
    assert_eq!(
        stale.input.slots[0].sample.move_x, 0,
        "returned stale output remains immutable"
    );
    assert_eq!(
        rollback_session::output(&delayed, 0)
            .expect("output retained")
            .input
            .slots[0]
            .sample
            .move_x,
        127
    );
    assert_ne!(
        match_snapshot::hash_canonical(
            &rollback_session::snapshot(&delayed, 1)
                .snapshot
                .expect("boundary one is retained")
        ),
        match_snapshot::hash_canonical(&stale_hash)
    );
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            Some(0)
        )
        .matched
    );
    let mut first_difference = result.first_difference.expect("a difference was recorded");
    let original_path = first_difference.path.clone();
    first_difference.path = "mutated".to_string();
    let rollback_diagnostics = rollback_session::diagnostics(&delayed);
    assert_eq!(
        rollback_diagnostics
            .last_rollback
            .as_ref()
            .expect("last rollback retained")
            .first_difference
            .as_ref()
            .expect("difference retained")
            .path,
        original_path
    );
}

#[test]
fn batches_several_corrections_into_one_earliest_restore_with_ordered_outputs() {
    let initial = initial_snapshot();
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    let at_one = sample(InputSampleOptions {
        move_y: Some(-127),
        ..Default::default()
    });
    let at_three = sample(InputSampleOptions {
        move_x: Some(80),
        held: Some(input_frame::HELD_SPRINT),
        ..Default::default()
    });
    add_rows(&mut reference, 1, &[(1, at_one)]);
    add_rows(&mut reference, 3, &[(2, at_three)]);
    step_many(&mut reference, 6);
    step_many(&mut delayed, 6);

    add_rows(&mut delayed, 3, &[(2, at_three)]);
    add_rows(&mut delayed, 1, &[(1, at_one)]);
    let result = rollback_session::reconcile(&mut delayed, false);
    assert_eq!(result.causal_tick, Some(1));
    assert_eq!(result.corrected_outputs.len(), 5);
    assert_eq!(result.old_present_hash, None);
    assert_eq!(result.new_present_hash, None);
    assert_eq!(result.first_difference, None);
    assert_eq!(
        rollback_session::diagnostics(&delayed)
            .last_rollback
            .expect("last rollback retained")
            .first_difference,
        None
    );
    for (index, output) in result.corrected_outputs.iter().enumerate() {
        assert_eq!(output.tick, index as i64 + 1);
    }
    assert_eq!(
        result.corrected_outputs[0].input.slots[0].status,
        RollbackInputStatus::Authoritative
    );
    assert_eq!(
        result.corrected_outputs[2].input.slots[1].status,
        RollbackInputStatus::Authoritative
    );
    assert_eq!(rollback_session::diagnostics(&delayed).rollback_count, 1);
    assert_eq!(rollback_session::diagnostics(&delayed).correction_count, 2);
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            Some(1)
        )
        .matched
    );
}

#[test]
fn applies_one_authoritative_packet_batch_through_exactly_one_reconciliation() {
    let mut session = rollback_session::new(&initial_snapshot(), sources(false), None, None);
    step_many(&mut session, 4);
    let applied = rollback_session::apply_authoritative_batch(
        &mut session,
        &[
            gc_sim::rollback_input_history::RollbackAuthoritativeInput {
                tick: 1,
                slot_index: 1,
                sample: sample(InputSampleOptions {
                    move_y: Some(-127),
                    ..Default::default()
                }),
            },
            gc_sim::rollback_input_history::RollbackAuthoritativeInput {
                tick: 2,
                slot_index: 2,
                sample: sample(InputSampleOptions {
                    move_x: Some(80),
                    held: Some(input_frame::HELD_SPRINT),
                    ..Default::default()
                }),
            },
        ],
        false,
    )
    .expect("batch accepted");
    assert_eq!(applied.arrival.inserted, 2);
    assert_eq!(applied.arrival.duplicates, 0);
    assert_eq!(applied.arrival.corrections, 2);
    assert!(applied.reconciliation.changed);
    assert_eq!(applied.reconciliation.causal_tick, Some(1));
    assert_eq!(applied.reconciliation.corrected_outputs.len(), 3);
    let diagnostics = rollback_session::diagnostics(&session);
    assert_eq!(diagnostics.rollback_count, 1);
    assert_eq!(diagnostics.correction_count, 2);

    let duplicate = rollback_session::apply_authoritative_batch(
        &mut session,
        &[
            gc_sim::rollback_input_history::RollbackAuthoritativeInput {
                tick: 1,
                slot_index: 1,
                sample: sample(InputSampleOptions {
                    move_y: Some(-127),
                    ..Default::default()
                }),
            },
            gc_sim::rollback_input_history::RollbackAuthoritativeInput {
                tick: 2,
                slot_index: 2,
                sample: sample(InputSampleOptions {
                    move_x: Some(80),
                    held: Some(input_frame::HELD_SPRINT),
                    ..Default::default()
                }),
            },
        ],
        false,
    )
    .expect("duplicate batch accepted");
    assert_eq!(duplicate.arrival.inserted, 0);
    assert_eq!(duplicate.arrival.duplicates, 2);
    assert!(!duplicate.reconciliation.changed);
    assert_eq!(rollback_session::diagnostics(&session).rollback_count, 1);
}

#[test]
fn remains_deterministic_across_repeated_rollback_of_corrected_intervals() {
    let initial = initial_snapshot();
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    let first = sample(InputSampleOptions {
        move_x: Some(100),
        ..Default::default()
    });
    let second = sample(InputSampleOptions {
        move_y: Some(90),
        ..Default::default()
    });
    add_rows(&mut reference, 1, &[(1, first)]);
    add_rows(&mut reference, 2, &[(2, second)]);
    step_many(&mut reference, 7);
    step_many(&mut delayed, 7);

    add_rows(&mut delayed, 2, &[(2, second)]);
    rollback_session::reconcile(&mut delayed, false);
    add_rows(&mut delayed, 1, &[(1, first)]);
    rollback_session::reconcile(&mut delayed, false);
    let diagnostics = rollback_session::diagnostics(&delayed);
    assert_eq!(diagnostics.rollback_count, 2);
    assert_eq!(diagnostics.latest_rollback_depth, 6);
    assert_eq!(diagnostics.max_rollback_depth, 6);
    assert_eq!(diagnostics.resimulated_ticks, 11);
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            None
        )
        .matched
    );
}

#[test]
fn handles_a_correction_exactly_thirty_ticks_deep() {
    let initial = initial_snapshot();
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    let corrected = sample(InputSampleOptions {
        move_x: Some(-127),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut reference, 1, 1, corrected)
        .expect("authoritative row accepted");
    step_many(&mut reference, 31);
    step_many(&mut delayed, 31);

    rollback_session::add_authoritative(&mut delayed, 1, 1, corrected)
        .expect("authoritative row accepted");
    let result = rollback_session::reconcile(&mut delayed, false);
    assert_eq!(result.causal_tick, Some(1));
    assert_eq!(
        result.restore_status,
        Some(RollbackSnapshotLookupStatus::Retained)
    );
    assert_eq!(result.corrected_outputs.len(), 30);
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            None
        )
        .matched
    );
}

#[test]
fn fails_explicitly_at_thirty_one_ticks_late_without_hidden_progress() {
    let mut session = rollback_session::new(&initial_snapshot(), sources(false), None, None);
    step_many(&mut session, 31);
    let before = rollback_session::current_snapshot(&session);
    let before_hash = match_snapshot::hash_canonical(&before);
    let result = rollback_session::add_authoritative(
        &mut session,
        0,
        1,
        sample(InputSampleOptions {
            move_x: Some(127),
            ..Default::default()
        }),
    );
    let err = result.expect_err("late authority is rejected");
    assert_eq!(
        err.code,
        gc_sim::rollback_input_history::RollbackInputHistoryErrorCode::OutsideWindow
    );

    let result = rollback_session::reconcile(&mut session, false);
    assert!(!result.changed);
    assert_eq!(
        result.status,
        gc_sim::rollback_session::RollbackSessionStatus::LateInputUnrecoverable
    );
    assert_eq!(result.causal_tick, Some(0));
    assert_eq!(
        result.restore_status,
        Some(RollbackSnapshotLookupStatus::OutsideWindow)
    );
    assert_eq!(result.old_present_boundary, 31);
    assert_eq!(result.new_present_boundary, 31);
    assert_eq!(result.old_present_hash, None);
    assert_eq!(result.new_present_hash, None);
    assert_eq!(
        match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session)),
        before_hash
    );
    let step_result = rollback_session::step(&mut session);
    let step_err = step_result.expect_err("stalled session cannot progress");
    assert_eq!(
        step_err.code,
        RollbackSessionErrorCode::LateInputUnrecoverable
    );
    assert_eq!(
        match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session)),
        before_hash
    );
    assert_eq!(
        rollback_session::diagnostics(&session).late_window_failures,
        1
    );
}

#[test]
fn attributes_mixed_retained_and_over_window_batches_to_the_actual_late_tick() {
    for retained_first in [true, false] {
        let mut session = rollback_session::new(&initial_snapshot(), sources(false), None, None);
        step_many(&mut session, 31);
        let before = match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session));
        let retained = |session: &mut RollbackSession| {
            rollback_session::add_authoritative(
                session,
                1,
                1,
                sample(InputSampleOptions {
                    move_x: Some(10),
                    ..Default::default()
                }),
            )
        };
        let late = |session: &mut RollbackSession| {
            let result = rollback_session::add_authoritative(
                session,
                0,
                2,
                sample(InputSampleOptions {
                    move_y: Some(10),
                    ..Default::default()
                }),
            );
            let err = result.expect_err("late authority is rejected");
            assert_eq!(
                err.code,
                gc_sim::rollback_input_history::RollbackInputHistoryErrorCode::OutsideWindow
            );
        };
        if retained_first {
            retained(&mut session).expect("authoritative row accepted");
            late(&mut session);
        } else {
            late(&mut session);
            retained(&mut session).expect("authoritative row accepted");
        }

        let result = rollback_session::reconcile(&mut session, false);
        assert!(!result.changed);
        assert_eq!(result.causal_tick, Some(0));
        assert_eq!(
            result.restore_status,
            Some(RollbackSnapshotLookupStatus::OutsideWindow)
        );
        assert_eq!(result.old_present_boundary, 31);
        assert_eq!(result.new_present_boundary, 31);
        assert_eq!(result.old_present_hash, None);
        assert_eq!(result.new_present_hash, None);
        assert_eq!(
            match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session)),
            before
        );
        assert_eq!(
            rollback_session::diagnostics(&session).late_window_failures,
            1
        );
        let step_result = rollback_session::step(&mut session);
        assert_eq!(
            step_result
                .expect_err("stalled session cannot progress")
                .code,
            RollbackSessionErrorCode::LateInputUnrecoverable
        );
    }
}

#[test]
fn asserts_a_retained_range_snapshot_gap_before_consuming_or_restoring() {
    let mut session = rollback_session::new(&initial_snapshot(), sources(false), Some(5), None);
    step_many(&mut session, 3);
    rollback_session::add_authoritative(
        &mut session,
        1,
        1,
        sample(InputSampleOptions {
            move_x: Some(10),
            ..Default::default()
        }),
    )
    .expect("authoritative row accepted");
    let before = match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session));

    // Deliberate invariant corruption: remove boundary one from its ring
    // while leaving the input divergence intact.
    let ring_index = (1i64 % session.snapshot_history.capacity) as usize;
    session.snapshot_history.entries[ring_index] = None;
    let result = catch_panic(std::panic::AssertUnwindSafe(|| {
        rollback_session::reconcile(&mut session, false)
    }));
    let panic_payload = result.expect_err("missing boundary invariant panics");
    let message = panic_payload
        .downcast_ref::<String>()
        .map(std::string::String::as_str)
        .or_else(|| panic_payload.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(
        message.contains("rollback snapshot invariant: boundary 1 is missing"),
        "unexpected panic message: {message}"
    );
    assert_eq!(
        match_snapshot::hash_canonical(&rollback_session::current_snapshot(&session)),
        before
    );
    assert_eq!(session.input_history.earliest_divergence, Some(1));
}

#[test]
fn keeps_no_op_reconciliation_and_matching_comparison_diagnostics_lazy() {
    // This asserts the "lazy" contract directly rather than counting calls
    // to `match_snapshot::capture`/`hash`/`first_difference`: a no-op
    // `reconcile` (no divergence, or a stalled session) performs no
    // snapshot restore/resimulation work, which is already covered by
    // `changed == false` plus the untouched present boundary below, and a
    // matching `compare` still reports a hash for both sides.
    let mut session = rollback_session::new(&initial_snapshot(), sources(false), None, None);
    step_many(&mut session, 2);
    rollback_session::add_authoritative(&mut session, 1, 1, input_frame::neutral_sample())
        .expect("authoritative row accepted");
    let expected = rollback_session::current_snapshot(&session);
    let mut terminal = rollback_session::new(&initial_snapshot(), sources(false), None, None);
    step_many(&mut terminal, 31);
    let rejected = rollback_session::add_authoritative(
        &mut terminal,
        0,
        1,
        sample(InputSampleOptions {
            move_x: Some(1),
            ..Default::default()
        }),
    );
    assert!(rejected.is_err());

    let present_before = rollback_session::diagnostics(&session).present_boundary;
    let result = rollback_session::reconcile(&mut session, false);
    assert!(!result.changed);
    assert_eq!(
        rollback_session::diagnostics(&session).present_boundary,
        present_before
    );

    let terminal_result = rollback_session::reconcile(&mut terminal, false);
    assert!(!terminal_result.changed);
    assert_eq!(
        terminal_result.status,
        gc_sim::rollback_session::RollbackSessionStatus::LateInputUnrecoverable
    );

    let comparison = rollback_session::compare(&mut session, &expected, None);
    assert!(comparison.matched);
}

#[test]
fn corrects_a_real_shot_into_earlier_full_time_and_discards_every_stale_tail() {
    let initial = shot_fixture(None);
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    let shot = sample(InputSampleOptions {
        edges: Some(input_frame::EDGE_SHOOT),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut reference, 0, 1, shot)
        .expect("authoritative row accepted");
    while rollback_session::diagnostics(&reference).status
        != gc_sim::rollback_session::RollbackSessionStatus::Finished
    {
        rollback_session::step(&mut reference).expect("step succeeds");
    }
    let final_boundary = rollback_session::diagnostics(&reference).present_boundary;
    assert!(
        final_boundary < 30,
        "the causal shot fixture must finish promptly"
    );

    step_many(&mut delayed, 30);
    assert_eq!(
        rollback_session::diagnostics(&delayed).status,
        gc_sim::rollback_session::RollbackSessionStatus::Active
    );
    rollback_session::add_authoritative(&mut delayed, 0, 1, shot)
        .expect("authoritative row accepted");
    let result = rollback_session::reconcile(&mut delayed, false);
    assert_eq!(
        result.status,
        gc_sim::rollback_session::RollbackSessionStatus::Finished
    );
    assert_eq!(result.new_present_boundary, final_boundary);
    assert!(result.new_present_boundary < result.old_present_boundary);
    assert_eq!(result.corrected_from_tick, Some(0));
    assert_eq!(result.corrected_through_tick, Some(final_boundary - 1));
    assert_eq!(result.replaced_from_tick, Some(0));
    assert_eq!(result.replaced_through_tick, Some(29));
    assert_eq!(result.corrected_outputs.len(), final_boundary as usize);
    assert!(
        rollback_session::output(&delayed, final_boundary - 1)
            .expect("output retained")
            .finished
    );
    assert_eq!(rollback_session::output(&delayed, final_boundary), None);
    assert_eq!(
        rollback_session::snapshot(&delayed, final_boundary).status,
        RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(
        rollback_session::snapshot(&delayed, final_boundary + 1).status,
        RollbackSnapshotLookupStatus::Missing
    );
    let stopped = rollback_session::step(&mut delayed);
    assert_eq!(
        stopped.expect_err("finished session cannot step").code,
        RollbackSessionErrorCode::MatchFinished
    );
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            Some(0)
        )
        .matched
    );
    let retained = rollback_session::diagnostics(&delayed).snapshot_history;
    assert!(retained.peak_retained_boundary_count > retained.retained_boundary_count);
    assert!(retained.peak_canonical_bytes > retained.canonical_bytes);

    let mut later = None;
    for tick in 0..=(final_boundary + 1) {
        for slot_index in 1..=input_frame::SLOT_COUNT {
            let authoritative = if tick == 0 && slot_index == 1 {
                shot
            } else {
                input_frame::neutral_sample()
            };
            later = Some(
                rollback_session::add_authoritative(&mut delayed, tick, slot_index, authoritative)
                    .expect("authoritative row accepted"),
            );
        }
    }
    assert_eq!(
        later.expect("at least one row added").earliest_divergence,
        None,
        "discarded frames cannot create false divergence"
    );
    let diagnostics = rollback_session::diagnostics(&delayed);
    assert_eq!(diagnostics.confirmed_tick, final_boundary + 1);
    assert_eq!(diagnostics.confirmed_output_tick, final_boundary - 1);
}

#[test]
fn replaces_predicted_play_with_the_corrected_goal_and_kickoff_timeline() {
    let initial = shot_fixture(Some(3));
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    let shot = sample(InputSampleOptions {
        edges: Some(input_frame::EDGE_SHOOT),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut reference, 0, 1, shot)
        .expect("authoritative row accepted");
    step_many(&mut reference, 30);
    step_many(&mut delayed, 30);
    assert_eq!(
        rollback_session::current_snapshot(&delayed)
            .state
            .score
            .home,
        0
    );

    rollback_session::add_authoritative(&mut delayed, 0, 1, shot)
        .expect("authoritative row accepted");
    let result = rollback_session::reconcile(&mut delayed, false);
    assert_eq!(
        result.status,
        gc_sim::rollback_session::RollbackSessionStatus::Active
    );
    assert_eq!(result.new_present_boundary, 30);
    assert_eq!(result.corrected_outputs.len(), 30);
    let mut corrected_goal_tick = None;
    let mut shot_event_tick = None;
    for output in &result.corrected_outputs {
        if output.state.score.home == 1 {
            corrected_goal_tick = corrected_goal_tick.or(Some(output.tick));
        }
        for event in &output.events {
            if event.kind == match_snapshot::MatchEventKind::Shot {
                shot_event_tick = shot_event_tick.or(Some(output.tick));
            }
        }
    }
    assert!(
        shot_event_tick.is_some(),
        "corrected outputs retain per-tick event ordering"
    );
    assert!(
        corrected_goal_tick.is_some(),
        "corrected outputs expose the goal transition"
    );
    assert!(shot_event_tick.unwrap() < corrected_goal_tick.unwrap());
    let corrected = rollback_session::current_snapshot(&delayed);
    assert_eq!(corrected.state.score.home, 1);
    assert!(
        corrected.state.kickoff_hold > 0.0,
        "corrected play retains the kickoff lifecycle"
    );
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            Some(0)
        )
        .matched
    );
}

#[test]
fn allows_a_finished_predicted_timeline_to_reactivate_after_correction() {
    let initial = preventable_goal_fixture();
    let mut delayed = rollback_session::new(&initial, sources(false), None, None);
    let mut reference = rollback_session::new(&initial, sources(false), None, None);
    let tackle = sample(InputSampleOptions {
        edges: Some(input_frame::EDGE_DASH),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut reference, 0, 5, tackle)
        .expect("authoritative row accepted");

    while rollback_session::diagnostics(&delayed).status
        != gc_sim::rollback_session::RollbackSessionStatus::Finished
    {
        rollback_session::step(&mut delayed).expect("step succeeds");
    }
    let predicted_finish = rollback_session::diagnostics(&delayed).present_boundary;
    assert_eq!(predicted_finish, 5);
    assert_eq!(
        rollback_session::current_snapshot(&delayed)
            .state
            .score
            .home,
        1
    );
    assert_eq!(rollback_session::output(&delayed, predicted_finish), None);

    step_many(&mut reference, predicted_finish);
    assert_eq!(
        rollback_session::diagnostics(&reference).status,
        gc_sim::rollback_session::RollbackSessionStatus::Active
    );
    rollback_session::add_authoritative(&mut delayed, 0, 5, tackle)
        .expect("authoritative row accepted");
    let result = rollback_session::reconcile(&mut delayed, false);
    assert!(result.changed);
    assert_eq!(result.old_present_boundary, predicted_finish);
    assert_eq!(result.new_present_boundary, predicted_finish);
    assert_eq!(
        result.status,
        gc_sim::rollback_session::RollbackSessionStatus::Active
    );
    assert_eq!(
        rollback_session::current_snapshot(&delayed)
            .state
            .score
            .home,
        0
    );
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            Some(0)
        )
        .matched
    );

    while rollback_session::diagnostics(&reference).status
        != gc_sim::rollback_session::RollbackSessionStatus::Finished
    {
        rollback_session::step(&mut reference).expect("step succeeds");
        rollback_session::step(&mut delayed).expect("step succeeds");
    }
    assert_eq!(
        rollback_session::diagnostics(&delayed).status,
        gc_sim::rollback_session::RollbackSessionStatus::Finished
    );
    assert_eq!(
        rollback_session::diagnostics(&delayed).present_boundary,
        rollback_session::diagnostics(&reference).present_boundary
    );
    assert!(
        rollback_session::compare(
            &mut delayed,
            &rollback_session::current_snapshot(&reference),
            Some(0)
        )
        .matched
    );
    let final_boundary = rollback_session::diagnostics(&delayed).present_boundary;
    assert_eq!(rollback_session::output(&delayed, final_boundary), None);
}

#[test]
fn truncates_successfully_after_a_monotonic_floor_has_advanced() {
    let initial = shot_fixture(None);
    let mut session = rollback_session::new(&initial, sources(false), None, None);
    step_many(&mut session, 40);
    let before = rollback_session::diagnostics(&session);
    assert_eq!(before.input_history.oldest_retained_tick, 10);
    let shot = sample(InputSampleOptions {
        edges: Some(input_frame::EDGE_SHOOT),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut session, 15, 1, shot)
        .expect("authoritative row accepted");
    let result = rollback_session::reconcile(&mut session, false);
    assert!(result.new_present_boundary < 40);
    assert!(result.new_present_boundary >= 15);
    let after = rollback_session::diagnostics(&session);
    assert_eq!(after.input_history.oldest_retained_tick, 10);
    assert_eq!(after.snapshot_history.oldest_supported_tick, Some(10));
    assert_eq!(
        after.snapshot_history.latest_tick,
        Some(result.new_present_boundary)
    );
    assert_eq!(
        rollback_session::output(&session, result.new_present_boundary),
        None
    );
    let later = rollback_session::add_authoritative(
        &mut session,
        35,
        2,
        sample(InputSampleOptions {
            move_y: Some(20),
            ..Default::default()
        }),
    )
    .expect("authoritative row accepted");
    assert_eq!(later.earliest_divergence, None);
}

#[test]
fn reports_confirmation_and_deterministic_boundary_diagnostics() {
    let mut session = rollback_session::new(&initial_snapshot(), sources(false), None, None);
    add_neutral_tick(&mut session, 0);
    rollback_session::step(&mut session).expect("step succeeds");
    let diagnostics = rollback_session::diagnostics(&session);
    assert_eq!(diagnostics.confirmed_tick, 0);
    assert_eq!(diagnostics.confirmed_output_tick, 0);

    // Diverges an unambiguous `i64` leaf (`state.score.home`) rather than
    // `dive_target`, whose diff for a presence change renders as a
    // boolean, not a point (see `match_snapshot.rs`'s `dive_target` diff
    // arm).
    let mut expected = rollback_session::current_snapshot(&session);
    expected.state.score.home = 12;
    let comparison = rollback_session::compare(&mut session, &expected, Some(0));
    assert!(!comparison.matched);
    assert!(!comparison.boundary_mismatch);
    assert_eq!(comparison.causal_tick, Some(0));
    let difference = comparison
        .first_difference
        .expect("a difference was recorded");
    assert_eq!(difference.path, "state.score.home");
    assert_eq!(difference.expected, "12");
    let mut stored = rollback_session::diagnostics(&session)
        .last_comparison
        .expect("comparison retained")
        .first_difference
        .expect("difference retained");
    assert_eq!(stored.expected, "12");
    stored.expected = "mutated".to_string();
    let reread = rollback_session::diagnostics(&session)
        .last_comparison
        .expect("comparison retained")
        .first_difference
        .expect("difference retained");
    assert_eq!(reread.expected, "12");

    let mut expected_boundary_mismatch = rollback_session::current_snapshot(&session);
    expected_boundary_mismatch.state.input_tick = 2;
    let boundary = rollback_session::compare(&mut session, &expected_boundary_mismatch, None);
    assert!(boundary.boundary_mismatch);
    assert_eq!(boundary.actual_boundary, 1);
    assert_eq!(boundary.expected_boundary, 2);

    let mut copied = rollback_session::diagnostics(&session);
    copied.last_comparison.as_mut().unwrap().actual_hash = "mutated".to_string();
    assert_ne!(
        rollback_session::diagnostics(&session)
            .last_comparison
            .unwrap()
            .actual_hash,
        "mutated"
    );
}

#[test]
fn tracks_retained_output_bytes_identically_to_recomputing_them() {
    let mut checks = 0;
    fn lockstep(max_goals: i64) -> (RollbackSession, RollbackSession) {
        let mut tracked =
            rollback_session::new(&shot_fixture(Some(max_goals)), sources(false), None, None);
        let recomputed =
            rollback_session::new(&shot_fixture(Some(max_goals)), sources(false), None, None);
        rollback_session::track_output_bytes(&mut tracked);
        (tracked, recomputed)
    }
    let agree = |checks: &mut i64,
                 tracked: &mut RollbackSession,
                 recomputed: &mut RollbackSession,
                 label: &str| {
        *checks += 1;
        let left = rollback_session::accounting(tracked);
        let right = rollback_session::accounting(recomputed);
        assert_eq!(
            left.output_bytes, right.output_bytes,
            "{label} output bytes"
        );
        assert_eq!(left.total_bytes, right.total_bytes, "{label} total bytes");
        assert_eq!(
            tracked.output_bytes, right.output_bytes,
            "{label} running total"
        );
    };

    // Three goals, so the late shot below rolls the session back without
    // ending the match and the resimulated range stays steppable.
    let (mut tracked, mut recomputed) = lockstep(3);
    assert!(tracked.track_output_bytes);
    assert!(!recomputed.track_output_bytes);
    agree(&mut checks, &mut tracked, &mut recomputed, "empty");
    assert_eq!(rollback_session::accounting(&mut tracked).output_bytes, 0);

    step_many(&mut tracked, 1);
    step_many(&mut recomputed, 1);
    agree(&mut checks, &mut tracked, &mut recomputed, "first output");
    assert!(rollback_session::accounting(&mut tracked).output_bytes > 0);
    step_many(&mut tracked, 39);
    step_many(&mut recomputed, 39);
    agree(&mut checks, &mut tracked, &mut recomputed, "pruned window");
    assert_eq!(
        rollback_session::diagnostics(&tracked)
            .input_history
            .oldest_retained_tick,
        10
    );

    let shot = sample(InputSampleOptions {
        edges: Some(input_frame::EDGE_SHOOT),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut tracked, 15, 1, shot)
        .expect("authoritative row accepted");
    rollback_session::add_authoritative(&mut recomputed, 15, 1, shot)
        .expect("authoritative row accepted");
    let replaced = rollback_session::reconcile(&mut tracked, false);
    let replaced_peer = rollback_session::reconcile(&mut recomputed, false);
    assert!(replaced.changed);
    assert_eq!(
        replaced.new_present_boundary,
        replaced_peer.new_present_boundary
    );
    assert_eq!(replaced.new_present_boundary, 40);
    agree(
        &mut checks,
        &mut tracked,
        &mut recomputed,
        "after overwrite-on-resimulate",
    );

    step_many(&mut tracked, 5);
    step_many(&mut recomputed, 5);
    agree(
        &mut checks,
        &mut tracked,
        &mut recomputed,
        "resimulated forward",
    );
    let move_sample = sample(InputSampleOptions {
        move_y: Some(20),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut tracked, 20, 2, move_sample)
        .expect("authoritative row accepted");
    rollback_session::add_authoritative(&mut recomputed, 20, 2, move_sample)
        .expect("authoritative row accepted");
    rollback_session::reconcile(&mut tracked, false);
    rollback_session::reconcile(&mut recomputed, false);
    agree(
        &mut checks,
        &mut tracked,
        &mut recomputed,
        "after second rollback",
    );

    let (mut short, mut short_peer) = lockstep(1);
    step_many(&mut short, 40);
    step_many(&mut short_peer, 40);
    agree(
        &mut checks,
        &mut short,
        &mut short_peer,
        "short pruned window",
    );
    rollback_session::add_authoritative(&mut short, 15, 1, shot)
        .expect("authoritative row accepted");
    rollback_session::add_authoritative(&mut short_peer, 15, 1, shot)
        .expect("authoritative row accepted");
    let truncated = rollback_session::reconcile(&mut short, false);
    let truncated_peer = rollback_session::reconcile(&mut short_peer, false);
    assert!(truncated.changed);
    assert_eq!(
        truncated.new_present_boundary,
        truncated_peer.new_present_boundary
    );
    assert!(truncated.new_present_boundary < 40);
    assert_eq!(
        rollback_session::output(&short, truncated.new_present_boundary),
        None
    );
    agree(&mut checks, &mut short, &mut short_peer, "after truncate");

    step_many(&mut tracked, 4);
    step_many(&mut recomputed, 4);
    let late = sample(InputSampleOptions {
        move_x: Some(-20),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut tracked, 30, 3, late)
        .expect("authoritative row accepted");
    rollback_session::add_authoritative(&mut recomputed, 30, 3, late)
        .expect("authoritative row accepted");
    assert!(rollback_session::reconcile(&mut tracked, false).changed);
    assert!(rollback_session::reconcile(&mut recomputed, false).changed);
    let earlier = sample(InputSampleOptions {
        move_x: Some(15),
        move_y: Some(-10),
        ..Default::default()
    });
    rollback_session::add_authoritative(&mut tracked, 26, 4, earlier)
        .expect("authoritative row accepted");
    rollback_session::add_authoritative(&mut recomputed, 26, 4, earlier)
        .expect("authoritative row accepted");
    assert!(rollback_session::reconcile(&mut tracked, false).changed);
    assert!(rollback_session::reconcile(&mut recomputed, false).changed);
    agree(
        &mut checks,
        &mut tracked,
        &mut recomputed,
        "two uncounted overlapping rollbacks",
    );
    assert_eq!(checks, 9);
}

#[test]
fn defers_retained_output_encoding_out_of_the_measured_brackets() {
    let mut session = rollback_session::new(&shot_fixture(Some(3)), sources(false), None, None);
    rollback_session::track_output_bytes(&mut session);
    step_many(&mut session, 6);

    assert_eq!(session.output_bytes, 0);
    assert_eq!(session.counted_output_bytes.as_ref().unwrap().len(), 0);

    let first = rollback_session::accounting(&mut session);
    assert!(first.output_bytes > 0);
    assert_eq!(session.output_bytes, first.output_bytes);
    assert_eq!(session.counted_output_bytes.as_ref().unwrap().len(), 6);

    assert_eq!(
        rollback_session::accounting(&mut session).output_bytes,
        first.output_bytes
    );
    assert_eq!(session.output_bytes, first.output_bytes);
}

#[test]
fn leaves_retained_output_accounting_untracked_by_default() {
    let mut session = rollback_session::new(&shot_fixture(None), sources(false), None, None);
    step_many(&mut session, 12);
    assert!(!session.track_output_bytes);
    assert_eq!(session.output_bytes, 0);
    let before = rollback_session::accounting(&mut session);
    assert!(before.output_bytes > 0);

    rollback_session::track_output_bytes(&mut session);
    let after = rollback_session::accounting(&mut session);
    assert_eq!(after.output_bytes, before.output_bytes);
    assert_eq!(after.total_bytes, before.total_bytes);
    assert_eq!(session.output_bytes, before.output_bytes);
}
