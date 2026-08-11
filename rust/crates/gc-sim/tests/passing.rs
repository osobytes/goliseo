//! Tests for `gc_sim::passing`.

use gc_core::vec2::Vec2;
use gc_sim::passing;

const FROM: Vec2 = Vec2 { x: 0.0, y: 0.0 };

#[test]
fn passing_target_picks_the_nearer_of_two_equally_aligned_teammates() {
    let mates = [Vec2::new(100.0, 0.0), Vec2::new(30.0, 0.0)];
    assert_eq!(
        passing::target(FROM, Vec2::new(1.0, 0.0), &mates, None),
        Some(1)
    );
}

#[test]
fn passing_target_a_tap_goes_short_the_near_man_beats_a_far_one_dead_on_the_line() {
    let mates = [
        Vec2::new(60.0, 55.0), // near, ~42 degrees off the aim
        Vec2::new(350.0, 0.0), // far, dead on the aim line
    ];
    assert_eq!(
        passing::target(FROM, Vec2::new(1.0, 0.0), &mates, None),
        Some(0)
    );
}

#[test]
fn passing_target_a_charged_range_picks_out_the_far_man_on_the_line() {
    let mates = [Vec2::new(60.0, 55.0), Vec2::new(350.0, 0.0)];
    assert_eq!(
        passing::target(FROM, Vec2::new(1.0, 0.0), &mates, Some(350.0)),
        Some(1)
    );
}

#[test]
fn passing_target_ignores_teammates_behind_the_aim_direction() {
    let mates = [Vec2::new(-50.0, 0.0)];
    assert_eq!(
        passing::target(FROM, Vec2::new(1.0, 0.0), &mates, None),
        None
    );
}

#[test]
fn passing_target_ignores_teammates_outside_the_60_degree_cone() {
    let mates = [Vec2::new(10.0, 100.0)]; // ~84 degrees off the aim
    assert_eq!(
        passing::target(FROM, Vec2::new(1.0, 0.0), &mates, None),
        None
    );
}

#[test]
fn passing_target_returns_nil_for_a_zero_aim_direction() {
    let mates = [Vec2::new(10.0, 0.0)];
    assert_eq!(
        passing::target(FROM, Vec2::new(0.0, 0.0), &mates, None),
        None
    );
}
