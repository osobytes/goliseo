//! Tests for `gc_sim::passing` — soft-scored receiver selection and the
//! registered distance-to-speed launch curve.
//!
//! What this file used to assert is worth naming, because two of its cases
//! are **deleted rather than repaired**: a teammate behind the passer, and a
//! teammate 84 degrees off the aim, both used to select nobody. That was the
//! hard 60-degree acceptance cone, and deleting it is the point of #491 — a
//! gate turns near-misses into non-passes and non-passes into faith. The
//! replacement claim is `a_teammate_the_old_cone_excluded_is_now_selected`,
//! which asserts the same geometry now resolves.

use gc_core::vec2::Vec2;
use gc_sim::passing::{self, SelectionKnobs};
use gc_sim::tuning::Tuning;

const FROM: Vec2 = Vec2 { x: 0.0, y: 0.0 };
const EAST: Vec2 = Vec2 { x: 1.0, y: 0.0 };

/// The shipped defaults, so the cases read against the balance that ships.
fn knobs() -> SelectionKnobs {
    SelectionKnobs::of(&Tuning::new())
}

/// Eligibility opened wide, for cases about scoring rather than bounds.
fn open_knobs(angular_weight: f64) -> SelectionKnobs {
    SelectionKnobs {
        angular_weight,
        eligible_min: 0.0,
        eligible_max: f64::INFINITY,
    }
}

#[test]
fn the_nearer_of_two_equally_aligned_teammates_wins() {
    let mates = [Vec2::new(100.0, 0.0), Vec2::new(30.0, 0.0)];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &knobs()),
        Some(1)
    );
}

#[test]
fn a_tap_goes_short_the_near_man_beats_a_far_one_dead_on_the_line() {
    let mates = [
        Vec2::new(60.0, 55.0), // near, ~42 degrees off the aim
        Vec2::new(350.0, 0.0), // far, dead on the aim line
    ];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &knobs()),
        Some(0)
    );
}

#[test]
fn a_charged_range_picks_out_the_far_man_on_the_line() {
    let mates = [Vec2::new(60.0, 55.0), Vec2::new(350.0, 0.0)];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, Some(350.0), &knobs()),
        Some(1)
    );
}

/// The deletion of the hard cone, asserted rather than claimed.
///
/// Both of these were `None` before #491: one teammate is 84 degrees off the
/// aim and one is directly behind the passer. The old 60-degree gate saw
/// neither, so a player who aimed a little wide got no pass at all and no
/// feedback about why.
#[test]
fn a_teammate_the_old_cone_excluded_is_now_selected() {
    let wide = [Vec2::new(10.0, 100.0)]; // ~84 degrees off the aim
    assert_eq!(
        passing::select_receiver(FROM, EAST, &wide, None, &knobs()),
        Some(0),
        "a soft cone always resolves to someone eligible"
    );
    let behind = [Vec2::new(-50.0, 0.0)];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &behind, None, &knobs()),
        Some(0),
        "there is no acceptance test on the angle at all, not even at 180 degrees"
    );
}

/// The soft-cone property the issue names: whether a far teammate dead on the
/// aim line beats a near one far off it is decided *by the angular term*, and
/// therefore flips across the weight's declared range rather than being
/// hardcoded either way.
#[test]
fn the_angular_weight_decides_between_near_off_axis_and_far_on_axis() {
    // Near man: 200 px away, 90 degrees off (chord 1.414).
    // Far man: 300 px away, dead on the line (chord 0).
    let mates = [Vec2::new(0.0, 200.0), Vec2::new(300.0, 0.0)];

    // At a low weight distance dominates and the near man wins.
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &open_knobs(10.0)),
        Some(0),
        "10 px/chord buys 14 px of penalty against a 100 px distance gap"
    );
    // At a high weight the aim dominates and the far man on the line wins.
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &open_knobs(200.0)),
        Some(1),
        "200 px/chord buys 283 px of penalty, which outweighs the 100 px gap"
    );

    // And the crossover is where the arithmetic says it is, not where a
    // hardcoded cone would put it: 100 px of distance gap over 1.414 chord.
    let crossover = 100.0 / 2.0_f64.sqrt();
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &open_knobs(crossover * 0.99)),
        Some(0)
    );
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &open_knobs(crossover * 1.01)),
        Some(1)
    );
}

#[test]
fn eligibility_bounds_exclude_a_handoff_and_a_punt() {
    let k = SelectionKnobs {
        angular_weight: 90.0,
        eligible_min: 20.0,
        eligible_max: 400.0,
    };
    // Inside on both ends.
    assert_eq!(
        passing::select_receiver(FROM, EAST, &[Vec2::new(20.0, 0.0)], None, &k),
        Some(0),
        "the bounds are inclusive"
    );
    assert_eq!(
        passing::select_receiver(FROM, EAST, &[Vec2::new(400.0, 0.0)], None, &k),
        Some(0)
    );
    // Outside on either end, and nobody else to fall back to.
    assert_eq!(
        passing::select_receiver(FROM, EAST, &[Vec2::new(19.9, 0.0)], None, &k),
        None,
        "a teammate at your elbow is a handoff, not a pass"
    );
    assert_eq!(
        passing::select_receiver(FROM, EAST, &[Vec2::new(400.1, 0.0)], None, &k),
        None
    );
    // An excluded teammate does not block an eligible one.
    let mates = [Vec2::new(5.0, 0.0), Vec2::new(120.0, 0.0)];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &k),
        Some(1)
    );
}

/// The tie-break, asserted at an EXACT tie rather than a near one.
///
/// Two teammates mirrored across the aim line score bit-identically — same
/// distance, same chord — so this is the case the `<` in `select_receiver`
/// exists for. `>` would return the later index and two peers would agree
/// only by luck.
#[test]
fn an_exact_tie_breaks_on_the_lower_index() {
    let mates = [Vec2::new(100.0, 100.0), Vec2::new(100.0, -100.0)];
    let k = open_knobs(90.0);
    assert_eq!(
        passing::score(FROM, EAST, mates[0], None, k.angular_weight),
        passing::score(FROM, EAST, mates[1], None, k.angular_weight),
        "the two candidates must actually tie, or this tests nothing"
    );
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &k),
        Some(0)
    );
    // Reversed input order returns the other player, which is what makes the
    // rule "lower index" rather than "this player".
    let reversed = [mates[1], mates[0]];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &reversed, None, &k),
        Some(0)
    );
}

/// A zero aim direction resolves rather than refusing.
///
/// The old function returned `None`, which is the gate wearing a different
/// hat: no aim meant no pass. With no aim direction no teammate is more
/// aimed-at than any other, so the angular term is zero for everyone and the
/// choice falls through to distance.
#[test]
fn a_zero_aim_direction_falls_through_to_distance() {
    let mates = [Vec2::new(300.0, 0.0), Vec2::new(0.0, 80.0)];
    assert_eq!(
        passing::select_receiver(FROM, Vec2::new(0.0, 0.0), &mates, None, &open_knobs(90.0)),
        Some(1)
    );
}

#[test]
fn the_angular_term_is_monotone_in_the_angle_and_bounded_by_two() {
    // 0, 60, 90, 180 degrees off the aim, as raw offsets.
    let offsets = [
        Vec2::new(1.0, 0.0),
        Vec2::new(0.5, 3.0_f64.sqrt() / 2.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(-1.0, 0.0),
    ];
    let terms: Vec<f64> = offsets
        .iter()
        .map(|o| passing::angular_term(EAST, *o))
        .collect();
    for pair in terms.windows(2) {
        assert!(
            pair[1] > pair[0],
            "chord must rise with the angle: {terms:?}"
        );
    }
    assert!((terms[0] - 0.0).abs() < 1e-12);
    assert!(
        (terms[1] - 1.0).abs() < 1e-12,
        "60 degrees is chord 1 exactly"
    );
    assert!((terms[3] - 2.0).abs() < 1e-12, "the term is bounded by 2");
}

#[test]
fn selection_is_a_pure_function_of_its_inputs() {
    // Recomputed per request and never cached: the same call twice is the
    // same answer, and a moved teammate changes it immediately.
    let k = knobs();
    let mates = [Vec2::new(120.0, 0.0), Vec2::new(140.0, 30.0)];
    let first = passing::select_receiver(FROM, EAST, &mates, None, &k);
    assert_eq!(
        first,
        passing::select_receiver(FROM, EAST, &mates, None, &k)
    );
    let moved = [Vec2::new(400.0, 0.0), Vec2::new(140.0, 30.0)];
    assert_ne!(
        first,
        passing::select_receiver(FROM, EAST, &moved, None, &k),
        "the world moved, so the answer must too"
    );
}

// ---------------------------------------------------------------------
// the registered distance-to-speed curve
// ---------------------------------------------------------------------

#[test]
fn the_speed_curve_rises_with_distance_between_its_registered_ends() {
    let tune = Tuning::new();
    let lo = tune.value(passing::SPEED_MIN_KNOB);
    let hi = tune.value(passing::SPEED_MAX_KNOB);
    assert_eq!(
        passing::speed_for(0.0, &tune),
        lo,
        "short balls hit the floor"
    );
    assert_eq!(
        passing::speed_for(5000.0, &tune),
        hi,
        "long balls hit the ceiling"
    );
    let mid: Vec<f64> = (0..8)
        .map(|i| passing::speed_for(200.0 + f64::from(i) * 40.0, &tune))
        .collect();
    for pair in mid.windows(2) {
        assert!(pair[1] >= pair[0], "the curve is non-decreasing: {mid:?}");
    }
    assert!(
        mid.last() > mid.first(),
        "and it genuinely rises somewhere in the middle: {mid:?}"
    );
}

#[test]
fn the_speed_curve_reads_its_knobs_rather_than_restating_constants() {
    let mut tune = Tuning::new();
    let before = passing::speed_for(300.0, &tune);
    tune.set(passing::ARRIVE_PACE_KNOB, 300.0);
    assert!(
        passing::speed_for(300.0, &tune) > before,
        "PASS_ARRIVE_PACE must reach the curve"
    );
    tune.set(passing::SPEED_MAX_KNOB, 450.0);
    assert_eq!(
        passing::speed_for(300.0, &tune),
        450.0,
        "PASS_SPEED_MAX must cap it"
    );
}

/// An inverted min/max pair must not abort the process.
///
/// The two knobs' declared ranges overlap, so a sweep may author
/// `min > max`, and `f64::clamp` panics on that — under `panic = "abort"`
/// that is a player's match ending over a knob combination the registry
/// itself permits.
#[test]
fn an_inverted_speed_range_clamps_instead_of_panicking() {
    let mut tune = Tuning::new();
    tune.set(passing::SPEED_MIN_KNOB, 600.0);
    tune.set(passing::SPEED_MAX_KNOB, 450.0);
    assert_eq!(passing::speed_for(10.0, &tune), 600.0);
    assert_eq!(passing::speed_for(4000.0, &tune), 600.0);
}
