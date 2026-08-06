//! Reusable character presentation identities. Runtime meshes and materials stay
//! in the render layer; these records carry no stat or action-family data.

/// A cosmetic theme shared by characters and equipment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrototypeThemeId {
    /// Medieval fantasy theme.
    MedievalFantasy,
    /// Galactic sci-fi theme.
    GalacticScifi,
    /// Toybox theme.
    Toybox,
}

/// The shared skeletal rig a presentation uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RigId {
    /// The single shared medium rig.
    RigMedium,
}

/// A reusable character presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterPresentationData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Cosmetic theme.
    pub theme_id: PrototypeThemeId,
    /// Shared skeletal rig.
    pub rig_id: RigId,
}

/// Every authored character presentation.
pub static ALL: &[CharacterPresentationData] = &[
    CharacterPresentationData {
        id: "medieval_rook_emberguard",
        name: "Rook Emberguard",
        theme_id: PrototypeThemeId::MedievalFantasy,
        rig_id: RigId::RigMedium,
    },
    CharacterPresentationData {
        id: "medieval_bramble_quickstep",
        name: "Bramble Quickstep",
        theme_id: PrototypeThemeId::MedievalFantasy,
        rig_id: RigId::RigMedium,
    },
    CharacterPresentationData {
        id: "scifi_nova_quell",
        name: "Nova Quell",
        theme_id: PrototypeThemeId::GalacticScifi,
        rig_id: RigId::RigMedium,
    },
    CharacterPresentationData {
        id: "scifi_axi",
        name: "AX-7 \"Axi\"",
        theme_id: PrototypeThemeId::GalacticScifi,
        rig_id: RigId::RigMedium,
    },
    CharacterPresentationData {
        id: "toy_moxie_modular",
        name: "Moxie Modular",
        theme_id: PrototypeThemeId::Toybox,
        rig_id: RigId::RigMedium,
    },
    CharacterPresentationData {
        id: "toy_tock",
        name: "Tock",
        theme_id: PrototypeThemeId::Toybox,
        rig_id: RigId::RigMedium,
    },
];

/// Look up a character presentation by id.
pub fn get(id: &str) -> Option<&'static CharacterPresentationData> {
    ALL.iter().find(|presentation| presentation.id == id)
}
