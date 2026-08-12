//! The rollback state machine: it owns the live [`MatchState`]/
//! [`CombatMatchState`] pair and composes [`crate::rollback_input_history`]
//! and [`crate::rollback_snapshot_history`] into deterministic predict,
//! store, and (on a late or corrected input) rewind-and-resimulate.
//!
//! ## No manual deep-copy helpers
//!
//! [`RollbackTickOutput`], [`RollbackComparison`], [`RollbackReconcileResult`],
//! [`RollbackSessionLastRollback`], and [`crate::match_snapshot::MatchSnapshotDifference`]
//! are all owned Rust values with real [`Clone`] impls (and
//! `MatchSnapshotDifference.expected`/`.actual` are already rendered
//! `String`s, not nested structures — see `crate::match_snapshot`), so a
//! `.clone()` at a storage or return boundary already gives an independent
//! copy. No manual deep-copy helper is needed anywhere in this module,
//! matching the precedent in `rollback_input_history` and
//! `rollback_events`.
//!
//! ## The measurement hook is not truly reentrant here
//!
//! `measured` needs to invoke the session's measurement hook at every
//! nesting level (`step`'s `"tick"` wraps `execute_tick`'s own `"capture"`;
//! `reconcile`'s `"rollback"` wraps `"capture"`, `"restore"`, and
//! `"resimulation"`, and the last of those itself wraps `execute_tick`'s
//! `"capture"` again). A `Box<dyn FnMut>` cannot be called again while a
//! call to it is already on the stack without `unsafe`, so [`measured`]
//! temporarily removes the hook from the session for the duration of its
//! own invocation (`Option::take`), restoring it before returning. A
//! `measured` call nested inside another therefore runs unobserved rather
//! than through the hook a second time.
//!
//! This is a deliberate, safe simplification, not an oversight: no test
//! assertion distinguishes "the hook fired for `capture` nested inside
//! `tick`" from "`capture` ran unmeasured because `tick`'s own hook call was
//! still on the stack" — every case here only checks the *outermost*
//! invocation's call count and the real operation's result. If a future
//! caller needs true nested measurement (for example, billing `capture`
//! time separately from `tick` time inside `resimulation`), that is the
//! seam to revisit, most likely with a small `unsafe` accessor documented
//! at that single call site rather than a general reentrant hook.
//!
//! ## `detailed_diagnostics` is a plain `bool`, not `bool?`
//!
//! `reconcile(session, detailed_diagnostics)` takes a plain `bool` rather
//! than an optional one: `false` is already its natural default, and every
//! call site already passes a literal, so an `Option` wrapper would add
//! nothing.
//!
//! ## Retained-output byte accounting is a diagnostic proxy, not an exact encoding
//!
//! `accounting` reports each retained output's byte count as a diagnostic —
//! not on the determinism path (see `tools/lua_reference/README.md`) — and
//! every test assertion here only checks that two independently-computed
//! totals agree (an incremental cache versus a full recompute), never a
//! specific byte count against a reference encoding. [`output_len`]
//! therefore reuses Rust's derived `Debug` rendering as a stable,
//! deterministic size proxy instead of reimplementing the
//! `n`/`b0`/`b1`/`s<len>:`/`d<len>:`/`t<count>:` tagging scheme
//! `rollback_events::accounting`'s doc comment describes a third time
//! (`rollback_events` and `rollback_input_history` each already carry their
//! own private copy for their own shapes); no observer depends on the exact
//! number matching any other encoding.
//!
//! ## Dropped runtime shape check
//!
//! There is no runtime check guarding against a caller passing something
//! other than a well-formed `RollbackSession` — structurally redundant
//! once `session: &RollbackSession` is enforced by the type system
//! (ARCHITECTURE.md §3 rule 7).

use crate::combat_snapshot::{CombatEvent, CombatMatchState};
use crate::fixed_clock;
use crate::input_frame::InputSample;
use crate::r#match::{self, StepInput};
use crate::match_snapshot::{
    self, ByTeam, MatchEvent, MatchSnapshot, MatchSnapshotDifference, MatchState,
};
use crate::rollback_input_history::{
    self, RollbackAuthoritativeInput, RollbackInputHistory, RollbackInputHistoryError,
    RollbackInputSlotRecord, RollbackInputSource, RollbackInputStatus, RollbackInputTickRecord,
};
use crate::rollback_snapshot_history::{
    self, RollbackSnapshotHistory, RollbackSnapshotLookupStatus,
};
use crate::tuning::Tuning;
use indexmap::IndexMap;

/// A [`RollbackSession`]'s lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackSessionStatus {
    /// Accepting further `step`/`reconcile` calls.
    Active,
    /// The live match has reached full time.
    Finished,
    /// Permanently stalled: a correction arrived older than the retained
    /// window.
    LateInputUnrecoverable,
}

/// Failure reasons a [`step`] call can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackSessionErrorCode {
    /// The live match has already reached full time.
    MatchFinished,
    /// The session is permanently stalled after an over-window correction.
    LateInputUnrecoverable,
}

/// An expected, recoverable [`step`] failure (ARCHITECTURE.md §3 rule 5): the caller
/// is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackSessionError {
    /// Machine-readable failure reason.
    pub code: RollbackSessionErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl RollbackSessionError {
    fn new(code: RollbackSessionErrorCode, message: impl Into<String>) -> Self {
        RollbackSessionError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RollbackSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RollbackSessionError {}

/// Result alias for fallible [`step`] calls.
pub type Result<T> = std::result::Result<T, RollbackSessionError>;

/// Which measured operation is running, passed to a [`RollbackSessionMeasure`]
/// hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackSessionMeasureLabel {
    /// One forward-predicted tick (`step`).
    Tick,
    /// One [`crate::match_snapshot::capture_owned`] call.
    Capture,
    /// One rollback restore from a retained boundary.
    Restore,
    /// One rewind-and-resimulate pass.
    Resimulation,
    /// One complete `reconcile` call.
    Rollback,
}

/// A pluggable, purely observational wall-time hook. It must invoke the
/// `operation` thunk it is handed exactly once; see the module doc comment
/// on why nested `measured` calls run unobserved instead of reentering this
/// hook a second time.
pub type RollbackSessionMeasure = Box<dyn FnMut(RollbackSessionMeasureLabel, &mut dyn FnMut())>;

/// The compact state view carried by [`RollbackTickOutput`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackOutputStateView {
    /// Score at this boundary.
    pub score: ByTeam<i64>,
    /// Seconds remaining at this boundary.
    pub time_left: f64,
    /// Whether the match had finished at this boundary.
    pub finished: bool,
}

/// One tick's complete, retained simulation output.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackTickOutput {
    /// Causal input tick.
    pub tick: i64,
    /// Always equal to `tick`.
    pub start_boundary: i64,
    /// Always `tick + 1`.
    pub end_boundary: i64,
    /// The materialized input record this tick simulated with.
    pub input: RollbackInputTickRecord,
    /// This tick's soccer events.
    pub events: Vec<MatchEvent>,
    /// This tick's combat events, present exactly when combat is active.
    pub combat_events: Option<Vec<CombatEvent>>,
    /// Compact post-step state.
    pub state: RollbackOutputStateView,
    /// Whether the match finished on this tick.
    pub finished: bool,
}

/// The outcome of one accepted (or duplicate) [`add_authoritative`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackSessionArrival {
    /// `true` only when this exact authoritative sample already existed.
    pub duplicate: bool,
    /// Highest contiguous all-authoritative tick, or `-1` before tick zero.
    pub confirmed_tick: i64,
    /// Earliest unconsumed corrected tick used by the simulation.
    pub earliest_divergence: Option<i64>,
    /// Whether the accepted authority differs from a previously consumed
    /// prediction (a true correction, not merely the first authority for a
    /// still-unsimulated tick).
    pub correction: bool,
}

/// The outcome of one [`add_authoritative_batch`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSessionBatchArrival {
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
    /// Newly inserted rows differing from samples already consumed.
    pub corrections: i64,
}

/// The outcome of one [`apply_authoritative_batch`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSessionBatchApplyResult {
    /// The batch retention outcome.
    pub arrival: RollbackSessionBatchArrival,
    /// The reconciliation this batch triggered.
    pub reconciliation: RollbackReconcileResult,
}

/// The outcome of one [`compare`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackComparison {
    /// Whether `actual` and `expected` hash identically.
    pub matched: bool,
    /// Whether `actual` and `expected` disagree on their boundary tick.
    pub boundary_mismatch: bool,
    /// The session's actual boundary tick.
    pub actual_boundary: i64,
    /// The expected snapshot's boundary tick.
    pub expected_boundary: i64,
    /// The session's actual canonical hash.
    pub actual_hash: String,
    /// The expected snapshot's canonical hash.
    pub expected_hash: String,
    /// The causal tick this comparison is attributed to, if any.
    pub causal_tick: Option<i64>,
    /// The first structural disagreement, present exactly on a mismatch.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// The outcome of one [`reconcile`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackReconcileResult {
    /// Whether this call actually rewound and resimulated.
    pub changed: bool,
    /// The session's status after this call.
    pub status: RollbackSessionStatus,
    /// The earliest divergent tick this call reconciled, if any.
    pub causal_tick: Option<i64>,
    /// The boundary restored from, present exactly when `changed`.
    pub restored_boundary: Option<i64>,
    /// The restored boundary's retention status, present exactly when
    /// `causal_tick` is present.
    pub restore_status: Option<RollbackSnapshotLookupStatus>,
    /// The present boundary before this call.
    pub old_present_boundary: i64,
    /// The present boundary after this call.
    pub new_present_boundary: i64,
    /// The first tick whose output was resimulated, present exactly when
    /// at least one tick was resimulated.
    pub corrected_from_tick: Option<i64>,
    /// The last tick whose output was resimulated, present exactly when
    /// at least one tick was resimulated.
    pub corrected_through_tick: Option<i64>,
    /// The first tick whose retained output is no longer valid, present
    /// exactly when `changed`.
    pub replaced_from_tick: Option<i64>,
    /// The last tick whose retained output is no longer valid, present
    /// exactly when the old present boundary was strictly after
    /// `causal_tick`.
    pub replaced_through_tick: Option<i64>,
    /// Every resimulated tick's fresh output, in causal order.
    pub corrected_outputs: Vec<RollbackTickOutput>,
    /// The predicted boundary's hash before restoring, present only when
    /// `detailed_diagnostics` was requested and this call changed state.
    pub old_present_hash: Option<String>,
    /// The corrected boundary's hash after resimulating, present only when
    /// `detailed_diagnostics` was requested and this call changed state.
    pub new_present_hash: Option<String>,
    /// The first structural disagreement between the old and new present
    /// boundary, present only when both hashes were computed and differ.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// The most recent [`reconcile`] call that actually changed state.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSessionLastRollback {
    /// The earliest divergent tick this rollback reconciled.
    pub causal_tick: i64,
    /// The boundary restored from.
    pub restored_boundary: i64,
    /// The present boundary before this rollback.
    pub old_present_boundary: i64,
    /// The present boundary after this rollback.
    pub new_present_boundary: i64,
    /// The predicted boundary's hash before restoring, present only when
    /// detailed diagnostics were requested.
    pub old_present_hash: Option<String>,
    /// The corrected boundary's hash after resimulating, present only when
    /// detailed diagnostics were requested.
    pub new_present_hash: Option<String>,
    /// The first structural disagreement between the old and new present
    /// boundary, present only when both hashes were computed and differ.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// A snapshot of a [`RollbackSession`]'s retained-state shape.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackSessionDiagnostics {
    /// Lifecycle status.
    pub status: RollbackSessionStatus,
    /// The session's current boundary tick.
    pub present_boundary: i64,
    /// Monotonic input-authority boundary.
    pub confirmed_tick: i64,
    /// Confirmed input capped to outputs that exist.
    pub confirmed_output_tick: i64,
    /// Total completed [`reconcile`] calls that changed state.
    pub rollback_count: i64,
    /// Total newly-inserted authoritative samples that differed from an
    /// already-consumed prediction.
    pub correction_count: i64,
    /// Cumulative predicted slot executions, including resimulation.
    pub predicted_slot_samples: i64,
    /// Cumulative tick executions with at least one predicted slot.
    pub predicted_ticks: i64,
    /// The most recent rollback's depth.
    pub latest_rollback_depth: i64,
    /// The largest rollback depth seen so far.
    pub max_rollback_depth: i64,
    /// Total ticks resimulated across every rollback.
    pub resimulated_ticks: i64,
    /// Total times an authoritative arrival was rejected as outside the
    /// retained window.
    pub late_window_failures: i64,
    /// The most recent rollback that changed state, if any.
    pub last_rollback: Option<RollbackSessionLastRollback>,
    /// The most recent [`compare`] call's result, if any.
    pub last_comparison: Option<RollbackComparison>,
    /// The retained input history's own diagnostics.
    pub input_history: rollback_input_history::RollbackInputHistoryDiagnostics,
    /// The retained snapshot history's own diagnostics.
    pub snapshot_history: rollback_snapshot_history::RollbackSnapshotHistoryDiagnostics,
}

/// The outcome of one [`accounting`] call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackSessionAccounting {
    /// The retained input history's own byte accounting.
    pub input: rollback_input_history::RollbackInputHistoryAccounting,
    /// Retained output bytes (see the module doc comment: a diagnostic size
    /// proxy, not an exact encoding).
    pub output_bytes: i64,
    /// Retained canonical snapshot bytes.
    pub snapshot_bytes: i64,
    /// Sum of `input.total_bytes`, `output_bytes`, and `snapshot_bytes`.
    pub total_bytes: i64,
}

/// The rollback coordinator for one live match. Every field is internal
/// state; use the free functions in this module to read or mutate it.
/// Fields are `pub` (ARCHITECTURE.md §3 rule 6: everything a test touches is `pub`).
pub struct RollbackSession {
    /// The live soccer state.
    pub state: MatchState,
    /// The live combat companion, present exactly when combat is active.
    pub combat_state: Option<CombatMatchState>,
    /// The retained authoritative/predicted input history.
    pub input_history: RollbackInputHistory,
    /// The retained start-of-tick snapshot ring.
    pub snapshot_history: RollbackSnapshotHistory,
    /// Retained per-tick outputs, keyed by causal tick.
    pub outputs: IndexMap<i64, RollbackTickOutput>,
    /// Whether [`accounting`] uses the incremental cache below.
    pub track_output_bytes: bool,
    /// The incremental retained-output byte total, valid only while
    /// `track_output_bytes` is set.
    pub output_bytes: i64,
    /// Per-tick cached retained-output byte lengths, present exactly while
    /// `track_output_bytes` is set.
    pub counted_output_bytes: Option<IndexMap<i64, i64>>,
    /// Lifecycle status.
    pub status: RollbackSessionStatus,
    /// Total completed [`reconcile`] calls that changed state.
    pub rollback_count: i64,
    /// Total newly-inserted authoritative samples that differed from an
    /// already-consumed prediction.
    pub correction_count: i64,
    /// Cumulative predicted slot executions, including resimulation.
    pub predicted_slot_samples: i64,
    /// Cumulative tick executions with at least one predicted slot.
    pub predicted_ticks: i64,
    /// The most recent rollback's depth.
    pub latest_rollback_depth: i64,
    /// The largest rollback depth seen so far.
    pub max_rollback_depth: i64,
    /// Total ticks resimulated across every rollback.
    pub resimulated_ticks: i64,
    /// Total times an authoritative arrival was rejected as outside the
    /// retained window.
    pub late_window_failures: i64,
    /// The tick an over-window correction was rejected at, if any.
    pub late_input_tick: Option<i64>,
    /// The most recent rollback that changed state, if any.
    pub last_rollback: Option<RollbackSessionLastRollback>,
    /// The most recent [`compare`] call's result, if any.
    pub last_comparison: Option<RollbackComparison>,
    /// Optional wall-time observer; see the module doc comment.
    pub measure: Option<RollbackSessionMeasure>,
    /// Sim tuning. `crate::r#match::step` takes tuning explicitly (AGENTS.md
    /// §2: `sim/` carries no ambient global state), so this session owns its
    /// own default registry rather than reading one off an implicit
    /// singleton.
    pub tune: Tuning,
}

fn sample_differs(sample: &InputSample, record: &RollbackInputSlotRecord) -> bool {
    *sample != record.sample
}

fn count_predictions(session: &mut RollbackSession, record: &RollbackInputTickRecord) {
    let predicted = record
        .slots
        .iter()
        .filter(|slot| slot.status == RollbackInputStatus::Predicted)
        .count() as i64;
    session.predicted_slot_samples += predicted;
    if predicted > 0 {
        session.predicted_ticks += 1;
    }
}

fn make_output(
    tick: i64,
    record: &RollbackInputTickRecord,
    snapshot: &MatchSnapshot,
) -> RollbackTickOutput {
    let state = &snapshot.state;
    RollbackTickOutput {
        tick,
        start_boundary: tick,
        end_boundary: tick + 1,
        input: *record,
        events: state.events.clone(),
        combat_events: snapshot.combat.as_ref().map(|combat| combat.events.clone()),
        state: RollbackOutputStateView {
            score: state.score,
            time_left: state.time_left,
            finished: state.finished,
        },
        finished: state.finished,
    }
}

/// Retained-output byte accounting is a diagnostic size proxy; see the
/// module doc comment.
fn output_len(output: &RollbackTickOutput) -> i64 {
    format!("{output:?}").len() as i64
}

fn store_output(session: &mut RollbackSession, tick: i64, output: Option<RollbackTickOutput>) {
    if session.track_output_bytes {
        let counted = session
            .counted_output_bytes
            .as_mut()
            .expect("track_output_bytes implies counted_output_bytes is Some");
        if let Some(bytes) = counted.shift_remove(&tick) {
            session.output_bytes -= bytes;
        }
    }
    match output {
        Some(output) => {
            session.outputs.insert(tick, output);
        }
        None => {
            session.outputs.shift_remove(&tick);
        }
    }
}

fn counted_output_bytes(session: &mut RollbackSession) -> i64 {
    let mut newly_counted: Vec<(i64, i64)> = Vec::new();
    for (&tick, output) in &session.outputs {
        let already_counted = session
            .counted_output_bytes
            .as_ref()
            .expect("track_output_bytes implies counted_output_bytes is Some")
            .contains_key(&tick);
        if !already_counted {
            newly_counted.push((tick, output_len(output)));
        }
    }
    let counted = session
        .counted_output_bytes
        .as_mut()
        .expect("track_output_bytes implies counted_output_bytes is Some");
    for (tick, bytes) in newly_counted {
        counted.insert(tick, bytes);
        session.output_bytes += bytes;
    }
    session.output_bytes
}

/// Run `operation` under the session's optional wall-time hook (see the
/// module doc comment for the nesting caveat). `session.measure` is removed
/// for the duration of this call and restored before returning, so
/// `operation` may freely mutate every other field.
fn measured<T>(
    session: &mut RollbackSession,
    label: RollbackSessionMeasureLabel,
    mut operation: impl FnMut(&mut RollbackSession) -> T,
) -> T {
    let mut measure = session.measure.take();
    let result = match measure.as_mut() {
        None => operation(session),
        Some(measure_fn) => {
            let mut calls = 0i32;
            let mut captured: Option<T> = None;
            {
                let mut inner = || {
                    calls += 1;
                    assert!(
                        calls == 1,
                        "rollback measurement operation must run exactly once"
                    );
                    captured = Some(operation(session));
                };
                measure_fn(label, &mut inner);
            }
            assert!(
                calls == 1,
                "rollback measurement observer must run its operation exactly once"
            );
            captured.expect("calls == 1 implies operation ran and set result")
        }
    };
    session.measure = measure;
    result
}

fn execute_tick(session: &mut RollbackSession, tick: i64) -> RollbackTickOutput {
    assert!(
        !session.state.finished,
        "rollback session cannot simulate after full time"
    );
    assert!(
        session.state.input_tick == tick,
        "rollback session boundary is inconsistent"
    );
    let (frame, record) = rollback_input_history::materialize(&mut session.input_history, tick);
    count_predictions(session, &record);
    r#match::step(
        &mut session.state,
        fixed_clock::TICK_SECONDS,
        StepInput::Frame(&frame),
        session.combat_state.as_mut(),
        &session.tune,
    );
    let boundary = measured(session, RollbackSessionMeasureLabel::Capture, |session| {
        match_snapshot::capture_owned(&session.state, session.combat_state.as_ref())
    });
    assert_eq!(
        boundary.state.input_tick,
        tick + 1,
        "rollback session step did not advance one boundary"
    );
    let output = make_output(tick, &record, &boundary);
    rollback_snapshot_history::store_owned(&mut session.snapshot_history, boundary)
        .expect("rollback session boundary is always within the retained window it just advanced");
    store_output(session, tick, Some(output.clone()));
    output
}

fn prune_retained_outputs(session: &mut RollbackSession) {
    let snapshot_diagnostics = rollback_snapshot_history::diagnostics(&session.snapshot_history);
    let floor = snapshot_diagnostics
        .oldest_supported_tick
        .expect("rollback snapshot history has no supported floor");
    rollback_input_history::prune_before(&mut session.input_history, floor)
        .expect("rollback snapshot floor never outruns a pending divergence");
    let stale: Vec<i64> = session
        .outputs
        .keys()
        .copied()
        .filter(|&tick| tick < floor)
        .collect();
    for tick in stale {
        store_output(session, tick, None);
    }
}

/// Construct a fresh session for one match. `initial_snapshot` must be the
/// canonical slot-mode boundary-zero snapshot; `sources` names every
/// canonical slot's local/remote ownership; `max_rollback_ticks` (default
/// [`crate::rollback_input_history::ROLLBACK_WINDOW_TICKS`]) bounds the
/// retained correction window; `measure` is an optional observer that
/// cannot change logical results (see the module doc comment).
///
/// Restored via [`match_snapshot::restore_owned`], not
/// [`match_snapshot::restore`]: `initial_snapshot` is documented as a
/// trusted canonical slot-mode boundary-zero snapshot, not arbitrary
/// external input, and `restore`'s extra `match_snapshot::validate`
/// requires `state.marks` sized to the full roster immediately — a real
/// boundary zero from `crate::r#match::new` legitimately has empty `marks`
/// (`marks = { home = {}, away = {} }`) until the first tick's
/// marking-assignment pass runs. Every other producer this module feeds
/// from is already `_owned` (`capture_owned`, `store_owned`), so this keeps
/// the same trust boundary rather than being the one caller that
/// revalidates a snapshot nothing else here treats as untrusted.
///
/// # Panics
///
/// Panics if `initial_snapshot` is not slot-mode, active, boundary-zero
/// canonical state (a producer invariant, ARCHITECTURE.md §3 rule 5).
#[must_use]
pub fn new(
    initial_snapshot: &MatchSnapshot,
    sources: [RollbackInputSource; 8],
    max_rollback_ticks: Option<i64>,
    measure: Option<RollbackSessionMeasure>,
) -> RollbackSession {
    let (state, combat_state) = match_snapshot::restore_owned(initial_snapshot);
    assert!(
        state.slot_mode,
        "rollback session requires a slot-mode match snapshot"
    );
    assert!(
        state.input_tick == 0,
        "rollback session requires the tick-zero boundary"
    );
    assert!(
        !state.finished,
        "rollback session tick-zero boundary must be active"
    );
    let canonical = match_snapshot::capture_owned(&state, combat_state.as_ref());
    let mut snapshots = rollback_snapshot_history::new(max_rollback_ticks);
    rollback_snapshot_history::store_owned(&mut snapshots, canonical)
        .expect("a freshly constructed history always accepts its own boundary zero");
    RollbackSession {
        state,
        combat_state,
        input_history: rollback_input_history::new(sources),
        snapshot_history: snapshots,
        outputs: IndexMap::new(),
        track_output_bytes: false,
        output_bytes: 0,
        counted_output_bytes: None,
        status: RollbackSessionStatus::Active,
        rollback_count: 0,
        correction_count: 0,
        predicted_slot_samples: 0,
        predicted_ticks: 0,
        latest_rollback_depth: 0,
        max_rollback_depth: 0,
        resimulated_ticks: 0,
        late_window_failures: 0,
        late_input_tick: None,
        last_rollback: None,
        last_comparison: None,
        measure,
        tune: Tuning::new(),
    }
}

/// Store one local or remote authoritative sample.
///
/// # Errors
///
/// Returns the [`RollbackInputHistoryError`] `rollback_input_history::add_authoritative`
/// reported. An `outside_window` error also stalls the session permanently
/// (`RollbackSessionStatus::LateInputUnrecoverable`).
pub fn add_authoritative(
    session: &mut RollbackSession,
    tick: i64,
    slot_index: i64,
    sample: InputSample,
) -> std::result::Result<RollbackSessionArrival, RollbackInputHistoryError> {
    let used = rollback_input_history::record(&session.input_history, tick);
    let arrival = match rollback_input_history::add_authoritative(
        &mut session.input_history,
        tick,
        slot_index,
        sample,
    ) {
        Ok(arrival) => arrival,
        Err(err) => {
            if err.code == rollback_input_history::RollbackInputHistoryErrorCode::OutsideWindow
                && session.status != RollbackSessionStatus::LateInputUnrecoverable
            {
                session.status = RollbackSessionStatus::LateInputUnrecoverable;
                session.late_window_failures += 1;
                session.late_input_tick = Some(tick);
            }
            return Err(err);
        }
    };
    let correction = used
        .map(|record| sample_differs(&sample, &record.slots[(slot_index - 1) as usize]))
        .unwrap_or(false);
    if correction && !arrival.duplicate {
        session.correction_count += 1;
    }
    Ok(RollbackSessionArrival {
        duplicate: arrival.duplicate,
        confirmed_tick: arrival.confirmed_tick,
        earliest_divergence: arrival.earliest_divergence,
        correction: correction && !arrival.duplicate,
    })
}

/// Validate and retain a complete transport-tick batch atomically.
///
/// # Errors
///
/// Returns the [`RollbackInputHistoryError`] `rollback_input_history::add_authoritative_batch`
/// reported; no row is retained on a rejected batch.
pub fn add_authoritative_batch(
    session: &mut RollbackSession,
    arrivals: &[RollbackAuthoritativeInput],
) -> std::result::Result<RollbackSessionBatchArrival, RollbackInputHistoryError> {
    let accepted =
        rollback_input_history::add_authoritative_batch(&mut session.input_history, arrivals)?;
    let mut corrections = 0i64;
    for arrival in &accepted.inserted_rows {
        if let Some(used) = rollback_input_history::record(&session.input_history, arrival.tick) {
            let existing = used.slots[(arrival.slot_index - 1) as usize];
            if sample_differs(&arrival.sample, &existing) {
                corrections += 1;
            }
        }
    }
    session.correction_count += corrections;
    Ok(RollbackSessionBatchArrival {
        inserted: accepted.inserted,
        duplicates: accepted.duplicates,
        inserted_rows: accepted.inserted_rows,
        confirmed_tick: accepted.confirmed_tick,
        earliest_divergence: accepted.earliest_divergence,
        corrections,
    })
}

/// Predict one tick forward.
///
/// # Errors
///
/// Returns `LateInputUnrecoverable` if a prior correction was rejected as
/// outside the retained window, or `MatchFinished` if the live match has
/// already reached full time. Neither error advances any state.
pub fn step(session: &mut RollbackSession) -> Result<RollbackTickOutput> {
    if session.status == RollbackSessionStatus::LateInputUnrecoverable {
        return Err(RollbackSessionError::new(
            RollbackSessionErrorCode::LateInputUnrecoverable,
            "rollback session cannot progress after an over-window correction",
        ));
    }
    if session.state.finished {
        session.status = RollbackSessionStatus::Finished;
        return Err(RollbackSessionError::new(
            RollbackSessionErrorCode::MatchFinished,
            "rollback session cannot simulate after full time",
        ));
    }
    let tick = session.state.input_tick;
    let output = measured(session, RollbackSessionMeasureLabel::Tick, |session| {
        execute_tick(session, tick)
    });
    session.status = if session.state.finished {
        RollbackSessionStatus::Finished
    } else {
        RollbackSessionStatus::Active
    };
    prune_retained_outputs(session);
    Ok(output)
}

fn unchanged_reconcile_result(
    session: &RollbackSession,
    causal_tick: Option<i64>,
    restore_status: Option<RollbackSnapshotLookupStatus>,
) -> RollbackReconcileResult {
    let present = session.state.input_tick;
    RollbackReconcileResult {
        changed: false,
        status: session.status,
        causal_tick,
        restored_boundary: None,
        restore_status,
        old_present_boundary: present,
        new_present_boundary: present,
        corrected_from_tick: None,
        corrected_through_tick: None,
        replaced_from_tick: None,
        replaced_through_tick: None,
        corrected_outputs: Vec::new(),
        old_present_hash: None,
        new_present_hash: None,
        first_difference: None,
    }
}

fn reconcile_changed(
    session: &mut RollbackSession,
    causal_tick: i64,
    detailed_diagnostics: bool,
) -> RollbackReconcileResult {
    let old_present = session.state.input_tick;
    let restore_status = rollback_snapshot_history::status(&session.snapshot_history, causal_tick);
    if restore_status == RollbackSnapshotLookupStatus::OutsideWindow {
        session.status = RollbackSessionStatus::LateInputUnrecoverable;
        session.late_window_failures += 1;
        session.late_input_tick = Some(causal_tick);
        return unchanged_reconcile_result(session, Some(causal_tick), Some(restore_status));
    }
    assert!(
        restore_status != RollbackSnapshotLookupStatus::Missing,
        "rollback snapshot invariant: boundary {causal_tick} is missing before correction of present {old_present}"
    );

    let mut old_snapshot: Option<MatchSnapshot> = None;
    let mut old_hash: Option<String> = None;
    if detailed_diagnostics {
        old_snapshot = Some(measured(
            session,
            RollbackSessionMeasureLabel::Capture,
            |session| match_snapshot::capture_owned(&session.state, session.combat_state.as_ref()),
        ));
        let (hash, _status) =
            rollback_snapshot_history::boundary_hash(&mut session.snapshot_history, old_present);
        old_hash = Some(hash.expect("a retained present boundary always hashes"));
    }
    assert_eq!(
        rollback_input_history::consume_earliest_divergence(&mut session.input_history),
        Some(causal_tick),
        "rollback divergence changed before restore"
    );
    let (restored_state, restored_combat) =
        measured(session, RollbackSessionMeasureLabel::Restore, |session| {
            let (state, combat, _status) = rollback_snapshot_history::restore_simulation(
                &session.snapshot_history,
                causal_tick,
            );
            (
                state.expect("a retained-or-present causal boundary always restores"),
                combat,
            )
        });
    session.state = restored_state;
    session.combat_state = restored_combat;

    let corrected_outputs = measured(
        session,
        RollbackSessionMeasureLabel::Resimulation,
        |session| {
            let mut outputs = Vec::new();
            let mut tick = causal_tick;
            while tick < old_present && !session.state.finished {
                outputs.push(execute_tick(session, tick));
                session.resimulated_ticks += 1;
                tick += 1;
            }
            outputs
        },
    );

    let new_present = session.state.input_tick;
    if new_present < old_present {
        rollback_snapshot_history::truncate_after(&mut session.snapshot_history, new_present)
            .expect("a shortened resimulation always kept its own new present boundary");
        rollback_input_history::truncate_from(&mut session.input_history, new_present)
            .expect("the new present boundary is always at or after the retained floor");
        let stale: Vec<i64> = session
            .outputs
            .keys()
            .copied()
            .filter(|&tick| tick >= new_present)
            .collect();
        for tick in stale {
            store_output(session, tick, None);
        }
    }
    prune_retained_outputs(session);

    let mut new_hash: Option<String> = None;
    let mut first_difference: Option<MatchSnapshotDifference> = None;
    if detailed_diagnostics {
        let (hash, _status) =
            rollback_snapshot_history::boundary_hash(&mut session.snapshot_history, new_present);
        new_hash = Some(hash.expect("a retained present boundary always hashes"));
        if old_hash != new_hash {
            let (difference, _status) = rollback_snapshot_history::first_difference(
                &session.snapshot_history,
                new_present,
                old_snapshot
                    .as_ref()
                    .expect("detailed diagnostics always captured the old snapshot"),
            );
            first_difference = difference;
        }
    }
    let depth = old_present - causal_tick;
    session.rollback_count += 1;
    session.latest_rollback_depth = depth;
    session.max_rollback_depth = session.max_rollback_depth.max(depth);
    session.status = if session.state.finished {
        RollbackSessionStatus::Finished
    } else {
        RollbackSessionStatus::Active
    };
    session.last_rollback = Some(RollbackSessionLastRollback {
        causal_tick,
        restored_boundary: causal_tick,
        old_present_boundary: old_present,
        new_present_boundary: new_present,
        old_present_hash: old_hash.clone(),
        new_present_hash: new_hash.clone(),
        first_difference: first_difference.clone(),
    });
    RollbackReconcileResult {
        changed: true,
        status: session.status,
        causal_tick: Some(causal_tick),
        restored_boundary: Some(causal_tick),
        restore_status: Some(restore_status),
        old_present_boundary: old_present,
        new_present_boundary: new_present,
        corrected_from_tick: (!corrected_outputs.is_empty()).then_some(causal_tick),
        corrected_through_tick: (!corrected_outputs.is_empty()).then_some(new_present - 1),
        replaced_from_tick: Some(causal_tick),
        replaced_through_tick: (old_present > causal_tick).then_some(old_present - 1),
        corrected_outputs,
        old_present_hash: old_hash,
        new_present_hash: new_hash,
        first_difference,
    }
}

/// Reconcile the session against every retained divergence: if a correction
/// changed an already-simulated tick, rewind to the earliest one and
/// resimulate forward. A no-op call (no divergence) is cheap and reads no
/// snapshot state. `detailed_diagnostics` additionally computes the
/// predicted-versus-corrected hashes and first structural difference.
pub fn reconcile(
    session: &mut RollbackSession,
    detailed_diagnostics: bool,
) -> RollbackReconcileResult {
    if session.status == RollbackSessionStatus::LateInputUnrecoverable {
        let late_tick = session
            .late_input_tick
            .expect("an unrecoverable rollback session always retains its causal late input tick");
        return unchanged_reconcile_result(
            session,
            Some(late_tick),
            Some(RollbackSnapshotLookupStatus::OutsideWindow),
        );
    }
    let causal_tick = match rollback_input_history::earliest_divergence(&session.input_history) {
        Some(tick) => tick,
        None => return unchanged_reconcile_result(session, None, None),
    };
    measured(session, RollbackSessionMeasureLabel::Rollback, |session| {
        reconcile_changed(session, causal_tick, detailed_diagnostics)
    })
}

/// Apply one host authority batch through exactly one reconciliation: the
/// complete authority set is retained atomically before the rollback
/// machinery runs once.
///
/// # Errors
///
/// Returns the [`RollbackInputHistoryError`] `add_authoritative_batch`
/// reported; no reconciliation runs on a rejected batch.
pub fn apply_authoritative_batch(
    session: &mut RollbackSession,
    arrivals: &[RollbackAuthoritativeInput],
    detailed_diagnostics: bool,
) -> std::result::Result<RollbackSessionBatchApplyResult, RollbackInputHistoryError> {
    let accepted = add_authoritative_batch(session, arrivals)?;
    Ok(RollbackSessionBatchApplyResult {
        arrival: accepted,
        reconciliation: reconcile(session, detailed_diagnostics),
    })
}

/// A **read-only** borrow of the session's live simulation state.
///
/// For read-only consumers that must run against the state the local sim
/// actually holds rather than a copy of it — `crate::ball_prediction`'s
/// query entry points are the motivating case, and a render-layer overlay
/// wanting pass travel times is the other. The shared borrow is the
/// contract: a consumer reached through this cannot mutate the session, so
/// it cannot enter a snapshot, a state hash, or a peer comparison.
#[must_use]
pub fn state(session: &RollbackSession) -> &MatchState {
    &session.state
}

/// An independent capture of the session's current live boundary.
#[must_use]
pub fn current_snapshot(session: &RollbackSession) -> MatchSnapshot {
    match_snapshot::capture_owned(&session.state, session.combat_state.as_ref())
}

/// An owned copy of a retained boundary, plus its retention status.
#[must_use]
pub fn snapshot(
    session: &RollbackSession,
    boundary_tick: i64,
) -> rollback_snapshot_history::RollbackSnapshotLookup {
    rollback_snapshot_history::lookup(&session.snapshot_history, boundary_tick)
}

/// Compare an independent retained history against this session's own.
pub fn compare_retained(
    session: &mut RollbackSession,
    expected: &mut RollbackSnapshotHistory,
    boundary_tick: i64,
) -> rollback_snapshot_history::RollbackSnapshotHistoryComparison {
    rollback_snapshot_history::compare(expected, &mut session.snapshot_history, boundary_tick)
}

/// Read back an authoritative sample this session already holds. Read-only
/// and copying, so a caller cannot mutate retained authority through it.
/// `None` means the row is either not authoritative yet or has been pruned
/// below the retained floor — callers must treat both as "not available"
/// rather than assume the tick is in the window.
#[must_use]
pub fn authoritative_sample(
    session: &RollbackSession,
    tick: i64,
    slot_index: i64,
) -> Option<InputSample> {
    rollback_input_history::authoritative_record(&session.input_history, tick, slot_index)
        .map(|record| record.sample)
}

/// The retained output for `input_tick`, if any.
#[must_use]
pub fn output(session: &RollbackSession, input_tick: i64) -> Option<RollbackTickOutput> {
    session.outputs.get(&input_tick).cloned()
}

/// Compare the session's live state against an independently held
/// `expected` snapshot, and remember the result as `last_comparison`.
///
/// Both sides are hashed and diffed via the *canonical* (non-revalidating)
/// path — `match_snapshot::hash_canonical`/`first_difference_canonical`,
/// matching `crate::rollback_snapshot_history::boundary_hash`'s own choice.
/// `match_snapshot::hash`/`first_difference` restore-then-validate their
/// input first (`crate::match_snapshot::validate` requires `state.marks`
/// sized to the full roster immediately), which a live in-kickoff-hold
/// `MatchState` legitimately fails — marking assignment has not run yet.
/// `actual` and `expected` are always already-canonical snapshots here
/// (`current_snapshot` and every caller's own retained/captured state), so
/// re-validating buys nothing this module's callers don't already
/// guarantee.
pub fn compare(
    session: &mut RollbackSession,
    expected: &MatchSnapshot,
    causal_tick: Option<i64>,
) -> RollbackComparison {
    let actual = match_snapshot::capture_owned(&session.state, session.combat_state.as_ref());
    let actual_hash = match_snapshot::hash_canonical(&actual);
    let expected_hash = match_snapshot::hash_canonical(expected);
    let matched = actual_hash == expected_hash;
    let first_difference = if matched {
        None
    } else {
        match_snapshot::first_difference_canonical(expected, &actual)
    };
    let comparison = RollbackComparison {
        matched,
        boundary_mismatch: actual.state.input_tick != expected.state.input_tick,
        actual_boundary: actual.state.input_tick,
        expected_boundary: expected.state.input_tick,
        actual_hash,
        expected_hash,
        causal_tick,
        first_difference,
    };
    session.last_comparison = Some(comparison.clone());
    comparison
}

/// A snapshot of this session's retained-state shape.
#[must_use]
pub fn diagnostics(session: &RollbackSession) -> RollbackSessionDiagnostics {
    let confirmed = rollback_input_history::confirmed_tick(&session.input_history);
    let output_ceiling = session.state.input_tick - 1;
    RollbackSessionDiagnostics {
        status: session.status,
        present_boundary: session.state.input_tick,
        confirmed_tick: confirmed,
        confirmed_output_tick: confirmed.min(output_ceiling),
        rollback_count: session.rollback_count,
        correction_count: session.correction_count,
        predicted_slot_samples: session.predicted_slot_samples,
        predicted_ticks: session.predicted_ticks,
        latest_rollback_depth: session.latest_rollback_depth,
        max_rollback_depth: session.max_rollback_depth,
        resimulated_ticks: session.resimulated_ticks,
        late_window_failures: session.late_window_failures,
        last_rollback: session.last_rollback.clone(),
        last_comparison: session.last_comparison.clone(),
        input_history: rollback_input_history::diagnostics(&session.input_history),
        snapshot_history: rollback_snapshot_history::diagnostics(&session.snapshot_history),
    }
}

/// Opt into incremental retained-output byte accounting for this session.
/// Off by default; only a caller that reads [`accounting`] far more often
/// than it retains outputs needs it. Enabling starts from an empty count, so
/// the reported total is unchanged by the act of switching tracking on.
pub fn track_output_bytes(session: &mut RollbackSession) {
    session.counted_output_bytes = Some(IndexMap::new());
    session.output_bytes = 0;
    session.track_output_bytes = true;
}

/// Exact retained payload accounting. Snapshot bytes use the
/// [`crate::match_snapshot::MatchSnapshot`] versioned canonical encoding;
/// input bytes use [`crate::rollback_input_history`]'s own versioned
/// canonical encoding; output bytes are a diagnostic size proxy (see the
/// module doc comment).
#[must_use]
pub fn accounting(session: &mut RollbackSession) -> RollbackSessionAccounting {
    let input = rollback_input_history::accounting(&session.input_history);
    let output_bytes = if session.track_output_bytes {
        counted_output_bytes(session)
    } else {
        session.outputs.values().map(output_len).sum()
    };
    let snapshot_bytes =
        rollback_snapshot_history::diagnostics(&session.snapshot_history).canonical_bytes;
    RollbackSessionAccounting {
        input,
        output_bytes,
        snapshot_bytes,
        total_bytes: input.total_bytes + output_bytes + snapshot_bytes,
    }
}
