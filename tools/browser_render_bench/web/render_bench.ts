// The v2 side of the #100 ten-player render benchmark, driven in a real
// browser. See this directory's sibling `scripts/browser_render_bench.py`
// (invoked as a driver, mirroring `scripts/browser_online_peers.py`'s
// structure per that task's brief) for how this page is built and read.
//
// WHY A STANDALONE HARNESS PAGE, NOT THE PRODUCT APP SHELL
// (`v2/ts/packages/app/src/browser_main.ts`)
//
// The product shell requires clicking through Title -> Squad -> Formation ->
// Tactic before a real match exists, and even then drives exactly one input
// slot from a live keyboard/gamepad capture. This page needs a deterministic,
// scriptable, no-UI path straight to a running match -- the same reasoning
// `v2/tools/browser_online_match/web/online_peer.ts`'s header gives for being
// "deliberately NOT @gc/app/@gc/screens/@gc/online wired up as a real
// client". It imports the already-built wasm web artifact by relative path
// (this task may read `dist/pkg-web`, not edit `packages/wasm/src`) plus
// `@gc/render`'s public package surface (`SceneRoot`, `frameBuffer`,
// `viewState`, and the `Benchmark` class ported from `game/render/
// benchmark.lua` -- see that file for the ported class's own doc, which
// explicitly says wiring a real driver/renderer is "for whoever builds that
// browser-build glue"; that is this file).
//
// LIVE MATCH, NOT AN INERT ONE. `crates/gc-wasm/src/session.rs`'s
// `Session::step` doc says only canonical slot `home_1` is read from the
// caller's wire; every other slot -- both full teams bar one player -- is
// overwritten by this session's own declared bot fills before the simulation
// ever sees it ("without that fill, every non-local slot would forever
// receive wire's neutral row, which is the whole-match bug this producer
// exists to fix"). This page always sends an all-neutral wire (nobody drives
// `home_1` either), so this is closer to an attract-mode match than a human
// game, but it is a LIVE one: nine of ten players are bot-controlled
// regardless, and the tenth's neutral input does not prevent goals, tackles
// or the ball moving. `liveness` in this page's report (score/time/tick) is
// how the driver script confirms that, rather than trusting this comment.
//
// UPDATE VS DRAW, KEPT SEPARATE, exactly like the Lua original: `update()`
// below costs exactly one `Session.step` (the wasm sim tick, nothing else).
// `draw()` costs building this tick's `RenderFrame` (`render_frame_build` +
// decode, the same cost `render_frame.build` pays inside Lua's `bloom.draw`
// call) plus one `SceneRoot.render` (three.js). Both run exactly once per
// loop iteration, and frames are PINNED (a fixed count), not wall-clock --
// the file this ports states why: with the browser producing frames far
// faster than 60 Hz, a wall-clock window would let the two builds simulate a
// different number of ticks and draw a different match state, so the
// comparison would not be measuring the same scene.
//
// `players()` (feeding `viewState.update`, gait/lean animation state only --
// it plays no part in either measured gate) reads the PREVIOUS frame's
// decoded positions rather than issuing a second wasm call during `update()`.
// That is a deliberate one-tick-stale approximation, called out here because
// it is a place this harness diverges from the Lua original (which reads
// live sim state directly, free): materializing a fresh position readout
// inside `update()` would leak `draw`-side wasm/decode cost into the
// `update` timing this file exists to keep honest. The approximation cannot
// affect either measured gate.

import init, { Session, __getRawExports } from "../../../ts/packages/wasm/dist/pkg-web/gc_wasm.js";
import * as THREE from "three";
import { SceneRoot, frameBuffer, Benchmark, evaluate, emit } from "@gc/render";
import type { frameBufferTypes, benchmarkTypes, sceneTypes } from "@gc/render";

// Mirrors `@gc/input`'s `inputSample.VERSION` (2) and `browser_sim_host.ts`'s
// `INPUT_SLOT_COUNT`/`NEUTRAL_SLOT_WIRE` -- reimplemented locally rather than
// imported, matching `online_peer.ts`'s stated policy of not depending on
// another package's src for a five-line constant/helper (this directory is
// outside the pnpm workspace; see this task's vite.config.ts for the aliases
// it does take).
const INPUT_FRAME_VERSION = 2;
const INPUT_SLOT_COUNT = 8;
const NEUTRAL_SLOT_WIRE = "0,0,0,0";

// Same placeholder team colors `browser_main.ts` uses (real per-team
// presentation palette has no TypeScript-reachable route yet -- see that
// file's own comment).
const HOME_COLOR: readonly [number, number, number] = [0.35, 0.75, 1.0];
const AWAY_COLOR: readonly [number, number, number] = [1.0, 0.55, 0.25];

function neutralWire(tick: number): string {
  const groups = new Array<string>(INPUT_SLOT_COUNT).fill(NEUTRAL_SLOT_WIRE);
  return [String(INPUT_FRAME_VERSION), String(tick), ...groups].join("|");
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

type Phase = "booting" | "running" | "done" | "error";

interface HarnessLiveness {
  readonly score_home: number;
  readonly score_away: number;
  readonly time_left: number;
  readonly finished: boolean;
  readonly final_tick: number;
}

/**
 * Draw calls split by what issued them, so "504 for ten players" is
 * diagnosable rather than just alarming (this task's problem 3). Computed
 * ONCE, after the timed loop finishes, from a fresh, non-composited
 * re-render of the final captured frame -- see `computeDrawCallBreakdown`'s
 * doc for exactly what "raw" means and why this cannot run inside the timed
 * `update`/`draw` samples.
 */
interface DrawCallBreakdown {
  /** Rigged `THREE.SkinnedMesh` characters only (`pitchGroup` children tagged `userData.riggedCharacter`), raw render, no bloom. */
  readonly characters: number;
  /** `pitchGroup` minus characters: arena backdrop/frame, markings, goals, effects, combat -- whatever this particular captured frame actually had live. Raw render, no bloom. */
  readonly pitch: number;
  /** `hudGroup`, raw render, no bloom. Always 0 in this harness -- see `hud_exercised`. */
  readonly hud: number;
  /** `total` minus the three raw buckets above: bright-pass extract + N blur passes + composite, i.e. everything `Bloom.draw`'s `EffectComposer` adds beyond one plain scene render. Not directly measured -- see method note. */
  readonly bloom_overhead: number;
  /** The real production path: `SceneRoot.render` (populate + bloom composite) on the same frame. Matches `result.draw_calls_mean`/`_max` in magnitude (same call), not identical (this is one extra sample, not the timed population). */
  readonly total: number;
  /** This harness never passes `SceneRenderOptions.hud`, so `hudGroup` is always empty (`SceneRoot.populate`'s own `else` branch clears it) -- `hud` above is honestly 0, not "HUD costs nothing". */
  readonly hud_exercised: false;
  readonly method: string;
}

interface HarnessReport {
  readonly result: benchmarkTypes.BenchmarkResult;
  readonly pass: boolean;
  readonly failures: readonly string[];
  readonly liveness: HarnessLiveness;
  readonly gl_renderer_string: string | null;
  readonly gl_vendor_string: string | null;
  readonly software_rendering_guess: boolean;
  readonly draw_call_breakdown: DrawCallBreakdown | { readonly error: string } | null;
}

interface HarnessState {
  status: Phase;
  error: string | null;
  report: HarnessReport | null;
}

declare global {
  interface Window {
    __gcRenderBench?: HarnessState;
  }
}

function marker(kind: string, fields: Record<string, string | number>): string {
  const parts = ["GC_RENDER_BENCH", kind];
  for (const [key, value] of Object.entries(fields)) {
    parts.push(`${key}=${value}`);
  }
  return parts.join("|");
}

/**
 * Splits one frame's draw calls into characters / pitch / bloom / HUD, run
 * ONCE after the timed loop -- never inside `bench.update()`/`bench.draw()`,
 * which would add extra GPU work into the exact samples this file exists to
 * keep honest (see the file header's UPDATE VS DRAW section). This is a
 * supplementary diagnostic read, not a third timed metric.
 *
 * METHOD, and why it is not just "reset info around SceneRoot.render three
 * times": `SceneRoot.render` is one call -- `populate` (assembles
 * `pitchGroup`/`hudGroup`, no rasterizing) then ONE `Bloom.draw`, which
 * itself renders the WHOLE scene (`pitchGroup` + `hudGroup` together) in a
 * single `EffectComposer` pass plus several more fullscreen-quad passes
 * (bright-pass extract, blur, composite). There is no per-object-category
 * hook inside that pipeline to intercept, and this file does not own (and
 * this task does not touch) `scene.ts`/`pitch.ts`/`bloom.ts` to add one.
 *
 * So this measures three RAW (non-bloom, non-composited) full-scene renders
 * instead, isolating buckets by toggling `Object3D.visible` on
 * `pitchGroup`/`hudGroup`'s children from the OUTSIDE, using only what those
 * classes already expose publicly (`scene`, `camera`, `pitchGroup`,
 * `hudGroup` are all `readonly` public fields) plus the one existing tag
 * `pitch.ts`'s `riggedCharacterObject` already sets on every rigged
 * character wrapper (`userData.riggedCharacter = true` -- see that
 * function). `characters` and `pitch` are therefore true, isolated,
 * uncomposited draw-call counts. `bloom_overhead` is NOT independently
 * measured the same way (there is no "just the post-process passes,
 * nothing else" mode to call into) -- it is `total - (characters + pitch +
 * hud)`, i.e. everything the real composited `total` costs beyond one plain
 * scene render, which is exactly what `EffectComposer` adds on top of
 * `renderer.render(scene, camera)` (see `Bloom.draw`'s own two-branch body:
 * the depth-buffer-present branch is `composer.render()`, nothing else).
 * `total` itself IS the real production path (`SceneRoot.render`, bloom
 * included), so it is not a diagnostic approximation -- it is drawn from
 * the same call the timed loop uses.
 *
 * Visibility is restored before the `total` render specifically so this
 * diagnostic pass cannot leak into the one number meant to match the timed
 * samples, regardless of whether `pitch.draw`'s pooling reasserts
 * `.visible` on every child every call (its own doc says objects are
 * pooled/reused across frames, not rebuilt, so this cannot be assumed).
 */
function computeDrawCallBreakdown(
  renderer: THREE.WebGLRenderer,
  sceneRoot: SceneRoot,
  frame: frameBufferTypes.RenderFrame,
  options: sceneTypes.SceneRenderOptions,
): DrawCallBreakdown {
  sceneRoot.populate(frame, options);

  const pitchChildren = sceneRoot.pitchGroup.children;
  const hudChildren = sceneRoot.hudGroup.children;
  const originalPitchVisible = pitchChildren.map((child) => child.visible);
  const originalHudVisible = hudChildren.map((child) => child.visible);

  const isCharacter = (obj: THREE.Object3D): boolean => obj.userData["riggedCharacter"] === true;

  function rawCallsWith(setVisibility: () => void): number {
    setVisibility();
    renderer.info.reset();
    renderer.render(sceneRoot.scene, sceneRoot.camera);
    return renderer.info.render.calls;
  }

  const characters = rawCallsWith(() => {
    for (const child of pitchChildren) child.visible = isCharacter(child);
    for (const child of hudChildren) child.visible = false;
  });
  const pitch = rawCallsWith(() => {
    for (const child of pitchChildren) child.visible = !isCharacter(child);
    for (const child of hudChildren) child.visible = false;
  });
  const hud = rawCallsWith(() => {
    for (const child of pitchChildren) child.visible = false;
    for (const child of hudChildren) child.visible = true;
  });

  pitchChildren.forEach((child, i) => {
    child.visible = originalPitchVisible[i] ?? true;
  });
  hudChildren.forEach((child, i) => {
    child.visible = originalHudVisible[i] ?? true;
  });

  renderer.info.reset();
  sceneRoot.render(frame, options);
  const total = renderer.info.render.calls;

  return {
    characters,
    pitch,
    hud,
    bloom_overhead: Math.max(0, total - characters - pitch - hud),
    total,
    hud_exercised: false,
    method:
      "characters/pitch/hud are raw (non-bloom) renders isolated by toggling Object3D.visible " +
      "on pitchGroup/hudGroup children, run once after the timed loop, not part of the timed " +
      "update/draw samples. bloom_overhead = total - (characters+pitch+hud), i.e. what the real " +
      "composited render (total) costs beyond one plain scene render. total is the real " +
      "production SceneRoot.render call on the same frame.",
  };
}

/** Reads `WEBGL_debug_renderer_info` if the browser exposes it -- the only
 * reliable way from page JS to tell software (SwiftShader/llvmpipe) from
 * hardware GL apart; `gl.getParameter(gl.RENDERER)` alone is masked to a
 * generic ANGLE string in Chrome. */
function glRendererInfo(gl: WebGL2RenderingContext | WebGLRenderingContext): { renderer: string | null; vendor: string | null } {
  const ext = gl.getExtension("WEBGL_debug_renderer_info");
  if (!ext) {
    return { renderer: null, vendor: null };
  }
  const renderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) as string;
  const vendor = gl.getParameter(ext.UNMASKED_VENDOR_WEBGL) as string;
  return { renderer, vendor };
}

async function main(): Promise<void> {
  const params = new URLSearchParams(location.search);
  const frames = Number(params.get("frames") ?? "1800");
  const warmupFrames = Number(params.get("warmup") ?? "180");
  const seed = Number(params.get("seed") ?? "20260803");
  const width = Number(params.get("width") ?? "960");
  const height = Number(params.get("height") ?? "540");
  const yieldEvery = Number(params.get("yield_every") ?? "150");

  const state: HarnessState = { status: "booting", error: null, report: null };
  window.__gcRenderBench = state;
  console.log(marker("booted", { frames, warmup: warmupFrames, seed, width, height }));

  try {
    await init();

    const canvas = document.getElementById("gl-canvas") as HTMLCanvasElement | null;
    if (canvas === null) {
      throw new Error("render_bench: #gl-canvas is missing");
    }
    canvas.width = width;
    canvas.height = height;

    const glRenderer = new THREE.WebGLRenderer({ canvas, antialias: true, powerPreference: "high-performance" });
    // `Bloom.draw` (`SceneRoot.render`'s composite step) issues several
    // internal `renderer.render()` calls of its own (bright-pass extract,
    // N blur passes, composite) via three.js's `EffectComposer`, and EACH
    // one resets `info.render.calls` at ITS OWN start when `autoReset` is
    // left at its default `true` -- so reading the counter after the whole
    // composite would silently report only the LAST internal pass's calls,
    // not the frame's real total. Disabling autoReset and resetting once,
    // by hand, right before this file's own `sceneRoot.render()` call (see
    // `rendererPort.draw()` below), makes the counter accumulate across
    // every pass in between.
    glRenderer.info.autoReset = false;
    const gl = glRenderer.getContext();
    const { renderer: glRendererString, vendor: glVendorString } = glRendererInfo(gl);
    // A pragmatic heuristic, not a certainty: SwiftShader (Chrome's software
    // ANGLE backend) and llvmpipe/Mesa software rasterizers both self-report
    // through this string. This page reports the raw string too, so a
    // reader who doubts the heuristic can judge for themselves.
    const softwareGuess = /swiftshader|llvmpipe|softpipe|software/i.test(glRendererString ?? "");
    console.log(marker("gl_info", { renderer: JSON.stringify(glRendererString ?? "?"), vendor: JSON.stringify(glVendorString ?? "?"), software_guess: String(softwareGuess) }));

    const sceneRoot = new SceneRoot(glRenderer, { viewport: { w: width, h: height } });

    // Long enough that the measured window never runs into full time --
    // mirrors `benchmark.lua`'s own `(warmup_frames + frames) * DT + 60`.
    const DT = 1 / 60;
    const durationSeconds = (warmupFrames + frames) * DT + 60;
    const session = new Session("nebula", "orion", seed, durationSeconds, 99, undefined, undefined, undefined, undefined, undefined);

    let lastPositions: { readonly x: readonly number[]; readonly y: readonly number[] } | undefined;
    let sessionFinishedEarly = false;

    // `Session`'s per-frame render payload crosses through the SAME raw
    // exports `browser_sim_host.ts` uses (`__getRawExports()` ->
    // `render_frame_build(handle, kickFollowSlots)` / `render_frame_ptr()` /
    // `render_frame_len()` -> a `Float64Array` view into wasm linear memory,
    // decoded fresh every call -- never cached across ticks, since
    // `raw.memory.buffer` is replaced wholesale whenever wasm memory grows).
    const raw = __getRawExports() as {
      memory: WebAssembly.Memory;
      render_frame_build: (handle: number, kickFollowSlots: number) => number;
      render_frame_ptr: () => number;
      render_frame_len: () => number;
    };

    function rosterOnce(): frameBufferTypes.DecodedRenderFrameRoster {
      const numeric = session.rosterNumeric();
      const strings = session.rosterIdsAndNames();
      return frameBuffer.decodeRoster(numeric, strings);
    }
    const roster = rosterOnce();

    function frameNow(): frameBufferTypes.RenderFrame {
      // Always 0: this page measures frame COST, and the release
      // follow-through window would only add a pose selection whose cost is
      // identical to the locomotion pose it currently resolves to. Declared
      // and passed explicitly rather than relied on as a missing-argument
      // coercion, so this cast cannot drift from the real ABI unnoticed.
      const ok = raw.render_frame_build(session.handle, 0);
      if (ok === 0) {
        throw new Error("render_bench: no live session for this handle");
      }
      const ptr = raw.render_frame_ptr();
      const len = raw.render_frame_len();
      const words = new Float64Array(raw.memory.buffer, ptr, len);
      const decoded = frameBuffer.decode(words);
      return frameBuffer.toRenderFrame(decoded, roster);
    }

    const driver: benchmarkTypes.BenchmarkFixedTimestepDriver = {
      step(): boolean {
        if (session.finished) {
          sessionFinishedEarly = true;
          return false;
        }
        session.step(neutralWire(session.inputTick));
        return !session.finished;
      },
      players(): readonly benchmarkTypes.BenchmarkPlayer[] {
        if (lastPositions === undefined) {
          return [];
        }
        const positions = lastPositions;
        return roster.ids.map((id, i) => ({
          id,
          is_keeper: roster.is_keeper[i] ?? false,
          pos: { x: positions.x[i] ?? 0, y: positions.y[i] ?? 0 },
        }));
      },
    };

    const rendererPort: benchmarkTypes.BenchmarkRenderer = {
      draw(): benchmarkTypes.BenchmarkFrameResult {
        const frame = frameNow();
        lastPositions = { x: frame.players.x, y: frame.players.y };
        // See the `autoReset = false` comment above: reset by hand right
        // before the one call that assembles and composites this whole
        // frame, so `info.render.calls` afterward is this frame's TOTAL
        // draw calls (pitch + bloom's internal passes), not just the last
        // internal pass's.
        glRenderer.info.reset();
        sceneRoot.render(frame, {
          pitch: { home_color: HOME_COLOR, away_color: AWAY_COLOR },
          now: performance.now() / 1000,
        });
        const info = glRenderer.info.render;
        // SAMPLED, not assumed -- was hardcoded `true` before this task,
        // which made the `rigged_active` gate (both here and in
        // scripts/browser_render_bench.py's cross-build check) meaningless:
        // it would report "rigged" even if every character had silently
        // fallen back to a procedural billboard. That fallback no longer
        // exists (#415 deleted it; `pitch.draw` throws instead of
        // downgrading), but this stays a SAMPLE rather than a constant: the
        // point is to report what the assembled object graph actually holds,
        // not what the code is believed to produce. `pitch.ts`'s
        // `riggedCharacterObject` tags every rigged character wrapper it
        // creates with `userData.riggedCharacter = true` (see that
        // function); checking for at least one such child in `pitchGroup`
        // AFTER this frame's `populate()` ran is a direct read of what was
        // actually just composited, the same "sampled rather than assumed"
        // contract `BenchmarkFrameResult.rigged_active`'s own doc comment
        // states.
        const riggedActive = sceneRoot.pitchGroup.children.some(
          (child) => child.userData["riggedCharacter"] === true,
        );
        return {
          rigged_active: riggedActive,
          draw_calls: info.calls,
        };
      },
    };

    const bench = new Benchmark({
      rigged: true,
      seed,
      frames,
      warmup_frames: warmupFrames,
      width,
      height,
      clock: () => performance.now() / 1000,
      simulation: driver,
      renderer: rendererPort,
      finalStateHash: () => session.snapshotHash(),
      environment: {
        runtime_version: `three.js r${THREE.REVISION}`,
        ...(glRendererString !== null ? { gpu_name: glRendererString } : {}),
        ...(glVendorString !== null ? { gpu_vendor: glVendorString } : {}),
      },
    });

    state.status = "running";
    console.log(marker("running", { frames, warmup: warmupFrames }));

    let iteration = 0;
    while (!bench.finished()) {
      bench.update();
      bench.draw();
      iteration += 1;
      if (iteration % yieldEvery === 0) {
        await sleep(0);
      }
      if (iteration > (frames + warmupFrames) * 4 + 10000) {
        // Defensive cap: should never trigger (session duration budgets 60s
        // of slack past the measured window), but a runaway loop in a
        // headless tab is worse than a clear failure.
        throw new Error(`render_bench: loop exceeded ${iteration} iterations without finishing`);
      }
    }

    const result = bench.result();
    const { pass, failures } = evaluate(result);
    emit(result);

    // Run ONCE, after the timed loop -- never inside it -- on one more
    // freshly-built frame of the match's final state. See
    // `computeDrawCallBreakdown`'s own doc for why this needs three extra
    // raw renders plus the real composited one, and why that cost has no
    // business inside `result.draw`'s timed samples. A failure here must not
    // cost the (already-collected, already-valid) timed percentiles, so it
    // is caught and reported as an explicit error field rather than thrown.
    let drawCallBreakdown: DrawCallBreakdown | { readonly error: string } | null;
    try {
      const finalFrame = frameNow();
      drawCallBreakdown = computeDrawCallBreakdown(glRenderer, sceneRoot, finalFrame, {
        pitch: { home_color: HOME_COLOR, away_color: AWAY_COLOR },
        now: performance.now() / 1000,
      });
    } catch (error) {
      drawCallBreakdown = { error: String(error) };
      console.error(marker("draw_call_breakdown_error", { message: JSON.stringify(String(error)) }));
    }

    const report: HarnessReport = {
      result,
      pass,
      failures,
      liveness: {
        score_home: session.scoreHome,
        score_away: session.scoreAway,
        time_left: session.timeLeft,
        finished: session.finished,
        final_tick: session.inputTick,
      },
      gl_renderer_string: glRendererString,
      gl_vendor_string: glVendorString,
      software_rendering_guess: softwareGuess,
      draw_call_breakdown: drawCallBreakdown,
    };
    state.report = report;
    state.status = "done";
    console.log(
      marker("done", {
        pass: String(pass),
        session_finished_early: String(sessionFinishedEarly),
        score_home: session.scoreHome,
        score_away: session.scoreAway,
        update_p95: result.update.p95_ms.toFixed(3),
        draw_p95: result.draw.p95_ms.toFixed(3),
      }),
    );
    if (drawCallBreakdown !== null && !("error" in drawCallBreakdown)) {
      console.log(
        marker("draw_call_breakdown", {
          characters: drawCallBreakdown.characters,
          pitch: drawCallBreakdown.pitch,
          hud: drawCallBreakdown.hud,
          bloom_overhead: drawCallBreakdown.bloom_overhead,
          total: drawCallBreakdown.total,
        }),
      );
    }
  } catch (error) {
    state.status = "error";
    state.error = String(error) + (error instanceof Error && error.stack ? `\n${error.stack}` : "");
    console.error(marker("error", { message: JSON.stringify(String(error)) }));
  }
}

void main();
