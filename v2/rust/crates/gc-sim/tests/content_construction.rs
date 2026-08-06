//! Port of `spec/sim/content_construction_spec.lua`.

use gc_data::players::PlayerData;
use gc_data::species::SimVerb;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchPlayer, MatchState, PitchSize};
use indexmap::IndexMap;

fn players_by_id() -> IndexMap<&'static str, PlayerData> {
    gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect()
}

fn new_match(by_id: &IndexMap<&str, PlayerData>) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(71.0),
        players_by_id: Some(by_id),
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

fn match_player<'a>(state: &'a MatchState, id: &str) -> &'a MatchPlayer {
    state
        .players
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("missing match player {id}"))
}

#[test]
fn prototype_content_match_construction_keeps_presentation_cosmetic_and_equipment_swaps_mechanically_inert()
 {
    let baseline_players = players_by_id();
    let mut swapped_players = players_by_id();
    {
        let swapped = swapped_players
            .get_mut("brakka")
            .expect("brakka is an authored player");
        swapped.presentation_id = "scifi_nova_quell";
        swapped.cosmetic_variant_id = Some("nova_cyan");
        swapped.loadout_id = Some("loadout_foam_champion");
    }

    let baseline_state = new_match(&baseline_players);
    let baseline = match_player(&baseline_state, "brakka");
    let changed_state = new_match(&swapped_players);
    let changed = match_player(&changed_state, "brakka");

    // `MatchPlayer` has no `presentation_id`/`loadout_id` field at all (a
    // fixed, compile-time field set), which is the structural form of the
    // Lua spec's `rawget(changed, "presentation_id") == nil` /
    // `rawget(changed, "loadout_id") == nil` assertions: there is no such
    // field to leak into, by construction.
    assert_eq!(
        changed.move_speed, baseline.move_speed,
        "move_speed changed with presentation data"
    );
    assert_eq!(
        changed.shot_speed, baseline.shot_speed,
        "shot_speed changed with presentation data"
    );
    assert_eq!(
        changed.dribble, baseline.dribble,
        "dribble changed with presentation data"
    );
    assert_eq!(
        changed.strength, baseline.strength,
        "strength changed with presentation data"
    );
    assert_eq!(
        changed.first_touch, baseline.first_touch,
        "first_touch changed with presentation data"
    );
    assert_eq!(
        changed.header_skill, baseline.header_skill,
        "header_skill changed with presentation data"
    );
    assert_eq!(
        changed.volley_skill, baseline.volley_skill,
        "volley_skill changed with presentation data"
    );
    assert_eq!(
        changed.bicycle_skill, baseline.bicycle_skill,
        "bicycle_skill changed with presentation data"
    );
    assert_eq!(
        changed.reach, baseline.reach,
        "reach changed with presentation data"
    );
    assert_eq!(
        changed.handling, baseline.handling,
        "handling changed with presentation data"
    );
}

#[test]
fn prototype_content_match_construction_preserves_the_galactic_showcase_species_seam_for_the_existing_fixture()
 {
    let by_id = players_by_id();
    let state = new_match(&by_id);
    assert_eq!(state.players.len(), 10);
    for player in &state.players {
        assert_eq!(player.species_id, "neutral");
        assert_eq!(player.owned_verb, SimVerb::None);
    }
}
