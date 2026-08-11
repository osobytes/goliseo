//! Tests for `gc_sim::brain`.

use gc_core::rng;
use gc_sim::brain;
use indexmap::IndexMap;

fn phase_context() -> brain::BrainPhaseContext {
    brain::BrainPhaseContext {
        possession: brain::BrainPossession::Team,
        transition: None,
        transition_elapsed: 0.0,
        counterpress_window: 2.5,
        counterattack_window: 2.5,
    }
}

fn press_context() -> brain::BrainPressContext {
    brain::BrainPressContext {
        heavy_touch: false,
        exposed_ball: false,
        cover_available: false,
        box_desperation: false,
        press_discipline: 0.8,
        low_discipline_threshold: 0.35,
    }
}

fn option(id: &str, score: f64) -> brain::BrainScoredOption {
    option_kind(id, score, "pass")
}

fn option_kind(id: &str, score: f64, kind: &str) -> brain::BrainScoredOption {
    brain::BrainScoredOption {
        id: id.to_string(),
        kind: kind.to_string(),
        score,
        payload: None,
        reference: Some(brain::OptionReference::Text(id.to_string())),
    }
}

fn run_slot(player_index: u32, granted_at: f64, expires_at: f64) -> brain::RunSlot {
    brain::RunSlot {
        player_index,
        run_type: brain::RunType::ComeShort,
        score: 1.0,
        target_x: 450.0,
        target_y: 270.0,
        granted_at,
        expires_at,
    }
}

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6,
        "expected ~{expected}, got {actual}"
    );
}

#[test]
fn brain_phase_returns_ordinary_phases_without_an_active_transition() {
    assert_eq!(brain::phase(&phase_context()), brain::TeamPhase::Attack);
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            possession: brain::BrainPossession::Opponent,
            ..phase_context()
        }),
        brain::TeamPhase::Defend
    );
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            possession: brain::BrainPossession::Loose,
            ..phase_context()
        }),
        brain::TeamPhase::Loose
    );
}

#[test]
fn brain_phase_keeps_transition_phases_only_inside_their_caller_owned_windows() {
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            possession: brain::BrainPossession::Loose,
            transition: Some(brain::BrainTransition::Lost),
            transition_elapsed: 2.49,
            counterpress_window: 2.5,
            ..phase_context()
        }),
        brain::TeamPhase::Counterpress
    );
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            transition: Some(brain::BrainTransition::Won),
            transition_elapsed: 2.49,
            counterattack_window: 2.5,
            ..phase_context()
        }),
        brain::TeamPhase::Counterattack
    );
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            possession: brain::BrainPossession::Opponent,
            transition: Some(brain::BrainTransition::Lost),
            transition_elapsed: 2.5,
            counterpress_window: 2.5,
            ..phase_context()
        }),
        brain::TeamPhase::Defend
    );
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            transition: Some(brain::BrainTransition::Won),
            transition_elapsed: 2.5,
            counterattack_window: 2.5,
            ..phase_context()
        }),
        brain::TeamPhase::Attack
    );
}

#[test]
fn brain_phase_disables_a_transition_when_its_supplied_window_is_zero() {
    assert_eq!(
        brain::phase(&brain::BrainPhaseContext {
            possession: brain::BrainPossession::Opponent,
            transition: Some(brain::BrainTransition::Lost),
            counterpress_window: 0.0,
            ..phase_context()
        }),
        brain::TeamPhase::Defend
    );
}

#[test]
fn brain_refresh_interval_maps_scan_rate_linearly_from_the_slow_endpoint_to_the_fast_endpoint() {
    assert_near(brain::refresh_interval(0.0, 0.45, 0.15), 0.45);
    assert_near(brain::refresh_interval(0.5, 0.45, 0.15), 0.3);
    assert_near(brain::refresh_interval(1.0, 0.45, 0.15), 0.15);
}

#[test]
fn brain_refresh_interval_saturates_out_of_range_and_non_finite_scan_rates() {
    assert_near(brain::refresh_interval(-2.0, 0.45, 0.15), 0.45);
    assert_near(brain::refresh_interval(2.0, 0.45, 0.15), 0.15);
    assert_near(brain::refresh_interval(f64::NAN, 0.45, 0.15), 0.45);
    assert_near(brain::refresh_interval(f64::INFINITY, 0.45, 0.15), 0.15);
}

#[test]
fn brain_refresh_interval_accepts_an_equal_fixed_interval() {
    assert_near(brain::refresh_interval(0.0, 0.3, 0.3), 0.3);
    assert_near(brain::refresh_interval(1.0, 0.3, 0.3), 0.3);
}

#[test]
#[should_panic]
fn brain_refresh_interval_rejects_semantically_reversed_interval_endpoints() {
    let _ = brain::refresh_interval(0.5, 0.15, 0.45);
}

#[test]
fn brain_assign_presser_uses_distance_then_player_index_as_a_deterministic_total_order() {
    let candidates = [
        brain::PresserCandidate {
            player_index: 7,
            distance_cost: 20.0,
            eligible: None,
        },
        brain::PresserCandidate {
            player_index: 3,
            distance_cost: 20.0,
            eligible: None,
        },
        brain::PresserCandidate {
            player_index: 2,
            distance_cost: 30.0,
            eligible: None,
        },
    ];
    assert_eq!(brain::assign_presser(&candidates, None, 0.15), Some(3));
}

#[test]
fn brain_assign_presser_keeps_the_current_eligible_presser_through_marginal_changes() {
    let mut candidates = [
        brain::PresserCandidate {
            player_index: 1,
            distance_cost: 100.0,
            eligible: None,
        },
        brain::PresserCandidate {
            player_index: 2,
            distance_cost: 86.0,
            eligible: None,
        },
    ];
    assert_eq!(brain::assign_presser(&candidates, Some(1), 0.15), Some(1));
    candidates[1].distance_cost = 85.0;
    assert_eq!(brain::assign_presser(&candidates, Some(1), 0.15), Some(2));
}

#[test]
fn brain_assign_presser_replaces_an_ineligible_current_presser_and_returns_nil_without_candidates()
{
    let candidates = [
        brain::PresserCandidate {
            player_index: 1,
            distance_cost: 5.0,
            eligible: Some(false),
        },
        brain::PresserCandidate {
            player_index: 2,
            distance_cost: 20.0,
            eligible: None,
        },
    ];
    assert_eq!(brain::assign_presser(&candidates, Some(1), 0.15), Some(2));
    assert_eq!(
        brain::assign_presser(
            &[brain::PresserCandidate {
                player_index: 1,
                distance_cost: 5.0,
                eligible: Some(false)
            }],
            Some(1),
            0.15
        ),
        None
    );
}

fn run_arbitration_context() -> brain::BrainRunContext {
    brain::BrainRunContext {
        players: vec![
            brain::BrainRunPlayer {
                player_index: 4,
                eligible: None,
                in_behind: Some(brain::BrainRunTarget {
                    score: 8.0,
                    x: 800.0,
                    y: 200.0,
                    duration: 1.8,
                }),
                come_short: Some(brain::BrainRunTarget {
                    score: 6.0,
                    x: 500.0,
                    y: 240.0,
                    duration: 1.5,
                }),
                hold_width: None,
            },
            brain::BrainRunPlayer {
                player_index: 2,
                eligible: None,
                in_behind: None,
                come_short: None,
                hold_width: Some(brain::BrainRunTarget {
                    score: 8.0,
                    x: 620.0,
                    y: 50.0,
                    duration: 1.8,
                }),
            },
            brain::BrainRunPlayer {
                player_index: 3,
                eligible: Some(false),
                in_behind: Some(brain::BrainRunTarget {
                    score: 99.0,
                    x: 900.0,
                    y: 270.0,
                    duration: 1.8,
                }),
                come_short: None,
                hold_width: None,
            },
        ],
    }
}

#[test]
fn brain_run_arbitration_builds_a_deterministic_total_order_across_all_run_types() {
    let candidates = brain::run_candidates(&run_arbitration_context());
    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].player_index, 2);
    assert_eq!(candidates[0].run_type, brain::RunType::HoldWidth);
    assert_eq!(candidates[1].player_index, 4);
    assert_eq!(candidates[1].run_type, brain::RunType::InBehind);
    assert_eq!(candidates[2].run_type, brain::RunType::ComeShort);
}

#[test]
fn brain_run_arbitration_respects_the_cap_and_grants_only_the_best_request_per_player() {
    let candidates = brain::run_candidates(&run_arbitration_context());
    let slots = brain::grant_runs(&candidates, &[], 2, 10.0);
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].player_index, 2);
    assert_eq!(slots[1].player_index, 4);
    assert_eq!(slots[0].expires_at, 11.8);
}

#[test]
fn brain_run_arbitration_preserves_unexpired_active_slots_instead_of_re_litigating_them() {
    let active = [brain::RunSlot {
        player_index: 4,
        run_type: brain::RunType::ComeShort,
        score: 1.0,
        target_x: 450.0,
        target_y: 270.0,
        granted_at: 8.0,
        expires_at: 12.0,
    }];
    let candidates = [brain::RunCandidate {
        player_index: 2,
        run_type: brain::RunType::InBehind,
        score: 100.0,
        target_x: 900.0,
        target_y: 200.0,
        duration: 1.8,
    }];
    let slots = brain::grant_runs(&candidates, &active, 1, 10.0);
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].player_index, 4);
    assert_eq!(slots[0].run_type, brain::RunType::ComeShort);
    assert_eq!(slots[0].expires_at, 12.0);
    assert_eq!(
        active[0].expires_at, 12.0,
        "the resolver does not mutate caller state"
    );
}

#[test]
fn brain_run_arbitration_replaces_expired_slots_from_the_ranked_candidate_list() {
    let active = [brain::RunSlot {
        player_index: 4,
        run_type: brain::RunType::ComeShort,
        score: 1.0,
        target_x: 450.0,
        target_y: 270.0,
        granted_at: 8.0,
        expires_at: 10.0,
    }];
    let candidates = [brain::RunCandidate {
        player_index: 2,
        run_type: brain::RunType::InBehind,
        score: 9.0,
        target_x: 900.0,
        target_y: 200.0,
        duration: 1.8,
    }];
    let slots = brain::grant_runs(&candidates, &active, 1, 10.0);
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].player_index, 2);
    assert_eq!(slots[0].granted_at, 10.0);
    assert_eq!(slots[0].expires_at, 11.8);
}

#[test]
fn brain_run_arbitration_returns_no_slots_when_the_configured_maximum_is_zero() {
    let candidates = [brain::RunCandidate {
        player_index: 2,
        run_type: brain::RunType::InBehind,
        score: 9.0,
        target_x: 900.0,
        target_y: 200.0,
        duration: 1.8,
    }];
    let slots = brain::grant_runs(&candidates, &[run_slot(4, 8.0, 12.0)], 0, 10.0);
    assert_eq!(slots.len(), 0);
}

#[test]
fn brain_run_arbitration_preserves_the_earliest_grants_when_active_slots_exceed_a_lowered_cap() {
    let active = [
        run_slot(4, 7.0, 20.0),
        run_slot(3, 5.0, 20.0),
        run_slot(2, 6.0, 20.0),
    ];
    let slots = brain::grant_runs(&[], &active, 2, 10.0);
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].player_index, 3);
    assert_eq!(slots[1].player_index, 2);
}

#[test]
fn brain_press_mode_attributes_every_disciplined_commit_to_one_stable_reason() {
    let cases: &[(&str, brain::PressReason)] = &[
        ("heavy_touch", brain::PressReason::HeavyTouch),
        ("exposed_ball", brain::PressReason::ExposedBall),
        ("cover_available", brain::PressReason::Cover),
        ("box_desperation", brain::PressReason::BoxDesperation),
    ];
    for (field, expected_reason) in cases {
        let context = match *field {
            "heavy_touch" => brain::BrainPressContext {
                heavy_touch: true,
                ..press_context()
            },
            "exposed_ball" => brain::BrainPressContext {
                exposed_ball: true,
                ..press_context()
            },
            "cover_available" => brain::BrainPressContext {
                cover_available: true,
                ..press_context()
            },
            "box_desperation" => brain::BrainPressContext {
                box_desperation: true,
                ..press_context()
            },
            _ => unreachable!(),
        };
        let (mode, reason) = brain::press_mode(&context);
        assert_eq!(mode, brain::PressMode::Commit, "{field}");
        assert_eq!(reason, *expected_reason, "{field}");
    }
}

#[test]
fn brain_press_mode_uses_a_distinct_low_discipline_fallback() {
    let (mode, reason) = brain::press_mode(&brain::BrainPressContext {
        press_discipline: 0.2,
        ..press_context()
    });
    assert_eq!(mode, brain::PressMode::Commit);
    assert_eq!(reason, brain::PressReason::LowDiscipline);
}

#[test]
fn brain_press_mode_contains_without_a_commit_trigger() {
    let (mode, reason) = brain::press_mode(&press_context());
    assert_eq!(mode, brain::PressMode::Contain);
    assert_eq!(reason, brain::PressReason::NoTrigger);
}

#[test]
fn brain_press_mode_contains_at_the_low_discipline_threshold_boundary() {
    let (mode, reason) = brain::press_mode(&brain::BrainPressContext {
        press_discipline: 0.35,
        low_discipline_threshold: 0.35,
        ..press_context()
    });
    assert_eq!(mode, brain::PressMode::Contain);
    assert_eq!(reason, brain::PressReason::NoTrigger);
}

#[test]
fn brain_press_mode_uses_a_stable_trigger_precedence() {
    let (mode, reason) = brain::press_mode(&brain::BrainPressContext {
        heavy_touch: true,
        exposed_ball: true,
        cover_available: true,
        box_desperation: true,
        press_discipline: 0.0,
        ..press_context()
    });
    assert_eq!(mode, brain::PressMode::Commit);
    assert_eq!(reason, brain::PressReason::HeavyTouch);
}

#[test]
fn brain_scored_option_selection_returns_exact_argmax_without_consuming_rng_at_zero_temperature() {
    let state = rng::seed(71.0);
    let options = [
        option("safe", 4.0),
        option("best", 9.0),
        option("risky", 7.0),
    ];
    let (selected, next_state) = brain::select_scored_option(&options, 0.0, state);
    assert_eq!(selected.id, "best");
    assert_eq!(next_state, state);
}

#[test]
fn brain_scored_option_selection_breaks_argmax_ties_by_stable_kind_and_id() {
    let options = [
        option_kind("z", 9.0, "shoot"),
        option_kind("b", 9.0, "pass"),
        option_kind("a", 9.0, "pass"),
    ];
    let (selected, _) = brain::select_scored_option(&options, 0.0, rng::seed(1.0));
    assert_eq!(selected.kind, "pass");
    assert_eq!(selected.id, "a");
}

#[test]
fn brain_scored_option_selection_is_reproducible_for_the_same_options_and_seed() {
    let options = [
        option("short", 1.0),
        option("through", 1.1),
        option_kind("carry", 0.9, "dribble"),
    ];
    let (first, first_state) = brain::select_scored_option(&options, 0.8, rng::seed(904.0));
    let (second, second_state) = brain::select_scored_option(&options, 0.8, rng::seed(904.0));
    assert_eq!(first.kind, second.kind);
    assert_eq!(first.id, second.id);
    assert_eq!(first_state, second_state);
}

#[test]
fn brain_scored_option_selection_is_stable_when_caller_option_order_changes() {
    let ordered = [
        option("short", 1.0),
        option("through", 1.1),
        option_kind("carry", 0.9, "dribble"),
    ];
    let reversed = [ordered[2].clone(), ordered[1].clone(), ordered[0].clone()];
    let (first, first_state) = brain::select_scored_option(&ordered, 0.8, rng::seed(904.0));
    let (second, second_state) = brain::select_scored_option(&reversed, 0.8, rng::seed(904.0));
    assert_eq!(first.kind, second.kind);
    assert_eq!(first.id, second.id);
    assert_eq!(first_state, second_state);
}

#[test]
fn brain_scored_option_selection_treats_delimiter_like_kind_and_id_bytes_as_distinct_identity_fields()
 {
    let options = [option_kind("c", 2.0, "a\0b"), option_kind("b\0c", 1.0, "a")];
    let (selected, _) = brain::select_scored_option(&options, 0.0, rng::seed(1.0));
    assert_eq!(selected.kind, "a\0b");
    assert_eq!(selected.id, "c");
}

#[test]
fn brain_scored_option_selection_keeps_the_generic_selector_open_to_non_soccer_kinds_and_payloads()
{
    let mut payload = IndexMap::new();
    payload.insert(
        "family".to_string(),
        brain::BrainPayloadValue::Text("light_melee".to_string()),
    );
    payload.insert("target".to_string(), brain::BrainPayloadValue::Number(4.0));
    let options = [
        brain::BrainScoredOption {
            id: "guard-break".to_string(),
            kind: "equipment".to_string(),
            score: 10.0,
            payload: Some(payload),
            reference: None,
        },
        option("pass", 2.0),
    ];
    let (selected, _) = brain::select_scored_option(&options, 0.0, rng::seed(15.0));
    assert_eq!(selected.kind, "equipment");
    match selected.payload.as_ref().and_then(|p| p.get("family")) {
        Some(brain::BrainPayloadValue::Text(text)) => assert_eq!(text, "light_melee"),
        other => panic!("expected a text family payload value, got {other:?}"),
    }
}

#[test]
fn brain_scored_option_selection_makes_a_fully_composed_carrier_deterministic_under_pressure() {
    let state = rng::seed(15.0);
    let options = [option("best", 10.0), option("other", 9.0)];
    let (selected, next_state) = brain::decide_carrier(&options, 1.0, 1.0, 2.0, state);
    assert_eq!(selected.id, "best");
    assert_eq!(next_state, state);
}

#[test]
fn brain_scored_option_selection_threads_explicit_rng_state_when_soft_selection_is_active() {
    let state = rng::seed(15.0);
    let options = [option("best", 10.0), option("other", 9.0)];
    let (_, next_state) = brain::decide_carrier(&options, 0.0, 1.0, 2.0, state);
    assert_ne!(next_state, state);
    let (expected_state, _) = rng::roll(state);
    assert_eq!(next_state, expected_state);
}

#[test]
fn brain_scored_option_selection_uses_uniform_seeded_selection_for_direct_positive_infinite_temperature()
 {
    let state = rng::seed(1.0);
    let options = [option("a_low", -100.0), option("z_best", 100.0)];
    let (selected, next_state) = brain::select_scored_option(&options, f64::INFINITY, state);
    let (expected_state, _) = rng::roll(state);
    assert_eq!(selected.id, "a_low");
    assert_eq!(next_state, expected_state);
}

#[test]
fn brain_scored_option_selection_uses_uniform_seeded_selection_when_finite_carrier_temperature_overflows()
 {
    let state = rng::seed(1.0);
    let options = [option("a_low", -100.0), option("z_best", 100.0)];
    let (selected, next_state) = brain::decide_carrier(&options, 0.0, 1.0, 1e308, state);
    let (expected_state, _) = rng::roll(state);
    assert_eq!(selected.id, "a_low");
    assert_eq!(next_state, expected_state);
}
