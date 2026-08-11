import { describe, expect, it } from "vitest";
import { bindings } from "./bindings.ts";
import { BrowserGamepadCapture, GAMEPAD_AXIS_MAP, GAMEPAD_BUTTON_MAP } from "./capture_gamepad.ts";
import type { GamepadLike, GamepadSource } from "./capture_gamepad.ts";

function pad(overrides: {
  connected?: boolean;
  buttons?: Readonly<Record<number, { pressed?: boolean; value?: number }>>;
  axes?: readonly number[];
}): GamepadLike {
  const buttons: { pressed: boolean; value: number }[] = [];
  const maxIndex = Math.max(0, ...Object.keys(overrides.buttons ?? {}).map(Number));
  for (let i = 0; i <= Math.max(maxIndex, 7); i++) {
    const entry = overrides.buttons?.[i];
    buttons.push({ pressed: entry?.pressed ?? false, value: entry?.value ?? 0 });
  }
  return {
    connected: overrides.connected ?? true,
    buttons,
    axes: overrides.axes ?? [0, 0, 0, 0],
  };
}

function sourceOf(current: () => GamepadLike | null): GamepadSource {
  return () => [current()];
}

describe("GAMEPAD_BUTTON_MAP / GAMEPAD_AXIS_MAP", () => {
  it("names every gamepad button/axis bindings.ts's CONTROLS table uses", () => {
    const reachableButtons = new Set(GAMEPAD_BUTTON_MAP.values());
    reachableButtons.add("triggerleft");
    reachableButtons.add("triggerright"); // resolved specially, see capture_gamepad.ts's header
    const reachableAxes = new Set(GAMEPAD_AXIS_MAP.values());
    reachableAxes.add("triggerleft");
    reachableAxes.add("triggerright");
    for (const entry of bindings.CONTROLS) {
      for (const button of entry.buttons) {
        expect(reachableButtons.has(button), `no gamepad button index maps to "${button}"`).toBe(
          true,
        );
      }
      for (const axis of entry.axes) {
        expect(reachableAxes.has(axis), `no gamepad axis maps to "${axis}"`).toBe(true);
      }
    }
  });
});

describe("BrowserGamepadCapture", () => {
  it("reads held buttons and raw axes after a poll", () => {
    const current: GamepadLike | null = pad({
      buttons: { 0: { pressed: true } },
      axes: [0.6, -0.3, 0, 0],
    });
    const capture = new BrowserGamepadCapture(
      0,
      sourceOf(() => current),
    );
    capture.poll();
    expect(capture.isGamepadDown("a")).toBe(true);
    expect(capture.isGamepadDown("b")).toBe(false);
    expect(capture.getGamepadAxis("leftx")).toBeCloseTo(0.6);
    expect(capture.getGamepadAxis("lefty")).toBeCloseTo(-0.3);
  });

  it("reads a trigger's analog value off buttons[6]/[7], not the axes array", () => {
    const current: GamepadLike | null = pad({ buttons: { 7: { pressed: true, value: 0.9 } } });
    const capture = new BrowserGamepadCapture(
      0,
      sourceOf(() => current),
    );
    capture.poll();
    expect(capture.getGamepadAxis("triggerright")).toBeCloseTo(0.9);
    expect(capture.getGamepadAxis("triggerleft")).toBe(0);
  });

  it("queues a press edge and a release edge across two polls", () => {
    let current: GamepadLike | null = pad({});
    const capture = new BrowserGamepadCapture(
      0,
      sourceOf(() => current!),
    );
    capture.poll();
    expect(capture.drainGamepadEvents()).toEqual([]);

    current = pad({ buttons: { 3: { pressed: true } } }); // y -> bound to "juke"
    capture.poll();
    expect(capture.drainGamepadEvents()).toEqual([{ kind: "gamepad", button: "y", pressed: true }]);

    current = pad({});
    capture.poll();
    expect(capture.drainGamepadEvents()).toEqual([
      { kind: "gamepad", button: "y", pressed: false },
    ]);
  });

  it("releases every held button when the pad disconnects mid-session", () => {
    let current: GamepadLike | null = pad({
      buttons: { 0: { pressed: true }, 1: { pressed: true } },
    });
    const capture = new BrowserGamepadCapture(
      0,
      sourceOf(() => current!),
    );
    capture.poll();
    capture.drainGamepadEvents();

    current = null;
    capture.poll();
    const events = capture.drainGamepadEvents();
    expect(events).toContainEqual({ kind: "gamepad", button: "a", pressed: false });
    expect(events).toContainEqual({ kind: "gamepad", button: "b", pressed: false });
    expect(capture.isGamepadDown("a")).toBe(false);
  });

  it("reads nothing from an empty gamepad slot without polling first", () => {
    const capture = new BrowserGamepadCapture(0, () => []);
    expect(capture.isGamepadDown("a")).toBe(false);
    expect(capture.getGamepadAxis("leftx")).toBe(0);
  });
});
