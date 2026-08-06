//! Port of `sim/combat_policy.lua`.
//!
//! `gameplay_ai/combat/v1`: the deterministic shipped gameplay combat
//! policy.
//!
//! It consumes only `combat_sim_observation/v1` and produces one scored
//! choice on the existing [`crate::brain::BrainScoredOption`] seam, so
//! combat extends the milestone-9 decision contract instead of forming a
//! second AI stack. It never calls a resolver internal and never sets an
//! outcome: an accepted decision becomes three abstract equipment signals
//! on an ordinary `MatchInput`, exactly like a human producer.
//!
//! Soccer value comes first. A candidate exists ONLY when one of the five
//! section 4.6 purpose predicates is true for the (source, target) pair AND
//! `family_commit_feasibility/v1` is true for the equipped family toward
//! that frozen identity. Purposeless off-ball harassment is therefore
//! structurally unreachable rather than merely discouraged, and every
//! accepted commit is inside `intervention_candidate/v2` by construction:
//! the zero-tick witness tape is one of the poses that envelope searches.
//!
//! Exactly one stable reason leaves this module per decision: one of the
//! five purpose ids, or `decline` when the episode closes with no action. A
//! commit with no true predicate or an infeasible target is
//! `unattributed_off_ball` and additionally raises
//! `representative_policy_context_violation`.

use crate::brain::{self, BrainPayloadValue, BrainScoredOption, OptionReference};
use crate::combat_feasibility::{self, CombatPurposeId};
use crate::combat_intent::CombatDecisionReason;
use crate::combat_observation::{self, CombatObservation};
use crate::fixed_clock;
use gc_data::action_families::{self, ActionFamilyId};
use indexmap::IndexMap;

/// The deterministic shipped gameplay combat policy id.
pub const POLICY_ID: &str = "gameplay_ai/combat/v1";
/// The same identity rendered for the session manifest wire.
pub const MANIFEST_POLICY_ID: &str = "gameplay_ai.combat.v1";

/// Declining has to beat a typical OFF-BALL candidate and lose to a real
/// ball contest. Everything above this line commits, so the number is the
/// whole "off-ball attacks are rare, ball contests are normal" policy in
/// one place.
pub const DECLINE_BASELINE: f64 = 45.0;
/// Distance range over which ball-value weight is scaled.
pub const BALL_VALUE_RANGE: f64 = 220.0;
/// Weight applied to `1 - clamp(target_ball_distance / BALL_VALUE_RANGE)`.
pub const BALL_VALUE_WEIGHT: f64 = 30.0;
/// Bonus for a ball-spilling hit on the actual carrier.
pub const TURNOVER_BONUS: f64 = 16.0;
/// Penalty per teammate already contesting the same target.
pub const COVERAGE_PENALTY: f64 = 13.0;
/// Weight applied to the commitment-ticks cost.
pub const COMMITMENT_WEIGHT: f64 = 26.0;
/// Penalty for raising `formation_risk_tradeoff` off the two on-ball
/// purposes.
pub const FORMATION_RISK_PENALTY: f64 = 18.0;
/// Ticks beyond which a guard's threat earns no urgency bonus.
pub const GUARD_URGENCY_TICKS: f64 = 24.0;
/// Weight applied to the guard urgency bonus.
pub const GUARD_URGENCY_WEIGHT: f64 = 34.0;
/// Distance inside which a mid-committed teammate counts as coverage.
pub const COVERAGE_RANGE_PX: f64 = 56.0;
/// Base softmax temperature for an ordinary combat decision.
pub const BASE_TEMPERATURE: f64 = 3.0;
/// Composure at or above this takes the exact argmax and spends no RNG,
/// mirroring `outfield_decision::decide_carrier`'s authored boundary.
pub const SHARP_COMPOSURE: f64 = 0.8;
/// A telegraphed threat landing inside this window earns the existing
/// sidestep rather than a commit.
pub const EVADE_WINDOW_TICKS: i64 = 12;
/// The minimum lead that keeps the AI honest: it reads a telegraph it could
/// plausibly have seen, never snaps out of an already-landing swing.
pub const EVADE_MIN_LEAD_TICKS: i64 = 4;

/// Why a decision could not even be attempted this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatUnavailableReason {
    /// The player is a keeper, or has no combat loadout.
    NoLoadout,
    /// A soccer action already claims priority this tick.
    SoccerCommitment,
    /// The player is mid-aerial or its recovery.
    AerialOrRecovery,
    /// The player is in a forced (stagger/knockback) state.
    Forced,
    /// The player already has an action committed.
    AlreadyCommitted,
    /// The player's action is still cooling down.
    Cooldown,
    /// No candidate cleared `family_commit_feasibility/v1`.
    FamilyCommitFeasibility,
}

/// The action a decision resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyAction {
    /// A commit was chosen.
    Commit,
    /// The episode had a ready tick and chose not to act.
    Decline,
    /// The episode was never action-ready.
    Unavailable,
}

/// One loadout-aware, feasibility-proven candidate for [`decide`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatPolicyCandidate {
    /// The candidate target's canonical player index.
    pub target_player: i64,
    /// The purpose that made this pair eligible.
    pub purpose: CombatPurposeId,
    /// The source's currently equipped family.
    pub family_id: ActionFamilyId,
    /// Target to ball, world px.
    pub target_ball_distance: f64,
    /// Source to ball, world px.
    pub source_ball_distance: f64,
    /// Whether the target is the current ball carrier.
    pub target_is_carrier: bool,
    /// Whether the source's own team currently owns the ball.
    pub team_owns_ball: bool,
    /// Whether the equipped family's unguarded outcome spills the ball.
    pub spills_ball: bool,
    /// Same-team outfielders already contesting this target.
    pub teammate_coverage: i64,
    /// Whether raising this commit would raise `formation_risk_tradeoff`.
    pub formation_risk: bool,
    /// Windup + active + recovery + cooldown ticks.
    pub commitment_ticks: i64,
    /// Ticks until the guarded threat lands; 0 when not guarding.
    pub guard_threat_ticks: i64,
}

/// One resolved combat decision.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatPolicyDecision {
    /// The resolved action.
    pub action: PolicyAction,
    /// The stable reason, always present.
    pub reason: CombatDecisionReason,
    /// The chosen family, when `action == Commit`.
    pub family_id: Option<ActionFamilyId>,
    /// The chosen target, when `action == Commit`.
    pub target_player: Option<i64>,
    /// Whether the chosen commit raises `formation_risk_tradeoff`.
    pub formation_risk: bool,
    /// Why the decision was unavailable, when `action == Unavailable`.
    pub unavailable_reason: Option<CombatUnavailableReason>,
    /// `representative_policy_context_violation`.
    pub context_violation: bool,
    /// The RNG stream state after this decision.
    pub rng_state: u32,
    /// The observation's digest this decision was made on.
    pub digest: String,
    /// The selected option's id, if a selection happened.
    pub option_id: Option<String>,
}

fn length(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

/// `windup + active + recovery + cooldown` ticks for `family_id`.
#[must_use]
pub fn commitment_ticks(family_id: ActionFamilyId) -> i64 {
    let family = action_families::get(family_id);
    family.windup_ticks
        + family.active_ticks.unwrap_or(0)
        + family.recovery_ticks
        + family.cooldown_ticks
}

fn purpose_wire(purpose: CombatPurposeId) -> &'static str {
    match purpose {
        CombatPurposeId::CarrierContest => "carrier_contest",
        CombatPurposeId::CarrierProtection => "carrier_protection",
        CombatPurposeId::LooseBallContest => "loose_ball_contest",
        CombatPurposeId::PassingLaneOrShotDenial => "passing_lane_or_shot_denial",
        CombatPurposeId::RecoveryPunish => "recovery_punish",
    }
}

fn purpose_to_reason(purpose: CombatPurposeId) -> CombatDecisionReason {
    match purpose {
        CombatPurposeId::CarrierContest => CombatDecisionReason::CarrierContest,
        CombatPurposeId::CarrierProtection => CombatDecisionReason::CarrierProtection,
        CombatPurposeId::LooseBallContest => CombatDecisionReason::LooseBallContest,
        CombatPurposeId::PassingLaneOrShotDenial => CombatDecisionReason::PassingLaneOrShotDenial,
        CombatPurposeId::RecoveryPunish => CombatDecisionReason::RecoveryPunish,
    }
}

/// `"<purpose>:<target_player>"`, the candidate's option id on the shared
/// selection seam.
#[must_use]
pub fn option_id(candidate: &CombatPolicyCandidate) -> String {
    format!(
        "{}:{}",
        purpose_wire(candidate.purpose),
        candidate.target_player
    )
}

fn purpose_base(purpose: CombatPurposeId) -> f64 {
    match purpose {
        CombatPurposeId::RecoveryPunish => 78.0,
        CombatPurposeId::CarrierContest => 72.0,
        CombatPurposeId::LooseBallContest => 58.0,
        CombatPurposeId::PassingLaneOrShotDenial => 46.0,
        CombatPurposeId::CarrierProtection => 44.0,
    }
}

fn is_on_ball_purpose(purpose: CombatPurposeId) -> bool {
    matches!(
        purpose,
        CombatPurposeId::CarrierContest | CombatPurposeId::LooseBallContest
    )
}

/// Pure scoring. Soccer value first: how close the contested body is to the
/// ball, whether winning it is a real turnover, whether a teammate already
/// covers it, what the commitment costs, and what it does to the
/// formation.
#[must_use]
pub fn score(candidate: &CombatPolicyCandidate) -> f64 {
    let mut score = purpose_base(candidate.purpose);
    score += (1.0 - (candidate.target_ball_distance / BALL_VALUE_RANGE).clamp(0.0, 1.0))
        * BALL_VALUE_WEIGHT;
    if candidate.target_is_carrier && candidate.spills_ball {
        score += TURNOVER_BONUS;
    }
    score -= candidate.teammate_coverage.max(0) as f64 * COVERAGE_PENALTY;
    score -= (candidate.commitment_ticks as f64 / fixed_clock::TICK_RATE) * COMMITMENT_WEIGHT;
    if candidate.formation_risk && !is_on_ball_purpose(candidate.purpose) {
        score -= FORMATION_RISK_PENALTY;
    }
    if candidate.family_id == ActionFamilyId::Guard {
        let urgency =
            1.0 - (candidate.guard_threat_ticks as f64 / GUARD_URGENCY_TICKS).clamp(0.0, 1.0);
        score += urgency * GUARD_URGENCY_WEIGHT;
    }
    score
}

/// Build the scored option set. The `decline` option is always present, so
/// the selector always has at least one option and a weak candidate loses
/// to doing nothing rather than being clamped away.
#[must_use]
pub fn options(candidates: &[CombatPolicyCandidate]) -> Vec<BrainScoredOption> {
    let mut options = vec![BrainScoredOption {
        id: "decline".to_string(),
        kind: "decline".to_string(),
        score: DECLINE_BASELINE,
        payload: None,
        reference: None,
    }];
    for candidate in candidates {
        let mut payload = IndexMap::new();
        payload.insert(
            "target_player".to_string(),
            BrainPayloadValue::Number(candidate.target_player as f64),
        );
        payload.insert(
            "purpose".to_string(),
            BrainPayloadValue::Text(purpose_wire(candidate.purpose).to_string()),
        );
        options.push(BrainScoredOption {
            id: option_id(candidate),
            kind: if candidate.family_id == ActionFamilyId::Guard {
                "guard".to_string()
            } else {
                "commit".to_string()
            },
            score: score(candidate),
            payload: Some(payload),
            reference: Some(OptionReference::Index(candidate.target_player as u32)),
        });
    }
    options
}

fn unavailable_reason(observation: &CombatObservation) -> Option<CombatUnavailableReason> {
    let own = &observation.own;
    if own.is_keeper || own.family_id.is_none() {
        return Some(CombatUnavailableReason::NoLoadout);
    }
    if own.forced_ticks > 0 {
        return Some(CombatUnavailableReason::Forced);
    }
    if own.phase != crate::combat_feasibility::CombatActionPhase::Ready {
        return Some(CombatUnavailableReason::AlreadyCommitted);
    }
    if own.soccer_action == combat_observation::SoccerAction::Aerial {
        return Some(CombatUnavailableReason::AerialOrRecovery);
    }
    if own.soccer_committed {
        return Some(CombatUnavailableReason::SoccerCommitment);
    }
    if own.cooldown_ticks > 0 {
        return Some(CombatUnavailableReason::Cooldown);
    }
    None
}

fn teammate_coverage(observation: &CombatObservation, target: &combat_observation::PeerRow) -> i64 {
    let mut count = 0;
    for mate in &observation.teammates {
        if !mate.is_keeper
            && mate.phase != crate::combat_feasibility::CombatActionPhase::Ready
            && length(mate.x - target.x, mate.y - target.y) <= COVERAGE_RANGE_PX
        {
            count += 1;
        }
    }
    count
}

fn guard_threat_ticks(
    feasibility: &combat_feasibility::CombatObservation,
    source_index: i64,
) -> i64 {
    let mut best = i64::MAX;
    for path in combat_feasibility::hostile_paths(feasibility, Some(source_index)) {
        best = best.min(path.start_tick);
    }
    if best == i64::MAX {
        GUARD_URGENCY_TICKS as i64
    } else {
        best
    }
}

/// Every loadout-aware candidate for this decision tick. A pair is a
/// candidate only when a purpose predicate holds and the EQUIPPED family
/// can reach it from the current pose; the zero-tick witness tape keeps the
/// result inside `intervention_candidate/v2` without re-running its search
/// every tick.
#[must_use]
pub fn candidates(observation: &CombatObservation) -> Vec<CombatPolicyCandidate> {
    let Some(family_id) = observation.own.family_id else {
        return Vec::new();
    };
    let feasibility = combat_observation::to_feasibility_view(observation);
    let family = action_families::get(family_id);
    let spills = family
        .unguarded_outcome
        .as_ref()
        .is_some_and(|o| o.ball_spill);
    let commitment = commitment_ticks(family_id);
    let formation_risk = combat_feasibility::formation_risk(&feasibility, family_id);
    let own = &observation.own;

    let mut result = Vec::new();
    for target in &observation.opponents {
        if target.is_keeper {
            continue;
        }
        let (bitset, count) = combat_feasibility::purpose_bitset(&feasibility, target.player_index);
        if count <= 0 {
            continue;
        }
        let witness =
            combat_feasibility::family_commit(&feasibility, target.player_index, family_id, None);
        if !witness.feasible {
            continue;
        }
        let purpose = combat_feasibility::dominant_purpose(bitset)
            .expect("a positive purpose count has a dominant purpose");
        result.push(CombatPolicyCandidate {
            target_player: target.player_index,
            purpose,
            family_id,
            target_ball_distance: length(
                target.x - observation.ball.x,
                target.y - observation.ball.y,
            ),
            source_ball_distance: length(own.x - observation.ball.x, own.y - observation.ball.y),
            target_is_carrier: observation.ball.owner_player_index == target.player_index,
            team_owns_ball: observation.ball.owner_team == Some(own.team),
            spills_ball: spills,
            teammate_coverage: teammate_coverage(observation, target),
            formation_risk,
            commitment_ticks: commitment,
            guard_threat_ticks: if family_id == ActionFamilyId::Guard {
                guard_threat_ticks(&feasibility, target.player_index)
            } else {
                0
            },
        });
    }
    result.sort_by_key(|c| c.target_player);
    result
}

/// Resolve one decision. `temperature` is caller-owned: the gameplay caller
/// derives it from the deciding player's own composure, exactly as
/// `outfield_decision::decide_carrier` does, so a settled player takes the
/// exact argmax and consumes no RNG.
///
/// # Panics
///
/// Panics if `observation` is not a `combat_sim_observation/v1` (schema
/// and version tag check only — the full allowlist pass lives in the
/// specs/[`combat_observation::validate`]).
#[must_use]
pub fn decide(
    observation: &CombatObservation,
    rng_state: u32,
    temperature: Option<f64>,
) -> CombatPolicyDecision {
    assert!(
        observation.schema == combat_observation::SCHEMA
            && observation.version == combat_observation::VERSION,
        "gameplay_ai/combat/v1 consumes only combat_sim_observation/v1"
    );
    let mut decision = CombatPolicyDecision {
        action: PolicyAction::Decline,
        reason: CombatDecisionReason::Decline,
        family_id: None,
        target_player: None,
        formation_risk: false,
        unavailable_reason: None,
        context_violation: false,
        rng_state,
        digest: observation.digest.clone(),
        option_id: None,
    };

    if let Some(blocked) = unavailable_reason(observation) {
        decision.action = PolicyAction::Unavailable;
        decision.reason = CombatDecisionReason::None;
        decision.unavailable_reason = Some(blocked);
        return decision;
    }
    if observation.match_row.stoppage || observation.match_row.finished {
        decision.action = PolicyAction::Unavailable;
        decision.reason = CombatDecisionReason::None;
        decision.unavailable_reason = Some(CombatUnavailableReason::SoccerCommitment);
        return decision;
    }

    let candidates = candidates(observation);
    if candidates.is_empty() {
        decision.action = PolicyAction::Unavailable;
        decision.reason = CombatDecisionReason::None;
        decision.unavailable_reason = Some(CombatUnavailableReason::FamilyCommitFeasibility);
        return decision;
    }

    let options = options(&candidates);
    let (selected, next_rng) =
        brain::select_scored_option(&options, temperature.unwrap_or(0.0), rng_state);
    decision.rng_state = next_rng;
    decision.option_id = Some(selected.id.clone());
    if selected.kind == "decline" {
        return decision;
    }

    assert!(
        matches!(selected.reference, Some(OptionReference::Index(_))),
        "a combat commit needs a numeric target"
    );
    let chosen = candidates
        .iter()
        .find(|c| option_id(c) == selected.id)
        .expect("the selected combat option left the candidate set");

    decision.action = PolicyAction::Commit;
    decision.family_id = Some(chosen.family_id);
    decision.target_player = Some(chosen.target_player);
    decision.formation_risk = chosen.formation_risk;
    decision.reason = purpose_to_reason(chosen.purpose);

    let feasibility = combat_observation::to_feasibility_view(observation);
    let (bitset, count) = combat_feasibility::purpose_bitset(&feasibility, chosen.target_player);
    if count == 0 || !bitset.get(chosen.purpose) {
        decision.reason = CombatDecisionReason::UnattributedOffBall;
        decision.context_violation = true;
    }
    decision
}

/// Guard is held: the policy owes a hold long enough to cover the threat it
/// is answering, then one release edge.
#[must_use]
pub fn hold_ticks(observation: &CombatObservation, decision: &CombatPolicyDecision) -> i64 {
    let (Some(ActionFamilyId::Guard), Some(target_player)) =
        (decision.family_id, decision.target_player)
    else {
        return 0;
    };
    let feasibility = combat_observation::to_feasibility_view(observation);
    let mut last = action_families::get(ActionFamilyId::Guard).windup_ticks;
    for path in combat_feasibility::hostile_paths(&feasibility, Some(target_player)) {
        last = last.max(path.last_tick);
    }
    last
}

/// Whether `reason` is one of the five purpose commit reasons.
#[must_use]
pub fn is_commit_reason(reason: CombatDecisionReason) -> bool {
    reason.is_commit_reason()
}
