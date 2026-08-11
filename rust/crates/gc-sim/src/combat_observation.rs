//! `combat_sim_observation/v1`: the only observation schema available inside
//! `sim::` and to the shipped gameplay AI.
//!
//! It carries authoritative PUBLIC simulation state and nothing else. Every
//! field is something a presented match already shows or a public catalog
//! already publishes. Pixels, viewport, theme, presentation/loadout
//! identity, render timing, unresolved outcomes, hidden collision results,
//! future input, and RNG state can never enter — [`FORBIDDEN_FIELDS`] names
//! the ones a caller could plausibly try to smuggle in, and
//! `combat_observation_leakage_spec` fails if one does.
//!
//! Rows are canonically ordered: teammate/opponent rows follow the
//! canonical player index; projectile rows follow `(source_sequence,
//! source_player_id, projectile_id)`. No field is ever absent: optional
//! values materialize as explicit sentinels (`0` for integers, `None`/
//! [`ActionFamilyId`] absent for "none", `false` for flags) rather than an
//! omitted key — a total Rust struct gives this for free.
//!
//! The digest is FNV-1a-64 over the canonical encoding below, mirroring
//! [`crate::match_snapshot::number_bytes`]'s scalar format exactly (this
//! module reuses it) — see `docs/design/combat_fun_evidence_contract.md`
//! §4.0/§4.7.
//!
//! ## Bridging to `combat_feasibility`
//!
//! [`crate::combat_feasibility`] declares its own narrower local
//! `CombatObservation*` view rather than depending on this module's full
//! schema, keeping that leaf module's dependency surface minimal (see that
//! module's doc comment). [`to_feasibility_view`] is the bridge: a pure
//! projection from this module's full schema down to `combat_feasibility`'s
//! narrower read shape, so [`crate::combat_policy`] and [`crate::combat`]
//! can call the feasibility predicates without this module duplicating
//! their logic.

use crate::combat_feasibility::{self, CombatActionPhase, Team as FeasibilityTeam};
use crate::combat_snapshot::{CombatForcedState, CombatMatchState, action_family_wire_id};
use crate::fixed_clock;
use crate::match_snapshot::{MatchState, number_bytes};
use gc_core::fnv1a64;
use gc_data::action_families::{self, ActionFamilyId};

/// Schema tag.
pub const SCHEMA: &str = "combat_sim_observation/v1";
/// Schema version.
pub const VERSION: i64 = 1;
/// The frozen static-collision rule this schema's projections assume:
/// public motion advances linearly at the fixed tick and is clamped to the
/// pitch by the player radius. [`crate::combat_feasibility`] uses exactly
/// this rule, so the feasibility predicate and the observation cannot drift
/// apart silently.
pub const COLLISION_CATALOG_VERSION: &str = "pitch.linear_clamp.v1";

/// Fields a caller could plausibly try to smuggle in. `validate` already
/// rejects every undeclared key by construction (a typed struct has no
/// undeclared fields); this is the named subset section 4.7 explicitly
/// excludes, kept as documentation/spec fixture data rather than an
/// enforcement mechanism.
pub const FORBIDDEN_FIELDS: [&str; 15] = [
    "loadout_id",
    "presentation_id",
    "cosmetic_variant_id",
    "theme",
    "viewport",
    "pixels",
    "render_tick",
    "frame_time",
    "rng",
    "rng_state",
    "snapshot",
    "future_input",
    "pending_contact",
    "contact_result",
    "outcome",
];

const FAMILY_ORDER: [ActionFamilyId; 4] = [
    ActionFamilyId::Unarmed,
    ActionFamilyId::Guard,
    ActionFamilyId::LightMelee,
    ActionFamilyId::Ranged,
];

/// The soccer action a row is publicly engaged in right now, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoccerAction {
    /// No soccer action.
    None,
    /// A slide tackle.
    Slide,
    /// A standing tackle.
    Tackle,
    /// A jockey stance.
    Jockey,
    /// A dodge/juke.
    Dodge,
    /// An aerial attempt or its recovery.
    Aerial,
    /// A shot/punt windup.
    Windup,
    /// Running onto an incoming pass.
    Receive,
}

impl SoccerAction {
    fn wire_str(self) -> &'static str {
        match self {
            SoccerAction::None => "none",
            SoccerAction::Slide => "slide",
            SoccerAction::Tackle => "tackle",
            SoccerAction::Jockey => "jockey",
            SoccerAction::Dodge => "dodge",
            SoccerAction::Aerial => "aerial",
            SoccerAction::Windup => "windup",
            SoccerAction::Receive => "receive",
        }
    }
}

fn forced_state_wire(v: Option<CombatForcedState>) -> &'static str {
    match v {
        None => "none",
        Some(CombatForcedState::Stagger) => "stagger",
        Some(CombatForcedState::Knockback) => "knockback",
    }
}

fn family_wire(v: Option<ActionFamilyId>) -> &'static str {
    v.map_or("none", action_family_wire_id)
}

fn phase_wire(v: CombatActionPhase) -> &'static str {
    crate::combat_snapshot::phase_wire_str(v)
}

/// A fixture side, matching `InputTeam`. Reuses
/// [`crate::combat_feasibility::Team`] rather than declaring a fourth: this
/// schema feeds `combat_feasibility` directly via [`to_feasibility_view`],
/// so sharing the type makes that bridge a pure field copy instead of a
/// conversion.
pub type Team = FeasibilityTeam;

fn team_wire(v: Team) -> &'static str {
    match v {
        Team::Home => "home",
        Team::Away => "away",
    }
}

/// This player's own row: `CombatObservationSelf`.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfRow {
    /// Canonical player index (one-based).
    pub player_index: i64,
    /// Stable player id.
    pub player_id: String,
    /// Canonical input slot, or 0 when this player owns none.
    pub slot: i64,
    /// Fixture side.
    pub team: Team,
    /// Whether this row is the keeper.
    pub is_keeper: bool,
    /// Currently equipped family, or `None` without a loadout.
    pub family_id: Option<ActionFamilyId>,
    /// 0 when no action is accepted.
    pub source_sequence: i64,
    /// The public soccer action this row is engaged in.
    pub soccer_action: SoccerAction,
    /// Whether a soccer commitment is in progress.
    pub soccer_committed: bool,
    /// Current combat action phase.
    pub phase: CombatActionPhase,
    /// Ticks spent in the current phase.
    pub phase_ticks: i64,
    /// Current forced (stagger/knockback) state, if any.
    pub forced_state: Option<CombatForcedState>,
    /// Ticks remaining in the current forced state.
    pub forced_ticks: i64,
    /// Ticks remaining in the post-hit immunity window.
    pub immunity_ticks: i64,
    /// Ticks remaining before this player's action can commit again.
    pub cooldown_ticks: i64,
    /// Whether this row can commit a fresh action right now.
    pub ready: bool,
    /// Whether the equipment control is currently held.
    pub control_held: bool,
    /// Whether a ranged release has publicly latched.
    pub release_latched: bool,
    /// Absolute tick a latched ranged projectile is projected to spawn on;
    /// 0 unless ranged and latched.
    pub projected_spawn_tick: i64,
    /// World x position.
    pub x: f64,
    /// World y position.
    pub y: f64,
    /// World x velocity.
    pub vx: f64,
    /// World y velocity.
    pub vy: f64,
    /// Unit facing x.
    pub facing_x: f64,
    /// Unit facing y.
    pub facing_y: f64,
    /// Collision radius.
    pub radius: f64,
    /// Own base locomotion speed, world px/s.
    pub move_speed: f64,
    /// Own materialized input through the observed tick: equipment held.
    pub last_equipment_held: bool,
    /// Own materialized input: equipment pressed.
    pub last_equipment_pressed: bool,
    /// Own materialized input: equipment released.
    pub last_equipment_released: bool,
}

/// A teammate or opponent row: `CombatObservationPeer`.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerRow {
    /// Canonical player index (one-based).
    pub player_index: i64,
    /// Stable player id.
    pub player_id: String,
    /// Canonical input slot, or 0 when this player owns none.
    pub slot: i64,
    /// Fixture side.
    pub team: Team,
    /// Whether this row is the keeper.
    pub is_keeper: bool,
    /// The public soccer action this row is engaged in.
    pub soccer_action: SoccerAction,
    /// Whether a soccer commitment is in progress.
    pub soccer_committed: bool,
    /// Currently equipped family, or `None` without a loadout.
    pub family_id: Option<ActionFamilyId>,
    /// 0 when no action is accepted.
    pub source_sequence: i64,
    /// Current combat action phase.
    pub phase: CombatActionPhase,
    /// Ticks spent in the current phase.
    pub phase_ticks: i64,
    /// Current forced state, if any.
    pub forced_state: Option<CombatForcedState>,
    /// Ticks remaining in the current forced state.
    pub forced_ticks: i64,
    /// Ticks remaining in the post-hit immunity window.
    pub immunity_ticks: i64,
    /// Whether this row is actively guarding right now.
    pub guard_active: bool,
    /// First relative tick of the public telegraph, 0 when none.
    pub telegraph_start_tick: i64,
    /// Last relative tick of the public telegraph, 0 when unbounded/none.
    pub telegraph_end_tick: i64,
    /// Catalogued melee reach, px; 0 off the melee families.
    pub projected_reach_px: f64,
    /// Catalogued frontal arc, degrees; 0 without a family.
    pub projected_arc_degrees: f64,
    /// Whether a ranged release has publicly latched.
    pub release_latched: bool,
    /// Absolute tick a latched ranged projectile is projected to spawn on.
    pub projected_spawn_tick: i64,
    /// World x position.
    pub x: f64,
    /// World y position.
    pub y: f64,
    /// World x velocity.
    pub vx: f64,
    /// World y velocity.
    pub vy: f64,
    /// Unit facing x.
    pub facing_x: f64,
    /// Unit facing y.
    pub facing_y: f64,
    /// Collision radius.
    pub radius: f64,
}

/// An in-flight projectile row: `CombatObservationProjectile`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectileRow {
    /// Stable within-observation identity.
    pub projectile_id: String,
    /// Firing player's canonical index.
    pub source_player_index: i64,
    /// Firing player's stable id.
    pub source_player_id: String,
    /// Firing player's fixture side.
    pub source_team: Team,
    /// Firing action's source sequence.
    pub source_sequence: i64,
    /// Always [`ActionFamilyId::Ranged`].
    pub family_id: ActionFamilyId,
    /// Current x position.
    pub x: f64,
    /// Current y position.
    pub y: f64,
    /// Unit travel direction x.
    pub dir_x: f64,
    /// Unit travel direction y.
    pub dir_y: f64,
    /// World x velocity.
    pub vx: f64,
    /// World y velocity.
    pub vy: f64,
    /// Always `true` (only in-flight projectiles are observed).
    pub in_flight: bool,
    /// Ticks remaining before this projectile expires.
    pub remaining_lifetime_ticks: i64,
    /// Ticks until either the lifetime or the pitch bound is reached,
    /// whichever comes first.
    pub horizon_ticks: i64,
}

/// Public ball state: `CombatObservationBall`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallRow {
    /// Current x position.
    pub x: f64,
    /// Current y position.
    pub y: f64,
    /// Height above the pitch.
    pub z: f64,
    /// World x velocity.
    pub vx: f64,
    /// World y velocity.
    pub vy: f64,
    /// Vertical velocity.
    pub vz: f64,
    /// Whether the ball is loose.
    pub loose: bool,
    /// Owner's canonical index, or 0 when loose.
    pub owner_player_index: i64,
    /// Owner's fixture side, or `None` when loose.
    pub owner_team: Option<Team>,
    /// Whether the owner is the keeper.
    pub owner_is_keeper: bool,
}

/// Static pitch/goal geometry and match phase: `CombatObservationMatch`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchRow {
    /// The tick this observation was produced for.
    pub tick: i64,
    /// Simulation tick rate, Hz.
    pub tick_rate: f64,
    /// Playable pitch width.
    pub field_w: f64,
    /// Playable pitch height.
    pub field_h: f64,
    /// Home goal mouth: left edge x.
    pub goal_home_x: f64,
    /// Home goal mouth: top edge y.
    pub goal_home_y: f64,
    /// Home goal mouth: width.
    pub goal_home_w: f64,
    /// Home goal mouth: height.
    pub goal_home_h: f64,
    /// Away goal mouth: left edge x.
    pub goal_away_x: f64,
    /// Away goal mouth: top edge y.
    pub goal_away_y: f64,
    /// Away goal mouth: width.
    pub goal_away_w: f64,
    /// Away goal mouth: height.
    pub goal_away_h: f64,
    /// Home score.
    pub score_home: i64,
    /// Away score.
    pub score_away: i64,
    /// Ticks remaining in the match.
    pub remaining_ticks: i64,
    /// Whether a kickoff hold is in effect.
    pub stoppage: bool,
    /// Ticks remaining in the kickoff hold.
    pub stoppage_ticks: i64,
    /// Whether the match has finished.
    pub finished: bool,
}

/// One player's authored formation anchor: `CombatObservationAnchor`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchorRow {
    /// Canonical player index.
    pub player_index: i64,
    /// Stable formation-slot identity (`"<formation_id>/keeper"` or
    /// `"<formation_id>/outfield/<ordinal>"`).
    pub formation_slot_id: String,
    /// Authored anchor x.
    pub anchor_x: f64,
    /// Authored anchor y.
    pub anchor_y: f64,
}

/// The complete `combat_sim_observation/v1` record.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatObservation {
    /// [`SCHEMA`].
    pub schema: String,
    /// [`VERSION`].
    pub version: i64,
    /// The producing policy's id.
    pub policy_id: String,
    /// The tick this observation was produced on.
    pub producer_tick: i64,
    /// The tick this observation describes.
    pub observed_tick: i64,
    /// The catalog digest of every action family's full definition.
    pub family_catalog_version: String,
    /// [`COLLISION_CATALOG_VERSION`].
    pub collision_catalog_version: String,
    /// Canonical player index order (`1..=players.len()`).
    pub player_order: Vec<i64>,
    /// Canonical projectile id order, matching `projectiles`.
    pub projectile_order: Vec<String>,
    /// This player's own row.
    pub own: SelfRow,
    /// Own-side rows, excluding `own`.
    pub teammates: Vec<PeerRow>,
    /// Opposing-side rows.
    pub opponents: Vec<PeerRow>,
    /// In-flight projectile rows.
    pub projectiles: Vec<ProjectileRow>,
    /// Public ball state.
    pub ball: BallRow,
    /// Static pitch/goal geometry and match phase.
    pub match_row: MatchRow,
    /// Authored formation anchors, in canonical player order.
    pub anchors: Vec<AnchorRow>,
    /// FNV-1a-64 digest over [`encode`]'s output.
    pub digest: String,
}

/// The catalog digest of every action family's full definition plus the
/// shared combat rule constants: `"family_catalog." <> fnv1a64(...)`.
/// Recomputed on demand rather than cached — it is a pure function of
/// `gc-data`'s content and never changes within a build.
#[must_use]
pub fn family_catalog_version() -> String {
    let mut parts = String::from("GCCV;1;");
    append_str(&mut parts, "MAX_DISABLE_TICKS");
    append_num(&mut parts, crate::combat_rules::MAX_DISABLE_TICKS as f64);
    append_str(&mut parts, "IMMUNITY_TICKS");
    append_num(&mut parts, crate::combat_rules::IMMUNITY_TICKS as f64);
    append_str(&mut parts, "KNOCKBACK_THRESHOLD_PX");
    append_num(&mut parts, crate::combat_rules::KNOCKBACK_THRESHOLD_PX);
    for family_id in FAMILY_ORDER {
        let family = action_families::get(family_id);
        append_str(&mut parts, action_family_wire_id(family_id));
        append_str(&mut parts, "id");
        append_str(&mut parts, action_family_wire_id(family.id));
        append_str(&mut parts, "activation");
        append_str(
            &mut parts,
            match family.activation {
                gc_data::action_families::ActionActivation::Press => "press",
                gc_data::action_families::ActionActivation::Held => "held",
                gc_data::action_families::ActionActivation::HeldRelease => "held_release",
            },
        );
        append_str(&mut parts, "contact_kind");
        append_str(
            &mut parts,
            match family.contact_kind {
                gc_data::action_families::ActionContactKind::Melee => "melee",
                gc_data::action_families::ActionContactKind::Guard => "guard",
                gc_data::action_families::ActionContactKind::Projectile => "projectile",
            },
        );
        append_str(&mut parts, "windup_ticks");
        append_num(&mut parts, family.windup_ticks as f64);
        append_str(&mut parts, "active_ticks");
        append_num(&mut parts, family.active_ticks.unwrap_or(0) as f64);
        append_str(&mut parts, "held_active");
        append_bool(&mut parts, family.held_active);
        append_str(&mut parts, "recovery_ticks");
        append_num(&mut parts, family.recovery_ticks as f64);
        append_str(&mut parts, "cooldown_ticks");
        append_num(&mut parts, family.cooldown_ticks as f64);
        append_str(&mut parts, "reach_px");
        append_num(&mut parts, family.reach_px.unwrap_or(0.0));
        append_str(&mut parts, "projectile_speed_px_per_second");
        append_num(
            &mut parts,
            family.projectile_speed_px_per_second.unwrap_or(0.0),
        );
        append_str(&mut parts, "projectile_lifetime_ticks");
        append_num(
            &mut parts,
            family.projectile_lifetime_ticks.unwrap_or(0) as f64,
        );
        append_str(&mut parts, "front_arc_degrees");
        append_num(&mut parts, family.front_arc_degrees);
        append_str(&mut parts, "movement_multiplier");
        append_num(&mut parts, family.movement_multiplier);
        append_str(&mut parts, "guarded_recoil_px");
        append_num(&mut parts, family.guarded_recoil_px);

        append_bool(&mut parts, family.unguarded_outcome.is_some());
        if let Some(outcome) = &family.unguarded_outcome {
            append_str(&mut parts, "interruption_ticks");
            append_num(&mut parts, outcome.interruption_ticks as f64);
            append_str(&mut parts, "displacement_px");
            append_num(&mut parts, outcome.displacement_px);
            append_str(&mut parts, "ball_spill");
            append_bool(&mut parts, outcome.ball_spill);
        }
    }
    format!("family_catalog.{}", fnv1a64::hash(parts.as_bytes()))
}

fn append_str(parts: &mut String, s: &str) {
    parts.push('s');
    parts.push_str(&s.len().to_string());
    parts.push(':');
    parts.push_str(s);
    parts.push(';');
}
fn append_num(parts: &mut String, n: f64) {
    parts.push('n');
    parts.push_str(&number_bytes(n));
    parts.push(';');
}
fn append_bool(parts: &mut String, b: bool) {
    parts.push_str(if b { "b1;" } else { "b0;" });
}
fn append_i(parts: &mut String, n: i64) {
    append_num(parts, n as f64);
}

/// `combat.telegraph`: the public telegraph another player can see, or
/// `None` without a combat companion or without an equipped family.
#[must_use]
pub fn telegraph(
    combat_state: Option<&CombatMatchState>,
    player_index: i64,
) -> Option<(ActionFamilyId, CombatActionPhase, Option<CombatForcedState>)> {
    let combat_state = combat_state?;
    let runtime = combat_state.players.get(player_index as usize - 1)?;
    let family_id = runtime.family_id?;
    Some((family_id, runtime.phase, runtime.forced_state))
}

/// A stable within-observation projectile identity:
/// `"<len(source_player_id)>:<source_player_id>/<source_sequence>"`.
#[must_use]
pub fn projectile_id(source_player_id: &str, source_sequence: i64) -> String {
    format!(
        "{}:{source_player_id}/{source_sequence}",
        source_player_id.len()
    )
}

fn soccer_action_of(player: &crate::match_snapshot::MatchPlayer) -> SoccerAction {
    if player.slide_timer > 0.0 {
        SoccerAction::Slide
    } else if player.dodge_timer > 0.0 {
        SoccerAction::Dodge
    } else if player.tackle_timer > 0.0 {
        SoccerAction::Tackle
    } else if player.aerial_timer > 0.0 || player.aerial_recovery > 0.0 {
        SoccerAction::Aerial
    } else if player.windup_timer > 0.0 || player.windup_shot.is_some() {
        SoccerAction::Windup
    } else if player.jockey_timer > 0.0 {
        SoccerAction::Jockey
    } else if player.receive_timer > 0.0 {
        SoccerAction::Receive
    } else {
        SoccerAction::None
    }
}

fn soccer_committed_of(player: &crate::match_snapshot::MatchPlayer) -> bool {
    player.slide_timer > 0.0
        || player.tackle_timer > 0.0
        || player.jockey_timer > 0.0
        || player.dodge_timer > 0.0
        || player.aerial_timer > 0.0
        || player.aerial_recovery > 0.0
        || player.windup_timer > 0.0
        || player.windup_shot.is_some()
}

fn elapsed_action_ticks(
    family: &gc_data::action_families::ActionFamilyData,
    phase: CombatActionPhase,
    phase_ticks: i64,
) -> i64 {
    match phase {
        CombatActionPhase::Windup => family.windup_ticks - phase_ticks,
        CombatActionPhase::Active => {
            family.windup_ticks + family.active_ticks.unwrap_or(1) - phase_ticks
        }
        CombatActionPhase::Aim | CombatActionPhase::Guard => family.windup_ticks,
        CombatActionPhase::Recovery => {
            family.windup_ticks
                + family.active_ticks.unwrap_or(0)
                + (family.recovery_ticks - phase_ticks)
        }
        CombatActionPhase::Ready => 0,
    }
}

fn threat_path_ticks(
    family: &gc_data::action_families::ActionFamilyData,
    phase: CombatActionPhase,
    phase_ticks: i64,
    release_latched: bool,
) -> i64 {
    use gc_data::action_families::ActionContactKind;
    if family.contact_kind == ActionContactKind::Guard {
        return 0;
    }
    if family.id == ActionFamilyId::Ranged {
        return match phase {
            CombatActionPhase::Windup => {
                if release_latched {
                    phase_ticks + 1
                } else {
                    0
                }
            }
            CombatActionPhase::Active => phase_ticks,
            _ => 0,
        };
    }
    match phase {
        CombatActionPhase::Windup => phase_ticks + family.active_ticks.unwrap_or(0),
        CombatActionPhase::Active => phase_ticks,
        _ => 0,
    }
}

fn convert_team(team: crate::match_snapshot::Team) -> Team {
    match team {
        crate::match_snapshot::Team::Home => Team::Home,
        crate::match_snapshot::Team::Away => Team::Away,
    }
}

fn peer_row(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    player_index: usize,
) -> PeerRow {
    let player = &state.players[player_index - 1];
    let runtime = combat_state.and_then(|c| c.players.get(player_index - 1));
    let family_id = runtime.and_then(|r| r.family_id);
    let family = family_id.map(action_families::get);
    let phase = runtime.map_or(CombatActionPhase::Ready, |r| r.phase);
    let phase_ticks = runtime.map_or(0, |r| r.phase_ticks);
    let release_latched = runtime.is_some_and(|r| r.release_latched);
    let mut telegraph_start = 0;
    let mut telegraph_end = 0;
    let mut spawn_tick = 0;
    if let (Some(family), true) = (family, phase != CombatActionPhase::Ready) {
        let tick = combat_state
            .expect("phase != ready implies a combat companion")
            .tick;
        telegraph_start = tick - elapsed_action_ticks(family, phase, phase_ticks);
        let remaining = threat_path_ticks(family, phase, phase_ticks, release_latched);
        telegraph_end = if remaining > 0 { tick + remaining } else { 0 };
        if family_id == Some(ActionFamilyId::Ranged) && release_latched {
            spawn_tick = if phase == CombatActionPhase::Active {
                tick
            } else {
                tick + phase_ticks
            };
        }
    }
    PeerRow {
        player_index: player_index as i64,
        player_id: player.id.clone(),
        slot: state.slot_for_player[player_index - 1].unwrap_or(0),
        team: convert_team(player.team),
        is_keeper: player.is_keeper,
        soccer_action: soccer_action_of(player),
        soccer_committed: soccer_committed_of(player),
        family_id,
        source_sequence: runtime.and_then(|r| r.source_sequence).unwrap_or(0),
        phase,
        phase_ticks,
        forced_state: runtime.and_then(|r| r.forced_state),
        forced_ticks: runtime.map_or(0, |r| r.forced_ticks),
        immunity_ticks: runtime.map_or(0, |r| r.immunity_ticks),
        guard_active: phase == CombatActionPhase::Guard,
        telegraph_start_tick: telegraph_start,
        telegraph_end_tick: telegraph_end,
        projected_reach_px: family.and_then(|f| f.reach_px).unwrap_or(0.0),
        projected_arc_degrees: family.map_or(0.0, |f| f.front_arc_degrees),
        release_latched: family_id == Some(ActionFamilyId::Ranged) && release_latched,
        projected_spawn_tick: spawn_tick,
        x: player.pos.x,
        y: player.pos.y,
        vx: player.vel.x,
        vy: player.vel.y,
        facing_x: player.facing.x,
        facing_y: player.facing.y,
        radius: player.radius,
    }
}

fn field_exit_ticks(
    state: &MatchState,
    projectile: &crate::combat_snapshot::CombatProjectile,
) -> i64 {
    let ranged = action_families::get(ActionFamilyId::Ranged);
    let step = ranged
        .projectile_speed_px_per_second
        .expect("ranged family defines a projectile speed")
        * fixed_clock::TICK_SECONDS;
    let lifetime = ranged
        .projectile_lifetime_ticks
        .expect("ranged family defines a projectile lifetime");
    let mut x = projectile.pos.x;
    let mut y = projectile.pos.y;
    for tick in 1..=lifetime {
        x += projectile.dir.x * step;
        y += projectile.dir.y * step;
        if x < 0.0 || x > state.field.w || y < 0.0 || y > state.field.h {
            return tick;
        }
    }
    lifetime
}

fn projectile_before(left: &ProjectileRow, right: &ProjectileRow) -> std::cmp::Ordering {
    left.source_sequence
        .cmp(&right.source_sequence)
        .then_with(|| left.source_player_id.cmp(&right.source_player_id))
        .then_with(|| left.projectile_id.cmp(&right.projectile_id))
}

fn formation_slot_id(state: &MatchState, player_index: usize) -> String {
    let player = &state.players[player_index - 1];
    let formation_id = state.formation.get(player.team);
    if player.is_keeper {
        return format!("{formation_id}/keeper");
    }
    let mut ordinal = 0;
    for (index, other) in state.players.iter().enumerate() {
        if other.team == player.team && !other.is_keeper {
            ordinal += 1;
            if index + 1 == player_index {
                break;
            }
        }
    }
    format!("{formation_id}/outfield/{ordinal}")
}

/// Own materialized input through the observed tick, for [`build`]'s
/// `last_input` parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LastInput {
    /// Equipment control physically held.
    pub equipment_held: bool,
    /// Equipment press edge.
    pub equipment_pressed: bool,
    /// Equipment release edge.
    pub equipment_released: bool,
}

/// Build one player's observation of the authoritative public boundary
/// state.
///
/// # Panics
///
/// Panics if `policy_id` is empty or `player_index` does not name a
/// fixture player.
#[must_use]
pub fn build(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    player_index: i64,
    policy_id: &str,
    last_input: Option<LastInput>,
) -> CombatObservation {
    assert!(!policy_id.is_empty(), "observations need a policy id");
    let player = state
        .players
        .get(player_index as usize - 1)
        .expect("unknown observation player");
    let runtime = combat_state.and_then(|c| c.players.get(player_index as usize - 1));
    let tick = combat_state.map_or(state.input_tick, |c| c.tick);

    let mut player_order = Vec::with_capacity(state.players.len());
    let mut teammates = Vec::new();
    let mut opponents = Vec::new();
    let mut anchors = Vec::with_capacity(state.players.len());
    for (zero_index, other) in state.players.iter().enumerate() {
        let index = zero_index + 1;
        player_order.push(index as i64);
        anchors.push(AnchorRow {
            player_index: index as i64,
            formation_slot_id: formation_slot_id(state, index),
            anchor_x: other.anchor.x,
            anchor_y: other.anchor.y,
        });
        if index != player_index as usize {
            let row = peer_row(state, combat_state, index);
            if other.team == player.team {
                teammates.push(row);
            } else {
                opponents.push(row);
            }
        }
    }

    let mut projectiles = Vec::new();
    if let Some(combat_state) = combat_state {
        let step = action_families::get(ActionFamilyId::Ranged)
            .projectile_speed_px_per_second
            .expect("ranged family defines a projectile speed");
        for projectile in &combat_state.projectiles {
            let source = &state.players[projectile.source_index as usize - 1];
            projectiles.push(ProjectileRow {
                projectile_id: projectile_id(&source.id, projectile.source_sequence),
                source_player_index: projectile.source_index,
                source_player_id: source.id.clone(),
                source_team: convert_team(source.team),
                source_sequence: projectile.source_sequence,
                family_id: projectile.family_id,
                x: projectile.pos.x,
                y: projectile.pos.y,
                dir_x: projectile.dir.x,
                dir_y: projectile.dir.y,
                vx: projectile.dir.x * step,
                vy: projectile.dir.y * step,
                in_flight: true,
                remaining_lifetime_ticks: projectile.remaining_ticks,
                horizon_ticks: projectile
                    .remaining_ticks
                    .min(field_exit_ticks(state, projectile)),
            });
        }
        projectiles.sort_by(projectile_before);
    }
    let projectile_order: Vec<String> = projectiles
        .iter()
        .map(|p| p.projectile_id.clone())
        .collect();

    let owner = state.owner.map(|o| &state.players[o as usize - 1]);
    let family_id = runtime.and_then(|r| r.family_id);
    let own = SelfRow {
        player_index,
        player_id: player.id.clone(),
        slot: state.slot_for_player[player_index as usize - 1].unwrap_or(0),
        team: convert_team(player.team),
        is_keeper: player.is_keeper,
        family_id,
        source_sequence: runtime.and_then(|r| r.source_sequence).unwrap_or(0),
        soccer_action: soccer_action_of(player),
        soccer_committed: soccer_committed_of(player),
        phase: runtime.map_or(CombatActionPhase::Ready, |r| r.phase),
        phase_ticks: runtime.map_or(0, |r| r.phase_ticks),
        forced_state: runtime.and_then(|r| r.forced_state),
        forced_ticks: runtime.map_or(0, |r| r.forced_ticks),
        immunity_ticks: runtime.map_or(0, |r| r.immunity_ticks),
        cooldown_ticks: runtime.map_or(0, |r| r.cooldown_ticks),
        ready: runtime.is_some_and(|r| {
            r.family_id.is_some()
                && r.phase == CombatActionPhase::Ready
                && r.cooldown_ticks == 0
                && r.forced_ticks == 0
        }),
        control_held: runtime.is_some_and(|r| r.control_held),
        release_latched: family_id == Some(ActionFamilyId::Ranged)
            && runtime.is_some_and(|r| r.release_latched),
        projected_spawn_tick: 0,
        x: player.pos.x,
        y: player.pos.y,
        vx: player.vel.x,
        vy: player.vel.y,
        facing_x: player.facing.x,
        facing_y: player.facing.y,
        radius: player.radius,
        move_speed: player.move_speed,
        last_equipment_held: last_input.is_some_and(|i| i.equipment_held),
        last_equipment_pressed: last_input.is_some_and(|i| i.equipment_pressed),
        last_equipment_released: last_input.is_some_and(|i| i.equipment_released),
    };

    let mut observation = CombatObservation {
        schema: SCHEMA.to_string(),
        version: VERSION,
        policy_id: policy_id.to_string(),
        producer_tick: tick,
        observed_tick: tick,
        family_catalog_version: family_catalog_version(),
        collision_catalog_version: COLLISION_CATALOG_VERSION.to_string(),
        player_order,
        projectile_order,
        own: own.clone(),
        teammates,
        opponents,
        projectiles,
        ball: BallRow {
            x: state.ball.x,
            y: state.ball.y,
            z: state.ball_z,
            vx: state.ball_vel.x,
            vy: state.ball_vel.y,
            vz: state.ball_vz,
            loose: state.owner.is_none(),
            owner_player_index: state.owner.unwrap_or(0),
            owner_team: owner.map(|o| convert_team(o.team)),
            owner_is_keeper: owner.is_some_and(|o| o.is_keeper),
        },
        match_row: MatchRow {
            tick,
            tick_rate: fixed_clock::TICK_RATE,
            field_w: state.field.w,
            field_h: state.field.h,
            goal_home_x: state.goal_home.x,
            goal_home_y: state.goal_home.y,
            goal_home_w: state.goal_home.w,
            goal_home_h: state.goal_home.h,
            goal_away_x: state.goal_away.x,
            goal_away_y: state.goal_away.y,
            goal_away_w: state.goal_away.w,
            goal_away_h: state.goal_away.h,
            score_home: state.score.home,
            score_away: state.score.away,
            remaining_ticks: (state.time_left * fixed_clock::TICK_RATE + 0.5).floor() as i64,
            stoppage: state.kickoff_hold > 0.0,
            stoppage_ticks: (state.kickoff_hold * fixed_clock::TICK_RATE + 0.5).floor() as i64,
            finished: state.finished,
        },
        anchors,
        digest: String::new(),
    };
    if family_id == Some(ActionFamilyId::Ranged) && observation.own.release_latched {
        let phase = observation.own.phase;
        observation.own.projected_spawn_tick = if phase == CombatActionPhase::Active {
            tick
        } else {
            tick + observation.own.phase_ticks
        };
    }
    observation.digest = digest(&observation);
    observation
}

fn append_self_row(parts: &mut String, row: &SelfRow) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            append_str(parts, $name);
            $body;
        };
    }
    f!("player_index", append_i(parts, row.player_index));
    f!("player_id", append_str(parts, &row.player_id));
    f!("slot", append_i(parts, row.slot));
    f!("team", append_str(parts, team_wire(row.team)));
    f!("is_keeper", append_bool(parts, row.is_keeper));
    f!("family_id", append_str(parts, family_wire(row.family_id)));
    f!("source_sequence", append_i(parts, row.source_sequence));
    f!(
        "soccer_action",
        append_str(parts, row.soccer_action.wire_str())
    );
    f!("soccer_committed", append_bool(parts, row.soccer_committed));
    f!("phase", append_str(parts, phase_wire(row.phase)));
    f!("phase_ticks", append_i(parts, row.phase_ticks));
    f!(
        "forced_state",
        append_str(parts, forced_state_wire(row.forced_state))
    );
    f!("forced_ticks", append_i(parts, row.forced_ticks));
    f!("immunity_ticks", append_i(parts, row.immunity_ticks));
    f!("cooldown_ticks", append_i(parts, row.cooldown_ticks));
    f!("ready", append_bool(parts, row.ready));
    f!("control_held", append_bool(parts, row.control_held));
    f!("release_latched", append_bool(parts, row.release_latched));
    f!(
        "projected_spawn_tick",
        append_i(parts, row.projected_spawn_tick)
    );
    f!("x", append_num(parts, row.x));
    f!("y", append_num(parts, row.y));
    f!("vx", append_num(parts, row.vx));
    f!("vy", append_num(parts, row.vy));
    f!("facing_x", append_num(parts, row.facing_x));
    f!("facing_y", append_num(parts, row.facing_y));
    f!("radius", append_num(parts, row.radius));
    f!("move_speed", append_num(parts, row.move_speed));
    f!(
        "last_equipment_held",
        append_bool(parts, row.last_equipment_held)
    );
    f!(
        "last_equipment_pressed",
        append_bool(parts, row.last_equipment_pressed)
    );
    f!(
        "last_equipment_released",
        append_bool(parts, row.last_equipment_released)
    );
}

fn append_peer_row(parts: &mut String, row: &PeerRow) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            append_str(parts, $name);
            $body;
        };
    }
    f!("player_index", append_i(parts, row.player_index));
    f!("player_id", append_str(parts, &row.player_id));
    f!("slot", append_i(parts, row.slot));
    f!("team", append_str(parts, team_wire(row.team)));
    f!("is_keeper", append_bool(parts, row.is_keeper));
    f!(
        "soccer_action",
        append_str(parts, row.soccer_action.wire_str())
    );
    f!("soccer_committed", append_bool(parts, row.soccer_committed));
    f!("family_id", append_str(parts, family_wire(row.family_id)));
    f!("source_sequence", append_i(parts, row.source_sequence));
    f!("phase", append_str(parts, phase_wire(row.phase)));
    f!("phase_ticks", append_i(parts, row.phase_ticks));
    f!(
        "forced_state",
        append_str(parts, forced_state_wire(row.forced_state))
    );
    f!("forced_ticks", append_i(parts, row.forced_ticks));
    f!("immunity_ticks", append_i(parts, row.immunity_ticks));
    f!("guard_active", append_bool(parts, row.guard_active));
    f!(
        "telegraph_start_tick",
        append_i(parts, row.telegraph_start_tick)
    );
    f!(
        "telegraph_end_tick",
        append_i(parts, row.telegraph_end_tick)
    );
    f!(
        "projected_reach_px",
        append_num(parts, row.projected_reach_px)
    );
    f!(
        "projected_arc_degrees",
        append_num(parts, row.projected_arc_degrees)
    );
    f!("release_latched", append_bool(parts, row.release_latched));
    f!(
        "projected_spawn_tick",
        append_i(parts, row.projected_spawn_tick)
    );
    f!("x", append_num(parts, row.x));
    f!("y", append_num(parts, row.y));
    f!("vx", append_num(parts, row.vx));
    f!("vy", append_num(parts, row.vy));
    f!("facing_x", append_num(parts, row.facing_x));
    f!("facing_y", append_num(parts, row.facing_y));
    f!("radius", append_num(parts, row.radius));
}

fn append_projectile_row(parts: &mut String, row: &ProjectileRow) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            append_str(parts, $name);
            $body;
        };
    }
    f!("projectile_id", append_str(parts, &row.projectile_id));
    f!(
        "source_player_index",
        append_i(parts, row.source_player_index)
    );
    f!("source_player_id", append_str(parts, &row.source_player_id));
    f!("source_team", append_str(parts, team_wire(row.source_team)));
    f!("source_sequence", append_i(parts, row.source_sequence));
    f!(
        "family_id",
        append_str(parts, action_family_wire_id(row.family_id))
    );
    f!("x", append_num(parts, row.x));
    f!("y", append_num(parts, row.y));
    f!("dir_x", append_num(parts, row.dir_x));
    f!("dir_y", append_num(parts, row.dir_y));
    f!("vx", append_num(parts, row.vx));
    f!("vy", append_num(parts, row.vy));
    f!("in_flight", append_bool(parts, row.in_flight));
    f!(
        "remaining_lifetime_ticks",
        append_i(parts, row.remaining_lifetime_ticks)
    );
    f!("horizon_ticks", append_i(parts, row.horizon_ticks));
}

fn append_ball_row(parts: &mut String, row: BallRow) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            append_str(parts, $name);
            $body;
        };
    }
    f!("x", append_num(parts, row.x));
    f!("y", append_num(parts, row.y));
    f!("z", append_num(parts, row.z));
    f!("vx", append_num(parts, row.vx));
    f!("vy", append_num(parts, row.vy));
    f!("vz", append_num(parts, row.vz));
    f!("loose", append_bool(parts, row.loose));
    f!(
        "owner_player_index",
        append_i(parts, row.owner_player_index)
    );
    f!(
        "owner_team",
        append_str(parts, row.owner_team.map_or("none", team_wire))
    );
    f!("owner_is_keeper", append_bool(parts, row.owner_is_keeper));
}

fn append_match_row(parts: &mut String, row: MatchRow) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            append_str(parts, $name);
            $body;
        };
    }
    f!("tick", append_i(parts, row.tick));
    f!("tick_rate", append_num(parts, row.tick_rate));
    f!("field_w", append_num(parts, row.field_w));
    f!("field_h", append_num(parts, row.field_h));
    f!("goal_home_x", append_num(parts, row.goal_home_x));
    f!("goal_home_y", append_num(parts, row.goal_home_y));
    f!("goal_home_w", append_num(parts, row.goal_home_w));
    f!("goal_home_h", append_num(parts, row.goal_home_h));
    f!("goal_away_x", append_num(parts, row.goal_away_x));
    f!("goal_away_y", append_num(parts, row.goal_away_y));
    f!("goal_away_w", append_num(parts, row.goal_away_w));
    f!("goal_away_h", append_num(parts, row.goal_away_h));
    f!("score_home", append_i(parts, row.score_home));
    f!("score_away", append_i(parts, row.score_away));
    f!("remaining_ticks", append_i(parts, row.remaining_ticks));
    f!("stoppage", append_bool(parts, row.stoppage));
    f!("stoppage_ticks", append_i(parts, row.stoppage_ticks));
    f!("finished", append_bool(parts, row.finished));
}

fn append_anchor_row(parts: &mut String, row: &AnchorRow) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            append_str(parts, $name);
            $body;
        };
    }
    f!("player_index", append_i(parts, row.player_index));
    f!(
        "formation_slot_id",
        append_str(parts, &row.formation_slot_id)
    );
    f!("anchor_x", append_num(parts, row.anchor_x));
    f!("anchor_y", append_num(parts, row.anchor_y));
}

/// Canonical byte-exact encoding. Two observations encode identically
/// exactly when they carry the same public information. `digest` is
/// deliberately absent from the encoding: it is a digest OF this body, so
/// it cannot also be inside it.
#[must_use]
pub fn encode(observation: &CombatObservation) -> String {
    let mut parts = format!("GCCO;{};", observation.version);
    append_str(&mut parts, "schema");
    append_str(&mut parts, &observation.schema);
    append_str(&mut parts, "version");
    append_i(&mut parts, observation.version);
    append_str(&mut parts, "policy_id");
    append_str(&mut parts, &observation.policy_id);
    append_str(&mut parts, "producer_tick");
    append_i(&mut parts, observation.producer_tick);
    append_str(&mut parts, "observed_tick");
    append_i(&mut parts, observation.observed_tick);
    append_str(&mut parts, "family_catalog_version");
    append_str(&mut parts, &observation.family_catalog_version);
    append_str(&mut parts, "collision_catalog_version");
    append_str(&mut parts, &observation.collision_catalog_version);
    append_str(&mut parts, "player_order");
    append_i(&mut parts, observation.player_order.len() as i64);
    for value in &observation.player_order {
        append_i(&mut parts, *value);
    }
    append_str(&mut parts, "projectile_order");
    append_i(&mut parts, observation.projectile_order.len() as i64);
    for value in &observation.projectile_order {
        append_str(&mut parts, value);
    }
    append_str(&mut parts, "self");
    append_self_row(&mut parts, &observation.own);
    append_str(&mut parts, "teammates");
    append_i(&mut parts, observation.teammates.len() as i64);
    for row in &observation.teammates {
        append_peer_row(&mut parts, row);
    }
    append_str(&mut parts, "opponents");
    append_i(&mut parts, observation.opponents.len() as i64);
    for row in &observation.opponents {
        append_peer_row(&mut parts, row);
    }
    append_str(&mut parts, "projectiles");
    append_i(&mut parts, observation.projectiles.len() as i64);
    for row in &observation.projectiles {
        append_projectile_row(&mut parts, row);
    }
    append_str(&mut parts, "ball");
    append_ball_row(&mut parts, observation.ball);
    append_str(&mut parts, "match");
    append_match_row(&mut parts, observation.match_row);
    append_str(&mut parts, "anchors");
    append_i(&mut parts, observation.anchors.len() as i64);
    for row in &observation.anchors {
        append_anchor_row(&mut parts, row);
    }
    parts
}

/// FNV-1a-64 digest of [`encode`]'s output.
#[must_use]
pub fn digest(observation: &CombatObservation) -> String {
    fnv1a64::hash(encode(observation).as_bytes())
}

/// Strict validation: every declared field is present, every enum member is
/// known, rows are in canonical order, and the digest covers the body.
///
/// Undeclared field, missing field, wrong scalar kind, and unknown
/// enum-member checks are unnecessary here — a typed [`CombatObservation`]
/// cannot have an undeclared or missing field, and every enum-shaped field
/// is a closed Rust enum. What remains, and is checked below, are the
/// invariants a type cannot express: schema/version/catalog tags, row
/// ordering, uniqueness, the teammate/opponent split matching `team`, and
/// the digest matching the body.
///
/// # Errors
///
/// Returns `Err` with a human-readable reason on the first violation found.
pub fn validate(observation: &CombatObservation) -> Result<(), String> {
    if observation.schema != SCHEMA {
        return Err(format!("observation schema tag is not {SCHEMA}"));
    }
    if observation.version != VERSION {
        return Err("unsupported combat observation version".to_string());
    }
    if observation.policy_id.is_empty() {
        return Err("observation needs a policy id".to_string());
    }
    if observation.family_catalog_version != family_catalog_version()
        || observation.collision_catalog_version != COLLISION_CATALOG_VERSION
    {
        return Err("observation catalog versions do not match this build".to_string());
    }

    let mut seen_index = std::collections::BTreeSet::new();
    for (key, rows) in [
        ("teammates", &observation.teammates),
        ("opponents", &observation.opponents),
    ] {
        let same_team = key == "teammates";
        let mut previous = 0i64;
        for (index, row) in rows.iter().enumerate() {
            let path = format!("observation.{key}.{}", index + 1);
            if (row.team == observation.own.team) != same_team {
                return Err(format!("{path} is in the wrong team array for its team"));
            }
            if row.player_index < 1 {
                return Err(format!("{path} needs a canonical player index"));
            }
            if row.player_index <= previous {
                return Err(format!(
                    "observation.{key} rows are not in canonical player order"
                ));
            }
            previous = row.player_index;
            if !seen_index.insert(row.player_index) {
                return Err("observation has a duplicate player row".to_string());
            }
        }
    }
    if seen_index.contains(&observation.own.player_index) {
        return Err("observation repeats the observing player as a peer".to_string());
    }

    let mut seen_projectile = std::collections::BTreeSet::new();
    let mut previous_projectile: Option<&ProjectileRow> = None;
    for (index, row) in observation.projectiles.iter().enumerate() {
        if !seen_projectile.insert(row.projectile_id.clone()) {
            return Err("observation has a duplicate projectile row".to_string());
        }
        if let Some(previous) = previous_projectile
            && projectile_before(previous, row) != std::cmp::Ordering::Less
        {
            return Err("observation.projectiles rows are not in canonical order".to_string());
        }
        previous_projectile = Some(row);
        let _ = index;
    }

    let mut previous_anchor = 0i64;
    for row in &observation.anchors {
        if row.player_index <= previous_anchor {
            return Err("observation.anchors rows are not in canonical player order".to_string());
        }
        previous_anchor = row.player_index;
    }

    for (index, value) in observation.player_order.iter().enumerate() {
        if *value != index as i64 + 1 {
            return Err(
                "observation.player_order is not the canonical player index order".to_string(),
            );
        }
    }
    if observation.player_order.len() != observation.anchors.len() {
        return Err("observation.player_order does not cover every anchor row".to_string());
    }
    if observation.projectile_order.len() != observation.projectiles.len() {
        return Err("observation.projectile_order does not match the projectile rows".to_string());
    }
    for (index, id) in observation.projectile_order.iter().enumerate() {
        if *id != observation.projectiles[index].projectile_id {
            return Err(
                "observation.projectile_order does not follow the projectile rows".to_string(),
            );
        }
    }

    if observation.digest != digest(observation) {
        return Err("observation digest does not cover its body".to_string());
    }
    Ok(())
}

/// Project this full schema down to [`crate::combat_feasibility`]'s
/// narrower local `CombatObservation` view. See the module doc's bridging
/// section.
#[must_use]
pub fn to_feasibility_view(
    observation: &CombatObservation,
) -> combat_feasibility::CombatObservation {
    combat_feasibility::CombatObservation {
        own: combat_feasibility::CombatObservationSelf {
            player_index: observation.own.player_index,
            team: observation.own.team,
            x: observation.own.x,
            y: observation.own.y,
            vx: observation.own.vx,
            vy: observation.own.vy,
            facing_x: observation.own.facing_x,
            facing_y: observation.own.facing_y,
            radius: observation.own.radius,
            move_speed: observation.own.move_speed,
            phase: observation.own.phase,
            family_id: observation.own.family_id,
            forced_ticks: observation.own.forced_ticks,
        },
        teammates: observation
            .teammates
            .iter()
            .map(to_feasibility_peer)
            .collect(),
        opponents: observation
            .opponents
            .iter()
            .map(to_feasibility_peer)
            .collect(),
        projectiles: observation
            .projectiles
            .iter()
            .map(|p| combat_feasibility::CombatObservationProjectile {
                x: p.x,
                y: p.y,
                dir_x: p.dir_x,
                dir_y: p.dir_y,
                source_team: p.source_team,
                source_player_index: p.source_player_index,
                horizon_ticks: p.horizon_ticks,
            })
            .collect(),
        ball: combat_feasibility::CombatObservationBall {
            x: observation.ball.x,
            y: observation.ball.y,
            owner_player_index: observation.ball.owner_player_index,
            owner_team: observation.ball.owner_team,
        },
        match_view: combat_feasibility::CombatObservationMatchView {
            field_w: observation.match_row.field_w,
            field_h: observation.match_row.field_h,
            goal_home_x: observation.match_row.goal_home_x,
            goal_home_y: observation.match_row.goal_home_y,
            goal_home_h: observation.match_row.goal_home_h,
            goal_away_x: observation.match_row.goal_away_x,
            goal_away_y: observation.match_row.goal_away_y,
            goal_away_w: observation.match_row.goal_away_w,
            goal_away_h: observation.match_row.goal_away_h,
        },
        anchors: observation
            .anchors
            .iter()
            .map(|a| combat_feasibility::CombatObservationAnchor {
                player_index: a.player_index,
                anchor_x: a.anchor_x,
                anchor_y: a.anchor_y,
            })
            .collect(),
        observed_tick: observation.observed_tick,
    }
}

fn to_feasibility_peer(row: &PeerRow) -> combat_feasibility::CombatObservationPeer {
    combat_feasibility::CombatObservationPeer {
        player_index: row.player_index,
        team: row.team,
        is_keeper: row.is_keeper,
        x: row.x,
        y: row.y,
        vx: row.vx,
        vy: row.vy,
        facing_x: row.facing_x,
        facing_y: row.facing_y,
        radius: row.radius,
        phase: row.phase,
        phase_ticks: row.phase_ticks,
        family_id: row.family_id,
        forced_ticks: row.forced_ticks,
        release_latched: row.release_latched,
        projected_spawn_tick: row.projected_spawn_tick,
        projected_reach_px: row.projected_reach_px,
    }
}
