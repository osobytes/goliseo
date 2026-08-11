import { describe, expect, it } from "vitest";
import { BrowserKeyboardCapture } from "./capture_keyboard.ts";
import type { KeyboardEventLike, KeyboardEventTarget } from "./capture_keyboard.ts";
import { BrowserGamepadCapture } from "./capture_gamepad.ts";
import type { GamepadLike, GamepadSource } from "./capture_gamepad.ts";
import { InputSampleCapture } from "./capture_frame.ts";

type Listener = (event: KeyboardEventLike) => void;

class FakeTarget implements KeyboardEventTarget {
  private readonly _keydown = new Set<Listener>();
  private readonly _keyup = new Set<Listener>();

  addEventListener(type: "keydown" | "keyup", listener: Listener): void {
    (type === "keydown" ? this._keydown : this._keyup).add(listener);
  }

  removeEventListener(type: "keydown" | "keyup", listener: Listener): void {
    (type === "keydown" ? this._keydown : this._keyup).delete(listener);
  }

  keydown(code: string): void {
    for (const listener of this._keydown) {
      listener({ code, repeat: false });
    }
  }

  keyup(code: string): void {
    for (const listener of this._keyup) {
      listener({ code, repeat: false });
    }
  }
}

function emptyPad(): GamepadLike {
  return { connected: true, buttons: new Array(16).fill({ pressed: false, value: 0 }), axes: [0, 0, 0, 0] };
}

function newRig(): { keyboard: BrowserKeyboardCapture; target: FakeTarget; gamepad: BrowserGamepadCapture; capture: InputSampleCapture } {
  const target = new FakeTarget();
  const keyboard = new BrowserKeyboardCapture(target);
  keyboard.attach();
  const source: GamepadSource = () => [emptyPad()];
  const gamepad = new BrowserGamepadCapture(0, source);
  const capture = new InputSampleCapture(
    keyboard,
    gamepad,
    () => keyboard.drainKeyEvents(),
    () => gamepad.drainGamepadEvents(),
  );
  return { keyboard, target, gamepad, capture };
}

describe("InputSampleCapture", () => {
  it("quantizes WASD into move_x/move_y", () => {
    const { target, capture } = newRig();
    target.keydown("KeyW");
    const sample = capture.sample();
    expect(sample.move_y).toBe(-127);
    expect(sample.move_x).toBe(0);
  });

  it("reads sprint as a continuous held bit", () => {
    const { target, capture } = newRig();
    target.keydown("ShiftLeft");
    expect(capture.sample().held & 4).toBe(4); // HELD_SPRINT
    expect(capture.sample().held & 4).toBe(4); // still held next frame
    target.keyup("ShiftLeft");
    expect(capture.sample().held & 4).toBe(0);
  });

  it("fires dodge as a one-shot edge on JUKE press, not on every subsequent frame", () => {
    const { target, capture } = newRig();
    target.keydown("KeyL"); // bound to juke
    expect(capture.sample().edges & 16).toBe(16); // EDGE_DODGE
    expect(capture.sample().edges & 16).toBe(0); // consumed
    target.keyup("KeyL");
    expect(capture.sample().edges & 16).toBe(0);
  });

  it("derives equipment_pressed/held/released from poll-to-poll diffing, matching match.lua", () => {
    const { target, capture } = newRig();
    let sample = capture.sample();
    expect(sample.held & 128).toBe(0); // HELD_EQUIPMENT
    expect(sample.edges & 32).toBe(0); // EDGE_EQUIPMENT_PRESSED
    expect(sample.edges & 64).toBe(0); // EDGE_EQUIPMENT_RELEASED

    target.keydown("KeyU");
    sample = capture.sample();
    expect(sample.held & 128).toBe(128);
    expect(sample.edges & 32).toBe(32);
    expect(sample.edges & 64).toBe(0);

    sample = capture.sample(); // still held, no repeated press edge
    expect(sample.held & 128).toBe(128);
    expect(sample.edges & 32).toBe(0);

    target.keyup("KeyU");
    sample = capture.sample();
    expect(sample.held & 128).toBe(0);
    expect(sample.edges & 64).toBe(64);
  });

  it("merges contextual fields for the bits @gc/input cannot compute on its own", () => {
    const { capture } = newRig();
    const sample = capture.sample({
      shootHeld: true,
      pass: true,
      aerialAcrobatic: true,
    });
    expect(sample.held & 1).toBe(1); // HELD_SHOOT
    expect(sample.edges & 2).toBe(2); // EDGE_PASS
    expect(sample.held & 64).toBe(64); // HELD_AERIAL_ACROBATIC
  });

  it("defaults every contextual field to neutral, matching neutralSample()", () => {
    const { capture } = newRig();
    const sample = capture.sample();
    expect(sample).toEqual({ move_x: 0, move_y: 0, held: 0, edges: 0 });
  });
});
