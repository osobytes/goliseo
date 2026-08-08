//! Ported from `spec/data/showcase_species_spec.lua`.

use gc_data::species::{Shape, SimVerb};
use gc_data::{players, showcase_player_compatibility, species};

#[test]
fn showcase_species_authors_four_distinct_visual_identities_without_activating_mechanics() {
    let ids = ["terran", "gravling", "voltari", "myceloid"];
    let mut seen_shapes: Vec<Shape> = Vec::new();
    let mut seen_ids: Vec<&str> = Vec::new();

    for id in ids {
        let data = species::get(id).unwrap();
        assert_eq!(data.id, id);
        assert!(!seen_ids.contains(&data.id));
        seen_ids.push(data.id);

        let tagline = data.tagline.unwrap();
        assert!(!tagline.is_empty());

        let palette = data.palette.unwrap();
        for value in palette {
            assert!((0.0..=1.0).contains(&value));
        }

        let shape = data.shape.unwrap();
        assert_eq!(data.verb, SimVerb::None);
        assert_eq!(data.modifiers.pace, 0);
        seen_shapes.push(shape);
    }

    assert_eq!(ids.len(), 4);
    assert!(seen_shapes.contains(&Shape::Round));
    assert!(seen_shapes.contains(&Shape::Broad));
    assert!(seen_shapes.contains(&Shape::Angular));
    assert!(seen_shapes.contains(&Shape::Cluster));
}

#[test]
fn showcase_species_gives_every_player_a_valid_presentation_species_while_simulation_stays_neutral()
{
    let mut seen_presentations: Vec<&str> = Vec::new();
    let mut player_ids: Vec<&str> = Vec::new();

    for player in players::ALL {
        assert!(!player_ids.contains(&player.id));
        player_ids.push(player.id);

        let showcase = showcase_player_compatibility::get(player.id).unwrap();
        assert_eq!(showcase.player_id, player.id);
        assert_eq!(showcase.species, "neutral");

        let presentation = showcase.presentation_species.unwrap();
        assert!(species::get(presentation).is_some());
        if !seen_presentations.contains(&presentation) {
            seen_presentations.push(presentation);
        }

        for value in [
            player.stats.pace,
            player.stats.strength,
            player.stats.technique,
            player.stats.stamina,
            player.stats.mental,
        ] {
            assert!((0..=10).contains(&value));
        }
    }

    for row in showcase_player_compatibility::ALL {
        assert!(
            player_ids.contains(&row.player_id),
            "compatibility row has no persistent player"
        );
    }

    assert!(seen_presentations.contains(&"terran"));
    assert!(seen_presentations.contains(&"gravling"));
    assert!(seen_presentations.contains(&"voltari"));
    assert!(seen_presentations.contains(&"myceloid"));
}
