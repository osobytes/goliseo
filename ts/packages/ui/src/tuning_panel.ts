// In-match tuning panel (F1): live gameplay-knob editing for playtesting.
// State transitions are pure and testable headless (see
// tuning_panel.spec.ts); `draw` is the only place a `GraphicsBackend`
// is touched, and `save`/`load` are the only place a `PanelStorage` is.
//
// The live gameplay-knob registry and preset list live in Rust
// (`crates/gc-sim`, `crates/gc-data`) and are read directly there: tuning
// knobs are read live by the simulation, and ARCHITECTURE.md's
// determinism-line rule is
// "can this code change what the simulation computes? If yes it is Rust,
// even when it feels like presentation." This package cannot import
// `crates/gc-sim`/`crates/gc-data` directly (TS never imports a Rust
// crate's source, ARCHITECTURE.md §7) and, unlike when this note was first
// written, the wasm bridge itself is no longer missing — `@gc/wasm` is real
// and working (`packages/wasm/src/index.ts`'s `SimHost`: session lifecycle,
// determinism evidence, the raw per-frame RenderFrame path). What `SimHost`
// does NOT expose is the tuning knob registry or the preset list — no
// method surfaces `gc_sim::tuning` or `gc_data::tuning_presets` (confirmed
// by grepping `crates/gc-wasm/src` — `tuning` only appears as an internal
// `Tuning` type threaded through session construction, never as a bound
// export). So the same pattern as `GraphicsBackend` still applies, for a
// narrower reason than before: the knob registry and preset list are
// injected through `TuningSource`/`TuningPreset[]`, and this file implements
// the panel's own state machine only. Wiring a real `TuningSource` needs a new
// `gc-wasm` export for the knob registry/presets specifically, not "the
// bridge" in general — that part already shipped.

import { invariant } from "./assert.ts";
import type { GraphicsBackend } from "./graphics_backend.ts";

export interface Knob {
  readonly key: string;
  readonly label: string;
  readonly cat: string;
  readonly default: number;
  readonly min: number;
  readonly max: number;
  readonly step: number;
}

/**
 * What the panel needs from the live gameplay-knob registry
 * (Rust `crates/gc-sim`). See this file's header for why it is injected
 * rather than imported.
 */
export interface TuningSource {
  categories(): readonly string[];
  inCategory(cat: string): readonly Knob[];
  valueOf(key: string): number;
  nudge(key: string, steps: number): void;
  reset(key?: string): void;
  isDefault(key: string): boolean;
  serialize(): string;
  deserialize(blob: string): void;
}

/** A named blob of non-default overrides (Rust `crates/gc-data`). */
export interface TuningPreset {
  readonly id: string;
  readonly name: string;
  readonly blob: string;
}

/** Storage abstraction for save/load. Omit it to get no-op headless behavior. */
export interface PanelStorage {
  write(filename: string, data: string): void;
  /** `null` when the file does not exist. */
  read(filename: string): string | null;
}

const SAVE_FILE = "tuning.txt";

/** Best-effort `%.6g` printf-style formatting, for display only — never on the determinism path. */
function formatG6(value: number): string {
  if (value === 0) {
    return "0";
  }
  const precision = 6;
  const exponent = Math.floor(Math.log10(Math.abs(value)));
  if (exponent < -4 || exponent >= precision) {
    return value.toExponential(precision - 1).replace(/\.?0+e/, "e");
  }
  const decimals = Math.max(0, precision - 1 - exponent);
  const fixed = value.toFixed(decimals);
  return fixed.includes(".") ? fixed.replace(/0+$/, "").replace(/\.$/, "") : fixed;
}

export class TuningPanel {
  open = false;
  cat = 0; // 0-based index into tuning.categories()
  row = 0; // 0-based index into the current category's knobs
  preset = 0; // 0-based index into presets; 0 = defaults
  status: string | null = null;

  private readonly tuning: TuningSource;
  private readonly presets: readonly TuningPreset[];
  private readonly storage: PanelStorage | undefined;

  constructor(tuning: TuningSource, presets: readonly TuningPreset[], storage?: PanelStorage) {
    this.tuning = tuning;
    this.presets = presets;
    this.storage = storage;
  }

  private cats(): readonly string[] {
    return this.tuning.categories();
  }

  private rows(): readonly Knob[] {
    const cats = this.cats();
    const name = cats[this.cat];
    invariant(name !== undefined, `tuning panel category index out of range: ${this.cat}`);
    return this.tuning.inCategory(name);
  }

  toggle(): void {
    this.open = !this.open;
    this.status = null;
  }

  /** Save overrides through `storage`. No-op when none was provided (headless). */
  save(): void {
    if (!this.storage) {
      return;
    }
    this.storage.write(SAVE_FILE, this.tuning.serialize());
    this.status = `saved to ${SAVE_FILE}`;
  }

  /** @returns whether a saved blob was found and loaded. */
  load(): boolean {
    if (!this.storage) {
      return false;
    }
    const data = this.storage.read(SAVE_FILE);
    if (data === null) {
      return false;
    }
    this.tuning.deserialize(data);
    this.status = `loaded ${SAVE_FILE}`;
    return true;
  }

  /** Handle a key while the panel is open. `big` = large steps (shift held). @returns whether it was consumed. */
  key(key: string, big: boolean): boolean {
    if (!this.open) {
      return false;
    }
    const rows = this.rows();
    const n = rows.length;
    const mult = big ? 10 : 1;
    if (key === "up") {
      this.row = (((this.row - 1) % n) + n) % n;
    } else if (key === "down") {
      this.row = (this.row + 1) % n;
    } else if (key === "left") {
      const knob = rows[this.row];
      invariant(knob !== undefined, `tuning panel row index out of range: ${this.row}`);
      this.tuning.nudge(knob.key, -mult);
    } else if (key === "right") {
      const knob = rows[this.row];
      invariant(knob !== undefined, `tuning panel row index out of range: ${this.row}`);
      this.tuning.nudge(knob.key, mult);
    } else if (key === "tab") {
      const cats = this.cats();
      this.cat = (this.cat + 1) % cats.length;
      this.row = 0;
    } else if (key === "backspace") {
      if (big) {
        this.tuning.reset();
        this.status = "ALL knobs reset to defaults";
      } else {
        const knob = rows[this.row];
        invariant(knob !== undefined, `tuning panel row index out of range: ${this.row}`);
        this.tuning.reset(knob.key);
        this.status = null;
      }
    } else if (key === "f2") {
      this.save();
    } else if (key === "f3") {
      if (!this.load()) {
        this.status = "no saved tuning";
      }
    } else if (key === "f4") {
      // Cycle balance presets: each applies its blob on top of a full reset,
      // so presets never stack.
      this.preset = (this.preset + 1) % this.presets.length;
      const p = this.presets[this.preset];
      invariant(p !== undefined, `tuning panel preset index out of range: ${this.preset}`);
      this.tuning.deserialize(p.blob);
      this.status = `preset: ${p.name}`;
    } else if (key === "f1" || key === "escape") {
      this.open = false;
    } else {
      return false;
    }
    return true;
  }

  /** Draw the overlay. Stub-safe: only setColor/rectangle/print are used. */
  draw(backend: GraphicsBackend, _vp: { readonly w: number; readonly h: number }): void {
    if (!this.open) {
      return;
    }
    const w = 380;
    const x0 = 24;
    const y0 = 60;
    const rows = this.rows();
    const h = 96 + rows.length * 22;
    backend.setColor(0.02, 0.05, 0.12, 0.92);
    backend.rectangle("fill", x0, y0, w, h);
    backend.setColor(0.35, 0.75, 1.0, 0.9);
    backend.rectangle("line", x0, y0, w, h);

    backend.setColor(1, 1, 1);
    backend.print("TUNING (paused)", x0 + 12, y0 + 10);
    const cats = this.cats();
    const catLine = cats.map((c, i) => (i === this.cat ? `[${c}]` : ` ${c} `)).join(" ");
    backend.setColor(0.7, 0.9, 1);
    backend.print(catLine, x0 + 12, y0 + 30);

    rows.forEach((k, i) => {
      const y = y0 + 56 + i * 22;
      if (i === this.row) {
        backend.setColor(0.2, 0.45, 0.7, 0.5);
        backend.rectangle("fill", x0 + 6, y - 2, w - 12, 20);
      }
      const v = this.tuning.valueOf(k.key);
      const changed = !this.tuning.isDefault(k.key);
      backend.setColor(changed ? 1 : 0.75, changed ? 0.8 : 0.85, changed ? 0.4 : 0.9);
      backend.print(k.label, x0 + 14, y);
      backend.print(`${formatG6(v)}${changed ? " *" : ""}`, x0 + 210, y);
      const frac = (v - k.min) / (k.max - k.min);
      backend.setColor(0.15, 0.3, 0.45, 0.9);
      backend.rectangle("fill", x0 + 292, y + 4, 74, 8);
      backend.setColor(0.45, 0.85, 1, 0.95);
      backend.rectangle("fill", x0 + 292, y + 4, 74 * Math.max(0, Math.min(1, frac)), 8);
    });

    backend.setColor(0.65, 0.75, 0.85);
    backend.print(
      "←/→ adjust (Shift = x10)   Tab category   Bksp reset (Shift = all)",
      x0 + 12,
      y0 + h - 34,
    );
    backend.print("F2 save   F3 load   F4 preset   F1/Esc close", x0 + 12, y0 + h - 18);
    if (this.status !== null) {
      backend.setColor(0.5, 1, 0.7);
      backend.print(this.status, x0 + 12, y0 + h + 6);
    }
  }
}
