//! Differential test of `brain` and `keeper` against the real Lua
//! implementation (README rule 5.9, `v2/tools/lua_reference/README.md`).
//!
//! `tests/fixtures/brain_keeper_lua_reference.txt` is the captured stdout of
//! running the real Lua `sim/brain.lua` and `sim/keeper.lua` under headless
//! `love` (no display, no `xvfb`), via a scratch `conf.lua`/`main.lua`
//! harness built per that README (not committed — scratch dirs are
//! session-local). Every case below reconstructs the identical inputs in
//! Rust and asserts the output matches the captured Lua value bit-for-bit:
//! floats are parsed from their captured `%.17g` text and compared via
//! `to_bits()`, so this is a comparison of IEEE 754 bit patterns, not
//! decimal text.
//!
//! Coverage, and why: `brain::select_scored_option` and
//! `brain::decide_carrier` are this crate's only RNG consumers among the
//! seven modules ported alongside this file, so every RNG-consuming branch
//! is exercised — zero temperature (exact argmax, no roll), infinite
//! temperature (uniform, one roll), and ordinary finite temperature softmax
//! (`exp`, weighted sum, `sample * total`, cumulative selection) — across
//! two, three, and five option sets and a spread of seeds including the
//! sharp-composure boundary from `outfield_decision_spec.lua`. `keeper`
//! carries no RNG but sits on the determinism path through
//! `deterministic_math::negative_log_one_minus`
//! (`keeper::travel_time`, exercised across seven distance/speed/friction
//! combinations spanning near-zero and near-the-0.95-cutoff ratios), plus
//! the downstream chip-solving arithmetic
//! (`chip_launch`/`committed_chip_launch`/`goal_line_height`) and the
//! `sqrt`-based positioning geometry (`arc_target`/`base_target`).

use gc_core::vec2::Vec2;
use gc_sim::brain;
use gc_sim::keeper;
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/brain_keeper_lua_reference.txt");

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

fn expect_f64(reference: &IndexMap<&str, &str>, key: &str) -> f64 {
    expect(reference, key)
        .parse()
        .unwrap_or_else(|_| panic!("reference value for {key} is not a float"))
}

/// Assert bit-exact equality against a captured `%.17g` Lua reference value.
/// `%.17g` round-trips binary64 exactly, so parsing it back gives the
/// identical f64 the Lua computation produced; comparing `.to_bits()`
/// (rather than `==`) also distinguishes `-0.0` from `0.0`, matching the
/// lua_reference README's guidance.
fn assert_matches_reference(actual: f64, reference: &IndexMap<&str, &str>, key: &str) {
    let expected = expect_f64(reference, key);
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "{key}: expected bits of {expected} ({:x}), got {actual} ({:x})",
        expected.to_bits(),
        actual.to_bits(),
    );
}

fn option(id: &str, kind: &str, score: f64) -> brain::BrainScoredOption {
    brain::BrainScoredOption {
        id: id.to_string(),
        kind: kind.to_string(),
        score,
        payload: None,
        reference: None,
    }
}

#[test]
fn select_scored_option_matches_the_reference_lua_for_a_moderate_temperature() {
    let reference = reference();
    let options = vec![
        option("short", "pass", 1.0),
        option("through", "pass", 1.1),
        option("carry", "dribble", 0.9),
    ];
    let (selected, next_state) =
        brain::select_scored_option(&options, 0.8, gc_core::rng::seed(904.0));
    assert_eq!(selected.id, expect(&reference, "select_a.id"));
    assert_eq!(
        next_state.to_string(),
        expect(&reference, "select_a.next_state")
    );
}

#[test]
fn select_scored_option_matches_the_reference_lua_for_infinite_temperature() {
    let reference = reference();
    let options = vec![
        option("a_low", "pass", -100.0),
        option("z_best", "pass", 100.0),
    ];
    let (selected, next_state) =
        brain::select_scored_option(&options, f64::INFINITY, gc_core::rng::seed(1.0));
    assert_eq!(selected.id, expect(&reference, "select_b.id"));
    assert_eq!(
        next_state.to_string(),
        expect(&reference, "select_b.next_state")
    );
}

#[test]
fn decide_carrier_matches_the_reference_lua_at_zero_composure_under_pressure() {
    let reference = reference();
    let options = vec![option("best", "pass", 10.0), option("other", "pass", 9.0)];
    let (selected, next_state) =
        brain::decide_carrier(&options, 0.0, 1.0, 2.0, gc_core::rng::seed(15.0));
    assert_eq!(selected.id, expect(&reference, "carrier_c.id"));
    assert_eq!(
        next_state.to_string(),
        expect(&reference, "carrier_c.next_state")
    );
}

#[test]
fn decide_carrier_matches_the_reference_lua_across_a_seed_spread_at_the_sharp_boundary() {
    let reference = reference();
    let options = vec![
        option("best", "dribble", 10.0),
        option("second", "pass", 9.0),
    ];
    for seed in [1000.0, 5000.0, 50000.0, 150000.0, 199997.0] {
        let (selected, next_state) =
            brain::decide_carrier(&options, 0.79, 1.0, 3.0, gc_core::rng::seed(seed));
        let seed_int = seed as i64;
        assert_eq!(
            selected.id,
            expect(&reference, &format!("carrier_d[{seed_int}].id"))
        );
        assert_eq!(
            next_state.to_string(),
            expect(&reference, &format!("carrier_d[{seed_int}].next_state"))
        );
    }
}

#[test]
fn select_scored_option_matches_the_reference_lua_for_a_five_option_softmax() {
    let reference = reference();
    let options = vec![
        option("p1", "pass", 4.2),
        option("p2", "pass", -1.5),
        option("p3", "shoot", 7.75),
        option("p4", "cross", 2.0),
        option("p5", "dribble", 0.0),
    ];
    for seed in [7.0, 42.0, 20_260_806.0] {
        let (selected, next_state) =
            brain::select_scored_option(&options, 1.35, gc_core::rng::seed(seed));
        let seed_int = seed as i64;
        assert_eq!(
            selected.id,
            expect(&reference, &format!("select_e[{seed_int}].id"))
        );
        assert_eq!(
            next_state.to_string(),
            expect(&reference, &format!("select_e[{seed_int}].next_state"))
        );
    }
}

#[test]
fn travel_time_matches_the_reference_lua_across_ordinary_and_near_cutoff_ratios() {
    let reference = reference();
    let cases: &[(f64, f64, f64)] = &[
        (200.0, 500.0, 0.3),
        (260.0, 500.0, 0.3),
        (180.0, 500.0, 0.3),
        (248.0, 500.0, 0.3),
        (100.0, 320.0, 1.2),
        (400.0, 900.0, 0.05),
        (5.0, 50.0, 0.9),
    ];
    for (index, (distance, speed, friction)) in cases.iter().enumerate() {
        let key = format!("travel_time[{}]", index + 1);
        let actual = keeper::travel_time(*distance, *speed, *friction).unwrap_or_else(|| {
            panic!("expected travel_time({distance}, {speed}, {friction}) to be feasible")
        });
        assert_matches_reference(actual, &reference, &key);
    }
}

fn away_goal() -> keeper::Rect {
    keeper::Rect {
        x: 960.0,
        y: 215.0,
        w: 30.0,
        h: 110.0,
    }
}

fn chip_context(keeper_x: f64, keeper_clearance: f64) -> keeper::KeeperChipContext {
    keeper::KeeperChipContext {
        origin: Vec2::new(700.0, 270.0),
        target: Vec2::new(960.0, 270.0),
        keeper_pos: Vec2::new(keeper_x, 270.0),
        defending_team: keeper::Team::Away,
        goal: away_goal(),
        horizontal_speed: 500.0,
        friction: 0.3,
        gravity: 900.0,
        keeper_clearance,
        crossbar: 70.0,
        desired_goal_height: 65.0,
    }
}

#[test]
fn chip_launch_matches_the_reference_lua_for_a_deep_and_an_advanced_keeper() {
    let reference = reference();
    let deep = keeper::chip_launch(&chip_context(900.0, 60.0)).expect("deep chip must be feasible");
    assert_matches_reference(deep, &reference, "chip_launch.deep");

    let advanced =
        keeper::chip_launch(&chip_context(880.0, 60.0)).expect("advanced chip must be feasible");
    assert_matches_reference(advanced, &reference, "chip_launch.advanced");
}

#[test]
fn chip_launch_and_committed_chip_launch_match_the_reference_lua_for_an_infeasible_keeper_plane() {
    let reference = reference();
    let context = chip_context(900.0, 100.0);
    assert_eq!(
        keeper::chip_launch(&context),
        None,
        "keeper_clearance=100 must be infeasible"
    );

    let vz = keeper::committed_chip_launch(&context);
    assert_matches_reference(vz, &reference, "committed_chip_launch.infeasible");

    let direction = context.target.sub(context.origin).normalized();
    let height = keeper::goal_line_height(&keeper::KeeperTrajectoryContext {
        origin: context.origin,
        direction,
        horizontal_speed: context.horizontal_speed,
        vertical_speed: vz,
        defending_team: context.defending_team,
        goal: context.goal,
        friction: context.friction,
        gravity: context.gravity,
    })
    .expect("goal_line_height must resolve for this trajectory");
    assert_matches_reference(height, &reference, "goal_line_height.infeasible");
}

#[test]
fn committed_chip_launch_matches_the_reference_lua_for_a_friction_bound_low_lob() {
    let reference = reference();
    let mut context = chip_context(900.0, 60.0);
    context.horizontal_speed = 50.0;
    assert_eq!(
        keeper::chip_launch(&context),
        None,
        "a 50px/s ball must not reach the goal"
    );
    let vz = keeper::committed_chip_launch(&context);
    assert_matches_reference(vz, &reference, "committed_chip_launch.low_lob");
}

#[test]
fn reaction_reach_and_commit_lead_match_the_reference_lua() {
    let reference = reference();
    assert_matches_reference(
        keeper::reaction_reach(100.0, 0.37, 0.32),
        &reference,
        "reaction_reach.a",
    );
    assert_matches_reference(
        keeper::reaction_reach(60.0, 1.0, 0.18),
        &reference,
        "reaction_reach.b",
    );
    assert_matches_reference(keeper::commit_lead(0.37, 0.24), &reference, "commit_lead.a");
}

#[test]
fn arc_target_and_base_target_match_the_reference_lua_sqrt_based_geometry() {
    let reference = reference();
    let context = keeper::KeeperPositionContext {
        keeper_pos: Vec2::new(12.0, 270.0),
        ball_pos: Vec2::new(233.0, 341.0),
        goal: keeper::Rect {
            x: -30.0,
            y: 215.0,
            w: 30.0,
            h: 110.0,
        },
        team: keeper::Team::Home,
        aggression: 57.0,
        in_1v1: false,
    };
    let arc = keeper::arc_target(&context);
    assert_matches_reference(arc.x, &reference, "arc_target.x");
    assert_matches_reference(arc.y, &reference, "arc_target.y");

    let base = keeper::base_target(&context);
    assert_matches_reference(base.x, &reference, "base_target.x");
    assert_matches_reference(base.y, &reference, "base_target.y");
}
