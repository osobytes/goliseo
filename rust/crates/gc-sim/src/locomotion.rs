//! The locomotion primitive: how a body's speed, heading and facing change
//! in one tick.
//!
//! Every walking/running movement in the match runs through [`step`], which
//! is the generalization of the momentum helper described in
//! `docs/design/momentum.md`. Three things are new relative to that helper,
//! and all three are the reason this module exists:
//!
//! 1. **Speed is a scalar and heading is a unit vector**, eased separately.
//!    The old helper nudged a velocity *vector* toward a desired vector,
//!    which meant a hard turn shed speed as a side effect of cutting the
//!    chord — a body could never hold pace through an arc, and a reversal
//!    was a chord across the origin rather than a stop.
//! 2. **Facing is an independent target.** It was a derived read of the run
//!    velocity plus eight scattered sites that slammed it afterwards.
//!    `strafe` and `backpedal` exist *because* facing is independent: they
//!    are the angle between movement and facing, not verbs anybody sets.
//! 3. **Kinematics are derived parametrically** from a context multiplier
//!    times a stat curve ([`profile`]), so a high-pace low-technique player
//!    is fast in a straight line and wide in the corners without anyone
//!    authoring a profile for them.
//!
//! ## No new state, deliberately
//!
//! The issue that specified this work budgeted for a snapshot-layout
//! migration: a speed scalar, an independent facing and a context id added to
//! `MatchPlayer`. None of the three is needed, and adding them would have
//! been redundant state that two peers can disagree about:
//!
//! - The **speed scalar and heading** are exactly `run_vel.length()` and
//!   `run_vel.normalized()`. The only information a separate heading would
//!   add is the direction a body is facing *at zero speed* — and at zero
//!   speed the heading is free anyway (see [`Command`]'s snap rule), so
//!   there is nothing to remember.
//! - **Facing** is already a `MatchPlayer` field.
//! - The **context** is a pure function of possession, the sprint flag, the
//!   commanded throttle and the movement-versus-facing angle, all of which
//!   are already state or already inputs. Persisting it would mean two
//!   representations of one fact, and a rollback restore that disagreed with
//!   a fresh resolve would desync silently.
//!
//! So the snapshot layout, [`crate::match_snapshot::VERSION`] and the wire
//! player layout are all untouched by this change. The *values* in a replay
//! move, because the simulation moves; the *shape* does not.
//!
//! ## Purity
//!
//! Every function here is pure: same inputs, same outputs, no RNG, no I/O.
//! Easing rates are per-second quantities integrated over the caller's fixed
//! tick, so a rollback resimulation reproduces an arc exactly.

use crate::tuning::Tuning;
use gc_core::vec2::Vec2;
use gc_data::locomotion::{
    self, BACKPEDAL_ARC_KNOB, BASE_TURN_KNOB, CONTEXT_KNOBS, DIR_SNAP_SPEED_KNOB,
    FACE_EASE_FLOOR_KNOB, FACE_TURN_MULT_KNOB, JOG_THROTTLE_KNOB, LocoContext, PACE_CURVE,
    PACE_REF_HI_KNOB, PACE_REF_LO_KNOB, REVERSE_ARC_KNOB, STRAFE_ARC_KNOB, STRENGTH_CURVE,
    TECHNIQUE_CURVE, TURN_EASE_KNOB, TURN_LOW_SPEED_BONUS_KNOB,
};

/// Speed (px/s) below which [`FacingIntent::Movement`] stops tracking the
/// heading and holds the last facing instead.
///
/// This is the old `RUN_VEL_FACE_MIN`: a stopped player must be able to aim
/// without their facing collapsing to zero the moment they stand still.
pub const MOVEMENT_FACE_MIN_SPEED: f64 = 20.0;

/// A body's kinematic state, as the primitive reads and writes it.
///
/// Both fields live on `MatchPlayer` already; this is the projection [`step`]
/// operates on so the primitive is testable without constructing a match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kinematics {
    /// Locomotion velocity, px/s. Its length is the speed scalar and its
    /// direction is the movement heading.
    pub run_vel: Vec2,
    /// Facing direction (unit, or zero before the first tick).
    pub facing: Vec2,
}

/// Where a body wants to look this tick, independent of where it is moving.
///
/// This enum is what replaced the eight sites that used to assign
/// `p.facing` immediately after the locomotion call. A consumer that wants a
/// stance — jockey today, defensive contain later — expresses it as a
/// facing target and never touches the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FacingIntent {
    /// Track the movement heading while actually moving, hold otherwise.
    Movement,
    /// Do not rotate at all this tick.
    Hold,
    /// Rotate toward this direction; a zero vector means [`Self::Hold`].
    Toward(Vec2),
}

/// What the caller is asking the body to do this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Command {
    /// Desired locomotion velocity in px/s, situationally scaled (stun,
    /// wind-up, combat, arrival easing, species burst) but **without** any
    /// context multiplier — the primitive applies that. A zero vector means
    /// "stop": the body brakes and keeps its heading.
    pub vel: Vec2,
    /// Whether the ball is at this body's feet, for context resolution.
    pub carrying: bool,
    /// Whether the sprint burst is engaged, for context resolution.
    pub sprinting: bool,
    /// Where the body wants to look.
    pub facing: FacingIntent,
}

/// Normalized stat inputs to the kinematic curves, each in `0..1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    /// Normalized pace: scales top speed and acceleration.
    pub pace: f64,
    /// Normalized strength: scales deceleration.
    pub strength: f64,
    /// Normalized technique: scales angular rate.
    pub technique: f64,
}

/// One context's derived kinematic parameters for one player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    /// The context these parameters were derived for.
    pub ctx: LocoContext,
    /// Top speed, px/s.
    pub top_speed: f64,
    /// Acceleration from a standstill, px/s^2.
    pub accel_start: f64,
    /// Acceleration once the run is built, px/s^2.
    pub accel_run: f64,
    /// Deceleration, px/s^2.
    pub decel: f64,
    /// Bounded angular rate for the movement heading, rad/s.
    pub turn_rate: f64,
    /// Bounded angular rate for facing, rad/s.
    pub face_rate: f64,
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}

fn curve(t: f64, lo: f64, hi: f64) -> f64 {
    lo + (hi - lo) * clamp01(t)
}

/// Normalize a player's concrete scalars into curve inputs.
///
/// `move_speed` is the player's authored base speed in px/s, which already
/// carries pace through `stats::move_speed`; the pace window maps it back to
/// `0..1` so the curve endpoints are readable as "slowest player" and
/// "fastest player" rather than as px/s. `dribble` stands in for technique:
/// it is the canonical touch-quality scalar `MatchPlayer` retains, and
/// turning tightly is a touch act.
#[must_use]
pub fn stats(move_speed: f64, strength: f64, dribble: f64, tune: &Tuning) -> Stats {
    let lo = tune.value(PACE_REF_LO_KNOB);
    let hi = tune.value(PACE_REF_HI_KNOB);
    let span = hi - lo;
    let pace = if span > 0.0 {
        clamp01((move_speed - lo) / span)
    } else {
        0.5
    };
    Stats {
        pace,
        strength: clamp01(strength),
        technique: clamp01(dribble),
    }
}

/// The fastest a body may travel `distance` from a target and still come to
/// rest on it, given `decel`.
///
/// `v = sqrt(2 * a * d)`, the standard stopping-distance identity inverted.
/// Steering AI that commands full speed right up to its target arrives with
/// speed it then has to shed *past* the target — the "aim for arrival, not
/// position" fix momentum makes mandatory rather than optional.
#[must_use]
pub fn arrival_speed(distance: f64, decel: f64) -> f64 {
    if distance <= 0.0 || decel <= 0.0 {
        return 0.0;
    }
    (2.0 * decel * distance).sqrt()
}

fn context_knobs(ctx: LocoContext) -> &'static locomotion::ContextKnobs {
    CONTEXT_KNOBS
        .iter()
        .find(|k| k.ctx == ctx)
        .expect("every LocoContext has a registered knob row")
}

/// Derive one context's kinematic parameters for one player.
///
/// The four formulas, in one place:
///
/// ```text
/// top_speed = ctx.top_mult   * base_speed
/// accel     = ctx.accel_mult * base_accel * pace_curve(pace)
/// decel     = ctx.decel_mult * base_decel * strength_curve(strength)
/// turn_rate = ctx.turn_mult  * base_turn  * technique_curve(technique)
/// ```
///
/// **One deliberate deviation from the issue's formula.** It writes
/// `top_speed = ctx.speed_mult * base_speed * pace_curve(pace)`. Our
/// `base_speed` is `MatchPlayer::move_speed`, which `stats::move_speed`
/// already computes as `BASE_MOVE + pace * MOVE_PER_PACE` — a pace curve
/// applied to a base speed, in exactly those words. Multiplying by a second
/// pace curve would count pace into top speed twice and quietly restretch
/// every authored player's speed. Pace still reaches the profile, through
/// acceleration, which is where it was missing.
///
/// `base_accel`/`base_decel` are the shipped `MOVE_ACCEL`/`MOVE_DECEL`
/// knobs, and `START_ACCEL` supplies the standing-start end of the
/// acceleration blend that `docs/design/momentum.md` introduced — reusing
/// them rather than minting `LOCO_BASE_ACCEL` keeps one knob per quantity
/// instead of two that would have to agree.
#[must_use]
pub fn profile(ctx: LocoContext, base_speed: f64, st: Stats, tune: &Tuning) -> Profile {
    let knobs = context_knobs(ctx);
    let pace_curve = curve(
        st.pace,
        tune.value(PACE_CURVE.lo),
        tune.value(PACE_CURVE.hi),
    );
    let strength_curve = curve(
        st.strength,
        tune.value(STRENGTH_CURVE.lo),
        tune.value(STRENGTH_CURVE.hi),
    );
    let technique_curve = curve(
        st.technique,
        tune.value(TECHNIQUE_CURVE.lo),
        tune.value(TECHNIQUE_CURVE.hi),
    );
    let turn =
        tune.value(knobs.turn_mult) * tune.value(BASE_TURN_KNOB).to_radians() * technique_curve;
    Profile {
        ctx,
        top_speed: tune.value(knobs.top_mult) * base_speed,
        accel_start: tune.value(knobs.accel_mult) * tune.value("START_ACCEL") * pace_curve,
        accel_run: tune.value(knobs.accel_mult) * tune.value("MOVE_ACCEL") * pace_curve,
        decel: tune.value(knobs.decel_mult) * tune.value("MOVE_DECEL") * strength_curve,
        turn_rate: turn,
        face_rate: turn * tune.value(FACE_TURN_MULT_KNOB),
    }
}

/// Resolve which context a body is moving under this tick.
///
/// Geometry wins over possession: a body moving broadly across or against
/// where it is looking is strafing or backpedalling whether or not it has the
/// ball, because that is a fact about the body rather than about the ball.
/// Possession then beats the speed ladder, so a carrier is never classified
/// as an empty-handed sprinter.
///
/// `move_dir` should be the commanded direction when there is one and the
/// current heading otherwise; `face_target` is where the body is being asked
/// to look. Either being zero means the angle is undefined and the geometric
/// contexts cannot apply.
#[must_use]
pub fn resolve(
    throttle: f64,
    carrying: bool,
    sprinting: bool,
    move_dir: Vec2,
    face_target: Vec2,
    tune: &Tuning,
) -> LocoContext {
    let moving = throttle > 0.0 && (move_dir.x != 0.0 || move_dir.y != 0.0);
    let looking = face_target.x != 0.0 || face_target.y != 0.0;
    if moving && looking {
        let cos = dot(move_dir.normalized(), face_target.normalized());
        if cos < tune.value(BACKPEDAL_ARC_KNOB).to_radians().cos() {
            return LocoContext::Backpedal;
        }
        if cos < tune.value(STRAFE_ARC_KNOB).to_radians().cos() {
            return LocoContext::Strafe;
        }
    }
    if carrying {
        return if sprinting {
            LocoContext::SprintCarry
        } else {
            LocoContext::Carry
        };
    }
    if sprinting {
        return LocoContext::Sprint;
    }
    if throttle < tune.value(JOG_THROTTLE_KNOB) {
        LocoContext::Jog
    } else {
        LocoContext::Run
    }
}

/// Rotate `from` toward `to` by at most `max_step` radians.
fn rotate_toward(from: Vec2, to: Vec2, max_step: f64) -> Vec2 {
    let angle = signed_angle(from, to);
    let magnitude = angle.abs();
    if magnitude <= max_step {
        return to;
    }
    rotate(from, max_step * angle.signum())
}

/// Signed angle from `from` to `to`, in `-pi..pi`.
fn signed_angle(from: Vec2, to: Vec2) -> f64 {
    let cross = from.x * to.y - from.y * to.x;
    (cross).atan2(dot(from, to))
}

fn rotate(v: Vec2, angle: f64) -> Vec2 {
    let (s, c) = (angle.sin(), angle.cos());
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Bounded-rate rotation with a damped ease-out below `ease`.
///
/// Above the threshold the body turns at the full rate. Below it the rate
/// falls off with the remaining angle, so the character glides into the
/// heading instead of hitting it and stopping dead. Two properties matter and
/// both are structural rather than tuned:
///
/// - **No overshoot, therefore no terminal oscillation.** The step is clamped
///   to the remaining angle, so the angle is monotonically non-increasing and
///   can never change sign. This holds at every value of every knob.
/// - **It lands.** A pure proportional ease is asymptotic; `floor` puts a
///   lower bound under the rate so the remaining angle reaches zero in
///   finite time rather than shrinking forever.
fn ease_toward(current: Vec2, want: Vec2, rate: f64, ease: f64, floor: f64, dt: f64) -> Vec2 {
    if current.x == 0.0 && current.y == 0.0 {
        return want;
    }
    let current = current.normalized();
    let want = want.normalized();
    let angle = signed_angle(current, want);
    let magnitude = angle.abs();
    if magnitude == 0.0 {
        return want;
    }
    let effective = if ease > 0.0 && magnitude < ease {
        rate * (magnitude / ease).max(clamp01(floor))
    } else {
        rate
    };
    let step = (effective * dt).min(magnitude);
    if step >= magnitude {
        return want;
    }
    rotate(current, step * angle.signum())
}

/// One tick of locomotion: the five ordered steps.
///
/// 1. The caller resolved the context (see [`resolve`]) and 2. derived its
///    profile (see [`profile`]); both are inputs here so a caller can inspect
///    or override them.
/// 3. Speed eases toward the context's target. **Acceleration and
///    deceleration are separate rates.**
/// 4. The movement heading eases toward the commanded direction at the
///    context's bounded angular rate — an arc at speed, free at rest.
/// 5. Facing eases toward its own target at its own rate.
///
/// ### Reversal is a stop, not a sign flip
///
/// When the commanded direction opposes the current heading by more than
/// `LOCO_REVERSE_ARC_DEG`, the target speed becomes zero and the heading is
/// held. The body brakes at the context's deceleration rate until it is at
/// rest, and only then adopts the new heading and accelerates. The velocity
/// therefore passes through exactly zero: there is no tick on which it jumps
/// from one sign to the other, at any tick rate and any knob value.
#[must_use]
pub fn step(k: Kinematics, cmd: &Command, prof: &Profile, dt: f64, tune: &Tuning) -> Kinematics {
    let speed0 = k.run_vel.length();
    let commanded = cmd.vel.length();
    let cmd_dir = cmd.vel.normalized();
    let snap = tune.value(DIR_SNAP_SPEED_KNOB);
    // The heading at zero speed is undefined, and free: adopt the command.
    let dir0 = if speed0 > 0.0 {
        k.run_vel.normalized()
    } else {
        cmd_dir
    };
    // Step 3/4 target selection. The context multiplier scales the caller's
    // situational command; `commanded` already carries stun, wind-up, combat
    // and arrival easing.
    let target_full = commanded * tune.value(context_knobs(prof.ctx).top_mult);
    let mut dir = dir0;
    let mut target = 0.0;
    let mut reversing = false;
    if commanded > 0.0 {
        if speed0 <= snap {
            dir = cmd_dir;
            target = target_full;
        } else if dot(dir0, cmd_dir) < tune.value(REVERSE_ARC_KNOB).to_radians().cos() {
            reversing = true;
        } else {
            // Near-zero-speed turns are cheap; a body at pace traces an arc.
            let low_speed = 1.0 - clamp01(speed0 / prof.top_speed.max(1.0));
            let bonus = 1.0 + tune.value(TURN_LOW_SPEED_BONUS_KNOB) * low_speed;
            dir = rotate_toward(dir0, cmd_dir, prof.turn_rate * bonus * dt);
            target = target_full;
        }
    }
    // Step 3: separate accel and decel rates.
    let speed = if target > speed0 {
        let momentum = clamp01(speed0 / prof.top_speed.max(1.0));
        let accel = prof.accel_start + (prof.accel_run - prof.accel_start) * momentum;
        (speed0 + accel * dt).min(target)
    } else {
        (speed0 - prof.decel * dt).max(target)
    };
    // A reversal pivots only at rest. Braking that would take the body below
    // the snap speed lands it at exactly zero and turns there, so the
    // velocity crosses the origin rather than jumping across it.
    let (speed, dir) = if reversing && speed <= snap {
        (0.0, cmd_dir)
    } else {
        (speed, dir)
    };
    // Step 5: facing, on its own target and its own rate.
    let want = match cmd.facing {
        FacingIntent::Hold => k.facing,
        FacingIntent::Toward(v) if v.x != 0.0 || v.y != 0.0 => v,
        FacingIntent::Toward(_) => k.facing,
        FacingIntent::Movement => {
            if speed > MOVEMENT_FACE_MIN_SPEED {
                dir
            } else {
                k.facing
            }
        }
    };
    let facing = if want.x == 0.0 && want.y == 0.0 {
        k.facing
    } else {
        ease_toward(
            k.facing,
            want,
            prof.face_rate,
            tune.value(TURN_EASE_KNOB).to_radians(),
            tune.value(FACE_EASE_FLOOR_KNOB),
            dt,
        )
    };
    Kinematics {
        run_vel: dir.scale(speed),
        facing,
    }
}
