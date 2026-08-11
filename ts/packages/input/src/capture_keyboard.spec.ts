import { describe, expect, it } from "vitest";
import { actions } from "./actions.ts";
import { bindings } from "./bindings.ts";
import { BrowserKeyboardCapture, KEY_CODE_MAP, translateCode } from "./capture_keyboard.ts";
import type { KeyboardEventLike, KeyboardEventTarget } from "./capture_keyboard.ts";

type Listener = (event: KeyboardEventLike) => void;

/** A fake `EventTarget` this spec drives directly, standing in for a real DOM under `environment: "node"`. */
class FakeTarget implements KeyboardEventTarget {
  private readonly _keydown = new Set<Listener>();
  private readonly _keyup = new Set<Listener>();

  addEventListener(type: "keydown" | "keyup", listener: Listener): void {
    (type === "keydown" ? this._keydown : this._keyup).add(listener);
  }

  removeEventListener(type: "keydown" | "keyup", listener: Listener): void {
    (type === "keydown" ? this._keydown : this._keyup).delete(listener);
  }

  keydown(code: string, repeat = false): void {
    for (const listener of this._keydown) {
      listener({ code, repeat });
    }
  }

  keyup(code: string): void {
    for (const listener of this._keyup) {
      listener({ code, repeat: false });
    }
  }
}

describe("translateCode", () => {
  // Every key bindings.ts's CONTROLS table names must have at least one
  // KeyboardEvent.code that reaches it -- otherwise that control is
  // silently unreachable from a real keyboard, which a passing
  // "layoutProblems" check (bindings.spec.ts) cannot see, since that check
  // only knows about LÖVE names and never looks at this translation table.
  it("translates every physical key bindings.ts's CONTROLS table names", () => {
    const reachableLoveNames = new Set(KEY_CODE_MAP.values());
    for (const entry of bindings.CONTROLS) {
      for (const key of entry.keys) {
        expect(reachableLoveNames.has(key), `no KeyboardEvent.code maps to LÖVE key "${key}"`).toBe(
          true,
        );
      }
    }
  });

  it("returns null for an unbound code", () => {
    expect(translateCode("KeyZ")).toBe(null);
  });
});

describe("BrowserKeyboardCapture", () => {
  it("tracks held state across press and release", () => {
    const target = new FakeTarget();
    const capture = new BrowserKeyboardCapture(target);
    capture.attach();

    expect(capture.isDown("w")).toBe(false);
    target.keydown("KeyW");
    expect(capture.isDown("w")).toBe(true);
    expect(capture.isDown("up", "w")).toBe(true);
    target.keyup("KeyW");
    expect(capture.isDown("w")).toBe(false);
  });

  it("ignores an unbound code entirely", () => {
    const target = new FakeTarget();
    const capture = new BrowserKeyboardCapture(target);
    capture.attach();
    target.keydown("KeyZ");
    expect(capture.drainKeyEvents()).toEqual([]);
  });

  it("drops OS key-repeat, never re-queuing a press already held", () => {
    const target = new FakeTarget();
    const capture = new BrowserKeyboardCapture(target);
    capture.attach();
    target.keydown("Space");
    target.keydown("Space", true);
    target.keydown("Space", true);
    expect(capture.drainKeyEvents()).toEqual([{ kind: "key", key: "space", pressed: true }]);
  });

  it("queues one press then one release, drained in order, and clears on drain", () => {
    const target = new FakeTarget();
    const capture = new BrowserKeyboardCapture(target);
    capture.attach();
    target.keydown("KeyJ");
    target.keyup("KeyJ");
    expect(capture.drainKeyEvents()).toEqual([
      { kind: "key", key: "j", pressed: true },
      { kind: "key", key: "j", pressed: false },
    ]);
    expect(capture.drainKeyEvents()).toEqual([]);
  });

  it("stops updating held state and the queue after detach", () => {
    const target = new FakeTarget();
    const capture = new BrowserKeyboardCapture(target);
    capture.attach();
    capture.detach();
    target.keydown("KeyW");
    expect(capture.isDown("w")).toBe(false);
    expect(capture.drainKeyEvents()).toEqual([]);
  });

  it("produces a KeyEvent that controller/actions resolve to the bound ActionEvent", () => {
    const target = new FakeTarget();
    const capture = new BrowserKeyboardCapture(target);
    capture.attach();
    target.keydown("KeyJ"); // bound to "modifier" -> action "lob"
    const [event] = capture.drainKeyEvents();
    expect(event).toBeDefined();
    const resolved = event ? actions.fromKey(event.key, event.pressed) : null;
    expect(resolved).toEqual({ kind: "action", action: "lob", pressed: true, source: "keyboard" });
  });
});
