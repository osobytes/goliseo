//! Stat-to-physical-quantity conversions for `gc_sim`.
//!
//! Derives concrete physical quantities from a player's effective stat block.
//! `species::apply` owns the first attribute-modifying readability layer and
//! match construction calls it exactly once before these mappings. Arenas own
//! the reserved second layer; this module must not stack either layer itself.

use gc_data::players::StatBlock;
use std::f64::consts::PI;

const BASE_MOVE: f64 = 60.0; // px/s at pace 0
const MOVE_PER_PACE: f64 = 20.0; // px/s per pace point

const BASE_SHOT: f64 = 150.0; // px/s at strength 0
const SHOT_PER_STRENGTH: f64 = 50.0; // px/s per strength point

const MAX_EXECUTION_ERROR_RADIANS: f64 = PI / 15.0; // 12 degrees at technique 0

fn execution_error_for_technique(technique: f64) -> f64 {
    let technique = technique.clamp(0.0, 10.0);
    (1.0 - technique / 10.0) * MAX_EXECUTION_ERROR_RADIANS
}

/// Move speed, in px/s.
#[must_use]
pub fn move_speed(s: StatBlock) -> f64 {
    BASE_MOVE + s.pace as f64 * MOVE_PER_PACE
}

/// Shot speed, in px/s.
#[must_use]
pub fn shot_speed(s: StatBlock) -> f64 {
    BASE_SHOT + s.strength as f64 * SHOT_PER_STRENGTH
}

// Dribble control: how tightly a carrier keeps the ball at their feet as they
// move. Technique is touch quality — a higher-technique player takes cleaner,
// closer touches, so the ball rides less far ahead and is harder to nick.
const BASE_DRIBBLE: f64 = 0.25; // control factor (0..1) at technique 0
const DRIBBLE_PER_TECH: f64 = 0.065; // extra control per technique point

/// Dribble control factor, 0..1 (higher = ball stays tighter to the feet).
#[must_use]
pub fn dribble(s: StatBlock) -> f64 {
    (BASE_DRIBBLE + s.technique as f64 * DRIBBLE_PER_TECH).min(1.0)
}

// Aerial actions reuse the authored five-stat vocabulary. Pace determines
// whether a player reaches the ball; these factors resolve contact quality.

/// Reception quality, 0..1.
#[must_use]
pub fn first_touch(s: StatBlock) -> f64 {
    ((s.technique as f64 * 0.75 + s.mental as f64 * 0.25) / 10.0).min(1.0)
}

/// Header contact quality, 0..1.
#[must_use]
pub fn header(s: StatBlock) -> f64 {
    ((s.technique as f64 * 0.35 + s.mental as f64 * 0.35 + s.strength as f64 * 0.30) / 10.0)
        .min(1.0)
}

/// Volley contact quality, 0..1.
#[must_use]
pub fn volley(s: StatBlock) -> f64 {
    ((s.technique as f64 * 0.65 + s.mental as f64 * 0.20 + s.strength as f64 * 0.15) / 10.0)
        .min(1.0)
}

/// Bicycle-kick contact quality, 0..1.
#[must_use]
pub fn bicycle(s: StatBlock) -> f64 {
    ((s.technique as f64 * 0.70 + s.mental as f64 * 0.20 + s.strength as f64 * 0.10) / 10.0)
        .min(1.0)
}

// Sprint: the hold-to-run burst. Stamina sets how long a full tank lasts.
const BASE_SPRINT: f64 = 2.2; // seconds of sprint at stamina 0
const SPRINT_PER_STAMINA: f64 = 0.25; // extra seconds per stamina point

/// Sprint duration, in seconds.
#[must_use]
pub fn sprint_duration(s: StatBlock) -> f64 {
    BASE_SPRINT + s.stamina as f64 * SPRINT_PER_STAMINA
}

// Outfield behavior attributes remain pure derivations from the canonical
// five-stat block. They do not add authored attributes or alter match
// behavior until downstream AI mechanics consume them.

/// Mental-led decision-scanning rate, 0..1.
#[must_use]
pub fn scan_rate(s: StatBlock) -> f64 {
    ((s.mental as f64 * 0.75 + s.stamina as f64 * 0.25) / 10.0).clamp(0.0, 1.0)
}

/// Mental-led action-selection composure, 0..1.
#[must_use]
pub fn composure(s: StatBlock) -> f64 {
    (s.mental as f64 / 10.0).clamp(0.0, 1.0)
}

/// Mental-led defensive discipline, 0..1.
#[must_use]
pub fn press_discipline(s: StatBlock) -> f64 {
    (s.mental as f64 / 10.0).clamp(0.0, 1.0)
}

/// Pace-led willingness to make an off-ball run, 0..1.
#[must_use]
pub fn run_drive(s: StatBlock) -> f64 {
    ((s.pace as f64 * 0.6 + s.mental as f64 * 0.4) / 10.0).clamp(0.0, 1.0)
}

/// Reconstruct the authored-stat derivation in its original operation order
/// from the two concrete scalars a match snapshot already retains, so
/// rollback restores reproduce every IEEE-754 bit for canonical stats.
#[must_use]
pub fn run_drive_from_match(move_speed: f64, composure: f64) -> f64 {
    let pace = (move_speed - BASE_MOVE) / MOVE_PER_PACE;
    let mental = composure * 10.0;
    ((pace * 0.6 + mental * 0.4) / 10.0).clamp(0.0, 1.0)
}

/// Maximum angular execution error, in radians (0..pi/15, i.e. 0..12 degrees).
#[must_use]
pub fn execution_error(s: StatBlock) -> f64 {
    execution_error_for_technique(s.technique as f64)
}

/// Reverse `first_touch` and `composure` in the same order they were
/// originally computed, so kick execution can consume the effective
/// technique without adding duplicate immutable state to snapshots.
#[must_use]
pub fn execution_error_from_outfield(first_touch: f64, composure: f64) -> f64 {
    let mental = composure * 10.0;
    let technique = (first_touch * 10.0 - mental * 0.25) / 0.75;
    execution_error_for_technique(technique)
}

// Keeper-specific derivations. Mental represents composure and positioning
// (reach), pace contributes diving range, and technique controls clean
// handling. Defensive ability remains derived from the canonical attributes
// rather than authored separately.
const BASE_REACH: f64 = 22.0; // dive radius (px) at mental 0
const REACH_PER_MENTAL: f64 = 6.0; // px per mental point
const REACH_PER_PACE: f64 = 2.0; // px per pace point (diving range)

// Conservative first-pass positioning depth. Canonical 0..10 stats produce
// 18..58 px, leaving later fixed-seed calibration to the goalkeeper
// milestone.
const BASE_KEEPER_AGGRESSION: f64 = 18.0; // px at pace 0 and mental 0
const KEEPER_AGGRESSION_PER_PACE: f64 = 2.0; // px per pace point
const KEEPER_AGGRESSION_PER_MENTAL: f64 = 2.0; // px per mental point

/// How far the keeper can get a hand to a shot, in pixels.
#[must_use]
pub fn keeper_reach(s: StatBlock) -> f64 {
    BASE_REACH + s.mental as f64 * REACH_PER_MENTAL + s.pace as f64 * REACH_PER_PACE
}

/// Clean-handling factor, 0..1 (higher = catches harder shots).
#[must_use]
pub fn keeper_handling(s: StatBlock) -> f64 {
    (s.technique as f64 / 10.0).min(1.0)
}

/// Mental-led shot-reading quality, 0..1.
#[must_use]
pub fn keeper_anticipation(s: StatBlock) -> f64 {
    (s.mental as f64 / 10.0).clamp(0.0, 1.0)
}

/// Positive positioning-depth cap, in pixels; 18..58 for canonical stats.
#[must_use]
pub fn keeper_aggression(s: StatBlock) -> f64 {
    BASE_KEEPER_AGGRESSION
        + s.pace as f64 * KEEPER_AGGRESSION_PER_PACE
        + s.mental as f64 * KEEPER_AGGRESSION_PER_MENTAL
}

/// Technique-led hand-distribution accuracy, 0..1.
#[must_use]
pub fn keeper_distribution_accuracy(s: StatBlock) -> f64 {
    (s.technique as f64 / 10.0).clamp(0.0, 1.0)
}
