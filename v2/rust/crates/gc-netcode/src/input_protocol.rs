//! Port of `game/online/input_protocol.lua`.
//!
//! **`input_protocol` is the wire format for player input.** Its bytes go on
//! the network and into rollback re-simulation: if two clients encode
//! differently by one bit, the match desyncs. [`encode`]/[`decode`] are
//! therefore differential-tested against the reference Lua implementation
//! (see `v2/tools/lua_reference/README.md` and
//! `crates/gc-netcode/tests/input_protocol.rs`).
//!
//! ## What is deliberately not here
//!
//! The Lua original also defines `input_protocol.validate_envelope` and
//! `input_protocol.canonical_host_batch`. Both are omitted from this port:
//!
//! - `validate_envelope` needs `TransportMessage` from `game/transport/contract.lua`.
//!   Per `v2/README.md` §2, `game/transport/**` is TypeScript-owned
//!   (`packages/transport`); no Rust port of it exists or is planned, so there
//!   is no type to validate against here.
//! - `canonical_host_batch` needs `SessionManifest`, `SessionSlotProducer`,
//!   `protocol.validate_manifest`, `protocol.manifest_id`, and
//!   `protocol.validate_assignment_manifest` from `game/online/protocol.lua`.
//!   This agent's brief explicitly excludes `protocol.lua` ("later agents own
//!   those"), and `crates/gc-netcode/src/protocol.rs` is still the unported
//!   placeholder, so those types do not exist yet either.
//!
//! Every constant and every function that does not need those two modules is
//! fully ported below. The spec assertions that exercise the two omitted
//! functions are marked `#[ignore]` in `tests/input_protocol.rs`, naming the
//! missing module.
//!
//! ## Constants duplicated from `protocol.lua`
//!
//! [`packet_id`] and [`validate`] bound `session_id`/`sender_id`/`sequence`
//! against `protocol.MAX_SESSION_ID_BYTES` (128), `protocol.MAX_PEER_ID_BYTES`
//! (128), and `protocol.MAX_SEQUENCE` (2147483647) — see
//! `game/online/protocol.lua:270-273`. Those are plain numeric bounds, not
//! logic, so they are duplicated here as local constants rather than pulling
//! in the rest of `protocol.lua`. The same is true of
//! `transport_contract.MAX_PAYLOAD_BYTES` (1024, `game/transport/contract.lua:162`),
//! asserted below exactly as the Lua module does at load time.

use gc_sim::input_frame;

/// Wire header, present as the first field of every encoded packet.
const HEADER: &[u8] = b"GCIP";
/// Standard base64 alphabet, index-addressed.
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Mirrors `protocol.MAX_SESSION_ID_BYTES` (`game/online/protocol.lua:270`).
/// See the module-level doc comment for why this is duplicated rather than
/// imported.
const PROTOCOL_MAX_SESSION_ID_BYTES: usize = 128;
/// Mirrors `protocol.MAX_PEER_ID_BYTES` (`game/online/protocol.lua:271`).
const PROTOCOL_MAX_PEER_ID_BYTES: usize = 128;
/// Mirrors `protocol.MAX_SEQUENCE` (`game/online/protocol.lua:273`).
const PROTOCOL_MAX_SEQUENCE: i64 = 2_147_483_647;
/// Mirrors `transport_contract.MAX_PAYLOAD_BYTES` (`game/transport/contract.lua:162`).
const TRANSPORT_MAX_PAYLOAD_BYTES: usize = 1024;

/// Frame protocol version. Bumping this is a breaking wire change.
pub const VERSION: i64 = 1;
/// How many prior input ticks a guest packet retains alongside its current tick.
pub const HISTORY_ROWS: i64 = 6;
/// `HISTORY_ROWS` plus the current tick: a guest packet's total row count.
pub const RETAINED_ROWS: i64 = HISTORY_ROWS + 1;
/// Fixed tick delay between a local input sample and its earliest legal send.
pub const FAIRNESS_DELAY_TICKS: i64 = 3;
/// The largest row count a guest packet may carry.
pub const MAX_GUEST_ROWS: i64 = RETAINED_ROWS;

/// How many input ticks one host batch carries for every canonical slot. See
/// `game/online/input_protocol.lua:81-107` for the full sizing rationale
/// behind this specific value (#243).
pub const HOST_WINDOW_ROWS: i64 = 9;
/// The largest row count a host batch may carry: one window per canonical slot.
pub const MAX_HOST_ROWS: i64 = input_frame::SLOT_COUNT * HOST_WINDOW_ROWS;
/// Fixed encoded byte size of one authority row: a 4-byte tick, a 1-byte slot
/// index, and a 4-byte sample.
pub const RECORD_BYTES: i64 = 9;
/// The transport payload bound every encoded packet must fit inside.
pub const MAX_WIRE_BYTES: usize = 1024;
/// The slack a maximally-full host batch must still leave inside
/// [`MAX_WIRE_BYTES`], so a future additive header field cannot quietly
/// consume the last byte unnoticed.
pub const MIN_WIRE_MARGIN_BYTES: usize = 64;

const _: () = assert!(
    TRANSPORT_MAX_PAYLOAD_BYTES >= MAX_WIRE_BYTES,
    "transport payload bound is smaller than the versioned input packet bound"
);

/// Which side produced an [`InputPacket`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketKind {
    /// A single-slot bundle from a guest (or the host acting as one of its
    /// own local producers).
    Guest,
    /// The host's canonical, multi-slot authority batch.
    Host,
}

/// Machine-readable reason an input-protocol operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The data violates a structural, range, or canonical-form invariant.
    Malformed,
    /// The encoded wire would exceed (or does exceed) [`MAX_WIRE_BYTES`].
    WireTooLarge,
    /// The data names a packet or sample version this build does not support.
    UnsupportedVersion,
    /// A packet's identity (session, manifest, sender, or packet id) does not
    /// match its decode context or its own computed id.
    IdentityMismatch,
    /// A tick or sequence disagrees with the envelope or batch it arrived in.
    TickMismatch,
    /// A producer claimed authority over a slot it does not own.
    OwnershipMismatch,
    /// The same sender/sequence identity was reused for a different reason
    /// than an idempotent resend.
    Duplicate,
    /// The same sender/sequence identity was reused with different bytes.
    PacketConflict,
    /// Two authority rows for the same `(tick, slot)` disagree.
    AuthorityConflict,
    /// A host-local input bypassed the fixed fairness delay.
    FairnessDelay,
    /// A newer packet does not cover every row a backpressure replacement
    /// must carry.
    BackpressureGap,
}

/// How a duplicate sender/sequence resend should be treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateDisposition {
    /// Byte-identical to the original: safe to drop.
    Idempotent,
    /// Not safe to accept (always reported via [`Error`] instead of this
    /// variant in practice; kept for type fidelity with the Lua alias).
    Reject,
}

/// An expected, recoverable input-protocol failure (AGENTS.md §7): the caller
/// is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Machine-readable failure reason.
    pub code: ErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl Error {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

/// Result alias for fallible input-protocol operations.
pub type Result<T> = std::result::Result<T, Error>;

fn failure<T>(code: ErrorCode, message: impl Into<String>) -> Result<T> {
    Err(Error::new(code, message))
}

/// One canonical slot's input authority for one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorityRow {
    /// Fixed-simulation tick this row is authority for.
    pub tick: i64,
    /// One-based canonical input slot (wire value, never converted to
    /// 0-based: this is the exact byte a peer decodes — README rule 5.3).
    pub slot_index: i64,
    /// The slot's quantized input for this tick.
    pub sample: input_frame::InputSample,
}

/// A decoded or about-to-be-encoded input packet.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Which side produced this packet.
    pub kind: PacketKind,
    /// The [`input_frame::InputSample`] schema version the rows use.
    pub input_version: i64,
    /// Context-bound session identity (never itself hashed into the wire).
    pub session_id: String,
    /// Hash identifying the frozen session manifest.
    pub manifest_id: String,
    /// Context-bound producer identity (transport peer or declared bot).
    pub sender_id: String,
    /// Monotonic per-sender packet sequence.
    pub sequence: i64,
    /// `fnv1a64` digest over kind/session/sender/sequence; see [`packet_id`].
    pub packet_id: String,
    /// The sender's transport clock when this packet was authored, never an
    /// authority input tick.
    pub transport_tick: i64,
    /// The frozen session's first authority input tick.
    pub first_input_tick: i64,
    /// Contiguous confirmed input ticks from `first_input_tick`; 0 = none.
    pub confirmed_span: i64,
    /// Exactly [`FAIRNESS_DELAY_TICKS`].
    pub input_delay_ticks: i64,
    /// Authority rows in canonical `(tick, slot_index)` ascending order.
    pub rows: Vec<AuthorityRow>,
}

/// Inputs to [`new_guest`]/[`new_host`].
#[derive(Clone, Debug, PartialEq)]
pub struct PacketOptions {
    /// See [`Packet::session_id`].
    pub session_id: String,
    /// See [`Packet::manifest_id`].
    pub manifest_id: String,
    /// See [`Packet::sender_id`].
    pub sender_id: String,
    /// See [`Packet::sequence`].
    pub sequence: i64,
    /// See [`Packet::transport_tick`].
    pub transport_tick: i64,
    /// See [`Packet::first_input_tick`].
    pub first_input_tick: i64,
    /// See [`Packet::confirmed_span`]. Defaults to 0 (claims nothing) when `None`.
    pub confirmed_span: Option<i64>,
    /// See [`Packet::rows`].
    pub rows: Vec<AuthorityRow>,
}

/// Context a decoded packet's context-bound identity is checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeContext {
    /// Expected [`Packet::session_id`].
    pub session_id: String,
    /// Expected [`Packet::manifest_id`].
    pub manifest_id: String,
    /// Expected [`Packet::sender_id`].
    pub sender_id: String,
}

fn is_ascii_id_char(byte: u8, first: bool) -> bool {
    if byte.is_ascii_alphanumeric() {
        return true;
    }
    !first && matches!(byte, b'_' | b'.' | b'-')
}

/// A bounded opaque ASCII id: alphanumeric start, then alphanumerics plus
/// `_`, `.`, `-`.
fn validate_id(value: &str, name: &str, maximum: usize) -> Result<()> {
    let bytes = value.as_bytes();
    let canonical = !bytes.is_empty()
        && bytes.len() <= maximum
        && bytes
            .iter()
            .enumerate()
            .all(|(index, &byte)| is_ascii_id_char(byte, index == 0));
    if !canonical {
        return failure(
            ErrorCode::Malformed,
            format!("{name} must be a bounded opaque ASCII id"),
        );
    }
    Ok(())
}

fn validate_integer(value: i64, name: &str, minimum: i64, maximum: i64) -> Result<()> {
    if value < minimum || value > maximum {
        return failure(
            ErrorCode::Malformed,
            format!("{name} must be a bounded integer"),
        );
    }
    Ok(())
}

fn validate_hash(value: &str, name: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let canonical = bytes.len() == 16
        && bytes
            .iter()
            .all(|&byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !canonical {
        return failure(
            ErrorCode::Malformed,
            format!("{name} must be a canonical 16-hex digest"),
        );
    }
    Ok(())
}

fn length_prefix(value: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(value.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(value);
}

fn frame_error_to_protocol(error: input_frame::InputFrameError) -> Error {
    let code = match error.code {
        input_frame::InputFrameErrorCode::Malformed => ErrorCode::Malformed,
        input_frame::InputFrameErrorCode::UnsupportedVersion => ErrorCode::UnsupportedVersion,
        input_frame::InputFrameErrorCode::WireTooLarge => ErrorCode::WireTooLarge,
    };
    Error::new(code, error.message)
}

/// A defensive, re-validated copy of `sample`. Panics if `sample` is not
/// already valid, mirroring the Lua original's `assert(input_frame.new_sample(sample))`.
fn copy_sample(sample: &input_frame::InputSample) -> input_frame::InputSample {
    input_frame::new_sample((*sample).into()).expect("copy_sample requires an already-valid sample")
}

fn copy_row(row: &AuthorityRow) -> AuthorityRow {
    AuthorityRow {
        tick: row.tick,
        slot_index: row.slot_index,
        sample: copy_sample(&row.sample),
    }
}

fn copy_rows(rows: &[AuthorityRow]) -> Vec<AuthorityRow> {
    rows.iter().map(copy_row).collect()
}

fn row_less(left: &AuthorityRow, right: &AuthorityRow) -> bool {
    left.tick < right.tick || (left.tick == right.tick && left.slot_index < right.slot_index)
}

fn base64_encode(value: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(value.len().div_ceil(3) * 4);
    let mut index = 0;
    while index < value.len() {
        let first = value[index];
        let second = value.get(index + 1).copied();
        let third = value.get(index + 2).copied();
        let packed =
            (first as u32) * 65536 + (second.unwrap_or(0) as u32) * 256 + third.unwrap_or(0) as u32;
        let a = (packed / 262144) % 64;
        let b = (packed / 4096) % 64;
        let c = (packed / 64) % 64;
        let d = packed % 64;
        result.push(BASE64[a as usize]);
        result.push(BASE64[b as usize]);
        result.push(if second.is_some() {
            BASE64[c as usize]
        } else {
            b'='
        });
        result.push(if third.is_some() {
            BASE64[d as usize]
        } else {
            b'='
        });
        index += 3;
    }
    result
}

fn base64_value(byte: u8) -> Option<u32> {
    BASE64.iter().position(|&b| b == byte).map(|p| p as u32)
}

fn base64_decode(value: &[u8]) -> Option<Vec<u8>> {
    if value.is_empty()
        || !value.len().is_multiple_of(4)
        || !value
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    {
        return None;
    }
    let padding = value.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return None;
    }
    if value[..value.len() - padding].contains(&b'=') {
        return None;
    }
    let mut result = Vec::with_capacity(value.len() / 4 * 3);
    let mut index = 0;
    while index < value.len() {
        let raw_c = value[index + 2];
        let raw_d = value[index + 3];
        let a = base64_value(value[index])?;
        let b = base64_value(value[index + 1])?;
        let c = if raw_c == b'=' {
            Some(0)
        } else {
            base64_value(raw_c)
        }?;
        let d = if raw_d == b'=' {
            Some(0)
        } else {
            base64_value(raw_d)
        }?;
        let last_quartet = index == value.len() - 4;
        if (raw_c == b'=' || raw_d == b'=') && !last_quartet {
            return None;
        }
        if raw_c == b'=' && raw_d != b'=' {
            return None;
        }
        let packed = a * 262144 + b * 4096 + c * 64 + d;
        result.push(((packed / 65536) % 256) as u8);
        if raw_c != b'=' {
            result.push(((packed / 256) % 256) as u8);
        }
        if raw_d != b'=' {
            result.push((packed % 256) as u8);
        }
        index += 4;
    }
    if base64_encode(&result) != value {
        return None;
    }
    Some(result)
}

fn encode_u32(value: i64) -> [u8; 4] {
    let v = value as u32;
    [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn decode_u32(value: &[u8], index: usize) -> i64 {
    ((value[index] as i64) << 24)
        | ((value[index + 1] as i64) << 16)
        | ((value[index + 2] as i64) << 8)
        | (value[index + 3] as i64)
}

fn encode_row(row: &AuthorityRow) -> Result<Vec<u8>> {
    let sample_hex = input_frame::encode_sample(&row.sample).map_err(frame_error_to_protocol)?;
    let sample_bytes: Vec<u8> = sample_hex
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                .expect("encode_sample always returns lowercase hex pairs")
        })
        .collect();
    let mut bytes = Vec::with_capacity(RECORD_BYTES as usize);
    bytes.extend_from_slice(&encode_u32(row.tick));
    bytes.push(row.slot_index as u8);
    bytes.extend_from_slice(&sample_bytes);
    Ok(bytes)
}

fn decode_row(value: &[u8], index: usize) -> Result<AuthorityRow> {
    let slot_index = value[index + 4] as i64;
    let sample_wire: String = value[index + 5..index + 9]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let sample = input_frame::decode_sample(&sample_wire).map_err(frame_error_to_protocol)?;
    Ok(AuthorityRow {
        tick: decode_u32(value, index),
        slot_index,
        sample,
    })
}

/// The stable `fnv1a64` identity of a packet: a digest over its kind,
/// session, sender, and sequence. Two packets sharing sender + sequence but
/// differing here can never be the same logical send.
pub fn packet_id(
    kind: PacketKind,
    session_id: &str,
    sender_id: &str,
    sequence: i64,
) -> Result<String> {
    validate_id(
        session_id,
        "input session id",
        PROTOCOL_MAX_SESSION_ID_BYTES,
    )?;
    validate_id(sender_id, "input sender id", PROTOCOL_MAX_PEER_ID_BYTES)?;
    validate_integer(sequence, "input sequence", 0, PROTOCOL_MAX_SEQUENCE)?;
    let domain: &[u8] = match kind {
        PacketKind::Guest => b"GCIG;1;",
        PacketKind::Host => b"GCIH;1;",
    };
    let mut preimage = Vec::from(domain);
    length_prefix(session_id.as_bytes(), &mut preimage);
    length_prefix(sender_id.as_bytes(), &mut preimage);
    length_prefix(sequence.to_string().as_bytes(), &mut preimage);
    Ok(gc_core::fnv1a64::hash(&preimage))
}

/// Validate every invariant a well-formed [`Packet`] must satisfy: field
/// bounds, row canonical order, guest single-slot contiguity, and that
/// `packet_id` matches its own computed identity.
pub fn validate(packet: &Packet) -> Result<()> {
    if packet.version != VERSION {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported input packet version",
        );
    }
    if packet.input_version != input_frame::VERSION {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported input sample version",
        );
    }
    validate_id(
        &packet.session_id,
        "input session id",
        PROTOCOL_MAX_SESSION_ID_BYTES,
    )?;
    validate_id(
        &packet.sender_id,
        "input sender id",
        PROTOCOL_MAX_PEER_ID_BYTES,
    )?;
    validate_hash(&packet.manifest_id, "input manifest id")?;
    validate_integer(packet.sequence, "input sequence", 0, PROTOCOL_MAX_SEQUENCE)?;
    validate_integer(
        packet.transport_tick,
        "input transport tick",
        0,
        input_frame::MAX_TICK,
    )?;
    validate_integer(
        packet.first_input_tick,
        "first input tick",
        0,
        input_frame::MAX_TICK,
    )?;
    // Confirmation feedback, carried as a *span* rather than a tick so it is
    // always a canonical unsigned integer. A sender that has confirmed
    // nothing sits at `first_input_tick - 1`, which is -1 for a session that
    // starts at tick zero and has no unsigned encoding; the span makes that
    // state 0.
    validate_integer(
        packet.confirmed_span,
        "confirmed span",
        0,
        input_frame::MAX_TICK - packet.first_input_tick + 1,
    )?;
    if packet.input_delay_ticks != FAIRNESS_DELAY_TICKS {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported input fairness delay",
        );
    }
    let maximum = match packet.kind {
        PacketKind::Guest => MAX_GUEST_ROWS,
        PacketKind::Host => MAX_HOST_ROWS,
    };
    if packet.rows.is_empty() || packet.rows.len() as i64 > maximum {
        return failure(
            ErrorCode::Malformed,
            "input packet rows must be a bounded canonical array",
        );
    }
    for (index, row) in packet.rows.iter().enumerate() {
        if row.tick < packet.first_input_tick || row.tick > input_frame::MAX_TICK {
            return failure(
                ErrorCode::Malformed,
                "input packet row tick is outside the session range",
            );
        }
        if row.slot_index < 1 || row.slot_index > input_frame::SLOT_COUNT {
            return failure(ErrorCode::Malformed, "input packet row slot is invalid");
        }
        input_frame::validate_sample(&row.sample).map_err(frame_error_to_protocol)?;
        if index > 0 && !row_less(&packet.rows[index - 1], row) {
            return failure(
                ErrorCode::Malformed,
                "input packet rows are not in canonical tick-slot order",
            );
        }
    }
    if packet.kind == PacketKind::Guest {
        let slot_index = packet.rows[0].slot_index;
        for (index, row) in packet.rows.iter().enumerate() {
            if row.slot_index != slot_index {
                return failure(
                    ErrorCode::OwnershipMismatch,
                    "guest packet contains more than one slot",
                );
            }
            if index > 0 && row.tick != packet.rows[index - 1].tick + 1 {
                return failure(
                    ErrorCode::Malformed,
                    "guest packet history is not contiguous",
                );
            }
        }
        let current_tick = packet.rows[packet.rows.len() - 1].tick;
        let expected_first = packet.first_input_tick.max(current_tick - HISTORY_ROWS);
        if packet.rows[0].tick != expected_first
            || packet.rows.len() as i64 != current_tick - expected_first + 1
        {
            return failure(
                ErrorCode::Malformed,
                "guest packet must contain current input plus exactly the retained prior rows",
            );
        }
    }
    let expected_id = packet_id(
        packet.kind,
        &packet.session_id,
        &packet.sender_id,
        packet.sequence,
    )?;
    if packet.packet_id != expected_id {
        return failure(
            ErrorCode::IdentityMismatch,
            "input packet id does not match its context",
        );
    }
    Ok(())
}

/// Encode a packet to its canonical wire bytes. Wire payloads are raw bytes
/// (never `String`): a Lua string is a byte array, and the canonical
/// invariant this function proves is exact-byte reproduction, not text
/// formatting.
pub fn encode(packet: &Packet) -> Result<Vec<u8>> {
    validate(packet)?;
    let mut raw_rows = Vec::with_capacity(packet.rows.len() * RECORD_BYTES as usize);
    for row in &packet.rows {
        raw_rows.extend(encode_row(row)?);
    }
    let kind_char: &[u8] = match packet.kind {
        PacketKind::Guest => b"G",
        PacketKind::Host => b"H",
    };
    let fields: Vec<Vec<u8>> = vec![
        HEADER.to_vec(),
        VERSION.to_string().into_bytes(),
        kind_char.to_vec(),
        input_frame::VERSION.to_string().into_bytes(),
        packet.manifest_id.clone().into_bytes(),
        packet.sequence.to_string().into_bytes(),
        packet.packet_id.clone().into_bytes(),
        packet.transport_tick.to_string().into_bytes(),
        packet.first_input_tick.to_string().into_bytes(),
        packet.confirmed_span.to_string().into_bytes(),
        FAIRNESS_DELAY_TICKS.to_string().into_bytes(),
        packet.rows.len().to_string().into_bytes(),
        base64_encode(&raw_rows),
    ];
    let mut wire = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            wire.push(b';');
        }
        wire.extend_from_slice(field);
    }
    if wire.len() > MAX_WIRE_BYTES {
        return failure(
            ErrorCode::WireTooLarge,
            "input packet exceeds the transport payload bound",
        );
    }
    Ok(wire)
}

fn parse_unsigned(bytes: &[u8]) -> Option<i64> {
    if bytes == b"0" {
        return Some(0);
    }
    if bytes.is_empty() || bytes[0] == b'0' || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse::<i64>().ok()
}

/// Decode and fully validate a wire packet against `context`. `wire` is a raw
/// byte payload (never `String`): untrusted external input must be able to
/// carry an invalid-UTF-8 byte through to a graceful [`ErrorCode::Malformed`]
/// rather than panicking before this function is even reached.
pub fn decode(wire: &[u8], context: &DecodeContext) -> Result<Packet> {
    if wire.len() > MAX_WIRE_BYTES {
        return failure(
            ErrorCode::WireTooLarge,
            "input packet exceeds the transport payload bound",
        );
    }
    let fields: Vec<&[u8]> = wire.split(|&b| b == b';').collect();
    if fields.len() != 13 || fields[0] != HEADER {
        return failure(ErrorCode::Malformed, "input packet wire has invalid fields");
    }
    let version = parse_unsigned(fields[1]);
    let input_version = parse_unsigned(fields[3]);
    let (Some(version), Some(input_version)) = (version, input_version) else {
        return failure(
            ErrorCode::Malformed,
            "input packet versions are not canonical integers",
        );
    };
    if version != VERSION || input_version != input_frame::VERSION {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported input packet or sample version",
        );
    }
    let kind = match fields[2] {
        b"G" => PacketKind::Guest,
        b"H" => PacketKind::Host,
        _ => return failure(ErrorCode::Malformed, "input packet kind is invalid"),
    };
    if fields[4] != context.manifest_id.as_bytes() {
        return failure(
            ErrorCode::IdentityMismatch,
            "input manifest id does not match decode context",
        );
    }
    let sequence = parse_unsigned(fields[5]);
    let transport_tick = parse_unsigned(fields[7]);
    let first_input_tick = parse_unsigned(fields[8]);
    let confirmed_span = parse_unsigned(fields[9]);
    let input_delay_ticks = parse_unsigned(fields[10]);
    let row_count = parse_unsigned(fields[11]);
    let (
        Some(sequence),
        Some(transport_tick),
        Some(first_input_tick),
        Some(confirmed_span),
        Some(input_delay_ticks),
        Some(row_count),
    ) = (
        sequence,
        transport_tick,
        first_input_tick,
        confirmed_span,
        input_delay_ticks,
        row_count,
    )
    else {
        return failure(ErrorCode::Malformed, "input packet integer is noncanonical");
    };
    let maximum = match kind {
        PacketKind::Guest => MAX_GUEST_ROWS,
        PacketKind::Host => MAX_HOST_ROWS,
    };
    if row_count < 1 || row_count > maximum {
        return failure(
            ErrorCode::Malformed,
            "input packet row count is outside the kind bound",
        );
    }
    let Some(raw_rows) = base64_decode(fields[12]) else {
        return failure(ErrorCode::Malformed, "input packet record block is invalid");
    };
    if raw_rows.len() as i64 != row_count * RECORD_BYTES {
        return failure(ErrorCode::Malformed, "input packet record block is invalid");
    }
    let mut rows = Vec::with_capacity(row_count as usize);
    for index in 0..row_count as usize {
        let offset = index * RECORD_BYTES as usize;
        match decode_row(&raw_rows, offset) {
            Ok(row) => rows.push(row),
            Err(_) => return failure(ErrorCode::Malformed, "input packet record is invalid"),
        }
    }
    let packet = Packet {
        version,
        kind,
        input_version,
        session_id: context.session_id.clone(),
        // `fields[4]` was already byte-compared equal to `context.manifest_id`
        // above, so reuse that (already-UTF-8) value instead of re-decoding.
        manifest_id: context.manifest_id.clone(),
        sender_id: context.sender_id.clone(),
        sequence,
        packet_id: String::from_utf8_lossy(fields[6]).into_owned(),
        transport_tick,
        first_input_tick,
        confirmed_span,
        input_delay_ticks,
        rows,
    };
    validate(&packet)?;
    if encode(&packet)? != wire {
        return failure(ErrorCode::Malformed, "input packet wire is not canonical");
    }
    Ok(packet)
}

fn new_packet(kind: PacketKind, options: PacketOptions) -> Result<Packet> {
    let packet_id = packet_id(
        kind,
        &options.session_id,
        &options.sender_id,
        options.sequence,
    )?;
    let packet = Packet {
        version: VERSION,
        kind,
        input_version: input_frame::VERSION,
        session_id: options.session_id.clone(),
        manifest_id: options.manifest_id.clone(),
        sender_id: options.sender_id.clone(),
        sequence: options.sequence,
        packet_id,
        transport_tick: options.transport_tick,
        first_input_tick: options.first_input_tick,
        confirmed_span: options.confirmed_span.unwrap_or(0),
        input_delay_ticks: FAIRNESS_DELAY_TICKS,
        rows: options.rows.clone(),
    };
    validate(&packet)?;
    let packet = Packet {
        rows: copy_rows(&options.rows),
        ..packet
    };
    let wire = encode(&packet)?;
    decode(
        &wire,
        &DecodeContext {
            session_id: options.session_id,
            manifest_id: options.manifest_id,
            sender_id: options.sender_id,
        },
    )
}

/// Build and validate a guest (single-slot) packet.
pub fn new_guest(options: PacketOptions) -> Result<Packet> {
    new_packet(PacketKind::Guest, options)
}

/// Build and validate a host (canonical multi-slot) packet.
pub fn new_host(options: PacketOptions) -> Result<Packet> {
    new_packet(PacketKind::Host, options)
}

/// The sender's confirmed input tick, recovered from the span it reported.
/// `first_input_tick - 1` means "this sender has confirmed nothing yet",
/// which is the only value below the session's first tick this can return.
///
/// # Panics
///
/// Panics if `packet` is not a valid packet — mirrors the Lua original's
/// `assert(input_protocol.validate(packet))`.
pub fn confirmed_tick(packet: &Packet) -> i64 {
    validate(packet).unwrap_or_else(|err| panic!("{err}"));
    packet.first_input_tick + packet.confirmed_span - 1
}

/// The span a sender should report to claim confirmation through `confirmed_tick`.
pub fn confirmed_span(first_input_tick: i64, confirmed_tick: i64) -> i64 {
    (confirmed_tick - first_input_tick + 1).max(0)
}

/// A defensive, re-validated round trip of `packet` through its own wire form.
pub fn copy(packet: &Packet) -> Result<Packet> {
    let wire = encode(packet)?;
    decode(
        &wire,
        &DecodeContext {
            session_id: packet.session_id.clone(),
            manifest_id: packet.manifest_id.clone(),
            sender_id: packet.sender_id.clone(),
        },
    )
}

/// A defensive copy of `packet`'s rows.
///
/// # Panics
///
/// Panics if `packet` is not valid, mirroring the Lua original.
pub fn rows(packet: &Packet) -> Vec<AuthorityRow> {
    validate(packet).unwrap_or_else(|err| panic!("{err}"));
    copy_rows(&packet.rows)
}

/// Classify a repeated sender/sequence identity: byte-identical resends are
/// idempotent, anything else sharing that identity is a conflict.
pub fn classify_duplicate(previous: &Packet, incoming: &Packet) -> Result<DuplicateDisposition> {
    let previous_wire = encode(previous)?;
    let incoming_wire = encode(incoming)?;
    if previous.kind != incoming.kind
        || previous.session_id != incoming.session_id
        || previous.sender_id != incoming.sender_id
        || previous.sequence != incoming.sequence
    {
        return failure(
            ErrorCode::Duplicate,
            "input packets do not share a sender sequence identity",
        );
    }
    if previous_wire == incoming_wire {
        return Ok(DuplicateDisposition::Idempotent);
    }
    failure(
        ErrorCode::PacketConflict,
        "input sender sequence was reused with different bytes",
    )
}

fn packet_covers(older: &Packet, newer: &Packet) -> bool {
    if older.kind != newer.kind
        || older.session_id != newer.session_id
        || older.manifest_id != newer.manifest_id
        || older.sender_id != newer.sender_id
        || older.first_input_tick != newer.first_input_tick
        || newer.sequence < older.sequence
        || newer.transport_tick < older.transport_tick
    {
        return false;
    }
    for row in &older.rows {
        let covered = newer
            .rows
            .iter()
            .find(|candidate| candidate.tick == row.tick && candidate.slot_index == row.slot_index);
        match covered {
            Some(candidate) if candidate.sample == row.sample => {}
            _ => return false,
        }
    }
    true
}

/// Queue replacement is safe only when the newer packet carries every byte of
/// authority from the unsent packet. Ordinary next-tick bundles do not cover
/// the row that just fell out of the six-row history and therefore fail closed.
pub fn supersede_for_backpressure(older: &Packet, newer: &Packet) -> Result<Packet> {
    validate(older)?;
    validate(newer)?;
    if older.kind == newer.kind
        && older.session_id == newer.session_id
        && older.sender_id == newer.sender_id
        && older.sequence == newer.sequence
    {
        classify_duplicate(older, newer)?;
        return copy(newer);
    }
    if !packet_covers(older, newer) {
        return failure(
            ErrorCode::BackpressureGap,
            "newer input packet does not cover every unsent authority row",
        );
    }
    copy(newer)
}
