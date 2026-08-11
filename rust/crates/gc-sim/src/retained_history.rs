//! One canonical reading of everything a rollback client retains: the
//! session's own accounting, the speculative event timeline's accounting,
//! their combined total, and the occupancy counters that say whether that
//! total is measuring anything.
//!
//! ## Why the combined total, and why it lives here
//!
//! `history_bytes` — the quantity
//! [`crate::rollback_lab`]'s peaks report, `tests/combat_load_fixtures.rs`
//! compares to `gc_data::omp2_rollback_validation::Omp2RollbackBudgets::history_bytes`,
//! and `tests/snapshot_headroom.rs` bands against that same budget — has
//! always meant [`crate::rollback_session::accounting`]'s `total_bytes`
//! (input + output + snapshots) **plus** [`crate::rollback_events::accounting`]'s
//! `total_bytes` (the retained speculative event window). The session
//! figure alone structurally omits the event timeline, and that timeline is
//! not a lab-only artifact: `gc-wasm`'s `rollback_events_bridge` and
//! `match_driver_bridge` both retain one per live browser peer, so it is
//! real retained client memory. A reading that reported only the session
//! half would let retained event encoding — a new `CombatEvent`/`MatchEvent`
//! field, a wider unconfirmed window — grow with nothing observing it.
//!
//! Before this module the definition was open-coded at each call site.
//! [`sample`] is now the one place it is written down, so a fourth consumer
//! (the wasm bridge that lets a browser peer sample this during a long run)
//! cannot quietly adopt a narrower one.
//!
//! ## Why occupancy counters travel with the bytes
//!
//! A byte total is trivially satisfiable by a measurement that has stopped
//! measuring, and — less obviously — by one that is measuring only
//! scaffolding. Thirty retained speculative steps holding *no events at all*
//! still report a plausible, nonzero, budget-comparable byte count in which
//! none of the per-event encoders were ever entered; a field added to
//! `MatchEvent` would move that number by exactly nothing. So every sample
//! carries [`RetainedHistorySample::retained_event_count`],
//! [`RetainedHistorySample::retained_step_count`] and
//! [`RetainedHistorySample::retained_boundary_count`] alongside the bytes.
//! A consumer that gates on the bytes is expected to reject a sample whose
//! counts say the window is empty, rather than pass on the bytes alone.
//!
//! ## Cost
//!
//! [`sample`] is O(retained input rows + retained outputs + retained
//! events); the snapshot component is a counter the ring maintains
//! incrementally, so no snapshot is re-encoded. It allocates short-lived
//! strings while re-deriving the input history's canonical encoding
//! ([`crate::rollback_input_history::accounting`]), which dominates. On a
//! full 31-boundary ring with a full 30-step speculative window this has
//! been measured at 0.27 ms and 0.38 ms per call in native release builds
//! on two different machines (`tests/retained_history.rs` prints whatever
//! the machine running it manages). Those are observations, not a bound:
//! treat them as the order of magnitude — sub-millisecond, so cheap enough
//! to sample once per second or even once per tick for hours — and not as a
//! budget anything should be asserted against. It is still not something to
//! call inside a per-entity loop.

use crate::rollback_events::{self, RollbackEventTimeline, RollbackEventsAccounting};
use crate::rollback_session::{self, RollbackSession, RollbackSessionAccounting};
use crate::rollback_snapshot_history;

/// One reading of a rollback client's retained storage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetainedHistorySample {
    /// The session's own retained accounting: input history, retained
    /// outputs, and the canonical snapshot ring.
    pub session: RollbackSessionAccounting,
    /// The retained speculative event timeline's accounting.
    pub events: RollbackEventsAccounting,
    /// `session.total_bytes + events.total_bytes` — the combined figure
    /// `Omp2RollbackBudgets::history_bytes` bounds. See the module doc.
    pub history_bytes: i64,
    /// Boundaries currently retained in the snapshot ring.
    pub retained_boundary_count: i64,
    /// High-water mark of `retained_boundary_count`, tracked by the ring
    /// itself — so it reports growth that happened *between* two samples.
    pub peak_retained_boundary_count: i64,
    /// High-water mark of the snapshot ring's canonical bytes, same
    /// between-samples property as `peak_retained_boundary_count`.
    pub peak_snapshot_bytes: i64,
    /// The oldest retained boundary tick, or `None` when the ring is empty.
    pub oldest_boundary_tick: Option<i64>,
    /// The newest boundary tick ever stored, or `None` when the ring is
    /// empty.
    pub latest_boundary_tick: Option<i64>,
    /// Retained speculative steps in the event timeline.
    pub retained_step_count: i64,
    /// Retained *events* across those steps — the field that separates a
    /// window holding real content from one holding empty step wrappers.
    /// See the module doc.
    pub retained_event_count: i64,
}

/// Read `session` and `timeline`'s combined retained storage.
///
/// `session` is taken by `&mut` because
/// [`crate::rollback_session::accounting`] is: it re-derives the retained
/// output accounting through the session's own counted-output cache.
/// Nothing observable about the session changes.
#[must_use]
pub fn sample(
    session: &mut RollbackSession,
    timeline: &RollbackEventTimeline,
) -> RetainedHistorySample {
    let snapshots = rollback_snapshot_history::diagnostics(&session.snapshot_history);
    let session_accounting = rollback_session::accounting(session);
    let events_accounting = rollback_events::accounting(timeline);
    let event_diagnostics = rollback_events::diagnostics(timeline);
    RetainedHistorySample {
        session: session_accounting,
        events: events_accounting,
        history_bytes: session_accounting.total_bytes + events_accounting.total_bytes,
        retained_boundary_count: snapshots.retained_boundary_count,
        peak_retained_boundary_count: snapshots.peak_retained_boundary_count,
        peak_snapshot_bytes: snapshots.peak_canonical_bytes,
        oldest_boundary_tick: snapshots.oldest_boundary_tick,
        latest_boundary_tick: snapshots.latest_tick,
        retained_step_count: event_diagnostics.retained_step_count,
        retained_event_count: event_diagnostics.retained_event_count,
    }
}
