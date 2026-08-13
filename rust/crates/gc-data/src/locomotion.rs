//! Authored locomotion content: the seven movement contexts, the knob ids
//! that supply each context's kinematic multipliers, and the stat-curve
//! control points those multipliers are scaled by.
//!
//! Data, not mechanism (AGENTS.md §8). Nothing here is a function over game
//! state: the tables name knobs and contexts, and `gc_sim::locomotion` owns
//! the pure derivation that reads them. The multiplier *values* are authored
//! next door in [`crate::tunables::SIM_TUNABLES`] so a sweep can enumerate
//! them like any other tier-1 scalar — this module is the map from a context
//! to the four knobs that describe it.
//!
//! ## Why a context is not a verb
//!
//! `strafe` and `backpedal` are not buttons and not states anybody sets. They
//! fall out of the angle between where a body is *moving* and where it is
//! *looking*, which is only expressible because facing is an independent
//! target rather than a read of the velocity. A future defensive contain
//! stance is therefore a facing rule layered on this primitive, not an eighth
//! context.
//!
//! ## Knob ids are flat SCREAMING_SNAKE, deliberately
//!
//! `gc_sim::tuning`'s `parse_knob_line` accepts only `[A-Za-z0-9_]` in a key
//! and *skips malformed lines without erroring*. `gc_sim::knob_contract`
//! perturbs a knob by round-tripping `"<id>=<value>"` through exactly that
//! parser, so a dotted id such as `loco.run.accel` would be silently dropped,
//! the perturbed run would be identical to the defaults run, and every
//! knob-moves-metric assertion would report the knob as decoration. The
//! registry's own parser does accept dots, so the knob would look perfectly
//! registered in the F1 panel while being unreachable from a contract test.

/// Which kinematic profile a body is moving under this tick.
///
/// Resolved fresh every tick by `gc_sim::locomotion::resolve` from ball
/// possession, the sprint flag, the commanded throttle and the angle between
/// commanded movement and the facing target. It is a *derivation*, not
/// retained state, which is why it is absent from the match snapshot: two
/// peers that agree on the inputs agree on the context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocoContext {
    /// Loose, low-throttle movement: cheap to turn, quick to stop.
    Jog,
    /// The neutral profile every multiplier is expressed relative to.
    Run,
    /// The sprint burst: fastest, heaviest, widest in the corners.
    Sprint,
    /// Moving with the ball at the feet.
    Carry,
    /// Sprinting with the ball at the feet — slower and heavier than a free
    /// sprint, because the touch has to keep up.
    SprintCarry,
    /// Moving broadly across the facing target.
    Strafe,
    /// Moving broadly opposite the facing target.
    Backpedal,
}

impl LocoContext {
    /// Every context, in registration order.
    pub const ALL: [LocoContext; 7] = [
        LocoContext::Jog,
        LocoContext::Run,
        LocoContext::Sprint,
        LocoContext::Carry,
        LocoContext::SprintCarry,
        LocoContext::Strafe,
        LocoContext::Backpedal,
    ];

    /// Stable wire/report tag for this context.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            LocoContext::Jog => "jog",
            LocoContext::Run => "run",
            LocoContext::Sprint => "sprint",
            LocoContext::Carry => "carry",
            LocoContext::SprintCarry => "sprint_carry",
            LocoContext::Strafe => "strafe",
            LocoContext::Backpedal => "backpedal",
        }
    }
}

/// The four knob ids that describe one context's kinematics.
///
/// Every id names a *multiplier* over a shared base, not an absolute rate:
/// per-character differentiation comes from the stat curves below, so a
/// context table stays a handful of numbers instead of one profile per
/// character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextKnobs {
    /// The context these knobs describe.
    pub ctx: LocoContext,
    /// Multiplier over the player's base move speed.
    pub top_mult: &'static str,
    /// Multiplier over the shared acceleration base.
    pub accel_mult: &'static str,
    /// Multiplier over the shared deceleration base.
    pub decel_mult: &'static str,
    /// Multiplier over the shared angular-rate base, for heading and facing.
    pub turn_mult: &'static str,
}

/// Each context's knob ids, in [`LocoContext::ALL`] order.
///
/// `Sprint`'s top-speed multiplier is the already-shipped `SPRINT_MULT`
/// rather than a new `LOCO_SPRINT_TOP_MULT`. Sprint's top speed had exactly
/// one knob before this table existed; adding a second would either
/// double-count it or leave one of the two reading as decoration, and
/// `SPRINT_MULT` is the one the F1 panel and `tuning_presets` already ship.
pub static CONTEXT_KNOBS: &[ContextKnobs] = &[
    ContextKnobs {
        ctx: LocoContext::Jog,
        top_mult: "LOCO_JOG_TOP_MULT",
        accel_mult: "LOCO_JOG_ACCEL_MULT",
        decel_mult: "LOCO_JOG_DECEL_MULT",
        turn_mult: "LOCO_JOG_TURN_MULT",
    },
    ContextKnobs {
        ctx: LocoContext::Run,
        top_mult: "LOCO_RUN_TOP_MULT",
        accel_mult: "LOCO_RUN_ACCEL_MULT",
        decel_mult: "LOCO_RUN_DECEL_MULT",
        turn_mult: "LOCO_RUN_TURN_MULT",
    },
    ContextKnobs {
        ctx: LocoContext::Sprint,
        top_mult: "SPRINT_MULT",
        accel_mult: "LOCO_SPRINT_ACCEL_MULT",
        decel_mult: "LOCO_SPRINT_DECEL_MULT",
        turn_mult: "LOCO_SPRINT_TURN_MULT",
    },
    ContextKnobs {
        ctx: LocoContext::Carry,
        top_mult: "LOCO_CARRY_TOP_MULT",
        accel_mult: "LOCO_CARRY_ACCEL_MULT",
        decel_mult: "LOCO_CARRY_DECEL_MULT",
        turn_mult: "LOCO_CARRY_TURN_MULT",
    },
    ContextKnobs {
        ctx: LocoContext::SprintCarry,
        top_mult: "LOCO_SPRINT_CARRY_TOP_MULT",
        accel_mult: "LOCO_SPRINT_CARRY_ACCEL_MULT",
        decel_mult: "LOCO_SPRINT_CARRY_DECEL_MULT",
        turn_mult: "LOCO_SPRINT_CARRY_TURN_MULT",
    },
    ContextKnobs {
        ctx: LocoContext::Strafe,
        top_mult: "LOCO_STRAFE_TOP_MULT",
        accel_mult: "LOCO_STRAFE_ACCEL_MULT",
        decel_mult: "LOCO_STRAFE_DECEL_MULT",
        turn_mult: "LOCO_STRAFE_TURN_MULT",
    },
    ContextKnobs {
        ctx: LocoContext::Backpedal,
        top_mult: "LOCO_BACKPEDAL_TOP_MULT",
        accel_mult: "LOCO_BACKPEDAL_ACCEL_MULT",
        decel_mult: "LOCO_BACKPEDAL_DECEL_MULT",
        turn_mult: "LOCO_BACKPEDAL_TURN_MULT",
    },
];

/// A linear stat curve, as two registered control points.
///
/// The curve maps a normalized stat in `0..1` to a multiplier: `lo` is the
/// output at stat 0, `hi` the output at stat 1. Linear is the deliberate
/// starting point — whether pace should have diminishing returns at the top
/// end is a sweep question, and registering the endpoints is what lets the
/// sweep answer it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatCurveKnobs {
    /// Knob id for the multiplier at normalized stat 0.
    pub lo: &'static str,
    /// Knob id for the multiplier at normalized stat 1.
    pub hi: &'static str,
}

/// Pace curve: scales acceleration and top speed.
pub static PACE_CURVE: StatCurveKnobs = StatCurveKnobs {
    lo: "LOCO_PACE_CURVE_LO",
    hi: "LOCO_PACE_CURVE_HI",
};

/// Strength curve: scales deceleration (braking is a strength act).
pub static STRENGTH_CURVE: StatCurveKnobs = StatCurveKnobs {
    lo: "LOCO_STRENGTH_CURVE_LO",
    hi: "LOCO_STRENGTH_CURVE_HI",
};

/// Technique curve: scales angular rate (turning tightly is a touch act).
pub static TECHNIQUE_CURVE: StatCurveKnobs = StatCurveKnobs {
    lo: "LOCO_TECHNIQUE_CURVE_LO",
    hi: "LOCO_TECHNIQUE_CURVE_HI",
};

/// Knob id for the shared angular-rate base, in degrees per second.
pub const BASE_TURN_KNOB: &str = "LOCO_BASE_TURN";
/// Knob id for the facing rate's multiplier over the heading rate.
pub const FACE_TURN_MULT_KNOB: &str = "LOCO_FACE_TURN_MULT";
/// Knob id for the remaining facing angle at which the damped ease-out takes
/// over from bounded-rate rotation, in degrees.
///
/// Consumed as a CHORD length after `to_radians()` (an exact multiply), for
/// the reason above: chord and arc agree to better than half a percent over
/// this knob's whole declared range, and the alternative is a transcendental
/// on the hot path.
pub const TURN_EASE_KNOB: &str = "LOCO_TURN_EASE_DEG";
/// Knob id for the floor under the damped ease-out's rate, as a fraction of
/// the full facing rate. Without a floor the ease is asymptotic and never
/// lands.
pub const FACE_EASE_FLOOR_KNOB: &str = "LOCO_FACE_EASE_FLOOR";
// The three arc thresholds below are authored as **cosines**, not degrees,
// and that is a determinism requirement rather than a style choice. Each is
// compared against a dot product once per player per tick; expressed in
// degrees, every comparison would need a `cos()` call on the simulation's
// hot path. `sin`/`cos`/`atan2` are not correctly-rounded, and Rust links a
// different libm for `wasm32-unknown-unknown` than the host libm a native
// build uses -- so a native peer and a browser peer would disagree in the low
// bits and a rollback resimulation would desync. Measured, not theorized: an
// earlier draft of this module used `to_radians().cos()` here and the
// compiled wasm module diverged from the native build at boundary 12 of the
// OMP-1 fixture. See `gc_sim::locomotion`'s module doc.

/// Knob id for the cosine of the commanded-versus-current angle beyond which
/// a direction change is a reversal (brake through zero) rather than an arc.
/// `-0.5` is 120 degrees.
pub const REVERSE_ARC_COS_KNOB: &str = "LOCO_REVERSE_ARC_COS";
/// Knob id for the cosine of the movement-versus-facing angle beyond which a
/// context is a strafe. `0.57` is about 55 degrees.
pub const STRAFE_ARC_COS_KNOB: &str = "LOCO_STRAFE_ARC_COS";
/// Knob id for the cosine of the movement-versus-facing angle beyond which a
/// context is a backpedal. `-0.5` is 120 degrees.
pub const BACKPEDAL_ARC_COS_KNOB: &str = "LOCO_BACKPEDAL_ARC_COS";
/// Knob id for the extra angular rate available at a standstill, as a
/// fraction of the context rate. Near-zero-speed turns are cheap.
pub const TURN_LOW_SPEED_BONUS_KNOB: &str = "LOCO_TURN_LOW_SPEED_BONUS";
/// Knob id for the speed below which the heading adopts the command outright
/// rather than arcing toward it, in px/s.
pub const DIR_SNAP_SPEED_KNOB: &str = "LOCO_DIR_SNAP_SPEED";
/// Knob id for the commanded throttle below which movement is a jog.
pub const JOG_THROTTLE_KNOB: &str = "LOCO_JOG_THROTTLE";
/// Knob id for the low end of the pace normalization window, in px/s.
pub const PACE_REF_LO_KNOB: &str = "LOCO_PACE_REF_LO";
/// Knob id for the high end of the pace normalization window, in px/s.
pub const PACE_REF_HI_KNOB: &str = "LOCO_PACE_REF_HI";
