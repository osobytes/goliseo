//! Tests for the grounded first-touch shot (#623): a designated receiver
//! striking an arriving pass first time at the moment collection would
//! otherwise grant them possession.
//!
//! The resolution is seeded (Clean/Heavy/Miss), so outcome-specific cases
//! scan a bounded seed list for the outcome they exercise and assert the
//! property there — deterministic across runs, and the scan panics if the
//! outcome never appears so a probability regression cannot pass silently.

use gc_core::vec2::Vec2;
use gc_sim::aerial::AerialOutcome;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;

fn new_match_seeded(seed: f64, human_controlled: Option<bool>) -> MatchState {
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
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled,
        input_ownership: None,
    })
}

/// First home outfielder, one-based.
fn home_outfielder(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no home outfielder");
}

/// Stage the moment collection fires: `receiver` (one-based) is the
/// designated receiver of a pass arriving at `ball_speed` from the east,
/// already inside possession reach, everyone else parked far away.
fn stage_arrival(s: &mut MatchState, receiver: i64, receiver_pos: Vec2, ball_speed: f64) {
    for (i, p) in s.players.iter_mut().enumerate() {
        let row = 40.0 + 30.0 * i as f64;
        p.pos = if p.team == Team::Home {
            Vec2::new(90.0, row)
        } else {
            Vec2::new(480.0, row) // far midfield: no pressure on the receiver
        };
        p.vel = Vec2::new(0.0, 0.0);
        p.run_vel = Vec2::new(0.0, 0.0);
        p.receive_timer = 0.0;
    }
    let rp = &mut s.players[(receiver - 1) as usize];
    rp.pos = receiver_pos;
    rp.receive_timer = 1.0;
    rp.facing = Vec2::new(1.0, 0.0);
    s.owner = None;
    s.kickoff_hold = 0.0;
    s.pickup_cd = 0.0;
    s.aerial_lock = 0.0;
    s.ball = receiver_pos.add(Vec2::new(10.0, 0.0));
    s.ball_vel = Vec2::new(-ball_speed, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    s.ball_spin = 0.0;
}

/// The exact shape `slot_input::to_match_input` produces while the ACTION
/// button is held off the ball: jockey AND aerial_strike together, plus the
/// held aim. Tests use this rather than a bare `aerial_strike` so the cases
/// exercise the wiring a real space-bar hold goes through.
fn strike_input(aim: Vec2) -> MatchInput {
    MatchInput {
        r#move: aim,
        jockey: true,
        aerial_strike: Some(true),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

fn first_touch_event(s: &MatchState) -> Option<&gc_sim::match_snapshot::MatchEvent> {
    s.events
        .iter()
        .find(|e| e.kind == MatchEventKind::FirstTouchShot)
}

const RECEIVER_POS: Vec2 = Vec2 { x: 700.0, y: 200.0 };

#[test]
fn a_receiver_holding_strike_shoots_first_time_instead_of_trapping() {
    // A maxed volley skill makes Clean overwhelmingly likely per seed; the
    // scan pins the first Clean seed deterministically.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        sim_match::set_controlled_player(&mut s, receiver);
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &Tuning::new(),
        );
        let Some(event) = first_touch_event(&s) else {
            panic!("a designated receiver holding strike must attempt the first-touch shot");
        };
        assert_eq!(
            s.owner, None,
            "a first-touch shot must never grant possession"
        );
        if event.outcome == Some(AerialOutcome::Clean) {
            // Clean angular error is at most 0.06 rad: the shot follows the
            // held stick, not the goal and not the arriving ball's line.
            let dir = s.ball_vel.normalized();
            assert!(
                dir.x * aim.x + dir.y * aim.y > 0.995,
                "clean first-touch shot must follow the aim (got {dir:?})"
            );
            assert!(
                s.ball_vel.length() > 250.0,
                "a clean one-timer outpaces the pass that fed it"
            );
            return;
        }
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

#[test]
fn a_receiver_not_holding_strike_traps_normally() {
    let mut s = new_match_seeded(7.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    sim_match::set_controlled_player(&mut s, receiver);
    let input = MatchInput {
        aerial_strike: Some(false),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    };
    sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &Tuning::new());
    assert_eq!(first_touch_event(&s), None);
    assert_eq!(
        s.owner,
        Some(receiver),
        "without the strike held, the designated receiver traps as before"
    );
}

#[test]
fn a_keeper_receiving_a_back_pass_never_strikes_first_time() {
    let mut s = new_match_seeded(3.0, Some(false));
    let keeper = (1..=s.players.len() as i64)
        .find(|&i| {
            let p = &s.players[(i - 1) as usize];
            p.team == Team::Home && p.is_keeper
        })
        .expect("home keeper");
    let keeper_pos = s.players[(keeper - 1) as usize].pos;
    stage_arrival(&mut s, keeper, keeper_pos, 200.0);
    // Even with the AI range covering the whole pitch, the keeper is out.
    let mut tune = Tuning::new();
    tune.set("AI_FIRST_TOUCH_RANGE", 2000.0);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(MatchInput::default()),
        None,
        &tune,
    );
    assert_eq!(first_touch_event(&s), None);
    assert_eq!(
        s.owner,
        Some(keeper),
        "a keeper meeting a teammate's pass takes it with the feet"
    );
}

#[test]
fn an_ai_receiver_inside_its_range_takes_the_one_timer() {
    // 260 px from the away goal-line centre, inside the 360 default.
    let mut s = new_match_seeded(11.0, Some(false));
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, Vec2::new(700.0, 270.0), 250.0);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(MatchInput::default()),
        None,
        &Tuning::new(),
    );
    assert!(
        first_touch_event(&s).is_some(),
        "an AI receiver in one-timer range must attempt the first-touch shot"
    );
    assert_eq!(s.owner, None);
}

#[test]
fn an_ai_receiver_outside_its_range_traps() {
    // 560 px out: beyond the 360 default, so the AI settles it.
    let mut s = new_match_seeded(11.0, Some(false));
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, Vec2::new(400.0, 270.0), 250.0);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(MatchInput::default()),
        None,
        &Tuning::new(),
    );
    assert_eq!(first_touch_event(&s), None);
    assert_eq!(s.owner, Some(receiver));
}

#[test]
fn range_zero_turns_the_ai_verb_off() {
    let mut s = new_match_seeded(11.0, Some(false));
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, Vec2::new(700.0, 270.0), 250.0);
    let mut tune = Tuning::new();
    tune.set("AI_FIRST_TOUCH_RANGE", 0.0);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(MatchInput::default()),
        None,
        &tune,
    );
    assert_eq!(first_touch_event(&s), None);
    assert_eq!(s.owner, Some(receiver));
}

#[test]
fn an_aerial_whiff_moments_earlier_does_not_lock_out_the_grounded_swing() {
    // The play-test failure shape (#623 follow-up): a dinked pass draws one
    // airborne swing on the way down; its 0.5 s `header_cd` must not turn
    // the ball landing a beat later into a forced plain trap. Recovery
    // (the animation actually in progress) still gates, so the fixture
    // models the moment recovery has just ended with the cooldown live.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        {
            let rp = &mut s.players[(receiver - 1) as usize];
            rp.volley_skill = 1.0;
            rp.header_cd = 0.4; // an aerial attempt 0.1 s ago
            rp.aerial_recovery = 0.0; // ... whose recovery has finished
        }
        sim_match::set_controlled_player(&mut s, receiver);
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &Tuning::new(),
        );
        if let Some(event) = first_touch_event(&s) {
            assert_eq!(s.owner, None);
            if event.outcome == Some(AerialOutcome::Clean) {
                return;
            }
        } else {
            panic!("a live header_cd must not force the receiver into a plain trap");
        }
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

#[test]
fn a_whiffed_first_touch_leaves_the_ball_loose_and_costs_the_designation() {
    // Zero volley skill against a driven ball: the margin goes negative and
    // Miss appears within a short seed scan.
    for seed in 0..60 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 340.0);
        s.players[(receiver - 1) as usize].volley_skill = 0.0;
        sim_match::set_controlled_player(&mut s, receiver);
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
            None,
            &Tuning::new(),
        );
        let Some(event) = first_touch_event(&s) else {
            panic!("the attempt itself must fire regardless of skill");
        };
        if event.outcome == Some(AerialOutcome::Miss) {
            assert_eq!(s.owner, None, "a whiff must not become a trap");
            assert!(
                s.ball_vel.x < 0.0,
                "the whiffed ball keeps rolling on its own line"
            );
            assert_eq!(
                s.players[(receiver - 1) as usize].receive_timer,
                0.0,
                "swinging spends the receiver designation"
            );
            assert!(
                s.pickup_cd > 0.0,
                "the whiff leaves the ball briefly ungrabbable, so it runs through"
            );
            return;
        }
    }
    panic!("no seed in 0..60 produced a Miss at volley skill 0.0");
}

// ---------------------------------------------------------------------
// The receive window (#623 follow-up): the designation must outlive the
// pass's own flight, or the verb silently never arms on a long floor pass.
// ---------------------------------------------------------------------

#[test]
fn the_receive_window_outlives_the_flight_at_every_legal_range() {
    // Ground flight under exponential grass friction: t = -ln(1 - F*d/v0)/F.
    // At every legal aim distance, the window must exceed that flight.
    let tune = Tuning::new();
    let friction = gc_sim::ball_flight::FRICTION;
    for d in [100.0_f64, 300.0, 500.0, 700.0, 890.0] {
        let v0 = gc_sim::passing::speed_for(d, &tune);
        let flight = -(1.0 - friction * d / v0).ln() / friction;
        let window = sim_match::receive_window(v0, d, false);
        assert!(
            window > flight,
            "at {d} px the window ({window:.2}) must outlive the flight ({flight:.2})"
        );
    }
}

#[test]
fn a_lob_and_a_dying_roll_get_the_capped_window() {
    // A lob's flight is an arc plus a roll, not one exponential roll, and a
    // dying roll's nominal flight never completes at all -- both take the
    // ceiling instead of an estimate.
    let lob = sim_match::receive_window(900.0, 400.0, true);
    let dying = sim_match::receive_window(300.0, 400.0, false);
    assert_eq!(lob, dying, "both unestimable shapes take the same ceiling");
    assert!(
        lob > 2.0,
        "the ceiling must comfortably exceed the old flat window"
    );
}

#[test]
fn a_max_range_floor_pass_still_first_touches_end_to_end() {
    // The play-test failure (#623 follow-up), as a full real flow rather
    // than a staged arrival: charge a pass to a teammate 890 px away with
    // every other body parked far behind the aim, release, then hold the
    // strike with a neutral stick -- the receive assist walks the receiver
    // to the meet point and the first touch fires at collection. Under the
    // old flat 1.3 s window the designation expired mid-flight and this
    // exact flow ended in a dead ball.
    let mut s = new_match_seeded(9.0, None);
    let tune = Tuning::new();
    let owner = 2_i64;
    let ridx = 3_i64;
    s.kickoff_hold = 0.0;
    for (i, p) in s.players.iter_mut().enumerate() {
        p.pos = Vec2::new(120.0 + 30.0 * i as f64, 60.0);
        p.receive_timer = 0.0;
    }
    s.players[(owner - 1) as usize].pos = Vec2::new(300.0, 700.0);
    s.players[(owner - 1) as usize].facing = Vec2::new(1.0, 0.0);
    s.players[(ridx - 1) as usize].pos = Vec2::new(1190.0, 700.0);
    sim_match::set_controlled_player(&mut s, owner);
    s.owner = Some(owner);
    s.ball = Vec2::new(300.0, 700.0);
    s.ball_z = 0.0;
    let aim = Vec2::new(1.0, 0.0);
    for _ in 0..30 {
        let input = MatchInput {
            r#move: aim,
            pass_held: true,
            ..MatchInput::default()
        };
        sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &tune);
    }
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(MatchInput {
            r#move: aim,
            pass: true,
            ..MatchInput::default()
        }),
        None,
        &tune,
    );
    assert!(
        s.players[(ridx - 1) as usize].receive_timer > 0.0,
        "the far teammate must be the designated receiver"
    );
    for _ in 0..400 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(Vec2::new(0.0, 0.0))),
            None,
            &tune,
        );
        if first_touch_event(&s).is_some() {
            return;
        }
        assert_eq!(
            s.owner, None,
            "nobody may win the ball before the receiver's first touch"
        );
    }
    panic!("the max-range floor pass never produced a first touch");
}

#[test]
fn aiming_the_shot_does_not_steer_the_receiver_off_the_pass() {
    // The owner's gesture (#623 follow-up 3): hold the strike AND a stick
    // direction — the direction is where the SHOT should go, not where the
    // body should run. Under stick-overrides-assist the receiver ran off
    // the meet line and the pass died; now the assist keeps them on it and
    // the clean first touch follows the held aim. Same full real flow as
    // `a_max_range_floor_pass_still_first_touches_end_to_end`, with the aim
    // held north the entire flight.
    let shot_aim = Vec2::new(0.0, -1.0);
    for seed in 0..12 {
        let mut s = new_match_seeded(seed as f64, None);
        let tune = Tuning::new();
        let owner = 2_i64;
        let ridx = 3_i64;
        s.kickoff_hold = 0.0;
        for (i, p) in s.players.iter_mut().enumerate() {
            p.pos = Vec2::new(120.0 + 30.0 * i as f64, 60.0);
            p.receive_timer = 0.0;
        }
        s.players[(owner - 1) as usize].pos = Vec2::new(300.0, 700.0);
        s.players[(owner - 1) as usize].facing = Vec2::new(1.0, 0.0);
        {
            let rp = &mut s.players[(ridx - 1) as usize];
            rp.pos = Vec2::new(1000.0, 700.0);
            rp.volley_skill = 1.0;
        }
        sim_match::set_controlled_player(&mut s, owner);
        s.owner = Some(owner);
        s.ball = Vec2::new(300.0, 700.0);
        s.ball_z = 0.0;
        let pass_aim = Vec2::new(1.0, 0.0);
        for _ in 0..30 {
            let input = MatchInput {
                r#move: pass_aim,
                pass_held: true,
                ..MatchInput::default()
            };
            sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &tune);
        }
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(MatchInput {
                r#move: pass_aim,
                pass: true,
                ..MatchInput::default()
            }),
            None,
            &tune,
        );
        assert!(s.players[(ridx - 1) as usize].receive_timer > 0.0);
        let mut fired = false;
        for _ in 0..400 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(shot_aim)),
                None,
                &tune,
            );
            if let Some(event) = first_touch_event(&s) {
                if event.outcome == Some(AerialOutcome::Clean) {
                    let dir = s.ball_vel.normalized();
                    assert!(
                        dir.x * shot_aim.x + dir.y * shot_aim.y > 0.995,
                        "the clean first touch must follow the held aim (got {dir:?})"
                    );
                    return;
                }
                fired = true;
                break; // Heavy/Miss: the attempt happened; try another seed for Clean.
            }
            assert_eq!(
                s.owner, None,
                "aiming the shot must not cost the receiver the ball"
            );
        }
        assert!(
            fired,
            "holding an aim during the flight must still produce the first-touch attempt"
        );
    }
    panic!("no seed in 0..12 produced a Clean aimed first touch at volley skill 1.0");
}

#[test]
fn the_striker_does_not_block_their_own_first_touch() {
    // Observed live (2026-08-28 debug log): a Clean first touch followed
    // one tick later by a Block BY THE STRIKER -- the arriving ball sits on
    // the near side of the body, the shot fires "through" it, and without
    // release grace the body block eats the shot. The fix is the same
    // `block_grace` every ordinary release takes; this pins it.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        sim_match::set_controlled_player(&mut s, receiver);
        let tune = Tuning::new();
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );
        let Some(event) = first_touch_event(&s) else {
            panic!("the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let launch_speed = s.ball_vel.length();
        for _ in 0..8 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert!(
                !s.events.iter().any(|e| e.kind == MatchEventKind::Block),
                "the striker's own body must not block the release"
            );
        }
        let dir = s.ball_vel.normalized();
        assert!(
            dir.x * aim.x + dir.y * aim.y > 0.9,
            "the shot must still be travelling where it was aimed"
        );
        assert!(
            s.ball_vel.length() > launch_speed * 0.7,
            "the shot must still carry its pace after leaving the body"
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}
