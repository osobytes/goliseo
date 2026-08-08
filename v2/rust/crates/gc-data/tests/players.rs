//! Ported from `spec/data/players_spec.lua`.

use gc_data::players::{self, StatBlock};

#[test]
fn player_data_authors_exactly_the_five_canonical_attributes() {
    // Lua checks a dynamic table has exactly the five canonical stat keys, all
    // numeric. Here that invariant is structural: `StatBlock` has exactly five
    // `i64` fields and nothing else could compile. This test still exercises
    // every field on every player so the intent stays visible and the check
    // cannot silently stop compiling against `StatBlock`.
    for player in players::ALL {
        let StatBlock {
            pace,
            strength,
            technique,
            stamina,
            mental,
        } = player.stats;
        for value in [pace, strength, technique, stamina, mental] {
            assert!(
                (0..=10).contains(&value),
                "{}: stat {value} out of range",
                player.id
            );
        }
    }
}

#[test]
fn player_data_keeps_persistent_identity_separate_from_presentation_and_loadout_ids() {
    let mut seen_ids: Vec<&str> = Vec::new();
    for player in players::ALL {
        assert!(
            !seen_ids.contains(&player.id),
            "duplicate player id {}",
            player.id
        );
        seen_ids.push(player.id);

        assert!(
            (1..=99).contains(&player.number),
            "{} has an out-of-range shirt number",
            player.id
        );
        assert!(!player.presentation_id.is_empty());

        match player.position {
            players::Position::Keeper => {
                assert_eq!(
                    player.loadout_id, None,
                    "{} keeper has no combat loadout",
                    player.id
                );
            }
            _ => {
                assert!(
                    player.loadout_id.is_some_and(|id| !id.is_empty()),
                    "{} non-keeper must have a combat loadout",
                    player.id
                );
            }
        }

        // `PlayerData` has no `species` or `presentation_species` field at all:
        // mechanical species moved to `showcase_player_compatibility`. The Lua
        // spec asserts `rawget(player, "species") == nil`; here that is
        // enforced at compile time by the struct's shape.
    }
}
