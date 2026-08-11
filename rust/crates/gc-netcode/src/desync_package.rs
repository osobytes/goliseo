//! Port of `game/online/desync_package.lua`.
//!
//! A deterministic desync capture: the smallest artifact that lets someone
//! else reproduce a divergence offline.
//!
//! The rule it is built around is that a reproduction needs *inputs and
//! identity*, not a transcript. Given the exact manifest, protocol, and
//! fixture identity, the canonical input wires over a bounded window, and
//! the boundary the peers last agreed on, an offline reproducer can
//! re-derive the divergence. None of that requires a full log, and a full
//! log would be both unbounded and the most likely place for something
//! private to end up.
//!
//! ## What "sufficient" means here, exactly
//!
//! [`ReproducibleFrom`] is a required field and it is the package's own
//! honest claim about itself:
//!
//! * [`ReproducibleFrom::FixtureBoundaryZero`] — the wires start at the
//!   session's first input tick, so a reproducer that can build boundary
//!   zero from the pinned fixture has everything. This is the strongest
//!   claim and the one the specs exercise.
//! * [`ReproducibleFrom::TapeReference`] — the wires do not reach boundary
//!   zero, but a recorded input tape does, and its id and digest are
//!   carried here.
//! * [`ReproducibleFrom::RetainedWindow`] — neither of the above. The
//!   package reproduces the window from the named boundary onward *if* the
//!   reproducer can independently produce the state at that boundary.
//!   Weakest claim, and it says so.
//!
//! A package never silently degrades between these: [`build`] derives the
//! claim from the evidence it was actually given.
//!
//! ## What is deliberately not in here
//!
//! No snapshot bytes. A canonical `MatchSnapshot` runs to hundreds of
//! kilobytes, and embedding one would turn a bounded capture into a file
//! nobody attaches to an issue. The agreed boundary is carried as a tick and
//! a hash, which is what a reproducer needs to *check* it rebuilt the right
//! state — and a hash is evidence, whereas a snapshot is a copy of the
//! match.
//!
//! No SDP, no ICE, no addresses, no participant payloads.
//!
//! ## Adapted from the Lua original: how diagnostics and identity arrive
//!
//! The Lua original's `desync_package.build` takes a `recorder` (a live
//! `NetDiagnostics` instance) and calls `net_diagnostics.export(recorder)`
//! itself, and takes a full `manifest: SessionManifest` /
//! `freeze: CoordinatorFreeze` for identity. None of those three types are
//! available here:
//!
//! - `net_diagnostics.lua` is TypeScript-owned (`v2/README.md` §2's
//!   `game/online/**` split; `game.online.net_diagnostics` is not in the
//!   Rust-owned list) — there is no `export` function to call.
//! - `SessionManifest` and `CoordinatorFreeze` are defined in
//!   `game/online/protocol.lua` and `game/online/coordinator.lua`
//!   respectively, both explicitly out of this agent's scope ("later agents
//!   own those").
//!
//! Per README §6 rule 7's TypeScript idiom (equally applicable here, since
//! the underlying problem is the same: a module that cannot import content
//! it does not own) [`build`] takes the *already-exported* diagnostics data
//! as an explicit [`Diagnostics`] parameter (`None` standing in for the Lua
//! original's `net_diagnostics.export` returning `nil` when a recorder
//! opted out) instead of a recorder, and takes the two or three identity
//! strings it actually reads (`session_id`, `manifest_id`,
//! `first_input_tick`) instead of the two whole structs. Everything [`build`]
//! itself computes — reproducibility classification, wire bounding and
//! truncation, digesting, shape validation — is unchanged.

use crate::diagnostics_schema::{self, Domain, Field, FieldKind, Shape, Value};
use crate::input_protocol::{self, AuthorityRow, DecodeContext};

/// Wire format version for the package artifact itself (independent of
/// [`diagnostics_schema::SERIALIZATION_VERSION`] and
/// [`input_protocol::VERSION`]).
pub const VERSION: i64 = 1;

/// 192 wires at the protocol's 1 KiB envelope bound is a 192 KiB ceiling.
/// That is an artifact a person can attach to an issue; a full match of
/// wires is not.
pub const MAX_WIRES: usize = 192;

/// The package's own honest claim about how much a reproducer needs beyond
/// what is in the package. See the module doc comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReproducibleFrom {
    /// The wires reach the session's first input tick.
    FixtureBoundaryZero,
    /// The wires do not reach boundary zero, but a recorded tape does.
    TapeReference,
    /// Neither: reproducible only if the reproducer independently has the
    /// state at the named boundary.
    RetainedWindow,
}

impl ReproducibleFrom {
    fn as_str(self) -> &'static str {
        match self {
            ReproducibleFrom::FixtureBoundaryZero => "fixture_boundary_zero",
            ReproducibleFrom::TapeReference => "tape_reference",
            ReproducibleFrom::RetainedWindow => "retained_window",
        }
    }
}

/// Whether a bounded collection dropped anything. A field, never a silently
/// shorter array: a truncated export that looks complete is worse than no
/// export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retention {
    /// Nothing was dropped.
    Complete,
    /// The collection was cut off at its bound.
    Truncated,
}

impl Retention {
    fn as_str(self) -> &'static str {
        match self {
            Retention::Complete => diagnostics_schema::COMPLETE,
            Retention::Truncated => diagnostics_schema::TRUNCATED,
        }
    }
}

/// A recorded input tape reference, carried when the wires alone do not
/// reach boundary zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeReference {
    /// The tape's own id.
    pub tape_id: String,
    /// `fnv1a64` digest of the tape's contents.
    pub tape_digest: String,
    /// The tape schema version.
    pub tape_version: i64,
}

/// A value naming the first point two snapshots disagreed. `expected`/`actual`
/// mirror the Lua original's `any` (a number formatted with `%.17g`, or
/// text) — see [`diagnostics_schema::format_g17`].
#[derive(Clone, Debug, PartialEq)]
pub enum DifferenceValue {
    /// A `%.17g`-formatted numeric difference.
    Number(f64),
    /// An already-textual difference.
    Text(String),
}

/// The first snapshot field two peers' state disagreed on, if locally known.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstDifference {
    /// Dotted path into the snapshot, e.g. `"state.ball.pos.x"`.
    pub path: String,
    /// The locally expected value.
    pub expected: DifferenceValue,
    /// The value actually observed.
    pub actual: DifferenceValue,
}

/// One session-identity field set, matching
/// `net_diagnostics.SESSION_SHAPE` (`game/online/net_diagnostics.lua:299`).
/// This crate does not own `net_diagnostics.lua` (TypeScript per
/// `v2/README.md` §2), so this struct is this module's own typed mirror of
/// that shape rather than a shared type — see the module doc comment.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionIdentity {
    /// Opaque session join key.
    pub session_id: String,
    /// This recorder's own peer id.
    pub peer_id: String,
    /// `"host"` or `"guest"`.
    pub role: String,
    /// `"1v1"`, `"2v2"`, or `"4v4"`.
    pub match_mode: String,
    /// Combat interaction status label.
    pub combat_status: String,
    /// Hash identifying the frozen session manifest.
    pub manifest_id: String,
    /// Hash identifying the frozen slot assignment.
    pub assignment_id: String,
    /// Id of the countdown that started this match.
    pub countdown_id: String,
    /// Build identity.
    pub build_id: String,
    /// Source identity.
    pub source_id: String,
    /// Content identity.
    pub content_id: String,
    /// Tuning identity.
    pub tuning_id: String,
    /// Match configuration identity.
    pub match_config_id: String,
    /// Fixture identity.
    pub fixture_id: String,
    /// Arena identity.
    pub arena_id: String,
    /// Combat rules identity.
    pub combat_rules_id: String,
    /// Gameplay AI policy identity.
    pub gameplay_ai_policy_id: String,
    /// Digest of the negotiated network profile, when one was negotiated.
    pub network_profile_digest: Option<String>,
    /// Session protocol wire version.
    pub protocol_version: i64,
    /// Input packet schema version.
    pub input_version: i64,
    /// Match-snapshot schema version.
    pub snapshot_version: i64,
    /// Input-tape schema version.
    pub tape_version: i64,
    /// Combat-snapshot schema version.
    pub combat_schema_version: i64,
    /// Deterministic match seed.
    pub seed: i64,
    /// Fixed-simulation tick rate.
    pub tick_rate: i64,
    /// Match duration, in ticks.
    pub duration_ticks: i64,
    /// Goal cap (99 = no limit, the shipped rule since #268).
    pub max_goals: i64,
}

/// One published rollback checkpoint: a boundary tick, its canonical hash,
/// and which producers were live at that boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointRecord {
    /// The boundary tick this checkpoint names.
    pub tick: i64,
    /// The canonical `fnv1a64` hash both peers should agree on at `tick`.
    pub hash: String,
    /// `producer_id -> role`-shaped liveness map at this checkpoint.
    pub live: Vec<(String, String)>,
}

/// One recorded control-channel message.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlRecord {
    /// Monotonic recording order.
    pub ordinal: i64,
    /// Message kind.
    pub kind: String,
    /// Sender's peer id.
    pub peer_id: String,
    /// Sender's per-peer sequence number.
    pub sequence: i64,
    /// The message's own id.
    pub message_id: String,
}

/// One recorded wall-clock runtime observation (connection state changes,
/// signalling events, ...).
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeEventRecord {
    /// Monotonic recording order.
    pub ordinal: i64,
    /// Event kind.
    pub kind: String,
    /// Wall-clock time the event was recorded, in milliseconds.
    pub monotonic_ms: f64,
    /// The peer this event concerns, if any.
    pub peer_id: Option<String>,
    /// The transport channel this event concerns, if any.
    pub channel: Option<String>,
    /// A short state label, if any.
    pub state: Option<String>,
    /// A short error/status code, if any.
    pub code: Option<String>,
    /// Free-text detail, if any (already redacted/bounded by the recorder).
    pub detail: Option<String>,
}

/// The already-exported diagnostics data this recorder opted in to sharing —
/// this module's stand-in for calling `net_diagnostics.export(recorder)`.
/// See the module doc comment.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    /// This recorder's session identity.
    pub session: SessionIdentity,
    /// Published rollback checkpoints.
    pub checkpoints: Vec<CheckpointRecord>,
    /// Recorded control-channel messages.
    pub control: Vec<ControlRecord>,
    /// Recorded runtime events.
    pub runtime_events: Vec<RuntimeEventRecord>,
}

/// Inputs to [`build`].
#[derive(Clone, Debug, PartialEq)]
pub struct BuildOptions {
    /// The recorder's already-exported diagnostics, or `None` if it never
    /// opted in to export (mirrors the Lua original's
    /// `net_diagnostics.export` returning `nil`).
    pub diagnostics: Option<Diagnostics>,
    /// The frozen session's `manifest.session_id`.
    pub session_id: String,
    /// The frozen session's `freeze.manifest_id`.
    pub manifest_id: String,
    /// The frozen session's `freeze.first_input_tick`.
    pub first_input_tick: i64,
    /// This peer's id.
    pub peer_id: String,
    /// The other peer's id.
    pub remote_peer_id: String,
    /// Last boundary tick both peers hashed identically.
    pub agreed_boundary_tick: i64,
    /// The hash both peers agreed on at `agreed_boundary_tick`.
    pub agreed_boundary_hash: String,
    /// First boundary tick the peers disagreed on.
    pub divergence_tick: i64,
    /// This peer's hash at `divergence_tick`.
    pub local_hash: String,
    /// The other peer's hash at `divergence_tick`.
    pub remote_hash: String,
    /// Canonical input packet wires, sender order preserved.
    pub input_wires: Vec<Vec<u8>>,
    /// The first locally-known differing snapshot field, if any.
    pub first_difference: Option<FirstDifference>,
    /// A recorded input tape reference, if the wires do not reach boundary zero.
    pub tape: Option<TapeReference>,
}

/// A complete desync package: everything [`build`] assembled.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    /// Exactly [`VERSION`].
    pub package_version: i64,
    /// Exactly [`diagnostics_schema::DIGEST`].
    pub digest_algorithm: String,
    /// This recorder's exported session identity.
    pub session: SessionIdentity,
    /// Who can reproduce this package and how.
    pub reproduction: Reproduction,
    /// Where and how the peers' state diverged.
    pub divergence: Divergence,
    /// The canonical input evidence.
    pub inputs: Inputs,
    /// Published rollback checkpoints.
    pub checkpoints: Vec<CheckpointRecord>,
    /// Recorded control-channel messages.
    pub control: Vec<ControlRecord>,
    /// Recorded runtime events.
    pub runtime_events: Vec<RuntimeEventRecord>,
}

/// See [`Package::reproduction`].
#[derive(Clone, Debug, PartialEq)]
pub struct Reproduction {
    /// The package's own honest reproducibility claim.
    pub reproducible_from: ReproducibleFrom,
    /// This peer's id.
    pub local_peer_id: String,
    /// The other peer's id.
    pub remote_peer_id: String,
    /// A recorded input tape reference, when carried.
    pub tape: Option<TapeReference>,
}

/// See [`Package::divergence`].
#[derive(Clone, Debug, PartialEq)]
pub struct Divergence {
    /// Last boundary tick both peers hashed identically.
    pub agreed_boundary_tick: i64,
    /// The hash both peers agreed on at `agreed_boundary_tick`.
    pub agreed_boundary_hash: String,
    /// First boundary tick the peers disagreed on.
    pub divergence_tick: i64,
    /// This peer's hash at `divergence_tick`.
    pub local_hash: String,
    /// The other peer's hash at `divergence_tick`.
    pub remote_hash: String,
    /// The first locally-known differing snapshot field, if any.
    pub first_difference: Option<FirstDifference>,
}

/// See [`Package::inputs`].
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    /// Fixed tick delay between a local input sample and its earliest legal send.
    pub input_delay_ticks: i64,
    /// Earliest input tick any carried wire names.
    pub from_input_tick: i64,
    /// Latest input tick any carried wire names.
    pub through_input_tick: i64,
    /// Number of wires carried.
    pub wire_count: i64,
    /// `fnv1a64` digest over the carried wires (order-sensitive).
    pub wire_digest: String,
    /// Whether the wire window was cut off at [`MAX_WIRES`].
    pub retention: Retention,
    /// Canonical input packet wires, sender order preserved.
    pub wires: Vec<Vec<u8>>,
}

fn text(value: impl Into<String>) -> Value {
    Value::str(value)
}

fn optional_text(value: &Option<String>) -> Option<Value> {
    value.as_ref().map(|v| Value::str(v.clone()))
}

impl SessionIdentity {
    fn to_value(&self) -> Value {
        let mut fields = vec![
            ("session_id".to_string(), text(&self.session_id)),
            ("peer_id".to_string(), text(&self.peer_id)),
            ("role".to_string(), text(&self.role)),
            ("match_mode".to_string(), text(&self.match_mode)),
            ("combat_status".to_string(), text(&self.combat_status)),
            ("manifest_id".to_string(), text(&self.manifest_id)),
            ("assignment_id".to_string(), text(&self.assignment_id)),
            ("countdown_id".to_string(), text(&self.countdown_id)),
            ("build_id".to_string(), text(&self.build_id)),
            ("source_id".to_string(), text(&self.source_id)),
            ("content_id".to_string(), text(&self.content_id)),
            ("tuning_id".to_string(), text(&self.tuning_id)),
            ("match_config_id".to_string(), text(&self.match_config_id)),
            ("fixture_id".to_string(), text(&self.fixture_id)),
            ("arena_id".to_string(), text(&self.arena_id)),
            ("combat_rules_id".to_string(), text(&self.combat_rules_id)),
            (
                "gameplay_ai_policy_id".to_string(),
                text(&self.gameplay_ai_policy_id),
            ),
        ];
        if let Some(value) = optional_text(&self.network_profile_digest) {
            fields.push(("network_profile_digest".to_string(), value));
        }
        fields.extend([
            (
                "protocol_version".to_string(),
                Value::Number(self.protocol_version as f64),
            ),
            (
                "input_version".to_string(),
                Value::Number(self.input_version as f64),
            ),
            (
                "snapshot_version".to_string(),
                Value::Number(self.snapshot_version as f64),
            ),
            (
                "tape_version".to_string(),
                Value::Number(self.tape_version as f64),
            ),
            (
                "combat_schema_version".to_string(),
                Value::Number(self.combat_schema_version as f64),
            ),
            ("seed".to_string(), Value::Number(self.seed as f64)),
            (
                "tick_rate".to_string(),
                Value::Number(self.tick_rate as f64),
            ),
            (
                "duration_ticks".to_string(),
                Value::Number(self.duration_ticks as f64),
            ),
            (
                "max_goals".to_string(),
                Value::Number(self.max_goals as f64),
            ),
        ]);
        Value::Record(fields)
    }
}

impl CheckpointRecord {
    fn to_value(&self) -> Value {
        Value::Record(vec![
            ("tick".to_string(), Value::Number(self.tick as f64)),
            ("hash".to_string(), text(&self.hash)),
            (
                "live".to_string(),
                Value::Map(
                    self.live
                        .iter()
                        .map(|(k, v)| (k.clone(), text(v)))
                        .collect(),
                ),
            ),
        ])
    }
}

impl ControlRecord {
    fn to_value(&self) -> Value {
        Value::Record(vec![
            ("ordinal".to_string(), Value::Number(self.ordinal as f64)),
            ("kind".to_string(), text(&self.kind)),
            ("peer_id".to_string(), text(&self.peer_id)),
            ("sequence".to_string(), Value::Number(self.sequence as f64)),
            ("message_id".to_string(), text(&self.message_id)),
        ])
    }
}

impl RuntimeEventRecord {
    fn to_value(&self) -> Value {
        let mut fields = vec![
            ("ordinal".to_string(), Value::Number(self.ordinal as f64)),
            ("kind".to_string(), text(&self.kind)),
            ("monotonic_ms".to_string(), Value::Number(self.monotonic_ms)),
        ];
        if let Some(value) = optional_text(&self.peer_id) {
            fields.push(("peer_id".to_string(), value));
        }
        if let Some(value) = optional_text(&self.channel) {
            fields.push(("channel".to_string(), value));
        }
        if let Some(value) = optional_text(&self.state) {
            fields.push(("state".to_string(), value));
        }
        if let Some(value) = optional_text(&self.code) {
            fields.push(("code".to_string(), value));
        }
        if let Some(value) = optional_text(&self.detail) {
            fields.push(("detail".to_string(), value));
        }
        Value::Record(fields)
    }
}

/// The shared `net_diagnostics.SESSION_SHAPE` fields
/// (`game/online/net_diagnostics.lua:299-327`), reproduced here as data
/// (not logic) because `net_diagnostics.lua` is TypeScript-owned — see the
/// module doc comment.
fn session_shape_fields() -> Vec<Field> {
    let integer = |name: &str| Field::new(FieldKind::Integer).named(name);
    vec![
        Field::new(FieldKind::Id).named("session_id"),
        Field::new(FieldKind::Id).named("peer_id"),
        Field::new(FieldKind::Enum)
            .named("role")
            .values(diagnostics_schema::enum_values(&["host", "guest"])),
        Field::new(FieldKind::Enum)
            .named("match_mode")
            .values(diagnostics_schema::enum_values(&["1v1", "2v2", "4v4"])),
        Field::new(FieldKind::Enum)
            .named("combat_status")
            .values(diagnostics_schema::enum_values(&[
                "provisional_114",
                "accepted_proceed",
                "accepted_revision",
            ])),
        Field::new(FieldKind::Hash).named("manifest_id"),
        Field::new(FieldKind::Hash).named("assignment_id"),
        Field::new(FieldKind::Id).named("countdown_id"),
        Field::new(FieldKind::Id).named("build_id"),
        Field::new(FieldKind::Id).named("source_id"),
        Field::new(FieldKind::Id).named("content_id"),
        Field::new(FieldKind::Id).named("tuning_id"),
        Field::new(FieldKind::Id).named("match_config_id"),
        Field::new(FieldKind::Id).named("fixture_id"),
        Field::new(FieldKind::Id).named("arena_id"),
        Field::new(FieldKind::Id).named("combat_rules_id"),
        Field::new(FieldKind::Id).named("gameplay_ai_policy_id"),
        Field::new(FieldKind::Hash)
            .named("network_profile_digest")
            .optional(),
        integer("protocol_version"),
        integer("input_version"),
        integer("snapshot_version"),
        integer("tape_version"),
        integer("combat_schema_version"),
        integer("seed"),
        integer("tick_rate"),
        integer("duration_ticks"),
        integer("max_goals"),
    ]
}

/// The desync package shape (`game/online/desync_package.lua:80-193`).
#[must_use]
pub fn shape() -> Shape {
    diagnostics_schema::record(
        "desync_package",
        None,
        vec![
            Field::new(FieldKind::Integer)
                .named("package_version")
                .domain(Domain::Identity)
                .min(1.0),
            Field::new(FieldKind::Str)
                .named("digest_algorithm")
                .domain(Domain::Identity),
            Field::new(FieldKind::Record)
                .named("session")
                .domain(Domain::Identity)
                .fields(session_shape_fields()),
            Field::new(FieldKind::Record)
                .named("reproduction")
                .domain(Domain::Identity)
                .fields(vec![
                    Field::new(FieldKind::Enum)
                        .named("reproducible_from")
                        .values(diagnostics_schema::enum_values(&[
                            "fixture_boundary_zero",
                            "tape_reference",
                            "retained_window",
                        ])),
                    Field::new(FieldKind::Id).named("local_peer_id"),
                    Field::new(FieldKind::Id).named("remote_peer_id"),
                    Field::new(FieldKind::Record)
                        .named("tape")
                        .optional()
                        .fields(vec![
                            Field::new(FieldKind::Id).named("tape_id"),
                            Field::new(FieldKind::Hash).named("tape_digest"),
                            Field::new(FieldKind::Integer)
                                .named("tape_version")
                                .min(0.0),
                        ]),
                ]),
            Field::new(FieldKind::Record)
                .named("divergence")
                .domain(Domain::Canonical)
                .fields(vec![
                    Field::new(FieldKind::Integer).named("agreed_boundary_tick"),
                    Field::new(FieldKind::Hash).named("agreed_boundary_hash"),
                    Field::new(FieldKind::Integer).named("divergence_tick"),
                    Field::new(FieldKind::Hash).named("local_hash"),
                    Field::new(FieldKind::Hash).named("remote_hash"),
                    Field::new(FieldKind::Str)
                        .named("first_difference_path")
                        .optional(),
                    Field::new(FieldKind::Text)
                        .named("first_difference_expected")
                        .optional(),
                    Field::new(FieldKind::Text)
                        .named("first_difference_actual")
                        .optional(),
                ]),
            Field::new(FieldKind::Record)
                .named("inputs")
                .domain(Domain::Canonical)
                .fields(vec![
                    Field::new(FieldKind::Integer)
                        .named("input_delay_ticks")
                        .min(0.0),
                    Field::new(FieldKind::Integer).named("from_input_tick"),
                    Field::new(FieldKind::Integer).named("through_input_tick"),
                    Field::new(FieldKind::Integer).named("wire_count").min(0.0),
                    Field::new(FieldKind::Hash).named("wire_digest"),
                    Field::new(FieldKind::Enum).named("retention").values(
                        diagnostics_schema::enum_values(&[
                            diagnostics_schema::COMPLETE,
                            diagnostics_schema::TRUNCATED,
                        ]),
                    ),
                    Field::new(FieldKind::Array)
                        .named("wires")
                        .max_length(MAX_WIRES)
                        .element(Field::new(FieldKind::Str).max_length(1024)),
                ]),
            Field::new(FieldKind::Array)
                .named("checkpoints")
                .domain(Domain::Canonical)
                .element(Field::new(FieldKind::Record).fields(vec![
                        Field::new(FieldKind::Integer).named("tick"),
                        Field::new(FieldKind::Hash).named("hash"),
                        Field::new(FieldKind::Map)
                            .named("live")
                            .element(Field::new(FieldKind::Id)),
                    ])),
            Field::new(FieldKind::Array)
                .named("control")
                .domain(Domain::Canonical)
                .element(Field::new(FieldKind::Record).fields(vec![
                        Field::new(FieldKind::Integer).named("ordinal").min(0.0),
                        Field::new(FieldKind::Id).named("kind"),
                        Field::new(FieldKind::Id).named("peer_id"),
                        Field::new(FieldKind::Integer).named("sequence").min(0.0),
                        Field::new(FieldKind::Str).named("message_id").max_length(320),
                    ])),
            Field::new(FieldKind::Array)
                .named("runtime_events")
                .domain(Domain::Runtime)
                .element(Field::new(FieldKind::Record).fields(vec![
                    Field::new(FieldKind::Integer).named("ordinal").min(0.0),
                    Field::new(FieldKind::Id).named("kind"),
                    Field::new(FieldKind::Number).named("monotonic_ms"),
                    Field::new(FieldKind::Id).named("peer_id").optional(),
                    Field::new(FieldKind::Id).named("channel").optional(),
                    Field::new(FieldKind::Str).named("state").optional(),
                    Field::new(FieldKind::Str).named("code").optional(),
                    Field::new(FieldKind::Text).named("detail").optional(),
                ])),
        ],
    )
}

fn difference_bytes(value: &DifferenceValue) -> Vec<u8> {
    let rendered = match value {
        DifferenceValue::Number(number) => diagnostics_schema::format_g17(*number).into_bytes(),
        DifferenceValue::Text(text) => text.clone().into_bytes(),
    };
    if rendered.len() <= diagnostics_schema::MAX_TEXT_LENGTH {
        return rendered;
    }
    let keep = diagnostics_schema::MAX_TEXT_LENGTH - diagnostics_schema::TRUNCATED.len() - 1;
    let mut truncated = rendered[..keep].to_vec();
    truncated.push(b' ');
    truncated.extend_from_slice(diagnostics_schema::TRUNCATED.as_bytes());
    truncated
}

impl Package {
    /// Convert to a [`Value`] for [`diagnostics_schema::validate`]/`encode`/`digest`.
    fn to_value(&self) -> Value {
        let mut reproduction_fields = vec![
            (
                "reproducible_from".to_string(),
                text(self.reproduction.reproducible_from.as_str()),
            ),
            (
                "local_peer_id".to_string(),
                text(&self.reproduction.local_peer_id),
            ),
            (
                "remote_peer_id".to_string(),
                text(&self.reproduction.remote_peer_id),
            ),
        ];
        if let Some(tape) = &self.reproduction.tape {
            reproduction_fields.push((
                "tape".to_string(),
                Value::Record(vec![
                    ("tape_id".to_string(), text(&tape.tape_id)),
                    ("tape_digest".to_string(), text(&tape.tape_digest)),
                    (
                        "tape_version".to_string(),
                        Value::Number(tape.tape_version as f64),
                    ),
                ]),
            ));
        }

        let mut divergence_fields = vec![
            (
                "agreed_boundary_tick".to_string(),
                Value::Number(self.divergence.agreed_boundary_tick as f64),
            ),
            (
                "agreed_boundary_hash".to_string(),
                text(&self.divergence.agreed_boundary_hash),
            ),
            (
                "divergence_tick".to_string(),
                Value::Number(self.divergence.divergence_tick as f64),
            ),
            ("local_hash".to_string(), text(&self.divergence.local_hash)),
            (
                "remote_hash".to_string(),
                text(&self.divergence.remote_hash),
            ),
        ];
        if let Some(difference) = &self.divergence.first_difference {
            divergence_fields.push(("first_difference_path".to_string(), text(&difference.path)));
            divergence_fields.push((
                "first_difference_expected".to_string(),
                Value::text(difference_bytes(&difference.expected)),
            ));
            divergence_fields.push((
                "first_difference_actual".to_string(),
                Value::text(difference_bytes(&difference.actual)),
            ));
        }

        Value::Record(vec![
            (
                "package_version".to_string(),
                Value::Number(self.package_version as f64),
            ),
            ("digest_algorithm".to_string(), text(&self.digest_algorithm)),
            ("session".to_string(), self.session.to_value()),
            (
                "reproduction".to_string(),
                Value::Record(reproduction_fields),
            ),
            ("divergence".to_string(), Value::Record(divergence_fields)),
            (
                "inputs".to_string(),
                Value::Record(vec![
                    (
                        "input_delay_ticks".to_string(),
                        Value::Number(self.inputs.input_delay_ticks as f64),
                    ),
                    (
                        "from_input_tick".to_string(),
                        Value::Number(self.inputs.from_input_tick as f64),
                    ),
                    (
                        "through_input_tick".to_string(),
                        Value::Number(self.inputs.through_input_tick as f64),
                    ),
                    (
                        "wire_count".to_string(),
                        Value::Number(self.inputs.wire_count as f64),
                    ),
                    ("wire_digest".to_string(), text(&self.inputs.wire_digest)),
                    (
                        "retention".to_string(),
                        text(self.inputs.retention.as_str()),
                    ),
                    (
                        "wires".to_string(),
                        Value::Array(
                            self.inputs
                                .wires
                                .iter()
                                .map(|w| Value::text(w.clone()))
                                .collect(),
                        ),
                    ),
                ]),
            ),
            (
                "checkpoints".to_string(),
                Value::Array(
                    self.checkpoints
                        .iter()
                        .map(CheckpointRecord::to_value)
                        .collect(),
                ),
            ),
            (
                "control".to_string(),
                Value::Array(self.control.iter().map(ControlRecord::to_value).collect()),
            ),
            (
                "runtime_events".to_string(),
                Value::Array(
                    self.runtime_events
                        .iter()
                        .map(RuntimeEventRecord::to_value)
                        .collect(),
                ),
            ),
        ])
    }
}

/// Assemble a package.
///
/// Returns `Err` on anything malformed: a capture is produced at the worst
/// moment of a session and must never be the thing that panics.
pub fn build(options: BuildOptions) -> std::result::Result<Package, String> {
    if options.divergence_tick <= options.agreed_boundary_tick {
        return Err(
            "the divergence must be later than the boundary the peers agreed on".to_string(),
        );
    }

    let mut wires = Vec::new();
    let mut truncated = false;
    for wire in &options.input_wires {
        if wires.len() >= MAX_WIRES {
            truncated = true;
            break;
        }
        wires.push(wire.clone());
    }

    // Coverage is read from the wires themselves rather than assumed,
    // because `reproducible_from` is a claim and an unchecked claim is worse
    // than none.
    let mut from_tick = i64::MAX;
    let mut through_tick = i64::MIN;
    for wire in &wires {
        let packet = input_protocol::decode(
            wire,
            &DecodeContext {
                session_id: options.session_id.clone(),
                manifest_id: options.manifest_id.clone(),
                sender_id: options.peer_id.clone(),
            },
        )
        .map_err(|_| "an input wire did not decode against this manifest".to_string())?;
        for row in &packet.rows {
            from_tick = from_tick.min(row.tick);
            through_tick = through_tick.max(row.tick);
        }
    }
    if wires.is_empty() {
        from_tick = options.agreed_boundary_tick;
        through_tick = options.agreed_boundary_tick;
    }

    let reproducible_from = if !truncated && from_tick <= options.first_input_tick {
        ReproducibleFrom::FixtureBoundaryZero
    } else if options.tape.is_some() {
        ReproducibleFrom::TapeReference
    } else {
        ReproducibleFrom::RetainedWindow
    };

    let Some(diagnostics) = options.diagnostics else {
        return Err("a desync package needs an opted-in diagnostic export".to_string());
    };

    let wire_refs: Vec<&[u8]> = wires.iter().map(Vec::as_slice).collect();
    let wire_digest = diagnostics_schema::tuple_digest("desync_input_wires", &wire_refs);

    let package = Package {
        package_version: VERSION,
        digest_algorithm: diagnostics_schema::DIGEST.to_string(),
        session: diagnostics.session,
        reproduction: Reproduction {
            reproducible_from,
            local_peer_id: options.peer_id,
            remote_peer_id: options.remote_peer_id,
            tape: options.tape,
        },
        divergence: Divergence {
            agreed_boundary_tick: options.agreed_boundary_tick,
            agreed_boundary_hash: options.agreed_boundary_hash,
            divergence_tick: options.divergence_tick,
            local_hash: options.local_hash,
            remote_hash: options.remote_hash,
            first_difference: options.first_difference,
        },
        inputs: Inputs {
            input_delay_ticks: input_protocol::FAIRNESS_DELAY_TICKS,
            from_input_tick: from_tick,
            through_input_tick: through_tick,
            wire_count: wires.len() as i64,
            wire_digest,
            retention: if truncated {
                Retention::Truncated
            } else {
                Retention::Complete
            },
            wires,
        },
        checkpoints: diagnostics.checkpoints,
        control: diagnostics.control,
        runtime_events: diagnostics.runtime_events,
    };
    diagnostics_schema::validate(&shape(), &package.to_value())?;
    Ok(package)
}

/// Canonical bytes for `package`.
pub fn encode(package: &Package) -> std::result::Result<Vec<u8>, String> {
    diagnostics_schema::encode(&shape(), &package.to_value())
}

/// `fnv1a64` digest of `package`.
pub fn digest(package: &Package) -> std::result::Result<String, String> {
    diagnostics_schema::digest(&shape(), &package.to_value())
}

/// Decode the package's wires back into canonical `(tick, slot)` order. This
/// is the entry point an offline reproducer uses: feed these rows to a
/// fresh rollback session built from the identity in `session`, step to
/// `divergence.divergence_tick`, and compare.
///
/// `session_id`/`sender_id` replace the Lua original's
/// `manifest: SessionManifest` parameter — see the module doc comment.
pub fn rows(
    package: &Package,
    session_id: &str,
    sender_id: &str,
) -> std::result::Result<Vec<AuthorityRow>, String> {
    let mut rows: Vec<AuthorityRow> = Vec::new();
    let mut seen: Vec<(i64, i64)> = Vec::new();
    for wire in &package.inputs.wires {
        let packet = input_protocol::decode(
            wire,
            &DecodeContext {
                session_id: session_id.to_string(),
                manifest_id: package.session.manifest_id.clone(),
                sender_id: sender_id.to_string(),
            },
        )
        .map_err(|err| err.message)?;
        for row in packet.rows {
            // The redundancy window re-sends the same row on consecutive
            // ticks, so a naive concatenation would replay each row up to
            // seven times. Authorship is a partition, so `(tick, slot)` is
            // unique authority and de-duplicating on it is lossless.
            let key = (row.tick, row.slot_index);
            if !seen.contains(&key) {
                seen.push(key);
                rows.push(row);
            }
        }
    }
    rows.sort_by(|left, right| (left.tick, left.slot_index).cmp(&(right.tick, right.slot_index)));
    Ok(rows)
}

/// A short read-out for an issue comment. Everything here is already in the
/// package and none of it is sensitive.
#[must_use]
pub fn summary(package: &Package) -> Vec<String> {
    let divergence = &package.divergence;
    let inputs = &package.inputs;
    vec![
        format!(
            "desync  manifest {}  fixture {}  {}",
            package.session.manifest_id, package.session.fixture_id, package.session.match_mode
        ),
        format!(
            "agreed boundary {}  hash {}",
            divergence.agreed_boundary_tick, divergence.agreed_boundary_hash
        ),
        format!(
            "diverged at {}  local {}  remote {}",
            divergence.divergence_tick, divergence.local_hash, divergence.remote_hash
        ),
        format!(
            "first difference  {}",
            divergence
                .first_difference
                .as_ref()
                .map(|d| d.path.as_str())
                .unwrap_or("not resolved locally")
        ),
        format!(
            "inputs  {} wires  ticks {}..{}  digest {}  {}",
            inputs.wire_count,
            inputs.from_input_tick,
            inputs.through_input_tick,
            inputs.wire_digest,
            inputs.retention.as_str()
        ),
        format!(
            "reproducible from  {}",
            package.reproduction.reproducible_from.as_str()
        ),
    ]
}
