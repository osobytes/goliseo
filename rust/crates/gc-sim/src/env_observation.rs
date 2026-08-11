//! Port of `sim/env_observation.lua`.
//!
//! Versioned learning-environment observations.
//!
//! Three explicitly tagged profiles:
//!
//!   - **representative** — one controlled slot, restricted to cues the
//!     presented match shows that slot's player at the current boundary tick.
//!   - **team** — several controlled slots, each still restricted to its own
//!     player-observable cues, with explicit slot ownership.
//!   - **privileged** — the representative view plus the authoritative
//!     canonical snapshot and RNG state. `player_observable` and
//!     `human_proxy_valid` are false: policies trained on this profile may
//!     never be reported as human proxies.
//!
//! A view is built from the boundary state BEFORE any action for the next
//! tick is accepted, so it structurally cannot contain a future tick, another
//! slot's same-tick choice, or a resolver verdict that has not happened yet.
//! Confirmed events describe the tick that already completed.
//!
//! Everything here is a fresh plain-data copy: an observation holds no
//! reference to `MatchState`, so a policy cannot reach through it into
//! simulation authority.
//!
//! Field-by-field source, unit, visibility, and history rules live in
//! `docs/design/learning_environment.md`.
//!
//! ## Port notes
//!
//! `env_observation.lua` never calls `require("sim.env_config")` — it uses
//! the `EnvObservationProfile` LuaCATS alias directly, needing no `require`.
//! `env_config.rs` already established the pattern this port follows: each
//! module that touches the alias declares its own local copy rather than
//! importing one from another module's port, so the per-module dependency
//! footprint matches the Lua original's `require` list exactly (see
//! `env_config.rs`'s module doc for the same note). This module's
//! `EnvObservationProfile` is therefore its own type, distinct from (but
//! naming-compatible with) `env_config::EnvObservationProfile`.
//!
//! The Lua `env_observation.encode(observation)` accepts either an
//! `EnvObservation` or a bare `EnvSlotView` (a LuaCATS union). Rust has no
//! free-function overloading, so this port exposes two entry points,
//! [`encode`] and [`encode_view`], sharing the same canonical-encoding
//! machinery.
//!
//! `EnvPrivilegedView.snapshot` is canonically encoded as
//! `match_snapshot::hash(&self.snapshot)` rather than a full field-by-field
//! walk of `MatchSnapshot`'s own ~50 nested fields. The Lua original's
//! generic `encode_value` really does recurse into the raw snapshot table,
//! but reproducing that here would mean re-deriving `match_snapshot.rs`'s
//! entire canonical-encoding surface a second time, in a different
//! generic-value shape, as a module this port does not own and must not
//! edit — a duplication liable to drift the moment that module's shape
//! changes. Using its own `hash` instead is still deterministic, still
//! reacts to every field the real encoding would (by that function's own
//! correctness contract), and is the same mechanism the rest of the
//! codebase already uses to answer "did this state change" (e.g.
//! `boundary_hash`). No test this module owns exercises the difference:
//! `spec/sim/env_leakage_spec.lua`, the spec that stresses privileged-profile
//! encoding specifically, needs `sim::env`/`sim::match` and is out of scope
//! here (see `tests/env_leakage.rs`). Flagged in the porting report.

use crate::combat_feasibility::CombatActionPhase;
use crate::combat_observation;
use crate::combat_snapshot::{self, CombatContactResult, CombatForcedState, CombatMatchState};
use crate::fixed_clock;
use crate::input_frame;
use crate::match_snapshot::{self, MatchSnapshot, MatchState, Rect};
use gc_data::action_families::ActionFamilyId;

/// Observation schema version.
pub const VERSION: i64 = 1;

// ---------------------------------------------------------------------------
// Small closed vocabularies.
// ---------------------------------------------------------------------------

/// A player or event actor's side relative to the observation's viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvRelativeSide {
    /// The viewer's own side.
    Own,
    /// The opposing side.
    Opponent,
}

/// The match's coarse phase, as far as an observation reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvMatchPhase {
    /// The post-restart hold before ordinary play resumes.
    Kickoff,
    /// Ordinary open play.
    Live,
    /// The match has ended.
    Finished,
}

/// Which observation lens a view is built from. See the module doc's three
/// profiles. Duplicated per-module by design; see the module doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvObservationProfile {
    /// One controlled slot restricted to presented, current cues.
    Representative,
    /// Several controlled slots, each still player-observable.
    Team,
    /// Authoritative oracle/debug profile; never a human proxy.
    Privileged,
}

fn profile_wire_str(profile: EnvObservationProfile) -> &'static str {
    match profile {
        EnvObservationProfile::Representative => "representative",
        EnvObservationProfile::Team => "team",
        EnvObservationProfile::Privileged => "privileged",
    }
}

/// One profile's declared observability/proxy-validity tags and description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvObservationProfileData {
    /// The profile this data describes.
    pub id: EnvObservationProfile,
    /// True only when every field has a presented analogue.
    pub player_observable: bool,
    /// False for oracle/debug/adversarial profiles.
    pub human_proxy_valid: bool,
    /// Whether this profile covers more than one controlled slot.
    pub multi_slot: bool,
    /// Human-readable summary.
    pub description: &'static str,
}

/// Every observation profile's declared data, in the order
/// [`EnvObservationProfile`] declares.
pub static PROFILES: [EnvObservationProfileData; 3] = [
    EnvObservationProfileData {
        id: EnvObservationProfile::Representative,
        player_observable: true,
        human_proxy_valid: true,
        multi_slot: false,
        description: "One controlled slot restricted to presented, current cues.",
    },
    EnvObservationProfileData {
        id: EnvObservationProfile::Team,
        player_observable: true,
        human_proxy_valid: true,
        multi_slot: true,
        description: "Per-slot player-observable views with explicit slot ownership.",
    },
    EnvObservationProfileData {
        id: EnvObservationProfile::Privileged,
        player_observable: false,
        human_proxy_valid: false,
        multi_slot: true,
        description: "Authoritative oracle/debug profile; never a human proxy.",
    },
];

/// This profile's declared data.
#[must_use]
pub fn profile_data(profile: EnvObservationProfile) -> &'static EnvObservationProfileData {
    PROFILES
        .iter()
        .find(|p| p.id == profile)
        .expect("PROFILES covers every EnvObservationProfile variant")
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// Failure reasons an observation-building operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvObservationErrorCode {
    /// The data violates a structural or range invariant.
    Malformed,
    /// The observation profile requested is not one this build defines.
    ///
    /// Unreachable through this module's own public API: `profile` is typed
    /// as [`EnvObservationProfile`], a closed enum, so an invalid profile
    /// cannot be constructed to begin with. The Lua original validates a
    /// bare profile *string* here (`spec/sim/env_observation_spec.lua`'s
    /// `UNKNOWN_PROFILE = "oracle"` case); this port pushes that
    /// string-to-enum validation to whichever layer parses a wire/config
    /// string (see `env_config::normalize`), and keeps this variant only so
    /// the error shape still matches the LuaCATS `EnvObservationErrorCode`
    /// alias.
    UnknownProfile,
    /// The observation profile and controlled-slot count disagree.
    ProfileMismatch,
}

/// An expected, recoverable env-observation failure (README rule 5.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvObservationError {
    /// Machine-readable failure reason.
    pub code: EnvObservationErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl EnvObservationError {
    fn new(code: EnvObservationErrorCode, message: impl Into<String>) -> Self {
        EnvObservationError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnvObservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EnvObservationError {}

/// Result alias for fallible env-observation operations.
pub type Result<T> = std::result::Result<T, EnvObservationError>;

fn failure<T>(code: EnvObservationErrorCode, message: impl Into<String>) -> Result<T> {
    Err(EnvObservationError::new(code, message))
}

// ---------------------------------------------------------------------------
// Observed shapes.
// ---------------------------------------------------------------------------

/// Presented pitch geometry from one slot's side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvObservedGeometry {
    /// Pitch width in world px.
    pub field_w: f64,
    /// Pitch height in world px.
    pub field_h: f64,
    /// Goal this slot defends, world px.
    pub own_goal: Rect,
    /// Goal this slot attacks, world px.
    pub target_goal: Rect,
}

/// Presented match/score/clock state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvObservedMatch {
    /// Boundary tick this observation describes.
    pub tick: i64,
    /// Fixed simulation tick rate.
    pub tick_rate: i64,
    /// Coarse match phase.
    pub phase: EnvMatchPhase,
    /// True while the post-restart hold suppresses pressing.
    pub stoppage: bool,
    /// Seconds remaining.
    pub time_left: f64,
    /// This slot's side's score.
    pub score_own: i64,
    /// The opposing side's score.
    pub score_opponent: i64,
    /// Goal cap.
    pub max_goals: i64,
    /// Whether the match has finished.
    pub finished: bool,
}

/// Presented ball state, relative to the viewing slot's side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvObservedBall {
    /// World px.
    pub x: f64,
    /// World px.
    pub y: f64,
    /// Height above the pitch, world px.
    pub z: f64,
    /// World px/s.
    pub vx: f64,
    /// World px/s.
    pub vy: f64,
    /// World px/s (vertical).
    pub vz: f64,
    /// Lateral curve applied to the loose ball.
    pub spin: f64,
    /// Whether the ball is off the ground.
    pub airborne: bool,
    /// True when no player owns the ball.
    pub loose: bool,
    /// Canonical input slot of the carrier, when it has one.
    pub owner_slot: Option<i64>,
    /// Carrier's side relative to the viewer.
    pub owner_side: Option<EnvRelativeSide>,
    /// Whether the carrier is a keeper.
    pub owner_is_keeper: bool,
}

/// Equipment telegraph for another player: the drawn arc colour, its alpha
/// phase, and the visible stagger/knockback pose. Remaining phase ticks,
/// cooldown, and loadout identity are withheld — see
/// `sim/env_observation.lua`'s `EnvObservedEquipment` doc for the render
/// citations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvObservedEquipment {
    /// Visible equipment silhouette / arc colour.
    pub family_id: ActionFamilyId,
    /// Visible animation phase; remaining ticks are withheld.
    pub phase: CombatActionPhase,
    /// Visible stagger/knockback pose.
    pub forced_state: Option<CombatForcedState>,
}

/// One's own equipment readout: everything the HUD shows the controlling
/// player.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvObservedSelfEquipment {
    /// Own fixed loadout identity.
    pub loadout_id: String,
    /// Own action family.
    pub family_id: ActionFamilyId,
    /// Current combat action phase.
    pub phase: CombatActionPhase,
    /// Own readiness readout.
    pub phase_ticks: i64,
    /// Ticks before the action can commit again.
    pub cooldown_ticks: i64,
    /// Whether the action is free to commit right now.
    pub ready: bool,
    /// Whether the equipment control is currently held.
    pub control_held: bool,
    /// Current forced (stagger/knockback) state, if any.
    pub forced_state: Option<CombatForcedState>,
    /// Ticks remaining in the current forced state.
    pub forced_ticks: i64,
    /// Ticks remaining in the post-hit immunity window.
    pub immunity_ticks: i64,
}

/// The complete, pinned field set of a non-self player record. Adding a
/// field here is a claim that the renderer draws it for a player the viewer
/// does not control; each field on [`EnvObservedPlayer`] documents its
/// citation. [`PLAYER_FIELD_NAMES`] pins the same set for a canonical-key
/// audit — see `tests/env_observation.rs`.
///
/// Deliberately absent, per an exhaustive read of `game/render/*.lua` and
/// `render/player_pose.lua`, which finds no pose, colour, or icon for them
/// on a non-local player: `charging`, `jockeying`, `tackling`, `dodging`,
/// and `stunned`. The discrete `tackle` `MatchEvent` does have presentation
/// and reaches an observation through the confirmed-event channel; that is a
/// separate, legitimate channel and is not the same as a continuous boolean
/// covering the whole standing-tackle window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvObservedPlayer {
    /// Canonical input slot; `None` for keepers. Public fixture fact.
    pub slot: Option<i64>,
    /// Kit colour.
    pub side: EnvRelativeSide,
    /// Kit and position.
    pub is_keeper: bool,
    /// Drawn position.
    pub x: f64,
    /// Drawn position.
    pub y: f64,
    /// Visible motion; run cadence follows speed.
    pub vx: f64,
    /// Visible motion; run cadence follows speed.
    pub vy: f64,
    /// Unit facing; the sprite is oriented by it.
    pub facing_x: f64,
    /// Unit facing; the sprite is oriented by it.
    pub facing_y: f64,
    /// Drawn size.
    pub radius: f64,
    /// The ball is drawn at the carrier's feet.
    pub has_ball: bool,
    /// Limb cadence from speed; implied by `vx`/`vy`.
    pub sprinting: bool,
    /// Slide-tackle pose.
    pub sliding: bool,
    /// Keeper dive pose.
    pub diving: bool,
    /// Aerial-jump lift.
    pub airborne: bool,
    /// Windup pose.
    pub winding_up: bool,
    /// Telegraph arc; see [`EnvObservedEquipment`].
    pub equipment: Option<EnvObservedEquipment>,
}

/// The pinned field set of [`EnvObservedPlayer`], for a canonical-key audit.
pub const PLAYER_FIELD_NAMES: &[&str] = &[
    "slot",
    "side",
    "is_keeper",
    "x",
    "y",
    "vx",
    "vy",
    "facing_x",
    "facing_y",
    "radius",
    "has_ball",
    "sprinting",
    "sliding",
    "diving",
    "airborne",
    "winding_up",
    "equipment",
];

/// The pinned field set of [`EnvObservedEquipment`], for a canonical-key
/// audit.
pub const EQUIPMENT_TELEGRAPH_FIELD_NAMES: &[&str] = &["family_id", "phase", "forced_state"];

/// One's own player record: everything the controlling player can read off
/// their own HUD and body.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvObservedSelf {
    /// Own stable player identity.
    pub player_id: String,
    /// Own canonical input slot.
    pub slot: i64,
    /// Always [`EnvRelativeSide::Own`].
    pub side: EnvRelativeSide,
    /// Kit and position. Always `false`: keepers never own an input slot.
    pub is_keeper: bool,
    /// Own position.
    pub x: f64,
    /// Own position.
    pub y: f64,
    /// Own velocity.
    pub vx: f64,
    /// Own velocity.
    pub vy: f64,
    /// Own facing.
    pub facing_x: f64,
    /// Own facing.
    pub facing_y: f64,
    /// Own drawn size.
    pub radius: f64,
    /// Own base locomotion speed, world px/s.
    pub move_speed: f64,
    /// Whether this player carries the ball.
    pub has_ball: bool,
    /// Own sprint pose.
    pub sprinting: bool,
    /// Own jockey stance.
    pub jockeying: bool,
    /// Own slide-tackle pose.
    pub sliding: bool,
    /// Own standing-tackle pose.
    pub tackling: bool,
    /// Own dodge/juke pose.
    pub dodging: bool,
    /// Own stun state.
    pub stunned: bool,
    /// Own dive pose.
    pub diving: bool,
    /// Own aerial-jump lift.
    pub airborne: bool,
    /// Own windup pose.
    pub winding_up: bool,
    /// Own shot/pass charge in progress.
    pub charging: bool,
    /// 0..1 own stamina readout.
    pub sprint_meter: f64,
    /// 0..1 own shot charge readout.
    pub charge: f64,
    /// 0..1 own pass-range charge readout.
    pub pass_charge: f64,
    /// Whether a tackle/slide can start right now.
    pub dash_ready: bool,
    /// Whether a dodge/juke can start right now.
    pub dodge_ready: bool,
    /// Whether a tackle can start right now.
    pub tackle_ready: bool,
    /// Whether a header/volley attempt can start right now.
    pub header_ready: bool,
    /// Seconds until a tackle/slide is ready again.
    pub dash_cooldown_s: f64,
    /// Seconds until a dodge/juke is ready again.
    pub dodge_cooldown_s: f64,
    /// Seconds until a tackle is ready again.
    pub tackle_cooldown_s: f64,
    /// Seconds until a header/volley attempt is ready again.
    pub header_cooldown_s: f64,
    /// Seconds knocked off balance.
    pub stun_s: f64,
    /// Seconds of active dodge/juke remaining.
    pub dodge_s: f64,
    /// Seconds of active slide remaining.
    pub slide_s: f64,
    /// Seconds of active standing tackle remaining.
    pub tackle_s: f64,
    /// Seconds of active jockey stance remaining.
    pub jockey_s: f64,
    /// Seconds until a pending shot/punt releases.
    pub windup_s: f64,
    /// Movement/action recovery after an aerial attempt.
    pub aerial_recovery_s: f64,
    /// Presented pass preview target, own carries only.
    pub pass_preview_slot: Option<i64>,
    /// Whether the pass preview target is a keeper.
    pub pass_preview_keeper: bool,
    /// Own equipment readout.
    pub equipment: Option<EnvObservedSelfEquipment>,
}

/// One confirmed match/combat event of the previous tick.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvObservedEvent {
    /// `MatchEventKind` or `CombatEventKind`'s canonical wire string.
    pub kind: String,
    /// Tick the event was confirmed on; always in the past.
    pub tick: i64,
    /// World x position.
    pub x: f64,
    /// World y position.
    pub y: f64,
    /// Acting player's canonical input slot, when identified.
    pub actor_slot: Option<i64>,
    /// Acting player's side, when identified.
    pub actor_side: Option<EnvRelativeSide>,
    /// Soccer strikes only: whether the strike was on target.
    pub on_target: Option<bool>,
    /// Confirmed combat verdict, combat events only.
    pub result: Option<CombatContactResult>,
    /// Action family, combat events only.
    pub family_id: Option<ActionFamilyId>,
}

/// The authoritative block appended to the privileged profile only.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvPrivilegedView {
    /// Authoritative PRNG state.
    pub rng: u32,
    /// Canonical deep copy; never the live state table.
    pub snapshot: MatchSnapshot,
    /// The captured snapshot's canonical hash.
    pub boundary_hash: String,
}

/// The narrow slot projection an action mask needs: own readiness and the
/// ball, and nothing else.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvActionView {
    /// Schema version.
    pub version: i64,
    /// Observation profile this view was built under.
    pub profile: EnvObservationProfile,
    /// Canonical input slot this view belongs to.
    pub slot: i64,
    /// Absolute side of this slot (public fixture fact).
    pub team: input_frame::Team,
    /// Ball state, relative to this slot.
    pub ball: EnvObservedBall,
    /// This slot's own player record.
    pub own: EnvObservedSelf,
}

/// A snapshot a caller has already captured, donated so the privileged
/// profile does not capture and hash the same boundary a second time.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EnvObservationContext {
    /// An already-captured snapshot to reuse, if any.
    pub snapshot: Option<MatchSnapshot>,
    /// An already-computed boundary hash to reuse, if any.
    pub boundary_hash: Option<String>,
    /// Diagnostic: incremented by [`privileged_view`] when it could not
    /// reuse a donated snapshot and captured the boundary itself instead.
    /// Never affects a hash — it exists so "a caller that donates a
    /// snapshot never pays for a second capture" is a real, checkable
    /// invariant rather than only a comment. A well-behaved caller that
    /// always donates a snapshot when one exists (as [`crate::env::step`]
    /// does) should see this stay at zero; `spec/sim/env_budget_spec.lua`'s
    /// "does not re-capture the boundary for the privileged profile" case
    /// measured this indirectly, as an allocation-budget ceiling. This is
    /// the same property recovered as an exact call count instead — see
    /// `crate::env::EnvInstance::snapshot_captures`'s doc, which folds this
    /// counter in after every [`crate::env::step`] call.
    ///
    /// `Cell` because [`privileged_view`] only ever holds a shared
    /// `&EnvObservationContext` — the same reason
    /// `MatchDriver::snapshot_captures` in `gc-netcode` is a `Cell`.
    pub redundant_captures: std::cell::Cell<i64>,
}

/// One controlled slot's complete observation.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvSlotView {
    /// Schema version.
    pub version: i64,
    /// Observation profile this view was built under.
    pub profile: EnvObservationProfile,
    /// Canonical input slot this view belongs to.
    pub slot: i64,
    /// This slot's stable identity.
    pub slot_id: input_frame::SlotId,
    /// Absolute side of this slot (public fixture fact).
    pub team: input_frame::Team,
    /// Presented match/score/clock state. Lua field name `match`; `match` is
    /// a Rust keyword, hence the raw identifier (same convention as
    /// `gc_sim::r#match`).
    pub r#match: EnvObservedMatch,
    /// Presented pitch geometry from this slot's side.
    pub geometry: EnvObservedGeometry,
    /// Ball state, relative to this slot.
    pub ball: EnvObservedBall,
    /// This slot's own player record.
    pub own: EnvObservedSelf,
    /// Teammates, canonical player order.
    pub teammates: Vec<EnvObservedPlayer>,
    /// Opponents, canonical player order.
    pub opponents: Vec<EnvObservedPlayer>,
    /// Confirmed events of the previous tick only.
    pub events: Vec<EnvObservedEvent>,
    /// Privileged profile only.
    pub privileged: Option<EnvPrivilegedView>,
}

/// The complete observation for a profile over one or more controlled slots.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvObservation {
    /// Schema version.
    pub version: i64,
    /// Observation profile this observation was built under.
    pub profile: EnvObservationProfile,
    /// Whether every field has a presented analogue.
    pub player_observable: bool,
    /// Whether a policy trained on this may be reported as a human proxy.
    pub human_proxy_valid: bool,
    /// Boundary tick this observation describes.
    pub tick: i64,
    /// Controlled slots covered, ascending.
    pub slots: Vec<i64>,
    /// Per-slot views, indexed by `slot - 1` (canonical slots are one-based
    /// 1..=[`crate::input_frame::SLOT_COUNT`]). `None` where that slot is
    /// not covered. A `Vec`, not a map (README rule 4/6): the key domain is
    /// exactly the eight canonical slots, so this is a dense array with
    /// holes rather than a banned hash table.
    pub views: Vec<Option<EnvSlotView>>,
}

// ---------------------------------------------------------------------------
// Building.
// ---------------------------------------------------------------------------

fn side_of(team: match_snapshot::Team, view_team: match_snapshot::Team) -> EnvRelativeSide {
    if team == view_team {
        EnvRelativeSide::Own
    } else {
        EnvRelativeSide::Opponent
    }
}

fn to_state_team(team: input_frame::Team) -> match_snapshot::Team {
    match team {
        input_frame::Team::Home => match_snapshot::Team::Home,
        input_frame::Team::Away => match_snapshot::Team::Away,
    }
}

fn phase_of(state: &MatchState) -> EnvMatchPhase {
    if state.finished {
        EnvMatchPhase::Finished
    } else if state.kickoff_hold > 0.0 {
        EnvMatchPhase::Kickoff
    } else {
        EnvMatchPhase::Live
    }
}

/// One shared derivation from presented combat state. `docs/design/
/// learning_environment.md:255-263` warns that this schema and
/// `combat_sim_observation/v1` must not grow two independent readings of the
/// same telegraph, so the projection lives in `combat_observation` and this
/// schema takes the strict presented subset of it: the drawn arc colour, its
/// alpha phase, and the stagger/knockback pose. Remaining phase ticks,
/// cooldown, release latch, and loadout identity stay withheld here because
/// the renderer does not draw them for a player the viewer does not control.
fn equipment_telegraph(
    combat_state: Option<&CombatMatchState>,
    player_index: i64,
) -> Option<EnvObservedEquipment> {
    let (family_id, phase, forced_state) =
        combat_observation::telegraph(combat_state, player_index)?;
    Some(EnvObservedEquipment {
        family_id,
        phase,
        forced_state,
    })
}

fn own_equipment(
    combat_state: Option<&CombatMatchState>,
    player_index: i64,
) -> Option<EnvObservedSelfEquipment> {
    let combat_state = combat_state?;
    let runtime = combat_state.players.get(player_index as usize - 1)?;
    let family_id = runtime.family_id?;
    let loadout_id = runtime.loadout_id.clone()?;
    Some(EnvObservedSelfEquipment {
        loadout_id,
        family_id,
        phase: runtime.phase,
        phase_ticks: runtime.phase_ticks,
        cooldown_ticks: runtime.cooldown_ticks,
        ready: runtime.phase == CombatActionPhase::Ready
            && runtime.cooldown_ticks == 0
            && runtime.forced_state.is_none(),
        control_held: runtime.control_held,
        forced_state: runtime.forced_state,
        forced_ticks: runtime.forced_ticks,
        immunity_ticks: runtime.immunity_ticks,
    })
}

fn observed_player(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    player_index: i64,
    view_team: match_snapshot::Team,
) -> EnvObservedPlayer {
    let player = &state.players[player_index as usize - 1];
    EnvObservedPlayer {
        slot: state.slot_for_player[player_index as usize - 1],
        side: side_of(player.team, view_team),
        is_keeper: player.is_keeper,
        x: player.pos.x,
        y: player.pos.y,
        vx: player.vel.x,
        vy: player.vel.y,
        facing_x: player.facing.x,
        facing_y: player.facing.y,
        radius: player.radius,
        has_ball: state.owner == Some(player_index),
        sprinting: player.sprinting,
        sliding: player.slide_timer > 0.0,
        diving: player.dive_timer > 0.0,
        airborne: player.aerial_jump > 0.0,
        winding_up: player.windup_timer > 0.0,
        equipment: equipment_telegraph(combat_state, player_index),
    }
}

fn observed_self(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    player_index: i64,
    slot: i64,
) -> EnvObservedSelf {
    let player = &state.players[player_index as usize - 1];
    let preview_index = if state.owner == Some(player_index) {
        player.pass_target
    } else {
        None
    };
    let preview = preview_index.map(|idx| &state.players[idx as usize - 1]);
    EnvObservedSelf {
        player_id: player.id.clone(),
        slot,
        side: EnvRelativeSide::Own,
        is_keeper: player.is_keeper,
        x: player.pos.x,
        y: player.pos.y,
        vx: player.vel.x,
        vy: player.vel.y,
        facing_x: player.facing.x,
        facing_y: player.facing.y,
        radius: player.radius,
        move_speed: player.move_speed,
        has_ball: state.owner == Some(player_index),
        sprinting: player.sprinting,
        jockeying: player.jockey_timer > 0.0,
        sliding: player.slide_timer > 0.0,
        tackling: player.tackle_timer > 0.0,
        dodging: player.dodge_timer > 0.0,
        stunned: player.stun_timer > 0.0,
        diving: player.dive_timer > 0.0,
        airborne: player.aerial_jump > 0.0,
        winding_up: player.windup_timer > 0.0,
        charging: player.charge > 0.0 || player.pass_charge > 0.0,
        sprint_meter: player.sprint_meter,
        charge: player.charge,
        pass_charge: player.pass_charge,
        dash_ready: player.dash_cd <= 0.0,
        dodge_ready: player.dodge_cd <= 0.0,
        tackle_ready: player.tackle_cd <= 0.0,
        header_ready: player.header_cd <= 0.0,
        dash_cooldown_s: player.dash_cd,
        dodge_cooldown_s: player.dodge_cd,
        tackle_cooldown_s: player.tackle_cd,
        header_cooldown_s: player.header_cd,
        stun_s: player.stun_timer,
        dodge_s: player.dodge_timer,
        slide_s: player.slide_timer,
        tackle_s: player.tackle_timer,
        jockey_s: player.jockey_timer,
        windup_s: player.windup_timer,
        aerial_recovery_s: player.aerial_recovery,
        pass_preview_slot: preview_index.and_then(|idx| state.slot_for_player[idx as usize - 1]),
        pass_preview_keeper: preview.map(|p| p.is_keeper).unwrap_or(false),
        equipment: own_equipment(combat_state, player_index),
    }
}

fn observed_ball(state: &MatchState, view_team: match_snapshot::Team) -> EnvObservedBall {
    let owner = state.owner.map(|o| &state.players[o as usize - 1]);
    EnvObservedBall {
        x: state.ball.x,
        y: state.ball.y,
        z: state.ball_z,
        vx: state.ball_vel.x,
        vy: state.ball_vel.y,
        vz: state.ball_vz,
        spin: state.ball_spin,
        airborne: state.ball_z > 0.0,
        loose: state.owner.is_none(),
        owner_slot: state
            .owner
            .and_then(|o| state.slot_for_player[o as usize - 1]),
        owner_side: owner.map(|p| side_of(p.team, view_team)),
        owner_is_keeper: owner.map(|p| p.is_keeper).unwrap_or(false),
    }
}

fn slot_and_team_of_id(
    state: &MatchState,
    id: &str,
) -> Option<(Option<i64>, match_snapshot::Team)> {
    state
        .players
        .iter()
        .enumerate()
        .find(|(_, p)| p.id == id)
        .map(|(i, p)| (state.slot_for_player[i], p.team))
}

/// Confirmed events of the tick that already completed. `state.events` is
/// cleared at the top of every match step, so this is never a speculative or
/// rollback-presentation event, and its tick is always strictly in the past.
fn observed_events(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    view_team: match_snapshot::Team,
) -> Vec<EnvObservedEvent> {
    let mut events = Vec::new();
    if state.input_tick <= 0 {
        return events;
    }
    let confirmed_tick = state.input_tick - 1;
    for event in &state.events {
        let actor = event
            .player
            .as_deref()
            .and_then(|id| slot_and_team_of_id(state, id));
        events.push(EnvObservedEvent {
            kind: event.kind.wire_str().to_string(),
            tick: confirmed_tick,
            x: event.x,
            y: event.y,
            actor_slot: actor.and_then(|(slot, _)| slot),
            actor_side: actor.map(|(_, team)| side_of(team, view_team)),
            on_target: event.on_target,
            result: None,
            family_id: None,
        });
    }
    if let Some(combat_state) = combat_state {
        for event in &combat_state.events {
            let source = event
                .source_index
                .map(|idx| &state.players[idx as usize - 1]);
            events.push(EnvObservedEvent {
                kind: event.kind.wire_str().to_string(),
                tick: event.tick,
                x: event.x,
                y: event.y,
                actor_slot: event
                    .source_index
                    .and_then(|idx| state.slot_for_player[idx as usize - 1]),
                actor_side: source.map(|p| side_of(p.team, view_team)),
                on_target: None,
                result: event.result,
                family_id: event.family_id,
            });
        }
    }
    events
}

/// The privileged block reuses a snapshot the caller already captured when
/// one is supplied: `env.step` needs the canonical snapshot for the boundary
/// hash anyway, and capturing it a second time here would triple the cost of
/// a privileged step.
fn privileged_view(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    context: Option<&EnvObservationContext>,
) -> EnvPrivilegedView {
    let snapshot = match context.and_then(|c| c.snapshot.clone()) {
        Some(snapshot) => snapshot,
        None => {
            // No donated snapshot was available to reuse: record it (see
            // `EnvObservationContext::redundant_captures`'s doc) and capture
            // the boundary ourselves.
            if let Some(context) = context {
                context
                    .redundant_captures
                    .set(context.redundant_captures.get() + 1);
            }
            match_snapshot::capture(state, combat_state)
        }
    };
    let boundary_hash = context
        .and_then(|c| c.boundary_hash.clone())
        .unwrap_or_else(|| match_snapshot::hash(&snapshot));
    EnvPrivilegedView {
        rng: state.rng,
        snapshot,
        boundary_hash,
    }
}

fn resolve_slot(state: &MatchState, slot: i64) -> Result<(i64, input_frame::InputSlot)> {
    let canonical_slot = input_frame::slot(slot)
        .map_err(|e| EnvObservationError::new(EnvObservationErrorCode::Malformed, e.message))?;
    if !state.slot_mode {
        return failure(
            EnvObservationErrorCode::Malformed,
            "observations require a fixed-slot match",
        );
    }
    let player_index = state.slot_players.get(slot as usize - 1).copied().flatten();
    let player_index = player_index.ok_or_else(|| {
        EnvObservationError::new(
            EnvObservationErrorCode::Malformed,
            format!("slot {slot} is not routed to a player"),
        )
    })?;
    Ok((player_index, canonical_slot))
}

/// The narrow projection an action mask needs: own readiness and the ball,
/// and nothing else. `env_action.mask` reads only these, so building a full
/// view to compute legality would allocate a teammate/opponent/event record
/// set per slot per step for no gain. It is built from the same
/// player-observable projections as the full view, so the two cannot drift
/// apart.
pub fn action_view(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    slot: i64,
    profile: EnvObservationProfile,
) -> Result<EnvActionView> {
    let (player_index, canonical_slot) = resolve_slot(state, slot)?;
    let view_team = to_state_team(canonical_slot.team);
    Ok(EnvActionView {
        version: VERSION,
        profile,
        slot,
        team: canonical_slot.team,
        ball: observed_ball(state, view_team),
        own: observed_self(state, combat_state, player_index, slot),
    })
}

/// Build one slot's view. `profile` only widens the result: the
/// player-observable body is identical for every profile, and `privileged`
/// appends the authoritative block on top of it.
pub fn view(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    slot: i64,
    profile: EnvObservationProfile,
    context: Option<&EnvObservationContext>,
) -> Result<EnvSlotView> {
    let (player_index, canonical_slot) = resolve_slot(state, slot)?;
    let view_team = to_state_team(canonical_slot.team);
    let mut teammates = Vec::new();
    let mut opponents = Vec::new();
    for (i, player) in state.players.iter().enumerate() {
        let index = i as i64 + 1;
        if index != player_index {
            let observed = observed_player(state, combat_state, index, view_team);
            if player.team == view_team {
                teammates.push(observed);
            } else {
                opponents.push(observed);
            }
        }
    }
    let (own_goal, target_goal) = match view_team {
        match_snapshot::Team::Home => (state.goal_home, state.goal_away),
        match_snapshot::Team::Away => (state.goal_away, state.goal_home),
    };
    let (score_own, score_opponent) = match view_team {
        match_snapshot::Team::Home => (state.score.home, state.score.away),
        match_snapshot::Team::Away => (state.score.away, state.score.home),
    };
    let privileged = if profile == EnvObservationProfile::Privileged {
        Some(privileged_view(state, combat_state, context))
    } else {
        None
    };
    Ok(EnvSlotView {
        version: VERSION,
        profile,
        slot,
        slot_id: canonical_slot.id,
        team: canonical_slot.team,
        r#match: EnvObservedMatch {
            tick: state.input_tick,
            tick_rate: fixed_clock::TICK_RATE as i64,
            phase: phase_of(state),
            stoppage: state.kickoff_hold > 0.0,
            time_left: state.time_left,
            score_own,
            score_opponent,
            max_goals: state.max_goals,
            finished: state.finished,
        },
        geometry: EnvObservedGeometry {
            field_w: state.field.w,
            field_h: state.field.h,
            own_goal,
            target_goal,
        },
        ball: observed_ball(state, view_team),
        own: observed_self(state, combat_state, player_index, slot),
        teammates,
        opponents,
        events: observed_events(state, combat_state, view_team),
        privileged,
    })
}

/// Build the observation for a profile over the controlled slots. Each
/// slot's view is built from the same boundary state, independently: no view
/// can carry another slot's pending action because no action has been
/// accepted yet.
pub fn build(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    slots: &[i64],
    profile: EnvObservationProfile,
    context: Option<&EnvObservationContext>,
) -> Result<EnvObservation> {
    let data = profile_data(profile);
    if slots.is_empty() {
        return failure(
            EnvObservationErrorCode::Malformed,
            "observations need at least one controlled slot",
        );
    }
    if !data.multi_slot && slots.len() != 1 {
        return failure(
            EnvObservationErrorCode::ProfileMismatch,
            format!(
                "the {} profile covers exactly one controlled slot",
                profile_wire_str(profile)
            ),
        );
    }
    let mut views: Vec<Option<EnvSlotView>> = vec![None; input_frame::SLOT_COUNT as usize];
    let mut ordered = Vec::with_capacity(slots.len());
    let mut previous = 0i64;
    for &slot in slots {
        if slot <= previous {
            return failure(
                EnvObservationErrorCode::Malformed,
                "controlled slots must be ascending unique integers",
            );
        }
        previous = slot;
        let slot_view = view(state, combat_state, slot, profile, context)?;
        views[(slot - 1) as usize] = Some(slot_view);
        ordered.push(slot);
    }
    Ok(EnvObservation {
        version: VERSION,
        profile,
        player_observable: data.player_observable,
        human_proxy_valid: data.human_proxy_valid,
        tick: state.input_tick,
        slots: ordered,
        views,
    })
}

/// The requested slot's view, or `None` when that slot is not covered.
#[must_use]
pub fn view_for(observation: &EnvObservation, slot: i64) -> Option<&EnvSlotView> {
    if slot < 1 {
        return None;
    }
    observation
        .views
        .get(slot as usize - 1)
        .and_then(|v| v.as_ref())
}

// ---------------------------------------------------------------------------
// Canonical encoding.
// ---------------------------------------------------------------------------

/// A canonical scalar or nested table — this port's stand-in for the dynamic
/// value the Lua original's `encode_value` walks via `pairs()`. Building this
/// tree from typed structs is how a fixed-shape Rust port still reproduces
/// that walk: each `*_to_canonical` function below lists only the fields
/// actually present. An absent `Option::None` in Lua is an absent table key
/// (`pairs` never yields a nil-valued key — a Lua table constructor's
/// `field = nil` never creates the key at all), not a key paired with a nil
/// value, so these functions omit a `None` field's key entirely rather than
/// emitting a null marker for it. Getting this right matters: it is what
/// keeps the canonical byte count (`t<N>;`) and the sorted key set faithful
/// to what the Lua encoder would actually produce for the same observation.
enum CanonicalValue {
    Bool(bool),
    Number(f64),
    Text(String),
    Table(Vec<(CanonicalTableKey, CanonicalValue)>),
}

/// A canonical table key: numeric (array position or slot number) or string
/// (a struct field name).
enum CanonicalTableKey {
    Number(f64),
    Text(String),
}

fn num(value: f64) -> CanonicalValue {
    CanonicalValue::Number(value)
}

fn table(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Table(
        entries
            .into_iter()
            .map(|(k, v)| (CanonicalTableKey::Text(k.to_string()), v))
            .collect(),
    )
}

fn array<T>(items: &[T], to_canonical: impl Fn(&T) -> CanonicalValue) -> CanonicalValue {
    CanonicalValue::Table(
        items
            .iter()
            .enumerate()
            .map(|(i, v)| (CanonicalTableKey::Number((i + 1) as f64), to_canonical(v)))
            .collect(),
    )
}

fn number_array(items: &[i64]) -> CanonicalValue {
    array(items, |v| num(*v as f64))
}

fn sparse_views(views: &[Option<EnvSlotView>]) -> CanonicalValue {
    CanonicalValue::Table(
        views
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                v.as_ref().map(|view| {
                    (
                        CanonicalTableKey::Number((i + 1) as f64),
                        slot_view_to_canonical(view),
                    )
                })
            })
            .collect(),
    )
}

fn relative_side_wire(side: EnvRelativeSide) -> &'static str {
    match side {
        EnvRelativeSide::Own => "own",
        EnvRelativeSide::Opponent => "opponent",
    }
}

fn match_phase_wire(phase: EnvMatchPhase) -> &'static str {
    match phase {
        EnvMatchPhase::Kickoff => "kickoff",
        EnvMatchPhase::Live => "live",
        EnvMatchPhase::Finished => "finished",
    }
}

fn slot_id_wire(id: input_frame::SlotId) -> &'static str {
    match id {
        input_frame::SlotId::Home1 => "home_1",
        input_frame::SlotId::Home2 => "home_2",
        input_frame::SlotId::Home3 => "home_3",
        input_frame::SlotId::Home4 => "home_4",
        input_frame::SlotId::Away1 => "away_1",
        input_frame::SlotId::Away2 => "away_2",
        input_frame::SlotId::Away3 => "away_3",
        input_frame::SlotId::Away4 => "away_4",
    }
}

fn input_team_wire(team: input_frame::Team) -> &'static str {
    match team {
        input_frame::Team::Home => "home",
        input_frame::Team::Away => "away",
    }
}

fn contact_result_wire(result: CombatContactResult) -> &'static str {
    match result {
        CombatContactResult::Hit => "hit",
        CombatContactResult::Extended => "extended",
        CombatContactResult::Guarded => "guarded",
        CombatContactResult::Immune => "immune",
        CombatContactResult::Superseded => "superseded",
    }
}

fn forced_state_wire(state: CombatForcedState) -> &'static str {
    match state {
        CombatForcedState::Stagger => "stagger",
        CombatForcedState::Knockback => "knockback",
    }
}

fn rect_to_canonical(rect: &Rect) -> CanonicalValue {
    table(vec![
        ("x", num(rect.x)),
        ("y", num(rect.y)),
        ("w", num(rect.w)),
        ("h", num(rect.h)),
    ])
}

fn geometry_to_canonical(geometry: &EnvObservedGeometry) -> CanonicalValue {
    table(vec![
        ("field_w", num(geometry.field_w)),
        ("field_h", num(geometry.field_h)),
        ("own_goal", rect_to_canonical(&geometry.own_goal)),
        ("target_goal", rect_to_canonical(&geometry.target_goal)),
    ])
}

fn match_to_canonical(m: &EnvObservedMatch) -> CanonicalValue {
    table(vec![
        ("tick", num(m.tick as f64)),
        ("tick_rate", num(m.tick_rate as f64)),
        (
            "phase",
            CanonicalValue::Text(match_phase_wire(m.phase).to_string()),
        ),
        ("stoppage", CanonicalValue::Bool(m.stoppage)),
        ("time_left", num(m.time_left)),
        ("score_own", num(m.score_own as f64)),
        ("score_opponent", num(m.score_opponent as f64)),
        ("max_goals", num(m.max_goals as f64)),
        ("finished", CanonicalValue::Bool(m.finished)),
    ])
}

fn ball_to_canonical(ball: &EnvObservedBall) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        ("x", num(ball.x)),
        ("y", num(ball.y)),
        ("z", num(ball.z)),
        ("vx", num(ball.vx)),
        ("vy", num(ball.vy)),
        ("vz", num(ball.vz)),
        ("spin", num(ball.spin)),
        ("airborne", CanonicalValue::Bool(ball.airborne)),
        ("loose", CanonicalValue::Bool(ball.loose)),
        (
            "owner_is_keeper",
            CanonicalValue::Bool(ball.owner_is_keeper),
        ),
    ];
    if let Some(slot) = ball.owner_slot {
        entries.push(("owner_slot", num(slot as f64)));
    }
    if let Some(side) = ball.owner_side {
        entries.push((
            "owner_side",
            CanonicalValue::Text(relative_side_wire(side).to_string()),
        ));
    }
    table(entries)
}

fn equipment_to_canonical(equipment: &EnvObservedEquipment) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        (
            "family_id",
            CanonicalValue::Text(
                combat_snapshot::action_family_wire_id(equipment.family_id).to_string(),
            ),
        ),
        (
            "phase",
            CanonicalValue::Text(combat_snapshot::phase_wire_str(equipment.phase).to_string()),
        ),
    ];
    if let Some(state) = equipment.forced_state {
        entries.push((
            "forced_state",
            CanonicalValue::Text(forced_state_wire(state).to_string()),
        ));
    }
    table(entries)
}

fn player_to_canonical(player: &EnvObservedPlayer) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        (
            "side",
            CanonicalValue::Text(relative_side_wire(player.side).to_string()),
        ),
        ("is_keeper", CanonicalValue::Bool(player.is_keeper)),
        ("x", num(player.x)),
        ("y", num(player.y)),
        ("vx", num(player.vx)),
        ("vy", num(player.vy)),
        ("facing_x", num(player.facing_x)),
        ("facing_y", num(player.facing_y)),
        ("radius", num(player.radius)),
        ("has_ball", CanonicalValue::Bool(player.has_ball)),
        ("sprinting", CanonicalValue::Bool(player.sprinting)),
        ("sliding", CanonicalValue::Bool(player.sliding)),
        ("diving", CanonicalValue::Bool(player.diving)),
        ("airborne", CanonicalValue::Bool(player.airborne)),
        ("winding_up", CanonicalValue::Bool(player.winding_up)),
    ];
    if let Some(slot) = player.slot {
        entries.push(("slot", num(slot as f64)));
    }
    if let Some(equipment) = &player.equipment {
        entries.push(("equipment", equipment_to_canonical(equipment)));
    }
    table(entries)
}

fn self_equipment_to_canonical(equipment: &EnvObservedSelfEquipment) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        (
            "loadout_id",
            CanonicalValue::Text(equipment.loadout_id.clone()),
        ),
        (
            "family_id",
            CanonicalValue::Text(
                combat_snapshot::action_family_wire_id(equipment.family_id).to_string(),
            ),
        ),
        (
            "phase",
            CanonicalValue::Text(combat_snapshot::phase_wire_str(equipment.phase).to_string()),
        ),
        ("phase_ticks", num(equipment.phase_ticks as f64)),
        ("cooldown_ticks", num(equipment.cooldown_ticks as f64)),
        ("ready", CanonicalValue::Bool(equipment.ready)),
        ("control_held", CanonicalValue::Bool(equipment.control_held)),
        ("forced_ticks", num(equipment.forced_ticks as f64)),
        ("immunity_ticks", num(equipment.immunity_ticks as f64)),
    ];
    if let Some(state) = equipment.forced_state {
        entries.push((
            "forced_state",
            CanonicalValue::Text(forced_state_wire(state).to_string()),
        ));
    }
    table(entries)
}

fn self_to_canonical(own: &EnvObservedSelf) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        ("player_id", CanonicalValue::Text(own.player_id.clone())),
        ("slot", num(own.slot as f64)),
        (
            "side",
            CanonicalValue::Text(relative_side_wire(own.side).to_string()),
        ),
        ("is_keeper", CanonicalValue::Bool(own.is_keeper)),
        ("x", num(own.x)),
        ("y", num(own.y)),
        ("vx", num(own.vx)),
        ("vy", num(own.vy)),
        ("facing_x", num(own.facing_x)),
        ("facing_y", num(own.facing_y)),
        ("radius", num(own.radius)),
        ("move_speed", num(own.move_speed)),
        ("has_ball", CanonicalValue::Bool(own.has_ball)),
        ("sprinting", CanonicalValue::Bool(own.sprinting)),
        ("jockeying", CanonicalValue::Bool(own.jockeying)),
        ("sliding", CanonicalValue::Bool(own.sliding)),
        ("tackling", CanonicalValue::Bool(own.tackling)),
        ("dodging", CanonicalValue::Bool(own.dodging)),
        ("stunned", CanonicalValue::Bool(own.stunned)),
        ("diving", CanonicalValue::Bool(own.diving)),
        ("airborne", CanonicalValue::Bool(own.airborne)),
        ("winding_up", CanonicalValue::Bool(own.winding_up)),
        ("charging", CanonicalValue::Bool(own.charging)),
        ("sprint_meter", num(own.sprint_meter)),
        ("charge", num(own.charge)),
        ("pass_charge", num(own.pass_charge)),
        ("dash_ready", CanonicalValue::Bool(own.dash_ready)),
        ("dodge_ready", CanonicalValue::Bool(own.dodge_ready)),
        ("tackle_ready", CanonicalValue::Bool(own.tackle_ready)),
        ("header_ready", CanonicalValue::Bool(own.header_ready)),
        ("dash_cooldown_s", num(own.dash_cooldown_s)),
        ("dodge_cooldown_s", num(own.dodge_cooldown_s)),
        ("tackle_cooldown_s", num(own.tackle_cooldown_s)),
        ("header_cooldown_s", num(own.header_cooldown_s)),
        ("stun_s", num(own.stun_s)),
        ("dodge_s", num(own.dodge_s)),
        ("slide_s", num(own.slide_s)),
        ("tackle_s", num(own.tackle_s)),
        ("jockey_s", num(own.jockey_s)),
        ("windup_s", num(own.windup_s)),
        ("aerial_recovery_s", num(own.aerial_recovery_s)),
        (
            "pass_preview_keeper",
            CanonicalValue::Bool(own.pass_preview_keeper),
        ),
    ];
    if let Some(slot) = own.pass_preview_slot {
        entries.push(("pass_preview_slot", num(slot as f64)));
    }
    if let Some(equipment) = &own.equipment {
        entries.push(("equipment", self_equipment_to_canonical(equipment)));
    }
    table(entries)
}

fn event_to_canonical(event: &EnvObservedEvent) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        ("kind", CanonicalValue::Text(event.kind.clone())),
        ("tick", num(event.tick as f64)),
        ("x", num(event.x)),
        ("y", num(event.y)),
    ];
    if let Some(slot) = event.actor_slot {
        entries.push(("actor_slot", num(slot as f64)));
    }
    if let Some(side) = event.actor_side {
        entries.push((
            "actor_side",
            CanonicalValue::Text(relative_side_wire(side).to_string()),
        ));
    }
    if let Some(on_target) = event.on_target {
        entries.push(("on_target", CanonicalValue::Bool(on_target)));
    }
    if let Some(result) = event.result {
        entries.push((
            "result",
            CanonicalValue::Text(contact_result_wire(result).to_string()),
        ));
    }
    if let Some(family_id) = event.family_id {
        entries.push((
            "family_id",
            CanonicalValue::Text(combat_snapshot::action_family_wire_id(family_id).to_string()),
        ));
    }
    table(entries)
}

fn privileged_to_canonical(privileged: &EnvPrivilegedView) -> CanonicalValue {
    // See the module doc: `snapshot` is represented by its own canonical
    // hash rather than a full field walk of `MatchSnapshot`.
    table(vec![
        ("rng", num(f64::from(privileged.rng))),
        (
            "snapshot",
            CanonicalValue::Text(match_snapshot::hash(&privileged.snapshot)),
        ),
        (
            "boundary_hash",
            CanonicalValue::Text(privileged.boundary_hash.clone()),
        ),
    ])
}

fn slot_view_to_canonical(view: &EnvSlotView) -> CanonicalValue {
    let mut entries: Vec<(&str, CanonicalValue)> = vec![
        ("version", num(view.version as f64)),
        (
            "profile",
            CanonicalValue::Text(profile_wire_str(view.profile).to_string()),
        ),
        ("slot", num(view.slot as f64)),
        (
            "slot_id",
            CanonicalValue::Text(slot_id_wire(view.slot_id).to_string()),
        ),
        (
            "team",
            CanonicalValue::Text(input_team_wire(view.team).to_string()),
        ),
        ("match", match_to_canonical(&view.r#match)),
        ("geometry", geometry_to_canonical(&view.geometry)),
        ("ball", ball_to_canonical(&view.ball)),
        ("own", self_to_canonical(&view.own)),
        ("teammates", array(&view.teammates, player_to_canonical)),
        ("opponents", array(&view.opponents, player_to_canonical)),
        ("events", array(&view.events, event_to_canonical)),
    ];
    if let Some(privileged) = &view.privileged {
        entries.push(("privileged", privileged_to_canonical(privileged)));
    }
    table(entries)
}

fn observation_to_canonical(observation: &EnvObservation) -> CanonicalValue {
    table(vec![
        ("version", num(observation.version as f64)),
        (
            "profile",
            CanonicalValue::Text(profile_wire_str(observation.profile).to_string()),
        ),
        (
            "player_observable",
            CanonicalValue::Bool(observation.player_observable),
        ),
        (
            "human_proxy_valid",
            CanonicalValue::Bool(observation.human_proxy_valid),
        ),
        ("tick", num(observation.tick as f64)),
        ("slots", number_array(&observation.slots)),
        ("views", sparse_views(&observation.views)),
    ])
}

fn append_num(parts: &mut String, value: f64) {
    parts.push('n');
    parts.push_str(&match_snapshot::number_bytes(value));
    parts.push(';');
}

fn append_str(parts: &mut String, value: &str) {
    parts.push('s');
    parts.push_str(&value.len().to_string());
    parts.push(':');
    parts.push_str(value);
    parts.push(';');
}

fn append_key(parts: &mut String, key: &CanonicalTableKey) {
    match key {
        CanonicalTableKey::Number(n) => append_num(parts, *n),
        CanonicalTableKey::Text(s) => append_str(parts, s),
    }
}

/// Mirrors the Lua original's generic `encode_value`: numeric and string
/// keys are collected, sorted independently (numeric group first, matching
/// `table.sort(numeric)` then `table.sort(strings)`), and the sorted union's
/// count leads the table's encoding. Lua's default `<` on strings is a byte
/// comparison, which is exactly what `str`'s `Ord` gives here for the ASCII
/// field names this module ever produces.
fn append_value(parts: &mut String, value: &CanonicalValue) {
    match value {
        CanonicalValue::Bool(b) => parts.push_str(if *b { "b1;" } else { "b0;" }),
        CanonicalValue::Number(n) => append_num(parts, *n),
        CanonicalValue::Text(s) => append_str(parts, s),
        CanonicalValue::Table(entries) => {
            let mut numeric: Vec<&(CanonicalTableKey, CanonicalValue)> = Vec::new();
            let mut stringy: Vec<&(CanonicalTableKey, CanonicalValue)> = Vec::new();
            for entry in entries {
                match &entry.0 {
                    CanonicalTableKey::Number(_) => numeric.push(entry),
                    CanonicalTableKey::Text(_) => stringy.push(entry),
                }
            }
            numeric.sort_by(|a, b| {
                let (CanonicalTableKey::Number(x), CanonicalTableKey::Number(y)) = (&a.0, &b.0)
                else {
                    unreachable!("numeric bucket holds only Number keys");
                };
                x.partial_cmp(y).expect("canonical numeric keys are finite")
            });
            stringy.sort_by(|a, b| {
                let (CanonicalTableKey::Text(x), CanonicalTableKey::Text(y)) = (&a.0, &b.0) else {
                    unreachable!("string bucket holds only Text keys");
                };
                x.cmp(y)
            });
            parts.push('t');
            parts.push_str(&(numeric.len() + stringy.len()).to_string());
            parts.push(';');
            for (k, v) in numeric.into_iter().chain(stringy.into_iter()) {
                append_key(parts, k);
                append_value(parts, v);
            }
        }
    }
}

/// Canonical byte-exact encoding of a complete observation. Two observations
/// encode identically exactly when they carry the same information.
#[must_use]
pub fn encode(observation: &EnvObservation) -> String {
    let mut parts = String::new();
    parts.push_str("GCEO;");
    parts.push_str(&VERSION.to_string());
    parts.push(';');
    append_value(&mut parts, &observation_to_canonical(observation));
    parts
}

/// Canonical byte-exact encoding of a single slot view. See [`encode`]'s doc
/// for why this port splits the Lua original's single
/// `encode(EnvObservation|EnvSlotView)` into two entry points.
#[must_use]
pub fn encode_view(view: &EnvSlotView) -> String {
    let mut parts = String::new();
    parts.push_str("GCEO;");
    parts.push_str(&VERSION.to_string());
    parts.push(';');
    append_value(&mut parts, &slot_view_to_canonical(view));
    parts
}
