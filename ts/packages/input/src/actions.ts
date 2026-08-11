// game/input/actions.lua — edge layer: one key or button press becomes one
// ActionEvent. Menus and the global toggles consume these. The (not-yet-
// ported) match screen additionally POLLS the same controls for held state.
//
// Both maps are derived from bindings.ts. Never add a literal key here; add
// the control there.

import { bindings } from "./bindings.ts";

export type ActionName =
  | "up"
  | "down"
  | "left"
  | "right"
  | "confirm"
  | "back"
  | "pause"
  | "pass_switch"
  | "sprint"
  | "lob"
  | "juke"
  | "equipment"
  | "toggle_fullscreen"
  | "toggle_mute";

export type InputSource = "keyboard" | "gamepad";

export interface ActionEvent {
  readonly kind: "action";
  readonly action: ActionName;
  readonly pressed?: boolean;
  readonly source?: InputSource;
}

const KEY_MAP: ReadonlyMap<string, ActionName> = bindings.keyMap();
const GAMEPAD_MAP: ReadonlyMap<string, ActionName> = bindings.gamepadMap();

function event(name: ActionName, pressed = true, source?: InputSource): ActionEvent {
  return {
    kind: "action",
    action: name,
    pressed,
    ...(source !== undefined ? { source } : {}),
  };
}

function fromKey(key: string, pressed?: boolean): ActionEvent | null {
  const action = KEY_MAP.get(key);
  return action !== undefined ? event(action, pressed, "keyboard") : null;
}

// Equipment used to ride gamepad B and shadow Back inside a match. It has
// its own button now, so Back means Back everywhere and this needs no
// context.
function fromGamepad(button: string, pressed?: boolean): ActionEvent | null {
  const action = GAMEPAD_MAP.get(button);
  return action !== undefined ? event(action, pressed, "gamepad") : null;
}

export const actions = { event, fromKey, fromGamepad };
