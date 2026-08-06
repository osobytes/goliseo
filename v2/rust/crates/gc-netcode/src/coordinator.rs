//! Port of `game/online/coordinator.lua`.
//!
//! `coordinator` reads like lobby code, and that is exactly the trap (see
//! `v2/README.md` §2.1). It is Rust because it records per-tick hashes,
//! decides `hash_mismatch` / `late_input` / `desync` outcomes, and freezes
//! the session at an agreed `first_input_tick`. Both peers run it over the
//! same event stream and must land in the same state; a divergence here is a
//! protocol violation, not a cosmetic difference.
//!
//! It is a pure `step(state, event) -> (state, outcome)` reducer: no clock
//! reads, no I/O, no globals. [`CoordinatorState`] derives `Clone`, so the
//! Lua original's hand-written `copy_state`/`copy_peer`/`copy_value`
//! deep-copy helpers collapse to `state.clone()`.
//!
//! ## Following `protocol`'s `Value` design
//!
//! `crate::protocol` (`game/online/protocol.lua`) represents a manifest,
//! a slot-assignment array, and a runtime identity as a generic
//! [`protocol::Value`] rather than one struct per shape — see that module's
//! doc comment for why (in short: the malformed/unknown-field spec coverage
//! needs a value that a struct's static shape cannot hold). This module
//! follows the same choice for the same data: [`CoordinatorState::manifest`]
//! and [`CoordinatorState::assignments`] are `protocol::Value`, read and
//! built through small accessor/constructor helpers below
//! (`manifest_*`, `producer_*`) rather than struct field access.
//!
//! Everything that is *coordinator-local* — never itself a `protocol.lua`
//! wire shape — stays a typed Rust enum: [`Role`], [`ProducerKind`],
//! [`RejectCode`], [`TerminalReason`], [`Origin`], [`Disposition`],
//! [`SlotDriver`], [`PreferenceState`]. Canonical slot identity reuses
//! `gc_sim::input_frame::SlotId` via `protocol::slot_wire_id`/
//! `protocol::slot_id_from_wire`; match mode reuses `protocol::MatchMode`;
//! lifecycle phase reuses `protocol::LifecyclePhase`; message kind reuses
//! `protocol::MessageKind`. Wire-vocabulary closed sets that `protocol.rs`
//! itself keeps as bare strings (reject codes, disconnect codes, preference
//! rejections, combat status, match phase) stay `String`/`&str` here too,
//! matching `protocol.rs`'s own choice rather than inventing enums the wire
//! layer does not have.

use crate::live_slot;
use crate::protocol::{self, Value};
use gc_sim::input_frame::{self, SlotId, Team};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------
// Coordinator-local closed-set enums (never a `protocol.lua` wire shape of
// their own, but pervasive enough in this module's control flow to be worth
// typing rather than left as loose strings).
// ---------------------------------------------------------------------------

/// `SessionRole`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The session's single host.
    Host,
    /// A seated guest.
    Guest,
}

impl Role {
    /// The exact Lua wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            Role::Host => "host",
            Role::Guest => "guest",
        }
    }

    /// The reverse of [`Role::wire_str`].
    #[must_use]
    pub fn from_wire_str(value: &str) -> Option<Role> {
        match value {
            "host" => Some(Role::Host),
            "guest" => Some(Role::Guest),
            _ => None,
        }
    }
}

/// `SessionProducerKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducerKind {
    /// A human, identified by a stable peer id.
    Peer,
    /// A declared bot, identified by a deterministic seed.
    Bot,
}

impl ProducerKind {
    /// The exact Lua wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            ProducerKind::Peer => "peer",
            ProducerKind::Bot => "bot",
        }
    }

    /// The reverse of [`ProducerKind::wire_str`].
    #[must_use]
    pub fn from_wire_str(value: &str) -> Option<ProducerKind> {
        match value {
            "peer" => Some(ProducerKind::Peer),
            "bot" => Some(ProducerKind::Bot),
            _ => None,
        }
    }
}

/// Where an event or a termination originated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// This peer's own local caller.
    Local,
    /// A wire message from the remote peer.
    Remote,
    /// A locally-detected timeout (no wire traffic involved).
    Timeout,
}

/// How a `step` outcome was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// The event changed state.
    Applied,
    /// The event was accepted but changed nothing (a safe resend or no-op).
    Idempotent,
    /// The event was refused; state is untouched.
    Rejected,
}

/// A netcode-layer failure class reported by the local caller via
/// [`Event::NetcodeFailure`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetcodeFailure {
    /// The input transport itself failed.
    InputChannel,
    /// An input arrived too late for its tick.
    LateInput,
    /// A boundary hash disagreement.
    Desync,
}

/// Why a session ended. Every wire disconnect code and every locally
/// detected failure maps to exactly one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TerminalReason {
    /// A normal result acknowledged by every peer. The default purely so
    /// `TerminationOptions` can derive `Default` for struct-update syntax at
    /// call sites (see [`TerminationOptions`]) — it carries no meaning on
    /// its own the way [`OriginOrDefault::Unset`] does not either.
    #[default]
    Completed,
    /// This peer aborted locally.
    LocalAbort,
    /// The remote peer aborted.
    PeerAbort,
    /// A guest left.
    GuestLeft,
    /// The host left.
    HostLeft,
    /// The host removed this peer.
    Removed,
    /// The transport itself was lost.
    TransportLost,
    /// The remote peer violated the wire protocol.
    ProtocolViolation,
    /// The peers disagree on session manifest identity.
    ManifestMismatch,
    /// The peers are not running the same build.
    BuildMismatch,
    /// A slot assignment failed a coordinator invariant.
    InvalidAssignment,
    /// A peer never acknowledged the canonical start boundary in time.
    StartAckTimeout,
    /// The input channel itself failed.
    InputChannelFailure,
    /// An input arrived too late to be used.
    LateInput,
    /// Boundary hashes disagreed past the retry budget.
    HashMismatch,
}

/// Who materializes a canonical slot's input row for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotDriver {
    /// The slot is this human's currently live slot.
    Human,
    /// The slot is AI-driven this tick (not live, or a declared bot fill).
    Ai,
}

/// The reject-code vocabulary a [`Outcome`] can carry: every
/// `protocol::ErrorCode`, plus admission/permission codes that are
/// coordinator-local and never appear on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectCode {
    /// The data violates a structural, range, or canonical-form invariant.
    Malformed,
    /// The encoded wire would exceed the protocol's bound.
    WireTooLarge,
    /// The data names a version this build does not support.
    UnsupportedVersion,
    /// No handler exists for the message or event kind.
    UnknownMessage,
    /// An identity (session, manifest, peer) does not match its context.
    IdentityMismatch,
    /// The peers' runtime identities are incompatible.
    RuntimeMismatch,
    /// The event or message is not legal in the current lifecycle phase.
    InvalidPhase,
    /// The same identity was reused for a different reason than a resend.
    Duplicate,
    /// A transcript comparison found conflicting bytes under one identity.
    TranscriptConflict,
    /// The manifest names an unsupported match mode.
    UnsupportedMatchMode,
    /// The assignment does not name a valid canonical slot ownership.
    InvalidOwnership,
    /// The session is full.
    Capacity,
    /// A peer id names an already-admitted peer.
    DuplicatePeer,
    /// A host/guest role identity conflicted with an existing one.
    RoleConflict,
    /// A slot assignment failed a coordinator-local invariant.
    InvalidAssignment,
    /// A control message named an unrecognized transport link.
    UnknownLink,
    /// The event is not permitted for the sender's role or context.
    NotPermitted,
}

impl From<protocol::ErrorCode> for RejectCode {
    fn from(code: protocol::ErrorCode) -> Self {
        match code {
            protocol::ErrorCode::Malformed => RejectCode::Malformed,
            protocol::ErrorCode::WireTooLarge => RejectCode::WireTooLarge,
            protocol::ErrorCode::UnsupportedVersion => RejectCode::UnsupportedVersion,
            protocol::ErrorCode::UnknownMessage => RejectCode::UnknownMessage,
            protocol::ErrorCode::IdentityMismatch => RejectCode::IdentityMismatch,
            protocol::ErrorCode::RuntimeMismatch => RejectCode::RuntimeMismatch,
            protocol::ErrorCode::InvalidPhase => RejectCode::InvalidPhase,
            protocol::ErrorCode::Duplicate => RejectCode::Duplicate,
            protocol::ErrorCode::TranscriptConflict => RejectCode::TranscriptConflict,
            protocol::ErrorCode::UnsupportedMatchMode => RejectCode::UnsupportedMatchMode,
            protocol::ErrorCode::InvalidOwnership => RejectCode::InvalidOwnership,
        }
    }
}

/// An expected, recoverable coordinator-local failure (AGENTS.md §7).
/// Distinct from `protocol::Error` because [`RejectCode`]'s
/// coordinator-local half has no `protocol::ErrorCode` counterpart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Machine-readable failure reason.
    pub code: RejectCode,
    /// Human-readable detail.
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

impl From<protocol::Error> for Error {
    fn from(err: protocol::Error) -> Self {
        Error {
            code: err.code.into(),
            message: err.message,
        }
    }
}

/// Result alias for fallible coordinator-local operations.
pub type Result<T> = std::result::Result<T, Error>;

fn failure<T>(code: RejectCode, message: impl Into<String>) -> Result<T> {
    Err(Error {
        code,
        message: message.into(),
    })
}

// ---------------------------------------------------------------------------
// `protocol::Value` accessors / constructors for manifests and slot
// producers. `protocol.lua`'s own shape, mirrored exactly: see the module
// doc comment.
// ---------------------------------------------------------------------------

pub(crate) fn value_str(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn value_opt_str(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn value_int(value: &Value, field: &str) -> i64 {
    value.get(field).and_then(Value::as_int).unwrap_or(0)
}

pub(crate) fn opt_str_value(value: Option<&str>) -> Value {
    value.map(Value::str).unwrap_or(Value::Nil)
}

/// Every value at `manifest.teams.*.roster.*` whose `position` is `"keeper"`.
pub(crate) fn manifest_keepers(manifest: &Value) -> Vec<String> {
    let mut keepers = Vec::new();
    let Some(teams) = manifest.get("teams") else {
        return keepers;
    };
    for team_index in 1..=2 {
        let Some(team) = teams.get_index(team_index) else {
            continue;
        };
        let Some(roster) = team.get("roster") else {
            continue;
        };
        for player_index in 1..=input_frame::FIXTURE_TEAM_SIZE {
            let Some(player) = roster.get_index(player_index) else {
                continue;
            };
            if player.get("position").and_then(Value::as_str) == Some("keeper") {
                keepers.push(value_str(player, "player_id"));
            }
        }
    }
    keepers
}

pub(crate) fn manifest_match_mode(manifest: &Value) -> protocol::MatchMode {
    protocol::MatchMode::from_wire_str(&value_str(manifest, "match_mode"))
        .expect("manifest already validated")
}

/// `manifest.slots[index].player_id`, 1-based canonical slot index.
pub(crate) fn manifest_slot_player_id(manifest: &Value, index: i64) -> String {
    value_str(
        manifest
            .get("slots")
            .and_then(|slots| slots.get_index(index))
            .expect("canonical manifest slot"),
        "player_id",
    )
}

/// Build one canonical slot's producer record.
pub(crate) fn producer_record(
    slot: SlotId,
    team: Team,
    player_id: &str,
    kind: ProducerKind,
    producer_id: &str,
    bot_seed: Option<i64>,
) -> Value {
    Value::record(vec![
        ("slot", Value::str(protocol::slot_wire_id(slot))),
        ("team", Value::str(protocol::team_wire_str(team))),
        ("player_id", Value::str(player_id)),
        ("producer_kind", Value::str(kind.wire_str())),
        ("producer_id", Value::str(producer_id)),
        ("bot_seed", bot_seed.map(Value::int).unwrap_or(Value::Nil)),
    ])
}

pub(crate) fn producer_slot(producer: &Value) -> SlotId {
    protocol::slot_id_from_wire(&value_str(producer, "slot")).expect("canonical producer slot")
}

pub(crate) fn producer_team(producer: &Value) -> Team {
    protocol::team_from_wire_str(&value_str(producer, "team")).expect("canonical producer team")
}

pub(crate) fn producer_player_id(producer: &Value) -> String {
    value_str(producer, "player_id")
}

pub(crate) fn producer_kind(producer: &Value) -> ProducerKind {
    ProducerKind::from_wire_str(&value_str(producer, "producer_kind"))
        .expect("canonical producer kind")
}

pub(crate) fn producer_id(producer: &Value) -> String {
    value_str(producer, "producer_id")
}

/// A slot producer's declared bot seed (`None` for a peer producer).
/// Exposed for callers such as `coordinator_conformance`'s ownership
/// rendering, which prints it verbatim.
#[must_use]
pub fn producer_bot_seed(producer: &Value) -> Option<i64> {
    producer.get("bot_seed").and_then(Value::as_int)
}

/// The producer at 1-based canonical slot `index` in an assignments array.
pub(crate) fn assignment_at(assignments: &Value, index: i64) -> &Value {
    assignments
        .get_index(index)
        .expect("canonical assignment index")
}

pub(crate) fn assignments_producers(assignments: &Value) -> Vec<&Value> {
    (1..=input_frame::SLOT_COUNT)
        .map(|index| assignment_at(assignments, index))
        .collect()
}

pub(crate) fn bot_producer_id(index: i64) -> String {
    format!(
        "{BOT_PRODUCER_PREFIX}{}",
        protocol::slot_wire_id(live_slot::slot_id(index))
    )
}

pub(crate) fn bot_seed_for(manifest: &Value, index: i64) -> i64 {
    (value_int(manifest, "seed") + index * BOT_SEED_STRIDE).rem_euclid(protocol::MAX_SEED + 1)
}

// ---------------------------------------------------------------------------
// Reason vocabularies (wire strings, matching `protocol.rs`'s own choice not
// to type these as enums — see the module doc comment).
// ---------------------------------------------------------------------------

/// Every lifecycle phase, in the order the wire vocabulary #161 admits.
pub const PHASES: &[protocol::LifecyclePhase] = &[
    protocol::LifecyclePhase::New,
    protocol::LifecyclePhase::Handshake,
    protocol::LifecyclePhase::Manifest,
    protocol::LifecyclePhase::Assigned,
    protocol::LifecyclePhase::Ready,
    protocol::LifecyclePhase::Countdown,
    protocol::LifecyclePhase::Running,
    protocol::LifecyclePhase::Result,
    protocol::LifecyclePhase::Terminal,
];

/// Protocol version this coordinator implements.
pub const VERSION: i64 = 1;
/// The most canonical slots a session can ever seat (one per human, at most).
pub const MAX_PEERS: i64 = input_frame::SLOT_COUNT;
/// `MAX_PEERS` minus the host itself.
pub const MAX_GUESTS: i64 = MAX_PEERS - 1;
/// Bounded recent inbound wires retained per peer for duplicate detection.
pub const DUPLICATE_WINDOW: usize = 8;
/// Bounded recent boundary hashes retained locally.
pub const HASH_WINDOW: usize = 8;
/// Consecutive boundary-hash disagreements tolerated before a desync ends the session.
pub const MAX_HASH_MISMATCHES: i64 = 3;
/// Ticks a peer is given to acknowledge the canonical start boundary.
pub const START_ACK_TIMEOUT_TICKS: i64 = 120;
/// Ticks a guest waits for the host's verdict on a pair request before giving up.
pub const PREFERENCE_TIMEOUT_TICKS: i64 = 300;
/// Stride used to derive a declared bot's deterministic seed from the manifest seed.
pub const BOT_SEED_STRIDE: i64 = 7919;
/// Prefix for every declared-bot producer id.
pub const BOT_PRODUCER_PREFIX: &str = "bot.";
/// Reason text for a duplicate outside the retained transcript window.
pub const STALE_DUPLICATE_REASON: &str = "duplicate fell outside the retained transcript window";
/// Reason text for readiness naming a superseded ownership generation.
pub const STALE_GENERATION_REASON: &str = "readiness names a superseded ownership generation";
/// Reason text for a pair preference result answering no pending request.
pub const STALE_PREFERENCE_REASON: &str = "pair preference result answers no pending request";

fn terminal_code(reason: TerminalReason) -> Option<&'static str> {
    use TerminalReason::*;
    Some(match reason {
        Completed => return None,
        LocalAbort | PeerAbort => "host_abort",
        GuestLeft | HostLeft | Removed | TransportLost => "peer_disconnect",
        ProtocolViolation => "malformed_message",
        ManifestMismatch | BuildMismatch => "manifest_mismatch",
        InvalidAssignment => "invalid_assignment",
        StartAckTimeout | InputChannelFailure => "peer_disconnect",
        LateInput | HashMismatch => "desync",
    })
}

fn netcode_reason(failure: &str) -> Option<TerminalReason> {
    match failure {
        "input_channel" => Some(TerminalReason::InputChannelFailure),
        "late_input" => Some(TerminalReason::LateInput),
        "desync" => Some(TerminalReason::HashMismatch),
        _ => None,
    }
}

fn disconnect_reason(code: &str) -> TerminalReason {
    match code {
        "peer_left" => TerminalReason::GuestLeft,
        "transport_lost" => TerminalReason::TransportLost,
        "host_left" => TerminalReason::HostLeft,
        _ => TerminalReason::ProtocolViolation,
    }
}

/// Which identity disagreed decides how a tester should read it: `build_id`
/// and `source_id` disagreements mean the peers are running different
/// builds; every other expectation field is an ordinary content/config
/// disagreement.
fn identity_reason(path: &str) -> TerminalReason {
    match path {
        "manifest.build_id" | "manifest.source_id" => TerminalReason::BuildMismatch,
        _ => TerminalReason::ManifestMismatch,
    }
}

const EXPECTATION_FIELDS: &[&str] = &[
    "build_id",
    "source_id",
    "content_id",
    "tuning_id",
    "match_config_id",
    "fixture_id",
    "arena_id",
    "combat_rules_id",
    "gameplay_ai_policy_id",
    "combat_status",
];

// ---------------------------------------------------------------------------
// Structs derived from this module's own `---@class` blocks.
// ---------------------------------------------------------------------------

/// A retained inbound wire, for duplicate/transcript-conflict detection.
#[derive(Clone, Debug, PartialEq)]
pub struct WireRecord {
    /// The message's sender-local sequence.
    pub sequence: i64,
    /// Its canonical encoded wire text.
    pub wire: String,
}

/// One retained local boundary hash.
#[derive(Clone, Debug, PartialEq)]
pub struct HashRecord {
    /// The tick this hash covers.
    pub tick: i64,
    /// The canonical hex digest.
    pub hash: String,
}

/// One admitted peer, local peer included (always `peers[0]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    /// Stable peer identity.
    pub peer_id: String,
    /// `None` for the local peer; a transport link id otherwise.
    pub link_id: Option<String>,
    /// Host or guest.
    pub role: Role,
    /// Declared runtime identity (`protocol::Value`, see the module doc).
    pub runtime: Value,
    /// The build this peer declared in its handshake, if any.
    pub build_id: Option<String>,
    /// The manifest id this peer has accepted, if any.
    pub accepted_manifest_id: Option<String>,
    /// Whether this peer has been told its role assignment.
    pub assigned: bool,
    /// Local readiness.
    pub ready: bool,
    /// Whether this peer has acknowledged the start boundary.
    pub started: bool,
    /// Whether this peer has acknowledged the final result.
    pub result_acked: bool,
    /// Consecutive boundary-hash disagreements against this peer.
    pub hash_mismatches: i64,
    /// -1 until this peer has been heard from.
    pub last_sequence: i64,
    /// Bounded recent inbound wires, oldest first.
    pub window: Vec<WireRecord>,
    /// Host-side: the owned set this peer chose for itself.
    pub pair_choice: Option<Vec<SlotId>>,
}

/// A peer's last pair request and the host's answer to it. Purely a record
/// of what was asked and what came back; the ownership in force is always
/// [`CoordinatorState::assignments`], never this.
#[derive(Clone, Debug, PartialEq)]
pub struct Preference {
    /// The requested owned set, ascending canonical order.
    pub slots: Vec<SlotId>,
    /// The ownership generation the request answered.
    pub assignment_id: String,
    /// The current state of the request.
    pub status: PreferenceState,
    /// Set exactly when `status` is `Rejected`: a `protocol::PREFERENCE_REJECTIONS`
    /// wire string, or `"no_response"`/`"reseated"` (never sent, minted locally).
    pub reason: Option<String>,
    /// Guest only, and only while `Pending`: the clock the wait expires at.
    pub deadline: Option<i64>,
}

/// A pair preference's lifecycle status, `Pending` extending the wire's own
/// `SessionPreferenceStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferenceState {
    /// Sent, awaiting the host's verdict.
    Pending,
    /// The host granted it.
    Granted,
    /// The host reports it already matched the ownership in force.
    Unchanged,
    /// The host refused it, or the local wait expired.
    Rejected,
}

impl PreferenceState {
    fn wire_str(self) -> &'static str {
        match self {
            PreferenceState::Granted => "granted",
            PreferenceState::Unchanged => "unchanged",
            PreferenceState::Rejected => "rejected",
            PreferenceState::Pending => unreachable!("a verdict is never pending"),
        }
    }

    fn from_wire_str(value: &str) -> Option<PreferenceState> {
        match value {
            "granted" => Some(PreferenceState::Granted),
            "unchanged" => Some(PreferenceState::Unchanged),
            "rejected" => Some(PreferenceState::Rejected),
            _ => None,
        }
    }
}

/// The host's verdict on one pair preference.
#[derive(Clone, Debug, PartialEq)]
pub struct PreferenceVerdict {
    /// The verdict's status (never `Pending`).
    pub status: PreferenceState,
    /// Set exactly when `status` is `Rejected`.
    pub reason: Option<String>,
    /// The requested set as it was received.
    pub slots: Vec<SlotId>,
    /// Ownership to publish; set only when `status` is `Granted`.
    pub assignments: Option<Value>,
}

/// What a guest can verify locally before accepting a proposal: immutable
/// build, content, tuning, fixture, and combat-disposition identity.
/// Session-scoped values such as the seed are chosen by the host and are not
/// pre-known.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ManifestExpectation {
    /// Expected build id.
    pub build_id: Option<String>,
    /// Expected source id.
    pub source_id: Option<String>,
    /// Expected content id.
    pub content_id: Option<String>,
    /// Expected tuning id.
    pub tuning_id: Option<String>,
    /// Expected match config id.
    pub match_config_id: Option<String>,
    /// Expected fixture id.
    pub fixture_id: Option<String>,
    /// Expected arena id.
    pub arena_id: Option<String>,
    /// Expected combat rules id.
    pub combat_rules_id: Option<String>,
    /// Expected gameplay AI policy id.
    pub gameplay_ai_policy_id: Option<String>,
    /// Expected combat status wire string.
    pub combat_status: Option<String>,
}

impl ManifestExpectation {
    fn field(&self, name: &str) -> Option<&str> {
        match name {
            "build_id" => self.build_id.as_deref(),
            "source_id" => self.source_id.as_deref(),
            "content_id" => self.content_id.as_deref(),
            "tuning_id" => self.tuning_id.as_deref(),
            "match_config_id" => self.match_config_id.as_deref(),
            "fixture_id" => self.fixture_id.as_deref(),
            "arena_id" => self.arena_id.as_deref(),
            "combat_rules_id" => self.combat_rules_id.as_deref(),
            "gameplay_ai_policy_id" => self.gameplay_ai_policy_id.as_deref(),
            "combat_status" => self.combat_status.as_deref(),
            other => unreachable!("unknown expectation field {other}"),
        }
    }
}

/// The frozen, immutable-for-the-rest-of-the-session identity minted at the
/// countdown boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct Freeze {
    /// The manifest this session is running.
    pub manifest_id: String,
    /// The exact ownership generation that was frozen.
    pub assignment_id: String,
    /// The countdown identity this freeze answers.
    pub countdown_id: String,
    /// The first tick a rollback input applies to.
    pub first_input_tick: i64,
    /// Simulation RNG seed.
    pub seed: i64,
    /// Simulation tick rate.
    pub tick_rate: i64,
    /// Match duration, in ticks.
    pub duration_ticks: i64,
    /// Goal cap.
    pub max_goals: i64,
    /// Content identity.
    pub content_id: String,
    /// Tuning identity.
    pub tuning_id: String,
    /// Combat rules identity.
    pub combat_rules_id: String,
    /// Gameplay AI policy identity.
    pub gameplay_ai_policy_id: String,
    /// Combat status wire string.
    pub combat_status: String,
    /// Frozen with everything else; never varies mid-match.
    pub match_mode: protocol::MatchMode,
    /// The frozen ownership: a `protocol::Value` slot-producer array.
    pub assignments: Value,
    /// Human producer id -> owned slots, canonical order.
    pub owned: IndexMap<String, Vec<SlotId>>,
    /// Human producer id -> the slot live at the first tick.
    pub live: IndexMap<String, SlotId>,
}

/// The frozen slot-producer record for `slot`, equivalent to the Lua
/// original's `freeze.sources[slot.id]` (this port drops the redundant
/// `sources` field and looks up directly into `freeze.assignments`, which
/// is already in canonical slot order — see [`Freeze::assignments`]).
#[must_use]
pub fn freeze_source(freeze: &Freeze, slot: SlotId) -> &Value {
    assignment_at(&freeze.assignments, live_slot::slot_index(slot))
}

/// The simulation's reported progress, as the host announces it.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchProgress {
    /// Current match phase wire string (`"kickoff"`, `"playing"`, ...).
    pub phase: String,
    /// Simulation tick this report covers.
    pub tick: i64,
    /// Home score at this report.
    pub home_score: i64,
    /// Away score at this report.
    pub away_score: i64,
}

fn match_phase_allowed(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("kickoff", "playing")
            | ("playing", "goal_stoppage")
            | ("playing", "full_time")
            | ("goal_stoppage", "kickoff")
    )
}

/// The final simulation result.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchResult {
    /// Final simulation tick.
    pub final_tick: i64,
    /// Final home score.
    pub home_score: i64,
    /// Final away score.
    pub away_score: i64,
    /// Final boundary hash.
    pub final_hash: String,
}

/// Host only: why the last guest was dropped. A drop is not a terminal
/// event — the lobby stands and can still be filled — so the reason has
/// nowhere else to live.
#[derive(Clone, Debug, PartialEq)]
pub struct Departure {
    /// The departed peer.
    pub peer_id: String,
    /// The host's own local reason.
    pub reason: TerminalReason,
    /// What actually went on the wire (a `SessionDisconnectCode` string).
    pub code: String,
    /// Additional detail, if any.
    pub detail: Option<String>,
}

/// The stable, terminal-forever reason a session ended.
#[derive(Clone, Debug, PartialEq)]
pub struct Terminal {
    /// Why the session ended.
    pub reason: TerminalReason,
    /// `None` only for a completed session (a `SessionRejectCode` string).
    pub code: Option<String>,
    /// Where the termination originated.
    pub origin: Origin,
    /// The peer this termination is about, if any.
    pub peer_id: Option<String>,
    /// Additional detail, if any.
    pub detail: Option<String>,
}

/// One coordinator side effect. Variant-per-kind translation of the Lua
/// original's tagged `CoordinatorAction` class: an enum is strictly more
/// precise and gives the exhaustiveness checking the port calls for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Send `message` over every link in `targets`.
    Send {
        /// Ordered link ids.
        targets: Vec<String>,
        /// The message to send.
        message: protocol::ControlMessage,
    },
    /// Tear down one transport link.
    Close {
        /// The link to close.
        link_id: String,
    },
    /// The frozen session has reached the canonical start boundary. Boxed:
    /// clippy's `large_enum_variant` flags `Freeze` (the largest field any
    /// `Action` variant carries, by a wide margin) inflating every `Action`
    /// to its size.
    StartMatch {
        /// The frozen session identity.
        freeze: Box<Freeze>,
    },
    /// The session ended.
    Terminate {
        /// The stable session reason.
        terminal: Terminal,
    },
}

/// The result of one [`step`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// Whether the event was accepted (applied or a safe no-op).
    pub accepted: bool,
    /// How the event was resolved.
    pub disposition: Disposition,
    /// Set exactly when `disposition` is `Rejected`.
    pub code: Option<RejectCode>,
    /// Human-readable detail.
    pub reason: Option<String>,
    /// Side effects to perform.
    pub actions: Vec<Action>,
}

/// The complete state of one coordinator, host or guest.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinatorState {
    /// [`VERSION`].
    pub version: i64,
    /// Host or guest.
    pub role: Role,
    /// Session identity.
    pub session_id: String,
    /// This coordinator's own peer id.
    pub peer_id: String,
    /// The host's peer id (equals `peer_id` for a host coordinator).
    pub host_peer_id: String,
    /// Guest only: the single link back to the host.
    pub host_link_id: Option<String>,
    /// This peer's declared runtime identity.
    pub runtime: Value,
    /// This peer's own build, declared in its handshake.
    pub build_id: Option<String>,
    /// What a guest expects an incoming manifest to match, if anything.
    pub expectation: Option<ManifestExpectation>,
    /// Current lifecycle phase.
    pub phase: protocol::LifecyclePhase,
    /// Local logical clock, advanced by [`Event::Tick`].
    pub clock: i64,
    /// Next outbound sender-local sequence.
    pub sequence: i64,
    /// Local peer first, then admission order.
    pub peers: Vec<Peer>,
    /// The accepted session manifest, if any.
    pub manifest: Option<Value>,
    /// The accepted manifest's identity, if any.
    pub manifest_id: Option<String>,
    /// The ownership in force, if published.
    pub assignments: Option<Value>,
    /// Identity of the ownership generation in force.
    pub assignment_id: Option<String>,
    /// Monotonic count of ownership publications.
    pub assignment_epoch: i64,
    /// The local peer's last pair request and its answer.
    pub preference: Option<Preference>,
    /// The frozen session identity, once countdown has started.
    pub freeze: Option<Freeze>,
    /// Ticks left in the countdown, once started.
    pub countdown_remaining: Option<i64>,
    /// Host only: the clock a pending start acknowledgement expires at.
    pub start_deadline: Option<i64>,
    /// The simulation's reported progress, once running.
    pub progress: Option<MatchProgress>,
    /// The host-confirmed final result.
    pub result: Option<MatchResult>,
    /// This peer's own locally-acknowledged result.
    pub local_result: Option<MatchResult>,
    /// Bounded recent local boundary hashes.
    pub hashes: Vec<HashRecord>,
    /// Host only: why the last guest was dropped.
    pub departure: Option<Departure>,
    /// Set once the session has ended.
    pub terminal: Option<Terminal>,
}

/// Inputs to [`new`].
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Host or guest.
    pub role: Role,
    /// Session identity.
    pub session_id: String,
    /// This coordinator's own peer id.
    pub peer_id: String,
    /// Guest only: the host's peer id.
    pub host_peer_id: Option<String>,
    /// Guest only: the link back to the host.
    pub host_link_id: Option<String>,
    /// This peer's declared runtime identity.
    pub runtime: Value,
    /// This peer's build, declared in its handshake.
    pub build_id: Option<String>,
    /// What a guest expects an incoming manifest to match.
    pub expectation: Option<ManifestExpectation>,
}

/// A compact snapshot of a coordinator's own state, for diagnostics/UI.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Host or guest.
    pub role: Role,
    /// Current lifecycle phase.
    pub phase: protocol::LifecyclePhase,
    /// Total admitted peers, including the local one.
    pub peer_count: usize,
    /// How many admitted peers are ready.
    pub ready_count: usize,
    /// The accepted manifest's identity, if any.
    pub manifest_id: Option<String>,
    /// Whether the session has frozen.
    pub frozen: bool,
    /// Set once the session has ended.
    pub terminal: Option<Terminal>,
}

// ---------------------------------------------------------------------------
// One coordinator event. Fields Lua validates at runtime against a closed
// vocabulary (rejecting with a typed "malformed"/"unknown" outcome on a
// miss) stay loosely typed (`String`) here on purpose, so those rejection
// paths — exercised by the spec — stay reachable.
// ---------------------------------------------------------------------------

/// One event driving [`step`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A guest opens its manual connection.
    Connect,
    /// A raw or already-decoded control message arrived over `link_id`.
    Control {
        /// The transport link this arrived on.
        link_id: String,
        /// An already-decoded message, if the caller decoded it itself.
        message: Option<protocol::ControlMessage>,
        /// Raw wire text, decoded internally when `message` is `None`.
        wire: Option<String>,
    },
    /// A transport link was lost.
    LinkLost {
        /// The lost link.
        link_id: String,
        /// The disconnect code, as raw wire text (validated inside the
        /// handler; an unrecognized code is a legitimate rejection path).
        code: Option<String>,
    },
    /// The host proposes the session manifest.
    ProposeManifest {
        /// The proposed manifest (`protocol::Value`).
        manifest: Value,
    },
    /// The host publishes slot ownership.
    AssignSlots {
        /// The ownership to publish (`protocol::Value` slot-producer array).
        assignments: Value,
        /// Whether to reseat around every pair claim this plan can still honour.
        preserve_claims: bool,
    },
    /// This peer requests an owned set.
    PreferPair {
        /// The requested owned set.
        slots: Vec<SlotId>,
    },
    /// This peer's own readiness changed.
    SetReady {
        /// The new readiness value.
        ready: bool,
    },
    /// The host starts the countdown.
    BeginCountdown {
        /// The countdown's identity.
        countdown_id: String,
        /// Countdown length, in ticks.
        remaining_ticks: i64,
        /// The first tick a rollback input applies to.
        first_input_tick: i64,
    },
    /// Advance the local logical clock by one tick.
    Tick,
    /// The host reports simulation progress.
    MatchPhase {
        /// The reported phase wire string.
        phase: String,
        /// The reported tick.
        tick: i64,
        /// The reported home score.
        home_score: i64,
        /// The reported away score.
        away_score: i64,
    },
    /// A local boundary hash was computed.
    HashReport {
        /// The tick this hash covers.
        tick: i64,
        /// The canonical hex digest.
        boundary_hash: String,
    },
    /// The local simulation reports its final result.
    Finish {
        /// Final simulation tick.
        final_tick: i64,
        /// Final home score.
        home_score: i64,
        /// Final away score.
        away_score: i64,
        /// Final boundary hash.
        final_hash: String,
    },
    /// The local netcode layer reports a failure.
    NetcodeFailure {
        /// The failure class, as raw text (an unrecognized class is a
        /// legitimate rejection path, exercised by the spec).
        failure: String,
        /// The peer this failure concerns, if any.
        peer_id: Option<String>,
        /// Additional detail.
        detail: Option<String>,
    },
    /// A guest leaves.
    Leave,
    /// This peer aborts the session.
    Abort {
        /// The reject code to announce, if any (defaults to `"host_abort"`).
        code: Option<String>,
        /// Additional detail.
        detail: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Outcome constructors.
// ---------------------------------------------------------------------------

fn rejected(code: RejectCode, reason: impl Into<String>) -> Outcome {
    Outcome {
        accepted: false,
        disposition: Disposition::Rejected,
        code: Some(code),
        reason: Some(reason.into()),
        actions: Vec::new(),
    }
}

fn applied(actions: Vec<Action>) -> Outcome {
    Outcome {
        accepted: true,
        disposition: Disposition::Applied,
        code: None,
        reason: None,
        actions,
    }
}

fn idempotent() -> Outcome {
    Outcome {
        accepted: true,
        disposition: Disposition::Idempotent,
        code: None,
        reason: None,
        actions: Vec::new(),
    }
}

fn stale(reason: impl Into<String>) -> Outcome {
    Outcome {
        accepted: true,
        disposition: Disposition::Idempotent,
        code: None,
        reason: Some(reason.into()),
        actions: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Small state accessors.
// ---------------------------------------------------------------------------

fn local_peer(state: &CoordinatorState) -> &Peer {
    &state.peers[0]
}

fn local_peer_mut(state: &mut CoordinatorState) -> &mut Peer {
    &mut state.peers[0]
}

fn peer_by_link<'a>(state: &'a CoordinatorState, link_id: &str) -> Option<&'a Peer> {
    state
        .peers
        .iter()
        .find(|peer| peer.link_id.as_deref() == Some(link_id))
}

fn peer_by_id<'a>(state: &'a CoordinatorState, peer_id: &str) -> Option<&'a Peer> {
    state.peers.iter().find(|peer| peer.peer_id == peer_id)
}

fn peer_by_id_mut<'a>(state: &'a mut CoordinatorState, peer_id: &str) -> Option<&'a mut Peer> {
    state.peers.iter_mut().find(|peer| peer.peer_id == peer_id)
}

fn link_targets(state: &CoordinatorState, exclude_link: Option<&str>) -> Vec<String> {
    state
        .peers
        .iter()
        .filter_map(|peer| peer.link_id.clone())
        .filter(|link_id| Some(link_id.as_str()) != exclude_link)
        .collect()
}

// ---------------------------------------------------------------------------
// Emitting: the single place a coordinator mints outbound control traffic.
// ---------------------------------------------------------------------------

fn try_emit(
    next_state: &mut CoordinatorState,
    kind: protocol::MessageKind,
    body: Value,
    targets: Vec<String>,
    actions: &mut Vec<Action>,
) -> protocol::Result<()> {
    let message = protocol::new(
        kind,
        &next_state.session_id,
        &next_state.peer_id,
        next_state.sequence,
        body,
    )?;
    protocol::validate_phase(&message, next_state.phase)?;
    next_state.sequence += 1;
    if !targets.is_empty() {
        actions.push(Action::Send { targets, message });
    }
    Ok(())
}

fn emit(
    next_state: &mut CoordinatorState,
    kind: protocol::MessageKind,
    body: Value,
    targets: Vec<String>,
    actions: &mut Vec<Action>,
) {
    try_emit(next_state, kind, body, targets, actions)
        .unwrap_or_else(|err| panic!("coordinator emitted an invalid message: {err}"));
}

/// Options for [`terminate_from`]/[`terminate_session`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminationOptions {
    /// Why the session ended.
    pub reason: TerminalReason,
    /// Overrides the code [`terminal_code`] would otherwise pick.
    pub code: Option<String>,
    /// Where the termination originated.
    pub origin: OriginOrDefault,
    /// The peer this termination is about, if any.
    pub peer_id: Option<String>,
    /// Additional detail.
    pub detail: Option<String>,
    /// Whether to announce an `abort` to every remaining link.
    pub announce: bool,
    /// A link to exclude from the announcement.
    pub exclude_link: Option<String>,
}

/// [`TerminationOptions::origin`] has no meaningful zero value in the Lua
/// original (every call site names one); this exists purely so
/// `TerminationOptions` can derive `Default` for struct-update syntax at call
/// sites, the same way Lua's table literals only set the fields they need.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OriginOrDefault {
    #[default]
    /// Placeholder; every real call site overrides this.
    Unset,
    /// See [`Origin::Local`].
    Local,
    /// See [`Origin::Remote`].
    Remote,
    /// See [`Origin::Timeout`].
    Timeout,
}

impl From<OriginOrDefault> for Origin {
    fn from(value: OriginOrDefault) -> Self {
        match value {
            OriginOrDefault::Unset => panic!("a termination must name its origin"),
            OriginOrDefault::Local => Origin::Local,
            OriginOrDefault::Remote => Origin::Remote,
            OriginOrDefault::Timeout => Origin::Timeout,
        }
    }
}

fn terminate_session(
    next_state: &mut CoordinatorState,
    options: TerminationOptions,
    actions: &mut Vec<Action>,
) {
    let code = options
        .code
        .or_else(|| terminal_code(options.reason).map(str::to_string));
    if options.announce {
        let targets = link_targets(next_state, options.exclude_link.as_deref());
        emit(
            next_state,
            protocol::MessageKind::Abort,
            Value::record(vec![(
                "code",
                Value::str(
                    code.clone()
                        .expect("an announced termination must carry a code"),
                ),
            )]),
            targets,
            actions,
        );
    }
    for peer in &next_state.peers {
        if let Some(link_id) = &peer.link_id {
            actions.push(Action::Close {
                link_id: link_id.clone(),
            });
        }
    }
    next_state.phase = protocol::LifecyclePhase::Terminal;
    let terminal = Terminal {
        reason: options.reason,
        code,
        origin: options.origin.into(),
        peer_id: options.peer_id,
        detail: options.detail,
    };
    next_state.terminal = Some(terminal.clone());
    actions.push(Action::Terminate { terminal });
}

fn terminate_from(
    state: &CoordinatorState,
    options: TerminationOptions,
) -> (CoordinatorState, Outcome) {
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    terminate_session(&mut next_state, options, &mut actions);
    (next_state, applied(actions))
}

// ---------------------------------------------------------------------------
// Public pure functions.
// ---------------------------------------------------------------------------

/// The first expectation field that disagrees with `manifest`, if any.
pub fn expectation_difference(
    expectation: Option<&ManifestExpectation>,
    manifest: &Value,
) -> Option<protocol::Difference> {
    let expectation = expectation?;
    for field in EXPECTATION_FIELDS {
        let Some(expected) = expectation.field(field) else {
            continue;
        };
        let actual = manifest.get(field).cloned().unwrap_or(Value::Nil);
        if actual.as_str() != Some(expected) {
            return Some(protocol::Difference {
                path: format!("manifest.{field}"),
                expected: Value::str(expected),
                actual,
            });
        }
    }
    None
}

/// Every canonical slot's declared source, from a manifest and a full set of
/// assignments. Refuses an assignment that seats a combat-protected keeper
/// on a canonical outfield slot, or that otherwise fails
/// `protocol::validate_assignment_manifest`.
pub fn slot_sources(manifest: &Value, assignments: &Value) -> Result<Value> {
    protocol::validate_manifest(manifest)?;
    let keepers = manifest_keepers(manifest);
    for producer in assignments_producers(assignments) {
        if keepers.contains(&producer_player_id(producer)) {
            return failure(
                // `game/online/coordinator.lua:614` returns "invalid_assignment"
                // here, not an ownership code. The distinction is visible to a
                // peer, which rejects on the code rather than the message.
                RejectCode::InvalidAssignment,
                "combat-protected keepers cannot own a canonical outfield slot",
            );
        }
    }
    protocol::validate_assignment_manifest(manifest, assignments)?;
    // Re-shape into a slot-id-keyed table, as `coordinator.lua:625-630` does.
    // `validate_assignment_manifest` has already proven the array holds exactly
    // the eight canonical slots in OMP-1 order, so indexing by slot id cannot
    // collide — this only makes `sources["home_1"]` work for callers. Returning
    // the 1-based integer-keyed array unchanged left every string lookup empty.
    let mut fields: Vec<(&str, Value)> = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).expect("canonical slot index");
        let producer = assignments
            .get_index(index)
            .cloned()
            .unwrap_or_else(|| panic!("assignment missing for canonical slot {index}"));
        fields.push((crate::protocol::slot_wire_id(slot.id), producer));
    }
    Ok(Value::record(fields))
}

/// Seat every human in `peer_ids` in contiguous canonical blocks of
/// `slots_per_human`, filling any remaining canonical slots with declared
/// bots. Humans are seated in contiguous canonical blocks of
/// `slots_per_human`, so the Nth human owns slots `(N-1)*k+1 ..= N*k`.
pub fn plan_assignments(manifest: &Value, peer_ids: &[String]) -> Result<Value> {
    protocol::validate_manifest(manifest)?;
    let shape = protocol::match_mode_shape(manifest_match_mode(manifest));
    if peer_ids.len() as i64 > shape.humans {
        return failure(
            RejectCode::Capacity,
            format!(
                "a {} session seats at most {} humans",
                shape.mode.wire_str(),
                shape.humans
            ),
        );
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut peer_by_slot: Vec<Option<&str>> = vec![None; input_frame::SLOT_COUNT as usize];
    for (order, peer_id) in peer_ids.iter().enumerate() {
        if seen.contains(&peer_id.as_str()) {
            return failure(
                RejectCode::DuplicatePeer,
                "human slot sources must be unique peer ids",
            );
        }
        seen.push(peer_id);
        for offset in 1..=shape.slots_per_human {
            let index = order as i64 * shape.slots_per_human + offset;
            peer_by_slot[(index - 1) as usize] = Some(peer_id.as_str());
        }
    }
    let mut assignments = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).expect("canonical slot index");
        let player_id = manifest_slot_player_id(manifest, index);
        assignments.push(if let Some(peer_id) = peer_by_slot[(index - 1) as usize] {
            producer_record(
                slot.id,
                slot.team,
                &player_id,
                ProducerKind::Peer,
                peer_id,
                None,
            )
        } else {
            producer_record(
                slot.id,
                slot.team,
                &player_id,
                ProducerKind::Bot,
                &bot_producer_id(index),
                Some(bot_seed_for(manifest, index)),
            )
        });
    }
    let assignments = Value::array(assignments);
    slot_sources(manifest, &assignments)?;
    Ok(assignments)
}

fn validate_local_assignments(state: &CoordinatorState, assignments: &Value) -> Result<()> {
    let Some(manifest) = state
        .manifest
        .as_ref()
        .filter(|_| state.manifest_id.is_some())
    else {
        return failure(
            RejectCode::InvalidPhase,
            "slot assignment requires an accepted manifest",
        );
    };
    slot_sources(manifest, assignments)?;
    let mut peer_slots: IndexMap<String, i64> = IndexMap::new();
    for producer in assignments_producers(assignments) {
        let id = producer_id(producer);
        if producer_kind(producer) == ProducerKind::Peer {
            if peer_by_id(state, &id).is_none() {
                return failure(
                    RejectCode::InvalidAssignment,
                    format!("slot producer {id} is not an admitted peer"),
                );
            }
            *peer_slots.entry(id).or_insert(0) += 1;
        } else if peer_by_id(state, &id).is_some() {
            return failure(
                RejectCode::InvalidAssignment,
                "a bot producer cannot reuse an admitted peer id",
            );
        }
    }
    let shape = protocol::match_mode_shape(manifest_match_mode(manifest));
    for peer in &state.peers {
        let owned = peer_slots.get(&peer.peer_id).copied().unwrap_or(0);
        if owned != shape.slots_per_human {
            return failure(
                RejectCode::InvalidAssignment,
                format!(
                    "admitted peer {} must own exactly {} canonical slot(s) in {}",
                    peer.peer_id,
                    shape.slots_per_human,
                    shape.mode.wire_str()
                ),
            );
        }
    }
    Ok(())
}

fn assignments_equal(a: Option<&Value>, b: &Value) -> bool {
    a == Some(b)
}

fn owns_slot(state: &CoordinatorState, peer_id: &str) -> bool {
    match &state.assignments {
        Some(assignments) => assignments_producers(assignments)
            .into_iter()
            .any(|producer| {
                producer_kind(producer) == ProducerKind::Peer && producer_id(producer) == peer_id
            }),
        None => false,
    }
}

/// The slots one producer owns, in canonical order: four in 1v1, two in
/// 2v2, one in 4v4. Empty when ownership is unpublished or the producer is
/// not seated.
pub fn owned_slots(state: &CoordinatorState, producer_id: &str) -> Vec<SlotId> {
    let assignments = state
        .freeze
        .as_ref()
        .map(|freeze| &freeze.assignments)
        .or(state.assignments.as_ref());
    match assignments {
        Some(assignments) => protocol::owned_slots(assignments, producer_id)
            .into_iter()
            .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
            .collect(),
        None => Vec::new(),
    }
}

/// The producer that owns `slot`, if ownership is published or frozen.
pub fn slot_owner(state: &CoordinatorState, slot: SlotId) -> Option<&Value> {
    let assignments = state
        .freeze
        .as_ref()
        .map(|freeze| &freeze.assignments)
        .or(state.assignments.as_ref())?;
    Some(assignment_at(assignments, live_slot::slot_index(slot)))
}

/// True when the ownership in force already seats exactly the admitted
/// roster for the frozen mode.
pub fn ownership_seats_roster(state: &CoordinatorState) -> bool {
    match &state.assignments {
        Some(assignments) => validate_local_assignments(state, assignments).is_ok(),
        None => false,
    }
}

/// The plan a claim-preserving reseat produces, plus which claims it kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Reseat {
    /// The plan, adjusted to honour every surviving claim.
    pub assignments: Value,
    /// Peer id -> the claim this reseat keeps.
    pub retained: IndexMap<String, Vec<SlotId>>,
}

/// Reconcile a seating plan (which knows the roster and nothing else)
/// against every peer's granted pair claim.
///
/// # Panics
///
/// Panics if `state.manifest` is unset, or if the plan cannot seat every
/// unclaimed human — both programmer errors under this crate's invariants.
pub fn reseat_claims(state: &CoordinatorState, assignments: &Value) -> Reseat {
    let manifest = state.manifest.as_ref().expect("reseat requires a manifest");
    let shape = protocol::match_mode_shape(manifest_match_mode(manifest));
    let producers = assignments_producers(assignments);
    let mut human_index: Vec<Option<usize>> = vec![None; 9];
    let mut human_order: Vec<usize> = Vec::new();
    for (index, producer) in producers.iter().enumerate() {
        if producer_kind(producer) == ProducerKind::Peer {
            let slot_index = live_slot::slot_index(producer_slot(producer)) as usize;
            human_index[slot_index] = Some(index);
            human_order.push(index);
        }
    }
    let mut kept: Vec<bool> = vec![false; 9];
    let survives = |claim: &[SlotId], kept: &[bool]| -> bool {
        if claim.len() as i64 != shape.slots_per_human {
            return false;
        }
        let mut team: Option<Team> = None;
        for &slot in claim {
            let slot_index = live_slot::slot_index(slot) as usize;
            let Some(index) = human_index[slot_index] else {
                return false;
            };
            if kept[slot_index] {
                return false;
            }
            let producer_team = producer_team(producers[index]);
            match team {
                None => team = Some(producer_team),
                Some(t) if t != producer_team => return false,
                _ => {}
            }
        }
        true
    };
    let mut retained: IndexMap<String, Vec<SlotId>> = IndexMap::new();
    for peer in &state.peers {
        if let Some(claim) = &peer.pair_choice
            && survives(claim, &kept)
        {
            retained.insert(peer.peer_id.clone(), claim.clone());
            for &slot in claim {
                kept[live_slot::slot_index(slot) as usize] = true;
            }
        }
    }
    let mut free: Vec<usize> = Vec::new();
    let mut unseated: Vec<String> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for &index in &human_order {
        let producer = producers[index];
        let slot_index = live_slot::slot_index(producer_slot(producer)) as usize;
        if !kept[slot_index] {
            free.push(index);
        }
        let pid = producer_id(producer);
        if !seen.contains(&pid.as_str()) && !retained.contains_key(&pid) {
            unseated.push(pid.clone());
        }
        if !seen.contains(&pid.as_str()) {
            seen.push(Box::leak(pid.into_boxed_str()));
        }
    }
    let mut reseated: Vec<Value> = producers.iter().map(|p| (*p).clone()).collect();
    for peer in &state.peers {
        if let Some(claim) = retained.get(&peer.peer_id) {
            for &slot in claim {
                let slot_index = live_slot::slot_index(slot) as usize;
                let index =
                    human_index[slot_index].expect("a retained claim must name a human slot");
                reseated[index].set("producer_id", Value::str(peer.peer_id.clone()));
            }
        }
    }
    let mut cursor = 0usize;
    for producer_id_value in &unseated {
        for _ in 0..shape.slots_per_human {
            let index = *free
                .get(cursor)
                .expect("a reseat ran out of free human slots");
            reseated[index].set("producer_id", Value::str(producer_id_value.clone()));
            cursor += 1;
        }
    }
    assert!(
        cursor == free.len(),
        "a reseat must seat every free human slot exactly once"
    );
    Reseat {
        assignments: Value::array(reseated),
        retained,
    }
}

fn exchange_assignments(state: &CoordinatorState, peer_id: &str, slots: &[SlotId]) -> Value {
    let manifest = state
        .manifest
        .as_ref()
        .expect("exchange requires a manifest");
    let current_assignments = state
        .assignments
        .as_ref()
        .expect("exchange requires ownership");
    let current: Vec<SlotId> = protocol::owned_slots(current_assignments, peer_id)
        .into_iter()
        .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
        .collect();
    let gained: Vec<SlotId> = slots
        .iter()
        .copied()
        .filter(|s| !current.contains(s))
        .collect();
    let vacated: Vec<SlotId> = current
        .iter()
        .copied()
        .filter(|s| !slots.contains(s))
        .collect();
    assert!(
        gained.len() == vacated.len(),
        "a pair preference must trade slot for slot"
    );
    let mut returned_to: Vec<(SlotId, String)> = Vec::new();
    for (index, &slot) in gained.iter().enumerate() {
        let owner = producer_id(assignment_at(
            current_assignments,
            live_slot::slot_index(slot),
        ));
        returned_to.push((vacated[index], owner));
    }
    let mut assignments = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).expect("canonical slot index");
        let player_id = manifest_slot_player_id(manifest, index);
        let producer_id_opt = if slots.contains(&slot.id) {
            Some(peer_id.to_string())
        } else {
            returned_to
                .iter()
                .find(|(vacated_slot, _)| *vacated_slot == slot.id)
                .map(|(_, id)| id.clone())
        };
        let producer = match producer_id_opt {
            Some(pid) if peer_by_id(state, &pid).is_some() => producer_record(
                slot.id,
                slot.team,
                &player_id,
                ProducerKind::Peer,
                &pid,
                None,
            ),
            Some(_pid) => producer_record(
                slot.id,
                slot.team,
                &player_id,
                ProducerKind::Bot,
                &bot_producer_id(index),
                Some(bot_seed_for(manifest, index)),
            ),
            None => assignment_at(current_assignments, index).clone(),
        };
        assignments.push(producer);
    }
    Value::array(assignments)
}

/// The host's verdict on one pair preference. Pure: it reads the ownership
/// in force and answers; the caller decides what to publish.
pub fn evaluate_preference(
    state: &CoordinatorState,
    peer_id: &str,
    slots: &[SlotId],
) -> PreferenceVerdict {
    let requested = slots.to_vec();
    let refuse = |reason: &str, requested: Vec<SlotId>| PreferenceVerdict {
        status: PreferenceState::Rejected,
        reason: Some(reason.to_string()),
        slots: requested,
        assignments: None,
    };
    if state.freeze.is_some() {
        return refuse("after_freeze", requested);
    }
    let (Some(manifest), Some(assignments)) = (&state.manifest, &state.assignments) else {
        return refuse("not_seated", requested);
    };
    let current: Vec<SlotId> = protocol::owned_slots(assignments, peer_id)
        .into_iter()
        .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
        .collect();
    if current.is_empty() {
        return refuse("not_seated", requested);
    }
    let requested_value = Value::array(
        requested
            .iter()
            .map(|s| Value::str(protocol::slot_wire_id(*s)))
            .collect(),
    );
    if protocol::validate_slot_set(&requested_value).is_err() {
        return refuse("invalid_slot", requested);
    }
    let shape = protocol::match_mode_shape(manifest_match_mode(manifest));
    if requested.len() as i64 != shape.slots_per_human {
        return refuse("invalid_slot", requested);
    }
    let team = producer_team(assignment_at(
        assignments,
        live_slot::slot_index(current[0]),
    ));
    for &slot in &requested {
        if producer_team(assignment_at(assignments, live_slot::slot_index(slot))) != team {
            return refuse("wrong_team", requested);
        }
    }
    let keeps = requested.iter().filter(|s| current.contains(s)).count();
    if keeps == 0 {
        return refuse("detached", requested);
    }
    let mut claimed: Vec<SlotId> = Vec::new();
    for peer in &state.peers {
        if peer.peer_id != peer_id
            && let Some(choice) = &peer.pair_choice
        {
            claimed.extend(choice.iter().copied());
        }
    }
    for &slot in &requested {
        if !current.contains(&slot) && claimed.contains(&slot) {
            return refuse("already_taken", requested);
        }
    }
    if keeps == current.len() {
        return PreferenceVerdict {
            status: PreferenceState::Unchanged,
            reason: None,
            slots: requested,
            assignments: None,
        };
    }
    let assignments = exchange_assignments(state, peer_id, &requested);
    assert!(
        validate_local_assignments(state, &assignments).is_ok(),
        "a granted pair preference produced ownership the host would refuse"
    );
    PreferenceVerdict {
        status: PreferenceState::Granted,
        reason: None,
        slots: requested,
        assignments: Some(assignments),
    }
}

/// One human's control moves within their owned set and nowhere else:
/// winning the ball switches control to the winner, and the `switch` edge
/// without the ball hands control to the outfielder nearest the ball, both
/// intersected with the owned set. In 4v4 the owned set has one member, so
/// every branch returns it and switching is inert — a consequence of the
/// general rule, not a special case.
///
/// # Panics
///
/// Panics if `live` is not a member of `owned` — a programmer error under
/// this crate's invariants.
pub fn next_live_slot(
    owned: &[SlotId],
    live: SlotId,
    transition: &live_slot::LiveTransition,
) -> SlotId {
    assert!(
        owned.contains(&live),
        "the live slot must belong to the owned set"
    );
    if let Some(winner) = transition.winner
        && owned.contains(&winner)
    {
        return winner;
    }
    if !transition.switch || transition.carrier == Some(live) {
        return live;
    }
    for &slot in &transition.ranked {
        if owned.contains(&slot) {
            return slot;
        }
    }
    live
}

/// The opening live slot of every human in one ownership generation: the
/// first owned slot in canonical order, derived from the assignments alone
/// so every peer computes the same one without exchanging anything further.
pub fn preview_live(assignments: &Value) -> IndexMap<String, SlotId> {
    let mut live: IndexMap<String, SlotId> = IndexMap::new();
    for producer in assignments_producers(assignments) {
        if producer_kind(producer) == ProducerKind::Peer {
            let pid = producer_id(producer);
            if !live.contains_key(&pid) {
                live.insert(pid, producer_slot(producer));
            }
        }
    }
    live
}

/// Who materializes each canonical slot's input row for one tick. A slot is
/// human-driven only while it is that human's live slot; every other owned
/// slot is AI, exactly like a declared bot fill.
pub fn slot_drivers(freeze: &Freeze, live: Option<&IndexMap<String, SlotId>>) -> Vec<SlotDriver> {
    let live = live.unwrap_or(&freeze.live);
    let mut drivers = vec![SlotDriver::Ai; input_frame::SLOT_COUNT as usize];
    for producer in assignments_producers(&freeze.assignments) {
        let pid = producer_id(producer);
        let slot = producer_slot(producer);
        let is_live =
            producer_kind(producer) == ProducerKind::Peer && live.get(&pid) == Some(&slot);
        let index = (live_slot::slot_index(slot) - 1) as usize;
        drivers[index] = if is_live {
            SlotDriver::Human
        } else {
            SlotDriver::Ai
        };
    }
    drivers
}

fn slot_lists_equal(a: &[SlotId], b: &[SlotId]) -> bool {
    a == b
}

fn clear_readiness(next_state: &mut CoordinatorState) {
    for peer in &mut next_state.peers {
        peer.ready = false;
    }
    if next_state.phase == protocol::LifecyclePhase::Ready {
        next_state.phase = protocol::LifecyclePhase::Assigned;
    }
}

fn all_ready(state: &CoordinatorState) -> bool {
    if state.role == Role::Guest {
        return local_peer(state).ready;
    }
    state.peers.iter().all(|peer| peer.ready)
}

fn refresh_ready_phase(next_state: &mut CoordinatorState) {
    if next_state.phase != protocol::LifecyclePhase::Assigned
        && next_state.phase != protocol::LifecyclePhase::Ready
    {
        return;
    }
    next_state.phase = if all_ready(next_state) {
        protocol::LifecyclePhase::Ready
    } else {
        protocol::LifecyclePhase::Assigned
    };
}

fn claims_after(
    state: &CoordinatorState,
    peer_id: &str,
    slots: &[SlotId],
) -> IndexMap<String, Vec<SlotId>> {
    let mut claims: IndexMap<String, Vec<SlotId>> = IndexMap::new();
    for peer in &state.peers {
        if let Some(choice) = &peer.pair_choice {
            claims.insert(peer.peer_id.clone(), choice.clone());
        }
    }
    claims.insert(peer_id.to_string(), slots.to_vec());
    claims
}

fn settle_preference(next_state: &mut CoordinatorState) {
    let Some(preference) = next_state.preference.clone() else {
        return;
    };
    if preference.status != PreferenceState::Granted
        && preference.status != PreferenceState::Unchanged
    {
        return;
    }
    let owned: Vec<SlotId> = protocol::owned_slots(
        next_state
            .assignments
            .as_ref()
            .expect("settle requires ownership"),
        &next_state.peer_id,
    )
    .into_iter()
    .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
    .collect();
    if slot_lists_equal(&owned, &preference.slots) {
        return;
    }
    next_state.preference = Some(Preference {
        slots: preference.slots,
        assignment_id: preference.assignment_id,
        status: PreferenceState::Rejected,
        reason: Some("reseated".to_string()),
        deadline: None,
    });
}

fn publish_ownership(
    next_state: &mut CoordinatorState,
    assignments: Value,
    retained: Option<IndexMap<String, Vec<SlotId>>>,
    actions: &mut Vec<Action>,
) {
    next_state.assignment_epoch += 1;
    next_state.assignment_id = Some(protocol::assignment_id(
        &assignments,
        next_state.assignment_epoch,
    ));
    next_state.assignments = Some(assignments.clone());
    next_state.phase = protocol::LifecyclePhase::Assigned;
    clear_readiness(next_state);
    for peer in &mut next_state.peers {
        let claim = retained
            .as_ref()
            .and_then(|r| r.get(&peer.peer_id))
            .cloned();
        if let Some(claim) = &claim {
            let owned: Vec<SlotId> = protocol::owned_slots(&assignments, &peer.peer_id)
                .into_iter()
                .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
                .collect();
            assert!(
                slot_lists_equal(&owned, claim),
                "a published ownership moved a pair its owner had claimed"
            );
        }
        peer.pair_choice = claim;
    }
    settle_preference(next_state);
    emit(
        next_state,
        protocol::MessageKind::SlotAssignment,
        Value::record(vec![
            (
                "manifest_id",
                Value::str(
                    next_state
                        .manifest_id
                        .clone()
                        .expect("publish requires a manifest"),
                ),
            ),
            (
                "assignment_id",
                Value::str(next_state.assignment_id.clone().expect("just set")),
            ),
            ("assignments", assignments),
        ]),
        link_targets(next_state, None),
        actions,
    );
}

fn freeze_session(
    next_state: &mut CoordinatorState,
    manifest: &Value,
    countdown_id: &str,
    first_input_tick: i64,
) -> Freeze {
    let assignments = next_state
        .assignments
        .clone()
        .expect("freeze requires ownership");
    slot_sources(manifest, &assignments).expect("frozen ownership must be valid");
    let mut owned: IndexMap<String, Vec<SlotId>> = IndexMap::new();
    for producer in assignments_producers(&assignments) {
        if producer_kind(producer) == ProducerKind::Peer {
            let pid = producer_id(producer);
            if !owned.contains_key(&pid) {
                let slots: Vec<SlotId> = protocol::owned_slots(&assignments, &pid)
                    .into_iter()
                    .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
                    .collect();
                owned.insert(pid, slots);
            }
        }
    }
    let live = preview_live(&assignments);
    let freeze = Freeze {
        manifest_id: next_state
            .manifest_id
            .clone()
            .expect("freeze requires a manifest id"),
        assignment_id: next_state
            .assignment_id
            .clone()
            .expect("freeze requires an assignment id"),
        countdown_id: countdown_id.to_string(),
        first_input_tick,
        seed: value_int(manifest, "seed"),
        tick_rate: value_int(manifest, "tick_rate"),
        duration_ticks: value_int(manifest, "duration_ticks"),
        max_goals: value_int(manifest, "max_goals"),
        content_id: value_str(manifest, "content_id"),
        tuning_id: value_str(manifest, "tuning_id"),
        combat_rules_id: value_str(manifest, "combat_rules_id"),
        gameplay_ai_policy_id: value_str(manifest, "gameplay_ai_policy_id"),
        combat_status: value_str(manifest, "combat_status"),
        match_mode: manifest_match_mode(manifest),
        assignments,
        owned,
        live,
    };
    next_state.freeze = Some(freeze.clone());
    freeze
}

fn record_hash(next_state: &mut CoordinatorState, tick: i64, hash: &str) {
    next_state.hashes.push(HashRecord {
        tick,
        hash: hash.to_string(),
    });
    while next_state.hashes.len() > HASH_WINDOW {
        next_state.hashes.remove(0);
    }
}

fn local_hash(state: &CoordinatorState, tick: i64) -> Option<&str> {
    state
        .hashes
        .iter()
        .rev()
        .find(|record| record.tick == tick)
        .map(|record| record.hash.as_str())
}

// ---------------------------------------------------------------------------
// Construction and read-only summaries.
// ---------------------------------------------------------------------------

/// Construct a new coordinator.
pub fn new(options: Options) -> Result<CoordinatorState> {
    let host_peer_id = options
        .host_peer_id
        .clone()
        .unwrap_or_else(|| options.peer_id.clone());
    if options.role == Role::Guest {
        let host_link_id = options.host_link_id.clone().filter(|s| !s.is_empty());
        if host_link_id.is_none() {
            return failure(
                RejectCode::Malformed,
                "a guest coordinator requires the host link id",
            );
        }
        if options.host_peer_id.is_none() {
            return failure(
                RejectCode::Malformed,
                "a guest coordinator requires the host peer id",
            );
        }
        if options.host_peer_id.as_deref() == Some(options.peer_id.as_str()) {
            return failure(
                RejectCode::RoleConflict,
                "a guest cannot claim the host peer identity",
            );
        }
    } else if let Some(declared) = &options.host_peer_id
        && declared != &options.peer_id
    {
        return failure(
            RejectCode::RoleConflict,
            "a host coordinator owns the host peer identity",
        );
    }
    let runtime = protocol::new_runtime(&options.runtime)?;
    protocol::validate_build_id(&opt_str_value(options.build_id.as_deref()))?;
    protocol::message_id(&options.session_id, &options.peer_id, 0)?;

    let self_peer = Peer {
        peer_id: options.peer_id.clone(),
        link_id: None,
        role: options.role,
        runtime: runtime.clone(),
        build_id: options.build_id.clone(),
        accepted_manifest_id: None,
        assigned: options.role == Role::Host,
        ready: false,
        started: false,
        result_acked: false,
        hash_mismatches: 0,
        last_sequence: -1,
        window: Vec::new(),
        pair_choice: None,
    };
    let mut peers = vec![self_peer];
    if options.role == Role::Guest {
        peers.push(Peer {
            peer_id: host_peer_id.clone(),
            link_id: options.host_link_id.clone(),
            role: Role::Host,
            runtime: runtime.clone(),
            build_id: None,
            accepted_manifest_id: None,
            assigned: true,
            ready: false,
            started: false,
            result_acked: false,
            hash_mismatches: 0,
            last_sequence: -1,
            window: Vec::new(),
            pair_choice: None,
        });
    }
    Ok(CoordinatorState {
        version: VERSION,
        role: options.role,
        session_id: options.session_id,
        peer_id: options.peer_id,
        host_peer_id,
        host_link_id: options.host_link_id,
        runtime,
        build_id: options.build_id,
        expectation: options.expectation,
        phase: if options.role == Role::Host {
            protocol::LifecyclePhase::Handshake
        } else {
            protocol::LifecyclePhase::New
        },
        clock: 0,
        sequence: 0,
        peers,
        manifest: None,
        manifest_id: None,
        assignments: None,
        assignment_id: None,
        assignment_epoch: 0,
        preference: None,
        freeze: None,
        countdown_remaining: None,
        start_deadline: None,
        progress: None,
        result: None,
        local_result: None,
        hashes: Vec::new(),
        departure: None,
        terminal: None,
    })
}

/// Whether the session has ended.
pub fn is_terminal(state: &CoordinatorState) -> bool {
    state.phase == protocol::LifecyclePhase::Terminal
}

/// Whether the session has frozen (countdown or later).
pub fn is_frozen(state: &CoordinatorState) -> bool {
    state.freeze.is_some()
}

/// A compact snapshot of `state`, for diagnostics/UI.
pub fn summary(state: &CoordinatorState) -> Summary {
    Summary {
        role: state.role,
        phase: state.phase,
        peer_count: state.peers.len(),
        ready_count: state.peers.iter().filter(|p| p.ready).count(),
        manifest_id: state.manifest_id.clone(),
        frozen: state.freeze.is_some(),
        terminal: state.terminal.clone(),
    }
}

// ---------------------------------------------------------------------------
// Local command handlers.
// ---------------------------------------------------------------------------

fn handshake_body(role: Role, runtime: &Value, build_id: Option<&str>) -> Value {
    Value::record(vec![
        ("role", Value::str(role.wire_str())),
        ("runtime", runtime.clone()),
        ("build_id", opt_str_value(build_id)),
    ])
}

fn handle_connect(state: &CoordinatorState) -> (CoordinatorState, Outcome) {
    if state.role != Role::Guest {
        return (
            state.clone(),
            rejected(
                RejectCode::NotPermitted,
                "only a guest opens a manual connection",
            ),
        );
    }
    if state.phase != protocol::LifecyclePhase::New {
        return (state.clone(), idempotent());
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    let body = handshake_body(
        Role::Guest,
        &next_state.runtime.clone(),
        next_state.build_id.as_deref(),
    );
    let targets = link_targets(&next_state, None);
    emit(
        &mut next_state,
        protocol::MessageKind::Handshake,
        body,
        targets,
        &mut actions,
    );
    next_state.phase = protocol::LifecyclePhase::Handshake;
    (next_state, applied(actions))
}

fn handle_propose_manifest(
    state: &CoordinatorState,
    manifest: &Value,
) -> (CoordinatorState, Outcome) {
    if state.role != Role::Host {
        return (
            state.clone(),
            rejected(
                RejectCode::NotPermitted,
                "only the host proposes the session manifest",
            ),
        );
    }
    let manifest = match protocol::new_manifest(manifest) {
        Ok(manifest) => manifest,
        Err(err) => return (state.clone(), rejected(err.code.into(), err.message)),
    };
    if value_str(&manifest, "session_id") != state.session_id {
        return (
            state.clone(),
            rejected(
                RejectCode::IdentityMismatch,
                "manifest names a different session",
            ),
        );
    }
    let manifest_id = protocol::manifest_id(&manifest);
    if let Some(existing) = &state.manifest_id {
        if *existing == manifest_id {
            return (state.clone(), idempotent());
        }
        return (
            state.clone(),
            rejected(
                RejectCode::IdentityMismatch,
                "the manifest is immutable after proposal",
            ),
        );
    }
    if state.phase != protocol::LifecyclePhase::Handshake
        && state.phase != protocol::LifecyclePhase::Manifest
    {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                format!("manifest proposal is closed in {:?}", state.phase),
            ),
        );
    }
    let shape = protocol::match_mode_shape(manifest_match_mode(&manifest));
    if state.peers.len() as i64 > shape.humans {
        return (
            state.clone(),
            rejected(
                RejectCode::Capacity,
                format!(
                    "{} seats {} humans but {} are admitted",
                    shape.mode.wire_str(),
                    shape.humans,
                    state.peers.len()
                ),
            ),
        );
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    let targets = link_targets(&next_state, None);
    emit(
        &mut next_state,
        protocol::MessageKind::ManifestProposal,
        Value::record(vec![
            ("manifest_id", Value::str(manifest_id.clone())),
            ("manifest", manifest.clone()),
        ]),
        targets,
        &mut actions,
    );
    next_state.manifest = Some(manifest);
    next_state.manifest_id = Some(manifest_id.clone());
    next_state.phase = protocol::LifecyclePhase::Manifest;
    local_peer_mut(&mut next_state).accepted_manifest_id = Some(manifest_id);
    for index in 1..next_state.peers.len() {
        let (peer_id, role, link_id) = {
            let peer = &mut next_state.peers[index];
            peer.assigned = true;
            (
                peer.peer_id.clone(),
                peer.role,
                peer.link_id.clone().expect("guest has a link"),
            )
        };
        emit(
            &mut next_state,
            protocol::MessageKind::PeerAssignment,
            Value::record(vec![
                ("assigned_peer_id", Value::str(peer_id)),
                ("role", Value::str(role.wire_str())),
            ]),
            vec![link_id],
            &mut actions,
        );
    }
    (next_state, applied(actions))
}

fn handle_assign_slots(
    state: &CoordinatorState,
    assignments: &Value,
    preserve_claims: bool,
) -> (CoordinatorState, Outcome) {
    if state.role != Role::Host {
        return (
            state.clone(),
            rejected(
                RejectCode::NotPermitted,
                "only the host publishes slot assignments",
            ),
        );
    }
    if state.phase != protocol::LifecyclePhase::Manifest
        && state.phase != protocol::LifecyclePhase::Assigned
        && state.phase != protocol::LifecyclePhase::Ready
    {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                format!("slot assignment is closed in {:?}", state.phase),
            ),
        );
    }
    for peer in &state.peers {
        if peer.accepted_manifest_id != state.manifest_id {
            return (
                state.clone(),
                rejected(
                    RejectCode::InvalidPhase,
                    "every peer must accept the manifest before assignment",
                ),
            );
        }
    }
    if let Err(err) = validate_local_assignments(state, assignments) {
        return (state.clone(), rejected(err.code, err.message));
    }
    let mut assignments = assignments.clone();
    let mut retained = None;
    if preserve_claims {
        let reseat = reseat_claims(state, &assignments);
        assignments = reseat.assignments;
        assert!(
            validate_local_assignments(state, &assignments).is_ok(),
            "a claim-preserving reseat produced ownership the host would refuse"
        );
        retained = Some(reseat.retained);
    }
    if assignments_equal(state.assignments.as_ref(), &assignments) {
        return (state.clone(), idempotent());
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    publish_ownership(&mut next_state, assignments, retained, &mut actions);
    (next_state, applied(actions))
}

fn slots_value(slots: &[SlotId]) -> Value {
    Value::array(
        slots
            .iter()
            .map(|s| Value::str(protocol::slot_wire_id(*s)))
            .collect(),
    )
}

fn handle_prefer_pair(state: &CoordinatorState, slots: &[SlotId]) -> (CoordinatorState, Outcome) {
    if state.phase != protocol::LifecyclePhase::Assigned
        && state.phase != protocol::LifecyclePhase::Ready
    {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                format!("pair preference is closed in {:?}", state.phase),
            ),
        );
    }
    if state.manifest_id.is_none() || state.assignment_id.is_none() {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "a pair preference answers a published ownership generation",
            ),
        );
    }
    if let Err(err) = protocol::validate_slot_set(&slots_value(slots)) {
        return (state.clone(), rejected(err.code.into(), err.message));
    }
    let slots = slots.to_vec();
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    if state.role == Role::Guest {
        next_state.preference = Some(Preference {
            slots: slots.clone(),
            assignment_id: state.assignment_id.clone().unwrap(),
            status: PreferenceState::Pending,
            reason: None,
            deadline: Some(state.clock + PREFERENCE_TIMEOUT_TICKS),
        });
        let targets = link_targets(&next_state, None);
        emit(
            &mut next_state,
            protocol::MessageKind::PairPreference,
            Value::record(vec![
                (
                    "manifest_id",
                    Value::str(state.manifest_id.clone().unwrap()),
                ),
                (
                    "assignment_id",
                    Value::str(state.assignment_id.clone().unwrap()),
                ),
                ("slots", slots_value(&slots)),
            ]),
            targets,
            &mut actions,
        );
        return (next_state, applied(actions));
    }
    let verdict = evaluate_preference(state, &state.peer_id, &slots);
    next_state.preference = Some(Preference {
        slots: slots.clone(),
        assignment_id: state.assignment_id.clone().unwrap(),
        status: verdict.status,
        reason: verdict.reason,
        deadline: None,
    });
    match verdict.status {
        PreferenceState::Granted => {
            publish_ownership(
                &mut next_state,
                verdict
                    .assignments
                    .expect("granted verdict carries assignments"),
                Some(claims_after(state, &state.peer_id, &slots)),
                &mut actions,
            );
        }
        PreferenceState::Unchanged => {
            local_peer_mut(&mut next_state).pair_choice = Some(slots);
        }
        _ => {}
    }
    (next_state, applied(actions))
}

fn handle_set_ready(state: &CoordinatorState, ready: bool) -> (CoordinatorState, Outcome) {
    if state.phase != protocol::LifecyclePhase::Assigned
        && state.phase != protocol::LifecyclePhase::Ready
    {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                format!("readiness is closed in {:?}", state.phase),
            ),
        );
    }
    let peer = local_peer(state);
    if ready {
        if peer.accepted_manifest_id != state.manifest_id {
            return (
                state.clone(),
                rejected(
                    RejectCode::IdentityMismatch,
                    "readiness requires accepting the exact manifest",
                ),
            );
        }
        if !owns_slot(state, &peer.peer_id) {
            return (
                state.clone(),
                rejected(
                    RejectCode::InvalidAssignment,
                    "readiness requires an owned set in this generation",
                ),
            );
        }
    }
    if peer.ready == ready {
        return (state.clone(), idempotent());
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    if next_state.role == Role::Guest {
        let manifest_id = next_state.manifest_id.clone().unwrap();
        let assignment_id = next_state.assignment_id.clone().unwrap();
        let targets = link_targets(&next_state, None);
        emit(
            &mut next_state,
            protocol::MessageKind::Ready,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("assignment_id", Value::str(assignment_id)),
                ("ready", Value::bool(ready)),
            ]),
            targets,
            &mut actions,
        );
    }
    local_peer_mut(&mut next_state).ready = ready;
    refresh_ready_phase(&mut next_state);
    (next_state, applied(actions))
}

fn emit_start(next_state: &mut CoordinatorState, actions: &mut Vec<Action>) {
    let freeze = next_state.freeze.clone().expect("start requires a freeze");
    emit(
        next_state,
        protocol::MessageKind::Start,
        Value::record(vec![
            ("manifest_id", Value::str(freeze.manifest_id.clone())),
            ("countdown_id", Value::str(freeze.countdown_id.clone())),
            ("first_input_tick", Value::int(freeze.first_input_tick)),
        ]),
        link_targets(next_state, None),
        actions,
    );
    next_state.countdown_remaining = Some(0);
    next_state.start_deadline = Some(next_state.clock + START_ACK_TIMEOUT_TICKS);
    let pending = next_state.peers[1..].iter().any(|p| !p.started);
    if !pending {
        next_state.phase = protocol::LifecyclePhase::Running;
        next_state.start_deadline = None;
        actions.push(Action::StartMatch {
            freeze: Box::new(freeze),
        });
    }
}

fn handle_begin_countdown(
    state: &CoordinatorState,
    countdown_id: &str,
    remaining_ticks: i64,
    first_input_tick: i64,
) -> (CoordinatorState, Outcome) {
    if state.role != Role::Host {
        return (
            state.clone(),
            rejected(
                RejectCode::NotPermitted,
                "only the host starts the countdown",
            ),
        );
    }
    if state.phase != protocol::LifecyclePhase::Ready {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "countdown requires every peer to be ready",
            ),
        );
    }
    if !(0..=protocol::MAX_COUNTDOWN_TICKS).contains(&remaining_ticks) {
        return (
            state.clone(),
            rejected(
                RejectCode::Malformed,
                "countdown length is outside the protocol bound",
            ),
        );
    }
    if !(0..=input_frame::MAX_TICK).contains(&first_input_tick) {
        return (
            state.clone(),
            rejected(
                RejectCode::Malformed,
                "the first input tick is outside the input bound",
            ),
        );
    }
    let manifest = state
        .manifest
        .clone()
        .expect("countdown requires a manifest");
    let empty = Value::array(Vec::new());
    if let Err(err) =
        validate_local_assignments(state, state.assignments.as_ref().unwrap_or(&empty))
    {
        return (state.clone(), rejected(err.code, err.message));
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    let freeze = freeze_session(&mut next_state, &manifest, countdown_id, first_input_tick);
    next_state.countdown_remaining = Some(remaining_ticks);
    let targets = link_targets(&next_state, None);
    if let Err(err) = try_emit(
        &mut next_state,
        protocol::MessageKind::Countdown,
        Value::record(vec![
            ("manifest_id", Value::str(freeze.manifest_id.clone())),
            ("countdown_id", Value::str(freeze.countdown_id.clone())),
            ("remaining_ticks", Value::int(remaining_ticks)),
            ("first_input_tick", Value::int(freeze.first_input_tick)),
        ]),
        targets,
        &mut actions,
    ) {
        return (state.clone(), rejected(err.code.into(), err.message));
    }
    next_state.phase = protocol::LifecyclePhase::Countdown;
    if remaining_ticks == 0 {
        emit_start(&mut next_state, &mut actions);
    }
    (next_state, applied(actions))
}

fn expire_preference(next_state: &mut CoordinatorState) {
    let Some(pending) = next_state.preference.clone() else {
        return;
    };
    if pending.status != PreferenceState::Pending {
        return;
    }
    let deadline = pending
        .deadline
        .expect("a pending pair preference must carry a deadline");
    if next_state.clock <= deadline {
        return;
    }
    next_state.preference = Some(Preference {
        slots: pending.slots,
        assignment_id: pending.assignment_id,
        status: PreferenceState::Rejected,
        reason: Some("no_response".to_string()),
        deadline: None,
    });
}

fn handle_tick(state: &CoordinatorState) -> (CoordinatorState, Outcome) {
    let mut next_state = state.clone();
    next_state.clock = state.clock + 1;
    if state.phase == protocol::LifecyclePhase::Terminal {
        return (next_state, applied(Vec::new()));
    }
    expire_preference(&mut next_state);
    let mut actions = Vec::new();
    if state.phase == protocol::LifecyclePhase::Countdown {
        let remaining = next_state.countdown_remaining.unwrap_or(0);
        if remaining > 0 {
            next_state.countdown_remaining = Some(remaining - 1);
            if next_state.countdown_remaining == Some(0) && next_state.role == Role::Host {
                emit_start(&mut next_state, &mut actions);
            }
        } else if next_state.role == Role::Host
            && next_state
                .start_deadline
                .is_some_and(|deadline| next_state.clock > deadline)
        {
            let missing = next_state.peers[1..]
                .iter()
                .find(|p| !p.started)
                .map(|p| p.peer_id.clone());
            terminate_session(
                &mut next_state,
                TerminationOptions {
                    reason: TerminalReason::StartAckTimeout,
                    origin: OriginOrDefault::Timeout,
                    peer_id: missing,
                    detail: Some(
                        "a peer never acknowledged the canonical start boundary".to_string(),
                    ),
                    announce: true,
                    ..Default::default()
                },
                &mut actions,
            );
            return (next_state, applied(actions));
        }
    }
    (next_state, applied(actions))
}

fn match_phase_body(phase: &str, tick: i64, home_score: i64, away_score: i64) -> Value {
    Value::record(vec![
        ("phase", Value::str(phase)),
        ("tick", Value::int(tick)),
        ("home_score", Value::int(home_score)),
        ("away_score", Value::int(away_score)),
    ])
}

fn handle_match_phase(
    state: &CoordinatorState,
    phase: &str,
    tick: i64,
    home_score: i64,
    away_score: i64,
) -> (CoordinatorState, Outcome) {
    if state.role != Role::Host {
        return (
            state.clone(),
            rejected(
                RejectCode::NotPermitted,
                "only the host publishes match phases",
            ),
        );
    }
    if state.phase != protocol::LifecyclePhase::Running {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "match phases require a running session",
            ),
        );
    }
    if let Some(progress) = &state.progress {
        if progress.phase == phase
            && progress.tick == tick
            && progress.home_score == home_score
            && progress.away_score == away_score
        {
            return (state.clone(), idempotent());
        }
        if !match_phase_allowed(&progress.phase, phase) {
            return (
                state.clone(),
                rejected(
                    RejectCode::InvalidPhase,
                    format!("match phase {phase} cannot follow {}", progress.phase),
                ),
            );
        }
        if tick < progress.tick {
            return (
                state.clone(),
                rejected(
                    RejectCode::Malformed,
                    "simulation ticks never move backwards",
                ),
            );
        }
        if home_score < progress.home_score || away_score < progress.away_score {
            return (
                state.clone(),
                rejected(
                    RejectCode::Malformed,
                    "simulation scores never move backwards",
                ),
            );
        }
        if phase == "goal_stoppage"
            && home_score + away_score <= progress.home_score + progress.away_score
        {
            return (
                state.clone(),
                rejected(
                    RejectCode::Malformed,
                    "a goal stoppage must follow a scored goal",
                ),
            );
        }
    } else if phase != "kickoff" {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "a running match opens with kickoff",
            ),
        );
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    let targets = link_targets(&next_state, None);
    if let Err(err) = try_emit(
        &mut next_state,
        protocol::MessageKind::MatchPhase,
        match_phase_body(phase, tick, home_score, away_score),
        targets,
        &mut actions,
    ) {
        return (state.clone(), rejected(err.code.into(), err.message));
    }
    next_state.progress = Some(MatchProgress {
        phase: phase.to_string(),
        tick,
        home_score,
        away_score,
    });
    (next_state, applied(actions))
}

fn handle_hash_report(
    state: &CoordinatorState,
    tick: i64,
    boundary_hash: &str,
) -> (CoordinatorState, Outcome) {
    if state.phase != protocol::LifecyclePhase::Running {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "boundary hashes require a running session",
            ),
        );
    }
    if let Some(existing) = local_hash(state, tick) {
        if existing == boundary_hash {
            return (state.clone(), idempotent());
        }
        return (
            state.clone(),
            rejected(
                RejectCode::Duplicate,
                "a local boundary tick reported two different hashes",
            ),
        );
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    let targets = link_targets(&next_state, None);
    if let Err(err) = try_emit(
        &mut next_state,
        protocol::MessageKind::HashReport,
        Value::record(vec![
            ("tick", Value::int(tick)),
            ("boundary_hash", Value::str(boundary_hash)),
        ]),
        targets,
        &mut actions,
    ) {
        return (state.clone(), rejected(err.code.into(), err.message));
    }
    record_hash(&mut next_state, tick, boundary_hash);
    (next_state, applied(actions))
}

fn result_ack_body(result: &MatchResult) -> Value {
    Value::record(vec![
        ("final_tick", Value::int(result.final_tick)),
        ("home_score", Value::int(result.home_score)),
        ("away_score", Value::int(result.away_score)),
        ("final_hash", Value::str(result.final_hash.clone())),
    ])
}

fn handle_finish(
    state: &CoordinatorState,
    final_tick: i64,
    home_score: i64,
    away_score: i64,
    final_hash: &str,
) -> (CoordinatorState, Outcome) {
    let result = MatchResult {
        final_tick,
        home_score,
        away_score,
        final_hash: final_hash.to_string(),
    };
    if state.role == Role::Guest {
        if state.phase != protocol::LifecyclePhase::Running
            && state.phase != protocol::LifecyclePhase::Result
        {
            return (
                state.clone(),
                rejected(
                    RejectCode::InvalidPhase,
                    "a result requires a running session",
                ),
            );
        }
        let mut next_state = state.clone();
        next_state.local_result = Some(result);
        return (next_state, applied(Vec::new()));
    }
    if state.phase != protocol::LifecyclePhase::Running {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "a result requires a running session",
            ),
        );
    }
    let Some(progress) = &state.progress else {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "the simulation must reach full time first",
            ),
        );
    };
    if progress.phase != "full_time" {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidPhase,
                "the simulation must reach full time first",
            ),
        );
    }
    if result.home_score != progress.home_score || result.away_score != progress.away_score {
        return (
            state.clone(),
            rejected(
                RejectCode::IdentityMismatch,
                "the coordinator never restates the score",
            ),
        );
    }
    if result.final_tick < progress.tick {
        return (
            state.clone(),
            rejected(RejectCode::Malformed, "the final tick precedes full time"),
        );
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    next_state.phase = protocol::LifecyclePhase::Result;
    next_state.result = Some(result.clone());
    next_state.local_result = Some(result.clone());
    let targets = link_targets(&next_state, None);
    let emitted = try_emit(
        &mut next_state,
        protocol::MessageKind::MatchPhase,
        match_phase_body(
            "result",
            result.final_tick,
            result.home_score,
            result.away_score,
        ),
        targets.clone(),
        &mut actions,
    )
    .and_then(|()| {
        try_emit(
            &mut next_state,
            protocol::MessageKind::ResultAck,
            result_ack_body(&result),
            targets.clone(),
            &mut actions,
        )
    });
    if let Err(err) = emitted {
        return (state.clone(), rejected(err.code.into(), err.message));
    }
    next_state.progress = Some(MatchProgress {
        phase: "result".to_string(),
        tick: result.final_tick,
        home_score: result.home_score,
        away_score: result.away_score,
    });
    local_peer_mut(&mut next_state).result_acked = true;
    if targets.is_empty() {
        terminate_session(
            &mut next_state,
            TerminationOptions {
                reason: TerminalReason::Completed,
                origin: OriginOrDefault::Local,
                ..Default::default()
            },
            &mut actions,
        );
    }
    (next_state, applied(actions))
}

fn handle_netcode_failure(
    state: &CoordinatorState,
    failure: &str,
    peer_id: Option<String>,
    detail: Option<String>,
) -> (CoordinatorState, Outcome) {
    let Some(reason) = netcode_reason(failure) else {
        return (
            state.clone(),
            rejected(RejectCode::Malformed, "unknown netcode failure class"),
        );
    };
    terminate_from(
        state,
        TerminationOptions {
            reason,
            origin: OriginOrDefault::Local,
            peer_id,
            detail,
            announce: true,
            ..Default::default()
        },
    )
}

fn handle_abort(
    state: &CoordinatorState,
    code: Option<String>,
    detail: Option<String>,
) -> (CoordinatorState, Outcome) {
    terminate_from(
        state,
        TerminationOptions {
            reason: TerminalReason::LocalAbort,
            code: Some(code.unwrap_or_else(|| "host_abort".to_string())),
            origin: OriginOrDefault::Local,
            detail,
            announce: true,
            ..Default::default()
        },
    )
}

fn handle_leave(state: &CoordinatorState) -> (CoordinatorState, Outcome) {
    if state.role != Role::Guest {
        return (
            state.clone(),
            rejected(
                RejectCode::NotPermitted,
                "the host ends a session with abort",
            ),
        );
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    if state.phase != protocol::LifecyclePhase::New {
        let peer_id = next_state.peer_id.clone();
        let targets = link_targets(&next_state, None);
        emit(
            &mut next_state,
            protocol::MessageKind::Disconnect,
            Value::record(vec![
                ("target_peer_id", Value::str(peer_id)),
                ("code", Value::str("peer_left")),
            ]),
            targets,
            &mut actions,
        );
    }
    let peer_id = next_state.peer_id.clone();
    terminate_session(
        &mut next_state,
        TerminationOptions {
            reason: TerminalReason::GuestLeft,
            origin: OriginOrDefault::Local,
            peer_id: Some(peer_id),
            ..Default::default()
        },
        &mut actions,
    );
    (next_state, applied(actions))
}

fn remove_peer(next_state: &mut CoordinatorState, peer_id: &str) {
    next_state
        .peers
        .retain(|peer| peer.peer_id != peer_id || peer.link_id.is_none());
}

fn build_skew(state: &CoordinatorState, peer: &Peer) -> bool {
    state.build_id.is_some() && peer.build_id != state.build_id
}

fn skew_reason(state: &CoordinatorState, peer: &Peer) -> Option<TerminalReason> {
    build_skew(state, peer).then_some(TerminalReason::BuildMismatch)
}

fn drop_guest(
    state: &CoordinatorState,
    peer: &Peer,
    code: &str,
    detail: &str,
    reason: Option<TerminalReason>,
) -> (CoordinatorState, Outcome) {
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    let departed = peer.peer_id.clone();
    next_state.departure = Some(Departure {
        peer_id: departed.clone(),
        reason: reason.unwrap_or_else(|| disconnect_reason(code)),
        code: code.to_string(),
        detail: Some(detail.to_string()),
    });
    let targets = link_targets(&next_state, None);
    emit(
        &mut next_state,
        protocol::MessageKind::Disconnect,
        Value::record(vec![
            ("target_peer_id", Value::str(departed.clone())),
            ("code", Value::str(code)),
        ]),
        targets,
        &mut actions,
    );
    remove_peer(&mut next_state, &departed);
    clear_readiness(&mut next_state);
    if owns_slot(state, &departed) {
        next_state.assignments = None;
        next_state.assignment_id = None;
    }
    actions.push(Action::Close {
        link_id: peer.link_id.clone().expect("a dropped guest has a link"),
    });
    (next_state, applied(actions))
}

fn handle_link_lost(
    state: &CoordinatorState,
    link_id: &str,
    code: Option<&str>,
) -> (CoordinatorState, Outcome) {
    let code = code.unwrap_or("transport_lost");
    if !matches!(
        code,
        "peer_left" | "transport_lost" | "host_left" | "protocol_error"
    ) {
        return (
            state.clone(),
            rejected(RejectCode::Malformed, "unknown transport disconnect code"),
        );
    }
    let Some(peer) = peer_by_link(state, link_id).cloned() else {
        return (
            state.clone(),
            rejected(RejectCode::UnknownLink, "no admitted peer owns that link"),
        );
    };
    if peer.role == Role::Host {
        return terminate_from(
            state,
            TerminationOptions {
                reason: if code == "transport_lost" {
                    TerminalReason::TransportLost
                } else {
                    TerminalReason::HostLeft
                },
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                ..Default::default()
            },
        );
    }
    if state.freeze.is_some() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: if code == "transport_lost" {
                    TerminalReason::TransportLost
                } else {
                    TerminalReason::GuestLeft
                },
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                announce: true,
                exclude_link: peer.link_id.clone(),
                ..Default::default()
            },
        );
    }
    drop_guest(
        state,
        &peer,
        code,
        &format!("a guest's link ended as {code}"),
        None,
    )
}

// ---------------------------------------------------------------------------
// Control message handlers.
// ---------------------------------------------------------------------------

fn reject_link(
    state: &CoordinatorState,
    link_id: &str,
    code: &str,
    reason: &str,
) -> (CoordinatorState, Outcome) {
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    emit(
        &mut next_state,
        protocol::MessageKind::Abort,
        Value::record(vec![("code", Value::str(code))]),
        vec![link_id.to_string()],
        &mut actions,
    );
    actions.push(Action::Close {
        link_id: link_id.to_string(),
    });
    (
        next_state,
        Outcome {
            accepted: false,
            disposition: Disposition::Rejected,
            code: Some(reject_code_from_session_reject(code)),
            reason: Some(reason.to_string()),
            actions,
        },
    )
}

fn admit_guest(
    state: &CoordinatorState,
    link_id: &str,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    if state.role != Role::Host {
        return reject_link(
            state,
            link_id,
            "protocol_mismatch",
            "a guest never admits peers",
        );
    }
    if state.phase != protocol::LifecyclePhase::New
        && state.phase != protocol::LifecyclePhase::Handshake
    {
        return reject_link(
            state,
            link_id,
            "invalid_phase",
            "admission closes when the manifest is proposed",
        );
    }
    if value_str(body, "role") != "guest" {
        return reject_link(
            state,
            link_id,
            "protocol_mismatch",
            "a session has one host",
        );
    }
    if peer_by_id(state, &message.peer_id).is_some() {
        return reject_link(
            state,
            link_id,
            "protocol_mismatch",
            "peer id is already admitted",
        );
    }
    if state.peers.len() as i64 >= MAX_PEERS {
        return reject_link(state, link_id, "capacity", "the session is full");
    }
    let runtime = body.get("runtime").cloned().unwrap_or(Value::Nil);
    if let Err(err) = protocol::compare_runtime(&state.runtime, &runtime) {
        return reject_link(state, link_id, "runtime_mismatch", &err.message);
    }
    let mut next_state = state.clone();
    next_state.departure = None;
    let wire = protocol::encode(message).expect("an already-validated message encodes");
    next_state.peers.push(Peer {
        peer_id: message.peer_id.clone(),
        link_id: Some(link_id.to_string()),
        role: Role::Guest,
        runtime,
        build_id: value_opt_str(body, "build_id"),
        accepted_manifest_id: None,
        assigned: false,
        ready: false,
        started: false,
        result_acked: false,
        hash_mismatches: 0,
        last_sequence: message.sequence,
        window: vec![WireRecord {
            sequence: message.sequence,
            wire,
        }],
        pair_choice: None,
    });
    next_state.phase = protocol::LifecyclePhase::Handshake;
    (next_state, applied(Vec::new()))
}

fn apply_manifest_proposal(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let manifest_id = value_str(body, "manifest_id");
    if let Some(existing) = &state.manifest_id {
        if *existing == manifest_id {
            return (state.clone(), idempotent());
        }
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("the manifest is immutable after proposal".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let manifest = body.get("manifest").cloned().unwrap_or(Value::Nil);
    if let Some(difference) = expectation_difference(state.expectation.as_ref(), &manifest) {
        return terminate_from(
            state,
            TerminationOptions {
                reason: identity_reason(&difference.path),
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some(format!("local identity differs at {}", difference.path)),
                announce: true,
                ..Default::default()
            },
        );
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    next_state.manifest = Some(manifest);
    next_state.manifest_id = Some(manifest_id.clone());
    next_state.phase = protocol::LifecyclePhase::Manifest;
    local_peer_mut(&mut next_state).accepted_manifest_id = Some(manifest_id.clone());
    let targets = link_targets(&next_state, None);
    emit(
        &mut next_state,
        protocol::MessageKind::ManifestAccept,
        Value::record(vec![("manifest_id", Value::str(manifest_id))]),
        targets,
        &mut actions,
    );
    (next_state, applied(actions))
}

fn apply_manifest_accept(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let manifest_id = value_str(&message.body, "manifest_id");
    if Some(&manifest_id) != state.manifest_id.as_ref() {
        return drop_guest(
            state,
            peer,
            "protocol_error",
            "a guest accepted a manifest this session never proposed",
            skew_reason(state, peer),
        );
    }
    if peer.accepted_manifest_id.as_ref() == Some(&manifest_id) {
        return (state.clone(), idempotent());
    }
    let mut next_state = state.clone();
    peer_by_id_mut(&mut next_state, &peer.peer_id)
        .expect("peer exists")
        .accepted_manifest_id = Some(manifest_id);
    (next_state, applied(Vec::new()))
}

fn apply_peer_assignment(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let assigned_peer_id = value_str(body, "assigned_peer_id");
    let role = Role::from_wire_str(&value_str(body, "role"));
    if assigned_peer_id != state.peer_id || role != Some(state.role) {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("the host named a different peer identity".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    if local_peer(state).assigned {
        return (state.clone(), idempotent());
    }
    let mut next_state = state.clone();
    local_peer_mut(&mut next_state).assigned = true;
    (next_state, applied(Vec::new()))
}

fn apply_slot_assignment(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let manifest_id = value_str(body, "manifest_id");
    let Some(manifest) = &state.manifest else {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("slot assignment names a different manifest".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    };
    if Some(&manifest_id) != state.manifest_id.as_ref() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("slot assignment names a different manifest".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let assignments = body.get("assignments").cloned().unwrap_or(Value::Nil);
    if let Err(err) = slot_sources(manifest, &assignments) {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::InvalidAssignment,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some(err.message),
                announce: true,
                ..Default::default()
            },
        );
    }
    let owned = assignments_producers(&assignments)
        .into_iter()
        .any(|producer| {
            producer_kind(producer) == ProducerKind::Peer && producer_id(producer) == state.peer_id
        });
    if !owned {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::InvalidAssignment,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("the published ownership seats no slot for this peer".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let assignment_id = value_str(body, "assignment_id");
    if Some(&assignment_id) == state.assignment_id.as_ref()
        && assignments_equal(state.assignments.as_ref(), &assignments)
    {
        return (state.clone(), idempotent());
    }
    let mut next_state = state.clone();
    next_state.assignments = Some(assignments);
    next_state.assignment_id = Some(assignment_id);
    next_state.phase = protocol::LifecyclePhase::Assigned;
    clear_readiness(&mut next_state);
    settle_preference(&mut next_state);
    (next_state, applied(Vec::new()))
}

fn apply_ready(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let manifest_id = value_str(body, "manifest_id");
    if Some(&manifest_id) != state.manifest_id.as_ref() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("readiness names a different manifest".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let ready = body.get("ready").and_then(Value::as_bool).unwrap_or(false);
    if ready && peer.accepted_manifest_id.as_ref() != state.manifest_id.as_ref() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("readiness preceded manifest acceptance".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    if ready && !owns_slot(state, &peer.peer_id) {
        return (
            state.clone(),
            rejected(
                RejectCode::InvalidAssignment,
                "readiness for voided slot ownership",
            ),
        );
    }
    let assignment_id = value_str(body, "assignment_id");
    if Some(&assignment_id) != state.assignment_id.as_ref() {
        return (
            state.clone(),
            rejected(RejectCode::InvalidAssignment, STALE_GENERATION_REASON),
        );
    }
    if peer.ready == ready {
        return (state.clone(), idempotent());
    }
    let mut next_state = state.clone();
    peer_by_id_mut(&mut next_state, &peer.peer_id)
        .expect("peer exists")
        .ready = ready;
    refresh_ready_phase(&mut next_state);
    (next_state, applied(Vec::new()))
}

fn apply_pair_preference(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let manifest_id = value_str(body, "manifest_id");
    if Some(&manifest_id) != state.manifest_id.as_ref() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("pair preference names a different manifest".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let assignment_id = value_str(body, "assignment_id");
    let slots_wire = body.get("slots").cloned().unwrap_or(Value::Nil);
    let slots: Vec<SlotId> = (1..=slots_wire.len() as i64)
        .filter_map(|i| {
            slots_wire
                .get_index(i)
                .and_then(Value::as_str)
                .and_then(protocol::slot_id_from_wire)
        })
        .collect();
    let verdict = if state.freeze.is_some() {
        PreferenceVerdict {
            status: PreferenceState::Rejected,
            reason: Some("after_freeze".to_string()),
            slots: slots.clone(),
            assignments: None,
        }
    } else if Some(&assignment_id) != state.assignment_id.as_ref() {
        PreferenceVerdict {
            status: PreferenceState::Rejected,
            reason: Some("superseded".to_string()),
            slots: slots.clone(),
            assignments: None,
        }
    } else {
        evaluate_preference(state, &peer.peer_id, &slots)
    };
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    match verdict.status {
        PreferenceState::Granted => {
            publish_ownership(
                &mut next_state,
                verdict
                    .assignments
                    .clone()
                    .expect("granted verdict carries assignments"),
                Some(claims_after(state, &peer.peer_id, &verdict.slots)),
                &mut actions,
            );
        }
        PreferenceState::Unchanged => {
            peer_by_id_mut(&mut next_state, &peer.peer_id)
                .expect("peer exists")
                .pair_choice = Some(verdict.slots.clone());
        }
        _ => {}
    }
    emit(
        &mut next_state,
        protocol::MessageKind::PairPreferenceResult,
        Value::record(vec![
            (
                "manifest_id",
                Value::str(state.manifest_id.clone().unwrap()),
            ),
            ("assignment_id", Value::str(assignment_id)),
            ("slots", slots_value(&verdict.slots)),
            ("status", Value::str(verdict.status.wire_str())),
            (
                "reason",
                verdict
                    .reason
                    .as_deref()
                    .map(Value::str)
                    .unwrap_or(Value::Nil),
            ),
        ]),
        vec![
            peer.link_id
                .clone()
                .expect("a control-message peer has a link"),
        ],
        &mut actions,
    );
    (next_state, applied(actions))
}

fn apply_pair_preference_result(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let manifest_id = value_str(body, "manifest_id");
    if Some(&manifest_id) != state.manifest_id.as_ref() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("pair preference result names a different manifest".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let assignment_id = value_str(body, "assignment_id");
    let slots_wire = body.get("slots").cloned().unwrap_or(Value::Nil);
    let slots: Vec<SlotId> = (1..=slots_wire.len() as i64)
        .filter_map(|i| {
            slots_wire
                .get_index(i)
                .and_then(Value::as_str)
                .and_then(protocol::slot_id_from_wire)
        })
        .collect();
    let matches_pending = state.preference.as_ref().is_some_and(|pending| {
        pending.status == PreferenceState::Pending
            && pending.assignment_id == assignment_id
            && slot_lists_equal(&pending.slots, &slots)
    });
    if !matches_pending {
        return (state.clone(), stale(STALE_PREFERENCE_REASON));
    }
    let pending = state.preference.clone().unwrap();
    let status = PreferenceState::from_wire_str(&value_str(body, "status"))
        .expect("already-validated status");
    let reason = value_opt_str(body, "reason");
    let mut next_state = state.clone();
    next_state.preference = Some(Preference {
        slots: pending.slots,
        assignment_id: pending.assignment_id,
        status,
        reason,
        deadline: None,
    });
    (next_state, applied(Vec::new()))
}

fn apply_countdown(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let manifest_id = value_str(body, "manifest_id");
    if Some(&manifest_id) != state.manifest_id.as_ref() {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ManifestMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("countdown names a different manifest".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let countdown_id = value_str(body, "countdown_id");
    if let Some(freeze) = &state.freeze {
        if freeze.countdown_id == countdown_id {
            return (state.clone(), idempotent());
        }
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("a frozen countdown cannot be restarted".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let mut next_state = state.clone();
    let manifest = next_state
        .manifest
        .clone()
        .expect("countdown requires a manifest");
    freeze_session(
        &mut next_state,
        &manifest,
        &countdown_id,
        value_int(body, "first_input_tick"),
    );
    next_state.countdown_remaining = Some(value_int(body, "remaining_ticks"));
    next_state.phase = protocol::LifecyclePhase::Countdown;
    (next_state, applied(Vec::new()))
}

fn apply_start(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let Some(freeze) = state.freeze.clone() else {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("start does not name the frozen countdown boundary".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    };
    if value_str(body, "manifest_id") != freeze.manifest_id
        || value_str(body, "countdown_id") != freeze.countdown_id
        || value_int(body, "first_input_tick") != freeze.first_input_tick
    {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("start does not name the frozen countdown boundary".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    if state.role == Role::Guest {
        if local_peer(state).started {
            return (state.clone(), idempotent());
        }
        let mut actions = Vec::new();
        let mut next_state = state.clone();
        let targets = link_targets(&next_state, None);
        emit(
            &mut next_state,
            protocol::MessageKind::Start,
            Value::record(vec![
                ("manifest_id", Value::str(freeze.manifest_id.clone())),
                ("countdown_id", Value::str(freeze.countdown_id.clone())),
                ("first_input_tick", Value::int(freeze.first_input_tick)),
            ]),
            targets,
            &mut actions,
        );
        local_peer_mut(&mut next_state).started = true;
        next_state.countdown_remaining = Some(0);
        next_state.phase = protocol::LifecyclePhase::Running;
        actions.push(Action::StartMatch {
            freeze: Box::new(freeze),
        });
        return (next_state, applied(actions));
    }
    if peer.started {
        return (state.clone(), idempotent());
    }
    let mut actions = Vec::new();
    let mut next_state = state.clone();
    peer_by_id_mut(&mut next_state, &peer.peer_id)
        .expect("peer exists")
        .started = true;
    let pending = next_state.peers[1..].iter().any(|p| !p.started);
    if !pending && next_state.countdown_remaining == Some(0) {
        next_state.phase = protocol::LifecyclePhase::Running;
        next_state.start_deadline = None;
        actions.push(Action::StartMatch {
            freeze: Box::new(freeze),
        });
    }
    (next_state, applied(actions))
}

fn apply_match_phase(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let phase = value_str(body, "phase");
    let tick = value_int(body, "tick");
    let home_score = value_int(body, "home_score");
    let away_score = value_int(body, "away_score");
    if let Some(progress) = &state.progress {
        if progress.phase == phase
            && progress.tick == tick
            && progress.home_score == home_score
            && progress.away_score == away_score
        {
            return (state.clone(), idempotent());
        }
        let ordered = match_phase_allowed(&progress.phase, &phase)
            || (phase == "result" && progress.phase == "full_time");
        if !ordered || tick < progress.tick {
            return terminate_from(
                state,
                TerminationOptions {
                    reason: TerminalReason::ProtocolViolation,
                    origin: OriginOrDefault::Remote,
                    peer_id: Some(peer.peer_id.clone()),
                    detail: Some("match phase ordering regressed".to_string()),
                    announce: true,
                    ..Default::default()
                },
            );
        }
    } else if phase != "kickoff" {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("a running match opens with kickoff".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    let mut next_state = state.clone();
    next_state.progress = Some(MatchProgress {
        phase: phase.clone(),
        tick,
        home_score,
        away_score,
    });
    if phase == "full_time" || phase == "result" {
        next_state.phase = protocol::LifecyclePhase::Result;
    }
    (next_state, applied(Vec::new()))
}

fn apply_hash_report(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let tick = value_int(body, "tick");
    let boundary_hash = value_str(body, "boundary_hash");
    let Some(expected) = local_hash(state, tick).map(str::to_string) else {
        return (state.clone(), applied(Vec::new()));
    };
    let mut next_state = state.clone();
    let tracked = peer_by_id_mut(&mut next_state, &peer.peer_id).expect("peer exists");
    if expected == boundary_hash {
        tracked.hash_mismatches = 0;
        return (next_state, applied(Vec::new()));
    }
    tracked.hash_mismatches += 1;
    if tracked.hash_mismatches < MAX_HASH_MISMATCHES {
        return (next_state, applied(Vec::new()));
    }
    let mismatches = tracked.hash_mismatches;
    let mut actions = Vec::new();
    terminate_session(
        &mut next_state,
        TerminationOptions {
            reason: TerminalReason::HashMismatch,
            origin: OriginOrDefault::Remote,
            peer_id: Some(peer.peer_id.clone()),
            detail: Some(format!(
                "{mismatches} consecutive boundary hashes disagreed"
            )),
            announce: true,
            ..Default::default()
        },
        &mut actions,
    );
    (next_state, applied(actions))
}

fn apply_result_ack(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let reported = MatchResult {
        final_tick: value_int(body, "final_tick"),
        home_score: value_int(body, "home_score"),
        away_score: value_int(body, "away_score"),
        final_hash: value_str(body, "final_hash"),
    };
    let known = state.result.as_ref().or(state.local_result.as_ref());
    if let Some(known) = known
        && (known.final_tick != reported.final_tick
            || known.home_score != reported.home_score
            || known.away_score != reported.away_score
            || known.final_hash != reported.final_hash)
    {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::HashMismatch,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some(
                    "the acknowledged result differs from the simulation result".to_string(),
                ),
                announce: true,
                ..Default::default()
            },
        );
    }
    if state.role == Role::Guest {
        if local_peer(state).result_acked {
            return (state.clone(), idempotent());
        }
        let mut actions = Vec::new();
        let mut next_state = state.clone();
        next_state.result = Some(reported.clone());
        next_state.phase = protocol::LifecyclePhase::Result;
        let targets = link_targets(&next_state, None);
        emit(
            &mut next_state,
            protocol::MessageKind::ResultAck,
            result_ack_body(&reported),
            targets,
            &mut actions,
        );
        local_peer_mut(&mut next_state).result_acked = true;
        terminate_session(
            &mut next_state,
            TerminationOptions {
                reason: TerminalReason::Completed,
                origin: OriginOrDefault::Remote,
                ..Default::default()
            },
            &mut actions,
        );
        return (next_state, applied(actions));
    }
    if peer.result_acked {
        return (state.clone(), idempotent());
    }
    let mut next_state = state.clone();
    peer_by_id_mut(&mut next_state, &peer.peer_id)
        .expect("peer exists")
        .result_acked = true;
    let pending = next_state.peers.iter().any(|p| !p.result_acked);
    if !pending {
        let mut actions = Vec::new();
        terminate_session(
            &mut next_state,
            TerminationOptions {
                reason: TerminalReason::Completed,
                origin: OriginOrDefault::Local,
                ..Default::default()
            },
            &mut actions,
        );
        return (next_state, applied(actions));
    }
    (next_state, applied(Vec::new()))
}

fn apply_abort(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let code = value_str(&message.body, "code");
    if state.role == Role::Host && state.freeze.is_none() {
        let reason = if code == "manifest_mismatch" {
            skew_reason(state, peer)
        } else {
            None
        };
        return drop_guest(
            state,
            peer,
            "protocol_error",
            &format!("a guest aborted with {code}"),
            reason,
        );
    }
    terminate_from(
        state,
        TerminationOptions {
            reason: TerminalReason::PeerAbort,
            code: Some(code),
            origin: OriginOrDefault::Remote,
            peer_id: Some(peer.peer_id.clone()),
            announce: true,
            exclude_link: peer.link_id.clone(),
            ..Default::default()
        },
    )
}

fn apply_disconnect(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    let body = &message.body;
    let target_peer_id = value_str(body, "target_peer_id");
    let code = value_str(body, "code");
    let reason = disconnect_reason(&code);
    if state.role == Role::Guest {
        if target_peer_id != state.peer_id && code != "host_left" {
            if !local_peer(state).ready {
                return (state.clone(), applied(Vec::new()));
            }
            let mut next_state = state.clone();
            clear_readiness(&mut next_state);
            return (next_state, applied(Vec::new()));
        }
        return terminate_from(
            state,
            TerminationOptions {
                reason: if code == "host_left" {
                    TerminalReason::HostLeft
                } else {
                    TerminalReason::Removed
                },
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                ..Default::default()
            },
        );
    }
    if target_peer_id != peer.peer_id {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("a guest cannot disconnect another peer".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    if state.freeze.is_some() {
        return terminate_from(
            state,
            TerminationOptions {
                reason,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                announce: true,
                exclude_link: peer.link_id.clone(),
                ..Default::default()
            },
        );
    }
    drop_guest(
        state,
        peer,
        &code,
        &format!("a guest announced its own disconnect as {code}"),
        None,
    )
}

fn apply_repeat_handshake(state: &CoordinatorState, peer: &Peer) -> (CoordinatorState, Outcome) {
    terminate_from(
        state,
        TerminationOptions {
            reason: TerminalReason::ProtocolViolation,
            origin: OriginOrDefault::Remote,
            peer_id: Some(peer.peer_id.clone()),
            detail: Some("an admitted link cannot handshake again".to_string()),
            announce: true,
            ..Default::default()
        },
    )
}

fn sender_role(kind: protocol::MessageKind) -> Option<Role> {
    use protocol::MessageKind::*;
    match kind {
        Handshake => Some(Role::Guest),
        ManifestProposal => Some(Role::Host),
        ManifestAccept => Some(Role::Guest),
        PeerAssignment => Some(Role::Host),
        SlotAssignment => Some(Role::Host),
        Ready => Some(Role::Guest),
        PairPreference => Some(Role::Guest),
        PairPreferenceResult => Some(Role::Host),
        Countdown => Some(Role::Host),
        MatchPhase => Some(Role::Host),
        // `start`, `hash_report`, and `result_ack` are deliberately absent:
        // the host publishes them and every peer echoes the identical body
        // back as its acknowledgement, so both directions are legal.
        Start | HashReport | ResultAck | Abort | Disconnect => None,
    }
}

fn classify_transcript(
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> protocol::Result<Option<&'static str>> {
    if message.sequence > peer.last_sequence {
        return Ok(Some("applied"));
    }
    for record in peer.window.iter().rev() {
        if record.sequence == message.sequence {
            let wire = protocol::encode(message)?;
            if wire == record.wire {
                return Ok(Some("idempotent"));
            }
            return Err(protocol::Error {
                code: protocol::ErrorCode::TranscriptConflict,
                message: "message id was reused with different canonical bytes".to_string(),
                path: None,
            });
        }
    }
    Ok(Some("stale"))
}

fn record_transcript(
    next_state: &mut CoordinatorState,
    peer_id: &str,
    message: &protocol::ControlMessage,
) {
    let wire = protocol::encode(message).expect("an already-validated message encodes");
    let Some(peer) = peer_by_id_mut(next_state, peer_id) else {
        return;
    };
    peer.last_sequence = message.sequence;
    peer.window.push(WireRecord {
        sequence: message.sequence,
        wire,
    });
    while peer.window.len() > DUPLICATE_WINDOW {
        peer.window.remove(0);
    }
}

fn dispatch_message(
    state: &CoordinatorState,
    peer: &Peer,
    message: &protocol::ControlMessage,
) -> (CoordinatorState, Outcome) {
    match message.kind {
        protocol::MessageKind::Handshake => apply_repeat_handshake(state, peer),
        protocol::MessageKind::ManifestProposal => apply_manifest_proposal(state, peer, message),
        protocol::MessageKind::ManifestAccept => apply_manifest_accept(state, peer, message),
        protocol::MessageKind::PeerAssignment => apply_peer_assignment(state, peer, message),
        protocol::MessageKind::SlotAssignment => apply_slot_assignment(state, peer, message),
        protocol::MessageKind::Ready => apply_ready(state, peer, message),
        protocol::MessageKind::PairPreference => apply_pair_preference(state, peer, message),
        protocol::MessageKind::PairPreferenceResult => {
            apply_pair_preference_result(state, peer, message)
        }
        protocol::MessageKind::Countdown => apply_countdown(state, peer, message),
        protocol::MessageKind::Start => apply_start(state, peer, message),
        protocol::MessageKind::MatchPhase => apply_match_phase(state, peer, message),
        protocol::MessageKind::HashReport => apply_hash_report(state, peer, message),
        protocol::MessageKind::ResultAck => apply_result_ack(state, peer, message),
        protocol::MessageKind::Abort => apply_abort(state, peer, message),
        protocol::MessageKind::Disconnect => apply_disconnect(state, peer, message),
    }
}

fn handle_control(
    state: &CoordinatorState,
    link_id: &str,
    message: Option<protocol::ControlMessage>,
    wire: Option<&str>,
) -> (CoordinatorState, Outcome) {
    let peer = peer_by_link(state, link_id).cloned();
    let message = match message {
        Some(message) => message,
        None => match wire {
            Some(wire) => match protocol::decode(wire) {
                Ok(message) => message,
                Err(err) => {
                    if let Some(peer) = &peer {
                        return terminate_from(
                            state,
                            TerminationOptions {
                                reason: TerminalReason::ProtocolViolation,
                                code: Some(
                                    if err.code == protocol::ErrorCode::UnsupportedVersion {
                                        "protocol_mismatch".to_string()
                                    } else {
                                        "malformed_message".to_string()
                                    },
                                ),
                                origin: OriginOrDefault::Remote,
                                peer_id: Some(peer.peer_id.clone()),
                                detail: Some(err.message),
                                announce: true,
                                ..Default::default()
                            },
                        );
                    }
                    return reject_link(state, link_id, "malformed_message", &err.message);
                }
            },
            None => {
                return (
                    state.clone(),
                    rejected(
                        RejectCode::Malformed,
                        "a control message needs a message or wire",
                    ),
                );
            }
        },
    };
    if let Err(err) = protocol::validate(&message) {
        if let Some(peer) = &peer {
            return terminate_from(
                state,
                TerminationOptions {
                    reason: TerminalReason::ProtocolViolation,
                    code: Some(if err.code == protocol::ErrorCode::UnsupportedVersion {
                        "protocol_mismatch".to_string()
                    } else {
                        "malformed_message".to_string()
                    }),
                    origin: OriginOrDefault::Remote,
                    peer_id: Some(peer.peer_id.clone()),
                    detail: Some(err.message),
                    announce: true,
                    ..Default::default()
                },
            );
        }
        return reject_link(state, link_id, "malformed_message", &err.message);
    }
    if message.session_id != state.session_id {
        if let Some(peer) = &peer {
            return terminate_from(
                state,
                TerminationOptions {
                    reason: TerminalReason::ProtocolViolation,
                    origin: OriginOrDefault::Remote,
                    peer_id: Some(peer.peer_id.clone()),
                    detail: Some("control message names a different session".to_string()),
                    announce: true,
                    ..Default::default()
                },
            );
        }
        return reject_link(state, link_id, "protocol_mismatch", "unknown session");
    }
    let Some(peer) = peer else {
        if message.kind != protocol::MessageKind::Handshake {
            return reject_link(
                state,
                link_id,
                "invalid_phase",
                "an unadmitted link may only handshake",
            );
        }
        return admit_guest(state, link_id, &message);
    };
    if message.peer_id != peer.peer_id {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some("control message claims another peer identity".to_string()),
                announce: true,
                ..Default::default()
            },
        );
    }
    if let Some(expected_sender) = sender_role(message.kind)
        && expected_sender != peer.role
    {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some(format!(
                    "{} may only be sent by the {}",
                    message.kind.wire_str(),
                    expected_sender.wire_str()
                )),
                announce: true,
                ..Default::default()
            },
        );
    }
    let disposition = match classify_transcript(&peer, &message) {
        Ok(disposition) => disposition,
        Err(err) => {
            return terminate_from(
                state,
                TerminationOptions {
                    reason: TerminalReason::ProtocolViolation,
                    code: Some("malformed_message".to_string()),
                    origin: OriginOrDefault::Remote,
                    peer_id: Some(peer.peer_id.clone()),
                    detail: Some(err.message),
                    announce: true,
                    ..Default::default()
                },
            );
        }
    };
    match disposition {
        Some("idempotent") => return (state.clone(), idempotent()),
        Some("stale") => return (state.clone(), stale(STALE_DUPLICATE_REASON)),
        _ => {}
    }
    // Republished ownership implicitly revokes readiness, so it is validated
    // against `assigned`: #161 forbids slot assignment during `ready`, and the
    // coordinator's answer to a configuration change is to leave `ready`.
    let phase = if message.kind == protocol::MessageKind::SlotAssignment
        && state.phase == protocol::LifecyclePhase::Ready
    {
        protocol::LifecyclePhase::Assigned
    } else {
        state.phase
    };
    if let Err(err) = protocol::validate_phase(&message, phase) {
        return terminate_from(
            state,
            TerminationOptions {
                reason: TerminalReason::ProtocolViolation,
                code: Some("invalid_phase".to_string()),
                origin: OriginOrDefault::Remote,
                peer_id: Some(peer.peer_id.clone()),
                detail: Some(err.message),
                announce: true,
                ..Default::default()
            },
        );
    }
    let (mut next_state, outcome) = dispatch_message(state, &peer, &message);
    if outcome.disposition == Disposition::Rejected {
        return (next_state, outcome);
    }
    if next_state == *state {
        next_state = state.clone();
    }
    record_transcript(&mut next_state, &peer.peer_id, &message);
    (next_state, outcome)
}

/// Advance a coordinator by one event.
///
/// # Panics
///
/// Panics if `state.phase` is not one of [`PHASES`] — a programmer error
/// under this crate's own invariants.
pub fn step(state: &CoordinatorState, event: Event) -> (CoordinatorState, Outcome) {
    assert!(
        PHASES.contains(&state.phase),
        "coordinator state is invalid"
    );
    if state.phase == protocol::LifecyclePhase::Terminal && !matches!(event, Event::Tick) {
        return (
            state.clone(),
            rejected(RejectCode::InvalidPhase, "the session already ended"),
        );
    }
    match event {
        Event::Connect => handle_connect(state),
        Event::Control {
            link_id,
            message,
            wire,
        } => handle_control(state, &link_id, message, wire.as_deref()),
        Event::LinkLost { link_id, code } => handle_link_lost(state, &link_id, code.as_deref()),
        Event::ProposeManifest { manifest } => handle_propose_manifest(state, &manifest),
        Event::AssignSlots {
            assignments,
            preserve_claims,
        } => handle_assign_slots(state, &assignments, preserve_claims),
        Event::PreferPair { slots } => handle_prefer_pair(state, &slots),
        Event::SetReady { ready } => handle_set_ready(state, ready),
        Event::BeginCountdown {
            countdown_id,
            remaining_ticks,
            first_input_tick,
        } => handle_begin_countdown(state, &countdown_id, remaining_ticks, first_input_tick),
        Event::Tick => handle_tick(state),
        Event::MatchPhase {
            phase,
            tick,
            home_score,
            away_score,
        } => handle_match_phase(state, &phase, tick, home_score, away_score),
        Event::HashReport {
            tick,
            boundary_hash,
        } => handle_hash_report(state, tick, &boundary_hash),
        Event::Finish {
            final_tick,
            home_score,
            away_score,
            final_hash,
        } => handle_finish(state, final_tick, home_score, away_score, &final_hash),
        Event::NetcodeFailure {
            failure,
            peer_id,
            detail,
        } => handle_netcode_failure(state, &failure, peer_id, detail),
        Event::Leave => handle_leave(state),
        Event::Abort { code, detail } => handle_abort(state, code, detail),
    }
}

/// Feed one inbound wire packet, as if freshly decoded from `link_id`.
pub fn receive(state: &CoordinatorState, link_id: &str, wire: &str) -> (CoordinatorState, Outcome) {
    step(
        state,
        Event::Control {
            link_id: link_id.to_string(),
            message: None,
            wire: Some(wire.to_string()),
        },
    )
}

// ---------------------------------------------------------------------------
// Small conversion shim from the wire's `SessionRejectCode` vocabulary
// (used when a link is refused before it names any `protocol::ErrorCode` of
// its own, e.g. `reject_link`) to this module's own `RejectCode`. The two
// vocabularies are different closed sets with no canonical 1:1 mapping in
// the Lua original either.
// ---------------------------------------------------------------------------

fn reject_code_from_session_reject(code: &str) -> RejectCode {
    match code {
        "protocol_mismatch" => RejectCode::UnsupportedVersion,
        "runtime_mismatch" => RejectCode::RuntimeMismatch,
        "manifest_mismatch" => RejectCode::IdentityMismatch,
        "capacity" => RejectCode::Capacity,
        "invalid_assignment" => RejectCode::InvalidAssignment,
        "invalid_phase" => RejectCode::InvalidPhase,
        "unsupported_message" => RejectCode::UnknownMessage,
        _ => RejectCode::Malformed,
    }
}
