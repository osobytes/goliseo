//! Determinism regression for the ACTUAL path `crates/gc-wasm/src/session.rs`'s
//! `Session` runs for an ordinary (non-rollback, non-online) browser match,
//! against a baseline recorded from THIS implementation.
//!
//! WHAT THIS PROVES.
//!
//! `Session::step` does not hand a `MatchInput` to `match::step`. It encodes a
//! canonical `gc_sim::input_frame` wire string, decodes it, validates it, and
//! dequantizes the single `home_1` sample back into a `MatchInput` via
//! `gc_sim::slot_input::to_match_input`, exactly as documented in
//! `crates/gc-wasm/src/session.rs`'s module doc ("An ordinary session is
//! LEGACY mode"). That wire round trip
//! (`input_frame::encode`/`::decode`/`::validate` plus
//! `slot_input::to_match_input`) is a real, additional layer that
//! `match_differential.rs` -- which steps
//! `StepInput::Legacy(MatchInput::default())` constructed directly in Rust,
//! with `human_controlled: Some(false)` so no player takes the human-input
//! branch at all -- does not exercise at ALL.
//!
//! THIS IS NOT THE ONLY TEST COVERING THAT LAYER TODAY, AND SAYING SO WOULD
//! BE FALSE. `session_ai_driven_differential.rs` drives the identical
//! pipeline over a full ordinary match with real bot-authored, non-neutral,
//! quantization-lossy input, and it currently passes. An earlier draft of
//! this doc claimed uniqueness; a reviewer was right to reject it, and
//! `tools/lua_reference/README.md`'s own catalogue contradicted it in the
//! same change.
//!
//! What justifies keeping this file is narrower and more durable:
//! `session_ai_driven_differential.rs` asserts against a Lua-captured
//! BEHAVIORAL vector. It is listed in that README's retirement catalogue, and
//! it will diverge under #488/#489/#490/#491 exactly as this file's vector
//! did, at which point it faces this same decision. A test that must be
//! renegotiated every time the game changes is not a durable home for a wire
//! guarantee. The assertions below are built to survive deliberate gameplay
//! change instead -- so the honest claim is not "the only coverage today", it
//! is "the coverage that outlives the next four reworks".
//!
//! It reproduces `Session::new`/`Session::step`'s LIVE-state construction and
//! per-tick stepping using `gc-sim`'s own public API directly (this crate
//! cannot depend on `gc-wasm`, which depends on it -- see README.md's layer
//! rule), so it is a faithful stand-in for what the wasm binding actually
//! runs, minus the wasm-bindgen marshalling itself (which carries no
//! simulation logic of its own -- see `session.rs`'s `Session::step` body,
//! four lines of glue around exactly the calls this test makes).
//!
//! Every field is compared at every tick, and floats are compared by bit
//! pattern after parsing, not by printed text: a divergence that
//! self-corrects a tick later is still a desync.
//!
//! FOUR ASSERTIONS, AND ONLY ONE OF THEM MOVES.
//!
//! The split matters, because a frozen per-tick trajectory is broken by any
//! deliberate gameplay change BY CONSTRUCTION -- that is what happened to the
//! Lua vector, and re-recording the same shape of fixture in Rust would only
//! have made it regenerable, not immune. So the coverage that justifies this
//! file's existence was moved out of the frozen part:
//!
//!   1. `..._reproduces_its_recorded_baseline_...` -- the pinned trajectory.
//!      Detects unintended change. This is the one a deliberate gameplay
//!      rework legitimately trips, and the one it re-records (see
//!      `record_session_legacy_ordinary_baseline` below, and
//!      `tools/lua_reference/README.md`'s rule for behavioral vectors).
//!   2. `..._is_bit_reproducible_across_two_independent_runs` -- the same
//!      match stepped twice must agree bit-for-bit. IMMUNE to gameplay
//!      change: both runs move together.
//!   3. `..._agrees_with_an_independently_specified_non_neutral_input` -- the
//!      wire round trip is transparent, over a full match, under per-slot-
//!      distinct non-neutral input, against an oracle that never calls
//!      `dequantize_axis`. IMMUNE for the same reason. **This is the
//!      assertion that carries the durable coverage described above**, and it
//!      keeps gating through every rework in #488-#491 without anyone
//!      re-recording anything.
//!   4. `wire_round_trip_preserves_every_slot_at_its_own_index` -- encode ->
//!      decode -> validate returns all eight distinct slots unpermuted.
//!      IMMUNE; it never steps the simulation at all.
//!
//! So a gameplay rework re-records one fixture and loses no wire coverage,
//! and a quantization or slot-indexing defect still fails the gate even in a
//! PR that is legitimately re-recording the baseline.
//!
//! WHAT THIS NO LONGER PROVES, AND WHY THE EVIDENCE IS WEAKER.
//!
//! Until #500 this test asserted against
//! `fixtures/session_legacy_ordinary_lua_reference.txt`, captured from the
//! original Lua `sim/match.lua` -- so a pass was CROSS-IMPLEMENTATION
//! evidence: two independently written simulations agreed bit-for-bit, which
//! is the only kind of evidence that can catch the Rust port being wrong
//! rather than merely being stable. That claim is retired. This test no
//! longer says anything about Lua.
//!
//! The baseline it reads now was recorded from this very code (see
//! `record_session_legacy_ordinary_baseline` below). Self-recorded evidence is
//! strictly weaker, and the gap is not a technicality:
//!
//!   * It DETECTS CHANGE. If any edit anywhere under this path perturbs one
//!     float at one tick, this goes red -- which is exactly what a
//!     determinism guarantee needs, because an unintended perturbation is a
//!     desync between a client that shipped before the edit and one that
//!     shipped after.
//!   * It CANNOT DETECT "WRONG BUT CONSISTENTLY WRONG". A bug that was in the
//!     baseline when it was recorded is in the expectations forever, and this
//!     test will defend it. The Lua vector could have caught that class; this
//!     one cannot. Nothing else in the workspace replaces that coverage,
//!     because there is no second implementation left to disagree with.
//!
//! Assertions 2-4 are not weakened this way -- they compare live runs against
//! each other, or against an oracle stated independently of the code under
//! test, rather than against a record. But none is a substitute for the lost
//! coverage: they prove the wire layer is faithful and the path is
//! self-consistent, not that the simulation it feeds is right.
//!
//! Do not read a pass here as "the simulation is correct". Read it as "the
//! simulation is the same one we recorded, and the wire layer is not adding
//! anything of its own".
//!
//! PROVENANCE.
//!
//! Lua bit-parity for this scenario held, and was asserted on every PR,
//! through commit `9127c5c` -- verified green there immediately before the
//! conversion. It was retired by a decision of the repository owner recorded
//! in issue #500, superseded by the goalkeeper rework for #490, which
//! replaces the keeper's hand-rolled gravity-only extrapolation (the specific
//! defect #486 was written to eliminate) with a real forward-prediction
//! query. That fix changes the tick at which a catch draws from the RNG
//! stream, diverging the Lua vector at `tick 743`; once the stream diverges
//! at one tick, every later tick differs, so there is no way to annotate
//! individual intended divergences.
//!
//! `fixtures/session_legacy_ordinary_lua_reference.txt` is deliberately KEPT,
//! unmodified and unread, as the historical record of that capture. See
//! `tools/lua_reference/README.md` for the standing rule separating those
//! frozen BEHAVIORAL vectors from the ENCODING vectors, which are still
//! load-bearing and must never be refreshed.
//!
//! The file keeps its `_differential` name: #500, the sibling module docs
//! that cross-reference it, and the failing-run transcripts all name it, and
//! a rename would cost that thread more than the name is worth. What it is
//! now is stated above, not in the filename.

use gc_core::vec2::Vec2;
use gc_sim::input_frame;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchState, PitchSize};
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/session_legacy_ordinary_baseline.txt");

const TICKS: i64 = 7_200;
const DT: f64 = 1.0 / 60.0;
const MATCH_SEED: f64 = 5.0;
const PLAYER_COUNT: usize = 10;
/// Field count: 11 scalar fields (tick, 6 ball fields, owner, 2 scores, rng)
/// plus 2 (x, y) per player. Identical layout to `match_differential.rs`'s
/// own fixture, and to the retired Lua capture this baseline replaced, so
/// the two files stay diffable against each other.
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
/// is the layer this test exists to cover.
fn neutral_local_match_input(tick: i64) -> gc_sim::match_snapshot::MatchInput {
    let frame = input_frame::new(tick, None).expect("an all-neutral frame is always valid");
    let wire = input_frame::encode(&frame).expect("a valid frame always encodes");
    let decoded = input_frame::decode(&wire).expect("a wire this test just encoded always decodes");
    input_frame::validate(&decoded).expect("a freshly encoded neutral frame is always valid");
    slot_input::to_match_input(&local_slot(&decoded))
}

/// The canonical `home_1` slot index -- `LOCAL_SLOT_ZERO_INDEX` in
/// `crates/gc-wasm/src/session.rs`.
const LOCAL_SLOT_ZERO_INDEX: usize = 0;

/// Reads the local player's sample out of a decoded frame.
///
/// Deliberately the SINGLE place this file selects a slot, shared by the
/// neutral baseline path and by the non-degenerate transparency assertion
/// below. Selecting a slot in two places would let a slot-selection mutation
/// hide in whichever copy the degenerate scenario feeds; with one place, the
/// transparency assertion covers the whole file's slot handling.
fn local_slot(decoded: &input_frame::InputFrame) -> input_frame::InputSample {
    decoded.slots[LOCAL_SLOT_ZERO_INDEX]
}

/// The quantized axis scale, **deliberately duplicated** from
/// `input_frame::MOVE_SCALE` rather than imported.
///
/// This is the independent oracle for the wire contract. Importing the
/// constant would make the expectation move whenever the implementation moved,
/// which is precisely the failure the transparency assertion below exists to
/// catch: a change to the dequantization scale would then alter both sides
/// equally and the test would stay green while every browser client started
/// reading a different stick deflection off the same bytes.
const ORACLE_MOVE_SCALE: f64 = 127.0;

/// A per-slot, per-tick sample chosen so that **no two slots are ever equal**
/// and slot 0's move axes are **never both zero**.
///
/// Both properties are load-bearing. The previous version of the transparency
/// assertion drove the wire with `input_frame::new(tick, None)` -- eight
/// identical `InputSample::default()`s with zero axes -- which made it
/// vacuous for the two defect classes it advertised: reading the wrong slot
/// index selected an identical value, and any change to the dequantization
/// scale multiplied zero by something and got zero. Both mutations were
/// confirmed to leave that assertion green.
///
/// `slot * 31` and `slot * 47` offsets are coprime-ish with the 255 modulus
/// and differ across every pair of slots at every tick, so the sample for
/// slot 0 differs from the sample for every other slot on every tick, not
/// merely on average.
fn distinctive_sample(slot: usize, tick: i64) -> input_frame::InputSample {
    let slot = slot as i64;
    input_frame::InputSample {
        move_x: (slot * 31 + tick * 7).rem_euclid(255) - 127,
        move_y: (slot * 47 + tick * 13).rem_euclid(255) - 127,
        held: 0,
        edges: 0,
    }
}

/// All eight slots for one tick, each distinct.
fn distinctive_slots(tick: i64) -> [input_frame::InputSample; 8] {
    let mut slots = [input_frame::InputSample::default(); 8];
    for (i, slot) in slots.iter_mut().enumerate() {
        *slot = distinctive_sample(i, tick);
    }
    slots
}

/// Mirrors `crates/gc-wasm/src/session.rs`'s `Session::new` construction of
/// its LIVE state exactly: no `input_ownership` (legacy mode), no
/// `human_controlled` override (defaults `true`, so one player really does
/// take the human-input branch), teams nebula/orion, seed 5, a 960x540
/// field, and `match.step`'s default duration/goal limit (120 seconds, no
/// goal cap).
///
/// Shared by the assertion and the recorder below, so a baseline can never
/// be recorded from a scenario the assertion does not replay.
fn new_ordinary_session() -> MatchState {
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
        seed: Some(MATCH_SEED),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

/// Advances one tick exactly as `Session::step` does. Shared for the same
/// reason as [`new_ordinary_session`].
fn step_one_tick(state: &mut MatchState, tick: i64, tune: &Tuning) {
    let match_input = neutral_local_match_input(tick - 1);
    sim_match::step(state, DT, StepInput::Legacy(match_input), None, tune);
}

/// Runs the whole wire-driven match and returns one rendered row per tick.
///
/// Rows are compared as text in the two assertions below, which is a
/// bit-exact comparison and not an approximate one: [`baseline_row`] renders
/// floats with Rust's shortest round-trippable `Display`, and that mapping is
/// injective on `f64` bit patterns -- two values render identically only if
/// they ARE identical, `-0` and `0` included.
fn run_wire_driven_match() -> Vec<String> {
    let tune = Tuning::new();
    let mut s = new_ordinary_session();
    let mut rows = Vec::with_capacity(TICKS as usize + 1);
    rows.push(baseline_row(0, &s));
    for tick in 1..=TICKS {
        step_one_tick(&mut s, tick, &tune);
        rows.push(baseline_row(tick, &s));
    }
    rows
}

#[test]
fn wire_driven_session_legacy_step_reproduces_its_recorded_baseline_for_a_7200_tick_ordinary_match()
{
    let tune = Tuning::new();
    let mut s = new_ordinary_session();

    let rows: Vec<Row> = FIXTURE
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(parse_row)
        .collect();
    assert_eq!(
        rows.len() as i64,
        TICKS + 1,
        "fixture must cover tick 0 through tick {TICKS}"
    );

    let mut compared = 0;
    for row in &rows {
        if row.tick > 0 {
            step_one_tick(&mut s, row.tick, &tune);
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
    assert_eq!(i64::from(compared), TICKS + 1);
}

/// The browser wire path is DETERMINISTIC: the same ordinary match, stepped
/// twice through `input_frame::encode`/`decode`/`validate` and
/// `slot_input::to_match_input`, produces identical bits at every one of
/// 7,201 ticks.
///
/// Immune to deliberate gameplay change by construction -- both runs move
/// together -- so unlike the baseline above this never needs re-recording,
/// and it keeps gating through every rework in #488-#491. It is also the
/// assertion that still means something if a baseline is ever re-recorded
/// carelessly: a nondeterministic path cannot be frozen into a fixture at
/// all, and would fail here no matter what the fixture says.
#[test]
fn wire_driven_session_legacy_step_is_bit_reproducible_across_two_independent_runs() {
    let first = run_wire_driven_match();
    let second = run_wire_driven_match();
    assert_eq!(first.len() as i64, TICKS + 1);
    for (tick, (a, b)) in first.iter().zip(second.iter()).enumerate() {
        assert_eq!(
            a, b,
            "tick {tick}: two runs of the same wire-driven match diverged"
        );
    }
}

/// `encode` -> `decode` -> `validate` returns every slot unchanged AT ITS OWN
/// INDEX, for a frame whose eight slots are all different.
///
/// Sharp and cheap, and it localizes a failure the session-level assertion
/// below would only report as "the match diverged at tick 1": if slot order
/// is permuted or a field is dropped on the wire, this names the slot and the
/// field.
#[test]
fn wire_round_trip_preserves_every_slot_at_its_own_index() {
    for tick in [0, 1, 743, TICKS] {
        let slots = distinctive_slots(tick);
        let frame = input_frame::new(tick, Some(slots)).expect("a distinctive frame is valid");
        let wire = input_frame::encode(&frame).expect("a valid frame always encodes");
        let decoded = input_frame::decode(&wire).expect("a wire just encoded always decodes");
        input_frame::validate(&decoded).expect("a freshly encoded frame is always valid");

        assert_eq!(decoded.tick, tick, "the tick survives the round trip");
        for (i, expected) in slots.iter().enumerate() {
            assert_eq!(
                decoded.slots[i], *expected,
                "tick {tick} slot {i}: the wire round trip did not return this slot unchanged \
                 at its own index"
            );
        }
        // Guards the guard: if these ever stop differing, every slot-selection
        // assertion in this file silently becomes vacuous again.
        for i in 1..8 {
            assert_ne!(
                slots[0], slots[i],
                "tick {tick}: slot 0 and slot {i} must differ, or slot selection is untestable"
            );
        }
    }
}

/// The wire round trip is TRANSPARENT: driving the session through the full
/// encode -> decode -> validate -> dequantize pipeline gives bit-identical
/// results to a session driven by the SAME input specified independently, for
/// all 7,200 ticks -- under **non-neutral, per-slot-distinct** input.
///
/// This is the coverage that justified keeping this file when its Lua-parity
/// claim was retired (#500), and the non-degeneracy is what makes it real.
/// The expectation side never calls `dequantize_axis`: it computes the axis
/// from [`ORACLE_MOVE_SCALE`], a deliberately duplicated constant, so the two
/// sides agree only if the wire format still means what it meant. That closes
/// both holes a reviewer demonstrated in the neutral version of this test:
///
///   * reading the wrong slot index off the decoded frame now selects a
///     different sample, because [`distinctive_sample`] makes every slot
///     differ on every tick; and
///   * changing the dequantization scale now moves the wire side away from
///     the oracle, because the axes are non-zero.
///
/// Both mutations were re-run against this version and both fail it.
///
/// `match_differential.rs` takes the direct-construction side only and can
/// say nothing about the pipeline; `gc-wasm`'s `Session::step` takes the wire
/// side only. Immune to deliberate gameplay change: a sim change moves both
/// sides identically.
#[test]
fn wire_driven_session_legacy_step_agrees_with_an_independently_specified_non_neutral_input() {
    let tune = Tuning::new();
    let mut wire_driven = new_ordinary_session();
    let mut expected = new_ordinary_session();

    for tick in 1..=TICKS {
        let slots = distinctive_slots(tick - 1);

        // Wire side: exactly what `Session::step` does, but with a frame that
        // is not all-neutral and whose slots are not all identical.
        let frame = input_frame::new(tick - 1, Some(slots)).expect("a distinctive frame is valid");
        let wire = input_frame::encode(&frame).expect("a valid frame always encodes");
        let decoded = input_frame::decode(&wire).expect("a wire just encoded always decodes");
        input_frame::validate(&decoded).expect("a freshly encoded frame is always valid");
        let from_wire = slot_input::to_match_input(&local_slot(&decoded));

        // Oracle side: the same slot-0 input, specified without the wire and
        // without `dequantize_axis`.
        let mut oracle = slot_input::to_match_input(&input_frame::InputSample::default());
        // Literal 0, deliberately NOT `LOCAL_SLOT_ZERO_INDEX`: the oracle
        // must assert which slot the local player's input comes from, not
        // inherit that answer from the code under test. Using the constant
        // here made a 0 -> 3 mutation of it move both sides together and the
        // assertion stayed green -- verified, and the reason this line is
        // spelled this way.
        let local = distinctive_sample(0, tick - 1);
        oracle.r#move = Vec2::new(
            local.move_x as f64 / ORACLE_MOVE_SCALE,
            local.move_y as f64 / ORACLE_MOVE_SCALE,
        );

        // Assert the DEQUANTIZED INPUT, not merely the state it produces.
        //
        // `match::step` normalizes `input.move` before using it, so a uniform
        // change to the dequantization scale is invisible in the resulting
        // simulation state -- halving `dequantize_axis` leaves the direction,
        // and therefore every downstream float, identical. Verified: with that
        // mutation live, the session comparison below stays green and only
        // this assertion catches it. A wire contract has to be checked at the
        // boundary; it cannot be inferred from sim state on the far side of a
        // normalization.
        assert_eq!(
            from_wire, oracle,
            "tick {tick}: the wire round trip did not reproduce the local slot's input"
        );
        assert_eq!(
            (from_wire.r#move.x.to_bits(), from_wire.r#move.y.to_bits()),
            (oracle.r#move.x.to_bits(), oracle.r#move.y.to_bits()),
            "tick {tick}: dequantized move axes differ in bit pattern (PartialEq would let \
             -0.0 pass for 0.0 here)"
        );

        sim_match::step(
            &mut wire_driven,
            DT,
            StepInput::Legacy(from_wire),
            None,
            &tune,
        );
        sim_match::step(&mut expected, DT, StepInput::Legacy(oracle), None, &tune);

        assert_eq!(
            baseline_row(tick, &wire_driven),
            baseline_row(tick, &expected),
            "tick {tick}: the wire round trip is not transparent -- a session driven through \
             input_frame encode/decode/validate and slot_input::to_match_input diverged from \
             one given the same slot-0 input specified independently of the wire"
        );
    }
}

/// Renders one baseline row in the fixture's tab-separated layout.
///
/// Floats use Rust's shortest round-trippable `Display` form, which -- unlike
/// a fixed decimal precision -- is guaranteed to parse back to the exact same
/// `f64` bit pattern, so the bit-exact comparison above stays sound across
/// the text round trip. Same rationale, and same choice, as
/// `gc_sim::outfield_ai_baseline::serialize`. (The retired Lua capture used
/// `%.17g` for the same reason: Lua had no shortest-round-trip formatter.)
fn baseline_row(tick: i64, s: &MatchState) -> String {
    let mut fields = vec![
        tick.to_string(),
        s.ball.x.to_string(),
        s.ball.y.to_string(),
        s.ball_vel.x.to_string(),
        s.ball_vel.y.to_string(),
        s.ball_z.to_string(),
        s.ball_vz.to_string(),
        s.owner.unwrap_or(-1).to_string(),
        s.score.home.to_string(),
        s.score.away.to_string(),
        s.rng.to_string(),
    ];
    for player in s.players.iter().take(PLAYER_COUNT) {
        fields.push(player.pos.x.to_string());
        fields.push(player.pos.y.to_string());
    }
    fields.join("\t")
}

/// Re-records `fixtures/session_legacy_ordinary_baseline.txt` from this
/// build.
///
/// NOT a CI-asserted test, and `#[ignore]`d so it never runs in the gate: it
/// PRINTS the baseline to stdout for a human to capture into the committed
/// fixture, the same workflow as `input_sample_vector_generator.rs` beside
/// it. A recorder that overwrote the fixture on every `cargo test` would
/// turn a determinism regression into a no-op.
///
/// **Re-recording is a decision, not a fix.** A red assertion above is a
/// FINDING first: something under the wire-driven ordinary-match path
/// changed. Re-record only when that change is deliberate, reviewed, and
/// named in the commit message -- exactly the rule
/// `tools/lua_reference/README.md` states for behavioral vectors. Refreshing
/// to clear a check you did not intend to trip deletes the only evidence
/// that the path is stable.
///
/// Run:
///
/// ```text
/// cd rust
/// cargo test -p gc-sim --test session_legacy_differential -- \
///     --ignored --nocapture record_session_legacy_ordinary_baseline \
///   | sed -n '/^# session_legacy_ordinary_baseline/,$p' \
///   | grep -E '^(#|[0-9])' \
///   > crates/gc-sim/tests/fixtures/session_legacy_ordinary_baseline.txt
/// ```
///
/// The `grep` drops libtest's own trailing status lines; every header line
/// starts with `#` and every data line with a digit, so nothing of the
/// baseline is filtered.
#[test]
#[ignore]
fn record_session_legacy_ordinary_baseline() {
    let tune = Tuning::new();
    let mut s = new_ordinary_session();

    println!("# session_legacy_ordinary_baseline");
    println!(
        "# Recorded from THIS Rust implementation by \
         `record_session_legacy_ordinary_baseline` in"
    );
    println!("# crates/gc-sim/tests/session_legacy_differential.rs -- see that file's module doc.");
    println!("#");
    println!(
        "# This is a self-recorded determinism baseline, NOT cross-implementation \
         evidence. It"
    );
    println!("# detects change; it cannot detect \"wrong but consistently wrong\".");
    println!("#");
    println!(
        "# It replaces the Lua-captured vector in \
         session_legacy_ordinary_lua_reference.txt, which"
    );
    println!(
        "# is kept unmodified beside it as a historical artifact. Lua bit-parity for this \
         scenario"
    );
    println!(
        "# held through commit 9127c5c and was retired by the decision in issue #500, \
         superseded"
    );
    println!("# by the goalkeeper forward-prediction rework for #490.");
    println!("#");
    println!(
        "# Scenario: teams nebula/orion, seed {MATCH_SEED}, 960x540 field, default \
         duration/goal-limit,"
    );
    println!(
        "# no input_ownership, no human_controlled override, an all-neutral canonical input \
         frame"
    );
    println!("# encoded/decoded/validated/dequantized every tick, {TICKS} ticks.");
    println!(
        "# Fields (tab-separated): tick, ball.x, ball.y, ball_vel.x, ball_vel.y, ball_z, \
         ball_vz,"
    );
    println!(
        "# owner (-1 when none), score.home, score.away, rng, then pos.x/pos.y for each of \
         the {PLAYER_COUNT} players."
    );
    println!("{}", baseline_row(0, &s));
    for tick in 1..=TICKS {
        step_one_tick(&mut s, tick, &tune);
        println!("{}", baseline_row(tick, &s));
    }
}
