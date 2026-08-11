//! Species-to-match stat translation.
//!
//! Pure species-to-match translation. Attribute deltas are the first of the
//! readability cap's two modifier layers; the arena layer is the reserved
//! second.

use gc_data::players::StatBlock;
use gc_data::species::{SimVerb, SpeciesData};

const MIN_STAT: i64 = 0;
const MAX_STAT: i64 = 10;

fn clamp_stat(value: i64) -> i64 {
    value.clamp(MIN_STAT, MAX_STAT)
}

fn neutral_hook(owned_verb: SimVerb, seam: SimVerb) -> f64 {
    if owned_verb == seam {
        return 0.0;
    }
    0.0
}

/// Apply one additive species modifier vector without mutating authored
/// player data.
#[must_use]
pub fn apply(stat_block: StatBlock, species_data: &SpeciesData) -> StatBlock {
    let modifiers = species_data.modifiers;
    StatBlock {
        pace: clamp_stat(stat_block.pace + modifiers.pace),
        strength: clamp_stat(stat_block.strength + modifiers.strength),
        technique: clamp_stat(stat_block.technique + modifiers.technique),
        stamina: clamp_stat(stat_block.stamina + modifiers.stamina),
        mental: clamp_stat(stat_block.mental + modifiers.mental),
    }
}

// Named verb hooks deliberately return neutral values. Future skill work can
// bind skill effects here while the match keeps one stable lookup at each
// action seam.

/// Extra jump reach, in pixels, from the owned verb.
#[must_use]
pub fn jump_reach(owned_verb: SimVerb) -> f64 {
    neutral_hook(owned_verb, SimVerb::Jump)
}

/// Extra jump lift, in pixels, from the owned verb.
#[must_use]
pub fn jump_lift(owned_verb: SimVerb) -> f64 {
    neutral_hook(owned_verb, SimVerb::Jump)
}

/// Extra collision reach, in pixels, from the owned verb.
#[must_use]
pub fn collision_reach(owned_verb: SimVerb) -> f64 {
    neutral_hook(owned_verb, SimVerb::Collision)
}

/// Burst speed multiplier from the owned verb.
#[must_use]
pub fn burst_speed(owned_verb: SimVerb) -> f64 {
    1.0 + neutral_hook(owned_verb, SimVerb::Burst)
}

/// Extra dribble protection, in pixels, from the owned verb.
#[must_use]
pub fn dribble_protection(owned_verb: SimVerb) -> f64 {
    neutral_hook(owned_verb, SimVerb::Dribble)
}

/// Extra block reach, in pixels, from the owned verb.
#[must_use]
pub fn block_reach(owned_verb: SimVerb) -> f64 {
    neutral_hook(owned_verb, SimVerb::Block)
}

/// Link pass speed multiplier from the owned verb.
#[must_use]
pub fn link_pass_speed(owned_verb: SimVerb) -> f64 {
    1.0 + neutral_hook(owned_verb, SimVerb::Link)
}
