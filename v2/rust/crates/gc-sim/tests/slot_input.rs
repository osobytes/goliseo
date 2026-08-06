//! Port of `spec/sim/slot_input_spec.lua`.
//!
//! Sixteen of these twenty-one cases build their fixture through the Lua
//! helpers `new_match()` / `new_legacy_match()`, which call `match.new`
//! (`sim/match.lua`, not yet ported); those are `#[ignore]`d. The remaining
//! five test `slot_input::to_match_input` / `to_sample` / `new_producer`
//! directly against hand-built `InputSample`/`MatchSlotSource` values with
//! no match fixture at all, and port in full.

use gc_sim::input_frame;
use gc_sim::slot_input::{self, MatchSlotSource, MatchSlotSourceKind};

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::match (sim/match.lua), not yet ported"]
        fn $name() {
            unimplemented!("needs sim::match (sim/match.lua)")
        }
    };
}

fn frame_sources() -> [MatchSlotSource; 8] {
    [MatchSlotSource {
        kind: MatchSlotSourceKind::Frame,
        seed: None,
    }; 8]
}

#[test]
fn fixed_match_input_slots_converts_a_neutral_sample_without_mistaking_valid_false_bits_for_an_error()
 {
    let input = slot_input::to_match_input(&input_frame::neutral_sample());
    assert_eq!(input.r#move.x, 0.0);
    assert_eq!(input.r#move.y, 0.0);
    assert!(!input.shoot);
    assert!(!input.pass);
    assert!(!input.sprint);
    assert_eq!(input.aerial_strike, Some(false));
    assert!(!input.equipment_held);
    assert!(!input.equipment_pressed);
    assert!(!input.equipment_released);
}

#[test]
fn fixed_match_input_slots_round_trips_canonical_equipment_held_and_edge_intent() {
    let mut pressed = slot_input::neutral_match_input();
    pressed.equipment_held = true;
    pressed.equipment_pressed = true;
    let pressed_sample = slot_input::to_sample(&pressed);
    let pressed_input = slot_input::to_match_input(&pressed_sample);
    assert!(pressed_input.equipment_held);
    assert!(pressed_input.equipment_pressed);
    assert!(!pressed_input.equipment_released);

    let mut tapped = slot_input::neutral_match_input();
    tapped.equipment_pressed = true;
    tapped.equipment_released = true;
    let tapped_input = slot_input::to_match_input(&slot_input::to_sample(&tapped));
    assert!(!tapped_input.equipment_held);
    assert!(tapped_input.equipment_pressed);
    assert!(tapped_input.equipment_released);
}

blocked!(fixed_match_input_slots_maps_exactly_four_permanent_outfield_slots_per_side_and_excludes_both_keepers);
blocked!(fixed_match_input_slots_routes_simultaneous_opposing_rows_without_reading_controlled_or_possession);
blocked!(fixed_match_input_slots_keeps_ownership_stable_through_legacy_metadata_turnover_kickoff_and_aerial_state_changes);
blocked!(fixed_match_input_slots_does_not_hand_slot_mode_legacy_selection_to_a_pass_receiver);
blocked!(
    fixed_match_input_slots_suppresses_the_real_legacy_turnover_and_aerial_reselection_branches
);
blocked!(fixed_match_input_slots_clears_a_heavy_touch_carrier_before_the_loss_tick_ends);
blocked!(
    fixed_match_input_slots_keeps_concurrent_holds_and_wind_up_cancellation_on_their_owning_players
);
blocked!(fixed_match_input_slots_resolves_simultaneous_pass_and_tackle_releases_without_overwriting_either_slot);
blocked!(
    fixed_match_input_slots_resolves_a_direct_same_tick_tackle_before_the_carriers_pass_release
);
blocked!(
    fixed_match_input_slots_keeps_slot_selection_fixed_through_switching_keeper_capture_and_kickoff
);
blocked!(fixed_match_input_slots_keeps_online_input_off_the_keeper_while_deterministic_keeper_ai_distributes);
blocked!(fixed_match_input_slots_materializes_only_explicitly_bot_configured_slots_from_independent_seeded_streams);

#[test]
fn fixed_match_input_slots_rejects_non_finite_bot_seeds() {
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(f64::INFINITY),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "positive infinity cannot seed a bot"
    );

    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(f64::NEG_INFINITY),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "negative infinity cannot seed a bot"
    );

    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(f64::NAN),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "NaN cannot seed a bot"
    );
}

#[test]
fn fixed_match_input_slots_rejects_seeds_on_non_bot_slot_sources() {
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Frame,
        seed: Some(73.0),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "frame rows cannot carry bot seed identity"
    );

    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Neutral,
        seed: Some(73.0),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "neutral rows cannot carry bot seed identity"
    );
}

#[test]
fn fixed_match_input_slots_canonicalizes_stored_bot_seeds_before_materializing() {
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(-901.0),
    };
    let producer = slot_input::new_producer(sources);
    assert_eq!(producer.sources[0].seed, Some(901.0));
}

blocked!(
    fixed_match_input_slots_materializes_frame_neutral_and_quantized_bot_rows_before_sim_match
);
blocked!(fixed_match_input_slots_replays_effective_bot_filled_frames_with_an_all_frame_producer);
blocked!(fixed_match_input_slots_requires_the_exact_fixed_tick_interval_in_slot_mode);
blocked!(fixed_match_input_slots_rejects_a_legacy_matchinput_in_slot_mode);
