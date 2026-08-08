//! Port of `spec/sim/input_frame_spec.lua`.
//!
//! This is the wire format (README §"input_frame.lua is the most important
//! file you will touch"): its bytes go on the network and into rollback
//! re-simulation, so two clients must encode identically. Every assertion
//! from the Lua spec is ported below, plus differential coverage against the
//! reference Lua implementation in `differential.rs`.

use gc_data::players::PlayerData;
use gc_sim::input_frame::{
    self, EdgeAction, HeldAction, InputFixtureRosters, InputFrameErrorCode, InputSample,
    InputSampleOptions, InputSlotAssignment, Team,
};
use indexmap::IndexMap;

fn players_by_id() -> IndexMap<&'static str, PlayerData> {
    gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect()
}

fn assignments() -> [InputSlotAssignment; 8] {
    let player_ids = [
        "zyro_vex",
        "mika_olu",
        "rok_tann",
        "sela_dwin",
        "drell",
        "morv",
        "krag",
        "tox_vren",
    ];
    std::array::from_fn(|i| {
        let index = i as i64 + 1;
        let slot = input_frame::slot(index).expect("valid slot index");
        InputSlotAssignment {
            slot: slot.id,
            team: slot.team,
            player_id: player_ids[i].to_string(),
        }
    })
}

fn fixture_rosters() -> InputFixtureRosters {
    InputFixtureRosters {
        home: vec!["ozzo", "zyro_vex", "mika_olu", "rok_tann", "sela_dwin"]
            .into_iter()
            .map(String::from)
            .collect(),
        away: vec!["gax_oru", "drell", "morv", "krag", "tox_vren"]
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

fn neutral_slots() -> [InputSample; 8] {
    [input_frame::neutral_sample(); 8]
}

#[test]
fn omp1_input_frame_defines_exactly_four_stable_outfield_slots_per_team() {
    let expected = [
        (input_frame::SlotId::Home1, Team::Home, 1),
        (input_frame::SlotId::Home2, Team::Home, 2),
        (input_frame::SlotId::Home3, Team::Home, 3),
        (input_frame::SlotId::Home4, Team::Home, 4),
        (input_frame::SlotId::Away1, Team::Away, 1),
        (input_frame::SlotId::Away2, Team::Away, 2),
        (input_frame::SlotId::Away3, Team::Away, 3),
        (input_frame::SlotId::Away4, Team::Away, 4),
    ];
    assert_eq!(input_frame::SLOT_COUNT, 8);
    assert_eq!(input_frame::FIXTURE_TEAM_SIZE, 5);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).expect("valid slot index");
        assert_eq!(slot.index, index);
        let (expected_id, expected_team, expected_outfield) = expected[(index - 1) as usize];
        assert_eq!(slot.id, expected_id);
        assert_eq!(slot.team, expected_team);
        assert_eq!(slot.outfield_index, expected_outfield);
    }
    assert_eq!(input_frame::slot_index(Team::Home, 4).unwrap(), 4);
    assert_eq!(input_frame::slot_index(Team::Away, 1).unwrap(), 5);
    let err = input_frame::slot(9).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);
}

#[test]
fn omp1_input_frame_maps_canonical_slots_to_unique_non_keeper_roster_players() {
    let by_id = players_by_id();
    let rosters = fixture_rosters();
    let ownership = input_frame::new_ownership(&assignments(), &rosters, &by_id).unwrap();
    assert_eq!(ownership.version, input_frame::VERSION);
    assert_eq!(ownership.rosters.home[0], "ozzo");
    assert_eq!(ownership.rosters.away[0], "gax_oru");
    assert_eq!(ownership.slots.len(), input_frame::SLOT_COUNT as usize);
    assert_eq!(ownership.slots[0].slot, input_frame::SlotId::Home1);
    assert_eq!(ownership.slots[4].team, Team::Away);
    assert_eq!(ownership.slots[7].player_id, "tox_vren");

    let mut legacy = input_frame::copy_ownership(&ownership, &by_id).unwrap();
    legacy.version = 1;
    let legacy_err = input_frame::validate_ownership(&legacy, &by_id).unwrap_err();
    assert_eq!(legacy_err.code, InputFrameErrorCode::UnsupportedVersion);

    let mut duplicate = assignments();
    duplicate[3].player_id = duplicate[0].player_id.clone();
    let err = input_frame::new_ownership(&duplicate, &rosters, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);

    let mut keeper = assignments();
    keeper[0].player_id = "ozzo".to_string();
    let err = input_frame::new_ownership(&keeper, &rosters, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);

    let mut wrong_team = assignments();
    wrong_team[4].team = Team::Home;
    let err = input_frame::new_ownership(&wrong_team, &rosters, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);

    let mut cross_side = assignments();
    cross_side[4].player_id = "zyro_vex".to_string();
    let err = input_frame::new_ownership(&cross_side, &rosters, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);

    let mut unknown_assignment = assignments();
    unknown_assignment[0].player_id = "missing_player".to_string();
    let err = input_frame::new_ownership(&unknown_assignment, &rosters, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);

    let mut unknown_roster = fixture_rosters();
    unknown_roster.away[4] = "missing_player".to_string();
    let err = input_frame::new_ownership(&assignments(), &unknown_roster, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);

    let mut no_keeper = fixture_rosters();
    no_keeper.home[0] = "brakka".to_string();
    let err = input_frame::new_ownership(&assignments(), &no_keeper, &by_id).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);
}

#[test]
fn omp1_input_frame_creates_independent_neutral_samples_for_every_tick_and_slot() {
    let mut frame = input_frame::neutral(120).unwrap();
    assert_eq!(frame.version, input_frame::VERSION);
    assert_eq!(frame.tick, 120);
    assert_eq!(frame.slots.len(), input_frame::SLOT_COUNT as usize);
    for sample in &frame.slots {
        assert_eq!(sample.move_x, 0);
        assert_eq!(sample.move_y, 0);
        assert_eq!(sample.held, 0);
        assert_eq!(sample.edges, 0);
    }
    frame.slots[0].move_x = 20;
    assert_eq!(
        frame.slots[1].move_x, 0,
        "neutral slots do not share a table"
    );
}

#[test]
fn omp1_input_frame_quantizes_movement_with_fixed_saturation_rounding_and_decode_rules() {
    assert_eq!(input_frame::quantize_axis(-2.0).unwrap(), -127);
    assert_eq!(input_frame::quantize_axis(-1.0).unwrap(), -127);
    assert_eq!(input_frame::quantize_axis(-0.5).unwrap(), -64);
    assert_eq!(input_frame::quantize_axis(0.0).unwrap(), 0);
    assert_eq!(input_frame::quantize_axis(-0.0).unwrap(), 0);
    assert_eq!(input_frame::quantize_axis(0.5).unwrap(), 64);
    assert_eq!(input_frame::quantize_axis(1.0).unwrap(), 127);
    assert_eq!(input_frame::quantize_axis(2.0).unwrap(), 127);
    let (move_x, move_y) = input_frame::quantize_move(-0.5, 0.5).unwrap();
    assert_eq!(move_x, -64);
    assert_eq!(move_y, 64);

    assert!((input_frame::dequantize_axis(-127).unwrap() - -1.0).abs() < 1e-6);
    assert!((input_frame::dequantize_axis(64).unwrap() - 64.0 / 127.0).abs() < 1e-6);
    let (decoded_x, decoded_y) = input_frame::dequantize_move(&InputSample {
        move_x: -64,
        move_y: 64,
        held: 0,
        edges: 0,
    })
    .unwrap();
    assert!((decoded_x - -64.0 / 127.0).abs() < 1e-6);
    assert!((decoded_y - 64.0 / 127.0).abs() < 1e-6);

    // The Lua spec writes this as 0/0; f64::NAN is the same value, and clippy
    // rejects the literal division as an always-NaN constant expression.
    let err = input_frame::quantize_axis(f64::NAN).unwrap_err();
    assert_eq!(err.code, InputFrameErrorCode::Malformed);
}

#[test]
fn omp1_input_frame_keeps_supplied_holds_and_one_tick_edges_distinct() {
    let sample = input_frame::new_sample(InputSampleOptions {
        held: Some(
            HeldAction::Shoot.bit() + HeldAction::Sprint.bit() + HeldAction::Equipment.bit(),
        ),
        edges: Some(
            EdgeAction::Shoot.bit() + EdgeAction::Dash.bit() + EdgeAction::EquipmentPressed.bit(),
        ),
        ..Default::default()
    })
    .unwrap();
    assert!(input_frame::is_held(&sample, HeldAction::Shoot).unwrap());
    assert!(input_frame::is_held(&sample, HeldAction::Sprint).unwrap());
    assert!(input_frame::is_held(&sample, HeldAction::Equipment).unwrap());
    assert!(!input_frame::is_held(&sample, HeldAction::Pass).unwrap());
    assert!(input_frame::has_edge(&sample, EdgeAction::Shoot).unwrap());
    assert!(input_frame::has_edge(&sample, EdgeAction::Dash).unwrap());
    assert!(input_frame::has_edge(&sample, EdgeAction::EquipmentPressed).unwrap());
    assert!(!input_frame::has_edge(&sample, EdgeAction::Pass).unwrap());

    let next_sample = input_frame::new_sample(InputSampleOptions {
        held: Some(sample.held),
        edges: Some(0),
        ..Default::default()
    })
    .unwrap();
    assert!(input_frame::is_held(&next_sample, HeldAction::Shoot).unwrap());
    assert!(!input_frame::has_edge(&next_sample, EdgeAction::Shoot).unwrap());
}

#[test]
fn omp1_input_frame_round_trips_the_compact_four_byte_sample_without_omitting_combat_bits() {
    let all_holds = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        move_y: Some(127),
        held: Some(255),
        edges: Some(EdgeAction::EquipmentPressed.bit()),
    })
    .unwrap();
    let hold_wire = input_frame::encode_sample(&all_holds).unwrap();
    assert_eq!(hold_wire, "00feff20");
    assert_eq!(hold_wire.len(), input_frame::MAX_SAMPLE_WIRE_BYTES);
    let decoded_holds = input_frame::decode_sample(&hold_wire).unwrap();
    assert_eq!(decoded_holds.move_x, -127);
    assert_eq!(decoded_holds.move_y, 127);
    assert_eq!(decoded_holds.held, 255);
    assert_eq!(decoded_holds.edges, EdgeAction::EquipmentPressed.bit());

    let all_edges = input_frame::new_sample(InputSampleOptions {
        held: Some(0),
        edges: Some(127),
        ..Default::default()
    })
    .unwrap();
    let edge_wire = input_frame::encode_sample(&all_edges).unwrap();
    assert_eq!(edge_wire, "7f7f007f");
    assert_eq!(input_frame::decode_sample(&edge_wire).unwrap().edges, 127);

    for wire in ["7F7f007f", "7f7f007", "7f7f0080", "zzzzzzzz"] {
        let err = input_frame::decode_sample(wire).unwrap_err();
        assert_eq!(err.code, InputFrameErrorCode::Malformed);
    }
}

#[test]
fn omp1_input_frame_encodes_and_decodes_one_byte_for_byte_canonical_frame() {
    let mut slots = neutral_slots();
    slots[0] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        move_y: Some(64),
        held: Some(HeldAction::Shoot.bit() + HeldAction::Lob.bit()),
        edges: Some(EdgeAction::Shoot.bit()),
    })
    .unwrap();
    slots[7] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(127),
        move_y: Some(-64),
        held: Some(HeldAction::Equipment.bit()),
        edges: Some(EdgeAction::EquipmentPressed.bit()),
    })
    .unwrap();
    let frame = input_frame::new(42, Some(slots)).unwrap();
    let wire = input_frame::encode(&frame).unwrap();
    assert_eq!(
        wire,
        "2|42|-127,64,17,1|0,0,0,0|0,0,0,0|0,0,0,0|0,0,0,0|0,0,0,0|0,0,0,0|127,-64,128,32"
    );

    let decoded = input_frame::decode(&wire).unwrap();
    let reencoded = input_frame::encode(&decoded).unwrap();
    assert_eq!(reencoded, wire);
    assert_eq!(decoded.tick, 42);
    assert_eq!(decoded.slots[0].move_x, -127);
    assert_eq!(decoded.slots[7].edges, EdgeAction::EquipmentPressed.bit());
}

#[test]
fn omp1_input_frame_rejects_malformed_noncanonical_and_oversized_frame_data() {
    // `frame.slots[9] = ...` (a 9th canonical slot) is unrepresentable: the
    // Rust `InputFrame.slots` is a `[InputSample; 8]`, so "too many slots"
    // cannot even be constructed, let alone need a runtime rejection. That
    // sub-case is structurally impossible here rather than merely untested.

    let sample = input_frame::new_sample(InputSampleOptions {
        held: Some(256),
        ..Default::default()
    });
    assert_eq!(sample.unwrap_err().code, InputFrameErrorCode::Malformed);
    let sample = input_frame::new_sample(InputSampleOptions {
        edges: Some(128),
        ..Default::default()
    });
    assert_eq!(sample.unwrap_err().code, InputFrameErrorCode::Malformed);

    let invalid_combinations = [
        InputSampleOptions {
            edges: Some(EdgeAction::EquipmentPressed.bit()),
            ..Default::default()
        },
        InputSampleOptions {
            held: Some(HeldAction::Equipment.bit()),
            edges: Some(EdgeAction::EquipmentReleased.bit()),
            ..Default::default()
        },
        InputSampleOptions {
            held: Some(HeldAction::Equipment.bit()),
            edges: Some(EdgeAction::EquipmentPressed.bit() + EdgeAction::EquipmentReleased.bit()),
            ..Default::default()
        },
    ];
    for invalid in invalid_combinations {
        let rejected = input_frame::new_sample(invalid);
        assert_eq!(rejected.unwrap_err().code, InputFrameErrorCode::Malformed);
    }

    let wire = input_frame::encode(&input_frame::neutral(0).unwrap()).unwrap();
    let old_wire = format!("1{}", &wire[1..]);
    let old_err = input_frame::decode(&old_wire).unwrap_err();
    assert_eq!(old_err.code, InputFrameErrorCode::UnsupportedVersion);
    let bad_version = format!("01{}", &wire[1..]);
    let decode_err = input_frame::decode(&bad_version).unwrap_err();
    assert_eq!(decode_err.code, InputFrameErrorCode::Malformed);
    let negative_zero_wire = wire.replacen("0,0,0,0", "-0,0,0,0", 1);
    let decode_err = input_frame::decode(&negative_zero_wire).unwrap_err();
    assert_eq!(decode_err.code, InputFrameErrorCode::Malformed);
    let decode_err = input_frame::decode(&format!("{wire}x")).unwrap_err();
    assert_eq!(decode_err.code, InputFrameErrorCode::Malformed);

    let mut maximum_slots = neutral_slots();
    for slot in &mut maximum_slots {
        *slot = input_frame::new_sample(InputSampleOptions {
            move_x: Some(-127),
            move_y: Some(-127),
            held: Some(127),
            edges: Some(127),
        })
        .unwrap();
    }
    let maximum_wire =
        input_frame::encode(&input_frame::new(input_frame::MAX_TICK, Some(maximum_slots)).unwrap())
            .unwrap();
    assert_eq!(maximum_wire.len(), input_frame::MAX_WIRE_BYTES);
    let decode_err = input_frame::decode(&format!("{maximum_wire}x")).unwrap_err();
    assert_eq!(decode_err.code, InputFrameErrorCode::WireTooLarge);
}
