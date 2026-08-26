//! Tests for `gc_sim::tuning`.
//!
//! The last case below ("the sim reads live values (a tuned knob changes
//! behavior)") drives a real `MatchState` via `sim_match::new`/
//! `sim_match::step` to confirm a tuned knob actually changes simulated
//! behavior.
//!
//! `Tuning` is an owned value, not a mutable global singleton (see
//! `crate::tuning`'s doc), so each case here starts from a fresh
//! `Tuning::new()` instead of resetting a shared instance between cases.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, PitchSize};
use gc_sim::tuning::Tuning;

#[test]
fn tuning_starts_at_defaults_and_clamps_sets_to_each_knobs_range() {
    let mut tuning = Tuning::new();
    assert_eq!(tuning.value("MOVE_ACCEL"), 1100.0);
    tuning.set("MOVE_ACCEL", 99999.0);
    assert_eq!(tuning.value("MOVE_ACCEL"), 2400.0);
    tuning.set("MOVE_ACCEL", -5.0);
    assert_eq!(tuning.value("MOVE_ACCEL"), 400.0);
    tuning.reset(None);
    assert_eq!(tuning.value("MOVE_ACCEL"), 1100.0);
}

#[test]
fn tuning_nudges_by_steps_and_resets_a_single_knob() {
    let mut tuning = Tuning::new();
    tuning.nudge("SHOT_WINDUP", 2.0);
    assert!((tuning.value("SHOT_WINDUP") - 0.17).abs() < 1e-9);
    assert!(!tuning.is_default("SHOT_WINDUP"));
    tuning.reset(Some("SHOT_WINDUP"));
    assert!(tuning.is_default("SHOT_WINDUP"));
}

#[test]
fn tuning_serializes_only_non_default_knobs_and_round_trips() {
    let mut tuning = Tuning::new();
    assert_eq!(tuning.serialize(), "");
    tuning.set("AI_STEAL_CD", 2.0);
    tuning.set("PUNT_MAX", 1500.0);
    let blob = tuning.serialize();
    tuning.reset(None);
    tuning.deserialize(&blob);
    assert_eq!(tuning.value("AI_STEAL_CD"), 2.0);
    assert_eq!(tuning.value("PUNT_MAX"), 1500.0);
    assert!(
        tuning.is_default("MOVE_ACCEL"),
        "untouched knobs stay default"
    );
    tuning.deserialize("garbage\nMOVE_ACCEL=abc\nPUNT_MAX=nope");
    assert!(tuning.is_default("PUNT_MAX"), "malformed lines are ignored");
}

#[test]
fn tuning_the_sim_reads_live_values_a_tuned_knob_changes_behavior() {
    // Zero the shot wind-up: a human shot must now release the same frame.
    let mut tune = Tuning::new();
    tune.set("SHOT_WINDUP", 0.0);

    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut s = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: None,
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    s.players[(s.controlled - 1) as usize].facing = Vec2::new(1.0, 0.0);
    sim_match::step(
        &mut s,
        0.016,
        StepInput::Legacy(MatchInput {
            shoot: true,
            ..MatchInput::default()
        }),
        None,
        &tune,
    );
    sim_match::step(
        &mut s,
        0.016,
        StepInput::Legacy(MatchInput::default()),
        None,
        &tune,
    ); // windup_timer==0 resolution frame
    assert!(
        s.owner.is_none(),
        "with SHOT_WINDUP=0 the shot releases immediately"
    );
}
