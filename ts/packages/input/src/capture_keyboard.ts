// Browser keyboard capture. bindings.ts's header comment flags this exact
// gap: every string in `CONTROLS` is drawn from this project's canonical
// control vocabulary ("lshift", "kpenter", "dpup", ...), not a browser
// `KeyboardEvent.code` ("ShiftLeft", "Enter", ...), and something has to
// translate between them before real capture works. This file is that
// translation layer, plus the capture itself.
//
// Split into a pure translation table/function (testable with plain
// strings, no DOM) and a small stateful class that listens for real
// `keydown`/`keyup`. The class takes its event target as a constructor
// parameter rather than reaching for the global `window`/`document` --
// this workspace's vitest config runs specs under `environment: "node"`
// (ts/vitest.config.ts), so there is no real DOM to reach for during a
// test, and an injected fake target keeps this file's own tests honest
// without adding a jsdom/happy-dom dependency (this package's task brief
// forbids editing package.json / installing anything mid-milestone).
//
// KeyboardState is satisfied directly (`bindings.ts`'s existing interface,
// already used by `bindings.isDown`/`control_down`-style polling) so a
// consumer needs nothing new to poll held state. Press/release EDGES are
// queued separately as `KeyEvent`s and drained on demand, for a consumer
// that wants to run them through `controller.normalize` (e.g. to derive
// `ActionEvent`s for menus, or the one-shot edges `capture_frame.ts` reads
// off `juke`).

import type { KeyboardState } from "./bindings.ts";
import type { KeyEvent } from "./controller.ts";

/**
 * `KeyboardEvent`'s relevant surface, kept minimal and DOM-independent so
 * this file typechecks and tests without `lib.dom` semantics attached to a
 * runtime that does not have a `window`.
 */
export interface KeyboardEventLike {
  readonly code: string;
  readonly repeat: boolean;
}

/** The subset of `EventTarget` this file needs to listen for key events. */
export interface KeyboardEventTarget {
  addEventListener(type: "keydown" | "keyup", listener: (event: KeyboardEventLike) => void): void;
  removeEventListener(
    type: "keydown" | "keyup",
    listener: (event: KeyboardEventLike) => void,
  ): void;
}

// Every `KeyboardEvent.code` bindings.ts's CONTROLS table names, mapped to
// its canonical control-vocabulary name. `code` (the physical-key
// identity) is used rather than `key` (the character produced) so the
// WASD cluster stays put under non-QWERTY layouts, matching that
// vocabulary's own scancode-based semantics closely enough for this
// game's bindings.
//
// Deliberately closed: a `code` with no entry here produces no `KeyEvent`
// at all (see `translateCode`) rather than a guessed/lowercased passthrough
// -- an untranslated code reaching `KEY_MAP`/`GAMEPAD_MAP` would silently
// look up nothing and do nothing, which is a worse failure to debug than a
// key that is visibly absent from this table.
export const KEY_CODE_MAP: ReadonlyMap<string, string> = new Map([
  ["ArrowUp", "up"],
  ["ArrowDown", "down"],
  ["ArrowLeft", "left"],
  ["ArrowRight", "right"],
  ["KeyW", "w"],
  ["KeyA", "a"],
  ["KeyS", "s"],
  ["KeyD", "d"],
  ["Space", "space"],
  ["KeyK", "k"],
  ["KeyJ", "j"],
  ["KeyL", "l"],
  ["KeyU", "u"],
  ["ShiftLeft", "lshift"],
  ["ShiftRight", "rshift"],
  ["Enter", "return"],
  ["NumpadEnter", "kpenter"],
  ["Escape", "escape"],
  ["KeyP", "p"],
  ["F11", "f11"],
  ["KeyM", "m"],
  // Not in CONTROLS today, but named literally by the match screen's
  // dev-harness keys (`r`/`b`/`f1`) -- kept here so a playtest-profile
  // capture layer can recognize them too, rather than every future
  // consumer growing its own private code->name entry for exactly these
  // three.
  ["KeyR", "r"],
  ["KeyB", "b"],
  ["F1", "f1"],
]);

/** Translate a browser `KeyboardEvent.code` into its canonical control-vocabulary name, or `null` if this game does not bind it. */
export function translateCode(code: string): string | null {
  return KEY_CODE_MAP.get(code) ?? null;
}

/**
 * Listens for real keyboard events and tracks held keys (by canonical
 * control-vocabulary name) for [`KeyboardState`], queuing a [`KeyEvent`]
 * per translated press/release for a consumer to drain.
 *
 * Repeats (OS key-repeat while a key stays down) and already-known
 * up/down transitions are filtered here, so a drained press/release
 * sequence always alternates -- a consumer never sees two presses in a
 * row for the same key without a release between them.
 */
export class BrowserKeyboardCapture implements KeyboardState {
  private readonly _held = new Set<string>();
  private _queue: KeyEvent[] = [];
  private readonly _target: KeyboardEventTarget | undefined;
  private readonly _onKeyDown = (event: KeyboardEventLike): void => {
    if (event.repeat) {
      return;
    }
    const name = translateCode(event.code);
    if (name === null || this._held.has(name)) {
      return;
    }
    this._held.add(name);
    this._queue.push({ kind: "key", key: name, pressed: true });
  };
  private readonly _onKeyUp = (event: KeyboardEventLike): void => {
    const name = translateCode(event.code);
    if (name === null || !this._held.has(name)) {
      return;
    }
    this._held.delete(name);
    this._queue.push({ kind: "key", key: name, pressed: false });
  };

  /** @param target Defaults to the global `window` when present (real browser use); pass a fake target under test. */
  constructor(target?: KeyboardEventTarget) {
    this._target = target ?? (typeof window === "undefined" ? undefined : window);
  }

  /** Start listening. Idempotent-unsafe by design (matching `addEventListener`): call [`detach`] before a second [`attach`]. */
  attach(): void {
    this._target?.addEventListener("keydown", this._onKeyDown);
    this._target?.addEventListener("keyup", this._onKeyUp);
  }

  /** Stop listening. Held state and any undrained queue are left as-is. */
  detach(): void {
    this._target?.removeEventListener("keydown", this._onKeyDown);
    this._target?.removeEventListener("keyup", this._onKeyUp);
  }

  /** True when at least one of `keys` is currently held. */
  isDown(...keys: readonly string[]): boolean {
    return keys.some((key) => this._held.has(key));
  }

  /** Every press/release queued since the last drain, oldest first. Clears the queue. */
  drainKeyEvents(): readonly KeyEvent[] {
    const drained = this._queue;
    this._queue = [];
    return drained;
  }
}
