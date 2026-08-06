//! `wasm-bindgen` control surface over `gc_netcode::match_driver` (the
//! OMP-3 online match driver) and its `gc_sim::rollback_events` feed.
//!
//! ## Reusing `match_driver_fixture::DriverRules`, not reimplementing it
//!
//! `MatchDriver` is built with an injected `Box<dyn MatchDriverRules>` —
//! `match_driver.rs`'s own doc explains why: `carrier`/`transition`/
//! `next_live_slot`/`slot_drivers` are real business logic
//! (`gc_netcode::live_slot`/`gc_netcode::coordinator`) the driver's own file
//! must not reimplement, and `canonical_host_batch` needs `protocol.lua`
//! machinery (`gc_netcode::protocol`, `gc_netcode::input_protocol`) that
//! file was built without. `gc_netcode::match_driver_fixture::DriverRules`
//! already closes every one of those gaps for real — its own doc says
//! plainly it is "the real `MatchDriverRules` implementation," not a test
//! double, despite the file it lives in being named `_fixture`. This bridge
//! reuses it verbatim ([`DriverRules::new`]) rather than writing a second
//! implementation that would either duplicate that logic or, worse, drift
//! from it.
//!
//! ## The queue/drain seam
//!
//! [`crate::wasm_transport::WasmStarTransport`] is the
//! `StarTransportAdapter` [`MatchDriver`] polls internally, once per driver
//! step, inside [`MatchDriverBridge::advance`] — see that module's doc and
//! [`crate::net_inbox`] for the discipline. [`MatchDriverBridge::enqueue_inbound`]
//! is the only network-facing entry point on this bridge; it only ever
//! appends to the transport's inbound queue.
//!
//! ## Why a `MatchSnapshot` never crosses to JS
//!
//! [`MatchDriverBridge::new`] takes a [`Session`] (this crate's own offline
//! session type) instead of raw team/roster parameters: it reuses
//! [`Session::capture_snapshot`] to get a real boundary-zero
//! `gc_sim::match_snapshot::MatchSnapshot`, and that snapshot never leaves
//! Rust. `gc_netcode::match_session.lua`'s own module doc explains why full
//! online content resolution (arena/loadout/roster identity resolved from a
//! manifest's `content_id`) needs `gc-data`, which `gc-netcode` deliberately
//! does not depend on; wiring that up generically is a separate task from
//! "bridge the reducers wasm needs." Reusing `Session`'s already-tested
//! team/ownership construction is the smaller, correct-today alternative:
//! it is exactly what an online match needs (a slot-mode, boundary-zero
//! `MatchState`), built the same way the existing offline path already
//! builds and tests it.
//!
//! ## `rollback_events`: fed automatically, but only for the safe case
//!
//! [`MatchDriverBridge::advance`] feeds each driver step's output into an
//! internally-owned `gc_sim::rollback_events::RollbackEventTimeline`
//! *only* when that step produced exactly one new, in-order tick (no
//! rollback, no correction) — see [`MatchDriverBridge::feed_rollback_timeline`].
//! `rollback_events::apply`'s contract requires a caller to know precisely
//! which interval a correction replaces (`replaced_from_tick`/
//! `replaced_through_tick`, matching the *complete* stale speculative tail)
//! — that bookkeeping lives one layer above `match_driver.rs` in the Lua
//! original (`match_presentation.lua`, TypeScript-owned per `v2/README.md`
//! §2, not ported into any crate this wave owns), so guessing at it here
//! risks feeding `apply` an interval it was never designed to validate.
//! Getting it wrong is an assertion panic, not a silent bug — so this
//! bridge simply does not attempt the general case: a rollback/correction
//! batch is reported in `advance`'s JSON (`"rollback_events_fed": false`)
//! and skipped, and once skipped the feed cannot resynchronize (documented
//! in [`MatchDriverBridge::feed_rollback_timeline`]). This is a real,
//! working feed for the common (no-desync) path, not a stub, and its limit
//! is reported honestly rather than papered over — see this crate's report
//! for the follow-up this leaves.

use gc_netcode::coordinator::Freeze;
use gc_netcode::fault_transport::{
    StarTransportAdapter, TransportChannel, TransportMessage, TransportMessageType,
    TransportPeerMessage,
};
use gc_netcode::match_driver::{
    self, MatchDriver, MatchDriverBatch, MatchDriverCheckpoint, MatchDriverDiagnostics,
    MatchDriverOptions, MatchDriverTerminal,
};
use gc_netcode::match_driver_fixture::{DriverRules, to_driver_freeze, to_driver_manifest};
use gc_netcode::protocol::{self, Value};
use gc_sim::input_frame::{self, SlotId};
use gc_sim::rollback_events::{self, RollbackEventTimeline};
use gc_sim::rollback_session::RollbackTickOutput;
use indexmap::IndexMap;
use wasm_bindgen::prelude::*;

use crate::coordinator_bridge::{freeze_from_json, value_from_json};
use crate::json::{Json, debug_tag};
use crate::rollback_events_bridge;
use crate::session::Session;
use crate::wasm_transport::{OutboundEnvelope, WasmStarTransport};

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

fn parse_json(text: &str) -> Result<Json, JsValue> {
    Json::parse(text).map_err(js_err)
}

fn channel_str(channel: TransportChannel) -> &'static str {
    match channel {
        TransportChannel::Control => "control",
        TransportChannel::Input => "input",
    }
}

fn channel_from_str(text: &str) -> Result<TransportChannel, String> {
    match text {
        "control" => Ok(TransportChannel::Control),
        "input" => Ok(TransportChannel::Input),
        other => Err(format!("unknown transport channel '{other}'")),
    }
}

fn message_kind_str(kind: TransportMessageType) -> &'static str {
    match kind {
        TransportMessageType::Input => "input",
        TransportMessageType::Event => "event",
        TransportMessageType::State => "state",
    }
}

fn message_kind_from_str(text: &str) -> Result<TransportMessageType, String> {
    match text {
        "input" => Ok(TransportMessageType::Input),
        "event" => Ok(TransportMessageType::Event),
        "state" => Ok(TransportMessageType::State),
        other => Err(format!("unknown transport message kind '{other}'")),
    }
}

fn transport_message_to_json(message: &TransportMessage) -> Json {
    Json::obj(vec![
        ("version", Json::int(message.version)),
        ("kind", Json::str(message_kind_str(message.kind))),
        ("seq", Json::int(message.seq)),
        ("tick", Json::opt_int(message.tick)),
        (
            "payload_bytes",
            Json::Array(
                message
                    .payload
                    .iter()
                    .map(|byte| Json::int(i64::from(*byte)))
                    .collect(),
            ),
        ),
        (
            "payload_text",
            match std::str::from_utf8(&message.payload) {
                Ok(text) => Json::str(text),
                Err(_) => Json::Null,
            },
        ),
    ])
}

fn peer_message_to_json(message: &TransportPeerMessage) -> Json {
    Json::obj(vec![
        ("peer_id", Json::str(message.peer_id.clone())),
        ("channel", Json::str(channel_str(message.channel))),
        ("message", transport_message_to_json(&message.message)),
        ("arrival_seq", Json::int(message.arrival_seq)),
    ])
}

fn outbound_envelope_to_json(envelope: &OutboundEnvelope) -> Json {
    Json::obj(vec![
        ("peer_id", Json::str(envelope.peer_id.clone())),
        ("channel", Json::str(channel_str(envelope.channel))),
        ("message", transport_message_to_json(&envelope.message)),
    ])
}

fn slot_map_to_json(entries: &IndexMap<String, SlotId>) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(peer_id, slot)| (peer_id.clone(), Json::str(protocol::slot_wire_id(*slot))))
            .collect(),
    )
}

fn checkpoint_to_json(checkpoint: &MatchDriverCheckpoint) -> Json {
    Json::obj(vec![
        ("tick", Json::int(checkpoint.tick)),
        ("hash", Json::str(checkpoint.hash.clone())),
        ("live", slot_map_to_json(&checkpoint.live)),
    ])
}

fn output_summary_to_json(output: &RollbackTickOutput) -> Json {
    Json::obj(vec![
        ("tick", Json::int(output.tick)),
        ("start_boundary", Json::int(output.start_boundary)),
        ("end_boundary", Json::int(output.end_boundary)),
        ("finished", Json::bool(output.finished)),
        (
            "score",
            Json::obj(vec![
                ("home", Json::int(output.state.score.home)),
                ("away", Json::int(output.state.score.away)),
            ]),
        ),
        ("time_left", Json::Number(output.state.time_left)),
        ("event_count", Json::int(output.events.len() as i64)),
        (
            "combat_event_count",
            Json::opt_int(
                output
                    .combat_events
                    .as_ref()
                    .map(|events| events.len() as i64),
            ),
        ),
    ])
}

fn driver_terminal_to_json(terminal: &MatchDriverTerminal) -> Json {
    Json::obj(vec![
        ("status", Json::str(debug_tag(&terminal.status))),
        (
            "failure",
            terminal
                .failure
                .map_or(Json::Null, |failure| Json::str(debug_tag(&failure))),
        ),
        ("detail", Json::str(terminal.detail.clone())),
        ("tick", Json::opt_int(terminal.tick)),
    ])
}

fn diagnostics_to_json(diagnostics: &MatchDriverDiagnostics) -> Json {
    Json::obj(vec![
        (
            "snapshot_captures",
            Json::int(diagnostics.snapshot_captures),
        ),
        ("role", Json::str(debug_tag(&diagnostics.role))),
        ("peer_id", Json::str(diagnostics.peer_id.clone())),
        ("status", Json::str(debug_tag(&diagnostics.status))),
        (
            "terminal",
            diagnostics
                .terminal
                .as_ref()
                .map_or(Json::Null, driver_terminal_to_json),
        ),
        ("step", Json::int(diagnostics.step)),
        ("transport_tick", Json::int(diagnostics.transport_tick)),
        (
            "present_input_tick",
            Json::int(diagnostics.present_input_tick),
        ),
        (
            "confirmed_input_tick",
            Json::int(diagnostics.confirmed_input_tick),
        ),
        (
            "confirmed_output_tick",
            Json::int(diagnostics.confirmed_output_tick),
        ),
        (
            "retained_floor_tick",
            Json::int(diagnostics.retained_floor_tick),
        ),
        (
            "owned",
            Json::Array(
                diagnostics
                    .owned
                    .iter()
                    .map(|slot| Json::str(protocol::slot_wire_id(*slot)))
                    .collect(),
            ),
        ),
        (
            "authored",
            Json::Array(
                diagnostics
                    .authored
                    .iter()
                    .map(|slot| Json::str(protocol::slot_wire_id(*slot)))
                    .collect(),
            ),
        ),
        ("live", slot_map_to_json(&diagnostics.live)),
        (
            "control_slot",
            diagnostics
                .control_slot
                .map_or(Json::Null, |slot| Json::str(protocol::slot_wire_id(slot))),
        ),
        ("rollback_count", Json::int(diagnostics.rollback_count)),
        ("correction_count", Json::int(diagnostics.correction_count)),
        (
            "predicted_slot_samples",
            Json::int(diagnostics.predicted_slot_samples),
        ),
        (
            "max_rollback_depth",
            Json::int(diagnostics.max_rollback_depth),
        ),
        (
            "late_input_tick",
            Json::opt_int(diagnostics.late_input_tick),
        ),
        ("hash_mismatches", Json::int(diagnostics.hash_mismatches)),
        ("checkpoint_count", Json::int(diagnostics.checkpoint_count)),
        ("settling", Json::bool(diagnostics.settling)),
        ("settled", Json::bool(diagnostics.settled)),
        (
            "full_time_boundary",
            Json::opt_int(diagnostics.full_time_boundary),
        ),
        ("settle_steps", Json::int(diagnostics.settle_steps)),
        ("dropped_outbound", Json::int(diagnostics.dropped_outbound)),
        ("dropped_inbound", Json::int(diagnostics.dropped_inbound)),
    ])
}

/// Verifies `confirmed_output_tick` names a fully-retained, contiguous
/// interval before calling [`rollback_events::confirm`] — which otherwise
/// panics on a gap. A gap is exactly what
/// [`MatchDriverBridge::feed_rollback_timeline`] can legitimately leave
/// behind (a skipped rollback/correction batch), so this re-derives
/// `confirm`'s own precondition and simply does nothing when it does not
/// hold, rather than risk the panic.
fn safe_confirm(timeline: &mut RollbackEventTimeline, confirmed_output_tick: i64) {
    if timeline.status != rollback_events::RollbackEventsStatus::Active {
        return;
    }
    if confirmed_output_tick < -1 || confirmed_output_tick <= timeline.confirmed_tick {
        return;
    }
    let mut boundary = timeline.confirmed_boundary;
    for tick in (timeline.confirmed_tick + 1)..=confirmed_output_tick {
        let Some(step) = timeline.steps.get(&tick) else {
            return;
        };
        if step.start_boundary != boundary {
            return;
        }
        boundary = step.end_boundary;
    }
    rollback_events::confirm(timeline, confirmed_output_tick);
}

/// One peer's online match driver, plus its transport and rollback-event
/// timeline. See the module doc.
#[wasm_bindgen]
pub struct MatchDriverBridge {
    driver: MatchDriver,
    transport: WasmStarTransport,
    timeline: RollbackEventTimeline,
    next_event_tick: i64,
}

#[wasm_bindgen]
impl MatchDriverBridge {
    /// Builds one peer's match driver from an already-constructed offline
    /// [`Session`] (reused for its boundary-zero snapshot — see the module
    /// doc), a real `gc_netcode::coordinator::Freeze`/manifest (JSON —
    /// see `crate::coordinator_bridge`'s array/object rule, and typically
    /// exactly what a [`crate::coordinator_bridge::Coordinator`]'s
    /// `Action::StartMatch`/its own state produced), `role` (`"host"` or
    /// `"guest"`), and this peer's id.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `role` is unrecognized or
    /// `freeze_json`/`manifest_json` fail to parse or decode.
    #[wasm_bindgen(constructor)]
    pub fn new(
        session: &Session,
        role: &str,
        peer_id: &str,
        freeze_json: &str,
        manifest_json: &str,
        max_guests: Option<f64>,
    ) -> Result<MatchDriverBridge, JsValue> {
        let driver_role = match role {
            "host" => match_driver::DriverRole::Host,
            "guest" => match_driver::DriverRole::Guest,
            _ => return Err(js_err("role must be \"host\" or \"guest\"")),
        };
        let transport_role = match driver_role {
            match_driver::DriverRole::Host => gc_netcode::fault_transport::TransportRole::Host,
            match_driver::DriverRole::Guest => gc_netcode::fault_transport::TransportRole::Guest,
        };
        let freeze: Freeze = freeze_from_json(&parse_json(freeze_json)?).map_err(js_err)?;
        let manifest: Value = value_from_json(&parse_json(manifest_json)?).map_err(js_err)?;
        let initial_snapshot = session.capture_snapshot();
        let timeline = rollback_events::new(&initial_snapshot, None);

        let transport = WasmStarTransport::new(transport_role, max_guests.map(|n| n as i64), None);
        let transport_for_driver = transport.clone();
        let rules = DriverRules::new(manifest.clone(), freeze.clone());

        let driver = match_driver::new(MatchDriverOptions {
            role: driver_role,
            peer_id: peer_id.to_string(),
            freeze: to_driver_freeze(&freeze),
            manifest: to_driver_manifest(&manifest),
            transport: Box::new(transport_for_driver),
            initial_snapshot,
            max_rollback_ticks: None,
            hash_interval_ticks: None,
            settle_timeout_ticks: None,
            settle_timeout_seconds: None,
            clock: None,
            rules: Box::new(rules),
        });

        Ok(MatchDriverBridge {
            driver,
            transport,
            timeline,
            next_event_tick: 0,
        })
    }

    /// Brings the transport up. Call once before opening any peer.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) on failure (mirrors every other
    /// transport operation on this bridge).
    #[wasm_bindgen(js_name = initializeTransport)]
    pub fn initialize_transport(&mut self) -> Result<(), JsValue> {
        self.transport
            .initialize()
            .map(|_| ())
            .map_err(|err| js_err(err.message))
    }

    /// Allocates a transport slot for `peer_id`. Returns the assigned slot
    /// number.
    ///
    /// # Errors
    ///
    /// See [`MatchDriverBridge::initialize_transport`].
    #[wasm_bindgen(js_name = openPeer)]
    pub fn open_peer(&mut self, peer_id: &str) -> Result<f64, JsValue> {
        self.transport
            .open_peer(peer_id)
            .map(|slot| slot as f64)
            .map_err(|err| js_err(err.message))
    }

    /// Reports that the real connection to `peer_id` (established by
    /// `@gc/transport`) is up. See [`crate::wasm_transport`]'s module doc
    /// for why signaling itself is out of this bridge's scope.
    #[wasm_bindgen(js_name = setPeerConnected)]
    pub fn set_peer_connected(&mut self, peer_id: &str) {
        self.transport.set_peer_connected(peer_id);
    }

    /// Reports that the real connection to `peer_id` dropped.
    #[wasm_bindgen(js_name = setPeerDisconnected)]
    pub fn set_peer_disconnected(&mut self, peer_id: &str, detail: &str) {
        self.transport.set_peer_disconnected(peer_id, detail);
    }

    /// Queues one arrived envelope, already decoded by `@gc/transport` into
    /// its structured fields — see [`crate::wasm_transport`]'s module doc
    /// for why the wire codec itself stays TypeScript-owned. `channel` is
    /// `"control"`/`"input"`; `kind` is `"input"`/`"event"`/`"state"`. Safe
    /// to call at any time; see the module doc and [`crate::net_inbox`].
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `channel`/`kind` are
    /// unrecognized, `peer_id` names no open link, or the envelope fails
    /// this transport's structural checks.
    #[wasm_bindgen(js_name = enqueueInbound)]
    pub fn enqueue_inbound(
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
        self.transport
            .enqueue_inbound(peer_id, channel, message)
            .map_err(|err| js_err(err.message))
    }

    /// Removes and returns every envelope currently queued to send, as
    /// JSON, oldest first. Call once per tick, after
    /// [`MatchDriverBridge::advance`], and actually transmit each one via
    /// `@gc/transport`.
    #[wasm_bindgen(js_name = drainOutboundJson)]
    #[must_use]
    pub fn drain_outbound_json(&mut self) -> String {
        Json::Array(
            self.transport
                .drain_outbound()
                .iter()
                .map(outbound_envelope_to_json)
                .collect(),
        )
        .to_json_string()
    }

    /// One fixed-tick driver step. `sample_wire` (if this peer authors a
    /// local slot this tick) is a canonical `gc_sim::input_frame::InputSample`
    /// wire (`encode_sample`/`decode_sample`'s format). Internally polls
    /// the transport's inbound queue exactly once — see the module doc and
    /// [`crate::net_inbox`] — and, when this step produced exactly one new
    /// in-order tick, feeds `gc_sim::rollback_events` (see the module doc's
    /// section on that feed's scope). Returns the batch as JSON.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `sample_wire` fails to decode.
    #[wasm_bindgen(js_name = advance)]
    pub fn advance(&mut self, sample_wire: Option<String>) -> Result<String, JsValue> {
        let sample = match sample_wire {
            Some(wire) => {
                Some(input_frame::decode_sample(&wire).map_err(|err| js_err(err.to_string()))?)
            }
            None => None,
        };
        let batch = match_driver::advance(&mut self.driver, sample);
        let (fed, diff_json) = self.feed_rollback_timeline(&batch);
        Ok(self.batch_json(&batch, fed, diff_json))
    }

    /// Current lifecycle status (`"active"`, `"completed"`, ...).
    #[wasm_bindgen(js_name = statusJson)]
    #[must_use]
    pub fn status_json(&self) -> String {
        Json::str(debug_tag(&match_driver::status(&self.driver))).to_json_string()
    }

    /// The terminal outcome, once reached, as JSON (`null` while active).
    #[wasm_bindgen(js_name = terminalJson)]
    #[must_use]
    pub fn terminal_json(&self) -> String {
        match_driver::terminal(&self.driver)
            .as_ref()
            .map_or(Json::Null, driver_terminal_to_json)
            .to_json_string()
    }

    /// A full read of this driver's observable state, as JSON.
    #[wasm_bindgen(js_name = diagnosticsJson)]
    #[must_use]
    pub fn diagnostics_json(&self) -> String {
        diagnostics_to_json(&match_driver::diagnostics(&self.driver)).to_json_string()
    }

    /// The rollback-event timeline's retained-state shape, as JSON. See the
    /// module doc for the feed's scope.
    #[wasm_bindgen(js_name = rollbackDiagnosticsJson)]
    #[must_use]
    pub fn rollback_diagnostics_json(&self) -> String {
        rollback_events_bridge::diagnostics_to_json(&rollback_events::diagnostics(&self.timeline))
            .to_json_string()
    }

    /// The rollback-event timeline's retained byte accounting, as JSON.
    #[wasm_bindgen(js_name = rollbackAccountingJson)]
    #[must_use]
    pub fn rollback_accounting_json(&self) -> String {
        rollback_events_bridge::accounting_to_json(&rollback_events::accounting(&self.timeline))
            .to_json_string()
    }

    /// Every currently retained (unconfirmed) speculative step, as JSON, in
    /// causal order. A debug/spectator view — the ordinary per-step
    /// presentation feed is `advance`'s own `rollback_diff`.
    #[wasm_bindgen(js_name = retainedRollbackStepsJson)]
    #[must_use]
    pub fn retained_rollback_steps_json(&self) -> String {
        Json::Array(
            self.timeline
                .steps
                .values()
                .map(rollback_events_bridge::step_to_json)
                .collect(),
        )
        .to_json_string()
    }
}

impl MatchDriverBridge {
    /// See the module doc's `rollback_events` section. Returns whether the
    /// timeline was fed this step, and its diff as JSON if so.
    fn feed_rollback_timeline(&mut self, batch: &MatchDriverBatch) -> (bool, Option<Json>) {
        if self.timeline.status != rollback_events::RollbackEventsStatus::Active {
            return (false, None);
        }
        let [output] = batch.outputs.as_slice() else {
            return (false, None);
        };
        if output.tick != self.next_event_tick {
            return (false, None);
        }
        let lookup = match_driver::snapshot(&self.driver, output.end_boundary);
        let Some(snapshot) = lookup.snapshot else {
            return (false, None);
        };
        let event_output = rollback_events::RollbackEventTickOutput {
            tick: output.tick,
            start_boundary: output.start_boundary,
            end_boundary: output.end_boundary,
            events: output.events.clone(),
            combat_events: output.combat_events.clone(),
            state: rollback_events::RollbackOutputStateView {
                score: output.state.score,
                time_left: output.state.time_left,
                finished: output.state.finished,
            },
            finished: output.finished,
        };
        let step_input = rollback_events::RollbackEventStepInput {
            output: event_output,
            snapshot,
        };
        let Ok(diff) = rollback_events::apply(
            &mut self.timeline,
            output.tick,
            output.tick,
            std::slice::from_ref(&step_input),
        ) else {
            return (false, None);
        };
        self.next_event_tick += 1;
        let confirmed_output_tick = match_driver::diagnostics(&self.driver).confirmed_output_tick;
        safe_confirm(&mut self.timeline, confirmed_output_tick);
        (true, Some(rollback_events_bridge::diff_to_json(&diff)))
    }

    fn batch_json(&self, batch: &MatchDriverBatch, fed: bool, diff_json: Option<Json>) -> String {
        Json::obj(vec![
            ("step", Json::int(batch.step)),
            ("input_tick", Json::int(batch.input_tick)),
            (
                "outputs",
                Json::Array(batch.outputs.iter().map(output_summary_to_json).collect()),
            ),
            ("reconciliations", Json::int(batch.reconciliations)),
            ("applied_rows", Json::int(batch.applied_rows)),
            ("corrections", Json::int(batch.corrections)),
            ("rollbacks", Json::int(batch.rollbacks)),
            ("sent_packets", Json::int(batch.sent_packets)),
            (
                "checkpoints",
                Json::Array(batch.checkpoints.iter().map(checkpoint_to_json).collect()),
            ),
            (
                "control",
                Json::Array(batch.control.iter().map(peer_message_to_json).collect()),
            ),
            ("live", slot_map_to_json(&batch.live)),
            ("status", Json::str(debug_tag(&batch.status))),
            ("rollback_events_fed", Json::bool(fed)),
            ("rollback_diff", diff_json.unwrap_or(Json::Null)),
        ])
        .to_json_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_netcode::{match_driver_fixture, protocol_fixture};

    use crate::coordinator_bridge::{freeze_to_json, value_to_json};

    /// Builds one connected host driver, seated for a fixture 1v1 session
    /// (host + one guest), with a fresh [`Session`] boundary-zero snapshot —
    /// mirrors [`Session::new`]'s own doc-comment fixture pairing
    /// (`"nebula"`/`"orion"`).
    fn new_host_bridge() -> MatchDriverBridge {
        let mode = protocol::MatchMode::OneVOne;
        let freeze = match_driver_fixture::freeze(mode, None, None);
        let manifest = protocol_fixture::manifest(Some(mode));
        let freeze_json = freeze_to_json(&freeze).to_json_string();
        let manifest_json = value_to_json(&manifest).to_json_string();

        let session = Session::new("nebula", "orion", 7.0, 20.0, 3)
            .expect("the fixture team ids always construct a valid session");

        let mut bridge = MatchDriverBridge::new(
            &session,
            "host",
            match_driver_fixture::HOST_PEER_ID,
            &freeze_json,
            &manifest_json,
            None,
        )
        .expect("a fixture freeze/manifest always constructs a valid driver");

        bridge.initialize_transport().expect("initializes");
        let guest_id = match_driver_fixture::guest_peer_id(1);
        bridge
            .open_peer(&guest_id)
            .expect("opens the fixture guest slot");
        bridge.set_peer_connected(&guest_id);
        bridge
    }

    #[test]
    fn constructs_and_reports_active_status() {
        let bridge = new_host_bridge();
        let status = Json::parse(&bridge.status_json()).unwrap();
        assert_eq!(status.as_str(), Some("active"));
        assert!(Json::parse(&bridge.terminal_json()).unwrap().is_null());
    }

    #[test]
    fn advance_runs_several_ticks_without_erroring_and_reports_a_well_shaped_batch() {
        let mut bridge = new_host_bridge();
        for _ in 0..5 {
            let batch = Json::parse(&bridge.advance(None).unwrap()).unwrap();
            assert!(batch.field_i64("step").is_some());
            assert!(batch.field_i64("input_tick").is_some());
            assert!(batch.get("outputs").unwrap().as_array().is_some());
            assert!(batch.get("checkpoints").unwrap().as_array().is_some());
            assert!(batch.get("control").unwrap().as_array().is_some());
            assert!(batch.field_bool("rollback_events_fed").is_some());
        }
        let status = Json::parse(&bridge.status_json()).unwrap();
        assert_eq!(status.as_str(), Some("active"));
    }

    #[test]
    fn enqueue_inbound_does_not_disturb_advance() {
        let mut bridge = new_host_bridge();
        let guest_id = match_driver_fixture::guest_peer_id(1);
        // An input-shaped envelope on the input channel; content does not
        // need to be a valid packet for this test — it only proves the
        // enqueue/advance seam does not panic or desync the driver's own
        // step counter, not that the driver accepts arbitrary bytes as
        // authority (`canonical_host_batch` validates that separately).
        bridge
            .enqueue_inbound(&guest_id, "input", "input", 0.0, Some(0.0), &[])
            .expect("a known, connected peer accepts a structurally valid envelope");

        let before = Json::parse(&bridge.diagnostics_json())
            .unwrap()
            .field_i64("step");
        bridge.advance(None).unwrap();
        let after = Json::parse(&bridge.diagnostics_json())
            .unwrap()
            .field_i64("step");
        assert_eq!(after, before.map(|step| step + 1));
    }

    #[test]
    fn diagnostics_and_rollback_surfaces_are_well_formed_json() {
        let mut bridge = new_host_bridge();
        bridge.advance(None).unwrap();

        let diagnostics = Json::parse(&bridge.diagnostics_json()).unwrap();
        assert_eq!(diagnostics.field_str("role"), Some("host"));
        assert!(Json::parse(&bridge.rollback_diagnostics_json()).is_ok());
        assert!(Json::parse(&bridge.rollback_accounting_json()).is_ok());
        assert!(
            Json::parse(&bridge.retained_rollback_steps_json())
                .unwrap()
                .as_array()
                .is_some()
        );
    }

    #[test]
    fn drain_outbound_json_is_always_a_parseable_array() {
        let mut bridge = new_host_bridge();
        bridge.advance(None).unwrap();
        let drained = Json::parse(&bridge.drain_outbound_json()).unwrap();
        assert!(drained.as_array().is_some());
    }
}
