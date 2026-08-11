//! `wasm-bindgen` control surface over `gc_sim::tuning` (the live gameplay
//! knob registry) and `gc_data::tuning_presets` (the authored preset list).
//!
//! `packages/ui/src/tuning_panel.ts`'s own header comment names exactly this
//! gap: `@gc/wasm`'s `SimHost` binds session lifecycle and the determinism
//! check, but "no method surfaces `gc_sim::tuning` or
//! `gc_data::tuning_presets`" — so the panel's `TuningSource`/
//! `TuningPreset[]` stay injected rather than backed by a real bridge. This
//! module is that bridge: [`TuningRegistry`] mirrors `TuningSource` method
//! for method (`categories`/`inCategory`/`valueOf`/`nudge`/`reset`/
//! `isDefault`/`serialize`/`deserialize`), and [`tuning_presets`] returns
//! `gc_data::tuning_presets::ALL` as `TuningPreset[]`.
//!
//! Both registries are small, closed-set, string/number-keyed data — no JSON
//! needed. [`WasmKnob`]/[`WasmTuningPreset`] are plain `wasm-bindgen`
//! value types (`getter_with_clone`), returned as ordinary `Vec<T>` (a JS
//! array of class instances — `Vec<T>` for a `#[wasm_bindgen]` type `T` is a
//! first-class `wasm-bindgen` return shape, the same mechanism
//! [`crate::match_driver_bridge::MatchDriverBridge::snapshot_lookup`] and
//! [`crate::rollback_events_bridge::RollbackEventsTimeline::apply`] use for
//! [`crate::rollback_events_bridge::WasmMatchSnapshot`]).
//!
//! `TuningRegistry` is a plain, freestanding registry — unlike
//! [`crate::session::Session`], no live match reads through it (this wave
//! does not wire a `TuningRegistry` into `Session`/`Entry`'s own `Tuning`;
//! that remains a separate follow-up, noted in this crate's report).

use gc_data::tuning_presets;
use gc_sim::tuning::{self, Tuning};
use wasm_bindgen::prelude::*;

/// One tuning knob's registry metadata and bounds — `packages/ui/src/tuning_panel.ts`'s `Knob`.
#[wasm_bindgen(getter_with_clone)]
#[derive(Clone)]
pub struct WasmKnob {
    /// Registry key, also the serialization key.
    pub key: String,
    /// Shown in the tuning panel.
    pub label: String,
    /// Panel tab this knob's category groups into.
    pub cat: String,
    /// Value a fresh match plays with.
    pub default: f64,
    /// Minimum allowed value.
    pub min: f64,
    /// Maximum allowed value.
    pub max: f64,
    /// Nudge step size.
    pub step: f64,
}

fn to_wasm_knob(knob: &tuning::Knob) -> WasmKnob {
    WasmKnob {
        key: knob.key.to_string(),
        label: knob.label.to_string(),
        cat: knob.cat.to_string(),
        default: knob.default,
        min: knob.min,
        max: knob.max,
        step: knob.step,
    }
}

/// A named blob of non-default overrides — `packages/ui/src/tuning_panel.ts`'s `TuningPreset`.
#[wasm_bindgen(getter_with_clone)]
#[derive(Clone)]
pub struct WasmTuningPreset {
    /// Persistent identity.
    pub id: String,
    /// Shown in the panel status line.
    pub name: String,
    /// Serialized tuning overrides, in `gc_sim::tuning` format.
    pub blob: String,
}

/// Every authored tuning preset (`gc_data::tuning_presets::ALL`), in panel
/// cycle order.
#[wasm_bindgen(js_name = tuningPresets)]
#[must_use]
pub fn tuning_presets() -> Vec<WasmTuningPreset> {
    tuning_presets::ALL
        .iter()
        .map(|preset| WasmTuningPreset {
            id: preset.id.to_string(),
            name: preset.name.to_string(),
            blob: preset.blob.to_string(),
        })
        .collect()
}

/// A live registry of tuning knob values (`gc_sim::tuning::Tuning`) —
/// `packages/ui/src/tuning_panel.ts`'s `TuningSource`.
#[wasm_bindgen]
pub struct TuningRegistry {
    inner: Tuning,
}

#[wasm_bindgen]
impl TuningRegistry {
    /// A fresh registry at every knob's default value.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> TuningRegistry {
        TuningRegistry {
            inner: Tuning::new(),
        }
    }

    /// Distinct categories, in registry order.
    #[must_use]
    pub fn categories(&self) -> Vec<String> {
        self.inner
            .categories()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Every knob in one category, in registry order.
    #[wasm_bindgen(js_name = inCategory)]
    #[must_use]
    pub fn in_category(&self, cat: &str) -> Vec<WasmKnob> {
        self.inner
            .in_category(cat)
            .into_iter()
            .map(to_wasm_knob)
            .collect()
    }

    /// The current value of a knob.
    ///
    /// # Panics
    ///
    /// Panics on an unknown key — see [`Tuning::value`]'s own doc: every
    /// caller reads a key it authored, so an unknown key is a programmer
    /// error, not a recoverable one.
    #[wasm_bindgen(js_name = valueOf)]
    #[must_use]
    pub fn value_of(&self, key: &str) -> f64 {
        self.inner.value(key)
    }

    /// Nudge a knob by `steps` (negative = down). Unknown keys are ignored.
    pub fn nudge(&mut self, key: &str, steps: f64) {
        self.inner.nudge(key, steps);
    }

    /// Reset one knob, or everything when `key` is omitted. Unknown keys are
    /// ignored.
    pub fn reset(&mut self, key: Option<String>) {
        self.inner.reset(key.as_deref());
    }

    /// Whether a knob currently sits at its default value.
    #[wasm_bindgen(js_name = isDefault)]
    #[must_use]
    pub fn is_default(&self, key: &str) -> bool {
        self.inner.is_default(key)
    }

    /// One `KEY=value` line per non-default knob.
    #[must_use]
    pub fn serialize(&self) -> String {
        self.inner.serialize()
    }

    /// Apply a serialized blob on top of defaults. Malformed lines are
    /// skipped.
    pub fn deserialize(&mut self, blob: &str) {
        self.inner.deserialize(blob);
    }
}

impl Default for TuningRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_in_category_mirror_the_registry() {
        let registry = TuningRegistry::new();
        let cats = registry.categories();
        assert!(!cats.is_empty());
        let first = cats.first().expect("at least one category");
        let knobs = registry.in_category(first);
        assert!(!knobs.is_empty());
        for knob in &knobs {
            assert_eq!(knob.cat.as_str(), first.as_str());
        }
    }

    #[test]
    fn nudge_reset_and_is_default_round_trip() {
        let mut registry = TuningRegistry::new();
        let key = registry
            .in_category(&registry.categories()[0])
            .first()
            .expect("at least one knob")
            .key
            .clone();
        assert!(registry.is_default(&key));
        registry.nudge(&key, 1.0);
        assert!(!registry.is_default(&key));
        registry.reset(Some(key.clone()));
        assert!(registry.is_default(&key));
    }

    #[test]
    fn serialize_then_deserialize_round_trips_a_non_default_value() {
        let mut registry = TuningRegistry::new();
        let key = registry
            .in_category(&registry.categories()[0])
            .first()
            .expect("at least one knob")
            .key
            .clone();
        registry.nudge(&key, 2.0);
        let value_before = registry.value_of(&key);
        let blob = registry.serialize();
        assert!(blob.contains(&key));

        let mut other = TuningRegistry::new();
        other.deserialize(&blob);
        assert_eq!(other.value_of(&key), value_before);
    }

    #[test]
    fn tuning_presets_are_non_empty_and_include_defaults() {
        let presets = tuning_presets();
        assert!(!presets.is_empty());
        assert!(
            presets
                .iter()
                .any(|preset| preset.id == "defaults" && preset.blob.is_empty())
        );
    }
}
