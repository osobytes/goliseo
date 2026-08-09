// Ported from game/render/benchmark.lua.
//
// The #100 ten-player render benchmark.
//
// Answers one question with evidence instead of estimate: does the rigged
// player renderer hold the omp0 frame gates with a full match on screen, on
// every runtime we ship to?
//
// WHAT THIS MEASURES, AND WHY IT IS NOT WHAT #100 ANTICIPATED
//
// #100 was written against a Menori/glTF path doing GPU skinning, so its
// metric list leads with GLB conversion, joint uploads and skeleton
// evaluation. That path was never built. The renderer that exists
// (game/render/rig3d/, now v2/ts/packages/render/src/rig3d/) generates
// meshes in code and draws one RIGID PART PER BONE with its own model
// matrix -- there is no skinning, no GLB, and no joint upload at all.
//
// That inverts the expected bottleneck. Skinned characters cost 1-2 draw
// calls each; this costs one per part, so ten players is several hundred
// draw calls and several hundred uniform sends per frame. Draw-call count is
// therefore a first-class metric here rather than a footnote.
//
// FAIRNESS RULES, all of which exist so the two renderers are comparable:
//   * One fixture: same seed, same bot, same fixed timestep, same duration,
//     same viewport. The renderer choice is the ONLY difference.
//   * Warm-up is discarded. Shader compilation, mesh generation and the
//     first GC cycle are startup costs, and #100 wants them measured
//     separately from steady state.
//   * Update and draw are timed separately, because the gates are separate.
//   * The simulation hash is captured at the end. Presentation must not
//     reach the simulation, and a matching hash across renderer choices is
//     what proves it -- a fast frame that changed the game is not a pass.
//
// Vsync is disabled by the caller for the measured run. With vsync on,
// frame time is pinned to the refresh rate and reports 16.67 ms whether the
// renderer has 90% headroom or none, which would make the whole exercise
// say nothing.
//
// Boundary note (v2/README.md rule 6.7 and §1): the Lua original wires
// together `sim.bot` + `sim.match` + `sim.fixed_clock` + `sim.metrics`
// (Rust-owned, `crates/gc-sim`), `data.arenas`/`data.teams` (Rust-owned,
// `crates/gc-data`), and `game.render.bloom`/`pitch`/`player_renderer_3d`
// (out of this task's scope -- see the top-level task's "do not port" list;
// `render/frame.lua`'s payload builder is also Rust-owned,
// `crates/gc-render`). None of that glue exists yet, and v2/README.md §1
// says not to build it in this milestone ("the glue that makes a playable
// browser build ... is a separate milestone. Do not build it, and do not
// block on it."). The class below keeps the Lua original's structure,
// method names, and (pure) statistics/gate-evaluation logic verbatim, and
// takes every one of those Lua dependencies as an explicit injected
// parameter instead of a `require`/`love.*` call -- the same pattern
// `love.timer.getTime()` becomes an injected `clock()` function. Wiring real
// implementations of `BenchmarkFixedTimestepDriver`/`BenchmarkRenderer` is
// for whoever builds that browser-build glue.

import { viewState, type ViewStatePlayer } from "./view_state.ts";
import * as animator from "./rig3d/animator.ts";

export interface BenchmarkGates {
  readonly update_p95_ms: number;
  readonly update_max_ms: number;
  readonly draw_p95_ms: number;
  readonly draw_max_ms: number;
  readonly slow_frame_ms: number;
  readonly slow_frames_per_60s: number;
  readonly stall_frame_ms: number;
}

export interface BenchmarkDeltaBudget {
  readonly update_p95_ms: number;
  readonly draw_p95_ms: number;
}

export interface BenchmarkSummary {
  readonly samples: number;
  readonly mean_ms: number;
  readonly p50_ms: number;
  readonly p95_ms: number;
  readonly p99_ms: number;
  readonly max_ms: number;
  readonly over_16_67: number;
  readonly over_33: number;
  readonly over_250: number;
}

// Thresholds from docs/online/omp0_acceptance.md. Named rather than inlined
// so the report and the gate cannot drift apart.
export const GATES: BenchmarkGates = {
  update_p95_ms: 8,
  update_max_ms: 33,
  draw_p95_ms: 8,
  draw_max_ms: 33,
  slow_frame_ms: 33,
  slow_frames_per_60s: 3,
  stall_frame_ms: 250,
};

// Feature-delta budget for the rigged renderer over the procedural baseline.
export const DELTA_BUDGET: BenchmarkDeltaBudget = { update_p95_ms: 1, draw_p95_ms: 2 };

function must<T>(value: T | undefined): T {
  if (value === undefined) {
    throw new Error("expected a defined value");
  }
  return value;
}

/** @param values sorted ascending; the caller sorts once and reuses. `q` is 0..1. */
function percentile(values: readonly number[], q: number): number {
  if (values.length === 0) {
    return 0;
  }
  // Nearest-rank on a sorted copy.
  const rank = Math.max(1, Math.ceil(q * values.length));
  return must(values[Math.min(rank, values.length) - 1]);
}

function countAbove(values: readonly number[], threshold: number): number {
  let n = 0;
  for (const v of values) {
    if (v > threshold) {
      n += 1;
    }
  }
  return n;
}

/** @param samples seconds */
function summarise(samples: readonly number[]): BenchmarkSummary {
  const ms = samples.map((v) => v * 1000);
  const sorted = [...ms].sort((a, b) => a - b);
  let total = 0;
  for (const v of ms) {
    total += v;
  }
  return {
    samples: ms.length,
    mean_ms: ms.length > 0 ? total / ms.length : 0,
    p50_ms: percentile(sorted, 0.5),
    p95_ms: percentile(sorted, 0.95),
    p99_ms: percentile(sorted, 0.99),
    max_ms: sorted.length > 0 ? must(sorted[sorted.length - 1]) : 0,
    over_16_67: countAbove(ms, 1000 / 60),
    over_33: countAbove(ms, GATES.slow_frame_ms),
    over_250: countAbove(ms, GATES.stall_frame_ms),
  };
}

// Mirrors `sim/fixed_clock.lua`'s `TICK_SECONDS` (`1 / TICK_RATE`, `TICK_RATE`
// = 60 by default). Not imported -- that module is Rust-owned
// (`crates/gc-sim`) -- so the well-known constant is restated here, the same
// way other ported presentation modules avoid a `require("sim.fixed_clock")`.
const DT = 1 / 60;

export interface BenchmarkPlayer extends ViewStatePlayer {
  readonly id: string;
  readonly is_keeper: boolean;
}

/**
 * Everything the Lua original reached via `sim.bot` + `sim.match` +
 * `sim.fixed_clock` + `sim.metrics`, collapsed to the two things this class
 * actually needs from a running match: advance one frame, and read the
 * player positions `view_state.update` derives cadence/lean/speed from.
 */
export interface BenchmarkFixedTimestepDriver {
  /** Runs the frame's fixed-timestep sub-steps (`fixed_clock.step` + `match.step` + `metrics.observe`, one or more ticks). Returns whether the match should keep running. */
  step(): boolean;
  /** The live match's players, for `view_state.update`. */
  players(): readonly BenchmarkPlayer[];
}

export interface BenchmarkFrameResult {
  readonly draw_calls?: number;
  readonly texture_memory_bytes?: number;
  /**
   * Which renderer actually ran, sampled rather than assumed -- the gate
   * fails on a mismatch, so a silent fallback (e.g. no depth buffer) can
   * never be reported as a pass.
   */
  readonly rigged_active: boolean;
}

/** Replaces `bloom.draw(() => pitch.draw(render_frame.build(...), ...))`, all out of this package's scope. */
export interface BenchmarkRenderer {
  draw(): BenchmarkFrameResult;
}

export interface BenchmarkEnvironmentInfo {
  /** Replaces `love.getVersion()`. */
  readonly runtime_version: string;
  /** Replace `love.graphics.getRendererInfo()`'s name/vendor/device. */
  readonly gpu_name?: string;
  readonly gpu_vendor?: string;
  readonly gpu_device?: string;
}

export interface BenchmarkOptions {
  readonly rigged: boolean;
  readonly seed?: number;
  /** Measured in FRAMES, not wall seconds -- see the file header's fairness rules. */
  readonly frames?: number;
  readonly warmup_frames?: number;
  readonly width?: number;
  readonly height?: number;
  /** Replaces `love.timer.getTime()`. Seconds, monotonic. */
  readonly clock: () => number;
  readonly simulation: BenchmarkFixedTimestepDriver;
  readonly renderer: BenchmarkRenderer;
  /** Replaces `collectgarbage("count")`; no standardized TS/browser equivalent exists, so this is optional. */
  readonly heapKb?: () => number;
  /** Replaces `match_snapshot.hash(match_snapshot.capture(self.state))`, computed once at `finish()`. */
  readonly finalStateHash: () => string;
  readonly environment: BenchmarkEnvironmentInfo;
}

export interface BenchmarkResult {
  readonly renderer: "rigged" | "procedural";
  // Requested vs actually used. These diverging is the failure this
  // benchmark is most likely to hit and least likely to notice.
  readonly rigged_requested: boolean;
  readonly rigged_active: boolean;
  readonly seed: number;
  readonly warmup_frames: number;
  readonly measured_frames: number;
  // Simulated seconds the measured window covers, which is what the
  // per-60-second pacing gate is written against.
  readonly measured_seconds: number;
  readonly viewport: { readonly w: number; readonly h: number };
  readonly players: number;
  readonly update: BenchmarkSummary;
  readonly draw: BenchmarkSummary;
  readonly frame: BenchmarkSummary;
  readonly draw_calls_mean: number;
  readonly draw_calls_max: number;
  readonly texture_memory_bytes?: number;
  readonly heap_warm_kb?: number;
  readonly heap_end_kb?: number;
  readonly final_state_hash?: string;
  readonly environment: BenchmarkEnvironmentInfo;
  readonly raw_update_us: readonly number[];
  readonly raw_draw_us: readonly number[];
  readonly raw_frame_us: readonly number[];
}

/** Builds the fixture. `rigged` selects the renderer under test; everything else is identical between runs by construction. */
export class Benchmark {
  readonly rigged: boolean;
  readonly seed: number;
  readonly frames: number;
  readonly warmup_frames: number;
  readonly viewport: { readonly w: number; readonly h: number };

  private readonly clock: () => number;
  private readonly simulation: BenchmarkFixedTimestepDriver;
  private readonly renderer: BenchmarkRenderer;
  private readonly heapKb: (() => number) | undefined;
  private readonly finalStateHashFn: () => string;
  private readonly environment: BenchmarkEnvironmentInfo;

  private readonly updateSamples: number[] = [];
  private readonly drawSamples: number[] = [];
  private readonly frameSamples: number[] = [];
  private readonly drawCalls: number[] = [];
  private textureMemory: number | undefined;
  // Which pose ids the fixture actually exercised. Reported rather than
  // asserted: a benchmark that silently covered only locomotion would look
  // like a pass while measuring a third of the work.
  private readonly posesSeen = new Set<"keeper" | "outfield">();

  private frameIndex = 0;
  private measuredFrames = 0;
  private warmed = false;
  private done = false;
  private lastFrameAt: number | undefined;
  private warmMemoryKb: number | undefined;
  private endMemoryKb: number | undefined;
  private finalHash: string | undefined;
  private riggedActiveLastFrame = false;

  constructor(options: BenchmarkOptions) {
    this.rigged = options.rigged;
    this.seed = options.seed ?? 20260803;
    // 60 s at 60 Hz. With vsync off a fast renderer produces frames far
    // quicker than 60 Hz; pinning frames (not a wall-clock window) pins the
    // match content, so both runs draw the identical sequence of game
    // states and only frame *duration* varies -- see the file header.
    this.frames = options.frames ?? 3600;
    this.warmup_frames = options.warmup_frames ?? 300; // 5 s at 60 Hz
    this.viewport = { w: options.width ?? 960, h: options.height ?? 540 };

    this.clock = options.clock;
    this.simulation = options.simulation;
    this.renderer = options.renderer;
    this.heapKb = options.heapKb;
    this.finalStateHashFn = options.finalStateHash;
    this.environment = options.environment;

    // Both halves of per-player presentation state: `viewState`'s derived
    // gait/lean/speed, and the animator's stance crossfades. A benchmark run
    // must start from a clean pose for the same reason it starts from clean
    // motion.
    viewState.reset();
    animator.reset();
  }

  warming(): boolean {
    return !this.warmed;
  }

  // Exactly one fixed-timestep simulation tick, timed. Rendering is timed
  // separately in draw(), because the omp0 gates budget them separately.
  update(): void {
    if (this.done) {
      return;
    }
    const now = this.clock();
    if (this.lastFrameAt !== undefined && this.warmed) {
      this.frameSamples.push(now - this.lastFrameAt);
    }
    this.lastFrameAt = now;

    const started = this.clock();
    this.simulation.step();
    const updateSeconds = this.clock() - started;

    viewState.update(this.simulation.players(), DT);

    this.frameIndex += 1;
    if (!this.warmed) {
      if (this.frameIndex >= this.warmup_frames) {
        this.warmed = true;
        // Start steady state from a known allocation floor so the first
        // measured frames are not dominated by warm-up garbage.
        this.warmMemoryKb = this.heapKb?.();
      }
      return;
    }

    this.updateSamples.push(updateSeconds);
    this.measuredFrames += 1;
    if (this.measuredFrames >= this.frames) {
      this.finish();
    }
  }

  draw(): void {
    if (this.done) {
      return;
    }
    const started = this.clock();
    const frame = this.renderer.draw();
    const drawSeconds = this.clock() - started;

    this.riggedActiveLastFrame = frame.rigged_active;

    if (this.warmed) {
      this.drawSamples.push(drawSeconds);
      if (frame.draw_calls !== undefined) {
        this.drawCalls.push(frame.draw_calls);
      }
      if (frame.texture_memory_bytes !== undefined) {
        this.textureMemory = frame.texture_memory_bytes;
      }
      for (const p of this.simulation.players()) {
        if (viewState.get(p.id) !== undefined) {
          this.posesSeen.add(p.is_keeper ? "keeper" : "outfield");
        }
      }
    }
  }

  finish(): void {
    if (this.done) {
      return;
    }
    this.done = true;
    this.finalHash = this.finalStateHashFn();
    this.endMemoryKb = this.heapKb?.();
  }

  finished(): boolean {
    return this.done;
  }

  /** Machine-readable evidence. Deliberately flat and JSON-shaped: #100 requires the metrics be reviewable without running the fixture. */
  result(): BenchmarkResult {
    let drawCallTotal = 0;
    let drawCallMax = 0;
    for (const v of this.drawCalls) {
      drawCallTotal += v;
      drawCallMax = Math.max(drawCallMax, v);
    }
    return {
      renderer: this.rigged ? "rigged" : "procedural",
      rigged_requested: this.rigged,
      rigged_active: this.riggedActiveLastFrame,
      seed: this.seed,
      warmup_frames: this.warmup_frames,
      measured_frames: this.measuredFrames,
      measured_seconds: this.measuredFrames * DT,
      viewport: this.viewport,
      players: this.simulation.players().length,
      update: summarise(this.updateSamples),
      draw: summarise(this.drawSamples),
      frame: summarise(this.frameSamples),
      draw_calls_mean: this.drawCalls.length > 0 ? drawCallTotal / this.drawCalls.length : 0,
      draw_calls_max: drawCallMax,
      ...(this.textureMemory !== undefined ? { texture_memory_bytes: this.textureMemory } : {}),
      ...(this.warmMemoryKb !== undefined ? { heap_warm_kb: this.warmMemoryKb } : {}),
      ...(this.endMemoryKb !== undefined ? { heap_end_kb: this.endMemoryKb } : {}),
      ...(this.finalHash !== undefined ? { final_state_hash: this.finalHash } : {}),
      environment: this.environment,
      raw_update_us: this.updateSamples,
      raw_draw_us: this.drawSamples,
      raw_frame_us: this.frameSamples,
    };
  }

  /** Which pose ids the fixture actually exercised (`"keeper"`/`"outfield"`). */
  posesSeenKinds(): ReadonlySet<"keeper" | "outfield"> {
    return this.posesSeen;
  }
}

// Gate evaluation, kept next to the thresholds so a report can never claim a
// pass the numbers do not support.
export function evaluate(result: BenchmarkResult): { readonly pass: boolean; readonly failures: readonly string[] } {
  const g = GATES;
  const failures: string[] = [];
  const check = (ok: boolean, message: string): void => {
    if (!ok) {
      failures.push(message);
    }
  };
  check(
    result.update.p95_ms <= g.update_p95_ms,
    `update p95 ${result.update.p95_ms.toFixed(2)} ms > ${g.update_p95_ms} ms`,
  );
  check(
    result.update.max_ms <= g.update_max_ms,
    `update max ${result.update.max_ms.toFixed(2)} ms > ${g.update_max_ms} ms`,
  );
  check(result.draw.p95_ms <= g.draw_p95_ms, `draw p95 ${result.draw.p95_ms.toFixed(2)} ms > ${g.draw_p95_ms} ms`);
  check(result.draw.max_ms <= g.draw_max_ms, `draw max ${result.draw.max_ms.toFixed(2)} ms > ${g.draw_max_ms} ms`);
  // The pacing gate is written per 60 seconds, so scale it to what we ran
  // rather than comparing a 10-second run against a 60-second allowance.
  const allowance = g.slow_frames_per_60s * Math.max(1, result.measured_seconds / 60);
  check(
    result.frame.over_33 <= allowance,
    `${result.frame.over_33} frames over 33 ms > allowance ${allowance.toFixed(1)}`,
  );
  check(result.frame.over_250 === 0, `${result.frame.over_250} frames over 250 ms`);
  // Evidence integrity: a rigged run that silently fell back to the
  // procedural renderer is not a fast rigged run, it is no measurement at
  // all.
  check(
    !result.rigged_requested || result.rigged_active,
    "rigged renderer was requested but fell back to procedural (no depth buffer?)",
  );
  return { pass: failures.length === 0, failures };
}

// Evidence goes out as pipe-delimited `GC_BENCH_*` lines rather than JSON,
// matching the rollback harness. That is not a style choice: a line-oriented
// format survives crossing into whatever headless console this fixture ends
// up running inside (matching the Lua original's love.js rationale), while
// a serialised object does not.
/** @param values seconds */
function microseconds(values: readonly number[]): string {
  return values.map((v) => String(Math.floor(v * 1e6 + 0.5))).join(",");
}

function summaryFields(summary: BenchmarkSummary, prefix: string): string {
  return (
    `${prefix}_p50=${summary.p50_ms.toFixed(4)}|` +
    `${prefix}_p95=${summary.p95_ms.toFixed(4)}|` +
    `${prefix}_p99=${summary.p99_ms.toFixed(4)}|` +
    `${prefix}_max=${summary.max_ms.toFixed(4)}|` +
    `${prefix}_mean=${summary.mean_ms.toFixed(4)}|` +
    `${prefix}_over16=${summary.over_16_67}|` +
    `${prefix}_over33=${summary.over_33}|` +
    `${prefix}_over250=${summary.over_250}|` +
    `${prefix}_n=${summary.samples}`
  );
}

export function emit(result: BenchmarkResult): void {
  const { pass, failures } = evaluate(result);
  console.log(
    `GC_BENCH_ENV|runtime=${result.environment.runtime_version}|gpu_name=${result.environment.gpu_name ?? "?"}` +
      `|gpu_vendor=${result.environment.gpu_vendor ?? "?"}|gpu_device=${result.environment.gpu_device ?? "?"}` +
      `|width=${result.viewport.w}|height=${result.viewport.h}|players=${result.players}`,
  );
  console.log(
    `GC_BENCH_RESULT|renderer=${result.renderer}|rigged_active=${String(result.rigged_active)}` +
      `|seed=${result.seed}|warmup_frames=${result.warmup_frames}|measured_frames=${result.measured_frames}` +
      `|${summaryFields(result.update, "update")}|${summaryFields(result.draw, "draw")}` +
      `|${summaryFields(result.frame, "frame")}|draw_calls_mean=${result.draw_calls_mean.toFixed(1)}` +
      `|draw_calls_max=${result.draw_calls_max}|texture_memory=${result.texture_memory_bytes ?? 0}` +
      `|heap_kb_warm=${result.heap_warm_kb ?? 0}|heap_kb_end=${result.heap_end_kb ?? 0}` +
      `|state_hash=${result.final_state_hash ?? "?"}`,
  );
  // Raw samples so a reviewer can recompute every percentile rather than
  // trusting this file's arithmetic, which is what #100 means by
  // reviewable.
  const raw: ReadonlyArray<readonly [string, readonly number[]]> = [
    ["update", result.raw_update_us],
    ["draw", result.raw_draw_us],
    ["frame", result.raw_frame_us],
  ];
  for (const [kind, values] of raw) {
    console.log(
      `GC_BENCH_SAMPLES|renderer=${result.renderer}|kind=${kind}|unit=microseconds|samples=${microseconds(values)}`,
    );
  }
  console.log(`GC_BENCH_GATE|renderer=${result.renderer}|pass=${String(pass)}|failures=${failures.join("; ")}`);
}
