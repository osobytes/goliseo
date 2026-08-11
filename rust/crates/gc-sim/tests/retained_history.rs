//! Proves `gc_sim::retained_history::sample` reads the whole retained
//! client footprint, and that the counters travelling with it can tell a
//! window holding real content from one holding empty scaffolding.
//!
//! ## What this file is for, and what it is not
//!
//! It is not a budget gate — `tests/snapshot_headroom.rs` bands a measured
//! window against `Omp2RollbackBudgets`, and `tests/combat_load_fixtures.rs`
//! prices the worst-case combat campaigns. This file establishes the
//! property those gates and the wasm bridge all depend on: that the number
//! called `history_bytes` is the *combined* session-plus-event-timeline
//! total, and that a nonzero reading is not by itself evidence the
//! measurement measured anything.
//!
//! The second point is the one that has actually gone wrong. A speculative
//! window of 30 retained steps holding **zero events** reports a plausible,
//! nonzero, budget-comparable byte count in which none of the per-event
//! encoders were ever entered. [`retained_window_of_empty_wrappers_is_visible_as_such`]
//! constructs exactly that window on purpose and shows the byte total alone
//! cannot distinguish it from the real one, while `retained_event_count`
//! can. That is the demonstration that the "is this sample real?" check can
//! go red (AGENTS.md §9).

use std::time::Instant;

use gc_data::teams;
use gc_sim::combat;
use gc_sim::input_frame::{self, EdgeAction, HeldAction, InputSample, InputSampleOptions};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchState, PitchSize};
use gc_sim::retained_history::{self, RetainedHistorySample};
use gc_sim::rollback_events;
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_session;

/// Enough to fill the 31-boundary snapshot ring and the 30-step speculative
/// event window, so every reading below is a steady-state one rather than a
/// partially-warmed one.
const STEPPED_TICKS: i64 = 48;

/// Scripted input that produces real match and combat events — both sides
/// converge on the ball at a sprint holding the equipment action, pressing a
/// combat request every sixth tick and dashing on the third. Deliberately
/// the same shape `tests/snapshot_headroom.rs` uses, and for the same
/// reason: neutral input plays out as ten players standing still and
/// produces no events at all (which is precisely what
/// [`neutral_sample`] below is used to demonstrate).
fn scripted_sample(tick: i64, slot_index: i64) -> InputSample {
    let toward_opponent = if slot_index <= input_frame::HOME_SLOT_COUNT {
        1
    } else {
        -1
    };
    let held = HeldAction::Equipment.bit() | HeldAction::Sprint.bit();
    let edges = if tick % 6 == 0 {
        EdgeAction::EquipmentPressed.bit()
    } else if tick % 6 == 3 {
        EdgeAction::Dash.bit()
    } else {
        0
    };
    input_frame::new_sample(InputSampleOptions {
        move_x: Some(toward_opponent * 100),
        move_y: Some(if slot_index % 2 == 0 { 40 } else { -40 }),
        held: Some(held),
        edges: Some(edges),
    })
    .expect("the scripted sample is a valid input sample")
}

fn neutral_sample(_tick: i64, _slot_index: i64) -> InputSample {
    input_frame::neutral_sample()
}

fn new_state() -> MatchState {
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        // Long enough that `STEPPED_TICKS` cannot reach full time: a
        // finished match stops stepping and would under-fill the ring.
        duration: Some(120.0),
        max_goals: Some(9),
        seed: Some(19.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(sim_match::ownership_for_teams(home, away, None)),
    })
}

fn sources() -> [RollbackInputSource; 8] {
    let mut sources = [RollbackInputSource::Remote; 8];
    sources[0] = RollbackInputSource::Local;
    sources
}

fn event_tick_output(
    output: &rollback_session::RollbackTickOutput,
) -> rollback_events::RollbackEventTickOutput {
    rollback_events::RollbackEventTickOutput {
        tick: output.tick,
        start_boundary: output.start_boundary,
        end_boundary: output.end_boundary,
        events: output.events.clone(),
        combat_events: output.combat_events.clone(),
        state: rollback_events::RollbackOutputStateView {
            score: output.state.score,
            time_left: output.state.time_left,
            finished: output.state.finished,
        },
        finished: output.finished,
    }
}

/// One steady-state run: a real combat-active session plus the speculative
/// event timeline it feeds, stepped past a full ring with `input` on every
/// slot, sampled at the end. Also returns the wall-clock cost of one
/// [`retained_history::sample`] call on that steady-state window, so the
/// documented per-sample cost is measured rather than asserted from memory.
fn run(input: fn(i64, i64) -> InputSample) -> (RetainedHistorySample, f64) {
    let mut state = new_state();
    let combat_state = Some(combat::new_state(&mut state, None));
    let boundary_zero = match_snapshot::capture_owned(&state, combat_state.as_ref());
    let mut session = rollback_session::new(&boundary_zero, sources(), None, None);
    let mut timeline = rollback_events::new(&boundary_zero, None);
    let max_unconfirmed = timeline.max_unconfirmed_ticks;

    for tick in 0..STEPPED_TICKS {
        for slot_index in 1..=input_frame::SLOT_COUNT {
            rollback_session::add_authoritative(
                &mut session,
                tick,
                slot_index,
                input(tick, slot_index),
            )
            .expect("an in-window authoritative row is accepted");
        }
        let output =
            rollback_session::step(&mut session).expect("the session steps inside its duration");
        // Confirm exactly late enough to hold the speculative window at its
        // maximum: a client whose confirmations lag by the full window is
        // both reachable and the case a retention budget has to bound.
        if tick >= max_unconfirmed {
            rollback_events::confirm(&mut timeline, tick - max_unconfirmed);
        }
        let lookup = rollback_session::snapshot(&session, output.end_boundary);
        let step_input = rollback_events::RollbackEventStepInput {
            output: event_tick_output(&output),
            snapshot: lookup
                .snapshot
                .expect("the just-produced end boundary is retained"),
        };
        rollback_events::apply(&mut timeline, tick, tick, &[step_input])
            .expect("a contiguous speculative step inside the unconfirmed window is accepted");
    }

    // Warm once (the input-history encoding allocates), then time.
    let _ = retained_history::sample(&mut session, &timeline);
    let started = Instant::now();
    let sample = retained_history::sample(&mut session, &timeline);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    (sample, elapsed_ms)
}

#[test]
fn history_bytes_is_the_combined_session_and_event_timeline_total() {
    let (sample, elapsed_ms) = run(scripted_sample);

    // The definition itself: the combined figure, not either half.
    assert_eq!(
        sample.history_bytes,
        sample.session.total_bytes + sample.events.total_bytes,
        "history_bytes must be the combined session-plus-event-timeline total"
    );
    assert_eq!(
        sample.session.total_bytes,
        sample.session.input.total_bytes
            + sample.session.output_bytes
            + sample.session.snapshot_bytes,
        "the session half must be input + output + snapshots"
    );
    // Both halves must be genuinely present. A reading equal to either one
    // alone is the exact defect this module exists to prevent: the session
    // accounting structurally omits the retained speculative event window,
    // and the event accounting omits everything else.
    assert!(
        sample.history_bytes > sample.session.total_bytes,
        "history must exceed its session half (event timeline contributed {} bytes)",
        sample.events.total_bytes
    );
    assert!(
        sample.history_bytes > sample.events.total_bytes,
        "history must exceed its event half"
    );
    assert!(
        sample.session.snapshot_bytes > 0 && sample.session.input.total_bytes > 0,
        "a stepped session retains snapshots and input rows"
    );

    // Occupancy: a full ring and a full speculative window, holding REAL
    // events. Without the last assertion the byte total above could be 30
    // empty step wrappers -- see the next test.
    assert_eq!(
        sample.retained_boundary_count, 31,
        "a session stepped {STEPPED_TICKS} ticks retains a full snapshot ring"
    );
    assert_eq!(sample.peak_retained_boundary_count, 31);
    assert!(sample.peak_snapshot_bytes >= sample.session.snapshot_bytes);
    assert_eq!(
        sample.retained_step_count, 30,
        "the speculative window is held full by the confirmation schedule"
    );
    assert!(
        sample.retained_event_count > 0,
        "the retained window must hold real events, not just {} empty step wrappers",
        sample.retained_step_count
    );
    assert_eq!(sample.oldest_boundary_tick, Some(STEPPED_TICKS - 30));
    assert_eq!(sample.latest_boundary_tick, Some(STEPPED_TICKS));

    // Visible under `cargo test -- --nocapture`: the measured per-sample
    // cost `retained_history`'s module doc quotes, and the retained content
    // behind the byte figure.
    println!(
        "GC_RETAINED_HISTORY|history_bytes={}|session={}|events={}|steps={}|events_retained={}\
         |sample_ms={elapsed_ms:.3}",
        sample.history_bytes,
        sample.session.total_bytes,
        sample.events.total_bytes,
        sample.retained_step_count,
        sample.retained_event_count
    );
}

#[test]
fn retained_window_of_empty_wrappers_is_visible_as_such() {
    let (real, _) = run(scripted_sample);
    let (wrappers, _) = run(neutral_sample);

    // The scaffolding-only run retains exactly as many steps, and reports a
    // perfectly plausible nonzero byte total. Nothing about the bytes says
    // it priced no event at all.
    assert_eq!(wrappers.retained_step_count, real.retained_step_count);
    assert!(
        wrappers.events.total_bytes > 0,
        "empty step wrappers still cost bytes -- which is exactly why the byte \
         total alone cannot certify a sample"
    );
    assert!(
        wrappers.history_bytes > 0,
        "and the combined total stays comfortably nonzero"
    );
    // Nonzero is not the claim. The claim is that the two figures are of the
    // same ORDER -- a gate reading only bytes would see nothing suspicious
    // about the scaffolding run -- and that claim is what makes the counters
    // load-bearing rather than decorative. Measured at ~89% (822,481 against
    // 927,304); asserted loosely at half, because the point is to catch a
    // collapse that would quietly turn this file's argument into an
    // anecdote, not to pin a ratio that legitimately drifts as the encoders
    // change.
    assert!(
        wrappers.history_bytes > real.history_bytes / 2,
        "the scaffolding-only window ({}) must stay within an order of the real one ({}), \
         or the bytes would in fact distinguish them and this test's premise is stale",
        wrappers.history_bytes,
        real.history_bytes
    );

    // The counter is what separates them.
    assert_eq!(
        wrappers.retained_event_count, 0,
        "ten players standing still produce no events at all"
    );
    assert!(real.retained_event_count > 0);
    println!(
        "GC_RETAINED_HISTORY|wrappers_only|history_bytes={}|events_retained={}|real_events={}",
        wrappers.history_bytes, wrappers.retained_event_count, real.retained_event_count
    );
}
