//! Port of `spec/sim/fixed_clock_spec.lua`.
//!
//! The Lua spec's "keeps gameplay state equivalent across 30/60/120 Hz and
//! irregular render cadences" case drives a real `MatchState` via
//! `sim.match.new`/`match.step`. `sim/match.lua` is another agent's module
//! and is still an unported placeholder (`gc_sim::r#match`), so that case is
//! ported as `#[ignore]` below with a note; every other case only needs
//! `fixed_clock` itself and is ported and passing.

use gc_sim::fixed_clock;

/// Drive the clock with a render-cadence pattern, recording every consumed
/// tick's provided input (which the Lua fixture arranges to equal the tick
/// number itself).
fn drive(pattern: &[f64]) -> (fixed_clock::FixedClockState, Vec<u64>) {
    let mut clock = fixed_clock::new();
    let mut consumed = Vec::new();
    for &dt in pattern {
        fixed_clock::advance(
            &mut clock,
            dt,
            |tick| tick,
            |tick, input| {
                assert_eq!(*input, tick, "provider input belongs to the consumed tick");
                consumed.push(tick);
                true
            },
        );
    }
    (clock, consumed)
}

#[test]
fn fixed_simulation_clock_numbers_inputs_from_zero_and_advances_exact_ticks_at_common_render_cadences()
 {
    let (at_30, input_30) = drive(&vec![1.0 / 30.0; 30]);
    let (at_60, input_60) = drive(&vec![1.0 / 60.0; 60]);
    let (at_120, input_120) = drive(&vec![1.0 / 120.0; 120]);

    assert_eq!(at_30.tick, 60);
    assert_eq!(at_60.tick, 60);
    assert_eq!(at_120.tick, 60);
    assert_eq!(input_30.len(), 60);
    assert_eq!(input_60.len(), 60);
    assert_eq!(input_120.len(), 60);
    assert_eq!(input_30[0], 0);
    assert_eq!(input_30[input_30.len() - 1], 59);
}

/// Blocked on `sim::match` (`sim/match.lua`), an unported placeholder owned
/// by another agent.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn fixed_simulation_clock_keeps_gameplay_state_equivalent_across_cadences() {
    unimplemented!("requires sim::match::new/step");
}

#[test]
fn fixed_simulation_clock_reports_zero_and_multiple_tick_render_updates() {
    let mut clock = fixed_clock::new();
    let mut calls = 0;

    let first = fixed_clock::advance(
        &mut clock,
        1.0 / 120.0,
        |tick| tick,
        |_, _| {
            calls += 1;
            true
        },
    );
    assert_eq!(first.ticks, 0);
    assert_eq!(calls, 0);

    let second = fixed_clock::advance(
        &mut clock,
        1.0 / 120.0,
        |tick| tick,
        |_, _| {
            calls += 1;
            true
        },
    );
    assert_eq!(second.ticks, 1);
    assert_eq!(second.first_tick, Some(0));
    assert_eq!(second.last_tick, Some(0));

    let third = fixed_clock::advance(
        &mut clock,
        1.0 / 20.0,
        |tick| tick,
        |_, _| {
            calls += 1;
            true
        },
    );
    assert_eq!(third.ticks, 3);
    assert_eq!(third.first_tick, Some(1));
    assert_eq!(third.last_tick, Some(3));
    assert_eq!(calls, 4);
}

#[test]
fn fixed_simulation_clock_drops_only_whole_excess_tick_debt_and_keeps_the_fractional_remainder() {
    let mut clock = fixed_clock::new();
    let result = fixed_clock::advance(
        &mut clock,
        fixed_clock::TICK_SECONDS * (fixed_clock::MAX_TICKS_PER_UPDATE as f64 + 3.5),
        |tick| tick,
        |_, _| true,
    );
    assert_eq!(result.ticks, fixed_clock::MAX_TICKS_PER_UPDATE);
    assert_eq!(result.dropped_ticks, 3);
    assert_eq!(clock.dropped_ticks, 3);
    assert_eq!(clock.overloads, 1);
    assert!((clock.accumulator - fixed_clock::TICK_SECONDS / 2.0).abs() < 1e-9);

    let remainder = fixed_clock::advance(
        &mut clock,
        fixed_clock::TICK_SECONDS / 2.0,
        |tick| tick,
        |_, _| true,
    );
    assert_eq!(remainder.ticks, 1);
    assert_eq!(
        remainder.first_tick,
        Some(u64::from(fixed_clock::MAX_TICKS_PER_UPDATE))
    );
    assert!(clock.accumulator.abs() < 1e-9);
}

#[test]
fn fixed_simulation_clock_lets_a_step_callback_stop_a_finished_simulation_without_retaining_debt() {
    let mut clock = fixed_clock::new();
    let result = fixed_clock::advance(&mut clock, 1.0 / 10.0, |tick| tick, |_, _| false);
    assert_eq!(result.ticks, 1);
    assert!(result.stopped);
    assert_eq!(clock.tick, 1);
    assert!(clock.accumulator.abs() < 1e-9);
}
