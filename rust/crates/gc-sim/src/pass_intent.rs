//! Retained, serializable pass intent for one AI-driven player.
//!
//! The gameplay AI has no direct pass/throw resolver access any more (#531):
//! it can only materialize the same `pass_held`/`pass`/`lob` signals a
//! human's `MatchInput` carries. A charged release takes several ticks, so
//! the producer must remember which teammate it is aiming at, whether it
//! wants loft, and how much charge it is holding out for — that memory
//! lives here as plain versioned scalars, exactly like
//! [`crate::combat_intent`]'s equipment memory, so it survives strict
//! snapshot capture, canonical hashing, and rollback resimulation instead of
//! hiding in a producer closure.
//!
//! It is deliberately smaller than `combat_intent`: there is no press/hold/
//! release staging to track, because "should I release this tick" is a
//! pure comparison between the live, shared `pass_charge` field (which the
//! ordinary charge-accumulation code in `sim::match` already owns) and this
//! record's own `target_charge`. Two stages are enough — `Idle` and
//! `Charging` — and neither owns a countdown.
//!
//! There is no per-record version field: this record is a field on
//! [`crate::match_snapshot::MatchPlayer`], and `match_snapshot::VERSION` is
//! what moves when this shape changes.

use gc_core::vec2::Vec2;

/// The stage of a pass/throw materialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassIntentStage {
    /// No pass or throw is being charged.
    Idle,
    /// Holding toward `target_player`, waiting for the shared `pass_charge`
    /// field to reach `target_charge`.
    Charging,
}

/// This module's retained per-player state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassIntentState {
    /// The current materialization stage.
    pub stage: PassIntentStage,
    /// The teammate this charge is aimed toward. An aim HINT, not the
    /// guaranteed receiver — the soft cone (`select_pass_target` /
    /// `select_throw_target`) still picks who actually receives the ball at
    /// release, exactly as it does for a human. See #531's design note on
    /// this deliberate divergence.
    pub target_player: Option<i64>,
    /// Whether release should request loft (`MatchInput::lob`).
    pub lofted: bool,
    /// The `pass_charge` value (`[0, 1]`) this intent is holding out for.
    pub target_charge: f64,
}

fn validate_state(state: &PassIntentState) {
    match state.stage {
        PassIntentStage::Idle => {
            assert!(
                state.target_player.is_none(),
                "an idle pass intent cannot retain a target"
            );
            assert_eq!(
                state.target_charge, 0.0,
                "an idle pass intent cannot retain a target charge"
            );
            assert!(
                !state.lofted,
                "an idle pass intent cannot retain a lofted flag"
            );
        }
        PassIntentStage::Charging => {
            assert!(
                state.target_player.is_some(),
                "a charging pass intent requires a target"
            );
            if let Some(target_player) = state.target_player {
                assert!(
                    target_player >= 1,
                    "pass intent target must be a positive integer"
                );
            }
            assert!(
                (0.0..=1.0).contains(&state.target_charge),
                "pass intent target charge must be in [0, 1]"
            );
        }
    }
}

/// A fresh, idle intent state.
#[must_use]
pub fn new_state() -> PassIntentState {
    PassIntentState {
        stage: PassIntentStage::Idle,
        target_player: None,
        lofted: false,
        target_charge: 0.0,
    }
}

/// Validate and defensively copy an intent state.
#[must_use]
pub fn copy_state(state: &PassIntentState) -> PassIntentState {
    validate_state(state);
    *state
}

/// Drop any in-flight charge. Used when a player loses possession (mid-charge
/// or otherwise) so a stale target and threshold can never be resumed
/// against a later, unrelated situation.
#[must_use]
pub fn reset(state: &PassIntentState) -> PassIntentState {
    let _ = copy_state(state);
    new_state()
}

/// Commit to charging a pass or throw toward `target_player`.
///
/// # Panics
///
/// Panics if `target_player` is not a positive player index, or if
/// `target_charge` (once clamped) still leaves the state invalid.
#[must_use]
pub fn commit(
    state: &PassIntentState,
    target_player: i64,
    lofted: bool,
    target_charge: f64,
) -> PassIntentState {
    let _ = copy_state(state);
    assert!(
        target_player >= 1,
        "pass intent commit target must be a positive integer"
    );
    let next_state = PassIntentState {
        stage: PassIntentStage::Charging,
        target_player: Some(target_player),
        lofted,
        target_charge: target_charge.clamp(0.0, 1.0),
    };
    copy_state(&next_state)
}

/// The `pass_held`/`pass` signals one materialization tick places on this
/// tick's `MatchInput`. Mirrors the human/bot convention
/// (`bot.rs`'s `pass_charge_t` branch): a release edge never carries
/// `pass_held`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassIntentSignals {
    /// The pass/throw key is currently held (builds charge).
    pub pass_held: bool,
    /// The pass/throw releases this frame.
    pub pass: bool,
}

/// Advance one owed materialization tick against the live `pass_charge`
/// value the shared charge-accumulation code already maintains. Returns the
/// abstract signals to place on this tick's `MatchInput` and the next
/// retained state.
///
/// `current_charge` is read BEFORE this tick's own accumulation runs (the
/// caller materializes first, then feeds the resulting input through the
/// same charge-accumulation code a human's input takes) — so a release
/// fires the tick the charge from PRIOR ticks already met the threshold,
/// never the tick it is first requested. For a sub-threshold target charge
/// (a short pass) that tick is the same tick the intent was committed:
/// `current_charge` starts at `0.0`, so `0.0 >= target_charge` is
/// immediately true and the tap releases with no hold at all.
///
/// BENIGN EDGE CASE at `target_charge == 1.0` exactly (reachable: a
/// committed distance beyond what any charge can reach clamps here, see
/// `commit_outfield_pass_intent`/`commit_keeper_pass_intent`'s callers).
/// This function can still return `Charging` (holding) on the very tick the
/// shared charge-accumulation code separately fires its OWN "full meter
/// lets fly on its own" check (`owner.pass_charge >= 1.0` inside
/// `outfield_actions`/`keeper_actions`), because that check runs AFTER this
/// one, against the value THIS tick's accumulation just produced — one step
/// ahead of what `current_charge` (this tick's pre-accumulation value) let
/// this function see. The release still fires correctly, through the same
/// verb, on that tick; only the retained `PassIntentState` is briefly stale
/// (`Charging`, pointing at a target/threshold nothing will read again).
/// It self-heals the very next tick regardless of what the player does
/// next: `update_ball`'s unconditional per-tick sweep resets `pass_intent`
/// for anyone who isn't the ball owner, and losing possession is exactly
/// what a release does. `validate_state` does not reject the stale value
/// (a `Charging` state with a target and an in-range threshold is
/// internally well-formed), so nothing observable breaks — this is
/// documented so the next person tracing a materialize/full-meter
/// interaction doesn't have to rediscover it.
#[must_use]
pub fn materialize(
    state: &PassIntentState,
    current_charge: f64,
) -> (PassIntentSignals, PassIntentState) {
    let s = copy_state(state);
    match s.stage {
        PassIntentStage::Idle => (
            PassIntentSignals {
                pass_held: false,
                pass: false,
            },
            s,
        ),
        PassIntentStage::Charging => {
            if current_charge >= s.target_charge {
                (
                    PassIntentSignals {
                        pass_held: false,
                        pass: true,
                    },
                    reset(&s),
                )
            } else {
                (
                    PassIntentSignals {
                        pass_held: true,
                        pass: false,
                    },
                    s,
                )
            }
        }
    }
}

/// A unit aim vector for `owner_pos` to hold toward `target_pos`, falling
/// back to `facing` when the two positions coincide (a target standing
/// exactly on the owner, or a degenerate fixture) so the aim is never the
/// zero vector.
#[must_use]
pub fn aim_toward(owner_pos: Vec2, target_pos: Vec2, facing: Vec2) -> Vec2 {
    let to_target = target_pos.sub(owner_pos);
    if to_target.length() > 1.0 {
        to_target.normalized()
    } else {
        facing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_threshold_distance_releases_immediately() {
        // A "tap": target_charge is 0, so the very first materialize call
        // (current_charge still 0.0, nothing accumulated yet) must release
        // on the spot rather than opening a hold.
        let state = commit(&new_state(), 3, false, 0.0);
        let (signals, next) = materialize(&state, 0.0);
        assert!(
            signals.pass,
            "a zero-threshold commit must release immediately"
        );
        assert!(
            !signals.pass_held,
            "a release edge must not also carry pass_held"
        );
        assert_eq!(
            next.stage,
            PassIntentStage::Idle,
            "a fired release must return to Idle"
        );
        assert_eq!(next.target_player, None);
    }

    #[test]
    fn long_distance_holds_then_releases_exactly_once() {
        let state = commit(&new_state(), 7, true, 0.6);
        // Three ticks of accumulated charge below threshold: hold, never
        // release.
        for charge in [0.0, 0.2, 0.4] {
            let (signals, next) = materialize(&state, charge);
            assert!(signals.pass_held, "charge {charge} should still be holding");
            assert!(!signals.pass, "charge {charge} must not release early");
            assert_eq!(next.stage, PassIntentStage::Charging);
            assert_eq!(next.target_player, Some(7));
            assert!(next.lofted);
        }
        // The threshold is crossed: release fires, exactly once.
        let (signals, next) = materialize(&state, 0.6);
        assert!(signals.pass, "charge at threshold must release");
        assert!(!signals.pass_held);
        assert_eq!(next.stage, PassIntentStage::Idle);
        // A second materialize call against the now-idle state must not
        // release again.
        let (signals2, next2) = materialize(&next, 0.6);
        assert!(!signals2.pass, "an idle intent must never re-release");
        assert!(!signals2.pass_held);
        assert_eq!(next2.stage, PassIntentStage::Idle);
    }

    #[test]
    fn reset_from_charging_leaks_no_target_or_lofted_or_threshold() {
        let state = commit(&new_state(), 4, true, 0.85);
        let cleared = reset(&state);
        assert_eq!(cleared.stage, PassIntentStage::Idle);
        assert_eq!(cleared.target_player, None);
        assert!(!cleared.lofted);
        assert_eq!(cleared.target_charge, 0.0);
    }

    #[test]
    fn aim_toward_falls_back_to_facing_when_degenerate() {
        let facing = Vec2::new(0.0, 1.0);
        let aim = aim_toward(Vec2::new(10.0, 10.0), Vec2::new(10.0, 10.0), facing);
        assert_eq!(aim, facing);
    }
}
