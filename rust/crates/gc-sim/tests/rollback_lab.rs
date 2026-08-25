//! Tests for `gc_sim::rollback_lab`, built on hand-built `varying_tape`/
//! `combat_tape`/`early_finish_tape` fixtures, plus
//! `determinism_evidence::fixture_tape` for the one case that pins the live
//! soccer tape's digest.
//!
//! **Two coverage gaps are deliberate** (report these as open gaps, not
//! missing tests):
//! - "reports causal hashes and the first state path for intentional
//!   corruption" does not assert against a human-readable report string
//!   — this crate has no `rollback_lab::summary` function to call. Every
//!   other assertion in that case is exercised directly against
//!   `divergence` instead, which is `summary`'s own data source.
//! - "uses collision-safe strings and exact tape/profile identity in
//!   markers" does not assert three string-joining-collision checks — this
//!   crate has no `rollback_lab::logical_marker` function, and those checks
//!   are specifically about that function's own collision safety, which has
//!   no equivalent to assert against without it.
//!
//! Two other cases ("keeps timing observers outside repeatable logical
//! evidence", "uses one logical result for incremental and synchronous
//! execution") also touch the missing `logical_marker`, but assert
//! whole-`RollbackLabResult` equality instead — see the comment on the
//! first of those tests below for why that is a legitimate, strictly
//! stronger stand-in rather than a weakened one.

use gc_core::vec2::Vec2;
use gc_sim::combat;
use gc_sim::combat_identity;
use gc_sim::determinism_evidence;
use gc_sim::fixed_clock;
use gc_sim::input_frame;
use gc_sim::input_tape::{self, InputTape, InputTapeIdentity};
use gc_sim::keeper::KeeperShotType;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot;
use gc_sim::network_conditions::NetworkProfile;
use gc_sim::rollback_input_history::{self, RollbackInputSource};
use gc_sim::rollback_lab::{self, RollbackLabCorruption, RollbackLabOptions, RollbackLabStatus};
use gc_sim::tuning::Tuning;

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

/// An 80-tick fixture with a `CombatMatchState` companion, driving one tick
/// of equipment-pressed, twenty ticks of equipment-held, then release.
fn combat_tape() -> InputTape {
    let mut state = new_state(20.0, 99);
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    pad_marks(&mut state);
    let initial = match_snapshot::capture(&state, Some(&combat_state));
    let mut frames = Vec::with_capacity(80);
    for tick in 0..80i64 {
        let mut frame = input_frame::neutral(tick).expect("neutral frame is always valid");
        for slot in 0..(input_frame::SLOT_COUNT as usize) {
            if tick == 0 {
                frame.slots[slot] = input_frame::new_sample(input_frame::InputSampleOptions {
                    held: Some(input_frame::HELD_EQUIPMENT),
                    edges: Some(input_frame::EDGE_EQUIPMENT_PRESSED),
                    ..Default::default()
                })
                .expect("valid sample");
            } else if tick < 20 {
                frame.slots[slot] = input_frame::new_sample(input_frame::InputSampleOptions {
                    held: Some(input_frame::HELD_EQUIPMENT),
                    ..Default::default()
                })
                .expect("valid sample");
            } else if tick == 20 {
                frame.slots[slot] = input_frame::new_sample(input_frame::InputSampleOptions {
                    edges: Some(input_frame::EDGE_EQUIPMENT_RELEASED),
                    ..Default::default()
                })
                .expect("valid sample");
            }
        }
        frames.push(frame);
    }
    let tape_identity = InputTapeIdentity {
        tape_version: input_tape::COMBAT_VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::COMBAT_VERSION,
        build: "rollback-lab-combat-spec".to_string(),
        source: "materialized-combat-spec-fixture".to_string(),
        content: "nebula-orion-spec-content".to_string(),
        tuning: Tuning::new().serialize(),
        config: "field=960x540;duration=20;max_goals=99;tick_rate=60".to_string(),
        fixture: "combat-rollback-80".to_string(),
        seed: 733.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership: initial
            .state
            .input_ownership
            .clone()
            .expect("fixture is always slot-mode"),
        combat: Some(combat_identity::for_state(Some(&combat_state))),
    };
    let tune = Tuning::new();
    input_tape::new(&tape_identity, &initial, &frames, &tune)
        .expect("hand-built combat tape is always well formed")
}

/// A snapshot one tick from a scored goal, so a defender's dash (see
/// [`early_finish_tape`]) can prevent it and force the match past its
/// predicted early finish.
fn preventable_goal_initial() -> match_snapshot::MatchSnapshot {
    let mut state = new_state(4.0, 1);
    let carrier_index = state.slot_players[0].expect("slot 1 has a player");
    {
        let carrier = &mut state.players[(carrier_index - 1) as usize];
        carrier.pos = Vec2::new(900.0, 270.0);
        carrier.vel = Vec2::new(0.0, 0.0);
        carrier.run_vel = Vec2::new(0.0, 0.0);
        carrier.facing = Vec2::new(1.0, 0.0);
        // #489: see the identical note in tests/rollback_session.rs's
        // `preventable_goal_fixture` -- a standing-poke tackle now takes
        // ~15 ticks to charge and execute instead of resolving instantly,
        // so the old 1-tick wind-up gave the defender no way to ever beat
        // the shot. Widened to the same 20 ticks.
        carrier.windup_timer = 20.0 * fixed_clock::TICK_SECONDS;
        carrier.windup_shot = Some(match_snapshot::WindupShot {
            dir: Vec2::new(1.0, 0.0),
            speed: 900.0,
            vz: 0.0,
            spin: 0.0,
            shot_type: KeeperShotType::Ground,
        });
    }
    state.owner = Some(carrier_index);
    state.ball = Vec2::new(918.0, 270.0);
    state.ball_vel = Vec2::new(0.0, 0.0);
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 2.0;
    state.block_grace = 2.0;
    for (zero_index, player) in state.players.iter_mut().enumerate() {
        let index = (zero_index + 1) as i64;
        if index != carrier_index {
            player.pos = Vec2::new(if index < 6 { 200.0 } else { 100.0 }, 40.0 + index as f64);
            player.vel = Vec2::new(0.0, 0.0);
            player.run_vel = Vec2::new(0.0, 0.0);
        }
    }
    let defender_index = state.slot_players[4].expect("slot 5 has a player");
    {
        let defender = &mut state.players[(defender_index - 1) as usize];
        defender.pos = Vec2::new(910.0, 240.0);
        defender.facing = Vec2::new(-1.0, 0.0);
    }
    pad_marks(&mut state);
    match_snapshot::capture(&state, None)
}

/// Steps [`preventable_goal_initial`] forward with a real slot-mode
/// `sim_match::step` loop (slot 5 dashes on tick 0 to deny the preventable
/// goal) until the match reaches full time, recording every effective frame
/// along the way.
fn early_finish_tape() -> InputTape {
    let initial = preventable_goal_initial();
    let tune = Tuning::new();
    let (mut state, _combat) = match_snapshot::restore(&initial);
    let mut frames = Vec::new();
    while !state.finished {
        let tick = state.input_tick;
        let mut slots = [input_frame::InputSample::default(); 8];
        if tick == 0 {
            slots[4] = input_frame::new_sample(input_frame::InputSampleOptions {
                edges: Some(input_frame::EDGE_DASH),
                ..Default::default()
            })
            .expect("valid sample");
        }
        let frame = input_frame::new(tick, Some(slots)).expect("canonical slots always validate");
        frames.push(frame);
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(&frame),
            None,
            &tune,
        );
        assert!(
            frames.len() < 400,
            "early-finish fixture did not reach full time"
        );
    }
    let tape_identity = identity("prevented-early-finish", &initial);
    input_tape::new(&tape_identity, &initial, &frames, &tune)
        .expect("hand-built early-finish tape is always well formed")
}

/// Run `f`, catching a panic and returning its message instead of letting it
/// abort the test — the stand-in for asserting "this call panics with
/// message X" rather than "this call returns an error".
fn catch_panic_message<F: FnOnce() + std::panic::UnwindSafe>(f: F) -> Option<String> {
    match std::panic::catch_unwind(f) {
        Ok(()) => None,
        Err(payload) => Some(
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "<non-string panic payload>".to_string()),
        ),
    }
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

/// `InputTape` is an owned value consumed by `run` (`RollbackLabCampaign`
/// owns it for the campaign's lifetime), so a test that wants to run the
/// same starting tape through two calls needs an explicit clone rather than
/// sharing one value.
fn clone_tape(tape: &InputTape) -> InputTape {
    InputTape {
        version: tape.version,
        identity: tape.identity.clone(),
        initial: tape.initial.clone(),
        frames: tape.frames.clone(),
        boundary_hashes: tape.boundary_hashes.clone(),
    }
}

#[test]
fn pins_the_live_soccer_tape_digest_without_a_synthetic_combat_segment() {
    let tune = Tuning::new();
    let tape = determinism_evidence::fixture_tape(&tune)
        .expect("the live soccer fixture tape is always well formed");
    // A digest over the OMP-1 fixture tape, so it is a companion of that
    // fixture's DERIVED half and moves with it. Re-recorded for #488 in the
    // same commit as `gc-data/src/omp1_determinism.json`. Like the two
    // literals in `gc_data::omp1_determinism`'s own test, #504's documented
    // re-record command does not mention this one either.
    //
    // Re-recorded again for #517's mechanical transcendental sweep, in the
    // same commit as `gc-data/src/omp1_determinism.json`'s `boundary_hashes`
    // re-record -- `fixture_tape` folds those boundary hashes into the
    // `InputTape` this digest covers, so it moves whenever they do, even
    // though nothing about the frozen `frame_wires`/`identity` half changed.
    //
    // Re-recorded again for #489, same reason: `identity.snapshot_version`
    // (12 -> 13, `MatchPlayer::action`) and the derived `boundary_hashes`
    // both moved in `gc-data/src/omp1_determinism.json`.
    //
    // And again for #490, same reason a third time: snapshot_version 13 -> 14
    // (`MatchPlayer::keeper_fatigue`) plus the derived `boundary_hashes`.
    //
    // And again for #572, completing #489's possession invariant. This time
    // the derived `boundary_hashes` moved ALONE -- no schema change, no new
    // field, `identity.snapshot_version` untouched at 14 -- which makes this
    // the cleanest illustration of what this pin actually tracks: OMP-1's
    // derived half and nothing else. Note the contrast with
    // `expected_final_hash`, which did NOT move in that same re-record: this
    // digest folds in every boundary, that one only the last.
    //
    // And again for #578 (a `Recovering` penalty survives the possession
    // change), the same shape as #572: derived `boundary_hashes` alone,
    // snapshot_version untouched, `expected_final_hash` unmoved. A first
    // isolated `--test rollback_lab` run during that re-freeze passed
    // because it ran BEFORE the OMP-1 derived re-record this digest folds
    // in -- the same only-the-full-suite-counts trap #572's own re-freeze
    // hit when cargo stopped at an earlier failing binary.
    //
    // And again for the 2026-08-25 pitch re-dimensioning (960x540 ->
    // 1648x927, docs/design/fun_metrics.md's drift log): every derived
    // `boundary_hashes` entry moved, including the first, because the
    // frozen input frames now play out against a different-sized pitch
    // from kickoff onward -- not just a schema bump this time.
    //
    // And again for `LOCO_PACE_REF_HI` settling at 280 (down from the 300
    // the pitch re-dimensioning above picked; see the note beside the knob
    // in `gc_data::tunables`), in the SAME commit as
    // `gc_data::omp1_determinism`'s own re-record of `expected_final_hash`,
    // `expected_sequence_digest` and `boundary_hash_lines()[0]` /
    // `boundary_hash_lines()[last]` -- this digest folds in every one of
    // those boundaries, so it moves whenever any of them do, same as the
    // #572/#578 entries above.
    assert_eq!(rollback_lab::tape_digest(&tape), "f2ab89ab575b4d1a");
}

#[test]
fn converges_combat_state_and_confirmed_events_through_delayed_authority() {
    let tape = combat_tape();
    let result = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("combat-delay-spec".to_string()),
            profile: Some(profile(3, 0, 0, 0.0, 0.0)),
            network_seed: Some(2001),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(result.success);
    assert_eq!(result.status, RollbackLabStatus::Converged);
    assert!(result.metrics.rollback_count > 0);
    assert!(result.event_metrics.confirmed_combat_events > 0);
    assert_eq!(
        result.event_metrics.reference_digest,
        result.event_metrics.confirmed_digest
    );
    assert_eq!(
        result
            .reference_final_snapshot
            .combat
            .as_ref()
            .expect("combat tape final snapshot carries combat state")
            .tick,
        80
    );
    assert_eq!(
        result
            .client_final_snapshot
            .combat
            .as_ref()
            .expect("combat tape final snapshot carries combat state")
            .tick,
        80
    );
    assert!(
        result.metrics.peaks.snapshot_bytes
            < gc_data::omp2_rollback_validation::DATA
                .budgets
                .snapshot_bytes
    );
    assert!(
        result.metrics.peaks.history_bytes
            < gc_data::omp2_rollback_validation::DATA
                .budgets
                .history_bytes
    );
}

#[test]
fn recovers_independent_loss_from_packet_history_and_the_final_row_drain() {
    let history = rollback_lab::run(
        varying_tape(5, "history-recovery"),
        RollbackLabOptions {
            profile_name: Some("loss-history-spec".to_string()),
            profile: Some(profile(0, 0, 0, 0.5, 0.0)),
            network_seed: Some(85),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(history.success);
    assert!(history.network_counters.independent_lost > 0);
    assert!(history.network_counters.history_recovered > 0);

    let final_result = rollback_lab::run(
        varying_tape(6, "final-row-recovery"),
        RollbackLabOptions {
            profile_name: Some("final-loss-spec".to_string()),
            profile: Some(profile(4, 0, 0, 0.5, 0.0)),
            network_seed: Some(85),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(final_result.success);
    assert!(final_result.drain.complete);
    assert_eq!(final_result.drain.recovered, 1);
    assert!(
        final_result.network_counters.sent > 6,
        "the lost final row must be resent"
    );
    assert!(
        final_result.metrics.peaks.network_pending_envelopes
            > final_result.network_diagnostics.pending_envelopes
    );
    assert!(
        final_result.metrics.peaks.network_pending_record_references
            > final_result.network_diagnostics.pending_record_references
    );
}

#[test]
fn reactivates_a_predicted_early_finish_and_later_reaches_reference_full_time() {
    let tape = early_finish_tape();
    // #489: `base_delay_ticks` has to sit in a narrow window now that the
    // preventing tackle is charge-and-execute rather than instant (see the
    // note on `preventable_goal_initial`'s widened `windup_timer`). Two
    // bounds, both measured directly against this fixture rather than
    // assumed:
    //   - it must be long enough that the LOCAL guess (no dash at all for
    //     the missing remote slot 5) actually reaches its own false
    //     "finished" conclusion before the real, dash-including row
    //     corrects it -- at 25 ticks the correction still lands before the
    //     local guess gets there and `predicted_early_finish` never fires.
    //   - it must stay under `rollback_input_history::ROLLBACK_WINDOW_TICKS`
    //     (30) or the delayed input is outside the rollback window entirely
    //     and the session gives up as `LateInputUnrecoverable`, which fails
    //     on `result.success` rather than on the prediction metrics -- 40
    //     ticks demonstrated that failure mode directly.
    // 26 through 30 all land in the working window; 28 is the middle of it.
    let result = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("early-finish-spec".to_string()),
            profile: Some(profile(28, 0, 0, 0.0, 0.0)),
            network_seed: Some(6),
            sources: Some(one_remote(5)),
            ..Default::default()
        },
    );

    assert!(result.success);
    assert!(result.metrics.predicted_early_finish);
    assert!(result.metrics.reactivation_count > 0);
    assert_eq!(
        result.client_final_boundary,
        result.reference_final_boundary
    );
    assert_eq!(
        result.confirmed_output_tick,
        result.reference_final_boundary - 1
    );
}

#[test]
fn reports_causal_hashes_and_the_first_state_path_for_intentional_corruption() {
    let result = rollback_lab::run(
        varying_tape(16, "varying-16-corruption"),
        RollbackLabOptions {
            profile_name: Some("clean".to_string()),
            network_seed: Some(9),
            sources: Some(one_remote(1)),
            corruption: Some(RollbackLabCorruption { tick: 2, slot: 1 }),
            ..Default::default()
        },
    );

    assert!(!result.success);
    assert_eq!(result.status, RollbackLabStatus::Diverged);
    let divergence = result
        .divergence
        .as_ref()
        .expect("a diverged run reports its divergence");
    assert_eq!(divergence.causal_tick, 2);
    assert_ne!(divergence.expected_hash, divergence.actual_hash);
    let first_difference = divergence
        .first_difference
        .as_ref()
        .expect("a diverged run reports its first structural disagreement");
    assert!(!first_difference.path.is_empty());
    // This crate has no `rollback_lab::summary` function (a human-readable
    // report string) — see this module's own doc comment — so there is
    // nothing to assert a "causal_tick=2" substring against. Every other
    // check in this case is asserted above against `divergence` directly,
    // which is `summary`'s own data source.
}

#[test]
fn verifies_every_frozen_authoritative_boundary_incrementally() {
    let mut tape = varying_tape(4, "varying-4-frozen");
    tape.boundary_hashes[2] = "0000000000000000".to_string();
    let frames = frame_count(&tape);
    let mut campaign = rollback_lab::new_campaign(
        tape,
        RollbackLabOptions {
            profile_name: Some("clean".to_string()),
            network_seed: Some(9),
            sources: Some(one_remote(1)),
            prevalidated_tape: true,
            ..Default::default()
        },
    );
    let message = catch_panic_message(std::panic::AssertUnwindSafe(|| {
        rollback_lab::step_campaign(&mut campaign, frames, None);
    }))
    .expect("a tampered frozen boundary must panic");
    assert!(
        message.contains("frozen boundary 2 hash mismatch"),
        "unexpected panic message: {message}"
    );
}

#[test]
fn keeps_timing_observers_outside_repeatable_logical_evidence() {
    let tape = varying_tape(20, "varying-20-timing");
    let calls_a = std::rc::Rc::new(std::cell::Cell::new(0i64));
    let calls_a_measure = calls_a.clone();
    let first = rollback_lab::run(
        clone_tape(&tape),
        RollbackLabOptions {
            profile_name: Some("repeat-spec".to_string()),
            profile: Some(profile(3, 0, 0, 0.0, 0.0)),
            network_seed: Some(77),
            sources: Some(one_remote(1)),
            measure: Some(Box::new(move |_label, operation: &mut dyn FnMut()| {
                calls_a_measure.set(calls_a_measure.get() + 1);
                operation();
            })),
            ..Default::default()
        },
    );
    let calls_b = std::rc::Rc::new(std::cell::Cell::new(1000i64));
    let calls_b_measure = calls_b.clone();
    let second = rollback_lab::run(
        tape,
        RollbackLabOptions {
            profile_name: Some("repeat-spec".to_string()),
            profile: Some(profile(3, 0, 0, 0.0, 0.0)),
            network_seed: Some(77),
            sources: Some(one_remote(1)),
            measure: Some(Box::new(move |_label, operation: &mut dyn FnMut()| {
                calls_b_measure.set(calls_b_measure.get() + 7);
                operation();
            })),
            ..Default::default()
        },
    );

    assert!(calls_a.get() > 0);
    assert!(calls_b.get() > calls_a.get());
    // This crate has no `rollback_lab::logical_marker` function (a
    // canonical string encoding for cross-run comparison) — see this
    // module's own doc comment. `RollbackLabResult` derives `PartialEq` and
    // carries no wall-clock/timing field at all (the module doc explains
    // why: "clock values are neither read nor retained here and cannot
    // enter the logical result"), so whole-struct equality is a strictly
    // stronger check that two runs whose `measure` observers behave
    // completely differently still produce the same logical result — and,
    // having no timing field, it trivially also confirms that nothing
    // timing-related has leaked into that result.
    assert_eq!(first, second);

    let missing_call = catch_panic_message(std::panic::AssertUnwindSafe(|| {
        let _ = rollback_lab::run(
            varying_tape(4, "varying-4-missing-call"),
            RollbackLabOptions {
                profile: Some(profile(0, 0, 0, 0.0, 0.0)),
                sources: Some(one_remote(1)),
                measure: Some(Box::new(|_label, _operation: &mut dyn FnMut()| {})),
                ..Default::default()
            },
        );
    }));
    assert!(
        missing_call.is_some(),
        "an observer cannot skip the owned operation"
    );

    let double_call = catch_panic_message(std::panic::AssertUnwindSafe(|| {
        let _ = rollback_lab::run(
            varying_tape(4, "varying-4-double-call"),
            RollbackLabOptions {
                profile: Some(profile(0, 0, 0, 0.0, 0.0)),
                sources: Some(one_remote(1)),
                measure: Some(Box::new(|_label, operation: &mut dyn FnMut()| {
                    operation();
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
                })),
                ..Default::default()
            },
        );
    }));
    assert!(
        double_call.is_some(),
        "a swallowed second call still fails the once guard"
    );
}

#[test]
fn uses_collision_safe_strings_and_exact_tape_profile_identity_in_markers() {
    let injected = rollback_lab::run(
        varying_tape(2, "fixture|profile=forged"),
        RollbackLabOptions {
            profile: Some(profile(0, 0, 0, 0.1, 0.0)),
            network_seed: Some(2),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    let adjacent = rollback_lab::run(
        varying_tape(2, "fixture|profile=forged="),
        RollbackLabOptions {
            profile_name: Some("custom|status=forged".to_string()),
            profile: Some(profile(0, 0, 0, 0.100_000_000_000_000_02, 0.0)),
            network_seed: Some(2),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );

    assert_eq!(injected.profile, "custom");
    assert_ne!(injected.profile_parameters, adjacent.profile_parameters);
    assert_ne!(injected.tape_digest, adjacent.tape_digest);
    // This crate has no `rollback_lab::logical_marker` function (see this
    // module's own doc comment and the "keeps timing observers..." test
    // above), so there is no string-joining collision safety to assert
    // here — whether a caller-controlled fixture name or profile name
    // containing a delimiter character (`|`, `=`) can forge what looks like
    // another field in a rendered marker string. This crate's
    // `RollbackLabResult` fields (asserted above) are typed and
    // independently readable, so the injection failure mode a joined
    // string encoding invites does not arise for them — a different,
    // weaker guarantee than a `logical_marker` collision-safety check would
    // pin, but the strongest one available without that function.
}

#[test]
fn bounds_retained_resources_and_never_passes_with_unconfirmed_authority() {
    let bounded = rollback_lab::run(
        varying_tape(80, "varying-80-bounded"),
        RollbackLabOptions {
            profile_name: Some("bounded-spec".to_string()),
            profile: Some(profile(3, 0, 0, 0.0, 0.0)),
            network_seed: Some(3),
            sources: Some(one_remote(1)),
            ..Default::default()
        },
    );
    assert!(bounded.success);
    assert_eq!(
        bounded.metrics.peaks.snapshot_count,
        rollback_input_history::ROLLBACK_WINDOW_TICKS + 1
    );
    assert!(bounded.metrics.peaks.snapshot_bytes >= bounded.metrics.current_snapshot_bytes);
    assert!(bounded.metrics.peaks.input_authoritative_samples <= 32 * 8);
    assert!(bounded.metrics.peaks.network_authoritative_records <= 7);
    assert_eq!(bounded.network_diagnostics.pending_envelopes, 0);
    assert!(
        bounded.metrics.peaks.network_pending_envelopes
            > bounded.network_diagnostics.pending_envelopes
    );
    assert!(
        bounded.metrics.peaks.network_pending_record_references
            > bounded.network_diagnostics.pending_record_references
    );
    // `RollbackLabDrainSummary` has no `deliveries` field in its
    // compile-time definition at all, so the guarantee that the drain
    // summary never grows a stray field holds structurally, with nothing
    // left to assert at runtime.

    let unconfirmed = rollback_lab::run(
        varying_tape(10, "unconfirmed"),
        RollbackLabOptions {
            profile_name: Some("all-loss-spec".to_string()),
            profile: Some(profile(0, 0, 0, 1.0, 0.0)),
            network_seed: Some(1),
            sources: Some(one_remote(1)),
            drain_ticks: Some(16),
            ..Default::default()
        },
    );
    assert!(!unconfirmed.success);
    assert_eq!(unconfirmed.status, RollbackLabStatus::UnconfirmedAuthority);
    assert_eq!(unconfirmed.unconfirmed_tick, Some(0));
    assert!(!unconfirmed.drain.complete);
}

#[test]
fn uses_one_logical_result_for_incremental_and_synchronous_execution() {
    let tape = varying_tape(24, "incremental-equivalence");
    let options = || RollbackLabOptions {
        profile_name: Some("incremental-spec".to_string()),
        profile: Some(profile(3, 0, 0, 0.0, 0.0)),
        network_seed: Some(2001),
        sources: Some(one_remote(1)),
        ..Default::default()
    };
    let synchronous = rollback_lab::run(clone_tape(&tape), options());
    let mut campaign = rollback_lab::new_campaign(tape, options());
    let incremental = loop {
        if let Some(result) = rollback_lab::step_campaign(&mut campaign, 1, None) {
            break result.clone();
        }
    };
    // See "keeps timing observers..." above: whole-struct equality is the
    // stand-in used here in place of a `logical_marker` comparison.
    assert_eq!(incremental, synchronous);
    assert_eq!(
        incremental.event_metrics.reference_digest,
        incremental.event_metrics.confirmed_digest
    );
    assert_eq!(incremental.event_metrics.speculative_residue, 0);
    let accounting = &incremental.history_accounting;
    assert_eq!(accounting.input.authoritative_bytes, 3421);
    assert_eq!(accounting.input.predecessor_anchor_bytes, 0);
    assert_eq!(accounting.input.effective_frame_bytes, 2315);
    assert_eq!(accounting.input.input_record_bytes, 6131);
    assert_eq!(accounting.input.total_bytes, 11867);
    // `accounting.output_bytes` and `metrics.peaks.output_bytes` are not
    // pinned to exact constants here. `rollback_session.rs`'s own module
    // doc comment ("Retained-output byte accounting is a diagnostic proxy")
    // documents that this crate deliberately uses Rust's derived `Debug`
    // rendering as its size proxy, and that "no observer depends on the
    // exact number" — this run's `output_bytes` is 32756, an incidental
    // value of that encoding rather than a guaranteed one. What the module
    // doc does guarantee — that independently-computed totals agree — is
    // what `assert_eq!(incremental, synchronous)` above already exercises
    // for this exact fixture.
    assert!(accounting.output_bytes > 0);
    assert_eq!(accounting.event_bytes, 0);
    assert_eq!(incremental.metrics.peaks.input_bytes, 11867);
    assert!(incremental.metrics.peaks.output_bytes > 0);
    // #489: standing-poke tackles now miss far more often than the old
    // instant-resolve check ever let them (see tests/knob_contract.rs's
    // measured whiff_rate), and each whiff is a new `TackleMiss` event this
    // scenario did not emit before -- the retained event window's peak
    // byte usage grew with the event count, 833 -> 2929.
    //
    // And again for the 2026-08-25 pitch re-dimensioning (960x540 ->
    // 1648x927): this fixture's own `new_state` still hardcodes a 960x540
    // `PitchSize`, but `KICKOFF_CLEAR` (120 -> 123) nudges kickoff placement
    // regardless of field size, which pushes a retained event's coordinate
    // field(s) across a float-formatting digit boundary -- the same
    // byte-length-by-`Debug`-rendering encoding `step_len`/`wrapped_event_len`
    // use here means a wider formatted string, not a new event, moved the
    // peak by 2 bytes, 2929 -> 2931.
    //
    // And again for `LOCO_PACE_REF_HI` settling at 280 (down from the 300
    // the pitch re-dimensioning above picked; see the note beside the knob
    // in `gc_data::tunables` for why): the same fixture's retained events
    // still carry player coordinates from a run whose movement speed
    // changed, so one or more of those coordinates format to a shorter
    // `Debug` string than before -- again a formatting-width shift, not a
    // new or dropped event, moved the peak by 5 bytes, 2931 -> 2926.
    assert_eq!(incremental.metrics.peaks.event_bytes, 2926);
    assert!(incremental.history_accounting.total_bytes > 0);
    assert!(incremental.metrics.peaks.history_bytes >= incremental.history_accounting.total_bytes);
}
