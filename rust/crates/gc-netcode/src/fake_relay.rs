//! Port of `game/transport/fake_relay.lua`.
//!
//! `game/transport/**` is TypeScript-owned (`v2/README.md` §2), so — exactly
//! as `crate::fake_star`'s module doc says of its own file — this is not
//! "the" Rust port in the ordinary sense. It exists so the four
//! `transport_relay_topology_probe_*` cases in
//! `crates/gc-netcode/tests/protocol.rs` (deferred from the TypeScript port)
//! have a real in-process Rust relay to drive `crate::fault_harness`'s
//! `relay` topology against, the same reason `crate::fake_star::FakeStarTransport`
//! exists for the `star` one.
//!
//! # What makes it a relay rather than a star
//!
//! In [`crate::fake_star`] one endpoint *is* the hub: a host `broadcast`
//! enqueues one copy per guest link on the host's own uplink, and a guest may
//! address nobody but the host. Here every endpoint holds exactly **one**
//! physical link, to the room, and the room is not a player:
//!
//!  * [`FakeRelayTransport::broadcast`] costs **one** uplink copy no matter
//!    how many members receive it. That single difference is the whole
//!    bandwidth claim under test (`transport_relay_topology_probe_measures_sequencer_less_per_node_wire_cost`).
//!  * Any member may address any other member (see [`StarTransportAdapter::send`]).
//!    No role is privileged, and [`StarTransportAdapter::role`] is carried
//!    only because the adapter contract and the session layer above still
//!    name one. The room enforces nothing on it.
//!  * There is no manual signaling. `request_offer`, `accept_offer`,
//!    `accept_answer`, and `take_signal` cannot mean anything when the far
//!    end is a server the client dials, so they refuse rather than pretend.
//!
//! # The room never parses a game packet
//!
//! This is the property the topology decision rests on, so it is structural
//! here rather than promised. An endpoint encodes its own outbound message
//! into the contract's peer-addressed wire (`origin|channel|<envelope>`,
//! [`encode_addressed`]) *before* handing it to the room. [`RoomInner`] only
//! ever appends those opaque byte lines to a per-destination list,
//! concatenates the list with `\n` once per forward pass, and hands the frame
//! down. It calls no decoder, reads no field, and knows nothing about slots,
//! ticks, ownership, or canonicalisation. The receiving endpoint splits the
//! frame and decodes each line itself. This is also why, unlike
//! [`crate::fake_star`] (which never round-trips a message through text —
//! `encode` there exists purely to size `bufferedAmount`), this module has to
//! restate the *whole* wire contract: `decode`/`decode_addressed`, not just
//! `encode`/`encode_addressed`, because a relay frame really is bytes on the
//! wire between one flush and the next.
//!
//! A member never receives its own line back: [`forward_room`] skips nothing
//! explicitly — a member's own outbound units are never targeted at itself in
//! the first place (see [`StarTransportAdapter::send`] and
//! [`StarTransportAdapter::broadcast`], neither of which ever puts the
//! sender's own id in a unit's target list).
//!
//! # Byte accounting
//!
//! [`FakeRelayTransport::wire_bytes`] and [`FakeRelayTransport::wire_counters`]
//! count encoded envelope wires, one count per copy that actually crosses the
//! link — exactly how [`crate::fake_star`] counts. On a star a seven-guest
//! `broadcast` is seven uplink copies; here it is one.
//! `frame_overhead_bytes` is kept separately so a comparison never has to
//! guess whether framing was folded into the payload figure.
//!
//! # Object references, in Rust
//!
//! The Lua original stores the room as a shared table every member holds a
//! reference to (`room.endpoints[peer_id] = self`), and a member reaches
//! another member only by looking its id up in that shared table. This port
//! mirrors [`crate::fake_star`]'s approach: each endpoint's state lives
//! behind `Rc<RefCell<Inner>>`, [`FakeRelayTransport`] is a cheap `Clone`
//! handle onto it, and the shared room ([`FakeRelayRoom`], itself
//! `Rc<RefCell<RoomInner>>`) holds a clone of every member's `Inner` handle.
//! This is a real reference cycle — every member's `Inner` reaches the room,
//! and the room reaches every member's `Inner` — accepted for the same
//! reason [`crate::fake_star`]'s host/guest cycle is: this is a bounded-life
//! test fixture, not a long-running service.

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
// module doc). Unlike `crate::fake_star`, this file needs the *decode* half
// too: a relay frame is real bytes between one flush and the next, not a
// direct method call.
// ---------------------------------------------------------------------------

/// Mirrors `contract.VERSION`.
const VERSION: i64 = 1;
/// Mirrors `contract.DEFAULT_QUEUE_LIMIT`.
const DEFAULT_QUEUE_LIMIT: i64 = 64;
/// Mirrors `contract.MAX_QUEUE_LIMIT`.
const MAX_QUEUE_LIMIT: i64 = 256;
/// Mirrors `contract.MAX_PAYLOAD_BYTES`.
const MAX_PAYLOAD_BYTES: usize = 1024;
/// Mirrors `contract.MAX_GUESTS`, reused here as the relay's own per-member
/// membership ceiling (`FakeRelayTransportOptions.max_peers`'s default).
const MAX_PEERS: i64 = 7;
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

fn message_type_from_label(label: &str) -> Option<TransportMessageType> {
    match label {
        "input" => Some(TransportMessageType::Input),
        "event" => Some(TransportMessageType::Event),
        "state" => Some(TransportMessageType::State),
        _ => None,
    }
}

/// Mirrors `contract.validate_peer_id`.
fn validate_peer_id(peer_id: &str) -> Result<(), (TransportErrorCode, String)> {
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

/// Mirrors `contract.validate_channel`.
fn validate_channel(value: &str) -> Result<TransportChannel, (TransportErrorCode, String)> {
    match value {
        "control" => Ok(TransportChannel::Control),
        "input" => Ok(TransportChannel::Input),
        _ => Err((
            TransportErrorCode::ChannelMismatch,
            "transport channel must be control or input".to_string(),
        )),
    }
}

/// Mirrors `contract.validate` (the structural half; channel pairing is
/// [`validate_channel_message`]).
fn validate_message(message: &TransportMessage) -> Result<(), (TransportErrorCode, String)> {
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

/// Mirrors `contract.validate_channel_message`.
fn validate_channel_message(
    channel: TransportChannel,
    message: &TransportMessage,
) -> Result<(), (TransportErrorCode, String)> {
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

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Mirrors `contract.lua`'s local `unescape`. Needed here (unlike
/// `crate::fake_star`, which never decodes a wire it produced) because a
/// relay frame is real text between one flush and the next.
fn unescape(bytes: &[u8]) -> Result<Vec<u8>, (TransportErrorCode, String)> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 3 > bytes.len() {
                return Err((
                    TransportErrorCode::Malformed,
                    "transport wire field has an invalid escape".to_string(),
                ));
            }
            let (Some(hi), Some(lo)) = (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
            else {
                return Err((
                    TransportErrorCode::Malformed,
                    "transport wire field has an invalid escape".to_string(),
                ));
            };
            out.push((hi << 4) | lo);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Ok(out)
}

fn is_ascii_digits(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit)
}

fn parse_digits(bytes: &[u8]) -> Result<i64, (TransportErrorCode, String)> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<i64>().ok())
        .ok_or_else(|| {
            (
                TransportErrorCode::Malformed,
                "transport wire message numbers are invalid".to_string(),
            )
        })
}

/// Mirrors `contract.encode`: the canonical wire bytes for one envelope.
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

/// Mirrors `contract.decode`.
fn decode(wire: &[u8]) -> Result<TransportMessage, (TransportErrorCode, String)> {
    let parts: Vec<&[u8]> = wire.split(|&b| b == b'|').collect();
    if parts.len() != 5 {
        return Err((
            TransportErrorCode::Malformed,
            "transport wire message has invalid fields".to_string(),
        ));
    }
    let (raw_version, raw_type, raw_seq, raw_tick, raw_payload) =
        (parts[0], parts[1], parts[2], parts[3], parts[4]);
    if !is_ascii_digits(raw_version) || !is_ascii_digits(raw_seq) {
        return Err((
            TransportErrorCode::Malformed,
            "transport wire message numbers are invalid".to_string(),
        ));
    }
    if !raw_tick.is_empty() && !is_ascii_digits(raw_tick) {
        return Err((
            TransportErrorCode::Malformed,
            "transport wire message tick is invalid".to_string(),
        ));
    }
    let type_bytes = unescape(raw_type)?;
    let payload = unescape(raw_payload)?;
    let type_str = String::from_utf8(type_bytes).map_err(|_| {
        (
            TransportErrorCode::Malformed,
            "transport wire escape is invalid".to_string(),
        )
    })?;
    let kind = message_type_from_label(&type_str).ok_or_else(|| {
        (
            TransportErrorCode::Malformed,
            "transport message type is invalid".to_string(),
        )
    })?;
    let version = parse_digits(raw_version)?;
    let seq = parse_digits(raw_seq)?;
    let tick = if raw_tick.is_empty() {
        None
    } else {
        Some(parse_digits(raw_tick)?)
    };
    let message = TransportMessage {
        version,
        kind,
        seq,
        tick,
        payload,
    };
    validate_message(&message)?;
    Ok(message)
}

/// Peer-addressed wire framing: `peer_id|channel|<envelope wire>`. Mirrors
/// `contract.encode_addressed`.
fn encode_addressed(
    peer_id: &str,
    channel: TransportChannel,
    message: &TransportMessage,
) -> Vec<u8> {
    let wire = encode(message);
    let mut out = Vec::with_capacity(peer_id.len() + 1 + 8 + 1 + wire.len());
    out.extend_from_slice(peer_id.as_bytes());
    out.push(b'|');
    out.extend_from_slice(channel_label(channel).as_bytes());
    out.push(b'|');
    out.extend_from_slice(&wire);
    out
}

/// Splits `line` on its first two `|` bytes, leaving everything after the
/// second exactly as-is (which itself contains further `|`s — the envelope
/// wire). Mirrors the Lua pattern `"^([^|]*)|([^|]*)|(.*)$"`: `None` when
/// fewer than two `|` bytes are present, exactly when that pattern fails to
/// match.
fn split_addressed(line: &[u8]) -> Option<[&[u8]; 3]> {
    let first = line.iter().position(|&b| b == b'|')?;
    let rest = &line[first + 1..];
    let second = rest.iter().position(|&b| b == b'|')?;
    Some([&line[..first], &rest[..second], &rest[second + 1..]])
}

/// Mirrors `contract.decode_addressed`.
fn decode_addressed(
    line: &[u8],
) -> Result<TransportAddressedMessage, (TransportErrorCode, String)> {
    let Some([peer_id_bytes, channel_bytes, wire]) = split_addressed(line) else {
        return Err((
            TransportErrorCode::Malformed,
            "transport addressed wire has invalid fields".to_string(),
        ));
    };
    let peer_id = String::from_utf8(peer_id_bytes.to_vec()).map_err(|_| {
        (
            TransportErrorCode::Malformed,
            "transport peer id contains unsupported characters".to_string(),
        )
    })?;
    validate_peer_id(&peer_id)?;
    let channel_str = String::from_utf8(channel_bytes.to_vec()).map_err(|_| {
        (
            TransportErrorCode::ChannelMismatch,
            "transport channel must be control or input".to_string(),
        )
    })?;
    let channel = validate_channel(&channel_str)?;
    let message = decode(wire)?;
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
    Ok(TransportAddressedMessage {
        peer_id,
        channel,
        message,
    })
}

// ---------------------------------------------------------------------------
// `fake_relay.lua` itself.
// ---------------------------------------------------------------------------

/// Construction options for [`FakeRelayTransport::new`]. Mirrors
/// `FakeRelayTransportOptions`.
#[derive(Clone)]
pub struct FakeRelayTransportOptions {
    /// Carried for the session layer; the room ignores it — see the module
    /// doc: no member is privileged.
    pub role: TransportRole,
    /// Room-unique member identity. Required; [`FakeRelayTransport::new`]
    /// panics without one, mirroring the Lua `assert`.
    pub peer_id: Option<String>,
    /// Share one across every member of a single room. `None` mints a
    /// private one, so sharing is an explicit, greppable decision (mirrors
    /// [`crate::fake_star::FakeStarTransportOptions::rendezvous`]).
    pub room: Option<FakeRelayRoom>,
    /// Per-channel bounded queue depth.
    pub queue_limit: Option<i64>,
    /// Maximum simultaneous member links this endpoint may hold.
    pub max_peers: Option<i64>,
    /// Simulated `bufferedAmount` send-buffer ceiling, shared by every
    /// target of one queued unit — see the module doc's byte-accounting
    /// section.
    pub buffered_amount_limit: Option<i64>,
}

impl Default for FakeRelayTransportOptions {
    fn default() -> Self {
        FakeRelayTransportOptions {
            role: TransportRole::Host,
            peer_id: None,
            room: None,
            queue_limit: None,
            max_peers: None,
            buffered_amount_limit: None,
        }
    }
}

struct RelayChannel {
    state: TransportPeerState,
    inbound: VecDeque<TransportPeerMessage>,
    received: i64,
    dropped_inbound: i64,
    last_seq: Option<i64>,
}

impl RelayChannel {
    fn new() -> Self {
        RelayChannel {
            state: TransportPeerState::Connecting,
            inbound: VecDeque::new(),
            received: 0,
            dropped_inbound: 0,
            last_seq: None,
        }
    }
}

struct RelayPeer {
    peer_id: String,
    slot: i64,
    state: TransportPeerState,
    ice_state: String,
    channels: [RelayChannel; 2],
    arrival_seq: i64,
    sequence_gaps: i64,
    backpressure: i64,
    malformed: i64,
    sent: i64,
    dropped_outbound: i64,
    last_error: Option<String>,
}

impl RelayPeer {
    fn new(peer_id: String, slot: i64) -> Self {
        RelayPeer {
            peer_id,
            slot,
            state: TransportPeerState::Connecting,
            ice_state: "new".to_string(),
            channels: [RelayChannel::new(), RelayChannel::new()],
            arrival_seq: 0,
            sequence_gaps: 0,
            backpressure: 0,
            malformed: 0,
            sent: 0,
            dropped_outbound: 0,
            last_error: None,
        }
    }
}

/// One unit on the single link to the room, whatever its destination set.
/// This is the whole topological difference from [`crate::fake_star`], where
/// the same call costs one queued copy per destination.
#[derive(Clone)]
struct RelayUnit {
    /// Member ids resolved at send time; canonical order.
    targets: Vec<String>,
    /// `origin|channel|<envelope wire>`; opaque to the room.
    line: Vec<u8>,
    /// Encoded envelope wire length, excluding the frame header.
    bytes: i64,
}

struct RelayUplink {
    units: Vec<RelayUnit>,
    buffered_amount: i64,
    backpressured: bool,
    sent: i64,
    dropped_outbound: i64,
}

impl RelayUplink {
    fn new() -> Self {
        RelayUplink {
            units: Vec::new(),
            buffered_amount: 0,
            backpressured: false,
            sent: 0,
            dropped_outbound: 0,
        }
    }
}

/// Forwarding counters a [`FakeRelayRoom`] accumulates across every member.
/// Mirrors `FakeRelayRoomCounters`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeRelayRoomCounters {
    /// Framed downlink messages the room emitted.
    pub frames: i64,
    /// Opaque lines forwarded, counted once per destination.
    pub lines: i64,
    /// Lines with no connected destination left.
    pub dropped: i64,
}

struct RoomInner {
    /// Join order; the only ordering the room has.
    members: Vec<String>,
    endpoints: IndexMap<String, Rc<RefCell<Inner>>>,
    /// Destination peer id -> opaque lines this pass.
    inbox: IndexMap<String, Vec<Vec<u8>>>,
    counters: FakeRelayRoomCounters,
}

/// A shared, cloneable handle onto one room's membership and forwarding
/// state. Mirrors `FakeRelayRoom`. Members reach each other only through a
/// room they were all handed, so two rooms in one process cannot cross-talk —
/// the same explicitness [`crate::fake_star::FakeStarTransport::new_rendezvous`]
/// buys for a single logical star.
#[derive(Clone)]
pub struct FakeRelayRoom(Rc<RefCell<RoomInner>>);

impl FakeRelayRoom {
    fn new() -> Self {
        FakeRelayRoom(Rc::new(RefCell::new(RoomInner {
            members: Vec::new(),
            endpoints: IndexMap::new(),
            inbox: IndexMap::new(),
            counters: FakeRelayRoomCounters::default(),
        })))
    }

    /// A snapshot of this room's forwarding counters.
    #[must_use]
    pub fn counters(&self) -> FakeRelayRoomCounters {
        self.0.borrow().counters
    }

    /// How many members are currently in the room.
    #[must_use]
    pub fn member_count(&self) -> i64 {
        self.0.borrow().members.len() as i64
    }
}

struct Inner {
    role: TransportRole,
    peer_id: String,
    state: TransportState,
    queue_limit: i64,
    event_limit: i64,
    max_peers: i64,
    buffered_amount_limit: i64,
    room: FakeRelayRoom,
    peers: IndexMap<String, RelayPeer>,
    order: Vec<String>,
    uplink: [RelayUplink; 2],
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
    downlink_framed_bytes: i64,
    uplink_units: i64,
    downlink_frames: i64,
    frame_overhead: i64,
    last_error: Option<String>,
}

impl Inner {
    fn channel(&self, peer_id: &str, channel: TransportChannel) -> Option<&RelayChannel> {
        self.peers
            .get(peer_id)
            .map(|peer| &peer.channels[channel_index(channel)])
    }
}

/// Pure in-process **relay** transport: a third [`StarTransportAdapter`]
/// implementation alongside [`crate::fake_star::FakeStarTransport`]. See the
/// module doc for exactly how it differs from the star.
#[derive(Clone)]
pub struct FakeRelayTransport {
    inner: Rc<RefCell<Inner>>,
}

fn push_event(inner: &mut Inner, event: TransportPeerEvent) {
    if inner.events.len() as i64 >= inner.event_limit {
        inner.events.pop_front();
        inner.overflow += 1;
        inner.last_error = Some("fake relay transport event queue is full".to_string());
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
            "relay transport is not initialized",
        )),
        TransportState::Closed => Err(TransportError::new(
            TransportErrorCode::Closed,
            "relay transport is closed",
        )),
        TransportState::Connected => Ok(()),
        _ => Err(TransportError::new(
            TransportErrorCode::NotConnected,
            "relay transport is not connected",
        )),
    }
}

fn add_peer(inner: &mut Inner, peer_id: &str) -> i64 {
    let slot = inner.order.len() as i64 + 1;
    inner.peers.insert(
        peer_id.to_string(),
        RelayPeer::new(peer_id.to_string(), slot),
    );
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

/// Brings one directed link up on `inner`'s side. Called from both sides of
/// a join (mirrors `_connect_to`), which is what makes a member's peer table
/// symmetric without either side being "the" opener.
fn connect_to(inner: &mut Inner, other_peer_id: &str) {
    if !inner.peers.contains_key(other_peer_id) {
        if inner.order.len() as i64 >= inner.max_peers {
            record_error(
                inner,
                TransportErrorCode::Capacity,
                "relay room membership is at capacity",
                None,
                None,
            );
            return;
        }
        add_peer(inner, other_peer_id);
    }
    let peer = inner
        .peers
        .get_mut(other_peer_id)
        .expect("just ensured present");
    if peer.state == TransportPeerState::Connected {
        return;
    }
    peer.ice_state = "connected".to_string();
    set_peer_state(inner, other_peer_id, TransportPeerState::Connected);
}

/// Mirrors `_drop_peer_queues`: drops `inner`'s own inbound backlog *from*
/// `peer_id`, and removes `peer_id` from every outbound uplink unit's target
/// list, dropping units left with no destination.
fn drop_peer_queues(inner: &mut Inner, peer_id: &str) {
    let mut dropped_in_total = 0i64;
    if let Some(peer) = inner.peers.get_mut(peer_id) {
        for channel in peer.channels.iter_mut() {
            let count = channel.inbound.len() as i64;
            dropped_in_total += count;
            channel.dropped_inbound += count;
            channel.inbound.clear();
        }
    }
    inner.dropped_inbound += dropped_in_total;

    for idx in 0..CHANNEL_ORDER.len() {
        let uplink = &mut inner.uplink[idx];
        let mut kept: Vec<RelayUnit> = Vec::with_capacity(uplink.units.len());
        let mut dropped: Vec<RelayUnit> = Vec::new();
        for mut unit in uplink.units.drain(..) {
            if unit.targets.iter().any(|target| target == peer_id) {
                unit.targets.retain(|target| target != peer_id);
            }
            if unit.targets.is_empty() {
                dropped.push(unit);
            } else {
                kept.push(unit);
            }
        }
        for unit in &dropped {
            uplink.buffered_amount = (uplink.buffered_amount - unit.bytes).max(0);
            uplink.dropped_outbound += 1;
        }
        uplink.units = kept;
        let count = dropped.len() as i64;
        if count > 0 {
            inner.dropped_outbound += count;
            if let Some(peer) = inner.peers.get_mut(peer_id) {
                peer.dropped_outbound += count;
            }
        }
    }
}

/// Mirrors `_enqueue`.
fn enqueue(
    inner_rc: &Rc<RefCell<Inner>>,
    channel: TransportChannel,
    message: &TransportMessage,
    targets: Vec<String>,
    attributed: Option<&str>,
) -> TransportResult<bool> {
    let mut inner = inner_rc.borrow_mut();
    let idx = channel_index(channel);
    if inner.uplink[idx].units.len() as i64 >= inner.queue_limit {
        inner.uplink[idx].dropped_outbound += 1;
        inner.dropped_outbound += 1;
        record_error(
            &mut inner,
            TransportErrorCode::Overflow,
            "relay uplink queue is full",
            attributed,
            Some(channel),
        );
        return Err(TransportError::new(
            TransportErrorCode::Overflow,
            "relay uplink queue is full",
        ));
    }
    let wire = encode(message);
    let peer_id = inner.peer_id.clone();
    let line = encode_addressed(&peer_id, channel, message);
    let wire_len = wire.len() as i64;
    let line_len = line.len() as i64;
    inner.uplink[idx].units.push(RelayUnit {
        targets,
        line,
        bytes: wire_len,
    });
    inner.uplink[idx].buffered_amount += wire_len;
    inner.uplink[idx].sent += 1;
    inner.sent += 1;
    inner.uplink_units += 1;
    inner.uplink_bytes += wire_len;
    if channel == TransportChannel::Input {
        inner.input_uplink_bytes += wire_len;
    }
    inner.frame_overhead += line_len - wire_len;
    Ok(true)
}

/// Mirrors `_receive`.
fn receive(inner: &mut Inner, peer_id: &str, channel: TransportChannel, message: TransportMessage) {
    let idx = channel_index(channel);
    let queue_limit = inner.queue_limit;
    let over = inner
        .channel(peer_id, channel)
        .is_some_and(|ch| ch.inbound.len() as i64 >= queue_limit);
    if over {
        if let Some(peer) = inner.peers.get_mut(peer_id) {
            peer.channels[idx].dropped_inbound += 1;
        }
        inner.dropped_inbound += 1;
        record_error(
            inner,
            TransportErrorCode::Overflow,
            "relay member inbound queue is full",
            Some(peer_id),
            Some(channel),
        );
        return;
    }
    let seq = message.seq;
    let Some(peer) = inner.peers.get_mut(peer_id) else {
        return;
    };
    let recv_peer_id = peer.peer_id.clone();
    if let Some(last_seq) = peer.channels[idx].last_seq
        && seq > last_seq + 1
    {
        peer.sequence_gaps += seq - last_seq - 1;
    }
    if peer.channels[idx].last_seq.is_none_or(|last| seq > last) {
        peer.channels[idx].last_seq = Some(seq);
    }
    peer.channels[idx].inbound.push_back(TransportPeerMessage {
        peer_id: recv_peer_id,
        channel,
        message,
        // Stamped at poll time, for the same reason the star does it there:
        // it must mean the same thing on every adapter.
        arrival_seq: 0,
    });
}

/// Splits the frame the room handed down and decodes each line back into an
/// addressed envelope. Mirrors `_receive_frame`.
fn receive_frame(inner_rc: &Rc<RefCell<Inner>>, frame: &[u8]) {
    let mut inner = inner_rc.borrow_mut();
    inner.downlink_frames += 1;
    inner.downlink_framed_bytes += frame.len() as i64;
    for line in frame.split(|&b| b == b'\n') {
        match decode_addressed(line) {
            Err((code, message)) => {
                record_error(&mut inner, code, &message, None, None);
            }
            Ok(addressed) => {
                let connected = inner
                    .peers
                    .get(&addressed.peer_id)
                    .is_some_and(|peer| peer.state == TransportPeerState::Connected);
                if !connected {
                    inner.dropped_inbound += 1;
                } else {
                    let wire_bytes = line.len() as i64
                        - addressed.peer_id.len() as i64
                        - channel_label(addressed.channel).len() as i64
                        - 2;
                    inner.downlink_bytes += wire_bytes;
                    if addressed.channel == TransportChannel::Input {
                        inner.input_downlink_bytes += wire_bytes;
                    }
                    let peer_id = addressed.peer_id.clone();
                    let channel = addressed.channel;
                    receive(&mut inner, &peer_id, channel, addressed.message);
                }
            }
        }
    }
}

/// The same persistent (slot, channel-rank) cursor `crate::fake_star` uses,
/// so release order is a property of the contract rather than of the
/// topology. `cursor`, `slots`, and `channels` are all non-negative by
/// construction (`cursor` only ever advances modulo a positive value), so
/// Rust's truncating `%` already agrees with Lua's floored `%` here — no
/// `rem_euclid` needed, same reasoning as `crate::fake_star::take`. Mirrors
/// `_take`.
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
            .channel(&peer_id, channel)
            .is_some_and(|ch| !ch.inbound.is_empty());
        if has_entry {
            let peer = inner.peers.get_mut(&peer_id).expect("peer exists");
            let mut entry = peer.channels[channel_index(channel)]
                .inbound
                .pop_front()
                .expect("checked non-empty above");
            peer.channels[channel_index(channel)].received += 1;
            inner.received += 1;
            let peer = inner.peers.get_mut(&peer_id).expect("peer exists");
            peer.arrival_seq += 1;
            entry.arrival_seq = peer.arrival_seq;
            return Some(entry);
        }
    }
    None
}

/// Drains one endpoint's single uplink under the same `bufferedAmount` model
/// `crate::fake_star` uses, with one difference that is the point: the
/// budget is per *link*, and this endpoint has one link however many members
/// are listening. Mirrors `_flush`.
fn flush(inner_rc: &Rc<RefCell<Inner>>, room: &FakeRelayRoom) {
    for &channel in &CHANNEL_ORDER {
        let idx = channel_index(channel);
        let mut budget = inner_rc.borrow().buffered_amount_limit;
        loop {
            let unit = inner_rc.borrow().uplink[idx].units.first().cloned();
            let Some(unit) = unit else { break };
            if unit.bytes > budget {
                let mut inner = inner_rc.borrow_mut();
                let already = inner.uplink[idx].backpressured;
                if !already {
                    inner.uplink[idx].backpressured = true;
                    inner.backpressure += 1;
                    let message = "relay uplink send buffer is full".to_string();
                    inner.last_error = Some(message.clone());
                    for target in &unit.targets {
                        if let Some(peer) = inner.peers.get_mut(target) {
                            peer.backpressure += 1;
                            peer.last_error = Some(message.clone());
                        }
                    }
                    push_event(
                        &mut inner,
                        TransportPeerEvent {
                            kind: TransportPeerEventKind::PeerError,
                            peer_id: unit.targets.first().cloned(),
                            channel: Some(channel),
                            state: None,
                            code: Some(TransportErrorCode::Backpressure),
                            message: Some(message),
                        },
                    );
                }
                break;
            }
            {
                let mut inner = inner_rc.borrow_mut();
                let uplink = &mut inner.uplink[idx];
                uplink.units.remove(0);
                uplink.backpressured = false;
                uplink.buffered_amount = (uplink.buffered_amount - unit.bytes).max(0);
            }
            budget -= unit.bytes;
            let mut delivered = 0i64;
            for target in &unit.targets {
                let peer_ready = inner_rc
                    .borrow()
                    .peers
                    .get(target)
                    .is_some_and(|peer| peer.state == TransportPeerState::Connected);
                let remote_exists = room.0.borrow().endpoints.contains_key(target);
                if !(peer_ready && remote_exists) {
                    continue;
                }
                let appended = {
                    let mut room_inner = room.0.borrow_mut();
                    if let Some(inbox) = room_inner.inbox.get_mut(target) {
                        inbox.push(unit.line.clone());
                        room_inner.counters.lines += 1;
                        true
                    } else {
                        false
                    }
                };
                if appended {
                    let mut inner = inner_rc.borrow_mut();
                    if let Some(peer) = inner.peers.get_mut(target) {
                        peer.sent += 1;
                    }
                    delivered += 1;
                }
            }
            if delivered == 0 {
                {
                    let mut inner = inner_rc.borrow_mut();
                    inner.uplink[idx].dropped_outbound += 1;
                    inner.dropped_outbound += 1;
                }
                room.0.borrow_mut().counters.dropped += 1;
            }
        }
    }
}

/// Flushes every member's uplink into the room in join order, then lets the
/// room frame one message per destination. Mirrors `FakeRelayTransport.pump_room`.
fn pump_room(room: &FakeRelayRoom) {
    let members = room.0.borrow().members.clone();
    for member_id in &members {
        let endpoint = room.0.borrow().endpoints.get(member_id).cloned();
        if let Some(endpoint) = endpoint {
            flush(&endpoint, room);
        }
    }
    forward_room(room);
}

/// The relay itself: concatenates the opaque lines received for each
/// destination this pass and hands the destination one frame. It decodes
/// nothing and never returns a line to its own origin — origin is decided by
/// which member handed the line up, never by reading it. Mirrors
/// `FakeRelayTransport.forward_room`.
fn forward_room(room: &FakeRelayRoom) {
    let members = room.0.borrow().members.clone();
    for member_id in &members {
        let lines = {
            let mut room_inner = room.0.borrow_mut();
            room_inner.inbox.get_mut(member_id).map(std::mem::take)
        };
        let Some(lines) = lines else { continue };
        if lines.is_empty() {
            continue;
        }
        let mut frame = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                frame.push(b'\n');
            }
            frame.extend_from_slice(line);
        }
        room.0.borrow_mut().counters.frames += 1;
        let endpoint = room.0.borrow().endpoints.get(member_id).cloned();
        match endpoint {
            Some(endpoint) => receive_frame(&endpoint, &frame),
            None => room.0.borrow_mut().counters.dropped += lines.len() as i64,
        }
    }
}

fn channel_diagnostics(
    inner: &Inner,
    peer: &RelayPeer,
    channel: TransportChannel,
) -> TransportChannelDiagnostics {
    let idx = channel_index(channel);
    // The uplink is shared, so a member's outbound view is the units still
    // queued that are addressed to it. Byte figures follow the same rule,
    // which keeps the depth gate meaningful without pretending the copies
    // are real.
    let mut depth = 0i64;
    let mut buffered = 0i64;
    for unit in &inner.uplink[idx].units {
        if unit.targets.iter().any(|target| target == &peer.peer_id) {
            depth += 1;
            buffered += unit.bytes;
        }
    }
    TransportChannelDiagnostics {
        state: Some(peer.channels[idx].state),
        outbound_depth: depth,
        inbound_depth: peer.channels[idx].inbound.len() as i64,
        buffered_amount: buffered,
        sent: peer.sent,
        received: peer.channels[idx].received,
        dropped_outbound: peer.dropped_outbound,
        dropped_inbound: peer.channels[idx].dropped_inbound,
    }
}

/// Encoded envelope wires this endpoint put on its uplink and took off its
/// downlink, and the framing this port needs to keep separately comparable
/// with `crate::fake_star`'s figures. Mirrors `FakeRelayWireCounters`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeRelayWireCounters {
    /// Envelope wire bytes sent.
    pub uplink_bytes: i64,
    /// Envelope wire bytes received.
    pub downlink_bytes: i64,
    /// The `input` channel alone, which is the per-tick match cost.
    pub input_uplink_bytes: i64,
    /// The `input` channel alone, received.
    pub input_downlink_bytes: i64,
    /// What actually crossed the link: addressing and separators included.
    pub downlink_framed_bytes: i64,
    /// Uplink units sent (one per `send`/`broadcast` call, however many
    /// members it reached).
    pub uplink_units: i64,
    /// Framed downlink messages received.
    pub downlink_frames: i64,
    /// Total addressing/separator overhead across every uplink unit.
    pub frame_overhead_bytes: i64,
}

impl FakeRelayTransport {
    /// One room. Members reach each other only through a room they were all
    /// handed. Mirrors `FakeRelayTransport.new_room`.
    #[must_use]
    pub fn new_room() -> FakeRelayRoom {
        FakeRelayRoom::new()
    }

    /// Builds one member endpoint. Mirrors `FakeRelayTransport.new`.
    ///
    /// # Panics
    ///
    /// Panics (mirroring the Lua `assert`s, all programmer errors per
    /// AGENTS.md §7) if `options.peer_id` is absent or not a valid transport
    /// peer id, or if `queue_limit`, `max_peers`, or `buffered_amount_limit`
    /// are outside their supported ranges.
    #[must_use]
    pub fn new(options: FakeRelayTransportOptions) -> Self {
        let queue_limit = options.queue_limit.unwrap_or(DEFAULT_QUEUE_LIMIT);
        assert!(
            queue_limit > 0 && queue_limit <= MAX_QUEUE_LIMIT,
            "fake relay transport queue_limit is outside the supported range"
        );
        let max_peers = options.max_peers.unwrap_or(MAX_PEERS);
        assert!(
            max_peers > 0 && max_peers <= MAX_PEERS,
            "fake relay transport max_peers is outside the supported range"
        );
        let buffered_amount_limit = options
            .buffered_amount_limit
            .unwrap_or(DEFAULT_BUFFERED_AMOUNT_LIMIT);
        assert!(
            buffered_amount_limit > 0 && buffered_amount_limit <= MAX_BUFFERED_AMOUNT_LIMIT,
            "fake relay transport buffered_amount_limit is outside the supported range"
        );
        let peer_id = options
            .peer_id
            .expect("a fake relay member needs its own peer id");
        validate_peer_id(&peer_id).expect("fake relay transport peer_id must be valid");
        let room = options.room.unwrap_or_else(FakeRelayTransport::new_room);
        let inner = Inner {
            role: options.role,
            peer_id,
            state: TransportState::New,
            queue_limit,
            event_limit: queue_limit.max(2),
            max_peers,
            buffered_amount_limit,
            room,
            peers: IndexMap::new(),
            order: Vec::new(),
            uplink: [RelayUplink::new(), RelayUplink::new()],
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
            downlink_framed_bytes: 0,
            uplink_units: 0,
            downlink_frames: 0,
            frame_overhead: 0,
            last_error: None,
        };
        FakeRelayTransport {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    /// Test seam: flushes every member's uplink into the shared room, then
    /// lets the room frame and deliver everything it collected. Because
    /// every member of a room shares the same [`FakeRelayRoom`], calling
    /// this on any one member pumps the whole room. Mirrors
    /// `FakeRelayTransport:pump`.
    pub fn pump(&self) {
        let room = self.inner.borrow().room.clone();
        pump_room(&room);
    }

    /// This endpoint's uplink/downlink envelope byte counters. Mirrors
    /// `FakeRelayTransport:wire_bytes`.
    #[must_use]
    pub fn wire_bytes(&self) -> (i64, i64) {
        let inner = self.inner.borrow();
        (inner.uplink_bytes, inner.downlink_bytes)
    }

    /// This endpoint's full wire counters. Mirrors
    /// `FakeRelayTransport:wire_counters`.
    #[must_use]
    pub fn wire_counters(&self) -> FakeRelayWireCounters {
        let inner = self.inner.borrow();
        FakeRelayWireCounters {
            uplink_bytes: inner.uplink_bytes,
            downlink_bytes: inner.downlink_bytes,
            input_uplink_bytes: inner.input_uplink_bytes,
            input_downlink_bytes: inner.input_downlink_bytes,
            downlink_framed_bytes: inner.downlink_framed_bytes,
            uplink_units: inner.uplink_units,
            downlink_frames: inner.downlink_frames,
            frame_overhead_bytes: inner.frame_overhead,
        }
    }
}

impl StarTransportAdapter for FakeRelayTransport {
    /// Joining the room is the whole handshake. Every member already in the
    /// room gains a link to the newcomer and the newcomer gains one to each
    /// of them, in join order, so slot numbering is a deterministic function
    /// of arrival and not of any hash order. Mirrors `FakeRelayTransport:initialize`.
    fn initialize(&mut self) -> TransportResult<bool> {
        if self.inner.borrow().state == TransportState::Connected {
            return Ok(true);
        }
        let room = self.inner.borrow().room.clone();
        let peer_id = self.inner.borrow().peer_id.clone();
        {
            let existing = room.0.borrow().endpoints.get(&peer_id).cloned();
            if let Some(existing) = existing
                && !Rc::ptr_eq(&existing, &self.inner)
            {
                return Err(TransportError::new(
                    TransportErrorCode::DuplicatePeer,
                    "the relay room already holds that member id",
                ));
            }
        }
        {
            let mut inner = self.inner.borrow_mut();
            set_state(&mut inner, TransportState::Connected);
        }
        {
            let mut room_inner = room.0.borrow_mut();
            room_inner
                .endpoints
                .insert(peer_id.clone(), self.inner.clone());
            room_inner.inbox.entry(peer_id.clone()).or_default();
            if !room_inner.members.iter().any(|member| member == &peer_id) {
                room_inner.members.push(peer_id.clone());
            }
        }
        let members = room.0.borrow().members.clone();
        for member_id in &members {
            if member_id == &peer_id {
                continue;
            }
            let other_rc = room.0.borrow().endpoints.get(member_id).cloned();
            let Some(other_rc) = other_rc else { continue };
            let other_connected = other_rc.borrow().state == TransportState::Connected;
            if !other_connected {
                continue;
            }
            {
                let mut inner = self.inner.borrow_mut();
                connect_to(&mut inner, member_id);
            }
            {
                let mut other_inner = other_rc.borrow_mut();
                connect_to(&mut other_inner, &peer_id);
            }
        }
        Ok(true)
    }

    fn shutdown(&mut self) -> TransportResult<bool> {
        if self.inner.borrow().state == TransportState::Closed {
            return Ok(true);
        }
        let order = self.inner.borrow().order.clone();
        for peer_id in &order {
            let _ = self.close_peer(peer_id, Some("relay transport shutdown"));
        }
        let peer_id = self.inner.borrow().peer_id.clone();
        let room = self.inner.borrow().room.clone();
        {
            let mut inner = self.inner.borrow_mut();
            inner.peers = IndexMap::new();
            inner.order = Vec::new();
            inner.cursor = 0;
            inner.uplink = [RelayUplink::new(), RelayUplink::new()];
        }
        {
            let mut room_inner = room.0.borrow_mut();
            room_inner.endpoints.shift_remove(&peer_id);
            room_inner.inbox.shift_remove(&peer_id);
            room_inner.members.retain(|member| member != &peer_id);
        }
        {
            let mut inner = self.inner.borrow_mut();
            set_state(&mut inner, TransportState::Closed);
        }
        Ok(true)
    }

    fn role(&self) -> TransportRole {
        self.inner.borrow().role
    }

    fn capacity(&self) -> i64 {
        self.inner.borrow().max_peers
    }

    /// Declaring a member explicitly. A relay has no privileged opener, so
    /// unlike the star this is not host-only: it exists because the adapter
    /// contract names it, and because a caller may want a link before the
    /// far member has joined. Mirrors `FakeRelayTransport:open_peer`.
    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        if let Err((code, message)) = validate_peer_id(peer_id) {
            let mut inner = self.inner.borrow_mut();
            record_error(&mut inner, code, &message, None, None);
            return Err(TransportError::new(code, message));
        }
        let mut inner = self.inner.borrow_mut();
        if peer_id == inner.peer_id {
            record_error(
                &mut inner,
                TransportErrorCode::DuplicatePeer,
                "a relay member cannot address itself",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::DuplicatePeer,
                "a relay member cannot address itself",
            ));
        }
        if inner.peers.contains_key(peer_id) {
            record_error(
                &mut inner,
                TransportErrorCode::DuplicatePeer,
                "relay member is already open",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::DuplicatePeer,
                "relay member is already open",
            ));
        }
        if inner.order.len() as i64 >= inner.max_peers {
            record_error(
                &mut inner,
                TransportErrorCode::Capacity,
                "relay room membership is at capacity",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::Capacity,
                "relay room membership is at capacity",
            ));
        }
        Ok(add_peer(&mut inner, peer_id))
    }

    /// Closes one member link without disturbing the rest of the room. The
    /// remote member is told, because a relay does know when a client's link
    /// to it drops. Mirrors `FakeRelayTransport:close_peer`.
    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        if !self.inner.borrow().peers.contains_key(peer_id) {
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "relay transport has no member with that id",
            ));
        }
        if self.inner.borrow().peers[peer_id].state == TransportPeerState::Closed {
            return Ok(true);
        }
        let detail = reason.unwrap_or("relay member closed").to_string();
        {
            let mut inner = self.inner.borrow_mut();
            drop_peer_queues(&mut inner, peer_id);
            if let Some(peer) = inner.peers.get_mut(peer_id) {
                peer.ice_state = "closed".to_string();
            }
            set_peer_state(&mut inner, peer_id, TransportPeerState::Closed);
            record_error(
                &mut inner,
                TransportErrorCode::Disconnected,
                &detail,
                Some(peer_id),
                None,
            );
        }
        let room = self.inner.borrow().room.clone();
        let other_rc = room.0.borrow().endpoints.get(peer_id).cloned();
        if let Some(other_rc) = other_rc
            && !Rc::ptr_eq(&other_rc, &self.inner)
        {
            let self_peer_id = self.inner.borrow().peer_id.clone();
            let remote_open = other_rc
                .borrow()
                .peers
                .get(&self_peer_id)
                .is_some_and(|peer| peer.state != TransportPeerState::Closed);
            if remote_open {
                let mut other_inner = other_rc.borrow_mut();
                drop_peer_queues(&mut other_inner, &self_peer_id);
                if let Some(remote_peer) = other_inner.peers.get_mut(&self_peer_id) {
                    remote_peer.ice_state = "closed".to_string();
                }
                set_peer_state(
                    &mut other_inner,
                    &self_peer_id,
                    TransportPeerState::Disconnected,
                );
                record_error(
                    &mut other_inner,
                    TransportErrorCode::Disconnected,
                    &detail,
                    Some(&self_peer_id),
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

    /// A relay client dials a server with a known address: there is no offer
    /// to hand to a human, no answer to paste back, and no ICE state worth
    /// reporting. Refuses rather than fakes it — see the module doc.
    fn request_offer(&mut self, _peer_id: &str) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        record_error(
            &mut inner,
            TransportErrorCode::SignalError,
            "relay endpoints do not create offers",
            None,
            None,
        );
        Err(TransportError::new(
            TransportErrorCode::SignalError,
            "a relay endpoint has no manual signaling; the room is the rendezvous",
        ))
    }

    fn accept_offer(&mut self, _signal: &str) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        record_error(
            &mut inner,
            TransportErrorCode::SignalError,
            "relay endpoints do not accept offers",
            None,
            None,
        );
        Err(TransportError::new(
            TransportErrorCode::SignalError,
            "a relay endpoint has no manual signaling; the room is the rendezvous",
        ))
    }

    fn accept_answer(&mut self, _peer_id: &str, _signal: &str) -> TransportResult<bool> {
        let mut inner = self.inner.borrow_mut();
        record_error(
            &mut inner,
            TransportErrorCode::SignalError,
            "relay endpoints do not accept answers",
            None,
            None,
        );
        Err(TransportError::new(
            TransportErrorCode::SignalError,
            "a relay endpoint has no manual signaling; the room is the rendezvous",
        ))
    }

    fn take_signal(&mut self, _peer_id: &str) -> TransportResult<Option<String>> {
        Ok(None)
    }

    /// Address one member. Any member may address any other: the relay has
    /// no privileged direction, which is precisely what removes the
    /// sequencer. Mirrors `FakeRelayTransport:send`.
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
        let peer_exists = self.inner.borrow().peers.contains_key(peer_id);
        if let Err((code, msg)) = validate_channel_message(channel, &message) {
            let mut inner = self.inner.borrow_mut();
            record_error(
                &mut inner,
                code,
                &msg,
                if peer_exists { Some(peer_id) } else { None },
                None,
            );
            return Err(TransportError::new(code, msg));
        }
        if !peer_exists {
            let mut inner = self.inner.borrow_mut();
            record_error(
                &mut inner,
                TransportErrorCode::UnknownPeer,
                "relay transport has no member with that id",
                None,
                None,
            );
            return Err(TransportError::new(
                TransportErrorCode::UnknownPeer,
                "relay transport has no member with that id",
            ));
        }
        let connected = self
            .inner
            .borrow()
            .peers
            .get(peer_id)
            .is_some_and(|peer| peer.state == TransportPeerState::Connected);
        if !connected {
            let mut inner = self.inner.borrow_mut();
            record_error(
                &mut inner,
                TransportErrorCode::NotConnected,
                "relay member link is not connected",
                Some(peer_id),
                Some(channel),
            );
            return Err(TransportError::new(
                TransportErrorCode::NotConnected,
                "relay member link is not connected",
            ));
        }
        enqueue(
            &self.inner,
            channel,
            &message,
            vec![peer_id.to_string()],
            Some(peer_id),
        )
    }

    /// Fan-out for the price of one upload. Returns how many members the
    /// room will frame this to; per-member failures stay visible through
    /// events and diagnostics, exactly as on the star. Mirrors
    /// `FakeRelayTransport:broadcast`.
    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        {
            let inner = self.inner.borrow();
            require_connected(&inner)?;
        }
        if let Err((code, msg)) = validate_channel_message(channel, &message) {
            let mut inner = self.inner.borrow_mut();
            record_error(&mut inner, code, &msg, None, None);
            return Err(TransportError::new(code, msg));
        }
        let targets: Vec<String> = {
            let inner = self.inner.borrow();
            inner
                .order
                .iter()
                .filter(|peer_id| inner.peers[*peer_id].state == TransportPeerState::Connected)
                .cloned()
                .collect()
        };
        if targets.is_empty() {
            return Ok(0);
        }
        let count = targets.len() as i64;
        enqueue(&self.inner, channel, &message, targets, None)?;
        Ok(count)
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
            peers.push(TransportPeerDiagnostics {
                peer_id: peer.peer_id.clone(),
                slot: peer.slot,
                state: Some(peer.state),
                ice_state: peer.ice_state.clone(),
                control: channel_diagnostics(&inner, peer, TransportChannel::Control),
                input: channel_diagnostics(&inner, peer, TransportChannel::Input),
                sequence_gaps: peer.sequence_gaps,
                backpressure: peer.backpressure,
                malformed: peer.malformed,
                last_error: peer.last_error.clone(),
            });
        }
        TransportStarDiagnostics {
            role: Some(inner.role),
            state: Some(inner.state),
            capacity: inner.max_peers,
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
    use crate::fake_star::{FakeStarTransport, FakeStarTransportOptions};

    fn member_id(index: i64) -> String {
        if index == 1 {
            "host".to_string()
        } else {
            format!("guest_{}", index - 1)
        }
    }

    fn build_room(count: i64) -> (FakeRelayRoom, Vec<FakeRelayTransport>) {
        let room = FakeRelayTransport::new_room();
        let mut members = Vec::new();
        for index in 1..=count {
            let mut endpoint = FakeRelayTransport::new(FakeRelayTransportOptions {
                role: if index == 1 {
                    TransportRole::Host
                } else {
                    TransportRole::Guest
                },
                peer_id: Some(member_id(index)),
                room: Some(room.clone()),
                ..Default::default()
            });
            endpoint.initialize().expect("member initializes");
            members.push(endpoint);
        }
        (room, members)
    }

    fn control(seq: i64, payload: &[u8]) -> TransportMessage {
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Event,
            seq,
            tick: None,
            payload: payload.to_vec(),
        }
    }

    fn input(seq: i64, tick: i64, payload: &[u8]) -> TransportMessage {
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq,
            tick: Some(tick),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn joins_every_member_to_every_other_without_a_privileged_opener() {
        let (_, members) = build_room(8);
        for (index, endpoint) in members.iter().enumerate() {
            let index = index as i64 + 1;
            assert_eq!(endpoint.peer_ids().len(), 7);
            assert_eq!(endpoint.state(), TransportState::Connected);
            for other in 1..=8 {
                if other != index {
                    assert_eq!(
                        endpoint.peer_state(&member_id(other)),
                        Some(TransportPeerState::Connected)
                    );
                }
            }
        }
    }

    #[test]
    fn delivers_an_addressed_envelope_and_never_echoes_a_members_own_line() {
        let (_, mut members) = build_room(3);
        members[0]
            .broadcast(TransportChannel::Input, input(1, 40, b"authority"))
            .unwrap();
        members[0].pump();
        assert_eq!(members[0].poll_batch(Some(16)).len(), 0);
        for member in members.iter_mut().skip(1) {
            let batch = member.poll_batch(Some(16));
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].peer_id, "host");
            assert_eq!(batch[0].channel, TransportChannel::Input);
            assert_eq!(batch[0].message.payload, b"authority");
        }
    }

    #[test]
    fn charges_one_upload_per_broadcast_where_the_star_charges_one_per_link() {
        let (_, mut members) = build_room(8);
        let payload = vec![b'x'; 200];
        members[0]
            .broadcast(TransportChannel::Input, input(1, 40, &payload))
            .unwrap();
        members[0].pump();
        let relay = members[0].wire_counters();

        let mut star = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Host,
            ..Default::default()
        });
        star.initialize().unwrap();
        for index in 2..=8 {
            let mut guest = FakeStarTransport::new(FakeStarTransportOptions {
                role: TransportRole::Guest,
                peer_id: Some(member_id(index)),
                ..Default::default()
            });
            guest.initialize().unwrap();
            star.open_peer(&member_id(index)).unwrap();
            star.link(&guest).unwrap();
        }
        star.broadcast(TransportChannel::Input, input(1, 40, &payload))
            .unwrap();
        let (hub_uplink_bytes, _) = star.wire_bytes();

        assert_eq!(relay.uplink_units, 1, "one relay upload");
        // The star pays the fan-out seven times; the relay pays it once.
        assert_eq!(hub_uplink_bytes, relay.uplink_bytes * 7);
    }

    #[test]
    fn lets_any_member_address_any_other_which_the_star_forbids() {
        let (_, mut members) = build_room(3);
        assert!(
            members[1]
                .send("guest_2", TransportChannel::Control, control(0, b"hello"))
                .unwrap()
        );
    }

    #[test]
    fn forwards_a_payload_the_room_cannot_possibly_parse() {
        let (room, mut members) = build_room(2);
        let payload = b"not|an|input|packet\nwith a newline and % escapes".to_vec();
        members[0]
            .send("guest_1", TransportChannel::Input, input(7, 12, &payload))
            .unwrap();
        members[0].pump();
        let batch = members[1].poll_batch(Some(16));
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].message.payload, payload);
        assert_eq!(room.counters().lines, 1);
        assert_eq!(room.counters().dropped, 0);
    }

    #[test]
    fn refuses_manual_signaling_instead_of_faking_it() {
        let (_, mut members) = build_room(2);
        let offer = members[0].request_offer("guest_1").unwrap_err();
        assert_eq!(offer.code, TransportErrorCode::SignalError);
        let accepted = members[1].accept_offer("offer:1").unwrap_err();
        assert_eq!(accepted.code, TransportErrorCode::SignalError);
        let answered = members[0]
            .accept_answer("guest_1", "answer:offer:1")
            .unwrap_err();
        assert_eq!(answered.code, TransportErrorCode::SignalError);
        assert_eq!(members[0].take_signal("guest_1").unwrap(), None);
    }

    #[test]
    fn closes_one_member_link_without_disturbing_the_room() {
        let (_, mut members) = build_room(3);
        members[0].close_peer("guest_1", Some("peer_left")).unwrap();
        assert_eq!(
            members[0].peer_state("guest_1"),
            Some(TransportPeerState::Closed)
        );
        assert_eq!(
            members[1].peer_state("host"),
            Some(TransportPeerState::Disconnected)
        );
        assert_eq!(
            members[2].peer_state("host"),
            Some(TransportPeerState::Connected)
        );
        members[0]
            .broadcast(TransportChannel::Input, input(1, 40, b"authority"))
            .unwrap();
        members[0].pump();
        assert_eq!(members[1].poll_batch(Some(16)).len(), 0);
        assert_eq!(members[2].poll_batch(Some(16)).len(), 1);
    }

    #[test]
    fn drains_and_leaves_the_room_on_shutdown() {
        let (room, mut members) = build_room(3);
        members[0]
            .send("guest_1", TransportChannel::Control, control(0, b"hello"))
            .unwrap();
        members[0].shutdown().unwrap();
        assert_eq!(members[0].state(), TransportState::Closed);
        assert_eq!(members[0].diagnostics().peers.len(), 0);
        assert_eq!(room.member_count(), 2);
        assert_eq!(
            members[1].peer_state("host"),
            Some(TransportPeerState::Disconnected)
        );
    }

    #[test]
    fn reports_diagnostics_the_contract_computes() {
        let (_, mut members) = build_room(3);
        members[0]
            .send("guest_1", TransportChannel::Control, control(0, b"hello"))
            .unwrap();
        let diagnostics = members[0].diagnostics();
        assert_eq!(diagnostics.peer_count, 2);
        assert_eq!(diagnostics.peers.len(), 2);
        assert_eq!(diagnostics.peers[0].peer_id, "guest_1");
        assert_eq!(diagnostics.peers[0].slot, 1);
        assert_eq!(
            diagnostics.peers[0].state,
            Some(TransportPeerState::Connected)
        );
        assert_eq!(
            diagnostics.peers[0].control.outbound_depth, 1,
            "the addressed unit shows on that link"
        );
        assert_eq!(
            diagnostics.peers[1].control.outbound_depth, 0,
            "and only on that link"
        );
    }
}
