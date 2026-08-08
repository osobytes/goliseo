//! Port of `spec/sim/bot_spec.lua`.
//!
//! Six of the eight Lua cases call `new_match()` (`sim.match.new`) only to
//! get a plausible ten-player fixture, then drive `bot.input` directly —
//! `match.step` is never called, so nothing in them actually depends on real
//! match physics. [`base_state`] below hand-builds that same minimal
//! surface, the same way `spec/sim/metrics_spec.lua` and this crate's
//! `tests/metrics.rs` already do for the identical reason (see that
//! module's doc): a `BotMatchView` with two full 5-a-side rosters at
//! plausible positions, `controlled` pointing at a home outfielder, and
//! `owner` defaulted to `controlled` — matching `sim/match.lua`'s
//! `place_kickoff` default (the kicking team's most advanced player owns the
//! ball at kickoff; see `sim/match.lua` around `s.owner = kicker`).
//!
//! The remaining two cases ("same seed produces the identical match", "keeps
//! the controlled player active") call `match.step` in a loop to run real
//! physics for up to 600 ticks. `sim::match` (`gc_sim::r#match`) is now
//! fully ported, so both cases build a real fixture via `sim_match::new`
//! and drive `sim_match::step`. Neither `bot.rs` nor its test file own
//! `crates/gc-sim/src/headless.rs`'s private `to_bot_view`/`to_bot_player`
//! adapters, so [`to_bot_view`] below is a test-local copy of that same
//! conversion (`MatchState`/`MatchPlayer` -> `BotMatchView`/`BotPlayerView`).
//!
//! `bot::input` additionally takes an explicit `&Tuning` where the Lua
//! closes over the `sim.tuning` global (AGENTS.md §3; see
//! `gc_sim::tuning`'s doc); every case here passes a fresh default
//! `Tuning`, matching the Lua spec's untouched tuning defaults.

use gc_core::vec2::Vec2;
use gc_sim::bot::{self, BotMatchView, BotOptions, BotPlayerView, Field, Team};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{self as snap, MatchState, PitchSize};
use gc_sim::tuning::Tuning;

fn to_bot_player(p: &snap::MatchPlayer) -> BotPlayerView {
    BotPlayerView {
        team: match p.team {
            snap::Team::Home => Team::Home,
            snap::Team::Away => Team::Away,
        },
        is_keeper: p.is_keeper,
        pos: p.pos,
        tackle_timer: p.tackle_timer,
        slide_timer: p.slide_timer,
        dodge_cd: p.dodge_cd,
        sprint_meter: p.sprint_meter,
    }
}

fn to_bot_view(s: &MatchState) -> BotMatchView {
    BotMatchView {
        players: s.players.iter().map(to_bot_player).collect(),
        field: Field {
            w: s.field.w,
            h: s.field.h,
        },
        controlled: (s.controlled - 1) as usize,
        owner: s.owner.map(|i| (i - 1) as usize),
        ball: s.ball,
        ball_vel: s.ball_vel,
        ball_z: s.ball_z,
        ball_vz: s.ball_vz,
    }
}

fn new_match(seed: f64) -> MatchState {
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
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

fn player(team: Team, is_keeper: bool, pos: Vec2) -> BotPlayerView {
    BotPlayerView {
        team,
        is_keeper,
        pos,
        tackle_timer: 0.0,
        slide_timer: 0.0,
        dodge_cd: 0.0,
        sprint_meter: 1.0,
    }
}

/// A plausible 5-a-side-per-team fixture: index 0 is the home keeper, 1..4
/// are home outfielders (1 is `controlled`), 5 is the away keeper, 6..9 are
/// away outfielders. `owner` starts at `controlled`, mirroring
/// `place_kickoff`'s "kicking team's most advanced player owns the ball".
fn base_state() -> BotMatchView {
    let players = vec![
        player(Team::Home, true, Vec2::new(40.0, 270.0)),
        player(Team::Home, false, Vec2::new(400.0, 270.0)),
        player(Team::Home, false, Vec2::new(0.0, 0.0)),
        player(Team::Home, false, Vec2::new(0.0, 0.0)),
        player(Team::Home, false, Vec2::new(0.0, 0.0)),
        player(Team::Away, true, Vec2::new(920.0, 270.0)),
        player(Team::Away, false, Vec2::new(0.0, 0.0)),
        player(Team::Away, false, Vec2::new(0.0, 0.0)),
        player(Team::Away, false, Vec2::new(0.0, 0.0)),
        player(Team::Away, false, Vec2::new(0.0, 0.0)),
    ];
    BotMatchView {
        players,
        field: Field { w: 960.0, h: 540.0 },
        controlled: 1,
        owner: Some(1),
        ball: Vec2::new(0.0, 0.0),
        ball_vel: Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
    }
}

const DT: f64 = 1.0 / 60.0;

#[test]
fn charges_and_releases_a_shot_when_in_range_of_the_goal() {
    let mut s = base_state();
    let controlled = s.controlled;
    s.players[controlled].pos = Vec2::new(800.0, 270.0); // well inside shooting range
    s.ball = s.players[controlled].pos.add(Vec2::new(18.0, 0.0));
    let tune = Tuning::new();
    let mut b = bot::new(BotOptions {
        seed: Some(1.0),
        ..Default::default()
    });
    let (mut held, mut fired) = (false, false);
    for _ in 0..60 {
        let input = bot::input(&mut b, &s, DT, &tune);
        held = held || input.shoot_held;
        if input.shoot {
            fired = true;
            break;
        }
    }
    assert!(held, "the bot builds charge before shooting");
    assert!(fired, "and releases the shot");
}

#[test]
fn passes_when_pressured_far_from_goal() {
    let mut s = base_state();
    let controlled = s.controlled;
    s.players[controlled].pos = Vec2::new(200.0, 270.0); // own half: out of shooting range
    s.ball = s.players[controlled].pos.add(Vec2::new(18.0, 0.0));
    // An opponent right on top of the carrier.
    let away_idx = s
        .players
        .iter()
        .position(|p| p.team == Team::Away && !p.is_keeper)
        .expect("an away outfielder");
    s.players[away_idx].pos = Vec2::new(230.0, 270.0);
    let tune = Tuning::new();
    let mut b = bot::new(BotOptions {
        seed: Some(1.0),
        ..Default::default()
    });
    let mut passed = false;
    for _ in 0..30 {
        let input = bot::input(&mut b, &s, DT, &tune);
        if input.pass {
            passed = true;
            break;
        }
    }
    assert!(passed, "the pressured bot moves the ball on");
}

#[test]
fn fires_one_shot_actions_for_exactly_one_frame() {
    let s = base_state();
    let tune = Tuning::new();
    let mut b = bot::new(BotOptions {
        seed: Some(1.0),
        ..Default::default()
    });
    b.pass = true;
    b.decide_t = 10.0; // no re-decision: isolate the latch
    let first = bot::input(&mut b, &s, DT, &tune);
    let second = bot::input(&mut b, &s, DT, &tune);
    assert!(first.pass, "the queued pass fires");
    assert!(!second.pass, "and does not repeat");
}

#[test]
fn jukes_a_nearby_defender_who_has_committed() {
    let mut s = base_state();
    let controlled = s.controlled;
    s.players[controlled].pos = Vec2::new(300.0, 270.0);
    s.ball = s.players[controlled].pos.add(Vec2::new(18.0, 0.0));
    let away_idx = s
        .players
        .iter()
        .position(|p| p.team == Team::Away && !p.is_keeper)
        .expect("an away outfielder");
    s.players[away_idx].pos = Vec2::new(340.0, 270.0);
    s.players[away_idx].tackle_timer = 0.2;
    let tune = Tuning::new();
    let mut dodged = false;
    for seed in 1..=50 {
        let mut b = bot::new(BotOptions {
            seed: Some(f64::from(seed)),
            ..Default::default()
        });
        let input = bot::input(&mut b, &s, DT, &tune);
        dodged = dodged || input.dodge;
    }
    assert!(
        dodged,
        "the proxy sometimes represents the player's reactive juke"
    );
}

#[test]
fn charges_a_deliberate_long_pass() {
    let mut s = base_state();
    let controlled = s.controlled;
    s.players[controlled].pos = Vec2::new(180.0, 270.0);
    s.ball = s.players[controlled].pos.add(Vec2::new(18.0, 0.0));
    for (i, p) in s.players.iter_mut().enumerate() {
        if p.team == Team::Home && i != controlled && !p.is_keeper {
            p.pos = Vec2::new(520.0, 100.0 + i as f64 * 40.0);
        } else if p.team == Team::Away && !p.is_keeper {
            p.pos = Vec2::new(230.0, 270.0);
        }
    }
    let tune = Tuning::new();
    let mut b = bot::new(BotOptions {
        seed: Some(2.0),
        ..Default::default()
    });
    let _ = bot::input(&mut b, &s, DT, &tune); // decision queues the charged pass
    let held = bot::input(&mut b, &s, DT, &tune);
    assert!(
        held.pass_held,
        "long options use the same range charge as a player"
    );
}

#[test]
fn requests_an_aerial_strike_under_a_dropping_ball() {
    let mut s = base_state();
    s.owner = None;
    let me_pos = s.players[s.controlled].pos;
    s.ball = me_pos.add(Vec2::new(20.0, 0.0));
    s.ball_z = 40.0;
    s.ball_vz = -100.0;
    let tune = Tuning::new();
    let mut b = bot::new(BotOptions {
        seed: Some(3.0),
        ..Default::default()
    });
    let input = bot::input(&mut b, &s, DT, &tune);
    assert_eq!(
        input.aerial_strike,
        Some(true),
        "the proxy can represent first-time aerial intent"
    );
}

#[test]
fn same_seed_produces_the_identical_match() {
    let tune = Tuning::new();
    fn play(seed: f64, tune: &Tuning) -> MatchState {
        let mut s = new_match(seed);
        let mut b = bot::new(BotOptions {
            seed: Some(seed),
            ..Default::default()
        });
        for _ in 0..600 {
            // 10 seconds
            let view = to_bot_view(&s);
            let input = bot::input(&mut b, &view, DT, tune);
            sim_match::step(&mut s, DT, StepInput::Legacy(input), None, tune);
        }
        s
    }
    let a = play(7.0, &tune);
    let c = play(7.0, &tune);
    assert!((a.ball.x - c.ball.x).abs() <= 1e-9);
    assert!((a.ball.y - c.ball.y).abs() <= 1e-9);
    assert_eq!(a.score.home, c.score.home);
    assert_eq!(a.score.away, c.score.away);
    for (pa, pc) in a.players.iter().zip(c.players.iter()) {
        assert!((pa.pos.x - pc.pos.x).abs() <= 1e-9);
        assert!((pa.pos.y - pc.pos.y).abs() <= 1e-9);
    }
}

#[test]
fn keeps_the_controlled_player_active_not_a_statue() {
    let tune = Tuning::new();
    let mut s = new_match(3.0);
    let mut b = bot::new(BotOptions {
        seed: Some(3.0),
        ..Default::default()
    });
    let mut moved_frames = 0;
    for _ in 0..600 {
        let view = to_bot_view(&s);
        let input = bot::input(&mut b, &view, DT, &tune);
        if input.r#move.x != 0.0 || input.r#move.y != 0.0 {
            moved_frames += 1;
        }
        sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &tune);
    }
    assert!(moved_frames > 300, "the bot steers most frames");
}
