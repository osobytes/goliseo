//! `wasm-bindgen` control surface over `gc_sim::rollback_playable_lab` — the
//! deterministic, network-simulated, bot-driven local rollback harness for
//! the playable match screen.
//!
//! ## The gap this closes
//!
//! `packages/screens/src/match.ts`'s "THE ROLLBACK LABORATORY" section
//! defines `RollbackHostPort`, a package-local interface `MatchScreen`
//! drives when constructed with `rollback_lab` options — but, by that
//! section's own header, there was no real implementation anywhere in this
//! codebase: `@gc/wasm`'s closest rollback surface,
//! [`crate::match_driver_bridge::MatchDriverBridge`], binds
//! `gc_netcode::match_driver` (an OMP-3 TWO-PEER ONLINE driver — `Coordinator`
//! freeze/manifest, `@gc/transport`-shaped envelope queues), not this
//! single-process local dev harness, and `packages/screens/src/
//! match_rollback_lab.spec.ts`/`packages/app/src/match_rollback_lab.spec.ts`
//! both name this precisely (their headers document a hand-written
//! `FakeRollbackHost` that can only ever report `"active"`/
//! `"unconfirmed_window_exceeded"`, never `"converged"`/`"settling"`, because
//! producing a real convergence verdict means running the real algorithm —
//! exactly what this module now exposes).
//!
//! This module does not build `RollbackHostPort` itself (that TypeScript
//! interface, and any `@gc/screens`/`@gc/app` code satisfying it, is outside
//! this crate's ownership this task). It exposes [`RollbackPlayableLab`], a
//! `#[wasm_bindgen]` class over [`gc_sim::rollback_playable_lab`]'s free
//! functions, plus (via [`crate::match_state_bridge`]) the raw `MatchState`
//! a caller building a real `RollbackHostPort` — or feeding
//! `packages/render/src/replay.ts` during a rollback-mode goal — needs and
//! could not get from anywhere else.
//!
//! ## Reused encodings, not reinvented ones
//!
//! [`gc_sim::rollback_playable_lab::advance`]'s batch is built entirely from
//! shapes this crate already binds elsewhere:
//!
//! - `outputs: Vec<gc_sim::rollback_session::RollbackTickOutput>` — the
//!   SAME type [`crate::match_driver_bridge`] already encodes
//!   ([`crate::match_driver_bridge::output_summary_to_json`], now
//!   `pub(crate)`); this module reuses it and layers on the one field that
//!   function drops for its own purpose (`input`, the materialized per-slot
//!   record — [`crate::match_driver_bridge`] does not need it because its
//!   own caller never asks "what did every slot actually play this tick",
//!   only "what did the tick produce", but a rollback-lab consumer
//!   rebuilding `RollbackLabOutput.input.slots[].sample` for its own host
//!   implementation does).
//! - `event_diffs: Vec<gc_sim::rollback_events::RollbackEventDiff>` and
//!   `confirmed_steps: Vec<gc_sim::rollback_events::RollbackEventStep>` —
//!   [`crate::rollback_events_bridge::diff_to_json`]/`step_to_json`, reused
//!   verbatim.
//!
//! Only `corrections` (`gc_sim::rollback_playable_lab::RollbackPlayableCorrection`,
//! a shape unique to this module) and the debug model
//! (`RollbackPlayableLabDebugModel`, likewise unique) get a new encoder
//! here.
//!
//! ## External input never panics; a snapshot handle is trusted
//!
//! Every scalar this module decodes from JS (`options_json`'s
//! `local_slot`/`profile_name`/`max_rollback_ticks`/`settlement_ticks`,
//! `advance`'s `transport_tick`/`sample_wire`) is validated BEFORE it
//! reaches `gc_sim::rollback_playable_lab`, which itself uses `assert!`/
//! `panic!` for these exact preconditions (README rule 5.5: "producer
//! invariants" — see that module's own `# Panics` doc comments). A trap
//! crossing the wasm boundary poisons the whole instance, so this module
//! never lets malformed JS input reach one of those asserts; every rejected
//! case returns a `Result::Err` instead (see [`RollbackPlayableLab::create`]/
//! [`RollbackPlayableLab::advance`]'s own doc for exactly which checks each
//! performs).
//!
//! The one exception, by design: `initial_snapshot`
//! ([`crate::rollback_events_bridge::WasmMatchSnapshot`]) is an OPAQUE
//! handle, not raw JS data — the same trust boundary
//! [`crate::match_driver_bridge::MatchDriverBridge::new`]'s
//! `initial_snapshot_override` already draws (see that constructor's doc).
//! `controlled_initial_snapshot`'s own structural asserts (slot mode,
//! boundary zero, not finished, a mapped local slot) stay panics here,
//! unvalidated by this bridge, exactly like every other `WasmMatchSnapshot`
//! consumer in this crate.
//!
//! ## `profile_name` has no custom-`NetworkProfile` override yet
//!
//! `gc_sim::rollback_playable_lab::RollbackPlayableLabOptions.profile` lets
//! a caller supply an arbitrary `NetworkProfile` instead of an authored
//! name. No test this wave needs that (`match_rollback_lab.spec.ts` and its
//! Lua ancestor both only ever name authored profiles — `"playable"` for the
//! pinned-seed convergence case), and a `NetworkProfile` JSON codec is a
//! straightforward, independent follow-up if a future caller needs one.
//! [`decode_options`] therefore only resolves `profile_name` against the
//! four authored names `gc_sim::rollback_playable_lab`'s own (private)
//! `profile_by_name` recognizes — confirmed by reading that function:
//! `"clean"`, `"omp0_parity"`, `"playable"`, `"stress"`. An unrecognized
//! name is a `Result::Err` here rather than the `panic!` that private
//! function would otherwise produce.

use gc_sim::input_frame::{self, InputSample};
use gc_sim::rollback_input_history::ROLLBACK_WINDOW_TICKS;
use gc_sim::rollback_playable_lab::{
    self, RollbackPlayableConvergence, RollbackPlayableConvergenceStatus,
    RollbackPlayableCorrection, RollbackPlayableLabBatch, RollbackPlayableLabDebugModel,
    RollbackPlayableLabOptions, RollbackPlayableLabStatus,
};
use gc_sim::rollback_session::RollbackTickOutput;
use gc_sim::rollback_snapshot_history::{RollbackSnapshotLookup, RollbackSnapshotLookupStatus};
use wasm_bindgen::prelude::*;

use crate::json::{Json, debug_tag};
use crate::match_state_bridge::match_state_to_json;
use crate::rollback_events_bridge::WasmMatchSnapshot;

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

const KNOWN_PROFILE_NAMES: [&str; 4] = ["clean", "omp0_parity", "playable", "stress"];

/// Decodes `options_json` (see [`RollbackPlayableLab::create`]'s doc) into
/// [`RollbackPlayableLabOptions`], validating every bound
/// `gc_sim::rollback_playable_lab::new` would otherwise `assert!`/`panic!`
/// on — see the module doc's "external input never panics" section.
fn decode_options(options_json: &str) -> Result<RollbackPlayableLabOptions, String> {
    let json = Json::parse(options_json)?;
    let local_slot = json.field_i64("local_slot");
    if let Some(slot) = local_slot
        && !(1..=input_frame::SLOT_COUNT).contains(&slot)
    {
        return Err(format!(
            "playable rollback local_slot must be between 1 and {} (got {slot})",
            input_frame::SLOT_COUNT
        ));
    }
    let profile_name = json.field_str("profile_name").map(str::to_string);
    if let Some(name) = &profile_name
        && !KNOWN_PROFILE_NAMES.contains(&name.as_str())
    {
        return Err(format!(
            "unknown playable rollback profile_name '{name}' (known: {})",
            KNOWN_PROFILE_NAMES.join(", ")
        ));
    }
    let max_rollback_ticks = json.field_i64("max_rollback_ticks");
    if let Some(ticks) = max_rollback_ticks
        && !(1..=ROLLBACK_WINDOW_TICKS).contains(&ticks)
    {
        return Err(format!(
            "playable rollback max_rollback_ticks must be between 1 and {ROLLBACK_WINDOW_TICKS} (got {ticks})"
        ));
    }
    let settlement_ticks = json.field_i64("settlement_ticks");
    if let Some(ticks) = settlement_ticks
        && ticks < 1
    {
        return Err(format!(
            "playable rollback settlement_ticks must be a positive integer (got {ticks})"
        ));
    }
    Ok(RollbackPlayableLabOptions {
        local_slot,
        profile_name,
        profile: None,
        network_seed: json.field_i64("network_seed"),
        bot_seed: json.field_i64("bot_seed"),
        max_rollback_ticks,
        settlement_ticks,
    })
}

/// Validates a [`RollbackPlayableLab::advance`] call BEFORE it reaches
/// `gc_sim::rollback_playable_lab::advance`, which documents every one of
/// these as a `panic!`-worthy producer invariant rather than a `Result`
/// (README rule 5.5) — see the module doc's "external input never panics"
/// section. A plain `Result<(), String>`, not `Result<(), JsValue>|`, so it
/// is unit-testable from a native `#[test]`: `wasm_bindgen::JsValue`
/// construction aborts off the wasm32 target (this crate's established
/// constraint — see `crate::coordinator_bridge`'s module doc), so every
/// `Result<_, JsValue>`-returning method in this crate keeps its actual
/// validation logic in a plain-Rust helper like this one and only wraps the
/// error at the `#[wasm_bindgen]` boundary.
fn validate_advance_call(
    lab: &rollback_playable_lab::RollbackPlayableLab,
    transport_tick: i64,
    sample_present: bool,
) -> Result<(), String> {
    if transport_tick != lab.transport_tick {
        return Err(format!(
            "playable rollback advance expected transport_tick {}, got {transport_tick}",
            lab.transport_tick
        ));
    }
    let needs_sample = rollback_playable_lab::needs_local_sample(lab);
    if needs_sample && !sample_present {
        return Err("playable rollback advance requires a sample_wire while active".to_string());
    }
    if !needs_sample && sample_present {
        return Err(
            "playable rollback advance must not receive a sample_wire during settlement/after finish"
                .to_string(),
        );
    }
    Ok(())
}

fn playable_lab_status_wire(status: RollbackPlayableLabStatus) -> String {
    debug_tag(&status)
}

fn convergence_status_wire(status: RollbackPlayableConvergenceStatus) -> String {
    debug_tag(&status)
}

fn snapshot_lookup_status_wire(status: RollbackSnapshotLookupStatus) -> &'static str {
    match status {
        RollbackSnapshotLookupStatus::Present => "present",
        RollbackSnapshotLookupStatus::Retained => "retained",
        RollbackSnapshotLookupStatus::Missing => "missing",
        RollbackSnapshotLookupStatus::OutsideWindow => "outside_window",
    }
}

fn match_snapshot_difference_json(diff: &gc_sim::match_snapshot::MatchSnapshotDifference) -> Json {
    Json::obj(vec![
        ("path", Json::str(diff.path.clone())),
        ("expected", Json::str(diff.expected.clone())),
        ("actual", Json::str(diff.actual.clone())),
    ])
}

fn convergence_json(c: &RollbackPlayableConvergence) -> Json {
    Json::obj_omit_null(vec![
        ("status", Json::str(convergence_status_wire(c.status))),
        ("boundary", Json::int(c.boundary)),
        ("expected_hash", Json::str(c.expected_hash.clone())),
        ("actual_hash", Json::str(c.actual_hash.clone())),
        (
            "first_difference",
            c.first_difference
                .as_ref()
                .map_or(Json::Null, match_snapshot_difference_json),
        ),
    ])
}

fn correction_json(c: &RollbackPlayableCorrection) -> Json {
    Json::obj_omit_null(vec![
        ("causal_tick", Json::int(c.causal_tick)),
        ("restored_boundary", Json::int(c.restored_boundary)),
        ("old_present_boundary", Json::int(c.old_present_boundary)),
        ("new_present_boundary", Json::int(c.new_present_boundary)),
        ("replaced_from_tick", Json::int(c.replaced_from_tick)),
        ("replaced_through_tick", Json::int(c.replaced_through_tick)),
        ("corrected_from_tick", Json::int(c.corrected_from_tick)),
        (
            "corrected_through_tick",
            Json::int(c.corrected_through_tick),
        ),
        (
            "old_present_hash",
            Json::opt_str(c.old_present_hash.as_deref()),
        ),
        (
            "new_present_hash",
            Json::opt_str(c.new_present_hash.as_deref()),
        ),
        (
            "first_difference",
            c.first_difference
                .as_ref()
                .map_or(Json::Null, match_snapshot_difference_json),
        ),
    ])
}

/// One materialized canonical-slot record
/// (`gc_sim::rollback_input_history::RollbackInputSlotRecord`), as JSON:
/// `source` (`"local"`/`"remote"`), `status`
/// (`"authoritative"`/`"predicted"`), `sample` (a canonical
/// `gc_sim::input_frame` sample wire, matching every other sample crossing
/// in this crate — see `crate::input_frame_bridge`'s module doc).
fn input_slot_record_json(
    record: &gc_sim::rollback_input_history::RollbackInputSlotRecord,
) -> Json {
    use gc_sim::rollback_input_history::{RollbackInputSource, RollbackInputStatus};
    let source = match record.source {
        RollbackInputSource::Local => "local",
        RollbackInputSource::Remote => "remote",
    };
    let status = match record.status {
        RollbackInputStatus::Authoritative => "authoritative",
        RollbackInputStatus::Predicted => "predicted",
    };
    Json::obj(vec![
        ("source", Json::str(source)),
        ("status", Json::str(status)),
        (
            "sample",
            Json::str(
                input_frame::encode_sample(&record.sample)
                    .expect("a materialized rollback sample is always canonical"),
            ),
        ),
    ])
}

fn input_tick_record_json(
    record: &gc_sim::rollback_input_history::RollbackInputTickRecord,
) -> Json {
    Json::obj(vec![
        ("tick", Json::int(record.tick)),
        (
            "slots",
            Json::Array(record.slots.iter().map(input_slot_record_json).collect()),
        ),
    ])
}

/// One [`RollbackTickOutput`], as JSON: reuses
/// [`crate::match_driver_bridge::output_summary_to_json`]'s encoding
/// (tick/boundaries/finished/score/time_left/events/combat_events) and adds
/// `input` (this tick's complete materialized record — see
/// [`input_tick_record_json`]), the one field that function's own caller
/// does not need but a rollback-lab consumer rebuilding
/// `RollbackLabOutput.input.slots[].sample` does. See the module doc's
/// "reused encodings" section.
fn output_json(output: &RollbackTickOutput) -> Json {
    let Json::Object(mut fields) = crate::match_driver_bridge::output_summary_to_json(output)
    else {
        unreachable!("output_summary_to_json always returns a JSON object");
    };
    fields.push(("input".to_string(), input_tick_record_json(&output.input)));
    Json::Object(fields)
}

fn batch_json(batch: &RollbackPlayableLabBatch) -> Json {
    Json::obj(vec![
        (
            "outputs",
            Json::Array(batch.outputs.iter().map(output_json).collect()),
        ),
        (
            "event_diffs",
            Json::Array(
                batch
                    .event_diffs
                    .iter()
                    .map(crate::rollback_events_bridge::diff_to_json)
                    .collect(),
            ),
        ),
        (
            "confirmed_steps",
            Json::Array(
                batch
                    .confirmed_steps
                    .iter()
                    .map(crate::rollback_events_bridge::step_to_json)
                    .collect(),
            ),
        ),
        (
            "corrections",
            Json::Array(batch.corrections.iter().map(correction_json).collect()),
        ),
        ("status", Json::str(playable_lab_status_wire(batch.status))),
    ])
}

fn debug_model_json(model: &RollbackPlayableLabDebugModel) -> Json {
    Json::obj(vec![
        ("profile", Json::str(model.profile.clone())),
        ("local_slot", Json::int(model.local_slot)),
        ("status", Json::str(playable_lab_status_wire(model.status))),
        ("reference_tick", Json::int(model.reference_tick)),
        ("current_tick", Json::int(model.current_tick)),
        ("transport_tick", Json::int(model.transport_tick)),
        (
            "confirmed_input_tick",
            Json::int(model.confirmed_input_tick),
        ),
        (
            "confirmed_output_tick",
            Json::int(model.confirmed_output_tick),
        ),
        (
            "predicted_slot_samples",
            Json::int(model.predicted_slot_samples),
        ),
        ("predicted_ticks", Json::int(model.predicted_ticks)),
        ("correction_count", Json::int(model.correction_count)),
        ("rollback_count", Json::int(model.rollback_count)),
        (
            "latest_rollback_depth",
            Json::int(model.latest_rollback_depth),
        ),
        ("max_rollback_depth", Json::int(model.max_rollback_depth)),
        ("resimulated_ticks", Json::int(model.resimulated_ticks)),
        (
            "retained_snapshot_count",
            Json::int(model.retained_snapshot_count),
        ),
        (
            "retained_snapshot_bytes",
            Json::int(model.retained_snapshot_bytes),
        ),
        ("network_pending", Json::int(model.network_pending)),
        (
            "network_counters",
            Json::obj(vec![
                ("sent", Json::int(model.network_counters.sent)),
                ("delivered", Json::int(model.network_counters.delivered)),
                (
                    "independent_lost",
                    Json::int(model.network_counters.independent_lost),
                ),
                ("burst_lost", Json::int(model.network_counters.burst_lost)),
                ("duplicated", Json::int(model.network_counters.duplicated)),
                ("reordered", Json::int(model.network_counters.reordered)),
                (
                    "history_recovered",
                    Json::int(model.network_counters.history_recovered),
                ),
            ]),
        ),
        (
            "network_high_water",
            Json::obj(vec![
                (
                    "retained_authoritative_records",
                    Json::int(model.network_high_water.retained_authoritative_records),
                ),
                (
                    "delivered_ledger_entries",
                    Json::int(model.network_high_water.delivered_ledger_entries),
                ),
                (
                    "pending_envelopes",
                    Json::int(model.network_high_water.pending_envelopes),
                ),
                (
                    "pending_record_references",
                    Json::int(model.network_high_water.pending_record_references),
                ),
                (
                    "peak_retained_authoritative_records",
                    Json::int(model.network_high_water.peak_retained_authoritative_records),
                ),
                (
                    "peak_delivered_ledger_entries",
                    Json::int(model.network_high_water.peak_delivered_ledger_entries),
                ),
                (
                    "peak_pending_envelopes",
                    Json::int(model.network_high_water.peak_pending_envelopes),
                ),
                (
                    "peak_pending_record_references",
                    Json::int(model.network_high_water.peak_pending_record_references),
                ),
            ]),
        ),
        ("convergence", convergence_json(&model.convergence)),
        ("event_status", Json::str(debug_tag(&model.event_status))),
        (
            "predicted_early_finish",
            Json::bool(model.predicted_early_finish),
        ),
        ("late_input_tick", Json::opt_int(model.late_input_tick)),
        ("settlement_ticks", Json::int(model.settlement_ticks)),
        ("settlement_limit", Json::int(model.settlement_limit)),
    ])
}

/// One `gc_sim::rollback_playable_lab::snapshot` lookup, as a
/// `wasm-bindgen` value type — mirrors
/// [`crate::match_driver_bridge::SnapshotLookup`] (a separate small type
/// rather than reusing that one: its fields are private to that module, and
/// duplicating a three-field wrapper here is cheaper than widening another
/// bridge's visibility for it).
#[wasm_bindgen]
pub struct RollbackPlayableLabSnapshotLookup {
    status: &'static str,
    tick: i64,
    snapshot: Option<gc_sim::match_snapshot::MatchSnapshot>,
}

#[wasm_bindgen]
impl RollbackPlayableLabSnapshotLookup {
    /// This boundary's retention status: `"present"`, `"retained"`,
    /// `"missing"`, or `"outside_window"`.
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
}

impl From<RollbackSnapshotLookup> for RollbackPlayableLabSnapshotLookup {
    fn from(lookup: RollbackSnapshotLookup) -> Self {
        RollbackPlayableLabSnapshotLookup {
            status: snapshot_lookup_status_wire(lookup.status),
            tick: lookup.tick,
            snapshot: lookup.snapshot,
        }
    }
}

/// The playable rollback laboratory for one local dev/playtest match — see
/// the module doc.
#[wasm_bindgen]
pub struct RollbackPlayableLab {
    inner: rollback_playable_lab::RollbackPlayableLab,
}

#[wasm_bindgen]
impl RollbackPlayableLab {
    /// Builds a fresh laboratory from `initial_snapshot` (the canonical
    /// slot-mode boundary-zero snapshot — e.g.
    /// [`crate::session::Session::snapshot_handle`], taken before the
    /// session's first [`crate::session::Session::step`]) and `options_json`,
    /// a JSON object: `local_slot` (1..=8, default 1), `profile_name`
    /// (default `"playable"` — see the module doc for the four recognized
    /// names), `network_seed`, `bot_seed`, `max_rollback_ticks`
    /// (1..=30, `gc_sim::rollback_input_history::ROLLBACK_WINDOW_TICKS`),
    /// `settlement_ticks` (>=1). Every field is optional; an absent one
    /// defaults exactly the way `gc_sim::rollback_playable_lab::new` does.
    /// Named `create`, not `new` — mirrors
    /// [`crate::rollback_events_bridge::RollbackEventsTimeline::create`]'s
    /// own naming rationale (`new` is JS's own reserved constructor call).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `options_json` fails to parse, or
    /// `local_slot`/`profile_name`/`max_rollback_ticks`/`settlement_ticks`
    /// violate a bound `gc_sim::rollback_playable_lab::new` would otherwise
    /// panic on — see the module doc's "external input never panics"
    /// section. Still panics (a Rust panic, not a `Result`) on a structurally
    /// invalid `initial_snapshot` (not slot mode, not boundary zero, already
    /// finished, or an unmapped local slot) — that handle is trusted,
    /// Rust-originated data, not raw JS input; see the module doc.
    #[wasm_bindgen(js_name = create)]
    pub fn create(
        initial_snapshot: &WasmMatchSnapshot,
        options_json: &str,
    ) -> Result<RollbackPlayableLab, JsValue> {
        let options = decode_options(options_json).map_err(js_err)?;
        Ok(RollbackPlayableLab {
            inner: rollback_playable_lab::new(&initial_snapshot.snapshot, options),
        })
    }

    /// Whether [`RollbackPlayableLab::advance`] requires a `sample_wire`
    /// this tick. Cheap; safe to call before the first `advance`.
    #[wasm_bindgen(js_name = needsLocalSample)]
    #[must_use]
    pub fn needs_local_sample(&self) -> bool {
        rollback_playable_lab::needs_local_sample(&self.inner)
    }

    /// Advances exactly one transport tick. `transport_tick` must equal the
    /// tick this laboratory expects next (readable beforehand via
    /// [`RollbackPlayableLab::debug_model_json`]'s `transport_tick`).
    /// `sample_wire` (a canonical `gc_sim::input_frame` sample wire — see
    /// `crate::input_frame_bridge`'s module doc) must be supplied exactly
    /// when [`RollbackPlayableLab::needs_local_sample`] is `true` this tick,
    /// and omitted otherwise. Returns the tick's batch as JSON: `outputs`
    /// (see [`output_json`]), `event_diffs`
    /// (`crate::rollback_events_bridge::diff_to_json`'s shape),
    /// `confirmed_steps` (`crate::rollback_events_bridge::step_to_json`'s
    /// shape), `corrections` (see [`correction_json`]), and `status` (this
    /// laboratory's status after this call — `"active"`, `"settling"`,
    /// `"converged"`, `"diverged"`, `"late_input_unrecoverable"`,
    /// `"unconfirmed_window_exceeded"`, `"comparison_history_missing"`, or
    /// `"drain_incomplete"`).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `transport_tick` does not equal
    /// the expected next tick, if `sample_wire` fails to decode, or if
    /// `sample_wire`'s presence disagrees with
    /// [`RollbackPlayableLab::needs_local_sample`] — every one of these is
    /// a precondition `gc_sim::rollback_playable_lab::advance` itself
    /// documents as a `panic!`-worthy producer invariant (see that
    /// function's `# Panics` doc); this method validates all three before
    /// calling into it so malformed JS input can never reach that panic
    /// (see the module doc).
    pub fn advance(
        &mut self,
        transport_tick: f64,
        sample_wire: Option<String>,
    ) -> Result<String, JsValue> {
        let transport_tick = transport_tick as i64;
        validate_advance_call(&self.inner, transport_tick, sample_wire.is_some())
            .map_err(js_err)?;
        let sample: Option<InputSample> = sample_wire
            .map(|wire| input_frame::decode_sample(&wire).map_err(|err| js_err(err.to_string())))
            .transpose()?;
        let batch = rollback_playable_lab::advance(&mut self.inner, transport_tick, sample);
        Ok(batch_json(&batch).to_json_string())
    }

    /// This laboratory's retained-state shape, as JSON — see
    /// [`debug_model_json`] for the exact fields (mirrors
    /// `gc_sim::rollback_playable_lab::debug_model`/the Lua original's
    /// `RollbackPlayableLabDebugModel`).
    #[wasm_bindgen(js_name = debugModelJson)]
    #[must_use]
    pub fn debug_model_json(&self) -> String {
        debug_model_json(&rollback_playable_lab::debug_model(&self.inner)).to_json_string()
    }

    /// An independent capture of the predicted client's current live
    /// boundary, as an opaque handle — see
    /// [`crate::rollback_events_bridge::WasmMatchSnapshot`]'s doc.
    #[wasm_bindgen(js_name = currentSnapshot)]
    #[must_use]
    pub fn current_snapshot(&self) -> WasmMatchSnapshot {
        WasmMatchSnapshot::new(rollback_playable_lab::current_snapshot(&self.inner))
    }

    /// An independent capture of the reference's current live boundary, as
    /// an opaque handle.
    #[wasm_bindgen(js_name = referenceSnapshot)]
    #[must_use]
    pub fn reference_snapshot(&self) -> WasmMatchSnapshot {
        WasmMatchSnapshot::new(rollback_playable_lab::reference_snapshot(&self.inner))
    }

    /// Looks up the predicted client's own retained boundary-snapshot
    /// history at `boundary_tick`.
    #[wasm_bindgen(js_name = snapshotLookup)]
    #[must_use]
    pub fn snapshot_lookup(&self, boundary_tick: f64) -> RollbackPlayableLabSnapshotLookup {
        rollback_playable_lab::snapshot(&self.inner, boundary_tick as i64).into()
    }

    /// The predicted client's current live boundary's RAW simulation state,
    /// as JSON — `crate::match_state_bridge::match_state_to_json`'s shape,
    /// for a caller feeding `packages/render/src/replay.ts` (or any other
    /// raw-`MatchState` consumer) during rollback-mode play. See
    /// `crate::match_state_bridge`'s module doc.
    #[wasm_bindgen(js_name = currentMatchStateJson)]
    #[must_use]
    pub fn current_match_state_json(&self) -> String {
        let snapshot = rollback_playable_lab::current_snapshot(&self.inner);
        let (state, _combat) = gc_sim::match_snapshot::restore_owned(&snapshot);
        match_state_to_json(&state).to_json_string()
    }

    /// The independent reference's current live boundary's RAW simulation
    /// state, as JSON — see [`RollbackPlayableLab::current_match_state_json`].
    #[wasm_bindgen(js_name = referenceMatchStateJson)]
    #[must_use]
    pub fn reference_match_state_json(&self) -> String {
        let snapshot = rollback_playable_lab::reference_snapshot(&self.inner);
        let (state, _combat) = gc_sim::match_snapshot::restore_owned(&snapshot);
        match_state_to_json(&state).to_json_string()
    }

    /// A retained boundary's RAW simulation state, as JSON — see
    /// [`RollbackPlayableLab::current_match_state_json`]. Use
    /// [`RollbackPlayableLab::snapshot_lookup`] first to confirm
    /// `boundary_tick` is `"present"`/`"retained"`.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `boundary_tick` is not currently
    /// retained (`"missing"` or `"outside_window"` — an expected, recoverable
    /// case for a boundary a caller has not confirmed is retained yet, never
    /// a panic).
    #[wasm_bindgen(js_name = matchStateJsonAt)]
    pub fn match_state_json_at(&self, boundary_tick: f64) -> Result<String, JsValue> {
        let lookup = rollback_playable_lab::snapshot(&self.inner, boundary_tick as i64);
        let snapshot = lookup.snapshot.ok_or_else(|| {
            js_err(format!(
                "playable rollback boundary {boundary_tick} is not retained (status: {})",
                snapshot_lookup_status_wire(lookup.status)
            ))
        })?;
        let (state, _combat) = gc_sim::match_snapshot::restore_owned(&snapshot);
        Ok(match_state_to_json(&state).to_json_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    fn fresh_lab_with_duration(
        local_slot: i64,
        profile_name: &str,
        duration_seconds: f64,
    ) -> RollbackPlayableLab {
        let session = Session::new("nebula", "orion", 7.0, duration_seconds, 3, None)
            .expect("the fixture team ids always construct a valid session");
        let snapshot = session.snapshot_handle();
        let options = format!(
            r#"{{"local_slot":{local_slot},"profile_name":"{profile_name}","network_seed":7302,"bot_seed":7400}}"#
        );
        RollbackPlayableLab::create(&snapshot, &options).expect("valid options construct a lab")
    }

    fn fresh_lab(local_slot: i64, profile_name: &str) -> RollbackPlayableLab {
        fresh_lab_with_duration(local_slot, profile_name, 20.0)
    }

    fn neutral_wire() -> String {
        input_frame::encode_sample(&input_frame::neutral_sample())
            .expect("the neutral sample is always canonical")
    }

    // `create`/`advance`'s error paths construct a `wasm_bindgen::JsValue`
    // (via `js_err`), which aborts the whole native test process off the
    // wasm32 target (`function not implemented on non-wasm32 targets` — the
    // same constraint `crate::coordinator_bridge`'s module doc documents for
    // every other exported error path in this crate). So these cases test
    // the plain-`Result<_, String>` validation helpers directly
    // (`decode_options`/`validate_advance_call`) rather than the
    // `#[wasm_bindgen]` methods that wrap them — `packages/wasm/src/*.spec.ts`
    // is where a `Result<_, JsValue>` error path gets exercised end to end,
    // against the real compiled artifact.

    #[test]
    fn decode_options_rejects_an_out_of_range_local_slot_without_panicking() {
        let err = decode_options(r#"{"local_slot":9}"#)
            .expect_err("an out-of-range local_slot is rejected");
        assert!(
            err.contains("local_slot"),
            "the error must name the offending field"
        );
    }

    #[test]
    fn decode_options_rejects_an_unknown_profile_name() {
        assert!(decode_options(r#"{"profile_name":"not_a_real_profile"}"#).is_err());
    }

    #[test]
    fn decode_options_rejects_an_out_of_range_max_rollback_ticks_and_settlement_ticks() {
        assert!(decode_options(r#"{"max_rollback_ticks":0}"#).is_err());
        assert!(decode_options(r#"{"max_rollback_ticks":31}"#).is_err());
        assert!(decode_options(r#"{"settlement_ticks":0}"#).is_err());
    }

    #[test]
    fn decode_options_accepts_every_known_profile_name_and_defaults_the_rest() {
        for name in KNOWN_PROFILE_NAMES {
            let options = decode_options(&format!(r#"{{"profile_name":"{name}"}}"#))
                .expect("every known profile name is accepted");
            assert_eq!(options.profile_name.as_deref(), Some(name));
            assert_eq!(
                options.local_slot, None,
                "an absent field stays None (defaulted later)"
            );
        }
    }

    #[test]
    fn a_fresh_lab_starts_active_and_needs_a_local_sample() {
        let lab = fresh_lab(1, "clean");
        assert!(lab.needs_local_sample());
        let debug = Json::parse(&lab.debug_model_json()).unwrap();
        assert_eq!(debug.field_str("status"), Some("active"));
        assert_eq!(debug.field_i64("transport_tick"), Some(0));
    }

    #[test]
    fn validate_advance_call_rejects_a_non_contiguous_transport_tick() {
        let lab = fresh_lab(1, "clean");
        let err = validate_advance_call(&lab.inner, 5, true).unwrap_err();
        assert!(err.contains("transport_tick"));
    }

    #[test]
    fn validate_advance_call_rejects_a_missing_sample_while_active() {
        let lab = fresh_lab(1, "clean");
        assert!(validate_advance_call(&lab.inner, 0, false).is_err());
    }

    #[test]
    fn validate_advance_call_rejects_a_present_sample_during_settlement() {
        // A lab that has already fully finished the reference match no
        // longer needs a local sample; a caller supplying one anyway must
        // be rejected, not panic -- mirrors `advance_reference`'s own
        // assertion in `gc_sim::rollback_playable_lab`. A short duration
        // (a handful of ticks) keeps this fast: the point under test is the
        // precondition check, not a realistic match length.
        let mut lab = fresh_lab_with_duration(1, "clean", 3.0 / 60.0);
        loop {
            if !rollback_playable_lab::needs_local_sample(&lab.inner) {
                break;
            }
            let tick = lab.inner.transport_tick;
            lab.advance(tick as f64, Some(neutral_wire())).unwrap();
        }
        let tick = lab.inner.transport_tick;
        assert!(validate_advance_call(&lab.inner, tick, true).is_err());
    }

    #[test]
    fn advance_steps_the_clean_profile_and_reports_a_well_shaped_batch() {
        let mut lab = fresh_lab(1, "clean");
        for tick in 0..10i64 {
            let batch = Json::parse(
                &lab.advance(tick as f64, Some(neutral_wire()))
                    .expect("a contiguous tick with a valid sample always advances"),
            )
            .unwrap();
            assert!(batch.get("outputs").and_then(Json::as_array).is_some());
            assert!(batch.field_str("status").is_some());
        }
        let debug = Json::parse(&lab.debug_model_json()).unwrap();
        assert_eq!(debug.field_i64("transport_tick"), Some(10));
        // The clean profile confirms every tick immediately (no delay), so
        // the client and reference stay in lockstep with no rollback.
        assert_eq!(debug.field_i64("rollback_count"), Some(0));
    }

    #[test]
    fn a_pinned_seed_playable_profile_eventually_converges() {
        // Mirrors the still-skipped tier-3 case's own scenario
        // (`match_rollback_lab.spec.ts`'s header): the checked-in
        // `"playable"` profile, pinned network/bot seeds, run to full time
        // plus settlement. This proves the underlying algorithm this
        // module exposes actually reaches `"converged"` -- the exact
        // capability no fake anywhere in this codebase could produce.
        let mut lab = fresh_lab(1, "playable");
        let mut transport_tick = 0i64;
        let mut saw_rollback = false;
        loop {
            let debug = Json::parse(&lab.debug_model_json()).unwrap();
            let status = debug.field_str("status").unwrap().to_string();
            if status != "active" && status != "settling" {
                assert_eq!(
                    status, "converged",
                    "a pinned-seed clean run must converge, not diverge or stall"
                );
                break;
            }
            if debug.field_i64("rollback_count").unwrap_or(0) > 0 {
                saw_rollback = true;
            }
            let needs_sample = lab.needs_local_sample();
            let batch_json = lab
                .advance(
                    transport_tick as f64,
                    if needs_sample {
                        Some(neutral_wire())
                    } else {
                        None
                    },
                )
                .expect("a contiguous tick always advances");
            let batch = Json::parse(&batch_json).unwrap();
            assert!(batch.get("outputs").and_then(Json::as_array).is_some());
            transport_tick += 1;
            assert!(
                transport_tick < 100_000,
                "a pinned twenty-second fixture must converge well before this bound"
            );
        }
        assert!(
            saw_rollback,
            "the playable profile's own base delay must produce at least one rollback \
             before convergence -- otherwise this run never exercised the reconciliation \
             path at all"
        );
    }
}
