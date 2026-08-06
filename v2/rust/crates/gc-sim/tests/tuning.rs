//! Port of `spec/sim/tuning_spec.lua`.
//!
//! The Lua spec's last case ("the sim reads live values (a tuned knob
//! changes behavior)") drives a real `MatchState` via
//! `sim.match.new`/`match.step`. `sim/match.lua` is another agent's module
//! and is still an unported placeholder (`gc_sim::r#match`), so that case is
//! ported as `#[ignore]` below with a note.
//!
//! The Lua original is a mutable module-level singleton; this port's
//! `Tuning` is an owned value instead (see `crate::tuning`'s doc), so each
//! case here starts from `Tuning::new()` in place of the Lua spec's
//! `tuning.reset()` calls that re-synchronize the shared singleton between
//! cases.

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
    tuning.set("PUNT_MAX", 700.0);
    let blob = tuning.serialize();
    tuning.reset(None);
    tuning.deserialize(&blob);
    assert_eq!(tuning.value("AI_STEAL_CD"), 2.0);
    assert_eq!(tuning.value("PUNT_MAX"), 700.0);
    assert!(
        tuning.is_default("MOVE_ACCEL"),
        "untouched knobs stay default"
    );
    tuning.deserialize("garbage\nMOVE_ACCEL=abc\nPUNT_MAX=nope");
    assert!(tuning.is_default("PUNT_MAX"), "malformed lines are ignored");
}

/// Blocked on `sim::match` (`sim/match.lua`), an unported placeholder owned
/// by another agent.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn tuning_the_sim_reads_live_values_a_tuned_knob_changes_behavior() {
    unimplemented!("requires sim::match::new/step");
}
