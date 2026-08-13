//! Differential test of `input_tape`'s FORMAT against reference vectors
//! captured from the Lua implementation this simulation was originally
//! validated against (ARCHITECTURE.md §3 rule 7,
//! `tools/lua_reference/README.md`). `sim/input_tape.lua` has no dedicated
//! spec file — it is exercised through `spec/sim/headless_spec.lua`,
//! `replay_spec.lua`, and `determinism_evidence_spec.lua` — but it is
//! squarely on the determinism path (it encodes/decodes recorded inputs and
//! produces the boundary hashes replay and rollback both trust), so it gets
//! this instead: a from-scratch construction of the recording, reproduced
//! identically in both languages, checked byte for byte.
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
//!
//! ## What this file asserts, and what it deliberately does not (#520)
//!
//! This is a FORMAT test. It used to compare all six of the reference's
//! boundary hashes against hashes the Rust side produced by STEPPING the
//! simulation five times — which folded a trajectory claim into a format
//! test. A deliberate gameplay change then turned it red while every
//! encoding it exists to protect was intact, which is #520's finding: the
//! frozen vector's frame wires and its tick-zero boundary compared fine, and
//! only simulated values had moved.
//!
//! So the reference is used for exactly the parts of it that are format:
//!
//! - the identity words (`tape_version`, `input_version`, `snapshot_version`,
//!   the serialized tuning blob, seed, tick rate) and `tape.version`;
//! - all five `frame_wire[i]` strings, byte for byte, in both directions —
//!   `input_frame::encode` must produce them and `input_frame::decode` must
//!   read them back to the identical frame;
//! - `boundary_hash[0]`, the boundary BEFORE any frame is applied. It hashes
//!   the kickoff snapshot, so it pins `match_snapshot`'s serialization order
//!   and `match_snapshot::hash`'s digest algorithm against real Lua without
//!   depending on a single stepped tick;
//! - the whole frozen six-hash sequence as DATA: `input_tape::from_frozen_recording`
//!   is handed the reference's own hashes and must accept them — which
//!   checks each one's canonical form, the one-more-hash-than-frames length
//!   contract, and `boundary_hash[0]` against the snapshot — and the
//!   resulting tape must pass `validate_structure`, which the reference
//!   itself records as `true`.
//!
//! What is NOT compared to the reference is `boundary_hash[1..5]`: those are
//! hashes of stepped state, i.e. the trajectory. Their non-vacuity is
//! covered instead by `input_tape::validate`, which replays the tape and
//! re-derives every boundary — a self-consistency claim that holds whatever
//! the physics does — plus the canonical-form, length and distinctness
//! assertions below.

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

/// Every fixed part of the recording: the identity the reference was
/// captured with, the kickoff snapshot it started from, and the five frames.
struct Recording {
    identity: InputTapeIdentity,
    initial: match_snapshot::MatchSnapshot,
    frames: Vec<InputFrame>,
    tune: Tuning,
}

fn recording() -> Recording {
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

    Recording {
        identity,
        initial,
        frames: build_frames(),
        tune,
    }
}

fn canonical_hash(hash: &str) -> bool {
    hash.len() == 16
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The identity words and the five frame wires: the parts of the recording
/// that are pure encoding, compared byte for byte with the Lua reference in
/// both directions.
#[test]
fn input_frame_wires_and_tape_identity_match_the_reference_lua_byte_for_byte() {
    let reference = reference();
    let recording = recording();
    let identity = &recording.identity;

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

    for (index, frame) in recording.frames.iter().enumerate() {
        let wire = input_frame::encode(frame).expect("canonical frame encodes");
        let expected = expect(&reference, &format!("frame_wire[{index}]"));
        assert_eq!(wire, expected, "frame_wire[{index}]");
        // The reader half of the same claim: an encoder and a decoder that
        // agreed with each other but not with the reference would pass the
        // assertion above only if BOTH matched Lua, and this one proves the
        // decoder reads Lua's own bytes rather than merely its own output.
        let decoded = input_frame::decode(expected).expect("the reference wire decodes");
        assert_eq!(&decoded, frame, "frame_wire[{index}] decodes back");
    }
}

/// The frozen reference recording, read as DATA by the Rust reader.
///
/// `from_frozen_recording` is handed the reference's own six boundary
/// hashes and must accept them. That checks, against real Lua output: every
/// hash's canonical form, the one-more-hash-than-frames length contract,
/// and — the load-bearing one — `boundary_hash[0]` against the hash this
/// build derives from the kickoff snapshot, which pins `match_snapshot`'s
/// serialization order and digest algorithm without stepping a tick.
#[test]
fn the_frozen_reference_recording_is_accepted_and_structurally_valid() {
    let reference = reference();
    let recording = recording();
    let frozen: Vec<String> = (0..=recording.frames.len())
        .map(|index| expect(&reference, &format!("boundary_hash[{index}]")).to_string())
        .collect();

    let tape = input_tape::from_frozen_recording(
        &recording.identity,
        &recording.initial,
        &recording.frames,
        &frozen,
        &recording.tune,
    )
    .expect("the frozen Lua recording is a well-formed tape for this build");

    assert_eq!(tape.version.to_string(), expect(&reference, "tape.version"));
    assert_eq!(
        tape.frames.len().to_string(),
        expect(&reference, "frame_count")
    );
    assert_eq!(tape.boundary_hashes, frozen);
    assert_eq!("true", expect(&reference, "validate_structure_ok"));
    assert!(input_tape::validate_structure(&tape).is_ok());

    // NON-VACUOUS. `from_frozen_recording` accepting the reference above is
    // only evidence if it would reject a recording whose tick-zero boundary
    // does not agree with the snapshot, and one whose hash is not canonical.
    let mut wrong_zero = frozen.clone();
    wrong_zero[0] = "0123456789abcdef".to_string();
    assert!(
        input_tape::from_frozen_recording(
            &recording.identity,
            &recording.initial,
            &recording.frames,
            &wrong_zero,
            &recording.tune,
        )
        .is_err(),
        "a tick-zero boundary that disagrees with the snapshot must be rejected"
    );

    let mut malformed = frozen.clone();
    malformed[3] = "not-a-hash".to_string();
    assert!(
        input_tape::from_frozen_recording(
            &recording.identity,
            &recording.initial,
            &recording.frames,
            &malformed,
            &recording.tune,
        )
        .is_err(),
        "a malformed boundary hash must be rejected"
    );

    let short = frozen[..frozen.len() - 1].to_vec();
    assert!(
        input_tape::from_frozen_recording(
            &recording.identity,
            &recording.initial,
            &recording.frames,
            &short,
            &recording.tune,
        )
        .is_err(),
        "a tape must carry one more boundary hash than it has frames"
    );
}

/// `input_tape::new`'s own output, checked for the shape the format
/// promises rather than for the trajectory it recorded.
///
/// `boundary_hash[0]` is still compared to the Lua reference — it is the
/// pre-step boundary, so it is content, not trajectory. Everything after it
/// is checked structurally (count, canonical form, distinctness) and for
/// self-consistency through `input_tape::validate`, which replays the tape
/// and re-derives every boundary hash it claims.
#[test]
fn a_constructed_tape_has_the_boundary_shape_the_format_promises() {
    let reference = reference();
    let recording = recording();

    let tape = input_tape::new(
        &recording.identity,
        &recording.initial,
        &recording.frames,
        &recording.tune,
    )
    .expect("tape constructs");

    assert_eq!(tape.version.to_string(), expect(&reference, "tape.version"));
    assert_eq!(
        tape.frames.len().to_string(),
        expect(&reference, "frame_count")
    );
    assert_eq!(
        tape.frames, recording.frames,
        "a tape carries the frames it was handed, unaltered"
    );
    assert_eq!(tape.boundary_hashes.len(), recording.frames.len() + 1);
    assert_eq!(
        tape.boundary_hashes[0],
        expect(&reference, "boundary_hash[0]"),
        "the pre-step boundary hash diverges from the reference Lua run \
         (a match_snapshot serialization or digest regression)"
    );
    for (index, hash) in tape.boundary_hashes.iter().enumerate() {
        assert!(
            canonical_hash(hash),
            "boundary_hash[{index}] is not 16 lowercase hex characters: {hash}"
        );
    }
    // Non-vacuity for everything after the first: a tape whose boundaries
    // never moved would satisfy every structural assertion above while
    // recording nothing.
    for index in 1..tape.boundary_hashes.len() {
        assert_ne!(
            tape.boundary_hashes[index],
            tape.boundary_hashes[index - 1],
            "boundary_hash[{index}] repeats its predecessor; the tape recorded no progress"
        );
    }

    // The replaced trajectory comparison's actual subject: a tape's
    // boundaries must be the ones a replay of it re-derives. This holds
    // whatever the physics does, and fails the moment the recorded
    // boundaries and the replayed ones disagree.
    assert_eq!("true", expect(&reference, "validate_ok"));
    assert!(input_tape::validate(&tape, &recording.tune).is_ok());

    // And the same construction twice is the same tape: the format carries
    // no ambient state.
    let again = input_tape::new(
        &recording.identity,
        &recording.initial,
        &recording.frames,
        &recording.tune,
    )
    .expect("tape constructs");
    assert_eq!(again.boundary_hashes, tape.boundary_hashes);
}
