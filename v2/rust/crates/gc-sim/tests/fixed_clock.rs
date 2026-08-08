//! Port of `spec/sim/fixed_clock_spec.lua`.
//!
//! The Lua spec's "keeps gameplay state equivalent across 30/60/120 Hz and
//! irregular render cadences" case drives a real `MatchState` via
//! `sim.match.new`/`match.step`. `sim::match` (`gc_sim::r#match`) is now
//! fully ported, so that case builds a real fixture too.

use gc_sim::fixed_clock;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize};
use gc_sim::tuning::Tuning;

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

fn play_script(pattern: &[f64], tune: &Tuning) -> (MatchState, fixed_clock::FixedClockState) {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(41.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    let mut clock = fixed_clock::new();
    for &dt in pattern {
        fixed_clock::advance(
            &mut clock,
            dt,
            |_tick| MatchInput::default(),
            |_tick, input: &MatchInput| {
                sim_match::step(
                    &mut state,
                    fixed_clock::TICK_SECONDS,
                    StepInput::Legacy(*input),
                    None,
                    tune,
                );
                !state.finished
            },
        );
    }
    (state, clock)
}

fn assert_same_state(a: &MatchState, b: &MatchState) {
    assert_eq!(a.time_left, b.time_left, "time left");
    assert_eq!(a.score.home, b.score.home, "home score");
    assert_eq!(a.score.away, b.score.away, "away score");
    assert_eq!(a.owner, b.owner, "ball owner");
    assert!((a.ball.x - b.ball.x).abs() < 1e-9, "ball x");
    assert!((a.ball.y - b.ball.y).abs() < 1e-9, "ball y");
    assert!((a.ball_z - b.ball_z).abs() < 1e-9, "ball z");
    assert!(
        (a.ball_vel.x - b.ball_vel.x).abs() < 1e-9,
        "ball velocity x"
    );
    assert!(
        (a.ball_vel.y - b.ball_vel.y).abs() < 1e-9,
        "ball velocity y"
    );
    for (i, (player, other)) in a.players.iter().zip(b.players.iter()).enumerate() {
        assert!((player.pos.x - other.pos.x).abs() < 1e-9, "player x {i}");
        assert!((player.pos.y - other.pos.y).abs() < 1e-9, "player y {i}");
        assert!(
            (player.vel.x - other.vel.x).abs() < 1e-9,
            "player velocity x {i}"
        );
        assert!(
            (player.vel.y - other.vel.y).abs() < 1e-9,
            "player velocity y {i}"
        );
    }
}

#[test]
fn fixed_simulation_clock_keeps_gameplay_state_equivalent_across_cadences() {
    let tune = Tuning::new();
    let at_30: Vec<f64> = vec![1.0 / 30.0; 30];
    let regular: Vec<f64> = vec![1.0 / 60.0; 60];
    let at_120: Vec<f64> = vec![1.0 / 120.0; 120];
    let mut irregular: Vec<f64> = Vec::with_capacity(60);
    for _ in 0..15 {
        irregular.push(1.0 / 120.0);
        irregular.push(1.0 / 40.0);
        irregular.push(1.0 / 120.0);
        irregular.push(1.0 / 40.0);
    }

    let (at_30_state, at_30_clock) = play_script(&at_30, &tune);
    let (regular_state, regular_clock) = play_script(&regular, &tune);
    let (at_120_state, at_120_clock) = play_script(&at_120, &tune);
    let (irregular_state, irregular_clock) = play_script(&irregular, &tune);
    assert_eq!(at_30_clock.tick, 60);
    assert_eq!(regular_clock.tick, 60);
    assert_eq!(at_120_clock.tick, 60);
    assert_eq!(irregular_clock.tick, 60);
    assert_same_state(&regular_state, &at_30_state);
    assert_same_state(&regular_state, &at_120_state);
    assert_same_state(&regular_state, &irregular_state);
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
