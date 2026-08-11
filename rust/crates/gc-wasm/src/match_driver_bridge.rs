//! `wasm-bindgen` control surface over `gc_netcode::match_driver` (the
//! OMP-3 online match driver) and its `gc_sim::rollback_events` feed.
//!
//! ## Reusing `match_driver_fixture::DriverRules`, not reimplementing it
//!
//! `MatchDriver` is built with an injected `Box<dyn MatchDriverRules>` —
//! `match_driver.rs`'s own doc explains why: `carrier`/`transition`/
//! `next_live_slot`/`slot_drivers` are real business logic
//! (`gc_netcode::live_slot`/`gc_netcode::coordinator`) the driver's own file
//! must not reimplement, and `canonical_host_batch` needs protocol machinery
//! (`gc_netcode::protocol`, `gc_netcode::input_protocol`) that file was
//! built without. `gc_netcode::match_driver_fixture::DriverRules`
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
//! appends to the transport's inbound queue, whose capacity
//! [`MatchDriverBridge::new`]'s `queue_limit` parameter controls (defaulting
//! to [`crate::wasm_transport::DEFAULT_QUEUE_LIMIT`] — 64 — like every other
//! `WasmStarTransport` caller). A caller that needs to hold open a delivery
//! gap wider than the default queue can buffer (e.g. to provoke a genuine
//! `gc_sim::rollback_events`/`gc_netcode::match_driver` unconfirmed-window
//! failure through a burst, rather than a permanent stall) must raise it
//! here — see that parameter's own doc for why.
//!
//! ## Why a `MatchSnapshot` never crosses to JS
//!
//! [`MatchDriverBridge::new`] takes a [`Session`] (this crate's own offline
//! session type) instead of raw team/roster parameters: it reuses
//! [`Session::capture_snapshot`] to get a real boundary-zero
//! `gc_sim::match_snapshot::MatchSnapshot`, and that snapshot never leaves
//! Rust. `gc_netcode::match_session`'s own module doc explains why full
//! online content resolution (arena/loadout/roster identity resolved from a
//! manifest's `content_id`) needs `gc-data`, which `gc-netcode` deliberately
//! does not depend on; wiring that up generically is a separate task from
//! "bridge the reducers wasm needs." Reusing `Session`'s already-tested
//! team/ownership construction is the smaller, correct-today alternative:
//! it is exactly what an online match needs (a slot-mode, boundary-zero
//! `MatchState`), built the same way the existing offline path already
//! builds and tests it.
//!
//! ## `rollback_events`: fed automatically, including corrections
//!
//! [`MatchDriverBridge::advance`] feeds every driver step's output into an
//! internally-owned `gc_sim::rollback_events::RollbackEventTimeline` — see
//! [`MatchDriverBridge::feed_rollback_timeline`]. A previous version of this
//! bridge fed only the single-output, no-rollback case and skipped a
//! rollback/correction batch entirely, reasoning that
//! `rollback_events::apply`'s replaced-interval contract
//! (`replaced_from_tick`/`replaced_through_tick`, matching the *complete*
//! stale speculative tail) belonged one layer up, in
//! `packages/online/src/match_presentation.ts`. That module turned out to
//! already encode the general rule: a driver batch's
//! `outputs` are in session-tick order, and any output at or below the
//! highest tick this timeline has already applied is the head of a
//! correction run that replaces the complete speculative tail from that
//! tick through the highest applied tick; anything above it is an ordinary
//! single-tick append. [`MatchDriverBridge::feed_rollback_timeline`] mirrors
//! that same split (see its own doc), so this bridge now feeds a batch of
//! any shape: one append, a correction run, or a correction run followed by
//! fresh appends in the same call.
//!
//! `advance`'s JSON reports `"rollback_events_fed": true` whenever at least
//! one output was applied this call, and `"rollback_diffs"` carries one
//! [`gc_sim::rollback_events::RollbackEventDiff`] per `apply` call made
//! (ordinarily one; two if a correction run is immediately followed by a
//! fresh append in the same batch). A batch this timeline cannot safely
//! apply — a tick gap, or an out-of-order output `rollback_events::apply`
//! was never designed to validate — stops the feed at that point rather
//! than guess; whatever was fed before the stop is still reported.

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
use gc_render::frame::{self as render_frame, RenderFrameOptions, RenderFrameRoster};
use gc_render::frame_buffer;
use gc_sim::input_frame::{self, SlotId};
use gc_sim::match_snapshot::MatchSnapshot;
use gc_sim::retained_history;
use gc_sim::rollback_events::{self, RollbackEventTimeline};
use gc_sim::rollback_session::RollbackTickOutput;
use gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus;
use indexmap::IndexMap;
use wasm_bindgen::prelude::*;

use crate::coordinator_bridge::{freeze_from_json, value_from_json};
use crate::json::{Json, debug_tag};
use crate::rollback_events_bridge::{self, WasmMatchSnapshot};
use crate::session::Session;
use crate::wasm_transport::{OutboundEnvelope, WasmStarTransport};

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

/// The settle-phase wall-clock source [`MatchDriverBridge::new`] injects as
/// `MatchDriverOptions.clock` (`gc_netcode::match_driver`'s own doc on that
/// field: "injected so the settle bound is testable. Defaults to
/// `default_clock`").
///
/// ## Why this exists at all
///
/// `gc_netcode::match_driver::default_clock` — the fallback a `None` here
/// would select — calls `std::time::SystemTime::now()`, which is
/// unimplemented on `wasm32-unknown-unknown` and **traps** the wasm
/// instance (`RuntimeError: unreachable`) rather than returning an error a
/// caller could catch. `reach_full_time` (`match_driver.rs`) calls the
/// injected clock unconditionally the moment a match reaches full time, so
/// every real online match that runs to completion in a browser hit this —
/// confirmed independently through `match_presentation.spec.ts`'s "publishes
/// the lifecycle exactly once through full time" case and a standalone
/// script driving this bridge to full time under node.
///
/// ## Why this is safe to fix off the determinism path
///
/// `default_clock`'s own doc states the invariant this fix must preserve:
/// "Nothing simulated reads this: it can only end a settle phase early,
/// never change a tick." `reach_full_time`/`settle` (`match_driver.rs`) only
/// ever use the clock to decide *when* to give up waiting for the final
/// boundary's confirmation (`settle_deadline`) — never to pick a tick, a
/// hash, or any simulated value two peers must agree on. So the wall-clock
/// value this returns can differ, even wildly, between the wasm build and a
/// native one (or between two peers) with zero effect on the frozen OMP-1
/// digests (`final_hash`/`sequence_digest`) — [`MatchDriverBridge::advance`]'s
/// own doc names this same clock-vs-determinism boundary for
/// `rollback_events`, and this is the same boundary.
///
/// ## Why injection here, not a `wasm32` branch inside `default_clock` itself
///
/// Three fixes were on the table (see this crate's final report): a
/// `wasm32` `cfg` branch inside `gc_netcode::match_driver::default_clock`
/// itself, a monotonic step counter, or injecting the host's real clock
/// through the `clock` seam that function's own field already exists for.
/// This picks the third. `gc_netcode` is the portable reducer crate --
/// nothing else in it depends on `wasm-bindgen`/`js-sys`, and every one of
/// its own tests already runs natively (`cargo test --workspace`, no wasm
/// target involved). Patching `default_clock` itself would mean adding a
/// browser-only dependency to that crate for exactly one caller; injecting
/// here instead keeps `gc_netcode` dependency-free of the browser and puts
/// the browser-specific answer where every other browser-specific answer in
/// this workspace already lives -- `gc-wasm`, the crate whose entire job is
/// bridging into JS. It also matches the field's own doc: "injected... so
/// the settle bound is testable" already names injection as the intended
/// mechanism, not an incidental one.
///
/// `js_sys::Date::now()` (milliseconds since the Unix epoch) divided by
/// `1000.0` reproduces `default_clock`'s own documented semantics exactly
/// ("wall-clock seconds since the Unix epoch") rather than switching to a
/// differently-anchored monotonic source like `performance.now()` (seconds
/// since the page's own navigation start) -- a real behavioral difference
/// this fix must not introduce silently.
///
/// `#[cfg(target_arch = "wasm32")]` only: see `Cargo.toml`'s
/// `[target.'cfg(target_arch = "wasm32")'.dependencies]` entry for why
/// `js-sys` is not part of the native build at all, so this function does
/// not exist on the native (non-wasm32) target this crate's own `mod tests`
/// runs under -- [`driver_settle_clock`] (the `not(wasm32)` twin, right
/// below) is what native `cargo test` links instead, preserving every
/// existing native test's behavior (`default_clock`/`SystemTime`) bit for
/// bit.
#[cfg(target_arch = "wasm32")]
fn driver_settle_clock() -> Option<Box<dyn FnMut() -> f64>> {
    Some(Box::new(|| js_sys::Date::now() / 1000.0))
}

/// The native (non-wasm32) twin of the `wasm32` [`driver_settle_clock`]
/// above: `None` selects `gc_netcode::match_driver::new`'s own fallback,
/// `default_clock` (`std::time::SystemTime`) -- exactly what every native
/// test in this crate (`mod tests` below, `match_driver_fixture_bridge.rs`'s
/// own tests, ...) already ran against before this fix, unchanged.
#[cfg(not(target_arch = "wasm32"))]
fn driver_settle_clock() -> Option<Box<dyn FnMut() -> f64>> {
    None
}

fn parse_json(text: &str) -> Result<Json, JsValue> {
    Json::parse(text).map_err(js_err)
}

/// `pub(crate)`, not private: [`crate::match_driver_fixture_bridge`] reuses
/// this exact wire-string mapping for its own `WasmFakeStarTransport`
/// surface rather than restating it a second time.
pub(crate) fn channel_str(channel: TransportChannel) -> &'static str {
    match channel {
        TransportChannel::Control => "control",
        TransportChannel::Input => "input",
    }
}

/// See [`channel_str`]'s doc.
pub(crate) fn channel_from_str(text: &str) -> Result<TransportChannel, String> {
    match text {
        "control" => Ok(TransportChannel::Control),
        "input" => Ok(TransportChannel::Input),
        other => Err(format!("unknown transport channel '{other}'")),
    }
}

/// See [`channel_str`]'s doc.
pub(crate) fn message_kind_str(kind: TransportMessageType) -> &'static str {
    match kind {
        TransportMessageType::Input => "input",
        TransportMessageType::Event => "event",
        TransportMessageType::State => "state",
    }
}

/// See [`channel_str`]'s doc.
pub(crate) fn message_kind_from_str(text: &str) -> Result<TransportMessageType, String> {
    match text {
        "input" => Ok(TransportMessageType::Input),
        "event" => Ok(TransportMessageType::Event),
        "state" => Ok(TransportMessageType::State),
        other => Err(format!("unknown transport message kind '{other}'")),
    }
}

/// See [`channel_str`]'s doc: [`crate::match_driver_fixture_bridge`] reuses
/// this JSON shape for its own transport's polled messages.
pub(crate) fn transport_message_to_json(message: &TransportMessage) -> Json {
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

/// See [`channel_str`]'s doc.
pub(crate) fn peer_message_to_json(message: &TransportPeerMessage) -> Json {
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

/// Converts a `gc_sim::rollback_session::RollbackTickOutput` (this driver's
/// own output shape) into `gc_sim::rollback_events::RollbackEventTickOutput`
/// (the narrow adapter that module's `apply` actually reads) — see that
/// module's doc for why the adapter carries only a subset of fields. Shared
/// by [`output_summary_to_json`] and [`MatchDriverBridge::feed_rollback_timeline`]
/// so there is exactly one place this conversion happens.
fn to_event_tick_output(output: &RollbackTickOutput) -> rollback_events::RollbackEventTickOutput {
    rollback_events::RollbackEventTickOutput {
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
    }
}

/// This output, as JSON — [`rollback_events_bridge::tick_output_to_json`]'s
/// shape, which is *also* exactly what
/// [`crate::rollback_events_bridge::RollbackEventsTimeline::apply`] accepts
/// per step (see that type's doc): one encoding serves reading `advance`'s
/// batch in JS and handing an entry straight back into a standalone
/// timeline's `apply` call, with no re-shaping in between.
///
/// `pub(crate)`, not private: [`crate::rollback_playable_lab_bridge`] binds
/// the same `gc_sim::rollback_session::RollbackTickOutput` shape (the
/// playable lab steps a `RollbackSession` directly, exactly like this
/// driver does) for its own batch's `outputs`, so this is the one place that
/// conversion happens rather than a third copy alongside this module's own
/// [`to_event_tick_output`].
pub(crate) fn output_summary_to_json(output: &RollbackTickOutput) -> Json {
    rollback_events_bridge::tick_output_to_json(&to_event_tick_output(output))
}

/// `failure`/`tick` are `Option`-shaped (`MatchDriverTerminal.failure`/
/// `.tick`) and use [`Json::obj_omit_null`] so an absent one is an omitted
/// key, not a present `null` — `packages/online/src/net_diagnostics.ts`'s
/// `MatchDriverTerminal.failure?`/`.tick?` are TypeScript optional
/// (absent-or-present) fields, and `recordStep` reads them with
/// `!== undefined`, which a JSON `null` fails (see [`diagnostics_to_json`]'s
/// doc for the full defect this fixes).
fn driver_terminal_to_json(terminal: &MatchDriverTerminal) -> Json {
    Json::obj_omit_null(vec![
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

/// `terminal`/`late_input_tick`/`control_slot`/`full_time_boundary` are all
/// `Option`-shaped and use [`Json::obj_omit_null`], not [`Json::obj`] — see
/// that function's doc. Before this fix every one of them serialized an
/// absent value as a present JSON `null`; `MatchDriverDiagnostics` in
/// `packages/online/src/net_diagnostics.ts` declares them
/// `readonly field?: T` (TypeScript optional, i.e. absent-or-present), and
/// `recordStep` checks `diagnostics.terminal !== undefined` before reading
/// `terminal.status` — a `null` satisfies `!== undefined` and the next line
/// throws "Cannot read properties of null (reading 'status')". Confirmed
/// empirically driving the real compiled artifact: a raw `JSON.parse` of
/// this method's own output crashed the moment a step had no active
/// terminal, which is every step until one is actually reached. Every other
/// field below is a required (always-present) scalar/array and is therefore
/// unaffected by switching to the omit-null variant.
fn diagnostics_to_json(diagnostics: &MatchDriverDiagnostics) -> Json {
    Json::obj_omit_null(vec![
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

/// One [`gc_sim::retained_history::RetainedHistorySample`], as JSON.
///
/// ## Why `history_bytes` is the combined figure
///
/// `history_bytes` is `session.total_bytes + events.total_bytes`, which is
/// what that name means everywhere else in this tree —
/// `gc_sim::rollback_lab`'s peak, `gc-sim`'s
/// `tests/combat_load_fixtures.rs`, and the band
/// `tests/snapshot_headroom.rs` compares to
/// `gc_data::omp2_rollback_validation::Omp2RollbackBudgets::history_bytes`.
/// Reporting only the session half would omit the retained speculative
/// event window, which this very bridge retains per peer (see the module
/// doc's `rollback_events` section) — real browser-side memory that would
/// then grow unobserved.
///
/// ## Why the counts travel with the bytes
///
/// `retained_boundary_count`/`retained_step_count`/`retained_event_count`
/// are what let a caller reject a sample that has stopped measuring. A
/// speculative window of 30 steps holding zero events still reports a
/// plausible nonzero byte total in which no per-event encoder was ever
/// entered (`gc-sim`'s `tests/retained_history.rs` builds exactly that
/// window on purpose). A soak that gated on bytes alone would pass on it.
///
/// Optional ticks are OMITTED rather than nulled, the same rule
/// [`diagnostics_to_json`] follows and for the same reason.
fn retained_history_to_json(sample: &retained_history::RetainedHistorySample) -> Json {
    Json::obj_omit_null(vec![
        ("history_bytes", Json::int(sample.history_bytes)),
        (
            "session",
            Json::obj(vec![
                ("input_bytes", Json::int(sample.session.input.total_bytes)),
                ("output_bytes", Json::int(sample.session.output_bytes)),
                ("snapshot_bytes", Json::int(sample.session.snapshot_bytes)),
                ("total_bytes", Json::int(sample.session.total_bytes)),
            ]),
        ),
        (
            "events",
            rollback_events_bridge::accounting_to_json(&sample.events),
        ),
        (
            "retained_boundary_count",
            Json::int(sample.retained_boundary_count),
        ),
        (
            "peak_retained_boundary_count",
            Json::int(sample.peak_retained_boundary_count),
        ),
        ("peak_snapshot_bytes", Json::int(sample.peak_snapshot_bytes)),
        (
            "oldest_boundary_tick",
            Json::opt_int(sample.oldest_boundary_tick),
        ),
        (
            "latest_boundary_tick",
            Json::opt_int(sample.latest_boundary_tick),
        ),
        ("retained_step_count", Json::int(sample.retained_step_count)),
        (
            "retained_event_count",
            Json::int(sample.retained_event_count),
        ),
    ])
}

fn snapshot_lookup_status_str(status: RollbackSnapshotLookupStatus) -> &'static str {
    match status {
        RollbackSnapshotLookupStatus::Present => "present",
        RollbackSnapshotLookupStatus::Retained => "retained",
        RollbackSnapshotLookupStatus::Missing => "missing",
        RollbackSnapshotLookupStatus::OutsideWindow => "outside_window",
    }
}

/// One `match_driver::snapshot` lookup result, as a `wasm-bindgen` value
/// type — `packages/online/src/match_presentation.ts`'s
/// `SnapshotLookup<TSnapshot>` (`TSnapshot` = [`WasmMatchSnapshot`]):
/// `status` (`"present"`/`"retained"`/`"missing"`/`"outside_window"`),
/// `tick`, `snapshot` (present exactly when `status` is `"present"` or
/// `"retained"`) and `canonicalBytes` (that snapshot's own exact canonical
/// wire size, the per-boundary figure the ring sums into
/// [`MatchDriverBridge::retained_history_accounting_json`]'s
/// `session.snapshot_bytes`).
#[wasm_bindgen]
pub struct SnapshotLookup {
    status: &'static str,
    tick: i64,
    snapshot: Option<MatchSnapshot>,
    canonical_bytes: Option<i64>,
}

#[wasm_bindgen]
impl SnapshotLookup {
    /// This boundary's retention status.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn status(&self) -> String {
        self.status.to_string()
    }

    /// The queried boundary tick.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn tick(&self) -> f64 {
        self.tick as f64
    }

    /// The retained snapshot, present exactly when [`Self::status`] is
    /// `"present"` or `"retained"`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn snapshot(&self) -> Option<WasmMatchSnapshot> {
        self.snapshot.clone().map(WasmMatchSnapshot::new)
    }

    /// This boundary's exact canonical wire byte count
    /// (`gc_sim::match_snapshot::encoded_size_canonical`, computed once when
    /// the ring stored it), present exactly when [`Self::snapshot`] is.
    ///
    /// Exposed because it is the only way a JS caller can re-derive
    /// [`MatchDriverBridge::retained_history_accounting_json`]'s
    /// `session.snapshot_bytes` independently: that field is the ring's own
    /// incrementally-maintained sum, and summing this getter across every
    /// retained boundary must reproduce it exactly. A retained-byte figure
    /// nothing outside the engine can check is a figure taken on trust.
    #[wasm_bindgen(getter, js_name = canonicalBytes)]
    #[must_use]
    pub fn canonical_bytes(&self) -> Option<f64> {
        self.canonical_bytes.map(|bytes| bytes as f64)
    }
}

/// One peer's online match driver, plus its transport and rollback-event
/// timeline. See the module doc.
#[wasm_bindgen]
pub struct MatchDriverBridge {
    driver: MatchDriver,
    transport: WasmStarTransport,
    timeline: RollbackEventTimeline,
    next_event_tick: i64,
    /// This driver's boundary-zero snapshot — see [`MatchDriverBridge::new`]'s
    /// doc. Kept for [`MatchDriverBridge::initial_snapshot_handle`]: a
    /// caller building a standalone
    /// [`crate::rollback_events_bridge::RollbackEventsTimeline`] (rather
    /// than relying on this bridge's own internal auto-feed) needs the same
    /// boundary-zero snapshot this driver itself started from.
    initial_snapshot: MatchSnapshot,
    /// Match-constant per-player roster fields, built once here (mirroring
    /// [`crate::session::Session::new`]'s own `entry.roster`) and reused by
    /// every [`MatchDriverBridge::render_frame_build`] call instead of
    /// rebuilding it every frame — see that method's doc.
    roster: RenderFrameRoster,
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
    /// Returns a `JsValue` (a `String`) if `role` is unrecognized,
    /// `peer_id` is empty, `freeze_json`/`manifest_json` fail to parse or
    /// decode, or `freeze_json`'s `assignments` do not validate against
    /// `manifest_json` (`gc_netcode::protocol::validate_assignment_manifest`
    /// — a structurally-plausible-but-wrong `assignments` array, built by
    /// hand rather than by [`crate::match_driver_fixture_bridge`] or a real
    /// [`crate::coordinator_bridge::Coordinator`], is external JS input and
    /// must be rejected here rather than reach
    /// `gc_netcode::match_driver_fixture::to_driver_freeze`'s internal
    /// `coordinator::assignment_at`/`producer_kind`/`producer_id`, which
    /// `.expect()` a shape this call already guarantees — AGENTS.md §7: a
    /// panic crossing the wasm boundary is not a recoverable `Result` a
    /// caller can catch, and it poisons the instance).
    ///
    /// `queue_limit` bounds this bridge's own internal transport's inbound
    /// queue (`crate::wasm_transport::WasmStarTransport`'s `queue_limit`,
    /// [`crate::wasm_transport::DEFAULT_QUEUE_LIMIT`] — 64 — when omitted).
    /// Before this parameter existed the bridge always passed `None` here,
    /// so the queue was permanently fixed at 64 with nothing on this
    /// bridge's public surface able to raise it — for a caller (typically a
    /// test) that needs to hold a genuine multi-tick delivery gap wide
    /// enough to exceed `gc_sim::rollback_input_history::ROLLBACK_WINDOW_TICKS`
    /// (30) without the queue itself overflowing first (a different,
    /// unrelated failure — `"wasm transport inbound queue is full"` — that
    /// would otherwise mask the window-exceeded condition the caller is
    /// actually trying to provoke), raise this past the delivery gap under
    /// test.
    ///
    /// `initial_snapshot_override`, when given, replaces `session`'s own
    /// `capture_snapshot()` as this driver's boundary-zero
    /// `gc_sim::match_snapshot::MatchSnapshot` — e.g. one of
    /// [`crate::online_combat_phases_bridge`]'s seven pinned combat-phase
    /// snapshots, rather than the default match `session` itself started
    /// from. `session` is still required either way (this constructor has
    /// no other source of the offline team/ownership resolution it needs —
    /// see the module doc's "Why a `MatchSnapshot` never crosses to JS"
    /// section), but its own snapshot goes unused once an override is
    /// supplied; any freshly-constructed `Session` for an authored team
    /// pair (e.g. `"nebula"`/`"orion"`) satisfies it.
    ///
    /// Consumed by value, like every other exported-class parameter this
    /// crate accepts across the wasm boundary (`wasm-bindgen` has no
    /// `OptionFromWasmAbi` for a *reference* to a custom class, only to an
    /// owned one) — so a caller seeding more than one peer in the same
    /// session (every peer must share the *same* boundary zero; a differing
    /// one is a desync fixture, not a correction fixture) does not reuse one
    /// handle across multiple constructor calls. Instead, call the
    /// snapshot's own builder once per peer:
    /// [`crate::online_combat_phases_bridge::online_combat_phase_boundary_zero`]
    /// is a pure, deterministic function of `(id, duration)`, so two calls
    /// with the same arguments produce byte-identical snapshots — exactly
    /// the same guarantee a shared handle would have given, without needing
    /// reference semantics this ABI does not support for a custom class.
    ///
    /// Caution: passing an already-consumed handle a second time does not
    /// throw. `wasm-bindgen`'s generated `Option<WasmMatchSnapshot>`
    /// decoding treats a freed JS wrapper's null pointer as `None` — so a
    /// caller that mistakenly reuses one silently falls back to `session`'s
    /// own snapshot rather than erroring loudly (confirmed empirically
    /// against the compiled artifact under node). This is inherent to how
    /// every `Option<T>`-of-an-exported-class parameter in this crate
    /// behaves once `T` is consumed, not something specific to this
    /// parameter; documented here because a silent behavioural fallback is
    /// easy to miss.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: &Session,
        role: &str,
        peer_id: &str,
        freeze_json: &str,
        manifest_json: &str,
        max_guests: Option<f64>,
        queue_limit: Option<f64>,
        initial_snapshot_override: Option<WasmMatchSnapshot>,
    ) -> Result<MatchDriverBridge, JsValue> {
        let driver_role = match role {
            "host" => match_driver::DriverRole::Host,
            "guest" => match_driver::DriverRole::Guest,
            _ => return Err(js_err("role must be \"host\" or \"guest\"")),
        };
        if peer_id.is_empty() {
            return Err(js_err("peer_id must not be empty"));
        }
        let transport_role = match driver_role {
            match_driver::DriverRole::Host => gc_netcode::fault_transport::TransportRole::Host,
            match_driver::DriverRole::Guest => gc_netcode::fault_transport::TransportRole::Guest,
        };
        let freeze: Freeze = freeze_from_json(&parse_json(freeze_json)?).map_err(js_err)?;
        let manifest: Value = value_from_json(&parse_json(manifest_json)?).map_err(js_err)?;
        // Structurally validates `freeze.assignments` against `manifest`
        // (canonical eight-producer array, valid producer kind/id/bot seed,
        // correct per-human slot count for the manifest's match mode) —
        // exactly the invariant `to_driver_freeze` below assumes already
        // holds. See this method's doc for why this call exists.
        protocol::validate_assignment_manifest(&manifest, &freeze.assignments)
            .map_err(|err| js_err(err.message))?;
        let initial_snapshot = initial_snapshot_override
            .map(|handle| handle.snapshot)
            .unwrap_or_else(|| session.capture_snapshot());
        let timeline = rollback_events::new(&initial_snapshot, None);
        let initial_snapshot_for_export = initial_snapshot.clone();

        let transport = WasmStarTransport::new(
            transport_role,
            max_guests.map(|n| n as i64),
            queue_limit.map(|n| n as i64),
        );
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
            // See `driver_settle_clock`'s own doc: `Some` on `wasm32`
            // (a real host clock, so `reach_full_time`/`settle` never call
            // `default_clock`'s `SystemTime::now()`, unimplemented and
            // trapping on that target), `None` everywhere else (this
            // crate's own native tests, which already exercised
            // `default_clock` correctly).
            clock: driver_settle_clock(),
            rules: Box::new(rules),
        });

        // Built from the driver's own live state right after construction,
        // exactly like `Session::new`'s `render_frame::roster(&state)` call
        // — the roster is match-constant, so this is the one place it needs
        // building, not once per `render_frame_build` call.
        let roster = render_frame::roster(&driver.session.state);

        Ok(MatchDriverBridge {
            driver,
            transport,
            timeline,
            next_event_tick: 0,
            initial_snapshot: initial_snapshot_for_export,
            roster,
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
        let (fed, diffs) = self.feed_rollback_timeline(&batch);
        Ok(self.batch_json(&batch, fed, diffs))
    }

    /// Builds this driver's CURRENT render frame (`self.driver.session.state`
    /// — the live boundary [`MatchDriverBridge::advance`] most recently
    /// produced, post-reconciliation) into a reused, thread-local buffer —
    /// read via [`crate::render_export::driver_render_frame_ptr`]/
    /// [`crate::render_export::driver_render_frame_len`], the exact
    /// three-call raw ptr/len contract
    /// [`crate::render_export::render_frame_build`] already uses for a
    /// [`crate::session::Session`]. Call again before every read; the
    /// pointer is only valid for the block most recently built.
    ///
    /// ## Why this is a bound method, not a `crate::registry`-handle free
    /// function
    ///
    /// See `crate::render_export`'s module doc, "The `MatchDriverBridge`
    /// half": this bridge owns its live `MatchState` directly (a struct
    /// field), not via a `crate::registry`-issued `u32` handle the way
    /// [`crate::session::Session`] does, so there is no FFI-safe scalar a raw
    /// `extern "C"` export could take to name "which bridge". An ordinary
    /// `wasm-bindgen` bound method taking `&mut self` and returning a bare
    /// `u32` costs nothing extra here — `wasm-bindgen` already passes an
    /// exported class's own pointer to a method call as cheaply as this
    /// crate's raw exports take a `u32` handle; the per-frame data this
    /// whole design exists to keep off the `wasm-bindgen` marshalling path is
    /// the ENCODED FRAME BLOCK itself, and that still crosses as a raw
    /// `Float64Array` view, exactly like `Session`'s own path.
    ///
    /// Always returns `1` (kept for interface symmetry with
    /// [`crate::render_export::render_frame_build`]'s `u32` success/failure
    /// convention): unlike a registry handle, a live `&mut self` can never
    /// name a freed or nonexistent bridge.
    ///
    /// `kick_follow_slots`: the renderer's release follow-through window as
    /// a roster-slot bitmask, the same scalar
    /// [`crate::render_export::render_frame_build`] takes — see
    /// [`crate::session::kick_follow_ids`] for why the window crosses as
    /// slots rather than as ids. Pass `0` when no window is open.
    ///
    /// `combat` comes off the driver's OWN rollback session
    /// (`gc_sim::rollback_session::RollbackSession::combat_state`), not off
    /// the [`Session`] this bridge was constructed from: the rollback
    /// session's companion is the one `rollback_session::step` advances and
    /// `rollback_session` rolls back and re-simulates, so it is the only
    /// combat state that describes the `MatchState` two lines below. It is
    /// `Some` exactly when this driver's initial snapshot carried a combat
    /// companion (`gc_sim::match_snapshot::restore_owned`) — i.e. an online
    /// session seated for combat — and `None` otherwise, which unlike
    /// `kick_follow_slots` above is a real value here rather than a
    /// placeholder for an unbuilt path. Without it all seven combat poses
    /// are structurally unreachable (#441).
    #[wasm_bindgen(js_name = renderFrameBuild)]
    pub fn render_frame_build(&mut self, kick_follow_slots: u32) -> u32 {
        let options = self.frame_options(kick_follow_slots);
        let built = render_frame::build(&self.driver.session.state, &options);
        crate::render_export::build_driver_frame(&built);
        1
    }

    /// [`MatchDriverBridge::render_frame_build`]'s options, split out for the
    /// same reason [`crate::session::frame_options`] is a function of its
    /// own: what a construction site puts in `RenderFrameOptions` decides
    /// which poses are reachable at all, and a field left at `Default` there
    /// is invisible from the encoded frame block a JS caller reads back.
    /// This crate's native tests assert on it directly.
    fn frame_options(&self, kick_follow_slots: u32) -> RenderFrameOptions {
        RenderFrameOptions {
            roster: Some(self.roster.clone()),
            kick_follow: crate::session::kick_follow_ids(&self.roster, kick_follow_slots),
            combat: self
                .driver
                .session
                .combat_state
                .as_ref()
                .map(|combat| render_frame::combat_model(&self.driver.session.state, combat)),
            ..Default::default()
        }
    }

    /// Match-constant per-player roster fields, as a flat `Float64Array` —
    /// `gc_render::frame_buffer::encode_roster`'s numeric half, over this
    /// bridge's own `roster` field (built once in [`MatchDriverBridge::new`],
    /// the same match-constant identity [`MatchDriverBridge::render_frame_build`]
    /// already reuses every frame). Mirrors
    /// [`crate::session::Session::roster_numeric`] exactly — before this
    /// method existed, `MatchDriverBridge` had no roster export of its own
    /// at all (unlike [`crate::session::Session`]'s `rosterNumeric`/
    /// `rosterIdsAndNames`), so an online renderer decoding a live
    /// `MatchDriverBridge` frame had no honest way to recover
    /// `species_shape`/`radius`/`is_keeper` per slot — it could only
    /// fabricate them (`packages/screens/src/online_match_flow.spec.ts`'s
    /// own "online match renderer smoke" case names this gap directly).
    #[wasm_bindgen(js_name = rosterNumeric)]
    #[must_use]
    pub fn roster_numeric(&self) -> Vec<f64> {
        frame_buffer::encode_roster(&self.roster).0
    }

    /// Match-constant roster STRING columns —
    /// `gc_render::frame_buffer::encode_roster`'s string half, a
    /// newline-joined blob per that function's documented format. The
    /// string counterpart of [`MatchDriverBridge::roster_numeric`]; see that
    /// method's doc for why this gap existed and what it closes.
    ///
    /// Since #447 the blob carries `id`, `name`, `presentation_id` and
    /// `loadout_id` per slot, so the JS name understates it; see
    /// [`crate::session::Session::roster_ids_and_names`] for why it was not
    /// renamed.
    #[wasm_bindgen(js_name = rosterIdsAndNames)]
    #[must_use]
    pub fn roster_ids_and_names(&self) -> String {
        frame_buffer::encode_roster(&self.roster).1
    }

    /// Mirrors `gc_netcode::match_driver::observe_checkpoint`: compares
    /// `hash` against this driver's own published checkpoint hash at
    /// `tick`, if it has one. Returns `true` when they agree (or this
    /// driver never published a checkpoint at `tick`, e.g. `hash` is stale
    /// or from a boundary this driver has not reached yet — see that
    /// function's own doc), `false` on a genuine one-off disagreement.
    /// Every consecutive disagreement is counted internally
    /// (`diagnosticsJson`'s `hash_mismatches`); at
    /// `gc_netcode::match_driver::MAX_HASH_MISMATCHES` consecutive
    /// disagreements this call itself terminates the driver
    /// (`status` becomes `"hash_mismatch"`, `terminal.failure` becomes
    /// `"desync"`) rather than the caller having to check the return value
    /// and act on it separately. Before this method existed there was no
    /// way to force a hash divergence deterministically from `@gc/wasm` at
    /// all — confirmed absent from this bridge's surface and `types.ts`.
    #[wasm_bindgen(js_name = observeCheckpoint)]
    pub fn observe_checkpoint(&mut self, tick: f64, hash: &str) -> bool {
        match_driver::observe_checkpoint(&mut self.driver, tick as i64, hash)
    }

    /// This driver's own boundary-zero snapshot (the same one
    /// [`MatchDriverBridge::new`] captured its internal timeline from), as
    /// an opaque handle — for building a standalone
    /// [`crate::rollback_events_bridge::RollbackEventsTimeline`] via
    /// [`crate::rollback_events_bridge::RollbackEventsTimeline::create`]
    /// against this same driver's snapshot history, satisfying
    /// `packages/online/src/match_presentation.ts`'s
    /// `newOnlineMatchPresentation`.
    #[wasm_bindgen(js_name = initialSnapshotHandle)]
    #[must_use]
    pub fn initial_snapshot_handle(&self) -> WasmMatchSnapshot {
        WasmMatchSnapshot::new(self.initial_snapshot.clone())
    }

    /// `gc_netcode::match_driver::snapshot`: looks up this driver's own
    /// retained boundary-snapshot history at `boundary_tick`, as an opaque
    /// [`SnapshotLookup`] — `packages/online/src/match_presentation.ts`'s
    /// `MatchDriverPort.snapshot`.
    #[wasm_bindgen(js_name = snapshotLookup)]
    #[must_use]
    pub fn snapshot_lookup(&self, boundary_tick: f64) -> SnapshotLookup {
        let lookup = match_driver::snapshot(&self.driver, boundary_tick as i64);
        SnapshotLookup {
            status: snapshot_lookup_status_str(lookup.status),
            tick: lookup.tick,
            snapshot: lookup.snapshot,
            canonical_bytes: lookup.canonical_bytes,
        }
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

    /// A full read of this bridge's own internal transport
    /// (`crate::wasm_transport::WasmStarTransport`) — star and per-peer
    /// channel counters (`sent`/`received`/`dropped_outbound`/
    /// `dropped_inbound`/`queue_limit`/...), as JSON
    /// (`crate::match_driver_fixture_bridge::star_diagnostics_to_json`'s
    /// shape, the exact `TransportStarDiagnostics` mapping
    /// [`crate::match_driver_fixture_bridge::WasmFakeStarTransport::diagnostics_json`]
    /// already exposes for the fixture's in-process star). Before this
    /// method existed nothing on this bridge's public surface could report
    /// this data at all — confirmed by reading `wasm_transport.rs` end to
    /// end, it carries no `#[wasm_bindgen]` attribute of its own — so a
    /// `DiagnosticTransport`-style tap could never observe a
    /// `MatchDriverBridge`'s own traffic (`net_diagnostics.spec.ts`'s file
    /// header names this gap precisely).
    #[wasm_bindgen(js_name = transportDiagnosticsJson)]
    #[must_use]
    pub fn transport_diagnostics_json(&self) -> String {
        crate::match_driver_fixture_bridge::star_diagnostics_to_json(&self.transport.diagnostics())
            .to_json_string()
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
    ///
    /// This is one HALF of what this peer retains. For the figure a
    /// retention budget is written against, use
    /// [`Self::retained_history_accounting_json`].
    #[wasm_bindgen(js_name = rollbackAccountingJson)]
    #[must_use]
    pub fn rollback_accounting_json(&self) -> String {
        rollback_events_bridge::accounting_to_json(&rollback_events::accounting(&self.timeline))
            .to_json_string()
    }

    /// **Everything this peer retains**, as JSON — the combined
    /// `history_bytes` figure plus the components and occupancy counts
    /// behind it ([`retained_history_to_json`]'s shape, from
    /// `gc_sim::retained_history::sample`).
    ///
    /// This is the seam a long-duration soak samples. Before it existed,
    /// nothing on this bridge could report the session half at all
    /// ([`Self::rollback_accounting_json`] is the event timeline only), so a
    /// browser peer had no honest way to observe its own retained-history
    /// growth: the snapshot ring, the input history and the retained
    /// outputs — together the large majority of the number — were invisible
    /// from JS. Sampling `performance.memory` instead is not an
    /// alternative; it excludes wasm linear memory entirely, and wasm linear
    /// memory never shrinks, so a whole-instance figure could not attribute
    /// growth to retention even if it were visible.
    ///
    /// Cost: O(retained input rows + retained outputs + retained events),
    /// with the snapshot component read from a counter the ring maintains
    /// incrementally (no snapshot is re-encoded). Measured at ~0.27 ms per
    /// call in a native release build on a full 31-boundary ring with a full
    /// 30-step speculative window (`gc-sim`'s `tests/retained_history.rs`
    /// prints it), and ~0.37 ms through this bridge under node
    /// (`packages/wasm/src/retained_history.spec.ts` prints that one).
    /// Cheap enough to sample every tick for hours; still not something to
    /// call per entity.
    ///
    /// Takes `&mut self` because `gc_sim::rollback_session::accounting` does
    /// (it re-derives retained output bytes through the session's own
    /// counted-output cache). Nothing observable about this driver changes.
    #[wasm_bindgen(js_name = retainedHistoryAccountingJson)]
    #[must_use]
    pub fn retained_history_accounting_json(&mut self) -> String {
        retained_history_to_json(&retained_history::sample(
            &mut self.driver.session,
            &self.timeline,
        ))
        .to_json_string()
    }

    /// Every currently retained (unconfirmed) speculative step, as JSON, in
    /// causal order. A debug/spectator view — the ordinary per-step
    /// presentation feed is `advance`'s own `rollback_diffs`.
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
    /// Builds one `rollback_events::apply` step input for `output`, by
    /// looking up this driver's own retained boundary snapshot at
    /// `output.end_boundary`. `None` if that boundary is not
    /// present/retained (a gap [`MatchDriverBridge::feed_rollback_timeline`]
    /// cannot safely apply through).
    fn step_input_for(
        &self,
        output: &RollbackTickOutput,
    ) -> Option<rollback_events::RollbackEventStepInput> {
        let lookup = match_driver::snapshot(&self.driver, output.end_boundary);
        let snapshot = lookup.snapshot?;
        Some(rollback_events::RollbackEventStepInput {
            output: to_event_tick_output(output),
            snapshot,
        })
    }

    /// Feeds `batch`'s outputs into this bridge's own internally-owned
    /// timeline, in as many `rollback_events::apply` calls as the batch's
    /// shape requires — see the module doc's `rollback_events` section for
    /// the append/correction split this mirrors from
    /// `match_presentation.ts`'s `consume`. Returns whether at least one
    /// output was applied, and one diff (as JSON) per `apply` call made, in
    /// order.
    ///
    /// `batch.outputs` is in session-tick order. An output at or above
    /// `self.next_event_tick` (the next tick this timeline expects to
    /// append) is an ordinary single-tick append; an output below it is the
    /// head of a correction run that replaces the complete speculative tail
    /// from that tick through `self.next_event_tick - 1` (the highest tick
    /// already applied) — `rollback_events::apply`'s own contract for a
    /// correcting call (see `gc_sim::rollback_events::apply`'s doc). A
    /// batch this timeline cannot safely apply through (an unexpected gap,
    /// a missing retained snapshot, or `apply` itself reporting the
    /// unconfirmed window exceeded) stops the feed at that point rather
    /// than guess; whatever was fed before the stop is still returned.
    fn feed_rollback_timeline(&mut self, batch: &MatchDriverBatch) -> (bool, Vec<Json>) {
        let mut diffs = Vec::new();
        if self.timeline.status != rollback_events::RollbackEventsStatus::Active {
            return (false, diffs);
        }
        let outputs = batch.outputs.as_slice();
        let mut index = 0;
        while index < outputs.len() {
            let output = &outputs[index];
            if output.tick >= self.next_event_tick {
                // An ordinary append: `apply`'s normal-call contract
                // requires exactly the next contiguous tick.
                if output.tick != self.next_event_tick {
                    break;
                }
                let Some(step_input) = self.step_input_for(output) else {
                    break;
                };
                let Ok(diff) = rollback_events::apply(
                    &mut self.timeline,
                    output.tick,
                    output.tick,
                    std::slice::from_ref(&step_input),
                ) else {
                    break;
                };
                diffs.push(rollback_events_bridge::diff_to_json(&diff));
                self.next_event_tick += 1;
                index += 1;
            } else {
                // A correction run: gather every contiguous corrected
                // output from here through the highest tick already
                // applied, then replace that whole interval in one `apply`
                // call — `rollback_events::apply`'s correcting-call
                // contract requires the complete stale speculative tail,
                // not a partial prefix of it.
                let from = output.tick;
                let through = self.next_event_tick - 1;
                let mut steps = Vec::new();
                let mut expected = from;
                while index < outputs.len() && expected <= through {
                    let candidate = &outputs[index];
                    if candidate.tick != expected {
                        break;
                    }
                    let Some(step_input) = self.step_input_for(candidate) else {
                        break;
                    };
                    steps.push(step_input);
                    expected += 1;
                    index += 1;
                }
                if steps.is_empty() {
                    break;
                }
                let corrected_count = steps.len() as i64;
                let Ok(diff) = rollback_events::apply(&mut self.timeline, from, through, &steps)
                else {
                    break;
                };
                diffs.push(rollback_events_bridge::diff_to_json(&diff));
                // A correction may legally shorten the stale tail only at
                // full time (`rollback_events::apply`'s own assertion), so
                // the next expected append follows the corrected run's
                // actual length, not the replaced interval's.
                self.next_event_tick = from + corrected_count;
            }
        }
        let fed = !diffs.is_empty();
        if fed {
            let confirmed_output_tick =
                match_driver::diagnostics(&self.driver).confirmed_output_tick;
            safe_confirm(&mut self.timeline, confirmed_output_tick);
        }
        (fed, diffs)
    }

    fn batch_json(&self, batch: &MatchDriverBatch, fed: bool, diffs: Vec<Json>) -> String {
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
            ("rollback_diffs", Json::Array(diffs)),
        ])
        .to_json_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_netcode::{match_driver_fixture, protocol_fixture};
    use gc_render::player_pose::PlayerPoseId;
    use gc_sim::combat_feasibility::CombatActionPhase;
    use gc_sim::combat_snapshot::CombatForcedState;
    use gc_sim::rollback_session;

    use crate::coordinator_bridge::{freeze_to_json, value_to_json};

    /// Builds one connected host driver, seated for a fixture 1v1 session
    /// (host + one guest), with a fresh [`Session`] boundary-zero snapshot —
    /// mirrors [`Session::new`]'s own doc-comment fixture pairing
    /// (`"nebula"`/`"orion"`). `queue_limit` is this bridge's own transport
    /// inbound-queue cap (`None` = the transport's default, 64).
    fn new_host_bridge_with_queue_limit(queue_limit: Option<f64>) -> MatchDriverBridge {
        let mode = protocol::MatchMode::OneVOne;
        let freeze = match_driver_fixture::freeze(mode, None, None);
        let manifest = protocol_fixture::manifest(Some(mode));
        let freeze_json = freeze_to_json(&freeze).to_json_string();
        let manifest_json = value_to_json(&manifest).to_json_string();

        let session = Session::new(
            "nebula", "orion", 7.0, 20.0, 3, None, None, None, None, None,
        )
        .expect("the fixture team ids always construct a valid session");

        let mut bridge = MatchDriverBridge::new(
            &session,
            "host",
            match_driver_fixture::HOST_PEER_ID,
            &freeze_json,
            &manifest_json,
            None,
            queue_limit,
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

    /// See [`new_host_bridge_with_queue_limit`]; the default queue limit
    /// (`None`) is what every pre-existing test in this module wants.
    fn new_host_bridge() -> MatchDriverBridge {
        new_host_bridge_with_queue_limit(None)
    }

    /// The same fixture host bridge, but seated on a COMBAT-ACTIVE initial
    /// snapshot (`match_driver_fixture::initial_snapshot`'s `combat_active`)
    /// instead of the plain [`Session`] boundary-zero one — the only way a
    /// driver's rollback session gets a `combat_state` companion at all
    /// (`gc_sim::match_snapshot::restore_owned`).
    fn new_host_bridge_with_combat() -> MatchDriverBridge {
        let mode = protocol::MatchMode::OneVOne;
        let freeze = match_driver_fixture::freeze(mode, None, None);
        let manifest = protocol_fixture::manifest(Some(mode));
        let freeze_json = freeze_to_json(&freeze).to_json_string();
        let manifest_json = value_to_json(&manifest).to_json_string();

        let session = Session::new(
            "nebula", "orion", 7.0, 20.0, 3, None, None, None, None, None,
        )
        .expect("the fixture team ids always construct a valid session");

        MatchDriverBridge::new(
            &session,
            "host",
            match_driver_fixture::HOST_PEER_ID,
            &freeze_json,
            &manifest_json,
            None,
            None,
            Some(WasmMatchSnapshot::new(
                match_driver_fixture::initial_snapshot(None, true, None),
            )),
        )
        .expect("a fixture freeze/manifest always constructs a valid driver")
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
        assert!(Json::parse(&bridge.retained_history_accounting_json()).is_ok());
    }

    /// Steps a fixture host bridge far enough to fill its 31-boundary
    /// snapshot ring and its 30-step speculative event window, so the
    /// retained reading below is a steady-state one.
    ///
    /// This lone-host fixture stops confirming once its silent guest's
    /// input falls below the retained floor (`status` becomes
    /// `"confirmation_stalled"` around step 31) — which is exactly the
    /// retention shape a soak has to be able to see: a full ring and a full
    /// unconfirmed window, held.
    fn stepped_host_bridge() -> MatchDriverBridge {
        let mut bridge = new_host_bridge();
        for _ in 0..40 {
            bridge.advance(None).expect("a fixture host step");
        }
        bridge
    }

    #[test]
    fn retained_history_accounting_reports_the_combined_total_not_the_event_half() {
        let mut bridge = stepped_host_bridge();
        let json = Json::parse(&bridge.retained_history_accounting_json()).unwrap();

        // The anti-regression this whole surface exists for: the number
        // that crosses must be `rollback_session::accounting().total_bytes`
        // PLUS `rollback_events::accounting().total_bytes`, read from this
        // bridge's own live driver. Exposing either half alone is a
        // plausible, nonzero, budget-comparable figure that silently omits
        // most of what the peer retains.
        let session_total = rollback_session::accounting(&mut bridge.driver.session).total_bytes;
        let event_total = rollback_events::accounting(&bridge.timeline).total_bytes;
        assert!(session_total > 0 && event_total > 0, "both halves are live");
        assert_eq!(
            json.field_i64("history_bytes"),
            Some(session_total + event_total),
            "history_bytes must be the engine's own combined figure"
        );
        assert_eq!(
            json.get("session").and_then(|s| s.field_i64("total_bytes")),
            Some(session_total)
        );
        assert_eq!(
            json.get("events").and_then(|e| e.field_i64("total_bytes")),
            Some(event_total)
        );
        // Stated as a comparison rather than a pinned constant so an
        // encoder change moves it without breaking this test: the session
        // half is the large majority, so a reading that had quietly become
        // the event half alone would be off by an order of magnitude.
        assert!(
            json.field_i64("history_bytes").unwrap() > event_total * 10,
            "the session half dominates; a reading close to the event half is the event half"
        );

        // The session half's own components must add up, so a mis-wired
        // field is visible rather than merely plausible.
        let session = json.get("session").unwrap();
        assert_eq!(
            session.field_i64("input_bytes").unwrap()
                + session.field_i64("output_bytes").unwrap()
                + session.field_i64("snapshot_bytes").unwrap(),
            session_total
        );
    }

    #[test]
    fn retained_history_snapshot_bytes_equal_the_sum_of_the_retained_boundaries() {
        // An independent re-derivation of the dominant component: the ring
        // maintains `snapshot_bytes` incrementally, while
        // `SnapshotLookup::canonicalBytes` reports each boundary's own
        // stored size. Summing the latter across every retained boundary
        // must reproduce the former exactly — the same check
        // `packages/wasm/src/retained_history.spec.ts` performs from JS, so
        // the figure a browser peer samples is one it can verify rather
        // than take on trust.
        let mut bridge = stepped_host_bridge();
        let json = Json::parse(&bridge.retained_history_accounting_json()).unwrap();
        let oldest = json.field_i64("oldest_boundary_tick").unwrap();
        let latest = json.field_i64("latest_boundary_tick").unwrap();
        let retained = json.field_i64("retained_boundary_count").unwrap();
        assert_eq!(
            latest - oldest + 1,
            retained,
            "the retained boundaries are contiguous"
        );

        let mut summed = 0;
        let mut counted = 0;
        for tick in oldest..=latest {
            let lookup = bridge.snapshot_lookup(tick as f64);
            let bytes = lookup
                .canonical_bytes()
                .expect("a retained boundary reports its canonical size");
            assert!(bytes > 0.0);
            summed += bytes as i64;
            counted += 1;
        }
        assert_eq!(counted, retained);
        assert_eq!(
            summed,
            json.get("session")
                .and_then(|s| s.field_i64("snapshot_bytes"))
                .unwrap(),
            "per-boundary canonical sizes must sum to the ring's own reported total"
        );
    }

    #[test]
    fn retained_history_reports_the_occupancy_that_says_whether_the_bytes_mean_anything() {
        let mut bridge = stepped_host_bridge();
        let json = Json::parse(&bridge.retained_history_accounting_json()).unwrap();

        // A full ring and a full unconfirmed window: a partially-warmed
        // reading would understate retention while looking comfortable.
        assert_eq!(json.field_i64("retained_boundary_count"), Some(31));
        assert_eq!(json.field_i64("peak_retained_boundary_count"), Some(31));
        assert_eq!(json.field_i64("retained_step_count"), Some(30));
        // And real events inside it. Thirty retained steps holding zero
        // events would still report a plausible byte total in which no
        // per-event encoder was ever entered — `gc-sim`'s
        // `tests/retained_history.rs` builds exactly that window and shows
        // the bytes cannot tell the difference, while this count can.
        assert!(
            json.field_i64("retained_event_count").unwrap() > 0,
            "the retained window must hold real events, not just step wrappers"
        );
        assert!(json.field_i64("peak_snapshot_bytes").unwrap() > 0);
    }

    #[test]
    fn retained_history_accounting_is_a_read_not_a_step() {
        // A soak samples this repeatedly; it must not perturb the driver it
        // measures.
        let mut bridge = stepped_host_bridge();
        let before = bridge.diagnostics_json();
        let first = bridge.retained_history_accounting_json();
        let second = bridge.retained_history_accounting_json();
        assert_eq!(first, second, "sampling twice must read the same window");
        assert_eq!(
            before,
            bridge.diagnostics_json(),
            "sampling must not advance or otherwise disturb the driver"
        );
    }

    #[test]
    fn diagnostics_json_omits_absent_optional_fields_rather_than_nulling_them() {
        // Proves defect #1 fixed: before this change, `terminal`,
        // `late_input_tick`, and `control_slot` were always-present keys,
        // `null` when this driver had nothing to report for them --
        // `packages/online/src/net_diagnostics.ts`'s `MatchDriverDiagnostics`
        // declares all three TypeScript-optional (`field?: T`), and its
        // `recordStep` checks `!== undefined` before dereferencing
        // `terminal.status`, which a JSON `null` satisfies -- crashing with
        // "Cannot read properties of null" on the very first tick, since no
        // driver has an active terminal that early. See
        // `diagnostics_to_json`'s doc for the full account.
        let mut bridge = new_host_bridge();
        bridge.advance(None).unwrap();
        let diagnostics = Json::parse(&bridge.diagnostics_json()).unwrap();
        assert_eq!(
            diagnostics.field_str("role"),
            Some("host"),
            "sanity: still a well-formed diagnostics object"
        );
        // `Json::get` (unlike `Json::field`) distinguishes an absent key
        // from a present `null` -- exactly the distinction this defect was
        // about. `terminal`/`late_input_tick` are genuinely absent this
        // early (no terminal reached, no late input observed yet).
        assert!(
            diagnostics.get("terminal").is_none(),
            "an inactive driver's diagnostics must omit 'terminal' entirely, not null it"
        );
        assert!(
            diagnostics.get("late_input_tick").is_none(),
            "must omit 'late_input_tick' entirely, not null it"
        );
        // `control_slot`, by contrast, genuinely IS present for a seated
        // host (`gc_netcode::match_driver::diagnostics` only reports `None`
        // when `owned` is empty) -- proving the other half of
        // `Json::obj_omit_null`'s contract: a present optional value must
        // still come through as its real value, not be swallowed along with
        // the absent ones.
        assert!(
            diagnostics
                .get("control_slot")
                .is_some_and(|v| v.as_str().is_some()),
            "a seated host's control_slot must be present (a wire slot id string), not omitted"
        );
    }

    #[test]
    fn queue_limit_is_configurable_and_reaches_the_wrapped_transport() {
        // Proves defect #2 fixed: `MatchDriverBridge::new` used to always
        // pass `None` for the wrapped transport's `queue_limit`, so the
        // inbound queue was permanently fixed at
        // `crate::wasm_transport::DEFAULT_QUEUE_LIMIT` (64) with nothing on
        // this bridge's public surface able to raise it -- a burst wide
        // enough to exceed the ~30-tick rollback window
        // (`gc_sim::rollback_input_history::ROLLBACK_WINDOW_TICKS`)
        // overflowed the queue itself first, so the window-exceeded
        // condition was unreachable through the public API.
        //
        // This reads the wired-through value back via
        // `transportDiagnosticsJson` rather than actually overflowing the
        // queue: `enqueue_inbound`'s overflow error path constructs a
        // `JsValue` (`crate::wasm_transport`'s `TransportError` mapped
        // through this crate's `js_err`), which panics off the native
        // (non-wasm32) test target this runs under -- the same constraint
        // `crate::coordinator_bridge`'s module doc documents for every
        // other exported error path in this crate. Reading the diagnostics
        // value back proves the same wiring without touching that path.
        let default_bridge = new_host_bridge_with_queue_limit(None);
        let default_diagnostics =
            Json::parse(&default_bridge.transport_diagnostics_json()).unwrap();
        assert_eq!(
            default_diagnostics.field_i64("queue_limit"),
            Some(crate::wasm_transport::DEFAULT_QUEUE_LIMIT),
            "omitting queue_limit must still fall back to the transport's own default"
        );

        let raised_bridge = new_host_bridge_with_queue_limit(Some(200.0));
        let raised_diagnostics = Json::parse(&raised_bridge.transport_diagnostics_json()).unwrap();
        assert_eq!(
            raised_diagnostics.field_i64("queue_limit"),
            Some(200),
            "a caller-supplied queue_limit must reach the wrapped WasmStarTransport"
        );

        // And the wiring is exercised end to end, not just read back: with
        // the limit raised, enqueueing well past the old fixed cap (64),
        // with nothing ever draining them (`advance` drains; this never
        // calls it), succeeds in full -- the exact scenario the defect made
        // unreachable.
        let mut raised_bridge = raised_bridge;
        let guest_id = match_driver_fixture::guest_peer_id(1);
        for seq in 0..100 {
            raised_bridge
                .enqueue_inbound(
                    &guest_id,
                    "input",
                    "input",
                    f64::from(seq),
                    Some(f64::from(seq)),
                    b"x",
                )
                .expect("a queue_limit of 200 accepts 100 unpolled enqueues");
        }
    }

    #[test]
    fn drain_outbound_json_is_always_a_parseable_array() {
        let mut bridge = new_host_bridge();
        bridge.advance(None).unwrap();
        let drained = Json::parse(&bridge.drain_outbound_json()).unwrap();
        assert!(drained.as_array().is_some());
    }

    /// Regression test for this milestone's gap: before
    /// `render_frame_build` existed, nothing could turn a live
    /// `MatchDriverBridge` into a `RenderFrame` at all -- this proves the
    /// full three-call contract works end to end (build, then read
    /// ptr/len), producing a non-empty block that changes shape between
    /// calls exactly the way `render_export`'s own `Session` path already
    /// does, and that it does not disturb the buffer
    /// `render_frame_build`/`render_frame_ptr`/`render_frame_len` (the
    /// `Session` path) uses.
    #[test]
    fn render_frame_build_produces_a_readable_block_independent_of_the_session_buffer() {
        let mut bridge = new_host_bridge();
        bridge.advance(None).unwrap();

        let ok = bridge.render_frame_build(0);
        assert_eq!(ok, 1);
        let ptr = crate::render_export::driver_render_frame_ptr();
        let len = crate::render_export::driver_render_frame_len();
        assert!(len > 0, "a built driver render frame must be non-empty");
        assert_ne!(ptr, 0);

        // Building a `Session`'s own frame afterward must not disturb the
        // driver's already-built block -- the two live in separate
        // thread-local buffers (see `render_export`'s module doc).
        let session = Session::new(
            "nebula", "orion", 7.0, 20.0, 3, None, None, None, None, None,
        )
        .expect("the fixture team ids always construct a valid session");
        let session_ok = crate::render_export::render_frame_build(session.handle(), 0);
        assert_eq!(session_ok, 1);
        assert_eq!(crate::render_export::driver_render_frame_ptr(), ptr);
        assert_eq!(crate::render_export::driver_render_frame_len(), len);
    }

    /// Regression test for this wave's gap: before [`MatchDriverBridge::roster_numeric`]/
    /// [`MatchDriverBridge::roster_ids_and_names`] existed, this bridge had
    /// no roster export of its own at all — unlike [`Session`]'s
    /// `rosterNumeric`/`rosterIdsAndNames` — so an online renderer decoding
    /// a live `MatchDriverBridge` frame had no honest way to recover
    /// `species_shape`/`radius`/`is_keeper` per slot.
    /// `new_host_bridge`'s own internal `Session` is built from the exact
    /// same `("nebula", "orion", ...)` construction as `session` below, so
    /// this asserts real equality, not merely "non-empty" — proving the
    /// bridge's own `roster` field (built once in `MatchDriverBridge::new`,
    /// the same match-constant identity `render_frame_build` already
    /// reuses) really is the fixture's roster, not a placeholder.
    #[test]
    fn roster_numeric_and_ids_and_names_mirror_a_session_built_the_same_way() {
        let bridge = new_host_bridge();
        let session = Session::new(
            "nebula", "orion", 7.0, 20.0, 3, None, None, None, None, None,
        )
        .expect("the fixture team ids always construct a valid session");
        assert_eq!(bridge.roster_numeric(), session.roster_numeric());
        assert_eq!(
            bridge.roster_ids_and_names(),
            session.roster_ids_and_names()
        );
    }

    /// #441's second construction site. A driver seated for a NON-combat
    /// session restores no combat companion, so its frame options carry
    /// `None` — a real value here, not a stand-in.
    #[test]
    fn a_non_combat_driver_carries_no_combat_model() {
        let bridge = new_host_bridge();
        assert!(bridge.driver.session.combat_state.is_none());
        assert!(bridge.frame_options(0).combat.is_none());
    }

    /// ...and a driver whose initial snapshot DID carry a combat companion
    /// reaches the frame's pose column with it, exactly like
    /// `crate::session::frame_options` does for a local match. This is the
    /// site that pass `0` for `kick_follow_slots` because no production
    /// caller supplies a follow-through window yet; combat is different —
    /// the rollback session owns a real companion, so there is nothing to
    /// stand in for.
    #[test]
    fn a_combat_seated_driver_reaches_the_frame_as_a_combat_pose() {
        let mut bridge = new_host_bridge_with_combat();
        let combat = bridge
            .driver
            .session
            .combat_state
            .as_mut()
            .expect("a combat-active initial snapshot restores its companion");
        combat.players[1].phase = CombatActionPhase::Guard;
        combat.players[2].forced_state = Some(CombatForcedState::Stagger);
        combat.players[2].forced_ticks = 4;

        let options = bridge.frame_options(0);
        assert!(options.combat.is_some());
        let built = render_frame::build(&bridge.driver.session.state, &options);
        assert_eq!(built.players.pose_id[1], PlayerPoseId::CombatGuard);
        assert_eq!(built.players.pose_id[2], PlayerPoseId::CombatStagger);
        assert_eq!(built.players.pose_id[3], PlayerPoseId::Locomotion);
    }
}
