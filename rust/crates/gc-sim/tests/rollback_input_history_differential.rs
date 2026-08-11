//! Differential test against reference vectors captured from the Lua
//! implementation this simulation was originally validated against (README
//! rule 5.9, `tools/lua_reference/README.md`), required for
//! `rollback_input_history`: it is the input ring buffer the rollback
//! re-simulation replays from, and a divergence here is a desync, not a
//! cosmetic difference.
//!
//! `tests/fixtures/rollback_input_history_lua_reference.txt` is the captured
//! stdout of running the real Lua `sim/rollback_input_history.lua` (plus its
//! `sim/input_frame.lua` and `sim/fixed_clock.lua` dependencies) under
//! headless `love` (no display, no `xvfb`), via a scratch
//! `conf.lua`/`main.lua` harness built per that README (not committed —
//! scratch dirs are session-local). This file reconstructs the identical
//! sequence of calls in Rust and asserts every captured line matches
//! byte-for-byte.
//!
//! Every value on both sides is an integer or a plain ASCII string —
//! `rollback_input_history` never produces a float (ticks, slot indices, and
//! sample components are all integers) — so plain equality is already
//! bit-exact comparison; there is no `%g` rounding ambiguity to route around
//! via `.to_bits()`.
//!
//! ## What this covers
//!
//! A single scenario, run against one [`RollbackInputHistory`], covering:
//!
//! - **A spread of inputs**: eight slots × 46 ticks (0..=45), added out of
//!   arrival order within each tick (slot 8 down to 1) to exercise
//!   `insert_tick`'s sorted insertion regardless of call order.
//! - **The wrap boundary**: the history is pruned to a rolling 30-tick
//!   window after every tick (mirroring `ROLLBACK_WINDOW_TICKS`), so by the
//!   end of the spread the oldest retained tick has advanced from `0` to
//!   `16`. The oldest retained tick still materializes fully authoritative;
//!   one tick earlier is rejected `outside_window`.
//! - **Batch insertion**: two duplicate rows against an already-retained
//!   tick, and a fresh tick (46) across all eight slots in one call, plus a
//!   separately rejected conflicting batch that must not mutate anything.
//! - **Out-of-range ticks at both ends**: a negative tick and
//!   `MAX_TICK + 1` are both rejected `malformed`.
//! - **The far end of the tick domain**: an authoritative sample at
//!   `MAX_TICK` itself, materialized directly. Every other slot has no
//!   authority at that exact tick, so each predicts from its own latest
//!   known authority (tick 46) — `predecessor_index`'s binary search at the
//!   largest possible index gap.
//! - **Truncation** of the tail (including the just-materialized `MAX_TICK`
//!   entry) back down to tick 46, leaving retained authoritative evidence
//!   untouched.

use gc_sim::input_frame::{self, InputSample, InputSampleOptions};
use gc_sim::rollback_input_history::{
    self, RollbackAuthoritativeInput, RollbackInputHistory, RollbackInputHistoryErrorCode,
    RollbackInputSource, RollbackInputStatus,
};
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/rollback_input_history_lua_reference.txt");

fn reference() -> IndexMap<&'static str, &'static str> {
    FIXTURE
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("malformed fixture line: {line}"));
            (key, value)
        })
        .collect()
}

fn expect<'a>(reference: &IndexMap<&'a str, &'a str>, key: &str) -> &'a str {
    reference
        .get(key)
        .unwrap_or_else(|| panic!("missing reference value for {key}"))
}

fn opt(value: Option<i64>) -> String {
    match value {
        None => "nil".to_string(),
        Some(v) => v.to_string(),
    }
}

fn code_label(code: RollbackInputHistoryErrorCode) -> &'static str {
    match code {
        RollbackInputHistoryErrorCode::Malformed => "malformed",
        RollbackInputHistoryErrorCode::ConflictingAuthoritative => "conflicting_authoritative",
        RollbackInputHistoryErrorCode::OutsideWindow => "outside_window",
        RollbackInputHistoryErrorCode::PendingDivergence => "pending_divergence",
    }
}

fn sample_for(tick: i64, slot: i64) -> InputSample {
    let move_x = (tick * 7 + slot * 13).rem_euclid(255) - 127;
    let move_y = (tick * 11 + slot * 3).rem_euclid(255) - 127;
    let held = (tick * 3 + slot).rem_euclid(128);
    let edges = (tick + slot * 5).rem_euclid(32);
    input_frame::new_sample(InputSampleOptions {
        move_x: Some(move_x),
        move_y: Some(move_y),
        held: Some(held),
        edges: Some(edges),
    })
    .expect("sample_for always produces an in-range sample")
}

/// Mirrors the Lua harness's `print_materialize`: `tick|slot,...|...|confirmed=X|divergence=Y`.
fn materialize_line(history: &mut RollbackInputHistory, tick: i64) -> String {
    let (_, record) = rollback_input_history::materialize(history, tick);
    let mut parts = vec![tick.to_string()];
    for slot in &record.slots {
        let source = match slot.source {
            RollbackInputSource::Local => "local",
            RollbackInputSource::Remote => "remote",
        };
        let status = match slot.status {
            RollbackInputStatus::Authoritative => "authoritative",
            RollbackInputStatus::Predicted => "predicted",
        };
        parts.push(format!(
            "{source},{status},{},{},{},{}",
            slot.sample.move_x, slot.sample.move_y, slot.sample.held, slot.sample.edges
        ));
    }
    parts.push(format!(
        "confirmed={}",
        rollback_input_history::confirmed_tick(history)
    ));
    parts.push(format!(
        "divergence={}",
        opt(rollback_input_history::earliest_divergence(history))
    ));
    parts.join("|")
}

/// Mirrors the Lua harness's `print_diagnostics` value half.
fn diagnostics_line(history: &RollbackInputHistory) -> String {
    let d = rollback_input_history::diagnostics(history);
    [
        format!("oldest={}", d.oldest_retained_tick),
        format!("newest={}", opt(d.newest_retained_tick)),
        format!("auth_ticks={}", d.authoritative_tick_count),
        format!("auth_samples={}", d.authoritative_sample_count),
        format!("eff_ticks={}", d.effective_tick_count),
        format!("rec_ticks={}", d.record_tick_count),
        format!("anchors={}", d.predecessor_anchor_count),
        format!("confirmed={}", d.confirmed_tick),
        format!("divergence={}", opt(d.earliest_divergence)),
    ]
    .join(";")
}

/// Mirrors the Lua harness's `print_accounting` value half.
fn accounting_line(history: &RollbackInputHistory) -> String {
    let a = rollback_input_history::accounting(history);
    [
        format!("auth_bytes={}", a.authoritative_bytes),
        format!("anchor_bytes={}", a.predecessor_anchor_bytes),
        format!("eff_bytes={}", a.effective_frame_bytes),
        format!("rec_bytes={}", a.input_record_bytes),
        format!("total_bytes={}", a.total_bytes),
    ]
    .join(";")
}

/// Mirrors the Lua harness's `print_add`.
fn add_line(
    history: &mut RollbackInputHistory,
    tick: i64,
    slot: i64,
    sample: InputSample,
) -> String {
    match rollback_input_history::add_authoritative(history, tick, slot, sample) {
        Ok(result) => format!(
            "ok;duplicate={};confirmed={};divergence={}",
            result.duplicate,
            result.confirmed_tick,
            opt(result.earliest_divergence)
        ),
        Err(err) => format!("err;code={};message={}", code_label(err.code), err.message),
    }
}

#[test]
fn rollback_input_history_matches_the_reference_lua_across_a_spread_wrap_boundary_and_tick_extremes()
 {
    let reference = reference();

    let sources: [RollbackInputSource; 8] = std::array::from_fn(|i| {
        if i == 0 {
            RollbackInputSource::Local
        } else {
            RollbackInputSource::Remote
        }
    });
    let mut history = rollback_input_history::new(sources);

    // Phase A: push a spread of authoritative inputs, out of arrival order
    // within each tick, then materialize and prune to a rolling 30-tick
    // window.
    for tick in 0..=45_i64 {
        for slot in (1..=8_i64).rev() {
            rollback_input_history::add_authoritative(
                &mut history,
                tick,
                slot,
                sample_for(tick, slot),
            )
            .expect("phase A arrivals are always in-window and non-conflicting");
        }
        rollback_input_history::materialize(&mut history, tick);
        rollback_input_history::consume_earliest_divergence(&mut history);
        rollback_input_history::prune_before(&mut history, (tick - 29).max(0))
            .expect("phase A pruning never targets a pending divergence");
    }

    assert_eq!(
        diagnostics_line(&history),
        expect(&reference, "after_phase_a_diagnostics")
    );
    assert_eq!(
        accounting_line(&history),
        expect(&reference, "after_phase_a_accounting")
    );

    // Wrap boundary: the oldest retained tick still materializes fully
    // authoritative; one tick earlier is rejected `outside_window`.
    assert_eq!(
        materialize_line(&mut history, 16),
        expect(&reference, "materialize_oldest_retained")
    );
    assert_eq!(
        add_line(&mut history, 15, 1, sample_for(15, 1)),
        expect(&reference, "add_below_window")
    );

    // Batch: two duplicates of already-retained rows, plus a fresh tick (46)
    // across all eight slots.
    let mut batch_ok = vec![
        RollbackAuthoritativeInput {
            tick: 40,
            slot_index: 1,
            sample: sample_for(40, 1),
        },
        RollbackAuthoritativeInput {
            tick: 40,
            slot_index: 2,
            sample: sample_for(40, 2),
        },
    ];
    for slot in 1..=8_i64 {
        batch_ok.push(RollbackAuthoritativeInput {
            tick: 46,
            slot_index: slot,
            sample: sample_for(46, slot),
        });
    }
    let batch_line = match rollback_input_history::add_authoritative_batch(&mut history, &batch_ok)
    {
        Ok(result) => format!(
            "ok;inserted={};duplicates={};confirmed={};divergence={}",
            result.inserted,
            result.duplicates,
            result.confirmed_tick,
            opt(result.earliest_divergence)
        ),
        Err(err) => format!("err;code={};message={}", code_label(err.code), err.message),
    };
    assert_eq!(batch_line, expect(&reference, "batch_ok"));

    // A conflicting batch (against an already-retained sample) must reject
    // the whole batch without mutating anything.
    let mut conflicting = sample_for(40, 3);
    conflicting.move_x = if conflicting.move_x == 0 { 1 } else { 0 };
    let err = rollback_input_history::add_authoritative_batch(
        &mut history,
        &[RollbackAuthoritativeInput {
            tick: 40,
            slot_index: 3,
            sample: conflicting,
        }],
    )
    .expect_err("a conflicting batch row must be rejected");
    assert_eq!(
        format!("nil;code={};message={}", code_label(err.code), err.message),
        expect(&reference, "batch_conflict")
    );

    assert_eq!(
        materialize_line(&mut history, 46),
        expect(&reference, "materialize_tick_46")
    );
    assert_eq!(
        diagnostics_line(&history),
        expect(&reference, "after_batch")
    );

    // Out-of-range ticks at both ends of the tick domain.
    assert_eq!(
        add_line(&mut history, -1, 1, sample_for(0, 1)),
        expect(&reference, "add_negative_tick")
    );
    assert_eq!(
        add_line(&mut history, input_frame::MAX_TICK + 1, 1, sample_for(0, 1)),
        expect(&reference, "add_tick_over_max")
    );

    assert_eq!(
        opt(rollback_input_history::earliest_divergence(&history)),
        expect(&reference, "divergence_before_max_tick")
    );

    // The far end: an authoritative sample at the largest representable
    // tick, materialized directly. Every other slot has no authority at
    // that exact tick, so it predicts from its own latest known authority
    // (tick 46, added by the batch above) -- `predecessor_index`'s search
    // at the largest possible index gap.
    assert_eq!(
        add_line(&mut history, input_frame::MAX_TICK, 1, sample_for(1, 1)),
        expect(&reference, "add_max_tick")
    );
    assert_eq!(
        materialize_line(&mut history, input_frame::MAX_TICK),
        expect(&reference, "materialize_max_tick")
    );
    assert_eq!(
        diagnostics_line(&history),
        expect(&reference, "after_max_tick_diagnostics")
    );
    assert_eq!(
        accounting_line(&history),
        expect(&reference, "after_max_tick_accounting")
    );

    // Truncate the tail (including the just-materialized MAX_TICK entry)
    // back down to tick 46, leaving retained authoritative evidence intact.
    let truncated = rollback_input_history::truncate_from(&mut history, 46)
        .expect("boundary 46 is within the retained window");
    let truncate_line = format!(
        "ok;boundary={};effective_removed={};records_removed={};cleared_divergence={}",
        truncated.boundary_tick,
        truncated.effective_removed,
        truncated.records_removed,
        truncated.cleared_divergence
    );
    assert_eq!(truncate_line, expect(&reference, "truncate_from_46"));

    let record_after = if rollback_input_history::record(&history, 46).is_none() {
        "nil"
    } else {
        "present"
    };
    assert_eq!(record_after, expect(&reference, "record_46_after_truncate"));
    let authoritative_after =
        if rollback_input_history::authoritative_record(&history, 46, 1).is_some() {
            "present"
        } else {
            "nil"
        };
    assert_eq!(
        authoritative_after,
        expect(&reference, "authoritative_46_after_truncate")
    );
    assert_eq!(
        diagnostics_line(&history),
        expect(&reference, "after_truncate")
    );
}
