//! Port of `spec/sim/aerial_spec.lua`.
//!
//! Every case in this Lua spec builds a real fixture via `match.new` and
//! drives it with `match.step` — it is testing the integration between
//! `sim/match.lua`'s tick (ball/player physics, possession) and
//! `sim/aerial.lua`'s contact resolution, not `aerial` in isolation.
//! `aerial`'s own resolver logic is fully ported and tested separately in
//! `tests/aerial_resolver.rs` (from `aerial_resolver_spec.lua`).

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const FIELD: PitchSize = PitchSize { w: 960.0, h: 540.0 };

fn new_match() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: FIELD,
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

fn input(o: MatchInput) -> MatchInput {
    o
}

fn has_event(s: &MatchState, kind: MatchEventKind) -> bool {
    s.events.iter().any(|e| e.kind == kind)
}

fn first_home_outfield(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no home outfielder");
}

#[test]
fn lets_a_human_meet_a_dropping_cross_for_a_volley_toward_goal() {
    let tune = Tuning::new();
    let mut s = new_match();
    let hp = first_home_outfield(&s);
    s.controlled = hp;
    s.owner = None;
    s.pickup_cd = 0.0;
    {
        let p = &mut s.players[(hp - 1) as usize];
        p.pos = Vec2::new(700.0, 270.0);
        p.header_cd = 0.0;
    }
    s.ball = Vec2::new(710.0, 270.0); // within reach
    s.ball_z = 30.0; // volley band
    s.ball_vz = -60.0; // descending
    s.ball_vel = Vec2::new(0.0, 0.0);
    sim_match::step(
        &mut s,
        1.0 / 60.0,
        StepInput::Legacy(input(MatchInput {
            jockey: true,
            ..MatchInput::default()
        })),
        None,
        &tune,
    ); // "go up for it"
    assert!(
        has_event(&s, MatchEventKind::Volley) || has_event(&s, MatchEventKind::Header),
        "the striker connects"
    );
    assert!(s.ball_vel.x > 0.0, "and drives it toward the opponent goal");
}

#[test]
fn connects_with_the_generous_assist_reach_not_just_point_blank() {
    let tune = Tuning::new();
    let mut s = new_match();
    let hp = first_home_outfield(&s);
    s.controlled = hp;
    s.owner = None;
    s.pickup_cd = 0.0;
    {
        let p = &mut s.players[(hp - 1) as usize];
        p.pos = Vec2::new(700.0, 270.0);
        p.header_cd = 0.0;
    }
    // 40px away: outside the 24px AI reach, inside the human assist reach.
    s.ball = Vec2::new(740.0, 270.0);
    s.ball_z = 30.0;
    s.ball_vz = -60.0;
    s.ball_vel = Vec2::new(0.0, 0.0);
    sim_match::step(
        &mut s,
        1.0 / 60.0,
        StepInput::Legacy(input(MatchInput {
            jockey: true,
            ..MatchInput::default()
        })),
        None,
        &tune,
    );
    assert!(
        has_event(&s, MatchEventKind::Volley) || has_event(&s, MatchEventKind::Header),
        "the assist reach connects"
    );
}

#[test]
fn hands_control_to_the_best_placed_attacker_as_a_cross_flies_in() {
    let tune = Tuning::new();
    let mut s = new_match();
    let att = first_home_outfield(&s);
    // Control someone else; the aid should switch to the attacker on the ball.
    let mut other: Option<i64> = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != att {
            other = Some(idx);
            break;
        }
    }
    s.controlled = other.expect("fixture has a second home outfielder");
    s.owner = None;
    s.players[(att - 1) as usize].pos = Vec2::new(760.0, 260.0);
    s.ball = Vec2::new(770.0, 260.0);
    s.ball_z = 40.0; // lofted cross into the attacking third
    s.ball_vz = -20.0;
    s.ball_vel = Vec2::new(20.0, 0.0);
    sim_match::step(
        &mut s,
        1.0 / 60.0,
        StepInput::Legacy(input(MatchInput::default())),
        None,
        &tune,
    );
    assert_eq!(s.controlled, att);
}
