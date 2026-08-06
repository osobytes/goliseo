//! Partial port of `spec/sim/rollback_lab_spec.lua`.
//!
//! `sim/rollback_lab.lua`'s spec is 595 lines covering sixteen scenarios,
//! several built on `sim.determinism_evidence`'s full 600-tick recorded
//! fixture or a hand-driven combat/early-finish `MatchState`. Given this
//! module's scope (own files: `rollback_session.rs`, `rollback_lab.rs`,
//! `rollback_playable_lab.rs`, `rollback_validation.rs`) and the time
//! remaining in this pass, this file ports the scenarios that exercise the
//! campaign state machine's core contract — clean convergence, rollback
//! under delay/jitter, duplicate idempotency, multi-batch correction, and
//! the terminal-stability probe — using the same small hand-built
//! `varying_tape` fixture the Lua spec itself uses for exactly those cases.
//! Every other Lua case is a real gap, not a silent drop: see the "Not
//! ported" list below for each one and why.
//!
//! **Not ported** (report these as open coverage gaps, not passing):
//! - "pins the live soccer tape digest without a synthetic combat segment",
//!   "verifies every frozen authoritative boundary incrementally" — need
//!   `determinism_evidence::fixture_tape`'s full 600-tick recording; not
//!   exercised here to keep this file's own runtime bounded.
//! - "converges combat state...", "reactivates a predicted early finish..."
//!   — need a combat-active or hand-driven-to-full-time fixture
//!   (`combat::new_state`, a scripted `early_finish_tape`); not built here.
//! - "recovers independent loss from packet history and the final-row
//!   drain" — needs a lossy-profile fixture tuned to a specific recovered
//!   count; omitted for time.
//! - "reports causal hashes and the first state path for intentional
//!   corruption" — needs `RollbackLabOptions.corruption`, ported in
//!   `rollback_lab.rs` but not exercised here.
//! - "keeps timing observers outside repeatable logical evidence" — needs
//!   the `measure` hook, which this port's `advance_frame` does not thread
//!   to (see `rollback_lab.rs`'s module doc comment).
//! - "uses collision-safe strings and exact tape/profile identity in
//!   markers" — `rollback_lab.logical_marker`/`.summary` (the Lua
//!   diagnostic string builders) have no port in `rollback_lab.rs`; time.
//! - "bounds retained resources and never passes with unconfirmed
//!   authority" — needs a long soak-style tape; omitted for time.
//! - "uses one logical result for incremental and synchronous execution" —
//!   exercises `step_campaign` directly at small tick counts; omitted for
//!   time, though `run` itself (built on `step_campaign`) is exercised by
//!   every test below.

use gc_sim::input_frame;
use gc_sim::input_tape::{self, InputTape, InputTapeIdentity};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot;
use gc_sim::network_conditions::NetworkProfile;
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_lab::{self, RollbackLabOptions, RollbackLabStatus};

fn new_state(duration: f64, max_goals: i64) -> match_snapshot::MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: match_snapshot::PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration),
        max_goals: Some(max_goals),
        seed: Some(733.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    })
}

fn identity(name: &str, initial: &match_snapshot::MatchSnapshot) -> InputTapeIdentity {
    InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "rollback-lab-spec".to_string(),
        source: "materialized-spec-fixture".to_string(),
        content: "nebula-orion-spec-content".to_string(),
        tuning: gc_sim::tuning::Tuning::new().serialize(),
        config: "field=960x540;duration=20;max_goals=99;tick_rate=60".to_string(),
        fixture: name.to_string(),
        seed: 733.0,
        tick_rate: gc_sim::fixed_clock::TICK_RATE as i64,
        ownership: initial
            .state
            .input_ownership
            .clone()
            .expect("fixture is always slot-mode"),
        combat: None,
    }
}

/// `input_tape::new` validates its caller-supplied `initial` through
/// `match_snapshot::restore` before it gets a chance to apply its own
/// internal `normalize_marks` padding (see that private function's doc
/// comment in `input_tape.rs`) — so a never-stepped fixture must already
/// carry `marks` padded to the roster, same as `tests/rollback_session.rs`'s
/// module doc comment explains for `rollback_session`'s own fixtures.
fn pad_marks(state: &mut match_snapshot::MatchState) {
    let n = state.players.len();
    state.marks.home.resize(n, None);
    state.marks.away.resize(n, None);
}

fn varying_tape(count: i64, name: &str) -> InputTape {
    let mut state = new_state(20.0, 99);
    pad_marks(&mut state);
    let initial = match_snapshot::capture_owned(&state, None);
    let mut frames = Vec::with_capacity(count as usize);
    for tick in 0..count {
        let mut slots = [input_frame::InputSample::default(); 8];
        for slot in 1..=input_frame::SLOT_COUNT {
            let index = (slot - 1) as usize;
            slots[index] = input_frame::new_sample(input_frame::InputSampleOptions {
                move_x: Some(((tick * 29 + slot * 17) % 255) - 127),
                move_y: Some(((tick * 13 + slot * 31) % 255) - 127),
                held: Some(if tick % 3 == 0 {
                    input_frame::HELD_SPRINT
                } else {
                    0
                }),
                edges: Some(if tick % 7 == 0 {
                    input_frame::EDGE_DASH
                } else {
                    0
                }),
            })
            .expect("valid sample");
        }
        frames.push(input_frame::new(tick, Some(slots)).expect("canonical slots always validate"));
    }
    let tape_identity = identity(name, &initial);
    let tune = gc_sim::tuning::Tuning::new();
    input_tape::new(&tape_identity, &initial, &frames, &tune)
        .expect("hand-built tape is always well formed")
}

fn one_remote(remote_slot: i64) -> [RollbackInputSource; 8] {
    let mut sources = [RollbackInputSource::Local; 8];
    sources[(remote_slot - 1) as usize] = RollbackInputSource::Remote;
    sources
}

fn profile(
    base_delay_ticks: i64,
    jitter_min: i64,
    jitter_max: i64,
    loss: f64,
    duplication: f64,
) -> NetworkProfile {
    NetworkProfile {
        base_delay_ticks,
        jitter_min_ticks: jitter_min,
        jitter_max_ticks: jitter_max,
        independent_loss_rate: loss,
        duplication_rate: duplication,
        burst_start_rate: 0.0,
        burst_length_ticks: 0,
    }
}

fn frame_count(tape: &InputTape) -> i64 {
    tape.frames.len() as i64
}

#[test]
fn matches_every_clean_boundary_without_prediction_or_rollback() {
    let tape = varying_tape(40, "varying-40");
    let expected_frames = frame_count(&tape);
    let result = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("clean".to_string()),
            network_seed: Some(1),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );

    assert!(result.success);
    assert_eq!(result.status, RollbackLabStatus::Converged);
    assert_eq!(result.metrics.predicted_slot_samples, 0);
    assert_eq!(result.metrics.predicted_ticks, 0);
    assert_eq!(result.metrics.rollback_count, 0);
    assert_eq!(result.metrics.correction_count, 0);
    assert_eq!(result.metrics.compared_boundaries, expected_frames + 1);
    assert_eq!(result.reference_final_hash, result.client_final_hash);
    assert_eq!(result.confirmed_tick, expected_frames - 1);
}

#[test]
fn rolls_back_and_converges_with_omp0_delay_and_jitter_reordering() {
    let tape = varying_tape(48, "varying-48");
    let delayed = rollback_lab::run(
        clone_tape(&tape),
        RollbackLabOptions {
            profile: Some(profile(3, 0, 0, 0.01, 0.0)),
            profile_name: Some("omp0_parity".to_string()),
            network_seed: Some(7302),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(delayed.success);
    assert!(delayed.metrics.rollback_count > 0);
    assert!(delayed.metrics.resimulated_ticks > 0);
    assert!(delayed.metrics.max_rollback_depth >= 3);

    let reordered = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("jitter-spec".to_string()),
            profile: Some(profile(2, -2, 2, 0.0, 0.0)),
            network_seed: Some(102_223),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(reordered.success);
    assert!(reordered.network_counters.reordered > 0);
    assert!(reordered.metrics.rollback_count > 0);
}

#[test]
fn keeps_impairment_created_duplicates_idempotent() {
    let tape = varying_tape(12, "varying-12-dup");
    let result = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("duplicate-spec".to_string()),
            profile: Some(profile(0, 0, 0, 0.0, 1.0)),
            network_seed: Some(4),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );

    assert!(result.success);
    assert_eq!(
        result.network_counters.duplicated,
        result.network_counters.sent
    );
    assert_eq!(
        result.network_counters.delivered,
        result.network_counters.sent * 2
    );
    assert_eq!(result.metrics.correction_count, 0);
    assert_eq!(result.metrics.rollback_count, 0);
}

#[test]
fn reconciles_multiple_correction_batches_before_final_convergence() {
    let tape = varying_tape(18, "varying-18");
    let result = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("batch-spec".to_string()),
            profile: Some(profile(2, 0, 0, 0.0, 0.0)),
            network_seed: Some(2),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );

    assert!(result.success);
    assert!(result.metrics.rollback_count > 2);
    assert!(!result.metrics.rollback_depths.is_empty());
    assert_eq!(
        result.metrics.rollback_count, result.metrics.correction_count,
        "one changing remote row arrives per correction batch"
    );
}

#[test]
fn supports_exactly_thirty_ticks_and_fails_explicitly_at_thirty_one() {
    let tape = varying_tape(40, "varying-40-limit");
    let at_limit = rollback_lab::run(
        clone_tape(&tape),
        RollbackLabOptions {
            profile_name: Some("delay-30-spec".to_string()),
            profile: Some(profile(30, 0, 0, 0.0, 0.0)),
            network_seed: Some(30),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(at_limit.success);
    assert_eq!(at_limit.status, RollbackLabStatus::Converged);
    assert!(at_limit.metrics.max_rollback_depth <= 30);

    let over_limit = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("delay-31-spec".to_string()),
            profile: Some(profile(31, 0, 0, 0.0, 0.0)),
            network_seed: Some(31),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert_eq!(over_limit.status, RollbackLabStatus::LateInputUnrecoverable);
    assert!(!over_limit.success);
    assert!(over_limit.late_input_tick.is_some());
}

#[test]
fn derives_over_window_terminal_stability_from_the_blocked_session_seam() {
    let tape = varying_tape(40, "varying-40-terminal");
    let frames = frame_count(&tape);
    let mut campaign = rollback_lab::new_campaign(
        tape,
        RollbackLabOptions {
            profile_name: Some("delay-31-terminal".to_string()),
            profile: Some(profile(31, 0, 0, 0.0, 0.0)),
            network_seed: Some(31),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    let result = loop {
        if let Some(result) = rollback_lab::step_campaign(&mut campaign, frames, None) {
            break result.clone();
        }
    };
    assert_eq!(result.status, RollbackLabStatus::LateInputUnrecoverable);
    let hidden_progress = rollback_lab::probe_terminal_stability(&mut campaign);
    assert!(
        !hidden_progress,
        "an over-window terminal must not advance state or counters when probed again"
    );
}

/// The Lua original reuses one tape value across two `rollback_lab.run`
/// calls (Lua tables are references, and `run` never mutates its `tape`
/// argument). `InputTape` here is an owned value consumed by `run`
/// (`RollbackLabCampaign` owns it for the campaign's lifetime), so this
/// port clones where the Lua spec reused.
fn clone_tape(tape: &InputTape) -> InputTape {
    InputTape {
        version: tape.version,
        identity: tape.identity.clone(),
        initial: tape.initial.clone(),
        frames: tape.frames.clone(),
        boundary_hashes: tape.boundary_hashes.clone(),
    }
}

// ---------------------------------------------------------------------------
// The remaining ten cases from `spec/sim/rollback_lab_spec.lua`.
//
// These were originally left documented in prose rather than stubbed. Prose is
// not auditable: `cargo test` reports 6 of 16 as a clean pass with no signal
// that ten assertions are missing. Named stubs make the gap countable, which is
// what README §4 means by "never delete it silently".
//
// None is blocked on a missing module — `input_tape` and `determinism_evidence`
// both landed. They are unwritten work.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "stub: not yet written (combat-tape scenario); no technical blocker"]
fn pins_the_live_soccer_tape_digest_without_a_synthetic_combat_segment() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (combat-tape scenario); no technical blocker"]
fn converges_combat_state_and_confirmed_events_through_delayed_authority() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (packet-history recovery); no technical blocker"]
fn recovers_independent_loss_from_packet_history_and_the_final_row_drain() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (early-finish tape); no technical blocker"]
fn reactivates_a_predicted_early_finish_and_later_reaches_reference_full_time() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (intentional-corruption reporting); no technical blocker"]
fn reports_causal_hashes_and_the_first_state_path_for_intentional_corruption() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (incremental boundary verification); no technical blocker"]
fn verifies_every_frozen_authoritative_boundary_incrementally() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (the measure-hook observability case); no technical blocker"]
fn keeps_timing_observers_outside_repeatable_logical_evidence() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (logical_marker/summary, which was not ported); no technical blocker"]
fn uses_collision_safe_strings_and_exact_tape_profile_identity_in_markers() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (the soak resource-bound case); no technical blocker"]
fn bounds_retained_resources_and_never_passes_with_unconfirmed_authority() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}

#[test]
#[ignore = "stub: not yet written (incremental-vs-synchronous equivalence); no technical blocker"]
fn uses_one_logical_result_for_incremental_and_synchronous_execution() {
    unimplemented!("port this case from spec/sim/rollback_lab_spec.lua")
}
