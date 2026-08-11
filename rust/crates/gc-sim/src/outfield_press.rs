//! Serializable team-owned presser state and pure pressing geometry. Match
//! owns world eligibility and trigger inputs; this module owns stable
//! arbitration, conservative tuning, and the compact versioned value
//! contract.

use crate::brain;
use gc_core::vec2::Vec2;

/// This state's format version.
pub const VERSION: u32 = 1;

const PRESSER_SWITCH_RATIO: f64 = 0.15;
const LOW_DISCIPLINE_THRESHOLD: f64 = 0.35;
const COVER_SUPPORT_DISTANCE: f64 = 140.0;
const CONTAIN_CENTER_BIAS: f64 = 8.0;
const CONTAIN_SLOW_RADIUS: f64 = 60.0;
const LANE_SHADOW_WEIGHT: f64 = 0.35;
const LANE_POINT_FRACTION: f64 = 0.55;

/// The team-level pressing mode: whether anyone is pressing at all, and if
/// so, how hard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StablePressMode {
    /// Nobody is pressing.
    Inactive,
    /// A presser is shadowing without committing.
    Contain,
    /// A presser has committed to winning the ball.
    Commit,
}

/// The team's serializable pressing state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldPressState {
    /// This state's format version.
    pub version: u32,
    /// The current presser, if any.
    pub presser_index: Option<u32>,
    /// The team-level pressing mode.
    pub mode: StablePressMode,
    /// Why the current mode was chosen (see [`brain::press_mode`]).
    pub reason: brain::PressReason,
}

/// One candidate for [`assign_presser`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldPressCandidate {
    /// The candidate's stable player identity.
    pub player_index: u32,
    /// Non-negative cost; lower wins.
    pub distance_cost: f64,
    /// `Some(false)` excludes the candidate; `None`/`Some(true)` include it.
    pub eligible: Option<bool>,
}

/// One candidate for [`highest_scored_lane`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldLaneCandidate {
    /// The candidate's stable player identity.
    pub player_index: u32,
    /// The candidate's score; higher is more attractive.
    pub score: f64,
    /// The candidate's position.
    pub pos: Vec2,
    /// `Some(false)` excludes the candidate; `None`/`Some(true)` include it.
    pub eligible: Option<bool>,
}

/// Inputs to [`resolve`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldPressContext {
    /// The carrier just took a heavy touch.
    pub heavy_touch: bool,
    /// The ball is loose and immediately winnable.
    pub exposed_ball: bool,
    /// A teammate is covering behind the presser.
    pub cover_available: bool,
    /// The team is desperate enough to commit regardless of discipline.
    pub box_desperation: bool,
    /// This team's current press discipline, in `[0, 1]`.
    pub press_discipline: f64,
}

fn is_commit_reason(reason: brain::PressReason) -> bool {
    !matches!(reason, brain::PressReason::NoTrigger)
}

fn validate_state(state: &OutfieldPressState) {
    assert_eq!(state.version, VERSION, "unsupported outfield press version");
    if let Some(presser_index) = state.presser_index {
        assert!(
            presser_index >= 1,
            "outfield presser index must be a positive integer"
        );
    }
    match state.mode {
        StablePressMode::Inactive => {
            assert!(
                state.presser_index.is_none(),
                "inactive outfield press cannot retain a presser"
            );
            assert_eq!(
                state.reason,
                brain::PressReason::NoTrigger,
                "inactive outfield press cannot retain a commit reason"
            );
        }
        StablePressMode::Contain => {
            assert!(
                state.presser_index.is_some(),
                "contain outfield press requires a presser"
            );
            assert_eq!(
                state.reason,
                brain::PressReason::NoTrigger,
                "contain outfield press cannot retain a commit reason"
            );
        }
        StablePressMode::Commit => {
            assert!(
                state.presser_index.is_some(),
                "commit outfield press requires a presser"
            );
            assert!(
                is_commit_reason(state.reason),
                "commit outfield press requires a commit reason"
            );
        }
    }
}

/// A fresh, inactive press state.
#[must_use]
pub fn new_state() -> OutfieldPressState {
    OutfieldPressState {
        version: VERSION,
        presser_index: None,
        mode: StablePressMode::Inactive,
        reason: brain::PressReason::NoTrigger,
    }
}

/// Drop back to inactive, unless already there.
#[must_use]
pub fn clear(state: &OutfieldPressState) -> OutfieldPressState {
    validate_state(state);
    if state.mode == StablePressMode::Inactive {
        return *state;
    }
    new_state()
}

/// Validate and defensively copy a press state.
#[must_use]
pub fn copy_state(state: &OutfieldPressState) -> OutfieldPressState {
    validate_state(state);
    *state
}

/// Resolve this tick's press mode and reason for the given presser.
#[must_use]
pub fn resolve(presser_index: u32, context: &OutfieldPressContext) -> OutfieldPressState {
    let (mode, reason) = brain::press_mode(&brain::BrainPressContext {
        heavy_touch: context.heavy_touch,
        exposed_ball: context.exposed_ball,
        cover_available: context.cover_available,
        box_desperation: context.box_desperation,
        press_discipline: context.press_discipline,
        low_discipline_threshold: LOW_DISCIPLINE_THRESHOLD,
    });
    let state = OutfieldPressState {
        version: VERSION,
        presser_index: Some(presser_index),
        mode: match mode {
            brain::PressMode::Commit => StablePressMode::Commit,
            brain::PressMode::Contain => StablePressMode::Contain,
        },
        reason,
    };
    copy_state(&state)
}

/// Build a contain state for the given presser, with no commit reason.
#[must_use]
pub fn contain(presser_index: u32) -> OutfieldPressState {
    let state = OutfieldPressState {
        version: VERSION,
        presser_index: Some(presser_index),
        mode: StablePressMode::Contain,
        reason: brain::PressReason::NoTrigger,
    };
    copy_state(&state)
}

/// Pick the presser to commit, delegating ranking and hysteresis to
/// [`brain::assign_presser`].
#[must_use]
pub fn assign_presser(candidates: &[OutfieldPressCandidate], current: Option<u32>) -> Option<u32> {
    let mapped: Vec<brain::PresserCandidate> = candidates
        .iter()
        .map(|c| brain::PresserCandidate {
            player_index: c.player_index,
            distance_cost: c.distance_cost,
            eligible: c.eligible,
        })
        .collect();
    brain::assign_presser(&mapped, current, PRESSER_SWITCH_RATIO)
}

/// Whether a covering teammate is close enough, on the goal side, to make a
/// commit safe.
#[must_use]
pub fn cover_available(distance: f64, goal_side: bool) -> bool {
    assert!(
        distance.is_finite() && distance >= 0.0,
        "cover support distance must be non-negative"
    );
    goal_side && distance <= COVER_SUPPORT_DISTANCE
}

/// The contain presser's movement speed: slowed by `contain_factor` only
/// inside the contain-slow radius.
#[must_use]
pub fn contain_speed(base_speed: f64, target_distance: f64, contain_factor: f64) -> f64 {
    assert!(
        base_speed.is_finite() && base_speed >= 0.0,
        "contain base speed must be non-negative"
    );
    assert!(
        target_distance.is_finite() && target_distance >= 0.0,
        "contain target distance must be non-negative"
    );
    assert!(
        contain_factor.is_finite(),
        "contain speed factor must be finite"
    );
    if target_distance <= CONTAIN_SLOW_RADIUS {
        let clamped = contain_factor.clamp(0.0, 1.0);
        return base_speed.min(base_speed * clamped);
    }
    base_speed
}

/// A contain target that holds goal-side of the carrier at `standoff`
/// distance, biased a fixed amount toward pitch centre.
#[must_use]
pub fn contain_target(carrier: Vec2, goal: Vec2, field_height: f64, standoff: f64) -> Vec2 {
    let target = carrier.add(goal.sub(carrier).normalized().scale(standoff));
    let center_delta = field_height / 2.0 - carrier.y;
    if center_delta == 0.0 {
        return target;
    }
    let bias = if center_delta > 0.0 { 1.0 } else { -1.0 } * CONTAIN_CENTER_BIAS;
    Vec2::new(target.x, target.y + bias)
}

/// The eligible lane candidate with the highest score, ties broken by the
/// lower player index.
#[must_use]
pub fn highest_scored_lane(candidates: &[OutfieldLaneCandidate]) -> Option<OutfieldLaneCandidate> {
    let mut best: Option<OutfieldLaneCandidate> = None;
    for candidate in candidates {
        assert!(
            candidate.player_index >= 1,
            "passing lane candidate player must be a positive integer"
        );
        assert!(
            candidate.score.is_finite(),
            "passing lane candidate score must be finite"
        );
        if candidate.eligible == Some(false) {
            continue;
        }
        let replace = match best {
            None => true,
            Some(current) => {
                candidate.score > current.score
                    || (candidate.score == current.score
                        && candidate.player_index < current.player_index)
            }
        };
        if replace {
            best = Some(*candidate);
        }
    }
    best
}

/// Shade a HELD position toward the highest-valued lane's point (the fixed
/// fraction of the way from the carrier to the receiver).
#[must_use]
pub fn lane_shadow_target(base: Vec2, carrier: Vec2, candidates: &[OutfieldLaneCandidate]) -> Vec2 {
    let receiver = match highest_scored_lane(candidates) {
        Some(receiver) => receiver,
        None => return base,
    };
    let lane_point = carrier.add(receiver.pos.sub(carrier).scale(LANE_POINT_FRACTION));
    base.add(lane_point.sub(base).scale(LANE_SHADOW_WEIGHT))
}

/// Shade a HELD position onto the highest-valued lane instead of drawing it
/// to the single shared lane point: each player steps toward the NEAREST
/// point of the lane, so several of them can cut it at once without
/// stacking and without giving up the ground they are standing on.
/// Counter-pressing uses this, because its contract is to hold the turnover
/// position rather than converge on the ball.
#[must_use]
pub fn lane_hold_target(base: Vec2, carrier: Vec2, candidates: &[OutfieldLaneCandidate]) -> Vec2 {
    let receiver = match highest_scored_lane(candidates) {
        Some(receiver) => receiver,
        None => return base,
    };
    let lane = receiver.pos.sub(carrier);
    let length_squared = lane.x * lane.x + lane.y * lane.y;
    let mut nearest = carrier;
    if length_squared > 0.0 {
        let fraction =
            ((base.x - carrier.x) * lane.x + (base.y - carrier.y) * lane.y) / length_squared;
        nearest = carrier.add(lane.scale(fraction.clamp(0.0, 1.0)));
    }
    base.add(nearest.sub(base).scale(LANE_SHADOW_WEIGHT))
}
