//! Port of `spec/sim/rollback_events_spec.lua`.
//!
//! `sim/match.lua` is not ported yet (`gc_sim::r#match` is a placeholder),
//! and this module's owning test file must not port `sim/rollback_session.lua`
//! either (it needs `sim/match.lua` too, and is being ported by another
//! agent as `sim::match` lands). Every case that only manipulates
//! `MatchSnapshot`s directly — the majority of the spec — ports using a
//! hand-built `MatchState` fixture, the same pattern
//! `tests/match_snapshot_differential.rs`'s `base_state` and
//! `tests/rollback_snapshot_history.rs` use. The five cases that drive a
//! real `rollback_session` (`run_real_match`, and the two combat-request
//! cases) are `#[ignore]`d with the exact string the porting brief asks for.

use gc_core::vec2::Vec2;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::match_snapshot::{
    self, ByTeam, MatchEvent, MatchEventKind, MatchPlayer, MatchSnapshot, MatchState, PitchSize,
    Rect,
};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::possession_transition::{self, TransitionWindows};
use gc_sim::rollback_events::{
    self, RollbackEventPayload, RollbackEventStepInput, RollbackEventTickOutput,
    RollbackEventsErrorCode, RollbackEventsStatus, RollbackOutputStateView,
};

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
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn rollback_events_revokes_corrected_away_combat_events_and_confirms_the_replacement_once() {
    unimplemented!(
        "needs sim::combat.new_state driven through a real rollback_session, which needs sim::match"
    );
}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn rollback_events_carries_a_rejected_request_which_has_no_sequence_through_the_timeline() {
    unimplemented!(
        "needs sim::combat.new_state driven through a real rollback_session, which needs sim::match"
    );
}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn rollback_events_derives_real_goal_plus_kickoff_once_from_match_snapshots() {
    unimplemented!("needs a real rollback_session driving sim::match to a scored goal");
}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn rollback_events_derives_real_max_goal_full_time_without_kickoff() {
    unimplemented!("needs a real rollback_session driving sim::match to the goal cap");
}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn rollback_events_derives_timer_full_time_alone_and_no_opening_kickoff() {
    unimplemented!("needs a real rollback_session driving sim::match to the clock running out");
}
