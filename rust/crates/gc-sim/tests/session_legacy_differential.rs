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
//! FIVE ASSERTIONS, AND ONLY ONE OF THEM MOVES.
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
//!   5. `every_wire_field_varies_across_the_corpus` -- the META-assertion:
//!      no field this file claims to cover may be constant across the whole
//!      corpus, because a constant field cannot catch a defect in itself.
//!      IMMUNE. It exists because three review rounds each found a different
//!      field sitting at a constant with an oracle derived from the code
//!      under test; fixing those one at a time was losing, so the gate now
//!      names the offending field itself.
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
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize};
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

/// Every button state one tick of one slot can carry, as plain booleans.
///
/// The corpus is defined in terms of THIS, not in terms of bitmasks, and it
/// is the reason the oracle can be independent: [`sample_from`] turns these
/// booleans into wire bitmasks through `HeldAction::bit`/`EdgeAction::bit`,
/// while [`oracle_from`] turns the SAME booleans into a `MatchInput` without
/// touching `is_held`, `has_edge` or `to_match_input`. Two code paths, one
/// set of facts -- so a decode defect makes them disagree.
#[derive(Clone, Copy, Debug)]
struct Buttons {
    shoot_held: bool,
    pass_held: bool,
    sprint: bool,
    jockey: bool,
    lob: bool,
    aerial_strike: bool,
    aerial_acrobatic: bool,
    equipment_held: bool,
    shoot: bool,
    pass: bool,
    switch: bool,
    dash: bool,
    dodge: bool,
    equipment_pressed: bool,
    equipment_released: bool,
}

/// The five `(equipment_held, pressed, released)` combinations
/// `input_frame::validate_sample` accepts.
///
/// It rejects `pressed && !released && !held` and `released && held`, so a
/// naive per-bit pattern would build frames that fail validation. Cycling
/// these keeps all three equipment fields varying while staying valid.
const EQUIPMENT_COMBOS: [(bool, bool, bool); 5] = [
    (false, false, false),
    (true, false, false),
    (false, true, true),
    (true, true, false),
    (false, false, true),
];

/// The button state for one slot on one tick.
///
/// Every field varies across the corpus -- asserted, not asserted-by-hand,
/// by `every_wire_field_varies_across_the_corpus` below.
fn buttons_for(slot: usize, tick: i64) -> Buttons {
    let k = slot as i64 * 13 + tick * 7;
    let bit = |i: u32| (k >> i) & 1 == 1;
    let (equipment_held, equipment_pressed, equipment_released) =
        EQUIPMENT_COMBOS[(slot + tick.rem_euclid(5) as usize) % EQUIPMENT_COMBOS.len()];
    Buttons {
        shoot_held: bit(0),
        pass_held: bit(1),
        sprint: bit(2),
        jockey: bit(3),
        lob: bit(4),
        aerial_strike: bit(5),
        aerial_acrobatic: bit(6),
        equipment_held,
        shoot: bit(7),
        pass: bit(8),
        switch: bit(9),
        dash: bit(10),
        dodge: bit(11),
        equipment_pressed,
        equipment_released,
    }
}

/// Packs [`Buttons`] into the wire sample's two bitmasks.
fn sample_masks(b: &Buttons) -> (i64, i64) {
    use input_frame::{EdgeAction, HeldAction};
    let mut held = 0;
    let mut set_held = |on: bool, action: HeldAction| {
        if on {
            held |= action.bit();
        }
    };
    set_held(b.shoot_held, HeldAction::Shoot);
    set_held(b.pass_held, HeldAction::Pass);
    set_held(b.sprint, HeldAction::Sprint);
    set_held(b.jockey, HeldAction::Jockey);
    set_held(b.lob, HeldAction::Lob);
    set_held(b.aerial_strike, HeldAction::AerialStrike);
    set_held(b.aerial_acrobatic, HeldAction::AerialAcrobatic);
    set_held(b.equipment_held, HeldAction::Equipment);

    let mut edges = 0;
    let mut set_edge = |on: bool, action: EdgeAction| {
        if on {
            edges |= action.bit();
        }
    };
    set_edge(b.shoot, EdgeAction::Shoot);
    set_edge(b.pass, EdgeAction::Pass);
    set_edge(b.switch, EdgeAction::Switch);
    set_edge(b.dash, EdgeAction::Dash);
    set_edge(b.dodge, EdgeAction::Dodge);
    set_edge(b.equipment_pressed, EdgeAction::EquipmentPressed);
    set_edge(b.equipment_released, EdgeAction::EquipmentReleased);

    (held, edges)
}

/// The quantized move axes for one slot on one tick.
///
/// `slot * 31` and `slot * 47` offsets differ across every pair of slots at
/// every tick, so slot 0's sample differs from every other slot's on every
/// tick, not merely on average -- which is what makes a slot-selection
/// mutation detectable.
fn axes_for(slot: usize, tick: i64) -> (i64, i64) {
    let slot = slot as i64;
    (
        (slot * 31 + tick * 7).rem_euclid(255) - 127,
        (slot * 47 + tick * 13).rem_euclid(255) - 127,
    )
}

/// A per-slot, per-tick sample: no two slots are ever equal, slot 0's move
/// axes are never both zero, and **every button bit varies**.
///
/// All three properties are load-bearing, and each was added only after a
/// reviewer proved its absence made an assertion vacuous. The original
/// version drove the wire with `input_frame::new(tick, None)` -- eight
/// identical `InputSample::default()`s -- so a wrong slot index selected an
/// identical value and a changed dequantization scale multiplied zero. The
/// version after that varied only the axes, leaving `held`/`edges` at zero,
/// so a stuck-bit decode defect (`is_held` returning `true` for `Sprint`
/// unconditionally -- every player permanently sprinting) also passed. All
/// three mutations now fail.
fn distinctive_sample(slot: usize, tick: i64) -> input_frame::InputSample {
    let (move_x, move_y) = axes_for(slot, tick);
    let (held, edges) = sample_masks(&buttons_for(slot, tick));
    input_frame::InputSample {
        move_x,
        move_y,
        held,
        edges,
    }
}

/// The `MatchInput` the wire is expected to produce for one slot on one tick,
/// derived WITHOUT the code under test.
///
/// Nothing here calls `dequantize_axis`, `is_held`, `has_edge` or
/// `to_match_input`. The field mapping is transcribed from
/// `slot_input::to_match_input`'s documented contract rather than delegated
/// to it, because an oracle that calls the implementation is not an oracle --
/// a lesson this file learned twice (see the `LOCAL_SLOT_ZERO_INDEX` note
/// below).
fn oracle_from(slot: usize, tick: i64) -> MatchInput {
    let b = buttons_for(slot, tick);
    let (move_x, move_y) = axes_for(slot, tick);
    MatchInput {
        r#move: Vec2::new(
            move_x as f64 / ORACLE_MOVE_SCALE,
            move_y as f64 / ORACLE_MOVE_SCALE,
        ),
        shoot: b.shoot,
        shoot_held: b.shoot_held,
        pass: b.pass,
        pass_held: b.pass_held,
        switch: b.switch,
        dash: b.dash,
        dodge: b.dodge,
        lob: b.lob,
        sprint: b.sprint,
        jockey: b.jockey,
        aerial_strike: Some(b.aerial_strike),
        aerial_acrobatic: Some(b.aerial_acrobatic),
        equipment_held: b.equipment_held,
        equipment_pressed: b.equipment_pressed,
        equipment_released: b.equipment_released,
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

        // Oracle side: the same slot-0 input, specified without the wire,
        // without `dequantize_axis`, and without `is_held`/`has_edge`.
        //
        // Literal 0, deliberately NOT `LOCAL_SLOT_ZERO_INDEX`: the oracle must
        // assert which slot the local player's input comes from, not inherit
        // that answer from the code under test. Using the constant here made a
        // 0 -> 3 mutation of it move both sides together and the assertion
        // stayed green -- verified, and the reason this line is spelled this
        // way.
        let oracle = oracle_from(0, tick - 1);

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

/// Fields that are allowed to be constant across the corpus, each with a
/// written reason.
///
/// Empty, deliberately. An entry here is a DISCLOSED gap, not a silenced
/// one: it says "this field is not covered by these assertions and here is
/// why", which a reviewer can weigh. Adding one to make the meta-assertion
/// below go green, without a reason that survives being read out loud,
/// re-creates the exact defect that meta-assertion exists to prevent.
const ALLOWED_CONSTANT_FIELDS: &[(&str, &str)] = &[];

/// Tracks whether one named field ever took a second value.
#[derive(Default)]
struct Variation {
    first: Option<u64>,
    varied: bool,
}

impl Variation {
    fn observe(&mut self, value: u64) {
        match self.first {
            None => self.first = Some(value),
            Some(seen) if seen != value => self.varied = true,
            Some(_) => {}
        }
    }
}

/// Every field of a wire [`input_frame::InputSample`], flattened to
/// `(name, value)` -- masks split PER BIT, because a `held` mask that varies
/// as a whole can still hold one particular button constant, which is
/// precisely the defect this catches.
fn sample_field_values(sample: &input_frame::InputSample) -> Vec<(&'static str, u64)> {
    use input_frame::{EdgeAction, HeldAction};
    let held_bit = |a: HeldAction| u64::from(sample.held & a.bit() != 0);
    let edge_bit = |a: EdgeAction| u64::from(sample.edges & a.bit() != 0);
    vec![
        ("sample.move_x", sample.move_x as u64),
        ("sample.move_y", sample.move_y as u64),
        ("sample.held.shoot", held_bit(HeldAction::Shoot)),
        ("sample.held.pass", held_bit(HeldAction::Pass)),
        ("sample.held.sprint", held_bit(HeldAction::Sprint)),
        ("sample.held.jockey", held_bit(HeldAction::Jockey)),
        ("sample.held.lob", held_bit(HeldAction::Lob)),
        (
            "sample.held.aerial_strike",
            held_bit(HeldAction::AerialStrike),
        ),
        (
            "sample.held.aerial_acrobatic",
            held_bit(HeldAction::AerialAcrobatic),
        ),
        ("sample.held.equipment", held_bit(HeldAction::Equipment)),
        ("sample.edges.shoot", edge_bit(EdgeAction::Shoot)),
        ("sample.edges.pass", edge_bit(EdgeAction::Pass)),
        ("sample.edges.switch", edge_bit(EdgeAction::Switch)),
        ("sample.edges.dash", edge_bit(EdgeAction::Dash)),
        ("sample.edges.dodge", edge_bit(EdgeAction::Dodge)),
        (
            "sample.edges.equipment_pressed",
            edge_bit(EdgeAction::EquipmentPressed),
        ),
        (
            "sample.edges.equipment_released",
            edge_bit(EdgeAction::EquipmentReleased),
        ),
    ]
}

/// Every field of a decoded [`MatchInput`], flattened to `(name, value)`.
fn match_input_field_values(input: &MatchInput) -> Vec<(&'static str, u64)> {
    vec![
        ("input.move.x", input.r#move.x.to_bits()),
        ("input.move.y", input.r#move.y.to_bits()),
        ("input.shoot", u64::from(input.shoot)),
        ("input.shoot_held", u64::from(input.shoot_held)),
        ("input.pass", u64::from(input.pass)),
        ("input.pass_held", u64::from(input.pass_held)),
        ("input.switch", u64::from(input.switch)),
        ("input.dash", u64::from(input.dash)),
        ("input.dodge", u64::from(input.dodge)),
        ("input.lob", u64::from(input.lob)),
        ("input.sprint", u64::from(input.sprint)),
        ("input.jockey", u64::from(input.jockey)),
        (
            "input.aerial_strike",
            u64::from(input.aerial_strike.unwrap_or(false)),
        ),
        (
            "input.aerial_acrobatic",
            u64::from(input.aerial_acrobatic.unwrap_or(false)),
        ),
        ("input.equipment_held", u64::from(input.equipment_held)),
        (
            "input.equipment_pressed",
            u64::from(input.equipment_pressed),
        ),
        (
            "input.equipment_released",
            u64::from(input.equipment_released),
        ),
    ]
}

/// **The meta-assertion.** Every wire field this file claims to cover must
/// actually take more than one value across the corpus the assertions drive.
///
/// WHY THIS EXISTS. Three separate review rounds found the same defect in
/// this file: a field or constant that never varied, with an oracle quietly
/// derived from the implementation, so an assertion that named the field
/// asserted nothing about it. First the move axes (all-neutral frames), then
/// the slot index (an oracle reading `LOCAL_SLOT_ZERO_INDEX`), then the
/// button bitmasks (`held: 0, edges: 0` with the oracle calling
/// `to_match_input`). Each was fixed one dimension at a time, and each fix
/// left the next instance in place.
///
/// Patching dimensions one at a time loses. A constant field cannot catch a
/// defect in itself, so this test refuses to pass while the corpus claims
/// coverage it does not have -- naming the offending field. That turns
/// "a reviewer noticed" into "the gate notices", per AGENTS.md §9's rule that
/// every gate needs a demonstration it can go red.
#[test]
fn every_wire_field_varies_across_the_corpus() {
    use indexmap::IndexMap;

    let mut fields: IndexMap<&'static str, Variation> = IndexMap::new();
    let observe = |values: Vec<(&'static str, u64)>,
                   fields: &mut IndexMap<&'static str, Variation>| {
        for (name, value) in values {
            fields.entry(name).or_default().observe(value);
        }
    };

    for tick in 0..=TICKS {
        // The wire samples every slot carries, exactly as the round-trip and
        // transparency assertions build them.
        for slot in 0..8 {
            observe(
                sample_field_values(&distinctive_sample(slot, tick)),
                &mut fields,
            );
        }
        // The dequantized input the transparency assertion actually compares.
        observe(match_input_field_values(&oracle_from(0, tick)), &mut fields);
    }

    let allowed: Vec<&str> = ALLOWED_CONSTANT_FIELDS.iter().map(|(n, _)| *n).collect();
    let constant: Vec<&str> = fields
        .iter()
        .filter(|(name, v)| !v.varied && !allowed.contains(*name))
        .map(|(name, _)| *name)
        .collect();

    assert!(
        constant.is_empty(),
        "these wire fields never change across the whole corpus, so every assertion in this \
         file that names them is vacuous for them -- vary them in `buttons_for`/`axes_for`, or \
         add an explicit `ALLOWED_CONSTANT_FIELDS` entry with a reason: {constant:?}"
    );

    // A stale allowlist entry is its own silent gap: it would keep excusing a
    // field that has since started varying, and hide the next regression to
    // constant.
    for (name, reason) in ALLOWED_CONSTANT_FIELDS {
        let v = fields
            .get(name)
            .unwrap_or_else(|| panic!("allowlisted field {name} is not in the corpus at all"));
        assert!(
            !v.varied,
            "allowlisted field {name} DOES vary now ({reason}) -- drop the entry"
        );
    }

    // Guards the guard: if the corpus stopped covering these field sets, the
    // loop above would trivially find nothing constant.
    assert_eq!(
        fields.len(),
        34,
        "corpus must cover all 17 + 17 named fields"
    );
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
