//! # The retirement (#520)
//!
//! Until #520 this asserted against
//! `fixtures/rollback_session_lua_reference.txt`, captured from the original
//! Lua implementation, so a pass was **cross-implementation** evidence. That
//! claim is retired: decision #520 (repository owner), superseded by #516 —
//! the locomotion rework for #488, which changes what every body does per
//! tick and so moves every boundary hash below by construction. The Lua
//! vector last held at `3f8f4a3`, verified green there by running this file
//! at that commit rather than assuming it. The vector file stays in the tree,
//! unmodified and unread.
//!
//! The baseline read now was recorded from this build. It DETECTS CHANGE but
//! CANNOT DETECT "wrong but consistently wrong" — a defect present when it
//! was recorded is in the expectations forever. Nothing in the workspace
//! replaces the lost cross-implementation coverage; there is no second
//! implementation left to disagree with.
//!
//! Per `tools/lua_reference/README.md` rule 5, the claim this file actually
//! exists for was moved out of the frozen part.
//! `..._resimulation_reaches_what_direct_simulation_reaches` states it
//! without any record at all: a rollback that resimulates ticks 15..19 after
//! a late correction must land on exactly the state a session that never
//! mispredicted lands on. That is what rollback IS, it is immune to gameplay
//! change because both sides move together, and it keeps gating through every
//! rework with nothing to re-record. Broad reconvergence over many scenarios
//! and network profiles is `rollback_validation`'s 66-case native matrix;
//! this is the single scripted case that names its own arithmetic.
//!
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

const FIXTURE: &str = include_str!("fixtures/rollback_session_baseline.txt");

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
fn rollback_session_reproduces_its_recorded_baseline_across_predicted_and_corrected_ticks() {
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

/// The rollback correctness property, stated against a live twin instead of a
/// record — so no gameplay change can move it.
///
/// A session steps ticks 0..=15 with tick 15 slot 2 *missing*, so that slot
/// is predicted. The corrected sample for it then arrives late but in window,
/// and the session rolls back and resimulates exactly one tick. The twin
/// receives every input authoritatively before its tick is stepped, so it
/// never mispredicts and never rolls back. Both must reach the identical
/// present boundary hash.
///
/// If resimulation were not exact — a stale cached value surviving a restore,
/// an input applied at the wrong tick, an RNG stream not rewound — the two
/// part company, and the recorded baseline above would happily pass on
/// whichever of the two got recorded.
///
/// **Why exactly one resimulated tick, and no predicted ticks after the
/// correction.** An earlier draft mispredicted ticks 10..=19 and corrected
/// tick 15, then built the twin from neutral input for 16..=19. It failed,
/// and the failure was the test's fault, not the code's: once the correction
/// lands, the session's own prediction for slot 2 at ticks 16..=19 repeats
/// the CORRECTED sample, not the neutral one. Writing the twin to match would
/// have meant encoding `rollback_input_history`'s prediction policy into a
/// test that exists to check the code implementing it — an oracle derived
/// from the code under test, which `tools/lua_reference/README.md` calls out
/// by name. Ending the scenario at the corrected tick removes the question.
#[test]
fn rollback_session_resimulation_reaches_what_direct_simulation_reaches() {
    let corrected = input_frame::new_sample(InputSampleOptions {
        move_x: Some(50),
        move_y: Some(-30),
        ..Default::default()
    })
    .expect("valid sample");
    const LAST: i64 = 15;
    const CORRECTED_SLOT: i64 = 2;

    // A: every slot authoritative except slot 2 on the final tick, which is
    // predicted and then corrected.
    let mut rolled_back = rollback_session::new(&initial_snapshot(), sources(), None, None);
    for tick in 0..=LAST {
        for slot in 1..=input_frame::SLOT_COUNT {
            if tick == LAST && slot == CORRECTED_SLOT {
                continue;
            }
            rollback_session::add_authoritative(
                &mut rolled_back,
                tick,
                slot,
                input_frame::neutral_sample(),
            )
            .expect("authoritative row accepted");
        }
        rollback_session::step(&mut rolled_back).expect("step succeeds");
    }
    let arrival =
        rollback_session::add_authoritative(&mut rolled_back, LAST, CORRECTED_SLOT, corrected)
            .expect("authoritative row accepted");
    assert_eq!(
        arrival.earliest_divergence,
        Some(LAST),
        "the correction must actually contradict the prediction, or this proves nothing"
    );
    let result = rollback_session::reconcile(&mut rolled_back, true);
    assert!(result.changed, "the scenario must actually roll back");

    // B: the same inputs, all delivered before their tick is stepped.
    let mut direct = rollback_session::new(&initial_snapshot(), sources(), None, None);
    for tick in 0..=LAST {
        for slot in 1..=input_frame::SLOT_COUNT {
            let sample = if tick == LAST && slot == CORRECTED_SLOT {
                corrected
            } else {
                input_frame::neutral_sample()
            };
            rollback_session::add_authoritative(&mut direct, tick, slot, sample)
                .expect("authoritative row accepted");
        }
        rollback_session::step(&mut direct).expect("step succeeds");
    }
    assert_eq!(
        rollback_session::diagnostics(&direct).rollback_count,
        0,
        "the twin must never roll back, or it is not an independent check"
    );

    let a = rollback_session::diagnostics(&rolled_back).present_boundary;
    let b = rollback_session::diagnostics(&direct).present_boundary;
    assert_eq!(
        a, b,
        "the two sessions reached different present boundaries"
    );
    assert_eq!(
        boundary_hash(&rolled_back, a),
        boundary_hash(&direct, b),
        "resimulating after a correction did not reach the state that never mispredicted \
         it -- rollback is not exact"
    );
}

/// Records the baseline. `#[ignore]`d and printing to stdout, for the reason
/// every recorder in this tree is: one that overwrote its own fixture during
/// `cargo test` would turn a determinism regression into a no-op.
///
/// **Re-recording is a decision, not a fix.** Re-record only when the change
/// is deliberate, reviewed and named in the commit message.
///
/// Run:
///
/// ```text
/// cd rust
/// cargo test -p gc-sim --test rollback_session_differential -- \
///     --ignored --nocapture record_rollback_session_baseline \
///   | grep -E '^[a-z_]+[a-z_.0-9]*=' \
///   > crates/gc-sim/tests/fixtures/rollback_session_baseline.txt
/// ```
#[test]
#[ignore = "recorder: prints a baseline for a human to capture, never asserts"]
fn record_rollback_session_baseline() {
    let initial = initial_snapshot();
    println!("initial_hash={}", match_snapshot::hash_canonical(&initial));
    let mut session = rollback_session::new(&initial, sources(), None, None);
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
        println!(
            "pre.tick.{tick}={}",
            boundary_hash(&session, output.end_boundary)
        );
    }
    for tick in 10..=19i64 {
        let output = rollback_session::step(&mut session).expect("step succeeds");
        println!(
            "pre.tick.{tick}={}",
            boundary_hash(&session, output.end_boundary)
        );
    }
    let corrected = input_frame::new_sample(InputSampleOptions {
        move_x: Some(50),
        move_y: Some(-30),
        ..Default::default()
    })
    .expect("valid sample");
    let arrival = rollback_session::add_authoritative(&mut session, 15, 2, corrected)
        .expect("authoritative row accepted");
    println!("correction.duplicate={}", arrival.duplicate);
    println!(
        "correction.earliest_divergence={}",
        arrival
            .earliest_divergence
            .map_or("nil".to_string(), |t| t.to_string())
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
    println!("reconcile.changed={}", result.changed);
    println!(
        "reconcile.causal_tick={}",
        result
            .causal_tick
            .map_or("nil".to_string(), |t| t.to_string())
    );
    println!(
        "reconcile.old_present_boundary={}",
        result.old_present_boundary
    );
    println!(
        "reconcile.new_present_boundary={}",
        result.new_present_boundary
    );
    println!(
        "reconcile.old_present_hash={}",
        result
            .old_present_hash
            .clone()
            .unwrap_or_else(|| "nil".to_string())
    );
    println!(
        "reconcile.new_present_hash={}",
        result
            .new_present_hash
            .clone()
            .unwrap_or_else(|| "nil".to_string())
    );
    for output in &result.corrected_outputs {
        println!(
            "post.tick.{}={}",
            output.tick,
            boundary_hash(&session, output.end_boundary)
        );
    }
    for tick in 20..=24i64 {
        let output = rollback_session::step(&mut session).expect("step succeeds");
        println!(
            "post.tick.{tick}={}",
            boundary_hash(&session, output.end_boundary)
        );
    }
    let d = rollback_session::diagnostics(&session);
    println!("final.present_boundary={}", d.present_boundary);
    println!("final.rollback_count={}", d.rollback_count);
    println!("final.resimulated_ticks={}", d.resimulated_ticks);
    println!("final.max_rollback_depth={}", d.max_rollback_depth);
}
