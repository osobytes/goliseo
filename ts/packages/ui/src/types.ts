// Pure data shapes shared by the layout / hit-testing / rendering modules.
// No rendering engine, no three.js, no DOM here — see AGENTS.md §9 and
// hit.ts, draw.ts.

/** An RGB color triple, normalized 0..1; alpha is passed separately. */
export type RgbColor = readonly [number, number, number];

export type TextAlign = "left" | "center" | "right";

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/** A fractional (0..1) position within a formation-preview widget's rect. */
export interface Anchor {
  readonly x: number;
  readonly y: number;
}

export interface FormationMarkerData {
  readonly color: RgbColor;
  readonly shape?: string;
  readonly name: string;
}

export interface FormationPreviewData {
  readonly keeper: Anchor;
  readonly outfield: readonly Anchor[];
  readonly markers?: readonly FormationMarkerData[];
}

/**
 * `widget.data` is duck-typed: which fields are present depends on
 * `widget.kind`, and callers (focus.ts's `focus` helpers today, screen
 * modules later) only ever read the few fields relevant to them. This
 * flattens every shape used anywhere in the UI layer into one optional-field
 * interface rather than a `kind`-discriminated union, matching that
 * looseness instead of fighting it.
 */
export interface WidgetData extends Partial<FormationPreviewData> {
  readonly focusable?: boolean;
  readonly disabled?: boolean;
  readonly align?: TextAlign;
  readonly accent?: RgbColor;
  readonly speciesShape?: string;
  readonly locked?: boolean;
  readonly tone?: "muted";
}

/** A single positioned, drawable element in a layout. */
export interface Widget {
  readonly id: string;
  // Optional, not `Rect` — `hit.at` guards `w.rect` before reading it, so a
  // widget without a rect is a real (if defensive) case to keep honoring.
  readonly rect?: Rect;
  // Open string, not a closed union: draw.ts's dispatch falls back to
  // "label" for any kind it doesn't special-case. Known values today:
  // "button", "label", "card", "formation_preview", "hero_title", "title",
  // "eyebrow".
  readonly kind?: string;
  readonly text?: string;
  readonly selected?: boolean;
  readonly focused?: boolean;
  readonly data?: WidgetData;
}

export type Layout = readonly Widget[];

/**
 * The subset of `@gc/app`'s screen-stack `InputEvent` that `focus.ts` reads.
 * Defined locally (structurally compatible with the real thing) rather than
 * imported, because the real `InputEvent` lives in `@gc/app`, a package this
 * one must not depend on — that wiring belongs to whichever later milestone
 * actually composes screens, input, and app together.
 */
export type FocusEvent =
  | { readonly kind: "click"; readonly x: number; readonly y: number; readonly button?: number }
  | { readonly kind: "action"; readonly action: string; readonly pressed?: boolean }
  | { readonly kind: "key"; readonly key: string; readonly pressed?: boolean }
  | { readonly kind: "gamepad"; readonly button: string; readonly pressed?: boolean };
