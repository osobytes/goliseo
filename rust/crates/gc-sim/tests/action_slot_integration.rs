//! Integration coverage for #489's committed-action slot
//! (`gc_sim::action_slot`) that needs a real `MatchState`/`sim_match::step`
//! rather than the pure unit tests already in `gc_sim::action_slot`'s own
//! `#[cfg(test)]` module.
//!
//! Each fixture below is a hand-built minimal `MatchState`, the same
//! pattern `tests/pass_intent_seam.rs` and `tests/rollback_events.rs` use:
//! precise control over every player's position and state is the point, and
//! an authored roster's stats/positions would fight that control. This file
//! owns no production code, so it carries its own copy of the fixture
//! helpers.

use gc_core::vec2::Vec2;
use gc_data::action_tuning::ActionVerb;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::action_slot::{self, ActionPhase};
use gc_sim::keeper::KeeperBehaviorState;
use gc_sim::r#match::{self as sim_match, StepInput};
use gc_sim::match_snapshot::{self, ByTeam, MatchPlayer, MatchState, PitchSize, Rect, Team};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::pass_intent;
use gc_sim::possession_transition::{self, TransitionWindows};
use gc_sim::tuning::Tuning;

const FIELD_W: f64 = 960.0;
const FIELD_H: f64 = 540.0;
const DT: f64 = 1.0 / 60.0;

fn make_player(id: &str, team: Team, is_keeper: bool, x: f64, y: f64) -> MatchPlayer {
    MatchPlayer {
        id: id.to_string(),
        name: format!("{id}_name"),
        team,
        pos: Vec2::new(x, y),
        vel: Vec2::new(0.0, 0.0),
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(if team == Team::Home { 1.0 } else { -1.0 }, 0.0),
        anchor: Vec2::new(x, y),
        species_id: "human_base".to_string(),
        owned_verb: gc_data::species::SimVerb::None,
        move_speed: 180.0,
        shot_speed: 500.0,
        dribble: 0.5,
        strength: 0.5,
        first_touch: 0.5,
        header_skill: 0.5,
        volley_skill: 0.5,
        bicycle_skill: 0.5,
        scan_rate: 1.0,
        composure: 1.0,
        outfield_decision: outfield_decision::new_state(None),
        is_keeper,
        radius: 12.0,
        dash_cd: 0.0,
        dodge_cd: 0.0,
        dodge_timer: 0.0,
        dodge_dir: Vec2::new(0.0, 0.0),
        reach: if is_keeper { 30.0 } else { 0.0 },
        handling: if is_keeper { 0.5 } else { 0.0 },
        keeper_aggression: if is_keeper { 40.0 } else { 0.0 },
        keeper_anticipation: if is_keeper { 0.5 } else { 0.0 },
        keeper_state: KeeperBehaviorState::Base,
        keeper_state_timer: 0.0,
        keeper_release_state: None,
        keeper_release_motion: 0.0,
        keeper_release_kind: None,
        keeper_release_depth: 0.0,
        keeper_set: 0.0,
        dive_timer: 0.0,
        dive_dir: Vec2::new(0.0, 0.0),
        dive_delay: 0.0,
        dive_target: None,
        keeper_get_up_timer: 0.0,
        hold_timer: 0.0,
        feet_ball: false,
        slide_timer: 0.0,
        slide_dir: Vec2::new(0.0, 0.0),
        slide_vel: 0.0,
        tackle_timer: 0.0,
        tackle_cd: 0.0,
        stun_timer: 0.0,
        grab_timer: 0.0,
        throw_timer: 0.0,
        receive_timer: 0.0,
        sprint_meter: 1.0,
        sprint_dur: 3.0,
        sprinting: false,
        save_pending: None,
        save_timer: 0.0,
        save_vx: 0.0,
        save_style: None,
        save_tip_emitted: false,
        settle_timer: 0.0,
        header_cd: 0.0,
        aerial_timer: 0.0,
        aerial_style: None,
        aerial_outcome: None,
        aerial_jump: 0.0,
        aerial_recovery: 0.0,
        charge: 0.0,
        pass_charge: 0.0,
        pass_target: None,
        pass_intent: pass_intent::new_state(),
        windup_timer: 0.0,
        windup_shot: None,
        jockey_timer: 0.0,
        action: action_slot::new_state(),
    }
}

/// A hand-built, already-valid `MatchState`: both teams AI-driven
/// (`human_controlled: false`), non-slot-mode, mid-play (no kickoff hold).
fn base_state(players: Vec<MatchPlayer>, owner: Option<i64>, ball: Vec2) -> MatchState {
    MatchState {
        field: PitchSize {
            w: FIELD_W,
            h: FIELD_H,
        },
        goal_home: Rect {
            x: 0.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        goal_away: Rect {
            x: FIELD_W - 10.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        players,
        ball,
        ball_vel: Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner,
        controlled: 2,
        human_controlled: false,
        score: ByTeam { home: 0, away: 0 },
        time_left: 300.0,
        max_goals: 99,
        finished: false,
        pickup_cd: 0.0,
        press: ByTeam { home: 1, away: 1 },
        marking: ByTeam {
            home: MarkingConfig {
                scheme: MarkingScheme::Hybrid,
                man_marks: 1,
                standoff: 32.0,
                compactness: 0.5,
                support: 0.5,
            },
            away: MarkingConfig {
                scheme: MarkingScheme::Zonal,
                man_marks: 0,
                standoff: 40.0,
                compactness: 0.6,
                support: 0.4,
            },
        },
        marks: ByTeam {
            home: vec![None; 10],
            away: vec![None; 10],
        },
        outfield_press: ByTeam {
            home: outfield_press::new_state(),
            away: outfield_press::new_state(),
        },
        transition_windows: TransitionWindows {
            home: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
            away: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
        },
        transition: possession_transition::new_state(),
        formation: ByTeam {
            home: "2-1-1".to_string(),
            away: "2-1-1".to_string(),
        },
        ball_spin: 0.0,
        rng: gc_core::rng::seed(489.0),
        block_grace: 0.0,
        aerial_lock: 0.0,
        kickoff_hold: 0.0,
        events: Vec::new(),
        slot_mode: false,
        input_ownership: None,
        slot_players: vec![None; 8],
        slot_for_player: vec![None; 10],
        input_tick: 0,
        unsupported_reason: None,
    }
}

fn step_once(s: &mut MatchState, tune: &Tuning) {
    sim_match::step(
        s,
        DT,
        StepInput::Legacy(match_snapshot::MatchInput::default()),
        None,
        tune,
    );
}

fn ten_players(owner_pos: Vec2, others_far_away: bool) -> Vec<MatchPlayer> {
    let far = if others_far_away { 5000.0 } else { 50.0 };
    vec![
        make_player("h_keeper", Team::Home, true, 20.0, 270.0),
        make_player("h1", Team::Home, false, far, far),
        make_player("h2", Team::Home, false, far, far + 10.0),
        make_player("h3", Team::Home, false, far, far + 20.0),
        make_player("h4", Team::Home, false, far, far + 30.0),
        make_player("a_keeper", Team::Away, true, 940.0, 270.0),
        make_player("a_carrier", Team::Away, false, owner_pos.x, owner_pos.y),
        make_player("a2", Team::Away, false, far, far + 40.0),
        make_player("a3", Team::Away, false, far, far + 50.0),
        make_player("a4", Team::Away, false, far, far + 60.0),
    ]
}

// ---------------------------------------------------------------------
// The possession invariant: one rule, one place, no verb-level bypass.
// ---------------------------------------------------------------------

/// #489's own acceptance criterion: "Possession-change clearing implemented
/// once at the possession-change site; a test proves no verb-level bypass
/// exists."
///
/// This constructs a player mid-`Charging`/`Executing`/`Recovering` for
/// EVERY phase the action slot has, drives one real possession change
/// through `sim_match::step` (a keeper smother -- `attempt_steals`'s own
/// unconditional-clear path, unrelated to the tackle verb under test), and
/// asserts the committed action clears every time. Proving this against the
/// GENERIC mechanism (`action_slot::clear`, exercised through
/// `r#match::set_owner`, the single ownership choke point every
/// `s.owner` assignment in this crate goes through) is what makes it a
/// structural guarantee rather than a per-call-site spot check: a future
/// verb adopting this module inherits the same guarantee for free, at the
/// one call site, without writing its own clearing code.
#[test]
fn possession_change_clears_a_committed_action_from_every_phase_no_matter_the_verb() {
    for phase in [
        ActionPhase::Charging,
        ActionPhase::Executing,
        ActionPhase::Recovering,
    ] {
        let owner_pos = Vec2::new(500.0, 270.0);
        let mut players = ten_players(owner_pos, true);
        // a_carrier (index 6, one-based 7) is the one whose action we plant
        // and whose possession we are about to take away with a keeper
        // smother: move it into the OPPOSING (home) keeper's claim
        // zone/smother range -- the smother excludes the carrier's own team
        // (`p.team != owner_team`), so it must be the home keeper, not the
        // away one that is a_carrier's own teammate.
        let carrier_idx = 7usize;
        players[carrier_idx - 1].pos = Vec2::new(30.0, 270.0);
        players[0].pos = Vec2::new(20.0, 270.0); // h_keeper, index 1
        let action = match phase {
            ActionPhase::Charging => action_slot::commit_charge(
                &action_slot::new_state(),
                ActionVerb::Tackle,
                Some(2),
                0.0,
            ),
            ActionPhase::Executing => {
                let charging = action_slot::commit_charge(
                    &action_slot::new_state(),
                    ActionVerb::Tackle,
                    Some(2),
                    0.0,
                );
                action_slot::release(&charging, 0.1, 1.0, 0.3)
            }
            ActionPhase::Recovering => {
                let charging = action_slot::commit_charge(
                    &action_slot::new_state(),
                    ActionVerb::Tackle,
                    Some(2),
                    0.0,
                );
                let executing = action_slot::release(&charging, 0.1, 1.0, 0.3);
                action_slot::resolve_miss(&executing, 0.4)
            }
            ActionPhase::None => unreachable!("None is not exercised by this loop"),
        };
        assert_ne!(
            action.phase,
            ActionPhase::None,
            "test setup must commit a real action"
        );
        players[carrier_idx - 1].action = action;

        let mut state = base_state(players, Some(carrier_idx as i64), Vec2::new(30.0, 270.0));
        let tune = Tuning::new();
        step_once(&mut state, &tune);

        assert_eq!(
            state.owner,
            Some(1),
            "the home keeper (index 1) must have smothered the ball this tick -- if this \
             fails, the fixture stopped proving a real possession change happened"
        );
        assert_eq!(
            state.players[carrier_idx - 1].action.phase,
            ActionPhase::None,
            "phase {phase:?}: the dispossessed player's committed action must clear \
             unconditionally on the possession change, through the single set_owner \
             choke point -- a verb-level bypass would leave this non-idle"
        );
    }
}

// ---------------------------------------------------------------------
// ACTION_RECOVERY_CONTROL: movement is measurably gated during recovery.
// ---------------------------------------------------------------------

/// `ACTION_RECOVERY_CONTROL`'s statistical knob-moves-metric contract
/// against `turnovers_per_min` does NOT clear its measured noise floor at
/// affordable seed counts -- see `tests/knob_contract.rs`'s module doc for
/// the numbers, the same "aggregate outcome is too indirect" finding #491
/// already made for its own passing knobs. This is the direct proof the
/// aggregate statistic could not give: `ACTION_RECOVERY_CONTROL` is read,
/// and it measurably scales a recovering player's displacement, which is
/// what "no new action may start... movement input scales down" (#489)
/// actually claims. Deterministic, not statistical -- no seed count to
/// pick, no noise floor to clear.
#[test]
fn action_recovery_control_measurably_scales_a_recovering_players_displacement() {
    let far = Vec2::new(500.0, 270.0);
    let mut players = ten_players(far, true);
    // a2 (index 7, one-based 8) is the player under test: give it a clean
    // run toward a distant free target with nothing else nearby to disturb
    // it, then place it into Recovering.
    let subject_idx = 8usize;
    players[subject_idx - 1].pos = Vec2::new(400.0, 270.0);
    players[subject_idx - 1].anchor = Vec2::new(700.0, 270.0);
    let mut recovering_players = players.clone();
    let charging =
        action_slot::commit_charge(&action_slot::new_state(), ActionVerb::Tackle, None, 0.0);
    let executing = action_slot::release(&charging, 0.1, 1.0, 0.3);
    recovering_players[subject_idx - 1].action = action_slot::resolve_miss(&executing, 5.0);

    let tune = Tuning::new();
    let mut control_state = base_state(players, None, Vec2::new(-1000.0, -1000.0));
    let mut recovering_state = base_state(recovering_players, None, Vec2::new(-1000.0, -1000.0));

    let control_start = control_state.players[subject_idx - 1].pos;
    let recovering_start = recovering_state.players[subject_idx - 1].pos;
    assert_eq!(control_start, recovering_start);

    for _ in 0..30 {
        step_once(&mut control_state, &tune);
        step_once(&mut recovering_state, &tune);
    }

    let control_distance = control_state.players[subject_idx - 1]
        .pos
        .dist(control_start);
    let recovering_distance = recovering_state.players[subject_idx - 1]
        .pos
        .dist(recovering_start);

    assert!(
        control_distance > 1.0,
        "the control player must actually have moved for this comparison to mean anything: \
         moved {control_distance}"
    );
    let scale = tune.value("ACTION_RECOVERY_CONTROL");
    assert!(
        (0.0..1.0).contains(&scale),
        "this test's own premise (recovery slows movement) requires the default to be a \
         real fraction below 1.0: got {scale}"
    );
    // Generous slack (2x the declared scale) around the expected ratio: the
    // point is "measurably less", not an exact kinematic replay -- both
    // players still accelerate from rest and neither desired-speed
    // trajectory is a straight line the whole way.
    assert!(
        recovering_distance < control_distance * (scale * 2.0).min(0.9),
        "a recovering player (scale {scale:.2}) moved {recovering_distance:.1}px, not \
         meaningfully less than the unthrottled control's {control_distance:.1}px -- \
         ACTION_RECOVERY_CONTROL is not reducing movement",
    );
    assert_eq!(
        recovering_state.players[subject_idx - 1].action.phase,
        ActionPhase::Recovering,
        "the subject must still be recovering at the end of the window for this comparison \
         to isolate the scale rather than the transition out of it"
    );
}
