//! Runtime-tunable gameplay knobs, for the in-match tuning panel (F1). The
//! sim reads live values through a [`Tuning`] instance; defaults are the
//! shipped balance, so a fresh match plays identically to the constants they
//! replaced. Pure module: no I/O — (de)serialization is string-based and the
//! game layer decides where bytes go.
//!
//! AGENTS.md §3 forbids stray global mutable state,
//! so the registry is an explicit value, [`Tuning`], that callers thread
//! through instead of a shared mutable global — every other `sim` module
//! reads live values through the instance it's given.
//!
//! ## This module is a view over the tunable registry
//!
//! Knob metadata is no longer authored here. [`crate::tunable_registry`] holds
//! the three-tier registry; `gc_data::tunables::SIM_TUNABLES` authors the
//! tier-1 entries; [`KNOBS`] is the panel-shaped projection of the registry's
//! tier-1 set, in registration (display) order. Nothing enumerates knobs from
//! a hand-maintained list any more — the sweep, the panel and the config hash
//! all read the same registry.
//!
//! ## Two serializations, deliberately
//!
//! [`Tuning::serialize`] keeps the untagged `KEY=value` format the F1 panel,
//! `gc_data::tuning_presets` and the OMP-1 determinism fixture have always
//! used, byte for byte. It is a tier-1 blob by construction — a `Tuning` only
//! ever holds tier-1 values. The tier-tagged format that keeps a presentation
//! blob from being pasted into the sim tier is
//! [`crate::tunable_registry::Registry::serialize_tier`]; both exist because
//! the untagged one is a shipped contract with a UI and a fixture, not because
//! tier 1 has two formats worth having.

use crate::tunable_registry::{self, Registry, format_g6};
use gc_data::tunables::Tier;
use std::sync::LazyLock;

/// A single tunable gameplay knob, as the tuning panel sees it.
///
/// The panel-facing projection of a `gc_data::tunables::TunableDef`: the
/// fields `packages/ui/src/tuning_panel.ts`'s `Knob` declares, and no others.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knob {
    /// Registry key, also the serialization key.
    pub key: &'static str,
    /// Shown in the tuning panel.
    pub label: &'static str,
    /// Panel tab this knob's category groups into.
    pub cat: &'static str,
    /// Value a fresh match plays with.
    pub default: f64,
    /// Minimum allowed value.
    pub min: f64,
    /// Maximum allowed value.
    pub max: f64,
    /// Nudge step size.
    pub step: f64,
}

/// Every registered tier-1 knob, in registry (display) order.
///
/// Derived from [`crate::tunable_registry::shipped`], not authored here: a
/// feature that registers a knob gets a panel entry and a sweep entry with no
/// edit to this file. Read-only, built once from `static` content — see
/// `tunable_registry`'s module doc on globals.
pub static KNOBS: LazyLock<Vec<Knob>> = LazyLock::new(|| {
    tunable_registry::shipped()
        .tier(Tier::Sim)
        .into_iter()
        .map(|d| Knob {
            key: d.id,
            label: d.label,
            cat: d.cat,
            default: d.default,
            min: d.min,
            max: d.max,
            step: d.step,
        })
        .collect()
});

/// A live registry of tuning knob values.
///
/// An owned value rather than a module-level singleton (see the module doc).
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    reg: Registry,
}

fn knob_by_key(key: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.key == key)
}

impl Default for Tuning {
    fn default() -> Self {
        Self::new()
    }
}

impl Tuning {
    /// A fresh registry at every knob's default value.
    #[must_use]
    pub fn new() -> Self {
        Tuning {
            reg: tunable_registry::shipped().clone(),
        }
    }

    /// The underlying tunable registry handle, for callers that need tiers,
    /// band sets or the config hash rather than panel-shaped knobs.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.reg
    }

    /// The current value of a knob. Panics on an unknown key: every caller
    /// reads a key it authored, so an unknown key is a programmer error.
    #[must_use]
    pub fn value(&self, key: &str) -> f64 {
        self.reg.value(key)
    }

    /// Distinct categories, in registry order.
    #[must_use]
    pub fn categories(&self) -> Vec<&'static str> {
        let mut seen: Vec<&'static str> = Vec::new();
        for k in KNOBS.iter() {
            if !seen.contains(&k.cat) {
                seen.push(k.cat);
            }
        }
        seen
    }

    /// Every knob in one category, in registry order.
    #[must_use]
    pub fn in_category(&self, cat: &str) -> Vec<&'static Knob> {
        KNOBS.iter().filter(|k| k.cat == cat).collect()
    }

    /// Set a knob, clamped to its range. Unknown keys are ignored.
    pub fn set(&mut self, key: &str, v: f64) {
        self.reg.set(key, v);
    }

    /// Nudge a knob by `dirs` steps (negative = down).
    pub fn nudge(&mut self, key: &str, dirs: f64) {
        if let Some(k) = knob_by_key(key) {
            let current = self.value(key);
            self.set(key, current + k.step * dirs);
        }
    }

    /// Reset one knob, or everything when `key` is `None`.
    pub fn reset(&mut self, key: Option<&str>) {
        self.reg.reset(key);
    }

    /// Whether a knob currently sits at its default value.
    #[must_use]
    pub fn is_default(&self, key: &str) -> bool {
        match knob_by_key(key) {
            Some(k) => self.value(key) == k.default,
            None => false,
        }
    }

    /// One `KEY=value` line per NON-default knob (a fresh registry means an
    /// empty string).
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut lines = Vec::new();
        for k in KNOBS.iter() {
            let v = self.value(k.key);
            if v != k.default {
                lines.push(format!("{}={}", k.key, format_g6(v)));
            }
        }
        lines.join("\n")
    }

    /// Apply a serialized blob on top of defaults. Malformed lines are
    /// skipped.
    pub fn deserialize(&mut self, blob: &str) {
        self.reset(None);
        for line in blob.split(['\r', '\n']) {
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = parse_knob_line(line)
                && let Ok(v) = value.parse::<f64>()
            {
                self.set(key, v);
            }
        }
    }
}

/// Parse one `KEY=value` line: the key is alphanumeric/underscore, the value
/// is restricted to digits, `-`, `.`, `e`, and `E`. Returns `None` if the
/// line does not match.
fn parse_knob_line(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let (key, rest) = (&line[..eq], &line[eq + 1..]);
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == 'e' || c == 'E')
    {
        return None;
    }
    Some((key, rest))
}
