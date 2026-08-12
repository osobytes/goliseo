//! Versioned identity for the combat-disabled gameplay-AI policy.
//!
//! #59's orchestrator refresh requires a policy id that #112/#148/#149 can cite
//! instead of copying constants or silently re-freezing them. [`id`] is that
//! citation: a canonical FNV-1a-64 hash over an explicitly DECLARED surface of
//! outfield gameplay-AI configuration, prefixed with the schema version and the
//! combat mode it was taken under.
//!
//! The surface is declared, never reflected, and that is the whole point:
//!
//!   * adding an unrelated field to an AI module does NOT move the id, so the id
//!     does not churn on refactors;
//!   * changing a declared constant DOES move it, so a policy change cannot be
//!     absorbed by a stale baseline;
//!   * renaming or deleting a declared field fails loudly instead of hashing
//!     `nil` into a plausible-looking id.
//!
//! A behavioural change that lives outside this surface (a file-local constant,
//! a rewritten heuristic) is caught by the recorded metric signature in
//! `gc_data::outfield_ai_baseline`, which compares exactly. Between the two,
//! "the policy changed" is always observable; neither check may be quieted by
//! refreshing the other. When a module changes behaviour without changing a
//! declared constant, bump that module's own `VERSION` — it is in the surface
//! precisely so a deliberate policy change always has somewhere to land.
//!
//! Pure module: no I/O.
//!
//! ## Declared surface, not reflection
//!
//! Rust has no runtime reflection over a module's items, so [`descriptor`]
//! is a hand-written list of rows in a fixed order, and [`SURFACE`] is the
//! declared checklist the test suite audits [`descriptor`] against (see
//! `tests/outfield_ai_policy.rs`) — the shape check that keeps the two in
//! sync, since nothing enforces it automatically.
//!
//! This module itself never calls into `ai`, `outfield_decision`,
//! `offball_runs`, `outfield_press`, or `possession_transition` — it only reads
//! their declared constants (`VERSION` and the named tuning fields) to hash
//! them. There is no RNG draw, no decision call, and nothing on the
//! rollback-resimulation path in this file; the AI decision logic these
//! modules implement lives elsewhere and is out of this module's scope.

use crate::ai;
use crate::match_snapshot;
use crate::offball_runs;
use crate::outfield_decision;
use crate::outfield_press;
use crate::possession_transition;
use crate::tuning;
use gc_core::fnv1a64;

/// Schema name embedded in the id.
pub const SCHEMA: &str = "outfield_ai_policy";

/// Schema version embedded in the id.
pub const SCHEMA_VERSION: i64 = 1;

/// The policy is frozen with combat off. `sim.headless` builds soccer-only
/// matches and never constructs a `CombatMatchState`, so this is a statement
/// of fact about the fixture, not a switch this module owns.
pub const COMBAT_MODE: &str = "disabled";

/// Knob category whose defaults belong to the AI policy rather than to the
/// world rules. The remaining categories are hashed separately as the tuning
/// identity of a fixture (see `outfield_ai_baseline`), so a movement tweak
/// invalidates the baseline without pretending the decision policy changed.
pub const KNOB_CATEGORY: &str = "AI";

// `offball_runs::VERSION` was missing when this module was first written —
// every sibling in this surface exports one, so it was a dropped constant
// rather than a design choice. It now exists; this alias keeps the local
// numeric type the canonical encoder wants.
const OFFBALL_RUNS_VERSION: f64 = crate::offball_runs::VERSION as f64;

/// One named module in the hashed policy surface.
///
/// Ordered; the order is part of the hashed form. Append to a group's field
/// list rather than reordering it when the surface grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutfieldAiPolicyGroup {
    /// Declared module name (matches the `descriptor()` row key prefix).
    pub module: &'static str,
    /// Declared field names, in hash order. `VERSION` always leads.
    pub fields: &'static [&'static str],
}

/// The declared policy surface. See the module doc for why this is a
/// checklist rather than the thing [`descriptor`] walks.
pub static SURFACE: &[OutfieldAiPolicyGroup] = &[
    OutfieldAiPolicyGroup {
        module: "outfield_decision",
        fields: &[
            "VERSION",
            "SLOW_REFRESH_SECONDS",
            "FAST_REFRESH_SECONDS",
            "BASE_TEMPERATURE",
            "RUN_LIFETIME_SECONDS",
        ],
    },
    OutfieldAiPolicyGroup {
        module: "outfield_press",
        fields: &["VERSION"],
    },
    OutfieldAiPolicyGroup {
        module: "offball_runs",
        fields: &[
            "VERSION",
            "RUN_LIFETIME_SECONDS",
            "TELEGRAPH_SECONDS",
            "MAX_ACTIVE_PER_TEAM",
            "RUN_DRIVE_THRESHOLD",
            "MIN_RUN_PROGRESS",
            "MIN_SUPPORT_DISTANCE",
            "MAX_SUPPORT_DISTANCE",
        ],
    },
    OutfieldAiPolicyGroup {
        module: "possession_transition",
        fields: &["VERSION", "ESTABLISH_SECONDS", "MAX_PRESSERS"],
    },
    OutfieldAiPolicyGroup {
        // `ai` supplies the off-ball support scoring `offball_runs`
        // consumes, so its weights are policy. Its intercept-sampling
        // constants stay file-local for the hot loop and are covered by
        // `VERSION`.
        module: "ai",
        fields: &[
            "VERSION",
            "IMPORTANCE_K",
            "CENTER_SIGMA",
            "LANE_WIDTH",
            "LANE_BLOCK",
        ],
    },
];

/// A policy-surface scalar: number or string.
#[derive(Clone, Debug, PartialEq)]
pub enum OutfieldAiPolicyValue {
    /// A numeric field. Numeric fields are `f64` (ARCHITECTURE.md §3 rule 1).
    Number(f64),
    /// A string field (schema name, combat mode).
    Text(String),
}

impl std::fmt::Display for OutfieldAiPolicyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutfieldAiPolicyValue::Number(n) => write!(f, "{n}"),
            OutfieldAiPolicyValue::Text(s) => write!(f, "{s}"),
        }
    }
}

/// One row of the resolved surface: a canonical key and its value.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldAiPolicyRow {
    /// `"<module>.<field>"`, or one of the three leading identity keys.
    pub key: String,
    /// The declared value.
    pub value: OutfieldAiPolicyValue,
}

fn row(key: impl Into<String>, value: OutfieldAiPolicyValue) -> OutfieldAiPolicyRow {
    OutfieldAiPolicyRow {
        key: key.into(),
        value,
    }
}

fn num(value: f64) -> OutfieldAiPolicyValue {
    OutfieldAiPolicyValue::Number(value)
}

fn text(value: impl Into<String>) -> OutfieldAiPolicyValue {
    OutfieldAiPolicyValue::Text(value.into())
}

/// Length-prefixed canonical string scalar, byte-identical to the encoding
/// `match_snapshot` uses, so an id is unambiguous under concatenation.
fn append_str(parts: &mut String, value: &str) {
    parts.push('s');
    parts.push_str(&value.len().to_string());
    parts.push(':');
    parts.push_str(value);
    parts.push(';');
}

/// Length-prefixed canonical numeric scalar; see [`append_str`].
fn append_num(parts: &mut String, value: f64) {
    parts.push('n');
    parts.push_str(&match_snapshot::number_bytes(value));
    parts.push(';');
}

fn append_value(parts: &mut String, value: &OutfieldAiPolicyValue) {
    match value {
        OutfieldAiPolicyValue::Number(n) => append_num(parts, *n),
        OutfieldAiPolicyValue::Text(s) => append_str(parts, s),
    }
}

/// The declared surface, resolved against the live modules, in hash order.
///
/// See the module doc: Rust has no runtime reflection to drive this from
/// [`SURFACE`] automatically, so the rows are assembled by hand in the same
/// order [`SURFACE`] declares.
#[must_use]
pub fn descriptor() -> Vec<OutfieldAiPolicyRow> {
    let mut rows = vec![
        row("schema", text(SCHEMA)),
        row("schema_version", num(SCHEMA_VERSION as f64)),
        row("combat", text(COMBAT_MODE)),
        row(
            "outfield_decision.VERSION",
            num(f64::from(outfield_decision::VERSION)),
        ),
        row(
            "outfield_decision.SLOW_REFRESH_SECONDS",
            num(outfield_decision::SLOW_REFRESH_SECONDS),
        ),
        row(
            "outfield_decision.FAST_REFRESH_SECONDS",
            num(outfield_decision::FAST_REFRESH_SECONDS),
        ),
        row(
            "outfield_decision.BASE_TEMPERATURE",
            num(outfield_decision::BASE_TEMPERATURE),
        ),
        row(
            "outfield_decision.RUN_LIFETIME_SECONDS",
            num(outfield_decision::RUN_LIFETIME_SECONDS),
        ),
        row(
            "outfield_press.VERSION",
            num(f64::from(outfield_press::VERSION)),
        ),
        row("offball_runs.VERSION", num(OFFBALL_RUNS_VERSION)),
        row(
            "offball_runs.RUN_LIFETIME_SECONDS",
            num(offball_runs::RUN_LIFETIME_SECONDS),
        ),
        row(
            "offball_runs.TELEGRAPH_SECONDS",
            num(offball_runs::TELEGRAPH_SECONDS),
        ),
        row(
            "offball_runs.MAX_ACTIVE_PER_TEAM",
            num(f64::from(offball_runs::MAX_ACTIVE_PER_TEAM)),
        ),
        row(
            "offball_runs.RUN_DRIVE_THRESHOLD",
            num(offball_runs::RUN_DRIVE_THRESHOLD),
        ),
        row(
            "offball_runs.MIN_RUN_PROGRESS",
            num(offball_runs::MIN_RUN_PROGRESS),
        ),
        row(
            "offball_runs.MIN_SUPPORT_DISTANCE",
            num(offball_runs::MIN_SUPPORT_DISTANCE),
        ),
        row(
            "offball_runs.MAX_SUPPORT_DISTANCE",
            num(offball_runs::MAX_SUPPORT_DISTANCE),
        ),
        row(
            "possession_transition.VERSION",
            num(f64::from(possession_transition::VERSION)),
        ),
        row(
            "possession_transition.ESTABLISH_SECONDS",
            num(possession_transition::ESTABLISH_SECONDS),
        ),
        row(
            "possession_transition.MAX_PRESSERS",
            num(f64::from(possession_transition::MAX_PRESSERS)),
        ),
        row("ai.VERSION", num(ai::VERSION as f64)),
        row("ai.IMPORTANCE_K", num(ai::IMPORTANCE_K)),
        row("ai.CENTER_SIGMA", num(ai::CENTER_SIGMA)),
        row("ai.LANE_WIDTH", num(ai::LANE_WIDTH)),
        row("ai.LANE_BLOCK", num(ai::LANE_BLOCK)),
    ];
    // The DEFAULT, not a live value: the policy is the shipped balance, so
    // an in-session tuning-panel nudge is not a new policy. `tuning::KNOBS`
    // is the static registry of defaults (see `tuning.rs`'s module doc for
    // why live values are kept in an owned `Tuning`, separate from this
    // static list); `descriptor` never takes a `Tuning` at all, so a
    // live nudge structurally cannot reach this function.
    for knob in tuning::KNOBS.iter() {
        if knob.cat == KNOB_CATEGORY {
            rows.push(row(format!("tuning.{}", knob.key), num(knob.default)));
        }
    }
    rows
}

/// Canonical bytes behind the id. Exposed so a mismatch can be diffed.
#[must_use]
pub fn canonical() -> String {
    canonical_of(&descriptor())
}

/// Canonical bytes for an arbitrary declared surface.
///
/// `canonical()` is this applied to [`descriptor()`]. It is separate because
/// the property that needs proving is "a different declared constant moves
/// the identity" — and Rust constants are not assignable at runtime, so that
/// can't be tested by mutating a live module field the way a dynamic
/// language could. The property under test is really "a different declared
/// surface hashes differently", and that is testable directly by perturbing
/// a row here rather than by mutating the module the row was read from.
///
/// This is a stronger test than reassigning a field would be, not a weaker
/// substitute: it also covers a row being *added* or *removed*, which
/// runtime field assignment cannot express at all.
#[must_use]
pub fn canonical_of(rows: &[OutfieldAiPolicyRow]) -> String {
    let mut parts = String::new();
    parts.push_str("GCOAP;");
    parts.push_str(&SCHEMA_VERSION.to_string());
    parts.push(';');
    for r in rows {
        append_str(&mut parts, &r.key);
        append_value(&mut parts, &r.value);
    }
    parts
}

/// The citable identity, e.g.
/// `outfield_ai_policy/v1/combat_disabled/0123456789abcdef`.
#[must_use]
pub fn id() -> String {
    id_of(&descriptor())
}

/// The citable identity for an arbitrary declared surface. See [`canonical_of`].
#[must_use]
pub fn id_of(rows: &[OutfieldAiPolicyRow]) -> String {
    format!(
        "{SCHEMA}/v{SCHEMA_VERSION}/combat_{COMBAT_MODE}/{}",
        fnv1a64::hash(canonical_of(rows).as_bytes())
    )
}

/// Human-readable dump of the surface behind an id.
#[must_use]
pub fn report() -> String {
    let mut lines = vec![format!("policy {}", id())];
    for r in descriptor() {
        lines.push(format!("  {:<44} {}", r.key, r.value));
    }
    lines.join("\n")
}
