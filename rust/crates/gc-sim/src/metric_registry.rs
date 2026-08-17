//! Balance-metric registration: the single source for what the harness
//! measures, what band each measurement is scored against, and how those
//! desirabilities fold into the fun score.
//!
//! The same declarative pattern as [`crate::tunable_registry`], for
//! measurements instead of knobs. A feature registers a metric with an id, an
//! extraction function over finished-match telemetry, a target band and a
//! direction of goodness; the harness computes desirability and folds it into
//! the geometric-mean fun score automatically, so a new metric changes the
//! score's composition without a harness edit.
//!
//! ## What this replaced
//!
//! Before this module the eight banded measurements existed in **three**
//! places: `crate::metrics`'s private `BAND_*` constants (the scoring copy),
//! `crate::headless`'s `band_for` (the report copy) and
//! `crate::lever_metrics`'s `band_width` + `BANDED_KEYS` (the analysis copy).
//! Both duplicates carried a comment admitting they were duplicates. All three
//! now read this registry, and the two hand-maintained copies are deleted.
//!
//! ## Fold order is load-bearing
//!
//! The fun score is a geometric mean, and floating-point multiplication is not
//! associative, so the order the desirabilities multiply in decides the
//! score's last bits. The fold order is `gc_data::tunables::METRICS`'s
//! authored order — deliberately NOT sorted, unlike the tunable registry's
//! canonical id order, because nothing about a metric crosses the wire and
//! byte-identical scores across a refactor matter more here than
//! registration-order independence. `tests/metric_registry.rs` pins it.
//!
//! ## Extraction runs outside the simulation loop
//!
//! Every extractor takes a finished [`MatchMetrics`], which
//! [`crate::metrics::finish`] folds from telemetry the observer collected.
//! No extractor reads live match state, so metric extraction can never
//! participate in a resimulation.

use crate::metrics::MatchMetrics;
use gc_data::tunables::{MetricDef, MetricDirection};
use indexmap::IndexMap;
use std::sync::LazyLock;

/// One registered balance metric: its authored definition plus the extraction
/// function that reads it off a finished match.
#[derive(Clone, Copy)]
pub struct Metric {
    /// The authored id, band and direction.
    pub def: &'static MetricDef,
    /// Reads this metric off a finished match. `None` when the metric's
    /// denominator never happened (e.g. `save_rate` with zero on-target
    /// shots); the fold skips it rather than defaulting it to zero.
    pub extract: fn(&MatchMetrics) -> Option<f64>,
}

impl core::fmt::Debug for Metric {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Metric").field("def", &self.def).finish()
    }
}

/// Assembles a [`MetricRegistry`] from declarative registrations.
#[derive(Debug, Default)]
pub struct MetricRegistryBuilder {
    metrics: Vec<Metric>,
    features: Vec<(&'static str, &'static str)>,
}

impl MetricRegistryBuilder {
    /// A builder with nothing registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one feature's metrics, in fold order.
    ///
    /// # Panics
    ///
    /// Panics on a duplicate id, for the same reason
    /// [`crate::tunable_registry::RegistryBuilder::register_tunables`] does:
    /// two features claiming one measurement is a programmer error, and
    /// letting one silently win makes the fun score mean something nobody
    /// declared.
    pub fn register_metrics(&mut self, feature: &'static str, metrics: &[Metric]) {
        for m in metrics {
            if let Some((owner, _)) = self.features.iter().find(|(_, id)| *id == m.def.id) {
                panic!(
                    "duplicate metric id {:?}: registered by {owner:?} and again by {feature:?}",
                    m.def.id
                );
            }
            self.features.push((feature, m.def.id));
            self.metrics.push(*m);
        }
    }

    /// Assemble the registry.
    #[must_use]
    pub fn build(self) -> MetricRegistry {
        MetricRegistry {
            metrics: self.metrics,
        }
    }
}

/// Every registered balance metric, in fold order.
#[derive(Clone, Debug)]
pub struct MetricRegistry {
    metrics: Vec<Metric>,
}

/// The shipped metric registry. Read-only, built from `static` content — see
/// [`crate::tunable_registry`]'s module doc on globals.
#[must_use]
pub fn shipped() -> &'static MetricRegistry {
    static SHIPPED: LazyLock<MetricRegistry> = LazyLock::new(assemble);
    &SHIPPED
}

fn def(id: &str) -> &'static MetricDef {
    gc_data::tunables::METRICS
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("unregistered metric id: {id}"))
}

/// Assemble a fresh registry from this build's authored metric content.
///
/// This is where `gc-sim` pairs each authored band with the extractor that
/// reads it. A feature that adds a measurement adds its `MetricDef` to
/// `gc_data::tunables::METRICS` and its extractor here — two edits, both next
/// to the thing being measured, and no harness edit at all.
#[must_use]
pub fn assemble() -> MetricRegistry {
    let mut b = MetricRegistryBuilder::new();
    b.register_metrics(
        "gc-sim",
        &[
            Metric {
                def: def("goals_total"),
                extract: |m| Some(m.goals_total as f64),
            },
            Metric {
                def: def("shots_per_goal"),
                extract: |m| m.shots_per_goal,
            },
            Metric {
                def: def("save_rate"),
                extract: |m| m.save_rate,
            },
            Metric {
                def: def("pass_completion"),
                extract: |m| m.pass_completion,
            },
            Metric {
                def: def("turnovers_per_min"),
                extract: |m| Some(m.turnovers_per_min),
            },
            Metric {
                def: def("possession_balance"),
                extract: |m| m.possession_balance,
            },
            Metric {
                def: def("longest_drought_s"),
                extract: |m| Some(m.longest_drought_s),
            },
            Metric {
                def: def("decided_late"),
                extract: |m| Some(m.decided_late),
            },
            // Appended last, matching `gc_data::tunables::METRICS`'s authored
            // order -- `metric_registry_folds_in_the_authored_order` compares
            // the two lists element by element, so this is not a free choice.
            Metric {
                def: def("time_to_reverse"),
                // `None` when the match armed no reversal at all, which the
                // fun score treats as absence rather than as a perfect zero.
                extract: |m| m.time_to_reverse,
            },
            // #491's two, in `gc_data::tunables::METRICS`'s authored order.
            // Both are `None` when the match contained no release of the
            // relevant kind, which the fold skips rather than scoring as a
            // perfect zero.
            //
            // Registered into the FOLD as an interim state, superseded by
            // #528's probation decision -- see their `MetricDef`s in
            // `gc_data::tunables` for the measured inflation and why this is
            // a known condition rather than an oversight. #531 phase 5
            // reviewed and kept both, on a per-knob dependency argument
            // rather than #527's original blanket one -- same interim fold,
            // narrower justification; see `gc_data::tunables` and
            // `docs/design/fun_metrics.md`'s phase-5 section.
            Metric {
                def: def("pass_aim_error"),
                extract: |m| m.pass_aim_error,
            },
            Metric {
                def: def("pass_lead_time"),
                extract: |m| m.pass_lead_time,
            },
            // #489, appended last matching `gc_data::tunables::METRICS`'s
            // authored order. `None` when the match attempted no standing-poke
            // tackle at all (the fold skips it rather than scoring a perfect
            // or catastrophic value nobody observed).
            Metric {
                def: def("whiff_rate"),
                extract: |m| m.whiff_rate,
            },
        ],
    );
    b.build()
}

impl MetricRegistry {
    /// Every registered metric, in fold order.
    #[must_use]
    pub fn metrics(&self) -> &[Metric] {
        &self.metrics
    }

    /// Every registered metric id, in fold order.
    #[must_use]
    pub fn ids(&self) -> Vec<&'static str> {
        self.metrics.iter().map(|m| m.def.id).collect()
    }

    /// One metric by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Metric> {
        self.metrics.iter().find(|m| m.def.id == id)
    }

    /// One metric's trapezoid band, `None` for an unregistered id.
    #[must_use]
    pub fn band(&self, id: &str) -> Option<[f64; 4]> {
        self.get(id).map(|m| m.def.band)
    }

    /// One metric's good-band width (`good_hi - good_lo`), the unit
    /// [`crate::lever_metrics`] normalizes shifts by.
    #[must_use]
    pub fn band_width(&self, id: &str) -> Option<f64> {
        self.band(id).map(|b| b[2] - b[1])
    }

    /// One metric's value on a finished match, `None` when its denominator
    /// never happened or the id is not registered.
    #[must_use]
    pub fn extract(&self, id: &str, m: &MatchMetrics) -> Option<f64> {
        self.get(id).and_then(|metric| (metric.extract)(m))
    }

    /// Geometric mean of every registered metric present in `m`.
    ///
    /// A collapsed dimension (desirability 0) zeroes the whole score by
    /// design; missing metrics (`None` denominators) are skipped, not
    /// defaulted. Returns the score (`0..1`) and each contributing metric's
    /// desirability, by id, in fold order.
    #[must_use]
    pub fn fun_score(&self, m: &MatchMetrics) -> (f64, IndexMap<&'static str, f64>) {
        let mut product = 1.0_f64;
        let mut n = 0_i32;
        let mut per = IndexMap::new();
        for metric in &self.metrics {
            if let Some(v) = (metric.extract)(m) {
                let d = match metric.def.direction {
                    MetricDirection::Banded => desirability(v, metric.def.band),
                };
                per.insert(metric.def.id, d);
                product *= d;
                n += 1;
            }
        }
        if n == 0 {
            return (0.0, per);
        }
        (product.powf(1.0 / f64::from(n)), per)
    }
}

/// Desirability of `v` under a trapezoid band `{zero_lo, good_lo, good_hi,
/// zero_hi}`. Returns `0..1`.
#[must_use]
pub fn desirability(v: f64, band: [f64; 4]) -> f64 {
    let [zl, gl, gh, zh] = band;
    if v <= zl || v >= zh {
        return 0.0;
    }
    if v < gl {
        return (v - zl) / (gl - zl);
    }
    if v > gh {
        return (zh - v) / (zh - gh);
    }
    1.0
}
