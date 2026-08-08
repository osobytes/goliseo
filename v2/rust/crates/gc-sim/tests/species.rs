//! Port of `spec/sim/species_spec.lua`.
//!
//! The Lua spec's last two cases ("applies the modifier exactly once for
//! controlled and match-AI slots", "makes a pace modifier visible as
//! distance covered in the same match") build a match via
//! `sim.match.new`/`match.step`. `sim::match` (`gc_sim::r#match`) is now
//! fully ported, so both build a real fixture via `sim_match::new`.

use gc_data::players::{PlayerData, StatBlock};
use gc_data::showcase_player_compatibility::{self, ShowcasePlayerCompatibilityData};
use gc_data::species::{self as species_data, SimVerb, SpeciesData, StatModifier};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchInput, MatchPlayer, MatchState, PitchSize};
use gc_sim::species;
use gc_sim::stats;
use gc_sim::tuning::Tuning;
use indexmap::IndexMap;

const SWIFT: SpeciesData = SpeciesData {
    id: "swift_fixture",
    name: "Swift Fixture",
    modifiers: StatModifier {
        pace: 1,
        strength: 0,
        technique: 0,
        stamina: 0,
        mental: 0,
    },
    verb: SimVerb::Burst,
    skill: None,
    tagline: None,
    palette: None,
    shape: None,
};

fn test_species() -> IndexMap<&'static str, SpeciesData> {
    let mut m = IndexMap::new();
    m.insert(
        "neutral",
        *species_data::get("neutral").expect("neutral species is authored"),
    );
    m.insert("swift_fixture", SWIFT);
    m
}

fn players_by_id() -> IndexMap<&'static str, PlayerData> {
    gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect()
}

/// `overrides` pairs a player id with a species id to substitute for that
/// player's showcase `species` field.
fn showcase_by_id(
    overrides: &[(&str, &'static str)],
) -> IndexMap<&'static str, ShowcasePlayerCompatibilityData> {
    showcase_player_compatibility::ALL
        .iter()
        .map(|row| {
            let mut row = *row;
            if let Some(&(_, species)) = overrides.iter().find(|(id, _)| *id == row.player_id) {
                row.species = species;
            }
            (row.player_id, row)
        })
        .collect()
}

fn new_match(
    by_id: &IndexMap<&str, PlayerData>,
    showcase: &IndexMap<&str, ShowcasePlayerCompatibilityData>,
) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let species_pool = test_species();
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(30.0),
        max_goals: None,
        seed: Some(11.0),
        players_by_id: Some(by_id),
        species_by_id: Some(&species_pool),
        showcase_players_by_id: Some(showcase),
        human_controlled: None,
        input_ownership: None,
    })
}

fn player_named<'a>(players: &'a [MatchPlayer], id: &str) -> &'a MatchPlayer {
    players
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("missing match player: {id}"))
}

fn move_right() -> MatchInput {
    MatchInput {
        r#move: gc_core::vec2::Vec2::new(1.0, 0.0),
        ..MatchInput::default()
    }
}

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

#[test]
fn species_applies_the_modifier_exactly_once_for_controlled_and_match_ai_slots() {
    let neutral_by_id = players_by_id();
    let neutral_showcase = showcase_by_id(&[]);
    let neutral_match = new_match(&neutral_by_id, &neutral_showcase);
    let controlled_id = neutral_match.players[(neutral_match.controlled - 1) as usize]
        .id
        .clone();

    let swift_by_id = players_by_id();
    let swift_showcase =
        showcase_by_id(&[(controlled_id.as_str(), SWIFT.id), ("tox_vren", SWIFT.id)]);
    let swift_match = new_match(&swift_by_id, &swift_showcase);

    let neutral_home = player_named(&neutral_match.players, &controlled_id);
    let swift_home = player_named(&swift_match.players, &controlled_id);
    let neutral_away = player_named(&neutral_match.players, "tox_vren");
    let swift_away = player_named(&swift_match.players, "tox_vren");

    assert_eq!(
        swift_match.players[(swift_match.controlled - 1) as usize].id,
        controlled_id
    );
    assert_eq!(
        swift_home.move_speed,
        stats::move_speed(species::apply(
            swift_by_id[controlled_id.as_str()].stats,
            &SWIFT
        ))
    );
    assert_eq!(
        swift_away.move_speed,
        stats::move_speed(species::apply(swift_by_id["tox_vren"].stats, &SWIFT))
    );
    assert_eq!(
        swift_home.move_speed - neutral_home.move_speed,
        20.0,
        "controlled slot gets one point"
    );
    assert_eq!(
        swift_away.move_speed - neutral_away.move_speed,
        20.0,
        "AI slot gets one point"
    );
    assert_eq!(swift_home.species_id, SWIFT.id);
    assert_eq!(swift_home.owned_verb, SimVerb::Burst);
}

#[test]
fn species_makes_a_pace_modifier_visible_as_distance_covered_in_the_same_match() {
    let tune = Tuning::new();
    let neutral_showcase = showcase_by_id(&[]);
    let neutral_match_by_id = players_by_id();
    let mut neutral_match = new_match(&neutral_match_by_id, &neutral_showcase);
    let controlled_id = neutral_match.players[(neutral_match.controlled - 1) as usize]
        .id
        .clone();

    let swift_showcase = showcase_by_id(&[(controlled_id.as_str(), SWIFT.id)]);
    let swift_match_by_id = players_by_id();
    let mut swift_match = new_match(&swift_match_by_id, &swift_showcase);

    let neutral_idx = neutral_match
        .players
        .iter()
        .position(|p| p.id == controlled_id)
        .expect("controlled player exists");
    let swift_idx = swift_match
        .players
        .iter()
        .position(|p| p.id == controlled_id)
        .expect("controlled player exists");
    let neutral_start = neutral_match.players[neutral_idx].pos.x;
    let swift_start = swift_match.players[swift_idx].pos.x;

    for _ in 0..60 {
        sim_match::step(
            &mut neutral_match,
            1.0 / 60.0,
            sim_match::StepInput::Legacy(move_right()),
            None,
            &tune,
        );
        sim_match::step(
            &mut swift_match,
            1.0 / 60.0,
            sim_match::StepInput::Legacy(move_right()),
            None,
            &tune,
        );
    }

    let neutral_distance = neutral_match.players[neutral_idx].pos.x - neutral_start;
    let swift_distance = swift_match.players[swift_idx].pos.x - swift_start;
    assert!(
        swift_distance > neutral_distance,
        "the faster player covers more ground"
    );
}
