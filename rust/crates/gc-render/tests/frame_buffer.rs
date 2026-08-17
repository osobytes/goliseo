//! Tier-1 logic tests for `RenderFrame`'s wire encoding: no display, no
//! wasm, no browser. What they pin is the WIRE — header field counts,
//! section offsets, enum codes, version stamps — the same things that would
//! also break a JavaScript reader of this block if they regressed.

use gc_data::species::Shape;
use gc_render::frame::{self as render_frame, RenderFrameOptions};
use gc_render::frame_buffer;
use gc_render::player_pose::PlayerPoseSource;
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchEventKind, MatchState, PitchSize, Team};

fn fixture(seed: f64) -> MatchState {
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
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

// Puts one event of `kind` into a built frame's event channel. Kickoff
// produces none, and the sparse/tri-state encoding rules only have anything
// to say once there is one.
fn with_event(frame: &mut render_frame::RenderFrame, kind: MatchEventKind) {
    let events = &mut frame.events;
    events.count = 1;
    events.kind = vec![kind];
    events.x = vec![0.0];
    events.y = vec![0.0];
    events.player = vec![None];
    events.slot = vec![None];
    events.save_style = vec![None];
    events.style = vec![None];
    events.outcome = vec![None];
    events.difficulty = vec![None];
    events.shot_type = vec![None];
    events.keeper_state = vec![None];
    events.keeper_depth = vec![None];
    events.jumping = vec![0];
    events.on_target = vec![0];
}

#[test]
fn round_trips_a_whole_frame_through_one_flat_array_of_doubles() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);

    let decoded = frame_buffer::decode(&words);
    assert_eq!(decoded.render_frame_version, render_frame::VERSION);
    assert_eq!(decoded.layout_version, frame_buffer::LAYOUT_VERSION);
    assert_eq!(decoded.scalars[13], frame.ball.x); // ball_x, see write_scalars order
    assert_eq!(decoded.scalars[14], frame.ball.y); // ball_y
    assert_eq!(decoded.scalars[0], frame.field.w); // field_w
    assert_eq!(decoded.scalars[30] as i64, frame.hud.home_score); // hud_home_score
    assert_eq!(decoded.players.x.len(), frame.players.count);
    for index in 0..frame.players.count {
        assert_eq!(decoded.players.x[index], frame.players.x[index]);
        assert_eq!(decoded.players.y[index], frame.players.y[index]);
        assert_eq!(
            frame_buffer::pose_id(&decoded, index),
            Some(frame.players.pose_id[index])
        );
    }
}

#[test]
fn stamps_both_versions_and_rejects_a_frame_carrying_another_one() {
    let state = fixture(17.0);
    let mut frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    assert_eq!(words[1], f64::from(frame_buffer::LAYOUT_VERSION));
    assert_eq!(words[2], f64::from(render_frame::VERSION));

    // The read-side assertion is `frame_buffer`'s. A payload from a future
    // producer must be refused, not read positionally and misinterpreted.
    frame.version = render_frame::VERSION + 1;
    let mut scratch = Vec::new();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| frame_buffer::encode(
            &frame,
            &mut scratch
        )))
        .is_err(),
        "encoding a frame stamped with another protocol version must fail"
    );
    frame.version = render_frame::VERSION;
    frame.roster.version = render_frame::VERSION + 1;
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| frame_buffer::encode(
            &frame,
            &mut scratch
        )))
        .is_err(),
        "encoding against a roster from another protocol version must fail"
    );
}

#[test]
fn detects_a_corrupted_header_rather_than_misreading_the_block() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);

    // Every header word a JavaScript reader would also check. If one of
    // these stops being checked here, the two readers have drifted apart.
    for &word in &[0usize, 1, 2, 4, 5, 7, 9] {
        let original = words[word];
        words[word] = original + 1.0;
        let corrupted = words.clone();
        assert!(
            std::panic::catch_unwind(|| frame_buffer::decode(&corrupted)).is_err(),
            "a corrupted header word {word} was read as valid"
        );
        words[word] = original;
    }
    let intact = words.clone();
    assert!(
        std::panic::catch_unwind(|| frame_buffer::decode(&intact)).is_ok(),
        "the intact block must still decode"
    );
}

#[test]
fn agrees_with_the_header_it_writes_about_its_own_geometry() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);

    assert_eq!(words[0], f64::from(frame_buffer::MAGIC));
    assert_eq!(
        words[3],
        words.len() as f64,
        "total_words must size the whole block"
    );
    assert_eq!(words[4], frame_buffer::HEADER_WORDS as f64);
    assert_eq!(words[5], frame_buffer::SCALAR_FIELD_COUNT as f64);
    assert_eq!(words[6], frame.players.count as f64);
    assert_eq!(words[7], frame_buffer::PLAYER_FIELD_COUNT as f64);
    assert_eq!(words[8], frame.events.count as f64);
    assert_eq!(words[9], frame_buffer::EVENT_FIELD_COUNT as f64);
    assert_eq!(
        words[3],
        frame_buffer::frame_words(frame.players.count, frame.events.count) as f64,
        "a reader sizing its view from the header must get the whole block"
    );
}

#[test]
fn lays_per_entity_data_out_field_major_so_one_field_is_one_run() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let count = frame.players.count;
    let at = frame_buffer::HEADER_WORDS + frame_buffer::SCALAR_FIELD_COUNT;

    // Structure of arrays is the whole reason this encoding is affordable: a
    // reader that wants positions takes two contiguous runs and never
    // touches the other nineteen fields. Spot-check the two float fields
    // whose source is directly comparable (`x`/`y`); the rest are exercised
    // through `decode` elsewhere.
    for index in 0..count {
        let x_word = words[at + index];
        let y_word = words[at + count + index];
        assert!((x_word - frame.players.x[index]).abs() < 1e-9, "x[{index}]");
        assert!((y_word - frame.players.y[index]).abs() < 1e-9, "y[{index}]");
    }
}

#[test]
fn maps_an_absent_enum_to_zero_and_a_present_one_to_its_code() {
    let state = fixture(17.0);
    let mut frame = render_frame::build(&state, &RenderFrameOptions::default());

    // Nothing is airborne at kickoff, so every aerial style is absent.
    for index in 0..frame.players.count {
        frame.players.aerial_style[index] = None;
        frame.players.aerial_outcome[index] = None;
    }
    frame.players.aerial_style[0] = Some(AerialStyle::Bicycle);
    frame.players.aerial_outcome[0] = Some(AerialOutcome::Heavy);

    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let decoded = frame_buffer::decode(&words);
    assert_eq!(
        decoded.players.aerial_style[0],
        frame_buffer::aerial_style_code(AerialStyle::Bicycle)
    );
    assert_eq!(
        decoded.players.aerial_outcome[0],
        frame_buffer::aerial_outcome_code(AerialOutcome::Heavy)
    );
    for index in 1..frame.players.count {
        assert_eq!(
            decoded.players.aerial_style[index], 0.0,
            "an absent enum must encode as zero"
        );
        assert_eq!(
            decoded.players.aerial_outcome[index], 0.0,
            "an absent enum must encode as zero"
        );
    }
}

#[test]
fn keeps_absent_false_and_true_distinguishable_for_a_sparse_number() {
    let state = fixture(17.0);
    let mut frame = render_frame::build(&state, &RenderFrameOptions::default());
    with_event(&mut frame, MatchEventKind::Header);
    frame.events.difficulty[0] = None;

    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let decoded = frame_buffer::decode(&words);
    assert_eq!(
        decoded.events.has_difficulty[0], 0.0,
        "an absent sparse number must say so"
    );
    assert_eq!(decoded.events.difficulty[0], 0.0);

    // A difficulty of exactly zero is a real, reported value, and it must
    // not read back as "not reported". This is why the presence flag exists
    // at all: unlike an enum, a number has no spare slot.
    frame.events.difficulty[0] = Some(0.0);
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let present = frame_buffer::decode(&words);
    assert_eq!(
        present.events.has_difficulty[0], 1.0,
        "a reported zero must not read as absent"
    );
    assert_eq!(present.events.difficulty[0], 0.0);
}

#[test]
fn carries_the_tri_state_booleans_through_unflattened() {
    let state = fixture(17.0);
    let mut frame = render_frame::build(&state, &RenderFrameOptions::default());
    with_event(&mut frame, MatchEventKind::Shot);
    for value in [0i64, 1, 2] {
        frame.events.jumping[0] = value;
        frame.events.on_target[0] = value;
        let mut words = Vec::new();
        frame_buffer::encode(&frame, &mut words);
        let decoded = frame_buffer::decode(&words);
        assert_eq!(
            decoded.events.jumping[0], value as f64,
            "a tri-state must survive the crossing"
        );
        assert_eq!(decoded.events.on_target[0], value as f64);
    }
}

#[test]
fn uses_slot_zero_for_none_so_a_one_based_index_needs_no_flag() {
    let state = fixture(17.0);
    let mut frame = render_frame::build(&state, &RenderFrameOptions::default());
    frame.possession.owner = None;
    frame.possession.owner_team = None;
    frame.control.pass_target = None;
    frame.control.charge_kind = None;

    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let loose = frame_buffer::decode(&words);
    assert_eq!(loose.scalars[23], 0.0, "a loose ball has no owner slot"); // possession_owner
    assert_eq!(loose.scalars[24], 0.0); // possession_owner_team
    assert_eq!(loose.scalars[27], 0.0); // control_pass_target
    assert_eq!(loose.scalars[28], 0.0); // control_charge_kind

    frame.possession.owner = Some(3);
    frame.possession.owner_team = Some(Team::Away);
    frame.control.pass_target = Some(5);
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let held = frame_buffer::decode(&words);
    assert_eq!(held.scalars[23], 3.0);
    assert_eq!(held.scalars[24], frame_buffer::team_code(Team::Away));
    assert_eq!(held.scalars[27], 5.0);
}

#[test]
fn reports_whether_combat_telegraphs_were_dropped_instead_of_hiding_it() {
    let state = fixture(17.0);
    let mut frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    assert!(!frame_buffer::decode(&words).combat_present);

    // This layout does not carry the nested combat model. A frame that HAD
    // one must say so, so a renderer can refuse a half-frame rather than
    // quietly draw no telegraphs.
    frame.combat = Some(render_frame::FrameCombatModel {
        enabled: true,
        players: Vec::new(),
    });
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    assert!(
        frame_buffer::decode(&words).combat_present,
        "a dropped combat model must be disclosed in the header"
    );
}

#[test]
fn crosses_the_roster_once_with_its_ids_and_names() {
    let state = fixture(17.0);
    let roster = render_frame::roster(&state);
    let (words, strings) = frame_buffer::encode_roster(&roster);
    let decoded = frame_buffer::decode_roster(&words, &strings);

    assert_eq!(decoded.count, roster.count);
    assert_eq!(
        words[3],
        words.len() as f64,
        "total_words must size the roster block too"
    );
    for index in 0..roster.count {
        assert_eq!(decoded.ids[index], roster.ids[index]);
        assert_eq!(decoded.names[index], roster.names[index]);
        assert_eq!(
            decoded.presentation_ids[index],
            roster.presentation_ids[index]
        );
        assert_eq!(decoded.loadout_ids[index], roster.loadout_ids[index]);
        assert_eq!(
            decoded.fields.team[index],
            frame_buffer::team_code(roster.teams[index])
        );
        assert_eq!(
            decoded.fields.is_keeper[index],
            if roster.is_keeper[index] { 1.0 } else { 0.0 }
        );
        assert!((decoded.fields.radius[index] - roster.radius[index]).abs() < 1e-9);
        assert!(
            (decoded.fields.species_color_r[index] - roster.species_color[index][0]).abs() < 1e-9
        );
    }
}

#[test]
fn refuses_a_roster_string_it_could_not_parse_back() {
    let state = fixture(17.0);
    let mut roster = render_frame::roster(&state);
    roster.names[0] = "Bad\nName".to_string();
    assert!(
        std::panic::catch_unwind(|| frame_buffer::encode_roster(&roster)).is_err(),
        "a newline in a roster string would make the blob unparseable"
    );

    // EVERY string column, not just the ones a change happened to touch. The
    // `id` guard is as real as the `name` one and had no direct test of its
    // own, which is exactly how a guard ends up deleted by someone who
    // checked whether anything covered it.
    let mut roster = render_frame::roster(&state);
    roster.ids[0] = "bad\nid".to_string();
    assert!(
        std::panic::catch_unwind(|| frame_buffer::encode_roster(&roster)).is_err(),
        "a newline in a roster id would make the blob unparseable"
    );

    let mut roster = render_frame::roster(&state);
    roster.presentation_ids[0] = "bad\npresentation".to_string();
    assert!(
        std::panic::catch_unwind(|| frame_buffer::encode_roster(&roster)).is_err(),
        "and so would one in a presentation id"
    );

    let mut roster = render_frame::roster(&state);
    roster.loadout_ids[0] = Some("bad\nloadout".to_string());
    assert!(
        std::panic::catch_unwind(|| frame_buffer::encode_roster(&roster)).is_err(),
        "the new #447 string columns are held to the same no-newline rule"
    );

    let mut roster = render_frame::roster(&state);
    roster.presentation_ids[0] = String::new();
    assert!(
        std::panic::catch_unwind(|| frame_buffer::encode_roster(&roster)).is_err(),
        "an empty presentation id is how absence is spelt for LOADOUTS only; \
         a presentation is never absent and must not silently become one"
    );
}

/// THE KEEPER RULE, CARRIED HONESTLY ACROSS THE WIRE (#447).
///
/// `gc-data` states it and `gc-data/tests/players.rs` enforces it; this is
/// the assertion that it survives the encode/decode round trip as an ABSENCE
/// rather than as some stand-in value. Non-vacuous in both directions: the
/// fixture roster is required to contain at least one keeper and at least one
/// outfielder, so neither branch can pass by never being reached.
#[test]
fn carries_a_keepers_missing_loadout_as_an_absence_not_a_sentinel() {
    let state = fixture(17.0);
    let roster = render_frame::roster(&state);
    let (words, strings) = frame_buffer::encode_roster(&roster);
    let decoded = frame_buffer::decode_roster(&words, &strings);

    let mut keepers = 0;
    let mut outfielders = 0;
    for index in 0..roster.count {
        let authored = gc_data::players::get(&roster.ids[index]).expect("authored player");
        assert_eq!(
            decoded.presentation_ids[index], authored.presentation_id,
            "slot {index} must carry the presentation gc-data authored"
        );
        if roster.is_keeper[index] {
            keepers += 1;
            assert_eq!(
                decoded.loadout_ids[index], None,
                "a keeper carries nothing, and that must decode as None"
            );
        } else {
            outfielders += 1;
            assert_eq!(
                decoded.loadout_ids[index].as_deref(),
                authored.loadout_id,
                "an outfielder must carry exactly the loadout gc-data authored"
            );
            assert!(
                decoded.loadout_ids[index].is_some(),
                "an outfielder's loadout must not decode as an absence"
            );
        }
    }
    assert!(keepers >= 2, "both fixture teams field a keeper");
    assert!(
        outfielders >= 8,
        "and the rest of both rosters are outfielders"
    );
}

#[test]
fn reuses_the_callers_array_and_truncates_it_to_this_frame() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let mut words = Vec::new();
    frame_buffer::encode(&frame, &mut words);
    let length = words.len();

    // A stale tail from a longer previous frame is indistinguishable from
    // live data once a reader sizes its view off the header.
    words.push(12345.0);
    words.push(67890.0);
    let ptr_before = words.as_ptr();
    frame_buffer::encode(&frame, &mut words);
    assert_eq!(
        words.as_ptr(),
        ptr_before,
        "the caller's array must be reused, not reallocated"
    );
    assert_eq!(
        words.len(),
        length,
        "a reused array must be truncated to this frame's length"
    );
}

#[test]
fn does_not_perturb_the_simulation() {
    let state = fixture(17.0);
    // Compares the whole `MatchState` directly rather than through
    // `match_snapshot::hash(match_snapshot::capture(...))` — see the
    // equivalent note in `tests/frame.rs`.
    let before = state.clone();
    let mut words = Vec::new();
    frame_buffer::encode(
        &render_frame::build(&state, &RenderFrameOptions::default()),
        &mut words,
    );
    let _ = frame_buffer::encode_roster(&render_frame::roster(&state));
    assert_eq!(
        state, before,
        "serialising a frame must not touch the state it read"
    );
}

#[test]
fn numbers_every_enum_from_one_leaving_zero_for_absent() {
    fn dense_from_one(codes: &[f64], what: &str) {
        let mut seen = codes.to_vec();
        seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for &code in &seen {
            assert!(code >= 1.0, "{what} must not use the absent code");
        }
        for window in seen.windows(2) {
            assert!(window[0] != window[1], "{what} has a duplicate code");
        }
        let highest = seen.last().copied().unwrap_or(0.0);
        assert_eq!(
            highest,
            seen.len() as f64,
            "{what} must be numbered densely from one"
        );
    }

    dense_from_one(
        &[
            frame_buffer::team_code(Team::Home),
            frame_buffer::team_code(Team::Away),
        ],
        "TEAM",
    );
    dense_from_one(
        &[
            frame_buffer::species_shape_code(Shape::Round),
            frame_buffer::species_shape_code(Shape::Broad),
            frame_buffer::species_shape_code(Shape::Angular),
            frame_buffer::species_shape_code(Shape::Cluster),
        ],
        "SPECIES_SHAPE",
    );
    dense_from_one(
        &[
            frame_buffer::charge_kind_code(render_frame::RenderChargeKind::Shot),
            frame_buffer::charge_kind_code(render_frame::RenderChargeKind::Pass),
        ],
        "CHARGE_KIND",
    );
    dense_from_one(
        &[
            frame_buffer::pose_source_code(PlayerPoseSource::Soccer),
            frame_buffer::pose_source_code(PlayerPoseSource::Combat),
            frame_buffer::pose_source_code(PlayerPoseSource::Locomotion),
        ],
        "POSE_SOURCE",
    );
    dense_from_one(
        &[
            frame_buffer::aerial_style_code(AerialStyle::LegControl),
            frame_buffer::aerial_style_code(AerialStyle::ChestControl),
            frame_buffer::aerial_style_code(AerialStyle::Volley),
            frame_buffer::aerial_style_code(AerialStyle::Header),
            frame_buffer::aerial_style_code(AerialStyle::Bicycle),
        ],
        "AERIAL_STYLE",
    );
    dense_from_one(
        &[
            frame_buffer::aerial_outcome_code(AerialOutcome::Clean),
            frame_buffer::aerial_outcome_code(AerialOutcome::Heavy),
            frame_buffer::aerial_outcome_code(AerialOutcome::Miss),
        ],
        "AERIAL_OUTCOME",
    );
    dense_from_one(
        &[
            frame_buffer::save_style_code(SaveStyle::Spread),
            frame_buffer::save_style_code(SaveStyle::Central),
            frame_buffer::save_style_code(SaveStyle::Stretch),
        ],
        "SAVE_STYLE",
    );
    dense_from_one(
        &[
            frame_buffer::keeper_state_code(KeeperBehaviorState::Base),
            frame_buffer::keeper_state_code(KeeperBehaviorState::Advance),
            frame_buffer::keeper_state_code(KeeperBehaviorState::Contain),
            frame_buffer::keeper_state_code(KeeperBehaviorState::Set),
            frame_buffer::keeper_state_code(KeeperBehaviorState::Retreat),
            frame_buffer::keeper_state_code(KeeperBehaviorState::Recover),
        ],
        "KEEPER_STATE",
    );
    dense_from_one(
        &[
            frame_buffer::shot_type_code(KeeperShotType::Ground),
            frame_buffer::shot_type_code(KeeperShotType::Chip),
        ],
        "SHOT_TYPE",
    );

    // POSE_ID and EVENT_KIND: every code the forward direction can produce,
    // recovered by decoding every candidate code and keeping the ones that
    // round-trip. `pose_id_round_trips_every_member` /
    // `event_kind_round_trips_every_member` in `src/frame_buffer.rs`'s own
    // `#[cfg(test)]` module already prove each of the 32/25 codes
    // round-trips explicitly; this reuses that density check on the same
    // public `*_from_code` functions from the external test crate.
    let pose_codes: Vec<f64> = (1..=64)
        .filter_map(|code| frame_buffer::pose_id_from_code(code as f64))
        .map(frame_buffer::pose_id_code)
        .collect();
    dense_from_one(&pose_codes, "POSE_ID");
    assert_eq!(pose_codes.len(), 32);

    let event_codes: Vec<f64> = (1..=64)
        .filter_map(|code| frame_buffer::event_kind_from_code(code as f64))
        .map(frame_buffer::event_kind_code)
        .collect();
    dense_from_one(&event_codes, "EVENT_KIND");
    // #489: 25 -> 26 for the new `TackleMiss` event kind (wire code 26).
    assert_eq!(event_codes.len(), 26);
}
