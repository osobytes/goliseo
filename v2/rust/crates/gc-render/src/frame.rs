//! Port of `render/frame.lua`.
//!
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
//!    smoothed on-screen speed, the correction smoothing state machine and the
//!    release follow-through window are NOT simulation and are not derived
//!    here. They feed in as explicit inputs ([`RenderFrameOptions::render_pose`],
//!    [`RenderFrameOptions::kick_follow`]) because the frame must report the
//!    positions and poses actually shown; their state machines stay where
//!    they are (`@gc/render`, TypeScript).
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
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::keeper::{self, KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{ByTeam, MatchEvent, MatchEventKind, MatchState, Rect, Team};
use gc_sim::outfield_press::StablePressMode;
use gc_sim::possession_transition::{self, TransitionTeam};

use crate::identity;
use crate::player_pose::{
    self, CombatPoseSample, KeeperPoseContext, OutfieldPoseContext, PlayerPoseId, PlayerPoseSource,
};

/// This protocol's version. Bump whenever [`RenderFrame`]'s shape changes.
pub const VERSION: u32 = 1;

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

/// The slice of `@gc/presentation`'s `CombatPresentationModel`
/// (`game/presentation/combat.lua`, TS-owned) this module reads.
/// `render/frame.lua` carries the whole model through the frame unflattened
/// (`RenderFrame.combat`) and reads only `combat.players[index]`'s
/// phase/forced-state fields, to hand [`crate::player_pose::select`] its
/// `combat` sample; [`crate::frame_buffer`] never encodes any of it (see that
/// module's own doc, "WHAT IS NOT CARRIED").
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
/// - The POSE-SELECTION half — `phase`, `forced_state`, `forced_ticks` — is
///   not TypeScript-owned at all. It is already native Rust simulation
///   state, stepped every tick, on this side of the wall:
///   `gc_sim::combat_snapshot::CombatPlayerState`. [`combat_model`] below
///   adapts it in-process, with no boundary crossing whatsoever.
///
/// So this type stays narrow because pose selection only ever needed three
/// fields, not because the data was out of reach.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameCombatModel {
    /// Whether combat presentation is enabled for this match. A model is
    /// only ever built for a match that runs combat (see [`combat_model`]),
    /// so a present model always reports `true`; a match without combat is
    /// represented by [`RenderFrameOptions::combat`] being `None`, exactly
    /// as it was before a model was ever built.
    pub enabled: bool,
    /// One entry per roster slot; exactly the shape
    /// [`crate::player_pose::CombatPoseSample`] reads.
    pub players: Vec<CombatPoseSample>,
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
/// Mirrors `game/presentation/combat.lua`'s `presentation.model` for the
/// three fields it shares with it, including that function's identity
/// assertions.
///
/// # Panics
///
/// If `combat` does not describe `state`: a different player count, or a
/// player id at some slot that is not `state`'s. Both are programmer errors
/// (a caller pairing a combat companion with the wrong match), and both
/// would otherwise show one player's combat pose on another player's body.
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

/// The slice of `@gc/render`'s `CorrectionSmoothingPose`
/// (`game/render/correction_smoothing.lua`, TS-owned) this module reads:
/// per-player and ball displayed positions, already smoothed. Declared
/// locally for the same reason as [`FrameCombatModel`].
#[derive(Clone, Debug, PartialEq)]
pub struct RenderPose {
    /// Displayed positions, keyed by player id. Sparse: a player absent
    /// here is drawn at its authoritative position. A `Vec` of pairs
    /// rather than a map (README rule 4); roster size is ~10, so a linear
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
    /// Combat telegraph model for this frame — [`combat_model`] over the
    /// match's live `CombatMatchState`, or `None` for a match that does not
    /// run combat. `None` is what every combat pose's reachability hinges
    /// on: [`build`] only ever offers
    /// [`crate::player_pose::select`] a combat sample when this is `Some`,
    /// so a caller leaving it at its `Default` makes all seven combat poses
    /// unselectable no matter what the simulation underneath is doing
    /// (#441).
    pub combat: Option<FrameCombatModel>,
}

// Pose-timer normalisers. These are presentation eases, not simulation
// durations: they only decide how fast a pose relaxes back to neutral.
const DIVE_EASE: f64 = 0.3;
const GRAB_EASE: f64 = 0.25;
const THROW_EASE: f64 = 0.25;
const WINDUP_EASE: f64 = 0.15;
const AERIAL_EASE: f64 = 0.22;
const AERIAL_EASE_BICYCLE: f64 = 0.6;
const AERIAL_EASE_JUMP: f64 = 0.35;
const AERIAL_EASE_CONTROL: f64 = 0.18;

// Loose-ball ballistics for the landing reticle. The solve is presentation
// (where will this cross come down?), so it lives here rather than making
// the simulation compute something it never uses — but it falls at the
// simulation's own gravity, read from `gc_sim::r#match`, so the two can
// never drift apart.
const BALL_GRAVITY: f64 = sim_match::GRAVITY_PX;
const RETICLE_MIN_HEIGHT: f64 = 20.0;
const RETICLE_MIN_TIME: f64 = 0.05;
const RETICLE_MAX_TIME: f64 = 3.0;

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

fn aerial_ease(player: &gc_sim::match_snapshot::MatchPlayer) -> f64 {
    if player.aerial_style == Some(AerialStyle::Bicycle) {
        AERIAL_EASE_BICYCLE
    } else if player.aerial_jump > 0.0 {
        AERIAL_EASE_JUMP
    } else if matches!(
        player.aerial_style,
        Some(AerialStyle::LegControl) | Some(AerialStyle::ChestControl)
    ) {
        AERIAL_EASE_CONTROL
    } else {
        AERIAL_EASE
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
/// # Panics
///
/// If a player's pitch presentation identity is missing — a content
/// authoring bug, not a recoverable condition.
#[must_use]
pub fn roster(state: &MatchState) -> RenderFrameRoster {
    let mut ids = Vec::with_capacity(state.players.len());
    let mut names = Vec::with_capacity(state.players.len());
    let mut teams = Vec::with_capacity(state.players.len());
    let mut is_keeper = Vec::with_capacity(state.players.len());
    let mut radius = Vec::with_capacity(state.players.len());
    let mut species_shape = Vec::with_capacity(state.players.len());
    let mut species_color = Vec::with_capacity(state.players.len());

    for player in &state.players {
        let presentation = identity::for_player(&player.id)
            .unwrap_or_else(|| panic!("missing pitch identity for {}", player.id));
        ids.push(player.id.clone());
        // The authored presentation name, not `MatchPlayer.name`: it is the
        // one a HUD actually shows, and it survives a replay frame's
        // partial copy.
        names.push(presentation.name.to_string());
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
// Later entries win, matching Lua's for-loop overwrite.
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
fn landing_point(state: &MatchState, ball_x: f64, ball_y: f64) -> (Option<f64>, Option<f64>) {
    let height = state.ball_z;
    if state.owner.is_some() || height <= RETICLE_MIN_HEIGHT {
        return (None, None);
    }
    let vz = state.ball_vz;
    let fall = (vz + (vz * vz + 2.0 * BALL_GRAVITY * height).sqrt()) / BALL_GRAVITY;
    if fall <= RETICLE_MIN_TIME || fall >= RETICLE_MAX_TIME {
        return (None, None);
    }
    let x = ball_x + state.ball_vel.x * fall;
    let y = ball_y + state.ball_vel.y * fall;
    if x <= 0.0 || x >= state.field.w || y <= 0.0 || y >= state.field.h {
        return (None, None);
    }
    (Some(x), Some(y))
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
        players.facing_x.push(player.facing.x);
        players.facing_y.push(player.facing.y);
        players.speed.push(player.run_vel.length());
        players.pose_id.push(pose.id);
        players.pose_priority.push(pose.priority);
        players.pose_source.push(pose.source);
        players.controlled.push(index == state.controlled);
        players.dashing.push(player.slide_timer > 0.0);
        players
            .holding
            .push(Some(index) == state.owner && player.is_keeper && !player.feet_ball);
        players.dive.push(eased(player.dive_timer, DIVE_EASE));
        players.dive_dir_x.push(dive_dir_x);
        players.dive_dir_y.push(dive_dir_y);
        players.grab.push(eased(player.grab_timer, GRAB_EASE));
        players.throw.push(eased(player.throw_timer, THROW_EASE));
        // The wind-up back-swing is deliberately unclamped: 0 = no windup,
        // 1 = just committed, and a long charge reads above 1.
        players.windup.push(if player.windup_timer > 0.0 {
            player.windup_timer / WINDUP_EASE
        } else {
            0.0
        });
        players
            .aerial
            .push(eased(player.aerial_timer, aerial_ease(player)));
        players.aerial_jump.push(player.aerial_jump);
        players.aerial_style.push(player.aerial_style);
        players.aerial_outcome.push(player.aerial_outcome);
    }

    let ball_point = render_pose.map_or(state.ball, |pose| pose.ball);
    let (landing_x, landing_y) = landing_point(state, ball_point.x, ball_point.y);

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
