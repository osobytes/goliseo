//! Differential test for the RenderFrame wire format, against the real Lua.
//!
//! `render/frame_buffer.lua` is the payload that crosses the wasm boundary: the
//! TypeScript renderer reads it by offset, so field order, widths and the
//! version word are a contract between two languages that never share a type.
//!
//! The package's other `frame_buffer` tests round-trip `encode` through
//! `decode`, which proves internal consistency — and a symmetric encoder/decoder
//! bug round-trips perfectly. Only a comparison against the original can catch
//! that class of defect, which is why README rule 5.9 requires this.
//!
//! The reference in `tests/fixtures/frame_buffer_lua_reference.txt` was captured
//! by running the real `render/frame_buffer.lua` under headless `love` — see
//! `v2/tools/lua_reference/README.md` for the harness. Its rows are
//! `label<TAB>word_count<TAB>comma-separated %.17g words`, covering the roster
//! encoding and three frames: kickoff, and after 37 and 200 stepped ticks, so
//! both a pristine and a moved-and-eventful state are compared.
//!
//! The inputs are `slot_input::neutral_match_input()`, not `MatchInput::default()`.
//! Those differ: the neutral input sets `aerial_strike` and `aerial_acrobatic` to
//! an explicit `Some(false)` where the default leaves them `None`, and
//! `aerial::strike_requested` branches on exactly that distinction. Using the
//! wrong one would silently drive a different simulation.

use gc_data::teams;
use gc_render::{frame, frame_buffer};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::PitchSize;
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/frame_buffer_lua_reference.txt");

/// Parse the captured `%.17g` words back into `f64`. The format round-trips
/// binary64 exactly, so this recovers the identical values the Lua held.
fn reference_row(label: &str) -> Vec<f64> {
    let line = FIXTURE
        .lines()
        .find(|l| l.starts_with(&format!("{label}\t")))
        .unwrap_or_else(|| panic!("no reference row labelled {label}"));
    let mut parts = line.split('\t');
    let _label = parts.next();
    let count: usize = parts.next().unwrap().parse().unwrap();
    let words: Vec<f64> = parts
        .next()
        .unwrap()
        .split(',')
        .map(|w| w.parse::<f64>().unwrap())
        .collect();
    assert_eq!(words.len(), count, "{label}: declared count disagrees");
    words
}

/// Compare bit patterns, not printed text: two encoders can agree to seventeen
/// digits and still differ in the last bit, and that bit is what desyncs.
fn assert_words_identical(label: &str, actual: &[f64], expected: &[f64]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{label}: word count {} but Lua produced {}",
        actual.len(),
        expected.len()
    );
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            a.to_bits(),
            e.to_bits(),
            "{label}: word {index} is {a} but Lua produced {e}"
        );
    }
}

#[test]
fn frame_buffer_encodes_the_same_wire_as_lua() {
    let home = teams::get("nebula").expect("nebula");
    let away = teams::get("orion").expect("orion");
    let tune = Tuning::default();

    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(17.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });

    let roster = frame::roster(&state);
    let (roster_words, _digest) = frame_buffer::encode_roster(&roster);
    assert_words_identical("roster", &roster_words, &reference_row("roster"));

    let encode_now = |state: &_| -> Vec<f64> {
        let opts = frame::RenderFrameOptions {
            roster: Some(roster.clone()),
            ..Default::default()
        };
        let built = frame::build(state, &opts);
        let mut words = Vec::new();
        frame_buffer::encode(&built, &mut words);
        words
    };

    assert_words_identical("t0", &encode_now(&state), &reference_row("t0"));

    for _ in 0..37 {
        sim_match::step(
            &mut state,
            1.0 / 60.0,
            StepInput::Legacy(slot_input::neutral_match_input()),
            None,
            &tune,
        );
    }
    assert_words_identical("t37", &encode_now(&state), &reference_row("t37"));

    for _ in 0..163 {
        sim_match::step(
            &mut state,
            1.0 / 60.0,
            StepInput::Legacy(slot_input::neutral_match_input()),
            None,
            &tune,
        );
    }
    assert_words_identical("t200", &encode_now(&state), &reference_row("t200"));
}
