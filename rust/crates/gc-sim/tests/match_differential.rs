//! Determinism regression for `gc_sim::r#match::step` over a full AI-vs-AI
//! match, against a baseline recorded from THIS implementation.
//!
//! # What this proves
//!
//! One 7,200-tick fully AI-driven match (`human_controlled: false`, seed 11,
//! `nebula` vs `orion`, 960x540) stepped with
//! `StepInput::Legacy(MatchInput::default())` — no human input stream, no
//! wire round trip — reproduces a recorded trajectory bit for bit, at every
//! tick, across 31 fields: the six ball quantities, the owner, both scores,
//! the RNG state, and all ten players' positions. Floats are compared by
//! `f64::to_bits` after parsing, never by printed text, because a divergence
//! that self-corrects a tick later is still a desync.
//!
//! It is the broadest single reader of `match::step`'s AI path in the
//! workspace, which is exactly why an unintended perturbation anywhere under
//! it surfaces here first, with a tick and a field name.
//!
//! # Two assertions, and only one of them moves
//!
//! A frozen per-tick trajectory is broken by any deliberate gameplay change
//! **by construction**. Re-recording the Lua vector's shape in Rust makes it
//! regenerable, not immune — `tools/lua_reference/README.md` rule 5 is
//! explicit that this is not enough on its own. So:
//!
//!   1. `..._reproduces_its_recorded_baseline_...` — the pinned trajectory.
//!      Detects unintended change. This is the one a deliberate gameplay
//!      rework legitimately trips and re-records (see
//!      `record_match_step_ai_ai_baseline` below).
//!   2. `..._is_bit_reproducible_across_two_independent_runs` — the same
//!      match constructed and stepped twice, in one process, must agree
//!      bit-for-bit on all 31 fields at all 7,201 ticks. **IMMUNE to
//!      gameplay change**, because both runs move together. This is the
//!      determinism claim proper: `match::step` is a pure function of state
//!      and inputs, carrying no hidden global, no clock read, and no
//!      iteration-order dependence on a hashed container. It keeps gating
//!      through every rework with nothing to re-record.
//!
//! So a gameplay rework re-records one fixture and loses no determinism
//! coverage, and a defect that made `step` depend on anything outside its
//! arguments still fails the gate in a PR that is legitimately re-recording.
//!
//! # What this no longer proves, and why the evidence is weaker
//!
//! Until #520 this asserted against
//! `fixtures/match_step_ai_ai_lua_reference.txt`, captured from the original
//! Lua `sim/match.lua`. A pass was **cross-implementation** evidence: two
//! independently written simulations agreed bit for bit, which is the only
//! kind of evidence that can catch the Rust port being *wrong* rather than
//! merely *stable*. **That claim is retired. This test no longer says
//! anything about Lua.**
//!
//! The baseline it reads now was recorded from this very code:
//!
//!   * It DETECTS CHANGE. Any edit that perturbs one float at one tick goes
//!     red — which is what a determinism guarantee needs, because an
//!     unintended perturbation is a desync between a client that shipped
//!     before the edit and one that shipped after.
//!   * It CANNOT DETECT "WRONG BUT CONSISTENTLY WRONG". A bug present when
//!     the baseline was recorded is in the expectations forever, and this
//!     test will defend it. The Lua vector could catch that class; this
//!     cannot, and nothing else in the workspace replaces it, because there
//!     is no second implementation left to disagree with.
//!
//! Assertion 2 is not weakened this way — it compares two live runs against
//! each other rather than against a record — but it is not a substitute
//! either: it proves `step` is self-consistent, not that it is right.
//!
//! # The retirement
//!
//! Decision #520 (repository owner). Superseded by #516, the locomotion
//! rework for #488, which changes what every body on the pitch does per tick
//! and therefore diverges a per-tick trajectory by construction. The Lua
//! vector last held at `3f8f4a3`, verified green there by running this file
//! and its three siblings at that commit, not assumed. The vector file stays
//! in the tree, unmodified and unread: it is the record of a capture that can
//! never be taken again.

use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::PitchSize;
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/match_step_ai_ai_baseline.txt");

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

/// The one scenario this file pins, built identically everywhere it is
/// needed so the baseline assertion, the reproducibility assertion and the
/// recorder cannot drift apart into three subtly different matches.
fn fresh_match() -> gc_sim::match_snapshot::MatchState {
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
        seed: Some(11.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: None,
    })
}

/// One tick of the scenario's stepping, in one place for the same reason.
fn step_once(s: &mut gc_sim::match_snapshot::MatchState, tune: &Tuning) {
    sim_match::step(
        s,
        1.0 / 60.0,
        StepInput::Legacy(gc_sim::match_snapshot::MatchInput::default()),
        None,
        tune,
    );
}

/// The 31 compared quantities of one tick, as bits, in fixture column order.
fn observe(s: &gc_sim::match_snapshot::MatchState) -> Vec<u64> {
    let mut out = vec![
        s.ball.x.to_bits(),
        s.ball.y.to_bits(),
        s.ball_vel.x.to_bits(),
        s.ball_vel.y.to_bits(),
        s.ball_z.to_bits(),
        s.ball_vz.to_bits(),
        s.owner.unwrap_or(-1) as u64,
        s.score.home as u64,
        s.score.away as u64,
        u64::from(s.rng),
    ];
    for p in &s.players {
        out.push(p.pos.x.to_bits());
        out.push(p.pos.y.to_bits());
    }
    out
}

#[test]
fn match_step_reproduces_its_recorded_baseline_for_a_7200_tick_ai_vs_ai_match() {
    let tune = Tuning::new();
    let mut s = fresh_match();

    let rows: Vec<Row> = FIXTURE.lines().map(parse_row).collect();
    assert_eq!(
        rows.len(),
        7201,
        "fixture must cover tick 0 through tick 7200"
    );

    let mut compared = 0;
    for row in &rows {
        if row.tick > 0 {
            step_once(&mut s, &tune);
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

/// The determinism claim proper, and the half of this file that no gameplay
/// rework can move: `match::step` is a pure function of state and inputs, so
/// two independently constructed runs of the same scenario in the same
/// process agree on every bit of every field at every tick.
///
/// A defect that made stepping depend on anything outside its arguments — a
/// hidden global, a clock read, iteration order over a hashed container,
/// uninitialized memory — fails here while the recorded baseline above
/// happily passes on whichever trajectory got recorded. That is why this is
/// not redundant with assertion 1, and why it is the assertion to keep if
/// only one could survive.
#[test]
fn match_step_is_bit_reproducible_across_two_independent_runs() {
    let tune = Tuning::new();
    let mut first = fresh_match();
    let mut second = fresh_match();
    assert_eq!(
        observe(&first),
        observe(&second),
        "tick 0: two fresh matches of the same scenario differ before any step"
    );
    for tick in 1..=7_200 {
        step_once(&mut first, &tune);
        step_once(&mut second, &tune);
        assert_eq!(
            observe(&first),
            observe(&second),
            "tick {tick}: two independent runs of the same scenario diverged -- \
             `match::step` is reading something outside its arguments"
        );
    }
}

/// Records the baseline assertion 1 reads, printing it to stdout for a human
/// to capture. NOT CI-asserted and `#[ignore]`d so it never runs in the gate:
/// a recorder that overwrote its own fixture during `cargo test` would turn a
/// determinism regression into a no-op.
///
/// **Re-recording is a decision, not a fix.** A red baseline is a FINDING
/// first: something under `match::step`'s AI path changed. Re-record only
/// when that change is deliberate, reviewed and named in the commit message
/// — `tools/lua_reference/README.md`'s rule for behavioral vectors, which
/// this baseline inherits even though it is no longer a Lua vector.
///
/// Run:
///
/// ```text
/// cd rust
/// cargo test -p gc-sim --test match_differential -- \
///     --ignored --nocapture record_match_step_ai_ai_baseline \
///   | grep -E '^[0-9]' \
///   > crates/gc-sim/tests/fixtures/match_step_ai_ai_baseline.txt
/// ```
#[test]
#[ignore = "recorder: prints a baseline for a human to capture, never asserts"]
fn record_match_step_ai_ai_baseline() {
    let tune = Tuning::new();
    let mut s = fresh_match();
    for tick in 0..=7_200 {
        if tick > 0 {
            step_once(&mut s, &tune);
        }
        let mut row = vec![tick.to_string()];
        row.push(s.ball.x.to_string());
        row.push(s.ball.y.to_string());
        row.push(s.ball_vel.x.to_string());
        row.push(s.ball_vel.y.to_string());
        row.push(s.ball_z.to_string());
        row.push(s.ball_vz.to_string());
        row.push(s.owner.unwrap_or(-1).to_string());
        row.push(s.score.home.to_string());
        row.push(s.score.away.to_string());
        row.push(s.rng.to_string());
        for p in &s.players {
            row.push(p.pos.x.to_string());
            row.push(p.pos.y.to_string());
        }
        println!("{}", row.join("\t"));
    }
}
