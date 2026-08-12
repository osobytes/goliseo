//! The declarative tunable registry: three tiers, separately serialized, with
//! one config hash over everything a peer must agree on.
//!
//! A feature registers its knobs where the feature lives, not in a central
//! list. `gc_data::tunables` authors the tier-1 scalars and the tier-3 band
//! sets; `gc_render::presentation_tunables` authors tier 2; a test or a future
//! feature crate registers more through [`RegistryBuilder::register_tunables`]
//! / [`RegistryBuilder::register_band_set`]. This module is the container and
//! the lookups — no game state, no I/O.
//!
//! ## The three tiers
//!
//! See `gc_data::tunables`'s module doc for what each tier means. What this
//! module adds is the mechanism:
//!
//! - **Separate serialization.** [`Registry::serialize_tier`] emits one blob
//!   per tier, each prefixed with `#tier=<tag>`. [`Registry::deserialize_tier`]
//!   refuses a blob whose header names a different tier, so a presentation
//!   blob can never be pasted into the sim tier by accident.
//! - **Structural tier-2 isolation.** This module can hold tier-2 entries, but
//!   nothing in `gc-sim` ever registers one: tier-2 values are authored in
//!   `gc-render`, which depends on `gc-sim`, so a `gc-sim` module that tried
//!   to read them would be a dependency cycle the compiler rejects. The
//!   promise is a fact about the build graph, not a comment.
//! - **Band-set atomicity.** A tier-3 edge has no setter at all:
//!   [`Registry::override_band_edge`] exists precisely so that attempting one
//!   is an explicit, testable `Err`. Whole-set substitution
//!   ([`Registry::substitute_band_set`]) is the only way in, and it validates
//!   that the replacement declares the same edge ids under a new version.
//! - **Config hash.** [`Registry::config_hash`] folds every tier-1 value and
//!   every tier-3 edge — never tier 2 — into one FNV-1a digest, over ids
//!   **sorted by id** so two peers that registered features in different
//!   orders still agree.
//!
//! ## No global mutable state
//!
//! AGENTS.md §3 forbids stray global mutable state. [`shipped`] is a
//! `LazyLock` over the *authored* registry — built once from `static` content
//! tables, never mutated, the same category as `gc_data`'s own `static`
//! tables. Anything that varies (a sweep override, a substituted band set) is
//! an owned [`Registry`] value the caller threads, exactly as
//! [`crate::tuning::Tuning`] already is.

use gc_core::fnv1a64;
use gc_data::tunables::{BandSet, Tier, TunableDef};
use indexmap::IndexMap;
use std::sync::LazyLock;

/// A tunable registry: the assembled tier-1 definitions, the tier-3 band sets,
/// and the current tier-1 values.
#[derive(Clone, Debug, PartialEq)]
pub struct Registry {
    defs: Vec<&'static TunableDef>,
    values: IndexMap<&'static str, f64>,
    band_sets: Vec<BandSet>,
}

/// Assembles a [`Registry`] from declarative registrations.
///
/// Registration order does not affect the config hash (ids are sorted before
/// hashing) but it does affect display order, which is why
/// [`RegistryBuilder::build`] keeps registrations in the order they arrived.
#[derive(Debug, Default)]
pub struct RegistryBuilder {
    defs: Vec<&'static TunableDef>,
    band_sets: Vec<BandSet>,
    features: Vec<(&'static str, &'static str)>,
}

impl RegistryBuilder {
    /// A builder with nothing registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one feature's tunables.
    ///
    /// # Panics
    ///
    /// Panics on a duplicate id — a startup assertion, per the issue and
    /// AGENTS.md §7: two features claiming one id is a programmer error, not a
    /// recoverable condition, and silently letting one win is how a knob stops
    /// meaning what its owner thinks it means.
    pub fn register_tunables(&mut self, feature: &'static str, defs: &'static [TunableDef]) {
        for def in defs {
            if let Some((owner, _)) = self.features.iter().find(|(_, id)| *id == def.id) {
                panic!(
                    "duplicate tunable id {:?}: registered by {owner:?} and again by {feature:?}",
                    def.id
                );
            }
            self.features.push((feature, def.id));
            self.defs.push(def);
        }
    }

    /// Register one feature's tier-3 band set.
    ///
    /// # Panics
    ///
    /// Panics on a duplicate set id, or on a set id that collides with a
    /// tunable id, for the same reason [`Self::register_tunables`] does.
    pub fn register_band_set(&mut self, feature: &'static str, set: BandSet) {
        if let Some((owner, _)) = self.features.iter().find(|(_, id)| *id == set.id) {
            panic!(
                "duplicate band-set id {:?}: registered by {owner:?} and again by {feature:?}",
                set.id
            );
        }
        self.features.push((feature, set.id));
        self.band_sets.push(set);
    }

    /// Assemble the registry, every tier-1 value at its default.
    #[must_use]
    pub fn build(self) -> Registry {
        let values = self.defs.iter().map(|d| (d.id, d.default)).collect();
        Registry {
            defs: self.defs,
            values,
            band_sets: self.band_sets,
        }
    }
}

/// The shipped registry: every tunable this build authors, at its default.
///
/// Read-only and built from `static` content (see the module doc on globals).
/// Callers that need to vary a value clone it — `Registry` is `Clone`.
#[must_use]
pub fn shipped() -> &'static Registry {
    static SHIPPED: LazyLock<Registry> = LazyLock::new(assemble);
    &SHIPPED
}

/// Assemble a fresh registry from this build's authored content.
///
/// This is the one place `gc-sim`'s own registrations are declared. A feature
/// crate that cannot be reached from here (`gc-render`, tier 2) assembles its
/// own registry the same way.
#[must_use]
pub fn assemble() -> Registry {
    let mut b = RegistryBuilder::new();
    b.register_tunables("gc-sim", gc_data::tunables::SIM_TUNABLES);
    for set in gc_data::tunables::BAND_SETS {
        b.register_band_set("gc-sim", *set);
    }
    b.build()
}

impl Registry {
    /// Every registered tunable definition, in registration (display) order.
    #[must_use]
    pub fn defs(&self) -> &[&'static TunableDef] {
        &self.defs
    }

    /// Every registered tunable in one tier, in registration order.
    #[must_use]
    pub fn tier(&self, tier: Tier) -> Vec<&'static TunableDef> {
        self.defs
            .iter()
            .copied()
            .filter(|d| d.tier == tier)
            .collect()
    }

    /// The tier-1 ids a sweep may vary, **sorted by id**.
    ///
    /// Sorted rather than authored order so that two builds which registered
    /// features in a different sequence still enumerate the same list in the
    /// same order — the property the config hash depends on, applied to the
    /// sweep so a sweep's own iteration is reproducible too.
    #[must_use]
    pub fn sweepable_ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self
            .defs
            .iter()
            .filter(|d| d.tier == Tier::Sim)
            .map(|d| d.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// One definition by id.
    #[must_use]
    pub fn def(&self, id: &str) -> Option<&'static TunableDef> {
        self.defs.iter().copied().find(|d| d.id == id)
    }

    /// The current value of a tier-1 or tier-2 tunable.
    ///
    /// # Panics
    ///
    /// Panics on an unknown id: every caller reads an id it authored, so an
    /// unknown id is a programmer error (AGENTS.md §7).
    #[must_use]
    pub fn value(&self, id: &str) -> f64 {
        *self
            .values
            .get(id)
            .unwrap_or_else(|| panic!("unknown tunable id: {id}"))
    }

    /// Set a tunable, clamped to its declared range. Unknown ids are ignored,
    /// matching [`crate::tuning::Tuning::set`]'s long-standing contract for a
    /// blob that names a knob this build no longer ships.
    pub fn set(&mut self, id: &str, v: f64) {
        if let Some(d) = self.def(id) {
            self.values.insert(d.id, v.max(d.min).min(d.max));
        }
    }

    /// Reset one tunable, or every tunable when `id` is `None`.
    pub fn reset(&mut self, id: Option<&str>) {
        match id {
            Some(id) => {
                if let Some(d) = self.def(id) {
                    self.values.insert(d.id, d.default);
                }
            }
            None => {
                for d in &self.defs {
                    self.values.insert(d.id, d.default);
                }
            }
        }
    }

    /// Every registered band set, in registration order.
    #[must_use]
    pub fn band_sets(&self) -> &[BandSet] {
        &self.band_sets
    }

    /// One band set by id.
    #[must_use]
    pub fn band_set(&self, id: &str) -> Option<&BandSet> {
        self.band_sets.iter().find(|s| s.id == id)
    }

    /// One band edge's value.
    ///
    /// # Panics
    ///
    /// Panics on an unknown set or edge id, for the same reason [`Self::value`]
    /// does.
    #[must_use]
    pub fn band_edge(&self, set_id: &str, edge_id: &str) -> f64 {
        self.band_set(set_id)
            .unwrap_or_else(|| panic!("unknown band set: {set_id}"))
            .edge(edge_id)
            .unwrap_or_else(|| panic!("unknown band edge: {set_id}.{edge_id}"))
    }

    /// Always fails. A tier-3 edge means nothing on its own — it is defined
    /// entirely by its neighbours — so there is no single-edge write path, and
    /// this exists so that trying is a clear error rather than a missing
    /// method somebody works around.
    ///
    /// # Errors
    ///
    /// Always, naming [`Self::substitute_band_set`] as the supported route.
    pub fn override_band_edge(
        &mut self,
        set_id: &str,
        edge_id: &str,
        _value: f64,
    ) -> Result<(), String> {
        Err(format!(
            "tier-3 band edges are versioned as one unit: {set_id}.{edge_id} cannot be \
             overridden alone. Substitute a whole versioned set with substitute_band_set."
        ))
    }

    /// Substitute a whole versioned band set.
    ///
    /// # Errors
    ///
    /// - the set id is not registered;
    /// - the replacement reuses the current version (a substitution that keeps
    ///   the version is indistinguishable from the original in evidence);
    /// - the replacement's edge ids are not exactly the registered set's, in
    ///   the same order (a set that classifies a different situation is a new
    ///   set, not a new version of this one).
    pub fn substitute_band_set(&mut self, replacement: BandSet) -> Result<(), String> {
        let Some(index) = self.band_sets.iter().position(|s| s.id == replacement.id) else {
            return Err(format!("unknown band set: {}", replacement.id));
        };
        let current = &self.band_sets[index];
        if current.version == replacement.version {
            return Err(format!(
                "band set {} substitution must change the version (still {:?})",
                current.id, current.version
            ));
        }
        let current_ids: Vec<&str> = current.edges.iter().map(|e| e.id).collect();
        let new_ids: Vec<&str> = replacement.edges.iter().map(|e| e.id).collect();
        if current_ids != new_ids {
            return Err(format!(
                "band set {} substitution must declare the same edges in the same order: \
                 expected {current_ids:?}, got {new_ids:?}",
                current.id
            ));
        }
        self.band_sets[index] = replacement;
        Ok(())
    }

    /// One `ID=value` line per NON-default tier-1 or tier-2 tunable, under a
    /// `#tier=<tag>` header.
    ///
    /// Tier 3 serializes its whole set identity instead of per-edge values —
    /// an edge value alone is not a thing anyone may write back.
    #[must_use]
    pub fn serialize_tier(&self, tier: Tier) -> String {
        let mut lines = vec![format!("#tier={}", tier.tag())];
        if tier == Tier::Bands {
            for set in &self.band_sets {
                lines.push(format!("{}@{}", set.id, set.version));
            }
        } else {
            for d in self.defs.iter().filter(|d| d.tier == tier) {
                let v = self.value(d.id);
                if v != d.default {
                    lines.push(format!("{}={}", d.id, format_g6(v)));
                }
            }
        }
        lines.join("\n")
    }

    /// Apply a tier blob emitted by [`Self::serialize_tier`], on top of that
    /// tier's defaults.
    ///
    /// # Errors
    ///
    /// If the blob's `#tier=` header is missing or names a different tier.
    /// That check is the point: without it a presentation blob and a sim blob
    /// are the same bytes with different keys, and pasting one into the other
    /// is a silent no-op rather than a mistake anyone notices.
    pub fn deserialize_tier(&mut self, tier: Tier, blob: &str) -> Result<(), String> {
        let mut lines = blob.split(['\r', '\n']).filter(|l| !l.is_empty());
        let header = lines.next().unwrap_or_default();
        let declared = header
            .strip_prefix("#tier=")
            .ok_or_else(|| format!("tunable blob has no #tier= header: {header:?}"))?;
        if declared != tier.tag() {
            return Err(format!(
                "tunable blob is tier {declared:?}, applied to tier {:?}",
                tier.tag()
            ));
        }
        for d in self.defs.iter().filter(|d| d.tier == tier) {
            self.values.insert(d.id, d.default);
        }
        if tier == Tier::Bands {
            // Band sets are content, not values: a blob records which version
            // was in force so evidence can name it, and substituting it back
            // is `substitute_band_set`'s job (the replacement's edge values
            // have to come from somewhere, and that somewhere is authored
            // content, never a blob).
            return Ok(());
        }
        for line in lines {
            if let Some((id, value)) = parse_line(line)
                && let Ok(v) = value.parse::<f64>()
                && self.def(id).is_some_and(|d| d.tier == tier)
            {
                self.set(id, v);
            }
        }
        Ok(())
    }

    /// The canonical bytes [`Self::config_hash`] digests. Exposed so a
    /// handshake mismatch can be diffed rather than only reported.
    ///
    /// Covers every tier-1 value and every tier-3 edge, **sorted by id**, and
    /// deliberately no tier-2 value: tier 2 cannot change what the simulation
    /// computes, so folding it in would fail peers over a smoothing rate.
    #[must_use]
    pub fn config_canonical(&self) -> String {
        let mut rows: Vec<(String, f64)> = self
            .defs
            .iter()
            .filter(|d| d.tier == Tier::Sim)
            .map(|d| (d.id.to_string(), self.value(d.id)))
            .collect();
        for set in &self.band_sets {
            for edge in set.edges {
                rows.push((format!("{}.{}", set.id, edge.id), edge.value));
            }
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut parts = String::from("GCTUNE;1;");
        for (id, value) in rows {
            parts.push_str(&id);
            parts.push('=');
            parts.push_str(&format_g17(value));
            parts.push(';');
        }
        parts
    }

    /// One digest over everything two peers must agree on before kickoff.
    ///
    /// Any tier-1 value or tier-3 edge that differs produces a different
    /// hash, so a divergence fails fast at handshake instead of desyncing at
    /// the first divergent read.
    #[must_use]
    pub fn config_hash(&self) -> String {
        fnv1a64::hash(self.config_canonical().as_bytes())
    }
}

impl Default for Registry {
    fn default() -> Self {
        assemble()
    }
}

/// Parse one `ID=value` line: the id is alphanumeric/underscore/dot, the value
/// is restricted to digits, `-`, `.`, `e`, and `E`. Returns `None` if the line
/// does not match.
fn parse_line(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let (id, rest) = (&line[..eq], &line[eq + 1..]);
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == 'e' || c == 'E')
    {
        return None;
    }
    Some((id, rest))
}

/// Format `value` the way C's `%.6g` would, over the magnitude range a tunable
/// lives in. Shared with [`crate::tuning`], which re-exports it rather than
/// keeping the third copy this repo used to carry.
#[must_use]
pub fn format_g6(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 17) as usize;
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Full-precision formatting for the config hash's canonical bytes.
///
/// `format_g6` is a *display* format and rounds; two knob values that differ
/// below its precision would hash identically, which is exactly the silent
/// divergence the hash exists to catch. Rust's `{:?}` for `f64` prints the
/// shortest string that round-trips, so it is injective over `f64` and
/// platform-independent.
fn format_g17(value: f64) -> String {
    format!("{value:?}")
}
