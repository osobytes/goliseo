//! The committed-action state machine (#489): sim-owned wind-ups, release
//! triggers, and post-miss recovery, for whichever verb currently occupies a
//! player's action slot.
//!
//! ```text
//! action: none
//!       | { verb, phase = charging,   charge_elapsed }
//!       | { verb, phase = executing,  remaining, power }
//!       | { verb, phase = recovering, remaining }
//! ```
//!
//! [`ActionSlot`] is the flat, `Copy` record every phase shares, on the
//! `PassIntentState`/`CombatIntentState` precedent (`crate::pass_intent`,
//! `crate::combat_intent`): one struct tagged by [`ActionPhase`], with each
//! phase's *irrelevant* fields pinned to a canonical zero by
//! [`validate_state`], rather than a data-carrying `enum` per phase. A flat
//! record diffs and snapshots as a handful of scalars (see
//! `crate::match_snapshot::diff_pass_intent` for the shape this mirrors);
//! an enum-with-payload would need the same per-variant plumbing this
//! module already gives it, for no reduction in what has to be written.
//!
//! ## Seconds, not ticks -- a deliberate deviation from #489's own sketch
//!
//! #489's own ASCII sketch names its fields `started_tick`/`resolve_tick`/
//! `until_tick`. This codebase does not carry a discrete integer tick
//! counter anywhere in `MatchState` -- every existing multi-tick timer
//! (`windup_timer`, `pass_charge`, `tackle_cd`, `stun_timer`, ...) is a
//! continuous `f64` seconds value, counted up or down by the fixed but
//! non-integer-indexed `dt` the whole simulation steps by
//! (`crate::fixed_clock`). [`ActionSlot`] keeps that idiom instead of
//! introducing a second, tick-indexed clock two producers and a renderer
//! would have to keep synchronized with the first: [`ActionSlot::charge_elapsed`]
//! counts UP during `Charging` (mirroring `pass_charge`'s own
//! accumulate-up field), and [`ActionSlot::remaining`] counts DOWN to zero
//! during `Executing`/`Recovering` (mirroring `windup_timer`). This is a
//! reasoned deviation from the issue's literal field names, not from its
//! semantics: determinism does not depend on which unit the countdown is
//! expressed in, only on `dt` being fixed, which it already is.
//!
//! ## Why this absorbs `pass_intent` by argument, not by code, in this PR
//!
//! #531's blocking condition on #489 was structural: a second,
//! verb-scoped state machine that the AI bypasses through a private path is
//! exactly the mistake `combat_policy`'s module doc warns against ("an
//! accepted decision becomes three abstract equipment signals on an
//! ordinary `MatchInput`, exactly like a human producer"). This module is
//! designed so that promise generalizes -- [`ActionSlot`] is verb-generic,
//! [`clear`] is the one possession-invariant primitive every verb shares,
//! and [`evaluate_release`]'s four triggers are the same four for any verb
//! that adopts them.
//!
//! It does NOT, in this PR, replace [`crate::pass_intent`] or the
//! `windup_timer`/`windup_shot`/`pass_charge` fields shot and pass already
//! use. Those fields are not a second *action-slot* implementation in
//! #531's sense: they never gained charging/executing/recovering phases,
//! release-trigger priority, or a miss-recovery cost, so there is no
//! competing state machine to reconcile -- only a still-unmigrated one.
//! `pass_intent` specifically is the AI's *materialization memory* for the
//! shared `pass_charge` accumulator (see its own module doc: "there is no
//! press/hold/release staging to track"), a narrower problem than the one
//! this module solves, and it keeps working unchanged.
//!
//! [`ActionVerb::Tackle`] (a standing poke) is the only verb wired through
//! this module today, in `crate::r#match`'s tackle-resolution code. Shot and
//! pass adopting it is future work, tracked in this PR's description rather
//! than built here: doing so rewrites `SHOT_WINDUP`/`CHARGE_RATE`/
//! `PASS_CHARGE_RATE`'s call sites, which is a large enough surface, with
//! its own baseline movement, to review on its own. A slide tackle is a
//! separate move (`MatchPlayer::slide_timer`) and stays outside this PR's
//! scope too.
//!
//! ## The possession invariant lives here, once
//!
//! [`clear`] is the single primitive `crate::r#match`'s one ownership
//! choke-point (`set_owner`) calls on the outgoing owner, for whichever verb
//! occupies their slot, in whichever phase. It is unconditional: no verb
//! gets a say. A tackler never owns the ball while charging or executing a
//! poke, so this invariant is presently inert for `Tackle` specifically
//! (nothing clears because nothing they hold depends on ball ownership) --
//! it exists, and is tested, so the day shot or pass adopts this module the
//! rule is already in force for them, at the one call site, for free.
//!
//! ## AI release jitter: drawn once, never per tick
//!
//! [`draw_ai_threshold`] must be called at most once per charge, at the tick
//! [`commit_charge`] is called, from the caller's live `MatchState::rng`
//! stream, in stable per-tick player order. A per-tick re-roll would make
//! realized release timing a function of tick rate and would consume a
//! nondeterministic number of stream entries per charge -- both break
//! rollback. `Tackle`'s own wiring in this PR does not call it: the AI's
//! decision to chase a tackle is already made by `outfield_press`'s
//! contain-or-commit resolver before a charge ever begins, so a second,
//! in-charge danger-threshold reassessment has nothing to add for this verb
//! and this PR keeps tackle's RNG footprint exactly what it was. The trigger
//! and the draw are implemented and unit-tested here regardless, for the
//! next verb that needs an in-charge reassessment.

use gc_core::rng;
use gc_data::action_tuning::ActionVerb;

/// Which phase a player's committed action is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionPhase {
    /// No committed action.
    None,
    /// Winding up; releases on the first trigger in [`evaluate_release`]'s
    /// priority order.
    Charging,
    /// Committed and immune to every interrupt not on the verb's declared
    /// list (`gc_data::action_tuning::ActionInterruptList`).
    Executing,
    /// Resolved without success; input is gated (reduced movement, no new
    /// commit) until `remaining` reaches zero.
    Recovering,
}

/// One player's committed-action state. Flat and `Copy`; see the module doc
/// for why this shape was chosen over a data-carrying enum, and why it
/// counts seconds rather than ticks. Which fields are load-bearing depends
/// on `phase` -- [`validate_state`] pins every phase-irrelevant field to a
/// canonical zero, so two states that "mean" the same thing are
/// byte-identical, which is what makes this safe to diff and hash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionSlot {
    /// The committed verb. `None` exactly when `phase == ActionPhase::None`.
    pub verb: Option<ActionVerb>,
    /// The current phase.
    pub phase: ActionPhase,
    /// `Charging`: seconds held so far, counting up from `0.0` (unbounded
    /// above -- the timer-expiry trigger is what caps it in practice).
    /// Retained unchanged through `Executing` (progress bookkeeping for a
    /// future renderer); `0.0` in `None`/`Recovering`.
    pub charge_elapsed: f64,
    /// `Executing`: seconds until resolution, counting down to `0.0`.
    /// `Recovering`: seconds until input ungates, counting down to `0.0`.
    /// `0.0` in `None`/`Charging`.
    pub remaining: f64,
    /// `Executing`: the power released at, in `[power_floor, 1]`. `0.0`
    /// elsewhere.
    pub power: f64,
    /// The verb's aim/target hint, if it has one (e.g. tackle's chased
    /// carrier). `None` in `None`/`Recovering`.
    pub target_player: Option<i64>,
    /// This charge's once-drawn AI release-jitter threshold. Only
    /// meaningful for a verb/producer pairing that exercises
    /// [`ReleaseTriggerKind::AiThreshold`]; `0.0` when unused. `0.0` outside
    /// `Charging`/`Executing`.
    pub release_threshold: f64,
}

fn validate_state(state: &ActionSlot) {
    match state.phase {
        ActionPhase::None => {
            assert!(state.verb.is_none(), "an idle action slot cannot retain a verb");
            assert_eq!(
                state.charge_elapsed, 0.0,
                "an idle action slot cannot retain charge progress"
            );
            assert_eq!(state.remaining, 0.0, "an idle action slot cannot retain a countdown");
            assert_eq!(state.power, 0.0, "an idle action slot cannot retain power");
            assert!(state.target_player.is_none(), "an idle action slot cannot retain a target");
            assert_eq!(
                state.release_threshold, 0.0,
                "an idle action slot cannot retain a release threshold"
            );
        }
        ActionPhase::Charging => {
            assert!(state.verb.is_some(), "a charging action slot requires a verb");
            assert!(
                state.charge_elapsed.is_finite() && state.charge_elapsed >= 0.0,
                "charge progress must be non-negative and finite"
            );
            assert_eq!(state.remaining, 0.0, "a charging action slot cannot retain a countdown");
            assert_eq!(state.power, 0.0, "a charging action slot cannot retain released power");
            assert!(
                state.release_threshold.is_finite(),
                "release threshold must be finite"
            );
        }
        ActionPhase::Executing => {
            assert!(state.verb.is_some(), "an executing action slot requires a verb");
            assert!(
                state.charge_elapsed.is_finite() && state.charge_elapsed >= 0.0,
                "charge progress must be non-negative and finite"
            );
            assert!(
                state.remaining.is_finite() && state.remaining >= 0.0,
                "the resolve countdown must be non-negative and finite"
            );
            assert!(
                (0.0..=1.0).contains(&state.power),
                "released power must be in [0, 1]"
            );
            assert!(
                state.release_threshold.is_finite(),
                "release threshold must be finite"
            );
        }
        ActionPhase::Recovering => {
            assert!(state.verb.is_some(), "a recovering action slot requires a verb");
            assert_eq!(
                state.charge_elapsed, 0.0,
                "a recovering action slot cannot retain charge progress"
            );
            assert!(
                state.remaining.is_finite() && state.remaining >= 0.0,
                "a recovering action slot must have a non-negative countdown"
            );
            assert_eq!(state.power, 0.0, "a recovering action slot cannot retain power");
            assert!(state.target_player.is_none(), "a recovering action slot cannot retain a target");
            assert_eq!(
                state.release_threshold, 0.0,
                "a recovering action slot cannot retain a release threshold"
            );
        }
    }
    if let Some(target) = state.target_player {
        assert!(target >= 1, "action slot target must be a positive integer");
    }
}

/// A fresh, idle action slot.
#[must_use]
pub fn new_state() -> ActionSlot {
    ActionSlot {
        verb: None,
        phase: ActionPhase::None,
        charge_elapsed: 0.0,
        remaining: 0.0,
        power: 0.0,
        target_player: None,
        release_threshold: 0.0,
    }
}

/// Validate and defensively copy an action slot.
#[must_use]
pub fn copy_state(state: &ActionSlot) -> ActionSlot {
    validate_state(state);
    *state
}

/// The possession invariant, in one place: unconditionally clear a
/// committed action, from any phase, for any verb. Called on the OUTGOING
/// owner at the single ownership choke point
/// (`crate::r#match`'s `set_owner`), never per verb.
#[must_use]
pub fn clear(state: &ActionSlot) -> ActionSlot {
    let _ = copy_state(state);
    new_state()
}

/// Begin charging `verb`. May only be called from an idle slot -- the
/// caller (a producer's commit decision) is responsible for checking
/// `phase == ActionPhase::None` first, exactly like `AlreadyCommitted`
/// gating a combat commit.
///
/// `release_threshold` is this charge's once-drawn AI jitter (see
/// [`draw_ai_threshold`]), or `0.0` for a verb/producer that does not use
/// [`ReleaseTriggerKind::AiThreshold`].
///
/// # Panics
///
/// Panics if `state` is not idle.
#[must_use]
pub fn commit_charge(
    state: &ActionSlot,
    verb: ActionVerb,
    target_player: Option<i64>,
    release_threshold: f64,
) -> ActionSlot {
    let s = copy_state(state);
    assert_eq!(
        s.phase,
        ActionPhase::None,
        "a new charge may only start from an idle action slot"
    );
    let next = ActionSlot {
        verb: Some(verb),
        phase: ActionPhase::Charging,
        charge_elapsed: 0.0,
        remaining: 0.0,
        power: 0.0,
        target_player,
        release_threshold,
    };
    copy_state(&next)
}

/// Accumulate one tick of held time onto a charging slot.
///
/// # Panics
///
/// Panics if `state` is not charging, or `dt` is negative.
#[must_use]
pub fn advance_charge(state: &ActionSlot, dt: f64) -> ActionSlot {
    let mut s = copy_state(state);
    assert_eq!(s.phase, ActionPhase::Charging, "only a charging slot accumulates held time");
    assert!(dt.is_finite() && dt >= 0.0, "charge dt must be non-negative");
    s.charge_elapsed += dt;
    copy_state(&s)
}

/// The power mapping: floor at a tap (`held == 0`), monotone increasing,
/// capped at `1.0` once `held >= full_charge` (no overhold penalty). A
/// verb whose data declares `full_charge <= 0.0` (a legal tunable value --
/// several `full_charge`-shaped knobs in this codebase have a `min` of
/// `0.0`) has no wind-up at all: every release is instantly full power,
/// consistent with the cap this same function applies once `held` reaches
/// any positive `full_charge` anyway.
///
/// # Panics
///
/// Panics if `power_floor` is outside `[0, 1]`, or `held_seconds` is
/// negative or non-finite.
#[must_use]
pub fn power_at_release(held_seconds: f64, full_charge: f64, power_floor: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&power_floor),
        "power_floor must be in [0, 1]"
    );
    assert!(
        held_seconds.is_finite() && held_seconds >= 0.0,
        "held_seconds must be non-negative and finite"
    );
    let frac = if full_charge > 0.0 {
        (held_seconds / full_charge).min(1.0)
    } else {
        1.0
    };
    power_floor + (1.0 - power_floor) * frac
}

/// Release a charging slot into `Executing`, computing its power from
/// [`power_at_release`] and scheduling `remaining = commit_seconds`.
///
/// # Panics
///
/// Panics if `state` is not charging, or `commit_seconds` is negative.
#[must_use]
pub fn release(
    state: &ActionSlot,
    full_charge: f64,
    power_floor: f64,
    commit_seconds: f64,
) -> ActionSlot {
    let s = copy_state(state);
    assert_eq!(s.phase, ActionPhase::Charging, "only a charging slot can release");
    assert!(
        commit_seconds.is_finite() && commit_seconds >= 0.0,
        "commit duration must be non-negative"
    );
    let power = power_at_release(s.charge_elapsed, full_charge, power_floor);
    let next = ActionSlot {
        verb: s.verb,
        phase: ActionPhase::Executing,
        charge_elapsed: s.charge_elapsed,
        remaining: commit_seconds,
        power,
        target_player: s.target_player,
        release_threshold: s.release_threshold,
    };
    copy_state(&next)
}

/// Count an executing or recovering slot's countdown down by `dt`, clamped
/// at zero. Does not itself transition the phase -- the caller checks
/// [`due`] and calls [`resolve_success`]/[`resolve_miss`]/[`end_recovery`].
///
/// # Panics
///
/// Panics if `state` is idle or charging, or `dt` is negative.
#[must_use]
pub fn advance_remaining(state: &ActionSlot, dt: f64) -> ActionSlot {
    let mut s = copy_state(state);
    assert!(
        matches!(s.phase, ActionPhase::Executing | ActionPhase::Recovering),
        "only an executing or recovering slot counts down"
    );
    assert!(dt.is_finite() && dt >= 0.0, "dt must be non-negative");
    s.remaining = (s.remaining - dt).max(0.0);
    copy_state(&s)
}

/// Whether an executing or recovering slot's countdown has elapsed.
///
/// # Panics
///
/// Panics if `state` is idle or charging.
#[must_use]
pub fn due(state: &ActionSlot) -> bool {
    let s = copy_state(state);
    assert!(
        matches!(s.phase, ActionPhase::Executing | ActionPhase::Recovering),
        "only an executing or recovering slot has a countdown to be due"
    );
    s.remaining <= 0.0
}

/// Resolve an executing action as a SUCCESS: back to idle, no recovery.
///
/// # Panics
///
/// Panics if `state` is not executing.
#[must_use]
pub fn resolve_success(state: &ActionSlot) -> ActionSlot {
    let s = copy_state(state);
    assert_eq!(s.phase, ActionPhase::Executing, "only an executing slot resolves");
    new_state()
}

/// Resolve an executing action as a MISS: enter `Recovering` for
/// `recovery_seconds`.
///
/// # Panics
///
/// Panics if `state` is not executing, or `recovery_seconds` is not
/// positive.
#[must_use]
pub fn resolve_miss(state: &ActionSlot, recovery_seconds: f64) -> ActionSlot {
    let s = copy_state(state);
    assert_eq!(s.phase, ActionPhase::Executing, "only an executing slot resolves");
    assert!(
        recovery_seconds.is_finite() && recovery_seconds > 0.0,
        "a miss must owe a positive recovery window"
    );
    let next = ActionSlot {
        verb: s.verb,
        phase: ActionPhase::Recovering,
        charge_elapsed: 0.0,
        remaining: recovery_seconds,
        power: 0.0,
        target_player: None,
        release_threshold: 0.0,
    };
    copy_state(&next)
}

/// End a recovery window, back to idle. Callers should only do this once
/// [`due`] is true, though this function does not itself check the
/// countdown -- regaining control mid-recovery must stay gated (#489's own
/// requirement), and that gating is the caller's job: it must keep calling
/// [`due`] every tick and never call this early just because input
/// returned.
///
/// # Panics
///
/// Panics if `state` is not recovering.
#[must_use]
pub fn end_recovery(state: &ActionSlot) -> ActionSlot {
    let s = copy_state(state);
    assert_eq!(s.phase, ActionPhase::Recovering, "only a recovering slot ends a recovery");
    new_state()
}

/// Which release trigger fired. Priority order (see [`evaluate_release`]):
/// input release, then timer expiry, then arrival, then an AI danger
/// threshold crossing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseTriggerKind {
    /// The producer released the charge input this tick.
    InputRelease,
    /// The charge reached its `full_charge` cap this tick.
    TimerExpiry,
    /// The charge arrived at its computed target this tick (e.g. a tackle
    /// closing to contact range).
    Arrival,
    /// An AI danger score crossed this charge's once-drawn jittered
    /// threshold this tick.
    AiThreshold,
}

/// Inputs to one tick's release-trigger evaluation. Every field is a plain,
/// precomputed fact the caller measured this tick -- this function makes no
/// world queries of its own, so it stays pure and trivially testable in
/// combination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReleaseTriggerInputs {
    /// Trigger 1: the charge input released this tick.
    pub input_released: bool,
    /// Seconds held so far, INCLUDING this tick's own accumulation.
    pub held_seconds: f64,
    /// This verb's full-charge duration.
    pub full_charge: f64,
    /// Trigger 3: the charge arrived at its computed target this tick.
    pub arrived: bool,
    /// Trigger 4: the live AI danger score, when this charge is AI-driven
    /// and exercises the threshold trigger. `None` disables trigger 4
    /// entirely (the common case today -- see the module doc on `Tackle`).
    pub ai_danger: Option<f64>,
    /// This charge's once-drawn jittered threshold (`ActionSlot::release_threshold`).
    pub ai_threshold: f64,
}

/// Evaluate this tick's release triggers in FIXED, DOCUMENTED PRIORITY
/// ORDER -- (1) input release, (2) charge timer expiry, (3) arrival at a
/// computed target, (4) an AI danger-threshold crossing -- so simultaneous
/// triggers resolve identically on every peer. Returns the first trigger
/// that fires, or `None` if the charge should keep holding.
#[must_use]
pub fn evaluate_release(inputs: ReleaseTriggerInputs) -> Option<ReleaseTriggerKind> {
    if inputs.input_released {
        return Some(ReleaseTriggerKind::InputRelease);
    }
    if inputs.held_seconds >= inputs.full_charge {
        return Some(ReleaseTriggerKind::TimerExpiry);
    }
    if inputs.arrived {
        return Some(ReleaseTriggerKind::Arrival);
    }
    if let Some(danger) = inputs.ai_danger
        && danger >= inputs.ai_threshold
    {
        return Some(ReleaseTriggerKind::AiThreshold);
    }
    None
}

/// Draw this charge's AI release-threshold jitter ONCE, from the live match
/// RNG stream, at the tick [`commit_charge`] is called -- never per tick
/// (see the module doc). `base` is the verb's nominal threshold;
/// `spread` widens the jitter band symmetrically around it, clamped to
/// stay non-negative. Returns the advanced RNG state and the drawn
/// threshold.
///
/// # Panics
///
/// Panics if `spread` is negative.
#[must_use]
pub fn draw_ai_threshold(rng_state: u32, base: f64, spread: f64) -> (u32, f64) {
    assert!(spread >= 0.0, "jitter spread must be non-negative");
    let (next_state, sample) = rng::roll(rng_state);
    let threshold = base + (sample - 0.5) * 2.0 * spread;
    (next_state, threshold.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tackle() -> ActionVerb {
        ActionVerb::Tackle
    }

    #[test]
    fn new_state_is_idle_and_valid() {
        let s = new_state();
        assert_eq!(s.phase, ActionPhase::None);
        assert!(s.verb.is_none());
        let _ = copy_state(&s); // does not panic
    }

    #[test]
    fn clear_resets_from_every_phase() {
        let charging = commit_charge(&new_state(), tackle(), Some(3), 0.0);
        assert_eq!(clear(&charging), new_state());

        let executing = release(&charging, 0.2, 0.5, 0.3);
        assert_eq!(clear(&executing), new_state());

        let recovering = resolve_miss(&executing, 0.4);
        assert_eq!(clear(&recovering), new_state());

        // Idempotent: clearing an already-idle slot is still idle.
        assert_eq!(clear(&new_state()), new_state());
    }

    #[test]
    fn commit_charge_requires_idle() {
        let charging = commit_charge(&new_state(), tackle(), None, 0.0);
        let result = std::panic::catch_unwind(|| commit_charge(&charging, tackle(), None, 0.0));
        assert!(result.is_err(), "committing over an in-progress action must panic");
    }

    #[test]
    fn power_mapping_floors_at_tap_and_caps_at_overhold_and_is_monotone() {
        let full_charge = 0.2;
        let floor = 0.4;
        let tap = power_at_release(0.0, full_charge, floor);
        assert_eq!(tap, floor, "a tap (zero hold) must release at exactly the floor");

        let capped = power_at_release(full_charge, full_charge, floor);
        assert_eq!(capped, 1.0, "reaching full_charge must cap power at 1.0");

        let overheld = power_at_release(full_charge * 10.0, full_charge, floor);
        assert_eq!(overheld, 1.0, "overholding past full_charge must not exceed 1.0");

        let mut last = tap;
        for i in 1..=10 {
            let held = full_charge * f64::from(i) / 10.0;
            let p = power_at_release(held, full_charge, floor);
            assert!(p >= last, "power must be monotone non-decreasing in held time");
            last = p;
        }
    }

    #[test]
    fn release_computes_power_and_schedules_remaining() {
        let charging = commit_charge(&new_state(), tackle(), Some(7), 0.0);
        let charging = advance_charge(&charging, 0.1);
        let executing = release(&charging, 0.2, 0.5, 0.18);
        assert_eq!(executing.phase, ActionPhase::Executing);
        assert_eq!(executing.verb, Some(tackle()));
        assert_eq!(executing.remaining, 0.18);
        assert_eq!(executing.charge_elapsed, 0.1);
        // held 0.1 of a 0.2 full charge, floor 0.5: 0.5 + 0.5*0.5 = 0.75
        assert!((executing.power - 0.75).abs() < 1e-12);
    }

    #[test]
    fn resolve_success_returns_to_idle_with_no_recovery() {
        let charging = commit_charge(&new_state(), tackle(), None, 0.0);
        let executing = release(&charging, 0.2, 0.5, 0.1);
        assert_eq!(resolve_success(&executing), new_state());
    }

    #[test]
    fn resolve_miss_enters_recovering_until_the_countdown_elapses() {
        let charging = commit_charge(&new_state(), tackle(), None, 0.0);
        let executing = release(&charging, 0.2, 0.5, 0.1);
        let mut recovering = resolve_miss(&executing, 0.3);
        assert_eq!(recovering.phase, ActionPhase::Recovering);
        assert_eq!(recovering.remaining, 0.3);
        assert!(!due(&recovering));
        for _ in 0..3 {
            recovering = advance_remaining(&recovering, 0.1);
        }
        assert!(due(&recovering));
        assert_eq!(end_recovery(&recovering), new_state());
    }

    #[test]
    fn regaining_input_mid_recovery_stays_gated() {
        // #489: "regaining control during recovering stays gated until the
        // window ends" -- modelled here as: the caller must keep observing
        // due() == false and must not call end_recovery early, no matter
        // how many ticks of (simulated) input arrive in between.
        let charging = commit_charge(&new_state(), tackle(), None, 0.0);
        let executing = release(&charging, 0.2, 0.5, 0.1);
        let mut recovering = resolve_miss(&executing, 0.3);
        for _ in 0..29 {
            recovering = advance_remaining(&recovering, 0.01);
            assert!(
                !due(&recovering),
                "recovery must stay gated for the whole window regardless of input"
            );
        }
        recovering = advance_remaining(&recovering, 0.01);
        assert!(due(&recovering));
    }

    #[test]
    fn input_loss_completes_the_charge_on_its_original_schedule() {
        // #489: "losing control mid-action completes on the original
        // schedule." Nothing in advance_charge/release/resolve reads a
        // continued input signal after commit_charge -- advancing and
        // releasing a charge here with NO further "input" argument at all
        // demonstrates the schedule is input-independent by construction.
        let charging = commit_charge(&new_state(), tackle(), None, 0.0);
        let mut s = charging;
        for _ in 0..3 {
            s = advance_charge(&s, 0.05);
        }
        assert!(
            (s.charge_elapsed - 0.15).abs() < 1e-12,
            "held time accrues regardless of input control"
        );
        let executing = release(&s, 0.2, 0.5, 0.22);
        assert_eq!(executing.remaining, 0.22);
    }

    #[test]
    fn release_trigger_priority_is_fixed() {
        // All four true at once: input release must win.
        let all = ReleaseTriggerInputs {
            input_released: true,
            held_seconds: 1.0,
            full_charge: 0.2,
            arrived: true,
            ai_danger: Some(10.0),
            ai_threshold: 1.0,
        };
        assert_eq!(evaluate_release(all), Some(ReleaseTriggerKind::InputRelease));

        // No input release: timer expiry beats arrival and AI threshold.
        let timer_and_below = ReleaseTriggerInputs {
            input_released: false,
            ..all
        };
        assert_eq!(evaluate_release(timer_and_below), Some(ReleaseTriggerKind::TimerExpiry));

        // No input release, held below full_charge: arrival beats AI threshold.
        let arrival_and_below = ReleaseTriggerInputs {
            input_released: false,
            held_seconds: 0.05,
            ..all
        };
        assert_eq!(evaluate_release(arrival_and_below), Some(ReleaseTriggerKind::Arrival));

        // Only the AI threshold clears.
        let ai_only = ReleaseTriggerInputs {
            input_released: false,
            held_seconds: 0.05,
            arrived: false,
            ai_danger: Some(2.0),
            ai_threshold: 1.0,
            full_charge: 0.2,
        };
        assert_eq!(evaluate_release(ai_only), Some(ReleaseTriggerKind::AiThreshold));

        // Nothing fires: keep holding.
        let none_fire = ReleaseTriggerInputs {
            input_released: false,
            held_seconds: 0.05,
            full_charge: 0.2,
            arrived: false,
            ai_danger: Some(0.1),
            ai_threshold: 1.0,
        };
        assert_eq!(evaluate_release(none_fire), None);
    }

    #[test]
    fn release_trigger_ai_threshold_disabled_when_danger_is_none() {
        let inputs = ReleaseTriggerInputs {
            input_released: false,
            held_seconds: 0.05,
            full_charge: 0.2,
            arrived: false,
            ai_danger: None,
            ai_threshold: 0.0,
        };
        assert_eq!(evaluate_release(inputs), None);
    }

    #[test]
    fn ai_threshold_draw_is_a_pure_function_of_rng_state() {
        let (state_a, threshold_a) = draw_ai_threshold(12345, 5.0, 1.0);
        let (state_b, threshold_b) = draw_ai_threshold(12345, 5.0, 1.0);
        assert_eq!(state_a, state_b);
        assert_eq!(threshold_a, threshold_b);
        // A different starting state must (almost certainly) draw
        // differently -- proving the draw actually consumes the stream
        // rather than returning a constant.
        let (state_c, threshold_c) = draw_ai_threshold(999_999, 5.0, 1.0);
        assert!(state_a != state_c || threshold_a != threshold_c);
    }

    #[test]
    fn ai_threshold_draw_advances_the_rng_by_exactly_one_roll() {
        let (next_from_draw, _) = draw_ai_threshold(777, 0.0, 0.0);
        let (next_from_roll, _) = rng::roll(777);
        assert_eq!(next_from_draw, next_from_roll);
    }
}
