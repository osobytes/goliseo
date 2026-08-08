//! Port of `sim/outfield_decision.lua`.
//!
//! Serializable personal decision cadence and carrier-option construction.
//! Callers own world geometry and execution; this module retains only value
//! state and resolves deterministic numeric contexts through
//! [`crate::brain`].

use crate::brain;
use gc_core::rng;
use indexmap::IndexMap;

/// Which eligibility context a decision was made under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutfieldDecisionContext {
    /// The player is not eligible to act right now.
    Ineligible,
    /// The player is off the ball.
    Offball,
    /// The player is carrying the ball.
    Carrier,
}

/// What a player has decided to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutfieldIntent {
    /// No decision has been made yet.
    None,
    /// Move to an ordinary support point.
    Move,
    /// Run in behind the last defender.
    InBehind,
    /// Come short to offer an outlet.
    ComeShort,
    /// Hold width out on the flank.
    HoldWidth,
    /// Shoot at goal.
    Shoot,
    /// Cross into the box.
    Cross,
    /// Pass to a teammate.
    Pass,
    /// Dribble with the ball.
    Dribble,
}

/// This module's serializable per-player decision state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldDecisionState {
    /// This state's format version.
    pub version: u32,
    /// Monotonic count of every [`refresh`] this state has been through.
    pub generation: u32,
    /// This player's dedicated decision RNG stream state.
    pub rng_state: u32,
    /// Seconds remaining before the next refresh is due.
    pub remaining: f64,
    /// The eligibility context this decision was made under.
    pub context: OutfieldDecisionContext,
    /// The decided intent.
    pub intent: OutfieldIntent,
    /// The decision's target x, if it targets a point.
    pub target_x: Option<f64>,
    /// The decision's target y, if it targets a point.
    pub target_y: Option<f64>,
    /// The decision's target player, if it targets a player.
    pub target_player: Option<u32>,
    /// The absolute expiry of an active run, if this is a run intent.
    pub run_expires_at: Option<f64>,
}

/// One pass option offered to [`carrier_options`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldPassOption {
    /// The receiving player's stable identity.
    pub player_index: u32,
    /// How open the receiver is.
    pub openness: f64,
    /// How much forward progress the pass makes.
    pub forward_progress: f64,
    /// The pass distance.
    pub distance: f64,
    /// Whether the caller has determined the lane is blocked.
    pub lane_blocked: bool,
    /// Whether the caller has flagged this pass as interceptable.
    pub interception_risk: bool,
    /// Where along the lane a blocker sits, if any.
    pub lane_fraction: Option<f64>,
}

/// Inputs to [`carrier_options`].
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldCarrierContext {
    /// Distance to the goal.
    pub goal_distance: f64,
    /// The carrier's effective shooting range. Must be positive.
    pub shoot_range: f64,
    /// How good the shooting angle is, in `[0, 1]`.
    pub angle_quality: f64,
    /// How well the keeper covers the goal, in `[0, 1]`.
    pub keeper_coverage: f64,
    /// How much space the carrier has, in `[0, 1]`.
    pub space: f64,
    /// How deep the flank runner is, in `[0, 1]`.
    pub flank_depth: f64,
    /// The best cross target, if any.
    pub cross_target: Option<u32>,
    /// How many teammates are in the box.
    pub box_targets: u32,
    /// How much space the cross target has, in `[0, 1]`.
    pub cross_space: f64,
    /// How much goal progress a dribble would make, in `[0, 1]`.
    pub goal_progress: f64,
    /// How much space a dribble has, in `[0, 1]`.
    pub dribble_space: f64,
    /// Every pass option available to the carrier.
    pub passes: Vec<OutfieldPassOption>,
}

/// This module's format version.
pub const VERSION: u32 = 2;
/// The refresh interval at zero scan rate.
pub const SLOW_REFRESH_SECONDS: f64 = 0.45;
/// The refresh interval at full scan rate.
pub const FAST_REFRESH_SECONDS: f64 = 0.15;
/// The base softmax temperature for an ordinary carrier decision.
pub const BASE_TEMPERATURE: f64 = 3.0;
/// How long a granted run stays active, in seconds.
pub const RUN_LIFETIME_SECONDS: f64 = 1.8;

const SHARP_COMPOSURE: f64 = 0.8;
const PASS_RISK_PENALTY: f64 = 5.0;
const PASS_SPACE_BALANCE: f64 = 0.6;
const PASS_SPACE_WEIGHT: f64 = 90.0;

fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return 0.0;
    }
    if value == f64::INFINITY {
        return 1.0;
    }
    value.clamp(0.0, 1.0)
}

/// Whether `intent` is one of the off-ball run intents.
#[must_use]
pub fn is_run_intent(intent: OutfieldIntent) -> bool {
    matches!(
        intent,
        OutfieldIntent::InBehind | OutfieldIntent::ComeShort | OutfieldIntent::HoldWidth
    )
}

fn is_carrier_intent(intent: OutfieldIntent) -> bool {
    matches!(
        intent,
        OutfieldIntent::Shoot
            | OutfieldIntent::Cross
            | OutfieldIntent::Pass
            | OutfieldIntent::Dribble
    )
}

fn validate_state(state: &OutfieldDecisionState) {
    assert_eq!(
        state.version, VERSION,
        "unsupported outfield decision version"
    );
    assert_eq!(
        rng::seed(f64::from(state.rng_state)),
        state.rng_state,
        "outfield decision RNG state must be a canonical positive integer"
    );
    assert!(
        state.remaining.is_finite(),
        "outfield decision remaining time must be finite"
    );
    assert!(
        state.remaining >= 0.0,
        "outfield decision remaining time must be non-negative"
    );
    if let Some(x) = state.target_x {
        assert!(x.is_finite(), "outfield decision target x must be finite");
    }
    if let Some(y) = state.target_y {
        assert!(y.is_finite(), "outfield decision target y must be finite");
    }
    assert_eq!(
        state.target_x.is_none(),
        state.target_y.is_none(),
        "outfield decision target coordinates must be paired"
    );
    if let Some(target_player) = state.target_player {
        assert!(
            target_player >= 1,
            "outfield decision target player must be a positive integer"
        );
    }
    if let Some(run_expires_at) = state.run_expires_at {
        assert!(
            run_expires_at.is_finite(),
            "outfield decision run expiry must be finite"
        );
    }
    match state.context {
        OutfieldDecisionContext::Ineligible => {
            assert_eq!(
                state.intent,
                OutfieldIntent::None,
                "ineligible outfield decision state must have no intent"
            );
            assert_eq!(
                state.remaining, 0.0,
                "ineligible outfield decision state cannot retain cadence"
            );
            assert!(
                state.target_x.is_none() && state.target_player.is_none(),
                "ineligible outfield decision state cannot retain a target"
            );
            assert!(
                state.run_expires_at.is_none(),
                "ineligible outfield decision state cannot retain a run expiry"
            );
        }
        OutfieldDecisionContext::Offball => {
            assert!(
                state.intent == OutfieldIntent::Move || is_run_intent(state.intent),
                "offball outfield decision state must retain a movement intent"
            );
            assert!(
                state.target_x.is_some(),
                "offball movement intent requires a target point"
            );
            assert!(
                state.target_player.is_none(),
                "offball movement intent cannot target a player"
            );
            if is_run_intent(state.intent) {
                assert!(
                    state.run_expires_at.is_some(),
                    "offball run intent requires an absolute expiry"
                );
            } else {
                assert!(
                    state.run_expires_at.is_none(),
                    "ordinary offball movement cannot retain a run expiry"
                );
            }
        }
        OutfieldDecisionContext::Carrier => {
            assert!(
                is_carrier_intent(state.intent),
                "carrier outfield decision state requires a carrier intent"
            );
            if state.intent == OutfieldIntent::Pass || state.intent == OutfieldIntent::Cross {
                assert!(
                    state.target_player.is_some(),
                    "pass and cross intents require a target player"
                );
                assert!(
                    state.target_x.is_none(),
                    "pass and cross intents cannot retain a target point"
                );
            } else {
                assert!(
                    state.target_x.is_none() && state.target_player.is_none(),
                    "shoot and dribble intents cannot retain a target"
                );
            }
            assert!(
                state.run_expires_at.is_none(),
                "carrier outfield decision state cannot retain a run expiry"
            );
        }
    }
}

/// A fresh, ineligible decision state, seeded from a raw (not necessarily
/// canonical) RNG seed. Defaults to seed `1.0` when `rng_seed` is `None`.
#[must_use]
pub fn new_state(rng_seed: Option<f64>) -> OutfieldDecisionState {
    OutfieldDecisionState {
        version: VERSION,
        generation: 0,
        rng_state: rng::seed(rng_seed.unwrap_or(1.0)),
        remaining: 0.0,
        context: OutfieldDecisionContext::Ineligible,
        intent: OutfieldIntent::None,
        target_x: None,
        target_y: None,
        target_player: None,
        run_expires_at: None,
    }
}

/// Validate and defensively copy a decision state.
#[must_use]
pub fn copy_state(state: &OutfieldDecisionState) -> OutfieldDecisionState {
    validate_state(state);
    *state
}

/// Advance the cadence countdown by `dt` seconds, floored at zero.
#[must_use]
pub fn advance(state: &OutfieldDecisionState, dt: f64) -> OutfieldDecisionState {
    let mut next_state = copy_state(state);
    assert!(dt.is_finite(), "outfield decision delta must be finite");
    assert!(dt >= 0.0, "outfield decision delta must be non-negative");
    next_state.remaining = (next_state.remaining - dt).max(0.0);
    next_state
}

/// Drop back to the ineligible baseline, keeping the RNG stream and the
/// refresh generation counter.
#[must_use]
pub fn reset(state: &OutfieldDecisionState) -> OutfieldDecisionState {
    let current = copy_state(state);
    let mut reset_state = new_state(Some(f64::from(current.rng_state)));
    reset_state.generation = current.generation;
    reset_state
}

/// Replace the RNG stream state (e.g. after a caller advances it via
/// [`decide_carrier`]).
#[must_use]
pub fn with_rng_state(state: &OutfieldDecisionState, rng_state: u32) -> OutfieldDecisionState {
    let mut next_state = copy_state(state);
    next_state.rng_state = rng_state;
    copy_state(&next_state)
}

/// End an active run without inventing a cadence boundary. The caller
/// supplies the ordinary support fallback that takes over for the retained
/// countdown.
///
/// # Panics
///
/// Panics if `state` does not hold an active run intent.
#[must_use]
pub fn cancel_run(
    state: &OutfieldDecisionState,
    target_x: f64,
    target_y: f64,
) -> OutfieldDecisionState {
    let mut next_state = copy_state(state);
    assert!(
        is_run_intent(next_state.intent),
        "cancel_run requires an active run intent"
    );
    next_state.intent = OutfieldIntent::Move;
    next_state.target_x = Some(target_x);
    next_state.target_y = Some(target_y);
    next_state.target_player = None;
    next_state.run_expires_at = None;
    copy_state(&next_state)
}

/// Whether a fresh decision is due: an ineligible context never refreshes;
/// otherwise a context change, an urgent request, or an expired cadence
/// forces one.
#[must_use]
pub fn should_refresh(
    state: &OutfieldDecisionState,
    context: OutfieldDecisionContext,
    urgent: Option<bool>,
) -> bool {
    validate_state(state);
    context != OutfieldDecisionContext::Ineligible
        && (urgent == Some(true) || state.context != context || state.remaining <= 0.0)
}

/// Commit a fresh decision: bumps the generation, resets the cadence from
/// `scan_rate`, and records the new context/intent/target.
///
/// # Panics
///
/// Panics if `context` is [`OutfieldDecisionContext::Ineligible`].
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn refresh(
    state: &OutfieldDecisionState,
    context: OutfieldDecisionContext,
    intent: OutfieldIntent,
    scan_rate: f64,
    target_x: Option<f64>,
    target_y: Option<f64>,
    target_player: Option<u32>,
    run_expires_at: Option<f64>,
) -> OutfieldDecisionState {
    validate_state(state);
    assert!(
        context != OutfieldDecisionContext::Ineligible,
        "refresh requires an eligible context"
    );
    let refreshed = OutfieldDecisionState {
        version: VERSION,
        generation: state.generation + 1,
        rng_state: state.rng_state,
        remaining: brain::refresh_interval(scan_rate, SLOW_REFRESH_SECONDS, FAST_REFRESH_SECONDS),
        context,
        intent,
        target_x,
        target_y,
        target_player,
        run_expires_at,
    };
    copy_state(&refreshed)
}

/// Build the carrier's complete legitimate option set: shoot, dribble, an
/// optional cross, then one pass option per candidate. Construction consumes
/// no RNG; only [`decide_carrier`] does.
#[must_use]
pub fn carrier_options(context: &OutfieldCarrierContext) -> Vec<brain::CarrierOption> {
    assert!(
        context.goal_distance.is_finite(),
        "carrier goal distance must be finite"
    );
    assert!(
        context.goal_distance >= 0.0,
        "carrier goal distance must be non-negative"
    );
    assert!(
        context.shoot_range.is_finite(),
        "carrier shoot range must be finite"
    );
    assert!(
        context.shoot_range > 0.0,
        "carrier shoot range must be positive"
    );
    assert!(context.space.is_finite(), "carrier space must be finite");
    assert!(
        context.cross_space.is_finite(),
        "carrier cross space must be finite"
    );
    assert!(
        context.flank_depth.is_finite(),
        "carrier flank depth must be finite"
    );
    assert!(
        context.goal_progress.is_finite(),
        "carrier goal progress must be finite"
    );
    assert!(
        context.dribble_space.is_finite(),
        "carrier dribble space must be finite"
    );

    let distance_falloff = context.goal_distance / context.shoot_range;
    let mut options: Vec<brain::CarrierOption> = vec![
        brain::CarrierOption {
            id: "shoot".to_string(),
            kind: "shoot".to_string(),
            score: 100.0 - distance_falloff * distance_falloff * 60.0
                + clamp_unit(context.angle_quality) * 25.0
                - clamp_unit(context.keeper_coverage) * 12.0
                + clamp_unit(context.space) * 18.0,
            payload: None,
            reference: None,
        },
        brain::CarrierOption {
            id: "dribble".to_string(),
            kind: "dribble".to_string(),
            score: 10.0
                + clamp_unit(context.dribble_space) * 35.0
                + clamp_unit(context.goal_progress) * 25.0,
            payload: None,
            reference: None,
        },
    ];

    if let Some(cross_target) = context.cross_target
        && context.box_targets > 0
    {
        options.push(brain::CarrierOption {
            id: "cross".to_string(),
            kind: "cross".to_string(),
            score: 60.0
                + clamp_unit(context.flank_depth) * 45.0
                + f64::from(context.box_targets.min(3)) * 8.0
                + clamp_unit(context.cross_space) * 15.0,
            payload: None,
            reference: Some(brain::OptionReference::Index(cross_target)),
        });
    }

    let mut seen: Vec<u32> = Vec::new();
    for (index, pass) in context.passes.iter().enumerate() {
        assert!(
            pass.player_index >= 1,
            "pass option {} player must be a positive integer",
            index + 1
        );
        assert!(
            !seen.contains(&pass.player_index),
            "pass option players must be unique"
        );
        seen.push(pass.player_index);
        assert!(
            pass.openness.is_finite(),
            "pass option openness must be finite"
        );
        assert!(
            pass.forward_progress.is_finite(),
            "pass option forward progress must be finite"
        );
        assert!(
            pass.distance.is_finite(),
            "pass option distance must be finite"
        );
        assert!(
            pass.distance >= 0.0,
            "pass option distance must be non-negative"
        );
        if let Some(lane_fraction) = pass.lane_fraction {
            assert!(
                lane_fraction.is_finite(),
                "pass option lane fraction must be finite"
            );
        }
        let payload = pass.lane_fraction.map(|lane_fraction| {
            let mut map = IndexMap::new();
            map.insert(
                "lane_fraction".to_string(),
                brain::BrainPayloadValue::Number(lane_fraction),
            );
            map
        });
        options.push(brain::CarrierOption {
            id: format!("pass_{}", pass.player_index),
            kind: "pass".to_string(),
            score: 18.0
                + (pass.openness.max(0.0) / 150.0).min(1.0) * 45.0
                + (pass.forward_progress / 300.0).clamp(-1.0, 1.0) * 28.0
                - (pass.distance / 420.0).min(1.0) * 18.0
                + (PASS_SPACE_BALANCE - clamp_unit(context.space)) * PASS_SPACE_WEIGHT
                - if pass.interception_risk {
                    PASS_RISK_PENALTY
                } else {
                    0.0
                },
            payload,
            reference: Some(brain::OptionReference::Index(pass.player_index)),
        });
    }
    options
}

/// Decide what the carrier does: at the authored high-composure boundary
/// the temperature is exactly zero, so an effectively settled choice takes
/// the argmax without needlessly advancing its dedicated decision RNG
/// stream.
#[must_use]
pub fn decide_carrier(
    options: &[brain::CarrierOption],
    composure: f64,
    pressure: f64,
    rng_state: u32,
) -> (brain::CarrierOption, u32) {
    let base_temperature = if composure >= SHARP_COMPOSURE {
        0.0
    } else {
        BASE_TEMPERATURE
    };
    brain::decide_carrier(options, composure, pressure, base_temperature, rng_state)
}
