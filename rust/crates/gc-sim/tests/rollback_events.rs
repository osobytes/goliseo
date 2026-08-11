//! Tests for `gc_sim::rollback_events`.
//!
//! Most cases that only manipulate `MatchSnapshot`s directly use the
//! hand-built `MatchState` fixture below (the same pattern
//! `tests/match_snapshot_differential.rs`'s `base_state` and
//! `tests/rollback_snapshot_history.rs` use — it is cheaper and does not
//! need a real match), but the five cases that drive a real
//! `rollback_session` (`run_real_match`, and the two combat-request cases)
//! build a real match via `gc_sim::r#match::new` and drive
//! `gc_sim::rollback_session` directly.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::combat;
use gc_sim::combat_snapshot::{
    CombatEventKind, CombatRequestOutcome, CombatRequestRejectionReason,
};
use gc_sim::input_frame::{self, InputSampleOptions};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{
    self, ByTeam, MatchEvent, MatchEventKind, MatchPlayer, MatchSnapshot, MatchState, PitchSize,
    Rect, Team,
};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::possession_transition::{self, TransitionWindows};
use gc_sim::rollback_events::{
    self, RollbackEventPayload, RollbackEventStepInput, RollbackEventTickOutput,
    RollbackEventsErrorCode, RollbackEventsStatus, RollbackOutputStateView,
};
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_session::{self, RollbackTickOutput};

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
        keeper_state: KeeperBehaviorState::Base,
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
        sprint_meter: 1.0,
        sprint_dur: 3.0,
        sprinting: false,
        save_pending: None,
        save_timer: 0.0,
        save_vx: 0.0,
        save_style: None,
        save_tip_emitted: false,
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
        windup_timer: 0.0,
        windup_shot: None,
        jockey_timer: 0.0,
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

/// Builds an already-valid `MatchState` fixture directly rather than going
/// through `sim_match::new` — cheaper, and the non-real-match cases below
/// don't need a fully initialized slot-mode match.
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
        score: ByTeam { home: 0, away: 0 },
        time_left: 240.0,
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
        rng: gc_core::rng::seed(720.0),
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

fn initial_snapshot() -> MatchSnapshot {
    match_snapshot::capture(&new_state(), None)
}

fn match_event(kind: MatchEventKind, x: f64, y: f64, player: Option<&str>) -> MatchEvent {
    MatchEvent {
        kind,
        x,
        y,
        player: player.map(str::to_string),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    }
}

#[derive(Default)]
struct NextSnapshotOptions {
    events: Vec<MatchEvent>,
    home_score: Option<i64>,
    away_score: Option<i64>,
    time_left: Option<f64>,
    finished: Option<bool>,
}

fn next_snapshot(before: &MatchSnapshot, options: NextSnapshotOptions) -> MatchSnapshot {
    let (mut state, _combat) = match_snapshot::restore(before);
    state.input_tick += 1;
    state.events = options.events;
    if let Some(home) = options.home_score {
        state.score.home = home;
    }
    if let Some(away) = options.away_score {
        state.score.away = away;
    }
    state.time_left = options
        .time_left
        .unwrap_or_else(|| (state.time_left - 1.0 / 60.0).max(0.0));
    if let Some(finished) = options.finished {
        state.finished = finished;
    }
    match_snapshot::capture(&state, None)
}

fn output_for(snapshot: &MatchSnapshot) -> RollbackEventTickOutput {
    let tick = snapshot.state.input_tick - 1;
    RollbackEventTickOutput {
        tick,
        start_boundary: tick,
        end_boundary: tick + 1,
        events: snapshot.state.events.clone(),
        combat_events: None,
        state: RollbackOutputStateView {
            score: snapshot.state.score,
            time_left: snapshot.state.time_left,
            finished: snapshot.state.finished,
        },
        finished: snapshot.state.finished,
    }
}

fn supplied(snapshot: &MatchSnapshot) -> RollbackEventStepInput {
    RollbackEventStepInput {
        output: output_for(snapshot),
        snapshot: snapshot.clone(),
    }
}

fn match_payload_x(event: &RollbackEventPayload) -> f64 {
    match event {
        RollbackEventPayload::Match(e) => e.x,
        _ => panic!("expected a match event payload"),
    }
}

// --- Real-match fixtures for the five cases that need a live rollback
// session (`sim::match` plus `sim::rollback_session`). Everything above
// this point stays on the hand-built `MatchState` fixture, which is
// cheaper for the cases that never need a real simulated tick.

/// Builds a real, slot-mode match via `sim::match.new`, defaulting to
/// `duration = 4, max_goals = 3, seed = 720`.
fn new_real_state(duration: f64, max_goals: i64, seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration),
        max_goals: Some(max_goals),
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    })
}

/// Every canonical slot owned remotely (a real session's local/remote
/// split does not matter to these cases; only its predicted-vs-authoritative
/// behaviour does).
fn sources_all_remote() -> [RollbackInputSource; 8] {
    [RollbackInputSource::Remote; 8]
}

/// Convert a live session's tick output into the shape `rollback_events`
/// expects, matching `rollback_lab.rs`'s private `event_tick_output` (this
/// test file owns no production code, so it carries its own copy).
fn event_tick_output(output: &RollbackTickOutput) -> RollbackEventTickOutput {
    RollbackEventTickOutput {
        tick: output.tick,
        start_boundary: output.start_boundary,
        end_boundary: output.end_boundary,
        events: output.events.clone(),
        combat_events: output.combat_events.clone(),
        state: RollbackOutputStateView {
            score: output.state.score,
            time_left: output.state.time_left,
            finished: output.state.finished,
        },
        finished: output.finished,
    }
}

/// A carrier planted just outside the away goal with a full shot charge, so
/// the fixture scores on its very first simulated tick.
fn shot_fixture(max_goals: i64) -> MatchSnapshot {
    let mut state = new_real_state(4.0, max_goals, 721.0);
    let carrier_index = state.slot_players[0].expect("slot 1 has a player");
    {
        let carrier = &mut state.players[(carrier_index - 1) as usize];
        carrier.pos = Vec2::new(900.0, 270.0);
        carrier.vel = Vec2::new(0.0, 0.0);
        carrier.run_vel = Vec2::new(0.0, 0.0);
        carrier.facing = Vec2::new(1.0, 0.0);
        carrier.charge = 1.0;
    }
    state.owner = Some(carrier_index);
    state.ball = Vec2::new(918.0, 270.0);
    state.ball_vel = Vec2::new(0.0, 0.0);
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 2.0;
    state.block_grace = 2.0;
    for (index, player) in state.players.iter_mut().enumerate() {
        let player_index = index as i64 + 1;
        if player_index != carrier_index {
            player.pos = Vec2::new(
                if player_index < 6 { 200.0 } else { 100.0 },
                40.0 + player_index as f64,
            );
            player.vel = Vec2::new(0.0, 0.0);
            player.run_vel = Vec2::new(0.0, 0.0);
        }
    }
    match_snapshot::capture(&state, None)
}

/// Drive a real session with one authoritative shoot press on slot one, then
/// keep stepping (predicting neutral input everywhere else) until `stop`
/// reports the target lifecycle transition.
fn run_real_match(
    initial: &MatchSnapshot,
    mut stop: impl FnMut(&MatchSnapshot) -> bool,
) -> (
    rollback_events::RollbackEventTimeline,
    Vec<rollback_events::RollbackEventDiff>,
) {
    let mut session = rollback_session::new(initial, sources_all_remote(), None, None);
    let mut timeline = rollback_events::new(initial, None);
    let shoot = input_frame::new_sample(InputSampleOptions {
        edges: Some(input_frame::EDGE_SHOOT),
        ..Default::default()
    })
    .expect("valid sample");
    rollback_session::add_authoritative(&mut session, 0, 1, shoot)
        .expect("authoritative sample accepted");
    let mut diffs = Vec::new();
    for _ in 0..40 {
        let output = rollback_session::step(&mut session).expect("session step succeeds");
        let lookup = rollback_session::snapshot(&session, output.end_boundary);
        let post = lookup
            .snapshot
            .expect("just-stepped boundary is always retained");
        let diff = rollback_events::apply(
            &mut timeline,
            output.tick,
            output.tick,
            &[RollbackEventStepInput {
                output: event_tick_output(&output),
                snapshot: post.clone(),
            }],
        )
        .expect("apply succeeds");
        diffs.push(diff);
        if stop(&post) {
            return (timeline, diffs);
        }
    }
    panic!("real goal fixture did not reach its lifecycle transition");
}

/// Every lifecycle-domain wrapped event added across a run, in order.
fn lifecycle_additions(
    diffs: &[rollback_events::RollbackEventDiff],
) -> Vec<rollback_events::RollbackWrappedEvent> {
    diffs
        .iter()
        .flat_map(|diff| diff.added.iter())
        .filter(|event| event.domain.starts_with("lifecycle/"))
        .cloned()
        .collect()
}

fn lifecycle_payload(
    event: &rollback_events::RollbackWrappedEvent,
) -> &rollback_events::RollbackLifecyclePayload {
    match &event.payload {
        RollbackEventPayload::Lifecycle(payload) => payload,
        _ => panic!("expected a lifecycle event payload"),
    }
}

fn combat_payload(
    event: &rollback_events::RollbackWrappedEvent,
) -> &gc_sim::combat_snapshot::CombatEvent {
    match &event.payload {
        RollbackEventPayload::Combat(payload) => payload,
        _ => panic!("expected a combat event payload"),
    }
}

#[test]
fn rollback_events_keeps_per_domain_ordinals_stable_and_identical_reapplication_silent() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let post = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![
                match_event(MatchEventKind::Shot, 10.0, 20.0, Some("a")),
                match_event(MatchEventKind::Tackle, 30.0, 40.0, Some("b")),
                match_event(MatchEventKind::Shot, 50.0, 60.0, Some("c")),
            ],
            ..Default::default()
        },
    );
    let first = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&post)]).unwrap();
    assert_eq!(first.added.len(), 3);
    assert_eq!(first.added[0].ordinal, 1);
    assert_eq!(first.added[1].ordinal, 1);
    assert_eq!(first.added[2].ordinal, 2);
    assert_eq!(first.added[0].id, "0000000000|010:match/shot|0001");
    assert_eq!(first.added[2].id, "0000000000|010:match/shot|0002");

    let again = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&post)]).unwrap();
    assert_eq!(again.added.len(), 0);
    assert_eq!(again.revoked.len(), 0);
    assert_eq!(again.replaced.len(), 0);
}

#[test]
fn rollback_events_reports_changed_shot_payload_as_replacement_and_shot_to_pass_as_revoke_plus_add()
{
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let shot = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![MatchEvent {
                shot_type: Some(KeeperShotType::Ground),
                ..match_event(MatchEventKind::Shot, 10.0, 20.0, Some("a"))
            }],
            ..Default::default()
        },
    );
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&shot)]).unwrap();
    let changed = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![MatchEvent {
                shot_type: Some(KeeperShotType::Chip),
                ..match_event(MatchEventKind::Shot, 12.0, 22.0, Some("a"))
            }],
            ..Default::default()
        },
    );
    let replaced = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&changed)]).unwrap();
    assert_eq!(replaced.replaced.len(), 1);
    assert_eq!(replaced.added.len(), 0);
    assert_eq!(replaced.revoked.len(), 0);
    assert_eq!(
        replaced.replaced[0].before.id,
        replaced.replaced[0].after.id
    );
    assert_eq!(match_payload_x(&replaced.replaced[0].before.payload), 10.0);
    let after_shot_type = match &replaced.replaced[0].after.payload {
        RollbackEventPayload::Match(event) => event.shot_type,
        _ => panic!("expected a match event payload"),
    };
    assert_eq!(after_shot_type, Some(KeeperShotType::Chip));

    let pass = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Pass, 12.0, 22.0, Some("a"))],
            ..Default::default()
        },
    );
    let changed_kind = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&pass)]).unwrap();
    assert_eq!(changed_kind.replaced.len(), 0);
    assert_eq!(changed_kind.revoked[0].domain, "match/shot");
    assert_eq!(changed_kind.added[0].domain, "match/pass");
}

#[test]
fn rollback_events_uses_canonical_signed_zero_equality_for_payload_replacement() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let positive = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Shot, 0.0, 1.0, Some("a"))],
            ..Default::default()
        },
    );
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&positive)]).unwrap();
    let negative = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Shot, -0.0, 1.0, Some("a"))],
            ..Default::default()
        },
    );
    let diff = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&negative)]).unwrap();
    assert_eq!(diff.replaced.len(), 1);
    assert_eq!(
        1.0 / match_payload_x(&diff.replaced[0].before.payload),
        f64::INFINITY
    );
    assert_eq!(
        1.0 / match_payload_x(&diff.replaced[0].after.payload),
        f64::NEG_INFINITY
    );
}

#[test]
fn rollback_events_revokes_a_predicted_goal_and_kickoff_once_and_never_confirms_them() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let goal = next_snapshot(
        &initial,
        NextSnapshotOptions {
            home_score: Some(1),
            ..Default::default()
        },
    );
    let predicted = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&goal)]).unwrap();
    assert_eq!(predicted.added[0].domain, "lifecycle/goal");
    assert_eq!(predicted.added[1].domain, "lifecycle/kickoff");

    let corrected = next_snapshot(&initial, NextSnapshotOptions::default());
    let revoked = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&corrected)]).unwrap();
    assert_eq!(revoked.revoked.len(), 2);
    assert_eq!(revoked.revoked[0].domain, "lifecycle/goal");
    assert_eq!(revoked.revoked[1].domain, "lifecycle/kickoff");
    let repeated = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&corrected)]).unwrap();
    assert_eq!(repeated.revoked.len(), 0);
    let confirmed = rollback_events::confirm(&mut timeline, 0);
    assert_eq!(confirmed[0].lifecycle_events.len(), 0);
}

#[test]
fn rollback_events_removes_a_tackle_repeatedly_then_confirms_a_different_event_at_the_same_tick() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let tackle = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(
                MatchEventKind::Tackle,
                1.0,
                2.0,
                Some("defender"),
            )],
            ..Default::default()
        },
    );
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&tackle)]).unwrap();
    let empty = next_snapshot(&initial, NextSnapshotOptions::default());
    let removed = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&empty)]).unwrap();
    assert_eq!(removed.revoked.len(), 1);
    let repeated = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&empty)]).unwrap();
    assert_eq!(repeated.revoked.len(), 0);

    let pass = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(
                MatchEventKind::Pass,
                3.0,
                4.0,
                Some("attacker"),
            )],
            ..Default::default()
        },
    );
    let added = rollback_events::apply(&mut timeline, 0, 0, &[supplied(&pass)]).unwrap();
    assert_eq!(added.added[0].domain, "match/pass");
    let confirmed = rollback_events::confirm(&mut timeline, 0);
    assert_eq!(confirmed[0].match_events.len(), 1);
    assert_eq!(confirmed[0].match_events[0].domain, "match/pass");
}

#[test]
fn rollback_events_confirms_catch_parry_tip_and_claim_vocabulary_exactly_once_with_stable_ids() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let post = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![
                MatchEvent {
                    save_style: Some(SaveStyle::Central),
                    ..match_event(MatchEventKind::Catch, 1.0, 2.0, Some("keeper"))
                },
                MatchEvent {
                    save_style: Some(SaveStyle::Stretch),
                    ..match_event(MatchEventKind::Parry, 3.0, 4.0, Some("keeper"))
                },
                match_event(MatchEventKind::Tip, 5.0, 6.0, Some("keeper")),
                match_event(MatchEventKind::Claim, 7.0, 8.0, Some("keeper")),
            ],
            ..Default::default()
        },
    );
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&post)]).unwrap();
    let confirmed = rollback_events::confirm(&mut timeline, 0);
    assert_eq!(confirmed[0].match_events.len(), 4);
    let expected_domains = ["match/catch", "match/parry", "match/tip", "match/claim"];
    for (index, domain) in expected_domains.iter().enumerate() {
        let event = &confirmed[0].match_events[index];
        assert_eq!(&event.domain, domain);
        assert_eq!(event.ordinal, 1);
    }
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 0);
}

#[test]
fn rollback_events_lets_earlier_corrected_full_time_remove_every_stale_later_tick_event() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let zero = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Shot, 1.0, 1.0, Some("a"))],
            ..Default::default()
        },
    );
    let one = next_snapshot(
        &zero,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Tackle, 2.0, 2.0, Some("b"))],
            ..Default::default()
        },
    );
    let two = next_snapshot(
        &one,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Pass, 3.0, 3.0, Some("c"))],
            ..Default::default()
        },
    );
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&zero)]).unwrap();
    rollback_events::apply(&mut timeline, 1, 1, &[supplied(&one)]).unwrap();
    rollback_events::apply(&mut timeline, 2, 2, &[supplied(&two)]).unwrap();

    let earlier_finish = next_snapshot(
        &initial,
        NextSnapshotOptions {
            finished: Some(true),
            time_left: Some(0.0),
            ..Default::default()
        },
    );
    let corrected =
        rollback_events::apply(&mut timeline, 0, 2, &[supplied(&earlier_finish)]).unwrap();
    assert_eq!(corrected.revoked.len(), 3);
    assert_eq!(corrected.added[0].domain, "lifecycle/full_time");
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 1);
}

#[test]
fn rollback_events_rejects_empty_or_active_short_corrections_before_mutating_the_stale_tail() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let zero = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Shot, 1.0, 1.0, Some("a"))],
            ..Default::default()
        },
    );
    let one = next_snapshot(
        &zero,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Tackle, 2.0, 2.0, Some("b"))],
            ..Default::default()
        },
    );
    let two = next_snapshot(
        &one,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Pass, 3.0, 3.0, Some("c"))],
            ..Default::default()
        },
    );
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&zero)]).unwrap();
    rollback_events::apply(&mut timeline, 1, 1, &[supplied(&one)]).unwrap();
    rollback_events::apply(&mut timeline, 2, 2, &[supplied(&two)]).unwrap();

    let active_short = next_snapshot(&initial, NextSnapshotOptions::default());
    let short_supplied = supplied(&active_short);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rollback_events::apply(&mut timeline, 0, 2, &[short_supplied])
    }));
    assert!(result.is_err());
    let empty_steps: Vec<RollbackEventStepInput> = Vec::new();
    let result2 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rollback_events::apply(&mut timeline, 0, 2, &empty_steps)
    }));
    assert!(result2.is_err());
    let diagnostics = rollback_events::diagnostics(&timeline);
    assert_eq!(diagnostics.status, RollbackEventsStatus::Active);
    assert_eq!(diagnostics.confirmed_tick, -1);
    assert_eq!(diagnostics.retained_step_count, 3);
    assert_eq!(diagnostics.retained_event_count, 3);
    let confirmed = rollback_events::confirm(&mut timeline, 2);
    assert_eq!(confirmed[0].match_events[0].domain, "match/shot");
    assert_eq!(confirmed[1].match_events[0].domain, "match/tackle");
    assert_eq!(confirmed[2].match_events[0].domain, "match/pass");
}

#[test]
fn rollback_events_bounds_stalled_confirmation_to_thirty_compact_step_records_and_fails_explicitly()
{
    let mut state = new_state();
    // `match.new`'s slot-mode ownership indirection (`state.slot_players[1]`)
    // is unavailable; the fact under test is that an owner's identity
    // survives from the timeline's initial roster, so this sets the owner
    // directly to a valid outfielder instead.
    state.owner = Some(2);
    let initial = match_snapshot::capture(&state, None);
    let mut timeline = rollback_events::new(&initial, None);
    let mut post = initial.clone();
    for tick in 0..=29i64 {
        post = next_snapshot(
            &post,
            NextSnapshotOptions {
                events: vec![match_event(
                    MatchEventKind::Touch,
                    tick as f64,
                    tick as f64,
                    Some("a"),
                )],
                ..Default::default()
            },
        );
        rollback_events::apply(&mut timeline, tick, tick, &[supplied(&post)]).unwrap();
    }

    let healthy = rollback_events::diagnostics(&timeline);
    assert_eq!(healthy.status, RollbackEventsStatus::Active);
    assert_eq!(healthy.max_unconfirmed_ticks, 30);
    assert_eq!(healthy.retained_step_count, 30);
    assert_eq!(healthy.retained_event_count, 30);
    assert_eq!(healthy.oldest_tick, Some(0));
    assert_eq!(healthy.latest_tick, Some(29));
    let retained = timeline.steps.get(&0).unwrap();
    let owner_index = initial.state.owner.unwrap() as usize - 1;
    assert_eq!(
        retained.state.owner_id,
        Some(initial.state.players[owner_index].id.clone())
    );
    assert_eq!(
        retained.state.owner_team,
        Some(initial.state.players[owner_index].team)
    );

    let over = next_snapshot(&post, NextSnapshotOptions::default());
    let result = rollback_events::apply(&mut timeline, 30, 30, &[supplied(&over)]);
    let err = result.unwrap_err();
    assert_eq!(err.code, RollbackEventsErrorCode::UnconfirmedWindowExceeded);
    let terminal = rollback_events::diagnostics(&timeline);
    assert_eq!(
        terminal.status,
        RollbackEventsStatus::UnconfirmedWindowExceeded
    );
    assert_eq!(terminal.retained_step_count, 30);
    assert_eq!(terminal.oldest_tick, Some(0));
    assert_eq!(terminal.latest_tick, Some(29));
}

#[test]
fn rollback_events_confirms_monotonically_across_calls_and_rejects_gaps_or_confirmed_correction() {
    let initial = initial_snapshot();
    let mut timeline = rollback_events::new(&initial, None);
    let zero = next_snapshot(&initial, NextSnapshotOptions::default());
    let one = next_snapshot(&zero, NextSnapshotOptions::default());
    let two = next_snapshot(&one, NextSnapshotOptions::default());
    rollback_events::apply(&mut timeline, 0, 0, &[supplied(&zero)]).unwrap();
    rollback_events::apply(&mut timeline, 1, 1, &[supplied(&one)]).unwrap();
    rollback_events::apply(&mut timeline, 2, 2, &[supplied(&two)]).unwrap();
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 1);
    assert_eq!(rollback_events::confirm(&mut timeline, 2).len(), 2);
    assert_eq!(rollback_events::confirm(&mut timeline, 2).len(), 0);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rollback_events::confirm(
            &mut timeline,
            1
        )))
        .is_err()
    );
    let zero_supplied = supplied(&zero);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rollback_events::apply(
            &mut timeline,
            0,
            0,
            &[zero_supplied]
        )))
        .is_err()
    );

    let mut missing = rollback_events::new(&initial, None);
    rollback_events::apply(&mut missing, 0, 0, &[supplied(&zero)]).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rollback_events::confirm(
            &mut missing,
            1
        )))
        .is_err()
    );
    let after_failed_confirm = rollback_events::diagnostics(&missing);
    assert_eq!(after_failed_confirm.confirmed_tick, -1);
    assert_eq!(after_failed_confirm.confirmed_boundary, 0);
    assert_eq!(after_failed_confirm.retained_step_count, 1);
    assert_eq!(rollback_events::confirm(&mut missing, 0).len(), 1);

    let mut noncontiguous = rollback_events::new(&initial, None);
    let bad = supplied(&one);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rollback_events::apply(
            &mut noncontiguous,
            0,
            0,
            &[bad]
        )))
        .is_err()
    );
}

#[test]
fn rollback_events_defensively_copies_inputs_diffs_confirmed_records_and_compact_state_views() {
    let initial = initial_snapshot();
    let initial_hash = match_snapshot::hash(&initial);
    let mut timeline = rollback_events::new(&initial, None);
    let post = next_snapshot(
        &initial,
        NextSnapshotOptions {
            events: vec![match_event(MatchEventKind::Shot, 10.0, 20.0, Some("a"))],
            ..Default::default()
        },
    );
    let mut input = supplied(&post);
    let mut diff = rollback_events::apply(&mut timeline, 0, 0, &[input.clone()]).unwrap();
    // Rust ownership already guarantees `apply` could not have retained
    // these — see the module doc comment — so mutating the caller's own
    // copies here documents that guarantee rather than probing for a bug
    // the type system rules out.
    input.output.events[0].x = 999.0;
    input.snapshot.state.events[0].x = 999.0;
    match &mut diff.added[0].payload {
        RollbackEventPayload::Match(event) => event.x = 888.0,
        _ => unreachable!(),
    }

    let mut confirmed = rollback_events::confirm(&mut timeline, 0);
    assert_eq!(match_payload_x(&confirmed[0].match_events[0].payload), 10.0);
    assert_eq!(confirmed[0].state.score.home, 0);
    match &mut confirmed[0].match_events[0].payload {
        RollbackEventPayload::Match(event) => event.x = 777.0,
        _ => unreachable!(),
    }
    confirmed[0].state.score.home = 99;

    let next = next_snapshot(&post, NextSnapshotOptions::default());
    let next_diff = rollback_events::apply(&mut timeline, 1, 1, &[supplied(&next)]).unwrap();
    assert_eq!(next_diff.added.len(), 0);
    assert_eq!(match_snapshot::hash(&initial), initial_hash);
    assert_eq!(match_snapshot::VERSION, 11);
}

#[test]
fn rollback_events_revokes_corrected_away_combat_events_and_confirms_the_replacement_once() {
    let mut state = new_real_state(4.0, 3, 720.0);
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    let mut source_player: Option<i64> = None;
    for (index, runtime) in combat_state.players.iter().enumerate() {
        if runtime.family_id == Some(ActionFamilyId::LightMelee) {
            source_player = Some(index as i64 + 1);
            break;
        }
    }
    let source_player = source_player.expect("fixture requires one melee player");
    let source_slot = state.slot_for_player[(source_player - 1) as usize]
        .expect("the melee player owns a canonical slot");
    let initial = match_snapshot::capture(&state, Some(&combat_state));

    let run = |attack: bool| -> (RollbackTickOutput, MatchSnapshot) {
        let mut session = rollback_session::new(&initial, sources_all_remote(), None, None);
        for slot in 1..=input_frame::SLOT_COUNT {
            let row = if attack && slot == source_slot {
                input_frame::new_sample(InputSampleOptions {
                    held: Some(input_frame::HELD_EQUIPMENT),
                    edges: Some(input_frame::EDGE_EQUIPMENT_PRESSED),
                    ..Default::default()
                })
                .expect("valid sample")
            } else {
                input_frame::neutral_sample()
            };
            rollback_session::add_authoritative(&mut session, 0, slot, row)
                .expect("authoritative sample accepted");
        }
        let output = rollback_session::step(&mut session).expect("session step succeeds");
        let snapshot = rollback_session::current_snapshot(&session);
        (output, snapshot)
    };

    let (attack_output, attack_snapshot) = run(true);
    let (neutral_output, neutral_snapshot) = run(false);
    let mut timeline = rollback_events::new(&initial, None);
    let added = rollback_events::apply(
        &mut timeline,
        0,
        0,
        &[RollbackEventStepInput {
            output: event_tick_output(&attack_output),
            snapshot: attack_snapshot,
        }],
    )
    .expect("apply succeeds");
    assert_eq!(added.added.len(), 1);
    assert_eq!(added.added[0].domain, "combat/commit/1");

    let corrected = rollback_events::apply(
        &mut timeline,
        0,
        0,
        &[RollbackEventStepInput {
            output: event_tick_output(&neutral_output),
            snapshot: neutral_snapshot,
        }],
    )
    .expect("apply succeeds");
    assert_eq!(corrected.revoked.len(), 1);
    assert_eq!(corrected.revoked[0].id, added.added[0].id);
    assert_eq!(corrected.added.len(), 0);
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 1);
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 0);
}

#[test]
fn rollback_events_carries_a_rejected_request_which_has_no_sequence_through_the_timeline() {
    // Kickoff hold refuses every press, so this is the shortest fixture that
    // puts a sequence-less combat row on the wire.
    let mut state = new_real_state(4.0, 3, 720.0);
    assert!(
        state.kickoff_hold > 0.0,
        "the fixture relies on a live kickoff hold"
    );
    let combat_state = combat::new_state(&mut state, None);
    let mut equipped = Vec::new();
    for (index, runtime) in combat_state.players.iter().enumerate() {
        let player_index = index as i64 + 1;
        if runtime.family_id.is_some() && state.slot_for_player[index].is_some() {
            equipped.push(player_index);
        }
    }
    assert!(
        equipped.len() >= 2,
        "fixture requires two equipped outfielders"
    );
    let first = equipped[0];
    let second = equipped[1];
    let initial = match_snapshot::capture(&state, Some(&combat_state));

    let run = |pressing: &[i64]| -> (RollbackTickOutput, MatchSnapshot) {
        let mut session = rollback_session::new(&initial, sources_all_remote(), None, None);
        for slot in 1..=input_frame::SLOT_COUNT {
            let mut row = input_frame::neutral_sample();
            for &player_index in pressing {
                if Some(slot) == state.slot_for_player[(player_index - 1) as usize] {
                    row = input_frame::new_sample(InputSampleOptions {
                        held: Some(input_frame::HELD_EQUIPMENT),
                        edges: Some(input_frame::EDGE_EQUIPMENT_PRESSED),
                        ..Default::default()
                    })
                    .expect("valid sample");
                }
            }
            rollback_session::add_authoritative(&mut session, 0, slot, row)
                .expect("authoritative sample accepted");
        }
        let output = rollback_session::step(&mut session).expect("session step succeeds");
        let snapshot = rollback_session::current_snapshot(&session);
        (output, snapshot)
    };

    let (both_output, both_snapshot) = run(&[first, second]);
    let mut timeline = rollback_events::new(&initial, None);
    let added = rollback_events::apply(
        &mut timeline,
        0,
        0,
        &[RollbackEventStepInput {
            output: event_tick_output(&both_output),
            snapshot: both_snapshot.clone(),
        }],
    )
    .expect("apply succeeds");
    assert_eq!(added.added.len(), 2);

    let mut by_player: Vec<(i64, String)> = Vec::new();
    for wrapped in &added.added {
        assert_eq!(wrapped.domain, "combat/request_rejected/0");
        let payload = combat_payload(wrapped);
        assert_eq!(payload.kind, CombatEventKind::RequestRejected);
        assert_eq!(payload.outcome, Some(CombatRequestOutcome::Rejected));
        assert_eq!(
            payload.reason,
            Some(CombatRequestRejectionReason::KickoffHold)
        );
        assert!(
            payload.source_sequence.is_none(),
            "a rejection opens no encounter"
        );
        let source_index = payload
            .source_index
            .expect("rejection carries its requester");
        // The requesting player is the identity, so the ordinal is its index
        // rather than a position in this tick's list.
        assert_eq!(wrapped.ordinal, source_index);
        by_player.push((source_index, wrapped.id.clone()));
    }
    assert!(by_player.iter().any(|(player, _)| *player == first));
    assert!(by_player.iter().any(|(player, _)| *player == second));

    // Confirmation copies the step, so the sequence-less rows have to survive
    // the confirmed-copy path too.
    let confirmed = rollback_events::confirm(&mut timeline, 0);
    assert_eq!(confirmed.len(), 1);
    let confirmed_combat = confirmed[0]
        .combat_events
        .as_ref()
        .expect("combat events are present");
    assert_eq!(confirmed_combat.len(), 2);
    let confirmed_first = combat_payload(&confirmed_combat[0]);
    assert_eq!(
        confirmed_first.reason,
        Some(CombatRequestRejectionReason::KickoffHold)
    );
    assert!(confirmed_first.source_sequence.is_none());
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 0);

    // The regression this pins: a resimulation in which only the second
    // player presses must revoke the first player's row and leave the
    // second's identity untouched. A per-tick running ordinal would instead
    // rewrite the first player's cue into the second player's.
    let (single_output, single_snapshot) = run(&[second]);
    let mut corrected = rollback_events::new(&initial, None);
    rollback_events::apply(
        &mut corrected,
        0,
        0,
        &[RollbackEventStepInput {
            output: event_tick_output(&both_output),
            snapshot: both_snapshot,
        }],
    )
    .expect("apply succeeds");
    let diff = rollback_events::apply(
        &mut corrected,
        0,
        0,
        &[RollbackEventStepInput {
            output: event_tick_output(&single_output),
            snapshot: single_snapshot,
        }],
    )
    .expect("apply succeeds");
    assert_eq!(diff.added.len(), 0, "the surviving row is not a new event");
    assert_eq!(
        diff.replaced.len(),
        0,
        "the surviving row is not a corrected payload"
    );
    assert_eq!(diff.revoked.len(), 1);
    let first_id = by_player
        .iter()
        .find(|(player, _)| *player == first)
        .map(|(_, id)| id.clone())
        .expect("first player's id was recorded");
    assert_eq!(diff.revoked[0].id, first_id);
}

#[test]
fn rollback_events_derives_real_goal_plus_kickoff_once_from_match_snapshots() {
    let initial = shot_fixture(3);
    let (mut timeline, diffs) = run_real_match(&initial, |snapshot| snapshot.state.score.home == 1);
    let lifecycle = lifecycle_additions(&diffs);
    assert_eq!(lifecycle.len(), 2);
    assert_eq!(lifecycle[0].domain, "lifecycle/goal");
    assert_eq!(lifecycle_payload(&lifecycle[0]).team, Some(Team::Home));
    assert_eq!(lifecycle_payload(&lifecycle[0]).score.home, 1);
    assert_eq!(lifecycle[1].domain, "lifecycle/kickoff");
    assert_eq!(lifecycle_payload(&lifecycle[1]).team, Some(Team::Away));

    let confirmed = rollback_events::confirm(&mut timeline, lifecycle[0].tick);
    let confirmed_lifecycle = &confirmed
        .last()
        .expect("at least one confirmed step")
        .lifecycle_events;
    assert_eq!(confirmed_lifecycle[0].domain, "lifecycle/goal");
    assert_eq!(
        lifecycle_payload(&confirmed_lifecycle[0]).team,
        Some(Team::Home)
    );
    assert_eq!(confirmed_lifecycle[1].domain, "lifecycle/kickoff");
    assert_eq!(
        lifecycle_payload(&confirmed_lifecycle[1]).team,
        Some(Team::Away)
    );
    assert_eq!(
        rollback_events::confirm(&mut timeline, lifecycle[0].tick).len(),
        0
    );
}

#[test]
fn rollback_events_derives_real_max_goal_full_time_without_kickoff() {
    let initial = shot_fixture(1);
    let (mut timeline, diffs) = run_real_match(&initial, |snapshot| snapshot.state.finished);
    let lifecycle = lifecycle_additions(&diffs);
    assert_eq!(lifecycle.len(), 2);
    assert_eq!(lifecycle[0].domain, "lifecycle/goal");
    assert_eq!(lifecycle[1].domain, "lifecycle/full_time");

    let confirmed = rollback_events::confirm(&mut timeline, lifecycle[0].tick);
    let confirmed_lifecycle = &confirmed
        .last()
        .expect("at least one confirmed step")
        .lifecycle_events;
    assert_eq!(confirmed_lifecycle[0].domain, "lifecycle/goal");
    assert_eq!(lifecycle_payload(&confirmed_lifecycle[0]).score.home, 1);
    assert_eq!(confirmed_lifecycle[1].domain, "lifecycle/full_time");
    assert_eq!(lifecycle_payload(&confirmed_lifecycle[1]).score.home, 1);
    assert_eq!(
        rollback_events::confirm(&mut timeline, lifecycle[0].tick).len(),
        0
    );
}

#[test]
fn rollback_events_derives_timer_full_time_alone_and_no_opening_kickoff() {
    let state = new_real_state(1.0 / 60.0, 3, 720.0);
    let initial = match_snapshot::capture(&state, None);
    let mut session = rollback_session::new(&initial, sources_all_remote(), None, None);
    let output = rollback_session::step(&mut session).expect("session step succeeds");
    let lookup = rollback_session::snapshot(&session, 1);
    let post = lookup.snapshot.expect("boundary one is retained");
    let mut timeline = rollback_events::new(&initial, None);
    let diff = rollback_events::apply(
        &mut timeline,
        0,
        0,
        &[RollbackEventStepInput {
            output: event_tick_output(&output),
            snapshot: post,
        }],
    )
    .expect("apply succeeds");
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.added[0].domain, "lifecycle/full_time");
    let confirmed = rollback_events::confirm(&mut timeline, 0);
    assert_eq!(
        confirmed[0].lifecycle_events[0].domain,
        "lifecycle/full_time"
    );
    assert_eq!(
        lifecycle_payload(&confirmed[0].lifecycle_events[0])
            .score
            .home,
        0
    );
    assert_eq!(rollback_events::confirm(&mut timeline, 0).len(), 0);
}
