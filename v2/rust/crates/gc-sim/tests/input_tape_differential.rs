//! Differential test of `input_tape` against the real Lua implementation
//! (README rule 5.9, `v2/tools/lua_reference/README.md`). `sim/input_tape.lua`
//! has no dedicated spec file — it is exercised through
//! `spec/sim/headless_spec.lua`, `replay_spec.lua`, and
//! `determinism_evidence_spec.lua` — but it is squarely on the determinism
//! path (it encodes/decodes recorded inputs and produces the boundary
//! hashes replay and rollback both trust), so it gets this instead: a
//! from-scratch construction of `input_tape.new`, reproduced identically in
//! both languages, checked byte for byte.
//!
//! `tests/fixtures/input_tape_lua_reference.txt` is the captured stdout of
//! running the real Lua `sim/input_tape.lua` (via `sim/match.lua`,
//! `sim/match_snapshot.lua`, `sim/fixed_clock.lua`, `sim/tuning.lua`, and
//! `data/teams.lua`) under headless `love` (no display, no `xvfb`), via a
//! scratch `conf.lua`/`main.lua` harness per that README (not committed —
//! scratch dirs are session-local). The fixture: `nebula` vs `orion`, seed
//! 7, canonical slot-mode ownership, five ticks. Slot `home_1` drives
//! forward the whole recording; slot `away_1` drives backward, and on tick
//! 2 additionally holds sprint and fires pass+dash edges in the same
//! sample — degenerate coverage for simultaneous held+edge bits on a
//! single slot, not just the neutral-everywhere case.

use gc_data::teams;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputFrame, InputSample, InputSampleOptions};
use gc_sim::input_tape::{self, InputTapeIdentity};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot;
use gc_sim::tuning::Tuning;
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/input_tape_lua_reference.txt");

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

fn build_frames() -> Vec<InputFrame> {
    let mut frames = Vec::with_capacity(5);
    for tick in 0..5i64 {
        let mut slots = [InputSample::default(); 8];
        slots[0] = input_frame::new_sample(InputSampleOptions {
            move_x: Some(127),
            ..Default::default()
        })
        .expect("canonical sample");
        slots[4] = if tick == 2 {
            input_frame::new_sample(InputSampleOptions {
                move_x: Some(-127),
                held: Some(input_frame::HELD_SPRINT),
                edges: Some(input_frame::EDGE_PASS + input_frame::EDGE_DASH),
                ..Default::default()
            })
            .expect("canonical sample")
        } else {
            input_frame::new_sample(InputSampleOptions {
                move_x: Some(-127),
                ..Default::default()
            })
            .expect("canonical sample")
        };
        frames.push(input_frame::new(tick, Some(slots)).expect("canonical frame"));
    }
    frames
}

#[test]
fn input_tape_new_matches_the_reference_lua_boundary_hashes_and_frame_wires() {
    let reference = reference();
    let tune = Tuning::new();

    let home = teams::get("nebula").expect("nebula is an authored team");
    let away = teams::get("orion").expect("orion is an authored team");
    let ownership = sim_match::ownership_for_teams(home, away, None);

    let identity = InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "test-build".to_string(),
        source: "test-source".to_string(),
        content: "test-content".to_string(),
        tuning: tune.serialize(),
        config: "test-config".to_string(),
        fixture: "test-fixture".to_string(),
        seed: 7.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership: ownership.clone(),
        combat: None,
    };

    assert_eq!(
        identity.tape_version.to_string(),
        expect(&reference, "identity.tape_version")
    );
    assert_eq!(
        identity.input_version.to_string(),
        expect(&reference, "identity.input_version")
    );
    assert_eq!(
        identity.snapshot_version.to_string(),
        expect(&reference, "identity.snapshot_version")
    );
    assert_eq!(identity.tuning, expect(&reference, "identity.tuning"));
    assert_eq!(
        (identity.seed as i64).to_string(),
        expect(&reference, "identity.seed")
    );
    assert_eq!(
        identity.tick_rate.to_string(),
        expect(&reference, "identity.tick_rate")
    );

    let mut state = sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: match_snapshot::PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(120.0),
        max_goals: Some(3),
        seed: Some(7.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    // Compensating normalization for an upstream `sim::match`/
    // `match_snapshot` contract mismatch — see `input_tape::normalize_marks`'s
    // doc comment (`crates/gc-sim/src/input_tape.rs`) for the full report.
    // `input_tape::new` applies the same normalization internally; this
    // call builds the `initial` snapshot the same way a real caller
    // (`sim::determinism_evidence`) does, so it needs its own copy here.
    let n = state.players.len();
    state.marks.home.resize(n, None);
    state.marks.away.resize(n, None);
    let initial = match_snapshot::capture(&state, None);
    let frames = build_frames();

    for (index, frame) in frames.iter().enumerate() {
        let wire = input_frame::encode(frame).expect("canonical frame encodes");
        assert_eq!(
            wire,
            expect(&reference, &format!("frame_wire[{index}]")),
            "frame_wire[{index}]"
        );
    }

    let tape = input_tape::new(&identity, &initial, &frames, &tune).expect("tape constructs");

    assert_eq!(tape.version.to_string(), expect(&reference, "tape.version"));
    assert_eq!(
        tape.frames.len().to_string(),
        expect(&reference, "frame_count")
    );
    assert_eq!(tape.boundary_hashes.len(), frames.len() + 1);
    for (index, hash) in tape.boundary_hashes.iter().enumerate() {
        assert_eq!(
            hash,
            expect(&reference, &format!("boundary_hash[{index}]")),
            "boundary_hash[{index}] diverges from the reference Lua run \
             (a determinism regression, not merely a spec failure)"
        );
    }

    // The same structural/full-replay validation `sim/replay.lua`'s
    // `validate_context` performs, confirmed against real Lua's own pass.
    assert_eq!("true", expect(&reference, "validate_structure_ok"));
    assert_eq!("true", expect(&reference, "validate_ok"));
    assert!(input_tape::validate_structure(&tape).is_ok());
    assert!(input_tape::validate(&tape, &tune).is_ok());
}
