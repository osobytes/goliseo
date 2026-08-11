//! The session control wire format.
//!
//! **`protocol` is the session control wire format.** Every message peers
//! exchange outside the input stream — handshake, manifest negotiation, slot
//! ownership, countdown, match phase, hash reports, results, aborts — is
//! encoded and decoded here. Two peers that encode this differently cannot
//! agree on anything, so [`encode`]/[`decode`] are differential-tested
//! against the pinned reference vectors; see
//! `tools/lua_reference/README.md` and `crates/gc-netcode/tests/protocol.rs`.
//!
//! ## Design: a generic canonical [`Value`], not one struct per message
//!
//! The wire codec (`encode`/`decode`) is fully generic over an untyped
//! value — it recurses over dynamically shaped data, sorts keys, and tags
//! each scalar (`z`/`b`/`i`/`s`/`t`). The *validation* layer
//! (`protocol::validate`, `protocol::validate_manifest`, ...) is what gives
//! that generic data its shape, entirely at runtime, which is what lets the
//! test suite construct sparse arrays, inject unknown fields, and mutate
//! closed-set string fields to values outside their domain, all to prove
//! `validate` — not a static type — is what rejects them.
//!
//! A design that turned every wire shape into a `struct` with enum fields
//! would make most of those tests *inexpressible*: you cannot set a `Role`
//! enum to `"captain"`, or add an unknown key to a closed `struct`. So
//! [`Value`] is the generic, recursive, dynamically-shaped wire value, and
//! every validator in this file operates on `&Value`.
//! [`ControlMessage`] is a thin typed wrapper (`version`/`kind`/`session_id`/
//! `peer_id`/`sequence`/`message_id`/`body: Value`) for ergonomic
//! construction and for the closed, wire-relevant `kind`; its `body` stays a
//! [`Value`] because body shape varies per kind and the malformed-body tests
//! need to mutate it dynamically.
//!
//! This is a deliberate exception to AGENTS.md ("a record
//! becomes a struct"), justified by the same rule's spirit: the natural shape
//! of an already-generic codec is a generic codec, not sixteen bespoke
//! structs bolted on top of it.
//!
//! ## ARCHITECTURE.md §3 rule 3 (wire values keep their canonical form)
//!
//! - OMP-1 canonical slot indices (1..=8, `input_frame::SLOT_COUNT`) are
//!   never converted: the wire format is 1-based, and this module's
//!   canonical-order loops walk `1..=input_frame::SLOT_COUNT` for the same
//!   reason — the index is compared against `input_frame::slot(index)`, a
//!   wire-adjacent identity, not a Rust-local offset.
//! - Canonical outfield slot ids (`"home_1"` .. `"away_4"`) are wire strings.
//!   `gc_sim::input_frame::SlotId` has no string form of its own, so
//!   [`slot_wire_id`]/[`slot_id_from_wire`] hold the exact wire strings —
//!   never renumbered, never renamed.
//! - `Team` wire strings (`"home"`/`"away"`) are likewise preserved exactly.
//! - Everything else in this module (decoder cursor position, Rust `Vec`
//!   indices used only to walk a slice) is a 0-based *implementation* detail
//!   that never appears on the wire, so it stays 0-based per the general rule.
//!
//! ## Numbers: `i64`, not `f64`
//!
//! Every protocol number is asserted an integer at encode time
//! (`is_integer`) and required to match a canonical decimal-digit regex at
//! decode time; the wire format has no room for a non-integer. Representing
//! [`Value::Int`] as `i64` rather than `f64` is therefore not a deviation
//! from ARCHITECTURE.md §3 rule 1 ("numeric fields default to `f64`") but the case the
//! same rule carves out: this is unambiguously a bounded integer domain
//! (every bound in this module — `MAX_SEQUENCE`, `MAX_SEED`, `MAX_TICK`, ...
//! — is comfortably inside `i64`), and `f64` would only invite silent
//! precision loss that the canonical-integer regex goes out of its way to
//! forbid.
//!
//! ## What is deliberately not here
//!
//! `match_manifest` and `match_session`
//! (`crates/gc-netcode/src/match_manifest.rs`,
//! `crates/gc-netcode/src/match_session.rs`) are separate modules in this
//! same crate; see their module docs for what they build on top of this one.

use gc_core::fnv1a64;
use gc_sim::{fixed_clock, input_frame, input_tape, match_snapshot};

// ---------------------------------------------------------------------------
// Generic canonical value
// ---------------------------------------------------------------------------

/// A generic, recursive, dynamically-shaped protocol value: what
/// `encode`/`decode` operate on. See the module doc comment for why this
/// module uses `Value` instead of one struct per message/manifest shape.
///
/// `Table` is a `Vec` of key/value pairs rather than a map: never
/// `HashMap`/`HashSet` (ARCHITECTURE.md §3 rule 4), and the canonical wire form always
/// sorts keys before emitting them, so insertion order carries no meaning.
/// [`Value::set`] overwrites an existing key exactly like a normal table
/// assignment, so a `Table` never holds a duplicate key.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `nil` / an absent field.
    Nil,
    /// A boolean.
    Bool(bool),
    /// An integer-valued number. See the module doc comment for why this
    /// is `i64` rather than `f64`.
    Int(i64),
    /// A string. Protocol strings are opaque bounded ASCII by
    /// construction (ids, hex hashes, closed-set keywords), so `String`
    /// (valid UTF-8) loses nothing a *canonical* wire can carry; malformed
    /// input containing non-UTF-8 bytes is rejected during decode instead of
    /// represented, which still lands on the same `malformed` outcome that
    /// bytes failing the ASCII id/hash regexes reach.
    Str(String),
    /// A table, used as either a string-keyed record or a 1-based
    /// contiguous array — the distinction is enforced by validators
    /// ([`has_only_fields`]/[`is_array_value`]), not by this type.
    Table(Vec<(Value, Value)>),
}

impl Value {
    /// A string value.
    #[must_use]
    pub fn str(value: impl Into<String>) -> Value {
        Value::Str(value.into())
    }

    /// An integer value.
    #[must_use]
    pub fn int(value: i64) -> Value {
        Value::Int(value)
    }

    /// A boolean value.
    #[must_use]
    pub fn bool(value: bool) -> Value {
        Value::Bool(value)
    }

    /// A string-keyed record, built from `(field name, value)` pairs in the
    /// order given (canonical wire order sorts them regardless).
    ///
    /// A pair whose value is [`Value::Nil`] is dropped rather than stored:
    /// `record([("build_id", Value::Nil)])` never creates a `build_id` key in
    /// the first place, because the wire format has no way to represent "key
    /// present, value nil" — assigning nil deletes. A literal `Value::Nil`
    /// entry could therefore never come from real wire data, and keeping one
    /// would make an optional field's absence encode differently (and
    /// wrongly) from the canonical wire form.
    #[must_use]
    pub fn record(fields: Vec<(&str, Value)>) -> Value {
        Value::Table(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_nil())
                .map(|(key, value)| (Value::Str(key.to_string()), value))
                .collect(),
        )
    }

    /// A 1-based contiguous array.
    #[must_use]
    pub fn array(items: Vec<Value>) -> Value {
        Value::Table(
            items
                .into_iter()
                .enumerate()
                .map(|(index, value)| (Value::Int(index as i64 + 1), value))
                .collect(),
        )
    }

    /// Borrows this value as a string, if it is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(value) => Some(value),
            _ => None,
        }
    }

    /// Borrows this value as an integer, if it is one.
    #[must_use]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// Borrows this value as a boolean, if it is one.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Borrows this value as a table, if it is one.
    #[must_use]
    pub fn as_table(&self) -> Option<&[(Value, Value)]> {
        match self {
            Value::Table(entries) => Some(entries),
            _ => None,
        }
    }

    /// True for [`Value::Nil`].
    #[must_use]
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Looks up a string-keyed field on a record table.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_table()?
            .iter()
            .find(|(field, _)| field.as_str() == Some(key))
            .map(|(_, value)| value)
    }

    /// Looks up a 1-based array element.
    #[must_use]
    pub fn get_index(&self, index: i64) -> Option<&Value> {
        self.as_table()?
            .iter()
            .find(|(key, _)| key.as_int() == Some(index))
            .map(|(_, value)| value)
    }

    /// Sets a string-keyed field, overwriting any existing entry for `key`.
    /// Setting [`Value::Nil`] *removes* the field instead of storing it
    /// present-with-nil — see [`Value::record`]'s doc comment for why a
    /// present-nil entry must never exist.
    pub fn set(&mut self, key: &str, value: Value) {
        if let Value::Table(entries) = self {
            let key_value = Value::Str(key.to_string());
            if value.is_nil() {
                entries.retain(|(field, _)| *field != key_value);
                return;
            }
            if let Some(slot) = entries.iter_mut().find(|(field, _)| *field == key_value) {
                slot.1 = value;
            } else {
                entries.push((key_value, value));
            }
        }
    }

    /// Removes a string-keyed field, if present.
    pub fn remove(&mut self, key: &str) {
        if let Value::Table(entries) = self {
            entries.retain(|(field, _)| field.as_str() != Some(key));
        }
    }

    /// The number of entries in a table value, or 0 otherwise.
    #[must_use]
    pub fn len(&self) -> usize {
        self.as_table().map_or(0, <[_]>::len)
    }

    /// True when this is an empty table (or not a table at all).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A string-keyed field is present in `allowed` and nowhere else, and no key
/// is anything but a string — integer keys are specifically excluded from a
/// record's field set.
fn has_only_fields(value: &Value, allowed: &[&str]) -> bool {
    match value.as_table() {
        Some(entries) => entries
            .iter()
            .all(|(key, _)| matches!(key, Value::Str(name) if allowed.contains(&name.as_str()))),
        None => false,
    }
}

/// A 1-based contiguous array with no gaps, optionally an exact `count` or a
/// `maximum` length.
fn is_array_value(value: &Value, count: Option<i64>, maximum: Option<i64>) -> bool {
    let entries = match value.as_table() {
        Some(entries) => entries,
        None => return false,
    };
    let mut length: i64 = 0;
    let mut maximum_index: i64 = 0;
    for (key, _) in entries {
        let index = match key.as_int() {
            Some(index) if index >= 1 => index,
            _ => return false,
        };
        length += 1;
        maximum_index = maximum_index.max(index);
    }
    if maximum_index != length {
        return false;
    }
    if let Some(count) = count
        && length != count
    {
        return false;
    }
    if let Some(maximum) = maximum
        && length > maximum
    {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// `SessionProtocolErrorCode`: a machine-readable reason a protocol
/// operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// The data violates a structural, range, or canonical-form invariant.
    Malformed,
    /// The encoded wire would exceed (or does exceed) [`MAX_WIRE_BYTES`].
    WireTooLarge,
    /// The data names a version this build does not support.
    UnsupportedVersion,
    /// A control message names a kind this build does not recognize.
    UnknownMessage,
    /// A manifest or runtime differs from an expected one deterministically.
    IdentityMismatch,
    /// A runtime identity differs from an expected one compatibility-wise.
    RuntimeMismatch,
    /// A message kind is not legal in the current lifecycle phase.
    InvalidPhase,
    /// A message id was reused for an unrelated send.
    Duplicate,
    /// A message id was reused with different canonical bytes.
    TranscriptConflict,
    /// A match mode names a size this build does not support.
    UnsupportedMatchMode,
    /// A slot ownership set disagrees with the frozen match mode's shape.
    InvalidOwnership,
}

/// An expected, recoverable protocol failure (AGENTS.md §7): the caller is
/// meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Machine-readable failure reason.
    pub code: ErrorCode,
    /// Human-readable detail. Not pinned by any conformance test; free to
    /// reword.
    pub message: String,
    /// The dotted field path a deterministic-identity mismatch was found at.
    /// `Some` only for [`compare_manifest`]/[`compare_runtime`] failures with
    /// [`ErrorCode::IdentityMismatch`]/[`ErrorCode::RuntimeMismatch`].
    pub path: Option<String>,
}

impl Error {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
            path: None,
        }
    }

    fn with_path(code: ErrorCode, message: impl Into<String>, path: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
            path: Some(path.into()),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

/// Result alias for fallible protocol operations.
pub type Result<T> = std::result::Result<T, Error>;

fn failure<T>(code: ErrorCode, message: impl Into<String>) -> Result<T> {
    Err(Error::new(code, message))
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Control protocol version.
pub const VERSION: i64 = 1;
/// Manifest schema version.
pub const MANIFEST_VERSION: i64 = 1;
/// Runtime identity schema version.
pub const RUNTIME_IDENTITY_VERSION: i64 = 1;
/// The largest canonical wire a control message may occupy.
pub const MAX_WIRE_BYTES: usize = 8192;
/// The bound every generic opaque id (build/source/content/... ids) obeys.
pub const MAX_ID_BYTES: usize = 128;
/// The bound a session id obeys.
pub const MAX_SESSION_ID_BYTES: usize = 128;
/// The bound a peer id obeys.
pub const MAX_PEER_ID_BYTES: usize = 128;
/// The bound a transcript message id obeys.
pub const MAX_MESSAGE_ID_BYTES: usize = 284;
/// The largest legal `sequence` value.
pub const MAX_SEQUENCE: i64 = 2_147_483_647;
/// The largest legal manifest `seed` value.
pub const MAX_SEED: i64 = 2_147_483_647;
/// The largest legal manifest `duration_ticks` value.
pub const MAX_DURATION_TICKS: i64 = 216_000;
/// The largest legal `max_goals` / score value.
pub const MAX_GOALS: i64 = 99;
/// The largest legal countdown `remaining_ticks` value.
pub const MAX_COUNTDOWN_TICKS: i64 = 600;
/// The largest number of runtime capabilities a handshake may declare.
pub const MAX_CAPABILITIES: i64 = 16;

const MESSAGE_ID_PREFIX: &str = "GCMI;1;";
const WIRE_HEADER_PREFIX: &str = "GCOP;";
const MAX_TABLE_ITEMS: i64 = 256;
const MAX_NESTING_DEPTH: i64 = 12;

/// Mirrors `gc_sim::combat_snapshot::VERSION`.
pub const COMBAT_SCHEMA_VERSION: i64 = gc_sim::combat_snapshot::VERSION;

/// The versions this build's frozen manifest is pinned to
/// (`protocol.CURRENT_VERSIONS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentVersions {
    /// [`VERSION`].
    pub protocol: i64,
    /// `input_frame::VERSION`.
    pub input: i64,
    /// `match_snapshot::COMBAT_VERSION`.
    pub snapshot: i64,
    /// `input_tape::COMBAT_VERSION`.
    pub tape: i64,
    /// [`COMBAT_SCHEMA_VERSION`].
    pub combat: i64,
}

/// This build's pinned version set.
pub const CURRENT_VERSIONS: CurrentVersions = CurrentVersions {
    protocol: VERSION,
    input: input_frame::VERSION,
    snapshot: match_snapshot::COMBAT_VERSION,
    tape: input_tape::COMBAT_VERSION,
    combat: COMBAT_SCHEMA_VERSION,
};

// ---------------------------------------------------------------------------
// Slot / team wire identities (ARCHITECTURE.md §3 rule 3: preserved exactly)
// ---------------------------------------------------------------------------

/// The exact wire string for a canonical outfield slot id
/// (`gc_sim::input_frame::SLOT_ORDER[index].id`). `gc_sim::input_frame`
/// has no string form of `SlotId` of its own; this is the one place that
/// gap is bridged, and the strings are never renumbered or renamed.
#[must_use]
pub fn slot_wire_id(id: input_frame::SlotId) -> &'static str {
    use input_frame::SlotId::{Away1, Away2, Away3, Away4, Home1, Home2, Home3, Home4};
    match id {
        Home1 => "home_1",
        Home2 => "home_2",
        Home3 => "home_3",
        Home4 => "home_4",
        Away1 => "away_1",
        Away2 => "away_2",
        Away3 => "away_3",
        Away4 => "away_4",
    }
}

/// The reverse of [`slot_wire_id`].
#[must_use]
pub fn slot_id_from_wire(value: &str) -> Option<input_frame::SlotId> {
    use input_frame::SlotId::{Away1, Away2, Away3, Away4, Home1, Home2, Home3, Home4};
    Some(match value {
        "home_1" => Home1,
        "home_2" => Home2,
        "home_3" => Home3,
        "home_4" => Home4,
        "away_1" => Away1,
        "away_2" => Away2,
        "away_3" => Away3,
        "away_4" => Away4,
        _ => return None,
    })
}

/// The exact wire string for a fixture side.
#[must_use]
pub fn team_wire_str(team: input_frame::Team) -> &'static str {
    match team {
        input_frame::Team::Home => "home",
        input_frame::Team::Away => "away",
    }
}

/// The reverse of [`team_wire_str`].
#[must_use]
pub fn team_from_wire_str(value: &str) -> Option<input_frame::Team> {
    match value {
        "home" => Some(input_frame::Team::Home),
        "away" => Some(input_frame::Team::Away),
        _ => None,
    }
}

/// The OMP-1 canonical index (1..=8, `input_frame::SLOT_COUNT`) of a
/// canonical outfield slot wire id, or `None` when the id names no canonical
/// slot (a keeper has none, in every mode). ARCHITECTURE.md §3 rule 3: this index is the exact
/// wire-adjacent identity `input_frame::slot(index)` uses, never renumbered.
#[must_use]
pub fn slot_index(slot: &str) -> Option<i64> {
    let id = slot_id_from_wire(slot)?;
    (1..=input_frame::SLOT_COUNT).find(|&index| input_frame::slot(index).unwrap().id == id)
}

// ---------------------------------------------------------------------------
// Match mode
// ---------------------------------------------------------------------------

/// `SessionMatchMode`: how the eight canonical outfield slots are shared.
/// The pitch is always 4v4; only the number of humans and the size of one
/// human's owned set changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
    /// One human per side, each owning all four outfield slots on their side.
    OneVOne,
    /// Two humans per side, each owning a pair of outfield slots.
    TwoVTwo,
    /// Four humans per side, each owning one outfield slot.
    FourVFour,
}

impl MatchMode {
    /// The exact wire string (`"1v1"`, `"2v2"`, `"4v4"`).
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            MatchMode::OneVOne => "1v1",
            MatchMode::TwoVTwo => "2v2",
            MatchMode::FourVFour => "4v4",
        }
    }

    /// The reverse of [`MatchMode::wire_str`].
    #[must_use]
    pub fn from_wire_str(value: &str) -> Option<MatchMode> {
        match value {
            "1v1" => Some(MatchMode::OneVOne),
            "2v2" => Some(MatchMode::TwoVTwo),
            "4v4" => Some(MatchMode::FourVFour),
            _ => None,
        }
    }
}

/// `SessionMatchModeShape`: the seating shape one match mode implies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchModeShape {
    /// The mode this shape describes.
    pub mode: MatchMode,
    /// Total humans the mode seats across both teams.
    pub humans: i64,
    /// Canonical outfield slots one human owns.
    pub slots_per_human: i64,
    /// Humans the mode seats on one team.
    pub team_humans: i64,
}

/// `humans * slots_per_human` is always `input_frame::SLOT_COUNT`; 3v3 has no
/// entry because four outfield slots per team do not divide into three
/// humans.
#[must_use]
pub fn match_mode_shape(mode: MatchMode) -> MatchModeShape {
    match mode {
        MatchMode::OneVOne => MatchModeShape {
            mode,
            humans: 2,
            slots_per_human: 4,
            team_humans: 1,
        },
        MatchMode::TwoVTwo => MatchModeShape {
            mode,
            humans: 4,
            slots_per_human: 2,
            team_humans: 2,
        },
        MatchMode::FourVFour => MatchModeShape {
            mode,
            humans: 8,
            slots_per_human: 1,
            team_humans: 4,
        },
    }
}

/// The one place an unsupported size is refused: `mode` is a [`Value`]
/// (`manifest.match_mode` is dynamically typed until this validates it)
/// naming a supported match mode, or an error. `"3v3"`
/// fails here exactly like `"5v5"` or a misspelling, because
/// [`match_mode_shape`]'s closed set — not a special case — is what defines
/// the supported shapes.
pub fn match_mode(mode: &Value) -> Result<MatchModeShape> {
    let shape = mode.as_str().and_then(MatchMode::from_wire_str);
    match shape {
        Some(mode) => Ok(match_mode_shape(mode)),
        None => failure(
            ErrorCode::UnsupportedMatchMode,
            format!(
                "match mode {} is not a supported size",
                mode.as_str().unwrap_or("<non-string>")
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Scalar validators
// ---------------------------------------------------------------------------

fn is_ascii_id_start(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn is_ascii_id_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
}

/// A bounded opaque ASCII id: alphanumeric start, then alphanumerics plus
/// `_`, `.`, `-`.
fn is_canonical_id(value: &str, maximum: usize) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= maximum
        && is_ascii_id_start(bytes[0])
        && bytes[1..].iter().all(|&byte| is_ascii_id_continue(byte))
}

fn validate_id(value: &Value, name: &str, maximum: usize) -> Result<()> {
    let ok = value
        .as_str()
        .is_some_and(|value| is_canonical_id(value, maximum));
    if !ok {
        return failure(
            ErrorCode::Malformed,
            format!("{name} must be a bounded opaque ASCII id"),
        );
    }
    Ok(())
}

fn validate_id_str(value: &str, name: &str, maximum: usize) -> Result<()> {
    if !is_canonical_id(value, maximum) {
        return failure(
            ErrorCode::Malformed,
            format!("{name} must be a bounded opaque ASCII id"),
        );
    }
    Ok(())
}

fn validate_player_id(value: &Value, name: &str) -> Result<()> {
    validate_id(value, name, input_frame::MAX_PLAYER_ID_BYTES)
}

fn validate_integer(value: &Value, name: &str, minimum: i64, maximum: i64) -> Result<()> {
    match value.as_int() {
        Some(number) if number >= minimum && number <= maximum => Ok(()),
        _ => failure(
            ErrorCode::Malformed,
            format!("{name} must be a bounded integer"),
        ),
    }
}

fn is_canonical_hash(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_hash(value: &Value, name: &str) -> Result<()> {
    let ok = value.as_str().is_some_and(is_canonical_hash);
    if !ok {
        return failure(
            ErrorCode::Malformed,
            format!("{name} must be a 16-character lowercase hash"),
        );
    }
    Ok(())
}

fn length_prefix(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

/// A declared build identity is an ordinary opaque id, or absent. Absence is
/// legal on purpose: the field is additive, so a peer built before it
/// existed still speaks a handshake this build accepts.
pub fn validate_build_id(build_id: &Value) -> Result<()> {
    if build_id.is_nil() {
        return Ok(());
    }
    validate_id(build_id, "handshake build id", MAX_ID_BYTES)
}

// ---------------------------------------------------------------------------
// message_id / manifest_id / assignment_id
// ---------------------------------------------------------------------------

/// `protocol.message_id`: the injective transcript identity of one
/// `(session_id, peer_id, sequence)` triple.
pub fn message_id(session_id: &str, peer_id: &str, sequence: i64) -> Result<String> {
    validate_id_str(session_id, "message.session_id", MAX_SESSION_ID_BYTES)?;
    validate_id_str(peer_id, "message.peer_id", MAX_PEER_ID_BYTES)?;
    if !(0..=MAX_SEQUENCE).contains(&sequence) {
        return failure(
            ErrorCode::Malformed,
            "message.sequence must be a bounded integer",
        );
    }
    let sequence_text = sequence.to_string();
    let id = format!(
        "{MESSAGE_ID_PREFIX}{}{}{}",
        length_prefix(session_id),
        length_prefix(peer_id),
        length_prefix(&sequence_text)
    );
    assert!(
        id.len() <= MAX_MESSAGE_ID_BYTES,
        "message id bound is inconsistent"
    );
    Ok(id)
}

/// The deterministic identity of a valid manifest.
///
/// # Panics
///
/// Panics if `manifest` is not already a valid manifest — a programmer
/// error (AGENTS.md §7): callers are expected to have validated first.
#[must_use]
pub fn manifest_id(manifest: &Value) -> String {
    validate_manifest(manifest).expect("manifest_id requires an already-valid manifest");
    fnv1a64::hash(format!("GCOM;{}", encode_value(manifest, 0)).as_bytes())
}

/// `protocol.assignment_id`: the identity of one ownership generation. The
/// epoch is folded into the digest, so republishing byte-identical ownership
/// still mints a distinct generation.
///
/// # Panics
///
/// Panics if `assignments` is not a valid slot-assignment array, or if
/// `epoch` is out of range — both programmer errors per AGENTS.md §7.
#[must_use]
pub fn assignment_id(assignments: &Value, epoch: i64) -> String {
    validate_slot_assignments(assignments)
        .expect("assignment_id requires already-valid slot assignments");
    assert!(
        (0..=MAX_SEQUENCE).contains(&epoch),
        "assignment epoch must be a bounded non-negative integer"
    );
    let preimage = format!(
        "GCOA;{}{}",
        length_prefix(&epoch.to_string()),
        encode_value(assignments, 0)
    );
    fnv1a64::hash(preimage.as_bytes())
}

// ---------------------------------------------------------------------------
// Canonical value codec
// ---------------------------------------------------------------------------

fn encode_value(value: &Value, depth: i64) -> String {
    assert!(
        depth <= MAX_NESTING_DEPTH,
        "protocol value nesting is too deep"
    );
    match value {
        Value::Nil => "z".to_string(),
        Value::Bool(flag) => if *flag { "b1" } else { "b0" }.to_string(),
        Value::Int(number) => {
            let text = number.to_string();
            format!("i{}:{}", text.len(), text)
        }
        Value::Str(text) => format!("s{}:{}", text.len(), text),
        Value::Table(entries) => {
            let mut keys: Vec<&Value> = Vec::with_capacity(entries.len());
            for (key, _) in entries {
                let valid = matches!(key, Value::Str(_)) || matches!(key, Value::Int(n) if *n >= 1);
                assert!(valid, "protocol table key is invalid");
                keys.push(key);
            }
            keys.sort_by(|left, right| match (left, right) {
                (Value::Int(a), Value::Int(b)) => a.cmp(b),
                (Value::Str(a), Value::Str(b)) => a.cmp(b),
                (Value::Int(_), Value::Str(_)) => std::cmp::Ordering::Less,
                (Value::Str(_), Value::Int(_)) => std::cmp::Ordering::Greater,
                _ => unreachable!("table keys are only ever Int or Str"),
            });
            let mut parts = format!("t{}:", keys.len());
            for key in keys {
                let item = value
                    .as_table()
                    .unwrap()
                    .iter()
                    .find(|(k, _)| k == key)
                    .unwrap()
                    .1
                    .clone();
                parts.push_str(&encode_value(key, depth + 1));
                parts.push_str(&encode_value(&item, depth + 1));
            }
            parts
        }
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    index: usize,
}

fn is_canonical_unsigned_bytes(bytes: &[u8]) -> bool {
    if bytes == b"0" {
        return true;
    }
    !bytes.is_empty()
        && (b'1'..=b'9').contains(&bytes[0])
        && bytes[1..].iter().all(u8::is_ascii_digit)
}

fn is_canonical_integer_bytes(bytes: &[u8]) -> bool {
    if is_canonical_unsigned_bytes(bytes) {
        return true;
    }
    bytes.len() >= 2
        && bytes[0] == b'-'
        && (b'1'..=b'9').contains(&bytes[1])
        && bytes[2..].iter().all(u8::is_ascii_digit)
}

/// Parses a canonical (already-validated-shape) decimal byte string into an
/// `i64`, `None` on overflow. Digit-count-capped first so a hostile,
/// arbitrarily long digit run never risks integer overflow.
fn parse_bounded_decimal(bytes: &[u8]) -> Option<i64> {
    if bytes.len() > 18 {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    text.parse::<i64>().ok()
}

fn decode_length(decoder: &mut Decoder) -> std::result::Result<i64, String> {
    let rest = &decoder.bytes[decoder.index..];
    let colon = rest.iter().position(|&b| b == b':');
    let colon = match colon {
        Some(colon) => colon,
        None => return Err("protocol value length is missing".to_string()),
    };
    let raw = &rest[..colon];
    if !is_canonical_unsigned_bytes(raw) {
        return Err("protocol value length is noncanonical".to_string());
    }
    decoder.index += colon + 1;
    match parse_bounded_decimal(raw) {
        Some(length) if length >= 0 && length as usize <= MAX_WIRE_BYTES => Ok(length),
        _ => Err("protocol value length exceeds the wire bound".to_string()),
    }
}

fn decode_value(decoder: &mut Decoder, depth: i64) -> std::result::Result<Value, String> {
    if depth > MAX_NESTING_DEPTH || decoder.index >= decoder.bytes.len() {
        return Err("protocol value is truncated or too deeply nested".to_string());
    }
    let tag = decoder.bytes[decoder.index];
    decoder.index += 1;
    match tag {
        b'z' => Ok(Value::Nil),
        b'b' => {
            if decoder.index >= decoder.bytes.len() {
                return Err("protocol boolean is invalid".to_string());
            }
            let flag = decoder.bytes[decoder.index];
            decoder.index += 1;
            match flag {
                b'0' => Ok(Value::Bool(false)),
                b'1' => Ok(Value::Bool(true)),
                _ => Err("protocol boolean is invalid".to_string()),
            }
        }
        b'i' | b's' => {
            let length = decode_length(decoder)? as usize;
            let finish = decoder.index + length;
            if finish > decoder.bytes.len() {
                return Err("protocol value is truncated".to_string());
            }
            let raw = &decoder.bytes[decoder.index..finish];
            decoder.index = finish;
            if tag == b's' {
                let text = std::str::from_utf8(raw)
                    .map_err(|_| "protocol string is not valid UTF-8".to_string())?;
                return Ok(Value::Str(text.to_string()));
            }
            if !is_canonical_integer_bytes(raw) {
                return Err("protocol integer is noncanonical".to_string());
            }
            match parse_bounded_decimal(raw) {
                Some(value) => Ok(Value::Int(value)),
                None => Err("protocol integer is outside the supported range".to_string()),
            }
        }
        b't' => {
            let count = decode_length(decoder)?;
            if count > MAX_TABLE_ITEMS {
                return Err("protocol table exceeds the item bound".to_string());
            }
            let mut result: Vec<(Value, Value)> = Vec::with_capacity(count.max(0) as usize);
            for _ in 0..count {
                let key = decode_value(decoder, depth + 1)?;
                let key_ok =
                    matches!(&key, Value::Str(_)) || matches!(&key, Value::Int(n) if *n >= 1);
                if !key_ok || result.iter().any(|(existing, _)| *existing == key) {
                    return Err("protocol table key is invalid or duplicated".to_string());
                }
                let item = decode_value(decoder, depth + 1)?;
                if item.is_nil() {
                    return Err("protocol tables cannot encode present nil fields".to_string());
                }
                result.push((key, item));
            }
            Ok(Value::Table(result))
        }
        _ => Err("protocol value tag is unknown".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Runtime identity
// ---------------------------------------------------------------------------

const RUNTIME_FIELDS: &[&str] = &[
    "version",
    "runtime_id",
    "runtime_revision",
    "presentation_id",
    "capabilities",
];

/// `protocol.validate_runtime`.
pub fn validate_runtime(runtime: &Value) -> Result<()> {
    if !has_only_fields(runtime, RUNTIME_FIELDS) {
        return failure(
            ErrorCode::Malformed,
            "runtime identity must contain only canonical fields",
        );
    }
    let version = runtime.get("version").unwrap_or(&Value::Nil);
    if version.as_int().is_none() {
        return failure(
            ErrorCode::Malformed,
            "runtime identity version must be an integer",
        );
    }
    if version.as_int() != Some(RUNTIME_IDENTITY_VERSION) {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported runtime identity version",
        );
    }
    for field in ["runtime_id", "runtime_revision", "presentation_id"] {
        validate_id(
            runtime.get(field).unwrap_or(&Value::Nil),
            &format!("runtime.{field}"),
            MAX_ID_BYTES,
        )?;
    }
    let capabilities = runtime.get("capabilities").unwrap_or(&Value::Nil);
    if !is_array_value(capabilities, None, Some(MAX_CAPABILITIES)) {
        return failure(
            ErrorCode::Malformed,
            "runtime capabilities must be a bounded canonical array",
        );
    }
    let count = capabilities.len() as i64;
    let mut previous: Option<&str> = None;
    for index in 1..=count {
        let capability = capabilities.get_index(index).unwrap();
        validate_id(capability, "runtime capability", MAX_ID_BYTES)?;
        let text = capability.as_str().unwrap();
        if let Some(previous) = previous
            && text <= previous
        {
            return failure(
                ErrorCode::Malformed,
                "runtime capabilities must be sorted and unique",
            );
        }
        previous = Some(text);
    }
    Ok(())
}

fn deep_copy(value: &Value) -> Value {
    value.clone()
}

/// `protocol.new_runtime`: a defensively copied, validated runtime identity.
pub fn new_runtime(runtime: &Value) -> Result<Value> {
    validate_runtime(runtime)?;
    Ok(deep_copy(runtime))
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const MANIFEST_FIELDS: &[&str] = &[
    "version",
    "session_id",
    "protocol_version",
    "input_version",
    "snapshot_version",
    "tape_version",
    "combat_schema_version",
    "build_id",
    "source_id",
    "content_id",
    "tuning_id",
    "match_config_id",
    "fixture_id",
    "arena_id",
    "combat_rules_id",
    "gameplay_ai_policy_id",
    "combat_status",
    "seed",
    "tick_rate",
    "duration_ticks",
    "max_goals",
    "match_mode",
    "teams",
    "slots",
];
const TEAM_FIELDS: &[&str] = &["team", "team_id", "roster"];
const ROSTER_PLAYER_FIELDS: &[&str] = &["player_id", "position", "loadout_id", "family_id"];
const MANIFEST_SLOT_FIELDS: &[&str] = &["slot", "team", "player_id"];
const SLOT_PRODUCER_FIELDS: &[&str] = &[
    "slot",
    "team",
    "player_id",
    "producer_kind",
    "producer_id",
    "bot_seed",
];
const POSITIONS: &[&str] = &["keeper", "defender", "midfielder", "forward"];
const COMBAT_STATUSES: &[&str] = &["provisional_114", "accepted_proceed", "accepted_revision"];
const MANIFEST_COMPARE_FIELDS: &[&str] = &[
    "version",
    "session_id",
    "protocol_version",
    "input_version",
    "snapshot_version",
    "tape_version",
    "combat_schema_version",
    "build_id",
    "source_id",
    "content_id",
    "tuning_id",
    "match_config_id",
    "fixture_id",
    "arena_id",
    "combat_rules_id",
    "gameplay_ai_policy_id",
    "combat_status",
    "seed",
    "tick_rate",
    "duration_ticks",
    "max_goals",
    "match_mode",
];
const RUNTIME_COMPARE_FIELDS: &[&str] = &[
    "version",
    "runtime_id",
    "runtime_revision",
    "presentation_id",
];

fn validate_roster(roster: &Value, team: &str, seen_players: &mut Vec<String>) -> Result<()> {
    if !is_array_value(roster, Some(input_frame::FIXTURE_TEAM_SIZE), None) {
        return failure(
            ErrorCode::Malformed,
            format!("{team} roster must contain exactly five players"),
        );
    }
    let mut keeper_count = 0;
    for index in 1..=input_frame::FIXTURE_TEAM_SIZE {
        let player = roster.get_index(index).unwrap();
        if !has_only_fields(player, ROSTER_PLAYER_FIELDS) {
            return failure(
                ErrorCode::Malformed,
                format!("{team} roster player has unknown fields"),
            );
        }
        let player_id_value = player.get("player_id").unwrap_or(&Value::Nil);
        validate_player_id(player_id_value, &format!("{team} roster player id"))?;
        let player_id = player_id_value.as_str().unwrap().to_string();
        let position = player.get("position").and_then(Value::as_str).unwrap_or("");
        if !POSITIONS.contains(&position) {
            return failure(
                ErrorCode::Malformed,
                format!("{team} roster player position is invalid"),
            );
        }
        if seen_players.contains(&player_id) {
            return failure(ErrorCode::Malformed, "manifest player ids must be unique");
        }
        seen_players.push(player_id);
        let has_loadout = player.get("loadout_id").is_some_and(|v| !v.is_nil());
        let has_family = player.get("family_id").is_some_and(|v| !v.is_nil());
        if position == "keeper" {
            keeper_count += 1;
            if has_loadout || has_family {
                return failure(ErrorCode::Malformed, "keepers cannot have combat loadouts");
            }
        } else {
            validate_id(
                player.get("loadout_id").unwrap_or(&Value::Nil),
                &format!("{team} roster loadout_id"),
                MAX_ID_BYTES,
            )?;
            validate_id(
                player.get("family_id").unwrap_or(&Value::Nil),
                &format!("{team} roster family_id"),
                MAX_ID_BYTES,
            )?;
        }
        if index == 1 && position != "keeper" {
            return failure(
                ErrorCode::Malformed,
                format!("{team} roster must place its keeper first"),
            );
        }
    }
    if keeper_count != 1 {
        return failure(
            ErrorCode::Malformed,
            format!("{team} roster must contain exactly one keeper"),
        );
    }
    Ok(())
}

/// `protocol.validate_manifest`.
pub fn validate_manifest(manifest: &Value) -> Result<()> {
    if !has_only_fields(manifest, MANIFEST_FIELDS) {
        return failure(
            ErrorCode::Malformed,
            "session manifest must contain only canonical fields",
        );
    }
    let versions: &[(&str, i64)] = &[
        ("version", MANIFEST_VERSION),
        ("protocol_version", VERSION),
        ("input_version", input_frame::VERSION),
        ("snapshot_version", match_snapshot::COMBAT_VERSION),
        ("tape_version", input_tape::COMBAT_VERSION),
        ("combat_schema_version", COMBAT_SCHEMA_VERSION),
    ];
    for (field, expected) in versions {
        let value = manifest.get(field).unwrap_or(&Value::Nil);
        if value.as_int().is_none() {
            return failure(
                ErrorCode::Malformed,
                format!("manifest {field} must be an integer"),
            );
        }
        if value.as_int() != Some(*expected) {
            return failure(
                ErrorCode::UnsupportedVersion,
                format!("unsupported manifest {field}"),
            );
        }
    }
    for field in [
        "session_id",
        "build_id",
        "source_id",
        "content_id",
        "tuning_id",
        "match_config_id",
        "fixture_id",
        "arena_id",
        "combat_rules_id",
        "gameplay_ai_policy_id",
    ] {
        validate_id(
            manifest.get(field).unwrap_or(&Value::Nil),
            &format!("manifest.{field}"),
            MAX_ID_BYTES,
        )?;
    }
    let combat_status = manifest
        .get("combat_status")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !COMBAT_STATUSES.contains(&combat_status) {
        return failure(ErrorCode::Malformed, "manifest combat status is invalid");
    }
    match_mode(manifest.get("match_mode").unwrap_or(&Value::Nil))?;
    let integer_fields: &[(&str, i64, i64)] = &[
        ("seed", 0, MAX_SEED),
        (
            "tick_rate",
            fixed_clock::TICK_RATE as i64,
            fixed_clock::TICK_RATE as i64,
        ),
        ("duration_ticks", 1, MAX_DURATION_TICKS),
        ("max_goals", 1, MAX_GOALS),
    ];
    for (field, minimum, maximum) in integer_fields {
        validate_integer(
            manifest.get(field).unwrap_or(&Value::Nil),
            &format!("manifest.{field}"),
            *minimum,
            *maximum,
        )?;
    }
    let teams = manifest.get("teams").unwrap_or(&Value::Nil);
    if !is_array_value(teams, Some(2), None) {
        return failure(
            ErrorCode::Malformed,
            "manifest teams must contain home then away",
        );
    }
    let mut all_players: Vec<String> = Vec::new();
    let mut players_by_team: [Vec<(String, &str)>; 2] = [Vec::new(), Vec::new()];
    for (team_position, expected_team) in ["home", "away"].into_iter().enumerate() {
        let team = teams.get_index(team_position as i64 + 1).unwrap();
        let team_name = team.get("team").and_then(Value::as_str);
        if !has_only_fields(team, TEAM_FIELDS) || team_name != Some(expected_team) {
            return failure(
                ErrorCode::Malformed,
                "manifest teams violate canonical home/away order",
            );
        }
        validate_id(
            team.get("team_id").unwrap_or(&Value::Nil),
            &format!("{expected_team} team id"),
            MAX_ID_BYTES,
        )?;
        let roster = team.get("roster").unwrap_or(&Value::Nil);
        validate_roster(roster, expected_team, &mut all_players)?;
        for index in 1..=input_frame::FIXTURE_TEAM_SIZE {
            let player = roster.get_index(index).unwrap();
            let player_id = player.get("player_id").and_then(Value::as_str).unwrap();
            let position = player.get("position").and_then(Value::as_str).unwrap();
            players_by_team[team_position].push((player_id.to_string(), position_leak(position)));
        }
    }
    let home_team_id = teams
        .get_index(1)
        .unwrap()
        .get("team_id")
        .and_then(Value::as_str);
    let away_team_id = teams
        .get_index(2)
        .unwrap()
        .get("team_id")
        .and_then(Value::as_str);
    if home_team_id == away_team_id {
        return failure(
            ErrorCode::Malformed,
            "manifest teams must have distinct ids",
        );
    }
    let slots = manifest.get("slots").unwrap_or(&Value::Nil);
    if !is_array_value(slots, Some(input_frame::SLOT_COUNT), None) {
        return failure(
            ErrorCode::Malformed,
            "manifest must contain exactly eight canonical slots",
        );
    }
    let mut assigned_players: Vec<String> = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).unwrap();
        let assignment = slots.get_index(index).unwrap();
        let matches_shape = has_only_fields(assignment, MANIFEST_SLOT_FIELDS)
            && assignment.get("slot").and_then(Value::as_str) == Some(slot_wire_id(slot.id))
            && assignment.get("team").and_then(Value::as_str) == Some(team_wire_str(slot.team));
        if !matches_shape {
            return failure(
                ErrorCode::Malformed,
                "manifest slots violate OMP-1 canonical order",
            );
        }
        let player_id_value = assignment.get("player_id").unwrap_or(&Value::Nil);
        validate_player_id(player_id_value, "manifest slot player id")?;
        let player_id = player_id_value.as_str().unwrap();
        let team_position = if slot.team == input_frame::Team::Home {
            0
        } else {
            1
        };
        let found = players_by_team[team_position]
            .iter()
            .find(|(id, _)| id == player_id)
            .map(|(_, position)| *position);
        if found.is_none() || found == Some("keeper") {
            return failure(
                ErrorCode::Malformed,
                "manifest slot must name an outfielder on its team",
            );
        }
        if assigned_players.iter().any(|id| id == player_id) {
            return failure(
                ErrorCode::Malformed,
                "manifest outfielder owns multiple slots",
            );
        }
        assigned_players.push(player_id.to_string());
    }
    for team in players_by_team {
        for (player_id, position) in team {
            if position != "keeper" && !assigned_players.contains(&player_id) {
                return failure(
                    ErrorCode::Malformed,
                    "manifest outfielder is missing a canonical slot",
                );
            }
        }
    }
    let _ = &all_players;
    Ok(())
}

/// Maps a validated `position` string to the equal `'static` string from
/// [`POSITIONS`], so `players_by_team` can borrow `'static` instead of
/// tying its lifetime to the manifest `Value`.
fn position_leak(position: &str) -> &'static str {
    POSITIONS
        .iter()
        .find(|&&candidate| candidate == position)
        .copied()
        .unwrap_or("")
}

/// `protocol.new_manifest`: a defensively copied, validated manifest.
pub fn new_manifest(manifest: &Value) -> Result<Value> {
    validate_manifest(manifest)?;
    Ok(deep_copy(manifest))
}

// ---------------------------------------------------------------------------
// Identity comparison
// ---------------------------------------------------------------------------

/// `SessionIdentityDifference`: the first field two identities disagree on.
#[derive(Clone, Debug, PartialEq)]
pub struct Difference {
    /// Dotted field path, e.g. `"manifest.slots.1.player_id"`.
    pub path: String,
    /// The expected value's canonical wire form.
    pub expected: Value,
    /// The actual value's canonical wire form.
    pub actual: Value,
}

fn difference(path: impl Into<String>, expected: &Value, actual: &Value) -> Difference {
    Difference {
        path: path.into(),
        expected: expected.clone(),
        actual: actual.clone(),
    }
}

/// The field-by-field difference between two runtime identities.
///
/// # Panics
///
/// Panics if either runtime is invalid (AGENTS.md §7: a programmer error,
/// not an expected failure — callers are expected to have validated first).
#[must_use]
pub fn runtime_difference(expected: &Value, actual: &Value) -> Option<Difference> {
    validate_runtime(expected).expect("runtime_difference requires a valid expected runtime");
    validate_runtime(actual).expect("runtime_difference requires a valid actual runtime");
    for field in RUNTIME_COMPARE_FIELDS {
        let left = expected.get(field).unwrap_or(&Value::Nil);
        let right = actual.get(field).unwrap_or(&Value::Nil);
        if left != right {
            return Some(difference(format!("runtime.{field}"), left, right));
        }
    }
    let left_capabilities = expected.get("capabilities").unwrap_or(&Value::Nil);
    let right_capabilities = actual.get("capabilities").unwrap_or(&Value::Nil);
    if left_capabilities.len() != right_capabilities.len() {
        return Some(difference(
            "runtime.capabilities.length",
            &Value::int(left_capabilities.len() as i64),
            &Value::int(right_capabilities.len() as i64),
        ));
    }
    for index in 1..=left_capabilities.len() as i64 {
        let left = left_capabilities.get_index(index).unwrap();
        let right = right_capabilities.get_index(index).unwrap();
        if left != right {
            return Some(difference(
                format!("runtime.capabilities.{index}"),
                left,
                right,
            ));
        }
    }
    None
}

/// The field-by-field difference between two manifests.
///
/// # Panics
///
/// Panics if either manifest is invalid (AGENTS.md §7: a programmer error,
/// not an expected failure — callers are expected to have validated first).
#[must_use]
pub fn manifest_difference(expected: &Value, actual: &Value) -> Option<Difference> {
    validate_manifest(expected).expect("manifest_difference requires a valid expected manifest");
    validate_manifest(actual).expect("manifest_difference requires a valid actual manifest");
    for field in MANIFEST_COMPARE_FIELDS {
        let left = expected.get(field).unwrap_or(&Value::Nil);
        let right = actual.get(field).unwrap_or(&Value::Nil);
        if left != right {
            return Some(difference(format!("manifest.{field}"), left, right));
        }
    }
    for team_index in 1..=2i64 {
        let left_team = expected
            .get("teams")
            .unwrap()
            .get_index(team_index)
            .unwrap();
        let right_team = actual.get("teams").unwrap().get_index(team_index).unwrap();
        for field in ["team", "team_id"] {
            let left = left_team.get(field).unwrap_or(&Value::Nil);
            let right = right_team.get(field).unwrap_or(&Value::Nil);
            if left != right {
                return Some(difference(
                    format!("manifest.teams.{team_index}.{field}"),
                    left,
                    right,
                ));
            }
        }
        for player_index in 1..=input_frame::FIXTURE_TEAM_SIZE {
            let left_player = left_team
                .get("roster")
                .unwrap()
                .get_index(player_index)
                .unwrap();
            let right_player = right_team
                .get("roster")
                .unwrap()
                .get_index(player_index)
                .unwrap();
            for field in ["player_id", "position", "loadout_id", "family_id"] {
                let left = left_player.get(field).unwrap_or(&Value::Nil);
                let right = right_player.get(field).unwrap_or(&Value::Nil);
                if left != right {
                    return Some(difference(
                        format!("manifest.teams.{team_index}.roster.{player_index}.{field}"),
                        left,
                        right,
                    ));
                }
            }
        }
    }
    for index in 1..=input_frame::SLOT_COUNT {
        let left_slot = expected.get("slots").unwrap().get_index(index).unwrap();
        let right_slot = actual.get("slots").unwrap().get_index(index).unwrap();
        for field in ["slot", "team", "player_id"] {
            let left = left_slot.get(field).unwrap_or(&Value::Nil);
            let right = right_slot.get(field).unwrap_or(&Value::Nil);
            if left != right {
                return Some(difference(
                    format!("manifest.slots.{index}.{field}"),
                    left,
                    right,
                ));
            }
        }
    }
    None
}

/// `protocol.compare_manifest`.
pub fn compare_manifest(expected: &Value, actual: &Value) -> Result<()> {
    match manifest_difference(expected, actual) {
        Some(diff) => Err(Error::with_path(
            ErrorCode::IdentityMismatch,
            format!("deterministic manifest mismatch at {}", diff.path),
            diff.path,
        )),
        None => Ok(()),
    }
}

/// `protocol.compare_runtime`.
pub fn compare_runtime(expected: &Value, actual: &Value) -> Result<()> {
    match runtime_difference(expected, actual) {
        Some(diff) => Err(Error::with_path(
            ErrorCode::RuntimeMismatch,
            format!("runtime compatibility mismatch at {}", diff.path),
            diff.path,
        )),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Slot ownership
// ---------------------------------------------------------------------------

struct ProducerOwnership {
    producer_kind: String,
    team: input_frame::Team,
    slots: Vec<String>,
}

/// Slot ownership is a function from the eight canonical slots onto
/// producers: every slot names exactly one declared source, but one human
/// source may cover several slots.
fn collect_slot_assignments(assignments: &Value) -> Result<Vec<(String, ProducerOwnership)>> {
    if !is_array_value(assignments, Some(input_frame::SLOT_COUNT), None) {
        return failure(
            ErrorCode::Malformed,
            "slot assignment must contain exactly eight producers",
        );
    }
    let mut producers: Vec<(String, ProducerOwnership)> = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).unwrap();
        let assignment = assignments.get_index(index).unwrap();
        let matches_shape = has_only_fields(assignment, SLOT_PRODUCER_FIELDS)
            && assignment.get("slot").and_then(Value::as_str) == Some(slot_wire_id(slot.id))
            && assignment.get("team").and_then(Value::as_str) == Some(team_wire_str(slot.team));
        if !matches_shape {
            return failure(
                ErrorCode::Malformed,
                "slot producers violate OMP-1 canonical order",
            );
        }
        validate_player_id(
            assignment.get("player_id").unwrap_or(&Value::Nil),
            "slot assignment player id",
        )?;
        validate_id(
            assignment.get("producer_id").unwrap_or(&Value::Nil),
            "slot assignment producer id",
            MAX_ID_BYTES,
        )?;
        let producer_kind = assignment
            .get("producer_kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        let bot_seed = assignment.get("bot_seed").unwrap_or(&Value::Nil);
        match producer_kind {
            "peer" => {
                if !bot_seed.is_nil() {
                    return failure(
                        ErrorCode::Malformed,
                        "peer slot producer cannot carry a bot seed",
                    );
                }
            }
            "bot" => {
                validate_integer(bot_seed, "bot seed", 0, MAX_SEED)?;
            }
            _ => return failure(ErrorCode::Malformed, "slot producer kind is invalid"),
        }
        let producer_id = assignment
            .get("producer_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let slot_id = slot_wire_id(slot.id).to_string();
        match producers.iter_mut().find(|(id, _)| *id == producer_id) {
            None => producers.push((
                producer_id,
                ProducerOwnership {
                    producer_kind: producer_kind.to_string(),
                    team: slot.team,
                    slots: vec![slot_id],
                },
            )),
            Some((_, owned)) if owned.producer_kind != producer_kind => {
                return failure(
                    ErrorCode::Malformed,
                    "slot producer ids must not cross the peer and bot namespaces",
                );
            }
            Some((_, _)) if producer_kind != "peer" => {
                return failure(
                    ErrorCode::Malformed,
                    "a bot producer drives exactly one canonical slot",
                );
            }
            Some((_, owned)) if owned.team != slot.team => {
                return failure(
                    ErrorCode::Malformed,
                    "a human producer owns slots on exactly one team",
                );
            }
            Some((_, owned)) => owned.slots.push(slot_id),
        }
    }
    Ok(producers)
}

fn validate_slot_assignments(assignments: &Value) -> Result<()> {
    collect_slot_assignments(assignments).map(|_| ())
}

/// `protocol.validate_assignment_manifest`.
pub fn validate_assignment_manifest(manifest: &Value, assignments: &Value) -> Result<()> {
    validate_manifest(manifest)?;
    let producers = collect_slot_assignments(assignments)?;
    let slots = manifest.get("slots").unwrap();
    for index in 1..=input_frame::SLOT_COUNT {
        let manifest_player = slots
            .get_index(index)
            .unwrap()
            .get("player_id")
            .and_then(Value::as_str);
        let assignment_player = assignments
            .get_index(index)
            .unwrap()
            .get("player_id")
            .and_then(Value::as_str);
        if manifest_player != assignment_player {
            return failure(
                ErrorCode::IdentityMismatch,
                format!("slot assignment player differs at manifest.slots.{index}.player_id"),
            );
        }
    }
    let mode =
        MatchMode::from_wire_str(manifest.get("match_mode").and_then(Value::as_str).unwrap())
            .expect("manifest already validated");
    let shape = match_mode_shape(mode);
    for index in 1..=input_frame::SLOT_COUNT {
        let producer_id = assignments
            .get_index(index)
            .unwrap()
            .get("producer_id")
            .and_then(Value::as_str)
            .unwrap();
        let (_, owned) = producers.iter().find(|(id, _)| id == producer_id).unwrap();
        if owned.producer_kind == "peer" && owned.slots.len() as i64 != shape.slots_per_human {
            return failure(
                ErrorCode::InvalidOwnership,
                format!(
                    "{} owns {} canonical slots but {} seats {} per human",
                    producer_id,
                    owned.slots.len(),
                    shape.mode.wire_str(),
                    shape.slots_per_human
                ),
            );
        }
    }
    Ok(())
}

/// The owned set of one producer, in OMP-1 order.
///
/// # Panics
///
/// Panics if `assignments` is not a valid slot-assignment array (AGENTS.md
/// §7: a programmer error — callers are expected to have validated first).
#[must_use]
pub fn owned_slots(assignments: &Value, producer_id: &str) -> Vec<String> {
    validate_slot_assignments(assignments).expect("owned_slots requires valid slot assignments");
    let mut owned = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = assignments.get_index(index).unwrap();
        if producer.get("producer_id").and_then(Value::as_str) == Some(producer_id) {
            owned.push(
                producer
                    .get("slot")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
            );
        }
    }
    owned
}

/// `protocol.validate_slot_set`: the wire shape of a requested owned set —
/// canonical outfield slot ids in strictly ascending canonical order.
pub fn validate_slot_set(slots: &Value) -> Result<()> {
    if !is_array_value(slots, None, Some(input_frame::SLOT_COUNT)) || slots.is_empty() {
        return failure(
            ErrorCode::Malformed,
            "a slot set must hold one to eight canonical slots",
        );
    }
    let mut previous = 0;
    for index in 1..=slots.len() as i64 {
        let slot = slots.get_index(index).unwrap();
        let slot_index_value = slot.as_str().and_then(slot_index);
        let current = match slot_index_value {
            Some(value) => value,
            None => {
                return failure(
                    ErrorCode::Malformed,
                    "slot set names an unknown canonical slot",
                );
            }
        };
        if current <= previous {
            return failure(
                ErrorCode::Malformed,
                "slot set is not in ascending canonical order",
            );
        }
        previous = current;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Control message kind / lifecycle phase
// ---------------------------------------------------------------------------

/// `SessionMessageKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// `"handshake"`.
    Handshake,
    /// `"manifest_proposal"`.
    ManifestProposal,
    /// `"manifest_accept"`.
    ManifestAccept,
    /// `"peer_assignment"`.
    PeerAssignment,
    /// `"slot_assignment"`.
    SlotAssignment,
    /// `"ready"`.
    Ready,
    /// `"pair_preference"`.
    PairPreference,
    /// `"pair_preference_result"`.
    PairPreferenceResult,
    /// `"countdown"`.
    Countdown,
    /// `"start"`.
    Start,
    /// `"match_phase"`.
    MatchPhase,
    /// `"hash_report"`.
    HashReport,
    /// `"result_ack"`.
    ResultAck,
    /// `"abort"`.
    Abort,
    /// `"disconnect"`.
    Disconnect,
}

/// Every message kind, in the same order `MessageKind::allowed_body_fields`
/// and `MessageKind::allowed_phases` declare them.
pub const ALL_MESSAGE_KINDS: &[MessageKind] = &[
    MessageKind::Handshake,
    MessageKind::ManifestProposal,
    MessageKind::ManifestAccept,
    MessageKind::PeerAssignment,
    MessageKind::SlotAssignment,
    MessageKind::Ready,
    MessageKind::PairPreference,
    MessageKind::PairPreferenceResult,
    MessageKind::Countdown,
    MessageKind::Start,
    MessageKind::MatchPhase,
    MessageKind::HashReport,
    MessageKind::ResultAck,
    MessageKind::Abort,
    MessageKind::Disconnect,
];

impl MessageKind {
    /// The exact wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            MessageKind::Handshake => "handshake",
            MessageKind::ManifestProposal => "manifest_proposal",
            MessageKind::ManifestAccept => "manifest_accept",
            MessageKind::PeerAssignment => "peer_assignment",
            MessageKind::SlotAssignment => "slot_assignment",
            MessageKind::Ready => "ready",
            MessageKind::PairPreference => "pair_preference",
            MessageKind::PairPreferenceResult => "pair_preference_result",
            MessageKind::Countdown => "countdown",
            MessageKind::Start => "start",
            MessageKind::MatchPhase => "match_phase",
            MessageKind::HashReport => "hash_report",
            MessageKind::ResultAck => "result_ack",
            MessageKind::Abort => "abort",
            MessageKind::Disconnect => "disconnect",
        }
    }

    /// The reverse of [`MessageKind::wire_str`].
    #[must_use]
    pub fn from_wire_str(value: &str) -> Option<MessageKind> {
        ALL_MESSAGE_KINDS
            .iter()
            .copied()
            .find(|kind| kind.wire_str() == value)
    }

    fn allowed_body_fields(self) -> &'static [&'static str] {
        match self {
            MessageKind::Handshake => &["role", "runtime", "build_id"],
            MessageKind::ManifestProposal => &["manifest_id", "manifest"],
            MessageKind::ManifestAccept => &["manifest_id"],
            MessageKind::PeerAssignment => &["assigned_peer_id", "role"],
            MessageKind::SlotAssignment => &["manifest_id", "assignment_id", "assignments"],
            MessageKind::Ready => &["manifest_id", "assignment_id", "ready"],
            MessageKind::PairPreference => &["manifest_id", "assignment_id", "slots"],
            MessageKind::PairPreferenceResult => {
                &["manifest_id", "assignment_id", "slots", "status", "reason"]
            }
            MessageKind::Countdown => &[
                "manifest_id",
                "countdown_id",
                "remaining_ticks",
                "first_input_tick",
            ],
            MessageKind::Start => &["manifest_id", "countdown_id", "first_input_tick"],
            MessageKind::MatchPhase => &["phase", "tick", "home_score", "away_score"],
            MessageKind::HashReport => &["tick", "boundary_hash"],
            MessageKind::ResultAck => &["final_tick", "home_score", "away_score", "final_hash"],
            MessageKind::Abort => &["code"],
            MessageKind::Disconnect => &["target_peer_id", "code"],
        }
    }

    fn allowed_phases(self) -> &'static [LifecyclePhase] {
        use LifecyclePhase::{
            Assigned, Countdown, Handshake, Manifest, New, Ready, Result as ResultPhase, Running,
        };
        match self {
            MessageKind::Handshake => &[New, Handshake],
            MessageKind::ManifestProposal => &[Handshake, Manifest],
            MessageKind::ManifestAccept => &[Manifest],
            MessageKind::PeerAssignment => &[Manifest, Assigned],
            MessageKind::SlotAssignment => &[Manifest, Assigned],
            MessageKind::Ready => &[Assigned, Ready],
            MessageKind::PairPreference => &[Assigned, Ready, Countdown],
            MessageKind::PairPreferenceResult => &[Assigned, Ready, Countdown],
            MessageKind::Countdown => &[Ready, Countdown],
            MessageKind::Start => &[Countdown],
            MessageKind::MatchPhase => &[Running, ResultPhase],
            MessageKind::HashReport => &[Running],
            MessageKind::ResultAck => &[ResultPhase],
            MessageKind::Abort | MessageKind::Disconnect => &[
                New,
                Handshake,
                Manifest,
                Assigned,
                Ready,
                Countdown,
                Running,
                ResultPhase,
            ],
        }
    }
}

/// `SessionLifecyclePhase`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecyclePhase {
    /// The session exists but nothing has been sent.
    New,
    /// The handshake is in flight.
    Handshake,
    /// A manifest is proposed or being negotiated.
    Manifest,
    /// Slots are assigned; ownership may still change.
    Assigned,
    /// Every peer has declared ready.
    Ready,
    /// The countdown to `first_input_tick` is running.
    Countdown,
    /// The match is running.
    Running,
    /// The match has ended; results are being exchanged.
    Result,
    /// The session is over.
    Terminal,
}

const MATCH_PHASE_KICKOFF: &str = "kickoff";
const MATCH_PHASE_PLAYING: &str = "playing";
const MATCH_PHASE_GOAL_STOPPAGE: &str = "goal_stoppage";
const MATCH_PHASE_FULL_TIME: &str = "full_time";
const MATCH_PHASE_RESULT: &str = "result";

fn match_phase_allowed_for_lifecycle(lifecycle: LifecyclePhase, phase: &str) -> bool {
    match lifecycle {
        LifecyclePhase::Running => matches!(
            phase,
            MATCH_PHASE_KICKOFF
                | MATCH_PHASE_PLAYING
                | MATCH_PHASE_GOAL_STOPPAGE
                | MATCH_PHASE_FULL_TIME
        ),
        LifecyclePhase::Result => phase == MATCH_PHASE_RESULT,
        _ => false,
    }
}

const ROLES: &[&str] = &["host", "guest"];
const MATCH_PHASES: &[&str] = &[
    MATCH_PHASE_KICKOFF,
    MATCH_PHASE_PLAYING,
    MATCH_PHASE_GOAL_STOPPAGE,
    MATCH_PHASE_FULL_TIME,
    MATCH_PHASE_RESULT,
];
const REJECT_CODES: &[&str] = &[
    "protocol_mismatch",
    "runtime_mismatch",
    "manifest_mismatch",
    "capacity",
    "invalid_assignment",
    "invalid_phase",
    "malformed_message",
    "unsupported_message",
    "host_abort",
    "peer_disconnect",
    "desync",
];
const DISCONNECT_CODES: &[&str] = &["peer_left", "transport_lost", "host_left", "protocol_error"];

/// Every typed refusal a pair preference can end on, wire-borne or local
/// (`protocol.PREFERENCE_REJECTIONS`).
pub const PREFERENCE_REJECTIONS: &[&str] = &[
    "already_taken",
    "wrong_team",
    "invalid_slot",
    "detached",
    "not_seated",
    "superseded",
    "after_freeze",
    "no_response",
    "reseated",
];
/// The refusals no host ever sends, because neither is a verdict on a
/// request (`protocol.LOCAL_PREFERENCE_REJECTIONS`).
pub const LOCAL_PREFERENCE_REJECTIONS: &[&str] = &["no_response", "reseated"];
/// `protocol.PREFERENCE_STATUSES`.
pub const PREFERENCE_STATUSES: &[&str] = &["granted", "unchanged", "rejected"];

fn wire_preference_rejections() -> Vec<&'static str> {
    PREFERENCE_REJECTIONS
        .iter()
        .copied()
        .filter(|reason| !LOCAL_PREFERENCE_REJECTIONS.contains(reason))
        .collect()
}

// ---------------------------------------------------------------------------
// Vocabulary digest
// ---------------------------------------------------------------------------

/// One entry of a [`Vocabulary`]: a message kind name mapped to a set of
/// field or phase names (order-independent; sorted before digesting).
pub type Vocabulary = Vec<(String, Vec<String>)>;

fn sorted_unique(values: &[String]) -> Vec<String> {
    let mut sorted: Vec<String> = values.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted
}

fn vocabulary_sorted_kinds(vocabulary: &Vocabulary) -> Vec<String> {
    let mut kinds: Vec<String> = vocabulary.iter().map(|(kind, _)| kind.clone()).collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

fn vocabulary_lookup<'a>(vocabulary: &'a Vocabulary, kind: &str) -> &'a [String] {
    vocabulary
        .iter()
        .find(|(name, _)| name == kind)
        .map_or(&[], |(_, fields)| fields.as_slice())
}

/// Digests everything a peer has to agree with before a control message is
/// acceptable — which kinds exist, which fields each body carries, and
/// which lifecycle phases each kind is legal in. Sorted throughout, so it
/// never depends on construction order.
///
/// # Panics
///
/// Panics if `kinds` and `phases` do not name exactly the same set of kinds
/// (AGENTS.md §7: a programmer error, not an external-input failure).
#[must_use]
pub fn vocabulary_digest(kinds: &Vocabulary, phases: &Vocabulary) -> String {
    let ordered = vocabulary_sorted_kinds(kinds);
    let ordered_phases = vocabulary_sorted_kinds(phases);
    assert_eq!(
        ordered, ordered_phases,
        "every message kind needs exactly one phase rule"
    );
    let mut parts = format!("GCPV;1;{VERSION};");
    for kind in &ordered {
        let fields = sorted_unique(vocabulary_lookup(kinds, kind)).join(",");
        let kind_phases = sorted_unique(vocabulary_lookup(phases, kind)).join(",");
        parts.push_str(&format!("{kind}:{fields}@{kind_phases};"));
    }
    fnv1a64::hash(parts.as_bytes())
}

fn this_build_body_fields_vocabulary() -> Vocabulary {
    ALL_MESSAGE_KINDS
        .iter()
        .map(|kind| {
            (
                kind.wire_str().to_string(),
                kind.allowed_body_fields()
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            )
        })
        .collect()
}

fn phase_wire_str(phase: LifecyclePhase) -> &'static str {
    match phase {
        LifecyclePhase::New => "new",
        LifecyclePhase::Handshake => "handshake",
        LifecyclePhase::Manifest => "manifest",
        LifecyclePhase::Assigned => "assigned",
        LifecyclePhase::Ready => "ready",
        LifecyclePhase::Countdown => "countdown",
        LifecyclePhase::Running => "running",
        LifecyclePhase::Result => "result",
        LifecyclePhase::Terminal => "terminal",
    }
}

fn this_build_allowed_phases_vocabulary() -> Vocabulary {
    ALL_MESSAGE_KINDS
        .iter()
        .map(|kind| {
            (
                kind.wire_str().to_string(),
                kind.allowed_phases()
                    .iter()
                    .map(|phase| phase_wire_str(*phase).to_string())
                    .collect(),
            )
        })
        .collect()
}

/// This build's own vocabulary, digested once at load from the same closed
/// data [`MessageKind::allowed_body_fields`] / [`MessageKind::allowed_phases`]
/// read, so it cannot drift from the vocabulary it claims to describe.
/// `match_manifest` folds this into `build_id`.
#[must_use]
pub fn vocabulary_id() -> String {
    vocabulary_digest(
        &this_build_body_fields_vocabulary(),
        &this_build_allowed_phases_vocabulary(),
    )
}

// ---------------------------------------------------------------------------
// Control message
// ---------------------------------------------------------------------------

const MESSAGE_FIELDS: &[&str] = &[
    "version",
    "kind",
    "session_id",
    "peer_id",
    "sequence",
    "message_id",
    "body",
];

/// `SessionControlMessage`. `body` stays a generic [`Value`] — see the module
/// doc comment for why.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlMessage {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Which control message this is.
    pub kind: MessageKind,
    /// The session this message belongs to.
    pub session_id: String,
    /// The peer that authored this message.
    pub peer_id: String,
    /// Monotonic per-peer message sequence.
    pub sequence: i64,
    /// [`message_id`] of `(session_id, peer_id, sequence)`.
    pub message_id: String,
    /// The kind-specific payload.
    pub body: Value,
}

impl ControlMessage {
    /// The canonical [`Value`] this message encodes to. `pub` so tests can
    /// build raw, deliberately non-canonical wires from a mutated copy
    /// (ARCHITECTURE.md §3 rule 6: "everything a test touches is pub").
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::record(vec![
            ("version", Value::int(self.version)),
            ("kind", Value::str(self.kind.wire_str())),
            ("session_id", Value::str(self.session_id.clone())),
            ("peer_id", Value::str(self.peer_id.clone())),
            ("sequence", Value::int(self.sequence)),
            ("message_id", Value::str(self.message_id.clone())),
            ("body", self.body.clone()),
        ])
    }

    fn from_value(value: &Value) -> std::result::Result<ControlMessage, String> {
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .and_then(MessageKind::from_wire_str)
            .ok_or_else(|| "control message kind is missing or unknown".to_string())?;
        Ok(ControlMessage {
            version: value.get("version").and_then(Value::as_int).unwrap_or(0),
            kind,
            session_id: value
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            peer_id: value
                .get("peer_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            sequence: value.get("sequence").and_then(Value::as_int).unwrap_or(0),
            message_id: value
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            body: value.get("body").cloned().unwrap_or(Value::Nil),
        })
    }
}

fn validate_value(message: &Value) -> Result<()> {
    if !has_only_fields(message, MESSAGE_FIELDS) {
        return failure(
            ErrorCode::Malformed,
            "control message must contain only canonical fields",
        );
    }
    let version = message.get("version").unwrap_or(&Value::Nil);
    if version.as_int().is_none() {
        return failure(
            ErrorCode::Malformed,
            "control message version must be an integer",
        );
    }
    if version.as_int() != Some(VERSION) {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported control protocol version",
        );
    }
    let kind_str = message.get("kind").and_then(Value::as_str);
    let kind = match kind_str.and_then(MessageKind::from_wire_str) {
        Some(kind) => kind,
        None => return failure(ErrorCode::UnknownMessage, "unknown control message kind"),
    };
    let session_id = message.get("session_id").unwrap_or(&Value::Nil);
    validate_id(session_id, "message.session_id", MAX_SESSION_ID_BYTES)?;
    let peer_id = message.get("peer_id").unwrap_or(&Value::Nil);
    validate_id(peer_id, "message.peer_id", MAX_PEER_ID_BYTES)?;
    let sequence = message.get("sequence").unwrap_or(&Value::Nil);
    validate_integer(sequence, "message.sequence", 0, MAX_SEQUENCE)?;
    let message_id_value = message.get("message_id").unwrap_or(&Value::Nil);
    let message_id_str = message_id_value.as_str();
    let message_id_ok = message_id_str.is_some_and(|value| value.len() <= MAX_MESSAGE_ID_BYTES);
    if !message_id_ok {
        return failure(
            ErrorCode::Malformed,
            "message id must be a bounded canonical transcript id",
        );
    }
    let expected_message_id = message_id(
        session_id.as_str().unwrap(),
        peer_id.as_str().unwrap(),
        sequence.as_int().unwrap(),
    )?;
    if message_id_str != Some(expected_message_id.as_str()) {
        return failure(
            ErrorCode::Malformed,
            "message id does not match its transcript identity",
        );
    }
    let body = message.get("body").unwrap_or(&Value::Nil);
    if !has_only_fields(body, kind.allowed_body_fields()) {
        return failure(
            ErrorCode::Malformed,
            format!("{} body contains unknown fields", kind.wire_str()),
        );
    }
    validate_body(kind, body)?;
    if kind == MessageKind::ManifestProposal {
        let manifest_session = body
            .get("manifest")
            .and_then(|m| m.get("session_id"))
            .and_then(Value::as_str);
        if manifest_session != session_id.as_str() {
            return failure(
                ErrorCode::Malformed,
                "manifest session id differs from message session",
            );
        }
    }
    Ok(())
}

fn validate_body(kind: MessageKind, body: &Value) -> Result<()> {
    match kind {
        MessageKind::Handshake => {
            let role = body.get("role").and_then(Value::as_str).unwrap_or("");
            if !ROLES.contains(&role) {
                return failure(ErrorCode::Malformed, "handshake role is invalid");
            }
            validate_build_id(body.get("build_id").unwrap_or(&Value::Nil))?;
            validate_runtime(body.get("runtime").unwrap_or(&Value::Nil))
        }
        MessageKind::ManifestProposal => {
            let manifest = body.get("manifest").unwrap_or(&Value::Nil);
            validate_manifest(manifest)?;
            let manifest_id_value = body.get("manifest_id").unwrap_or(&Value::Nil);
            validate_hash(manifest_id_value, "manifest id")?;
            if manifest_id_value.as_str() != Some(manifest_id(manifest).as_str()) {
                return failure(
                    ErrorCode::Malformed,
                    "manifest id does not match canonical manifest",
                );
            }
            Ok(())
        }
        MessageKind::ManifestAccept => validate_hash(
            body.get("manifest_id").unwrap_or(&Value::Nil),
            "manifest id",
        ),
        MessageKind::PeerAssignment => {
            let role = body.get("role").and_then(Value::as_str).unwrap_or("");
            if !ROLES.contains(&role) {
                return failure(ErrorCode::Malformed, "peer assignment role is invalid");
            }
            validate_id(
                body.get("assigned_peer_id").unwrap_or(&Value::Nil),
                "assigned peer id",
                MAX_ID_BYTES,
            )
        }
        MessageKind::SlotAssignment => {
            validate_hash(
                body.get("manifest_id").unwrap_or(&Value::Nil),
                "manifest id",
            )?;
            validate_hash(
                body.get("assignment_id").unwrap_or(&Value::Nil),
                "assignment id",
            )?;
            validate_slot_assignments(body.get("assignments").unwrap_or(&Value::Nil))
        }
        MessageKind::Ready => {
            validate_hash(
                body.get("manifest_id").unwrap_or(&Value::Nil),
                "manifest id",
            )?;
            validate_hash(
                body.get("assignment_id").unwrap_or(&Value::Nil),
                "assignment id",
            )?;
            if body.get("ready").and_then(Value::as_bool).is_none() {
                return failure(ErrorCode::Malformed, "ready state must be boolean");
            }
            Ok(())
        }
        MessageKind::PairPreference => {
            validate_hash(
                body.get("manifest_id").unwrap_or(&Value::Nil),
                "manifest id",
            )?;
            validate_hash(
                body.get("assignment_id").unwrap_or(&Value::Nil),
                "assignment id",
            )?;
            validate_slot_set(body.get("slots").unwrap_or(&Value::Nil))
        }
        MessageKind::PairPreferenceResult => {
            validate_hash(
                body.get("manifest_id").unwrap_or(&Value::Nil),
                "manifest id",
            )?;
            validate_hash(
                body.get("assignment_id").unwrap_or(&Value::Nil),
                "assignment id",
            )?;
            validate_slot_set(body.get("slots").unwrap_or(&Value::Nil))?;
            let status = body.get("status").and_then(Value::as_str).unwrap_or("");
            if !PREFERENCE_STATUSES.contains(&status) {
                return failure(ErrorCode::Malformed, "pair preference status is invalid");
            }
            let reason = body.get("reason").unwrap_or(&Value::Nil);
            if status == "rejected" {
                let reason_str = reason.as_str().unwrap_or("");
                if !wire_preference_rejections().contains(&reason_str) {
                    return failure(
                        ErrorCode::Malformed,
                        "a refused pair preference needs a typed reason",
                    );
                }
            } else if !reason.is_nil() {
                return failure(
                    ErrorCode::Malformed,
                    "only a refused pair preference carries a reason",
                );
            }
            Ok(())
        }
        MessageKind::Countdown => {
            validate_hash(
                body.get("manifest_id").unwrap_or(&Value::Nil),
                "manifest id",
            )?;
            validate_id(
                body.get("countdown_id").unwrap_or(&Value::Nil),
                "countdown id",
                MAX_ID_BYTES,
            )?;
            validate_integer(
                body.get("remaining_ticks").unwrap_or(&Value::Nil),
                "remaining countdown ticks",
                0,
                MAX_COUNTDOWN_TICKS,
            )?;
            validate_integer(
                body.get("first_input_tick").unwrap_or(&Value::Nil),
                "countdown first input tick",
                0,
                input_frame::MAX_TICK,
            )
        }
        MessageKind::Start => {
            validate_hash(
                body.get("manifest_id").unwrap_or(&Value::Nil),
                "manifest id",
            )?;
            validate_id(
                body.get("countdown_id").unwrap_or(&Value::Nil),
                "countdown id",
                MAX_ID_BYTES,
            )?;
            validate_integer(
                body.get("first_input_tick").unwrap_or(&Value::Nil),
                "start first input tick",
                0,
                input_frame::MAX_TICK,
            )
        }
        MessageKind::MatchPhase => {
            let phase = body.get("phase").and_then(Value::as_str).unwrap_or("");
            if !MATCH_PHASES.contains(&phase) {
                return failure(ErrorCode::Malformed, "match phase is invalid");
            }
            validate_integer(
                body.get("tick").unwrap_or(&Value::Nil),
                "match phase tick",
                0,
                input_frame::MAX_TICK,
            )?;
            validate_integer(
                body.get("home_score").unwrap_or(&Value::Nil),
                "match phase home_score",
                0,
                MAX_GOALS,
            )?;
            validate_integer(
                body.get("away_score").unwrap_or(&Value::Nil),
                "match phase away_score",
                0,
                MAX_GOALS,
            )
        }
        MessageKind::HashReport => {
            validate_integer(
                body.get("tick").unwrap_or(&Value::Nil),
                "hash report tick",
                0,
                input_frame::MAX_TICK,
            )?;
            validate_hash(
                body.get("boundary_hash").unwrap_or(&Value::Nil),
                "boundary hash",
            )
        }
        MessageKind::ResultAck => {
            validate_integer(
                body.get("final_tick").unwrap_or(&Value::Nil),
                "result final_tick",
                0,
                input_frame::MAX_TICK,
            )?;
            validate_integer(
                body.get("home_score").unwrap_or(&Value::Nil),
                "result home_score",
                0,
                MAX_GOALS,
            )?;
            validate_integer(
                body.get("away_score").unwrap_or(&Value::Nil),
                "result away_score",
                0,
                MAX_GOALS,
            )?;
            validate_hash(body.get("final_hash").unwrap_or(&Value::Nil), "final hash")
        }
        MessageKind::Abort => {
            let code = body.get("code").and_then(Value::as_str).unwrap_or("");
            if !REJECT_CODES.contains(&code) {
                return failure(ErrorCode::Malformed, "abort code is invalid");
            }
            Ok(())
        }
        MessageKind::Disconnect => {
            let code = body.get("code").and_then(Value::as_str).unwrap_or("");
            if !DISCONNECT_CODES.contains(&code) {
                return failure(ErrorCode::Malformed, "disconnect code is invalid");
            }
            validate_id(
                body.get("target_peer_id").unwrap_or(&Value::Nil),
                "disconnect target peer id",
                MAX_ID_BYTES,
            )
        }
    }
}

/// `protocol.validate`.
pub fn validate(message: &ControlMessage) -> Result<()> {
    validate_value(&message.to_value())
}

/// `protocol.encode`.
pub fn encode(message: &ControlMessage) -> Result<String> {
    validate(message)?;
    let wire = format!(
        "{WIRE_HEADER_PREFIX}{VERSION};{}",
        encode_value(&message.to_value(), 0)
    );
    if wire.len() > MAX_WIRE_BYTES {
        return failure(
            ErrorCode::WireTooLarge,
            "control message exceeds the canonical wire bound",
        );
    }
    Ok(wire)
}

/// `protocol.decode`.
pub fn decode(wire: &str) -> Result<ControlMessage> {
    if wire.len() > MAX_WIRE_BYTES {
        return failure(
            ErrorCode::WireTooLarge,
            "control wire exceeds the canonical wire bound",
        );
    }
    let bytes = wire.as_bytes();
    if !bytes.starts_with(WIRE_HEADER_PREFIX.as_bytes()) {
        return failure(ErrorCode::Malformed, "control wire header is malformed");
    }
    let after_prefix = &bytes[WIRE_HEADER_PREFIX.len()..];
    let semicolon = match after_prefix.iter().position(|&b| b == b';') {
        Some(index) => index,
        None => return failure(ErrorCode::Malformed, "control wire header is malformed"),
    };
    let raw_version = &after_prefix[..semicolon];
    if !is_canonical_unsigned_bytes(raw_version) {
        return failure(ErrorCode::Malformed, "control wire header is malformed");
    }
    let version = match parse_bounded_decimal(raw_version) {
        Some(version) => version,
        None => return failure(ErrorCode::Malformed, "control wire header is malformed"),
    };
    if version != VERSION {
        return failure(
            ErrorCode::UnsupportedVersion,
            "unsupported control wire version",
        );
    }
    let body_bytes = &after_prefix[semicolon + 1..];
    let mut decoder = Decoder {
        bytes: body_bytes,
        index: 0,
    };
    let decoded = match decode_value(&mut decoder, 0) {
        Ok(value) => value,
        Err(err) => return failure(ErrorCode::Malformed, err),
    };
    if decoder.index != body_bytes.len() || decoded.as_table().is_none() {
        return failure(ErrorCode::Malformed, "control wire has trailing bytes");
    }
    validate_value(&decoded)?;
    let control = ControlMessage::from_value(&decoded)
        .map_err(|err| Error::new(ErrorCode::Malformed, err))?;
    let canonical = encode(&control)?;
    if canonical != wire {
        return failure(ErrorCode::Malformed, "control wire is not canonical");
    }
    Ok(control)
}

/// `protocol.new`: builds, encodes, and decodes a message in one step, so the
/// result is always canonical.
pub fn new(
    kind: MessageKind,
    session_id: &str,
    peer_id: &str,
    sequence: i64,
    body: Value,
) -> Result<ControlMessage> {
    let id = message_id(session_id, peer_id, sequence)?;
    let message = ControlMessage {
        version: VERSION,
        kind,
        session_id: session_id.to_string(),
        peer_id: peer_id.to_string(),
        sequence,
        message_id: id,
        body,
    };
    let wire = encode(&message)?;
    decode(&wire)
}

// ---------------------------------------------------------------------------
// Phase, duplicates, transcript
// ---------------------------------------------------------------------------

/// `protocol.validate_phase`.
pub fn validate_phase(message: &ControlMessage, phase: LifecyclePhase) -> Result<()> {
    validate(message)?;
    if !message.kind.allowed_phases().contains(&phase) {
        return failure(
            ErrorCode::InvalidPhase,
            format!(
                "{} is invalid during session phase {}",
                message.kind.wire_str(),
                phase_wire_str(phase)
            ),
        );
    }
    if message.kind == MessageKind::MatchPhase {
        let body_phase = message
            .body
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !match_phase_allowed_for_lifecycle(phase, body_phase) {
            return failure(
                ErrorCode::InvalidPhase,
                format!(
                    "match phase {body_phase} is invalid during session phase {}",
                    phase_wire_str(phase)
                ),
            );
        }
    }
    Ok(())
}

/// `SessionDuplicateDisposition`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateDisposition {
    /// Byte-identical to the original: safe to drop.
    Idempotent,
    /// Not safe to accept (in practice always reported via [`Error`] instead
    /// of this variant; kept for type fidelity with `input_protocol`'s own
    /// `DuplicateDisposition`, which has the same variant).
    Reject,
}

/// Whether `incoming` is a byte-identical retransmission of `previous`, or a
/// conflicting reuse of the same message id.
pub fn classify_duplicate(
    previous: &ControlMessage,
    incoming: &ControlMessage,
) -> Result<DuplicateDisposition> {
    let previous_wire = encode(previous)?;
    let incoming_wire = encode(incoming)?;
    if previous.message_id != incoming.message_id {
        return failure(
            ErrorCode::Duplicate,
            "messages do not share a transcript identity",
        );
    }
    if previous_wire == incoming_wire {
        return Ok(DuplicateDisposition::Idempotent);
    }
    failure(
        ErrorCode::TranscriptConflict,
        "message id was reused with different canonical bytes",
    )
}

/// The deterministic identity of an ordered transcript of messages.
///
/// # Panics
///
/// Panics if a peer's sequence numbers are not strictly monotonic across
/// `messages`, or if encoding any message fails — both programmer errors per
/// AGENTS.md §7.
#[must_use]
pub fn transcript_id(messages: &[ControlMessage]) -> String {
    let mut state = fnv1a64::Fnv1a64State::new();
    state.update(b"GCOT;1;");
    let mut last_by_peer: Vec<(String, i64)> = Vec::new();
    for message in messages {
        let wire = encode(message).expect("transcript_id requires every message to encode");
        if let Some((_, previous)) = last_by_peer
            .iter()
            .find(|(peer, _)| *peer == message.peer_id)
        {
            assert!(
                message.sequence > *previous,
                "transcript sequence is not monotonic"
            );
        }
        match last_by_peer
            .iter_mut()
            .find(|(peer, _)| *peer == message.peer_id)
        {
            Some(entry) => entry.1 = message.sequence,
            None => last_by_peer.push((message.peer_id.clone(), message.sequence)),
        }
        state.update(format!("{}:{}", wire.len(), wire).as_bytes());
    }
    state.hex()
}
