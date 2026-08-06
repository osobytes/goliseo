//! Fixed prototype loadouts. Both family and presentation ids are stored so
//! validation can reject a mechanical/presentation mismatch before a match.

use crate::action_families::ActionFamilyId;

/// A fixed prototype loadout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedLoadoutData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Mechanical action family.
    pub family_id: ActionFamilyId,
    /// Equipment presentation id, from `equipment_presentations`.
    pub equipment_presentation_id: &'static str,
}

/// Every authored fixed loadout.
pub static ALL: &[FixedLoadoutData] = &[
    FixedLoadoutData {
        id: "loadout_emberguard_shield",
        family_id: ActionFamilyId::Guard,
        equipment_presentation_id: "medieval_heater_shield",
    },
    FixedLoadoutData {
        id: "loadout_tournament_sword",
        family_id: ActionFamilyId::LightMelee,
        equipment_presentation_id: "medieval_tournament_sword",
    },
    FixedLoadoutData {
        id: "loadout_vector_blade",
        family_id: ActionFamilyId::LightMelee,
        equipment_presentation_id: "scifi_energy_blade",
    },
    FixedLoadoutData {
        id: "loadout_pulse_blaster",
        family_id: ActionFamilyId::Ranged,
        equipment_presentation_id: "scifi_pulse_blaster",
    },
    FixedLoadoutData {
        id: "loadout_spring_gloves",
        family_id: ActionFamilyId::Unarmed,
        equipment_presentation_id: "toy_spring_gloves",
    },
    FixedLoadoutData {
        id: "loadout_foam_champion",
        family_id: ActionFamilyId::LightMelee,
        equipment_presentation_id: "toy_foam_sword",
    },
];

/// Look up a fixed loadout by id.
pub fn get(id: &str) -> Option<&'static FixedLoadoutData> {
    ALL.iter().find(|loadout| loadout.id == id)
}
