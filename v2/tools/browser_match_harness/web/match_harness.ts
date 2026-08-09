// A match that is already running when the page loads.
//
// WHY THIS PAGE EXISTS. Comparing v2 against the Lua build, or profiling
// either, kept being defeated by the product shell rather than by the thing
// under test:
//
//   * Four menu steps stand between load and kickoff, and the two builds row
//     their buttons at slightly different heights, so any coordinate-driven
//     script diverges between them and lands on the wrong control.
//   * `browser_main.ts` pauses the match when the window loses focus
//     (`window.addEventListener("blur", ...)`). The Lua build does the same,
//     so it is CORRECT there -- and fatal here: a window driven over CDP, or
//     sat beside a second window, is never focused, so the match freezes the
//     instant you look away. Several "the simulation is frozen" observations
//     during this port were only ever that.
//   * A click that misses a menu button by a few pixels lands on the running
//     match and pauses it, which looks identical to a hang.
//
// So this page has no menus, no screen stack, and no focus handling at all.
// It boots wasm, builds one `SceneRoot`, constructs a `Session` directly, and
// runs a fixed-timestep loop forever. It renders whether or not the window is
// focused, which is the whole point.
//
// WHAT IT MODELS, AND WHAT IT DOES NOT. The simulation, the render frame
// crossing, and the draw are the REAL ones -- the same `Session`, the same
// raw pointer -> `Float64Array` view, the same `SceneRoot.render`, on the
// same browser wasm artifact the app ships. What it deliberately leaves out
// is the product shell: no HUD, no input, no screen transitions.
//
// EVERY player is AI-driven, INCLUDING the one on the human-input branch.
// That is not a detail. Feeding the local slot a neutral wire does not make
// it AI-driven, it makes it an idle human -- and the symptom is unmistakable
// once you have seen it: the instant that player wins the ball they stand
// still holding it, because nothing is telling them to do anything. A harness
// that does this is showing a match with one broken player in it, which is
// worse than useless for judging feel. `Session.enableBot`/`botWire` drive
// that slot from `gc_sim::bot` -- the same bot `sim/headless.lua` and
// `game/render/benchmark.lua` use on the Lua side, and the same one
// `session_ai_driven_differential` proves bit-exact against it.
//
// The Lua counterpart of this page is `love . --benchmark` (windowed,
// bot-driven, no menus -- `game/render/benchmark.lua`), reachable in the
// love.js build as `?arg=["--benchmark", ...]`. Comparing this page against
// the Lua PRODUCT build instead would be comparing a bot-driven match against
// a menu-driven one that pauses when you look away.
//
// `window.__gcMatchHarness` carries live stats for a driver script; the
// on-screen readout shows the same numbers for a human watching.

import init, { Session, __getRawExports } from "../../../ts/packages/wasm/dist/pkg-web/gc_wasm.js";
import * as THREE from "three";
import { SceneRoot, Stadium, camera, cameraFollow, effects, frameBuffer, pitch, releaseFollow, viewState } from "@gc/render";
import type { frameBufferTypes } from "@gc/render";

const DT = 1 / 60;

// The product shell's own match colours (`browser_main.ts`).
const HOME_COLOR: readonly [number, number, number] = [0.35, 0.75, 1.0];
const AWAY_COLOR: readonly [number, number, number] = [1.0, 0.55, 0.25];

interface HarnessStats {
  status: "booting" | "running" | "finished" | "error";
  error: string | null;
  /** Rendered frames per second, over the last sampling window. */
  fps: number;
  /** Simulation ticks per second. Should hold ~60 regardless of `fps`. */
  tps: number;
  /** Draw calls in the most recent frame. */
  drawCalls: number;
  /** Mean ms per frame spent stepping the simulation. */
  simMs: number;
  /** Mean ms per frame spent decoding the render frame. */
  decodeMs: number;
  /** Mean ms per frame inside `SceneRoot.render` -- scene rebuild + GL. */
  renderMs: number;
  /** Mean ms per frame in `SceneRoot.populate` alone: CPU scene assembly, no
   * rasterisation. `renderMs - populateMs` is the GL half (plus, in a real
   * window, the vsync stall the driver blocks on). */
  populateMs: number;
  /** Milliseconds per frame burned by the `?spin=` measurement lever, if any.
   * Outside sim/decode/populate/render, so those stay comparable across a
   * sweep. */
  spinMs: number;
  /** Ticks the most recent render call consumed. >1 means the renderer is
   * behind the simulation, which is also when the shell's known
   * one-sample-per-render-call input bug would double an edge. */
  ticksLastFrame: number;
  tick: number;
  timeLeft: number;
  score: string;
  /** Per-roster-slot pose id from the most recent decoded frame, and the
   * per-slot `viewState` lean that drives the rigged torso tilt. Diagnostics
   * only, in the same spirit as `__gcScene` below: a driver script watching
   * for "a shot just happened" or "this player is leaning hard" has no other
   * way to know WHEN to capture, because both facts live inside the frame
   * loop and neither is legible from a screenshot alone. Nothing under
   * v2/ts reads these. */
  poses: (string | undefined)[];
  leans: number[];
  /** Per-roster-slot WORLD position, the ball's world position, the field's
   * size, and the follow camera's current view -- all read straight off the
   * frame this loop already decoded and the `cameraFollow` it already
   * updated. Same diagnostics-only status as `poses`/`leans` above, added
   * for the same reason and to answer the question those two could not:
   * WHERE the posed player is relative to what the camera is framing.
   *
   * `scripts/browser_match_harness.py --pose` finds ticks at which some
   * player holds a given pose; without these fields it cannot tell a pose
   * held in shot from one held off screen, which is exactly how #438's one
   * `keeper_tip` sighting turned out to be useless. The driver derives the
   * on-camera test in Python from `view_zoom` and the field size rather
   * than projecting here, so this stays a read of state and adds no
   * geometry to the page.
   *
   * `view_*` are null until `cameraFollow` has a smoothed focus (its own
   * `view()` returns undefined before the first update) or when
   * `?stadium=0` leaves the follow camera off. Nothing under v2/ts reads
   * any of this. */
  playerX: number[];
  playerY: number[];
  ballX: number;
  ballY: number;
  fieldW: number;
  fieldH: number;
  viewX: number | null;
  viewY: number | null;
  viewZoom: number | null;
}

declare global {
  // eslint-disable-next-line no-var
  var __gcMatchHarness: HarnessStats | undefined;
}

async function main(): Promise<void> {
  const params = new URLSearchParams(location.search);
  const seed = Number(params.get("seed") ?? "1");
  // RENDER AT THE FIELD'S OWN SIZE AND UPSCALE, which is what the love.js
  // build does ("keeps the 960x540 logical canvas at 16:9",
  // docs/online/browser_build.md).
  //
  // HISTORY, and why this is no longer load-bearing. This started as a
  // workaround for #414: `camera.projectFixed` -- ported character for
  // character from Lua -- put the viewport factor in the SCREEN POSITION and
  // not in the depth scale:
  //
  //     sx    = vp.w/2 + (wx - field.w/2) * scale * (vp.w / field.w)
  //     scale = far_scale + (near_scale - far_scale) * t
  //
  // Everything sized off that scale (`r = radius * scale`, and so every
  // character, billboard, shadow and reticle derived from it) therefore had
  // NO viewport factor. That was invisible while vp.w == field.w, which is
  // the only case Lua ever runs and was the only case the specs covered.
  // Rendering at the window's native size instead stretched the pitch while
  // the players stayed put -- measured: a 2x viewport moved pitch width
  // 830 -> 1634 px and left character ppm bit-identical at 29.8969.
  //
  // `camera.ts`'s `projectFixed` now carries a single uniform world-to-pixel
  // factor into both positions and sizes, so rendering at the window's native
  // size would be CORRECT here too, and `packages/app`'s browser entry does
  // exactly that with no workaround of its own.
  //
  // Pinning is kept because it is still the right choice for THIS page for
  // two independent reasons that have nothing to do with the defect: the
  // drawing buffer stays small (fewer pixels, and this page exists to measure
  // frame cost, so the pixel count should be a constant of the harness rather
  // than a property of whatever window it was opened in), and a fixed logical
  // size keeps successive runs comparable. `?width=`/`?height=` override it.
  const width = Number(params.get("width") ?? "960");
  const height = Number(params.get("height") ?? "540");
  // Long by default: this page is for watching, and a match that ends after
  // two minutes stops being useful mid-observation. `?duration=120` gives the
  // product's own length when that is what you want to compare.
  const durationSeconds = Number(params.get("duration") ?? "3600");
  // Separate from the match seed on purpose, so a difference in how the match
  // unfolds is attributable to one or the other rather than to both at once.
  const botSeed = Number(params.get("bot_seed") ?? "11");

  const stats: HarnessStats = {
    status: "booting",
    error: null,
    fps: 0,
    tps: 0,
    drawCalls: 0,
    simMs: 0,
    decodeMs: 0,
    renderMs: 0,
    populateMs: 0,
    ticksLastFrame: 0,
    spinMs: 0,
    tick: 0,
    timeLeft: 0,
    score: "0-0",
    poses: [],
    leans: [],
    playerX: [],
    playerY: [],
    ballX: 0,
    ballY: 0,
    fieldW: 0,
    fieldH: 0,
    viewX: null,
    viewY: null,
    viewZoom: null,
  };
  globalThis.__gcMatchHarness = stats;

  const readout = document.getElementById("stats");
  const canvas = document.getElementById("gl-canvas") as HTMLCanvasElement | null;
  if (canvas === null) {
    throw new Error("match_harness: #gl-canvas is missing");
  }
  canvas.width = width;
  canvas.height = height;
  // CSS size is independent of the drawing buffer: letterbox the largest
  // field-aspect rectangle that fits, exactly as the love.js host does for a
  // non-16:9 window.
  const surface = canvas;
  function fitToWindow(): void {
    const scale = Math.min(window.innerWidth / width, window.innerHeight / height);
    surface.style.width = `${Math.floor(width * scale)}px`;
    surface.style.height = `${Math.floor(height * scale)}px`;
    surface.style.display = "block";
    surface.style.margin = "0 auto";
  }
  fitToWindow();
  window.addEventListener("resize", fitToWindow);

  await init();

  const glRenderer = new THREE.WebGLRenderer({ canvas, antialias: true, powerPreference: "high-performance" });
  // Same reason as the render bench: `SceneRoot.render` runs several internal
  // passes (bloom), each of which resets the counter at its own start when
  // `autoReset` is left on, so the number read afterwards would be the LAST
  // pass's rather than the frame's total.
  glRenderer.info.autoReset = false;

  // NATIVE RESOLUTION OVER A LOGICAL COORDINATE SPACE.
  //
  // The logical viewport has to stay at the field's size (see the note on
  // `width`/`height` above), but rendering a 960x540 BUFFER and letting CSS
  // stretch it to the window throws away every device pixel above that -- a
  // 960x540 image blown up ~1.9x, which reads as chunky and soft. love.js pays
  // that cost because it has no way not to; three.js does. The pixel ratio
  // scales the DRAWING BUFFER while the scene keeps its own coordinate space,
  // so the game still draws in Lua's 960x540 and still rasterises at the
  // display's real resolution.
  //
  // Must be passed to `SceneRoot`, not set on the renderer beforehand: its
  // constructor applies `options.pixelRatio ?? 1` and would reset it.
  // `SceneRoot.resize` uses `setSize`, which preserves whatever ratio is
  // current, so this survives resizes.
  //
  // Capped at 3 so a HiDPI display cannot quietly ask for a 9x-area buffer.
  function pixelRatioForWindow(): number {
    // `?ratio=` pins it, for separating "we render more pixels than love.js
    // does" from "something is pathologically slow". love.js is effectively
    // ratio 1: a 960x540 buffer stretched by the browser.
    const pinned = params.get("ratio");
    if (pinned !== null) {
      return Math.max(Number(pinned), 0.1);
    }
    const fit = Math.min(window.innerWidth / width, window.innerHeight / height);
    return Math.min(Math.max(fit, 1) * (window.devicePixelRatio || 1), 3);
  }
  // `?bloom=0` turns the post-process off, for attributing frame cost. The
  // product always has it on; this is a measurement lever, not a setting.
  const bloomEnabled = params.get("bloom") !== "0";
  // THERE IS NO `?rigged=0` (#415, removing what #411 added here). It fell the
  // whole roster back to `player_renderer.ts`'s procedural 2.5D billboards, to
  // attribute frame cost to the ten `THREE.SkinnedMesh` characters and their
  // per-frame bone-matrix uploads versus everything else. That renderer is
  // gone -- the product only ever drew the rigged path (#403 measured
  // `rigged_characters=10, skinned_meshes=10` on hardware) and keeping a second
  // one alive so it could be measured meant keeping a path a rig-build failure
  // could silently drop into. Removed rather than left as a dead query string:
  // a lever that quietly does nothing is worse than no lever. `?bloom=0` and
  // `?ratio=` above are unaffected.
  //
  // See the `?spin=` note in the frame loop below. 0 (the default) skips the
  // spin entirely, so this page behaves exactly as before unless asked.
  const spinMs = Math.max(Number(params.get("spin") ?? "0"), 0);
  // Accumulates the spin loop's counter so it cannot be optimised away, and
  // is deliberately never read for anything else.
  let spinSink = 0;
  const sceneRoot = new SceneRoot(glRenderer, {
    viewport: { w: width, h: height },
    pixelRatio: pixelRatioForWindow(),
    bloom: { enabled: bloomEnabled },
  });
  window.addEventListener("resize", () => {
    glRenderer.setPixelRatio(pixelRatioForWindow());
    sceneRoot.resize({ w: width, h: height });
  });
  // `?combat=1` OPTS THE SESSION INTO THE COMBAT LAYER. Off by default, so
  // this page behaves exactly as it did before unless asked -- same posture
  // as `?bloom=0`/`?ratio=`/`?spin=` above.
  //
  // WHY IT WAS ADDED. `Session::new`'s `combat_enabled` argument defaults to
  // `false` when omitted (`session.rs`: "None/omitted defaults to false"),
  // and this page omitted it, so the session never even built a
  // `CombatMatchState`. Turning it on demonstrably changes the match -- run
  // `scripts/browser_match_harness.py scan` with and without `--combat` and
  // the pose histogram differs from the first few hundred ticks.
  //
  // IT NOW ALSO MAKES COMBAT POSES APPEAR (#441). This comment used to say
  // the opposite, and the reason it was right at the time is worth keeping:
  // `player_pose::select` only considers `combat_stagger`,
  // `combat_knockback`, `combat_guard`, `combat_active`, `combat_windup`,
  // `combat_aim` and `combat_recovery` when it is handed a
  // `FrameCombatModel`, and `gc_wasm`'s `frame_options` (session.rs) built
  // `RenderFrameOptions` with `..Default::default()`, so `options.combat`
  // was always `None` and those seven poses were unreachable in EVERY v2
  // render frame -- not merely rare in this harness. #438 recorded three of
  // them as "never reached in ~22,000 ticks across four seeds" and read that
  // as rarity; that was the cause.
  //
  // What was wrong was the DIAGNOSIS of the remedy, not the observation:
  // this was filed under the out-of-scope JS<->wasm marshalling milestone,
  // when in fact the three fields pose selection reads (`phase`,
  // `forced_state`, `forced_ticks`) were already native Rust state on the
  // wasm side of the wall (`gc_sim::combat_snapshot::CombatPlayerState`).
  // `frame_options` now adapts them in-process via
  // `gc_render::frame::combat_model`. Nothing crosses the boundary for it,
  // and the wire still does not carry the model (`frame_buffer.ts`: "WHAT IS
  // NOT CARRIED: RenderFrame.combat") -- only the numeric `pose_id` column
  // it decides.
  //
  // So with `?combat=1` this page can now show a combat pose, and without it
  // nothing about the frame changes at all.
  //
  // The lever is off by default because the product's own match screen has
  // the same opt-in (`game/screens/match.lua`'s `_opts.combat_enabled`).
  const combatEnabled = params.get("combat") === "1";
  const session = new Session("nebula", "orion", seed, durationSeconds, 99, undefined, combatEnabled, undefined, undefined, undefined);
  session.enableBot(botSeed);

  const raw = __getRawExports() as {
    memory: WebAssembly.Memory;
    render_frame_build: (handle: number, kickFollowSlots: number) => number;
    render_frame_ptr: () => number;
    render_frame_len: () => number;
  };

  const roster = frameBuffer.decodeRoster(session.rosterNumeric(), session.rosterIdsAndNames());

  // The coliseum stadium + true-perspective broadcast camera. On by default
  // (it is the product look this harness exists to evaluate); `?stadium=0`
  // restores the legacy fixed-trapezoid space backdrop for A/B comparison.
  // The Stadium needs the field's real geometry (pitch size, goal rects,
  // crossbar height), which only exists once a session is live -- built from
  // the first decoded frame below, before the loop starts.
  const stadiumEnabled = params.get("stadium") !== "0";

  function frameNow(): frameBufferTypes.RenderFrame {
    // The renderer's release follow-through window, as the roster-slot
    // bitmask the per-frame path takes. Third instance of the same shape
    // `viewState`/`cameraFollow` already have on this page (see the two
    // comments in the loop below): renderer-owned state that only exists if
    // something drives it, owned by `@gc/screens`'s `match.ts` in the real
    // app and by this page here, since it has no screen stack. Without it
    // `kick_follow` is always empty and a shot snaps straight back to the run
    // cycle -- which is exactly what this page would be used to check.
    if (raw.render_frame_build(session.handle, releaseFollow.slotMask(roster.ids)) === 0) {
      throw new Error("match_harness: no live session for this handle");
    }
    // Never cached across ticks: `raw.memory.buffer` is replaced wholesale
    // whenever wasm memory grows, which would leave a stale view detached.
    const words = new Float64Array(raw.memory.buffer, raw.render_frame_ptr(), raw.render_frame_len());
    return frameBuffer.toRenderFrame(frameBuffer.decode(words), roster);
  }

  // A plain accumulator rather than `FixedClock`: this page is not making a
  // determinism claim (the evidence surfaces do that, headless), and keeping
  // the loop obvious matters more here than sharing the catch-up/drop policy.
  // Capped so a long stall cannot spiral into a spike of catch-up ticks.
  const MAX_TICKS_PER_FRAME = 8;
  let accumulator = 0;
  let lastTime = performance.now();
  let framesInWindow = 0;
  let ticksInWindow = 0;
  let simMsInWindow = 0;
  let decodeMsInWindow = 0;
  let renderMsInWindow = 0;
  let populateMsInWindow = 0;
  let windowStart = lastTime;

  // Diagnostics handle. This page exists to be measured, and attributing draw
  // calls needs the scene graph and the renderer -- see the breakdown driver
  // in scripts/. Not a product affordance: nothing under v2/ts reads this.
  //
  // `effects` rides along for one specific reason. It is the only thing this
  // page can reach that would break a deterministic capture: `effects.burst`
  // spawns spark particles from bare `Math.random()` -- deliberately, since
  // it is juice that never feeds back into the simulation, unlike
  // `stadium_prng.ts` whose output must be reproducible. Nothing here drives
  // it today (this page never calls `effects.update`/`consume`/
  // `apply_event_diff`/`confirm_event`/`consume_combat`, so its arrays stay
  // empty and `pitch.ts`'s draw calls emit nothing), but `pitch.ts` already
  // imports it, so wiring match or combat events in here is a plausible
  // future edit. Exposing it lets
  // `scripts/browser_match_harness.py`'s `renderer_state_verdict` refuse a
  // capture with a NAMED cause instead of leaving the next person with a
  // control that fails unpredictably near every shot. Read-only: the driver
  // calls `diagnostics()` and nothing else.
  (globalThis as unknown as { __gcScene?: unknown }).__gcScene = { sceneRoot, glRenderer, THREE, effects };

  if (stadiumEnabled) {
    // Flags first, then the layer: `SceneRoot.render` only routes through the
    // world layer when `camera.perspective_mode` is on (scene.ts), and
    // `pitch.stadium_mode` hands the backdrop/floor/markings/goals over to
    // the stadium (pitch.ts) -- setting one without the other draws either
    // two pitches or none.
    camera.perspective_mode = true;
    pitch.stadium_mode = true;
    // Matches browser_main.ts's product flag set, so a screenshot taken here
    // is representative of the real shot. Unlike the product this harness
    // drives `cameraFollow.update` itself (see the loop below): it renders
    // frames straight from a wasm `Session` and never mounts `@gc/screens`'s
    // `match.ts`, which is what drives the follow camera in the real app.
    pitch.follow_camera = true;
    const stadium = new Stadium({
      field: frameNow().field,
      home_color: HOME_COLOR,
      away_color: AWAY_COLOR,
    });
    sceneRoot.setWorldLayer(stadium);
  }

  stats.status = "running";

  function loop(now: number): void {
    const elapsed = Math.min((now - lastTime) / 1000, 0.25);
    lastTime = now;
    accumulator += elapsed;

    const tSim = performance.now();
    let ticks = 0;
    while (accumulator >= DT && ticks < MAX_TICKS_PER_FRAME) {
      if (session.finished) {
        break;
      }
      // Read the state BEFORE the step it feeds -- `botWire` samples the
      // match as it stands right now, exactly as the Lua benchmark's
      // `bot.input(self.bot, self.state, DT)` does.
      session.step(session.botWire());
      accumulator -= DT;
      ticks += 1;
    }
    simMsInWindow += performance.now() - tSim;
    stats.ticksLastFrame = ticks;
    ticksInWindow += ticks;

    const tDecode = performance.now();
    const frame = frameNow();
    // WITHOUT THIS EVERY CHARACTER STANDS STILL. `player_renderer_3d.poseFor`
    // blends idle/walk/run by `view.speed` and phases the cycle by
    // `view.gait`, and BOTH come from `viewState` -- which is a renderer-owned
    // accumulator, not something the render frame carries. Never updating it
    // leaves every player at speed 0, so the rig holds the idle clip no matter
    // how fast they are actually moving across the pitch. `MatchScreen` and
    // `benchmark.ts` both call this; this page did not, which is why its
    // characters slid around frozen next to the Lua benchmark's running ones.
    const followPlayers = roster.ids.map((id, i) => ({ id, pos: { x: frame.players.x[i] ?? 0, y: frame.players.y[i] ?? 0 } }));
    viewState.update(followPlayers, ticks * DT);
    // Ages and latches the follow-through window from this frame's own event
    // batch -- see `frameNow`'s comment for why this page owns the call.
    releaseFollow.update(
      Array.from({ length: frame.events.count }, (_unused, i) => {
        const slot = frame.events.slot[i];
        const player = slot !== undefined ? roster.ids[slot - 1] : undefined;
        return player !== undefined ? { kind: frame.events.kind[i] ?? "", player } : { kind: frame.events.kind[i] ?? "" };
      }),
      ticks * DT,
    );
    stats.poses = roster.ids.map((_id, i) => frame.players.pose_id[i]);
    stats.leans = roster.ids.map((id) => viewState.get(id)?.lean ?? 0);
    // The follow camera has the same "renderer-owned accumulator nothing else
    // updates" shape the comment above describes for `viewState`: without this
    // its smoothed focus never leaves `undefined`, `cameraFollow.view` returns
    // `undefined`, and `pitch.follow_camera` above silently does nothing --
    // the page would draw the fixed whole-pitch shot while claiming to show
    // the product's framing. In the real app `@gc/screens`'s `match.ts` owns
    // this call (its `updateCameraFollow`); this page has no screen stack, so
    // it drives it directly, from the same decoded frame.
    cameraFollow.update(
      { field: frame.field, ball: { x: frame.ball.x, y: frame.ball.y }, players: followPlayers },
      ticks * DT,
    );
    // Read AFTER `cameraFollow.update` so the view reported for this tick is
    // the one this frame is about to be drawn through, not the previous
    // frame's. See `HarnessStats`'s `playerX` note for why a driver needs
    // these at all; nothing here derives geometry, it only republishes state
    // the loop already has.
    stats.playerX = roster.ids.map((_id, i) => frame.players.x[i] ?? 0);
    stats.playerY = roster.ids.map((_id, i) => frame.players.y[i] ?? 0);
    stats.ballX = frame.ball.x;
    stats.ballY = frame.ball.y;
    stats.fieldW = frame.field.w;
    stats.fieldH = frame.field.h;
    const followView = pitch.follow_camera ? cameraFollow.view(frame.field) : undefined;
    stats.viewX = followView?.x ?? null;
    stats.viewY = followView?.y ?? null;
    stats.viewZoom = followView?.zoom ?? null;
    decodeMsInWindow += performance.now() - tDecode;

    // `?spin=<ms>` burns that many milliseconds of CPU here, BEFORE `tRender`
    // and outside every other accumulator, so it lands in its own `spinMs`
    // bucket and inflates none of the numbers being compared.
    //
    // WHAT IT IS FOR. #403's frame was attributed 26.29 ms of `renderMs` --
    // almost exactly two 13.33 ms refresh periods -- and the claim was that
    // this is one MISSED VSYNC being charged to `render`, not 26 ms of work:
    // the driver blocks inside a GL call once the swap queue is full, and
    // `renderMs` is plain wall-clock around the synchronous `sceneRoot.render`
    // call, so a block lands inside it. That claim was inference, not
    // measurement.
    //
    // This lever tests it directly and destructively. Add a KNOWN amount of
    // work outside `render`, sweep it across the vsync deadline, and watch
    // `renderMs`:
    //
    //   * if `renderMs` jumps discontinuously the moment the frame misses the
    //     deadline -- from a nudge far smaller than 13 ms -- the block is real
    //     and lands inside `render`, and it should then FALL as the spin grows
    //     further, since the two share one fixed 26.6 ms period;
    //   * if `renderMs` just stays flat while the rAF interval doubles, the
    //     block is not inside `render` at all and #403's 26.29 ms means
    //     genuinely doubled work.
    //
    // A sawtooth and a flat line are not close to each other, which is what
    // makes this decidable. Same category as `?ratio=`/`?bloom=`:
    // a measurement lever, never a setting. (`?rigged=` used to be listed here
    // too; #415 removed it along with the renderer it selected -- see the note
    // above `bloomEnabled`.)
    if (spinMs > 0) {
      const spinUntil = performance.now() + spinMs;
      // A read the JIT cannot fold away, so the loop actually burns time.
      let sink = 0;
      while (performance.now() < spinUntil) {
        sink += 1;
      }
      spinSink += sink;
    }

    const tRender = performance.now();
    glRenderer.info.reset();
    const sceneOptions = {
      pitch: { home_color: HOME_COLOR, away_color: AWAY_COLOR },
      now: now / 1000,
    };
    // Split deliberately: `populate` assembles the scene graph and rasterises
    // nothing, so this separates CPU scene-building from GL time. `render`
    // calls `populate` itself, so calling it here would do the work twice --
    // time `populate`, then let `render` redo it, and subtract. The double
    // assembly makes `renderMs` here larger than the product's; the SPLIT is
    // what this is for, not the absolute.
    sceneRoot.populate(frame, sceneOptions);
    populateMsInWindow += performance.now() - tRender;
    sceneRoot.render(frame, sceneOptions);
    renderMsInWindow += performance.now() - tRender;
    stats.drawCalls = glRenderer.info.render.calls;

    framesInWindow += 1;
    stats.tick = session.inputTick;
    stats.timeLeft = frame.hud.time_left;
    stats.score = `${frame.hud.home_score}-${frame.hud.away_score}`;
    if (session.finished) {
      stats.status = "finished";
    }

    const windowSeconds = (now - windowStart) / 1000;
    if (windowSeconds >= 1) {
      stats.fps = Number((framesInWindow / windowSeconds).toFixed(1));
      stats.tps = Number((ticksInWindow / windowSeconds).toFixed(1));
      stats.simMs = Number((simMsInWindow / Math.max(framesInWindow, 1)).toFixed(2));
      stats.decodeMs = Number((decodeMsInWindow / Math.max(framesInWindow, 1)).toFixed(2));
      stats.renderMs = Number((renderMsInWindow / Math.max(framesInWindow, 1)).toFixed(2));
      stats.spinMs = spinMs;
      stats.populateMs = Number((populateMsInWindow / Math.max(framesInWindow, 1)).toFixed(2));
      simMsInWindow = 0;
      decodeMsInWindow = 0;
      renderMsInWindow = 0;
      populateMsInWindow = 0;
      framesInWindow = 0;
      ticksInWindow = 0;
      windowStart = now;
      if (readout !== null) {
        readout.textContent =
          `fps ${stats.fps}   sim ${stats.tps}/s   ticks/frame ${stats.ticksLastFrame}\n` +
          `ms/frame  sim ${stats.simMs}  decode ${stats.decodeMs}  populate ${stats.populateMs}  render ${stats.renderMs}\n` +
          `draw calls ${stats.drawCalls}   tick ${stats.tick}   ${stats.score}   ${stats.timeLeft.toFixed(1)}s left`;
      }
    }

    requestAnimationFrame(loop);
  }

  requestAnimationFrame(loop);
}

main().catch((cause: unknown) => {
  const stats = globalThis.__gcMatchHarness;
  const message = cause instanceof Error ? cause.message : String(cause);
  if (stats !== undefined) {
    stats.status = "error";
    stats.error = message;
  }
  const readout = document.getElementById("stats");
  if (readout !== null) {
    readout.textContent = `error: ${message}`;
  }
  console.error("[match_harness]", cause);
});
