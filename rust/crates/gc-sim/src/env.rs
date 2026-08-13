//! The pure Rust learning-environment core.
//!
//! `reset`/`observe`/`step` over the canonical fixed tick, built entirely
//! from the seams that already exist: [`crate::fixed_clock`] owns tick
//! numbering, [`crate::slot_input`] materializes the complete effective
//! `InputFrame` for all eight slots, [`crate::r#match`] is the only
//! authority, [`crate::match_snapshot`] produces boundary hashes,
//! [`crate::metrics`] collects evaluation diagnostics, and
//! [`crate::input_tape`]/[`crate::replay`] verify the recording afterwards.
//!
//! This module is pure: no I/O, no networking, no learning framework. Any
//! transport, batching, or serialization layer lives outside this crate and
//! can only drive this API, so it can never change simulation authority.
//!
//! Reward and observation policy live in [`crate::env_reward`] and
//! [`crate::env_observation`]; the design record is
//! `docs/design/learning_environment.md`.
//!
//! ## `EnvInstance` fields are `pub`
//!
//! `tests/env.rs` and `tests/env_leakage.rs` both reach directly into
//! `EnvInstance`'s internals (`instance.state.rng`,
//! `instance.state.slot_players[3] = None`, `instance.combat`), so
//! ARCHITECTURE.md §3 rule 6 ("everything a test touches is `pub`") applies: every field is
//! genuinely `pub` — [`EnvInstance::state`], [`EnvInstance::combat`],
//! [`EnvInstance::producer`], [`EnvInstance::metrics`],
//! [`EnvInstance::clock`], [`EnvInstance::initial`],
//! [`EnvInstance::tape_frames`], [`EnvInstance::final_metrics`],
//! [`EnvInstance::fault`]. A real caller outside the test suite is expected
//! to treat these as read-only bookkeeping, not to mutate them.
//!
//! ## Three duplicate `EnvObservationProfile`s meet here
//!
//! [`crate::env_config`], [`crate::env_action`], and [`crate::env_observation`]
//! each declare their own `EnvObservationProfile`, keeping each module's
//! dependency surface minimal. This module is the one place all three
//! meet — a config's profile drives an observation build, whose profile then
//! drives an action mask — so it owns the small conversions between them
//! ([`to_observation_profile`], [`to_action_profile`]).
//!
//! ## `metrics::MetricsMatchView` adapter
//!
//! [`crate::metrics`] reads a narrow [`crate::metrics::MetricsMatchView`]
//! rather than the real `MatchState`. [`crate::headless`] already owns one
//! `MatchState -> MetricsMatchView` adapter for its own call site; this
//! module drives `metrics::observe`/`metrics::finish` from a different call
//! site (inside the per-tick `fixed_clock::step` closure, not a headless
//! batch loop) and needs its own copy of that adapter for the same reason
//! `headless.rs`'s module doc gives for not sharing one: the adapter isn't
//! `pub` there, and duplicating ~20 lines of field mapping is cheaper than
//! inventing a shared adapter module neither call site owns.
//!
//! ## Simulation faults become `catch_unwind`, not `Result`
//!
//! Unlike [`crate::input_tape`] and [`crate::replay`] (which validate
//! *external input* and are `Result`-returning throughout, per those
//! modules' own doc comments), [`step`] wraps [`crate::r#match::step`] and
//! [`crate::slot_input::materialize`] themselves — functions that `assert!`/
//! `.expect()` on genuine simulation invariants (a non-canonical tick, an
//! incomplete slot mapping). The intent: an invariant violation anywhere
//! between materialization and the boundary is a fault the caller must be
//! able to reproduce, not a crash that takes the trainer down with it — i.e.
//! this is deliberately catching a *bug*, not validating untrusted input, so
//! there is no `Result` to thread through those functions instead (they are
//! not this module's to change regardless).
//!
//! [`std::panic::catch_unwind`] is what [`step`] uses to catch it. But this
//! workspace's `[profile.release]` sets `panic = "abort"` (`rust/Cargo.toml`),
//! under which `catch_unwind` does not run — a panic aborts the process.
//! Under the default `dev`/`test` profile (unwind enabled, no override in
//! `Cargo.toml`), `catch_unwind` works as intended and every "surfaces a
//! simulation fault as a reproducible diagnostic" assertion in
//! `tests/env.rs` holds under `cargo test`. In a release build a simulation
//! fault aborts the process instead of returning [`EnvErrorCode::SimFault`]
//! — a known, pre-existing limitation of this workspace's panic strategy
//! (see `input_tape.rs`'s module doc for the same point made about a
//! different boundary), not something introduced or fixable by this module.

use crate::combat;
use crate::combat_identity;
use crate::combat_snapshot::{
    CombatContactResult as CombatSnapshotContactResult, CombatMatchState,
};
use crate::env_action;
use crate::env_config;
use crate::env_observation;
use crate::env_reward;
use crate::fixed_clock::{self, FixedClockState};
use crate::input_frame::{self, InputFrame};
use crate::input_tape::{self, InputTape, InputTapeIdentity};
use crate::r#match as sim_match;
use crate::match_snapshot::{self, MatchEvent, MatchSnapshot, MatchState, Team as StateTeam};
use crate::metrics::{self, MetricsCollector, MetricsMatchView, MetricsPlayerView};
use crate::slot_input::{self, MatchSlotSource, MatchSlotSourceKind, SlotInputProducerState};
use crate::tuning::Tuning;
use indexmap::IndexMap;

/// The env module version.
pub const VERSION: i64 = 1;
/// The manifest schema version.
pub const MANIFEST_VERSION: i64 = 1;
/// The step-result schema version.
pub const STEP_VERSION: i64 = 1;

const DT: f64 = fixed_clock::TICK_SECONDS;

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// Failure reasons an env operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvErrorCode {
    /// The data violates a structural or type invariant.
    Malformed,
    /// The config names a version this build does not support.
    UnsupportedVersion,
    /// The config names authored content that does not exist.
    UnknownContent,
    /// A required, non-empty selection was empty.
    EmptySelection,
    /// The observation profile and controlled-slot count disagree.
    ProfileMismatch,
    /// A tape-driven slot has no recorded frames.
    MissingTape,
    /// A tape-driven slot ran out of recorded rows.
    TapeExhausted,
    /// The episode has already ended.
    EpisodeOver,
    /// An action named a slot that is not a policy slot.
    UnknownSlot,
    /// A policy slot had no action this step.
    MissingSlot,
    /// An action was well-formed but illegal (out of range, or refused by a
    /// mask).
    IllegalAction,
    /// An invariant violation inside the simulation itself — see the module
    /// doc's `catch_unwind` section.
    SimFault,
    /// The environment already faulted and cannot continue.
    Faulted,
}

/// An expected, recoverable env failure (ARCHITECTURE.md §3 rule 5).
#[derive(Clone, Debug, PartialEq)]
pub struct EnvError {
    /// Machine-readable failure reason.
    pub code: EnvErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl EnvError {
    fn new(code: EnvErrorCode, message: impl Into<String>) -> Self {
        EnvError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EnvError {}

/// Result alias for fallible env operations.
pub type Result<T> = std::result::Result<T, EnvError>;

fn failure<T>(code: EnvErrorCode, message: impl Into<String>) -> Result<T> {
    Err(EnvError::new(code, message))
}

fn convert_config_error(e: env_config::EnvConfigError) -> EnvError {
    let code = match e.code {
        env_config::EnvConfigErrorCode::Malformed => EnvErrorCode::Malformed,
        env_config::EnvConfigErrorCode::UnsupportedVersion => EnvErrorCode::UnsupportedVersion,
        env_config::EnvConfigErrorCode::UnknownContent => EnvErrorCode::UnknownContent,
        env_config::EnvConfigErrorCode::EmptySelection => EnvErrorCode::EmptySelection,
        env_config::EnvConfigErrorCode::ProfileMismatch => EnvErrorCode::ProfileMismatch,
    };
    EnvError::new(code, e.message)
}

fn convert_action_error(e: env_action::EnvActionError) -> EnvError {
    let code = match e.code {
        env_action::EnvActionErrorCode::Malformed => EnvErrorCode::Malformed,
        env_action::EnvActionErrorCode::MoveOutOfRange => EnvErrorCode::IllegalAction,
        env_action::EnvActionErrorCode::UnknownHeldAction => EnvErrorCode::IllegalAction,
        env_action::EnvActionErrorCode::UnknownEdgeAction => EnvErrorCode::IllegalAction,
        env_action::EnvActionErrorCode::UnavailableAction => EnvErrorCode::IllegalAction,
    };
    EnvError::new(code, e.message)
}

// ---------------------------------------------------------------------------
// Reference fixtures.
// ---------------------------------------------------------------------------

/// A declared synthetic reference environment id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceFixture {
    /// Disables the combat companion entirely.
    SoccerOnly,
    /// Enables combat on a fixture whose four outfield loadouts per side
    /// cover unarmed, guard, light_melee, and ranged.
    CombatAllFamilies,
}

fn reference_fixture_from_name(id: &str) -> Option<ReferenceFixture> {
    match id {
        "soccer_only" => Some(ReferenceFixture::SoccerOnly),
        "combat_all_families" => Some(ReferenceFixture::CombatAllFamilies),
        _ => None,
    }
}

/// Overrides [`reference_config`] applies on top of its declared defaults.
/// A narrow, typed struct rather than an untyped bag of "any field, any
/// value": every override any test actually uses is one of these four
/// scalar fields. A future caller needing a different field can extend this
/// struct; that is a smaller, more honest change than widening this to
/// accept an arbitrary field/value pair.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReferenceConfigOverrides {
    /// Overrides the fixture's default seed.
    pub seed: Option<i64>,
    /// Overrides the fixture's default duration, in seconds.
    pub duration: Option<f64>,
    /// Overrides the fixture's default goal cap.
    pub max_goals: Option<i64>,
    /// Overrides the fixture's default episode tick budget.
    pub max_episode_ticks: Option<i64>,
}

/// Synthetic reference environments. `soccer_only` disables the combat
/// companion entirely; `combat_all_families` enables it on a fixture whose
/// four outfield loadouts per side cover unarmed, guard, light_melee, and
/// ranged.
pub fn reference_config(
    id: &str,
    overrides: Option<ReferenceConfigOverrides>,
) -> Result<env_config::RawEnvConfig> {
    let Some(fixture) = reference_fixture_from_name(id) else {
        return failure(
            EnvErrorCode::UnknownContent,
            format!("unknown reference fixture: {id}"),
        );
    };
    let overrides = overrides.unwrap_or_default();
    let combat = fixture == ReferenceFixture::CombatAllFamilies;
    let config = env_config::RawEnvConfig {
        has_unknown_field: false,
        version: Some(env_config::VERSION),
        build: Some(format!("reference-{id}")),
        source: None,
        content: Some(env_config::DEFAULT_CONTENT.to_string()),
        fixture: None,
        seed: Some(overrides.seed.unwrap_or(1) as f64),
        duration: Some(overrides.duration.unwrap_or(4.0)),
        max_goals: Some(overrides.max_goals.unwrap_or(sim_match::NO_GOAL_LIMIT) as f64),
        field: Some(env_config::RawFieldSize {
            w: Some(env_config::DEFAULT_FIELD.w),
            h: Some(env_config::DEFAULT_FIELD.h),
            has_unknown_field: false,
        }),
        home_team_id: Some(env_config::DEFAULT_HOME_TEAM_ID.to_string()),
        away_team_id: Some(env_config::DEFAULT_AWAY_TEAM_ID.to_string()),
        home_formation: None,
        away_formation: None,
        home_tactic_id: None,
        away_tactic_id: None,
        combat: Some(combat),
        observation_profile: Some("representative".to_string()),
        slot_sources: Some(env_config::default_slot_sources()),
        reward_team: Some("home".to_string()),
        objective_channels: Some(vec!["match_outcome".to_string()]),
        shaping_channels: Some(Vec::new()),
        max_episode_ticks: overrides.max_episode_ticks.map(|t| t as f64),
    };
    Ok(config)
}

// ---------------------------------------------------------------------------
// Observation-profile conversions (see the module doc).
// ---------------------------------------------------------------------------

#[must_use]
fn to_observation_profile(
    profile: env_config::EnvObservationProfile,
) -> env_observation::EnvObservationProfile {
    match profile {
        env_config::EnvObservationProfile::Representative => {
            env_observation::EnvObservationProfile::Representative
        }
        env_config::EnvObservationProfile::Team => env_observation::EnvObservationProfile::Team,
        env_config::EnvObservationProfile::Privileged => {
            env_observation::EnvObservationProfile::Privileged
        }
    }
}

#[must_use]
fn to_action_profile(
    profile: env_observation::EnvObservationProfile,
) -> env_action::EnvObservationProfile {
    match profile {
        env_observation::EnvObservationProfile::Representative => {
            env_action::EnvObservationProfile::Representative
        }
        env_observation::EnvObservationProfile::Team => env_action::EnvObservationProfile::Team,
        env_observation::EnvObservationProfile::Privileged => {
            env_action::EnvObservationProfile::Privileged
        }
    }
}

fn to_action_view(view: &env_observation::EnvActionView) -> env_action::EnvActionView {
    env_action::EnvActionView {
        profile: to_action_profile(view.profile),
        slot: view.slot,
        own: env_action::EnvActionViewSelf {
            stunned: view.own.stunned,
            header_ready: view.own.header_ready,
            tackle_ready: view.own.tackle_ready,
            dodge_ready: view.own.dodge_ready,
            equipment: view
                .own
                .equipment
                .as_ref()
                .map(|e| env_action::EnvActionViewEquipment { ready: e.ready }),
        },
        ball: env_action::EnvActionViewBall {
            airborne: view.ball.airborne,
        },
    }
}

fn to_env_reward_contact_result(
    result: CombatSnapshotContactResult,
) -> env_reward::CombatContactResult {
    match result {
        CombatSnapshotContactResult::Hit => env_reward::CombatContactResult::Hit,
        CombatSnapshotContactResult::Extended => env_reward::CombatContactResult::Extended,
        CombatSnapshotContactResult::Guarded => env_reward::CombatContactResult::Guarded,
        CombatSnapshotContactResult::Immune => env_reward::CombatContactResult::Immune,
        CombatSnapshotContactResult::Superseded => env_reward::CombatContactResult::Superseded,
    }
}

fn to_env_side(team: StateTeam) -> env_reward::EnvSide {
    match team {
        StateTeam::Home => env_reward::EnvSide::Home,
        StateTeam::Away => env_reward::EnvSide::Away,
    }
}

// ---------------------------------------------------------------------------
// `metrics::MetricsMatchView` adapter (see the module doc).
// ---------------------------------------------------------------------------

fn to_metrics_keeper_state(k: crate::keeper::KeeperBehaviorState) -> metrics::KeeperState {
    use crate::keeper::KeeperBehaviorState as K;
    match k {
        K::Base => metrics::KeeperState::Base,
        K::Advance => metrics::KeeperState::Advance,
        K::Contain => metrics::KeeperState::Contain,
        K::Set => metrics::KeeperState::Set,
        K::Retreat => metrics::KeeperState::Retreat,
        K::Recover => metrics::KeeperState::Recover,
    }
}

fn to_metrics_event(e: &MatchEvent) -> metrics::MetricsEventView {
    metrics::MetricsEventView {
        kind: e.kind.wire_str().to_string(),
        player: e.player.clone(),
        shot_type: e.shot_type.map(|t| {
            match t {
                crate::keeper::KeeperShotType::Chip => "chip",
                crate::keeper::KeeperShotType::Ground => "ground",
            }
            .to_string()
        }),
        on_target: e.on_target,
        keeper_state: e.keeper_state.map(to_metrics_keeper_state),
        keeper_depth: e.keeper_depth,
    }
}

fn to_metrics_view(s: &MatchState) -> MetricsMatchView {
    MetricsMatchView {
        players: s
            .players
            .iter()
            .map(|p| MetricsPlayerView {
                id: p.id.clone(),
                team: match p.team {
                    StateTeam::Home => metrics::MatchTeam::Home,
                    StateTeam::Away => metrics::MatchTeam::Away,
                },
                is_keeper: p.is_keeper,
                vel: p.vel,
                run_vel: p.run_vel,
                move_speed: p.move_speed,
                sprinting: p.sprinting,
                dodge_timer: p.dodge_timer,
                keeper_state: Some(to_metrics_keeper_state(p.keeper_state)),
            })
            .collect(),
        human_controlled: s.human_controlled,
        controlled: (s.controlled - 1) as usize,
        score: metrics::Score {
            home: s.score.home,
            away: s.score.away,
        },
        owner: s.owner.map(|i| (i - 1) as usize),
        events: s.events.iter().map(to_metrics_event).collect(),
    }
}

// ---------------------------------------------------------------------------
// Instance construction.
// ---------------------------------------------------------------------------

fn producer_sources(config: &env_config::EnvConfig) -> [MatchSlotSource; 8] {
    let mut sources = [MatchSlotSource {
        kind: MatchSlotSourceKind::Neutral,
        seed: None,
    }; 8];
    for (index, source) in config.slot_sources.iter().enumerate() {
        sources[index] = match source.kind {
            env_config::EnvSlotSourceKind::Bot => MatchSlotSource {
                kind: MatchSlotSourceKind::Bot,
                seed: Some(
                    source
                        .seed
                        .expect("normalize validates a bot source always carries a seed")
                        as f64,
                ),
            },
            env_config::EnvSlotSourceKind::Neutral => MatchSlotSource {
                kind: MatchSlotSourceKind::Neutral,
                seed: None,
            },
            // Policy and tape rows are both authored upstream of the
            // producer: the environment writes them into the base frame it
            // materializes.
            env_config::EnvSlotSourceKind::Policy | env_config::EnvSlotSourceKind::Tape => {
                MatchSlotSource {
                    kind: MatchSlotSourceKind::Frame,
                    seed: None,
                }
            }
        };
    }
    sources
}

/// `sim::r#match::new` has no away-formation override, so an away formation
/// is expressed as a per-run `TeamData` view exactly the way
/// `sim::headless` does it. Canonical team data stays immutable and the
/// existing `team.formation` path still builds the side.
fn with_formation(
    team: &gc_data::teams::TeamData,
    formation: Option<&str>,
) -> gc_data::teams::TeamData {
    match formation {
        None => *team,
        Some(f) => {
            // `formation` borrows from `env_config::EnvConfig`'s owned
            // `String`, but `TeamData::formation` is `&'static str`.
            // `env_config::normalize` already validated `f` names an
            // authored formation, so look the canonical `'static` id back
            // up rather than borrowing the runtime string directly.
            let canonical = gc_data::formations::get(f)
                .expect("normalize validates the formation id")
                .id;
            gc_data::teams::TeamData {
                formation: canonical,
                ..*team
            }
        }
    }
}

fn copy_tape_frames(frames: Option<&[InputFrame]>) -> Result<Vec<InputFrame>> {
    let Some(frames) = frames else {
        return Ok(Vec::new());
    };
    let mut copied = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let copy = input_frame::copy(frame).map_err(|e| {
            EnvError::new(
                EnvErrorCode::Malformed,
                format!("tape frame {} is invalid: {}", index + 1, e.message),
            )
        })?;
        if copy.tick != index as i64 {
            return failure(
                EnvErrorCode::Malformed,
                format!(
                    "tape frames must start at tick zero and be contiguous (frame {})",
                    index + 1
                ),
            );
        }
        copied.push(copy);
    }
    Ok(copied)
}

/// A complete episode instance. Every field is `pub` — see the module doc's
/// "`EnvInstance` fields are `pub`" section for why.
#[derive(Debug)]
pub struct EnvInstance {
    /// Always [`VERSION`].
    pub version: i64,
    /// The normalized config this episode was built from.
    pub config: env_config::EnvConfig,
    /// Canonical slots a caller must supply an action for, ascending.
    pub controlled_slots: Vec<i64>,
    /// Canonical slots driven by a recorded tape.
    pub tape_slots: Vec<i64>,
    /// Next tick to simulate; equals the current boundary tick.
    pub tick: i64,
    /// Ticks actually simulated so far this episode.
    pub episode_ticks: i64,
    /// Whether the match itself has ended.
    pub terminated: bool,
    /// Whether the episode was cut short without the match ending.
    pub truncated: bool,
    /// Why the match ended, when [`Self::terminated`].
    pub termination: Option<EnvTerminationReason>,
    /// Why the episode was cut short, when [`Self::truncated`].
    pub truncation: Option<EnvTruncationReason>,
    /// One hash per boundary; index zero is the reset boundary.
    pub boundary_hashes: Vec<String>,
    /// Materialized effective rows, in tick order.
    pub frames: Vec<InputFrame>,
    /// This episode's tape identity.
    pub identity: InputTapeIdentity,
    /// The live match state. `pub`, not `_state` — see the module doc.
    pub state: MatchState,
    /// The live combat companion, when [`env_config::EnvConfig::combat`].
    /// `pub`, not `_combat` — see the module doc.
    pub combat: Option<CombatMatchState>,
    /// The retained slot-input producer. `pub`, not `_producer`.
    pub producer: SlotInputProducerState,
    /// The retained #128 metrics collector. `pub`, not `_metrics`.
    pub metrics: MetricsCollector,
    /// The retained fixed-tick clock. `pub`, not `_clock`.
    pub clock: FixedClockState,
    /// The reset boundary's canonical snapshot. `pub`, not `_initial`.
    pub initial: MatchSnapshot,
    /// The caller-supplied recorded rows for tape-driven slots. `pub`, not
    /// `_tape_frames`.
    pub tape_frames: Vec<InputFrame>,
    /// The episode-end #128 metrics, computed once and cached.
    pub final_metrics: Option<metrics::MatchMetrics>,
    /// The fault message, once [`step`] has caught a simulation panic.
    pub fault: Option<String>,
    /// The tuning knobs this episode plays with, as an owned value rather
    /// than a shared global (see `crate::tuning`'s module doc). A config has
    /// no field to override it, so this is always a fresh, default registry
    /// (`Tuning::new()`, which serializes to `""`), i.e. no active
    /// tuning-panel override.
    pub tune: Tuning,
    /// Diagnostic: how many times [`env_observation::build`] has been
    /// called for this instance, across [`observe`] and [`step`]. Never
    /// affects a hash — modeled on `MatchDriver::snapshot_captures` in
    /// `gc-netcode`. `tests/env_budget.rs`'s "builds exactly one
    /// observation ... per slot per step" case checks this counter directly
    /// against the expected call count.
    ///
    /// `Cell` because [`observe`] only takes `&EnvInstance`; making it
    /// `&mut` would cascade through call sites that have no other reason to
    /// need one.
    pub observation_builds: std::cell::Cell<i64>,
    /// Diagnostic: how many times [`env_observation::action_view`] has been
    /// called for this instance, i.e. how many per-slot masks
    /// [`action_masks`] has built. Never affects a hash; same precedent and
    /// `Cell` rationale as [`Self::observation_builds`].
    pub action_views: std::cell::Cell<i64>,
    /// Diagnostic: how many times this instance's boundary has been
    /// captured via `match_snapshot::capture` — once at [`reset`], once per
    /// simulated tick inside [`step`], once per call to the standalone
    /// [`snapshot`] function, plus any fallback capture
    /// [`env_observation::EnvObservationContext::redundant_captures`]
    /// records when the privileged profile could not reuse a donated
    /// snapshot. Never affects a hash. `tests/env_budget.rs`'s "does not
    /// re-capture the boundary for the privileged profile" case checks this
    /// counter as an exact call count: a step's delta on this field is
    /// `ticks_simulated` regardless of observation profile.
    pub snapshot_captures: std::cell::Cell<i64>,
}

/// Construct a fresh episode. Everything the episode depends on is named in
/// the config, so the manifest returned by [`manifest`] is sufficient to
/// reproduce this run tick for tick.
///
/// # Errors
///
/// Returns `Err` if `config` cannot be normalized, tape frames are missing
/// or supplied without a tape-driven slot, or the recorded frames are
/// malformed.
pub fn reset(
    config: &env_config::RawEnvConfig,
    tape_frames: Option<&[InputFrame]>,
) -> Result<EnvInstance> {
    let normalized = env_config::normalize(config).map_err(convert_config_error)?;
    let copied_frames = copy_tape_frames(tape_frames)?;
    let tape_slots = env_config::tape_slots(&normalized);
    if !tape_slots.is_empty() && copied_frames.is_empty() {
        return failure(
            EnvErrorCode::MissingTape,
            "tape-driven slots require recorded frames",
        );
    }
    if tape_slots.is_empty() && !copied_frames.is_empty() {
        return failure(
            EnvErrorCode::Malformed,
            "recorded frames were supplied but no slot source is a tape",
        );
    }

    let fixture = env_config::resolve(&normalized);
    let away = with_formation(&fixture.away, normalized.away_formation.as_deref());
    let ownership =
        sim_match::ownership_for_teams(&fixture.home, &away, Some(&fixture.players_by_id));
    let mut state = sim_match::new(sim_match::NewMatchOptions {
        home: &fixture.home,
        away: &away,
        field: match_snapshot::PitchSize {
            w: normalized.field.w,
            h: normalized.field.h,
        },
        home_formation: normalized.home_formation.as_deref(),
        tactic: Some(&fixture.home_tactic),
        away_tactic: Some(&fixture.away_tactic),
        duration: Some(normalized.duration),
        max_goals: Some(normalized.max_goals),
        seed: Some(normalized.seed as f64),
        players_by_id: Some(&fixture.players_by_id),
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: Some(ownership.clone()),
    });
    let combat_state = normalized
        .combat
        .then(|| combat::new_state(&mut state, None));
    let initial = match_snapshot::capture(&state, combat_state.as_ref());
    let tune = Tuning::new();
    let identity = InputTapeIdentity {
        tape_version: if combat_state.is_some() {
            input_tape::COMBAT_VERSION
        } else {
            input_tape::VERSION
        },
        input_version: input_frame::VERSION,
        snapshot_version: initial.version,
        build: normalized.build.clone(),
        source: normalized.source.clone(),
        content: normalized.content.clone(),
        tuning: tune.serialize(),
        config: env_config::digest(&normalized),
        fixture: normalized.fixture.clone(),
        seed: normalized.seed as f64,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: combat_state
            .as_ref()
            .map(|c| combat_identity::for_state(Some(c))),
    };
    let metrics_view = to_metrics_view(&state);
    let boundary_hash = match_snapshot::hash(&initial);
    Ok(EnvInstance {
        version: VERSION,
        controlled_slots: env_config::controlled_slots(&normalized),
        tape_slots,
        tick: state.input_tick,
        episode_ticks: 0,
        terminated: false,
        truncated: false,
        termination: None,
        truncation: None,
        boundary_hashes: vec![boundary_hash],
        frames: Vec::new(),
        identity,
        combat: combat_state,
        producer: slot_input::new_producer(producer_sources(&normalized)),
        metrics: metrics::new(&metrics_view),
        clock: fixed_clock::new(),
        initial,
        tape_frames: copied_frames,
        final_metrics: None,
        fault: None,
        tune,
        config: normalized,
        state,
        observation_builds: std::cell::Cell::new(0),
        action_views: std::cell::Cell::new(0),
        // One capture already happened just above, for `initial`.
        snapshot_captures: std::cell::Cell::new(1),
    })
}

// ---------------------------------------------------------------------------
// Observation / masks / snapshot.
// ---------------------------------------------------------------------------

/// The observation at the current boundary, built before any action for the
/// next tick exists.
///
/// # Panics
///
/// Panics if the instance's own controlled slots are not routable — an
/// invariant [`reset`] already guarantees.
#[must_use]
pub fn observe(instance: &EnvInstance) -> env_observation::EnvObservation {
    instance
        .observation_builds
        .set(instance.observation_builds.get() + 1);
    env_observation::build(
        &instance.state,
        instance.combat.as_ref(),
        &instance.controlled_slots,
        to_observation_profile(instance.config.observation_profile),
        None,
    )
    .expect("reset guarantees controlled_slots are routable under the configured profile")
}

/// Legality masks for the controlled slots. These are derived from the
/// narrow own+ball projection rather than a full observation: a mask reads
/// nothing else, and building the teammate/opponent/event records per slot
/// just to compute legality would double the per-step observation cost.
///
/// # Panics
///
/// Panics if a controlled slot is not routable — an invariant [`reset`]
/// already guarantees.
#[must_use]
pub fn action_masks(instance: &EnvInstance) -> IndexMap<i64, env_action::EnvActionMask> {
    let mut masks = IndexMap::new();
    for &slot in &instance.controlled_slots {
        instance.action_views.set(instance.action_views.get() + 1);
        let view = env_observation::action_view(
            &instance.state,
            instance.combat.as_ref(),
            slot,
            to_observation_profile(instance.config.observation_profile),
        )
        .expect("reset guarantees controlled_slots are routable");
        masks.insert(slot, env_action::mask(&to_action_view(&view)));
    }
    masks
}

/// The current boundary's canonical snapshot.
#[must_use]
pub fn snapshot(instance: &EnvInstance) -> MatchSnapshot {
    instance
        .snapshot_captures
        .set(instance.snapshot_captures.get() + 1);
    match_snapshot::capture(&instance.state, instance.combat.as_ref())
}

// ---------------------------------------------------------------------------
// Stepping.
// ---------------------------------------------------------------------------

/// Why the match itself ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvTerminationReason {
    /// Regulation time ran out.
    TimeExpired,
    /// A side reached the configured goal cap.
    GoalCap,
}

/// Why the episode was cut short without the match ending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvTruncationReason {
    /// The configured episode tick budget was spent.
    StepLimit,
    /// A tape-driven slot ran out of recorded rows.
    TapeExhausted,
}

/// Why play is currently held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvStoppageReason {
    /// The post-restart hold before ordinary play resumes.
    KickoffHold,
}

fn normalize_actions(
    instance: &EnvInstance,
    actions: &IndexMap<i64, env_action::RawAction>,
) -> Result<IndexMap<i64, env_action::EnvSlotAction>> {
    for &key in actions.keys() {
        if !instance.controlled_slots.contains(&key) {
            return failure(
                EnvErrorCode::UnknownSlot,
                format!("slot {key} is not a policy slot"),
            );
        }
    }
    let masks = action_masks(instance);
    let mut normalized = IndexMap::new();
    for &slot in &instance.controlled_slots {
        let Some(action) = actions.get(&slot) else {
            return failure(
                EnvErrorCode::MissingSlot,
                format!("policy slot {slot} has no action this step"),
            );
        };
        let valid = env_action::validate(action).map_err(convert_action_error)?;
        let mask = masks
            .get(&slot)
            .expect("action_masks covers every controlled slot");
        env_action::check_mask(&valid, mask)
            .map_err(convert_action_error)
            .map_err(|e| EnvError::new(e.code, format!("slot {slot}: {}", e.message)))?;
        normalized.insert(slot, valid);
    }
    Ok(normalized)
}

fn base_frame(
    instance: &EnvInstance,
    tick: i64,
    actions: &IndexMap<i64, env_action::EnvSlotAction>,
) -> Result<InputFrame> {
    let mut slots = [input_frame::InputSample::default(); 8];
    for index in 1..=input_frame::SLOT_COUNT {
        let source = &instance.config.slot_sources[(index - 1) as usize];
        slots[(index - 1) as usize] = match source.kind {
            env_config::EnvSlotSourceKind::Policy => {
                let action = actions.get(&index).expect("policy slot action is missing");
                env_action::to_sample(action).map_err(|e| {
                    EnvError::new(
                        EnvErrorCode::IllegalAction,
                        format!("slot {index}: {}", e.message),
                    )
                })?
            }
            env_config::EnvSlotSourceKind::Tape => {
                let Some(frame) = instance.tape_frames.get(tick as usize) else {
                    return failure(
                        EnvErrorCode::TapeExhausted,
                        format!("tape slot {index} has no recorded row for tick {tick}"),
                    );
                };
                frame.slots[(index - 1) as usize]
            }
            // Overwritten by the producer; the base row must still be
            // complete.
            _ => input_frame::neutral_sample(),
        };
    }
    input_frame::new(tick, Some(slots))
        .map_err(|e| EnvError::new(EnvErrorCode::Malformed, e.message))
}

/// One confirmed event of a completed tick.
#[derive(Clone, Debug, PartialEq)]
pub enum EnvEventPayload {
    /// A soccer-domain match event.
    Soccer(MatchEvent),
    /// A combat-domain event.
    Combat(crate::combat_snapshot::CombatEvent),
}

/// One event confirmed by a step.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvStepEvent {
    /// Tick the event was confirmed on.
    pub tick: i64,
    /// Absolute acting side, when identified.
    pub team: Option<StateTeam>,
    /// Canonical input slot of the actor, when it has one.
    pub slot: Option<i64>,
    /// The full-fidelity confirmed record.
    pub payload: EnvEventPayload,
}

fn collect_events(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    confirmed_tick: i64,
    events: &mut Vec<EnvStepEvent>,
    reward_events: &mut Vec<env_reward::EnvRewardEvent>,
) {
    let mut team_of_id: IndexMap<&str, StateTeam> = IndexMap::new();
    let mut slot_of_id: IndexMap<&str, Option<i64>> = IndexMap::new();
    for (index, player) in state.players.iter().enumerate() {
        team_of_id.insert(player.id.as_str(), player.team);
        slot_of_id.insert(player.id.as_str(), state.slot_for_player[index]);
    }
    for event in &state.events {
        let team = event
            .player
            .as_deref()
            .and_then(|id| team_of_id.get(id).copied());
        let slot = event
            .player
            .as_deref()
            .and_then(|id| slot_of_id.get(id).copied().flatten());
        events.push(EnvStepEvent {
            tick: confirmed_tick,
            team,
            slot,
            payload: EnvEventPayload::Soccer(event.clone()),
        });
        reward_events.push(env_reward::EnvRewardEvent {
            kind: event.kind.wire_str().to_string(),
            team: team.map(to_env_side),
            result: None,
        });
    }
    if let Some(combat_state) = combat_state {
        for event in &combat_state.events {
            let source = event
                .source_index
                .and_then(|idx| state.players.get(idx as usize - 1));
            let team = source.map(|p| p.team);
            let slot = event.source_index.and_then(|idx| {
                state
                    .slot_for_player
                    .get(idx as usize - 1)
                    .copied()
                    .flatten()
            });
            events.push(EnvStepEvent {
                tick: event.tick,
                team,
                slot,
                payload: EnvEventPayload::Combat(event.clone()),
            });
            reward_events.push(env_reward::EnvRewardEvent {
                kind: event.kind.wire_str().to_string(),
                team: team.map(to_env_side),
                result: event.result.map(to_env_reward_contact_result),
            });
        }
    }
}

fn advance_tick(
    producer: &mut SlotInputProducerState,
    state: &mut MatchState,
    mut combat_state: Option<&mut CombatMatchState>,
    metrics_collector: &mut MetricsCollector,
    clock: &mut FixedClockState,
    tune: &Tuning,
    frame: &InputFrame,
) -> (InputFrame, String) {
    let (effective, _decisions) =
        slot_input::materialize(producer, state, frame, combat_state.as_deref());
    fixed_clock::step(clock, &effective, |_tick, tick_frame: &InputFrame| {
        sim_match::step(
            state,
            DT,
            sim_match::StepInput::Frame(tick_frame),
            combat_state.as_deref_mut(),
            tune,
        );
        let view = to_metrics_view(state);
        metrics::observe(metrics_collector, &view, DT, tune);
        !state.finished
    });
    let wire = input_frame::encode(&effective).expect("materialized frame is always valid");
    (effective, wire)
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn termination_reason(state: &MatchState) -> EnvTerminationReason {
    if state.time_left <= 0.0 {
        EnvTerminationReason::TimeExpired
    } else {
        EnvTerminationReason::GoalCap
    }
}

fn final_metrics(instance: &mut EnvInstance) -> metrics::MatchMetrics {
    if instance.final_metrics.is_none() {
        let view = to_metrics_view(&instance.state);
        instance.final_metrics = Some(metrics::finish(&mut instance.metrics, &view));
    }
    instance.final_metrics.expect("just populated above")
}

/// Evaluation diagnostics for one step. Never an optimization target.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvDiagnostics {
    /// Always `"evaluation"`.
    pub role: &'static str,
    /// The diagnostic channel these belong to.
    pub channel: env_reward::EnvRewardChannelId,
    /// Ticks simulated so far this episode.
    pub episode_ticks: i64,
    /// Ticks simulated by this step.
    pub simulated_ticks: i64,
    /// Confirmed event counts, keyed by wire kind.
    pub event_counts: IndexMap<String, i64>,
    /// #128 metrics, produced once the episode ends.
    pub metrics: Option<metrics::MatchMetrics>,
}

/// One `step` call's result.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvStepResult {
    /// Always [`STEP_VERSION`].
    pub version: i64,
    /// Boundary tick after the step.
    pub tick: i64,
    /// Ticks actually simulated; zero when the step could not advance.
    pub ticks_simulated: i64,
    /// The observation at the new boundary.
    pub observation: env_observation::EnvObservation,
    /// The evaluated reward.
    pub reward: env_reward::EnvRewardResult,
    /// The match itself ended.
    pub terminated: bool,
    /// The episode was cut short; the match did not end.
    pub truncated: bool,
    /// Why the match ended, when `terminated`.
    pub termination: Option<EnvTerminationReason>,
    /// Why the episode was cut short, when `truncated`.
    pub truncation: Option<EnvTruncationReason>,
    /// Play is held (restart), distinct from termination.
    pub stoppage: bool,
    /// Why play is held, when `stoppage`.
    pub stoppage_reason: Option<EnvStoppageReason>,
    /// Every event confirmed by this step.
    pub events: Vec<EnvStepEvent>,
    /// Evaluation diagnostics.
    pub diagnostics: EnvDiagnostics,
    /// Canonical snapshot hash after the step.
    pub boundary_hash: String,
    /// One hash per simulated tick, in order.
    pub boundary_hashes: Vec<String>,
    /// Encoded effective `InputFrame`s consumed, for audit.
    pub action_wires: Vec<String>,
}

/// Advance the episode by `ticks` canonical fixed ticks (default one),
/// holding the supplied actions across them. One-shot edges only fire on
/// the first tick: an edge means "this tick only", so repeating it would
/// fabricate presses the policy never made. Held intents persist for every
/// tick of the step.
///
/// Termination (the match ended) and truncation (the episode was cut
/// short) are returned separately and are never both true.
///
/// # Errors
///
/// Returns `Err` if the instance already faulted or the episode already
/// ended, `ticks` is not a positive integer, an action is illegal or
/// missing, or a simulation invariant is violated (see the module doc's
/// `catch_unwind` section).
pub fn step(
    instance: &mut EnvInstance,
    actions: &IndexMap<i64, env_action::RawAction>,
    ticks: Option<i64>,
) -> Result<EnvStepResult> {
    if let Some(fault) = &instance.fault {
        return failure(
            EnvErrorCode::Faulted,
            format!("environment is faulted: {fault}"),
        );
    }
    if instance.terminated || instance.truncated {
        return failure(EnvErrorCode::EpisodeOver, "the episode has already ended");
    }
    let requested = ticks.unwrap_or(1);
    if requested < 1 {
        return failure(
            EnvErrorCode::Malformed,
            "step tick count must be a positive integer",
        );
    }
    let normalized = normalize_actions(instance, actions)?;

    let score_before = env_reward::EnvRewardScore {
        home: instance.state.score.home,
        away: instance.state.score.away,
    };
    let owner_before = instance
        .state
        .owner
        .map(|i| instance.state.players[i as usize - 1].team);
    let mut events: Vec<EnvStepEvent> = Vec::new();
    let mut reward_events: Vec<env_reward::EnvRewardEvent> = Vec::new();
    let mut boundary_hashes: Vec<String> = Vec::new();
    let mut action_wires: Vec<String> = Vec::new();
    let mut simulated: i64 = 0;
    let mut held_actions = normalized;
    let mut snapshot: Option<MatchSnapshot> = None;

    for index in 1..=requested {
        if index > 1 {
            held_actions = held_actions
                .into_iter()
                .map(|(slot, action)| (slot, env_action::without_edges(&action)))
                .collect();
        }
        let tick = instance.state.input_tick;
        let frame = match base_frame(instance, tick, &held_actions) {
            Ok(frame) => frame,
            Err(e) if e.code == EnvErrorCode::TapeExhausted => {
                // An incomplete tape truncates the episode; it is not an
                // error and it is never reported as the match having ended.
                instance.truncated = true;
                instance.truncation = Some(EnvTruncationReason::TapeExhausted);
                break;
            }
            Err(e) => return Err(e),
        };

        // The whole tick runs under `catch_unwind`: an invariant violation
        // anywhere between materialization and the boundary is a fault the
        // caller must be able to reproduce, not a crash that takes the
        // trainer down with it. See the module doc's `catch_unwind`
        // section.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            advance_tick(
                &mut instance.producer,
                &mut instance.state,
                instance.combat.as_mut(),
                &mut instance.metrics,
                &mut instance.clock,
                &instance.tune,
                &frame,
            )
        }));
        let (effective, wire) = match result {
            Ok(pair) => pair,
            Err(payload) => {
                let fault_message = panic_message(payload.as_ref());
                instance.fault = Some(fault_message.clone());
                let last_boundary = instance.boundary_hashes.last().cloned().unwrap_or_default();
                let input_wire =
                    input_frame::encode(&frame).unwrap_or_else(|_| "<unencodable>".to_string());
                return failure(
                    EnvErrorCode::SimFault,
                    format!(
                        "simulation fault at tick {tick} (boundary {last_boundary}, input {input_wire}): {fault_message}"
                    ),
                );
            }
        };

        simulated += 1;
        instance.episode_ticks += 1;
        instance.tick = instance.state.input_tick;
        instance.frames.push(effective);
        instance
            .snapshot_captures
            .set(instance.snapshot_captures.get() + 1);
        let captured = match_snapshot::capture(&instance.state, instance.combat.as_ref());
        let hash = match_snapshot::hash(&captured);
        snapshot = Some(captured);
        instance.boundary_hashes.push(hash.clone());
        boundary_hashes.push(hash);
        action_wires.push(wire);
        collect_events(
            &instance.state,
            instance.combat.as_ref(),
            tick,
            &mut events,
            &mut reward_events,
        );
        if instance.state.finished {
            instance.terminated = true;
            instance.termination = Some(termination_reason(&instance.state));
            break;
        }
        if let Some(budget) = instance.config.max_episode_ticks
            && instance.episode_ticks >= budget
        {
            instance.truncated = true;
            instance.truncation = Some(EnvTruncationReason::StepLimit);
            break;
        }
    }

    let owner_after = instance
        .state
        .owner
        .map(|i| instance.state.players[i as usize - 1].team);
    let selection = env_reward::EnvRewardSelection {
        objectives: instance.config.objective_channels.clone(),
        shaping: instance.config.shaping_channels.clone(),
    };
    let reward = env_reward::evaluate(
        &env_reward::EnvRewardTransition {
            team: instance.config.reward_team,
            score_before,
            score_after: env_reward::EnvRewardScore {
                home: instance.state.score.home,
                away: instance.state.score.away,
            },
            owner_team_before: owner_before.map(to_env_side),
            owner_team_after: owner_after.map(to_env_side),
            events: reward_events,
            terminated: instance.terminated,
        },
        &selection,
    );

    let mut event_counts: IndexMap<String, i64> = IndexMap::new();
    for record in &events {
        let kind = match &record.payload {
            EnvEventPayload::Soccer(e) => e.kind.wire_str().to_string(),
            EnvEventPayload::Combat(e) => e.kind.wire_str().to_string(),
        };
        *event_counts.entry(kind).or_insert(0) += 1;
    }
    let episode_over = instance.terminated || instance.truncated;
    let context = snapshot
        .as_ref()
        .map(|snap| env_observation::EnvObservationContext {
            snapshot: Some(snap.clone()),
            boundary_hash: instance.boundary_hashes.last().cloned(),
            redundant_captures: std::cell::Cell::new(0),
        });
    instance
        .observation_builds
        .set(instance.observation_builds.get() + 1);
    let observation = env_observation::build(
        &instance.state,
        instance.combat.as_ref(),
        &instance.controlled_slots,
        to_observation_profile(instance.config.observation_profile),
        context.as_ref(),
    )
    .expect("reset guarantees controlled_slots are routable");
    // Fold in any fallback capture `privileged_view` recorded because the
    // donated `context` above lacked a snapshot (see
    // `EnvObservationContext::redundant_captures`'s doc). For a
    // representative/team profile this is always zero; for privileged it
    // should also be zero here, since `context.snapshot` is always `Some`
    // whenever at least one tick was simulated.
    if let Some(context) = &context {
        instance
            .snapshot_captures
            .set(instance.snapshot_captures.get() + context.redundant_captures.get());
    }

    Ok(EnvStepResult {
        version: STEP_VERSION,
        tick: instance.state.input_tick,
        ticks_simulated: simulated,
        observation,
        reward,
        terminated: instance.terminated,
        truncated: instance.truncated,
        termination: instance.termination,
        truncation: instance.truncation,
        stoppage: instance.state.kickoff_hold > 0.0,
        stoppage_reason: (instance.state.kickoff_hold > 0.0)
            .then_some(EnvStoppageReason::KickoffHold),
        events,
        diagnostics: EnvDiagnostics {
            role: "evaluation",
            channel: env_reward::DIAGNOSTIC_METRIC_CHANNEL,
            episode_ticks: instance.episode_ticks,
            simulated_ticks: simulated,
            event_counts,
            metrics: episode_over.then(|| final_metrics(instance)),
        },
        boundary_hash: instance.boundary_hashes.last().cloned().unwrap_or_default(),
        boundary_hashes,
        action_wires,
    })
}

// ---------------------------------------------------------------------------
// Manifest / tape export.
// ---------------------------------------------------------------------------

/// Everything needed to reproduce this episode: contract versions,
/// provenance, tuning and fixture identity, seed, per-slot ownership with
/// policy ids, reward selection, and the boundary hashes that pin the
/// trajectory.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvManifest {
    /// Always [`MANIFEST_VERSION`].
    pub version: i64,
    /// Always [`VERSION`].
    pub env_version: i64,
    /// The config's own version.
    pub config_version: i64,
    /// [`env_observation::VERSION`].
    pub observation_version: i64,
    /// [`env_action::VERSION`].
    pub action_version: i64,
    /// [`env_reward::VERSION`].
    pub reward_version: i64,
    /// [`input_frame::VERSION`].
    pub input_version: i64,
    /// The reset boundary snapshot's format version.
    pub snapshot_version: i64,
    /// The tape identity's declared version.
    pub tape_version: i64,
    /// [`fixed_clock::TICK_RATE`].
    pub tick_rate: i64,
    /// Build/commit provenance.
    pub build: String,
    /// Recording source label.
    pub source: String,
    /// Authored content revision label.
    pub content: String,
    /// Fixture label.
    pub fixture: String,
    /// Canonical config digest.
    pub config: String,
    /// Serialized active tuning knobs.
    pub tuning: String,
    /// Canonical match seed.
    pub seed: i64,
    /// Combat mechanics identity, when combat is enabled.
    pub combat: Option<String>,
    /// Stable fixture slot-to-player identity.
    pub ownership: input_frame::InputOwnership,
    /// Which observation lens this episode's slot views are built from.
    pub observation_profile: env_config::EnvObservationProfile,
    /// Canonical slots a caller must supply an action for.
    pub controlled_slots: Vec<i64>,
    /// Audit labels for policy slots, keyed by canonical slot.
    pub policy_ids: IndexMap<i64, String>,
    /// Every canonical slot's source configuration.
    pub slot_sources: [env_config::EnvSlotSourceConfig; 8],
    /// Selected objective reward channels.
    pub objective_channels: Vec<env_reward::EnvRewardChannelId>,
    /// Selected shaping reward channels.
    pub shaping_channels: Vec<env_reward::EnvRewardChannelId>,
    /// The diagnostics channel every step reports.
    pub diagnostic_channel: env_reward::EnvRewardChannelId,
    /// The reset boundary's hash.
    pub initial_boundary_hash: String,
    /// The current boundary's hash.
    pub boundary_hash: String,
    /// Ticks simulated so far this episode.
    pub episode_ticks: i64,
}

/// Everything needed to reproduce this episode.
#[must_use]
pub fn manifest(instance: &EnvInstance) -> EnvManifest {
    let mut policy_ids = IndexMap::new();
    for (index, source) in instance.config.slot_sources.iter().enumerate() {
        if let Some(policy_id) = &source.policy_id {
            policy_ids.insert(index as i64 + 1, policy_id.clone());
        }
    }
    EnvManifest {
        version: MANIFEST_VERSION,
        env_version: VERSION,
        config_version: instance.config.version,
        observation_version: env_observation::VERSION,
        action_version: env_action::VERSION,
        reward_version: env_reward::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: instance.initial.version,
        tape_version: instance.identity.tape_version,
        tick_rate: fixed_clock::TICK_RATE as i64,
        build: instance.config.build.clone(),
        source: instance.config.source.clone(),
        content: instance.config.content.clone(),
        fixture: instance.config.fixture.clone(),
        config: instance.identity.config.clone(),
        tuning: instance.identity.tuning.clone(),
        seed: instance.config.seed,
        combat: instance.identity.combat.clone(),
        ownership: instance.identity.ownership.clone(),
        observation_profile: instance.config.observation_profile,
        controlled_slots: instance.controlled_slots.clone(),
        policy_ids,
        slot_sources: instance.config.slot_sources.clone(),
        objective_channels: instance.config.objective_channels.clone(),
        shaping_channels: instance.config.shaping_channels.clone(),
        diagnostic_channel: env_reward::DIAGNOSTIC_METRIC_CHANNEL,
        initial_boundary_hash: instance.boundary_hashes[0].clone(),
        boundary_hash: instance
            .boundary_hashes
            .last()
            .cloned()
            .expect("boundary_hashes always carries at least the reset boundary"),
        episode_ticks: instance.episode_ticks,
    }
}

/// Export the episode as a canonical `InputTape`. The tape carries the
/// effective rows the simulation actually consumed and the boundary hashes
/// the environment recorded, so [`crate::replay::run`] re-deriving those
/// hashes is an independent check that the environment path and the replay
/// path agree.
///
/// # Errors
///
/// Returns `Err` if the recorded identity, initial snapshot, frames, and
/// boundary hashes are not mutually consistent — an invariant [`reset`]/
/// [`step`] already guarantee, so this should not fail in practice.
pub fn tape(instance: &EnvInstance) -> Result<InputTape> {
    input_tape::from_frozen_recording(
        &instance.identity,
        &instance.initial,
        &instance.frames,
        &instance.boundary_hashes,
        &instance.tune,
    )
    .map_err(|e| EnvError::new(EnvErrorCode::Malformed, e))
}
