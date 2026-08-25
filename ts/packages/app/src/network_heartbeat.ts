// #612 part 1: keeps the ONLINE network pump breathing when `browser_main.ts`'s
// `requestAnimationFrame` loop stalls.
//
// THE DEFECT (see the issue for the full, independently-verified diagnosis).
// `App.update(dt)` -- which drains `LobbyLink.poll()`/the room-signaling
// channel and steps the online coordinator's fixed tick clock
// (`OnlineLobby.update`, `OnlineMatch.update`, both `packages/screens/src`)
// -- is called from exactly one place: `browser_main.ts`'s rAF callback. A
// throttled, occluded, backgrounded, or dragged window stops that callback
// firing AT ALL while `RTCDataChannel.onmessage` keeps queueing wires no one
// reads. The host's own start-boundary handshake is a one-shot, 2-second
// deadline (`coordinator.rs`'s `START_ACK_TIMEOUT_TICKS`) funded entirely by
// processed tick-time -- so a guest whose rAF stalls for that long never
// echoes the Start wire, and the session dies with "a peer never reached the
// start boundary" / "the connection to a peer was lost". The exact same
// exposure applies to in-match rollback once a match is live.
//
// THE FIX. A `setInterval` heartbeat, owned by the app shell (never by a
// pure screen -- AGENTS.md §9), that calls the SAME `App.update(dt)` the rAF
// loop calls, but ONLY when:
//   1. rAF looks stalled (`shouldPumpHeartbeat`, below) -- either the
//      document is hidden (browsers throttle a hidden tab's rAF to zero) or
//      more than `HEARTBEAT_STALL_THRESHOLD_SECONDS` has passed since the
//      last real rAF frame (an occluded-but-visible, dragged, or
//      GPU-starved window never fires `visibilitychange` at all); and
//   2. the app's current route is an online one (`isOnlineHeartbeatRoute`)
//      -- an offline match already pauses on blur (`App.focus`), and a menu
//      screen has nothing that needs real-time funding, so the heartbeat
//      leaves every other route untouched.
//
// `setInterval` over a recursive `setTimeout` chain: this heartbeat has no
// need for precise, drift-corrected cadence (a menu transition or a title
// idle animation is not on the other end of it) -- it only needs "roughly
// every `HEARTBEAT_INTERVAL_MS`, cheaply, for the app's whole lifetime", and
// `setInterval` is the one-line way to say that. Both timer flavors are
// throttled identically by every major browser once a tab is hidden (Chrome
// caps a hidden tab's timers to ~1 Hz, then further after ~5 minutes), so a
// `setTimeout` chain would buy no extra precision here, only more code to
// reschedule itself. A throttled 1 Hz heartbeat still beats the 2-second
// start-boundary deadline this issue is about by a comfortable margin, and
// still funds in-match rollback far better than the zero pumps a stalled
// rAF delivers today.
//
// RENDERING STAYS rAF-ONLY. `MatchScreen.update`/`OnlineMatch.update`/
// `OnlineLobby.update` never touch WebGL or a `GraphicsBackend` --
// `RenderPort.draw` (three.js) is called only from `MatchScreen.draw`
// (`packages/screens/src/match.ts`), and `OnlineLobby.draw`/menu `draw()`
// calls are made only from `browser_main.ts`'s own rAF `frame()`. This
// module calls `update(dt)` and nothing else, so a heartbeat tick can never
// submit a GL frame.
//
// NO DOUBLE PUMP. The risk: if the rAF loop and this heartbeat each tracked
// their own "time of my last pump", a tick right at the resume boundary
// could re-cover wall-clock time the other pump already funded (or, the
// opposite failure, leave a gap). This module avoids both by keeping ONE
// authority (`lastPumpAt`, private to a `createNetworkHeartbeat` instance)
// for "the last moment ANY pump -- rAF or heartbeat -- funded `update`",
// updated by both `notifyRafFrame` and a firing `tick()`. Every dt this
// module hands to `update` is `now - lastPumpAt`, so the wall-clock
// intervals the two pumps fund never overlap and never gap, regardless of
// which one runs when -- see `network_heartbeat.spec.ts`'s "no double pump"
// case, which proves interleaved rAF+heartbeat delivers the exact same
// total tick count as rAF alone over the same wall time.
//
// A second, SEPARATE timestamp (`lastRafAt`) tracks only "the last real rAF
// frame", written exclusively by `notifyRafFrame`. It exists purely to
// answer "has rAF actually run recently?" -- if the heartbeat's own pumps
// also moved this clock, a stalled rAF would look "fresh" again after a
// single heartbeat tick and the interval would go quiet, undershooting its
// own cadence during a real stall.
//
// UNCLAMPED dt, ON PURPOSE. `browser_main.ts`'s rAF loop clamps its own dt
// to `MAX_FRAME_DT_SECONDS` (0.25s) before calling `update` -- intentional
// for the common case (mirrors love2d's own post-stall dt clamp, bounds a
// single frame's simulation catch-up). This heartbeat does NOT apply that
// clamp: the online coordinator's own deadlines are funded by processed
// tick-time, so under-funding a stall on purpose is exactly the bug this
// file exists to close. Because the heartbeat keeps `lastPumpAt` fresh
// throughout a stall (every ~`HEARTBEAT_INTERVAL_MS`, or ~1s once a hidden
// tab's timers are throttled), the gap rAF eventually sees on resume stays
// small anyway -- so its own clamp, unchanged, never actually bites for the
// case this file is about.

/** How often the heartbeat's own timer fires while the tab is visible.
 * Once `document.hidden`, the browser throttles this on its own (typically
 * to ~1 Hz) -- see this file's header for why that is still comfortably
 * enough. */
export const HEARTBEAT_INTERVAL_MS = 250;

/** How far behind `performance.now()` the last real rAF frame must be
 * before this heartbeat treats rAF as stalled (visible-but-stuck case --
 * an occluded, dragged, or GPU-starved window never fires
 * `visibilitychange`, so elapsed time is the only signal available). Well
 * under the coordinator's 2-second start-boundary deadline, and short
 * enough that a single slow-but-not-actually-stalled frame (a GC pause,
 * a heavy scene rebuild) does not spuriously trigger it. */
export const HEARTBEAT_STALL_THRESHOLD_SECONDS = 0.5;

/** The routes this heartbeat is allowed to pump -- `App.currentRoute()`'s
 * own route names (`app.ts`'s `pushRoute("lobby", ...)` /
 * `pushRoute("online_match", ...)`). Every other route (title, team sheet,
 * an OFFLINE match, ...) is left alone: an offline match already pauses on
 * blur (`App.focus`), and nothing else on this list needs real-time
 * funding while backgrounded. */
export const ONLINE_HEARTBEAT_ROUTES: readonly string[] = ["lobby", "online_match"];

/** Pure: is `route` one this heartbeat should ever pump? */
export function isOnlineHeartbeatRoute(route: string): boolean {
  return ONLINE_HEARTBEAT_ROUTES.includes(route);
}

/** The pure decision this file's whole mechanism turns on -- extracted so
 * it is unit-testable with no timer, no DOM, and no `App` at all. `hidden`
 * short-circuits to `true` unconditionally: a hidden document's rAF is
 * throttled by every major browser, often to zero, well before
 * `now - lastRafAt` would exceed the threshold on its own. */
export interface HeartbeatStallInput {
  /** `performance.now()` (or an equivalent monotonic clock) at the last
   * real rAF frame -- see this file's header for why this is tracked
   * separately from the dt-funding clock. */
  readonly lastRafAt: number;
  /** `performance.now()` right now. */
  readonly now: number;
  /** `document.hidden`. */
  readonly hidden: boolean;
}

/** Should the heartbeat drive an update tick right now? */
export function shouldPumpHeartbeat(input: HeartbeatStallInput): boolean {
  if (input.hidden) {
    return true;
  }
  return input.now - input.lastRafAt > HEARTBEAT_STALL_THRESHOLD_SECONDS * 1000;
}

/** The impure ports a real `browser_main.ts` wires to real globals, and a
 * spec wires to a synthetic clock and a recording `update` -- the same
 * injected-port discipline every screen/driver in this codebase uses. */
export interface NetworkHeartbeatPorts {
  /** `performance.now`, or a spec's own counter. Milliseconds. */
  readonly now: () => number;
  /** `() => document.hidden`. */
  readonly hidden: () => boolean;
  /** `() => app.currentRoute()` -- read fresh on every check, never cached,
   * so a heartbeat firing mid-route-transition always pumps (or does not)
   * the route that is ACTUALLY current at that instant. */
  readonly currentRoute: () => string;
  /** `App.update`, or a fake screen's `update` in a spec. Called ONLY while
   * {@link isOnlineHeartbeatRoute} holds for `currentRoute()` and
   * {@link shouldPumpHeartbeat} holds for the current clock reading. */
  readonly update: (dtSeconds: number) => void;
  readonly setInterval: (callback: () => void, ms: number) => number;
  readonly clearInterval: (handle: number) => void;
}

export interface NetworkHeartbeat {
  /** Runs one heartbeat check at `ports.now()`'s current instant, pumping
   * `ports.update` if warranted. Exposed directly (not only reachable via
   * `start()`'s real interval) so a spec can drive the decision at a
   * synthetic time with no real timer involved at all. */
  tick(): void;
  /** Tells this heartbeat a real rAF frame just ran at `now` -- call once
   * per `browser_main.ts` `frame()` invocation, with the SAME timestamp
   * that callback received. Marks rAF alive (so the next `tick()` finds no
   * stall) and folds `now` into the dt-funding clock (so a `tick()` right
   * after a healthy frame funds only the time since THAT frame, never
   * double-counting it). */
  notifyRafFrame(now: number): void;
  /** Starts calling {@link tick} on a `HEARTBEAT_INTERVAL_MS` interval via
   * `ports.setInterval`. Idempotent. */
  start(): void;
  /** Stops the interval started by {@link start}. Idempotent. Not needed
   * for a page-lifetime shell instance, but keeps this disposable for a
   * spec/teardown. */
  stop(): void;
}

/** Builds one heartbeat instance. See this file's header for the full
 * design and the no-double-pump argument. */
export function createNetworkHeartbeat(ports: NetworkHeartbeatPorts): NetworkHeartbeat {
  let lastRafAt = ports.now();
  let lastPumpAt = lastRafAt;
  let handle: number | undefined;

  function tick(): void {
    const now = ports.now();
    if (!shouldPumpHeartbeat({ lastRafAt, now, hidden: ports.hidden() })) {
      return;
    }
    const elapsedSeconds = Math.max((now - lastPumpAt) / 1000, 0);
    lastPumpAt = now;
    if (isOnlineHeartbeatRoute(ports.currentRoute())) {
      ports.update(elapsedSeconds);
    }
  }

  return {
    tick,
    notifyRafFrame(now: number): void {
      lastRafAt = now;
      lastPumpAt = now;
    },
    start(): void {
      if (handle === undefined) {
        handle = ports.setInterval(tick, HEARTBEAT_INTERVAL_MS);
      }
    },
    stop(): void {
      if (handle !== undefined) {
        ports.clearInterval(handle);
        handle = undefined;
      }
    },
  };
}
