//! Tests for `gc_sim::keeper`.

use gc_core::vec2::Vec2;
use gc_sim::keeper;

const HOME_GOAL: keeper::Rect = keeper::Rect {
    x: -30.0,
    y: 215.0,
    w: 30.0,
    h: 110.0,
};
const AWAY_GOAL: keeper::Rect = keeper::Rect {
    x: 960.0,
    y: 215.0,
    w: 30.0,
    h: 110.0,
};

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6,
        "expected ~{expected}, got {actual}"
    );
}

fn home_context(ball_pos: Vec2, aggression: f64, in_1v1: bool) -> keeper::KeeperPositionContext {
    keeper::KeeperPositionContext {
        keeper_pos: Vec2::new(12.0, 270.0),
        ball_pos,
        goal: HOME_GOAL,
        team: keeper::Team::Home,
        aggression,
        in_1v1,
    }
}

fn away_context(ball_pos: Vec2, aggression: f64, in_1v1: bool) -> keeper::KeeperPositionContext {
    keeper::KeeperPositionContext {
        keeper_pos: Vec2::new(948.0, 270.0),
        ball_pos,
        goal: AWAY_GOAL,
        team: keeper::Team::Away,
        aggression,
        in_1v1,
    }
}

#[test]
fn keeper_arc_target_moves_monotonically_off_the_home_goal_line_as_the_ball_approaches() {
    let midfield = keeper::arc_target(&home_context(Vec2::new(480.0, 270.0), 80.0, false));
    let approaching = keeper::arc_target(&home_context(Vec2::new(320.0, 270.0), 80.0, false));
    let claim_edge = keeper::arc_target(&home_context(Vec2::new(160.0, 270.0), 80.0, false));

    near(midfield.x, 0.0);
    assert!(approaching.x > midfield.x);
    assert!(claim_edge.x > approaching.x);
    near(claim_edge.x, 80.0);
}

#[test]
fn keeper_arc_target_mirrors_the_approaching_arc_for_the_away_goal() {
    let midfield = keeper::arc_target(&away_context(Vec2::new(480.0, 270.0), 80.0, false));
    let approaching = keeper::arc_target(&away_context(Vec2::new(640.0, 270.0), 80.0, false));
    let claim_edge = keeper::arc_target(&away_context(Vec2::new(800.0, 270.0), 80.0, false));

    near(midfield.x, 960.0);
    assert!(approaching.x < midfield.x);
    assert!(claim_edge.x < approaching.x);
    near(claim_edge.x, 880.0);
}

#[test]
fn keeper_arc_target_holds_deep_and_centred_while_the_ball_is_beyond_midfield() {
    let home = keeper::arc_target(&home_context(Vec2::new(700.0, 400.0), 80.0, false));
    let away = keeper::arc_target(&away_context(Vec2::new(260.0, 140.0), 80.0, false));

    near(home.x, 0.0);
    near(home.y, 270.0);
    near(away.x, 960.0);
    near(away.y, 270.0);
}

#[test]
fn keeper_arc_target_keeps_one_on_one_targets_deep_at_and_beyond_midfield_in_both_directions() {
    let home_boundary = keeper::arc_target(&home_context(Vec2::new(480.0, 400.0), 80.0, true));
    let home_beyond = keeper::arc_target(&home_context(Vec2::new(700.0, 400.0), 80.0, true));
    let away_boundary = keeper::arc_target(&away_context(Vec2::new(480.0, 140.0), 80.0, true));
    let away_beyond = keeper::arc_target(&away_context(Vec2::new(260.0, 140.0), 80.0, true));

    near(home_boundary.x, 0.0);
    near(home_boundary.y, 270.0);
    near(home_beyond.x, 0.0);
    near(home_beyond.y, 270.0);
    near(away_boundary.x, 960.0);
    near(away_boundary.y, 270.0);
    near(away_beyond.x, 960.0);
    near(away_beyond.y, 270.0);
}

#[test]
fn keeper_arc_target_clamps_lateral_targets_to_the_28_pixel_guard_on_both_sides() {
    let low = keeper::arc_target(&home_context(Vec2::new(160.0, 540.0), 120.0, false));
    let high = keeper::arc_target(&away_context(Vec2::new(800.0, 0.0), 120.0, false));

    near(low.y, 298.0);
    near(high.y, 242.0);
}

#[test]
fn keeper_arc_target_uses_maximum_aggression_depth_for_a_one_on_one() {
    let normal_home = keeper::arc_target(&home_context(Vec2::new(320.0, 270.0), 80.0, false));
    let one_on_one_home = keeper::arc_target(&home_context(Vec2::new(320.0, 270.0), 80.0, true));
    let normal_away = keeper::arc_target(&away_context(Vec2::new(640.0, 270.0), 80.0, false));
    let one_on_one_away = keeper::arc_target(&away_context(Vec2::new(640.0, 270.0), 80.0, true));

    near(normal_home.x, 40.0);
    near(one_on_one_home.x, 80.0);
    near(normal_away.x, 920.0);
    near(one_on_one_away.x, 880.0);
}

#[test]
fn keeper_arc_target_never_exceeds_aggression_along_the_goal_to_ball_ray() {
    let home = home_context(Vec2::new(160.0, 500.0), 65.0, true);
    let away = away_context(Vec2::new(800.0, 40.0), 65.0, true);
    let home_target = keeper::arc_target(&home);
    let away_target = keeper::arc_target(&away);
    let home_center = Vec2::new(0.0, 270.0);
    let away_center = Vec2::new(960.0, 270.0);

    assert!(home_target.dist(home_center) <= 65.0);
    assert!(away_target.dist(away_center) <= 65.0);
}

#[test]
fn keeper_arc_target_uses_keeper_position_as_a_deterministic_fallback_for_a_degenerate_ray() {
    let mut context = home_context(Vec2::new(0.0, 270.0), 60.0, true);
    context.keeper_pos = Vec2::new(20.0, 270.0);
    let target = keeper::arc_target(&context);

    near(target.x, 60.0);
    near(target.y, 270.0);
}

#[test]
fn keeper_base_target_uses_shallow_mirrored_dynamic_depth_as_the_ball_approaches() {
    let home_midfield = keeper::base_target(&home_context(Vec2::new(480.0, 270.0), 80.0, false));
    let home_approaching = keeper::base_target(&home_context(Vec2::new(320.0, 270.0), 80.0, false));
    let home_claim_edge = keeper::base_target(&home_context(Vec2::new(160.0, 270.0), 80.0, false));
    let away_midfield = keeper::base_target(&away_context(Vec2::new(480.0, 270.0), 80.0, false));
    let away_approaching = keeper::base_target(&away_context(Vec2::new(640.0, 270.0), 80.0, false));
    let away_claim_edge = keeper::base_target(&away_context(Vec2::new(800.0, 270.0), 80.0, false));

    near(home_midfield.x, 12.0);
    near(home_approaching.x, 15.0);
    near(home_claim_edge.x, 18.0);
    near(away_midfield.x, 948.0);
    near(away_approaching.x, 945.0);
    near(away_claim_edge.x, 942.0);
}

#[test]
fn keeper_base_target_stays_deep_and_central_beyond_midfield() {
    let home = keeper::base_target(&home_context(Vec2::new(700.0, 500.0), 120.0, false));
    let away = keeper::base_target(&away_context(Vec2::new(260.0, 40.0), 120.0, false));

    near(home.x, 12.0);
    near(home.y, 270.0);
    near(away.x, 948.0);
    near(away.y, 270.0);
}

#[test]
fn keeper_base_target_retains_the_deliberate_lateral_corner_concession() {
    let home = keeper::base_target(&home_context(Vec2::new(160.0, 540.0), 120.0, false));
    let away = keeper::base_target(&away_context(Vec2::new(800.0, 0.0), 120.0, false));

    near(home.y, 310.0);
    near(away.y, 230.0);
    assert!(home.x > 12.0 && home.x <= 18.0);
    assert!(away.x < 948.0 && away.x >= 942.0);
}

#[test]
fn keeper_save_style_leaves_the_26_pixel_smother_boundary_to_the_claim_branch() {
    assert!(keeper::in_smother_range(26.0));
    assert!(!keeper::in_smother_range(26.000001));
    assert_eq!(
        keeper::save_style(26.000001, 100.0, 100.0),
        keeper::SaveStyle::Spread
    );
}

#[test]
#[should_panic]
fn keeper_save_style_panics_within_the_smother_boundary() {
    let _ = keeper::save_style(26.0, 0.0, 100.0);
}

#[test]
fn keeper_save_style_keeps_the_78_pixel_boundary_in_the_spread_style() {
    assert_eq!(
        keeper::save_style(78.0, 100.0, 100.0),
        keeper::SaveStyle::Spread
    );
    assert_eq!(
        keeper::save_style(78.000001, 40.0, 100.0),
        keeper::SaveStyle::Central
    );
}

#[test]
fn keeper_save_style_includes_exactly_40_percent_of_reach_in_the_central_style() {
    assert_eq!(
        keeper::save_style(100.0, 40.0, 100.0),
        keeper::SaveStyle::Central
    );
    assert_eq!(
        keeper::save_style(100.0, 40.000001, 100.0),
        keeper::SaveStyle::Stretch
    );
}

#[test]
fn keeper_commit_lead_clamps_anticipation_and_negative_windups_to_safe_bounds() {
    near(keeper::commit_lead(-1.0, 2.0), 0.0);
    near(keeper::commit_lead(0.5, 2.0), 1.0);
    near(keeper::commit_lead(2.0, 2.0), 2.0);
    near(keeper::commit_lead(0.5, -2.0), 0.0);
}

#[test]
fn keeper_commit_lead_is_monotonic_in_anticipation_across_the_supported_range() {
    let mut previous = keeper::commit_lead(0.0, 0.3);
    let mut anticipation = 0.1;
    while anticipation <= 1.0 + 1e-9 {
        let current = keeper::commit_lead(anticipation, 0.3);
        assert!(current >= previous);
        assert!((0.0..=0.3).contains(&current));
        previous = current;
        anticipation += 0.1;
    }
}

#[test]
fn keeper_early_set_eligibility_projects_captured_directions_into_either_defending_goal_mouth() {
    assert!(keeper::shot_targets_goal(&keeper::KeeperShotContext {
        defending_team: keeper::Team::Away,
        shooter_team: keeper::Team::Home,
        origin: Vec2::new(700.0, 250.0),
        direction: Vec2::new(260.0, 20.0),
        goal: AWAY_GOAL,
    }));
    assert!(keeper::shot_targets_goal(&keeper::KeeperShotContext {
        defending_team: keeper::Team::Home,
        shooter_team: keeper::Team::Away,
        origin: Vec2::new(260.0, 290.0),
        direction: Vec2::new(-260.0, -20.0),
        goal: HOME_GOAL,
    }));
}

#[test]
fn keeper_early_set_eligibility_rejects_teammates_backwards_shots_and_projections_outside_the_mouth()
 {
    assert!(!keeper::shot_targets_goal(&keeper::KeeperShotContext {
        defending_team: keeper::Team::Away,
        shooter_team: keeper::Team::Away,
        origin: Vec2::new(700.0, 270.0),
        direction: Vec2::new(260.0, 0.0),
        goal: AWAY_GOAL,
    }));
    assert!(!keeper::shot_targets_goal(&keeper::KeeperShotContext {
        defending_team: keeper::Team::Away,
        shooter_team: keeper::Team::Home,
        origin: Vec2::new(700.0, 270.0),
        direction: Vec2::new(-260.0, 0.0),
        goal: AWAY_GOAL,
    }));
    assert!(!keeper::shot_targets_goal(&keeper::KeeperShotContext {
        defending_team: keeper::Team::Away,
        shooter_team: keeper::Team::Home,
        origin: Vec2::new(700.0, 270.0),
        direction: Vec2::new(260.0, 80.0),
        goal: AWAY_GOAL,
    }));
}

#[test]
fn keeper_early_set_eligibility_bounds_set_timing_by_the_captured_wind_up_without_making_zero_reactive_early()
 {
    let mut context = keeper::KeeperSetContext {
        defending_team: keeper::Team::Away,
        shooter_team: keeper::Team::Home,
        origin: Vec2::new(700.0, 270.0),
        direction: Vec2::new(260.0, 0.0),
        goal: AWAY_GOAL,
        anticipation: 0.0,
        windup_duration: 0.15,
        windup_remaining: 0.000001,
    };
    assert!(!keeper::should_set(&context));

    context.anticipation = 0.5;
    context.windup_remaining = 0.075001;
    assert!(!keeper::should_set(&context));
    context.windup_remaining = 0.075;
    assert!(keeper::should_set(&context));

    context.anticipation = 1.0;
    context.windup_remaining = 0.15;
    assert!(keeper::should_set(&context));
    context.windup_remaining = 0.0;
    assert!(!keeper::should_set(&context));
}

fn advance_eligible() -> keeper::KeeperAdvanceContext {
    keeper::KeeperAdvanceContext {
        in_claim_zone: true,
        attacker_controlled: true,
        loose_touch: false,
        support_near: false,
        defender_engaged: false,
        threat_distance: 150.0,
    }
}

#[test]
fn keeper_advance_eligibility_uses_control_and_visible_support_context_instead_of_ball_depth_alone()
{
    let eligible = advance_eligible();
    assert!(keeper::should_advance(&eligible));

    let supported = keeper::KeeperAdvanceContext {
        support_near: true,
        ..eligible
    };
    assert!(!keeper::should_advance(&supported));
    assert!(keeper::should_contain(&supported));

    let uncontrolled = keeper::KeeperAdvanceContext {
        attacker_controlled: false,
        ..eligible
    };
    assert!(!keeper::should_advance(&uncontrolled));
}

#[test]
fn keeper_advance_eligibility_lets_a_loose_touch_create_a_smother_chance_despite_an_engaged_defender()
 {
    let mut context = keeper::KeeperAdvanceContext {
        attacker_controlled: false,
        loose_touch: true,
        defender_engaged: true,
        ..advance_eligible()
    };
    assert!(keeper::should_advance(&context));

    context.loose_touch = false;
    context.attacker_controlled = true;
    assert!(!keeper::should_advance(&context));
}

fn behavior_context(state: keeper::KeeperBehaviorState) -> keeper::KeeperBehaviorContext {
    keeper::KeeperBehaviorContext {
        current_state: state,
        state_timer: 0.0,
        keeper_pos: Vec2::new(12.0, 270.0),
        ball_pos: Vec2::new(150.0, 220.0),
        goal: HOME_GOAL,
        team: keeper::Team::Home,
        aggression: 42.0,
        advance_eligible: false,
        contain_eligible: false,
        ground_cue: false,
        lob_cue: false,
        through_ball_cue: false,
        dt: 1.0 / 60.0,
    }
}

#[test]
fn keeper_behavior_states_advances_and_contains_on_a_bounded_centre_ray_target() {
    let advancing = keeper::behavior(&keeper::KeeperBehaviorContext {
        advance_eligible: true,
        ..behavior_context(keeper::KeeperBehaviorState::Base)
    });
    assert_eq!(advancing.state, keeper::KeeperBehaviorState::Advance);
    assert!(advancing.target.dist(Vec2::new(0.0, 270.0)) <= 42.0);

    let containing = keeper::behavior(&keeper::KeeperBehaviorContext {
        keeper_pos: advancing.target,
        advance_eligible: true,
        ..behavior_context(keeper::KeeperBehaviorState::Advance)
    });
    assert_eq!(containing.state, keeper::KeeperBehaviorState::Contain);
    assert_eq!(containing.movement_scale, 0.45);
}

#[test]
fn keeper_behavior_states_sets_for_a_ground_cue_and_retreats_for_lob_or_through_ball_preparation() {
    let set_context = keeper::KeeperBehaviorContext {
        ground_cue: true,
        ..behavior_context(keeper::KeeperBehaviorState::Advance)
    };
    let set = keeper::behavior(&set_context);
    assert_eq!(set.state, keeper::KeeperBehaviorState::Set);
    assert_eq!(set.movement_scale, 0.0);
    assert_eq!(set.target, set_context.keeper_pos);

    let lob = keeper::behavior(&keeper::KeeperBehaviorContext {
        lob_cue: true,
        ..behavior_context(keeper::KeeperBehaviorState::Advance)
    });
    assert_eq!(lob.state, keeper::KeeperBehaviorState::Retreat);
    near(lob.target.x, 0.0);
    near(lob.target.y, 270.0);

    assert_eq!(
        keeper::behavior(&keeper::KeeperBehaviorContext {
            through_ball_cue: true,
            ..behavior_context(keeper::KeeperBehaviorState::Contain)
        })
        .state,
        keeper::KeeperBehaviorState::Retreat
    );
    assert_eq!(
        keeper::behavior(&keeper::KeeperBehaviorContext {
            through_ball_cue: true,
            ..behavior_context(keeper::KeeperBehaviorState::Base)
        })
        .state,
        keeper::KeeperBehaviorState::Base
    );
}

#[test]
fn keeper_behavior_states_holds_recover_before_retreating_instead_of_snapping_to_base() {
    let recover = keeper::behavior(&behavior_context(keeper::KeeperBehaviorState::Advance));
    assert_eq!(recover.state, keeper::KeeperBehaviorState::Recover);
    assert_eq!(recover.movement_scale, 0.0);

    let holding = keeper::behavior(&keeper::KeeperBehaviorContext {
        state_timer: recover.state_timer,
        ..behavior_context(keeper::KeeperBehaviorState::Recover)
    });
    assert_eq!(holding.state, keeper::KeeperBehaviorState::Recover);
    assert_eq!(holding.movement_scale, 0.0);

    let retreat = keeper::behavior(&keeper::KeeperBehaviorContext {
        state_timer: 0.0,
        keeper_pos: Vec2::new(40.0, 270.0),
        ..behavior_context(keeper::KeeperBehaviorState::Recover)
    });
    assert_eq!(retreat.state, keeper::KeeperBehaviorState::Retreat);
    assert!(retreat.movement_scale > 0.0);
}

fn chip_context(keeper_x: f64) -> keeper::KeeperChipContext {
    keeper::KeeperChipContext {
        origin: Vec2::new(700.0, 270.0),
        target: Vec2::new(960.0, 270.0),
        keeper_pos: Vec2::new(keeper_x, 270.0),
        defending_team: keeper::Team::Away,
        goal: AWAY_GOAL,
        horizontal_speed: 500.0,
        friction: 0.3,
        gravity: 900.0,
        keeper_clearance: 60.0,
        crossbar: 70.0,
        desired_goal_height: 65.0,
    }
}

#[test]
fn keeper_chip_counterplay_solves_against_the_actual_keeper_plane_and_under_the_crossbar() {
    let context = chip_context(900.0);
    let vz = keeper::chip_launch(&context).expect("chip must be feasible");
    let direction = context.target.sub(context.origin).normalized();
    let height = keeper::goal_line_height(&keeper::KeeperTrajectoryContext {
        origin: context.origin,
        direction,
        horizontal_speed: context.horizontal_speed,
        vertical_speed: vz,
        defending_team: context.defending_team,
        goal: context.goal,
        friction: context.friction,
        gravity: context.gravity,
    })
    .expect("trajectory must resolve");
    assert!(height >= 0.0 && height < context.crossbar);

    let keeper_distance = (context.keeper_pos.x - context.origin.x) / direction.x;
    let keeper_time =
        keeper::travel_time(keeper_distance, context.horizontal_speed, context.friction)
            .expect("keeper travel time must resolve");
    let keeper_height = vz * keeper_time - 0.5 * context.gravity * keeper_time * keeper_time;
    assert!(keeper_height > context.keeper_clearance);
}

#[test]
fn keeper_chip_counterplay_makes_a_committed_advance_no_harder_to_chip_and_rejects_an_empty_path() {
    let deep = keeper::chip_launch(&chip_context(948.0)).expect("deep chip must be feasible");
    let advanced =
        keeper::chip_launch(&chip_context(880.0)).expect("advanced chip must be feasible");
    assert!(advanced <= deep);

    let mut impossible = chip_context(900.0);
    impossible.crossbar = 50.0;
    assert_eq!(keeper::chip_launch(&impossible), None);
}

#[test]
fn keeper_chip_counterplay_keeps_an_infeasible_human_chip_as_an_under_bar_poor_chip() {
    let mut context = chip_context(900.0);
    context.keeper_clearance = 100.0;
    assert_eq!(keeper::chip_launch(&context), None);

    let vz = keeper::committed_chip_launch(&context);
    let direction = context.target.sub(context.origin).normalized();
    let keeper_time =
        keeper::travel_time(200.0, 500.0, 0.3).expect("keeper travel time must resolve");
    let keeper_height = vz * keeper_time - 450.0 * keeper_time * keeper_time;
    let goal_height = keeper::goal_line_height(&keeper::KeeperTrajectoryContext {
        origin: context.origin,
        direction,
        horizontal_speed: context.horizontal_speed,
        vertical_speed: vz,
        defending_team: context.defending_team,
        goal: context.goal,
        friction: context.friction,
        gravity: context.gravity,
    })
    .expect("trajectory must resolve");

    assert!(vz > 0.0);
    assert!(keeper_height < context.keeper_clearance);
    near(goal_height, context.desired_goal_height);
    assert!(goal_height < context.crossbar);
}

#[test]
fn keeper_chip_counterplay_uses_a_deterministic_low_lob_when_friction_makes_the_goal_unreachable() {
    let mut context = chip_context(900.0);
    context.horizontal_speed = 50.0;
    assert_eq!(keeper::chip_launch(&context), None);
    assert_eq!(
        keeper::travel_time(260.0, context.horizontal_speed, context.friction),
        None
    );

    let first = keeper::committed_chip_launch(&context);
    let second = keeper::committed_chip_launch(&context);
    let apex = first * first / (2.0 * context.gravity);
    assert_eq!(first, second);
    assert!(first > 0.0);
    assert!(apex <= context.keeper_clearance * 0.5);
}

#[test]
fn keeper_chip_counterplay_exposes_a_visible_high_line_and_lets_a_deep_keeper_meet_a_poor_chip() {
    assert!(!keeper::chip_is_visible(
        Vec2::new(19.999999, 270.0),
        keeper::Team::Home,
        HOME_GOAL
    ));
    assert!(keeper::chip_is_visible(
        Vec2::new(20.0, 270.0),
        keeper::Team::Home,
        HOME_GOAL
    ));
    assert!(keeper::chip_is_visible(
        Vec2::new(80.0, 270.0),
        keeper::Team::Home,
        HOME_GOAL
    ));
    assert!(!keeper::chip_is_visible(
        Vec2::new(940.000001, 270.0),
        keeper::Team::Away,
        AWAY_GOAL
    ));
    assert!(keeper::chip_is_visible(
        Vec2::new(940.0, 270.0),
        keeper::Team::Away,
        AWAY_GOAL
    ));
    assert!(keeper::chip_is_visible(
        Vec2::new(880.0, 270.0),
        keeper::Team::Away,
        AWAY_GOAL
    ));

    let speed = 500.0;
    let vz = 350.0;
    let advanced_time =
        keeper::travel_time(180.0, speed, 0.3).expect("advanced travel time must resolve");
    let deep_time = keeper::travel_time(248.0, speed, 0.3).expect("deep travel time must resolve");
    let goal_time = keeper::travel_time(260.0, speed, 0.3).expect("goal travel time must resolve");
    let advanced_height = vz * advanced_time - 450.0 * advanced_time * advanced_time;
    let deep_height = vz * deep_time - 450.0 * deep_time * deep_time;
    let goal_height = vz * goal_time - 450.0 * goal_time * goal_time;
    assert!(
        advanced_height > 60.0,
        "the poor chip clears the committed keeper"
    );
    assert!(
        deep_height <= 60.0,
        "the same chip reaches a deep keeper's hands"
    );
    assert!(
        (0.0..70.0).contains(&goal_height),
        "the chip remains on target"
    );
}

#[test]
fn keeper_chip_counterplay_never_gives_a_moving_keeper_more_reaction_reach_than_a_set_keeper() {
    let set = keeper::reaction_reach(100.0, 0.0, 0.32);
    let moving = keeper::reaction_reach(100.0, 1.0, 0.32);
    assert_eq!(set, 100.0);
    assert!(moving < set);
}
