//! Port of `sim/combat_feasibility.lua`.
//!
//! `intervention_candidate/v2` and `family_commit_feasibility/v1`.
//!
//! Both are PURE temporal predicates over `combat_sim_observation/v1`. They read
//! no confirmed future input, no rollback-only or hidden state, no unresolved
//! outcome, no presentation, and no eventual contact. Everything they project is
//! a counterfactual, not a claim about what will happen.
//!
//! Public motion projection (the frozen rule these predicates share with the
//! observation schema's `COLLISION_CATALOG_VERSION`): a body advances linearly
//! at the fixed tick from its observed position and velocity and is clamped to
//! the pitch by its radius. A committed or forced public phase scales that
//! velocity by the catalogued family movement multiplier (zero while forced),
//! which is the "catalogued public pose path anchored at the observed state"
//! the contract asks for. Nothing here searches a non-source trajectory.
//!
//! The five purpose ids are exactly section 4.6's: carrier contest, carrier
//! protection, loose-ball contest, passing-lane/shot denial, and recovery
//! punish. `formation_risk_tradeoff` is a cost flag returned alongside them and
//! is never a sixth purpose.
//!
//! ## Local observation types, not `combat_observation`
//!
//! The Lua original does not `require("sim.combat_observation")` — it only
//! duck-types over the `combat_sim_observation/v1` shape in LuaCATS comments.
//! `sim/combat_observation.lua`, `sim/combat.lua`, `sim/combat_policy.lua`, and
//! `sim/match.lua` are owned by other agents and are not ported yet, so this
//! module cannot import a `combat_observation` crate module either. Instead it
//! defines its own local, narrowed view of the schema: only the fields this
//! file actually reads, named to match the Lua field names field-for-field
//! (`observation.self` → [`CombatObservation::own`], `observation.match` →
//! [`CombatObservation::match_view`], both renamed only because `self` and
//! `match` are Rust keywords). When `combat_observation` is ported, the
//! canonical schema type should be able to populate these fields directly.

use crate::fixed_clock;
use gc_data::action_families::{self, ActionFamilyData, ActionFamilyId};

/// Frame/schema version this module implements.
pub const VERSION: i64 = 1;
/// `intervention_candidate` payload version.
pub const ENVELOPE_VERSION: i64 = 2;
/// Default number of commit ticks the envelope search tries.
pub const SEARCH_TICKS: i64 = 30;

// Section 4.6 constants. These are contract numbers, not tuning knobs.
/// Max distance, in px, between the ball owner and a defender for that
/// defender to count as protecting the carrier.
pub const CARRIER_PROTECTION_PX: f64 = 48.0;
/// Max distance, in px, from a loose ball for a player to be in contest range.
pub const LOOSE_BALL_PX: f64 = 96.0;
/// Minimum projected drift, in px, from a source's anchor for committing to
/// count as a formation risk.
pub const FORMATION_RISK_GAIN_PX: f64 = 36.0;
/// Distance, in px, from a source's anchor beyond which it is already a
/// formation risk regardless of projected drift.
pub const FORMATION_RISK_ANCHOR_PX: f64 = 120.0;

// Lane geometry. A blocker "controls" a segment when its projected body is
// within its own radius plus this corridor of the segment.
const LANE_CORRIDOR_PX: f64 = 18.0;
const SHOT_RANGE_PX: f64 = 260.0;
const EPSILON: f64 = 1e-9;

/// One of the five section-4.6 combat purposes a (source, target) pair can
/// hold. Declared in ascending alphabetical order of the Lua string ids
/// (`"carrier_contest"` < `"carrier_protection"` < `"loose_ball_contest"` <
/// `"passing_lane_or_shot_denial"` < `"recovery_punish"`) so the derived
/// [`Ord`] reproduces the Lua `<` string comparison
/// [`intervention_candidates`] sorts by. This is deliberately **not** dominance
/// order — see [`PURPOSE_PRIORITY`] for that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CombatPurposeId {
    /// The target is the opposing ball carrier.
    CarrierContest,
    /// The target is the nearest opponent to our own carrier.
    CarrierProtection,
    /// The target and the source both sit within reach of a loose ball.
    LooseBallContest,
    /// The target is the sole blocker of one of our passing lanes, or the
    /// likely shooter / sole controller of an open shot lane at our goal.
    PassingLaneOrShotDenial,
    /// The target is mid-recovery on top of another true purpose.
    RecoveryPunish,
}

/// Every valid purpose id, in no particular order. Mirrors the Lua module's
/// `PURPOSES` membership set.
pub const PURPOSES: [CombatPurposeId; 5] = [
    CombatPurposeId::CarrierContest,
    CombatPurposeId::CarrierProtection,
    CombatPurposeId::LooseBallContest,
    CombatPurposeId::PassingLaneOrShotDenial,
    CombatPurposeId::RecoveryPunish,
];

/// Purpose priority when several predicates hold for the same pair. #148
/// keeps the whole bitset and reports `multi_context`; #112 must emit exactly
/// one stable reason, so the most specific true purpose wins and the order
/// never depends on scores. This is dominance order, distinct from
/// [`CombatPurposeId`]'s derived alphabetical `Ord`.
pub const PURPOSE_PRIORITY: [CombatPurposeId; 5] = [
    CombatPurposeId::RecoveryPunish,
    CombatPurposeId::CarrierContest,
    CombatPurposeId::LooseBallContest,
    CombatPurposeId::PassingLaneOrShotDenial,
    CombatPurposeId::CarrierProtection,
];

/// A fixture side. Local to this module for the same reason the observation
/// types are local: the Lua original never `require("sim.input_frame")`
/// either, it only names `InputTeam` in a type comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    /// The home side.
    Home,
    /// The away side.
    Away,
}

/// A row's current combat action phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatActionPhase {
    /// No action in progress.
    Ready,
    /// Committed but not yet active.
    Windup,
    /// Contact-capable.
    Active,
    /// Charging a held-release action (ranged aim).
    Aim,
    /// Actively guarding.
    Guard,
    /// Post-action recovery.
    Recovery,
}

fn length(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

fn unit_or(x: f64, y: f64, fallback_x: f64, fallback_y: f64) -> (f64, f64) {
    let len = length(x, y);
    if len > EPSILON {
        (x / len, y / len)
    } else {
        (fallback_x, fallback_y)
    }
}

/// A body `project` can advance: `self`'s own row or a peer's row. Mirrors
/// the Lua signature's duck-typed `row: CombatObservationPeer|CombatObservationSelf`.
pub trait ProjectableRow {
    /// Ticks remaining in an active forced state (stagger/knockback); a
    /// positive value zeroes the body's projected velocity.
    fn forced_ticks(&self) -> i64;
    /// The row's current combat phase.
    fn phase(&self) -> CombatActionPhase;
    /// The row's currently equipped family, or `None` for the Lua `"none"`
    /// sentinel (no loadout).
    fn family_id(&self) -> Option<ActionFamilyId>;
    /// World-space x velocity, px/s.
    fn vx(&self) -> f64;
    /// World-space y velocity, px/s.
    fn vy(&self) -> f64;
    /// World-space x position, px.
    fn x(&self) -> f64;
    /// World-space y position, px.
    fn y(&self) -> f64;
    /// Collision radius, px.
    fn radius(&self) -> f64;
}

/// Own materialized combat state and physical pose: the schema's `self` row,
/// renamed [`CombatObservation::own`] because `self` is a Rust keyword.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatObservationSelf {
    /// Canonical player index.
    pub player_index: i64,
    /// Fixture side.
    pub team: Team,
    /// World-space x position, px.
    pub x: f64,
    /// World-space y position, px.
    pub y: f64,
    /// World-space x velocity, px/s.
    pub vx: f64,
    /// World-space y velocity, px/s.
    pub vy: f64,
    /// Unit facing x component.
    pub facing_x: f64,
    /// Unit facing y component.
    pub facing_y: f64,
    /// Collision radius, px.
    pub radius: f64,
    /// Own base locomotion speed, world px/s.
    pub move_speed: f64,
    /// Current combat action phase.
    pub phase: CombatActionPhase,
    /// Currently equipped family, or `None` without a loadout.
    pub family_id: Option<ActionFamilyId>,
    /// Ticks remaining in an active forced state.
    pub forced_ticks: i64,
}

impl ProjectableRow for CombatObservationSelf {
    fn forced_ticks(&self) -> i64 {
        self.forced_ticks
    }
    fn phase(&self) -> CombatActionPhase {
        self.phase
    }
    fn family_id(&self) -> Option<ActionFamilyId> {
        self.family_id
    }
    fn vx(&self) -> f64 {
        self.vx
    }
    fn vy(&self) -> f64 {
        self.vy
    }
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn radius(&self) -> f64 {
        self.radius
    }
}

/// A teammate or opponent row: the schema's `CombatObservationPeer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatObservationPeer {
    /// Canonical player index.
    pub player_index: i64,
    /// Fixture side.
    pub team: Team,
    /// Whether this row is a keeper.
    pub is_keeper: bool,
    /// World-space x position, px.
    pub x: f64,
    /// World-space y position, px.
    pub y: f64,
    /// World-space x velocity, px/s.
    pub vx: f64,
    /// World-space y velocity, px/s.
    pub vy: f64,
    /// Unit facing x component.
    pub facing_x: f64,
    /// Unit facing y component.
    pub facing_y: f64,
    /// Collision radius, px.
    pub radius: f64,
    /// Current combat action phase.
    pub phase: CombatActionPhase,
    /// Ticks spent in the current phase.
    pub phase_ticks: i64,
    /// Currently equipped family, or `None` without a loadout.
    pub family_id: Option<ActionFamilyId>,
    /// Ticks remaining in an active forced state.
    pub forced_ticks: i64,
    /// Whether a ranged release has publicly latched.
    pub release_latched: bool,
    /// Absolute tick a latched ranged projectile is projected to spawn on;
    /// `0` unless ranged and latched.
    pub projected_spawn_tick: i64,
    /// Catalogued melee reach, px; `0` off the melee families.
    pub projected_reach_px: f64,
}

impl ProjectableRow for CombatObservationPeer {
    fn forced_ticks(&self) -> i64 {
        self.forced_ticks
    }
    fn phase(&self) -> CombatActionPhase {
        self.phase
    }
    fn family_id(&self) -> Option<ActionFamilyId> {
        self.family_id
    }
    fn vx(&self) -> f64 {
        self.vx
    }
    fn vy(&self) -> f64 {
        self.vy
    }
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn radius(&self) -> f64 {
        self.radius
    }
}

/// An in-flight projectile row: the schema's `CombatObservationProjectile`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatObservationProjectile {
    /// Current x position, px.
    pub x: f64,
    /// Current y position, px.
    pub y: f64,
    /// Unit travel direction x component.
    pub dir_x: f64,
    /// Unit travel direction y component.
    pub dir_y: f64,
    /// The firing player's fixture side.
    pub source_team: Team,
    /// The firing player's canonical index.
    pub source_player_index: i64,
    /// Ticks remaining before the projectile expires.
    pub horizon_ticks: i64,
}

/// Public ball state: the schema's `CombatObservationBall`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatObservationBall {
    /// Current x position, px.
    pub x: f64,
    /// Current y position, px.
    pub y: f64,
    /// Canonical index of the current owner, or `0` when loose.
    pub owner_player_index: i64,
    /// Fixture side of the current owner, or `None` for the Lua `"none"`
    /// sentinel (loose ball).
    pub owner_team: Option<Team>,
}

/// Static pitch and goal geometry: the schema's `CombatObservationMatch`,
/// renamed [`CombatObservation::match_view`] because `match` is a Rust
/// keyword.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatObservationMatchView {
    /// Playable pitch width, px.
    pub field_w: f64,
    /// Playable pitch height, px.
    pub field_h: f64,
    /// Home goal mouth rect: left edge x.
    pub goal_home_x: f64,
    /// Home goal mouth rect: top edge y.
    pub goal_home_y: f64,
    /// Home goal mouth rect: height.
    pub goal_home_h: f64,
    /// Away goal mouth rect: left edge x.
    pub goal_away_x: f64,
    /// Away goal mouth rect: top edge y.
    pub goal_away_y: f64,
    /// Away goal mouth rect: width.
    pub goal_away_w: f64,
    /// Away goal mouth rect: height.
    pub goal_away_h: f64,
}

/// One player's authored formation anchor: the schema's
/// `CombatObservationAnchor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatObservationAnchor {
    /// Canonical player index.
    pub player_index: i64,
    /// Authored anchor x position, px.
    pub anchor_x: f64,
    /// Authored anchor y position, px.
    pub anchor_y: f64,
}

/// A narrowed local view of `combat_sim_observation/v1`: only the fields this
/// module reads. See the module doc comment for why this is not the
/// `combat_observation` crate's own type.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatObservation {
    /// This player's own row. Named `own` because `self` is a Rust keyword;
    /// the schema calls this field `self`.
    pub own: CombatObservationSelf,
    /// Own-side rows, excluding `own`.
    pub teammates: Vec<CombatObservationPeer>,
    /// Opposing-side rows.
    pub opponents: Vec<CombatObservationPeer>,
    /// In-flight projectile rows.
    pub projectiles: Vec<CombatObservationProjectile>,
    /// Public ball state.
    pub ball: CombatObservationBall,
    /// Static pitch and goal geometry. Named `match_view` because `match` is
    /// a Rust keyword; the schema calls this field `match`.
    pub match_view: CombatObservationMatchView,
    /// Authored formation anchors.
    pub anchors: Vec<CombatObservationAnchor>,
    /// The tick this observation was produced for.
    pub observed_tick: i64,
}

/// A body's position/velocity/radius after the frozen public no-response
/// projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatProjectedBody {
    /// Projected x position, px.
    pub x: f64,
    /// Projected y position, px.
    pub y: f64,
    /// Projected x velocity, px/s (after the family/forced scale).
    pub vx: f64,
    /// Projected y velocity, px/s (after the family/forced scale).
    pub vy: f64,
    /// Collision radius, px (unchanged by projection).
    pub radius: f64,
}

/// The frozen public no-response projection. A committed or forced public
/// phase scales the observed velocity by its catalogued multiplier; nothing
/// else moves the body but the pitch clamp.
#[must_use]
pub fn project<R: ProjectableRow>(
    row: &R,
    observation: &CombatObservation,
    ticks: i64,
) -> CombatProjectedBody {
    let mut scale = 1.0;
    if row.forced_ticks() > 0 {
        scale = 0.0;
    } else if row.phase() != CombatActionPhase::Ready
        && let Some(family_id) = row.family_id()
    {
        scale = action_families::get(family_id).movement_multiplier;
    }
    let vx = row.vx() * scale;
    let vy = row.vy() * scale;
    let seconds = ticks as f64 * fixed_clock::TICK_SECONDS;
    CombatProjectedBody {
        x: (row.x() + vx * seconds)
            .clamp(row.radius(), observation.match_view.field_w - row.radius()),
        y: (row.y() + vy * seconds)
            .clamp(row.radius(), observation.match_view.field_h - row.radius()),
        vx,
        vy,
        radius: row.radius(),
    }
}

/// Find a canonical player's row among opponents, then teammates.
#[must_use]
pub fn row_for(
    observation: &CombatObservation,
    player_index: i64,
) -> Option<&CombatObservationPeer> {
    observation
        .opponents
        .iter()
        .find(|row| row.player_index == player_index)
        .or_else(|| {
            observation
                .teammates
                .iter()
                .find(|row| row.player_index == player_index)
        })
}

// Melee contact geometry, mirroring `sim.combat`'s resolver exactly: inside
// the reach plus the target radius, in front of the facing, and inside the
// arc.
fn melee_contact(
    source_x: f64,
    source_y: f64,
    facing_x: f64,
    facing_y: f64,
    target: &CombatProjectedBody,
    family: &ActionFamilyData,
) -> bool {
    let dx = target.x - source_x;
    let dy = target.y - source_y;
    let distance = length(dx, dy);
    let reach = family
        .reach_px
        .expect("melee_contact is only called with a melee family");
    if distance > reach + target.radius {
        return false;
    }
    if distance <= EPSILON {
        return true;
    }
    if dx * facing_x + dy * facing_y < 0.0 {
        return false;
    }
    let half_arc = (family.front_arc_degrees / 2.0).to_radians();
    (dx / distance) * facing_x + (dy / distance) * facing_y + EPSILON >= half_arc.cos()
}

fn segment_hits_circle(
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    center_x: f64,
    center_y: f64,
    radius: f64,
) -> bool {
    let sx = end_x - start_x;
    let sy = end_y - start_y;
    let length_sq = sx * sx + sy * sy;
    if length_sq <= EPSILON {
        return length(center_x - start_x, center_y - start_y) <= radius;
    }
    let t = (((center_x - start_x) * sx + (center_y - start_y) * sy) / length_sq).clamp(0.0, 1.0);
    let px = start_x + sx * t;
    let py = start_y + sy * t;
    length(center_x - px, center_y - py) <= radius + EPSILON
}

fn all_peers(observation: &CombatObservation) -> Vec<&CombatObservationPeer> {
    let mut rows: Vec<&CombatObservationPeer> = observation
        .teammates
        .iter()
        .chain(observation.opponents.iter())
        .collect();
    rows.sort_by_key(|row| row.player_index);
    rows
}

/// The source pose the witness tape leaves behind. `family_commit_feasibility`
/// carries the tape's last legal movement and facing input after the commit;
/// an empty tape means neutral movement and the observed facing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatWitnessTape {
    /// Raw movement input x axis, direction only (magnitude is normalized).
    pub move_x: f64,
    /// Raw movement input y axis, direction only (magnitude is normalized).
    pub move_y: f64,
    /// Ticks this movement input was held for before the commit.
    pub ticks: i64,
}

fn source_pose_after_tape(
    observation: &CombatObservation,
    tape: Option<&CombatWitnessTape>,
) -> (f64, f64, f64, f64) {
    let own = &observation.own;
    let no_tape_motion = match tape {
        None => true,
        Some(tape) => tape.ticks <= 0 || (tape.move_x == 0.0 && tape.move_y == 0.0),
    };
    if no_tape_motion {
        let fallback_x = if own.team == Team::Home { 1.0 } else { -1.0 };
        let (fx, fy) = unit_or(own.facing_x, own.facing_y, fallback_x, 0.0);
        return (own.x, own.y, fx, fy);
    }
    let tape = tape.expect("no_tape_motion is false only when tape is Some");
    let (mx, my) = unit_or(tape.move_x, tape.move_y, 0.0, 0.0);
    let seconds = tape.ticks as f64 * fixed_clock::TICK_SECONDS;
    let x = (own.x + mx * own.move_speed * seconds)
        .clamp(own.radius, observation.match_view.field_w - own.radius);
    let y = (own.y + my * own.move_speed * seconds)
        .clamp(own.radius, observation.match_view.field_h - own.radius);
    (x, y, mx, my)
}

/// The source's committed pose at a relative tick after the commit: it
/// repeats the tape's last legal movement vector through the ordinary family
/// movement multiplier, the pitch, and the collision transition.
fn committed_source_pose(
    observation: &CombatObservation,
    tape: Option<&CombatWitnessTape>,
    family: &ActionFamilyData,
    ticks: i64,
) -> (f64, f64, f64, f64) {
    let (x, y, fx, fy) = source_pose_after_tape(observation, tape);
    let own = &observation.own;
    let (mx, my) = match tape {
        Some(tape) if tape.move_x != 0.0 || tape.move_y != 0.0 => {
            unit_or(tape.move_x, tape.move_y, 0.0, 0.0)
        }
        _ => (0.0, 0.0),
    };
    if mx == 0.0 && my == 0.0 {
        return (x, y, fx, fy);
    }
    let seconds = ticks as f64 * fixed_clock::TICK_SECONDS;
    let speed = own.move_speed * family.movement_multiplier;
    (
        (x + mx * speed * seconds).clamp(own.radius, observation.match_view.field_w - own.radius),
        (y + my * speed * seconds).clamp(own.radius, observation.match_view.field_h - own.radius),
        fx,
        fy,
    )
}

/// A `family_commit_feasibility/v1` outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatFeasibilityWitness {
    /// Whether a witnessing commit was found.
    pub feasible: bool,
    /// The family this witness proves.
    pub family_id: ActionFamilyId,
    /// Frozen target, or the hostile threat source for guard.
    pub target_player: i64,
    /// Relative tick the commit starts on.
    pub commit_tick: i64,
    /// Relative tick of the witnessed intersection; `0` when infeasible.
    pub contact_tick: i64,
    /// Last relative tick the proof searched.
    pub horizon_ticks: i64,
}

fn melee_feasibility(
    observation: &CombatObservation,
    target: &CombatObservationPeer,
    family: &ActionFamilyData,
    tape: Option<&CombatWitnessTape>,
) -> CombatFeasibilityWitness {
    let commit_tick = tape.map_or(0, |tape| tape.ticks);
    let active_ticks = family
        .active_ticks
        .expect("melee_feasibility is only called with a melee family");
    let horizon = commit_tick + family.windup_ticks + active_ticks;
    for offset in 1..=active_ticks {
        let relative = family.windup_ticks + offset;
        let (sx, sy, fx, fy) = committed_source_pose(observation, tape, family, relative);
        let projected = project(target, observation, commit_tick + relative);
        if melee_contact(sx, sy, fx, fy, &projected, family) {
            return CombatFeasibilityWitness {
                feasible: true,
                family_id: family.id,
                target_player: target.player_index,
                commit_tick,
                contact_tick: commit_tick + relative,
                horizon_ticks: horizon,
            };
        }
    }
    CombatFeasibilityWitness {
        feasible: false,
        family_id: family.id,
        target_player: target.player_index,
        commit_tick,
        contact_tick: 0,
        horizon_ticks: horizon,
    }
}

// The canonical ranged witness: commit with `pressed, held`, emit the early
// release edge on the next tick, and let it latch through the rest of the
// windup. At the spawn transition the aim must be legal and the line clear
// against projected public blockers; the direction then freezes and the
// projectile travels at the catalogued speed for at most its lifetime or
// until it leaves the pitch.
fn ranged_feasibility(
    observation: &CombatObservation,
    target: &CombatObservationPeer,
    tape: Option<&CombatWitnessTape>,
) -> CombatFeasibilityWitness {
    let family = action_families::get(ActionFamilyId::Ranged);
    let commit_tick = tape.map_or(0, |tape| tape.ticks);
    let lifetime = family
        .projectile_lifetime_ticks
        .expect("the ranged family defines a projectile lifetime");
    let spawn_relative = family.windup_ticks + 1;
    let horizon = commit_tick + spawn_relative + lifetime;
    let (sx, sy, fx, fy) = committed_source_pose(observation, tape, family, spawn_relative);
    let step = family
        .projectile_speed_px_per_second
        .expect("the ranged family defines a projectile speed")
        * fixed_clock::TICK_SECONDS;
    let blockers = all_peers(observation);
    let mut x = sx;
    let mut y = sy;
    for travel in 1..=lifetime {
        let next_x = x + fx * step;
        let next_y = y + fy * step;
        let absolute = commit_tick + spawn_relative + travel;
        let mut first: Option<&CombatObservationPeer> = None;
        let mut first_time = f64::INFINITY;
        for row in &blockers {
            if row.is_keeper || row.team == observation.own.team {
                continue;
            }
            let projected = project(*row, observation, absolute);
            if segment_hits_circle(
                x,
                y,
                next_x,
                next_y,
                projected.x,
                projected.y,
                projected.radius,
            ) {
                let time = length(projected.x - x, projected.y - y);
                if time < first_time {
                    first_time = time;
                    first = Some(row);
                }
            }
        }
        if let Some(first_row) = first {
            let feasible = first_row.player_index == target.player_index;
            return CombatFeasibilityWitness {
                feasible,
                family_id: ActionFamilyId::Ranged,
                target_player: target.player_index,
                commit_tick,
                contact_tick: if feasible { absolute } else { 0 },
                horizon_ticks: horizon,
            };
        }
        x = next_x;
        y = next_y;
        if x < 0.0
            || x > observation.match_view.field_w
            || y < 0.0
            || y > observation.match_view.field_h
        {
            break;
        }
    }
    CombatFeasibilityWitness {
        feasible: false,
        family_id: ActionFamilyId::Ranged,
        target_player: target.player_index,
        commit_tick,
        contact_tick: 0,
        horizon_ticks: horizon,
    }
}

/// One finite public hostile path a `guard` commit or a spacing/evade read
/// can react to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatHostilePath<'a> {
    /// The public row this path originates from, when it is not a bare
    /// projectile.
    pub source: Option<&'a CombatObservationPeer>,
    /// The public projectile row this path originates from, when it is not a
    /// row's own melee/ranged action.
    pub projectile: Option<&'a CombatObservationProjectile>,
    /// First relative tick of the path.
    pub start_tick: i64,
    /// Last relative tick of the path.
    pub last_tick: i64,
}

/// Guard's relevant threat set: only finite paths the public observation
/// already proves. A held or aimed ranged row without a public release latch
/// never participates.
#[must_use]
pub fn hostile_paths(
    observation: &CombatObservation,
    source_index: Option<i64>,
) -> Vec<CombatHostilePath<'_>> {
    let mut paths = Vec::new();
    let tick = observation.observed_tick;
    for row in &observation.opponents {
        if row.is_keeper || source_index.is_some_and(|index| index != row.player_index) {
            continue;
        }
        match row.family_id {
            Some(ActionFamilyId::Unarmed) | Some(ActionFamilyId::LightMelee) => {
                let family = action_families::get(row.family_id.expect("matched Some above"));
                match row.phase {
                    CombatActionPhase::Windup => paths.push(CombatHostilePath {
                        source: Some(row),
                        projectile: None,
                        start_tick: row.phase_ticks + 1,
                        last_tick: row.phase_ticks
                            + family
                                .active_ticks
                                .expect("unarmed/light_melee always define active_ticks"),
                    }),
                    CombatActionPhase::Active => paths.push(CombatHostilePath {
                        source: Some(row),
                        projectile: None,
                        start_tick: 1,
                        last_tick: row.phase_ticks,
                    }),
                    _ => {}
                }
            }
            Some(ActionFamilyId::Ranged) if row.release_latched => {
                let spawn = row.projected_spawn_tick - tick;
                if spawn >= 0 {
                    let lifetime = action_families::get(ActionFamilyId::Ranged)
                        .projectile_lifetime_ticks
                        .expect("the ranged family defines a projectile lifetime");
                    paths.push(CombatHostilePath {
                        source: Some(row),
                        projectile: None,
                        start_tick: spawn + 1,
                        last_tick: spawn + lifetime,
                    });
                }
            }
            _ => {}
        }
    }
    for row in &observation.projectiles {
        if row.source_team != observation.own.team
            && source_index.is_none_or(|index| index == row.source_player_index)
        {
            paths.push(CombatHostilePath {
                source: None,
                projectile: Some(row),
                start_tick: 1,
                last_tick: row.horizon_ticks,
            });
        }
    }
    paths
}

fn hostile_contact_point(
    observation: &CombatObservation,
    path: &CombatHostilePath<'_>,
    tick: i64,
) -> Option<(f64, f64)> {
    if let Some(projectile) = path.projectile {
        let step = action_families::get(ActionFamilyId::Ranged)
            .projectile_speed_px_per_second
            .expect("the ranged family defines a projectile speed")
            * fixed_clock::TICK_SECONDS;
        return Some((
            projectile.x + projectile.dir_x * step * tick as f64,
            projectile.y + projectile.dir_y * step * tick as f64,
        ));
    }
    let row = path
        .source
        .expect("a hostile path always carries a source or a projectile");
    if row.family_id == Some(ActionFamilyId::Ranged) {
        let spawn = row.projected_spawn_tick - observation.observed_tick;
        let travel = tick - spawn;
        if travel < 1 {
            return None;
        }
        let body = project(row, observation, spawn);
        let fallback_x = if row.team == Team::Home { 1.0 } else { -1.0 };
        let (fx, fy) = unit_or(row.facing_x, row.facing_y, fallback_x, 0.0);
        let step = action_families::get(ActionFamilyId::Ranged)
            .projectile_speed_px_per_second
            .expect("the ranged family defines a projectile speed")
            * fixed_clock::TICK_SECONDS;
        return Some((
            body.x + fx * step * travel as f64,
            body.y + fy * step * travel as f64,
        ));
    }
    let body = project(row, observation, tick);
    Some((body.x, body.y))
}

// Guard is self-only. It must intersect the frozen hostile source's melee or
// projectile contact path inside its catalogued arc on an active guard tick
// at or before the cap, then release on the next tick. No relevant public
// path makes guard infeasible; the release tick can never create an
// intersection.
fn guard_feasibility(
    observation: &CombatObservation,
    source_index: i64,
    tape: Option<&CombatWitnessTape>,
) -> CombatFeasibilityWitness {
    let family = action_families::get(ActionFamilyId::Guard);
    let commit_tick = tape.map_or(0, |tape| tape.ticks);
    let paths = hostile_paths(observation, Some(source_index));
    let mut cap = family.windup_ticks;
    for path in &paths {
        cap = cap.max(path.last_tick);
    }
    let mut witness = CombatFeasibilityWitness {
        feasible: false,
        family_id: ActionFamilyId::Guard,
        target_player: source_index,
        commit_tick,
        contact_tick: 0,
        horizon_ticks: commit_tick + cap,
    };
    if paths.is_empty() {
        return witness;
    }
    let guard_arc = (family.front_arc_degrees / 2.0).to_radians().cos();
    for relative in family.windup_ticks..=cap {
        let (sx, sy, fx, fy) = committed_source_pose(observation, tape, family, relative);
        for path in &paths {
            if relative < path.start_tick || relative > path.last_tick {
                continue;
            }
            let Some((hx, hy)) = hostile_contact_point(observation, path, relative) else {
                continue;
            };
            let dx = hx - sx;
            let dy = hy - sy;
            let distance = length(dx, dy);
            let mut reach = observation.own.radius;
            if let Some(source) = path.source {
                if path.projectile.is_none() {
                    reach += source.projected_reach_px;
                }
                if source.family_id == Some(ActionFamilyId::Ranged) {
                    reach = observation.own.radius;
                }
            }
            let inside_arc = distance <= EPSILON
                || (dx / distance) * fx + (dy / distance) * fy + EPSILON >= guard_arc;
            if distance <= reach && inside_arc {
                witness.feasible = true;
                witness.contact_tick = commit_tick + relative;
                return witness;
            }
        }
    }
    witness
}

/// `family_commit_feasibility/v1`. Family feasibility ignores which family is
/// actually equipped and ignores actual cooldown, recovery, commitment,
/// request acceptance, and hit outcome; those stay separate policy inputs.
///
/// The Lua original asserts the family id is a known `ActionFamilyId`; here
/// `family_id: ActionFamilyId` makes that state unrepresentable, so the
/// assertion is dropped as structurally redundant (README rule 9 / the same
/// principle `input_frame`'s port documents).
#[must_use]
pub fn family_commit(
    observation: &CombatObservation,
    target_player: i64,
    family_id: ActionFamilyId,
    tape: Option<&CombatWitnessTape>,
) -> CombatFeasibilityWitness {
    if family_id == ActionFamilyId::Guard {
        return guard_feasibility(observation, target_player, tape);
    }
    let commit_tick = tape.map_or(0, |tape| tape.ticks);
    let infeasible = || CombatFeasibilityWitness {
        feasible: false,
        family_id,
        target_player,
        commit_tick,
        contact_tick: 0,
        horizon_ticks: commit_tick,
    };
    let Some(target) = row_for(observation, target_player) else {
        return infeasible();
    };
    if target.is_keeper || target.team == observation.own.team {
        return infeasible();
    }
    if family_id == ActionFamilyId::Ranged {
        ranged_feasibility(observation, target, tape)
    } else {
        melee_feasibility(observation, target, action_families::get(family_id), tape)
    }
}

fn ball_owner_row(observation: &CombatObservation) -> Option<&CombatObservationPeer> {
    let owner = observation.ball.owner_player_index;
    if owner == 0 || owner == observation.own.player_index {
        return None;
    }
    row_for(observation, owner)
}

fn nearest_opponent(observation: &CombatObservation, gx: f64, gy: f64) -> Option<i64> {
    let mut best_index: Option<i64> = None;
    let mut best_distance = f64::INFINITY;
    for row in &observation.opponents {
        if row.is_keeper {
            continue;
        }
        let distance = length(row.x - gx, row.y - gy);
        if distance < best_distance - EPSILON {
            best_distance = distance;
            best_index = Some(row.player_index);
        } else if (distance - best_distance).abs() <= EPSILON
            && let Some(current) = best_index
        {
            best_index = Some(current.min(row.player_index));
        }
    }
    best_index
}

fn carrier_protection_predicate(
    observation: &CombatObservation,
    target: &CombatObservationPeer,
) -> bool {
    if observation.ball.owner_team != Some(observation.own.team) {
        return false;
    }
    let owner_index = observation.ball.owner_player_index;
    if owner_index == 0 {
        return false;
    }
    let (owner_x, owner_y) = if owner_index == observation.own.player_index {
        (observation.own.x, observation.own.y)
    } else {
        match row_for(observation, owner_index) {
            Some(owner) => (owner.x, owner.y),
            None => return false,
        }
    };
    if length(target.x - owner_x, target.y - owner_y) > CARRIER_PROTECTION_PX {
        return false;
    }
    nearest_opponent(observation, owner_x, owner_y) == Some(target.player_index)
}

fn loose_ball_predicate(observation: &CombatObservation, target: &CombatObservationPeer) -> bool {
    if observation.ball.owner_player_index != 0 {
        return false;
    }
    let own = &observation.own;
    let radius = LOOSE_BALL_PX;
    length(own.x - observation.ball.x, own.y - observation.ball.y) <= radius
        && length(target.x - observation.ball.x, target.y - observation.ball.y) <= radius
}

fn attack_goal_center(observation: &CombatObservation, team: Team) -> (f64, f64) {
    let match_view = &observation.match_view;
    if team == Team::Home {
        (
            match_view.goal_away_x + match_view.goal_away_w,
            match_view.goal_away_y + match_view.goal_away_h / 2.0,
        )
    } else {
        (
            match_view.goal_home_x,
            match_view.goal_home_y + match_view.goal_home_h / 2.0,
        )
    }
}

// Sole blocker of a frozen candidate passing segment, the frozen likely
// shooter, or the controller of an open shot lane.
fn lane_or_shot_predicate(observation: &CombatObservation, target: &CombatObservationPeer) -> bool {
    if target.team == observation.own.team {
        return false;
    }
    let own = &observation.own;
    if observation.ball.owner_team == Some(own.team) {
        // Our possession: the target denies one of our frozen passing
        // segments and is the only opponent standing in it.
        let owner_index = observation.ball.owner_player_index;
        let (ox, oy) = if owner_index == own.player_index {
            (own.x, own.y)
        } else {
            match row_for(observation, owner_index) {
                Some(owner) => (owner.x, owner.y),
                None => return false,
            }
        };
        for mate in &observation.teammates {
            if mate.is_keeper || mate.player_index == owner_index {
                continue;
            }
            let mut blockers = 0;
            let mut sole: i64 = 0;
            for row in &observation.opponents {
                if !row.is_keeper
                    && segment_hits_circle(
                        ox,
                        oy,
                        mate.x,
                        mate.y,
                        row.x,
                        row.y,
                        row.radius + LANE_CORRIDOR_PX,
                    )
                {
                    blockers += 1;
                    sole = row.player_index;
                }
            }
            if blockers == 1 && sole == target.player_index {
                return true;
            }
        }
        return false;
    }

    // Their possession or a loose ball: the target is the frozen likely
    // shooter or controls an open shot lane at our goal.
    let (gx, gy) = attack_goal_center(observation, target.team);
    if length(target.x - gx, target.y - gy) > SHOT_RANGE_PX {
        return false;
    }
    for row in &observation.teammates {
        if !row.is_keeper
            && segment_hits_circle(
                target.x,
                target.y,
                gx,
                gy,
                row.x,
                row.y,
                row.radius + LANE_CORRIDOR_PX,
            )
        {
            return false;
        }
    }
    if observation.ball.owner_player_index == target.player_index {
        return true;
    }
    // Not the carrier: only the nearest opposing outfielder to our goal
    // counts as the frozen likely shooter, so a whole team is never in this
    // bucket.
    nearest_opponent(observation, gx, gy) == Some(target.player_index)
}

/// Which purpose predicates hold for one (source, target) pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CombatPurposeBitset {
    /// The target is the opposing ball carrier.
    pub carrier_contest: bool,
    /// The target is the nearest opponent to our own carrier.
    pub carrier_protection: bool,
    /// The target and the source both sit within reach of a loose ball.
    pub loose_ball_contest: bool,
    /// The target is the sole blocker of a passing lane or shot lane.
    pub passing_lane_or_shot_denial: bool,
    /// The target is mid-recovery on top of another true purpose.
    pub recovery_punish: bool,
}

impl CombatPurposeBitset {
    /// Whether `purpose` is set in this bitset.
    #[must_use]
    pub fn get(self, purpose: CombatPurposeId) -> bool {
        match purpose {
            CombatPurposeId::CarrierContest => self.carrier_contest,
            CombatPurposeId::CarrierProtection => self.carrier_protection,
            CombatPurposeId::LooseBallContest => self.loose_ball_contest,
            CombatPurposeId::PassingLaneOrShotDenial => self.passing_lane_or_shot_denial,
            CombatPurposeId::RecoveryPunish => self.recovery_punish,
        }
    }

    fn set(&mut self, purpose: CombatPurposeId) {
        match purpose {
            CombatPurposeId::CarrierContest => self.carrier_contest = true,
            CombatPurposeId::CarrierProtection => self.carrier_protection = true,
            CombatPurposeId::LooseBallContest => self.loose_ball_contest = true,
            CombatPurposeId::PassingLaneOrShotDenial => self.passing_lane_or_shot_denial = true,
            CombatPurposeId::RecoveryPunish => self.recovery_punish = true,
        }
    }
}

/// Every true purpose predicate for one (source, target) pair, as a bitset,
/// plus how many are set.
#[must_use]
pub fn purpose_bitset(
    observation: &CombatObservation,
    target_player: i64,
) -> (CombatPurposeBitset, i64) {
    let mut bitset = CombatPurposeBitset::default();
    let Some(target) = row_for(observation, target_player) else {
        return (bitset, 0);
    };
    if target.is_keeper || target.team == observation.own.team {
        return (bitset, 0);
    }
    let mut count: i64 = 0;
    if let Some(carrier) = ball_owner_row(observation)
        && carrier.player_index == target.player_index
        && !carrier.is_keeper
    {
        bitset.set(CombatPurposeId::CarrierContest);
        count += 1;
    }
    if carrier_protection_predicate(observation, target) {
        bitset.set(CombatPurposeId::CarrierProtection);
        count += 1;
    }
    if loose_ball_predicate(observation, target) {
        bitset.set(CombatPurposeId::LooseBallContest);
        count += 1;
    }
    if lane_or_shot_predicate(observation, target) {
        bitset.set(CombatPurposeId::PassingLaneOrShotDenial);
        count += 1;
    }
    // Recovery alone is diagnostic. It becomes a purpose only on top of one
    // of the four ball/lane predicates.
    if target.phase == CombatActionPhase::Recovery && count > 0 {
        bitset.set(CombatPurposeId::RecoveryPunish);
        count += 1;
    }
    (bitset, count)
}

/// The single most specific true purpose in `bitset`, per [`PURPOSE_PRIORITY`].
#[must_use]
pub fn dominant_purpose(bitset: CombatPurposeBitset) -> Option<CombatPurposeId> {
    PURPOSE_PRIORITY
        .into_iter()
        .find(|&purpose| bitset.get(purpose))
}

/// `formation_risk_tradeoff`: a cost flag, never a purpose. Committing raises
/// it when the source is already far from its authored anchor or when the
/// projected committed motion adds at least the contract's gain.
#[must_use]
pub fn formation_risk(observation: &CombatObservation, family_id: ActionFamilyId) -> bool {
    let own = &observation.own;
    let Some(anchor) = observation
        .anchors
        .iter()
        .find(|row| row.player_index == own.player_index)
    else {
        return false;
    };
    let now = length(own.x - anchor.anchor_x, own.y - anchor.anchor_y);
    if now > FORMATION_RISK_ANCHOR_PX {
        return true;
    }
    let family = action_families::get(family_id);
    let horizon = family.windup_ticks + family.active_ticks.unwrap_or(0) + family.recovery_ticks;
    let seconds = horizon as f64 * fixed_clock::TICK_SECONDS;
    let scale = family.movement_multiplier;
    let projected_x = (own.x + own.vx * scale * seconds)
        .clamp(own.radius, observation.match_view.field_w - own.radius);
    let projected_y = (own.y + own.vy * scale * seconds)
        .clamp(own.radius, observation.match_view.field_h - own.radius);
    let later = length(projected_x - anchor.anchor_x, projected_y - anchor.anchor_y);
    later - now >= FORMATION_RISK_GAIN_PX
}

/// The canonical movement alphabet the envelope search varies. Order is
/// frozen: neutral first, then the eight compass directions clockwise from
/// +x.
pub const MOVE_ALPHABET: [(f64, f64); 9] = [
    (0.0, 0.0),
    (1.0, 0.0),
    (1.0, 1.0),
    (0.0, 1.0),
    (-1.0, 1.0),
    (-1.0, 0.0),
    (-1.0, -1.0),
    (0.0, -1.0),
    (1.0, -1.0),
];

/// Options for [`intervention_candidates`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CombatEnvelopeOptions {
    /// Overrides [`SEARCH_TICKS`].
    pub search_ticks: Option<i64>,
    /// Overrides the default catalogued families (all four).
    pub families: Option<Vec<ActionFamilyId>>,
}

/// Which of the four catalogued families a searched pose proved feasible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CombatFamilyBitset {
    /// The unarmed family was proven feasible.
    pub unarmed: bool,
    /// The guard family was proven feasible.
    pub guard: bool,
    /// The light melee family was proven feasible.
    pub light_melee: bool,
    /// The ranged family was proven feasible.
    pub ranged: bool,
}

impl CombatFamilyBitset {
    /// Whether `family_id` is set in this bitset.
    #[must_use]
    pub fn get(self, family_id: ActionFamilyId) -> bool {
        match family_id {
            ActionFamilyId::Unarmed => self.unarmed,
            ActionFamilyId::Guard => self.guard,
            ActionFamilyId::LightMelee => self.light_melee,
            ActionFamilyId::Ranged => self.ranged,
        }
    }

    fn set(&mut self, family_id: ActionFamilyId) {
        match family_id {
            ActionFamilyId::Unarmed => self.unarmed = true,
            ActionFamilyId::Guard => self.guard = true,
            ActionFamilyId::LightMelee => self.light_melee = true,
            ActionFamilyId::Ranged => self.ranged = true,
        }
    }
}

/// One `(target, purpose)` pair the family-neutral envelope search admitted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatInterventionPair {
    /// The frozen target.
    pub target_player: i64,
    /// The purpose that made this pair eligible.
    pub purpose: CombatPurposeId,
    /// The soonest commit tick any searched pose proved feasible for.
    pub commit_tick: i64,
    /// Which of the searched families were feasible at `commit_tick` or
    /// earlier in the search.
    pub family_bitset: CombatFamilyBitset,
    /// Always `false`: `formation_risk_tradeoff` is computed by a caller
    /// separately, never inside the envelope search itself. Preserved
    /// verbatim from the Lua original, which never assigns anything else
    /// here.
    pub formation_risk: bool,
}

const DEFAULT_ENVELOPE_FAMILIES: [ActionFamilyId; 4] = [
    ActionFamilyId::Unarmed,
    ActionFamilyId::Guard,
    ActionFamilyId::LightMelee,
    ActionFamilyId::Ranged,
];

/// `intervention_candidate/v2`: the family-neutral feasibility envelope. A
/// `(target, purpose)` pair enters only when its purpose predicate is true
/// and at least one searched source pose makes `family_commit_feasibility/v1`
/// true for at least one of the four catalogued families toward the same
/// frozen identity.
///
/// The search varies only the source's canonical legal movement/facing
/// inputs, for at most `search_ticks`. Every non-source trajectory is the
/// public no-response projection. Search order is the frozen movement
/// alphabet crossed with the ascending commit tick, so the first feasible
/// commit tick is stable.
#[must_use]
pub fn intervention_candidates(
    observation: &CombatObservation,
    options: Option<&CombatEnvelopeOptions>,
) -> Vec<CombatInterventionPair> {
    let search_ticks = options
        .and_then(|options| options.search_ticks)
        .unwrap_or(SEARCH_TICKS);
    let families: &[ActionFamilyId] = options
        .and_then(|options| options.families.as_deref())
        .unwrap_or(&DEFAULT_ENVELOPE_FAMILIES);

    let mut pairs_out = Vec::new();
    for target in &observation.opponents {
        if target.is_keeper {
            continue;
        }
        let (bitset, count) = purpose_bitset(observation, target.player_index);
        if count <= 0 {
            continue;
        }
        let mut best_tick: Option<i64> = None;
        let mut family_bitset = CombatFamilyBitset::default();
        for commit_tick in 0..=search_ticks {
            for &(move_x, move_y) in &MOVE_ALPHABET {
                let tape = if commit_tick == 0 {
                    None
                } else {
                    Some(CombatWitnessTape {
                        move_x,
                        move_y,
                        ticks: commit_tick,
                    })
                };
                for &family_id in families {
                    if family_bitset.get(family_id) {
                        continue;
                    }
                    let witness =
                        family_commit(observation, target.player_index, family_id, tape.as_ref());
                    if witness.feasible {
                        family_bitset.set(family_id);
                        if best_tick.is_none() {
                            best_tick = Some(commit_tick);
                        }
                    }
                }
                if commit_tick == 0 {
                    break;
                }
            }
        }
        if let Some(best_tick) = best_tick {
            for &purpose in &PURPOSE_PRIORITY {
                if bitset.get(purpose) {
                    pairs_out.push(CombatInterventionPair {
                        target_player: target.player_index,
                        purpose,
                        commit_tick: best_tick,
                        family_bitset,
                        formation_risk: false,
                    });
                }
            }
        }
    }
    pairs_out.sort_by(|left, right| {
        if left.target_player != right.target_player {
            left.target_player.cmp(&right.target_player)
        } else {
            left.purpose.cmp(&right.purpose)
        }
    });
    pairs_out
}

/// The soonest tick a public hostile path would reach this player's body,
/// inside `window_ticks`. This is the spacing/evade read: it asks only
/// whether a telegraphed threat lands, never whether it would hit, be
/// guarded, or be superseded.
///
/// Also carries where the threat IS, which is not the same thing as where
/// its source player stands. An in-flight projectile's shooter may be across
/// the pitch, or dead astern of it; a caller that wants to step out of the
/// way has to read the projectile, not the body that launched it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatIncomingThreat {
    /// Ticks until the threat reaches this player's body.
    pub ticks_to_contact: i64,
    /// Canonical index of the threat's source player.
    pub source_player: i64,
    /// Current public x position of the arriving threat.
    pub threat_x: f64,
    /// Current public y position of the arriving threat.
    pub threat_y: f64,
}

/// See [`CombatIncomingThreat`]. Returns `None` when no public hostile path
/// reaches this player's body inside `window_ticks`.
#[must_use]
pub fn incoming_threat(
    observation: &CombatObservation,
    window_ticks: i64,
) -> Option<CombatIncomingThreat> {
    let mut best: Option<CombatIncomingThreat> = None;
    for path in hostile_paths(observation, None) {
        let last = path.last_tick.min(window_ticks);
        for tick in path.start_tick..=last {
            let Some((hx, hy)) = hostile_contact_point(observation, &path, tick) else {
                continue;
            };
            let body = project(&observation.own, observation, tick);
            let mut reach = body.radius;
            if let Some(source) = path.source {
                if path.projectile.is_none() {
                    reach += source.projected_reach_px;
                }
                if source.family_id == Some(ActionFamilyId::Ranged) {
                    reach = body.radius;
                }
            }
            if length(hx - body.x, hy - body.y) <= reach {
                let source_player = path.source.map_or_else(
                    || {
                        path.projectile
                            .expect("path always has a source or projectile")
                            .source_player_index
                    },
                    |row| row.player_index,
                );
                let (threat_x, threat_y) = if let Some(projectile) = path.projectile {
                    (projectile.x, projectile.y)
                } else {
                    let source = path.source.expect("path always has a source or projectile");
                    (source.x, source.y)
                };
                let replace = match best {
                    None => true,
                    Some(current) => {
                        tick < current.ticks_to_contact
                            || (tick == current.ticks_to_contact
                                && source_player < current.source_player)
                    }
                };
                if replace {
                    best = Some(CombatIncomingThreat {
                        ticks_to_contact: tick,
                        source_player,
                        threat_x,
                        threat_y,
                    });
                }
                break;
            }
        }
    }
    best
}
