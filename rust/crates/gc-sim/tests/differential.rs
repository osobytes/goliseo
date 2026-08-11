//! Differential test against the reference Lua implementation (README rule
//! 5.9, `v2/tools/lua_reference/README.md`), required for `input_frame` (the
//! wire format) and `network_conditions` (uses `core.rng`, also on the
//! determinism path).
//!
//! `tests/fixtures/network_input_frame_lua_reference.txt` is the captured
//! stdout of running the real Lua `sim/input_frame.lua` and
//! `sim/network_conditions.lua` under headless `love` (no display, no
//! `xvfb`), via a scratch `conf.lua`/`main.lua` harness built per that
//! README (not committed — scratch dirs are session-local). Every case
//! below reconstructs the identical inputs in Rust and asserts the output
//! matches the captured Lua line byte-for-byte. Degenerate cases covered:
//! zero, negative axis values, values above/at the quantization bound,
//! non-integer axis inputs the Lua floors/rounds, an empty (all-neutral)
//! frame, and the largest tick/sample values the wire format admits.
//!
//! Everything printed here is an integer or an ASCII wire string — nothing
//! floating-point crosses the Lua/Rust boundary in observable output (RNG
//! rolls are consumed entirely inside `network_conditions` and only
//! influence integer decisions: drop/keep, an integer jitter offset), so
//! plain string/integer equality is bit-exact comparison; there is no `%g`
//! rounding ambiguity to route around via `.to_bits()`.

use gc_data::network_profiles::{self, NetworkProfileName};
use gc_sim::input_frame::{self, EdgeAction, HeldAction, InputSample, InputSampleOptions};
use gc_sim::network_conditions::{self, NetworkConditions, NetworkDelivery, NetworkProfile};
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/network_input_frame_lua_reference.txt");

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

#[test]
fn quantize_axis_matches_the_reference_lua_for_ordinary_and_degenerate_axis_values() {
    let reference = reference();
    let cases: &[(&str, f64)] = &[
        ("-2", -2.0),
        ("-1", -1.0),
        ("-0.999", -0.999),
        ("-0.5", -0.5),
        ("-0.0001", -0.0001),
        ("-0", -0.0),
        ("0", 0.0),
        ("0.0001", 0.0001),
        ("0.5", 0.5),
        ("0.999", 0.999),
        ("1", 1.0),
        ("2", 2.0),
    ];
    for (label, x) in cases {
        let key = format!("quantize_axis({label})");
        let expected: i64 = expect(&reference, &key)
            .parse()
            .expect("reference value is an integer");
        assert_eq!(
            input_frame::quantize_axis(*x).unwrap(),
            expected,
            "quantize_axis({x})"
        );
    }
}

#[test]
fn encode_sample_matches_the_reference_lua_for_extrema_and_ordinary_samples() {
    let reference = reference();

    let min = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        move_y: Some(-127),
        held: Some(0),
        edges: Some(0),
    })
    .unwrap();
    assert_eq!(
        input_frame::encode_sample(&min).unwrap(),
        expect(&reference, "encode_sample(min)")
    );

    // held=255 (every held bit, including equipment) combined with
    // edges=63 (every edge bit except equipment_released, which conflicts
    // with an already-held equipment action).
    let max = input_frame::new_sample(InputSampleOptions {
        move_x: Some(127),
        move_y: Some(127),
        held: Some(255),
        edges: Some(63),
    })
    .unwrap();
    assert_eq!(
        input_frame::encode_sample(&max).unwrap(),
        expect(&reference, "encode_sample(max)")
    );

    let zero = InputSample::default();
    assert_eq!(
        input_frame::encode_sample(&zero).unwrap(),
        expect(&reference, "encode_sample(zero)")
    );

    let mixed = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-64),
        move_y: Some(64),
        held: Some(42),
        edges: Some(85),
    })
    .unwrap();
    assert_eq!(
        input_frame::encode_sample(&mixed).unwrap(),
        expect(&reference, "encode_sample(mixed)")
    );
}

fn neutral_slots() -> [InputSample; 8] {
    [input_frame::neutral_sample(); 8]
}

fn mixed_frame() -> input_frame::InputFrame {
    let mut slots = neutral_slots();
    slots[0] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        move_y: Some(64),
        held: Some(HeldAction::Shoot.bit() + HeldAction::Lob.bit()),
        edges: Some(EdgeAction::Shoot.bit()),
    })
    .unwrap();
    slots[3] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-1),
        move_y: Some(-1),
        held: Some(0),
        edges: Some(0),
    })
    .unwrap();
    slots[7] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(127),
        move_y: Some(-64),
        held: Some(HeldAction::Equipment.bit()),
        edges: Some(EdgeAction::EquipmentPressed.bit()),
    })
    .unwrap();
    input_frame::new(305_419_896, Some(slots)).unwrap()
}

#[test]
fn encode_matches_the_reference_lua_for_a_spread_of_frames_including_degenerate_extrema() {
    let reference = reference();

    let empty_frame = input_frame::new(0, Some(neutral_slots())).unwrap();
    let empty_wire = input_frame::encode(&empty_frame).unwrap();
    assert_eq!(empty_wire, expect(&reference, "encode(empty@0)"));

    let max_tick_frame = input_frame::new(input_frame::MAX_TICK, Some(neutral_slots())).unwrap();
    let max_tick_wire = input_frame::encode(&max_tick_frame).unwrap();
    assert_eq!(max_tick_wire, expect(&reference, "encode(empty@max_tick)"));

    let max_slots: [InputSample; 8] = std::array::from_fn(|_| {
        input_frame::new_sample(InputSampleOptions {
            move_x: Some(-127),
            move_y: Some(-127),
            held: Some(127),
            edges: Some(127),
        })
        .unwrap()
    });
    let max_frame = input_frame::new(input_frame::MAX_TICK, Some(max_slots)).unwrap();
    let max_wire = input_frame::encode(&max_frame).unwrap();
    assert_eq!(max_wire, expect(&reference, "encode(max)"));

    let mixed_wire = input_frame::encode(&mixed_frame()).unwrap();
    assert_eq!(mixed_wire, expect(&reference, "encode(mixed)"));

    // Round trip: decode(wire) re-encodes to the identical wire, for every
    // wire above — matches the reference Lua's own round-trip capture.
    for wire in [&empty_wire, &max_tick_wire, &max_wire, &mixed_wire] {
        let decoded = input_frame::decode(wire).unwrap();
        let reencoded = input_frame::encode(&decoded).unwrap();
        let key = format!("roundtrip({wire})");
        assert_eq!(reencoded, expect(&reference, &key));
        assert_eq!(&reencoded, wire);
    }
}

fn nc_sample(move_x: i64, edges: i64) -> InputSample {
    input_frame::new_sample(InputSampleOptions {
        move_x: Some(move_x),
        edges: Some(edges),
        ..Default::default()
    })
    .unwrap()
}

fn nc_outcome(conditions: &mut NetworkConditions) -> String {
    let deliveries = network_conditions::poll(conditions, 1000);
    let counters = network_conditions::counters(conditions);
    let delivery_parts: Vec<String> = deliveries
        .iter()
        .map(|delivery: &NetworkDelivery| {
            let record_parts: Vec<String> = network_conditions::records(delivery)
                .iter()
                .map(|record| {
                    format!(
                        "{}/{}/{}/{}/{}",
                        record.tick,
                        record.sample.move_x,
                        record.sample.move_y,
                        record.sample.held,
                        record.sample.edges
                    )
                })
                .collect();
            format!(
                "{}:{}:{}:{}:{}:{}",
                delivery.source_slot,
                delivery.send_tick,
                delivery.sequence,
                delivery.duplicate_ordinal,
                delivery.arrival_tick,
                record_parts.join(",")
            )
        })
        .collect();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        delivery_parts.join(";"),
        counters.sent,
        counters.delivered,
        counters.independent_lost,
        counters.burst_lost,
        counters.duplicated,
        counters.reordered,
        counters.history_recovered
    )
}

#[test]
fn network_conditions_outcome_matches_the_reference_lua_across_playable_seeds() {
    let reference = reference();
    let playable: NetworkProfile = network_profiles::get(NetworkProfileName::Playable).into();
    for seed in [431.0_f64, 432.0, 7777.0] {
        let mut conditions = network_conditions::new(&playable, seed);
        for tick in 0..40_i64 {
            network_conditions::send(
                &mut conditions,
                tick,
                (tick % 8) + 1,
                tick,
                &nc_sample(tick % 128, 0),
            )
            .unwrap();
        }
        let key = format!("network_outcome(playable,{})", seed as i64);
        assert_eq!(nc_outcome(&mut conditions), expect(&reference, &key));
    }
}

#[test]
fn network_conditions_outcome_matches_the_reference_lua_under_the_stress_profile() {
    let reference = reference();
    let stress: NetworkProfile = network_profiles::get(NetworkProfileName::Stress).into();
    let mut conditions = network_conditions::new(&stress, 20_260_806.0);
    for tick in 0..200_i64 {
        network_conditions::send(
            &mut conditions,
            tick,
            (tick % 8) + 1,
            tick,
            &nc_sample((tick * 37) % 128, tick % 7),
        )
        .unwrap();
    }
    assert_eq!(
        nc_outcome(&mut conditions),
        expect(&reference, "network_outcome(stress,20260806)")
    );
}

#[test]
fn network_conditions_outcome_matches_the_reference_lua_under_the_clean_profile() {
    let reference = reference();
    let clean: NetworkProfile = network_profiles::get(NetworkProfileName::Clean).into();
    let mut conditions = network_conditions::new(&clean, 1.0);
    for tick in 0..10_i64 {
        network_conditions::send(&mut conditions, tick, 1, tick, &nc_sample(tick, 0)).unwrap();
    }
    assert_eq!(
        nc_outcome(&mut conditions),
        expect(&reference, "network_outcome(clean,1)")
    );
}
