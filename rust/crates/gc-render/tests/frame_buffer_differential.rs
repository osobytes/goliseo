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
//! words`, covering the roster encoding and eleven frames.
//!
//! ## This file encodes FROM the frozen wire, not from a stepped match (#520)
//!
//! It used to build a seed-17 match, step it 37 and 200 ticks, encode each
//! frame and compare the words to the reference. That made a FORMAT test
//! depend on a TRAJECTORY: a deliberate locomotion change moved a player by
//! half a pixel and this test reported `t37: word 56 is 14.303, Lua produced
//! 13.843` — same word index, same width, same version word, and nothing
//! about the encoding wrong. Meanwhile the TypeScript twin
//! (`ts/packages/render/src/frame_buffer.spec.ts`), which DECODES the same
//! frozen fixture instead of re-simulating it, stayed green. That asymmetry
//! is the whole finding.
//!
//! So the reference state is now recovered from the reference itself. Each
//! row is read back into a `RenderFrame` by this file's OWN offset
//! arithmetic and its own hand-written inverse enum tables — deliberately
//! not `frame_buffer::decode`, so that a defect mirrored in the encoder and
//! the decoder alike cannot cancel out — and then re-encoded and compared to
//! the row word for word. Every word of the frozen vector is still pinned,
//! by exactly the same bit-for-bit comparison as before; what is gone is the
//! simulation that used to stand between the fixture and the encoder.
//!
//! `decode_reads_the_frozen_wire_with_the_documented_field_order` then holds
//! `frame_buffer::decode` to the same independent reading, so the reader
//! half of the contract is pinned against the frozen vector too — which the
//! stepped version of this file never checked at all.
//!
//! The three tests here are, in order: the layout constants against their
//! documented literals (which anchors every offset below), the encoder
//! against the frozen wire, and the decoder against it.
//!
//! ## What the reference rows cover
//!
//! `roster`, `t0`, `t37` and `t200` come from a seed-17 match — kickoff and
//! two stepped ticks, so both a pristine and a moved-and-eventful state are
//! represented. All three carry `event_count = 0`: that seed produces no
//! discrete `MatchEvent` in its first ~200 ticks, so none of `EVENT_FIELDS`
//! (`kind`, `save_style`, `style`, `outcome`, `difficulty`, `shot_type`,
//! `keeper_state`, `keeper_depth`, `jumping`, `on_target`) had differential
//! coverage on either side of the wasm boundary (#332 follow-up).
//!
//! The eight `s1_t*` rows close that gap. They are a SEPARATE match — same
//! teams, same field, same `slot_input::neutral_match_input()` stepping, but
//! seed `1` instead of `17` — chosen empirically because it produces a
//! livelier match: 14 distinct `MatchEventKind`s across 3000 ticks, where
//! several other seeds surveyed (including 17 itself, which stalls after
//! tick ~1083) run the ball into a corner and stop generating events
//! entirely. Roster is unaffected by seed (it's derived from team/player
//! data, not RNG), so it is not re-captured.
//!
//! The eight ticks were hand-picked from that seed's event log — ten
//! distinct kinds total (`press_commit_cover`, `tackle`, `reception`,
//! `shot`, `claim`, `juke`, `touch`, `parry`, `catch`, `header`) — to cover:
//! multi-event single-tick frames (`s1_t226`, `s1_t1236`); the aerial group
//! `style`/`outcome`/`jumping`/`difficulty` across `reception` (`s1_t294`)
//! and `header` (`s1_t2356`) with both `jumping = false` and `jumping =
//! true`; the keeper group `shot_type`/`keeper_state`/`keeper_depth`/
//! `on_target` on a `shot` (`s1_t374`); `save_style`/`keeper_state`/
//! `keeper_depth` on a `parry` (`s1_t1466`); and — the sharpest edge case —
//! `s1_t2012`'s `catch`, where `keeper_depth` is exactly `0.0` (a legitimate
//! value, not "absent") while `keeper_state` is genuinely absent on that
//! same event, exercising the `has_keeper_depth` presence flag against its
//! own zero payload.
//!
//! Not covered even here: `pass`, `volley`, `press_commit_heavy_touch` and
//! `press_commit_low_discipline` all occurred elsewhere in this same seed-1
//! run but were not among the eight ticks captured; `combat_commit_*` events
//! never fired without a `CombatMatchState` (the capture harness passed
//! `None`); and `on_target = true`, `tip`, `block`, `bicycle` and
//! `press_commit_box_desperation` were rare or absent across the 22 seeds x
//! 6000 ticks surveyed and were judged not worth engineering a scenario for.

use gc_data::species::Shape;
use gc_render::frame::{
    self, RenderChargeKind, RenderFrame, RenderFrameBall, RenderFrameControl, RenderFrameEvents,
    RenderFrameField, RenderFrameHud, RenderFramePlayers, RenderFramePossession, RenderFrameRoster,
};
use gc_render::frame_buffer;
use gc_render::player_pose::{PlayerPoseId, PlayerPoseSource};
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::match_snapshot::{MatchEventKind, Rect, Team};

const FIXTURE: &str = include_str!("fixtures/frame_buffer_lua_reference.txt");

/// Every frame row in the reference, in fixture order.
const FRAME_ROWS: &[&str] = &[
    "t0", "t37", "t200", "s1_t226", "s1_t294", "s1_t374", "s1_t393", "s1_t1236", "s1_t1466",
    "s1_t2012", "s1_t2356",
];

// This file's OWN copy of the layout, written as literals rather than taken
// from the module under test. `layout_constants_match_their_documented_shape`
// asserts the module agrees with every one of them, so a mutation to the
// module moves the module and not these — which is what lets the offset
// arithmetic below detect it instead of silently following it.
const HEADER: usize = 12;
const SCALARS: usize = 44;
const PLAYER_FIELDS: usize = 22;
const EVENT_FIELDS: usize = 15;
const ROSTER_HEADER: usize = 7;
const ROSTER_FIELDS: usize = 7;

// The layout the FROZEN REFERENCE was captured under. #576 appended one
// per-player field (`phase_fraction`, field index 21) and bumped
// `LAYOUT_VERSION`/`frame::VERSION` 1 → 2; the reference predates the port
// and cannot be regenerated, so its rows stay layout-1 forever and are read
// with these offsets. `migrate_frame_row`/`migrate_roster_row` below are the
// documented bridge from the frozen shape to the current one.
const V1_LAYOUT_VERSION: f64 = 1.0;
const V1_RENDER_FRAME_VERSION: f64 = 1.0;
const V1_PLAYER_FIELDS: usize = 21;

/// Mechanically lift a frozen layout-1 FRAME row to the current layout-2
/// shape, so the bit-for-bit comparisons below keep pinning every captured
/// word (#576).
///
/// This is NOT a fixture refresh, and it deliberately is not free-form: the
/// only permitted differences between the frozen row and its migrated form
/// are (a) the three header words the bump restamps (`layout_version`,
/// `render_frame_version`, `player_field_count`, plus the `total_words` that
/// follows arithmetically) and (b) one appended all-zeros player column —
/// zeros because the capture harness passed no `CombatMatchState`
/// (`rebuild_frame` asserts `combat_present == 0` on every row), and a frame
/// without combat encodes `phase_fraction` as a dense zero column by
/// construction. Every other word crosses unchanged, at an offset derived
/// from the same arithmetic the reader uses, so a captured word that moved
/// anywhere else still fails the comparison.
fn migrate_frame_row(label: &str, words: &[f64]) -> Vec<f64> {
    assert_eq!(words[1], V1_LAYOUT_VERSION, "{label}: frozen layout");
    assert_eq!(words[2], V1_RENDER_FRAME_VERSION, "{label}: frozen version");
    assert_eq!(
        words[7], V1_PLAYER_FIELDS as f64,
        "{label}: frozen player fields"
    );
    let count = words[6] as usize;
    let event_count = words[8] as usize;
    let players_at = HEADER + SCALARS;
    let events_at_v1 = players_at + V1_PLAYER_FIELDS * count;
    assert_eq!(
        words.len(),
        events_at_v1 + EVENT_FIELDS * event_count,
        "{label}: frozen row length"
    );

    let mut out = Vec::with_capacity(words.len() + count);
    out.extend_from_slice(&words[..events_at_v1]);
    out.extend(std::iter::repeat_n(0.0, count)); // phase_fraction, field 21
    out.extend_from_slice(&words[events_at_v1..]);
    out[1] = f64::from(frame_buffer::LAYOUT_VERSION);
    out[2] = f64::from(frame::VERSION);
    out[3] = out.len() as f64;
    out[7] = PLAYER_FIELDS as f64;
    out
}

/// The roster block's #576 migration is restamping alone: no roster field
/// was added, so only the two version words move and every payload word is
/// pinned exactly where the capture put it.
fn migrate_roster_row(words: &[f64]) -> Vec<f64> {
    assert_eq!(words[1], V1_LAYOUT_VERSION, "roster: frozen layout");
    assert_eq!(words[2], V1_RENDER_FRAME_VERSION, "roster: frozen version");
    let mut out = words.to_vec();
    out[1] = f64::from(frame_buffer::LAYOUT_VERSION);
    out[2] = f64::from(frame::VERSION);
    out
}

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
        "{label}: word count {} but the reference holds {}",
        actual.len(),
        expected.len()
    );
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            a.to_bits(),
            e.to_bits(),
            "{label}: word {index} is {a} but the reference holds {e}"
        );
    }
}

// ---------------------------------------------------------------------------
// This file's own reader: offsets and inverse enum tables written out by
// hand from `crates/gc-render/src/frame_buffer.rs`'s documented numbering,
// so that a change to the module's own tables shows up as a disagreement
// rather than being followed silently.
// ---------------------------------------------------------------------------

/// One structure-of-arrays cell: `base` is the section's first word,
/// `field` its 0-based field index, `slot` the entry.
fn soa(words: &[f64], base: usize, field: usize, slot: usize, count: usize) -> f64 {
    words[base + field * count + slot]
}

fn flag(word: f64) -> bool {
    assert!(word == 0.0 || word == 1.0, "boolean word is {word}");
    word == 1.0
}

fn team_from(code: f64) -> Option<Team> {
    match code as i64 {
        0 => None,
        1 => Some(Team::Home),
        2 => Some(Team::Away),
        other => panic!("unknown team code {other}"),
    }
}

fn shape_from(code: f64) -> Shape {
    match code as i64 {
        1 => Shape::Round,
        2 => Shape::Broad,
        3 => Shape::Angular,
        4 => Shape::Cluster,
        other => panic!("unknown species shape code {other}"),
    }
}

fn charge_kind_from(code: f64) -> Option<RenderChargeKind> {
    match code as i64 {
        0 => None,
        1 => Some(RenderChargeKind::Shot),
        2 => Some(RenderChargeKind::Pass),
        other => panic!("unknown charge kind code {other}"),
    }
}

fn pose_source_from(code: f64) -> PlayerPoseSource {
    match code as i64 {
        1 => PlayerPoseSource::Soccer,
        2 => PlayerPoseSource::Combat,
        3 => PlayerPoseSource::Locomotion,
        other => panic!("unknown pose source code {other}"),
    }
}

fn aerial_style_from(code: f64) -> Option<AerialStyle> {
    match code as i64 {
        0 => None,
        1 => Some(AerialStyle::LegControl),
        2 => Some(AerialStyle::ChestControl),
        3 => Some(AerialStyle::Volley),
        4 => Some(AerialStyle::Header),
        5 => Some(AerialStyle::Bicycle),
        other => panic!("unknown aerial style code {other}"),
    }
}

fn aerial_outcome_from(code: f64) -> Option<AerialOutcome> {
    match code as i64 {
        0 => None,
        1 => Some(AerialOutcome::Clean),
        2 => Some(AerialOutcome::Heavy),
        3 => Some(AerialOutcome::Miss),
        other => panic!("unknown aerial outcome code {other}"),
    }
}

fn save_style_from(code: f64) -> Option<SaveStyle> {
    match code as i64 {
        0 => None,
        1 => Some(SaveStyle::Spread),
        2 => Some(SaveStyle::Central),
        3 => Some(SaveStyle::Stretch),
        other => panic!("unknown save style code {other}"),
    }
}

fn keeper_state_from(code: f64) -> Option<KeeperBehaviorState> {
    match code as i64 {
        0 => None,
        1 => Some(KeeperBehaviorState::Base),
        2 => Some(KeeperBehaviorState::Advance),
        3 => Some(KeeperBehaviorState::Contain),
        4 => Some(KeeperBehaviorState::Set),
        5 => Some(KeeperBehaviorState::Retreat),
        6 => Some(KeeperBehaviorState::Recover),
        other => panic!("unknown keeper state code {other}"),
    }
}

fn shot_type_from(code: f64) -> Option<KeeperShotType> {
    match code as i64 {
        0 => None,
        1 => Some(KeeperShotType::Ground),
        2 => Some(KeeperShotType::Chip),
        other => panic!("unknown shot type code {other}"),
    }
}

/// `pose_id` and `event_kind` are the two numberings with thirty-two and
/// twenty-five members; transcribing them a second time would buy a copy of
/// the same table rather than an independent one, so the module's own
/// inverse is used for them. Everything positional about them — which word
/// carries them, and how wide it is — is still pinned by the comparison
/// against the frozen row.
fn pose_id_from(code: f64) -> PlayerPoseId {
    frame_buffer::pose_id_from_code(code).unwrap_or_else(|| panic!("unknown pose id code {code}"))
}

fn event_kind_from(code: f64) -> MatchEventKind {
    frame_buffer::event_kind_from_code(code)
        .unwrap_or_else(|| panic!("unknown event kind code {code}"))
}

/// An empty roster, only so `encode`'s `roster.version` assertion has
/// something to read: the roster block is encoded separately, by
/// `encode_roster`, and `encode` never looks at any other field of it.
fn placeholder_roster() -> RenderFrameRoster {
    RenderFrameRoster {
        version: frame::VERSION,
        count: 0,
        ids: Vec::new(),
        names: Vec::new(),
        presentation_ids: Vec::new(),
        loadout_ids: Vec::new(),
        teams: Vec::new(),
        is_keeper: Vec::new(),
        radius: Vec::new(),
        species_shape: Vec::new(),
        species_color: Vec::new(),
    }
}

/// Recover the `RenderFrame` a reference row was encoded from, reading the
/// frozen words with this file's own LAYOUT-1 offsets (the shape the
/// reference was captured under — see `migrate_frame_row`) rather than the
/// module's decoder.
fn rebuild_frame(label: &str, words: &[f64]) -> RenderFrame {
    let count = words[6] as usize;
    let event_count = words[8] as usize;
    assert!(
        words[10] == 0.0,
        "{label}: the frozen reference never captured a combat model"
    );

    let s = |index: usize| words[HEADER + index];
    let players_at = HEADER + SCALARS;
    let events_at = players_at + V1_PLAYER_FIELDS * count;

    let field = RenderFrameField {
        w: s(0),
        h: s(1),
        crossbar_h: s(2),
        penalty_box_depth: s(3),
        penalty_box_h: s(4),
        goal_home: Rect {
            x: s(5),
            y: s(6),
            w: s(7),
            h: s(8),
        },
        goal_away: Rect {
            x: s(9),
            y: s(10),
            w: s(11),
            h: s(12),
        },
    };
    let landing = flag(s(20));
    let ball = RenderFrameBall {
        x: s(13),
        y: s(14),
        z: s(15),
        vx: s(16),
        vy: s(17),
        vz: s(18),
        visible: flag(s(19)),
        landing_x: landing.then(|| s(21)),
        landing_y: landing.then(|| s(22)),
    };
    let owner = s(23) as i64;
    let possession = RenderFramePossession {
        owner: (owner != 0).then_some(owner),
        owner_team: team_from(s(24)),
        keeper_holds: flag(s(25)),
    };
    let pass_target = s(27) as i64;
    let control = RenderFrameControl {
        controlled: s(26) as i64,
        pass_target: (pass_target != 0).then_some(pass_target),
        charge_kind: charge_kind_from(s(28)),
        charge: s(29),
    };
    let hud = RenderFrameHud {
        home_score: s(30) as i64,
        away_score: s(31) as i64,
        time_left: s(32),
        finished: flag(s(33)),
        possession_team: team_from(s(34)),
        controlled: s(35) as i64,
        // Not encoded: it is a roster lookup of `controlled`, which is.
        controlled_id: String::new(),
        controlled_team: team_from(s(36)).expect("the controlled player always has a team"),
        controlled_is_keeper: flag(s(37)),
        controlled_owns_ball: flag(s(38)),
        controlled_stamina: s(39),
        species_shape: shape_from(s(40)),
        species_color: [s(41), s(42), s(43)],
    };

    let p = |field: usize, slot: usize| soa(words, players_at, field, slot, count);
    let players = RenderFramePlayers {
        count,
        x: (0..count).map(|i| p(0, i)).collect(),
        y: (0..count).map(|i| p(1, i)).collect(),
        facing_x: (0..count).map(|i| p(2, i)).collect(),
        facing_y: (0..count).map(|i| p(3, i)).collect(),
        speed: (0..count).map(|i| p(4, i)).collect(),
        pose_id: (0..count).map(|i| pose_id_from(p(5, i))).collect(),
        pose_priority: (0..count).map(|i| p(6, i) as i64).collect(),
        pose_source: (0..count).map(|i| pose_source_from(p(7, i))).collect(),
        controlled: (0..count).map(|i| flag(p(8, i))).collect(),
        dashing: (0..count).map(|i| flag(p(9, i))).collect(),
        holding: (0..count).map(|i| flag(p(10, i))).collect(),
        dive: (0..count).map(|i| p(11, i)).collect(),
        dive_dir_x: (0..count).map(|i| p(12, i)).collect(),
        dive_dir_y: (0..count).map(|i| p(13, i)).collect(),
        grab: (0..count).map(|i| p(14, i)).collect(),
        throw: (0..count).map(|i| p(15, i)).collect(),
        windup: (0..count).map(|i| p(16, i)).collect(),
        aerial: (0..count).map(|i| p(17, i)).collect(),
        aerial_jump: (0..count).map(|i| p(18, i)).collect(),
        aerial_style: (0..count).map(|i| aerial_style_from(p(19, i))).collect(),
        aerial_outcome: (0..count).map(|i| aerial_outcome_from(p(20, i))).collect(),
        // Not in the frozen capture: the reference predates #576's column,
        // and its rows carried no combat model (asserted above), so the
        // state it was encoded from can only have held zeros here.
        phase_fraction: vec![0.0; count],
    };

    let e = |field: usize, slot: usize| soa(words, events_at, field, slot, event_count);
    let events = RenderFrameEvents {
        count: event_count,
        kind: (0..event_count).map(|i| event_kind_from(e(0, i))).collect(),
        x: (0..event_count).map(|i| e(1, i)).collect(),
        y: (0..event_count).map(|i| e(2, i)).collect(),
        // Not encoded: `slot` indexes the roster, which crossed already.
        player: (0..event_count).map(|_| None).collect(),
        slot: (0..event_count)
            .map(|i| {
                let slot = e(3, i) as i64;
                (slot != 0).then_some(slot)
            })
            .collect(),
        save_style: (0..event_count).map(|i| save_style_from(e(4, i))).collect(),
        style: (0..event_count)
            .map(|i| aerial_style_from(e(5, i)))
            .collect(),
        outcome: (0..event_count)
            .map(|i| aerial_outcome_from(e(6, i)))
            .collect(),
        difficulty: (0..event_count)
            .map(|i| flag(e(7, i)).then(|| e(8, i)))
            .collect(),
        shot_type: (0..event_count).map(|i| shot_type_from(e(9, i))).collect(),
        keeper_state: (0..event_count)
            .map(|i| keeper_state_from(e(10, i)))
            .collect(),
        keeper_depth: (0..event_count)
            .map(|i| flag(e(11, i)).then(|| e(12, i)))
            .collect(),
        jumping: (0..event_count).map(|i| e(13, i) as i64).collect(),
        on_target: (0..event_count).map(|i| e(14, i) as i64).collect(),
    };

    RenderFrame {
        version: frame::VERSION,
        roster: placeholder_roster(),
        field,
        players,
        ball,
        possession,
        control,
        hud,
        events,
        combat: None,
    }
}

/// Recover the `RenderFrameRoster` the reference's roster row was encoded
/// from. Its string blob was never captured (the numeric block is what the
/// two languages read positionally), so the strings are synthetic — and
/// `encode_roster` rejects an empty presentation id, which is asserted
/// separately in `tests/frame_buffer.rs`.
fn rebuild_roster(words: &[f64]) -> RenderFrameRoster {
    let count = words[5] as usize;
    let r = |field: usize, slot: usize| soa(words, ROSTER_HEADER, field, slot, count);
    RenderFrameRoster {
        version: frame::VERSION,
        count,
        ids: (0..count).map(|i| format!("player_{i}")).collect(),
        names: (0..count).map(|i| format!("Player {i}")).collect(),
        presentation_ids: (0..count).map(|i| format!("presentation_{i}")).collect(),
        loadout_ids: (0..count)
            .map(|i| (i % 2 == 0).then(|| format!("loadout_{i}")))
            .collect(),
        teams: (0..count)
            .map(|i| team_from(r(0, i)).expect("a roster slot always has a team"))
            .collect(),
        is_keeper: (0..count).map(|i| flag(r(1, i))).collect(),
        radius: (0..count).map(|i| r(2, i)).collect(),
        species_shape: (0..count).map(|i| shape_from(r(3, i))).collect(),
        species_color: (0..count).map(|i| [r(4, i), r(5, i), r(6, i)]).collect(),
    }
}

/// The literals every offset in this file is written against. Without this
/// case, a mutation to one of these constants would move the module and the
/// offsets below together and cancel out.
#[test]
fn layout_constants_match_their_documented_shape() {
    assert_eq!(frame_buffer::MAGIC, 0x474f_4c46);
    assert_eq!(frame_buffer::ROSTER_MAGIC, 0x474f_4c52);
    // #576: layout and protocol both moved 1 → 2 for the appended
    // `phase_fraction` player column.
    assert_eq!(frame_buffer::LAYOUT_VERSION, 2);
    assert_eq!(frame::VERSION, 2);
    assert_eq!(frame_buffer::HEADER_WORDS, HEADER);
    assert_eq!(frame_buffer::SCALAR_FIELD_COUNT, SCALARS);
    assert_eq!(frame_buffer::PLAYER_FIELD_COUNT, PLAYER_FIELDS);
    assert_eq!(frame_buffer::EVENT_FIELD_COUNT, EVENT_FIELDS);
    assert_eq!(frame_buffer::ROSTER_HEADER_WORDS, ROSTER_HEADER);
    assert_eq!(frame_buffer::ROSTER_FIELD_COUNT, ROSTER_FIELDS);
    assert_eq!(frame_buffer::ROSTER_STRING_FIELD_COUNT, 4);

    // The reference stamps the shape it was CAPTURED under into every row it
    // holds — layout 1, the pre-#576 21-field player section. That is what
    // the other language was compiled against at capture time, and it is
    // what `migrate_frame_row` requires before lifting a row.
    let roster = reference_row("roster");
    assert_eq!(roster[0], f64::from(0x474f_4c52_u32), "roster row magic");
    assert_eq!(roster[1], V1_LAYOUT_VERSION, "roster row layout version");
    assert_eq!(
        roster[2], V1_RENDER_FRAME_VERSION,
        "roster row render frame version"
    );
    assert_eq!(roster[4], ROSTER_HEADER as f64);
    assert_eq!(roster[6], ROSTER_FIELDS as f64);
    for label in FRAME_ROWS {
        let words = reference_row(label);
        assert_eq!(words[0], f64::from(0x474f_4c46_u32), "{label} magic");
        assert_eq!(words[1], V1_LAYOUT_VERSION, "{label} layout version");
        assert_eq!(
            words[2], V1_RENDER_FRAME_VERSION,
            "{label} render frame version"
        );
        assert_eq!(words[4], HEADER as f64, "{label} header words");
        assert_eq!(words[5], SCALARS as f64, "{label} scalar words");
        assert_eq!(words[7], V1_PLAYER_FIELDS as f64, "{label} player fields");
        assert_eq!(words[9], EVENT_FIELDS as f64, "{label} event fields");
    }
}

/// The encoder against the frozen wire, word for word: every reference row
/// is read back into the state it was encoded from and re-encoded, and the
/// result must be bit-identical to the row's MIGRATED form — the frozen
/// layout-1 words lifted through `migrate_frame_row`'s restamp-and-append
/// bridge, which is itself constrained to touch nothing but the #576 header
/// words and the appended zero column. Every captured payload word is still
/// compared bit for bit.
#[test]
fn re_encodes_every_frozen_reference_row_bit_for_bit() {
    // "Every" is a claim about the fixture, not about `FRAME_ROWS`: a row
    // added to the vector and left out of the list below would otherwise be
    // silently uncompared.
    let labels: Vec<&str> = FIXTURE
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').next().expect("a labelled row"))
        .collect();
    let mut expected: Vec<&str> = FRAME_ROWS.to_vec();
    expected.push("roster");
    expected.sort_unstable();
    let mut found = labels.clone();
    found.sort_unstable();
    assert_eq!(found, expected, "every reference row must be compared");

    let roster_words = reference_row("roster");
    let (encoded_roster, _blob) = frame_buffer::encode_roster(&rebuild_roster(&roster_words));
    assert_words_identical(
        "roster",
        &encoded_roster,
        &migrate_roster_row(&roster_words),
    );

    let mut events_seen = 0usize;
    for label in FRAME_ROWS {
        let words = reference_row(label);
        let frame = rebuild_frame(label, &words);
        events_seen += frame.events.count;
        let mut encoded = Vec::new();
        frame_buffer::encode(&frame, &mut encoded);
        assert_words_identical(label, &encoded, &migrate_frame_row(label, &words));
    }

    // NON-VACUOUS. The event section is the half of this layout that the
    // seed-17 rows cannot reach at all; if the `s1_t*` rows stopped carrying
    // events, ten of the fifteen event fields would be compared only as
    // zeroes and this file would silently stop covering them.
    assert_eq!(
        events_seen, 10,
        "the reference rows must carry the event section: eight `s1_t*` rows, \
         two of which hold two events"
    );
}

/// The decoder against the same frozen wire. `frame_buffer::decode` must
/// read the reference's words into the same columns this file's own offsets
/// find them in — so an offset defect mirrored in `encode` and `decode`,
/// which the re-encode above would let cancel out, fails here.
#[test]
fn decode_reads_the_frozen_wire_with_the_documented_field_order() {
    let roster_words = migrate_roster_row(&reference_row("roster"));
    let count = roster_words[5] as usize;
    let blob: Vec<String> = (0..count)
        .flat_map(|i| {
            [
                format!("player_{i}"),
                format!("Player {i}"),
                format!("presentation_{i}"),
                if i % 2 == 0 {
                    format!("loadout_{i}")
                } else {
                    String::new()
                },
            ]
        })
        .collect();
    let decoded_roster = frame_buffer::decode_roster(&roster_words, &blob.join("\n"));
    assert_eq!(decoded_roster.layout_version, 2);
    assert_eq!(decoded_roster.render_frame_version, 2);
    assert_eq!(decoded_roster.count, count);
    let r = |field: usize| -> Vec<f64> {
        (0..count)
            .map(|i| soa(&roster_words, ROSTER_HEADER, field, i, count))
            .collect()
    };
    let fields = &decoded_roster.fields;
    assert_eq!(fields.team, r(0), "roster field 0 is team");
    assert_eq!(fields.is_keeper, r(1), "roster field 1 is is_keeper");
    assert_eq!(fields.radius, r(2), "roster field 2 is radius");
    assert_eq!(
        fields.species_shape,
        r(3),
        "roster field 3 is species_shape"
    );
    assert_eq!(fields.species_color_r, r(4), "roster field 4 is color r");
    assert_eq!(fields.species_color_g, r(5), "roster field 5 is color g");
    assert_eq!(fields.species_color_b, r(6), "roster field 6 is color b");

    for label in FRAME_ROWS {
        let words = migrate_frame_row(label, &reference_row(label));
        let decoded = frame_buffer::decode(&words);
        let count = words[6] as usize;
        let event_count = words[8] as usize;

        assert_eq!(decoded.layout_version, 2, "{label} layout version");
        assert_eq!(
            decoded.render_frame_version, 2,
            "{label} render frame version"
        );
        assert!(!decoded.combat_present, "{label} combat_present");
        assert_eq!(
            decoded.scalars.as_slice(),
            &words[HEADER..HEADER + SCALARS],
            "{label} scalar block"
        );

        let players_at = HEADER + SCALARS;
        let events_at = players_at + PLAYER_FIELDS * count;
        let column = |base: usize, field: usize, len: usize| -> Vec<f64> {
            (0..len).map(|i| soa(&words, base, field, i, len)).collect()
        };
        let pc = |field: usize| column(players_at, field, count);
        let p = &decoded.players;
        let player_columns: [(&str, &Vec<f64>, usize); PLAYER_FIELDS] = [
            ("x", &p.x, 0),
            ("y", &p.y, 1),
            ("facing_x", &p.facing_x, 2),
            ("facing_y", &p.facing_y, 3),
            ("speed", &p.speed, 4),
            ("pose_id", &p.pose_id, 5),
            ("pose_priority", &p.pose_priority, 6),
            ("pose_source", &p.pose_source, 7),
            ("controlled", &p.controlled, 8),
            ("dashing", &p.dashing, 9),
            ("holding", &p.holding, 10),
            ("dive", &p.dive, 11),
            ("dive_dir_x", &p.dive_dir_x, 12),
            ("dive_dir_y", &p.dive_dir_y, 13),
            ("grab", &p.grab, 14),
            ("throw", &p.throw, 15),
            ("windup", &p.windup, 16),
            ("aerial", &p.aerial, 17),
            ("aerial_jump", &p.aerial_jump, 18),
            ("aerial_style", &p.aerial_style, 19),
            ("aerial_outcome", &p.aerial_outcome, 20),
            ("phase_fraction", &p.phase_fraction, 21),
        ];
        for (name, actual, field) in player_columns {
            assert_eq!(
                actual,
                &pc(field),
                "{label}: player field {field} is not {name}"
            );
        }

        let ec = |field: usize| column(events_at, field, event_count);
        let e = &decoded.events;
        let event_columns: [(&str, &Vec<f64>, usize); EVENT_FIELDS] = [
            ("kind", &e.kind, 0),
            ("x", &e.x, 1),
            ("y", &e.y, 2),
            ("slot", &e.slot, 3),
            ("save_style", &e.save_style, 4),
            ("style", &e.style, 5),
            ("outcome", &e.outcome, 6),
            ("has_difficulty", &e.has_difficulty, 7),
            ("difficulty", &e.difficulty, 8),
            ("shot_type", &e.shot_type, 9),
            ("keeper_state", &e.keeper_state, 10),
            ("has_keeper_depth", &e.has_keeper_depth, 11),
            ("keeper_depth", &e.keeper_depth, 12),
            ("jumping", &e.jumping, 13),
            ("on_target", &e.on_target, 14),
        ];
        for (name, actual, field) in event_columns {
            assert_eq!(
                actual,
                &ec(field),
                "{label}: event field {field} is not {name}"
            );
        }
    }
}
