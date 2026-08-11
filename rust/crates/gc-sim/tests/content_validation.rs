//! Tests for `gc_sim::content_validation`.
//!
//! `validate_catalog`/`validate_fixture` use `assert!` to reject invalid
//! authored content (AGENTS.md §7 — authored content is a
//! programmer-controlled invariant, not a recoverable runtime condition), so
//! each rejection case below wraps the call in `std::panic::catch_unwind`.
//!
//! One sub-case of "rejects out-of-bounds family data and presentation-owned
//! mechanics" is deliberately not covered: adding an undeclared field (e.g.
//! a stray `pace` field) to a presentation record, to check that
//! `validate_catalog` rejects the resulting malformed data. A typed Rust
//! `struct` cannot carry an undeclared extra field at all — the check that
//! case would exercise is structurally impossible to violate here, not
//! merely already-passing — so only that one sub-case is omitted; the
//! file's other two sub-cases (`long_interrupt`, `invalid_arc`) are covered
//! below and pass.

use gc_data::action_families::ActionFamilyId;
use gc_data::{
    action_families, character_presentations, cosmetic_variants, equipment_presentations, loadouts,
    players, teams,
};
use gc_sim::content_validation::{self, PrototypeContentCatalog, PrototypeFixturePolicy, Team};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn catalog() -> PrototypeContentCatalog {
    PrototypeContentCatalog {
        character_presentations: character_presentations::ALL
            .iter()
            .map(|p| (p.id, *p))
            .collect(),
        cosmetic_variants: cosmetic_variants::ALL.iter().map(|c| (c.id, *c)).collect(),
        action_families: action_families::ALL.iter().map(|f| (f.id, *f)).collect(),
        equipment_presentations: equipment_presentations::ALL
            .iter()
            .map(|e| (e.id, *e))
            .collect(),
        loadouts: loadouts::ALL.iter().map(|l| (l.id, *l)).collect(),
        players: players::ALL.to_vec(),
    }
}

fn player_mut<'a>(values: &'a mut [players::PlayerData], id: &str) -> &'a mut players::PlayerData {
    values
        .iter_mut()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("missing player {id}"))
}

fn nebula() -> Team {
    Team::from(teams::get("nebula").expect("nebula team is authored"))
}

fn orion() -> Team {
    Team::from(teams::get("orion").expect("orion team is authored"))
}

fn fixture_is_valid(
    fixture: &PrototypeContentCatalog,
    home: &Team,
    away: &Team,
    policy: Option<PrototypeFixturePolicy>,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        content_validation::validate_fixture(fixture, home, away, policy)
    }))
    .is_ok()
}

fn fails<F: FnOnce()>(f: F) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

#[test]
fn prototype_content_validation_accepts_the_authored_catalog_and_one_of_each_default_fixture() {
    let fixture = catalog();
    assert!(content_validation::validate_catalog(&fixture));
    assert!(fixture_is_valid(&fixture, &nebula(), &orion(), None));
}

#[test]
fn prototype_content_validation_rejects_unknown_ids_and_family_presentation_mismatches() {
    let mut unknown_presentation = catalog();
    player_mut(&mut unknown_presentation.players, "zyro_vex").presentation_id = "missing";
    assert!(fails(|| {
        content_validation::validate_catalog(&unknown_presentation);
    }));

    let mut unknown_loadout = catalog();
    player_mut(&mut unknown_loadout.players, "zyro_vex").loadout_id = Some("missing");
    assert!(fails(|| {
        content_validation::validate_catalog(&unknown_loadout);
    }));

    let mut mismatch = catalog();
    mismatch
        .loadouts
        .get_mut("loadout_vector_blade")
        .unwrap()
        .family_id = ActionFamilyId::Ranged;
    assert!(fails(|| {
        content_validation::validate_catalog(&mismatch);
    }));
}

#[test]
fn prototype_content_validation_rejects_duplicate_player_ids_and_prohibited_team_numbers() {
    let mut duplicate_id = catalog();
    let first_id = duplicate_id.players[0].id;
    duplicate_id.players[1].id = first_id;
    assert!(fails(|| {
        content_validation::validate_catalog(&duplicate_id);
    }));

    let mut duplicate_number = catalog();
    let veil_number = player_mut(&mut duplicate_number.players, "veil_nyx").number;
    player_mut(&mut duplicate_number.players, "brakka").number = veil_number;
    assert!(!fixture_is_valid(
        &duplicate_number,
        &nebula(),
        &orion(),
        None
    ));
}

#[test]
fn prototype_content_validation_rejects_illegal_keeper_and_outfielder_loadouts() {
    let mut armed_keeper = catalog();
    player_mut(&mut armed_keeper.players, "ozzo").loadout_id = Some("loadout_spring_gloves");
    assert!(fails(|| {
        content_validation::validate_catalog(&armed_keeper);
    }));

    let mut empty_outfielder = catalog();
    player_mut(&mut empty_outfielder.players, "zyro_vex").loadout_id = None;
    assert!(fails(|| {
        content_validation::validate_catalog(&empty_outfielder);
    }));
}

#[test]
fn prototype_content_validation_rejects_wrong_team_size_unknown_roster_ids_and_repeated_default_families()
 {
    let mut short_home = nebula();
    short_home.roster.pop();
    assert!(!fixture_is_valid(&catalog(), &short_home, &orion(), None));

    let mut unknown_home = nebula();
    unknown_home.roster[4] = "missing";
    assert!(!fixture_is_valid(&catalog(), &unknown_home, &orion(), None));

    let mut repeated = catalog();
    player_mut(&mut repeated.players, "veil_nyx").loadout_id = Some("loadout_emberguard_shield");
    assert!(!fixture_is_valid(&repeated, &nebula(), &orion(), None));
    assert!(fixture_is_valid(
        &repeated,
        &nebula(),
        &orion(),
        Some(PrototypeFixturePolicy {
            allow_repeated_families: true
        })
    ));
}

#[test]
fn prototype_content_validation_rejects_sparse_squads_and_players_eligible_for_both_teams() {
    // A Rust `Vec` cannot hold a hole (a missing middle index while a later
    // index stays present) — removing an element always yields a shorter,
    // still dense, sequence. The assertion this case is really pinning is
    // "a squad that stops covering a roster starter is rejected", so it
    // drops a STARTER from the squad directly, which is a strictly stronger
    // check of the same invariant than removing an arbitrary bench player
    // would be.
    let mut sparse_home = nebula();
    let starter = sparse_home.roster[4]; // "zyro_vex", a nebula starter
    sparse_home
        .squad
        .as_mut()
        .expect("nebula has an explicit squad")
        .retain(|&id| id != starter);
    assert!(!fixture_is_valid(&catalog(), &sparse_home, &orion(), None));

    let mut shared_home = nebula();
    let orion_player = orion().roster[0];
    shared_home
        .squad
        .as_mut()
        .expect("nebula has an explicit squad")
        .push(orion_player);
    assert!(!fixture_is_valid(&catalog(), &shared_home, &orion(), None));
}

#[test]
fn prototype_content_validation_rejects_out_of_bounds_family_data_and_presentation_owned_mechanics()
{
    let mut long_interrupt = catalog();
    {
        let (_, family) = long_interrupt
            .action_families
            .iter_mut()
            .find(|(id, _)| *id == ActionFamilyId::LightMelee)
            .unwrap();
        family
            .unguarded_outcome
            .as_mut()
            .unwrap()
            .interruption_ticks = 31;
    }
    assert!(fails(|| {
        content_validation::validate_catalog(&long_interrupt);
    }));

    let mut invalid_arc = catalog();
    {
        let (_, family) = invalid_arc
            .action_families
            .iter_mut()
            .find(|(id, _)| *id == ActionFamilyId::Ranged)
            .unwrap();
        family.front_arc_degrees = 181.0;
    }
    assert!(fails(|| {
        content_validation::validate_catalog(&invalid_arc);
    }));

    // The undeclared-field sub-case (`hidden_stat`) has no Rust equivalent
    // — see the module doc above.
}
