//! Lobby-facing bindings over `gc_netcode::protocol`'s wire format.
//!
//! See this crate's top-level doc for why this is narrower than "the
//! coordinator": `coordinator::CoordinatorState`/`Event` are not
//! `serde`-shaped and this task does not own `gc-netcode/**` to make them
//! so. What IS bound here is the part of the wire protocol that was already
//! plain text before this crate existed — decoding an inbound control
//! message's routing fields, and reading the protocol's own version
//! identity — which a JS lobby layer needs regardless of how much of the
//! reducer eventually gets a full binding.

use gc_netcode::protocol;
use wasm_bindgen::prelude::*;

/// The routing fields of one decoded control message: enough for a JS lobby
/// layer to dispatch a wire message without needing the full `Value` body
/// bridged.
#[wasm_bindgen]
pub struct ControlMessageHeader {
    #[wasm_bindgen(getter_with_clone)]
    /// The message kind's wire string (e.g. `"propose_manifest"`).
    pub kind: String,
    #[wasm_bindgen(getter_with_clone)]
    /// The session this message belongs to.
    pub session_id: String,
    #[wasm_bindgen(getter_with_clone)]
    /// The peer that authored this message.
    pub peer_id: String,
    /// Monotonic per-peer message sequence.
    pub sequence: f64,
    #[wasm_bindgen(getter_with_clone)]
    /// This message's canonical id.
    pub message_id: String,
}

/// Decode `wire`'s routing header. Does not validate or expose the
/// kind-specific `body` payload.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `wire` is not a canonical control
/// message.
#[wasm_bindgen(js_name = decodeControlMessageHeader)]
pub fn decode_control_message_header(wire: &str) -> Result<ControlMessageHeader, JsValue> {
    let message = protocol::decode(wire).map_err(|err| JsValue::from_str(&err.to_string()))?;
    Ok(ControlMessageHeader {
        kind: message.kind.wire_str().to_string(),
        session_id: message.session_id,
        peer_id: message.peer_id,
        sequence: message.sequence as f64,
        message_id: message.message_id,
    })
}

/// This build's protocol vocabulary digest: every message kind, allowed
/// body field, and lifecycle phase this build recognizes, folded into one
/// stable identity (`gc_netcode::protocol::vocabulary_id`) a lobby can
/// exchange during peer negotiation to detect a version skew before it
/// causes a decode failure mid-session.
#[wasm_bindgen(js_name = protocolVocabularyId)]
#[must_use]
pub fn protocol_vocabulary_id() -> String {
    protocol::vocabulary_id()
}
