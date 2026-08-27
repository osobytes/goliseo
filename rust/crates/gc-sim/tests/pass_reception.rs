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

// ---------------------------------------------------------------------
// Scripted pass-scenario suite: one case per pass TYPE, each driving the
// sim with an explicit input sequence and asserting the outcome the
// rework promises. The cases above pin the two mechanisms; these pin the
// vocabulary — through-ball, come-short, tap vs charge, lob, dink,
// covered lane (slow and fast), touchline lead, keeper back-pass, long
// ball at the reach margin, and a one-two. All deterministic (seed 7,
// scripted inputs, no AI randomness on the asserted path).
// ---------------------------------------------------------------------

/// Like `running_receiver_setup`, with one opponent parked at `opp_pos`
/// instead of out of play.
fn contested_setup(mate_pos: Vec2, mate_run: Vec2, opp_pos: Vec2) -> (MatchState, i64, i64, i64) {
    let (mut s, passer, mate) = running_receiver_setup(mate_pos, mate_run);
    let mut opp = None;
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Away && !p.is_keeper && opp.is_none() {
            p.pos = opp_pos;
            opp = Some(idx);
        }
    }
    (
        s,
        passer,
        mate,
        opp.expect("away fixture has an outfielder"),
    )
}

/// Step until `pred` holds or `budget` ticks pass; returns ticks stepped.
fn step_until(
    s: &mut MatchState,
    input: &MatchInput,
    tune: &Tuning,
    budget: u32,
    pred: impl Fn(&MatchState) -> bool,
) -> u32 {
    let mut ticks = 0;
    while ticks < budget && !pred(s) {
        step(s, input, tune);
        ticks += 1;
    }
    ticks
}

#[test]
fn a_through_ball_leads_a_receiver_running_up_the_lane() {
    let tune = Tuning::new();
    // The receiver runs AWAY from the passer, straight along the aim: the
    // classic through-ball. The aim must land ahead of their start, and
    // they must collect it around that point without turning back.
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(800.0, 463.0), Vec2::new(220.0, 0.0));
    let aim = release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    assert!(
        aim.x > 800.0,
        "a through-ball is played into the run, not at the heels (aim {aim:?})"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 240, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 240, "the through-ball is collected");
    assert!(
        s.ball.dist(aim) < 120.0,
        "collected around the meeting point, {:.0} px off",
        s.ball.dist(aim)
    );
}

#[test]
fn a_receiver_coming_short_is_met_between_the_lines() {
    let tune = Tuning::new();
    // The receiver runs TOWARD the passer. The lead projects along that
    // run, so the meeting point sits between the two — the ball must not
    // be played behind the receiver's start.
    let start = Vec2::new(900.0, 463.0);
    let (mut s, _passer, mate) = running_receiver_setup(start, Vec2::new(-200.0, 0.0));
    let aim = release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    assert!(
        aim.x < start.x,
        "a come-short receiver is met early, not chased backwards (aim {aim:?})"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 180, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 180, "the come-short pass is collected");
    assert!(
        s.ball.x < start.x + 22.0,
        "reception happened at or before the start of the run"
    );
}

#[test]
fn a_tap_finds_the_near_man_and_a_full_charge_finds_the_far_man() {
    let tune = Tuning::new();
    // Two mates dead on the aim line at 300 and 780 px. A tap scores raw
    // distance (near wins); a full charge scores |d - range| with range at
    // PASS_RANGE_MAX-ish (far wins). Same aim both times.
    let build = || {
        let mut s = new_match(None);
        let passer = s.controlled;
        s.players[(passer - 1) as usize].pos = Vec2::new(300.0, 463.0);
        s.players[(passer - 1) as usize].facing = Vec2::new(1.0, 0.0);
        let mut mates = Vec::new();
        let mut parky = 60.0;
        for (i, p) in s.players.iter_mut().enumerate() {
            let idx = (i + 1) as i64;
            if p.team == Team::Home && !p.is_keeper && idx != passer {
                if mates.len() < 2 {
                    // The far man sits OFF the near man's line: collinear
                    // mates would let the driven ball ricochet off the near
                    // body (a real rule, a different case).
                    p.pos = if mates.is_empty() {
                        Vec2::new(600.0, 463.0)
                    } else {
                        Vec2::new(1080.0, 300.0)
                    };
                    mates.push(idx);
                } else {
                    p.pos = Vec2::new(120.0, parky);
                    parky += 100.0;
                }
            } else if p.team == Team::Away && !p.is_keeper {
                p.pos = Vec2::new(1500.0, 100.0);
            }
        }
        s.owner = Some(passer);
        let passer_pos = s.players[(passer - 1) as usize].pos;
        s.ball = passer_pos.add(Vec2::new(18.0, 0.0));
        (s, mates[0], mates[1])
    };

    // Tap: single pass edge.
    let (mut s, near, _far) = build();
    step(&mut s, &pass_input(Vec2::new(1.0, 0.0)), &tune);
    assert!(
        s.players[(near - 1) as usize].receive_timer > 0.0,
        "a tap goes to the near man"
    );

    // Full charge: hold pass past a full PASS_CHARGE_RATE window, release.
    let (mut s, _near, far) = build();
    let hold = MatchInput {
        r#move: Vec2::new(1.0, 0.0),
        pass_held: true,
        ..MatchInput::default()
    };
    for _ in 0..30 {
        step(&mut s, &hold, &tune);
        if s.events.iter().any(|e| e.kind == MatchEventKind::Pass) {
            break;
        }
    }
    assert!(
        s.players[(far - 1) as usize].receive_timer > 0.0,
        "a full charge reaches past the near man to the far one"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 240, |s| {
        s.owner == Some(far)
    });
    assert!(n < 240, "the charged ball is collected by the far man");
}

#[test]
fn a_lobbed_pass_is_met_at_its_landing_point() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 463.0), Vec2::new(0.0, 0.0));
    let lob = MatchInput {
        r#move: Vec2::new(1.0, 0.0),
        pass: true,
        lob: true,
        ..MatchInput::default()
    };
    step(&mut s, &lob, &tune);
    assert!(
        s.events.iter().any(|e| e.kind == MatchEventKind::Pass),
        "the lob releases"
    );
    assert!(s.ball_vz > 0.0, "a lob leaves the ground");
    assert!(
        s.players[(mate - 1) as usize].receive_timer > 0.0,
        "the mate is the designated receiver of the lob"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 300, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 300, "the lob is gathered by its receiver");
}

#[test]
fn a_dink_over_a_lane_presser_keeps_the_lead_and_arrives() {
    let tune = Tuning::new();
    // An opponent parked 30 px down the lane (inside RELEASE_DINK_DIST)
    // converts the driven ball into a dink. The rework's fix: the arc aims
    // at the SOLVED lead point, not the receiver's stale position.
    let (mut s, _passer, mate, _opp) = contested_setup(
        Vec2::new(850.0, 420.0),
        Vec2::new(0.0, 180.0),
        Vec2::new(430.0, 466.0),
    );
    step(&mut s, &pass_input(Vec2::new(1.0, 0.0)), &tune);
    assert!(
        s.events.iter().any(|e| e.kind == MatchEventKind::Pass),
        "the dink releases"
    );
    assert!(s.ball_vz > 0.0, "the dink arcs over the presser");
    let target = s.players[(mate - 1) as usize]
        .receive_target
        .expect("the dink stores its landing as the rendezvous");
    assert!(
        target.y > 420.0,
        "the dink keeps the solved lead into the run (target {target:?})"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 300, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 300, "the dinked lead is collected by its receiver");
}

#[test]
fn a_slow_ball_on_a_covered_lane_is_intercepted() {
    let tune = Tuning::new();
    // A defender standing 380 px down a 560 px lane: by then the floor-paced
    // ball has decayed under POSSESS_MAX_SPEED and the release cooldown has
    // lapsed, so the interception is legal — a bad pass stays a bad pass.
    let (mut s, passer, mate, opp) = contested_setup(
        Vec2::new(960.0, 463.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(780.0, 463.0),
    );
    release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    let n = step_until(&mut s, &MatchInput::default(), &tune, 240, |s| {
        s.owner.is_some()
    });
    assert!(n < 240, "somebody resolves the covered lane");
    let owner = s.owner.expect("resolved");
    assert_ne!(owner, mate, "the intended receiver never sees this one");
    assert_ne!(owner, passer, "and the passer certainly does not");
    assert_eq!(
        s.players[(owner - 1) as usize].team,
        Team::Away,
        "the covering defender collects"
    );
    assert!(
        s.players.iter().all(|p| p.receive_timer <= 0.0),
        "resolution clears every receiver mark"
    );
    let _ = opp;
}

#[test]
fn a_fast_ball_deflects_off_a_lane_body() {
    let tune = Tuning::new();
    // A defender 250 px down the lane meets the ball at ~420 px/s — above
    // anyone's trap speed, past the block grace: it ricochets rather than
    // sails through, and the ricochet ends the pass for everyone.
    let (mut s, _passer, mate, _opp) = contested_setup(
        Vec2::new(960.0, 463.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(650.0, 463.0),
    );
    release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    let mut blocked = false;
    for _ in 0..60 {
        step(&mut s, &MatchInput::default(), &tune);
        if s.events.iter().any(|e| e.kind == MatchEventKind::Block) {
            blocked = true;
            break;
        }
    }
    assert!(blocked, "the fast ball ricochets off the lane body");
    assert!(
        s.players.iter().all(|p| p.receive_timer <= 0.0),
        "the ricochet ends the pass: nobody is receiving any more"
    );
    assert!(
        s.players[(mate - 1) as usize].receive_target.is_none(),
        "and the stored rendezvous dies with it"
    );
}

#[test]
fn a_touchline_lead_is_clamped_in_bounds_and_still_met() {
    let tune = Tuning::new();
    // The receiver sprints at the bottom touchline: the lead point must be
    // clamped onto the pitch and the pass still completes in play.
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 860.0), Vec2::new(0.0, 220.0));
    let aim = release(&mut s, Vec2::new(1.0, 0.8), mate, &tune);
    assert!(
        aim.y <= 927.0 - 12.0,
        "the lead never leaves the pitch (aim {aim:?})"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 240, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 240, "the touchline lead is collected");
    assert!(
        s.ball.y <= 927.0 && s.ball.x <= 1648.0,
        "reception happens in play"
    );
}

#[test]
fn a_square_aim_at_the_keeper_is_a_deliberate_back_pass() {
    let tune = Tuning::new();
    let (mut s, passer, _mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 0.0));
    let keeper = s
        .players
        .iter()
        .position(|p| p.team == Team::Home && p.is_keeper)
        .map(|i| (i + 1) as i64)
        .expect("home keeper");
    let kpos = s.players[(keeper - 1) as usize].pos;
    let ppos = s.players[(passer - 1) as usize].pos;
    let aim = kpos.sub(ppos).normalized();
    step(&mut s, &pass_input(aim), &tune);
    assert!(
        s.players[(keeper - 1) as usize].receive_timer > 0.0,
        "aiming square at the keeper selects the keeper, however far back"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 400, |s| {
        s.owner == Some(keeper)
    });
    assert!(
        n < 400,
        "the keeper comes to meet the back-pass and takes it"
    );
}

#[test]
fn a_long_ball_at_the_reach_margin_is_led_honestly_and_arrives() {
    let tune = Tuning::new();
    // A 1000 px ball to a receiver still running away: the margin refuses
    // only the candidates the launch cannot roll to, and what remains is a
    // real, collectable lead.
    let mut s = new_match(None);
    let passer = s.controlled;
    s.players[(passer - 1) as usize].pos = Vec2::new(150.0, 463.0);
    s.players[(passer - 1) as usize].facing = Vec2::new(1.0, 0.0);
    let mut mate = None;
    let mut parky = 60.0;
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != passer {
            if mate.is_none() {
                // 940 px out: inside PASS_ELIGIBLE_MAX (960) so selection
                // can pick him at all, far enough that the margin governs
                // which leads survive.
                p.pos = Vec2::new(1090.0, 463.0);
                p.run_vel = Vec2::new(150.0, 0.0);
                p.facing = Vec2::new(1.0, 0.0);
                mate = Some(idx);
            } else {
                p.pos = Vec2::new(120.0, parky);
                parky += 100.0;
            }
        } else if p.team == Team::Away && !p.is_keeper {
            p.pos = Vec2::new(1550.0, 100.0);
        }
    }
    let mate = mate.expect("mate placed");
    s.owner = Some(passer);
    s.ball = Vec2::new(168.0, 463.0);
    let aim = release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    let launch = s.ball_vel.length();
    assert!(
        Vec2::new(150.0, 463.0).dist(aim) * gc_sim::ball_flight::FRICTION
            < launch * gc_sim::pass_lead::REACH_MARGIN,
        "the launch can physically roll to its aim with margin"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 400, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 400, "the long ball arrives and is collected");
}

#[test]
fn a_one_two_returns_the_ball_through_two_control_switches() {
    let tune = Tuning::new();
    // Pass, control follows, pass straight back: two releases, two control
    // switches, two latches armed and retired — the full scripted loop.
    let (mut s, passer, mate) =
        running_receiver_setup(Vec2::new(760.0, 463.0), Vec2::new(0.0, 0.0));
    release(&mut s, Vec2::new(1.0, 0.0), mate, &tune);
    assert_eq!(s.controlled, mate, "control follows the first ball");
    let n = step_until(&mut s, &MatchInput::default(), &tune, 120, |s| {
        s.owner == Some(mate)
    });
    assert!(n < 120, "the first ball is collected");
    // Return it first time: aim straight back at the original passer.
    let mut released = false;
    for _ in 0..30 {
        step(&mut s, &pass_input(Vec2::new(-1.0, 0.0)), &tune);
        if s.players[(passer - 1) as usize].receive_timer > 0.0 {
            released = true;
            break;
        }
    }
    assert!(
        released,
        "the return ball releases toward the original passer"
    );
    assert_eq!(s.controlled, passer, "control follows the return ball too");
    assert!(
        s.stick_latch.is_some(),
        "the second handoff re-arms the stale-stick latch"
    );
    let n = step_until(&mut s, &MatchInput::default(), &tune, 120, |s| {
        s.owner == Some(passer)
    });
    assert!(n < 120, "the one-two comes back to its starter");
}

// ---------------------------------------------------------------------
// Input-noise resilience: real sticks are never neutral at the handoff.
// The latch must forgive the CONTINUED gesture (aim wobble included) and
// nothing else — a fresh or clearly-changed gesture wins outright, so
// input always matters.
// ---------------------------------------------------------------------

/// `dir` rotated by `deg` degrees.
fn rot(dir: Vec2, deg: f64) -> Vec2 {
    let r = deg.to_radians();
    let (s, c) = (r.sin(), r.cos());
    Vec2::new(dir.x * c - dir.y * s, dir.x * s + dir.y * c)
}

/// A passer holding `(1, 0)` whose soft-cone receiver sits ~58° OFF that
/// axis: the launch bearing and the held gesture are genuinely different
/// vectors, which is what separates residue-anchoring from
/// launch-anchoring.
fn off_axis_setup() -> (MatchState, i64, i64) {
    running_receiver_setup(Vec2::new(620.0, 60.0), Vec2::new(150.0, 0.0))
}

#[test]
fn motion_residue_stays_stale_even_for_an_off_axis_receiver() {
    let tune = Tuning::new();
    // The passer was moving along (1, 0) and passes off that same held
    // stick; the cone picks the only forward option, ~52° off-axis. The
    // held stick is farther from the LAUNCH bearing than the latch cone —
    // anchoring on the launch would misread the residue as a deliberate
    // run and strand the receiver. Anchoring on the residue keeps the
    // assist steering.
    let (mut s, _passer, mate) = off_axis_setup();
    let held = Vec2::new(1.0, 0.0);
    release(&mut s, held, mate, &tune);
    let latch = s.stick_latch.expect("the handoff latches the residue");
    assert!(
        latch.x * held.x + latch.y * held.y > 0.99,
        "the latch anchors on the held gesture, not the launch bearing ({latch:?})"
    );
    let launch_dir = s.ball_vel.normalized();
    assert!(
        launch_dir.x * held.x + launch_dir.y * held.y < 0.7,
        "fixture check: the launch really is outside the latch cone of the hold"
    );
    // The player keeps holding exactly what they held: pure residue.
    let n = step_until(&mut s, &move_input(held), &tune, 240, |s| {
        s.owner == Some(mate)
    });
    assert!(
        n < 240,
        "the residue-held receiver is still assisted onto the pass"
    );
}

#[test]
fn analog_wobble_around_the_held_aim_stays_stale() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let held = Vec2::new(1.0, 0.0);
    release(&mut s, held, mate, &tune);
    assert!(s.stick_latch.is_some());
    // A real thumb never holds a vector: oscillate ±20° around the aim,
    // changing every few ticks. Every sample stays inside the latch cone,
    // so the whole wobble reads as one continued gesture.
    let mut collected = false;
    for i in 0..240 {
        let wobble = rot(held, if (i / 4) % 2 == 0 { 20.0 } else { -20.0 });
        step(&mut s, &move_input(wobble), &tune);
        assert!(
            s.owner.is_some() || s.stick_latch.is_some(),
            "wobble inside the cone must never clear the latch (tick {i})"
        );
        if s.owner == Some(mate) {
            collected = true;
            break;
        }
    }
    assert!(
        collected,
        "the wobbling hold is still assisted onto the pass"
    );
}

#[test]
fn a_drift_past_the_cone_becomes_intent_and_input_wins() {
    let tune = Tuning::new();
    let (mut s, _passer, mate) =
        running_receiver_setup(Vec2::new(900.0, 400.0), Vec2::new(0.0, 200.0));
    let held = Vec2::new(1.0, 0.0);
    release(&mut s, held, mate, &tune);
    assert!(s.stick_latch.is_some());
    // The thumb rolls away from the aim in 20° steps. 0/20/40 stay inside
    // the cone (still the same gesture); 60 crosses it — that tick is a
    // decision, the latch dies, and from then on the input steers the
    // receiver AWAY from the pass. Resilient, not overfit: drift far
    // enough and your input matters again.
    for deg in [0.0, 20.0, 40.0] {
        step(&mut s, &move_input(rot(held, deg)), &tune);
        assert!(
            s.stick_latch.is_some(),
            "a drift inside the cone is still residue ({deg}°)"
        );
    }
    let intent = rot(held, 60.0);
    step(&mut s, &move_input(intent), &tune);
    assert!(
        s.stick_latch.is_none(),
        "crossing the cone is a decision: the latch clears that tick"
    );
    let before = s.players[(mate - 1) as usize].pos;
    for _ in 0..30 {
        step(&mut s, &move_input(intent), &tune);
    }
    let after = s.players[(mate - 1) as usize].pos;
    let moved = after.sub(before);
    assert!(
        moved.x * intent.x + moved.y * intent.y > 10.0,
        "the receiver follows the deliberate input, not the assist"
    );
}
