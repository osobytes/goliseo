//! Port of `spec/render/player_identity_spec.lua`.
//!
//! The Lua spec has two `t.it` cases. Only the first belongs here: the second
//! ("keeps the four silhouettes geometrically distinct") exercises
//! `game/render/player_renderer.lua`'s `silhouette`, the procedural billboard
//! avatar's own geometry. That module was never carried into this port: #415
//! deleted `@gc/render`'s `player_renderer.ts` and its spec, because the rigged
//! `THREE.SkinnedMesh` path is the only way v2 ever draws a player. So the
//! second case has no counterpart on either side of the port, and the silhouette
//! shapes it pinned are now the rig's concern (`@gc/render`'s
//! `rig3d/species_presentation.ts`, which has its own spec). Recorded here
//! rather than dropped silently.

use gc_data::species::Shape;
use gc_render::identity;

#[test]
fn pitch_presentation_identity_resolves_every_authored_player_without_changing_mechanical_species()
{
    let mut seen: Vec<Shape> = Vec::new();

    for player in gc_data::players::ALL {
        let presentation = identity::for_player(player.id)
            .unwrap_or_else(|| panic!("no presentation identity for {}", player.id));

        // The load-bearing assertion: presentation may swap a player's *look*,
        // but the mechanical species stays neutral. A presentation change that
        // moved stats would show up right here.
        let showcase = gc_data::showcase_player_compatibility::get(player.id)
            .unwrap_or_else(|| panic!("no showcase row for {}", player.id));
        assert_eq!(showcase.species, "neutral");

        // `palette` is a fixed [f64; 3] in Rust, so the Lua's "#palette == 3"
        // check is a type-level guarantee here rather than a runtime one. Assert
        // the values are usable colours instead, which is strictly stronger.
        for channel in presentation.palette {
            assert!(
                (0.0..=1.0).contains(&channel),
                "palette channel out of range for {}: {channel}",
                player.id
            );
        }

        if !seen.contains(&presentation.shape) {
            seen.push(presentation.shape);
        }
    }

    for shape in [Shape::Round, Shape::Broad, Shape::Angular, Shape::Cluster] {
        assert!(seen.contains(&shape), "missing pitch silhouette {shape:?}");
    }
}

#[test]
fn pitch_presentation_identity_returns_none_for_an_unknown_player() {
    // The Lua returns nil on three distinct misses; this is the reachable one
    // without authoring broken content.
    assert!(identity::for_player("no_such_player").is_none());
}
