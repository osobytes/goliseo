//! Tests for `gc_sim::outfield_decision`.

use gc_core::rng;
use gc_sim::brain;
use gc_sim::outfield_decision as od;

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6,
        "expected ~{expected}, got {actual}"
    );
}

fn carrier_context() -> od::OutfieldCarrierContext {
    od::OutfieldCarrierContext {
        goal_distance: 280.0,
        shoot_range: 260.0,
        angle_quality: 0.8,
        keeper_coverage: 0.5,
        space: 0.5,
        flank_depth: 0.8,
        cross_target: Some(4),
        box_targets: 2,
        cross_space: 0.7,
        goal_progress: 0.65,
        dribble_space: 0.55,
        passes: vec![
            od::OutfieldPassOption {
                player_index: 2,
                openness: 100.0,
                forward_progress: 80.0,
                distance: 150.0,
                lane_blocked: false,
                interception_risk: false,
                lane_fraction: None,
            },
            od::OutfieldPassOption {
                player_index: 3,
                openness: 75.0,
                forward_progress: 150.0,
                distance: 230.0,
                lane_blocked: true,
                interception_risk: false,
                lane_fraction: Some(0.45),
            },
        ],
    }
}

#[test]
fn outfield_decision_cadence_refreshes_high_scan_rate_no_slower_and_advances_only_stored_countdown_state()
 {
    let initial = od::new_state(Some(53.0));
    let slow = od::refresh(
        &initial,
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::Move,
        0.0,
        Some(100.0),
        Some(200.0),
        None,
        None,
    );
    let fast = od::refresh(
        &initial,
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::Move,
        1.0,
        Some(100.0),
        Some(200.0),
        None,
        None,
    );
    near(slow.remaining, 0.45);
    near(fast.remaining, 0.15);
    assert!(fast.remaining <= slow.remaining);
    assert_eq!(slow.generation, 1);
    assert_eq!(
        initial.generation, 0,
        "refresh does not mutate caller state"
    );
    assert_eq!(slow.rng_state, 53);

    let retained = od::advance(&slow, 0.1);
    near(retained.remaining, 0.35);
    assert_eq!(retained.target_x, Some(100.0));
    assert!(!od::should_refresh(
        &retained,
        od::OutfieldDecisionContext::Offball,
        None
    ));
    assert!(od::should_refresh(
        &retained,
        od::OutfieldDecisionContext::Offball,
        Some(true)
    ));
}

#[test]
fn outfield_decision_cadence_keeps_a_monotonic_refresh_generation_across_deliberate_resets() {
    let state = od::refresh(
        &od::new_state(None),
        od::OutfieldDecisionContext::Carrier,
        od::OutfieldIntent::Dribble,
        0.5,
        None,
        None,
        None,
        None,
    );
    let state = od::refresh(
        &state,
        od::OutfieldDecisionContext::Carrier,
        od::OutfieldIntent::Shoot,
        0.5,
        None,
        None,
        None,
        None,
    );
    assert_eq!(state.generation, 2);
    let reset = od::reset(&state);
    assert_eq!(reset.generation, 2);
    assert_eq!(reset.context, od::OutfieldDecisionContext::Ineligible);
    assert_eq!(reset.intent, od::OutfieldIntent::None);
    assert_eq!(reset.remaining, 0.0);
    assert_eq!(reset.rng_state, state.rng_state);
    assert_eq!(
        od::refresh(
            &reset,
            od::OutfieldDecisionContext::Offball,
            od::OutfieldIntent::Move,
            0.5,
            Some(10.0),
            Some(20.0),
            None,
            None
        )
        .generation,
        3
    );
}

#[test]
fn outfield_decision_cadence_advances_only_the_dedicated_decision_stream_when_selection_samples() {
    let state = od::new_state(Some(53.0));
    let options = vec![
        brain::BrainScoredOption {
            id: "best".to_string(),
            kind: "dribble".to_string(),
            score: 10.0,
            payload: None,
            reference: None,
        },
        brain::BrainScoredOption {
            id: "second".to_string(),
            kind: "pass".to_string(),
            score: 9.0,
            payload: None,
            reference: Some(brain::OptionReference::Index(2)),
        },
    ];
    let (_, next_rng) = od::decide_carrier(&options, 0.79, 1.0, state.rng_state);
    let advanced = od::with_rng_state(&state, next_rng);
    assert_ne!(advanced.rng_state, state.rng_state);
    assert_eq!(advanced.generation, state.generation);
    assert_eq!(state.rng_state, 53);
}

#[test]
fn outfield_decision_cadence_requires_a_refresh_when_eligibility_context_changes_or_cadence_expires()
 {
    let state = od::refresh(
        &od::new_state(None),
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::Move,
        0.0,
        Some(1.0),
        Some(2.0),
        None,
        None,
    );
    assert!(!od::should_refresh(
        &state,
        od::OutfieldDecisionContext::Offball,
        None
    ));
    assert!(od::should_refresh(
        &state,
        od::OutfieldDecisionContext::Carrier,
        None
    ));
    let state = od::advance(&state, state.remaining);
    assert!(od::should_refresh(
        &state,
        od::OutfieldDecisionContext::Offball,
        None
    ));
}

#[test]
fn outfield_decision_cadence_retains_one_fixed_run_expiry_across_personal_cadence_refreshes() {
    let expiry = -17.2;
    let state = od::refresh(
        &od::new_state(Some(53.0)),
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::InBehind,
        0.5,
        Some(700.0),
        Some(220.0),
        None,
        Some(expiry),
    );
    let generation = state.generation;
    let state = od::advance(&state, state.remaining);
    let state = od::refresh(
        &state,
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::InBehind,
        0.5,
        Some(700.0),
        Some(220.0),
        None,
        Some(expiry),
    );
    assert_eq!(state.generation, generation + 1);
    assert_eq!(state.run_expires_at, Some(expiry));
}

#[test]
fn outfield_decision_cadence_cancels_a_run_into_support_without_inventing_a_cadence_boundary() {
    let running = od::refresh(
        &od::new_state(Some(91.0)),
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::HoldWidth,
        0.4,
        Some(500.0),
        Some(80.0),
        None,
        Some(1.8),
    );
    let cancelled = od::cancel_run(&running, 420.0, 160.0);
    assert_eq!(cancelled.intent, od::OutfieldIntent::Move);
    assert_eq!(cancelled.target_x, Some(420.0));
    assert_eq!(cancelled.target_y, Some(160.0));
    assert_eq!(cancelled.run_expires_at, None);
    assert_eq!(cancelled.generation, running.generation);
    assert_eq!(cancelled.remaining, running.remaining);
    assert_eq!(cancelled.rng_state, running.rng_state);
}

#[test]
#[should_panic]
fn outfield_decision_cadence_rejects_expiry_on_an_ordinary_move() {
    let initial = od::new_state(None);
    let _ = od::refresh(
        &initial,
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::Move,
        0.5,
        Some(10.0),
        Some(20.0),
        None,
        Some(1.8),
    );
}

#[test]
#[should_panic]
fn outfield_decision_cadence_rejects_missing_expiry_on_a_run() {
    let initial = od::new_state(None);
    let _ = od::refresh(
        &initial,
        od::OutfieldDecisionContext::Offball,
        od::OutfieldIntent::ComeShort,
        0.5,
        Some(10.0),
        Some(20.0),
        None,
        None,
    );
}

#[test]
fn outfield_carrier_choices_constructs_the_complete_legitimate_option_set_without_consuming_rng() {
    let seed = rng::seed(91.0);
    let options = od::carrier_options(&carrier_context());
    assert_eq!(options.len(), 5);
    assert_eq!(options[0].kind, "shoot");
    assert_eq!(options[1].kind, "dribble");
    assert_eq!(options[2].kind, "cross");
    assert_eq!(options[3].id, "pass_2");
    assert_eq!(options[4].id, "pass_3");
    assert_eq!(
        seed,
        rng::seed(91.0),
        "candidate construction has no RNG state"
    );
    match options[4]
        .payload
        .as_ref()
        .and_then(|p| p.get("lane_fraction"))
    {
        Some(brain::BrainPayloadValue::Number(value)) => assert_eq!(*value, 0.45),
        other => panic!("expected a numeric lane_fraction payload value, got {other:?}"),
    }
}

#[test]
fn outfield_carrier_choices_keeps_shooting_as_a_scored_falloff_beyond_the_old_range() {
    let options = od::carrier_options(&od::OutfieldCarrierContext {
        goal_distance: 600.0,
        shoot_range: 260.0,
        cross_target: None,
        box_targets: 0,
        passes: vec![],
        ..carrier_context()
    });
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].kind, "shoot");
    assert!(options[0].score < options[1].score);
}

#[test]
fn outfield_carrier_choices_favors_carrying_in_open_space_and_a_legitimate_outlet_under_pressure() {
    let pass = od::OutfieldPassOption {
        player_index: 2,
        openness: 100.0,
        forward_progress: 150.0,
        distance: 170.0,
        lane_blocked: false,
        interception_risk: false,
        lane_fraction: None,
    };
    let open_options = od::carrier_options(&od::OutfieldCarrierContext {
        goal_distance: 600.0,
        cross_target: None,
        box_targets: 0,
        goal_progress: 0.3,
        dribble_space: 1.0,
        space: 1.0,
        passes: vec![pass],
        ..carrier_context()
    });
    let pressured_options = od::carrier_options(&od::OutfieldCarrierContext {
        goal_distance: 600.0,
        cross_target: None,
        box_targets: 0,
        goal_progress: 0.3,
        dribble_space: 0.65,
        space: 0.45,
        passes: vec![pass],
        ..carrier_context()
    });
    assert!(open_options[1].score > open_options[2].score);
    assert!(pressured_options[2].score > pressured_options[1].score);
}

#[test]
fn outfield_carrier_choices_uses_exact_argmax_at_the_high_composure_boundary_without_advancing_rng()
{
    let options = od::carrier_options(&carrier_context());
    let seed = rng::seed(17.0);
    let (selected, next_seed) = od::decide_carrier(&options, 0.8, 1.0, seed);
    let mut best = &options[0];
    for option in &options {
        if option.score > best.score {
            best = option;
        }
    }
    assert_eq!(selected.id, best.id);
    assert_eq!(next_seed, seed);
}

#[test]
fn outfield_carrier_choices_reproduces_a_sampled_action_and_next_rng_state_from_the_same_ordered_options()
 {
    let options = od::carrier_options(&carrier_context());
    let seed = rng::seed(71.0);
    let (first, first_next) = od::decide_carrier(&options, 0.4, 0.9, seed);
    let (second, second_next) = od::decide_carrier(&options, 0.4, 0.9, seed);
    assert_eq!(first.id, second.id);
    assert_eq!(first_next, second_next);
    assert_ne!(first_next, seed);
}

#[test]
fn outfield_carrier_choices_can_choose_only_a_legitimate_lower_ranked_option_just_below_the_sharp_boundary()
 {
    let options = vec![
        brain::BrainScoredOption {
            id: "best".to_string(),
            kind: "dribble".to_string(),
            score: 10.0,
            payload: None,
            reference: None,
        },
        brain::BrainScoredOption {
            id: "second".to_string(),
            kind: "pass".to_string(),
            score: 9.0,
            payload: None,
            reference: Some(brain::OptionReference::Index(2)),
        },
    ];
    let mut best = &options[0];
    for option in &options {
        if option.score > best.score {
            best = option;
        }
    }
    let best_id = best.id.clone();
    let mut saw_lower = false;
    let mut seed = 1000_i64;
    while seed <= 200_000 {
        let (selected, _) = od::decide_carrier(&options, 0.79, 1.0, rng::seed(seed as f64));
        let legitimate = options.iter().any(|option| selected.id == option.id);
        assert!(legitimate);
        if selected.id != best_id {
            saw_lower = true;
            break;
        }
        seed += 997;
    }
    assert!(
        saw_lower,
        "pressure never produced a legitimate lower-ranked choice"
    );
}
