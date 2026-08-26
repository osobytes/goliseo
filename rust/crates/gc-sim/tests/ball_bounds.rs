//! Tests for ball-out-of-bounds handling in `gc_sim::r#match`.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const FIELD: PitchSize = PitchSize {
    w: 1648.0,
    h: 927.0,
};
const PLAYER_RADIUS: f64 = 12.0;

fn no_input() -> MatchInput {
    MatchInput::default()
}

fn step(s: &mut MatchState, tune: &Tuning) {
    sim_match::step(s, 1.0 / 60.0, StepInput::Legacy(no_input()), None, tune);
}

fn scenario() -> (MatchState, i64) {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut s = sim_match::new(NewMatchOptions {
        home,
        away,
        field: FIELD,
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(11.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: None,
    });
    let carrier: i64 = 2;
    {
        let p = &mut s.players[(carrier - 1) as usize];
        p.pos = Vec2::new(FIELD.w * 0.5, FIELD.h - 2.0);
        p.facing = Vec2::new(0.0, 1.0);
    }
    s.owner = Some(carrier);
    s.ball_vel = Vec2::new(0.0, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    // Cancel place_kickoff's defensive freeze; these scenarios start mid-play.
    s.kickoff_hold = 0.0;
    (s, carrier)
}

/// Puts the carrier next to `bx, by` so the ball stays inside dribbling
/// control and the POSSESSION path keeps running.
fn carrier_at(bx: f64, by: f64, face_x: f64, face_y: f64, team: Option<Team>) -> (MatchState, i64) {
    let (mut s, mut carrier) = scenario();
    if let Some(team) = team {
        for (index, player) in s.players.iter().enumerate() {
            if player.team == team && !player.is_keeper {
                carrier = (index + 1) as i64;
                break;
            }
        }
        s.owner = Some(carrier);
    }
    {
        let p = &mut s.players[(carrier - 1) as usize];
        p.pos = Vec2::new(
            (bx - face_x * 14.0).clamp(PLAYER_RADIUS, FIELD.w - PLAYER_RADIUS),
            (by - face_y * 14.0).clamp(PLAYER_RADIUS, FIELD.h - PLAYER_RADIUS),
        );
        p.facing = Vec2::new(face_x, face_y);
        p.vel = Vec2::new(0.0, 0.0);
        p.run_vel = Vec2::new(0.0, 0.0);
    }
    s.ball = Vec2::new(bx, by);
    s.ball_vel = Vec2::new(0.0, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    (s, carrier)
}

#[test]
fn pulls_a_possessed_ball_back_onto_the_pitch() {
    let tune = Tuning::new();
    let (mut s, _) = scenario();
    s.ball = Vec2::new(FIELD.w * 0.5, FIELD.h + 25.0);

    for _ in 0..30 {
        step(&mut s, &tune);
    }

    assert!(
        s.ball.y <= FIELD.h,
        "ball stayed outside the touchline at y={}",
        s.ball.y
    );
}

#[test]
fn keeps_a_possessed_ball_inside_the_arena_on_every_side() {
    let tune = Tuning::new();
    let cases = [
        (-20.0, 60.0, -1.0, 0.0),
        (FIELD.w + 20.0, FIELD.h - 60.0, 1.0, 0.0),
        (FIELD.w / 2.0, -20.0, 0.0, -1.0),
        (FIELD.w / 2.0, FIELD.h + 20.0, 0.0, 1.0),
    ];
    for (x, y, fx, fy) in cases {
        let (mut s, carrier) = carrier_at(x, y, fx, fy, None);
        for tick in 1..=12 {
            step(&mut s, &tune);
            assert_eq!(
                s.owner,
                Some(carrier),
                "possession lost at tick {tick}, so this case stopped testing the clamp"
            );
        }
        assert!(
            s.ball.x >= 0.0 && s.ball.x <= FIELD.w && s.ball.y >= 0.0 && s.ball.y <= FIELD.h,
            "ball escaped at ({}, {})",
            s.ball.x,
            s.ball.y
        );
    }
}

#[test]
fn leaves_a_possessed_ball_exactly_on_a_boundary_alone() {
    let tune = Tuning::new();
    for x in [0.0, FIELD.w] {
        let face = if x == 0.0 { -1.0 } else { 1.0 };
        let (mut s, _) = carrier_at(x, 60.0, face, 0.0, None);
        step(&mut s, &tune);
        assert!(
            s.ball.x >= 0.0 && s.ball.x <= FIELD.w,
            "boundary ball moved out to x={}",
            s.ball.x
        );
    }
}

#[test]
fn still_lets_a_carrier_dribble_the_ball_over_each_goal_line() {
    let tune = Tuning::new();
    struct Case {
        home_goal: bool,
        face: f64,
        team: Team,
    }
    let cases = [
        Case {
            home_goal: false,
            face: 1.0,
            team: Team::Home,
        },
        Case {
            home_goal: true,
            face: -1.0,
            team: Team::Away,
        },
    ];
    for case in cases {
        let (probe, _) = scenario();
        let goal = if case.home_goal {
            probe.goal_home
        } else {
            probe.goal_away
        };
        let mouth_y = goal.y + goal.h / 2.0;
        let start_x = if case.face > 0.0 { FIELD.w - 4.0 } else { 4.0 };
        let (mut s, carrier) = carrier_at(start_x, mouth_y, case.face, 0.0, Some(case.team));
        {
            let p = &mut s.players[(carrier - 1) as usize];
            p.run_vel = Vec2::new(case.face * 200.0, 0.0);
            p.vel = p.run_vel;
        }

        let mut crossed_while_owned = false;
        let before = if case.team == Team::Home {
            s.score.home
        } else {
            s.score.away
        };
        for _ in 0..40 {
            step(&mut s, &tune);
            let past = if case.face > 0.0 {
                s.ball.x > FIELD.w
            } else {
                s.ball.x < 0.0
            };
            if past && s.owner == Some(carrier) {
                crossed_while_owned = true;
            }
            let now = if case.team == Team::Home {
                s.score.home
            } else {
                s.score.away
            };
            if now > before {
                break;
            }
        }
        assert!(
            crossed_while_owned,
            "the ball never crossed the line while owned, so the possession clamp was never the \
             thing being tested"
        );
        let after = if case.team == Team::Home {
            s.score.home
        } else {
            s.score.away
        };
        assert!(
            after > before,
            "a ball dribbled over the line must still score"
        );
    }
}

#[test]
fn does_not_displace_a_ball_a_keeper_is_holding() {
    let tune = Tuning::new();
    let (mut s, _) = scenario();
    let keeper = s
        .players
        .iter()
        .position(|p| p.is_keeper)
        .expect("fixture has no keeper");
    let keeper_idx = (keeper + 1) as i64;
    s.owner = Some(keeper_idx);
    s.ball = s.players[keeper].pos;
    step(&mut s, &tune);
    let held = s.ball;
    step(&mut s, &tune);
    assert!((s.ball.x - held.x).abs() <= 12.0);
    assert!((s.ball.y - held.y).abs() <= 12.0);
    assert!(
        s.ball.x >= s.goal_home.x && s.ball.x <= s.goal_away.x + s.goal_away.w,
        "keeper's ball left the arena at x={}",
        s.ball.x
    );
}

#[test]
fn recovers_a_ball_stranded_behind_the_net() {
    let tune = Tuning::new();
    let (mut s, _) = scenario();
    let goal = s.goal_away;
    s.ball = Vec2::new(goal.x + goal.w + 200.0, goal.y + goal.h / 2.0);
    for _ in 0..20 {
        step(&mut s, &tune);
    }
    assert!(
        s.ball.x <= goal.x + goal.w,
        "ball stayed behind the net at x={}",
        s.ball.x
    );
}
