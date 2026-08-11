//! Tests for `gc_sim::placement`.

use gc_data::formations;
use gc_sim::placement::{self, Field, Side};

const FIELD: Field = Field { w: 960.0, h: 540.0 };

fn formation(id: &str) -> &'static formations::FormationData {
    formations::get(id).expect("known formation")
}

#[test]
fn placement_anchors_produces_a_keeper_plus_four_outfield_anchors() {
    let f = formation("2-1-1");
    assert_eq!(placement::anchors(f, Side::Home, FIELD).len(), 5);
}

#[test]
fn placement_anchors_places_the_home_keeper_left_of_centre_away_keeper_right_of_centre() {
    let f = formation("2-1-1");
    let home = placement::anchors(f, Side::Home, FIELD);
    let away = placement::anchors(f, Side::Away, FIELD);
    assert!(home[0].x < FIELD.w / 2.0);
    assert!(away[0].x > FIELD.w / 2.0);
}

#[test]
fn placement_anchors_mirrors_away_anchors_across_the_vertical_centre_line() {
    let f = formation("2-1-1");
    let home = placement::anchors(f, Side::Home, FIELD);
    let away = placement::anchors(f, Side::Away, FIELD);
    for i in 0..home.len() {
        assert!((home[i].x + away[i].x - FIELD.w).abs() < 1e-6);
        assert!((home[i].y - away[i].y).abs() < 1e-6);
    }
}

#[test]
fn placement_anchors_tags_every_built_in_outfield_slot_with_the_closed_role_contract() {
    use formations::FormationRole::{Def, Fwd, Mid, Wide};
    let expected: &[(&str, [formations::FormationRole; 4])] = &[
        ("2-1-1", [Def, Def, Mid, Fwd]),
        ("1-2-1", [Def, Wide, Wide, Fwd]),
        ("1-1-2", [Def, Mid, Fwd, Fwd]),
    ];
    for (formation_id, roles) in expected {
        let f = formation(formation_id);
        assert_eq!(f.outfield.len(), 4);
        for (ordinal, anchor) in f.outfield.iter().enumerate() {
            assert_eq!(
                anchor.role,
                roles[ordinal],
                "{formation_id} slot {}",
                ordinal + 1
            );
        }
    }
}
