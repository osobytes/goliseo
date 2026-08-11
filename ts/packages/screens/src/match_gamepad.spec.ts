// Ported from spec/screens/match_gamepad_spec.lua.
//
// Same reframing `match_screen.spec.ts`'s header already establishes for
// this milestone's `MatchScreen`: `SimHostPort` (step/planTicks/
// cancelPlannedTicks/frame/roster/tick/dispose) has no way to read back
// real per-player kinematics or shot-charge accumulation (that lives in
// `sim.match`, Rust, proven by `crates/gc-sim/tests`), so a case that
// depended on real physics is ported against the TS-glue-observable
// analog -- the `InputSample` this screen actually built and would have
// sent to `SimHostPort.step` -- using the same hand-written `FakeSimHost`/
// fake `GamepadState` pattern `match_screen.spec.ts` already uses.

import { describe, expect, it } from "vitest";
import { bindings, inputSample } from "@gc/input";
import type { GamepadState, KeyboardState } from "@gc/input";
import { MatchScreen } from "./match.ts";
import type { InputSample, RenderFrame, RenderFrameRoster, RenderPort, SimHostFactory, SimHostPort } from "./match.ts";

function first<T>(items: readonly T[]): T {
  const value = items[0];
  if (value === undefined) {
    throw new Error("expected a non-empty array");
  }
  return value;
}

function fakeKeyboard(down: Record<string, boolean> = {}): KeyboardState {
  return {
    isDown: (...keys: readonly string[]): boolean => keys.some((key) => down[key] === true),
  };
}

/** Mirrors the ported spec's `with_pad` fake `love.joystick` -- `held` names the gamepad buttons that are down, `pull` names the trigger/stick axes. */
function fakeGamepad(held: Record<string, boolean> = {}, pull: Record<string, number> = {}): GamepadState {
  return {
    isGamepadDown: (button: string): boolean => held[button] === true,
    getGamepadAxis: (axis: string): number => pull[axis] ?? 0,
  };
}

const noopRenderer: RenderPort = { draw: (): void => {} };

/** Same `FakeSimHost` role `match_screen.spec.ts` documents -- a hand-written `SimHostPort` standing in for a real wasm-compiled `gc-sim`. `hud.controlled_owns_ball` defaults to `true` (carrying at kickoff). */
class FakeSimHost implements SimHostPort {
  readonly stepCalls: InputSample[] = [];
  readonly hud: { finished: boolean; controlled_owns_ball: boolean; home_score: number; away_score: number; time_left: number } = {
    finished: false,
    controlled_owns_ball: true,
    home_score: 0,
    away_score: 0,
    time_left: 300,
  };

  planTicks(_dt: number): number {
    return 1;
  }

  step(sample: InputSample): void {
    this.stepCalls.push(sample);
  }

  cancelPlannedTicks(): void {}

  frame(): RenderFrame {
    return { hud: this.hud, possession: {} };
  }

  roster(): RenderFrameRoster {
    return {};
  }

  tick(): number {
    return this.stepCalls.length;
  }

  dispose(): void {}
}

function makeHostFactory(): { readonly factory: SimHostFactory; readonly hosts: FakeSimHost[] } {
  const hosts: FakeSimHost[] = [];
  const factory: SimHostFactory = (): SimHostPort => {
    const host = new FakeSimHost();
    hosts.push(host);
    return host;
  };
  return { factory, hosts };
}

describe("match screen gamepad input", () => {
  it("polls the left stick and contextual action button", () => {
    const { factory, hosts } = makeHostFactory(); // default host: carrying at kickoff
    const gamepad = fakeGamepad({ a: true }, { leftx: 1 });
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(), gamepad });

    screen.update(1 / 60);

    const sample = hosts[0]?.stepCalls[0];
    expect(sample, "the render update stepped the fake host").not.toBeUndefined();
    expect((sample?.move_x ?? 0) > 0, "left stick drives the controlled player").toBe(true);
    expect(
      ((sample?.held ?? 0) & inputSample.packHeld(["shoot"])) !== 0,
      "A holds the contextual shot charge",
    ).toBe(true);
  });

  it("accepts abstract gamepad actions for off-ball switching", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard() });
    hosts[0]!.hud.controlled_owns_ball = false;

    screen.event({ kind: "action", action: "pass_switch", pressed: true });

    expect(screen.debugSwitchPending).toBe(true);
  });

  // The whole point of moving the modifier onto a trigger. This chord used to
  // be gamepad Y held with A, which one thumb cannot do, so the bicycle kick
  // was documented, simulated, and unreachable on a controller. Assert the
  // request survives the full render-frame path into the sampled input
  // rather than trusting that the two halves compose.
  it("requests an acrobatic strike from MODIFIER + ACTION off the ball", () => {
    const actionButton = first(bindings.control("action").buttons);
    const modifierAxis = first(bindings.control("modifier").axes);

    // ACTION alone off the ball is an ordinary first-time strike...
    {
      const { factory, hosts } = makeHostFactory();
      const gamepad = fakeGamepad({ [actionButton]: true });
      const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(), gamepad });
      hosts[0]!.hud.controlled_owns_ball = false;
      screen.update(1 / 60);
      const held = hosts[0]?.stepCalls[0]?.held ?? 0;
      expect((held & inputSample.packHeld(["aerial_strike"])) !== 0, "ACTION off the ball should request a strike").toBe(
        true,
      );
      expect(
        (held & inputSample.packHeld(["aerial_acrobatic"])) !== 0,
        "without the modifier it is not acrobatic",
      ).toBe(false);
    }

    // ...and the same press with the trigger pulled is the acrobatic one.
    {
      const { factory, hosts } = makeHostFactory();
      const gamepad = fakeGamepad({ [actionButton]: true }, { [modifierAxis]: bindings.TRIGGER_THRESHOLD });
      const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(), gamepad });
      hosts[0]!.hud.controlled_owns_ball = false;
      screen.update(1 / 60);
      const held = hosts[0]?.stepCalls[0]?.held ?? 0;
      expect(
        (held & inputSample.packHeld(["aerial_acrobatic"])) !== 0,
        "MODIFIER + ACTION should request a bicycle",
      ).toBe(true);
      expect((held & inputSample.packHeld(["jockey"])) !== 0, "and it is still an off-ball hold").toBe(true);
    }

    // A trigger short of the threshold must not arm it, or the chord would
    // fire on an idle stick's resting noise.
    {
      const { factory, hosts } = makeHostFactory();
      const gamepad = fakeGamepad({ [actionButton]: true }, { [modifierAxis]: bindings.TRIGGER_THRESHOLD - 0.01 });
      const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(), gamepad });
      hosts[0]!.hud.controlled_owns_ball = false;
      screen.update(1 / 60);
      const held = hosts[0]?.stepCalls[0]?.held ?? 0;
      expect(
        (held & inputSample.packHeld(["aerial_acrobatic"])) !== 0,
        "a trigger below the threshold must not request a bicycle",
      ).toBe(false);
    }
  });
});
