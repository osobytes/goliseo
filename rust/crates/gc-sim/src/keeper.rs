//! Pure keeper positioning, shot-stopping, and chip geometry. Callers own the
//! world (ball, players, timers) and drive these functions once per tick.
//!
//! `keeper::travel_time` routes its transcendental step through
//! [`gc_core::deterministic_math::negative_log_one_minus`], never
//! `f64::ln`, so every wasm runtime computes identical bits — this module
//! sits on the determinism path (see the crate root docs and
//! `tools/lua_reference/README.md`).

use crate::tunable_registry;
use gc_core::deterministic_math;
use gc_core::vec2::Vec2;

/// Which goal a keeper defends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    /// Defends the left-hand goal and attacks right.
    Home,
    /// Defends the right-hand goal and attacks left.
    Away,
}

/// An axis-aligned rectangle, used here for a goal mouth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// How a keeper commits to a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStyle {
    /// A save made without a full dive.
    Spread,
    /// A dive that stays within the central reach fraction.
    Central,
    /// A full-stretch dive.
    Stretch,
}

/// The keeper's current positioning state machine state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeeperBehaviorState {
    /// Ordinary shallow neutral arc positioning.
    Base,
    /// Advancing to close down a loose ball or unsettled attacker.
    Advance,
    /// Holding a contain position rather than fully committing.
    Contain,
    /// Set for an anticipated shot.
    Set,
    /// Retreating to a deep or base target after a cue.
    Retreat,
    /// A brief settle after leaving advance/contain/set, before retreating.
    Recover,
}

/// How a shot travels: rolled along the ground or lofted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeeperShotType {
    /// A ground-level shot.
    Ground,
    /// A lofted, chipped shot.
    Chip,
}

/// Inputs to [`depth_target`], [`base_target`], and [`arc_target`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperPositionContext {
    /// The keeper's current position.
    pub keeper_pos: Vec2,
    /// The ball's current position.
    pub ball_pos: Vec2,
    /// The goal this keeper defends.
    pub goal: Rect,
    /// Which goal this keeper defends.
    pub team: Team,
    /// How far off the line the keeper is willing to move.
    pub aggression: f64,
    /// Whether this is a one-on-one situation.
    pub in_1v1: bool,
}

/// Inputs to [`shot_targets_goal`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperShotContext {
    /// The team defending this goal.
    pub defending_team: Team,
    /// The team taking the shot.
    pub shooter_team: Team,
    /// Where the shot was struck from.
    pub origin: Vec2,
    /// The shot's flight direction (not necessarily normalized).
    pub direction: Vec2,
    /// The goal being shot at.
    pub goal: Rect,
}

/// Inputs to [`should_set`]. Extends [`KeeperShotContext`] with wind-up
/// timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperSetContext {
    /// The team defending this goal.
    pub defending_team: Team,
    /// The team taking the shot.
    pub shooter_team: Team,
    /// Where the shot was struck from.
    pub origin: Vec2,
    /// The shot's flight direction (not necessarily normalized).
    pub direction: Vec2,
    /// The goal being shot at.
    pub goal: Rect,
    /// How early the keeper reads the shot, in `[0, 1]`.
    pub anticipation: f64,
    /// The shot's total wind-up duration.
    pub windup_duration: f64,
    /// Seconds remaining in the shot's wind-up.
    pub windup_remaining: f64,
}

impl KeeperSetContext {
    fn as_shot_context(&self) -> KeeperShotContext {
        KeeperShotContext {
            defending_team: self.defending_team,
            shooter_team: self.shooter_team,
            origin: self.origin,
            direction: self.direction,
            goal: self.goal,
        }
    }
}

/// Inputs to [`should_advance`] and [`should_contain`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperAdvanceContext {
    /// The keeper is within its claim zone.
    pub in_claim_zone: bool,
    /// The attacker has the ball under control.
    pub attacker_controlled: bool,
    /// The ball is loose near the attacker.
    pub loose_touch: bool,
    /// A teammate is already close enough to support.
    pub support_near: bool,
    /// A defender is already engaging the attacker.
    pub defender_engaged: bool,
    /// Distance from the keeper to the threat.
    pub threat_distance: f64,
}

/// Inputs to [`behavior`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperBehaviorContext {
    /// The keeper's current state.
    pub current_state: KeeperBehaviorState,
    /// Seconds remaining in the current state's timer.
    pub state_timer: f64,
    /// The keeper's current position.
    pub keeper_pos: Vec2,
    /// The ball's current position.
    pub ball_pos: Vec2,
    /// The goal this keeper defends.
    pub goal: Rect,
    /// Which goal this keeper defends.
    pub team: Team,
    /// How far off the line the keeper is willing to move.
    pub aggression: f64,
    /// Whether [`should_advance`] is currently true.
    pub advance_eligible: bool,
    /// Whether [`should_contain`] is currently true.
    pub contain_eligible: bool,
    /// A ground shot has been read.
    pub ground_cue: bool,
    /// A lob has been read.
    pub lob_cue: bool,
    /// A through ball has been read.
    pub through_ball_cue: bool,
    /// The tick's delta time.
    pub dt: f64,
}

/// The result of one [`behavior`] tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperBehaviorDecision {
    /// The keeper's next state.
    pub state: KeeperBehaviorState,
    /// Seconds remaining in the next state's timer.
    pub state_timer: f64,
    /// Where the keeper should move toward.
    pub target: Vec2,
    /// A movement speed multiplier for this tick, in `[0, 1]`.
    pub movement_scale: f64,
}

/// Inputs to [`chip_launch`] and [`committed_chip_launch`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperChipContext {
    /// Where the chip is struck from.
    pub origin: Vec2,
    /// Where the chip is aimed.
    pub target: Vec2,
    /// The keeper's current position.
    pub keeper_pos: Vec2,
    /// The team defending this goal.
    pub defending_team: Team,
    /// The goal being chipped at.
    pub goal: Rect,
    /// The ball's horizontal speed, px/s.
    pub horizontal_speed: f64,
    /// Fraction of horizontal speed shed per second.
    pub friction: f64,
    /// Downward acceleration, px/s^2.
    pub gravity: f64,
    /// How high above the ground the keeper can reach.
    pub keeper_clearance: f64,
    /// The crossbar height.
    pub crossbar: f64,
    /// The desired ball height as it crosses the goal line.
    pub desired_goal_height: f64,
}

/// Inputs to [`goal_line_height`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeeperTrajectoryContext {
    /// Where the shot was struck from.
    pub origin: Vec2,
    /// The shot's flight direction (not necessarily normalized).
    pub direction: Vec2,
    /// The ball's horizontal speed, px/s.
    pub horizontal_speed: f64,
    /// The ball's initial vertical speed, px/s.
    pub vertical_speed: f64,
    /// The team defending this goal.
    pub defending_team: Team,
    /// The goal being shot at.
    pub goal: Rect,
    /// Fraction of horizontal speed shed per second.
    pub friction: f64,
    /// Downward acceleration, px/s^2.
    pub gravity: f64,
}

const CLAIM_DEPTH: f64 = 275.0;
// The issue's fixed context deliberately omits field dimensions. GOLISEO's
// canonical 1648px pitch therefore supplies the 824px goal-line-to-midfield
// half-depth.
const MIDFIELD_DEPTH: f64 = 824.0;
const CONTEXT_LATERAL_GUARD: f64 = 28.0;
const BASE_LATERAL_GUARD: f64 = 40.0;
const BASE_MIN_DEPTH: f64 = 12.0;
const BASE_ARC_EXTRA_FRACTION: f64 = 0.15;
const BASE_ARC_MAX_EXTRA: f64 = 6.0;
const CONTAIN_DISTANCE: f64 = 4.0;
const RECOVER_DURATION: f64 = 0.18;
const CONTAIN_DEPTH_FRACTION: f64 = 0.8;
const CHIP_VISIBLE_MIN_DEPTH: f64 = 20.0;
const CHIP_CLEARANCE_PAD: f64 = 2.0;
const CROSSBAR_PAD: f64 = 2.0;
const CHIP_FALLBACK_HEIGHT_FRACTION: f64 = 0.5;
const MIN_CHIP_LAUNCH_SPEED: f64 = 1.0;
const MOVEMENT_SETTLE_TIME: f64 = 0.12;

// Tier-3 AI membership bands (#487). These five numbers used to be raw
// `const`s here. They are not five independent knobs: `smother_distance`,
// `spread_distance` and `central_reach_fraction` between them decide which of
// four save styles a shot produces, and `defender_handoff_distance` only means
// anything relative to `advance_threat_distance`. Editing one in isolation
// produces a classification nobody designed, which is exactly why they are
// authored as versioned band SETS in `gc_data::tunables::BAND_SETS` and
// substituted whole (`tunable_registry::Registry::substitute_band_set`);
// there is no single-edge write path.
//
// Read from the shipped registry rather than threaded through every
// signature: these are pure classifier helpers with no `Tuning` in scope, and
// `tunable_registry::shipped()` is read-only `static` content (see that
// module's doc on globals), so the values are identical on every peer and on
// every resimulation.
fn save_style_edge(edge: &str) -> f64 {
    tunable_registry::shipped().band_edge("keeper_save_style", edge)
}

fn engagement_edge(edge: &str) -> f64 {
    tunable_registry::shipped().band_edge("keeper_engagement", edge)
}

/// Whether `distance` is close enough for the keeper to smother the ball
/// directly instead of diving.
#[must_use]
pub fn in_smother_range(distance: f64) -> bool {
    distance <= save_style_edge("smother_distance")
}

fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.min(maximum).max(minimum)
}

fn goal_axis(team: Team, goal: Rect) -> (f64, f64) {
    match team {
        Team::Home => (goal.x + goal.w, 1.0),
        Team::Away => (goal.x, -1.0),
    }
}

/// A point `depth` pixels out from the goal centre, along the ray from the
/// goal centre toward the ball (falling back to the keeper's own position,
/// then to the straight infield direction, for a degenerate ray). Clamped
/// laterally to the context guard.
#[must_use]
pub fn depth_target(context: &KeeperPositionContext, depth: f64) -> Vec2 {
    let (goal_line_x, infield_direction) = goal_axis(context.team, context.goal);
    let goal_center = Vec2::new(goal_line_x, context.goal.y + context.goal.h / 2.0);
    if depth <= 0.0 {
        return goal_center;
    }

    let mut ray = context.ball_pos.sub(goal_center);
    if ray.length() == 0.0 {
        ray = context.keeper_pos.sub(goal_center);
    }
    if ray.length() == 0.0 || ray.x * infield_direction <= 0.0 {
        ray = Vec2::new(infield_direction, 0.0);
    }

    let target = goal_center.add(ray.normalized().scale(depth));
    Vec2::new(
        target.x,
        clamp(
            target.y,
            goal_center.y - CONTEXT_LATERAL_GUARD,
            goal_center.y + CONTEXT_LATERAL_GUARD,
        ),
    )
}

/// Choose a shallow neutral arc from the physical one-radius goal-line
/// inset. Depth and the deliberately exaggerated lateral corner concession
/// grow as an attack approaches, without becoming a continuously optimized
/// save surface.
#[must_use]
pub fn base_target(context: &KeeperPositionContext) -> Vec2 {
    let (goal_line_x, infield_direction) = goal_axis(context.team, context.goal);
    let ball_depth = (context.ball_pos.x - goal_line_x) * infield_direction;
    let goal_center_y = context.goal.y + context.goal.h / 2.0;
    if ball_depth >= MIDFIELD_DEPTH {
        return Vec2::new(
            goal_line_x + infield_direction * BASE_MIN_DEPTH,
            goal_center_y,
        );
    }

    let approach = clamp(
        (MIDFIELD_DEPTH - ball_depth) / (MIDFIELD_DEPTH - CLAIM_DEPTH),
        0.0,
        1.0,
    );
    let extra_cap = (context.aggression.max(0.0) * BASE_ARC_EXTRA_FRACTION).min(BASE_ARC_MAX_EXTRA);
    let target_depth = BASE_MIN_DEPTH + extra_cap * approach;
    let lateral = clamp(
        context.ball_pos.y - goal_center_y,
        -BASE_LATERAL_GUARD,
        BASE_LATERAL_GUARD,
    );
    Vec2::new(
        goal_line_x + infield_direction * target_depth,
        goal_center_y + lateral * approach,
    )
}

/// The keeper's aggression-scaled advance target: deep at goal centre beyond
/// midfield, extending out along the ball ray (up to full aggression in a
/// one-on-one) as the ball approaches.
#[must_use]
pub fn arc_target(context: &KeeperPositionContext) -> Vec2 {
    let (goal_line_x, infield_direction) = goal_axis(context.team, context.goal);
    let goal_center = Vec2::new(goal_line_x, context.goal.y + context.goal.h / 2.0);
    let ball_depth = (context.ball_pos.x - goal_line_x) * infield_direction;
    if ball_depth >= MIDFIELD_DEPTH {
        return goal_center;
    }

    let approach = clamp(
        (MIDFIELD_DEPTH - ball_depth) / (MIDFIELD_DEPTH - CLAIM_DEPTH),
        0.0,
        1.0,
    );
    let target_depth = context.aggression.max(0.0) * if context.in_1v1 { 1.0 } else { approach };
    if target_depth == 0.0 {
        return goal_center;
    }

    depth_target(context, target_depth)
}

/// Whether the keeper should advance off the line to close down the threat.
#[must_use]
pub fn should_advance(context: &KeeperAdvanceContext) -> bool {
    context.in_claim_zone
        && context.threat_distance <= engagement_edge("advance_threat_distance")
        && (context.attacker_controlled || context.loose_touch)
        && !context.support_near
        && (context.loose_touch
            || !context.defender_engaged
            || context.threat_distance <= engagement_edge("defender_handoff_distance"))
}

/// Whether the keeper should at least contain the threat (a superset of
/// [`should_advance`]'s eligibility).
#[must_use]
pub fn should_contain(context: &KeeperAdvanceContext) -> bool {
    context.in_claim_zone
        && context.threat_distance <= engagement_edge("advance_threat_distance")
        && (context.attacker_controlled || context.loose_touch)
}

/// Advance the keeper's positioning state machine by one tick.
#[must_use]
pub fn behavior(context: &KeeperBehaviorContext) -> KeeperBehaviorDecision {
    let position_context = KeeperPositionContext {
        keeper_pos: context.keeper_pos,
        ball_pos: context.ball_pos,
        goal: context.goal,
        team: context.team,
        aggression: context.aggression,
        in_1v1: true,
    };
    let base = base_target(&position_context);
    let retreat_target = base;
    let deep_target = depth_target(&position_context, 0.0);
    let advance_target = depth_target(&position_context, context.aggression.max(0.0));
    let contain_target = depth_target(
        &position_context,
        context.aggression.max(0.0) * CONTAIN_DEPTH_FRACTION,
    );
    let mut state = context.current_state;
    let mut state_timer = (context.state_timer - context.dt).max(0.0);
    let mut target = base;
    let mut movement_scale = 1.0;

    if context.lob_cue {
        state = KeeperBehaviorState::Retreat;
        state_timer = 0.0;
        target = deep_target;
        movement_scale = 0.85;
    } else if context.through_ball_cue && state != KeeperBehaviorState::Base {
        state = KeeperBehaviorState::Retreat;
        state_timer = 0.0;
        target = retreat_target;
        movement_scale = 0.85;
    } else if context.ground_cue {
        state = KeeperBehaviorState::Set;
        state_timer = 0.0;
        target = context.keeper_pos;
        movement_scale = 0.0;
    } else if context.advance_eligible {
        target = advance_target;
        if context.keeper_pos.dist(advance_target) <= CONTAIN_DISTANCE {
            state = KeeperBehaviorState::Contain;
            movement_scale = 0.45;
        } else {
            state = KeeperBehaviorState::Advance;
        }
        state_timer = 0.0;
    } else if context.contain_eligible {
        state = KeeperBehaviorState::Contain;
        state_timer = 0.0;
        target = contain_target;
        movement_scale = 0.6;
    } else if state == KeeperBehaviorState::Advance
        || state == KeeperBehaviorState::Contain
        || state == KeeperBehaviorState::Set
    {
        state = KeeperBehaviorState::Recover;
        state_timer = RECOVER_DURATION;
        target = context.keeper_pos;
        movement_scale = 0.0;
    } else if state == KeeperBehaviorState::Recover && state_timer > 0.0 {
        target = context.keeper_pos;
        movement_scale = 0.0;
    } else if state == KeeperBehaviorState::Retreat || state == KeeperBehaviorState::Recover {
        target = retreat_target;
        if context.keeper_pos.dist(retreat_target) <= CONTAIN_DISTANCE {
            state = KeeperBehaviorState::Base;
            movement_scale = 1.0;
        } else {
            state = KeeperBehaviorState::Retreat;
            movement_scale = 0.85;
        }
    } else {
        state = KeeperBehaviorState::Base;
    }

    KeeperBehaviorDecision {
        state,
        state_timer,
        target,
        movement_scale,
    }
}

/// Whether the keeper's own position leaves it visible enough to be chipped
/// (i.e. it has advanced past the minimum chip-visible depth).
#[must_use]
pub fn chip_is_visible(keeper_pos: Vec2, team: Team, goal: Rect) -> bool {
    let (goal_line_x, infield_direction) = goal_axis(team, goal);
    let depth = (keeper_pos.x - goal_line_x) * infield_direction;
    depth >= CHIP_VISIBLE_MIN_DEPTH
}

/// Time for a friction-decayed ground travel of `distance` at initial
/// `speed`. Routes the transcendental step through
/// [`deterministic_math::negative_log_one_minus`] rather than `f64::ln`, so
/// results are bit-identical across every wasm runtime. Returns `None` when
/// the distance is unreachable (`speed <= 0`) or the friction decay never
/// gets there (`ratio >= 0.95`).
#[must_use]
pub fn travel_time(distance: f64, speed: f64, friction: f64) -> Option<f64> {
    if distance <= 0.0 {
        return Some(0.0);
    }
    if speed <= 0.0 {
        return None;
    }
    if friction <= 0.0 {
        return Some(distance / speed);
    }
    let ratio = distance * friction / speed;
    if ratio >= 0.95 {
        return None;
    }
    Some(deterministic_math::negative_log_one_minus(ratio) / friction)
}

/// A moving keeper spends part of the fixed dive budget planting before
/// pushing off. This is release-time state debt, not a positional reach
/// bonus or penalty.
#[must_use]
pub fn reaction_reach(reach: f64, normalized_motion: f64, dive_duration: f64) -> f64 {
    if dive_duration <= 0.0 {
        return 0.0;
    }
    let settle_time = MOVEMENT_SETTLE_TIME * clamp(normalized_motion, 0.0, 1.0);
    let available_time = (dive_duration - settle_time).max(0.0);
    reach.max(0.0) * available_time / dive_duration
}

fn team_goal_line_x(team: Team, goal: Rect) -> f64 {
    if team == Team::Home {
        goal.x + goal.w
    } else {
        goal.x
    }
}

/// Solve for the vertical launch speed of a chip that clears the keeper
/// (plus a clearance pad) and crosses the goal line under the crossbar
/// (minus a pad), landing as close as possible to the desired goal height.
/// Returns `None` when no such solution exists (the keeper and goal-line
/// constraints are infeasible, or the path is degenerate).
#[must_use]
pub fn chip_launch(context: &KeeperChipContext) -> Option<f64> {
    let raw_direction = context.target.sub(context.origin);
    let distance = raw_direction.length();
    if distance <= 0.0 || context.horizontal_speed <= 0.0 {
        return None;
    }
    let direction = raw_direction.scale(1.0 / distance);

    let goal_line_x = team_goal_line_x(context.defending_team, context.goal);
    if direction.x == 0.0 {
        return None;
    }
    let goal_distance = (goal_line_x - context.origin.x) / direction.x;
    let keeper_distance = (context.keeper_pos.x - context.origin.x) / direction.x;
    if goal_distance <= 0.0 || keeper_distance <= 0.0 || keeper_distance >= goal_distance {
        return None;
    }

    let keeper_time = travel_time(keeper_distance, context.horizontal_speed, context.friction)?;
    let goal_time = travel_time(goal_distance, context.horizontal_speed, context.friction)?;

    let lower_keeper = (context.keeper_clearance + CHIP_CLEARANCE_PAD) / keeper_time
        + 0.5 * context.gravity * keeper_time;
    let lower_goal = 0.5 * context.gravity * goal_time;
    let desired_goal = context.desired_goal_height / goal_time + 0.5 * context.gravity * goal_time;
    let upper_goal =
        (context.crossbar - CROSSBAR_PAD) / goal_time + 0.5 * context.gravity * goal_time;
    let vertical_speed = lower_keeper.max(lower_goal).max(desired_goal);
    if vertical_speed >= upper_goal {
        return None;
    }
    Some(vertical_speed)
}

/// Lock a human-selected chip verb at commit time. Prefer the fully
/// feasible keeper-clearing solution ([`chip_launch`]); when that interval
/// is empty, keep the chip intent with an under-bar goal-height arc. If
/// friction prevents the ball reaching the goal at all, use a deterministic
/// low lob that will land short.
#[must_use]
pub fn committed_chip_launch(context: &KeeperChipContext) -> f64 {
    if let Some(feasible) = chip_launch(context) {
        return feasible;
    }

    let raw_direction = context.target.sub(context.origin);
    if raw_direction.length() > 0.0 && raw_direction.x != 0.0 {
        let direction = raw_direction.normalized();
        let goal_line_x = team_goal_line_x(context.defending_team, context.goal);
        let goal_distance = (goal_line_x - context.origin.x) / direction.x;
        if let Some(goal_time) =
            travel_time(goal_distance, context.horizontal_speed, context.friction)
        {
            let desired_height = clamp(
                context.desired_goal_height,
                0.0,
                (context.crossbar - CROSSBAR_PAD).max(0.0),
            );
            return MIN_CHIP_LAUNCH_SPEED
                .max(desired_height / goal_time + 0.5 * context.gravity * goal_time);
        }
    }

    let low_height = CHIP_CLEARANCE_PAD.max(
        context
            .desired_goal_height
            .min(context.keeper_clearance)
            .min((context.crossbar - CROSSBAR_PAD).max(0.0))
            * CHIP_FALLBACK_HEIGHT_FRACTION,
    );
    MIN_CHIP_LAUNCH_SPEED.max((2.0 * context.gravity * low_height).max(0.0).sqrt())
}

/// The ball's height as it crosses the defended goal line, given a launched
/// trajectory. Returns `None` when the flight never reaches the goal line.
#[must_use]
pub fn goal_line_height(context: &KeeperTrajectoryContext) -> Option<f64> {
    if context.direction.x == 0.0 {
        return None;
    }
    let goal_line_x = team_goal_line_x(context.defending_team, context.goal);
    let distance = (goal_line_x - context.origin.x) / context.direction.x;
    if distance < 0.0 {
        return None;
    }
    let eta = travel_time(distance, context.horizontal_speed, context.friction)?;
    Some(context.vertical_speed * eta - 0.5 * context.gravity * eta * eta)
}

/// Classify a save beyond the smother distance as spread, central, or a
/// full stretch.
///
/// # Panics
///
/// Panics if `dist_to_keeper` is within [`in_smother_range`] — smothers are
/// a distinct, earlier branch and are never classified here.
#[must_use]
pub fn save_style(dist_to_keeper: f64, dive_dist: f64, reach: f64) -> SaveStyle {
    assert!(
        !in_smother_range(dist_to_keeper),
        "save_style only classifies saves beyond the smother distance"
    );
    if dist_to_keeper <= save_style_edge("spread_distance") {
        return SaveStyle::Spread;
    }
    if dive_dist <= reach * save_style_edge("central_reach_fraction") {
        return SaveStyle::Central;
    }
    SaveStyle::Stretch
}

/// Seconds before the shot lands at which the keeper should commit to
/// setting for it.
#[must_use]
pub fn commit_lead(anticipation: f64, windup_duration: f64) -> f64 {
    clamp(anticipation, 0.0, 1.0) * windup_duration.max(0.0)
}

/// Whether a captured shot direction is projected to cross the defended
/// goal mouth.
#[must_use]
pub fn shot_targets_goal(context: &KeeperShotContext) -> bool {
    if context.shooter_team == context.defending_team || context.direction.x == 0.0 {
        return false;
    }

    let goal_line_x = team_goal_line_x(context.defending_team, context.goal);
    let flight = (goal_line_x - context.origin.x) / context.direction.x;
    if flight < 0.0 {
        return false;
    }

    let goal_y = context.origin.y + context.direction.y * flight;
    goal_y >= context.goal.y && goal_y <= context.goal.y + context.goal.h
}

/// Whether the keeper should transition into the `set` state right now, given
/// the captured shot's wind-up timing.
#[must_use]
pub fn should_set(context: &KeeperSetContext) -> bool {
    let lead = commit_lead(context.anticipation, context.windup_duration);
    lead > 0.0
        && context.windup_remaining > 0.0
        && context.windup_remaining <= lead
        && shot_targets_goal(&context.as_shot_context())
}
