//! Port of `sim/rating.lua`.
//!
//! Frozen, position-aware squad strength estimate. This is deliberately a
//! small content-independent ordering function, not a prediction formula for
//! match results.

use gc_data::players::{PlayerData, Position};
use indexmap::IndexMap;

/// Per-position weights applied to a player's five canonical stats.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RatingWeights {
    pace: f64,
    strength: f64,
    technique: f64,
    stamina: f64,
    mental: f64,
}

// Frozen before table_signal/upset_rate per season_metrics red-team #11. Do
// not tune these weights against validation output or change them alongside
// content/metric bands; any future edit resets the downstream baselines.
const KEEPER_WEIGHTS: RatingWeights = RatingWeights {
    pace: 0.20,
    strength: 0.10,
    technique: 0.25,
    stamina: 0.10,
    mental: 0.35,
};
const DEFENDER_WEIGHTS: RatingWeights = RatingWeights {
    pace: 0.15,
    strength: 0.30,
    technique: 0.15,
    stamina: 0.20,
    mental: 0.20,
};
const MIDFIELDER_WEIGHTS: RatingWeights = RatingWeights {
    pace: 0.20,
    strength: 0.15,
    technique: 0.30,
    stamina: 0.20,
    mental: 0.15,
};
const FORWARD_WEIGHTS: RatingWeights = RatingWeights {
    pace: 0.30,
    strength: 0.25,
    technique: 0.30,
    stamina: 0.10,
    mental: 0.05,
};

fn weights_for(position: Position) -> RatingWeights {
    match position {
        Position::Keeper => KEEPER_WEIGHTS,
        Position::Defender => DEFENDER_WEIGHTS,
        Position::Midfielder => MIDFIELDER_WEIGHTS,
        Position::Forward => FORWARD_WEIGHTS,
    }
}

/// Each starter contributes a 0..10 weighted score chosen by their authored
/// position. A legal five-player roster therefore rates from 0..50.
///
/// # Panics
///
/// Panics if `roster` is not exactly five distinct starters with exactly one
/// keeper among them, or if `roster` names a player missing from
/// `players_by_id` — these are authored-content invariants, not recoverable
/// runtime failures.
#[must_use]
pub fn squad(roster: &[&str], players_by_id: &IndexMap<&str, PlayerData>) -> f64 {
    assert!(
        roster.len() == 5,
        "squad rating needs exactly five starters"
    );

    let mut total = 0.0_f64;
    let mut keeper_count = 0;
    let mut seen: Vec<&str> = Vec::with_capacity(5);
    for &id in roster {
        assert!(
            !seen.contains(&id),
            "squad rating cannot count a starter twice: {id}"
        );
        seen.push(id);

        let player = players_by_id
            .get(id)
            .unwrap_or_else(|| panic!("unknown player in squad rating: {id}"));
        let weights = weights_for(player.position);
        let stats = player.stats;
        total = total
            + stats.pace as f64 * weights.pace
            + stats.strength as f64 * weights.strength
            + stats.technique as f64 * weights.technique
            + stats.stamina as f64 * weights.stamina
            + stats.mental as f64 * weights.mental;
        if player.position == Position::Keeper {
            keeper_count += 1;
        }
    }

    assert!(keeper_count == 1, "squad rating needs exactly one keeper");
    total
}
