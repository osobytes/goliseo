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
// NO DOUBLE PUMP -- SINGLE dt AUTHORITY (post-review revision). The risk:
// if the rAF loop and this heartbeat each tracked their own "time of my
// last pump", a tick right at the resume boundary could re-cover
// wall-clock time the other pump already funded (or, the opposite failure,
// leave a gap). An EARLIER version of this file tried to guard against
// that with a `notifyRafFrame(now)` method that only updated this module's
// OWN internal bookkeeping -- but `browser_main.ts`'s `frame()` kept
// computing its OWN separate dt for the `app.update()` call, from ITS OWN
// `lastFrameTime`, which ONLY `frame()` writes. That `lastFrameTime` goes
// stale for an entire stall (rAF never runs, so `frame()` -- its only
// writer -- is never called), so the FIRST frame after ANY heartbeat-
// covered stall computed `dt = MAX_FRAME_DT_SECONDS` from that stale
// timestamp and fed `app.update()` wall time the heartbeat had ALREADY
// delivered during the stall -- a real double pump, traced numerically in
// review: a 1.9s stall funded by heartbeat ticks up to t=1850 got roughly
// another `MAX_FRAME_DT_SECONDS` re-funded at the t=1900 resume frame.
// Every extra millisecond becomes an extra coordinator tick (the
// accumulators in `online_lobby.ts`/`online_match.ts` have no cap), so
// repeated stalls would accumulate LOCAL tick-clock drift against the
// peer -- the exact defect class this whole file exists to eliminate.
//
// The fix: {@link NetworkHeartbeat.consumeElapsed} is now the ONE
// authority for the `app.update()` call, for BOTH pumps. It returns
// `now - lastPumpAt` and advances `lastPumpAt` to `now`, so `frame()` MUST
// call it and use its return value for `app.update()` instead of computing
// a dt of its own for that call. `frame()` still computes its own separate
// `lastFrameTime`/clamped dt too -- but that pair now serves ONLY its
// original render-facing consumers (`lastFrameDtSeconds`, read by
// `viewState.update`/`cameraFollow.update` inside `RenderPort.draw`),
// which this heartbeat has no reason to touch and `browser_main.ts` leaves
// untouched. Every dt `consumeElapsed`/`tick()` hand to `update` covers a
// non-overlapping, gap-free slice of wall time regardless of which pump
// ran when -- see `network_heartbeat.spec.ts`'s "no double pump" cases,
// including one that reproduces the reviewed defect's exact shape (a stale
// render-side `lastFrameTime` alongside the correct `consumeElapsed`
// value) and proves the fixture can tell them apart.
//
// A second, SEPARATE timestamp (`lastRafAt`) tracks only "the last real rAF
// frame", written exclusively by `consumeElapsed` (the rAF path's own call
// -- `tick()` never touches it). It exists purely to answer "has rAF
// actually run recently?" -- if the heartbeat's own pumps also moved this
// clock, a stalled rAF would look "fresh" again after a single heartbeat
// tick and the interval would go quiet, undershooting its own cadence
// during a real stall.
//
// UNCLAMPED dt, ALWAYS, FOR EVERY CALLER -- ON PURPOSE. `browser_main.ts`'s
// rAF loop clamps ITS OWN, SEPARATE render-facing dt to
// `MAX_FRAME_DT_SECONDS` (0.25s) -- intentional there (mirrors love2d's
// own post-stall dt clamp, bounds a single frame's animation/camera
// catch-up). `consumeElapsed`/`tick()` never apply that clamp, or any
// clamp, to the value handed to `app.update()` -- deliberately the SAME
// choice for a healthy rAF frame, a stalled rAF's resume frame, AND a
// heartbeat tick, rather than three different rules to keep synchronized:
//   - The online coordinator's own deadlines are funded by processed
//     tick-time (this file's whole reason to exist), so under-funding a
//     stall on purpose is exactly the bug being fixed -- clamping a
//     HEARTBEAT tick's dt would silently drop the very time it exists to
//     deliver. `online_match.ts`'s `OnlineMatch.update` accumulator is
//     itself uncapped for the same reason (see its own comment, and
//     `match.ts`'s `MAX_ONLINE_TICKS_PER_UPDATE` for the DIFFERENT,
//     deliberately-still-capped concern: how much LOCAL RENDERED
//     simulation one call attempts -- the two are not the same knob, and
//     capping the coordinator's funding to match the render cap would
//     reintroduce exactly the starvation #612 fixes).
//   - Clamping ONLY the rAF-side call would not be safe either: the
//     resumed frame's own `consumeElapsed` gap (time since the LAST
//     heartbeat pump, which can itself be up to the heartbeat's own
//     cadence -- ~250ms visible, ~1s once a hidden tab's timers are
//     throttled) can exceed 0.25s on its own, so clamping it would drop
//     genuinely-unfunded time right at the resume boundary -- a smaller
//     instance of the very bug this revision fixes.
//   - On a HEALTHY frame (no stall, either pump), the elapsed value is a
//     tiny fraction of a frame either way, so a clamp would never engage
//     there regardless -- there is no "healthy path" behavior a clamp
//     would meaningfully preserve.
// Every other route (title, team sheet, an OFFLINE match, ...) either
// already stops calling `update` on the current screen while backgrounded
// (an offline match pauses via `App.focus`) or has no dt-driven catch-up
// loop that a large, unclamped dt could turn into a runaway cost (a menu
// screen's own `motion.advance`-style transition simply saturates).

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
  /**
   * THE SINGLE dt AUTHORITY for `App.update()` -- call this once per real
   * rAF frame, from `browser_main.ts`'s `frame()`, with the SAME `now`
   * that callback received, and feed ITS RETURN VALUE to `app.update()`
   * for that frame. Do not compute a separate dt for that call: see this
   * file's header ("NO DOUBLE PUMP -- SINGLE dt AUTHORITY") for the bug
   * that came from doing exactly that. Marks rAF alive (so the next
   * `tick()` finds no stall) and returns `now - lastPumpAt` in seconds,
   * UNCLAMPED, before advancing `lastPumpAt` to `now` -- so a call right
   * after a healthy frame returns only the time since THAT frame, and a
   * call right after a heartbeat-covered stall returns only the
   * genuinely-new time since the heartbeat's own last pump, never
   * double-counting either way.
   */
  consumeElapsed(now: number): number;
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
    consumeElapsed(now: number): number {
      lastRafAt = now;
      const elapsedSeconds = Math.max((now - lastPumpAt) / 1000, 0);
      lastPumpAt = now;
      return elapsedSeconds;
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
