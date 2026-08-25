//! `wasm-bindgen` control surface over `gc_netcode::coordinator`'s reducer.
//!
//! This is the follow-up wave 1's [`crate`] doc flags: `CoordinatorState`/
//! `Event` carry no `serde` impl, and this module is the hand-written bridge
//! instead — see the crate doc for why hand-written won, and [`crate::json`]
//! for the encoder it is built on.
//!
//! ## The queue/drain seam
//!
//! Every event that can originate from the network — an inbound control
//! message, a lost link — is queued via [`Coordinator::enqueue_control_wire`]/
//! [`Coordinator::enqueue_link_lost`] and only actually applied to the
//! reducer inside [`Coordinator::tick`], which drains the queue once, in
//! arrival order, folds each queued event through
//! `gc_netcode::coordinator::step` in sequence, and finally applies
//! [`gc_netcode::coordinator::Event::Tick`] to advance the logical clock —
//! all as one atomic JS call. See [`crate::net_inbox`] for the discipline
//! this rests on and the test that proves it.
//!
//! Events that originate locally (this peer's own UI intent — proposing a
//! manifest, setting readiness, and so on) are already synchronous
//! JS-to-Rust calls with no async gap to defend against, so they apply
//! immediately rather than queuing — queuing them would only delay a local
//! actor's own action for no correctness reason.
//!
//! ## Wire shapes
//!
//! Every method that takes or returns structured coordinator data (a
//! manifest, an ownership plan, a full outcome) uses JSON text via
//! [`crate::json::Json`]. `gc_netcode::protocol::Value` bridges to JSON via
//! [`value_to_json`]/[`value_from_json`] below — a table with a dense
//! `1..=n` integer key run serializes as a JSON array (matching
//! `Value::array`'s own construction), anything else as a JSON object with
//! integer keys stringified (matching `Value::record`). A round trip
//! through [`value_from_json`]/[`value_to_json`] is therefore lossless for
//! every `Value` this crate's own constructors produce.

use gc_netcode::coordinator::{
    self, Action, CoordinatorState, Departure, Disposition, Event, Freeze, HashRecord,
    ManifestExpectation, MatchProgress, MatchResult, Options, Outcome, Peer as CoordinatorPeer,
    Preference, PreferenceState, RejectCode, Role, Summary, Terminal,
};
use gc_netcode::protocol::{self, LifecyclePhase, Value};
use gc_sim::input_frame::SlotId;
use indexmap::IndexMap;
use wasm_bindgen::prelude::*;

use crate::json::Json;
use crate::json::debug_tag as tag;
use crate::net_inbox::TickQueue;

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

fn parse_json(text: &str) -> Result<Json, JsValue> {
    Json::parse(text).map_err(js_err)
}

// ---------------------------------------------------------------------------
// `protocol::Value` <-> JSON.
// ---------------------------------------------------------------------------

/// Converts a `gc_netcode::protocol::Value` to [`Json`]. See the module doc
/// for the array/object rule.
#[must_use]
pub(crate) fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Nil => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Int(number) => Json::int(*number),
        Value::Str(text) => Json::str(text.clone()),
        Value::Table(entries) => {
            let is_dense_array = !entries.is_empty()
                && entries.iter().enumerate().all(
                    |(index, (key, _))| matches!(key, Value::Int(n) if *n == index as i64 + 1),
                );
            if is_dense_array {
                Json::Array(
                    entries
                        .iter()
                        .map(|(_, item)| value_to_json(item))
                        .collect(),
                )
            } else {
                Json::Object(
                    entries
                        .iter()
                        .map(|(key, item)| {
                            let key_text = match key {
                                Value::Str(text) => text.clone(),
                                Value::Int(number) => number.to_string(),
                                _ => String::new(),
                            };
                            (key_text, value_to_json(item))
                        })
                        .collect(),
                )
            }
        }
    }
}

/// The reverse of [`value_to_json`]. `Err` for a JSON number carrying a
/// fractional part — every `protocol::Value::Int` is a bounded integer
/// (`protocol.rs`'s own module doc), so a fraction here is a caller error,
/// not something to round away silently.
pub(crate) fn value_from_json(json: &Json) -> Result<Value, String> {
    match json {
        Json::Null => Ok(Value::Nil),
        Json::Bool(flag) => Ok(Value::Bool(*flag)),
        Json::Number(number) => {
            if number.fract() != 0.0 {
                return Err(
                    "protocol values are canonical integers, not fractional numbers".to_string(),
                );
            }
            Ok(Value::Int(*number as i64))
        }
        Json::String(text) => Ok(Value::Str(text.clone())),
        Json::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(value_from_json(item)?);
            }
            Ok(Value::array(values))
        }
        Json::Object(fields) => {
            let mut pairs = Vec::with_capacity(fields.len());
            for (key, item) in fields {
                pairs.push((key.as_str(), value_from_json(item)?));
            }
            Ok(Value::record(pairs))
        }
    }
}

// ---------------------------------------------------------------------------
// Coordinator-local shapes -> JSON.
// ---------------------------------------------------------------------------

fn manifest_expectation_from_json(json: &Json) -> ManifestExpectation {
    ManifestExpectation {
        build_id: json.field_str("build_id").map(str::to_string),
        source_id: json.field_str("source_id").map(str::to_string),
        content_id: json.field_str("content_id").map(str::to_string),
        tuning_id: json.field_str("tuning_id").map(str::to_string),
        match_config_id: json.field_str("match_config_id").map(str::to_string),
        fixture_id: json.field_str("fixture_id").map(str::to_string),
        arena_id: json.field_str("arena_id").map(str::to_string),
        combat_rules_id: json.field_str("combat_rules_id").map(str::to_string),
        gameplay_ai_policy_id: json.field_str("gameplay_ai_policy_id").map(str::to_string),
        combat_status: json.field_str("combat_status").map(str::to_string),
    }
}

fn slot_ids_to_json(slots: &[SlotId]) -> Json {
    Json::Array(
        slots
            .iter()
            .map(|slot| Json::str(protocol::slot_wire_id(*slot)))
            .collect(),
    )
}

fn indexed_slot_map_to_json<T>(entries: &IndexMap<String, T>, each: impl Fn(&T) -> Json) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(key, value): (&String, &T)| (key.clone(), each(value)))
            .collect(),
    )
}

pub(crate) fn freeze_to_json(freeze: &Freeze) -> Json {
    Json::obj(vec![
        ("manifest_id", Json::str(freeze.manifest_id.clone())),
        ("assignment_id", Json::str(freeze.assignment_id.clone())),
        ("countdown_id", Json::str(freeze.countdown_id.clone())),
        ("first_input_tick", Json::int(freeze.first_input_tick)),
        ("seed", Json::int(freeze.seed)),
        ("tick_rate", Json::int(freeze.tick_rate)),
        ("duration_ticks", Json::int(freeze.duration_ticks)),
        ("max_goals", Json::int(freeze.max_goals)),
        ("content_id", Json::str(freeze.content_id.clone())),
        ("tuning_id", Json::str(freeze.tuning_id.clone())),
        ("combat_rules_id", Json::str(freeze.combat_rules_id.clone())),
        (
            "gameplay_ai_policy_id",
            Json::str(freeze.gameplay_ai_policy_id.clone()),
        ),
        ("combat_status", Json::str(freeze.combat_status.clone())),
        ("match_mode", Json::str(freeze.match_mode.wire_str())),
        ("assignments", value_to_json(&freeze.assignments)),
        (
            "owned",
            indexed_slot_map_to_json(&freeze.owned, |slots| slot_ids_to_json(slots)),
        ),
        (
            "live",
            indexed_slot_map_to_json(&freeze.live, |slot| {
                Json::str(protocol::slot_wire_id(*slot))
            }),
        ),
    ])
}

/// The reverse of [`freeze_to_json`]. Used by [`crate::match_driver_bridge`]
/// to reconstruct the real [`Freeze`] a coordinator produced (as
/// [`Action::StartMatch`]) from the JSON [`Coordinator::tick`] handed back
/// to JS — see that module's doc for why the real `Freeze` is what a
/// [`gc_netcode::match_driver_fixture::DriverRules`] needs, not
/// `gc_netcode::match_driver`'s own narrower placeholder shape.
///
/// # Errors
///
/// Returns a message describing the first structurally invalid or missing
/// field.
pub(crate) fn freeze_from_json(json: &Json) -> Result<Freeze, String> {
    let field_str = |key: &str| -> Result<String, String> {
        json.field_str(key)
            .map(str::to_string)
            .ok_or_else(|| format!("freeze json is missing string field '{key}'"))
    };
    let field_int = |key: &str| -> Result<i64, String> {
        json.field_i64(key)
            .ok_or_else(|| format!("freeze json is missing integer field '{key}'"))
    };
    let slot_from_wire = |wire: &str| -> Result<gc_sim::input_frame::SlotId, String> {
        protocol::slot_id_from_wire(wire)
            .ok_or_else(|| format!("freeze json names an unrecognized canonical slot '{wire}'"))
    };
    let match_mode_wire = field_str("match_mode")?;
    let match_mode = protocol::MatchMode::from_wire_str(&match_mode_wire).ok_or_else(|| {
        format!("freeze json names an unrecognized match mode '{match_mode_wire}'")
    })?;
    let assignments = value_from_json(
        json.field("assignments")
            .ok_or_else(|| "freeze json is missing field 'assignments'".to_string())?,
    )?;
    let mut owned = IndexMap::new();
    if let Some(Json::Object(fields)) = json.field("owned") {
        for (peer_id, slots_json) in fields {
            let slots_json = slots_json
                .as_array()
                .ok_or_else(|| "freeze json 'owned' entry must be an array".to_string())?;
            let mut slots = Vec::with_capacity(slots_json.len());
            for slot_json in slots_json {
                let wire = slot_json
                    .as_str()
                    .ok_or_else(|| "freeze json 'owned' slot must be a string".to_string())?;
                slots.push(slot_from_wire(wire)?);
            }
            owned.insert(peer_id.clone(), slots);
        }
    }
    let mut live = IndexMap::new();
    if let Some(Json::Object(fields)) = json.field("live") {
        for (peer_id, slot_json) in fields {
            let wire = slot_json
                .as_str()
                .ok_or_else(|| "freeze json 'live' entry must be a string".to_string())?;
            live.insert(peer_id.clone(), slot_from_wire(wire)?);
        }
    }
    Ok(Freeze {
        manifest_id: field_str("manifest_id")?,
        assignment_id: field_str("assignment_id")?,
        countdown_id: field_str("countdown_id")?,
        first_input_tick: field_int("first_input_tick")?,
        seed: field_int("seed")?,
        tick_rate: field_int("tick_rate")?,
        duration_ticks: field_int("duration_ticks")?,
        max_goals: field_int("max_goals")?,
        content_id: field_str("content_id")?,
        tuning_id: field_str("tuning_id")?,
        combat_rules_id: field_str("combat_rules_id")?,
        gameplay_ai_policy_id: field_str("gameplay_ai_policy_id")?,
        combat_status: field_str("combat_status")?,
        match_mode,
        assignments,
        owned,
        live,
    })
}

fn terminal_to_json(terminal: &Terminal) -> Json {
    Json::obj(vec![
        ("reason", Json::str(tag(&terminal.reason))),
        ("code", Json::opt_str(terminal.code.as_deref())),
        ("origin", Json::str(tag(&terminal.origin))),
        ("peer_id", Json::opt_str(terminal.peer_id.as_deref())),
        ("detail", Json::opt_str(terminal.detail.as_deref())),
    ])
}

fn action_to_json(action: &Action) -> Json {
    match action {
        Action::Send { targets, message } => Json::obj(vec![
            ("kind", Json::str("send")),
            (
                "targets",
                Json::Array(targets.iter().map(|t| Json::str(t.clone())).collect()),
            ),
            (
                "wire",
                Json::str(
                    protocol::encode(message)
                        .expect("a coordinator only ever emits an already-valid control message"),
                ),
            ),
        ]),
        Action::Close { link_id } => Json::obj(vec![
            ("kind", Json::str("close")),
            ("link_id", Json::str(link_id.clone())),
        ]),
        Action::StartMatch { freeze } => Json::obj(vec![
            ("kind", Json::str("start_match")),
            ("freeze", freeze_to_json(freeze)),
        ]),
        Action::Terminate { terminal } => Json::obj(vec![
            ("kind", Json::str("terminate")),
            ("terminal", terminal_to_json(terminal)),
        ]),
    }
}

fn outcome_to_json(outcome: &Outcome) -> Json {
    Json::obj(vec![
        ("accepted", Json::bool(outcome.accepted)),
        (
            "disposition",
            Json::str(disposition_str(outcome.disposition)),
        ),
        (
            "code",
            Json::opt_str(outcome.code.map(reject_code_str).as_deref()),
        ),
        ("reason", Json::opt_str(outcome.reason.as_deref())),
        (
            "actions",
            Json::Array(outcome.actions.iter().map(action_to_json).collect()),
        ),
    ])
}

fn disposition_str(value: Disposition) -> String {
    tag(&value)
}

fn reject_code_str(value: RejectCode) -> String {
    tag(&value)
}

fn preference_to_json(preference: &Preference) -> Json {
    Json::obj(vec![
        ("slots", slot_ids_to_json(&preference.slots)),
        ("assignment_id", Json::str(preference.assignment_id.clone())),
        (
            "status",
            Json::str(tag::<PreferenceState>(&preference.status)),
        ),
        ("reason", Json::opt_str(preference.reason.as_deref())),
        ("deadline", Json::opt_int(preference.deadline)),
    ])
}

fn progress_to_json(progress: &MatchProgress) -> Json {
    Json::obj(vec![
        ("phase", Json::str(progress.phase.clone())),
        ("tick", Json::int(progress.tick)),
        ("home_score", Json::int(progress.home_score)),
        ("away_score", Json::int(progress.away_score)),
    ])
}

fn result_to_json(result: &MatchResult) -> Json {
    Json::obj(vec![
        ("final_tick", Json::int(result.final_tick)),
        ("home_score", Json::int(result.home_score)),
        ("away_score", Json::int(result.away_score)),
        ("final_hash", Json::str(result.final_hash.clone())),
    ])
}

fn hash_record_to_json(record: &HashRecord) -> Json {
    Json::obj(vec![
        ("tick", Json::int(record.tick)),
        ("hash", Json::str(record.hash.clone())),
    ])
}

fn departure_to_json(departure: &Departure) -> Json {
    Json::obj(vec![
        ("peer_id", Json::str(departure.peer_id.clone())),
        ("reason", Json::str(tag(&departure.reason))),
        ("code", Json::str(departure.code.clone())),
        ("detail", Json::opt_str(departure.detail.as_deref())),
    ])
}

fn peer_to_json(peer: &CoordinatorPeer) -> Json {
    Json::obj(vec![
        ("peer_id", Json::str(peer.peer_id.clone())),
        ("link_id", Json::opt_str(peer.link_id.as_deref())),
        ("role", Json::str(peer.role.wire_str())),
        ("runtime", value_to_json(&peer.runtime)),
        ("build_id", Json::opt_str(peer.build_id.as_deref())),
        (
            "accepted_manifest_id",
            Json::opt_str(peer.accepted_manifest_id.as_deref()),
        ),
        ("assigned", Json::bool(peer.assigned)),
        ("ready", Json::bool(peer.ready)),
        ("started", Json::bool(peer.started)),
        ("result_acked", Json::bool(peer.result_acked)),
        ("hash_mismatches", Json::int(peer.hash_mismatches)),
        ("last_sequence", Json::int(peer.last_sequence)),
        (
            "pair_choice",
            match &peer.pair_choice {
                Some(slots) => slot_ids_to_json(slots),
                None => Json::Null,
            },
        ),
    ])
}

fn summary_to_json(summary: &Summary) -> Json {
    Json::obj(vec![
        ("role", Json::str(summary.role.wire_str())),
        ("phase", Json::str(phase_str(summary.phase))),
        ("peer_count", Json::int(summary.peer_count as i64)),
        ("ready_count", Json::int(summary.ready_count as i64)),
        ("manifest_id", Json::opt_str(summary.manifest_id.as_deref())),
        ("frozen", Json::bool(summary.frozen)),
        (
            "terminal",
            summary
                .terminal
                .as_ref()
                .map_or(Json::Null, terminal_to_json),
        ),
    ])
}

fn phase_str(phase: LifecyclePhase) -> String {
    tag(&phase)
}

fn state_to_json(state: &CoordinatorState) -> Json {
    Json::obj(vec![
        ("version", Json::int(state.version)),
        ("role", Json::str(state.role.wire_str())),
        ("session_id", Json::str(state.session_id.clone())),
        ("peer_id", Json::str(state.peer_id.clone())),
        ("host_peer_id", Json::str(state.host_peer_id.clone())),
        ("host_link_id", Json::opt_str(state.host_link_id.as_deref())),
        ("runtime", value_to_json(&state.runtime)),
        ("build_id", Json::opt_str(state.build_id.as_deref())),
        ("phase", Json::str(phase_str(state.phase))),
        ("clock", Json::int(state.clock)),
        ("sequence", Json::int(state.sequence)),
        (
            "peers",
            Json::Array(state.peers.iter().map(peer_to_json).collect()),
        ),
        (
            "manifest",
            state.manifest.as_ref().map_or(Json::Null, value_to_json),
        ),
        ("manifest_id", Json::opt_str(state.manifest_id.as_deref())),
        (
            "assignments",
            state.assignments.as_ref().map_or(Json::Null, value_to_json),
        ),
        (
            "assignment_id",
            Json::opt_str(state.assignment_id.as_deref()),
        ),
        ("assignment_epoch", Json::int(state.assignment_epoch)),
        (
            "preference",
            state
                .preference
                .as_ref()
                .map_or(Json::Null, preference_to_json),
        ),
        (
            "freeze",
            state.freeze.as_ref().map_or(Json::Null, freeze_to_json),
        ),
        (
            "countdown_remaining",
            Json::opt_int(state.countdown_remaining),
        ),
        ("start_armed_at", Json::opt_int(state.start_armed_at)),
        ("start_resend_at", Json::opt_int(state.start_resend_at)),
        (
            "progress",
            state.progress.as_ref().map_or(Json::Null, progress_to_json),
        ),
        (
            "result",
            state.result.as_ref().map_or(Json::Null, result_to_json),
        ),
        (
            "local_result",
            state
                .local_result
                .as_ref()
                .map_or(Json::Null, result_to_json),
        ),
        (
            "hashes",
            Json::Array(state.hashes.iter().map(hash_record_to_json).collect()),
        ),
        (
            "departure",
            state
                .departure
                .as_ref()
                .map_or(Json::Null, departure_to_json),
        ),
        (
            "terminal",
            state.terminal.as_ref().map_or(Json::Null, terminal_to_json),
        ),
    ])
}

// ---------------------------------------------------------------------------
// The queue/drain seam.
// ---------------------------------------------------------------------------

/// One network-originated item, queued by [`Coordinator::enqueue_control_wire`]/
/// [`Coordinator::enqueue_link_lost`] and only turned into a
/// `gc_netcode::coordinator::Event` inside [`Coordinator::tick`]. See the
/// module doc and [`crate::net_inbox`].
enum QueuedEvent {
    Control {
        link_id: String,
        wire: String,
    },
    LinkLost {
        link_id: String,
        code: Option<String>,
    },
}

impl QueuedEvent {
    fn into_event(self) -> Event {
        match self {
            QueuedEvent::Control { link_id, wire } => Event::Control {
                link_id,
                message: None,
                wire: Some(wire),
            },
            QueuedEvent::LinkLost { link_id, code } => Event::LinkLost { link_id, code },
        }
    }
}

/// One session coordinator (host or guest). See the module doc.
#[wasm_bindgen]
pub struct Coordinator {
    state: CoordinatorState,
    inbox: TickQueue<QueuedEvent>,
}

#[wasm_bindgen]
impl Coordinator {
    /// Builds a fresh coordinator. `role` is `"host"` or `"guest"`.
    /// `runtime_json`/`expectation_json` are JSON encodings of
    /// `gc_netcode::protocol::Value`/`ManifestExpectation` (see the module
    /// doc's array/object rule for `Value`).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `role` is unrecognized,
    /// `runtime_json`/`expectation_json` fail to parse, or the coordinator's
    /// own construction invariants reject the options.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        role: &str,
        session_id: &str,
        peer_id: &str,
        host_peer_id: Option<String>,
        host_link_id: Option<String>,
        runtime_json: &str,
        build_id: Option<String>,
        expectation_json: Option<String>,
    ) -> Result<Coordinator, JsValue> {
        let role = Role::from_wire_str(role)
            .ok_or_else(|| js_err("role must be \"host\" or \"guest\""))?;
        let runtime = value_from_json(&parse_json(runtime_json)?).map_err(js_err)?;
        let expectation = match expectation_json {
            Some(text) => Some(manifest_expectation_from_json(&parse_json(&text)?)),
            None => None,
        };
        let state = coordinator::new(Options {
            role,
            session_id: session_id.to_string(),
            peer_id: peer_id.to_string(),
            host_peer_id,
            host_link_id,
            runtime,
            build_id,
            expectation,
        })
        .map_err(|err| js_err(err.message))?;
        Ok(Coordinator {
            state,
            inbox: TickQueue::new(),
        })
    }

    /// Queues an inbound control-wire arrival. Safe to call at any time —
    /// see the module doc and [`crate::net_inbox`]: this only ever appends
    /// to the internal queue, never touches coordinator state, and the
    /// queued item is not applied until the next [`Coordinator::tick`] call.
    #[wasm_bindgen(js_name = enqueueControlWire)]
    pub fn enqueue_control_wire(&mut self, link_id: &str, wire: &str) {
        self.inbox.enqueue(QueuedEvent::Control {
            link_id: link_id.to_string(),
            wire: wire.to_string(),
        });
    }

    /// Queues a transport-reported lost link. Same discipline as
    /// [`Coordinator::enqueue_control_wire`].
    #[wasm_bindgen(js_name = enqueueLinkLost)]
    pub fn enqueue_link_lost(&mut self, link_id: &str, code: Option<String>) {
        self.inbox.enqueue(QueuedEvent::LinkLost {
            link_id: link_id.to_string(),
            code,
        });
    }

    /// How many network-originated items are queued, awaiting the next
    /// [`Coordinator::tick`] call.
    #[wasm_bindgen(getter, js_name = queuedCount)]
    #[must_use]
    pub fn queued_count(&self) -> f64 {
        self.inbox.len() as f64
    }

    /// The tick boundary: drains every queued network item (in arrival
    /// order), applies each through the reducer, then applies one
    /// `Event::Tick`. Returns `{"outcomes": [...], "summary": {...}}` as
    /// JSON — one outcome per applied event, in the same order, the last
    /// one always the `Tick` event's own outcome.
    pub fn tick(&mut self) -> String {
        let queued = self.inbox.drain();
        let mut outcomes: Vec<Outcome> = queued
            .into_iter()
            .map(|item| self.apply(item.into_event()))
            .collect();
        outcomes.push(self.apply(Event::Tick));
        self.batch_json(&outcomes)
    }

    /// This peer opens its manual connection. Local intent — see the module
    /// doc for why this applies immediately rather than queuing.
    pub fn connect(&mut self) -> String {
        let outcome = self.apply(Event::Connect);
        self.batch_json(&[outcome])
    }

    /// The host proposes a session manifest, given as JSON (see the module
    /// doc's `Value` array/object rule).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `manifest_json` fails to parse.
    /// A structurally invalid *manifest* is not a parse error — the
    /// coordinator itself rejects it, reachable in the returned batch's
    /// outcome.
    #[wasm_bindgen(js_name = proposeManifest)]
    pub fn propose_manifest(&mut self, manifest_json: &str) -> Result<String, JsValue> {
        let manifest = value_from_json(&parse_json(manifest_json)?).map_err(js_err)?;
        let outcome = self.apply(Event::ProposeManifest { manifest });
        Ok(self.batch_json(&[outcome]))
    }

    /// The host publishes slot ownership, given as JSON.
    ///
    /// # Errors
    ///
    /// See [`Coordinator::propose_manifest`].
    #[wasm_bindgen(js_name = assignSlots)]
    pub fn assign_slots(
        &mut self,
        assignments_json: &str,
        preserve_claims: bool,
    ) -> Result<String, JsValue> {
        let assignments = value_from_json(&parse_json(assignments_json)?).map_err(js_err)?;
        let outcome = self.apply(Event::AssignSlots {
            assignments,
            preserve_claims,
        });
        Ok(self.batch_json(&[outcome]))
    }

    /// This peer requests an owned set, given as canonical slot wire ids
    /// (e.g. `["home_1", "home_2"]`).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if any entry in `slots` is not a
    /// canonical slot wire id.
    #[wasm_bindgen(js_name = preferPair)]
    pub fn prefer_pair(&mut self, slots: Vec<String>) -> Result<String, JsValue> {
        let mut resolved = Vec::with_capacity(slots.len());
        for wire in &slots {
            resolved.push(
                protocol::slot_id_from_wire(wire)
                    .ok_or_else(|| js_err(format!("'{wire}' is not a canonical slot id")))?,
            );
        }
        let outcome = self.apply(Event::PreferPair { slots: resolved });
        Ok(self.batch_json(&[outcome]))
    }

    /// This peer's own readiness changed.
    #[wasm_bindgen(js_name = setReady)]
    pub fn set_ready(&mut self, ready: bool) -> String {
        let outcome = self.apply(Event::SetReady { ready });
        self.batch_json(&[outcome])
    }

    /// The host starts the countdown.
    #[wasm_bindgen(js_name = beginCountdown)]
    pub fn begin_countdown(
        &mut self,
        countdown_id: &str,
        remaining_ticks: f64,
        first_input_tick: f64,
    ) -> String {
        let outcome = self.apply(Event::BeginCountdown {
            countdown_id: countdown_id.to_string(),
            remaining_ticks: remaining_ticks as i64,
            first_input_tick: first_input_tick as i64,
        });
        self.batch_json(&[outcome])
    }

    /// The host reports simulation progress.
    #[wasm_bindgen(js_name = matchPhase)]
    pub fn match_phase(
        &mut self,
        phase: &str,
        tick: f64,
        home_score: f64,
        away_score: f64,
    ) -> String {
        let outcome = self.apply(Event::MatchPhase {
            phase: phase.to_string(),
            tick: tick as i64,
            home_score: home_score as i64,
            away_score: away_score as i64,
        });
        self.batch_json(&[outcome])
    }

    /// A local boundary hash was computed.
    #[wasm_bindgen(js_name = hashReport)]
    pub fn hash_report(&mut self, tick: f64, boundary_hash: &str) -> String {
        let outcome = self.apply(Event::HashReport {
            tick: tick as i64,
            boundary_hash: boundary_hash.to_string(),
        });
        self.batch_json(&[outcome])
    }

    /// The local simulation reports its final result.
    #[wasm_bindgen(js_name = matchFinish)]
    pub fn match_finish(
        &mut self,
        final_tick: f64,
        home_score: f64,
        away_score: f64,
        final_hash: &str,
    ) -> String {
        let outcome = self.apply(Event::Finish {
            final_tick: final_tick as i64,
            home_score: home_score as i64,
            away_score: away_score as i64,
            final_hash: final_hash.to_string(),
        });
        self.batch_json(&[outcome])
    }

    /// The local netcode layer reports a failure (`failure` is raw text —
    /// see `gc_netcode::coordinator::Event::NetcodeFailure`'s own doc for
    /// why an unrecognized class is a legitimate, exercised rejection path
    /// rather than something this bridge pre-validates).
    #[wasm_bindgen(js_name = netcodeFailure)]
    pub fn netcode_failure(
        &mut self,
        failure: &str,
        peer_id: Option<String>,
        detail: Option<String>,
    ) -> String {
        let outcome = self.apply(Event::NetcodeFailure {
            failure: failure.to_string(),
            peer_id,
            detail,
        });
        self.batch_json(&[outcome])
    }

    /// A guest leaves.
    pub fn leave(&mut self) -> String {
        let outcome = self.apply(Event::Leave);
        self.batch_json(&[outcome])
    }

    /// This peer aborts the session.
    pub fn abort(&mut self, code: Option<String>, detail: Option<String>) -> String {
        let outcome = self.apply(Event::Abort { code, detail });
        self.batch_json(&[outcome])
    }

    /// A compact summary (role, phase, peer/ready counts, manifest id,
    /// frozen flag, terminal), as JSON. Cheap to call every frame; see
    /// [`Coordinator::state_json`] for the full state.
    #[wasm_bindgen(js_name = summaryJson)]
    #[must_use]
    pub fn summary_json(&self) -> String {
        summary_to_json(&coordinator::summary(&self.state)).to_json_string()
    }

    /// The complete coordinator state (every peer, the manifest, ownership,
    /// freeze, hash history, ...), as JSON.
    #[wasm_bindgen(js_name = stateJson)]
    #[must_use]
    pub fn state_json(&self) -> String {
        state_to_json(&self.state).to_json_string()
    }
}

impl Coordinator {
    fn apply(&mut self, event: Event) -> Outcome {
        let (next_state, outcome) = coordinator::step(&self.state, event);
        self.state = next_state;
        outcome
    }

    fn batch_json(&self, outcomes: &[Outcome]) -> String {
        Json::obj(vec![
            (
                "outcomes",
                Json::Array(outcomes.iter().map(outcome_to_json).collect()),
            ),
            (
                "summary",
                summary_to_json(&coordinator::summary(&self.state)),
            ),
        ])
        .to_json_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_host() -> Coordinator {
        let runtime_json =
            value_to_json(&gc_netcode::coordinator_fixture::runtime()).to_json_string();
        Coordinator::new(
            "host",
            "session.1",
            "host",
            None,
            None,
            &runtime_json,
            None,
            None,
        )
        .expect("a fixture runtime always constructs a valid host coordinator")
    }

    #[test]
    fn value_json_round_trips_records_and_arrays() {
        let value = Value::record(vec![
            ("a", Value::int(1)),
            ("b", Value::str("x")),
            ("c", Value::array(vec![Value::int(1), Value::int(2)])),
            ("d", Value::Nil),
        ]);
        let json = value_to_json(&value);
        let back = value_from_json(&json).expect("round-trips");
        // `Value::record` drops nil fields on construction (see its own
        // doc), so the round trip is compared against that same
        // normalization rather than the pre-drop literal above.
        assert_eq!(back, value);
    }

    #[test]
    fn a_fresh_host_coordinator_starts_at_clock_zero_in_the_handshake_phase() {
        let coordinator = new_host();
        let state = Json::parse(&coordinator.state_json()).unwrap();
        assert_eq!(state.field_i64("clock"), Some(0));
        assert_eq!(state.field_str("phase"), Some("handshake"));
    }

    #[test]
    fn tick_advances_the_clock_by_exactly_one_and_reports_one_outcome_per_applied_event() {
        let mut coordinator = new_host();
        coordinator.enqueue_link_lost("no_such_link", None);
        assert_eq!(coordinator.queued_count(), 1.0);

        let batch = Json::parse(&coordinator.tick()).unwrap();
        let outcomes = batch.get("outcomes").unwrap().as_array().unwrap();
        // One outcome for the queued `LinkLost`, one for the trailing `Tick`.
        assert_eq!(outcomes.len(), 2);
        assert_eq!(coordinator.queued_count(), 0.0);

        let state = Json::parse(&coordinator.state_json()).unwrap();
        assert_eq!(state.field_i64("clock"), Some(1));
    }

    /// The wave's central property, proven against the real `Coordinator`
    /// wasm-bindgen surface JS calls — not just the underlying
    /// `TickQueue`/`WasmStarTransport` primitives
    /// [`crate::net_inbox`]/[`crate::wasm_transport`] already prove it
    /// against. A network item queued *after* one `tick()` call must not be
    /// reflected in state until the *next* `tick()` call.
    #[test]
    fn an_item_enqueued_after_tick_is_invisible_until_the_next_tick() {
        let mut coordinator = new_host();

        coordinator.enqueue_link_lost("link_a", None);
        coordinator.tick();
        let clock_after_first_tick = Json::parse(&coordinator.state_json())
            .unwrap()
            .field_i64("clock");
        assert_eq!(clock_after_first_tick, Some(1));

        // Stands in for the network callback firing between ticks.
        coordinator.enqueue_link_lost("link_b", None);
        assert_eq!(coordinator.queued_count(), 1.0);
        // Nothing about calling `enqueue_link_lost` may change coordinator
        // state — it only appends to the queue.
        let clock_before_second_tick = Json::parse(&coordinator.state_json())
            .unwrap()
            .field_i64("clock");
        assert_eq!(clock_before_second_tick, Some(1));

        let batch = Json::parse(&coordinator.tick()).unwrap();
        assert_eq!(batch.get("outcomes").unwrap().as_array().unwrap().len(), 2);
        let clock_after_second_tick = Json::parse(&coordinator.state_json())
            .unwrap()
            .field_i64("clock");
        assert_eq!(clock_after_second_tick, Some(2));
    }

    // `propose_manifest`/`prefer_pair`'s error paths return `JsValue`, which
    // `wasm_bindgen::JsValue::from_str` cannot construct off the wasm32
    // target ("function not implemented on non-wasm32 targets", aborting
    // the whole native test process — a hard abort, not a normal panic
    // recoverable by `#[should_panic]`). This mirrors wave 1's own split
    // (`crates/gc-wasm/src/lib.rs`'s doc, `session.spec.ts`'s
    // `expect(() => session.step("not a wire")).toThrow()`): a
    // `Result<_, JsValue>` error path is exercised from the TypeScript
    // suite, never from a native `#[test]`. `packages/wasm/src/coordinator.spec.ts`
    // (this wave) covers `proposeManifest`/`preferPair` rejecting malformed
    // input the same way.

    #[test]
    fn set_ready_applies_immediately_without_going_through_the_queue() {
        let mut coordinator = new_host();
        let batch = Json::parse(&coordinator.set_ready(true)).unwrap();
        let outcomes = batch.get("outcomes").unwrap().as_array().unwrap();
        assert_eq!(outcomes.len(), 1);
        // Local intent never touches the network inbox.
        assert_eq!(coordinator.queued_count(), 0.0);
    }
}
