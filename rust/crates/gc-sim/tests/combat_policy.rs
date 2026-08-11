//! Tests for `gc_sim::combat_policy`.
//!
//! `gameplay_ai/combat/v1` scoring, reason vocabulary, and availability.
//!
//! Scoring is pure: it takes a candidate record, not a world. That is what
//! lets the soccer-value ordering be asserted directly instead of inferred
//! from whichever match happened to be simulated. The "decisions" tests
//! build their fixture via `bare_match()` below, which calls
//! `sim_match::new`.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_sim::brain::{self, OptionReference};
use gc_sim::combat::{self};
use gc_sim::combat_feasibility::{CombatActionPhase, CombatPurposeId as Purpose};
use gc_sim::combat_intent::{self, CombatDecisionReason};
use gc_sim::combat_observation::{self, CombatObservation};
use gc_sim::combat_policy::{self, CombatPolicyCandidate, CombatUnavailableReason, PolicyAction};
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchState, PitchSize, Team};

/// The `bare_match()` fixture's fixed source/target player indices
/// (1-based, matching `crate::r#match`'s indexing convention — see that
/// module's "Indexing convention" doc). `SOURCE` sits on the home roster
/// (nebula), `TARGET` on the away roster (orion).
const SOURCE: i64 = 2;
const TARGET: i64 = 7;

/// A fresh 5v5 fixture with every player parked on a diagonal, facing their
/// attacking direction, and the ball sitting untouched in the corner — a
/// blank canvas the "decisions" cases pose explicitly rather than relying on
/// kickoff placement.
fn bare_match() -> (MatchState, CombatMatchState) {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(19.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    for (zero_index, player) in state.players.iter_mut().enumerate() {
        let index = (zero_index + 1) as f64;
        player.pos = Vec2::new(40.0 + index * 3.0, 40.0 + index * 3.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.facing = Vec2::new(if player.team == Team::Home { 1.0 } else { -1.0 }, 0.0);
    }
    state.ball = Vec2::new(900.0, 500.0);
    state.owner = None;
    (state, combat_state)
}

/// Builds a `CombatObservation` for `SOURCE` from `state`/`combat_state`.
fn observe(state: &MatchState, combat_state: &CombatMatchState) -> CombatObservation {
    combat_observation::build(
        state,
        Some(combat_state),
        SOURCE,
        combat_policy::POLICY_ID,
        None,
    )
}

/// `CombatPolicyCandidate` in its commit-a-carrier-contest baseline shape,
/// so each test only names the fields it varies.
fn candidate(
    purpose: Purpose,
    family_id: ActionFamilyId,
    target_ball_distance: f64,
    teammate_coverage: i64,
    formation_risk: bool,
    guard_threat_ticks: i64,
    commitment_ticks: i64,
) -> CombatPolicyCandidate {
    CombatPolicyCandidate {
        target_player: 7,
        purpose,
        family_id,
        target_ball_distance,
        source_ball_distance: 30.0,
        target_is_carrier: true,
        team_owns_ball: false,
        spills_ball: true,
        teammate_coverage,
        formation_risk,
        commitment_ticks,
        guard_threat_ticks,
    }
}

fn base_candidate() -> CombatPolicyCandidate {
    candidate(
        Purpose::CarrierContest,
        ActionFamilyId::Unarmed,
        0.0,
        0,
        false,
        0,
        combat_policy::commitment_ticks(ActionFamilyId::Unarmed),
    )
}

#[test]
fn scoring_puts_a_carrier_contest_above_every_other_purpose_from_the_same_geometry() {
    let mut c = base_candidate();
    c.purpose = Purpose::CarrierContest;
    let carrier_contest = combat_policy::score(&c);
    c.purpose = Purpose::LooseBallContest;
    let loose_ball_contest = combat_policy::score(&c);
    c.purpose = Purpose::PassingLaneOrShotDenial;
    let passing_lane = combat_policy::score(&c);
    c.purpose = Purpose::CarrierProtection;
    let carrier_protection = combat_policy::score(&c);

    assert!(carrier_contest > loose_ball_contest);
    assert!(loose_ball_contest > passing_lane);
    assert!(passing_lane > carrier_protection);

    c.purpose = Purpose::RecoveryPunish;
    assert!(combat_policy::score(&c) > carrier_contest);
}

#[test]
fn scoring_prefers_the_contest_closer_to_the_ball() {
    let mut near = base_candidate();
    near.target_ball_distance = 0.0;
    let mut far = base_candidate();
    far.target_ball_distance = 400.0;
    assert!(combat_policy::score(&near) > combat_policy::score(&far));
}

#[test]
fn scoring_discounts_a_target_a_teammate_already_covers() {
    let mut uncovered = base_candidate();
    uncovered.teammate_coverage = 0;
    let mut covered = base_candidate();
    covered.teammate_coverage = 2;
    assert!(combat_policy::score(&uncovered) > combat_policy::score(&covered));
}

#[test]
fn scoring_charges_the_whole_commitment_so_a_cheap_family_beats_an_expensive_one() {
    let mut unarmed = base_candidate();
    unarmed.family_id = ActionFamilyId::Unarmed;
    unarmed.commitment_ticks = combat_policy::commitment_ticks(ActionFamilyId::Unarmed);
    let mut ranged = base_candidate();
    ranged.family_id = ActionFamilyId::Ranged;
    ranged.commitment_ticks = combat_policy::commitment_ticks(ActionFamilyId::Ranged);
    assert!(combat_policy::score(&unarmed) > combat_policy::score(&ranged));

    assert_eq!(
        combat_policy::commitment_ticks(ActionFamilyId::Unarmed),
        6 + 4 + 12 + 24
    );
    assert_eq!(
        combat_policy::commitment_ticks(ActionFamilyId::LightMelee),
        12 + 5 + 21 + 42
    );
    assert_eq!(
        combat_policy::commitment_ticks(ActionFamilyId::Ranged),
        18 + 1 + 27 + 60
    );
    // Written as the literal `windup + active + recovery + cooldown` sum
    // (active/cooldown are zero for guard) rather than pre-folded, so a
    // reader can check it against the family definition term by term.
    #[allow(clippy::identity_op)]
    let guard_expected = 6 + 0 + 9 + 0;
    assert_eq!(
        combat_policy::commitment_ticks(ActionFamilyId::Guard),
        guard_expected
    );
}

#[test]
fn scoring_charges_the_formation_risk_flag_as_a_cost_and_never_as_a_purpose() {
    let mut safe = base_candidate();
    safe.purpose = Purpose::CarrierProtection;
    safe.formation_risk = false;
    let mut risky = base_candidate();
    risky.purpose = Purpose::CarrierProtection;
    risky.formation_risk = true;
    assert_eq!(
        combat_policy::score(&safe) - combat_policy::score(&risky),
        combat_policy::FORMATION_RISK_PENALTY
    );

    // Chasing the ball IS leaving the anchor, so the flag is still raised
    // and reported for an on-ball purpose but does not veto the contest.
    let mut on_ball_risky = base_candidate();
    on_ball_risky.formation_risk = true;
    let mut on_ball_safe = base_candidate();
    on_ball_safe.formation_risk = false;
    assert_eq!(
        combat_policy::score(&on_ball_risky),
        combat_policy::score(&on_ball_safe)
    );
}

#[test]
fn scoring_keeps_a_pure_off_ball_candidate_below_the_decline_baseline() {
    // Lane denial, away from the ball, with the source already pulled off
    // its anchor: exactly the "purposeless harassment" shape the issue asks
    // to make rare.
    let mut harassment = base_candidate();
    harassment.purpose = Purpose::PassingLaneOrShotDenial;
    harassment.target_ball_distance = 300.0;
    harassment.target_is_carrier = false;
    harassment.formation_risk = true;
    assert!(combat_policy::score(&harassment) < combat_policy::DECLINE_BASELINE);

    // The same denial from a disciplined position, close to the ball, is a
    // real option rather than a forbidden one.
    let mut disciplined = base_candidate();
    disciplined.purpose = Purpose::PassingLaneOrShotDenial;
    disciplined.target_ball_distance = 40.0;
    disciplined.target_is_carrier = false;
    disciplined.formation_risk = false;
    assert!(combat_policy::score(&disciplined) > combat_policy::DECLINE_BASELINE);
}

#[test]
fn scoring_raises_a_guard_the_closer_its_answered_threat_is() {
    let mut imminent = base_candidate();
    imminent.family_id = ActionFamilyId::Guard;
    imminent.guard_threat_ticks = 1;
    imminent.commitment_ticks = combat_policy::commitment_ticks(ActionFamilyId::Guard);
    let mut distant = base_candidate();
    distant.family_id = ActionFamilyId::Guard;
    distant.guard_threat_ticks = combat_policy::GUARD_URGENCY_TICKS as i64;
    distant.commitment_ticks = combat_policy::commitment_ticks(ActionFamilyId::Guard);
    assert!(combat_policy::score(&imminent) > combat_policy::score(&distant));
}

#[test]
fn scoring_loses_to_declining_when_the_purpose_is_weak_and_far_from_the_ball() {
    let mut weak = base_candidate();
    weak.purpose = Purpose::CarrierProtection;
    weak.target_ball_distance = 500.0;
    weak.target_is_carrier = false;
    weak.teammate_coverage = 2;
    weak.formation_risk = true;
    let options = combat_policy::options(&[weak]);
    let (selected, _) = brain::select_scored_option(&options, 0.0, 1);
    assert_eq!(selected.kind, "decline");
}

#[test]
fn scoring_always_offers_a_decline_option_and_unique_kind_id_pairs() {
    let mut a = base_candidate();
    a.target_player = 7;
    let mut b = base_candidate();
    b.target_player = 8;
    b.purpose = Purpose::LooseBallContest;
    let options = combat_policy::options(&[a, b]);
    assert_eq!(options.len(), 3);
    let (selected, _) = brain::select_scored_option(&options, 0.0, 1);
    assert_eq!(selected.kind, "commit");
    assert_eq!(selected.reference, Some(OptionReference::Index(7)));
}

#[test]
fn combat_decision_reasons_keeps_decline_out_of_the_commit_vocabulary_and_unattributed_inside_it() {
    // A string-vocabulary membership check would be structurally redundant
    // here: `CombatDecisionReason` is a closed Rust enum, so every value
    // already IS a valid reason, and the `formation_risk_tradeoff` cost-flag
    // string cannot even be constructed as one. What is not structurally
    // guaranteed — which reasons are COMMIT reasons — is asserted directly.
    assert!(!CombatDecisionReason::Decline.is_commit_reason());
    assert!(CombatDecisionReason::UnattributedOffBall.is_commit_reason());
    for purpose in [
        CombatDecisionReason::CarrierContest,
        CombatDecisionReason::CarrierProtection,
        CombatDecisionReason::LooseBallContest,
        CombatDecisionReason::PassingLaneOrShotDenial,
        CombatDecisionReason::RecoveryPunish,
    ] {
        assert!(purpose.is_commit_reason());
    }
}

#[test]
#[should_panic(expected = "a commit requires a commit reason")]
fn combat_decision_reasons_refuses_to_label_a_commit_with_decline() {
    let _ = combat_intent::commit(
        &combat_intent::new_state(),
        CombatDecisionReason::Decline,
        7,
        0,
    );
}

#[test]
fn combat_decision_reasons_materializes_only_legal_equipment_transitions() {
    // Kept in the `!(a) && !(b)` shape (rather than the De Morgan-folded
    // form clippy prefers) so it reads next to `input_frame::validate_sample`'s
    // prose rule it mirrors.
    #[allow(clippy::nonminimal_bool)]
    fn legal(signals: combat_intent::CombatIntentSignals) -> bool {
        let pressed = signals.equipment_pressed;
        let released = signals.equipment_released;
        let held = signals.equipment_held;
        !(pressed && !released && !held) && !(released && held)
    }

    assert!(legal(combat_intent::commit_signals()));
    let state = combat_intent::commit(
        &combat_intent::new_state(),
        CombatDecisionReason::CarrierContest,
        7,
        0,
    );
    let (signals, after) = combat_intent::materialize(&state);
    let signals = signals.expect("a release-owed intent materializes a signal");
    assert!(legal(signals));
    assert!(signals.equipment_released);
    assert_eq!(after.stage, combat_intent::CombatIntentStage::Idle);

    let guard = combat_intent::commit(
        &combat_intent::new_state(),
        CombatDecisionReason::CarrierContest,
        7,
        3,
    );
    assert_eq!(guard.stage, combat_intent::CombatIntentStage::Hold);
    let mut current = guard;
    for _ in 0..3 {
        let (held_signals, next) = combat_intent::materialize(&current);
        current = next;
        let held_signals = held_signals.expect("a held intent materializes a signal");
        assert!(legal(held_signals));
        assert!(held_signals.equipment_held);
    }
    let (release_signals, next) = combat_intent::materialize(&current);
    current = next;
    let release_signals = release_signals.expect("a released intent materializes a signal");
    assert!(release_signals.equipment_released);
    assert!(!release_signals.equipment_held);
    assert_eq!(current.stage, combat_intent::CombatIntentStage::Idle);
    assert_eq!(combat_intent::materialize(&current).0, None);
}

#[test]
fn combat_decision_reasons_derives_a_stat_scaled_cadence_and_a_per_tick_decision_seed() {
    let slow = combat_intent::decision_period(0.0);
    let fast = combat_intent::decision_period(1.0);
    assert!(slow > fast);
    assert_eq!(slow, 27);
    assert_eq!(fast, 9);
    assert!(combat_intent::should_decide(0, slow, 0.0));
    assert!(!combat_intent::should_decide(1, slow, 0.0));
    // Two players with the same scan rate do not decide on the same tick.
    assert!(combat_intent::should_decide(0, 9, 1.0));
    assert!(!combat_intent::should_decide(0, 10, 1.0));
    let seed = combat_intent::decision_seed(41, 3);
    assert_eq!(combat_intent::decision_seed(41, 3), seed);
    assert_ne!(combat_intent::decision_seed(41, 4), seed);
    assert!(seed >= 1);
}

// The remaining "gameplay_ai/combat/v1 decisions" tests build their fixture
// via `bare_match()`, which calls `gc_sim::r#match::new`. The first five
// cases below use the `bare_match()`/`observe()` helpers above; the sixth
// (`decisions_rejects_any_observation_that_is_not_this_schema`) needs no
// match fixture at all.

#[test]
fn decisions_reports_the_closed_unavailability_reason_instead_of_guessing() {
    let (mut state, mut combat_state) = bare_match();
    state.players[(SOURCE - 1) as usize].pos = Vec2::new(400.0, 270.0);
    state.players[(TARGET - 1) as usize].pos = Vec2::new(424.0, 270.0);
    state.owner = Some(TARGET);
    state.ball = state.players[(TARGET - 1) as usize].pos;

    let original_family = combat_state.players[(SOURCE - 1) as usize].family_id;
    combat_state.players[(SOURCE - 1) as usize].family_id = None;
    combat_state.players[(SOURCE - 1) as usize].loadout_id = None;
    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(
        decision.unavailable_reason,
        Some(CombatUnavailableReason::NoLoadout)
    );

    combat_state.players[(SOURCE - 1) as usize].family_id = original_family;
    combat_state.players[(SOURCE - 1) as usize].loadout_id =
        Some("loadout_spring_gloves".to_string());
    combat_state.players[(SOURCE - 1) as usize].cooldown_ticks = 12;
    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(
        decision.unavailable_reason,
        Some(CombatUnavailableReason::Cooldown)
    );

    combat_state.players[(SOURCE - 1) as usize].cooldown_ticks = 0;
    combat_state.players[(SOURCE - 1) as usize].forced_ticks = 5;
    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(
        decision.unavailable_reason,
        Some(CombatUnavailableReason::Forced)
    );

    combat_state.players[(SOURCE - 1) as usize].forced_ticks = 0;
    combat_state.players[(SOURCE - 1) as usize].phase = CombatActionPhase::Windup;
    combat_state.players[(SOURCE - 1) as usize].phase_ticks = 3;
    combat_state.players[(SOURCE - 1) as usize].source_sequence = Some(1);
    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(
        decision.unavailable_reason,
        Some(CombatUnavailableReason::AlreadyCommitted)
    );

    combat_state.players[(SOURCE - 1) as usize].phase = CombatActionPhase::Ready;
    combat_state.players[(SOURCE - 1) as usize].phase_ticks = 0;
    combat_state.players[(SOURCE - 1) as usize].source_sequence = None;
    state.players[(SOURCE - 1) as usize].slide_timer = 0.4;
    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(
        decision.unavailable_reason,
        Some(CombatUnavailableReason::SoccerCommitment)
    );
}

#[test]
fn decisions_reports_no_reachable_opportunity_as_a_feasibility_unavailability() {
    let (mut state, mut combat_state) = bare_match();
    combat_state.players[(SOURCE - 1) as usize].family_id = Some(ActionFamilyId::Unarmed);
    combat_state.players[(SOURCE - 1) as usize].loadout_id =
        Some("loadout_spring_gloves".to_string());
    state.players[(SOURCE - 1) as usize].pos = Vec2::new(60.0, 60.0);
    state.players[(TARGET - 1) as usize].pos = Vec2::new(900.0, 500.0);
    state.owner = Some(TARGET);
    state.ball = state.players[(TARGET - 1) as usize].pos;

    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(decision.action, PolicyAction::Unavailable);
    assert_eq!(
        decision.unavailable_reason,
        Some(CombatUnavailableReason::FamilyCommitFeasibility)
    );
    assert_eq!(decision.reason, CombatDecisionReason::None);
}

#[test]
fn decisions_commits_with_exactly_one_purpose_reason_and_no_context_violation() {
    let (mut state, mut combat_state) = bare_match();
    combat_state.players[(SOURCE - 1) as usize].family_id = Some(ActionFamilyId::Unarmed);
    combat_state.players[(SOURCE - 1) as usize].loadout_id =
        Some("loadout_spring_gloves".to_string());
    state.players[(SOURCE - 1) as usize].pos = Vec2::new(400.0, 270.0);
    state.players[(SOURCE - 1) as usize].facing = Vec2::new(1.0, 0.0);
    state.players[(TARGET - 1) as usize].pos = Vec2::new(424.0, 270.0);
    state.owner = Some(TARGET);
    state.ball = state.players[(TARGET - 1) as usize].pos;

    let decision = combat_policy::decide(&observe(&state, &combat_state), 1, Some(0.0));
    assert_eq!(decision.action, PolicyAction::Commit);
    assert_eq!(decision.reason, CombatDecisionReason::CarrierContest);
    assert_eq!(decision.target_player, Some(TARGET));
    assert_eq!(decision.family_id, Some(ActionFamilyId::Unarmed));
    assert!(!decision.context_violation);
    assert!(combat_policy::is_commit_reason(decision.reason));
    assert_eq!(decision.digest.len(), 16);
}

#[test]
fn decisions_is_byte_identical_for_the_same_observation_and_seed() {
    let (mut state, mut combat_state) = bare_match();
    combat_state.players[(SOURCE - 1) as usize].family_id = Some(ActionFamilyId::Unarmed);
    combat_state.players[(SOURCE - 1) as usize].loadout_id =
        Some("loadout_spring_gloves".to_string());
    state.players[(SOURCE - 1) as usize].pos = Vec2::new(400.0, 270.0);
    state.players[(SOURCE - 1) as usize].facing = Vec2::new(1.0, 0.0);
    state.players[(TARGET - 1) as usize].pos = Vec2::new(424.0, 270.0);
    state.owner = Some(TARGET);
    state.ball = state.players[(TARGET - 1) as usize].pos;

    let observation = observe(&state, &combat_state);
    let first = combat_policy::decide(&observation, 20003, Some(3.0));
    let second = combat_policy::decide(&observation, 20003, Some(3.0));
    assert_eq!(second.option_id, first.option_id);
    assert_eq!(second.reason, first.reason);
    assert_eq!(second.target_player, first.target_player);
    assert_eq!(second.rng_state, first.rng_state);
}

#[test]
fn decisions_spends_no_rng_at_zero_temperature() {
    let (mut state, mut combat_state) = bare_match();
    combat_state.players[(SOURCE - 1) as usize].family_id = Some(ActionFamilyId::Unarmed);
    combat_state.players[(SOURCE - 1) as usize].loadout_id =
        Some("loadout_spring_gloves".to_string());
    state.players[(SOURCE - 1) as usize].pos = Vec2::new(400.0, 270.0);
    state.players[(SOURCE - 1) as usize].facing = Vec2::new(1.0, 0.0);
    state.players[(TARGET - 1) as usize].pos = Vec2::new(424.0, 270.0);
    state.owner = Some(TARGET);
    state.ball = state.players[(TARGET - 1) as usize].pos;

    let decision = combat_policy::decide(&observe(&state, &combat_state), 20003, Some(0.0));
    assert_eq!(decision.rng_state, 20003);
}

#[test]
fn decisions_rejects_any_observation_that_is_not_this_schema() {
    // Unlike the other five "decisions" cases, this one needs no real match
    // fixture — a schema/version tag mismatch is rejected before any
    // observation field is read — so it ports directly against a minimal
    // (unvalidated, deliberately foreign-schema) observation value.
    let foreign = fake_foreign_observation();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        combat_policy::decide(&foreign, 1, Some(0.0))
    }));
    assert!(
        result.is_err(),
        "gameplay_ai/combat/v1 must reject a foreign observation schema"
    );
}

fn fake_foreign_observation() -> gc_sim::combat_observation::CombatObservation {
    use gc_sim::combat_feasibility::{CombatActionPhase, Team};
    use gc_sim::combat_observation::{BallRow, CombatObservation, MatchRow, SelfRow, SoccerAction};
    CombatObservation {
        schema: "human_proxy_observation/v1".to_string(),
        version: 1,
        policy_id: "gameplay_ai/combat/v1".to_string(),
        producer_tick: 0,
        observed_tick: 0,
        family_catalog_version: String::new(),
        collision_catalog_version: String::new(),
        player_order: vec![1],
        projectile_order: Vec::new(),
        own: SelfRow {
            player_index: 1,
            player_id: "p1".to_string(),
            slot: 0,
            team: Team::Home,
            is_keeper: false,
            family_id: None,
            source_sequence: 0,
            soccer_action: SoccerAction::None,
            soccer_committed: false,
            phase: CombatActionPhase::Ready,
            phase_ticks: 0,
            forced_state: None,
            forced_ticks: 0,
            immunity_ticks: 0,
            cooldown_ticks: 0,
            ready: false,
            control_held: false,
            release_latched: false,
            projected_spawn_tick: 0,
            x: 0.0,
            y: 0.0,
            vx: 0.0,
            vy: 0.0,
            facing_x: 1.0,
            facing_y: 0.0,
            radius: 12.0,
            move_speed: 180.0,
            last_equipment_held: false,
            last_equipment_pressed: false,
            last_equipment_released: false,
        },
        teammates: Vec::new(),
        opponents: Vec::new(),
        projectiles: Vec::new(),
        ball: BallRow {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            loose: true,
            owner_player_index: 0,
            owner_team: None,
            owner_is_keeper: false,
        },
        match_row: MatchRow {
            tick: 0,
            tick_rate: 60.0,
            field_w: 960.0,
            field_h: 540.0,
            goal_home_x: 0.0,
            goal_home_y: 0.0,
            goal_home_w: 0.0,
            goal_home_h: 0.0,
            goal_away_x: 0.0,
            goal_away_y: 0.0,
            goal_away_w: 0.0,
            goal_away_h: 0.0,
            score_home: 0,
            score_away: 0,
            remaining_ticks: 0,
            stoppage: false,
            stoppage_ticks: 0,
            finished: false,
        },
        anchors: Vec::new(),
        digest: String::new(),
    }
}
