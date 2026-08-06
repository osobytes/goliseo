//! Port of `spec/game/online_match_driver_spec.lua`.
//!
//! ## Why every assertion below is `#[ignore]`d
//!
//! Every one of the spec's 53 assertions (46 static `t.it` cases plus one
//! `t.it` inside a loop over `combat_phases.PHASES`'s 7 named phases) is
//! built through the spec's own `harness(mode, options)` helper
//! (`spec/game/online_match_driver_spec.lua:34-82`), which calls
//! `fixture.session(mode, ...)`. That, in turn, needs `coordinator.lua`
//! (`plan_assignments`, `slot_sources`), `protocol.lua` (`match_mode`,
//! `owned_slots`, `manifest_id`, `assignment_id`), `protocol_fixture.lua`,
//! and `game/transport/fake_star.lua`.
//!
//! As of this port: `coordinator.rs`, `protocol.rs`, and `protocol_fixture.rs`
//! are `NOT YET PORTED` placeholders in this crate, owned by concurrent
//! agents (per this file's brief, out of scope here); `game/transport/**` is
//! permanently TypeScript-owned (`v2/README.md` §2) with no Rust port
//! planned. So `harness(...)` cannot be built in Rust today, and neither can
//! anything the spec asserts through it.
//!
//! `crate::match_driver` itself is not idle while this is blocked: every
//! pure helper it exposes (tick-space math, `owned_slots`, the trivial
//! `live_slot` wrappers, `derived_ai_seed`) has its own unit tests in
//! `crates/gc-netcode/src/match_driver.rs`'s `#[cfg(test)]` module, and the
//! **required differential test** against real Lua
//! (see `crate::match_driver`'s module doc and this crate's report) captures
//! a reference trace from the real Lua `match_driver.lua` driving a scripted
//! sequence with a late input forcing a rollback, committed under
//! `tests/fixtures/match_driver_lua_reference.txt`. The Rust-side half of
//! that comparison is blocked for the same reason every case below is: there
//! is no way to construct a `MatchDriver` end to end without `coordinator.rs`
//! and `protocol.rs`. [`match_driver_differential_matches_the_lua_reference`]
//! is the placeholder for that comparison, `#[ignore]`d naming the same
//! blocker, with the captured trace already committed so re-enabling it is a
//! rewiring, not a rerun.

const BLOCKED: &str = "blocked on fixture.session's dependencies: crate::coordinator, \
    crate::protocol, and crate::protocol_fixture are NOT YET PORTED placeholders in this \
    crate (owned by concurrent agents), and game/transport/fake_star.lua is permanently \
    TypeScript-owned (v2/README.md §2) with no Rust port planned. See crate::match_driver_fixture's \
    module doc.";

#[test]
#[ignore = "blocked on the real Lua match_driver.lua reference trace's Rust-side comparison: \
    building a MatchDriver end to end needs crate::coordinator and crate::protocol, both \
    NOT YET PORTED placeholders. The Lua-side reference is captured and committed at \
    tests/fixtures/match_driver_lua_reference.txt; re-enable this once those modules land."]
fn match_driver_differential_matches_the_lua_reference() {}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn seats_a_4v4_host_on_one_slot_and_authors_every_bot_fill() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn gives_a_1v1_human_four_owned_slots_and_one_control_slot() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn makes_the_host_author_every_declared_bot_fill_in_a_short_lobby() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn converges_every_peer_on_the_same_confirmed_boundary_hashes_in_4v4() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_1v1() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_2v2() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn moves_the_control_slot_only_through_the_canonical_stream() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn applies_one_transport_tick_arrival_batch_as_one_reconciliation() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn is_insensitive_to_the_order_peers_are_polled_and_stepped() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn hands_control_channel_traffic_back_instead_of_eating_it() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn keeps_protected_keepers_ai_only_and_slotless() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn refuses_authority_from_outside_a_peers_frozen_owned_set() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn is_idempotent_for_a_replayed_bundle_and_terminal_for_a_conflicting_one() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn accepts_every_slot_inside_a_peers_frozen_owned_set() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn stops_all_progress_after_a_terminal_status() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn tolerates_one_boundary_disagreement_and_clears_it_on_the_next_agreement() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn publishes_confirmed_boundary_hashes_on_the_documented_interval() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn ends_with_a_typed_completed_status_at_full_time() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn converges_under_impaired_delivery_after_real_corrections() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn converges_in_1v1_under_impaired_delivery_with_live_slot_switching() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn terminates_explicitly_when_authority_falls_outside_the_retained_window() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn publishes_checkpoints_on_the_simulated_ceiling_not_raw_confirmation() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn never_publishes_a_checkpoint_boundary_that_was_not_simulated() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn terminates_on_confirmation_liveness_before_a_below_floor_row_can_arrive() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn agrees_on_the_live_slot_in_2v2_under_impaired_delivery_with_switching() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn reaches_full_time_under_impaired_delivery() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn carries_the_combat_companion_through_correction_and_resimulation() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn opens_every_combat_phase_scenario_from_a_ready_combat_state() {
    unreachable!("{BLOCKED}");
}

/// The spec's `for _, phase_id in ipairs(combat_phases.PHASES) do t.it(...) end`
/// loop (`spec/game/online_match_driver_spec.lua:867`) produces one case per
/// named combat correction phase; `crate::fault_harness::declare_contingent`
/// names the same 7 phases. Ported as 7 distinctly named cases rather than
/// one generic loop, so each is independently countable and independently
/// re-enableable.
mod converges_a_correction_taken_during_each_combat_phase {
    const REASON: &str = "blocked on fixture.session and spec/support/online_combat_phases.lua's \
        fixtures; see the parent module doc";

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn wind_up() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn guard() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn contact() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn projectile_flight() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn stagger() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn ball_spill() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn immunity_expiry() {
        unreachable!("{REASON}");
    }
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn finds_no_driver_level_geometry_where_the_policy_guards_often_enough() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn still_reconciles_if_a_local_insert_ever_reports_a_divergence() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn costs_no_extra_snapshot_work_when_a_peer_authors_only_its_control_slot() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn settles_the_final_boundary_before_completing_under_clean_delivery() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn completes_with_an_agreed_final_hash_under_a_burst_across_full_time() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn does_not_swallow_a_boundary_disagreement_reported_while_settling() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn settles_a_genuinely_divergent_peer_without_hiding_the_divergence() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn ends_a_settle_nobody_can_finish_with_a_bounded_typed_reason() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn bounds_the_settle_phase_in_wall_clock_as_well_as_in_ticks() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn re_publishes_the_tail_while_settling_and_simulates_nothing() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn reports_a_stalled_confirmation_at_the_step_it_becomes_permanent() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn maps_a_rejected_over_window_batch_onto_late_input_unreachable_by_design() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn fans_out_a_full_redundancy_window_for_a_slot_that_sent_nothing() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn keeps_the_host_relaying_until_its_guests_have_stopped_asking() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn keeps_relaying_for_a_peer_that_reported_behind_and_then_went_silent() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn repairs_a_guest_whose_hole_has_aged_out_of_the_redundancy_window() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn keeps_a_guest_re_publishing_until_the_host_has_confirmed_its_tail() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "blocked on fixture.session; see BLOCKED"]
fn reports_a_lost_transport_as_a_typed_terminal_status() {
    unreachable!("{BLOCKED}");
}
