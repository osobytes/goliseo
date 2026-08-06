//! Port of `game/transport/fake_star.lua`.
//!
//! `game/transport/**` is TypeScript-owned (`v2/README.md` §2), so this is
//! not "the" Rust port of that file — there is no Rust home for it in the
//! ordinary sense. It exists so `game/online/match_driver_fixture.lua`'s
//! `fixture.session` (this crate's [`crate::match_driver_fixture::session`])
//! has a real star transport to build a live host+guest session against: a
//! Rust in-process fake, faithful to the Lua original's polled/queued
//! shape — one host endpoint, up to seven independently addressed guest
//! links, a reliable ordered control channel and a lossy unordered input
//! channel per link, bounded queues, simulated `bufferedAmount`
//! backpressure, typed events, and deterministic poll batching. It owns no
//! simulation authority and never inspects a payload, exactly like the Lua.
//!
//! # Object references, in Rust
//!
//! The Lua original links two endpoints by storing a direct table reference
//! on each side (`peer.link = guest`), and `_flush`/`pump` walk that
//! reference to call methods on the other endpoint directly. Rust has no
//! shared-mutable-reference equivalent, so each endpoint's state lives
//! behind `Rc<RefCell<Inner>>`; a [`FakeStarTransport`] is a cheap `Clone`
//! handle onto that state (mirroring a Lua table's reference semantics), and
//! [`FakeStarTransport::link`] stores a clone of the *other* endpoint's
//! `Rc<RefCell<Inner>>` on each side's peer record, exactly where the Lua
//! stores the other object.
//!
//! `game/transport/contract.lua`'s message-shape validation and wire
//! encoding are restated locally (never imported: this crate has no Rust
//! home for that file either — see `crate::fault_transport`'s module doc for
//! the identical situation), because a *transport* — unlike
//! `fault_transport.rs`'s decorator, which forwards `send`/`broadcast`
//! verbatim — is the one place that logic has to actually run.

use crate::fault_transport::{
    StarTransportAdapter, TransportAddressedMessage, TransportChannel, TransportChannelDiagnostics,
    TransportError, TransportErrorCode, TransportMessage, TransportMessageType,
    TransportPeerDiagnostics, TransportPeerEvent, TransportPeerEventKind, TransportPeerMessage,
    TransportPeerState, TransportResult, TransportRole, TransportStarDiagnostics, TransportState,
};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// `game/transport/contract.lua`'s bounds and wire logic, restated (see
// module doc).
// ---------------------------------------------------------------------------

/// Mirrors `contract.VERSION`.
const VERSION: i64 = 1;
/// Mirrors `contract.DEFAULT_QUEUE_LIMIT`.
const DEFAULT_QUEUE_LIMIT: i64 = 64;
/// Mirrors `contract.MAX_QUEUE_LIMIT`.
const MAX_QUEUE_LIMIT: i64 = 256;
/// Mirrors `contract.MAX_PAYLOAD_BYTES`.
const MAX_PAYLOAD_BYTES: usize = 1024;
/// Mirrors `contract.MAX_GUESTS`.
const MAX_GUESTS: i64 = 7;
/// Mirrors `contract.HOST_PEER_ID`.
pub const HOST_PEER_ID: &str = "host";
/// Mirrors `contract.MAX_PEER_ID_BYTES`.
const MAX_PEER_ID_BYTES: usize = 128;
/// Mirrors `contract.DEFAULT_BUFFERED_AMOUNT_LIMIT`.
const DEFAULT_BUFFERED_AMOUNT_LIMIT: i64 = 65536;
/// Mirrors `contract.MAX_BUFFERED_AMOUNT_LIMIT`.
const MAX_BUFFERED_AMOUNT_LIMIT: i64 = 1_048_576;
/// Mirrors `contract.DEFAULT_POLL_BATCH`.
const DEFAULT_POLL_BATCH: i64 = 32;
/// Mirrors `contract.CHANNEL_ORDER`, fixing the deterministic drain order.
const CHANNEL_ORDER: [TransportChannel; 2] = [TransportChannel::Control, TransportChannel::Input];

fn channel_index(channel: TransportChannel) -> usize {
    match channel {
        TransportChannel::Control => 0,
        TransportChannel::Input => 1,
    }
}

fn channel_label(channel: TransportChannel) -> &'static str {
    match channel {
        TransportChannel::Control => "control",
        TransportChannel::Input => "input",
    }
}

fn message_type_label(kind: TransportMessageType) -> &'static str {
    match kind {
        TransportMessageType::Input => "input",
        TransportMessageType::Event => "event",
        TransportMessageType::State => "state",
    }
}

/// Mirrors `contract.validate_peer_id`.
fn validate_peer_id(peer_id: &str) -> std::result::Result<(), (TransportErrorCode, String)> {
    if peer_id.is_empty() {
        return Err((
            TransportErrorCode::Malformed,
            "transport peer id must be a non-empty string".to_string(),
        ));
    }
    if peer_id.len() > MAX_PEER_ID_BYTES {
        return Err((
            TransportErrorCode::Malformed,
            "transport peer id exceeds the byte limit".to_string(),
        ));
    }
    let bytes = peer_id.as_bytes();
    let first_ok = bytes[0].is_ascii_alphanumeric();
    let rest_ok = bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !first_ok || !rest_ok {
        return Err((
            TransportErrorCode::Malformed,
            "transport peer id contains unsupported characters".to_string(),
        ));
    }
    Ok(())
}

/// Mirrors `contract.validate` (the structural half; channel pairing is
/// [`validate_channel_message`]).
fn validate_message(
    message: &TransportMessage,
) -> std::result::Result<(), (TransportErrorCode, String)> {
    if message.version != VERSION {
        return Err((
            TransportErrorCode::UnsupportedVersion,
            "unsupported transport message version".to_string(),
        ));
    }
    if message.seq < 0 {
        return Err((
            TransportErrorCode::Malformed,
            "transport message seq must be a non-negative integer".to_string(),
        ));
    }
    match message.kind {
        TransportMessageType::Input => {
            if !matches!(message.tick, Some(t) if t >= 0) {
                return Err((
                    TransportErrorCode::Malformed,
                    "input message tick must be a non-negative integer".to_string(),
                ));
            }
        }
        _ => {
            if matches!(message.tick, Some(t) if t < 0) {
                return Err((
                    TransportErrorCode::Malformed,
                    "transport message tick must be a non-negative integer".to_string(),
                ));
            }
        }
    }
    if message.payload.len() > MAX_PAYLOAD_BYTES {
        return Err((
            TransportErrorCode::PayloadTooLarge,
            "transport message payload exceeds the byte limit".to_string(),
        ));
    }
    Ok(())
}

/// Mirrors `contract.validate_channel_message`: enforces the channel/type
/// pairing before anything reaches a data channel.
fn validate_channel_message(
    channel: TransportChannel,
    message: &TransportMessage,
) -> std::result::Result<(), (TransportErrorCode, String)> {
    validate_message(message)?;
    let carries = match channel {
        TransportChannel::Control => {
            matches!(
                message.kind,
                TransportMessageType::Event | TransportMessageType::State
            )
        }
        TransportChannel::Input => matches!(message.kind, TransportMessageType::Input),
    };
    if !carries {
        return Err((
            TransportErrorCode::ChannelMismatch,
            format!(
                "transport {} channel does not carry {} messages",
                channel_label(channel),
                message_type_label(message.kind)
            ),
        ));
    }
    Ok(())
}

fn escape(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            out.push(byte);
        } else {
            out.extend_from_slice(format!("%{byte:02X}").as_bytes());
        }
    }
    out
}

/// Mirrors `contract.encode`: the canonical wire bytes for one envelope.
/// Only used here to size `bufferedAmount`/backpressure and dropped-byte
/// counters, exactly as the Lua original does; no caller decodes this wire.
fn encode(message: &TransportMessage) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(message.version.to_string().as_bytes());
    out.push(b'|');
    out.extend_from_slice(&escape(message_type_label(message.kind).as_bytes()));
    out.push(b'|');
    out.extend_from_slice(message.seq.to_string().as_bytes());
    out.push(b'|');
    if let Some(tick) = message.tick {
        out.extend_from_slice(tick.to_string().as_bytes());
    }
    out.push(b'|');
    out.extend_from_slice(&escape(&message.payload));
    out
}

// ---------------------------------------------------------------------------
// `fake_star.lua` itself.
// ---------------------------------------------------------------------------

/// Construction options for [`FakeStarTransport::new`]. Mirrors
/// `FakeStarTransportOptions`.
#[derive(Clone)]
pub struct FakeStarTransportOptions {
    /// Which side of the star this endpoint is.
    pub role: TransportRole,
    /// Guest link identity; ignored for a host.
    pub peer_id: Option<String>,
    /// Per-channel bounded queue depth.
    pub queue_limit: Option<i64>,
    /// Host-only: maximum simultaneous guest links.
    pub max_guests: Option<i64>,
    /// Simulated `bufferedAmount` send-buffer ceiling.
    pub buffered_amount_limit: Option<i64>,
    /// Share one across the endpoints of a single logical star. `None`
    /// mints a private one, so sharing is an explicit, greppable decision.
    pub rendezvous: Option<FakeStarRendezvous>,
}

impl Default for FakeStarTransportOptions {
    fn default() -> Self {
        FakeStarTransportOptions {
            role: TransportRole::Host,
            peer_id: None,
            queue_limit: None,
            max_guests: None,
            buffered_amount_limit: None,
            rendezvous: None,
        }
    }
}

struct FakeStarChannel {
    state: TransportPeerState,
    outbound: VecDeque<TransportAddressedMessage>,
    inbound: VecDeque<TransportPeerMessage>,
    backpressured: bool,
    buffered_amount: i64,
    sent: i64,
    received: i64,
    dropped_outbound: i64,
    dropped_inbound: i64,
    last_seq: Option<i64>,
}

impl FakeStarChannel {
    fn new() -> Self {
        FakeStarChannel {
            state: TransportPeerState::Connecting,
            outbound: VecDeque::new(),
            inbound: VecDeque::new(),
            backpressured: false,
            buffered_amount: 0,
            sent: 0,
            received: 0,
            dropped_outbound: 0,
            dropped_inbound: 0,
            last_seq: None,
        }
    }
}

struct Peer {
    peer_id: String,
    slot: i64,
    state: TransportPeerState,
    ice_state: String,
    channels: [FakeStarChannel; 2],
    arrival_seq: i64,
    signal: Option<String>,
    sequence_gaps: i64,
    backpressure: i64,
    malformed: i64,
    last_error: Option<String>,
    link: Option<Rc<RefCell<Inner>>>,
    link_peer_id: Option<String>,
}

impl Peer {
    fn new(peer_id: String, slot: i64) -> Self {
        Peer {
            peer_id,
            slot,
            state: TransportPeerState::Connecting,
            ice_state: "new".to_string(),
            channels: [FakeStarChannel::new(), FakeStarChannel::new()],
            arrival_seq: 0,
            signal: None,
            sequence_gaps: 0,
            backpressure: 0,
            malformed: 0,
            last_error: None,
            link: None,
            link_peer_id: None,
        }
    }

    fn channel(&self, channel: TransportChannel) -> &FakeStarChannel {
        &self.channels[channel_index(channel)]
    }

    fn channel_mut(&mut self, channel: TransportChannel) -> &mut FakeStarChannel {
        &mut self.channels[channel_index(channel)]
    }
}

struct Offer {
    host: Rc<RefCell<Inner>>,
    peer_id: String,
}

/// In-process signaling rendezvous, scoped to one logical star. See
/// `fake_star.lua`'s module doc: an endpoint constructed without one gets a
/// private rendezvous, so sharing is an explicit, greppable decision.
struct RendezvousInner {
    offers: IndexMap<String, Offer>,
    answers: IndexMap<String, Rc<RefCell<Inner>>>,
    sequence: i64,
}

/// A shared, cloneable handle onto one logical star's signaling rendezvous.
/// Mirrors `FakeStarRendezvous`.
#[derive(Clone)]
pub struct FakeStarRendezvous(Rc<RefCell<RendezvousInner>>);

impl FakeStarRendezvous {
    /// Mints a fresh, empty rendezvous.
    #[must_use]
    pub fn new() -> Self {
        FakeStarRendezvous(Rc::new(RefCell::new(RendezvousInner {
            offers: IndexMap::new(),
            answers: IndexMap::new(),
            sequence: 0,
        })))
    }
}

impl Default for FakeStarRendezvous {
    fn default() -> Self {
        FakeStarRendezvous::new()
    }
}

struct Inner {
    role: TransportRole,
    peer_id: String,
    state: TransportState,
    queue_limit: i64,
    event_limit: i64,
    max_guests: i64,
    buffered_amount_limit: i64,
    rendezvous: FakeStarRendezvous,
    peers: IndexMap<String, Peer>,
    order: Vec<String>,
    events: VecDeque<TransportPeerEvent>,
    cursor: i64,
    sent: i64,
    received: i64,
    dropped_outbound: i64,
    dropped_inbound: i64,
    malformed: i64,
    unsupported_version: i64,
    overflow: i64,
    backpressure: i64,
    uplink_bytes: i64,
    downlink_bytes: i64,
    input_uplink_bytes: i64,
    input_downlink_bytes: i64,
    uplink_units: i64,
    downlink_frames: i64,
    last_error: Option<String>,
}

/// Pure in-process host-star transport. See the module doc.
#[derive(Clone)]
pub struct FakeStarTransport {
    inner: Rc<RefCell<Inner>>,
}

fn push_event(inner: &mut Inner, event: TransportPeerEvent) {
    if inner.events.len() as i64 >= inner.event_limit {
        inner.events.pop_front();
        inner.overflow += 1;
        inner.last_error = Some("fake star transport event queue is full".to_string());
    }
    inner.events.push_back(event);
}

fn record_error(
    inner: &mut Inner,
    code: TransportErrorCode,
    message: &str,
    peer_id: Option<&str>,
    channel: Option<TransportChannel>,
) {
    inner.last_error = Some(message.to_string());
    match code {
        TransportErrorCode::Malformed
        | TransportErrorCode::PayloadTooLarge
        | TransportErrorCode::ChannelMismatch => {
            inner.malformed += 1;
            if let Some(peer) = peer_id.and_then(|id| inner.peers.get_mut(id)) {
                peer.malformed += 1;
            }
        }
        TransportErrorCode::UnsupportedVersion => inner.unsupported_version += 1,
        TransportErrorCode::Overflow => inner.overflow += 1,
        TransportErrorCode::Backpressure => {
            inner.backpressure += 1;
            if let Some(peer) = peer_id.and_then(|id| inner.peers.get_mut(id)) {
                peer.backpressure += 1;
            }
        }
        _ => {}
    }
    if let Some(peer_id) = peer_id {
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.last_error = Some(message.to_string());
        }
        push_event(
            inner,
            TransportPeerEvent {
                kind: TransportPeerEventKind::PeerError,
                peer_id: Some(peer_id.to_string()),
                channel,
                state: None,
                code: Some(code),
                message: Some(message.to_string()),
            },
        );
    } else {
        push_event(
            inner,
            TransportPeerEvent {
                kind: TransportPeerEventKind::StarError,
                peer_id: None,
                channel: None,
                state: None,
                code: Some(code),
                message: Some(message.to_string()),
            },
        );
    }
}

fn set_peer_state(inner: &mut Inner, peer_id: &str, state: TransportPeerState) {
    if let Some(peer) = inner.peers.get_mut(peer_id) {
        peer.state = state;
        for channel in peer.channels.iter_mut() {
            channel.state = state;
        }
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

fn set_state(inner: &mut Inner, state: TransportState) {
    inner.state = state;
    push_event(
        inner,
        TransportPeerEvent {
            kind: TransportPeerEventKind::StarState,
            peer_id: None,
            channel: None,
            state: None,
            code: None,
            message: None,
        },
    );
}

fn require_connected(inner: &Inner) -> TransportResult<()> {
    match inner.state {
        TransportState::New => Err(TransportError::new(
            TransportErrorCode::NotInitialized,
            "star transport is not initialized",
        )),
        TransportState::Closed => Err(TransportError::new(
            TransportErrorCode::Closed,
            "star transport is closed",
        )),
        TransportState::Connected => Ok(()),
        _ => Err(TransportError::new(
            TransportErrorCode::NotConnected,
            "star transport is not connected",
        )),
    }
}

fn add_peer(inner: &mut Inner, peer_id: &str) -> i64 {
    let slot = inner.order.len() as i64 + 1;
    inner
        .peers
        .insert(peer_id.to_string(), Peer::new(peer_id.to_string(), slot));
    inner.order.push(peer_id.to_string());
    push_event(
        inner,
        TransportPeerEvent {
            kind: TransportPeerEventKind::PeerState,
            peer_id: Some(peer_id.to_string()),
            channel: None,
            state: Some(TransportPeerState::Connecting),
            code: None,
            message: None,
        },
    );
    slot
}

fn drop_peer_queues(inner: &mut Inner, peer_id: &str) {
    let Some(peer) = inner.peers.get_mut(peer_id) else {
        return;
    };
    let mut dropped_out_total = 0i64;
    let mut dropped_in_total = 0i64;
    for channel in peer.channels.iter_mut() {
        let out = channel.outbound.len() as i64;
        let inb = channel.inbound.len() as i64;
        dropped_out_total += out;
        dropped_in_total += inb;
        channel.dropped_outbound += out;
        channel.dropped_inbound += inb;
        channel.outbound.clear();
        channel.inbound.clear();
        channel.buffered_amount = 0;
    }
    inner.dropped_outbound += dropped_out_total;
    inner.dropped_inbound += dropped_in_total;
}

fn enqueue(
    inner: &mut Inner,
    peer_id: &str,
    channel: TransportChannel,
    message: TransportMessage,
) -> TransportResult<bool> {
    if let Err((code, message)) = validate_channel_message(channel, &message) {
        record_error(inner, code, &message, Some(peer_id), None);
        return Err(TransportError::new(code, message));
    }
    let queue_limit = inner.queue_limit;
    let (peer_state, link_peer_id, own_peer_id) = {
        let peer = inner.peers.get(peer_id).expect("peer exists for enqueue");
        (peer.state, peer.link_peer_id.clone(), peer.peer_id.clone())
    };
    if peer_state != TransportPeerState::Connected {
        record_error(
            inner,
            TransportErrorCode::NotConnected,
            "star peer link is not connected",
            Some(peer_id),
            Some(channel),
        );
        return Err(TransportError::new(
            TransportErrorCode::NotConnected,
            "star peer link is not connected",
        ));
    }
    let wire = encode(&message);
    let wire_len = wire.len() as i64;
    let outbound_len = inner
        .peers
        .get(peer_id)
        .expect("peer exists")
        .channel(channel)
        .outbound
        .len() as i64;
    if outbound_len >= queue_limit {
        let peer = inner.peers.get_mut(peer_id).expect("peer exists");
        peer.channel_mut(channel).dropped_outbound += 1;
        inner.dropped_outbound += 1;
        record_error(
            inner,
            TransportErrorCode::Overflow,
            "star peer outbound queue is full",
            Some(peer_id),
            Some(channel),
        );
        return Err(TransportError::new(
            TransportErrorCode::Overflow,
            "star peer outbound queue is full",
        ));
    }
    let addressed = TransportAddressedMessage {
        peer_id: link_peer_id.unwrap_or(own_peer_id),
        channel,
        message: message.clone(),
    };
    let peer = inner.peers.get_mut(peer_id).expect("peer exists");
    let ch = peer.channel_mut(channel);
    ch.outbound.push_back(addressed);
    ch.buffered_amount += wire_len;
    ch.sent += 1;
    inner.sent += 1;
    inner.uplink_units += 1;
    inner.uplink_bytes += wire_len;
    if channel == TransportChannel::Input {
        inner.input_uplink_bytes += wire_len;
    }
    Ok(true)
}

fn receive_on(
    inner: &mut Inner,
    peer_id: &str,
    channel: TransportChannel,
    addressed: &TransportAddressedMessage,
    wire_bytes: i64,
) {
    inner.downlink_frames += 1;
    inner.downlink_bytes += wire_bytes;
    if channel == TransportChannel::Input {
        inner.input_downlink_bytes += wire_bytes;
    }
    let queue_limit = inner.queue_limit;
    let outbound_full = {
        let peer = inner.peers.get(peer_id).expect("receiving peer exists");
        peer.channel(channel).inbound.len() as i64 >= queue_limit
    };
    if outbound_full {
        let peer = inner.peers.get_mut(peer_id).expect("receiving peer exists");
        peer.channel_mut(channel).dropped_inbound += 1;
        inner.dropped_inbound += 1;
        record_error(
            inner,
            TransportErrorCode::Overflow,
            "star peer inbound queue is full",
            Some(peer_id),
            Some(channel),
        );
        return;
    }
    let seq = addressed.message.seq;
    let peer = inner.peers.get_mut(peer_id).expect("receiving peer exists");
    let recv_peer_id = peer.peer_id.clone();
    let ch = peer.channel_mut(channel);
    if let Some(last_seq) = ch.last_seq
        && seq > last_seq + 1
    {
        peer.sequence_gaps += seq - last_seq - 1;
    }
    let ch = peer.channel_mut(channel);
    if ch.last_seq.is_none_or(|last| seq > last) {
        ch.last_seq = Some(seq);
    }
    ch.inbound.push_back(TransportPeerMessage {
        peer_id: recv_peer_id,
        channel,
        message: addressed.message.clone(),
        arrival_seq: 0,
    });
}

/// Drains each channel of one endpoint until its send budget is exhausted.
/// Mirrors `_flush`.
fn flush(endpoint: &Rc<RefCell<Inner>>) {
    let order = endpoint.borrow().order.clone();
    for peer_id in &order {
        for &channel in &CHANNEL_ORDER {
            let buffered_amount_limit = endpoint.borrow().buffered_amount_limit;
            let mut budget = buffered_amount_limit;
            loop {
                let (link, link_peer_id, peer_connected, next) = {
                    let inner = endpoint.borrow();
                    let Some(peer) = inner.peers.get(peer_id) else {
                        break;
                    };
                    let ch = peer.channel(channel);
                    let Some(addressed) = ch.outbound.front().cloned() else {
                        break;
                    };
                    (
                        peer.link.clone(),
                        peer.link_peer_id.clone(),
                        peer.state == TransportPeerState::Connected,
                        addressed,
                    )
                };
                let size = encode(&next.message).len() as i64;
                if size > budget {
                    let mut inner = endpoint.borrow_mut();
                    let already = inner
                        .peers
                        .get(peer_id)
                        .expect("peer exists")
                        .channel(channel)
                        .backpressured;
                    if !already {
                        if let Some(peer) = inner.peers.get_mut(peer_id) {
                            peer.channel_mut(channel).backpressured = true;
                        }
                        record_error(
                            &mut inner,
                            TransportErrorCode::Backpressure,
                            "star peer channel send buffer is full",
                            Some(peer_id),
                            Some(channel),
                        );
                    }
                    break;
                }
                {
                    let mut inner = endpoint.borrow_mut();
                    let peer = inner.peers.get_mut(peer_id).expect("peer exists");
                    let ch = peer.channel_mut(channel);
                    ch.outbound.pop_front();
                    ch.backpressured = false;
                    ch.buffered_amount = (ch.buffered_amount - size).max(0);
                }
                budget -= size;
                if let (Some(link), Some(link_peer_id)) = (&link, &link_peer_id)
                    && peer_connected
                {
                    let remote_has_peer = link.borrow().peers.contains_key(link_peer_id);
                    if remote_has_peer {
                        let mut remote = link.borrow_mut();
                        receive_on(&mut remote, link_peer_id, channel, &next, size);
                        continue;
                    }
                }
                let mut inner = endpoint.borrow_mut();
                if let Some(peer) = inner.peers.get_mut(peer_id) {
                    peer.channel_mut(channel).dropped_outbound += 1;
                }
                inner.dropped_outbound += 1;
            }
        }
    }
}

fn take(inner: &mut Inner) -> Option<TransportPeerMessage> {
    let slots = inner.order.len() as i64;
    let channels = CHANNEL_ORDER.len() as i64;
    if slots == 0 {
        return None;
    }
    for _ in 0..(slots * channels) {
        let peer_index = (inner.cursor / channels) % slots;
        let channel_idx = inner.cursor % channels;
        inner.cursor = (inner.cursor + 1) % (slots * channels);
        let peer_id = inner.order[peer_index as usize].clone();
        let channel = CHANNEL_ORDER[channel_idx as usize];
        let has_entry = inner
            .peers
            .get(&peer_id)
            .is_some_and(|peer| !peer.channel(channel).inbound.is_empty());
        if has_entry {
            let peer = inner.peers.get_mut(&peer_id).expect("peer exists");
            let mut entry = peer
                .channel_mut(channel)
                .inbound
                .pop_front()
                .expect("checked non-empty above");
            let ch = peer.channel_mut(channel);
            ch.received += 1;
            inner.received += 1;
            let peer = inner.peers.get_mut(&peer_id).expect("peer exists");
            peer.arrival_seq += 1;
            entry.arrival_seq = peer.arrival_seq;
            return Some(entry);
        }
    }
    None
}

impl FakeStarTransport {
    /// Mints a fresh, empty rendezvous, shareable across the endpoints of
    /// one logical star. Mirrors `FakeStarTransport.new_rendezvous`.
    #[must_use]
    pub fn new_rendezvous() -> FakeStarRendezvous {
        FakeStarRendezvous::new()
    }

    /// Builds one endpoint. Mirrors `FakeStarTransport.new`.
    ///
    /// # Panics
    ///
    /// Panics (mirroring the Lua `assert`s, all programmer errors per
    /// AGENTS.md §7) if `queue_limit`, `max_guests`, or
    /// `buffered_amount_limit` are outside their supported ranges, or if
    /// `peer_id` is not a valid transport peer id.
    #[must_use]
    pub fn new(options: FakeStarTransportOptions) -> Self {
        let queue_limit = options.queue_limit.unwrap_or(DEFAULT_QUEUE_LIMIT);
        assert!(
            queue_limit > 0 && queue_limit <= MAX_QUEUE_LIMIT,
            "fake star transport queue_limit is outside the supported range"
        );
        let max_guests = options.max_guests.unwrap_or(MAX_GUESTS);
        assert!(
            max_guests > 0 && max_guests <= MAX_GUESTS,
            "fake star transport max_guests is outside the supported range"
        );
        let buffered_amount_limit = options
            .buffered_amount_limit
            .unwrap_or(DEFAULT_BUFFERED_AMOUNT_LIMIT);
        assert!(
            buffered_amount_limit > 0 && buffered_amount_limit <= MAX_BUFFERED_AMOUNT_LIMIT,
            "fake star transport buffered_amount_limit is outside the supported range"
        );
        let peer_id = options.peer_id.unwrap_or_else(|| "guest".to_string());
        validate_peer_id(&peer_id).expect("fake star transport peer_id must be valid");
        let role = options.role;
        let inner = Inner {
            role,
            peer_id,
            state: TransportState::New,
            queue_limit,
            event_limit: queue_limit.max(2),
            max_guests: if role == TransportRole::Host {
                max_guests
            } else {
                1
            },
            buffered_amount_limit,
            rendezvous: options.rendezvous.unwrap_or_default(),
            peers: IndexMap::new(),
            order: Vec::new(),
            events: VecDeque::new(),
            cursor: 0,
            sent: 0,
            received: 0,
            dropped_outbound: 0,
            dropped_inbound: 0,
            malformed: 0,
            unsupported_version: 0,
            overflow: 0,
            backpressure: 0,
            uplink_bytes: 0,
            downlink_bytes: 0,
            input_uplink_bytes: 0,
            input_downlink_bytes: 0,
            uplink_units: 0,
            downlink_frames: 0,
            last_error: None,
        };
        FakeStarTransport {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    /// Test seam: joins a guest endpoint to an already-opened host slot. The
    /// browser adapter reaches the same state through manual offer/answer
    /// exchange. Mirrors `FakeStarTransport:link`.
    pub fn link(&self, guest: &FakeStarTransport) -> TransportResult<bool> {
        let self_role = self.inner.borrow().role;
        let guest_role = guest.inner.borrow().role;
        if self_role != TransportRole::Host || guest_role != TransportRole::Guest {
            return Err(TransportError::new(
                TransportErrorCode::RoleForbidden,
                "link joins one host endpoint to one guest endpoint",
            ));
        }
        let guest_peer_id = guest.inner.borrow().peer_id.clone();
        if !self.inner.borrow().peers.contains_key(&guest_peer_id) {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "star host has no slot for that guest",
            ));
        }
        if !guest.inner.borrow().peers.contains_key(HOST_PEER_ID) {
            return Err(TransportError::new(
                TransportErrorCode::NotConnected,
                "guest endpoint is not initialized",
            ));
        }
        {
            let mut s = self.inner.borrow_mut();
            let peer = s.peers.get_mut(&guest_peer_id).expect("checked above");
            peer.link = Some(guest.inner.clone());
            peer.link_peer_id = Some(HOST_PEER_ID.to_string());
            peer.ice_state = "connected".to_string();
        }
        {
            let mut g = guest.inner.borrow_mut();
            let host_peer = g.peers.get_mut(HOST_PEER_ID).expect("checked above");
            host_peer.link = Some(self.inner.clone());
            host_peer.link_peer_id = Some(guest_peer_id.clone());
            host_peer.ice_state = "connected".to_string();
        }
        set_peer_state(
            &mut self.inner.borrow_mut(),
            &guest_peer_id,
            TransportPeerState::Connected,
        );
        set_peer_state(
            &mut guest.inner.borrow_mut(),
            HOST_PEER_ID,
            TransportPeerState::Connected,
        );
        Ok(true)
    }

    /// Test seam: delivers everything currently buffered on this endpoint
    /// and on every endpoint linked to it, then clears the simulated
    /// `bufferedAmount`. Mirrors `FakeStarTransport:pump`.
    pub fn pump(&self) {
        let mut endpoints: Vec<Rc<RefCell<Inner>>> = vec![self.inner.clone()];
        {
            let inner = self.inner.borrow();
            for peer_id in &inner.order {
                if let Some(link) = inner.peers[peer_id].link.clone()
                    && !endpoints.iter().any(|e| Rc::ptr_eq(e, &link))
                {
                    endpoints.push(link);
                }
            }
        }
        for endpoint in &endpoints {
            flush(endpoint);
        }
    }

    /// This endpoint's uplink/downlink byte counters. Mirrors
    /// `FakeStarTransport:wire_bytes`.
    #[must_use]
    pub fn wire_bytes(&self) -> (i64, i64) {
        let inner = self.inner.borrow();
        (inner.uplink_bytes, inner.downlink_bytes)
    }
}

impl StarTransportAdapter for FakeStarTransport {
    fn initialize(&mut self) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        if inner.state == TransportState::Connected {
            return Ok(true);
        }
        set_state(&mut inner, TransportState::Connected);
        if inner.role == TransportRole::Guest && !inner.peers.contains_key(HOST_PEER_ID) {
            add_peer(&mut inner, HOST_PEER_ID);
        }
        Ok(true)
    }

    fn shutdown(&mut self) -> TransportResult<bool> {
        let inner = self.inner.borrow_mut();
        if inner.state == TransportState::Closed {
            return Ok(true);
        }
        let order = inner.order.clone();
        drop(inner);
        for peer_id in &order {
            let _ = self.close_peer(peer_id, Some("star transport shutdown"));
        }
        let mut inner = self.inner.borrow_mut();
        inner.peers = IndexMap::new();
        inner.order = Vec::new();
        inner.cursor = 0;
        set_state(&mut inner, TransportState::Closed);
        Ok(true)
    }

    fn role(&self) -> TransportRole {
        self.inner.borrow().role
    }

    fn capacity(&self) -> i64 {
        self.inner.borrow().max_guests
    }

    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.role != TransportRole::Host {
            return Err(TransportError::new(
                TransportErrorCode::RoleForbidden,
                "only a host opens guest links",
            ));
        }
        if let Err((code, message)) = validate_peer_id(peer_id) {
            record_error(&mut inner, code, &message, None, None);
            return Err(TransportError::new(code, message));
        }
        if peer_id == HOST_PEER_ID {
            record_error(
                &mut inner,
                TransportErrorCode::DuplicatePeer,
                "the host peer id is reserved",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::DuplicatePeer,
                "the host peer id is reserved",
            ));
        }
        if inner.peers.contains_key(peer_id) {
            record_error(
                &mut inner,
                TransportErrorCode::DuplicatePeer,
                "star peer is already open",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::DuplicatePeer,
                "star peer is already open",
            ));
        }
        if inner.order.len() as i64 >= inner.max_guests {
            record_error(
                &mut inner,
                TransportErrorCode::Capacity,
                "star transport is at guest capacity",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::Capacity,
                "star transport is at guest capacity",
            ));
        }
        Ok(add_peer(&mut inner, peer_id))
    }

    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        if !inner.peers.contains_key(peer_id) {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "star transport has no peer with that id",
            ));
        }
        if inner.peers[peer_id].state == TransportPeerState::Closed {
            return Ok(true);
        }
        let detail = reason.unwrap_or("star peer closed").to_string();
        drop_peer_queues(&mut inner, peer_id);
        let (link, link_peer_id) = {
            let peer = inner.peers.get_mut(peer_id).expect("checked above");
            peer.ice_state = "closed".to_string();
            (peer.link.take(), peer.link_peer_id.take())
        };
        set_peer_state(&mut inner, peer_id, TransportPeerState::Closed);
        record_error(
            &mut inner,
            TransportErrorCode::Disconnected,
            &detail,
            Some(peer_id),
            None,
        );
        drop(inner);
        if let (Some(link), Some(link_peer_id)) = (link, link_peer_id) {
            let mut remote = link.borrow_mut();
            if let Some(remote_peer) = remote.peers.get_mut(&link_peer_id)
                && remote_peer.state != TransportPeerState::Closed
            {
                remote_peer.link = None;
                remote_peer.link_peer_id = None;
                drop_peer_queues(&mut remote, &link_peer_id);
                if let Some(remote_peer) = remote.peers.get_mut(&link_peer_id) {
                    remote_peer.ice_state = "closed".to_string();
                }
                set_peer_state(&mut remote, &link_peer_id, TransportPeerState::Disconnected);
                record_error(
                    &mut remote,
                    TransportErrorCode::Disconnected,
                    &detail,
                    Some(&link_peer_id),
                    None,
                );
            }
        }
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

    fn request_offer(&mut self, peer_id: &str) -> TransportResult<bool> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.role != TransportRole::Host {
            record_error(
                &mut inner,
                TransportErrorCode::RoleForbidden,
                "only a host creates offers",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::RoleForbidden,
                "only a host creates offers",
            ));
        }
        if !inner.peers.contains_key(peer_id) {
            record_error(
                &mut inner,
                TransportErrorCode::UnknownPeer,
                "unknown peer",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "unknown peer",
            ));
        }
        let sequence = {
            let mut rendezvous = inner.rendezvous.0.borrow_mut();
            rendezvous.sequence += 1;
            rendezvous.sequence
        };
        let token = format!("offer:{sequence}");
        inner.rendezvous.0.borrow_mut().offers.insert(
            token.clone(),
            Offer {
                host: self.inner.clone(),
                peer_id: peer_id.to_string(),
            },
        );
        let peer = inner.peers.get_mut(peer_id).expect("checked above");
        peer.signal = Some(token);
        peer.ice_state = "checking".to_string();
        Ok(true)
    }

    fn accept_offer(&mut self, signal: &str) -> TransportResult<bool> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.role != TransportRole::Guest {
            record_error(
                &mut inner,
                TransportErrorCode::RoleForbidden,
                "only a guest accepts an offer",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::RoleForbidden,
                "only a guest accepts an offer",
            ));
        }
        let known = inner.rendezvous.0.borrow().offers.contains_key(signal);
        if !known {
            record_error(
                &mut inner,
                TransportErrorCode::SignalError,
                "remote offer is not a known fake star offer",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::SignalError,
                "remote offer is not a known fake star offer",
            ));
        }
        if !inner.peers.contains_key(HOST_PEER_ID) {
            return Err(TransportError::new(
                TransportErrorCode::NotConnected,
                "guest endpoint is not initialized",
            ));
        }
        inner
            .rendezvous
            .0
            .borrow_mut()
            .answers
            .insert(signal.to_string(), self.inner.clone());
        let host_peer = inner.peers.get_mut(HOST_PEER_ID).expect("checked above");
        host_peer.signal = Some(format!("answer:{signal}"));
        host_peer.ice_state = "checking".to_string();
        Ok(true)
    }

    fn accept_answer(&mut self, peer_id: &str, signal: &str) -> TransportResult<bool> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        {
            let mut inner = self.inner.borrow_mut();
            if inner.role != TransportRole::Host {
                record_error(
                    &mut inner,
                    TransportErrorCode::RoleForbidden,
                    "only a host accepts an answer",
                    None,
                    None,
                );
                return Err(TransportError::new(
                    TransportErrorCode::RoleForbidden,
                    "only a host accepts an answer",
                ));
            }
            if !inner.peers.contains_key(peer_id) {
                record_error(
                    &mut inner,
                    TransportErrorCode::UnknownPeer,
                    "unknown peer",
                    None,
                    None,
                );
                return Err(TransportError::new(
                    TransportErrorCode::UnknownPeer,
                    "unknown peer",
                ));
            }
        }
        let token = signal.strip_prefix("answer:").map(|s| s.to_string());
        let guest = {
            let inner = self.inner.borrow();
            let rendezvous = inner.rendezvous.0.borrow();
            let matched = token.as_deref().and_then(|token| {
                rendezvous.offers.get(token).and_then(|offer| {
                    if Rc::ptr_eq(&offer.host, &self.inner) && offer.peer_id == peer_id {
                        Some(token.to_string())
                    } else {
                        None
                    }
                })
            });
            matched.and_then(|token| rendezvous.answers.get(&token).cloned().map(|g| (token, g)))
        };
        let Some((token, guest_inner)) = guest else {
            let mut inner = self.inner.borrow_mut();
            record_error(
                &mut inner,
                TransportErrorCode::SignalError,
                "remote answer does not match a pending offer",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::SignalError,
                "remote answer does not match a pending offer",
            ));
        };
        {
            let inner = self.inner.borrow();
            let mut rendezvous = inner.rendezvous.0.borrow_mut();
            rendezvous.offers.shift_remove(&token);
            rendezvous.answers.shift_remove(&token);
        }
        {
            let mut inner = self.inner.borrow_mut();
            if let Some(peer) = inner.peers.get_mut(peer_id) {
                peer.signal = None;
            }
        }
        let guest = FakeStarTransport { inner: guest_inner };
        self.link(&guest)
    }

    fn take_signal(&mut self, peer_id: &str) -> TransportResult<Option<String>> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        let mut inner = self.inner.borrow_mut();
        let Some(peer) = inner.peers.get_mut(peer_id) else {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "star transport has no peer with that id",
            ));
        };
        Ok(peer.signal.take())
    }

    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.role == TransportRole::Guest && peer_id != HOST_PEER_ID {
            record_error(
                &mut inner,
                TransportErrorCode::RoleForbidden,
                "a guest may only address the host",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::RoleForbidden,
                "a guest may only address the host",
            ));
        }
        let known_peer = inner.peers.contains_key(peer_id);
        if let Err((code, msg)) = validate_channel_message(channel, &message) {
            record_error(
                &mut inner,
                code,
                &msg,
                if known_peer { Some(peer_id) } else { None },
                None,
            );
            return Err(TransportError::new(code, msg));
        }
        if !known_peer {
            record_error(
                &mut inner,
                TransportErrorCode::UnknownPeer,
                "star transport has no peer with that id",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "star transport has no peer with that id",
            ));
        }
        enqueue(&mut inner, peer_id, channel, message)
    }

    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.role != TransportRole::Host {
            record_error(
                &mut inner,
                TransportErrorCode::RoleForbidden,
                "only a host fans out canonical batches",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::RoleForbidden,
                "only a host fans out canonical batches",
            ));
        }
        if let Err((code, msg)) = validate_channel_message(channel, &message) {
            record_error(&mut inner, code, &msg, None, None);
            return Err(TransportError::new(code, msg));
        }
        let order = inner.order.clone();
        let mut delivered = 0i64;
        for peer_id in &order {
            if inner.peers[peer_id].state == TransportPeerState::Connected
                && enqueue(&mut inner, peer_id, channel, message.clone()).is_ok()
            {
                delivered += 1;
            }
        }
        Ok(delivered)
    }

    fn poll(&mut self) -> Option<TransportPeerMessage> {
        self.poll_batch(Some(1)).into_iter().next()
    }

    fn poll_batch(&mut self, limit: Option<i64>) -> Vec<TransportPeerMessage> {
        let budget = limit.unwrap_or(DEFAULT_POLL_BATCH).max(0);
        let mut inner = self.inner.borrow_mut();
        let mut batch = Vec::new();
        if inner.state != TransportState::Connected {
            return batch;
        }
        while (batch.len() as i64) < budget {
            match take(&mut inner) {
                Some(entry) => batch.push(entry),
                None => break,
            }
        }
        batch
    }

    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        self.inner.borrow_mut().events.pop_front()
    }

    fn state(&self) -> TransportState {
        self.inner.borrow().state
    }

    fn diagnostics(&self) -> TransportStarDiagnostics {
        let inner = self.inner.borrow();
        let mut peers = Vec::with_capacity(inner.order.len());
        for peer_id in &inner.order {
            let peer = &inner.peers[peer_id];
            let channel_diag = |channel: TransportChannel| -> TransportChannelDiagnostics {
                let ch = peer.channel(channel);
                TransportChannelDiagnostics {
                    state: Some(ch.state),
                    outbound_depth: ch.outbound.len() as i64,
                    inbound_depth: ch.inbound.len() as i64,
                    buffered_amount: ch.buffered_amount,
                    sent: ch.sent,
                    received: ch.received,
                    dropped_outbound: ch.dropped_outbound,
                    dropped_inbound: ch.dropped_inbound,
                }
            };
            peers.push(TransportPeerDiagnostics {
                peer_id: peer.peer_id.clone(),
                slot: peer.slot,
                state: Some(peer.state),
                ice_state: peer.ice_state.clone(),
                control: channel_diag(TransportChannel::Control),
                input: channel_diag(TransportChannel::Input),
                sequence_gaps: peer.sequence_gaps,
                backpressure: peer.backpressure,
                malformed: peer.malformed,
                last_error: peer.last_error.clone(),
            });
        }
        TransportStarDiagnostics {
            role: Some(inner.role),
            state: Some(inner.state),
            capacity: inner.max_guests,
            peer_count: inner.order.len() as i64,
            queue_limit: inner.queue_limit,
            buffered_amount_limit: inner.buffered_amount_limit,
            event_depth: inner.events.len() as i64,
            sent: inner.sent,
            received: inner.received,
            dropped_outbound: inner.dropped_outbound,
            dropped_inbound: inner.dropped_inbound,
            malformed: inner.malformed,
            unsupported_version: inner.unsupported_version,
            overflow: inner.overflow,
            backpressure: inner.backpressure,
            last_error: inner.last_error.clone(),
            peers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_and_guest() -> (FakeStarTransport, FakeStarTransport) {
        let rendezvous = FakeStarTransport::new_rendezvous();
        let mut host = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Host,
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        host.initialize().unwrap();
        let mut guest = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some("guest_1".to_string()),
            rendezvous: Some(rendezvous),
            ..Default::default()
        });
        guest.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.link(&guest).unwrap();
        (host, guest)
    }

    #[test]
    fn link_connects_both_sides() {
        let (host, guest) = host_and_guest();
        assert_eq!(
            host.peer_state("guest_1"),
            Some(TransportPeerState::Connected)
        );
        assert_eq!(
            guest.peer_state(HOST_PEER_ID),
            Some(TransportPeerState::Connected)
        );
    }

    #[test]
    fn a_sent_input_message_arrives_after_a_pump() {
        let (mut host, mut guest) = host_and_guest();
        host.send(
            "guest_1",
            TransportChannel::Input,
            TransportMessage {
                version: 1,
                kind: TransportMessageType::Input,
                seq: 0,
                tick: Some(0),
                payload: b"hello".to_vec(),
            },
        )
        .unwrap();
        host.pump();
        let batch = guest.poll_batch(None);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].peer_id, HOST_PEER_ID);
        assert_eq!(batch[0].message.payload, b"hello");
    }

    #[test]
    fn broadcast_fans_out_to_every_connected_guest() {
        let rendezvous = FakeStarTransport::new_rendezvous();
        let mut host = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Host,
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        host.initialize().unwrap();
        let mut guests = Vec::new();
        for index in 1..=3 {
            let peer_id = format!("guest_{index}");
            let mut guest = FakeStarTransport::new(FakeStarTransportOptions {
                role: TransportRole::Guest,
                peer_id: Some(peer_id.clone()),
                rendezvous: Some(rendezvous.clone()),
                ..Default::default()
            });
            guest.initialize().unwrap();
            host.open_peer(&peer_id).unwrap();
            host.link(&guest).unwrap();
            guests.push(guest);
        }
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
        assert_eq!(delivered, 3);
        host.pump();
        for guest in &mut guests {
            assert_eq!(guest.poll_batch(None).len(), 1);
        }
    }

    #[test]
    fn close_peer_disconnects_the_remote_link_too() {
        let (mut host, guest) = host_and_guest();
        host.close_peer("guest_1", None).unwrap();
        assert_eq!(host.peer_state("guest_1"), Some(TransportPeerState::Closed));
        assert_eq!(
            guest.peer_state(HOST_PEER_ID),
            Some(TransportPeerState::Disconnected)
        );
    }

    #[test]
    fn manual_signaling_reaches_the_same_linked_state() {
        let rendezvous = FakeStarTransport::new_rendezvous();
        let mut host = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Host,
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        host.initialize().unwrap();
        let mut guest = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some("guest_1".to_string()),
            rendezvous: Some(rendezvous),
            ..Default::default()
        });
        guest.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.request_offer("guest_1").unwrap();
        let offer = host.take_signal("guest_1").unwrap().unwrap();
        guest.accept_offer(&offer).unwrap();
        let answer = guest.take_signal(HOST_PEER_ID).unwrap().unwrap();
        host.accept_answer("guest_1", &answer).unwrap();
        assert_eq!(
            host.peer_state("guest_1"),
            Some(TransportPeerState::Connected)
        );
        assert_eq!(
            guest.peer_state(HOST_PEER_ID),
            Some(TransportPeerState::Connected)
        );
    }

    #[test]
    fn poll_batch_drains_control_and_input_round_robin_across_guests() {
        let rendezvous = FakeStarTransport::new_rendezvous();
        let mut host = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Host,
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        host.initialize().unwrap();
        let mut a = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some("guest_1".to_string()),
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        a.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.link(&a).unwrap();
        a.send(
            HOST_PEER_ID,
            TransportChannel::Input,
            TransportMessage {
                version: 1,
                kind: TransportMessageType::Input,
                seq: 0,
                tick: Some(0),
                payload: Vec::new(),
            },
        )
        .unwrap();
        a.pump();
        let batch = host.poll_batch(None);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].peer_id, "guest_1");
    }

    #[test]
    fn backpressure_latches_once_and_drains_on_a_later_pump() {
        let rendezvous = FakeStarTransport::new_rendezvous();
        // One encoded envelope with this shape (ASCII payload, no percent
        // escaping) is 23 bytes; a 25-byte budget fits exactly one per
        // flush pass, so four queued envelopes must drain over four pumps.
        let mut host = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Host,
            buffered_amount_limit: Some(25),
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        host.initialize().unwrap();
        let mut guest = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some("guest_1".to_string()),
            rendezvous: Some(rendezvous),
            ..Default::default()
        });
        guest.initialize().unwrap();
        host.open_peer("guest_1").unwrap();
        host.link(&guest).unwrap();
        for seq in 0..4 {
            host.send(
                "guest_1",
                TransportChannel::Input,
                TransportMessage {
                    version: 1,
                    kind: TransportMessageType::Input,
                    seq,
                    tick: Some(seq),
                    payload: b"abcdefghij".to_vec(),
                },
            )
            .unwrap();
        }
        host.pump();
        let diagnostics = host.diagnostics();
        assert!(
            diagnostics.peers[0].input.outbound_depth > 0,
            "backpressure must leave a residual queue"
        );
        assert!(
            host.diagnostics().backpressure > 0,
            "a latched backpressure event must be recorded"
        );
        let mut delivered = guest.poll_batch(None).len();
        for _ in 0..4 {
            host.pump();
            delivered += guest.poll_batch(None).len();
        }
        assert_eq!(delivered, 4, "every queued envelope must eventually drain");
    }
}
