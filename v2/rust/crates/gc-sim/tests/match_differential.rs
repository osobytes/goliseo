//! Differential test of `gc_sim::r#match::step` against the real Lua
//! `sim/match.lua`, per `v2/tools/lua_reference/README.md`.
//!
//! The fixture (`fixtures/match_step_ai_ai_lua_reference.txt`) was captured
//! by running the unmodified Lua tree under headless `love` (`love .` in a
//! scratch copy of `core/`, `sim/`, `data/`, with graphics/audio/window
//! disabled) for a fully AI-driven match (`human_controlled = false`, so
//! this is deterministic without any human input stream), seed 11,
//! `nebula` vs `orion`, a 960x540 field, stepping `match.step(s, 1/60,
//! NO_INPUT)` 7200 times (120 real-time seconds — a full match, matching
//! `sim::outfield_ai_baseline::DURATION_SECONDS` and the tick count of
//! `determinism_evidence`'s frozen replay) and printing, every tick
//! (including tick 0, before any step): `ball.x`, `ball.y`, `ball_vel.x`,
//! `ball_vel.y`, `ball_z`, `ball_vz`, `owner` (-1 for a loose ball),
//! `score.home`, `score.away`, `rng`, then `players[i].pos.{x,y}` for
//! `i` = 1 through 10 (every outfielder and both keepers) — floats via
//! `%.17g`, which round-trips binary64 exactly.
//!
//! The fixture originally covered only 600 ticks. It was extended to the
//! full 7200-tick match length to chase a correctness defect where AI-driven
//! matches diverge from Lua over long play (see
//! `outfield_ai_baseline_reproduces_the_frozen_fixture_exactly`'s frozen-fixture
//! evidence): 600 ticks is not enough to exercise every AI decision branch,
//! and this test's job is to find the earliest tick, player, and quantity
//! where the port disagrees with the reference.
//!
//! Every field is compared at every tick (not just the last), and floats
//! are compared by bit pattern (`f64::to_bits`) after parsing, not by
//! printed text — see the porting README's warning that a divergence which
//! self-corrects a tick later is still a desync.

use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::PitchSize;
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/match_step_ai_ai_lua_reference.txt");

const PLAYER_COUNT: usize = 10;
/// Field count: 11 scalar fields (tick, 6 ball fields, owner, 2 scores, rng)
/// plus 2 (x, y) per player.
const FIELD_COUNT: usize = 11 + PLAYER_COUNT * 2;

struct Row {
    tick: i64,
    ball_x: f64,
    ball_y: f64,
    ball_vel_x: f64,
    ball_vel_y: f64,
    ball_z: f64,
    ball_vz: f64,
    owner: i64,
    score_home: i64,
    score_away: i64,
    rng: u32,
    players: [(f64, f64); PLAYER_COUNT],
}

fn parse_row(line: &str) -> Row {
    let f: Vec<&str> = line.split('\t').collect();
    assert_eq!(
        f.len(),
        FIELD_COUNT,
        "fixture row must have {FIELD_COUNT} tab-separated fields"
    );
    let mut players = [(0.0_f64, 0.0_f64); PLAYER_COUNT];
    for (i, slot) in players.iter_mut().enumerate() {
        let base = 11 + i * 2;
        *slot = (f[base].parse().unwrap(), f[base + 1].parse().unwrap());
    }
    Row {
        tick: f[0].parse().unwrap(),
        ball_x: f[1].parse().unwrap(),
        ball_y: f[2].parse().unwrap(),
        ball_vel_x: f[3].parse().unwrap(),
        ball_vel_y: f[4].parse().unwrap(),
        ball_z: f[5].parse().unwrap(),
        ball_vz: f[6].parse().unwrap(),
        owner: f[7].parse().unwrap(),
        score_home: f[8].parse().unwrap(),
        score_away: f[9].parse().unwrap(),
        rng: f[10].parse().unwrap(),
        players,
    }
}

fn assert_bits_eq(actual: f64, expected: f64, tick: i64, field: &str) {
    assert!(
        actual.to_bits() == expected.to_bits(),
        "tick {tick} field {field}: bit-exact mismatch — expected {expected:.17}, got \
         {actual:.17} (expected bits {:x}, got bits {:x})",
        expected.to_bits(),
        actual.to_bits(),
    );
}

#[test]
fn match_step_matches_lua_tick_by_tick_for_a_7200_tick_ai_vs_ai_match() {
    let tune = Tuning::new();
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut s = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(11.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: None,
    });

    let rows: Vec<Row> = FIXTURE.lines().map(parse_row).collect();
    assert_eq!(
        rows.len(),
        7201,
        "fixture must cover tick 0 through tick 7200"
    );

    let mut compared = 0;
    for row in &rows {
        if row.tick > 0 {
            sim_match::step(
                &mut s,
                1.0 / 60.0,
                StepInput::Legacy(gc_sim::match_snapshot::MatchInput::default()),
                None,
                &tune,
            );
        }
        let tick = row.tick;
        assert_bits_eq(s.ball.x, row.ball_x, tick, "ball.x");
        assert_bits_eq(s.ball.y, row.ball_y, tick, "ball.y");
        assert_bits_eq(s.ball_vel.x, row.ball_vel_x, tick, "ball_vel.x");
        assert_bits_eq(s.ball_vel.y, row.ball_vel_y, tick, "ball_vel.y");
        assert_bits_eq(s.ball_z, row.ball_z, tick, "ball_z");
        assert_bits_eq(s.ball_vz, row.ball_vz, tick, "ball_vz");
        let owner = s.owner.unwrap_or(-1);
        assert_eq!(owner, row.owner, "tick {tick}: owner mismatch");
        assert_eq!(
            s.score.home, row.score_home,
            "tick {tick}: score.home mismatch"
        );
        assert_eq!(
            s.score.away, row.score_away,
            "tick {tick}: score.away mismatch"
        );
        assert_eq!(s.rng, row.rng, "tick {tick}: rng state mismatch");
        for (i, &(px, py)) in row.players.iter().enumerate() {
            let field_x = format!("players[{}].pos.x", i + 1);
            let field_y = format!("players[{}].pos.y", i + 1);
            assert_bits_eq(s.players[i].pos.x, px, tick, &field_x);
            assert_bits_eq(s.players[i].pos.y, py, tick, &field_y);
        }
        compared += 1;
    }
    assert_eq!(compared, 7201);
}
