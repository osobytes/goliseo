//! Pure stable-event timeline for rollback presentation and confirmed
//! consumers. Identity is derived from simulation causality and never enters
//! [`crate::match_snapshot::MatchState`].
//!
//! ## `RollbackEventTickOutput` — a narrow adapter, not `RollbackTickOutput`
//!
//! This module's step input is typed against [`RollbackEventTickOutput`]
//! rather than [`crate::rollback_session::RollbackTickOutput`] directly.
//! Rather than take on a dependency on a `MatchState`-shaped type this
//! module does not own, [`RollbackEventTickOutput`] declares only the
//! fields this module actually reads (`tick`, `start_boundary`,
//! `end_boundary`, `events`, `combat_events`, `state`, `finished`) —
//! `RollbackTickOutput.input` is part of that type's shape but is never
//! read here, so it has no field in this narrow view.
//!
//! ## Deep copies are ambient, not threaded
//!
//! Every payload type here ([`crate::match_snapshot::MatchEvent`],
//! [`crate::combat_snapshot::CombatEvent`], [`RollbackLifecyclePayload`],
//! ...) is an owned Rust value with a real [`Clone`] impl, so passing by
//! value or `.clone()`-ing at a storage boundary already gives an
//! independent copy — there is no reference aliasing to defend against, so
//! no manual deep-copy helper is needed anywhere in this module.
//!
//! ## Canonical signed-zero equality
//!
//! Every payload comparison this module's diffing does needs `0.0` and
//! `-0.0` to compare *unequal* (this is exercised directly: "uses canonical
//! signed-zero equality for payload replacement"), because they are
//! distinct simulation values here even though Rust's `f64` `==` treats
//! them as equal. That comparison goes through a local `same_f64` that
//! matches the sign of `1.0 / x`, the same approach `match_snapshot` uses in
//! its own private helper of the same name (that one is not `pub`, so this
//! module keeps its own copy rather than reaching across the crate for a
//! private item).
//!
//! ## No `%`, and no `is_integer`/dense-array checks
//!
//! This module has no modulo arithmetic, and it has no runtime "is this
//! actually an integer" or "is this array dense" checks: `i64` cannot be
//! fractional and a Rust `&[T]` slice is always dense, so both classes of
//! check would be structurally redundant here (ARCHITECTURE.md §3 rule 7).
//!
//! ## `accounting` is diagnostic, not differential-tested
//!
//! [`accounting`] is a byte counter, but it is presentation-facing
//! bookkeeping — not hashed, not wired, not on the determinism path — and
//! is not covered by differential testing against a reference
//! implementation. It uses a tagging scheme (`n`/`b0`/`b1`/`s<len>:`/
//! `d<len>:`/`t<count>:`) but, since `accounting` only ever reports a total
//! *byte count*, it only needs to reproduce each contribution's length, not
//! the concatenation order the way a hash or wire format would.

use crate::aerial::{AerialOutcome, AerialStyle};
use crate::combat_snapshot::{self, CombatEvent, CombatEventKind, CombatMatchState};
use crate::input_frame;
use crate::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use crate::match_snapshot::{self, ByTeam, MatchEvent, MatchSnapshot, MatchState, Team};
use indexmap::IndexMap;

/// Free-form event identity namespace: `match/<kind>`, `combat/<kind>/<n>`,
/// or `lifecycle/goal`\|`lifecycle/kickoff`\|`lifecycle/full_time`. Those
/// literals are the documented cases, but an arbitrary string is also
/// allowed, so this is not a closed set — an `enum` would be lossy; this
/// stays `String`.
pub type RollbackEventDomain = String;

/// The closed lifecycle vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackLifecycleKind {
    /// A goal was scored.
    Goal,
    /// Restart following a goal.
    Kickoff,
    /// The match ended.
    FullTime,
}

impl RollbackLifecycleKind {
    /// This kind's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            RollbackLifecycleKind::Goal => "goal",
            RollbackLifecycleKind::Kickoff => "kickoff",
            RollbackLifecycleKind::FullTime => "full_time",
        }
    }
}

/// A derived lifecycle transition's payload.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackLifecyclePayload {
    /// The lifecycle kind.
    pub kind: RollbackLifecycleKind,
    /// The team this transition is attributed to, if any.
    pub team: Option<Team>,
    /// The score at this transition.
    pub score: ByTeam<i64>,
}

/// A wrapped event's payload, discriminated by its origin.
#[derive(Clone, Debug, PartialEq)]
pub enum RollbackEventPayload {
    /// A soccer match event.
    Match(MatchEvent),
    /// A combat event.
    Combat(CombatEvent),
    /// A derived lifecycle transition.
    Lifecycle(RollbackLifecyclePayload),
}

/// One identity-stable event on the timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackWrappedEvent {
    /// Stable identity: `event_id(tick, domain, ordinal)`.
    pub id: String,
    /// The input tick this event is attributed to.
    pub tick: i64,
    /// This event's identity namespace.
    pub domain: RollbackEventDomain,
    /// This event's position within its domain (or a fixed identity slot).
    pub ordinal: i64,
    /// The wrapped payload.
    pub payload: RollbackEventPayload,
}

/// One event replaced in place: same `id`, different `payload`.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventReplacement {
    /// The event before this correction.
    pub before: RollbackWrappedEvent,
    /// The event after this correction.
    pub after: RollbackWrappedEvent,
}

/// The outcome of one [`apply`] call.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RollbackEventDiff {
    /// Events with no prior identity in the replaced interval.
    pub added: Vec<RollbackWrappedEvent>,
    /// Events whose identity no longer appears.
    pub revoked: Vec<RollbackWrappedEvent>,
    /// Events whose identity survived with a changed payload.
    pub replaced: Vec<RollbackEventReplacement>,
}

/// A [`RollbackEventTimeline`]'s lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackEventsStatus {
    /// Accepting further [`apply`]/[`confirm`] calls.
    Active,
    /// Permanently stalled: too many unconfirmed ticks accumulated.
    UnconfirmedWindowExceeded,
}

/// Failure reasons a [`RollbackEventTimeline`] operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackEventsErrorCode {
    /// The unconfirmed speculative window was exceeded.
    UnconfirmedWindowExceeded,
}

/// An expected, recoverable [`RollbackEventTimeline`] failure (ARCHITECTURE.md
/// §3 rule 5): the caller is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackEventsError {
    /// Machine-readable failure reason.
    pub code: RollbackEventsErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl RollbackEventsError {
    fn new(code: RollbackEventsErrorCode, message: impl Into<String>) -> Self {
        RollbackEventsError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RollbackEventsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RollbackEventsError {}

/// Result alias for fallible rollback-event operations.
pub type Result<T> = std::result::Result<T, RollbackEventsError>;

fn failure<T>(code: RollbackEventsErrorCode, message: impl Into<String>) -> Result<T> {
    Err(RollbackEventsError::new(code, message))
}

/// A compact, confirmed-safe view of match state carried between steps.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackConfirmedStateView {
    /// Score at this boundary.
    pub score: ByTeam<i64>,
    /// Seconds remaining at this boundary.
    pub time_left: f64,
    /// Whether the match had finished at this boundary.
    pub finished: bool,
    /// The ball owner's stable id, from the timeline's initial roster.
    pub owner_id: Option<String>,
    /// The ball owner's team, from the timeline's initial roster.
    pub owner_team: Option<Team>,
}

/// One fixture player's identity, fixed at [`new`].
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventPlayerIdentity {
    /// Stable player id.
    pub id: String,
    /// Fixture side.
    pub team: Team,
}

/// A narrow adapter for [`crate::rollback_session::RollbackTickOutput`]. See
/// the module doc comment: this carries only the fields this module reads.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventTickOutput {
    /// Causal input tick.
    pub tick: i64,
    /// Always equal to `tick`.
    pub start_boundary: i64,
    /// Always `tick + 1`.
    pub end_boundary: i64,
    /// This tick's soccer events.
    pub events: Vec<MatchEvent>,
    /// This tick's combat events, present exactly when combat is active.
    pub combat_events: Option<Vec<CombatEvent>>,
    /// Compact post-step state.
    pub state: RollbackOutputStateView,
    /// Whether the match finished on this tick.
    pub finished: bool,
}

/// The compact state view carried by [`RollbackEventTickOutput`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackOutputStateView {
    /// Score at this boundary.
    pub score: ByTeam<i64>,
    /// Seconds remaining at this boundary.
    pub time_left: f64,
    /// Whether the match had finished at this boundary.
    pub finished: bool,
}

/// One [`apply`] call's input row: a candidate output plus the canonical
/// post-step boundary it produced.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventStepInput {
    /// The candidate tick output.
    pub output: RollbackEventTickOutput,
    /// The canonical post-step boundary snapshot.
    pub snapshot: MatchSnapshot,
}

/// One tick's retained speculative record.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventStep {
    /// Causal input tick.
    pub tick: i64,
    /// Always equal to `tick`.
    pub start_boundary: i64,
    /// Always `tick + 1`.
    pub end_boundary: i64,
    /// Compact post-step state.
    pub state: RollbackConfirmedStateView,
    /// This tick's wrapped soccer events.
    pub match_events: Vec<RollbackWrappedEvent>,
    /// This tick's wrapped combat events, present exactly when combat is
    /// active.
    pub combat_events: Option<Vec<RollbackWrappedEvent>>,
    /// Derived lifecycle transitions for this tick.
    pub lifecycle_events: Vec<RollbackWrappedEvent>,
}

/// Pure stable-event timeline for one rollback client.
///
/// Every field is internal state; use the free functions in this module to
/// read or mutate it. Fields are `pub` (ARCHITECTURE.md §3 rule 6: everything
/// a test touches is `pub`). `steps` is keyed by tick — a sparse key, not a
/// sequential buffer position (ARCHITECTURE.md §3 rule 3), so it is an [`IndexMap`]
/// rather than a fixed-size array, matching `rollback_input_history`'s
/// `authoritative`/`effective`/`records` maps.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventTimeline {
    /// Lifecycle status.
    pub status: RollbackEventsStatus,
    /// Maximum retained unconfirmed ticks before this timeline stalls.
    pub max_unconfirmed_ticks: i64,
    /// The highest tick confirmed so far, or `-1` before any confirmation.
    pub confirmed_tick: i64,
    /// The boundary tick immediately after `confirmed_tick`.
    pub confirmed_boundary: i64,
    /// Compact state as of `confirmed_boundary`.
    pub confirmed_state: RollbackConfirmedStateView,
    /// Every fixture player's identity, fixed at construction.
    pub players: Vec<RollbackEventPlayerIdentity>,
    /// Retained speculative steps, keyed by tick.
    pub steps: IndexMap<i64, RollbackEventStep>,
}

/// A snapshot of a [`RollbackEventTimeline`]'s retained-state shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackEventsDiagnostics {
    /// Lifecycle status.
    pub status: RollbackEventsStatus,
    /// The highest tick confirmed so far, or `-1` before any confirmation.
    pub confirmed_tick: i64,
    /// The boundary tick immediately after `confirmed_tick`.
    pub confirmed_boundary: i64,
    /// Maximum retained unconfirmed ticks before this timeline stalls.
    pub max_unconfirmed_ticks: i64,
    /// Retained speculative step count.
    pub retained_step_count: i64,
    /// Total retained events across every retained step.
    pub retained_event_count: i64,
    /// The oldest retained speculative tick, if any.
    pub oldest_tick: Option<i64>,
    /// The newest retained speculative tick, if any.
    pub latest_tick: Option<i64>,
}

/// The outcome of one [`accounting`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackEventsAccounting {
    /// Exact logical bytes retained in the speculative event window.
    pub retained_step_bytes: i64,
    /// Confirmed presentation history lives in game-layer consumers and is
    /// not rollback history, so this always equals `retained_step_bytes`.
    pub total_bytes: i64,
}

fn same_f64(a: f64, b: f64) -> bool {
    if a != b {
        return false;
    }
    if a == 0.0 && b == 0.0 {
        return (1.0 / a) == (1.0 / b);
    }
    true
}

fn opt_f64_eq(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_f64(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn match_event_eq(a: &MatchEvent, b: &MatchEvent) -> bool {
    a.kind == b.kind
        && same_f64(a.x, b.x)
        && same_f64(a.y, b.y)
        && a.player == b.player
        && a.save_style == b.save_style
        && a.style == b.style
        && a.outcome == b.outcome
        && a.jumping == b.jumping
        && opt_f64_eq(a.difficulty, b.difficulty)
        && a.shot_type == b.shot_type
        && a.keeper_state == b.keeper_state
        && opt_f64_eq(a.keeper_depth, b.keeper_depth)
        && a.on_target == b.on_target
}

fn match_events_eq(a: &[MatchEvent], b: &[MatchEvent]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| match_event_eq(x, y))
}

fn combat_event_eq(a: &CombatEvent, b: &CombatEvent) -> bool {
    a.kind == b.kind
        && a.tick == b.tick
        && a.family_id == b.family_id
        && a.source_index == b.source_index
        && a.target_index == b.target_index
        && a.source_sequence == b.source_sequence
        && a.result == b.result
        && a.outcome == b.outcome
        && a.reason == b.reason
        && a.terminal == b.terminal
        && same_f64(a.x, b.x)
        && same_f64(a.y, b.y)
        && a.interruption_ticks == b.interruption_ticks
        && opt_f64_eq(a.displacement_px, b.displacement_px)
}

fn combat_events_eq(a: &[CombatEvent], b: &[CombatEvent]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| combat_event_eq(x, y))
}

fn lifecycle_payload_eq(a: &RollbackLifecyclePayload, b: &RollbackLifecyclePayload) -> bool {
    a.kind == b.kind
        && a.team == b.team
        && a.score.home == b.score.home
        && a.score.away == b.score.away
}

fn payload_eq(a: &RollbackEventPayload, b: &RollbackEventPayload) -> bool {
    match (a, b) {
        (RollbackEventPayload::Match(x), RollbackEventPayload::Match(y)) => match_event_eq(x, y),
        (RollbackEventPayload::Combat(x), RollbackEventPayload::Combat(y)) => combat_event_eq(x, y),
        (RollbackEventPayload::Lifecycle(x), RollbackEventPayload::Lifecycle(y)) => {
            lifecycle_payload_eq(x, y)
        }
        _ => false,
    }
}

fn event_id(tick: i64, domain: &str, ordinal: i64) -> String {
    assert!(
        (0..=input_frame::MAX_TICK).contains(&tick),
        "rollback event tick is outside the input range"
    );
    assert!(domain.len() <= 999, "rollback event domain is too long");
    assert!(
        (1..=9999).contains(&ordinal),
        "rollback event ordinal is outside the ID range"
    );
    format!("{tick:010}|{:03}:{domain}|{ordinal:04}", domain.len())
}

fn wrap_match_events(tick: i64, events: &[MatchEvent]) -> Vec<RollbackWrappedEvent> {
    let mut counts: IndexMap<String, i64> = IndexMap::new();
    events
        .iter()
        .map(|event| {
            let domain = format!("match/{}", event.kind.wire_str());
            let ordinal = counts.get(&domain).copied().unwrap_or(0) + 1;
            counts.insert(domain.clone(), ordinal);
            RollbackWrappedEvent {
                id: event_id(tick, &domain, ordinal),
                tick,
                domain,
                ordinal,
                payload: RollbackEventPayload::Match(event.clone()),
            }
        })
        .collect()
}

/// An encounter row is keyed by the sequence it belongs to. A rejected
/// request opens no encounter and has no sequence, so its domain carries the
/// `0` that readers already interpret as "this source has no sequence", and
/// its identity comes from the requesting player instead.
///
/// The player index is the ordinal rather than another domain segment for
/// two reasons. A third numeric segment is read as a source sequence
/// downstream, and a player index is not one. And a running per-domain
/// counter would not be stable: a resimulation that refuses a different set
/// of players would renumber the rows that survived, so one player's cue
/// would be rewritten into another player's. Each player is offered its
/// press exactly once per tick, so its index is unique within the tick and
/// unchanged by whoever else was refused.
///
/// The typed reason deliberately stays out of the key. The same player
/// refused on the same tick for a different reason is one corrected row, not
/// a revoked cue plus a new one.
fn combat_identity(event: &CombatEvent) -> (String, Option<i64>) {
    if event.kind == CombatEventKind::RequestRejected {
        let player_index = event
            .source_index
            .expect("rollback combat rejection is missing its requesting player");
        return (
            format!("combat/{}/0", event.kind.wire_str()),
            Some(player_index),
        );
    }
    let sequence = event
        .source_sequence
        .expect("rollback combat event is missing its stable source sequence");
    (format!("combat/{}/{sequence}", event.kind.wire_str()), None)
}

fn wrap_combat_events(tick: i64, events: &[CombatEvent]) -> Vec<RollbackWrappedEvent> {
    let mut counts: IndexMap<String, i64> = IndexMap::new();
    events
        .iter()
        .map(|event| {
            let (domain, fixed_ordinal) = combat_identity(event);
            let ordinal = match fixed_ordinal {
                Some(ordinal) => ordinal,
                None => {
                    let next = counts.get(&domain).copied().unwrap_or(0) + 1;
                    counts.insert(domain.clone(), next);
                    next
                }
            };
            RollbackWrappedEvent {
                id: event_id(tick, &domain, ordinal),
                tick,
                domain,
                ordinal,
                payload: RollbackEventPayload::Combat(event.clone()),
            }
        })
        .collect()
}

fn lifecycle_event(
    tick: i64,
    domain: &str,
    payload: RollbackLifecyclePayload,
) -> RollbackWrappedEvent {
    RollbackWrappedEvent {
        id: event_id(tick, domain, 1),
        tick,
        domain: domain.to_string(),
        ordinal: 1,
        payload: RollbackEventPayload::Lifecycle(payload),
    }
}

fn derive_lifecycle_events(
    tick: i64,
    pre: &RollbackConfirmedStateView,
    post: &RollbackConfirmedStateView,
) -> Vec<RollbackWrappedEvent> {
    let score = post.score;
    let mut events = Vec::new();
    let scoring_team = if post.score.home == pre.score.home + 1 && post.score.away == pre.score.away
    {
        Some(Team::Home)
    } else if post.score.away == pre.score.away + 1 && post.score.home == pre.score.home {
        Some(Team::Away)
    } else {
        assert!(
            post.score.home == pre.score.home && post.score.away == pre.score.away,
            "rollback lifecycle score transition must contain at most one goal"
        );
        None
    };

    if let Some(team) = scoring_team {
        events.push(lifecycle_event(
            tick,
            "lifecycle/goal",
            RollbackLifecyclePayload {
                kind: RollbackLifecycleKind::Goal,
                team: Some(team),
                score,
            },
        ));
        if !post.finished {
            let kickoff_team = match team {
                Team::Home => Team::Away,
                Team::Away => Team::Home,
            };
            events.push(lifecycle_event(
                tick,
                "lifecycle/kickoff",
                RollbackLifecyclePayload {
                    kind: RollbackLifecycleKind::Kickoff,
                    team: Some(kickoff_team),
                    score,
                },
            ));
        }
    }
    if !pre.finished && post.finished {
        events.push(lifecycle_event(
            tick,
            "lifecycle/full_time",
            RollbackLifecyclePayload {
                kind: RollbackLifecycleKind::FullTime,
                team: None,
                score,
            },
        ));
    } else {
        assert!(
            !pre.finished || post.finished,
            "rollback lifecycle cannot return from full time"
        );
    }
    events
}

fn assert_step_coherent(
    output: &RollbackEventTickOutput,
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
) {
    assert_eq!(
        output.tick, output.start_boundary,
        "rollback event output start boundary is invalid"
    );
    assert_eq!(
        output.end_boundary,
        output.tick + 1,
        "rollback event output must advance one tick"
    );
    assert_eq!(
        state.input_tick, output.end_boundary,
        "rollback event snapshot boundary does not match output"
    );
    assert_eq!(
        output.state.score.home, state.score.home,
        "rollback output home score differs"
    );
    assert_eq!(
        output.state.score.away, state.score.away,
        "rollback output away score differs"
    );
    assert!(
        same_f64(output.state.time_left, state.time_left),
        "rollback output time differs"
    );
    assert_eq!(
        output.state.finished, state.finished,
        "rollback output finish differs"
    );
    assert_eq!(
        output.finished, state.finished,
        "rollback output finish flag differs"
    );
    assert!(
        match_events_eq(&output.events, &state.events),
        "rollback output events differ from post-step snapshot"
    );
    match (combat_state, &output.combat_events) {
        (Some(combat), Some(events)) => assert!(
            combat_events_eq(events, &combat.events),
            "rollback output combat events differ from post-step snapshot"
        ),
        (Some(_), None) => panic!("rollback output combat events differ from post-step snapshot"),
        (None, None) => {}
        (None, Some(_)) => panic!("soccer output cannot contain combat events"),
    }
}

fn player_identities(state: &MatchState) -> Vec<RollbackEventPlayerIdentity> {
    state
        .players
        .iter()
        .map(|player| RollbackEventPlayerIdentity {
            id: player.id.clone(),
            team: player.team,
        })
        .collect()
}

fn state_view(
    state: &MatchState,
    players: &[RollbackEventPlayerIdentity],
) -> RollbackConfirmedStateView {
    let owner = state.owner.map(|owner_index| {
        let index = owner_index as usize - 1;
        let owner = players
            .get(index)
            .expect("rollback event owner is outside the initial roster");
        let current = state
            .players
            .get(index)
            .expect("rollback event owner player is missing");
        assert!(
            current.id == owner.id && current.team == owner.team,
            "rollback event owner identity differs from the initial roster"
        );
        owner.clone()
    });
    RollbackConfirmedStateView {
        score: state.score,
        time_left: state.time_left,
        finished: state.finished,
        owner_id: owner.as_ref().map(|owner| owner.id.clone()),
        owner_team: owner.map(|owner| owner.team),
    }
}

fn latest_speculative_tick(timeline: &RollbackEventTimeline) -> i64 {
    timeline
        .steps
        .keys()
        .copied()
        .fold(timeline.confirmed_tick, i64::max)
}

fn state_before(timeline: &RollbackEventTimeline, tick: i64) -> RollbackConfirmedStateView {
    if tick == timeline.confirmed_boundary {
        return timeline.confirmed_state.clone();
    }
    let previous = timeline
        .steps
        .get(&(tick - 1))
        .unwrap_or_else(|| panic!("rollback event step {tick} is missing its previous boundary"));
    assert_eq!(
        previous.end_boundary, tick,
        "rollback event step {tick} has a noncontiguous previous boundary"
    );
    previous.state.clone()
}

fn make_step(
    output: &RollbackEventTickOutput,
    snapshot: &MatchSnapshot,
    before: &RollbackConfirmedStateView,
    players: &[RollbackEventPlayerIdentity],
) -> RollbackEventStep {
    let (canonical_state, canonical_combat) = match_snapshot::restore(snapshot);
    assert_step_coherent(output, &canonical_state, canonical_combat.as_ref());
    let post = state_view(&canonical_state, players);
    let lifecycle_events = derive_lifecycle_events(output.tick, before, &post);
    let combat_events = output
        .combat_events
        .as_ref()
        .map(|events| wrap_combat_events(output.tick, events));
    RollbackEventStep {
        tick: output.tick,
        start_boundary: output.start_boundary,
        end_boundary: output.end_boundary,
        state: post,
        match_events: wrap_match_events(output.tick, &output.events),
        combat_events,
        lifecycle_events,
    }
}

fn ordered_events(
    steps: &IndexMap<i64, RollbackEventStep>,
    from_tick: i64,
    through_tick: i64,
) -> Vec<RollbackWrappedEvent> {
    let mut events = Vec::new();
    for tick in from_tick..=through_tick {
        if let Some(step) = steps.get(&tick) {
            events.extend(step.match_events.iter().cloned());
            if let Some(combat) = &step.combat_events {
                events.extend(combat.iter().cloned());
            }
            events.extend(step.lifecycle_events.iter().cloned());
        }
    }
    events
}

fn diff_events(
    old_events: &[RollbackWrappedEvent],
    new_events: &[RollbackWrappedEvent],
) -> RollbackEventDiff {
    let mut old_by_id: IndexMap<&str, &RollbackWrappedEvent> = IndexMap::new();
    for event in old_events {
        assert!(
            old_by_id.insert(event.id.as_str(), event).is_none(),
            "rollback event identity collision in stale timeline"
        );
    }
    let mut new_by_id: IndexMap<&str, &RollbackWrappedEvent> = IndexMap::new();
    for event in new_events {
        assert!(
            new_by_id.insert(event.id.as_str(), event).is_none(),
            "rollback event identity collision in corrected timeline"
        );
    }

    let mut diff = RollbackEventDiff::default();
    for event in new_events {
        match old_by_id.get(event.id.as_str()) {
            None => diff.added.push(event.clone()),
            Some(old) => {
                if !payload_eq(&old.payload, &event.payload) {
                    diff.replaced.push(RollbackEventReplacement {
                        before: (*old).clone(),
                        after: event.clone(),
                    });
                }
            }
        }
    }
    for event in old_events {
        if !new_by_id.contains_key(event.id.as_str()) {
            diff.revoked.push(event.clone());
        }
    }
    diff
}

/// Construct a fresh timeline. `initial_snapshot` is canonical boundary
/// zero; `max_unconfirmed_ticks` (default
/// [`crate::rollback_input_history::ROLLBACK_WINDOW_TICKS`]) bounds how many
/// unconfirmed speculative ticks may accumulate before the timeline stalls.
#[must_use]
pub fn new(
    initial_snapshot: &MatchSnapshot,
    max_unconfirmed_ticks: Option<i64>,
) -> RollbackEventTimeline {
    let max_unconfirmed_ticks =
        max_unconfirmed_ticks.unwrap_or(crate::rollback_input_history::ROLLBACK_WINDOW_TICKS);
    assert!(
        (1..=crate::rollback_input_history::ROLLBACK_WINDOW_TICKS).contains(&max_unconfirmed_ticks),
        "rollback event unconfirmed window must be a positive bounded integer"
    );
    let (initial_state, _combat_state) = match_snapshot::restore(initial_snapshot);
    assert_eq!(
        initial_state.input_tick, 0,
        "rollback event timeline requires boundary zero"
    );
    let players = player_identities(&initial_state);
    let confirmed_state = state_view(&initial_state, &players);
    RollbackEventTimeline {
        status: RollbackEventsStatus::Active,
        max_unconfirmed_ticks,
        confirmed_tick: -1,
        confirmed_boundary: 0,
        confirmed_state,
        players,
        steps: IndexMap::new(),
    }
}

/// Apply one contiguous or corrective batch of steps covering
/// `replaced_from_tick..=replaced_through_tick`. A normal (non-correcting)
/// call must append exactly one step immediately after the latest
/// speculative tick; a correcting call must replace the complete stale
/// speculative tail (or shorten it only down to a step that reaches full
/// time).
pub fn apply(
    timeline: &mut RollbackEventTimeline,
    replaced_from_tick: i64,
    replaced_through_tick: i64,
    steps: &[RollbackEventStepInput],
) -> Result<RollbackEventDiff> {
    if timeline.status == RollbackEventsStatus::UnconfirmedWindowExceeded {
        return failure(
            RollbackEventsErrorCode::UnconfirmedWindowExceeded,
            "rollback event timeline cannot accept steps after its unconfirmed window",
        );
    }
    assert!(
        replaced_from_tick >= 0,
        "rollback event replacement start must be a non-negative integer"
    );
    assert!(
        replaced_through_tick >= replaced_from_tick,
        "rollback event replacement end must not precede its start"
    );
    assert!(
        replaced_from_tick > timeline.confirmed_tick,
        "rollback events cannot correct an already-confirmed tick"
    );
    assert!(
        !steps.is_empty(),
        "rollback event replacement requires at least one corrected step"
    );

    let latest = latest_speculative_tick(timeline);
    let replacing = timeline.steps.contains_key(&replaced_from_tick);
    if replacing {
        assert_eq!(
            replaced_through_tick, latest,
            "rollback event correction must replace the complete speculative tail"
        );
        for tick in replaced_from_tick..=replaced_through_tick {
            assert!(
                timeline.steps.contains_key(&tick),
                "rollback event correction is missing stale tick {tick}"
            );
        }
    } else {
        assert!(
            replaced_from_tick == latest + 1
                && replaced_through_tick == replaced_from_tick
                && steps.len() == 1,
            "rollback event normal apply must append exactly one contiguous step"
        );
    }

    let mut previous_state = state_before(timeline, replaced_from_tick);
    let old_events = ordered_events(&timeline.steps, replaced_from_tick, replaced_through_tick);

    let mut new_steps: IndexMap<i64, RollbackEventStep> = IndexMap::new();
    for (index, supplied) in steps.iter().enumerate() {
        assert!(
            !previous_state.finished,
            "rollback event timeline cannot apply a step after full time"
        );
        let output = &supplied.output;
        let expected_tick = replaced_from_tick + index as i64;
        assert_eq!(
            output.tick, expected_tick,
            "rollback event corrected step {} is noncontiguous",
            output.tick
        );
        assert!(
            output.tick <= replaced_through_tick,
            "rollback event corrected steps extend beyond the replaced interval"
        );
        let step = make_step(
            output,
            &supplied.snapshot,
            &previous_state,
            &timeline.players,
        );
        previous_state = step.state.clone();
        new_steps.insert(step.tick, step);
    }
    let replaced_count = replaced_through_tick - replaced_from_tick + 1;
    if replacing && (steps.len() as i64) < replaced_count {
        let final_tick = replaced_from_tick + steps.len() as i64 - 1;
        let final_step = new_steps
            .get(&final_tick)
            .expect("just inserted by the loop above");
        assert!(
            final_step.state.finished,
            "rollback event correction may shorten the stale tail only at full time"
        );
    }

    let retained_after = timeline.steps.len() as i64 - if replacing { replaced_count } else { 0 }
        + steps.len() as i64;
    if retained_after > timeline.max_unconfirmed_ticks {
        timeline.status = RollbackEventsStatus::UnconfirmedWindowExceeded;
        return failure(
            RollbackEventsErrorCode::UnconfirmedWindowExceeded,
            format!(
                "rollback event confirmation stalled beyond the {}-tick unconfirmed window",
                timeline.max_unconfirmed_ticks
            ),
        );
    }

    let corrected_through = replaced_from_tick + steps.len() as i64 - 1;
    let new_events = ordered_events(&new_steps, replaced_from_tick, corrected_through);
    let diff = diff_events(&old_events, &new_events);

    for tick in replaced_from_tick..=replaced_through_tick {
        timeline.steps.shift_remove(&tick);
    }
    for (tick, step) in new_steps {
        timeline.steps.insert(tick, step);
    }
    Ok(diff)
}

/// Confirm every step up to and including `confirmed_output_tick`, removing
/// them from the speculative window and returning them in causal order.
/// Passing the already-confirmed tick again returns an empty list.
pub fn confirm(
    timeline: &mut RollbackEventTimeline,
    confirmed_output_tick: i64,
) -> Vec<RollbackEventStep> {
    assert_eq!(
        timeline.status,
        RollbackEventsStatus::Active,
        "rollback event timeline cannot confirm after its unconfirmed window is exceeded"
    );
    assert!(
        confirmed_output_tick >= -1,
        "rollback event confirmation must be an integer at or after -1"
    );
    assert!(
        confirmed_output_tick >= timeline.confirmed_tick,
        "rollback event confirmation cannot move backward"
    );
    if confirmed_output_tick == timeline.confirmed_tick {
        return Vec::new();
    }

    let mut confirmed = Vec::new();
    let mut boundary = timeline.confirmed_boundary;
    let mut final_state = timeline.confirmed_state.clone();
    for tick in (timeline.confirmed_tick + 1)..=confirmed_output_tick {
        let step = timeline
            .steps
            .get(&tick)
            .unwrap_or_else(|| panic!("rollback event confirmation is missing tick {tick}"));
        assert_eq!(
            step.start_boundary, boundary,
            "rollback event confirmation is noncontiguous at tick {tick}"
        );
        confirmed.push(step.clone());
        boundary = step.end_boundary;
        final_state = step.state.clone();
    }

    timeline.confirmed_tick = confirmed_output_tick;
    timeline.confirmed_boundary = boundary;
    timeline.confirmed_state = final_state;
    for tick in (confirmed_output_tick - confirmed.len() as i64 + 1)..=confirmed_output_tick {
        timeline.steps.shift_remove(&tick);
    }
    confirmed
}

/// A snapshot of this timeline's retained-state shape.
#[must_use]
pub fn diagnostics(timeline: &RollbackEventTimeline) -> RollbackEventsDiagnostics {
    let mut oldest: Option<i64> = None;
    let mut latest: Option<i64> = None;
    let mut event_count: i64 = 0;
    for (&tick, step) in &timeline.steps {
        event_count += step.match_events.len() as i64
            + step
                .combat_events
                .as_ref()
                .map_or(0, |events| events.len() as i64)
            + step.lifecycle_events.len() as i64;
        oldest = Some(oldest.map_or(tick, |current| current.min(tick)));
        latest = Some(latest.map_or(tick, |current| current.max(tick)));
    }
    RollbackEventsDiagnostics {
        status: timeline.status,
        confirmed_tick: timeline.confirmed_tick,
        confirmed_boundary: timeline.confirmed_boundary,
        max_unconfirmed_ticks: timeline.max_unconfirmed_ticks,
        retained_step_count: timeline.steps.len() as i64,
        retained_event_count: event_count,
        oldest_tick: oldest,
        latest_tick: latest,
    }
}

fn digits(n: i64) -> i64 {
    n.to_string().len() as i64
}

fn number_len(n: f64) -> i64 {
    let payload_len = match_snapshot::number_bytes(n).len() as i64;
    1 + digits(payload_len) + 1 + payload_len
}

fn bool_len() -> i64 {
    2
}

fn text_len(s: &str) -> i64 {
    let len = s.len() as i64;
    1 + digits(len) + 1 + len
}

fn record_len(fields: &[(&str, i64)]) -> i64 {
    let body: i64 = fields
        .iter()
        .map(|(name, bytes)| text_len(name) + bytes)
        .sum();
    1 + digits(fields.len() as i64) + 1 + body
}

fn array_len(elements: &[i64]) -> i64 {
    let body: i64 = elements
        .iter()
        .enumerate()
        .map(|(index, &bytes)| number_len(index as f64 + 1.0) + bytes)
        .sum();
    1 + digits(elements.len() as i64) + 1 + body
}

// `MatchEvent`'s enum-valued fields are wire-mapped by `match_snapshot`'s
// own private helpers of the same name; those are not `pub`, so
// `accounting` (diagnostic-only, see the module doc comment) keeps a small
// local mirror rather than reaching across the crate for a private item.
fn save_style_wire(v: SaveStyle) -> &'static str {
    match v {
        SaveStyle::Spread => "spread",
        SaveStyle::Central => "central",
        SaveStyle::Stretch => "stretch",
    }
}
fn aerial_style_wire(v: AerialStyle) -> &'static str {
    match v {
        AerialStyle::LegControl => "leg_control",
        AerialStyle::ChestControl => "chest_control",
        AerialStyle::Volley => "volley",
        AerialStyle::Header => "header",
        AerialStyle::Bicycle => "bicycle",
    }
}
fn aerial_outcome_wire(v: AerialOutcome) -> &'static str {
    match v {
        AerialOutcome::Clean => "clean",
        AerialOutcome::Heavy => "heavy",
        AerialOutcome::Miss => "miss",
    }
}
fn keeper_shot_type_wire(v: KeeperShotType) -> &'static str {
    match v {
        KeeperShotType::Ground => "ground",
        KeeperShotType::Chip => "chip",
    }
}
fn keeper_behavior_state_wire(v: KeeperBehaviorState) -> &'static str {
    match v {
        KeeperBehaviorState::Base => "base",
        KeeperBehaviorState::Advance => "advance",
        KeeperBehaviorState::Contain => "contain",
        KeeperBehaviorState::Set => "set",
        KeeperBehaviorState::Retreat => "retreat",
        KeeperBehaviorState::Recover => "recover",
    }
}

fn match_event_len(event: &MatchEvent) -> i64 {
    let mut fields: Vec<(&str, i64)> = vec![
        ("kind", text_len(event.kind.wire_str())),
        ("x", number_len(event.x)),
        ("y", number_len(event.y)),
    ];
    if let Some(v) = &event.player {
        fields.push(("player", text_len(v)));
    }
    if let Some(v) = event.save_style {
        fields.push(("save_style", text_len(save_style_wire(v))));
    }
    if let Some(v) = event.style {
        fields.push(("style", text_len(aerial_style_wire(v))));
    }
    if let Some(v) = event.outcome {
        fields.push(("outcome", text_len(aerial_outcome_wire(v))));
    }
    if event.jumping.is_some() {
        fields.push(("jumping", bool_len()));
    }
    if let Some(v) = event.difficulty {
        fields.push(("difficulty", number_len(v)));
    }
    if let Some(v) = event.shot_type {
        fields.push(("shot_type", text_len(keeper_shot_type_wire(v))));
    }
    if let Some(v) = event.keeper_state {
        fields.push(("keeper_state", text_len(keeper_behavior_state_wire(v))));
    }
    if let Some(v) = event.keeper_depth {
        fields.push(("keeper_depth", number_len(v)));
    }
    if event.on_target.is_some() {
        fields.push(("on_target", bool_len()));
    }
    record_len(&fields)
}

fn combat_event_len(event: &CombatEvent) -> i64 {
    let mut fields: Vec<(&str, i64)> = vec![
        ("kind", text_len(event.kind.wire_str())),
        ("tick", number_len(event.tick as f64)),
        ("x", number_len(event.x)),
        ("y", number_len(event.y)),
    ];
    if let Some(v) = event.family_id {
        fields.push((
            "family_id",
            text_len(combat_snapshot::action_family_wire_id(v)),
        ));
    }
    if let Some(v) = event.source_index {
        fields.push(("source_index", number_len(v as f64)));
    }
    if let Some(v) = event.target_index {
        fields.push(("target_index", number_len(v as f64)));
    }
    if let Some(v) = event.source_sequence {
        fields.push(("source_sequence", number_len(v as f64)));
    }
    if let Some(v) = event.result {
        fields.push(("result", text_len(v.wire_str())));
    }
    if let Some(v) = event.outcome {
        fields.push(("outcome", text_len(v.wire_str())));
    }
    if let Some(v) = event.reason {
        fields.push(("reason", text_len(v.wire_str())));
    }
    if let Some(v) = event.terminal {
        fields.push(("terminal", text_len(v.wire_str())));
    }
    if let Some(v) = event.interruption_ticks {
        fields.push(("interruption_ticks", number_len(v as f64)));
    }
    if let Some(v) = event.displacement_px {
        fields.push(("displacement_px", number_len(v)));
    }
    record_len(&fields)
}

fn lifecycle_payload_len(payload: &RollbackLifecyclePayload) -> i64 {
    let mut fields: Vec<(&str, i64)> = vec![("kind", text_len(payload.kind.wire_str()))];
    if let Some(team) = payload.team {
        fields.push(("team", text_len(team.wire_str())));
    }
    fields.push((
        "score",
        record_len(&[
            ("home", number_len(payload.score.home as f64)),
            ("away", number_len(payload.score.away as f64)),
        ]),
    ));
    record_len(&fields)
}

fn wrapped_event_len(event: &RollbackWrappedEvent) -> i64 {
    let payload_bytes = match &event.payload {
        RollbackEventPayload::Match(e) => match_event_len(e),
        RollbackEventPayload::Combat(e) => combat_event_len(e),
        RollbackEventPayload::Lifecycle(p) => lifecycle_payload_len(p),
    };
    record_len(&[
        ("id", text_len(&event.id)),
        ("tick", number_len(event.tick as f64)),
        ("domain", text_len(&event.domain)),
        ("ordinal", number_len(event.ordinal as f64)),
        ("payload", payload_bytes),
    ])
}

fn state_view_len(state: &RollbackConfirmedStateView) -> i64 {
    let mut fields: Vec<(&str, i64)> = vec![
        (
            "score",
            record_len(&[
                ("home", number_len(state.score.home as f64)),
                ("away", number_len(state.score.away as f64)),
            ]),
        ),
        ("time_left", number_len(state.time_left)),
        ("finished", bool_len()),
    ];
    if let Some(id) = &state.owner_id {
        fields.push(("owner_id", text_len(id)));
    }
    if let Some(team) = state.owner_team {
        fields.push(("owner_team", text_len(team.wire_str())));
    }
    record_len(&fields)
}

fn step_len(step: &RollbackEventStep) -> i64 {
    let match_lens: Vec<i64> = step.match_events.iter().map(wrapped_event_len).collect();
    let lifecycle_lens: Vec<i64> = step
        .lifecycle_events
        .iter()
        .map(wrapped_event_len)
        .collect();
    let mut fields: Vec<(&str, i64)> = vec![
        ("tick", number_len(step.tick as f64)),
        ("start_boundary", number_len(step.start_boundary as f64)),
        ("end_boundary", number_len(step.end_boundary as f64)),
        ("state", state_view_len(&step.state)),
        ("match_events", array_len(&match_lens)),
        ("lifecycle_events", array_len(&lifecycle_lens)),
    ];
    let combat_lens: Vec<i64>;
    if let Some(combat) = &step.combat_events {
        combat_lens = combat.iter().map(wrapped_event_len).collect();
        fields.push(("combat_events", array_len(&combat_lens)));
    }
    record_len(&fields)
}

/// Exact logical bytes retained in the speculative event window. Confirmed
/// presentation history lives in game-layer consumers and is not rollback
/// history.
#[must_use]
pub fn accounting(timeline: &RollbackEventTimeline) -> RollbackEventsAccounting {
    let retained_step_bytes: i64 = timeline.steps.values().map(step_len).sum();
    RollbackEventsAccounting {
        retained_step_bytes,
        total_bytes: retained_step_bytes,
    }
}
