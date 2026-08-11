//! Retained, serializable combat intent for one AI-driven player.
//!
//! The AI has no direct simulation action channel: it can only materialize the
//! same three abstract equipment signals a human produces. Ranged needs a press
//! tick followed by a release edge, and guard needs a held run followed by a
//! release, so the producer must remember where in that materialization it is.
//! That memory lives here as plain versioned scalars, so it survives strict
//! snapshot capture, canonical hashing, and rollback resimulation instead of
//! hiding in a producer closure.
//!
//! It is deliberately TINY. Retained state is charged against the bounded
//! rollback snapshot budget at ten players times thirty-one boundaries, so
//! anything derivable is derived instead of
//! stored: the decision cadence is a stateless function of the canonical tick and
//! the player's scan rate, and the policy's selection seed is a pure function of
//! the tick and the player index. Only what genuinely cannot be recomputed --
//! where a multi-tick materialization stands, and the stable reason and target it
//! serves -- is kept here.
//!
//! There is no per-record version field: this record is a child of the versioned
//! `CombatMatchState` companion, exactly like a windup shot or a combat event,
//! and `combat_snapshot::VERSION` is what moves when this shape changes.
//!
//! It requires only leaf modules: [`crate::combat_snapshot`] requires it, so it
//! must not reach back into the snapshot or the match.

use crate::brain;
use crate::fixed_clock;
use gc_core::rng;

/// The stage of a multi-tick equipment materialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatIntentStage {
    /// No materialization in progress.
    Idle,
    /// Owes exactly one release edge next tick.
    Release,
    /// A guard hold in progress; owes `hold_ticks` more held ticks, then a
    /// release.
    Hold,
}

/// The five soccer-purpose ids this module can commit under, plus `None` (no
/// decision yet) and `Decline` (an episode closed with no action).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatDecisionReason {
    /// No decision has been made yet.
    None,
    /// An episode closed with no action.
    Decline,
    /// The target is the opposing ball carrier.
    CarrierContest,
    /// The target is the nearest opponent to our own carrier.
    CarrierProtection,
    /// The target and the source both sit within reach of a loose ball.
    LooseBallContest,
    /// The target is the sole blocker of a passing lane or shot lane.
    PassingLaneOrShotDenial,
    /// The target is mid-recovery on top of another true purpose.
    RecoveryPunish,
    /// A zero-context or infeasible commit's closed outcome.
    UnattributedOffBall,
}

impl CombatDecisionReason {
    /// Whether this reason is one of the five purpose ids a materializing
    /// commit requires (`COMMIT_REASONS`).
    #[must_use]
    pub fn is_commit_reason(self) -> bool {
        matches!(
            self,
            CombatDecisionReason::CarrierContest
                | CombatDecisionReason::CarrierProtection
                | CombatDecisionReason::LooseBallContest
                | CombatDecisionReason::PassingLaneOrShotDenial
                | CombatDecisionReason::RecoveryPunish
                | CombatDecisionReason::UnattributedOffBall
        )
    }
}

/// This module's retained per-player state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatIntentState {
    /// The current materialization stage.
    pub stage: CombatIntentStage,
    /// Guard hold ticks still owed before the release edge.
    pub hold_ticks: i64,
    /// The stable reason this materialization serves.
    pub reason: CombatDecisionReason,
    /// The stable target this materialization serves.
    pub target_player: Option<i64>,
}

/// This state's format version. Field order is the canonical serialization
/// order [`crate::combat_snapshot`] uses.
pub const VERSION: i64 = 1;
/// The refresh interval at zero scan rate.
pub const SLOW_REFRESH_SECONDS: f64 = 0.45;
/// The refresh interval at full scan rate.
pub const FAST_REFRESH_SECONDS: f64 = 0.15;

/// Stride for the per-decision seed. A large prime keeps consecutive ticks and
/// adjacent player indices far apart in the stream.
pub const DECISION_SEED_STRIDE: i64 = 15_485_863;

fn validate_state(state: &CombatIntentState) {
    assert!(
        state.hold_ticks >= 0,
        "combat intent hold ticks must be a non-negative integer"
    );
    if let Some(target_player) = state.target_player {
        assert!(
            target_player >= 1,
            "combat intent target must be a positive integer"
        );
    }
    match state.stage {
        CombatIntentStage::Idle => {
            assert_eq!(
                state.hold_ticks, 0,
                "an idle combat intent cannot owe guard hold ticks"
            );
        }
        _ => {
            assert!(
                state.reason.is_commit_reason(),
                "a materializing combat intent requires a commit reason"
            );
            assert!(
                state.target_player.is_some(),
                "a materializing combat intent requires a target"
            );
        }
    }
    if state.stage != CombatIntentStage::Hold {
        assert_eq!(state.hold_ticks, 0, "only a guard hold owes hold ticks");
    }
}

/// A fresh, idle intent state.
#[must_use]
pub fn new_state() -> CombatIntentState {
    CombatIntentState {
        stage: CombatIntentStage::Idle,
        hold_ticks: 0,
        reason: CombatDecisionReason::None,
        target_player: None,
    }
}

/// Validate and defensively copy an intent state.
#[must_use]
pub fn copy_state(state: &CombatIntentState) -> CombatIntentState {
    validate_state(state);
    *state
}

/// Drop any in-flight materialization. Used when a player becomes combat
/// ineligible (forced, keeper, aerial, match over) so a stale release edge can
/// never be emitted into a later, unrelated situation.
#[must_use]
pub fn reset(state: &CombatIntentState) -> CombatIntentState {
    let _ = copy_state(state);
    new_state()
}

/// The action closing an episode without a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclineAction {
    /// The episode had a ready tick and chose not to act.
    Decline,
    /// The episode was never action-ready.
    Unavailable,
}

/// Close an episode without an action. `Decline` is the contract's outcome for
/// an episode that had a ready tick and chose not to act; an episode that was
/// never action-ready is `unavailable:<reason>` and must NOT be labelled a
/// decline, so it carries no reason at all.
#[must_use]
pub fn decline(state: &CombatIntentState, action: Option<DeclineAction>) -> CombatIntentState {
    let mut next_state = copy_state(state);
    next_state.stage = CombatIntentStage::Idle;
    next_state.hold_ticks = 0;
    next_state.reason = if action == Some(DeclineAction::Unavailable) {
        CombatDecisionReason::None
    } else {
        CombatDecisionReason::Decline
    };
    next_state.target_player = None;
    copy_state(&next_state)
}

/// Record an accepted commit and the materialization it still owes.
/// `hold_ticks` is guard-only; every other family owes exactly one release
/// edge.
///
/// # Panics
///
/// Panics if `reason` is not one of the five purpose commit reasons.
#[must_use]
pub fn commit(
    state: &CombatIntentState,
    reason: CombatDecisionReason,
    target_player: i64,
    hold_ticks: i64,
) -> CombatIntentState {
    let mut next_state = copy_state(state);
    assert!(
        reason.is_commit_reason(),
        "a commit requires a commit reason"
    );
    next_state.reason = reason;
    next_state.target_player = Some(target_player);
    if hold_ticks > 0 {
        next_state.stage = CombatIntentStage::Hold;
        next_state.hold_ticks = hold_ticks;
    } else {
        next_state.stage = CombatIntentStage::Release;
        next_state.hold_ticks = 0;
    }
    copy_state(&next_state)
}

/// The personal decision cadence, derived rather than retained: a stat-scaled
/// interval plus a stable per-player phase offset. Two players with the same
/// scan rate still decide on different ticks, and nobody re-decides every
/// tick, which is what stops targets and actions oscillating.
#[must_use]
pub fn decision_period(scan_rate: f64) -> i64 {
    let seconds = brain::refresh_interval(scan_rate, SLOW_REFRESH_SECONDS, FAST_REFRESH_SECONDS);
    1i64.max((seconds * fixed_clock::TICK_RATE + 0.5).floor() as i64)
}

/// Whether `(tick, player_index)` is due a decision.
#[must_use]
pub fn should_decide(tick: i64, player_index: i64, scan_rate: f64) -> bool {
    (tick + player_index).rem_euclid(decision_period(scan_rate)) == 0
}

/// A dedicated canonical Park-Miller seed for one decision. It never touches
/// the match RNG and never persists: the same (tick, player) always yields the
/// same stream, which is exactly what a rollback resimulation of that
/// boundary needs.
#[must_use]
pub fn decision_seed(tick: i64, player_index: i64) -> u32 {
    rng::seed((tick * DECISION_SEED_STRIDE + player_index) as f64)
}

/// The three abstract equipment signals one materialization tick places on
/// this tick's `MatchInput`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatIntentSignals {
    /// Equipment control is physically held.
    pub equipment_held: bool,
    /// Equipment press edge for this fixed tick.
    pub equipment_pressed: bool,
    /// Equipment release edge for this fixed tick.
    pub equipment_released: bool,
}

/// Advance one owed materialization tick. Returns the abstract signals to
/// place on this tick's `MatchInput` and the next retained state.
///
/// Every triple this returns satisfies `input_frame::validate_sample`'s
/// legality rule: a press always carries `held`, and a release never carries
/// `held`.
#[must_use]
pub fn materialize(state: &CombatIntentState) -> (Option<CombatIntentSignals>, CombatIntentState) {
    let mut next_state = copy_state(state);
    match next_state.stage {
        CombatIntentStage::Release => {
            next_state.stage = CombatIntentStage::Idle;
            next_state.hold_ticks = 0;
            (
                Some(CombatIntentSignals {
                    equipment_held: false,
                    equipment_pressed: false,
                    equipment_released: true,
                }),
                copy_state(&next_state),
            )
        }
        CombatIntentStage::Hold => {
            if next_state.hold_ticks > 0 {
                next_state.hold_ticks -= 1;
                if next_state.hold_ticks == 0 {
                    next_state.stage = CombatIntentStage::Release;
                }
                (
                    Some(CombatIntentSignals {
                        equipment_held: true,
                        equipment_pressed: false,
                        equipment_released: false,
                    }),
                    copy_state(&next_state),
                )
            } else {
                next_state.stage = CombatIntentStage::Idle;
                (
                    Some(CombatIntentSignals {
                        equipment_held: false,
                        equipment_pressed: false,
                        equipment_released: true,
                    }),
                    copy_state(&next_state),
                )
            }
        }
        CombatIntentStage::Idle => (None, copy_state(&next_state)),
    }
}

/// The signals a fresh commit places on the tick it is decided.
#[must_use]
pub fn commit_signals() -> CombatIntentSignals {
    CombatIntentSignals {
        equipment_held: true,
        equipment_pressed: true,
        equipment_released: false,
    }
}
