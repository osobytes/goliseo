//! Port of `sim/rollback_snapshot_history.lua`.
//!
//! Bounded start-of-tick snapshot storage for rollback sessions. Boundaries
//! are indexed by the [`crate::input_frame::InputFrame`] tick they will
//! consume; the ring owns independent canonical snapshots and has no
//! simulation, transport, or presentation role.
//!
//! ## Ring index convention (README rule 5.3)
//!
//! `_entries` is a fixed-size ring buffer, purely internal indexing, and is
//! 0-based here even though the Lua original's `ring_index` returned a
//! 1-based table index (`(tick % capacity) + 1`). [`ring_index`] is the
//! single conversion point (it simply drops the `+ 1`). `tick` itself is
//! never converted anywhere in this module: it is a simulation tick used as
//! a lookup key, not a buffer position.
//!
//! ## The one `%`, and why it is safe unconverted
//!
//! `ring_index` computes `tick % capacity`. Every public entry point that
//! reaches it (`store`/`store_owned`, `status`, `lookup`, `restore`,
//! `restore_simulation`, `boundary_hash`, `first_difference`, `compare`,
//! `truncate_after`) asserts `tick` is in `0..=input_frame::MAX_TICK` before
//! calling it, and `capacity` is always `max_rollback_ticks + 1 >= 1` (see
//! [`new`]'s own assertion). Both operands are therefore always
//! non-negative, so Rust's truncating `%` agrees with Lua's floored `%`
//! exactly; no `rem_euclid` is needed here.
//!
//! ## `is_integer` — a dropped runtime check
//!
//! The Lua original's `is_integer` guards against a `tick`/`boundary_tick`
//! argument that is fractional, `NaN`, or infinite — all only possible
//! because Lua numbers are untyped. `i64` cannot hold any of those, so the
//! check is structurally redundant here (README rule 9) and is dropped;
//! only the range check (`0..=input_frame::MAX_TICK`) survives as the
//! `Malformed` condition in [`truncate_after`]. The spec's "fractional tick
//! is malformed" case has no Rust equivalent for the same reason and is
//! replaced with an out-of-range tick in the port (see
//! `tests/rollback_snapshot_history.rs`), which still exercises the
//! `Malformed` branch.

use crate::combat_snapshot::CombatMatchState;
use crate::input_frame;
use crate::match_snapshot::{self, MatchSnapshot, MatchSnapshotDifference, MatchState};
use crate::rollback_input_history;
use gc_core::fnv1a64;

/// Default rollback window, mirroring [`rollback_input_history::ROLLBACK_WINDOW_TICKS`].
pub const DEFAULT_MAX_ROLLBACK_TICKS: i64 = rollback_input_history::ROLLBACK_WINDOW_TICKS;

/// Where a queried boundary stands relative to what is retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackSnapshotLookupStatus {
    /// The exact latest retained boundary.
    Present,
    /// Retained, but older than the latest boundary.
    Retained,
    /// Inside the supported window but never stored (a ring gap).
    Missing,
    /// Older than the oldest tick this history still supports.
    OutsideWindow,
}

/// Failure reasons a [`RollbackSnapshotHistory`] operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackSnapshotHistoryErrorCode {
    /// The data violates a structural or range invariant.
    Malformed,
    /// The tick named is older than the retained window.
    OutsideWindow,
    /// The tick named is inside the window but was never stored.
    Missing,
}

/// An expected, recoverable [`RollbackSnapshotHistory`] failure (README rule
/// 5.5): the caller is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackSnapshotHistoryError {
    /// Machine-readable failure reason.
    pub code: RollbackSnapshotHistoryErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl RollbackSnapshotHistoryError {
    fn new(code: RollbackSnapshotHistoryErrorCode, message: impl Into<String>) -> Self {
        RollbackSnapshotHistoryError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RollbackSnapshotHistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RollbackSnapshotHistoryError {}

/// Result alias for fallible rollback-snapshot-history operations.
pub type Result<T> = std::result::Result<T, RollbackSnapshotHistoryError>;

fn failure<T>(code: RollbackSnapshotHistoryErrorCode, message: impl Into<String>) -> Result<T> {
    Err(RollbackSnapshotHistoryError::new(code, message))
}

/// One retained ring slot.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSnapshotEntry {
    /// The boundary tick this entry retains.
    pub tick: i64,
    /// The retained canonical snapshot.
    pub snapshot: MatchSnapshot,
    /// Exact canonical wire byte count, counted at store time.
    pub canonical_bytes: i64,
    /// Lazily materialized canonical wire string.
    pub canonical_wire: Option<String>,
    /// Lazily materialized FNV-1a-64 hash of `canonical_wire`.
    pub hash: Option<String>,
}

/// Bounded start-of-tick snapshot storage for one rollback client.
///
/// Every field is internal state; use the free functions in this module to
/// read or mutate it. Fields are `pub` (README rule: everything a test
/// touches is `pub`), but `entries` is 0-based per the module doc comment
/// even though the Lua original's equivalent table was 1-based.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSnapshotHistory {
    /// Maximum prior input ticks that remain restorable.
    pub max_rollback_ticks: i64,
    /// `max_rollback_ticks + 1`: the ring's fixed slot count.
    pub capacity: i64,
    /// The fixed-size retention ring, 0-based (see the module doc comment).
    pub entries: Vec<Option<RollbackSnapshotEntry>>,
    /// The newest boundary tick ever stored.
    pub latest_tick: Option<i64>,
    /// The oldest boundary tick this history still supports restoring.
    pub oldest_supported_tick: Option<i64>,
    /// Retained entry count.
    pub count: i64,
    /// Sum of every retained entry's `canonical_bytes`.
    pub canonical_bytes: i64,
    /// High-water mark of `count`.
    pub peak_count: i64,
    /// High-water mark of `canonical_bytes`.
    pub peak_canonical_bytes: i64,
}

/// Convert a validated, non-negative `tick` into this ring's internal
/// 0-based slot (README rule 5.3; see the module doc comment for why the
/// `%` is safe unconverted).
fn ring_index(capacity: i64, tick: i64) -> usize {
    (tick % capacity) as usize
}

fn assert_tick_bounds(tick: i64, label: &str) {
    assert!(
        (0..=input_frame::MAX_TICK).contains(&tick),
        "{label} must be a bounded non-negative integer"
    );
}

fn remove_entry(history: &mut RollbackSnapshotHistory, index: usize) {
    if let Some(entry) = history.entries[index].take() {
        history.count -= 1;
        history.canonical_bytes -= entry.canonical_bytes;
    }
}

/// Construct a fresh, empty history. `max_rollback_ticks` (default
/// [`DEFAULT_MAX_ROLLBACK_TICKS`]) is the maximum number of prior input
/// ticks that remain restorable; the ring therefore holds
/// `max_rollback_ticks + 1` boundaries (the window plus the present tick).
#[must_use]
pub fn new(max_rollback_ticks: Option<i64>) -> RollbackSnapshotHistory {
    let max_rollback_ticks = max_rollback_ticks.unwrap_or(DEFAULT_MAX_ROLLBACK_TICKS);
    assert!(
        (0..=input_frame::MAX_TICK).contains(&max_rollback_ticks),
        "maximum rollback ticks must be a bounded non-negative integer"
    );
    let capacity = max_rollback_ticks + 1;
    RollbackSnapshotHistory {
        max_rollback_ticks,
        capacity,
        entries: vec![None; capacity as usize],
        latest_tick: None,
        oldest_supported_tick: None,
        count: 0,
        canonical_bytes: 0,
        peak_count: 0,
        peak_canonical_bytes: 0,
    }
}

fn copy_snapshot(snapshot: &MatchSnapshot) -> MatchSnapshot {
    let (state, combat_state) = match_snapshot::restore(snapshot);
    match_snapshot::capture(&state, combat_state.as_ref())
}

fn copy_owned_snapshot(snapshot: &MatchSnapshot) -> MatchSnapshot {
    match_snapshot::capture_owned(&snapshot.state, snapshot.combat.as_ref())
}

/// Store or replace one canonical start-of-tick boundary. Advancing the
/// latest boundary evicts every entry older than the supported correction
/// window.
fn store_retained(
    history: &mut RollbackSnapshotHistory,
    retained: MatchSnapshot,
) -> Result<RollbackSnapshotStoreResult> {
    let tick = retained.state.input_tick;
    assert_tick_bounds(tick, "snapshot input tick");

    if let Some(supported) = history.oldest_supported_tick
        && tick < supported
    {
        return failure(
            RollbackSnapshotHistoryErrorCode::OutsideWindow,
            format!("snapshot boundary {tick} is older than retained boundary {supported}"),
        );
    }

    if history.latest_tick.is_none_or(|latest| tick > latest) {
        history.latest_tick = Some(tick);
        let next_supported = (tick - history.max_rollback_ticks).max(0);
        if history
            .oldest_supported_tick
            .is_none_or(|current| next_supported > current)
        {
            history.oldest_supported_tick = Some(next_supported);
        }
    }
    let supported = history
        .oldest_supported_tick
        .expect("just set above, or already present from an earlier store");

    let stale_indices: Vec<usize> = history
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            slot.as_ref()
                .filter(|entry| entry.tick < supported)
                .map(|_| index)
        })
        .collect();
    let mut evicted: i64 = 0;
    for index in stale_indices {
        remove_entry(history, index);
        evicted += 1;
    }

    let index = ring_index(history.capacity, tick);
    let replaced = history.entries[index]
        .as_ref()
        .is_some_and(|entry| entry.tick == tick);
    if history.entries[index].is_some() {
        remove_entry(history, index);
    }
    let canonical_bytes = match_snapshot::encoded_size_canonical(&retained) as i64;
    history.entries[index] = Some(RollbackSnapshotEntry {
        tick,
        snapshot: retained,
        canonical_bytes,
        canonical_wire: None,
        hash: None,
    });
    history.count += 1;
    history.canonical_bytes += canonical_bytes;
    history.peak_count = history.peak_count.max(history.count);
    history.peak_canonical_bytes = history.peak_canonical_bytes.max(history.canonical_bytes);
    Ok(RollbackSnapshotStoreResult {
        tick,
        replaced,
        evicted,
    })
}

/// Store a defensively-copied boundary. The caller's `snapshot` remains
/// independent of what is retained.
pub fn store(
    history: &mut RollbackSnapshotHistory,
    snapshot: &MatchSnapshot,
) -> Result<RollbackSnapshotStoreResult> {
    store_retained(history, copy_snapshot(snapshot))
}

/// Transfer a freshly captured canonical snapshot into the ring. The caller
/// must not retain and mutate `snapshot` after this call. This avoids a
/// redundant restore/capture copy on the simulation hot path while
/// [`store`] remains the ownership-safe public boundary for arbitrary
/// callers.
pub fn store_owned(
    history: &mut RollbackSnapshotHistory,
    snapshot: MatchSnapshot,
) -> Result<RollbackSnapshotStoreResult> {
    store_retained(history, snapshot)
}

fn lookup_status(history: &RollbackSnapshotHistory, tick: i64) -> RollbackSnapshotLookupStatus {
    if let Some(supported) = history.oldest_supported_tick
        && tick < supported
    {
        return RollbackSnapshotLookupStatus::OutsideWindow;
    }
    let index = ring_index(history.capacity, tick);
    match &history.entries[index] {
        Some(entry) if entry.tick == tick => {
            if history.latest_tick == Some(tick) {
                RollbackSnapshotLookupStatus::Present
            } else {
                RollbackSnapshotLookupStatus::Retained
            }
        }
        _ => RollbackSnapshotLookupStatus::Missing,
    }
}

/// This boundary's retention status.
#[must_use]
pub fn status(history: &RollbackSnapshotHistory, tick: i64) -> RollbackSnapshotLookupStatus {
    assert_tick_bounds(tick, "rollback snapshot status tick");
    lookup_status(history, tick)
}

/// Keep the named start-of-tick boundary and discard only later snapshots
/// from an obsolete predicted timeline. The historical floor never moves
/// backward: already-evicted boundaries cannot become supported merely
/// because the corrected timeline finished earlier.
pub fn truncate_after(
    history: &mut RollbackSnapshotHistory,
    boundary_tick: i64,
) -> Result<RollbackSnapshotTruncateResult> {
    if !(0..=input_frame::MAX_TICK).contains(&boundary_tick) {
        return failure(
            RollbackSnapshotHistoryErrorCode::Malformed,
            "rollback snapshot truncate tick must be a bounded non-negative integer",
        );
    }
    let status = lookup_status(history, boundary_tick);
    if status == RollbackSnapshotLookupStatus::OutsideWindow {
        let supported = history
            .oldest_supported_tick
            .expect("outside_window implies a supported floor");
        return failure(
            RollbackSnapshotHistoryErrorCode::OutsideWindow,
            format!(
                "snapshot boundary {boundary_tick} is older than retained boundary {supported}"
            ),
        );
    }
    if status == RollbackSnapshotLookupStatus::Missing {
        return failure(
            RollbackSnapshotHistoryErrorCode::Missing,
            format!("snapshot boundary {boundary_tick} is missing from retained history"),
        );
    }

    let stale_indices: Vec<usize> = history
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            slot.as_ref()
                .filter(|entry| entry.tick > boundary_tick)
                .map(|_| index)
        })
        .collect();
    let mut removed: i64 = 0;
    for index in stale_indices {
        remove_entry(history, index);
        removed += 1;
    }
    history.latest_tick = Some(boundary_tick);
    Ok(RollbackSnapshotTruncateResult {
        boundary_tick,
        removed,
        diagnostics: diagnostics(history),
    })
}

/// An owned copy of the retained boundary, plus its retention status.
#[must_use]
pub fn lookup(history: &RollbackSnapshotHistory, tick: i64) -> RollbackSnapshotLookup {
    assert_tick_bounds(tick, "rollback snapshot lookup tick");
    let status = lookup_status(history, tick);
    if status == RollbackSnapshotLookupStatus::Missing
        || status == RollbackSnapshotLookupStatus::OutsideWindow
    {
        return RollbackSnapshotLookup {
            status,
            tick,
            snapshot: None,
            canonical_bytes: None,
        };
    }
    let index = ring_index(history.capacity, tick);
    let entry = history.entries[index]
        .as_ref()
        .expect("lookup_status Present/Retained implies a stored entry");
    assert_eq!(
        entry.tick, tick,
        "rollback snapshot ring lookup is inconsistent"
    );
    RollbackSnapshotLookup {
        status,
        tick,
        snapshot: Some(copy_owned_snapshot(&entry.snapshot)),
        canonical_bytes: Some(entry.canonical_bytes),
    }
}

/// Restore an independent [`MatchState`] directly from a retained entry.
/// Callers that need a state, rather than a snapshot copy, avoid a
/// restore/capture round-trip through [`lookup`].
#[must_use]
pub fn restore(
    history: &RollbackSnapshotHistory,
    tick: i64,
) -> (Option<MatchState>, RollbackSnapshotLookupStatus) {
    assert_tick_bounds(tick, "rollback snapshot restore tick");
    let status = lookup_status(history, tick);
    if status == RollbackSnapshotLookupStatus::Missing
        || status == RollbackSnapshotLookupStatus::OutsideWindow
    {
        return (None, status);
    }
    let index = ring_index(history.capacity, tick);
    let entry = history.entries[index]
        .as_ref()
        .expect("lookup_status Present/Retained implies a stored entry");
    assert_eq!(
        entry.tick, tick,
        "rollback snapshot ring restore is inconsistent"
    );
    let (state, _combat_state) = match_snapshot::restore_owned(&entry.snapshot);
    (Some(state), status)
}

/// Restore both authoritative halves of a retained simulation boundary.
#[must_use]
pub fn restore_simulation(
    history: &RollbackSnapshotHistory,
    tick: i64,
) -> (
    Option<MatchState>,
    Option<CombatMatchState>,
    RollbackSnapshotLookupStatus,
) {
    assert_tick_bounds(tick, "rollback simulation restore tick");
    let status = lookup_status(history, tick);
    if status == RollbackSnapshotLookupStatus::Missing
        || status == RollbackSnapshotLookupStatus::OutsideWindow
    {
        return (None, None, status);
    }
    let index = ring_index(history.capacity, tick);
    let entry = history.entries[index]
        .as_ref()
        .expect("lookup_status Present/Retained implies a stored entry");
    assert_eq!(
        entry.tick, tick,
        "rollback snapshot ring restore is inconsistent"
    );
    let (state, combat_state) = match_snapshot::restore_owned(&entry.snapshot);
    (Some(state), combat_state, status)
}

/// Hashing and canonical wire materialization are diagnostic and
/// deliberately lazy. Retention counts exact bytes without allocating every
/// wire string; replacement creates a fresh unhashed entry.
pub fn boundary_hash(
    history: &mut RollbackSnapshotHistory,
    tick: i64,
) -> (Option<String>, RollbackSnapshotLookupStatus) {
    assert_tick_bounds(tick, "rollback snapshot hash tick");
    let status = lookup_status(history, tick);
    if status == RollbackSnapshotLookupStatus::Missing
        || status == RollbackSnapshotLookupStatus::OutsideWindow
    {
        return (None, status);
    }
    let index = ring_index(history.capacity, tick);
    let entry = history.entries[index]
        .as_mut()
        .expect("lookup_status Present/Retained implies a stored entry");
    if entry.hash.is_none() {
        let wire = entry
            .canonical_wire
            .clone()
            .unwrap_or_else(|| match_snapshot::encode_canonical(&entry.snapshot));
        entry.hash = Some(fnv1a64::hash(wire.as_bytes()));
        entry.canonical_wire = Some(wire);
    }
    (entry.hash.clone(), status)
}

/// Compare an owned diagnostic snapshot with a retained boundary without
/// copying the retained entry. The caller's `expected` remains independent.
#[must_use]
pub fn first_difference(
    history: &RollbackSnapshotHistory,
    tick: i64,
    expected: &MatchSnapshot,
) -> (
    Option<MatchSnapshotDifference>,
    RollbackSnapshotLookupStatus,
) {
    assert_tick_bounds(tick, "rollback snapshot difference tick");
    let status = lookup_status(history, tick);
    if status == RollbackSnapshotLookupStatus::Missing
        || status == RollbackSnapshotLookupStatus::OutsideWindow
    {
        return (None, status);
    }
    let index = ring_index(history.capacity, tick);
    let entry = history.entries[index]
        .as_ref()
        .expect("lookup_status Present/Retained implies a stored entry");
    (
        match_snapshot::first_difference_canonical(expected, &entry.snapshot),
        status,
    )
}

/// Compare retained canonical boundaries without copying them through a
/// restore/capture cycle. Hashes are materialized only for a mismatch,
/// where they become part of the diagnostic contract.
pub fn compare(
    expected: &mut RollbackSnapshotHistory,
    actual: &mut RollbackSnapshotHistory,
    tick: i64,
) -> RollbackSnapshotHistoryComparison {
    assert_tick_bounds(tick, "rollback snapshot comparison tick");
    let expected_status = lookup_status(expected, tick);
    let actual_status = lookup_status(actual, tick);
    let retained_or_present = |status: RollbackSnapshotLookupStatus| {
        matches!(
            status,
            RollbackSnapshotLookupStatus::Present | RollbackSnapshotLookupStatus::Retained
        )
    };
    if !retained_or_present(expected_status) || !retained_or_present(actual_status) {
        return RollbackSnapshotHistoryComparison {
            matched: false,
            expected_status,
            actual_status,
            expected_hash: None,
            actual_hash: None,
            first_difference: None,
        };
    }

    let first_difference = {
        let expected_index = ring_index(expected.capacity, tick);
        let actual_index = ring_index(actual.capacity, tick);
        let expected_entry = expected.entries[expected_index]
            .as_ref()
            .expect("lookup_status Present/Retained implies a stored entry");
        let actual_entry = actual.entries[actual_index]
            .as_ref()
            .expect("lookup_status Present/Retained implies a stored entry");
        match_snapshot::first_difference_canonical(&expected_entry.snapshot, &actual_entry.snapshot)
    };
    if first_difference.is_none() {
        return RollbackSnapshotHistoryComparison {
            matched: true,
            expected_status,
            actual_status,
            expected_hash: None,
            actual_hash: None,
            first_difference: None,
        };
    }

    let (expected_hash, _) = boundary_hash(expected, tick);
    let (actual_hash, _) = boundary_hash(actual, tick);
    RollbackSnapshotHistoryComparison {
        matched: false,
        expected_status,
        actual_status,
        expected_hash: Some(expected_hash.expect("boundary_hash Present/Retained is always Some")),
        actual_hash: Some(actual_hash.expect("boundary_hash Present/Retained is always Some")),
        first_difference,
    }
}

/// A snapshot of this history's retained-state shape.
#[must_use]
pub fn diagnostics(history: &RollbackSnapshotHistory) -> RollbackSnapshotHistoryDiagnostics {
    let oldest = history
        .entries
        .iter()
        .filter_map(|slot| slot.as_ref().map(|entry| entry.tick))
        .min();
    RollbackSnapshotHistoryDiagnostics {
        max_rollback_ticks: history.max_rollback_ticks,
        capacity: history.capacity,
        retained_boundary_count: history.count,
        canonical_bytes: history.canonical_bytes,
        peak_retained_boundary_count: history.peak_count,
        peak_canonical_bytes: history.peak_canonical_bytes,
        oldest_supported_tick: history.oldest_supported_tick,
        oldest_boundary_tick: oldest,
        latest_tick: history.latest_tick,
    }
}

/// An owned copy of a retained boundary, plus its retention status.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSnapshotLookup {
    /// This boundary's retention status.
    pub status: RollbackSnapshotLookupStatus,
    /// The queried tick.
    pub tick: i64,
    /// The retained snapshot, present exactly when `status` is `Present` or
    /// `Retained`.
    pub snapshot: Option<MatchSnapshot>,
    /// `snapshot`'s exact canonical wire byte count.
    pub canonical_bytes: Option<i64>,
}

/// The outcome of one [`compare`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSnapshotHistoryComparison {
    /// Whether both sides retain the same canonical boundary.
    pub matched: bool,
    /// `expected`'s retention status for this tick.
    pub expected_status: RollbackSnapshotLookupStatus,
    /// `actual`'s retention status for this tick.
    pub actual_status: RollbackSnapshotLookupStatus,
    /// `expected`'s hash, materialized only on a mismatch.
    pub expected_hash: Option<String>,
    /// `actual`'s hash, materialized only on a mismatch.
    pub actual_hash: Option<String>,
    /// The first structural disagreement, materialized only on a mismatch.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// The outcome of one [`store`] or [`store_owned`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackSnapshotStoreResult {
    /// The stored boundary's tick.
    pub tick: i64,
    /// Whether this call replaced an existing entry at the same tick.
    pub replaced: bool,
    /// Entries evicted by this call's advance of the supported window.
    pub evicted: i64,
}

/// The outcome of one [`truncate_after`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSnapshotTruncateResult {
    /// The boundary tick truncation ran from (kept).
    pub boundary_tick: i64,
    /// Entries strictly newer than `boundary_tick` that were removed.
    pub removed: i64,
    /// Diagnostics taken after truncation.
    pub diagnostics: RollbackSnapshotHistoryDiagnostics,
}

/// A snapshot of a [`RollbackSnapshotHistory`]'s retained-state shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackSnapshotHistoryDiagnostics {
    /// Maximum prior input ticks that remain restorable.
    pub max_rollback_ticks: i64,
    /// The ring's fixed slot count.
    pub capacity: i64,
    /// Retained entry count.
    pub retained_boundary_count: i64,
    /// Sum of every retained entry's canonical byte count.
    pub canonical_bytes: i64,
    /// High-water mark of `retained_boundary_count`.
    pub peak_retained_boundary_count: i64,
    /// High-water mark of `canonical_bytes`.
    pub peak_canonical_bytes: i64,
    /// The oldest boundary tick this history still supports restoring.
    pub oldest_supported_tick: Option<i64>,
    /// The oldest boundary tick currently retained (may exceed
    /// `oldest_supported_tick` after wraparound eviction, or be `None` when
    /// the ring is empty).
    pub oldest_boundary_tick: Option<i64>,
    /// The newest boundary tick ever stored.
    pub latest_tick: Option<i64>,
}
