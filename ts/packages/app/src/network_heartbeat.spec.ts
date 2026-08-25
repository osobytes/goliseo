// Four claims about #612 part 1's fix, in one file because they are all
// really claims about `network_heartbeat.ts` and nothing else:
//
//   (a) `shouldPumpHeartbeat` -- the pure stall/hidden decision -- unit
//       tested with no clock, no DOM, no `App` at all.
//   (b) driven ONLY through the heartbeat (never a direct `App.update`
//       call), a stalled/hidden rAF still delivers the online pump to a
//       REAL mounted lobby -- `App` + `createOnlinePorts` + a real,
//       wasm-backed coordinator, mirroring `join_link_boot.spec.ts`'s own
//       fixture.
//   (c) interleaving rAF and heartbeat pumps over the same wall-clock span
//       delivers the exact same total tick count as rAF alone -- the
//       no-double-pump guarantee `network_heartbeat.ts`'s header argues
//       for, proven directly against its own dt bookkeeping.
//   (d) THE REVIEWED DEFECT, reproduced directly: `browser_main.ts`'s own
//       `frame()` computes a SEPARATE, render-facing dt from a
//       `lastFrameTime` that only `frame()` writes, so it goes stale for
//       an entire heartbeat-covered stall. `SimulatedRafLoop` below
//       mirrors that exact shape (production's ACTUAL dt computation, not
//       an idealized fixed `1/60` per call the way an earlier version of
//       this file's fixture did -- which is exactly why that version could
//       not have caught this bug), so (c)'s and (d)'s cases exercise the
//       real hazard: a resume frame's stale render dt versus
//       `consumeElapsed`'s genuinely-new dt.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar } from "@gc/transport";
import type { LobbyScreenState, RoomSignalingEvent, RoomSignalingHandle } from "@gc/screens";
import {
  createNetworkHeartbeat,
  HEARTBEAT_STALL_THRESHOLD_SECONDS,
  isOnlineHeartbeatRoute,
  shouldPumpHeartbeat,
  type NetworkHeartbeatPorts,
} from "./network_heartbeat.ts";
import { createOnlinePorts, type OnlinePortsDeps } from "./online_ports.ts";
import type { OnlineWasmHost } from "./online_wasm_host.ts";
import { App } from "./app.ts";
import { APP_CONTENT, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";

const THRESHOLD_MS = HEARTBEAT_STALL_THRESHOLD_SECONDS * 1000;

describe("shouldPumpHeartbeat (pure)", () => {
  it("fires unconditionally while the document is hidden, even with a fresh rAF timestamp", () => {
    expect(shouldPumpHeartbeat({ lastRafAt: 1000, now: 1000, hidden: true })).toBe(true);
  });

  it("does not fire while visible and rAF ran within the threshold", () => {
    expect(
      shouldPumpHeartbeat({ lastRafAt: 1000, now: 1000 + THRESHOLD_MS - 1, hidden: false }),
    ).toBe(false);
  });

  it("does not fire exactly at the threshold (a strict inequality, not >=)", () => {
    expect(shouldPumpHeartbeat({ lastRafAt: 1000, now: 1000 + THRESHOLD_MS, hidden: false })).toBe(
      false,
    );
  });

  it("fires once visible rAF is more than the threshold stale -- an occluded/dragged window that never fires visibilitychange", () => {
    expect(
      shouldPumpHeartbeat({ lastRafAt: 1000, now: 1000 + THRESHOLD_MS + 1, hidden: false }),
    ).toBe(true);
  });
});

describe("isOnlineHeartbeatRoute (pure)", () => {
  it("covers the lobby and the online match route", () => {
    expect(isOnlineHeartbeatRoute("lobby")).toBe(true);
    expect(isOnlineHeartbeatRoute("online_match")).toBe(true);
  });

  it("excludes every offline/menu route -- an offline match already pauses on blur", () => {
    for (const route of ["title", "team_sheet", "match", "multiplayer", "result", "pause"]) {
      expect(isOnlineHeartbeatRoute(route)).toBe(false);
    }
  });
});

// --- (b): a real mounted lobby, pumped ONLY through the heartbeat --------

function nodeWasmHost(): OnlineWasmHost {
  const sim = loadSimHost();
  return sim as unknown as OnlineWasmHost;
}

interface DispatchableLobby {
  update(dt: number): void;
  readonly state: LobbyScreenState;
}

function currentLobby(app: App): DispatchableLobby {
  return app.stack.current() as unknown as DispatchableLobby;
}

/** Mirrors `join_link_boot.spec.ts`'s identically-named fixture: a
 * room-code relay that confirms a guest's join on the very first poll, no
 * real WebSocket and no host-side peer needed. */
function fakeGuestOnlyRoomSignaling(): NonNullable<OnlinePortsDeps["roomSignaling"]> {
  return {
    openHost(): RoomSignalingHandle {
      throw new Error("not exercised by this file");
    },
    openGuest(code: string): RoomSignalingHandle {
      let queue: RoomSignalingEvent[] = [{ kind: "joined", code }];
      return {
        poll: () => {
          const drained = queue;
          queue = [];
          return drained;
        },
        send: () => {},
        close: () => {},
      };
    },
  };
}

function newGuestApp(): App {
  const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
    // No counterpart peer is ever dialed in this file -- see this
    // function's own header. `rendezvous` is deliberately omitted: a
    // never-contacted default is enough for `initialize()` to succeed and
    // for `LobbyLink.poll()` to be a real, callable no-op.
    const star = fakeStar({ role, peer_id: peerId });
    return star.initialize().ok ? star : undefined;
  };
  const onlinePorts = createOnlinePorts({
    wasm: nodeWasmHost(),
    starFactory,
    renderer: noopRenderPort,
    keyboard: fakeKeyboard(),
    content: APP_CONTENT.matchContract,
    roomSignaling: fakeGuestOnlyRoomSignaling(),
  });
  return new App(APP_CONTENT, { online: onlinePorts, presetRoomCode: "A3F9K2" });
}

/** A synthetic clock a spec fully controls -- `now` only ever advances when
 * a case calls {@link SyntheticClock.advance}, so "wall time" in this file
 * means exactly what the case says it means, never real `Date.now()`. */
class SyntheticClock {
  private current = 0;

  now(): number {
    return this.current;
  }

  advance(ms: number): void {
    this.current += ms;
  }
}

describe("a stalled/hidden rAF still delivers the online pump (#612 part 1)", () => {
  it("drives a real lobby's room-code join to completion purely through heartbeat.tick(), with rAF never running at all", () => {
    const app = newGuestApp();
    expect(app.currentRoute()).toBe("lobby");
    const lobby = currentLobby(app);
    // The join is submitted at construction (#598's preset-room-code path)
    // but not yet polled -- proving the heartbeat, not construction itself,
    // is what makes progress from here.
    expect(lobby.state.model.role).toBeUndefined();
    expect(lobby.state.model.room_status).toBe("connecting");

    const clock = new SyntheticClock();
    const updateCalls: number[] = [];
    const heartbeat = createNetworkHeartbeat({
      now: () => clock.now(),
      // The document is hidden for this whole case -- standing in for a
      // backgrounded tab whose rAF callback never fires again until the
      // player returns.
      hidden: () => true,
      currentRoute: () => app.currentRoute(),
      update: (dt) => {
        updateCalls.push(dt);
        app.update(dt);
      },
      setInterval: () => 0,
      clearInterval: () => {},
    });

    // Never call `app.update` directly, never call `heartbeat.consumeElapsed`
    // -- rAF is entirely absent from this case. Fire the heartbeat's own
    // `tick()` the way its real `setInterval` would, every
    // `HEARTBEAT_INTERVAL_MS`, entirely through the synthetic clock.
    for (let i = 0; i < 8; i += 1) {
      clock.advance(250);
      heartbeat.tick();
    }

    expect(updateCalls.length).toBeGreaterThan(0);
    expect(lobby.state.model.role).toBe("guest");
    expect(lobby.state.model.room_status).toBe("connected");
  });

  // These two are about the HEARTBEAT's own gating, not lobby/coordinator
  // behavior, so they use a plain closure for `currentRoute` rather than a
  // real `App` -- the real-lobby fixture above is what proves this file's
  // central claim; these confirm the two conditions that keep it from
  // pumping when it should not.
  it("does nothing while rAF is healthy (not hidden, last frame recent) -- no spurious pump on every route", () => {
    const clock = new SyntheticClock();
    let updateCalls = 0;
    const heartbeat = createNetworkHeartbeat({
      now: () => clock.now(),
      hidden: () => false,
      currentRoute: () => "lobby",
      update: () => {
        updateCalls += 1;
      },
      setInterval: () => 0,
      clearInterval: () => {},
    });
    heartbeat.consumeElapsed(clock.now());

    clock.advance(100); // well under the stall threshold
    heartbeat.tick();

    expect(updateCalls).toBe(0);
  });

  it("never pumps an offline/menu route, even while genuinely stalled", () => {
    const clock = new SyntheticClock();
    let updateCalls = 0;
    const heartbeat = createNetworkHeartbeat({
      now: () => clock.now(),
      hidden: () => true,
      currentRoute: () => "title",
      update: () => {
        updateCalls += 1;
      },
      setInterval: () => 0,
      clearInterval: () => {},
    });

    clock.advance(1000);
    heartbeat.tick();

    expect(updateCalls).toBe(0);
  });
});

// --- (c): no double pump --------------------------------------------------

/** A minimal stand-in for `OnlineLobby.update`/`OnlineMatch.update`'s own
 * shape (`packages/screens/src/online_lobby.ts:412-416`,
 * `online_match.ts:548-552`): poll a link, then run a persistent
 * fixed-step accumulator over `dt`. The claim under test here is about the
 * HEARTBEAT's dt bookkeeping, not about lobby/coordinator protocol
 * correctness (already covered by `lobby_flow.spec.ts` and the real-lobby
 * case above) -- so this fake carries only the one property that matters
 * for it: total ticks emitted depends solely on the SUM of `dt` it
 * receives, never on how that sum is chunked across calls. */
class FixedStepAccumulator {
  private static readonly TICK_SECONDS = 1 / 60;
  pollCalls = 0;
  ticks = 0;
  private accumulator = 0;

  update(dt: number): void {
    this.pollCalls += 1;
    this.accumulator += dt;
    while (this.accumulator >= FixedStepAccumulator.TICK_SECONDS) {
      this.accumulator -= FixedStepAccumulator.TICK_SECONDS;
      this.ticks += 1;
    }
  }
}

/** `browser_main.ts`'s own `MAX_FRAME_DT_SECONDS` -- duplicated here (not
 * imported: that module needs a DOM/canvas to even load) so this fixture's
 * render-side clamp matches production's exactly. */
const MAX_FRAME_DT_SECONDS = 0.25;

/**
 * Mirrors `browser_main.ts`'s `frame()` FAITHFULLY -- this is the fixture
 * the review round found missing. An earlier version of this file's
 * "simulate one rAF frame" helper called `screen.update(dt)` with an
 * IDEALIZED, always-correct fixed dt and separately told the heartbeat rAF
 * had run; it could never diverge from the heartbeat's own bookkeeping, so
 * it could not have caught the reviewed bug even in principle. This class
 * instead reproduces production's actual TWO-dt-computation shape:
 *
 *   - `renderDt`: this loop's OWN clamped dt, from `lastFrameTime` --
 *     which ONLY a `frame()` call ever advances, so it goes stale for an
 *     entire rAF stall exactly like `browser_main.ts`'s does. Exposed by
 *     {@link frame} only so a case can show what it would have been;
 *     production never feeds it to `update()` (see `browser_main.ts`'s own
 *     comment at that call site).
 *   - the `app.update()` dt: `heartbeat.consumeElapsed(now)`, and nothing
 *     else -- the single authority `network_heartbeat.ts`'s header
 *     describes.
 */
class SimulatedRafLoop {
  private readonly clock: SyntheticClock;
  private readonly heartbeat: ReturnType<typeof createNetworkHeartbeat>;
  private readonly screen: FixedStepAccumulator;
  private lastFrameTime: number;

  constructor(
    clock: SyntheticClock,
    heartbeat: ReturnType<typeof createNetworkHeartbeat>,
    screen: FixedStepAccumulator,
  ) {
    this.clock = clock;
    this.heartbeat = heartbeat;
    this.screen = screen;
    this.lastFrameTime = clock.now();
  }

  /** One rAF callback at the clock's current instant. Returns the
   * render-facing `renderDt` it computed (and deliberately did NOT feed to
   * `update()`). */
  frame(): number {
    const now = this.clock.now();
    const renderDt = Math.min(Math.max((now - this.lastFrameTime) / 1000, 0), MAX_FRAME_DT_SECONDS);
    this.lastFrameTime = now;
    const appDt = this.heartbeat.consumeElapsed(now);
    this.screen.update(appDt);
    return renderDt;
  }
}

function heartbeatOver(
  clock: SyntheticClock,
  screen: FixedStepAccumulator,
  currentRoute: () => string = () => "lobby",
): ReturnType<typeof createNetworkHeartbeat> {
  const ports: NetworkHeartbeatPorts = {
    now: () => clock.now(),
    hidden: () => false,
    currentRoute,
    update: (dt) => screen.update(dt),
    setInterval: () => 0,
    clearInterval: () => {},
  };
  return createNetworkHeartbeat(ports);
}

describe("no double pump: interleaved rAF+heartbeat matches rAF alone", () => {
  it("delivers the same total tick count over identical wall-clock time", () => {
    const RAF_DT = 1 / 60; // seconds
    const TOTAL_SECONDS = 2;
    const FRAME_COUNT = Math.round(TOTAL_SECONDS / RAF_DT); // 120

    // Scenario A: rAF alone, healthy, for the whole span.
    const clockA = new SyntheticClock();
    const screenA = new FixedStepAccumulator();
    const loopA = new SimulatedRafLoop(clockA, heartbeatOver(clockA, screenA), screenA);
    for (let frame = 0; frame < FRAME_COUNT; frame += 1) {
      clockA.advance(RAF_DT * 1000);
      loopA.frame();
    }

    // Scenario B: the SAME total wall time (2 seconds), but one second of
    // it is an rAF stall covered entirely by the heartbeat instead of rAF
    // (visible-but-stuck: `hidden` stays false, the
    // elapsed-since-last-rAF-frame check is what fires) -- rAF therefore
    // only covers the OTHER second, split evenly before and after the
    // stall, so total wall time still matches scenario A exactly. Crucially,
    // `loopB.frame()` is simply never called during the stall -- exactly
    // like production, where rAF stalling means `frame()` itself is not
    // invoked -- so `loopB`'s own `lastFrameTime` goes stale for that whole
    // second, the same way `browser_main.ts`'s does.
    const clockB = new SyntheticClock();
    const screenB = new FixedStepAccumulator();
    const heartbeatB = heartbeatOver(clockB, screenB);
    const loopB = new SimulatedRafLoop(clockB, heartbeatB, screenB);
    const rafFramesPerSide = FRAME_COUNT / 4; // 30 frames = 0.5s each side
    for (let frame = 0; frame < rafFramesPerSide; frame += 1) {
      clockB.advance(RAF_DT * 1000);
      loopB.frame();
    }
    // rAF stalls for one full second -- four heartbeat ticks at the
    // production 250ms cadence cover it instead, entirely unclamped. (The
    // first two land under the 500ms stall threshold and do nothing; the
    // third and fourth fire and together still fund the full second,
    // because dt is computed off `lastPumpAt`, never off a fixed
    // per-tick chunk -- no wall-clock time is ever lost to a quiet tick.)
    for (let i = 0; i < 4; i += 1) {
      clockB.advance(250);
      heartbeatB.tick();
    }
    // rAF resumes and finishes out the remaining half-second. `loopB`'s own
    // `lastFrameTime` is stale here (frozen since the 30th frame, 1.5s
    // ago) -- proving `frame()`'s render-facing dt clamp never leaks into
    // `screenB` is exactly what keeps this scenario's ticks matching
    // scenario A's.
    for (let frame = 0; frame < rafFramesPerSide; frame += 1) {
      clockB.advance(RAF_DT * 1000);
      loopB.frame();
    }

    // Not bit-exact equality: summing the SAME total wall time as many
    // small `1/60` steps (scenario A) versus a mix of `1/60` steps and two
    // larger heartbeat-funded chunks (scenario B) can land the accumulator
    // on different sides of a tick boundary by one ULP-scale rounding
    // difference -- inherent to floating-point summation order, and
    // already true of the fixed-step accumulator this fake mirrors
    // (`online_lobby.ts`/`online_match.ts`) regardless of this heartbeat.
    // The property actually under test is "no SYSTEMATIC double-count or
    // gap from re-chunking the same wall time", i.e. at most one tick of
    // quantization noise at the boundary -- not that every possible
    // chunking reproduces the identical residual. (Before the fix, this
    // assertion failed hard: the resume frame's stale, clamped render dt
    // re-funded up to `MAX_FRAME_DT_SECONDS` of already-delivered time,
    // an error many tens of ticks wide, not a one-tick rounding wobble.)
    expect(clockA.now()).toBeCloseTo(clockB.now(), 6);
    expect(Math.abs(screenB.ticks - screenA.ticks)).toBeLessThanOrEqual(1);
  });

  it("a heartbeat tick landing the SAME instant rAF resumes does not double-count that instant", () => {
    const clock = new SyntheticClock();
    const screen = new FixedStepAccumulator();
    const heartbeat = heartbeatOver(clock, screen);
    const loop = new SimulatedRafLoop(clock, heartbeat, screen);

    // A healthy frame, then silence (a stall) for 600ms -- past the 500ms
    // threshold, so the heartbeat is now willing to fire.
    loop.frame();
    clock.advance(600);
    heartbeat.tick(); // funds the 600ms gap
    expect(screen.pollCalls).toBe(2);

    // rAF "resumes" at the exact same instant. The heartbeat must not ALSO
    // fire for time already covered by the tick() above.
    loop.frame();
    heartbeat.tick(); // same instant -- must be a no-op (elapsed since last pump is 0)

    // A reference accumulator fed the SAME total WALL-CLOCK time
    // (`clock.now()`, not a hand-assembled guess at what each call "should"
    // contribute -- the first `loop.frame()` and the resume `loop.frame()`
    // both genuinely fund ~0s here, since nothing advanced the clock before
    // the first one and the heartbeat tick already caught the second one
    // fully up) in ONE call, i.e. with no chunking-order rounding noise at
    // all -- the double-pump guarantee is that the chunked version above
    // never OVER- or UNDER-shoots that reference by more than the one-tick
    // floating-point margin `toBeLessThanOrEqual(1)` allows (see the
    // previous case's own comment for why bit-exact parity is not the
    // right bar).
    const reference = new FixedStepAccumulator();
    reference.update(clock.now() / 1000);
    expect(Math.abs(screen.ticks - reference.ticks)).toBeLessThanOrEqual(1);
    // 3 update() calls total: the two `loop.frame()` calls plus the one
    // heartbeat tick that actually fired -- proves the second `tick()` call
    // (same instant as rAF's resume) genuinely did nothing, not merely that
    // its dt happened to round to zero ticks.
    expect(screen.pollCalls).toBe(3);
  });
});

// --- (d): the reviewed defect, reproduced and pinned ----------------------

describe("the reviewed defect: a stale rAF-side lastFrameTime must never re-fund wall time (#612 follow-up)", () => {
  it("a resume frame funds only genuinely-new wall time, not the render clamp's stale re-read", () => {
    const clock = new SyntheticClock();
    const screen = new FixedStepAccumulator();
    const heartbeat = heartbeatOver(clock, screen);
    const loop = new SimulatedRafLoop(clock, heartbeat, screen);

    // One healthy frame at t=0 -- `lastFrameTime` (loop-internal) and the
    // heartbeat's own clocks all start here.
    loop.frame();

    // A visible-but-stuck stall: `loop.frame()` -- i.e. rAF -- is never
    // called again until t=1900ms, exactly like production (`frame()`'s
    // only writer of `lastFrameTime` simply does not run during a stall).
    // The heartbeat covers it at its production 250ms cadence; the first
    // two ticks land under the 500ms stall threshold and do nothing, then
    // four more each fund the genuinely-new 250ms since the last one,
    // reaching t=1750ms with `lastPumpAt` caught all the way up.
    for (let i = 0; i < 7; i += 1) {
      clock.advance(250);
      heartbeat.tick();
    }
    expect(clock.now()).toBe(1750);

    // rAF resumes 150ms after the heartbeat's last pump -- NOT at the same
    // instant, the more realistic case: there is no reason rAF's first
    // resumed frame lands exactly on a heartbeat tick boundary.
    clock.advance(150);
    expect(clock.now()).toBe(1900);
    const renderDt = loop.frame();

    // The hazard is real and this fixture reproduces it: `lastFrameTime`
    // was last written at t=0, so the render-facing dt reads as the full
    // `MAX_FRAME_DT_SECONDS` clamp ceiling -- NOT the 150ms that is
    // genuinely new since the heartbeat's last pump. Before the fix, THIS
    // value (mislabelled `dt` at the single call site) was what got handed
    // to `app.update()`, over-funding this one resume frame by 100ms.
    expect(renderDt).toBeCloseTo(MAX_FRAME_DT_SECONDS, 6);

    // The fix: the resume frame's actual funded ticks reflect only the
    // genuinely-new 150ms, via `consumeElapsed` -- proven end to end
    // through the accumulator itself (not by reading `appDt` back out of
    // `SimulatedRafLoop`, which does not expose it, but by checking total
    // ticks against a reference fed the exact same total wall time in one
    // shot).
    const totalWallSeconds = clock.now() / 1000; // 1.9s of wall time, all told
    const reference = new FixedStepAccumulator();
    reference.update(totalWallSeconds);
    expect(Math.abs(screen.ticks - reference.ticks)).toBeLessThanOrEqual(1);
  });
});
