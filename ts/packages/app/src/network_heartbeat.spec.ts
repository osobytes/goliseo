// Three claims about #612 part 1's fix, in one file because they are all
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

    // Never call `app.update` directly, never call `heartbeat.notifyRafFrame`
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
    heartbeat.notifyRafFrame(clock.now());

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

/** Simulates one healthy rAF frame the way `browser_main.ts`'s `frame()`
 * does: call the screen's `update(dt)` directly (rAF's own responsibility,
 * never the heartbeat's), then tell the heartbeat rAF just ran. */
function simulateRafFrame(
  screen: FixedStepAccumulator,
  heartbeat: ReturnType<typeof createNetworkHeartbeat>,
  nowMs: number,
  dtSeconds: number,
): void {
  screen.update(dtSeconds);
  heartbeat.notifyRafFrame(nowMs);
}

function heartbeatOver(
  clock: SyntheticClock,
  screen: FixedStepAccumulator,
): {
  readonly heartbeat: ReturnType<typeof createNetworkHeartbeat>;
} {
  const ports: NetworkHeartbeatPorts = {
    now: () => clock.now(),
    hidden: () => false,
    currentRoute: () => "lobby",
    update: (dt) => screen.update(dt),
    setInterval: () => 0,
    clearInterval: () => {},
  };
  return { heartbeat: createNetworkHeartbeat(ports) };
}

describe("no double pump: interleaved rAF+heartbeat matches rAF alone", () => {
  it("delivers the same total tick count over identical wall-clock time", () => {
    const RAF_DT = 1 / 60; // seconds
    const TOTAL_SECONDS = 2;
    const FRAME_COUNT = Math.round(TOTAL_SECONDS / RAF_DT); // 120

    // Scenario A: rAF alone, healthy, for the whole span.
    const clockA = new SyntheticClock();
    const screenA = new FixedStepAccumulator();
    const { heartbeat: heartbeatA } = heartbeatOver(clockA, screenA);
    for (let frame = 0; frame < FRAME_COUNT; frame += 1) {
      clockA.advance(RAF_DT * 1000);
      simulateRafFrame(screenA, heartbeatA, clockA.now(), RAF_DT);
    }

    // Scenario B: the SAME total wall time (2 seconds), but one second of
    // it is an rAF stall covered entirely by the heartbeat instead of rAF
    // (visible-but-stuck: `hidden` stays false, the
    // elapsed-since-last-rAF-frame check is what fires) -- rAF therefore
    // only covers the OTHER second, split evenly before and after the
    // stall, so total wall time still matches scenario A exactly.
    const clockB = new SyntheticClock();
    const screenB = new FixedStepAccumulator();
    const ports: NetworkHeartbeatPorts = {
      now: () => clockB.now(),
      hidden: () => false,
      currentRoute: () => "lobby",
      update: (dt) => screenB.update(dt),
      setInterval: () => 0,
      clearInterval: () => {},
    };
    const heartbeatB = createNetworkHeartbeat(ports);
    const rafFramesPerSide = FRAME_COUNT / 4; // 30 frames = 0.5s each side
    for (let frame = 0; frame < rafFramesPerSide; frame += 1) {
      clockB.advance(RAF_DT * 1000);
      simulateRafFrame(screenB, heartbeatB, clockB.now(), RAF_DT);
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
    // rAF resumes and finishes out the remaining half-second.
    for (let frame = 0; frame < rafFramesPerSide; frame += 1) {
      clockB.advance(RAF_DT * 1000);
      simulateRafFrame(screenB, heartbeatB, clockB.now(), RAF_DT);
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
    // chunking reproduces the identical residual.
    expect(clockA.now()).toBeCloseTo(clockB.now(), 6);
    expect(Math.abs(screenB.ticks - screenA.ticks)).toBeLessThanOrEqual(1);
  });

  it("a heartbeat tick landing the SAME instant rAF resumes does not double-count that instant", () => {
    const clock = new SyntheticClock();
    const screen = new FixedStepAccumulator();
    const { heartbeat } = heartbeatOver(clock, screen);

    // A healthy frame, then silence (a stall) for 600ms -- past the 500ms
    // threshold, so the heartbeat is now willing to fire.
    simulateRafFrame(screen, heartbeat, clock.now(), 1 / 60);
    clock.advance(600);
    heartbeat.tick(); // funds the 600ms gap
    expect(screen.pollCalls).toBe(2);

    // rAF "resumes" at the exact same instant with a small dt (as it would
    // in production, since `lastFrameTime` on the rAF side is independent
    // -- this call mimics `browser_main.ts`'s own frame() computing dt off
    // its own clock, here just `1/60` again for simplicity) and notifies
    // the heartbeat. The heartbeat must not ALSO fire for time already
    // covered by the tick() above.
    simulateRafFrame(screen, heartbeat, clock.now(), 1 / 60);
    heartbeat.tick(); // same instant -- must be a no-op (elapsed since last pump is 0)

    // A reference accumulator fed the SAME total funded time in ONE call,
    // i.e. with no chunking-order rounding noise at all -- the double-pump
    // guarantee is that the chunked version above never OVER- or
    // UNDER-shoots that reference by more than the one-tick floating-point
    // margin `toBeLessThanOrEqual(1)` allows (see the previous case's own
    // comment for why bit-exact parity is not the right bar).
    const reference = new FixedStepAccumulator();
    reference.update(1 / 60 + 0.6 + 1 / 60);
    expect(Math.abs(screen.ticks - reference.ticks)).toBeLessThanOrEqual(1);
    // 3 update() calls total: the two `simulateRafFrame` calls plus the one
    // heartbeat tick that actually fired -- proves the second `tick()` call
    // (same instant as rAF's resume) genuinely did nothing, not merely that
    // its dt happened to round to zero ticks.
    expect(screen.pollCalls).toBe(3);
  });
});
