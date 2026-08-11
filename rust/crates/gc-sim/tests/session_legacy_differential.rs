//! Differential test of the ACTUAL path `crates/gc-wasm/src/session.rs`'s
//! `Session` now runs for an ordinary (non-rollback, non-online) browser
//! match, against reference vectors captured from real Lua `sim/match.lua`,
//! per `tools/lua_reference/README.md`.
//!
//! `match_differential.rs` (in this same directory) already proves
//! `gc_sim::r#match::step`'s legacy branch is bit-exact against Lua for
//! 7,201 ticks -- but it does so with `human_controlled: Some(false)` (no
//! player takes the human-input branch at all) and steps with
//! `StepInput::Legacy(MatchInput::default())` constructed directly in Rust.
//! Neither of those is quite what the browser does: `Session::new` builds
//! `human_controlled: None` (defaults `true` -- one player, `controlled`,
//! DOES take the human-input branch), and `Session::step` does not
//! construct a `MatchInput` directly -- it decodes a canonical
//! `gc_sim::input_frame` wire string, reads the single `home_1` sample out
//! of it, and dequantizes THAT into a `MatchInput` via
//! `gc_sim::slot_input::to_match_input`, exactly as documented in
//! `crates/gc-wasm/src/session.rs`'s module doc ("An ordinary session is
//! LEGACY mode"). That wire round trip
//! (`input_frame::encode`/`::decode`/`::validate` plus
//! `slot_input::to_match_input`) is a real, additional layer
//! `match_differential.rs` does not exercise at all, and it is exactly the
//! layer this wave's bug (browser matches diverging from Lua) turned out to
//! matter for: a structurally different path can be fully deterministic and
//! still not be Lua.
//!
//! This test reproduces `Session::new`/`Session::step`'s LIVE-state
//! construction and per-tick stepping using `gc-sim`'s own public API
//! directly (this crate cannot depend on `gc-wasm`, which depends on it --
//! see README.md's layer rule), so it is a faithful stand-in for what
//! the wasm binding actually runs, minus the wasm-bindgen marshalling
//! itself (which carries no simulation logic of its own -- see
//! `session.rs`'s `Session::step` body, four lines of glue around exactly
//! the calls this test makes).
//!
//! The fixture (`fixtures/session_legacy_ordinary_lua_reference.txt`) was
//! captured by running the unmodified Lua tree under headless `love`
//! (`tools/lua_reference/capture_session_legacy_ordinary_match.lua`, see
//! that script's own header) for an ORDINARY match exactly as
//! `game/screens/match.lua`'s `Match:restart` built one outside the
//! development-only rollback lab: no `input_ownership`, no
//! `human_controlled` override (so it defaults `true`), teams
//! nebula/orion, seed 5, a 960x540 field, `match.step`'s default
//! duration/goal-limit (120 seconds, no goal cap), fed
//! `slot_input.neutral_match_input()` every tick (an idle/AFK local
//! player -- the same scenario `crates/gc-wasm/src/session.rs`'s own
//! `a_full_match_on_an_idle_local_wire_...` regression test drives) for the
//! full 7,200-tick match (a complete match, not a pinned excerpt --
//! capturing it took well under a second, so there was no reason to pin
//! anything shorter), printing the same per-tick field layout
//! `match_differential.rs`'s own fixture uses (see that file's header for
//! the exact field list).
//!
//! Every field is compared at every tick, and floats are compared by bit
//! pattern (`f64::to_bits`) after parsing, not by printed text -- see
//! `tools/lua_reference/README.md`'s warning that a divergence which
//! self-corrects a tick later is still a desync.

use gc_sim::input_frame;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::PitchSize;
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/session_legacy_ordinary_lua_reference.txt");

const PLAYER_COUNT: usize = 10;
/// Field count: 11 scalar fields (tick, 6 ball fields, owner, 2 scores, rng)
/// plus 2 (x, y) per player. Identical layout to `match_differential.rs`'s
/// own fixture.
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

/// Builds the same all-neutral canonical wire
/// `packages/app/src/sim_host.ts`'s `WasmSimHost.step` would encode for an
/// idle/AFK local player, and runs it through the exact decode -> validate
/// -> dequantize pipeline `crates/gc-wasm/src/session.rs`'s `Session::step`
/// runs, rather than constructing a `MatchInput` directly -- that pipeline
/// is the layer this differential exists to cover.
fn neutral_local_match_input(tick: i64) -> gc_sim::match_snapshot::MatchInput {
    let frame = input_frame::new(tick, None).expect("an all-neutral frame is always valid");
    let wire = input_frame::encode(&frame).expect("a valid frame always encodes");
    let decoded = input_frame::decode(&wire).expect("a wire this test just encoded always decodes");
    input_frame::validate(&decoded).expect("a freshly encoded neutral frame is always valid");
    // Index 0 is the canonical `home_1` slot -- `LOCAL_SLOT_ZERO_INDEX` in
    // `crates/gc-wasm/src/session.rs`.
    slot_input::to_match_input(&decoded.slots[0])
}

#[test]
fn wire_driven_session_legacy_step_matches_lua_tick_by_tick_for_a_7200_tick_ordinary_match() {
    let tune = Tuning::new();
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    // Mirrors `crates/gc-wasm/src/session.rs`'s `Session::new` construction
    // of its LIVE state exactly: no `input_ownership` (legacy mode), no
    // `human_controlled` override (defaults `true`) -- and mirrors the
    // capture script's own `match.new({home=, away=, field=, seed=5})`,
    // which likewise never overrides duration/max_goals, so both sides take
    // the same defaults (120 seconds, no goal cap).
    let mut s = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(5.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
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
            let match_input = neutral_local_match_input(row.tick - 1);
            sim_match::step(
                &mut s,
                1.0 / 60.0,
                StepInput::Legacy(match_input),
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
