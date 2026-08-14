//! Tests for `gc_sim::replay`.
//!
//! `short_match_tape` below builds a known-good fixture (a fixed seed, fixed
//! frames, and the pinned `EXPECTED_BOUNDARY_HASHES`) as a local test
//! helper.
//!
//! Every `match_snapshot::capture` call in this file is preceded by the
//! same `normalize_marks` compensating step documented at length in
//! `crate::input_tape`'s module doc (a `sim::match`/`match_snapshot`
//! contract mismatch, not owned by this file) — see that doc comment for
//! the full report.
//!
//! Some malformed-shape cases are not exercised here: constructing a value
//! with an extra field, a missing field, or a field of the wrong type is
//! not possible in Rust's static type system — the compiler enforces the
//! same invariant more strongly than a runtime check could. Each omitted
//! case is called out at its site with the same reasoning `input_frame.rs`'s
//! module doc already established for this crate.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::teams;
use gc_sim::combat;
use gc_sim::combat_identity;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputFrame};
use gc_sim::input_tape::{self, InputTape, InputTapeIdentity};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{self, MatchState, PitchSize};
use gc_sim::replay::{self, ReplayFailureCode};
use gc_sim::tuning::Tuning;

// Boundary 0 is the CAPTURED initial state and is unchanged by #488 -- the
// locomotion rework alters how bodies move, not what a fresh match looks like.
// Boundaries 1..3 are three stepped ticks of that state, so they move with any
// deliberate gameplay change; these three were re-derived from this build for
// #488. They are a self-recorded baseline, not frozen cross-implementation
// evidence: they detect change, they cannot detect "wrong but consistently
// wrong". What this file actually proves -- that `replay` reports the first
// diverging boundary, rejects tampering, and refuses mismatched identities --
// is asserted against these hashes rather than by them.
//
// #531 phase 2 EXCEPTION: unlike #488, this bumped `match_snapshot::VERSION`
// (11 -> 12, the new `pass_intent` field on every `MatchPlayer`), which
// changes the canonical byte encoding of every captured snapshot including
// boundary 0's zero-tick kickoff state -- so this one time all four values
// below moved together, for the schema reason and no other.
const EXPECTED_BOUNDARY_HASHES: [&str; 4] = [
    "9447e27f341cfaf9",
    "e598a0f82e726e86",
    "d1da2f62315fa218",
    "a82ccba59631f487",
];

/// Compensating normalization for the `sim::match`/`match_snapshot` marks
/// contract mismatch — see `crate::input_tape::normalize_marks`'s doc
/// comment (`crates/gc-sim/src/input_tape.rs`) for the full report.
fn normalize_marks(state: &mut MatchState) {
    let n = state.players.len();
    state.marks.home.resize(n, None);
    state.marks.away.resize(n, None);
}

fn copy_frames(tape: &InputTape) -> Vec<InputFrame> {
    tape.frames
        .iter()
        .map(|f| input_frame::copy(f).expect("recorded frame is already valid"))
        .collect()
}

fn short_match_tape() -> (InputTape, InputTapeIdentity) {
    let tune = Tuning::new();
    let home = teams::get("nebula").expect("nebula is an authored team");
    let away = teams::get("orion").expect("orion is an authored team");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(fixed_clock::TICK_SECONDS * 2.5),
        max_goals: Some(3),
        seed: Some(38.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    normalize_marks(&mut state);

    let mut slots = [input_frame::InputSample::default(); 8];
    let mut frames = Vec::with_capacity(3);
    frames.push(input_frame::neutral(0).expect("canonical frame"));
    slots[0] = input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(input_frame::MOVE_SCALE),
        ..Default::default()
    })
    .expect("canonical sample");
    let mut frame1 = input_frame::neutral(1).expect("canonical frame");
    frame1.slots[0] = slots[0];
    frames.push(frame1);
    frames.push(input_frame::neutral(2).expect("canonical frame"));

    let identity = InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "spec-build".to_string(),
        source: "197ce23264abff1d6ac0c71e9b313ee820814f7e".to_string(),
        content: "showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: "field=960x540;duration_ticks=2.5;max_goals=3".to_string(),
        fixture: "nebula-v-orion;balanced-v-balanced".to_string(),
        seed: 38.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: None,
    };
    let initial = match_snapshot::capture(&state, None);
    let tape = input_tape::new(&identity, &initial, &frames, &tune).expect("tape constructs");
    assert_eq!(tape.boundary_hashes.len(), EXPECTED_BOUNDARY_HASHES.len());
    for (index, expected) in EXPECTED_BOUNDARY_HASHES.iter().enumerate() {
        assert_eq!(
            &tape.boundary_hashes[index], expected,
            "short tape boundary {index} hash drifted; observed sequence={:?}",
            tape.boundary_hashes
        );
    }
    (tape, identity)
}

#[test]
fn input_tape_replay_pins_combat_mechanics_in_a_v2_tape_and_replays_every_composite_boundary() {
    let tune = Tuning::new();
    let home = teams::get("nebula").expect("nebula is an authored team");
    let away = teams::get("orion").expect("orion is an authored team");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(2.0),
        max_goals: Some(3),
        seed: Some(811.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    normalize_marks(&mut state);
    let source_player = combat_state
        .players
        .iter()
        .position(|runtime| runtime.family_id == Some(ActionFamilyId::Ranged))
        .expect("fixture requires one ranged player");
    let source_slot = state.slot_for_player[source_player].expect("ranged player owns a slot");

    let mut frames = Vec::with_capacity(6);
    for tick in 0..=5i64 {
        frames.push(input_frame::neutral(tick).expect("canonical frame"));
    }
    frames[0].slots[(source_slot - 1) as usize] =
        input_frame::new_sample(input_frame::InputSampleOptions {
            held: Some(input_frame::HELD_EQUIPMENT),
            edges: Some(input_frame::EDGE_EQUIPMENT_PRESSED),
            ..Default::default()
        })
        .expect("canonical sample");
    for frame in frames.iter_mut().skip(1) {
        frame.slots[(source_slot - 1) as usize] =
            input_frame::new_sample(input_frame::InputSampleOptions {
                held: Some(input_frame::HELD_EQUIPMENT),
                ..Default::default()
            })
            .expect("canonical sample");
    }

    let identity = InputTapeIdentity {
        tape_version: input_tape::COMBAT_VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::COMBAT_VERSION,
        build: "combat-v2-spec".to_string(),
        source: "combat-v2-spec".to_string(),
        content: "nebula-orion-showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: "field=960x540;duration=2;max_goals=3;tick_rate=60".to_string(),
        fixture: "combat-v2-spec".to_string(),
        seed: 811.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: Some(combat_identity::for_state(Some(&combat_state))),
    };
    let initial = match_snapshot::capture(&state, Some(&combat_state));
    let tape = input_tape::new(&identity, &initial, &frames, &tune).expect("tape constructs");
    let replayed = replay::run(&tape, &identity, &tune).expect("replay context is valid");
    assert_eq!(tape.version, input_tape::COMBAT_VERSION);
    assert_eq!(replayed.boundaries.len(), frames.len() + 1);
    assert_eq!(
        replayed
            .combat_state
            .as_ref()
            .expect("combat companion present")
            .players[source_player]
            .phase,
        gc_sim::combat_snapshot::CombatPhase::Windup
    );
    for (index, row) in replayed.boundaries.iter().enumerate() {
        assert_eq!(row.hash, tape.boundary_hashes[index]);
    }

    let (mirror_state, mirror_combat) = match_snapshot::restore(&tape.initial);
    let mirror_initial = match_snapshot::capture(&mirror_state, mirror_combat.as_ref());
    let mirror = input_tape::new(&identity, &mirror_initial, &copy_frames(&tape), &tune)
        .expect("mirror tape constructs");
    let compared = replay::compare(&tape, &mirror, &identity, &tune).expect("comparison succeeds");
    assert!(compared.equal);
    for (index, hash) in tape.boundary_hashes.iter().enumerate() {
        assert_eq!(&mirror.boundary_hashes[index], hash);
    }

    let mut mismatched = input_tape::copy_identity(&identity).expect("identity is valid");
    mismatched.combat = mismatched.combat.map(|c| format!("{c};changed"));
    let failure = replay::run(&tape, &mismatched, &tune).expect_err("combat identity mismatched");
    assert_eq!(failure.code, ReplayFailureCode::IdentityMismatch);
    assert_eq!(failure.path.as_deref(), Some("identity.combat"));

    let mut soccer_identity = input_tape::copy_identity(&identity).expect("identity is valid");
    soccer_identity.tape_version = input_tape::VERSION;
    soccer_identity.snapshot_version = match_snapshot::VERSION;
    soccer_identity.combat = None;
    assert!(
        input_tape::new(&soccer_identity, &initial, &frames, &tune).is_err(),
        "a soccer identity cannot describe a combat-active snapshot"
    );
}

#[test]
fn input_tape_replay_converges_through_a_v5_goal_kickoff_reset_and_post_kickoff_boundary() {
    let tune = Tuning::new();
    let home = teams::get("nebula").expect("nebula is an authored team");
    let away = teams::get("orion").expect("orion is an authored team");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(2.0),
        max_goals: Some(3),
        seed: Some(83.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    {
        let away_keeper = &mut state.players[5];
        away_keeper.keeper_state = KeeperBehaviorState::Retreat;
        away_keeper.keeper_state_timer = 0.1;
        away_keeper.keeper_release_state = Some(KeeperBehaviorState::Advance);
        away_keeper.keeper_release_motion = 0.5;
        away_keeper.keeper_release_kind = Some(KeeperShotType::Chip);
        away_keeper.keeper_release_depth = 42.0;
        away_keeper.receive_timer = 1.0;
    }
    state.owner = None;
    state.ball = Vec2::new(965.0, 270.0);
    state.ball_vel = Vec2::new(600.0, 0.0);
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 1.0;
    state.block_grace = 1.0;
    normalize_marks(&mut state);

    let initial = match_snapshot::capture(&state, None);
    let frames = vec![
        input_frame::neutral(0).expect("canonical frame"),
        input_frame::neutral(1).expect("canonical frame"),
        input_frame::neutral(2).expect("canonical frame"),
    ];
    let identity = InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "goal-kickoff-v5-spec".to_string(),
        source: "synthetic-goal-kickoff-v1".to_string(),
        content: "nebula-orion-showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: "field=960x540;duration=2;max_goals=3;tick_rate=60".to_string(),
        fixture: "goal-kickoff-v5-spec".to_string(),
        seed: 83.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: None,
    };
    let tape = input_tape::new(&identity, &initial, &frames, &tune).expect("tape constructs");
    let replayed = replay::run(&tape, &identity, &tune).expect("replay context is valid");

    assert_eq!(replayed.boundaries.len(), 4);
    for (index, boundary) in replayed.boundaries.iter().enumerate() {
        assert_eq!(
            boundary.hash, tape.boundary_hashes[index],
            "goal/kickoff boundary {index}"
        );
    }
    assert_eq!(replayed.state.score.home, 1);
    assert!(replayed.state.kickoff_hold > 0.0);
    let owner = replayed.state.owner.expect("ball is owned after kickoff");
    assert_eq!(
        replayed.state.players[(owner - 1) as usize].team,
        match_snapshot::Team::Away
    );
    assert_eq!(
        replayed.state.players[5].keeper_state,
        KeeperBehaviorState::Base
    );
    assert_eq!(replayed.state.players[5].keeper_release_state, None);
    assert_eq!(replayed.state.players[5].keeper_release_kind, None);
    assert!(replayed.divergence.is_none());

    let (restored_state, restored_combat) = match_snapshot::restore(&initial);
    let mirror_initial = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    let mirror = input_tape::new(&identity, &mirror_initial, &frames, &tune)
        .expect("mirror tape constructs");
    let compared = replay::compare(&tape, &mirror, &identity, &tune).expect("comparison succeeds");
    assert!(compared.equal);
    assert!(compared.divergence.is_none());
}

#[test]
fn input_tape_replay_replays_a_short_complete_match_with_every_boundary_hash() {
    let tune = Tuning::new();
    let (tape, identity) = short_match_tape();
    let replayed = replay::run(&tape, &identity, &tune).expect("replay context is valid");
    assert_eq!(replayed.boundaries.len(), tape.frames.len() + 1);
    for (index, expected) in EXPECTED_BOUNDARY_HASHES
        .iter()
        .enumerate()
        .take(replayed.boundaries.len())
    {
        assert_eq!(
            &tape.boundary_hashes[index], expected,
            "constructed tape boundary {index}"
        );
        assert_eq!(
            &replayed.boundaries[index].hash, expected,
            "replay boundary {index}"
        );
    }
    assert!(replayed.state.finished, "the fixture reaches its match end");
    assert_eq!(replayed.state.input_tick, 3);
    assert!(replayed.divergence.is_none());

    let mut replayed = replayed;
    replayed.state.ball.x = -500.0;
    let second = replay::run(&tape, &identity, &tune).expect("replay context is valid");
    assert!(
        second.state.ball.x != -500.0,
        "each replay returns independent final state"
    );
}

#[test]
fn input_tape_replay_deep_copies_snapshot_frames_identity_and_ownership_at_construction() {
    let tune = Tuning::new();
    let (tape, identity) = short_match_tape();
    let mut frames = copy_frames(&tape);
    let (restored_state, restored_combat) = match_snapshot::restore(&tape.initial);
    let mut initial = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    let copied = input_tape::new(&identity, &initial, &frames, &tune).expect("tape constructs");

    frames[0].slots[0].move_x = -127;
    initial.state.ball.x = -1.0;
    let mut identity = identity;
    identity.ownership.slots[0].player_id = "changed".to_string();
    identity.config = "changed".to_string();

    assert_eq!(
        copied.frames[0].slots[0].move_x,
        tape.frames[0].slots[0].move_x
    );
    assert!(copied.initial.state.ball.x != -1.0);
    assert!(copied.identity.ownership.slots[0].player_id != "changed");
    assert!(copied.identity.config != "changed");
}

#[test]
fn input_tape_replay_accepts_and_owns_independently_verified_frozen_boundary_recordings() {
    let tune = Tuning::new();
    let (tape, identity) = short_match_tape();
    let mut frames = copy_frames(&tape);
    let hashes = tape.boundary_hashes.clone();
    let frozen =
        input_tape::from_frozen_recording(&identity, &tape.initial, &frames, &hashes, &tune)
            .expect("frozen recording accepted");
    assert!(input_tape::validate_structure(&frozen).is_ok());
    assert_eq!(frozen.boundary_hashes.last(), hashes.last());

    let mut hashes = hashes;
    hashes[0] = "0000000000000000".to_string();
    frames[0].slots[0].move_x = if frozen.frames[0].slots[0].move_x == -127 {
        127
    } else {
        -127
    };
    assert_ne!(frozen.boundary_hashes[0], hashes[0]);
    assert_ne!(frozen.frames[0].slots[0].move_x, frames[0].slots[0].move_x);

    let mut malformed = frozen.boundary_hashes.clone();
    malformed[0] = "0000000000000000".to_string();
    assert!(
        input_tape::from_frozen_recording(
            &identity,
            &tape.initial,
            &copy_frames(&tape),
            &malformed,
            &tune
        )
        .is_err(),
        "a boundary hash that disagrees with the snapshot must be rejected"
    );
}

#[test]
fn input_tape_replay_rejects_identity_and_active_tuning_mismatches_separately() {
    let tune = Tuning::new();
    let (tape, identity) = short_match_tape();
    let mut wrong = input_tape::copy_identity(&identity).expect("identity is valid");
    wrong.content = "different-content".to_string();
    let failure = replay::run(&tape, &wrong, &tune).expect_err("content mismatched");
    assert_eq!(failure.code, ReplayFailureCode::IdentityMismatch);
    assert_eq!(failure.path.as_deref(), Some("identity.content"));

    let mut tuned_tune = Tuning::new();
    tuned_tune.set("AI_SHOOT_RANGE", tuned_tune.value("AI_SHOOT_RANGE") + 1.0);
    let tune_failure = replay::run(&tape, &identity, &tuned_tune).expect_err("tuning mismatched");
    assert_eq!(tune_failure.code, ReplayFailureCode::IdentityMismatch);
    assert_eq!(tune_failure.path.as_deref(), Some("identity.tuning"));
}

#[test]
fn input_tape_replay_rejects_identity_before_simulation_aware_tape_validation() {
    let tune = Tuning::new();
    let (mut tape, identity) = short_match_tape();
    tape.frames
        .push(input_frame::neutral(3).expect("canonical frame"));
    tape.boundary_hashes.push(
        tape.boundary_hashes
            .last()
            .expect("at least one boundary")
            .clone(),
    );

    let mut wrong = input_tape::copy_identity(&identity).expect("identity is valid");
    wrong.content = "different-content".to_string();
    let mismatch = replay::run(&tape, &wrong, &tune).expect_err("content mismatched");
    assert_eq!(mismatch.code, ReplayFailureCode::IdentityMismatch);
    assert_eq!(mismatch.path.as_deref(), Some("identity.content"));

    let mut tuned_tune = Tuning::new();
    tuned_tune.set("AI_SHOOT_RANGE", tuned_tune.value("AI_SHOOT_RANGE") + 1.0);
    let tune_mismatch = replay::run(&tape, &identity, &tuned_tune).expect_err("tuning mismatched");
    assert_eq!(tune_mismatch.code, ReplayFailureCode::IdentityMismatch);
    assert_eq!(tune_mismatch.path.as_deref(), Some("identity.tuning"));

    // Constructing a tape with no `identity` at all is not tested here:
    // `InputTape::identity` is a non-optional `InputTapeIdentity` field in
    // Rust, so there is no way to build a tape without one. That case is a
    // compile-time impossibility here rather than a runtime check, the same
    // reasoning as `input_frame.rs`'s dropped `assert_fields` checks.
}

#[test]
fn input_tape_replay_names_the_first_causal_input_and_differing_state_path() {
    let tune = Tuning::new();
    let (reference, identity) = short_match_tape();
    let mut changed_frames = copy_frames(&reference);
    changed_frames[0].slots[0] = input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(-127),
        ..Default::default()
    })
    .expect("canonical sample");
    let candidate = input_tape::new(&identity, &reference.initial, &changed_frames, &tune)
        .expect("candidate tape constructs");
    let compared = replay::compare(&reference, &candidate, &identity, &tune)
        .expect("comparison context is valid");
    assert!(!compared.equal);
    let divergence = compared.divergence.expect("a divergence was found");
    assert_eq!(divergence.causal_input_tick, Some(0));
    assert_eq!(divergence.boundary_tick, 1);
    assert_ne!(divergence.expected_hash, divergence.actual_hash);
    assert!(divergence.state_path.starts_with("state.players."));
    assert_ne!(divergence.expected_input, divergence.actual_input);
}

#[test]
fn input_tape_replay_diagnoses_a_modified_initial_state_before_any_causal_input() {
    let tune = Tuning::new();
    let (reference, identity) = short_match_tape();
    let (restored_state, restored_combat) = match_snapshot::restore(&reference.initial);
    let mut changed = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    changed.state.ball_z = 1.0;
    let candidate = input_tape::new(&identity, &changed, &copy_frames(&reference), &tune)
        .expect("candidate tape constructs");
    let compared = replay::compare(&reference, &candidate, &identity, &tune)
        .expect("comparison context is valid");
    let divergence = compared.divergence.expect("a divergence was found");
    assert_eq!(divergence.causal_input_tick, None);
    assert_eq!(divergence.boundary_tick, 0);
    assert_eq!(divergence.state_path, "state.ball_z");
    assert_eq!(divergence.expected_state.as_deref(), Some("0.0"));
    assert_eq!(divergence.actual_state.as_deref(), Some("1.0"));
}

#[test]
fn input_tape_replay_diagnoses_a_changed_keeper_behavior_state_before_any_causal_input() {
    let tune = Tuning::new();
    let (reference, identity) = short_match_tape();
    let (restored_state, restored_combat) = match_snapshot::restore(&reference.initial);
    let mut changed = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    changed.state.players[0].keeper_state = KeeperBehaviorState::Advance;
    let candidate = input_tape::new(&identity, &changed, &copy_frames(&reference), &tune)
        .expect("candidate tape constructs");
    let compared = replay::compare(&reference, &candidate, &identity, &tune)
        .expect("comparison context is valid");
    let divergence = compared.divergence.expect("a divergence was found");
    assert_eq!(divergence.causal_input_tick, None);
    assert_eq!(divergence.boundary_tick, 0);
    // `match_snapshot`'s diff paths use a 1-based player index (this is a
    // diagnostic string, not a wire value — ARCHITECTURE.md §3 rule 3 does
    // not apply to it).
    assert_eq!(divergence.state_path, "state.players.1.keeper_state");
}

#[test]
fn input_tape_replay_detects_tampering_against_a_tapes_frozen_boundary_hashes() {
    let tune = Tuning::new();
    let (mut tape, identity) = short_match_tape();
    tape.frames[0].slots[0].move_x = -127;
    let result = replay::run(&tape, &identity, &tune).expect("replay context is valid");
    let divergence = result.divergence.expect("tampering is detected");
    assert_eq!(divergence.causal_input_tick, Some(0));
    assert_eq!(divergence.boundary_tick, 1);
    assert_eq!(divergence.state_path, "unavailable_without_reference_tape");
}

#[test]
fn input_tape_replay_reports_malformed_and_unsupported_tape_versions() {
    let tune = Tuning::new();
    let (mut legacy, current_identity) = short_match_tape();
    legacy.identity.input_version = 1;
    legacy.identity.ownership.version = 1;
    let incompatible =
        replay::run(&legacy, &current_identity, &tune).expect_err("legacy input version rejected");
    assert_eq!(incompatible.code, ReplayFailureCode::IdentityMismatch);
    assert_eq!(incompatible.path.as_deref(), Some("identity.input_version"));
    assert_eq!(
        incompatible.expected.as_deref(),
        Some(input_frame::VERSION.to_string().as_str())
    );
    assert_eq!(incompatible.actual.as_deref(), Some("1"));

    let (mut tape, identity) = short_match_tape();
    tape.version = 999;
    let malformed = replay::run(&tape, &identity, &tune).expect_err("unsupported tape version");
    assert_eq!(malformed.code, ReplayFailureCode::Malformed);
    assert!(malformed.message.contains("unsupported input tape version"));

    let (mut snapshot_tape, snapshot_identity) = short_match_tape();
    snapshot_tape.initial.version = match_snapshot::VERSION - 1;
    let prior_schema = replay::run(&snapshot_tape, &snapshot_identity, &tune)
        .expect_err("unsupported match snapshot version");
    assert_eq!(prior_schema.code, ReplayFailureCode::Malformed);
    assert!(
        prior_schema
            .message
            .contains("unsupported match snapshot version")
    );
}

#[test]
fn input_tape_replay_rejects_ownership_detached_from_the_initial_snapshot_and_post_match_frames() {
    let tune = Tuning::new();
    let (mut tape, _identity) = short_match_tape();
    tape.identity.ownership.slots[0].player_id = "detached-owner".to_string();
    let expected = input_tape::copy_identity(&tape.identity).expect("identity is valid");
    let detached = replay::run(&tape, &expected, &tune).expect_err("detached ownership rejected");
    assert_eq!(detached.code, ReplayFailureCode::Malformed);

    let (complete, complete_identity) = short_match_tape();
    let mut frames = copy_frames(&complete);
    frames.push(input_frame::neutral(3).expect("canonical frame"));
    assert!(
        input_tape::new(&complete_identity, &complete.initial, &frames, &tune).is_err(),
        "a materialized frame cannot follow the finished boundary"
    );

    let mut complete = complete;
    complete
        .frames
        .push(input_frame::neutral(3).expect("canonical frame"));
    let last = complete
        .boundary_hashes
        .last()
        .expect("at least one boundary")
        .clone();
    complete.boundary_hashes.push(last);
    let appended = replay::run(&complete, &complete_identity, &tune)
        .expect_err("a frame past the finished boundary is rejected");
    assert_eq!(appended.code, ReplayFailureCode::Malformed);
    assert!(appended.message.contains("frame after the match finished"));
}

#[test]
fn input_tape_replay_rejects_jointly_malformed_ownership_and_snapshot_routing() {
    let tune = Tuning::new();
    let (tape, identity) = short_match_tape();

    let mut malformed_identity = input_tape::copy_identity(&identity).expect("identity is valid");
    let (restored_state, restored_combat) = match_snapshot::restore(&tape.initial);
    let mut malformed_snapshot = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    malformed_identity.ownership.slots[0].team = input_frame::Team::Away;
    malformed_snapshot
        .state
        .input_ownership
        .as_mut()
        .expect("slot-mode snapshot has ownership")
        .slots[0]
        .team = input_frame::Team::Away;
    assert!(
        input_tape::new(
            &malformed_identity,
            &malformed_snapshot,
            &copy_frames(&tape),
            &tune
        )
        .is_err(),
        "matching malformed slot semantics must still be rejected"
    );

    let mut duplicate_identity = input_tape::copy_identity(&identity).expect("identity is valid");
    let (restored_state, restored_combat) = match_snapshot::restore(&tape.initial);
    let mut duplicate_snapshot = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    duplicate_identity.ownership.rosters.home[1] =
        duplicate_identity.ownership.rosters.home[0].clone();
    duplicate_snapshot
        .state
        .input_ownership
        .as_mut()
        .expect("slot-mode snapshot has ownership")
        .rosters
        .home[1] = duplicate_snapshot
        .state
        .input_ownership
        .as_ref()
        .expect("slot-mode snapshot has ownership")
        .rosters
        .home[0]
        .clone();
    assert!(
        input_tape::new(
            &duplicate_identity,
            &duplicate_snapshot,
            &copy_frames(&tape),
            &tune
        )
        .is_err(),
        "matching duplicate roster membership must still be rejected"
    );

    let (restored_state, restored_combat) = match_snapshot::restore(&tape.initial);
    let mut route_snapshot = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    route_snapshot.state.slot_players[0] = route_snapshot.state.slot_players[1];
    assert!(
        input_tape::new(&identity, &route_snapshot, &copy_frames(&tape), &tune).is_err(),
        "slot_players must agree with immutable ownership"
    );

    let (restored_state, restored_combat) = match_snapshot::restore(&tape.initial);
    let mut inverse_snapshot = match_snapshot::capture(&restored_state, restored_combat.as_ref());
    let first_player = inverse_snapshot.state.slot_players[0].expect("slot 1 is assigned");
    inverse_snapshot.state.slot_for_player[(first_player - 1) as usize] = Some(2);
    assert!(
        input_tape::new(&identity, &inverse_snapshot, &copy_frames(&tape), &tune).is_err(),
        "slot_for_player must agree with immutable ownership"
    );
}
