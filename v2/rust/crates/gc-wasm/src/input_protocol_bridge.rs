//! `wasm-bindgen` control surface over `gc_netcode::input_protocol` —
//! `packages/online/src/net_diagnostics_fixture.ts`'s `InputProtocolPort`,
//! including the forged single-slot bundles
//! `net_diagnostics.spec.ts`'s ownership-violation/authority-conflict cases
//! need (see that fixture's `forgedBundle`).
//!
//! ## Wire payloads are bytes, never strings
//!
//! [`gc_netcode::input_protocol::encode`]/[`gc_netcode::input_protocol::decode`]
//! operate on `Vec<u8>`/`&[u8]` — a Lua string is a raw byte array, and this
//! crate's own differential tests hold that codec to exact-byte
//! reproduction, not text formatting (see `input_protocol.rs`'s own doc).
//! [`input_protocol_encode`]/[`input_protocol_decode`] below keep that:
//! `Vec<u8>`/`&[u8]` cross the wasm boundary as `Uint8Array`, wasm-bindgen's
//! native (lossless, corruption-free) byte marshalling — never `String`,
//! which would force a UTF-8 re-encode a non-UTF-8 byte cannot survive.
//! `packages/wasm/src/binary_string.ts` is the one place this package
//! converts that `Uint8Array` to the "binary string" (one UTF-16 code unit
//! per byte) convention `@gc/transport`'s `TransportMessage.payload`
//! already uses (`packages/transport/src/contract.ts`'s own doc comment) —
//! so a packet encoded here can be handed straight to `newMessage` as a
//! transport payload with no further re-encoding, and the two conventions
//! never fight each other.
//!
//! ## Why a packet crosses as an opaque handle with two exposed fields
//!
//! `InputProtocolPort.newGuest`'s declared return carries an
//! `InputProtocolPacket` typed with exactly `sequence`/`transport_tick` —
//! "opaque except for the two fields this fixture reads back" (that
//! interface's own doc comment). [`WasmInputPacket`] matches that: those two
//! fields are plain `pub` values (so `packet.sequence`/`packet.transport_tick`
//! read as ordinary JS properties, the same convention
//! `crate::protocol_bridge::ControlMessageHeader`/`crate::tuning_bridge::WasmKnob`
//! already use for snake_case struct fields), while the full packet stays a
//! private Rust field, reachable only through [`input_protocol_encode`] and
//! this module's other free functions. A few more identity fields
//! (`session_id`, `manifest_id`, `sender_id`, `packet_id`, `kind`,
//! `first_input_tick`, `confirmed_span`, `rows_json`) are exposed too, since
//! they cost nothing extra and are useful for a caller building diagnostics
//! or a second packet from the first (`inputProtocolClassifyDuplicate`,
//! `inputProtocolSupersedeForBackpressure`) — but nothing in
//! `NetDiagnosticsFixtureEnv`'s ports requires hiding them.
//!
//! ## Rows cross as JSON, samples as `inputFrame`'s own wire string
//!
//! A row is `{"tick", "slot_index", "sample"}`, where `sample` is exactly
//! [`crate::input_frame_bridge::input_frame_new_sample`]/
//! `input_frame_neutral_sample`'s canonical hex wire — never a `Vec<
//! WasmInputSample>`-shaped wasm-bindgen parameter. That sidesteps the ABI
//! gap this wave's brief calls out explicitly: a `Vec<CustomExportedClass>`
//! *parameter* crossing wasm-bindgen is exercised by nothing in this crate's
//! native `cargo test` (only a real compiled-module smoke test proves it
//! marshals), whereas a JSON array of plain values is exactly the shape
//! `crate::coordinator_bridge`/`crate::match_driver_bridge` already prove
//! works, both natively and under node (`session.spec.ts`,
//! `determinism.spec.ts`). See `input_protocol.spec.ts` for the node-side
//! proof this module's own rows/bytes marshal correctly.

use gc_netcode::input_protocol::{self, AuthorityRow, Packet, PacketKind};
use gc_sim::input_frame;
use wasm_bindgen::prelude::*;

use crate::json::Json;

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

fn parse_json(text: &str) -> Result<Json, JsValue> {
    Json::parse(text).map_err(js_err)
}

fn kind_wire(kind: PacketKind) -> &'static str {
    match kind {
        PacketKind::Guest => "guest",
        PacketKind::Host => "host",
    }
}

fn kind_from_wire(text: &str) -> Result<PacketKind, JsValue> {
    match text {
        "guest" => Ok(PacketKind::Guest),
        "host" => Ok(PacketKind::Host),
        other => Err(js_err(format!(
            "input protocol packet kind must be \"guest\" or \"host\", got '{other}'"
        ))),
    }
}

fn row_to_json(row: &AuthorityRow) -> Result<Json, JsValue> {
    let sample_wire =
        input_frame::encode_sample(&row.sample).map_err(|err| js_err(err.to_string()))?;
    Ok(Json::obj(vec![
        ("tick", Json::int(row.tick)),
        ("slot_index", Json::int(row.slot_index)),
        ("sample", Json::str(sample_wire)),
    ]))
}

fn rows_to_json(rows: &[AuthorityRow]) -> Result<Json, JsValue> {
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(row_to_json(row)?);
    }
    Ok(Json::Array(items))
}

fn row_from_json(json: &Json) -> Result<AuthorityRow, JsValue> {
    let tick = json
        .field_i64("tick")
        .ok_or_else(|| js_err("input protocol row is missing 'tick'"))?;
    let slot_index = json
        .field_i64("slot_index")
        .ok_or_else(|| js_err("input protocol row is missing 'slot_index'"))?;
    let sample_wire = json
        .field_str("sample")
        .ok_or_else(|| js_err("input protocol row is missing 'sample'"))?;
    let sample = input_frame::decode_sample(sample_wire).map_err(|err| js_err(err.to_string()))?;
    Ok(AuthorityRow {
        tick,
        slot_index,
        sample,
    })
}

fn rows_from_json(json: &Json) -> Result<Vec<AuthorityRow>, JsValue> {
    let items = json
        .as_array()
        .ok_or_else(|| js_err("input protocol rows_json must be a JSON array"))?;
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        rows.push(row_from_json(item)?);
    }
    Ok(rows)
}

/// This build's `gc_netcode::input_protocol` bounds, as JSON:
/// `version`, `history_rows`, `retained_rows`, `fairness_delay_ticks`,
/// `max_guest_rows`, `host_window_rows`, `max_host_rows`, `record_bytes`,
/// `max_wire_bytes`, `min_wire_margin_bytes` — covers
/// `InputProtocolPort.FAIRNESS_DELAY_TICKS`/`HISTORY_ROWS` plus the rest of
/// the module's public bounds.
#[wasm_bindgen(js_name = inputProtocolConstantsJson)]
#[must_use]
pub fn input_protocol_constants_json() -> String {
    Json::obj(vec![
        ("version", Json::int(input_protocol::VERSION)),
        ("history_rows", Json::int(input_protocol::HISTORY_ROWS)),
        ("retained_rows", Json::int(input_protocol::RETAINED_ROWS)),
        (
            "fairness_delay_ticks",
            Json::int(input_protocol::FAIRNESS_DELAY_TICKS),
        ),
        ("max_guest_rows", Json::int(input_protocol::MAX_GUEST_ROWS)),
        (
            "host_window_rows",
            Json::int(input_protocol::HOST_WINDOW_ROWS),
        ),
        ("max_host_rows", Json::int(input_protocol::MAX_HOST_ROWS)),
        ("record_bytes", Json::int(input_protocol::RECORD_BYTES)),
        (
            "max_wire_bytes",
            Json::int(input_protocol::MAX_WIRE_BYTES as i64),
        ),
        (
            "min_wire_margin_bytes",
            Json::int(input_protocol::MIN_WIRE_MARGIN_BYTES as i64),
        ),
    ])
    .to_json_string()
}

/// A decoded or freshly-built `gc_netcode::input_protocol::Packet`. See the
/// module doc for which fields are exposed and why.
#[wasm_bindgen(getter_with_clone)]
pub struct WasmInputPacket {
    /// Monotonic per-sender packet sequence.
    pub sequence: f64,
    /// The sender's transport clock when this packet was authored.
    pub transport_tick: f64,
    /// The frozen session's first authority input tick.
    pub first_input_tick: f64,
    /// Contiguous confirmed input ticks from `first_input_tick`; 0 = none.
    pub confirmed_span: f64,
    /// Context-bound session identity.
    pub session_id: String,
    /// Hash identifying the frozen session manifest.
    pub manifest_id: String,
    /// Context-bound producer identity.
    pub sender_id: String,
    /// `fnv1a64` digest identifying this packet.
    pub packet_id: String,
    /// `"guest"` or `"host"`.
    pub kind: String,
    /// This packet's authority rows, as JSON (`[{"tick", "slot_index",
    /// "sample"}, ...]`, `"sample"` an `inputFrame` canonical hex wire), in
    /// canonical `(tick, slot_index)` order.
    pub rows_json: String,
    packet: Packet,
}

impl WasmInputPacket {
    fn from_packet(packet: Packet) -> Result<WasmInputPacket, JsValue> {
        let rows_json = rows_to_json(&packet.rows)?.to_json_string();
        Ok(WasmInputPacket {
            sequence: packet.sequence as f64,
            transport_tick: packet.transport_tick as f64,
            first_input_tick: packet.first_input_tick as f64,
            confirmed_span: packet.confirmed_span as f64,
            session_id: packet.session_id.clone(),
            manifest_id: packet.manifest_id.clone(),
            sender_id: packet.sender_id.clone(),
            packet_id: packet.packet_id.clone(),
            kind: kind_wire(packet.kind).to_string(),
            rows_json,
            packet,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    kind: PacketKind,
    session_id: &str,
    manifest_id: &str,
    sender_id: &str,
    sequence: f64,
    transport_tick: f64,
    first_input_tick: f64,
    confirmed_span: Option<f64>,
    rows_json: &str,
) -> Result<WasmInputPacket, JsValue> {
    let rows = rows_from_json(&parse_json(rows_json)?)?;
    let options = input_protocol::PacketOptions {
        session_id: session_id.to_string(),
        manifest_id: manifest_id.to_string(),
        sender_id: sender_id.to_string(),
        sequence: sequence as i64,
        transport_tick: transport_tick as i64,
        first_input_tick: first_input_tick as i64,
        confirmed_span: confirmed_span.map(|v| v as i64),
        rows,
    };
    let packet = match kind {
        PacketKind::Guest => input_protocol::new_guest(options),
        PacketKind::Host => input_protocol::new_host(options),
    }
    .map_err(|err| js_err(err.to_string()))?;
    WasmInputPacket::from_packet(packet)
}

/// `InputProtocolPort.newGuest`: builds and validates a single-slot guest
/// bundle. `rows_json` is a JSON array of `{"tick", "slot_index", "sample"}`
/// — see the module doc.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `rows_json` fails to parse/decode, or
/// the resulting packet violates a `gc_netcode::input_protocol::validate`
/// invariant (a malformed field, a non-contiguous history, more than one
/// slot in a guest bundle, ...).
#[wasm_bindgen(js_name = inputProtocolNewGuest)]
#[allow(clippy::too_many_arguments)]
pub fn input_protocol_new_guest(
    session_id: &str,
    manifest_id: &str,
    sender_id: &str,
    sequence: f64,
    transport_tick: f64,
    first_input_tick: f64,
    confirmed_span: Option<f64>,
    rows_json: &str,
) -> Result<WasmInputPacket, JsValue> {
    build(
        PacketKind::Guest,
        session_id,
        manifest_id,
        sender_id,
        sequence,
        transport_tick,
        first_input_tick,
        confirmed_span,
        rows_json,
    )
}

/// The host's canonical, multi-slot authority batch — mirrors
/// `input_protocol.new_host`. Not required by `InputProtocolPort` (only
/// `newGuest`/`encode` are declared there), but the full module's other
/// producing half, at the same cost as [`input_protocol_new_guest`].
///
/// # Errors
///
/// See [`input_protocol_new_guest`].
#[wasm_bindgen(js_name = inputProtocolNewHost)]
#[allow(clippy::too_many_arguments)]
pub fn input_protocol_new_host(
    session_id: &str,
    manifest_id: &str,
    sender_id: &str,
    sequence: f64,
    transport_tick: f64,
    first_input_tick: f64,
    confirmed_span: Option<f64>,
    rows_json: &str,
) -> Result<WasmInputPacket, JsValue> {
    build(
        PacketKind::Host,
        session_id,
        manifest_id,
        sender_id,
        sequence,
        transport_tick,
        first_input_tick,
        confirmed_span,
        rows_json,
    )
}

/// `InputProtocolPort.encode`: this packet's canonical wire bytes. A
/// [`WasmInputPacket`] is always already valid (built by `newGuest`/
/// `newHost`/`decode`/`supersedeForBackpressure`, each of which validates),
/// so this never fails the way the underlying
/// `gc_netcode::input_protocol::encode` can for an arbitrary caller-built
/// packet.
#[wasm_bindgen(js_name = inputProtocolEncode)]
#[must_use]
pub fn input_protocol_encode(packet: &WasmInputPacket) -> Vec<u8> {
    input_protocol::encode(&packet.packet).expect("a WasmInputPacket is always already valid")
}

/// `gc_netcode::input_protocol::decode`: decodes and fully validates a wire
/// packet against its decode context. `wire` is the raw packet bytes (a
/// `Uint8Array` in JS — see the module doc for why never a `string`).
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `wire` fails to decode or does not
/// match `session_id`/`manifest_id`/`sender_id`.
#[wasm_bindgen(js_name = inputProtocolDecode)]
pub fn input_protocol_decode(
    session_id: &str,
    manifest_id: &str,
    sender_id: &str,
    wire: &[u8],
) -> Result<WasmInputPacket, JsValue> {
    let context = input_protocol::DecodeContext {
        session_id: session_id.to_string(),
        manifest_id: manifest_id.to_string(),
        sender_id: sender_id.to_string(),
    };
    let packet = input_protocol::decode(wire, &context).map_err(|err| js_err(err.to_string()))?;
    WasmInputPacket::from_packet(packet)
}

/// `gc_netcode::input_protocol::packet_id`: the stable `fnv1a64` identity a
/// packet with this `kind`/`session_id`/`sender_id`/`sequence` must carry.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `kind` is not `"guest"`/`"host"`, or
/// `session_id`/`sender_id`/`sequence` are out of bounds.
#[wasm_bindgen(js_name = inputProtocolPacketId)]
pub fn input_protocol_packet_id(
    kind: &str,
    session_id: &str,
    sender_id: &str,
    sequence: f64,
) -> Result<String, JsValue> {
    let kind = kind_from_wire(kind)?;
    input_protocol::packet_id(kind, session_id, sender_id, sequence as i64)
        .map_err(|err| js_err(err.to_string()))
}

/// `gc_netcode::input_protocol::confirmed_tick`: the sender's confirmed
/// input tick, recovered from this packet's reported span.
#[wasm_bindgen(js_name = inputProtocolConfirmedTick)]
#[must_use]
pub fn input_protocol_confirmed_tick(packet: &WasmInputPacket) -> f64 {
    input_protocol::confirmed_tick(&packet.packet) as f64
}

/// `gc_netcode::input_protocol::confirmed_span`: the span a sender should
/// report to claim confirmation through `confirmed_tick`.
#[wasm_bindgen(js_name = inputProtocolConfirmedSpan)]
#[must_use]
pub fn input_protocol_confirmed_span(first_input_tick: f64, confirmed_tick: f64) -> f64 {
    input_protocol::confirmed_span(first_input_tick as i64, confirmed_tick as i64) as f64
}

/// `gc_netcode::input_protocol::classify_duplicate`: `"idempotent"` for a
/// byte-identical resend sharing `previous`'s sender/sequence identity.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `previous`/`incoming` do not share a
/// sender/sequence identity, or share one with different bytes (a
/// `packet_conflict` — reported as an error here, mirroring
/// `classify_duplicate`'s own `Result`, never silently coerced to a
/// disposition value).
#[wasm_bindgen(js_name = inputProtocolClassifyDuplicate)]
pub fn input_protocol_classify_duplicate(
    previous: &WasmInputPacket,
    incoming: &WasmInputPacket,
) -> Result<String, JsValue> {
    input_protocol::classify_duplicate(&previous.packet, &incoming.packet)
        .map(|disposition| match disposition {
            input_protocol::DuplicateDisposition::Idempotent => "idempotent".to_string(),
            input_protocol::DuplicateDisposition::Reject => "reject".to_string(),
        })
        .map_err(|err| js_err(err.to_string()))
}

/// `gc_netcode::input_protocol::supersede_for_backpressure`: replaces
/// `older` with `newer` only when `newer` carries every byte of authority
/// `older` did.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if either packet is invalid, or `newer`
/// does not cover every row of unsent authority `older` carried (a
/// `backpressure_gap`).
#[wasm_bindgen(js_name = inputProtocolSupersedeForBackpressure)]
pub fn input_protocol_supersede_for_backpressure(
    older: &WasmInputPacket,
    newer: &WasmInputPacket,
) -> Result<WasmInputPacket, JsValue> {
    let packet = input_protocol::supersede_for_backpressure(&older.packet, &newer.packet)
        .map_err(|err| js_err(err.to_string()))?;
    WasmInputPacket::from_packet(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire(edges: i64) -> String {
        input_frame::encode_sample(&input_frame::InputSample {
            move_x: 0,
            move_y: 0,
            held: 0,
            edges,
        })
        .unwrap()
    }

    fn guest_rows_json(slot_index: i64, edges: i64) -> String {
        let mut items = Vec::new();
        for tick in 0..=input_protocol::HISTORY_ROWS {
            let this_edges = if tick == input_protocol::HISTORY_ROWS {
                edges
            } else {
                0
            };
            items.push(Json::obj(vec![
                ("tick", Json::int(tick)),
                ("slot_index", Json::int(slot_index)),
                ("sample", Json::str(sample_wire(this_edges))),
            ]));
        }
        Json::Array(items).to_json_string()
    }

    #[test]
    fn new_guest_then_encode_then_decode_round_trips() {
        let rows_json = guest_rows_json(3, 1);
        let packet = input_protocol_new_guest(
            "session_1",
            "0123456789abcdef",
            "guest_1",
            0.0,
            6.0,
            0.0,
            None,
            &rows_json,
        )
        .expect("a well-formed guest packet builds");
        assert_eq!(packet.sequence, 0.0);
        assert_eq!(packet.transport_tick, 6.0);
        assert_eq!(packet.kind, "guest");

        let wire = input_protocol_encode(&packet);
        let decoded =
            input_protocol_decode("session_1", "0123456789abcdef", "guest_1", &wire).unwrap();
        assert_eq!(decoded.packet_id, packet.packet_id);
        assert_eq!(decoded.rows_json, packet.rows_json);
    }

    // `newGuest`/`newHost`/`decode`/... error paths return `JsValue`, which
    // `wasm_bindgen::JsValue::from_str` cannot construct off the wasm32
    // target -- aborting the whole native test process (see
    // `crate::coordinator_bridge`'s module doc for the established split).
    // `packages/wasm/src/input_protocol.spec.ts` covers `inputProtocolNewGuest`
    // rejecting a malformed bundle -- including the exact
    // ownership-violation-shaped forgery `net_diagnostics_fixture.ts`'s
    // `forgedBundle` builds downstream of this binding (a single row that
    // does not start at the expected first tick).

    #[test]
    fn packet_id_is_stable_for_the_same_identity() {
        let a = input_protocol_packet_id("guest", "session_1", "guest_1", 0.0).unwrap();
        let b = input_protocol_packet_id("guest", "session_1", "guest_1", 0.0).unwrap();
        assert_eq!(a, b);
        let c = input_protocol_packet_id("guest", "session_1", "guest_1", 1.0).unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn classify_duplicate_reports_idempotent_for_a_byte_identical_resend() {
        let rows_json = guest_rows_json(2, 0);
        let a = input_protocol_new_guest(
            "session_1",
            "0123456789abcdef",
            "guest_1",
            0.0,
            6.0,
            0.0,
            None,
            &rows_json,
        )
        .unwrap();
        let b = input_protocol_new_guest(
            "session_1",
            "0123456789abcdef",
            "guest_1",
            0.0,
            6.0,
            0.0,
            None,
            &rows_json,
        )
        .unwrap();
        assert_eq!(
            input_protocol_classify_duplicate(&a, &b).unwrap(),
            "idempotent"
        );
    }

    #[test]
    fn confirmed_tick_and_span_round_trip() {
        let span = input_protocol_confirmed_span(10.0, 15.0);
        assert_eq!(span, 6.0);
    }

    #[test]
    fn constants_json_reports_the_fairness_delay_and_history() {
        let json = Json::parse(&input_protocol_constants_json()).unwrap();
        assert_eq!(
            json.field_i64("fairness_delay_ticks"),
            Some(input_protocol::FAIRNESS_DELAY_TICKS)
        );
        assert_eq!(
            json.field_i64("history_rows"),
            Some(input_protocol::HISTORY_ROWS)
        );
    }
}
