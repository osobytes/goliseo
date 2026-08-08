//! Port of `sim/rating_validation.lua`.
//!
//! Deterministic validation harness for the frozen squad rating. Every
//! unordered squad pair plays both home orientations on common seeds. The
//! home side gets the human-proxy bot in each leg, so each squad receives
//! the same side/proxy treatment. Results remain relative to that proxy,
//! never predictions of humans.
//!
//! ## Test seam (no function mocking in Rust)
//!
//! `spec/sim/rating_validation_spec.lua` monkey-patches `headless.run_match`
//! to observe every call and return a scripted, rank-based winner instead of
//! playing a real match — the same seam problem [`crate::headless`]'s own
//! module doc names (see "Test seams" there). Rust has no runtime function
//! replacement, so [`run_with`] takes the match-runner as a parameter;
//! [`run`] is just `run_with` closed over the real
//! [`crate::headless::run_match`]. The spec's assertions about call order,
//! seed pairing and side-swap are preserved by asserting on the calls a
//! fake passed to `run_with` observed, instead of a mocked global.

use crate::headless::{self, HeadlessBot, HeadlessOpts, MatchResult, Winner};
use gc_data::players::PlayerData;
use gc_data::teams::TeamData;
use indexmap::IndexMap;

/// A squad entered into the validation set, with its frozen rating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatedSquad {
    /// The squad's roster.
    pub team: TeamData,
    /// `crate::rating::squad`'s frozen position-weighted sum, `0..50`.
    pub rating: f64,
}

/// One unordered squad pair's two-leg outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatingPairResult {
    /// The higher-rated squad.
    pub higher: RatedSquad,
    /// The lower-rated squad.
    pub lower: RatedSquad,
    /// `higher.rating - lower.rating`.
    pub gap: f64,
    /// Total games played for this pair (`seeds_per_leg * 2`).
    pub games: i64,
    /// Games the higher-rated squad won.
    pub wins: i64,
    /// Drawn games.
    pub draws: i64,
    /// Games the higher-rated squad lost.
    pub losses: i64,
    /// Decisive-games-only win rate; `None` when every game drew.
    pub win_rate: Option<f64>,
    /// `(wins + draws * 0.5) / games`; win = 1, draw = 0.5, loss = 0.
    pub score_share: f64,
}

/// A full validation run's result.
#[derive(Clone, Debug, PartialEq)]
pub struct RatingValidationResult {
    /// Seeds played per leg (so `2 * seeds_per_leg` games per pair).
    pub seeds_per_leg: i64,
    /// Every squad, sorted by rating ascending.
    pub squads: Vec<RatedSquad>,
    /// Every unordered pair, sorted by gap ascending (ties broken by the
    /// higher squad's team id).
    pub pairs: Vec<RatingPairResult>,
    /// Aggregate higher-rated wins across every pair.
    pub wins: i64,
    /// Aggregate draws across every pair.
    pub draws: i64,
    /// Aggregate higher-rated losses across every pair.
    pub losses: i64,
    /// Aggregate decisive-games-only win rate; `None` when every game drew.
    pub win_rate: Option<f64>,
    /// Aggregate score share across every pair.
    pub score_share: f64,
    /// How many pairs had `score_share > 0.5`.
    pub above_half_pairs: i64,
    /// OLS score-share change per rating point.
    pub slope: f64,
}

// All fixtures use existing player ids/stat blocks and the same
// role-aligned 1-1-2 shape. Only lineup strength varies.
static SQUADS: &[TeamData] = &[
    TeamData {
        id: "rating_prospects",
        name: "Prospects",
        color: [0.4, 0.4, 0.4],
        formation: "1-1-2",
        roster: &["gax_oru", "veil_nyx", "morv", "krag", "tox_vren"],
        squad: None,
    },
    TeamData {
        id: "rating_developing",
        name: "Developing",
        color: [0.4, 0.6, 0.7],
        formation: "1-1-2",
        roster: &["gax_oru", "brakka", "tib_quell", "krag", "tox_vren"],
        squad: None,
    },
    TeamData {
        id: "rating_balanced",
        name: "Balanced",
        color: [0.5, 0.7, 0.5],
        formation: "1-1-2",
        roster: &["ozzo", "veil_nyx", "sela_dwin", "mika_olu", "tox_vren"],
        squad: None,
    },
    TeamData {
        id: "rating_contenders",
        name: "Contenders",
        color: [0.8, 0.7, 0.3],
        formation: "1-1-2",
        roster: &["ozzo", "brakka", "rok_tann", "zyro_vex", "tox_vren"],
        squad: None,
    },
    TeamData {
        id: "rating_elite",
        name: "Elite",
        color: [0.9, 0.4, 0.3],
        formation: "1-1-2",
        roster: &["ozzo", "drell", "rok_tann", "zyro_vex", "mika_olu"],
        squad: None,
    },
];

fn players_by_id() -> IndexMap<&'static str, PlayerData> {
    gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect()
}

fn record_outcome(result: &MatchResult, higher_side: Winner, row: &mut RatingPairResult) {
    match result.winner {
        None => row.draws += 1,
        Some(w) if w == higher_side => row.wins += 1,
        Some(_) => row.losses += 1,
    }
}

fn score_share_slope(pairs: &[RatingPairResult]) -> f64 {
    let mut mean_gap = 0.0;
    let mut mean_share = 0.0;
    for pair in pairs {
        mean_gap += pair.gap;
        mean_share += pair.score_share;
    }
    mean_gap /= pairs.len() as f64;
    mean_share /= pairs.len() as f64;

    let mut covariance = 0.0;
    let mut variance = 0.0;
    for pair in pairs {
        let centered_gap = pair.gap - mean_gap;
        covariance += centered_gap * (pair.score_share - mean_share);
        variance += centered_gap * centered_gap;
    }
    if variance > 0.0 {
        covariance / variance
    } else {
        0.0
    }
}

/// Run the validation harness against the real simulation.
///
/// # Panics
///
/// Panics if `seeds_per_leg < 1`.
#[must_use]
pub fn run(seeds_per_leg: i64) -> RatingValidationResult {
    run_with(seeds_per_leg, headless::run_match)
}

/// [`run`] with the match runner as a parameter — see the module doc's
/// "Test seam" section. `run_match` is called exactly where the Lua
/// original calls `headless.run_match`, in the same order and with the
/// same options, so a fake can assert on it exactly as the spec's mock did.
///
/// # Panics
///
/// Panics if `seeds_per_leg < 1`.
#[must_use]
pub fn run_with<F>(seeds_per_leg: i64, mut run_match: F) -> RatingValidationResult
where
    F: FnMut(&HeadlessOpts<'_>) -> MatchResult,
{
    assert!(seeds_per_leg >= 1, "seed count must be a positive integer");

    let by_id = players_by_id();
    let mut squads: Vec<RatedSquad> = SQUADS
        .iter()
        .map(|team| RatedSquad {
            team: *team,
            rating: crate::rating::squad(team.roster, &by_id),
        })
        .collect();
    squads.sort_by(|a, b| a.rating.partial_cmp(&b.rating).expect("ratings are finite"));

    let mut pairs: Vec<RatingPairResult> = Vec::new();
    let mut all_wins = 0i64;
    let mut all_draws = 0i64;
    let mut all_losses = 0i64;
    for lower_index in 0..squads.len().saturating_sub(1) {
        for higher_index in (lower_index + 1)..squads.len() {
            let lower = squads[lower_index];
            let higher = squads[higher_index];
            let mut row = RatingPairResult {
                higher,
                lower,
                gap: higher.rating - lower.rating,
                games: seeds_per_leg * 2,
                wins: 0,
                draws: 0,
                losses: 0,
                win_rate: None,
                score_share: 0.0,
            };
            for seed in 1..=seeds_per_leg {
                let higher_home = run_match(&HeadlessOpts {
                    seed: seed as f64,
                    home: Some(&higher.team),
                    away: Some(&lower.team),
                    players_by_id: Some(&by_id),
                    bot: Some(HeadlessBot::Home),
                    ..Default::default()
                });
                record_outcome(&higher_home, Winner::Home, &mut row);

                let lower_home = run_match(&HeadlessOpts {
                    seed: seed as f64,
                    home: Some(&lower.team),
                    away: Some(&higher.team),
                    players_by_id: Some(&by_id),
                    bot: Some(HeadlessBot::Home),
                    ..Default::default()
                });
                record_outcome(&lower_home, Winner::Away, &mut row);
            }

            let decisions = row.wins + row.losses;
            row.win_rate = (decisions > 0).then(|| row.wins as f64 / decisions as f64);
            row.score_share = (row.wins as f64 + row.draws as f64 * 0.5) / row.games as f64;
            all_wins += row.wins;
            all_draws += row.draws;
            all_losses += row.losses;
            pairs.push(row);
        }
    }
    pairs.sort_by(|a, b| {
        if a.gap == b.gap {
            a.higher.team.id.cmp(b.higher.team.id)
        } else {
            a.gap.partial_cmp(&b.gap).expect("gaps are finite")
        }
    });

    let decisions = all_wins + all_losses;
    let games = all_wins + all_draws + all_losses;
    let above_half_pairs = pairs.iter().filter(|p| p.score_share > 0.5).count() as i64;
    let slope = score_share_slope(&pairs);
    RatingValidationResult {
        seeds_per_leg,
        squads,
        pairs,
        wins: all_wins,
        draws: all_draws,
        losses: all_losses,
        win_rate: (decisions > 0).then(|| all_wins as f64 / decisions as f64),
        score_share: (all_wins as f64 + all_draws as f64 * 0.5) / games as f64,
        above_half_pairs,
        slope,
    }
}

/// Render a human-readable validation report.
#[must_use]
pub fn report(result: &RatingValidationResult) -> String {
    let mut out = vec![
        format!(
            "squad-rating validation: {} existing-data squads, {} shared seeds, two legs per pair",
            result.squads.len(),
            result.seeds_per_leg
        ),
        "relative bot-proxy result: each squad gets home + human-proxy once per shared seed"
            .to_string(),
        "ratings (frozen position-weighted sum, 0..50):".to_string(),
    ];
    for squad in &result.squads {
        out.push(format!("  {:<12} {:>6.2}", squad.team.name, squad.rating));
    }

    out.push(String::new());
    out.push(format!(
        "{:<12} {:<12} {:>6} {:>9} {:>10} {:>11}",
        "higher", "lower", "gap", "W-D-L", "decisive", "score share"
    ));
    for pair in &result.pairs {
        let decisive = pair
            .win_rate
            .map(|r| format!("{:>6.1}%", r * 100.0))
            .unwrap_or_else(|| "   n/a".to_string());
        out.push(format!(
            "{:<12} {:<12} {:>6.2} {:>2}-{:>2}-{:>2} {:>10} {:>10.1}%",
            pair.higher.team.name,
            pair.lower.team.name,
            pair.gap,
            pair.wins,
            pair.draws,
            pair.losses,
            decisive,
            pair.score_share * 100.0
        ));
    }

    let decisive = result
        .win_rate
        .map(|r| format!("{:.1}%", r * 100.0))
        .unwrap_or_else(|| "n/a".to_string());
    out.push(String::new());
    out.push(format!(
        "aggregate higher-rated: {}-{}-{}, decisive win {}, score share {:.1}%",
        result.wins,
        result.draws,
        result.losses,
        decisive,
        result.score_share * 100.0
    ));
    out.push(format!(
        "pairs above 50% score share: {}/{}",
        result.above_half_pairs,
        result.pairs.len()
    ));
    out.push(format!(
        "curve steepness (OLS score share): {:+.2} percentage points per rating point",
        result.slope * 100.0
    ));
    out.join("\n")
}
