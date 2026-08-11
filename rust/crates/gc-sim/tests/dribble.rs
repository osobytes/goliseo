//! Tests for dribbling behavior in `gc_sim::r#match`.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

fn new_match() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
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
    })
}

struct InputOpts {
    r#move: Vec2,
    sprint: bool,
}

impl Default for InputOpts {
    fn default() -> Self {
        InputOpts {
            r#move: Vec2::new(0.0, 0.0),
            sprint: false,
        }
    }
}

fn input(o: InputOpts) -> MatchInput {
    MatchInput {
        r#move: o.r#move,
        sprint: o.sprint,
        ..MatchInput::default()
    }
}

fn step(s: &mut MatchState, i: &MatchInput, tune: &Tuning) {
    sim_match::step(s, 1.0 / 60.0, StepInput::Legacy(*i), None, tune);
}

/// Forward offset of the ball from the carrier's feet, along their facing.
fn forward_offset(s: &MatchState) -> f64 {
    let owner = s.owner.expect("carrier owns the ball");
    let p = &s.players[(owner - 1) as usize];
    let off = s.ball.sub(p.pos);
    off.x * p.facing.x + off.y * p.facing.y
}

/// Park everyone except the controlled carrier far away: these tests
/// isolate BALL CONTROL. Duels (pokes, body-blocks) are contested
/// elsewhere.
fn isolate_carrier(s: &mut MatchState) {
    let controlled = s.controlled;
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if idx != controlled {
            p.pos = if p.team == Team::Home {
                Vec2::new(80.0, 40.0 + idx as f64 * 30.0)
            } else {
                Vec2::new(880.0, 40.0 + idx as f64 * 30.0)
            };
            p.anchor = p.pos;
        }
    }
}

fn count_touches(s: &MatchState) -> usize {
    s.events
        .iter()
        .filter(|e| e.kind == MatchEventKind::Touch)
        .count()
}

#[test]
fn keeps_a_grounded_ball_within_the_carriers_control_while_dribbling() {
    let tune = Tuning::new();
    let mut s = new_match();
    let run = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        sprint: true,
    });
    for _ in 0..40 {
        step(&mut s, &run, &tune);
        let Some(owner) = s.owner else {
            break; // a heavy touch got away; that's covered elsewhere
        };
        let p = &s.players[(owner - 1) as usize];
        let control = tune.value("DRIBBLE_CONTROL") + 26.0 * p.dribble;
        assert!(s.ball_z == 0.0, "an owned ball is grounded");
        assert!(
            p.pos.dist(s.ball) <= control + 1.0,
            "ball stays within control radius"
        );
    }
}

#[test]
fn pushes_the_ball_further_ahead_when_sprinting_than_when_standing() {
    let tune = Tuning::new();
    // Standing: the ball settles a short step ahead of the feet.
    let mut stand = new_match();
    for _ in 0..20 {
        step(&mut stand, &input(InputOpts::default()), &tune);
    }
    let stand_lead = forward_offset(&stand);

    // Sprinting: the same carrier knocks the ball noticeably further ahead
    // (45 frames: the standing-start ramp must reach knock-on speed
    // first).
    let mut run = new_match();
    isolate_carrier(&mut run);
    run.players[(run.controlled - 1) as usize].dribble = 1.0; // retention is tested elsewhere
    let sprint_input = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        sprint: true,
    });
    for _ in 0..45 {
        step(&mut run, &sprint_input, &tune);
    }
    assert!(run.owner.is_some(), "still dribbling after the run");
    let run_lead = forward_offset(&run);
    assert!(
        run_lead > stand_lead + 6.0,
        "sprinting pushes the ball ahead: run={run_lead:.1} stand={stand_lead:.1}"
    );
}

#[test]
fn close_control_at_a_jog_glued_ball_no_knock_ons_nothing_to_lose() {
    let tune = Tuning::new();
    let mut s = new_match();
    isolate_carrier(&mut s);
    let jog = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        ..InputOpts::default()
    }); // no sprint: ordinary running
    let mut touches = 0;
    for _ in 0..90 {
        step(&mut s, &jog, &tune);
        assert!(s.owner.is_some(), "close control never risks possession");
        touches += count_touches(&s);
        let owner = s.owner.expect("checked above");
        assert!(
            s.players[(owner - 1) as usize].pos.dist(s.ball) <= 30.0,
            "the ball stays glued near the feet"
        );
    }
    assert_eq!(
        touches, 0,
        "no knock-on kicks below the close-control speed"
    );
}

#[test]
fn dribbles_in_discrete_kicks_repeated_touches_and_a_pulsing_gap() {
    let tune = Tuning::new();
    let mut s = new_match();
    isolate_carrier(&mut s);
    s.players[(s.controlled - 1) as usize].dribble = 1.0; // retention is tested elsewhere
    s.players[(s.controlled - 1) as usize].sprint_meter = 1.0;
    let run = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        sprint: true,
    });
    let mut touches = 0;
    let mut min_gap = f64::INFINITY;
    let mut max_gap = 0.0_f64;
    for _ in 0..240 {
        step(&mut s, &run, &tune);
        let Some(owner) = s.owner else {
            break; // a heavy touch got away; that's covered elsewhere
        };
        touches += count_touches(&s);
        let gap = s.players[(owner - 1) as usize].pos.dist(s.ball);
        min_gap = min_gap.min(gap);
        max_gap = max_gap.max(gap);
    }
    assert!(
        touches >= 3,
        "kick-chase-kick, not a servo: {touches} touches"
    );
    assert!(
        max_gap - min_gap > 8.0,
        "the ball runs ahead and comes back: gap {min_gap:.1}..{max_gap:.1}"
    );
}

#[test]
fn hooks_the_carrier_back_to_a_run_on_ball_the_stick_turns_the_touch() {
    let tune = Tuning::new();
    let mut s = new_match();
    let controlled = s.controlled;
    {
        let me = &mut s.players[(controlled - 1) as usize];
        me.dribble = 1.0; // clean feet: this test is about the hook, not error
        me.pos = Vec2::new(300.0, 270.0);
        me.facing = Vec2::new(1.0, 0.0);
        me.run_vel = Vec2::new(0.0, 0.0);
    }
    s.owner = Some(controlled);
    s.ball = Vec2::new(340.0, 270.0); // the touch ran on: beyond reach, within control
    s.ball_vel = Vec2::new(0.0, 0.0);
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if idx != controlled {
            // Park everyone far away: nobody bumps or challenges the
            // carrier.
            p.pos = if p.team == Team::Home {
                Vec2::new(80.0, 40.0 + idx as f64 * 30.0)
            } else {
                Vec2::new(880.0, 40.0 + idx as f64 * 30.0)
            };
            p.anchor = p.pos;
        }
    }
    // Sprint: knock-on touches only happen above close-control speed.
    let up = input(InputOpts {
        r#move: Vec2::new(0.0, -1.0),
        sprint: true,
    });
    // Chase phase: the stick points up, but the carrier runs to the BALL.
    for _ in 0..10 {
        step(&mut s, &up, &tune);
    }
    assert_eq!(
        s.owner,
        Some(controlled),
        "possession held through the chase"
    );
    let me = &s.players[(controlled - 1) as usize];
    assert!(me.pos.x > 304.0, "the carrier ran toward the ball...");
    assert!(
        (me.pos.y - 270.0).abs() < 3.0,
        "...not where the stick points"
    );
    // Touch phase: keep holding up until the ball is back at the feet —
    // the next touch obeys the stick and turns the dribble upward.
    let mut turned = false;
    for _ in 0..60 {
        step(&mut s, &up, &tune);
        if count_touches(&s) > 0 {
            turned = true;
        }
        if turned {
            break;
        }
    }
    assert!(turned, "a touch fired once the ball was back at the feet");
    assert!(
        s.ball_vel.y < 0.0,
        "and it went where the stick pointed (up)"
    );
    assert!(
        s.ball_vel.x.abs() < -s.ball_vel.y,
        "clearly upward, not along the old chase line"
    );
}

#[test]
fn loses_possession_on_a_heavy_touch_at_pace() {
    let mut tune = Tuning::new();
    // Crank the touch heavy and the control tight: a sprint runs the ball
    // away.
    let push_max = gc_sim::tuning::KNOBS
        .iter()
        .find(|k| k.key == "DRIBBLE_PUSH")
        .expect("DRIBBLE_PUSH is an authored knob")
        .max;
    let control_min = gc_sim::tuning::KNOBS
        .iter()
        .find(|k| k.key == "DRIBBLE_CONTROL")
        .expect("DRIBBLE_CONTROL is an authored knob")
        .min;
    tune.set("DRIBBLE_PUSH", push_max);
    tune.set("DRIBBLE_CONTROL", control_min);
    let mut s = new_match();
    let mut lost = false;
    let run = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        sprint: true,
    });
    for _ in 0..120 {
        step(&mut s, &run, &tune);
        if s.owner.is_none() {
            lost = true;
            break;
        }
    }
    assert!(lost, "a heavy touch at speed ran the ball out of control");
}
