//! Showcase species identity. MVP species use authored player stats for their
//! mechanical identity; modifiers and verbs remain neutral until after MVP.

/// A simulation verb a species could eventually trigger. All MVP species are `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimVerb {
    /// A vertical leap action.
    Jump,
    /// A physical collision action.
    Collision,
    /// A speed burst action.
    Burst,
    /// A close-control dribble action.
    Dribble,
    /// A blocking action.
    Block,
    /// A teammate-linking action.
    Link,
    /// No verb; mechanically inert.
    None,
}

/// A species' visual silhouette family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Round silhouette.
    Round,
    /// Broad silhouette.
    Broad,
    /// Angular silhouette.
    Angular,
    /// Clustered silhouette.
    Cluster,
}

/// Stat deltas a species would apply. All MVP species are zeroed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatModifier {
    /// Pace delta.
    pub pace: i64,
    /// Strength delta.
    pub strength: i64,
    /// Technique delta.
    pub technique: i64,
    /// Stamina delta.
    pub stamina: i64,
    /// Mental delta.
    pub mental: i64,
}

/// A showcase species.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeciesData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Stat deltas this species applies.
    pub modifiers: StatModifier,
    /// Simulation verb this species could trigger.
    pub verb: SimVerb,
    /// Named skill, if any.
    pub skill: Option<&'static str>,
    /// Short marketing tagline.
    pub tagline: Option<&'static str>,
    /// `{r, g, b}` in 0..1.
    pub palette: Option<[f64; 3]>,
    /// Visual silhouette family.
    pub shape: Option<Shape>,
}

const ZERO_MODIFIERS: StatModifier = StatModifier {
    pace: 0,
    strength: 0,
    technique: 0,
    stamina: 0,
    mental: 0,
};

/// Every authored species.
pub static ALL: &[SpeciesData] = &[
    SpeciesData {
        id: "neutral",
        name: "Neutral",
        modifiers: ZERO_MODIFIERS,
        verb: SimVerb::None,
        skill: None,
        tagline: Some("Adaptable all-rounders"),
        palette: Some([0.55, 0.72, 0.92]),
        shape: Some(Shape::Round),
    },
    SpeciesData {
        id: "terran",
        name: "Terran",
        modifiers: ZERO_MODIFIERS,
        verb: SimVerb::None,
        skill: None,
        tagline: Some("Composed and versatile"),
        palette: Some([0.35, 0.75, 1.0]),
        shape: Some(Shape::Round),
    },
    SpeciesData {
        id: "gravling",
        name: "Gravling",
        modifiers: ZERO_MODIFIERS,
        verb: SimVerb::None,
        skill: None,
        tagline: Some("Powerful anchor players"),
        palette: Some([1.0, 0.55, 0.25]),
        shape: Some(Shape::Broad),
    },
    SpeciesData {
        id: "voltari",
        name: "Voltari",
        modifiers: ZERO_MODIFIERS,
        verb: SimVerb::None,
        skill: None,
        tagline: Some("Electric breakaway speed"),
        palette: Some([0.9, 0.85, 0.2]),
        shape: Some(Shape::Angular),
    },
    SpeciesData {
        id: "myceloid",
        name: "Myceloid",
        modifiers: ZERO_MODIFIERS,
        verb: SimVerb::None,
        skill: None,
        tagline: Some("Technical collective minds"),
        palette: Some([0.7, 0.4, 0.95]),
        shape: Some(Shape::Cluster),
    },
];

/// Look up a species by id.
pub fn get(id: &str) -> Option<&'static SpeciesData> {
    ALL.iter().find(|species| species.id == id)
}
