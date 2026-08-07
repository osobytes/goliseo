// A browser-loading counterpart to `sim_host.ts`'s `WasmSimHost` -- NOT an
// edit of that file (out of this batch's scope; see the task brief that
// assigned this milestone: `sim_host.ts` is owned by a different agent).
//
// `sim_host.ts`'s `loadSimHost` (`@gc/wasm`'s `src/index.ts`) is
// unconditionally Node-shaped: `createRequire` + a synchronous `require()`
// of `dist/pkg/gc_wasm.cjs`, whose own wasm-bindgen `--target nodejs`
// glue reads the `.wasm` bytes with `node:fs`'s synchronous
// `readFileSync`. None of `require`, `node:module`, or `node:fs` exist in a
// browser, so that path cannot run there regardless of bundler -- this
// is exactly the gap `v2/README.md`'s app-shell milestone brief calls out
// ("wasm loading in a browser differs from node"). `packages/wasm/scripts/
// build_web.mjs` (this batch) builds a second, `--target web` artifact for
// exactly this reason; this file is its one, browser-only consumer.
//
// The `SimHostPort` contract itself (`planTicks`/`step`/`cancelPlannedTicks`/
// `frame`/`roster`/`tick`/`dispose`) is `@gc/screens`'s `match.ts`'s, not
// this file's to redefine -- imported by name below (unlike `sim_host.ts`,
// which predates `@gc/screens` exporting it and so hand-mirrors the shape;
// this file has no such constraint) so `createBrowserSimHost` is checked
// against the EXACT type `MatchScreenPorts.createHost` expects, not a
// hand-copied approximation of it. Implements it the same way
// `sim_host.ts`'s `createSimHost` does, including the same
// single-canonical-slot input mapping and the same "never cache the
// `Float64Array` view across ticks" memory-invalidation rule (`sim_host.ts`'s
// header explains why). The two implementations are deliberately similar --
// they wrap the same Rust ABI -- but are independent code, one async
// (`init()` must resolve before any wasm export is callable), one sync.
//
// `frame()`/`roster()` return `@gc/screens`'s deliberately narrow
// `RenderFrame`/`RenderFrameRoster` types (that package's own header:
// "only the fields this module reads are declared locally"), while the
// concrete values built here are `@gc/render`'s full `frame_buffer.ts`
// `RenderFrame`/`DecodedRenderFrameRoster` -- a strict superset for
// `RenderFrame` (assignable without a cast) but NOT for `RenderFrameRoster`,
// whose target type (`Readonly<Record<string, unknown>>`) needs an index
// signature `DecodedRenderFrameRoster` structurally lacks even though every
// field it has is compatible. `roster()` casts for exactly that reason,
// documented at the cast site below.

import { inputSample } from "@gc/input";
import type { inputSampleTypes } from "@gc/input";
import { frameBuffer } from "@gc/render";
import type { frameBufferTypes } from "@gc/render";
import type { RenderFrame, RenderFrameRoster, SimHostPort } from "@gc/screens";
import init, * as gcWasmWeb from "@gc/wasm/web";

/** Re-exported so callers of this module need not also import `@gc/input`. */
export type InputSample = inputSampleTypes.InputSample;
export type { RenderFrame, RenderFrameRoster, SimHostPort };

/** Mirrors `gc_sim::input_frame::SLOT_COUNT` -- see `sim_host.ts`'s "Input wire encoding" section. */
const INPUT_SLOT_COUNT = 8;
const NEUTRAL_SLOT_WIRE = "0,0,0,0";

function encodeSampleGroup(sample: InputSample): string {
  return `${sample.move_x},${sample.move_y},${sample.held},${sample.edges}`;
}

function encodeInputFrameWire(tick: number, localSlot: number, sample: InputSample): string {
  const groups: string[] = [];
  for (let slot = 1; slot <= INPUT_SLOT_COUNT; slot += 1) {
    groups.push(slot === localSlot ? encodeSampleGroup(sample) : NEUTRAL_SLOT_WIRE);
  }
  return [String(inputSample.VERSION), String(tick), ...groups].join("|");
}

export interface BrowserSimHostOptions {
  /** See `sim_host.ts`'s `SimHostOptions.localSlot` -- same default, same meaning. */
  readonly localSlot?: number;
  /** See `sim_host.ts`'s `SimHostOptions.combatEnabled` -- same default, same
   * meaning, forwarded to the browser-target `gcWasmWeb.Session` constructor's
   * own seventh, optional `combat_enabled` parameter (`crates/gc-wasm/src/
   * session.rs`'s `Session::new`; confirmed present in the regenerated
   * `dist/pkg-web/gc_wasm.d.ts` by reading it directly after rebuilding). */
  readonly combatEnabled?: boolean;
}

/**
 * The loaded, browser-target `gc-wasm` module -- the async counterpart of
 * `@gc/wasm`'s `SimHost`. Loaded once per page (module-level cache, mirroring
 * `loadSimHost`'s own `cached` variable) since re-running `init()` a second
 * time against the same module is wasted work and `init()` itself is
 * idempotent-safe but not free.
 */
let cachedInit: Promise<void> | undefined;

async function ensureInitialized(): Promise<void> {
  if (cachedInit === undefined) {
    cachedInit = init().then(() => undefined);
  }
  await cachedInit;
}

class BrowserWasmSimHost implements SimHostPort {
  private readonly session: InstanceType<typeof gcWasmWeb.Session>;
  private readonly clock: InstanceType<typeof gcWasmWeb.FixedClock>;
  private readonly localSlot: number;
  private disposed = false;
  private rosterCache: frameBufferTypes.DecodedRenderFrameRoster | undefined;
  private frameCache: { readonly tick: number; readonly frame: RenderFrame } | undefined;

  constructor(
    homeTeamId: string,
    awayTeamId: string,
    seed: number,
    durationSeconds: number,
    maxGoals: number,
    options: BrowserSimHostOptions = {},
  ) {
    const localSlot = options.localSlot ?? 1;
    if (!Number.isInteger(localSlot) || localSlot < 1 || localSlot > INPUT_SLOT_COUNT) {
      throw new Error(`browser_sim_host: localSlot must be an integer in [1, ${INPUT_SLOT_COUNT}]`);
    }
    this.localSlot = localSlot;
    this.session = new gcWasmWeb.Session(
      homeTeamId,
      awayTeamId,
      seed,
      durationSeconds,
      maxGoals,
      undefined,
      options.combatEnabled,
    );
    this.clock = new gcWasmWeb.FixedClock();
  }

  private assertLive(): void {
    if (this.disposed) {
      throw new Error("browser_sim_host: use after dispose");
    }
  }

  planTicks(dt: number): number {
    this.assertLive();
    return this.clock.advance(dt);
  }

  step(sample: InputSample): void {
    this.assertLive();
    inputSample.validateSample(sample);
    const wire = encodeInputFrameWire(this.session.inputTick, this.localSlot, sample);
    this.session.step(wire);
    this.frameCache = undefined;
  }

  cancelPlannedTicks(): void {
    this.assertLive();
    this.clock.stopEarly();
  }

  frame(): RenderFrame {
    this.assertLive();
    const tick = this.session.inputTick;
    if (this.frameCache !== undefined && this.frameCache.tick === tick) {
      return this.frameCache.frame;
    }
    const raw = gcWasmWeb.__getRawExports();
    const ok = raw.render_frame_build(this.session.handle);
    if (ok === 0) {
      throw new Error("browser_sim_host: no live session for this handle (already disposed?)");
    }
    const ptr = raw.render_frame_ptr();
    const len = raw.render_frame_len();
    // Fresh every call, never cached across ticks -- `raw.memory.buffer` is
    // replaced wholesale whenever wasm linear memory grows (see sim_host.ts's
    // header on memory-view invalidation, which this mirrors exactly).
    const words = new Float64Array(raw.memory.buffer, ptr, len);
    const decoded = frameBuffer.decode(words);
    const frame = frameBuffer.toRenderFrame(decoded, this.rosterInternal());
    this.frameCache = { tick, frame };
    return frame;
  }

  private rosterInternal(): frameBufferTypes.DecodedRenderFrameRoster {
    this.assertLive();
    if (this.rosterCache === undefined) {
      const numeric = this.session.rosterNumeric();
      const strings = this.session.rosterIdsAndNames();
      this.rosterCache = frameBuffer.decodeRoster(numeric, strings);
    }
    return this.rosterCache;
  }

  /** See this file's header for why this casts: `RenderFrameRoster`
   * (`@gc/screens`'s deliberately narrow declared type) needs a generic
   * string index signature `DecodedRenderFrameRoster` structurally lacks,
   * even though the real object is a strict superset of it field-for-field. */
  roster(): RenderFrameRoster {
    return this.rosterInternal() as unknown as RenderFrameRoster;
  }

  tick(): number {
    this.assertLive();
    return this.session.inputTick;
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.session.free();
  }
}

/**
 * Construct a {@link SimHostPort} over `@gc/wasm`'s compiled simulation, via
 * the browser (`--target web`) artifact. Callers MUST `await
 * ensureBrowserSimHostReady()` once (module init) before this -- see that
 * function's doc -- `createBrowserSimHost` itself stays synchronous so it can
 * satisfy `@gc/screens`'s `SimHostFactory` (`() => SimHostPort`, no `Promise`
 * in its signature) once init has resolved.
 */
export function createBrowserSimHost(
  homeTeamId: string,
  awayTeamId: string,
  seed: number,
  durationSeconds: number,
  maxGoals: number,
  options?: BrowserSimHostOptions,
): SimHostPort {
  return new BrowserWasmSimHost(homeTeamId, awayTeamId, seed, durationSeconds, maxGoals, options);
}

/**
 * Resolves once the browser wasm module has been fetched and instantiated.
 * Idempotent and safe to call more than once (e.g. once at app boot, and
 * again defensively before the first real match) -- subsequent calls reuse
 * the same in-flight/settled promise rather than re-fetching.
 */
export async function ensureBrowserSimHostReady(): Promise<void> {
  await ensureInitialized();
}
