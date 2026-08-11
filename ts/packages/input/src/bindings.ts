// game/input/bindings.lua — every physical input the game reads, declared
// exactly once.
//
// Two consumers sit on top of this table and must never grow a private
// keymap:
//   * `actions.ts` turns key and button EDGES into ActionEvents, which
//     menus and the global toggles consume.
//   * the (not-yet-ported) match screen POLLS held state every render frame
//     and derives the charge holds and release edges a MatchInput needs.
// `docs/controls.md` and the help screen both render `bindings.reference`,
// so a binding cannot drift from what the game tells the player it is.
//
// The layout obeys four ergonomic rules. They are the reason each binding
// sits where it does, and a future rebinding screen should re-check them:
//   1. The left hand never leaves WASD. Sprint is its pinky and ACTION its
//      thumb; no other hold lands on that hand.
//   2. A held modifier never shares a finger with what it modifies. On
//      keyboard that is the right index (J) against PLAY on the right
//      middle (K), the most independent same-hand pair. On gamepad it is a
//      trigger, never a face button, because one thumb cannot hold two.
//   3. Movement-adjacent taps (juke) stay off the movement hand.
//   4. No EDGE action sits on a gamepad trigger. LÖVE reports triggers as
//      axes, so `love.gamepadpressed` never fires for one; triggers carry
//      holds only.
//
// KEY-NAME DIVERGENCE (read before wiring real browser input): every string
// below is a LÖVE `KeyConstant`/`GamepadButton` name ("lshift", "kpenter",
// "dpup", ...), not a browser `KeyboardEvent.code` ("ShiftLeft", "Enter",
// ...). They are kept as the canonical internal identifiers on purpose —
// spec/game/input_bindings_spec.lua (ported as bindings.spec.ts) asserts on
// these exact strings, e.g. `control("modifier").keys[0] === "j"`. A later
// milestone needs a translation layer from `KeyboardEvent.code`/Gamepad API
// button indices into these names before real capture can be wired up; see
// this package's porting report for the full list of what that milestone
// still owes.

import { invariant } from "./assert.ts";
import type { ActionName } from "./actions.ts";

export type ControlId =
  | "move_up"
  | "move_down"
  | "move_left"
  | "move_right"
  | "action" // Contextual: shoot / punt while carrying, tackle / jockey without.
  | "play" // Contextual: pass / throw while carrying, switch player without.
  | "sprint"
  | "modifier" // Loft: chip a shot, lob a pass, request a bicycle kick.
  | "juke"
  | "equipment"
  | "confirm"
  | "back"
  | "pause"
  | "toggle_fullscreen"
  | "toggle_mute";

export interface ControlBinding {
  readonly id: ControlId;
  /** Edge action this control dispatches to screens. */
  readonly action: ActionName;
  /** LÖVE KeyConstant names. */
  readonly keys: readonly string[];
  /** LÖVE GamepadButton names, edge-capable. */
  readonly buttons: readonly string[];
  /** LÖVE GamepadAxis names read as a held trigger, hold-only. */
  readonly axes: readonly string[];
  /** A consumer reads this control's press or release, not just its held state. */
  readonly edge: boolean;
}

// A trigger is analog. Past this pull it counts as held, matching the
// deadzone the movement stick already uses.
export const TRIGGER_THRESHOLD = 0.5;

// `edge` is what makes rule 4 checkable instead of a convention. It says a
// consumer reads this control's PRESS or RELEASE, not merely whether it is
// held. `layoutProblems` then enforces that such a control always has a key
// or button to deliver that edge from, because an axis physically cannot.
//
// Mutable (not `readonly`), matching the Lua original: bindings.spec.ts's
// "rejects an edge control that only an axis could deliver" test pushes a
// synthetic entry onto this array and pops it back off to probe
// `layoutProblems`.
export const CONTROLS: ControlBinding[] = [
  // The four movement roles are polled into a vector for the match AND
  // dispatched as edges for menu navigation, so they are edge controls.
  { id: "move_up", action: "up", keys: ["up", "w"], buttons: ["dpup"], axes: [], edge: true },
  {
    id: "move_down",
    action: "down",
    keys: ["down", "s"],
    buttons: ["dpdown"],
    axes: [],
    edge: true,
  },
  {
    id: "move_left",
    action: "left",
    keys: ["left", "a"],
    buttons: ["dpleft"],
    axes: [],
    edge: true,
  },
  {
    id: "move_right",
    action: "right",
    keys: ["right", "d"],
    buttons: ["dpright"],
    axes: [],
    edge: true,
  },
  // Space and gamepad A read as `confirm` in a menu and as ACTION in a
  // match. One physical control, two contexts -- that is deliberate, not an
  // overload. The match derives its shot release from polled state, but the
  // menu confirm is a real edge.
  { id: "action", action: "confirm", keys: ["space"], buttons: ["a"], axes: [], edge: true },
  // Off the ball, PLAY switches player on the press.
  { id: "play", action: "pass_switch", keys: ["k"], buttons: ["x"], axes: [], edge: true },
  // Pure hold: nothing reads a sprint press or release.
  {
    id: "sprint",
    action: "sprint",
    keys: ["lshift", "rshift"],
    buttons: ["leftshoulder"],
    axes: [],
    edge: false,
  },
  // Also a pure hold, which is exactly why it may take the trigger that
  // rule 4 closes to edges. It was gamepad Y, which no thumb can hold while
  // pressing A or X, so the bicycle kick was unreachable on a pad.
  {
    id: "modifier",
    action: "lob",
    keys: ["j"],
    buttons: [],
    axes: ["triggerright"],
    edge: false,
  },
  // A tap, and never combined with the modifier, so it keeps a face button.
  { id: "juke", action: "juke", keys: ["l"], buttons: ["y"], axes: [], edge: true },
  {
    id: "equipment",
    action: "equipment",
    keys: ["u"],
    buttons: ["rightshoulder"],
    axes: [],
    edge: true,
  },
  {
    id: "confirm",
    action: "confirm",
    keys: ["return", "kpenter"],
    buttons: [],
    axes: [],
    edge: true,
  },
  { id: "back", action: "back", keys: ["escape"], buttons: ["b"], axes: [], edge: true },
  { id: "pause", action: "pause", keys: ["p"], buttons: ["start"], axes: [], edge: true },
  {
    id: "toggle_fullscreen",
    action: "toggle_fullscreen",
    keys: ["f11"],
    buttons: [],
    axes: [],
    edge: true,
  },
  { id: "toggle_mute", action: "toggle_mute", keys: ["m"], buttons: [], axes: [], edge: true },
];

// Built once from the initial CONTROLS, matching the Lua original — a test
// that mutates CONTROLS later does not (and should not) retarget these.
const BY_ID = new Map<ControlId, ControlBinding>();
for (const entry of CONTROLS) {
  BY_ID.set(entry.id, entry);
}

// BY_ID stays private: `control()` is the only lookup, so a bad id fails
// loud instead of returning undefined.
function control(id: ControlId): ControlBinding {
  const found = BY_ID.get(id);
  invariant(found !== undefined, `unknown control: ${String(id)}`);
  return found;
}

// The structural half of the layout rules, checked against the table
// rather than against a list of today's control names. A spec asserts
// this, but it lives here so the answer is derived from the binding a
// future editor writes and not from a second copy of the facts that editor
// would have to remember.
/** @returns problems found; empty when the layout is sound. */
function layoutProblems(): readonly string[] {
  const problems: string[] = [];
  const seenKeys = new Map<string, ControlId>();
  const seenButtons = new Map<string, ControlId>();
  const seenAxes = new Map<string, ControlId>();

  function claim(pool: Map<string, ControlId>, name: string, kind: string, entryId: ControlId): void {
    const existing = pool.get(name);
    if (existing !== undefined) {
      problems.push(`${kind} ${name} is bound to both ${existing} and ${entryId}`);
    }
    pool.set(name, entryId);
  }

  for (const entry of CONTROLS) {
    // Rule 4: LÖVE reports a trigger as an axis, so `love.gamepadpressed`
    // never fires for one. A control whose edge is read needs a key or a
    // button to deliver it.
    if (entry.edge && entry.keys.length === 0 && entry.buttons.length === 0) {
      problems.push(`${entry.id} reads an edge but is bound only to an axis, which cannot fire one`);
    }
    // A control readable on the pad at all, whose edge is read, needs that
    // edge deliverable on the pad specifically.
    if (entry.edge && entry.axes.length > 0 && entry.buttons.length === 0) {
      problems.push(`${entry.id} reads an edge but its only gamepad binding is an axis`);
    }
    for (const key of entry.keys) {
      claim(seenKeys, key, "key", entry.id);
    }
    for (const button of entry.buttons) {
      claim(seenButtons, button, "button", entry.id);
    }
    for (const axis of entry.axes) {
      claim(seenAxes, axis, "axis", entry.id);
    }
  }
  return problems;
}

function keyMap(): ReadonlyMap<string, ActionName> {
  const map = new Map<string, ActionName>();
  for (const entry of CONTROLS) {
    for (const key of entry.keys) {
      map.set(key, entry.action);
    }
  }
  return map;
}

function gamepadMap(): ReadonlyMap<string, ActionName> {
  const map = new Map<string, ActionName>();
  for (const entry of CONTROLS) {
    for (const button of entry.buttons) {
      map.set(button, entry.action);
    }
  }
  return map;
}

// Built once (matching the Lua original's KEY_CONTROLS): which role a raw
// key belongs to. The match screen uses this on the direct-mount path,
// where it sees unnormalized key events, so that path cannot drift from
// the polled one.
const KEY_CONTROLS = new Map<string, ControlId>();
for (const entry of CONTROLS) {
  for (const key of entry.keys) {
    KEY_CONTROLS.set(key, entry.id);
  }
}

function controlForKey(key: string): ControlId | null {
  return KEY_CONTROLS.get(key) ?? null;
}

/** Mirrors `love.keyboard.isDown(...)`: true when at least one of `keys` is held. */
export interface KeyboardState {
  isDown(...keys: readonly string[]): boolean;
}

/** Mirrors `love.Joystick`'s gamepad surface closely enough for `gamepadDown`. */
export interface GamepadState {
  isGamepadDown(button: string): boolean;
  getGamepadAxis(axis: string): number;
}

function keyboardDown(id: ControlId, keyboard?: KeyboardState): boolean {
  const keys = control(id).keys;
  if (keys.length === 0 || !keyboard) {
    return false;
  }
  return keyboard.isDown(...keys);
}

function gamepadDown(id: ControlId, gamepad?: GamepadState): boolean {
  if (!gamepad) {
    return false;
  }
  const entry = control(id);
  for (const button of entry.buttons) {
    if (gamepad.isGamepadDown(button)) {
      return true;
    }
  }
  for (const axis of entry.axes) {
    if (gamepad.getGamepadAxis(axis) >= TRIGGER_THRESHOLD) {
      return true;
    }
  }
  return false;
}

// The one question the match asks of a control: is a human holding it
// right now, on either device? Unlike the Lua original (`is_down(id,
// joystick)`), this also takes `keyboard` explicitly — LÖVE exposes a
// global `love.keyboard`, but there is no browser equivalent to poll
// synchronously, so keyboard state has to be injected too. See this
// package's porting report.
function isDown(id: ControlId, keyboard?: KeyboardState, gamepad?: GamepadState): boolean {
  return keyboardDown(id, keyboard) || gamepadDown(id, gamepad);
}

const KEY_LABELS: ReadonlyMap<string, string> = new Map([
  ["up", "↑"],
  ["down", "↓"],
  ["left", "←"],
  ["right", "→"],
  ["lshift", "Shift"],
  ["rshift", "Shift"],
  ["return", "Enter"],
  ["kpenter", "KP Enter"],
  ["escape", "Esc"],
  ["space", "Space"],
]);

const BUTTON_LABELS: ReadonlyMap<string, string> = new Map([
  ["a", "A"],
  ["b", "B"],
  ["x", "X"],
  ["y", "Y"],
  ["start", "Start"],
  ["back", "Back"],
  ["leftshoulder", "LB"],
  ["rightshoulder", "RB"],
  ["leftstick", "L3"],
  ["rightstick", "R3"],
  ["dpup", "D-pad ↑"],
  ["dpdown", "D-pad ↓"],
  ["dpleft", "D-pad ←"],
  ["dpright", "D-pad →"],
  ["triggerleft", "LT"],
  ["triggerright", "RT"],
]);

function labelled(name: string, labels: ReadonlyMap<string, string>): string {
  return labels.get(name) ?? name.toUpperCase();
}

function keyLabel(id: ControlId): string {
  const seen = new Set<string>();
  const parts: string[] = [];
  for (const key of control(id).keys) {
    const text = labelled(key, KEY_LABELS);
    if (!seen.has(text)) {
      seen.add(text);
      parts.push(text);
    }
  }
  return parts.join(" / ");
}

function gamepadLabel(id: ControlId): string {
  const entry = control(id);
  const parts: string[] = [];
  for (const button of entry.buttons) {
    parts.push(labelled(button, BUTTON_LABELS));
  }
  for (const axis of entry.axes) {
    parts.push(labelled(axis, BUTTON_LABELS));
  }
  return parts.join(" / ");
}

export type ControlSection = "match" | "system";

export interface ControlReferenceRow {
  /** Player-facing name of the role. */
  readonly label: string;
  /** What the role does, when the name alone is not enough. */
  readonly note?: string;
  readonly section: ControlSection;
  /** Carries the combat-prototype asterisk. */
  readonly footnote: boolean;
  readonly keyboard: string;
  readonly gamepad: string;
}

interface ControlReferenceSpec {
  readonly label: string;
  readonly note?: string;
  readonly section: ControlSection;
  readonly footnote?: boolean;
  readonly controls: readonly ControlId[];
  /** Overrides the derived text where grouping reads better. */
  readonly keyboard?: string;
  readonly gamepad?: string;
}

// Display grouping only. Every row still resolves its text through the
// bindings above, so a rebind cannot leave the printed reference stale.
const REFERENCE: readonly ControlReferenceSpec[] = [
  {
    label: "Move",
    section: "match",
    controls: ["move_up", "move_down", "move_left", "move_right"],
    keyboard: "WASD or Arrows",
    gamepad: "Left Stick or D-pad",
  },
  { label: "ACTION", note: "shoot / tackle", section: "match", controls: ["action"] },
  { label: "PLAY", note: "pass / switch", section: "match", controls: ["play"] },
  { label: "Sprint", note: "hold", section: "match", controls: ["sprint"] },
  { label: "Modifier", note: "loft / chip, hold", section: "match", controls: ["modifier"] },
  { label: "Juke", section: "match", controls: ["juke"] },
  {
    label: "Equipment",
    note: "hold / tap",
    section: "match",
    footnote: true,
    controls: ["equipment"],
  },
  { label: "Pause", section: "match", controls: ["pause", "back"] },
  { label: "Toggle mute", section: "system", controls: ["toggle_mute"] },
  { label: "Toggle fullscreen", section: "system", controls: ["toggle_fullscreen"] },
];

function joined(ids: readonly ControlId[], labelOf: (id: ControlId) => string): string {
  const parts: string[] = [];
  for (const id of ids) {
    const text = labelOf(id);
    if (text !== "") {
      parts.push(text);
    }
  }
  return parts.join(" / ");
}

// The rendered control reference, in the order a player should read it.
// Pass a section to take only the match rows (the help card) or only the
// system ones.
function reference(section?: ControlSection): readonly ControlReferenceRow[] {
  const rows: ControlReferenceRow[] = [];
  for (const spec of REFERENCE) {
    if (section === undefined || spec.section === section) {
      rows.push({
        label: spec.label,
        ...(spec.note !== undefined ? { note: spec.note } : {}),
        section: spec.section,
        footnote: spec.footnote === true,
        keyboard: spec.keyboard ?? joined(spec.controls, keyLabel),
        gamepad: spec.gamepad ?? joined(spec.controls, gamepadLabel),
      });
    }
  }
  return rows;
}

export const bindings = {
  TRIGGER_THRESHOLD,
  CONTROLS,
  control,
  layoutProblems,
  keyMap,
  gamepadMap,
  controlForKey,
  keyboardDown,
  gamepadDown,
  isDown,
  keyLabel,
  gamepadLabel,
  reference,
};
