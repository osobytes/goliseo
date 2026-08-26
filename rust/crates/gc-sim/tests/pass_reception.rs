//! The running-lanes reception harness: end-to-end proof that a pass to a
//! MOVING receiver is actually met and collected, under the real off-ball
//! AI / receive-assist steering — the scenario issue #491's own risk list
//! called for ("if receiver AI rarely runs onto led balls, leading looks
//! broken regardless of solver quality") and which no test provided until
//! the pass-reception rework. Every pre-existing end-to-end reception test
//! froze the receiver (`run_vel` zero), which exercises only the unled
//! fallback; every solver test stopped at the returned point. This file is
//! the missing middle: real velocity in, ball collected (or deliberately
//! not) out.
//!
//! The fixtures run on the shipped futsal pitch (1648x927), not the frozen
//! 960x540 the `tests/match.rs` geometry fixtures deliberately keep — the
//! defects this file pins regressions against (the receiver-lockout
//! collision, the tail-chase steering) were artifacts of the shipped
//! scale's speeds.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const TICK: f64 = 1.0 / 60.0;

fn new_match(human_controlled: Option<bool>) -> MatchState {
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
        seed: Some(7.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled,
        input_ownership: None,
    })
}

fn step(s: &mut MatchState, i: &MatchInput, tune: &Tuning) {
    sim_match::step(s, TICK, StepInput::Legacy(*i), None, tune);
}

fn pass_input(aim: Vec2) -> MatchInput {
    MatchInput {
        r#move: aim,
        pass: true,
        ..MatchInput::default()
    }
}

fn move_input(dir: Vec2) -> MatchInput {
    MatchInput {
        r#move: dir,
        ..MatchInput::default()
    }
}

/// A passer mid-pitch with the ball at their feet, one teammate placed at
/// `mate_pos` already running at `mate_run` (facing along it), everyone
/// else parked far away so selection cannot pick anyone but the mate and
/// no opponent contests the lane.
fn running_receiver_setup(mate_pos: Vec2, mate_run: Vec2) -> (MatchState, i64, i64) {
    let mut s = new_match(None);
    let passer = s.controlled;
    s.players[(passer - 1) as usize].pos = Vec2::new(400.0, 463.0);
    s.players[(passer - 1) as usize].facing = Vec2::new(1.0, 0.0);
    let mut mate = None;
    let mut parky = 60.0;
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != passer {
            if mate.is_none() {
                mate = Some(idx);
                p.pos = mate_pos;
                p.run_vel = mate_run;
                if mate_run.length() > 0.0 {
                    p.facing = mate_run.normalized();
                }
            } else {
                p.pos = Vec2::new(120.0, parky);
                parky += 100.0;
            }
        } else if p.team == Team::Away && !p.is_keeper {
            p.pos = Vec2::new(1500.0, 100.0);
        }
    }
    let mate = mate.expect("home fixture has another outfielder");
    s.owner = Some(passer);
    let passer_pos = s.players[(passer - 1) as usize].pos;
    s.ball = passer_pos.add(Vec2::new(18.0, 0.0));
    (s, passer, mate)
}

/// Release the pass on the first stepped tick and hand back the receiver's
/// stored reception point.
fn release(s: &mut MatchState, aim: Vec2, mate: i64, tune: &Tuning) -> Vec2 {
    step(s, &pass_input(aim), tune);
    assert!(
        s.events.iter().any(|e| e.kind == MatchEventKind::Pass),
        "fixture must release on the first tick"
    );
    assert!(
        s.players[(mate - 1) as usize].receive_timer > 0.0,
        "the mate is the designated receiver"
    );
    s.players[(mate - 1) as usize]
        .receive_target
        .expect("a released pass stores its reception point on the receiver")
}

// ---------------------------------------------------------------------
// The lockout fix: the designated receiver's first touch is legal even
// inside the release cooldown; the passer's re-touch still is not.
// ---------------------------------------------------------------------

#[test]
fn the_designated_receiver_traps_a_short_pass_inside_the_release_cooldown() {
    let tune = Tuning::new();
    // 150 px pass at the 720 px/s floor: the ball reaches the mate's feet
    // in ~0.23 s, inside RELEASE_CD (0.3 s). Before the rework this exact
    // flight was structurally uncollectable at arrival.
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(550.0, 463.0), Vec2::new(0.0, 0.0));
    release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    let mut ticks = 0;
    while s.owner.is_none() && ticks < 18 {
        step(&mut s, &MatchInput::default(), &tune);
        ticks += 1;
    }
    assert_eq!(
        s.owner,
        Some(mate),
        "the receiver traps at the meeting point, not after a runout chase"
    );
    assert!(
        (ticks as f64) * TICK < 0.3,
        "collection happened inside the release cooldown window ({}s)",
        ticks as f64 * TICK
    );
}

#[test]
fn the_passer_cannot_recollect_their_own_release() {
    let tune = Tuning::new();
    let (mut s, passer, mate) =
        running_receiver_setup(Vec2::new(550.0, 463.0), Vec2::new(0.0, 0.0));
    release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    // The ball leaves through the passer's own possession radius. Their
    // lockout must hold: possession may only resolve to the receiver.
    for _ in 0..30 {
        step(&mut s, &MatchInput::default(), &tune);
        assert_ne!(
            s.owner,
            Some(passer),
            "the release cooldown is the passer's"
        );
        if s.owner.is_some() {
            break;
        }
    }
    assert_eq!(s.owner, Some(mate));
}

// ---------------------------------------------------------------------
// The steering fix: a receiver with a genuine run is led, runs onto the
// stored reception point, and collects near it — under AI steering and
// under human receive assist alike.
// ---------------------------------------------------------------------

#[test]
fn an_ai_receiver_runs_onto_a_led_pass_and_collects_it_near_the_aim() {
    let tune = Tuning::new();
    // A receiver crossing the pass lane at speed: the classic led ball.
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let aim = release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    assert!(
        aim.y > 400.0,
        "a receiver running down-pitch is led into the run (aim {aim:?})"
    );
    // Hand the whole match to the AI: the receiver's movement now comes
    // from the off-ball receive override, not from human input.
    s.human_controlled = false;
    let mut collected_at = None;
    for _ in 0..240 {
        step(&mut s, &MatchInput::default(), &tune);
        if s.owner == Some(mate) {
            collected_at = Some(s.ball);
            break;
        }
        assert!(
            s.owner.is_none(),
            "nobody else may take an uncontested lane"
        );
    }
    let collected_at = collected_at.expect("the led pass is collected by its receiver");
    assert!(
        collected_at.dist(aim) < 120.0,
        "reception happens around the solved point, not down a runout: \
         collected {:.0} px from the aim",
        collected_at.dist(aim)
    );
}

#[test]
fn a_human_receiver_with_a_neutral_stick_is_assisted_onto_the_pass() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let aim = release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    // Control followed the pass (legacy mode, human passer): the receiver
    // is now the controlled player. A neutral stick must not strand them —
    // receive assist steers them onto the reception point.
    assert_eq!(s.controlled, mate, "control follows a human pass");
    let mut collected_at = None;
    for _ in 0..240 {
        step(&mut s, &MatchInput::default(), &tune);
        if s.owner == Some(mate) {
            collected_at = Some(s.ball);
            break;
        }
    }
    let collected_at = collected_at.expect("the assisted receiver collects");
    assert!(
        collected_at.dist(aim) < 120.0,
        "assisted reception happens around the solved point ({:.0} px off)",
        collected_at.dist(aim)
    );
}

// ---------------------------------------------------------------------
// The stale-stick latch: the aim the passer was still holding at release
// must not steer the receiver off the pass; a real redirect must.
// ---------------------------------------------------------------------

#[test]
fn a_held_over_aim_does_not_steer_the_receiver_off_the_pass() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let aim_dir = Vec2::new(1.0, 0.0);
    let aim = release(&mut s, aim_dir, mate, &tune);
    assert!(
        s.stick_latch.is_some(),
        "the pass-follow control switch latches the launch bearing"
    );
    // The player KEEPS HOLDING the direction they aimed the pass with —
    // the exact residue that used to sprint the receiver off the lane.
    let held = move_input(aim_dir);
    let mut collected_at = None;
    for _ in 0..240 {
        step(&mut s, &held, &tune);
        if s.owner == Some(mate) {
            collected_at = Some(s.ball);
            break;
        }
    }
    let collected_at = collected_at.expect("the latched hold reads as neutral: assist collects");
    assert!(
        collected_at.dist(aim) < 120.0,
        "reception still happens around the solved point ({:.0} px off)",
        collected_at.dist(aim)
    );
}

#[test]
fn a_deliberate_redirect_clears_the_latch_and_steers_the_receiver() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    assert!(s.stick_latch.is_some());
    // Pushing well outside the latch cone is a decision: the latch clears
    // on that very tick and the input steers from then on.
    let redirect = move_input(Vec2::new(0.0, -1.0));
    step(&mut s, &redirect, &tune);
    assert!(
        s.stick_latch.is_none(),
        "a clear redirect clears the latch immediately"
    );
    let before = s.players[(mate - 1) as usize].pos;
    for _ in 0..30 {
        step(&mut s, &redirect, &tune);
    }
    let after = s.players[(mate - 1) as usize].pos;
    assert!(
        after.y < before.y - 10.0,
        "the receiver follows the deliberate input, not the assist"
    );
}

#[test]
fn releasing_the_stick_clears_the_latch_for_good() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let aim_dir = Vec2::new(1.0, 0.0);
    release(&mut s, aim_dir, mate, &tune);
    assert!(s.stick_latch.is_some());
    step(&mut s, &MatchInput::default(), &tune);
    assert!(s.stick_latch.is_none(), "neutral input retires the latch");
    // Re-pressing the old aim direction afterwards is a fresh decision and
    // steers, exactly as if the latch had never existed.
    let before = s.players[(mate - 1) as usize].pos;
    for _ in 0..30 {
        step(&mut s, &move_input(aim_dir), &tune);
    }
    let after = s.players[(mate - 1) as usize].pos;
    assert!(
        after.x > before.x + 10.0,
        "post-latch input steers the receiver normally"
    );
}

// ---------------------------------------------------------------------
// The approach-target helper's phases, exercised directly.
// ---------------------------------------------------------------------

#[test]
fn the_approach_target_is_the_point_first_then_the_live_ball() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let aim = release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    let _ = &tune;
    // Phase 1: ball inbound, receiver short of the point — run to the point.
    assert_eq!(
        sim_match::receive_approach_target(&s, mate),
        aim,
        "while the pass is inbound the receiver attacks the reception point"
    );
    // Phase 2: once the ball is essentially at (or past) the point, the
    // live ball is the target — simulate that by parking the ball on the
    // point with its velocity carrying it away.
    s.ball = aim;
    s.ball_vel = Vec2::new(300.0, 0.0);
    let t = sim_match::receive_approach_target(&s, mate);
    assert_eq!(
        t,
        Vec2::new(s.ball.x, s.ball.y),
        "a ball no longer inbound is chased live, not waited for"
    );
    // No stored point (cleared) — always the live ball.
    s.players[(mate - 1) as usize].receive_target = None;
    let t = sim_match::receive_approach_target(&s, mate);
    assert_eq!(t, Vec2::new(s.ball.x, s.ball.y));
}
