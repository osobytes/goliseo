//! Port of `spec/sim/slot_input_spec.lua`.

use gc_core::vec2::Vec2;
use gc_sim::input_frame::{
    self, EdgeAction, HeldAction, InputFrame, InputSample, InputSampleOptions,
};
use gc_sim::keeper::KeeperShotType;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team, WindupShot};
use gc_sim::slot_input::{self, MatchSlotSource, MatchSlotSourceKind};
use gc_sim::tuning::Tuning;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn frame_sources() -> [MatchSlotSource; 8] {
    [MatchSlotSource {
        kind: MatchSlotSourceKind::Frame,
        seed: None,
    }; 8]
}

/// Build an `InputFrame` for `tick`, overriding the given one-based slot
/// indices with `samples`' provided samples; every other slot is neutral.
/// Mirrors the Lua spec's `frame(tick, samples)` helper.
fn frame(tick: i64, samples: &[(i64, InputSample)]) -> InputFrame {
    let mut slots = [input_frame::neutral_sample(); 8];
    for &(index, sample) in samples {
        slots[(index - 1) as usize] = sample;
    }
    input_frame::new(tick, Some(slots)).expect("frame is canonical")
}

fn new_match() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(73.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    })
}

fn new_legacy_match() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(73.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

fn copy_slots(s: &MatchState) -> Vec<Option<i64>> {
    s.slot_players.clone()
}

/// `MatchPlayer.team` (`match_snapshot::Team`) and `InputSlot.team`
/// (`input_frame::Team`) are deliberately separate enums (one per module,
/// per README rule 6); this is the same-side comparison the Lua original
/// gets for free from a shared `"home"|"away"` string.
fn same_side(player_team: Team, slot_team: input_frame::Team) -> bool {
    matches!(
        (player_team, slot_team),
        (Team::Home, input_frame::Team::Home) | (Team::Away, input_frame::Team::Away)
    )
}

#[test]
fn fixed_match_input_slots_converts_a_neutral_sample_without_mistaking_valid_false_bits_for_an_error()
 {
    let input = slot_input::to_match_input(&input_frame::neutral_sample());
    assert_eq!(input.r#move.x, 0.0);
    assert_eq!(input.r#move.y, 0.0);
    assert!(!input.shoot);
    assert!(!input.pass);
    assert!(!input.sprint);
    assert_eq!(input.aerial_strike, Some(false));
    assert!(!input.equipment_held);
    assert!(!input.equipment_pressed);
    assert!(!input.equipment_released);
}

#[test]
fn fixed_match_input_slots_round_trips_canonical_equipment_held_and_edge_intent() {
    let mut pressed = slot_input::neutral_match_input();
    pressed.equipment_held = true;
    pressed.equipment_pressed = true;
    let pressed_sample = slot_input::to_sample(&pressed);
    let pressed_input = slot_input::to_match_input(&pressed_sample);
    assert!(pressed_input.equipment_held);
    assert!(pressed_input.equipment_pressed);
    assert!(!pressed_input.equipment_released);

    let mut tapped = slot_input::neutral_match_input();
    tapped.equipment_pressed = true;
    tapped.equipment_released = true;
    let tapped_input = slot_input::to_match_input(&slot_input::to_sample(&tapped));
    assert!(!tapped_input.equipment_held);
    assert!(tapped_input.equipment_pressed);
    assert!(tapped_input.equipment_released);
}

#[test]
fn fixed_match_input_slots_maps_exactly_four_permanent_outfield_slots_per_side_and_excludes_both_keepers()
 {
    let s = new_match();
    let mut seen = vec![false; s.players.len() + 1];
    for index in 1..=input_frame::SLOT_COUNT {
        let player_index = s.slot_players[(index - 1) as usize].expect("slot mapping is complete");
        let player = &s.players[(player_index - 1) as usize];
        let slot = input_frame::slot(index).expect("canonical slot index");
        assert!(same_side(player.team, slot.team));
        assert!(!player.is_keeper);
        assert_eq!(s.slot_for_player[(player_index - 1) as usize], Some(index));
        assert!(!seen[player_index as usize]);
        seen[player_index as usize] = true;
    }
    assert_eq!(
        s.input_ownership
            .as_ref()
            .expect("slot mode ownership")
            .slots
            .len(),
        input_frame::SLOT_COUNT as usize
    );
    assert_eq!(
        s.slot_for_player[0], None,
        "home keeper is never an input owner"
    );
    assert_eq!(
        s.slot_for_player[5], None,
        "away keeper is never an input owner"
    );
}

#[test]
fn fixed_match_input_slots_routes_simultaneous_opposing_rows_without_reading_controlled_or_possession()
 {
    let tune = Tuning::new();
    let mut s = new_match();
    let home_player = s.slot_players[0].expect("home slot 1 is filled");
    let away_player = s.slot_players[4].expect("away slot 1 is filled");
    s.players[(home_player - 1) as usize].pos.x = 180.0;
    s.players[(away_player - 1) as usize].pos.x = 780.0;
    s.players[(home_player - 1) as usize].run_vel.x = 0.0;
    s.players[(away_player - 1) as usize].run_vel.x = 0.0;
    s.controlled = 1; // Legacy metadata must not redirect either frame row.
    s.owner = s.slot_players[3];

    let right = input_frame::new_sample(InputSampleOptions {
        move_x: Some(127),
        ..Default::default()
    })
    .expect("valid sample");
    let left = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        ..Default::default()
    })
    .expect("valid sample");
    let f = frame(0, &[(1, right), (5, left)]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f), None, &tune);

    assert!(
        s.players[(home_player - 1) as usize].run_vel.x > 0.0,
        "home_1 consumes its own right input"
    );
    assert!(
        s.players[(away_player - 1) as usize].run_vel.x < 0.0,
        "away_1 consumes its own left input"
    );
}

#[test]
fn fixed_match_input_slots_keeps_ownership_stable_through_legacy_metadata_turnover_kickoff_and_aerial_state_changes()
 {
    let tune = Tuning::new();
    let mut s = new_match();
    let before = copy_slots(&s);
    s.controlled = 1;
    s.owner = s.slot_players[4];
    s.ball_z = 40.0;
    s.ball_vz = -20.0;
    let f0 = frame(0, &[]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f0), None, &tune);
    s.owner = s.slot_players[1];
    s.controlled = 6;
    let f1 = frame(1, &[]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f1), None, &tune);

    for index in 1..=input_frame::SLOT_COUNT {
        assert_eq!(
            s.slot_players[(index - 1) as usize],
            before[(index - 1) as usize],
            "slot mapping stays immutable"
        );
        let player_index = before[(index - 1) as usize].expect("slot mapping is complete");
        assert_eq!(
            s.slot_for_player[(player_index - 1) as usize],
            Some(index),
            "player routes back to its original slot"
        );
    }
}

#[test]
fn fixed_match_input_slots_does_not_hand_slot_mode_legacy_selection_to_a_pass_receiver() {
    let tune = Tuning::new();
    let mut s = new_match();
    let local_player = s.slot_players[3].expect("home slot 4 is filled");
    s.controlled = local_player;
    let pass = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Pass.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f = frame(0, &[(4, pass)]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f), None, &tune);

    assert_eq!(
        s.controlled, local_player,
        "a pass cannot move slot-mode legacy metadata"
    );
    assert_ne!(
        s.owner,
        Some(local_player),
        "the pass was released from the fixed local player"
    );
}

fn setup_smother(s: &mut MatchState) {
    s.controlled = 2;
    s.owner = Some(5);
    s.players[4].pos = Vec2::new(850.0, 270.0);
    s.players[4].facing = Vec2::new(1.0, 0.0);
    s.players[5].pos = Vec2::new(868.0, 270.0);
    s.ball = Vec2::new(868.0, 270.0);
    s.ball_vel = Vec2::new(0.0, 0.0);
}

fn setup_aerial(s: &mut MatchState) {
    s.controlled = 2;
    s.owner = None;
    s.pickup_cd = 1.0;
    s.players[4].pos = Vec2::new(700.0, 270.0);
    s.ball = Vec2::new(700.0, 270.0);
    s.ball_z = 50.0;
    s.ball_vz = 100.0;
    s.ball_vel = Vec2::new(0.0, 0.0);
}

#[test]
fn fixed_match_input_slots_suppresses_the_real_legacy_turnover_and_aerial_reselection_branches() {
    let tune = Tuning::new();
    let mut slot_turnover = new_match();
    let mut legacy_turnover = new_legacy_match();
    setup_smother(&mut slot_turnover);
    setup_smother(&mut legacy_turnover);
    let f0 = frame(0, &[]);
    sim_match::step(
        &mut slot_turnover,
        fixed_seconds(),
        StepInput::Frame(&f0),
        None,
        &tune,
    );
    sim_match::step(
        &mut legacy_turnover,
        fixed_seconds(),
        StepInput::Legacy(slot_input::neutral_match_input()),
        None,
        &tune,
    );

    assert_eq!(
        slot_turnover.owner,
        Some(6),
        "away keeper really takes the home carrier's ball"
    );
    assert_eq!(
        legacy_turnover.owner,
        Some(6),
        "the comparison reaches the same turnover"
    );
    assert_eq!(
        slot_turnover.controlled, 2,
        "slot mode suppresses turnover reselection"
    );
    assert_ne!(
        legacy_turnover.controlled, 2,
        "the same transition exercises legacy turnover reselection"
    );

    let mut slot_aerial = new_match();
    let mut legacy_aerial = new_legacy_match();
    setup_aerial(&mut slot_aerial);
    setup_aerial(&mut legacy_aerial);
    let f0b = frame(0, &[]);
    sim_match::step(
        &mut slot_aerial,
        fixed_seconds(),
        StepInput::Frame(&f0b),
        None,
        &tune,
    );
    sim_match::step(
        &mut legacy_aerial,
        fixed_seconds(),
        StepInput::Legacy(slot_input::neutral_match_input()),
        None,
        &tune,
    );

    assert!(slot_aerial.owner.is_none() && slot_aerial.ball_z > 30.0);
    assert!(legacy_aerial.owner.is_none() && legacy_aerial.ball_z > 30.0);
    assert_eq!(
        slot_aerial.controlled, 2,
        "slot mode suppresses aerial assistance"
    );
    assert_eq!(
        legacy_aerial.controlled, 5,
        "the same rising cross triggers legacy assistance"
    );
}

#[test]
fn fixed_match_input_slots_clears_a_heavy_touch_carrier_before_the_loss_tick_ends() {
    let tune = Tuning::new();
    let mut s = new_match();
    let carrier_idx = s.owner.expect("kickoff assigns an owner");
    {
        let carrier = &mut s.players[(carrier_idx - 1) as usize];
        carrier.pos = Vec2::new(300.0, 270.0);
        carrier.facing = Vec2::new(1.0, 0.0);
        carrier.vel = Vec2::new(0.0, 0.0);
        carrier.run_vel = Vec2::new(0.0, 0.0);
        carrier.dribble = 0.0;
        carrier.charge = 0.7;
        carrier.pass_charge = 0.8;
        carrier.pass_target = s.slot_players[0];
        carrier.windup_timer = fixed_seconds() * 2.0;
        carrier.windup_shot = Some(WindupShot {
            dir: carrier.facing,
            speed: 500.0,
            vz: 0.0,
            spin: 0.0,
            shot_type: KeeperShotType::Ground,
        });
    }
    s.ball = Vec2::new(325.0, 270.0);
    s.ball_vel = Vec2::new(1000.0, 0.0);

    let f0 = frame(0, &[]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f0), None, &tune);

    assert_eq!(
        s.owner, None,
        "the fast touch runs outside the carrier's control radius"
    );
    let carrier = &s.players[(carrier_idx - 1) as usize];
    assert_eq!(carrier.charge, 0.0);
    assert_eq!(carrier.pass_charge, 0.0);
    assert_eq!(carrier.pass_target, None);
    assert_eq!(carrier.windup_timer, 0.0);
    assert_eq!(
        carrier.windup_shot, None,
        "the loss tick clears the pending release"
    );
    let carrier_pos = carrier.pos;
    let carrier_facing = carrier.facing;

    s.owner = Some(carrier_idx);
    s.ball = carrier_pos.add(carrier_facing.scale(18.0));
    s.ball_vel = Vec2::new(0.0, 0.0);
    let f1 = frame(1, &[]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f1), None, &tune);
    assert_eq!(
        s.owner,
        Some(carrier_idx),
        "reacquisition cannot release the cancelled wind-up"
    );
    for event in &s.events {
        assert_ne!(
            event.kind,
            MatchEventKind::Shot,
            "no stale shot event can fire after reacquisition"
        );
    }
}

#[test]
fn fixed_match_input_slots_keeps_concurrent_holds_and_wind_up_cancellation_on_their_owning_players()
{
    let tune = Tuning::new();
    let mut s = new_match();
    let first = s.owner.expect("kickoff assigns an owner");
    let first_slot = s.slot_for_player[(first - 1) as usize].expect("owner has a slot");
    let second_slot: i64 = 5;
    let second = s.slot_players[(second_slot - 1) as usize].expect("away slot 1 is filled");

    let pass_hold = input_frame::new_sample(InputSampleOptions {
        held: Some(HeldAction::Pass.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f0 = frame(0, &[(first_slot, pass_hold)]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f0), None, &tune);
    assert!(
        s.players[(first - 1) as usize].pass_charge > 0.0,
        "the carrier owns its pass charge"
    );
    assert!(
        s.players[(first - 1) as usize].pass_target.is_some(),
        "the carrier owns its pass preview"
    );

    s.owner = Some(second);
    let second_pos = s.players[(second - 1) as usize].pos;
    let second_facing = s.players[(second - 1) as usize].facing;
    s.ball = second_pos.add(second_facing.scale(18.0));
    s.ball_vel = Vec2::new(0.0, 0.0);
    let shot_hold = input_frame::new_sample(InputSampleOptions {
        held: Some(HeldAction::Shoot.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f1 = frame(1, &[(first_slot, pass_hold), (second_slot, shot_hold)]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f1), None, &tune);

    assert_eq!(
        s.players[(first - 1) as usize].pass_charge,
        0.0,
        "possession loss cancels only the old owner"
    );
    assert_eq!(
        s.players[(first - 1) as usize].pass_target,
        None,
        "the old owner's preview is cleared"
    );
    assert!(
        s.players[(second - 1) as usize].charge > 0.0,
        "the new owner's simultaneous hold is independent"
    );

    let shot_release = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Shoot.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f2 = frame(2, &[(second_slot, shot_release)]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f2), None, &tune);
    assert!(
        s.players[(second - 1) as usize].windup_shot.is_some(),
        "release commits the owning player's shot"
    );
    assert!(s.players[(second - 1) as usize].windup_timer > 0.0);

    s.owner = Some(first);
    let first_pos = s.players[(first - 1) as usize].pos;
    let first_facing = s.players[(first - 1) as usize].facing;
    s.ball = first_pos.add(first_facing.scale(18.0));
    let f3 = frame(3, &[]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f3), None, &tune);
    assert_eq!(
        s.players[(second - 1) as usize].windup_shot,
        None,
        "possession loss cancels the pending payload"
    );
    assert_eq!(
        s.players[(second - 1) as usize].windup_timer,
        0.0,
        "cancellation cannot release on a later possession"
    );
}

#[test]
fn fixed_match_input_slots_resolves_simultaneous_pass_and_tackle_releases_without_overwriting_either_slot()
 {
    let tune = Tuning::new();
    let mut s = new_match();
    let passer = s.owner.expect("kickoff assigns an owner");
    let passer_slot = s.slot_for_player[(passer - 1) as usize].expect("owner has a slot");
    let defender_slot: i64 = 5;
    let defender = s.slot_players[(defender_slot - 1) as usize].expect("away slot 1 is filled");
    s.players[(defender - 1) as usize].pos.x = s.field.w - 80.0;
    s.players[(defender - 1) as usize].pos.y = 60.0;

    let pass_release = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Pass.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let tackle_release = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Dash.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f = frame(
        0,
        &[(passer_slot, pass_release), (defender_slot, tackle_release)],
    );
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f), None, &tune);

    assert_ne!(s.owner, Some(passer), "the passer's release is consumed");
    assert!(
        s.players[(defender - 1) as usize].tackle_timer > 0.0,
        "the defender's release is also consumed"
    );
    let passer_id = s.players[(passer - 1) as usize].id.clone();
    let saw_pass = s.events.iter().any(|event| {
        event.kind == MatchEventKind::Pass && event.player.as_deref() == Some(passer_id.as_str())
    });
    assert!(
        saw_pass,
        "the pass action remains attributed to its owning slot"
    );
}

#[test]
fn fixed_match_input_slots_resolves_a_direct_same_tick_tackle_before_the_carriers_pass_release() {
    let tune = Tuning::new();
    let mut s = new_match();
    let passer = s.owner.expect("kickoff assigns an owner");
    let passer_slot = s.slot_for_player[(passer - 1) as usize].expect("owner has a slot");
    let defender_slot: i64 = 5;
    let defender = s.slot_players[(defender_slot - 1) as usize].expect("away slot 1 is filled");
    let ball = s.ball;
    s.players[(defender - 1) as usize].pos = ball;

    let pass_release = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Pass.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let tackle_release = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Dash.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f = frame(
        0,
        &[(passer_slot, pass_release), (defender_slot, tackle_release)],
    );
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f), None, &tune);

    let saw_tackle = s.events.iter().any(|e| e.kind == MatchEventKind::Tackle);
    let saw_pass = s.events.iter().any(|e| e.kind == MatchEventKind::Pass);
    assert!(saw_tackle, "the in-range release wins the ball");
    assert!(
        !saw_pass,
        "canonical movement/tackle priority cancels the later pass"
    );
    assert_eq!(
        s.players[(passer - 1) as usize].pass_charge,
        0.0,
        "the dispossessed carrier ends the tick clean"
    );
}

fn setup_capture(s: &mut MatchState) {
    s.controlled = 2;
    s.owner = None;
    s.players[0].pos = Vec2::new(60.0, 270.0);
    s.ball = Vec2::new(65.0, 270.0);
    s.ball_vel = Vec2::new(0.0, 0.0);
    s.pickup_cd = 0.0;
}

#[test]
fn fixed_match_input_slots_keeps_slot_selection_fixed_through_switching_keeper_capture_and_kickoff()
{
    let tune = Tuning::new();
    let mut slot_capture = new_match();
    let mut legacy_capture = new_legacy_match();

    setup_capture(&mut slot_capture);
    setup_capture(&mut legacy_capture);
    let switch = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Switch.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f0 = frame(0, &[(1, switch)]);
    sim_match::step(
        &mut slot_capture,
        fixed_seconds(),
        StepInput::Frame(&f0),
        None,
        &tune,
    );
    sim_match::step(
        &mut legacy_capture,
        fixed_seconds(),
        StepInput::Legacy(slot_input::neutral_match_input()),
        None,
        &tune,
    );

    assert_eq!(
        slot_capture.owner,
        Some(1),
        "the loose ball is actually captured by the home keeper"
    );
    assert_eq!(
        legacy_capture.owner,
        Some(1),
        "the legacy comparison reaches the same keeper capture"
    );
    assert_eq!(
        slot_capture.controlled, 2,
        "switch and keeper capture cannot reselect in slot mode"
    );
    assert_eq!(
        legacy_capture.controlled, 1,
        "legacy mode still hands a new capture to the keeper"
    );
    assert_eq!(
        slot_capture.slot_for_player[0], None,
        "keeper capture never creates a keeper slot"
    );

    for player in &mut slot_capture.players {
        player.pos.y = 50.0;
    }
    slot_capture.owner = None;
    slot_capture.pickup_cd = 1.0;
    slot_capture.ball = Vec2::new(slot_capture.field.w - 7.0, slot_capture.field.h / 2.0);
    slot_capture.ball_vel = Vec2::new(1000.0, 0.0);
    slot_capture.ball_z = 0.0;
    slot_capture.ball_vz = 0.0;
    let f1 = frame(1, &[]);
    sim_match::step(
        &mut slot_capture,
        fixed_seconds(),
        StepInput::Frame(&f1),
        None,
        &tune,
    );

    assert_eq!(
        slot_capture.score.home, 1,
        "the forced goal reaches the kickoff path"
    );
    assert_eq!(
        slot_capture.controlled, 2,
        "kickoff does not rewrite slot-mode selection metadata"
    );
    assert_eq!(
        slot_capture.slot_for_player[0], None,
        "the restarted keeper remains AI-only"
    );
}

#[test]
fn fixed_match_input_slots_keeps_online_input_off_the_keeper_while_deterministic_keeper_ai_distributes()
 {
    let tune = Tuning::new();
    let mut s = new_match();
    let selected = s.slot_players[0].expect("home slot 1 is filled");
    s.controlled = selected;
    s.owner = Some(1);
    let field_h = s.field.h;
    {
        let keeper = &mut s.players[0];
        keeper.pos = Vec2::new(60.0, field_h / 2.0);
        keeper.facing = Vec2::new(1.0, 0.0);
        keeper.hold_timer = 0.0;
    }
    s.ball = s.players[0].pos;
    let attempted_keeper_pass = input_frame::new_sample(InputSampleOptions {
        edges: Some(EdgeAction::Pass.bit()),
        ..Default::default()
    })
    .expect("valid sample");
    let f0 = frame(0, &[(1, attempted_keeper_pass)]);
    sim_match::step(&mut s, fixed_seconds(), StepInput::Frame(&f0), None, &tune);

    assert_eq!(
        s.slot_for_player[0], None,
        "no frame row can route to the keeper"
    );
    assert_eq!(
        s.controlled, selected,
        "keeper possession cannot change slot selection"
    );
    assert_ne!(
        s.owner,
        Some(1),
        "the keeper AI releases the ball on its own schedule"
    );
    let keeper_id = s.players[0].id.clone();
    let distributed = s.events.iter().any(|event| {
        (event.kind == MatchEventKind::Pass || event.kind == MatchEventKind::Shot)
            && event.player.as_deref() == Some(keeper_id.as_str())
    });
    assert!(distributed, "the keeper AI owns distribution in slot mode");
}

#[test]
fn fixed_match_input_slots_rejects_non_finite_bot_seeds() {
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(f64::INFINITY),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "positive infinity cannot seed a bot"
    );

    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(f64::NEG_INFINITY),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "negative infinity cannot seed a bot"
    );

    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(f64::NAN),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "NaN cannot seed a bot"
    );
}

#[test]
fn fixed_match_input_slots_rejects_seeds_on_non_bot_slot_sources() {
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Frame,
        seed: Some(73.0),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "frame rows cannot carry bot seed identity"
    );

    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Neutral,
        seed: Some(73.0),
    };
    assert!(
        std::panic::catch_unwind(|| slot_input::new_producer(sources)).is_err(),
        "neutral rows cannot carry bot seed identity"
    );
}

#[test]
fn fixed_match_input_slots_canonicalizes_stored_bot_seeds_before_materializing() {
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(-901.0),
    };
    let producer = slot_input::new_producer(sources);
    assert_eq!(producer.sources[0].seed, Some(901.0));
}

#[test]
fn fixed_match_input_slots_materializes_only_explicitly_bot_configured_slots_from_independent_seeded_streams()
 {
    let tune = Tuning::new();
    let mut sources = frame_sources();
    sources[0] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(901.0),
    };
    sources[4] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(902.0),
    };
    let mut left = new_match();
    let mut right = new_match();
    let mut left_producer = slot_input::new_producer(sources);
    let mut right_producer = slot_input::new_producer(sources);
    for tick in 0..=30 {
        let base = frame(tick, &[]);
        let (left_frame, _) = slot_input::materialize(&mut left_producer, &left, &base, None);
        let (right_frame, _) = slot_input::materialize(&mut right_producer, &right, &base, None);
        sim_match::step(
            &mut left,
            fixed_seconds(),
            StepInput::Frame(&left_frame),
            None,
            &tune,
        );
        sim_match::step(
            &mut right,
            fixed_seconds(),
            StepInput::Frame(&right_frame),
            None,
            &tune,
        );
    }
    for index in 0..left.players.len() {
        assert!((left.players[index].pos.x - right.players[index].pos.x).abs() <= 1e-9);
        assert!((left.players[index].pos.y - right.players[index].pos.y).abs() <= 1e-9);
    }
    assert_eq!(left_producer.sources[0].seed, Some(901.0));
    assert_eq!(left_producer.sources[4].seed, Some(902.0));
    assert_eq!(left_producer.sources[1].kind, MatchSlotSourceKind::Frame);
}

#[test]
fn fixed_match_input_slots_materializes_frame_neutral_and_quantized_bot_rows_before_sim_match() {
    let s = new_match();
    let mut sources = frame_sources();
    sources[1] = MatchSlotSource {
        kind: MatchSlotSourceKind::Neutral,
        seed: None,
    };
    sources[2] = MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(901.0),
    };
    let mut producer = slot_input::new_producer(sources);
    let source_sample = input_frame::new_sample(InputSampleOptions {
        move_x: Some(42),
        edges: Some(1),
        ..Default::default()
    })
    .expect("valid sample");
    let f0 = frame(0, &[(1, source_sample)]);
    let (effective, _) = slot_input::materialize(&mut producer, &s, &f0, None);

    assert_eq!(effective.slots[0].move_x, 42, "frame rows are copied");
    assert_eq!(effective.slots[1].move_x, 0, "neutral rows are rewritten");
    assert!(effective.slots[2].move_x >= -127 && effective.slots[2].move_x <= 127);
    assert!(effective.slots[2].held >= 0 && effective.slots[2].edges >= 0);
    assert!(
        producer.bots.get(&3).is_some(),
        "only the producer owns bot RNG state"
    );
}

#[test]
fn fixed_match_input_slots_replays_effective_bot_filled_frames_with_an_all_frame_producer() {
    let tune = Tuning::new();
    let mut sources = frame_sources();
    for (index, source) in sources.iter_mut().enumerate() {
        *source = MatchSlotSource {
            kind: MatchSlotSourceKind::Bot,
            seed: Some(400.0 + (index as f64 + 1.0)),
        };
    }
    let mut live = new_match();
    let mut producer = slot_input::new_producer(sources);
    let mut recording: Vec<InputFrame> = Vec::new();
    for tick in 0..=120 {
        let base = frame(tick, &[]);
        let (effective, _) = slot_input::materialize(&mut producer, &live, &base, None);
        recording.push(effective);
        sim_match::step(
            &mut live,
            fixed_seconds(),
            StepInput::Frame(&effective),
            None,
            &tune,
        );
    }

    let mut replay = new_match();
    let mut all_frame = slot_input::new_producer(frame_sources());
    assert!(
        all_frame.bots.is_empty(),
        "the replay producer has no bot state"
    );
    for recorded in &recording {
        let (effective, _) = slot_input::materialize(&mut all_frame, &replay, recorded, None);
        sim_match::step(
            &mut replay,
            fixed_seconds(),
            StepInput::Frame(&effective),
            None,
            &tune,
        );
    }

    assert_eq!(replay.score.home, live.score.home);
    assert_eq!(replay.score.away, live.score.away);
    assert_eq!(replay.owner, live.owner);
    assert_eq!(replay.rng, live.rng);
    assert_eq!(replay.input_tick, live.input_tick);
    assert!((replay.ball.x - live.ball.x).abs() <= 1e-12);
    assert!((replay.ball.y - live.ball.y).abs() <= 1e-12);
    for index in 0..live.players.len() {
        assert!((replay.players[index].pos.x - live.players[index].pos.x).abs() <= 1e-12);
        assert!((replay.players[index].pos.y - live.players[index].pos.y).abs() <= 1e-12);
    }
}

#[test]
fn fixed_match_input_slots_requires_the_exact_fixed_tick_interval_in_slot_mode() {
    let tune = Tuning::new();
    let mut s = new_match();
    let f0 = frame(0, &[]);
    let ok = catch_unwind(AssertUnwindSafe(|| {
        sim_match::step(
            &mut s,
            fixed_seconds() * 2.0,
            StepInput::Frame(&f0),
            None,
            &tune,
        );
    }));
    assert!(ok.is_err());
}

#[test]
fn fixed_match_input_slots_rejects_a_legacy_matchinput_in_slot_mode() {
    let tune = Tuning::new();
    let mut s = new_match();
    let legacy_input = MatchInput::default();
    let ok = catch_unwind(AssertUnwindSafe(|| {
        sim_match::step(
            &mut s,
            fixed_seconds(),
            StepInput::Legacy(legacy_input),
            None,
            &tune,
        );
    }));
    assert!(ok.is_err());
}

fn fixed_seconds() -> f64 {
    gc_sim::fixed_clock::TICK_SECONDS
}
