//! Pure authoritative-reference rollback laboratory. The reference consumes
//! only already-materialized tape frames; the client receives those same
//! rows immediately for local slots or through deterministic network
//! conditions for remote slots.
//!
//! ## No manual deep-copy helpers
//!
//! As with `rollback_session`/`rollback_playable_lab`, every payload type
//! here is an owned Rust value with a real [`Clone`] impl, so a `.clone()`
//! at a read boundary already gives an independent copy — no manual
//! deep-copy helper is needed.
//!
//! ## The runner-owned `measure` hook loses its secondary reference-capture
//! use
//!
//! `RollbackLabOptions.measure` needs to be usable in two places:
//! `rollback_session::new` hands it to the session for its own internal
//! timing, and `advance_frame` separately wraps the *independent
//! reference's* capture with the same hook. A Rust `RollbackSessionMeasure`
//! (`Box<dyn FnMut(...)>`) is uniquely owned once passed to
//! `crate::rollback_session::new`, and changing that already shipped,
//! differential-tested type to a shareable `Rc<RefCell<...>>` was judged too
//! large a change to make from this file for a measurement path that
//! changes no logical result: clock values are neither read nor retained
//! here and cannot enter the logical result. This module therefore passes
//! `options.measure` to the session (the primary, documented use) and does
//! not additionally wrap the reference capture in `advance_frame`.
//!
//!
//! Every independent reference `MatchState` this module steps directly
//! comment explains — see `rollback_playable_lab.rs` for the same pattern.

use crate::combat_snapshot::CombatMatchState;
use crate::input_frame::{self, InputFrame, InputSample};
use crate::input_tape::{self, InputTape};
use crate::match_snapshot::{self, MatchEvent, MatchSnapshot, MatchSnapshotDifference, MatchState};
use crate::network_conditions::{self, NetworkConditions, NetworkDelivery, NetworkProfile};
use crate::retained_history;
use crate::rollback_events::{self, RollbackEventDiff, RollbackEventStep, RollbackEventTimeline};
use crate::rollback_input_history::{self, RollbackInputSource};
use crate::rollback_session::{self, RollbackSession, RollbackSessionMeasure, RollbackTickOutput};
use crate::rollback_snapshot_history::{self, RollbackSnapshotHistory};
use crate::tuning::Tuning;
use gc_core::fnv1a64::Fnv1a64State;
use indexmap::IndexMap;

/// Schema version for [`RollbackLabResult`].
pub const SCHEMA: i64 = 1;
/// Default network impairment RNG seed.
pub const DEFAULT_NETWORK_SEED: i64 = 7302;
/// Default bounded post-tape drain budget, in transport ticks.
pub const DEFAULT_DRAIN_TICKS: i64 = 256;

/// A [`RollbackLabResult`]'s outcome classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackLabStatus {
    /// The client reproduced the reference exactly.
    Converged,
    /// A retained comparison mismatched.
    Diverged,
    /// A correction was rejected as outside the retained window.
    LateInputUnrecoverable,
    /// Authority for a retained tick never arrived.
    UnconfirmedAuthority,
    /// The bounded final drain ran out its budget incomplete.
    DrainIncomplete,
    /// The client never reached the reference's final boundary.
    IncompleteClient,
    /// A comparison boundary was neither retained nor present on one side.
    ComparisonHistoryMissing,
    /// The stable-event timeline diverged from the reference's own.
    EventDiverged,
}

/// A deliberate single-tick, single-slot input corruption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackLabCorruption {
    /// The tick to corrupt.
    pub tick: i64,
    /// The canonical slot to corrupt.
    pub slot: i64,
}

/// A per-tick, read-only campaign observer; see
/// [`RollbackLabOptions::observer`].
pub type RollbackLabObserver = Box<dyn FnMut(i64, &RollbackLabRunState)>;

/// Construction options for [`new_campaign`].
#[derive(Default)]
pub struct RollbackLabOptions {
    /// A named authored profile; defaults to `"clean"`.
    pub profile_name: Option<String>,
    /// A caller-supplied profile, overriding the authored lookup.
    pub profile: Option<NetworkProfile>,
    /// Network impairment RNG seed; defaults to [`DEFAULT_NETWORK_SEED`].
    pub network_seed: Option<i64>,
    /// Canonical eight-slot local/remote ownership; defaults to the first
    /// [`input_frame::HOME_SLOT_COUNT`] slots local, the rest remote.
    pub sources: Option<[RollbackInputSource; 8]>,
    /// Maximum retained rollback ticks; defaults to
    /// [`crate::rollback_input_history::ROLLBACK_WINDOW_TICKS`].
    pub max_rollback_ticks: Option<i64>,
    /// Bounded post-tape drain budget, in transport ticks; defaults to
    /// [`DEFAULT_DRAIN_TICKS`].
    pub drain_ticks: Option<i64>,
    /// An optional deliberate corruption.
    pub corruption: Option<RollbackLabCorruption>,
    /// Optional runner-owned wall-time observer; see the module doc
    /// comment.
    pub measure: Option<RollbackSessionMeasure>,
    /// Skip full replay validation, trusting `tape` was already validated
    /// by its producer.
    pub prevalidated_tape: bool,
    /// Optional per-tick observer for read-only per-tick work — issuing
    /// queries against a sim service, for example — run inside a real
    /// campaign. Receives the tick just advanced and a **shared** reference
    /// to the run state, so a test can drive a query-heavy consumer through
    /// the nine-scenario matrix without `RollbackLabRunState` widening any
    /// field to mutable access outside this module. Unlike `measure`
    /// (above), this has exactly one call site (`step_campaign`), so it
    /// carries none of that option's "two places, one already-shipped type"
    /// complication. Called once per advanced input-tape frame, after that
    /// frame's rollback/resimulation work for the tick has settled.
    pub observer: Option<RollbackLabObserver>,
}

/// One rollback-depth histogram row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackLabDepth {
    /// The rollback depth this row counts.
    pub depth: i64,
    /// How many rollbacks landed at this depth.
    pub count: i64,
}

/// The first retained-boundary mismatch a campaign observed.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackLabDivergence {
    /// The causal tick attributed to this divergence.
    pub causal_tick: i64,
    /// The mismatched boundary.
    pub boundary: i64,
    /// The reference's hash at this boundary.
    pub expected_hash: String,
    /// The client's hash at this boundary.
    pub actual_hash: String,
    /// The first structural disagreement, if computed.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// High-water marks observed across a campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RollbackLabPeaks {
    /// Peak retained snapshot count.
    pub snapshot_count: i64,
    /// Peak retained snapshot bytes.
    pub snapshot_bytes: i64,
    /// Peak retained authoritative input samples.
    pub input_authoritative_samples: i64,
    /// Peak retained materialized effective ticks.
    pub input_effective_ticks: i64,
    /// Peak retained materialized tick records.
    pub input_record_ticks: i64,
    /// Peak pending network envelopes.
    pub network_pending_envelopes: i64,
    /// Peak pending network record references.
    pub network_pending_record_references: i64,
    /// Peak delivered-ledger entries.
    pub network_delivered_ledger_entries: i64,
    /// Peak retained authoritative network records.
    pub network_authoritative_records: i64,
    /// Peak retained input-history bytes.
    pub input_bytes: i64,
    /// Peak retained output bytes.
    pub output_bytes: i64,
    /// Peak retained event bytes.
    pub event_bytes: i64,
    /// Peak retained total history bytes.
    pub history_bytes: i64,
}

/// Combined byte accounting across the session and the event timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackLabHistoryAccounting {
    /// The session's own input-history accounting.
    pub input: rollback_input_history::RollbackInputHistoryAccounting,
    /// Retained output bytes.
    pub output_bytes: i64,
    /// Retained canonical snapshot bytes.
    pub snapshot_bytes: i64,
    /// Retained event bytes.
    pub event_bytes: i64,
    /// Sum of every category above.
    pub total_bytes: i64,
}

/// Summary metrics for one completed campaign.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackLabMetrics {
    /// Cumulative predicted slot executions, including resimulation.
    pub predicted_slot_samples: i64,
    /// Cumulative tick executions with at least one predicted slot.
    pub predicted_ticks: i64,
    /// Total newly-inserted authoritative samples that differed from an
    /// already-consumed prediction.
    pub correction_count: i64,
    /// Total completed reconciliations that changed state.
    pub rollback_count: i64,
    /// Total ticks resimulated across every rollback.
    pub resimulated_ticks: i64,
    /// The most recent rollback's depth.
    pub latest_rollback_depth: i64,
    /// The largest rollback depth seen so far.
    pub max_rollback_depth: i64,
    /// Rollback-depth histogram, sorted by depth.
    pub rollback_depths: Vec<RollbackLabDepth>,
    /// Whether the client predicted full time before the reference reached
    /// it.
    pub predicted_early_finish: bool,
    /// Times the client reactivated from `finished` after a correction.
    pub reactivation_count: i64,
    /// Delivered rows skipped because their tick was already confirmed.
    pub confirmed_redundant_rows_skipped: i64,
    /// Retained-boundary comparisons performed.
    pub compared_boundaries: i64,
    /// Expected retained-boundary comparisons for a fully converged run.
    pub expected_boundaries: i64,
    /// Retained client snapshot count.
    pub current_snapshot_count: i64,
    /// Retained client snapshot bytes.
    pub current_snapshot_bytes: i64,
    /// High-water marks across the whole campaign.
    pub peaks: RollbackLabPeaks,
}

/// Stable-event agreement metrics between the reference and the confirmed
/// client timeline.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RollbackLabEventMetrics {
    /// Events newly present relative to the prior speculative interval,
    /// summed across every impaired diff.
    pub added: i64,
    /// Events revoked, summed across every impaired diff.
    pub revoked: i64,
    /// Events replaced in place, summed across every impaired diff.
    pub replaced: i64,
    /// Confirmed client steps observed.
    pub confirmed_steps: i64,
    /// Confirmed client soccer events observed.
    pub confirmed_match_events: i64,
    /// Confirmed client combat events observed.
    pub confirmed_combat_events: i64,
    /// Confirmed client lifecycle events observed.
    pub confirmed_lifecycle_events: i64,
    /// Speculative steps still retained at the end of the campaign.
    pub speculative_residue: i64,
    /// Digest of every confirmed reference step, in causal order.
    pub reference_digest: String,
    /// Digest of every confirmed client step, in causal order.
    pub confirmed_digest: String,
    /// Whether the two digests agree and the client fully confirmed with
    /// no residue.
    pub matched: bool,
}

/// One row of a campaign's fixed-order event trace.
#[derive(Clone, Debug, PartialEq)]
pub enum RollbackLabEventTraceRow {
    /// One reference tick's step was confirmed.
    ReferenceConfirmed {
        /// The confirmed reference step.
        step: RollbackEventStep,
    },
    /// The impaired client's speculative timeline diffed against its prior
    /// state.
    ImpairedDiff {
        /// The diff.
        diff: RollbackEventDiff,
    },
    /// One impaired client tick's step was confirmed.
    ImpairedConfirmed {
        /// The confirmed client step.
        step: RollbackEventStep,
    },
    /// A retained boundary was sampled for the replay trace.
    ReplayBoundary {
        /// The sampled boundary.
        boundary: i64,
        /// The sample.
        replay_sample: RollbackLabReplaySample,
    },
    /// A rollback truncated the client's speculative tail.
    ReplayTruncate {
        /// The boundary truncation ran from.
        boundary: i64,
    },
}

/// One replay-trace sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackLabReplaySample {
    /// The sampled boundary.
    pub boundary: i64,
    /// Ball x position at this boundary.
    pub ball_x: f64,
    /// Ball y position at this boundary.
    pub ball_y: f64,
    /// Home score at this boundary.
    pub score_home: i64,
    /// Away score at this boundary.
    pub score_away: i64,
}

/// The outcome of a campaign's bounded final drain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollbackLabDrainSummary {
    /// The last transport tick the drain advanced to.
    pub final_tick: i64,
    /// Whether every requested input was recovered and nothing remains
    /// pending.
    pub complete: bool,
    /// Pending envelope count at the end of the drain.
    pub pending: i64,
    /// Requested inputs recovered by the end of the drain.
    pub recovered: i64,
    /// Requested inputs total.
    pub requested: i64,
}

/// The complete, immutable outcome of one campaign.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackLabResult {
    /// Always [`SCHEMA`].
    pub schema: i64,
    /// Whether every acceptance criterion for a fully converged run held.
    pub success: bool,
    /// This result's classification.
    pub status: RollbackLabStatus,
    /// The tape's fixture identity string.
    pub fixture: String,
    /// The tape's recording seed.
    pub fixture_seed: f64,
    /// The active network profile's name.
    pub profile: String,
    /// The active network profile's parameters, rendered for diagnostics.
    pub profile_parameters: String,
    /// Network impairment RNG seed.
    pub network_seed: i64,
    /// The canonical local/remote source pattern (`L`/`R` per slot).
    pub source_pattern: String,
    /// The tape's frame count.
    pub input_ticks: i64,
    /// The tape's own content digest.
    pub tape_digest: String,
    /// The tape's initial boundary hash.
    pub initial_hash: String,
    /// The reference's final input tick.
    pub reference_final_boundary: i64,
    /// The client's final present boundary.
    pub client_final_boundary: i64,
    /// The reference's final canonical hash.
    pub reference_final_hash: String,
    /// The client's final canonical hash.
    pub client_final_hash: String,
    /// The reference's final snapshot.
    pub reference_final_snapshot: MatchSnapshot,
    /// The client's final snapshot.
    pub client_final_snapshot: MatchSnapshot,
    /// Monotonic input-authority boundary.
    pub confirmed_tick: i64,
    /// Confirmed input capped to outputs that exist.
    pub confirmed_output_tick: i64,
    /// The tick a correction was rejected as outside the retained window,
    /// if any.
    pub late_input_tick: Option<i64>,
    /// The earliest tick whose authority never arrived, if any.
    pub unconfirmed_tick: Option<i64>,
    /// The first retained-boundary mismatch, if any.
    pub divergence: Option<RollbackLabDivergence>,
    /// Summary metrics.
    pub metrics: RollbackLabMetrics,
    /// The client's input-history diagnostics.
    pub input_diagnostics: rollback_input_history::RollbackInputHistoryDiagnostics,
    /// The client's snapshot-history diagnostics.
    pub snapshot_diagnostics: rollback_snapshot_history::RollbackSnapshotHistoryDiagnostics,
    /// Running network counters.
    pub network_counters: network_conditions::NetworkConditionCounters,
    /// Current and peak retained network state.
    pub network_diagnostics: network_conditions::NetworkConditionDiagnostics,
    /// Stable-event agreement metrics.
    pub event_metrics: RollbackLabEventMetrics,
    /// The client's stable-event timeline diagnostics.
    pub event_diagnostics: rollback_events::RollbackEventsDiagnostics,
    /// The complete fixed-order event trace.
    pub event_trace: Vec<RollbackLabEventTraceRow>,
    /// Combined byte accounting.
    pub history_accounting: RollbackLabHistoryAccounting,
    /// The bounded final drain's outcome.
    pub drain: RollbackLabDrainSummary,
}

/// Incremental campaign run state.
pub struct RollbackLabRunState {
    /// The client rollback session.
    pub session: RollbackSession,
    /// Simulated network impairment between the reference and client.
    pub network: NetworkConditions,
    /// The independent reference soccer state.
    pub reference: MatchState,
    /// The independent reference's combat companion.
    pub reference_combat: Option<CombatMatchState>,
    /// The independent reference's retained snapshot ring.
    pub reference_history: RollbackSnapshotHistory,
    /// The reference's own stable-event timeline.
    pub reference_events: RollbackEventTimeline,
    /// The impaired client's stable-event timeline.
    pub events: RollbackEventTimeline,
    /// The complete fixed-order event trace.
    pub event_trace: Vec<RollbackLabEventTraceRow>,
    /// Whether the event timeline has permanently stalled.
    pub event_window_exceeded: bool,
    /// Canonical eight-slot local/remote ownership.
    pub sources: [RollbackInputSource; 8],
    /// Rollback-depth histogram, keyed by depth.
    pub depth_counts: IndexMap<i64, i64>,
    /// The last output tick a comparison ran for, or `-1`.
    pub last_compared_output: i64,
    /// Retained-boundary comparisons performed.
    pub compared_boundaries: i64,
    /// The first retained-boundary mismatch, if any.
    pub divergence: Option<RollbackLabDivergence>,
    /// The tick a correction was rejected as outside the retained window,
    /// if any.
    pub late_input_tick: Option<i64>,
    /// The earliest tick whose authority never arrived, if any.
    pub unconfirmed_tick: Option<i64>,
    /// Whether a comparison boundary was neither retained nor present.
    pub comparison_history_missing: bool,
    /// Whether the client predicted full time before the reference reached
    /// it.
    pub predicted_early_finish: bool,
    /// Times the client reactivated from `finished` after a correction.
    pub reactivation_count: i64,
    /// Delivered rows skipped because their tick was already confirmed.
    pub confirmed_redundant_rows_skipped: i64,
    /// High-water marks across the whole campaign.
    pub peaks: RollbackLabPeaks,
}

/// An incremental campaign. Runtime entrypoints use this seam to yield
/// between logical ticks in browsers; [`run`] synchronously drains the same
/// state machine.
pub struct RollbackLabCampaign {
    /// The tape driving this campaign.
    pub tape: InputTape,
    /// The active network profile.
    pub profile: NetworkProfile,
    /// The active network profile's name.
    pub profile_name: String,
    /// Network impairment RNG seed.
    pub network_seed: i64,
    /// Bounded post-tape drain budget, in transport ticks.
    pub drain_ticks: i64,
    /// The next tape frame index to advance (1-based, matching the tape's
    /// own frame ordering).
    pub next_frame_index: i64,
    /// The completed result, present once the campaign has finished.
    pub result: Option<RollbackLabResult>,
    state: RollbackLabRunState,
    /// See [`RollbackLabOptions::observer`].
    observer: Option<RollbackLabObserver>,
}

fn digest_segment(state: &mut Fnv1a64State, value: &str) {
    state.update(value.len().to_string().as_bytes());
    state.update(b":");
    state.update(value.as_bytes());
}

/// Compute a tape's content digest.
///
/// # Panics
///
/// Panics if `tape`'s frames do not encode (a producer invariant,
/// ARCHITECTURE.md §3 rule 5).
#[must_use]
pub fn tape_digest(tape: &InputTape) -> String {
    let mut state = Fnv1a64State::new();
    digest_segment(
        &mut state,
        &match_snapshot::number_bytes(tape.version as f64),
    );
    let identity = &tape.identity;
    digest_segment(
        &mut state,
        &match_snapshot::number_bytes(identity.tape_version as f64),
    );
    digest_segment(
        &mut state,
        &match_snapshot::number_bytes(identity.input_version as f64),
    );
    digest_segment(
        &mut state,
        &match_snapshot::number_bytes(identity.snapshot_version as f64),
    );
    digest_segment(&mut state, &match_snapshot::number_bytes(identity.seed));
    digest_segment(
        &mut state,
        &match_snapshot::number_bytes(identity.tick_rate as f64),
    );
    digest_segment(&mut state, &identity.build);
    digest_segment(&mut state, &identity.source);
    digest_segment(&mut state, &identity.content);
    digest_segment(&mut state, &identity.tuning);
    digest_segment(&mut state, &identity.config);
    digest_segment(&mut state, &identity.fixture);
    if let Some(combat) = &identity.combat {
        digest_segment(&mut state, combat);
    }
    digest_segment(&mut state, &match_snapshot::encode(&tape.initial));
    for frame in &tape.frames {
        digest_segment(
            &mut state,
            &input_frame::encode(frame).expect("tape frames are always valid"),
        );
    }
    for hash in &tape.boundary_hashes {
        digest_segment(&mut state, hash);
    }
    state.hex()
}

fn copy_match_event(event: &MatchEvent) -> MatchEvent {
    event.clone()
}

fn reference_output(
    frame: &InputFrame,
    sources: &[RollbackInputSource; 8],
    snapshot: &MatchSnapshot,
) -> RollbackTickOutput {
    let mut slots = [rollback_input_history::RollbackInputSlotRecord {
        source: RollbackInputSource::Remote,
        status: rollback_input_history::RollbackInputStatus::Authoritative,
        sample: InputSample::default(),
    }; 8];
    for slot in 1..=input_frame::SLOT_COUNT {
        let index = (slot - 1) as usize;
        slots[index] = rollback_input_history::RollbackInputSlotRecord {
            source: sources[index],
            status: rollback_input_history::RollbackInputStatus::Authoritative,
            sample: frame.slots[index],
        };
    }
    let state = &snapshot.state;
    RollbackTickOutput {
        tick: frame.tick,
        start_boundary: frame.tick,
        end_boundary: frame.tick + 1,
        input: rollback_input_history::RollbackInputTickRecord {
            tick: frame.tick,
            slots,
        },
        events: state.events.iter().map(copy_match_event).collect(),
        combat_events: snapshot.combat.as_ref().map(|combat| combat.events.clone()),
        state: rollback_session::RollbackOutputStateView {
            score: state.score,
            time_left: state.time_left,
            finished: state.finished,
        },
        finished: state.finished,
    }
}

fn event_tick_output(output: &RollbackTickOutput) -> rollback_events::RollbackEventTickOutput {
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

fn event_step(
    state: &RollbackLabRunState,
    output: RollbackTickOutput,
) -> rollback_events::RollbackEventStepInput {
    let lookup = rollback_session::snapshot(&state.session, output.end_boundary);
    assert!(
        matches!(
            lookup.status,
            rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
                | rollback_snapshot_history::RollbackSnapshotLookupStatus::Retained
        ),
        "rollback lab event boundary is not retained"
    );
    rollback_events::RollbackEventStepInput {
        output: event_tick_output(&output),
        snapshot: lookup
            .snapshot
            .expect("rollback lab event boundary snapshot is missing"),
    }
}

fn trace_replay_boundary(
    state: &mut RollbackLabRunState,
    boundary: i64,
    supplied: Option<MatchSnapshot>,
) {
    let snapshot = match supplied {
        Some(snapshot) => snapshot,
        None => {
            let lookup = rollback_session::snapshot(&state.session, boundary);
            assert!(
                matches!(
                    lookup.status,
                    rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
                        | rollback_snapshot_history::RollbackSnapshotLookupStatus::Retained
                ),
                "rollback lab replay boundary is not retained"
            );
            lookup
                .snapshot
                .expect("rollback lab replay boundary snapshot is missing")
        }
    };
    state
        .event_trace
        .push(RollbackLabEventTraceRow::ReplayBoundary {
            boundary,
            replay_sample: RollbackLabReplaySample {
                boundary,
                ball_x: snapshot.state.ball.x,
                ball_y: snapshot.state.ball.y,
                score_home: snapshot.state.score.home,
                score_away: snapshot.state.score.away,
            },
        });
}

fn trace_diff(state: &mut RollbackLabRunState, diff: RollbackEventDiff) {
    if !diff.added.is_empty() || !diff.revoked.is_empty() || !diff.replaced.is_empty() {
        state
            .event_trace
            .push(RollbackLabEventTraceRow::ImpairedDiff { diff });
    }
}

fn apply_client_outputs(
    state: &mut RollbackLabRunState,
    from_tick: i64,
    through_tick: i64,
    outputs: Vec<RollbackTickOutput>,
) -> bool {
    if state.event_window_exceeded {
        return false;
    }
    let boundaries: Vec<i64> = outputs.iter().map(|output| output.end_boundary).collect();
    let steps: Vec<rollback_events::RollbackEventStepInput> = outputs
        .into_iter()
        .map(|output| event_step(state, output))
        .collect();
    match rollback_events::apply(&mut state.events, from_tick, through_tick, &steps) {
        Ok(diff) => {
            trace_diff(state, diff);
            for boundary in boundaries {
                trace_replay_boundary(state, boundary, None);
            }
            true
        }
        Err(err) => {
            assert!(
                rollback_events::diagnostics(&state.events).status
                    == rollback_events::RollbackEventsStatus::UnconfirmedWindowExceeded,
                "rollback lab event application failed: {}",
                err.message
            );
            state.event_window_exceeded = true;
            false
        }
    }
}

fn publish_confirmation(state: &mut RollbackLabRunState) {
    if state.event_window_exceeded {
        return;
    }
    let confirmed_output = rollback_session::diagnostics(&state.session).confirmed_output_tick;
    for step in rollback_events::confirm(&mut state.events, confirmed_output) {
        state
            .event_trace
            .push(RollbackLabEventTraceRow::ImpairedConfirmed { step });
    }
}

fn publish_reference(
    state: &mut RollbackLabRunState,
    frame: &InputFrame,
    snapshot: &MatchSnapshot,
) {
    let output = reference_output(frame, &state.sources, snapshot);
    let step_input = rollback_events::RollbackEventStepInput {
        output: event_tick_output(&output),
        snapshot: snapshot.clone(),
    };
    rollback_events::apply(
        &mut state.reference_events,
        frame.tick,
        frame.tick,
        &[step_input],
    )
    .expect("rollback lab reference event application never exceeds its own window");
    let confirmed = rollback_events::confirm(&mut state.reference_events, frame.tick);
    assert_eq!(
        confirmed.len(),
        1,
        "rollback lab reference must confirm exactly one step"
    );
    state
        .event_trace
        .push(RollbackLabEventTraceRow::ReferenceConfirmed {
            step: confirmed.into_iter().next().expect("checked above"),
        });
}

fn default_sources() -> [RollbackInputSource; 8] {
    let mut sources = [RollbackInputSource::Remote; 8];
    for slot in 1..=input_frame::HOME_SLOT_COUNT {
        sources[(slot - 1) as usize] = RollbackInputSource::Local;
    }
    sources
}

fn source_pattern(sources: &[RollbackInputSource; 8]) -> String {
    sources
        .iter()
        .map(|source| {
            if *source == RollbackInputSource::Local {
                'L'
            } else {
                'R'
            }
        })
        .collect()
}

fn profile_parameters(profile: &NetworkProfile) -> String {
    [
        match_snapshot::number_bytes(profile.base_delay_ticks as f64),
        match_snapshot::number_bytes(profile.jitter_min_ticks as f64),
        match_snapshot::number_bytes(profile.jitter_max_ticks as f64),
        match_snapshot::number_bytes(profile.independent_loss_rate),
        match_snapshot::number_bytes(profile.duplication_rate),
        match_snapshot::number_bytes(profile.burst_start_rate),
        match_snapshot::number_bytes(profile.burst_length_ticks as f64),
    ]
    .join(",")
}

fn profile_by_name(name: &str) -> Option<NetworkProfile> {
    use gc_data::network_profiles::NetworkProfileName::{Clean, Omp0Parity, Playable, Stress};
    let authored = match name {
        "clean" => Clean,
        "omp0_parity" => Omp0Parity,
        "playable" => Playable,
        "stress" => Stress,
        _ => return None,
    };
    Some(gc_data::network_profiles::get(authored).into())
}

fn selected_profile(options: &RollbackLabOptions) -> (NetworkProfile, String) {
    if let Some(profile) = options.profile {
        return (
            profile,
            options
                .profile_name
                .clone()
                .unwrap_or_else(|| "custom".to_string()),
        );
    }
    let name = options
        .profile_name
        .clone()
        .unwrap_or_else(|| "clean".to_string());
    let profile =
        profile_by_name(&name).unwrap_or_else(|| panic!("unknown rollback lab profile: {name}"));
    (profile, name)
}

fn client_sample(
    corruption: Option<RollbackLabCorruption>,
    tick: i64,
    slot: i64,
    sample: InputSample,
) -> InputSample {
    match corruption {
        Some(corruption) if corruption.tick == tick && corruption.slot == slot => InputSample {
            move_x: if sample.move_x == input_frame::MOVE_SCALE {
                -input_frame::MOVE_SCALE
            } else {
                input_frame::MOVE_SCALE
            },
            edges: if sample.edges == input_frame::EDGE_DASH {
                0
            } else {
                input_frame::EDGE_DASH
            },
            ..sample
        },
        _ => sample,
    }
}

fn update_peaks(state: &mut RollbackLabRunState) {
    let session = rollback_session::diagnostics(&state.session);
    let snapshots = session.snapshot_history;
    let inputs = session.input_history;
    let network = network_conditions::diagnostics(&state.network);
    // One reading, one definition of `history_bytes` — see
    // `crate::retained_history`'s module doc for why the combined
    // session-plus-event-timeline figure is the one this peak means.
    let retained = retained_history::sample(&mut state.session, &state.events);
    let peaks = &mut state.peaks;
    peaks.snapshot_count = peaks
        .snapshot_count
        .max(snapshots.peak_retained_boundary_count);
    peaks.snapshot_bytes = peaks.snapshot_bytes.max(snapshots.peak_canonical_bytes);
    peaks.input_authoritative_samples = peaks
        .input_authoritative_samples
        .max(inputs.authoritative_sample_count);
    peaks.input_effective_ticks = peaks.input_effective_ticks.max(inputs.effective_tick_count);
    peaks.input_record_ticks = peaks.input_record_ticks.max(inputs.record_tick_count);
    peaks.network_pending_envelopes = peaks
        .network_pending_envelopes
        .max(network.peak_pending_envelopes);
    peaks.network_pending_record_references = peaks
        .network_pending_record_references
        .max(network.peak_pending_record_references);
    peaks.network_delivered_ledger_entries = peaks
        .network_delivered_ledger_entries
        .max(network.peak_delivered_ledger_entries);
    peaks.network_authoritative_records = peaks
        .network_authoritative_records
        .max(network.peak_retained_authoritative_records);
    peaks.input_bytes = peaks.input_bytes.max(retained.session.input.total_bytes);
    peaks.output_bytes = peaks.output_bytes.max(retained.session.output_bytes);
    peaks.event_bytes = peaks.event_bytes.max(retained.events.total_bytes);
    peaks.history_bytes = peaks.history_bytes.max(retained.history_bytes);
}

fn detect_unconfirmed_floor(state: &mut RollbackLabRunState) {
    let diagnostics = rollback_session::diagnostics(&state.session);
    let input = diagnostics.input_history;
    if input.oldest_retained_tick > diagnostics.confirmed_tick + 1 {
        let missing = diagnostics.confirmed_tick + 1;
        if state
            .unconfirmed_tick
            .is_none_or(|current| missing < current)
        {
            state.unconfirmed_tick = Some(missing);
        }
    }
}

fn compare_snapshots(
    state: &mut RollbackLabRunState,
    expected: &MatchSnapshot,
    actual: &MatchSnapshot,
    boundary: i64,
) {
    let expected_hash = match_snapshot::hash_canonical(expected);
    let actual_hash = match_snapshot::hash_canonical(actual);
    state.compared_boundaries += 1;
    if expected_hash != actual_hash && state.divergence.is_none() {
        state.divergence = Some(RollbackLabDivergence {
            causal_tick: (boundary - 1).max(0),
            boundary,
            expected_hash: expected_hash.clone(),
            actual_hash: actual_hash.clone(),
            first_difference: match_snapshot::first_difference_canonical(expected, actual),
        });
    }
}

fn compare_newly_confirmed(state: &mut RollbackLabRunState) {
    loop {
        let diagnostics = rollback_session::diagnostics(&state.session);
        if state.last_compared_output >= diagnostics.confirmed_output_tick {
            return;
        }
        let output_tick = state.last_compared_output + 1;
        let boundary = output_tick + 1;
        let comparison = rollback_session::compare_retained(
            &mut state.session,
            &mut state.reference_history,
            boundary,
        );
        let retained_or_present =
            |status: rollback_snapshot_history::RollbackSnapshotLookupStatus| {
                matches!(
                    status,
                    rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
                        | rollback_snapshot_history::RollbackSnapshotLookupStatus::Retained
                )
            };
        if !retained_or_present(comparison.expected_status)
            || !retained_or_present(comparison.actual_status)
        {
            state.comparison_history_missing = true;
            return;
        }
        state.compared_boundaries += 1;
        if !comparison.matched && state.divergence.is_none() {
            state.divergence = Some(RollbackLabDivergence {
                causal_tick: (boundary - 1).max(0),
                boundary,
                expected_hash: comparison
                    .expected_hash
                    .expect("a matched-status comparison always hashes"),
                actual_hash: comparison
                    .actual_hash
                    .expect("a matched-status comparison always hashes"),
                first_difference: comparison.first_difference,
            });
        }
        state.last_compared_output = output_tick;
    }
}

fn add_client_authority(
    state: &mut RollbackLabRunState,
    tick: i64,
    slot: i64,
    sample: InputSample,
) {
    match rollback_session::add_authoritative(&mut state.session, tick, slot, sample) {
        Ok(_) => {}
        Err(err) => {
            assert!(
                err.code == rollback_input_history::RollbackInputHistoryErrorCode::OutsideWindow,
                "rollback lab rejected authority with {:?}: {}",
                err.code,
                err.message
            );
            if state.late_input_tick.is_none_or(|current| tick < current) {
                state.late_input_tick = Some(tick);
            }
        }
    }
}

fn process_delivery_batch(state: &mut RollbackLabRunState, deliveries: &[NetworkDelivery]) {
    if deliveries.is_empty() {
        return;
    }
    let before = rollback_session::diagnostics(&state.session).status;
    for delivery in deliveries {
        for record in network_conditions::records(delivery) {
            let confirmed_tick = rollback_session::diagnostics(&state.session).confirmed_tick;
            if record.tick <= confirmed_tick {
                state.confirmed_redundant_rows_skipped += 1;
            } else {
                add_client_authority(state, record.tick, delivery.source_slot, record.sample);
            }
        }
    }
    let reconciled = rollback_session::reconcile(&mut state.session, false);
    if reconciled.changed {
        let depth = reconciled.old_present_boundary
            - reconciled
                .causal_tick
                .expect("a changed reconciliation has a causal tick");
        *state.depth_counts.entry(depth).or_insert(0) += 1;
        let replay_boundary = reconciled
            .replaced_from_tick
            .expect("a changed reconciliation has a replaced-from tick");
        state
            .event_trace
            .push(RollbackLabEventTraceRow::ReplayTruncate {
                boundary: replay_boundary,
            });
        trace_replay_boundary(state, replay_boundary, None);
        apply_client_outputs(
            state,
            replay_boundary,
            reconciled
                .replaced_through_tick
                .expect("a changed reconciliation has a replaced-through tick"),
            reconciled.corrected_outputs,
        );
    }
    if before == rollback_session::RollbackSessionStatus::Finished
        && reconciled.status == rollback_session::RollbackSessionStatus::Active
    {
        state.reactivation_count += 1;
    }
    update_peaks(state);
    detect_unconfirmed_floor(state);
}

fn catch_up_client(state: &mut RollbackLabRunState, target_boundary: i64) {
    loop {
        let diagnostics = rollback_session::diagnostics(&state.session);
        if diagnostics.present_boundary >= target_boundary {
            return;
        }
        if diagnostics.status == rollback_session::RollbackSessionStatus::LateInputUnrecoverable {
            return;
        }
        if diagnostics.status == rollback_session::RollbackSessionStatus::Finished {
            state.predicted_early_finish = true;
            return;
        }
        match rollback_session::step(&mut state.session) {
            Err(err) => {
                assert!(
                    err.code == rollback_session::RollbackSessionErrorCode::MatchFinished
                        || err.code
                            == rollback_session::RollbackSessionErrorCode::LateInputUnrecoverable,
                    "rollback lab client step failed: {}",
                    err.message
                );
                if err.code == rollback_session::RollbackSessionErrorCode::MatchFinished {
                    state.predicted_early_finish = true;
                }
                return;
            }
            Ok(output) => {
                let tick = output.tick;
                apply_client_outputs(state, tick, tick, vec![output]);
                update_peaks(state);
                detect_unconfirmed_floor(state);
            }
        }
    }
}

fn process_drain_deliveries(
    state: &mut RollbackLabRunState,
    deliveries: &[NetworkDelivery],
    reference_tick: i64,
) {
    let mut group: Vec<NetworkDelivery> = Vec::new();
    let mut arrival_tick: Option<i64> = None;
    for delivery in deliveries {
        if let Some(current) = arrival_tick
            && delivery.arrival_tick != current
        {
            process_delivery_batch(state, &group);
            publish_confirmation(state);
            catch_up_client(state, reference_tick);
            publish_confirmation(state);
            compare_newly_confirmed(state);
            group.clear();
        }
        arrival_tick = Some(delivery.arrival_tick);
        group.push(delivery.clone());
    }
    if !group.is_empty() {
        process_delivery_batch(state, &group);
        publish_confirmation(state);
        catch_up_client(state, reference_tick);
        publish_confirmation(state);
        compare_newly_confirmed(state);
    }
}

fn sorted_depths(counts: &IndexMap<i64, i64>) -> Vec<RollbackLabDepth> {
    let mut depths: Vec<i64> = counts.keys().copied().collect();
    depths.sort_unstable();
    depths
        .into_iter()
        .map(|depth| RollbackLabDepth {
            depth,
            count: *counts
                .get(&depth)
                .expect("depth was just collected from counts"),
        })
        .collect()
}

fn drain_requests(
    sources: &[RollbackInputSource; 8],
    last_tick: i64,
) -> Vec<network_conditions::NetworkResendRequest> {
    let mut requests = Vec::new();
    for slot in 1..=input_frame::SLOT_COUNT {
        if sources[(slot - 1) as usize] == RollbackInputSource::Remote {
            requests.push(network_conditions::NetworkResendRequest {
                source_slot: slot,
                input_tick: last_tick,
            });
        }
    }
    requests
}

#[allow(clippy::too_many_lines)]
fn finish_result(
    state: &mut RollbackLabRunState,
    tape: &InputTape,
    profile_name: &str,
    profile: &NetworkProfile,
    network_seed: i64,
    drain: network_conditions::NetworkDrainResult,
) -> RollbackLabResult {
    let session = rollback_session::diagnostics(&state.session);
    let reference_snapshot =
        match_snapshot::capture_owned(&state.reference, state.reference_combat.as_ref());
    let client_snapshot = rollback_session::current_snapshot(&state.session);
    let final_comparison = rollback_session::compare(
        &mut state.session,
        &reference_snapshot,
        state.late_input_tick,
    );
    let events = event_metrics(state);
    let event_diagnostics = rollback_events::diagnostics(&state.events);
    let retained = retained_history::sample(&mut state.session, &state.events);
    let history_accounting = RollbackLabHistoryAccounting {
        input: retained.session.input,
        output_bytes: retained.session.output_bytes,
        snapshot_bytes: retained.session.snapshot_bytes,
        event_bytes: retained.events.total_bytes,
        total_bytes: retained.history_bytes,
    };
    let last_tick = tape.frames.len() as i64 - 1;
    if session.confirmed_tick < last_tick && state.unconfirmed_tick.is_none() {
        state.unconfirmed_tick = Some(session.confirmed_tick + 1);
    }

    let mut status = RollbackLabStatus::Converged;
    if state.late_input_tick.is_some()
        || session.status == rollback_session::RollbackSessionStatus::LateInputUnrecoverable
    {
        status = RollbackLabStatus::LateInputUnrecoverable;
    } else if state.unconfirmed_tick.is_some() {
        status = RollbackLabStatus::UnconfirmedAuthority;
    } else if !drain.complete || drain.pending != 0 {
        status = RollbackLabStatus::DrainIncomplete;
    } else if state.comparison_history_missing {
        status = RollbackLabStatus::ComparisonHistoryMissing;
    } else if state.divergence.is_some() || !final_comparison.matched {
        status = RollbackLabStatus::Diverged;
    } else if !events.matched {
        status = RollbackLabStatus::EventDiverged;
    } else if session.present_boundary != state.reference.input_tick {
        status = RollbackLabStatus::IncompleteClient;
    }

    let expected_boundaries = tape.frames.len() as i64 + 1;
    let mut success = matches!(status, RollbackLabStatus::Converged)
        && session.confirmed_tick == last_tick
        && session.confirmed_output_tick == state.reference.input_tick - 1
        && state.compared_boundaries == expected_boundaries
        && network_conditions::pending(&state.network) == 0
        && events.matched;
    if !success && matches!(status, RollbackLabStatus::Converged) {
        status = RollbackLabStatus::IncompleteClient;
        success = false;
    }

    RollbackLabResult {
        schema: SCHEMA,
        success,
        status,
        fixture: tape.identity.fixture.clone(),
        fixture_seed: tape.identity.seed,
        profile: profile_name.to_string(),
        profile_parameters: profile_parameters(profile),
        network_seed,
        source_pattern: source_pattern(&state.sources),
        input_ticks: tape.frames.len() as i64,
        tape_digest: tape_digest(tape),
        initial_hash: match_snapshot::hash_canonical(&tape.initial),
        reference_final_boundary: state.reference.input_tick,
        client_final_boundary: session.present_boundary,
        reference_final_hash: match_snapshot::hash_canonical(&reference_snapshot),
        client_final_hash: match_snapshot::hash_canonical(&client_snapshot),
        reference_final_snapshot: reference_snapshot,
        client_final_snapshot: client_snapshot,
        confirmed_tick: session.confirmed_tick,
        confirmed_output_tick: session.confirmed_output_tick,
        late_input_tick: state.late_input_tick,
        unconfirmed_tick: state.unconfirmed_tick,
        divergence: state.divergence.clone(),
        metrics: RollbackLabMetrics {
            predicted_slot_samples: session.predicted_slot_samples,
            predicted_ticks: session.predicted_ticks,
            correction_count: session.correction_count,
            rollback_count: session.rollback_count,
            resimulated_ticks: session.resimulated_ticks,
            latest_rollback_depth: session.latest_rollback_depth,
            max_rollback_depth: session.max_rollback_depth,
            rollback_depths: sorted_depths(&state.depth_counts),
            predicted_early_finish: state.predicted_early_finish,
            reactivation_count: state.reactivation_count,
            confirmed_redundant_rows_skipped: state.confirmed_redundant_rows_skipped,
            compared_boundaries: state.compared_boundaries,
            expected_boundaries,
            current_snapshot_count: session.snapshot_history.retained_boundary_count,
            current_snapshot_bytes: session.snapshot_history.canonical_bytes,
            peaks: state.peaks,
        },
        input_diagnostics: session.input_history,
        snapshot_diagnostics: session.snapshot_history,
        network_counters: network_conditions::counters(&state.network),
        network_diagnostics: network_conditions::diagnostics(&state.network),
        event_metrics: events,
        event_diagnostics,
        event_trace: std::mem::take(&mut state.event_trace),
        history_accounting,
        drain: RollbackLabDrainSummary {
            final_tick: drain.final_tick,
            complete: drain.complete,
            pending: drain.pending,
            recovered: drain.recovered,
            requested: drain.requested,
        },
    }
}

fn digest_value_canonical_key(kind: &str, text: &str) -> String {
    format!("{kind}:{text}")
}

fn digest_step(state: &mut Fnv1a64State, step: &RollbackEventStep) {
    // A stable, order-independent digest of one confirmed step: every field
    // folded through `digest_segment`, keyed and sorted in canonical key
    // order, specialized to `RollbackEventStep`'s known shape rather than a
    // dynamic table walk.
    digest_segment(state, &match_snapshot::number_bytes(step.tick as f64));
    digest_segment(
        state,
        &match_snapshot::number_bytes(step.start_boundary as f64),
    );
    digest_segment(
        state,
        &match_snapshot::number_bytes(step.end_boundary as f64),
    );
    digest_segment(
        state,
        &digest_value_canonical_key(
            "home",
            &match_snapshot::number_bytes(step.state.score.home as f64),
        ),
    );
    digest_segment(
        state,
        &digest_value_canonical_key(
            "away",
            &match_snapshot::number_bytes(step.state.score.away as f64),
        ),
    );
    digest_segment(state, &match_snapshot::number_bytes(step.state.time_left));
    digest_segment(state, if step.state.finished { "1" } else { "0" });
    if let Some(id) = &step.state.owner_id {
        digest_segment(state, id);
    }
    digest_segment(state, &format!("{:?}", step.match_events));
    if let Some(combat) = &step.combat_events {
        digest_segment(state, &format!("{combat:?}"));
    }
    digest_segment(state, &format!("{:?}", step.lifecycle_events));
}

fn event_metrics(state: &RollbackLabRunState) -> RollbackLabEventMetrics {
    let mut reference = Fnv1a64State::new();
    let mut confirmed = Fnv1a64State::new();
    let mut metrics = RollbackLabEventMetrics {
        speculative_residue: rollback_events::diagnostics(&state.events).retained_step_count,
        ..Default::default()
    };
    for row in &state.event_trace {
        match row {
            RollbackLabEventTraceRow::ImpairedDiff { diff } => {
                metrics.added += diff.added.len() as i64;
                metrics.revoked += diff.revoked.len() as i64;
                metrics.replaced += diff.replaced.len() as i64;
            }
            RollbackLabEventTraceRow::ReferenceConfirmed { step } => {
                digest_step(&mut reference, step);
            }
            RollbackLabEventTraceRow::ImpairedConfirmed { step } => {
                metrics.confirmed_steps += 1;
                metrics.confirmed_match_events += step.match_events.len() as i64;
                metrics.confirmed_combat_events += step
                    .combat_events
                    .as_ref()
                    .map_or(0, |events| events.len() as i64);
                metrics.confirmed_lifecycle_events += step.lifecycle_events.len() as i64;
                digest_step(&mut confirmed, step);
            }
            _ => {}
        }
    }
    metrics.reference_digest = reference.hex();
    metrics.confirmed_digest = confirmed.hex();
    metrics.matched = metrics.reference_digest == metrics.confirmed_digest
        && metrics.confirmed_steps == state.reference.input_tick
        && metrics.speculative_residue == 0;
    metrics
}

fn copy_sources(supplied: Option<[RollbackInputSource; 8]>) -> [RollbackInputSource; 8] {
    supplied.unwrap_or_else(default_sources)
}

/// Create an incremental authoritative-reference campaign.
///
/// # Panics
///
/// Panics on a malformed `tape` (unless `options.prevalidated_tape` skips
/// full replay validation), an unknown profile name, or an out-of-range
/// corruption tick/slot (producer invariants, ARCHITECTURE.md §3 rule 5).
#[must_use]
pub fn new_campaign(tape: InputTape, options: RollbackLabOptions) -> RollbackLabCampaign {
    let tune = Tuning::new();
    if options.prevalidated_tape {
        input_tape::validate_structure(&tape).expect("rollback lab tape structure is invalid");
    } else {
        input_tape::validate(&tape, &tune).expect("rollback lab tape is invalid");
    }
    assert!(
        !tape.frames.is_empty(),
        "rollback lab tape must contain at least one input frame"
    );
    let sources = copy_sources(options.sources);
    let (profile, profile_name) = selected_profile(&options);
    let network_seed = options.network_seed.unwrap_or(DEFAULT_NETWORK_SEED);
    let max_rollback_ticks = options
        .max_rollback_ticks
        .unwrap_or(rollback_input_history::ROLLBACK_WINDOW_TICKS);
    assert!(
        (0..=rollback_input_history::ROLLBACK_WINDOW_TICKS).contains(&max_rollback_ticks),
        "rollback lab rollback window must be a bounded non-negative integer"
    );
    let drain_ticks = options.drain_ticks.unwrap_or(DEFAULT_DRAIN_TICKS);
    assert!(drain_ticks > 0, "rollback lab drain must be positive");
    if let Some(corruption) = options.corruption {
        assert!(
            (0..tape.frames.len() as i64).contains(&corruption.tick),
            "rollback lab corruption tick is outside the tape"
        );
        assert!(
            (1..=input_frame::SLOT_COUNT).contains(&corruption.slot),
            "rollback lab corruption slot is outside the input frame"
        );
    }

    let (reference, reference_combat) = match_snapshot::restore(&tape.initial);
    let mut reference_history = rollback_snapshot_history::new(Some(max_rollback_ticks));
    rollback_snapshot_history::store(&mut reference_history, &tape.initial)
        .expect("a freshly constructed history always accepts its own boundary zero");
    let session = rollback_session::new(
        &tape.initial,
        sources,
        Some(max_rollback_ticks),
        options.measure,
    );
    // `update_peaks` reads session accounting four times per simulated tick,
    // so re-encoding every retained output on demand dominated this
    // campaign's run time. The incremental total is byte-identical; only the
    // harness opts in.
    let mut session = session;
    rollback_session::track_output_bytes(&mut session);
    let network = network_conditions::new(&profile, network_seed as f64);
    let event_window = max_rollback_ticks.max(1);
    let mut state = RollbackLabRunState {
        session,
        network,
        reference,
        reference_combat,
        reference_history,
        reference_events: rollback_events::new(&tape.initial, Some(event_window)),
        events: rollback_events::new(&tape.initial, Some(event_window)),
        event_trace: Vec::new(),
        event_window_exceeded: false,
        sources,
        depth_counts: IndexMap::new(),
        last_compared_output: -1,
        compared_boundaries: 0,
        divergence: None,
        late_input_tick: None,
        unconfirmed_tick: None,
        comparison_history_missing: false,
        predicted_early_finish: false,
        reactivation_count: 0,
        confirmed_redundant_rows_skipped: 0,
        peaks: RollbackLabPeaks::default(),
    };
    let initial_client_snapshot = rollback_session::current_snapshot(&state.session);
    compare_snapshots(&mut state, &tape.initial, &initial_client_snapshot, 0);
    trace_replay_boundary(&mut state, 0, None);
    update_peaks(&mut state);

    RollbackLabCampaign {
        tape,
        profile,
        profile_name,
        network_seed,
        drain_ticks,
        next_frame_index: 1,
        result: None,
        state,
        observer: options.observer,
    }
}

fn advance_frame(
    campaign: &mut RollbackLabCampaign,
    frame: InputFrame,
    corruption: Option<RollbackLabCorruption>,
) {
    let state = &mut campaign.state;
    let tape = &campaign.tape;
    assert_eq!(
        frame.tick, state.reference.input_tick,
        "rollback lab tape and reference boundary disagree"
    );
    crate::r#match::step(
        &mut state.reference,
        1.0 / tape.identity.tick_rate as f64,
        crate::r#match::StepInput::Frame(&frame),
        state.reference_combat.as_mut(),
        &Tuning::new(),
    );
    let reference_boundary =
        match_snapshot::capture_owned(&state.reference, state.reference_combat.as_ref());
    let reference_hash = match_snapshot::hash_canonical(&reference_boundary);
    let expected_hash = tape
        .boundary_hashes
        .get((frame.tick + 1) as usize)
        .expect("rollback lab frozen recording is missing an authoritative boundary hash");
    assert_eq!(
        &reference_hash,
        expected_hash,
        "rollback lab frozen boundary {} hash mismatch: expected {}, got {}",
        frame.tick + 1,
        expected_hash,
        reference_hash
    );
    rollback_snapshot_history::store_owned(
        &mut state.reference_history,
        reference_boundary.clone(),
    )
    .expect("reference boundary is always within its own retained window");
    publish_reference(state, &frame, &reference_boundary);

    for slot in 1..=input_frame::SLOT_COUNT {
        let index = (slot - 1) as usize;
        let sample = client_sample(corruption, frame.tick, slot, frame.slots[index]);
        if state.sources[index] == RollbackInputSource::Local {
            add_client_authority(state, frame.tick, slot, sample);
        } else {
            network_conditions::send(&mut state.network, frame.tick, slot, frame.tick, &sample)
                .expect("rollback lab network send is always valid");
        }
    }
    update_peaks(state);

    let deliveries = network_conditions::poll(&mut state.network, frame.tick);
    process_delivery_batch(state, &deliveries);
    publish_confirmation(state);
    let reference_tick = state.reference.input_tick;
    catch_up_client(state, reference_tick);
    publish_confirmation(state);
    compare_newly_confirmed(state);
    update_peaks(state);
    detect_unconfirmed_floor(state);
}

fn finish_campaign(campaign: &mut RollbackLabCampaign) -> RollbackLabResult {
    let last_tick = campaign.tape.frames.len() as i64 - 1;
    let sources = campaign.state.sources;
    let drain = network_conditions::drain(
        &mut campaign.state.network,
        last_tick + 1,
        campaign.drain_ticks,
        &drain_requests(&sources, last_tick),
    )
    .expect("rollback lab drain request set is always valid");
    update_peaks(&mut campaign.state);
    let reference_tick = campaign.state.reference.input_tick;
    process_drain_deliveries(&mut campaign.state, &drain.deliveries, reference_tick);
    catch_up_client(&mut campaign.state, reference_tick);
    publish_confirmation(&mut campaign.state);
    compare_newly_confirmed(&mut campaign.state);
    update_peaks(&mut campaign.state);
    detect_unconfirmed_floor(&mut campaign.state);

    finish_result(
        &mut campaign.state,
        &campaign.tape,
        &campaign.profile_name,
        &campaign.profile,
        campaign.network_seed,
        drain,
    )
}

/// Advance at most `max_ticks` authoritative input rows. The final bounded
/// drain is performed only after the final row, returning the immutable
/// result from then on.
///
/// # Panics
///
/// Panics if `max_ticks` is not positive, or on a frozen recording hash
/// mismatch (producer invariants, ARCHITECTURE.md §3 rule 5).
pub fn step_campaign(
    campaign: &mut RollbackLabCampaign,
    max_ticks: i64,
    corruption: Option<RollbackLabCorruption>,
) -> Option<&RollbackLabResult> {
    assert!(
        max_ticks > 0,
        "rollback lab campaign step must be a positive integer"
    );
    if campaign.result.is_some() {
        return campaign.result.as_ref();
    }
    let last_index =
        (campaign.tape.frames.len() as i64).min(campaign.next_frame_index + max_ticks - 1);
    let mut index = campaign.next_frame_index;
    while index <= last_index {
        let frame = campaign.tape.frames[(index - 1) as usize];
        advance_frame(campaign, frame, corruption);
        // `.take()`/put-back rather than a direct disjoint-field borrow:
        // the observer is `FnMut`, so calling it needs `&mut
        // campaign.observer` live at the same time as the `&campaign.state`
        // argument, and the closure body's own capture is opaque to the
        // borrow checker.
        if let Some(mut observer) = campaign.observer.take() {
            observer(frame.tick, &campaign.state);
            campaign.observer = Some(observer);
        }
        index += 1;
    }
    campaign.next_frame_index = last_index + 1;
    if campaign.next_frame_index > campaign.tape.frames.len() as i64 {
        campaign.result = Some(finish_campaign(campaign));
    }
    campaign.result.as_ref()
}

/// Exercise the terminal session seams and prove that an over-window
/// failure cannot advance state or logical counters after it has been
/// reported.
///
/// # Panics
///
/// Panics if `campaign` has no result yet, or its result is not
/// `LateInputUnrecoverable` (producer invariants, ARCHITECTURE.md §3 rule 5).
pub fn probe_terminal_stability(campaign: &mut RollbackLabCampaign) -> bool {
    {
        let result = campaign
            .result
            .as_ref()
            .expect("rollback lab terminal probe requires a result");
        assert!(
            matches!(result.status, RollbackLabStatus::LateInputUnrecoverable),
            "rollback lab terminal probe requires an over-window result"
        );
    }
    let session = &mut campaign.state.session;
    let before = rollback_session::diagnostics(session);
    let before_snapshot = rollback_session::current_snapshot(session);
    let output = rollback_session::step(session);
    assert!(
        output.is_err()
            && output.unwrap_err().code
                == rollback_session::RollbackSessionErrorCode::LateInputUnrecoverable
    );
    let reconciled = rollback_session::reconcile(session, false);
    assert!(
        !reconciled.changed
            && reconciled.status == rollback_session::RollbackSessionStatus::LateInputUnrecoverable
    );
    let after = rollback_session::diagnostics(session);
    let after_snapshot = rollback_session::current_snapshot(session);
    match_snapshot::first_difference_canonical(&before_snapshot, &after_snapshot).is_some()
        || before.status != after.status
        || before.present_boundary != after.present_boundary
        || before.confirmed_tick != after.confirmed_tick
        || before.confirmed_output_tick != after.confirmed_output_tick
        || before.rollback_count != after.rollback_count
        || before.correction_count != after.correction_count
        || before.predicted_slot_samples != after.predicted_slot_samples
        || before.predicted_ticks != after.predicted_ticks
        || before.resimulated_ticks != after.resimulated_ticks
        || before.late_window_failures != after.late_window_failures
}

/// Run a validated, already-materialized authoritative tape against an
/// independent reference and one impaired rollback client. `options.measure`
/// is observational only (see the module doc comment) and cannot enter the
/// logical result.
#[must_use]
pub fn run(tape: InputTape, options: RollbackLabOptions) -> RollbackLabResult {
    let corruption = options.corruption;
    let frame_count = tape.frames.len() as i64;
    let mut campaign = new_campaign(tape, options);
    loop {
        if let Some(result) = step_campaign(&mut campaign, frame_count, corruption) {
            return result.clone();
        }
    }
}
