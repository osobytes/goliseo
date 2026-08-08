//! Port of `sim/rollback_playable_lab.lua`.
//!
//! Pure incremental rollback laboratory for the playable match screen. One
//! render-owned fixed clock supplies transport ticks; this controller owns
//! the independent reference, predicted client, network impairment, and
//! stable presentation-event timeline.
//!
//! ## Value types replace the Lua `copy_value` helper
//!
//! The Lua original threads a generic `copy_value` through every batch and
//! debug-model read to defend a caller from mutating (or aliasing) a table
//! this module still owns. Every payload type here
//! ([`RollbackPlayableLabBatch`], [`RollbackPlayableLabDebugModel`],
//! [`crate::rollback_session::RollbackTickOutput`],
//! [`crate::rollback_events::RollbackEventDiff`], ...) is an owned Rust
//! value with a real [`Clone`] impl, so a `.clone()` at a read boundary
//! already gives the independent copy `copy_value` exists to fake — matching
//! the precedent in `rollback_session`, `rollback_input_history`, and
//! `rollback_events`.
//!
//! ## Profile name lookup has no Rust-side string table
//!
//! `data/network_profiles.lua` is a string-keyed table
//! (`"clean"`/`"omp0_parity"`/`"playable"`/`"stress"`); `gc_data::network_profiles`
//! exposes the same four rows through a closed `NetworkProfileName` enum
//! with no string lookup. [`profile_by_name`] is this module's own narrow
//! adapter between the two — a string is still the public surface here
//! (`RollbackPlayableLabOptions.profile_name`) because a caller-supplied
//! custom `profile` is still named by an arbitrary string for diagnostics.
//!
//!
//! The independent reference state this module steps directly (not through
//! `RollbackSession`) needs the same `marks` padding
//! representational gap between a fixed-length Rust `Vec` and Lua's sparse,
//! implicitly-nil table, not a simulation difference. This module reuses
//! that helper (`pub(crate)`) rather than keeping a second copy.

use crate::input_frame::{self, InputSample};
use crate::match_snapshot::{self, MatchSnapshot, MatchSnapshotDifference};
use crate::network_conditions::{self, NetworkConditions, NetworkProfile};
use crate::rollback_events::{self, RollbackEventDiff, RollbackEventStep, RollbackEventTimeline};
use crate::rollback_input_history::{self, RollbackInputSource};
use crate::rollback_session::{self, RollbackSession, RollbackTickOutput};
use crate::rollback_snapshot_history::{self, RollbackSnapshotHistory};
use crate::slot_input::{self, MatchSlotSource, MatchSlotSourceKind, SlotInputProducerState};

/// A [`RollbackPlayableLab`]'s lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackPlayableLabStatus {
    /// Actively consuming local samples and reference frames.
    Active,
    /// The reference has finished; draining remaining network settlement.
    Settling,
    /// Settlement finished and the client matched the reference.
    Converged,
    /// A retained comparison mismatched.
    Diverged,
    /// A correction was rejected as outside the retained window.
    LateInputUnrecoverable,
    /// The stable-event timeline stalled beyond its unconfirmed window.
    UnconfirmedWindowExceeded,
    /// A comparison boundary was neither retained nor present on one side.
    ComparisonHistoryMissing,
    /// Settlement ran out its bounded budget without confirming.
    DrainIncomplete,
}

/// A [`RollbackPlayableConvergence`]'s status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackPlayableConvergenceStatus {
    /// The most recent comparison matched (also construction's default).
    Matched,
    /// The most recent comparison mismatched.
    Diverged,
}

/// Construction options for [`new`].
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RollbackPlayableLabOptions {
    /// The canonical slot the local player occupies; defaults to `1`.
    pub local_slot: Option<i64>,
    /// A named authored profile; defaults to `"playable"`.
    pub profile_name: Option<String>,
    /// A caller-supplied profile, overriding the authored lookup (but not
    /// `profile_name`, which still labels it for diagnostics).
    pub profile: Option<NetworkProfile>,
    /// Network impairment RNG seed; defaults to [`DEFAULT_NETWORK_SEED`].
    pub network_seed: Option<i64>,
    /// Declared-bot RNG seed base; defaults to [`DEFAULT_BOT_SEED`].
    pub bot_seed: Option<i64>,
    /// Maximum retained rollback ticks; defaults to
    /// [`crate::rollback_input_history::ROLLBACK_WINDOW_TICKS`].
    pub max_rollback_ticks: Option<i64>,
    /// Bounded post-finish settlement budget, in transport ticks; defaults
    /// to [`DEFAULT_SETTLEMENT_TICKS`].
    pub settlement_ticks: Option<i64>,
}

/// One reconciliation this lab observed.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackPlayableCorrection {
    /// The earliest divergent tick this correction reconciled.
    pub causal_tick: i64,
    /// The boundary restored from.
    pub restored_boundary: i64,
    /// The present boundary before this correction.
    pub old_present_boundary: i64,
    /// The present boundary after this correction.
    pub new_present_boundary: i64,
    /// The first tick whose retained output is no longer valid.
    pub replaced_from_tick: i64,
    /// The last tick whose retained output is no longer valid.
    pub replaced_through_tick: i64,
    /// The first tick whose output was resimulated.
    pub corrected_from_tick: i64,
    /// The last tick whose output was resimulated.
    pub corrected_through_tick: i64,
    /// The predicted boundary's hash before restoring.
    pub old_present_hash: Option<String>,
    /// The corrected boundary's hash after resimulating.
    pub new_present_hash: Option<String>,
    /// The first structural disagreement between the old and new present
    /// boundary, present only when both hashes were computed and differ.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// The most recent retained-boundary comparison this lab observed.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackPlayableConvergence {
    /// This comparison's status.
    pub status: RollbackPlayableConvergenceStatus,
    /// The compared boundary tick.
    pub boundary: i64,
    /// The reference's hash at this boundary.
    pub expected_hash: String,
    /// The client's hash at this boundary.
    pub actual_hash: String,
    /// The first structural disagreement, present exactly on a mismatch.
    pub first_difference: Option<MatchSnapshotDifference>,
}

/// The outcome of one [`advance`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackPlayableLabBatch {
    /// Freshly predicted outputs from this tick's `catch_up_client` pass.
    pub outputs: Vec<RollbackTickOutput>,
    /// Stable-event diffs this tick's output applications produced.
    pub event_diffs: Vec<RollbackEventDiff>,
    /// Newly confirmed stable-event steps, in causal order.
    pub confirmed_steps: Vec<RollbackEventStep>,
    /// Corrections this tick's reconciliation performed.
    pub corrections: Vec<RollbackPlayableCorrection>,
    /// The lab's status after this call.
    pub status: RollbackPlayableLabStatus,
}

/// A snapshot of a [`RollbackPlayableLab`]'s retained-state shape.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackPlayableLabDebugModel {
    /// The active network profile's name.
    pub profile: String,
    /// The canonical slot the local player occupies.
    pub local_slot: i64,
    /// Lifecycle status.
    pub status: RollbackPlayableLabStatus,
    /// The independent reference's current input tick.
    pub reference_tick: i64,
    /// The client session's present boundary.
    pub current_tick: i64,
    /// The most recent transport tick advanced.
    pub transport_tick: i64,
    /// Monotonic input-authority boundary.
    pub confirmed_input_tick: i64,
    /// Confirmed input capped to outputs that exist.
    pub confirmed_output_tick: i64,
    /// Cumulative predicted slot executions, including resimulation.
    pub predicted_slot_samples: i64,
    /// Cumulative tick executions with at least one predicted slot.
    pub predicted_ticks: i64,
    /// Total newly-inserted authoritative samples that differed from an
    /// already-consumed prediction.
    pub correction_count: i64,
    /// Total completed reconciliations that changed state.
    pub rollback_count: i64,
    /// The most recent rollback's depth.
    pub latest_rollback_depth: i64,
    /// The largest rollback depth seen so far.
    pub max_rollback_depth: i64,
    /// Total ticks resimulated across every rollback.
    pub resimulated_ticks: i64,
    /// Retained client snapshot count.
    pub retained_snapshot_count: i64,
    /// Retained client snapshot bytes.
    pub retained_snapshot_bytes: i64,
    /// Currently pending (in-flight) network envelopes.
    pub network_pending: i64,
    /// Running network counters.
    pub network_counters: network_conditions::NetworkConditionCounters,
    /// Current and peak retained network state.
    pub network_high_water: network_conditions::NetworkConditionDiagnostics,
    /// The most recent retained-boundary comparison.
    pub convergence: RollbackPlayableConvergence,
    /// The stable-event timeline's lifecycle status.
    pub event_status: rollback_events::RollbackEventsStatus,
    /// Whether the client predicted full time before the reference reached
    /// it.
    pub predicted_early_finish: bool,
    /// The tick a correction was rejected as outside the retained window,
    /// if any.
    pub late_input_tick: Option<i64>,
    /// Bounded post-finish settlement transport ticks elapsed so far.
    pub settlement_ticks: i64,
    /// The bounded post-finish settlement budget.
    pub settlement_limit: i64,
}

/// The playable rollback controller for one live match. Every field is
/// internal state; use the free functions in this module to read or mutate
/// it. Fields are `pub` (README rule: everything a test touches is `pub`).
pub struct RollbackPlayableLab {
    /// The independent reference soccer state.
    pub reference: match_snapshot::MatchState,
    /// The independent reference's combat companion.
    pub reference_combat: Option<crate::combat_snapshot::CombatMatchState>,
    /// The independent reference's retained snapshot ring.
    pub reference_history: RollbackSnapshotHistory,
    /// The predicted client rollback session.
    pub session: RollbackSession,
    /// The stable presentation-event timeline.
    pub events: RollbackEventTimeline,
    /// Simulated network impairment between the reference and client.
    pub network: NetworkConditions,
    /// The declared-bot input producer for every non-local slot.
    pub producer: SlotInputProducerState,
    /// Canonical eight-slot local/remote ownership.
    pub sources: [RollbackInputSource; 8],
    /// The canonical slot the local player occupies.
    pub local_slot: i64,
    /// The active network profile's name.
    pub profile_name: String,
    /// The most recent transport tick advanced.
    pub transport_tick: i64,
    /// Lifecycle status.
    pub status: RollbackPlayableLabStatus,
    /// The last output tick a comparison ran for, or `-1`.
    pub last_compared_output: i64,
    /// The most recent retained-boundary comparison.
    pub latest_convergence: RollbackPlayableConvergence,
    /// Whether the client predicted full time before the reference reached
    /// it.
    pub predicted_early_finish: bool,
    /// The tick a correction was rejected as outside the retained window,
    /// if any.
    pub late_input_tick: Option<i64>,
    /// Bounded post-finish settlement transport ticks elapsed so far.
    pub settlement_ticks: i64,
    /// The bounded post-finish settlement budget.
    pub settlement_limit: i64,
    /// The most recent reference input tick produced, if any.
    pub last_reference_input_tick: Option<i64>,
}

/// Default network impairment RNG seed.
pub const DEFAULT_NETWORK_SEED: i64 = 7302;
/// Default declared-bot RNG seed base.
pub const DEFAULT_BOT_SEED: i64 = 7400;
/// Default bounded post-finish settlement budget, in transport ticks.
pub const DEFAULT_SETTLEMENT_TICKS: i64 = 256;

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

fn selected_profile(options: &RollbackPlayableLabOptions) -> (NetworkProfile, String) {
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
        .unwrap_or_else(|| "playable".to_string());
    let profile = profile_by_name(&name)
        .unwrap_or_else(|| panic!("unknown playable rollback profile: {name}"));
    (profile, name)
}

fn controlled_initial_snapshot(initial_snapshot: &MatchSnapshot, local_slot: i64) -> MatchSnapshot {
    let (mut state, combat_state) = match_snapshot::restore_owned(initial_snapshot);
    assert!(
        state.slot_mode,
        "playable rollback lab requires a slot-mode match"
    );
    assert!(
        state.input_tick == 0,
        "playable rollback lab requires boundary zero"
    );
    assert!(
        !state.finished,
        "playable rollback lab requires an active initial match"
    );
    state.controlled =
        state.slot_players[(local_slot - 1) as usize].expect("local rollback slot is unmapped");
    // (unlike this module's own `_owned`/`_canonical` calls) restores
    // through the validating `match_snapshot::restore`, which requires
    // `marks` sized to the roster immediately — a boundary zero this fresh
    // legitimately is not yet.
    match_snapshot::capture_owned(&state, combat_state.as_ref())
}

fn new_producer(local_slot: i64, bot_seed: i64) -> SlotInputProducerState {
    let mut sources = [MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: None,
    }; 8];
    for slot in 1..=input_frame::SLOT_COUNT {
        let index = (slot - 1) as usize;
        sources[index] = if slot == local_slot {
            MatchSlotSource {
                kind: MatchSlotSourceKind::Frame,
                seed: None,
            }
        } else {
            MatchSlotSource {
                kind: MatchSlotSourceKind::Bot,
                seed: Some((bot_seed + slot * 97) as f64),
            }
        };
    }
    slot_input::new_producer(sources)
}

fn new_sources(local_slot: i64) -> [RollbackInputSource; 8] {
    let mut sources = [RollbackInputSource::Remote; 8];
    sources[(local_slot - 1) as usize] = RollbackInputSource::Local;
    sources
}

fn terminate(lab: &mut RollbackPlayableLab, status: RollbackPlayableLabStatus) {
    if lab.status == RollbackPlayableLabStatus::Active
        || lab.status == RollbackPlayableLabStatus::Settling
    {
        lab.status = status;
    }
}

fn add_authority(lab: &mut RollbackPlayableLab, tick: i64, slot: i64, sample: InputSample) {
    match rollback_session::add_authoritative(&mut lab.session, tick, slot, sample) {
        Ok(_) => {}
        Err(err) => {
            assert!(
                err.code == rollback_input_history::RollbackInputHistoryErrorCode::OutsideWindow,
                "playable rollback lab rejected authority with {:?}: {}",
                err.code,
                err.message
            );
            if lab.late_input_tick.is_none_or(|current| tick < current) {
                lab.late_input_tick = Some(tick);
            }
            terminate(lab, RollbackPlayableLabStatus::LateInputUnrecoverable);
        }
    }
}

/// Narrow `rollback_session::RollbackTickOutput` down to
/// `rollback_events::RollbackEventTickOutput` — the adapter that module's
/// own doc comment describes, since it was ported before this module's real
/// type existed and reads only these fields.
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
    lab: &RollbackPlayableLab,
    output: RollbackTickOutput,
) -> rollback_events::RollbackEventStepInput {
    let lookup = rollback_session::snapshot(&lab.session, output.end_boundary);
    assert!(
        matches!(
            lookup.status,
            rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
                | rollback_snapshot_history::RollbackSnapshotLookupStatus::Retained
        ),
        "rollback event output boundary is not retained"
    );
    rollback_events::RollbackEventStepInput {
        output: event_tick_output(&output),
        snapshot: lookup
            .snapshot
            .expect("rollback event output snapshot is missing"),
    }
}

fn apply_outputs(
    lab: &mut RollbackPlayableLab,
    batch: &mut RollbackPlayableLabBatch,
    from_tick: i64,
    through_tick: i64,
    outputs: Vec<RollbackTickOutput>,
) -> bool {
    let steps: Vec<rollback_events::RollbackEventStepInput> = outputs
        .into_iter()
        .map(|output| event_step(lab, output))
        .collect();
    match rollback_events::apply(&mut lab.events, from_tick, through_tick, &steps) {
        Ok(diff) => {
            batch.event_diffs.push(diff);
            true
        }
        Err(err) => {
            assert!(
                err.code == rollback_events::RollbackEventsErrorCode::UnconfirmedWindowExceeded,
                "playable rollback event application failed: {}",
                err.message
            );
            terminate(lab, RollbackPlayableLabStatus::UnconfirmedWindowExceeded);
            false
        }
    }
}

fn publish_confirmation(
    lab: &mut RollbackPlayableLab,
    batch: &mut RollbackPlayableLabBatch,
) -> bool {
    if rollback_events::diagnostics(&lab.events).status
        != rollback_events::RollbackEventsStatus::Active
    {
        terminate(lab, RollbackPlayableLabStatus::UnconfirmedWindowExceeded);
        return false;
    }
    let confirmed_output = rollback_session::diagnostics(&lab.session).confirmed_output_tick;
    batch
        .confirmed_steps
        .extend(rollback_events::confirm(&mut lab.events, confirmed_output));
    true
}

fn compare_boundary(
    lab: &mut RollbackPlayableLab,
    expected: &MatchSnapshot,
    actual: &MatchSnapshot,
    boundary: i64,
) {
    let expected_hash = match_snapshot::hash_canonical(expected);
    let actual_hash = match_snapshot::hash_canonical(actual);
    let matched = expected_hash == actual_hash;
    lab.latest_convergence = RollbackPlayableConvergence {
        status: if matched {
            RollbackPlayableConvergenceStatus::Matched
        } else {
            RollbackPlayableConvergenceStatus::Diverged
        },
        boundary,
        expected_hash,
        actual_hash,
        first_difference: if matched {
            None
        } else {
            match_snapshot::first_difference_canonical(expected, actual)
        },
    };
    if !matched {
        terminate(lab, RollbackPlayableLabStatus::Diverged);
    }
}

fn compare_newly_confirmed(lab: &mut RollbackPlayableLab) {
    let confirmed = rollback_session::diagnostics(&lab.session).confirmed_output_tick;
    while lab.last_compared_output < confirmed {
        let output_tick = lab.last_compared_output + 1;
        let boundary = output_tick + 1;
        let expected = rollback_snapshot_history::lookup(&lab.reference_history, boundary);
        let actual = rollback_session::snapshot(&lab.session, boundary);
        let retained_or_present =
            |status: rollback_snapshot_history::RollbackSnapshotLookupStatus| {
                matches!(
                    status,
                    rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
                        | rollback_snapshot_history::RollbackSnapshotLookupStatus::Retained
                )
            };
        if !retained_or_present(expected.status) || !retained_or_present(actual.status) {
            terminate(lab, RollbackPlayableLabStatus::ComparisonHistoryMissing);
            return;
        }
        let expected_snapshot = expected
            .snapshot
            .expect("reference comparison snapshot is missing");
        let actual_snapshot = actual
            .snapshot
            .expect("client comparison snapshot is missing");
        compare_boundary(lab, &expected_snapshot, &actual_snapshot, boundary);
        lab.last_compared_output = output_tick;
        if lab.status == RollbackPlayableLabStatus::Diverged {
            return;
        }
    }
}

fn correction_view(
    result: &rollback_session::RollbackReconcileResult,
) -> RollbackPlayableCorrection {
    RollbackPlayableCorrection {
        causal_tick: result
            .causal_tick
            .expect("changed reconciliation always has a causal tick"),
        restored_boundary: result
            .restored_boundary
            .expect("changed reconciliation always has a restored boundary"),
        old_present_boundary: result.old_present_boundary,
        new_present_boundary: result.new_present_boundary,
        replaced_from_tick: result
            .replaced_from_tick
            .expect("changed reconciliation always has a replaced-from tick"),
        replaced_through_tick: result
            .replaced_through_tick
            .expect("changed reconciliation always has a replaced-through tick"),
        corrected_from_tick: result
            .corrected_from_tick
            .expect("changed reconciliation always has a corrected-from tick"),
        corrected_through_tick: result
            .corrected_through_tick
            .expect("changed reconciliation always has a corrected-through tick"),
        old_present_hash: result.old_present_hash.clone(),
        new_present_hash: result.new_present_hash.clone(),
        first_difference: result.first_difference.clone(),
    }
}

fn process_deliveries(
    lab: &mut RollbackPlayableLab,
    batch: &mut RollbackPlayableLabBatch,
    deliveries: &[network_conditions::NetworkDelivery],
) -> bool {
    for delivery in deliveries {
        for record in network_conditions::records(delivery) {
            let confirmed = rollback_session::diagnostics(&lab.session).confirmed_tick;
            if record.tick > confirmed {
                add_authority(lab, record.tick, delivery.source_slot, record.sample);
            }
        }
    }
    let reconciled = rollback_session::reconcile(&mut lab.session, true);
    if reconciled.status == rollback_session::RollbackSessionStatus::LateInputUnrecoverable {
        if lab.late_input_tick.is_none() {
            lab.late_input_tick = reconciled.causal_tick;
        }
        terminate(lab, RollbackPlayableLabStatus::LateInputUnrecoverable);
        return false;
    }
    if !reconciled.changed {
        return lab.status == RollbackPlayableLabStatus::Active
            || lab.status == RollbackPlayableLabStatus::Settling;
    }
    assert!(
        !reconciled.corrected_outputs.is_empty(),
        "changed rollback must return corrected outputs"
    );
    let from_tick = reconciled
        .replaced_from_tick
        .expect("changed reconciliation always has a replaced-from tick");
    let through_tick = reconciled
        .replaced_through_tick
        .expect("changed reconciliation always has a replaced-through tick");
    let correction = correction_view(&reconciled);
    if !apply_outputs(
        lab,
        batch,
        from_tick,
        through_tick,
        reconciled.corrected_outputs,
    ) {
        return false;
    }
    batch.corrections.push(correction);
    if reconciled.status == rollback_session::RollbackSessionStatus::Active
        && lab.predicted_early_finish
    {
        lab.predicted_early_finish = false;
    }
    true
}

fn catch_up_client(
    lab: &mut RollbackPlayableLab,
    batch: &mut RollbackPlayableLabBatch,
    target_boundary: i64,
) -> bool {
    loop {
        let diagnostics = rollback_session::diagnostics(&lab.session);
        if diagnostics.present_boundary >= target_boundary {
            return true;
        }
        if diagnostics.status == rollback_session::RollbackSessionStatus::LateInputUnrecoverable {
            terminate(lab, RollbackPlayableLabStatus::LateInputUnrecoverable);
            return false;
        }
        if diagnostics.status == rollback_session::RollbackSessionStatus::Finished {
            lab.predicted_early_finish = true;
            return true;
        }
        match rollback_session::step(&mut lab.session) {
            Err(err) => {
                if err.code == rollback_session::RollbackSessionErrorCode::MatchFinished {
                    lab.predicted_early_finish = true;
                    return true;
                }
                assert!(
                    err.code == rollback_session::RollbackSessionErrorCode::LateInputUnrecoverable,
                    "rollback client step failed: {}",
                    err.message
                );
                terminate(lab, RollbackPlayableLabStatus::LateInputUnrecoverable);
                return false;
            }
            Ok(output) => {
                batch.outputs.push(output.clone());
                if !apply_outputs(lab, batch, output.tick, output.tick, vec![output]) {
                    return false;
                }
            }
        }
    }
}

fn reconcile_and_publish(
    lab: &mut RollbackPlayableLab,
    batch: &mut RollbackPlayableLabBatch,
) -> bool {
    let deliveries = network_conditions::poll(&mut lab.network, lab.transport_tick);
    if !process_deliveries(lab, batch, &deliveries) {
        return false;
    }
    // Confirmation immediately after a delivery/correction frees the oldest
    // event slot before the next predicted output is appended. At a
    // 30-tick delay this is the difference between a supported correction
    // and a false unconfirmed-window terminal.
    if !publish_confirmation(lab, batch) {
        return false;
    }
    if !catch_up_client(lab, batch, lab.reference.input_tick) {
        return false;
    }
    if !publish_confirmation(lab, batch) {
        return false;
    }
    compare_newly_confirmed(lab);
    lab.status == RollbackPlayableLabStatus::Active
        || lab.status == RollbackPlayableLabStatus::Settling
}

fn advance_reference(lab: &mut RollbackPlayableLab, local_sample: InputSample) {
    assert_eq!(
        lab.reference.input_tick, lab.transport_tick,
        "reference input tick and transport tick must stay aligned"
    );
    let mut slots = [input_frame::neutral_sample(); 8];
    slots[(lab.local_slot - 1) as usize] = local_sample;
    let base =
        input_frame::new(lab.transport_tick, Some(slots)).expect("canonical slots always validate");
    let (frame, _decisions) = slot_input::materialize(
        &mut lab.producer,
        &lab.reference,
        &base,
        lab.reference_combat.as_ref(),
    );
    crate::r#match::step(
        &mut lab.reference,
        crate::fixed_clock::TICK_SECONDS,
        crate::r#match::StepInput::Frame(&frame),
        lab.reference_combat.as_mut(),
        &crate::tuning::Tuning::new(),
    );
    let boundary = match_snapshot::capture_owned(&lab.reference, lab.reference_combat.as_ref());
    rollback_snapshot_history::store_owned(&mut lab.reference_history, boundary)
        .expect("reference boundary is always within its own retained window");

    add_authority(
        lab,
        frame.tick,
        lab.local_slot,
        frame.slots[(lab.local_slot - 1) as usize],
    );
    for slot in 1..=input_frame::SLOT_COUNT {
        if slot != lab.local_slot {
            network_conditions::send(
                &mut lab.network,
                lab.transport_tick,
                slot,
                frame.tick,
                &frame.slots[(slot - 1) as usize],
            )
            .expect("playable rollback network send is always valid");
        }
    }
    lab.last_reference_input_tick = Some(frame.tick);
}

fn resend_final_remote_rows(lab: &mut RollbackPlayableLab) {
    let last_tick = lab
        .last_reference_input_tick
        .expect("rollback settlement has no final reference input");
    if rollback_session::diagnostics(&lab.session).confirmed_tick >= last_tick {
        return;
    }
    for slot in 1..=input_frame::SLOT_COUNT {
        if slot != lab.local_slot {
            network_conditions::resend(&mut lab.network, lab.transport_tick, slot, last_tick)
                .expect("playable rollback resend is always valid");
        }
    }
}

fn finish_settlement_if_ready(lab: &mut RollbackPlayableLab) {
    let final_tick = lab
        .last_reference_input_tick
        .expect("settlement requires a final reference tick");
    let session = rollback_session::diagnostics(&lab.session);
    if session.confirmed_tick < final_tick
        || session.confirmed_output_tick < final_tick
        || network_conditions::pending(&lab.network) > 0
    {
        return;
    }
    let expected = match_snapshot::capture_owned(&lab.reference, lab.reference_combat.as_ref());
    let comparison = rollback_session::compare(&mut lab.session, &expected, Some(final_tick));
    lab.latest_convergence = RollbackPlayableConvergence {
        status: if comparison.matched {
            RollbackPlayableConvergenceStatus::Matched
        } else {
            RollbackPlayableConvergenceStatus::Diverged
        },
        boundary: comparison.expected_boundary,
        expected_hash: comparison.expected_hash,
        actual_hash: comparison.actual_hash,
        first_difference: comparison.first_difference,
    };
    lab.status = if comparison.matched {
        RollbackPlayableLabStatus::Converged
    } else {
        RollbackPlayableLabStatus::Diverged
    };
}

/// Construct a fresh playable rollback controller. `initial_snapshot` must
/// be the canonical slot-mode boundary-zero snapshot.
///
/// # Panics
///
/// Panics on a malformed `initial_snapshot`, an out-of-range `local_slot`,
/// or an unknown `profile_name` (producer invariants, README rule 5.5).
#[must_use]
pub fn new(
    initial_snapshot: &MatchSnapshot,
    options: RollbackPlayableLabOptions,
) -> RollbackPlayableLab {
    let local_slot = options.local_slot.unwrap_or(1);
    assert!(
        (1..=input_frame::SLOT_COUNT).contains(&local_slot),
        "playable rollback local slot must be between one and eight"
    );
    let network_seed = options.network_seed.unwrap_or(DEFAULT_NETWORK_SEED);
    let bot_seed = options.bot_seed.unwrap_or(DEFAULT_BOT_SEED);
    let maximum = options
        .max_rollback_ticks
        .unwrap_or(rollback_input_history::ROLLBACK_WINDOW_TICKS);
    assert!(
        (1..=rollback_input_history::ROLLBACK_WINDOW_TICKS).contains(&maximum),
        "playable rollback window must be a positive bounded integer"
    );
    let settlement = options.settlement_ticks.unwrap_or(DEFAULT_SETTLEMENT_TICKS);
    assert!(
        settlement >= 1,
        "playable rollback settlement must be a positive integer"
    );
    let (profile, profile_name) = selected_profile(&options);
    let canonical = controlled_initial_snapshot(initial_snapshot, local_slot);
    let (reference, reference_combat) = match_snapshot::restore_owned(&canonical);
    let mut reference_history = rollback_snapshot_history::new(Some(maximum));
    rollback_snapshot_history::store_owned(&mut reference_history, canonical.clone())
        .expect("a freshly constructed history always accepts its own boundary zero");
    let sources = new_sources(local_slot);
    let session = rollback_session::new(&canonical, sources, Some(maximum), None);
    let initial_hash = match_snapshot::hash_canonical(&canonical);
    RollbackPlayableLab {
        reference,
        reference_combat,
        reference_history,
        session,
        events: rollback_events::new(&canonical, Some(maximum)),
        network: network_conditions::new(&profile, network_seed as f64),
        producer: new_producer(local_slot, bot_seed),
        sources,
        local_slot,
        profile_name,
        transport_tick: 0,
        status: RollbackPlayableLabStatus::Active,
        last_compared_output: -1,
        latest_convergence: RollbackPlayableConvergence {
            status: RollbackPlayableConvergenceStatus::Matched,
            boundary: 0,
            expected_hash: initial_hash.clone(),
            actual_hash: initial_hash,
            first_difference: None,
        },
        predicted_early_finish: false,
        late_input_tick: None,
        settlement_ticks: 0,
        settlement_limit: settlement,
        last_reference_input_tick: None,
    }
}

/// Whether the caller must supply a local sample to [`advance`] this tick.
#[must_use]
pub fn needs_local_sample(lab: &RollbackPlayableLab) -> bool {
    lab.status == RollbackPlayableLabStatus::Active && !lab.reference.finished
}

/// Advance exactly one transport tick. During active play this consumes one
/// local sample and one complete reference `InputFrame`. After reference
/// full time it advances only network settlement and never invents another
/// frame.
///
/// # Panics
///
/// Panics if `transport_tick` is not contiguous, if a local sample is
/// missing while active, or if one is supplied during settlement
/// (producer invariants, README rule 5.5).
pub fn advance(
    lab: &mut RollbackPlayableLab,
    transport_tick: i64,
    local_sample: Option<InputSample>,
) -> RollbackPlayableLabBatch {
    assert_eq!(
        transport_tick, lab.transport_tick,
        "playable rollback transport tick is not contiguous"
    );
    let mut batch = RollbackPlayableLabBatch {
        outputs: Vec::new(),
        event_diffs: Vec::new(),
        confirmed_steps: Vec::new(),
        corrections: Vec::new(),
        status: RollbackPlayableLabStatus::Active,
    };
    if lab.status != RollbackPlayableLabStatus::Active
        && lab.status != RollbackPlayableLabStatus::Settling
    {
        batch.status = lab.status;
        return batch;
    }

    if lab.status == RollbackPlayableLabStatus::Active {
        let sample = local_sample.expect("active playable rollback tick requires local input");
        input_frame::validate_sample(&sample)
            .expect("active playable rollback tick requires a valid sample");
        advance_reference(lab, sample);
        if lab.status == RollbackPlayableLabStatus::Active {
            reconcile_and_publish(lab, &mut batch);
        }
        if lab.status == RollbackPlayableLabStatus::Active && lab.reference.finished {
            lab.status = RollbackPlayableLabStatus::Settling;
            finish_settlement_if_ready(lab);
        }
    } else {
        assert!(
            local_sample.is_none(),
            "rollback settlement must not consume match input"
        );
        lab.settlement_ticks += 1;
        resend_final_remote_rows(lab);
        reconcile_and_publish(lab, &mut batch);
        if lab.status == RollbackPlayableLabStatus::Settling {
            finish_settlement_if_ready(lab);
        }
        if lab.status == RollbackPlayableLabStatus::Settling
            && lab.settlement_ticks >= lab.settlement_limit
        {
            lab.status = RollbackPlayableLabStatus::DrainIncomplete;
        }
    }

    lab.transport_tick += 1;
    batch.status = lab.status;
    batch
}

/// An independent capture of the predicted client's current live boundary.
#[must_use]
pub fn current_snapshot(lab: &RollbackPlayableLab) -> MatchSnapshot {
    rollback_session::current_snapshot(&lab.session)
}

/// An independent capture of the reference's current live boundary.
#[must_use]
pub fn reference_snapshot(lab: &RollbackPlayableLab) -> MatchSnapshot {
    match_snapshot::capture_owned(&lab.reference, lab.reference_combat.as_ref())
}

/// An owned copy of a retained client boundary, plus its retention status.
#[must_use]
pub fn snapshot(
    lab: &RollbackPlayableLab,
    boundary_tick: i64,
) -> rollback_snapshot_history::RollbackSnapshotLookup {
    rollback_session::snapshot(&lab.session, boundary_tick)
}

/// A snapshot of this lab's retained-state shape.
#[must_use]
pub fn debug_model(lab: &RollbackPlayableLab) -> RollbackPlayableLabDebugModel {
    let session = rollback_session::diagnostics(&lab.session);
    let network = network_conditions::diagnostics(&lab.network);
    let event_diagnostics = rollback_events::diagnostics(&lab.events);
    RollbackPlayableLabDebugModel {
        profile: lab.profile_name.clone(),
        local_slot: lab.local_slot,
        status: lab.status,
        reference_tick: lab.reference.input_tick,
        current_tick: session.present_boundary,
        transport_tick: lab.transport_tick,
        confirmed_input_tick: session.confirmed_tick,
        confirmed_output_tick: session.confirmed_output_tick,
        predicted_slot_samples: session.predicted_slot_samples,
        predicted_ticks: session.predicted_ticks,
        correction_count: session.correction_count,
        rollback_count: session.rollback_count,
        latest_rollback_depth: session.latest_rollback_depth,
        max_rollback_depth: session.max_rollback_depth,
        resimulated_ticks: session.resimulated_ticks,
        retained_snapshot_count: session.snapshot_history.retained_boundary_count,
        retained_snapshot_bytes: session.snapshot_history.canonical_bytes,
        network_pending: network.pending_envelopes,
        network_counters: network_conditions::counters(&lab.network),
        network_high_water: network,
        convergence: lab.latest_convergence.clone(),
        event_status: event_diagnostics.status,
        predicted_early_finish: lab.predicted_early_finish,
        late_input_tick: lab.late_input_tick,
        settlement_ticks: lab.settlement_ticks,
        settlement_limit: lab.settlement_limit,
    }
}
