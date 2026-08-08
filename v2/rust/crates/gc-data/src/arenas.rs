//! Arenas. Content, not logic — see AGENTS.md §8.

/// An arena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArenaData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// In-universe location flavor text.
    pub location: &'static str,
    /// `{r, g, b}` in 0..1.
    pub floor_color: [f64; 3],
    /// `{r, g, b}` in 0..1.
    pub marking_color: [f64; 3],
    /// `{r, g, b}` in 0..1.
    pub rail_color: [f64; 3],
    /// `{r, g, b}` in 0..1.
    pub highlight_color: [f64; 3],
}

/// Every authored arena.
pub static ALL: &[ArenaData] = &[ArenaData {
    id: "helios_crown",
    name: "Helios Crown",
    location: "Kairon-9 Orbit",
    floor_color: [0.025, 0.16, 0.17],
    marking_color: [0.35, 0.72, 1.0],
    rail_color: [0.25, 0.88, 1.0],
    highlight_color: [1.0, 0.66, 0.24],
}];

/// Look up an arena by id.
pub fn get(id: &str) -> Option<&'static ArenaData> {
    ALL.iter().find(|arena| arena.id == id)
}
