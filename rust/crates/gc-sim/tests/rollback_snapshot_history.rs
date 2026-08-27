//! Tests for `gc_sim::rollback_snapshot_history`.
//!
//! Every fixture here builds a `MatchState` directly through
//! `match_snapshot`'s public types rather than via `gc_sim::r#match::new` —
//! a hand-built minimal roster instead of real team data, the same
//! "minimal MatchState-shaped fixture" pattern
//! `tests/match_snapshot_differential.rs`'s `base_state` uses. No test in
//! this file drives a real `rollback_session`; that state machine is
//! exercised in its own `tests/rollback_session.rs`.

use gc_core::vec2::Vec2;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_snapshot::{CombatMatchState, CombatPlayerState};
use gc_sim::input_frame;
use gc_sim::match_snapshot::{
    self, ByTeam, MatchPlayer, MatchSnapshot, MatchState, PitchSize, Rect,
};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::possession_transition::{self, TransitionWindows};
use gc_sim::rollback_input_history::{self, RollbackInputSource};
use gc_sim::rollback_snapshot_history::{self, RollbackSnapshotLookupStatus};

#[allow(clippy::too_many_arguments)]
fn make_player(
    id: &str,
    team: match_snapshot::Team,
    is_keeper: bool,
    x: f64,
    y: f64,
) -> MatchPlayer {
    MatchPlayer {
        id: id.to_string(),
        name: format!("{id}_name"),
        team,
        pos: Vec2::new(x, y),
        vel: Vec2::new(0.0, 0.0),
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(
            if team == match_snapshot::Team::Home {
                1.0
            } else {
                -1.0
            },
            0.0,
        ),
        anchor: Vec2::new(x, y),
        species_id: "human_base".to_string(),
        owned_verb: gc_data::species::SimVerb::None,
        move_speed: 180.0,
        shot_speed: 500.0,
        dribble: 0.5,
        strength: 0.5,
        first_touch: 0.5,
        header_skill: 0.5,
        volley_skill: 0.5,
        bicycle_skill: 0.5,
        scan_rate: 0.5,
        composure: 0.5,
        outfield_decision: outfield_decision::new_state(None),
        is_keeper,
        radius: 12.0,
        dash_cd: 0.0,
        dodge_cd: 0.0,
        dodge_timer: 0.0,
        dodge_dir: Vec2::new(0.0, 0.0),
        reach: if is_keeper { 30.0 } else { 0.0 },
        handling: if is_keeper { 0.5 } else { 0.0 },
        keeper_aggression: if is_keeper { 40.0 } else { 0.0 },
        keeper_anticipation: if is_keeper { 0.5 } else { 0.0 },
        keeper_state: gc_sim::keeper::KeeperBehaviorState::Base,
        keeper_state_timer: 0.0,
        keeper_release_state: None,
        keeper_release_motion: 0.0,
        keeper_release_kind: None,
        keeper_release_depth: 0.0,
        keeper_set: 0.0,
        dive_timer: 0.0,
        dive_dir: Vec2::new(0.0, 0.0),
        dive_delay: 0.0,
        dive_target: None,
        keeper_get_up_timer: 0.0,
        hold_timer: 0.0,
        feet_ball: false,
        slide_timer: 0.0,
        slide_dir: Vec2::new(0.0, 0.0),
        slide_vel: 0.0,
        tackle_timer: 0.0,
        tackle_cd: 0.0,
        stun_timer: 0.0,
        grab_timer: 0.0,
        throw_timer: 0.0,
        receive_timer: 0.0,
        receive_target: None,
        sprint_meter: 1.0,
        sprint_dur: 3.0,
        sprinting: false,
        save_pending: None,
        save_timer: 0.0,
        save_vx: 0.0,
        save_style: None,
        save_tip_emitted: false,
        keeper_fatigue: 0.0,
        settle_timer: 0.0,
        header_cd: 0.0,
        aerial_timer: 0.0,
        aerial_style: None,
        aerial_outcome: None,
        aerial_jump: 0.0,
        aerial_recovery: 0.0,
        charge: 0.0,
        pass_charge: 0.0,
        pass_target: None,
        pass_intent: gc_sim::pass_intent::new_state(),
        windup_timer: 0.0,
        windup_shot: None,
        jockey_timer: 0.0,
        action: gc_sim::action_slot::new_state(),
    }
}

fn make_players() -> Vec<MatchPlayer> {
    use match_snapshot::Team::{Away, Home};
    vec![
        make_player("h_keeper", Home, true, 20.0, 270.0),
        make_player("h1", Home, false, 200.0, 150.0),
        make_player("h2", Home, false, 200.0, 390.0),
        make_player("h3", Home, false, 400.0, 200.0),
        make_player("h4", Home, false, 400.0, 340.0),
        make_player("a_keeper", Away, true, 940.0, 270.0),
        make_player("a1", Away, false, 760.0, 150.0),
        make_player("a2", Away, false, 760.0, 390.0),
        make_player("a3", Away, false, 560.0, 200.0),
        make_player("a4", Away, false, 560.0, 340.0),
    ]
}

/// Equivalent of the spec's `new_state()`: `match.new` is unavailable, so
/// this builds an already-valid `MatchState` fixture directly.
fn new_state() -> MatchState {
    MatchState {
        field: PitchSize { w: 960.0, h: 540.0 },
        goal_home: Rect {
            x: 0.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        goal_away: Rect {
            x: 950.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        players: make_players(),
        ball: Vec2::new(480.0, 270.0),
        ball_vel: Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner: None,
        controlled: 2,
        human_controlled: false,
        stick_latch: None,
        score: ByTeam { home: 0, away: 0 },
        time_left: 120.0,
        max_goals: 3,
        finished: false,
        pickup_cd: 0.0,
        press: ByTeam { home: 1, away: 1 },
        marking: ByTeam {
            home: MarkingConfig {
                scheme: MarkingScheme::Hybrid,
                man_marks: 1,
                standoff: 32.0,
                compactness: 0.5,
                support: 0.5,
            },
            away: MarkingConfig {
                scheme: MarkingScheme::Zonal,
                man_marks: 0,
                standoff: 40.0,
                compactness: 0.6,
                support: 0.4,
            },
        },
        marks: ByTeam {
            home: vec![None; 10],
            away: vec![None; 10],
        },
        outfield_press: ByTeam {
            home: outfield_press::new_state(),
            away: outfield_press::new_state(),
        },
        transition_windows: TransitionWindows {
            home: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
            away: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
        },
        transition: possession_transition::new_state(),
        formation: ByTeam {
            home: "2-1-1".to_string(),
            away: "2-1-1".to_string(),
        },
        ball_spin: 0.0,
        rng: gc_core::rng::seed(69.0),
        block_grace: 0.0,
        aerial_lock: 0.0,
        kickoff_hold: 0.0,
        events: Vec::new(),
        slot_mode: false,
        input_ownership: None,
        slot_players: vec![None; 8],
        slot_for_player: vec![None; 10],
        input_tick: 0,
        unsupported_reason: None,
    }
}

fn boundary(state: &mut MatchState, tick: i64) -> MatchSnapshot {
    state.input_tick = tick;
    match_snapshot::capture(state, None)
}

fn remote_sources() -> [RollbackInputSource; 8] {
    [RollbackInputSource::Remote; 8]
}

fn empty_combat_state(state: &MatchState, tick: i64) -> CombatMatchState {
    CombatMatchState {
        version: gc_sim::combat_snapshot::VERSION,
        tick,
        player_ids: state.players.iter().map(|p| p.id.clone()).collect(),
        players: (0..state.players.len())
            .map(|_| CombatPlayerState {
                loadout_id: None,
                family_id: None,
                phase: CombatActionPhase::Ready,
                phase_ticks: 0,
                cooldown_ticks: 0,
                source_sequence: None,
                contacted: false,
                release_latched: false,
                control_held: false,
                projectile_spawned: false,
                forced_state: None,
                forced_ticks: 0,
                chain_ticks: 0,
                immunity_ticks: 0,
                intent: gc_sim::combat_intent::new_state(),
            })
            .collect(),
        projectiles: Vec::new(),
        events: Vec::new(),
        next_source_sequence: 1,
    }
}

#[test]
fn rollback_snapshot_history_retains_present_boundary_plus_thirty_prior() {
    let mut history = rollback_snapshot_history::new(None);
    let mut state = new_state();
    for tick in 0..=31 {
        rollback_snapshot_history::store(&mut history, &boundary(&mut state, tick)).unwrap();
    }

    let diagnostics = rollback_snapshot_history::diagnostics(&history);
    assert_eq!(diagnostics.max_rollback_ticks, 30);
    assert_eq!(diagnostics.capacity, 31);
    assert_eq!(diagnostics.retained_boundary_count, 31);
    assert_eq!(diagnostics.oldest_supported_tick, Some(1));
    assert_eq!(diagnostics.oldest_boundary_tick, Some(1));
    assert_eq!(diagnostics.latest_tick, Some(31));
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 31).status,
        RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 1).status,
        RollbackSnapshotLookupStatus::Retained
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 0).status,
        RollbackSnapshotLookupStatus::OutsideWindow
    );
}

#[test]
fn rollback_snapshot_history_distinguishes_missing_boundaries_inside_supported_range() {
    let mut history = rollback_snapshot_history::new(Some(3));
    let mut state = new_state();
    rollback_snapshot_history::store(&mut history, &boundary(&mut state, 10)).unwrap();
    rollback_snapshot_history::store(&mut history, &boundary(&mut state, 12)).unwrap();

    assert_eq!(
        rollback_snapshot_history::lookup(&history, 12).status,
        RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 10).status,
        RollbackSnapshotLookupStatus::Retained
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 11).status,
        RollbackSnapshotLookupStatus::Missing
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 8).status,
        RollbackSnapshotLookupStatus::OutsideWindow
    );
    let rejected = rollback_snapshot_history::store(&mut history, &boundary(&mut state, 8));
    assert_eq!(
        rejected.unwrap_err().code,
        gc_sim::rollback_snapshot_history::RollbackSnapshotHistoryErrorCode::OutsideWindow
    );
}

#[test]
fn rollback_snapshot_history_owns_independent_stored_and_returned_snapshots() {
    let mut history = rollback_snapshot_history::new(None);
    let mut state = new_state();
    let mut supplied = boundary(&mut state, 0);
    let original_x = supplied.state.players[0].pos.x;
    rollback_snapshot_history::store(&mut history, &supplied).unwrap();
    supplied.state.players[0].pos.x = original_x + 100.0;

    let first = rollback_snapshot_history::lookup(&history, 0)
        .snapshot
        .unwrap();
    assert_eq!(first.state.players[0].pos.x, original_x);
    let mut first = first;
    first.state.players[0].pos.x = original_x - 100.0;
    let second = rollback_snapshot_history::lookup(&history, 0)
        .snapshot
        .unwrap();
    assert_eq!(second.state.players[0].pos.x, original_x);

    let (restored, status) = rollback_snapshot_history::restore(&history, 0);
    assert_eq!(status, RollbackSnapshotLookupStatus::Present);
    let mut restored = restored.unwrap();
    restored.players[0].pos.x = original_x + 50.0;
    let (unchanged, _) = rollback_snapshot_history::restore(&history, 0);
    assert_eq!(unchanged.unwrap().players[0].pos.x, original_x);
    assert_eq!(
        rollback_snapshot_history::status(&history, 0),
        RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(
        rollback_snapshot_history::status(&history, 1),
        RollbackSnapshotLookupStatus::Missing
    );
}

#[test]
fn rollback_snapshot_history_stores_accounts_and_restores_combat_companion_atomically() {
    let mut history = rollback_snapshot_history::new(None);
    let mut state = new_state();
    state.kickoff_hold = 0.0;
    state.input_tick = 0;
    let combat_state = empty_combat_state(&state, 0);
    let supplied = match_snapshot::capture(&state, Some(&combat_state));
    let expected_bytes = match_snapshot::encoded_size_canonical(&supplied) as i64;
    rollback_snapshot_history::store(&mut history, &supplied).unwrap();
    let mut supplied = supplied;
    supplied.combat.as_mut().unwrap().next_source_sequence = 99;

    let lookup = rollback_snapshot_history::lookup(&history, 0);
    assert_eq!(lookup.canonical_bytes, Some(expected_bytes));
    assert_eq!(
        lookup.snapshot.as_ref().unwrap().version,
        match_snapshot::COMBAT_VERSION
    );
    assert_eq!(
        lookup
            .snapshot
            .unwrap()
            .combat
            .unwrap()
            .next_source_sequence,
        1
    );

    let (restored, restored_combat, status) =
        rollback_snapshot_history::restore_simulation(&history, 0);
    assert_eq!(status, RollbackSnapshotLookupStatus::Present);
    assert_eq!(restored.unwrap().input_tick, 0);
    let mut restored_combat = restored_combat.unwrap();
    assert_eq!(restored_combat.next_source_sequence, 1);
    restored_combat.next_source_sequence = 55;
    let (_, unchanged, _) = rollback_snapshot_history::restore_simulation(&history, 0);
    assert_eq!(unchanged.unwrap().next_source_sequence, 1);
    assert_eq!(
        rollback_snapshot_history::diagnostics(&history).canonical_bytes,
        expected_bytes
    );
}

#[test]
fn rollback_snapshot_history_compares_retained_boundaries_and_materializes_hashes_only_for_divergence()
 {
    let mut expected = rollback_snapshot_history::new(Some(2));
    let mut actual = rollback_snapshot_history::new(Some(2));
    let mut state = new_state();
    rollback_snapshot_history::store(&mut expected, &boundary(&mut state, 0)).unwrap();
    rollback_snapshot_history::store(&mut actual, &boundary(&mut state, 0)).unwrap();

    let matched = rollback_snapshot_history::compare(&mut expected, &mut actual, 0);
    assert!(matched.matched);
    assert_eq!(
        matched.expected_status,
        RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(matched.actual_status, RollbackSnapshotLookupStatus::Present);
    assert_eq!(matched.expected_hash, None);
    assert_eq!(matched.actual_hash, None);
    assert_eq!(matched.first_difference, None);

    state.score.home = 1;
    rollback_snapshot_history::store(&mut actual, &boundary(&mut state, 0)).unwrap();
    let diverged = rollback_snapshot_history::compare(&mut expected, &mut actual, 0);
    assert!(!diverged.matched);
    assert_eq!(diverged.first_difference.unwrap().path, "state.score.home");
    assert_ne!(diverged.expected_hash, diverged.actual_hash);

    rollback_snapshot_history::store(&mut expected, &boundary(&mut state, 3)).unwrap();
    let missing = rollback_snapshot_history::compare(&mut expected, &mut actual, 3);
    assert!(!missing.matched);
    assert_eq!(
        missing.expected_status,
        RollbackSnapshotLookupStatus::Present
    );
    assert_eq!(missing.actual_status, RollbackSnapshotLookupStatus::Missing);
}

#[test]
fn rollback_snapshot_history_rejects_retained_boundaries_differing_only_by_windup_shot_type() {
    let mut expected = rollback_snapshot_history::new(Some(2));
    let mut actual = rollback_snapshot_history::new(Some(2));
    let mut state = new_state();
    state.players[1].windup_shot = Some(match_snapshot::WindupShot {
        dir: state.players[1].facing,
        speed: 456.0,
        vz: 123.0,
        spin: -8.0,
        shot_type: gc_sim::keeper::KeeperShotType::Chip,
    });
    rollback_snapshot_history::store(&mut expected, &boundary(&mut state, 0)).unwrap();
    state.players[1].windup_shot.as_mut().unwrap().shot_type =
        gc_sim::keeper::KeeperShotType::Ground;
    rollback_snapshot_history::store(&mut actual, &boundary(&mut state, 0)).unwrap();

    let comparison = rollback_snapshot_history::compare(&mut expected, &mut actual, 0);
    assert!(!comparison.matched);
    assert_eq!(
        comparison.first_difference.unwrap().path,
        "state.players.2.windup_shot.shot_type"
    );
    assert_ne!(comparison.expected_hash, comparison.actual_hash);
}

#[test]
fn rollback_snapshot_history_updates_byte_accounting_and_invalidates_replaced_lazy_hash() {
    let mut history = rollback_snapshot_history::new(Some(1));
    let mut state = new_state();
    let initial = boundary(&mut state, 0);
    let initial_bytes = match_snapshot::encode(&initial).len() as i64;
    let first = rollback_snapshot_history::store(&mut history, &initial).unwrap();
    assert!(!first.replaced);
    // Capacity 1: the ring's sole 0-based slot is index 0 (ARCHITECTURE.md
    // §3 rule 3).
    assert_eq!(history.entries[0].as_ref().unwrap().canonical_wire, None);
    assert_eq!(
        rollback_snapshot_history::diagnostics(&history).canonical_bytes,
        initial_bytes
    );
    let (first_hash, _) = rollback_snapshot_history::boundary_hash(&mut history, 0);
    let first_hash = first_hash.unwrap();
    assert_eq!(
        history.entries[0]
            .as_ref()
            .unwrap()
            .canonical_wire
            .as_ref()
            .unwrap()
            .len() as i64,
        initial_bytes
    );

    state.score.home = 1;
    let replacement = boundary(&mut state, 0);
    let replacement_bytes = match_snapshot::encode(&replacement).len() as i64;
    let replaced = rollback_snapshot_history::store(&mut history, &replacement).unwrap();
    assert!(replaced.replaced);
    let diagnostics = rollback_snapshot_history::diagnostics(&history);
    assert_eq!(diagnostics.retained_boundary_count, 1);
    assert_eq!(diagnostics.canonical_bytes, replacement_bytes);
    let (replacement_hash, _) = rollback_snapshot_history::boundary_hash(&mut history, 0);
    assert_ne!(replacement_hash.unwrap(), first_hash);

    let next_boundary = boundary(&mut state, 1);
    let next_bytes = match_snapshot::encode(&next_boundary).len() as i64;
    rollback_snapshot_history::store(&mut history, &next_boundary).unwrap();
    assert_eq!(
        rollback_snapshot_history::diagnostics(&history).canonical_bytes,
        replacement_bytes + next_bytes
    );
    let third_boundary = boundary(&mut state, 2);
    let third_bytes = match_snapshot::encode(&third_boundary).len() as i64;
    let advanced = rollback_snapshot_history::store(&mut history, &third_boundary).unwrap();
    assert_eq!(advanced.evicted, 1);
    assert_eq!(
        rollback_snapshot_history::diagnostics(&history).canonical_bytes,
        next_bytes + third_bytes
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 0).status,
        RollbackSnapshotLookupStatus::OutsideWindow
    );
}

#[test]
fn rollback_snapshot_history_evicts_deterministically_across_wraparound_and_sparse_advances() {
    let mut left = rollback_snapshot_history::new(Some(3));
    let mut right = rollback_snapshot_history::new(Some(3));
    let mut state = new_state();
    for tick in [0, 1, 4, 6, 7] {
        rollback_snapshot_history::store(&mut left, &boundary(&mut state, tick)).unwrap();
        rollback_snapshot_history::store(&mut right, &boundary(&mut state, tick)).unwrap();
    }
    let left_diagnostics = rollback_snapshot_history::diagnostics(&left);
    let right_diagnostics = rollback_snapshot_history::diagnostics(&right);
    assert_eq!(
        left_diagnostics.retained_boundary_count,
        right_diagnostics.retained_boundary_count
    );
    assert_eq!(
        left_diagnostics.canonical_bytes,
        right_diagnostics.canonical_bytes
    );
    for tick in 0..=7 {
        let left_lookup = rollback_snapshot_history::lookup(&left, tick);
        let right_lookup = rollback_snapshot_history::lookup(&right, tick);
        assert_eq!(left_lookup.status, right_lookup.status);
        if left_lookup.snapshot.is_some() {
            let (left_hash, _) = rollback_snapshot_history::boundary_hash(&mut left, tick);
            let (right_hash, _) = rollback_snapshot_history::boundary_hash(&mut right, tick);
            assert_eq!(left_hash, right_hash);
        }
    }
}

#[test]
fn rollback_snapshot_history_rejects_malformed_missing_and_outside_floor_truncation_without_mutation()
 {
    let mut history = rollback_snapshot_history::new(Some(2));
    let mut state = new_state();
    rollback_snapshot_history::store(&mut history, &boundary(&mut state, 5)).unwrap();
    rollback_snapshot_history::store(&mut history, &boundary(&mut state, 7)).unwrap();
    let before = rollback_snapshot_history::diagnostics(&history);

    let outside = rollback_snapshot_history::truncate_after(&mut history, 4);
    assert_eq!(
        outside.unwrap_err().code,
        gc_sim::rollback_snapshot_history::RollbackSnapshotHistoryErrorCode::OutsideWindow
    );
    let missing = rollback_snapshot_history::truncate_after(&mut history, 6);
    assert_eq!(
        missing.unwrap_err().code,
        gc_sim::rollback_snapshot_history::RollbackSnapshotHistoryErrorCode::Missing
    );
    // A fractional tick (e.g. `6.5`) cannot be constructed here at all
    // (`i64` cannot be fractional); an out-of-range tick exercises the same
    // `Malformed` branch instead.
    let malformed = rollback_snapshot_history::truncate_after(&mut history, -1);
    assert_eq!(
        malformed.unwrap_err().code,
        gc_sim::rollback_snapshot_history::RollbackSnapshotHistoryErrorCode::Malformed
    );

    let after = rollback_snapshot_history::diagnostics(&history);
    assert_eq!(after.latest_tick, before.latest_tick);
    assert_eq!(after.oldest_supported_tick, before.oldest_supported_tick);
    assert_eq!(
        after.retained_boundary_count,
        before.retained_boundary_count
    );
    assert_eq!(after.canonical_bytes, before.canonical_bytes);
}

#[test]
fn rollback_snapshot_history_discards_corrected_full_time_tail_at_exact_causal_boundaries() {
    let mut snapshots = rollback_snapshot_history::new(Some(10));
    let mut inputs = rollback_input_history::new(remote_sources());
    let mut state = new_state();
    for tick in 0..=5 {
        rollback_snapshot_history::store(&mut snapshots, &boundary(&mut state, tick)).unwrap();
        if tick < 5 {
            rollback_input_history::materialize(&mut inputs, tick);
        }
    }
    let (obsolete_hash, _) = rollback_snapshot_history::boundary_hash(&mut snapshots, 5);
    assert_ne!(obsolete_hash.unwrap(), "");

    let sample = input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(70),
        ..Default::default()
    })
    .unwrap();
    rollback_input_history::add_authoritative(&mut inputs, 4, 1, sample).unwrap();
    assert_eq!(
        rollback_input_history::earliest_divergence(&inputs),
        Some(4)
    );

    // The corrected timeline reaches full time at boundary 3: frames 3+ and
    // snapshots 4+ belong only to the obsolete predicted timeline.
    state.finished = true;
    rollback_snapshot_history::store(&mut snapshots, &boundary(&mut state, 3)).unwrap();
    let mut retained_bytes: i64 = 0;
    for tick in 0..=3 {
        retained_bytes += rollback_snapshot_history::lookup(&snapshots, tick)
            .canonical_bytes
            .unwrap();
    }
    let (retained_hash, _) = rollback_snapshot_history::boundary_hash(&mut snapshots, 3);
    let retained_hash = retained_hash.unwrap();
    let snapshot_tail = rollback_snapshot_history::truncate_after(&mut snapshots, 3).unwrap();
    let input_tail = rollback_input_history::truncate_from(&mut inputs, 3).unwrap();

    assert_eq!(snapshot_tail.removed, 2);
    assert_eq!(snapshot_tail.diagnostics.latest_tick, Some(3));
    assert_eq!(snapshot_tail.diagnostics.retained_boundary_count, 4);
    assert_eq!(snapshot_tail.diagnostics.canonical_bytes, retained_bytes);
    assert_eq!(snapshot_tail.diagnostics.peak_retained_boundary_count, 6);
    assert!(
        snapshot_tail.diagnostics.peak_canonical_bytes > snapshot_tail.diagnostics.canonical_bytes
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&snapshots, 3).status,
        RollbackSnapshotLookupStatus::Present
    );
    assert!(
        rollback_snapshot_history::lookup(&snapshots, 3)
            .snapshot
            .unwrap()
            .state
            .finished
    );
    assert_eq!(
        rollback_snapshot_history::lookup(&snapshots, 4).status,
        RollbackSnapshotLookupStatus::Missing
    );
    let (kept_hash, _) = rollback_snapshot_history::boundary_hash(&mut snapshots, 3);
    assert_eq!(kept_hash.unwrap(), retained_hash);
    let (removed_hash, removed_status) =
        rollback_snapshot_history::boundary_hash(&mut snapshots, 5);
    assert_eq!(removed_hash, None);
    assert_eq!(removed_status, RollbackSnapshotLookupStatus::Missing);

    assert_eq!(input_tail.effective_removed, 2);
    assert_eq!(input_tail.records_removed, 2);
    assert!(input_tail.cleared_divergence);
    assert_eq!(input_tail.diagnostics.effective_tick_count, 3);
    assert_eq!(input_tail.diagnostics.record_tick_count, 3);
    assert_eq!(rollback_input_history::record(&inputs, 3), None);
    assert_eq!(rollback_input_history::earliest_divergence(&inputs), None);
    assert!(rollback_input_history::authoritative_record(&inputs, 4, 1).is_some());

    let later_sample = input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(-70),
        ..Default::default()
    })
    .unwrap();
    let later = rollback_input_history::add_authoritative(&mut inputs, 4, 2, later_sample).unwrap();
    assert_eq!(
        later.earliest_divergence, None,
        "discarded input was never used by this timeline"
    );
}

#[test]
fn rollback_snapshot_history_retains_final_full_time_boundary_without_inventing_another_input() {
    let mut history = rollback_snapshot_history::new(None);
    let mut state = new_state();
    rollback_snapshot_history::store(&mut history, &boundary(&mut state, 6)).unwrap();
    state.finished = true;
    rollback_snapshot_history::store(&mut history, &boundary(&mut state, 7)).unwrap();

    let final_lookup = rollback_snapshot_history::lookup(&history, 7);
    assert_eq!(final_lookup.status, RollbackSnapshotLookupStatus::Present);
    assert!(final_lookup.snapshot.unwrap().state.finished);
    assert_eq!(
        rollback_snapshot_history::lookup(&history, 8).status,
        RollbackSnapshotLookupStatus::Missing
    );
    assert_eq!(
        rollback_snapshot_history::diagnostics(&history).latest_tick,
        Some(7)
    );
}
