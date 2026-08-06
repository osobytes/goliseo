//! Port of `spec/sim/combat_spec.lua`.
//!
//! Every case in this spec builds its fixture through the Lua helper
//! `new_match()`, which calls `match.new` (`sim/match.lua`). That module is
//! still an unported placeholder (`gc_sim::r#match`), so none of these
//! eighteen cases can be expressed yet — each is stubbed here, accurately
//! named and `#[ignore]`d, so the assertion set is tracked rather than
//! silently dropped (README rule: "never delete it silently").

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::match (sim/match.lua), not yet ported"]
        fn $name() {
            unimplemented!("needs sim::match (sim/match.lua)")
        }
    };
}

// -- "combat request lifecycle" --------------------------------------------

blocked!(
    combat_request_lifecycle_types_every_blocked_equipment_press_with_its_closed_rejection_reason
);
blocked!(combat_request_lifecycle_records_nothing_without_a_press_edge_and_types_an_accepted_press);
blocked!(combat_request_lifecycle_closes_an_accepted_sequence_exactly_once_for_every_family);
blocked!(combat_request_lifecycle_closes_a_landed_melee_with_its_contact_and_never_adds_a_second_terminal);
blocked!(
    combat_request_lifecycle_interrupts_a_committed_action_once_and_only_when_it_still_owes_one
);
blocked!(combat_request_lifecycle_hands_the_terminal_to_a_spawned_projectile_instead_of_the_cancelled_action);
blocked!(
    combat_request_lifecycle_round_trips_the_typed_request_and_terminal_rows_through_the_snapshot
);

// -- "deterministic combat resolver" ---------------------------------------

blocked!(deterministic_combat_resolver_uses_exact_integer_phase_windows_and_pays_recovery_and_cooldown_on_misses);
blocked!(deterministic_combat_resolver_raises_and_lowers_guard_without_inventing_a_held_interval_for_a_fast_tap);
blocked!(
    deterministic_combat_resolver_latches_an_early_ranged_release_and_never_auto_fires_a_held_aim
);
blocked!(deterministic_combat_resolver_selects_one_legal_melee_target_by_forward_projection_and_stable_index);
blocked!(deterministic_combat_resolver_lets_a_projectile_pass_a_dodge_and_uses_swept_contact_on_a_later_tick);
blocked!(deterministic_combat_resolver_expires_a_missed_projectile_after_exactly_sixty_deterministic_travel_ticks);
blocked!(deterministic_combat_resolver_blocks_a_frontal_hit_with_one_bounded_recoil_and_leaves_the_ball_owned);
blocked!(
    deterministic_combat_resolver_resolves_simultaneous_trades_and_one_dominant_outcome_per_target
);
blocked!(deterministic_combat_resolver_does_not_invent_a_spill_when_an_unguarded_contact_finds_a_loose_ball);
blocked!(deterministic_combat_resolver_keeps_combat_presentation_free_neutral_fixture_bound_and_snapshot_deferred);
blocked!(deterministic_combat_resolver_caps_a_hit_chain_and_grants_a_full_immunity_window_without_repeat_displacement);
