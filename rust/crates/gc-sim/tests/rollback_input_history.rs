//! Port of `spec/sim/rollback_input_history_spec.lua`.
//!
//! Two Lua sub-cases cannot be expressed here and are dropped, not merely
//! already-passing — both documented again at their specific test below:
//!
//! - "uses neutral first-sample prediction and preserves local/remote
//!   status" mutates its `sources` table after `new()` to prove the
//!   constructor copies rather than aliases it. `[RollbackInputSource; 8]`
//!   is `Copy` and passed by value, so that aliasing bug class does not
//!   exist here.
//! - "fails malformed source configuration loudly and malformed arrivals
//!   recoverably" sets `malformed_sources[8] = "spectator"` and asserts
//!   `new` panics. [`rollback_input_history::RollbackInputSource`] is a
//!   closed two-variant enum, so that state cannot be constructed at all —
//!   the Lua runtime assertion is a compile-time guarantee here instead.
//!
//! "truncates only the obsolete effective tail and preserves earlier
//! divergence" also adapts one sub-case: the Lua original's second malformed
//! `truncate_from` tick is a non-integer float (`3.5`); `boundary_tick: i64`
//! makes that unrepresentable, so an out-of-range integer tick is
//! substituted, exercising the identical "malformed" branch (see that
//! test's comment).
//!
//! Every other assertion is ported in full. "deep-copies authoritative,
//! effective, and caller-returned records" keeps every assertion verbatim:
//! `InputSample`/`InputFrame`/`RollbackInputTickRecord`/
//! `RollbackInputSlotRecord` are all `Copy`, so the Lua original's
//! `copy_sample`/`copy_frame`/`copy_record` defenses have no Rust
//! equivalent to invoke, but what they protected still holds.

use gc_sim::input_frame::{self, EdgeAction, HeldAction, InputSample, InputSampleOptions};
use gc_sim::rollback_input_history::{
    self, RollbackAuthoritativeInput, RollbackInputHistory, RollbackInputHistoryErrorCode,
    RollbackInputSource, RollbackInputStatus,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn fails<F: FnOnce()>(f: F) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

fn sources(local_slot: Option<i64>) -> [RollbackInputSource; 8] {
    std::array::from_fn(|i| {
        let slot = i as i64 + 1;
        if Some(slot) == local_slot {
            RollbackInputSource::Local
        } else {
            RollbackInputSource::Remote
        }
    })
}

fn sample(move_x: i64, move_y: i64, held: i64, edges: i64) -> InputSample {
    input_frame::new_sample(InputSampleOptions {
        move_x: Some(move_x),
        move_y: Some(move_y),
        held: Some(held),
        edges: Some(edges),
    })
    .unwrap()
}

fn add_full_tick(history: &mut RollbackInputHistory, tick: i64, value: Option<InputSample>) {
    for slot in 1..=input_frame::SLOT_COUNT {
        rollback_input_history::add_authoritative(
            history,
            tick,
            slot,
            value.unwrap_or_else(input_frame::neutral_sample),
        )
        .unwrap();
    }
}

#[test]
fn omp2_rollback_input_history_uses_neutral_first_sample_prediction_and_preserves_local_remote_status()
 {
    let mut history = rollback_input_history::new(sources(Some(1)));

    assert_eq!(
        rollback_input_history::source(&history, 1),
        RollbackInputSource::Local
    );
    assert_eq!(rollback_input_history::confirmed_tick(&history), -1);
    assert!(
        fails(|| {
            rollback_input_history::materialize(&mut history, 0);
        }),
        "a missing local sample is a producer invariant failure"
    );

    rollback_input_history::add_authoritative(
        &mut history,
        0,
        1,
        sample(20, -30, HeldAction::Sprint.bit(), EdgeAction::Dash.bit()),
    )
    .unwrap();
    rollback_input_history::add_authoritative(
        &mut history,
        0,
        2,
        sample(-10, 40, HeldAction::Jockey.bit(), EdgeAction::Dodge.bit()),
    )
    .unwrap();
    let (frame, record) = rollback_input_history::materialize(&mut history, 0);
    assert_eq!(frame.slots[0].move_x, 20);
    assert_eq!(record.slots[0].source, RollbackInputSource::Local);
    assert_eq!(record.slots[0].status, RollbackInputStatus::Authoritative);
    assert_eq!(record.slots[1].source, RollbackInputSource::Remote);
    assert_eq!(record.slots[1].status, RollbackInputStatus::Authoritative);
    assert_eq!(record.slots[2].status, RollbackInputStatus::Predicted);
    assert_eq!(frame.slots[2].move_x, 0);
    assert_eq!(frame.slots[2].move_y, 0);
    assert_eq!(frame.slots[2].held, 0);
    assert_eq!(frame.slots[2].edges, 0);
}

#[test]
fn omp2_rollback_input_history_repeats_only_axes_and_held_bits_from_the_latest_prior_authority() {
    let mut history = rollback_input_history::new(sources(None));
    let authoritative = sample(
        -64,
        90,
        HeldAction::Shoot.bit() + HeldAction::Lob.bit() + HeldAction::Equipment.bit(),
        EdgeAction::Shoot.bit() + EdgeAction::Dash.bit() + EdgeAction::EquipmentPressed.bit(),
    );
    rollback_input_history::add_authoritative(&mut history, 5, 1, authoritative).unwrap();

    let (before, before_record) = rollback_input_history::materialize(&mut history, 4);
    assert_eq!(
        before.slots[0].move_x, 0,
        "future authority cannot predict backward"
    );
    assert_eq!(
        before_record.slots[0].status,
        RollbackInputStatus::Predicted
    );

    let (at_tick, at_record) = rollback_input_history::materialize(&mut history, 5);
    assert_eq!(at_tick.slots[0].edges, authoritative.edges);
    assert_eq!(
        at_record.slots[0].status,
        RollbackInputStatus::Authoritative
    );

    let (predicted, predicted_record) = rollback_input_history::materialize(&mut history, 6);
    assert_eq!(predicted.slots[0].move_x, authoritative.move_x);
    assert_eq!(predicted.slots[0].move_y, authoritative.move_y);
    assert_eq!(predicted.slots[0].held, authoritative.held);
    assert_eq!(predicted.slots[0].edges, 0, "edge bits never repeat");
    assert!(input_frame::is_held(&predicted.slots[0], HeldAction::Equipment).unwrap());
    assert!(!input_frame::has_edge(&predicted.slots[0], EdgeAction::EquipmentPressed).unwrap());
    assert!(!input_frame::has_edge(&predicted.slots[0], EdgeAction::EquipmentReleased).unwrap());
    assert_eq!(
        predicted_record.slots[0].status,
        RollbackInputStatus::Predicted
    );
}

#[test]
fn omp2_rollback_input_history_finds_the_exact_release_edge_after_predicting_only_the_equipment_hold()
 {
    let mut history = rollback_input_history::new(sources(None));
    let pressed = sample(
        0,
        0,
        HeldAction::Equipment.bit(),
        EdgeAction::EquipmentPressed.bit(),
    );
    rollback_input_history::add_authoritative(&mut history, 0, 1, pressed).unwrap();
    rollback_input_history::materialize(&mut history, 0);
    let (predicted, _) = rollback_input_history::materialize(&mut history, 1);
    assert!(input_frame::is_held(&predicted.slots[0], HeldAction::Equipment).unwrap());
    assert_eq!(predicted.slots[0].edges, 0);

    let released = sample(0, 0, 0, EdgeAction::EquipmentReleased.bit());
    let arrival = rollback_input_history::add_authoritative(&mut history, 1, 1, released).unwrap();
    assert_eq!(arrival.earliest_divergence, Some(1));
    assert_eq!(
        rollback_input_history::earliest_divergence(&history),
        Some(1)
    );
}

#[test]
fn omp2_rollback_input_history_reports_divergence_only_against_an_effective_sample_already_used() {
    let mut history = rollback_input_history::new(sources(None));
    rollback_input_history::materialize(&mut history, 2);
    rollback_input_history::materialize(&mut history, 5);

    let same = rollback_input_history::add_authoritative(
        &mut history,
        2,
        1,
        input_frame::neutral_sample(),
    )
    .unwrap();
    assert_eq!(
        same.earliest_divergence, None,
        "an identical prediction needs no correction"
    );

    let later =
        rollback_input_history::add_authoritative(&mut history, 5, 1, sample(10, 0, 0, 0)).unwrap();
    assert_eq!(later.earliest_divergence, Some(5));
    let earlier =
        rollback_input_history::add_authoritative(&mut history, 2, 2, sample(-10, 0, 0, 0))
            .unwrap();
    assert_eq!(earlier.earliest_divergence, Some(2));
    assert_eq!(
        rollback_input_history::earliest_divergence(&history),
        Some(2)
    );
    assert!(
        fails(|| {
            rollback_input_history::materialize(&mut history, 2);
        }),
        "a corrected timeline cannot overwrite used records before divergence is consumed"
    );
    assert_eq!(
        rollback_input_history::consume_earliest_divergence(&mut history),
        Some(2)
    );
    assert_eq!(
        rollback_input_history::consume_earliest_divergence(&mut history),
        None
    );

    let (corrected, _) = rollback_input_history::materialize(&mut history, 2);
    assert_eq!(corrected.slots[1].move_x, -10);
    rollback_input_history::add_authoritative(&mut history, 3, 1, sample(40, 0, 0, 0)).unwrap();
    assert_eq!(
        rollback_input_history::earliest_divergence(&history),
        None,
        "unmaterialized input is authority, not a correction"
    );
}

#[test]
fn omp2_rollback_input_history_advances_confirmation_monotonically_across_contiguous_complete_ticks()
 {
    let mut history = rollback_input_history::new(sources(None));
    assert_eq!(rollback_input_history::confirmed_tick(&history), -1);

    add_full_tick(&mut history, 1, None);
    assert_eq!(
        rollback_input_history::confirmed_tick(&history),
        -1,
        "tick zero remains a gap"
    );
    add_full_tick(&mut history, 0, None);
    assert_eq!(rollback_input_history::confirmed_tick(&history), 1);
    add_full_tick(&mut history, 3, None);
    assert_eq!(
        rollback_input_history::confirmed_tick(&history),
        1,
        "tick two remains a gap"
    );
    add_full_tick(&mut history, 2, None);
    assert_eq!(rollback_input_history::confirmed_tick(&history), 3);
}

#[test]
fn omp2_rollback_input_history_accepts_identical_duplicates_and_rejects_conflicts_without_mutation()
{
    let mut history = rollback_input_history::new(sources(None));
    let original = sample(12, -4, HeldAction::Pass.bit(), EdgeAction::Pass.bit());
    let first = rollback_input_history::add_authoritative(&mut history, 0, 1, original).unwrap();
    assert!(!first.duplicate);
    let duplicate =
        rollback_input_history::add_authoritative(&mut history, 0, 1, sample(12, -4, 2, 2))
            .unwrap();
    assert!(duplicate.duplicate);

    let rejected =
        rollback_input_history::add_authoritative(&mut history, 0, 1, sample(13, -4, 2, 2));
    let err = rejected.unwrap_err();
    assert_eq!(
        err.code,
        RollbackInputHistoryErrorCode::ConflictingAuthoritative
    );
    assert!(err.message.contains("tick 0 slot 1"));
    let retained = rollback_input_history::authoritative_record(&history, 0, 1).unwrap();
    assert_eq!(retained.sample.move_x, 12);
    assert_eq!(rollback_input_history::earliest_divergence(&history), None);

    for slot in 2..=input_frame::SLOT_COUNT {
        rollback_input_history::add_authoritative(
            &mut history,
            0,
            slot,
            input_frame::neutral_sample(),
        )
        .unwrap();
    }
    assert_eq!(rollback_input_history::confirmed_tick(&history), 0);
}

#[test]
fn omp2_rollback_input_history_preflights_complete_authority_batches_before_one_atomic_insertion() {
    let mut history = rollback_input_history::new(sources(None));
    let first = sample(12, -4, HeldAction::Pass.bit(), EdgeAction::Pass.bit());
    let third = sample(30, 0, 0, 0);
    rollback_input_history::add_authoritative(&mut history, 0, 1, first).unwrap();
    rollback_input_history::add_authoritative(&mut history, 0, 3, third).unwrap();
    let before = rollback_input_history::diagnostics(&history);

    let rejected = rollback_input_history::add_authoritative_batch(
        &mut history,
        &[
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 1,
                sample: first,
            },
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 2,
                sample: sample(20, 0, 0, 0),
            },
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 3,
                sample: sample(31, 0, 0, 0),
            },
        ],
    );
    assert_eq!(
        rejected.unwrap_err().code,
        RollbackInputHistoryErrorCode::ConflictingAuthoritative
    );
    let after = rollback_input_history::diagnostics(&history);
    assert_eq!(
        after.authoritative_sample_count,
        before.authoritative_sample_count
    );
    assert_eq!(
        rollback_input_history::authoritative_record(&history, 0, 2),
        None
    );
    assert_eq!(
        rollback_input_history::authoritative_record(&history, 0, 3)
            .unwrap()
            .sample
            .move_x,
        30
    );

    let inserted = rollback_input_history::add_authoritative_batch(
        &mut history,
        &[
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 1,
                sample: first,
            },
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 2,
                sample: sample(20, 0, 0, 0),
            },
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 2,
                sample: sample(20, 0, 0, 0),
            },
            RollbackAuthoritativeInput {
                tick: 1,
                slot_index: 1,
                sample: sample(40, 0, 0, 0),
            },
        ],
    )
    .unwrap();
    assert_eq!(inserted.inserted, 2);
    assert_eq!(inserted.duplicates, 2);
    assert_eq!(inserted.inserted_rows.len(), 2);
    assert_eq!(
        rollback_input_history::authoritative_record(&history, 0, 2)
            .unwrap()
            .sample
            .move_x,
        20
    );
    assert_eq!(
        rollback_input_history::authoritative_record(&history, 1, 1)
            .unwrap()
            .sample
            .move_x,
        40
    );

    let rejected = rollback_input_history::add_authoritative_batch(
        &mut history,
        &[
            RollbackAuthoritativeInput {
                tick: 1,
                slot_index: 2,
                sample: input_frame::neutral_sample(),
            },
            RollbackAuthoritativeInput {
                tick: 0,
                slot_index: 4,
                sample: input_frame::neutral_sample(),
            },
        ],
    );
    assert_eq!(
        rejected.unwrap_err().code,
        RollbackInputHistoryErrorCode::Malformed
    );
    assert_eq!(
        rollback_input_history::authoritative_record(&history, 1, 2),
        None
    );
}

#[test]
// Every mutation below is deliberately of a local, already-independent copy,
// to prove (by asserting against a freshly re-fetched value afterward) that
// it cannot reach the retained state. rustc cannot see that intent and
// treats the mutations as dead stores.
#[allow(unused_assignments, unused_variables)]
fn omp2_rollback_input_history_deep_copies_authoritative_effective_and_caller_returned_records() {
    // `InputSample` is `Copy`, so every hand-off below is already an
    // independent value; the Lua original's `copy_sample`/`copy_frame`/
    // `copy_record` defenses have no Rust equivalent to invoke, but every
    // assertion they protected still holds and is kept verbatim.
    let mut history = rollback_input_history::new(sources(None));
    let mut supplied = sample(70, -20, HeldAction::Sprint.bit(), EdgeAction::Dodge.bit());
    rollback_input_history::add_authoritative(&mut history, 0, 1, supplied).unwrap();
    supplied.move_x = -99;

    let (mut frame, mut tick_record) = rollback_input_history::materialize(&mut history, 0);
    assert_eq!(frame.slots[0].move_x, 70);
    frame.slots[0].move_x = 1;
    tick_record.slots[0].sample.move_x = 2;
    let retained_record = rollback_input_history::record(&history, 0).unwrap();
    assert_eq!(retained_record.slots[0].sample.move_x, 70);

    let mut authoritative = rollback_input_history::authoritative_record(&history, 0, 1).unwrap();
    authoritative.sample.move_x = 3;
    let retained_authoritative =
        rollback_input_history::authoritative_record(&history, 0, 1).unwrap();
    assert_eq!(retained_authoritative.sample.move_x, 70);
}

#[test]
fn omp2_rollback_input_history_prunes_through_a_public_floor_while_retaining_one_prediction_anchor_per_slot()
 {
    let mut history = rollback_input_history::new(sources(None));
    rollback_input_history::add_authoritative(&mut history, 0, 1, sample(10, 0, 0, 0)).unwrap();
    rollback_input_history::add_authoritative(&mut history, 5, 1, sample(50, 0, 0, 0)).unwrap();
    rollback_input_history::add_authoritative(&mut history, 10, 1, sample(100, 0, 0, 0)).unwrap();
    for tick in 0..=10 {
        rollback_input_history::materialize(&mut history, tick);
    }

    let diagnostics = rollback_input_history::prune_before(&mut history, 8).unwrap();
    assert_eq!(diagnostics.oldest_retained_tick, 8);
    assert_eq!(diagnostics.newest_retained_tick, Some(10));
    assert_eq!(diagnostics.authoritative_tick_count, 1);
    assert_eq!(diagnostics.authoritative_sample_count, 1);
    assert_eq!(diagnostics.effective_tick_count, 3);
    assert_eq!(diagnostics.record_tick_count, 3);
    assert_eq!(diagnostics.predecessor_anchor_count, 1);
    assert_eq!(
        rollback_input_history::authoritative_record(&history, 5, 1),
        None
    );

    let (replayed, _) = rollback_input_history::materialize(&mut history, 8);
    assert_eq!(
        replayed.slots[0].move_x, 50,
        "the copied predecessor anchors prediction"
    );
    let rejected =
        rollback_input_history::add_authoritative(&mut history, 7, 1, sample(70, 0, 0, 0));
    let err = rejected.unwrap_err();
    assert_eq!(err.code, RollbackInputHistoryErrorCode::OutsideWindow);
    assert!(err.message.contains("older than retained tick 8"));
}

#[test]
fn omp2_rollback_input_history_never_prunes_an_unconsumed_divergence_and_preserves_monotonic_confirmation()
 {
    let mut history = rollback_input_history::new(sources(None));
    for tick in 0..=2 {
        add_full_tick(&mut history, tick, None);
        rollback_input_history::materialize(&mut history, tick);
    }
    assert_eq!(rollback_input_history::confirmed_tick(&history), 2);
    rollback_input_history::add_authoritative(&mut history, 3, 1, sample(30, 0, 0, 0)).unwrap();
    rollback_input_history::materialize(&mut history, 3);
    let correction =
        rollback_input_history::add_authoritative(&mut history, 3, 2, sample(-30, 0, 0, 0))
            .unwrap();
    assert_eq!(correction.earliest_divergence, Some(3));

    let pruned = rollback_input_history::prune_before(&mut history, 4);
    assert_eq!(
        pruned.unwrap_err().code,
        RollbackInputHistoryErrorCode::PendingDivergence
    );
    assert_eq!(
        rollback_input_history::diagnostics(&history).oldest_retained_tick,
        0
    );
    assert_eq!(
        rollback_input_history::consume_earliest_divergence(&mut history),
        Some(3)
    );
    let (replayed, _) = rollback_input_history::materialize(&mut history, 3);
    assert_eq!(
        replayed.slots[1].move_x, -30,
        "resimulation replaces the effective record"
    );
    let replaced = rollback_input_history::record(&history, 3).unwrap();
    assert_eq!(replaced.slots[1].sample.move_x, -30);
    assert_eq!(
        rollback_input_history::diagnostics(&history).effective_tick_count,
        4
    );

    rollback_input_history::prune_before(&mut history, 3).unwrap();
    for slot in 2..=input_frame::SLOT_COUNT {
        if slot != 2 {
            rollback_input_history::add_authoritative(
                &mut history,
                3,
                slot,
                input_frame::neutral_sample(),
            )
            .unwrap();
        }
    }
    assert_eq!(rollback_input_history::confirmed_tick(&history), 3);
}

#[test]
fn omp2_rollback_input_history_truncates_only_the_obsolete_effective_tail_and_preserves_earlier_divergence()
 {
    let mut history = rollback_input_history::new(sources(None));
    rollback_input_history::materialize(&mut history, 1);
    rollback_input_history::materialize(&mut history, 4);
    rollback_input_history::add_authoritative(&mut history, 1, 1, sample(10, 0, 0, 0)).unwrap();
    rollback_input_history::add_authoritative(&mut history, 4, 1, sample(40, 0, 0, 0)).unwrap();
    assert_eq!(
        rollback_input_history::earliest_divergence(&history),
        Some(1)
    );

    let truncated = rollback_input_history::truncate_from(&mut history, 4).unwrap();
    assert_eq!(truncated.effective_removed, 1);
    assert_eq!(truncated.records_removed, 1);
    assert!(!truncated.cleared_divergence);
    assert_eq!(truncated.diagnostics.earliest_divergence, Some(1));
    assert_eq!(rollback_input_history::record(&history, 4), None);
    assert!(rollback_input_history::authoritative_record(&history, 4, 1).is_some());

    let mut bounded = rollback_input_history::new(sources(None));
    rollback_input_history::prune_before(&mut bounded, 3).unwrap();
    let rejected = rollback_input_history::truncate_from(&mut bounded, 2);
    assert_eq!(
        rejected.unwrap_err().code,
        RollbackInputHistoryErrorCode::OutsideWindow
    );
    // The Lua original's second malformed sub-case passes a non-integer tick
    // (`3.5`); `boundary_tick: i64` makes that state unrepresentable here
    // (the same "dropped as structurally redundant" reasoning `input_frame`'s
    // port documents). An out-of-range integer tick exercises the identical
    // "malformed" branch instead: the Lua source combines both conditions
    // (`not is_integer(...)` and the range check) into one `or`-chained
    // check with the same error code, so this is the same branch, not a
    // weaker one.
    let rejected = rollback_input_history::truncate_from(&mut bounded, -1);
    assert_eq!(
        rejected.unwrap_err().code,
        RollbackInputHistoryErrorCode::Malformed
    );
    assert_eq!(
        rollback_input_history::diagnostics(&bounded).oldest_retained_tick,
        3
    );
}

#[test]
fn omp2_rollback_input_history_fails_malformed_source_configuration_loudly_and_malformed_arrivals_recoverably()
 {
    // The Lua original's first sub-case sets `malformed_sources[8] =
    // "spectator"` and asserts `new` panics. `[RollbackInputSource; 8]` is a
    // fixed-size array of a closed two-variant enum, so that state cannot be
    // constructed at all -- the Lua runtime assertion is a compile-time
    // guarantee here instead, and there is no runtime call left to make.

    let mut history = rollback_input_history::new(sources(None));
    let accepted = rollback_input_history::add_authoritative(
        &mut history,
        0,
        9,
        input_frame::neutral_sample(),
    );
    assert_eq!(
        accepted.unwrap_err().code,
        RollbackInputHistoryErrorCode::Malformed
    );
    let accepted = rollback_input_history::add_authoritative(
        &mut history,
        0,
        1,
        InputSample {
            move_x: 999,
            move_y: 0,
            held: 0,
            edges: 0,
        },
    );
    assert_eq!(
        accepted.unwrap_err().code,
        RollbackInputHistoryErrorCode::Malformed
    );
    assert_eq!(
        rollback_input_history::diagnostics(&history).authoritative_sample_count,
        0
    );
}

#[test]
fn omp2_rollback_input_history_keeps_a_7201_tick_three_delay_fixture_bounded() {
    let mut history = rollback_input_history::new(sources(None));
    for tick in 0..=7200_i64 {
        let arrived_tick = tick - 3;
        if arrived_tick >= 0 {
            let value = sample(arrived_tick.rem_euclid(255) - 127, 0, 0, 0);
            add_full_tick(&mut history, arrived_tick, Some(value));
            rollback_input_history::consume_earliest_divergence(&mut history);
        }
        rollback_input_history::materialize(&mut history, tick);
        rollback_input_history::prune_before(&mut history, (tick - 30).max(0)).unwrap();
    }

    let diagnostics = rollback_input_history::diagnostics(&history);
    assert_eq!(diagnostics.oldest_retained_tick, 7170);
    assert_eq!(diagnostics.newest_retained_tick, Some(7200));
    assert!(diagnostics.authoritative_tick_count <= 31);
    assert!(diagnostics.authoritative_sample_count <= 31 * input_frame::SLOT_COUNT);
    assert!(diagnostics.effective_tick_count <= 31);
    assert!(diagnostics.record_tick_count <= 31);
    assert_eq!(
        diagnostics.predecessor_anchor_count,
        input_frame::SLOT_COUNT
    );
}

#[test]
fn omp2_rollback_input_history_pins_the_initial_rollback_window_to_thirty_60hz_ticks() {
    assert_eq!(rollback_input_history::ROLLBACK_WINDOW_TICKS, 30);
    assert_eq!(rollback_input_history::ROLLBACK_WINDOW_MILLISECONDS, 500.0);
}
