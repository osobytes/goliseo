// Wires keyboard/gamepad capture, `bindings.ts`'s CONTROLS table, and
// `input_sample.ts`'s quantize/pack rules together into an actual
// `InputSample` per render frame -- the concrete answer to this package's
// GOAL ("keyboard and gamepad events -> values shaped like InputSample").
//
// It only goes as far as the physical controls that mean the same thing in
// every context. The match screen's real per-frame construction shows two
// different kinds of `InputSample` bit:
//
//   * CONTEXT-FREE: `move` (from the raw movement axis), `dodge` (a JUKE
//     press always fires it, no possession check), `sprint` (a direct
//     held-key read), and `equipment_held`/`equipment_pressed`/
//     `equipment_released` (also direct/poll-diffed, no possession check).
//     These need nothing but the physical controls, and this file computes
//     them for real.
//
//   * CONTEXTUAL: `shoot`/`shoot_held`, `pass`/`pass_held`, `switch`,
//     `dash`, `jockey`, `lob`, `aerial_strike`, `aerial_acrobatic`. Every
//     one of these is gated on whether this player currently has
//     possession, or stateful latches derived from that -- match-state
//     knowledge `@gc/input` structurally cannot have (ARCHITECTURE.md §1's
//     package table puts the match screen itself in
//     `@gc/screens`, not `@gc/input`; this package owns "capture
//     and bindings" only).
//
// So this module does NOT reimplement the match screen's contextual state
// machine -- doing so here would duplicate logic that belongs one layer up
// and, worse, fork it from the real thing the moment either copy changed.
// Instead, [`InputSampleCapture.sample`] takes the contextual booleans as an
// explicit parameter, following the same seam `controller.ts` already
// uses for `ViewportMapper` and the README's §6 rule 7 for content tables:
// take what you cannot own as a parameter, not an import. Once
// `packages/screens`' match screen exists (Wave 2, needs a wasm-compiled
// `gc-sim` per the four skipped specs this package's task brief points at:
// match_screen.spec.ts, match_gamepad.spec.ts), it computes those booleans
// from live `MatchState` and calls this function once per render frame.
// Until then every contextual field defaults to neutral, matching
// `neutralSample()`.

import { bindings } from "./bindings.ts";
import type { GamepadState, KeyboardState } from "./bindings.ts";
import {
  buildInputSample,
  type EdgeActionName,
  type HeldActionName,
  type InputSample,
} from "./input_sample.ts";
import { controller } from "./controller.ts";
import type { ControllerInputEvent, ViewportMapper, ViewportTransform } from "./controller.ts";

/**
 * The `InputSample` bits `@gc/input` cannot compute on its own -- see this
 * file's header. Every field defaults to `false` (neutral), matching
 * `neutralSample()`; Wave 2's match screen is expected to supply the real
 * values once it has `MatchState` to derive them from.
 */
export interface ContextualInputFields {
  readonly shoot?: boolean;
  readonly shootHeld?: boolean;
  readonly pass?: boolean;
  readonly passHeld?: boolean;
  readonly switchPlayer?: boolean;
  readonly jockey?: boolean;
  readonly lob?: boolean;
  readonly aerialStrike?: boolean;
  readonly aerialAcrobatic?: boolean;
}

// `controller.normalize` needs a transform/viewportMapper argument for its
// "click" branch, which this file's events (key/gamepad only) never take.
// A trivial pair satisfies the signature without this package depending on
// `@gc/ui` for real viewport math it has no click to apply.
const NOOP_TRANSFORM: ViewportTransform = {
  baseW: 1,
  baseH: 1,
  actualW: 1,
  actualH: 1,
  scale: 1,
  offsetX: 0,
  offsetY: 0,
};
const NOOP_VIEWPORT_MAPPER: ViewportMapper = { toVirtual: () => null };

/** The analog stick's raw magnitude below which it reads as centered. */
export const STICK_DEADZONE = 0.2;

/** WASD/D-pad plus the raw left stick, context-free. */
export function readMoveAxis(
  keyboard: KeyboardState,
  gamepad?: GamepadState,
): readonly [x: number, y: number] {
  let x = 0;
  let y = 0;
  if (bindings.isDown("move_left", keyboard, gamepad)) {
    x -= 1;
  }
  if (bindings.isDown("move_right", keyboard, gamepad)) {
    x += 1;
  }
  if (bindings.isDown("move_up", keyboard, gamepad)) {
    y -= 1;
  }
  if (bindings.isDown("move_down", keyboard, gamepad)) {
    y += 1;
  }
  if (gamepad) {
    const gx = gamepad.getGamepadAxis("leftx");
    const gy = gamepad.getGamepadAxis("lefty");
    if (Math.abs(gx) >= STICK_DEADZONE) {
      x += gx;
    }
    if (Math.abs(gy) >= STICK_DEADZONE) {
      y += gy;
    }
  }
  return [x, y];
}

function heldActionsOf(
  keyboard: KeyboardState,
  gamepad: GamepadState | undefined,
  equipmentHeld: boolean,
  contextual: ContextualInputFields,
): readonly HeldActionName[] {
  const held: HeldActionName[] = [];
  if (contextual.shootHeld === true) {
    held.push("shoot");
  }
  if (contextual.passHeld === true) {
    held.push("pass");
  }
  if (bindings.isDown("sprint", keyboard, gamepad)) {
    held.push("sprint");
  }
  if (contextual.jockey === true) {
    held.push("jockey");
  }
  if (contextual.lob === true) {
    held.push("lob");
  }
  if (contextual.aerialStrike === true) {
    held.push("aerial_strike");
  }
  if (contextual.aerialAcrobatic === true) {
    held.push("aerial_acrobatic");
  }
  if (equipmentHeld) {
    held.push("equipment");
  }
  return held;
}

function edgeActionsOf(
  contextual: ContextualInputFields,
  dodge: boolean,
  equipmentPressed: boolean,
  equipmentReleased: boolean,
): readonly EdgeActionName[] {
  const edges: EdgeActionName[] = [];
  if (contextual.shoot === true) {
    edges.push("shoot");
  }
  if (contextual.pass === true) {
    edges.push("pass");
  }
  if (contextual.switchPlayer === true) {
    edges.push("switch");
  }
  // `dash` has no context-free source and no contextual field either: it
  // is derived from the jockey-release edge, which needs both a
  // previous-frame contextual value AND this frame's contextual value --
  // strictly Wave 2's state to hold, not a boolean this call can take
  // neutrally.
  if (dodge) {
    edges.push("dodge");
  }
  if (equipmentPressed) {
    edges.push("equipment_pressed");
  }
  if (equipmentReleased) {
    edges.push("equipment_released");
  }
  return edges;
}

/**
 * Captures one render frame's context-free physical input (movement,
 * sprint, dodge, equipment) and merges it with `contextual`'s match-aware
 * fields (defaulting to neutral) into one real `InputSample`.
 *
 * Stateful across calls for the two edges that need frame-to-frame
 * diffing: `dodge` from queued JUKE press events (a one-shot flag per
 * press), and `equipment_pressed`/`equipment_released` from comparing
 * this poll's `equipment` hold against the previous one.
 */
export class InputSampleCapture {
  // Explicit fields rather than constructor parameter properties: this
  // workspace's tsconfig sets `erasableSyntaxOnly` (ARCHITECTURE.md §4 rule
  // 1's TypeScript strictness posture), which forbids parameter properties
  // because they are not erasable syntax (they emit real field-assignment
  // code, not just types).
  private readonly keyboard: KeyboardState;
  private readonly gamepad: GamepadState | undefined;
  private readonly drainKeyEvents: () => readonly ControllerInputEvent[];
  private readonly drainGamepadEvents: () => readonly ControllerInputEvent[];
  private _equipmentWasHeld = false;

  /**
   * @param keyboard Typically a [`BrowserKeyboardCapture`] (`./capture_keyboard.ts`).
   * @param gamepad Typically a [`BrowserGamepadCapture`] (`./capture_gamepad.ts`); omit when no pad is connected.
   * @param drainKeyEvents Drains queued `KeyEvent`s since the last sample (e.g. `keyboard.drainKeyEvents`).
   * @param drainGamepadEvents Drains queued `RawGamepadEvent`s since the last sample (e.g. `gamepad.drainGamepadEvents`).
   */
  constructor(
    keyboard: KeyboardState,
    gamepad: GamepadState | undefined,
    drainKeyEvents: () => readonly ControllerInputEvent[],
    drainGamepadEvents: () => readonly ControllerInputEvent[],
  ) {
    this.keyboard = keyboard;
    this.gamepad = gamepad;
    this.drainKeyEvents = drainKeyEvents;
    this.drainGamepadEvents = drainGamepadEvents;
  }

  /** Build this frame's `InputSample`. `contextual` fields default to neutral -- see [`ContextualInputFields`]. */
  sample(contextual: ContextualInputFields = {}): InputSample {
    const dodge = this.consumeDodgeEdge();
    const equipmentHeld = bindings.isDown("equipment", this.keyboard, this.gamepad);
    const equipmentPressed = equipmentHeld && !this._equipmentWasHeld;
    const equipmentReleased = !equipmentHeld && this._equipmentWasHeld;
    this._equipmentWasHeld = equipmentHeld;

    const [rawX, rawY] = readMoveAxis(this.keyboard, this.gamepad);
    return buildInputSample({
      rawMoveX: rawX,
      rawMoveY: rawY,
      held: heldActionsOf(this.keyboard, this.gamepad, equipmentHeld, contextual),
      edges: edgeActionsOf(contextual, dodge, equipmentPressed, equipmentReleased),
    });
  }

  private consumeDodgeEdge(): boolean {
    let dodge = false;
    for (const event of [...this.drainKeyEvents(), ...this.drainGamepadEvents()]) {
      const normalized = controller.normalize(event, NOOP_TRANSFORM, NOOP_VIEWPORT_MAPPER);
      if (
        normalized?.kind === "action" &&
        normalized.action === "juke" &&
        normalized.pressed === true
      ) {
        dodge = true;
      }
    }
    return dodge;
  }
}
