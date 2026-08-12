//! The knob-moves-metric contract: proof that a registered tunable is wired.
//!
//! > **Every feature ships a test asserting that moving its knob moves its
//! > metric.** Run the harness at the default and at a perturbed value; assert
//! > the registered metric moves in the documented direction by more than the
//! > measured noise floor. A knob that cannot shift its own metric is not
//! > wired — it is decoration, and it fails review.
//!
//! This is the balance counterpart of AGENTS.md §9's rule that every CI gate
//! ships a demonstration it can go red. A sweep over dead knobs produces
//! confident nonsense: at the time this module landed, `VOLLEY_SKY_P`,
//! `REPLAY_SLOWMO` and `REPLAY_SECONDS` were registered, swept, and read by no
//! simulation code at all. `tests/knob_contract.rs` uses one of them as this
//! helper's own red demonstration.
//!
//! ## What it measures
//!
//! Two headless batches on the SAME seed set (common random numbers), one at
//! the knob's default and one at `default ± perturbation × (max - min)`,
//! clamped to the knob's range. The simulation is deterministic, so two runs
//! of the *same* config on the same seeds are bit-identical and the paired
//! per-seed difference is exactly zero — there is no run-to-run noise to
//! measure. The noise that matters is **seed-to-seed spread**: a small seed
//! set can show a shift that a different seed set would not, so the threshold
//! is built from the metric's own dispersion at defaults.
//!
//! [`noise_floor`] measures that dispersion — the standard error of the
//! metric's mean over the seed set at defaults — and [`knob_moves_metric`]
//! requires the paired mean shift to clear
//! `NOISE_SIGMAS × max(defaults standard error, paired standard error)`.
//! Both terms are needed: the first catches a metric that is simply noisy,
//! the second catches a knob whose effect is real on average but wildly
//! inconsistent per seed.
//!
//! ## Direction is part of the contract, not a per-test afterthought
//!
//! AGENTS.md §9's rule says the metric must move "in the documented
//! direction", so [`KnobMoveOpts::expect`] is where that direction is
//! documented and [`knob_moves_metric`] is what enforces it. A knob wired
//! BACKWARDS — a keeper-reach knob that lowers `save_rate` when raised — clears
//! any magnitude-only threshold comfortably; it is a bug that looks exactly
//! like a success. [`ExpectedShift`] is deliberately a different type from
//! `gc_data::tunables::MetricDirection`: that one is the metric's own
//! desirability slope (which way is *good*), this one is the knob-to-metric
//! causal claim the feature is making (which way this knob *pushes*).
//!
//! ## Harness knobs, not tier-1 knobs
//!
//! [`DEFAULT_PERTURBATION_FRACTION`], [`DEFAULT_NOISE_FLOOR_MATCHES`] and
//! [`NOISE_SIGMAS`] configure the *contract*, not the game. They are
//! deliberately not registered tunables: nothing in a match reads them, they
//! must not enter the config hash, and a sweep varying them would be varying
//! its own measuring stick.
//!
//! ## The default perturbation fraction is provisional
//!
//! The issue that asked for this helper flagged the perturbation fraction as
//! untrustworthy without a measured per-metric variance. [`noise_floor`] is
//! the pilot: it is a real measurement, run per invocation on the caller's own
//! seed set, so the *threshold* is never a guess. The perturbation *fraction*
//! still is, and 0.35 is a conservative default rather than a derived one —
//! see its doc comment.

use crate::headless::{self, BatchOpts};
use crate::metric_registry;
use crate::tunable_registry::{self, format_g6};

/// Default knob displacement, as a fraction of the knob's `[min, max]` range.
///
/// **Provisional.** The issue's suggested range is 0.2–0.5. 0.35 sits in the
/// middle: large enough that a genuinely wired knob clears the measured noise
/// floor on a modest seed set, small enough that it stays inside the range
/// a designer would consider plausible rather than pinning the knob to its
/// fence (where several knobs are known to saturate — see
/// `docs/design/fun_metrics.md`'s phase-3 note about optima sitting on old
/// fences). It is not derived from a variance pilot, because the quantity a
/// pilot would inform is the *threshold*, and [`noise_floor`] measures that
/// directly instead. Raise it per-call via [`KnobMoveOpts::perturbation`] when
/// a knob is known to be coarse.
pub const DEFAULT_PERTURBATION_FRACTION: f64 = 0.35;

/// Default matches per point when estimating a metric's noise floor. The
/// issue's suggested range is 20–200; 40 is the smallest count at which the
/// standard error of the mean is stable enough to threshold against, at a
/// cost a feature test can afford.
pub const DEFAULT_NOISE_FLOOR_MATCHES: usize = 40;

/// How many standard errors a shift must clear to count as a move. Two is the
/// conventional "outside the noise" line and keeps a false pass from a knob
/// that nudges a metric by less than the seed set can resolve.
pub const NOISE_SIGMAS: f64 = 2.0;

/// The smallest seed set [`knob_moves_metric`] and [`noise_floor`] will run on.
///
/// A structural floor, not test-author discipline: with three or four seeds a
/// lucky small `delta_se` produces a small threshold, and a real-but-unrepresentative
/// shift passes. Eight is the point below which the standard error of a mean is
/// not worth thresholding against at all. This gates four upcoming feature PRs,
/// so it panics rather than warns.
pub const MIN_SEEDS: usize = 8;

/// The direction a feature claims its knob pushes its metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedShift {
    /// Perturbing the knob (in `direction`) must RAISE the metric.
    Increases,
    /// Perturbing the knob (in `direction`) must LOWER the metric.
    Decreases,
    /// No direction claimed: magnitude only.
    ///
    /// Deliberately explicit rather than the default, so choosing not to state
    /// a direction is a decision somebody typed and a reviewer can see.
    Unstated,
}

/// Which way a perturbation is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Perturb {
    /// Displace upward from the default, clamping at `max`.
    #[default]
    Up,
    /// Displace downward from the default, clamping at `min`.
    Down,
}

/// A metric's measured dispersion at defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseFloor {
    /// Matches the metric was present for.
    pub n: usize,
    /// Mean at defaults.
    pub mean: f64,
    /// Per-match sample standard deviation.
    pub sd: f64,
    /// Standard error of the mean (`sd / sqrt(n)`), the threshold's basis.
    pub standard_error: f64,
}

/// [`knob_moves_metric`]'s options.
#[derive(Debug)]
pub struct KnobMoveOpts<'a> {
    /// The registered tier-1 tunable id under test.
    pub knob: &'static str,
    /// The registered metric id it is claimed to move.
    pub metric: &'static str,
    /// Seeds both configs run on (common random numbers).
    pub seeds: &'a [f64],
    /// Shorter matches for tests; `None` = the real 120 s.
    pub duration: Option<f64>,
    /// Displacement as a fraction of the knob's range; defaults to
    /// [`DEFAULT_PERTURBATION_FRACTION`].
    pub perturbation: Option<f64>,
    /// Displacement direction; defaults to [`Perturb::Up`].
    ///
    /// This is which way the KNOB moves, not which way the metric is expected
    /// to respond — that is [`Self::expect`].
    pub expect: ExpectedShift,
    /// The direction the metric must move in when the knob is perturbed. A
    /// shift that clears the noise floor with the OPPOSITE sign fails the
    /// contract: a backwards-wired knob is a bug, not a pass.
    pub direction: Option<Perturb>,
}

/// One knob-moves-metric verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct KnobMoveOutcome {
    /// The tunable under test.
    pub knob: &'static str,
    /// The metric under test.
    pub metric: &'static str,
    /// The knob's default value.
    pub baseline_value: f64,
    /// The perturbed value actually used (post-clamp).
    pub perturbed_value: f64,
    /// The metric's measured noise floor at defaults, on these seeds.
    pub noise: NoiseFloor,
    /// Mean paired per-seed shift (perturbed - default).
    pub delta: f64,
    /// Standard error of that paired mean.
    pub delta_se: f64,
    /// The value `|delta|` had to clear.
    pub threshold: f64,
    /// Whether the shift cleared the noise floor at all, regardless of sign.
    pub moved: bool,
    /// Whether the shift cleared the noise floor AND matched
    /// [`KnobMoveOpts::expect`]. This is what [`assert_moves`] asserts.
    pub passes: bool,
    /// The direction the caller claimed.
    pub expect: ExpectedShift,
    /// Human-readable verdict, for an assertion message.
    pub report: String,
}

fn mean_sd(values: &[f64]) -> (f64, f64) {
    let n = values.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    let mean = values.iter().sum::<f64>() / n as f64;
    if n == 1 {
        return (mean, 0.0);
    }
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    (mean, var.sqrt())
}

fn metric_series(blob: &str, opts: &KnobMoveOpts<'_>) -> Vec<Option<f64>> {
    let batch = headless::run_batch(&BatchOpts {
        seeds: Some(opts.seeds),
        tuning_blob: Some(blob),
        duration: opts.duration,
        ..Default::default()
    });
    let registry = metric_registry::shipped();
    batch
        .matches
        .iter()
        .map(|r| registry.extract(opts.metric, &r.metrics))
        .collect()
}

/// Measure one metric's dispersion at default knobs, over `seeds`.
///
/// This is the pilot the contract's threshold is built from. It is a real
/// measurement on the caller's own seed set, not a table of assumed variances:
/// a metric's spread depends on match length and seed count, and a threshold
/// derived from someone else's run is exactly the false-pass the issue warned
/// about.
///
/// # Panics
///
/// Panics if `metric` is not registered.
#[must_use]
pub fn noise_floor(metric: &'static str, seeds: &[f64], duration: Option<f64>) -> NoiseFloor {
    assert!(
        metric_registry::shipped().get(metric).is_some(),
        "unregistered metric id: {metric}"
    );
    assert!(
        seeds.len() >= MIN_SEEDS,
        "a noise floor over {} seeds is not a measurement: {MIN_SEEDS} is the minimum",
        seeds.len()
    );
    let opts = KnobMoveOpts {
        knob: "",
        metric,
        seeds,
        duration,
        perturbation: None,
        expect: ExpectedShift::Unstated,
        direction: None,
    };
    let present: Vec<f64> = metric_series("", &opts).into_iter().flatten().collect();
    let (mean, sd) = mean_sd(&present);
    let n = present.len();
    let standard_error = if n > 1 {
        sd / (n as f64).sqrt()
    } else {
        f64::INFINITY
    };
    NoiseFloor {
        n,
        mean,
        sd,
        standard_error,
    }
}

/// Run the knob-moves-metric contract for one registered knob and metric.
///
/// # Panics
///
/// Panics if `opts.knob` is not a registered tier-1 tunable or `opts.metric`
/// is not a registered metric — naming something that does not exist is a
/// programmer error, not a failing contract (AGENTS.md §7).
#[must_use]
pub fn knob_moves_metric(opts: &KnobMoveOpts<'_>) -> KnobMoveOutcome {
    let registry = tunable_registry::shipped();
    let def = registry
        .def(opts.knob)
        .unwrap_or_else(|| panic!("unregistered tunable id: {}", opts.knob));
    assert!(
        metric_registry::shipped().get(opts.metric).is_some(),
        "unregistered metric id: {}",
        opts.metric
    );
    assert!(
        opts.seeds.len() >= MIN_SEEDS,
        "the knob-moves-metric contract needs at least {MIN_SEEDS} seeds, got {}: \
         below that a lucky small standard error passes a shift no one could reproduce",
        opts.seeds.len()
    );

    let fraction = opts.perturbation.unwrap_or(DEFAULT_PERTURBATION_FRACTION);
    let span = (def.max - def.min) * fraction;
    let perturbed_value = match opts.direction.unwrap_or_default() {
        Perturb::Up => (def.default + span).min(def.max),
        Perturb::Down => (def.default - span).max(def.min),
    };

    let baseline = metric_series("", opts);
    let perturbed = metric_series(&format!("{}={}", def.id, format_g6(perturbed_value)), opts);

    let mut diffs = Vec::new();
    let mut base_present = Vec::new();
    for (b, p) in baseline.iter().zip(perturbed.iter()) {
        if let (Some(b), Some(p)) = (b, p) {
            diffs.push(p - b);
            base_present.push(*b);
        }
    }
    let (delta, diff_sd) = mean_sd(&diffs);
    let delta_se = if diffs.len() > 1 {
        diff_sd / (diffs.len() as f64).sqrt()
    } else {
        f64::INFINITY
    };

    let (mean, sd) = mean_sd(&base_present);
    let n = base_present.len();
    let standard_error = if n > 1 {
        sd / (n as f64).sqrt()
    } else {
        f64::INFINITY
    };
    let noise = NoiseFloor {
        n,
        mean,
        sd,
        standard_error,
    };

    let threshold = NOISE_SIGMAS * standard_error.max(delta_se);
    let moved = delta.abs() > threshold;
    let sign_agrees = match opts.expect {
        ExpectedShift::Increases => delta > 0.0,
        ExpectedShift::Decreases => delta < 0.0,
        ExpectedShift::Unstated => true,
    };
    let passes = moved && sign_agrees;
    let verdict = match (moved, sign_agrees) {
        (false, _) => "DECORATION",
        // The dangerous case, and the reason this check exists: a shift that
        // clears the noise floor in the wrong direction looks like success to
        // any magnitude-only test.
        (true, false) => "BACKWARDS",
        (true, true) => "WIRED",
    };
    let claim = match opts.expect {
        ExpectedShift::Increases => " (expected to increase)",
        ExpectedShift::Decreases => " (expected to decrease)",
        ExpectedShift::Unstated => " (no direction claimed)",
    };
    let report = format!(
        "{} {} -> {} moves {}{}: delta {:+.4} (+/-{:.4} se), noise floor {:.4} \
         (sd {:.4}, n {}), threshold {:.4} => {}",
        def.id,
        format_g6(def.default),
        format_g6(perturbed_value),
        opts.metric,
        claim,
        delta,
        delta_se,
        standard_error,
        sd,
        n,
        threshold,
        verdict,
    );

    KnobMoveOutcome {
        knob: def.id,
        metric: opts.metric,
        baseline_value: def.default,
        perturbed_value,
        noise,
        delta,
        delta_se,
        threshold,
        moved,
        passes,
        expect: opts.expect,
        report,
    }
}

/// Assert the contract, panicking with [`KnobMoveOutcome::report`] on failure.
///
/// The one-line form a feature test uses:
/// `knob_contract::assert_moves(&opts);`
///
/// Fails on BOTH ways the contract can be broken: a knob whose metric does not
/// move past the measured noise floor, and a knob whose metric moves the
/// opposite way to [`KnobMoveOpts::expect`]. The second is the one a
/// magnitude-only helper waves through.
///
/// # Panics
///
/// If the knob does not move the metric past its measured noise floor, or
/// moves it against the declared direction.
pub fn assert_moves(opts: &KnobMoveOpts<'_>) -> KnobMoveOutcome {
    let outcome = knob_moves_metric(opts);
    assert!(outcome.passes, "{}", outcome.report);
    outcome
}
