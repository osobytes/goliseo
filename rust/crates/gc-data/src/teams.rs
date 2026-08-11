//! Teams for Slice 0. A roster lists exactly 5 player ids and must contain exactly
//! one keeper; the remaining 4 are listed in formation line order (defence -> attack),
//! to match the chosen formation's `outfield` order (see `formations`).

/// A team.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeamData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// `{r, g, b}` in 0..1.
    pub color: [f64; 3],
    /// Key into `formations`.
    pub formation: &'static str,
    /// 5 player ids from `players`.
    pub roster: &'static [&'static str],
    /// Eligible player ids; defaults to `roster` when absent.
    pub squad: Option<&'static [&'static str]>,
}

/// Every authored team.
pub static ALL: &[TeamData] = &[
    TeamData {
        id: "nebula",
        name: "Nebula FC",
        color: [0.3, 0.7, 1.0],
        formation: "2-1-1",
        roster: &["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
        squad: Some(&[
            "ozzo",
            "brakka",
            "veil_nyx",
            "rok_tann",
            "zyro_vex",
            "mika_olu",
            "sela_dwin",
            "tib_quell",
        ]),
    },
    TeamData {
        id: "orion",
        name: "Orion Miners",
        color: [1.0, 0.5, 0.3],
        formation: "1-1-2",
        roster: &["gax_oru", "drell", "morv", "krag", "tox_vren"],
        squad: None,
    },
];

/// Look up a team by id.
pub fn get(id: &str) -> Option<&'static TeamData> {
    ALL.iter().find(|team| team.id == id)
}
