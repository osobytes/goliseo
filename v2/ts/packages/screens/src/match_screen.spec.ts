// Ported from spec/screens/match_screen_spec.lua.
//
// The Lua original drives a live `Match.new()` through `sim.match`'s real
// physics (tackle timers, pass charge, ball flight) via stubbed
// `love.keyboard`/`love.joystick` polling and a real fixed-clock render
// loop, then asserts on real `MatchState` fields (`me.jockey_timer`,
// `m.state.ball_vz`, `m.state.players[...].pass_charge`, ...). That is no
// longer this package's job: `v2/README.md` draws the determinism line at
// `sim/**` -> Rust specifically so a TS package never has to reproduce
// physics to prove its own logic correct. `crates/gc-sim/tests` (Rust) is
// where "does holding PLAY actually charge the pass" gets proven; this file
// proves the thing that IS `@gc/screens`' job -- that `match.ts`'s
// `MatchScreen` computes the right CONTEXTUAL INPUT and drives
// `SimHostPort`/`RenderPort` correctly -- using a hand-written fake
// `SimHostPort` (`FakeSimHost` below) in place of a real wasm-compiled
// `gc-sim`, the same "small hand-written fakes" pattern
// `combat_feedback_rollback_spec.ts` already uses for `EffectsPort`/
// `AudioPort`/`ReplayPort`.
//
// Each `it` below keeps its original title (these are the ported
// assertions) but its body asserts on the TS-glue-observable analog of the
// original Lua assertion -- e.g. "K never switches while carrying" now
// checks `MatchScreen`'s buffered switch state rather than a real
// `sim.match` never issuing a switch order. Two cases remain `it.skip`
// because `SimHostPort` (the fixed five-method contract this milestone's
// game loop is built against) genuinely cannot support them yet; see each
// skip's own comment for the specific blocker.

import { describe, expect, it } from "vitest";
import { bindings, inputSample } from "@gc/input";
import type { KeyboardState } from "@gc/input";
import { MatchScreen } from "./match.ts";
import type { InputSample, RenderFrame, RenderFrameRoster, RenderPort, SimHostFactory, SimHostPort } from "./match.ts";

function nth<T>(items: readonly T[], index: number): T {
  const value = items[index];
  if (value === undefined) {
    throw new Error(`expected an element at index ${index}`);
  }
  return value;
}

function first<T>(items: readonly T[]): T {
  return nth(items, 0);
}

function fakeKeyboard(down: Record<string, boolean>): KeyboardState {
  return {
    isDown: (...keys: readonly string[]): boolean => keys.some((key) => down[key] === true),
  };
}

const noopRenderer: RenderPort = {
  draw: (): void => {},
};

/**
 * A hand-written `SimHostPort` fake. `hud.controlled_owns_ball` defaults to
 * `true` -- matching kickoff in the Lua original, where the controlled
 * player carries the ball at the whistle -- and `hud.finished` defaults to
 * `false`. Tests mutate `hud` directly to drive the scenarios the ported
 * spec titles describe.
 */
class FakeSimHost implements SimHostPort {
  readonly stepCalls: InputSample[] = [];
  disposeCalls = 0;
  readonly hud: { finished: boolean; controlled_owns_ball: boolean } = {
    finished: false,
    controlled_owns_ball: true,
  };
  private tickCount = 0;

  step(sample: InputSample): void {
    this.stepCalls.push(sample);
    this.tickCount += 1;
  }

  frame(): RenderFrame {
    return { hud: this.hud, possession: {} };
  }

  roster(): RenderFrameRoster {
    return {};
  }

  tick(): number {
    return this.tickCount;
  }

  dispose(): void {
    this.disposeCalls += 1;
  }
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

const PLAY_KEY = first(bindings.control("play").keys);
const MODIFIER_KEY = first(bindings.control("modifier").keys);

describe("match screen rematch (tier 2)", () => {
  it("R restarts a finished match with the same pre-match choices", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });
    nth(hosts, 0).hud.finished = true;

    screen.event({ kind: "key", key: "r" });

    expect(screen.finished, "a fresh match is underway").toBe(false);
    expect(hosts.length, "the same factory -- same pre-match choices -- built a new host").toBe(2);
    expect(nth(hosts, 0).disposeCalls, "the old host is released").toBe(1);
  });

  // Every key bound to CONFIRM, not just the first one. This branch used to
  // match the literal "return", so the second confirm key skipped a replay
  // but silently did not rematch.
  it("rematches on every key bound to CONFIRM", () => {
    const confirmKeys = bindings.control("confirm").keys;
    expect(confirmKeys.length, "this only proves something with more than one confirm key").toBeGreaterThan(1);
    for (const key of confirmKeys) {
      const { factory, hosts } = makeHostFactory();
      const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });
      nth(hosts, 0).hud.finished = true;
      screen.event({ kind: "key", key });
      expect(screen.finished, `${key} should trigger the rematch`).toBe(false);
    }
  });

  it("ignores the rematch keys while the match is live", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });

    screen.event({ kind: "key", key: "r" });

    expect(hosts.length, "no restart mid-match").toBe(1);
  });

  it("ignores match inputs after full time", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });
    nth(hosts, 0).hud.finished = true;

    // Lua's analogous assertion reads `not m._pass` -- a field `Match:event`
    // never actually sets true on this path (or any path: it is declared
    // and reset but never assigned in `game/screens/match.lua`). The
    // meaningful, live analog here is that a K press does not buffer a
    // switch either, and does not resurrect the match.
    screen.event({ kind: "key", key: "k" });

    expect(screen.finished, "the full-time screen does not un-finish").toBe(true);
    expect(screen.debugSwitchPending, "input does not buffer on the full-time screen").toBe(false);
  });

  it("leaves rematch ownership to the result screen in product mode", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen(
      { createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) },
      { profile: "product" },
    );
    nth(hosts, 0).hud.finished = true;

    screen.event({ kind: "key", key: "r" });
    screen.event({ kind: "action", action: "confirm" });

    expect(screen.finished).toBe(true);
    expect(hosts.length, "product mode never restarts itself").toBe(1);
  });
});

describe("match screen fixed simulation clock (tier 2)", () => {
  it("samples render frames but only steps the match at the canonical interval", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });
    const host = nth(hosts, 0);

    screen.update(1 / 120);
    expect(host.stepCalls.length, "a zero-tick render update never steps the simulator").toBe(0);
    screen.update(1 / 120);
    screen.update(1 / 30);
    expect(host.stepCalls.length).toBe(3);
    expect(screen.tick).toBe(3);
  });
});

describe("match screen contextual controls (tier 2)", () => {
  // SimHostPort's fixed five-method contract (step/frame/roster/tick/
  // dispose) has no combat toggle and no way to read per-player combat
  // state (`self._combat_state.players[...].phase` in the Lua original).
  // `sim.combat`'s wasm binding is a separate, out-of-scope surface this
  // milestone -- v2/README.md §1 scopes "the glue that makes a playable
  // browser build" (which this task IS building) as still not including
  // every sim subsystem's bridge, only the one this task's brief named.
  it.skip(
    "constructs and drives combat only behind the explicit option [SimHostPort exposes no combat toggle or combat state]",
    () => {},
  );

  it("K never switches while carrying the ball (it charges a pass)", () => {
    const { factory } = makeHostFactory(); // default host: carrying at kickoff
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });

    screen.event({ kind: "key", key: "k" });

    expect(screen.debugSwitchPending, "on the ball, K is the (polled) pass charge, not a switch").toBe(false);
  });

  it("K switches player when not carrying", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) });
    nth(hosts, 0).hud.controlled_owns_ball = false;

    screen.event({ kind: "key", key: "k" });

    expect(screen.debugSwitchPending, "K is a switch off the ball").toBe(true);
  });

  it("Space hold = jockey, release = poke (off the ball)", () => {
    const down: Record<string, boolean> = {};
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(down) });
    nth(hosts, 0).hud.controlled_owns_ball = false;

    down.space = true;
    screen.update(1 / 60); // hold one frame
    const held = screen.debugLastSample?.held ?? 0;
    expect(
      (held & inputSample.packHeld(["jockey"])) !== 0,
      "holding Space off the ball engages jockey stance",
    ).toBe(true);

    // _action_held_prev is set; release now -> the poke (the `dash` edge) fires next update.
    down.space = false;
    screen.update(1 / 60); // release
    const edges = screen.debugLastSample?.edges ?? 0;
    expect((edges & inputSample.packEdges(["dash"])) !== 0, "releasing Space off the ball fires the poke").toBe(
      true,
    );
  });

  it("Space never produces a poke while carrying (it charges the shot)", () => {
    const down: Record<string, boolean> = {};
    const { factory } = makeHostFactory(); // default host: carrying
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(down) });

    down.space = true;
    screen.update(1 / 60); // hold while carrying
    const held = screen.debugLastSample?.held ?? 0;
    expect((held & inputSample.packHeld(["shoot"])) !== 0, "Space while carrying charges the shot").toBe(true);
    expect((held & inputSample.packHeld(["jockey"])) !== 0, "not jockey, while carrying").toBe(false);

    down.space = false;
    screen.update(1 / 60); // release while still carrying
    const edges = screen.debugLastSample?.edges ?? 0;
    expect((edges & inputSample.packEdges(["dash"])) !== 0, "Space release while carrying does not fire a poke").toBe(
      false,
    );
  });
});

describe("match screen lob latch (tier 2)", () => {
  it("MODIFIER held during a charged pass lofts it even if it lifts early", () => {
    const down: Record<string, boolean> = {};
    const { factory } = makeHostFactory(); // default host: carrying at kickoff
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(down) });

    down[PLAY_KEY] = true;
    down[MODIFIER_KEY] = true;
    screen.update(1 / 60); // charging the pass with the modifier held
    screen.update(1 / 60);
    down[MODIFIER_KEY] = false;
    screen.update(1 / 60); // modifier released a beat before PLAY...
    down[PLAY_KEY] = false;
    screen.update(1 / 60); // ...PLAY release fires the pass

    const sample = screen.debugLastSample;
    const edges = sample?.edges ?? 0;
    const held = sample?.held ?? 0;
    expect((edges & inputSample.packEdges(["pass"])) !== 0, "the pass released").toBe(true);
    expect((held & inputSample.packHeld(["lob"])) !== 0, "and it was lofted: the latch held the modifier for us").toBe(
      true,
    );
  });

  it("holding PLAY charges the pass range for an outfielder", () => {
    const down: Record<string, boolean> = {};
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard(down) });

    down[PLAY_KEY] = true;
    for (let i = 0; i < 20; i += 1) {
      // a third of a second of holding PLAY
      screen.update(1 / 60);
    }

    // Real pass-charge math is `sim.match`'s job (Rust, proven by
    // `crates/gc-sim/tests`); this module's job is sustaining the "pass
    // held" signal across every one of those ticks so the sim CAN charge
    // it, which is what this asserts.
    const host = nth(hosts, 0);
    expect(host.stepCalls.length).toBe(20);
    for (const sample of host.stepCalls) {
      expect((sample.held & inputSample.packHeld(["pass"])) !== 0, "the pass range charged up").toBe(true);
      expect((sample.edges & inputSample.packEdges(["pass"])) !== 0, "not fired yet -- still held").toBe(false);
    }
  });
});

describe("match screen goal replay (tier 2)", () => {
  // `SimHostPort` has no replay/snapshot-recording capability (no analog of
  // `game/render/replay.lua`'s recorded-footage buffer -- `step`/`frame`/
  // `roster`/`tick`/`dispose` is the whole contract), and `@gc/render`
  // (which owns the real `replay.ts`) is still not a declared dependency of
  // `@gc/screens` -- both blockers the original skip already named, and
  // both still accurate after this task.
  it.skip(
    "a goal freezes the sim into a slow-mo replay; skipping resumes [SimHostPort has no replay/snapshot capability; @gc/render not a declared dependency]",
    () => {},
  );
});
