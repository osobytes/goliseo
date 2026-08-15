//! Tests for `gc_sim::rollback_playable_lab`.
//!
//! Fixture snapshots use `match_snapshot::capture_owned`/`hash_canonical`
//! (a never-stepped `MatchState`'s `marks` are legitimately shorter than the
//! roster — see `rollback_session::normalize_marks`'s doc comment, and
//! `tests/rollback_session.rs`'s own module doc for the same fixture
//! shape). `RollbackPlayableCorrection.causal_tick` mutation demonstrations
//! are kept even though batches are owned, `Clone` values, so independence
//! is already a type-system guarantee rather than a behavior under test —
//! the case still performs one such mutation to make that guarantee
//! visible.

use gc_core::vec2::Vec2;
use gc_sim::fixed_clock;
use gc_sim::input_frame;
use gc_sim::keeper::KeeperShotType;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchSnapshot, PitchSize, WindupShot};
use gc_sim::network_conditions::NetworkProfile;
use gc_sim::rollback_playable_lab::{
    self, RollbackPlayableLab, RollbackPlayableLabOptions, RollbackPlayableLabStatus,
};

fn initial_snapshot(duration: f64) -> MatchSnapshot {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration),
        max_goals: Some(99),
        seed: Some(74.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    match_snapshot::capture_owned(&state, None)
}

fn preventable_goal_snapshot() -> MatchSnapshot {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(1.0),
        max_goals: Some(1),
        seed: Some(733.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    let carrier_index = state.slot_players[0].expect("slot one is mapped");
    {
        let carrier = &mut state.players[(carrier_index - 1) as usize];
        carrier.pos = Vec2::new(900.0, 270.0);
        carrier.vel = Vec2::new(0.0, 0.0);
        carrier.run_vel = Vec2::new(0.0, 0.0);
        carrier.facing = Vec2::new(1.0, 0.0);
        // #489: see the identical note in tests/rollback_session.rs's
        // `preventable_goal_fixture` -- a standing-poke tackle now takes
        // ~15-20 ticks to charge and execute instead of resolving instantly,
        // so the old 1-tick wind-up gave the bot-controlled defender no way
        // to ever beat the shot. Widened to the same 20 ticks.
        carrier.windup_timer = 20.0 * fixed_clock::TICK_SECONDS;
        carrier.windup_shot = Some(WindupShot {
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
    for (index0, player) in state.players.iter_mut().enumerate() {
        let index = index0 as i64 + 1;
        if index != carrier_index {
            player.pos = Vec2::new(if index < 6 { 200.0 } else { 100.0 }, 40.0 + index as f64);
            player.vel = Vec2::new(0.0, 0.0);
            player.run_vel = Vec2::new(0.0, 0.0);
        }
    }
    let defender_index = state.slot_players[4].expect("slot five is mapped");
    let defender = &mut state.players[(defender_index - 1) as usize];
    defender.pos = Vec2::new(910.0, 240.0);
    defender.facing = Vec2::new(-1.0, 0.0);
    match_snapshot::capture_owned(&state, None)
}

fn fixed_profile(delay: i64, loss: f64) -> NetworkProfile {
    NetworkProfile {
        base_delay_ticks: delay,
        jitter_min_ticks: 0,
        jitter_max_ticks: 0,
        independent_loss_rate: loss,
        duplication_rate: 0.0,
        burst_start_rate: 0.0,
        burst_length_ticks: 0,
    }
}

fn run_to_terminal(
    lab: &mut RollbackPlayableLab,
    maximum: i64,
) -> rollback_playable_lab::RollbackPlayableLabDebugModel {
    for _ in 0..maximum {
        let before = rollback_playable_lab::debug_model(lab);
        if before.status != RollbackPlayableLabStatus::Active
            && before.status != RollbackPlayableLabStatus::Settling
        {
            return before;
        }
        let sample =
            rollback_playable_lab::needs_local_sample(lab).then(input_frame::neutral_sample);
        rollback_playable_lab::advance(lab, before.transport_tick, sample);
    }
    rollback_playable_lab::debug_model(lab)
}

#[test]
fn constructs_stable_slot_ownership_and_copied_boundary_zero() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(2.0),
        RollbackPlayableLabOptions {
            local_slot: Some(6),
            profile_name: Some("clean".to_string()),
            ..Default::default()
        },
    );
    let current = rollback_playable_lab::current_snapshot(&lab);
    assert!(current.state.slot_mode);
    assert_eq!(
        current.state.controlled,
        current.state.slot_players[5].unwrap()
    );
    let boundary = rollback_playable_lab::snapshot(&lab, 0);
    assert_eq!(
        boundary.status,
        gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(
        boundary.snapshot.unwrap().state.controlled,
        current.state.controlled
    );

    let mut mutated = current;
    mutated.state.score.home = 9;
    assert_eq!(
        rollback_playable_lab::current_snapshot(&lab)
            .state
            .score
            .home,
        0
    );
    let debug = rollback_playable_lab::debug_model(&lab);
    assert_eq!(debug.reference_tick, 0);
    assert_eq!(debug.current_tick, 0);
    assert_eq!(debug.transport_tick, 0);
    assert_eq!(debug.local_slot, 6);
    let mut mutated_debug = debug;
    mutated_debug.convergence.status =
        gc_sim::rollback_playable_lab::RollbackPlayableConvergenceStatus::Diverged;
    assert_eq!(
        rollback_playable_lab::debug_model(&lab).convergence.status,
        gc_sim::rollback_playable_lab::RollbackPlayableConvergenceStatus::Matched
    );
    // Silence "unused mut"-style dead-store lints on the demonstration
    // values above without changing their intent.
    let _ = &mut lab;
}

#[test]
fn submits_one_local_row_immediately_and_predicts_seven_networked_rows() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(2.0),
        RollbackPlayableLabOptions {
            local_slot: Some(4),
            profile: Some(fixed_profile(2, 0.0)),
            profile_name: Some("two_tick".to_string()),
            ..Default::default()
        },
    );
    let batch = rollback_playable_lab::advance(&mut lab, 0, Some(input_frame::neutral_sample()));
    assert_eq!(batch.outputs.len(), 1);
    let record = &batch.outputs[0].input;
    assert_eq!(record.tick, 0);
    assert_eq!(record.slots.len(), input_frame::SLOT_COUNT as usize);
    for slot in 1..=input_frame::SLOT_COUNT {
        let index = (slot - 1) as usize;
        if slot == 4 {
            assert_eq!(
                record.slots[index].source,
                gc_sim::rollback_input_history::RollbackInputSource::Local
            );
            assert_eq!(
                record.slots[index].status,
                gc_sim::rollback_input_history::RollbackInputStatus::Authoritative
            );
        } else {
            assert_eq!(
                record.slots[index].source,
                gc_sim::rollback_input_history::RollbackInputSource::Remote
            );
            assert_eq!(
                record.slots[index].status,
                gc_sim::rollback_input_history::RollbackInputStatus::Predicted
            );
        }
    }
    let debug = rollback_playable_lab::debug_model(&lab);
    assert_eq!(debug.reference_tick, 1);
    assert_eq!(debug.current_tick, 1);
    assert_eq!(debug.transport_tick, 1);
    assert_eq!(debug.predicted_slot_samples, 7);
}

#[test]
fn performs_real_rollback_and_converges_to_the_independent_reference() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(8.0 * fixed_clock::TICK_SECONDS),
        RollbackPlayableLabOptions {
            local_slot: Some(1),
            profile: Some(fixed_profile(2, 0.0)),
            profile_name: Some("scripted".to_string()),
            network_seed: Some(19),
            bot_seed: Some(91),
            ..Default::default()
        },
    );
    let mut saw_correction = false;
    for _ in 0..64 {
        let debug = rollback_playable_lab::debug_model(&lab);
        if debug.status != RollbackPlayableLabStatus::Active
            && debug.status != RollbackPlayableLabStatus::Settling
        {
            break;
        }
        let sample =
            rollback_playable_lab::needs_local_sample(&lab).then(input_frame::neutral_sample);
        let mut batch = rollback_playable_lab::advance(&mut lab, debug.transport_tick, sample);
        if !batch.corrections.is_empty() {
            saw_correction = true;
            batch.corrections[0].causal_tick = 999;
        }
    }
    let debug = rollback_playable_lab::debug_model(&lab);
    assert!(
        saw_correction,
        "the impaired remote bots must correct prediction"
    );
    assert_eq!(debug.status, RollbackPlayableLabStatus::Converged);
    assert!(debug.rollback_count > 0);
    assert!(debug.resimulated_ticks > 0);
    assert_eq!(
        debug.convergence.status,
        gc_sim::rollback_playable_lab::RollbackPlayableConvergenceStatus::Matched
    );
    assert_eq!(debug.reference_tick, debug.current_tick);
    assert_eq!(debug.confirmed_input_tick, debug.reference_tick - 1);
    assert!(
        debug.convergence.boundary != 999,
        "mutating a returned correction cannot change controller diagnostics"
    );
}

#[test]
fn settles_transport_incrementally_without_post_finish_match_frames() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(fixed_clock::TICK_SECONDS),
        RollbackPlayableLabOptions {
            local_slot: Some(1),
            profile: Some(fixed_profile(3, 0.0)),
            settlement_ticks: Some(16),
            ..Default::default()
        },
    );
    let first = rollback_playable_lab::advance(&mut lab, 0, Some(input_frame::neutral_sample()));
    let after_finish = rollback_playable_lab::debug_model(&lab);
    assert_eq!(after_finish.status, RollbackPlayableLabStatus::Settling);
    assert_eq!(first.outputs.len(), 1);
    let reference_tick = after_finish.reference_tick;

    let second = rollback_playable_lab::advance(&mut lab, 1, None);
    let after_settle = rollback_playable_lab::debug_model(&lab);
    assert_eq!(after_settle.reference_tick, reference_tick);
    assert_eq!(after_settle.transport_tick, 2);
    for output in &second.outputs {
        assert!(
            output.tick < reference_tick,
            "settlement may only replay existing ticks"
        );
    }
    let final_debug = run_to_terminal(&mut lab, 32);
    assert_eq!(final_debug.status, RollbackPlayableLabStatus::Converged);
    assert_eq!(final_debug.reference_tick, reference_tick);
}

#[test]
fn keeps_producing_reference_authority_after_a_predicted_early_finish() {
    // #489: same two-bound reasoning as tests/rollback_lab.rs's
    // `reactivates_a_predicted_early_finish_and_later_reaches_reference_full_time`
    // -- the delay has to be long enough for the local, uncorrected guess
    // (which does not yet know the bot-controlled defender is charging a
    // tackle) to actually reach a false "finished" conclusion given the
    // preceding fixture's now-20-tick `windup_timer` (25 ticks measured too
    // short), and short enough to stay inside
    // `rollback_input_history::ROLLBACK_WINDOW_TICKS` (30) or the run
    // terminates as unrecoverable instead of predicting early. 28 sits in
    // the middle of the working range (26-30 all pass).
    let mut lab = rollback_playable_lab::new(
        &preventable_goal_snapshot(),
        RollbackPlayableLabOptions {
            local_slot: Some(1),
            profile: Some(fixed_profile(28, 0.0)),
            profile_name: Some("predicted_early_finish".to_string()),
            bot_seed: Some(rollback_playable_lab::DEFAULT_BOT_SEED),
            settlement_ticks: Some(64),
            ..Default::default()
        },
    );
    let mut saw_predicted_finish = false;
    let mut reference_advanced_after_finish = false;
    let mut predicted_reference_tick: Option<i64> = None;
    for _ in 0..128 {
        let before = rollback_playable_lab::debug_model(&lab);
        if before.status != RollbackPlayableLabStatus::Active
            && before.status != RollbackPlayableLabStatus::Settling
        {
            break;
        }
        let sample =
            rollback_playable_lab::needs_local_sample(&lab).then(input_frame::neutral_sample);
        rollback_playable_lab::advance(&mut lab, before.transport_tick, sample);
        let after = rollback_playable_lab::debug_model(&lab);
        if after.predicted_early_finish {
            saw_predicted_finish = true;
            predicted_reference_tick = predicted_reference_tick.or(Some(after.reference_tick));
        } else if let Some(predicted) = predicted_reference_tick
            && after.reference_tick > predicted
        {
            reference_advanced_after_finish = true;
        }
    }
    let final_debug = rollback_playable_lab::debug_model(&lab);
    assert!(saw_predicted_finish);
    assert!(reference_advanced_after_finish);
    assert_eq!(final_debug.status, RollbackPlayableLabStatus::Converged);
    assert_eq!(
        final_debug.convergence.status,
        gc_sim::rollback_playable_lab::RollbackPlayableConvergenceStatus::Matched
    );
    assert_eq!(final_debug.current_tick, final_debug.reference_tick);
}

#[test]
fn confirms_before_appending_at_the_exact_event_window_capacity() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(31.0 * fixed_clock::TICK_SECONDS),
        RollbackPlayableLabOptions {
            local_slot: Some(1),
            profile: Some(fixed_profile(30, 0.0)),
            max_rollback_ticks: Some(30),
            settlement_ticks: Some(64),
            ..Default::default()
        },
    );
    for tick in 0..=30i64 {
        let batch =
            rollback_playable_lab::advance(&mut lab, tick, Some(input_frame::neutral_sample()));
        assert!(
            batch.status != RollbackPlayableLabStatus::UnconfirmedWindowExceeded,
            "delivery at the supported limit must free capacity before the new output"
        );
    }
    let debug = rollback_playable_lab::debug_model(&lab);
    assert_eq!(
        debug.event_status,
        gc_sim::rollback_events::RollbackEventsStatus::Active
    );
    assert!(debug.confirmed_output_tick >= 0);
}

#[test]
fn stops_explicitly_when_event_confirmation_exceeds_the_bounded_window() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(40.0 * fixed_clock::TICK_SECONDS),
        RollbackPlayableLabOptions {
            local_slot: Some(1),
            profile: Some(fixed_profile(31, 0.0)),
            max_rollback_ticks: Some(30),
            ..Default::default()
        },
    );
    let debug = run_to_terminal(&mut lab, 40);
    assert_eq!(
        debug.status,
        RollbackPlayableLabStatus::UnconfirmedWindowExceeded
    );
    assert_eq!(
        debug.event_status,
        gc_sim::rollback_events::RollbackEventsStatus::UnconfirmedWindowExceeded
    );
    let stopped_tick = debug.transport_tick;
    let batch = rollback_playable_lab::advance(&mut lab, stopped_tick, None);
    assert_eq!(
        batch.status,
        RollbackPlayableLabStatus::UnconfirmedWindowExceeded
    );
    assert_eq!(
        rollback_playable_lab::debug_model(&lab).transport_tick,
        stopped_tick
    );
}

#[test]
fn reports_bounded_drain_failure_instead_of_looping_or_inventing_input() {
    let mut lab = rollback_playable_lab::new(
        &initial_snapshot(fixed_clock::TICK_SECONDS),
        RollbackPlayableLabOptions {
            local_slot: Some(1),
            profile: Some(fixed_profile(0, 1.0)),
            settlement_ticks: Some(3),
            ..Default::default()
        },
    );
    let debug = run_to_terminal(&mut lab, 8);
    assert_eq!(debug.status, RollbackPlayableLabStatus::DrainIncomplete);
    assert_eq!(debug.settlement_ticks, 3);
    assert_eq!(debug.reference_tick, 1);
    assert_eq!(debug.current_tick, 1);
    assert_eq!(debug.confirmed_input_tick, -1);
}
