//! Named learning-environment reward channels.
//!
//! There is no implicit scalar reward. Every channel is registered with an
//! explicit role: `objective` channels are competition targets, `shaping`
//! channels must be preregistered and are always reported separately so an
//! ablation is a subtraction rather than a rerun, and `diagnostic` channels
//! are evaluation data that may never be optimized. The #128 fun-proxy
//! metric family is a diagnostic channel: selecting it as an objective or
//! shaping term is a validation error, and no channel may be named `fun`.
//!
//! This module has no `sim` dependencies of its own: the fixture side
//! ([`EnvSide`]) and confirmed combat verdict ([`CombatContactResult`])
//! types it references are small closed sets duplicated locally rather than
//! imported from `input_frame`/`combat`, keeping this module's dependency
//! surface minimal.

use indexmap::IndexMap;

/// The env reward module version.
pub const VERSION: i64 = 1;

/// The banned reward name from #138: predicted human experience is
/// evaluation evidence, never an optimization target. No [`EnvRewardChannelId`]
/// may spell this name; enforced by construction since the id is a closed
/// enum, and checked by name in the registry self-test.
pub const FORBIDDEN_CHANNEL_NAME: &str = "fun";

/// The #128 fun-proxy metric family: diagnostic, evaluation-only.
pub const DIAGNOSTIC_METRIC_CHANNEL: EnvRewardChannelId =
    EnvRewardChannelId::ExperienceProxyMetrics;

/// The default objective selection: the sparse match outcome only.
pub const DEFAULT_OBJECTIVES: [EnvRewardChannelId; 1] = [EnvRewardChannelId::MatchOutcome];

const SHOT_EVENT_KINDS: [&str; 4] = ["shot", "header", "volley", "bicycle"];

/// A fixture side. Declared locally rather than reusing
/// [`crate::input_frame::Team`], keeping this module's dependency surface
/// minimal — it checks side membership without needing `input_frame`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnvSide {
    /// Home side.
    Home,
    /// Away side.
    Away,
}

/// A confirmed combat contact verdict. Declared locally rather than reusing
/// [`crate::combat_snapshot::CombatContactResult`], keeping this module's
/// dependency surface minimal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatContactResult {
    /// Landed cleanly.
    Hit,
    /// Landed past a partial guard.
    Extended,
    /// Blocked by a guard.
    Guarded,
    /// No-effect due to immunity frames.
    Immune,
    /// Superseded by a later, higher-priority action.
    Superseded,
}

/// A reward channel's declared role in an episode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvRewardRole {
    /// A competition target.
    Objective,
    /// A preregistered, separately-reported shaping term.
    Shaping,
    /// Evaluation data that may never be optimized.
    Diagnostic,
}

fn role_name(role: EnvRewardRole) -> &'static str {
    match role {
        EnvRewardRole::Objective => "objective",
        EnvRewardRole::Shaping => "shaping",
        EnvRewardRole::Diagnostic => "diagnostic",
    }
}

/// The closed set of registered reward channel ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnvRewardChannelId {
    /// Sparse +1 win / -1 loss / 0 draw, paid once when the match ends.
    MatchOutcome,
    /// +1 per goal scored by the reward team during the step.
    GoalScored,
    /// -1 per goal conceded by the reward team during the step.
    GoalConceded,
    /// Change in goal difference from the reward team's perspective.
    GoalDifferenceDelta,
    /// +1 when ball ownership transitions to the reward team, -1 when lost.
    PossessionGain,
    /// +1 per confirmed shot, header, volley, or bicycle strike by the
    /// reward team.
    ShotAttempt,
    /// +1 per unguarded combat contact landed by the reward team.
    EquipmentContact,
    /// The #128 fun-proxy metric family, returned as evaluation data only.
    ExperienceProxyMetrics,
}

/// The channel id's canonical string form, as it appears on the wire (a
/// config digest, a serialized selection).
#[must_use]
pub fn channel_id_name(id: EnvRewardChannelId) -> &'static str {
    match id {
        EnvRewardChannelId::MatchOutcome => "match_outcome",
        EnvRewardChannelId::GoalScored => "goal_scored",
        EnvRewardChannelId::GoalConceded => "goal_conceded",
        EnvRewardChannelId::GoalDifferenceDelta => "goal_difference_delta",
        EnvRewardChannelId::PossessionGain => "possession_gain",
        EnvRewardChannelId::ShotAttempt => "shot_attempt",
        EnvRewardChannelId::EquipmentContact => "equipment_contact",
        EnvRewardChannelId::ExperienceProxyMetrics => "experience_proxy_metrics",
    }
}

/// The reverse of [`channel_id_name`].
#[must_use]
pub fn channel_id_from_name(name: &str) -> Option<EnvRewardChannelId> {
    ALL_CHANNEL_IDS
        .into_iter()
        .find(|id| channel_id_name(*id) == name)
}

const ALL_CHANNEL_IDS: [EnvRewardChannelId; 8] = [
    EnvRewardChannelId::MatchOutcome,
    EnvRewardChannelId::GoalScored,
    EnvRewardChannelId::GoalConceded,
    EnvRewardChannelId::GoalDifferenceDelta,
    EnvRewardChannelId::PossessionGain,
    EnvRewardChannelId::ShotAttempt,
    EnvRewardChannelId::EquipmentContact,
    EnvRewardChannelId::ExperienceProxyMetrics,
];

/// One registered reward channel's declared identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvRewardChannel {
    /// The channel's id.
    pub id: EnvRewardChannelId,
    /// The channel's declared role.
    pub role: EnvRewardRole,
    /// False for diagnostics: never a training target.
    pub optimizable: bool,
    /// Shaping terms are declared here, not invented per run.
    pub preregistered: bool,
    /// Human-readable description.
    pub description: &'static str,
}

/// Every registered reward channel, in canonical order.
pub const CHANNELS: [EnvRewardChannel; 8] = [
    EnvRewardChannel {
        id: EnvRewardChannelId::MatchOutcome,
        role: EnvRewardRole::Objective,
        optimizable: true,
        preregistered: true,
        description: "Sparse +1 win / -1 loss / 0 draw, paid once when the match ends.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::GoalScored,
        role: EnvRewardRole::Objective,
        optimizable: true,
        preregistered: true,
        description: "+1 per goal scored by the reward team during the step.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::GoalConceded,
        role: EnvRewardRole::Objective,
        optimizable: true,
        preregistered: true,
        description: "-1 per goal conceded by the reward team during the step.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::GoalDifferenceDelta,
        role: EnvRewardRole::Objective,
        optimizable: true,
        preregistered: true,
        description: "Change in goal difference from the reward team's perspective.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::PossessionGain,
        role: EnvRewardRole::Shaping,
        optimizable: true,
        preregistered: true,
        description: "+1 when ball ownership transitions to the reward team, -1 when it is lost.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::ShotAttempt,
        role: EnvRewardRole::Shaping,
        optimizable: true,
        preregistered: true,
        description: "+1 per confirmed shot, header, volley, or bicycle strike by the reward team.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::EquipmentContact,
        role: EnvRewardRole::Shaping,
        optimizable: true,
        preregistered: true,
        description: "+1 per unguarded combat contact landed by the reward team.",
    },
    EnvRewardChannel {
        id: EnvRewardChannelId::ExperienceProxyMetrics,
        role: EnvRewardRole::Diagnostic,
        optimizable: false,
        preregistered: true,
        description: "The #128 fun-proxy metric family, returned as evaluation data only.",
    },
];

/// Look up a registered channel's full declaration by id.
#[must_use]
pub fn channel(id: EnvRewardChannelId) -> EnvRewardChannel {
    CHANNELS
        .into_iter()
        .find(|channel| channel.id == id)
        .expect("every EnvRewardChannelId is registered in CHANNELS")
}

/// Failure reasons an env-reward operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvRewardErrorCode {
    /// The data violates a structural or type invariant.
    Malformed,
    /// A channel id names something outside the registry.
    UnknownChannel,
    /// A channel id is registered, but not for the requested role.
    WrongRole,
    /// A channel id was selected twice for the same role.
    DuplicateChannel,
}

/// An expected, recoverable env-reward failure (ARCHITECTURE.md §3 rule 5): a channel
/// selection comes from a run config (external input), so a bad selection is
/// a recoverable rejection with a machine-readable reason, never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvRewardError {
    /// Machine-readable failure reason.
    pub code: EnvRewardErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl EnvRewardError {
    fn new(code: EnvRewardErrorCode, message: impl Into<String>) -> Self {
        EnvRewardError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnvRewardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EnvRewardError {}

/// Result alias for fallible env-reward operations.
pub type Result<T> = std::result::Result<T, EnvRewardError>;

fn failure<T>(code: EnvRewardErrorCode, message: impl Into<String>) -> Result<T> {
    Err(EnvRewardError::new(code, message))
}

// ---------------------------------------------------------------------------
// Raw, not-yet-validated external input.
//
// A channel selection arrives from a run config, so `validate_selection` and
// `validate` accept a genuinely untyped shape rather than assuming it is
// already well-formed.
// ---------------------------------------------------------------------------

/// One raw, not-yet-validated channel id entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawChannelIdEntry {
    /// A string id — the only legal shape.
    Str(String),
    /// Anything else.
    Other,
}

/// A raw, not-yet-validated channel selection list for one role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawChannelIds {
    /// An ordered list of raw ids.
    List(Vec<RawChannelIdEntry>),
    /// Anything that is not a list.
    Other,
}

/// Validate an authored channel selection for one role. Selections come from
/// a run config (external input), so failures are recoverable returns.
pub fn validate_selection(
    ids: &RawChannelIds,
    role: EnvRewardRole,
) -> Result<Vec<EnvRewardChannelId>> {
    let entries = match ids {
        RawChannelIds::List(entries) => entries,
        RawChannelIds::Other => {
            return failure(
                EnvRewardErrorCode::Malformed,
                format!("{} channel selection must be an array", role_name(role)),
            );
        }
    };
    let mut seen: Vec<EnvRewardChannelId> = Vec::new();
    let mut copied: Vec<EnvRewardChannelId> = Vec::new();
    for entry in entries {
        let RawChannelIdEntry::Str(id) = entry else {
            return failure(
                EnvRewardErrorCode::Malformed,
                format!("{} channel id must be a string", role_name(role)),
            );
        };
        let Some(channel_id) = channel_id_from_name(id) else {
            return failure(
                EnvRewardErrorCode::UnknownChannel,
                format!("unknown reward channel: {id}"),
            );
        };
        let declared = channel(channel_id);
        if declared.role != role {
            return failure(
                EnvRewardErrorCode::WrongRole,
                format!(
                    "reward channel {} is a {} channel, not {}",
                    id,
                    role_name(declared.role),
                    role_name(role)
                ),
            );
        }
        if seen.contains(&channel_id) {
            return failure(
                EnvRewardErrorCode::DuplicateChannel,
                format!("reward channel {id} is selected twice"),
            );
        }
        seen.push(channel_id);
        copied.push(channel_id);
    }
    Ok(copied)
}

/// A raw, not-yet-validated top-level reward-channel selection, exactly as
/// received from run configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawSelection {
    /// A table naming only `objectives`/`shaping`.
    Table(RawSelectionTable),
    /// Anything that is not a table.
    Other,
}

/// The two legal fields of a [`RawSelection::Table`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RawSelectionTable {
    /// The raw `objectives` field, when present.
    pub objectives: Option<RawChannelIds>,
    /// The raw `shaping` field, when present.
    pub shaping: Option<RawChannelIds>,
    /// True when the caller's table carried a field other than the two
    /// above.
    pub has_unknown_field: bool,
}

/// A validated, complete reward-channel selection.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EnvRewardSelection {
    /// Selected objective channel ids, in caller order.
    pub objectives: Vec<EnvRewardChannelId>,
    /// Selected shaping channel ids, in caller order.
    pub shaping: Vec<EnvRewardChannelId>,
}

/// Validate a caller's whole reward-channel selection.
pub fn validate(selection: &RawSelection) -> Result<EnvRewardSelection> {
    let table = match selection {
        RawSelection::Table(table) => table,
        RawSelection::Other => {
            return failure(
                EnvRewardErrorCode::Malformed,
                "reward selection must be a table",
            );
        }
    };
    if table.has_unknown_field {
        return failure(
            EnvRewardErrorCode::Malformed,
            "reward selection has unknown field",
        );
    }
    let empty = RawChannelIds::List(Vec::new());
    let objectives = validate_selection(
        table.objectives.as_ref().unwrap_or(&empty),
        EnvRewardRole::Objective,
    )?;
    let shaping = validate_selection(
        table.shaping.as_ref().unwrap_or(&empty),
        EnvRewardRole::Shaping,
    )?;
    Ok(EnvRewardSelection {
        objectives,
        shaping,
    })
}

// ---------------------------------------------------------------------------
// Evaluation.
// ---------------------------------------------------------------------------

/// Home/away goal counts at one instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EnvRewardScore {
    /// Home goals.
    pub home: i64,
    /// Away goals.
    pub away: i64,
}

/// One confirmed match/combat event, as far as reward evaluation needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvRewardEvent {
    /// `MatchEventKind` or `CombatEventKind`, as a bare string: this field
    /// can hold either kind depending on the event's source, and a plain
    /// `String` avoids inventing a third enum that is just a union of the
    /// other two.
    pub kind: String,
    /// Acting side, when the event has an identified actor.
    pub team: Option<EnvSide>,
    /// Confirmed combat contact verdict, when applicable.
    pub result: Option<CombatContactResult>,
}

/// One already-simulated transition, expressed from one team's perspective.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvRewardTransition {
    /// Perspective the reward is expressed from.
    pub team: EnvSide,
    /// Score immediately before the simulated ticks.
    pub score_before: EnvRewardScore,
    /// Score immediately after the simulated ticks.
    pub score_after: EnvRewardScore,
    /// Ball owner before the simulated ticks, if any.
    pub owner_team_before: Option<EnvSide>,
    /// Ball owner after the simulated ticks, if any.
    pub owner_team_after: Option<EnvSide>,
    /// Confirmed events of the simulated ticks.
    pub events: Vec<EnvRewardEvent>,
    /// True only on the tick the match itself ended.
    pub terminated: bool,
}

/// One evaluated reward: every selected channel, plus the objective/shaping
/// subtotals and their sum.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvRewardResult {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Perspective the reward is expressed from.
    pub team: EnvSide,
    /// Per-channel objective values, only for selected channels.
    pub objectives: IndexMap<EnvRewardChannelId, f64>,
    /// Per-channel shaping values, only for selected channels.
    pub shaping: IndexMap<EnvRewardChannelId, f64>,
    /// Sum of `objectives`.
    pub objective_total: f64,
    /// Sum of `shaping`.
    pub shaping_total: f64,
    /// `objective_total + shaping_total`, for callers that want one scalar.
    pub total: f64,
}

fn perspective(team: EnvSide, score: &EnvRewardScore) -> (i64, i64) {
    match team {
        EnvSide::Home => (score.home, score.away),
        EnvSide::Away => (score.away, score.home),
    }
}

fn goal_deltas(transition: &EnvRewardTransition) -> (i64, i64) {
    let (own_before, opponent_before) = perspective(transition.team, &transition.score_before);
    let (own_after, opponent_after) = perspective(transition.team, &transition.score_after);
    (own_after - own_before, opponent_after - opponent_before)
}

fn match_outcome(transition: &EnvRewardTransition) -> f64 {
    if !transition.terminated {
        return 0.0;
    }
    let (own, opponent) = perspective(transition.team, &transition.score_after);
    if own > opponent {
        1.0
    } else if own < opponent {
        -1.0
    } else {
        0.0
    }
}

fn possession_gain(transition: &EnvRewardTransition) -> f64 {
    let before = transition.owner_team_before == Some(transition.team);
    let after = transition.owner_team_after == Some(transition.team);
    if after && !before {
        1.0
    } else if before && !after {
        -1.0
    } else {
        0.0
    }
}

fn shot_attempt(transition: &EnvRewardTransition) -> f64 {
    let mut total = 0.0;
    for event in &transition.events {
        if SHOT_EVENT_KINDS.contains(&event.kind.as_str()) && event.team == Some(transition.team) {
            total += 1.0;
        }
    }
    total
}

fn equipment_contact(transition: &EnvRewardTransition) -> f64 {
    let mut total = 0.0;
    for event in &transition.events {
        if event.kind == "contact"
            && event.team == Some(transition.team)
            && event.result == Some(CombatContactResult::Hit)
        {
            total += 1.0;
        }
    }
    total
}

fn evaluate_channel(id: EnvRewardChannelId, transition: &EnvRewardTransition) -> f64 {
    match id {
        EnvRewardChannelId::MatchOutcome => match_outcome(transition),
        EnvRewardChannelId::GoalScored => {
            let (own, _) = goal_deltas(transition);
            own as f64
        }
        EnvRewardChannelId::GoalConceded => {
            let (_, opponent) = goal_deltas(transition);
            -(opponent as f64)
        }
        EnvRewardChannelId::GoalDifferenceDelta => {
            let (own, opponent) = goal_deltas(transition);
            (own - opponent) as f64
        }
        EnvRewardChannelId::PossessionGain => possession_gain(transition),
        EnvRewardChannelId::ShotAttempt => shot_attempt(transition),
        EnvRewardChannelId::EquipmentContact => equipment_contact(transition),
        EnvRewardChannelId::ExperienceProxyMetrics => {
            unreachable!(
                "experience_proxy_metrics is diagnostic; validate_selection never admits it \
                 into an objective/shaping selection"
            )
        }
    }
}

/// Score one already-simulated transition. Objectives and shaping are summed
/// separately so an ablation report can drop the shaping column without
/// re-running the episode.
///
/// `transition` and `selection` are already-typed values, not raw external
/// input; a `selection` naming an unregistered or wrong-role channel is a
/// programmer error caught by `assert` (AGENTS.md §7), not a recoverable
/// rejection — [`validate_selection`] is what external callers must run
/// first.
#[must_use]
pub fn evaluate(
    transition: &EnvRewardTransition,
    selection: &EnvRewardSelection,
) -> EnvRewardResult {
    let mut objectives = IndexMap::new();
    let mut shaping = IndexMap::new();
    let mut objective_total = 0.0;
    let mut shaping_total = 0.0;
    for &id in &selection.objectives {
        let declared = channel(id);
        assert_eq!(
            declared.role,
            EnvRewardRole::Objective,
            "channel {id:?} is not an objective"
        );
        let value = evaluate_channel(id, transition);
        objectives.insert(id, value);
        objective_total += value;
    }
    for &id in &selection.shaping {
        let declared = channel(id);
        assert_eq!(
            declared.role,
            EnvRewardRole::Shaping,
            "channel {id:?} is not shaping"
        );
        let value = evaluate_channel(id, transition);
        shaping.insert(id, value);
        shaping_total += value;
    }
    EnvRewardResult {
        version: VERSION,
        team: transition.team,
        objectives,
        shaping,
        objective_total,
        shaping_total,
        total: objective_total + shaping_total,
    }
}
