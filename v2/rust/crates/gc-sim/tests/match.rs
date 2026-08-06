//! Port of `spec/sim/match_spec.lua`.
//!
//! This is a representative subset, not the full 3,642-line original — see
//! the porting agent's final report for exactly which `describe` blocks
//! landed and which remain a follow-up. Every case ported here is a
//! faithful, unabridged translation of its Lua counterpart.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

fn new_match() -> MatchState {
    new_match_opts(None, false)
}

fn new_match_opts(
    tactic: Option<&'static gc_data::tactics::TacticData>,
    human_controlled_false: bool,
) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: None,
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: if human_controlled_false {
            Some(false)
        } else {
            None
        },
        input_ownership: None,
    })
}

#[derive(Default, Clone, Copy)]
struct InputOpts {
    r#move: Vec2,
    shoot: bool,
    shoot_held: bool,
    pass: bool,
    dash: bool,
    switch: bool,
    sprint: bool,
}

fn input(o: InputOpts) -> MatchInput {
    MatchInput {
        r#move: o.r#move,
        shoot: o.shoot,
        shoot_held: o.shoot_held,
        pass: o.pass,
        dash: o.dash,
        switch: o.switch,
        sprint: o.sprint,
        ..MatchInput::default()
    }
}

fn no_input() -> MatchInput {
    MatchInput::default()
}

fn step(s: &mut MatchState, dt: f64, i: &MatchInput, tune: &Tuning) {
    sim_match::step(s, dt, StepInput::Legacy(*i), None, tune);
}

// Frames needed to advance past the 0.15s shot wind-up at 1/60 s per step.
const WINDUP_FRAMES: u32 = 10;

fn step_frames(s: &mut MatchState, n: u32, tune: &Tuning) {
    for _ in 0..n {
        step(s, 1.0 / 60.0, &no_input(), tune);
    }
}

// ---------------------------------------------------------------------
// match.new
// ---------------------------------------------------------------------

#[test]
fn kicks_off_with_10_players_and_the_home_side_in_possession() {
    let s = new_match();
    assert_eq!(s.players.len(), 10);
    assert!(
        s.owner == Some(s.controlled),
        "controlled player should start with the ball"
    );
    assert_eq!(s.score.home, 0);
    assert_eq!(s.score.away, 0);
    assert!(s.players[(s.controlled - 1) as usize].team == Team::Home);
    assert!(!s.players[(s.controlled - 1) as usize].is_keeper);
}

#[test]
fn lets_match_ai_drive_both_teams_when_no_player_is_human_controlled() {
    let tune = Tuning::new();
    let assert_ai_owner_moves = |owner_idx: i64, direction: f64| {
        let mut s = new_match_opts(None, true);
        let before_x = s.players[(owner_idx - 1) as usize].pos.x;
        s.owner = Some(owner_idx);
        s.ball = s.players[(owner_idx - 1) as usize].pos;

        step(&mut s, 1.0 / 60.0, &no_input(), &tune);

        let team = s.players[(owner_idx - 1) as usize].team;
        assert!(
            (s.players[(owner_idx - 1) as usize].pos.x - before_x) * direction > 0.0,
            "{team:?} owner should dribble toward the opposing goal"
        );
    };

    let opening_owner = new_match().controlled;
    assert_ai_owner_moves(opening_owner, 1.0);
    assert_ai_owner_moves(7, -1.0);
}

// ---------------------------------------------------------------------
// match.step timer
// ---------------------------------------------------------------------

#[test]
fn counts_down_and_ends_at_full_time() {
    let tune = Tuning::new();
    let mut s = new_match();
    step(&mut s, 10.0, &no_input(), &tune);
    assert!((s.time_left - 110.0).abs() < 1e-6);
    assert!(!s.finished);
    step(&mut s, 200.0, &no_input(), &tune);
    assert!(s.finished);
    assert_eq!(s.time_left, 0.0);
}

// ---------------------------------------------------------------------
// match.step shooting & passing
// ---------------------------------------------------------------------

#[test]
fn shooting_releases_the_ball_toward_the_goal() {
    let tune = Tuning::new();
    let mut s = new_match();
    s.players[(s.controlled - 1) as usize].facing = Vec2::new(1.0, 0.0);
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            shoot: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    // Ball is still owned during the wind-up; step past it.
    step_frames(&mut s, WINDUP_FRAMES, &tune);
    assert!(s.owner.is_none());
    assert!(s.ball_vel.x > 0.0, "home shoots toward the right goal");
}

#[test]
fn aiming_up_sends_the_shot_to_the_top_corner() {
    let tune = Tuning::new();
    let mut s = new_match();
    s.players[(s.controlled - 1) as usize].facing = Vec2::new(0.0, -1.0);
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            shoot: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    step_frames(&mut s, WINDUP_FRAMES, &tune);
    assert!(s.owner.is_none());
    assert!(s.ball_vel.x > 0.0, "still goal-ward");
    assert!(s.ball_vel.y < 0.0, "and toward the top corner");
}

#[test]
fn passing_sends_the_ball_toward_a_teammate_in_the_aim_direction() {
    let tune = Tuning::new();
    let mut s = new_match();
    let controlled = s.controlled;
    let owner_pos = s.players[(controlled - 1) as usize].pos;
    let mut mate_pos = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && idx != controlled && !p.is_keeper {
            mate_pos = Some(p.pos);
            break;
        }
    }
    let mate_pos = mate_pos.expect("home fixture has an outfielder besides the controlled one");
    s.players[(controlled - 1) as usize].facing = mate_pos.sub(owner_pos).normalized();
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            pass: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    assert!(s.owner.is_none(), "ball should be released on a pass");
    assert!((s.ball_vel.length() - 420.0).abs() < 0.5, "pass speed");
}

// ---------------------------------------------------------------------
// match.step charge shot
// ---------------------------------------------------------------------

fn shot_speed(charge: f64, tune: &Tuning) -> f64 {
    let mut s = new_match();
    let controlled = s.controlled;
    s.players[(controlled - 1) as usize].facing = Vec2::new(1.0, 0.0);
    s.players[(controlled - 1) as usize].charge = charge;
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            shoot: true,
            ..InputOpts::default()
        }),
        tune,
    );
    step_frames(&mut s, WINDUP_FRAMES, tune);
    s.ball_vel.length()
}

#[test]
fn a_full_charge_shoots_meaningfully_harder_than_a_tap() {
    let tune = Tuning::new();
    assert!(shot_speed(1.0, &tune) > shot_speed(0.0, &tune) * 1.5);
}

#[test]
fn holding_shoot_builds_charge() {
    let tune = Tuning::new();
    let mut s = new_match();
    step(
        &mut s,
        0.1,
        &input(InputOpts {
            shoot_held: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    assert!(s.players[(s.controlled - 1) as usize].charge > 0.0);
}

// ---------------------------------------------------------------------
// match.step juke
// ---------------------------------------------------------------------

#[test]
fn a_dodging_carrier_is_immune_to_tackles() {
    let tune = Tuning::new();
    let mut s = new_match();
    let controlled = s.controlled;
    let me_pos = s.players[(controlled - 1) as usize].pos;
    let mut foe_idx = None;
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Away && !p.is_keeper {
            foe_idx = Some(i);
            break;
        }
    }
    let foe_idx = foe_idx.expect("away fixture has an outfielder");
    s.players[foe_idx].pos = Vec2::new(me_pos.x + 8.0, me_pos.y);
    s.players[foe_idx].dash_cd = 0.0;
    s.players[(controlled - 1) as usize].dodge_timer = 1.0;
    step(&mut s, 0.016, &no_input(), &tune);
    assert!(
        s.owner == Some(controlled),
        "dodging carrier keeps the ball"
    );
}

// ---------------------------------------------------------------------
// match.step tackling
// ---------------------------------------------------------------------

fn carrier_setup() -> (MatchState, i64) {
    let mut s = new_match();
    let mut away_idx = None;
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Away && !p.is_keeper {
            away_idx = Some((i + 1) as i64);
            break;
        }
    }
    let away_idx = away_idx.expect("away fixture has an outfielder");
    s.owner = Some(away_idx);
    // Challenges reach for the ball, so put it at the carrier's feet.
    let c = s.players[(away_idx - 1) as usize].clone();
    s.ball = c.pos.add(c.facing.scale(18.0));
    // Park the carrier's teammates out of passing range so it can't bail
    // out of the challenge with a pressure pass.
    for (i, p) in s.players.iter_mut().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Away && !p.is_keeper && idx != away_idx {
            p.pos = Vec2::new(40.0, 380.0 + idx as f64 * 15.0);
        }
    }
    (s, away_idx)
}

#[test]
fn a_standing_tackle_slow_knocks_the_ball_loose() {
    let tune = Tuning::new();
    let (mut s, away_idx) = carrier_setup();
    let carrier_pos = s.players[(away_idx - 1) as usize].pos;
    let controlled = s.controlled;
    // stand on the BALL side (in front of the carrier), inside poke reach
    {
        let me = &mut s.players[(controlled - 1) as usize];
        me.pos = Vec2::new(carrier_pos.x - 26.0, carrier_pos.y);
        me.vel = Vec2::new(0.0, 0.0); // standing -> standing poke
    }
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            dash: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    assert!(
        s.owner != Some(away_idx),
        "carrier loses possession to the standing tackle"
    );
}

#[test]
fn a_slide_sprinting_wins_the_ball_from_further_away_and_stuns_the_carrier() {
    let tune = Tuning::new();
    let (mut s, away_idx) = carrier_setup();
    let carrier_pos = s.players[(away_idx - 1) as usize].pos;
    let controlled = s.controlled;
    // approach the BALL side (in front of the carrier), sprinting into a
    // slide
    {
        let me = &mut s.players[(controlled - 1) as usize];
        me.pos = Vec2::new(carrier_pos.x - 32.0, carrier_pos.y);
        me.vel = Vec2::new(200.0, 0.0);
        me.sprinting = true; // sprint + tackle = slide
    }
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            dash: true,
            r#move: Vec2::new(1.0, 0.0),
            sprint: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    assert!(
        s.owner != Some(away_idx),
        "slide wins the ball at extended reach"
    );
    assert!(
        s.players[(away_idx - 1) as usize].stun_timer > 0.0,
        "the slid-through carrier is knocked off balance"
    );
}

#[test]
fn a_carrier_shields_the_ball_from_a_challenge_behind_them() {
    let tune = Tuning::new();
    let (mut s, away_idx) = carrier_setup();
    let controlled = s.controlled;
    s.players[(away_idx - 1) as usize].facing = Vec2::new(-1.0, 0.0); // ball sticks a step toward -x
    let carrier_pos = s.players[(away_idx - 1) as usize].pos;
    s.ball = carrier_pos.add(Vec2::new(-18.0, 0.0));
    let mut defender = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != controlled {
            defender = Some(idx);
            break;
        }
    }
    let defender = defender.expect("home fixture has a non-controlled outfielder");
    {
        let d = &mut s.players[(defender - 1) as usize];
        d.pos = Vec2::new(carrier_pos.x + 20.0, carrier_pos.y); // on their back
        d.dash_cd = 0.0;
        d.composure = 0.0; // legacy low-discipline dive-in fixture
    }
    s.players[(controlled - 1) as usize].pos = Vec2::new(60.0, 60.0); // human well away
    s.kickoff_hold = 0.0;
    step(&mut s, 0.016, &no_input(), &tune);
    assert_eq!(
        s.owner,
        Some(away_idx),
        "the shielded ball stays with the carrier"
    );
    assert!(
        s.players[(defender - 1) as usize].dash_cd > 0.0,
        "the failed poke still goes on cooldown"
    );
}

#[test]
fn the_same_challenge_from_the_ball_side_wins_it() {
    let tune = Tuning::new();
    let (mut s, away_idx) = carrier_setup();
    let controlled = s.controlled;
    s.players[(away_idx - 1) as usize].facing = Vec2::new(-1.0, 0.0);
    let carrier_pos = s.players[(away_idx - 1) as usize].pos;
    s.ball = carrier_pos.add(Vec2::new(-18.0, 0.0));
    let mut defender = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != controlled {
            defender = Some(idx);
            break;
        }
    }
    let defender = defender.expect("home fixture has a non-controlled outfielder");
    {
        let d = &mut s.players[(defender - 1) as usize];
        d.pos = Vec2::new(carrier_pos.x - 20.0, carrier_pos.y); // goal side, on the ball
        d.dash_cd = 0.0;
        d.composure = 0.0; // legacy low-discipline dive-in fixture
    }
    s.players[(controlled - 1) as usize].pos = Vec2::new(60.0, 60.0);
    s.kickoff_hold = 0.0;
    step(&mut s, 0.016, &no_input(), &tune);
    assert!(
        s.owner != Some(away_idx),
        "a front-on challenge dislodges the ball"
    );
}

#[test]
fn the_human_can_poke_the_ball_loose_from_behind_at_contact_range() {
    let tune = Tuning::new();
    let (mut s, away_idx) = carrier_setup();
    let carrier_pos = s.players[(away_idx - 1) as usize].pos;
    let controlled = s.controlled;
    {
        let me = &mut s.players[(controlled - 1) as usize];
        me.pos = Vec2::new(carrier_pos.x + 24.0, carrier_pos.y); // on the carrier's back
        me.vel = Vec2::new(0.0, 0.0);
    }
    // Chase while poking: the carrier dribbles away during the frame.
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            dash: true,
            r#move: Vec2::new(-1.0, 0.0),
            ..InputOpts::default()
        }),
        &tune,
    );
    assert!(
        s.owner != Some(away_idx),
        "a contact-range poke wins even from behind"
    );
}

#[test]
fn a_jogging_non_sprint_tackle_is_a_poke_not_a_slide() {
    let tune = Tuning::new();
    let mut s = new_match();
    let controlled = s.controlled;
    s.players[(controlled - 1) as usize].vel = Vec2::new(-150.0, 0.0); // moving, but not sprinting
    step(
        &mut s,
        0.001,
        &input(InputOpts {
            dash: true,
            r#move: Vec2::new(-1.0, 0.0),
            ..InputOpts::default()
        }),
        &tune,
    );
    let me = &s.players[(controlled - 1) as usize];
    assert!(me.slide_timer <= 0.0, "no committed slide without sprint");
    assert!(me.tackle_timer > 0.0, "a standing poke instead");
}

#[test]
fn slide_speed_scales_with_current_velocity() {
    let tune = Tuning::new();
    let slide_vel = |speed: f64| -> f64 {
        let mut s = new_match();
        let controlled = s.controlled;
        {
            let me = &mut s.players[(controlled - 1) as usize];
            me.vel = Vec2::new(speed, 0.0);
            me.facing = Vec2::new(1.0, 0.0);
            me.sprinting = true;
        }
        step(
            &mut s,
            0.001,
            &input(InputOpts {
                dash: true,
                r#move: Vec2::new(1.0, 0.0),
                sprint: true,
                ..InputOpts::default()
            }),
            &tune,
        );
        s.players[(controlled - 1) as usize].slide_vel
    };
    assert!(
        slide_vel(300.0) > slide_vel(150.0),
        "a faster run produces a faster slide"
    );
}

#[test]
fn a_stunned_defender_cannot_tackle() {
    let tune = Tuning::new();
    let (mut s, away_idx) = carrier_setup();
    let carrier_pos = s.players[(away_idx - 1) as usize].pos;
    let controlled = s.controlled;
    {
        let me = &mut s.players[(controlled - 1) as usize];
        me.pos = Vec2::new(carrier_pos.x + 8.0, carrier_pos.y);
        me.vel = Vec2::new(0.0, 0.0);
        me.stun_timer = 1.0;
    }
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            dash: true,
            ..InputOpts::default()
        }),
        &tune,
    );
    assert_eq!(
        s.owner,
        Some(away_idx),
        "a stunned player can't win the ball"
    );
}

// ---------------------------------------------------------------------
// match.step switching
// ---------------------------------------------------------------------

#[test]
fn hands_control_to_the_home_outfielder_nearest_the_ball() {
    let tune = Tuning::new();
    let mut s = new_match();
    let before = s.controlled;
    // Park a loose ball next to a specific teammate.
    let mut target = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != before {
            target = Some(idx);
        }
    }
    let target = target.expect("home fixture has another outfielder");
    s.owner = None;
    s.pickup_cd = 1.0;
    let target_pos = s.players[(target - 1) as usize].pos;
    s.ball = Vec2::new(target_pos.x + 30.0, target_pos.y);
    s.players[(target - 1) as usize].run_vel = Vec2::new(0.0, 0.0);
    step(
        &mut s,
        0.016,
        &input(InputOpts {
            switch: true,
            r#move: Vec2::new(1.0, 0.0),
            ..InputOpts::default()
        }),
        &tune,
    );
    assert_eq!(
        s.controlled, target,
        "switch picks the player closest to the ball"
    );
    assert!(s.players[(s.controlled - 1) as usize].team == Team::Home);
    assert!(!s.players[(s.controlled - 1) as usize].is_keeper);
    assert!(
        s.players[(target - 1) as usize].run_vel.x > 0.0,
        "the newly selected player receives this tick's movement"
    );
}

// ---------------------------------------------------------------------
// match tactics
// ---------------------------------------------------------------------

fn mean_home_outfield_x(s: &MatchState) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    for p in &s.players {
        if p.team == Team::Home && !p.is_keeper {
            sum += p.anchor.x;
            n += 1.0;
        }
    }
    sum / n
}

#[test]
fn press_high_pushes_the_home_shape_higher_up_the_pitch() {
    let balanced = new_match();
    let press_high = gc_data::tactics::get("press_high").expect("press_high tactic is authored");
    let high = new_match_opts(Some(press_high), false);
    assert!(mean_home_outfield_x(&high) > mean_home_outfield_x(&balanced));
}

#[test]
fn counter_attack_drops_the_home_shape_deeper() {
    let balanced = new_match();
    let counter = gc_data::tactics::get("counter").expect("counter tactic is authored");
    let counter_match = new_match_opts(Some(counter), false);
    assert!(mean_home_outfield_x(&counter_match) < mean_home_outfield_x(&balanced));
}

#[test]
fn press_high_assigns_two_home_pressers() {
    let press_high = gc_data::tactics::get("press_high").expect("press_high tactic is authored");
    let high = new_match_opts(Some(press_high), false);
    assert_eq!(high.press.home, 2);
}

// ---------------------------------------------------------------------
// match.step scoring
// ---------------------------------------------------------------------

#[test]
fn a_ball_wholly_crossing_the_right_goal_line_scores_for_home() {
    let tune = Tuning::new();
    let mut s = new_match();
    s.owner = None;
    s.pickup_cd = 1.0; // keep anyone from collecting it
    s.ball = Vec2::new(s.field.w - 5.0, s.field.h / 2.0);
    s.ball_vel = Vec2::new(400.0, 0.0);
    for _ in 0..10 {
        step(&mut s, 0.016, &no_input(), &tune);
        if s.score.home > 0 {
            break;
        }
    }
    assert_eq!(s.score.home, 1);
    assert_eq!(s.score.away, 0);
}

#[test]
fn a_ball_on_the_line_is_not_yet_a_goal_must_wholly_cross() {
    let tune = Tuning::new();
    let mut s = new_match();
    s.owner = None;
    s.pickup_cd = 1.0;
    // Ball centre right on the goal line, barely moving: still in play.
    s.ball = Vec2::new(s.field.w, s.field.h / 2.0);
    s.ball_vel = Vec2::new(1.0, 0.0);
    step(&mut s, 0.016, &no_input(), &tune);
    assert_eq!(s.score.home, 0, "on the line is not across the line");
}
