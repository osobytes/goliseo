//! Formations for small-sided 5v5 (1 keeper + 4 outfield). Content, not logic.
//!
//! Anchors are normalized in an "attacking right" frame:
//!   x: 0 = own goal line, 1 = opponent goal line (depth)
//!   y: 0 = top touchline, 1 = bottom touchline (width)
//! `sim::placement` converts these to absolute pitch coords and mirrors them
//! for the away side. The `outfield` order is the line order (defence -> attack);
//! a team's roster (minus keeper) must be listed in this same order.

/// A normalized pitch anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// 0..1 depth (own goal -> opponent goal).
    pub x: f64,
    /// 0..1 width (top -> bottom).
    pub y: f64,
}

/// The tactical role an outfield anchor plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormationRole {
    /// Defensive line.
    Def,
    /// Central midfield.
    Mid,
    /// Wide midfield.
    Wide,
    /// Forward line.
    Fwd,
}

/// An outfield anchor: a normalized position plus its tactical role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldAnchor {
    /// 0..1 depth (own goal -> opponent goal).
    pub x: f64,
    /// 0..1 width (top -> bottom).
    pub y: f64,
    /// Tactical role.
    pub role: FormationRole,
}

/// A formation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormationData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Strength blurb shown in formation-select UI.
    pub strength: Option<&'static str>,
    /// Risk blurb shown in formation-select UI.
    pub risk: Option<&'static str>,
    /// Keeper anchor.
    pub keeper: Anchor,
    /// Exactly 4 outfield anchors, in defence -> attack line order.
    pub outfield: &'static [OutfieldAnchor],
}

const GK: Anchor = Anchor { x: 0.06, y: 0.5 };

/// Every authored formation.
pub static ALL: &[FormationData] = &[
    FormationData {
        id: "2-1-1",
        name: "Balanced",
        strength: Some("Two defenders protect the middle."),
        risk: Some("The lone forward can become isolated."),
        keeper: GK,
        outfield: &[
            OutfieldAnchor {
                x: 0.28,
                y: 0.30,
                role: FormationRole::Def,
            },
            OutfieldAnchor {
                x: 0.28,
                y: 0.70,
                role: FormationRole::Def,
            },
            OutfieldAnchor {
                x: 0.52,
                y: 0.50,
                role: FormationRole::Mid,
            },
            OutfieldAnchor {
                x: 0.76,
                y: 0.50,
                role: FormationRole::Fwd,
            },
        ],
    },
    FormationData {
        id: "1-2-1",
        name: "Control",
        strength: Some("Two midfielders create passing angles."),
        risk: Some("Only one defender guards counterattacks."),
        keeper: GK,
        outfield: &[
            OutfieldAnchor {
                x: 0.26,
                y: 0.50,
                role: FormationRole::Def,
            },
            OutfieldAnchor {
                x: 0.52,
                y: 0.30,
                role: FormationRole::Wide,
            },
            OutfieldAnchor {
                x: 0.52,
                y: 0.70,
                role: FormationRole::Wide,
            },
            OutfieldAnchor {
                x: 0.78,
                y: 0.50,
                role: FormationRole::Fwd,
            },
        ],
    },
    FormationData {
        id: "1-1-2",
        name: "Aggressive",
        strength: Some("Two forwards keep constant goal pressure."),
        risk: Some("Large spaces open behind the first press."),
        keeper: GK,
        outfield: &[
            OutfieldAnchor {
                x: 0.26,
                y: 0.50,
                role: FormationRole::Def,
            },
            OutfieldAnchor {
                x: 0.50,
                y: 0.50,
                role: FormationRole::Mid,
            },
            OutfieldAnchor {
                x: 0.76,
                y: 0.30,
                role: FormationRole::Fwd,
            },
            OutfieldAnchor {
                x: 0.76,
                y: 0.70,
                role: FormationRole::Fwd,
            },
        ],
    },
];

/// Look up a formation by id.
pub fn get(id: &str) -> Option<&'static FormationData> {
    ALL.iter().find(|formation| formation.id == id)
}
