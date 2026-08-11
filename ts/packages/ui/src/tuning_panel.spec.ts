import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { TuningPanel } from "./tuning_panel.ts";
import type { Knob, TuningPreset, TuningSource } from "./tuning_panel.ts";

// This fake stands in for the real knob registry (Rust `crates/gc-sim` —
// see tuning_panel.ts's header comment) just accurately enough to drive the
// panel's own state-machine logic (apply-on-top-of-reset, no stacking, wrap
// to defaults). It is a test double, not a reimplementation of the knob
// registry: its keys and values are synthetic, not the shipped balance
// numbers.
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

// This "tuning presets data" block validates the REAL preset blobs against
// the REAL knob registry — every preset line names a real knob within its
// min/max, and the first preset is pure defaults. Both source modules are
// Rust (crates/gc-data/src/tuning_presets.rs, crates/gc-sim/src/tuning.rs).
//
// This used to be `it.skip`, twice over: first because "the JS<->wasm
// bridge does not exist" (stale by the time the second pass landed), then
// because "SimHost's surface does not include the knob registry or preset
// list" (also now stale). `crates/gc-wasm/src/tuning_bridge.rs` landed
// since: `@gc/wasm`'s `SimHost` now exposes `TuningRegistry` (a live
// `gc_sim::tuning` knob registry) and `tuningPresets()`
// (`gc_data::tuning_presets::ALL`), and `TuningRegistry`'s own method set
// (`categories`/`inCategory`/`valueOf`/`nudge`/`reset`/`isDefault`/
// `serialize`/`deserialize`) already satisfies this file's own
// `TuningSource` interface structurally. Implemented for real below.
describe("tuning presets data", () => {
  it("every preset line names a real knob with an in-range value", () => {
    const host = loadSimHost();
    const registry = new host.TuningRegistry();
    const presets = host.tuningPresets();
    const knobsByKey = new Map<string, Knob>();
    for (const category of registry.categories()) {
      for (const knob of registry.inCategory(category)) {
        knobsByKey.set(knob.key, knob);
      }
    }
    for (const preset of presets) {
      for (const line of preset.blob.split(/\r?\n/)) {
        if (line === "") {
          continue;
        }
        const match = /^([A-Za-z0-9_]+)=(-?[\d.eE-]+)$/.exec(line);
        expect(match, `${preset.id}: malformed line ${line}`).not.toBeNull();
        const key = match?.[1] as string;
        const value = Number(match?.[2]);
        const knob = knobsByKey.get(key);
        expect(knob, `${preset.id}: unknown knob ${key}`).toBeDefined();
        expect(
          knob !== undefined && value >= knob.min && value <= knob.max,
          `${preset.id}: ${key} out of range`,
        ).toBe(true);
      }
    }
  });

  it("the first preset is pure defaults", () => {
    const host = loadSimHost();
    const presets = host.tuningPresets();
    expect(presets[0]?.blob).toBe("");
  });
});
