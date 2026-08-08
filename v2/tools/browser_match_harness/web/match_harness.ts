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
import { SceneRoot, frameBuffer, viewState } from "@gc/render";
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
  /** Ticks the most recent render call consumed. >1 means the renderer is
   * behind the simulation, which is also when the shell's known
   * one-sample-per-render-call input bug would double an edge. */
  ticksLastFrame: number;
  tick: number;
  timeLeft: number;
  score: string;
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
  // docs/online/browser_build.md) and what the projection quietly requires.
  //
  // `camera.projectFixed` -- ported character for character from Lua -- puts
  // the viewport factor in the SCREEN POSITION and not in the depth scale:
  //
  //     sx    = vp.w/2 + (wx - field.w/2) * scale * (vp.w / field.w)
  //     scale = far_scale + (near_scale - far_scale) * t
  //
  // Everything sized off that scale (`r = radius * scale`, and so every
  // character, billboard, shadow and reticle derived from it) therefore has
  // NO viewport factor. That is invisible while vp.w == field.w, which is the
  // only case Lua ever runs and the only case the specs cover. Render at the
  // window's native size instead and the pitch stretches while the players
  // stay put -- measured: a 2x viewport moved pitch width 830 -> 1634 px and
  // left character ppm bit-identical at 29.8969.
  //
  // So the drawing buffer stays at field size and CSS scales it up. Fewer
  // pixels, and the invariant the projection assumes holds by construction
  // rather than by luck.
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
    ticksLastFrame: 0,
    tick: 0,
    timeLeft: 0,
    score: "0-0",
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
    const fit = Math.min(window.innerWidth / width, window.innerHeight / height);
    return Math.min(Math.max(fit, 1) * (window.devicePixelRatio || 1), 3);
  }
  const sceneRoot = new SceneRoot(glRenderer, {
    viewport: { w: width, h: height },
    pixelRatio: pixelRatioForWindow(),
  });
  window.addEventListener("resize", () => {
    glRenderer.setPixelRatio(pixelRatioForWindow());
    sceneRoot.resize({ w: width, h: height });
  });
  const session = new Session("nebula", "orion", seed, durationSeconds, 99, undefined, undefined, undefined, undefined, undefined);
  session.enableBot(botSeed);

  const raw = __getRawExports() as {
    memory: WebAssembly.Memory;
    render_frame_build: (handle: number) => number;
    render_frame_ptr: () => number;
    render_frame_len: () => number;
  };

  const roster = frameBuffer.decodeRoster(session.rosterNumeric(), session.rosterIdsAndNames());

  function frameNow(): frameBufferTypes.RenderFrame {
    if (raw.render_frame_build(session.handle) === 0) {
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
  let windowStart = lastTime;

  // Diagnostics handle. This page exists to be measured, and attributing draw
  // calls needs the scene graph and the renderer -- see the breakdown driver
  // in scripts/. Not a product affordance: nothing under v2/ts reads this.
  (globalThis as unknown as { __gcScene?: unknown }).__gcScene = { sceneRoot, glRenderer, THREE };

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
    viewState.update(
      roster.ids.map((id, i) => ({ id, pos: { x: frame.players.x[i] ?? 0, y: frame.players.y[i] ?? 0 } })),
      ticks * DT,
    );
    decodeMsInWindow += performance.now() - tDecode;
    const tRender = performance.now();
    glRenderer.info.reset();
    sceneRoot.render(frame, {
      pitch: { home_color: HOME_COLOR, away_color: AWAY_COLOR },
      now: now / 1000,
    });
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
      simMsInWindow = 0;
      decodeMsInWindow = 0;
      renderMsInWindow = 0;
      framesInWindow = 0;
      ticksInWindow = 0;
      windowStart = now;
      if (readout !== null) {
        readout.textContent =
          `fps ${stats.fps}   sim ${stats.tps}/s   ticks/frame ${stats.ticksLastFrame}\n` +
          `ms/frame  sim ${stats.simMs}  decode ${stats.decodeMs}  render ${stats.renderMs}\n` +
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
