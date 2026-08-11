//! Port of `spec/sim/combat_feasibility_spec.lua`.
//!
//! The Lua spec builds its fixtures through `sim.match`, `sim.combat`, and
//! `sim.combat_observation` (`bare_match()` + `combat_observation.build`).
//! None of those are ported yet — `combat_feasibility` itself never
//! `require`s them either (see `gc_sim::combat_feasibility`'s module doc
//! comment), so this file cannot build through them. Instead it constructs
//! [`gc_sim::combat_feasibility::CombatObservation`] fixtures directly, using
//! the same geometry, roster layout, and per-test mutations the Lua spec
//! describes (home is 1..5 with 1 the keeper, away is 6..10 with 6 the
//! keeper; `SOURCE = 2`, `TARGET = 7`, `OTHER_OPPONENT = 8`), plus two real
//! engine constants a direct fixture still needs: `sim/match.lua`'s
//! `PLAYER_RADIUS = 12` and its goal rects for a 960×540 field
//! (`GOAL_DEPTH = 30`, `GOAL_MOUTH = 110`). Every geometric outcome checked
//! below (in-reach/out-of-reach, in-arc/out-of-arc, blocked/clear lines,
//! sole-blocker lane denial) follows from those fixed constants and the
//! positions each test states, not from a specific roster's move speed or
//! pace — the one test that does exercise locomotion (a 20-tick movement
//! tape) only needs *some* plausible speed to close a 16px gap in 1/3
//! second, so a representative move speed (pace 5: `60 + 5*20 = 160px/s`,
//! `gc_sim::stats::move_speed`) stands in for whichever roster player the
//! real fixture would have used.

use gc_data::action_families::ActionFamilyId;
use gc_data::players::StatBlock;
use gc_sim::combat_feasibility::{
    self, CombatActionPhase, CombatEnvelopeOptions, CombatObservation, CombatObservationAnchor,
    CombatObservationBall, CombatObservationMatchView, CombatObservationPeer,
    CombatObservationProjectile, CombatObservationSelf, CombatPurposeId, CombatWitnessTape, Team,
};
use gc_sim::stats;

/// `sim/match.lua`'s `PLAYER_RADIUS`.
const RADIUS: f64 = 12.0;
/// A representative move speed standing in for a real roster player's pace
/// (see the module doc comment). Pace 5 is mid-roster, not special-cased by
/// any test.
const MOVE_SPEED_STAT_BLOCK: StatBlock = StatBlock {
    pace: 5,
    strength: 5,
    technique: 5,
    stamina: 5,
    mental: 5,
};

const SOURCE: i64 = 2;
const TARGET: i64 = 7;
const OTHER_OPPONENT: i64 = 8;

fn move_speed() -> f64 {
    stats::move_speed(MOVE_SPEED_STAT_BLOCK)
}

/// `sim/match.lua`'s goal rects for a 960x540 field (`GOAL_DEPTH = 30`,
/// `GOAL_MOUTH = 110`, `mouth_y = field.h / 2 - GOAL_MOUTH / 2`).
fn match_view() -> CombatObservationMatchView {
    CombatObservationMatchView {
        field_w: 960.0,
        field_h: 540.0,
        goal_home_x: -30.0,
        goal_home_y: 215.0,
        goal_home_h: 110.0,
        goal_away_x: 960.0,
        goal_away_y: 215.0,
        goal_away_w: 30.0,
        goal_away_h: 110.0,
    }
}

/// One roster player's mutable fixture state, mirroring the fields
/// `bare_match()` / `combat_observation.build` would have populated from a
/// real `sim.match` + `sim.combat` state.
#[derive(Clone, Copy, Debug)]
struct PlayerFixture {
    index: i64,
    team: Team,
    is_keeper: bool,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    facing_x: f64,
    facing_y: f64,
    phase: CombatActionPhase,
    phase_ticks: i64,
    family_id: Option<ActionFamilyId>,
    forced_ticks: i64,
    release_latched: bool,
    projected_spawn_tick: i64,
}

/// A hand-built `combat_sim_observation/v1`-shaped fixture: ten players (home
/// 1..5, away 6..10, each side's first player the keeper), parked far apart
/// by default (mirroring the Lua spec's `bare_match()`), a neutral ball, no
/// anchors, and no projectiles, until a test overrides them.
struct Scenario {
    players: [PlayerFixture; 10],
    ball: CombatObservationBall,
    anchors: Vec<CombatObservationAnchor>,
    projectiles: Vec<CombatObservationProjectile>,
    observed_tick: i64,
}

impl Scenario {
    fn new() -> Self {
        let players = std::array::from_fn(|i| {
            let index = i as i64 + 1;
            let team = if index <= 5 { Team::Home } else { Team::Away };
            PlayerFixture {
                index,
                team,
                is_keeper: index == 1 || index == 6,
                // "Park everyone far apart so each scenario states its own
                // geometry" (bare_match()'s own comment).
                x: 40.0 + index as f64 * 3.0,
                y: 40.0 + index as f64 * 3.0,
                vx: 0.0,
                vy: 0.0,
                facing_x: if team == Team::Home { 1.0 } else { -1.0 },
                facing_y: 0.0,
                phase: CombatActionPhase::Ready,
                phase_ticks: 0,
                family_id: None,
                forced_ticks: 0,
                release_latched: false,
                projected_spawn_tick: 0,
            }
        });
        Scenario {
            players,
            ball: CombatObservationBall {
                x: 900.0,
                y: 500.0,
                owner_player_index: 0,
                owner_team: None,
            },
            anchors: Vec::new(),
            projectiles: Vec::new(),
            observed_tick: 0,
        }
    }

    fn player_mut(&mut self, index: i64) -> &mut PlayerFixture {
        self.players
            .iter_mut()
            .find(|p| p.index == index)
            .expect("index 1..=10")
    }

    /// Build the `CombatObservation` `source_index` would see: canonical
    /// index order, `source_index`'s own row split out as `own`, its side as
    /// `teammates`, the other side as `opponents`.
    fn observe(&self, source_index: i64) -> CombatObservation {
        let source = self
            .players
            .iter()
            .find(|p| p.index == source_index)
            .expect("index 1..=10");
        let own = CombatObservationSelf {
            player_index: source.index,
            team: source.team,
            x: source.x,
            y: source.y,
            vx: source.vx,
            vy: source.vy,
            facing_x: source.facing_x,
            facing_y: source.facing_y,
            radius: RADIUS,
            move_speed: move_speed(),
            phase: source.phase,
            family_id: source.family_id,
            forced_ticks: source.forced_ticks,
        };
        let mut teammates = Vec::new();
        let mut opponents = Vec::new();
        for player in &self.players {
            if player.index == source_index {
                continue;
            }
            let reach_px = player
                .family_id
                .and_then(|id| gc_data::action_families::get(id).reach_px)
                .unwrap_or(0.0);
            let peer = CombatObservationPeer {
                player_index: player.index,
                team: player.team,
                is_keeper: player.is_keeper,
                x: player.x,
                y: player.y,
                vx: player.vx,
                vy: player.vy,
                facing_x: player.facing_x,
                facing_y: player.facing_y,
                radius: RADIUS,
                phase: player.phase,
                phase_ticks: player.phase_ticks,
                family_id: player.family_id,
                forced_ticks: player.forced_ticks,
                release_latched: player.release_latched,
                projected_spawn_tick: player.projected_spawn_tick,
                projected_reach_px: reach_px,
            };
            if player.team == source.team {
                teammates.push(peer);
            } else {
                opponents.push(peer);
            }
        }
        CombatObservation {
            own,
            teammates,
            opponents,
            projectiles: self.projectiles.clone(),
            ball: self.ball,
            match_view: match_view(),
            anchors: self.anchors.clone(),
            observed_tick: self.observed_tick,
        }
    }
}

// ---------------------------------------------------------------------
// family_commit_feasibility/v1
// ---------------------------------------------------------------------

#[test]
fn family_commit_feasibility_v1_proves_an_unarmed_swept_melee_contact_inside_reach_and_arc() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    let target = scenario.player_mut(TARGET);
    target.x = 424.0;
    target.y = 270.0;
    let observation = scenario.observe(SOURCE);

    let witness =
        combat_feasibility::family_commit(&observation, TARGET, ActionFamilyId::Unarmed, None);
    assert!(witness.feasible);
    assert_eq!(witness.family_id, ActionFamilyId::Unarmed);
    assert_eq!(witness.target_player, TARGET);
    // Six windup ticks, then the first of four active ticks.
    assert_eq!(witness.contact_tick, 7);
    assert_eq!(witness.horizon_ticks, 10);
}

#[test]
fn family_commit_feasibility_v1_refuses_an_unarmed_commit_behind_the_source_and_beyond_its_reach() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 376.0;
    scenario.player_mut(TARGET).y = 270.0;
    assert!(
        !combat_feasibility::family_commit(
            &scenario.observe(SOURCE),
            TARGET,
            ActionFamilyId::Unarmed,
            None
        )
        .feasible
    );

    scenario.player_mut(TARGET).x = 520.0;
    scenario.player_mut(TARGET).y = 270.0;
    assert!(
        !combat_feasibility::family_commit(
            &scenario.observe(SOURCE),
            TARGET,
            ActionFamilyId::Unarmed,
            None
        )
        .feasible
    );
}

#[test]
fn family_commit_feasibility_v1_reaches_further_with_light_melee_than_unarmed_from_the_same_pose() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 444.0;
    scenario.player_mut(TARGET).y = 270.0;
    let observation = scenario.observe(SOURCE);
    assert!(
        !combat_feasibility::family_commit(&observation, TARGET, ActionFamilyId::Unarmed, None)
            .feasible
    );
    let witness =
        combat_feasibility::family_commit(&observation, TARGET, ActionFamilyId::LightMelee, None);
    assert!(witness.feasible);
    // Twelve windup ticks, then the first of five active ticks.
    assert_eq!(witness.contact_tick, 13);
}

#[test]
fn family_commit_feasibility_v1_lets_a_searched_movement_tape_make_an_out_of_reach_target_reachable()
 {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 470.0;
    scenario.player_mut(TARGET).y = 270.0;
    let observation = scenario.observe(SOURCE);
    assert!(
        !combat_feasibility::family_commit(&observation, TARGET, ActionFamilyId::Unarmed, None)
            .feasible
    );
    let tape = CombatWitnessTape {
        move_x: 1.0,
        move_y: 0.0,
        ticks: 20,
    };
    let moved = combat_feasibility::family_commit(
        &observation,
        TARGET,
        ActionFamilyId::Unarmed,
        Some(&tape),
    );
    assert!(moved.feasible);
    assert_eq!(moved.commit_tick, 20);
}

#[test]
fn family_commit_feasibility_v1_proves_a_ranged_commit_only_along_a_clear_projected_line() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 200.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 400.0;
    scenario.player_mut(TARGET).y = 270.0;
    let clear = scenario.observe(SOURCE);
    assert!(
        combat_feasibility::family_commit(&clear, TARGET, ActionFamilyId::Ranged, None).feasible
    );

    // Another opponent standing in front of the target takes the shot.
    scenario.player_mut(OTHER_OPPONENT).x = 320.0;
    scenario.player_mut(OTHER_OPPONENT).y = 270.0;
    let blocked = scenario.observe(SOURCE);
    assert!(
        !combat_feasibility::family_commit(&blocked, TARGET, ActionFamilyId::Ranged, None).feasible
    );
    assert!(
        combat_feasibility::family_commit(&blocked, OTHER_OPPONENT, ActionFamilyId::Ranged, None)
            .feasible
    );
}

#[test]
fn family_commit_feasibility_v1_never_proves_a_commit_against_a_protected_keeper() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    let away_keeper = scenario.player_mut(6);
    away_keeper.x = 424.0;
    away_keeper.y = 270.0;
    let observation = scenario.observe(SOURCE);
    for family in [
        ActionFamilyId::Unarmed,
        ActionFamilyId::LightMelee,
        ActionFamilyId::Ranged,
    ] {
        assert!(
            !combat_feasibility::family_commit(&observation, 6, family, None).feasible,
            "{family:?} proved a commit against a keeper"
        );
    }
}

#[test]
fn family_commit_feasibility_v1_guards_a_hostile_melee_windup_and_nothing_without_a_public_threat()
{
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    {
        let target = scenario.player_mut(TARGET);
        target.x = 424.0;
        target.y = 270.0;
        target.facing_x = -1.0;
        target.facing_y = 0.0;
    }

    let idle = scenario.observe(SOURCE);
    assert!(
        !combat_feasibility::family_commit(&idle, TARGET, ActionFamilyId::Guard, None).feasible
    );

    {
        let hostile = scenario.player_mut(TARGET);
        hostile.family_id = Some(ActionFamilyId::LightMelee);
        hostile.phase = CombatActionPhase::Windup;
        hostile.phase_ticks = 10;
    }
    let telegraphed = scenario.observe(SOURCE);
    let witness =
        combat_feasibility::family_commit(&telegraphed, TARGET, ActionFamilyId::Guard, None);
    assert!(witness.feasible);
    assert_eq!(witness.family_id, ActionFamilyId::Guard);
    assert_eq!(witness.target_player, TARGET);
    assert!(
        witness.contact_tick >= 6,
        "guard cannot intersect before it is raised"
    );
}

#[test]
fn family_commit_feasibility_v1_ignores_an_aimed_ranged_row_until_its_release_latch_is_public() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    {
        let target = scenario.player_mut(TARGET);
        target.x = 470.0;
        target.y = 270.0;
        target.facing_x = -1.0;
        target.facing_y = 0.0;
        target.family_id = Some(ActionFamilyId::Ranged);
        target.phase = CombatActionPhase::Windup;
        target.phase_ticks = 8;
    }

    let unlatched = scenario.observe(SOURCE);
    assert_eq!(
        combat_feasibility::hostile_paths(&unlatched, Some(TARGET)).len(),
        0
    );
    assert!(
        !combat_feasibility::family_commit(&unlatched, TARGET, ActionFamilyId::Guard, None)
            .feasible
    );

    scenario.player_mut(TARGET).release_latched = true;
    let latched = scenario.observe(SOURCE);
    assert_eq!(
        combat_feasibility::hostile_paths(&latched, Some(TARGET)).len(),
        1
    );
    assert!(
        combat_feasibility::family_commit(&latched, TARGET, ActionFamilyId::Guard, None).feasible
    );
}

#[test]
fn family_commit_feasibility_v1_guards_an_already_in_flight_hostile_projectile_inside_its_horizon()
{
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 600.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.projectiles.push(CombatObservationProjectile {
        x: 480.0,
        y: 270.0,
        dir_x: -1.0,
        dir_y: 0.0,
        source_team: Team::Away,
        source_player_index: TARGET,
        horizon_ticks: 40,
    });
    let observation = scenario.observe(SOURCE);
    assert_eq!(observation.projectiles.len(), 1);
    assert!(observation.projectiles[0].horizon_ticks > 0);
    let witness =
        combat_feasibility::family_commit(&observation, TARGET, ActionFamilyId::Guard, None);
    assert!(witness.feasible);

    let threat = combat_feasibility::incoming_threat(&observation, 30)
        .expect("a public projectile threat exists");
    assert_eq!(threat.source_player, TARGET);
    // The threat position is the PROJECTILE's, not the shooter's. The
    // shooter stands at x=600, well behind the body it is about to hit, so
    // a caller that stepped away from the shooter would step into the shot.
    assert_eq!(threat.threat_x, 480.0);
    assert_eq!(threat.threat_y, 270.0);
    assert_ne!(threat.threat_x, scenario.players[TARGET as usize - 1].x);
}

// ---------------------------------------------------------------------
// combat purpose predicates
// ---------------------------------------------------------------------

#[test]
fn combat_purpose_predicates_names_the_opposing_carrier_a_carrier_contest() {
    let mut scenario = Scenario::new();
    scenario.player_mut(SOURCE).x = 400.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.player_mut(TARGET).x = 424.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.ball = CombatObservationBall {
        x: 424.0,
        y: 270.0,
        owner_player_index: TARGET,
        owner_team: Some(Team::Away),
    };
    let (bitset, count) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert!(bitset.carrier_contest);
    assert!(count >= 1);
    assert_eq!(
        combat_feasibility::dominant_purpose(bitset),
        Some(CombatPurposeId::CarrierContest)
    );
}

#[test]
fn combat_purpose_predicates_names_the_nearest_opponent_to_our_own_carrier_a_carrier_protection() {
    let mut scenario = Scenario::new();
    scenario.player_mut(SOURCE).x = 400.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.player_mut(3).x = 410.0;
    scenario.player_mut(3).y = 270.0;
    scenario.ball = CombatObservationBall {
        x: 410.0,
        y: 270.0,
        owner_player_index: 3,
        owner_team: Some(Team::Home),
    };
    scenario.player_mut(TARGET).x = 440.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.player_mut(OTHER_OPPONENT).x = 700.0;
    scenario.player_mut(OTHER_OPPONENT).y = 270.0;
    let observation = scenario.observe(SOURCE);
    let (bitset, _) = combat_feasibility::purpose_bitset(&observation, TARGET);
    assert!(bitset.carrier_protection);
    // The far opponent is neither near the owner nor the nearest.
    let (other, _) = combat_feasibility::purpose_bitset(&observation, OTHER_OPPONENT);
    assert!(!other.carrier_protection);
}

#[test]
fn combat_purpose_predicates_names_a_shared_loose_ball_a_loose_ball_contest() {
    let mut scenario = Scenario::new();
    scenario.ball = CombatObservationBall {
        x: 400.0,
        y: 270.0,
        owner_player_index: 0,
        owner_team: None,
    };
    scenario.player_mut(SOURCE).x = 360.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.player_mut(TARGET).x = 440.0;
    scenario.player_mut(TARGET).y = 270.0;
    let (bitset, _) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert!(bitset.loose_ball_contest);

    // Move the source out of the 96px window: the pair stops being a
    // contest even though the target has not moved.
    scenario.player_mut(SOURCE).x = 100.0;
    let (far, _) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert!(!far.loose_ball_contest);
}

#[test]
fn combat_purpose_predicates_names_the_sole_blocker_of_one_of_our_passing_lanes_a_lane_denial() {
    let mut scenario = Scenario::new();
    scenario.player_mut(SOURCE).x = 300.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.ball = CombatObservationBall {
        x: 300.0,
        y: 270.0,
        owner_player_index: SOURCE,
        owner_team: Some(Team::Home),
    };
    scenario.player_mut(3).x = 600.0;
    scenario.player_mut(3).y = 270.0;
    scenario.player_mut(TARGET).x = 450.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.player_mut(OTHER_OPPONENT).x = 120.0;
    scenario.player_mut(OTHER_OPPONENT).y = 60.0;
    let (bitset, _) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert!(bitset.passing_lane_or_shot_denial);

    // A second body in the same lane means neither is the SOLE blocker.
    scenario.player_mut(OTHER_OPPONENT).x = 500.0;
    scenario.player_mut(OTHER_OPPONENT).y = 270.0;
    let (shared, _) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert!(!shared.passing_lane_or_shot_denial);
}

#[test]
fn combat_purpose_predicates_upgrades_a_ball_context_target_in_recovery_to_a_recovery_punish() {
    let mut scenario = Scenario::new();
    scenario.player_mut(SOURCE).x = 400.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.player_mut(TARGET).x = 424.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.ball = CombatObservationBall {
        x: 424.0,
        y: 270.0,
        owner_player_index: TARGET,
        owner_team: Some(Team::Away),
    };
    {
        let target = scenario.player_mut(TARGET);
        target.family_id = Some(ActionFamilyId::Unarmed);
        target.phase = CombatActionPhase::Recovery;
        target.phase_ticks = 8;
    }
    let (bitset, count) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert!(bitset.recovery_punish);
    assert!(bitset.carrier_contest);
    assert_eq!(count, 2);
    assert_eq!(
        combat_feasibility::dominant_purpose(bitset),
        Some(CombatPurposeId::RecoveryPunish)
    );
}

#[test]
fn combat_purpose_predicates_keeps_recovery_alone_diagnostic_rather_than_a_purpose() {
    let mut scenario = Scenario::new();
    scenario.player_mut(SOURCE).x = 400.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.player_mut(TARGET).x = 424.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.ball = CombatObservationBall {
        x: 900.0,
        y: 500.0,
        owner_player_index: 0,
        owner_team: None,
    };
    {
        let target = scenario.player_mut(TARGET);
        target.family_id = Some(ActionFamilyId::Unarmed);
        target.phase = CombatActionPhase::Recovery;
        target.phase_ticks = 8;
    }
    let (bitset, count) = combat_feasibility::purpose_bitset(&scenario.observe(SOURCE), TARGET);
    assert_eq!(count, 0);
    assert!(!bitset.recovery_punish);
    assert_eq!(combat_feasibility::dominant_purpose(bitset), None);
}

#[test]
fn combat_purpose_predicates_never_names_a_keeper_or_a_teammate() {
    let mut scenario = Scenario::new();
    scenario.ball = CombatObservationBall {
        x: 400.0,
        y: 270.0,
        owner_player_index: 0,
        owner_team: None,
    };
    scenario.player_mut(SOURCE).x = 390.0;
    scenario.player_mut(SOURCE).y = 270.0;
    scenario.player_mut(6).x = 410.0;
    scenario.player_mut(6).y = 270.0;
    scenario.player_mut(3).x = 412.0;
    scenario.player_mut(3).y = 270.0;
    let observation = scenario.observe(SOURCE);
    assert_eq!(combat_feasibility::purpose_bitset(&observation, 6).1, 0);
    assert_eq!(combat_feasibility::purpose_bitset(&observation, 3).1, 0);
}

// ---------------------------------------------------------------------
// intervention_candidate/v2
// ---------------------------------------------------------------------

#[test]
fn intervention_candidate_v2_admits_a_reachable_purpose_pair_and_records_its_family_bitset() {
    let mut scenario = Scenario::new();
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 400.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 424.0;
    scenario.player_mut(TARGET).y = 270.0;
    scenario.ball = CombatObservationBall {
        x: 424.0,
        y: 270.0,
        owner_player_index: TARGET,
        owner_team: Some(Team::Away),
    };
    let options = CombatEnvelopeOptions {
        search_ticks: Some(0),
        families: None,
    };
    let envelope =
        combat_feasibility::intervention_candidates(&scenario.observe(SOURCE), Some(&options));
    assert_eq!(envelope.len(), 1);
    assert_eq!(envelope[0].target_player, TARGET);
    assert_eq!(envelope[0].purpose, CombatPurposeId::CarrierContest);
    assert!(envelope[0].family_bitset.unarmed);
    assert!(envelope[0].family_bitset.light_melee);
}

#[test]
fn intervention_candidate_v2_keeps_a_true_purpose_that_no_searched_pose_can_reach_out_of_the_envelope()
 {
    let mut scenario = Scenario::new();
    // A carrier on the far touchline: the purpose predicate is true, but no
    // movement inside the search window brings any family into contact.
    // Section 4.6 calls this `context_only_remote`: a diagnostic, never an
    // opportunity.
    scenario.player_mut(SOURCE).x = 60.0;
    scenario.player_mut(SOURCE).y = 60.0;
    scenario.player_mut(TARGET).x = 900.0;
    scenario.player_mut(TARGET).y = 500.0;
    scenario.ball = CombatObservationBall {
        x: 900.0,
        y: 500.0,
        owner_player_index: TARGET,
        owner_team: Some(Team::Away),
    };
    let observation = scenario.observe(SOURCE);
    assert!(
        combat_feasibility::purpose_bitset(&observation, TARGET)
            .0
            .carrier_contest
    );
    let options = CombatEnvelopeOptions {
        search_ticks: Some(4),
        families: None,
    };
    assert_eq!(
        combat_feasibility::intervention_candidates(&observation, Some(&options)).len(),
        0
    );
}

#[test]
fn intervention_candidate_v2_returns_pairs_in_a_stable_target_then_purpose_order() {
    let mut scenario = Scenario::new();
    scenario.ball = CombatObservationBall {
        x: 400.0,
        y: 270.0,
        owner_player_index: 0,
        owner_team: None,
    };
    {
        let source = scenario.player_mut(SOURCE);
        source.x = 390.0;
        source.y = 270.0;
        source.facing_x = 1.0;
        source.facing_y = 0.0;
    }
    scenario.player_mut(TARGET).x = 414.0;
    scenario.player_mut(TARGET).y = 272.0;
    scenario.player_mut(OTHER_OPPONENT).x = 412.0;
    scenario.player_mut(OTHER_OPPONENT).y = 268.0;
    let options = CombatEnvelopeOptions {
        search_ticks: Some(0),
        families: None,
    };
    let envelope =
        combat_feasibility::intervention_candidates(&scenario.observe(SOURCE), Some(&options));
    assert!(envelope.len() >= 2);
    let mut previous = 0;
    for pair in &envelope {
        assert!(pair.target_player >= previous);
        previous = pair.target_player;
    }
}

// ---------------------------------------------------------------------
// formation_risk_tradeoff
// ---------------------------------------------------------------------

#[test]
fn formation_risk_tradeoff_flags_a_source_far_from_its_authored_anchor_and_clears_one_at_home() {
    let mut scenario = Scenario::new();
    scenario.anchors.push(CombatObservationAnchor {
        player_index: SOURCE,
        anchor_x: 300.0,
        anchor_y: 200.0,
    });
    let source = scenario.player_mut(SOURCE);
    source.x = 300.0;
    source.y = 200.0;
    assert!(!combat_feasibility::formation_risk(
        &scenario.observe(SOURCE),
        ActionFamilyId::Unarmed
    ));

    let source = scenario.player_mut(SOURCE);
    source.x = 940.0_f64.min(300.0 + 200.0);
    source.y = 520.0_f64.min(200.0 + 150.0);
    assert!(combat_feasibility::formation_risk(
        &scenario.observe(SOURCE),
        ActionFamilyId::Unarmed
    ));
}
