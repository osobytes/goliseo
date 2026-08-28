//! Tests for `gc_sim::stats`.

use gc_data::players::StatBlock;
use gc_sim::stats;
use std::f64::consts::PI;

fn block(pace: i64, strength: i64, mental: i64, technique: i64, stamina: i64) -> StatBlock {
    StatBlock {
        pace,
        strength,
        technique,
        stamina,
        mental,
    }
}

fn block2(pace: i64, strength: i64) -> StatBlock {
    block(pace, strength, 5, 5, 5)
}

fn block3(pace: i64, strength: i64, mental: i64) -> StatBlock {
    block(pace, strength, mental, 5, 5)
}

fn block4(pace: i64, strength: i64, mental: i64, technique: i64) -> StatBlock {
    block(pace, strength, mental, technique, 5)
}

#[test]
fn stats_move_speed_increases_with_pace_without_changing_the_established_mapping() {
    let slow = stats::move_speed(block2(2, 5));
    let fast = stats::move_speed(block2(8, 5));
    assert!(fast > slow, "faster player should have higher move speed");
    assert_eq!(fast, 220.0, "pace 8 keeps the pre-migration derived speed");
}

#[test]
fn stats_shot_speed_increases_with_strength_without_changing_the_established_mapping() {
    let weak = stats::shot_speed(block2(5, 2));
    let strong = stats::shot_speed(block2(5, 8));
    assert!(strong > weak, "stronger player should shoot faster");
    assert_eq!(
        strong, 550.0,
        "strength 8 keeps the pre-migration derived shot speed"
    );
}

#[test]
fn stats_move_speed_is_positive_at_zero_pace() {
    assert!(stats::move_speed(block2(0, 0)) > 0.0);
}

#[test]
fn stats_maps_outfield_scan_rate_exactly_and_clamps_it_to_the_unit_interval() {
    assert_eq!(stats::scan_rate(block(5, 5, 0, 5, 0)), 0.0);
    assert_eq!(stats::scan_rate(block(5, 5, 5, 5, 5)), 0.5);
    assert_eq!(stats::scan_rate(block(5, 5, 10, 5, 10)), 1.0);
    assert_eq!(stats::scan_rate(block(5, 5, -2, 5, -2)), 0.0);
    assert_eq!(stats::scan_rate(block(5, 5, 12, 5, 12)), 1.0);
}

#[test]
fn stats_weights_mental_and_stamina_independently_in_outfield_scan_rate() {
    assert_eq!(stats::scan_rate(block(5, 5, 8, 5, 4)), 0.7);
    assert_eq!(stats::scan_rate(block(5, 5, 4, 5, 8)), 0.5);
}

#[test]
fn stats_never_lowers_outfield_scan_rate_as_mental_or_stamina_increases() {
    let mut previous_mental = stats::scan_rate(block(5, 5, 0, 5, 5));
    let mut previous_stamina = stats::scan_rate(block(5, 5, 5, 5, 0));
    for value in 1..=10 {
        let current_mental = stats::scan_rate(block(5, 5, value, 5, 5));
        let current_stamina = stats::scan_rate(block(5, 5, 5, 5, value));
        assert!(current_mental >= previous_mental);
        assert!(current_stamina >= previous_stamina);
        previous_mental = current_mental;
        previous_stamina = current_stamina;
    }
}

#[test]
fn stats_maps_outfield_composure_and_press_discipline_from_mental_only() {
    let functions: [fn(StatBlock) -> f64; 2] = [stats::composure, stats::press_discipline];
    for derive in functions {
        assert_eq!(derive(block3(5, 5, 0)), 0.0);
        assert_eq!(derive(block3(5, 5, 5)), 0.5);
        assert_eq!(derive(block3(5, 5, 10)), 1.0);
        assert_eq!(derive(block3(5, 5, -2)), 0.0);
        assert_eq!(derive(block3(5, 5, 12)), 1.0);
        assert_eq!(derive(block(0, 0, 5, 0, 0)), 0.5);
        assert_eq!(derive(block(10, 10, 5, 10, 10)), 0.5);
    }
}

#[test]
fn stats_never_lowers_outfield_composure_or_press_discipline_as_mental_increases() {
    let mut previous_composure = stats::composure(block3(5, 5, 0));
    let mut previous_discipline = stats::press_discipline(block3(5, 5, 0));
    for mental in 1..=10 {
        let current_composure = stats::composure(block3(5, 5, mental));
        let current_discipline = stats::press_discipline(block3(5, 5, mental));
        assert!(current_composure >= previous_composure);
        assert!(current_discipline >= previous_discipline);
        previous_composure = current_composure;
        previous_discipline = current_discipline;
    }
}

#[test]
fn stats_maps_outfield_run_drive_exactly_and_clamps_it_to_the_unit_interval() {
    assert_eq!(stats::run_drive(block3(0, 5, 0)), 0.0);
    assert_eq!(stats::run_drive(block3(5, 5, 5)), 0.5);
    assert_eq!(stats::run_drive(block3(10, 5, 10)), 1.0);
    assert_eq!(stats::run_drive(block3(-2, 5, -2)), 0.0);
    assert_eq!(stats::run_drive(block3(12, 5, 12)), 1.0);
    assert_eq!(stats::run_drive(block3(8, 5, 4)), 0.64);
}

#[test]
fn stats_never_lowers_outfield_run_drive_as_pace_or_mental_increases() {
    let mut previous_pace = stats::run_drive(block3(0, 5, 5));
    let mut previous_mental = stats::run_drive(block3(5, 5, 0));
    for value in 1..=10 {
        let current_pace = stats::run_drive(block3(value, 5, 5));
        let current_mental = stats::run_drive(block3(5, 5, value));
        assert!(current_pace >= previous_pace);
        assert!(current_mental >= previous_mental);
        previous_pace = current_pace;
        previous_mental = current_mental;
    }
}

#[test]
fn stats_reconstructs_run_drive_bit_exactly_from_every_canonical_match_scalar_pair() {
    for pace in 0..=10 {
        for mental in 0..=10 {
            let stat_block = block3(pace, 5, mental);
            assert_eq!(
                stats::run_drive_from_match(
                    stats::move_speed(stat_block),
                    stats::composure(stat_block)
                ),
                stats::run_drive(stat_block),
                "pace={pace} mental={mental}"
            );
        }
    }
}

#[test]
fn stats_maps_technique_to_a_bounded_maximum_execution_error_in_radians() {
    let maximum = PI / 15.0;
    assert_eq!(stats::execution_error(block4(5, 5, 5, 0)), maximum);
    assert_eq!(stats::execution_error(block4(5, 5, 5, 5)), maximum / 2.0);
    assert_eq!(stats::execution_error(block4(5, 5, 5, 10)), 0.0);
    assert_eq!(stats::execution_error(block4(5, 5, 5, -2)), maximum);
    assert_eq!(stats::execution_error(block4(5, 5, 5, 12)), 0.0);
}

#[test]
fn stats_never_increases_execution_error_as_technique_increases() {
    let mut previous = stats::execution_error(block4(5, 5, 5, 0));
    for technique in 1..=10 {
        let current = stats::execution_error(block4(5, 5, 5, technique));
        assert!(current <= previous);
        previous = current;
    }
}

#[test]
fn stats_recovers_execution_error_bit_exactly_from_serialized_outfield_derivations() {
    for technique in 0..=10 {
        for mental in 0..=10 {
            let source = block4(5, 5, mental, technique);
            assert_eq!(
                stats::execution_error_from_outfield(
                    stats::first_touch(source),
                    stats::composure(source)
                ),
                stats::execution_error(source),
                "technique={technique} mental={mental}"
            );
        }
    }
}

#[test]
fn stats_keeps_unrelated_stats_out_of_outfield_behavior_derivations() {
    let low = block(4, 0, 6, 7, 8);
    let high = block(4, 10, 6, 7, 8);
    assert_eq!(stats::scan_rate(low), stats::scan_rate(high));
    assert_eq!(stats::composure(low), stats::composure(high));
    assert_eq!(stats::press_discipline(low), stats::press_discipline(high));
    assert_eq!(stats::run_drive(low), stats::run_drive(high));
    assert_eq!(stats::execution_error(low), stats::execution_error(high));
}

#[test]
fn stats_derives_keeper_reach_from_mental_and_pace() {
    let composed = stats::keeper_reach(block3(4, 5, 8));
    let unsettled = stats::keeper_reach(block3(4, 5, 2));
    assert_eq!(
        composed, 78.0,
        "the migrated mental value keeps the existing reach mapping"
    );
    assert!(
        composed > unsettled,
        "mental should improve derived defensive reach"
    );
}

#[test]
fn stats_maps_keeper_anticipation_exactly_and_clamps_it_to_the_unit_interval() {
    assert_eq!(stats::keeper_anticipation(block3(5, 5, 0)), 0.0);
    assert_eq!(stats::keeper_anticipation(block3(5, 5, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block3(5, 5, 10)), 1.0);
    assert_eq!(stats::keeper_anticipation(block3(5, 5, -2)), 0.0);
    assert_eq!(stats::keeper_anticipation(block3(5, 5, 12)), 1.0);
}

#[test]
fn stats_never_lowers_keeper_anticipation_as_mental_increases() {
    let mut previous = stats::keeper_anticipation(block3(5, 5, 0));
    for mental in 1..=10 {
        let current = stats::keeper_anticipation(block3(5, 5, mental));
        assert!(current >= previous);
        previous = current;
    }
}

#[test]
fn stats_uses_only_mental_to_derive_keeper_anticipation() {
    assert_eq!(stats::keeper_anticipation(block(0, 5, 5, 5, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(10, 5, 5, 5, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(5, 0, 5, 5, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(5, 10, 5, 5, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(5, 5, 5, 0, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(5, 5, 5, 10, 5)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(5, 5, 5, 5, 0)), 0.5);
    assert_eq!(stats::keeper_anticipation(block(5, 5, 5, 5, 10)), 0.5);
}

#[test]
fn stats_maps_keeper_aggression_to_a_futsal_scaled_positive_pixel_distance() {
    assert_eq!(stats::keeper_aggression(block3(0, 5, 0)), 31.0);
    assert_eq!(stats::keeper_aggression(block3(5, 5, 5)), 66.0);
    assert_eq!(stats::keeper_aggression(block3(10, 5, 10)), 101.0);
}

#[test]
fn stats_adds_exact_independent_pace_and_mental_contributions_to_keeper_aggression() {
    let pace_four = stats::keeper_aggression(block3(4, 5, 7));
    let pace_five = stats::keeper_aggression(block3(5, 5, 7));
    assert_eq!(pace_four, 69.5);
    assert_eq!(pace_five, 73.0);
    assert_eq!(pace_five - pace_four, 3.5);

    let mental_four = stats::keeper_aggression(block3(7, 5, 4));
    let mental_five = stats::keeper_aggression(block3(7, 5, 5));
    assert_eq!(mental_four, 69.5);
    assert_eq!(mental_five, 73.0);
    assert_eq!(mental_five - mental_four, 3.5);
}

#[test]
fn stats_never_lowers_keeper_aggression_as_pace_increases() {
    let mut previous = stats::keeper_aggression(block3(0, 5, 5));
    for pace in 1..=10 {
        let current = stats::keeper_aggression(block3(pace, 5, 5));
        assert!(current >= previous);
        previous = current;
    }
}

#[test]
fn stats_never_lowers_keeper_aggression_as_mental_increases() {
    let mut previous = stats::keeper_aggression(block3(5, 5, 0));
    for mental in 1..=10 {
        let current = stats::keeper_aggression(block3(5, 5, mental));
        assert!(current >= previous);
        previous = current;
    }
}

#[test]
fn stats_uses_only_pace_and_mental_to_derive_keeper_aggression() {
    assert_eq!(stats::keeper_aggression(block(5, 0, 5, 5, 5)), 66.0);
    assert_eq!(stats::keeper_aggression(block(5, 10, 5, 5, 5)), 66.0);
    assert_eq!(stats::keeper_aggression(block(5, 5, 5, 0, 5)), 66.0);
    assert_eq!(stats::keeper_aggression(block(5, 5, 5, 10, 5)), 66.0);
    assert_eq!(stats::keeper_aggression(block(5, 5, 5, 5, 0)), 66.0);
    assert_eq!(stats::keeper_aggression(block(5, 5, 5, 5, 10)), 66.0);
}

#[test]
fn stats_maps_keeper_distribution_accuracy_exactly_and_clamps_it_to_the_unit_interval() {
    assert_eq!(stats::keeper_distribution_accuracy(block4(5, 5, 5, 0)), 0.0);
    assert_eq!(stats::keeper_distribution_accuracy(block4(5, 5, 5, 5)), 0.5);
    assert_eq!(
        stats::keeper_distribution_accuracy(block4(5, 5, 5, 10)),
        1.0
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block4(5, 5, 5, -2)),
        0.0
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block4(5, 5, 5, 12)),
        1.0
    );
}

#[test]
fn stats_never_lowers_keeper_distribution_accuracy_as_technique_increases() {
    let mut previous = stats::keeper_distribution_accuracy(block4(5, 5, 5, 0));
    for technique in 1..=10 {
        let current = stats::keeper_distribution_accuracy(block4(5, 5, 5, technique));
        assert!(current >= previous);
        previous = current;
    }
}

#[test]
fn stats_uses_only_technique_to_derive_keeper_distribution_accuracy() {
    assert_eq!(
        stats::keeper_distribution_accuracy(block(0, 5, 5, 5, 5)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(10, 5, 5, 5, 5)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(5, 0, 5, 5, 5)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(5, 10, 5, 5, 5)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(5, 5, 0, 5, 5)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(5, 5, 10, 5, 5)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(5, 5, 5, 5, 0)),
        0.5
    );
    assert_eq!(
        stats::keeper_distribution_accuracy(block(5, 5, 5, 5, 10)),
        0.5
    );
}
