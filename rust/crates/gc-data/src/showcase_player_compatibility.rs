//! GOLISEO showcase-only identity and species seams. These records preserve
//! the shipped showcase while `PlayerData` moves to the presentation/loadout model.

/// A player's showcase-only planet, species, and trait identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShowcasePlayerCompatibilityData {
    /// Player id, from `players`; also the lookup key.
    pub player_id: &'static str,
    /// Home planet flavor text.
    pub planet: &'static str,
    /// Mechanical species id, from `species`.
    pub species: &'static str,
    /// Species id used for presentation only, from `species`.
    pub presentation_species: Option<&'static str>,
    /// Showcase trait flavor text.
    pub r#trait: &'static str,
}

/// Every authored showcase player compatibility row.
pub static ALL: &[ShowcasePlayerCompatibilityData] = &[
    ShowcasePlayerCompatibilityData {
        player_id: "zyro_vex",
        planet: "Kairon-9",
        species: "neutral",
        presentation_species: Some("voltari"),
        r#trait: "comet_first_touch",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "mika_olu",
        planet: "Vega Prime",
        species: "neutral",
        presentation_species: Some("myceloid"),
        r#trait: "nebula_vision",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "rok_tann",
        planet: "Titan Reach",
        species: "neutral",
        presentation_species: Some("terran"),
        r#trait: "quantum_pass",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "sela_dwin",
        planet: "Andromeda Fringe",
        species: "neutral",
        presentation_species: Some("voltari"),
        r#trait: "solar_flare_sprint",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "brakka",
        planet: "Orion Belt",
        species: "neutral",
        presentation_species: Some("gravling"),
        r#trait: "meteor_tackle",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "veil_nyx",
        planet: "Europa Deep",
        species: "neutral",
        presentation_species: Some("gravling"),
        r#trait: "gravity_anchor",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "ozzo",
        planet: "Kairon-9",
        species: "neutral",
        presentation_species: Some("terran"),
        r#trait: "zero_g_reflex",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "tib_quell",
        planet: "Mars Colony",
        species: "neutral",
        presentation_species: Some("myceloid"),
        r#trait: "comet_first_touch",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "gax_oru",
        planet: "Orion Belt",
        species: "neutral",
        presentation_species: Some("gravling"),
        r#trait: "gravity_anchor",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "drell",
        planet: "Orion Belt",
        species: "neutral",
        presentation_species: Some("gravling"),
        r#trait: "meteor_tackle",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "morv",
        planet: "Ceres Outpost",
        species: "neutral",
        presentation_species: Some("terran"),
        r#trait: "meteor_tackle",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "krag",
        planet: "Orion Belt",
        species: "neutral",
        presentation_species: Some("gravling"),
        r#trait: "comet_first_touch",
    },
    ShowcasePlayerCompatibilityData {
        player_id: "tox_vren",
        planet: "Ceres Outpost",
        species: "neutral",
        presentation_species: Some("voltari"),
        r#trait: "solar_flare_sprint",
    },
];

/// Look up a showcase compatibility row by player id.
pub fn get(player_id: &str) -> Option<&'static ShowcasePlayerCompatibilityData> {
    ALL.iter().find(|row| row.player_id == player_id)
}
