//! Tests for `gc_sim::knob_contract` — the knob-moves-metric contract.
//!
//! Two of these matter more than the rest, and they are a pair:
//!
//! - [`knob_contract_passes_for_a_wired_knob`] is the contract's one shipped
//!   example, using an already-wired tunable (`AI_SHOOT_RANGE`).
//! - [`knob_contract_goes_red_for_a_decoration_knob`] and
//!   [`knob_contract_goes_red_for_a_backwards_wired_knob`] are its
//!   demonstrations that it can fail — one for a knob that moves nothing
//!   (`REPLAY_SLOWMO`, registered and swept and read by no simulation code),
//!   one for a knob whose metric moves the WRONG WAY. Both matter: a
//!   magnitude-only contract passes the second case, and a backwards-wired
//!   knob is a bug that looks exactly like a success. AGENTS.md §9 requires
//!   every gate to ship a demonstration it can go red, and this gate has two
//!   distinct ways to be broken.

use gc_sim::knob_contract::{self, ExpectedShift, KnobMoveOpts, Perturb};

// Short matches and a modest seed set: enough for a wired knob to clear its
// measured noise floor, cheap enough for a per-feature test. A feature whose
// knob is subtler raises either number.
const DURATION: Option<f64> = Some(30.0);

fn seeds(n: usize) -> Vec<f64> {
    (0..n).map(|i| 20_001.0 + i as f64).collect()
}

#[test]
fn knob_contract_passes_for_a_wired_knob() {
    // `AI_SHOOT_RANGE` against `longest_drought_s`: shooting from further out
    // shortens the gaps between chances, which is a claim about the metric's
    // direction, not just its magnitude. Deterministic — the same seeds and
    // duration produce the same numbers on every run, so this cannot flake;
    // the seed count is what buys margin over the measured noise floor.
    let seeds = seeds(48);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "AI_SHOOT_RANGE",
        metric: "longest_drought_s",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        // The claim, made through the helper rather than re-checked by hand
        // afterwards: raising the AI's shooting range SHORTENS the gaps
        // between chances.
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.passes && outcome.moved, "{}", outcome.report);
    assert!(
        outcome.delta.abs() > outcome.threshold,
        "the verdict must be the measurement, not a flag: {}",
        outcome.report
    );
    assert!(
        outcome.noise.sd > 0.0 && outcome.noise.standard_error > 0.0,
        "the noise floor must be measured, not assumed: {}",
        outcome.report
    );
    assert!(
        outcome.delta < 0.0,
        "shooting from further out must SHORTEN the drought, not merely move it: {}",
        outcome.report
    );
    assert_eq!(outcome.expect, ExpectedShift::Decreases);
    assert!(
        outcome.report.contains("WIRED"),
        "the report must say what it concluded: {}",
        outcome.report
    );
}

#[test]
fn knob_contract_goes_red_for_a_decoration_knob() {
    // `REPLAY_SLOWMO` is registered, appears in the F1 panel, and is swept —
    // and no `gc-sim` code reads it (`packages/render/src/replay.ts` restates
    // the value on the TypeScript side). Perturbing it therefore cannot change
    // a single match, so the paired delta is exactly zero and the contract
    // must refuse it.
    let seeds = seeds(8);
    let outcome = knob_contract::knob_moves_metric(&KnobMoveOpts {
        knob: "REPLAY_SLOWMO",
        metric: "goals_total",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Up),
    });
    assert!(
        !outcome.moved && !outcome.passes,
        "a knob no simulation code reads must fail the contract: {}",
        outcome.report
    );
    assert_eq!(
        outcome.delta, 0.0,
        "a knob nothing reads must produce a bit-identical match: {}",
        outcome.report
    );
    assert!(
        outcome.report.contains("DECORATION"),
        "the report must name the failure: {}",
        outcome.report
    );

    // And the assertion helper a feature test would call actually panics.
    let result = std::panic::catch_unwind(|| {
        knob_contract::assert_moves(&KnobMoveOpts {
            knob: "REPLAY_SLOWMO",
            metric: "goals_total",
            seeds: &seeds,
            duration: DURATION,
            perturbation: None,
            expect: ExpectedShift::Increases,
            direction: Some(Perturb::Up),
        })
    });
    let err = result.expect_err("assert_moves must panic for a decoration knob");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        message.contains("REPLAY_SLOWMO") && message.contains("DECORATION"),
        "the panic must be the report, so a failing review reads why: {message}"
    );
}

#[test]
fn knob_contract_goes_red_for_a_backwards_wired_knob() {
    // The failure a magnitude-only contract waves through. `AI_SHOOT_RANGE`
    // genuinely moves `longest_drought_s` and genuinely clears the noise floor
    // — it just moves it DOWN. A feature claiming it moves up is describing a
    // knob wired the wrong way round, and that must fail exactly as loudly as
    // a knob wired to nothing.
    let seeds = seeds(48);
    let outcome = knob_contract::knob_moves_metric(&KnobMoveOpts {
        knob: "AI_SHOOT_RANGE",
        metric: "longest_drought_s",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Up),
    });

    assert!(
        outcome.moved,
        "the shift itself must be real, or this tests the wrong thing: {}",
        outcome.report
    );
    assert!(
        !outcome.passes,
        "a real shift in the WRONG direction must still fail the contract: {}",
        outcome.report
    );
    assert!(
        outcome.report.contains("BACKWARDS"),
        "a backwards-wired knob must be named as such, not lumped in with dead ones: {}",
        outcome.report
    );

    let result = std::panic::catch_unwind(|| {
        let seeds = (0..48).map(|i| 20_001.0 + i as f64).collect::<Vec<f64>>();
        knob_contract::assert_moves(&KnobMoveOpts {
            knob: "AI_SHOOT_RANGE",
            metric: "longest_drought_s",
            seeds: &seeds,
            duration: DURATION,
            perturbation: None,
            expect: ExpectedShift::Increases,
            direction: Some(Perturb::Up),
        })
    });
    let err = result.expect_err("assert_moves must panic for a backwards-wired knob");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        message.contains("BACKWARDS") && message.contains("expected to increase"),
        "the panic must state the claim it failed: {message}"
    );

    // The same measurement with the correct claim passes: the difference is
    // the declared direction and nothing else.
    let corrected = knob_contract::knob_moves_metric(&KnobMoveOpts {
        knob: "AI_SHOOT_RANGE",
        metric: "longest_drought_s",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Up),
    });
    assert!(corrected.passes, "{}", corrected.report);
    assert_eq!(
        corrected.delta, outcome.delta,
        "the claim must not change the measurement"
    );
}

#[test]
fn knob_contract_measures_a_noise_floor_rather_than_assuming_one() {
    let seeds = seeds(16);
    let floor = knob_contract::noise_floor("goals_total", &seeds, DURATION);
    assert_eq!(floor.n, seeds.len());
    assert!(floor.sd >= 0.0);
    assert!(
        (floor.standard_error - floor.sd / (floor.n as f64).sqrt()).abs() < 1e-12,
        "the threshold's basis is the standard error of the mean"
    );

    // Two runs of the same measurement agree exactly: the simulation is
    // deterministic, so the "noise" being measured is seed-to-seed spread, not
    // run-to-run variation. That distinction is why the helper thresholds on
    // dispersion rather than repeating a batch.
    let again = knob_contract::noise_floor("goals_total", &seeds, DURATION);
    assert_eq!(floor, again);
}

#[test]
fn knob_contract_panics_on_an_unregistered_knob_or_metric() {
    let enough = seeds(knob_contract::MIN_SEEDS);
    let knob = std::panic::catch_unwind(|| {
        knob_contract::knob_moves_metric(&KnobMoveOpts {
            knob: "NOT_A_KNOB",
            metric: "goals_total",
            seeds: &enough,
            duration: DURATION,
            perturbation: None,
            expect: ExpectedShift::Unstated,
            direction: None,
        })
    });
    assert!(knob.is_err(), "an unregistered knob is a programmer error");
    let metric =
        std::panic::catch_unwind(|| knob_contract::noise_floor("not_a_metric", &enough, DURATION));
    assert!(
        metric.is_err(),
        "an unregistered metric is a programmer error"
    );
}

#[test]
fn knob_contract_refuses_a_seed_set_too_small_to_threshold_against() {
    // A structural floor, not a convention: below it a lucky small standard
    // error yields a small threshold, and a shift nobody could reproduce
    // reports WIRED.
    let too_few = seeds(knob_contract::MIN_SEEDS - 1);
    let contract = std::panic::catch_unwind(|| {
        knob_contract::knob_moves_metric(&KnobMoveOpts {
            knob: "AI_SHOOT_RANGE",
            metric: "goals_total",
            seeds: &too_few,
            duration: DURATION,
            perturbation: None,
            expect: ExpectedShift::Unstated,
            direction: None,
        })
    });
    let err = contract.expect_err("too few seeds must be refused, not silently accepted");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        message.contains("at least") && message.contains("seeds"),
        "the refusal must say what the floor is: {message}"
    );

    let floor =
        std::panic::catch_unwind(|| knob_contract::noise_floor("goals_total", &too_few, DURATION));
    assert!(floor.is_err(), "noise_floor holds the same floor");

    // And exactly at the floor it runs.
    let at_floor = seeds(knob_contract::MIN_SEEDS);
    assert_eq!(
        knob_contract::noise_floor("goals_total", &at_floor, DURATION).n,
        knob_contract::MIN_SEEDS
    );
}

/// The noise-floor pilot, run by hand rather than on every CI run.
///
/// `cargo test -p gc-sim --test knob_contract -- --ignored --nocapture` prints
/// every registered metric's mean, per-match standard deviation and standard
/// error at default knobs, over 60 full-length matches. Ignored because it
/// costs minutes, not because the number is optional: it is the evidence
/// behind `knob_contract`'s claim that its threshold is measured, and the PR
/// that introduced this module records one run of it.
#[test]
#[ignore = "noise-floor pilot: minutes, run by hand"]
fn noise_floor_pilot_reports_per_metric_variance_at_defaults() {
    let seeds: Vec<f64> = (0..60).map(|i| 20_001.0 + i as f64).collect();
    println!(
        "{:<22} {:>4} {:>10} {:>10} {:>10}",
        "metric", "n", "mean", "sd", "se"
    );
    for id in gc_sim::metric_registry::shipped().ids() {
        let floor = knob_contract::noise_floor(id, &seeds, None);
        println!(
            "{:<22} {:>4} {:>10.4} {:>10.4} {:>10.4}",
            id, floor.n, floor.mean, floor.sd, floor.standard_error
        );
    }
}

/// #488's contract entry: the locomotion primitive's own knob-moves-metric
/// proof.
///
/// AGENTS.md §9 requires every feature to ship one of these, and the
/// locomotion rework is the first feature to register a whole *family* of
/// knobs. `LOCO_BASE_TURN` is the one worth asserting: it is the shared
/// angular rate every context multiplies, for the movement heading and for
/// facing alike, so it is the knob that makes turn arcs exist at all. Halve
/// it and bodies commit to a line for longer, take longer to work the ball
/// into a chance, and the gaps between chances get LONGER.
///
/// ## What the issue proposed, and what it measures
///
/// The issue names two pairings: `loco.run.decel` against a new
/// `time_to_reverse` metric, and `loco.carry.top_speed_mult` against
/// `possession_balance`. Neither is used here, for two different reasons, and
/// both are worth recording rather than quietly dropping.
///
/// `time_to_reverse` does not exist yet: it needs new `MetricsPlayerView`
/// fields and both harness adapters, which is a follow-up PR in this stack.
///
/// The carry/possession pairing was measured and is too subtle for any seed
/// count a per-PR gate can afford. `possession_balance`'s per-match sd was
/// 0.11 against a paired delta of 0.02; swapping to `turnovers_per_min`
/// (same pressure, far less variance) and going to 96 seeds and full-length
/// matches still landed at -0.28 against a 0.38 threshold — five minutes of
/// compute to report `DECORATION` for a knob that is not decoration. The
/// contract is right to refuse it; that is the gate working. It is recorded
/// here because "raise the seeds until it passes" is the tempting wrong
/// answer, and because the balance sweep, which can afford the seeds, is
/// where that pairing belongs.
///
/// **Those four figures were measured on the chord-bounded draft, before the
/// turn rate was corrected, and are not re-measured here.** The conclusion
/// they support — that the pairing is too subtle for a per-PR gate — is the
/// durable part and got *more* true, not less: correcting the rate shrank
/// every locomotion knob's measured effect (see the note in the body below).
/// Treat the numbers as the order of magnitude they are, not as current
/// readings.
#[test]
fn turn_rate_moves_the_gaps_between_chances() {
    // 96 rather than the 48 the other cases here use. Worth knowing why: the
    // CHORD-bounded draft of `locomotion` (the rate defect design review
    // caught) passed this at 48, because collapsing the achieved rate at
    // large remaining angles amplified the knob's apparent effect. Fixing the
    // rate to be honest SHRANK the measured delta, from +1.48 to +0.69. A
    // gate that got easier when the code got wronger is worth naming.
    let seeds = seeds(96);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "LOCO_BASE_TURN",
        metric: "longest_drought_s",
        seeds: &seeds,
        duration: DURATION,
        // The full declared range rather than the default third of it: this
        // knob is the shared BASE, so every context multiplier damps it, and
        // a third of the range does not clear the metric's own noise.
        perturbation: Some(1.0),
        // The direction is the claim, not an afterthought: bodies that turn
        // more slowly take LONGER between chances. A knob wired backwards
        // passes any magnitude-only assertion.
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Down),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.delta.abs() > outcome.threshold,
        "the verdict must be the measurement, not a flag: {}",
        outcome.report
    );
    assert!(
        outcome.noise.sd > 0.0,
        "the noise floor must be measured, not assumed: {}",
        outcome.report
    );
    assert!(
        outcome.report.contains("WIRED"),
        "a registered locomotion knob that moves nothing is decoration: {}",
        outcome.report
    );
}

/// #488's specified pairing, corrected by measurement — and the correction is
/// the finding, not a detail.
///
/// The issue says "`LOCO_RUN_DECEL` up must lower `time_to_reverse`". Measured
/// over 24 full-length matches at the knob's **full declared range**, that
/// pairing reports `DECORATION`: delta -0.011 against a 0.012 threshold.
/// Not weak — structurally wrong. `locomotion::resolve` tests the
/// movement-versus-facing geometry BEFORE possession or sprint, so from the
/// first tick of a commanded reversal, when the body is moving opposite the
/// way it looks, the context is `Backpedal`. The run context does not own the
/// braking phase of a reversal and cannot move this metric. The census in the
/// PR body has the full table.
///
/// So the claim is asserted against `MOVE_DECEL`, the shared brake base every
/// context multiplies, which is the knob that genuinely governs "how hard a
/// body brakes" wherever the reversal happens to resolve.
///
/// The full range rather than the default third of it, deliberately: braking
/// is a real but minority share of a reversal, most of which is rebuilding
/// speed from a standstill. At a third of range the effect is honestly below
/// the noise, and the right response is a bigger lever, not more seeds.
#[test]
fn braking_harder_shortens_a_reversal() {
    let seeds = seeds(24);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "MOVE_DECEL",
        metric: "time_to_reverse",
        seeds: &seeds,
        duration: None,
        perturbation: Some(1.0),
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the shared brake base cannot move the reversal metric: {}",
        outcome.report
    );
}

/// The other half of the reversal, and the half that dominates it.
///
/// A reversal is brake-then-accelerate, and the clock does not stop until the
/// body has rebuilt half its base speed in the new direction. Accelerating
/// from a standstill is most of that, which is why this knob moves the metric
/// three times as far as the braking one does.
#[test]
fn accelerating_harder_shortens_a_reversal() {
    let seeds = seeds(24);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "LOCO_RUN_ACCEL_MULT",
        metric: "time_to_reverse",
        seeds: &seeds,
        duration: None,
        perturbation: None,
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the run context's accel multiplier is decoration: {}",
        outcome.report
    );
}

/// The same claim in a second context, so what is exercised is the PARAMETRIC
/// DERIVATION rather than one hand-wired knob.
///
/// `LOCO_BACKPEDAL_ACCEL_MULT` reaches this metric only if
/// `locomotion::profile` really does multiply the shared base by the resolved
/// context's own multiplier, and only if `resolve` really does put a
/// reversing body in `Backpedal`. A build that special-cased one context and
/// left the rest reading the same numbers would pass the test above and fail
/// this one.
///
/// It is also the single strongest lever on `time_to_reverse` of all 45
/// `LOCO_*` knobs, which is not a coincidence: it is the accel multiplier of
/// the context a reversal actually runs in.
#[test]
fn the_derivation_reaches_a_second_context() {
    let seeds = seeds(24);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "LOCO_BACKPEDAL_ACCEL_MULT",
        metric: "time_to_reverse",
        seeds: &seeds,
        duration: None,
        perturbation: None,
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the backpedal context's accel multiplier is decoration: {}",
        outcome.report
    );
}

/// The measurement that justifies adding a metric at all, standing as a test
/// so it cannot quietly stop being true.
///
/// #488's knobs are kinematic and the eight metrics that existed before this
/// one are all match OUTCOMES — goals, shots, possession, droughts. A
/// kinematics change reaches an outcome only through many layers of AI
/// decision-making, which is why 44 of 45 `LOCO_*` knobs reported
/// `DECORATION` against every one of them at seed counts a gate can afford.
/// That was never a tuning failure; it was a measurement mismatch, and
/// AGENTS.md §9 is unambiguous that a knob which cannot move a metric fails
/// review.
///
/// Measured on 24 seeds of full-length matches, relative standard error
/// (`se / mean`, so the numbers are comparable across units):
///
/// | metric | mean | sd | se/mean |
/// | --- | --- | --- | --- |
/// | `time_to_reverse` | 0.481 | 0.023 | **1.0%** |
/// | `possession_balance` | 0.54 | 0.11 | ~4% |
/// | `turnovers_per_min` | 8.9 | 2.7 | ~6% |
/// | `goals_total` | 1.4 | 0.75 | ~11% |
///
/// An order of magnitude, and it is structural rather than lucky: this is a
/// mean over hundreds of reversal events per match, where `goals_total` is a
/// count of roughly one and a half.
#[test]
fn time_to_reverse_resolves_where_the_outcome_metrics_do_not() {
    let seeds = seeds(24);
    let reversal = knob_contract::noise_floor("time_to_reverse", &seeds, None);
    assert!(
        reversal.mean > 0.0,
        "the metric never armed -- no match in the seed set contained a reversal from a run, \
         so every assertion above is vacuous"
    );
    let relative = reversal.standard_error / reversal.mean;
    assert!(
        relative < 0.02,
        "time_to_reverse's relative standard error is {relative:.4}, which is not the \
         resolution this metric was added for"
    );
    for coarse in ["goals_total", "possession_balance"] {
        let other = knob_contract::noise_floor(coarse, &seeds, None);
        let other_relative = other.standard_error / other.mean.abs();
        assert!(
            other_relative > relative * 2.0,
            "{coarse} now resolves within 2x of time_to_reverse ({other_relative:.4} vs \
             {relative:.4}) -- the argument for adding a kinematic metric has changed and \
             this test should be revisited rather than relaxed"
        );
    }
}

/// The band in `gc_data::tunables::METRICS` is #488's PRIOR, and this is the
/// measurement that either backs it or does not.
///
/// The issue proposes 0.25-0.6 s and says explicitly that the suggested
/// ranges are priors needing a hands-on pass. At the shipped defaults the
/// measured mean is 0.48 s — inside the band, nearer its slow edge than its
/// fast one. So the prior survives contact with the measurement, which is
/// worth pinning: if a later rework moves the default outside the band, that
/// is a decision someone should make deliberately rather than discover.
#[test]
fn the_shipped_defaults_land_inside_the_proposed_band() {
    let seeds = seeds(24);
    let floor = knob_contract::noise_floor("time_to_reverse", &seeds, None);
    let def = gc_data::tunables::METRICS
        .iter()
        .find(|d| d.id == "time_to_reverse")
        .expect("the metric is registered");
    assert!(
        floor.mean > def.band[1] && floor.mean < def.band[2],
        "the shipped defaults reverse in {:.3}s, outside the ideal band {:?}..{:?}",
        floor.mean,
        def.band[1],
        def.band[2]
    );
}
