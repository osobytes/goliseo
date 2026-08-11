// Mirrors the wire-shape half of `gc_sim::input_frame`
// (rust/crates/gc-sim/src/input_frame.rs) -- constants, quantization,
// and bit-packing only, not the module wholesale: validation of the
// frame/ownership/slot machinery stays Rust-owned, and encode()/decode() (the
// wire codec) stay Rust-owned too -- ARCHITECTURE.md §1.1 draws the line at
// "input capture is TS; input *encoding* is Rust."
//
// This file exists on the TS side of that line anyway, and that needs
// explaining. The wasm boundary this milestone stops short of does not
// exist yet, so there is today no way
// for a browser capture layer to hand a raw held/edge snapshot to Rust and
// get an `InputSample` back -- Wave 2 wires that marshalling. Until then,
// producing an actual `InputSample`-shaped value at all means quantizing
// and bit-packing here, in TypeScript, which duplicates a determinism-path
// algorithm exactly the way ARCHITECTURE.md §1.2 describes for `fnv1a64`: allowed,
// but only pinned by a shared vector file, never trusted by construction.
// `../fixtures/input_sample_vector.txt` (generated straight from this
// crate, see `gc-sim/tests/input_sample_vector_generator.rs`) is that
// vector; `input_sample.spec.ts` asserts every case in it. Once Wave 2's
// marshalling exists, the intent is for this module's quantize/pack half
// to be deleted in favor of calling the real wasm `gc_sim::input_frame`
// functions, leaving only capture behind the determinism line.
//
// `InputSample` itself has no optional/nullable field -- `move_x`,
// `move_y`, `held`, and `edges` are always-populated integers, packed as
// bitmasks. That is worth stating plainly because the Rust sibling type one
// layer up, `gc_sim::match_snapshot::MatchInput`, is NOT like this: its
// `aerial_strike`/`aerial_acrobatic` fields are `Option<bool>`, and
// `Some(false)` ("explicitly not requested") is not interchangeable with
// `None` ("unset, fall back to jockey||dash") -- see
// `gc_sim::aerial::strike_requested`'s doc comment. `InputSample` cannot
// represent that distinction at all: its bitmask has exactly two states per
// action, held or not. `gc_sim::slot_input::to_match_input` (the function
// that turns a captured `InputSample` back into a `MatchInput` for the sim
// to consume) reflects this on purpose -- it always writes
// `aerial_strike: Some(held(...))`, a real, decided boolean, never `None`.
// A capture layer built on `InputSample`, like this one, therefore can
// never accidentally produce the `None`/fallback case: every bit this
// module packs is a definite yes-or-no, exactly matching what a real human
// pressing or not pressing a physical control means. There is nothing for
// this file to get wrong here, and that is by construction, not vigilance.

/** Mirrors `gc_sim::input_frame::VERSION`. */
export const VERSION = 2;
/** Mirrors `gc_sim::input_frame::MOVE_SCALE`. */
export const MOVE_SCALE = 127;
/** Mirrors `gc_sim::input_frame::MAX_HELD_MASK` (not exported by the Rust module, but implied by its eight bits). */
export const MAX_HELD_MASK = 255;
/** Mirrors `gc_sim::input_frame::MAX_EDGE_MASK` (not exported by the Rust module, but implied by its seven bits). */
export const MAX_EDGE_MASK = 127;

/** Mirrors `gc_sim::input_frame::HeldAction`. */
export type HeldActionName =
  | "shoot"
  | "pass"
  | "sprint"
  | "jockey"
  | "lob"
  | "aerial_strike"
  | "aerial_acrobatic"
  | "equipment";

/** Mirrors `gc_sim::input_frame::EdgeAction`. */
export type EdgeActionName =
  | "shoot"
  | "pass"
  | "switch"
  | "dash"
  | "dodge"
  | "equipment_pressed"
  | "equipment_released";

/** Mirrors each `HeldAction::bit()` value. */
const HELD_BITS: Readonly<Record<HeldActionName, number>> = {
  shoot: 1,
  pass: 2,
  sprint: 4,
  jockey: 8,
  lob: 16,
  aerial_strike: 32,
  aerial_acrobatic: 64,
  equipment: 128,
};

/** Mirrors each `EdgeAction::bit()` value. */
const EDGE_BITS: Readonly<Record<EdgeActionName, number>> = {
  shoot: 1,
  pass: 2,
  switch: 4,
  dash: 8,
  dodge: 16,
  equipment_pressed: 32,
  equipment_released: 64,
};

/** Mirrors `gc_sim::input_frame::InputSample`. */
export interface InputSample {
  /** Signed quantized horizontal axis in `[-127, 127]`. */
  readonly move_x: number;
  /** Signed quantized vertical axis in `[-127, 127]`. */
  readonly move_y: number;
  /** Bitmask of [`HeldActionName`] values currently held for this tick. */
  readonly held: number;
  /** Bitmask of [`EdgeActionName`] values that occurred during this tick only. */
  readonly edges: number;
}

/** The all-zero sample: no movement, nothing held, no edges. Mirrors `neutral_sample`. */
export function neutralSample(): InputSample {
  return { move_x: 0, move_y: 0, held: 0, edges: 0 };
}

/**
 * Quantize a raw `[-1, 1]` axis into a signed 8-bit value, saturating
 * outside that range and rounding half-away-from-zero. Mirrors
 * `gc_sim::input_frame::quantize_axis` bit for bit -- see that function's
 * doc for the algorithm this is not permitted to "simplify" (ARCHITECTURE.md
 * §3 rule 2, which binds TS ports of determinism-path math exactly as it
 * binds Rust ports).
 */
export function quantizeAxis(rawAxis: number): number {
  if (!Number.isFinite(rawAxis)) {
    throw new Error("raw movement axis must be finite");
  }
  const clamped = Math.max(-1, Math.min(1, rawAxis));
  const scaled = clamped * MOVE_SCALE;
  const quantized = scaled >= 0 ? Math.floor(scaled + 0.5) : Math.ceil(scaled - 0.5);
  if (quantized === 0) {
    return 0;
  }
  return quantized;
}

/** Quantize a raw `(x, y)` movement vector. Mirrors `quantize_move`. */
export function quantizeMove(rawX: number, rawY: number): readonly [moveX: number, moveY: number] {
  return [quantizeAxis(rawX), quantizeAxis(rawY)];
}

/** Pack a set of held actions into an [`InputSample.held`] bitmask. */
export function packHeld(actions: readonly HeldActionName[]): number {
  let mask = 0;
  for (const action of actions) {
    mask |= HELD_BITS[action];
  }
  return mask;
}

/** Pack a set of edge actions into an [`InputSample.edges`] bitmask. */
export function packEdges(actions: readonly EdgeActionName[]): number {
  let mask = 0;
  for (const action of actions) {
    mask |= EDGE_BITS[action];
  }
  return mask;
}

function isAxis(value: number): boolean {
  return Number.isInteger(value) && value >= -MOVE_SCALE && value <= MOVE_SCALE;
}

function isMask(value: number, maximum: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= maximum;
}

function hasBit(mask: number, bit: number): boolean {
  return (mask & bit) !== 0;
}

/**
 * Validate a sample's movement axes, masks, and equipment edge/hold
 * consistency. Mirrors `gc_sim::input_frame::validate_sample`.
 */
export function validateSample(sample: InputSample): void {
  if (!isAxis(sample.move_x) || !isAxis(sample.move_y)) {
    throw new Error("input sample movement axes must be signed 8-bit integers");
  }
  if (!isMask(sample.held, MAX_HELD_MASK)) {
    throw new Error("input sample held mask is invalid");
  }
  if (!isMask(sample.edges, MAX_EDGE_MASK)) {
    throw new Error("input sample edge mask is invalid");
  }
  const equipmentHeld = hasBit(sample.held, HELD_BITS.equipment);
  const equipmentPressed = hasBit(sample.edges, EDGE_BITS.equipment_pressed);
  const equipmentReleased = hasBit(sample.edges, EDGE_BITS.equipment_released);
  if (
    (equipmentPressed && !equipmentReleased && !equipmentHeld) ||
    (equipmentReleased && equipmentHeld)
  ) {
    throw new Error("input sample equipment transition combination is invalid");
  }
}

/** Options for [`buildInputSample`]; every field defaults to empty/zero, matching [`neutralSample`]. */
export interface InputSampleOptions {
  readonly rawMoveX?: number;
  readonly rawMoveY?: number;
  readonly held?: readonly HeldActionName[];
  readonly edges?: readonly EdgeActionName[];
}

/**
 * Build a validated [`InputSample`] from raw movement and named held/edge
 * actions. Mirrors the composition of `quantize_move` + bit-packing +
 * `new_sample`/`validate_sample` a captured frame goes through on the Rust
 * side (see `gc_sim::slot_input::to_sample`, which performs the same
 * quantize-then-pack sequence from the other direction).
 */
export function buildInputSample(options: InputSampleOptions = {}): InputSample {
  const [moveX, moveY] = quantizeMove(options.rawMoveX ?? 0, options.rawMoveY ?? 0);
  const sample: InputSample = {
    move_x: moveX,
    move_y: moveY,
    held: packHeld(options.held ?? []),
    edges: packEdges(options.edges ?? []),
  };
  validateSample(sample);
  return sample;
}
