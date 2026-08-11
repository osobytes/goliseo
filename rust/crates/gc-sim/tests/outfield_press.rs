//! Port of `spec/sim/outfield_press_spec.lua`.

use gc_core::vec2::Vec2;
use gc_sim::brain;
use gc_sim::outfield_press as press;

fn press_context() -> press::OutfieldPressContext {
    press::OutfieldPressContext {
        heavy_touch: false,
        exposed_ball: false,
        cover_available: false,
        box_desperation: false,
        press_discipline: 0.8,
    }
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6,
        "expected ~{expected}, got {actual}"
    );
}

#[test]
fn team_owned_outfield_press_state_keeps_an_eligible_presser_at_86_percent_cost_and_switches_at_the_exact_85_percent_boundary()
 {
    let mut candidates = [
        press::OutfieldPressCandidate {
            player_index: 2,
            distance_cost: 100.0,
            eligible: None,
        },
        press::OutfieldPressCandidate {
            player_index: 3,
            distance_cost: 86.0,
            eligible: None,
        },
    ];
    assert_eq!(press::assign_presser(&candidates, Some(2)), Some(2));
    candidates[1].distance_cost = 85.0;
    assert_eq!(press::assign_presser(&candidates, Some(2)), Some(3));
}

#[test]
fn team_owned_outfield_press_state_uses_a_conservative_0_35_low_discipline_boundary() {
    let boundary = press::resolve(
        2,
        &press::OutfieldPressContext {
            press_discipline: 0.35,
            ..press_context()
        },
    );
    assert_eq!(boundary.mode, press::StablePressMode::Contain);
    assert_eq!(boundary.reason, brain::PressReason::NoTrigger);

    let fallback = press::resolve(
        2,
        &press::OutfieldPressContext {
            press_discipline: 0.349,
            ..press_context()
        },
    );
    assert_eq!(fallback.mode, press::StablePressMode::Commit);
    assert_eq!(fallback.reason, brain::PressReason::LowDiscipline);
}

#[test]
fn team_owned_outfield_press_state_keeps_heavy_touch_first_in_the_stable_multi_trigger_precedence()
{
    let state = press::resolve(
        2,
        &press::OutfieldPressContext {
            heavy_touch: true,
            exposed_ball: true,
            cover_available: true,
            box_desperation: true,
            press_discipline: 0.0,
        },
    );
    assert_eq!(state.mode, press::StablePressMode::Commit);
    assert_eq!(state.reason, brain::PressReason::HeavyTouch);
}

#[test]
fn team_owned_outfield_press_state_validates_inactive_contain_and_attributed_commit_relations() {
    assert_eq!(press::new_state().mode, press::StablePressMode::Inactive);
    assert_eq!(press::contain(2).reason, brain::PressReason::NoTrigger);
    let mut malformed = press::contain(2);
    malformed.reason = brain::PressReason::Cover;
    let result = std::panic::catch_unwind(move || press::copy_state(&malformed));
    assert!(result.is_err());
}

#[test]
fn outfield_press_geometry_and_tuning_holds_goal_side_while_conceding_eight_pixels_toward_pitch_centre()
 {
    let carrier = Vec2::new(480.0, 100.0);
    let target = press::contain_target(carrier, Vec2::new(0.0, 270.0), 540.0, 32.0);
    assert!(target.x < carrier.x, "contain target remains goal-side");
    let unbiassed = carrier.add(Vec2::new(0.0, 270.0).sub(carrier).normalized().scale(32.0));
    near(target.y, unbiassed.y + 8.0);
}

#[test]
fn outfield_press_geometry_and_tuning_slows_only_inside_the_named_60_pixel_contain_radius() {
    near(press::contain_speed(200.0, 61.0, 0.75), 200.0);
    near(press::contain_speed(200.0, 60.0, 0.75), 150.0);
}

#[test]
fn outfield_press_geometry_and_tuning_accepts_goal_side_cover_through_the_named_140_pixel_boundary()
{
    assert!(press::cover_available(140.0, true));
    assert!(!press::cover_available(140.001, true));
    assert!(!press::cover_available(100.0, false));
}

#[test]
fn outfield_press_geometry_and_tuning_shadows_the_highest_scored_eligible_lane_with_stable_index_ties()
 {
    let carrier = Vec2::new(480.0, 270.0);
    let base = Vec2::new(336.0, 270.0);
    let candidates = [
        press::OutfieldLaneCandidate {
            player_index: 9,
            score: 10.0,
            pos: Vec2::new(360.0, 390.0),
            eligible: None,
        },
        press::OutfieldLaneCandidate {
            player_index: 8,
            score: 10.0,
            pos: Vec2::new(360.0, 150.0),
            eligible: None,
        },
        press::OutfieldLaneCandidate {
            player_index: 7,
            score: 99.0,
            pos: Vec2::new(300.0, 270.0),
            eligible: Some(false),
        },
    ];
    let best = press::highest_scored_lane(&candidates).expect("a lane candidate must be selected");
    assert_eq!(best.player_index, 8);
    let target = press::lane_shadow_target(base, carrier, &candidates);
    assert!(
        target.y < base.y,
        "cover biases toward the selected upper lane"
    );
    assert!(
        target.x > base.x,
        "lane bias preserves the interpose foundation"
    );
}
