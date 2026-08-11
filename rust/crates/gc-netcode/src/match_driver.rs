//! One [`MatchDriver`] drives one peer's match: it polls the star transport,
//! authors the rows it owns, sequences host authority, feeds real arrivals
//! into the OMP-2 rollback session ([`gc_sim::rollback_session`]), advances
//! the fixed 60 Hz clock, recomputes the live-slot timeline, publishes
//! boundary hashes, and ends with one typed terminal status
//! ([`MatchDriverStatus`]).
//!
//! # Clocks
//!
//! Three clocks are distinct: the **driver step** `T` (one per [`advance`]
//! call), the **transport tick** `T + DELAY_TICKS`, and the **input tick**
//! `first_input_tick + T`. A sample taken during driver step `T` is
//! authority for input tick `first_input_tick + T + DELAY_TICKS`.
//!
//! # Local placeholders and duplicated logic
//!
//! This file predates `crate::coordinator`, `crate::live_slot`, and
//! `crate::protocol`, and has not been updated to depend on them directly,
//! so several pieces of it still restate logic those modules now own
//! outright:
//!
//! 1. **Trivial 1:1 restatements.** [`slot_id_of`]/[`slot_index_of`]/
//!    [`switch_edge`] restate `crate::live_slot::slot_id`/`slot_index`/
//!    `switch_edge` exactly — no ranking, no carrier derivation, just the
//!    same lookup under a different name, the same duplication precedent
//!    `input_protocol.rs` sets for the rest of the wire's bound constants.
//!    [`owned_slots`] is likewise a pure filter over the assignment shape
//!    this file defines its own placeholder for.
//! 2. **Everything else is injected, not duplicated.** The nearest-to-ball
//!    ranking and carrier derivation (`crate::live_slot::carrier`/`rank`),
//!    the live slot transition rule (`crate::coordinator::next_live_slot`),
//!    the host's authority canonicalization
//!    (`input_protocol::canonical_host_batch`), and the coordinator's
//!    slot-driver view (`crate::coordinator::slot_drivers`) are genuine
//!    business logic this file must not reimplement. [`MatchDriverRules`]
//!    states their contract as a trait instead of a direct dependency, so
//!    every other function in this file — tick bookkeeping, batching,
//!    redundancy, checkpoint policy, the settle phase — stays testable
//!    against a fake without pulling in the coordinator's full session
//!    state machine. `crate::match_driver_fixture::DriverRules` supplies
//!    the real implementation, delegating to `crate::live_slot`,
//!    `crate::coordinator`, and `input_protocol`.
//!
//! [`CoordinatorFreeze`], [`SessionManifest`], and [`SlotAssignment`] are
//! likewise narrow local placeholders for shapes `crate::coordinator`/
//! `crate::protocol` define directly (`coordinator::Freeze` and a
//! `protocol::Value`-encoded manifest). TODO: whoever wires `MatchDriver` to
//! depend on those modules directly should replace these placeholders with
//! the real types instead of converting between them, the way
//! `crate::match_driver_fixture::to_driver_freeze`/`to_driver_manifest`
//! currently do.

use crate::fault_transport::{
    StarTransportAdapter, TransportChannel, TransportError, TransportErrorCode, TransportMessage,
    TransportMessageType, TransportPeerEventKind, TransportPeerMessage, TransportPeerState,
};
use crate::input_protocol;
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::input_frame::{self, EdgeAction, InputSample, SlotId};
use gc_sim::match_snapshot::{self, MatchSnapshot, MatchState};
use gc_sim::rollback_input_history::{self, RollbackAuthoritativeInput, RollbackInputSource};
use gc_sim::rollback_session::{
    self, RollbackSession, RollbackSessionBatchArrival, RollbackSessionErrorCode,
    RollbackSessionStatus,
};
use gc_sim::rollback_snapshot_history::{RollbackSnapshotLookup, RollbackSnapshotLookupStatus};
use gc_sim::slot_input::{self, MatchSlotSource, MatchSlotSourceKind, SlotInputProducerState};
use indexmap::{IndexMap, IndexSet};

// ---------------------------------------------------------------------------
// TypeScript-owned `transport_contract` constants this file needs (see
// module doc: the full contract has no Rust home; only the small bounds are
// duplicated, exactly as `input_protocol.rs` already does for the rest of
// it).
// ---------------------------------------------------------------------------

/// The transport's bounded per-channel queue limit.
const TRANSPORT_MAX_QUEUE_LIMIT: i64 = 256;
/// The host's fixed peer id on the transport link.
const HOST_PEER_ID: &str = "host";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Fixed tick delay between a local input sample and its earliest legal
/// send.
pub const DELAY_TICKS: i64 = input_protocol::FAIRNESS_DELAY_TICKS;
/// Default driver steps between published boundary hashes.
pub const DEFAULT_HASH_INTERVAL_TICKS: i64 = 30;
/// Consecutive disagreeing checkpoints before a `hash_mismatch` terminal.
pub const MAX_HASH_MISMATCHES: i64 = 3;
/// One transport poll's row budget.
pub const POLL_BATCH_LIMIT: i64 = TRANSPORT_MAX_QUEUE_LIMIT;
/// How many contiguous input ticks one targeted host repair covers.
pub const REPAIR_SPAN_TICKS: i64 = 4;
/// Settle-phase bound in further driver steps.
pub const SETTLE_TIMEOUT_TICKS: i64 = 60;
/// Settle-phase bound in monotonic seconds.
pub const SETTLE_TIMEOUT_SECONDS: f64 = 2.0;

// ---------------------------------------------------------------------------
// Terminal status
// ---------------------------------------------------------------------------

/// A [`MatchDriver`]'s lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchDriverStatus {
    /// Accepting further [`advance`] calls.
    Active,
    /// The final boundary was confirmed after full time.
    Completed,
    /// The settle phase expired before the final boundary confirmed.
    SettleTimeout,
    /// Confirmation stopped advancing and fell below the retained floor.
    ConfirmationStalled,
    /// Authority arrived (or a correction landed) outside the retained
    /// rollback window.
    LateInput,
    /// A peer's published checkpoint disagreed too many times.
    HashMismatch,
    /// A producer authored a slot outside its frozen owned set.
    OwnershipViolation,
    /// Canonical authority conflicted with retained history.
    AuthorityConflict,
    /// The input channel failed or exceeded its bounded queue.
    InputChannelFailure,
    /// The star transport (or a frozen peer link) was lost.
    TransportLost,
}

/// Which family of failure to report into the session coordinator, if any.
/// Three literals actually used at this file's `terminate` call sites:
/// `desync`, `input_channel`, `late_input`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinatorNetcodeFailure {
    /// Peers disagreed on a boundary they both hashed.
    Desync,
    /// The input channel itself failed, was violated, or was lost.
    InputChannel,
    /// Authority this peer can no longer place in its rollback window.
    LateInput,
}

/// A driver's terminal outcome, present exactly once [`status`] leaves
/// [`MatchDriverStatus::Active`].
#[derive(Clone, Debug, PartialEq)]
pub struct MatchDriverTerminal {
    /// The terminal status reached.
    pub status: MatchDriverStatus,
    /// What to report into the coordinator, if anything.
    pub failure: Option<CoordinatorNetcodeFailure>,
    /// Human-readable detail.
    pub detail: String,
    /// Input tick the failure was attributed to.
    pub tick: Option<i64>,
}

/// One confirmed boundary's published hash and live-slot map.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchDriverCheckpoint {
    /// Confirmed boundary tick, in input-tick space.
    pub tick: i64,
    /// The boundary's canonical hash.
    pub hash: String,
    /// Live slot per human at that boundary.
    pub live: IndexMap<String, SlotId>,
}

// ---------------------------------------------------------------------------
// Local placeholders for `crate::coordinator`/`crate::protocol` shapes (see
// module doc).
// ---------------------------------------------------------------------------

/// Which kind of producer authors one canonical slot. Mirrors the
/// `producer_kind` field of `crate::coordinator`'s assignment shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducerKind {
    /// A connected peer (human).
    Peer,
    /// A declared, host-authored bot fill.
    Bot,
}

/// One canonical slot's frozen producer assignment. A narrow placeholder for
/// `crate::coordinator`'s real assignment shape — see the module doc.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotAssignment {
    /// Which kind of producer this is.
    pub producer_kind: ProducerKind,
    /// The producer's stable identity (a peer id, or a declared bot id).
    pub producer_id: String,
    /// Canonical Park-Miller seed override; only meaningful for
    /// [`ProducerKind::Bot`].
    pub bot_seed: Option<i64>,
}

/// The frozen session state a [`MatchDriver`] is constructed from. A narrow
/// placeholder for `crate::coordinator::Freeze` — see the module doc. Every
/// field here is one this file actually reads.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinatorFreeze {
    /// Hash identifying the frozen session manifest.
    pub manifest_id: String,
    /// The frozen session's first authority input tick.
    pub first_input_tick: i64,
    /// The frozen match seed.
    pub seed: i64,
    /// Every canonical slot's frozen producer, in canonical order.
    pub assignments: [SlotAssignment; 8],
    /// Producer id -> every slot it owns, canonical order.
    pub owned: IndexMap<String, Vec<SlotId>>,
    /// Producer id -> its opening live slot.
    pub live: IndexMap<String, SlotId>,
}

/// The frozen session manifest. A narrow placeholder for the manifest
/// `crate::protocol` defines — see the module doc. This file reads only
/// `session_id` off it directly (everything else routes through
/// [`CoordinatorFreeze`], which already carries the frozen `manifest_id`).
#[derive(Clone, Debug, PartialEq)]
pub struct SessionManifest {
    /// The session's stable identity.
    pub session_id: String,
}

/// Which side of the session one driver is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverRole {
    /// The sequencing peer.
    Host,
    /// A non-sequencing peer.
    Guest,
}

/// A local restatement of `crate::protocol::owned_slots`, over this file's
/// own placeholder [`SlotAssignment`] shape rather than `crate::protocol`'s
/// own: every slot `peer_id` owns, in canonical order. A pure filter over
/// data this file already defines the shape of (see module doc point 1);
/// not `crate::protocol`'s canonical identity/hashing logic.
#[must_use]
pub fn owned_slots(assignments: &[SlotAssignment; 8], peer_id: &str) -> Vec<SlotId> {
    let mut result = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = &assignments[(index - 1) as usize];
        if producer.producer_kind == ProducerKind::Peer && producer.producer_id == peer_id {
            result.push(slot_id_of(index));
        }
    }
    result
}

/// `crate::live_slot::slot_id(index)`: the canonical slot identity at
/// 1-based `index`. Mechanical restatement of [`gc_sim::input_frame::slot`]
/// (see module doc point 1).
///
/// # Panics
///
/// Panics if `index` is outside `1..=8`.
#[must_use]
pub fn slot_id_of(index: i64) -> SlotId {
    input_frame::slot(index)
        .expect("canonical slot index is out of range")
        .id
}

/// `crate::live_slot::slot_index(slot)`: the 1-based canonical index for
/// `slot`. Mechanical restatement of [`gc_sim::input_frame::slot`]'s own
/// table (see module doc point 1).
#[must_use]
pub fn slot_index_of(slot: SlotId) -> i64 {
    match slot {
        SlotId::Home1 => 1,
        SlotId::Home2 => 2,
        SlotId::Home3 => 3,
        SlotId::Home4 => 4,
        SlotId::Away1 => 5,
        SlotId::Away2 => 6,
        SlotId::Away3 => 7,
        SlotId::Away4 => 8,
    }
}

/// `crate::live_slot::switch_edge(sample)`: whether the `switch` edge fired.
/// Mechanical restatement of [`gc_sim::input_frame::has_edge`] (see module
/// doc point 1).
///
/// # Panics
///
/// Panics if `sample` is not a valid quantized sample.
#[must_use]
pub fn switch_edge(sample: &InputSample) -> bool {
    input_frame::has_edge(sample, EdgeAction::Switch)
        .expect("live-slot switch edge requires a valid sample")
}

// ---------------------------------------------------------------------------
// The injected environment: `crate::live_slot`'s ranking/carrier logic,
// `crate::coordinator`'s transition rule and slot-driver view, and
// `input_protocol::canonical_host_batch` (see module doc point 2).
// ---------------------------------------------------------------------------

/// One ranked candidate for a live-slot handoff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveSlotRankEntry {
    /// The candidate slot.
    pub slot: SlotId,
    /// Canonical slot index, the total-order tiebreak.
    pub index: i64,
    /// Squared distance to the ball.
    pub distance_sq: f64,
}

/// Inputs to [`MatchDriverRules::transition`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveSlotTransitionOptions {
    /// The canonical `switch` edge of the live slot's row.
    pub switch: bool,
    /// Excluded from `ranked`, matching offline switching.
    pub live: Option<SlotId>,
    /// Carrier at the preceding boundary.
    pub previous_carrier: Option<SlotId>,
}

/// The outcome of one live-slot transition, mirroring the shape
/// `crate::live_slot::transition` returns.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveSlotTransition {
    /// The canonical `switch` edge, restated.
    pub switch: bool,
    /// The ball carrier at this boundary, if any.
    pub carrier: Option<SlotId>,
    /// The carrier, but only when it just changed.
    pub winner: Option<SlotId>,
    /// The full nearest-to-ball ranking, canonical order.
    pub ranked: Vec<LiveSlotRankEntry>,
}

/// Opaque placeholder for `crate::coordinator::SlotDriver`. This file does
/// not depend on `crate::coordinator` directly (see module doc); replace
/// with the real type if `MatchDriver` is ever wired to depend on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinatorSlotDriver {
    /// The producer id driving this slot right now, if any is known.
    pub producer_id: Option<String>,
}

/// Machine-readable reason [`MatchDriverRules::canonical_host_batch`]
/// refused a batch. Mirrors the `code`
/// `crate::match_driver_fixture::DriverRules::canonical_host_batch` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostBatchErrorCode {
    /// A row named a slot its sender does not own.
    OwnershipMismatch,
    /// Two arrivals disagreed on one `(tick, slot)`'s authority.
    AuthorityConflict,
    /// The same sender/sequence identity was reused with different bytes.
    PacketConflict,
    /// Any other canonicalization failure.
    Other,
}

/// An expected, recoverable [`MatchDriverRules::canonical_host_batch`]
/// failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostBatchError {
    /// Machine-readable failure reason.
    pub code: HostBatchErrorCode,
    /// Human-readable detail.
    pub message: String,
}

/// One [`MatchDriverRules::canonical_host_batch`] request.
pub struct HostBatchRequest<'a> {
    /// The frozen session manifest.
    pub manifest: &'a SessionManifest,
    /// Every canonical slot's frozen producer assignment.
    pub assignments: &'a [SlotAssignment; 8],
    /// This host's own peer id.
    pub host_peer_id: &'a str,
    /// This batch's monotonic sequence number.
    pub sequence: i64,
    /// The transport tick this batch is sent on.
    pub transport_tick: i64,
    /// The frozen session's first authority input tick.
    pub first_input_tick: i64,
    /// Contiguous confirmed input ticks from `first_input_tick`.
    pub confirmed_span: i64,
    /// A targeted repair window, if [`plan_repair`] found one due.
    pub repair_rows: Option<&'a [input_protocol::AuthorityRow]>,
    /// The selected, budget-bounded arrivals this batch canonicalizes.
    pub arrivals: &'a [InputPacketArrival],
}

/// The narrow slice of `crate::live_slot`, `crate::coordinator`, and
/// `crate::match_driver_fixture::DriverRules::canonical_host_batch` that
/// [`MatchDriver`] needs without depending on those modules directly — see
/// the module doc for why. `crate::match_driver_fixture::DriverRules`
/// supplies the real implementation; a test supplies a fake.
///
/// **Do not add gameplay/ranking/hashing logic to an implementation of this
/// trait inside this crate.** That is exactly the work `crate::live_slot`,
/// `crate::coordinator`, and `crate::protocol` own; an implementation here
/// should only ever delegate to them.
pub trait MatchDriverRules {
    /// Mirrors `crate::live_slot::carrier`.
    fn carrier(&self, state: &MatchState) -> Option<SlotId>;
    /// Mirrors `crate::live_slot::transition`.
    fn transition(
        &self,
        state: &MatchState,
        options: LiveSlotTransitionOptions,
    ) -> LiveSlotTransition;
    /// Mirrors `crate::coordinator::next_live_slot`.
    fn next_live_slot(
        &self,
        owned: &[SlotId],
        current: SlotId,
        transition: &LiveSlotTransition,
    ) -> SlotId;
    /// Mirrors `crate::coordinator::slot_drivers(freeze, live)`.
    fn slot_drivers(
        &self,
        freeze: &CoordinatorFreeze,
        live: &IndexMap<String, SlotId>,
    ) -> Vec<(SlotId, CoordinatorSlotDriver)>;
    /// Mirrors `crate::match_driver_fixture::DriverRules::canonical_host_batch(options, arrivals)`.
    ///
    /// # Errors
    ///
    /// Returns a [`HostBatchError`] that the caller must handle, not treat
    /// as a panic.
    fn canonical_host_batch(
        &self,
        request: HostBatchRequest<'_>,
    ) -> std::result::Result<input_protocol::Packet, HostBatchError>;
}

// ---------------------------------------------------------------------------
// Driver shapes
// ---------------------------------------------------------------------------

/// Construction options for [`new`].
pub struct MatchDriverOptions {
    /// Which side of the session this driver runs.
    pub role: DriverRole,
    /// This peer's own id.
    pub peer_id: String,
    /// The frozen session state.
    pub freeze: CoordinatorFreeze,
    /// The frozen session manifest.
    pub manifest: SessionManifest,
    /// The star transport this driver polls and sends on.
    pub transport: Box<dyn StarTransportAdapter>,
    /// Canonical slot-mode boundary zero.
    pub initial_snapshot: MatchSnapshot,
    /// Overrides the retained rollback window; bounded by
    /// [`rollback_input_history::ROLLBACK_WINDOW_TICKS`].
    pub max_rollback_ticks: Option<i64>,
    /// Overrides [`DEFAULT_HASH_INTERVAL_TICKS`].
    pub hash_interval_ticks: Option<i64>,
    /// Overrides [`SETTLE_TIMEOUT_TICKS`].
    pub settle_timeout_ticks: Option<i64>,
    /// Overrides [`SETTLE_TIMEOUT_SECONDS`].
    pub settle_timeout_seconds: Option<f64>,
    /// Overrides the monotonic seconds source; injected so the settle bound
    /// is testable. Defaults to [`default_clock`].
    pub clock: Option<Box<dyn FnMut() -> f64>>,
    /// The injected [`MatchDriverRules`] environment (see module doc).
    pub rules: Box<dyn MatchDriverRules>,
}

/// One [`advance`] call's outputs.
pub struct MatchDriverBatch {
    /// Driver step that produced this batch.
    pub step: i64,
    /// Input tick simulated during this step.
    pub input_tick: i64,
    /// Every tick this step actually simulated (prediction or resimulation).
    pub outputs: Vec<rollback_session::RollbackTickOutput>,
    /// Always at most one per driver step.
    pub reconciliations: i64,
    /// Authoritative rows newly retained this step.
    pub applied_rows: i64,
    /// Newly inserted rows that corrected an already-consumed prediction.
    pub corrections: i64,
    /// Reconciliations that actually rewound and resimulated.
    pub rollbacks: i64,
    /// Envelopes sent this step.
    pub sent_packets: i64,
    /// Checkpoints published this step.
    pub checkpoints: Vec<MatchDriverCheckpoint>,
    /// Control-channel traffic for the session coordinator to dispatch.
    pub control: Vec<TransportPeerMessage>,
    /// Live slot per human after this step.
    pub live: IndexMap<String, SlotId>,
    /// Status after this step.
    pub status: MatchDriverStatus,
}

/// A pending, delayed host-local collector entry.
#[derive(Clone)]
pub struct MatchDriverPending {
    /// Driver step on which the collector may read this packet.
    pub due: i64,
    /// The packet itself.
    pub packet: input_protocol::Packet,
    /// Its wire envelope.
    pub envelope: TransportMessage,
    /// The producer id it was authored for.
    pub producer_id: String,
}

/// One arrived input packet, still to be validated/applied.
#[derive(Clone)]
pub struct InputPacketArrival {
    /// The decoded packet.
    pub packet: input_protocol::Packet,
    /// Its wire envelope.
    pub envelope: TransportMessage,
    /// Transport tick this arrival was observed on.
    pub arrival_tick: i64,
    /// The transport peer id it arrived from (the host's own id for its
    /// delayed local collector path).
    pub transport_peer_id: String,
}

/// A full read of [`MatchDriver`]'s observable state.
pub struct MatchDriverDiagnostics {
    /// Present-boundary captures so far. See [`MatchDriver::snapshot_captures`].
    pub snapshot_captures: i64,
    /// Which side of the session this driver runs.
    pub role: DriverRole,
    /// This peer's own id.
    pub peer_id: String,
    /// Current lifecycle status.
    pub status: MatchDriverStatus,
    /// The terminal outcome, once reached.
    pub terminal: Option<MatchDriverTerminal>,
    /// Driver steps taken so far.
    pub step: i64,
    /// Current transport tick.
    pub transport_tick: i64,
    /// Present boundary, in input-tick space.
    pub present_input_tick: i64,
    /// Raw all-input authority confirmation.
    pub confirmed_input_tick: i64,
    /// Confirmation capped to simulated outputs.
    pub confirmed_output_tick: i64,
    /// Oldest input tick the rollback window still holds.
    pub retained_floor_tick: i64,
    /// Every slot this peer owns.
    pub owned: Vec<SlotId>,
    /// Every slot this peer authors (owned plus any declared bot fills).
    pub authored: Vec<SlotId>,
    /// Live slot per human, at the current live tick.
    pub live: IndexMap<String, SlotId>,
    /// This peer's own control slot, if it owns any slot.
    pub control_slot: Option<SlotId>,
    /// Total completed reconciliations that changed state.
    pub rollback_count: i64,
    /// Total corrections observed.
    pub correction_count: i64,
    /// Cumulative predicted slot executions.
    pub predicted_slot_samples: i64,
    /// The largest rollback depth seen so far.
    pub max_rollback_depth: i64,
    /// The input tick a `late_input` terminal was attributed to, if any.
    pub late_input_tick: Option<i64>,
    /// Consecutive disagreeing checkpoints observed.
    pub hash_mismatches: i64,
    /// Checkpoints published so far.
    pub checkpoint_count: i64,
    /// Full time reached, final boundary not confirmed yet.
    pub settling: bool,
    /// The final boundary was confirmed before the driver completed.
    pub settled: bool,
    /// Final boundary, in input-tick space, once full time is reached.
    pub full_time_boundary: Option<i64>,
    /// Settle steps taken so far.
    pub settle_steps: i64,
    /// Outbound envelopes the transport dropped.
    pub dropped_outbound: i64,
    /// Inbound envelopes the transport dropped.
    pub dropped_inbound: i64,
}

/// One peer's OMP-3 online match driver. See the module doc.
pub struct MatchDriver {
    /// How many times this driver has captured the present boundary.
    ///
    /// A diagnostic, not simulation state — it never affects a hash. It exists
    /// because "authoring only your own control slot costs no extra snapshot
    /// work" is a real invariant with no other observable: the original test
    /// suite asserted it by replacing `rollback_session.current_snapshot` at
    /// runtime, a technique Rust cannot do without putting a trait object in
    /// this module's per-tick path. A counter is the smaller change, and
    /// unlike a test mock it is also readable on a live session.
    ///
    /// `Cell` because the capture happens behind `&MatchDriver`; making it `&mut`
    /// would cascade through call sites that have no other reason to need one.
    pub snapshot_captures: std::cell::Cell<i64>,
    /// Which side of the session this driver runs.
    pub role: DriverRole,
    /// This peer's own id.
    pub peer_id: String,
    /// The frozen session state.
    pub freeze: CoordinatorFreeze,
    /// The frozen session manifest.
    pub manifest: SessionManifest,
    /// The frozen `manifest_id`, cached from `freeze` (see [`SessionManifest`]'s doc).
    pub manifest_id: String,
    /// The star transport this driver polls and sends on.
    pub transport: Box<dyn StarTransportAdapter>,
    /// The OMP-2 rollback session.
    pub session: RollbackSession,
    /// Local/remote ownership per canonical slot.
    pub sources: [RollbackInputSource; 8],
    /// The deterministic slot-input producer.
    pub producer: SlotInputProducerState,
    /// Every slot this peer owns, canonical order.
    pub owned: Vec<SlotId>,
    /// Canonical slot indexes this peer authors, ascending.
    pub authored: Vec<i64>,
    /// `authored`, as a fixed-size membership set (index `slot - 1`).
    pub authored_set: [bool; 8],
    /// `owned`, as a fixed-size membership set (index `slot - 1`).
    pub owned_set: [bool; 8],
    /// Peer producer id -> owned canonical slot indexes.
    pub peer_slots: IndexMap<String, Vec<i64>>,
    /// Human producer ids, in frozen assignment order.
    pub humans: Vec<String>,
    /// Host-only: declared bot fills, ascending.
    pub bot_slots: Vec<i64>,
    /// The frozen session's first authority input tick.
    pub first: i64,
    /// Driver steps taken so far.
    pub step: i64,
    /// Input tick -> live slot per human.
    pub live: IndexMap<i64, IndexMap<String, SlotId>>,
    /// Input tick -> ball carrier at that boundary.
    pub carrier: IndexMap<i64, Option<SlotId>>,
    /// Highest input tick with a computed live map.
    pub live_tick: i64,
    /// Canonical slot index (0-based array position) -> tick -> authored
    /// sample.
    pub history: [IndexMap<i64, InputSample>; 8],
    /// Sender id -> next monotonic packet sequence.
    pub sequences: IndexMap<String, i64>,
    /// Host-only: the delayed local collector path.
    pub pending: Vec<MatchDriverPending>,
    /// Host-only: arrivals held back by the row bound.
    pub deferred: Vec<InputPacketArrival>,
    /// Host-only: newest accepted bundle per canonical slot.
    pub relay: IndexMap<i64, InputPacketArrival>,
    /// Host-only: canonical slot -> that author's newest reported confirmed
    /// input tick.
    pub peer_confirmed: IndexMap<i64, i64>,
    /// Guest-only: the host's newest reported confirmed input tick.
    pub host_confirmed: Option<i64>,
    /// Every published checkpoint so far.
    pub checkpoints: Vec<MatchDriverCheckpoint>,
    /// Boundary tick -> this peer's own hash for it.
    pub checkpoint_by_tick: IndexMap<i64, String>,
    /// Driver steps between published boundary hashes.
    pub hash_interval: i64,
    /// The next boundary due to be hashed.
    pub next_checkpoint: i64,
    /// Consecutive disagreeing checkpoints observed.
    pub hash_mismatches: i64,
    /// Current lifecycle status.
    pub status: MatchDriverStatus,
    /// The terminal outcome, once reached.
    pub terminal: Option<MatchDriverTerminal>,
    /// The input tick a `late_input` terminal was attributed to, if any.
    pub late_input_tick: Option<i64>,
    /// Highest input tick this peer has authored rows for.
    pub authored_tick: i64,
    /// Settle-phase bound in further driver steps.
    pub settle_ticks: i64,
    /// Settle-phase bound in monotonic seconds.
    pub settle_seconds: f64,
    /// Monotonic seconds source.
    pub clock: Box<dyn FnMut() -> f64>,
    /// Final boundary, in input-tick space, once full time is reached.
    pub full_time_boundary: Option<i64>,
    /// Settle steps taken so far, capped by `settle_ticks`.
    pub settle_steps: i64,
    /// The monotonic second the settle phase expires at.
    pub settle_deadline: Option<f64>,
    /// The final boundary was confirmed before the driver completed.
    pub settled: bool,
    /// The injected [`MatchDriverRules`] environment.
    pub rules: Box<dyn MatchDriverRules>,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Monotonic seconds for the settle phase's wall-clock bound: wall-clock
/// seconds since the Unix epoch. Nothing simulated reads this: it can only
/// end a settle phase early, never change a tick.
#[must_use]
pub fn default_clock() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `derived_ai_seed(seed, slot_index)`: a non-live owned slot's bot stream
/// seed, derived from the frozen match seed so every peer can name it
/// without negotiation. Park-Miller states are `1..=2^31-2`.
///
/// Both operands here are always non-negative (`seed` is
/// a frozen non-negative match seed and `slot_index * 7919` is non-negative
/// for every canonical `slot_index`), so no `rem_euclid` is needed.
#[must_use]
pub fn derived_ai_seed(seed: i64, slot_index: i64) -> i64 {
    let mixed = (seed % 2_147_483_629) * 31 + slot_index * 7919;
    1 + (mixed % 2_147_483_645)
}

// ---------------------------------------------------------------------------
// Terminal status
// ---------------------------------------------------------------------------

fn terminate(
    driver: &mut MatchDriver,
    status: MatchDriverStatus,
    detail: String,
    failure: Option<CoordinatorNetcodeFailure>,
    tick: Option<i64>,
) {
    if driver.status != MatchDriverStatus::Active {
        return;
    }
    driver.status = status;
    driver.terminal = Some(MatchDriverTerminal {
        status,
        failure,
        detail,
        tick,
    });
}

// ---------------------------------------------------------------------------
// Tick spaces
// ---------------------------------------------------------------------------

fn session_tick(driver: &MatchDriver, input_tick: i64) -> i64 {
    input_tick - driver.first
}

fn to_input_tick(driver: &MatchDriver, tick: i64) -> i64 {
    tick + driver.first
}

fn present_input_tick(driver: &MatchDriver) -> i64 {
    to_input_tick(
        driver,
        rollback_session::diagnostics(&driver.session).present_boundary,
    )
}

fn confirmed_input_tick(driver: &MatchDriver) -> i64 {
    let confirmed = rollback_session::diagnostics(&driver.session).confirmed_tick;
    if confirmed < 0 {
        driver.first - 1
    } else {
        to_input_tick(driver, confirmed)
    }
}

fn confirmed_output_input_tick(driver: &MatchDriver) -> i64 {
    let confirmed = rollback_session::diagnostics(&driver.session).confirmed_output_tick;
    if confirmed < 0 {
        driver.first - 1
    } else {
        to_input_tick(driver, confirmed)
    }
}

fn retained_floor_input_tick(driver: &MatchDriver) -> i64 {
    to_input_tick(
        driver,
        rollback_session::diagnostics(&driver.session)
            .input_history
            .oldest_retained_tick,
    )
}

// ---------------------------------------------------------------------------
// Live-slot timeline
// ---------------------------------------------------------------------------

fn copy_live_map(
    driver: &MatchDriver,
    live: &IndexMap<String, SlotId>,
) -> IndexMap<String, SlotId> {
    let mut copied = IndexMap::new();
    for producer_id in &driver.humans {
        if let Some(&slot) = live.get(producer_id) {
            copied.insert(producer_id.clone(), slot);
        }
    }
    copied
}

fn boundary_state(driver: &MatchDriver, boundary: i64) -> MatchState {
    let lookup: RollbackSnapshotLookup =
        rollback_session::snapshot(&driver.session, session_tick(driver, boundary));
    assert!(
        matches!(
            lookup.status,
            RollbackSnapshotLookupStatus::Present | RollbackSnapshotLookupStatus::Retained
        ),
        "the live-slot timeline needs a retained boundary snapshot"
    );
    let snapshot = lookup
        .snapshot
        .expect("retained boundary snapshot is missing");
    match_snapshot::restore(&snapshot).0
}

fn present_state(driver: &MatchDriver) -> (MatchState, Option<CombatMatchState>) {
    driver
        .snapshot_captures
        .set(driver.snapshot_captures.get() + 1);
    match_snapshot::restore(&rollback_session::current_snapshot(&driver.session))
}

fn extend_live(
    driver: &mut MatchDriver,
    tick: i64,
    record: &rollback_input_history::RollbackInputTickRecord,
) {
    let previous = driver
        .live
        .get(&tick)
        .cloned()
        .expect("the live-slot timeline has a gap");
    let after = boundary_state(driver, tick + 1);
    let carrier = driver.rules.carrier(&after);
    let previous_carrier = driver.carrier.get(&tick).copied().flatten();
    let mut next_live = IndexMap::new();
    let humans = driver.humans.clone();
    for producer_id in humans {
        let current = *previous
            .get(&producer_id)
            .expect("a human has no live slot");
        let control = control_slot(driver, &producer_id, tick);
        let slot_index = slot_index_of(control);
        let sample = record.slots[(slot_index - 1) as usize].sample;
        let transition = driver.rules.transition(
            &after,
            LiveSlotTransitionOptions {
                switch: switch_edge(&sample),
                live: Some(current),
                previous_carrier,
            },
        );
        let owned = driver
            .freeze
            .owned
            .get(&producer_id)
            .cloned()
            .unwrap_or_default();
        let next = driver.rules.next_live_slot(&owned, current, &transition);
        next_live.insert(producer_id, next);
    }
    driver.live.insert(tick + 1, next_live);
    driver.carrier.insert(tick + 1, carrier);
    if tick + 1 > driver.live_tick {
        driver.live_tick = tick + 1;
    }
}

fn prune_live(driver: &mut MatchDriver, floor: i64) {
    let oldest = (floor - DELAY_TICKS).max(driver.first);
    let mut tick = driver.first;
    while tick < oldest {
        driver.live.shift_remove(&tick);
        driver.carrier.shift_remove(&tick);
        tick += 1;
    }
}

/// The slot that carries a human's authored bits at `input_tick`: the live
/// slot from [`DELAY_TICKS`] earlier.
///
/// # Panics
///
/// Panics if the live-slot timeline has no entry for the source tick, or the
/// producer owns no live slot there.
#[must_use]
pub fn control_slot(driver: &MatchDriver, producer_id: &str, input_tick: i64) -> SlotId {
    let source = (input_tick - DELAY_TICKS).max(driver.first);
    let live = driver
        .live
        .get(&source)
        .expect("the live-slot timeline has no entry for this tick");
    *live
        .get(producer_id)
        .expect("the producer owns no live slot")
}

/// Live slot per human at `input_tick` (the current live tick when `None`).
///
/// # Panics
///
/// Panics if the live-slot timeline has no entry for `input_tick`.
#[must_use]
pub fn live(driver: &MatchDriver, input_tick: Option<i64>) -> IndexMap<String, SlotId> {
    let tick = input_tick.unwrap_or(driver.live_tick);
    let entry = driver
        .live
        .get(&tick)
        .expect("the live-slot timeline has no entry for this tick");
    copy_live_map(driver, entry)
}

// ---------------------------------------------------------------------------
// Authoring
// ---------------------------------------------------------------------------

fn next_sequence(driver: &mut MatchDriver, sender_id: &str) -> i64 {
    let sequence = *driver.sequences.get(sender_id).unwrap_or(&0);
    driver.sequences.insert(sender_id.to_string(), sequence + 1);
    sequence
}

fn record_authored(driver: &mut MatchDriver, slot_index: i64, tick: i64, sample: InputSample) {
    let idx = (slot_index - 1) as usize;
    driver.history[idx].insert(tick, sample);
    let oldest = tick - rollback_input_history::ROLLBACK_WINDOW_TICKS - 1;
    if oldest >= driver.first {
        driver.history[idx].shift_remove(&oldest);
    }
}

fn redundant_rows(
    driver: &MatchDriver,
    slot_index: i64,
    tick: i64,
) -> Vec<input_protocol::AuthorityRow> {
    let slot_history = &driver.history[(slot_index - 1) as usize];
    let first = (tick - input_protocol::HISTORY_ROWS).max(driver.first);
    let mut rows = Vec::new();
    let mut row_tick = first;
    while row_tick <= tick {
        let sample = *slot_history
            .get(&row_tick)
            .expect("authored redundancy row is missing");
        rows.push(input_protocol::AuthorityRow {
            tick: row_tick,
            slot_index,
            sample,
        });
        row_tick += 1;
    }
    rows
}

fn materialize_authored(
    driver: &mut MatchDriver,
    input_tick: i64,
    human_sample: Option<InputSample>,
    geometry: Option<(MatchState, Option<CombatMatchState>)>,
) -> IndexMap<i64, InputSample> {
    let control = if !driver.owned.is_empty() {
        Some(slot_index_of(control_slot(
            driver,
            &driver.peer_id.clone(),
            input_tick,
        )))
    } else {
        None
    };
    let human = human_sample.unwrap_or_else(input_frame::neutral_sample);
    if driver.authored.len() == 1 && Some(driver.authored[0]) == control {
        let mut out = IndexMap::new();
        out.insert(
            control.expect("a single authored slot is always the control slot"),
            human,
        );
        return out;
    }
    let (state, combat) = geometry.unwrap_or_else(|| present_state(driver));
    let slots = [input_frame::neutral_sample(); 8];
    let base = input_frame::new(state.input_tick, Some(slots))
        .expect("a neutral base frame is always valid");
    let (frame, _decisions) =
        slot_input::materialize(&mut driver.producer, &state, &base, combat.as_ref());
    let mut authored = IndexMap::new();
    for &slot_index in &driver.authored {
        if Some(slot_index) == control {
            authored.insert(slot_index, human);
        } else {
            authored.insert(slot_index, frame.slots[(slot_index - 1) as usize]);
        }
    }
    authored
}

fn producer_id_for(driver: &MatchDriver, slot_index: i64) -> String {
    driver.freeze.assignments[(slot_index - 1) as usize]
        .producer_id
        .clone()
}

fn build_packet(
    driver: &mut MatchDriver,
    slot_index: i64,
    tick: i64,
    transport_tick: i64,
) -> (input_protocol::Packet, TransportMessage) {
    let sender_id = producer_id_for(driver, slot_index);
    let sequence = next_sequence(driver, &sender_id);
    let confirmed_span = input_protocol::confirmed_span(driver.first, confirmed_input_tick(driver));
    let packet = input_protocol::new_guest(input_protocol::PacketOptions {
        session_id: driver.manifest.session_id.clone(),
        manifest_id: driver.manifest_id.clone(),
        sender_id,
        sequence,
        transport_tick,
        first_input_tick: driver.first,
        confirmed_span: Some(confirmed_span),
        rows: redundant_rows(driver, slot_index, tick),
    })
    .expect("a driver-authored packet is always well-formed");
    let wire = input_protocol::encode(&packet).expect("a driver-authored packet always encodes");
    debug_assert!(
        wire.len() <= 1024,
        "transport payload exceeds the byte budget"
    );
    let envelope = TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq: packet.sequence,
        tick: Some(packet.transport_tick),
        payload: wire,
    };
    (packet, envelope)
}

// ---------------------------------------------------------------------------
// Applying authority
// ---------------------------------------------------------------------------

enum ApplyOutcome {
    Applied {
        inserted: i64,
    },
    Reconciled {
        // Boxed: `RollbackReconcileResult` is far larger than the other
        // variants' payloads, and clippy's `large_enum_variant` flags the
        // resulting size skew.
        arrival: Box<RollbackSessionBatchArrival>,
        reconciliation: Box<rollback_session::RollbackReconcileResult>,
    },
    Failed {
        code: rollback_input_history::RollbackInputHistoryErrorCode,
        message: String,
    },
}

fn apply_rows(
    driver: &mut MatchDriver,
    rows: &[input_protocol::AuthorityRow],
    batch: &mut MatchDriverBatch,
    arrival: bool,
) {
    if rows.is_empty() {
        return;
    }
    let confirmed = confirmed_input_tick(driver);
    let mut arrivals: Vec<RollbackAuthoritativeInput> = Vec::new();
    for row in rows {
        if row.tick > confirmed {
            arrivals.push(RollbackAuthoritativeInput {
                tick: session_tick(driver, row.tick),
                slot_index: row.slot_index,
                sample: row.sample,
            });
        }
    }
    if arrivals.is_empty() {
        return;
    }

    let outcome = if !arrival {
        match rollback_session::add_authoritative_batch(&mut driver.session, &arrivals) {
            Ok(accepted) => {
                if accepted.earliest_divergence.is_none() {
                    ApplyOutcome::Applied {
                        inserted: accepted.inserted,
                    }
                } else {
                    let reconciliation = rollback_session::reconcile(&mut driver.session, false);
                    ApplyOutcome::Reconciled {
                        arrival: Box::new(accepted),
                        reconciliation: Box::new(reconciliation),
                    }
                }
            }
            Err(err) => ApplyOutcome::Failed {
                code: err.code,
                message: err.message,
            },
        }
    } else {
        match rollback_session::apply_authoritative_batch(&mut driver.session, &arrivals, false) {
            Ok(result) => ApplyOutcome::Reconciled {
                arrival: Box::new(result.arrival),
                reconciliation: Box::new(result.reconciliation),
            },
            Err(err) => ApplyOutcome::Failed {
                code: err.code,
                message: err.message,
            },
        }
    };

    match outcome {
        ApplyOutcome::Applied { inserted } => {
            batch.applied_rows += inserted;
        }
        ApplyOutcome::Failed { code, message } => {
            let first_tick = to_input_tick(driver, arrivals[0].tick);
            match code {
                rollback_input_history::RollbackInputHistoryErrorCode::OutsideWindow => {
                    let tick = driver.late_input_tick.unwrap_or(first_tick);
                    driver.late_input_tick = Some(tick);
                    terminate(
                        driver,
                        MatchDriverStatus::LateInput,
                        message,
                        Some(CoordinatorNetcodeFailure::LateInput),
                        Some(tick),
                    );
                }
                rollback_input_history::RollbackInputHistoryErrorCode::ConflictingAuthoritative => {
                    terminate(
                        driver,
                        MatchDriverStatus::AuthorityConflict,
                        message,
                        Some(CoordinatorNetcodeFailure::InputChannel),
                        None,
                    );
                }
                _ => {
                    terminate(
                        driver,
                        MatchDriverStatus::InputChannelFailure,
                        message,
                        Some(CoordinatorNetcodeFailure::InputChannel),
                        None,
                    );
                }
            }
        }
        ApplyOutcome::Reconciled {
            arrival: accepted,
            reconciliation,
        } => {
            if arrival {
                batch.reconciliations += 1;
            }
            batch.applied_rows += accepted.inserted;
            batch.corrections += accepted.corrections;
            if reconciliation.status == RollbackSessionStatus::LateInputUnrecoverable {
                let tick = driver
                    .late_input_tick
                    .or_else(|| reconciliation.causal_tick.map(|t| to_input_tick(driver, t)));
                driver.late_input_tick = tick;
                terminate(
                    driver,
                    MatchDriverStatus::LateInput,
                    "the rollback window overflowed while reconciling".to_string(),
                    Some(CoordinatorNetcodeFailure::LateInput),
                    tick,
                );
                return;
            }
            if !reconciliation.changed {
                return;
            }
            batch.rollbacks += 1;
            for output in &reconciliation.corrected_outputs {
                let tick = to_input_tick(driver, output.tick);
                extend_live(driver, tick, &output.input);
                batch.outputs.push(output.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

fn drain_events(driver: &mut MatchDriver) {
    while driver.status == MatchDriverStatus::Active {
        let Some(event) = driver.transport.poll_event() else {
            return;
        };
        match event.kind {
            TransportPeerEventKind::StarError | TransportPeerEventKind::PeerError => {
                let overflow = matches!(
                    event.code,
                    Some(TransportErrorCode::Overflow) | Some(TransportErrorCode::Backpressure)
                );
                let detail = event.message.unwrap_or_else(|| {
                    if overflow {
                        "the input channel exceeded its queue budget".to_string()
                    } else {
                        "the transport reported a terminal error".to_string()
                    }
                });
                terminate(
                    driver,
                    MatchDriverStatus::InputChannelFailure,
                    detail,
                    Some(CoordinatorNetcodeFailure::InputChannel),
                    None,
                );
            }
            TransportPeerEventKind::StarState => {
                if matches!(
                    event.state,
                    Some(TransportPeerState::Closed)
                        | Some(TransportPeerState::Error)
                        | Some(TransportPeerState::Disconnected)
                ) {
                    terminate(
                        driver,
                        MatchDriverStatus::TransportLost,
                        "the star transport is no longer connected".to_string(),
                        None,
                        None,
                    );
                }
            }
            TransportPeerEventKind::PeerState => {
                // Every link is frozen at the countdown, so losing one after
                // the freeze ends the match rather than shrinking the roster.
                if matches!(
                    event.state,
                    Some(TransportPeerState::Closed)
                        | Some(TransportPeerState::Error)
                        | Some(TransportPeerState::Disconnected)
                ) {
                    terminate(
                        driver,
                        MatchDriverStatus::TransportLost,
                        "a frozen peer link was lost".to_string(),
                        None,
                        None,
                    );
                }
            }
        }
    }
}

fn poll_input(driver: &mut MatchDriver, batch: &mut MatchDriverBatch) -> Vec<TransportPeerMessage> {
    let polled = driver.transport.poll_batch(Some(POLL_BATCH_LIMIT));
    let mut messages = Vec::new();
    for entry in polled {
        if entry.channel == TransportChannel::Input {
            messages.push(entry);
        } else {
            batch.control.push(entry);
        }
    }
    messages
}

// ---------------------------------------------------------------------------
// Host collection
// ---------------------------------------------------------------------------

fn collect_arrivals(
    driver: &mut MatchDriver,
    messages: Vec<TransportPeerMessage>,
    transport_tick: i64,
) -> Option<Vec<InputPacketArrival>> {
    let mut arrivals = Vec::new();
    for entry in messages {
        let context = input_protocol::DecodeContext {
            session_id: driver.manifest.session_id.clone(),
            manifest_id: driver.manifest_id.clone(),
            sender_id: entry.peer_id.clone(),
        };
        let packet = match input_protocol::decode(&entry.message.payload, &context) {
            Ok(packet) => packet,
            Err(err) => {
                let status = if err.code == input_protocol::ErrorCode::OwnershipMismatch {
                    MatchDriverStatus::OwnershipViolation
                } else {
                    MatchDriverStatus::InputChannelFailure
                };
                terminate(
                    driver,
                    status,
                    err.message,
                    Some(CoordinatorNetcodeFailure::InputChannel),
                    None,
                );
                return None;
            }
        };
        let slot_index = packet.rows[0].slot_index;
        let owned = driver
            .peer_slots
            .get(&entry.peer_id)
            .cloned()
            .unwrap_or_default();
        if !owned.contains(&slot_index) {
            terminate(
                driver,
                MatchDriverStatus::OwnershipViolation,
                "a peer authored a slot outside its frozen owned set".to_string(),
                Some(CoordinatorNetcodeFailure::InputChannel),
                None,
            );
            return None;
        }
        arrivals.push(InputPacketArrival {
            packet,
            envelope: entry.message,
            arrival_tick: transport_tick,
            transport_peer_id: entry.peer_id,
        });
    }
    // The host's own human and bot bundles enter through the same collector
    // and only become readable once they have spent the versioned fairness
    // delay.
    let mut remaining = Vec::new();
    for pending in std::mem::take(&mut driver.pending) {
        if pending.due <= driver.step {
            arrivals.push(InputPacketArrival {
                packet: pending.packet,
                envelope: pending.envelope,
                arrival_tick: transport_tick,
                transport_peer_id: driver.peer_id.clone(),
            });
        } else {
            remaining.push(pending);
        }
    }
    driver.pending = remaining;
    Some(arrivals)
}

fn arrival_order(
    driver: &MatchDriver,
    left: &InputPacketArrival,
    right: &InputPacketArrival,
) -> std::cmp::Ordering {
    let left_local = left.transport_peer_id == driver.peer_id;
    let right_local = right.transport_peer_id == driver.peer_id;
    if left_local != right_local {
        return if left_local {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
    }
    left.packet
        .transport_tick
        .cmp(&right.packet.transport_tick)
        .then_with(|| left.packet.sender_id.cmp(&right.packet.sender_id))
        .then_with(|| left.packet.sequence.cmp(&right.packet.sequence))
}

fn select_within_bound(
    driver: &mut MatchDriver,
    mut arrivals: Vec<InputPacketArrival>,
    budget: i64,
) -> (Vec<InputPacketArrival>, IndexSet<(i64, i64)>, i64) {
    arrivals.sort_by(|l, r| arrival_order(driver, l, r));
    let mut selected = Vec::new();
    let mut deferred = Vec::new();
    let mut keys: IndexSet<(i64, i64)> = IndexSet::new();
    let mut count = 0i64;
    for arrival in arrivals {
        let mut added: IndexSet<(i64, i64)> = IndexSet::new();
        let mut additional = 0i64;
        for row in &arrival.packet.rows {
            let key = (row.tick, row.slot_index);
            if !keys.contains(&key) && !added.contains(&key) {
                added.insert(key);
                additional += 1;
            }
        }
        if deferred.is_empty() && count + additional <= budget {
            for row in &arrival.packet.rows {
                keys.insert((row.tick, row.slot_index));
            }
            count += additional;
            selected.push(arrival);
        } else {
            // Once one arrival is held back, everything after it is held
            // back too, so the carried-over stream keeps its canonical
            // order.
            deferred.push(arrival);
        }
    }
    let deferred_len = deferred.len() as i64;
    driver.deferred = deferred;
    if deferred_len > TRANSPORT_MAX_QUEUE_LIMIT {
        terminate(
            driver,
            MatchDriverStatus::InputChannelFailure,
            "the host authority backlog exceeded its bounded queue".to_string(),
            Some(CoordinatorNetcodeFailure::InputChannel),
            None,
        );
    }
    (selected, keys, count)
}

fn remember_relay(driver: &mut MatchDriver, selected: &[InputPacketArrival]) {
    for arrival in selected {
        let rows = &arrival.packet.rows;
        let slot_index = rows[0].slot_index;
        let newest = rows[rows.len() - 1].tick;
        let replace = match driver.relay.get(&slot_index) {
            None => true,
            Some(held) => newest > held.packet.rows[held.packet.rows.len() - 1].tick,
        };
        if replace {
            driver.relay.insert(slot_index, arrival.clone());
        }
    }
}

fn remember_confirmation(driver: &mut MatchDriver, arrivals: &[InputPacketArrival]) {
    for arrival in arrivals {
        if arrival.transport_peer_id != driver.peer_id {
            let slot_index = arrival.packet.rows[0].slot_index;
            let reported = input_protocol::confirmed_tick(&arrival.packet);
            let update = match driver.peer_confirmed.get(&slot_index) {
                None => true,
                Some(&held) => reported > held,
            };
            if update {
                driver.peer_confirmed.insert(slot_index, reported);
            }
        }
    }
}

fn plan_repair(
    driver: &MatchDriver,
    capacity_ticks: i64,
) -> Option<Vec<input_protocol::AuthorityRow>> {
    let mut frontier: Option<i64> = None;
    for slot_index in 1..=input_frame::SLOT_COUNT {
        if let Some(&confirmed) = driver.peer_confirmed.get(&slot_index) {
            frontier = Some(match frontier {
                None => confirmed,
                Some(f) => f.min(confirmed),
            });
        }
    }
    let frontier = frontier?;
    if capacity_ticks <= 0 {
        return None;
    }
    let needed = frontier + 1;
    let host_confirmed = confirmed_input_tick(driver);
    if needed > host_confirmed || host_confirmed - needed <= input_protocol::HISTORY_ROWS {
        return None;
    }
    if needed < retained_floor_input_tick(driver) {
        return None;
    }
    let mut rows = Vec::new();
    let span = REPAIR_SPAN_TICKS.min(capacity_ticks);
    let last = (needed + span - 1).min(host_confirmed);
    let mut tick = needed;
    while tick <= last {
        let mut complete = Vec::new();
        for slot_index in 1..=input_frame::SLOT_COUNT {
            match rollback_session::authoritative_sample(
                &driver.session,
                session_tick(driver, tick),
                slot_index,
            ) {
                Some(sample) => complete.push(input_protocol::AuthorityRow {
                    tick,
                    slot_index,
                    sample,
                }),
                None => break,
            }
        }
        if (complete.len() as i64) < input_frame::SLOT_COUNT {
            break;
        }
        rows.extend(complete);
        tick += 1;
    }
    if rows.is_empty() { None } else { Some(rows) }
}

fn fill_relay_window(
    driver: &MatchDriver,
    selected: &mut Vec<InputPacketArrival>,
    keys: &mut IndexSet<(i64, i64)>,
    mut count: i64,
    budget: i64,
    transport_tick: i64,
) -> i64 {
    let mut covered = [false; 8];
    for arrival in selected.iter() {
        covered[(arrival.packet.rows[0].slot_index - 1) as usize] = true;
    }
    let floor = retained_floor_input_tick(driver);
    for slot_index in 1..=input_frame::SLOT_COUNT {
        if covered[(slot_index - 1) as usize] {
            continue;
        }
        let Some(held) = driver.relay.get(&slot_index) else {
            continue;
        };
        if held.packet.rows[0].tick < floor {
            continue;
        }
        let mut additional = 0i64;
        for row in &held.packet.rows {
            if !keys.contains(&(row.tick, row.slot_index)) {
                additional += 1;
            }
        }
        if count + additional <= budget {
            for row in &held.packet.rows {
                keys.insert((row.tick, row.slot_index));
            }
            count += additional;
            selected.push(InputPacketArrival {
                packet: held.packet.clone(),
                envelope: held.envelope.clone(),
                arrival_tick: transport_tick,
                transport_peer_id: held.transport_peer_id.clone(),
            });
        }
    }
    count
}

fn host_sequence_authority(
    driver: &mut MatchDriver,
    batch: &mut MatchDriverBatch,
    transport_tick: i64,
    messages: Vec<TransportPeerMessage>,
) {
    let Some(mut arrivals) = collect_arrivals(driver, messages, transport_tick) else {
        return;
    };
    // Anything held back by the row bound last tick is re-stamped onto this
    // transport tick and sequenced ahead of the new arrivals.
    let carried = std::mem::take(&mut driver.deferred);
    for mut arrival in carried {
        arrival.arrival_tick = transport_tick;
        arrivals.push(arrival);
    }
    remember_confirmation(driver, &arrivals);
    let budget = input_protocol::MAX_HOST_ROWS;
    let (mut selected, mut keys, mut count) = select_within_bound(driver, arrivals, budget);
    if driver.status != MatchDriverStatus::Active {
        return;
    }
    remember_relay(driver, &selected);
    let repairs = plan_repair(driver, (budget - count) / input_frame::SLOT_COUNT);
    count += repairs.as_ref().map_or(0, |r| r.len() as i64);
    let _ = fill_relay_window(
        driver,
        &mut selected,
        &mut keys,
        count,
        budget,
        transport_tick,
    );
    if selected.is_empty() {
        return;
    }

    let sequence = next_sequence(driver, &format!("{}.batch", driver.peer_id));
    let confirmed_span = input_protocol::confirmed_span(driver.first, confirmed_input_tick(driver));
    let peer_id = driver.peer_id.clone();
    let packet = {
        let request = HostBatchRequest {
            manifest: &driver.manifest,
            assignments: &driver.freeze.assignments,
            host_peer_id: &peer_id,
            sequence,
            transport_tick,
            first_input_tick: driver.first,
            confirmed_span,
            repair_rows: repairs.as_deref(),
            arrivals: &selected,
        };
        driver.rules.canonical_host_batch(request)
    };
    let packet = match packet {
        Ok(packet) => packet,
        Err(err) => {
            let status = match err.code {
                HostBatchErrorCode::OwnershipMismatch => MatchDriverStatus::OwnershipViolation,
                HostBatchErrorCode::AuthorityConflict | HostBatchErrorCode::PacketConflict => {
                    MatchDriverStatus::AuthorityConflict
                }
                HostBatchErrorCode::Other => MatchDriverStatus::InputChannelFailure,
            };
            terminate(
                driver,
                status,
                err.message,
                Some(CoordinatorNetcodeFailure::InputChannel),
                None,
            );
            return;
        }
    };
    apply_rows(driver, &packet.rows, batch, true);
    if driver.status != MatchDriverStatus::Active {
        return;
    }
    let wire = input_protocol::encode(&packet).expect("a canonicalized host packet always encodes");
    let envelope = TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq: packet.sequence,
        tick: Some(packet.transport_tick),
        payload: wire,
    };
    match driver
        .transport
        .broadcast(TransportChannel::Input, envelope)
    {
        Ok(_delivered) => {
            batch.sent_packets += 1;
        }
        Err(TransportError { code, message }) => {
            let status = if code == TransportErrorCode::Backpressure {
                MatchDriverStatus::InputChannelFailure
            } else {
                MatchDriverStatus::TransportLost
            };
            terminate(
                driver,
                status,
                message,
                Some(CoordinatorNetcodeFailure::InputChannel),
                None,
            );
        }
    }
}

fn guest_apply_authority(
    driver: &mut MatchDriver,
    batch: &mut MatchDriverBatch,
    messages: Vec<TransportPeerMessage>,
) {
    let mut rows: Vec<input_protocol::AuthorityRow> = Vec::new();
    let mut seen: IndexMap<(i64, i64), InputSample> = IndexMap::new();
    for entry in messages {
        let context = input_protocol::DecodeContext {
            session_id: driver.manifest.session_id.clone(),
            manifest_id: driver.manifest_id.clone(),
            sender_id: entry.peer_id.clone(),
        };
        let packet = match input_protocol::decode(&entry.message.payload, &context) {
            Ok(packet) => packet,
            Err(err) => {
                terminate(
                    driver,
                    MatchDriverStatus::InputChannelFailure,
                    err.message,
                    Some(CoordinatorNetcodeFailure::InputChannel),
                    None,
                );
                return;
            }
        };
        if packet.kind != input_protocol::PacketKind::Host {
            terminate(
                driver,
                MatchDriverStatus::OwnershipViolation,
                "a guest received authority that was not a host batch".to_string(),
                Some(CoordinatorNetcodeFailure::InputChannel),
                None,
            );
            return;
        }
        let host_confirmed = input_protocol::confirmed_tick(&packet);
        driver.host_confirmed = Some(match driver.host_confirmed {
            None => host_confirmed,
            Some(h) => h.max(host_confirmed),
        });
        for row in &packet.rows {
            let key = (row.tick, row.slot_index);
            match seen.get(&key) {
                None => {
                    seen.insert(key, row.sample);
                    rows.push(row.clone());
                }
                Some(prior) => {
                    if *prior != row.sample {
                        terminate(
                            driver,
                            MatchDriverStatus::AuthorityConflict,
                            "two host batches on one transport tick disagreed on authority"
                                .to_string(),
                            Some(CoordinatorNetcodeFailure::InputChannel),
                            None,
                        );
                        return;
                    }
                }
            }
        }
    }
    // Callback order cannot reach the simulation: the union is sorted into
    // the canonical (tick, slot) order before one atomic application.
    rows.sort_by(|l, r| {
        l.tick
            .cmp(&r.tick)
            .then_with(|| l.slot_index.cmp(&r.slot_index))
    });
    apply_rows(driver, &rows, batch, true);
}

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

fn publish_checkpoints(driver: &mut MatchDriver, batch: &mut MatchDriverBatch) {
    let confirmed = confirmed_output_input_tick(driver);
    while driver.status == MatchDriverStatus::Active && driver.next_checkpoint <= confirmed + 1 {
        let boundary = driver.next_checkpoint;
        let lookup = rollback_session::snapshot(&driver.session, session_tick(driver, boundary));
        if !matches!(
            lookup.status,
            RollbackSnapshotLookupStatus::Present | RollbackSnapshotLookupStatus::Retained
        ) {
            terminate(
                driver,
                MatchDriverStatus::LateInput,
                "a confirmed hash checkpoint fell out of the retained window".to_string(),
                Some(CoordinatorNetcodeFailure::LateInput),
                Some(boundary),
            );
            return;
        }
        let hash = match_snapshot::hash(
            lookup
                .snapshot
                .as_ref()
                .expect("checkpoint boundary snapshot is missing"),
        );
        let live_entry = driver
            .live
            .get(&boundary)
            .cloned()
            .expect("the live-slot timeline has no entry for a confirmed checkpoint");
        let live = copy_live_map(driver, &live_entry);
        driver.checkpoints.push(MatchDriverCheckpoint {
            tick: boundary,
            hash: hash.clone(),
            live: live.clone(),
        });
        driver.checkpoint_by_tick.insert(boundary, hash.clone());
        batch.checkpoints.push(MatchDriverCheckpoint {
            tick: boundary,
            hash,
            live,
        });
        driver.next_checkpoint = boundary + driver.hash_interval;
    }
}

/// Compare a peer's published checkpoint.
///
/// Returns `false` exactly when the boundary was hashed by this peer and
/// disagreed.
pub fn observe_checkpoint(driver: &mut MatchDriver, tick: i64, hash: &str) -> bool {
    let mine = match driver.checkpoint_by_tick.get(&tick) {
        Some(h) => h.clone(),
        None => return true,
    };
    if mine == hash {
        driver.hash_mismatches = 0;
        return true;
    }
    driver.hash_mismatches += 1;
    if driver.hash_mismatches >= MAX_HASH_MISMATCHES {
        terminate(
            driver,
            MatchDriverStatus::HashMismatch,
            format!("boundary hashes disagreed at {MAX_HASH_MISMATCHES} consecutive checkpoints"),
            Some(CoordinatorNetcodeFailure::Desync),
            Some(tick),
        );
    }
    false
}

/// Every checkpoint published so far.
#[must_use]
pub fn checkpoints(driver: &MatchDriver) -> Vec<MatchDriverCheckpoint> {
    driver.checkpoints.clone()
}

// ---------------------------------------------------------------------------
// Confirmation liveness
// ---------------------------------------------------------------------------

fn detect_confirmation_stall(driver: &mut MatchDriver) {
    if driver.status != MatchDriverStatus::Active {
        return;
    }
    let diagnostics = rollback_session::diagnostics(&driver.session);
    let floor = diagnostics.input_history.oldest_retained_tick;
    let gap = diagnostics.confirmed_tick + 1;
    if gap >= floor {
        return;
    }
    let stalled = to_input_tick(driver, gap);
    let tick = driver.late_input_tick.unwrap_or(stalled);
    driver.late_input_tick = Some(tick);
    let detail = format!(
        "confirmation stopped advancing at input tick {}: tick {} never became authoritative and has fallen below the retained floor at tick {}",
        stalled - 1,
        stalled,
        to_input_tick(driver, floor)
    );
    terminate(
        driver,
        MatchDriverStatus::ConfirmationStalled,
        detail,
        Some(CoordinatorNetcodeFailure::LateInput),
        Some(stalled),
    );
}

// ---------------------------------------------------------------------------
// Full time and settling
// ---------------------------------------------------------------------------

fn reach_full_time(driver: &mut MatchDriver) {
    if driver.full_time_boundary.is_some() {
        return;
    }
    driver.full_time_boundary = Some(present_input_tick(driver));
    driver.settle_deadline = Some((driver.clock)() + driver.settle_seconds);
}

fn settle_tail_tick(driver: &mut MatchDriver) -> i64 {
    let newest = driver.authored_tick;
    let host_confirmed = if driver.role == DriverRole::Host {
        Some(confirmed_input_tick(driver))
    } else {
        driver.host_confirmed
    };
    let Some(host_confirmed) = host_confirmed else {
        return newest;
    };
    let anchored = host_confirmed + 1 + input_protocol::HISTORY_ROWS;
    let oldest = (newest - rollback_input_history::ROLLBACK_WINDOW_TICKS
        + input_protocol::HISTORY_ROWS)
        .max(driver.first);
    oldest.max(newest.min(anchored))
}

fn resend_tail(driver: &mut MatchDriver, batch: &mut MatchDriverBatch, transport_tick: i64) {
    let tick = settle_tail_tick(driver);
    for slot_index in driver.authored.clone() {
        let (packet, envelope) = build_packet(driver, slot_index, tick, transport_tick);
        if driver.role == DriverRole::Host {
            driver.pending.push(MatchDriverPending {
                due: driver.step + DELAY_TICKS,
                packet: packet.clone(),
                envelope,
                producer_id: packet.sender_id,
            });
        } else {
            match driver
                .transport
                .send(HOST_PEER_ID, TransportChannel::Input, envelope)
            {
                Ok(_) => {
                    batch.sent_packets += 1;
                }
                Err(TransportError { code, message }) => {
                    let status = if code == TransportErrorCode::Backpressure {
                        MatchDriverStatus::InputChannelFailure
                    } else {
                        MatchDriverStatus::TransportLost
                    };
                    terminate(
                        driver,
                        status,
                        message,
                        Some(CoordinatorNetcodeFailure::InputChannel),
                        None,
                    );
                    return;
                }
            }
        }
    }
}

fn tail_delivered(driver: &MatchDriver) -> bool {
    let Some(boundary) = driver.full_time_boundary else {
        return true;
    };
    if driver.role != DriverRole::Host {
        // Silence is not consent: a guest that has never heard a host batch
        // has no evidence its tail landed, and must keep re-publishing.
        return driver.host_confirmed.is_some_and(|h| h + 1 >= boundary);
    }
    for slot_index in 1..=input_frame::SLOT_COUNT {
        if let Some(&confirmed) = driver.peer_confirmed.get(&slot_index)
            && confirmed + 1 < boundary
        {
            return false;
        }
    }
    true
}

fn settle(driver: &mut MatchDriver) {
    if driver.full_time_boundary.is_none() || driver.status != MatchDriverStatus::Active {
        return;
    }
    let boundary = driver.full_time_boundary.expect("checked above");
    let confirmed = confirmed_output_input_tick(driver) + 1 >= boundary;
    if confirmed && tail_delivered(driver) {
        driver.settled = true;
        terminate(
            driver,
            MatchDriverStatus::Completed,
            "the match reached full time with the final boundary confirmed".to_string(),
            None,
            None,
        );
        return;
    }
    let mut expired = driver.settle_steps >= driver.settle_ticks;
    let mut detail = format!(
        "full time was reached but the final boundary was still unconfirmed after {} settle ticks",
        driver.settle_ticks
    );
    let now = (driver.clock)();
    if !expired
        && now
            >= driver
                .settle_deadline
                .expect("settle deadline set at full time")
    {
        expired = true;
        detail = format!(
            "full time was reached but the final boundary was still unconfirmed after {} seconds of settling",
            driver.settle_seconds
        );
    }
    if !expired {
        driver.settle_steps += 1;
        return;
    }
    if confirmed {
        driver.settled = true;
        terminate(
            driver,
            MatchDriverStatus::Completed,
            "the match reached full time with the final boundary confirmed".to_string(),
            None,
            None,
        );
        return;
    }
    terminate(
        driver,
        MatchDriverStatus::SettleTimeout,
        detail,
        Some(CoordinatorNetcodeFailure::InputChannel),
        Some(boundary),
    );
}

/// True once the final boundary was confirmed.
#[must_use]
pub fn settled(driver: &MatchDriver) -> bool {
    driver.settled
}

/// The final boundary, in input-tick space, once full time is reached.
#[must_use]
pub fn full_time_boundary(driver: &MatchDriver) -> Option<i64> {
    driver.full_time_boundary
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

fn prime(driver: &mut MatchDriver) {
    let geometry = if driver.authored.len() > 1 {
        Some(present_state(driver))
    } else {
        None
    };
    let mut tick = driver.first;
    while tick < driver.first + DELAY_TICKS {
        let authored = materialize_authored(driver, tick, None, geometry.clone());
        for slot_index in driver.authored.clone() {
            let sample = authored[&slot_index];
            record_authored(driver, slot_index, tick, sample);
        }
        tick += 1;
    }
    if driver.role != DriverRole::Host {
        return;
    }
    let last = driver.first + DELAY_TICKS - 1;
    for slot_index in driver.authored.clone() {
        let (packet, envelope) = build_packet(driver, slot_index, last, 0);
        driver.pending.push(MatchDriverPending {
            due: 0,
            packet: packet.clone(),
            envelope,
            producer_id: packet.sender_id,
        });
    }
}

/// Constructs a [`MatchDriver`].
///
/// # Panics
///
/// Panics on every producer invariant this constructor asserts (ARCHITECTURE.md
/// §3 rule 5): an empty peer id, an out-of-range hash interval/settle
/// window/rollback window, or a freeze with an unassigned slot.
#[must_use]
pub fn new(options: MatchDriverOptions) -> MatchDriver {
    let freeze = options.freeze;
    let manifest = options.manifest;
    let role = options.role;
    let peer_id = options.peer_id;
    assert!(!peer_id.is_empty(), "match driver requires a peer id");
    let manifest_id = freeze.manifest_id.clone();

    let hash_interval = options
        .hash_interval_ticks
        .unwrap_or(DEFAULT_HASH_INTERVAL_TICKS);
    assert!(
        hash_interval >= 1,
        "match driver hash interval must be a positive integer"
    );
    let settle_ticks = options.settle_timeout_ticks.unwrap_or(SETTLE_TIMEOUT_TICKS);
    assert!(
        settle_ticks >= 1,
        "match driver settle window must be a positive integer number of ticks"
    );
    let settle_seconds = options
        .settle_timeout_seconds
        .unwrap_or(SETTLE_TIMEOUT_SECONDS);
    assert!(
        settle_seconds.is_finite() && settle_seconds > 0.0,
        "match driver settle window must be a positive number of seconds"
    );
    let clock = options
        .clock
        .unwrap_or_else(|| Box::new(default_clock) as Box<dyn FnMut() -> f64>);
    let maximum = options
        .max_rollback_ticks
        .unwrap_or(rollback_input_history::ROLLBACK_WINDOW_TICKS);
    assert!(
        (1..=rollback_input_history::ROLLBACK_WINDOW_TICKS).contains(&maximum),
        "match driver rollback window must be a bounded positive integer"
    );

    let owned = owned_slots(&freeze.assignments, &peer_id);
    let mut owned_set = [false; 8];
    for &slot in &owned {
        owned_set[(slot_index_of(slot) - 1) as usize] = true;
    }

    let mut peer_slots: IndexMap<String, Vec<i64>> = IndexMap::new();
    let mut humans: Vec<String> = Vec::new();
    let mut bot_slots: Vec<i64> = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = &freeze.assignments[(index - 1) as usize];
        if producer.producer_kind == ProducerKind::Peer {
            if !peer_slots.contains_key(&producer.producer_id) {
                peer_slots.insert(producer.producer_id.clone(), Vec::new());
                humans.push(producer.producer_id.clone());
            }
            peer_slots
                .get_mut(&producer.producer_id)
                .expect("just inserted")
                .push(index);
        } else {
            bot_slots.push(index);
        }
    }

    let mut authored: Vec<i64> = Vec::new();
    let mut authored_set = [false; 8];
    let mut sources: [MatchSlotSource; 8] = [MatchSlotSource {
        kind: MatchSlotSourceKind::Neutral,
        seed: None,
    }; 8];
    let mut rollback_sources: [RollbackInputSource; 8] = [RollbackInputSource::Remote; 8];
    for index in 1..=input_frame::SLOT_COUNT {
        let idx = (index - 1) as usize;
        let producer = &freeze.assignments[idx];
        let mine = owned_set[idx]
            || (role == DriverRole::Host && producer.producer_kind == ProducerKind::Bot);
        if mine {
            authored.push(index);
            authored_set[idx] = true;
            let seed = producer
                .bot_seed
                .unwrap_or_else(|| derived_ai_seed(freeze.seed, index));
            sources[idx] = MatchSlotSource {
                kind: MatchSlotSourceKind::Bot,
                seed: Some(seed as f64),
            };
            rollback_sources[idx] = RollbackInputSource::Local;
        } else {
            sources[idx] = MatchSlotSource {
                kind: MatchSlotSourceKind::Neutral,
                seed: None,
            };
            rollback_sources[idx] = RollbackInputSource::Remote;
        }
    }

    let session = rollback_session::new(
        &options.initial_snapshot,
        rollback_sources,
        Some(maximum),
        None,
    );
    let history: [IndexMap<i64, InputSample>; 8] = std::array::from_fn(|_| IndexMap::new());
    let first_input_tick = freeze.first_input_tick;

    let mut driver = MatchDriver {
        snapshot_captures: std::cell::Cell::new(0),
        role,
        peer_id,
        manifest,
        manifest_id,
        transport: options.transport,
        session,
        sources: rollback_sources,
        producer: slot_input::new_producer(sources),
        owned,
        authored,
        authored_set,
        owned_set,
        peer_slots,
        humans: humans.clone(),
        bot_slots,
        first: first_input_tick,
        step: 0,
        live: IndexMap::new(),
        carrier: IndexMap::new(),
        live_tick: first_input_tick,
        history,
        sequences: IndexMap::new(),
        pending: Vec::new(),
        deferred: Vec::new(),
        relay: IndexMap::new(),
        peer_confirmed: IndexMap::new(),
        host_confirmed: None,
        checkpoints: Vec::new(),
        checkpoint_by_tick: IndexMap::new(),
        hash_interval,
        next_checkpoint: first_input_tick,
        hash_mismatches: 0,
        status: MatchDriverStatus::Active,
        terminal: None,
        late_input_tick: None,
        authored_tick: first_input_tick + DELAY_TICKS - 1,
        settle_ticks,
        settle_seconds,
        clock,
        full_time_boundary: None,
        settle_steps: 0,
        settle_deadline: None,
        settled: false,
        rules: options.rules,
        freeze,
    };

    let mut opening: IndexMap<String, SlotId> = IndexMap::new();
    for producer_id in &humans {
        let slot = *driver
            .freeze
            .live
            .get(producer_id)
            .expect("a human has no opening live slot");
        opening.insert(producer_id.clone(), slot);
    }
    driver.live.insert(first_input_tick, opening);
    let boundary = boundary_state(&driver, first_input_tick);
    let carrier = driver.rules.carrier(&boundary);
    driver.carrier.insert(first_input_tick, carrier);
    prime(&mut driver);
    driver
}

// ---------------------------------------------------------------------------
// The fixed-tick step
// ---------------------------------------------------------------------------

fn author_and_send(
    driver: &mut MatchDriver,
    batch: &mut MatchDriverBatch,
    input_tick: i64,
    transport_tick: i64,
    sample: Option<InputSample>,
) {
    let authored = materialize_authored(driver, input_tick, sample, None);
    for slot_index in driver.authored.clone() {
        let sample = authored[&slot_index];
        record_authored(driver, slot_index, input_tick, sample);
    }
    driver.authored_tick = input_tick;
    let mut local_rows: Vec<input_protocol::AuthorityRow> = Vec::new();
    for slot_index in driver.authored.clone() {
        let (packet, envelope) = build_packet(driver, slot_index, input_tick, transport_tick);
        if driver.role == DriverRole::Host {
            driver.pending.push(MatchDriverPending {
                due: driver.step + DELAY_TICKS,
                packet: packet.clone(),
                envelope,
                producer_id: packet.sender_id,
            });
        } else {
            local_rows.extend(packet.rows.clone());
            match driver
                .transport
                .send(HOST_PEER_ID, TransportChannel::Input, envelope)
            {
                Ok(_) => {
                    batch.sent_packets += 1;
                }
                Err(TransportError { code, message }) => {
                    let status = if code == TransportErrorCode::Backpressure {
                        MatchDriverStatus::InputChannelFailure
                    } else {
                        MatchDriverStatus::TransportLost
                    };
                    terminate(
                        driver,
                        status,
                        message,
                        Some(CoordinatorNetcodeFailure::InputChannel),
                        None,
                    );
                    return;
                }
            }
        }
    }
    // A guest's own rows are local authority immediately, applied as one
    // batch so its own authoring never costs a second reconciliation. The
    // host's canonical echo is byte-identical and therefore idempotent.
    local_rows.sort_by(|l, r| {
        l.tick
            .cmp(&r.tick)
            .then_with(|| l.slot_index.cmp(&r.slot_index))
    });
    apply_rows(driver, &local_rows, batch, false);
}

fn step_to(driver: &mut MatchDriver, batch: &mut MatchDriverBatch, input_tick: i64) {
    loop {
        if driver.status != MatchDriverStatus::Active {
            return;
        }
        let diagnostics = rollback_session::diagnostics(&driver.session);
        if to_input_tick(driver, diagnostics.present_boundary) > input_tick {
            return;
        }
        if diagnostics.status == RollbackSessionStatus::Finished {
            reach_full_time(driver);
            return;
        }
        match rollback_session::step(&mut driver.session) {
            Ok(output) => {
                let tick = to_input_tick(driver, output.tick);
                let input = output.input;
                let finished = output.finished;
                extend_live(driver, tick, &input);
                batch.outputs.push(output);
                let floor = rollback_session::diagnostics(&driver.session)
                    .input_history
                    .oldest_retained_tick;
                prune_live(driver, to_input_tick(driver, floor));
                if finished {
                    reach_full_time(driver);
                    return;
                }
            }
            Err(err) => {
                if err.code == RollbackSessionErrorCode::MatchFinished {
                    reach_full_time(driver);
                    return;
                }
                let tick = driver.late_input_tick;
                terminate(
                    driver,
                    MatchDriverStatus::LateInput,
                    err.message,
                    Some(CoordinatorNetcodeFailure::LateInput),
                    tick,
                );
                return;
            }
        }
    }
}

/// One fixed 60 Hz driver step.
#[must_use]
pub fn advance(driver: &mut MatchDriver, sample: Option<InputSample>) -> MatchDriverBatch {
    let input_tick = driver.first + driver.step;
    let transport_tick = driver.step + DELAY_TICKS;
    let mut batch = MatchDriverBatch {
        step: driver.step,
        input_tick,
        outputs: Vec::new(),
        reconciliations: 0,
        applied_rows: 0,
        corrections: 0,
        rollbacks: 0,
        sent_packets: 0,
        checkpoints: Vec::new(),
        control: Vec::new(),
        live: IndexMap::new(),
        status: driver.status,
    };
    if driver.status != MatchDriverStatus::Active {
        // No hidden progress after a terminal status: nothing is polled,
        // sent, applied, or simulated.
        let entry = driver
            .live
            .get(&driver.live_tick)
            .cloned()
            .unwrap_or_default();
        batch.live = copy_live_map(driver, &entry);
        return batch;
    }

    drain_events(driver);
    let messages = if driver.status == MatchDriverStatus::Active {
        poll_input(driver, &mut batch)
    } else {
        Vec::new()
    };
    if driver.status == MatchDriverStatus::Active {
        if driver.role == DriverRole::Host {
            host_sequence_authority(driver, &mut batch, transport_tick, messages);
        } else {
            guest_apply_authority(driver, &mut batch, messages);
        }
    }
    if driver.status == MatchDriverStatus::Active {
        if driver.full_time_boundary.is_some() {
            resend_tail(driver, &mut batch, transport_tick);
        } else {
            author_and_send(
                driver,
                &mut batch,
                input_tick + DELAY_TICKS,
                transport_tick,
                sample,
            );
        }
    }
    if driver.status == MatchDriverStatus::Active && driver.full_time_boundary.is_none() {
        step_to(driver, &mut batch, input_tick);
    }
    // Boundary hashes stop at full time, including on the step that reaches
    // it.
    if driver.full_time_boundary.is_none() {
        publish_checkpoints(driver, &mut batch);
    }
    // Before settling, because a peer whose confirmation is already dead
    // must say so with the reason that is true rather than wait out a phase
    // whose deadline would describe a different fault.
    detect_confirmation_stall(driver);
    settle(driver);

    driver.step += 1;
    batch.status = driver.status;
    let entry = driver
        .live
        .get(&driver.live_tick)
        .cloned()
        .unwrap_or_default();
    batch.live = copy_live_map(driver, &entry);
    batch
}

// ---------------------------------------------------------------------------
// Pure diagnostics
// ---------------------------------------------------------------------------

/// Current lifecycle status.
#[must_use]
pub fn status(driver: &MatchDriver) -> MatchDriverStatus {
    driver.status
}

/// The terminal outcome, once reached.
#[must_use]
pub fn terminal(driver: &MatchDriver) -> Option<MatchDriverTerminal> {
    driver.terminal.clone()
}

/// An independent capture of the driver's current live boundary.
#[must_use]
pub fn current_snapshot(driver: &MatchDriver) -> MatchSnapshot {
    rollback_session::current_snapshot(&driver.session)
}

/// An owned copy of a retained boundary.
#[must_use]
pub fn snapshot(driver: &MatchDriver, boundary: i64) -> RollbackSnapshotLookup {
    rollback_session::snapshot(&driver.session, session_tick(driver, boundary))
}

/// The coordinator's slot-driver view at the current live tick. Delegates to
/// the injected [`MatchDriverRules::slot_drivers`] (see module doc: this
/// file does not depend on `crate::coordinator` directly).
#[must_use]
pub fn slot_drivers(driver: &MatchDriver) -> Vec<(SlotId, CoordinatorSlotDriver)> {
    let entry = driver
        .live
        .get(&driver.live_tick)
        .cloned()
        .unwrap_or_default();
    driver.rules.slot_drivers(&driver.freeze, &entry)
}

/// A full read of this driver's observable state.
#[must_use]
pub fn diagnostics(driver: &MatchDriver) -> MatchDriverDiagnostics {
    let session = rollback_session::diagnostics(&driver.session);
    let transport = driver.transport.diagnostics();
    let authored: Vec<SlotId> = driver.authored.iter().map(|&i| slot_id_of(i)).collect();
    let control = if !driver.owned.is_empty() {
        Some(control_slot(driver, &driver.peer_id, driver.live_tick))
    } else {
        None
    };
    let live_entry = driver
        .live
        .get(&driver.live_tick)
        .cloned()
        .unwrap_or_default();
    MatchDriverDiagnostics {
        snapshot_captures: driver.snapshot_captures.get(),
        role: driver.role,
        peer_id: driver.peer_id.clone(),
        status: driver.status,
        terminal: driver.terminal.clone(),
        step: driver.step,
        transport_tick: driver.step + DELAY_TICKS,
        present_input_tick: present_input_tick(driver),
        confirmed_input_tick: confirmed_input_tick(driver),
        confirmed_output_tick: confirmed_output_input_tick(driver),
        retained_floor_tick: retained_floor_input_tick(driver),
        owned: driver.owned.clone(),
        authored,
        live: copy_live_map(driver, &live_entry),
        control_slot: control,
        rollback_count: session.rollback_count,
        correction_count: session.correction_count,
        predicted_slot_samples: session.predicted_slot_samples,
        max_rollback_depth: session.max_rollback_depth,
        late_input_tick: driver.late_input_tick,
        hash_mismatches: driver.hash_mismatches,
        checkpoint_count: driver.checkpoints.len() as i64,
        settling: driver.full_time_boundary.is_some() && driver.status == MatchDriverStatus::Active,
        settled: driver.settled,
        full_time_boundary: driver.full_time_boundary,
        settle_steps: driver.settle_steps,
        dropped_outbound: transport.dropped_outbound,
        dropped_inbound: transport.dropped_inbound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ai_seed_stays_in_the_park_miller_range() {
        for slot in 1..=8 {
            let seed = derived_ai_seed(74, slot);
            assert!((1..=2_147_483_646).contains(&seed));
        }
    }

    #[test]
    fn slot_id_and_slot_index_round_trip() {
        for index in 1..=8 {
            assert_eq!(slot_index_of(slot_id_of(index)), index);
        }
    }

    #[test]
    fn switch_edge_reads_the_canonical_switch_bit() {
        let sample = input_frame::new_sample(input_frame::InputSampleOptions {
            edges: Some(input_frame::EDGE_SWITCH),
            ..Default::default()
        })
        .expect("valid sample");
        assert!(switch_edge(&sample));
        let neutral = input_frame::neutral_sample();
        assert!(!switch_edge(&neutral));
    }

    #[test]
    fn owned_slots_filters_by_producer_id_and_kind() {
        let assignments: [SlotAssignment; 8] = std::array::from_fn(|i| SlotAssignment {
            producer_kind: if i == 0 {
                ProducerKind::Peer
            } else {
                ProducerKind::Bot
            },
            producer_id: if i == 0 {
                "guest_1".to_string()
            } else {
                "bot".to_string()
            },
            bot_seed: None,
        });
        let owned = owned_slots(&assignments, "guest_1");
        assert_eq!(owned, vec![SlotId::Home1]);
    }
}
