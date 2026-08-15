//! Shared combat mechanics. Equipment presentations and player appearance point
//! here by id; they never copy these tuning values.

/// The mechanical family an action belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionFamilyId {
    /// No equipment; bare-handed action.
    Unarmed,
    /// Blocking / guarding action.
    Guard,
    /// Light melee weapon action.
    LightMelee,
    /// Ranged projectile action.
    Ranged,
}

/// How an action is activated by input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionActivation {
    /// Fires once on button press.
    Press,
    /// Active for as long as the button is held.
    Held,
    /// Charges while held, fires on release.
    HeldRelease,
}

/// The kind of contact an action makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionContactKind {
    /// Melee contact.
    Melee,
    /// Guard contact.
    Guard,
    /// Projectile contact.
    Projectile,
}

/// The consequence of contact landing on an unguarded target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatOutcomeData {
    /// Ticks the target is interrupted for.
    pub interruption_ticks: i64,
    /// Px the target is displaced.
    pub displacement_px: f64,
    /// Whether the ball spills loose on contact.
    pub ball_spill: bool,
}

/// A mechanical action family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionFamilyData {
    /// Persistent identity, also the lookup key.
    pub id: ActionFamilyId,
    /// Display name.
    pub name: &'static str,
    /// How the action is activated by input.
    pub activation: ActionActivation,
    /// The kind of contact the action makes.
    pub contact_kind: ActionContactKind,
    /// Ticks of windup before the action becomes active.
    pub windup_ticks: i64,
    /// Ticks the action stays active, for press-activated actions.
    pub active_ticks: Option<i64>,
    /// Whether the action stays active for as long as it is held.
    pub held_active: bool,
    /// Ticks of recovery after the action ends.
    pub recovery_ticks: i64,
    /// Ticks before the action can be used again.
    pub cooldown_ticks: i64,
    /// Melee reach in px.
    pub reach_px: Option<f64>,
    /// Projectile speed in px/second.
    pub projectile_speed_px_per_second: Option<f64>,
    /// Projectile lifetime in ticks.
    pub projectile_lifetime_ticks: Option<i64>,
    /// Width of the frontal contact arc, in degrees. Presentation (arc
    /// drawing) and hashing/identity read this; the simulation's per-tick
    /// contact and feasibility checks read [`Self::front_arc_cos`] instead.
    pub front_arc_degrees: f64,
    /// Cosine of half [`Self::front_arc_degrees`], precomputed rather than
    /// taken with `.to_radians().cos()` on the simulation path. `cos` is not
    /// correctly rounded, and Rust links a different libm for
    /// `wasm32-unknown-unknown` than a native build uses, so a runtime call
    /// here would make a native peer and a browser peer disagree in the low
    /// bits on every combat contact/feasibility check, every tick. Same
    /// reasoning and shipped precedent as `gc_data::locomotion`'s
    /// `LOCO_REVERSE_ARC_COS` / `LOCO_STRAFE_ARC_COS` /
    /// `LOCO_BACKPEDAL_ARC_COS`. Content is data (AGENTS.md §8): this is the
    /// authored value, not a value derived by code at runtime.
    pub front_arc_cos: f64,
    /// Movement speed multiplier while the action is active.
    pub movement_multiplier: f64,
    /// Consequence of contact landing on an unguarded target, if this family has one.
    pub unguarded_outcome: Option<CombatOutcomeData>,
    /// Px the actor recoils when contact is guarded.
    pub guarded_recoil_px: f64,
}

/// Every authored action family.
pub static ALL: &[ActionFamilyData] = &[
    ActionFamilyData {
        id: ActionFamilyId::Unarmed,
        name: "Unarmed",
        activation: ActionActivation::Press,
        contact_kind: ActionContactKind::Melee,
        windup_ticks: 6,
        active_ticks: Some(4),
        held_active: false,
        recovery_ticks: 12,
        cooldown_ticks: 24,
        reach_px: Some(30.0),
        projectile_speed_px_per_second: None,
        projectile_lifetime_ticks: None,
        front_arc_degrees: 100.0,
        front_arc_cos: 0.6427876096865394,
        movement_multiplier: 0.8,
        unguarded_outcome: Some(CombatOutcomeData {
            interruption_ticks: 10,
            displacement_px: 8.0,
            ball_spill: true,
        }),
        guarded_recoil_px: 6.0,
    },
    ActionFamilyData {
        id: ActionFamilyId::Guard,
        name: "Guard",
        activation: ActionActivation::Held,
        contact_kind: ActionContactKind::Guard,
        windup_ticks: 6,
        active_ticks: None,
        held_active: true,
        recovery_ticks: 9,
        cooldown_ticks: 0,
        reach_px: None,
        projectile_speed_px_per_second: None,
        projectile_lifetime_ticks: None,
        front_arc_degrees: 120.0,
        front_arc_cos: 0.5000000000000001,
        movement_multiplier: 0.55,
        unguarded_outcome: None,
        guarded_recoil_px: 0.0,
    },
    ActionFamilyData {
        id: ActionFamilyId::LightMelee,
        name: "Light Melee",
        activation: ActionActivation::Press,
        contact_kind: ActionContactKind::Melee,
        windup_ticks: 12,
        active_ticks: Some(5),
        held_active: false,
        recovery_ticks: 21,
        cooldown_ticks: 42,
        reach_px: Some(42.0),
        projectile_speed_px_per_second: None,
        projectile_lifetime_ticks: None,
        front_arc_degrees: 75.0,
        front_arc_cos: 0.7933533402912352,
        movement_multiplier: 0.5,
        unguarded_outcome: Some(CombatOutcomeData {
            interruption_ticks: 18,
            displacement_px: 18.0,
            ball_spill: true,
        }),
        guarded_recoil_px: 6.0,
    },
    ActionFamilyData {
        id: ActionFamilyId::Ranged,
        name: "Ranged",
        activation: ActionActivation::HeldRelease,
        contact_kind: ActionContactKind::Projectile,
        windup_ticks: 18,
        active_ticks: Some(1),
        held_active: false,
        recovery_ticks: 27,
        cooldown_ticks: 60,
        reach_px: None,
        projectile_speed_px_per_second: Some(300.0),
        projectile_lifetime_ticks: Some(60),
        front_arc_degrees: 20.0,
        front_arc_cos: 0.984807753012208,
        movement_multiplier: 0.4,
        unguarded_outcome: Some(CombatOutcomeData {
            interruption_ticks: 12,
            displacement_px: 10.0,
            ball_spill: true,
        }),
        guarded_recoil_px: 6.0,
    },
];

/// Look up an action family by id.
pub fn get(id: ActionFamilyId) -> &'static ActionFamilyData {
    ALL.iter()
        .find(|family| family.id == id)
        .expect("every ActionFamilyId has a record")
}
