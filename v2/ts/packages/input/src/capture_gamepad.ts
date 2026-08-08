// Browser gamepad capture. The Gamepad API has no press/change events --
// `navigator.getGamepads()` is a polled snapshot, unlike keyboard's real
// `keydown`/`keyup` (capture_keyboard.ts) -- so this file's shape differs
// from that one on purpose: [`poll`] must be called once per render frame,
// and edges are derived by diffing this poll's button states against the
// previous one, not by listening for anything.
//
// Button/axis INDEX -> LÖVE name follows the Gamepad API's "standard"
// mapping (https://www.w3.org/TR/gamepad/#remapping), which is also what
// `bindings.ts`'s `BUTTON_LABELS` map already assumes (its keys --
// "a"/"b"/"x"/"y"/"leftshoulder"/"dpup"/... -- are exactly the LÖVE names a
// standard-mapped pad produces here).
//
// One divergence worth flagging: LÖVE reports the analog triggers as
// AXES ("triggerleft"/"triggerright" -- see bindings.ts's rule 4 comment,
// `love.gamepadpressed` never fires for one). The browser's standard
// mapping reports them as BUTTONS 6/7 instead, each carrying an analog
// `.value` alongside `.pressed` -- there is no separate triggerleft/
// triggerright entry in `gamepad.axes` on a standard-mapped pad. So
// `getGamepadAxis("triggerleft"/"triggerright")` reads `buttons[6]/[7]
// .value` here, not `axes[...]`, to land on the same LÖVE-shaped read
// `GamepadState` promises regardless of which array the browser actually
// puts it in.

import type { GamepadState } from "./bindings.ts";
import type { RawGamepadEvent } from "./controller.ts";

/** `Gamepad`'s relevant surface, kept minimal and DOM-independent -- see capture_keyboard.ts's header for why. */
export interface GamepadLike {
  readonly connected: boolean;
  readonly buttons: readonly { readonly pressed: boolean; readonly value: number }[];
  readonly axes: readonly number[];
}

/** A poll function returning the current gamepad snapshot array, mirroring `navigator.getGamepads()`. */
export type GamepadSource = () => readonly (GamepadLike | null | undefined)[];

// Standard Gamepad Mapping button index -> LÖVE `GamepadButton` name.
// Index 16 ("guide"/home) has no LÖVE analogue in bindings.ts and is
// omitted on purpose -- an unmapped index reads as simply not present,
// matching `translateCode`'s "closed, not guessed" rule in
// capture_keyboard.ts.
export const GAMEPAD_BUTTON_MAP: ReadonlyMap<number, string> = new Map([
  [0, "a"],
  [1, "b"],
  [2, "x"],
  [3, "y"],
  [4, "leftshoulder"],
  [5, "rightshoulder"],
  [8, "back"],
  [9, "start"],
  [10, "leftstick"],
  [11, "rightstick"],
  [12, "dpup"],
  [13, "dpdown"],
  [14, "dpleft"],
  [15, "dpright"],
]);

// Standard Gamepad Mapping axis index -> LÖVE `GamepadAxis` name. The
// trigger axes are handled separately (see the module doc): they are not
// in this table because on a standard-mapped pad they never appear in
// `gamepad.axes` at all.
export const GAMEPAD_AXIS_MAP: ReadonlyMap<number, string> = new Map([
  [0, "leftx"],
  [1, "lefty"],
  [2, "rightx"],
  [3, "righty"],
]);

const TRIGGER_LEFT_BUTTON_INDEX = 6;
const TRIGGER_RIGHT_BUTTON_INDEX = 7;

function defaultSource(): readonly (GamepadLike | null | undefined)[] {
  if (typeof navigator === "undefined" || typeof navigator.getGamepads !== "function") {
    return [];
  }
  return navigator.getGamepads();
}

/**
 * Polls one gamepad slot and tracks its button/axis state for
 * [`GamepadState`], queuing a [`RawGamepadEvent`] per button press/release
 * edge for a consumer to drain (mirroring `BrowserKeyboardCapture`'s
 * `drainKeyEvents`).
 *
 * Rule 4 (bindings.ts) holds here too: only BUTTONS produce edges. Trigger
 * reads (`getGamepadAxis("triggerleft"/"triggerright")`) never queue a
 * `RawGamepadEvent`, matching LÖVE's `love.gamepadpressed` never firing for
 * an axis.
 */
export class BrowserGamepadCapture implements GamepadState {
  private readonly _index: number;
  private readonly _source: GamepadSource;
  private _buttonsPressed = new Map<string, boolean>();
  private _axisValues = new Map<string, number>();
  private _queue: RawGamepadEvent[] = [];

  /**
   * @param index Which `navigator.getGamepads()` slot to read (LÖVE only
   *   ever reads `joysticks[1]`, see match.lua's `active_gamepad` -- one
   *   slot is all this game uses).
   * @param source Defaults to `navigator.getGamepads`; pass a fake source
   *   under test (this workspace's vitest runs under `environment: "node"`,
   *   see capture_keyboard.ts's header).
   */
  constructor(index = 0, source?: GamepadSource) {
    this._index = index;
    this._source = source ?? defaultSource;
  }

  /** Refresh this slot's snapshot and queue any button edges since the last poll. Call once per render frame. */
  poll(): void {
    const pad = this._source()[this._index];
    const nextButtons = new Map<string, boolean>();
    const nextAxes = new Map<string, number>();
    if (pad?.connected) {
      for (const [buttonIndex, name] of GAMEPAD_BUTTON_MAP) {
        nextButtons.set(name, pad.buttons[buttonIndex]?.pressed ?? false);
      }
      for (const [axisIndex, name] of GAMEPAD_AXIS_MAP) {
        nextAxes.set(name, pad.axes[axisIndex] ?? 0);
      }
      nextAxes.set("triggerleft", pad.buttons[TRIGGER_LEFT_BUTTON_INDEX]?.value ?? 0);
      nextAxes.set("triggerright", pad.buttons[TRIGGER_RIGHT_BUTTON_INDEX]?.value ?? 0);
    }
    for (const [name, pressed] of nextButtons) {
      const was = this._buttonsPressed.get(name) ?? false;
      if (pressed !== was) {
        this._queue.push({ kind: "gamepad", button: name, pressed });
      }
    }
    // A pad that disconnected mid-poll releases every button it still
    // showed held, so a consumer's held state (and any edge-tracked
    // action) never gets stuck on.
    if (!pad?.connected) {
      for (const [name, was] of this._buttonsPressed) {
        if (was) {
          this._queue.push({ kind: "gamepad", button: name, pressed: false });
        }
      }
    }
    this._buttonsPressed = nextButtons;
    this._axisValues = nextAxes;
  }

  /** Mirrors `love.Joystick:isGamepadDown`. */
  isGamepadDown(button: string): boolean {
    return this._buttonsPressed.get(button) ?? false;
  }

  /**
   * Mirrors `love.Joystick:getGamepadAxis`. `"triggerleft"`/`"triggerright"`
   * resolve from `buttons[6]/[7].value` (see this module's header); every
   * other axis name resolves from `gamepad.axes` via [`GAMEPAD_AXIS_MAP`].
   */
  getGamepadAxis(axis: string): number {
    return this._axisValues.get(axis) ?? 0;
  }

  /** Every button press/release queued since the last drain, oldest first. Clears the queue. */
  drainGamepadEvents(): readonly RawGamepadEvent[] {
    const drained = this._queue;
    this._queue = [];
    return drained;
  }
}
