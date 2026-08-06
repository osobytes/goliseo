//! Port of `spec/sim/combat_observation_spec.lua`.
//!
//! Every case in this spec builds its fixture through the Lua helper
//! `new_match()`, which calls `match.new` (`sim/match.lua`). That module is
//! still an unported placeholder (`gc_sim::r#match`), so none of these
//! twelve cases can be expressed yet — each is stubbed here, accurately
//! named and `#[ignore]`d, so the assertion set is tracked rather than
//! silently dropped.

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::match (sim/match.lua), not yet ported"]
        fn $name() {
            unimplemented!("needs sim::match (sim/match.lua)")
        }
    };
}

blocked!(combat_sim_observation_v1_builds_a_valid_observation_whose_digest_covers_its_body);
blocked!(
    combat_sim_observation_v1_orders_teammate_opponent_and_anchor_rows_by_canonical_player_index
);
blocked!(combat_sim_observation_v1_rejects_a_reordered_peer_row_array);
blocked!(combat_sim_observation_v1_rejects_a_row_filed_under_the_wrong_team_array);
// These two additionally test a shape check (missing/undeclared field) that
// is structurally redundant once `CombatObservation` is a typed Rust struct
// — see `combat_observation::validate`'s doc comment — but still cannot be
// exercised without a `sim::match`-built fixture to validate.
blocked!(combat_sim_observation_v1_rejects_a_missing_declared_field);
blocked!(combat_sim_observation_v1_rejects_an_undeclared_field_anywhere_in_the_schema);
blocked!(combat_sim_observation_v1_rejects_a_duplicated_peer_row_and_a_duplicated_projectile_row);
blocked!(combat_sim_observation_v1_orders_projectiles_by_source_sequence_source_player_then_id);
blocked!(combat_sim_observation_v1_gives_every_projectile_a_collision_free_id_from_its_source_and_sequence);
blocked!(
    combat_sim_observation_v1_publishes_the_bounded_public_threat_path_of_a_committed_melee_row
);
blocked!(combat_sim_observation_v1_holds_the_same_sentinel_discipline_on_the_self_row);
blocked!(
    combat_sim_observation_v1_publishes_a_ranged_spawn_tick_only_once_the_release_latch_is_public
);
