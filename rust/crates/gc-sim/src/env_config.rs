//! A config is the complete, serializable identity of an episode:
//! build/content provenance, fixture and tuning identity, seed, duration,
//! per-slot ownership, observation profile, and reward channel selection. It
//! resolves authored content ids into the fixture tables `sim::match`
//! already consumes; it never reads files and never carries live simulation
//! state.
//!
//! Configs arrive from outside the simulation, so every rejection is a
//! recoverable [`Result`] rather than a panic (ARCHITECTURE.md §3 rule 5 /
//! AGENTS.md §7). [`normalize`] is the entry point that accepts genuinely untyped
//! external input; a typed Rust struct cannot represent "an extra unknown
//! field" or "the wrong fundamental type" on its own, so [`RawEnvConfig`]
//! and its raw nested shapes ([`RawFieldSize`], [`RawSlotSource`]) stand in
//! for that one untyped boundary. Every other function here works with the
//! fully typed [`EnvConfig`] that [`normalize`] produces.

use crate::env_reward;
use crate::fixed_clock;
use crate::input_frame;

/// The env config module version.
pub const VERSION: i64 = 1;
/// Default authored content revision label.
pub const DEFAULT_CONTENT: &str = "showcase-content-v1";
/// Default match length, in seconds.
pub const DEFAULT_DURATION: f64 = 30.0;
/// Default goal cap. 99 is `r#match::NO_GOAL_LIMIT`, spelled as a literal
/// here because this module validates configs and deliberately does not
/// pull in the match engine. See the decision record in
/// `docs/online/match_flow.md`.
pub const DEFAULT_MAX_GOALS: i64 = 99;
/// Default pitch size.
pub const DEFAULT_FIELD: EnvFieldSize = EnvFieldSize {
    w: 1648.0,
    h: 927.0,
};
/// Default tactic id for both sides.
pub const DEFAULT_TACTIC_ID: &str = "balanced";
/// Default home team id.
pub const DEFAULT_HOME_TEAM_ID: &str = "nebula";
/// Default away team id.
pub const DEFAULT_AWAY_TEAM_ID: &str = "orion";

fn is_finite(value: f64) -> bool {
    value.is_finite()
}

fn is_integer(value: f64) -> bool {
    value.is_finite() && value == value.floor()
}

/// Failure reasons an env-config operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvConfigErrorCode {
    /// The data violates a structural, type, or value invariant.
    Malformed,
    /// The config names a version this build does not support.
    UnsupportedVersion,
    /// The config names authored content (a team, formation, or tactic)
    /// that does not exist.
    UnknownContent,
    /// A required, non-empty selection (e.g. at least one policy slot) was
    /// empty.
    EmptySelection,
    /// The observation profile and controlled-slot count disagree.
    ProfileMismatch,
}

/// An expected, recoverable env-config failure (ARCHITECTURE.md §3 rule 5): a config
/// comes from outside the simulation, so an illegal config is a recoverable
/// rejection with a machine-readable reason, never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvConfigError {
    /// Machine-readable failure reason.
    pub code: EnvConfigErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl EnvConfigError {
    fn new(code: EnvConfigErrorCode, message: impl Into<String>) -> Self {
        EnvConfigError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnvConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EnvConfigError {}

/// Result alias for fallible env-config operations.
pub type Result<T> = std::result::Result<T, EnvConfigError>;

fn failure<T>(code: EnvConfigErrorCode, message: impl Into<String>) -> Result<T> {
    Err(EnvConfigError::new(code, message))
}

fn reward_error_code_name(code: env_reward::EnvRewardErrorCode) -> &'static str {
    match code {
        env_reward::EnvRewardErrorCode::Malformed => "malformed",
        env_reward::EnvRewardErrorCode::UnknownChannel => "unknown_channel",
        env_reward::EnvRewardErrorCode::WrongRole => "wrong_role",
        env_reward::EnvRewardErrorCode::DuplicateChannel => "duplicate_channel",
    }
}

// ---------------------------------------------------------------------------
// Raw, not-yet-validated external input.
// ---------------------------------------------------------------------------

/// A raw, not-yet-validated `{ w, h }` field-size override.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawFieldSize {
    /// Raw pitch width, when present.
    pub w: Option<f64>,
    /// Raw pitch height, when present.
    pub h: Option<f64>,
    /// True when the caller's table carried a field other than `w`/`h`.
    pub has_unknown_field: bool,
}

/// One raw, not-yet-validated slot source entry. `kind` stays a bare string
/// (rather than [`EnvSlotSourceKind`]) because it may not name a known kind
/// — that is exactly what `copy_slot_sources` must detect and reject.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawSlotSource {
    /// Raw `kind` field; may not name a known [`EnvSlotSourceKind`].
    pub kind: String,
    /// Raw `seed` field, when present. Required for `bot`, forbidden
    /// otherwise; kept as `f64` (rather than `i64`) so a non-integer value
    /// can still be rejected explicitly.
    pub seed: Option<f64>,
    /// Raw `policy_id` field, when present. Only meaningful for `policy`.
    pub policy_id: Option<String>,
}

/// The untyped payload offered to [`normalize`]: every documented
/// [`EnvConfig`] field, before its value (and, at the top level and inside
/// `field`, its very presence as a *known* field) has been checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawEnvConfig {
    /// True when the caller's table carried a field other than the ones
    /// named below.
    pub has_unknown_field: bool,
    /// Raw `version` field, when present.
    pub version: Option<i64>,
    /// Raw `build` field.
    pub build: Option<String>,
    /// Raw `source` field, when present.
    pub source: Option<String>,
    /// Raw `content` field, when present.
    pub content: Option<String>,
    /// Raw `fixture` field, when present.
    pub fixture: Option<String>,
    /// Raw `seed` field. Kept as `f64` so a non-integer value can still be
    /// rejected explicitly.
    pub seed: Option<f64>,
    /// Raw `duration` field, when present.
    pub duration: Option<f64>,
    /// Raw `max_goals` field, when present. Kept as `f64` so a non-integer
    /// value can still be rejected explicitly.
    pub max_goals: Option<f64>,
    /// Raw `field` field, when present.
    pub field: Option<RawFieldSize>,
    /// Raw `home_team_id` field, when present.
    pub home_team_id: Option<String>,
    /// Raw `away_team_id` field, when present.
    pub away_team_id: Option<String>,
    /// Raw `home_formation` field, when present.
    pub home_formation: Option<String>,
    /// Raw `away_formation` field, when present.
    pub away_formation: Option<String>,
    /// Raw `home_tactic_id` field, when present.
    pub home_tactic_id: Option<String>,
    /// Raw `away_tactic_id` field, when present.
    pub away_tactic_id: Option<String>,
    /// Raw `combat` field, when present. A wrong-type-guard is unnecessary
    /// here (a Rust `bool` cannot hold anything else), matching the
    /// `content_validation` precedent for checks a typed language already
    /// guarantees.
    pub combat: Option<bool>,
    /// Raw `observation_profile` field, when present.
    pub observation_profile: Option<String>,
    /// Raw `slot_sources` field, when present. `None` means "use
    /// [`default_slot_sources`]".
    pub slot_sources: Option<Vec<RawSlotSource>>,
    /// Raw `reward_team` field, when present.
    pub reward_team: Option<String>,
    /// Raw `objective_channels` field, when present.
    pub objective_channels: Option<Vec<String>>,
    /// Raw `shaping_channels` field, when present.
    pub shaping_channels: Option<Vec<String>>,
    /// Raw `max_episode_ticks` field, when present. Kept as `f64` so a
    /// non-integer value can still be rejected explicitly.
    pub max_episode_ticks: Option<f64>,
}

fn raw_channel_ids(names: &[String]) -> env_reward::RawChannelIds {
    env_reward::RawChannelIds::List(
        names
            .iter()
            .cloned()
            .map(env_reward::RawChannelIdEntry::Str)
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Normalized, typed shapes.
// ---------------------------------------------------------------------------

/// Which observation lens an episode's slot views are built from. Declared
/// locally, and duplicated in `env_action.rs` too (see that module's doc
/// comment), keeping each module's dependency surface minimal rather than
/// sharing one canonical type across modules that only need this small
/// closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvObservationProfile {
    /// One controlled slot restricted to presented, current cues.
    Representative,
    /// Several controlled slots, each still player-observable.
    Team,
    /// Authoritative oracle/debug profile; never a human proxy.
    Privileged,
}

fn observation_profile_name(profile: EnvObservationProfile) -> &'static str {
    match profile {
        EnvObservationProfile::Representative => "representative",
        EnvObservationProfile::Team => "team",
        EnvObservationProfile::Privileged => "privileged",
    }
}

fn observation_profile_from_name(name: &str) -> Option<EnvObservationProfile> {
    match name {
        "representative" => Some(EnvObservationProfile::Representative),
        "team" => Some(EnvObservationProfile::Team),
        "privileged" => Some(EnvObservationProfile::Privileged),
        _ => None,
    }
}

fn env_side_name(side: env_reward::EnvSide) -> &'static str {
    match side {
        env_reward::EnvSide::Home => "home",
        env_reward::EnvSide::Away => "away",
    }
}

fn env_side_from_name(name: &str) -> Option<env_reward::EnvSide> {
    match name {
        "home" => Some(env_reward::EnvSide::Home),
        "away" => Some(env_reward::EnvSide::Away),
        _ => None,
    }
}

/// A pitch size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvFieldSize {
    /// Pitch width, world px.
    pub w: f64,
    /// Pitch height, world px.
    pub h: f64,
}

/// Which mechanism drives one canonical input slot for an episode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvSlotSourceKind {
    /// The caller supplies this slot's action every step.
    Policy,
    /// The slot replays a supplied recorded `InputFrame` row.
    Tape,
    /// The slot is filled by the deterministic built-in slot bot.
    Bot,
    /// The slot receives the canonical neutral row.
    Neutral,
}

fn slot_source_kind_name(kind: EnvSlotSourceKind) -> &'static str {
    match kind {
        EnvSlotSourceKind::Policy => "policy",
        EnvSlotSourceKind::Tape => "tape",
        EnvSlotSourceKind::Bot => "bot",
        EnvSlotSourceKind::Neutral => "neutral",
    }
}

fn slot_source_kind_from_name(name: &str) -> Option<EnvSlotSourceKind> {
    match name {
        "policy" => Some(EnvSlotSourceKind::Policy),
        "tape" => Some(EnvSlotSourceKind::Tape),
        "bot" => Some(EnvSlotSourceKind::Bot),
        "neutral" => Some(EnvSlotSourceKind::Neutral),
        _ => None,
    }
}

/// One canonical input slot's normalized source configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvSlotSourceConfig {
    /// Which mechanism drives this slot.
    pub kind: EnvSlotSourceKind,
    /// Required for `bot`, forbidden otherwise.
    pub seed: Option<i64>,
    /// Audit label for `policy` slots (checkpoint / population id).
    pub policy_id: Option<String>,
}

/// The default slot-source configuration: slot one is a policy slot, every
/// other slot is neutral. Returns the same raw shape [`normalize`] accepts
/// for `slot_sources`, so a caller can take this, mutate a couple of
/// entries, and hand it back.
#[must_use]
pub fn default_slot_sources() -> Vec<RawSlotSource> {
    (1..=input_frame::SLOT_COUNT)
        .map(|index| RawSlotSource {
            kind: if index == 1 { "policy" } else { "neutral" }.to_string(),
            seed: None,
            policy_id: None,
        })
        .collect()
}

fn copy_slot_sources(sources: &[RawSlotSource]) -> Result<[EnvSlotSourceConfig; 8]> {
    if sources.len() != input_frame::SLOT_COUNT as usize {
        return failure(
            EnvConfigErrorCode::Malformed,
            format!(
                "slot_sources must have exactly {} entries",
                input_frame::SLOT_COUNT
            ),
        );
    }
    let mut copied: Vec<EnvSlotSourceConfig> = Vec::with_capacity(sources.len());
    for (position, source) in sources.iter().enumerate() {
        let index = position + 1;
        let Some(kind) = slot_source_kind_from_name(&source.kind) else {
            return failure(
                EnvConfigErrorCode::Malformed,
                format!("slot source {index} has an unknown kind"),
            );
        };
        let seed = if kind == EnvSlotSourceKind::Bot {
            match source.seed {
                Some(raw_seed) if is_integer(raw_seed) => Some(raw_seed as i64),
                _ => {
                    return failure(
                        EnvConfigErrorCode::Malformed,
                        format!("bot slot source {index} needs an integer seed"),
                    );
                }
            }
        } else {
            if source.seed.is_some() {
                return failure(
                    EnvConfigErrorCode::Malformed,
                    "only bot slot sources may carry a seed",
                );
            }
            None
        };
        let policy_id = if kind == EnvSlotSourceKind::Policy {
            match &source.policy_id {
                None => None,
                Some(id) if !id.is_empty() => Some(id.clone()),
                Some(_) => {
                    return failure(
                        EnvConfigErrorCode::Malformed,
                        format!("policy slot source {index} policy_id must be a non-empty string"),
                    );
                }
            }
        } else {
            if source.policy_id.is_some() {
                return failure(
                    EnvConfigErrorCode::Malformed,
                    "only policy slot sources may carry a policy_id",
                );
            }
            None
        };
        copied.push(EnvSlotSourceConfig {
            kind,
            seed,
            policy_id,
        });
    }
    Ok(copied
        .try_into()
        .expect("copy_slot_sources always produces exactly SLOT_COUNT entries"))
}

/// A complete, validated, serializable episode identity.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvConfig {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Build/commit provenance supplied by the caller.
    pub build: String,
    /// Recording source label; defaults to `build`.
    pub source: String,
    /// Authored content revision label.
    pub content: String,
    /// Fixture label; derived from the teams when omitted.
    pub fixture: String,
    /// Canonical match seed.
    pub seed: i64,
    /// Match length in seconds.
    pub duration: f64,
    /// Goal cap that terminates the match.
    pub max_goals: i64,
    /// Pitch size.
    pub field: EnvFieldSize,
    /// Key into `gc_data::teams`.
    pub home_team_id: String,
    /// Key into `gc_data::teams`.
    pub away_team_id: String,
    /// Key into `gc_data::formations`; `None` keeps the team default.
    pub home_formation: Option<String>,
    /// Key into `gc_data::formations`; `None` keeps the team default.
    pub away_formation: Option<String>,
    /// Key into `gc_data::tactics`.
    pub home_tactic_id: String,
    /// Key into `gc_data::tactics`.
    pub away_tactic_id: String,
    /// Enable the combat companion state (all four action families).
    pub combat: bool,
    /// Which observation lens builds this episode's slot views.
    pub observation_profile: EnvObservationProfile,
    /// Exactly eight, canonical `InputSlot` order.
    pub slot_sources: [EnvSlotSourceConfig; 8],
    /// Perspective every reward channel is expressed from.
    pub reward_team: env_reward::EnvSide,
    /// Selected objective reward channels.
    pub objective_channels: Vec<env_reward::EnvRewardChannelId>,
    /// Selected shaping reward channels.
    pub shaping_channels: Vec<env_reward::EnvRewardChannelId>,
    /// Truncation budget; `None` means only the match ends the episode.
    pub max_episode_ticks: Option<i64>,
}

/// The fixture side of an [`EnvConfig`]'s canonical slots that are policy
/// slots.
#[must_use]
pub fn controlled_slots(config: &EnvConfig) -> Vec<i64> {
    (1..=input_frame::SLOT_COUNT)
        .filter(|&index| {
            config.slot_sources[(index - 1) as usize].kind == EnvSlotSourceKind::Policy
        })
        .collect()
}

/// The canonical slots that replay a recorded tape.
#[must_use]
pub fn tape_slots(config: &EnvConfig) -> Vec<i64> {
    (1..=input_frame::SLOT_COUNT)
        .filter(|&index| config.slot_sources[(index - 1) as usize].kind == EnvSlotSourceKind::Tape)
        .collect()
}

/// Validate and complete a caller config. The result is a fresh, owned
/// value: nothing is shared with the caller, so a config cannot mutate
/// underneath an episode.
pub fn normalize(config: &RawEnvConfig) -> Result<EnvConfig> {
    if config.has_unknown_field {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config must be a table of known fields",
        );
    }
    if let Some(version) = config.version
        && version != VERSION
    {
        return failure(
            EnvConfigErrorCode::UnsupportedVersion,
            "unsupported env config version",
        );
    }
    let build = match &config.build {
        Some(value) if !value.is_empty() => value.clone(),
        _ => {
            return failure(
                EnvConfigErrorCode::Malformed,
                "config.build must be a non-empty string",
            );
        }
    };
    if let Some(value) = &config.source
        && value.is_empty()
    {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config.source must be a non-empty string",
        );
    }
    if let Some(value) = &config.content
        && value.is_empty()
    {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config.content must be a non-empty string",
        );
    }
    if let Some(value) = &config.fixture
        && value.is_empty()
    {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config.fixture must be a non-empty string",
        );
    }
    let seed = match config.seed {
        Some(value) if is_integer(value) => value as i64,
        _ => {
            return failure(
                EnvConfigErrorCode::Malformed,
                "config.seed must be a finite integer",
            );
        }
    };
    let duration = config.duration.unwrap_or(DEFAULT_DURATION);
    if !is_finite(duration) || duration <= 0.0 {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config.duration must be a positive finite number",
        );
    }
    let max_goals_raw = config.max_goals.unwrap_or(DEFAULT_MAX_GOALS as f64);
    if !is_integer(max_goals_raw) || max_goals_raw < 1.0 {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config.max_goals must be a positive integer",
        );
    }
    let max_goals = max_goals_raw as i64;
    let field = match &config.field {
        None => DEFAULT_FIELD,
        Some(raw_field) => {
            if raw_field.has_unknown_field {
                return failure(
                    EnvConfigErrorCode::Malformed,
                    "config.field must be a { w, h } table",
                );
            }
            let w = raw_field.w.unwrap_or(f64::NAN);
            let h = raw_field.h.unwrap_or(f64::NAN);
            if !is_finite(w) || w <= 0.0 || !is_finite(h) || h <= 0.0 {
                return failure(
                    EnvConfigErrorCode::Malformed,
                    "config.field dimensions must be positive finite numbers",
                );
            }
            EnvFieldSize { w, h }
        }
    };
    let home_team_id = config
        .home_team_id
        .clone()
        .unwrap_or_else(|| DEFAULT_HOME_TEAM_ID.to_string());
    let away_team_id = config
        .away_team_id
        .clone()
        .unwrap_or_else(|| DEFAULT_AWAY_TEAM_ID.to_string());
    let Some(home_team) = gc_data::teams::get(&home_team_id) else {
        return failure(
            EnvConfigErrorCode::UnknownContent,
            format!("unknown home team: {home_team_id}"),
        );
    };
    let Some(away_team) = gc_data::teams::get(&away_team_id) else {
        return failure(
            EnvConfigErrorCode::UnknownContent,
            format!("unknown away team: {away_team_id}"),
        );
    };
    if let Some(formation) = &config.home_formation
        && gc_data::formations::get(formation).is_none()
    {
        return failure(
            EnvConfigErrorCode::UnknownContent,
            format!("unknown formation: {formation}"),
        );
    }
    if let Some(formation) = &config.away_formation
        && gc_data::formations::get(formation).is_none()
    {
        return failure(
            EnvConfigErrorCode::UnknownContent,
            format!("unknown formation: {formation}"),
        );
    }
    let home_tactic_id = config
        .home_tactic_id
        .clone()
        .unwrap_or_else(|| DEFAULT_TACTIC_ID.to_string());
    let away_tactic_id = config
        .away_tactic_id
        .clone()
        .unwrap_or_else(|| DEFAULT_TACTIC_ID.to_string());
    if gc_data::tactics::get(&home_tactic_id).is_none() {
        return failure(
            EnvConfigErrorCode::UnknownContent,
            format!("unknown home tactic: {home_tactic_id}"),
        );
    }
    if gc_data::tactics::get(&away_tactic_id).is_none() {
        return failure(
            EnvConfigErrorCode::UnknownContent,
            format!("unknown away tactic: {away_tactic_id}"),
        );
    }
    let combat = config.combat.unwrap_or(false);
    let profile_name = config
        .observation_profile
        .clone()
        .unwrap_or_else(|| "representative".to_string());
    let Some(profile) = observation_profile_from_name(&profile_name) else {
        return failure(
            EnvConfigErrorCode::Malformed,
            format!("unknown observation profile: {profile_name}"),
        );
    };
    let default_sources;
    let raw_sources: &[RawSlotSource] = match &config.slot_sources {
        Some(sources) => sources,
        None => {
            default_sources = default_slot_sources();
            &default_sources
        }
    };
    let sources = copy_slot_sources(raw_sources)?;
    let policy_slots = sources
        .iter()
        .filter(|source| source.kind == EnvSlotSourceKind::Policy)
        .count();
    if policy_slots == 0 {
        return failure(
            EnvConfigErrorCode::EmptySelection,
            "at least one slot source must be a policy slot",
        );
    }
    if profile == EnvObservationProfile::Representative && policy_slots != 1 {
        return failure(
            EnvConfigErrorCode::ProfileMismatch,
            "the representative profile controls exactly one slot; use the team profile",
        );
    }
    let reward_team_name = config
        .reward_team
        .clone()
        .unwrap_or_else(|| "home".to_string());
    let Some(reward_team) = env_side_from_name(&reward_team_name) else {
        return failure(
            EnvConfigErrorCode::Malformed,
            "config.reward_team must be home or away",
        );
    };
    let objectives = match &config.objective_channels {
        None => env_reward::DEFAULT_OBJECTIVES.to_vec(),
        Some(names) => env_reward::validate_selection(
            &raw_channel_ids(names),
            env_reward::EnvRewardRole::Objective,
        )
        .map_err(|e| {
            EnvConfigError::new(
                EnvConfigErrorCode::Malformed,
                format!("{} ({})", e.message, reward_error_code_name(e.code)),
            )
        })?,
    };
    let shaping = match &config.shaping_channels {
        None => Vec::new(),
        Some(names) => env_reward::validate_selection(
            &raw_channel_ids(names),
            env_reward::EnvRewardRole::Shaping,
        )
        .map_err(|e| {
            EnvConfigError::new(
                EnvConfigErrorCode::Malformed,
                format!("{} ({})", e.message, reward_error_code_name(e.code)),
            )
        })?,
    };
    let max_episode_ticks = match config.max_episode_ticks {
        None => None,
        Some(value) => {
            if !is_integer(value) || value < 1.0 {
                return failure(
                    EnvConfigErrorCode::Malformed,
                    "config.max_episode_ticks must be a positive integer",
                );
            }
            Some(value as i64)
        }
    };

    let content = config
        .content
        .clone()
        .unwrap_or_else(|| DEFAULT_CONTENT.to_string());
    let source = config.source.clone().unwrap_or_else(|| build.clone());
    let fixture = config.fixture.clone().unwrap_or_else(|| {
        let home_formation = config
            .home_formation
            .clone()
            .unwrap_or_else(|| home_team.formation.to_string());
        let away_formation = config
            .away_formation
            .clone()
            .unwrap_or_else(|| away_team.formation.to_string());
        format!(
            "{home_team_id}-v-{away_team_id};{home_formation}-v-{away_formation};{home_tactic_id}-v-{away_tactic_id};combat={combat}"
        )
    });

    Ok(EnvConfig {
        version: VERSION,
        build,
        source,
        content,
        fixture,
        seed,
        duration,
        max_goals,
        field,
        home_team_id,
        away_team_id,
        home_formation: config.home_formation.clone(),
        away_formation: config.away_formation.clone(),
        home_tactic_id,
        away_tactic_id,
        combat,
        observation_profile: profile,
        slot_sources: sources,
        reward_team,
        objective_channels: objectives,
        shaping_channels: shaping,
        max_episode_ticks,
    })
}

/// The fixture tables `sim::match` consumes, resolved from an already
/// normalized config.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvResolvedFixture {
    /// Home team.
    pub home: gc_data::teams::TeamData,
    /// Away team.
    pub away: gc_data::teams::TeamData,
    /// Home tactic.
    pub home_tactic: gc_data::tactics::TacticData,
    /// Away tactic.
    pub away_tactic: gc_data::tactics::TacticData,
    /// Every authored player, keyed by id.
    pub players_by_id: indexmap::IndexMap<&'static str, gc_data::players::PlayerData>,
}

/// Resolve authored ids into the tables `sim::match` consumes. Fixture
/// content is read-only here: the returned tables are the canonical data
/// entries.
///
/// # Panics
///
/// Panics if `config` names ids `normalize` did not already validate — an
/// invariant violation (AGENTS.md §7), since every `EnvConfig` is meant to
/// originate from [`normalize`].
#[must_use]
pub fn resolve(config: &EnvConfig) -> EnvResolvedFixture {
    let home =
        *gc_data::teams::get(&config.home_team_id).expect("normalize validates home_team_id");
    let away =
        *gc_data::teams::get(&config.away_team_id).expect("normalize validates away_team_id");
    let home_tactic =
        *gc_data::tactics::get(&config.home_tactic_id).expect("normalize validates home_tactic_id");
    let away_tactic =
        *gc_data::tactics::get(&config.away_tactic_id).expect("normalize validates away_tactic_id");
    let players_by_id = gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect();
    EnvResolvedFixture {
        home,
        away,
        home_tactic,
        away_tactic,
        players_by_id,
    }
}

/// The canonical config digest recorded in the tape identity and the
/// manifest. Two runs that agree on this string plus build/content/tuning/
/// seed agree on everything the environment can decide.
///
/// Unlike `fnv1a64`/`diagnostics_schema` (ARCHITECTURE.md §1.2), this digest is not
/// on the list of hashes two independently-implemented clients must agree on
/// bit-for-bit — it is a Rust-internal manifest string, so it need only be
/// stable and sensitive to every knob within this engine.
#[must_use]
pub fn digest(config: &EnvConfig) -> String {
    let mut parts = vec![
        format!("env_config={}", config.version),
        format!("field={}x{}", config.field.w, config.field.h),
        format!("duration={}", config.duration),
        format!("max_goals={}", config.max_goals),
        format!("tick_rate={}", fixed_clock::TICK_RATE),
        format!("home={}", config.home_team_id),
        format!("away={}", config.away_team_id),
        format!(
            "home_formation={}",
            config.home_formation.as_deref().unwrap_or("default")
        ),
        format!(
            "away_formation={}",
            config.away_formation.as_deref().unwrap_or("default")
        ),
        format!("home_tactic={}", config.home_tactic_id),
        format!("away_tactic={}", config.away_tactic_id),
        format!("combat={}", config.combat),
        format!(
            "profile={}",
            observation_profile_name(config.observation_profile)
        ),
        format!("reward_team={}", env_side_name(config.reward_team)),
        format!(
            "max_episode_ticks={}",
            config
                .max_episode_ticks
                .map(|ticks| ticks.to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
    ];
    let slots: Vec<String> = config
        .slot_sources
        .iter()
        .enumerate()
        .map(|(position, source)| {
            let mut label = format!("{}:{}", position + 1, slot_source_kind_name(source.kind));
            if let Some(seed) = source.seed {
                label.push_str(&format!("@{seed}"));
            }
            if let Some(policy_id) = &source.policy_id {
                label.push_str(&format!("#{policy_id}"));
            }
            label
        })
        .collect();
    parts.push(format!("slots={}", slots.join(",")));
    parts.push(format!(
        "objectives={}",
        config
            .objective_channels
            .iter()
            .map(|&id| env_reward::channel_id_name(id))
            .collect::<Vec<_>>()
            .join(",")
    ));
    parts.push(format!(
        "shaping={}",
        config
            .shaping_channels
            .iter()
            .map(|&id| env_reward::channel_id_name(id))
            .collect::<Vec<_>>()
            .join(",")
    ));
    parts.join(";")
}
