//! Port of `spec/sim/aerial_spec.lua`.
//!
//! Every case in this Lua spec builds a real fixture via `match.new` and
//! drives it with `match.step` — it is testing the integration between
//! `sim/match.lua`'s tick (ball/player physics, possession) and
//! `sim/aerial.lua`'s contact resolution, not `aerial` in isolation.
//! `sim/match.lua` is another agent's module and is still an unported
//! placeholder (`gc_sim::r#match`), so all three cases are ported as
//! `#[ignore]` below with a note, matching the precedent in
//! `tests/species.rs`. `aerial`'s own resolver logic is fully ported and
//! tested in `tests/aerial_resolver.rs` (from `aerial_resolver_spec.lua`).

/// Blocked on `sim::match` (`sim/match.lua`), an unported placeholder owned
/// by another agent. Unblocks when `gc_sim::r#match::new`/`::step` land.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn lets_a_human_meet_a_dropping_cross_for_a_volley_toward_goal() {
    unimplemented!("requires sim::match::new/step");
}

/// Blocked on `sim::match`, same as above.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn connects_with_the_generous_assist_reach_not_just_point_blank() {
    unimplemented!("requires sim::match::new/step");
}

/// Blocked on `sim::match`, same as above.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn hands_control_to_the_best_placed_attacker_as_a_cross_flies_in() {
    unimplemented!("requires sim::match::new/step");
}
