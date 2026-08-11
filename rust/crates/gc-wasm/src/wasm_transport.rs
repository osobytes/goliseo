//! A [`gc_netcode::fault_transport::StarTransportAdapter`] backed by a real
//! browser transport, via the same enqueue/drain discipline
//! [`crate::net_inbox`] documents.
//!
//! ## Why this is the shape `fake_star`/`fake_relay` already model
//!
//! `gc_netcode::match_driver::MatchDriver` polls a `Box<dyn
//! StarTransportAdapter>` internally, once per driver step
//! (`MatchDriver::advance` calls `poll_batch` through the injected
//! transport) — see that crate's `fake_star.rs`/`fake_relay.rs`, which
//! implement the identical trait as pure in-process fakes for testing. This
//! module is the third implementation: the same interface, backed by
//! whatever real channel the browser gives JS (a WebRTC data channel, a
//! `WebSocket`), instead of an in-process peer.
//!
//! Transport wire encode/decode and WebRTC signaling are TypeScript-owned
//! per ARCHITECTURE.md §1 — `packages/transport` already owns that, and
//! `fake_star.rs`/`fake_relay.rs` only restate it locally because *they*
//! need to round-trip real bytes between two in-process fakes. This module
//! has no such need: it hands JS structured [`TransportMessage`] fields
//! directly (peer id, channel, kind, sequence, tick, and the raw payload
//! bytes as a `Uint8Array`) and lets `packages/transport` do the actual wire
//! encoding and the actual send. Symmetrically, an arrival is decoded by
//! `packages/transport` on the way in and handed to
//! [`WasmStarTransport::enqueue_inbound`] already structured. This crate
//! never re-implements the transport contract's wire codec a third time.
//!
//! ## The seam itself
//!
//! [`WasmStarTransport::enqueue_inbound`] is the only network-facing entry
//! point, and it only ever pushes onto a [`crate::net_inbox::TickQueue`] —
//! see that module's doc for the full argument. `poll`/`poll_batch` (called
//! by `MatchDriver::advance`, once per driver step) are the only readers.
//! Nothing here gives JS a way to reach into a driver's simulation state
//! directly; the driver only ever sees what was queued *before* the
//! `advance()` call that triggers this poll.
//!
//! `send`/`broadcast` run the opposite direction: `MatchDriver` (or, for
//! control traffic, [`crate::coordinator_bridge`] via its own outbound
//! actions) calls them to hand this endpoint an outbound envelope, which is
//! appended to an outbound queue JS drains explicitly
//! ([`WasmStarTransport::drain_outbound`]) once per tick, after the
//! `advance()`/`tick()` call that produced it, and actually transmits over
//! the real channel using `packages/transport`.
//!
//! ## Peer lifecycle is JS-driven, not signaling-driven
//!
//! A real connection's signaling (ICE, SDP offer/answer) already has an
//! owner: `packages/transport`/`packages/online`, entirely in TypeScript,
//! established before a match (and this transport) exists. This adapter
//! therefore treats [`StarTransportAdapter::request_offer`]/
//! [`StarTransportAdapter::accept_offer`]/
//! [`StarTransportAdapter::accept_answer`]/[`StarTransportAdapter::take_signal`]
//! as out of its scope and returns a clear [`TransportErrorCode::RoleForbidden`]
//! for each — `MatchDriver` never calls them once a match is running (they
//! exist on the trait for the lobby-phase in-process fakes). What this
//! adapter *does* manage is the already-connected peer's data-plane
//! bookkeeping: [`WasmStarTransport::open_peer`]/`close_peer` allocate/
//! release a slot, and [`WasmStarTransport::set_peer_connected`] is how JS
//! reports that the real connection (established elsewhere) is up, once —
//! matching the moment `fake_star.rs`'s `link` marks both sides `Connected`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use gc_netcode::fault_transport::{
    StarTransportAdapter, TransportChannel, TransportChannelDiagnostics, TransportError,
    TransportErrorCode, TransportMessage, TransportPeerDiagnostics, TransportPeerEvent,
    TransportPeerEventKind, TransportPeerMessage, TransportPeerState, TransportResult,
    TransportRole, TransportStarDiagnostics, TransportState,
};
use indexmap::IndexMap;

use crate::net_inbox::TickQueue;

/// Mirrors `packages/transport/src/contract.ts`'s `MAX_PAYLOAD_BYTES`,
/// duplicated the same way `gc_netcode::match_driver`/`gc_netcode::fake_star`
/// duplicate it: a plain numeric bound, not logic that belongs to the
/// TypeScript-owned contract module.
const MAX_PAYLOAD_BYTES: usize = 1024;
/// Mirrors `transport_contract.MAX_GUESTS`.
const MAX_GUESTS: i64 = 7;
/// Mirrors `transport_contract.DEFAULT_QUEUE_LIMIT`. `pub(crate)`, not
/// private: [`crate::match_driver_bridge::MatchDriverBridge::new`]'s own doc
/// references this as the default its `queue_limit` parameter falls back to
/// when omitted.
pub(crate) const DEFAULT_QUEUE_LIMIT: i64 = 64;
/// Mirrors `transport_contract.DEFAULT_POLL_BATCH`.
const DEFAULT_POLL_BATCH: i64 = 32;

/// One outbound envelope this endpoint wants sent, queued for JS to actually
/// transmit. Not `TransportAddressedMessage` (that type names one link;
/// `broadcast` fans out to several) — this crate's own shape, one entry per
/// destination, matching what `drain_outbound` hands JS one row per real
/// send.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboundEnvelope {
    /// The destination link id.
    pub peer_id: String,
    /// Which channel to send on.
    pub channel: TransportChannel,
    /// The envelope itself.
    pub message: TransportMessage,
}

/// One channel's traffic counters on one peer link — `sent`/`received`/
/// `dropped_outbound`/`dropped_inbound`, tracked separately for `control`
/// and `input` because they are legitimately independent channels with
/// independent traffic. Mirrors the fields
/// `gc_netcode::fake_star::FakeStarChannel` tracks per channel; unlike that
/// in-process fake, this transport does not model a channel's own queue
/// depth or `bufferedAmount` — those live in the real browser data channel
/// (`packages/transport`), not in anything reachable from here, so
/// `diagnostics()` reports them as `0` for *both* channels rather than
/// inventing a value this adapter has no way to observe.
#[derive(Clone, Copy, Debug, Default)]
struct ChannelCounters {
    sent: i64,
    received: i64,
    dropped_outbound: i64,
    dropped_inbound: i64,
}

struct PeerLink {
    peer_id: String,
    slot: i64,
    state: TransportPeerState,
    /// The underlying ICE connection state, mirroring
    /// `gc_netcode::fake_star::FakeStarTransport`'s own peer field of the
    /// same name and the exact same vocabulary
    /// (`"new"`/`"connected"`/`"closed"`) — see this module's own doc for
    /// why this transport reproduces that vocabulary instead of inventing
    /// one. `fake_star.rs` also uses `"checking"` while its in-process
    /// signaling (`request_offer`/`accept_answer`) is pending, but this
    /// transport's own signaling methods are unconditionally
    /// `RoleForbidden` (see the module doc's "Peer lifecycle is JS-driven,
    /// not signaling-driven" section) — a real connection's ICE negotiation
    /// already happened, entirely outside this adapter, before
    /// `set_peer_connected` is ever called, so `"checking"` never applies
    /// here and is not reproduced.
    ice_state: String,
    control: ChannelCounters,
    input: ChannelCounters,
    last_error: Option<String>,
}

impl PeerLink {
    fn new(peer_id: String, slot: i64) -> Self {
        PeerLink {
            peer_id,
            slot,
            state: TransportPeerState::New,
            // Mirrors `fake_star::PeerLink::new`'s own `ice_state: "new"`.
            ice_state: "new".to_string(),
            control: ChannelCounters::default(),
            input: ChannelCounters::default(),
            last_error: None,
        }
    }

    /// The counters for `channel`, mirroring
    /// `gc_netcode::fake_star::Peer::channel_mut`.
    fn channel_mut(&mut self, channel: TransportChannel) -> &mut ChannelCounters {
        match channel {
            TransportChannel::Control => &mut self.control,
            TransportChannel::Input => &mut self.input,
        }
    }
}

struct Inner {
    role: TransportRole,
    state: TransportState,
    capacity: i64,
    queue_limit: i64,
    peers: IndexMap<String, PeerLink>,
    order: Vec<String>,
    inbound: TickQueue<TransportPeerMessage>,
    outbound: VecDeque<OutboundEnvelope>,
    events: TickQueue<TransportPeerEvent>,
    arrival_seq: IndexMap<String, i64>,
    sent: i64,
    received: i64,
    dropped_outbound: i64,
    dropped_inbound: i64,
    malformed: i64,
    last_error: Option<String>,
}

fn validate_channel_message(
    channel: TransportChannel,
    message: &TransportMessage,
) -> Result<(), (TransportErrorCode, String)> {
    if message.payload.len() > MAX_PAYLOAD_BYTES {
        return Err((
            TransportErrorCode::PayloadTooLarge,
            "transport message payload exceeds the byte limit".to_string(),
        ));
    }
    let carries = match channel {
        TransportChannel::Control => matches!(
            message.kind,
            gc_netcode::fault_transport::TransportMessageType::Event
                | gc_netcode::fault_transport::TransportMessageType::State
        ),
        TransportChannel::Input => {
            matches!(
                message.kind,
                gc_netcode::fault_transport::TransportMessageType::Input
            ) && message.tick.is_some()
        }
    };
    if !carries {
        return Err((
            TransportErrorCode::ChannelMismatch,
            "transport channel does not carry this message type".to_string(),
        ));
    }
    Ok(())
}

/// A real, browser-backed [`StarTransportAdapter`]. See the module doc.
///
/// Cheap `Clone`: state lives behind `Rc<RefCell<Inner>>`, mirroring
/// `gc_netcode::fake_star::FakeStarTransport`'s own handle pattern — one
/// clone is handed to a `MatchDriver` (as `Box<dyn StarTransportAdapter>`,
/// which takes ownership) while another stays with the caller to drive
/// [`WasmStarTransport::enqueue_inbound`]/[`WasmStarTransport::drain_outbound`],
/// so both sides observe the same queues.
#[derive(Clone)]
pub struct WasmStarTransport {
    inner: Rc<RefCell<Inner>>,
}

impl WasmStarTransport {
    /// Builds one endpoint, not yet initialized.
    #[must_use]
    pub fn new(role: TransportRole, max_guests: Option<i64>, queue_limit: Option<i64>) -> Self {
        let inner = Inner {
            role,
            state: TransportState::New,
            capacity: if role == TransportRole::Host {
                max_guests.unwrap_or(MAX_GUESTS)
            } else {
                1
            },
            queue_limit: queue_limit.unwrap_or(DEFAULT_QUEUE_LIMIT),
            peers: IndexMap::new(),
            order: Vec::new(),
            inbound: TickQueue::new(),
            outbound: VecDeque::new(),
            events: TickQueue::new(),
            arrival_seq: IndexMap::new(),
            sent: 0,
            received: 0,
            dropped_outbound: 0,
            dropped_inbound: 0,
            malformed: 0,
            last_error: None,
        };
        WasmStarTransport {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    /// Queues one arrived envelope. This is the *only* network-facing entry
    /// point — see the module doc and [`crate::net_inbox`]'s. Call this from
    /// the real transport's `onmessage` handler, at any time; it never
    /// touches anything but this endpoint's inbound queue, and never blocks.
    ///
    /// Returns `Err` (a `TransportError`, mirroring every other adapter
    /// operation in this crate) if `peer_id` names no open link, or if the
    /// envelope fails the same structural checks `send`/`broadcast` enforce
    /// on the way out — an unknown or malformed arrival is a legitimate,
    /// expected condition on a real network, never a panic.
    pub fn enqueue_inbound(
        &self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<()> {
        let mut inner = self.inner.borrow_mut();
        if !inner.peers.contains_key(peer_id) {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "wasm transport has no peer with that id",
            ));
        }
        if let Err((code, detail)) = validate_channel_message(channel, &message) {
            inner.malformed += 1;
            inner.last_error = Some(detail.clone());
            if let Some(peer) = inner.peers.get_mut(peer_id) {
                peer.last_error = Some(detail.clone());
            }
            return Err(TransportError::new(code, detail));
        }
        let queue_limit = inner.queue_limit;
        if inner.inbound.len() as i64 >= queue_limit {
            inner.dropped_inbound += 1;
            if let Some(peer) = inner.peers.get_mut(peer_id) {
                peer.channel_mut(channel).dropped_inbound += 1;
            }
            let detail = "wasm transport inbound queue is full".to_string();
            inner.last_error = Some(detail.clone());
            return Err(TransportError::new(TransportErrorCode::Overflow, detail));
        }
        let next_seq = inner.arrival_seq.get(peer_id).copied().unwrap_or(0) + 1;
        inner.arrival_seq.insert(peer_id.to_string(), next_seq);
        inner.received += 1;
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.channel_mut(channel).received += 1;
        }
        inner.inbound.enqueue(TransportPeerMessage {
            peer_id: peer_id.to_string(),
            channel,
            message,
            arrival_seq: next_seq,
        });
        Ok(())
    }

    /// Removes and returns every envelope currently queued to send, oldest
    /// first. Call once per tick, after the `advance()`/`tick()` call that
    /// may have produced outbound traffic, and actually transmit each one
    /// over the real channel (via `packages/transport`).
    #[must_use]
    pub fn drain_outbound(&self) -> Vec<OutboundEnvelope> {
        let mut inner = self.inner.borrow_mut();
        inner.outbound.drain(..).collect()
    }

    /// Reports that the real connection to `peer_id` (established elsewhere,
    /// by `packages/transport`) is up. Mirrors the moment
    /// `fake_star.rs`'s `link` marks both sides `Connected` *and* sets
    /// `ice_state = "connected"` on both peer links. A no-op if `peer_id` is
    /// not an open link.
    pub fn set_peer_connected(&self, peer_id: &str) {
        let mut inner = self.inner.borrow_mut();
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.ice_state = "connected".to_string();
        }
        set_peer_state(&mut inner, peer_id, TransportPeerState::Connected);
    }

    /// Reports that the real connection to `peer_id` dropped without an
    /// explicit [`StarTransportAdapter::close_peer`] call (e.g. an ICE
    /// failure). A no-op if `peer_id` is not an open link.
    pub fn set_peer_disconnected(&self, peer_id: &str, detail: &str) {
        let mut inner = self.inner.borrow_mut();
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.last_error = Some(detail.to_string());
            // `fake_star.rs`'s `close_peer` sets the *remote* peer's own
            // `ice_state` to `"closed"` for exactly this case (a link torn
            // down without that side calling `close_peer` itself) -- it has
            // no separate "disconnected" ICE string, and neither does this
            // transport.
            peer.ice_state = "closed".to_string();
        }
        set_peer_state(&mut inner, peer_id, TransportPeerState::Disconnected);
        push_event(
            &mut inner,
            TransportPeerEvent {
                kind: TransportPeerEventKind::PeerError,
                peer_id: Some(peer_id.to_string()),
                channel: None,
                state: None,
                code: Some(TransportErrorCode::Disconnected),
                message: Some(detail.to_string()),
            },
        );
    }
}

fn set_peer_state(inner: &mut Inner, peer_id: &str, state: TransportPeerState) {
    if let Some(peer) = inner.peers.get_mut(peer_id) {
        peer.state = state;
    }
    push_event(
        inner,
        TransportPeerEvent {
            kind: TransportPeerEventKind::PeerState,
            peer_id: Some(peer_id.to_string()),
            channel: None,
            state: Some(state),
            code: None,
            message: None,
        },
    );
}

fn push_event(inner: &mut Inner, event: TransportPeerEvent) {
    inner.events.enqueue(event);
}

impl StarTransportAdapter for WasmStarTransport {
    fn initialize(&mut self) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        inner.state = TransportState::Connected;
        push_event(
            &mut inner,
            TransportPeerEvent {
                kind: TransportPeerEventKind::StarState,
                peer_id: None,
                channel: None,
                state: None,
                code: None,
                message: None,
            },
        );
        Ok(true)
    }

    fn shutdown(&mut self) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        inner.peers.clear();
        inner.order.clear();
        inner.state = TransportState::Closed;
        push_event(
            &mut inner,
            TransportPeerEvent {
                kind: TransportPeerEventKind::StarState,
                peer_id: None,
                channel: None,
                state: None,
                code: None,
                message: None,
            },
        );
        Ok(true)
    }

    fn role(&self) -> TransportRole {
        self.inner.borrow().role
    }

    fn capacity(&self) -> i64 {
        self.inner.borrow().capacity
    }

    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        let mut inner = self.inner.borrow_mut();
        if inner.state != TransportState::Connected {
            return Err(TransportError::new(
                TransportErrorCode::NotInitialized,
                "wasm transport is not initialized",
            ));
        }
        if inner.peers.contains_key(peer_id) {
            return Err(TransportError::new(
                TransportErrorCode::DuplicatePeer,
                "wasm transport peer is already open",
            ));
        }
        if inner.order.len() as i64 >= inner.capacity {
            return Err(TransportError::new(
                TransportErrorCode::Capacity,
                "wasm transport is at peer capacity",
            ));
        }
        let slot = inner.order.len() as i64 + 1;
        inner.peers.insert(
            peer_id.to_string(),
            PeerLink::new(peer_id.to_string(), slot),
        );
        inner.order.push(peer_id.to_string());
        push_event(
            &mut inner,
            TransportPeerEvent {
                kind: TransportPeerEventKind::PeerState,
                peer_id: Some(peer_id.to_string()),
                channel: None,
                state: Some(TransportPeerState::New),
                code: None,
                message: None,
            },
        );
        Ok(slot)
    }

    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        if !inner.peers.contains_key(peer_id) {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "wasm transport has no peer with that id",
            ));
        }
        let detail = reason.unwrap_or("wasm transport peer closed").to_string();
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.last_error = Some(detail.clone());
            // Mirrors `fake_star::close_peer`'s own `peer.ice_state =
            // "closed"` on the closing side.
            peer.ice_state = "closed".to_string();
        }
        set_peer_state(&mut inner, peer_id, TransportPeerState::Closed);
        Ok(true)
    }

    fn peer_ids(&self) -> Vec<String> {
        self.inner.borrow().order.clone()
    }

    fn peer_state(&self, peer_id: &str) -> Option<TransportPeerState> {
        self.inner
            .borrow()
            .peers
            .get(peer_id)
            .map(|peer| peer.state)
    }

    fn request_offer(&mut self, _peer_id: &str) -> TransportResult<bool> {
        Err(TransportError::new(
            TransportErrorCode::RoleForbidden,
            "the wasm transport does not perform signaling; connect the peer with @gc/transport \
             before opening it here",
        ))
    }

    fn accept_offer(&mut self, _signal: &str) -> TransportResult<bool> {
        Err(TransportError::new(
            TransportErrorCode::RoleForbidden,
            "the wasm transport does not perform signaling; connect the peer with @gc/transport \
             before opening it here",
        ))
    }

    fn accept_answer(&mut self, _peer_id: &str, _signal: &str) -> TransportResult<bool> {
        Err(TransportError::new(
            TransportErrorCode::RoleForbidden,
            "the wasm transport does not perform signaling; connect the peer with @gc/transport \
             before opening it here",
        ))
    }

    fn take_signal(&mut self, _peer_id: &str) -> TransportResult<Option<String>> {
        Err(TransportError::new(
            TransportErrorCode::RoleForbidden,
            "the wasm transport does not perform signaling; connect the peer with @gc/transport \
             before opening it here",
        ))
    }

    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        let Some(peer) = inner.peers.get(peer_id) else {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "wasm transport has no peer with that id",
            ));
        };
        if peer.state != TransportPeerState::Connected {
            return Err(TransportError::new(
                TransportErrorCode::NotConnected,
                "wasm transport peer link is not connected",
            ));
        }
        if let Err((code, detail)) = validate_channel_message(channel, &message) {
            return Err(TransportError::new(code, detail));
        }
        inner.outbound.push_back(OutboundEnvelope {
            peer_id: peer_id.to_string(),
            channel,
            message,
        });
        inner.sent += 1;
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.channel_mut(channel).sent += 1;
        }
        Ok(true)
    }

    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        if let Err((code, detail)) = validate_channel_message(channel, &message) {
            return Err(TransportError::new(code, detail));
        }
        let mut inner = self.inner.borrow_mut();
        let targets: Vec<String> = inner
            .order
            .iter()
            .filter(|peer_id| {
                inner
                    .peers
                    .get(*peer_id)
                    .is_some_and(|peer| peer.state == TransportPeerState::Connected)
            })
            .cloned()
            .collect();
        for peer_id in &targets {
            inner.outbound.push_back(OutboundEnvelope {
                peer_id: peer_id.clone(),
                channel,
                message: message.clone(),
            });
            inner.sent += 1;
            if let Some(peer) = inner.peers.get_mut(peer_id) {
                peer.channel_mut(channel).sent += 1;
            }
        }
        Ok(targets.len() as i64)
    }

    fn poll(&mut self) -> Option<TransportPeerMessage> {
        self.inner
            .borrow_mut()
            .inbound
            .drain_up_to(1)
            .into_iter()
            .next()
    }

    fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage> {
        let budget = limit.unwrap_or(DEFAULT_POLL_BATCH).max(0) as usize;
        self.inner.borrow_mut().inbound.drain_up_to(budget)
    }

    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        self.inner
            .borrow_mut()
            .events
            .drain_up_to(1)
            .into_iter()
            .next()
    }

    fn state(&self) -> TransportState {
        self.inner.borrow().state
    }

    fn diagnostics(&self) -> TransportStarDiagnostics {
        let inner = self.inner.borrow();
        let peers = inner
            .order
            .iter()
            .map(|peer_id| {
                let peer = &inner.peers[peer_id];
                // `state` mirrors `peer.state` on both channels, the same
                // way `fake_star::set_peer_state` stamps every one of
                // `peer.channels`' entries with the peer's own state — this
                // transport has one real connection per peer, not an
                // independently-negotiated channel per direction, so both
                // channels always agree with the link as a whole.
                // `outbound_depth`/`inbound_depth`/`buffered_amount` stay
                // `0` for both channels: that queue/buffer state lives in
                // the real browser data channel (`packages/transport`),
                // which this adapter never reads, so `0` is "not observed
                // here", not a fabricated measurement.
                let control = TransportChannelDiagnostics {
                    state: Some(peer.state),
                    outbound_depth: 0,
                    inbound_depth: 0,
                    buffered_amount: 0,
                    sent: peer.control.sent,
                    received: peer.control.received,
                    dropped_outbound: peer.control.dropped_outbound,
                    dropped_inbound: peer.control.dropped_inbound,
                };
                let input = TransportChannelDiagnostics {
                    state: Some(peer.state),
                    outbound_depth: 0,
                    inbound_depth: 0,
                    buffered_amount: 0,
                    sent: peer.input.sent,
                    received: peer.input.received,
                    dropped_outbound: peer.input.dropped_outbound,
                    dropped_inbound: peer.input.dropped_inbound,
                };
                TransportPeerDiagnostics {
                    peer_id: peer.peer_id.clone(),
                    slot: peer.slot,
                    state: Some(peer.state),
                    ice_state: peer.ice_state.clone(),
                    control,
                    input,
                    sequence_gaps: 0,
                    backpressure: 0,
                    malformed: 0,
                    last_error: peer.last_error.clone(),
                }
            })
            .collect();
        TransportStarDiagnostics {
            role: Some(inner.role),
            state: Some(inner.state),
            capacity: inner.capacity,
            peer_count: inner.order.len() as i64,
            queue_limit: inner.queue_limit,
            buffered_amount_limit: 0,
            event_depth: inner.events.len() as i64,
            sent: inner.sent,
            received: inner.received,
            dropped_outbound: inner.dropped_outbound,
            dropped_inbound: inner.dropped_inbound,
            malformed: inner.malformed,
            unsupported_version: 0,
            overflow: 0,
            backpressure: 0,
            last_error: inner.last_error.clone(),
            peers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_netcode::fault_transport::TransportMessageType;

    fn connected_pair() -> (WasmStarTransport, WasmStarTransport) {
        let mut host = WasmStarTransport::new(TransportRole::Host, None, None);
        host.initialize().unwrap();
        let mut guest = WasmStarTransport::new(TransportRole::Guest, None, None);
        guest.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.set_peer_connected("guest_1");
        (host, guest)
    }

    fn input_message(tick: i64, payload: &[u8]) -> TransportMessage {
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq: tick,
            tick: Some(tick),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn enqueue_inbound_requires_a_known_peer() {
        let transport = WasmStarTransport::new(TransportRole::Host, None, None);
        let err = transport
            .enqueue_inbound("nobody", TransportChannel::Input, input_message(0, b"x"))
            .unwrap_err();
        assert_eq!(err.code, TransportErrorCode::UnknownPeer);
    }

    #[test]
    fn poll_batch_drains_exactly_what_was_enqueued_in_order() {
        let (mut host, _guest) = connected_pair();
        host.enqueue_inbound("guest_1", TransportChannel::Input, input_message(0, b"a"))
            .unwrap();
        host.enqueue_inbound("guest_1", TransportChannel::Input, input_message(1, b"b"))
            .unwrap();

        let batch = host.poll_batch(None);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].message.payload, b"a");
        assert_eq!(batch[1].message.payload, b"b");
        assert!(host.poll_batch(None).is_empty());
    }

    /// The seam's proof, restated against the real trait `MatchDriver`
    /// drives: an arrival queued *after* one `poll_batch` call (standing in
    /// for one driver step's transport poll) is not part of that step's
    /// batch, and is only visible on the *next* `poll_batch` call — never
    /// before. This is [`crate::net_inbox`]'s
    /// `a_message_enqueued_after_drain_is_invisible_until_the_next_drain`,
    /// exercised through the actual `StarTransportAdapter` impl a driver
    /// polls, not just the underlying queue in isolation.
    #[test]
    fn an_arrival_enqueued_after_one_tick_poll_is_invisible_until_the_next_tick_poll() {
        let (mut host, _guest) = connected_pair();

        host.enqueue_inbound(
            "guest_1",
            TransportChannel::Input,
            input_message(0, b"tick_0"),
        )
        .unwrap();
        let tick_0_batch = host.poll_batch(None);
        assert_eq!(tick_0_batch.len(), 1);
        assert_eq!(tick_0_batch[0].message.payload, b"tick_0");

        // Stands in for the network callback firing while tick 0's own
        // downstream processing (feeding `tick_0_batch` through
        // `MatchDriver::advance`) is still running.
        host.enqueue_inbound(
            "guest_1",
            TransportChannel::Input,
            input_message(1, b"tick_1"),
        )
        .unwrap();
        assert_eq!(
            tick_0_batch[0].message.payload, b"tick_0",
            "the already-returned tick batch must not gain a member"
        );

        let tick_1_batch = host.poll_batch(None);
        assert_eq!(tick_1_batch.len(), 1);
        assert_eq!(tick_1_batch[0].message.payload, b"tick_1");
    }

    #[test]
    fn send_queues_an_outbound_envelope_drained_once_by_drain_outbound() {
        let (mut host, _guest) = connected_pair();
        host.send(
            "guest_1",
            TransportChannel::Input,
            input_message(0, b"hello"),
        )
        .unwrap();

        let drained = host.drain_outbound();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].peer_id, "guest_1");
        assert_eq!(drained[0].message.payload, b"hello");
        assert!(host.drain_outbound().is_empty());
    }

    #[test]
    fn send_to_a_disconnected_peer_is_rejected_not_queued() {
        let mut host = WasmStarTransport::new(TransportRole::Host, None, None);
        host.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        // No `set_peer_connected`: the link is still `New`.
        let err = host
            .send("guest_1", TransportChannel::Input, input_message(0, b"x"))
            .unwrap_err();
        assert_eq!(err.code, TransportErrorCode::NotConnected);
        assert!(host.drain_outbound().is_empty());
    }

    #[test]
    fn broadcast_fans_out_to_every_connected_peer_only() {
        let mut host = WasmStarTransport::new(TransportRole::Host, None, None);
        host.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.open_peer("guest_2").unwrap();
        host.set_peer_connected("guest_1");
        // guest_2 stays unconnected.

        let delivered = host
            .broadcast(
                TransportChannel::Control,
                TransportMessage {
                    version: 1,
                    kind: TransportMessageType::Event,
                    seq: 0,
                    tick: None,
                    payload: b"hi".to_vec(),
                },
            )
            .unwrap();
        assert_eq!(delivered, 1);
        let drained = host.drain_outbound();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].peer_id, "guest_1");
    }

    #[test]
    fn a_channel_kind_mismatch_is_rejected_on_both_directions() {
        let (mut host, _guest) = connected_pair();
        let control_shaped = TransportMessage {
            version: 1,
            kind: TransportMessageType::Event,
            seq: 0,
            tick: None,
            payload: Vec::new(),
        };
        assert_eq!(
            host.send("guest_1", TransportChannel::Input, control_shaped.clone())
                .unwrap_err()
                .code,
            TransportErrorCode::ChannelMismatch
        );
        assert_eq!(
            host.enqueue_inbound("guest_1", TransportChannel::Input, control_shaped)
                .unwrap_err()
                .code,
            TransportErrorCode::ChannelMismatch
        );
    }

    #[test]
    fn cloned_handles_observe_the_same_queues() {
        let host = WasmStarTransport::new(TransportRole::Host, None, None);
        let mut for_driver = host.clone();
        for_driver.initialize().unwrap();
        for_driver.open_peer("guest_1").unwrap();
        host.set_peer_connected("guest_1");

        assert_eq!(
            for_driver.peer_state("guest_1"),
            Some(TransportPeerState::Connected),
            "a state change made through one clone must be visible through another"
        );
    }

    #[test]
    fn signaling_methods_are_refused_not_silently_accepted() {
        let mut transport = WasmStarTransport::new(TransportRole::Host, None, None);
        transport.initialize().unwrap();
        assert_eq!(
            transport.request_offer("guest_1").unwrap_err().code,
            TransportErrorCode::RoleForbidden
        );
        assert_eq!(
            transport.accept_offer("token").unwrap_err().code,
            TransportErrorCode::RoleForbidden
        );
        assert_eq!(
            transport
                .accept_answer("guest_1", "token")
                .unwrap_err()
                .code,
            TransportErrorCode::RoleForbidden
        );
        assert_eq!(
            transport.take_signal("guest_1").unwrap_err().code,
            TransportErrorCode::RoleForbidden
        );
    }

    #[test]
    fn ice_state_is_never_empty_once_a_peer_exists_and_tracks_its_lifecycle() {
        // Before this fix, `diagnostics()` always reported `ice_state:
        // String::new()` -- failing the export schema's "at least 1 byte"
        // invariant the moment any peer link existed. This proves the real
        // fix: `ice_state` mirrors `gc_netcode::fake_star`'s own vocabulary
        // ("new" -> "connected" -> "closed") at each corresponding lifecycle
        // event, and is never empty.
        let mut host = WasmStarTransport::new(TransportRole::Host, None, None);
        host.initialize().unwrap();
        host.open_peer("guest_1").unwrap();

        let new_ice_state = host.diagnostics().peers[0].ice_state.clone();
        assert_eq!(
            new_ice_state, "new",
            "a freshly opened peer link is 'new', never empty"
        );

        host.set_peer_connected("guest_1");
        let connected_ice_state = host.diagnostics().peers[0].ice_state.clone();
        assert_eq!(connected_ice_state, "connected");

        host.close_peer("guest_1", None).unwrap();
        let closed_ice_state = host.diagnostics().peers[0].ice_state.clone();
        assert_eq!(closed_ice_state, "closed");
    }

    #[test]
    fn control_and_input_channel_diagnostics_are_tracked_independently() {
        // Before this fix, `diagnostics()` handed back
        // `input: TransportChannelDiagnostics::default()` verbatim --
        // `state: None`, every counter zero -- no matter how much input
        // traffic actually crossed the link. This proves the real fix:
        // `control` and `input` each carry their own `sent`/`received`
        // counters, driven only by traffic that actually used that
        // channel, and neither is the struct's `Default`.
        let (mut host, _guest) = connected_pair();

        // Traffic on the input channel only: one outbound send.
        host.send(
            "guest_1",
            TransportChannel::Input,
            input_message(0, b"input_payload"),
        )
        .unwrap();

        // Traffic on the control channel only: one inbound arrival.
        host.enqueue_inbound(
            "guest_1",
            TransportChannel::Control,
            TransportMessage {
                version: 1,
                kind: TransportMessageType::Event,
                seq: 0,
                tick: None,
                payload: b"control_payload".to_vec(),
            },
        )
        .unwrap();

        let diagnostics = host.diagnostics();
        let peer = &diagnostics.peers[0];

        assert_eq!(peer.input.sent, 1, "the input send must count on input");
        assert_eq!(
            peer.input.received, 0,
            "no inbound traffic crossed the input channel"
        );
        assert_eq!(
            peer.control.sent, 0,
            "no outbound traffic crossed the control channel"
        );
        assert_eq!(
            peer.control.received, 1,
            "the control arrival must count on control"
        );

        // Neither channel is left at the struct `Default` the old code
        // returned unconditionally for `input`.
        assert_ne!(peer.input, TransportChannelDiagnostics::default());
        assert_ne!(peer.control, TransportChannelDiagnostics::default());
        assert!(peer.input.state.is_some());
        assert!(peer.control.state.is_some());
    }

    #[test]
    fn set_peer_disconnected_reports_ice_state_closed_like_fake_star_does() {
        // `gc_netcode::fake_star::close_peer` sets the *remote* (non-closing)
        // peer's own `ice_state` to `"closed"` too -- there is no separate
        // "disconnected" ICE string in that vocabulary. `set_peer_disconnected`
        // is this transport's equivalent of learning about an unexpected
        // remote teardown, so it must report the same value.
        let mut host = WasmStarTransport::new(TransportRole::Host, None, None);
        host.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.set_peer_connected("guest_1");

        host.set_peer_disconnected("guest_1", "ice failure");
        let ice_state = host.diagnostics().peers[0].ice_state.clone();
        assert_eq!(ice_state, "closed");
    }
}
