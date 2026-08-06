//! Port of `spec/sim/match_snapshot_spec.lua`.
//!
//! Every case in this spec builds its fixture through the Lua helpers
//! `new_state()` / `new_ai_state()` / `new_attacking_ai_state()`, which call
//! `match.new` (`sim/match.lua`). That module is still an unported
//! placeholder (`gc_sim::r#match`), so none of these twenty-six cases can be
//! expressed yet — each is stubbed here, accurately named and `#[ignore]`d,
//! so the assertion set is tracked rather than silently dropped.
//!
//! The determinism-critical part of this module — canonical scalar
//! encoding, the wire format, and the FNV-1a-64 hash, exercised across a
//! soccer-only and a combat-active snapshot — is differential-tested
//! against the real Lua implementation separately, in
//! `match_snapshot_differential.rs`; that coverage does not depend on
//! `sim::match` and is not duplicated here.

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::match (sim/match.lua), not yet ported"]
        fn $name() {
            unimplemented!("needs sim::match (sim/match.lua)")
        }
    };
}

blocked!(canonical_match_snapshots_pins_matchstate_and_matchplayer_additions_to_explicit_versioned_allowlists);
blocked!(canonical_match_snapshots_captures_combat_as_one_owned_canonical_versioned_boundary);
blocked!(
    canonical_match_snapshots_rejects_holes_in_authoritative_combat_projectile_and_event_arrays
);
blocked!(canonical_match_snapshots_replays_combat_phase_boundaries_exactly_after_restore);
blocked!(canonical_match_snapshots_persists_active_ai_runs_through_a_v10_soccer_boundary_and_continuation);
blocked!(canonical_match_snapshots_persists_active_ai_runs_through_a_v11_combat_boundary_and_continuation);
blocked!(canonical_match_snapshots_rejects_a_combat_blocked_active_run_as_a_malformed_v11_boundary);
blocked!(canonical_match_snapshots_captures_and_restores_every_nested_payload_as_independent_state);
blocked!(canonical_match_snapshots_keeps_trusted_rollback_copies_exact_and_independently_owned);
blocked!(canonical_match_snapshots_guards_the_shallow_trusted_copy_ownership_contract);
blocked!(
    canonical_match_snapshots_canonically_restores_a_v10_keeper_state_through_goal_and_kickoff
);
blocked!(canonical_match_snapshots_converges_snapshot_advance_restore_and_replay_at_every_boundary);
blocked!(canonical_match_snapshots_serializes_independent_of_table_insertion_order);
blocked!(
    canonical_match_snapshots_compares_owned_canonical_snapshots_without_normalizing_them_again
);
blocked!(canonical_match_snapshots_compares_every_canonical_windup_shot_field);
blocked!(canonical_match_snapshots_compares_every_canonical_outfield_decision_field);
blocked!(canonical_match_snapshots_restores_and_diffs_both_authoritative_formation_identities);
blocked!(canonical_match_snapshots_restores_and_hashes_team_press_state_across_soccer_and_combat_boundaries);
blocked!(canonical_match_snapshots_diffs_and_strictly_validates_nested_press_state_relations);
blocked!(canonical_match_snapshots_rejects_malformed_v10_and_v11_decision_contracts_during_restore);
blocked!(canonical_match_snapshots_encodes_decision_children_positionally_with_exact_no_run_v11_arithmetic);
blocked!(canonical_match_snapshots_prices_four_hypothetical_runs_on_the_valid_high_overhead_slot_mode_base);
blocked!(
    canonical_match_snapshots_prices_the_worst_case_combat_event_row_against_the_retained_window
);
blocked!(canonical_match_snapshots_rejects_unhandled_state_and_player_fields);
blocked!(
    canonical_match_snapshots_rejects_the_prior_snapshot_schema_instead_of_inventing_keeper_state
);
blocked!(canonical_match_snapshots_uses_exact_canonical_finite_number_spelling);
