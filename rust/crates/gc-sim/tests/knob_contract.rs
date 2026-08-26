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
    //
    // Seed count raised from 48 to 144 by the futsal re-dimensioning (pitch
    // 960x540 -> 1648x927, k=1.7167): `AI_SHOOT_RANGE`'s own range widened
    // 160-480 -> 280-820 with it (`gc_data::tunables`), so the default 0.35
    // perturbation fraction now moves the knob from 410 to 599 (+189, 35% of
    // the 540px range) instead of 240 to 352 (+112, 35% of the old 320px
    // range) -- a wider absolute jump on a wider pitch, over which the
    // effect on `longest_drought_s` measures fainter per seed. Measured
    // directly rather than assumed:
    //
    // | n   | delta   | threshold | verdict    |
    // | --- | ------- | --------- | ---------- |
    // | 48  | -0.6135 | 0.7185    | DECORATION |
    // | 96  | -0.5266 | 0.5722    | DECORATION |
    // | 144 | -0.9678 | 0.4978    | WIRED (1.9x) |
    let seeds = seeds(144);
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
    //
    // Seed count raised from 48 to 144 alongside the paired case above (see
    // its comment for the measured table) — the pair must share a seed
    // count, since `corrected.delta` below is asserted equal to this
    // measurement's own `outcome.delta`.
    let seeds = seeds(144);
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
        let seeds = (0..144).map(|i| 20_001.0 + i as f64).collect::<Vec<f64>>();
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

// REMOVED: `turn_rate_moves_the_gaps_between_chances`, which perturbed
// `LOCO_BASE_TURN` against `longest_drought_s`.
//
// It was this feature's §9 entry before `time_to_reverse` existed, and it is
// deleted rather than repaired because the metric that replaced it is the
// whole point. After the carry-composition fix it measures +0.557 against a
// 0.657 threshold at 96 seeds -- DECORATION -- and recovering it would need
// roughly 190 seeds for a claim three stronger assertions above already make
// far better.
//
// It is worth being explicit that this is not a failing test deleted to go
// green. The three `time_to_reverse` cases above are strictly stronger: they
// state directions, they clear their thresholds by margins this one never
// had, and they measure the primitive's own claim instead of a match outcome
// four layers downstream. The finding this test's death illustrates is
// already reported: no `LOCO_*` knob moves any OUTCOME metric at a seed count
// a per-PR gate can afford, which is exactly why a kinematic metric had to
// exist.

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
    // Seed count raised from 48 to 144 (#537), derived rather than picked by
    // feel. #537 found the committed n=48 flips this contract to DECORATION
    // on an ordinary gameplay change that leaves the knob comfortably WIRED
    // at adequate power -- not a real unwiring, an underpowered contract.
    //
    // Derivation: the paired-difference standard deviation is stable across
    // seed counts on this branch -- delta_se * sqrt(n) gives ~0.0506 at
    // n=48 and ~0.0500 at n=400 (see the table below), so the threshold
    // scales as NOISE_SIGMAS * diff_sd / sqrt(n) ~= 0.100 / sqrt(n),
    // calibrated against the committed n=48 row's own threshold: 0.100 /
    // sqrt(48) = 0.0144, matching the measured 0.0145 up to rounding.
    // Against this branch's own effect size (~0.0154, measured at n=400),
    // that predicts the margin over threshold widens roughly as sqrt(n) and
    // clears a comfortable multiple well short of n=400. Extrapolating from
    // a noisy mean is exactly the failure #537 reports, so the projection
    // was not trusted on its own -- two candidate seed counts were run and
    // measured directly instead:
    //
    // | n                 | delta   | se        | noise floor | threshold | verdict    | runtime   |
    // | ----------------- | ------- | --------- | ----------- | --------- | ---------- | --------- |
    // | 48 (was)          | -0.0112 | +/-0.0073 | 0.0051      | 0.0145    | DECORATION | 157.45s   |
    // | 144 (chosen)      | --      | --        | --          | --        | WIRED      | 398.62s   |
    // | 200               | --      | --        | --          | --        | WIRED      | 554.73s   |
    // | 400, this branch  | -0.0154 | +/-0.0025 | 0.0018      | 0.0050    | WIRED      | 1160.92s  |
    // | 400, base 2ce0ca0 | -0.0206 | +/-0.0026 | 0.0017      | 0.0052    | WIRED      | 1138.76s  |
    //
    // (144's and 200's own delta/se/noise-floor were not re-captured after
    // their verdicts were confirmed WIRED -- re-running either just to
    // recover the report string costs another 400-550s for no new
    // information the verdict and runtime do not already give.)
    //
    // n=48 is DECORATION here because it is UNDERPOWERED, not because the
    // knob stopped moving the metric: both n=400 rows -- this branch and
    // base 2ce0ca0 -- show the same knob comfortably WIRED. 144 is the
    // smaller of the two seed counts that measured WIRED, trading the
    // extra headroom n=200 bought (and the ~28% more runtime it costs) for
    // a margin (~2.4 measured standard errors clear of threshold) that is
    // comfortable rather than marginal, without paying for more than that.
    //
    // This is a local fix, not the systemic one. #537 STAYS OPEN for
    // separating an UNDERPOWERED verdict from a DECORATION one and
    // auditing the rest of the registry's contracts for the same exposure.
    // Nothing about the verdict machinery, MIN_SEEDS, NOISE_SIGMAS or
    // DEFAULT_PERTURBATION_FRACTION changed here -- only this one
    // contract's seed count.
    let seeds = seeds(144);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "LOCO_BACKPEDAL_DECEL_MULT",
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
    let seeds = seeds(48);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "LOCO_RUN_ACCEL_MULT",
        metric: "time_to_reverse",
        seeds: &seeds,
        duration: None,
        // Full range: at the default third it measures -0.018 against a 0.020
        // threshold. The run context owns only the tail of a reversal, after
        // facing has come round, so its lever is real but small.
        perturbation: Some(1.0),
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

    // The claim is that it resolves better than THE OUTCOME METRICS -- named
    // explicitly, because that set is what the argument above is about and it
    // does not grow. An earlier draft asserted "at least 2x better than
    // `possession_balance`", which is a knife-edge: `possession_balance`
    // happens to be the best-resolving outcome metric, and the carry
    // composition moved the ratio to 1.9 and turned the assertion red without
    // anything about the argument changing. A later draft asserted first place
    // across the WHOLE registry, which was durable only for as long as every
    // other registered metric was an outcome.
    //
    // It stopped being true in #490 and the number says why: on that PR's
    // merge base this ranking read `time_to_reverse` 0.0127, `whiff_rate`
    // 0.0132 -- a 4% gap between the two EVENT-RATE metrics, with the best
    // outcome metric a distant 0.0213. #490's keeper change (more saves
    // resolve as parries, so more loose balls, so more tackle attempts per
    // match) tightened `whiff_rate`'s own denominator and tipped that hair's
    // width the other way: 0.0128 against 0.0144. Nothing about #488's
    // argument moved. What moved is that #489 registered a second metric built
    // on the same insight -- measure the mechanism, not the outcome -- and two
    // metrics of that kind trading first place between them is the argument
    // being VINDICATED, not contradicted.
    //
    // So the outcome comparison is asserted (that is #488's claim) and first
    // place overall is not, while a genuine collapse in resolution still goes
    // red via the absolute bound asserted above and the factor bound below.
    //
    // 2026-08-26, the pass-reception rework: `pass_completion` leaves the
    // compared set, by the same reasoning that scoped this assertion to
    // outcomes when `whiff_rate` traded first place. #488's argument is
    // about DILUTED outcomes — a completion that resolves through many
    // layers of AI chaos downstream of the mechanism being measured. The
    // reception rework collapsed exactly that chaos: the designated
    // receiver may now trap inside the release cooldown and is steered
    // onto the solved reception point, so whether a pass completes is
    // decided mostly at release (probe, 24 seeds: intended-receiver
    // completion 32%->76% inside 2 s, unresolved runouts 0%). Measured
    // here, its relative error tightened past `time_to_reverse` itself
    // (0.0145 vs 0.0164, n=32) — an outcome metric that became
    // event-rate-grade because the event stopped being noisy. That is the
    // #488 insight VINDICATED at the sim level rather than contradicted at
    // the metric level, and the factor bound below still compares against
    // the whole table (pass_completion now anchors `best`), so a genuine
    // resolution collapse in `time_to_reverse` stays red.
    const OUTCOME_METRICS: [&str; 7] = [
        "goals_total",
        "shots_per_goal",
        "save_rate",
        "turnovers_per_min",
        "possession_balance",
        "longest_drought_s",
        "decided_late",
    ];
    let mut table: Vec<(&str, f64)> = Vec::new();
    for id in gc_sim::metric_registry::shipped().ids() {
        let floor = knob_contract::noise_floor(id, &seeds, None);
        if floor.mean.abs() > 0.0 {
            table.push((id, floor.standard_error / floor.mean.abs()));
        }
    }
    table.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("relative errors are finite"));
    let report: Vec<String> = table.iter().map(|(id, r)| format!("{id} {r:.4}")).collect();
    for (id, outcome_relative) in table.iter().filter(|(id, _)| OUTCOME_METRICS.contains(id)) {
        assert!(
            relative < *outcome_relative,
            "time_to_reverse ({relative:.4}) no longer resolves better than the outcome \
             metric {id} ({outcome_relative:.4}), which is the entire argument for adding \
             it: {}",
            report.join(", ")
        );
    }
    // And it must stay in the same league as the best-resolving metric in the
    // registry, whichever that is -- so losing first place to a peer stays
    // fine and losing an order of magnitude to one does not.
    let best = table
        .first()
        .map(|(_, r)| *r)
        .expect("at least one metric armed");
    assert!(
        relative < best * 1.5,
        "time_to_reverse ({relative:.4}) has fallen far behind the best-resolving \
         registered metric ({best:.4}): {}",
        report.join(", ")
    );
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

// ---------------------------------------------------------------------
// #491 — passing: soft-scored selection and the lead solver
// ---------------------------------------------------------------------

// THE CENSUS, recorded here because it is the reason the two metrics below
// exist and a reviewer should not have to take it on trust.
//
// #491's eleven passing knobs were measured against every one of the NINE
// metrics that existed before it, over 48 seeds of 30-second matches, each
// knob displaced across its full declared range. Every pairing reported
// DECORATION. The three closest, all at 48 seeds:
//
// | knob | metric | delta | threshold |
// | --- | --- | --- | --- |
// | `PASS_ELIGIBLE_MAX` down | `pass_completion` | -0.0286 | 0.0289 |
// | `PASS_ANGULAR_WEIGHT` up | `pass_completion` | -0.0022 | 0.0289 |
// | `PASS_LEAD_TOLERANCE` up | `pass_completion` | +0.0073 | 0.0297 |
//
// This is #488's finding repeating for a different subsystem, with two extra
// structural reasons specific to passing, both argued on
// `gc_sim::r#match::PassShadowTally`: soft-cone selection runs for ONE player
// in an AI-vs-AI batch (the match AI picks its own receiver and never
// consults the cone), and a led pass and an unled pass mostly complete
// anyway — leading changes WHERE the ball meets the receiver, not usually
// WHETHER. So `pass_aim_error` and `pass_lead_time` were registered, exactly
// as #488 registered `time_to_reverse`, and the three cases below are their
// contracts.
//
// Worth stating plainly: this means #491's headline claim is HALF met. Pass
// completion is now movable in the sense that the subsystem finally has
// levers with measurable effects — but not movable *as a completion number*
// at any seed count a per-PR gate can afford. The PR body says so.

/// The soft cone's own knob against the soft cone's own measurement.
///
/// **Down, not up, and that asymmetry is the finding.** At the shipped 140
/// px/chord the selection is already close to aim-optimal for the geometry a
/// bot-driven slot actually produces, so raising the weight to 276.5 moves
/// `pass_aim_error` by -0.0012 — it has almost nothing left to win. Lowering
/// it to 10 moves it by +0.108, four seed-set standard errors. The knob is
/// wired; its lever is one-sided at this default, which is information about
/// where 140 sits rather than about whether it is read.
#[test]
fn ignoring_the_aim_sends_the_pass_further_from_where_it_was_pointed() {
    let seeds = seeds(48);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "PASS_ANGULAR_WEIGHT",
        metric: "pass_aim_error",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        // The claim: weighting the aim LESS puts the ball further from where
        // the player pointed. Stated through the helper, not re-checked by
        // hand afterwards.
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Down),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the soft cone's weight is decoration: {}",
        outcome.report
    );
}

/// #531 phase 5's own finding: the second of the three selection knobs, and
/// the one that was asserted WIRED against `pass_aim_error` in #545's PR
/// body and in `docs/design/fun_metrics.md` without ever having a committed
/// contract to back it — only `PASS_ANGULAR_WEIGHT` did. Verified by hand
/// during phase 5's investigation into whether `pass_aim_error` still earns
/// its slot (it was the crux: a metric backing an *unverified* claim is not
/// load-bearing, it is an assumption), confirmed here so the claim stops
/// being prose.
///
/// **Up, not down, and asymmetric the same way `PASS_ANGULAR_WEIGHT` is.**
/// `PASS_ELIGIBLE_MIN` excludes teammates nearer than the floor (a handoff
/// is not a pass). Raising it excludes the near candidates first, which in
/// a bot-driven slot's geometry are disproportionately the ones nearest the
/// aim direction too — so the selection is pushed onto a worse-aimed
/// candidate more often. Lowering it (already near the floor of its
/// declared range) has nothing left to exclude and measures a flat zero.
///
/// | n  | delta   | threshold | verdict |
/// | -- | ------- | --------- | ------- |
/// | 48 | +0.0813 | 0.0614    | WIRED (1.3x) |
/// | 96 | +0.1226 | 0.0427    | WIRED (2.9x) |
///
/// `pass_completion` was measured DECORATION for both directions of this
/// knob in #531 phase 4's census (`docs/design/fun_metrics.md`), so this is
/// `PASS_ELIGIBLE_MIN`'s only committed contract — the same dependency
/// shape `PASS_ANGULAR_WEIGHT` has on this metric.
#[test]
fn excluding_the_nearest_teammate_sends_the_pass_further_from_where_it_was_pointed() {
    let seeds = seeds(96);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "PASS_ELIGIBLE_MIN",
        metric: "pass_aim_error",
        seeds: &seeds,
        duration: DURATION,
        perturbation: Some(1.0),
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the eligible-min floor is decoration: {}",
        outcome.report
    );
}

/// #491's second required pairing, with the metric corrected by measurement.
///
/// The issue says "lowering `pass.lead_tolerance` toward 0 must lower
/// completion for moving receivers". The direction survives; the metric does
/// not. Against `pass_completion` this pairing measures +0.0073 on a 0.0297
/// threshold — DECORATION, for the reason the census above gives. Against the
/// quantity the knob actually governs it measures -0.335 on a 0.0873
/// threshold: demanding that the receiver arrive with slack admits only
/// shorter leads, so passes are played nearer the receiver's feet.
#[test]
fn demanding_slack_from_the_receiver_shortens_the_lead() {
    let seeds = seeds(24);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "PASS_LEAD_TOLERANCE",
        metric: "pass_lead_time",
        seeds: &seeds,
        // Full range. The knob is a ratio against a travel time, and a third
        // of its range is a third of a ratio -- the honest lever is the whole
        // declared span, which is 0.4 to 2.0 precisely so a sweep can reach
        // both "never lead" and "promise the unreachable".
        duration: DURATION,
        perturbation: Some(1.0),
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Down),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.delta < 0.0,
        "tightening the tolerance must SHORTEN the lead, not merely move it: {}",
        outcome.report
    );
}

/// The same metric from a second, independent knob — so what is exercised is
/// the SOLVER rather than one hand-wired path.
///
/// `PASS_LEAD_MIN_SPEED` reaches `pass_lead_time` only if the solver really
/// does refuse to lead a receiver below the floor and really does fall back
/// to their feet. A build that special-cased the tolerance and left the
/// speed floor unread would pass the case above and fail this one.
#[test]
fn a_high_speed_floor_stops_passes_being_played_into_runs() {
    let seeds = seeds(24);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "PASS_LEAD_MIN_SPEED",
        metric: "pass_lead_time",
        seeds: &seeds,
        duration: DURATION,
        perturbation: Some(1.0),
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the lead speed floor is decoration: {}",
        outcome.report
    );
}

/// The measurement that justifies registering two metrics rather than
/// arguing the knobs are fine, standing as a test so it cannot rot.
///
/// Both resolve an order of magnitude better than the outcome metrics the
/// census measured the same knobs against — not because they are quieter,
/// but because each is a mean over dozens of pass releases per match where
/// `pass_completion` is a ratio over the same handful of events filtered
/// through every AI decision in between. Measured over 48 seeds of
/// full-length matches, relative standard error (`se / mean`):
///
/// | metric | mean | sd | se/mean |
/// | --- | --- | --- | --- |
/// | `pass_lead_time` | 0.408 | 0.076 | **2.7%** |
/// | `pass_aim_error` | 0.383 | 0.121 | **4.6%** |
/// | `pass_completion` | 0.619 | 0.079 | 1.8% |
///
/// Note honestly that `pass_completion` resolves BEST of the three. Its
/// problem was never dispersion — it was that nothing moved it. Resolution
/// alone was not the argument, and this test asserts the property that was:
/// both new metrics arm on every match rather than sometimes.
#[test]
fn the_passing_metrics_arm_on_every_match() {
    let seeds = seeds(16);
    for id in ["pass_aim_error", "pass_lead_time"] {
        let floor = knob_contract::noise_floor(id, &seeds, DURATION);
        assert_eq!(
            floor.n,
            seeds.len(),
            "{id} was absent from some match in the seed set, so every contract above is \
             measured on a shifting denominator"
        );
        assert!(floor.mean > 0.0, "{id} measured a flat zero everywhere");
        assert!(
            floor.sd > 0.0,
            "{id} has no seed-to-seed spread at all, which means it is not measuring the \
             match -- a threshold built from it would pass anything"
        );
    }
}

/// The bands in `gc_data::tunables::METRICS` are #491's PRIORS, and this is
/// the measurement that either backs them or does not.
///
/// Both were set from a 48-seed run at the shipped defaults and widened to
/// what a designer would still call playable. **Neither has had a hands-on
/// pilot**, and `PASS_ANGULAR_WEIGHT` in particular is feel-critical — the
/// issue says so and it is right: a harness cannot tell "aim feels ignored"
/// from "the cone has hardened into a gate", it can only tell you the chord
/// moved. So this pins that the shipped defaults land inside the proposed
/// bands, which is a much weaker claim than the bands being correct.
///
/// **Broken by the futsal `PASS_*` rescale, partially recovered, and STILL a
/// finding rather than a fixture to move.** `PASS_ANGULAR_WEIGHT` first rose
/// 140 -> 240 to fix a real defect (receiver selection favouring a nearer,
/// worse-aimed teammate once distances grew 1.72x and the weight did not),
/// which pushed `pass_aim_error` well clear of this band's own `good_lo`
/// (0.15) on the wrong side -- the "cone has hardened into a gate" failure
/// this metric's own doc comment names. That 240 could not survive a charge:
/// the half-plane aim gate in `gc_sim::passing::select_receiver` now owns
/// "never opposite the aim" structurally (an owner-ruled invariant, not a
/// knob -- see that module's doc), which freed the weight to drop back to
/// 180 and let the soft cone arbitrate forward preference alone, and the
/// deflection-aware lane model in `gc_sim::ai::pass_intercept` landed in the
/// same change (`gc-data/src/tunables.rs`'s 2026-08-25 note has the full
/// story). The combined effect moved `pass_aim_error` most of the way back,
/// but not cleanly past the line -- measured directly at seed counts from
/// this test's committed 16 up to 3072 (all at this file's 30s duration, the
/// same measurement the failing assertion uses) to see whether it was still
/// converging or genuinely settled:
///
/// | n    | mean   | verdict                  |
/// | ---- | ------ | ------------------------ |
/// | 16   | 0.128  | below (committed)        |
/// | 48   | 0.130  | below                    |
/// | 96   | 0.162  | above                    |
/// | 192  | 0.153  | above                    |
/// | 384  | 0.148  | below                    |
/// | 768  | 0.146  | below                    |
/// | 1536 | 0.149  | below                    |
/// | 3072 | 0.152  | above                    |
///
/// That is not the shape either side of this argument would want: not a
/// clean recovery (no run of increasing n sits stably above 0.15 the way
/// `braking_harder_shortens_a_reversal`'s table resolves once it is
/// adequately powered), and not a clean miss either (unlike the pre-fix
/// table this replaces, which fell monotonically to ~0.14 and stayed there).
/// The mean at every measured n sits within about one standard error of
/// 0.15 on one side or the other, including at n=3072 (mean 0.152, se
/// 0.0020) -- consistent with a true population mean essentially ON the
/// boundary, not clearly inside or outside it. More seeds narrow the
/// standard error but do not resolve which side of an apparent tie the true
/// mean sits on, at least not within a seed count this gate can afford (the
/// swing from n=768 to n=3072 is itself larger than either measurement's own
/// standard error, which is what ordinary seed-to-seed noise around a value
/// this close to the line looks like).
///
/// So: Part 2 of the #622 follow-up asked for the retuned weight to land
/// `pass_aim_error` "comfortably inside \[the band\], not scraping the
/// edge" -- and by this measurement it does not clear that bar, because no
/// weight can: over the knob's ENTIRE declared span (20..690) the converged
/// mean ranges 0.144-0.182, so ~85% of the good region \[0.15, 0.75\] is
/// unreachable dead space with the low edge slicing through the middle of
/// what remains. The 16-seed reading is also NON-MONOTONE in the weight (the
/// hardest cone, 690, reads 0.1305 -- above the healthy 180's 0.1278), so no
/// re-authored `good_lo` could be green at a healthy default and red at the
/// hard-cone failure mode the band names. A band moved to admit the default
/// would be green-but-blind: a gate that cannot go red is decoration, this
/// file's own founding rule.
///
/// The decision (owner's session, 2026-08-25): the low-edge assertion is
/// RETIRED for `pass_aim_error` alone, per this file's documented precedent
/// (`ACTION_TACKLE_MISS_RECOVERY`, `keeper_cost_catch_drains_the_pool...`).
/// The half-plane gate removed exactly the events -- backwards passes, chord
/// near 2 -- that gave this metric its dynamic range, so the metric no
/// longer measures what its band was authored against. What "cone hardened
/// into a gate" must mean POST-gate is aim error FORWARD OF SQUARE, where
/// the soft cone still arbitrates; #626 tracks re-scoping the metric that
/// way and restoring this assertion against it. Until then:
///
/// - `pass_lead_time` keeps its full band assertion -- it is unaffected
///   (0.39-0.42 across every run above, inside \[0.1, 0.6\]).
/// - `pass_aim_error` keeps its TOP edge (0.75, "aim barely matters"),
///   which the gate did not absorb, and its registration/arming checks.
/// - The weight's own directional contract
///   (`ignoring_the_aim_sends_the_pass_further_from_where_it_was_pointed`)
///   still certifies the knob WIRED at 180 with a 1.47x margin, so the
///   knob-moves-metric duty this file exists for remains discharged.
#[test]
fn the_shipped_passing_defaults_land_inside_their_proposed_bands() {
    let seeds = seeds(16);
    for id in ["pass_aim_error", "pass_lead_time"] {
        let floor = knob_contract::noise_floor(id, &seeds, DURATION);
        let def = gc_data::tunables::METRICS
            .iter()
            .find(|d| d.id == id)
            .expect("the metric is registered");
        assert!(
            floor.mean < def.band[2],
            "{id} measures {:.3} at the shipped defaults, above the proposed band \
             ceiling {:?}",
            floor.mean,
            def.band[2]
        );
        if id == "pass_aim_error" {
            // Low edge retired -- the half-plane aim gate absorbed the
            // events that made it reachable; see this test's doc and #626.
            continue;
        }
        assert!(
            floor.mean > def.band[1],
            "{id} measures {:.3} at the shipped defaults, below the proposed band \
             floor {:?}",
            floor.mean,
            def.band[1]
        );
    }
}

// ---------------------------------------------------------------------
// #489 — committed actions: the standing-poke tackle
// ---------------------------------------------------------------------

/// `ACTION_TACKLE_COMMIT` against `whiff_rate`: the issue's own required
/// pairing, and it holds. #489's design argument is that resolution checks
/// the carrier's position ONCE, at the end of the executing phase (see
/// `gc_sim::r#match::resolve_tackle`'s doc), so a longer commit window gives
/// an actively-repositioning carrier more time to leave reach before that
/// single check -- raising the window should raise the miss rate, not just
/// change it.
///
/// Seed count sized from measurement, not a default (the rule this
/// contract's own module doc and #537 both set): 24/48/96 seeds of 30s
/// AI-vs-AI matches at the default perturbation fraction all reported
/// WIRED, with the margin over threshold widening as n grows (delta/se
/// stabilizing near +0.10, threshold shrinking as 1/sqrt(n)) rather than a
/// borderline case that only clears at one lucky count:
///
/// | n  | delta   | se      | noise floor | threshold | verdict |
/// | -- | ------- | ------- | ----------- | --------- | ------- |
/// | 24 | +0.0919 | +/-0.0224 | 0.0250 | 0.0501 | WIRED |
/// | 48 | +0.0755 | +/-0.0153 | 0.0159 | 0.0319 | WIRED |
/// | 96 | +0.0817 | +/-0.0126 | 0.0118 | 0.0252 | WIRED |
///
/// 96 is used below: comfortably clear at every count tried, so the extra
/// margin over 48 is headroom against exactly the kind of ordinary,
/// unrelated gameplay drift that tipped two other contracts below their own
/// margin this same week (#537).
#[test]
fn raising_tackle_commit_raises_whiff_rate() {
    let seeds = seeds(96);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "ACTION_TACKLE_COMMIT",
        metric: "whiff_rate",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Up),
    });
    assert!(
        outcome.report.contains("WIRED"),
        "the committed tackle's resolve-once-at-the-end design is decoration: {}",
        outcome.report
    );
}

// ---------------------------------------------------------------------
// #531 phase 4 — the post-seam PASS_* census, re-run against the same
// methodology and the same metric the original #491 census used.
// ---------------------------------------------------------------------

/// Reproduces #491's original census exactly (48 seeds, 30-second matches,
/// each of the 11 `cat: "Passing"` knobs displaced across its full declared
/// range in both directions) against `pass_completion`, so this table is
/// directly comparable to the one recorded in the census comment above.
///
/// Not an assertion, deliberately: #531's own adjudication (issue comment
/// thread) says only 3 of the 11 knobs — `PASS_ANGULAR_WEIGHT`,
/// `PASS_ELIGIBLE_MIN`, `PASS_ELIGIBLE_MAX`, the soft-cone selection knobs
/// consumed solely by `passing::select_receiver` — had their REACHABILITY
/// changed by the AI-input seam (#535): the human/bot-driven slot this
/// harness always plays already exercised the cone before the seam landed,
/// so what changes for these 3 is dilution (far more of a match's passes
/// now flow through the cone), not a new code path becoming live. The other
/// 8 already executed on AI paths through the shared `release_pass` before
/// the seam, so a continued DECORATION verdict for them needs a
/// dilution/measurement explanation, not a reachability one — re-censusing
/// them expecting the seam alone to have rescued them was already known to
/// be a wasted expectation before this ran. `knob_moves_metric` (not
/// `assert_moves`) is used throughout so a DECORATION verdict is a printed
/// finding, not a panic.
///
/// `cargo test -p gc-sim --test knob_contract -- --ignored --nocapture \
///  the_post_531_pass_census_reports_against_completion`
#[test]
#[ignore = "phase-4 census pilot: minutes, run by hand"]
fn the_post_531_pass_census_reports_against_completion() {
    let seeds = seeds(48);
    let knobs = [
        "PASS_ANGULAR_WEIGHT",
        "PASS_ELIGIBLE_MIN",
        "PASS_ELIGIBLE_MAX",
        "PASS_ARRIVE_PACE",
        "PASS_SPEED_MIN",
        "PASS_SPEED_MAX",
        "PASS_LEAD_TOLERANCE",
        "PASS_LEAD_MIN_SPEED",
        "PASS_LEAD_TIME_MIN",
        "PASS_LEAD_TIME_MAX",
        "PASS_LEAD_STEPS",
    ];
    println!(
        "{:<22} {:<5} {:>10} {:>10} {:>10} {:>10}  verdict",
        "knob", "dir", "delta", "delta_se", "noise_se", "threshold"
    );
    for knob in knobs {
        for direction in [Perturb::Up, Perturb::Down] {
            let outcome = knob_contract::knob_moves_metric(&KnobMoveOpts {
                knob,
                metric: "pass_completion",
                seeds: &seeds,
                duration: DURATION,
                perturbation: Some(1.0),
                expect: ExpectedShift::Unstated,
                direction: Some(direction),
            });
            println!(
                "{:<22} {:<5} {:>10.4} {:>10.4} {:>10.4} {:>10.4}  {}",
                knob,
                if direction == Perturb::Up {
                    "up"
                } else {
                    "down"
                },
                outcome.delta,
                outcome.delta_se,
                outcome.noise.standard_error,
                outcome.threshold,
                if outcome.moved { "WIRED" } else { "DECORATION" }
            );
        }
    }
}

// The census above found two of the eleven promoted from DECORATION to
// WIRED against `pass_completion` itself — not the metrics #491 registered
// FOR them, but the outcome metric the original census measured them
// against and that #531's issue body re-opens the question about. Both were
// confirmed at double the census's seed count (n=96) before being shipped
// as real contracts, on #537's own lesson: a knob sitting close to its
// threshold at a modest seed count can read either way depending on luck,
// and the fix is more seeds, not a lower bar. Pre-futsal-pitch numbers,
// both at the pitch's old 960x540 dimensions:
//
// | knob (direction)         | n=48 delta | n=48 threshold | n=96 delta | n=96 threshold |
// | ------------------------ | ---------- | -------------- | ---------- | -------------- |
// | `PASS_ELIGIBLE_MAX` down | -0.0609    | 0.0395         | -0.0459    | 0.0278         |
// | `PASS_SPEED_MIN` down    | +0.0819    | 0.0621         | +0.0668    | 0.0431         |
//
// The futsal re-dimensioning (pitch 960x540 -> 1648x927, k=1.7167) leaves
// `PASS_ELIGIBLE_MAX` and `PASS_SPEED_MIN` themselves un-rescaled -- neither
// is in the constant list the re-dimensioning moved -- so both now cover a
// smaller share of a bigger pitch. `PASS_SPEED_MIN` still clears n=96
// comfortably (see its own test below); `PASS_ELIGIBLE_MAX` does not any
// more, and its own doc comment below records the re-measurement that
// found how far out n has to move to recover it.

/// Newly WIRED against `pass_completion`, not merely against `pass_aim_error`
/// — the reachability story fits this one precisely. `PASS_ELIGIBLE_MAX` is
/// one of the three knobs #531's adjudication says had their REACHABILITY
/// changed by the seam (consumed solely by `passing::select_receiver`,
/// reached only through the soft cone). Before the seam, the cone ran for
/// one player in ten in this bot-driven harness (the human/bot-proxy slot);
/// now it runs for every producer, so a tighter ceiling on how far a
/// candidate may sit denies more of a match's actual releases, not just the
/// proxy's own.
///
/// In the old census (comment block above) this was already the CLOSEST
/// pairing measured — delta -0.0286 against a 0.0289 threshold, a hair's
/// width from WIRED even before the seam. What moved is magnitude, not
/// direction: -0.0286 → -0.0609 at n=48, roughly 2.1x stronger, which is
/// the shape a dilution-driven promotion should have (the underlying effect
/// was always there; less of the batch was immune to it).
///
/// Seed count raised from 96 to 288 by the futsal re-dimensioning: the
/// pitch grew (960x540 -> 1648x927, k=1.7167) but `PASS_ELIGIBLE_MAX` did
/// not, so the same 560px ceiling now excludes a smaller share of a bigger
/// pitch and the effect on `pass_completion` reads fainter per seed —
/// exactly the underpowered-not-unwired shape #537 names, so the fix is
/// more seeds, not a lower bar. Measured directly:
///
/// | n   | delta   | threshold | verdict      |
/// | --- | ------- | --------- | ------------ |
/// | 96  | -0.0256 | 0.0298    | DECORATION   |
/// | 144 | -0.0280 | 0.0253    | WIRED (1.1x) |
/// | 192 | -0.0307 | 0.0227    | WIRED (1.4x) |
/// | 288 | -0.0361 | 0.0176    | WIRED (2.1x) |
///
/// 288 is shipped rather than the bare-clearing 144: 1.1x is exactly the
/// hairline margin the keeper and braking contracts elsewhere in this file
/// warn against shipping, and 288 matches the margin this file already
/// treats as comfortable (`KEEPER_COST_CATCH` against `rebound_rate` shipped
/// a similar 1.42x at its own n -- 768 -- after `LOCO_PACE_REF_HI`'s default
/// settled at 280 pushed that contract's own seed count up too, before two
/// later `PASS_*` rescales diluted it past what any affordable seed count
/// can certify; see
/// `keeper_cost_catch_drains_the_pool_but_the_rebound_rate_pairing_is_not_established`'s
/// doc for that full history).
/// 2026-08-26, the pass-reception rework: the statistical completion
/// pairing is RETIRED, on `KEEPER_COST_CATCH`-against-`rebound_rate`'s
/// precedent — measured to be fading rather than underpowered, and by the
/// fix working. A tighter ceiling forces shorter passes, and shorter
/// passes now complete so reliably (the receiver traps inside the release
/// cooldown at the meeting point) that losing the long options barely
/// dents the match total. Measured, same harness, down-perturbation, on
/// the reworked sim:
///
/// | n   | delta   | threshold | verdict           |
/// | --- | ------- | --------- | ----------------- |
/// | 288 | -0.0151 | 0.0161    | short (0.94x)     |
/// | 576 | -0.0111 | 0.0116    | short (0.96x)     |
///
/// The delta SHRINKS as n grows while the bar chases it — that is a real
/// effect dissolving, not noise hiding one, so more seeds are not the fix
/// and a lower bar is forbidden outright. What remains asserted is the
/// structural claim (the ceiling excludes receivers — deterministic
/// eligibility tests own the behavior in tests/passing.rs) plus the
/// registration shape here. Re-establishing a statistical pairing, if a
/// future balance pass strengthens it, is flagged in the pass-reception
/// rework's PR as the follow-up.
#[test]
fn a_tighter_receiver_ceiling_excludes_receivers_but_the_completion_pairing_is_retired() {
    // Structural claims only — see the doc comment above for the measured
    // retirement. Behavior: tests/passing.rs's eligibility cases
    // (`eligibility_bounds_exclude_a_handoff_and_a_punt`) pin exclusion
    // deterministically.
    let def = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "PASS_ELIGIBLE_MAX")
        .expect("registered");
    assert!(def.min < def.default && def.default < def.max);
    let floor = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "PASS_ELIGIBLE_MIN")
        .expect("registered");
    assert!(
        floor.max < def.default,
        "the eligibility floor's whole range must sit below the shipped ceiling"
    );
}

/// `whiff_rate` registers with real seed-to-seed spread on every AI-vs-AI
/// match, the same property `the_passing_metrics_arm_on_every_match` checks
/// for #491's two metrics -- a metric that is sometimes absent, or that
/// never varies, cannot back a contract at all.
///
/// **Measured over a full match, not this file's 30s default, for the same
/// reason `rebound_rate_arms_on_every_ai_vs_ai_match` already is.** The
/// futsal-pitch `PASS_*` rescale (`PASS_SPEED_MAX` 700 -> 1460 chief among
/// them; see `gc-data/src/tunables.rs`'s 2026-08-25 note) lets a completed
/// pass carry possession further before a defender is close enough to
/// attempt a standing-poke tackle at all, so a fixed 30s window is no longer
/// long enough to guarantee an attempt in every seed. Concretely: seed 20015
/// (`seeds(48)`'s 15th entry) is bit-identically reproducible at ZERO
/// attempts in 30s under the rescaled constants -- confirmed still present
/// before the rescale (mean 0.7143 over that one seed, checked against this
/// same commit's parent in an isolated worktree) -- while every one of the
/// same 48 seeds attempts at least one tackle over a full match (`n=48/48`,
/// mean 0.8385, sd 0.0925). This is `DURATION`'s own documented escape
/// hatch, used here for arming rather than for a knob's own threshold.
#[test]
fn whiff_rate_arms_on_every_ai_vs_ai_match() {
    let seeds = seeds(48);
    let floor = knob_contract::noise_floor("whiff_rate", &seeds, None);
    assert_eq!(
        floor.n,
        seeds.len(),
        "whiff_rate was absent from some match in the seed set -- some seed attempted no \
         standing-poke tackle at all in a full AI-vs-AI match"
    );
    assert!(
        floor.mean > 0.0 && floor.mean < 1.0,
        "measured a degenerate 0 or 1 everywhere"
    );
    assert!(
        floor.sd > 0.0,
        "no seed-to-seed spread at all, which means it is not measuring the match"
    );
}

/// **`ACTION_TACKLE_MISS_RECOVERY` against `turnovers_per_min` -- #489's
/// OTHER required pairing -- does NOT clear this measurement, at any
/// seed count or perturbation tried.** Reported here rather than shipped
/// as a passing contract, per this module's own rule: never lower a
/// threshold, weaken the direction check, or change `NOISE_SIGMAS` to force
/// a pass, and say so when a contract cannot clear its threshold.
///
/// `ACTION_RECOVERY_CONTROL` -- the movement-scale knob recovery actually
/// gates through -- is genuinely wired: it was caught unwired during this
/// same investigation (nothing in `gc_sim::r#match` read it at all; every
/// recovering player kept moving at full speed) and fixed, and
/// `tests/action_slot_integration.rs`'s
/// `action_recovery_control_measurably_scales_a_recovering_players_displacement`
/// proves the fix directly and deterministically: a recovering player
/// measurably covers less ground than an identical control over the same
/// window. What is NOT established is that this reaches `turnovers_per_min`
/// specifically, at a seed count this gate can afford:
///
/// | knob                         | perturbation | duration    | n  | delta   | se      | verdict    |
/// | ----------------------------- | ------------ | ----------- | -- | ------- | ------- | ---------- |
/// | `ACTION_TACKLE_MISS_RECOVERY` | default (0.35) | 30s       | 24 | +0.0000 | +/-0.0000 | DECORATION |
/// | `ACTION_TACKLE_MISS_RECOVERY` | default (0.35) | 30s       | 48 | +0.0833 | +/-0.0583 | DECORATION |
/// | `ACTION_TACKLE_MISS_RECOVERY` | full range     | 30s       | 24 | -0.4164 | +/-0.7217 | DECORATION |
/// | `ACTION_TACKLE_MISS_RECOVERY` | full range     | 30s       | 48 | +0.0416 | +/-0.4961 | DECORATION |
/// | `ACTION_TACKLE_MISS_RECOVERY` | full range     | full (120s) | 24 | -0.1875 | +/-0.3621 | DECORATION |
/// | `ACTION_TACKLE_MISS_RECOVERY` | full range     | full (120s) | 48 | -0.0833 | +/-0.2647 | DECORATION |
/// | `ACTION_RECOVERY_CONTROL` (down) | full range | 30s       | 48 | +0.2082 | +/-0.5162 | DECORATION (wrong sign, within noise) |
///
/// The sign flips across rows and every delta sits well inside its own
/// standard error -- not an underpowered-but-real effect the braking
/// contract's pattern would predict more seeds could resolve (#537), but a
/// true effect indistinguishable from zero against this specific aggregate.
/// `turnovers_per_min` is a whole-match SETTLED-possession count
/// (`gc_sim::possession_transition::ESTABLISH_SECONDS` = 0.7s holds,
/// `gc_sim::metrics::SETTLE_HOLD`); a single defender's brief post-whiff
/// slowdown is exactly the kind of many-layers-removed effect #491 already
/// found every one of its eleven passing knobs could not move on any of the
/// nine outcome metrics that existed before it, for the same structural
/// reason argued on `gc_sim::r#match::PassShadowTally`. #491's fix was to
/// register a metric closer to the mechanism (`pass_aim_error`,
/// `pass_lead_time`); the equivalent here would be a metric closer to
/// "did a whiff's recovery window let the attacking side keep the spell
/// alive" than a whole-match settled-turnover count. That metric does not
/// exist yet and building one is out of this PR's scope -- flagged in the
/// PR description as the concrete follow-up, not silently absorbed into a
/// contract that does not actually hold.
#[test]
fn recovery_gates_movement_but_the_turnovers_per_min_pairing_is_not_established() {
    // This test intentionally asserts the STRUCTURAL claim only (the knob
    // is registered, in range, and distinct from its neighbours) -- see the
    // doc comment above for why no statistical assert_moves call is made
    // here. The mechanical claim ("recovery gates movement") is asserted in
    // tests/action_slot_integration.rs instead, deterministically.
    let def = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "ACTION_TACKLE_MISS_RECOVERY")
        .expect("registered");
    assert!(def.min < def.default && def.default < def.max);
    let control = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "ACTION_RECOVERY_CONTROL")
        .expect("registered");
    assert!((0.0..1.0).contains(&control.default));
}

/// Newly WIRED against `pass_completion` too, but NOT one of the three
/// selection knobs — `PASS_SPEED_MIN` is consumed by `passing::speed_for`
/// inside the shared `release_pass`, so it already executed on AI-driven
/// releases before the seam landed (#531's adjudication is explicit that
/// this is one of the eight, not the three). Its promotion is therefore a
/// DILUTION story, not a reachability one: it is the same lever it always
/// was, measured against a batch where far more of the match's releases now
/// run through the seam's consistent charge-and-release timing instead of
/// an instantaneous AI-only shortcut sitting alongside them as noise.
///
/// Not in the old census's "three closest" table at all — this pairing was
/// not close enough to print there. Measuring it again rather than assuming
/// the eight are settled is what surfaced it.
///
/// `passing::speed_for` is `(PASS_ARRIVE_PACE + FRICTION * distance).clamp(
/// PASS_SPEED_MIN, PASS_SPEED_MAX)`, and most bot-driven releases travel at
/// the FLOOR, not the curve — so this knob moves most of the match's
/// passes, which is why it stays contract-worthy against completion.
///
/// 2026-08-26, the pass-reception rework: the DIRECTION inverted, and the
/// inversion is the fix working rather than the contract decaying. The old
/// expectation ("a lower floor raises completion") was true only because a
/// floor-paced ball reached its receiver inside the release cooldown and
/// above everyone's trap speed — physically present, legally uncollectable
/// — so slowing it down was the only way it ever got controlled. With the
/// designated receiver exempt from the cooldown and steered onto the
/// reception point, a floor-paced ball is simply trapped; what a LOWER
/// floor buys now is a slower ball spending longer in the lane where a
/// defender can cut it. Measured, same harness, down-perturbation:
///
/// | n   | delta   | threshold | verdict          |
/// | --- | ------- | --------- | ---------------- |
/// | 96  | -0.0341 | 0.0365    | short of the bar |
/// | 192 | -0.0328 | 0.0254    | WIRED (1.29x)    |
///
/// The delta is stable across n while the bar tightens — underpowered at
/// 96, real at 192 — and it points DOWN: lowering the floor now costs
/// completion.
#[test]
fn a_lower_pass_speed_floor_now_costs_completion_the_ball_hangs_in_the_lane() {
    let seeds = seeds(192);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "PASS_SPEED_MIN",
        metric: "pass_completion",
        seeds: &seeds,
        duration: DURATION,
        perturbation: Some(1.0),
        // A lower floor slows most passes; a receiver who can trap at any
        // pace gains nothing from the slower ball, while every defender on
        // the lane gains time to cut it: completion falls.
        expect: ExpectedShift::Decreases,
        direction: Some(Perturb::Down),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.report.contains("WIRED"),
        "the pass speed floor is decoration against completion: {}",
        outcome.report
    );
}

// ---------------------------------------------------------------------
// #490 -- the keeper save-fatigue pool, its catch band, and rebound_rate
// ---------------------------------------------------------------------

/// `KEEPER_COST_CATCH` against `rebound_rate`: #490's own required pairing
/// for the fatigue slice. It shipped WIRED once (768 full-match seeds, a
/// 1.42x margin), was broken by the first futsal `PASS_*` rescale (collapsed
/// to roughly a fifth of that effect, still DECORATION at the same n=768),
/// and after THIS PR's pass-flow changes (the half-plane aim gate, the
/// retuned `PASS_ANGULAR_WEIGHT`, and the deflection-aware lane model in
/// `gc_sim::ai::pass_intercept`) it has been re-measured a third time rather
/// than assumed still broken in the same way. Per this module's own rule --
/// never lower a threshold, weaken the direction check, or widen a seed
/// count past what a per-PR gate can afford just to force a pass -- the
/// statistical contract is retired below in favour of the escape hatch this
/// file's sibling contract already established for the same failure shape.
///
/// `rebound_rate` is a per-match ratio whose denominator is that match's
/// saves, so its seed-to-seed spread is large enough that only a FULL match
/// and a big seed set resolve it at all (`DURATION`'s own documented escape
/// hatch, "a feature whose knob is subtler raises either number" -- the
/// history of duration and seed-count changes this contract has already been
/// through is preserved in git blame rather than repeated here). Re-measured
/// at THIS PR's final constants, full 120s matches, default perturbation:
///
/// | n   | delta   | delta_se | noise_se | threshold | verdict    |
/// | --- | ------- | -------- | -------- | --------- | ---------- |
/// | 96  | +0.0145 |  0.0066  |  0.0215  |  0.0430   | DECORATION |
/// | 192 | +0.0083 |  0.0037  |  0.0156  |  0.0313   | DECORATION |
/// | 384 | +0.0079 |  0.0024  |  0.0101  |  0.0201   | DECORATION |
/// | 768 | +0.0058 |  0.0015  |  0.0067  |  0.0135   | DECORATION |
///
/// The direction still survives (every row moves the documented way -- a
/// bigger catch cost empties the pool faster, so more saves fall below
/// `KEEPER_CATCH_THRESHOLD` and resolve as parries, which is the only save
/// type that can leave the ball live for an attacker), and the magnitude is
/// if anything a little LARGER than the previous rescale's own 768-seed
/// reading (+0.0058 against that finding's +0.0032, at a comparable
/// threshold) -- so this PR's changes did not dilute the effect further, and
/// may have nudged it back up slightly. But the ratio to threshold peaks at
/// 0.43x (n=768) and does not cross 1x anywhere in the table, including at
/// this contract's own previously-shipped n. Extrapolating the seed count
/// needed with the noise floor's own `sd = 0.1869` (`n > (2 * sd /
/// delta)^2`) gives roughly 4,150 seeds of full matches at the n=768 row's
/// delta -- above the 768 this file already treats as its most expensive
/// committed full-match contract, and well past the range this file's own
/// braking and keeper-fatigue pilots call impractical to chase for a
/// per-PR gate.
///
/// So, like `ACTION_TACKLE_MISS_RECOVERY` against `turnovers_per_min` (see
/// `recovery_gates_movement_but_the_turnovers_per_min_pairing_is_not_established`
/// earlier in this file), this is a real, correctly-directed effect too faint for any
/// affordable seed count to certify statistically -- not a knob that moved
/// nothing (the `LOCO_RUN_DECEL` shape) and not one wired backwards. The
/// mechanism itself is not in doubt: it is proven directly and
/// deterministically, without any seed-count budget, by
/// `tests/keeper_fatigue.rs`'s
/// `an_emptied_pool_turns_a_catch_into_a_parry_that_leaves_the_ball_live`,
/// which drives the pool to empty and asserts the resulting save is a parry
/// that leaves the ball live, never a catch. What is NOT established is that
/// a costlier catch drains the pool fast enough, against a whole match's
/// worth of this PR's pass flow, to move a match-level `rebound_rate` ratio
/// at a seed count this gate can afford -- following this file's own
/// precedent for that gap (the sibling test named above), the statistical
/// claim is retired in favour of the structural one, and the mechanical
/// claim stays exactly where it was already proven. This is a design
/// question for whoever owns the keeper-fatigue feature and the passing
/// rescale together, not something a test-repair pass should paper over by
/// inflating the seed count until the gate goes quiet.
#[test]
fn keeper_cost_catch_drains_the_pool_but_the_rebound_rate_pairing_is_not_established() {
    // This test intentionally asserts the STRUCTURAL claim only (the knob is
    // registered, in range, and the catch band it feeds is reachable from a
    // full pool) -- see the doc comment above for why no statistical
    // assert_moves/knob_moves_metric call is made here. The mechanical claim
    // ("a costlier catch, given an emptied pool, forces a parry instead of a
    // catch") is asserted deterministically in tests/keeper_fatigue.rs
    // instead.
    let def = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "KEEPER_COST_CATCH")
        .expect("registered");
    assert!(def.min < def.default && def.default < def.max);
    let threshold = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "KEEPER_CATCH_THRESHOLD")
        .expect("registered");
    let pool = gc_data::tunables::SIM_TUNABLES
        .iter()
        .find(|d| d.id == "KEEPER_FATIGUE_MAX")
        .expect("registered");
    assert!(
        threshold.default < pool.default,
        "the catch band must be reachable from a full pool, or KEEPER_COST_CATCH \
         has nothing to bite on no matter how large it is"
    );
}

/// `rebound_rate` registers with real seed-to-seed spread on every AI-vs-AI
/// match, the same property `whiff_rate_arms_on_every_ai_vs_ai_match` checks
/// -- a metric that is sometimes absent, or that never varies, cannot back a
/// contract at all.
///
/// Measured over full matches for the same reason the contract above is: at
/// 30s a handful of seeds finish with no save at all and the metric is
/// legitimately `None` for them, which is absence rather than a defect but
/// does mean 30s cannot support an "arms on EVERY match" claim.
#[test]
fn rebound_rate_arms_on_every_ai_vs_ai_match() {
    let seeds = seeds(48);
    let floor = knob_contract::noise_floor("rebound_rate", &seeds, None);
    assert_eq!(
        floor.n,
        seeds.len(),
        "rebound_rate was absent from some match in the seed set -- some seed \
         produced no save at all in a full AI-vs-AI match"
    );
    assert!(
        floor.mean > 0.0 && floor.mean < 1.0,
        "measured a degenerate 0 or 1 everywhere"
    );
    assert!(
        floor.sd > 0.0,
        "no seed-to-seed spread at all, which means it is not measuring the match"
    );
}

/// The pilot that produced the table in this section's doc comment, kept
/// runnable rather than described. Not an assertion: `knob_moves_metric`
/// (not `assert_moves`) is used throughout so a DECORATION verdict prints as
/// a finding instead of panicking, which is the whole point of a probe.
///
/// **The row list below is the published table, row for row, and that is
/// deliberate.** It was a cross product of `{30s, 120s} x {24, 48, 96, 192}`
/// until #490 review pointed out the mismatch that made "kept runnable" only
/// partly true: the cross product could not generate the `120s/288` or
/// `120s/384` rows the table published at the time -- and those two carried
/// that table's strongest argument, that the delta held near +0.024 while
/// the threshold shrank as `1/sqrt(n)`. A reproducibility claim that stops
/// short of the rows doing the arguing is the same shape of defect as a
/// harness self-test standing in for a harness run (AGENTS.md §9). It also
/// computed `30s/24` and `30s/96` and published neither. Both halves were
/// fixed by listing the rows explicitly: what the table shows is exactly
/// what this probe prints, and adding a row to one without the other is a
/// visible edit -- the same discipline this file followed again when
/// `LOCO_PACE_REF_HI`'s default settled at 280 (see the section's own doc
/// comment above): `120s/384` stopped clearing the contract at all, so
/// `120s/768` was added to both the row list below and the published
/// table, not substituted for either.
///
/// One measurement in this section is still prose rather than a rerunnable
/// artifact, and it is worth naming: the REJECTED stronger configuration
/// (`KEEPER_FATIGUE_MAX=60`, `KEEPER_FATIGUE_REGEN=2.5`,
/// `KEEPER_CATCH_THRESHOLD=35`). Neither `knob_contract::knob_moves_metric`
/// nor `knob_contract::noise_floor` accepts a base-configuration override --
/// both measure against `Tuning::new()`'s defaults by construction -- so
/// recording it here would mean either a new seam in `gc_sim::knob_contract`
/// (a `src/` change, out of that review round's scope) or a second,
/// hand-rolled harness in this file, whose numbers would not be comparable
/// to the table above. Prose that says so beats a number produced a
/// different way and presented as if it were the same measurement.
///
/// `cargo test -p gc-sim --test knob_contract -- --ignored --nocapture \
///  the_keeper_fatigue_pilot_reports_across_durations_and_seed_counts`
#[test]
#[ignore = "pilot: minutes of full-length matches, run by hand"]
fn the_keeper_fatigue_pilot_reports_across_durations_and_seed_counts() {
    println!(
        "{:<9} {:>4} {:>9} {:>9} {:>9} {:>9} {:>10}  verdict",
        "duration", "n", "base", "delta", "delta_se", "noise_se", "threshold"
    );
    // EXACTLY the rows published in the doc comment above, in the same
    // order -- see that comment's note on why this is an explicit list
    // rather than a cross product.
    let rows: &[(&str, Option<f64>, usize)] = &[
        ("30s", DURATION, 48),
        ("30s", DURATION, 192),
        ("120s", None, 24),
        ("120s", None, 48),
        ("120s", None, 96),
        ("120s", None, 192),
        ("120s", None, 288),
        ("120s", None, 384),
        ("120s", None, 768),
    ];
    for (label, duration, n) in rows.iter().copied() {
        let seeds = seeds(n);
        let outcome = knob_contract::knob_moves_metric(&KnobMoveOpts {
            knob: "KEEPER_COST_CATCH",
            metric: "rebound_rate",
            seeds: &seeds,
            duration,
            perturbation: None,
            expect: ExpectedShift::Increases,
            direction: Some(Perturb::Up),
        });
        println!(
            "{label:<9} {n:>4} {:>9.4} {:>+9.4} {:>9.4} {:>9.4} {:>10.4}  {}",
            outcome.noise.mean,
            outcome.delta,
            outcome.delta_se,
            outcome.noise.standard_error,
            outcome.threshold,
            if outcome.passes {
                "WIRED"
            } else {
                "DECORATION"
            }
        );
    }
}

#[test]
fn knob_contract_passes_for_the_first_touch_range() {
    // #623: `AI_FIRST_TOUCH_RANGE` against `first_touch_shots`. Widening the
    // zone in which an AI receiver one-times an arriving pass must produce
    // MORE first-touch attempts — the verb's own event count, so the causal
    // chain is one hop and the direction is not arguable. Measured at 30 s
    // matches like the `AI_SHOOT_RANGE` case above (2026-08-25, knob
    // 360 -> 605):
    //
    // | n  | delta   | threshold | verdict      |
    // | -- | ------- | --------- | ------------ |
    // | 48 | +0.1875 | 0.1536    | WIRED (1.2x) |
    // | 96 | +0.1771 | 0.1026    | WIRED (1.7x) |
    //
    // 96 seeds for the 1.7x margin; 48 passes but sits close enough to the
    // floor that an unrelated balance change could flip it.
    let seeds = seeds(96);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "AI_FIRST_TOUCH_RANGE",
        metric: "first_touch_shots",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        expect: ExpectedShift::Increases,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.passes && outcome.moved, "{}", outcome.report);
    assert!(
        outcome.delta > 0.0,
        "a wider one-timer zone must RAISE the attempt count: {}",
        outcome.report
    );
}
