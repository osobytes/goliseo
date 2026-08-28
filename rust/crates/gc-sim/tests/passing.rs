//! Tests for `gc_sim::passing` — soft-scored receiver selection and the
//! registered distance-to-speed launch curve.
//!
//! What this file used to assert is worth naming, because two of its cases
//! are **deleted rather than repaired**: a teammate behind the passer, and a
//! teammate 84 degrees off the aim, both used to select nobody. That was the
//! hard 60-degree acceptance cone, and deleting it is the point of #491 — a
//! gate turns near-misses into non-passes and non-passes into faith. The
//! replacement claim is `a_teammate_the_old_cone_excluded_is_now_selected`,
//! which asserts the near-miss geometry resolves.
//!
//! The dead-behind half of the old geometry has since swung back — but to a
//! **half-plane**, not to the 60-degree cone. The futsal re-dimensioning
//! made a full-charge `|d - range|` term big enough for a teammate directly
//! behind the aim to out-score one dead on it, and the owner ruled a pass
//! must never go opposite to the aim. That is the invariant the "never
//! backwards" cases below pin; `gc_sim::passing`'s module doc argues why the
//! half-plane does not reintroduce #491's failure (a 45-degree — or
//! 89-degree — miss still passes).

use gc_core::vec2::Vec2;
use gc_sim::passing::{self, SelectionKnobs};
use gc_sim::tuning::Tuning;
use gc_sim::{ball_flight, pass_lead};

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
    // k-scaled with the pitch: at the old 30 px the nearer mate now falls
    // below PASS_ELIGIBLE_MIN (20 -> 34) and is filtered before scoring, which
    // would make this assert the eligibility filter rather than the tie-break.
    let mates = [Vec2::new(172.0, 0.0), Vec2::new(52.0, 0.0)];
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
/// This was `None` before #491: a teammate 84 degrees off the aim, invisible
/// to the old 60-degree gate, so a player who aimed a little wide got no
/// pass at all and no feedback about why. It is deliberately the widest
/// forward miss there is — one degree short of square — because that is the
/// edge of what the half-plane aim gate leaves to the soft cone.
///
/// This test's second half used to assert the mirror image: a teammate
/// directly BEHIND the passer was also selectable, "no acceptance test on
/// the angle at all, not even at 180 degrees". The owner's futsal play-test
/// ruling reversed exactly and only that half — see
/// `a_teammate_behind_the_aim_is_never_selected_at_any_charge` — so the
/// claim now stops at square instead of extending to 180.
#[test]
fn a_teammate_the_old_cone_excluded_is_now_selected() {
    let wide = [Vec2::new(10.0, 100.0)]; // ~84 degrees off the aim
    assert_eq!(
        passing::select_receiver(FROM, EAST, &wide, None, &knobs()),
        Some(0),
        "a soft cone resolves any eligible near-miss forward of square"
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
    // Probe inside the LINEAR band. The rescaled floor (PASS_SPEED_MIN 720)
    // now sits above the curve at the old 300 px probe, so measuring there
    // would only ever observe the floor and would pass whatever ARRIVE_PACE
    // did. The band is [(MIN - PACE) / FRICTION, (MAX - PACE) / FRICTION],
    // which is roughly 425..1042 px at the shipped defaults.
    let d = 700.0;
    let before = passing::speed_for(d, &tune);
    tune.set(passing::ARRIVE_PACE_KNOB, 300.0);
    assert!(
        passing::speed_for(d, &tune) > before,
        "PASS_ARRIVE_PACE must reach the curve"
    );
    tune.set(passing::SPEED_MAX_KNOB, 800.0);
    assert_eq!(
        passing::speed_for(d, &tune),
        800.0,
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
    // Authored at the two knobs' own declared extremes, so `tune.set` cannot
    // quietly clamp the pair back into a valid order and leave this asserting
    // nothing: PASS_SPEED_MIN's ceiling is above PASS_SPEED_MAX's floor, which
    // is exactly the overlap that makes an inverted pair authorable at all.
    let mut tune = Tuning::new();
    tune.set(passing::SPEED_MIN_KNOB, 1030.0);
    tune.set(passing::SPEED_MAX_KNOB, 770.0);
    assert_eq!(passing::speed_for(10.0, &tune), 1030.0);
    assert_eq!(passing::speed_for(4000.0, &tune), 1030.0);
}

// ---------------------------------------------------------------------------
// The reach invariant (#622 play-test follow-up).
//
// Reported after play-testing the futsal pitch: "passes often don't even reach
// the player -- the teammate has to run to a ball that stopped half way."
//
// The cause is not randomness. `speed_for` clamps at `PASS_SPEED_MAX`, so
// `passing::reach` is a hard ceiling on how far a ground pass can travel. If a
// pass may legally be AIMED further than that ceiling, the ball provably stops
// short every single time it happens. Nothing enforced that the aimable
// maximum and the reachable maximum were compatible, and they were not -- the
// pre-resize pair violated it by 193 px, the post-resize pair by far more.
//
// These tests are the enforcement. They are deliberately written against the
// SHIPPED defaults rather than a fixture, because the defect is a relationship
// between four independently-authored knobs, and any one of them drifting is
// what reintroduces it.

// HISTORY: this used to assert `reach >= PASS_ELIGIBLE_MAX +
// PASS_LEAD_TIME_MAX * LOCO_PACE_REF_HI` — the furthest LED aim. That bound
// was computed with the wrong receiver speed: the lead solver reads
// `run_vel`, whose sprinting maximum is `LOCO_PACE_REF_HI * SPRINT_MULT`
// (280 × 1.35 = 378 px/s), not the bare 280. At the true speed the old
// assertion is violated by ~83 px with the shipped knobs — it only passed
// by under-measuring. The led case is now enforced STRUCTURALLY instead:
// `pass_lead::solve` refuses any candidate consuming more than
// `pass_lead::REACH_MARGIN` of its own launch's roll-out (see
// tests/pass_lead.rs), so no knob relationship can reintroduce a led pass
// that dies short. What remains a knob relationship is the UNLED case
// below.

#[test]
fn an_unled_pass_can_always_roll_as_far_as_it_can_legally_be_aimed() {
    // An unled pass aims at the receiver's current feet, and selection caps
    // that distance at PASS_ELIGIBLE_MAX. Nothing structural clamps it, so
    // the knobs must keep agreeing.
    let tune = Tuning::new();
    let reach = passing::reach(&tune);
    let eligible = tune.value(passing::ELIGIBLE_MAX_KNOB);
    assert!(
        reach >= eligible,
        "an unled pass can be aimed {eligible:.1} px away but can only roll \
         {reach:.1} px: every pass past {reach:.1} px dies short by \
         construction. Raise PASS_SPEED_MAX (reach = PASS_SPEED_MAX / \
         FRICTION) or lower PASS_ELIGIBLE_MAX."
    );
}

#[test]
fn a_clamped_lead_still_stretches_past_the_eligibility_ceiling() {
    // The solver's REACH_MARGIN clamp must not be so tight that a led pass
    // cannot even reach where an unled one legally aims — leading must
    // never be the SHORTER option.
    let tune = Tuning::new();
    let reach = passing::reach(&tune);
    let eligible = tune.value(passing::ELIGIBLE_MAX_KNOB);
    assert!(
        reach * pass_lead::REACH_MARGIN >= eligible,
        "the furthest admissible led aim ({:.1} px) is inside the \
         eligibility ceiling ({eligible:.1} px)",
        reach * pass_lead::REACH_MARGIN
    );
}

#[test]
fn the_reach_ceiling_is_the_speed_ceiling_over_friction() {
    // Pins the derivation itself, so a future change to how speed_for clamps
    // cannot leave `reach` quietly describing a curve that no longer exists.
    let tune = Tuning::new();
    let expected = tune.value(passing::SPEED_MAX_KNOB) / ball_flight::FRICTION;
    assert!((passing::reach(&tune) - expected).abs() <= 1e-9);

    // And that the clamp is real: asking for a distance past the ceiling
    // returns the ceiling speed, not a speed that would carry the ball there.
    let past = passing::reach(&tune) * 2.0;
    assert!((passing::speed_for(past, &tune) - tune.value(passing::SPEED_MAX_KNOB)).abs() <= 1e-9);
}

/// The second reported defect: aiming forward at a teammate ahead, the pass
/// went backwards to a closer teammate. This test used to assert the weight
/// arithmetic (`150 + WEIGHT * 2 > 500`), which is the brute-force fix that
/// shipped first — and which broke down again at full charge, where the
/// `|d - range|` distance term reaches `PASS_RANGE_MAX` and no affordable
/// weight can cover it. "Never backwards" is now the half-plane aim gate's
/// job, a structural invariant rather than a knob's, so this asserts the
/// GATE: the behind candidate is rejected outright, at a tap and at the
/// full-charge geometry that beat the weight.
#[test]
fn an_aimed_teammate_beats_a_closer_one_behind_the_passer() {
    // The original report's geometry: dead on aim at 500 px, versus 180
    // degrees off at 150 px. Both inside the eligibility window.
    let mates = [Vec2::new(-150.0, 0.0), Vec2::new(500.0, 0.0)];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &knobs()),
        Some(1),
        "a tap must go to the man on the aim, not the closer man behind it"
    );

    // The full-charge reproduction from the play-test: charge range at
    // PASS_RANGE_MAX (890), teammate on-aim at 300 px (charge term 590),
    // teammate dead behind at exactly the charged range (charge term 0).
    // Under weight-only arbitration the behind man wins at any weight below
    // 295 px/chord; under the gate he is not a candidate at all.
    let tune = Tuning::new();
    let range = tune.value("PASS_RANGE_MAX");
    let charged = [Vec2::new(-range, 0.0), Vec2::new(300.0, 0.0)];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &charged, Some(range), &knobs()),
        Some(1),
        "at full charge the man behind the aim sits exactly at the charged \
         range and out-scores the aimed man unless the gate removes him"
    );
}

/// The owner's ruling as a property: for ANY charge level, a candidate with
/// negative dot against the aim is never selected while a forward candidate
/// exists. Each behind candidate is planted where it would WIN the soft
/// score without the gate — dead behind at exactly the charged range (charge
/// term zero against the forward man's hundreds), and just barely behind
/// square (95 degrees, chord ~1.47, cheaper still) — so this fails against
/// weight-only arbitration rather than passing vacuously.
#[test]
fn a_teammate_behind_the_aim_is_never_selected_at_any_charge() {
    let k = knobs();
    // The last entry reads the knob rather than restating 890, so a future
    // PASS_RANGE_MAX raise cannot silently stop this sweep exercising full
    // charge -- the geometry above is what makes full charge the case where
    // the behind candidate would win without the gate.
    let full = Tuning::new().value("PASS_RANGE_MAX");
    for range in [None, Some(110.0), Some(300.0), Some(500.0), Some(full)] {
        let r = range.unwrap_or(150.0);
        // 95 degrees off the aim: sin/cos of 5 degrees, dot < 0 by a hair.
        let barely_behind = Vec2::new(-r * 0.08716, r * 0.99619);
        let mates = [
            Vec2::new(-r, 0.0), // dead behind, at the charged range
            barely_behind,
            Vec2::new(230.0, 193.0), // forward, ~40 degrees off, 300 px
        ];
        assert_eq!(
            passing::select_receiver(FROM, EAST, &mates, range, &k),
            Some(2),
            "at charge range {range:?} a behind-the-aim candidate won"
        );
    }
}

/// The owner's worked example: aim bisecting two forward teammates, so the
/// angular terms cancel exactly and the charge term alone arbitrates. A tap
/// finds the near man; a full-length charge picks out the far one.
#[test]
fn aim_bisecting_two_forward_mates_a_tap_goes_near_and_a_charge_goes_far() {
    // Mirrored at ~37 degrees either side of the aim: identical chords.
    let near = Vec2::new(300.0, 225.0); // 375 px
    let far = Vec2::new(600.0, -450.0); // 750 px
    let mates = [near, far];
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, None, &knobs()),
        Some(0),
        "a tap prefers the near man"
    );
    assert_eq!(
        passing::select_receiver(FROM, EAST, &mates, Some(750.0), &knobs()),
        Some(1),
        "a charge to the far man's range picks him out"
    );
}

/// When the gate empties the candidate set, selection lands on the same
/// `None` the distance bounds produce when they exclude everyone — the
/// callers' existing no-eligible-receiver fallback handles both, and no new
/// behaviour was invented for the gate.
#[test]
fn a_set_with_everyone_behind_the_aim_selects_nobody_like_an_ineligible_set() {
    let k = knobs();
    let all_behind = [
        Vec2::new(-100.0, 50.0),
        Vec2::new(-300.0, 0.0),
        Vec2::new(-50.0, -200.0),
    ];
    let all_ineligible = [Vec2::new(5.0, 0.0)]; // inside PASS_ELIGIBLE_MIN
    assert_eq!(
        passing::select_receiver(FROM, EAST, &all_ineligible, None, &k),
        None,
        "(the existing no-eligible outcome this case compares against)"
    );
    assert_eq!(
        passing::select_receiver(FROM, EAST, &all_behind, None, &k),
        passing::select_receiver(FROM, EAST, &all_ineligible, None, &k),
        "an emptied-by-the-gate set must take the exact no-eligible path"
    );
}
