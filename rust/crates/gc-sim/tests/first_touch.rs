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

fn strike_input(aim: Vec2) -> MatchInput {
    MatchInput {
        r#move: aim,
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
