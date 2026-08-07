// The per-frame glue tying `@gc/wasm` (the compiled simulation), `@gc/render`
// (frame decode + presentation types) and `@gc/input` (captured player
// intent) into one port: `SimHostPort`. `@gc/screens`' match screen consumes
// this port via injection -- it must not import `@gc/wasm` directly (v2/
// README.md's file-mapping table puts `game/screens/**` in `packages/screens`,
// which must not depend on `packages/app`; this module lives in `packages/app`
// specifically so the dependency runs the right way).
//
// ## Memory-view invalidation (v2/README.md's determinism-line doc; see
// `crates/gc-wasm/src/render_export.rs` and `packages/wasm/src/index.ts`)
//
// `SimHost.buildRenderFrame` (from `@gc/wasm`) returns a `Float64Array` VIEW
// over the wasm module's linear memory, read through the raw (non-wasm-
// bindgen) `render_frame_build`/`render_frame_ptr`/`render_frame_len`
// exports -- no wasm-bindgen glue on the per-frame path, and no word-by-word
// copy loop; the view is constructed once, directly, from the pointer and
// length those exports hand back.
//
// That view is only valid until wasm linear memory next grows: growth
// replaces `instance.exports.memory.buffer` wholesale, and any `Float64Array`
// still pointing at the old `ArrayBuffer` reads a detached, stale buffer
// (zeros or a "can't perform typed array operation on detached ArrayBuffer"
// throw, depending on the engine). `@gc/wasm`'s `buildRenderFrame` already
// re-derives its view from `raw.memory.buffer` on every single call rather
// than caching it (see that module's doc) -- so the fix here is negative
// space: `frame()` below calls `buildRenderFrame` fresh on every read and
// NEVER caches the `Float64Array` it returns across ticks. `sim_host.spec.ts`
// grows the wasm heap directly (`loadSimHost().memory.grow(n)`) between two
// `frame()` reads and asserts the second read still decodes correctly --
// that is the test this file's task brief asked for; a cached view would
// make it throw or silently read garbage.
//
// ## The decoder is `@gc/render`'s, not a copy
//
// `@gc/render` now exports `frame_buffer.ts` as `frameBuffer` (its
// `index.ts`: `export * as frameBuffer from "./frame_buffer.ts";`, the same
// pattern `@gc/input`'s `index.ts` uses for `inputSample`) -- this module
// used to carry a deliberate, temporary, hand-duplicated decoder here
// (trimmed to the fields `pitchTypes.RenderFrame` declares) precisely
// because that export did not exist yet. It does now, so that duplicate is
// gone: `frame()`/`roster()` below call `@gc/render`'s real
// `frameBuffer.decode`/`decodeRoster`/`toRenderFrame` directly, and this
// file's `RenderFrame` is `@gc/render`'s complete wire type (`hud`/
// `possession` included, not just `pitch.ts`'s drawing slice) -- see
// `frame_buffer.ts`'s own doc on why one canonical type replaces two
// drifting ones.
//
// `frame_buffer.decode`/`decodeRoster` take `ArrayLike<number>` rather than
// `readonly number[]` specifically so the `Float64Array` VIEW
// `buildRenderFrame` hands back (see this file's memory-view-invalidation
// section above) can be passed straight through with no copy: converting it
// to a plain array would copy every frame and defeat the zero-copy design
// the raw per-frame export exists for.

import { loadSimHost } from "@gc/wasm";
import type { FixedClock, SimHost, SimSession } from "@gc/wasm";
import { inputSample } from "@gc/input";
import type { inputSampleTypes } from "@gc/input";
import { frameBuffer } from "@gc/render";
import type { frameBufferTypes } from "@gc/render";

/** Re-exported so callers of this module need not also import `@gc/input`. */
export type InputSample = inputSampleTypes.InputSample;

/** Re-exported so callers of this module need not also import `@gc/render`. */
export type RenderFrame = frameBufferTypes.RenderFrame;
/** Re-exported so callers of this module need not also import `@gc/render`. */
export type RenderFrameRoster = frameBufferTypes.DecodedRenderFrameRoster;

// ---------------------------------------------------------------------------
// The `SimHostPort` contract (see this wave's task brief -- W2-C's match
// screen is written against this exact shape concurrently; it is not this
// file's free choice).
// ---------------------------------------------------------------------------

export interface SimHostPort {
  /**
   * Plan how many fixed ticks this render update should simulate, given
   * `dt` seconds elapsed since the last call -- delegates to
   * `gc_sim::fixed_clock`'s accumulator/catch-up/drop policy
   * (v2/README.md §2.1) via the wasm session's `FixedClock`, so this
   * decision has exactly one implementation, in Rust. Call once per render
   * update, then call `step` up to that many times, in order.
   */
  planTicks(dt: number): number;
  /** Advance the simulation by exactly one fixed tick with the given sample. */
  step(sample: InputSample): void;
  /**
   * The caller stopped running `step` before using every tick `planTicks`
   * authorized (e.g. the match finished mid-batch) -- must be called in
   * that case so the clock's carried-over accumulator resets, matching
   * what stopping early means to `gc_sim::fixed_clock::advance`'s own
   * step callback.
   */
  cancelPlannedTicks(): void;
  /** The current frame, decoded. Cheap to call; may return a reused object. */
  frame(): RenderFrame;
  /** Match-constant roster, decoded once. */
  roster(): RenderFrameRoster;
  /** Ticks simulated so far. */
  tick(): number;
  /** Release the wasm handle. Safe to call twice. */
  dispose(): void;
}

// ---------------------------------------------------------------------------
// Input wire encoding.
//
// `SimSession.step` (see `@gc/wasm`) takes a canonical `gc_sim::input_frame`
// wire string covering all eight canonical slots
// (`version|tick|move_x,move_y,held,edges|...`, one group per slot) --
// v2/README.md's own slot-mode doc (`crates/gc-wasm/src/session.rs`'s
// header). `SimHostPort.step`, by contract, takes exactly one `InputSample`.
// This wave's sim host is a LOCAL single-controlled-slot host: the given
// sample drives one canonical slot (`SimHostOptions.localSlot`, default 1 --
// home outfield slot 1) and every other slot stays neutral. See this file's
// end-of-module report: this is a genuine gap in `SimHostPort`'s declared
// shape (it has no way to express "which slot" or "all eight slots" for a
// multi-controller/online match), not a free design choice made here.
// ---------------------------------------------------------------------------

/** Mirrors `gc_sim::input_frame::SLOT_COUNT`. */
const INPUT_SLOT_COUNT = 8;

const NEUTRAL_SLOT_WIRE = "0,0,0,0";

function encodeSampleGroup(sample: InputSample): string {
  return `${sample.move_x},${sample.move_y},${sample.held},${sample.edges}`;
}

/** Mirrors `gc_sim::input_frame::encode`'s wire shape for a slot-mode frame. */
function encodeInputFrameWire(tick: number, localSlot: number, sample: InputSample): string {
  const groups: string[] = [];
  for (let slot = 1; slot <= INPUT_SLOT_COUNT; slot += 1) {
    groups.push(slot === localSlot ? encodeSampleGroup(sample) : NEUTRAL_SLOT_WIRE);
  }
  return [String(inputSample.VERSION), String(tick), ...groups].join("|");
}

// ---------------------------------------------------------------------------
// SimHostPort implementation.
// ---------------------------------------------------------------------------

export interface SimHostOptions {
  /**
   * The canonical input slot (one-based, `1..8`) this host's `step(sample)`
   * drives; every other slot is held neutral every tick. Defaults to `1`
   * (home outfield slot 1). See this file's "Input wire encoding" section
   * for why a single-slot mapping is this wave's chosen default.
   */
  readonly localSlot?: number;
  /**
   * Mirrors `crates/gc-wasm/src/session.rs`'s `Session::new`'s own
   * `combat_enabled` parameter (this wave): `false`/omitted (the default)
   * reproduces the pre-existing behavior exactly -- no combat companion is
   * ever built, byte for byte what this constructor did before that
   * parameter existed. See [`SessionConstructorWithCombat`]'s doc for why
   * this crosses to the real wasm constructor via a local, temporary type
   * widening rather than `@gc/wasm`'s own declared `SimSessionConstructor`.
   */
  readonly combatEnabled?: boolean;
}

/**
 * TEMPORARY LOCAL WIDENING -- not an edit of `@gc/wasm`'s own types, and not
 * meant to outlive the reason it exists.
 *
 * `packages/wasm/src/types.ts`'s hand-written `SimSessionConstructor` has
 * not yet grown the `combat_enabled` parameter `Session::new` gained this
 * wave (`crates/gc-wasm/src/session.rs`; see that module's doc) -- confirmed
 * stale by reading `types.ts` directly, not assumed: another agent's
 * concurrent task, per this wave's own brief, so fixing it there is not
 * this file's to do (`packages/wasm` is out of this batch's ownership).
 *
 * The REAL, rebuilt wasm-bindgen constructor (`dist/pkg/gc_wasm.d.cts`,
 * regenerated from the current Rust source via `packages/wasm/scripts/
 * build.mjs`) already accepts a seventh, optional `combat_enabled?: boolean
 * | null` parameter -- confirmed the same way, by reading the regenerated
 * declaration file, not merely the Rust source. Calling `this.host.Session`
 * with a seventh argument is therefore safe at RUNTIME today; only the
 * hand-written `SimSessionConstructor` type stops it compiling directly.
 * This local type plus cast is the narrowest possible bridge: it changes
 * nothing `@gc/wasm` exports, only how THIS file calls a constructor it
 * already knows (from reading `session.rs` and the regenerated `.d.cts`
 * directly) accepts a seventh argument. Delete this type and the cast site
 * below, and call `new this.host.Session(...)` directly with the combat
 * argument, once `SimSessionConstructor` itself grows the parameter.
 */
type SessionConstructorWithCombat = new (
  homeTeamId: string,
  awayTeamId: string,
  seed: number,
  durationSeconds: number,
  maxGoals: number,
  homeFormation: string | undefined,
  combatEnabled: boolean | undefined,
) => SimSession;

class WasmSimHost implements SimHostPort {
  private readonly host: SimHost;
  private readonly session: SimSession;
  private readonly clock: FixedClock;
  private readonly localSlot: number;
  private disposed = false;
  private rosterCache: RenderFrameRoster | undefined;
  private frameCache: { readonly tick: number; readonly frame: RenderFrame } | undefined;

  constructor(
    homeTeamId: string,
    awayTeamId: string,
    seed: number,
    durationSeconds: number,
    maxGoals: number,
    options: SimHostOptions = {},
  ) {
    const localSlot = options.localSlot ?? 1;
    if (!Number.isInteger(localSlot) || localSlot < 1 || localSlot > INPUT_SLOT_COUNT) {
      throw new Error(`sim_host: localSlot must be an integer in [1, ${INPUT_SLOT_COUNT}]`);
    }
    this.localSlot = localSlot;
    this.host = loadSimHost();
    // See `SessionConstructorWithCombat`'s doc for why this cast exists --
    // `@gc/wasm`'s own declared constructor type has not caught up to
    // `Session::new`'s seventh (`combat_enabled`) parameter yet.
    const SessionCtor = this.host.Session as unknown as SessionConstructorWithCombat;
    this.session = new SessionCtor(
      homeTeamId,
      awayTeamId,
      seed,
      durationSeconds,
      maxGoals,
      undefined,
      options.combatEnabled,
    );
    this.clock = new this.host.FixedClock();
  }

  private assertLive(): void {
    if (this.disposed) {
      throw new Error("sim_host: use after dispose");
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
    // Fresh every call, never cached across ticks -- see this file's header
    // on memory-view invalidation. `buildRenderFrame` itself re-derives its
    // `Float64Array` from the module's current `memory.buffer` every call.
    const words = this.host.buildRenderFrame(this.session.handle);
    if (words === null) {
      throw new Error("sim_host: no live session for this handle (already disposed?)");
    }
    // `words` is a `Float64Array` VIEW; `frameBuffer.decode` takes
    // `ArrayLike<number>` specifically so it can be passed straight through
    // with no copy -- see this file's header.
    const decoded = frameBuffer.decode(words);
    const frame = frameBuffer.toRenderFrame(decoded, this.roster());
    this.frameCache = { tick, frame };
    return frame;
  }

  roster(): RenderFrameRoster {
    this.assertLive();
    if (this.rosterCache === undefined) {
      const numeric = this.session.rosterNumeric();
      const strings = this.session.rosterIdsAndNames();
      this.rosterCache = frameBuffer.decodeRoster(numeric, strings);
    }
    return this.rosterCache;
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
 * Construct a {@link SimHostPort} over `@gc/wasm`'s compiled simulation.
 *
 * @param homeTeamId,awayTeamId Authored team ids (`gc_data::teams::ALL`).
 * @param seed Match RNG seed.
 * @param durationSeconds Match clock length.
 * @param maxGoals Golden-goal/mercy cap.
 * @param options See {@link SimHostOptions}.
 * @throws If either team id is unknown, or does not carry a five-player
 *   roster (mirrors `Session.new`'s errors -- see `@gc/wasm`).
 */
export function createSimHost(
  homeTeamId: string,
  awayTeamId: string,
  seed: number,
  durationSeconds: number,
  maxGoals: number,
  options?: SimHostOptions,
): SimHostPort {
  return new WasmSimHost(homeTeamId, awayTeamId, seed, durationSeconds, maxGoals, options);
}
