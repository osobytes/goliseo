//! Authored player identities for the showcase and prototype fixtures.
//! Content, not logic — see AGENTS.md §8.

/// A player's field position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// Goalkeeper.
    Keeper,
    /// Defender.
    Defender,
    /// Midfielder.
    Midfielder,
    /// Forward.
    Forward,
}

/// The five canonical mechanical attributes for a player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatBlock {
    /// Movement speed.
    pub pace: i64,
    /// Physical strength.
    pub strength: i64,
    /// Ball control and passing precision.
    pub technique: i64,
    /// Endurance over the match.
    pub stamina: i64,
    /// Decision making and composure.
    pub mental: i64,
}

/// A single authored player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerData {
    /// Persistent identity, stable across presentation and loadout changes.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Shirt number.
    pub number: i64,
    /// Field position.
    pub position: Position,
    /// The five canonical mechanical attributes.
    pub stats: StatBlock,
    /// Reusable character presentation; never contributes stats.
    pub presentation_id: &'static str,
    /// Persistent presentation-only variation.
    pub cosmetic_variant_id: Option<&'static str>,
    /// Fixed prototype loadout; keepers have none.
    pub loadout_id: Option<&'static str>,
}

/// Every authored player.
pub static ALL: &[PlayerData] = &[
    PlayerData {
        id: "zyro_vex",
        name: "Zyro Vex",
        number: 9,
        position: Position::Forward,
        stats: StatBlock {
            pace: 8,
            strength: 6,
            technique: 7,
            stamina: 5,
            mental: 2,
        },
        presentation_id: "medieval_bramble_quickstep",
        cosmetic_variant_id: Some("bramble_berry"),
        loadout_id: Some("loadout_spring_gloves"),
    },
    PlayerData {
        id: "mika_olu",
        name: "Mika Olu",
        number: 11,
        position: Position::Forward,
        stats: StatBlock {
            pace: 7,
            strength: 5,
            technique: 8,
            stamina: 6,
            mental: 3,
        },
        presentation_id: "toy_tock",
        cosmetic_variant_id: Some("tock_cherry"),
        loadout_id: Some("loadout_foam_champion"),
    },
    PlayerData {
        id: "rok_tann",
        name: "Rok Tann",
        number: 8,
        position: Position::Midfielder,
        stats: StatBlock {
            pace: 5,
            strength: 7,
            technique: 6,
            stamina: 7,
            mental: 5,
        },
        presentation_id: "scifi_nova_quell",
        cosmetic_variant_id: Some("nova_cyan"),
        loadout_id: Some("loadout_pulse_blaster"),
    },
    PlayerData {
        id: "sela_dwin",
        name: "Sela Dwin",
        number: 10,
        position: Position::Midfielder,
        stats: StatBlock {
            pace: 6,
            strength: 4,
            technique: 7,
            stamina: 6,
            mental: 6,
        },
        presentation_id: "scifi_axi",
        cosmetic_variant_id: Some("axi_orange"),
        loadout_id: Some("loadout_emberguard_shield"),
    },
    PlayerData {
        id: "brakka",
        name: "Brakka",
        number: 4,
        position: Position::Defender,
        stats: StatBlock {
            pace: 4,
            strength: 8,
            technique: 3,
            stamina: 7,
            mental: 8,
        },
        presentation_id: "medieval_rook_emberguard",
        cosmetic_variant_id: Some("rook_ember"),
        loadout_id: Some("loadout_emberguard_shield"),
    },
    PlayerData {
        id: "veil_nyx",
        name: "Veil Nyx",
        number: 5,
        position: Position::Defender,
        stats: StatBlock {
            pace: 5,
            strength: 6,
            technique: 4,
            stamina: 6,
            mental: 7,
        },
        presentation_id: "toy_moxie_modular",
        cosmetic_variant_id: Some("moxie_ocean"),
        loadout_id: Some("loadout_tournament_sword"),
    },
    PlayerData {
        id: "ozzo",
        name: "Ozzo",
        number: 1,
        position: Position::Keeper,
        stats: StatBlock {
            pace: 4,
            strength: 5,
            technique: 4,
            stamina: 8,
            mental: 8,
        },
        presentation_id: "scifi_axi",
        cosmetic_variant_id: Some("axi_blue"),
        loadout_id: None,
    },
    PlayerData {
        id: "tib_quell",
        name: "Tib Quell",
        number: 6,
        position: Position::Midfielder,
        stats: StatBlock {
            pace: 6,
            strength: 6,
            technique: 6,
            stamina: 5,
            mental: 5,
        },
        presentation_id: "medieval_bramble_quickstep",
        cosmetic_variant_id: Some("bramble_moss"),
        loadout_id: Some("loadout_pulse_blaster"),
    },
    // Orion Miners: a physical asteroid-colony team (low technique, high strength/mental).
    PlayerData {
        id: "gax_oru",
        name: "Gax Oru",
        number: 13,
        position: Position::Keeper,
        stats: StatBlock {
            pace: 4,
            strength: 6,
            technique: 3,
            stamina: 8,
            mental: 8,
        },
        presentation_id: "toy_tock",
        cosmetic_variant_id: Some("tock_brass"),
        loadout_id: None,
    },
    PlayerData {
        id: "drell",
        name: "Drell",
        number: 2,
        position: Position::Defender,
        stats: StatBlock {
            pace: 4,
            strength: 9,
            technique: 2,
            stamina: 7,
            mental: 8,
        },
        presentation_id: "medieval_rook_emberguard",
        cosmetic_variant_id: Some("rook_steel"),
        loadout_id: Some("loadout_emberguard_shield"),
    },
    PlayerData {
        id: "morv",
        name: "Morv",
        number: 7,
        position: Position::Midfielder,
        stats: StatBlock {
            pace: 5,
            strength: 7,
            technique: 4,
            stamina: 7,
            mental: 6,
        },
        presentation_id: "scifi_axi",
        cosmetic_variant_id: Some("axi_orange"),
        loadout_id: Some("loadout_vector_blade"),
    },
    PlayerData {
        id: "krag",
        name: "Krag",
        number: 14,
        position: Position::Forward,
        stats: StatBlock {
            pace: 6,
            strength: 8,
            technique: 4,
            stamina: 6,
            mental: 3,
        },
        presentation_id: "toy_moxie_modular",
        cosmetic_variant_id: Some("moxie_sun"),
        loadout_id: Some("loadout_pulse_blaster"),
    },
    PlayerData {
        id: "tox_vren",
        name: "Tox Vren",
        number: 17,
        position: Position::Forward,
        stats: StatBlock {
            pace: 7,
            strength: 7,
            technique: 5,
            stamina: 5,
            mental: 3,
        },
        presentation_id: "scifi_nova_quell",
        cosmetic_variant_id: Some("nova_magenta"),
        loadout_id: Some("loadout_spring_gloves"),
    },
];

/// Look up a player by persistent id.
pub fn get(id: &str) -> Option<&'static PlayerData> {
    ALL.iter().find(|player| player.id == id)
}
