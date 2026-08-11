// Verifies match.ts's MatchScreen against a hand-written fake SimHostPort.
//
// The determinism boundary is `sim/**` -> Rust specifically, so this
// package never has to reproduce physics to prove its own
// logic correct: `crates/gc-sim/tests` (Rust) is where "does holding PLAY
// actually charge the pass" gets proven. This file proves the thing that IS
// `@gc/screens`' job -- that `match.ts`'s `MatchScreen` computes the right
// CONTEXTUAL INPUT and drives `SimHostPort`/`RenderPort` correctly -- using
// a hand-written fake `SimHostPort` (`FakeSimHost` below) in place of a real
// wasm-compiled `gc-sim`, the same "small hand-written fakes" pattern
// `combat_feedback_rollback.spec.ts` already uses for `EffectsPort`/
// `AudioPort`/`ReplayPort`.
//
// Each `it` below asserts on the TS-observable analog of the underlying
// behavior -- e.g. "K never switches while carrying" checks `MatchScreen`'s
// buffered switch state rather than a real `sim.match` never issuing a
// switch order. The combat construction case
// ("constructs and drives combat only behind the explicit option") used to
// stay `it.skip` here because `SimHostPort` (the fixed contract this
// milestone's game loop is built against -- `step`/`planTicks`/
// `cancelPlannedTicks`/`frame`/`roster`/`tick`/`dispose`) had no combat
// surface at all; `crates/gc-wasm/src/session.rs`'s `Session::new`/`step`
// gained one this wave (see that module's doc), and `MatchScreenOptions.combat_enabled`
// now has somewhere real to reach -- see that case's own comment for
// exactly what is, and is not, provable without a wasm-bound getter for
// combat presence or per-tick combat state.
//
// "match screen goal replay"'s case USED to be skipped for the same
// reason -- `SimHostPort.frame()` returns a presentation-derived
// `RenderFrame` with no raw `MatchState` -- but `MatchScreenPorts.matchState`
// (match.ts's "THE GAME LOOP" section) now closes that gap: a caller
// injects raw state directly, sidestepping `SimHostPort` entirely (a FIXED
// contract shared with `@gc/app`'s `sim_host.ts`, so it could not be widened
// here -- see match.ts's own doc). That case now drives `@gc/render`'s REAL
// `replay` module (already pure/real, no wasm involved -- the same
// "TS-observable analog" pattern `match_rollback_lab.spec.ts` already
// uses for `correctionSmoothing`/`viewState`) against a fake `matchState`
// source, the same role `FakeSimHost` already plays for `SimHostPort`.

import { afterEach, describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import { bindings, inputSample } from "@gc/input";
import type { KeyboardState } from "@gc/input";
import { cameraFollow, player3dVariantBuildCount, releaseFollow, replay } from "@gc/render";
import type { replayTypes } from "@gc/render";
import { MatchScreen, MatchScreenAsRealMatchScreen } from "./match.ts";
import type {
  InputSample,
  RealMatchRenderFrameEvents,
  RenderFrame,
  RenderFrameRoster,
  RenderPort,
  SimHostFactory,
  SimHostPort,
} from "./match.ts";

// A test-only stand-in for the wasm session's `gc_sim::fixed_clock`-backed
// `FixedClock` (`crates/gc-wasm/src/session.rs`, bound through `@gc/wasm`).
// The production accumulator/catch-up/drop algorithm now has exactly one
// implementation, in Rust (ARCHITECTURE.md §1.1) -- `FakeSimHost.planTicks`
// below mirrors it purely so this suite can drive `MatchScreen`'s CALLING
// contract (does it call `step` the right number of times, with the right
// sample) without a real wasm build, the same role every other `FakeSimHost`
// method already plays for `SimSession`.
const FAKE_TICK_SECONDS = 1 / 60;
const FAKE_MAX_TICKS_PER_UPDATE = 8;
const FAKE_CLOCK_EPSILON = FAKE_TICK_SECONDS * 1e-9;

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
 * `true` -- matching kickoff, where the controlled player carries the ball
 * at the whistle -- and `hud.finished` defaults to `false`. Tests mutate
 * `hud` directly to drive each scenario below.
 */
class FakeSimHost implements SimHostPort {
  readonly stepCalls: InputSample[] = [];
  disposeCalls = 0;
  readonly hud: {
    finished: boolean;
    controlled_owns_ball: boolean;
    home_score: number;
    away_score: number;
    time_left: number;
  } = {
    finished: false,
    controlled_owns_ball: true,
    home_score: 0,
    away_score: 0,
    time_left: 300,
  };
  /** One tick's worth of `RenderFrame.events`, returned by `frame()` and
   * cleared by the next `step` -- mirrors the real host, whose `frame()`
   * only ever reports the LAST stepped tick's events. Left empty unless a
   * test arms it via `emitOnNextStep`. */
  private pendingEvents: RealMatchRenderFrameEvents | undefined;
  private armedEvents: RealMatchRenderFrameEvents | undefined;
  private armedHomeGoal = false;
  private tickCount = 0;

  /** Make the NEXT `step` produce `events` on the following `frame()` read. */
  emitOnNextStep(events: RealMatchRenderFrameEvents): void {
    this.armedEvents = events;
  }

  /** Make the NEXT `step` score for the home side -- the score EDGE
   * `finishBaseUpdate` treats as a scene discontinuity. */
  scoreOnNextStep(): void {
    this.armedHomeGoal = true;
  }
  private clockAccumulator = 0;

  planTicks(dt: number): number {
    this.clockAccumulator += dt;
    let ticks = 0;
    while (
      this.clockAccumulator + FAKE_CLOCK_EPSILON >= FAKE_TICK_SECONDS &&
      ticks < FAKE_MAX_TICKS_PER_UPDATE
    ) {
      this.clockAccumulator -= FAKE_TICK_SECONDS;
      if (this.clockAccumulator < 0) {
        this.clockAccumulator = 0;
      }
      ticks += 1;
    }
    if (this.clockAccumulator + FAKE_CLOCK_EPSILON >= FAKE_TICK_SECONDS) {
      const dropped = Math.floor((this.clockAccumulator + FAKE_CLOCK_EPSILON) / FAKE_TICK_SECONDS);
      this.clockAccumulator -= dropped * FAKE_TICK_SECONDS;
      if (this.clockAccumulator < 0) {
        this.clockAccumulator = 0;
      }
    }
    return ticks;
  }

  step(sample: InputSample): void {
    this.stepCalls.push(sample);
    this.pendingEvents = this.armedEvents;
    this.armedEvents = undefined;
    if (this.armedHomeGoal) {
      this.armedHomeGoal = false;
      this.hud.home_score += 1;
    }
    this.tickCount += 1;
    // A goal replay's "the sim is frozen during the replay" assertion needs
    // SOMETHING to observably advance only while `step` is actually called
    // -- `time_left` is the cheapest live field this fake already carries.
    this.hud.time_left = Math.max(0, this.hud.time_left - FAKE_TICK_SECONDS);
  }

  cancelPlannedTicks(): void {
    this.clockAccumulator = 0;
  }

  frame(): RenderFrame {
    return {
      hud: this.hud,
      possession: {},
      ...(this.pendingEvents !== undefined ? { events: this.pendingEvents } : {}),
    };
  }

  roster(): RenderFrameRoster {
    return { ids: ["keeper", "striker", "playmaker"] };
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

// --- a minimal raw-MatchState fixture, for MatchScreenPorts.matchState -----
// (goal-replay tests only; every field this module never reads is a fixed,
// static placeholder -- unlike replay.spec.ts's own fixture, which needs
// varied per-player values to prove capture/celebration/playback carry them).

function fixtureMatchPlayer(id: string, team: "home" | "away", pos: Vec2): replayTypes.MatchPlayer {
  return {
    id,
    team,
    pos,
    run_vel: new Vec2(0, 0),
    facing: new Vec2(team === "home" ? 1 : -1, 0),
    radius: 10,
    is_keeper: false,
    keeper_state: "base",
    keeper_set: 0,
    slide_timer: 0,
    tackle_timer: 0,
    stun_timer: 0,
    settle_timer: 0,
    sprinting: false,
    outfield_decision: {
      version: 1,
      generation: 0,
      rng_state: 1,
      remaining: 0,
      context: "offball",
      intent: "none",
    },
    dive_timer: 0,
    dive_dir: new Vec2(0, 0),
    keeper_get_up_timer: 0,
    grab_timer: 0,
    throw_timer: 0,
    windup_timer: 0,
    aerial_timer: 0,
    aerial_jump: 0,
    sprint_meter: 1,
    jockey_timer: 0,
  };
}

/**
 * A `MatchScreenPorts.matchState` fake, for goal-replay tests: one drifting
 * player so the replay buffer holds observable motion, `time_left`/`score`
 * mirrored live off `getHost()`'s current `hud`. Takes a THUNK, not a
 * `FakeSimHost` directly -- `MatchScreenPorts` is built as an object literal
 * before `new MatchScreen(...)` has run its constructor (which is what
 * actually populates `makeHostFactory`'s `hosts` array), so a captured host
 * reference would still be `undefined` at that point; a thunk defers the
 * lookup to each call, by which time the host exists.
 */
function fixtureMatchStateSource(getHost: () => FakeSimHost): () => replayTypes.MatchState {
  let ballX = 480;
  return (): replayTypes.MatchState => {
    const host = getHost();
    ballX += 2;
    return {
      field: { w: 960, h: 540 },
      goal_home: { x: -12, y: 210, w: 12, h: 120 },
      goal_away: { x: 960, y: 210, w: 12, h: 120 },
      score: { home: host.hud.home_score, away: host.hud.away_score },
      time_left: host.hud.time_left,
      outfield_press: {
        home: { version: 1, mode: "inactive", reason: "no_trigger" },
        away: { version: 1, mode: "inactive", reason: "no_trigger" },
      },
      transition: { version: 1, hold: 0, elapsed: 0 },
      transition_windows: {
        home: { counterpress: 0, counterattack: 0 },
        away: { counterpress: 0, counterattack: 0 },
      },
      ball: new Vec2(ballX, 270),
      ball_vel: new Vec2(120, 0),
      ball_z: 0,
      ball_vz: 0,
      players: [fixtureMatchPlayer("home_1", "home", new Vec2(ballX - 20, 270))],
      events: [],
    };
  };
}

describe("match screen rematch (tier 2)", () => {
  it("R restarts a finished match with the same pre-match choices", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
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
    expect(
      confirmKeys.length,
      "this only proves something with more than one confirm key",
    ).toBeGreaterThan(1);
    for (const key of confirmKeys) {
      const { factory, hosts } = makeHostFactory();
      const screen = new MatchScreen({
        createHost: factory,
        renderer: noopRenderer,
        keyboard: fakeKeyboard({}),
      });
      nth(hosts, 0).hud.finished = true;
      screen.event({ kind: "key", key });
      expect(screen.finished, `${key} should trigger the rematch`).toBe(false);
    }
  });

  it("ignores the rematch keys while the match is live", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });

    screen.event({ kind: "key", key: "r" });

    expect(hosts.length, "no restart mid-match").toBe(1);
  });

  it("ignores match inputs after full time", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
    nth(hosts, 0).hud.finished = true;

    // The meaningful assertion here is that a K press does not buffer a
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
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
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
  // Re-checked against current code, not just the stale comment this case
  // used to carry: `crates/gc-wasm/src/session.rs`'s `Session::new` gained a
  // `combat_enabled` parameter and `Session::step` now threads it through
  // every tick (previously hard-coded `None`) -- see that module's doc.
  // That landed one layer BELOW `SimHostPort`, though: `SimHostFactory` is a
  // no-arg closure by design (`match.ts`'s own doc -- team ids, seed, and
  // formation are already "baked in by the caller's closure", not passed
  // through `SimHostPort` itself), so combat's opt-in belongs there too, not
  // as a widening of the shared `step`/`planTicks`/`cancelPlannedTicks`/
  // `frame`/`roster`/`tick`/`dispose` contract. `MatchScreenOptions.combat_enabled`
  // (already a "separate construction option", not a free addition this
  // case invents) is therefore the right shape, and is exercised below via
  // `MatchScreen.debugCombatEnabled`.
  //
  // What THIS case does NOT prove, and still cannot: neither `Session` nor
  // `SimHostPort` exposes any getter for combat presence or per-tick combat
  // events/state -- `sim.combat`'s observable wasm surface is a separate,
  // out-of-scope binding (not every sim subsystem's bridge
  // is this task's to add). So "drives" below is the same, narrower claim
  // `match_screen.spec.ts`'s own rollback-lab combat case already settles
  // for: the option is recorded accurately at construction and stays stable
  // for the life of the screen -- the TS-glue-observable analog of the
  // Rust regression suite's own "the combat companion must never disappear
  // mid-match" (`crates/gc-wasm/src/session.rs`'s
  // `combat_enabled_true_builds_and_drives_a_combat_companion_every_tick`) --
  // not that real per-tick combat state is readable here, which it still
  // is not.
  //
  // This is a DIFFERENT surface from `match_rollback_lab.spec.ts`'s
  // "constructs the combat companion for an explicit rollback playtest" --
  // that one validates a rollback snapshot's combat-companion PRESENCE at
  // construction time (`rollback_lab`'s own `RollbackHostPort`, this
  // package's own design); this one is the BASE (non-rollback) game loop,
  // which has no such host-reported presence to validate against at all.
  // Solving one does not solve the other.
  it("constructs and drives combat only behind the explicit option", () => {
    const { factory: onFactory } = makeHostFactory();
    const onScreen = new MatchScreen(
      { createHost: onFactory, renderer: noopRenderer, keyboard: fakeKeyboard({}) },
      { combat_enabled: true },
    );
    expect(onScreen.debugCombatEnabled, "the explicit opt-in is recorded at construction").toBe(
      true,
    );

    const { factory: offFactory } = makeHostFactory();
    const offScreenDefault = new MatchScreen({
      createHost: offFactory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
    expect(
      offScreenDefault.debugCombatEnabled,
      "omitted defaults to no combat, byte for byte the prior behavior",
    ).toBe(false);

    const { factory: explicitOffFactory } = makeHostFactory();
    const offScreenExplicit = new MatchScreen(
      { createHost: explicitOffFactory, renderer: noopRenderer, keyboard: fakeKeyboard({}) },
      { combat_enabled: false },
    );
    expect(offScreenExplicit.debugCombatEnabled).toBe(false);

    // "drives": the flag never flips across ticks this screen actually
    // runs -- the TS-glue-observable analog of the Rust regression test
    // named above.
    for (let i = 0; i < 30; i += 1) {
      onScreen.update(1 / 60);
      offScreenDefault.update(1 / 60);
    }
    expect(onScreen.debugCombatEnabled, "the combat companion must never disappear mid-match").toBe(
      true,
    );
    expect(offScreenDefault.debugCombatEnabled).toBe(false);
  });

  it("K never switches while carrying the ball (it charges a pass)", () => {
    const { factory } = makeHostFactory(); // default host: carrying at kickoff
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });

    screen.event({ kind: "key", key: "k" });

    expect(
      screen.debugSwitchPending,
      "on the ball, K is the (polled) pass charge, not a switch",
    ).toBe(false);
  });

  it("K switches player when not carrying", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
    nth(hosts, 0).hud.controlled_owns_ball = false;

    screen.event({ kind: "key", key: "k" });

    expect(screen.debugSwitchPending, "K is a switch off the ball").toBe(true);
  });

  it("Space hold = jockey, release = poke (off the ball)", () => {
    const down: Record<string, boolean> = {};
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard(down),
    });
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
    expect(
      (edges & inputSample.packEdges(["dash"])) !== 0,
      "releasing Space off the ball fires the poke",
    ).toBe(true);
  });

  it("Space never produces a poke while carrying (it charges the shot)", () => {
    const down: Record<string, boolean> = {};
    const { factory } = makeHostFactory(); // default host: carrying
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard(down),
    });

    down.space = true;
    screen.update(1 / 60); // hold while carrying
    const held = screen.debugLastSample?.held ?? 0;
    expect(
      (held & inputSample.packHeld(["shoot"])) !== 0,
      "Space while carrying charges the shot",
    ).toBe(true);
    expect((held & inputSample.packHeld(["jockey"])) !== 0, "not jockey, while carrying").toBe(
      false,
    );

    down.space = false;
    screen.update(1 / 60); // release while still carrying
    const edges = screen.debugLastSample?.edges ?? 0;
    expect(
      (edges & inputSample.packEdges(["dash"])) !== 0,
      "Space release while carrying does not fire a poke",
    ).toBe(false);
  });
});

describe("match screen lob latch (tier 2)", () => {
  it("MODIFIER held during a charged pass lofts it even if it lifts early", () => {
    const down: Record<string, boolean> = {};
    const { factory } = makeHostFactory(); // default host: carrying at kickoff
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard(down),
    });

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
    expect(
      (held & inputSample.packHeld(["lob"])) !== 0,
      "and it was lofted: the latch held the modifier for us",
    ).toBe(true);
  });

  it("holding PLAY charges the pass range for an outfielder", () => {
    const down: Record<string, boolean> = {};
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard(down),
    });

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
      expect(
        (sample.held & inputSample.packHeld(["pass"])) !== 0,
        "the pass range charged up",
      ).toBe(true);
      expect(
        (sample.edges & inputSample.packEdges(["pass"])) !== 0,
        "not fired yet -- still held",
      ).toBe(false);
    }
  });
});

describe("match screen draw separation (tier 2)", () => {
  it("update() no longer draws -- draw() is a separate, explicit call", () => {
    let drawCalls = 0;
    const renderer: RenderPort = {
      draw: (): void => {
        drawCalls += 1;
      },
    };
    const { factory } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer, keyboard: fakeKeyboard({}) });

    screen.update(1 / 60);
    expect(drawCalls, "update() alone must not draw -- see MatchScreen.update's doc").toBe(0);

    screen.draw();
    expect(drawCalls, "draw() draws exactly once").toBe(1);
  });
});

describe("MatchScreenAsRealMatchScreen (tier 2)", () => {
  it("adapts state/finished/draw/teardown to real_match.ts's RealMatchScreenPort", () => {
    let drawCalls = 0;
    const renderer: RenderPort = {
      draw: (): void => {
        drawCalls += 1;
      },
    };
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({ createHost: factory, renderer, keyboard: fakeKeyboard({}) });
    const port = new MatchScreenAsRealMatchScreen(screen);
    const host = nth(hosts, 0);

    host.hud.home_score = 2;
    host.hud.away_score = 1;
    host.hud.time_left = 42;

    expect(port.state).toEqual({ time_left: 42, score: { home: 2, away: 1 } });
    expect(port.rollbackLab).toBe(false);
    expect(port.rollbackConfirmedSteps).toEqual([]);
    expect(port.frameEvents).toEqual([]);
    expect(port.fullTimeConfirmed()).toBe(false);
    expect(port.resultCompletionBlocked()).toBe(false);

    port.draw();
    expect(drawCalls, "draw() forwards to the wrapped screen's draw()").toBe(1);

    host.hud.finished = true;
    expect(port.fullTimeConfirmed()).toBe(true);

    port.teardown();
    expect(host.disposeCalls, "teardown() disposes the wrapped screen's host").toBe(1);
  });

  it("bridges RealMatchInputEvent's action variant into the wrapped screen's event()", () => {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen(
      { createHost: factory, renderer: noopRenderer, keyboard: fakeKeyboard({}) },
      { profile: "playtest" },
    );
    const port = new MatchScreenAsRealMatchScreen(screen);
    nth(hosts, 0).hud.finished = true;

    // `RealMatchInputEvent`'s action variant has no `pressed`/`source` --
    // this is the boundary `toControllerInputEvent` bridges (see match.ts).
    port.event({ kind: "action", action: "confirm" });

    expect(
      screen.finished,
      "the bridged event reached the wrapped screen and triggered a rematch",
    ).toBe(false);
  });
});

describe("match screen goal replay (tier 2)", () => {
  // Cleared: `MatchScreenPorts.matchState` (match.ts's "THE GAME LOOP"
  // section) is the new, second wasm-boundary crossing this describe
  // block's OLD header said would be the real fix -- see this file's own
  // header for the full re-check. `SimHostPort` itself stays exactly the
  // fixed contract it was; this is a SEPARATE, optional port, so
  // `@gc/app`'s `sim_host.ts` is unaffected either way.
  it("a goal freezes the sim into a slow-mo replay; skipping resumes", () => {
    replay.reset();
    replay.resetTuning();
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
      replay,
      matchState: fixtureMatchStateSource(() => hosts[0]!),
    });

    for (let i = 0; i < 40; i += 1) {
      // build up recorded footage
      screen.update(1 / 60);
    }
    hosts[0]!.hud.home_score += 1; // goal edge
    screen.update(1 / 60);
    expect(replay.active(), "the replay rolls after a goal").toBe(true);

    const t0 = hosts[0]!.hud.time_left;
    screen.update(1 / 60);
    expect(hosts[0]!.hud.time_left, "the sim is frozen during the replay").toBe(t0);

    screen.event({ kind: "key", key: "space" });
    expect(replay.active(), "Space skips the replay").toBe(false);

    screen.update(1 / 60);
    expect(hosts[0]!.hud.time_left, "live play resumes").toBeLessThan(t0);
  });
});

// The broadcast follow camera (render/camera_follow.ts) is renderer-owned
// accumulator state: `pitch.follow_camera` only reframes anything if SOMETHING
// calls `cameraFollow.update` every frame, right beside `viewState.update`.
//
// `camera_follow.ts` was added to this workspace without that call site,
// so `cameraFollow.update` had no caller anywhere in the tree: the
// smoothed focus never left `undefined`, `cameraFollow.view` therefore always
// returned `undefined`, and pitch.ts's `currentView` silently fell back to the
// whole-pitch view no matter what the flag said. The follow camera was inert
// code and the flag selecting it could not change the picture -- which is
// exactly AGENTS.md §9's "a harness self-test is not a harness run" failure
// shape: everything present, nothing running. Asserting `view` goes from
// undefined to defined-and-TRACKING is what pins the driver rather than the
// mere presence of the module.
describe("match screen broadcast follow camera (tier 2)", () => {
  it("MatchScreen.update drives cameraFollow, and the focus tracks the moving ball", () => {
    cameraFollow.reset();
    const field = { w: 960, h: 540 };
    expect(cameraFollow.view(field), "dormant until something updates it").toBeUndefined();

    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
      // `fixtureMatchStateSource`'s ball drifts +2 world units per read, which
      // is what gives the camera something to actually follow.
      matchState: fixtureMatchStateSource(() => hosts[0]!),
    });

    for (let i = 0; i < 30; i += 1) {
      screen.update(1 / 60);
    }
    const early = cameraFollow.view(field);
    expect(early, "MatchScreen.update must drive cameraFollow.update").toBeDefined();

    for (let i = 0; i < 120; i += 1) {
      screen.update(1 / 60);
    }
    const late = cameraFollow.view(field);
    expect(late).toBeDefined();
    // Tracking, not merely initialised: a focus that is defined but frozen
    // would pass the assertion above while still framing the wrong thing.
    expect(late!.x, "the focus follows the drifting ball downfield").toBeGreaterThan(early!.x);
    // ...and it stays clamped inside the pitch (camera_follow.ts's `margin_x`).
    expect(late!.x).toBeLessThanOrEqual(field.w);

    screen.dispose();
    cameraFollow.reset();
  });
});

// The producer half of the kick follow-through wiring. `release_follow.ts`
// had no caller anywhere in the tree: nothing fed it match events, so no
// window ever opened, so the `kick_follow` pose was unreachable in a live
// match regardless of what the payload builder did with it. These cases
// drive `MatchScreen.update` and assert on the REAL `@gc/render`
// `releaseFollow` module (already pure, no wasm involved -- the same
// "TS-glue-observable analog" pattern this file's goal-replay and follow-
// camera cases already use for `replay`/`cameraFollow`).
describe("match screen release follow-through (tier 3)", () => {
  afterEach(() => {
    // Module-level state shared with every other consumer in this process.
    releaseFollow.reset();
  });

  function screenWith(): { readonly screen: MatchScreen; readonly host: FakeSimHost } {
    const { factory, hosts } = makeHostFactory();
    const screen = new MatchScreen({
      createHost: factory,
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
      // A goal is only a scene discontinuity `MatchScreen` can act on when
      // it has a correction source to reseed from -- the same port
      // `viewState.reset()` already depends on for this branch.
      matchState: fixtureMatchStateSource(() => nth(hosts, 0)),
    });
    return { screen, host: nth(hosts, 0) };
  }

  it("opens a window for the player a shot is attributed to", () => {
    const { screen, host } = screenWith();
    // Slot 2 of `FakeSimHost.roster()`'s ids -- one-based on the wire.
    host.emitOnNextStep({ count: 1, kind: ["shot"], slot: [2] });

    screen.update(1 / 60);

    expect(releaseFollow.windows()).toEqual({ striker: true });
  });

  it("opens one for a pass too, and for nothing else", () => {
    const { screen, host } = screenWith();
    host.emitOnNextStep({ count: 2, kind: ["pass", "tackle"], slot: [3, 2] });

    screen.update(1 / 60);

    expect(releaseFollow.windows()).toEqual({ playmaker: true });
  });

  it("ages the window out over DURATION seconds of render time", () => {
    const { screen, host } = screenWith();
    host.emitOnNextStep({ count: 1, kind: ["shot"], slot: [2] });
    screen.update(1 / 60);
    expect(releaseFollow.active("striker")).toBe(true);

    // Enough render calls to cover DURATION with room to spare.
    for (let call = 0; call < 12; call += 1) {
      screen.update(1 / 60);
    }
    expect(releaseFollow.active("striker")).toBe(false);
  });

  it("clears every window on a goal, so a follow-through cannot outlive its timeline", () => {
    const { screen, host } = screenWith();
    host.emitOnNextStep({ count: 1, kind: ["shot"], slot: [2] });
    screen.update(1 / 60);
    expect(releaseFollow.active("striker")).toBe(true);

    host.emitOnNextStep({ count: 1, kind: ["shot"], slot: [3] });
    host.scoreOnNextStep();
    screen.update(1 / 60);

    // Latched and then discarded within the same render call: the scene
    // discontinuity gets the last word.
    expect(releaseFollow.windows()).toEqual({});
  });

  it("clears every window on a rematch", () => {
    const { screen, host } = screenWith();
    host.emitOnNextStep({ count: 1, kind: ["shot"], slot: [2] });
    screen.update(1 / 60);
    expect(releaseFollow.active("striker")).toBe(true);

    host.hud.finished = true;
    screen.event({ kind: "key", key: "r" });

    expect(releaseFollow.windows()).toEqual({});
  });

  it("ignores an unattributed release", () => {
    const { screen, host } = screenWith();
    host.emitOnNextStep({ count: 1, kind: ["shot"], slot: [undefined] });

    screen.update(1 / 60);

    expect(releaseFollow.windows()).toEqual({});
  });
});

// ---------------------------------------------------------------------------
// CHARACTER PRE-WARM WIRING (#447 follow-up)
// ---------------------------------------------------------------------------
//
// `player_renderer_3d_prewarm.spec.ts` (in `@gc/render`) proves the MECHANISM:
// given a roster, `prewarmCharacters` builds its variants and a subsequent
// draw builds none. That suite cannot prove anything about the CALL SITE, and
// the call site is the half that regresses -- deleting
// `MatchScreen.prewarmCharacters()` leaves every assertion over there green
// while every real match goes back to building nine geometries inside its
// first drawn frame.
//
// So this block asserts the wiring, and only the wiring: constructing a
// `MatchScreen` over a host with real content ids builds geometry, and does so
// BEFORE `draw()` is ever called.
describe("match screen character pre-warm (#447)", () => {
  // Two `scifi_axi` slots -- one a keeper carrying nothing, one an outfielder
  // with a shield -- plus a medieval slot: three distinct variants from three
  // players, mirroring `gc-data`'s own authored content.
  class ContentfulHost extends FakeSimHost {
    override roster(): RenderFrameRoster {
      return {
        ids: ["ozzo", "sela_dwin", "brakka"],
        teams: ["home", "home", "away"],
        is_keeper: [true, false, false],
        presentation_ids: ["scifi_axi", "scifi_axi", "medieval_rook_emberguard"],
        loadout_ids: [undefined, "loadout_emberguard_shield", "loadout_emberguard_shield"],
      };
    }
  }

  it("builds this match's character geometry at construction, before any frame is drawn", () => {
    const before = player3dVariantBuildCount();
    const screen = new MatchScreen({
      createHost: (): SimHostPort => new ContentfulHost(),
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
    const afterConstruction = player3dVariantBuildCount();

    // NON-VACUOUS: the roster really does carry three distinct variants, so
    // "built at construction" is a statement about work that exists.
    expect(
      afterConstruction - before,
      "three players, three distinct variants, all built up front",
    ).toBe(3);

    // And the frame that follows costs nothing. `noopRenderer` does not reach
    // `pitch.draw`, so this is a weaker statement than the render package's
    // own suite makes -- it is here to pin the ORDER (construction first,
    // draw after), which is the property this call site owns.
    screen.draw();
    expect(player3dVariantBuildCount(), "and drawing adds none").toBe(afterConstruction);
  });

  // A REMATCH DECODES A NEW ROSTER, so the pre-warm has to run again. Driven
  // through the public "R" path (`restart` itself is private), with a factory
  // that hands out a DIFFERENT roster the second time -- so the assertion is
  // about geometry that genuinely did not exist before, not about a call that
  // could be deleted without any number changing.
  it("re-warms on a rematch, whose roster may not be the one already built", () => {
    class SecondMatchHost extends FakeSimHost {
      override roster(): RenderFrameRoster {
        return {
          ids: ["mika_olu", "krag"],
          teams: ["home", "away"],
          is_keeper: [false, false],
          presentation_ids: ["toy_tock", "toy_moxie_modular"],
          loadout_ids: ["loadout_foam_champion", "loadout_pulse_blaster"],
        };
      }
    }
    let created = 0;
    let first: ContentfulHost | undefined;
    const screen = new MatchScreen({
      createHost: (): SimHostPort => {
        created += 1;
        if (created === 1) {
          first = new ContentfulHost();
          return first;
        }
        return new SecondMatchHost();
      },
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
    const afterFirst = player3dVariantBuildCount();

    // "R" only rematches once the match is over, same precondition every
    // other restart case in this file uses.
    if (first === undefined) {
      throw new Error("expected the first host to have been created");
    }
    first.hud.finished = true;
    screen.event({ kind: "key", key: "r" });
    const afterRematch = player3dVariantBuildCount();
    expect(created, "the rematch really did build a second host").toBe(2);
    expect(
      afterRematch - afterFirst,
      "the rematch's two new variants are built at restart, not on its first frame",
    ).toBe(2);

    screen.draw();
    expect(player3dVariantBuildCount(), "so the rematch's first frame builds nothing either").toBe(
      afterRematch,
    );
  });

  it("is a no-op for the content-free fakes every other case in this file uses", () => {
    const before = player3dVariantBuildCount();
    new MatchScreen({
      createHost: (): SimHostPort => new FakeSimHost(),
      renderer: noopRenderer,
      keyboard: fakeKeyboard({}),
    });
    expect(player3dVariantBuildCount(), "a roster with no presentation ids builds nothing").toBe(
      before,
    );
  });
});
