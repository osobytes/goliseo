//! A [`StarTransportAdapter`] decorator that impairs *arrivals* with the
//! OMP-2 probabilistic network model ([`gc_sim::network_conditions`]).
//!
//! # Why the inbound side
//!
//! Impairing the outbound side would mean intercepting `send`, which is
//! where the star reports `overflow`, `not_connected`, `role_forbidden`, and
//! `malformed` synchronously. Holding a message there would either delay
//! those verdicts or force this module to reimplement them, and a fault
//! harness that reimplements the failure paths it is meant to exercise
//! proves nothing. So `send`/`broadcast` are forwarded verbatim and every
//! code the real star returns is the code the driver sees. The impairment
//! happens on the way *in*: the decorator drains the wrapped endpoint, hands
//! each envelope to [`gc_sim::network_conditions`], and releases it on the
//! arrival tick that module chose.
//!
//! # The profiles are the real ones
//!
//! [`gc_data::network_profiles`] and [`gc_sim::network_conditions`] are used
//! as-is, not re-derived. Every envelope consumes one `send` on the
//! conditions state, so the jitter, independent-loss, duplication, and burst
//! rolls are the documented four per packet in the documented order, against
//! a private network seed. Arrival order is whatever
//! `network_conditions::poll` returns.
//!
//! # What is impaired, and what deliberately is not
//!
//! Only the `input` channel. A WebRTC control channel is reliable and
//! ordered, so silently losing session mail would model a transport this
//! project does not ship. Control-channel faults are therefore *declared*
//! rather than probabilistic: [`FaultTransport::tick`]'s duplication
//! (`duplicate_control_every`) re-delivers every Nth control envelope.
//! [`FaultTransport::withhold`]/[`FaultTransport::hold`] are likewise
//! declared, not probabilistic: a burst that must straddle a named boundary
//! cannot be left to a profile's RNG.
//!
//! # The transport wire contract has no Rust home
//!
//! All of `game/transport/**` is TypeScript-owned (`ts/packages/transport`,
//! `ARCHITECTURE.md` §2's directory layout) and no Rust crate ports it. This
//! decorator's whole job is to wrap *some* [`StarTransportAdapter`], so it
//! needs that interface's shape to be generic over it and to be
//! unit-testable with a fake endpoint — the same role [`crate::fake_star`]
//! plays for the real thing. [`StarTransportAdapter`] and its
//! message/event/diagnostics types below are therefore minimal local
//! restatements of `contract.ts`'s shapes (never its
//! `encode`/`decode`/`validate` *logic*, which stays out of scope here same
//! as everywhere else in this crate). Every field is named and typed to
//! match the TypeScript one for one.
//!
//! `crate::match_driver` reuses these same types rather than redeclaring
//! them, since it decorates the identical interface one layer up.

use gc_data::network_profiles::{self, NetworkProfileName};
use gc_sim::input_frame;
use gc_sim::network_conditions::{self, NetworkConditionDiagnostics, NetworkConditions};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------
// The wire contract's (`ts/packages/transport/src/contract.ts`) shapes,
// restated (see module doc).
// ---------------------------------------------------------------------------

/// Which logical channel one envelope travels on. Mirrors `TransportChannel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportChannel {
    /// Reliable, ordered: session lifecycle and control payloads.
    Control,
    /// Unordered, never retransmitted: tick-numbered input packets.
    Input,
}

/// Which side of a direct-host star one endpoint is. Mirrors `TransportRole`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportRole {
    /// The hub endpoint; addresses every guest link directly.
    Host,
    /// A spoke endpoint; addresses only the host link.
    Guest,
}

/// An endpoint's own connection state. Mirrors `TransportState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportState {
    /// Constructed, not yet initialized.
    New,
    /// Initialized and able to open peers.
    Connected,
    /// No longer connected but not explicitly closed.
    Disconnected,
    /// Explicitly shut down.
    Closed,
    /// Failed terminally.
    Error,
}

/// One peer link's connection state. Mirrors `TransportPeerState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportPeerState {
    /// Allocated, not yet connecting.
    New,
    /// Signaling in progress.
    Connecting,
    /// Ready to send and receive.
    Connected,
    /// No longer connected but not explicitly closed.
    Disconnected,
    /// Explicitly closed.
    Closed,
    /// Failed terminally.
    Error,
}

/// Machine-readable transport failure reason. Mirrors `TransportErrorCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportErrorCode {
    /// The endpoint has not been initialized.
    NotInitialized,
    /// The endpoint or peer link is not connected.
    NotConnected,
    /// The endpoint or peer link is closed.
    Closed,
    /// The data violates a structural or range invariant.
    Malformed,
    /// The data names a version this build does not support.
    UnsupportedVersion,
    /// The payload exceeds the byte budget.
    PayloadTooLarge,
    /// A bounded queue exceeded its limit.
    Overflow,
    /// The underlying signaling/data-channel bridge is unavailable.
    BridgeUnavailable,
    /// The underlying bridge reported an error.
    BridgeError,
    /// The peer link disconnected.
    Disconnected,
    /// The star is at capacity.
    Capacity,
    /// The named peer is not known to this endpoint.
    UnknownPeer,
    /// The named peer is already open.
    DuplicatePeer,
    /// The operation is forbidden for this endpoint's role.
    RoleForbidden,
    /// The channel named does not match the message type.
    ChannelMismatch,
    /// A send buffer is clamped and currently full.
    Backpressure,
    /// Manual signaling failed.
    SignalError,
}

/// An expected, recoverable transport failure (AGENTS.md §7): the caller is
/// meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportError {
    /// Machine-readable failure reason.
    pub code: TransportErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl TransportError {
    /// Builds a [`TransportError`] from a code and a message.
    #[must_use]
    pub fn new(code: TransportErrorCode, message: impl Into<String>) -> Self {
        TransportError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TransportError {}

/// Result alias for fallible [`StarTransportAdapter`] operations.
pub type TransportResult<T> = std::result::Result<T, TransportError>;

/// Which payload shape one [`TransportMessage`] carries. Mirrors
/// `TransportMessageType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportMessageType {
    /// A tick-numbered input packet ([`crate::input_protocol`]'s wire).
    Input,
    /// A session lifecycle/control event.
    Event,
    /// A session state payload.
    State,
}

/// One wire envelope, already validated against `contract.ts`'s bounds.
/// Mirrors `TransportMessage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportMessage {
    /// Wire format version.
    pub version: i64,
    /// Which payload shape [`Self::payload`] holds.
    pub kind: TransportMessageType,
    /// Monotonic transport sequence, starting at zero.
    pub seq: i64,
    /// Required for `input` messages; `None` for control messages.
    pub tick: Option<i64>,
    /// The next protocol layer's opaque payload.
    pub payload: Vec<u8>,
}

/// One [`TransportMessage`] addressed to or received from a specific link.
/// Mirrors `TransportAddressedMessage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportAddressedMessage {
    /// Link identity chosen by the star, never read from the payload.
    pub peer_id: String,
    /// Which channel this message travelled on.
    pub channel: TransportChannel,
    /// The envelope itself.
    pub message: TransportMessage,
}

/// One [`TransportAddressedMessage`] as delivered by `poll`/`poll_batch`.
/// Mirrors `TransportPeerMessage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportPeerMessage {
    /// Link identity chosen by the star, never read from the payload.
    pub peer_id: String,
    /// Which channel this message travelled on.
    pub channel: TransportChannel,
    /// The envelope itself.
    pub message: TransportMessage,
    /// Per-peer transport receive counter; not a simulation tick.
    pub arrival_seq: i64,
}

/// Which condition a [`TransportPeerEvent`] reports. Mirrors the
/// `TransportPeerEvent.kind` union.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportPeerEventKind {
    /// One peer link changed state.
    PeerState,
    /// One peer link reported an error.
    PeerError,
    /// The star endpoint itself changed state.
    StarState,
    /// The star endpoint itself reported an error.
    StarError,
}

/// One event drained from `poll_event`. Mirrors `TransportPeerEvent`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportPeerEvent {
    /// Which condition this event reports.
    pub kind: TransportPeerEventKind,
    /// The peer this event concerns, when it concerns one peer.
    pub peer_id: Option<String>,
    /// The channel this event concerns, when relevant.
    pub channel: Option<TransportChannel>,
    /// The new peer state, for a `peer_state` event.
    pub state: Option<TransportPeerState>,
    /// Machine-readable failure reason, for an error event.
    pub code: Option<TransportErrorCode>,
    /// Human-readable detail.
    pub message: Option<String>,
}

/// One channel's queue/traffic counters. Mirrors `TransportChannelDiagnostics`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TransportChannelDiagnostics {
    /// This channel's connection state.
    pub state: Option<TransportPeerState>,
    /// Envelopes queued to send.
    pub outbound_depth: i64,
    /// Envelopes queued to be polled.
    pub inbound_depth: i64,
    /// Bytes currently buffered for send.
    pub buffered_amount: i64,
    /// Envelopes sent.
    pub sent: i64,
    /// Envelopes received.
    pub received: i64,
    /// Outbound envelopes dropped.
    pub dropped_outbound: i64,
    /// Inbound envelopes dropped.
    pub dropped_inbound: i64,
}

/// One peer link's full diagnostics. Mirrors `TransportPeerDiagnostics`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TransportPeerDiagnostics {
    /// This link's peer id.
    pub peer_id: String,
    /// Stable star slot, `1..=MAX_GUESTS` on a host, always `1` on a guest.
    pub slot: i64,
    /// This link's connection state.
    pub state: Option<TransportPeerState>,
    /// The underlying ICE connection state, as reported by the bridge.
    pub ice_state: String,
    /// The control channel's diagnostics.
    pub control: TransportChannelDiagnostics,
    /// The input channel's diagnostics.
    pub input: TransportChannelDiagnostics,
    /// Sequence gaps observed on this link.
    pub sequence_gaps: i64,
    /// Backpressure events observed on this link.
    pub backpressure: i64,
    /// Malformed envelopes observed on this link.
    pub malformed: i64,
    /// The most recent error message, if any.
    pub last_error: Option<String>,
}

/// A star endpoint's full diagnostics. Mirrors `TransportStarDiagnostics`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TransportStarDiagnostics {
    /// This endpoint's role.
    pub role: Option<TransportRole>,
    /// This endpoint's connection state.
    pub state: Option<TransportState>,
    /// Maximum peer links this endpoint may hold.
    pub capacity: i64,
    /// Currently open peer links.
    pub peer_count: i64,
    /// The bounded queue limit every channel is held to.
    pub queue_limit: i64,
    /// The clamped send-buffer byte limit, if one is configured.
    pub buffered_amount_limit: i64,
    /// Queued but undelivered `poll_event` entries.
    pub event_depth: i64,
    /// Envelopes sent across every link.
    pub sent: i64,
    /// Envelopes received across every link.
    pub received: i64,
    /// Outbound envelopes dropped across every link.
    pub dropped_outbound: i64,
    /// Inbound envelopes dropped across every link.
    pub dropped_inbound: i64,
    /// Malformed envelopes observed.
    pub malformed: i64,
    /// Unsupported-version envelopes observed.
    pub unsupported_version: i64,
    /// Queue-overflow events observed.
    pub overflow: i64,
    /// Backpressure events observed.
    pub backpressure: i64,
    /// The most recent error message, if any.
    pub last_error: Option<String>,
    /// Every open peer link's diagnostics, ordered by slot, never by
    /// arrival.
    pub peers: Vec<TransportPeerDiagnostics>,
}

/// The interface every OMP-3 star endpoint implements: a direct-host star
/// ([`crate::fake_star`]'s fake of one) or a relay ([`crate::fake_relay`]'s).
/// Mirrors `StarTransportAdapter`.
///
/// This crate never implements a *real* endpoint — `game/transport/**` is
/// TypeScript-owned (see module doc) — but [`FaultTransport`] both wraps one
/// generically and, by implementing this trait itself, is substitutable for
/// the endpoint it wraps.
pub trait StarTransportAdapter {
    /// Brings the endpoint up. Idempotent-on-success by convention.
    fn initialize(&mut self) -> TransportResult<bool>;
    /// Tears the endpoint down.
    fn shutdown(&mut self) -> TransportResult<bool>;
    /// This endpoint's role.
    fn role(&self) -> TransportRole;
    /// Maximum peer links this endpoint may hold.
    fn capacity(&self) -> i64;
    /// Allocates a slot for `peer_id` and returns it.
    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64>;
    /// Closes `peer_id`'s link, with an optional reason.
    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool>;
    /// Every currently open peer id.
    fn peer_ids(&self) -> Vec<String>;
    /// `peer_id`'s current link state, if it is known.
    fn peer_state(&self, peer_id: &str) -> Option<TransportPeerState>;
    /// Starts asynchronous offer creation for `peer_id`.
    fn request_offer(&mut self, peer_id: &str) -> TransportResult<bool>;
    /// Accepts a remote offer blob.
    fn accept_offer(&mut self, signal: &str) -> TransportResult<bool>;
    /// Accepts a remote answer blob for `peer_id`.
    fn accept_answer(&mut self, peer_id: &str, signal: &str) -> TransportResult<bool>;
    /// Yields the next outbound signaling blob for `peer_id`, if any.
    fn take_signal(&mut self, peer_id: &str) -> TransportResult<Option<String>>;
    /// Sends one envelope to `peer_id` on `channel`.
    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool>;
    /// Sends one envelope to every open peer on `channel`; returns the
    /// delivery count.
    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64>;
    /// Pops the single oldest queued arrival, if any.
    fn poll(&mut self) -> Option<TransportPeerMessage>;
    /// Pops up to `limit` queued arrivals (the endpoint's own default when
    /// `None`), oldest first.
    fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage>;
    /// Pops the single oldest queued lifecycle event, if any.
    fn poll_event(&mut self) -> Option<TransportPeerEvent>;
    /// This endpoint's own connection state.
    fn state(&self) -> TransportState;
    /// This endpoint's full diagnostics.
    fn diagnostics(&self) -> TransportStarDiagnostics;
}

// ---------------------------------------------------------------------------
// `FaultTransport` itself.
// ---------------------------------------------------------------------------

/// Release order [`FaultTransport::poll_batch`] hands input arrivals to the
/// driver in. Mirrors `FaultTransportPollOrder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultTransportPollOrder {
    /// Whatever order the impairment model released them in.
    Arrival,
    /// Input arrivals last-first; the control channel keeps arrival order in
    /// both modes (see [`FaultTransport::poll_batch`]).
    Reverse,
}

/// Construction options for [`FaultTransport::new`]. Mirrors
/// `FaultTransportOptions`.
pub struct FaultTransportOptions {
    /// The wrapped star endpoint.
    pub transport: Box<dyn StarTransportAdapter>,
    /// The named impairment profile to run the input channel through.
    pub profile: NetworkProfileName,
    /// Network-only seed; never the match seed.
    pub seed: f64,
    /// Remote peer ids this endpoint receives from, canonical order.
    pub legs: Vec<String>,
    /// Release order handed to the driver; defaults to
    /// [`FaultTransportPollOrder::Arrival`].
    pub poll_order: Option<FaultTransportPollOrder>,
    /// Re-deliver every Nth control envelope; `None`/`0` disables it.
    pub duplicate_control_every: Option<i64>,
}

/// A snapshot of every impairment/delivery counter. Mirrors
/// `FaultTransportCounters`.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultTransportCounters {
    /// The profile this transport runs.
    pub profile: NetworkProfileName,
    /// Input envelopes handed to the impairment model.
    pub scheduled: i64,
    /// Input envelopes released to the driver, duplicates included.
    pub delivered: i64,
    /// Input envelopes the model discarded.
    pub dropped: i64,
    /// Independent-loss drops.
    pub independent_lost: i64,
    /// Burst-loss drops.
    pub burst_lost: i64,
    /// Model-created duplicate deliveries.
    pub duplicated: i64,
    /// Unique sequence identities delivered after a later sequence.
    pub reordered: i64,
    /// Input envelopes dropped by a scripted [`FaultTransport::withhold`]
    /// window, not the profile.
    pub withheld: i64,
    /// Input envelopes delayed by a scripted [`FaultTransport::hold`] window
    /// and released after it.
    pub held: i64,
    /// Control envelopes delivered.
    pub control_delivered: i64,
    /// Scripted control duplicates.
    pub control_duplicated: i64,
    /// Envelopes from a peer outside the declared legs.
    pub unmapped: i64,
    /// Currently pending (in-flight) envelopes.
    pub pending: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct DeclaredCounters {
    scheduled: i64,
    delivered: i64,
    dropped: i64,
    withheld: i64,
    held: i64,
    control_delivered: i64,
    control_duplicated: i64,
    unmapped: i64,
}

/// A `[from, through]` transport-tick window, inclusive on both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    from: i64,
    through: i64,
}

fn inside(windows: &[Window], tick: i64) -> bool {
    windows.iter().any(|w| tick >= w.from && tick <= w.through)
}

/// The impairment model addresses legs by canonical slot index, and a
/// direct-host star has at most seven guests plus the host link, so every
/// leg of every endpoint fits inside one slot axis.
pub const MAX_LEGS: i64 = input_frame::SLOT_COUNT;

/// A [`StarTransportAdapter`] decorator that impairs arrivals on the `input`
/// channel with [`gc_sim::network_conditions`]. See the module doc.
pub struct FaultTransport {
    transport: Box<dyn StarTransportAdapter>,
    conditions: NetworkConditions,
    profile: NetworkProfileName,
    /// Remote peer id -> impairment source slot (`1..=8`). Insertion order
    /// is `legs`' order; never iterated for anything determinism-sensitive,
    /// but `IndexMap` costs nothing extra here and keeps the type honest
    /// about being a map (ARCHITECTURE.md §3 rule 4: never `HashMap`/`HashSet`).
    slots: IndexMap<String, i64>,
    next_input_tick: Vec<i64>,
    /// Slot (index `slot - 1`) -> input tick -> queued envelope, awaiting
    /// the impairment model's delivery verdict.
    queued: Vec<IndexMap<i64, TransportPeerMessage>>,
    ready: Vec<TransportPeerMessage>,
    tick: i64,
    order: FaultTransportPollOrder,
    control_every: i64,
    control_seen: i64,
    withhold: Vec<Window>,
    hold: Vec<Window>,
    held: Vec<TransportPeerMessage>,
    counters: DeclaredCounters,
}

impl FaultTransport {
    /// Builds a fault transport wrapping `options.transport`.
    ///
    /// # Panics
    ///
    /// Panics (invariant violations, per AGENTS.md §7) if `options.legs`
    /// exceeds [`MAX_LEGS`], if a leg is declared twice, or if
    /// `options.seed` is not finite.
    #[must_use]
    pub fn new(options: FaultTransportOptions) -> Self {
        assert!(
            options.legs.len() as i64 <= MAX_LEGS,
            "a fault transport models at most eight legs"
        );
        assert!(
            options.seed.is_finite(),
            "a fault transport needs a network seed"
        );
        let mut slots = IndexMap::new();
        for (index, peer_id) in options.legs.iter().enumerate() {
            assert!(
                !slots.contains_key(peer_id),
                "a fault transport leg is declared twice"
            );
            slots.insert(peer_id.clone(), index as i64 + 1);
        }
        let profile: network_conditions::NetworkProfile =
            network_profiles::get(options.profile).into();
        FaultTransport {
            transport: options.transport,
            conditions: network_conditions::new(&profile, options.seed),
            profile: options.profile,
            slots,
            next_input_tick: vec![0; input_frame::SLOT_COUNT as usize],
            queued: (0..input_frame::SLOT_COUNT)
                .map(|_| IndexMap::new())
                .collect(),
            ready: Vec::new(),
            tick: -1,
            order: options
                .poll_order
                .unwrap_or(FaultTransportPollOrder::Arrival),
            control_every: options.duplicate_control_every.unwrap_or(0),
            control_seen: 0,
            withhold: Vec::new(),
            hold: Vec::new(),
            held: Vec::new(),
            counters: DeclaredCounters::default(),
        }
    }

    // -----------------------------------------------------------------
    // Declared (non-probabilistic) faults
    // -----------------------------------------------------------------

    /// Drops every input envelope arriving in `[from, through]`. Declared
    /// rather than rolled, because a burst that has to straddle a named
    /// boundary cannot be left to a profile's RNG.
    ///
    /// # Panics
    ///
    /// Panics if `through < from`.
    pub fn withhold(&mut self, from: i64, through: i64) {
        assert!(
            through >= from,
            "a withheld window ends at or after it starts"
        );
        self.withhold.push(Window { from, through });
    }

    /// Buffers every input envelope arriving in `[from, through]` and
    /// releases the whole backlog on the first tick after it. The only way
    /// to reach `late_input` on purpose: a hold longer than the retained
    /// floor delivers rows the window can no longer accept.
    ///
    /// # Panics
    ///
    /// Panics if `through < from`.
    pub fn hold(&mut self, from: i64, through: i64) {
        assert!(through >= from, "a held window ends at or after it starts");
        self.hold.push(Window { from, through });
    }

    fn withholding(&self) -> bool {
        inside(&self.withhold, self.tick)
    }

    // -----------------------------------------------------------------
    // The impairment pipeline
    // -----------------------------------------------------------------

    fn release(&mut self, entry: TransportPeerMessage) {
        self.ready.push(entry);
    }

    fn schedule(&mut self, entry: TransportPeerMessage) {
        if entry.channel != TransportChannel::Input {
            self.counters.control_delivered += 1;
            self.control_seen += 1;
            let duplicate_now =
                self.control_every > 0 && self.control_seen % self.control_every == 0;
            let duplicate = if duplicate_now {
                Some(TransportPeerMessage {
                    peer_id: entry.peer_id.clone(),
                    channel: entry.channel,
                    message: entry.message.clone(),
                    arrival_seq: entry.arrival_seq,
                })
            } else {
                None
            };
            self.release(entry);
            if let Some(duplicate) = duplicate {
                self.counters.control_duplicated += 1;
                self.release(duplicate);
            }
            return;
        }
        let Some(&slot) = self.slots.get(&entry.peer_id) else {
            // An envelope from a peer outside the declared legs has no
            // impairment identity. Dropping it would be a silent fault the
            // report could not explain, so it is delivered and counted.
            self.counters.unmapped += 1;
            self.release(entry);
            return;
        };
        if self.withholding() {
            self.counters.withheld += 1;
            return;
        }
        if inside(&self.hold, self.tick) {
            self.counters.held += 1;
            self.held.push(entry);
            return;
        }
        let slot_idx = (slot - 1) as usize;
        let input_tick = self.next_input_tick[slot_idx];
        self.next_input_tick[slot_idx] = input_tick + 1;
        let receipt = network_conditions::send(
            &mut self.conditions,
            self.tick,
            slot,
            input_tick,
            &input_frame::neutral_sample(),
        )
        // The impairment model refused to schedule. That is a harness bug,
        // not a transport fault, so it fails loudly rather than quietly
        // delivering.
        .unwrap_or_else(|err| {
            panic!(
                "the impairment model refused a packet at transport tick {}: {err}",
                self.tick
            )
        });
        self.counters.scheduled += 1;
        if receipt.dropped {
            self.counters.dropped += 1;
            return;
        }
        self.queued[slot_idx].insert(input_tick, entry);
    }

    fn ingest(&mut self) {
        loop {
            let batch = self.transport.poll_batch(None);
            if batch.is_empty() {
                return;
            }
            for entry in batch {
                self.schedule(entry);
            }
        }
    }

    fn deliver_due(&mut self) {
        let deliveries = network_conditions::poll(&mut self.conditions, self.tick);
        let mut consumed: Vec<(i64, i64)> = Vec::new();
        for delivery in &deliveries {
            let slot_idx = (delivery.source_slot - 1) as usize;
            let Some(entry) = self.queued[slot_idx].get(&delivery.current.tick) else {
                continue;
            };
            self.counters.delivered += 1;
            if delivery.duplicate_ordinal == 0 {
                self.release(entry.clone());
            } else {
                let duplicate = TransportPeerMessage {
                    peer_id: entry.peer_id.clone(),
                    channel: entry.channel,
                    message: entry.message.clone(),
                    arrival_seq: entry.arrival_seq,
                };
                self.release(duplicate);
            }
            consumed.push((delivery.source_slot, delivery.current.tick));
        }
        // Both ordinals of a duplicated packet arrive on the same tick, so
        // the entry is only released from the queue once every delivery for
        // this tick has been handed out.
        for (slot, tick) in consumed {
            self.queued[(slot - 1) as usize].shift_remove(&tick);
        }
    }

    /// Advances this endpoint's transport clock by exactly one tick: drains
    /// the wrapped endpoint, impairs, and releases everything the model says
    /// is due. The harness calls this once per driver step, before the
    /// driver polls.
    ///
    /// # Panics
    ///
    /// Panics if `transport_tick` does not strictly advance the clock.
    pub fn tick(&mut self, transport_tick: i64) {
        assert!(
            transport_tick > self.tick,
            "a fault transport clock only moves forward"
        );
        self.tick = transport_tick;
        if !self.held.is_empty() && !inside(&self.hold, transport_tick) {
            let backlog = std::mem::take(&mut self.held);
            for entry in backlog {
                self.schedule(entry);
            }
        }
        self.ingest();
        self.deliver_due();
    }

    /// A snapshot of every impairment/delivery counter.
    #[must_use]
    pub fn counters(&self) -> FaultTransportCounters {
        let model = network_conditions::counters(&self.conditions);
        FaultTransportCounters {
            profile: self.profile,
            scheduled: self.counters.scheduled,
            delivered: self.counters.delivered,
            dropped: self.counters.dropped,
            independent_lost: model.independent_lost,
            burst_lost: model.burst_lost,
            duplicated: model.duplicated,
            reordered: model.reordered,
            withheld: self.counters.withheld,
            held: self.counters.held,
            control_delivered: self.counters.control_delivered,
            control_duplicated: self.counters.control_duplicated,
            unmapped: self.counters.unmapped,
            pending: network_conditions::pending(&self.conditions),
        }
    }

    /// The underlying impairment model's own diagnostics.
    #[must_use]
    pub fn model_diagnostics(&self) -> NetworkConditionDiagnostics {
        network_conditions::diagnostics(&self.conditions)
    }
}

impl StarTransportAdapter for FaultTransport {
    fn initialize(&mut self) -> TransportResult<bool> {
        self.transport.initialize()
    }

    fn shutdown(&mut self) -> TransportResult<bool> {
        self.transport.shutdown()
    }

    fn role(&self) -> TransportRole {
        self.transport.role()
    }

    fn capacity(&self) -> i64 {
        self.transport.capacity()
    }

    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        self.transport.open_peer(peer_id)
    }

    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        self.transport.close_peer(peer_id, reason)
    }

    fn peer_ids(&self) -> Vec<String> {
        self.transport.peer_ids()
    }

    fn peer_state(&self, peer_id: &str) -> Option<TransportPeerState> {
        self.transport.peer_state(peer_id)
    }

    fn request_offer(&mut self, peer_id: &str) -> TransportResult<bool> {
        self.transport.request_offer(peer_id)
    }

    fn accept_offer(&mut self, signal: &str) -> TransportResult<bool> {
        self.transport.accept_offer(signal)
    }

    fn accept_answer(&mut self, peer_id: &str, signal: &str) -> TransportResult<bool> {
        self.transport.accept_answer(peer_id, signal)
    }

    fn take_signal(&mut self, peer_id: &str) -> TransportResult<Option<String>> {
        self.transport.take_signal(peer_id)
    }

    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        self.transport.send(peer_id, channel, message)
    }

    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        self.transport.broadcast(channel, message)
    }

    /// `reverse` hands input arrivals over last-first, to prove that the
    /// driver's confirmed boundaries never depend on poll order. The
    /// **control** channel keeps its order in both modes: a WebRTC control
    /// channel is reliable *and ordered*, and reordering it would break a
    /// guarantee the real transport provides.
    fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage> {
        let budget = limit.unwrap_or(32).max(0) as usize;
        if self.order == FaultTransportPollOrder::Arrival {
            let take = budget.min(self.ready.len());
            return self.ready.drain(0..take).collect();
        }
        let mut control = Vec::new();
        let mut input = Vec::new();
        for entry in self.ready.drain(..) {
            if entry.channel == TransportChannel::Control {
                control.push(entry);
            } else {
                input.push(entry);
            }
        }
        let mut ordered = control;
        ordered.extend(input.into_iter().rev());
        let mut batch = Vec::new();
        for (index, entry) in ordered.into_iter().enumerate() {
            if index < budget {
                batch.push(entry);
            } else {
                self.ready.push(entry);
            }
        }
        batch
    }

    fn poll(&mut self) -> Option<TransportPeerMessage> {
        self.poll_batch(Some(1)).into_iter().next()
    }

    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        self.transport.poll_event()
    }

    fn state(&self) -> TransportState {
        self.transport.state()
    }

    fn diagnostics(&self) -> TransportStarDiagnostics {
        self.transport.diagnostics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct StubTransport {
        inbound: Vec<TransportPeerMessage>,
        sent: Vec<(String, TransportChannel, TransportMessage)>,
        events: Vec<TransportPeerEvent>,
    }

    impl StarTransportAdapter for StubTransport {
        fn initialize(&mut self) -> TransportResult<bool> {
            Ok(true)
        }
        fn shutdown(&mut self) -> TransportResult<bool> {
            Ok(true)
        }
        fn role(&self) -> TransportRole {
            TransportRole::Host
        }
        fn capacity(&self) -> i64 {
            8
        }
        fn open_peer(&mut self, _peer_id: &str) -> TransportResult<i64> {
            Ok(1)
        }
        fn close_peer(&mut self, _peer_id: &str, _reason: Option<&str>) -> TransportResult<bool> {
            Ok(true)
        }
        fn peer_ids(&self) -> Vec<String> {
            Vec::new()
        }
        fn peer_state(&self, _peer_id: &str) -> Option<TransportPeerState> {
            None
        }
        fn request_offer(&mut self, _peer_id: &str) -> TransportResult<bool> {
            Ok(true)
        }
        fn accept_offer(&mut self, _signal: &str) -> TransportResult<bool> {
            Ok(true)
        }
        fn accept_answer(&mut self, _peer_id: &str, _signal: &str) -> TransportResult<bool> {
            Ok(true)
        }
        fn take_signal(&mut self, _peer_id: &str) -> TransportResult<Option<String>> {
            Ok(None)
        }
        fn send(
            &mut self,
            peer_id: &str,
            channel: TransportChannel,
            message: TransportMessage,
        ) -> TransportResult<bool> {
            self.sent.push((peer_id.to_string(), channel, message));
            Ok(true)
        }
        fn broadcast(
            &mut self,
            _channel: TransportChannel,
            _message: TransportMessage,
        ) -> TransportResult<i64> {
            Ok(0)
        }
        fn poll(&mut self) -> Option<TransportPeerMessage> {
            if self.inbound.is_empty() {
                None
            } else {
                Some(self.inbound.remove(0))
            }
        }
        fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage> {
            let budget = limit.unwrap_or(32).max(0) as usize;
            let take = budget.min(self.inbound.len());
            self.inbound.drain(0..take).collect()
        }
        fn poll_event(&mut self) -> Option<TransportPeerEvent> {
            if self.events.is_empty() {
                None
            } else {
                Some(self.events.remove(0))
            }
        }
        fn state(&self) -> TransportState {
            TransportState::Connected
        }
        fn diagnostics(&self) -> TransportStarDiagnostics {
            TransportStarDiagnostics::default()
        }
    }

    fn message(seq: i64, tick: i64) -> TransportMessage {
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq,
            tick: Some(tick),
            payload: vec![0u8; 4],
        }
    }

    fn input_entry(peer_id: &str, seq: i64, tick: i64) -> TransportPeerMessage {
        TransportPeerMessage {
            peer_id: peer_id.to_string(),
            channel: TransportChannel::Input,
            message: message(seq, tick),
            arrival_seq: seq,
        }
    }

    fn clean_transport(legs: Vec<String>) -> FaultTransport {
        FaultTransport::new(FaultTransportOptions {
            transport: Box::new(StubTransport::default()),
            profile: NetworkProfileName::Clean,
            seed: 7.0,
            legs,
            poll_order: None,
            duplicate_control_every: None,
        })
    }

    #[test]
    fn clean_profile_delivers_every_input_envelope_on_the_tick_it_was_sent() {
        let mut transport = clean_transport(vec!["guest_1".to_string()]);
        let stub = StubTransport {
            inbound: vec![input_entry("guest_1", 0, 0)],
            ..Default::default()
        };
        transport.transport = Box::new(stub);
        transport.tick(1);
        let counters = transport.counters();
        assert_eq!(counters.scheduled, 1);
        assert_eq!(counters.delivered, 1);
        assert_eq!(counters.dropped, 0);
        assert_eq!(transport.poll_batch(None).len(), 1);
    }

    #[test]
    fn withhold_drops_every_envelope_in_the_window() {
        let mut transport = clean_transport(vec!["guest_1".to_string()]);
        transport.withhold(1, 1);
        transport.transport = Box::new(StubTransport {
            inbound: vec![input_entry("guest_1", 0, 0)],
            ..Default::default()
        });
        transport.tick(1);
        let counters = transport.counters();
        assert_eq!(counters.withheld, 1);
        assert_eq!(counters.scheduled, 0);
        assert!(transport.poll_batch(None).is_empty());
    }

    #[test]
    fn hold_releases_the_backlog_after_the_window_closes() {
        let mut transport = clean_transport(vec!["guest_1".to_string()]);
        transport.hold(1, 2);
        transport.transport = Box::new(StubTransport {
            inbound: vec![input_entry("guest_1", 0, 0)],
            ..Default::default()
        });
        transport.tick(1);
        assert_eq!(transport.counters().held, 1);
        assert!(transport.poll_batch(None).is_empty());
        transport.tick(2);
        assert!(transport.poll_batch(None).is_empty());
        transport.tick(3);
        // Scheduling happens on the tick the hold releases; delivery still
        // follows the clean profile's zero base delay, landing one tick later.
        assert_eq!(transport.counters().scheduled, 1);
        transport.tick(4);
        assert_eq!(transport.poll_batch(None).len(), 1);
    }

    #[test]
    fn an_envelope_from_an_undeclared_peer_is_delivered_and_counted_unmapped() {
        let mut transport = clean_transport(vec!["guest_1".to_string()]);
        transport.transport = Box::new(StubTransport {
            inbound: vec![input_entry("guest_9", 0, 0)],
            ..Default::default()
        });
        transport.tick(1);
        assert_eq!(transport.counters().unmapped, 1);
        assert_eq!(transport.poll_batch(None).len(), 1);
    }

    #[test]
    fn control_envelopes_are_never_impaired_and_can_be_declared_duplicated() {
        let mut transport = FaultTransport::new(FaultTransportOptions {
            transport: Box::new(StubTransport::default()),
            profile: NetworkProfileName::Clean,
            seed: 3.0,
            legs: vec!["guest_1".to_string()],
            poll_order: None,
            duplicate_control_every: Some(2),
        });
        let control = |seq: i64| TransportPeerMessage {
            peer_id: "guest_1".to_string(),
            channel: TransportChannel::Control,
            message: TransportMessage {
                version: 1,
                kind: TransportMessageType::Event,
                seq,
                tick: None,
                payload: Vec::new(),
            },
            arrival_seq: seq,
        };
        transport.transport = Box::new(StubTransport {
            inbound: vec![control(0), control(1)],
            ..Default::default()
        });
        transport.tick(1);
        // The second control envelope is the 2nd seen, so it is duplicated.
        assert_eq!(transport.counters().control_delivered, 2);
        assert_eq!(transport.counters().control_duplicated, 1);
        assert_eq!(transport.poll_batch(None).len(), 3);
    }

    #[test]
    fn reverse_poll_order_reverses_input_but_not_control() {
        let mut transport = FaultTransport::new(FaultTransportOptions {
            transport: Box::new(StubTransport::default()),
            profile: NetworkProfileName::Clean,
            seed: 11.0,
            legs: vec!["guest_1".to_string(), "guest_2".to_string()],
            poll_order: Some(FaultTransportPollOrder::Reverse),
            duplicate_control_every: None,
        });
        transport.transport = Box::new(StubTransport {
            inbound: vec![input_entry("guest_1", 0, 0), input_entry("guest_2", 1, 0)],
            ..Default::default()
        });
        transport.tick(1);
        let batch = transport.poll_batch(None);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].peer_id, "guest_2");
        assert_eq!(batch[1].peer_id, "guest_1");
    }

    #[test]
    #[should_panic(expected = "a fault transport clock only moves forward")]
    fn tick_must_strictly_advance() {
        let mut transport = clean_transport(vec!["guest_1".to_string()]);
        transport.tick(1);
        transport.tick(1);
    }

    #[test]
    #[should_panic(expected = "a fault transport leg is declared twice")]
    fn duplicate_legs_panic_at_construction() {
        clean_transport(vec!["guest_1".to_string(), "guest_1".to_string()]);
    }
}
