//! Port of `spec/sim/rating_validation_spec.lua`.
//!
//! The Lua spec monkey-patches `headless.run_match`; this port uses
//! `rating_validation::run_with`'s injected match-runner instead — see that
//! module's "Test seam" doc.

use gc_sim::headless::{HeadlessBot, MatchResult, Winner};
use gc_sim::metrics;
use gc_sim::rating_validation;
use indexmap::IndexMap;

fn rank(id: &str) -> i64 {
    match id {
        "rating_prospects" => 1,
        "rating_developing" => 2,
        "rating_balanced" => 3,
        "rating_contenders" => 4,
        "rating_elite" => 5,
        other => panic!("unranked squad id: {other}"),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CallRecord {
    seed: f64,
    home_id: String,
    away_id: String,
    bot: Option<HeadlessBot>,
}

#[test]
fn rating_validation_plays_fair_home_proxy_legs_on_common_seeds_and_reports_the_curve() {
    let mut calls: Vec<CallRecord> = Vec::new();
    let result = rating_validation::run_with(2, |opts| {
        let home = opts.home.expect("fixture always sets home");
        let away = opts.away.expect("fixture always sets away");
        calls.push(CallRecord {
            seed: opts.seed,
            home_id: home.id.to_string(),
            away_id: away.id.to_string(),
            bot: opts.bot,
        });
        let winner = if rank(home.id) > rank(away.id) {
            Winner::Home
        } else {
            Winner::Away
        };
        MatchResult {
            seed: opts.seed,
            score: match winner {
                Winner::Home => metrics::Score { home: 1, away: 0 },
                Winner::Away => metrics::Score { home: 0, away: 1 },
            },
            winner: Some(winner),
            metrics: metrics::MatchMetrics::default(),
            desirability: IndexMap::new(),
        }
    });

    assert_eq!(result.squads.len(), 5);
    assert_eq!(result.pairs.len(), 10);
    assert_eq!(calls.len(), 40, "10 pairs x 2 seeds x 2 orientations");
    for i in (0..calls.len()).step_by(2) {
        let first = &calls[i];
        let second = &calls[i + 1];
        assert_eq!(
            first.seed, second.seed,
            "both orientations use the same seed"
        );
        assert_eq!(first.home_id, second.away_id, "home and away are swapped");
        assert_eq!(first.away_id, second.home_id, "home and away are swapped");
        assert_eq!(first.bot, Some(HeadlessBot::Home));
        assert_eq!(second.bot, Some(HeadlessBot::Home));
    }
    assert_eq!(result.wins, 40);
    assert_eq!(result.draws, 0);
    assert_eq!(result.losses, 0);
    assert_eq!(result.win_rate, Some(1.0));
    assert_eq!(result.score_share, 1.0);
    assert_eq!(result.above_half_pairs, 10);

    let report = rating_validation::report(&result);
    assert!(report.contains("relative bot-proxy result"));
    assert!(report.contains("curve steepness (OLS score share)"));
}
