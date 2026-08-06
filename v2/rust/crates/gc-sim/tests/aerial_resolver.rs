//! Port of `spec/sim/aerial_resolver_spec.lua`.
//!
//! Nothing here needs the unported `gc_sim::r#match`: every case builds an
//! `AerialContext` directly, exactly like the Lua spec's own `context()`
//! helper, and calls `aerial::best_contact`/`resolve`/`claim_score` in
//! isolation.

use gc_core::vec2::Vec2;
use gc_sim::aerial::{self, AerialContext, AerialIntent, AerialStyle};
use gc_sim::stats;

/// Mirrors the Lua spec's `context(overrides)` helper: every field has the
/// same default, callers override only what the case cares about.
struct ContextBuilder {
    ball_pos: Vec2,
    ball_vel: Vec2,
    ball_z: f64,
    ball_vz: f64,
    player_pos: Vec2,
    player_vel: Vec2,
    facing: Vec2,
    move_speed: f64,
    skill: f64,
    strength: f64,
    opponent_distance: f64,
    anticipated: bool,
    instability: f64,
    extra_reach: f64,
    extra_lift: f64,
}

impl Default for ContextBuilder {
    fn default() -> Self {
        ContextBuilder {
            ball_pos: Vec2::new(10.0, 0.0),
            ball_vel: Vec2::new(-120.0, 0.0),
            ball_z: 50.0,
            ball_vz: -120.0,
            player_pos: Vec2::new(0.0, 0.0),
            player_vel: Vec2::new(0.0, 0.0),
            facing: Vec2::new(1.0, 0.0),
            move_speed: 200.0,
            skill: 0.6,
            strength: 0.5,
            opponent_distance: 100.0,
            anticipated: true,
            instability: 0.0,
            extra_reach: 0.0,
            extra_lift: 0.0,
        }
    }
}

impl ContextBuilder {
    fn build(self) -> AerialContext {
        AerialContext {
            ball_pos: self.ball_pos,
            ball_vel: self.ball_vel,
            ball_z: self.ball_z,
            ball_vz: self.ball_vz,
            player_pos: self.player_pos,
            player_vel: self.player_vel,
            facing: self.facing,
            move_speed: self.move_speed,
            skill: self.skill,
            strength: self.strength,
            opponent_distance: self.opponent_distance,
            anticipated: self.anticipated,
            instability: self.instability,
            extra_reach: self.extra_reach,
            extra_lift: self.extra_lift,
        }
    }
}

#[test]
fn derives_reception_and_strike_skills_from_the_existing_stat_vocabulary() {
    use gc_data::players::StatBlock;
    let technical = StatBlock {
        pace: 5,
        strength: 4,
        technique: 9,
        stamina: 5,
        mental: 7,
    };
    let raw = StatBlock {
        pace: 5,
        strength: 8,
        technique: 2,
        stamina: 5,
        mental: 3,
    };
    assert!(stats::first_touch(technical) > stats::first_touch(raw));
    assert!(stats::volley(technical) > stats::volley(raw));
    assert!(stats::bicycle(technical) > stats::bicycle(raw));
    assert!(
        stats::header(raw) > stats::volley(raw),
        "strength and mental keep headers viable"
    );
}

#[test]
fn chooses_chest_control_above_the_standing_leg_band() {
    let ctx = ContextBuilder {
        ball_z: 55.0,
        ..Default::default()
    }
    .build();
    let contact = aerial::best_contact(&ctx, AerialIntent::Receive).expect("a contact exists");
    assert_eq!(contact.style, AerialStyle::ChestControl);
    assert!(!contact.jumping);
}

#[test]
fn marks_a_high_header_as_jumping() {
    let ctx = ContextBuilder {
        ball_z: 88.0,
        ..Default::default()
    }
    .build();
    let contact = aerial::best_contact(&ctx, AerialIntent::Strike).expect("a contact exists");
    assert_eq!(contact.style, AerialStyle::Header);
    assert!(contact.jumping);
    assert!(contact.jump_ratio > 0.0);
}

#[test]
fn rejects_a_ball_above_the_maximum_jumping_reach() {
    let ctx = ContextBuilder {
        ball_z: 110.0,
        ..Default::default()
    }
    .build();
    assert_eq!(aerial::best_contact(&ctx, AerialIntent::Strike), None);
}

#[test]
fn requires_overhead_or_behind_geometry_for_a_bicycle() {
    let in_front = ContextBuilder {
        ball_pos: Vec2::new(18.0, 0.0),
        ball_z: 60.0,
        ..Default::default()
    }
    .build();
    let front_contact = aerial::best_contact(&in_front, AerialIntent::Acrobatic)
        .expect("falls back to a conventional hit");
    assert_ne!(
        front_contact.style,
        AerialStyle::Bicycle,
        "front ball falls back to a conventional hit"
    );

    let behind = ContextBuilder {
        ball_pos: Vec2::new(-10.0, 0.0),
        ball_z: 60.0,
        ..Default::default()
    }
    .build();
    let behind_contact =
        aerial::best_contact(&behind, AerialIntent::Acrobatic).expect("a contact exists");
    assert_eq!(behind_contact.style, AerialStyle::Bicycle);
    assert!(behind_contact.jumping);
}

#[test]
fn makes_stretch_pace_jump_instability_and_pressure_increase_difficulty() {
    let easy = ContextBuilder {
        ball_z: 38.0,
        ..Default::default()
    }
    .build();
    let easy_contact =
        aerial::best_contact(&easy, AerialIntent::Receive).expect("a contact exists");

    let hard = ContextBuilder {
        ball_pos: Vec2::new(25.0, 0.0),
        ball_vel: Vec2::new(-560.0, 0.0),
        ball_z: 60.0,
        ball_vz: -480.0,
        opponent_distance: 8.0,
        anticipated: false,
        instability: 1.0,
        ..Default::default()
    }
    .build();
    let hard_contact =
        aerial::best_contact(&hard, AerialIntent::Receive).expect("a contact exists");

    assert!(hard_contact.difficulty > easy_contact.difficulty);
}

#[test]
fn is_deterministic_for_the_same_seed_and_context() {
    let ctx = ContextBuilder {
        ball_z: 60.0,
        ball_pos: Vec2::new(-10.0, 0.0),
        ..Default::default()
    }
    .build();
    let contact = aerial::best_contact(&ctx, AerialIntent::Acrobatic).expect("a contact exists");
    let a = aerial::resolve(&ctx, &contact, 4471);
    let b = aerial::resolve(&ctx, &contact, 4471);
    assert_eq!(a.outcome, b.outcome);
    assert_eq!(a.rng, b.rng);
    assert!((a.angle_error - b.angle_error).abs() <= 1e-6);
    assert!((a.weight_error - b.weight_error).abs() <= 1e-6);
}

#[test]
fn raises_both_outcome_probabilities_when_skill_increases() {
    let low_ctx = ContextBuilder {
        skill: 0.2,
        ..Default::default()
    }
    .build();
    let high_ctx = ContextBuilder {
        skill: 0.9,
        ..Default::default()
    }
    .build();
    let low_contact =
        aerial::best_contact(&low_ctx, AerialIntent::Receive).expect("a contact exists");
    let high_contact =
        aerial::best_contact(&high_ctx, AerialIntent::Receive).expect("a contact exists");
    let low = aerial::resolve(&low_ctx, &low_contact, 91);
    let high = aerial::resolve(&high_ctx, &high_contact, 91);
    assert!(high.contact_probability > low.contact_probability);
    assert!(high.clean_probability > low.clean_probability);
}

#[test]
fn rewards_position_and_skill_in_aerial_contests() {
    let good_ctx = ContextBuilder {
        skill: 0.9,
        ball_pos: Vec2::new(4.0, 0.0),
        ..Default::default()
    }
    .build();
    let bad_ctx = ContextBuilder {
        skill: 0.2,
        ball_pos: Vec2::new(25.0, 0.0),
        ..Default::default()
    }
    .build();
    let good_contact =
        aerial::best_contact(&good_ctx, AerialIntent::Receive).expect("a contact exists");
    let bad_contact =
        aerial::best_contact(&bad_ctx, AerialIntent::Receive).expect("a contact exists");
    let good = aerial::claim_score(&good_ctx, &good_contact, 0.0, 0.0, 0.0);
    let bad = aerial::claim_score(&bad_ctx, &bad_contact, 0.0, 0.0, 0.0);
    assert!(good > bad);
}
