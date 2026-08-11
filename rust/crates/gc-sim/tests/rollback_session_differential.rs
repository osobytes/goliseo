//! Differential test against reference vectors captured from the Lua
//! implementation this simulation was originally validated against (README
//! rule 9, `tools/lua_reference/README.md`), required for
//! `rollback_session`: it is the actual rollback state machine, and a
//! re-simulation that self-corrects to the wrong bits is a desync a
//! spec-only unit test cannot catch.
//!
//! `tests/fixtures/rollback_session_lua_reference.txt` is the captured
//! stdout of running the real Lua `sim/rollback_session.lua` under headless
//! `love` (no display, no `xvfb`), via a scratch `conf.lua`/`main.lua`
//! harness built per that README (not committed — scratch dirs are
//! session-local). This file reconstructs the identical scripted scenario
//! in Rust and asserts every captured hash line matches.
//!
//! ## The scenario
//!
//! One nebula-vs-orion fixture (seed 70, all eight slots `"remote"`, so
//! every prediction is neutral until authority arrives):
//!
//! - Ticks 0–9: fully authoritative-before-simulated (neutral), no
//!   divergence at all — establishes the boundary hash never drifts on the
//!   ordinary path.
//! - Ticks 10–19: predicted only, no authority supplied yet — ten
//!   purely-predicted boundaries.
//! - A late but in-window authoritative correction lands at tick 15, slot
//!   2 (`move_x = 50, move_y = -30`, diverging from the neutral prediction
//!   already consumed there), triggering one `reconcile` that rewinds to
//!   tick 15 and resimulates ticks 15–19.
//! - Ticks 20–24 continue forward, predicted, after the correction — proves
//!   the resimulated tail keeps producing consistent hashes going forward,
//!   not just at the moment of correction.
//!
//! Every boundary's canonical hash (`match_snapshot::hash_canonical`,
//! matching the harness's `match_snapshot.hash_canonical`) is captured
//! twice for ticks 15–19: once from the stale predicted timeline (`pre.*`)
//! and once from the corrected resimulation (`post.*`). The two disagree in
//! the fixture (a real, self-correcting divergence, not a no-op), so this
//! also proves this Rust implementation lands on the *new* bits, not merely
//! on *some* stable bits.

use gc_sim::input_frame::{self, InputSampleOptions};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot;
use gc_sim::match_snapshot::PitchSize;
use gc_sim::rollback_session::{self, RollbackSession};
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/rollback_session_lua_reference.txt");

fn reference() -> IndexMap<&'static str, &'static str> {
    FIXTURE
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("malformed fixture line: {line}"));
            (key, value)
        })
        .collect()
}

fn expect<'a>(reference: &IndexMap<&'a str, &'a str>, key: &str) -> &'a str {
    reference
        .get(key)
        .unwrap_or_else(|| panic!("missing reference value for {key}"))
}

/// `crate::match_snapshot::encode_snapshot`'s wire encoding for `marks`
/// walks `1..=player_count` and reads `marks.home[i]`/`marks.away[i]`
/// positionally — Rust `Vec` indexing, so a shorter (or, immediately after
/// `crate::r#match::new`, empty) `Vec` simply contributes fewer wire
/// entries. `sim/match_snapshot.lua`'s original walks the same range over a
/// plain Lua table, where an out-of-range read is just `nil` — so the same
/// short/empty `marks` there still contributes one wire entry per player,
/// each `nil`. A `MatchState` fresh from `sim/match.lua`'s `match.new`
/// legitimately stays this short (`marks = { home = {}, away = {} }`,
/// unsized until the first marking-assignment pass; `home` specifically
/// stays empty for the entire multi-second kickoff hold this fixture's
/// scripted scenario runs inside, confirmed against a real trace). This is
/// therefore not a simulation difference this Rust implementation needs to
/// reproduce, only a fixed-length-`Vec`-versus-sparse-table representational
/// gap this
/// harness closes itself: pad both sides out to `player_count` with `None`
/// before every hash, so the wire bytes agree with what Lua's positional
/// read already produces.
fn pad_marks(snapshot: &mut match_snapshot::MatchSnapshot) {
    let player_count = snapshot.state.players.len();
    snapshot.state.marks.home.resize(player_count, None);
    snapshot.state.marks.away.resize(player_count, None);
}

fn initial_snapshot() -> match_snapshot::MatchSnapshot {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(4.0),
        max_goals: Some(3),
        seed: Some(70.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    // See `tests/rollback_session.rs`'s module doc comment: `capture_owned`,
    // not `capture` — a never-stepped fixture's `marks` are legitimately
    // empty, matching `sim/match.lua` exactly.
    let mut snapshot = match_snapshot::capture_owned(&state, None);
    pad_marks(&mut snapshot);
    snapshot
}

fn sources() -> [gc_sim::rollback_input_history::RollbackInputSource; 8] {
    [gc_sim::rollback_input_history::RollbackInputSource::Remote; 8]
}

fn boundary_hash(session: &RollbackSession, boundary: i64) -> String {
    let lookup = rollback_session::snapshot(session, boundary);
    let mut snapshot = lookup
        .snapshot
        .unwrap_or_else(|| panic!("boundary {boundary} is not retained"));
    pad_marks(&mut snapshot);
    match_snapshot::hash_canonical(&snapshot)
}

#[test]
fn rollback_session_matches_lua_reference_across_predicted_and_corrected_ticks() {
    let reference = reference();
    let initial = initial_snapshot();
    assert_eq!(
        match_snapshot::hash_canonical(&initial),
        expect(&reference, "initial_hash")
    );

    let mut session = rollback_session::new(&initial, sources(), None, None);

    // Ticks 0..=9: authoritative-before-simulated neutral input.
    for tick in 0..=9i64 {
        for slot in 1..=input_frame::SLOT_COUNT {
            rollback_session::add_authoritative(
                &mut session,
                tick,
                slot,
                input_frame::neutral_sample(),
            )
            .expect("authoritative row accepted");
        }
        let output = rollback_session::step(&mut session).expect("step succeeds");
        assert_eq!(
            boundary_hash(&session, output.end_boundary),
            expect(&reference, &format!("pre.tick.{tick}")),
            "tick {tick}"
        );
    }

    // Ticks 10..=19: predicted only.
    for tick in 10..=19i64 {
        let output = rollback_session::step(&mut session).expect("step succeeds");
        assert_eq!(
            boundary_hash(&session, output.end_boundary),
            expect(&reference, &format!("pre.tick.{tick}")),
            "tick {tick}"
        );
    }

    // A late but in-window correction at tick 15, slot 2.
    let corrected = input_frame::new_sample(InputSampleOptions {
        move_x: Some(50),
        move_y: Some(-30),
        ..Default::default()
    })
    .expect("valid sample");
    let arrival = rollback_session::add_authoritative(&mut session, 15, 2, corrected)
        .expect("authoritative row accepted");
    assert_eq!(
        arrival.duplicate.to_string(),
        expect(&reference, "correction.duplicate")
    );
    assert_eq!(
        arrival
            .earliest_divergence
            .map_or("nil".to_string(), |t| t.to_string()),
        expect(&reference, "correction.earliest_divergence")
    );
    for slot in 1..=input_frame::SLOT_COUNT {
        if slot != 2 {
            rollback_session::add_authoritative(
                &mut session,
                15,
                slot,
                input_frame::neutral_sample(),
            )
            .expect("authoritative row accepted");
        }
    }

    let result = rollback_session::reconcile(&mut session, true);
    assert_eq!(
        result.changed.to_string(),
        expect(&reference, "reconcile.changed")
    );
    assert_eq!(
        result
            .causal_tick
            .map_or("nil".to_string(), |t| t.to_string()),
        expect(&reference, "reconcile.causal_tick")
    );
    assert_eq!(
        result.old_present_boundary.to_string(),
        expect(&reference, "reconcile.old_present_boundary")
    );
    assert_eq!(
        result.new_present_boundary.to_string(),
        expect(&reference, "reconcile.new_present_boundary")
    );
    assert_eq!(
        result
            .old_present_hash
            .clone()
            .unwrap_or_else(|| "nil".to_string()),
        expect(&reference, "reconcile.old_present_hash")
    );
    assert_eq!(
        result
            .new_present_hash
            .clone()
            .unwrap_or_else(|| "nil".to_string()),
        expect(&reference, "reconcile.new_present_hash")
    );

    for output in &result.corrected_outputs {
        assert_eq!(
            boundary_hash(&session, output.end_boundary),
            expect(&reference, &format!("post.tick.{}", output.tick)),
            "corrected tick {}",
            output.tick
        );
    }

    // Ticks 20..=24: forward, predicted, after the correction.
    for tick in 20..=24i64 {
        let output = rollback_session::step(&mut session).expect("step succeeds");
        assert_eq!(
            boundary_hash(&session, output.end_boundary),
            expect(&reference, &format!("post.tick.{tick}")),
            "post tick {tick}"
        );
    }

    let diagnostics = rollback_session::diagnostics(&session);
    assert_eq!(
        diagnostics.present_boundary.to_string(),
        expect(&reference, "final.present_boundary")
    );
    assert_eq!(
        diagnostics.rollback_count.to_string(),
        expect(&reference, "final.rollback_count")
    );
    assert_eq!(
        diagnostics.resimulated_ticks.to_string(),
        expect(&reference, "final.resimulated_ticks")
    );
    assert_eq!(
        diagnostics.max_rollback_depth.to_string(),
        expect(&reference, "final.max_rollback_depth")
    );
}
