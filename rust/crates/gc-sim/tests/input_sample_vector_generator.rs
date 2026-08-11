//! Generates the cross-language vector that pins `@gc/input`'s TypeScript
//! port of `input_frame`'s quantization and bit-packing rules
//! (`ts/packages/input/fixtures/input_sample_vector.txt`).
//!
//! `gc_sim::input_frame` is the canonical, already-differential-tested
//! implementation (see `input_frame.rs`/`differential.rs` in this
//! directory), so unlike `tools/lua_reference` (whose vectors were captured
//! by running the Lua implementation this simulation was originally
//! validated against), this generator captures reference values from Rust
//! directly -- Rust is the source of truth this vector pins TypeScript
//! against, per ARCHITECTURE.md §1.2's rule that a module duplicated into a
//! second language must be pinned by a shared vector file.
//!
//! Not a CI-asserted test: it PRINTS the vector to stdout for a human to
//! capture into the committed fixture, same workflow as the Lua-capture
//! harness. Run:
//!
//!   cargo test -p gc-sim --test input_sample_vector_generator -- --ignored --nocapture

use gc_sim::input_frame::{self, EdgeAction, HeldAction, InputSampleOptions};

struct Case {
    name: &'static str,
    raw_move_x: f64,
    raw_move_y: f64,
    held: &'static [HeldAction],
    edges: &'static [EdgeAction],
}

fn held_name(action: HeldAction) -> &'static str {
    match action {
        HeldAction::Shoot => "shoot",
        HeldAction::Pass => "pass",
        HeldAction::Sprint => "sprint",
        HeldAction::Jockey => "jockey",
        HeldAction::Lob => "lob",
        HeldAction::AerialStrike => "aerial_strike",
        HeldAction::AerialAcrobatic => "aerial_acrobatic",
        HeldAction::Equipment => "equipment",
    }
}

fn edge_name(action: EdgeAction) -> &'static str {
    match action {
        EdgeAction::Shoot => "shoot",
        EdgeAction::Pass => "pass",
        EdgeAction::Switch => "switch",
        EdgeAction::Dash => "dash",
        EdgeAction::Dodge => "dodge",
        EdgeAction::EquipmentPressed => "equipment_pressed",
        EdgeAction::EquipmentReleased => "equipment_released",
    }
}

fn pack_held(actions: &[HeldAction]) -> i64 {
    actions.iter().fold(0, |acc, a| acc | a.bit())
}

fn pack_edges(actions: &[EdgeAction]) -> i64 {
    actions.iter().fold(0, |acc, a| acc | a.bit())
}

fn joined_held(actions: &[HeldAction]) -> String {
    actions
        .iter()
        .map(|a| held_name(*a))
        .collect::<Vec<_>>()
        .join(",")
}

fn joined_edges(actions: &[EdgeAction]) -> String {
    actions
        .iter()
        .map(|a| edge_name(*a))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
#[ignore]
fn generate_input_sample_vector() {
    use EdgeAction::{
        Dash, Dodge, EquipmentPressed, EquipmentReleased, Pass as EPass, Shoot as EShoot, Switch,
    };
    use HeldAction::{
        AerialAcrobatic, AerialStrike, Equipment, Jockey, Lob, Pass as HPass, Shoot as HShoot,
        Sprint,
    };

    let cases: Vec<Case> = vec![
        Case {
            name: "neutral",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[],
        },
        Case {
            name: "axis_positive_half",
            raw_move_x: 0.5,
            raw_move_y: 0.0,
            held: &[],
            edges: &[],
        },
        Case {
            name: "axis_negative_half",
            raw_move_x: -0.5,
            raw_move_y: 0.0,
            held: &[],
            edges: &[],
        },
        Case {
            name: "axis_clamped_high",
            raw_move_x: 2.5,
            raw_move_y: 1.0,
            held: &[],
            edges: &[],
        },
        Case {
            name: "axis_clamped_low",
            raw_move_x: -3.7,
            raw_move_y: -1.0,
            held: &[],
            edges: &[],
        },
        Case {
            name: "axis_rounds_to_zero",
            raw_move_x: 0.001,
            raw_move_y: -0.001,
            held: &[],
            edges: &[],
        },
        Case {
            name: "held_shoot",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[HShoot],
            edges: &[],
        },
        Case {
            name: "held_pass",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[HPass],
            edges: &[],
        },
        Case {
            name: "held_sprint",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[Sprint],
            edges: &[],
        },
        Case {
            name: "held_jockey",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[Jockey],
            edges: &[],
        },
        Case {
            name: "held_lob",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[Lob],
            edges: &[],
        },
        Case {
            name: "held_aerial_strike",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[AerialStrike],
            edges: &[],
        },
        Case {
            name: "held_aerial_acrobatic",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[AerialAcrobatic],
            edges: &[],
        },
        Case {
            name: "held_equipment",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[Equipment],
            edges: &[],
        },
        Case {
            name: "edge_shoot",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[EShoot],
        },
        Case {
            name: "edge_pass",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[EPass],
        },
        Case {
            name: "edge_switch",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[Switch],
        },
        Case {
            name: "edge_dash",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[Dash],
        },
        Case {
            name: "edge_dodge",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[Dodge],
        },
        // equipment_pressed alone is only a legal sample while equipment is
        // held (validate_sample forbids pressed && !released && !held).
        Case {
            name: "equipment_pressed",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[Equipment],
            edges: &[EquipmentPressed],
        },
        // A release with equipment NOT held is legal on its own (it was
        // held a moment ago; this sample lands after the release).
        Case {
            name: "equipment_released",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[EquipmentReleased],
        },
        // Press and release inside the same sample, never held: a complete
        // tap that fit inside one tick.
        Case {
            name: "equipment_fast_tap",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[EquipmentPressed, EquipmentReleased],
        },
        Case {
            name: "all_held",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[
                HShoot,
                HPass,
                Sprint,
                Jockey,
                Lob,
                AerialStrike,
                AerialAcrobatic,
                Equipment,
            ],
            edges: &[],
        },
        // All seven edge bits at once, equipment not held: equipment's pair
        // collapses to the same "fast tap" legality as equipment_fast_tap.
        Case {
            name: "all_edges",
            raw_move_x: 0.0,
            raw_move_y: 0.0,
            held: &[],
            edges: &[
                EShoot,
                EPass,
                Switch,
                Dash,
                Dodge,
                EquipmentPressed,
                EquipmentReleased,
            ],
        },
        // Maximal legal combination: every held bit plus every edge bit
        // that can coexist with equipment already held (a repeated press
        // edge is legal; a release edge is not, while held is true).
        Case {
            name: "full_combo",
            raw_move_x: 1.0,
            raw_move_y: -1.0,
            held: &[
                HShoot,
                HPass,
                Sprint,
                Jockey,
                Lob,
                AerialStrike,
                AerialAcrobatic,
                Equipment,
            ],
            edges: &[EShoot, EPass, Switch, Dash, Dodge, EquipmentPressed],
        },
    ];

    println!("# Cross-language vector for gc_sim::input_frame::InputSample's quantization");
    println!("# and bit-packing rules, generated by this file. See its module doc for");
    println!("# the generation command and rationale.");
    println!("#");
    println!("# Format: one field per non-comment, non-blank line, tab-separated:");
    println!("#   field_name<TAB>value");
    println!("# Cases are separated by a blank line, each opened by a `case` line.");
    println!("# `held_actions`/`edge_actions` are comma-joined canonical action names");
    println!("# (empty when none); `move_x`/`move_y`/`held`/`edges` are the expected");
    println!("# InputSample fields `gc_sim::input_frame::new_sample` produced from");
    println!("# quantizing raw_move_x/raw_move_y and packing held_actions/edge_actions'");
    println!("# bits. raw_move_x/raw_move_y use Rust's default f64 Display (shortest");
    println!("# round-trippable decimal) rather than %.17g -- simpler to generate");
    println!("# correctly and an equal-or-stronger round-trip guarantee: every value");
    println!("# here parses back to the identical f64 bit pattern via both Rust's");
    println!("# f64::from_str and JavaScript's Number(). move_x/move_y/held/edges are");
    println!("# plain integers (every one of them is i64 here).");

    for case in &cases {
        let (move_x, move_y) = input_frame::quantize_move(case.raw_move_x, case.raw_move_y)
            .expect("case raw axes are finite");
        let held = pack_held(case.held);
        let edges = pack_edges(case.edges);
        let sample = input_frame::new_sample(InputSampleOptions {
            move_x: Some(move_x),
            move_y: Some(move_y),
            held: Some(held),
            edges: Some(edges),
        })
        .unwrap_or_else(|e| panic!("case {} does not build a valid sample: {}", case.name, e));

        println!();
        println!("case\t{}", case.name);
        println!("raw_move_x\t{}", case.raw_move_x);
        println!("raw_move_y\t{}", case.raw_move_y);
        println!("held_actions\t{}", joined_held(case.held));
        println!("edge_actions\t{}", joined_edges(case.edges));
        println!("move_x\t{}", sample.move_x);
        println!("move_y\t{}", sample.move_y);
        println!("held\t{}", sample.held);
        println!("edges\t{}", sample.edges);
    }
}
