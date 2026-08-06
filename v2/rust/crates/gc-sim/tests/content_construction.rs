//! Port of `spec/sim/content_construction_spec.lua`.
//!
//! Every case in this file builds a real `MatchState` via
//! `sim.match.new`/`match.step` and inspects the resulting `MatchPlayer`
//! records. `sim/match.lua` is another agent's module and is still an
//! unported placeholder (`gc_sim::r#match`), so this whole file is blocked
//! and ported as `#[ignore]` cases below with a note; there is nothing in
//! `content_construction_spec.lua` that exercises a module this agent owns
//! independent of `sim::match`.

/// Blocked on `sim::match` (`sim/match.lua`), an unported placeholder owned
/// by another agent.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn prototype_content_match_construction_keeps_presentation_cosmetic_and_equipment_swaps_mechanically_inert()
 {
    unimplemented!("requires sim::match::new/step");
}

/// Blocked on `sim::match`, same as above.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn prototype_content_match_construction_preserves_the_galactic_showcase_species_seam_for_the_existing_fixture()
 {
    unimplemented!("requires sim::match::new/step");
}
