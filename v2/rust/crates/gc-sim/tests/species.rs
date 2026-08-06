//! Port of `spec/sim/species_spec.lua`.
//!
//! The Lua spec's last three cases ("applies the modifier exactly once for
//! controlled and match-AI slots", "makes a pace modifier visible as
//! distance covered in the same match", plus any assertion needing a real
//! `MatchState`) all build a match via `sim.match.new`/`match.step`.
//! `sim/match.lua` is another agent's module and is still an unported
//! placeholder (`gc_sim::r#match`), so those cases are ported as `#[ignore]`
//! below with a note; everything `species.apply` and the neutral verb hooks
//! own directly is ported and passing.

use gc_data::players::StatBlock;
use gc_data::species::{SimVerb, SpeciesData, StatModifier};
use gc_sim::species;

#[test]
fn species_leaves_stats_unchanged_for_the_production_neutral_species() {
    let neutral = gc_data::species::get("neutral").expect("neutral species is authored");
    let authored = StatBlock {
        pace: 4,
        strength: 5,
        technique: 6,
        stamina: 7,
        mental: 8,
    };
    let effective = species::apply(authored, neutral);
    assert_eq!(effective.pace, authored.pace);
    assert_eq!(effective.strength, authored.strength);
    assert_eq!(effective.technique, authored.technique);
    assert_eq!(effective.stamina, authored.stamina);
    assert_eq!(effective.mental, authored.mental);
}

#[test]
fn species_applies_additive_modifiers_deterministically_and_clamps_every_stat_to_0_10() {
    let extreme = SpeciesData {
        id: "extreme_fixture",
        name: "Extreme Fixture",
        modifiers: StatModifier {
            pace: 4,
            strength: -4,
            technique: 2,
            stamina: -2,
            mental: 0,
        },
        verb: SimVerb::None,
        skill: None,
        tagline: None,
        palette: None,
        shape: None,
    };
    let authored = StatBlock {
        pace: 8,
        strength: 2,
        technique: 5,
        stamina: 1,
        mental: 6,
    };
    let first = species::apply(authored, &extreme);
    let second = species::apply(authored, &extreme);

    assert_eq!(first.pace, 10);
    assert_eq!(first.strength, 0);
    assert_eq!(first.technique, 7);
    assert_eq!(first.stamina, 0);
    assert_eq!(first.mental, 6);
    assert_eq!(second, first);
}

#[test]
fn species_keeps_every_owned_verb_hook_neutral_until_signature_skills_bind_it() {
    assert_eq!(species::jump_reach(SimVerb::Jump), 0.0);
    assert_eq!(species::collision_reach(SimVerb::Collision), 0.0);
    assert_eq!(species::burst_speed(SimVerb::Burst), 1.0);
    assert_eq!(species::dribble_protection(SimVerb::Dribble), 0.0);
    assert_eq!(species::block_reach(SimVerb::Block), 0.0);
    assert_eq!(species::link_pass_speed(SimVerb::Link), 1.0);

    assert_eq!(species::jump_reach(SimVerb::None), 0.0);
    assert_eq!(species::collision_reach(SimVerb::None), 0.0);
    assert_eq!(species::burst_speed(SimVerb::None), 1.0);
    assert_eq!(species::dribble_protection(SimVerb::None), 0.0);
    assert_eq!(species::block_reach(SimVerb::None), 0.0);
    assert_eq!(species::link_pass_speed(SimVerb::None), 1.0);
}

/// Blocked on `sim::match` (`sim/match.lua`), an unported placeholder owned
/// by another agent. Unblocks when `gc_sim::r#match::new`/`::step` land.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn species_applies_the_modifier_exactly_once_for_controlled_and_match_ai_slots() {
    unimplemented!("requires sim::match::new/step");
}

/// Blocked on `sim::match`, same as above.
#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn species_makes_a_pace_modifier_visible_as_distance_covered_in_the_same_match() {
    unimplemented!("requires sim::match::new/step");
}
