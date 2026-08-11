//! Tests prototype content invariants: the accepted budget, the initial
//! family records, mechanical-identity resolution across themed variants,
//! and starter/presentation counts.

use gc_data::action_families::{
    self, ActionActivation, ActionContactKind, ActionFamilyId, CombatOutcomeData,
};
use gc_data::{
    character_presentations, cosmetic_variants, equipment_presentations, loadouts, players, teams,
};

#[test]
fn prototype_content_authors_the_accepted_budget() {
    assert_eq!(character_presentations::ALL.len(), 6);
    assert_eq!(equipment_presentations::ALL.len(), 6);
    assert_eq!(action_families::ALL.len(), 4);
    assert_eq!(loadouts::ALL.len(), 6);
    assert!(cosmetic_variants::ALL.len() >= 6);
}

#[test]
fn prototype_content_pins_the_accepted_initial_family_records() {
    let unarmed = action_families::get(ActionFamilyId::Unarmed);
    assert_eq!(unarmed.activation, ActionActivation::Press);
    assert_eq!(unarmed.contact_kind, ActionContactKind::Melee);
    assert_eq!(unarmed.windup_ticks, 6);
    assert_eq!(unarmed.active_ticks, Some(4));
    assert!(!unarmed.held_active);
    assert_eq!(unarmed.recovery_ticks, 12);
    assert_eq!(unarmed.cooldown_ticks, 24);
    assert_eq!(unarmed.reach_px, Some(30.0));
    assert_eq!(unarmed.projectile_speed_px_per_second, None);
    assert_eq!(unarmed.projectile_lifetime_ticks, None);
    assert_eq!(unarmed.front_arc_degrees, 100.0);
    assert_eq!(unarmed.movement_multiplier, 0.8);
    assert_eq!(
        unarmed.unguarded_outcome,
        Some(CombatOutcomeData {
            interruption_ticks: 10,
            displacement_px: 8.0,
            ball_spill: true
        })
    );
    assert_eq!(unarmed.guarded_recoil_px, 6.0);

    let guard = action_families::get(ActionFamilyId::Guard);
    assert_eq!(guard.activation, ActionActivation::Held);
    assert_eq!(guard.contact_kind, ActionContactKind::Guard);
    assert_eq!(guard.windup_ticks, 6);
    assert_eq!(guard.active_ticks, None);
    assert!(guard.held_active);
    assert_eq!(guard.recovery_ticks, 9);
    assert_eq!(guard.cooldown_ticks, 0);
    assert_eq!(guard.reach_px, None);
    assert_eq!(guard.projectile_speed_px_per_second, None);
    assert_eq!(guard.projectile_lifetime_ticks, None);
    assert_eq!(guard.front_arc_degrees, 120.0);
    assert_eq!(guard.movement_multiplier, 0.55);
    assert_eq!(guard.unguarded_outcome, None);
    assert_eq!(guard.guarded_recoil_px, 0.0);

    let melee = action_families::get(ActionFamilyId::LightMelee);
    assert_eq!(melee.activation, ActionActivation::Press);
    assert_eq!(melee.contact_kind, ActionContactKind::Melee);
    assert_eq!(melee.windup_ticks, 12);
    assert_eq!(melee.active_ticks, Some(5));
    assert!(!melee.held_active);
    assert_eq!(melee.recovery_ticks, 21);
    assert_eq!(melee.cooldown_ticks, 42);
    assert_eq!(melee.reach_px, Some(42.0));
    assert_eq!(melee.projectile_speed_px_per_second, None);
    assert_eq!(melee.projectile_lifetime_ticks, None);
    assert_eq!(melee.front_arc_degrees, 75.0);
    assert_eq!(melee.movement_multiplier, 0.5);
    assert_eq!(
        melee.unguarded_outcome,
        Some(CombatOutcomeData {
            interruption_ticks: 18,
            displacement_px: 18.0,
            ball_spill: true
        })
    );
    assert_eq!(melee.guarded_recoil_px, 6.0);

    let ranged = action_families::get(ActionFamilyId::Ranged);
    assert_eq!(ranged.activation, ActionActivation::HeldRelease);
    assert_eq!(ranged.contact_kind, ActionContactKind::Projectile);
    assert_eq!(ranged.windup_ticks, 18);
    assert_eq!(ranged.active_ticks, Some(1));
    assert!(!ranged.held_active);
    assert_eq!(ranged.recovery_ticks, 27);
    assert_eq!(ranged.cooldown_ticks, 60);
    assert_eq!(ranged.reach_px, None);
    assert_eq!(ranged.projectile_speed_px_per_second, Some(300.0));
    assert_eq!(ranged.projectile_lifetime_ticks, Some(60));
    assert_eq!(ranged.front_arc_degrees, 20.0);
    assert_eq!(ranged.movement_multiplier, 0.4);
    assert_eq!(
        ranged.unguarded_outcome,
        Some(CombatOutcomeData {
            interruption_ticks: 12,
            displacement_px: 10.0,
            ball_spill: true
        })
    );
    assert_eq!(ranged.guarded_recoil_px, 6.0);
}

#[test]
fn prototype_content_resolves_all_three_themed_swords_to_one_mechanical_table_by_identity() {
    let tournament = equipment_presentations::get("medieval_tournament_sword").unwrap();
    let vector = equipment_presentations::get("scifi_energy_blade").unwrap();
    let foam = equipment_presentations::get("toy_foam_sword").unwrap();

    let tournament_family = action_families::get(tournament.family_id);
    let vector_family = action_families::get(vector.family_id);
    let foam_family = action_families::get(foam.family_id);

    assert_eq!(
        tournament_family,
        action_families::get(ActionFamilyId::LightMelee)
    );
    assert_eq!(vector_family, tournament_family);
    assert_eq!(foam_family, tournament_family);
}

#[test]
fn prototype_content_uses_ten_stable_starters_and_no_more_than_six_reusable_presentations() {
    let mut seen_players: Vec<&str> = Vec::new();
    let mut seen_presentations: Vec<&str> = Vec::new();

    for team in [teams::get("nebula").unwrap(), teams::get("orion").unwrap()] {
        for player_id in team.roster {
            assert!(
                !seen_players.contains(player_id),
                "fixture repeats {player_id}"
            );
            seen_players.push(player_id);
            let player = players::get(player_id).unwrap();
            if !seen_presentations.contains(&player.presentation_id) {
                seen_presentations.push(player.presentation_id);
            }
        }
    }

    assert_eq!(seen_players.len(), 10);
    assert_eq!(seen_presentations.len(), 6);
}
