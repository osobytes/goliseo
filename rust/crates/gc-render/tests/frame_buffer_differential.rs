//! Differential test for the `RenderFrame` wire format, against a frozen
//! reference.
//!
//! `frame_buffer.rs` produces the payload that crosses the wasm boundary: the
//! TypeScript renderer reads it by offset, so field order, widths and the
//! version word are a contract between two languages that never share a type.
//!
//! The package's other `frame_buffer` tests round-trip `encode` through
//! `decode`, which proves internal consistency — and a symmetric encoder/decoder
//! bug round-trips perfectly. Only a comparison against an independently
//! produced reference can catch that class of defect, which is why
//! ARCHITECTURE.md's determinism-path differential-testing rule requires this.
//!
//! The reference in `tests/fixtures/frame_buffer_lua_reference.txt` is a
//! frozen capture — it cannot be regenerated, so a failure here is a finding
//! about this code, not a stale fixture to refresh. See
//! `tools/lua_reference/README.md` for the capture methodology and
//! provenance. Its rows are `label<TAB>word_count<TAB>comma-separated %.17g
//! words`, covering the roster encoding and three frames: kickoff, and after
//! 37 and 200 stepped ticks, so both a pristine and a moved-and-eventful
//! state are compared.
//!
//! The inputs are `slot_input::neutral_match_input()`, not `MatchInput::default()`.
//! Those differ: the neutral input sets `aerial_strike` and `aerial_acrobatic` to
//! an explicit `Some(false)` where the default leaves them `None`, and
//! `aerial::strike_requested` branches on exactly that distinction. Using the
//! wrong one would silently drive a different simulation.
//!
//! ## The event section had zero coverage (#332 follow-up)
//!
//! `t0`, `t37` and `t200` above all carry `event_count = 0` — a kickoff and two
//! early ticks of a seed-17 match that, it turns out, never produces a discrete
//! `MatchEvent` in its first ~200 ticks. So none of `EVENT_FIELDS` (`kind`,
//! `save_style`, `style`, `outcome`, `difficulty`, `shot_type`, `keeper_state`,
//! `keeper_depth`, `jumping`, `on_target`) had differential coverage on either
//! side of the wasm boundary — a defect anywhere in event encoding/decoding
//! would have passed every test in the repository.
//!
//! The `s1_t*` rows close that gap. They are a SEPARATE match — same teams,
//! same field, same `slot_input::neutral_match_input()` stepping, but seed `1`
//! instead of `17` — chosen empirically because it produces a livelier
//! match: 14 distinct `MatchEventKind`s across 3000 ticks, where several
//! other seeds surveyed (including 17 itself, which stalls after tick
//! ~1083) run the ball into a corner and stop generating events entirely.
//! Roster is unaffected by seed (it's derived from team/player data, not
//! RNG), so it is not re-captured; only new eventful frames are.
//!
//! The eight `s1_t*` ticks were hand-picked from that seed's event log — ten
//! distinct kinds total (`press_commit_cover`, `tackle`, `reception`, `shot`,
//! `claim`, `juke`, `touch`, `parry`, `catch`, `header`) — to cover: multi-event
//! single-tick frames (`s1_t226`: `press_commit_cover` + `tackle`; `s1_t1236`:
//! `juke` + `touch`); the aerial group `style`/`outcome`/`jumping`/`difficulty`
//! across `reception` (`s1_t294`) and `header` (`s1_t2356`) with both
//! `jumping = false` and `jumping = true`; the keeper group
//! `shot_type`/`keeper_state`/`keeper_depth`/`on_target` on a `shot`
//! (`s1_t374`); `save_style`/`keeper_state`/`keeper_depth` on a `parry`
//! (`s1_t1466`); and — the sharpest edge case — `s1_t2012`'s `catch`, where
//! `keeper_depth` is exactly `0.0` (a legitimate value, not "absent") while
//! `keeper_state` is genuinely absent on that same event, exercising the
//! `has_keeper_depth` presence flag against its own zero payload.
//!
//! Not covered even here: `pass`, `volley`, `press_commit_heavy_touch` and
//! `press_commit_low_discipline` all occurred elsewhere in this same seed-1
//! run but were not among the eight ticks captured; `combat_commit_*` events
//! never fired without a `CombatMatchState` (this harness passes `None`,
//! matching the rest of this file); and `on_target = true`, `tip`, `block`,
//! `bicycle` and `press_commit_box_desperation` were rare or absent across the
//! 22 seeds x 6000 ticks surveyed and were judged not worth engineering a
//! scenario for.

use gc_data::teams;
use gc_render::{frame, frame_buffer};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::PitchSize;
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/frame_buffer_lua_reference.txt");

/// Parse the captured `%.17g` words back into `f64`. The format round-trips
/// binary64 exactly, so this recovers the identical values the reference
/// held.
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

/// Differential coverage for the RenderFrame event section, which the test
/// above never exercises (`t0`/`t37`/`t200` are all `event_count = 0` — see
/// this module's header). A separate seed-1 match, stepped with the same
/// `slot_input::neutral_match_input()`, produces ten distinct
/// `MatchEventKind`s across eight captured ticks; see the header for exactly
/// which fields and edge cases each one exercises.
#[test]
fn frame_buffer_encodes_event_fields_the_same_wire_as_lua() {
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
        seed: Some(1.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });

    let roster = frame::roster(&state);

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

    // Cumulative tick targets, in ascending order: step to each and assert
    // against its fixture row before continuing to the next.
    let targets: &[(u32, &str)] = &[
        (226, "s1_t226"),
        (294, "s1_t294"),
        (374, "s1_t374"),
        (393, "s1_t393"),
        (1236, "s1_t1236"),
        (1466, "s1_t1466"),
        (2012, "s1_t2012"),
        (2356, "s1_t2356"),
    ];

    let mut tick = 0u32;
    for (target_tick, label) in targets {
        while tick < *target_tick {
            sim_match::step(
                &mut state,
                1.0 / 60.0,
                StepInput::Legacy(slot_input::neutral_match_input()),
                None,
                &tune,
            );
            tick += 1;
        }
        assert!(
            !state.events.is_empty(),
            "{label}: expected at least one MatchEvent at tick {tick}"
        );
        assert_words_identical(label, &encode_now(&state), &reference_row(label));
    }
}
