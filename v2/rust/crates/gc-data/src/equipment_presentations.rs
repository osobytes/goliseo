//! Equipment appearance maps onto shared action families. Runtime models,
//! sockets, effects, and audio stay outside this crate.

use crate::action_families::ActionFamilyId;
use crate::character_presentations::PrototypeThemeId;

/// Where an equipment presentation attaches on the rig.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentAttachment {
    /// Attaches to the left hand socket.
    LeftHand,
    /// Attaches to the right hand socket.
    RightHand,
    /// Attaches to both hand sockets.
    BothHands,
}

/// A themed appearance for a mechanical action family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquipmentPresentationData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Cosmetic theme.
    pub theme_id: PrototypeThemeId,
    /// Mechanical action family this presentation appears for.
    pub family_id: ActionFamilyId,
    /// Rig attachment socket.
    pub attachment: EquipmentAttachment,
}

/// Every authored equipment presentation.
pub static ALL: &[EquipmentPresentationData] = &[
    EquipmentPresentationData {
        id: "medieval_heater_shield",
        name: "Emberguard Shield",
        theme_id: PrototypeThemeId::MedievalFantasy,
        family_id: ActionFamilyId::Guard,
        attachment: EquipmentAttachment::LeftHand,
    },
    EquipmentPresentationData {
        id: "medieval_tournament_sword",
        name: "Tournament Sword",
        theme_id: PrototypeThemeId::MedievalFantasy,
        family_id: ActionFamilyId::LightMelee,
        attachment: EquipmentAttachment::RightHand,
    },
    EquipmentPresentationData {
        id: "scifi_energy_blade",
        name: "Vector Blade",
        theme_id: PrototypeThemeId::GalacticScifi,
        family_id: ActionFamilyId::LightMelee,
        attachment: EquipmentAttachment::RightHand,
    },
    EquipmentPresentationData {
        id: "scifi_pulse_blaster",
        name: "Pulse Blaster",
        theme_id: PrototypeThemeId::GalacticScifi,
        family_id: ActionFamilyId::Ranged,
        attachment: EquipmentAttachment::RightHand,
    },
    EquipmentPresentationData {
        id: "toy_spring_gloves",
        name: "Spring Gloves",
        theme_id: PrototypeThemeId::Toybox,
        family_id: ActionFamilyId::Unarmed,
        attachment: EquipmentAttachment::BothHands,
    },
    EquipmentPresentationData {
        id: "toy_foam_sword",
        name: "Foam Champion",
        theme_id: PrototypeThemeId::Toybox,
        family_id: ActionFamilyId::LightMelee,
        attachment: EquipmentAttachment::RightHand,
    },
];

/// Look up an equipment presentation by id.
pub fn get(id: &str) -> Option<&'static EquipmentPresentationData> {
    ALL.iter().find(|presentation| presentation.id == id)
}
