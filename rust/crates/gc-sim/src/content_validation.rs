//! Pure validation for authored player, presentation, equipment, and
//! fixed-loadout content. Invalid authored data is a programmer error, so
//! this module asserts (AGENTS.md §7) — every failure here is a panic, and
//! every function that can fail returns normally on success (there is
//! nothing recoverable to hand back to a caller).
//!
//! This module validates only what a `struct`/`enum` type cannot already
//! guarantee at compile time: numeric bounds, uniqueness, and
//! cross-reference checks. Shape and closed-set-membership checks are
//! unnecessary here precisely because the type system already enforces
//! them.

use gc_data::action_families::{
    ActionActivation, ActionContactKind, ActionFamilyData, ActionFamilyId, CombatOutcomeData,
};
use gc_data::character_presentations::CharacterPresentationData;
use gc_data::cosmetic_variants::CosmeticVariantData;
use gc_data::equipment_presentations::EquipmentPresentationData;
use gc_data::loadouts::FixedLoadoutData;
use gc_data::players::{PlayerData, Position};
use indexmap::IndexMap;

const MAX_ACTION_TICKS: i64 = 600;
const MAX_INTERRUPTION_TICKS: i64 = 30;
const MAX_REACH_PX: f64 = 240.0;
const MAX_PROJECTILE_SPEED: f64 = 2000.0;
const MAX_DISPLACEMENT_PX: f64 = 60.0;

/// A team, in the owned/mutable shape [`validate_fixture`] needs: unlike
/// `gc_data::teams::TeamData`'s `&'static [&'static str]` roster/squad
/// (authored content is immutable), a caller validating a *candidate*
/// fixture needs to build and mutate rosters freely.
#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    /// Persistent identity.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Key into `formations`.
    pub formation: &'static str,
    /// Exactly 5 player ids.
    pub roster: Vec<&'static str>,
    /// Eligible player ids; defaults to `roster` when absent.
    pub squad: Option<Vec<&'static str>>,
}

impl From<&gc_data::teams::TeamData> for Team {
    fn from(team: &gc_data::teams::TeamData) -> Self {
        Team {
            id: team.id,
            name: team.name,
            formation: team.formation,
            roster: team.roster.to_vec(),
            squad: team.squad.map(|squad| squad.to_vec()),
        }
    }
}

/// The complete authored content one fixture draws from.
pub struct PrototypeContentCatalog {
    /// Reusable character presentations, keyed by id.
    pub character_presentations: IndexMap<&'static str, CharacterPresentationData>,
    /// Persistent cosmetic variants, keyed by id.
    pub cosmetic_variants: IndexMap<&'static str, CosmeticVariantData>,
    /// The four shared combat mechanics families, keyed by id.
    ///
    /// A `Vec` rather than a map: `ActionFamilyId` does not derive `Hash`
    /// (it lives in `gc-data`, a crate this module doesn't own), and four
    /// entries make a linear scan free either way.
    pub action_families: Vec<(ActionFamilyId, ActionFamilyData)>,
    /// Equipment presentations, keyed by id.
    pub equipment_presentations: IndexMap<&'static str, EquipmentPresentationData>,
    /// Fixed prototype loadouts, keyed by id.
    pub loadouts: IndexMap<&'static str, FixedLoadoutData>,
    /// Every authored player.
    pub players: Vec<PlayerData>,
}

/// Optional relaxations for [`validate_fixture`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrototypeFixturePolicy {
    /// Allow a team to field more than one outfielder from the same combat
    /// family (the showcase default requires one of each).
    pub allow_repeated_families: bool,
}

fn action_family(catalog: &PrototypeContentCatalog, id: ActionFamilyId) -> ActionFamilyData {
    catalog
        .action_families
        .iter()
        .find(|(family_id, _)| *family_id == id)
        .unwrap_or_else(|| panic!("missing action family: {id:?}"))
        .1
}

fn validate_outcome(outcome: &CombatOutcomeData, label: &str) {
    assert!(
        (0..=MAX_INTERRUPTION_TICKS).contains(&outcome.interruption_ticks),
        "{label}.interruption_ticks must be bounded non-negative ticks"
    );
    assert!(
        outcome.displacement_px >= 0.0 && outcome.displacement_px <= MAX_DISPLACEMENT_PX,
        "{label}.displacement_px is outside the prototype bounds"
    );
}

fn validate_family(family: &ActionFamilyData, label: &str) {
    assert!(
        !family.name.is_empty(),
        "{label}.name must be a non-empty string"
    );
    assert!(
        (0..=MAX_ACTION_TICKS).contains(&family.windup_ticks),
        "{label}.windup_ticks must be bounded non-negative ticks"
    );
    assert!(
        (0..=MAX_ACTION_TICKS).contains(&family.recovery_ticks),
        "{label}.recovery_ticks must be bounded non-negative ticks"
    );
    assert!(
        (0..=MAX_ACTION_TICKS).contains(&family.cooldown_ticks),
        "{label}.cooldown_ticks must be bounded non-negative ticks"
    );
    assert!(
        family.front_arc_degrees > 0.0 && family.front_arc_degrees <= 180.0,
        "{label}.front_arc_degrees must be in (0, 180]"
    );
    assert!(
        family.movement_multiplier > 0.0 && family.movement_multiplier <= 1.0,
        "{label}.movement_multiplier must be in (0, 1]"
    );
    assert!(
        (0.0..=6.0).contains(&family.guarded_recoil_px),
        "{label}.guarded_recoil_px must be in [0, 6]"
    );

    if family.id == ActionFamilyId::Guard {
        assert!(
            family.activation == ActionActivation::Held,
            "{label} must use held activation"
        );
        assert!(
            family.contact_kind == ActionContactKind::Guard,
            "{label} must use guard contact"
        );
        assert!(family.held_active, "{label} must remain active while held");
        assert!(
            family.active_ticks.is_none(),
            "{label} cannot have timed active ticks"
        );
        assert!(family.cooldown_ticks == 0, "{label} has no cooldown");
        assert!(
            family.reach_px.is_none(),
            "{label} guards self rather than using reach"
        );
        assert!(
            family.unguarded_outcome.is_none(),
            "{label} cannot author an attack outcome"
        );
        assert!(
            family.projectile_speed_px_per_second.is_none()
                && family.projectile_lifetime_ticks.is_none(),
            "{label} cannot author projectile fields"
        );
    } else {
        assert!(
            !family.held_active,
            "{label} cannot remain active while held"
        );
        let active_ticks = family
            .active_ticks
            .unwrap_or_else(|| panic!("{label}.active_ticks is required"));
        assert!(
            (0..=MAX_ACTION_TICKS).contains(&active_ticks),
            "{label}.active_ticks must be bounded non-negative ticks"
        );
        assert!(active_ticks > 0, "{label}.active_ticks must be positive");
        let outcome = family
            .unguarded_outcome
            .unwrap_or_else(|| panic!("{label}.unguarded_outcome is required"));
        validate_outcome(&outcome, &format!("{label}.unguarded_outcome"));
        assert!(outcome.ball_spill, "{label} must spill an owned ball");

        if family.id == ActionFamilyId::Ranged {
            assert!(
                family.activation == ActionActivation::HeldRelease,
                "{label} must fire on release"
            );
            assert!(
                family.contact_kind == ActionContactKind::Projectile,
                "{label} must use projectile contact"
            );
            assert!(
                family.reach_px.is_none(),
                "{label} uses projectile travel instead of melee reach"
            );
            let speed = family
                .projectile_speed_px_per_second
                .unwrap_or_else(|| panic!("{label}.projectile_speed_px_per_second is required"));
            assert!(
                speed > 0.0 && speed <= MAX_PROJECTILE_SPEED,
                "{label}.projectile_speed_px_per_second is outside the prototype bounds"
            );
            let lifetime = family
                .projectile_lifetime_ticks
                .unwrap_or_else(|| panic!("{label}.projectile_lifetime_ticks is required"));
            assert!(
                (0..=MAX_ACTION_TICKS).contains(&lifetime),
                "{label}.projectile_lifetime_ticks must be bounded non-negative ticks"
            );
            assert!(
                lifetime > 0,
                "{label}.projectile_lifetime_ticks must be positive"
            );
        } else {
            assert!(
                family.activation == ActionActivation::Press,
                "{label} must commit on press"
            );
            assert!(
                family.contact_kind == ActionContactKind::Melee,
                "{label} must use melee contact"
            );
            let reach = family
                .reach_px
                .unwrap_or_else(|| panic!("{label}.reach_px is required"));
            assert!(
                reach > 0.0 && reach <= MAX_REACH_PX,
                "{label}.reach_px is outside the prototype bounds"
            );
            assert!(
                family.projectile_speed_px_per_second.is_none()
                    && family.projectile_lifetime_ticks.is_none(),
                "{label} cannot author projectile fields"
            );
        }
    }
}

fn players_by_id(catalog: &PrototypeContentCatalog) -> IndexMap<&'static str, PlayerData> {
    let mut by_id = IndexMap::new();
    for player in &catalog.players {
        assert!(
            !player.id.is_empty(),
            "players.id must be a non-empty string"
        );
        assert!(
            !by_id.contains_key(player.id),
            "duplicate player id: {}",
            player.id
        );
        by_id.insert(player.id, *player);
    }
    by_id
}

fn validate_player(player: &PlayerData, catalog: &PrototypeContentCatalog, label: &str) {
    assert!(
        !player.name.is_empty(),
        "{label}.name must be a non-empty string"
    );
    assert!(
        (1..=99).contains(&player.number),
        "{label}.number must be an integer in [1, 99]"
    );
    let stats = player.stats;
    for (name, value) in [
        ("pace", stats.pace),
        ("strength", stats.strength),
        ("technique", stats.technique),
        ("stamina", stats.stamina),
        ("mental", stats.mental),
    ] {
        assert!(
            (0..=10).contains(&value),
            "{label}.stats.{name} must be an integer in [0, 10]"
        );
    }

    assert!(
        catalog
            .character_presentations
            .contains_key(player.presentation_id),
        "{label} has unknown presentation id"
    );
    if let Some(cosmetic_variant_id) = player.cosmetic_variant_id {
        let cosmetic = catalog
            .cosmetic_variants
            .get(cosmetic_variant_id)
            .unwrap_or_else(|| panic!("{label} has unknown cosmetic variant id"));
        assert!(
            cosmetic.presentation_id == player.presentation_id,
            "{label} cosmetic variant belongs to another presentation"
        );
    }
    if player.position == Position::Keeper {
        assert!(
            player.loadout_id.is_none(),
            "{label} keeper cannot have a combat loadout"
        );
    } else {
        let loadout_id = player
            .loadout_id
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| panic!("{label}.loadout_id must be a non-empty string"));
        assert!(
            catalog.loadouts.contains_key(loadout_id),
            "{label} has unknown loadout id"
        );
    }
}

/// Validate one authored content catalog as a coherent, self-consistent
/// whole: registry sizes, cross-references, and every numeric bound.
///
/// # Panics
///
/// Panics (via `assert!`) on the first invariant violation — authored
/// content is programmer-controlled, so a violation is a bug, not a
/// recoverable runtime condition (AGENTS.md §7).
pub fn validate_catalog(catalog: &PrototypeContentCatalog) -> bool {
    assert!(
        catalog.character_presentations.len() == 6,
        "prototype needs exactly six character presentations"
    );
    for (key, presentation) in &catalog.character_presentations {
        let label = format!("character_presentations.{key}");
        assert!(
            !presentation.name.is_empty(),
            "{label}.name must be a non-empty string"
        );
    }

    assert!(
        catalog.cosmetic_variants.len() >= 6,
        "prototype needs reusable cosmetic variants"
    );
    for (key, cosmetic) in &catalog.cosmetic_variants {
        let label = format!("cosmetic_variants.{key}");
        assert!(
            catalog
                .character_presentations
                .contains_key(cosmetic.presentation_id),
            "{label} has unknown presentation id"
        );
        assert!(
            !cosmetic.material_variant_id.is_empty(),
            "{label}.material_variant_id must be a non-empty string"
        );
        if let Some(head_variant_id) = cosmetic.head_variant_id {
            assert!(
                !head_variant_id.is_empty(),
                "{label}.head_variant_id must be a non-empty string"
            );
        }
        if let Some(accessory_id) = cosmetic.accessory_id {
            assert!(
                !accessory_id.is_empty(),
                "{label}.accessory_id must be a non-empty string"
            );
        }
    }

    assert!(
        catalog.action_families.len() == 4,
        "prototype needs exactly four action families"
    );
    for id in [
        ActionFamilyId::Unarmed,
        ActionFamilyId::Guard,
        ActionFamilyId::LightMelee,
        ActionFamilyId::Ranged,
    ] {
        let family = action_family(catalog, id);
        validate_family(&family, &format!("action_families.{id:?}"));
    }

    assert!(
        catalog.equipment_presentations.len() == 6,
        "prototype needs exactly six equipment presentations"
    );
    for (key, equipment) in &catalog.equipment_presentations {
        let label = format!("equipment_presentations.{key}");
        assert!(
            !equipment.name.is_empty(),
            "{label}.name must be a non-empty string"
        );
        assert!(
            catalog
                .action_families
                .iter()
                .any(|(id, _)| *id == equipment.family_id),
            "{label} has unknown family id"
        );
    }

    assert!(
        catalog.loadouts.len() == 6,
        "prototype needs exactly six fixed loadouts"
    );
    for (key, loadout) in &catalog.loadouts {
        let label = format!("loadouts.{key}");
        assert!(
            catalog
                .action_families
                .iter()
                .any(|(id, _)| *id == loadout.family_id),
            "{label} has unknown family id"
        );
        let equipment = catalog
            .equipment_presentations
            .get(loadout.equipment_presentation_id)
            .unwrap_or_else(|| panic!("{label} has unknown equipment presentation id"));
        assert!(
            equipment.family_id == loadout.family_id,
            "{label} family does not match its equipment presentation"
        );
    }

    let light_melee = action_family(catalog, ActionFamilyId::LightMelee);
    for id in [
        "medieval_tournament_sword",
        "scifi_energy_blade",
        "toy_foam_sword",
    ] {
        let equipment = catalog
            .equipment_presentations
            .get(id)
            .unwrap_or_else(|| panic!("missing light-melee presentation: {id}"));
        assert!(
            equipment.family_id == light_melee.id,
            "{id} must resolve to the shared light_melee record"
        );
    }

    let by_id = players_by_id(catalog);
    for (id, player) in &by_id {
        validate_player(player, catalog, &format!("players.{id}"));
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn validate_team(
    team: &Team,
    players: &IndexMap<&'static str, PlayerData>,
    catalog: &PrototypeContentCatalog,
    fixture_players: &mut Vec<&'static str>,
    eligible_players: &mut Vec<&'static str>,
    allow_repeated_families: bool,
) {
    let label = format!("team.{}", team.id);
    assert!(!team.id.is_empty(), "{label}.id must be a non-empty string");
    assert!(
        !team.name.is_empty(),
        "{label}.name must be a non-empty string"
    );
    assert!(
        !team.formation.is_empty(),
        "{label}.formation must be a non-empty string"
    );
    assert!(
        team.roster.len() == 5,
        "{label} roster must contain exactly five players"
    );

    let mut team_players: Vec<&'static str> = Vec::new();
    let mut numbers: Vec<i64> = Vec::new();
    let mut keeper_count = 0;
    let mut family_counts: Vec<(ActionFamilyId, i64)> = Vec::new();
    for &player_id in &team.roster {
        assert!(
            !player_id.is_empty(),
            "{label} roster id must be a non-empty string"
        );
        assert!(
            !team_players.contains(&player_id),
            "{label} roster contains duplicate player {player_id}"
        );
        assert!(
            !fixture_players.contains(&player_id),
            "fixture contains player on both teams: {player_id}"
        );
        team_players.push(player_id);
        fixture_players.push(player_id);

        let player = players
            .get(player_id)
            .unwrap_or_else(|| panic!("{label} has unknown player {player_id}"));
        assert!(
            !numbers.contains(&player.number),
            "{label} duplicates shirt number {}",
            player.number
        );
        numbers.push(player.number);
        if player.position == Position::Keeper {
            keeper_count += 1;
        } else {
            let loadout_id = player
                .loadout_id
                .unwrap_or_else(|| panic!("{label} outfielder {player_id} has no loadout"));
            let loadout = catalog
                .loadouts
                .get(loadout_id)
                .unwrap_or_else(|| panic!("{label} outfielder {player_id} has unknown loadout"));
            match family_counts
                .iter_mut()
                .find(|(id, _)| *id == loadout.family_id)
            {
                Some((_, count)) => *count += 1,
                None => family_counts.push((loadout.family_id, 1)),
            }
        }
    }
    assert!(
        keeper_count == 1,
        "{label} roster must contain exactly one keeper"
    );
    if !allow_repeated_families {
        for id in [
            ActionFamilyId::Unarmed,
            ActionFamilyId::Guard,
            ActionFamilyId::LightMelee,
            ActionFamilyId::Ranged,
        ] {
            let count = family_counts
                .iter()
                .find(|(f, _)| *f == id)
                .map_or(0, |(_, c)| *c);
            assert!(
                count == 1,
                "{label} must contain exactly one {id:?} outfielder"
            );
        }
    }

    let eligible_ids: Vec<&'static str> = team.squad.clone().unwrap_or_else(|| team.roster.clone());
    let mut squad_players: Vec<&'static str> = Vec::new();
    let mut squad_numbers: Vec<i64> = Vec::new();
    for &player_id in &eligible_ids {
        let squad_player = players
            .get(player_id)
            .unwrap_or_else(|| panic!("{label} squad has unknown player {player_id}"));
        assert!(
            !squad_players.contains(&player_id),
            "{label} squad contains duplicate player"
        );
        assert!(
            !eligible_players.contains(&player_id),
            "fixture contains eligible player on both teams: {player_id}"
        );
        assert!(
            !squad_numbers.contains(&squad_player.number),
            "{label} squad duplicates shirt number {}",
            squad_player.number
        );
        squad_players.push(player_id);
        eligible_players.push(player_id);
        squad_numbers.push(squad_player.number);
    }
    for player_id in &team_players {
        assert!(
            squad_players.contains(player_id),
            "{label} starter is missing from the squad"
        );
    }
}

/// Validate that `home` and `away` form a legal fixture drawn from `catalog`:
/// a valid catalog, distinct team records, five-player rosters with exactly
/// one keeper and (unless relaxed by `policy`) one outfielder per combat
/// family, a superset squad, and ten total distinct fixture players spanning
/// at most six character presentations.
///
/// # Panics
///
/// Panics (via `assert!`) on the first invariant violation, same as
/// [`validate_catalog`].
pub fn validate_fixture(
    catalog: &PrototypeContentCatalog,
    home: &Team,
    away: &Team,
    policy: Option<PrototypeFixturePolicy>,
) -> bool {
    validate_catalog(catalog);
    assert!(
        !std::ptr::eq(home, away),
        "fixture teams must be distinct records"
    );
    let players = players_by_id(catalog);
    let mut fixture_players: Vec<&'static str> = Vec::new();
    let mut eligible_players: Vec<&'static str> = Vec::new();
    let allow_repeated_families = policy.is_some_and(|p| p.allow_repeated_families);
    validate_team(
        home,
        &players,
        catalog,
        &mut fixture_players,
        &mut eligible_players,
        allow_repeated_families,
    );
    validate_team(
        away,
        &players,
        catalog,
        &mut fixture_players,
        &mut eligible_players,
        allow_repeated_families,
    );

    assert!(
        fixture_players.len() == 10,
        "prototype fixture must contain ten distinct players"
    );
    let mut presentation_ids: Vec<&'static str> = Vec::new();
    for player_id in &fixture_players {
        let presentation_id = players[player_id].presentation_id;
        if !presentation_ids.contains(&presentation_id) {
            presentation_ids.push(presentation_id);
        }
    }
    assert!(
        presentation_ids.len() <= 6,
        "prototype fixture exceeds six character presentations"
    );
    true
}
