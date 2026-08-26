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

/// A juke is a sidestep, not a strike. The dribble touch reads the
/// carrier's REALIZED speed, and a juke is bespoke movement at
/// `DODGE_SPEED_MULT` (2.4x) that shows up in exactly that channel — so
/// before #629 the sidestep was read as carrier pace and the ball was
/// kicked away at 1.5x it again, along the PRE-juke facing, while the body
/// travelled perpendicular. The carrier sidestepped into space and the
/// ball ran off on its own.
#[test]
fn a_juke_carries_the_ball_rather_than_striking_it_away() {
    let tune = Tuning::new();
    let mut s = new_match();
    isolate_carrier(&mut s);
    let controlled = s.controlled;
    let jog = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        ..InputOpts::default()
    });
    // Settle into a close-control jog: ball at the feet, nothing in flight.
    for _ in 0..20 {
        step(&mut s, &jog, &tune);
    }
    assert_eq!(s.owner, Some(controlled), "carrying before the juke");
    let ball_before = s.ball;
    let pos_before = s.players[(controlled - 1) as usize].pos;

    let juke = MatchInput {
        r#move: Vec2::new(1.0, 0.0),
        dodge: true,
        ..MatchInput::default()
    };
    step(&mut s, &juke, &tune);
    let dodge_dir = s.players[(controlled - 1) as usize].dodge_dir;
    assert!(
        s.players[(controlled - 1) as usize].dodge_timer > 0.0,
        "the juke fired"
    );

    // Walk the sidestep out tick by tick. `dodge_timer` is decremented at
    // the top of a tick, so a positive value after `step` means this tick's
    // ball update ran under an active juke.
    let mut mid_juke_ticks = 0;
    while s.players[(controlled - 1) as usize].dodge_timer > 0.0 {
        assert_eq!(
            s.owner,
            Some(controlled),
            "the sidestep never hands the ball over"
        );
        assert_eq!(count_touches(&s), 0, "a sidestep is not a touch");
        let p = &s.players[(controlled - 1) as usize];
        // Close control gives the ball the carrier's OWN velocity plus a
        // corrective nudge, so relative to the body it barely moves. The
        // gentlest kick-and-chase touch the game can produce separates ball
        // from body by at least `speed * (DRIBBLE_PUSH - 1)`; stay under it.
        let drift = s.ball_vel.sub(p.vel).length();
        let touch_drift = p.vel.length() * (tune.value("DRIBBLE_PUSH") - 1.0);
        assert!(
            drift < touch_drift,
            "the ball rides the body through the juke: drift={drift:.1} touch_drift={touch_drift:.1}"
        );
        mid_juke_ticks += 1;
        step(&mut s, &jog, &tune);
    }
    assert!(
        mid_juke_ticks >= 8,
        "the sidestep lasted a real window: {mid_juke_ticks} ticks"
    );
    assert_eq!(
        s.owner,
        Some(controlled),
        "still carrying when the sidestep ends"
    );

    // And the ball went sideways WITH the body, not straight on along the
    // pre-juke facing.
    let body = s.players[(controlled - 1) as usize].pos.sub(pos_before);
    let ball = s.ball.sub(ball_before);
    let body_across = body.x * dodge_dir.x + body.y * dodge_dir.y;
    let ball_across = ball.x * dodge_dir.x + ball.y * dodge_dir.y;
    assert!(
        body_across > 40.0,
        "the body really did sidestep: {body_across:.1} px"
    );
    assert!(
        ball_across > body_across * 0.5,
        "the ball came along: body={body_across:.1} px, ball={ball_across:.1} px"
    );
}

/// Outcome of a committed human slide against an away carrier.
struct Challenge {
    /// A challenge landed: the ball was popped loose and a `Tackle` fired.
    /// This is the signal, since the winner does not take ownership
    /// directly.
    tackled: bool,
    /// The carrier still had the ball at the end of the window.
    kept_the_ball: bool,
}

/// Run a committed human slide at an away carrier for the length of a
/// sidestep, with (`juking = true`) or without an active juke.
fn slide_beats_the_carrier(juking: bool) -> Challenge {
    let tune = Tuning::new();
    let mut s = new_match();
    let carrier = s
        .players
        .iter()
        .position(|p| p.team == Team::Away && !p.is_keeper)
        .expect("the away side fields outfielders") as i64
        + 1;
    let defender = s
        .players
        .iter()
        .position(|p| p.team == Team::Home && !p.is_keeper)
        .expect("the home side fields outfielders") as i64
        + 1;
    s.controlled = defender;
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if idx != carrier && idx != defender {
            p.pos = Vec2::new(
                if p.team == Team::Home { 80.0 } else { 880.0 },
                40.0 + idx as f64 * 30.0,
            );
            p.anchor = p.pos;
        }
    }
    {
        let c = &mut s.players[(carrier - 1) as usize];
        c.pos = Vec2::new(480.0, 270.0);
        c.facing = Vec2::new(1.0, 0.0);
        c.vel = Vec2::new(0.0, 0.0);
        c.run_vel = Vec2::new(0.0, 0.0);
        // Pin the sidestep rather than let the AI pick one: this test is
        // about the i-frames, not about when the AI reaches for them.
        c.dodge_cd = 1.0;
        c.dodge_timer = if juking { 0.16 } else { 0.0 };
        c.dodge_dir = Vec2::new(0.0, 1.0);
    }
    s.owner = Some(carrier);
    s.ball = Vec2::new(498.0, 270.0); // a step ahead of the carrier's feet
    s.ball_vel = Vec2::new(0.0, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    {
        let d = &mut s.players[(defender - 1) as usize];
        d.pos = Vec2::new(516.0, 270.0); // inside slide reach of the ball
        d.facing = Vec2::new(-1.0, 0.0);
        // Committed and already past its opening frames, so the carrier's
        // own AI does not read it as a fresh telegraph and juke on its own.
        d.slide_timer = 1.0;
        d.slide_dir = Vec2::new(0.0, 0.0);
        d.slide_vel = 0.0;
    }
    // Ten ticks at 60 Hz is the sidestep's own window (`DODGE_DURATION`).
    let mut tackled = false;
    for _ in 0..10 {
        step(&mut s, &input(InputOpts::default()), &tune);
        tackled |= s.events.iter().any(|e| e.kind == MatchEventKind::Tackle);
    }
    Challenge {
        tackled,
        kept_the_ball: s.owner == Some(carrier),
    }
}

/// The juke's tackle i-frames were unreachable in play before #629: the
/// sidestep threw the ball away, so by the time a challenge landed there
/// was no owner left to protect. Carrying the ball through the sidestep
/// makes them live, so assert they still deny a committed slide — and that
/// the very same slide wins the ball against a carrier who is not juking,
/// because an i-frame test that cannot go red proves nothing.
///
/// The two tests above both start from a settled close-control jog, so
/// neither reaches the run-on window this one covers: a juke fired while
/// the carrier's OWN last touch is still rolling out ahead of the feet.
/// `update_ball`'s dribble arm orders its four cases ball-runs-free /
/// close-control / next-touch / just-kicked-run-on, and #629's
/// `dodge_timer` clause lives in the second — so it claims a tick the
/// FOURTH would otherwise have taken.
///
/// **That ordering improves the run-on case rather than regressing it**,
/// and the numbers here are measured on this fixture, not assumed. At the
/// tick the juke fires the ball is 21.7 px ahead (inside the 24 px
/// `DRIBBLE_TOUCH_REACH`) rolling at 262 px/s, and the carrier is running
/// at 176 px/s — case 4, because 262 > 176 + `DRIBBLE_CATCH_PACE`. But a
/// juke replaces realized speed with `move_speed * DODGE_SPEED_MULT`
/// (160 * 2.4 = 384 px/s here), and 262 is NOT greater than 384 + 10 — so
/// before the fix the juke tick fell straight through case 4 into case 3
/// and RE-STRUCK the rolling ball, measured at 596 px/s (= 384 * 1.5 *
/// weight). Possession was gone before the sidestep ended. The assertions
/// below pin that arithmetic so the claim cannot rot.
#[test]
fn a_juke_keeps_the_ball_even_while_the_last_touch_is_still_running_on() {
    let tune = Tuning::new();
    let mut s = new_match();
    isolate_carrier(&mut s);
    let controlled = s.controlled;
    s.players[(controlled - 1) as usize].dribble = 1.0; // clean feet: this is about branch order
    s.players[(controlled - 1) as usize].sprint_meter = 1.0;
    let run = input(InputOpts {
        r#move: Vec2::new(1.0, 0.0),
        sprint: true,
    });
    // Sprint until a real touch fires: only above close-control speed does
    // the carrier kick the ball ahead of the run at all.
    let mut struck = false;
    for _ in 0..90 {
        step(&mut s, &run, &tune);
        if count_touches(&s) > 0 {
            struck = true;
            break;
        }
    }
    assert!(struck, "the carrier played a touch to run on to");

    // The run-on case is live RIGHT NOW, and this is asserted rather than
    // assumed so the fixture cannot silently drift into close control and
    // quietly stop covering what it says it covers.
    let p = s.players[(controlled - 1) as usize].clone();
    let speed = p.vel.length();
    let ball_vel = s.ball_vel.length();
    assert!(
        p.pos.dist(s.ball) <= 24.0,
        "the ball is still within touch reach: {:.1} px",
        p.pos.dist(s.ball)
    );
    assert!(
        speed >= p.move_speed * tune.value("DRIBBLE_CLOSE"),
        "above close-control speed, so case 2 is NOT what would claim this \
         tick: speed={speed:.1}"
    );
    assert!(
        ball_vel > speed + 10.0,
        "and the touch is still leaving the boot, so case 4 owns it: \
         ball_vel={ball_vel:.1} speed={speed:.1}"
    );

    // The pre-fix diagnosis, pinned: a juke's realized speed is high enough
    // that case 4's own guard flips and case 3 would re-strike the ball.
    let juke_speed = p.move_speed * 2.4; // DODGE_SPEED_MULT
    assert!(
        ball_vel <= juke_speed + 10.0,
        "a juke inflates realized speed past the run-on guard, which is how \
         this used to become a second strike: ball_vel={ball_vel:.1} \
         juke_speed={juke_speed:.1}"
    );

    let juke = MatchInput {
        r#move: Vec2::new(1.0, 0.0),
        dodge: true,
        ..MatchInput::default()
    };
    step(&mut s, &juke, &tune);
    assert!(
        s.players[(controlled - 1) as usize].dodge_timer > 0.0,
        "the juke fired"
    );
    let mut touches = count_touches(&s);
    while s.players[(controlled - 1) as usize].dodge_timer > 0.0 {
        assert_eq!(
            s.owner,
            Some(controlled),
            "the sidestep keeps the ball even mid-touch-cycle"
        );
        step(&mut s, &run, &tune);
        touches += count_touches(&s);
    }
    assert_eq!(
        s.owner,
        Some(controlled),
        "still carrying when the sidestep ends"
    );
    assert_eq!(
        touches, 0,
        "and the sidestep never became a second strike on the rolling ball"
    );
}

#[test]
fn juke_i_frames_deny_a_committed_slide() {
    let juked = slide_beats_the_carrier(true);
    assert!(
        !juked.tackled,
        "a juking carrier is immune to a committed slide"
    );
    assert!(
        juked.kept_the_ball,
        "and comes out of the sidestep still carrying"
    );
    assert!(
        slide_beats_the_carrier(false).tackled,
        "...while the same slide wins the ball when there is no juke, so \
         the immunity above is the juke's doing"
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
