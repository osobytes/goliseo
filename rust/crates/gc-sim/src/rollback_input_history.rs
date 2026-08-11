//! Pure per-tick input history for rollback clients. It records
//! transport-facing authoritative arrivals and materializes the complete
//! [`InputFrame`] consumed by the match simulation without knowing about
//! sockets, wall clocks, or presentation.
//!
//! This is the input ring buffer the rollback re-simulation replays from: if
//! it returns a different input for a tick on two clients, the match
//! desyncs. See `tools/lua_reference/README.md` and
//! `tests/fixtures/rollback_input_history_lua_reference.txt` for the
//! differential coverage this module gets because of that.
//!
//! ## Slot index convention (ARCHITECTURE.md §3 rule 3)
//!
//! `slot_index` on this module's public surface stays **1-based**, matching
//! [`input_frame::slot`]'s own public convention — this is the same
//! cross-module "canonical input slot" identity, not a wire payload, but
//! converting one sibling module's copy of it and not the other is exactly
//! the asymmetric off-by-one rule 3 warns about. Every internal
//! fixed-size, 8-element buffer this module keeps (`sources`,
//! `authoritative_ticks`, `anchors`, and each tick's
//! `[Option<InputSample>; 8]`) is purely internal indexing and is 0-based;
//! [`slot_idx`] is the single conversion point.
//!
//! `tick` values are never converted: they are already 0-based simulation
//! ticks used as sparse keys, not sequential buffer positions.
//!
//! ## No `%`, and why the two `/` are still safe
//!
//! This module contains no `%` (floored modulo) anywhere. The only integer
//! division is inside the two binary-search helpers ([`predecessor_index`],
//! [`insert_tick`]), and in both, the loop invariant keeps `low` and `high`
//! non-negative for the entire time a division executes (`low` starts at
//! `0` and only increases; `high` starts at `length - 1`, and the loop
//! body — where the division happens — only runs while `low <= high`, which
//! already implies `high >= low >= 0`). Since both operands are always
//! non-negative, Rust's truncating `/` matches floor division exactly for
//! every division this module performs; no `rem_euclid` is needed anywhere
//! in this file.
//!
//! ## No manual deep-copy helpers
//!
//! [`InputSample`], [`InputFrame`], [`RollbackInputSlotRecord`], and
//! [`RollbackInputTickRecord`] are all `Copy` value types here, so every
//! read already hands back an independent copy — no manual deep-copy helper
//! is needed anywhere in this module, because the bug class such a helper
//! would defend against (a caller mutating a table it was merely handed)
//! does not exist for `Copy` types.

use crate::input_frame::{self, InputFrame, InputSample};
use indexmap::IndexMap;

/// Ticks the rollback window retains before pruning.
pub const ROLLBACK_WINDOW_TICKS: i64 = 30;
/// [`ROLLBACK_WINDOW_TICKS`] expressed in milliseconds at
/// [`crate::fixed_clock::TICK_RATE`].
pub const ROLLBACK_WINDOW_MILLISECONDS: f64 =
    ROLLBACK_WINDOW_TICKS as f64 * 1000.0 / crate::fixed_clock::TICK_RATE;

/// Who owns one canonical input slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackInputSource {
    /// This client's own slot.
    Local,
    /// A remote peer's slot.
    Remote,
}

/// Whether a materialized sample is confirmed authority or a prediction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackInputStatus {
    /// Backed by a retained authoritative arrival at this exact tick.
    Authoritative,
    /// Repeats the latest known authority (or neutral); not yet confirmed.
    Predicted,
}

/// Failure reasons a [`RollbackInputHistory`] operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackInputHistoryErrorCode {
    /// The data violates a structural or range invariant.
    Malformed,
    /// A new authoritative sample conflicts with one already retained.
    ConflictingAuthoritative,
    /// The tick named is older than the retained window.
    OutsideWindow,
    /// Pruning would discard a correction the caller has not consumed yet.
    PendingDivergence,
}

/// An expected, recoverable [`RollbackInputHistory`] failure (ARCHITECTURE.md
/// §3 rule 5): the caller is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackInputHistoryError {
    /// Machine-readable failure reason.
    pub code: RollbackInputHistoryErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl RollbackInputHistoryError {
    fn new(code: RollbackInputHistoryErrorCode, message: impl Into<String>) -> Self {
        RollbackInputHistoryError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RollbackInputHistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RollbackInputHistoryError {}

/// Result alias for fallible rollback-input-history operations.
pub type Result<T> = std::result::Result<T, RollbackInputHistoryError>;

fn failure<T>(code: RollbackInputHistoryErrorCode, message: impl Into<String>) -> Result<T> {
    Err(RollbackInputHistoryError::new(code, message))
}

/// One slot's materialized sample for a tick, with its source and status.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackInputSlotRecord {
    /// Whether this slot is locally or remotely owned.
    pub source: RollbackInputSource,
    /// Whether `sample` is confirmed authority or a prediction.
    pub status: RollbackInputStatus,
    /// The materialized sample.
    pub sample: InputSample,
}

/// One tick's complete materialized record across every canonical slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackInputTickRecord {
    /// The tick this record was materialized for.
    pub tick: i64,
    /// One record per canonical slot, in canonical slot order.
    pub slots: [RollbackInputSlotRecord; 8],
}

/// The outcome of one accepted (or duplicate) [`add_authoritative`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackAuthoritativeArrival {
    /// `true` only when this exact authoritative sample already existed.
    pub duplicate: bool,
    /// Highest contiguous all-authoritative tick, or `-1` before tick zero.
    pub confirmed_tick: i64,
    /// Earliest unconsumed corrected tick used by the simulation.
    pub earliest_divergence: Option<i64>,
}

/// One row of an [`add_authoritative_batch`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackAuthoritativeInput {
    /// The tick this sample arrived for.
    pub tick: i64,
    /// The 1-based canonical slot this sample arrived for.
    pub slot_index: i64,
    /// The arrived sample.
    pub sample: InputSample,
}

/// The outcome of one [`add_authoritative_batch`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackAuthoritativeBatchArrival {
    /// Rows newly retained by this call.
    pub inserted: i64,
    /// Rows that already existed, identically, before this call.
    pub duplicates: i64,
    /// The rows newly retained by this call, in canonical order.
    pub inserted_rows: Vec<RollbackAuthoritativeInput>,
    /// Highest contiguous all-authoritative tick, or `-1` before tick zero.
    pub confirmed_tick: i64,
    /// Earliest unconsumed corrected tick used by the simulation.
    pub earliest_divergence: Option<i64>,
}

/// One slot's last known authoritative sample before a pruned tick range,
/// kept so prediction can still repeat it after the evidence itself is gone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackInputAnchor {
    /// The anchor sample's originating tick.
    pub tick: i64,
    /// The anchor sample.
    pub sample: InputSample,
}

/// A snapshot of a [`RollbackInputHistory`]'s retained-state shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackInputHistoryDiagnostics {
    /// Oldest tick this history still retains evidence for.
    pub oldest_retained_tick: i64,
    /// Newest tick this history has evidence or a materialized record for.
    pub newest_retained_tick: Option<i64>,
    /// Distinct ticks with at least one retained authoritative sample.
    pub authoritative_tick_count: i64,
    /// Total retained authoritative samples across every tick and slot.
    pub authoritative_sample_count: i64,
    /// Distinct ticks with a materialized effective frame.
    pub effective_tick_count: i64,
    /// Distinct ticks with a materialized tick record.
    pub record_tick_count: i64,
    /// Slots carrying a prediction anchor from before the retained window.
    pub predecessor_anchor_count: i64,
    /// Highest contiguous all-authoritative tick, or `-1` before tick zero.
    pub confirmed_tick: i64,
    /// Earliest unconsumed corrected tick used by the simulation.
    pub earliest_divergence: Option<i64>,
}

/// Exact byte count for the documented canonical retained-history encoding.
/// This accounts for logical rollback payload only; it is deliberately not
/// an estimate of allocator overhead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackInputHistoryAccounting {
    /// Bytes for every retained authoritative sample.
    pub authoritative_bytes: i64,
    /// Bytes for every retained prediction anchor.
    pub predecessor_anchor_bytes: i64,
    /// Bytes for every retained materialized effective frame.
    pub effective_frame_bytes: i64,
    /// Bytes for every retained materialized tick record.
    pub input_record_bytes: i64,
    /// Sum of the four byte counts above.
    pub total_bytes: i64,
}

/// The outcome of one [`truncate_from`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackInputTruncateResult {
    /// The boundary tick truncation ran from.
    pub boundary_tick: i64,
    /// Materialized effective frames removed.
    pub effective_removed: i64,
    /// Materialized tick records removed.
    pub records_removed: i64,
    /// Whether an unconsumed divergence at or after `boundary_tick` was
    /// cleared by this call.
    pub cleared_divergence: bool,
    /// Diagnostics taken after truncation.
    pub diagnostics: RollbackInputHistoryDiagnostics,
}

/// Pure per-tick input history for one rollback client.
///
/// Every field is internal state; use the free functions in this module to
/// read or mutate it. Fields are `pub` (ARCHITECTURE.md §3 rule 6: everything
/// a test touches is `pub`; sibling `gc-sim` state structs — [`crate::bot::BotState`],
/// [`crate::outfield_decision::OutfieldDecisionState`] — are fully open data
/// too), but the arrays here are 0-based per the module doc comment.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackInputHistory {
    /// Canonical eight-slot local/remote ownership metadata.
    pub sources: [RollbackInputSource; 8],
    /// Retained authoritative samples, keyed by tick.
    pub authoritative: IndexMap<i64, [Option<InputSample>; 8]>,
    /// Sorted authoritative ticks for each slot (0-based slot index).
    pub authoritative_ticks: [Vec<i64>; 8],
    /// One prediction anchor per slot, surviving a pruned tick range.
    pub anchors: [Option<RollbackInputAnchor>; 8],
    /// Materialized effective frames, keyed by tick.
    pub effective: IndexMap<i64, InputFrame>,
    /// Materialized tick records, keyed by tick.
    pub records: IndexMap<i64, RollbackInputTickRecord>,
    /// Oldest tick this history still retains evidence for.
    pub oldest_retained_tick: i64,
    /// Distinct ticks with at least one retained authoritative sample.
    pub authoritative_tick_count: i64,
    /// Total retained authoritative samples across every tick and slot.
    pub authoritative_sample_count: i64,
    /// Distinct ticks with a materialized effective frame.
    pub effective_tick_count: i64,
    /// Distinct ticks with a materialized tick record.
    pub record_tick_count: i64,
    /// Highest contiguous all-authoritative tick, or `-1` before tick zero.
    pub confirmed_tick: i64,
    /// Earliest unconsumed corrected tick used by the simulation.
    pub earliest_divergence: Option<i64>,
}

/// Convert a public 1-based canonical slot index into this module's internal
/// 0-based buffer index (ARCHITECTURE.md §3 rule 3). Callers must have already
/// validated `slot_index` is in `1..=input_frame::SLOT_COUNT`.
fn slot_idx(slot_index: i64) -> usize {
    (slot_index - 1) as usize
}

fn filled_count(slots: &[Option<InputSample>; 8]) -> i64 {
    slots.iter().filter(|sample| sample.is_some()).count() as i64
}

/// Index of the greatest element `<= target`, or `None` if every element is
/// greater than `target` (or `ticks` is empty). Purely internal buffer
/// indexing (ARCHITECTURE.md §3 rule 3): 0-based, returning `None` for "not found"
/// rather than a sentinel index.
fn predecessor_index(ticks: &[i64], target: i64) -> Option<usize> {
    let mut low: isize = 0;
    let mut high: isize = ticks.len() as isize - 1;
    let mut found: Option<usize> = None;
    while low <= high {
        // `low` and `high` are both non-negative whenever this runs (see the
        // module doc comment), so this truncating division is exactly
        // `floor((low + high) / 2)`.
        let middle = (low + high) / 2;
        if ticks[middle as usize] <= target {
            found = Some(middle as usize);
            low = middle + 1;
        } else {
            high = middle - 1;
        }
    }
    found
}

/// Insert `tick` into the sorted `ticks` buffer, preserving order.
fn insert_tick(ticks: &mut Vec<i64>, tick: i64) {
    let mut low: usize = 0;
    let mut high: usize = ticks.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if ticks[middle] < tick {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    ticks.insert(low, tick);
}

fn latest_authoritative(
    history: &RollbackInputHistory,
    tick: i64,
    index: usize,
) -> Option<InputSample> {
    let ticks = &history.authoritative_ticks[index];
    if let Some(predecessor) = predecessor_index(ticks, tick) {
        let found_tick = ticks[predecessor];
        return Some(
            history
                .authoritative
                .get(&found_tick)
                .and_then(|slots| slots[index])
                .expect("rollback authoritative index is inconsistent"),
        );
    }
    if let Some(anchor) = history.anchors[index]
        && anchor.tick <= tick
    {
        return Some(anchor.sample);
    }
    None
}

/// Construct a fresh history for one match. `sources` names every canonical
/// slot's local/remote ownership; a `RollbackInputSource` array is always
/// exactly eight entries, so no runtime shape/membership assertion is
/// needed here — the same principle `input_frame` documents for its own
/// dropped shape checks.
#[must_use]
pub fn new(sources: [RollbackInputSource; 8]) -> RollbackInputHistory {
    RollbackInputHistory {
        sources,
        authoritative: IndexMap::new(),
        authoritative_ticks: std::array::from_fn(|_| Vec::new()),
        anchors: [None; 8],
        effective: IndexMap::new(),
        records: IndexMap::new(),
        oldest_retained_tick: 0,
        authoritative_tick_count: 0,
        authoritative_sample_count: 0,
        effective_tick_count: 0,
        record_tick_count: 0,
        confirmed_tick: -1,
        earliest_divergence: None,
    }
}

/// This slot's local/remote ownership. `slot_index` is 1-based (ARCHITECTURE.md
/// §3 rule 3). Panics (programmer error, ARCHITECTURE.md §3 rule 5) if `slot_index` is out
/// of `1..=8`.
#[must_use]
pub fn source(history: &RollbackInputHistory, slot_index: i64) -> RollbackInputSource {
    assert!(
        (1..=input_frame::SLOT_COUNT).contains(&slot_index),
        "rollback input slot must be between one and eight"
    );
    history.sources[slot_idx(slot_index)]
}

fn validate_arrival(tick: i64, slot_index: i64, sample: &InputSample) -> Result<()> {
    if !(0..=input_frame::MAX_TICK).contains(&tick) {
        return failure(
            RollbackInputHistoryErrorCode::Malformed,
            "authoritative input tick must be a bounded non-negative integer",
        );
    }
    if !(1..=input_frame::SLOT_COUNT).contains(&slot_index) {
        return failure(
            RollbackInputHistoryErrorCode::Malformed,
            "authoritative input slot must be between one and eight",
        );
    }
    if let Err(err) = input_frame::validate_sample(sample) {
        return failure(RollbackInputHistoryErrorCode::Malformed, err.message);
    }
    Ok(())
}

fn advance_confirmation(history: &mut RollbackInputHistory) {
    let mut next_tick = history.confirmed_tick + 1;
    while history
        .authoritative
        .get(&next_tick)
        .is_some_and(|slots| filled_count(slots) == input_frame::SLOT_COUNT)
    {
        history.confirmed_tick = next_tick;
        next_tick += 1;
    }
}

/// Store one local or remote authoritative sample. Transport-shaped
/// validation failures, including conflicting duplicates, are recoverable
/// and leave the history unchanged.
pub fn add_authoritative(
    history: &mut RollbackInputHistory,
    tick: i64,
    slot_index: i64,
    sample: InputSample,
) -> Result<RollbackAuthoritativeArrival> {
    validate_arrival(tick, slot_index, &sample)?;
    if tick < history.oldest_retained_tick {
        return failure(
            RollbackInputHistoryErrorCode::OutsideWindow,
            format!(
                "authoritative input tick {tick} is older than retained tick {}",
                history.oldest_retained_tick
            ),
        );
    }

    let index = slot_idx(slot_index);
    let existing = history
        .authoritative
        .get(&tick)
        .and_then(|slots| slots[index]);
    if let Some(existing) = existing {
        if existing != sample {
            return failure(
                RollbackInputHistoryErrorCode::ConflictingAuthoritative,
                format!("authoritative input conflicts at tick {tick} slot {slot_index}"),
            );
        }
        return Ok(RollbackAuthoritativeArrival {
            duplicate: true,
            confirmed_tick: history.confirmed_tick,
            earliest_divergence: history.earliest_divergence,
        });
    }

    if let Some(used_frame) = history.effective.get(&tick)
        && used_frame.slots[index] != sample
        && history
            .earliest_divergence
            .is_none_or(|divergence| tick < divergence)
    {
        history.earliest_divergence = Some(tick);
    }

    if !history.authoritative.contains_key(&tick) {
        history.authoritative.insert(tick, [None; 8]);
        history.authoritative_tick_count += 1;
    }
    let slots = history
        .authoritative
        .get_mut(&tick)
        .expect("just inserted or already present");
    slots[index] = Some(sample);
    insert_tick(&mut history.authoritative_ticks[index], tick);
    history.authoritative_sample_count += 1;
    advance_confirmation(history);

    Ok(RollbackAuthoritativeArrival {
        duplicate: false,
        confirmed_tick: history.confirmed_tick,
        earliest_divergence: history.earliest_divergence,
    })
}

/// Validate a complete transport-tick batch before retaining any row. This
/// prevents a late conflict or malformed row from leaving a partially
/// applied authority set whose result depends on peer polling order.
pub fn add_authoritative_batch(
    history: &mut RollbackInputHistory,
    arrivals: &[RollbackAuthoritativeInput],
) -> Result<RollbackAuthoritativeBatchArrival> {
    if arrivals.is_empty() {
        return failure(
            RollbackInputHistoryErrorCode::Malformed,
            "authoritative input batch must be a non-empty canonical array",
        );
    }

    let mut planned: IndexMap<(i64, i64), RollbackAuthoritativeInput> = IndexMap::new();
    let mut planned_rows: Vec<RollbackAuthoritativeInput> = Vec::new();
    let mut duplicates: i64 = 0;
    let mut previous: Option<(i64, i64)> = None;
    for arrival in arrivals {
        validate_arrival(arrival.tick, arrival.slot_index, &arrival.sample)?;
        if let Some((previous_tick, previous_slot)) = previous
            && (arrival.tick < previous_tick
                || (arrival.tick == previous_tick && arrival.slot_index < previous_slot))
        {
            return failure(
                RollbackInputHistoryErrorCode::Malformed,
                "authoritative input batch is not in canonical tick-slot order",
            );
        }
        previous = Some((arrival.tick, arrival.slot_index));
        if arrival.tick < history.oldest_retained_tick {
            return failure(
                RollbackInputHistoryErrorCode::OutsideWindow,
                format!(
                    "authoritative input tick {} is older than retained tick {}",
                    arrival.tick, history.oldest_retained_tick
                ),
            );
        }

        let key = (arrival.tick, arrival.slot_index);
        if let Some(prior) = planned.get(&key) {
            if prior.sample != arrival.sample {
                return failure(
                    RollbackInputHistoryErrorCode::ConflictingAuthoritative,
                    format!(
                        "authoritative input conflicts at tick {} slot {}",
                        arrival.tick, arrival.slot_index
                    ),
                );
            }
            duplicates += 1;
            continue;
        }

        let index = slot_idx(arrival.slot_index);
        let existing = history
            .authoritative
            .get(&arrival.tick)
            .and_then(|slots| slots[index]);
        if let Some(existing) = existing {
            if existing != arrival.sample {
                return failure(
                    RollbackInputHistoryErrorCode::ConflictingAuthoritative,
                    format!(
                        "authoritative input conflicts at tick {} slot {}",
                        arrival.tick, arrival.slot_index
                    ),
                );
            }
            duplicates += 1;
        } else {
            planned.insert(key, *arrival);
            planned_rows.push(*arrival);
        }
    }

    let mut inserted_rows = Vec::with_capacity(planned_rows.len());
    for arrival in &planned_rows {
        let result = add_authoritative(history, arrival.tick, arrival.slot_index, arrival.sample)
            .expect("batch preflight already validated every planned row");
        assert!(
            !result.duplicate,
            "rollback batch preflight produced a duplicate insertion"
        );
        inserted_rows.push(*arrival);
    }
    Ok(RollbackAuthoritativeBatchArrival {
        inserted: inserted_rows.len() as i64,
        duplicates,
        inserted_rows,
        confirmed_tick: history.confirmed_tick,
        earliest_divergence: history.earliest_divergence,
    })
}

/// Materialize the exact frame the simulation is about to use. Calling this
/// tick again during resimulation replaces its former used-input record with
/// a frame derived from the now-current authoritative history.
///
/// Every `assert!` below is a producer invariant (ARCHITECTURE.md §3 rule 5), not a
/// recoverable failure: a missing local sample, a tick older than the
/// retained window, or an unconsumed divergence are all programmer/protocol
/// errors, so this never returns a `Result`.
pub fn materialize(
    history: &mut RollbackInputHistory,
    tick: i64,
) -> (InputFrame, RollbackInputTickRecord) {
    assert!(
        (0..=input_frame::MAX_TICK).contains(&tick),
        "rollback input tick must be a bounded non-negative integer"
    );
    assert!(
        tick >= history.oldest_retained_tick,
        "rollback input tick is older than retained history"
    );
    assert!(
        history
            .earliest_divergence
            .is_none_or(|divergence| tick < divergence),
        "consume the earliest divergence before materializing the corrected timeline"
    );

    let authoritative = history.authoritative.get(&tick).copied();
    let mut samples = [InputSample::default(); 8];
    let mut records = [RollbackInputSlotRecord {
        source: RollbackInputSource::Remote,
        status: RollbackInputStatus::Predicted,
        sample: InputSample::default(),
    }; 8];
    for index in 0..8usize {
        let mut sample = authoritative.and_then(|slots| slots[index]);
        assert!(
            sample.is_some() || history.sources[index] == RollbackInputSource::Remote,
            "local rollback input must be authoritative before materialization"
        );
        let status = if sample.is_some() {
            RollbackInputStatus::Authoritative
        } else {
            let prior = latest_authoritative(history, tick, index);
            sample = Some(match prior {
                None => input_frame::neutral_sample(),
                Some(prior) => InputSample {
                    move_x: prior.move_x,
                    move_y: prior.move_y,
                    held: prior.held,
                    edges: 0,
                },
            });
            RollbackInputStatus::Predicted
        };
        let sample = sample.expect("sample is always set above");
        samples[index] = sample;
        records[index] = RollbackInputSlotRecord {
            source: history.sources[index],
            status,
            sample,
        };
    }

    let frame =
        input_frame::new(tick, Some(samples)).expect("materialized samples are always valid");
    let record = RollbackInputTickRecord {
        tick,
        slots: records,
    };
    if !history.effective.contains_key(&tick) {
        history.effective_tick_count += 1;
    }
    if !history.records.contains_key(&tick) {
        history.record_tick_count += 1;
    }
    history.effective.insert(tick, frame);
    history.records.insert(tick, record);
    (frame, record)
}

/// The tick record materialized for `tick`, if any.
#[must_use]
pub fn record(history: &RollbackInputHistory, tick: i64) -> Option<RollbackInputTickRecord> {
    history.records.get(&tick).copied()
}

/// The authoritative slot record for `(tick, slot_index)`, if any authority
/// is retained for it. `slot_index` is 1-based (ARCHITECTURE.md §3 rule 3); an
/// out-of-range `slot_index` simply reports no record, rather than
/// panicking — this accessor asserts no bound.
#[must_use]
pub fn authoritative_record(
    history: &RollbackInputHistory,
    tick: i64,
    slot_index: i64,
) -> Option<RollbackInputSlotRecord> {
    if !(1..=input_frame::SLOT_COUNT).contains(&slot_index) {
        return None;
    }
    let index = slot_idx(slot_index);
    let sample = history
        .authoritative
        .get(&tick)
        .and_then(|slots| slots[index])?;
    Some(RollbackInputSlotRecord {
        source: source(history, slot_index),
        status: RollbackInputStatus::Authoritative,
        sample,
    })
}

/// Highest contiguous all-authoritative tick, or `-1` before tick zero.
#[must_use]
pub fn confirmed_tick(history: &RollbackInputHistory) -> i64 {
    history.confirmed_tick
}

/// Earliest unconsumed corrected tick used by the simulation, if any.
#[must_use]
pub fn earliest_divergence(history: &RollbackInputHistory) -> Option<i64> {
    history.earliest_divergence
}

/// Consume the current correction batch boundary. Later differing arrivals
/// can then establish a new earliest divergence after the caller has
/// resimulated.
pub fn consume_earliest_divergence(history: &mut RollbackInputHistory) -> Option<i64> {
    history.earliest_divergence.take()
}

/// A snapshot of this history's retained-state shape.
#[must_use]
pub fn diagnostics(history: &RollbackInputHistory) -> RollbackInputHistoryDiagnostics {
    let mut newest: Option<i64> = None;
    for ticks in &history.authoritative_ticks {
        if let Some(&tick) = ticks.last()
            && newest.is_none_or(|current| tick > current)
        {
            newest = Some(tick);
        }
    }
    for &tick in history.effective.keys() {
        if newest.is_none_or(|current| tick > current) {
            newest = Some(tick);
        }
    }
    let anchor_count = history
        .anchors
        .iter()
        .filter(|anchor| anchor.is_some())
        .count() as i64;
    RollbackInputHistoryDiagnostics {
        oldest_retained_tick: history.oldest_retained_tick,
        newest_retained_tick: newest,
        authoritative_tick_count: history.authoritative_tick_count,
        authoritative_sample_count: history.authoritative_sample_count,
        effective_tick_count: history.effective_tick_count,
        record_tick_count: history.record_tick_count,
        predecessor_anchor_count: anchor_count,
        confirmed_tick: history.confirmed_tick,
        earliest_divergence: history.earliest_divergence,
    }
}

fn source_label(source: RollbackInputSource) -> &'static str {
    match source {
        RollbackInputSource::Local => "local",
        RollbackInputSource::Remote => "remote",
    }
}

fn status_label(status: RollbackInputStatus) -> &'static str {
    match status {
        RollbackInputStatus::Authoritative => "authoritative",
        RollbackInputStatus::Predicted => "predicted",
    }
}

fn sample_wire(sample: InputSample) -> String {
    format!(
        "{},{},{},{}",
        sample.move_x, sample.move_y, sample.held, sample.edges
    )
}

/// Exact byte count for the documented canonical retained-history encoding.
#[must_use]
pub fn accounting(history: &RollbackInputHistory) -> RollbackInputHistoryAccounting {
    let mut authoritative_bytes: i64 = 0;
    for (&tick, slots) in &history.authoritative {
        for (index, sample) in slots.iter().enumerate() {
            if let Some(sample) = sample {
                let slot = index as i64 + 1;
                authoritative_bytes +=
                    format!("A|{tick}|{slot}|{}\n", sample_wire(*sample)).len() as i64;
            }
        }
    }
    let mut predecessor_anchor_bytes: i64 = 0;
    for (index, anchor) in history.anchors.iter().enumerate() {
        if let Some(anchor) = anchor {
            let slot = index as i64 + 1;
            predecessor_anchor_bytes +=
                format!("P|{slot}|{}|{}\n", anchor.tick, sample_wire(anchor.sample)).len() as i64;
        }
    }
    let mut effective_frame_bytes: i64 = 0;
    for frame in history.effective.values() {
        let encoded = input_frame::encode(frame).expect("materialized frames are always encodable");
        effective_frame_bytes += format!("E|{encoded}\n").len() as i64;
    }
    let mut input_record_bytes: i64 = 0;
    for (&tick, record) in &history.records {
        let mut parts: Vec<String> = vec!["R".to_string(), tick.to_string()];
        for row in &record.slots {
            parts.push(source_label(row.source).to_string());
            parts.push(status_label(row.status).to_string());
            parts.push(sample_wire(row.sample));
        }
        input_record_bytes += format!("{}\n", parts.join("|")).len() as i64;
    }
    RollbackInputHistoryAccounting {
        authoritative_bytes,
        predecessor_anchor_bytes,
        effective_frame_bytes,
        input_record_bytes,
        total_bytes: authoritative_bytes
            + predecessor_anchor_bytes
            + effective_frame_bytes
            + input_record_bytes,
    }
}

/// Boundary `tick` is the state before `InputFrame` `tick`. Discarding from
/// `tick` removes only effective frames and source/status diagnostics from
/// an obsolete simulated tail; authoritative arrivals and confirmation
/// remain valid upstream facts.
pub fn truncate_from(
    history: &mut RollbackInputHistory,
    boundary_tick: i64,
) -> Result<RollbackInputTruncateResult> {
    if !(0..=input_frame::MAX_TICK).contains(&boundary_tick) {
        return failure(
            RollbackInputHistoryErrorCode::Malformed,
            "rollback input truncate tick must be a bounded non-negative integer",
        );
    }
    if boundary_tick < history.oldest_retained_tick {
        return failure(
            RollbackInputHistoryErrorCode::OutsideWindow,
            format!(
                "rollback input boundary {boundary_tick} is older than retained tick {}",
                history.oldest_retained_tick
            ),
        );
    }

    let stale_effective: Vec<i64> = history
        .effective
        .keys()
        .copied()
        .filter(|&tick| tick >= boundary_tick)
        .collect();
    for tick in &stale_effective {
        history.effective.swap_remove(tick);
    }
    history.effective_tick_count -= stale_effective.len() as i64;

    let stale_records: Vec<i64> = history
        .records
        .keys()
        .copied()
        .filter(|&tick| tick >= boundary_tick)
        .collect();
    for tick in &stale_records {
        history.records.swap_remove(tick);
    }
    history.record_tick_count -= stale_records.len() as i64;

    let cleared_divergence = history
        .earliest_divergence
        .is_some_and(|divergence| divergence >= boundary_tick);
    if cleared_divergence {
        history.earliest_divergence = None;
    }
    Ok(RollbackInputTruncateResult {
        boundary_tick,
        effective_removed: stale_effective.len() as i64,
        records_removed: stale_records.len() as i64,
        cleared_divergence,
        diagnostics: diagnostics(history),
    })
}

/// Drop input/effective records that no retained snapshot can restore. One
/// copied authoritative predecessor per slot survives as the prediction
/// anchor. A pending correction must be handed to the rollback session
/// before its evidence can leave the retained window.
pub fn prune_before(
    history: &mut RollbackInputHistory,
    oldest_retained_tick: i64,
) -> Result<RollbackInputHistoryDiagnostics> {
    assert!(
        oldest_retained_tick >= history.oldest_retained_tick
            && oldest_retained_tick <= input_frame::MAX_TICK,
        "rollback input prune tick must advance within the bounded tick range"
    );
    if let Some(divergence) = history.earliest_divergence
        && divergence < oldest_retained_tick
    {
        return failure(
            RollbackInputHistoryErrorCode::PendingDivergence,
            format!(
                "cannot prune unconsumed divergence at tick {divergence} before retained tick {oldest_retained_tick}"
            ),
        );
    }
    if oldest_retained_tick == history.oldest_retained_tick {
        return Ok(diagnostics(history));
    }

    for index in 0..8usize {
        let predecessor = predecessor_index(
            &history.authoritative_ticks[index],
            oldest_retained_tick - 1,
        );
        if let Some(predecessor) = predecessor {
            let tick = history.authoritative_ticks[index][predecessor];
            let sample = history
                .authoritative
                .get(&tick)
                .and_then(|slots| slots[index])
                .expect("rollback prune index is inconsistent");
            history.anchors[index] = Some(RollbackInputAnchor { tick, sample });
        }
        let start = predecessor.map_or(0, |index| index + 1);
        history.authoritative_ticks[index] = history.authoritative_ticks[index][start..].to_vec();
    }

    let stale_authoritative: Vec<i64> = history
        .authoritative
        .keys()
        .copied()
        .filter(|&tick| tick < oldest_retained_tick)
        .collect();
    for tick in &stale_authoritative {
        let slots = history
            .authoritative
            .get(tick)
            .expect("tick was just collected from history.authoritative");
        let sample_count = filled_count(slots);
        assert!(sample_count > 0, "rollback authoritative tick is empty");
        history.authoritative.swap_remove(tick);
        history.authoritative_tick_count -= 1;
        history.authoritative_sample_count -= sample_count;
    }

    let stale_effective: Vec<i64> = history
        .effective
        .keys()
        .copied()
        .filter(|&tick| tick < oldest_retained_tick)
        .collect();
    for tick in &stale_effective {
        history.effective.swap_remove(tick);
        history.effective_tick_count -= 1;
    }

    let stale_records: Vec<i64> = history
        .records
        .keys()
        .copied()
        .filter(|&tick| tick < oldest_retained_tick)
        .collect();
    for tick in &stale_records {
        history.records.swap_remove(tick);
        history.record_tick_count -= 1;
    }

    history.oldest_retained_tick = oldest_retained_tick;
    Ok(diagnostics(history))
}
