//! Authored tunable content: the tier-1 sim-affecting scalars, the tier-3 AI
//! membership band-sets, and the balance metrics the harness scores.
//!
//! Data, not mechanism (AGENTS.md §8). This module holds the *shapes* and the
//! *values*; the registry container, registration API, ordering, duplicate
//! detection, serialization and config hashing all live in
//! `gc_sim::tunable_registry`. Nothing here is a function over game state, and
//! this crate keeps requiring nothing (AGENTS.md §2: `data/` requires nothing),
//! which is why the type declarations live beside the tables rather than in
//! `gc-core`.
//!
//! ## The three tiers, and where each one physically lives
//!
//! 1. **[`Tier::Sim`] — sim-affecting scalars.** Every number the simulation
//!    reads that changes match outcomes. The ONLY tier a balance sweep may
//!    vary. Authored here as [`SIM_TUNABLES`].
//! 2. **[`Tier::Presentation`] — presentation-only constants.** Smoothing
//!    rates, follow-through windows, marker sizes. Never read by `gc-sim`.
//!    Deliberately **not** authored in this crate: `gc-sim` depends on
//!    `gc-data`, so anything here is reachable from the simulation and the
//!    isolation promise would be a comment rather than a fact. Tier 2 is
//!    authored in `gc_render::presentation_tunables`, and `gc-render` depends
//!    on `gc-sim` — so a `gc-sim` module that tried to read a tier-2 value
//!    would be a dependency cycle the compiler rejects. That is the structural
//!    enforcement; see that module's doc.
//! 3. **[`Tier::Bands`] — AI membership bands.** The edges that classify a
//!    continuous situation into a discrete AI choice. Versioned and
//!    substituted **as one unit**: an edge only means anything relative to its
//!    neighbours, so editing one in isolation produces classifications nobody
//!    designed. Authored here as [`BAND_SETS`].
//!
//! The boundaries fall on the two operational questions: *can changing this
//! desync a match?* (tier 2 no, tiers 1 and 3 yes) and *is this value
//! meaningful alone?* (tier 1 yes, tier 3 no).

/// Which tunable tier an entry belongs to. See the module doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Sim-affecting scalar: sweepable, hashed into the config identity.
    Sim,
    /// Presentation-only: invisible to the sim and to the sweep, never in a
    /// snapshot.
    Presentation,
    /// An edge inside a versioned AI membership band-set: hashed, but only
    /// substitutable as a whole set.
    Bands,
}

impl Tier {
    /// Stable wire/serialization tag for this tier. Each tier serializes into
    /// its own blob, tagged with this string.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Tier::Sim => "sim",
            Tier::Presentation => "presentation",
            Tier::Bands => "bands",
        }
    }
}

/// One declaratively registered tunable.
///
/// `label`, `cat` and `step` exist for the F1 tuning panel; `unit`, `desc`
/// and the `min`/`max` range exist so a sweep can enumerate a knob it has
/// never heard of and still produce meaningful values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TunableDef {
    /// Registry id, also the serialization key. Unique across all tiers.
    pub id: &'static str,
    /// Which tier this value belongs to.
    pub tier: Tier,
    /// Shown in the tuning panel.
    pub label: &'static str,
    /// Panel tab this knob's category groups into.
    pub cat: &'static str,
    /// Value a fresh match plays with.
    pub default: f64,
    /// Physical unit, for sweep reports and panel tooltips.
    pub unit: &'static str,
    /// Minimum allowed value.
    pub min: f64,
    /// Maximum allowed value.
    pub max: f64,
    /// Nudge step size.
    pub step: f64,
    /// One line on what this controls.
    pub desc: &'static str,
}

/// One edge inside a [`BandSet`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandEdge {
    /// Edge id, unique within its set. The registry id is
    /// `"<set_id>.<edge_id>"`.
    pub id: &'static str,
    /// The edge value.
    pub value: f64,
    /// What crossing this edge changes.
    pub desc: &'static str,
}

/// A versioned group of AI membership band edges, substituted as one unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandSet {
    /// Set id, stable across versions.
    pub id: &'static str,
    /// Version of this set's edge values. A substitution must change it.
    pub version: &'static str,
    /// What continuous situation this set classifies.
    pub desc: &'static str,
    /// The edges, in the order the classifier tests them.
    pub edges: &'static [BandEdge],
}

impl BandSet {
    /// One edge's value by id.
    #[must_use]
    pub fn edge(&self, id: &str) -> Option<f64> {
        self.edges.iter().find(|e| e.id == id).map(|e| e.value)
    }
}

/// Which way a balance metric wants to move to get better.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricDirection {
    /// Desirable inside its band; neither end is "better" on its own.
    Banded,
}

/// One registered balance metric: what the harness measures and what band it
/// scores against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricDef {
    /// Metric id, also its key in batch aggregates and reports.
    pub id: &'static str,
    /// Trapezoid desirability band `{zero_lo, good_lo, good_hi, zero_hi}`:
    /// worth 1 inside `[good_lo, good_hi]`, falling linearly to 0 at the outer
    /// edges.
    pub band: [f64; 4],
    /// Direction of goodness.
    pub direction: MetricDirection,
    /// One line on what this measures.
    pub desc: &'static str,
}

/// Every authored tier-1 sim-affecting scalar, in panel display order.
///
/// Display order, not id order: the F1 panel groups by `cat` and shows knobs
/// in this sequence. The registry derives its own sorted-by-id canonical order
/// for hashing (see `gc_sim::tunable_registry`), so a re-order here is a
/// cosmetic panel change and never moves the config hash.
pub static SIM_TUNABLES: &[TunableDef] = &[
    // Movement
    TunableDef {
        id: "MOVE_ACCEL",
        tier: Tier::Sim,
        label: "Acceleration",
        cat: "Movement",
        default: 1100.0,
        unit: "px/s^2",
        min: 400.0,
        max: 2400.0,
        step: 100.0,
        desc: "Acceleration a player at speed applies toward its move target.",
    },
    TunableDef {
        id: "START_ACCEL",
        tier: Tier::Sim,
        label: "Standing start",
        cat: "Movement",
        default: 450.0,
        unit: "px/s^2",
        min: 150.0,
        max: 1100.0,
        step: 50.0,
        desc: "Acceleration from a standstill, before the run builds.",
    },
    TunableDef {
        id: "MOVE_DECEL",
        tier: Tier::Sim,
        label: "Deceleration",
        cat: "Movement",
        default: 1500.0,
        unit: "px/s^2",
        min: 400.0,
        max: 3000.0,
        step: 100.0,
        desc: "Braking rate when a player stops or reverses.",
    },
    TunableDef {
        id: "SPRINT_MULT",
        tier: Tier::Sim,
        label: "Sprint speed x",
        cat: "Movement",
        default: 1.35,
        unit: "x",
        min: 1.1,
        max: 1.8,
        step: 0.05,
        desc: "Top-speed multiplier while the sprint meter is being spent.",
    },
    TunableDef {
        id: "SPRINT_REFILL",
        tier: Tier::Sim,
        label: "Sprint refill /s",
        cat: "Movement",
        default: 0.4,
        unit: "meter/s",
        min: 0.1,
        max: 1.0,
        step: 0.05,
        desc: "Sprint meter regained per second while not sprinting.",
    },
    TunableDef {
        id: "JOCKEY_SLOW",
        tier: Tier::Sim,
        label: "Jockey speed x",
        cat: "Movement",
        default: 0.75,
        unit: "x",
        min: 0.5,
        max: 1.0,
        step: 0.05,
        desc: "Speed multiplier while jockeying a carrier.",
    },
    // Locomotion (per-context kinematics; see gc_data::locomotion)
    TunableDef {
        id: "LOCO_JOG_TOP_MULT",
        tier: Tier::Sim,
        label: "Jog top speed x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.5,
        max: 1.4,
        step: 0.02,
        desc: "Top-speed multiplier over base move speed while in the jog context.",
    },
    TunableDef {
        id: "LOCO_JOG_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Jog accel x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the shared base while in the jog context.",
    },
    TunableDef {
        id: "LOCO_JOG_DECEL_MULT",
        tier: Tier::Sim,
        label: "Jog decel x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the shared base while in the jog context.",
    },
    TunableDef {
        id: "LOCO_JOG_TURN_MULT",
        tier: Tier::Sim,
        label: "Jog turn x",
        cat: "Locomotion",
        default: 1.6,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the shared base while in the jog context.",
    },
    TunableDef {
        id: "LOCO_RUN_TOP_MULT",
        tier: Tier::Sim,
        label: "Run top speed x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.5,
        max: 1.4,
        step: 0.02,
        desc: "Top-speed multiplier over base move speed while in the run context.",
    },
    TunableDef {
        id: "LOCO_RUN_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Run accel x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the shared base while in the run context.",
    },
    TunableDef {
        id: "LOCO_RUN_DECEL_MULT",
        tier: Tier::Sim,
        label: "Run decel x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the shared base while in the run context.",
    },
    TunableDef {
        id: "LOCO_RUN_TURN_MULT",
        tier: Tier::Sim,
        label: "Run turn x",
        cat: "Locomotion",
        default: 1.0,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the shared base while in the run context.",
    },
    TunableDef {
        id: "LOCO_SPRINT_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Sprint accel x",
        cat: "Locomotion",
        default: 0.85,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the shared base while in the sprint context.",
    },
    TunableDef {
        id: "LOCO_SPRINT_DECEL_MULT",
        tier: Tier::Sim,
        label: "Sprint decel x",
        cat: "Locomotion",
        default: 0.8,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the shared base while in the sprint context.",
    },
    TunableDef {
        id: "LOCO_SPRINT_TURN_MULT",
        tier: Tier::Sim,
        label: "Sprint turn x",
        cat: "Locomotion",
        default: 0.55,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the shared base while in the sprint context.",
    },
    TunableDef {
        id: "LOCO_CARRY_TOP_MULT",
        tier: Tier::Sim,
        label: "Carry top speed x",
        cat: "Locomotion",
        default: 0.92,
        unit: "x",
        min: 0.5,
        max: 1.4,
        step: 0.02,
        desc: "Top-speed multiplier over the direction context's, while carrying.",
    },
    TunableDef {
        id: "LOCO_CARRY_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Carry accel x",
        cat: "Locomotion",
        default: 0.9,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the direction context's, while carrying.",
    },
    TunableDef {
        id: "LOCO_CARRY_DECEL_MULT",
        tier: Tier::Sim,
        label: "Carry decel x",
        cat: "Locomotion",
        default: 1.05,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the direction context's, while carrying.",
    },
    TunableDef {
        id: "LOCO_CARRY_TURN_MULT",
        tier: Tier::Sim,
        label: "Carry turn x",
        cat: "Locomotion",
        default: 0.8,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the direction context's, while carrying.",
    },
    TunableDef {
        id: "LOCO_SPRINT_CARRY_TOP_MULT",
        tier: Tier::Sim,
        label: "Sprint carry top speed x",
        cat: "Locomotion",
        default: 0.874,
        unit: "x",
        min: 0.4,
        max: 1.1,
        step: 0.02,
        desc: "Top-speed multiplier over the SPRINT context's, while sprint-carrying (composes: 1.35 x 0.874 = the old absolute 1.18).",
    },
    TunableDef {
        id: "LOCO_SPRINT_CARRY_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Sprint carry accel x",
        cat: "Locomotion",
        default: 0.882,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the SPRINT context's, while sprint-carrying.",
    },
    TunableDef {
        id: "LOCO_SPRINT_CARRY_DECEL_MULT",
        tier: Tier::Sim,
        label: "Sprint carry decel x",
        cat: "Locomotion",
        default: 1.0625,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the SPRINT context's, while sprint-carrying.",
    },
    TunableDef {
        id: "LOCO_SPRINT_CARRY_TURN_MULT",
        tier: Tier::Sim,
        label: "Sprint carry turn x",
        cat: "Locomotion",
        default: 0.818,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the SPRINT context's, while sprint-carrying.",
    },
    TunableDef {
        id: "LOCO_STRAFE_TOP_MULT",
        tier: Tier::Sim,
        label: "Strafe top speed x",
        cat: "Locomotion",
        default: 0.75,
        unit: "x",
        min: 0.5,
        max: 1.4,
        step: 0.02,
        desc: "Top-speed multiplier over base move speed while in the strafe context.",
    },
    TunableDef {
        id: "LOCO_STRAFE_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Strafe accel x",
        cat: "Locomotion",
        default: 0.85,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the shared base while in the strafe context.",
    },
    TunableDef {
        id: "LOCO_STRAFE_DECEL_MULT",
        tier: Tier::Sim,
        label: "Strafe decel x",
        cat: "Locomotion",
        default: 1.15,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the shared base while in the strafe context.",
    },
    TunableDef {
        id: "LOCO_STRAFE_TURN_MULT",
        tier: Tier::Sim,
        label: "Strafe turn x",
        cat: "Locomotion",
        default: 1.3,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the shared base while in the strafe context.",
    },
    TunableDef {
        id: "LOCO_BACKPEDAL_TOP_MULT",
        tier: Tier::Sim,
        label: "Backpedal top speed x",
        cat: "Locomotion",
        default: 0.6,
        unit: "x",
        min: 0.5,
        max: 1.4,
        step: 0.02,
        desc: "Top-speed multiplier over base move speed while in the backpedal context.",
    },
    TunableDef {
        id: "LOCO_BACKPEDAL_ACCEL_MULT",
        tier: Tier::Sim,
        label: "Backpedal accel x",
        cat: "Locomotion",
        default: 0.7,
        unit: "x",
        min: 0.3,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration multiplier over the shared base while in the backpedal context.",
    },
    TunableDef {
        id: "LOCO_BACKPEDAL_DECEL_MULT",
        tier: Tier::Sim,
        label: "Backpedal decel x",
        cat: "Locomotion",
        default: 1.25,
        unit: "x",
        min: 0.3,
        max: 2.5,
        step: 0.05,
        desc: "Deceleration multiplier over the shared base while in the backpedal context.",
    },
    TunableDef {
        id: "LOCO_BACKPEDAL_TURN_MULT",
        tier: Tier::Sim,
        label: "Backpedal turn x",
        cat: "Locomotion",
        default: 1.1,
        unit: "x",
        min: 0.2,
        max: 2.5,
        step: 0.05,
        desc: "Angular-rate multiplier over the shared base while in the backpedal context.",
    },
    TunableDef {
        id: "LOCO_BASE_TURN",
        tier: Tier::Sim,
        label: "Base turn rate",
        cat: "Locomotion",
        default: 360.0,
        unit: "deg/s",
        min: 180.0,
        max: 1080.0,
        step: 20.0,
        desc: "Shared angular rate a body redirects its heading at, before context and technique.",
    },
    TunableDef {
        id: "LOCO_FACE_TURN_MULT",
        tier: Tier::Sim,
        label: "Facing rate x",
        cat: "Locomotion",
        default: 2.0,
        unit: "x",
        min: 0.5,
        max: 4.0,
        step: 0.1,
        desc: "How much faster facing rotates than the movement heading.",
    },
    TunableDef {
        id: "LOCO_TURN_EASE_DEG",
        tier: Tier::Sim,
        label: "Facing ease-out",
        cat: "Locomotion",
        default: 20.0,
        unit: "deg",
        min: 5.0,
        max: 45.0,
        step: 1.0,
        desc: "Remaining facing angle where bounded rotation hands over to a damped ease-out.",
    },
    TunableDef {
        id: "LOCO_FACE_EASE_FLOOR",
        tier: Tier::Sim,
        label: "Ease-out floor",
        cat: "Locomotion",
        default: 0.2,
        unit: "x",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "Floor under the ease-out rate, so facing lands in finite time instead of asymptoting.",
    },
    // Cosines, not degrees: each is compared against a dot product once per
    // player per tick, and a `cos()` there would put a non-correctly-rounded
    // libm call on the determinism path, where native and wasm disagree. See
    // `gc_data::locomotion`.
    TunableDef {
        id: "LOCO_REVERSE_ARC_COS",
        tier: Tier::Sim,
        label: "Reversal arc (cos)",
        cat: "Locomotion",
        default: -0.5,
        unit: "cos",
        min: -1.0,
        max: 0.0,
        step: 0.05,
        desc: "Cosine of the heading change beyond which a body brakes through zero (-0.5 = 120 deg).",
    },
    TunableDef {
        id: "LOCO_STRAFE_ARC_COS",
        tier: Tier::Sim,
        label: "Strafe arc (cos)",
        cat: "Locomotion",
        default: 0.57,
        unit: "cos",
        min: 0.02,
        max: 0.94,
        step: 0.02,
        desc: "Cosine of the movement-versus-facing angle beyond which the context is a strafe (0.57 = 55 deg).",
    },
    TunableDef {
        id: "LOCO_BACKPEDAL_ARC_COS",
        tier: Tier::Sim,
        label: "Backpedal arc (cos)",
        cat: "Locomotion",
        default: -0.5,
        unit: "cos",
        min: -1.0,
        max: 0.0,
        step: 0.05,
        desc: "Cosine of the movement-versus-facing angle beyond which the context is a backpedal (-0.5 = 120 deg).",
    },
    TunableDef {
        id: "LOCO_TURN_LOW_SPEED_BONUS",
        tier: Tier::Sim,
        label: "Standstill turn x",
        cat: "Locomotion",
        default: 2.0,
        unit: "x",
        min: 0.0,
        max: 5.0,
        step: 0.25,
        desc: "Extra angular rate available at a standstill, falling to zero at top speed.",
    },
    TunableDef {
        id: "LOCO_DIR_SNAP_SPEED",
        tier: Tier::Sim,
        label: "Heading snap speed",
        cat: "Locomotion",
        default: 6.0,
        unit: "px/s",
        min: 0.0,
        max: 60.0,
        step: 2.0,
        desc: "Speed below which the heading adopts the command outright rather than arcing.",
    },
    TunableDef {
        id: "LOCO_JOG_THROTTLE",
        tier: Tier::Sim,
        label: "Jog throttle",
        cat: "Locomotion",
        default: 0.65,
        unit: "x",
        min: 0.1,
        max: 1.0,
        step: 0.05,
        desc: "Commanded throttle below which unburdened movement resolves as a jog.",
    },
    TunableDef {
        id: "LOCO_PACE_CURVE_LO",
        tier: Tier::Sim,
        label: "Pace curve at 0",
        cat: "Locomotion",
        default: 0.8,
        unit: "x",
        min: 0.4,
        max: 1.0,
        step: 0.05,
        desc: "Acceleration and top-speed multiplier for a player at the bottom of the pace window.",
    },
    TunableDef {
        id: "LOCO_PACE_CURVE_HI",
        tier: Tier::Sim,
        label: "Pace curve at 1",
        cat: "Locomotion",
        default: 1.2,
        unit: "x",
        min: 1.0,
        max: 2.0,
        step: 0.05,
        desc: "Acceleration and top-speed multiplier for a player at the top of the pace window.",
    },
    TunableDef {
        id: "LOCO_STRENGTH_CURVE_LO",
        tier: Tier::Sim,
        label: "Strength curve at 0",
        cat: "Locomotion",
        default: 0.8,
        unit: "x",
        min: 0.4,
        max: 1.0,
        step: 0.05,
        desc: "Deceleration multiplier for a player with no strength.",
    },
    TunableDef {
        id: "LOCO_STRENGTH_CURVE_HI",
        tier: Tier::Sim,
        label: "Strength curve at 1",
        cat: "Locomotion",
        default: 1.2,
        unit: "x",
        min: 1.0,
        max: 2.0,
        step: 0.05,
        desc: "Deceleration multiplier for a maximum-strength player.",
    },
    TunableDef {
        id: "LOCO_TECHNIQUE_CURVE_LO",
        tier: Tier::Sim,
        label: "Technique curve at 0",
        cat: "Locomotion",
        default: 0.8,
        unit: "x",
        min: 0.4,
        max: 1.0,
        step: 0.05,
        desc: "Angular-rate multiplier for a player with no technique.",
    },
    TunableDef {
        id: "LOCO_TECHNIQUE_CURVE_HI",
        tier: Tier::Sim,
        label: "Technique curve at 1",
        cat: "Locomotion",
        default: 1.2,
        unit: "x",
        min: 1.0,
        max: 2.0,
        step: 0.05,
        desc: "Angular-rate multiplier for a maximum-technique player.",
    },
    TunableDef {
        id: "LOCO_PACE_REF_LO",
        tier: Tier::Sim,
        label: "Pace window low",
        cat: "Locomotion",
        default: 130.0,
        unit: "px/s",
        min: 80.0,
        max: 250.0,
        step: 10.0,
        desc: "Base move speed mapped to normalized pace 0.",
    },
    TunableDef {
        id: "LOCO_PACE_REF_HI",
        tier: Tier::Sim,
        label: "Pace window high",
        cat: "Locomotion",
        // 280, and the ceiling here is netcode rather than feel. The futsal
        // re-dimensioning wants roughly 289-329 px/s to cross the 1648px pitch
        // in real futsal's 5.0-5.7 s, so 300 was the natural pick -- but
        // gc-netcode's
        // `finds_no_driver_level_geometry_where_the_policy_guards_often_enough`
        // goes red between 290 and 295: at 295+ the `vs_ranged_scrum` geometry
        // produces a sustained rollback-correction storm, corrections on nearly
        // every tick for 60+ consecutive ticks, far past that test's
        // GUARD_POLICY_ROUTE_MINIMUM of 4. Bisected on the default alone, with
        // min/max held fixed because they feed the tape identity: 240/260/280
        // green, 290 and 310 break two combat-phase scenarios whose scripted
        // bursts stop landing inside their windup/contact windows, 295/300 trip
        // the guard storm. 280 is the only value in this neighbourhood where
        // the whole match_driver suite is green.
        //
        // It crosses in 5.89 s against futsal's 5.0-5.7 s -- about 3% slower
        // than this re-dimensioning aimed at, a deliberate concession to the
        // netcode ceiling rather than a miss. The real fix is the one that
        // test's own module doc prescribes: move `guard` onto the `Policy`
        // route. Raise this only after that lands.
        //
        // Do NOT read 292 as a clean threshold. A follow-up sweep of speed x
        // authored network profile found the effect is not monotonic in speed:
        // under the guard test's own delivery cadence plus the MILDEST authored
        // profile (Omp0Parity -- 3-tick delay, 1% loss, no jitter), 280 storms
        // (11 corrected guard ticks, 3/3 seeds) while 292 stays clean. The
        // likely cause is that the guard probe pumps its transport only every
        // 5th step, and that artificial batching aliases against movement
        // speed. Production does not batch: per docs/online/network_architecture.md
        // it sends every driver tick. Under a per-tick relay 280 stayed clean
        // against all four authored profiles, and 300+ stormed under every one
        // of them. So 280 is sound for how the game actually ships -- but the
        // guard probe is testing a worse-than-production delivery pattern over
        // a better-than-production wire, and no single number here is a hard
        // edge.
        default: 280.0,
        unit: "px/s",
        min: 180.0,
        max: 420.0,
        step: 10.0,
        desc: "Base move speed mapped to normalized pace 1.",
    },
    // Dribble (touch-based ball control)
    TunableDef {
        id: "DRIBBLE_CLOSE",
        tier: Tier::Sim,
        label: "Close ctrl x speed",
        cat: "Dribble",
        default: 1.05,
        unit: "x",
        min: 0.2,
        max: 1.4,
        step: 0.05,
        desc: "Carrier speed multiplier under close control.",
    },
    TunableDef {
        id: "DRIBBLE_PUSH",
        tier: Tier::Sim,
        label: "Touch push xspeed",
        cat: "Dribble",
        default: 1.5,
        unit: "x",
        min: 1.15,
        max: 2.2,
        step: 0.05,
        desc: "How far ahead of the carrier a pushed touch travels.",
    },
    TunableDef {
        id: "DRIBBLE_ERR",
        tier: Tier::Sim,
        label: "Touch error (rad)",
        cat: "Dribble",
        default: 0.3,
        unit: "rad",
        min: 0.0,
        max: 0.6,
        step: 0.05,
        desc: "Maximum angular error on a dribble touch.",
    },
    TunableDef {
        id: "DRIBBLE_TOUCH",
        tier: Tier::Sim,
        label: "Gather stiffness",
        cat: "Dribble",
        default: 14.0,
        unit: "1/s",
        min: 4.0,
        max: 30.0,
        step: 1.0,
        desc: "How hard the ball is pulled back onto the carrier's foot.",
    },
    TunableDef {
        id: "DRIBBLE_CONTROL",
        tier: Tier::Sim,
        label: "Control radius",
        cat: "Dribble",
        default: 34.0,
        unit: "px",
        min: 20.0,
        max: 80.0,
        step: 2.0,
        desc: "Radius inside which the carrier still counts as in control.",
    },
    // Aerial (headers, volleys, finishing crosses)
    TunableDef {
        id: "AERIAL_ASSIST",
        tier: Tier::Sim,
        label: "Strike reach aid",
        cat: "Aerial",
        default: 44.0,
        unit: "px",
        min: 0.0,
        max: 80.0,
        step: 4.0,
        desc: "Extra reach granted when striking a ball out of the air.",
    },
    TunableDef {
        id: "AERIAL_MAGNET",
        tier: Tier::Sim,
        label: "Ball magnet /s",
        cat: "Aerial",
        default: 260.0,
        unit: "px/s",
        min: 0.0,
        max: 600.0,
        step: 20.0,
        desc: "Pull applied to an airborne ball inside the strike window.",
    },
    // Attacking
    TunableDef {
        id: "CHARGE_RATE",
        tier: Tier::Sim,
        label: "Shot charge /s",
        cat: "Attacking",
        default: 1.5,
        unit: "charge/s",
        min: 0.5,
        max: 3.0,
        step: 0.1,
        desc: "How fast a held shot builds power.",
    },
    TunableDef {
        id: "PASS_CHARGE_RATE",
        tier: Tier::Sim,
        label: "Pass charge /s",
        cat: "Attacking",
        default: 2.4,
        unit: "charge/s",
        min: 0.5,
        max: 3.0,
        step: 0.1,
        desc: "How fast a held pass builds range.",
    },
    TunableDef {
        id: "SHOT_WINDUP",
        tier: Tier::Sim,
        label: "Shot wind-up s",
        cat: "Attacking",
        default: 0.15,
        unit: "s",
        min: 0.0,
        max: 0.4,
        step: 0.01,
        desc: "Committed wind-up before a shot leaves the foot.",
    },
    TunableDef {
        id: "PASS_RANGE_MIN",
        tier: Tier::Sim,
        label: "Min pass range",
        cat: "Attacking",
        default: 110.0,
        unit: "px",
        min: 60.0,
        max: 300.0,
        step: 10.0,
        desc: "Range of an uncharged pass; the floor PASS_RANGE_MAX charges up from.",
    },
    TunableDef {
        id: "PASS_RANGE_MAX",
        tier: Tier::Sim,
        label: "Max pass range",
        cat: "Attacking",
        default: 520.0,
        unit: "px",
        min: 300.0,
        max: 800.0,
        step: 20.0,
        desc: "Range of a fully charged pass.",
    },
    TunableDef {
        id: "HEADER_SPEED",
        tier: Tier::Sim,
        label: "Header pace x",
        cat: "Attacking",
        default: 0.85,
        unit: "x",
        min: 0.5,
        max: 1.2,
        step: 0.05,
        desc: "Ball pace multiplier off a header, relative to a foot strike.",
    },
    TunableDef {
        id: "VOLLEY_SKY_P",
        tier: Tier::Sim,
        label: "Volley sky odds",
        cat: "Attacking",
        default: 0.35,
        unit: "probability",
        min: 0.0,
        max: 1.0,
        step: 0.05,
        desc: "Chance a mistimed volley is skied. NOT YET WIRED: no sim code \
               reads this id, so it fails the knob-moves-metric contract \
               (gc_sim::knob_contract). Tracked as decoration, not balance.",
    },
    // Passing. Its own panel tab rather than eleven more entries under
    // Attacking, on #488's precedent for Locomotion: a subsystem with this
    // many knobs is a tab, and a designer piloting the soft cone should not
    // have to hunt for them between shot charge and header pace.
    //
    // Every range below STARTED as a prior converted from the metric ranges
    // the issue proposed, at a pitch scale of "960 px across a full-size
    // pitch, about 9 px per metre". That conversion was wrong and the note
    // claiming it has been removed rather than repaired. The renderer is the
    // only thing in this repository that ties a world unit to a metre: it
    // draws a player `PLAYER_RADIUS * HEIGHT_IN_RADII * 2` = 72 px tall, so
    // at a 1.75 m player one px is 24.31 mm and the pitch is about 41 px per
    // metre — 4.5x finer than the figure these ranges were converted at, and
    // the pitch was never full-size anyway. It is now 1648 px across a 40.1 m
    // futsal court.
    //
    // So treat these as EMPIRICAL values that survived play-testing, not as
    // metric priors: the numbers are load-bearing, the derivation behind them
    // is not. `PASS_ELIGIBLE_MAX`'s 560 px is 13.6 m, a sensible long pass on
    // a 40 m court, which is why re-deriving them was not worth the balance
    // churn. `PASS_ANGULAR_WEIGHT` in particular is feel-critical, and a
    // harness cannot feel frustration — see `gc_sim::passing`'s module doc.
    TunableDef {
        id: "PASS_ANGULAR_WEIGHT",
        tier: Tier::Sim,
        label: "Aim weight",
        cat: "Passing",
        // 140, not the 90 this shipped with in review: at 90 a teammate
        // DIRECTLY BEHIND the passer costs only 180 px of score, so aiming
        // upfield at a man 400 px away played the ball backwards to one
        // 200 px behind (`a_long_pass_is_driven_hard_enough_to_actually_arrive`
        // in gc-sim's tests/match.rs caught it). That is the "aim feels
        // ignored" failure the issue warns about at low weights, and the
        // measured constraint it produces is the only empirical input this
        // default has. At 140 a backward teammate must be 280 px NEARER to
        // win, while a 60-degree teammate still costs only 140 px -- soft,
        // not a gate. Still a prior: it needs hands-on pilots.
        default: 140.0,
        // Per unit CHORD, not per radian, and that is a determinism
        // requirement rather than a unit preference: see `gc_sim::passing`.
        unit: "px/chord",
        // The range spans the two failure modes the issue names, which is
        // what makes it a design space rather than a comfort zone: at 10 the
        // aim is essentially ignored, and at 400 the angular term exceeds the
        // whole eligible distance span (PASS_ELIGIBLE_MAX - PASS_ELIGIBLE_MIN
        // is 540 px, and 2 chord x 400 is 800), so the cone has hardened into
        // a gate by another name. A sweep must be able to reach both.
        min: 10.0,
        max: 400.0,
        step: 10.0,
        desc: "How much a teammate's angle off the aim costs against raw distance in \
               receiver scoring. Soft: there is no acceptance cone.",
    },
    TunableDef {
        id: "PASS_ELIGIBLE_MIN",
        tier: Tier::Sim,
        label: "Min receiver range",
        cat: "Passing",
        default: 20.0,
        unit: "px",
        min: 5.0,
        max: 45.0,
        step: 1.0,
        desc: "Teammates nearer than this are not pass candidates; a handoff is not a pass.",
    },
    TunableDef {
        id: "PASS_ELIGIBLE_MAX",
        tier: Tier::Sim,
        label: "Max receiver range",
        cat: "Passing",
        default: 560.0,
        unit: "px",
        min: 200.0,
        max: 1100.0,
        step: 20.0,
        desc: "Teammates further than this are not pass candidates.",
    },
    TunableDef {
        id: "PASS_ARRIVE_PACE",
        tier: Tier::Sim,
        label: "Arrival pace",
        cat: "Passing",
        default: 120.0,
        unit: "px/s",
        min: 40.0,
        max: 300.0,
        step: 10.0,
        desc: "Pace a ground pass still carries when it reaches the receiver; the \
               distance-to-speed curve's intercept.",
    },
    TunableDef {
        id: "PASS_SPEED_MIN",
        tier: Tier::Sim,
        label: "Min pass pace",
        cat: "Passing",
        default: 420.0,
        unit: "px/s",
        min: 200.0,
        max: 600.0,
        step: 10.0,
        desc: "Floor under a ground pass's launch speed, so a short ball is not a tap.",
    },
    TunableDef {
        id: "PASS_SPEED_MAX",
        tier: Tier::Sim,
        label: "Max pass pace",
        cat: "Passing",
        default: 700.0,
        unit: "px/s",
        min: 450.0,
        max: 1000.0,
        step: 10.0,
        desc: "Ceiling over a ground pass's launch speed, so a long ball is not a bullet.",
    },
    TunableDef {
        id: "PASS_LEAD_TOLERANCE",
        tier: Tier::Sim,
        label: "Lead tolerance",
        cat: "Passing",
        default: 1.0,
        unit: "x",
        // Above 1.0 is a real (bad) setting rather than padding: it promises
        // leads the receiver needs MORE than the ball's whole flight to
        // reach, which is the over-promise the flat-cap model made by
        // accident. The range has to contain it for a sweep to find the edge.
        min: 0.4,
        max: 2.0,
        step: 0.05,
        desc: "How much of the receiver's modelled arrival capability a led pass may demand, \
               as a fraction of the ball's travel time. Below 1 requires slack; above 1 \
               promises leads the receiver cannot reach.",
    },
    TunableDef {
        id: "PASS_LEAD_MIN_SPEED",
        tier: Tier::Sim,
        label: "Lead speed floor",
        cat: "Passing",
        default: 60.0,
        unit: "px/s",
        min: 0.0,
        max: 200.0,
        step: 5.0,
        desc: "Receiver speed below which a pass is played to their feet, not led.",
    },
    TunableDef {
        id: "PASS_LEAD_TIME_MIN",
        tier: Tier::Sim,
        label: "Shortest lead",
        cat: "Passing",
        default: 0.1,
        unit: "s",
        min: 0.05,
        max: 0.4,
        step: 0.05,
        desc: "Shortest candidate lead time the solver evaluates.",
    },
    TunableDef {
        id: "PASS_LEAD_TIME_MAX",
        tier: Tier::Sim,
        label: "Longest lead",
        cat: "Passing",
        default: 0.9,
        unit: "s",
        min: 0.3,
        max: 1.4,
        step: 0.05,
        desc: "Longest candidate lead time the solver evaluates.",
    },
    TunableDef {
        id: "PASS_LEAD_STEPS",
        tier: Tier::Sim,
        label: "Lead candidates",
        cat: "Passing",
        default: 5.0,
        unit: "count",
        min: 1.0,
        max: 6.0,
        step: 1.0,
        desc: "How many candidate lead times the fixed set holds. This IS the per-release \
               prediction query burst, so it is bounded rather than open.",
    },
    // Defending
    TunableDef {
        id: "AI_STEAL_CD",
        tier: Tier::Sim,
        label: "AI poke cooldown",
        cat: "Defending",
        default: 1.2,
        unit: "s",
        min: 0.4,
        max: 2.5,
        step: 0.1,
        desc: "Seconds an AI defender waits between poke tackles.",
    },
    TunableDef {
        id: "STEAL_ATTEMPT",
        tier: Tier::Sim,
        label: "AI poke range",
        cat: "Defending",
        default: 40.0,
        unit: "px",
        min: 28.0,
        max: 60.0,
        step: 2.0,
        desc: "Distance at which an AI defender attempts a poke tackle.",
    },
    TunableDef {
        id: "WHIFF_STUMBLE",
        tier: Tier::Sim,
        label: "Whiff stumble s",
        cat: "Defending",
        default: 0.3,
        unit: "s",
        min: 0.0,
        max: 0.8,
        step: 0.05,
        desc: "NOT WIRED as of #489: superseded by ACTION_TACKLE_MISS_RECOVERY, which \
               gc_sim::r#match's tackle-resolution code reads instead (see that knob's \
               desc). No sim code reads this id any more, so it fails the \
               knob-moves-metric contract (gc_sim::knob_contract) like VOLLEY_SKY_P and \
               the Replay knobs below. Left registered rather than deleted: a real \
               deletion is a separate, reviewable removal (panel wiring, sweep history).",
    },
    TunableDef {
        id: "CARRIER_SETTLE",
        tier: Tier::Sim,
        label: "AI settle touch s",
        cat: "Defending",
        default: 0.35,
        unit: "s",
        min: 0.0,
        max: 0.8,
        step: 0.05,
        desc: "Seconds an AI carrier settles the ball before acting on it.",
    },
    TunableDef {
        id: "AI_PASS_PRESSURE",
        tier: Tier::Sim,
        label: "AI pass-out range",
        cat: "Defending",
        default: 70.0,
        unit: "px",
        min: 30.0,
        max: 120.0,
        step: 5.0,
        desc: "Opponent proximity that makes an AI carrier pass out of trouble.",
    },
    // Keeper
    TunableDef {
        id: "SAVE_SPEED_REF",
        tier: Tier::Sim,
        label: "Save pace ref",
        cat: "Keeper",
        default: 1300.0,
        unit: "px/s",
        // min widened 700 -> 400: the balance search's optimum sat on the old
        // fence (docs/design/fun_metrics.md, phase 3).
        min: 400.0,
        max: 2000.0,
        step: 50.0,
        desc: "Shot pace a save is judged against; higher makes keepers softer.",
    },
    TunableDef {
        id: "CATCH_EVEN_QUALITY",
        tier: Tier::Sim,
        label: "Catch coin-flip q",
        cat: "Keeper",
        default: 0.45,
        unit: "quality",
        min: 0.2,
        max: 0.8,
        step: 0.02,
        desc: "Save quality at which a catch and a parry are equally likely.",
    },
    TunableDef {
        id: "KEEPER_RESPECT_DIST",
        tier: Tier::Sim,
        label: "Keeper ring",
        cat: "Keeper",
        default: 120.0,
        unit: "px",
        min: 60.0,
        max: 180.0,
        step: 10.0,
        desc: "Radius attackers keep off a keeper holding the ball.",
    },
    TunableDef {
        id: "KEEPER_HOLD_HUMAN",
        tier: Tier::Sim,
        label: "Keeper hold limit s",
        cat: "Keeper",
        default: 5.0,
        unit: "s",
        min: 2.0,
        max: 10.0,
        step: 0.5,
        desc: "Seconds a human-controlled keeper may hold before a forced release.",
    },
    // Keeper save fatigue (#490). The pool and its catch band: what makes
    // sustained pressure earn rebounds instead of nothing. Every range below
    // is #490's own authored suggestion; the DEFAULTS are pilot-measured
    // (`gc-sim/tests/knob_contract.rs` carries the probe table) rather than
    // taken from the middle of the range.
    //
    // The pool gates only whether the keeper can HOLD the ball. It is
    // deliberately absent from the reach/beaten decision and from the
    // `quality` scalar that feeds it -- see `gc_sim::r#match`'s `attempt_save`
    // and `tests/keeper_fatigue.rs`, which proves the reach verdict is
    // identical at a full pool and an empty one.
    TunableDef {
        id: "KEEPER_FATIGUE_MAX",
        tier: Tier::Sim,
        label: "Keeper fatigue pool",
        cat: "Keeper",
        default: 100.0,
        unit: "points",
        min: 50.0,
        max: 200.0,
        step: 5.0,
        desc: "Size of the keeper's save-fatigue pool, and the value it starts \
               every match and restart at.",
    },
    TunableDef {
        id: "KEEPER_FATIGUE_REGEN",
        tier: Tier::Sim,
        label: "Keeper fatigue regen",
        cat: "Keeper",
        default: 4.0,
        unit: "points/s",
        min: 1.0,
        max: 10.0,
        step: 0.5,
        desc: "Passive pool recovery, in points per SECOND -- integrated per fixed \
               tick, never a per-tick increment, so changing the tick rate cannot \
               silently redefine it.",
    },
    TunableDef {
        id: "KEEPER_COST_CATCH",
        tier: Tier::Sim,
        label: "Keeper catch cost",
        cat: "Keeper",
        default: 25.0,
        unit: "points",
        min: 5.0,
        max: 40.0,
        step: 1.0,
        desc: "Pool drained by a clean catch -- the most expensive save type, because \
               holding a shot is the outcome the band exists to ration.",
    },
    TunableDef {
        id: "KEEPER_COST_PARRY",
        tier: Tier::Sim,
        label: "Keeper parry cost",
        cat: "Keeper",
        default: 12.0,
        unit: "points",
        min: 5.0,
        max: 40.0,
        step: 1.0,
        desc: "Pool drained by a parry. Cheaper than a catch, so a keeper forced into \
               parrying recovers toward the catch band rather than spiralling.",
    },
    TunableDef {
        id: "KEEPER_COST_DEFLECT",
        tier: Tier::Sim,
        label: "Keeper deflect cost",
        cat: "Keeper",
        default: 8.0,
        unit: "points",
        min: 5.0,
        max: 40.0,
        step: 1.0,
        desc: "Pool drained by a fingertip deflection on a shot that beat the keeper \
               (the Tip event) -- effort spent without a save to show for it.",
    },
    TunableDef {
        id: "KEEPER_CATCH_THRESHOLD",
        tier: Tier::Sim,
        label: "Keeper catch band",
        cat: "Keeper",
        default: 45.0,
        unit: "points",
        min: 10.0,
        max: 80.0,
        step: 5.0,
        desc: "Pool level below which a clean catch stops being a legal outcome: the \
               save still happens, but it resolves as a parry with the live rebound \
               that already carries.",
    },
    TunableDef {
        id: "KEEPER_CATCH_POWER_CEILING",
        tier: Tier::Sim,
        label: "Keeper catch power cap",
        cat: "Keeper",
        // Shot pace at the moment of the save. A strike leaves the boot at
        // `stats::shot_speed` (150-650 px/s by strength) times up to
        // 1 + CHARGE_POWER, so the reachable band is roughly 150-1235 px/s and
        // this sits in its upper half: a normal strike is holdable, a
        // full-blooded one is not, however fresh the keeper is.
        default: 700.0,
        unit: "px/s",
        min: 300.0,
        max: 1400.0,
        step: 25.0,
        desc: "Shot pace above which the keeper parries regardless of fatigue. The \
               second, independent half of the catch band.",
    },
    TunableDef {
        id: "PUNT_MAX",
        tier: Tier::Sim,
        label: "Max punt range",
        cat: "Keeper",
        default: 1100.0,
        unit: "px",
        min: 680.0,
        max: 1540.0,
        step: 20.0,
        desc: "Range of a fully charged keeper punt.",
    },
    // AI
    TunableDef {
        id: "AI_SHOOT_RANGE",
        tier: Tier::Sim,
        label: "AI shoot range",
        cat: "AI",
        default: 410.0,
        unit: "px",
        min: 280.0,
        // max widened 340 -> 480 (half pitch), then rescaled 480 -> 820 with
        // the pitch itself: the balance search's optimum sat on the old fence
        // (docs/design/fun_metrics.md, phase 3).
        max: 820.0,
        step: 10.0,
        desc: "Distance from goal inside which the AI shoots.",
    },
    TunableDef {
        id: "AI_HEADER_RANGE",
        tier: Tier::Sim,
        label: "AI header range",
        cat: "AI",
        default: 340.0,
        unit: "px",
        min: 210.0,
        // max widened 300 -> 420, then rescaled 420 -> 720 with the pitch
        // itself: the balance search's optimum sat on the old fence
        // (docs/design/fun_metrics.md, phase 3).
        max: 720.0,
        step: 10.0,
        desc: "Distance from goal inside which the AI attacks a cross with its head.",
    },
    TunableDef {
        id: "CROSS_MIN_SPACE",
        tier: Tier::Sim,
        label: "Cross space need",
        cat: "AI",
        default: 30.0,
        unit: "px",
        min: 10.0,
        max: 60.0,
        step: 5.0,
        desc: "Space an AI wide player needs before it crosses.",
    },
    TunableDef {
        id: "LOOSE_MAGNET",
        tier: Tier::Sim,
        label: "Loose-ball magnet",
        cat: "AI",
        default: 90.0,
        unit: "px",
        min: 40.0,
        max: 160.0,
        step: 10.0,
        desc: "Radius inside which AI players chase a loose ball.",
    },
    TunableDef {
        id: "TRIANGLE_DIST",
        tier: Tier::Sim,
        label: "Triangle pass range",
        cat: "AI",
        default: 170.0,
        unit: "px",
        min: 120.0,
        max: 260.0,
        step: 10.0,
        desc: "Preferred distance for a short AI support pass.",
    },
    TunableDef {
        id: "STAND_WAKE",
        tier: Tier::Sim,
        label: "Positional calm",
        cat: "AI",
        default: 34.0,
        unit: "px",
        min: 16.0,
        max: 80.0,
        step: 2.0,
        desc: "Slack an AI player tolerates around its positional anchor.",
    },
    TunableDef {
        id: "AI_SPRINT_SPACE",
        tier: Tier::Sim,
        label: "Sprint into space",
        cat: "AI",
        default: 70.0,
        unit: "px",
        min: 60.0,
        max: 240.0,
        step: 10.0,
        desc: "Open space ahead that makes an AI player sprint.",
    },
    TunableDef {
        id: "AI_JUKE_DIST",
        tier: Tier::Sim,
        label: "Juke reaction range",
        cat: "AI",
        default: 44.0,
        unit: "px",
        min: 30.0,
        max: 90.0,
        step: 2.0,
        desc: "Defender proximity that triggers an AI carrier's juke.",
    },
    TunableDef {
        id: "AI_JUKE_CD",
        tier: Tier::Sim,
        label: "Juke cooldown s",
        cat: "AI",
        default: 2.0,
        unit: "s",
        min: 0.6,
        max: 5.0,
        step: 0.2,
        desc: "Seconds an AI carrier waits between jukes.",
    },
    // Committed actions (#489). Only the standing-poke tackle is wired
    // through `gc_sim::action_slot` in this PR -- see that module's doc.
    TunableDef {
        id: "ACTION_TACKLE_FULL_CHARGE",
        tier: Tier::Sim,
        label: "Tackle wind-up s",
        cat: "Defending",
        default: 0.1,
        unit: "s",
        min: 0.0,
        max: 0.3,
        step: 0.02,
        desc: "Charge duration before a standing-poke tackle becomes live. Read by \
               both a human's press and an AI presser's chase (the arrival trigger can \
               still release earlier, once in range).",
    },
    TunableDef {
        id: "ACTION_TACKLE_COMMIT",
        tier: Tier::Sim,
        label: "Tackle commit s",
        cat: "Defending",
        // 0.22 (matching the retired STAND_TIMER) measured whiff_rate at
        // 0.834 over 48 seeds of 30s AI-vs-AI matches -- deep in "defenders
        // would be expected to stop tackling" territory. 0.15, this knob's
        // own registered range floor being 0.05, was the more moderate
        // reading: still inside #489's own suggested 0.15-0.5s range, and
        // measured lower (see tests/knob_contract.rs's probe notes) without
        // reaching for the literal floor just to chase a number. It does not
        // land whiff_rate inside the proposed band either -- see that
        // metric's own doc in this file: the band is the thing still
        // unsettled, not this default.
        default: 0.15,
        unit: "s",
        // Range matches #489's own suggestion for `action.tackle.commit`.
        min: 0.05,
        max: 0.5,
        step: 0.02,
        desc: "Executing-phase duration: how long a committed poke takes to resolve. \
               Resolution is checked ONCE at the end of this window against the \
               carrier's THEN-current position, so raising this gives a carrier more \
               time to escape before the single check -- see gc_sim::action_slot and \
               tests/knob_contract.rs for the measured direction.",
    },
    TunableDef {
        id: "ACTION_TACKLE_MISS_RECOVERY",
        tier: Tier::Sim,
        label: "Tackle miss recovery s",
        cat: "Defending",
        default: 0.3,
        unit: "s",
        min: 0.15,
        max: 1.0,
        step: 0.05,
        desc: "Recovery-phase duration after a whiffed tackle (stacks AFTER \
               ACTION_TACKLE_COMMIT), during which movement is scaled by \
               ACTION_RECOVERY_CONTROL and no new action may start. Replaces the old \
               AI-only WHIFF_STUMBLE stumble: this one applies to a human's miss too.",
    },
    TunableDef {
        id: "ACTION_RECOVERY_CONTROL",
        tier: Tier::Sim,
        label: "Recovery move x",
        cat: "Defending",
        default: 0.35,
        unit: "x",
        min: 0.0,
        max: 0.5,
        step: 0.05,
        desc: "Movement-input scale while a player is recovering from a whiffed \
               committed action.",
    },
    // Replay. Presentation by nature (see the module doc's tier 2), but kept
    // in tier 1 for now because they have shipped in the F1 panel's "Replay"
    // tab since before this registry existed and moving them is a visible
    // panel change, not a refactor. Neither id is read by any `gc-sim` code:
    // `packages/render/src/replay.ts` restates both values on the TypeScript
    // side. They are the clearest existing example of a knob that cannot move
    // any metric — `tests/knob_contract.rs` uses one as the red demonstration
    // that the knob-moves-metric helper actually fails.
    TunableDef {
        id: "REPLAY_SLOWMO",
        tier: Tier::Sim,
        label: "Replay speed x",
        cat: "Replay",
        default: 0.35,
        unit: "x",
        min: 0.1,
        max: 1.0,
        step: 0.05,
        desc: "Playback rate of a goal replay. NOT WIRED into the sim: \
               presentation, and a tier-2 candidate.",
    },
    TunableDef {
        id: "REPLAY_SECONDS",
        tier: Tier::Sim,
        label: "Replay length s",
        cat: "Replay",
        default: 4.0,
        unit: "s",
        min: 2.0,
        max: 8.0,
        step: 0.5,
        desc: "Length of the replay window. NOT WIRED into the sim: \
               presentation, and a tier-2 candidate.",
    },
];

/// Every authored tier-3 AI membership band-set.
///
/// Each set is substituted as a whole (`gc_sim::tunable_registry`'s
/// `substitute_band_set`); there is deliberately no path that edits one edge.
pub static BAND_SETS: &[BandSet] = &[
    BandSet {
        id: "keeper_save_style",
        version: "v1",
        desc: "Classifies a save into smother / spread / central / stretch by \
               how close the striker is and how far the keeper must reach.",
        edges: &[
            BandEdge {
                id: "smother_distance",
                value: 26.0,
                desc: "At or inside this striker distance the keeper smothers instead of diving.",
            },
            BandEdge {
                id: "spread_distance",
                value: 78.0,
                desc: "Between the smother edge and this one the keeper spreads.",
            },
            BandEdge {
                id: "central_reach_fraction",
                value: 0.4,
                desc: "Dive distance below this fraction of reach is central, not a stretch.",
            },
        ],
    },
    BandSet {
        id: "keeper_engagement",
        version: "v1",
        desc: "Classifies a threat into contain / advance by how close it is \
               and whether a defender already owns it.",
        edges: &[
            BandEdge {
                id: "defender_handoff_distance",
                value: 120.0,
                desc: "Inside this the keeper takes the threat over from an engaged defender.",
            },
            BandEdge {
                id: "advance_threat_distance",
                value: 200.0,
                desc: "Outside this the keeper neither contains nor advances.",
            },
        ],
    },
];

/// Every registered balance metric, **in fold order**.
///
/// The fun score is a geometric mean, and floating-point multiplication is not
/// associative, so this order is load-bearing: it is the exact order the score
/// multiplies desirabilities in, and reordering it changes the score's last
/// bits. `gc_sim::metric_registry` folds in this order and
/// `tests/metric_registry.rs` pins the resulting scores.
pub static METRICS: &[MetricDef] = &[
    MetricDef {
        id: "goals_total",
        band: [0.0, 2.0, 5.0, 8.0],
        direction: MetricDirection::Banded,
        desc: "Goals scored by both sides in one match.",
    },
    MetricDef {
        id: "shots_per_goal",
        // zero_hi sits at "catastrophic", not "bad": search needs gradient out
        // here (the baseline lives around 18 — see the doc's baseline
        // signature).
        band: [1.0, 2.5, 6.0, 25.0],
        direction: MetricDirection::Banded,
        desc: "Outfield strikes at goal per goal scored.",
    },
    MetricDef {
        id: "save_rate",
        band: [0.15, 0.45, 0.75, 0.95],
        direction: MetricDirection::Banded,
        desc: "Share of on-target attempts the keeper stops.",
    },
    MetricDef {
        id: "pass_completion",
        band: [0.25, 0.55, 0.85, 1.001],
        direction: MetricDirection::Banded,
        desc: "Share of attempted passes that reach a team-mate.",
    },
    MetricDef {
        id: "turnovers_per_min",
        // SETTLED possession changes (see `gc_sim::metrics::SETTLE_HOLD`) —
        // raw ownership flicker runs ~40x higher and means nothing.
        band: [0.3, 1.0, 5.0, 10.0],
        direction: MetricDirection::Banded,
        desc: "Settled-possession turnovers per minute.",
    },
    MetricDef {
        id: "possession_balance",
        band: [0.1, 0.35, 0.65, 0.9],
        direction: MetricDirection::Banded,
        desc: "Home's share of owned ball time.",
    },
    MetricDef {
        id: "longest_drought_s",
        band: [-1.0, 0.0, 35.0, 80.0],
        direction: MetricDirection::Banded,
        desc: "Longest stretch with no goal and no clear chance, in seconds.",
    },
    MetricDef {
        id: "decided_late",
        band: [0.05, 0.4, 1.0, f64::INFINITY],
        direction: MetricDirection::Banded,
        desc: "Fraction of the match elapsed when the winner led for good.",
    },
    // APPENDED, not inserted: this list is deliberately unsorted and its fold
    // order is pinned by `gc-sim/tests/metric_registry.rs`, so inserting
    // anywhere but the end moves a hash nobody meant to move.
    //
    // #488's proposed band is 0.25-0.6 s and it is a PRIOR, not ground truth
    // -- the issue says so, and the measurement is the thing that decides.
    // The outer edges are set where a reversal stops being interesting rather
    // than by measurement: below 0.1 s a body is effectively still snapping,
    // which is the behaviour this whole rework removes, and past 1.2 s it is
    // sluggish enough that no amount of positioning payoff justifies it.
    MetricDef {
        id: "time_to_reverse",
        band: [0.1, 0.25, 0.6, 1.2],
        direction: MetricDirection::Banded,
        desc: "Mean seconds to complete a 180-degree reversal, from a run.",
    },
    // #491's two passing metrics, appended in this order. Both exist for
    // exactly #488's reason and are argued at length on
    // `gc_sim::r#match::PassShadowTally`: the ten metrics above are
    // whole-match OUTCOMES, and a 48-seed census found every one of #491's
    // eleven passing knobs DECORATION against every one of them. These
    // measure what the knobs actually do, at dozens of events per match.
    //
    // BANDS ARE PRIORS. Both are set from a 48-seed measurement at the
    // shipped defaults, widened to the range a designer would still call
    // playable, and neither has had a hands-on pilot. Say so before quoting
    // either as a target.
    //
    // THEY FOLD INTO THE FUN SCORE AS AN INTERIM STATE, SUPERSEDED BY #528.
    // Both sit well inside their own self-fit bands at defaults, so both
    // score desirability ~1.0 per match and lift the geometric mean of every
    // build regardless of balance quality -- measured at +4.10% relative on a
    // fixture where both arm and +0.46% on the all-AI control where only one
    // does (`docs/design/fun_metrics.md`'s 2026-08-13 passing entry). #528
    // puts newly-registered metrics on probation for exactly that reason:
    // reported beside the score, excluded from the mean until piloted. If you
    // are reading this and they are still folded, that is the known
    // time-boxed condition, not an oversight.
    //
    // REVIEWED, KEPT, ON A NARROWER GROUND (#531 phase 5, 2026-08-15). By the
    // time pass_completion became movable for two of eleven PASS_* knobs
    // (#531 phase 4), the original "nothing else moves" argument for these
    // two metrics only partly held. Checked per-knob rather than assumed:
    // `pass_aim_error` is still the ONLY committed contract either
    // `PASS_ANGULAR_WEIGHT` or `PASS_ELIGIBLE_MIN` has (neither clears
    // `pass_completion`); `pass_lead_time` is still the only one either
    // `PASS_LEAD_TOLERANCE` or `PASS_LEAD_MIN_SPEED` has, and per #531's own
    // adjudication it was never human-starved the way `pass_aim_error` was
    // (its `ground_releases` tally arms on any producer's ground release,
    // not only the human/bot-only `try_pass` path) -- a genuine
    // measurement-resolution case, not an AI-blindness workaround. Full
    // per-knob table in `docs/design/fun_metrics.md`'s phase-5 section. The
    // interim-state fold above is UNCHANGED by this review -- still blocked
    // on #528, still +4.10% -- only the justification for keeping the
    // metrics registered is now per-knob evidence instead of a blanket
    // claim.
    MetricDef {
        id: "pass_aim_error",
        // Chord, not radians (`gc_sim::passing`). 0.52 is 30 degrees off
        // aim, 1.0 is 60 degrees. Below `good_lo` the cone has hardened into
        // a gate — a pass that ALWAYS goes exactly where you point is a pass
        // that refuses every near-miss, which is the failure #491 deleted.
        band: [0.0, 0.15, 0.75, 1.4],
        direction: MetricDirection::Banded,
        desc: "Mean angle, in chord, between where a pass was aimed and the receiver \
               soft-cone selection chose.",
    },
    MetricDef {
        id: "pass_lead_time",
        // Seconds, averaged over every driven ground pass with an unled one
        // counted as zero. Neither fence is desirable: at 0 nothing is ever
        // played into a run, and past the candidate set's own ceiling the
        // solver is throwing the ball at where a receiver might be a second
        // later, which is a guess dressed as geometry.
        band: [0.0, 0.1, 0.6, 1.2],
        direction: MetricDirection::Banded,
        desc: "Mean seconds of lead on a driven ground pass, counting a pass to the \
               receiver's feet as zero.",
    },
    // #489's whiff_rate, appended last. PROPOSED, NOT SETTLED: the issue's own
    // words. #528 (not yet built -- see that issue) is supposed to put a
    // newly-registered metric on probation, reported beside the fun score and
    // excluded from the geometric mean until piloted, exactly like
    // `pass_aim_error`/`pass_lead_time` above SHOULD already be and are not.
    // Since that mechanism does not exist yet, this metric folds into the
    // score as an interim state on the same terms those two do -- see this
    // PR's description for the explicit call on whether that's acceptable
    // pending #528, or whether #528 should land first.
    MetricDef {
        id: "whiff_rate",
        // #489's own proposed band. Below good_lo tackles are effectively
        // free wins and the recovery cost never prices in; above good_hi
        // defenders would be expected to stop tackling altogether, which the
        // outer edge is set past rather than at, so the search still has
        // gradient out to a genuinely broken configuration.
        //
        // MEASURED AGAINST IT AND FOUND WANTING: at every value
        // ACTION_TACKLE_COMMIT/ACTION_TACKLE_FULL_CHARGE can legally take
        // (including both knobs at their registered floor), 48 seeds of 30s
        // AI-vs-AI matches measured a mean whiff_rate between 0.63 and 0.83 --
        // never inside this band, not even close at the low end. The shipped
        // defaults (tests/knob_contract.rs has the full probe table) land at
        // 0.63, the best reading found without hand-tuning STAND_REACH or
        // locomotion speeds, which is a larger, unrelated change. So this
        // band is doubly unsettled: proposed by the issue with no pilot
        // behind it AND contradicted by measurement at every point this PR's
        // knobs can reach. Left as authored rather than hand-fit to the
        // measurement, because moving a band to match whatever the code
        // currently does is how a band stops meaning anything -- the #489
        // playtest pass this issue itself calls for is where this gets
        // resolved, not a PR that is still shipping the mechanism.
        band: [0.0, 0.25, 0.55, 1.0],
        direction: MetricDirection::Banded,
        desc: "Missed standing-poke tackles / attempted standing-poke tackles \
               (attempt = a charge that reached its executing-phase resolution). \
               PROPOSED band, not measured against a pilot, and not yet met by \
               the shipped defaults -- see this def's own comment.",
    },
    // #490's rebound_rate, APPENDED LAST for the same reason every entry above
    // it was: this list is deliberately unsorted, its fold order is pinned by
    // `gc-sim/tests/metric_registry.rs`, and inserting anywhere but the end
    // moves a hash nobody meant to move.
    //
    // It folds into the fun score on exactly the terms `whiff_rate`,
    // `pass_aim_error` and `pass_lead_time` above already do -- #528's
    // probation mechanism, which would report a newly-registered metric beside
    // the score and exclude it from the geometric mean until piloted, still
    // does not exist. Same known, time-boxed condition, called out here rather
    // than discovered later.
    MetricDef {
        id: "rebound_rate",
        // #490's own proposed band, and a PRIOR rather than ground truth:
        // "below it, fatigue is invisible; above it, every save is a goalmouth
        // scramble". The outer edges are set where the reading stops being
        // interesting rather than by measurement -- 0.0 is a keeper who never
        // coughs one up (the wall this issue exists to remove) and 0.8 is a
        // goalmouth that never clears.
        band: [0.0, 0.15, 0.40, 0.8],
        direction: MetricDirection::Banded,
        desc: "Parries that put the ball back on an attacker's foot within a short \
               window, over total saves. Measures whether sustained pressure earns a \
               second chance -- PROPOSED band, no hands-on pilot behind it.",
    },
];
