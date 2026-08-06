//! Team mentalities. Each one nudges AI behavior; effects are intentionally simple
//! and readable (see AGENTS.md vision: strategy felt, not a full simulation).
//!
//!   press      : how many off-ball players hunt the contested ball
//!   line_shift : anchor depth bias along the attack axis (fraction of pitch);
//!                + pushes the shape upfield, - drops it deeper
//!   stamina_drain : multiplier (stub for M4)
//!   transition : how long a turnover keeps this team counter-pressing or
//!                counter-attacking before it returns to its ordinary shape

/// How non-presser defenders behave. `match::new` fills any missing block with
/// the hybrid default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkingScheme {
    /// Defenders track zones rather than opponents.
    Zonal,
    /// Defenders track assigned opponents.
    Man,
    /// A mix of man-marking and zonal coverage.
    Hybrid,
}

/// Off-ball marking/shape config (see `sim::ai` + `move_players`). `scheme` picks
/// how non-presser defenders behave; the rest are tuning knobs for how tight the
/// press is, how hard the block collapses toward the ball, and how far attackers
/// push to support. `match::new` fills any missing block with the hybrid default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkingConfig {
    /// How non-presser defenders behave.
    pub scheme: MarkingScheme,
    /// Opponents to man-mark in hybrid (0 ignored otherwise).
    pub man_marks: i64,
    /// Px the presser holds off the carrier.
    pub standoff: f64,
    /// 0..1 how hard the block shifts to ball/goal when defending.
    pub compactness: f64,
    /// 0..1 attacking off-ball aggressiveness (support depth).
    pub support: f64,
}

/// Possession-transition windows (see `sim::possession_transition`, whose
/// `brain.phase` decay turns them into per-team counterpress/counterattack
/// phases). A turnover gives the team that LOST the ball `counterpress` seconds
/// of urgent hunting and the team that WON it `counterattack` seconds of extra
/// attacking push; both then decay into ordinary defend/attack. Zero disables a
/// window outright. `match::new` fills any missing block with the balanced
/// default, so a tactic authored without one still simulates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionConfig {
    /// Seconds the losing team counter-presses.
    pub counterpress: f64,
    /// Seconds the winning team counter-attacks.
    pub counterattack: f64,
}

/// A team mentality.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TacticData {
    /// Persistent identity, also the lookup key.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Strength blurb shown in tactic-select UI.
    pub strength: Option<&'static str>,
    /// Risk blurb shown in tactic-select UI.
    pub risk: Option<&'static str>,
    /// How many off-ball players hunt the contested ball.
    pub press: i64,
    /// Anchor depth bias along the attack axis (fraction of pitch).
    pub line_shift: f64,
    /// Stamina drain multiplier (stub for M4).
    pub stamina_drain: f64,
    /// Off-ball marking/shape config.
    pub marking: MarkingConfig,
    /// Possession-transition windows.
    pub transition: TransitionConfig,
}

/// Every authored tactic.
pub static ALL: &[TacticData] = &[
    TacticData {
        id: "balanced",
        name: "Balanced",
        strength: Some("Keeps one presser and a compact supporting shape."),
        risk: Some("Creates fewer overloads at either end."),
        press: 1,
        line_shift: 0.0,
        stamina_drain: 1.0,
        marking: MarkingConfig {
            scheme: MarkingScheme::Hybrid,
            man_marks: 1,
            standoff: 32.0,
            compactness: 0.5,
            support: 0.5,
        },
        transition: TransitionConfig {
            counterpress: 2.5,
            counterattack: 2.5,
        },
    },
    TacticData {
        id: "press_high",
        name: "Press High",
        strength: Some("Wins the ball closer to the opponent goal."),
        risk: Some("A beaten press exposes space behind it."),
        press: 2,
        line_shift: 0.12,
        stamina_drain: 1.4,
        marking: MarkingConfig {
            scheme: MarkingScheme::Man,
            man_marks: 3,
            standoff: 22.0,
            compactness: 0.7,
            support: 0.65,
        },
        transition: TransitionConfig {
            counterpress: 3.0,
            counterattack: 2.5,
        },
    },
    TacticData {
        id: "counter",
        name: "Counter Attack",
        strength: Some("Drops compact, then attacks open space quickly."),
        risk: Some("Concedes territory and sustained possession."),
        press: 1,
        line_shift: -0.12,
        stamina_drain: 0.9,
        marking: MarkingConfig {
            scheme: MarkingScheme::Zonal,
            man_marks: 0,
            standoff: 40.0,
            compactness: 0.35,
            support: 0.4,
        },
        transition: TransitionConfig {
            counterpress: 0.0,
            counterattack: 3.0,
        },
    },
];

/// Look up a tactic by id.
pub fn get(id: &str) -> Option<&'static TacticData> {
    ALL.iter().find(|tactic| tactic.id == id)
}
