import { describe, expect, it } from "vitest";
import { TuningPanel } from "./tuning_panel.ts";
import type { Knob, TuningPreset, TuningSource } from "./tuning_panel.ts";

// The Lua spec (spec/ui/tuning_panel_spec.lua) exercises the panel against
// the REAL knob registry (sim/tuning.lua) and REAL presets
// (data/tuning_presets.lua). Both are Rust now — see tuning_panel.ts's
// header comment — so this fake stands in for `sim/tuning.lua`'s behavior
// just accurately enough to drive the panel's own state-machine logic
// (apply-on-top-of-reset, no stacking, wrap to defaults). It is a test
// double, not a second port of the knob registry: its keys and values are
// synthetic, not the shipped balance numbers.
function makeFakeTuning(knobs: readonly Knob[]): TuningSource {
  const byKey = new Map(knobs.map((k) => [k.key, k]));
  const values = new Map(knobs.map((k) => [k.key, k.default]));

  const source: TuningSource = {
    categories() {
      const seen = new Set<string>();
      const cats: string[] = [];
      for (const k of knobs) {
        if (!seen.has(k.cat)) {
          seen.add(k.cat);
          cats.push(k.cat);
        }
      }
      return cats;
    },
    inCategory(cat) {
      return knobs.filter((k) => k.cat === cat);
    },
    valueOf(key) {
      return values.get(key) ?? 0;
    },
    nudge(key, steps) {
      const k = byKey.get(key);
      if (!k) {
        return;
      }
      const current = values.get(key) ?? k.default;
      values.set(key, Math.max(k.min, Math.min(k.max, current + k.step * steps)));
    },
    reset(key) {
      if (key === undefined) {
        for (const k of knobs) {
          values.set(k.key, k.default);
        }
        return;
      }
      const k = byKey.get(key);
      if (k) {
        values.set(key, k.default);
      }
    },
    isDefault(key) {
      const k = byKey.get(key);
      return k !== undefined && values.get(key) === k.default;
    },
    serialize() {
      const parts: string[] = [];
      for (const k of knobs) {
        const v = values.get(k.key);
        if (v !== undefined && v !== k.default) {
          parts.push(`${k.key}=${v}`);
        }
      }
      return parts.join("\n");
    },
    deserialize(blob) {
      source.reset();
      for (const line of blob.split(/\r?\n/)) {
        const match = /^([A-Za-z0-9_]+)=(-?[\d.eE-]+)$/.exec(line);
        if (!match) {
          continue;
        }
        const key = match[1];
        const num = match[2];
        if (key === undefined || num === undefined) {
          continue;
        }
        const v = Number(num);
        const k = byKey.get(key);
        if (k && Number.isFinite(v)) {
          values.set(key, Math.max(k.min, Math.min(k.max, v)));
        }
      }
    },
  };
  return source;
}

const KNOBS: readonly Knob[] = [
  { key: "ALPHA", label: "Alpha", cat: "Movement", default: 100, min: 0, max: 500, step: 10 },
  { key: "BETA", label: "Beta", cat: "Movement", default: 700, min: 100, max: 2000, step: 50 },
  { key: "GAMMA", label: "Gamma", cat: "Keeper", default: 300, min: 100, max: 480, step: 10 },
];

const PRESETS: readonly TuningPreset[] = [
  { id: "defaults", name: "Defaults", blob: "" },
  { id: "candidate_a", name: "Candidate A", blob: "ALPHA=340\nBETA=700" },
  { id: "candidate_b", name: "Candidate B", blob: "ALPHA=300" },
];

describe("tuning panel F4 preset cycling", () => {
  it("applies each preset on top of a reset and wraps back to defaults", () => {
    const tuning = makeFakeTuning(KNOBS);
    const panel = new TuningPanel(tuning, PRESETS);
    panel.open = true;
    panel.preset = 0;

    panel.key("f4", false); // -> candidate A
    expect(tuning.valueOf("ALPHA")).toBe(340);
    expect(tuning.valueOf("BETA")).toBe(700);
    expect(panel.status?.includes("Candidate A")).toBe(true);

    panel.key("f4", false); // -> candidate B: A's other overrides must clear
    expect(tuning.valueOf("ALPHA")).toBe(300);
    expect(tuning.isDefault("BETA")).toBe(true); // presets replace, never stack

    panel.key("f4", false); // -> wraps to defaults
    expect(tuning.isDefault("ALPHA")).toBe(true);

    panel.open = false;
  });
});

describe("tuning panel row/category navigation", () => {
  it("wraps rows within a category and categories on Tab", () => {
    const tuning = makeFakeTuning(KNOBS);
    const panel = new TuningPanel(tuning, PRESETS);
    panel.open = true;

    expect(panel.row).toBe(0);
    panel.key("up", false); // wraps to the last row in "Movement" (2 knobs)
    expect(panel.row).toBe(1);
    panel.key("down", false); // wraps back to the first
    expect(panel.row).toBe(0);

    panel.key("tab", false); // -> "Keeper"
    expect(panel.cat).toBe(1);
    expect(panel.row).toBe(0);
    panel.key("tab", false); // wraps back to "Movement"
    expect(panel.cat).toBe(0);

    panel.open = false;
  });

  it("nudges and resets a single knob without touching the others", () => {
    const tuning = makeFakeTuning(KNOBS);
    const panel = new TuningPanel(tuning, PRESETS);
    panel.open = true;

    panel.key("right", false); // ALPHA += step (10)
    expect(tuning.valueOf("ALPHA")).toBe(110);
    panel.key("right", true); // big step: ALPHA += step * 10
    expect(tuning.valueOf("ALPHA")).toBe(210);
    panel.key("backspace", false); // reset just ALPHA
    expect(tuning.isDefault("ALPHA")).toBe(true);

    panel.open = false;
  });
});

// Skipped: the Lua spec's "tuning presets data" block validates the REAL
// preset blobs (data/tuning_presets.lua) against the REAL knob registry
// (sim/tuning.lua) — every preset line names a real knob within its
// min/max, and the first preset is pure defaults. Both source modules are
// Rust now (crates/gc-data/src/tuning_presets.rs, crates/gc-sim), per
// v2/README.md's determinism-line rule, and this milestone explicitly
// excludes the JS<->wasm bridge that would let this package read them (see
// TuningSource's doc comment in tuning_panel.ts). Re-port this coverage as
// a Rust test once crates/gc-sim/src/tuning.rs exists.
describe.skip("tuning presets data", () => {
  it.skip("every preset line names a real knob with an in-range value", () => {
    // Not expressible from @gc/ui: needs the real sim/tuning.lua knob
    // registry and data/tuning_presets.lua blobs, both Rust-owned.
  });

  it.skip("the first preset is pure defaults", () => {
    // Same as above.
  });
});
