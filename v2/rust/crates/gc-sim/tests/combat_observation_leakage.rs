//! Port of `spec/sim/combat_observation_leakage_spec.lua`.
//!
//! Every case in this spec builds its fixture through the Lua helper
//! `new_match()`, which calls `match.new` (`sim/match.lua`); one case also
//! calls `match.step`. That module is still an unported placeholder
//! (`gc_sim::r#match`), so none of these six cases can be expressed yet —
//! each is stubbed here, accurately named and `#[ignore]`d, so the
//! assertion set is tracked rather than silently dropped.

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::match (sim/match.lua), not yet ported"]
        fn $name() {
            unimplemented!("needs sim::match (sim/match.lua)")
        }
    };
}

blocked!(combat_observation_leakage_carries_no_presentation_hidden_or_authority_key);
blocked!(
    combat_observation_leakage_positive_control_the_same_scanner_rejects_the_authoritative_snapshot
);
blocked!(combat_observation_leakage_carries_no_presentation_or_loadout_string_value);
blocked!(combat_observation_leakage_carries_no_rng_state_and_no_resolver_verdict);
blocked!(combat_observation_leakage_produces_an_identical_digest_for_a_presentation_swap_inside_one_family);
blocked!(
    combat_observation_leakage_keeps_ai_decisions_and_materialized_tapes_identical_across_that_swap
);
