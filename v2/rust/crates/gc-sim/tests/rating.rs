//! Port of `spec/sim/rating_spec.lua`.

use gc_data::players::{PlayerData, Position, StatBlock};
use indexmap::IndexMap;

fn player(id: &'static str, position: Position, value: i64) -> PlayerData {
    PlayerData {
        id,
        name: id,
        number: 1,
        position,
        stats: StatBlock {
            pace: value,
            strength: value,
            technique: value,
            stamina: value,
            mental: value,
        },
        presentation_id: "test",
        cosmetic_variant_id: None,
        loadout_id: if position == Position::Keeper {
            None
        } else {
            Some("test")
        },
    }
}

fn squad(value: i64) -> (Vec<&'static str>, IndexMap<&'static str, PlayerData>) {
    let roster = vec!["keeper", "defender", "midfielder", "forward_a", "forward_b"];
    let mut by_id = IndexMap::new();
    by_id.insert("keeper", player("keeper", Position::Keeper, value));
    by_id.insert("defender", player("defender", Position::Defender, value));
    by_id.insert(
        "midfielder",
        player("midfielder", Position::Midfielder, value),
    );
    by_id.insert("forward_a", player("forward_a", Position::Forward, value));
    by_id.insert("forward_b", player("forward_b", Position::Forward, value));
    (roster, by_id)
}

#[test]
fn rating_squad_strictly_stronger_starters_out_rate_weaker_starters() {
    let (weak_roster, weak) = squad(2);
    let (strong_roster, strong) = squad(8);
    assert!(
        gc_sim::rating::squad(&strong_roster, &strong) > gc_sim::rating::squad(&weak_roster, &weak)
    );
}

#[test]
fn rating_squad_pins_the_frozen_red_team_11_weights() {
    let (roster, mut by_id) = squad(0);
    for (_, starter) in by_id.iter_mut() {
        starter.stats = StatBlock {
            pace: 1,
            strength: 2,
            technique: 3,
            stamina: 4,
            mental: 5,
        };
    }
    let value = gc_sim::rating::squad(&roster, &by_id);
    assert!(
        (value - 13.95).abs() < 1e-12,
        "expected ~13.95, got {value}"
    );
}

#[test]
fn rating_squad_is_invariant_to_roster_order_including_where_the_keeper_id_appears() {
    let (roster, by_id) = squad(6);
    let shuffled = vec![roster[3], roster[1], roster[4], roster[0], roster[2]];
    let a = gc_sim::rating::squad(&roster, &by_id);
    let b = gc_sim::rating::squad(&shuffled, &by_id);
    assert!((a - b).abs() < 1e-12);
}

#[test]
fn rating_squad_uses_goalkeeper_identity_and_authored_position_not_an_array_slot() {
    let (roster, mut by_id) = squad(5);
    by_id.insert(
        "reserve_keeper",
        player("reserve_keeper", Position::Keeper, 9),
    );
    let upgraded = vec!["reserve_keeper", roster[1], roster[2], roster[3], roster[4]];
    assert!(gc_sim::rating::squad(&upgraded, &by_id) > gc_sim::rating::squad(&roster, &by_id));
}

#[test]
fn rating_squad_is_deterministic() {
    let (roster, by_id) = squad(7);
    let first = gc_sim::rating::squad(&roster, &by_id);
    assert_eq!(gc_sim::rating::squad(&roster, &by_id), first);
    assert_eq!(gc_sim::rating::squad(&roster, &by_id), first);
}
