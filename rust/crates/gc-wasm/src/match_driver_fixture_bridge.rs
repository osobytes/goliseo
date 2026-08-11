//! `wasm-bindgen` control surface over `gc_netcode::match_driver_fixture` —
//! `packages/online/src/net_diagnostics_fixture.ts`'s `MatchDriverFixturePort`.
//!
//! ## `session`'s transports: a real in-process star, not a stub
//!
//! The Lua/Rust fixture's `session` builds a fully connected in-process star
//! (`gc_netcode::fake_star::FakeStarTransport`, itself a port of
//! `game/transport/fake_star.lua` kept in `gc-netcode` only because
//! `match_driver_fixture.rs` needs a real transport to link — see that
//! module's own doc) — one host endpoint plus one linked guest endpoint per
//! seated guest. [`WasmFakeStarTransport`] wraps that real type, delegating
//! every `StarTransportAdapter` operation straight through rather than
//! reimplementing the fixture's networking, exactly the discipline this
//! crate's doc names for [`crate::match_driver_bridge`] reusing
//! `DriverRules` verbatim.
//!
//! It does **not** literally implement TypeScript's `StarTransportAdapter`
//! interface (`@gc/transport`'s `contract.ts`): that interface's methods
//! return a `TransportResult<T>` discriminated union and never throw, while
//! every fallible method here throws a `JsValue` (a `String`) on error —
//! this crate's uniform convention (`crate::match_driver_bridge`'s
//! `enqueueInbound`, `crate::coordinator_bridge`'s `Coordinator` methods,
//! ...). A caller wanting a real `StarTransportAdapter` needs a thin
//! `try`/`catch` adapter over this class, the same situation this crate's
//! own doc already documents for `MatchDriverBridge` not satisfying
//! `MatchDriverPort` without one. Building that adapter is `@gc/online`'s
//! job, not this crate's — see this crate's final report.
//!
//! `send`/`poll`/`pollBatch` payloads are `Vec<u8>`/`&[u8]` (`Uint8Array` in
//! JS), matching [`crate::match_driver_bridge::MatchDriverBridge::enqueue_inbound`]'s
//! own convention — never `String`. `packages/wasm/src/binary_string.ts` is
//! where a caller converts to/from `@gc/transport`'s "binary string"
//! `TransportMessage.payload` convention; see
//! [`crate::input_protocol_bridge`]'s module doc for the fuller argument.
//!
//! ## `session`'s pure/JSON pieces
//!
//! [`match_driver_fixture_peer_ids`], [`match_driver_fixture_freeze_json`],
//! [`match_driver_fixture_manifest_json`], and
//! [`match_driver_fixture_initial_snapshot`] bind
//! `gc_netcode::match_driver_fixture::peer_ids`/`freeze`/
//! `gc_netcode::protocol_fixture::manifest`/`initial_snapshot` directly,
//! independent of `session`/`WasmFakeStarTransport` — a caller that only
//! needs the frozen ownership plan, its paired manifest, or the
//! boundary-zero snapshot (e.g. to hand to
//! [`crate::match_driver_bridge::MatchDriverBridge::new`] against its own
//! transport) never needs to construct a fixture star at all.
//!
//! This closes a real gap, not a hypothetical one: before this module
//! existed, nothing in `@gc/wasm` could construct the `freezeJson`/
//! `manifestJson` pair `MatchDriverBridge`'s own constructor requires —
//! `Coordinator.proposeManifest` only *validates* a manifest the caller
//! already holds, and there was no binding at all over
//! `match_driver_fixture::freeze`/`session`. [`match_driver_fixture_freeze_json`]
//! and [`match_driver_fixture_manifest_json`] together are exactly that
//! pair (proven end to end by this module's own
//! `freeze_and_manifest_from_this_module_construct_a_real_match_driver_bridge`
//! test), unblocking `packages/online/src/match_presentation.ts`'s and
//! `net_diagnostics.spec.ts`'s driver-backed cases from the TypeScript side.

use gc_netcode::fake_star::FakeStarTransport;
use gc_netcode::fault_transport::{
    StarTransportAdapter, TransportChannelDiagnostics, TransportMessage, TransportPeerDiagnostics,
    TransportPeerEvent, TransportPeerEventKind, TransportStarDiagnostics,
};
use gc_netcode::match_driver_fixture;
use gc_netcode::protocol;
use gc_netcode::protocol_fixture;
use wasm_bindgen::prelude::*;

use crate::coordinator_bridge::{freeze_to_json, value_to_json};
use crate::json::{Json, debug_tag};
use crate::match_driver_bridge::{
    channel_from_str, channel_str, message_kind_from_str, peer_message_to_json,
};
use crate::rollback_events_bridge::WasmMatchSnapshot;

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

fn mode_from_wire(text: &str) -> Result<protocol::MatchMode, JsValue> {
    protocol::MatchMode::from_wire_str(text)
        .ok_or_else(|| js_err(format!("'{text}' is not a canonical match mode")))
}

/// This fixture's own constants, as JSON: `host_peer_id`, `countdown_id`,
/// `default_duration`, `default_seed`.
#[wasm_bindgen(js_name = matchDriverFixtureConstantsJson)]
#[must_use]
pub fn match_driver_fixture_constants_json() -> String {
    Json::obj(vec![
        (
            "host_peer_id",
            Json::str(match_driver_fixture::HOST_PEER_ID),
        ),
        (
            "countdown_id",
            Json::str(match_driver_fixture::COUNTDOWN_ID),
        ),
        (
            "default_duration",
            Json::Number(match_driver_fixture::DEFAULT_DURATION),
        ),
        (
            "default_seed",
            Json::Number(match_driver_fixture::DEFAULT_SEED),
        ),
    ])
    .to_json_string()
}

/// Mirrors `fixture.guest_peer_id`.
#[wasm_bindgen(js_name = matchDriverFixtureGuestPeerId)]
#[must_use]
pub fn match_driver_fixture_guest_peer_id(index: f64) -> String {
    match_driver_fixture::guest_peer_id(index as i64)
}

/// Mirrors `fixture.peer_ids`: every peer id this session seats, host first.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `mode_wire` is not a canonical match
/// mode.
///
/// # Panics
///
/// Panics if `humans` is outside the mode's supported human count —
/// mirrors the underlying fixture's own `assert!` (a caller-side invariant,
/// not a recoverable input error; see `match_driver_fixture::peer_ids`'s
/// doc).
#[wasm_bindgen(js_name = matchDriverFixturePeerIds)]
pub fn match_driver_fixture_peer_ids(
    mode_wire: &str,
    humans: Option<f64>,
) -> Result<Vec<String>, JsValue> {
    let mode = mode_from_wire(mode_wire)?;
    Ok(match_driver_fixture::peer_ids(
        mode,
        humans.map(|value| value as i64),
    ))
}

/// Mirrors `fixture.freeze`, as JSON
/// (`crate::coordinator_bridge::freeze_to_json`'s shape).
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `mode_wire` is not a canonical match
/// mode.
#[wasm_bindgen(js_name = matchDriverFixtureFreezeJson)]
pub fn match_driver_fixture_freeze_json(
    mode_wire: &str,
    first_input_tick: Option<f64>,
    humans: Option<f64>,
) -> Result<String, JsValue> {
    let mode = mode_from_wire(mode_wire)?;
    let freeze = match_driver_fixture::freeze(
        mode,
        first_input_tick.map(|value| value as i64),
        humans.map(|value| value as i64),
    );
    Ok(freeze_to_json(&freeze).to_json_string())
}

/// The frozen session manifest a [`match_driver_fixture_freeze_json`] call
/// for the same `mode_wire` is framed against (`gc_netcode::protocol_fixture::manifest`),
/// as JSON (`crate::coordinator_bridge::value_to_json`'s array/object rule)
/// — together, the exact `freezeJson`/`manifestJson` pair
/// [`crate::match_driver_bridge::MatchDriverBridge::new`] requires,
/// without needing to also construct a full [`WasmMatchDriverFixtureSession`]
/// (transports and all). `WasmMatchDriverFixtureSession::manifestJson`/
/// `freezeJson` return this exact same pair when a caller *does* want the
/// full connected session too — the two never drift, since both go through
/// `gc_netcode::match_driver_fixture::freeze`/`gc_netcode::protocol_fixture::manifest`
/// directly.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `mode_wire` is not a canonical match
/// mode.
#[wasm_bindgen(js_name = matchDriverFixtureManifestJson)]
pub fn match_driver_fixture_manifest_json(mode_wire: &str) -> Result<String, JsValue> {
    let mode = mode_from_wire(mode_wire)?;
    Ok(value_to_json(&protocol_fixture::manifest(Some(mode))).to_json_string())
}

/// Mirrors `fixture.initial_snapshot`: the pinned slot-mode boundary zero,
/// as an opaque handle (never JSON — see
/// `crate::rollback_events_bridge::WasmMatchSnapshot`'s doc).
#[wasm_bindgen(js_name = matchDriverFixtureInitialSnapshot)]
#[must_use]
pub fn match_driver_fixture_initial_snapshot(
    duration: Option<f64>,
    combat_active: Option<bool>,
    seed: Option<f64>,
) -> WasmMatchSnapshot {
    WasmMatchSnapshot::new(match_driver_fixture::initial_snapshot(
        duration,
        combat_active.unwrap_or(false),
        seed,
    ))
}

// ---------------------------------------------------------------------------
// `WasmFakeStarTransport` — see the module doc.
// ---------------------------------------------------------------------------

fn peer_event_kind_str(kind: TransportPeerEventKind) -> String {
    debug_tag(&kind)
}

fn peer_event_to_json(event: &TransportPeerEvent) -> Json {
    Json::obj(vec![
        ("kind", Json::str(peer_event_kind_str(event.kind))),
        ("peer_id", Json::opt_str(event.peer_id.as_deref())),
        (
            "channel",
            event
                .channel
                .map_or(Json::Null, |channel| Json::str(channel_str(channel))),
        ),
        (
            "state",
            event
                .state
                .map_or(Json::Null, |state| Json::str(debug_tag(&state))),
        ),
        (
            "code",
            event
                .code
                .map_or(Json::Null, |code| Json::str(debug_tag(&code))),
        ),
        ("message", Json::opt_str(event.message.as_deref())),
    ])
}

fn channel_diagnostics_to_json(diagnostics: &TransportChannelDiagnostics) -> Json {
    Json::obj(vec![
        (
            "state",
            diagnostics
                .state
                .map_or(Json::Null, |state| Json::str(debug_tag(&state))),
        ),
        ("outbound_depth", Json::int(diagnostics.outbound_depth)),
        ("inbound_depth", Json::int(diagnostics.inbound_depth)),
        ("buffered_amount", Json::int(diagnostics.buffered_amount)),
        ("sent", Json::int(diagnostics.sent)),
        ("received", Json::int(diagnostics.received)),
        ("dropped_outbound", Json::int(diagnostics.dropped_outbound)),
        ("dropped_inbound", Json::int(diagnostics.dropped_inbound)),
    ])
}

fn peer_diagnostics_to_json(diagnostics: &TransportPeerDiagnostics) -> Json {
    Json::obj(vec![
        ("peer_id", Json::str(diagnostics.peer_id.clone())),
        ("slot", Json::int(diagnostics.slot)),
        (
            "state",
            diagnostics
                .state
                .map_or(Json::Null, |state| Json::str(debug_tag(&state))),
        ),
        ("ice_state", Json::str(diagnostics.ice_state.clone())),
        ("control", channel_diagnostics_to_json(&diagnostics.control)),
        ("input", channel_diagnostics_to_json(&diagnostics.input)),
        ("sequence_gaps", Json::int(diagnostics.sequence_gaps)),
        ("backpressure", Json::int(diagnostics.backpressure)),
        ("malformed", Json::int(diagnostics.malformed)),
        (
            "last_error",
            Json::opt_str(diagnostics.last_error.as_deref()),
        ),
    ])
}

/// `pub(crate)`, not private: [`crate::match_driver_bridge`]'s
/// `transportDiagnosticsJson` reuses this exact shape for
/// `MatchDriverBridge`'s own wrapped `WasmStarTransport` — the
/// `TransportStarDiagnostics`-shaped data `net_diagnostics.spec.ts`'s
/// header comment names as absent from that bridge's surface — rather than
/// restating this mapping a second time.
pub(crate) fn star_diagnostics_to_json(diagnostics: &TransportStarDiagnostics) -> Json {
    Json::obj(vec![
        (
            "role",
            diagnostics
                .role
                .map_or(Json::Null, |role| Json::str(debug_tag(&role))),
        ),
        (
            "state",
            diagnostics
                .state
                .map_or(Json::Null, |state| Json::str(debug_tag(&state))),
        ),
        ("capacity", Json::int(diagnostics.capacity)),
        ("peer_count", Json::int(diagnostics.peer_count)),
        ("queue_limit", Json::int(diagnostics.queue_limit)),
        (
            "buffered_amount_limit",
            Json::int(diagnostics.buffered_amount_limit),
        ),
        ("event_depth", Json::int(diagnostics.event_depth)),
        ("sent", Json::int(diagnostics.sent)),
        ("received", Json::int(diagnostics.received)),
        ("dropped_outbound", Json::int(diagnostics.dropped_outbound)),
        ("dropped_inbound", Json::int(diagnostics.dropped_inbound)),
        ("malformed", Json::int(diagnostics.malformed)),
        (
            "unsupported_version",
            Json::int(diagnostics.unsupported_version),
        ),
        ("overflow", Json::int(diagnostics.overflow)),
        ("backpressure", Json::int(diagnostics.backpressure)),
        (
            "last_error",
            Json::opt_str(diagnostics.last_error.as_deref()),
        ),
        (
            "peers",
            Json::Array(
                diagnostics
                    .peers
                    .iter()
                    .map(peer_diagnostics_to_json)
                    .collect(),
            ),
        ),
    ])
}

/// A real in-process star endpoint (`gc_netcode::fake_star::FakeStarTransport`).
/// See the module doc for how closely this mirrors (and does not literally
/// implement) `@gc/transport`'s `StarTransportAdapter`.
///
/// Cheap `Clone`-backed handle, like the wrapped type itself: obtained from
/// [`WasmMatchDriverFixtureSession::host_transport`]/
/// [`WasmMatchDriverFixtureSession::guest_transport`], never constructed
/// directly — a standalone, unlinked star is not a fixture this port needs
/// to expose.
#[wasm_bindgen]
pub struct WasmFakeStarTransport {
    inner: FakeStarTransport,
}

impl WasmFakeStarTransport {
    fn new(inner: FakeStarTransport) -> Self {
        WasmFakeStarTransport { inner }
    }
}

#[wasm_bindgen]
impl WasmFakeStarTransport {
    /// Brings this endpoint up. Mirrors `StarTransportAdapter.initialize`.
    /// The fixture's own `session` already initializes and links every
    /// endpoint it hands back, so a caller driving the fixture harness never
    /// needs to call this — it exists for completeness and for a caller that
    /// wants a fresh, unlinked pair via `gc_netcode::fake_star` semantics.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) on failure.
    pub fn initialize(&mut self) -> Result<(), JsValue> {
        self.inner
            .initialize()
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Mirrors `StarTransportAdapter.shutdown`.
    ///
    /// # Errors
    ///
    /// See [`WasmFakeStarTransport::initialize`].
    pub fn shutdown(&mut self) -> Result<(), JsValue> {
        self.inner
            .shutdown()
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// `"host"` or `"guest"`.
    #[must_use]
    pub fn role(&self) -> String {
        match self.inner.role() {
            gc_netcode::fault_transport::TransportRole::Host => "host".to_string(),
            gc_netcode::fault_transport::TransportRole::Guest => "guest".to_string(),
        }
    }

    /// This endpoint's guest capacity (always 1 on a guest endpoint).
    #[must_use]
    pub fn capacity(&self) -> f64 {
        self.inner.capacity() as f64
    }

    /// Allocates a slot for `peer_id`. Returns the assigned slot number.
    ///
    /// # Errors
    ///
    /// See [`WasmFakeStarTransport::initialize`].
    #[wasm_bindgen(js_name = openPeer)]
    pub fn open_peer(&mut self, peer_id: &str) -> Result<f64, JsValue> {
        self.inner
            .open_peer(peer_id)
            .map(|slot| slot as f64)
            .map_err(|err| js_err(err.message))
    }

    /// Closes `peer_id`'s slot, disconnecting the endpoint linked to it too.
    ///
    /// # Errors
    ///
    /// See [`WasmFakeStarTransport::initialize`].
    #[wasm_bindgen(js_name = closePeer)]
    pub fn close_peer(&mut self, peer_id: &str, reason: Option<String>) -> Result<(), JsValue> {
        self.inner
            .close_peer(peer_id, reason.as_deref())
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Every currently open peer id, in slot order.
    #[wasm_bindgen(js_name = peerIds)]
    #[must_use]
    pub fn peer_ids(&self) -> Vec<String> {
        self.inner.peer_ids()
    }

    /// `peer_id`'s connection state, or `undefined` if `peer_id` is not
    /// open.
    #[wasm_bindgen(js_name = peerState)]
    #[must_use]
    pub fn peer_state(&self, peer_id: &str) -> Option<String> {
        self.inner
            .peer_state(peer_id)
            .map(|state| debug_tag(&state))
    }

    /// Test seam: joins this (host) endpoint to `guest`'s already-open slot.
    /// The browser adapter reaches the same state through manual
    /// offer/answer exchange; the fixture's own `session` already calls
    /// this internally for every seated guest.
    ///
    /// # Errors
    ///
    /// See [`WasmFakeStarTransport::initialize`].
    pub fn link(&mut self, guest: &WasmFakeStarTransport) -> Result<(), JsValue> {
        self.inner
            .link(&guest.inner)
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Test seam: delivers everything currently buffered on this endpoint
    /// and every endpoint linked to it. Mirrors
    /// `FakeStarTransport::pump`/`net_diagnostics_fixture.ts`'s own `pump`
    /// helper.
    pub fn pump(&self) {
        self.inner.pump();
    }

    /// Starts an offer/answer exchange for `peer_id` (host only). Returns
    /// `true`; retrieve the actual signal blob with
    /// [`WasmFakeStarTransport::take_signal`].
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if this endpoint is not a host, or
    /// `peer_id` is not open.
    #[wasm_bindgen(js_name = requestOffer)]
    pub fn request_offer(&mut self, peer_id: &str) -> Result<(), JsValue> {
        self.inner
            .request_offer(peer_id)
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Accepts a host's offer signal (guest only).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if this endpoint is not a guest, or
    /// `signal` names no pending offer.
    #[wasm_bindgen(js_name = acceptOffer)]
    pub fn accept_offer(&mut self, signal: &str) -> Result<(), JsValue> {
        self.inner
            .accept_offer(signal)
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Accepts a guest's answer signal for `peer_id` (host only) — links the
    /// two endpoints on success, the same as [`WasmFakeStarTransport::link`].
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if this endpoint is not a host, or
    /// `signal` does not match a pending offer for `peer_id`.
    #[wasm_bindgen(js_name = acceptAnswer)]
    pub fn accept_answer(&mut self, peer_id: &str, signal: &str) -> Result<(), JsValue> {
        self.inner
            .accept_answer(peer_id, signal)
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Takes (and clears) `peer_id`'s pending outbound signal blob, if any.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `peer_id` is not open.
    #[wasm_bindgen(js_name = takeSignal)]
    pub fn take_signal(&mut self, peer_id: &str) -> Result<Option<String>, JsValue> {
        self.inner
            .take_signal(peer_id)
            .map_err(|err| js_err(err.message))
    }

    /// Sends one envelope to `peer_id`. `channel` is `"control"`/`"input"`;
    /// `kind` is `"input"`/`"event"`/`"state"`; `payload` is the raw message
    /// bytes (a `Uint8Array` — see the module doc).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `channel`/`kind` are
    /// unrecognized, `peer_id` is not an open, connected peer, or the
    /// message fails this transport's structural checks.
    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &mut self,
        peer_id: &str,
        channel: &str,
        kind: &str,
        seq: f64,
        tick: Option<f64>,
        payload: &[u8],
    ) -> Result<(), JsValue> {
        let channel = channel_from_str(channel).map_err(js_err)?;
        let kind = message_kind_from_str(kind).map_err(js_err)?;
        let message = TransportMessage {
            version: 1,
            kind,
            seq: seq as i64,
            tick: tick.map(|value| value as i64),
            payload: payload.to_vec(),
        };
        self.inner
            .send(peer_id, channel, message)
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Fans one envelope out to every connected peer (host only). Returns
    /// how many peers received it.
    ///
    /// # Errors
    ///
    /// See [`WasmFakeStarTransport::send`].
    pub fn broadcast(
        &mut self,
        channel: &str,
        kind: &str,
        seq: f64,
        tick: Option<f64>,
        payload: &[u8],
    ) -> Result<f64, JsValue> {
        let channel = channel_from_str(channel).map_err(js_err)?;
        let kind = message_kind_from_str(kind).map_err(js_err)?;
        let message = TransportMessage {
            version: 1,
            kind,
            seq: seq as i64,
            tick: tick.map(|value| value as i64),
            payload: payload.to_vec(),
        };
        self.inner
            .broadcast(channel, message)
            .map(|count| count as f64)
            .map_err(|err| js_err(err.message))
    }

    /// Removes and returns up to one queued arrival, as JSON
    /// (`crate::match_driver_bridge`'s `peer_message_to_json` shape), or
    /// `null` if nothing is queued.
    #[wasm_bindgen(js_name = pollJson)]
    #[must_use]
    pub fn poll_json(&mut self) -> String {
        self.inner
            .poll()
            .map_or(Json::Null, |message| peer_message_to_json(&message))
            .to_json_string()
    }

    /// Removes and returns up to `limit` queued arrivals (default: the
    /// transport's own batch default), as a JSON array, oldest first.
    #[wasm_bindgen(js_name = pollBatchJson)]
    #[must_use]
    pub fn poll_batch_json(&mut self, limit: Option<f64>) -> String {
        Json::Array(
            self.inner
                .poll_batch(limit.map(|value| value as i64))
                .iter()
                .map(peer_message_to_json)
                .collect(),
        )
        .to_json_string()
    }

    /// Removes and returns up to one queued peer/star event, as JSON, or
    /// `null` if nothing is queued.
    #[wasm_bindgen(js_name = pollEventJson)]
    #[must_use]
    pub fn poll_event_json(&mut self) -> String {
        self.inner
            .poll_event()
            .as_ref()
            .map_or(Json::Null, peer_event_to_json)
            .to_json_string()
    }

    /// This endpoint's own connection state.
    #[must_use]
    pub fn state(&self) -> String {
        debug_tag(&self.inner.state())
    }

    /// A full read of this endpoint's counters and every peer's, as JSON.
    #[wasm_bindgen(js_name = diagnosticsJson)]
    #[must_use]
    pub fn diagnostics_json(&self) -> String {
        star_diagnostics_to_json(&self.inner.diagnostics()).to_json_string()
    }
}

// ---------------------------------------------------------------------------
// `WasmMatchDriverFixtureSession` — `MatchDriverFixturePort.session`.
// ---------------------------------------------------------------------------

/// Mirrors `fixture.session`: the full connected session
/// (`MatchDriverFixturePort.session`'s return). See the module doc for
/// `hostTransport`/`guestTransport`'s relationship to `StarTransportAdapter`.
#[wasm_bindgen]
pub struct WasmMatchDriverFixtureSession {
    inner: match_driver_fixture::MatchDriverFixtureSession,
}

#[wasm_bindgen]
impl WasmMatchDriverFixtureSession {
    /// This session's match mode wire string (`"1v1"`/`"2v2"`/`"4v4"`).
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn mode(&self) -> String {
        self.inner.mode.wire_str().to_string()
    }

    /// The frozen session manifest, as JSON
    /// (`crate::coordinator_bridge::value_to_json`'s array/object rule).
    #[wasm_bindgen(js_name = manifestJson)]
    #[must_use]
    pub fn manifest_json(&self) -> String {
        value_to_json(&self.inner.manifest).to_json_string()
    }

    /// The frozen session state, as JSON (`freeze_to_json`'s shape) —
    /// exactly what [`crate::match_driver_bridge::MatchDriverBridge::new`]'s
    /// `freeze_json` parameter expects.
    #[wasm_bindgen(js_name = freezeJson)]
    #[must_use]
    pub fn freeze_json(&self) -> String {
        freeze_to_json(&self.inner.freeze).to_json_string()
    }

    /// The host's own peer id.
    #[wasm_bindgen(getter, js_name = hostPeerId)]
    #[must_use]
    pub fn host_peer_id(&self) -> String {
        self.inner.host_peer_id.clone()
    }

    /// Every seated guest's peer id, in seating order.
    #[wasm_bindgen(js_name = guestPeerIds)]
    #[must_use]
    pub fn guest_peer_ids(&self) -> Vec<String> {
        self.inner.guest_peer_ids.clone()
    }

    /// The host endpoint of this session's in-process star.
    #[wasm_bindgen(js_name = hostTransport)]
    #[must_use]
    pub fn host_transport(&self) -> WasmFakeStarTransport {
        WasmFakeStarTransport::new(self.inner.host_transport.clone())
    }

    /// `peer_id`'s guest endpoint, or `undefined` if `peer_id` did not seat
    /// as a guest in this session.
    #[wasm_bindgen(js_name = guestTransport)]
    #[must_use]
    pub fn guest_transport(&self, peer_id: &str) -> Option<WasmFakeStarTransport> {
        self.inner
            .guest_transports
            .get(peer_id)
            .cloned()
            .map(WasmFakeStarTransport::new)
    }
}

/// `MatchDriverFixturePort.session`: builds the full connected fixture
/// session for `mode_wire`. `humans` defaults to a full lobby — see
/// `gc_netcode::match_driver_fixture::peer_ids`'s doc.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `mode_wire` is not a canonical match
/// mode.
///
/// # Panics
///
/// Panics on the same fixture-invariant conditions
/// `gc_netcode::match_driver_fixture::session` itself panics on (never
/// expected to fail against real content).
#[wasm_bindgen(js_name = matchDriverFixtureSession)]
pub fn match_driver_fixture_session(
    mode_wire: &str,
    first_input_tick: Option<f64>,
    humans: Option<f64>,
) -> Result<WasmMatchDriverFixtureSession, JsValue> {
    let mode = mode_from_wire(mode_wire)?;
    let inner = match_driver_fixture::session(
        mode,
        first_input_tick.map(|value| value as i64),
        humans.map(|value| value as i64),
    );
    Ok(WasmMatchDriverFixtureSession { inner })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_ids_seats_the_host_first() {
        let ids = match_driver_fixture_peer_ids("1v1", None).unwrap();
        assert_eq!(ids[0], "host");
        assert_eq!(ids.len(), 2);
    }

    // `peer_ids`/`freeze_json`/`manifest_json`/`session`'s error paths
    // return `JsValue`, which `wasm_bindgen::JsValue::from_str` cannot
    // construct off the wasm32 target -- aborting the whole native test
    // process (see `crate::coordinator_bridge`'s module doc for the
    // established split). `packages/wasm/src/match_driver_fixture.spec.ts`
    // covers `matchDriverFixturePeerIds`/`FreezeJson`/`ManifestJson`/`Session`
    // rejecting an unrecognized match mode, and the constructor-panic
    // regression this wave's brief called out.

    #[test]
    fn freeze_json_reports_a_contiguous_owned_block_per_human() {
        let json =
            Json::parse(&match_driver_fixture_freeze_json("2v2", None, None).unwrap()).unwrap();
        let owned = json.get("owned").unwrap();
        assert!(matches!(owned, Json::Object(fields) if fields.len() == 4));
    }

    #[test]
    fn session_links_every_seated_guest_to_the_host() {
        let session = match_driver_fixture_session("2v2", None, None).unwrap();
        let guest_ids = session.guest_peer_ids();
        assert_eq!(guest_ids.len(), 3);
        let host_transport = session.host_transport();
        for guest_id in &guest_ids {
            assert_eq!(
                host_transport.peer_state(guest_id),
                Some("connected".to_string())
            );
            let guest_transport = session.guest_transport(guest_id).expect("guest seated");
            assert_eq!(
                guest_transport.peer_state("host"),
                Some("connected".to_string())
            );
        }
        assert!(session.guest_transport("nobody").is_none());
    }

    #[test]
    fn send_then_pump_then_poll_delivers_across_the_link() {
        let session = match_driver_fixture_session("1v1", None, None).unwrap();
        let mut host_transport = session.host_transport();
        let guest_id = session.guest_peer_ids()[0].clone();
        let mut guest_transport = session.guest_transport(&guest_id).unwrap();

        host_transport
            .send(&guest_id, "input", "input", 0.0, Some(0.0), b"hello")
            .expect("a connected peer accepts a structurally valid input envelope");
        host_transport.pump();

        let batch_json = guest_transport.poll_batch_json(None);
        let batch = Json::parse(&batch_json).unwrap();
        let items = batch.as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].field_str("peer_id"), Some("host"));
    }

    #[test]
    fn initial_snapshot_returns_an_opaque_handle() {
        let snapshot = match_driver_fixture_initial_snapshot(None, None, None);
        // Opaque: nothing to assert on its content, only that it constructs
        // without panicking and can be dropped cleanly.
        drop(snapshot);
    }

    #[test]
    fn constants_json_reports_the_host_peer_id() {
        let json = Json::parse(&match_driver_fixture_constants_json()).unwrap();
        assert_eq!(json.field_str("host_peer_id"), Some("host"));
    }

    /// Proves the orchestrator-flagged gap is actually closed, end to end,
    /// through the real `MatchDriverBridge` constructor -- not just that
    /// this module's own JSON happens to parse. Before
    /// [`match_driver_fixture_freeze_json`]/[`match_driver_fixture_manifest_json`]
    /// existed, nothing in `@gc/wasm` could produce this pair at all (see
    /// the module doc).
    #[test]
    fn freeze_and_manifest_from_this_module_construct_a_real_match_driver_bridge() {
        use crate::session::Session;

        let freeze_json = match_driver_fixture_freeze_json("1v1", None, None).unwrap();
        let manifest_json = match_driver_fixture_manifest_json("1v1").unwrap();
        let session = Session::new(
            "nebula", "orion", 7.0, 20.0, 3, None, None, None, None, None,
        )
        .expect("the fixture team ids always construct a valid session");

        let bridge = crate::match_driver_bridge::MatchDriverBridge::new(
            &session,
            "host",
            match_driver_fixture::HOST_PEER_ID,
            &freeze_json,
            &manifest_json,
            None,
            None,
            None,
        );
        assert!(
            bridge.is_ok(),
            "a freeze/manifest pair built entirely from this module's own \
             bindings must construct a valid MatchDriverBridge"
        );
    }
}
