//! Tests for `gc_sim::metrics`.
//!
//! `metrics::observe` reads a minimal match-shaped view rather than a full
//! `MatchState`: [`fake_state`]/[`frame`] below build that minimal surface,
//! typed against [`gc_sim::metrics::MetricsMatchView`] — enough surface for
//! the collector, not a real match.
//!
//! `metrics::observe` takes an explicit `&Tuning` argument instead of
//! reading a mutable global singleton (see `crate::tuning`'s doc); every
//! case here passes a fresh default [`Tuning`], so `TUNE.DRIBBLE_CLOSE`
//! stays at its default throughout.

use gc_core::vec2::Vec2;
use gc_sim::metrics::{
    self, MatchTeam, MetricsEventView, MetricsMatchView, MetricsPlayerView, Score,
};
use gc_sim::tuning::Tuning;

fn player(id: &str, team: MatchTeam, is_keeper: bool) -> MetricsPlayerView {
    MetricsPlayerView {
        id: id.to_string(),
        team,
        is_keeper,
        vel: Vec2::new(0.0, 0.0),
        run_vel: Vec2::new(0.0, 0.0),
        move_speed: 180.0,
        sprinting: false,
        dodge_timer: 0.0,
        keeper_state: None,
    }
}

fn fake_state() -> MetricsMatchView {
    MetricsMatchView {
        players: vec![
            player("h_keeper", MatchTeam::Home, true),
            player("h1", MatchTeam::Home, false),
            player("h2", MatchTeam::Home, false),
            player("a_keeper", MatchTeam::Away, true),
            player("a1", MatchTeam::Away, false),
        ],
        human_controlled: true,
        controlled: 1, // "h1"
        score: Score::default(),
        owner: None,
        events: Vec::new(),
    }
}

fn event(kind: &str, player: Option<&str>) -> MetricsEventView {
    MetricsEventView {
        kind: kind.to_string(),
        player: player.map(str::to_string),
        shot_type: None,
        on_target: None,
        keeper_state: None,
        keeper_depth: None,
    }
}

/// One observed frame: set the frame's events/owner, then observe `dt`
/// (defaults to 1 second).
struct Frame {
    events: Vec<MetricsEventView>,
    owner: Option<usize>,
    dt: f64,
}

impl Default for Frame {
    fn default() -> Self {
        Frame {
            events: Vec::new(),
            owner: None,
            dt: 1.0,
        }
    }
}

fn frame(c: &mut metrics::MetricsCollector, s: &mut MetricsMatchView, o: Frame, tuning: &Tuning) {
    s.events = o.events;
    s.owner = o.owner;
    metrics::observe(c, s, o.dt, tuning);
}

#[test]
fn metrics_observe_counts_outfield_strikes_but_not_keeper_punts_as_shots() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("shot", Some("h1"))],
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("shot", Some("h_keeper"))],
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("header", Some("a1"))],
            ..Default::default()
        },
        &tuning,
    );
    assert_eq!(
        c.shots, 2,
        "keeper 'shot' events are clearances, not strikes"
    );
}

#[test]
fn metrics_observe_resolves_a_pass_as_completed_when_the_same_team_regains_ownership() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("pass", Some("h1"))],
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(2),
            ..Default::default()
        },
        &tuning,
    ); // h2 collects
    assert_eq!(c.passes, 1);
    assert_eq!(c.passes_completed, 1);
}

#[test]
fn metrics_observe_an_intercepted_pass_is_incomplete_and_counts_a_turnover() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            ..Default::default()
        },
        &tuning,
    ); // h1 owns
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("pass", Some("h1"))],
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(4),
            ..Default::default()
        },
        &tuning,
    ); // a1 cuts it out
    assert_eq!(c.passes_completed, 0);
    assert_eq!(c.turnovers, 1);
}

#[test]
fn metrics_observe_splits_possession_time_by_owning_team_and_bridges_loose_spells() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            dt: 3.0,
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: None,
            dt: 5.0,
            ..Default::default()
        },
        &tuning,
    ); // loose: no turnover, no owned time
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(2),
            dt: 1.0,
            ..Default::default()
        },
        &tuning,
    ); // same team regains
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(4),
            dt: 2.0,
            ..Default::default()
        },
        &tuning,
    );
    assert!((c.own_time.home - 4.0).abs() < 1e-9);
    assert!((c.own_time.away - 2.0).abs() < 1e-9);
    assert_eq!(
        c.turnovers, 1,
        "home->home across the gap is not a turnover"
    );
}

#[test]
fn metrics_observe_ownership_flicker_below_the_settle_threshold_is_not_a_turnover() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            dt: 1.0,
            ..Default::default()
        },
        &tuning,
    ); // settled home
    // A poke-and-scramble: the ball ping-pongs in sub-settle touches.
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(4),
            dt: 0.2,
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            dt: 0.2,
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(4),
            dt: 0.2,
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            dt: 1.0,
            ..Default::default()
        },
        &tuning,
    ); // home rides out the scramble
    assert_eq!(
        c.turnovers, 0,
        "the scramble never settled with the other team"
    );
}

#[test]
fn metrics_observe_records_goals_from_score_deltas_and_tracks_droughts() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            dt: 30.0,
            ..Default::default()
        },
        &tuning,
    );
    s.score.home = 1;
    frame(
        &mut c,
        &mut s,
        Frame {
            dt: 1.0,
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            dt: 50.0,
            ..Default::default()
        },
        &tuning,
    );
    let m = metrics::finish(&mut c, &s);
    assert_eq!(m.goals_home, 1);
    assert!((c.goals[0].t - 31.0).abs() < 1e-9);
    assert!(
        (m.longest_drought_s - 50.0).abs() < 1e-9,
        "the post-goal tail is a drought"
    );
}

#[test]
fn metrics_observe_observes_keeper_state_release_depth_and_chip_outcomes_without_changing_play() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    s.players[0].keeper_state = Some(metrics::KeeperState::Advance);
    s.players[3].keeper_state = Some(metrics::KeeperState::Set);
    frame(
        &mut c,
        &mut s,
        Frame {
            dt: 0.5,
            events: vec![MetricsEventView {
                kind: "shot".to_string(),
                player: Some("h1".to_string()),
                shot_type: Some("chip".to_string()),
                keeper_state: Some(metrics::KeeperState::Advance),
                keeper_depth: Some(42.0),
                on_target: Some(true),
            }],
            ..Default::default()
        },
        &tuning,
    );
    s.score.home = 1;
    frame(
        &mut c,
        &mut s,
        Frame {
            dt: 0.5,
            ..Default::default()
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![MetricsEventView {
                kind: "parry".to_string(),
                player: Some("a_keeper".to_string()),
                shot_type: None,
                on_target: None,
                keeper_state: Some(metrics::KeeperState::Set),
                keeper_depth: None,
            }],
            ..Default::default()
        },
        &tuning,
    );

    let result = metrics::finish(&mut c, &s);
    assert!((result.keeper_advance_s - 2.0).abs() < 1e-6);
    assert!((result.keeper_set_s - 2.0).abs() < 1e-6);
    assert_eq!(result.keeper_shot_depth_mean, Some(42.0));
    assert_eq!(result.chip_shots, 1);
    assert_eq!(result.chip_on_target, 1);
    assert_eq!(result.chip_goals, 1);
    assert_eq!(result.chip_conversion, Some(1.0));
    assert_eq!(result.keeper_goals_advance, 1);
    assert_eq!(result.keeper_saves_set, 1);
}

#[test]
fn metrics_observe_keeps_aerial_goals_and_context_free_saves_explicitly_unclassified() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("header", Some("a1"))],
            ..Default::default()
        },
        &tuning,
    );
    s.score.away = 1;
    frame(&mut c, &mut s, Frame::default(), &tuning);
    frame(
        &mut c,
        &mut s,
        Frame {
            events: vec![event("catch", Some("h_keeper"))],
            ..Default::default()
        },
        &tuning,
    );

    let result = metrics::finish(&mut c, &s);
    assert_eq!(result.keeper_goals_unclassified, 1);
    assert_eq!(result.keeper_saves_unclassified, 1);
    assert_eq!(result.keeper_goals_base, 0);
    assert_eq!(result.keeper_saves_base, 0);
}

#[test]
fn metrics_observe_separates_controlled_and_team_ai_dribble_usage() {
    let tuning = Tuning::new();
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            dt: 1.0,
            ..Default::default()
        },
        &tuning,
    ); // controlled close control
    s.players[1].vel = Vec2::new(260.0, 0.0);
    s.players[1].sprinting = true;
    s.players[1].dodge_timer = 0.1;
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(1),
            dt: 1.0,
            events: vec![event("touch", Some("h1")), event("juke", Some("h1"))],
        },
        &tuning,
    );
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: None,
            events: vec![event("touch", Some("h1"))],
            ..Default::default()
        },
        &tuning,
    );
    s.players[4].vel = Vec2::new(-180.0, 0.0);
    frame(
        &mut c,
        &mut s,
        Frame {
            owner: Some(4),
            dt: 2.0,
            ..Default::default()
        },
        &tuning,
    ); // team AI, close control
    let m = metrics::finish(&mut c, &s);
    assert!((m.controlled_dribble_carry_s - 2.0).abs() < 1e-9);
    assert_eq!(m.controlled_dribble_close_share, Some(0.5));
    assert_eq!(m.controlled_dribble_sprint_share, Some(0.5));
    assert_eq!(m.controlled_jukes, 1);
    assert!((m.controlled_dribble_touches_per_min - 30.0).abs() < 1e-9);
    assert!((m.controlled_dribble_heavy_losses_per_min - 30.0).abs() < 1e-9);
    assert!((m.ai_dribble_carry_s - 2.0).abs() < 1e-9);
    assert_eq!(m.ai_dribble_close_share, Some(1.0));
}

#[test]
fn metrics_finish_computes_decided_late_as_when_the_winner_went_ahead_for_good() {
    let s = fake_state();
    let mut c = metrics::new(&s);
    c.t = 100.0;
    c.goals = vec![
        metrics::GoalEvent {
            t: 10.0,
            team: MatchTeam::Away,
        },
        metrics::GoalEvent {
            t: 40.0,
            team: MatchTeam::Home,
        },
        metrics::GoalEvent {
            t: 60.0,
            team: MatchTeam::Home,
        }, // home ahead for good from here
    ];
    let mut s = s;
    s.score.home = 2;
    s.score.away = 1;
    c.prev_home = 2;
    c.prev_away = 1;
    let m = metrics::finish(&mut c, &s);
    assert!((m.decided_late - 0.6).abs() < 1e-9);
    assert_eq!(m.lead_changes, 1, "away led, then home took over");
}

#[test]
fn metrics_finish_a_draw_is_undecided_until_the_end() {
    let mut s = fake_state();
    let mut c = metrics::new(&s);
    c.t = 100.0;
    c.goals = vec![
        metrics::GoalEvent {
            t: 10.0,
            team: MatchTeam::Home,
        },
        metrics::GoalEvent {
            t: 90.0,
            team: MatchTeam::Away,
        },
    ];
    s.score.home = 1;
    s.score.away = 1;
    let m = metrics::finish(&mut c, &s);
    assert!((m.decided_late - 1.0).abs() < 1e-9);
}

#[test]
fn metrics_finish_rate_metrics_are_nil_when_their_denominator_never_happened() {
    let s = fake_state();
    let mut c = metrics::new(&s);
    c.t = 100.0;
    let m = metrics::finish(&mut c, &s);
    assert!(m.shots_per_goal.is_none(), "no goals -> no shots_per_goal");
    assert!(m.save_rate.is_none(), "nothing on target -> no save_rate");
    assert!(m.pass_completion.is_none(), "no passes -> no completion");
    assert!(m.possession_balance.is_none(), "never owned -> no balance");
}

#[test]
fn metrics_desirability_is_1_inside_the_good_band_and_0_at_the_hard_edges() {
    let band = [0.0, 2.0, 5.0, 8.0];
    assert_eq!(metrics::desirability(3.0, band), 1.0);
    assert_eq!(metrics::desirability(0.0, band), 0.0);
    assert_eq!(metrics::desirability(8.0, band), 0.0);
    assert_eq!(metrics::desirability(10.0, band), 0.0);
}

#[test]
fn metrics_desirability_falls_off_linearly_between_the_edges() {
    let band = [0.0, 2.0, 5.0, 8.0];
    assert!((metrics::desirability(1.0, band) - 0.5).abs() < 1e-9);
    assert!((metrics::desirability(6.5, band) - 0.5).abs() < 1e-9);
}

fn good_match() -> metrics::MatchMetrics {
    metrics::MatchMetrics {
        goals_total: 3,
        shots_per_goal: Some(4.0),
        save_rate: Some(0.6),
        pass_completion: Some(0.7),
        turnovers_per_min: 4.0,
        possession_balance: Some(0.5),
        longest_drought_s: 20.0,
        decided_late: 0.8,
        ..Default::default()
    }
}

#[test]
fn metrics_fun_score_scores_1_when_every_metric_sits_in_its_band() {
    let (score, _) = metrics::fun_score(&good_match());
    assert!((score - 1.0).abs() < 1e-9);
}

#[test]
fn metrics_fun_score_a_single_collapsed_dimension_zeroes_the_score_geometric_mean() {
    let mut m = good_match();
    m.goals_total = 0;
    let (score, _) = metrics::fun_score(&m);
    assert_eq!(score, 0.0);
}

#[test]
fn metrics_fun_score_skips_missing_metrics_instead_of_defaulting_them() {
    let mut m = good_match();
    m.save_rate = None;
    let (score, per) = metrics::fun_score(&m);
    assert!((score - 1.0).abs() < 1e-9);
    assert!(!per.contains_key("save_rate"));
}

#[test]
fn metrics_aggregate_computes_mean_sd_min_max_per_key_skipping_nils() {
    let mut a = indexmap::IndexMap::new();
    a.insert("goals_total", 2.0);
    a.insert("save_rate", 0.5);
    let mut b = indexmap::IndexMap::new();
    b.insert("goals_total", 4.0);
    let mut d = indexmap::IndexMap::new();
    d.insert("goals_total", 6.0);
    d.insert("save_rate", 0.7);

    let agg = metrics::aggregate(&[a, b, d]);
    assert_eq!(agg["goals_total"].n, 3);
    assert!((agg["goals_total"].mean - 4.0).abs() < 1e-9);
    assert!((agg["goals_total"].sd - 2.0).abs() < 1e-9);
    assert_eq!(agg["goals_total"].min, 2.0);
    assert_eq!(agg["goals_total"].max, 6.0);
    assert_eq!(agg["save_rate"].n, 2);
    assert!((agg["save_rate"].mean - 0.6).abs() < 1e-9);
}

// ---------------------------------------------------------------------
// `time_to_reverse` (#488). Tier-1 tests for the detector's own logic:
// `metrics::observe` is pure over a view, so each case is a scripted speed
// profile checked in microseconds rather than inferred from a 24-seed
// harness run. The harness assertions live in `knob_contract.rs` and prove
// something different — that the knobs reach it.
// ---------------------------------------------------------------------

/// Drive one player through a scripted sequence of `run_vel` values, one per
/// tick at 60Hz, and return the finished metrics.
fn drive_run_vel(sequence: &[Vec2]) -> gc_sim::metrics::MatchMetrics {
    let tune = Tuning::new();
    let mut state = fake_state();
    let mut c = metrics::new(&state);
    for v in sequence {
        state.players[1].run_vel = *v;
        metrics::observe(&mut c, &state, 1.0 / 60.0, &tune);
    }
    metrics::finish(&mut c, &state)
}

/// The measurement itself: cruise, brake through zero, leave the other way.
///
/// The clock starts on the last tick the body was still cruising, so a
/// three-tick reversal at 60Hz measures 0.05s exactly. Asserting the exact
/// value rather than a range is deliberate — this is integer tick counting,
/// and a fencepost error is the defect most likely to live here.
#[test]
fn time_to_reverse_measures_a_brake_through_zero_reversal() {
    // move_speed is 180 for the fixture's players.
    let m = drive_run_vel(&[
        Vec2::new(180.0, 0.0), // cruising: arms, resets the clock each tick
        Vec2::new(180.0, 0.0),
        Vec2::new(90.0, 0.0),  // braking, below the arm threshold
        Vec2::new(9.0, 0.0),   // through zero
        Vec2::new(-90.0, 0.0), // rebuilt half speed, opposite: complete
    ]);
    assert_eq!(m.reversals, 1);
    let measured = m.time_to_reverse.expect("one reversal was completed");
    assert!(
        (measured - 3.0 / 60.0).abs() < 1e-12,
        "expected exactly three ticks, got {measured}"
    );
}

/// A wide arc is not a reversal, and this is the assertion that keeps the two
/// populations apart.
///
/// The body ends up going the other way at speed, so every other condition is
/// met — but it never comes near a standstill, so it went around rather than
/// through. Without this gate the metric measures "ended up going the other
/// way", which folds in a population where braking barely matters and dilutes
/// the term the metric exists to measure.
#[test]
fn time_to_reverse_ignores_a_turn_that_kept_its_pace() {
    let m = drive_run_vel(&[
        Vec2::new(180.0, 0.0),
        Vec2::new(90.0, 90.0), // carries pace around the corner
        Vec2::new(0.0, 120.0),
        Vec2::new(-90.0, 90.0),
        Vec2::new(-120.0, 0.0), // opposite, at speed, but never near zero
    ]);
    assert_eq!(m.reversals, 0);
    assert_eq!(
        m.time_to_reverse, None,
        "an arc must not be reported as a reversal"
    );
}

/// A body that stops for unrelated reasons and leaves again much later has
/// not reversed, and must not record a multi-second one.
#[test]
fn time_to_reverse_discards_a_cruise_nothing_followed() {
    let mut sequence = vec![Vec2::new(180.0, 0.0), Vec2::new(0.0, 0.0)];
    // Two and a half seconds of standing still, past REVERSAL_TIMEOUT_S.
    sequence.extend(std::iter::repeat_n(Vec2::new(0.0, 0.0), 150));
    sequence.push(Vec2::new(-180.0, 0.0));
    let m = drive_run_vel(&sequence);
    assert_eq!(
        m.reversals, 0,
        "a reversal separated from its cruise by seconds is not one reversal"
    );
}

/// Absence is reported as absence. `0.0` would be a *fast* reversal and would
/// read as the best possible score, so a match that never armed the
/// measurement would silently certify the primitive as perfect.
#[test]
fn time_to_reverse_is_none_rather_than_zero_when_nothing_reversed() {
    let m = drive_run_vel(&[Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0)]);
    assert_eq!(m.reversals, 0);
    assert_eq!(m.time_to_reverse, None);
}
