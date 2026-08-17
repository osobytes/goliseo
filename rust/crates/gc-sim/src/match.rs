//! Pure 5v5 match simulation. No rendering, no input gathering.
//!
//! Home attacks right (scores in the right goal); away attacks left. By
//! default, one home player is `controlled` by the human and everyone else
//! is AI. Fully simulated fixtures can set `human_controlled = false` so
//! every player uses the match AI. Possession is a single `owner` index
//! (`None` = loose ball). All state lives in [`MatchState`] and [`step`]
//! advances it deterministically.
//!
//! ## Adopting `match_snapshot`'s canonical types
//!
//! This module adopts [`crate::match_snapshot`]'s `MatchState` / `MatchPlayer`
//! / `MatchInput` / `MatchEvent` directly rather than declaring its own — that
//! module's entire job is to describe this shape.
//!
//! ## Indexing convention
//!
//! Every `player_index` in this module (and threaded through
//! `match_snapshot`, `combat`, `slot_input`, `outfield_press`,
//! `possession_transition`, `outfield_decision`, `offball_runs`) is
//! **1-based** (home indices 1..5, away 6..10) — this crate's own
//! player-identity convention (every downstream consumer of `MatchState`
//! `assert(index >= 1)` and bakes it in). Indexing `s.players` therefore
//! always reads `s.players[(index - 1) as usize]`. This is a deliberate
//! departure from ARCHITECTURE.md §3 rule 3's default (1-based -> 0-based) because these
//! indices are the same "wire identity" the rule's exception already covers.
//!
//! `sim::ai`'s helpers (`assign_marks`, `pass_intercept`, `support_spot`,
//! `separation`, ...) are the one exception: they index the caller-built
//! slices passed to them, which are ordinary 0-based Rust collections
//! (ARCHITECTURE.md §3 rule 3's default), not `s.players`. Call sites translate between
//! the two conventions explicitly.

use crate::action_slot::{self, ActionPhase};
use crate::aerial;
use crate::ai;
use crate::ball_flight::{
    self, AIR_FRICTION, BALL_RADIUS, FRICTION, GRAVITY, GROUND_GRAB_HEIGHT, in_mouth,
};
use crate::ball_prediction::BallPredictor;
use crate::brain;
use crate::combat;
use crate::combat_feasibility;
use crate::combat_intent;
use crate::combat_observation;
use crate::combat_policy;
use crate::combat_snapshot::CombatMatchState;
use crate::fixed_clock;
use crate::input_frame::{
    self, InputFixtureRosters, InputFrame, InputOwnership, InputSlotAssignment,
};
use crate::keeper;
use crate::locomotion;
use crate::match_snapshot::{
    MatchEvent, MatchEventKind, MatchInput, MatchPlayer, MatchState, PitchSize, Rect, Team,
    WindupShot,
};
use crate::offball_runs;
use crate::outfield_decision::{self, OutfieldDecisionContext};
use crate::outfield_press::{self, OutfieldPressState};
use crate::pass_intent;
use crate::pass_lead;
use crate::passing;
use crate::placement;
use crate::possession_transition::{self, TransitionTeam, TransitionWindows};
use crate::slot_input;
use crate::species;
use crate::stats;
use crate::tuning::Tuning;
use gc_core::deterministic_math;
use gc_core::rng;
use gc_core::vec2::Vec2;
use gc_data::action_tuning::ActionVerb;
use gc_data::formations::{self, FormationRole};
use gc_data::players::{PlayerData, Position};
use gc_data::showcase_player_compatibility::{self, ShowcasePlayerCompatibilityData};
use gc_data::species::SpeciesData;
use gc_data::tactics::{MarkingConfig, TacticData, TransitionConfig};
use gc_data::teams::TeamData;
use indexmap::IndexMap;

const PLAYER_RADIUS: f64 = 12.0;
const STICK_AHEAD: f64 = PLAYER_RADIUS + BALL_RADIUS;
const DRIBBLE_LEAD_MIN: f64 = STICK_AHEAD;
const DRIBBLE_TOUCH_REACH: f64 = STICK_AHEAD + 6.0;
const DRIBBLE_CATCH_PACE: f64 = 10.0;
const DRIBBLE_ERR_SKILL: f64 = 0.85;
const DRIBBLE_CONTROL_SKILL: f64 = 26.0;
const POSSESS_DIST: f64 = 22.0;
const KEEPER_DIST: f64 = 18.0;
const KEEPER_BOX_DEPTH: f64 = 160.0;
const PENALTY_DEPTH: f64 = 95.0;
const PENALTY_H: f64 = 200.0;
const KEEPER_BOX_PAD: f64 = 30.0;
const KEEPER_CLAIM_DIST: f64 = 40.0;
const KEEPER_LEAD: f64 = 0.01;
const KEEPER_1V1_SUPPORT: f64 = 120.0;
const POSSESS_MAX_SPEED: f64 = 350.0;
const CROSS_CLEAR_H: f64 = 50.0;
/// Range of an uncharged pass.
///
/// The registry spot-check (#487): this was a raw `const PASS_RANGE_MIN: f64 =
/// 110.0` in this file, invisible to the sweep and to the config hash even
/// though it sits inside the same expression as `PASS_RANGE_MAX`, which was
/// already a knob. The raw definition is deleted; the value is authored in
/// `gc_data::tunables::SIM_TUNABLES` and read through the registry handle the
/// caller already holds, so a sweep can move it and two peers hash it.
fn pass_range_min(tune: &Tuning) -> f64 {
    tune.value("PASS_RANGE_MIN")
}

/// The `pass_charge` fraction (`[0, 1]`) that would make `try_pass`'s own
/// range formula (`pass_range_min(tune) + charge * (PASS_RANGE_MAX -
/// pass_range_min(tune))`) reach `distance` — the inverse of that formula.
///
/// This is a mechanical translation of an already-scored distance into a
/// hold duration, not a new decision (#531): the gameplay AI's own scorer
/// picked `distance`; this only recovers the charge time a human would have
/// needed to reach the same range tier, so the AI pays the identical charge
/// cost a human pays for the same release. A distance at or below
/// `pass_range_min(tune)` needs no hold at all — a tap.
fn desired_pass_charge(distance: f64, tune: &Tuning) -> f64 {
    let min = pass_range_min(tune);
    let max = tune.value("PASS_RANGE_MAX");
    if max <= min {
        return 0.0;
    }
    ((distance - min) / (max - min)).clamp(0.0, 1.0)
}

const GOAL_MOUTH: f64 = 110.0;
const GOAL_DEPTH: f64 = 30.0;
const RELEASE_CD: f64 = 0.3;

const STEAL_DIST: f64 = 26.0;
const KICKOFF_CLEAR: f64 = 120.0;
const KICKOFF_HOLD: f64 = 2.5;
const TACKLE_POP_SPEED: f64 = 150.0;

const BLOCK_HEIGHT: f64 = 20.0;
const BLOCK_HEIGHT_DESC: f64 = 12.0;
const BLOCK_GRACE: f64 = 0.08;
const BLOCK_DAMP: f64 = 0.5;

const AI_PASS_MIN_OPEN: f64 = 40.0;
const AI_PASS_MIN_DIST: f64 = 40.0;
const AI_PASS_MAX_DIST: f64 = 420.0;

const AI_CHARGE_MIN_SPACE: f64 = 25.0;
const AI_CHARGE_SPACE_RANGE: f64 = 120.0;

const SLIDE_DURATION: f64 = 0.4;
const SLIDE_MULT: f64 = 1.5;
const SLIDE_BASE_MIN: f64 = 200.0;
const SLIDE_FRICTION: f64 = 2.5;
const SLIDE_REACH: f64 = 38.0;
const SLIDE_CD: f64 = 0.9;
const STAND_REACH: f64 = 34.0;
const STAND_CD: f64 = 0.4;
const STUN_SLOW: f64 = 0.4;
const STUN_TIME: f64 = 0.5;

const JOCKEY_REACH_BONUS: f64 = 6.0;
const JOCKEY_HOLD: f64 = 0.2;

const KEEPER_AIR_GRAB: f64 = 60.0;
const CROSSBAR: f64 = 70.0;
const LOB_CLEAR_H: f64 = 24.0;
const MAX_LOB_VH: f64 = 400.0;
const CHIP_LINE_Z: f64 = 65.0;

const AERIAL_ANTICIPATE: f64 = 84.0;
const CROSS_AID_Z: f64 = 30.0;
const CROSS_AID_THIRD: f64 = 0.6;
const CROSS_AID_RANGE: f64 = 150.0;
const CLEAR_HEADER_SPEED: f64 = 320.0;
const VOLLEY_SPEED: f64 = 1.3;

const CATCH_SOFTNESS: f64 = 0.12;
const PARRY_QUALITY: f64 = 0.1;
const HANDLING_WEIGHT: f64 = 0.5;
const PARRY_CD: f64 = 0.18;
const PARRY_SPEED_MULT: f64 = 0.6;
const MIN_PARRY_CLEAR: f64 = 260.0;
const PARRY_POP_VZ: f64 = 240.0;
const KEEPER_HOLD: f64 = 0.9;
const PUNT_MIN: f64 = 240.0;
const PUNT_CLEAR_H: f64 = 60.0;
const KEEPER_DIVE_DURATION: f64 = 0.32;
const KEEPER_HANDS: f64 = 30.0;
const SAVE_TIMEOUT_PAD: f64 = 0.25;
const DEAD_SHOT_SPEED: f64 = 30.0;
const KEEPER_SAFE_DIST: f64 = 60.0;
const THROW_MIN_OPEN: f64 = 30.0;
const DROPKICK_DIST: f64 = 420.0;
const DROPKICK_CLEAR_H: f64 = 46.0;
const THROW_CLEAR_H: f64 = 34.0;
const THROW_LANE_W: f64 = 60.0;
const THROW_LEAD_MAX: f64 = 55.0;
const THROW_COVER_DIST: f64 = 140.0;
const RELEASE_DINK_DIST: f64 = 44.0;
const SAVE_PAD: f64 = 18.0;
const SAVE_ZONE: f64 = 130.0;
const KEEPER_GRAB_POSE: f64 = 0.25;
const KEEPER_THROW_POSE: f64 = 0.25;
const KEEPER_GET_UP_POSE: f64 = 0.18;
const RECEIVE_TIME: f64 = 1.3;
const KEEPER_RECEIVE_TIME: f64 = 4.0;
const BACKPASS_AIM_COS: f64 = 0.92;

/// Speed below which a moving player's facing stops tracking their run
/// velocity. Lives in `locomotion` now that facing is an independent target;
/// re-exported here because the human branch still needs it to decide whether
/// the stick or the heading owns facing this tick.
use crate::locomotion::MOVEMENT_FACE_MIN_SPEED as RUN_VEL_FACE_MIN;

const WINDUP_MOVE: f64 = 0.3;

const CHARGE_POWER: f64 = 0.9;
const CURVE_MAX: f64 = 520.0;
const DODGE_DURATION: f64 = 0.16;
const DODGE_CD: f64 = 0.6;
const DODGE_SPEED_MULT: f64 = 2.4;

const SPRINT_ENGAGE: f64 = 0.25;

// Off-ball movement tuning.
const TRIANGLE_JOIN: f64 = 320.0;
const ARRIVE_RADIUS: f64 = 60.0;
const KEEPER_BASE_ARRIVE_RADIUS: f64 = 18.0;
const STAND_DEADBAND: f64 = 14.0;
const STAND_STILL_SPEED: f64 = 25.0;
const PURSUE_LEAD: f64 = 0.004;
const MARK_GOALSIDE: f64 = 16.0;
const MARK_LANE_OFF: f64 = 44.0;
const COVER_FRAC: f64 = 0.3;
const BLOCK_SHIFT: f64 = 0.45;
const ATTACK_PUSH: f64 = 90.0;
const SUPPORT_FAN: f64 = 70.0;
const SEP_RADIUS: f64 = 64.0;
const SEP_PUSH: f64 = 16.0;
const MARK_STICK: f64 = 20.0;

/// `(cos, sin)` of `support_target`'s four fixed triangle angles (radians:
/// -1.4, -0.7, 0.7, 1.4), precomputed rather than taken with `.cos()`/`.sin()`
/// on the simulation path. Same reasoning as `gc_data::locomotion`'s arc
/// cosines: neither function is correctly rounded, Rust links a different
/// libm for `wasm32-unknown-unknown` than a native build uses, and this is a
/// per-tick call once a carrier's teammates close into `TRIANGLE_JOIN`. The
/// angles are fixed authored geometry, not derived, so there is nothing to
/// recompute at runtime.
const SUPPORT_TRIANGLE_DIRECTIONS: [(f64, f64); 4] = [
    (0.16996714290024104, -0.9854497299884601),
    (0.7648421872844885, -0.644217687237691),
    (0.7648421872844885, 0.644217687237691),
    (0.16996714290024104, 0.9854497299884601),
];

/// There is no goal limit: a match is decided on score at full time, in
/// every mode. `max_goals` stays in the state, the snapshot, and the session
/// manifest because removing it would be a wire-vocabulary change, so "no
/// limit" is spelled as the largest value the protocol accepts
/// (`protocol.MAX_GOALS`, 99) — a total 120 seconds cannot reach. Callers
/// that deliberately want a match to end on goals (evidence fixtures,
/// rollback laboratories, specs that need a short match) pass their own
/// `max_goals`.
pub const NO_GOAL_LIMIT: i64 = 99;

/// Renderer data: the goal frame height (posts/crossbar), world units.
pub const CROSSBAR_H: f64 = CROSSBAR;
/// Renderer data: the penalty area depth (world units).
pub const PENALTY_BOX_DEPTH: f64 = PENALTY_DEPTH;
/// Renderer data: the penalty area height (world units).
pub const PENALTY_BOX_H: f64 = PENALTY_H;
/// Renderer data: the outfield player collision radius, world units.
pub const PLAYER_RADIUS_PX: f64 = PLAYER_RADIUS;
/// Renderer data: the ball collision radius, world units.
pub const BALL_RADIUS_PX: f64 = BALL_RADIUS;
/// Renderer data: ball gravity, px/s^2.
pub const GRAVITY_PX: f64 = GRAVITY;
/// Renderer data: the sprint meter fraction that re-engages a sprint.
pub const SPRINT_ENGAGE_FRAC: f64 = SPRINT_ENGAGE;

/// Options for [`new`].
pub struct NewMatchOptions<'a> {
    /// The home team.
    pub home: &'a TeamData,
    /// The away team.
    pub away: &'a TeamData,
    /// Playable pitch dimensions.
    pub field: PitchSize,
    /// Override for `home.formation`.
    pub home_formation: Option<&'a str>,
    /// Home tactic; defaults to `tactics::get("balanced")`.
    pub tactic: Option<&'a TacticData>,
    /// Away tactic; defaults to `tactics::get("balanced")`.
    pub away_tactic: Option<&'a TacticData>,
    /// Match duration in seconds; defaults to 120.
    pub duration: Option<f64>,
    /// Goal limit; defaults to [`NO_GOAL_LIMIT`].
    pub max_goals: Option<i64>,
    /// RNG seed; defaults to 42.
    pub seed: Option<f64>,
    /// Player pool; defaults to every authored player.
    pub players_by_id: Option<&'a IndexMap<&'a str, PlayerData>>,
    /// Species pool; defaults to every authored species.
    pub species_by_id: Option<&'a IndexMap<&'a str, SpeciesData>>,
    /// Showcase compatibility pool; defaults to every authored row.
    pub showcase_players_by_id: Option<&'a IndexMap<&'a str, ShowcasePlayerCompatibilityData>>,
    /// Whether `controlled` takes human-input branches; defaults to true.
    pub human_controlled: Option<bool>,
    /// Stable fixture slot-to-player identity; enables slot mode when set.
    pub input_ownership: Option<InputOwnership>,
}

fn pool_by_id() -> IndexMap<&'static str, PlayerData> {
    let mut by_id = IndexMap::new();
    for player in gc_data::players::ALL {
        by_id.insert(player.id, *player);
    }
    by_id
}

fn species_pool() -> IndexMap<&'static str, SpeciesData> {
    let mut by_id = IndexMap::new();
    for s in gc_data::species::ALL {
        by_id.insert(s.id, *s);
    }
    by_id
}

fn showcase_pool() -> IndexMap<&'static str, ShowcasePlayerCompatibilityData> {
    let mut by_id = IndexMap::new();
    for row in showcase_player_compatibility::ALL {
        by_id.insert(row.player_id, *row);
    }
    by_id
}

/// Every outfield (non-keeper) player id on `team`, in roster order.
///
/// # Panics
///
/// Panics if a roster id is unknown, or the team does not have exactly
/// [`input_frame::HOME_SLOT_COUNT`] outfielders.
fn fixture_outfield_ids(
    team: &TeamData,
    players_by_id: &IndexMap<&str, PlayerData>,
) -> Vec<String> {
    let mut ids = Vec::new();
    for id in team.roster {
        let player = players_by_id
            .get(id)
            .unwrap_or_else(|| panic!("unknown player: {id}"));
        if player.position != Position::Keeper {
            ids.push((*id).to_string());
        }
    }
    assert!(
        ids.len() as i64 == input_frame::HOME_SLOT_COUNT,
        "{} must have four outfielders",
        team.id
    );
    ids
}

/// Construct the canonical input ownership for two authored fixture teams.
/// Useful to local/headless adapters; callers with a selected fixture roster
/// may instead pass their validated `InputOwnership` directly to [`new`].
///
/// # Panics
///
/// Panics if either roster is malformed for slot-mode ownership.
#[must_use]
pub fn ownership_for_teams(
    home: &TeamData,
    away: &TeamData,
    players_by_id: Option<&IndexMap<&str, PlayerData>>,
) -> InputOwnership {
    let owned;
    let by_id = match players_by_id {
        Some(v) => v,
        None => {
            owned = pool_by_id();
            &owned
        }
    };
    let home_outfield = fixture_outfield_ids(home, by_id);
    let away_outfield = fixture_outfield_ids(away, by_id);
    let mut assignments: Vec<InputSlotAssignment> = Vec::with_capacity(8);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).expect("canonical slot index");
        let ids = if slot.team == input_frame::Team::Home {
            &home_outfield
        } else {
            &away_outfield
        };
        assignments.push(InputSlotAssignment {
            slot: slot.id,
            team: slot.team,
            player_id: ids[(slot.outfield_index - 1) as usize].clone(),
        });
    }
    let assignments: [InputSlotAssignment; 8] = assignments
        .try_into()
        .unwrap_or_else(|_| panic!("exactly eight slot assignments"));
    let rosters = InputFixtureRosters {
        home: home.roster.iter().map(|s| (*s).to_string()).collect(),
        away: away.roster.iter().map(|s| (*s).to_string()).collect(),
    };
    input_frame::new_ownership(&assignments, &rosters, by_id).expect("valid fixture ownership")
}

#[allow(clippy::too_many_arguments)]
fn build_team(
    team: &TeamData,
    side: Team,
    field: PitchSize,
    by_id: &IndexMap<&str, PlayerData>,
    species_by_id: &IndexMap<&str, SpeciesData>,
    showcase_by_id: &IndexMap<&str, ShowcasePlayerCompatibilityData>,
    formation_id: Option<&str>,
    line_shift: f64,
) -> Vec<MatchPlayer> {
    let formation_key = formation_id.unwrap_or(team.formation);
    let formation = formations::get(formation_key)
        .unwrap_or_else(|| panic!("unknown formation: {formation_key}"));
    let anchors = placement::anchors(
        formation,
        match side {
            Team::Home => placement::Side::Home,
            Team::Away => placement::Side::Away,
        },
        placement::Field {
            w: field.w,
            h: field.h,
        },
    );
    let shift = (if side == Team::Home { 1.0 } else { -1.0 }) * line_shift * field.w;

    let mut keeper_id: Option<&str> = None;
    let mut outfield: Vec<&str> = Vec::new();
    for id in team.roster {
        let pd = by_id
            .get(id)
            .unwrap_or_else(|| panic!("unknown player: {id}"));
        if pd.position == Position::Keeper && keeper_id.is_none() {
            keeper_id = Some(id);
        } else {
            outfield.push(id);
        }
    }
    let keeper_id = keeper_id.unwrap_or_else(|| panic!("{} roster needs a keeper", team.id));

    let mut ordered: Vec<&str> = vec![keeper_id];
    ordered.extend(outfield);

    let mut list = Vec::with_capacity(ordered.len());
    for (i, id) in ordered.iter().enumerate() {
        let pd = *by_id
            .get(id)
            .unwrap_or_else(|| panic!("unknown player: {id}"));
        let showcase = showcase_by_id.get(pd.id);
        let species_id = showcase.map_or("neutral", |s| s.species);
        let species_data = *species_by_id
            .get(species_id)
            .unwrap_or_else(|| panic!("unknown species: {species_id}"));
        let effective_stats = species::apply(pd.stats, &species_data);
        assert!(
            stats::press_discipline(effective_stats) == stats::composure(effective_stats),
            "mental-derived press discipline must share the serialized composure scalar"
        );
        let base = anchors[i];
        let ax = if i > 0 {
            (base.x + shift).clamp(PLAYER_RADIUS, field.w - PLAYER_RADIUS)
        } else {
            base.x
        };
        let anchor = Vec2::new(ax, base.y);
        let is_keeper = pd.position == Position::Keeper;
        list.push(MatchPlayer {
            id: (*id).to_string(),
            name: pd.name.to_string(),
            team: side,
            pos: Vec2::new(anchor.x, anchor.y),
            vel: Vec2::new(0.0, 0.0),
            run_vel: Vec2::new(0.0, 0.0),
            facing: Vec2::new(if side == Team::Home { 1.0 } else { -1.0 }, 0.0),
            anchor,
            species_id: species_data.id.to_string(),
            owned_verb: species_data.verb,
            move_speed: stats::move_speed(effective_stats),
            shot_speed: stats::shot_speed(effective_stats),
            dribble: (stats::dribble(effective_stats)
                + species::dribble_protection(species_data.verb))
            .clamp(0.0, 1.0),
            strength: (effective_stats.strength as f64) / 10.0,
            first_touch: stats::first_touch(effective_stats),
            header_skill: stats::header(effective_stats),
            volley_skill: stats::volley(effective_stats),
            bicycle_skill: stats::bicycle(effective_stats),
            scan_rate: stats::scan_rate(effective_stats),
            composure: stats::composure(effective_stats),
            outfield_decision: outfield_decision::new_state(None),
            is_keeper,
            radius: PLAYER_RADIUS,
            dash_cd: 0.0,
            dodge_cd: 0.0,
            dodge_timer: 0.0,
            dodge_dir: Vec2::new(0.0, 0.0),
            reach: if is_keeper {
                stats::keeper_reach(effective_stats)
            } else {
                0.0
            },
            handling: if is_keeper {
                stats::keeper_handling(effective_stats)
            } else {
                0.0
            },
            keeper_aggression: if is_keeper {
                stats::keeper_aggression(effective_stats)
            } else {
                0.0
            },
            keeper_anticipation: if is_keeper {
                stats::keeper_anticipation(effective_stats)
            } else {
                0.0
            },
            keeper_state: keeper::KeeperBehaviorState::Base,
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
            sprint_dur: stats::sprint_duration(effective_stats),
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
        });
    }
    list
}

/// Index (1-based) of the most advanced home outfield player (the default
/// controlled one).
fn most_advanced_home(players: &[MatchPlayer]) -> i64 {
    let mut best: Option<i64> = None;
    let mut best_x: Option<f64> = None;
    for (i, p) in players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            let idx = (i + 1) as i64;
            if best_x.is_none_or(|bx| p.pos.x > bx) {
                best_x = Some(p.pos.x);
                best = Some(idx);
            }
        }
    }
    best.unwrap_or(1)
}

fn is_human_player(s: &MatchState, player_idx: i64) -> bool {
    if s.slot_mode {
        return s
            .slot_for_player
            .get((player_idx - 1) as usize)
            .copied()
            .flatten()
            .is_some();
    }
    s.human_controlled && player_idx == s.controlled
}

/// Reset both teams' stable presser state.
pub fn reset_press_states(s: &mut MatchState) {
    s.outfield_press.home = outfield_press::clear(&s.outfield_press.home);
    s.outfield_press.away = outfield_press::clear(&s.outfield_press.away);
}

/// Forget the possession memory. Every lifecycle boundary routes through
/// here so a restart, a full-time whistle, or a fresh match can never leak a
/// live window into play or manufacture a turnover out of the possession
/// that preceded it.
pub fn reset_transition_state(s: &mut MatchState) {
    s.transition.clear();
}

/// Reset every player's active off-ball run intent.
pub fn reset_run_states(s: &mut MatchState) {
    for player in &mut s.players {
        if outfield_decision::is_run_intent(player.outfield_decision.intent) {
            player.outfield_decision = outfield_decision::reset(&player.outfield_decision);
        }
    }
}

/// Route single-player control through one boundary so a player becoming
/// human cannot retain a serialized AI choice from earlier in this tick.
pub fn set_controlled_player(s: &mut MatchState, player_idx: i64) {
    s.controlled = player_idx;
    {
        let player = &mut s.players[(player_idx - 1) as usize];
        if !player.is_keeper
            && player.outfield_decision.context != OutfieldDecisionContext::Ineligible
        {
            player.outfield_decision = outfield_decision::reset(&player.outfield_decision);
        }
    }
    for team in [Team::Home, Team::Away] {
        let press = team_press(s, team);
        if press.presser_index == Some(player_idx as u32) {
            set_team_press(s, team, outfield_press::clear(&press));
        }
    }
}

fn team_press(s: &MatchState, team: Team) -> OutfieldPressState {
    match team {
        Team::Home => s.outfield_press.home,
        Team::Away => s.outfield_press.away,
    }
}

fn set_team_press(s: &mut MatchState, team: Team, value: OutfieldPressState) {
    match team {
        Team::Home => s.outfield_press.home = value,
        Team::Away => s.outfield_press.away = value,
    }
}

fn team_marking(s: &MatchState, team: Team) -> MarkingConfig {
    match team {
        Team::Home => s.marking.home,
        Team::Away => s.marking.away,
    }
}

fn team_transition_windows(s: &MatchState, team: Team) -> TransitionConfig {
    match team {
        Team::Home => s.transition_windows.home,
        Team::Away => s.transition_windows.away,
    }
}

fn team_marks(s: &MatchState, team: Team) -> &[Option<i64>] {
    match team {
        Team::Home => &s.marks.home,
        Team::Away => &s.marks.away,
    }
}

fn set_team_marks(s: &mut MatchState, team: Team, value: Vec<Option<i64>>) {
    match team {
        Team::Home => s.marks.home = value,
        Team::Away => s.marks.away = value,
    }
}

fn opposite(team: Team) -> Team {
    match team {
        Team::Home => Team::Away,
        Team::Away => Team::Home,
    }
}

fn to_transition_team(team: Team) -> TransitionTeam {
    match team {
        Team::Home => TransitionTeam::Home,
        Team::Away => TransitionTeam::Away,
    }
}

/// Reset for a kickoff. `kicking` is the team restarting play (after
/// conceding, per the laws of the game); the opening kickoff is the home
/// side's.
fn place_kickoff(s: &mut MatchState, kicking: Team) {
    let half = s.field.w / 2.0;
    for p in &mut s.players {
        let mut ax = p.anchor.x;
        if p.team == Team::Home {
            ax = ax.min(half - PLAYER_RADIUS);
        } else {
            ax = ax.max(half + PLAYER_RADIUS);
        }
        p.pos = Vec2::new(ax, p.anchor.y);
        p.vel = Vec2::new(0.0, 0.0);
        p.run_vel = Vec2::new(0.0, 0.0);
        p.facing = Vec2::new(if p.team == Team::Home { 1.0 } else { -1.0 }, 0.0);
        p.dive_timer = 0.0;
        p.dive_dir = Vec2::new(0.0, 0.0);
        p.dive_delay = 0.0;
        p.dive_target = None;
        p.keeper_get_up_timer = 0.0;
        p.keeper_state = keeper::KeeperBehaviorState::Base;
        p.keeper_state_timer = 0.0;
        p.keeper_release_state = None;
        p.keeper_release_motion = 0.0;
        p.keeper_release_kind = None;
        p.keeper_release_depth = 0.0;
        p.hold_timer = 0.0;
        p.feet_ball = false;
        p.slide_timer = 0.0;
        p.slide_dir = Vec2::new(0.0, 0.0);
        p.slide_vel = 0.0;
        p.tackle_timer = 0.0;
        p.tackle_cd = 0.0;
        p.stun_timer = 0.0;
        p.grab_timer = 0.0;
        p.throw_timer = 0.0;
        p.receive_timer = 0.0;
        p.sprint_meter = 1.0;
        p.sprinting = false;
        p.save_pending = None;
        p.save_timer = 0.0;
        p.save_vx = 0.0;
        p.keeper_set = 0.0;
        p.save_style = None;
        p.save_tip_emitted = false;
        p.settle_timer = 0.0;
        p.header_cd = 0.0;
        p.aerial_timer = 0.0;
        p.aerial_style = None;
        p.aerial_outcome = None;
        p.aerial_jump = 0.0;
        p.aerial_recovery = 0.0;
        p.outfield_decision = outfield_decision::reset(&p.outfield_decision);
        p.charge = 0.0;
        p.pass_charge = 0.0;
        p.pass_target = None;
        p.pass_intent = pass_intent::reset(&p.pass_intent);
        p.windup_timer = 0.0;
        p.windup_shot = None;
        p.jockey_timer = 0.0;
        p.action = action_slot::clear(&p.action);
    }
    // Give the kicking team the ball at the centre spot.
    let kicker: i64;
    if kicking == Team::Home {
        kicker = most_advanced_home(&s.players);
        if !s.slot_mode {
            s.controlled = kicker;
        }
    } else {
        let mut best_x: Option<f64> = None;
        let mut best: Option<i64> = None;
        for (i, p) in s.players.iter().enumerate() {
            if p.team == Team::Away && !p.is_keeper {
                let idx = (i + 1) as i64;
                if best_x.is_none_or(|bx| p.pos.x < bx) {
                    best_x = Some(p.pos.x);
                    best = Some(idx);
                }
            }
        }
        kicker = best.expect("away team has an outfielder");
        if !s.slot_mode {
            s.controlled = most_advanced_home(&s.players);
        }
    }
    {
        let c = &mut s.players[(kicker - 1) as usize];
        c.facing = Vec2::new(if kicking == Team::Home { 1.0 } else { -1.0 }, 0.0);
        c.pos = Vec2::new(
            s.field.w * (if kicking == Team::Home { 0.45 } else { 0.55 }),
            s.field.h / 2.0,
        );
        s.ball = c.pos.add(c.facing.scale(STICK_AHEAD));
    }
    s.ball_vel = Vec2::new(0.0, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    set_owner(s, Some(kicker));
    s.pickup_cd = 0.0;
    s.block_grace = 0.0;
    s.aerial_lock = 0.0;
    s.ball_spin = 0.0;
    s.kickoff_hold = KICKOFF_HOLD;
    reset_press_states(s);
    // The restart hands the ball over by law, not by a turnover: clear the
    // possession memory so nobody counter-presses a kickoff.
    reset_transition_state(s);
    // Centre-circle rule: the non-kicking team keeps its distance from the
    // ball at the restart — push any intruder straight back out.
    let ball = s.ball;
    for p in &mut s.players {
        if p.team != kicking && !p.is_keeper {
            let off = p.pos.sub(ball);
            let d = off.length();
            if d < KICKOFF_CLEAR {
                let dir = if d > 0.0 {
                    off.normalized()
                } else {
                    Vec2::new(if p.team == Team::Home { -1.0 } else { 1.0 }, 0.0)
                };
                let np = ball.add(dir.scale(KICKOFF_CLEAR));
                p.pos = Vec2::new(
                    np.x.clamp(PLAYER_RADIUS, s.field.w - PLAYER_RADIUS),
                    np.y.clamp(PLAYER_RADIUS, s.field.h - PLAYER_RADIUS),
                );
            }
        }
    }
}

fn marking_of(tactic: &TacticData) -> MarkingConfig {
    tactic.marking
}

fn transition_of(tactic: &TacticData) -> TransitionConfig {
    possession_transition::copy_windows(tactic.transition)
}

/// Construct a fresh match from fixture options.
///
/// # Panics
///
/// Panics on any invariant violation in the authored content or a supplied
/// `InputOwnership`.
#[must_use]
pub fn new(opts: NewMatchOptions<'_>) -> MatchState {
    let field = opts.field;
    let owned_by_id;
    let by_id = match opts.players_by_id {
        Some(v) => v,
        None => {
            owned_by_id = pool_by_id();
            &owned_by_id
        }
    };
    let owned_species;
    let species_by_id = match opts.species_by_id {
        Some(v) => v,
        None => {
            owned_species = species_pool();
            &owned_species
        }
    };
    let owned_showcase;
    let showcase_by_id = match opts.showcase_players_by_id {
        Some(v) => v,
        None => {
            owned_showcase = showcase_pool();
            &owned_showcase
        }
    };
    let balanced = gc_data::tactics::get("balanced").expect("balanced tactic is authored");
    let home_tactic = opts.tactic.unwrap_or(balanced);
    let away_tactic = opts.away_tactic.unwrap_or(balanced);

    // Seeded randomness (grab-vs-parry rolls). Warm the state up a few
    // steps: minstd's first draws correlate with small seeds (seed 3 -> tiny
    // sample).
    let mut rstate = rng::seed(opts.seed.unwrap_or(42.0));
    for _ in 0..3 {
        let (next, _) = rng::roll(rstate);
        rstate = next;
    }

    let home = build_team(
        opts.home,
        Team::Home,
        field,
        by_id,
        species_by_id,
        showcase_by_id,
        opts.home_formation,
        home_tactic.line_shift,
    );
    let away = build_team(
        opts.away,
        Team::Away,
        field,
        by_id,
        species_by_id,
        showcase_by_id,
        None,
        away_tactic.line_shift,
    );
    let mut players = Vec::with_capacity(home.len() + away.len());
    players.extend(home);
    players.extend(away);
    for (index, player) in players.iter_mut().enumerate() {
        let seed = f64::from(rstate) + ((index as i64 + 1) * 104_729) as f64;
        player.outfield_decision = outfield_decision::new_state(Some(rng::seed(seed) as f64));
    }

    let mut slot_players: Vec<Option<i64>> = Vec::new();
    let mut slot_for_player: Vec<Option<i64>> = vec![None; players.len()];
    let slot_mode = opts.input_ownership.is_some();
    let mut stored_ownership = None;
    if let Some(ownership) = &opts.input_ownership {
        input_frame::validate_ownership(ownership, by_id).expect("valid input ownership");
        for team in [Team::Home, Team::Away] {
            let (expected, recorded): (&[&str], &[String]) = match team {
                Team::Home => (opts.home.roster, &ownership.rosters.home),
                Team::Away => (opts.away.roster, &ownership.rosters.away),
            };
            assert!(
                recorded.len() == expected.len(),
                "input ownership roster does not match fixture team"
            );
            for index in 0..expected.len() {
                assert!(
                    recorded[index] == expected[index],
                    "input ownership roster does not match fixture team"
                );
            }
        }
        let mut index_by_id: IndexMap<&str, i64> = IndexMap::new();
        for (index, player) in players.iter().enumerate() {
            index_by_id.insert(player.id.as_str(), (index + 1) as i64);
        }
        slot_players = vec![None; input_frame::SLOT_COUNT as usize];
        for slot_index in 1..=input_frame::SLOT_COUNT {
            let assignment = &ownership.slots[(slot_index - 1) as usize];
            let player_index = *index_by_id
                .get(assignment.player_id.as_str())
                .expect("slot player is not in match");
            let player = &players[(player_index - 1) as usize];
            let expected_team = match assignment.team {
                input_frame::Team::Home => Team::Home,
                input_frame::Team::Away => Team::Away,
            };
            assert!(
                player.team == expected_team,
                "slot team does not match match player"
            );
            assert!(
                !player.is_keeper,
                "keeper cannot be mapped to an input slot"
            );
            slot_players[(slot_index - 1) as usize] = Some(player_index);
            slot_for_player[(player_index - 1) as usize] = Some(slot_index);
        }
        stored_ownership =
            Some(input_frame::copy_ownership(ownership, by_id).expect("copyable input ownership"));
    }

    let mouth_y = field.h / 2.0 - GOAL_MOUTH / 2.0;
    let controlled = most_advanced_home(&players);
    let mut s = MatchState {
        field,
        goal_home: Rect {
            x: -GOAL_DEPTH,
            y: mouth_y,
            w: GOAL_DEPTH,
            h: GOAL_MOUTH,
        },
        goal_away: Rect {
            x: field.w,
            y: mouth_y,
            w: GOAL_DEPTH,
            h: GOAL_MOUTH,
        },
        players,
        ball: Vec2::new(0.0, 0.0),
        ball_vel: Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner: None,
        controlled,
        human_controlled: opts.human_controlled != Some(false),
        score: crate::match_snapshot::ByTeam { home: 0, away: 0 },
        time_left: opts.duration.unwrap_or(120.0),
        max_goals: opts.max_goals.unwrap_or(NO_GOAL_LIMIT),
        finished: false,
        pickup_cd: 0.0,
        press: crate::match_snapshot::ByTeam {
            home: home_tactic.press,
            away: away_tactic.press,
        },
        marking: crate::match_snapshot::ByTeam {
            home: marking_of(home_tactic),
            away: marking_of(away_tactic),
        },
        marks: crate::match_snapshot::ByTeam {
            home: Vec::new(),
            away: Vec::new(),
        },
        outfield_press: crate::match_snapshot::ByTeam {
            home: outfield_press::new_state(),
            away: outfield_press::new_state(),
        },
        transition_windows: TransitionWindows {
            home: transition_of(home_tactic),
            away: transition_of(away_tactic),
        },
        transition: possession_transition::new_state(),
        formation: crate::match_snapshot::ByTeam {
            home: opts
                .home_formation
                .unwrap_or(opts.home.formation)
                .to_string(),
            away: opts.away.formation.to_string(),
        },
        ball_spin: 0.0,
        rng: rstate,
        block_grace: 0.0,
        aerial_lock: 0.0,
        kickoff_hold: 0.0,
        events: Vec::new(),
        slot_mode,
        input_ownership: stored_ownership,
        slot_players,
        slot_for_player,
        input_tick: 0,
        unsupported_reason: None,
    };
    place_kickoff(&mut s, Team::Home);
    s
}

fn to_keeper_team(t: Team) -> keeper::Team {
    match t {
        Team::Home => keeper::Team::Home,
        Team::Away => keeper::Team::Away,
    }
}

fn to_offball_team(t: Team) -> offball_runs::Team {
    match t {
        Team::Home => offball_runs::Team::Home,
        Team::Away => offball_runs::Team::Away,
    }
}

fn to_keeper_rect(r: Rect) -> keeper::Rect {
    keeper::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

/// Clamp `pos` to the field bounds. Takes `field` by value (rather than
/// `&MatchState`, which every earlier draft of this helper used) so it can
/// be called while a caller separately holds a `&mut MatchPlayer` borrowed
/// from `s.players` — `PitchSize` is `Copy`, so `s.field` can be read out
/// once up front without conflicting with that borrow.
fn clamp_to_field(field: PitchSize, pos: Vec2) -> Vec2 {
    let r = PLAYER_RADIUS;
    Vec2::new(pos.x.clamp(r, field.w - r), pos.y.clamp(r, field.h - r))
}

/// Set (1-based player index -> true) of the `count` non-keepers of `team`
/// nearest the ball.
fn nearest_n(s: &MatchState, team: Team, count: i64) -> Vec<i64> {
    let mut cand: Vec<placement::DistanceCandidate> = Vec::new();
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        // The controlled player is the human's business, not an AI
        // resource: counting them here used to spend the whole chase
        // allocation on the human, leaving every AI teammate statically
        // watching a loose ball.
        if p.team == team && !p.is_keeper && !is_human_player(s, idx) {
            cand.push(placement::DistanceCandidate {
                idx: idx as usize,
                d: p.pos.dist(s.ball),
            });
        }
    }
    cand.sort_by(|a, b| {
        if placement::distance_candidate_before(*a, *b) {
            std::cmp::Ordering::Less
        } else if placement::distance_candidate_before(*b, *a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    cand.iter()
        .take(count.max(0) as usize)
        .map(|c| c.idx as i64)
        .collect()
}

/// The home outfielder best placed to defend right now: nearest to the ball
/// (the current player included — no forced change if they're already it).
fn best_defender(s: &MatchState) -> i64 {
    let mut best: Option<i64> = None;
    let mut best_d: Option<f64> = None;
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            let d = p.pos.dist(s.ball);
            if best_d.is_none_or(|bd| d < bd) {
                best_d = Some(d);
                best = Some((i + 1) as i64);
            }
        }
    }
    best.unwrap_or(s.controlled)
}

/// Manual switch: hand control to the home outfielder nearest the ball
/// (other than the current one) — the player you actually want when
/// defending.
fn next_home_outfield(s: &MatchState, cur: i64) -> i64 {
    let mut best: Option<i64> = None;
    let mut best_d: Option<f64> = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != cur {
            let d = p.pos.dist(s.ball);
            if best_d.is_none_or(|bd| d < bd) {
                best_d = Some(d);
                best = Some(idx);
            }
        }
    }
    best.unwrap_or(cur)
}

/// Launch a lob from `from` to `to` clearing height `h` over a blocker at
/// lane fraction `f`. Returns the horizontal velocity and the vertical
/// launch speed so the ball lands on `to`. Closed-form, deterministic.
fn lob_launch(from: Vec2, to: Vec2, f: f64, h: f64) -> (Vec2, f64) {
    let dir = to.sub(from);
    let d = dir.length();
    if d < 1.0 {
        return (Vec2::new(0.0, 0.0), (2.0 * h * GRAVITY).sqrt());
    }
    let f = f.clamp(0.15, 0.85);
    let mut tf = ((2.0 * h / (GRAVITY * f * (1.0 - f))).sqrt()).max(d / MAX_LOB_VH);
    tf = tf.min(1.0); // keep lobs from becoming moon-balls
    (dir.normalized().scale(d / tf), 0.5 * GRAVITY * tf)
}

/// Apply the one shared execution-noise contract at the actual release
/// seam. Targeting and flight parameters are already fixed before this
/// runs: only the final horizontal direction rotates, preserving speed and
/// every vertical quantity. Eligible releases always advance the match RNG
/// exactly once, including maximum-technique releases whose angle is zero.
fn apply_ai_outfield_execution_error(
    s: &mut MatchState,
    owner_idx: i64,
    intended_velocity: Vec2,
) -> Vec2 {
    let owner = &s.players[(owner_idx - 1) as usize];
    if owner.is_keeper || is_human_player(s, owner_idx) {
        return intended_velocity;
    }
    let first_touch = owner.first_touch;
    let composure = owner.composure;
    let (next_rng, sample) = rng::roll(s.rng);
    s.rng = next_rng;
    let angle = (sample * 2.0 - 1.0) * stats::execution_error_from_outfield(first_touch, composure);
    let (cosine, sine) = deterministic_math::cos_sin(angle);
    Vec2::new(
        intended_velocity.x * cosine - intended_velocity.y * sine,
        intended_velocity.x * sine + intended_velocity.y * cosine,
    )
}

#[allow(clippy::too_many_arguments)]
fn release_shot(
    s: &mut MatchState,
    owner_idx: i64,
    dir: Vec2,
    speed: Option<f64>,
    vz: Option<f64>,
    shot_type: Option<crate::keeper::KeeperShotType>,
) {
    let owner_team = s.players[(owner_idx - 1) as usize].team;
    let owner_id = s.players[(owner_idx - 1) as usize].id.clone();
    let launch_speed = speed.unwrap_or(s.players[(owner_idx - 1) as usize].shot_speed);
    let launch_vz = vz.unwrap_or(0.0);
    let launch_velocity =
        apply_ai_outfield_execution_error(s, owner_idx, dir.normalized().scale(launch_speed));
    let launch_dir = launch_velocity.normalized();
    let released_type = shot_type.unwrap_or(if launch_vz > 0.0 {
        crate::keeper::KeeperShotType::Chip
    } else {
        crate::keeper::KeeperShotType::Ground
    });
    let mut threatened_keeper_idx: Option<usize> = None;
    for (i, player) in s.players.iter_mut().enumerate() {
        if player.is_keeper {
            player.save_style = None;
            player.save_tip_emitted = false;
            if player.team != owner_team {
                threatened_keeper_idx = Some(i);
            }
        }
    }
    let mut on_target = false;
    let mut event_keeper_state = None;
    let mut event_keeper_depth = None;
    if let Some(ki) = threatened_keeper_idx {
        let team = s.players[ki].team;
        let goal = if team == Team::Home {
            s.goal_home
        } else {
            s.goal_away
        };
        let goal_line_x = if team == Team::Home {
            goal.x + goal.w
        } else {
            goal.x
        };
        let infield_direction = if team == Team::Home { 1.0 } else { -1.0 };
        let keeper_pos = s.players[ki].pos;
        let keeper_state = s.players[ki].keeper_state;
        let keeper_run_vel_len = s.players[ki].run_vel.length();
        let keeper_move_speed = s.players[ki].move_speed;
        {
            let keeper_player = &mut s.players[ki];
            keeper_player.keeper_release_state = Some(keeper_player.keeper_state);
            if keeper_state != crate::keeper::KeeperBehaviorState::Set {
                keeper_player.keeper_release_motion =
                    (keeper_run_vel_len / keeper_move_speed.max(1.0)).min(1.0);
            }
            keeper_player.keeper_release_kind = Some(released_type);
            keeper_player.keeper_release_depth =
                ((keeper_pos.x - goal_line_x) * infield_direction).max(0.0);
        }
        event_keeper_state = s.players[ki].keeper_release_state;
        event_keeper_depth = Some(s.players[ki].keeper_release_depth);
        let height = keeper::goal_line_height(&keeper::KeeperTrajectoryContext {
            origin: s.ball,
            direction: launch_dir,
            horizontal_speed: launch_speed,
            vertical_speed: launch_vz,
            defending_team: to_keeper_team(team),
            goal: to_keeper_rect(goal),
            friction: if launch_vz > 0.0 {
                AIR_FRICTION
            } else {
                FRICTION
            },
            gravity: GRAVITY,
        });
        on_target = height.is_some_and(|h| (0.0..CROSSBAR).contains(&h))
            && keeper::shot_targets_goal(&keeper::KeeperShotContext {
                defending_team: to_keeper_team(team),
                shooter_team: to_keeper_team(owner_team),
                origin: s.ball,
                direction: launch_dir,
                goal: to_keeper_rect(goal),
            });
    }
    s.events.push(MatchEvent {
        kind: MatchEventKind::Shot,
        x: s.ball.x,
        y: s.ball.y,
        player: Some(owner_id),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: Some(released_type),
        keeper_state: event_keeper_state,
        keeper_depth: event_keeper_depth,
        on_target: Some(on_target),
    });
    set_owner(s, None);
    s.kickoff_hold = 0.0;
    s.ball_vel = launch_velocity;
    s.ball_z = 0.0;
    s.ball_vz = launch_vz;
    s.pickup_cd = RELEASE_CD;
    s.block_grace = BLOCK_GRACE;
}

/// Opposing outfielders as interception threats against a pass by `team`.
/// Keepers are excluded: they hold their box instead of chasing lanes.
fn pass_threats(s: &MatchState, team: Team) -> Vec<ai::Threat> {
    let mut threats = Vec::new();
    for p in &s.players {
        if p.team != team && !p.is_keeper {
            threats.push(ai::Threat {
                pos: p.pos,
                speed: p.move_speed,
            });
        }
    }
    threats
}

/// Earliest lane fraction where a chaser would cut out a driven ground pass
/// from->to (paced by `pass_speed_for`), or `None` when the pass outruns
/// everyone.
fn pass_risk(from: Vec2, to: Vec2, threats: &[ai::Threat], tune: &Tuning) -> Option<f64> {
    let speed = passing::speed_for(from.dist(to), tune);
    ai::pass_intercept(
        from,
        to,
        speed,
        FRICTION,
        threats,
        POSSESS_DIST,
        POSSESS_MAX_SPEED,
    )
}

/// Release a pass from `owner_idx` to teammate `target_idx`: fires the
/// event, paces the ball by the registered distance-to-speed curve (or lobs
/// it over `blocker_f`), leads a running receiver via
/// [`crate::pass_lead::solve`], and sets the receiver running onto it so
/// passes are met instead of left to roll dead.
#[allow(clippy::too_many_arguments)]
fn release_pass(
    s: &mut MatchState,
    owner_idx: i64,
    target_idx: i64,
    mut blocker_f: Option<f64>,
    clear_h: Option<f64>,
    land_pos: Option<Vec2>,
    tune: &Tuning,
) {
    // Every producer's every release passes through here exactly once, so
    // this is the denominator #531 phase 4 needed and #535 deferred as not
    // cheap within that PR's budget: `ground_releases` (below) divided by
    // this is the fraction of releases that reach the lead-solve gate
    // (`land_pos.is_none() && blocker_f.is_none() && !target_is_keeper`),
    // not just the fraction that end up resolving as a ground pass (a
    // solved lead can still be discarded into a lob by the dink check
    // further down, which is why the two are related but not identical).
    pass_shadow_record(|tally| tally.total_releases += 1);
    let owner_pos = s.players[(owner_idx - 1) as usize].pos;
    let owner_id = s.players[(owner_idx - 1) as usize].id.clone();
    let target_pos = s.players[(target_idx - 1) as usize].pos;
    let target_is_keeper = s.players[(target_idx - 1) as usize].is_keeper;

    // THE LEAD SOLVE, and it happens here — first, against the pre-release
    // world, before this function has touched a single field. The solver
    // borrows `s` immutably; running it after the ball has been reassigned
    // would be solving against a world that no longer exists.
    //
    // A fresh predictor per release rather than a match-long one, on
    // `attempt_save`'s precedent (#486): the service is pure and its cache is
    // fingerprint-keyed, so a local instance answers identically, and its
    // per-tick budget is then a per-RELEASE budget that no other consumer can
    // have spent. That makes the burst independent of query ORDER, which is
    // what keeps a resimulated release bit-identical to the original even if
    // some other consumer's query count differs between the two timelines.
    //
    // Not solved for a lob (the arc is `lob_launch`'s, not a ground roll's),
    // for a planned keeper throw (`land_pos` is already placed), or into a
    // keeper's gloves.
    let lead = if land_pos.is_none() && blocker_f.is_none() && !target_is_keeper {
        let owner_verb = s.players[(owner_idx - 1) as usize].owned_verb;
        let mut predictor = pass_lead::release_predictor();
        pass_lead::solve(
            s,
            &mut predictor,
            owner_pos,
            &s.players[(target_idx - 1) as usize],
            POSSESS_DIST,
            species::link_pass_speed(owner_verb),
            tune,
        )
    } else {
        None
    };

    // A defender right on the release point eats a driven ball — and even a
    // lob is still low in its first strides (the lane check ignores segment
    // ends). Dink over them: an arc that clears at 15% of the lane also
    // stays above head height through the middle, so any mid-lane blocker
    // is cleared too. (A planned throw — land_pos set — already cleared its
    // own lane; the dink would LOWER its arc back into the presser's
    // reach.)
    if land_pos.is_none() {
        let dirn = target_pos.sub(owner_pos).normalized();
        for (qi, q) in s.players.iter().enumerate() {
            let qidx = (qi + 1) as i64;
            if qidx != owner_idx && qidx != target_idx && !q.is_keeper {
                let off = q.pos.sub(owner_pos);
                let d = off.length();
                if d < RELEASE_DINK_DIST && (off.x * dirn.x + off.y * dirn.y) > d * 0.2 {
                    blocker_f = Some(0.15);
                    break;
                }
            }
        }
    }

    // A keeper receiver gets a long window: it must keep coming for the
    // pass (or its dying roll) until the ball is actually resolved, not for
    // a beat.
    s.players[(target_idx - 1) as usize].receive_timer = if target_is_keeper {
        KEEPER_RECEIVE_TIME
    } else {
        RECEIVE_TIME
    };
    s.events.push(MatchEvent {
        kind: MatchEventKind::Pass,
        x: s.ball.x,
        y: s.ball.y,
        player: Some(owner_id),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    set_owner(s, None);
    s.kickoff_hold = 0.0;
    s.ball_z = 0.0;
    s.ball_spin = 0.0;
    s.pickup_cd = RELEASE_CD;
    s.block_grace = BLOCK_GRACE;
    if let Some(f) = blocker_f {
        let (vel, vz) = lob_launch(
            owner_pos,
            land_pos.unwrap_or(target_pos),
            f,
            clear_h.unwrap_or(LOB_CLEAR_H),
        );
        s.ball_vel = vel;
        s.ball_vz = vz;
    } else {
        // Aim at the solved lead point when one was admissible, and at the
        // receiver's feet otherwise — the issue's unled fallback, which is a
        // real pass rather than a failure.
        let aim_pt = lead.map_or(target_pos, |solution| solution.point);
        pass_shadow_record(|tally| {
            tally.ground_releases += 1;
            tally.lead_time_sum += lead.map_or(0.0, |solution| solution.lead_time);
        });
        let d = owner_pos.dist(aim_pt);
        let owner_verb = s.players[(owner_idx - 1) as usize].owned_verb;
        let pass_speed = passing::speed_for(d, tune) * species::link_pass_speed(owner_verb);
        s.ball_vel = aim_pt.sub(owner_pos).normalized().scale(pass_speed);
        s.ball_vz = 0.0;
    }
    s.ball_vel = apply_ai_outfield_execution_error(s, owner_idx, s.ball_vel);
    // Control follows a HUMAN pass to its receiver (standard soccer-game
    // behavior): you take over the man the ball is travelling to — attack
    // the cross, time the first touch — while it is still in flight.
    // Resolve execution first while the releasing owner is still the
    // controlled player. A back-pass is the exception: the keeper AI steps
    // out to meet it, and control hands over in step() when the keeper
    // traps it.
    if !s.slot_mode
        && is_human_player(s, owner_idx)
        && s.players[(target_idx - 1) as usize].team == Team::Home
        && !target_is_keeper
    {
        set_controlled_player(s, target_idx);
    }
}

/// Pure receiver selection for an outfield pass: returns the player index
/// that would receive a pass if released right now, or `None` if nobody is
/// available. The own keeper is a valid receiver like anyone else — but
/// only via the aim cone (a deliberate back-pass); the openness fallback
/// never panics it home. Aim SQUARE at the keeper (best-aligned of all
/// candidates, within `BACKPASS_AIM_COS`) and the keeper wins outright: the
/// generic scoring's distance penalty must not hand a long deliberate
/// back-pass to a mid-lane defender instead.
///
/// Does NOT draw from `s.rng` — deterministic, safe to call every frame for
/// preview.
/// Exposed for the integration tests in `tests/match.rs`, per ARCHITECTURE.md
/// §3 rule 6 ("everything a test touches is `pub`"): crates here are internal, so
/// visibility is not worth fighting to keep a spec case unportable.
pub fn select_pass_target(
    s: &MatchState,
    owner_idx: i64,
    lofted: bool,
    aim: Option<Vec2>,
    range: Option<f64>,
    tune: &Tuning,
) -> Option<i64> {
    let owner = &s.players[(owner_idx - 1) as usize];
    let owner_pos = owner.pos;
    let owner_team = owner.team;
    let aim = aim.unwrap_or(owner.facing);
    let mut cand: Vec<i64> = Vec::new();
    let mut positions: Vec<Vec2> = Vec::new();
    let mut opp_positions: Vec<Vec2> = Vec::new();
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == owner_team && idx != owner_idx {
            cand.push(idx);
            positions.push(p.pos);
        } else if p.team != owner_team {
            opp_positions.push(p.pos);
        }
    }
    // Deliberate back-pass: the keeper is the best-aligned candidate and
    // the aim points near-square at it — it receives, however far it
    // stands.
    {
        let naim = aim.normalized();
        if naim.x != 0.0 || naim.y != 0.0 {
            let mut best_cos: Option<f64> = None;
            let mut best_idx: Option<i64> = None;
            for (k, pk) in positions.iter().enumerate() {
                let to = pk.sub(owner_pos);
                let d = to.length();
                if d > 1.0 {
                    let cos = (to.x * naim.x + to.y * naim.y) / d;
                    if best_cos.is_none_or(|bc| cos > bc) {
                        best_cos = Some(cos);
                        best_idx = Some(cand[k]);
                    }
                }
            }
            if let (Some(bi), Some(bc)) = (best_idx, best_cos)
                && s.players[(bi - 1) as usize].is_keeper
                && bc >= BACKPASS_AIM_COS
            {
                return Some(bi);
            }
        }
    }
    // Past the deliberate back-pass, the own keeper is NOT a soft-cone
    // candidate, and that is a consequence of deleting the hard cone rather
    // than a policy change. The old 60-degree gate made "the keeper is a
    // candidate like anyone else" safe: a keeper outside the cone was
    // invisible. With a soft blend nobody is ever invisible, so an unaimed or
    // wide pass would pick the keeper on raw proximity alone — the panic
    // back-pass `a_blind_pass_under_no_aim_never_dumps_the_ball_at_the_keeper`
    // has guarded against since long before #491. The keeper stays reachable
    // by the ONE route that is unambiguously deliberate: aiming square at it,
    // handled above.
    let mut cand: Vec<i64> = cand;
    let mut positions: Vec<Vec2> = positions;
    {
        let mut kept_cand = Vec::with_capacity(cand.len());
        let mut kept_pos = Vec::with_capacity(positions.len());
        for (k, idx) in cand.iter().enumerate() {
            if !s.players[(*idx - 1) as usize].is_keeper {
                kept_cand.push(*idx);
                kept_pos.push(positions[k]);
            }
        }
        cand = kept_cand;
        positions = kept_pos;
    }
    let knobs = passing::SelectionKnobs::of(tune);
    let mut rel: Option<usize>;
    let pick_cand: Vec<i64>;
    let pick_pos: Vec<Vec2>;
    if !lofted {
        let threats = pass_threats(s, owner_team);
        let mut safe_cand: Vec<i64> = Vec::new();
        let mut safe_pos: Vec<Vec2> = Vec::new();
        for (k, idx) in cand.iter().enumerate() {
            if pass_risk(owner_pos, positions[k], &threats, tune).is_none() {
                safe_cand.push(*idx);
                safe_pos.push(positions[k]);
            }
        }
        rel = passing::select_receiver(owner_pos, aim, &safe_pos, range, &knobs);
        if rel.is_some() {
            pick_cand = safe_cand;
            pick_pos = safe_pos;
        } else {
            rel = passing::select_receiver(owner_pos, aim, &positions, range, &knobs);
            if let Some(r) = rel {
                pick_cand = cand.clone();
                pick_pos = positions.clone();
                return finish_select_pass_target(
                    s, owner_team, owner_pos, lofted, s.field, r, pick_cand, pick_pos,
                );
            }
            return select_pass_target_fallback(s, owner_team, owner_pos, cand, positions, lofted);
        }
    } else {
        rel = passing::select_receiver(owner_pos, aim, &positions, range, &knobs);
        if let Some(r) = rel {
            pick_cand = cand.clone();
            pick_pos = positions.clone();
            return finish_select_pass_target(
                s, owner_team, owner_pos, lofted, s.field, r, pick_cand, pick_pos,
            );
        }
        return select_pass_target_fallback(s, owner_team, owner_pos, cand, positions, lofted);
    }
    finish_select_pass_target(
        s,
        owner_team,
        owner_pos,
        lofted,
        s.field,
        rel.expect("rel is Some on this path"),
        pick_cand,
        pick_pos,
    )
}

/// Fallback (nobody in the cone) considers outfielders only: an unaimed
/// pass must never dump the ball back at the keeper.
fn select_pass_target_fallback(
    s: &MatchState,
    owner_team: Team,
    owner_pos: Vec2,
    cand: Vec<i64>,
    positions: Vec<Vec2>,
    lofted: bool,
) -> Option<i64> {
    let mut opp_positions: Vec<Vec2> = Vec::new();
    for p in &s.players {
        if p.team != owner_team {
            opp_positions.push(p.pos);
        }
    }
    let mut best_fb: Option<f64> = None;
    let mut rel: Option<usize> = None;
    for (k, pk) in positions.iter().enumerate() {
        if !s.players[(cand[k] - 1) as usize].is_keeper {
            let mut open = f64::INFINITY;
            for qp in &opp_positions {
                open = open.min(qp.dist(*pk));
            }
            let score = open.min(80.0) - owner_pos.dist(*pk) * 0.15;
            if best_fb.is_none_or(|bf| score > bf) {
                best_fb = Some(score);
                rel = Some(k);
            }
        }
    }
    let rel = rel?;
    finish_select_pass_target(
        s, owner_team, owner_pos, lofted, s.field, rel, cand, positions,
    )
}

/// Cross override (lofted from wide in attacking third): redirect to box
/// runner.
#[allow(clippy::too_many_arguments)]
fn finish_select_pass_target(
    s: &MatchState,
    owner_team: Team,
    owner_pos: Vec2,
    lofted: bool,
    field: PitchSize,
    rel: usize,
    pick_cand: Vec<i64>,
    _pick_pos: Vec<Vec2>,
) -> Option<i64> {
    let mut rel = rel;
    if lofted {
        let third = (owner_team == Team::Home && owner_pos.x > field.w * 0.62)
            || (owner_team == Team::Away && owner_pos.x < field.w * 0.38);
        let wide = (owner_pos.y - field.h / 2.0).abs() > 120.0;
        if third && wide {
            let mut best_k: Option<usize> = None;
            let mut best_d: Option<f64> = None;
            for (k, i2) in pick_cand.iter().enumerate() {
                let q = s.players[(*i2 - 1) as usize].pos;
                let depth = if owner_team == Team::Home {
                    field.w - q.x
                } else {
                    q.x
                };
                if depth < 220.0 && (q.y - field.h / 2.0).abs() < 140.0 {
                    let gd = depth + (q.y - field.h / 2.0).abs();
                    if best_d.is_none_or(|bd| gd < bd) {
                        best_d = Some(gd);
                        best_k = Some(k);
                    }
                }
            }
            if let Some(bk) = best_k {
                rel = bk;
            }
        }
    }
    Some(pick_cand[rel])
}

/// Pure receiver selection for a keeper throw: returns the player index
/// that would receive the throw if released right now, or `None` if nobody
/// is available. Does NOT draw from `s.rng` — deterministic, safe to call
/// every frame for preview.
fn select_throw_target(
    s: &MatchState,
    keeper_idx: i64,
    range: f64,
    aim: Option<Vec2>,
) -> Option<i64> {
    let keeper = &s.players[(keeper_idx - 1) as usize];
    let keeper_pos = keeper.pos;
    let keeper_team = keeper.team;
    let aim = aim.unwrap_or(keeper.facing);
    let mut cand: Vec<i64> = Vec::new();
    let mut positions: Vec<Vec2> = Vec::new();
    let mut opp_positions: Vec<Vec2> = Vec::new();
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == keeper_team && idx != keeper_idx && !p.is_keeper {
            cand.push(idx);
            positions.push(p.pos);
        } else if p.team != keeper_team {
            opp_positions.push(p.pos);
        }
    }
    let naim = aim.normalized();
    let mut rel: Option<usize> = None;
    let mut best_score: Option<f64> = None;
    if naim.x != 0.0 || naim.y != 0.0 {
        for (k, pk) in positions.iter().enumerate() {
            let to = pk.sub(keeper_pos);
            let d = to.length();
            if d > 1.0 {
                let cos = (to.x * naim.x + to.y * naim.y) / d;
                if cos >= 0.5 {
                    let tf = ((2.0 * THROW_CLEAR_H / (GRAVITY * 0.25)).sqrt())
                        .max(d / MAX_LOB_VH)
                        .min(1.0);
                    let mut open = f64::INFINITY;
                    for qp in &opp_positions {
                        open = open.min(qp.dist(*pk) - tf * 170.0);
                    }
                    let score =
                        cos * 4.0 - (d - range).abs() / 150.0 + open.clamp(0.0, 100.0) / 40.0;
                    if best_score.is_none_or(|bs| score > bs) {
                        best_score = Some(score);
                        rel = Some(k);
                    }
                }
            }
        }
    }
    if rel.is_none() {
        let mut best_fb: Option<f64> = None;
        for (k, pk) in positions.iter().enumerate() {
            let d = keeper_pos.dist(*pk);
            let tf = ((2.0 * THROW_CLEAR_H / (GRAVITY * 0.25)).sqrt())
                .max(d / MAX_LOB_VH)
                .min(1.0);
            let mut open = f64::INFINITY;
            for qp in &opp_positions {
                open = open.min(qp.dist(*pk) - tf * 170.0);
            }
            let score = open.min(100.0) - (d - range).abs() * 0.2;
            if best_fb.is_none_or(|bf| score > bf) {
                best_fb = Some(score);
                rel = Some(k);
            }
        }
    }
    rel.map(|r| cand[r])
}

fn try_pass(s: &mut MatchState, owner_idx: i64, lofted: bool, aim: Option<Vec2>, tune: &Tuning) {
    let owner = &s.players[(owner_idx - 1) as usize];
    let aim = aim.unwrap_or(owner.facing);
    let owner_team = owner.team;
    let owner_pos = owner.pos;
    let pass_charge = owner.pass_charge;
    // Hold-to-charge picks the RANGE: a tap prefers someone close, a
    // charged release picks out the long option along the aim.
    let range = if pass_charge > 0.12 {
        Some(
            pass_range_min(tune)
                + pass_charge * (tune.value("PASS_RANGE_MAX") - pass_range_min(tune)),
        )
    } else {
        None
    };
    let target_idx = select_pass_target(s, owner_idx, lofted, Some(aim), range, tune);
    let Some(target_idx) = target_idx else {
        return;
    };
    // The soft cone's own accuracy, recorded where the aim actually exists.
    // `select_pass_target` is pure and called every PREVIEW frame too, so
    // recording inside it would count frames instead of releases.
    pass_shadow_record(|tally| {
        tally.aimed_releases += 1;
        tally.aim_error_sum += passing::angular_term(
            aim.normalized(),
            s.players[(target_idx - 1) as usize].pos.sub(owner_pos),
        );
    });
    // Determine loft: cross gets CROSS_CLEAR_H; regular lob gets
    // lane-blocker fraction.
    let mut opp_positions: Vec<Vec2> = Vec::new();
    for p in &s.players {
        if p.team != owner_team {
            opp_positions.push(p.pos);
        }
    }
    let target_pos = s.players[(target_idx - 1) as usize].pos;
    let mut clear_h = None;
    let mut f = None;
    if lofted {
        let third = (owner_team == Team::Home && owner_pos.x > s.field.w * 0.62)
            || (owner_team == Team::Away && owner_pos.x < s.field.w * 0.38);
        let wide = (owner_pos.y - s.field.h / 2.0).abs() > 120.0;
        if third && wide {
            let depth = if owner_team == Team::Home {
                s.field.w - target_pos.x
            } else {
                target_pos.x
            };
            if depth < 220.0 && (target_pos.y - s.field.h / 2.0).abs() < 140.0 {
                clear_h = Some(CROSS_CLEAR_H);
            }
        }
        f = Some(
            ai::lane_blocker(owner_pos, target_pos, &opp_positions, POSSESS_DIST).unwrap_or(0.5),
        );
    }
    release_pass(s, owner_idx, target_idx, f, clear_h, None, tune);
}

/// Plan a keeper HAND throw to `target_idx`. Hands see the whole pitch: a
/// throw is not a hopeful ball, it is placed. Returns `None` for the
/// blocker fraction when the straight line to `target_idx` (after any
/// near-opponent lead shift) is genuinely clear of `THROW_LANE_W` --
/// `keeper_throw` reads that as "bowl it, don't loft it": a fully unmarked
/// throw was always released flat and near-instantly, both before #531 (as
/// `keeper_distribute`'s own clear-lane ground-bowl case, deleted with
/// `release_throw`) and now. Losing that distinction after #531 landed
/// meant EVERY throw, marked or not, spent a full lob's hang time and
/// post-landing roll in the air or on a loose ball — genuinely contestable
/// the whole way, not just when an opponent stood in the lane. Restoring
/// it here (rather than only in the caller that first exposed it) fixes it
/// for both keepers this function serves, exactly like `try_pass`'s own
/// `lofted` parameter already does for outfield passes.
fn plan_throw(s: &MatchState, keeper_idx: i64, target_idx: i64) -> (Vec2, Option<f64>, f64) {
    let keeper = &s.players[(keeper_idx - 1) as usize];
    let keeper_pos = keeper.pos;
    let keeper_team = keeper.team;
    let keeper_facing = keeper.facing;
    let target_pos = s.players[(target_idx - 1) as usize].pos;
    let mut near_d: Option<f64> = None;
    let mut near_opp_pos: Option<Vec2> = None;
    for q in &s.players {
        if q.team != keeper_team {
            let d = q.pos.dist(target_pos);
            if near_d.is_none_or(|nd| d < nd) {
                near_d = Some(d);
                near_opp_pos = Some(q.pos);
            }
        }
    }
    let mut land = target_pos;
    if let (Some(near_opp_pos), Some(near_d)) = (near_opp_pos, near_d)
        && near_d < THROW_COVER_DIST
    {
        let away = target_pos.sub(near_opp_pos);
        let dir = if away.length() > 1.0 {
            away.normalized()
        } else {
            keeper_facing
        };
        let lead = THROW_LEAD_MAX.min((THROW_COVER_DIST - near_d) * 0.5);
        land = Vec2::new(
            (target_pos.x + dir.x * lead).clamp(25.0, s.field.w - 25.0),
            (target_pos.y + dir.y * lead).clamp(25.0, s.field.h - 25.0),
        );
    }
    let mut opp_positions: Vec<Vec2> = Vec::new();
    for q in &s.players {
        if q.team != keeper_team {
            opp_positions.push(q.pos);
        }
    }
    let f = ai::lane_blocker(keeper_pos, land, &opp_positions, THROW_LANE_W);
    match f {
        Some(f) => (land, Some(f.clamp(0.2, 0.8)), aerial::max_touch_z() + 16.0),
        None => (land, None, THROW_CLEAR_H),
    }
}

/// Keeper throw, driven by an ordinary `MatchInput` -- a human's or a
/// charging AI's (#531): aimed like a pass (facing cone), the charged range
/// picking WHICH teammate. The flight comes from `plan_throw`
/// (uninterferable): lofted over a marked lane, or bowled flat and fast
/// when the lane is genuinely clear -- see that function's doc.
fn keeper_throw(s: &mut MatchState, keeper_idx: i64, range: f64, aim: Option<Vec2>, tune: &Tuning) {
    let keeper_facing = s.players[(keeper_idx - 1) as usize].facing;
    let aim = aim.unwrap_or(keeper_facing);
    let target_idx = select_throw_target(s, keeper_idx, range, Some(aim));
    let Some(target_idx) = target_idx else {
        return;
    };
    let (land, f, clear_h) = plan_throw(s, keeper_idx, target_idx);
    match f {
        Some(f) => release_pass(
            s,
            keeper_idx,
            target_idx,
            Some(f),
            Some(clear_h),
            Some(land),
            tune,
        ),
        None => release_pass(s, keeper_idx, target_idx, None, None, None, tune),
    }
    s.players[(keeper_idx - 1) as usize].throw_timer = KEEPER_THROW_POSE;
}

/// Build the existing one-per-teammate pass set with its openness,
/// progress, distance, lane and interception inputs. No RNG is consumed
/// here.
fn ai_pass_eligible(distance: f64, openness: f64) -> bool {
    (AI_PASS_MIN_DIST..=AI_PASS_MAX_DIST).contains(&distance) && openness >= AI_PASS_MIN_OPEN
}

fn ai_pass_options(
    s: &MatchState,
    owner_idx: i64,
    tune: &Tuning,
) -> Vec<outfield_decision::OutfieldPassOption> {
    let owner = &s.players[(owner_idx - 1) as usize];
    let owner_pos = owner.pos;
    let owner_team = owner.team;
    let fwd = if owner_team == Team::Home { 1.0 } else { -1.0 };
    let mut opp_positions: Vec<Vec2> = Vec::new();
    for p in &s.players {
        if p.team != owner_team {
            opp_positions.push(p.pos);
        }
    }
    let threats = pass_threats(s, owner_team);
    let mut options = Vec::new();
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == owner_team && idx != owner_idx && !p.is_keeper {
            let d = owner_pos.dist(p.pos);
            let mut open = f64::INFINITY;
            for qp in &opp_positions {
                open = open.min(qp.dist(p.pos));
            }
            if ai_pass_eligible(d, open) {
                let blocked = ai::lane_blocker(owner_pos, p.pos, &opp_positions, POSSESS_DIST);
                let risk = if blocked.is_none() {
                    pass_risk(owner_pos, p.pos, &threats, tune)
                } else {
                    None
                };
                options.push(outfield_decision::OutfieldPassOption {
                    player_index: idx as u32,
                    openness: open,
                    forward_progress: (p.pos.x - owner_pos.x) * fwd,
                    distance: d,
                    lane_blocked: blocked.is_some(),
                    interception_risk: risk.is_some(),
                    lane_fraction: blocked.or(risk),
                });
            }
        }
    }
    options
}

/// Resolve the existing cross receiver and count all legitimate box
/// targets. The selected action still releases through the established
/// pass/cross path.
fn ai_cross_target(s: &MatchState, owner_idx: i64) -> (Option<i64>, i64) {
    let owner = &s.players[(owner_idx - 1) as usize];
    let owner_team = owner.team;
    let mut best: Option<i64> = None;
    let mut best_d: Option<f64> = None;
    let mut box_targets = 0i64;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == owner_team && idx != owner_idx && !p.is_keeper {
            let depth = if owner_team == Team::Home {
                s.field.w - p.pos.x
            } else {
                p.pos.x
            };
            if depth < 220.0 && (p.pos.y - s.field.h / 2.0).abs() < 140.0 {
                box_targets += 1;
                let gd = depth + (p.pos.y - s.field.h / 2.0).abs();
                if best_d.is_none_or(|bd| gd < bd) {
                    best_d = Some(gd);
                    best = Some(idx);
                }
            }
        }
    }
    (best, box_targets)
}

fn attack_goal(s: &MatchState, team: Team) -> Rect {
    if team == Team::Home {
        s.goal_away
    } else {
        s.goal_home
    }
}

fn team_keeper(s: &MatchState, team: Team) -> Option<i64> {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == team && p.is_keeper {
            return Some((i + 1) as i64);
        }
    }
    None
}

/// World point in the opponent goal to aim at. `vbias` in `[-1, 1]` picks
/// vertical placement (0 = centre, +/-1 = the posts).
fn shot_target(s: &MatchState, shooter_team: Team, vbias: f64) -> Vec2 {
    let g = attack_goal(s, shooter_team);
    let gx = if shooter_team == Team::Home {
        g.x
    } else {
        g.x + g.w
    };
    let cy = g.y + g.h / 2.0;
    let half = g.h / 2.0 - 8.0;
    Vec2::new(gx, cy + vbias.clamp(-1.0, 1.0) * half)
}

/// True when the ball is inside the keeper's claim zone (its penalty area):
/// close to its own goal line and within the mouth +/- a margin. The
/// keeper comes off its line to gather loose balls here and to close down
/// a carrier.
fn in_claim_zone(s: &MatchState, keeper_idx: i64) -> bool {
    let keeper_team = s.players[(keeper_idx - 1) as usize].team;
    let g = if keeper_team == Team::Home {
        s.goal_home
    } else {
        s.goal_away
    };
    let depth = if keeper_team == Team::Home {
        s.ball.x
    } else {
        s.field.w - s.ball.x
    };
    depth <= KEEPER_BOX_DEPTH
        && s.ball.y >= g.y - KEEPER_BOX_PAD
        && s.ball.y <= g.y + g.h + KEEPER_BOX_PAD
}

/// Where a keeper holds a gathered ball: at its hands, but clamped safely
/// inside the line so the hold itself can never read as a goal in
/// `check_goal` (which counts ball + radius).
fn keeper_hold_pos(s: &MatchState, keeper_idx: i64) -> Vec2 {
    let keeper = &s.players[(keeper_idx - 1) as usize];
    let mut hold_x = keeper.pos.x;
    if keeper.team == Team::Home {
        hold_x = hold_x.max(s.goal_home.x + s.goal_home.w + BALL_RADIUS + 1.0);
    } else {
        hold_x = hold_x.min(s.goal_away.x - BALL_RADIUS - 1.0);
    }
    Vec2::new(hold_x, keeper.pos.y)
}

/// The single choke point for changing ball ownership (#489). Applies the
/// possession invariant here, once: the OUTGOING owner's committed action
/// (whichever verb, whichever phase) clears unconditionally, because
/// `action_slot::clear` does not ask which verb it is clearing.
///
/// Every change to a `MatchState::owner` field anywhere in this crate goes
/// through this function instead of touching the field directly —
/// including the three outside this module (`crate::combat`'s ball spill,
/// and `crate::rollback_validation`'s two scenario builders), which is why
/// this is `pub(crate)` rather than private. The one exception is the
/// `owner: None` initialiser in [`new`]'s `MatchState` literal, where there
/// is no outgoing owner to clear because the state does not exist yet.
///
/// `tests/action_slot_possession_invariant.rs` is the structural proof of
/// that paragraph: it scans every source file in this crate and fails on
/// any assignment, mutating `Option` call, `&mut` borrow, or
/// `MatchState { owner: … }` literal outside those two windows, so a future
/// verb's bypass is a red test rather than a silent regression. A spot
/// check of one scripted possession change could not say the same — see
/// that file's module doc, which also records the scan's own two limits and
/// the two construction-time `owner` writes in `gc-wasm` that sit outside
/// the invariant's scope.
///
/// ## Ordering, for a verb whose own execution ends the possession
///
/// Tackle — the only verb wired to the slot today — never holds the ball
/// while it charges or executes, so this clear never lands on the actor:
/// `resolve_tackle` calls `win_ball` (which sets the owner) and only then
/// resolves the CHALLENGER's slot, and the challenger is the incoming
/// owner, not the outgoing one. A verb that releases the ball itself (the
/// shot and pass migrations #489 anticipates) inverts that: the actor IS
/// the outgoing owner, so `release_shot`/`release_pass` would clear the
/// actor's own slot out from under it, and `action_slot::resolve_success`
/// / `resolve_miss` would then panic on the `Executing` phase the clear
/// erased. That is a real decision for the migration — resolve the slot
/// before handing possession over, or give this function an explicit
/// actor exemption — and it is left open here rather than pre-decided by
/// a verb that cannot exercise it.
pub(crate) fn set_owner(s: &mut MatchState, new_owner: Option<i64>) {
    if s.owner != new_owner
        && let Some(outgoing) = s.owner
    {
        let p = &mut s.players[(outgoing - 1) as usize];
        p.action = action_slot::clear(&p.action);
    }
    s.owner = new_owner;
}

/// Distance from `challenger_idx` to the ball, and the effective reach a
/// standing-poke tackle needs to beat it, against `target_idx` (the
/// player this specific committed poke is aimed at). The exact
/// `d`/`reach`/`species_reach` computation `attempt_steals` used to make
/// once per continuous check; now made once at the arrival-trigger check
/// and once again at resolution, both inside `advance_tackle_actions`.
fn tackle_distance_and_reach(
    s: &MatchState,
    challenger_idx: i64,
    target_idx: i64,
    human: bool,
) -> (f64, f64) {
    let challenger = &s.players[(challenger_idx - 1) as usize];
    let target = &s.players[(target_idx - 1) as usize];
    let mut d = challenger.pos.dist(s.ball);
    let species_reach = species::collision_reach(challenger.owned_verb)
        - species::dribble_protection(target.owned_verb);
    // The human's poke also works at body-contact range from ANY angle (a
    // toe through the legs): chasing a carrier is the default defensive
    // situation and must be winnable.
    if human && challenger.pos.dist(target.pos) <= STEAL_DIST + species_reach {
        d = d.min(STEAL_DIST);
    }
    let reach = if human {
        let jockey_bonus = if challenger.jockey_timer > 0.0 {
            JOCKEY_REACH_BONUS
        } else {
            0.0
        };
        STAND_REACH + jockey_bonus
    } else {
        STEAL_DIST
    };
    (d, reach + species_reach)
}

/// Win the ball off `owner_idx` for `challenger_idx`: pop it loose toward
/// the challenger, cancel the owner's pending wind-up, and (for a slide)
/// knock the owner down. Shared by the slide tackle (`attempt_steals`,
/// still an instantaneous, continuously-checked lunge) and a resolved
/// standing-poke tackle (`advance_tackle_actions`) so the two producers
/// cannot drift.
fn win_ball(s: &mut MatchState, owner_idx: i64, challenger_idx: i64, sliding: bool) {
    let owner_pos = s.players[(owner_idx - 1) as usize].pos;
    let p_pos = s.players[(challenger_idx - 1) as usize].pos;
    let p_facing = s.players[(challenger_idx - 1) as usize].facing;
    let mut dir = p_pos.sub(owner_pos);
    if dir.x == 0.0 && dir.y == 0.0 {
        dir = p_facing;
    }
    let pid = s.players[(challenger_idx - 1) as usize].id.clone();
    s.events.push(MatchEvent {
        kind: MatchEventKind::Tackle,
        x: owner_pos.x,
        y: owner_pos.y,
        player: Some(pid),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    {
        let owner_mut = &mut s.players[(owner_idx - 1) as usize];
        owner_mut.windup_timer = 0.0;
        owner_mut.windup_shot = None;
    }
    for player in &mut s.players {
        player.keeper_set = 0.0;
    }
    set_owner(s, None);
    s.ball_vel = dir.normalized().scale(TACKLE_POP_SPEED);
    s.pickup_cd = 0.12;
    if sliding {
        let owner_mut = &mut s.players[(owner_idx - 1) as usize];
        owner_mut.stun_timer = owner_mut.stun_timer.max(STUN_TIME);
    }
}

/// Resolve a standing-poke tackle whose executing phase is due: a single
/// check against the target's THEN-current position (not the continuous
/// per-tick check the old instant-resolve tackle used) — see
/// `crate::action_slot`'s module doc and this PR's description for why a
/// longer `ACTION_TACKLE_COMMIT` therefore raises the whiff rate rather
/// than only ever helping. A target that no longer owns the ball at all
/// (it changed hands to someone else, or went out, while this charge was
/// committed) is an automatic miss: commit immunity keeps the swing on
/// schedule, but there is nothing left to win it from.
/// Whether `target_idx` is still a legitimate standing-poke target for
/// `challenger_idx`: they must still hold the ball, and must be on the
/// OPPOSING team -- never a teammate and never `challenger_idx` itself.
/// Shared by the charging-phase abort check and the executing-phase hit
/// check so the two can never disagree about what counts as a live target.
fn tackle_target_is_live(s: &MatchState, challenger_idx: i64, target_idx: i64) -> bool {
    s.owner == Some(target_idx)
        && target_idx != challenger_idx
        && s.players[(target_idx - 1) as usize].team
            != s.players[(challenger_idx - 1) as usize].team
}

fn resolve_tackle(s: &mut MatchState, challenger_idx: i64, miss_recovery: f64) {
    let target = s.players[(challenger_idx - 1) as usize]
        .action
        .target_player;
    let human = is_human_player(s, challenger_idx);
    let hit = target.is_some_and(|t| {
        tackle_target_is_live(s, challenger_idx, t) && {
            let (d, reach) = tackle_distance_and_reach(s, challenger_idx, t, human);
            d <= reach
        }
    });
    if hit {
        let owner_idx = target.expect("hit requires a live target");
        win_ball(s, owner_idx, challenger_idx, false);
        let p = &mut s.players[(challenger_idx - 1) as usize];
        p.action = action_slot::resolve_success(&p.action);
    } else {
        let (px, py, pid) = {
            let p = &s.players[(challenger_idx - 1) as usize];
            (p.pos.x, p.pos.y, p.id.clone())
        };
        s.events.push(MatchEvent {
            kind: MatchEventKind::TackleMiss,
            x: px,
            y: py,
            player: Some(pid),
            save_style: None,
            style: None,
            outcome: None,
            jumping: None,
            difficulty: None,
            shot_type: None,
            keeper_state: None,
            keeper_depth: None,
            on_target: None,
        });
        let p = &mut s.players[(challenger_idx - 1) as usize];
        p.action = action_slot::resolve_miss(&p.action, miss_recovery);
    }
}

/// Advance every player's standing-poke tackle action slot by one tick:
/// charge, release, resolve, and recover (#489). Possession changes and the
/// invariant are handled as they happen, at the single `set_owner` choke
/// point; this function is the rest of the per-tick ordering the issue
/// documents -- advance phase timers, check release triggers in stable
/// player order, resolve a due execution, enter recovery for a resolved
/// miss.
///
/// Pass 1 advances every player already mid-action, in stable index order.
/// Pass 2 considers a FRESH AI commit for anyone idle -- a human's commit
/// is decided in `move_human_player`, where the raw button edge lives, on
/// the same #531 discipline `pass_intent`/`combat_intent` already follow:
/// the AI never gets a private fast path, only the same `MatchInput`-shaped
/// decision a human's press represents.
fn advance_tackle_actions(
    s: &mut MatchState,
    combat_state: Option<&CombatMatchState>,
    dt: f64,
    tune: &Tuning,
) {
    let full_charge = tune.value("ACTION_TACKLE_FULL_CHARGE");
    let commit_seconds = tune.value("ACTION_TACKLE_COMMIT");
    let miss_recovery = tune.value("ACTION_TACKLE_MISS_RECOVERY");

    for i in 0..s.players.len() {
        let idx = (i + 1) as i64;
        let (verb, phase) = (s.players[i].action.verb, s.players[i].action.phase);
        if verb != Some(ActionVerb::Tackle) {
            continue;
        }
        match phase {
            ActionPhase::Charging => {
                let human = is_human_player(s, idx);
                let target = s.players[i].action.target_player;
                if let Some(t) = target
                    && !tackle_target_is_live(s, idx, t)
                {
                    // The situation this charge was chasing evaporated
                    // before it ever got a shot at it (the ball changed
                    // hands, went loose, or -- should a producer bug ever
                    // hand this a same-team or self target again --
                    // never was a legitimate target to begin with): abort
                    // rather than resolve against it.
                    s.players[i].action = action_slot::clear(&s.players[i].action);
                    continue;
                }
                let arrived = target.is_some_and(|t| {
                    let (d, reach) = tackle_distance_and_reach(s, idx, t, human);
                    d <= reach
                });
                s.players[i].action = action_slot::advance_charge(&s.players[i].action, dt);
                let held = s.players[i].action.charge_elapsed;
                let trigger = action_slot::evaluate_release(action_slot::ReleaseTriggerInputs {
                    input_released: false,
                    held_seconds: held,
                    full_charge,
                    arrived,
                    // Trigger 4 (AI danger threshold) is not exercised by
                    // this verb in this PR -- see `action_slot`'s module doc.
                    ai_danger: None,
                    ai_threshold: 0.0,
                });
                if trigger.is_some() {
                    s.players[i].action = action_slot::release(
                        &s.players[i].action,
                        full_charge,
                        // No graduated power for a standing poke in this PR
                        // (see this PR's description): every release lands
                        // at full power.
                        1.0,
                        commit_seconds,
                    );
                }
            }
            ActionPhase::Executing => {
                s.players[i].action = action_slot::advance_remaining(&s.players[i].action, dt);
                if action_slot::due(&s.players[i].action) {
                    resolve_tackle(s, idx, miss_recovery);
                }
            }
            ActionPhase::Recovering => {
                s.players[i].action = action_slot::advance_remaining(&s.players[i].action, dt);
                if action_slot::due(&s.players[i].action) {
                    s.players[i].action = action_slot::end_recovery(&s.players[i].action);
                }
            }
            ActionPhase::None => {}
        }
        // `tackle_timer` is presentation-facing (`gc_render::player_pose`
        // derives the existing poke pose from it): mirror the action
        // slot's own executing-phase countdown into it rather than run a
        // second, independently-drifting timer. Zero outside `Executing` --
        // charging and recovering get no pose of their own in this PR (see
        // this PR's description on render scope).
        s.players[i].tackle_timer = if s.players[i].action.phase == ActionPhase::Executing {
            s.players[i].action.remaining
        } else {
            0.0
        };
    }

    let Some(owner_idx) = s.owner else {
        return;
    };
    let owner = &s.players[(owner_idx - 1) as usize];
    if owner.is_keeper && !owner.feet_ball {
        return;
    }
    if owner.dodge_timer > 0.0 {
        return;
    }
    if s.ball_z > GROUND_GRAB_HEIGHT {
        return;
    }
    let owner_team = owner.team;
    for i in 0..s.players.len() {
        let idx = (i + 1) as i64;
        let blocked = combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), idx));
        let p = &s.players[i];
        if p.team == owner_team
            || p.is_keeper
            || p.stun_timer > 0.0
            || blocked
            || p.action.phase != ActionPhase::None
            || p.dash_cd > 0.0
            || is_human_player(s, idx)
        {
            continue;
        }
        if p.pos.dist(s.ball) > tune.value("STEAL_ATTEMPT") {
            continue;
        }
        let press = team_press(s, p.team);
        if press.presser_index != Some(idx as u32)
            || press.mode != outfield_press::StablePressMode::Commit
        {
            continue;
        }
        let reason = press.reason;
        assert!(
            reason != brain::PressReason::NoTrigger,
            "AI press commit requires a stable reason"
        );
        let (pid, ppos) = {
            let p = &s.players[i];
            (p.id.clone(), p.pos)
        };
        {
            let pm = &mut s.players[i];
            pm.dash_cd = tune.value("AI_STEAL_CD");
            pm.action =
                action_slot::commit_charge(&pm.action, ActionVerb::Tackle, Some(owner_idx), 0.0);
        }
        let commit_kind = press_reason_to_event_kind(reason);
        s.events.push(MatchEvent {
            kind: commit_kind,
            x: ppos.x,
            y: ppos.y,
            player: Some(pid),
            save_style: None,
            style: None,
            outcome: None,
            jumping: None,
            difficulty: None,
            shot_type: None,
            keeper_state: None,
            keeper_depth: None,
            on_target: None,
        });
    }
}

/// Knock the ball loose when a challenger reaches THE BALL — not the
/// carrier's body. The ball sticks a step ahead of the carrier's feet, so a
/// carrier who turns their body between the challenger and the ball
/// SHIELDS it: challenges from behind come up short. A stunned defender
/// can't tackle. The ball pops toward the challenger so a clean tackle
/// tends to win possession.
///
/// This function now handles only the KEEPER SMOTHER and the SLIDE tackle
/// (#489): both stay the instantaneous, continuously-checked lunges they
/// always were. The standing-poke tackle — human and AI alike — moved to
/// `advance_tackle_actions`'s committed-action slot, where a charge,
/// executing window, and a resolved miss's recovery cost all live; see that
/// function and `crate::action_slot`'s module doc.
fn attempt_steals(s: &mut MatchState, combat_state: Option<&CombatMatchState>) {
    let Some(owner_idx) = s.owner else {
        return;
    };
    let owner = &s.players[(owner_idx - 1) as usize];
    if owner.is_keeper && !owner.feet_ball {
        return; // a keeper has the ball in hand: it can't be tackled off them
    }
    if owner.dodge_timer > 0.0 {
        return; // juke i-frames: the carrier can't be tackled mid-dodge
    }
    if s.ball_z > GROUND_GRAB_HEIGHT {
        return; // ball is in the air, not at the carrier's feet (owned ball is grounded)
    }
    let owner_team = owner.team;
    let owner_verb = owner.owned_verb;

    // Keeper smother: a carrier who brings the ball into the keeper's box
    // gets it picked straight off their feet (into the keeper's hands, not
    // knocked loose). This is the 1v1 close-down; without it a carrier
    // could walk the ball in.
    for i in 0..s.players.len() {
        let idx = (i + 1) as i64;
        let p = &s.players[i];
        if p.is_keeper
            && p.team != owner_team
            && p.stun_timer <= 0.0
            && in_claim_zone(s, idx)
            && keeper::in_smother_range(p.pos.dist(s.ball))
        {
            let pid = p.id.clone();
            s.events.push(MatchEvent {
                kind: MatchEventKind::Claim,
                x: s.ball.x,
                y: s.ball.y,
                player: Some(pid),
                save_style: None,
                style: None,
                outcome: None,
                jumping: None,
                difficulty: None,
                shot_type: None,
                keeper_state: None,
                keeper_depth: None,
                on_target: None,
            });
            // Cancel any pending wind-up: the smother beats the shot.
            let owner_mut = &mut s.players[(owner_idx - 1) as usize];
            owner_mut.windup_timer = 0.0;
            owner_mut.windup_shot = None;
            set_owner(s, Some(idx));
            s.ball_vel = Vec2::new(0.0, 0.0);
            s.ball_spin = 0.0;
            let keeper_mut = &mut s.players[i];
            keeper_mut.grab_timer = KEEPER_GRAB_POSE;
            keeper_mut.hold_timer = KEEPER_HOLD;
            keeper_mut.feet_ball = false;
            if keeper_mut.dive_timer > 0.0 {
                end_dive(keeper_mut); // possession ends the dive (#450)
            }
            return;
        }
    }

    for i in 0..s.players.len() {
        let idx = (i + 1) as i64;
        let owner_pos = s.players[(owner_idx - 1) as usize].pos;
        let p = &s.players[i];
        let blocked = combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), idx));
        if p.team != owner_team
            && !p.is_keeper
            && p.stun_timer <= 0.0
            && !blocked
            && is_human_player(s, idx)
            && p.slide_timer > 0.0
        {
            let mut d = p.pos.dist(s.ball); // reach for the ball: shielding matters
            let species_reach =
                species::collision_reach(p.owned_verb) - species::dribble_protection(owner_verb);
            // The human's poke also works at body-contact range from ANY
            // angle (a toe through the legs): chasing a carrier is the
            // default defensive situation and must be winnable.
            if p.pos.dist(owner_pos) <= STEAL_DIST + species_reach {
                d = d.min(STEAL_DIST);
            }
            if d <= SLIDE_REACH + species_reach {
                win_ball(s, owner_idx, idx, true);
                return;
            }
        }
    }
}

fn press_reason_to_event_kind(reason: brain::PressReason) -> MatchEventKind {
    match reason {
        brain::PressReason::HeavyTouch => MatchEventKind::PressCommitHeavyTouch,
        brain::PressReason::ExposedBall => MatchEventKind::PressCommitExposedBall,
        brain::PressReason::Cover => MatchEventKind::PressCommitCover,
        brain::PressReason::BoxDesperation => MatchEventKind::PressCommitBoxDesperation,
        brain::PressReason::LowDiscipline => MatchEventKind::PressCommitLowDiscipline,
        brain::PressReason::NoTrigger => {
            unreachable!("AI press commit requires a stable reason")
        }
    }
}

fn combat_reason_to_event_kind(reason: combat_intent::CombatDecisionReason) -> MatchEventKind {
    use combat_intent::CombatDecisionReason as R;
    match reason {
        R::CarrierContest => MatchEventKind::CombatCommitCarrierContest,
        R::CarrierProtection => MatchEventKind::CombatCommitCarrierProtection,
        R::LooseBallContest => MatchEventKind::CombatCommitLooseBallContest,
        R::PassingLaneOrShotDenial => MatchEventKind::CombatCommitPassingLaneOrShotDenial,
        R::RecoveryPunish => MatchEventKind::CombatCommitRecoveryPunish,
        R::UnattributedOffBall => MatchEventKind::CombatCommitUnattributedOffBall,
        R::None | R::Decline => {
            unreachable!("AI combat commit requires a stable reason")
        }
    }
}

fn own_goal_center(s: &MatchState, team: Team) -> Vec2 {
    let g = if team == Team::Home {
        s.goal_home
    } else {
        s.goal_away
    };
    Vec2::new(g.x + g.w / 2.0, g.y + g.h / 2.0)
}

/// Slide an anchor toward the ball without losing shape (block shift).
fn block_shift(anchor: Vec2, ball: Vec2, compactness: f64) -> Vec2 {
    anchor.add(ball.sub(anchor).scale(BLOCK_SHIFT * compactness))
}

/// Existing attacking support fallback, shared by ordinary off-ball
/// planning and same-boundary run invalidation so a cancelled run never
/// parks on a raw formation anchor for the rest of its retained cadence.
fn support_target(
    s: &MatchState,
    player_index: i64,
    pos: &[Vec2],
    opponent_positions: Option<&[Vec2]>,
    push_scale: Option<f64>,
    tune: &Tuning,
) -> Vec2 {
    let player = &s.players[(player_index - 1) as usize];
    let owner_index = s.owner.expect("support target requires a carrier");
    let owner = &s.players[(owner_index - 1) as usize];
    assert!(
        owner.team == player.team && !owner.is_keeper,
        "support target requires open attack"
    );
    let attack = if player.team == Team::Home { 1.0 } else { -1.0 };
    let carrier_pos = pos[(owner_index - 1) as usize];
    let support = team_marking(s, player.team).support;
    let base = Vec2::new(
        player.anchor.x + attack * ATTACK_PUSH * support * push_scale.unwrap_or(1.0),
        player.anchor.y,
    );
    let mut candidates = vec![
        base,
        Vec2::new(base.x, base.y - SUPPORT_FAN),
        Vec2::new(base.x, base.y + SUPPORT_FAN),
        Vec2::new(base.x + attack * SUPPORT_FAN, base.y),
        Vec2::new(base.x - attack * SUPPORT_FAN, base.y),
    ];
    if pos[(player_index - 1) as usize].dist(carrier_pos) < TRIANGLE_JOIN {
        for (cos_angle, sin_angle) in SUPPORT_TRIANGLE_DIRECTIONS {
            let direction = Vec2::new(cos_angle * attack, sin_angle);
            let candidate = carrier_pos.add(direction.scale(tune.value("TRIANGLE_DIST")));
            candidates.push(Vec2::new(
                candidate.x.clamp(20.0, s.field.w - 20.0),
                candidate.y.clamp(20.0, s.field.h - 20.0),
            ));
        }
    }
    let ahead = carrier_pos.add(owner.facing.scale(120.0));
    let mut open_candidates: Vec<Vec2> = candidates
        .iter()
        .filter(|c| c.dist(ahead) > 70.0)
        .copied()
        .collect();
    if open_candidates.is_empty() {
        open_candidates = candidates;
    }
    let mut opponents: Vec<Vec2> = opponent_positions.map(<[Vec2]>::to_vec).unwrap_or_default();
    let mut teammates: Vec<Vec2> = Vec::new();
    for (index, other) in s.players.iter().enumerate() {
        let oidx = (index + 1) as i64;
        if opponent_positions.is_none() && other.team != player.team {
            opponents.push(pos[index]);
        } else if other.team == player.team
            && oidx != player_index
            && oidx != owner_index
            && !other.is_keeper
            && !is_human_player(s, oidx)
        {
            teammates.push(pos[index]);
        }
    }
    let target = ai::support_spot(
        carrier_pos,
        &open_candidates,
        &opponents,
        attack,
        ai::Field {
            w: s.field.w,
            h: s.field.h,
        },
    );
    target.add(ai::separation(target, &teammates, SEP_RADIUS).scale(SEP_PUSH))
}

/// A marker stands just goal-side of its man, leading the man's motion.
fn marker_target(defpos: Vec2, opp_pos: Vec2, opp_vel: Vec2, goal: Vec2, off: Option<f64>) -> Vec2 {
    let aim = ai::pursue(defpos, opp_pos, opp_vel, PURSUE_LEAD);
    aim.add(
        goal.sub(aim)
            .normalized()
            .scale(off.unwrap_or(MARK_GOALSIDE)),
    )
}

/// The team's current possession phase. `brain::phase` decays a live
/// turnover into ordinary attack/defend/loose once the team's tactic window
/// elapses, so this is the single place a counterpress or counterattack
/// phase is decided.
fn team_phase(s: &MatchState, team: Team) -> brain::TeamPhase {
    let owner_team = s.owner.map(|o| s.players[(o - 1) as usize].team);
    possession_transition::phase(
        &s.transition,
        to_transition_team(team),
        owner_team.map(to_transition_team),
        team_transition_windows(s, team),
    )
}

fn outfield_ordinal(s: &MatchState, team: Team, player_index: i64) -> i64 {
    let mut ordinal = 0i64;
    for (index, player) in s.players.iter().enumerate() {
        if player.team == team && !player.is_keeper {
            ordinal += 1;
            if (index + 1) as i64 == player_index {
                return ordinal;
            }
        }
    }
    unreachable!("player is not a team outfielder");
}

fn formation_role(s: &MatchState, team: Team, player_index: i64) -> FormationRole {
    let formation = match team {
        Team::Home => s.formation.home.as_str(),
        Team::Away => s.formation.away.as_str(),
    };
    let ordinal = outfield_ordinal(s, team, player_index);
    offball_runs::formation_role(formation, (ordinal - 1) as usize)
}

fn press_eligible(
    s: &MatchState,
    player_index: i64,
    combat_state: Option<&CombatMatchState>,
) -> bool {
    let player = &s.players[(player_index - 1) as usize];
    !player.is_keeper
        && !is_human_player(s, player_index)
        && player.stun_timer <= 0.0
        && player.aerial_recovery <= 0.0
        && !combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), player_index))
}

fn run_type_to_intent(run_type: brain::RunType) -> crate::outfield_decision::OutfieldIntent {
    use crate::outfield_decision::OutfieldIntent as I;
    match run_type {
        brain::RunType::InBehind => I::InBehind,
        brain::RunType::ComeShort => I::ComeShort,
        brain::RunType::HoldWidth => I::HoldWidth,
    }
}

/// Reconcile active run slots against this tick's role-gated offers and
/// commit new/retained runs into `targets`/`urgent`.
#[allow(clippy::too_many_arguments)]
fn assign_runs(
    s: &mut MatchState,
    team: Team,
    candidates: &[i64],
    targets: &mut IndexMap<i64, Vec2>,
    pos: &[Vec2],
    combat_state: Option<&CombatMatchState>,
    urgent: &mut IndexMap<i64, bool>,
    counterattack: bool,
    tune: &Tuning,
) {
    let now = -s.time_left;
    let mut active: Vec<brain::RunSlot> = Vec::new();
    let mut active_players: Vec<i64> = Vec::new();
    let mut ended_players: Vec<i64> = Vec::new();
    for &player_index in candidates {
        let decision = s.players[(player_index - 1) as usize].outfield_decision;
        if outfield_decision::is_run_intent(decision.intent) {
            let expires_at = decision.run_expires_at.expect("run intent has expiry");
            let target = Vec2::new(
                decision.target_x.expect("run intent has target x"),
                decision.target_y.expect("run intent has target y"),
            );
            let ppos = s.players[(player_index - 1) as usize].pos;
            if expires_at > now
                && ppos.dist(target) > STAND_DEADBAND
                && run_eligible(s, player_index, combat_state)
            {
                active.push(brain::RunSlot {
                    player_index: player_index as u32,
                    run_type: match decision.intent {
                        crate::outfield_decision::OutfieldIntent::InBehind => {
                            brain::RunType::InBehind
                        }
                        crate::outfield_decision::OutfieldIntent::ComeShort => {
                            brain::RunType::ComeShort
                        }
                        crate::outfield_decision::OutfieldIntent::HoldWidth => {
                            brain::RunType::HoldWidth
                        }
                        _ => unreachable!("is_run_intent guarantees a run intent"),
                    },
                    score: 0.0,
                    target_x: target.x,
                    target_y: target.y,
                    granted_at: expires_at - offball_runs::RUN_LIFETIME_SECONDS,
                    expires_at,
                });
                active_players.push(player_index);
            } else {
                let fallback = *targets.get(&player_index).expect("target exists");
                let d = &mut s.players[(player_index - 1) as usize].outfield_decision;
                *d = outfield_decision::cancel_run(d, fallback.x, fallback.y);
                ended_players.push(player_index);
            }
        }
    }

    let mut needs_candidates = false;
    if (active.len() as u32) < offball_runs::MAX_ACTIVE_PER_TEAM {
        for &player_index in candidates {
            if !active_players.contains(&player_index)
                && !ended_players.contains(&player_index)
                && run_eligible(s, player_index, combat_state)
                && outfield_decision::should_refresh(
                    &s.players[(player_index - 1) as usize].outfield_decision,
                    OutfieldDecisionContext::Offball,
                    None,
                )
            {
                needs_candidates = true;
                break;
            }
        }
    }
    let mut slots = active;
    if needs_candidates {
        let owner_index = s.owner.expect("assign_runs requires a carrier");
        let owner_settled = s.players[(owner_index - 1) as usize].settle_timer <= 0.0;
        let mut pressure = s.field.w;
        let mut opponents: Vec<offball_runs::OffballRunOpponent> = Vec::new();
        let mut teammates: Vec<offball_runs::OffballRunTeammate> = Vec::new();
        let mut players: Vec<offball_runs::OffballRunPlayer> = Vec::new();
        for (index, player) in s.players.iter().enumerate() {
            let idx = (index + 1) as i64;
            if player.team != team {
                if !player.is_keeper {
                    pressure = pressure.min(pos[index].dist(pos[(owner_index - 1) as usize]));
                }
                opponents.push(offball_runs::OffballRunOpponent {
                    pos: pos[index],
                    is_keeper: player.is_keeper,
                });
            } else if !player.is_keeper && idx != owner_index {
                teammates.push(offball_runs::OffballRunTeammate {
                    player_index: idx as u32,
                    pos: pos[index],
                });
            }
        }
        for &player_index in candidates {
            let player = &s.players[(player_index - 1) as usize];
            if !active_players.contains(&player_index)
                && !ended_players.contains(&player_index)
                && run_eligible(s, player_index, combat_state)
                && outfield_decision::should_refresh(
                    &player.outfield_decision,
                    OutfieldDecisionContext::Offball,
                    None,
                )
            {
                let ordinal = outfield_ordinal(s, team, player_index);
                let formation = match team {
                    Team::Home => s.formation.home.as_str(),
                    Team::Away => s.formation.away.as_str(),
                };
                players.push(offball_runs::OffballRunPlayer {
                    player_index: player_index as u32,
                    role: offball_runs::formation_role(formation, (ordinal - 1) as usize),
                    run_drive: stats::run_drive_from_match(player.move_speed, player.composure),
                    pos: pos[(player_index - 1) as usize],
                    anchor_y: offball_runs::formation_anchor_y(formation, (ordinal - 1) as usize),
                });
            }
        }
        slots = offball_runs::grant(
            &offball_runs::OffballRunContext {
                team: to_offball_team(team),
                field: offball_runs::Field {
                    w: s.field.w,
                    h: s.field.h,
                },
                carrier_pos: pos[(owner_index - 1) as usize],
                carrier_settled: owner_settled,
                carrier_pressure: pressure,
                pressure_distance: tune.value("AI_PASS_PRESSURE"),
                counterattack,
                players,
                teammates,
                opponents,
            },
            &slots,
            now,
        );
    }
    for slot in &slots {
        let player_index = i64::from(slot.player_index);
        let decision = s.players[(player_index - 1) as usize].outfield_decision;
        let retained = outfield_decision::is_run_intent(decision.intent)
            && decision.intent == run_type_to_intent(slot.run_type)
            && decision.run_expires_at == Some(slot.expires_at);
        if !retained || decision.remaining <= 0.0 {
            let scan_rate = s.players[(player_index - 1) as usize].scan_rate;
            let d = &mut s.players[(player_index - 1) as usize].outfield_decision;
            *d = outfield_decision::refresh(
                d,
                OutfieldDecisionContext::Offball,
                run_type_to_intent(slot.run_type),
                scan_rate,
                Some(slot.target_x),
                Some(slot.target_y),
                None,
                Some(slot.expires_at),
            );
        }
        targets.insert(player_index, Vec2::new(slot.target_x, slot.target_y));
        urgent.insert(player_index, true);
    }
}

fn sanitize_run_states(s: &mut MatchState, combat_state: Option<&CombatMatchState>, tune: &Tuning) {
    let owner = s.owner.map(|o| s.players[(o - 1) as usize].team);
    let owner_is_keeper = s
        .owner
        .is_some_and(|o| s.players[(o - 1) as usize].is_keeper);
    let now = -s.time_left;
    for index in 0..s.players.len() {
        let idx = (index + 1) as i64;
        let decision = s.players[index].outfield_decision;
        if outfield_decision::is_run_intent(decision.intent) {
            let player_team = s.players[index].team;
            let ordinary_attack = owner.is_some_and(|ot| ot == player_team)
                && !owner_is_keeper
                && s.kickoff_hold <= 0.0;
            let target = Vec2::new(
                decision.target_x.expect("run intent has target x"),
                decision.target_y.expect("run intent has target y"),
            );
            let ppos = s.players[index].pos;
            let human = is_human_player(s, idx);
            if human
                || !ordinary_attack
                || !run_eligible(s, idx, combat_state)
                || decision.run_expires_at.expect("run intent has expiry") <= now
                || ppos.dist(target) <= STAND_DEADBAND
            {
                if human {
                    let d = &mut s.players[index].outfield_decision;
                    *d = outfield_decision::reset(d);
                } else {
                    let receive_timer = s.players[index].receive_timer;
                    let fallback = if receive_timer > 0.0 {
                        s.ball
                    } else if ordinary_attack {
                        let mut pos: Vec<Vec2> = s.players.iter().map(|p| p.pos).collect();
                        pos[index] = s.players[index].pos;
                        support_target(s, idx, &pos, None, None, tune)
                    } else {
                        let anchor = s.players[index].anchor;
                        let compactness = team_marking(s, player_team).compactness;
                        block_shift(anchor, s.ball, compactness)
                    };
                    let d = &mut s.players[index].outfield_decision;
                    *d = outfield_decision::cancel_run(d, fallback.x, fallback.y);
                }
            }
        }
    }
}

fn passing_lane_candidates(
    s: &MatchState,
    team: Team,
    carrier_index: i64,
    pos: &[Vec2],
) -> Vec<outfield_press::OutfieldLaneCandidate> {
    let carrier = &s.players[(carrier_index - 1) as usize];
    let carrier_team = carrier.team;
    let goal = own_goal_center(s, team);
    let mut candidates = Vec::new();
    let diagonal = (s.field.w * s.field.w + s.field.h * s.field.h).sqrt();
    let mut opponents: Vec<Vec2> = Vec::new();
    for (index, player) in s.players.iter().enumerate() {
        if player.team != carrier_team {
            opponents.push(pos[index]);
        }
    }
    for (index, player) in s.players.iter().enumerate() {
        let idx = (index + 1) as i64;
        if idx != carrier_index && player.team == carrier_team && !player.is_keeper {
            let distance = pos[(carrier_index - 1) as usize].dist(pos[index]);
            let mut openness = f64::INFINITY;
            for opponent_pos in &opponents {
                openness = openness.min(opponent_pos.dist(pos[index]));
            }
            if ai_pass_eligible(distance, openness) {
                // Rank only actual carrier pass options. Threat leads, while
                // openness and range break out otherwise similar lanes.
                let goal_threat = diagonal - pos[index].dist(goal);
                candidates.push(outfield_press::OutfieldLaneCandidate {
                    player_index: idx as u32,
                    score: goal_threat + openness * 0.35 - distance * 0.25,
                    pos: pos[index],
                    eligible: None,
                });
            }
        }
    }
    candidates
}

fn lane_shadow_target(
    s: &MatchState,
    team: Team,
    carrier_index: i64,
    base: Vec2,
    pos: &[Vec2],
    hold: bool,
) -> Vec2 {
    let candidates = passing_lane_candidates(s, team, carrier_index, pos);
    if hold {
        outfield_press::lane_hold_target(base, pos[(carrier_index - 1) as usize], &candidates)
    } else {
        outfield_press::lane_shadow_target(base, pos[(carrier_index - 1) as usize], &candidates)
    }
}

/// Returns (presser, cover), both 1-based player indices.
#[allow(clippy::too_many_arguments)]
fn assign_press(
    s: &mut MatchState,
    team: Team,
    candidates: &[i64],
    carrier_index: i64,
    goal: Vec2,
    ball: Vec2,
    pos: &[Vec2],
    combat_state: Option<&CombatMatchState>,
    tune: &Tuning,
) -> (Option<i64>, Option<i64>) {
    let carrier_settle = s.players[(carrier_index - 1) as usize].settle_timer;
    let carrier_pos = pos[(carrier_index - 1) as usize];
    let mut ranked: Vec<outfield_press::OutfieldPressCandidate> = Vec::new();
    for &player_index in candidates {
        ranked.push(outfield_press::OutfieldPressCandidate {
            player_index: player_index as u32,
            distance_cost: pos[(player_index - 1) as usize].dist(carrier_pos),
            eligible: Some(press_eligible(s, player_index, combat_state)),
        });
    }
    let current = team_press(s, team).presser_index;
    let presser = outfield_press::assign_presser(&ranked, current);
    let Some(presser) = presser else {
        set_team_press(s, team, outfield_press::clear(&team_press(s, team)));
        return (None, None);
    };
    let presser_idx = i64::from(presser);

    let mut cover: Option<i64> = None;
    let mut cover_distance: Option<f64> = None;
    for &player_index in candidates {
        if player_index != presser_idx && press_eligible(s, player_index, combat_state) {
            let distance = pos[(player_index - 1) as usize].dist(carrier_pos);
            if cover_distance.is_none_or(|cd| distance < cd)
                || (Some(distance) == cover_distance
                    && player_index < cover.expect("cover_distance implies cover"))
            {
                cover = Some(player_index);
                cover_distance = Some(distance);
            }
        }
    }

    let mut cover_available = false;
    if let Some(cover_idx) = cover {
        cover_available = outfield_press::cover_available(
            cover_distance.expect("cover implies cover_distance"),
            pos[(cover_idx - 1) as usize].dist(goal) < carrier_pos.dist(goal),
        );
    }
    let box_depth = if team == Team::Home {
        carrier_pos.x
    } else {
        s.field.w - carrier_pos.x
    };
    let box_top = s.field.h / 2.0 - PENALTY_H / 2.0;
    let box_desperation = box_depth <= PENALTY_DEPTH
        && carrier_pos.y >= box_top
        && carrier_pos.y <= box_top + PENALTY_H;
    let reachable = s.players[(presser_idx - 1) as usize].dash_cd <= 0.0
        && pos[(presser_idx - 1) as usize].dist(ball) <= tune.value("STEAL_ATTEMPT");
    if reachable {
        let press_discipline = s.players[(presser_idx - 1) as usize].composure;
        let next = outfield_press::resolve(
            presser,
            &outfield_press::OutfieldPressContext {
                heavy_touch: carrier_settle > 0.0,
                exposed_ball: carrier_pos.dist(ball) > DRIBBLE_TOUCH_REACH,
                cover_available,
                box_desperation,
                press_discipline,
            },
        );
        set_team_press(s, team, next);
    } else {
        set_team_press(s, team, outfield_press::contain(presser));
    }
    (Some(presser_idx), cover)
}

fn sanitize_press_states(s: &mut MatchState, combat_state: Option<&CombatMatchState>) {
    let owner = s.owner.map(|o| (o, s.players[(o - 1) as usize].team));
    for team in [Team::Home, Team::Away] {
        let state = team_press(s, team);
        let presser = state.presser_index;
        let defending = owner.is_some_and(|(oi, ot)| {
            ot != team
                && s.kickoff_hold <= 0.0
                && (!s.players[(oi - 1) as usize].is_keeper
                    || s.players[(oi - 1) as usize].feet_ball)
        });
        if !defending || presser.is_some_and(|p| !press_eligible(s, i64::from(p), combat_state)) {
            set_team_press(s, team, outfield_press::clear(&state));
        }
    }
}

/// Compute off-ball steering targets for every AI player NOT handled by the
/// controlled / owner / keeper branches. Pure function of the top-of-tick
/// snapshot `pos`. Returns player_index -> target Vec2 plus an URGENCY set
/// (roles needing full-speed precision, exempt from positional calm), and
/// refreshes `s.marks` for man-marking hysteresis.
#[allow(clippy::too_many_lines)]
/// Exposed for the integration tests in `tests/match.rs`, per ARCHITECTURE.md
/// §3 rule 6 ("everything a test touches is `pub`"): crates here are internal, so
/// visibility is not worth fighting to keep a spec case unportable.
pub fn offball_targets(
    s: &mut MatchState,
    pos: &[Vec2],
    combat_state: Option<&CombatMatchState>,
    tune: &Tuning,
) -> (
    IndexMap<i64, Vec2>,
    IndexMap<i64, bool>,
    IndexMap<i64, bool>,
) {
    let mut targets: IndexMap<i64, Vec2> = IndexMap::new();
    let mut urgent: IndexMap<i64, bool> = IndexMap::new();
    let mut closing: IndexMap<i64, bool> = IndexMap::new();
    let owner_team = s.owner.map(|o| s.players[(o - 1) as usize].team);

    for team in [Team::Home, Team::Away] {
        let cfg = team_marking(s, team);
        let goal = own_goal_center(s, team);
        let phase = team_phase(s, team);

        // This team's off-ball outfielders (exclude keeper, ball-owner, human).
        let mut mine: Vec<i64> = Vec::new();
        for (i, p) in s.players.iter().enumerate() {
            let idx = (i + 1) as i64;
            if p.team == team && !p.is_keeper && Some(idx) != s.owner && !is_human_player(s, idx) {
                mine.push(idx);
            }
        }
        // Opponents: all (for openness/lanes) and outfield-only (for marking).
        let mut opp_all_pos: Vec<Vec2> = Vec::new();
        let mut opp_out: Vec<i64> = Vec::new();
        let mut opp_out_pos: Vec<Vec2> = Vec::new();
        for (i, p) in s.players.iter().enumerate() {
            let idx = (i + 1) as i64;
            if p.team != team {
                opp_all_pos.push(pos[i]);
                if !p.is_keeper {
                    opp_out.push(idx);
                    opp_out_pos.push(pos[i]);
                }
            }
        }

        // Teammate positions for separation (spread out, don't stack).
        let sep = |idx: i64, target: Vec2, mine: &[i64]| -> Vec2 {
            let mut others = Vec::new();
            for &j in mine {
                if j != idx {
                    others.push(pos[(j - 1) as usize]);
                }
            }
            target.add(ai::separation(target, &others, SEP_RADIUS).scale(SEP_PUSH))
        };

        if let Some(ot) = owner_team
            && ot != team
        {
            // DEFENDING: retain one eligible team-owned presser unless a
            // challenger clears the explicit 15% distance-cost threshold.
            let carrier_index = s.owner.expect("owner_team implies owner");
            let carrier_team = s.players[(carrier_index - 1) as usize].team;
            let carrier_is_keeper = s.players[(carrier_index - 1) as usize].is_keeper;
            let carrier_feet_ball = s.players[(carrier_index - 1) as usize].feet_ball;
            let carrier_vel = s.players[(carrier_index - 1) as usize].vel;
            let cpos = pos[(carrier_index - 1) as usize];
            debug_assert!(carrier_team != team);

            // Kickoff law and keeper-hand protection both suspend the
            // ordinary defended-possession phase instead of pre-assigning a
            // challenger.
            let held = s.kickoff_hold > 0.0 || (carrier_is_keeper && !carrier_feet_ball);
            let mut presser: Option<i64> = None;
            let mut cover: Option<i64> = None;
            if held {
                set_team_press(s, team, outfield_press::clear(&team_press(s, team)));
            } else {
                let (p, c) = assign_press(
                    s,
                    team,
                    &mine,
                    carrier_index,
                    goal,
                    s.ball,
                    pos,
                    combat_state,
                    tune,
                );
                presser = p;
                cover = c;
            }
            // COUNTER-PRESS: the losing team gets two hunters instead of one
            // presser plus a standing-off cover, and neither of them contains.
            let counterpress = phase == brain::TeamPhase::Counterpress && !held;
            if let Some(presser) = presser {
                let press_state = team_press(s, team);
                if counterpress || press_state.mode == outfield_press::StablePressMode::Commit {
                    targets.insert(
                        presser,
                        ai::pursue(
                            pos[(presser - 1) as usize],
                            s.ball,
                            carrier_vel,
                            PURSUE_LEAD,
                        ),
                    );
                } else {
                    targets.insert(
                        presser,
                        outfield_press::contain_target(cpos, goal, s.field.h, cfg.standoff),
                    );
                }
                urgent.insert(presser, true);
                if counterpress {
                    closing.insert(presser, true);
                }
            }
            if let Some(cover) = cover {
                if counterpress {
                    targets.insert(
                        cover,
                        ai::pursue(pos[(cover - 1) as usize], s.ball, carrier_vel, PURSUE_LEAD),
                    );
                    urgent.insert(cover, true);
                    closing.insert(cover, true);
                } else {
                    let base = ai::interpose(cpos, goal, COVER_FRAC);
                    targets.insert(
                        cover,
                        lane_shadow_target(s, team, carrier_index, base, pos, false),
                    );
                }
            }

            // The rest: which defenders should man-mark, which hold zone.
            let rest: Vec<i64> = mine
                .iter()
                .copied()
                .filter(|&idx| Some(idx) != presser && Some(idx) != cover)
                .collect();

            if counterpress {
                // Everyone else holds the position the turnover left them in
                // and shades the highest-valued outlet lane instead of
                // retreating to a formation anchor. Defensive roles are the
                // exception: they recover first when their team loses the
                // ball.
                for &idx in &rest {
                    if formation_role(s, team, idx) == FormationRole::Def {
                        let anchor = s.players[(idx - 1) as usize].anchor;
                        targets.insert(idx, block_shift(anchor, s.ball, cfg.compactness));
                    } else {
                        targets.insert(
                            idx,
                            lane_shadow_target(
                                s,
                                team,
                                carrier_index,
                                pos[(idx - 1) as usize],
                                pos,
                                true,
                            ),
                        );
                    }
                }
                set_team_marks(s, team, Vec::new());
            } else {
                // Pick the opponents to be man-marked (0-based indices into opp_out).
                let mut mark_locals: Vec<usize> = Vec::new();
                match cfg.scheme {
                    gc_data::tactics::MarkingScheme::Man => {
                        mark_locals.extend(0..opp_out.len());
                    }
                    gc_data::tactics::MarkingScheme::Hybrid => {
                        let mut rank: Vec<usize> = (0..opp_out.len()).collect();
                        rank.sort_by(|&a, &b| {
                            let da = opp_out_pos[a].dist(goal);
                            let db = opp_out_pos[b].dist(goal);
                            if da != db {
                                da.partial_cmp(&db).expect("distances are comparable")
                            } else {
                                opp_out[a].cmp(&opp_out[b])
                            }
                        });
                        let take = (cfg.man_marks.max(0) as usize).min(rank.len());
                        mark_locals.extend(rank.into_iter().take(take));
                    }
                    gc_data::tactics::MarkingScheme::Zonal => {}
                }

                let mut newmarks: Vec<Option<i64>> = vec![None; rest.len()];
                if !mark_locals.is_empty() && !rest.is_empty() {
                    let restpos: Vec<Vec2> =
                        rest.iter().map(|&idx| pos[(idx - 1) as usize]).collect();
                    let markpos: Vec<Vec2> =
                        mark_locals.iter().map(|&li| opp_out_pos[li]).collect();
                    // Build prev assignment in local indices for hysteresis.
                    let prev_marks = team_marks(s, team);
                    let mut prev_local: IndexMap<usize, usize> = IndexMap::new();
                    for (di, &pidx) in rest.iter().enumerate() {
                        if let Some(Some(prev_opp)) = prev_marks.get((pidx - 1) as usize) {
                            for (mi, &li) in mark_locals.iter().enumerate() {
                                if opp_out[li] == *prev_opp {
                                    prev_local.insert(di, mi);
                                }
                            }
                        }
                    }
                    let map =
                        ai::assign_marks(&restpos, &markpos, Some(&prev_local), Some(MARK_STICK));
                    for (&di, &mi) in &map {
                        let def_idx = rest[di];
                        let opp_idx = opp_out[mark_locals[mi]];
                        newmarks[di] = Some(opp_idx);
                        // Tight on a live carrier's teammates; lane distance
                        // while the keeper surveys, so throws can actually be
                        // received.
                        let off = if carrier_is_keeper {
                            MARK_LANE_OFF
                        } else {
                            MARK_GOALSIDE
                        };
                        targets.insert(
                            def_idx,
                            marker_target(
                                pos[(def_idx - 1) as usize],
                                pos[(opp_idx - 1) as usize],
                                s.players[(opp_idx - 1) as usize].vel,
                                goal,
                                Some(off),
                            ),
                        );
                    }
                }
                // newmarks is dense over `rest`'s positions; store into the
                // team's marks array indexed by (player_index - 1), matching
                // match_snapshot's `Vec<Option<i64>>` layout.
                let mut marks_by_index: Vec<Option<i64>> = vec![None; s.players.len()];
                for (di, &pidx) in rest.iter().enumerate() {
                    marks_by_index[(pidx - 1) as usize] = newmarks[di];
                }
                set_team_marks(s, team, marks_by_index);

                // Any defender without a mark holds a ball-shifted zone.
                for &idx in &rest {
                    if !targets.contains_key(&idx) {
                        let anchor = s.players[(idx - 1) as usize].anchor;
                        targets.insert(idx, block_shift(anchor, s.ball, cfg.compactness));
                    }
                }
            }
            for &idx in &rest {
                let t = *targets.get(&idx).expect("rest player has a target");
                targets.insert(idx, sep(idx, t, &mine));
            }
        } else if owner_team == Some(team) {
            set_team_press(s, team, outfield_press::clear(&team_press(s, team)));
            // ATTACKING off the ball. When OUR KEEPER has it we're in
            // build-up: hold stable, spread outlet positions (don't roam) so
            // the keeper's throw reaches a teammate who's actually there.
            // Otherwise make support runs.
            let owner_idx = s.owner.expect("owner_team implies owner");
            let build_up = s.players[(owner_idx - 1) as usize].is_keeper;
            // COUNTER-ATTACK: the team that just won the ball pushes its
            // support depth by formation role (forwards hardest) and buys
            // one immediate in-behind request. Keeper build-up and the
            // kickoff hold are laws, not phases, so they still take
            // precedence.
            let counterattack =
                phase == brain::TeamPhase::Counterattack && !build_up && s.kickoff_hold <= 0.0;
            for &idx in &mine {
                if build_up {
                    let anchor = s.players[(idx - 1) as usize].anchor;
                    let t = block_shift(anchor, s.ball, 0.15);
                    targets.insert(idx, sep(idx, t, &mine));
                } else {
                    let push = if counterattack {
                        Some(possession_transition::support_push(formation_role(
                            s, team, idx,
                        )))
                    } else {
                        None
                    };
                    targets.insert(
                        idx,
                        support_target(s, idx, pos, Some(&opp_all_pos), push, tune),
                    );
                }
            }
            if !build_up && s.kickoff_hold <= 0.0 {
                assign_runs(
                    s,
                    team,
                    &mine,
                    &mut targets,
                    pos,
                    combat_state,
                    &mut urgent,
                    counterattack,
                    tune,
                );
            }
            set_team_marks(s, team, Vec::new());
        } else {
            set_team_press(s, team, outfield_press::clear(&team_press(s, team)));
            // LOOSE ball: the press-set chases it with a pursuit lead,
            // cutting off a rolling ball instead of trailing it. Passers
            // already price this in: pass safety is interception-aware
            // (ai::pass_intercept), not just a static lane check. Everyone
            // else holds shape. Inside a counter-press window the
            // assignment is exactly the two nearest eligible hunters
            // instead: the rest hold their turnover position (defensive
            // roles recover) rather than joining the chase.
            let counterpress = phase == brain::TeamPhase::Counterpress && s.kickoff_hold <= 0.0;
            let mut hunters: Option<Vec<i64>> = None;
            if counterpress {
                let mut ranked: Vec<possession_transition::TransitionPresserCandidate> = Vec::new();
                for &idx in &mine {
                    ranked.push(possession_transition::TransitionPresserCandidate {
                        player_index: idx as u32,
                        distance_cost: pos[(idx - 1) as usize].dist(s.ball),
                        eligible: Some(press_eligible(s, idx, combat_state)),
                    });
                }
                let chosen = possession_transition::select_pressers(
                    &ranked,
                    possession_transition::MAX_PRESSERS,
                );
                hunters = Some(chosen.into_iter().map(i64::from).collect());
            }
            let chasers: Vec<i64> =
                hunters.unwrap_or_else(|| nearest_n(s, team, team_press_count(s, team)));
            for &idx in &mine {
                // The press-set chases — and so does ANYONE the ball lands
                // near: a ball at your feet is yours to claim, whatever your
                // assigned role (the ball magnet).
                if chasers.contains(&idx)
                    || (!counterpress
                        && pos[(idx - 1) as usize].dist(s.ball) < tune.value("LOOSE_MAGNET"))
                {
                    targets.insert(
                        idx,
                        ai::pursue(pos[(idx - 1) as usize], s.ball, s.ball_vel, PURSUE_LEAD),
                    );
                    urgent.insert(idx, true);
                    if counterpress {
                        closing.insert(idx, true);
                    }
                } else if counterpress && formation_role(s, team, idx) != FormationRole::Def {
                    let t = pos[(idx - 1) as usize];
                    targets.insert(idx, sep(idx, t, &mine));
                } else {
                    let anchor = s.players[(idx - 1) as usize].anchor;
                    targets.insert(idx, block_shift(anchor, s.ball, cfg.compactness));
                }
            }
            set_team_marks(s, team, Vec::new());
        }
    }

    // A designated receiver runs onto the incoming ball (overrides its
    // other role) so a keeper's distribution is actually met and gathered,
    // not left in space.
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.receive_timer > 0.0 && targets.contains_key(&idx) {
            targets.insert(idx, Vec2::new(s.ball.x, s.ball.y));
            urgent.insert(idx, true);
        }
    }

    // Hard retreat: you can't challenge a keeper holding the ball, so the
    // opposing team must give it space — push any target inside the respect
    // ring back out. A keeper playing a back-pass with the FEET gets no
    // such protection.
    if let Some(oi) = s.owner
        && s.players[(oi - 1) as usize].is_keeper
        && !s.players[(oi - 1) as usize].feet_ball
    {
        let kpos = s.players[(oi - 1) as usize].pos;
        let kteam = s.players[(oi - 1) as usize].team;
        let keys: Vec<i64> = targets.keys().copied().collect();
        for i in keys {
            if s.players[(i - 1) as usize].team != kteam {
                let tgt = *targets.get(&i).expect("key exists");
                let off = tgt.sub(kpos);
                let d = off.length();
                if d < tune.value("KEEPER_RESPECT_DIST") {
                    let dir = if d > 0.0 {
                        off.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    targets.insert(i, kpos.add(dir.scale(tune.value("KEEPER_RESPECT_DIST"))));
                }
            }
        }
    }

    (targets, urgent, closing)
}

fn team_press_count(s: &MatchState, team: Team) -> i64 {
    match team {
        Team::Home => s.press.home,
        Team::Away => s.press.away,
    }
}

/// Retain an AI outfielder's chosen movement point until its personal
/// cadence expires. A designated receiver and anyone close enough to
/// contest a loose ball within one slow cadence interval keep reacting to
/// its live trajectory. `urgent` also preserves the existing active
/// presser/press-set chase contract; choosing or explaining those defenders
/// remains a separate concern from this cadence.
fn retain_offball_targets(
    s: &mut MatchState,
    targets: &mut IndexMap<i64, Vec2>,
    urgent: &mut IndexMap<i64, bool>,
    combat_state: Option<&CombatMatchState>,
    tune: &Tuning,
) {
    for index in 0..s.players.len() {
        let idx = (index + 1) as i64;
        let blocked = combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), idx));
        let player = &s.players[index];
        let mut running = outfield_decision::is_run_intent(player.outfield_decision.intent);
        let ineligible =
            player.is_keeper || is_human_player(s, idx) || blocked || player.stun_timer > 0.0;
        if ineligible {
            if player.outfield_decision.context != OutfieldDecisionContext::Ineligible {
                let d = &mut s.players[index].outfield_decision;
                *d = outfield_decision::reset(d);
            }
        } else if Some(idx) != s.owner && targets.contains_key(&idx) {
            let owner = s.owner.map(|o| s.players[(o - 1) as usize].team);
            let owner_is_keeper = s
                .owner
                .is_some_and(|o| s.players[(o - 1) as usize].is_keeper);
            let ordinary_attack = owner.is_some_and(|ot| ot == player.team)
                && !owner_is_keeper
                && s.kickoff_hold <= 0.0;
            if running {
                let decision = s.players[index].outfield_decision;
                let ppos = s.players[index].pos;
                let expired =
                    decision.run_expires_at.expect("run intent has expiry") <= -s.time_left;
                let close = ppos.dist(Vec2::new(
                    decision.target_x.expect("run intent has target x"),
                    decision.target_y.expect("run intent has target y"),
                )) <= STAND_DEADBAND;
                if !ordinary_attack || !run_eligible(s, idx, combat_state) || expired || close {
                    let fallback = *targets.get(&idx).expect("target exists");
                    let d = &mut s.players[index].outfield_decision;
                    *d = outfield_decision::cancel_run(d, fallback.x, fallback.y);
                    running = false;
                }
            }
            if running {
                let decision = s.players[index].outfield_decision;
                let t = Vec2::new(
                    decision.target_x.expect("run intent has target x"),
                    decision.target_y.expect("run intent has target y"),
                );
                targets.insert(idx, t);
                urgent.insert(idx, true);
            } else {
                let decisions_slow = outfield_decision::SLOW_REFRESH_SECONDS;
                let contest_reach =
                    tune.value("LOOSE_MAGNET") + s.players[index].move_speed * decisions_slow;
                let loose_contest =
                    s.owner.is_none() && s.players[index].pos.dist(s.ball) <= contest_reach;
                let must_refresh = urgent.get(&idx).copied().unwrap_or(false)
                    || s.players[index].receive_timer > 0.0
                    || loose_contest;
                if outfield_decision::should_refresh(
                    &s.players[index].outfield_decision,
                    OutfieldDecisionContext::Offball,
                    Some(must_refresh),
                ) {
                    let target = *targets.get(&idx).expect("target exists");
                    let scan_rate = s.players[index].scan_rate;
                    let d = &mut s.players[index].outfield_decision;
                    *d = outfield_decision::refresh(
                        d,
                        OutfieldDecisionContext::Offball,
                        outfield_decision::OutfieldIntent::Move,
                        scan_rate,
                        Some(target.x),
                        Some(target.y),
                        None,
                        None,
                    );
                } else {
                    let decision = s.players[index].outfield_decision;
                    let target_x = decision.target_x.expect("refresh set target x");
                    let target_y = decision.target_y.expect("refresh set target y");
                    targets.insert(idx, Vec2::new(target_x, target_y));
                }
            }
        }
    }
}

/// Resolve player-vs-player overlaps so bodies block instead of passing
/// through. Each pair pushed apart by its penetration; a sliding player
/// barges through (takes less of the push) and knocks the other off
/// balance (stun). O(n^2)=45 pairs, deterministic.
/// Exposed for the integration tests in `tests/match.rs`, per ARCHITECTURE.md
/// §3 rule 6 ("everything a test touches is `pub`"): crates here are internal, so
/// visibility is not worth fighting to keep a spec case unportable.
pub fn resolve_collisions(s: &mut MatchState) {
    let field = s.field;
    let n = s.players.len();
    for a in 0..n {
        for b in (a + 1)..n {
            let pa = s.players[a].clone();
            let pb = s.players[b].clone();
            let delta = pa.pos.sub(pb.pos);
            let d = delta.length();
            let collision_reach = species::collision_reach(pa.owned_verb)
                .max(species::collision_reach(pb.owned_verb));
            let mind = pa.radius + pb.radius + collision_reach;
            if d < mind {
                let dir = if d > 0.0 {
                    delta.normalized()
                } else {
                    Vec2::new(1.0, 0.0)
                };
                let pen = mind - d;
                let mut fa = 0.5;
                let mut fb = 0.5; // share the push evenly by default
                let owner = s.owner;
                let a_idx = (a + 1) as i64;
                let b_idx = (b + 1) as i64;
                // A defender leaning on the BALL CARRIER shoves them off
                // their spot: standing still under pressure is never fully
                // safe.
                if owner == Some(a_idx) && pb.team != pa.team && pb.slide_timer <= 0.0 {
                    fa = 0.7;
                    fb = 0.3;
                } else if owner == Some(b_idx) && pa.team != pb.team && pa.slide_timer <= 0.0 {
                    fa = 0.3;
                    fb = 0.7;
                }
                if pa.slide_timer > 0.0 && pb.slide_timer <= 0.0 {
                    fa = 0.15;
                    fb = 0.85;
                    if pb.stun_timer <= 0.0 {
                        s.players[b].stun_timer = STUN_TIME;
                    }
                } else if pb.slide_timer > 0.0 && pa.slide_timer <= 0.0 {
                    fa = 0.85;
                    fb = 0.15;
                    if pa.stun_timer <= 0.0 {
                        s.players[a].stun_timer = STUN_TIME;
                    }
                }
                let new_a = clamp_to_field(field, pa.pos.add(dir.scale(pen * fa)));
                let new_b = clamp_to_field(field, pb.pos.sub(dir.scale(pen * fb)));
                s.players[a].pos = new_a;
                s.players[b].pos = new_b;
            }
        }
    }
}

/// What a caller wants from one locomotion tick beyond the desired velocity.
///
/// `desired` still carries the caller's situational speed (stun, wind-up,
/// combat scaling, arrival easing, species burst); everything a *context*
/// owns — the sprint/carry/strafe multipliers, the accel/decel/turn rates —
/// is resolved inside [`apply_locomotion`].
#[derive(Clone, Copy, Debug)]
struct LocoOpts {
    /// Whether the ball is at this player's feet, for context resolution.
    carrying: bool,
    /// Where this player wants to look, independent of where they move.
    facing: locomotion::FacingIntent,
}

impl LocoOpts {
    /// Off the ball, facing tracking the heading — the common case.
    fn offball() -> Self {
        LocoOpts {
            carrying: false,
            facing: locomotion::FacingIntent::Movement,
        }
    }

    /// Off the ball, looking somewhere specific.
    fn facing(target: Vec2) -> Self {
        LocoOpts {
            carrying: false,
            facing: locomotion::FacingIntent::Toward(target),
        }
    }

    /// With the ball at the feet.
    fn carrying(self) -> Self {
        LocoOpts {
            carrying: true,
            ..self
        }
    }
}

/// The fastest `p` may be travelling `dist` from a positional target and
/// still stop on it, in px/s.
///
/// Reads the neutral `Run` profile's deceleration: an off-ball body easing
/// into a spot is not in a context that brakes unusually well or badly, and
/// resolving the context here would be circular — the answer feeds the
/// command the context is resolved from.
fn arrival_cap(p: &MatchPlayer, dist: f64, tune: &Tuning) -> f64 {
    let stats = locomotion::stats(p.move_speed, p.strength, p.dribble, tune);
    let profile = locomotion::profile(
        locomotion::Resolution {
            ctx: gc_data::locomotion::LocoContext::Run,
            carry: gc_data::locomotion::CarryMode::Empty,
        },
        p.move_speed,
        stats,
        tune,
    );
    locomotion::arrival_speed(dist, profile.decel)
}

/// Locomotion helper: run one tick of [`locomotion::step`] for `p`, then move
/// `p.pos` by the resulting run velocity, clamped to the field.
///
/// This is the single seam every walking/running movement in the match goes
/// through. It resolves the player's context, derives that context's
/// kinematic parameters from their stats, and hands both to the primitive.
/// Bespoke movement — slides, jukes, keeper dives — deliberately does not
/// come through here.
fn apply_locomotion(
    field: PitchSize,
    p: &mut MatchPlayer,
    desired: Vec2,
    dt: f64,
    tune: &Tuning,
    opts: LocoOpts,
) {
    // Step 1: resolve the context. The throttle is the caller's commanded
    // speed as a fraction of this player's base, so the jog/run edge is a
    // property of the command rather than of the result — a slower context
    // multiplier can never feed back and reclassify the body.
    let base_speed = p.move_speed.max(1.0);
    let commanded = desired.length();
    let throttle = commanded / base_speed;
    let move_dir = if commanded > 0.0 {
        desired.normalized()
    } else {
        p.run_vel.normalized()
    };
    let face_target = match opts.facing {
        locomotion::FacingIntent::Toward(v) => v,
        _ => p.facing,
    };
    let resolution = locomotion::resolve(
        throttle,
        opts.carrying,
        p.sprinting,
        move_dir,
        face_target,
        tune,
    );
    // Step 2: derive the context's kinematic parameters for this player.
    let stats = locomotion::stats(p.move_speed, p.strength, p.dribble, tune);
    let profile = locomotion::profile(resolution, p.move_speed, stats, tune);
    // Steps 3-5.
    let next = locomotion::step(
        locomotion::Kinematics {
            run_vel: p.run_vel,
            facing: p.facing,
        },
        &locomotion::Command {
            vel: desired,
            carrying: opts.carrying,
            sprinting: p.sprinting,
            facing: opts.facing,
        },
        &profile,
        dt,
        tune,
    );
    p.run_vel = next.run_vel;
    p.facing = next.facing;
    p.pos = clamp_to_field(field, p.pos.add(p.run_vel.scale(dt)));
}

fn nearest_outfield_opponent(s: &MatchState, carrier: &MatchPlayer) -> (f64, Option<usize>) {
    let mut best = f64::INFINITY;
    let mut opponent = None;
    for (i, p) in s.players.iter().enumerate() {
        if p.team != carrier.team && !p.is_keeper {
            let d = carrier.pos.dist(p.pos);
            if d < best {
                best = d;
                opponent = Some(i);
            }
        }
    }
    (best, opponent)
}

fn update_sprint(p: &mut MatchPlayer, want: bool, dt: f64, tune: &Tuning) {
    let can = p.sprint_meter > (if p.sprinting { 0.0 } else { SPRINT_ENGAGE });
    p.sprinting = want && can;
    if p.sprinting {
        p.sprint_meter = (p.sprint_meter - dt / p.sprint_dur).max(0.0);
    } else {
        p.sprint_meter = (p.sprint_meter + tune.value("SPRINT_REFILL") * dt).min(1.0);
    }
}

fn aerial_active_for_input(s: &MatchState, player_index: i64, input: &MatchInput) -> bool {
    let player = &s.players[(player_index - 1) as usize];
    Some(player_index) != s.owner
        && player.aerial_recovery <= 0.0
        && s.ball_z > GROUND_GRAB_HEIGHT
        && s.ball_vz < 0.0
        && player.pos.dist(s.ball) <= AERIAL_ANTICIPATE
        && (aerial::strike_requested(input) || player.receive_timer > 0.0)
}

/// Move every player one tick. Ports `move_players`.
#[allow(clippy::too_many_lines)]
fn move_players(
    s: &mut MatchState,
    dt: f64,
    inputs: &IndexMap<i64, MatchInput>,
    combat_state: Option<&CombatMatchState>,
    tune: &Tuning,
) {
    let field = s.field;
    // Snapshot positions so role targets read one consistent world state and
    // we can derive each player's realized velocity after everyone has
    // moved. Vec2 is immutable, so aliasing p.pos here is safe.
    let prev: Vec<Vec2> = s.players.iter().map(|p| p.pos).collect();
    let (mut targets, mut urgent, closing) = offball_targets(s, &prev, combat_state, tune);
    retain_offball_targets(s, &mut targets, &mut urgent, combat_state, tune);

    for i in 0..s.players.len() {
        let idx = (i + 1) as i64;
        let combat_scale =
            combat_state.map_or(1.0, |cs| combat::movement_multiplier(Some(cs), idx));
        let combat_blocked = combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), idx));
        let is_keeper = s.players[i].is_keeper;
        if is_keeper && s.owner == Some(idx) {
            let p = &mut s.players[i];
            p.keeper_state = keeper::KeeperBehaviorState::Base;
            p.keeper_state_timer = 0.0;
            p.keeper_release_state = None;
            p.keeper_release_motion = 0.0;
            p.keeper_release_kind = None;
            p.keeper_release_depth = 0.0;
        }
        if is_human_player(s, idx) {
            move_human_player(s, i, idx, dt, inputs, combat_scale, tune);
        } else if Some(idx) == s.owner {
            if !is_keeper {
                move_ai_owner(s, i, idx, dt, combat_blocked, combat_scale, tune);
            } else {
                move_ai_owner_keeper(s, i, dt, tune);
            }
        } else if is_keeper {
            move_offball_keeper(s, i, &prev, dt, combat_state, tune);
        } else {
            move_offball_outfield(
                s,
                i,
                idx,
                &targets,
                &urgent,
                &closing,
                dt,
                combat_scale,
                tune,
            );
        }
    }

    // A keeper in possession is PHYSICALLY protected (laws of the game: you
    // cannot challenge a keeper holding the ball). AI targets already
    // retreat; this ring catches the human-controlled player and any
    // straggler. Ball at the keeper's FEET (a received back-pass) is fair
    // game — no ring.
    if let Some(oi) = s.owner
        && s.players[(oi - 1) as usize].is_keeper
        && !s.players[(oi - 1) as usize].feet_ball
    {
        let kpos = s.players[(oi - 1) as usize].pos;
        let kteam = s.players[(oi - 1) as usize].team;
        for i in 0..s.players.len() {
            if s.players[i].team != kteam {
                let off = s.players[i].pos.sub(kpos);
                let d = off.length();
                if d < tune.value("KEEPER_RESPECT_DIST") {
                    let dir = if d > 0.0 {
                        off.normalized()
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    s.players[i].pos = clamp_to_field(
                        field,
                        kpos.add(dir.scale(tune.value("KEEPER_RESPECT_DIST"))),
                    );
                }
            }
        }
    }

    // Push apart any overlapping bodies before deriving velocity, so a
    // shove registers as motion and players never occupy the same point.
    resolve_collisions(s);

    // Realized velocity (px/s) from this tick's movement; AI prediction
    // source.
    if dt > 0.0 {
        for (i, p) in s.players.iter_mut().enumerate() {
            p.vel = p.pos.sub(prev[i]).scale(1.0 / dt);
        }
    }
}

#[allow(clippy::too_many_lines)]
fn move_human_player(
    s: &mut MatchState,
    i: usize,
    idx: i64,
    dt: f64,
    inputs: &IndexMap<i64, MatchInput>,
    combat_scale: f64,
    tune: &Tuning,
) {
    let field = s.field;
    let neutral = MatchInput::default();
    let input = *inputs.get(&idx).unwrap_or(&neutral);
    let aerial_active = aerial_active_for_input(s, idx, &input);
    // A tackle charge only ever names an OPPOSING owner as its target --
    // never a teammate and never the presser's own self, so a ball-carrying
    // human pressing tackle on themselves can never resolve a "hit" against
    // their own action slot (see `advance_tackle_actions`'s matching guard
    // on the AI side, and this PR's description on the crash this closes).
    let team = s.players[i].team;
    let opposing_owner = s
        .owner
        .filter(|&o| o != idx && s.players[(o - 1) as usize].team != team);
    let p = &mut s.players[i];
    // Tackle button: a committed slide while SPRINTING, else a standing
    // poke — one legible rule (sprint + tackle = the big slide). Slide
    // speed scales off current velocity (p.vel) so it feels relative. The
    // standing poke commits through the same action slot an AI presser
    // does (#489, `advance_tackle_actions`) -- no private human fast path.
    if input.dash
        && !aerial_active
        && p.aerial_recovery <= 0.0
        && p.slide_timer <= 0.0
        && p.action.phase == ActionPhase::None
        && p.tackle_cd <= 0.0
        && p.stun_timer <= 0.0
    {
        let sp = p.vel.length();
        if p.sprinting {
            let d = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
                input.r#move.normalized()
            } else {
                p.facing
            };
            p.slide_timer = SLIDE_DURATION;
            p.slide_dir = d;
            p.slide_vel = (sp * SLIDE_MULT).max(SLIDE_BASE_MIN);
            p.facing = d;
            p.tackle_cd = SLIDE_CD;
        } else {
            p.action =
                action_slot::commit_charge(&p.action, ActionVerb::Tackle, opposing_owner, 0.0);
            p.tackle_cd = STAND_CD;
        }
    }

    // Trigger a juke (not while sliding): a quick sidestep with tackle
    // immunity.
    if input.dodge && p.dodge_cd <= 0.0 && p.slide_timer <= 0.0 && p.aerial_recovery <= 0.0 {
        let mut perp = Vec2::new(-p.facing.y, p.facing.x);
        if input.r#move.x * perp.x + input.r#move.y * perp.y < 0.0 {
            perp = perp.scale(-1.0);
        }
        p.dodge_timer = DODGE_DURATION;
        p.dodge_cd = DODGE_CD;
        p.dodge_dir = perp;
        let (px, py, pid) = (p.pos.x, p.pos.y, p.id.clone());
        s.events.push(MatchEvent {
            kind: MatchEventKind::Juke,
            x: px,
            y: py,
            player: Some(pid),
            save_style: None,
            style: None,
            outcome: None,
            jumping: None,
            difficulty: None,
            shot_type: None,
            keeper_state: None,
            keeper_depth: None,
            on_target: None,
        });
    }

    let p = &mut s.players[i];
    if p.slide_timer > 0.0 {
        // Committed slide: locked direction, decaying speed, can't steer.
        // Also drain run_vel so momentum doesn't carry after the slide
        // ends.
        p.pos = clamp_to_field(field, p.pos.add(p.slide_dir.scale(p.slide_vel * dt)));
        p.slide_vel *= (1.0 - SLIDE_FRICTION * dt).max(0.0);
        p.run_vel = Vec2::new(0.0, 0.0);
    } else if p.dodge_timer > 0.0 {
        // Juke overrides steering: slide sideways fast. Drain run_vel so
        // the exit from the juke starts from rest.
        p.pos = clamp_to_field(
            field,
            p.pos
                .add(p.dodge_dir.scale(p.move_speed * DODGE_SPEED_MULT * dt)),
        );
        p.run_vel = Vec2::new(0.0, 0.0);
    } else {
        let dir = input.r#move;
        let moving = dir.x != 0.0 || dir.y != 0.0;
        // Jockey stance (Space held off the ball): shadow the carrier at
        // reduced speed, facing locked toward the ball. Mutually exclusive
        // with sprint (jockey wins). Grants bonus poke reach on release.
        let jockeying =
            input.jockey && Some(idx) != s.owner && p.stun_timer <= 0.0 && !aerial_active;
        if jockeying {
            p.jockey_timer = JOCKEY_HOLD;
        }
        // Sprint: needs a quarter tank to (re)engage, but once running it
        // burns to empty — so a drained meter doesn't flicker the boost on
        // and off at the refill rate.
        let want = input.sprint && moving && p.stun_timer <= 0.0 && !jockeying;
        update_sprint(p, want, dt, tune);
        let recovery_scale = if p.action.phase == ActionPhase::Recovering {
            tune.value("ACTION_RECOVERY_CONTROL")
        } else {
            1.0
        };
        let mut mv = p.move_speed
            * (if p.stun_timer > 0.0 { STUN_SLOW } else { 1.0 })
            * recovery_scale
            * combat_scale;
        if jockeying {
            mv *= tune.value("JOCKEY_SLOW");
        } else if p.sprinting {
            // The sprint TOP-SPEED multiplier is the sprint context's own
            // knob now (`SPRINT_MULT`, applied inside `apply_locomotion`);
            // only the species burst stays here, because it is a property of
            // the body rather than of the context.
            mv *= species::burst_speed(p.owned_verb);
        }
        // Plant during wind-up: striker slows to 30% while winding up.
        if p.windup_timer > 0.0 {
            mv *= WINDUP_MOVE;
        }
        if p.aerial_recovery > 0.0 {
            if p.aerial_style == Some(aerial::AerialStyle::Bicycle) {
                mv = 0.0;
            } else if p.aerial_jump > 0.0 {
                mv *= 0.35;
            } else if p.aerial_style == Some(aerial::AerialStyle::LegControl)
                || p.aerial_style == Some(aerial::AerialStyle::ChestControl)
            {
                mv *= 0.6;
            } else {
                mv *= 0.55;
            }
        }
        // Stationary-aiming exception: when input is held but the player
        // hasn't built speed yet, facing follows the stick, not the heading.
        let mut desired = if moving {
            dir.normalized().scale(mv)
        } else {
            Vec2::new(0.0, 0.0)
        };
        let had_input_facing = moving && p.run_vel.length() <= RUN_VEL_FACE_MIN;
        // Dribble hook: while the carrier's touch runs on ahead they are
        // NOT free to run elsewhere — movement steers back to the ball (the
        // chase half of kick-chase-kick), automatically when no input is
        // held. The stick keeps choosing the FACING — where the NEXT touch
        // goes — and free movement returns the moment the ball is back at
        // the feet. Owners only: off the ball you run wherever you like.
        let hooked = Some(idx) == s.owner
            && (p.feet_ball || !p.is_keeper)
            && p.pos.dist(s.ball) > DRIBBLE_TOUCH_REACH;
        if hooked {
            let to_ball = s.ball.sub(p.pos);
            if to_ball.length() > 1.0 {
                desired = to_ball.normalized().scale(mv);
            }
        } else if !moving && p.receive_timer > 0.0 && s.owner.is_none() {
            // Receive assist: the designated receiver of a pass works to
            // meet it by default — hold a direction to override and attack
            // a different spot instead.
            let to_ball = s.ball.sub(p.pos);
            if to_ball.length() > 1.0 {
                desired = to_ball.normalized().scale(mv);
            }
        }
        // Aerial magnet: going up for a dropping ball nearby (holding the
        // aerial button off the ball) glides the player toward it, so a
        // cross is met without pixel-perfect positioning. It overrides the
        // jockey slowdown and steers even from a standstill.
        let going_aerial = aerial_active;
        if going_aerial && s.ball_z > GROUND_GRAB_HEIGHT && s.ball_vz < 0.0 {
            let to_ball = s.ball.sub(p.pos);
            let d = to_ball.length();
            if d > 1.0 && d <= AERIAL_ANTICIPATE {
                desired = desired.add(to_ball.normalized().scale(tune.value("AERIAL_MAGNET")));
                let cap = p.move_speed * 1.3;
                if desired.length() > cap {
                    desired = desired.normalized().scale(cap);
                }
            }
        }
        // Facing is its own target, resolved BEFORE the tick rather than
        // slammed over the result afterwards. The precedence is unchanged;
        // what changed is that each case now states a heading the body
        // rotates toward at a bounded rate instead of teleporting there.
        let ball_off = s.ball.sub(p.pos);
        let intent = if going_aerial && aerial::acrobatic_requested(&input) {
            // Bicycle geometry reads the approach facing; the contact
            // magnet must not rotate the player to face a ball behind.
            locomotion::FacingIntent::Hold
        } else if jockeying && ball_off.length() > 1.0 {
            // Jockey stance: face the ball regardless of movement. This is
            // the case that makes `strafe` and `backpedal` reachable for a
            // human — shadowing sideways is now a different profile from
            // running sideways.
            locomotion::FacingIntent::Toward(ball_off)
        } else if (Some(idx) == s.owner && moving) || had_input_facing {
            // A carrier's facing always obeys the stick — even while hooked
            // to a run-on ball — so the next touch turns the dribble where
            // you point, not where the chase ran.
            locomotion::FacingIntent::Toward(dir)
        } else {
            locomotion::FacingIntent::Movement
        };
        // `jockeying` already requires not being the owner, so the two are
        // mutually exclusive by construction.
        let opts = LocoOpts {
            carrying: Some(idx) == s.owner,
            facing: intent,
        };
        apply_locomotion(field, p, desired, dt, tune, opts);
    }
    // A keeper holding the ball in its HANDS may not carry it out of the
    // penalty area (the drawn box) — the laws, and the renderer, agree. Off
    // the ball or with a back-pass at the feet it may roam.
    let p = &mut s.players[i];
    if p.is_keeper && s.owner == Some(idx) && !p.feet_ball {
        let minx = if p.team == Team::Home {
            PLAYER_RADIUS
        } else {
            s.field.w - PENALTY_DEPTH
        };
        let maxx = if p.team == Team::Home {
            PENALTY_DEPTH
        } else {
            s.field.w - PLAYER_RADIUS
        };
        let top = s.field.h / 2.0 - PENALTY_H / 2.0 + PLAYER_RADIUS;
        let bot = s.field.h / 2.0 + PENALTY_H / 2.0 - PLAYER_RADIUS;
        p.pos = Vec2::new(p.pos.x.clamp(minx, maxx), p.pos.y.clamp(top, bot));
    }
}

fn move_ai_owner(
    s: &mut MatchState,
    i: usize,
    idx: i64,
    dt: f64,
    combat_blocked: bool,
    combat_scale: f64,
    tune: &Tuning,
) {
    let field = s.field;
    // AI owner dribbles toward the opponent goal.
    let p = s.players[i].clone();
    let goal = if p.team == Team::Home {
        s.goal_away
    } else {
        s.goal_home
    };
    let gc = Vec2::new(goal.x + goal.w / 2.0, goal.y + goal.h / 2.0);
    let (pressure, threat_i) = nearest_outfield_opponent(s, &p);
    // React while a standing-poke tackle is CHARGING, not merely pressed
    // this instant (#489 gives the charge phase a real, telegraphed
    // duration precisely so a carrier can read and answer it — the same
    // "evade a telegraph" idea `combat_policy`'s `EVADE_WINDOW_TICKS`
    // already uses for shots). A same-tick reaction to a same-tick slide
    // would still make AI carriers psychic, so that half keeps its
    // one-tick-late check.
    let threat_committed = threat_i.is_some_and(|ti| {
        let t = &s.players[ti];
        (t.action.verb == Some(ActionVerb::Tackle) && t.action.phase == ActionPhase::Charging)
            || (t.slide_timer > 0.0 && t.slide_timer < SLIDE_DURATION)
    });
    if threat_committed
        && !combat_blocked
        && pressure <= tune.value("AI_JUKE_DIST")
        && p.dodge_cd <= 0.0
        && p.dodge_timer <= 0.0
        && p.stun_timer <= 0.0
        && p.pos.dist(s.ball) <= DRIBBLE_TOUCH_REACH
    {
        // React to a defender who has actually committed: sidestep away
        // from their side, rather than spamming jukes on proximity.
        let threat_pos = s.players[threat_i.expect("threat_committed implies threat")].pos;
        let mut perp = Vec2::new(-p.facing.y, p.facing.x);
        let to_threat = threat_pos.sub(p.pos);
        if to_threat.x * perp.x + to_threat.y * perp.y > 0.0 {
            perp = perp.scale(-1.0);
        }
        let pm = &mut s.players[i];
        pm.dodge_timer = DODGE_DURATION;
        pm.dodge_cd = tune.value("AI_JUKE_CD");
        pm.dodge_dir = perp;
        let (px, py, pid) = (pm.pos.x, pm.pos.y, pm.id.clone());
        s.events.push(MatchEvent {
            kind: MatchEventKind::Juke,
            x: px,
            y: py,
            player: Some(pid),
            save_style: None,
            style: None,
            outcome: None,
            jumping: None,
            difficulty: None,
            shot_type: None,
            keeper_state: None,
            keeper_depth: None,
            on_target: None,
        });
    }
    let p = s.players[i].clone();
    let goal_dist = p.pos.dist(gc);
    let want_sprint = pressure >= tune.value("AI_SPRINT_SPACE")
        && !combat_blocked
        && goal_dist > tune.value("AI_SHOOT_RANGE")
        && p.windup_timer <= 0.0
        && p.dodge_timer <= 0.0
        && p.stun_timer <= 0.0;
    update_sprint(&mut s.players[i], want_sprint, dt, tune);
    let p = s.players[i].clone();
    let mut mv = p.move_speed * (if p.stun_timer > 0.0 { STUN_SLOW } else { 1.0 }) * combat_scale;
    if p.sprinting {
        // Top speed comes from the sprint_carry context inside
        // `apply_locomotion`; only the species burst is a body property.
        mv *= species::burst_speed(p.owned_verb);
    }
    // Plant during wind-up: AI striker slows to 30% while winding up.
    if p.windup_timer > 0.0 {
        mv *= WINDUP_MOVE;
    }
    // Derive desired direction from ai::steer (which returns a clamped
    // position and a unit direction), then feed apply_locomotion.
    let (_, dir) = ai::steer(p.pos, gc, mv * dt);
    let mut desired = if dir.x != 0.0 || dir.y != 0.0 {
        dir.scale(mv)
    } else {
        Vec2::new(0.0, 0.0)
    };
    // Dribble hook (same rule as the human carrier): chase the run-on touch
    // before anything else, facing kept on the dribble line so the next
    // touch continues toward goal.
    if p.pos.dist(s.ball) > DRIBBLE_TOUCH_REACH {
        let to_ball = s.ball.sub(p.pos);
        if to_ball.length() > 1.0 {
            desired = to_ball.normalized().scale(mv);
        }
    }
    if p.dodge_timer > 0.0 {
        let field = s.field;
        let pm = &mut s.players[i];
        pm.pos = clamp_to_field(
            field,
            p.pos
                .add(p.dodge_dir.scale(p.move_speed * DODGE_SPEED_MULT * dt)),
        );
        s.players[i].run_vel = Vec2::new(0.0, 0.0);
        // A juke is bespoke movement that bypasses locomotion, so its facing
        // stays the instant assignment it always was.
        if dir.x != 0.0 || dir.y != 0.0 {
            s.players[i].facing = dir;
        }
    } else {
        // The AI carrier looks along its dribble line, not along the chase
        // that a run-on touch forces — the same rule the human carrier gets,
        // and the reason a hooked carrier resolves as a strafe rather than a
        // straight carry.
        let mut pm = s.players[i].clone();
        apply_locomotion(
            field,
            &mut pm,
            desired,
            dt,
            tune,
            LocoOpts::facing(dir).carrying(),
        );
        s.players[i] = pm;
    }
    let _ = idx;
}

fn move_ai_owner_keeper(s: &mut MatchState, i: usize, dt: f64, tune: &Tuning) {
    let field = s.field;
    // A keeper holding the ball faces upfield; if an opponent is camped
    // right in front of it, step laterally to open a throwing angle.
    let p = s.players[i].clone();
    // Upfield is this keeper's standing facing target. Sidestepping to open a
    // throwing angle is therefore a STRAFE by construction — movement across
    // the facing — which is precisely the shape the independent facing target
    // exists to make expressible.
    let upfield = Vec2::new(if p.team == Team::Home { 1.0 } else { -1.0 }, 0.0);
    let mut camper: Option<Vec2> = None;
    for q in &s.players {
        if q.team != p.team && q.pos.dist(p.pos) < tune.value("KEEPER_RESPECT_DIST") {
            camper = Some(q.pos);
            break;
        }
    }
    let desired = match camper {
        Some(camper_pos) => {
            let side = if camper_pos.y >= p.pos.y { -1.0 } else { 1.0 };
            Vec2::new(0.0, side * p.move_speed)
        }
        None => Vec2::new(0.0, 0.0),
    };
    let mut pm = s.players[i].clone();
    apply_locomotion(
        field,
        &mut pm,
        desired,
        dt,
        tune,
        LocoOpts::facing(upfield).carrying(),
    );
    s.players[i] = pm;
}

#[allow(clippy::too_many_lines)]
fn move_offball_keeper(
    s: &mut MatchState,
    i: usize,
    prev: &[Vec2],
    dt: f64,
    combat_state: Option<&CombatMatchState>,
    tune: &Tuning,
) {
    let field = s.field;
    let _ = combat_state;
    update_sprint(&mut s.players[i], false, dt, tune);
    let p = s.players[i].clone();
    let idx = (i + 1) as i64;
    let entry_motion = (p.run_vel.length() / p.move_speed.max(1.0)).min(1.0);
    let goal = if p.team == Team::Home {
        s.goal_home
    } else {
        s.goal_away
    };
    let goal_line_x = if p.team == Team::Home {
        goal.x + goal.w
    } else {
        goal.x
    };
    let infield_direction = if p.team == Team::Home { 1.0 } else { -1.0 };
    let carrier_idx = s.owner;
    let mut carrier = carrier_idx.map(|ci| s.players[(ci - 1) as usize].clone());
    if let Some(c) = &carrier
        && (c.is_keeper || c.team == p.team)
    {
        carrier = None;
    }

    let mut support_near = false;
    let mut defender_engaged = false;
    let mut carrier_pos: Option<Vec2> = None;
    if let (Some(carrier), Some(carrier_idx)) = (&carrier, carrier_idx) {
        let cpos = prev[(carrier_idx - 1) as usize];
        carrier_pos = Some(cpos);
        for (other_idx, other) in s.players.iter().enumerate() {
            let oi = (other_idx + 1) as i64;
            if oi != carrier_idx
                && !other.is_keeper
                && other.team == carrier.team
                && prev[other_idx].dist(cpos) <= KEEPER_1V1_SUPPORT
            {
                support_near = true;
            } else if other.team == p.team
                && !other.is_keeper
                && prev[other_idx].dist(cpos) <= KEEPER_1V1_SUPPORT * 0.6
            {
                defender_engaged = true;
            }
        }
    }

    let loose_touch = carrier_pos.is_some_and(|cp| cp.dist(s.ball) > DRIBBLE_TOUCH_REACH);
    let attacker_controlled = carrier.is_some() && !loose_touch;
    let attacker_in_front = (s.ball.x - p.pos.x) * infield_direction > 0.0;
    let position_context = keeper::KeeperAdvanceContext {
        in_claim_zone: in_claim_zone(s, idx),
        attacker_controlled,
        loose_touch,
        support_near,
        defender_engaged,
        threat_distance: p.pos.dist(s.ball),
    };
    let contain_eligible =
        carrier.is_some() && attacker_in_front && keeper::should_contain(&position_context);
    let advance_eligible = contain_eligible && keeper::should_advance(&position_context);

    // NOT a team-conditional ternary, on purpose. For the away team the
    // first clause is always false, so this reduces to the intended
    // `ball_vel.x > 0`. But for the HOME team it reduces to
    // `(ball_vel.x < 0) or (ball_vel.x > 0)` — true for ANY nonzero ball
    // velocity, not just "moving toward the home goal" — which is the
    // simulation's actual, differentially-verified behavior (a home
    // keeper's parry rebound with ball_vel.x > 0 still reads as
    // `toward_goal = true`; see `tests/match_differential.rs`). Keep this
    // expression exactly as written rather than "fixing" it to the
    // seemingly-intended per-team check — that would move the pinned
    // determinism evidence.
    let toward_goal = (p.team == Team::Home && s.ball_vel.x < 0.0) || s.ball_vel.x > 0.0;
    if s.owner.is_some() || !toward_goal {
        let pm = &mut s.players[i];
        pm.keeper_release_state = None;
        pm.keeper_release_motion = 0.0;
        pm.keeper_release_kind = None;
        pm.keeper_release_depth = 0.0;
    }
    let p = s.players[i].clone();

    let mut ground_cue =
        p.keeper_release_kind == Some(keeper::KeeperShotType::Ground) && toward_goal;
    let mut lob_cue = p.keeper_release_kind == Some(keeper::KeeperShotType::Chip) && toward_goal;
    if let Some(carrier) = &carrier
        && let Some(pending) = &carrier.windup_shot
    {
        if pending.shot_type == keeper::KeeperShotType::Chip {
            lob_cue = true;
        } else {
            let shot_context = keeper::KeeperShotContext {
                defending_team: to_keeper_team(p.team),
                shooter_team: to_keeper_team(carrier.team),
                origin: carrier.pos,
                direction: pending.dir,
                goal: to_keeper_rect(goal),
            };
            if (carrier.windup_timer <= 0.0 && keeper::shot_targets_goal(&shot_context))
                || keeper::should_set(&keeper::KeeperSetContext {
                    defending_team: shot_context.defending_team,
                    shooter_team: shot_context.shooter_team,
                    origin: shot_context.origin,
                    direction: shot_context.direction,
                    goal: shot_context.goal,
                    anticipation: p.keeper_anticipation,
                    windup_duration: tune.value("SHOT_WINDUP"),
                    windup_remaining: carrier.windup_timer,
                })
            {
                ground_cue = true;
                if carrier.windup_timer <= 0.0 {
                    // Planting on the release tick must not erase the
                    // velocity carried into it. The captured debt survives
                    // until the save attempt.
                    s.players[i].keeper_release_motion = entry_motion;
                }
            }
        }
    }

    let mut through_ball_cue = false;
    if s.owner.is_none() && toward_goal && p.keeper_release_kind.is_none() {
        for receiver in &s.players {
            if receiver.team != p.team && receiver.receive_timer > 0.0 {
                let receiver_depth = (receiver.pos.x - goal_line_x) * infield_direction;
                if receiver_depth <= KEEPER_BOX_DEPTH + KEEPER_1V1_SUPPORT * 0.5 {
                    through_ball_cue = true;
                    break;
                }
            }
        }
    }

    let behavior = keeper::behavior(&keeper::KeeperBehaviorContext {
        current_state: p.keeper_state,
        state_timer: p.keeper_state_timer,
        keeper_pos: prev[i],
        ball_pos: s.ball,
        goal: to_keeper_rect(goal),
        team: to_keeper_team(p.team),
        aggression: p.keeper_aggression,
        advance_eligible,
        contain_eligible,
        ground_cue,
        lob_cue,
        through_ball_cue,
        dt,
    });
    {
        let pm = &mut s.players[i];
        pm.keeper_state = behavior.state;
        pm.keeper_state_timer = behavior.state_timer;
        if pm.dive_timer > 0.0 || pm.save_pending.is_some() || pm.dive_delay > 0.0 {
            pm.keeper_set = 0.0;
        } else if pm.keeper_state == keeper::KeeperBehaviorState::Set {
            pm.keeper_set += dt;
        } else {
            pm.keeper_set = 0.0;
        }
    }

    let p = s.players[i].clone();
    if p.dive_timer > 0.0 {
        // Diving: lunge hard toward the intercept point — and STOP there.
        // Unclamped, a near-straight shot (a 2px correction) became a
        // full-speed lunge PAST the ball: gloves closing on empty air while
        // the save resolved elsewhere. Dives bypass locomotion (bespoke
        // movement — keep as-is).
        let step = p.move_speed * 1.6 * dt;
        let to_target = p.dive_target.map(|t| t.sub(p.pos));
        let pm = &mut s.players[i];
        if let Some(tt) = to_target
            && tt.length() > 0.5
        {
            let dir = tt.normalized();
            pm.pos = clamp_to_field(field, p.pos.add(dir.scale(step.min(tt.length()))));
            pm.facing = dir;
        } else if p.dive_target.is_none() {
            // No known intercept (legacy path): the old straight lunge.
            pm.pos = clamp_to_field(field, p.pos.add(p.dive_dir.scale(step)));
            pm.facing = p.dive_dir;
        }
        pm.run_vel = Vec2::new(0.0, 0.0);
    } else if p.save_pending.is_some() || p.dive_delay > 0.0 {
        // A committed reaction owns the keeper until it launches or
        // resolves. Decelerate through locomotion instead of layering
        // positioning movement under the queued save.
        let mut pm = s.players[i].clone();
        apply_locomotion(
            field,
            &mut pm,
            Vec2::new(0.0, 0.0),
            dt,
            tune,
            LocoOpts::offball(),
        );
        s.players[i] = pm;
    } else if s.owner.is_none() && p.receive_timer > 0.0 {
        // Meet a teammate's back-pass at the ball. Generic predictive
        // pursuit is wrong here: its horizon grows with distance, so an
        // incoming pass projects behind the keeper and sends it backward
        // through the goal instead of forward to receive.
        let (_, dir) = ai::steer(p.pos, s.ball, p.move_speed * dt);
        let desired = if dir.x != 0.0 || dir.y != 0.0 {
            dir.scale(p.move_speed)
        } else {
            Vec2::new(0.0, 0.0)
        };
        let mut pm = s.players[i].clone();
        apply_locomotion(field, &mut pm, desired, dt, tune, LocoOpts::offball());
        s.players[i] = pm;
    } else if s.owner.is_none()
        && in_claim_zone(s, idx)
        && p.keeper_release_kind.is_none()
        && !through_ball_cue
    {
        // Come off the line to claim a loose ball in the box. Predictive
        // pursuit remains useful for non-designated claims.
        let aim = ai::pursue(p.pos, s.ball, s.ball_vel, KEEPER_LEAD);
        let (_, dir) = ai::steer(p.pos, aim, p.move_speed * dt);
        let desired = if dir.x != 0.0 || dir.y != 0.0 {
            dir.scale(p.move_speed)
        } else {
            Vec2::new(0.0, 0.0)
        };
        let mut pm = s.players[i].clone();
        apply_locomotion(field, &mut pm, desired, dt, tune, LocoOpts::offball());
        s.players[i] = pm;
    } else {
        // Ordinary keeper movement is owned by the explicit behavior state.
        // Base uses shallow dynamic depth plus the lateral corner
        // concession; contain/advance can commit further before recovery.
        let (_, dir) = ai::steer(p.pos, behavior.target, p.move_speed * dt);
        let mut movement_speed = p.move_speed * behavior.movement_scale;
        if p.keeper_state == keeper::KeeperBehaviorState::Base {
            // Neutral targets are deliberately shallow. Ease into them so
            // ordinary acceleration cannot oscillate past the 18 px base
            // cap and accidentally advertise a committed high line.
            let distance = p.pos.dist(behavior.target);
            movement_speed *= (distance / KEEPER_BASE_ARRIVE_RADIUS).min(1.0);
            // Same arrival rule as the off-ball outfielders: a keeper with
            // momentum that commands speed right up to a shallow base target
            // sails past it, which reads on screen as a committed high line
            // it never chose.
            movement_speed = movement_speed.min(arrival_cap(&p, distance, tune));
        }
        let desired = if dir.x != 0.0 || dir.y != 0.0 {
            dir.scale(movement_speed)
        } else {
            Vec2::new(0.0, 0.0)
        };
        let mut pm = s.players[i].clone();
        apply_locomotion(field, &mut pm, desired, dt, tune, LocoOpts::offball());
        s.players[i] = pm;
    }
}

#[allow(clippy::too_many_arguments)]
fn move_offball_outfield(
    s: &mut MatchState,
    i: usize,
    idx: i64,
    targets: &IndexMap<i64, Vec2>,
    urgent: &IndexMap<i64, bool>,
    closing: &IndexMap<i64, bool>,
    dt: f64,
    combat_scale: f64,
    tune: &Tuning,
) {
    let field = s.field;
    update_sprint(&mut s.players[i], false, dt, tune);
    // Off-ball AI: role-assigned target (press/cover/mark/support/zone).
    // Positional roles have CALM: ease in on approach, plant inside the
    // deadband, and once standing, stay planted until the spot drifts
    // beyond the wake radius — no robotic shuffling on the spot.
    let p = s.players[i].clone();
    let target = targets.get(&idx).copied().unwrap_or(p.anchor);
    let recovery_scale = if p.action.phase == ActionPhase::Recovering {
        tune.value("ACTION_RECOVERY_CONTROL")
    } else {
        1.0
    };
    let mv = p.move_speed
        * (if p.stun_timer > 0.0 { STUN_SLOW } else { 1.0 })
        * recovery_scale
        * combat_scale;
    let press_state = team_press(s, p.team);
    let active_presser = press_state.presser_index == Some(idx as u32);
    // A counter-presser is closing the ball, not containing it: the
    // contain slowdown and its ball-facing lock stay out of the window.
    let containing = active_presser
        && press_state.mode == outfield_press::StablePressMode::Contain
        && !closing.get(&idx).copied().unwrap_or(false);
    if active_presser && p.run_vel.length() > mv {
        s.players[i].run_vel = p.run_vel.normalized().scale(mv);
    }
    let p = s.players[i].clone();
    let dist = p.pos.dist(target);
    let standing = p.run_vel.length() < STAND_STILL_SPEED;
    let mut desired = Vec2::new(0.0, 0.0);
    if urgent.get(&idx).copied().unwrap_or(false)
        || dist
            > (if standing {
                tune.value("STAND_WAKE")
            } else {
                STAND_DEADBAND
            })
    {
        let (_, dir) = ai::steer(p.pos, target, mv * dt);
        if dir.x != 0.0 || dir.y != 0.0 {
            let mut speed = if urgent.get(&idx).copied().unwrap_or(false) {
                mv
            } else {
                mv * (dist / ARRIVE_RADIUS).min(1.0)
            };
            if containing {
                speed = outfield_press::contain_speed(speed, dist, tune.value("JOCKEY_SLOW"));
            }
            // Aim for ARRIVAL, not for the position. A body with momentum
            // that commands full speed right up to its spot arrives with
            // speed it then has to shed past the spot, and off-ball AI spends
            // the match oscillating around targets it keeps overshooting.
            // Capping the command at the speed it could still brake from is
            // the whole "minimal AI patch" the issue's risk section asks for.
            speed = speed.min(arrival_cap(&p, dist, tune));
            desired = dir.scale(speed);
        }
    }
    // Containing is a facing rule layered on the primitive — face the ball,
    // movement direction free — which is exactly the shape a future defensive
    // contain stance takes. It is also what makes a contain resolve as a
    // strafe or backpedal when the defender shuffles across or off the ball.
    let opts = if containing {
        LocoOpts::facing(s.ball.sub(p.pos))
    } else {
        LocoOpts::offball()
    };
    let mut pm = s.players[i].clone();
    apply_locomotion(field, &mut pm, desired, dt, tune, opts);
    s.players[i] = pm;
}

fn run_eligible(
    s: &MatchState,
    player_index: i64,
    combat_state: Option<&CombatMatchState>,
) -> bool {
    let player = &s.players[(player_index - 1) as usize];
    player_index != s.owner.unwrap_or(-1)
        && !player.is_keeper
        && !is_human_player(s, player_index)
        && player.stun_timer <= 0.0
        && player.slide_timer <= 0.0
        && player.action.phase == ActionPhase::None
        && player.dodge_timer <= 0.0
        && player.jockey_timer <= 0.0
        && player.windup_timer <= 0.0
        && player.windup_shot.is_none()
        && player.aerial_timer <= 0.0
        && player.aerial_recovery <= 0.0
        && player.receive_timer <= 0.0
        && !combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), player_index))
}

/// Decide the AI keeper's distribution target and commit a charging pass
/// intent for it (#531). The candidate scan — prefer a clear, safe ground
/// lane; fall back to the most-open teammate; fall back further to a
/// swarmed-everyone clearance — is the exact scan `keeper_distribute` always
/// ran; only what happens with the winner changed: it used to release
/// immediately through `release_throw`, and now it becomes an aim hint and a
/// charge threshold that materializes through the same `MatchInput` path a
/// human keeper's throw takes (`keeper_actions`). Which VERB executes
/// (`try_pass`'s kicked ground pass vs. `keeper_throw`'s hand throw) is not
/// decided here at all: it is read live from `feet_ball` at materialization,
/// same as for a human, so it cannot go stale across a multi-tick charge.
///
/// The swarmed case is not a pass or throw: it stays the direct drop-kick
/// clearance it always was in `keeper_dropkick_clearance` — it never called
/// `release_pass`, so the seam does not reach it.
fn commit_keeper_pass_intent(s: &mut MatchState, keeper_idx: i64, tune: &Tuning) {
    let keeper = s.players[(keeper_idx - 1) as usize].clone();
    let fwd = if keeper.team == Team::Home { 1.0 } else { -1.0 }; // +x is upfield for home
    let mut opp: Vec<Vec2> = Vec::new();
    for q in &s.players {
        if q.team != keeper.team {
            opp.push(q.pos);
        }
    }

    let threats = pass_threats(s, keeper.team);
    let mut best: Option<i64> = None;
    let mut best_score: Option<f64> = None;
    let mut best_f: Option<f64> = None;
    let mut open_best: Option<i64> = None;
    let mut open_best_d: Option<f64> = None;
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == keeper.team && !p.is_keeper {
            let mut opp_d = f64::INFINITY;
            for qp in &opp {
                opp_d = opp_d.min(qp.dist(p.pos));
            }
            if opp_d >= KEEPER_SAFE_DIST {
                // A ground lane only counts as clear when nobody stands on
                // it AND no chaser can cut the rolling ball out mid-flight;
                // a cuttable lane is floated over the interception point
                // instead.
                let f = ai::lane_blocker(keeper.pos, p.pos, &opp, POSSESS_DIST)
                    .or_else(|| pass_risk(keeper.pos, p.pos, &threats, tune));
                // Prefer a clear ground lane, then a short safe range, then
                // openness and a little upfield progress. The clear-lane
                // bonus dominates so a reliable ground pass always beats a
                // risky lob.
                let score = (if f.is_some() { 0.0 } else { 1000.0 }) - keeper.pos.dist(p.pos)
                    + opp_d * 0.5
                    + (p.pos.x - keeper.pos.x) * fwd * 0.2;
                if best_score.is_none_or(|bs| score > bs) {
                    best_score = Some(score);
                    best = Some(idx);
                    best_f = f;
                }
            }
            // Tier-2 fallback candidate: the most-open teammate overall.
            if opp_d >= THROW_MIN_OPEN && open_best_d.is_none_or(|obd| opp_d > obd) {
                open_best_d = Some(opp_d);
                open_best = Some(idx);
            }
        }
    }

    // `lofted` mirrors the old release choice: a blocked or risky lane went
    // over the top; a clear lane (or the tier-2 fallback, which never had a
    // clear-lane guarantee) stayed grounded only when nothing at all stood
    // in the way. The tier-2 fallback is treated as lofted, matching its
    // pre-#531 flight (it always resolved through a computed arc fraction).
    let (target, lofted) = if let Some(best) = best {
        (Some(best), best_f.is_some())
    } else if let Some(open_best) = open_best {
        (Some(open_best), true)
    } else {
        (None, false)
    };

    if let Some(target) = target {
        let target_pos = s.players[(target - 1) as usize].pos;
        let target_charge = desired_pass_charge(keeper.pos.dist(target_pos), tune);
        let p = &mut s.players[(keeper_idx - 1) as usize];
        p.pass_intent = pass_intent::commit(&p.pass_intent, target, lofted, target_charge);
    } else {
        keeper_dropkick_clearance(s, &keeper, fwd);
    }
}

/// Everyone swarmed: drop-kick a high clearance upfield (lands around
/// `DROPKICK_DIST` away, toward the middle of the pitch). Unchanged by
/// #531: this is a clearance, not a pass, and never called `release_pass`.
/// Whether `target_idx` sits at least `KEEPER_SAFE_DIST` from every
/// opponent -- the same hard gate `commit_keeper_pass_intent`'s own tier-1
/// scan applies before a candidate is even scored. `select_throw_target`
/// has no equivalent gate of its own: opponent proximity there is only a
/// mild score bonus (`open.clamp(0.0, 100.0) / 40.0`), never an exclusion,
/// so a caller that skips the scan and goes straight to the cone can hand
/// the ball to a covered teammate the scan would have refused. Used by the
/// human keeper's forced-release rule, which has no scan of its own to
/// inherit this from.
fn keeper_throw_target_is_safe(s: &MatchState, keeper_team: Team, target_idx: i64) -> bool {
    let target_pos = s.players[(target_idx - 1) as usize].pos;
    s.players
        .iter()
        .filter(|q| q.team != keeper_team)
        .map(|q| q.pos.dist(target_pos))
        .fold(f64::INFINITY, f64::min)
        >= KEEPER_SAFE_DIST
}

fn keeper_dropkick_clearance(s: &mut MatchState, keeper: &MatchPlayer, fwd: f64) {
    let tx = (keeper.pos.x + fwd * DROPKICK_DIST).clamp(40.0, s.field.w - 40.0);
    let target = Vec2::new(tx, s.field.h / 2.0);
    s.events.push(MatchEvent {
        kind: MatchEventKind::Shot,
        x: s.ball.x,
        y: s.ball.y,
        player: Some(keeper.id.clone()),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    set_owner(s, None);
    s.ball_z = 0.0;
    s.ball_spin = 0.0;
    s.pickup_cd = RELEASE_CD;
    s.block_grace = BLOCK_GRACE;
    let (vel, vz) = lob_launch(keeper.pos, target, 0.5, DROPKICK_CLEAR_H);
    s.ball_vel = vel;
    s.ball_vz = vz;
}

/// Keeper distribution, driven by an ordinary `MatchInput` (#531; formerly
/// `human_keeper_actions` — it is no longer human-exclusive, and this is the
/// SAME function a charging AI keeper's synthesized input runs through, from
/// `execute_pass_intent_tick`). For a human: Space (hold + release) is a
/// charged PUNT off the foot — the longer the hold, the further upfield it
/// sails. K (hold + release) is a charged THROW: the range picks which
/// teammate along your aim receives it. With the ball at the FEET (a
/// received back-pass) the throw becomes a normal outfield-style pass.
///
/// The trailing six-second-rule fallback below is HUMAN-ONLY: it is the
/// shot clock that fires when a human never holds anything at all, not a
/// second AI decision path. An AI keeper's own distribution is decided
/// entirely by `commit_keeper_pass_intent`, gated on its own (much shorter)
/// `hold_timer` at the call site in `update_ball` — see that dispatch for
/// why this function itself must not re-run it.
fn keeper_actions(s: &mut MatchState, dt: f64, input: &MatchInput, owner_idx: i64, tune: &Tuning) {
    // A full meter releases on its own (predictable); early release fires
    // at the current charge.
    let mut fire_shot = input.shoot;
    let mut fire_pass = input.pass;
    {
        let owner = &mut s.players[(owner_idx - 1) as usize];
        if input.shoot_held {
            owner.charge = (owner.charge + tune.value("CHARGE_RATE") * dt).min(1.0);
            owner.pass_target = None;
            fire_shot = fire_shot || owner.charge >= 1.0;
        } else if input.pass_held {
            owner.pass_charge = (owner.pass_charge + tune.value("PASS_CHARGE_RATE") * dt).min(1.0);
            fire_pass = fire_pass || owner.pass_charge >= 1.0;
        }
    }
    let owner_windup_timer = s.players[(owner_idx - 1) as usize].windup_timer;
    if fire_shot && owner_windup_timer == 0.0 {
        let owner = s.players[(owner_idx - 1) as usize].clone();
        let dist = PUNT_MIN + owner.charge * (tune.value("PUNT_MAX") - PUNT_MIN);
        let mut dir = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
            input.r#move.normalized()
        } else {
            owner.facing
        };
        if dir.x == 0.0 && dir.y == 0.0 {
            dir = Vec2::new(if owner.team == Team::Home { 1.0 } else { -1.0 }, 0.0);
        }
        let tgt = owner.pos.add(dir.scale(dist));
        let tgt = Vec2::new(
            tgt.x.clamp(40.0, s.field.w - 40.0),
            tgt.y.clamp(40.0, s.field.h - 40.0),
        );
        // Parameters captured at commit; ball releases after the wind-up.
        let (vel, vz) = lob_launch(owner.pos, tgt, 0.5, PUNT_CLEAR_H);
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.charge = 0.0;
        owner.pass_target = None;
        owner.windup_timer = tune.value("SHOT_WINDUP");
        owner.windup_shot = Some(WindupShot {
            dir: vel.normalized(),
            speed: vel.length(),
            vz,
            spin: 0.0,
            shot_type: keeper::KeeperShotType::Ground,
        });
    } else if fire_pass {
        let owner = s.players[(owner_idx - 1) as usize].clone();
        let aim = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
            input.r#move.normalized()
        } else {
            owner.facing
        };
        if owner.feet_ball {
            try_pass(s, owner_idx, input.lob, Some(aim), tune);
        } else {
            let range = pass_range_min(tune)
                + owner.pass_charge * (tune.value("PASS_RANGE_MAX") - pass_range_min(tune));
            keeper_throw(s, owner_idx, range, Some(aim), tune);
        }
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.pass_charge = 0.0;
        owner.pass_target = None;
    }
    // The six-second rule (#531 acceptance criterion 1's documented
    // carve-out): a HUMAN keeper who never held pass or shoot at all still
    // has to release before `hold_timer` runs out, or play stalls forever.
    // This used to re-run `keeper_distribute`'s full AI scoring stack on the
    // human's behalf — handing a stalled human the AI's target choice, not
    // "your own pass, forced." It now fires the same verb `fire_pass` above
    // would have fired, at whatever charge and aim already exist (usually
    // none: the whole point of this clock is a human who never held
    // anything), through `keeper_throw` — one of `release_pass`'s two
    // blessed callers, the same one a real charged throw uses. Gated to a
    // human owner: an AI keeper's distribution is entirely
    // `commit_keeper_pass_intent`'s, and it never arms this `hold_timer`
    // value to begin with (it arms its own, much shorter one — see the
    // call site in `update_ball`), so this guard is a correctness
    // statement, not just a belt-and-braces check.
    //
    // `commit_keeper_pass_intent`'s own scan refuses a covered teammate
    // outright (`KEEPER_SAFE_DIST`, checked before a candidate is even
    // scored); `select_throw_target`'s cone has no such gate of its own —
    // opponent proximity there is only a mild score bonus. A forced release
    // that skipped straight to the cone inherited none of that scan's
    // safety net, so it's re-applied here explicitly
    // (`keeper_throw_target_is_safe`): thrown when the cone's own pick
    // clears `KEEPER_SAFE_DIST`, cleared upfield exactly like the AI's own
    // everyone's-covered case otherwise. This is exercised, not
    // theoretical — `tests/match.rs`'s
    // `the_keeper_builds_out_without_losing_the_ball_to_the_opponent`
    // parks a human on the keeper specifically to drive this path, and
    // failed on it (handing the ball straight to an opponent) before this
    // safety check existed.
    let owner = &s.players[(owner_idx - 1) as usize];
    if is_human_player(s, owner_idx)
        && s.owner == Some(owner_idx)
        && !owner.feet_ball
        && owner.hold_timer <= 0.0
        && owner.windup_timer == 0.0
    {
        let owner = s.players[(owner_idx - 1) as usize].clone();
        let aim = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
            input.r#move.normalized()
        } else {
            owner.facing
        };
        let range = pass_range_min(tune)
            + owner.pass_charge * (tune.value("PASS_RANGE_MAX") - pass_range_min(tune));
        let target = select_throw_target(s, owner_idx, range, Some(aim));
        let safe = target.is_some_and(|t| keeper_throw_target_is_safe(s, owner.team, t));
        if safe {
            keeper_throw(s, owner_idx, range, Some(aim), tune);
        } else {
            let fwd = if owner.team == Team::Home { 1.0 } else { -1.0 };
            keeper_dropkick_clearance(s, &owner, fwd);
        }
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.pass_charge = 0.0;
        owner.pass_target = None;
    }
}

/// Fire the actual dive lunge: aim at the freshest prediction of where the
/// shot crosses the keeper's line. The movement clamps to `dive_target`
/// (see `move_offball_keeper`), so a near-straight shot is a small step,
/// not a full-speed 100px lunge past the ball.
fn launch_dive(s: &mut MatchState, keeper_idx: i64) {
    let mut y_cross = s.ball.y;
    if s.ball_vel.x != 0.0 {
        let keeper_pos_x = s.players[(keeper_idx - 1) as usize].pos.x;
        let t = (keeper_pos_x - s.ball.x) / s.ball_vel.x;
        if t > 0.0 {
            y_cross = s.ball.y + s.ball_vel.y * t;
        }
    }
    let keeper = &mut s.players[(keeper_idx - 1) as usize];
    keeper.dive_timer = KEEPER_DIVE_DURATION;
    keeper.dive_target = Some(Vec2::new(keeper.pos.x, y_cross));
    let to_cross = keeper.dive_target.expect("just set").sub(keeper.pos);
    keeper.dive_dir = if to_cross.length() > 1.0 {
        to_cross.normalized()
    } else {
        Vec2::new(0.0, 0.0)
    };
}

/// End a dive and hand the keeper to its get-up recovery.
///
/// THE ONLY DIVE-END TRANSITION, and the only place `keeper_get_up_timer` is
/// armed — `gc-render`'s `frame::drawn_facing` rests on that, so keep it true.
/// Two things reach it, and a dive reaches exactly one of them because this
/// zeroes `dive_timer`: the lunge window running out (`step`'s timer sweep),
/// and the keeper taking the ball.
///
/// THE SECOND CALLER IS THE POINT (#450). `dive_timer` used to outlive the
/// catch that ended the dive, and the gap was not cosmetic. For as long as it
/// ran, `move_offball_keeper`'s dive branch owned the keeper's `pos` and
/// `facing` on every tick the keeper was not the owner — so the instant it
/// released the ball, a dive from BEFORE the save dragged it back toward a
/// stale `dive_target` and pointed `facing` along the way.
/// `select_throw_target` defaults its aim cone to `facing`, so which teammate
/// received the NEXT distribution was decided by a dive that had already been
/// caught. A keeper who has caught the ball is not diving any more; ending the
/// dive at the moment of possession cuts that coupling in the state machine
/// rather than at either of its symptoms.
///
/// Every site that hands a keeper the ball calls it — the completed catch, the
/// smother, the loose-ball gather — each guarded by `dive_timer > 0.0`, so a
/// keeper that was not diving is never handed a get-up window it did not earn.
/// A kickoff resets the whole field outright and needs no call. A QUEUED dive
/// (`dive_delay`) needs no equivalent either: it only fires on an inbound ball,
/// and a held ball has zero velocity.
///
fn end_dive(p: &mut MatchPlayer) {
    p.dive_timer = 0.0;
    p.dive_target = None;
    p.save_style = None;
    p.save_tip_emitted = false;
    // The lunge is over: the keeper is on the floor and pushes back up before
    // any ready posture reads as truthful again.
    p.keeper_get_up_timer = KEEPER_GET_UP_POSE;
}

/// One `attempt_save` candidate evaluation: what the deleted gravity-only
/// quadratic would have decided, beside what the predictor-backed code
/// actually decided, for the #490 save-rate investigation.
///
/// This is a diagnostic value, not a gameplay one — nothing in `step`'s
/// normal path reads it. It exists so the classifier
/// `tests/keeper_shadow_classifier.rs` drives can re-derive the exact
/// candidate/deferred/disagree counts from the real `attempt_save` logic,
/// rather than a second, hand-maintained implementation of its gating that
/// could silently drift from the real one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SaveShadowObservation {
    /// What `s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz` (the
    /// deleted formula) would have decided, at the same `tz` the real
    /// decision uses.
    pub old_on_target: bool,
    /// Whether the real predictor query resolved at all
    /// (`position_at_time` returned `Some`) — `false` means this candidate
    /// deferred rather than committing or refusing.
    pub new_resolved: bool,
    /// What the real, predictor-backed code decided. Only meaningful when
    /// `new_resolved` is `true`; `false` (not `None`) when the query
    /// deferred, so a caller that ignores `new_resolved` still gets a safe
    /// "not on target" reading rather than a stale `true`.
    pub new_on_target: bool,
    /// The team the shot threatens (i.e. NOT the keeper's own team).
    pub attacker: Team,
}

thread_local! {
    /// `None` (the default) means no test has subscribed: `attempt_save`
    /// skips the shadow computation entirely, so this costs nothing and
    /// changes nothing in the path the sim-correctness review already
    /// covered. `Some` accumulates observations for whichever test called
    /// [`shadow_observations_begin`].
    static SAVE_SHADOW: std::cell::RefCell<Option<Vec<SaveShadowObservation>>> =const {
        std::cell::RefCell::new(None)
    };
}

/// Start recording [`SaveShadowObservation`]s on this thread. Diagnostic
/// only — see that type's doc comment.
pub fn shadow_observations_begin() {
    SAVE_SHADOW.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
}

/// Stop recording and return everything captured since
/// [`shadow_observations_begin`], leaving recording off.
pub fn shadow_observations_take() -> Vec<SaveShadowObservation> {
    SAVE_SHADOW.with(|cell| cell.borrow_mut().take().unwrap_or_default())
}

/// What one pass release did, for the two registered passing metrics.
///
/// A **diagnostic tally**, on exactly the seam
/// [`SaveShadowObservation`]/`SAVE_SHADOW` established (#490, `4c6d1eb`):
/// thread-local, `None` by default, so a normal step spends nothing and
/// behaves identically. It is deliberately NOT a `MatchState` field and
/// deliberately NOT a `MatchEvent` field — either would enter the snapshot,
/// the state hash and the wire layout, and a measurement must not be able to
/// change what it measures.
///
/// # Why these two numbers exist at all
///
/// #491 registers eleven passing knobs, and a 48-seed census measured every
/// one of them against every one of the NINE metrics that existed before this
/// struct: all DECORATION. That is #488's finding repeating itself for a
/// different subsystem — the pre-existing metrics are whole-match OUTCOMES,
/// and a selection or leading change reaches an outcome only through many
/// layers of AI decision-making. Two structural facts make the dilution
/// worse here than it was for locomotion:
///
/// - **Soft-cone selection runs for ONE player.** `select_pass_target` is
///   reached only from the human/bot-driven input path; the match AI picks
///   its own receiver through `outfield_decision` and never consults the
///   cone. So `PASS_ANGULAR_WEIGHT` moves at most a tenth of the passes in
///   an AI-vs-AI batch.
/// - **A led pass and an unled pass mostly complete anyway.** Leading changes
///   *where* the ball meets the receiver, not usually *whether*.
///
/// AGENTS.md §9 is unambiguous that a knob which cannot move a metric fails
/// review, and #488's answer to the identical problem was to register the
/// metric that resolves (`time_to_reverse`). These are that, for passing:
/// each measures the thing its knobs actually do, at dozens of events per
/// match instead of one match outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassShadowTally {
    /// Releases that went through aimed, soft-cone selection.
    pub aimed_releases: i64,
    /// Sum of those releases' aim error, in chord (see `gc_sim::passing`).
    pub aim_error_sum: f64,
    /// Driven ground passes released (the ones the lead solver runs for).
    pub ground_releases: i64,
    /// Sum of those releases' lead times in seconds, counting an unled pass
    /// as exactly zero — an unled pass IS a lead of nothing, and treating it
    /// as absent would make the mean say "how long are the leads we chose to
    /// play" instead of "how far into the run do passes go".
    pub lead_time_sum: f64,
    /// Every `release_pass` call, from any of the four call sites and any
    /// producer — the denominator for asking what fraction of releases are
    /// ground releases (see `ground_releases`'s doc comment for the caveat
    /// that a solved lead can still be discarded into a lob afterward).
    pub total_releases: i64,
}

thread_local! {
    /// `None` (the default) means nobody subscribed, and every recording
    /// site below short-circuits.
    static PASS_SHADOW: std::cell::RefCell<Option<PassShadowTally>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Start tallying pass releases on this thread. Diagnostic only.
pub fn pass_shadow_begin() {
    PASS_SHADOW.with(|cell| *cell.borrow_mut() = Some(PassShadowTally::default()));
}

/// Stop tallying and return what was captured, leaving recording off.
pub fn pass_shadow_take() -> PassShadowTally {
    PASS_SHADOW.with(|cell| cell.borrow_mut().take().unwrap_or_default())
}

fn pass_shadow_record<F: FnOnce(&mut PassShadowTally)>(f: F) {
    PASS_SHADOW.with(|cell| {
        if let Some(tally) = cell.borrow_mut().as_mut() {
            f(tally);
        }
    });
}

/// The keeper of the threatened goal COMMITS against an on-target shot: it
/// picks its verdict now (catch / parry / beaten — pure and deterministic,
/// a function of reach, handling, pace and angle), but the ball is NOT
/// touched: it keeps flying its real trajectory and the save completes on
/// contact in [`resolve_pending_save`]. The dive itself is QUEUED
/// (`dive_delay`) so the lunge lands when the ball does — committing early
/// must not mean diving early. So a shot always visibly travels from the
/// shooter's boot to the keeper's glove — no teleports, no gloves closing
/// on empty air.
///
/// The contact HEIGHT that decides on-target comes from
/// [`crate::ball_prediction::BallPredictor::position_at_time`], an
/// authoritative query against the shared prediction service — never the
/// closed-form ballistic fallback ([`crate::ball_prediction::BallEstimate`]),
/// which this function cannot even reach: the two types don't convert (#486).
/// A shot whose arrival falls outside the service's horizon/budget defers
/// the commit to a later tick rather than guessing.
#[allow(clippy::too_many_lines)]
fn attempt_save(s: &mut MatchState, tune: &Tuning) {
    let speed = s.ball_vel.length();
    if speed < 1.0 || s.ball_vel.x == 0.0 {
        return; // a dead or purely-vertical ball is not an on-target shot
    }
    // One predictor for the whole call, not one per keeper: at most one
    // keeper's "toward this goal" gate is ever true for a given ball
    // velocity, but sharing the instance means a second candidate's query
    // (same ball, same tick) reuses the first's buffer instead of re-running
    // scratch ticks the cache already has (#486).
    let mut predictor = BallPredictor::default();
    for ki in 0..s.players.len() {
        let keeper_idx = (ki + 1) as i64;
        let keeper = s.players[ki].clone();
        if !(keeper.is_keeper
            && keeper.receive_timer <= 0.0 // a teammate's back-pass is RECEIVED (feet), never saved
            && keeper.dive_timer <= 0.0
            && keeper.dive_delay <= 0.0
            && keeper.save_pending.is_none())
        {
            continue;
        }
        let goal = if keeper.team == Team::Home {
            s.goal_home
        } else {
            s.goal_away
        };
        // Same quirk as `toward_goal`/`inbound` above — for the home team
        // this reduces to `(ball_vel.x < 0) or (ball_vel.x > 0)`, true for
        // ANY nonzero ball velocity, not a per-team ternary. Keep this
        // expression exactly as written: see the `toward_goal` comment in
        // `move_offball_keeper` for the full derivation. This one matters in
        // practice — a shot that has already bounced (ball_vel.x flipped
        // positive) by the time `attempt_save` runs still reads as "toward"
        // the home goal under this expression's actual semantics, which is
        // what commits the keeper to the save this function exists to
        // resolve.
        let toward = (keeper.team == Team::Home && s.ball_vel.x < 0.0) || s.ball_vel.x > 0.0;
        // Time for the ball to reach the keeper's line. Must be ahead of
        // the ball (keeper between ball and goal) and close enough that the
        // keeper commits.
        let t = (keeper.pos.x - s.ball.x) / s.ball_vel.x;
        if !(toward && t >= 0.0 && (keeper.pos.x - s.ball.x).abs() <= SAVE_ZONE) {
            continue;
        }
        let y_cross = s.ball.y + s.ball_vel.y * t; // where it crosses the keeper's line
        let plane_x = if keeper.team == Team::Home {
            goal.x + goal.w
        } else {
            goal.x
        };
        let tg = (plane_x - s.ball.x) / s.ball_vel.x;
        let y_goal = s.ball.y + s.ball_vel.y * tg; // where it crosses the goal plane
        // Height of the shot when it reaches the keeper's line. A ball over
        // the keeper's aerial reach (a chip) sails past it; over the bar
        // isn't a save either. When the ball ACTUALLY arrives, friction
        // included. dist/speed lies for slow shots (they decelerate the
        // whole way), and a dying ball never arrives at all — that one is
        // claimed off the grass by the normal keeper logic, never dived at.
        let dxa = (keeper.pos.x - s.ball.x).abs();
        let x_frac = speed / s.ball_vel.x.abs(); // path px per x px
        let k_fric = if s.ball_z > 0.5 || s.ball_vz > 0.0 {
            AIR_FRICTION
        } else {
            FRICTION
        };
        let eta = keeper::travel_time(dxa * x_frac, speed, k_fric);
        // The dive is timed to GLOVE CONTACT (hands' radius short of the
        // line), not the line itself — so the lunge window covers the
        // moment the save actually resolves.
        let eta_contact =
            keeper::travel_time((dxa - KEEPER_HANDS).max(0.0) * x_frac, speed, k_fric);
        // Height when it reaches the keeper's line, at the real arrival time
        // (the geometric t is fine for y — friction shrinks both velocity
        // components equally, so the path stays straight). The height is
        // NOT fine to solve by hand: a gravity-only quadratic ignores drag
        // and the ground bounce, so it can place a shot under the bar (or
        // over it) that the real stepped trajectory would not — the exact
        // failure mode #486 exists to close. Query the shared prediction
        // service instead: it steps the ball through the same
        // `ball_flight::step` the live sim runs, so this answer cannot
        // drift from what the ball will actually do.
        let tz = eta.unwrap_or(t);
        // Diagnostic only (#490): computed and recorded ONLY when a test has
        // called `shadow_observations_begin`, so the normal `step` path
        // (SAVE_SHADOW always `None`) never spends this work and never sees
        // a behavior difference from the code the sim-correctness review
        // covered.
        let shadow_recording = SAVE_SHADOW.with(|cell| cell.borrow().is_some());
        let old_on_target_shadow = shadow_recording && {
            let old_z = s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz;
            y_goal >= goal.y - SAVE_PAD
                && y_goal <= goal.y + goal.h + SAVE_PAD
                && old_z < CROSSBAR
                && old_z <= KEEPER_AIR_GRAB
        };
        let attacker_shadow = if keeper.team == Team::Home {
            Team::Away
        } else {
            Team::Home
        };
        let Some(sample) = predictor.position_at_time(s, tz) else {
            // The real trajectory doesn't resolve inside the service's
            // horizon/budget — only possible for a shot so slow `tz` runs
            // past `predict.max_horizon` (2s). Conservative and deliberate:
            // do not commit this tick rather than guess with the old
            // gravity-only formula. `attempt_save` runs every live tick and
            // `tz` only shrinks as the ball closes in, so a genuine
            // on-target shot still resolves on a later tick, well before it
            // reaches the line — this defers the commit, it never causes a
            // miss. See `tests/keeper_prediction.rs`'s
            // `a_query_that_cannot_resolve_inside_the_horizon_defers_the_commit_instead_of_guessing`.
            if shadow_recording {
                SAVE_SHADOW.with(|cell| {
                    if let Some(obs) = cell.borrow_mut().as_mut() {
                        obs.push(SaveShadowObservation {
                            old_on_target: old_on_target_shadow,
                            new_resolved: false,
                            new_on_target: false,
                            attacker: attacker_shadow,
                        });
                    }
                });
            }
            continue;
        };
        let z_cross = sample.z;
        let on_target = y_goal >= goal.y - SAVE_PAD
            && y_goal <= goal.y + goal.h + SAVE_PAD
            && z_cross < CROSSBAR
            && z_cross <= KEEPER_AIR_GRAB;
        if shadow_recording {
            SAVE_SHADOW.with(|cell| {
                if let Some(obs) = cell.borrow_mut().as_mut() {
                    obs.push(SaveShadowObservation {
                        old_on_target: old_on_target_shadow,
                        new_resolved: true,
                        new_on_target: on_target,
                        attacker: attacker_shadow,
                    });
                }
            });
        }
        // How far the keeper has to dive along its line to reach the shot.
        let dive_dist = (keeper.pos.y - y_cross).abs();
        let block_reach = species::block_reach(keeper.owned_verb);
        // Physical save eligibility already includes the species
        // block-reach seam. Use that same effective reach for style and tip
        // geometry; catch/parry quality deliberately keeps its existing
        // base-reach math.
        let effective_reach = keeper.reach + block_reach;
        let reaction_reach = keeper::reaction_reach(
            effective_reach,
            keeper.keeper_release_motion,
            KEEPER_DIVE_DURATION,
        );
        if on_target && let Some(eta_value) = eta {
            // Any pre-release set has now reached the real save-processing
            // seam. The existing verdict and contact-timed dive own the
            // presentation from here.
            s.players[ki].keeper_set = 0.0;
            let dist_to_keeper = keeper.pos.dist(s.ball);
            let save_style = if keeper::in_smother_range(dist_to_keeper) {
                None
            } else {
                Some(keeper::save_style(
                    dist_to_keeper,
                    dive_dist,
                    effective_reach,
                ))
            };

            if dive_dist > reaction_reach {
                if dive_dist > effective_reach
                    && dive_dist <= effective_reach * 1.1
                    && !keeper.save_tip_emitted
                {
                    s.players[ki].save_tip_emitted = true;
                    s.events.push(MatchEvent {
                        kind: MatchEventKind::Tip,
                        x: keeper.pos.x,
                        y: y_cross,
                        player: Some(keeper.id.clone()),
                        save_style: None,
                        style: None,
                        outcome: None,
                        jumping: None,
                        difficulty: None,
                        shot_type: None,
                        keeper_state: keeper.keeper_release_state,
                        keeper_depth: Some(keeper.keeper_release_depth),
                        on_target: None,
                    });
                }
                return;
            }

            s.players[ki].save_style = save_style;
            // Queue the dive so the lunge window covers the arrival: a shot
            // still half a second out gets a set keeper first, then a dive
            // that meets the ball, not one that finished while the shot was
            // still traveling.
            let dive_delay = (eta_contact.unwrap_or(eta_value) - KEEPER_DIVE_DURATION).max(0.0);
            s.players[ki].dive_delay = dive_delay;
            if dive_delay == 0.0 {
                launch_dive(s, keeper_idx);
            }

            // Closeness of the dive + handling, minus pace. A shot straight
            // at the keeper (dive_dist ~ 0) is gathered even when hard;
            // only wide or blistering shots drop to a parry or beat the
            // keeper.
            let quality = (1.0 - dive_dist / keeper.reach) + keeper.handling * HANDLING_WEIGHT
                - speed / tune.value("SAVE_SPEED_REF");

            if quality >= PARRY_QUALITY {
                // Grab or parry: probabilistic, from the match's seeded
                // RNG. The catch odds are a logistic curve over quality —
                // soft and central sticks in the gloves, hot or at full
                // stretch usually gets pushed away.
                let p_catch = 1.0
                    / (1.0
                        + deterministic_math::exp(
                            -(quality - tune.value("CATCH_EVEN_QUALITY")) / CATCH_SOFTNESS,
                        ));
                let (next_rng, sample) = rng::roll(s.rng);
                s.rng = next_rng;
                let keeper = &mut s.players[ki];
                keeper.save_pending = Some(if sample < p_catch {
                    crate::match_snapshot::SavePending::Catch
                } else {
                    crate::match_snapshot::SavePending::Parry
                });
                keeper.save_timer = eta_value + SAVE_TIMEOUT_PAD;
                keeper.save_vx = s.ball_vel.x;
            }
            // Beaten: the dive is committed but the ball flies through.
            return;
        }
    }
}

/// Complete a committed save when the ball actually arrives: at hands'
/// reach, on crossing the keeper's plane (a fast far-corner ball met right
/// at the line), or on the timeout backstop. The dive is abandoned (a
/// whiff) if the shot got deflected away in flight — a body block or
/// bounce reversing its direction.
fn resolve_pending_save(s: &mut MatchState, dt: f64) -> Option<crate::match_snapshot::SavePending> {
    use crate::match_snapshot::SavePending;
    for ki in 0..s.players.len() {
        let keeper = s.players[ki].clone();
        let Some(pend) = keeper.save_pending else {
            continue;
        };
        let keeper_idx = (ki + 1) as i64;
        s.players[ki].save_timer -= dt;
        let reversed = s.ball_vel.x * keeper.save_vx <= 0.0;
        let crossed = (keeper.save_vx > 0.0 && s.ball.x >= keeper.pos.x)
            || (keeper.save_vx < 0.0 && s.ball.x <= keeper.pos.x);
        let contact = keeper.pos.dist(s.ball) <= KEEPER_HANDS;
        if reversed {
            let k = &mut s.players[ki];
            k.save_pending = None; // the shot was deflected: the dive whiffs
            k.dive_delay = 0.0; // and a still-queued lunge stays holstered
            k.save_style = None;
            k.save_tip_emitted = false;
        } else if s.ball_vel.length() < DEAD_SHOT_SPEED && !contact {
            // The shot died short of the gloves: it is a loose ball now.
            // Drop the commitment so the normal claim logic gathers it —
            // never vacuum a stationary ball across open grass.
            let k = &mut s.players[ki];
            k.save_pending = None;
            k.dive_delay = 0.0;
            k.save_style = None;
            k.save_tip_emitted = false;
        } else if contact || crossed || s.players[ki].save_timer <= 0.0 {
            let k = &mut s.players[ki];
            k.save_pending = None;
            k.dive_delay = 0.0;
            let save_style = k.save_style;
            k.save_style = None;
            k.save_tip_emitted = false;
            if pend == SavePending::Catch {
                s.events.push(MatchEvent {
                    kind: MatchEventKind::Catch,
                    x: s.ball.x,
                    y: s.ball.y,
                    player: Some(keeper.id.clone()),
                    save_style,
                    style: None,
                    outcome: None,
                    jumping: None,
                    difficulty: None,
                    shot_type: None,
                    keeper_state: keeper.keeper_release_state,
                    keeper_depth: Some(keeper.keeper_release_depth),
                    on_target: None,
                });
                s.ball = keeper_hold_pos(s, keeper_idx);
                set_owner(s, Some(keeper_idx));
                s.ball_vel = Vec2::new(0.0, 0.0);
                s.ball_z = 0.0;
                s.ball_vz = 0.0;
                s.ball_spin = 0.0;
                let k = &mut s.players[ki];
                k.grab_timer = KEEPER_GRAB_POSE;
                k.hold_timer = KEEPER_HOLD;
                k.feet_ball = false;
                if k.dive_timer > 0.0 {
                    end_dive(k); // possession ends the dive (#450)
                }
                return Some(SavePending::Catch);
            }
            // Parry from the actual contact point: punch it clear — out
            // AND up, so the deflection sails over the shooter, never
            // served into their body. Keep the ball safely outside the
            // goal line.
            s.events.push(MatchEvent {
                kind: MatchEventKind::Parry,
                x: s.ball.x,
                y: s.ball.y,
                player: Some(keeper.id.clone()),
                save_style,
                style: None,
                outcome: None,
                jumping: None,
                difficulty: None,
                shot_type: None,
                keeper_state: keeper.keeper_release_state,
                keeper_depth: Some(keeper.keeper_release_depth),
                on_target: None,
            });
            let goal = if keeper.team == Team::Home {
                s.goal_home
            } else {
                s.goal_away
            };
            let mut bx = s.ball.x;
            if keeper.team == Team::Home {
                bx = bx.max(goal.x + goal.w + BALL_RADIUS + 1.0);
            } else {
                bx = bx.min(goal.x - BALL_RADIUS - 1.0);
            }
            s.ball = Vec2::new(bx, s.ball.y);
            let gc = Vec2::new(goal.x + goal.w / 2.0, goal.y + goal.h / 2.0);
            let mut dir = s.ball.sub(gc).normalized();
            if dir.x == 0.0 && dir.y == 0.0 {
                dir = keeper.facing;
            }
            let speed = s.ball_vel.length();
            s.ball_vel = dir.scale((speed * PARRY_SPEED_MULT).max(MIN_PARRY_CLEAR));
            s.ball_vz = PARRY_POP_VZ;
            s.ball_spin = 0.0;
            s.pickup_cd = PARRY_CD;
            s.block_grace = BLOCK_GRACE;
            return Some(SavePending::Parry);
        }
    }
    None
}

/// Recompute `pass_target` for an outfield carrier holding the pass button
/// — a human's real input or a charging AI's synthesized one (#531). Pure:
/// no RNG draws. Safe to call every frame while `pass_held` is true.
fn update_pass_target_outfield(
    s: &mut MatchState,
    owner_idx: i64,
    input: &MatchInput,
    tune: &Tuning,
) {
    let owner = s.players[(owner_idx - 1) as usize].clone();
    let range = if owner.pass_charge > 0.12 {
        Some(
            pass_range_min(tune)
                + owner.pass_charge * (tune.value("PASS_RANGE_MAX") - pass_range_min(tune)),
        )
    } else {
        None
    };
    let aim = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
        Some(input.r#move.normalized())
    } else {
        None
    };
    let target = select_pass_target(s, owner_idx, input.lob, aim, range, tune);
    s.players[(owner_idx - 1) as usize].pass_target = target;
}

/// Recompute `pass_target` for a keeper holding the pass button — a human's
/// real input or a charging AI keeper's synthesized one (#531). Pure: no RNG
/// draws. Safe to call every frame while `pass_held` is true.
fn update_pass_target_keeper(
    s: &mut MatchState,
    keeper_idx: i64,
    input: &MatchInput,
    tune: &Tuning,
) {
    let keeper = s.players[(keeper_idx - 1) as usize].clone();
    let range = pass_range_min(tune)
        + keeper.pass_charge * (tune.value("PASS_RANGE_MAX") - pass_range_min(tune));
    let aim = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
        Some(input.r#move.normalized())
    } else {
        None
    };
    let target = select_throw_target(s, keeper_idx, range, aim);
    s.players[(keeper_idx - 1) as usize].pass_target = target;
}

/// Preview the keeper's pending distribution: ball at the feet previews like
/// an outfielder's pass; in the hands, like a throw. Shared by a human's
/// real input and a charging AI keeper's synthesized one (#531) — this used
/// to be inlined at `update_ball`'s human-only dispatch branch.
fn update_keeper_pass_preview(
    s: &mut MatchState,
    owner_idx: i64,
    input: &MatchInput,
    tune: &Tuning,
) {
    if input.pass_held {
        if s.players[(owner_idx - 1) as usize].feet_ball {
            update_pass_target_outfield(s, owner_idx, input, tune);
        } else {
            update_pass_target_keeper(s, owner_idx, input, tune);
        }
    } else {
        s.players[(owner_idx - 1) as usize].pass_target = None;
    }
}

/// Outfield controlled shot commit: build charge and, on fire, store a
/// wind-up payload (parameters captured now, released after
/// `TUNE.SHOT_WINDUP` seconds). Driven by an ordinary `MatchInput` (#531;
/// formerly `human_outfield_actions` — it is no longer human-exclusive: the
/// SAME function a charging AI's synthesized input runs through, from
/// `execute_pass_intent_tick`).
fn outfield_actions(
    s: &mut MatchState,
    dt: f64,
    input: &MatchInput,
    owner_idx: i64,
    tune: &Tuning,
) {
    let mut fire_shot = input.shoot;
    let mut fire_pass = input.pass;
    if input.shoot_held {
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.charge = (owner.charge + tune.value("CHARGE_RATE") * dt).min(1.0);
        fire_shot = fire_shot || owner.charge >= 1.0;
        owner.pass_target = None;
    } else if input.pass_held {
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.pass_charge = (owner.pass_charge + tune.value("PASS_CHARGE_RATE") * dt).min(1.0);
        fire_pass = fire_pass || owner.pass_charge >= 1.0;
        // Preview: recompute intended receiver every frame (pure, no RNG).
        update_pass_target_outfield(s, owner_idx, input, tune);
    } else {
        s.players[(owner_idx - 1) as usize].pass_target = None;
    }
    if fire_shot {
        // Aim at the goal; vertical of `facing` picks the corner. Charge
        // (held shoot) scales power; lateral input bends the shot.
        // Parameters are CAPTURED NOW and released after the wind-up.
        let owner = s.players[(owner_idx - 1) as usize].clone();
        let vbias = (owner.facing.y * 1.4).clamp(-1.0, 1.0);
        let speed = owner.shot_speed * (1.0 + owner.charge * CHARGE_POWER);
        let target = shot_target(s, owner.team, vbias);
        let mut vz = 0.0;
        let shot_type = if input.lob {
            keeper::KeeperShotType::Chip
        } else {
            keeper::KeeperShotType::Ground
        };
        if input.lob {
            // Human chip intent is locked now, like direction and power. A
            // feasible chip clears the keeper's current committed plane; an
            // infeasible request remains a deterministic poor chip rather
            // than silently becoming a ground-shot decoy during the
            // wind-up.
            let defending_team = opposite(owner.team);
            let defending_keeper_idx =
                team_keeper(s, defending_team).expect("defending team has a keeper");
            let defending_keeper_pos = s.players[(defending_keeper_idx - 1) as usize].pos;
            let goal = if defending_team == Team::Home {
                s.goal_home
            } else {
                s.goal_away
            };
            vz = keeper::committed_chip_launch(&keeper::KeeperChipContext {
                origin: owner.pos,
                target,
                keeper_pos: defending_keeper_pos,
                defending_team: to_keeper_team(defending_team),
                goal: to_keeper_rect(goal),
                horizontal_speed: speed,
                friction: AIR_FRICTION,
                gravity: GRAVITY,
                keeper_clearance: KEEPER_AIR_GRAB,
                crossbar: CROSSBAR,
                desired_goal_height: CHIP_LINE_Z,
            });
        }
        let side = if input.r#move.x > 0.0 {
            1.0
        } else if input.r#move.x < 0.0 {
            -1.0
        } else {
            0.0
        };
        let spin = if shot_type == keeper::KeeperShotType::Ground {
            side * owner.charge * CURVE_MAX
        } else {
            0.0
        };
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.charge = 0.0;
        owner.pass_target = None;
        owner.windup_timer = tune.value("SHOT_WINDUP");
        owner.windup_shot = Some(WindupShot {
            dir: target.sub(owner.pos),
            speed,
            vz,
            spin,
            shot_type,
        });
    } else if fire_pass {
        let aim = if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
            Some(input.r#move.normalized())
        } else {
            None
        };
        try_pass(s, owner_idx, input.lob, aim, tune);
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.pass_charge = 0.0;
        owner.pass_target = None;
    } else if !(input.shoot_held || input.pass_held) {
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.charge = 0.0;
        // Belt and braces alongside `charge`: `pass_target` is already
        // cleared above whenever `pass_held` is false, but `pass_charge`
        // had no neutral-tick reset at all, so any edge that lost its
        // release (see `combat::prepare_inputs`'s suppression path) left it
        // latched forever with possession retained. A fully neutral tick —
        // no press, no hold, on either button — is the one moment with no
        // charge in flight to protect, so it is always safe to clear here.
        owner.pass_charge = 0.0;
        owner.pass_target = None;
    }
}

/// Commit a charging pass or cross intent toward `target_player` for an AI
/// outfield carrier (#531). The verb's own scorer
/// (`ai_pass_options`/`ai_cross_target`, still unchanged) already picked
/// `target_player`; this only converts that pick into an aim hint and a
/// charge threshold — the soft cone, not this commit, decides who actually
/// receives the ball at release (see the design note on `pass_intent`).
fn commit_outfield_pass_intent(
    s: &mut MatchState,
    owner_idx: i64,
    target_player: i64,
    lofted: bool,
    tune: &Tuning,
) {
    let owner_pos = s.players[(owner_idx - 1) as usize].pos;
    let target_pos = s.players[(target_player - 1) as usize].pos;
    let target_charge = desired_pass_charge(owner_pos.dist(target_pos), tune);
    let p = &mut s.players[(owner_idx - 1) as usize];
    p.pass_intent = pass_intent::commit(&p.pass_intent, target_player, lofted, target_charge);
}

/// The gameplay AI's pass-intent equipment channel: materializes the
/// abstract `r#move`/`pass_held`/`pass`/`lob` signals a charging pass intent
/// owes this tick into an ordinary `MatchInput` (#531) — the direct
/// analogue of `combat_equipment_input` for the passing seam. The aim is
/// read LIVE from the intent's target player's current position every tick,
/// exactly as a human's stick direction is read live, falling back to
/// `facing` only in the degenerate case where the target coincides with the
/// owner.
fn ai_pass_input(
    s: &MatchState,
    owner_idx: i64,
    intent: &pass_intent::PassIntentState,
    signals: pass_intent::PassIntentSignals,
) -> MatchInput {
    let owner = &s.players[(owner_idx - 1) as usize];
    let aim = intent
        .target_player
        .map(|target_idx| {
            pass_intent::aim_toward(
                owner.pos,
                s.players[(target_idx - 1) as usize].pos,
                owner.facing,
            )
        })
        .unwrap_or(owner.facing);
    let mut input = slot_input::neutral_match_input();
    input.r#move = aim;
    input.pass_held = signals.pass_held;
    input.pass = signals.pass;
    input.lob = intent.lofted;
    input
}

/// Advance one owed tick of `owner_idx`'s charging pass intent and return
/// the `MatchInput` it materializes (#531). The caller still routes that
/// input through the same preview/action functions a human's input takes —
/// this only produces the input, it never executes anything itself.
fn execute_pass_intent_tick(s: &mut MatchState, owner_idx: i64) -> MatchInput {
    let intent = pass_intent::copy_state(&s.players[(owner_idx - 1) as usize].pass_intent);
    let current_charge = s.players[(owner_idx - 1) as usize].pass_charge;
    let (signals, next_intent) = pass_intent::materialize(&intent, current_charge);
    s.players[(owner_idx - 1) as usize].pass_intent = next_intent;
    ai_pass_input(s, owner_idx, &intent, signals)
}

/// AI outfield owner decision: build the legitimate scored option set
/// without RNG, then select only when this player's personal cadence
/// refreshes.
fn carrier_forward_space(s: &MatchState, owner_idx: i64, tune: &Tuning) -> f64 {
    let owner = &s.players[(owner_idx - 1) as usize];
    let attack_x = if owner.team == Team::Home { 1.0 } else { -1.0 };
    let max_route_distance = tune.value("AI_SPRINT_SPACE").max(1.0);
    let mut route_distance = max_route_distance;
    let route_half_width = POSSESS_DIST * 2.0;
    for opponent in &s.players {
        if opponent.team != owner.team && !opponent.is_keeper {
            let forward = (opponent.pos.x - owner.pos.x) * attack_x;
            let lateral = (opponent.pos.y - owner.pos.y).abs();
            if forward > 0.0 && lateral < route_half_width {
                let edge_fraction = lateral / route_half_width;
                let centered_distance = max_route_distance.min(forward);
                let effective_distance =
                    centered_distance + (max_route_distance - centered_distance) * edge_fraction;
                route_distance = route_distance.min(effective_distance);
            }
        }
    }
    (route_distance / max_route_distance).min(1.0)
}

#[allow(clippy::too_many_lines)]
fn ai_outfield_decision(s: &mut MatchState, owner_idx: i64, tune: &Tuning) {
    let owner = s.players[(owner_idx - 1) as usize].clone();
    if !outfield_decision::should_refresh(
        &owner.outfield_decision,
        OutfieldDecisionContext::Carrier,
        None,
    ) {
        return;
    }

    let g = attack_goal(s, owner.team);
    let gc = Vec2::new(
        if owner.team == Team::Home {
            g.x
        } else {
            g.x + g.w
        },
        g.y + g.h / 2.0,
    );
    let keeper_player_idx = team_keeper(s, opposite(owner.team));
    let mut space = s.field.w;
    for opponent in &s.players {
        if opponent.team != owner.team && !opponent.is_keeper {
            space = space.min(opponent.pos.dist(owner.pos));
        }
    }

    let attack_depth = if owner.team == Team::Home {
        owner.pos.x / s.field.w
    } else {
        (s.field.w - owner.pos.x) / s.field.w
    };
    let width = (owner.pos.y - s.field.h / 2.0).abs() / (s.field.h / 2.0);
    let third = attack_depth > 0.62;
    let wide = (owner.pos.y - s.field.h / 2.0).abs() > 130.0;
    let (mut cross_target, mut box_targets) = ai_cross_target(s, owner_idx);
    if !third || !wide || owner.settle_timer > 0.0 || space <= tune.value("CROSS_MIN_SPACE") {
        cross_target = None;
        box_targets = 0;
    }
    let passes = if owner.settle_timer <= 0.0 {
        ai_pass_options(s, owner_idx, tune)
    } else {
        Vec::new()
    };
    let mut keeper_coverage = 0.0;
    if let Some(kpi) = keeper_player_idx {
        let kpos = s.players[(kpi - 1) as usize].pos;
        keeper_coverage = 1.0 - (1.0f64).min((kpos.y - gc.y).abs() / g.h.max(1.0));
    }
    let options = outfield_decision::carrier_options(&outfield_decision::OutfieldCarrierContext {
        goal_distance: owner.pos.dist(gc),
        shoot_range: tune.value("AI_SHOOT_RANGE"),
        angle_quality: 1.0 - (1.0f64).min((owner.pos.y - gc.y).abs() / (s.field.h / 2.0)),
        keeper_coverage,
        space: (1.0f64).min(space / AI_CHARGE_SPACE_RANGE),
        flank_depth: (1.0f64).min((0.0f64).max((attack_depth - 0.5) / 0.3))
            * (1.0f64).min(width / 0.7),
        cross_target: cross_target.map(|t| t as u32),
        box_targets: box_targets.max(0) as u32,
        cross_space: (1.0f64).min(space / (tune.value("CROSS_MIN_SPACE") * 2.0).max(1.0)),
        goal_progress: attack_depth,
        dribble_space: carrier_forward_space(s, owner_idx, tune),
        passes,
    });
    let (selected, next_decision_rng) = outfield_decision::decide_carrier(
        &options,
        owner.composure,
        1.0 - (1.0f64).min(space / tune.value("AI_PASS_PRESSURE").max(1.0)),
        owner.outfield_decision.rng_state,
    );
    {
        let d = &mut s.players[(owner_idx - 1) as usize].outfield_decision;
        *d = outfield_decision::with_rng_state(d, next_decision_rng);
    }

    let scan_rate = owner.scan_rate;
    if selected.kind == "pass" || selected.kind == "cross" {
        let target_player = match selected.reference {
            Some(brain::OptionReference::Index(v)) => v,
            _ => panic!("carrier target player must be numeric"),
        };
        let d = &mut s.players[(owner_idx - 1) as usize].outfield_decision;
        *d = outfield_decision::refresh(
            d,
            OutfieldDecisionContext::Carrier,
            if selected.kind == "pass" {
                outfield_decision::OutfieldIntent::Pass
            } else {
                outfield_decision::OutfieldIntent::Cross
            },
            scan_rate,
            None,
            None,
            Some(target_player),
            None,
        );
    } else if selected.kind == "shoot" {
        let d = &mut s.players[(owner_idx - 1) as usize].outfield_decision;
        *d = outfield_decision::refresh(
            d,
            OutfieldDecisionContext::Carrier,
            outfield_decision::OutfieldIntent::Shoot,
            scan_rate,
            None,
            None,
            None,
            None,
        );
    } else {
        assert!(selected.kind == "dribble", "unknown carrier option");
        let d = &mut s.players[(owner_idx - 1) as usize].outfield_decision;
        *d = outfield_decision::refresh(
            d,
            OutfieldDecisionContext::Carrier,
            outfield_decision::OutfieldIntent::Dribble,
            scan_rate,
            None,
            None,
            None,
            None,
        );
    }

    if selected.kind == "pass" {
        // The verb (pass) and the target (`ai_pass_options`'s pick) are
        // still this scorer's own call. What used to happen next —
        // releasing immediately through `release_pass`, skipping charge,
        // hold and the soft cone entirely — is #531's defect. The target
        // becomes an aim hint instead: `commit_outfield_pass_intent` charges
        // toward it, and the cone (`select_pass_target`, reached through
        // `try_pass` from `outfield_actions`) decides who actually receives
        // the ball once that charge is spent, exactly as it does for a
        // human. `lane_fraction` no longer has a caller here — the blocker
        // fraction it fed is now recomputed live, at release time, by
        // `try_pass`/`release_pass`'s own dink-over-a-blocker check.
        let target_player = match selected.reference {
            Some(brain::OptionReference::Index(v)) => v as i64,
            _ => panic!("carrier target player must be numeric"),
        };
        commit_outfield_pass_intent(s, owner_idx, target_player, false, tune);
    } else if selected.kind == "cross" {
        // Same seam, lofted: a cross intent charges toward the box target
        // `ai_cross_target` picked, and `try_pass`'s own lofted branch
        // recomputes the arc/blocker at release instead of the fixed
        // `Some(0.5)`/`CROSS_CLEAR_H` this used to hand `release_pass`
        // directly.
        let target_player = match selected.reference {
            Some(brain::OptionReference::Index(v)) => v as i64,
            _ => panic!("carrier target player must be numeric"),
        };
        commit_outfield_pass_intent(s, owner_idx, target_player, true, tune);
    } else if selected.kind == "shoot" {
        // Shoot to the corner away from the defending keeper, with power
        // scaled by the space the striker has been given (see constants).
        let mut vbias = 0.85;
        if let Some(kpi) = keeper_player_idx {
            let kpos_y = s.players[(kpi - 1) as usize].pos.y;
            vbias = if kpos_y < gc.y { 0.85 } else { -0.85 };
        }
        let frac = ((space - AI_CHARGE_MIN_SPACE) / AI_CHARGE_SPACE_RANGE).clamp(0.0, 1.0);
        let owner = s.players[(owner_idx - 1) as usize].clone();
        let speed = owner.shot_speed * (1.0 + frac * CHARGE_POWER);
        let shot_end = shot_target(s, owner.team, vbias);
        let sdir = shot_end.sub(owner.pos);
        let mut vz = 0.0;
        if let Some(kpi) = keeper_player_idx
            && owner.settle_timer <= 0.0
            && owner.pos.dist(s.ball) <= DRIBBLE_TOUCH_REACH
        {
            let kpos = s.players[(kpi - 1) as usize].pos;
            let kteam = s.players[(kpi - 1) as usize].team;
            if keeper::chip_is_visible(kpos, to_keeper_team(kteam), to_keeper_rect(g)) {
                vz = keeper::chip_launch(&keeper::KeeperChipContext {
                    origin: owner.pos,
                    target: shot_end,
                    keeper_pos: kpos,
                    defending_team: to_keeper_team(kteam),
                    goal: to_keeper_rect(g),
                    horizontal_speed: speed,
                    friction: AIR_FRICTION,
                    gravity: GRAVITY,
                    keeper_clearance: KEEPER_AIR_GRAB,
                    crossbar: CROSSBAR,
                    desired_goal_height: CHIP_LINE_Z,
                })
                .unwrap_or(0.0);
            }
        }
        let owner = &mut s.players[(owner_idx - 1) as usize];
        owner.windup_timer = tune.value("SHOT_WINDUP");
        owner.windup_shot = Some(WindupShot {
            dir: sdir,
            speed,
            vz,
            spin: 0.0,
            shot_type: if vz > 0.0 {
                keeper::KeeperShotType::Chip
            } else {
                keeper::KeeperShotType::Ground
            },
        });
    } else {
        // Dribble is the retained carrier intent; movement continues
        // through the established touch, sprint and juke paths until the
        // next refresh.
        assert!(
            selected.kind == "dribble",
            "carrier selection did not resolve"
        );
    }
}

/// The gameplay AI's equipment-intent channel.
///
/// Gameplay-AI outfielders steer by mutating `MatchState`; this
/// materializes an ordinary `MatchInput` carrying only the three abstract
/// equipment signals, so the same commit gate, the same
/// `suppress_soccer_actions` arbitration, and the same keeper protection
/// that govern a human govern them too.
///
/// Slots are untouched: `is_human_player` is true for every fixed slot in
/// slot mode and for the human-controlled player otherwise, and any index
/// that already carries a supplied row is skipped. Keepers are
/// combat-disabled.
fn ai_combat_inputs(
    s: &mut MatchState,
    combat_state: &mut CombatMatchState,
    inputs: &mut IndexMap<i64, MatchInput>,
) {
    for index in 0..s.players.len() {
        let idx = (index + 1) as i64;
        if combat_state.players.get(index).is_none() {
            continue;
        }
        let player = s.players[index].clone();
        if player.is_keeper || is_human_player(s, idx) || inputs.contains_key(&idx) {
            continue;
        }

        if combat_state.players[index].forced_ticks > 0
            && combat_state.players[index].intent.stage != combat_intent::CombatIntentStage::Idle
        {
            // A landed hit cancelled the action this materialization was
            // serving. Do not carry its release edge into a new situation.
            combat_state.players[index].intent =
                combat_intent::reset(&combat_state.players[index].intent);
        }

        let (signals, next_intent) =
            combat_intent::materialize(&combat_state.players[index].intent);
        combat_state.players[index].intent = next_intent;
        if let Some(signals) = signals {
            inputs.insert(idx, combat_equipment_input(signals));
        } else if combat_state.players[index].family_id.is_some()
            && combat_state.players[index].phase == combat_feasibility::CombatActionPhase::Ready
            && combat_state.players[index].forced_ticks == 0
            && combat_state.players[index].cooldown_ticks == 0
            && s.kickoff_hold <= 0.0
            && !s.finished
            && combat_intent::should_decide(combat_state.tick, idx, player.scan_rate)
        {
            let observation = combat_observation::build(
                s,
                Some(combat_state),
                idx,
                combat_policy::POLICY_ID,
                None,
            );
            // Composure sharpens the choice exactly as it does for carrier
            // decisions: a settled player takes the argmax and spends no
            // RNG.
            let temperature = if player.composure >= combat_policy::SHARP_COMPOSURE {
                0.0
            } else {
                combat_policy::BASE_TEMPERATURE * (1.0 - player.composure)
            };
            let decision = combat_policy::decide(
                &observation,
                combat_intent::decision_seed(combat_state.tick, idx),
                Some(temperature),
            );
            if decision.action == combat_policy::PolicyAction::Commit {
                assert!(
                    !decision.context_violation,
                    "representative_policy_context_violation: gameplay_ai/combat/v1 committed \
                     without a purpose context"
                );
                let reason = decision.reason;
                assert!(
                    combat_policy::is_commit_reason(reason),
                    "AI combat commit requires a stable reason"
                );
                let target_index = decision
                    .target_player
                    .expect("commit decision has a target");
                let hold_ticks = combat_policy::hold_ticks(&observation, &decision);
                combat_state.players[index].intent = combat_intent::commit(
                    &combat_state.players[index].intent,
                    reason,
                    target_index,
                    hold_ticks,
                );
                inputs.insert(idx, combat_equipment_input(combat_intent::commit_signals()));
                let commit_kind = combat_reason_to_event_kind(reason);
                s.events.push(MatchEvent {
                    kind: commit_kind,
                    x: player.pos.x,
                    y: player.pos.y,
                    player: Some(player.id.clone()),
                    save_style: None,
                    style: None,
                    outcome: None,
                    jumping: None,
                    difficulty: None,
                    shot_type: None,
                    keeper_state: None,
                    keeper_depth: None,
                    on_target: None,
                });
            } else {
                let decline_action = match decision.action {
                    combat_policy::PolicyAction::Decline => {
                        Some(combat_intent::DeclineAction::Decline)
                    }
                    combat_policy::PolicyAction::Unavailable => {
                        Some(combat_intent::DeclineAction::Unavailable)
                    }
                    combat_policy::PolicyAction::Commit => unreachable!("handled above"),
                };
                combat_state.players[index].intent =
                    combat_intent::decline(&combat_state.players[index].intent, decline_action);
                // Not committing is the spacing answer: hold the ground the
                // formation wants. A telegraphed threat that is genuinely
                // about to land gets the existing sidestep instead.
                //
                // This runs for `unavailable` as well as `decline`. Evading
                // is a soccer primitive, not an equipment request, so a
                // player with no loadout or no reachable target still has
                // to be allowed to step out of an incoming projectile. The
                // evade's own gate covers the states where a sidestep is
                // impossible.
                ai_combat_evade(s, idx, &observation);
            }
        }
    }
}

/// One-shot equipment-only `MatchInput` for a combat intent materialization.
fn combat_equipment_input(signals: combat_intent::CombatIntentSignals) -> MatchInput {
    let mut input = slot_input::neutral_match_input();
    input.equipment_held = signals.equipment_held;
    input.equipment_pressed = signals.equipment_pressed;
    input.equipment_released = signals.equipment_released;
    input
}

/// Sidestep a telegraphed threat that is genuinely about to land. This
/// reuses the existing juke primitive and its authored constants; it adds
/// no new steering. `sim::combat` skips a dodging body when it selects
/// melee and projectile targets, so the sidestep is the mechanical evasion,
/// not a pose.
fn ai_combat_evade(
    s: &mut MatchState,
    player_idx: i64,
    observation: &combat_observation::CombatObservation,
) {
    let player = s.players[(player_idx - 1) as usize].clone();
    if player.dodge_cd > 0.0
        || player.dodge_timer > 0.0
        || player.slide_timer > 0.0
        || player.aerial_recovery > 0.0
        || player.stun_timer > 0.0
    {
        return;
    }
    let feasibility_observation = combat_observation::to_feasibility_view(observation);
    let Some(threat) = combat_feasibility::incoming_threat(
        &feasibility_observation,
        combat_policy::EVADE_WINDOW_TICKS,
    ) else {
        return;
    };
    if threat.ticks_to_contact < combat_policy::EVADE_MIN_LEAD_TICKS {
        return;
    }
    // Step away from the THREAT, not from the player who launched it. For
    // melee the two coincide; for a projectile already in flight the
    // shooter can be anywhere, including directly behind the body it is
    // about to hit, and reading the shooter would sidestep straight into
    // the shot.
    let away = player.pos.sub(Vec2::new(threat.threat_x, threat.threat_y));
    let mut perp = Vec2::new(-player.facing.y, player.facing.x);
    if away.x * perp.x + away.y * perp.y < 0.0 {
        perp = perp.scale(-1.0);
    }
    let p = &mut s.players[(player_idx - 1) as usize];
    p.dodge_timer = DODGE_DURATION;
    p.dodge_cd = DODGE_CD;
    p.dodge_dir = perp;
    let (px, py, pid) = (p.pos.x, p.pos.y, p.id.clone());
    s.events.push(MatchEvent {
        kind: MatchEventKind::Juke,
        x: px,
        y: py,
        player: Some(pid),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
}

// ---------------------------------------------------------------------
// Aerial glue
// ---------------------------------------------------------------------
//
// `crate::aerial` now adopts this module's canonical `MatchState`/
// `MatchPlayer`/`MatchInput`/`MatchEvent` directly (see that module's doc),
// so no view conversion happens at this boundary any more;
// [`aerial_resolve_play`] only builds the 0-based input
// slice `aerial::resolve_play` expects and forwards `s` by mutable
// reference.

/// Resolve this frame's aerial play via [`crate::aerial::resolve_play`].
/// Returns whether the ball's trajectory changed.
fn aerial_resolve_play(
    s: &mut MatchState,
    inputs: &IndexMap<i64, MatchInput>,
    ineligible: Option<&[bool]>,
    tune: &Tuning,
) -> bool {
    let aerial_inputs: Vec<Option<MatchInput>> = (0..s.players.len())
        .map(|i| inputs.get(&((i + 1) as i64)).copied())
        .collect();
    let config = aerial::AerialMatchConfig {
        ground_grab_height: GROUND_GRAB_HEIGHT,
        stick_ahead: STICK_AHEAD,
        gravity: GRAVITY,
        release_cd: RELEASE_CD,
        clear_header_speed: CLEAR_HEADER_SPEED,
        volley_speed: VOLLEY_SPEED,
    };
    aerial::resolve_play(s, &aerial_inputs, &config, ineligible, tune)
}

#[allow(clippy::too_many_lines)]
fn update_ball(
    s: &mut MatchState,
    dt: f64,
    inputs: &IndexMap<i64, MatchInput>,
    combat_state: Option<&CombatMatchState>,
    tune: &Tuning,
) {
    // Controller transients belong to the ball owner. Losing possession
    // cancels that player's charge, preview, and any committed wind-up; a
    // later possession can never inherit stale state from another slot.
    //
    // This used to also fire for an AI-owned ball every tick regardless of
    // possession (`!is_human_player(s, idx)` on its own was always true for
    // the AI), which is why AI charge could never persist across ticks
    // (#531): the AI's own decision code ran, set `pass_charge`, and the
    // very next tick's sweep here zeroed it again before anything read it.
    // Gating on possession alone — exactly the human rule — is what lets an
    // AI-driven charge survive from one tick to the next.
    for index in 0..s.players.len() {
        let idx = (index + 1) as i64;
        if Some(idx) != s.owner {
            let p = &mut s.players[index];
            p.charge = 0.0;
            p.pass_charge = 0.0;
            p.pass_target = None;
            p.pass_intent = pass_intent::reset(&p.pass_intent);
        }
        if s.slot_mode && Some(idx) != s.owner && s.players[index].windup_shot.is_some() {
            let p = &mut s.players[index];
            p.windup_timer = 0.0;
            p.windup_shot = None;
        }
    }

    // Wind-up resolution: a player whose timer just hit 0 and still owns
    // the ball fires the stored shot payload. If they lost possession
    // during the wind-up (tackle, smother) the payload was already cleared
    // in `attempt_steals`.
    if let Some(owner_idx) = s.owner {
        let wowner = s.players[(owner_idx - 1) as usize].clone();
        if wowner.windup_timer == 0.0
            && let Some(ws) = wowner.windup_shot
        {
            s.players[(owner_idx - 1) as usize].windup_shot = None;
            release_shot(
                s,
                owner_idx,
                ws.dir,
                Some(ws.speed),
                Some(ws.vz),
                Some(ws.shot_type),
            );
            s.ball_spin = ws.spin;
            // Keeper punt gets a throw pose; outfield shot doesn't (handled
            // below).
            if wowner.is_keeper {
                s.players[(owner_idx - 1) as usize].throw_timer = KEEPER_THROW_POSE;
            }
            return;
        }
    }

    if let Some(owner_idx) = s.owner {
        // An owned ball invalidates any committed save still waiting on
        // contact.
        for q in &mut s.players {
            q.save_pending = None;
            q.save_style = None;
            q.save_tip_emitted = false;
        }
        let owner = s.players[(owner_idx - 1) as usize].clone();
        let neutral = MatchInput::default();
        let input = *inputs.get(&owner_idx).unwrap_or(&neutral);
        if owner.is_keeper && !owner.feet_ball {
            // A keeper holds the ball in its hands, clamped clear of its
            // own line.
            s.ball = keeper_hold_pos(s, owner_idx);
            s.ball_vel = Vec2::new(0.0, 0.0);
            s.ball_z = 0.0; // an owned ball is grounded (at feet / in hands)
            s.ball_vz = 0.0;
        } else {
            // Touch-based dribble, DISCRETE (see the constants block):
            // kick, chase, kick again. Between touches the ball runs free
            // under grass friction; each new touch is a visible, audible
            // kick ahead of the run with skill-scaled direction/weight
            // error. Push one past the (skill-scaled) control radius and
            // possession breaks — a heavy touch, robbed.
            let skill = owner.dribble;
            // REALIZED speed (actual motion), not run_vel: a carrier
            // body-checked to a stop must not keep pushing the ball at the
            // pace their legs are asking for — the ball rides what the
            // body DOES.
            let speed = owner.vel.length();
            let at_feet = owner.pos.dist(s.ball) <= DRIBBLE_TOUCH_REACH;
            s.ball_z = 0.0;
            s.ball_vz = 0.0;
            if !at_feet {
                // The ball is away from the feet: it rolls free — the
                // PLAYER goes to the BALL (the hook in move_players), never
                // the other way around.
                s.ball_vel = s.ball_vel.scale((1.0 - FRICTION * dt).max(0.0));
            } else if speed < owner.move_speed * tune.value("DRIBBLE_CLOSE") {
                // CLOSE CONTROL (standing through an ordinary jog): the
                // ball stays glued to the feet with soft corrective
                // touches — natural, safe, nothing knocked away. Sprinting
                // breaks into the kick-and-chase below.
                let rest = owner.pos.add(owner.facing.scale(DRIBBLE_LEAD_MIN));
                let correct = rest
                    .sub(s.ball)
                    .scale(tune.value("DRIBBLE_TOUCH") * (0.5 + 0.5 * skill));
                s.ball_vel = owner.vel.add(correct);
            } else if s.ball_vel.length() <= speed + DRIBBLE_CATCH_PACE {
                // The ball has slowed back to the feet: play the next
                // touch — a kick ahead of the run, struck harder than the
                // carrier moves so it runs on and returns. Sloppier feet
                // (low skill) spray the angle and the weight; the seeded
                // rolls keep the sim reproducible.
                let (next_rng, roll_a) = rng::roll(s.rng);
                s.rng = next_rng;
                let (next_rng, roll_w) = rng::roll(s.rng);
                s.rng = next_rng;
                let slop = 1.0 - DRIBBLE_ERR_SKILL * skill;
                let ang = (roll_a * 2.0 - 1.0) * tune.value("DRIBBLE_ERR") * slop;
                let (ca, sa) = deterministic_math::cos_sin(ang);
                let dir = Vec2::new(
                    owner.facing.x * ca - owner.facing.y * sa,
                    owner.facing.x * sa + owner.facing.y * ca,
                );
                let weight = 1.0 + (roll_w * 2.0 - 1.0) * tune.value("DRIBBLE_ERR") * 0.8 * slop;
                s.ball_vel = dir.scale(speed * tune.value("DRIBBLE_PUSH") * weight);
                s.events.push(MatchEvent {
                    kind: MatchEventKind::Touch,
                    x: s.ball.x,
                    y: s.ball.y,
                    player: Some(owner.id.clone()),
                    save_style: None,
                    style: None,
                    outcome: None,
                    jumping: None,
                    difficulty: None,
                    shot_type: None,
                    keeper_state: None,
                    keeper_depth: None,
                    on_target: None,
                });
            } else {
                // At the feet but still leaving the boot (just kicked): let
                // it run, shedding pace on the grass.
                s.ball_vel = s.ball_vel.scale((1.0 - FRICTION * dt).max(0.0));
            }
            s.ball = s.ball.add(s.ball_vel.scale(dt));
            let control = tune.value("DRIBBLE_CONTROL") + DRIBBLE_CONTROL_SKILL * skill;
            if owner.pos.dist(s.ball) > control {
                // Clear at the ownership-loss transition so this tick is
                // self-contained: reacquisition cannot revive stale input
                // state.
                let owner_mut = &mut s.players[(owner_idx - 1) as usize];
                owner_mut.charge = 0.0;
                owner_mut.pass_charge = 0.0;
                owner_mut.pass_target = None;
                owner_mut.pass_intent = pass_intent::reset(&owner_mut.pass_intent);
                // The fixed-slot contract requires same-tick wind-up
                // cancellation. Legacy match AI keeps its tripwire-pinned
                // heavy-touch behavior at the explicit offline boundary.
                if s.slot_mode {
                    owner_mut.windup_timer = 0.0;
                    owner_mut.windup_shot = None;
                }
                for player in &mut s.players {
                    player.keeper_set = 0.0;
                }
                // The touch got away from the feet: it's loose now.
                set_owner(s, None);
                return; // no owner actions this frame; the ball plays loose next
            }
        }

        if owner.is_keeper {
            if is_human_player(s, owner_idx) {
                // Preview: while pass_held, show which teammate would
                // receive. Ball at the feet passes like an outfielder; in
                // the hands it throws.
                update_keeper_pass_preview(s, owner_idx, &input, tune);
                keeper_actions(s, dt, &input, owner_idx, tune);
            } else {
                // AI keeper (#531): survey and commit a target once, then
                // charge toward it and execute through the same
                // `MatchInput` path a human keeper's throw takes. Two-phase
                // so deciding and the tick's first materialization fuse
                // into one tick: if idle, `hold_timer` gates the "survey,
                // then distribute" delay exactly as it always did (build
                // from the back instead of hoofing it upfield every frame);
                // once charging, materialize/execute run every tick
                // regardless of `hold_timer` (it stays expired for the rest
                // of a continuous hold, so this is not a re-gate).
                if s.players[(owner_idx - 1) as usize].pass_intent.stage
                    == pass_intent::PassIntentStage::Idle
                {
                    s.players[(owner_idx - 1) as usize].pass_target = None;
                    if owner.hold_timer <= 0.0 {
                        commit_keeper_pass_intent(s, owner_idx, tune);
                    }
                }
                if s.players[(owner_idx - 1) as usize].pass_intent.stage
                    == pass_intent::PassIntentStage::Charging
                {
                    let ai_input = execute_pass_intent_tick(s, owner_idx);
                    update_keeper_pass_preview(s, owner_idx, &ai_input, tune);
                    keeper_actions(s, dt, &ai_input, owner_idx, tune);
                }
            }
        } else if is_human_player(s, owner_idx) {
            // A full meter LETS FLY on its own (predictable, like the
            // meter promises); release fires early at the current charge.
            // During wind-up: inputs are locked out (shot is committed,
            // params already captured), so skip all action logic this
            // frame.
            if owner.windup_timer == 0.0
                && !combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), owner_idx))
            {
                outfield_actions(s, dt, &input, owner_idx, tune);
            }
        } else if owner.windup_timer == 0.0
            && owner.stun_timer <= 0.0
            && !combat_state.is_some_and(|cs| combat::blocks_actions(Some(cs), owner_idx))
        {
            // AI owner (#531): decide what to do (shoot/cross/pass/carry)
            // only while idle on a pass/cross intent, then, unconditionally,
            // if charging, materialize and execute through the same
            // `MatchInput` path a human's input takes. Shoot and dribble are
            // unaffected — shoot already commits a wind-up payload
            // (`windup_timer`/`windup_shot`), and dribble is not a
            // multi-tick commitment at all.
            //
            // DISCLOSED PRODUCER ASYMMETRY: `owner.stun_timer <= 0.0` predates
            // #531 and used to gate only the (single-tick, no persisted
            // state) call to `ai_outfield_decision` -- a stunned AI simply
            // skipped one decision. `outfield_actions` (the human branch
            // just above) has never had an equivalent stun check at all. Now
            // that a charging `pass_intent` survives across ticks, the two
            // producers diverge under stun: a stunned AI carrier's charge
            // FREEZES (this whole branch, including the "if Charging,
            // materialize" arm, is skipped while `stun_timer > 0.0`), while
            // an equally-stunned human's `pass_charge` keeps accumulating
            // through `outfield_actions` regardless. This costs the AI MORE
            // hold time, not less, so it is not exploitable, but it is a
            // real producer asymmetry this PR's own thesis is to remove, and
            // it is reachable: a still-in-possession player can be stunned
            // by the non-dispossessing slide-tackle body collision (see the
            // stun assignment near the slide-collision handling above).
            // Left as a disclosed asymmetry rather than fixed here — closing
            // it means deciding whether stun should also freeze a human's
            // charge (a second behavioural change) or never freeze the AI's
            // (losing a pre-existing AI-only protection), and that decision
            // belongs with review, not a silent choice in this diff.
            if s.players[(owner_idx - 1) as usize].pass_intent.stage
                == pass_intent::PassIntentStage::Idle
            {
                ai_outfield_decision(s, owner_idx, tune);
            }
            if s.players[(owner_idx - 1) as usize].pass_intent.stage
                == pass_intent::PassIntentStage::Charging
            {
                let ai_input = execute_pass_intent_tick(s, owner_idx);
                outfield_actions(s, dt, &ai_input, owner_idx, tune);
            }
        }

        // Keep a possessed ball on the pitch. See `update_ball`'s doc
        // comment above — the clamp region is the ARENA, not the pitch, and
        // reflects the outward pace exactly like the loose-ball walls below.
        let mut min_x = 0.0;
        let mut max_x = s.field.w;
        if s.ball.x < 0.0 && in_mouth(s.ball, s.goal_home) {
            min_x = s.goal_home.x;
        } else if s.ball.x > s.field.w && in_mouth(s.ball, s.goal_away) {
            max_x = s.goal_away.x + s.goal_away.w;
        }
        let cx = s.ball.x.clamp(min_x, max_x);
        let cy = s.ball.y.clamp(0.0, s.field.h);
        if cx != s.ball.x || cy != s.ball.y {
            if cx != s.ball.x {
                s.ball_vel.x = -s.ball_vel.x;
            }
            if cy != s.ball.y {
                s.ball_vel.y = -s.ball_vel.y;
            }
            s.ball = Vec2::new(cx, cy);
        }
        return;
    }

    // Loose ball: integrate, decay, curve, bounce off touchlines/back
    // walls. The integration itself lives in `crate::ball_flight` so that
    // the forward-prediction service's scratch world steps THIS code rather
    // than a second copy of it (see that module's doc comment).
    let mut flight = ball_flight::BallFlight::of(s);
    let trajectory_bounced = ball_flight::step(&mut flight, &ball_flight::BallArena::of(s), dt);
    flight.write_back(s);
    if trajectory_bounced {
        for player in &mut s.players {
            player.keeper_set = 0.0;
        }
    }

    // Body blocking: a fast, low ball that runs into an outfield body
    // ricochets off it. Only a ball moving TOWARD the body blocks, so a
    // shooter never blocks their own release. Keepers are excluded — they
    // interact with the ball through saves and claims, never as a passive
    // wall.
    {
        let speed = s.ball_vel.length();
        let block_h = if s.ball_vz < 0.0 {
            BLOCK_HEIGHT_DESC
        } else {
            BLOCK_HEIGHT
        };
        if speed >= POSSESS_MAX_SPEED && s.ball_z <= block_h && s.block_grace == 0.0 {
            for i in 0..s.players.len() {
                let p = s.players[i].clone();
                // The designated receiver never walls a ball off: they let
                // it arrive and take the touch.
                if !p.is_keeper && p.receive_timer <= 0.0 {
                    let off = s.ball.sub(p.pos);
                    let d = off.length();
                    let contact = p.radius + BALL_RADIUS + species::block_reach(p.owned_verb);
                    if d < contact {
                        let n = if d > 0.0 {
                            off.normalized()
                        } else {
                            Vec2::new(1.0, 0.0)
                        };
                        let vn = s.ball_vel.x * n.x + s.ball_vel.y * n.y;
                        if vn < 0.0 {
                            s.events.push(MatchEvent {
                                kind: MatchEventKind::Block,
                                x: s.ball.x,
                                y: s.ball.y,
                                player: Some(p.id.clone()),
                                save_style: None,
                                style: None,
                                outcome: None,
                                jumping: None,
                                difficulty: None,
                                shot_type: None,
                                keeper_state: None,
                                keeper_depth: None,
                                on_target: None,
                            });
                            // Reflect off the body normal, damped, and push
                            // the ball clear so it can't re-block next
                            // frame.
                            s.ball_vel = s.ball_vel.sub(n.scale(2.0 * vn)).scale(BLOCK_DAMP);
                            s.ball = p.pos.add(n.scale(contact));
                            s.ball_spin = 0.0;
                            // The ricochet ends the pass: nobody is
                            // receiving this ball any more (a keeper's save
                            // reflexes included).
                            for q in &mut s.players {
                                q.receive_timer = 0.0;
                                q.keeper_set = 0.0;
                                q.save_style = None;
                                q.save_tip_emitted = false;
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    // Keeper save: commit against an on-target shot, then complete the
    // save when the ball actually arrives (real trajectory, no teleport).
    // NOT gated on pickup_cd: that cooldown is the SHOOTER's re-collection
    // lockout, and a close-range shot reaches the line well inside it —
    // the keeper must still be allowed to react. A resolved catch ends the
    // frame here; a parry sets pickup_cd so the deflection isn't re-grabbed
    // instantly (the parried ball travels away from goal, so it can't
    // trigger an immediate re-save); being beaten falls through to a
    // possible goal.
    attempt_save(s, tune);
    if resolve_pending_save(s, dt) == Some(crate::match_snapshot::SavePending::Catch) {
        return;
    }

    // Descending high-ball reception and first-time strikes. Geometry,
    // difficulty, and seeded quality live in `crate::aerial`.
    let aerial_ineligible: Option<Vec<bool>> = combat_state.map(|cs| {
        cs.players
            .iter()
            .map(|runtime| runtime.forced_ticks > 0)
            .collect()
    });
    let aerial_redirected = aerial_resolve_play(s, inputs, aerial_ineligible.as_deref(), tune);
    if aerial_redirected {
        // A successful first touch or strike owns a new presentation
        // trajectory. Clear style and the one-shot guard even when x
        // direction is unchanged, but preserve the pre-existing
        // verdict/dive timing: the keeper's catch/parry/beaten verdict must
        // not change from the aerial redirect.
        for player in &mut s.players {
            player.keeper_set = 0.0;
            player.save_style = None;
            player.save_tip_emitted = false;
        }
    }

    // Collection. A keeper has PRIORITY in its own box: it claims any
    // loose ball it can reach there (with its hands), beating outfielders
    // even if they are a touch closer. Otherwise the nearest eligible
    // player grabs it.
    if s.pickup_cd == 0.0 {
        let speed = s.ball_vel.length();
        let mut best: Option<i64> = None;
        let mut best_dist: Option<f64> = None;

        for (i, p) in s.players.iter().enumerate() {
            let idx = (i + 1) as i64;
            if p.is_keeper
                && combat_state.is_none_or(|cs| cs.players[i].forced_ticks == 0)
                && p.save_pending.is_none() // a committed save resolves on contact instead
                && p.receive_timer <= 0.0 // a teammate's pass is taken with the FEET below
                && in_claim_zone(s, idx)
                && s.ball_z <= KEEPER_AIR_GRAB
                && p.pos.dist(s.ball) <= KEEPER_CLAIM_DIST + species::jump_reach(p.owned_verb)
            {
                best = Some(idx);
                break;
            }
        }

        if best.is_none() {
            for (i, p) in s.players.iter().enumerate() {
                let idx = (i + 1) as i64;
                // A keeper meeting a teammate's pass traps it with an
                // outfield reach; hand grabs use the tighter keeper radius.
                let reach = if p.is_keeper && p.receive_timer <= 0.0 {
                    KEEPER_DIST
                } else {
                    POSSESS_DIST
                };
                // A ball above head height flies over everyone — not
                // collectable. The DESIGNATED receiver traps a driven pass
                // at full pace (the first touch is theirs); everyone else
                // needs it slowed down.
                let eligible = (p.is_keeper || p.receive_timer > 0.0 || speed < POSSESS_MAX_SPEED)
                    && s.ball_z <= GROUND_GRAB_HEIGHT
                    && combat_state.is_none_or(|cs| cs.players[i].forced_ticks == 0);
                let d = p.pos.dist(s.ball);
                if eligible && d <= reach && best_dist.is_none_or(|bd| d < bd) {
                    best_dist = Some(d);
                    best = Some(idx);
                }
            }
        }

        if let Some(best) = best {
            let bp = s.players[(best - 1) as usize].clone();
            for player in &mut s.players {
                player.keeper_set = 0.0;
                player.save_style = None;
                player.save_tip_emitted = false;
            }
            if bp.is_keeper && bp.receive_timer <= 0.0 {
                // A keeper gather: claim event + gather pose, then it
                // surveys/holds. Snap the ball into its hands so a claim
                // right on the line can't still register as a goal this
                // frame.
                s.events.push(MatchEvent {
                    kind: MatchEventKind::Claim,
                    x: s.ball.x,
                    y: s.ball.y,
                    player: Some(bp.id.clone()),
                    save_style: None,
                    style: None,
                    outcome: None,
                    jumping: None,
                    difficulty: None,
                    shot_type: None,
                    keeper_state: None,
                    keeper_depth: None,
                    on_target: None,
                });
                s.ball = keeper_hold_pos(s, best);
                let bpm = &mut s.players[(best - 1) as usize];
                bpm.grab_timer = KEEPER_GRAB_POSE;
                bpm.hold_timer = KEEPER_HOLD;
                bpm.feet_ball = false;
            } else {
                // A back-pass rule of sorts: a keeper receiving a
                // teammate's deliberate pass takes it with the FEET — it
                // can dribble, pass, or punt, and is tackleable like any
                // carrier.
                let bpm = &mut s.players[(best - 1) as usize];
                bpm.feet_ball = bp.is_keeper;
                if speed > 1.0 {
                    // A moving ball trapped by a player reads as a
                    // "touch"; an AI needs a beat of control before it can
                    // pass on.
                    s.events.push(MatchEvent {
                        kind: MatchEventKind::Touch,
                        x: s.ball.x,
                        y: s.ball.y,
                        player: Some(bp.id.clone()),
                        save_style: None,
                        style: None,
                        outcome: None,
                        jumping: None,
                        difficulty: None,
                        shot_type: None,
                        keeper_state: None,
                        keeper_depth: None,
                        on_target: None,
                    });
                    s.players[(best - 1) as usize].settle_timer = tune.value("CARRIER_SETTLE");
                }
            }
            set_owner(s, Some(best));
            s.ball_vel = Vec2::new(0.0, 0.0);
            s.ball_spin = 0.0;
            let best_mut = &mut s.players[(best - 1) as usize];
            if best_mut.dive_timer > 0.0 {
                end_dive(best_mut); // possession ends the dive (#450)
            }
            // Auto-switch: the human takes over whichever home outfielder
            // wins the ball (like FIFA / Mario Strikers). Keepers stay AI.
            if !s.slot_mode && s.human_controlled && bp.team == Team::Home && !bp.is_keeper {
                set_controlled_player(s, best);
            }
        }
    }
}

/// A goal per the laws of the game: the WHOLE ball crosses the goal line,
/// between the posts, under the bar — judged on the frame it crosses (edge
/// triggered on `prev_x`). A ball that sailed over the bar and drops
/// inside the net box afterwards never counts; a ball nestling in the net
/// can't re-count.
fn check_goal(s: &mut MatchState, prev_x: f64) -> Option<Team> {
    let line_a = s.goal_away.x; // right goal line: home scores crossing it
    if s.ball.x - BALL_RADIUS > line_a
        && prev_x - BALL_RADIUS <= line_a
        && in_mouth(s.ball, s.goal_away)
        && s.ball_z < CROSSBAR
    {
        s.score.home += 1;
        return Some(Team::Home);
    }
    let line_h = s.goal_home.x + s.goal_home.w; // left goal line: away scores
    if s.ball.x + BALL_RADIUS < line_h
        && prev_x + BALL_RADIUS >= line_h
        && in_mouth(s.ball, s.goal_home)
        && s.ball_z < CROSSBAR
    {
        s.score.away += 1;
        return Some(Team::Away);
    }
    None
}

/// One tick's input to [`step`]: either a legacy `MatchInput` for the
/// human-controlled player (non-slot-mode fixtures) or a full `InputFrame`
/// (slot-mode fixtures), as an explicit enum rather than a dynamically
/// typed union parameter.
pub enum StepInput<'a> {
    /// Legacy path: one player's `MatchInput`.
    Legacy(MatchInput),
    /// Slot-mode path: a complete, tick-numbered `InputFrame`.
    Frame(&'a InputFrame),
}

/// Advance the match one fixed tick.
///
/// # Panics
///
/// Panics on any invariant violation (a non-canonical tick under
/// combat/slot mode, a malformed `InputFrame`, a missing slot mapping).
#[allow(clippy::too_many_lines)]
pub fn step(
    s: &mut MatchState,
    dt: f64,
    input: StepInput<'_>,
    combat_state: Option<&mut CombatMatchState>,
    tune: &Tuning,
) {
    if s.finished {
        reset_press_states(s);
        reset_run_states(s);
        reset_transition_state(s);
        for player in &mut s.players {
            player.keeper_set = 0.0;
        }
        return;
    }
    if let Some(cs) = &combat_state {
        assert!(
            dt == fixed_clock::TICK_SECONDS,
            "combat matches require the canonical fixed tick"
        );
        if s.slot_mode {
            assert!(
                cs.tick == s.input_tick,
                "combat boundary does not match input tick"
            );
        }
    }

    let mut inputs: IndexMap<i64, MatchInput> = IndexMap::new();
    if s.slot_mode {
        // Slot mode has no legacy-input fallback. A complete,
        // tick-numbered effective InputFrame is the simulation boundary;
        // producers must materialize bots or neutral rows before calling
        // the simulation.
        let StepInput::Frame(frame) = input else {
            panic!("slot-mode match requires an InputFrame");
        };
        input_frame::validate(frame).expect("valid input frame");
        assert!(
            frame.tick == s.input_tick,
            "input frame tick does not match match state"
        );
        assert!(
            dt == fixed_clock::TICK_SECONDS,
            "slot-mode matches require the canonical fixed tick"
        );
        for index in 1..=input_frame::SLOT_COUNT {
            let player_idx =
                s.slot_players[(index - 1) as usize].expect("slot mapping is incomplete");
            inputs.insert(
                player_idx,
                slot_input::to_match_input(&frame.slots[(index - 1) as usize]),
            );
        }
        s.input_tick += 1;
    }
    let legacy_input = match input {
        StepInput::Legacy(i) => Some(i),
        StepInput::Frame(_) => None,
    };

    // Discrete events are per-frame: clear last frame's before producing
    // this one's.
    s.events.clear();
    let mut combat_state = combat_state;
    if let Some(cs) = combat_state.as_deref_mut() {
        combat::clear_events(cs);
    }

    s.time_left -= dt;
    if s.time_left <= 0.0 {
        s.time_left = 0.0;
        s.finished = true;
        reset_press_states(s);
        reset_run_states(s);
        reset_transition_state(s);
        if let Some(cs) = combat_state.as_deref_mut() {
            // Full time closes every open encounter before the boundary
            // moves, so the lifecycle owes no orphan when the match
            // becomes terminal.
            combat::terminate_open_sequences(s, cs);
            combat::advance_boundary(cs);
        }
        for player in &mut s.players {
            player.keeper_set = 0.0;
        }
        return;
    }

    if s.pickup_cd > 0.0 {
        s.pickup_cd = (s.pickup_cd - dt).max(0.0);
    }
    if s.block_grace > 0.0 {
        s.block_grace = (s.block_grace - dt).max(0.0);
    }
    if s.aerial_lock > 0.0 {
        s.aerial_lock = (s.aerial_lock - dt).max(0.0);
    }
    if s.kickoff_hold > 0.0 {
        s.kickoff_hold = (s.kickoff_hold - dt).max(0.0);
    }
    // Decay the live turnover before this tick's movement reads its
    // phases, and retire it once both tactic windows have elapsed so
    // nothing accumulates.
    {
        let windows = s.transition_windows;
        s.transition.advance(dt, windows);
    }
    for i in 0..s.players.len() {
        let idx = (i + 1) as i64;
        let p = &mut s.players[i];
        if p.dash_cd > 0.0 {
            p.dash_cd = (p.dash_cd - dt).max(0.0);
        }
        if p.dodge_cd > 0.0 {
            p.dodge_cd = (p.dodge_cd - dt).max(0.0);
        }
        if p.dodge_timer > 0.0 {
            p.dodge_timer = (p.dodge_timer - dt).max(0.0);
        }
        // Decays before the dive block so the tick that arms it below
        // still spends the full window on screen.
        if p.keeper_get_up_timer > 0.0 {
            p.keeper_get_up_timer = (p.keeper_get_up_timer - dt).max(0.0);
        }
        if p.dive_timer > 0.0 {
            p.dive_timer = (p.dive_timer - dt).max(0.0);
            if p.dive_timer == 0.0 {
                // The lunge window ran out. `end_dive` is the one dive-end
                // transition; the other way in is possession, and a dive
                // reaches exactly one of them because both zero `dive_timer`.
                end_dive(p);
            }
        }
        let mut launch_dive_now = false;
        if p.dive_delay > 0.0 {
            p.dive_delay = (p.dive_delay - dt).max(0.0);
            if p.dive_delay == 0.0 {
                // The queued dive fires — unless the shot is no longer
                // inbound (deflected away mid-flight): then the keeper
                // stays home.
                //
                // Same quirk as `toward_goal` above: for the home team this
                // is `(ball_vel.x < 0) or (ball_vel.x > 0)`, true for any
                // nonzero velocity, not a per-team ternary. Keep this
                // expression exactly as written — see the `toward_goal`
                // comment for the full derivation.
                let inbound = (p.team == Team::Home && s.ball_vel.x < 0.0) || s.ball_vel.x > 0.0;
                if inbound {
                    launch_dive_now = true;
                } else {
                    p.save_style = None;
                    p.save_tip_emitted = false;
                }
            }
        }
        if p.hold_timer > 0.0 {
            p.hold_timer = (p.hold_timer - dt).max(0.0);
        }
        if p.slide_timer > 0.0 {
            p.slide_timer = (p.slide_timer - dt).max(0.0);
        }
        // `tackle_timer` is no longer decayed here: `advance_tackle_actions`
        // (#489) is its sole writer now, mirroring the action slot's own
        // executing-phase countdown -- see that function's trailing comment.
        if p.tackle_cd > 0.0 {
            p.tackle_cd = (p.tackle_cd - dt).max(0.0);
        }
        if p.stun_timer > 0.0 {
            p.stun_timer = (p.stun_timer - dt).max(0.0);
        }
        if p.grab_timer > 0.0 {
            p.grab_timer = (p.grab_timer - dt).max(0.0);
        }
        if p.throw_timer > 0.0 {
            p.throw_timer = (p.throw_timer - dt).max(0.0);
        }
        if p.receive_timer > 0.0 {
            p.receive_timer = (p.receive_timer - dt).max(0.0);
        }
        if p.settle_timer > 0.0 {
            p.settle_timer = (p.settle_timer - dt).max(0.0);
        }
        if p.header_cd > 0.0 {
            p.header_cd = (p.header_cd - dt).max(0.0);
        }
        if p.aerial_timer > 0.0 {
            p.aerial_timer = (p.aerial_timer - dt).max(0.0);
            if p.aerial_timer == 0.0 {
                p.aerial_style = None;
                p.aerial_outcome = None;
                p.aerial_jump = 0.0;
            }
        }
        if p.aerial_recovery > 0.0 {
            p.aerial_recovery = (p.aerial_recovery - dt).max(0.0);
        }
        if p.windup_timer > 0.0 {
            p.windup_timer = (p.windup_timer - dt).max(0.0);
        }
        if p.jockey_timer > 0.0 {
            p.jockey_timer = (p.jockey_timer - dt).max(0.0);
        }
        // MatchPlayer is already mutable state. Tick the scalar countdown
        // in place instead of allocating ten short-lived decision tables
        // per frame.
        p.outfield_decision.remaining = (p.outfield_decision.remaining - dt).max(0.0);
        if launch_dive_now {
            // `launch_dive` takes `&mut MatchState`, which would alias `p`
            // (borrowed from `s.players[i]`) — call it once `p`'s borrow
            // has ended (it is not used again in this iteration).
            launch_dive(s, idx);
        }
    }

    if !s.slot_mode
        && s.human_controlled
        && legacy_input.as_ref().is_some_and(|i| i.switch)
        && !combat_state
            .as_deref()
            .is_some_and(|cs| combat::blocks_actions(Some(cs), s.controlled))
    {
        set_controlled_player(s, next_home_outfield(s, s.controlled));
    }
    if !s.slot_mode
        && let Some(li) = legacy_input
    {
        inputs.insert(s.controlled, li);
    }
    if let Some(cs) = combat_state.as_deref_mut() {
        ai_combat_inputs(s, cs, &mut inputs);
        let equipment_ineligible: IndexMap<i64, bool> = inputs
            .iter()
            .map(|(&player_index, player_input)| {
                (
                    player_index,
                    aerial_active_for_input(s, player_index, player_input),
                )
            })
            .collect();
        inputs = combat::prepare_inputs(s, cs, &inputs, Some(&equipment_ineligible));
    }

    let prev_ball_x = s.ball.x; // for edge-triggered goal-line crossing
    let prev_owner = s.owner;
    let prev_owner_team = s.owner.map(|o| s.players[(o - 1) as usize].team);
    move_players(s, dt, &inputs, combat_state.as_deref(), tune);
    let combat_contacts = combat_state
        .as_deref_mut()
        .map(|cs| combat::collect_contacts(s, cs));
    attempt_steals(s, combat_state.as_deref());
    advance_tackle_actions(s, combat_state.as_deref(), dt, tune);
    if let (Some(cs), Some(contacts)) = (combat_state.as_deref_mut(), combat_contacts) {
        combat::resolve_contacts(s, cs, &contacts);
    }
    update_ball(s, dt, &inputs, combat_state.as_deref(), tune);
    if s.owner != prev_owner {
        if let Some(po) = prev_owner
            && !s.players[(po - 1) as usize].is_keeper
        {
            let d = &mut s.players[(po - 1) as usize].outfield_decision;
            *d = outfield_decision::reset(d);
        }
        if let Some(oi) = s.owner
            && !s.players[(oi - 1) as usize].is_keeper
        {
            let d = &mut s.players[(oi - 1) as usize].outfield_decision;
            *d = outfield_decision::reset(d);
        }
    }
    if let Some(cs) = combat_state.as_deref_mut() {
        combat::sanitize_forced_players(s, cs);
        for index in 0..s.players.len() {
            let idx = (index + 1) as i64;
            if !s.players[index].is_keeper
                && combat::blocks_actions(Some(cs), idx)
                && s.players[index].outfield_decision.context != OutfieldDecisionContext::Ineligible
            {
                let d = &mut s.players[index].outfield_decision;
                *d = outfield_decision::reset(d);
            }
        }
        combat::finish_tick(s, cs);
    }
    sanitize_run_states(s, combat_state.as_deref(), tune);
    sanitize_press_states(s, combat_state.as_deref());
    // Authoritative possession for this tick is settled. A loose ball
    // keeps the prior team, so one flip opens exactly one transition and a
    // spill inside it neither restarts nor cancels the window.
    {
        let owner_team = s
            .owner
            .map(|o| to_transition_team(s.players[(o - 1) as usize].team));
        s.transition.observe(owner_team, dt);
    }

    // A gained ball resolves any in-flight pass: nobody is "running onto"
    // it any more. In particular an INTERCEPTED back-pass ends the
    // keeper's receive window, so its save reflexes come straight back
    // online.
    if s.owner.is_some() && s.owner != prev_owner {
        for p in &mut s.players {
            p.receive_timer = 0.0;
        }
    }

    // Auto-switch on turnover: the moment the opponent wins the ball, hand
    // control to the home outfielder best placed to defend (nearest the
    // ball) — mirroring the existing auto-switch when a home player wins
    // it.
    let owner_team = s.owner.map(|o| s.players[(o - 1) as usize].team);
    if !s.slot_mode
        && s.human_controlled
        && owner_team == Some(Team::Away)
        && prev_owner_team != Some(Team::Away)
    {
        set_controlled_player(s, best_defender(s));
    }

    // Cross aid: when a lofted ball flies into the human's attacking third
    // and the human isn't already on it, hand control to the attacker
    // best placed to meet it — so a single strike (with the aerial magnet)
    // finishes the cross.
    if !s.slot_mode
        && s.human_controlled
        && owner_team != Some(Team::Home)
        && s.ball_z > CROSS_AID_Z
        && s.ball.x > s.field.w * CROSS_AID_THIRD
    {
        let mut best: Option<i64> = None;
        let mut best_d: Option<f64> = None;
        for (i, p) in s.players.iter().enumerate() {
            if p.team == Team::Home && !p.is_keeper {
                let d = p.pos.dist(s.ball);
                if best_d.is_none_or(|bd| d < bd) {
                    best_d = Some(d);
                    best = Some((i + 1) as i64);
                }
            }
        }
        if let Some(best) = best
            && best_d.expect("best implies best_d") <= CROSS_AID_RANGE
        {
            set_controlled_player(s, best);
        }
    }

    // Keeper control: the human takes over the HOME keeper while it holds
    // the ball (to pick the distribution), and control returns to an
    // outfielder the moment the keeper no longer has it.
    if !s.slot_mode && s.human_controlled {
        if let Some(oi) = s.owner
            && s.players[(oi - 1) as usize].team == Team::Home
            && s.players[(oi - 1) as usize].is_keeper
        {
            if s.owner != prev_owner {
                set_controlled_player(s, oi);
                // The six-second clock only runs on a ball held in the
                // HANDS; a back-pass trapped at the feet plays on at your
                // own pace.
                if !s.players[(oi - 1) as usize].feet_ball {
                    s.players[(oi - 1) as usize].hold_timer = tune.value("KEEPER_HOLD_HUMAN");
                }
            }
        } else if s.players[(s.controlled - 1) as usize].is_keeper {
            set_controlled_player(s, best_defender(s));
        }
    }

    let scorer = check_goal(s, prev_ball_x);
    if let Some(scorer) = scorer {
        // Last use of `combat_state` this tick: move it out rather than
        // reborrowing via `as_deref_mut()`.
        if let Some(cs) = combat_state {
            combat::reset_for_kickoff(s, cs);
        }
        for player in &mut s.players {
            player.keeper_set = 0.0;
            player.save_style = None;
            player.save_tip_emitted = false;
        }
        if s.score.home >= s.max_goals || s.score.away >= s.max_goals {
            s.finished = true;
            reset_press_states(s);
            reset_run_states(s);
            reset_transition_state(s);
        } else {
            // The team that conceded restarts play.
            place_kickoff(s, opposite(scorer));
        }
    }
}
