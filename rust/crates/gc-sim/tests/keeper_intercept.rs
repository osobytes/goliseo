//! Integration tests for the keeper's race-to-ball interception (the
//! SM-Strikers-shaped loose-ball chase): a through ball rolling into the
//! claim zone is contested and claimed when `keeper::intercept_race` says
//! the keeper wins it, deferred when the receiver clearly gets there first,
//! and left to a covering defender when a teammate is quicker.
//!
//! The decision helpers are pure and unit-tested in `tests/keeper.rs`;
//! these tests drive the private wiring (`keeper_intercept_target`, the
//! through-ball-cue suppression, and the widened claim branch) the same way
//! every other `match.rs` behavior test does: through the public
//! [`sim_match::step`] entry point on the futsal pitch, reading back events
//! and player state.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;

fn new_match_seeded(seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize {
            w: 1648.0,
            h: 927.0,
        },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: None,
    })
}

fn step(s: &mut MatchState, tune: &Tuning) {
    sim_match::step(s, DT, StepInput::Legacy(MatchInput::default()), None, tune);
}

/// Index of the first player on `team` with `is_keeper`.
fn keeper_index(s: &MatchState, team: Team) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == team && p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no keeper for {team:?}");
}

/// Index of the first home outfielder (the designated through-ball
/// receiver in these scenarios).
fn home_striker_index(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no home outfielder");
}

/// A through ball rolling from the away half into the away keeper's claim
/// zone (ground friction kills it around x~1458, depth ~190 inside the
/// 275-deep box), with a designated home receiver (`receive_timer` armed)
/// chasing from `striker_pos`. Everyone else is parked in the far corner
/// so only the runners under test can contest it.
fn setup_through_ball_race(s: &mut MatchState, striker_pos: Vec2) -> (i64, i64) {
    let ki = keeper_index(s, Team::Away);
    let si = home_striker_index(s);
    let mut slot = 0.0;
    for p in &mut s.players {
        if !p.is_keeper {
            p.pos = Vec2::new(40.0 + slot * 30.0, 40.0);
            slot += 1.0;
        }
    }
    s.players[(ki - 1) as usize].pos = Vec2::new(1636.0, 463.5);
    s.players[(si - 1) as usize].pos = striker_pos;
    s.players[(si - 1) as usize].receive_timer = 1.5;
    s.owner = None;
    s.pickup_cd = 0.0;
    s.block_grace = 0.0;
    s.ball = Vec2::new(1250.0, 463.5);
    s.ball_vel = Vec2::new(250.0, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    (ki, si)
}

/// Step until somebody owns the ball, returning (owner, elapsed seconds,
/// the deepest the keeper came off its line). Panics if nobody ever
/// collects it inside `limit_s`.
fn race_to_ownership(s: &mut MatchState, tune: &Tuning, ki: i64, limit_s: f64) -> (i64, f64, f64) {
    let mut elapsed = 0.0;
    let mut keeper_min_x = s.players[(ki - 1) as usize].pos.x;
    while elapsed < limit_s {
        step(s, tune);
        elapsed += DT;
        keeper_min_x = keeper_min_x.min(s.players[(ki - 1) as usize].pos.x);
        if let Some(owner) = s.owner {
            return (owner, elapsed, keeper_min_x);
        }
    }
    panic!("nobody collected the through ball within {limit_s}s");
}

#[test]
fn keeper_races_and_claims_a_winnable_through_ball_while_the_receiver_is_still_designated() {
    let mut s = new_match_seeded(7.0);
    let tune = Tuning::default();
    // The receiver starts ~450px behind the meet point: the keeper wins
    // the race by well over the win_margin_s band edge.
    let (ki, _si) = setup_through_ball_race(&mut s, Vec2::new(950.0, 463.5));

    let (owner, elapsed, keeper_min_x) = race_to_ownership(&mut s, &tune, ki, 2.5);

    assert_eq!(
        owner, ki,
        "the keeper should win a race it can win (owner was player {owner})"
    );
    assert!(
        elapsed < 1.5,
        "the keeper should contest the ball while the receiver is still \
         designated (receive_timer 1.5s), not wait the cue out; claimed at {elapsed:.2}s"
    );
    assert!(
        keeper_min_x < 1590.0,
        "the keeper should genuinely leave its line for the ball (base arc \
         holds x >= ~1630); deepest was {keeper_min_x:.1}"
    );
}

#[test]
fn keeper_defers_a_through_ball_the_receiver_clearly_wins() {
    let mut s = new_match_seeded(7.0);
    let tune = Tuning::default();
    // The receiver starts at the box edge, ahead of the ball's path: the
    // keeper cannot win this and must hold its ground instead of rushing.
    let (ki, si) = setup_through_ball_race(&mut s, Vec2::new(1330.0, 463.5));

    let mut elapsed = 0.0;
    while elapsed < 1.2 && s.owner.is_none() {
        step(&mut s, &tune);
        elapsed += DT;
        let keeper_x = s.players[(ki - 1) as usize].pos.x;
        assert!(
            keeper_x >= 1600.0,
            "an unwinnable race must not pull the keeper off its line \
             (keeper at x={keeper_x:.1} after {elapsed:.2}s)"
        );
    }
    if let Some(owner) = s.owner {
        assert_eq!(owner, si, "the designated receiver should collect it");
    }
}

#[test]
fn keeper_leaves_a_winnable_ball_to_a_quicker_covering_defender() {
    let mut s = new_match_seeded(7.0);
    let tune = Tuning::default();
    let (ki, _si) = setup_through_ball_race(&mut s, Vec2::new(950.0, 463.5));
    // A defending away outfielder sits right on the ball's path into the
    // box, far quicker to the meet point than the keeper.
    let di = {
        let mut found = None;
        for (i, p) in s.players.iter().enumerate() {
            if p.team == Team::Away && !p.is_keeper {
                found = Some((i + 1) as i64);
                break;
            }
        }
        found.expect("fixture has an away outfielder")
    };
    s.players[(di - 1) as usize].pos = Vec2::new(1380.0, 480.0);

    let mut elapsed = 0.0;
    while elapsed < 1.2 && s.owner.is_none() {
        step(&mut s, &tune);
        elapsed += DT;
        let keeper_x = s.players[(ki - 1) as usize].pos.x;
        assert!(
            keeper_x >= 1600.0,
            "a covered ball must not pull the keeper off its line \
             (keeper at x={keeper_x:.1} after {elapsed:.2}s)"
        );
    }
}
