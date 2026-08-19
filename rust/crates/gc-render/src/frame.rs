//! `build` turns a [`MatchState`] into a flat, engine-free description of ONE
//! drawable frame. Nothing downstream of it needs to know what a `MatchState`
//! is, and nothing in it needs an engine.
//!
//! Three rules shape this module, and none of them are style preferences:
//!
//! 1. THE BOUNDARY IS CROSSED ONCE PER RENDERED FRAME, IN BATCH. Never per
//!    entity, never per tick. Rollback re-simulates up to eight ticks inside a
//!    single rendered frame; a per-tick crossing is the thing that would make
//!    a non-Rust renderer unaffordable. [`build`] is therefore one call
//!    producing one whole frame.
//!
//! 2. PER-ENTITY DATA IS STRUCTURE-OF-ARRAYS. [`RenderFramePlayers`] is a set
//!    of parallel arrays indexed by roster slot, not an array of structs. The
//!    sparse ones (`aerial_style`, ...) are `Vec<Option<T>>`: an absent entry
//!    is `None`, and [`crate::frame_buffer`] maps that to a zero enum. Booleans
//!    encode as 0/1 there, not here.
//!
//! 3. PRESENTATION-DERIVED STATE STAYS ON THE RENDERER SIDE. Gait, lean, the
//!    smoothed on-screen speed, the correction smoothing state machine, the
//!    release follow-through window and the dispossession flinch window are
//!    NOT simulation and are not derived here. They feed in as explicit
//!    inputs ([`RenderFrameOptions::render_pose`],
//!    [`RenderFrameOptions::kick_follow`], [`RenderFrameOptions::dispossessed`])
//!    because the frame must report the positions and poses actually shown;
//!    their state machines stay where they are (`@gc/render`, TypeScript).
//!
//! The frame splits into a static half and a per-frame half. [`RenderFrameRoster`]
//! is match-constant (ids, teams, species shape and palette) and crosses
//! once — pass it back in as [`RenderFrameOptions::roster`] so it is not
//! rebuilt. Everything else in [`RenderFrame`] is rebuilt each frame.
//!
//! Versioning borrows the STAMPING convention of `sim::input_frame` and
//! `sim::match_snapshot` — an integer [`VERSION`] written into every payload
//! and bumped whenever the shape changes — and only that half.
//! [`crate::frame_buffer`] is the first consumer that deserializes this
//! across a boundary and is where the read-side assertion on `version` lives.

use gc_core::vec2::Vec2;
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::brain::TeamPhase;
use gc_sim::combat::IMMUNITY_TICKS;
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::keeper::{self, KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{
    ByTeam, MatchEvent, MatchEventKind, MatchPlayer, MatchState, Rect, Team,
};
use gc_sim::outfield_press::StablePressMode;
use gc_sim::possession_transition::{self, TransitionTeam};
use gc_sim::tunable_registry::Registry;

use crate::identity;
use crate::player_pose::{
    self, CombatPoseSample, KeeperPoseContext, OutfieldPoseContext, PlayerPoseId, PlayerPoseSource,
};

/// This protocol's version. Bump whenever [`RenderFrame`]'s shape changes.
/// 1 → 2: [`RenderFramePlayers::phase_fraction`] (#576).
pub const VERSION: u32 = 2;

/// A charge the locally controlled player has partway built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderChargeKind {
    /// A shot charge.
    Shot,
    /// A pass charge.
    Pass,
}

/// Match-constant per-player identity. Crosses the boundary once, not per
/// frame.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFrameRoster {
    /// Exactly [`VERSION`].
    pub version: u32,
    /// Roster slot count.
    pub count: usize,
    /// One-based-order stable player ids, indexed 0..count.
    pub ids: Vec<String>,
    /// Display names (the authored presentation name).
    pub names: Vec<String>,
    /// The authored `gc_data::players::PlayerData::presentation_id` per slot
    /// — which character presentation, and therefore which theme, the
    /// renderer builds this player's geometry from (#447). Always present:
    /// every authored player names one.
    pub presentation_ids: Vec<String>,
    /// The authored `gc_data::players::PlayerData::loadout_id` per slot, or
    /// `None` where the player carries nothing (#447).
    ///
    /// `None` IS THE KEEPER RULE, not a missing value. `gc-data` states it
    /// ("Fixed prototype loadout; keepers have none") and
    /// `gc-data/tests/players.rs` enforces it in both directions, so a slot
    /// with no loadout must survive to the renderer as an absence rather
    /// than being defaulted into whatever the theme happens to carry — which
    /// is exactly the defect #447 records, keepers diving with a shield.
    pub loadout_ids: Vec<Option<String>>,
    /// Fixture side per slot.
    pub teams: Vec<Team>,
    /// Whether each slot is the keeper.
    pub is_keeper: Vec<bool>,
    /// Collision radius per slot, px.
    pub radius: Vec<f64>,
    /// Visual silhouette family per slot.
    pub species_shape: Vec<gc_data::species::Shape>,
    /// `{r, g, b}` in 0..1 per slot.
    pub species_color: Vec<[f64; 3]>,
}

/// Pitch geometry the renderer draws lines and goals from. Constant for a
/// match, carried on the frame because it is three numbers and a pair of
/// rects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderFrameField {
    /// Playable width, px.
    pub w: f64,
    /// Playable height, px.
    pub h: f64,
    /// Crossbar height, px.
    pub crossbar_h: f64,
    /// Penalty box depth, px.
    pub penalty_box_depth: f64,
    /// Penalty box height, px.
    pub penalty_box_h: f64,
    /// Left goal mouth; away scores here.
    pub goal_home: Rect,
    /// Right goal mouth; home scores here.
    pub goal_away: Rect,
}

/// Structure of arrays, indexed 0..count by roster slot. Every array is the
/// same length; the sparse ones (`Vec<Option<T>>`) leave holes where a value
/// does not apply. Timers arrive already normalised to 0..1 so no renderer
/// has to re-derive a duration constant.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFramePlayers {
    /// Roster slot count.
    pub count: usize,
    /// Displayed world position (correction smoothing applied), x.
    pub x: Vec<f64>,
    /// Displayed world position (correction smoothing applied), y.
    pub y: Vec<f64>,
    /// Facing direction, x.
    pub facing_x: Vec<f64>,
    /// Facing direction, y.
    pub facing_y: Vec<f64>,
    /// Locomotion speed in world units/sec, straight off the sim.
    pub speed: Vec<f64>,
    /// The pose selected for each slot.
    pub pose_id: Vec<PlayerPoseId>,
    /// The priority the selected pose won at.
    pub pose_priority: Vec<i64>,
    /// Which system the selected pose came from.
    pub pose_source: Vec<PlayerPoseSource>,
    /// Whether this slot is the locally controlled player.
    pub controlled: Vec<bool>,
    /// Whether this slot is mid slide-tackle.
    pub dashing: Vec<bool>,
    /// Keeper carrying the ball in the hands.
    pub holding: Vec<bool>,
    /// 0..1 dive ease.
    pub dive: Vec<f64>,
    /// Dive direction, x.
    pub dive_dir_x: Vec<f64>,
    /// Dive direction, y.
    pub dive_dir_y: Vec<f64>,
    /// 0..1 grab ease.
    pub grab: Vec<f64>,
    /// 0..1 throw ease.
    pub throw: Vec<f64>,
    /// Wind-up back-swing; 0 = none, 1 = just committed, unclamped above 1.
    pub windup: Vec<f64>,
    /// 0..1 aerial ease.
    pub aerial: Vec<f64>,
    /// 0..1 required lift for rendering.
    pub aerial_jump: Vec<f64>,
    /// Physical technique used, if mid aerial attempt. Sparse.
    pub aerial_style: Vec<Option<AerialStyle>>,
    /// Resolved aerial outcome, if any. Sparse.
    pub aerial_outcome: Vec<Option<AerialOutcome>>,
    /// Elapsed progress through this slot's current timed combat phase, in
    /// `[0, 1)` — [`CombatPoseSample::phase_fraction`], carried per player so
    /// the animator can sweep the swing clip through its strike key (#576).
    /// `0.0` for a match without combat, for the held combat phases, and for
    /// a player with no action family; dense, never sparse, because `0.0`
    /// ("at the start of the phase, or nothing to report") is the correct
    /// reading in every one of those cases.
    pub phase_fraction: Vec<f64>,
}

/// The ball's per-frame presentation state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderFrameBall {
    /// Displayed world position (correction smoothing applied), x.
    pub x: f64,
    /// Displayed world position (correction smoothing applied), y.
    pub y: f64,
    /// Height above the pitch.
    pub z: f64,
    /// x velocity.
    pub vx: f64,
    /// y velocity.
    pub vy: f64,
    /// z velocity.
    pub vz: f64,
    /// False while a keeper holds it in the hands.
    pub visible: bool,
    /// Ballistic landing point of a lofted loose ball, x. `None` if none.
    pub landing_x: Option<f64>,
    /// Ballistic landing point of a lofted loose ball, y. `None` if none.
    pub landing_y: Option<f64>,
}

/// Which roster slot, if any, holds the ball this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderFramePossession {
    /// Roster slot holding the ball; `None` if loose. One-based (README
    /// rule 3's wire-identity exception: this is the same slot number
    /// carried in [`RenderFrameHud::controlled`] and an event's `slot`).
    pub owner: Option<i64>,
    /// The owning slot's team.
    pub owner_team: Option<Team>,
    /// Owner is a keeper carrying it in the hands.
    pub keeper_holds: bool,
}

/// What the locally controlled player is doing with the input. Distinct
/// from [`RenderFramePossession`]: a player can be charging a shot without
/// the ball.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderFrameControl {
    /// Roster slot, one-based.
    pub controlled: i64,
    /// Roster slot the pass would go to, `None` if none.
    pub pass_target: Option<i64>,
    /// `None` when nothing is charging.
    pub charge_kind: Option<RenderChargeKind>,
    /// 0..1; 0 when `charge_kind` is `None`.
    pub charge: f64,
}

/// Scoreboard/identity facts a HUD needs, all derived from the simulation.
/// Match metadata (team names, arena, tactic) is authored content supplied
/// by the screen, not per-frame simulation state, so it stays out of the
/// payload.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFrameHud {
    /// Home goals scored.
    pub home_score: i64,
    /// Away goals scored.
    pub away_score: i64,
    /// Seconds remaining.
    pub time_left: f64,
    /// Whether the match has finished.
    pub finished: bool,
    /// `None` while the ball is loose.
    pub possession_team: Option<Team>,
    /// Roster slot, one-based.
    pub controlled: i64,
    /// Controlled player's stable id.
    pub controlled_id: String,
    /// Controlled player's team.
    pub controlled_team: Team,
    /// Whether the controlled player is the keeper.
    pub controlled_is_keeper: bool,
    /// Whether the controlled player owns the ball.
    pub controlled_owns_ball: bool,
    /// 0..1 sprint meter, clamped for the meter widget.
    pub controlled_stamina: f64,
    /// Controlled player's silhouette family.
    pub species_shape: gc_data::species::Shape,
    /// Controlled player's `{r, g, b}` palette.
    pub species_color: [f64; 3],
}

/// This frame's discrete match events, flattened. This is the
/// effect-trigger channel: a renderer spawns particles, shakes and audio
/// cues from it without ever seeing a [`MatchEvent`].
///
/// The non-boolean optional arrays are sparse: an absent entry is `None`,
/// which [`crate::frame_buffer`] maps to a zero enum. `jumping`/`on_target`
/// are TRI-STATE rather than sparse booleans: absent ("this event kind does
/// not report it"), reported `false`, and reported `true` are three distinct,
/// real states — a released outfield shot carries `on_target` while a
/// keeper's distribution kick, also `Shot`, does not. 0 = not reported, 1 =
/// false, 2 = true.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFrameEvents {
    /// Event count.
    pub count: usize,
    /// Event kind per entry.
    pub kind: Vec<MatchEventKind>,
    /// World x position per entry.
    pub x: Vec<f64>,
    /// World y position per entry.
    pub y: Vec<f64>,
    /// The actor's roster id, if attributed. Sparse.
    pub player: Vec<Option<String>>,
    /// The actor's roster slot, if attributed. Sparse.
    pub slot: Vec<Option<i64>>,
    /// Save presentation style, for a save event. Sparse.
    pub save_style: Vec<Option<SaveStyle>>,
    /// Aerial style, for an aerial event. Sparse.
    pub style: Vec<Option<AerialStyle>>,
    /// Aerial outcome, for an aerial event. Sparse.
    pub outcome: Vec<Option<AerialOutcome>>,
    /// Save difficulty. Sparse.
    pub difficulty: Vec<Option<f64>>,
    /// Shot type, for a keeper-relevant event. Sparse.
    pub shot_type: Vec<Option<KeeperShotType>>,
    /// Keeper behavior state at the moment of this event. Sparse.
    pub keeper_state: Vec<Option<KeeperBehaviorState>>,
    /// Keeper line depth the strike was released from. Sparse.
    pub keeper_depth: Vec<Option<f64>>,
    /// Tri-state: 0 = not reported, 1 = false, 2 = true. Dense.
    pub jumping: Vec<i64>,
    /// Tri-state: 0 = not reported, 1 = false, 2 = true. Dense.
    pub on_target: Vec<i64>,
}

/// The slice of `@gc/presentation`'s `CombatPresentationModel` (TS-owned)
/// this module reads. `RenderFrame.combat` carries the whole model through
/// the frame unflattened and reads only `combat.players[index]`'s
/// phase/forced-state/immunity fields, to hand [`crate::player_pose::select`]
/// its `combat` sample and to give a renderer the post-hit immunity cue
/// [`crate::player_pose::CombatPoseSample::immunity_fraction`] carries;
/// [`crate::frame_buffer`] never encodes any of it (see that module's own
/// doc, "WHAT IS NOT CARRIED").
///
/// ## This type is narrow on purpose, and it is NOT blocked on marshalling
///
/// This doc used to say the JS↔wasm marshalling layer that would carry a
/// live model into this crate was "a separate, out-of-scope milestone", and
/// a reader took that to mean the whole model — pose selection included —
/// had to wait for it. It does not, and reading it that way is what left
/// [`RenderFrameOptions::combat`] unpopulated at both production
/// construction sites, making all seven combat poses structurally
/// unreachable in a real match (#441; #426 was the identical mistake one
/// field over).
///
/// The distinction is which half of `CombatPresentationModel` a field comes
/// from:
///
/// - The RICH PRESENTATION half — equipment and family display names,
///   telegraph kinds, reach/arc geometry, readiness fractions — is genuinely
///   TypeScript-owned and genuinely does need a marshalling layer this crate
///   does not have. None of it is declared here.
/// - The POSE-SELECTION half — `phase`, `forced_state`, `forced_ticks`, plus
///   `immunity_ticks` (carried as [`crate::player_pose::CombatPoseSample::immunity_fraction`],
///   normalised below) and the current phase's progress (carried as
///   [`crate::player_pose::CombatPoseSample::phase_fraction`], #576) — is not
///   TypeScript-owned at all. It is already
///   native Rust simulation state, stepped every tick, on this side of the
///   wall: `gc_sim::combat_snapshot::CombatPlayerState`. [`combat_model`]
///   below adapts it in-process, with no boundary crossing whatsoever.
///
/// So this type stays narrow because pose selection and its renderer's
/// cues only ever needed five fields, not because the data was out
/// of reach. `immunity_fraction` and `phase_fraction` are the two fields
/// `select` itself never reads — see [`crate::player_pose::CombatPoseSample`]'s
/// own doc — but they are still the POSE-SELECTION half, not the RICH
/// PRESENTATION one: both are native Rust state read straight off
/// `CombatPlayerState`, not a TypeScript-computed projection.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameCombatModel {
    /// RESERVED, AND NOT THE SIGNAL ANYTHING READS. Carried for shape
    /// parity with `@gc/presentation`'s `CombatPresentationModel.enabled`
    /// field, which that TypeScript side uses to distinguish an
    /// empty-but-present model from a real one. Rust has `Option`, and this
    /// module uses it instead: [`combat_model`] hardcodes `true` and is
    /// only ever called for a match that runs combat, so this field carries
    /// no information a reader can act on.
    ///
    /// TODAY'S ACTUAL SIGNAL IS `Option::is_some` on
    /// [`RenderFrameOptions::combat`] / [`RenderFrame::combat`] — that is
    /// what [`build`] branches on to offer
    /// [`crate::player_pose::select`] a combat sample, and what
    /// [`crate::frame_buffer`] encodes as its header's `combat_present`.
    /// Neither reads this field. Anything that starts branching on combat
    /// should branch on the `Option` too, or give this field a meaning
    /// first; do not read a `true` here as an independent confirmation of
    /// anything.
    pub enabled: bool,
    /// One entry per roster slot; exactly the shape
    /// [`crate::player_pose::CombatPoseSample`] reads.
    pub players: Vec<CombatPoseSample>,
}

// [`CombatPoseSample::phase_fraction`]'s normalisation (#576): elapsed
// progress through the current TIMED phase, `(total - remaining) / total`,
// where `total` is the phase length `gc_data::action_families` authors for
// this player's family. The held phases (ready, guard, aim) have no fixed
// length, so they — and a player with no family — report `0.0`. Clamped so
// a `phase_ticks` outside `[0, total]` (an interrupt reset mid-phase) can
// never send the animator outside the clip.
fn phase_fraction(runtime: &gc_sim::combat_snapshot::CombatPlayerState) -> f64 {
    let Some(family_id) = runtime.family_id else {
        return 0.0;
    };
    let family = gc_data::action_families::get(family_id);
    let total = match runtime.phase {
        CombatActionPhase::Windup => family.windup_ticks,
        // Melee/unarmed author `active_ticks`; ranged's is 1. `unwrap_or(1)`
        // rather than `expect` because a guard runtime that somehow reported
        // `Active` should degrade to "phase over" (fraction 0 of 1), not take
        // the frame down — the phase machine upstream never produces it.
        CombatActionPhase::Active => family.active_ticks.unwrap_or(1),
        CombatActionPhase::Recovery => family.recovery_ticks,
        CombatActionPhase::Ready | CombatActionPhase::Guard | CombatActionPhase::Aim => return 0.0,
    };
    if total <= 0 {
        return 0.0;
    }
    let elapsed = (total - runtime.phase_ticks).clamp(0, total);
    elapsed as f64 / total as f64
}

/// Adapt one live [`CombatMatchState`] into the pose-selection slice
/// [`build`] hands [`crate::player_pose::select`], one entry per roster slot
/// in canonical player order.
///
/// This is the in-process adapter [`FrameCombatModel`]'s doc describes: both
/// sides are Rust in the same process, so nothing is marshalled, copied
/// across a boundary, or re-derived. `phase` in particular needs no mapping
/// at all — `gc_sim::combat_snapshot::CombatPhase` is a re-exported alias of
/// `gc_sim::combat_feasibility::CombatActionPhase` (`combat_snapshot.rs`'s
/// `pub type CombatPhase = CombatActionPhase`), which is the very type
/// [`crate::player_pose::CombatPoseSample::phase`] declares, so the field
/// copy below is the compiler's own identity, not a hand-written
/// correspondence that could silently pick the wrong pose.
///
/// Mirrors `@gc/presentation`'s `presentation.model` for the three fields it
/// shares with it, including that function's identity assertions.
///
/// # Panics
///
/// If `combat` does not describe `state`: a different player count, or a
/// player id at some slot that is not `state`'s.
///
/// ## Why this panics rather than degrading, ON THE RENDER PATH
///
/// A panic here takes down the frame, and the frame is what the player is
/// looking at — a harsher blast radius than the same assertion in `gc-sim`.
/// It is still the right call, and the reason is that the failure it catches
/// is IMPOSSIBLE BY CONSTRUCTION UPSTREAM, so reaching it means something
/// this module cannot reason about is already wrong:
///
/// - `gc_sim::combat::new_state` builds `player_ids`/`players` by iterating
///   `state.players` in order, so the correspondence is positional from
///   birth;
/// - `gc_sim::combat_snapshot::copy` re-asserts `player_ids[i] ==
///   state.players[i].id` at every index;
/// - every step and every rollback moves state and combat together —
///   `r#match::step` takes both, `rollback_snapshot_history::restore_simulation`
///   returns both from one ring entry — so they cannot come from different
///   ticks.
///
/// Degrading (returning `None`, or skipping the mismatched slot) would trade
/// a loud stop for the one failure mode here that LOOKS LIKE WORKING
/// SOFTWARE: a bystander staggering while the player who was actually struck
/// runs on untouched. Nothing downstream could detect that, and no player
/// would report it as a bug. Per AGENTS.md §7 this is an invariant, not
/// recoverable input; fail loud.
#[must_use]
pub fn combat_model(state: &MatchState, combat: &CombatMatchState) -> FrameCombatModel {
    assert_eq!(
        combat.players.len(),
        state.players.len(),
        "combat presentation player count mismatch"
    );
    assert_eq!(
        combat.player_ids.len(),
        state.players.len(),
        "combat presentation identity mismatch"
    );
    let players = state
        .players
        .iter()
        .zip(combat.player_ids.iter())
        .zip(combat.players.iter())
        .map(|((player, id), runtime)| {
            assert_eq!(
                id, &player.id,
                "combat presentation player identity mismatch"
            );
            CombatPoseSample {
                phase: runtime.phase,
                forced_state: runtime.forced_state,
                forced_ticks: runtime.forced_ticks,
                // `IMMUNITY_TICKS` is the window's full length, so this is
                // exactly 1.0 the tick immunity starts and ramps linearly to
                // 0.0 as `runtime.immunity_ticks` counts down to it. Divides
                // by a nonzero constant unconditionally: `IMMUNITY_TICKS` is
                // `combat_rules`' own fixed constant, never zero.
                immunity_fraction: runtime.immunity_ticks as f64 / IMMUNITY_TICKS as f64,
                phase_fraction: phase_fraction(runtime),
            }
        })
        .collect();
    FrameCombatModel {
        enabled: true,
        players,
    }
}

/// One drawable frame: the whole interface between the simulation and any
/// renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderFrame {
    /// Exactly [`VERSION`].
    pub version: u32,
    /// Match-constant per-player identity.
    pub roster: RenderFrameRoster,
    /// Pitch geometry.
    pub field: RenderFrameField,
    /// Per-player structure-of-arrays state.
    pub players: RenderFramePlayers,
    /// The ball's per-frame state.
    pub ball: RenderFrameBall,
    /// Who holds the ball.
    pub possession: RenderFramePossession,
    /// What the locally controlled player is doing with the input.
    pub control: RenderFrameControl,
    /// Scoreboard/identity facts.
    pub hud: RenderFrameHud,
    /// This frame's discrete match events, flattened.
    pub events: RenderFrameEvents,
    /// Combat telegraphs, carried unflattened. The one disclosed nested
    /// exception; see [`FrameCombatModel`].
    pub combat: Option<FrameCombatModel>,
}

/// The slice of `@gc/render`'s `CorrectionSmoothingPose` (TS-owned) this
/// module reads: per-player and ball displayed positions, already smoothed.
/// Declared locally for the same reason as [`FrameCombatModel`].
#[derive(Clone, Debug, PartialEq)]
pub struct RenderPose {
    /// Displayed positions, keyed by player id. Sparse: a player absent
    /// here is drawn at its authoritative position. A `Vec` of pairs
    /// rather than a map (ARCHITECTURE.md §3 rule 4); roster size is ~10, so a linear
    /// scan costs nothing.
    pub players: Vec<(String, Vec2)>,
    /// The ball's displayed position.
    pub ball: Vec2,
}

impl RenderPose {
    fn displayed(&self, player_id: &str) -> Option<Vec2> {
        self.players
            .iter()
            .find(|(id, _)| id == player_id)
            .map(|(_, pos)| *pos)
    }
}

/// Inputs the frame cannot derive from [`MatchState`] alone.
#[derive(Clone, Debug, Default)]
pub struct RenderFrameOptions {
    /// Reuse a roster built earlier for this match, instead of rebuilding
    /// it. When absent, [`build`] builds one via [`roster`].
    pub roster: Option<RenderFrameRoster>,
    /// Displayed positions from correction smoothing.
    pub render_pose: Option<RenderPose>,
    /// Frame event batch; defaults to `state.events` when absent.
    pub events: Option<Vec<MatchEvent>>,
    /// Renderer-owned release follow-through window: the ids currently
    /// following through. A `Vec` rather than a map for the same reason as
    /// [`RenderPose::players`].
    pub kick_follow: Option<Vec<String>>,
    /// Renderer-owned dispossession flinch window: the ids of players a
    /// standing-poke tackle recently took the ball from, still inside their
    /// presentation-only reaction beat. Feeds
    /// [`crate::player_pose::OutfieldPoseContext::dispossessed`] the same
    /// way `kick_follow` feeds `.kick_follow` — see that field's doc for why
    /// this lives here rather than in `gc-sim` (#591).
    pub dispossessed: Option<Vec<String>>,
    /// Combat telegraph model for this frame — [`combat_model`] over the
    /// match's live `CombatMatchState`, or `None` for a match that does not
    /// run combat. `None` is what every combat pose's reachability hinges
    /// on: [`build`] only ever offers
    /// [`crate::player_pose::select`] a combat sample when this is `Some`,
    /// so a caller leaving it at its `Default` makes all seven combat poses
    /// unselectable no matter what the simulation underneath is doing
    /// (#441).
    pub combat: Option<FrameCombatModel>,
    /// Tier-2 presentation values this frame is drawn with. `None` means
    /// [`crate::presentation_tunables::shipped`], which is what every shipping
    /// caller wants.
    ///
    /// It is injectable at all so that tier-2 isolation can be *measured*
    /// rather than asserted: `tests/presentation_tunables.rs` builds one frame
    /// with the shipped values and one with every value perturbed, and shows
    /// the rendered output differs while the simulation's boundary-hash
    /// sequence does not. Without this seam that test could only re-run the
    /// same global twice and would pass no matter what the isolation was
    /// doing.
    pub presentation: Option<Registry>,
}

// Pose-timer normalisers and the reticle window are TIER 2 of the tunable
// registry: presentation eases, not simulation durations — they only decide
// how fast a pose relaxes back to neutral, and how long a predicted fall has
// to be before a landing reticle is worth drawing. They are authored in
// `crate::presentation_tunables` (see that module for why tier 2 lives in this
// crate rather than `gc-data`) and read through the registry here, so nothing
// in this file is a raw number the simulation might one day be tempted to
// read.
fn tier2(opts: &RenderFrameOptions, id: &str) -> f64 {
    opts.presentation
        .as_ref()
        .unwrap_or_else(|| crate::presentation_tunables::shipped())
        .value(id)
}

// Loose-ball ballistics for the landing reticle. The solve is presentation
// (where will this cross come down?), so it lives here rather than making
// the simulation compute something it never uses — but it falls at the
// simulation's own gravity, read from `gc_sim::r#match`, so the two can
// never drift apart.
const BALL_GRAVITY: f64 = sim_match::GRAVITY_PX;

// Absent, false and true have to survive the crossing as three distinct
// values.
fn tri_state(value: Option<bool>) -> i64 {
    match value {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

fn eased(timer: f64, ease: f64) -> f64 {
    if timer <= 0.0 {
        return 0.0;
    }
    (timer / ease).min(1.0)
}

fn aerial_ease(opts: &RenderFrameOptions, player: &gc_sim::match_snapshot::MatchPlayer) -> f64 {
    if player.aerial_style == Some(AerialStyle::Bicycle) {
        tier2(opts, "presentation.aerial_ease_bicycle")
    } else if player.aerial_jump > 0.0 {
        tier2(opts, "presentation.aerial_ease_jump")
    } else if matches!(
        player.aerial_style,
        Some(AerialStyle::LegControl) | Some(AerialStyle::ChestControl)
    ) {
        tier2(opts, "presentation.aerial_ease_control")
    } else {
        tier2(opts, "presentation.aerial_ease")
    }
}

// `sim::match::to_transition_team` is private to that module; this mirrors
// it exactly. Both teams' enums are the same two-member shape (README
// wouldn't gain anything from unifying them: `Team` also carries meaning
// nothing to do with the transition window contract).
fn to_transition_team(team: Team) -> TransitionTeam {
    match team {
        Team::Home => TransitionTeam::Home,
        Team::Away => TransitionTeam::Away,
    }
}

/// Match-constant per-player identity. Build once and pass back in as
/// [`RenderFrameOptions::roster`]: nothing in it can change while a match is
/// running.
///
/// TWO IDENTITY SYSTEMS COEXIST HERE, DELIBERATELY (#447). [`identity::for_player`]
/// resolves the OLDER `showcase_player_compatibility` → `species` path, which is
/// what `species_shape`/`species_color`/`names` come from. `presentation_id` and
/// `loadout_id` come straight off `gc_data::players`, the newer authored
/// content, because that is the pair the renderer's character geometry is keyed
/// on and the pair `gc-data/tests/players.rs` enforces the keeper rule over.
/// Reconciling or retiring the species path is explicitly out of #447's scope;
/// it is left working and named here so a follow-up has somewhere to start.
///
/// # Panics
///
/// If a player's pitch presentation identity is missing, or the player is not
/// an authored `gc_data::players` record — both content authoring bugs, not
/// recoverable conditions.
#[must_use]
pub fn roster(state: &MatchState) -> RenderFrameRoster {
    let mut ids = Vec::with_capacity(state.players.len());
    let mut names = Vec::with_capacity(state.players.len());
    let mut presentation_ids = Vec::with_capacity(state.players.len());
    let mut loadout_ids = Vec::with_capacity(state.players.len());
    let mut teams = Vec::with_capacity(state.players.len());
    let mut is_keeper = Vec::with_capacity(state.players.len());
    let mut radius = Vec::with_capacity(state.players.len());
    let mut species_shape = Vec::with_capacity(state.players.len());
    let mut species_color = Vec::with_capacity(state.players.len());

    for player in &state.players {
        let presentation = identity::for_player(&player.id)
            .unwrap_or_else(|| panic!("missing pitch identity for {}", player.id));
        let authored = gc_data::players::get(&player.id)
            .unwrap_or_else(|| panic!("missing authored player record for {}", player.id));
        ids.push(player.id.clone());
        // The authored presentation name, not `MatchPlayer.name`: it is the
        // one a HUD actually shows, and it survives a replay frame's
        // partial copy.
        names.push(presentation.name.to_string());
        presentation_ids.push(authored.presentation_id.to_string());
        loadout_ids.push(authored.loadout_id.map(str::to_string));
        teams.push(player.team);
        is_keeper.push(player.is_keeper);
        radius.push(player.radius);
        species_shape.push(presentation.shape);
        species_color.push(presentation.palette);
    }

    RenderFrameRoster {
        version: VERSION,
        count: state.players.len(),
        ids,
        names,
        presentation_ids,
        loadout_ids,
        teams,
        is_keeper,
        radius,
        species_shape,
        species_color,
    }
}

/// The scoreboard half of the payload. Exposed on its own because a
/// broadcast HUD outlives the frame the pitch is drawing: during a goal
/// replay the pitch shows a past frame while the HUD keeps reporting the
/// live match.
#[must_use]
pub fn hud(state: &MatchState, roster_in: Option<&RenderFrameRoster>) -> RenderFrameHud {
    let built;
    let roster_ref = match roster_in {
        Some(r) => r,
        None => {
            built = roster(state);
            &built
        }
    };
    let controlled = &state.players[(state.controlled - 1) as usize];
    let owner = state.owner.map(|o| &state.players[(o - 1) as usize]);
    RenderFrameHud {
        home_score: state.score.home,
        away_score: state.score.away,
        time_left: state.time_left,
        finished: state.finished,
        possession_team: owner.map(|o| o.team),
        controlled: state.controlled,
        controlled_id: controlled.id.clone(),
        controlled_team: controlled.team,
        controlled_is_keeper: controlled.is_keeper,
        controlled_owns_ball: state.owner == Some(state.controlled),
        controlled_stamina: controlled.sprint_meter.clamp(0.0, 1.0),
        species_shape: roster_ref.species_shape[(state.controlled - 1) as usize],
        species_color: roster_ref.species_color[(state.controlled - 1) as usize],
    }
}

fn build_field(state: &MatchState) -> RenderFrameField {
    RenderFrameField {
        w: state.field.w,
        h: state.field.h,
        crossbar_h: sim_match::CROSSBAR_H,
        penalty_box_depth: sim_match::PENALTY_BOX_DEPTH,
        penalty_box_h: sim_match::PENALTY_BOX_H,
        goal_home: state.goal_home,
        goal_away: state.goal_away,
    }
}

// The counter-press window is what separates a presser shepherding the
// carrier from one hunting it at full speed: `gc_sim::r#match` exempts a
// counter-pressing presser from the contain slowdown and its ball-facing
// lock, so presentation must not claim contain there either. Read once per
// team, not once per player.
fn counterpressing_teams(state: &MatchState) -> ByTeam<bool> {
    let owner_team = state
        .owner
        .map(|o| to_transition_team(state.players[(o - 1) as usize].team));
    ByTeam {
        home: possession_transition::phase(
            &state.transition,
            TransitionTeam::Home,
            owner_team,
            state.transition_windows.get(TransitionTeam::Home),
        ) == TeamPhase::Counterpress,
        away: possession_transition::phase(
            &state.transition,
            TransitionTeam::Away,
            owner_team,
            state.transition_windows.get(TransitionTeam::Away),
        ) == TeamPhase::Counterpress,
    }
}

// A tip is a keeper event that overrides the dive direction for one frame,
// so it resolves here and the renderer never scans the event batch for it.
// Later entries win: `rfind` returns the rightmost match.
fn tip_event_for<'a>(events: &'a [MatchEvent], player_id: &str) -> Option<&'a MatchEvent> {
    events.iter().rfind(|event| {
        event.kind == MatchEventKind::Tip && event.player.as_deref() == Some(player_id)
    })
}

fn slot_of(roster: &RenderFrameRoster, id: &str) -> Option<i64> {
    roster
        .ids
        .iter()
        .position(|candidate| candidate == id)
        .map(|index| (index + 1) as i64)
}

fn build_events(events: &[MatchEvent], roster: &RenderFrameRoster) -> RenderFrameEvents {
    let count = events.len();
    let mut out = RenderFrameEvents {
        count,
        kind: Vec::with_capacity(count),
        x: Vec::with_capacity(count),
        y: Vec::with_capacity(count),
        player: Vec::with_capacity(count),
        slot: Vec::with_capacity(count),
        save_style: Vec::with_capacity(count),
        style: Vec::with_capacity(count),
        outcome: Vec::with_capacity(count),
        difficulty: Vec::with_capacity(count),
        shot_type: Vec::with_capacity(count),
        keeper_state: Vec::with_capacity(count),
        keeper_depth: Vec::with_capacity(count),
        jumping: Vec::with_capacity(count),
        on_target: Vec::with_capacity(count),
    };
    for event in events {
        out.kind.push(event.kind);
        out.x.push(event.x);
        out.y.push(event.y);
        out.slot
            .push(event.player.as_deref().and_then(|id| slot_of(roster, id)));
        out.player.push(event.player.clone());
        out.save_style.push(event.save_style);
        out.style.push(event.style);
        out.outcome.push(event.outcome);
        out.jumping.push(tri_state(event.jumping));
        out.difficulty.push(event.difficulty);
        out.shot_type.push(event.shot_type);
        out.keeper_state.push(event.keeper_state);
        out.keeper_depth.push(event.keeper_depth);
        out.on_target.push(tri_state(event.on_target));
    }
    out
}

// Where a lofted, loose ball will come down. Only for a genuinely airborne
// ball (a cross or a lob), never a grounded pass, and only when it lands on
// the pitch inside a readable window.
fn landing_point(
    state: &MatchState,
    opts: &RenderFrameOptions,
    ball_x: f64,
    ball_y: f64,
) -> (Option<f64>, Option<f64>) {
    let height = state.ball_z;
    if state.owner.is_some() || height <= tier2(opts, "presentation.reticle_min_height") {
        return (None, None);
    }
    let vz = state.ball_vz;
    let fall = (vz + (vz * vz + 2.0 * BALL_GRAVITY * height).sqrt()) / BALL_GRAVITY;
    if fall <= tier2(opts, "presentation.reticle_min_time")
        || fall >= tier2(opts, "presentation.reticle_max_time")
    {
        return (None, None);
    }
    let x = ball_x + state.ball_vel.x * fall;
    let y = ball_y + state.ball_vel.y * fall;
    if x <= 0.0 || x >= state.field.w || y <= 0.0 || y >= state.field.h {
        return (None, None);
    }
    (Some(x), Some(y))
}

// A KEEPER IS NOT DRAWN FACING ITS OWN DIVE (#449).
//
// `MatchPlayer.facing` serves two jobs that had never been separated: it is
// the direction the body is DRAWN pointing, and it is the aim the simulation
// reads to decide who receives a keeper's throw (`match.rs`'s `keeper_throw`
// / `select_throw_target`). `move_offball_keeper` points it along the dive
// while `dive_timer` runs, which is defensible for the second job and wrong
// for the first — so the split happens here, at the sim-to-renderer boundary,
// exactly where AGENTS.md §2 puts presentation-derived state. The simulation
// keeps its own value untouched; nothing downstream of `match.rs` changes.
//
// WHY IT IS NOT MERELY UNTIDY. `launch_dive` builds `dive_target` as
// `Vec2::new(keeper.pos.x, y_cross)`, so `to_cross.x` is exactly `0.0`
// (IEEE-754 `a - a`) and `Vec2::normalized` keeps it exactly `0.0`: a
// keeper's `dive_dir` is ALWAYS `(0, ±1)`, and the direction the dive branch
// writes to `facing` is the same lateral unit vector. The rig decides which
// side a save rolls to from the 2D cross product of those two
// (`rig3d/action_pose.ts`'s `lateralSign`), and two exactly-parallel vectors
// have a zero cross product — so `save()` returned `null` and the ENTIRE
// overlay, roll and travel together, was skipped for the large majority of
// save frames. Worse, it was not skipped consistently: `apply_locomotion`
// leaves a velocity-derived `facing` for the one tick before the dive branch
// overwrites it, so a save that opened with a lean lost the whole overlay
// partway through the episode.
//
// No count appears in this comment on purpose. The impact tally came from one
// harness session, and nothing committed to this tree re-derives it, so a
// number here would read as a measured fact that no reader can check. #449 and
// PR #452 carry the figures, dated and with the method stated, which is where
// a point-in-time measurement belongs.
//
// WHY THE GOAL-LINE NORMAL rather than the facing latched at launch. The
// latched value is whatever locomotion last left, and it can point back into
// the keeper's own goal — latching that yaws the drawn keeper backwards,
// which is a different wrong picture rather than a fix. A keeper faces up the
// pitch. Taking it from the defended goal's own rect rather than from the team
// enum means a side swap carries it.
//
// WHY ALL THREE WINDOWS. `lateralSign` is reached from exactly three states,
// and the degeneracy is one defect across them, not three:
//
//   * the dive itself (`dive_timer > 0`) — the five save poses;
//   * its recovery (`keeper_get_up_timer > 0`) — `keeper_get_up` reads the
//     same `lateralSign` with the same `dive_dir` (`dive_dir` is NOT cleared
//     when `dive_timer` expires), and the keeper is on the floor with
//     `run_vel` at zero, so `facing` still holds the dive-parallel value the
//     dive branch last wrote, and the recovery loses its lean the same way;
//   * a tip (`keeper_tip`), whose `dive_dir` THIS FUNCTION'S CALLER
//     synthesises as `(0, ±1)` from the tip target while `dive_timer` is
//     already zero. Pairing a synthesised lateral direction with a raw
//     simulation facing is the same design error one step removed, and it
//     does land in play rather than only in principle.
//
// Together they give the invariant
// `frame_facing_never_tracks_dive_dir_while_a_keeper_leans_along_it` pins: whenever
// the frame selects a pose that reads `lateralSign`, the published facing is
// the goal-line normal, so it can never be parallel to the published
// `dive_dir`. The only way a lean is still skipped after this is `dive_dir`
// itself being `(0, 0)`, which `launch_dive` leaves when the correction is
// under a pixel — a separate simulation-side defect, filed separately, and the
// reason this does not reach every save frame.
//
// WHY THIS OVERRIDES `facing_x`/`facing_y` RATHER THAN ADDING
// `drawn_facing_x`/`drawn_facing_y`. Redefining a field inside a payload
// AGENTS.md §2 calls versioned for a future renderer is not free, and the
// separate field was considered and declined for three reasons. (1) The
// consumer audit found no reader that treats the FRAME's `facing` as
// simulation truth: rollback snapshots and both replay paths read
// `MatchPlayer.facing` off the sim struct directly, `gc-netcode` has zero
// references to the frame field, and the RL observation encoders read the raw
// sim struct too. (2) It is a stateless per-tick derivation from fields
// `render/` already reads — categorically unlike the stateful presentation
// state (gait, lean, correction smoothing) §2 requires be passed in as an
// explicit input, which is why it may live here at all. (3) A second field
// would widen the wire, and the versioned payload it widens, for exactly one
// consumer. If a reader ever does need the raw simulation aim per frame, that
// is the moment to add the field — and this note is the argument to revisit.
//
// The frame's `facing_x`/`facing_y` has exactly two other consumers and
// neither is harmed: `pitch.ts` (the draw path this exists for) and
// `screens/match.ts`'s `onlineState`, which passes it through
// `online_match.ts` to `combat.model` as a telegraph direction — inert for a
// keeper, and asserted so rather than assumed: `combat_snapshot.rs` refuses a
// keeper a combat loadout and `validate_player` refuses a family without one,
// so a keeper's `family_id` is `None` and `telegraphKind` returns `undefined`.
// `onlineState` carries a comment pointing back here.
//
// PRECONDITION: NOTHING BUT A KEEPER CARRIES A `dive_timer`.
//
// This function never tests `is_keeper`, and that is a decision rather than an
// oversight — the test would be dead code today. In `gc-sim`'s `match.rs`,
// `dive_timer` is set to a nonzero value in exactly one place, `launch_dive`,
// which has exactly two call sites: one inside the keeper save path indexed by
// `keeper_idx`, and one gated on `dive_delay > 0.0`, whose only nonzero
// assignment is `s.players[ki].dive_delay` inside that same keeper save path.
// `keeper_get_up_timer` is armed in one place too — the dive-end transition,
// which a player can only reach by having dived. So both windows imply
// `is_keeper` by construction, and a guard here would be a branch that never
// takes its false arm.
//
// WHAT WOULD BREAK IF THAT CHANGED. Give an outfield player a dive and this
// override reorients it too: it would be drawn facing up the pitch for the
// length of that dive and its recovery, which is right for a keeper defending
// a goal line and wrong for anyone else. Note that bolting an `is_keeper`
// guard on at that point would merely restore the ORIGINAL degenerate facing
// for outfield dives, so the fix then is a real decision about what an
// outfield dive should face — not a fall-through.
//
// The precondition is pinned rather than assumed, by
// `only_a_keeper_ever_carries_a_dive_timer` in `gc-sim`'s own
// `tests/match.rs`: it sweeps stepped matches and goes red on the first tick
// an outfield player holds either timer. It lives with the simulation it
// constrains, next to
// `launch_dive`, so the person editing the dive logic meets it — this pointer
// is the other half of that link. A debug assertion would be
// WRONG in its place: a hand-built fixture may legitimately put the field on a
// non-keeper — `normalises_pose_timers_so_no_renderer_re_derives_a_duration`
// does exactly that — and such a player does receive the override, which is
// harmless in a fixture and is not a reachable simulation state.
fn drawn_facing(state: &MatchState, player: &MatchPlayer, tipping: bool) -> (f64, f64) {
    if player.dive_timer <= 0.0 && player.keeper_get_up_timer <= 0.0 && !tipping {
        return (player.facing.x, player.facing.y);
    }
    let goal = if player.team == Team::Home {
        state.goal_home
    } else {
        state.goal_away
    };
    // Into the field of play, away from the goal this keeper defends.
    let inward = if goal.x + goal.w / 2.0 < state.field.w / 2.0 {
        1.0
    } else {
        -1.0
    };
    (inward, 0.0)
}

/// Turn one [`MatchState`] into one drawable frame. Pure: it reads the state
/// and allocates a new payload, and never mutates anything it was handed.
#[must_use]
pub fn build(state: &MatchState, opts: &RenderFrameOptions) -> RenderFrame {
    let built_roster;
    let roster_ref: &RenderFrameRoster = match &opts.roster {
        Some(r) => r,
        None => {
            built_roster = roster(state);
            &built_roster
        }
    };
    let render_pose = opts.render_pose.as_ref();
    let kick_follow = opts.kick_follow.as_ref();
    let dispossessed = opts.dispossessed.as_ref();
    let events_owned;
    let events: &[MatchEvent] = match &opts.events {
        Some(e) => e,
        None => {
            events_owned = state.events.clone();
            &events_owned
        }
    };
    let counterpressing = counterpressing_teams(state);
    let now = -state.time_left;

    // Held in the HANDS only: a keeper with a back-pass at its feet
    // dribbles a ground ball like anyone else.
    let owner = state.owner.map(|o| &state.players[(o - 1) as usize]);
    let keeper_holds = owner.is_some_and(|o| o.is_keeper && !o.feet_ball);

    let count = roster_ref.count;
    let mut players = RenderFramePlayers {
        count,
        x: Vec::with_capacity(count),
        y: Vec::with_capacity(count),
        facing_x: Vec::with_capacity(count),
        facing_y: Vec::with_capacity(count),
        speed: Vec::with_capacity(count),
        pose_id: Vec::with_capacity(count),
        pose_priority: Vec::with_capacity(count),
        pose_source: Vec::with_capacity(count),
        controlled: Vec::with_capacity(count),
        dashing: Vec::with_capacity(count),
        holding: Vec::with_capacity(count),
        dive: Vec::with_capacity(count),
        dive_dir_x: Vec::with_capacity(count),
        dive_dir_y: Vec::with_capacity(count),
        grab: Vec::with_capacity(count),
        throw: Vec::with_capacity(count),
        windup: Vec::with_capacity(count),
        aerial: Vec::with_capacity(count),
        aerial_jump: Vec::with_capacity(count),
        aerial_style: Vec::with_capacity(count),
        aerial_outcome: Vec::with_capacity(count),
        phase_fraction: Vec::with_capacity(count),
    };

    for zero_based in 0..count {
        let index = (zero_based + 1) as i64;
        let player = &state.players[zero_based];

        // Displayed position: correction smoothing has already decided
        // where this player is shown. Every distance the SIMULATION
        // reasons about (smother range, tip direction) keeps reading the
        // authoritative pos.
        let displayed = render_pose
            .and_then(|pose| pose.displayed(&player.id))
            .unwrap_or(player.pos);

        let tip = tip_event_for(events, &player.id);
        let (dive_dir_x, dive_dir_y) = if let Some(tip) = tip {
            (
                0.0,
                if (tip.y - player.pos.y) >= 0.0 {
                    1.0
                } else {
                    -1.0
                },
            )
        } else {
            (player.dive_dir.x, player.dive_dir.y)
        };

        // Displayed facing: a keeper leaning along a dive is DRAWN facing up
        // the pitch, whatever the simulation's `facing` says (#449). See
        // [`drawn_facing`].
        let (facing_x, facing_y) = drawn_facing(state, player, tip.is_some());

        let keeper_context = player.is_keeper.then(|| KeeperPoseContext {
            near_ball: keeper::in_smother_range(player.pos.dist(state.ball)),
            shuffling: player.keeper_state == KeeperBehaviorState::Base
                && player.run_vel.y.abs() > 0.0,
            tip: tip.is_some(),
        });

        // Outfield pose inputs. The press mode is team-owned simulation
        // state, the telegraph window is measured against the match
        // clock, and the follow-through is the render-owned release
        // window supplied by the caller. Both teams read from the same
        // three sources.
        let outfield_context = (!player.is_keeper).then(|| {
            let press = state.outfield_press.get(player.team);
            OutfieldPoseContext {
                now: Some(now),
                containing: press.mode == StablePressMode::Contain
                    && press.presser_index == Some(index as u32)
                    && !counterpressing.get(player.team),
                kick_follow: kick_follow.is_some_and(|ids| ids.contains(&player.id)),
                dispossessed: dispossessed.is_some_and(|ids| ids.contains(&player.id)),
            }
        });

        let combat_sample = opts
            .combat
            .as_ref()
            .and_then(|combat| combat.players.get(zero_based));
        let pose = player_pose::select(
            player,
            combat_sample,
            keeper_context.as_ref(),
            outfield_context.as_ref(),
        );

        players.x.push(displayed.x);
        players.y.push(displayed.y);
        players.facing_x.push(facing_x);
        players.facing_y.push(facing_y);
        players.speed.push(player.run_vel.length());
        players.pose_id.push(pose.id);
        players.pose_priority.push(pose.priority);
        players.pose_source.push(pose.source);
        players.controlled.push(index == state.controlled);
        players.dashing.push(player.slide_timer > 0.0);
        players
            .holding
            .push(Some(index) == state.owner && player.is_keeper && !player.feet_ball);
        players.dive.push(eased(
            player.dive_timer,
            tier2(opts, "presentation.dive_ease"),
        ));
        players.dive_dir_x.push(dive_dir_x);
        players.dive_dir_y.push(dive_dir_y);
        players.grab.push(eased(
            player.grab_timer,
            tier2(opts, "presentation.grab_ease"),
        ));
        players.throw.push(eased(
            player.throw_timer,
            tier2(opts, "presentation.throw_ease"),
        ));
        // The wind-up back-swing is deliberately unclamped: 0 = no windup,
        // 1 = just committed, and a long charge reads above 1.
        players.windup.push(if player.windup_timer > 0.0 {
            player.windup_timer / tier2(opts, "presentation.windup_ease")
        } else {
            0.0
        });
        players
            .aerial
            .push(eased(player.aerial_timer, aerial_ease(opts, player)));
        players.aerial_jump.push(player.aerial_jump);
        players.aerial_style.push(player.aerial_style);
        players.aerial_outcome.push(player.aerial_outcome);
        // A match without combat reports 0.0 everywhere — "nothing to
        // report" and "at the start of the phase" deliberately share the
        // value; see the field's doc.
        players
            .phase_fraction
            .push(combat_sample.map_or(0.0, |sample| sample.phase_fraction));
    }

    let ball_point = render_pose.map_or(state.ball, |pose| pose.ball);
    let (landing_x, landing_y) = landing_point(state, opts, ball_point.x, ball_point.y);

    let controlled = &state.players[(state.controlled - 1) as usize];
    let (charge_kind, charge) = if controlled.charge > 0.02 {
        (Some(RenderChargeKind::Shot), controlled.charge)
    } else if controlled.pass_charge > 0.02 {
        (Some(RenderChargeKind::Pass), controlled.pass_charge)
    } else {
        (None, 0.0)
    };

    let hud_section = hud(state, Some(roster_ref));
    let events_section = build_events(events, roster_ref);
    let roster_out = roster_ref.clone();

    RenderFrame {
        version: VERSION,
        roster: roster_out,
        field: build_field(state),
        players,
        ball: RenderFrameBall {
            x: ball_point.x,
            y: ball_point.y,
            z: state.ball_z,
            vx: state.ball_vel.x,
            vy: state.ball_vel.y,
            vz: state.ball_vz,
            visible: !keeper_holds,
            landing_x,
            landing_y,
        },
        possession: RenderFramePossession {
            owner: state.owner,
            owner_team: owner.map(|o| o.team),
            keeper_holds,
        },
        control: RenderFrameControl {
            controlled: state.controlled,
            pass_target: controlled.pass_target,
            charge_kind,
            charge,
        },
        hud: hud_section,
        events: events_section,
        combat: opts.combat.clone(),
    }
}
