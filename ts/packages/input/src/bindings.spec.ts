import { describe, expect, it } from "vitest";
import { actions } from "./actions.ts";
import { bindings } from "./bindings.ts";
import type { ControlBinding, ControlId, GamepadState } from "./bindings.ts";

function control(id: ControlId): ControlBinding {
  return bindings.control(id);
}

function contains(list: readonly string[], value: string): boolean {
  return list.includes(value);
}

/** Indexes an array without `!`, failing the test loudly if the index is missing. */
function at<T>(list: readonly T[], i: number): T {
  const v = list[i];
  if (v === undefined) {
    throw new Error(`index ${i} out of range (length ${list.length})`);
  }
  return v;
}

describe("input bindings", () => {
  // Rule 4 of the layout, plus the no-double-binding check. Both are
  // derived from the table itself rather than from a list of today's
  // control names -- a hand-copied list would read `undefined` for a
  // control added later and pass, which is exactly the regression these
  // are here to stop.
  it("declares a sound layout", () => {
    expect(bindings.layoutProblems().join("; ")).toBe("");
  });

  // The guard above is only worth having if it actually fires, so drive it
  // with each way a future rebind could break the layout.
  it("rejects an edge control that only an axis could deliver", () => {
    function withExtra(entry: ControlBinding, fn: () => string): string {
      bindings.CONTROLS.push(entry);
      try {
        return fn();
      } finally {
        bindings.CONTROLS.pop();
      }
    }

    // An edge bound to nothing but a trigger: unreachable on both devices.
    const triggerOnly = withExtra(
      { id: "probe" as ControlId, action: "juke", keys: [], buttons: [], axes: ["triggerleft"], edge: true },
      () => bindings.layoutProblems().join("; "),
    );
    expect(triggerOnly.includes("cannot fire one")).toBe(true);

    // An edge with a keyboard binding but only a trigger on the pad: works
    // on a keyboard and is silently dead on a controller.
    const padDead = withExtra(
      {
        id: "probe" as ControlId,
        action: "juke",
        keys: ["z"],
        buttons: [],
        axes: ["triggerleft"],
        edge: true,
      },
      () => bindings.layoutProblems().join("; "),
    );
    expect(padDead.includes("only gamepad binding is an axis")).toBe(true);

    // And a straightforward double binding.
    const clash = withExtra(
      { id: "probe" as ControlId, action: "juke", keys: ["k"], buttons: [], axes: [], edge: true },
      () => bindings.layoutProblems().join("; "),
    );
    expect(clash.includes("bound to both")).toBe(true);
  });

  // The ergonomic core of the keyboard layout: the modifier is the right
  // index and PLAY the right middle, the most independent same-hand pair.
  // The left hand keeps WASD plus its pinky and thumb, and nothing else.
  it("keeps the modifier off the movement hand and off PLAY's finger", () => {
    expect(at(control("modifier").keys, 0)).toBe("j");
    expect(at(control("play").keys, 0)).toBe("k");
    expect(control("modifier").buttons.length).toBe(0);
    expect(contains(control("modifier").axes, "triggerright")).toBe(true);
  });

  it("keeps juke off the movement hand", () => {
    const movement = ["w", "a", "s", "d", "lshift", "rshift", "space"];
    for (const key of control("juke").keys) {
      expect(contains(movement, key)).toBe(false);
    }
    expect(contains(control("juke").buttons, "leftstick")).toBe(false);
  });

  it("derives both action maps from the one table", () => {
    expect(actions.fromKey(at(control("juke").keys, 0))?.action).toBe("juke");
    expect(actions.fromGamepad(at(control("juke").buttons, 0))?.action).toBe("juke");
    expect(actions.fromKey(at(control("sprint").keys, 0))?.action).toBe("sprint");
  });

  it("reads a trigger past its threshold as held", () => {
    let pull = 0;
    const joystick: GamepadState = {
      isGamepadDown: () => false,
      getGamepadAxis: () => pull,
    };
    pull = bindings.TRIGGER_THRESHOLD - 0.01;
    expect(bindings.gamepadDown("modifier", joystick)).toBe(false);
    pull = bindings.TRIGGER_THRESHOLD;
    expect(bindings.gamepadDown("modifier", joystick)).toBe(true);
  });

  // The help-card case now lives in @gc/screens/src/help.spec.ts, where
  // the help screen actually is, and it drives THIS module's
  // `bindings.reference("match")` directly rather than a transcription —
  // so a rebind here flows through and can fail it. Retired
  // from this file rather than duplicated, because two copies of the same
  // assertion is how one of them goes stale unnoticed.
  it("labels every reference row for both devices", () => {
    const rows = bindings.reference();
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.label.length).toBeGreaterThan(0);
      expect(row.keyboard.length).toBeGreaterThan(0);
    }
    for (const row of bindings.reference("match")) {
      expect(row.gamepad.length).toBeGreaterThan(0);
    }
  });
});
