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
// ## A duplicated decoder, and why it exists here instead of an import
//
// The natural shape of this file imports `decode`/`decodeRoster`/
// `toRenderFrame` from `@gc/render`'s `frame_buffer.ts` -- that module exists
// precisely to turn the wire block `buildRenderFrame` hands back into the
// `RenderFrame` shape `SimHostPort.frame()` promises. It is NOT reachable
// from here: `packages/render/src/index.ts` re-exports `pitch.ts`,
// `arena.ts`, `scene.ts` and the rest of the wave-1 surface, but never
// `frame_buffer.ts` (its own header explicitly lists what index.ts leaves
// off -- `draw2d.ts`, `gl_probe.ts` -- and `frame_buffer.ts` is not on that
// list either; it is simply not wired up yet). `packages/render/package.json`
// also only maps `"."` to `src/index.ts`, and this workspace's
// `moduleResolution: "bundler"` honors that -- there is no subpath import
// that reaches an unexported module.
//
// This task's file ownership is `sim_host.ts`/its spec/this package's
// manifest only -- `packages/render` is explicitly out of scope this wave
// (three other agents are concurrently editing other packages, `packages/
// render` among them), so adding the one-line fix
// (`export * as frameBuffer from "./frame_buffer.ts";`, matching the exact
// pattern `packages/input/src/index.ts` already uses for `inputSample`) is
// not this file's call to make. Leaving the import unresolved was also not
// an option: an unresolved import fails `tsc --build` for the WHOLE `@gc/app`
// project (composite build, one compilation unit), breaking every other
// file in this package, not just this one.
//
// So `decodeFrameWords`/`decodeRosterWords`/`toRenderFrame` below are a
// **deliberate, temporary duplicate** of `packages/render/src/frame_buffer
// .ts`'s `decode`/`decode_roster`/`toRenderFrame`, trimmed to the subset
// `RenderFrame` (`pitch.ts`) actually declares -- field/ball/control/players/
// roster. `hud`, `possession` and `events` (which a full decode also
// produces) are read past but not constructed, since `RenderFrame` does not
// carry them and nothing here consumes them. Every constant, offset and enum
// mapping below is copied verbatim from that module rather than
// re-derived, specifically to avoid introducing a second, silently-drifting
// implementation of a wire format two languages already need to agree on.
// See this file's end-of-module report for the flagged follow-up: once
// `packages/render` exports `frame_buffer`, this whole DUPLICATED DECODER
// section should be deleted and replaced with the import it stands in for.

import { loadSimHost } from "@gc/wasm";
import type { SimHost, SimSession } from "@gc/wasm";
import { inputSample } from "@gc/input";
import type { inputSampleTypes } from "@gc/input";
import type { pitchTypes } from "@gc/render";

/** Re-exported so callers of this module need not also import `@gc/input`. */
export type InputSample = inputSampleTypes.InputSample;

/** Re-exported so callers of this module need not also import `@gc/render`. */
export type RenderFrame = pitchTypes.RenderFrame;
/** Re-exported so callers of this module need not also import `@gc/render`. */
export type RenderFrameRoster = pitchTypes.RenderFrameRoster;

type Field = pitchTypes.RenderFrameField;
type Ball = pitchTypes.RenderFrameBall;
type Control = pitchTypes.RenderFrameControl;
type Players = pitchTypes.RenderFramePlayers;
type RGB = readonly [number, number, number];

// ---------------------------------------------------------------------------
// The `SimHostPort` contract (see this wave's task brief -- W2-C's match
// screen is written against this exact shape concurrently; it is not this
// file's free choice).
// ---------------------------------------------------------------------------

export interface SimHostPort {
  /** Advance the simulation by exactly one fixed tick with the given sample. */
  step(sample: InputSample): void;
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
// DUPLICATED DECODER -- see this file's header. Verbatim port of the layout
// constants, offsets and enum tables in `packages/render/src/frame_buffer
// .ts`, trimmed to the fields `RenderFrame` declares.
// ---------------------------------------------------------------------------

const LAYOUT_VERSION = 1;
const RENDER_FRAME_VERSION = 1;
const MAGIC = 0x474f_4c46;
const ROSTER_MAGIC = 0x474f_4c52;
const HEADER_WORDS = 12;
const SCALAR_FIELD_COUNT = 44;
const PLAYER_FIELD_COUNT = 21;
const EVENT_FIELD_COUNT = 15;
const ROSTER_HEADER_WORDS = 7;
const ROSTER_FIELD_COUNT = 7;

function frameWords(count: number, eventCount: number): number {
  return HEADER_WORDS + SCALAR_FIELD_COUNT + PLAYER_FIELD_COUNT * count + EVENT_FIELD_COUNT * eventCount;
}

function at(words: Float64Array, index: number): number {
  const value = words[index];
  if (value === undefined) {
    throw new Error(`sim_host: word ${index} out of range (block has ${words.length} words)`);
  }
  return value;
}

function decodeBool(word: number): boolean {
  if (word === 1) {
    return true;
  }
  if (word === 0) {
    return false;
  }
  throw new Error(`sim_host: not a boolean word: ${word}`);
}

function optDecode<T>(code: number, fromCode: (code: number) => T | undefined, what: string): T | undefined {
  if (code === 0) {
    return undefined;
  }
  const value = fromCode(code);
  if (value === undefined) {
    throw new Error(`sim_host: unknown ${what} code ${code}`);
  }
  return value;
}

function soaIndex(fieldIndex: number, slotIndex: number, count: number): number {
  return fieldIndex * count + slotIndex;
}

function column(words: Float64Array, at0: number, fieldIndex: number, count: number): number[] {
  const start = at0 + soaIndex(fieldIndex, 0, count);
  const out: number[] = [];
  for (let i = 0; i < count; i += 1) {
    out.push(at(words, start + i));
  }
  return out;
}

function teamFromCode(code: number): "home" | "away" | undefined {
  switch (code) {
    case 1:
      return "home";
    case 2:
      return "away";
    default:
      return undefined;
  }
}

function speciesShapeFromCode(code: number): "round" | "broad" | "angular" | "cluster" | undefined {
  switch (code) {
    case 1:
      return "round";
    case 2:
      return "broad";
    case 3:
      return "angular";
    case 4:
      return "cluster";
    default:
      return undefined;
  }
}

function chargeKindFromCode(code: number): "shot" | "pass" | undefined {
  switch (code) {
    case 1:
      return "shot";
    case 2:
      return "pass";
    default:
      return undefined;
  }
}

function poseSourceFromCode(code: number): string | undefined {
  switch (code) {
    case 1:
      return "soccer";
    case 2:
      return "combat";
    case 3:
      return "locomotion";
    default:
      return undefined;
  }
}

function aerialStyleFromCode(
  code: number,
): "leg_control" | "chest_control" | "volley" | "header" | "bicycle" | undefined {
  switch (code) {
    case 1:
      return "leg_control";
    case 2:
      return "chest_control";
    case 3:
      return "volley";
    case 4:
      return "header";
    case 5:
      return "bicycle";
    default:
      return undefined;
  }
}

function aerialOutcomeFromCode(code: number): "clean" | "heavy" | "miss" | undefined {
  switch (code) {
    case 1:
      return "clean";
    case 2:
      return "heavy";
    case 3:
      return "miss";
    default:
      return undefined;
  }
}

function poseIdFromCode(code: number): string | undefined {
  switch (code) {
    case 1:
      return "keeper_grab";
    case 2:
      return "keeper_throw";
    case 3:
      return "keeper_punt";
    case 4:
      return "keeper_tip";
    case 5:
      return "keeper_spread";
    case 6:
      return "keeper_central";
    case 7:
      return "keeper_stretch";
    case 8:
      return "keeper_dive";
    case 9:
      return "keeper_get_up";
    case 10:
      return "keeper_set";
    case 11:
      return "keeper_ready_low";
    case 12:
      return "keeper_shuffle";
    case 13:
      return "keeper_ready_tall";
    case 14:
      return "aerial_bicycle";
    case 15:
      return "aerial_action";
    case 16:
      return "combat_knockback";
    case 17:
      return "combat_stagger";
    case 18:
      return "combat_guard";
    case 19:
      return "combat_active";
    case 20:
      return "combat_windup";
    case 21:
      return "combat_aim";
    case 22:
      return "combat_recovery";
    case 23:
      return "soccer_windup";
    case 24:
      return "slide";
    case 25:
      return "tackle";
    case 26:
      return "stumble";
    case 27:
      return "kick_follow";
    case 28:
      return "settle";
    case 29:
      return "run_telegraph";
    case 30:
      return "contain";
    case 31:
      return "fatigue";
    case 32:
      return "locomotion";
    default:
      return undefined;
  }
}

interface DecodedFrame {
  readonly field: Field;
  readonly ball: Ball;
  readonly control: Control;
  readonly players: Players;
}

/**
 * Trimmed port of `frame_buffer.ts`'s `decode` -- see this file's header.
 * Reads field/ball/control/players; validates but does not construct
 * possession/hud/events (`RenderFrame` does not carry them).
 */
function decodeFrameWords(words: Float64Array): DecodedFrame {
  if (at(words, 0) !== MAGIC) {
    throw new Error(`sim_host: not a render frame block: magic ${at(words, 0)}`);
  }
  if (at(words, 1) !== LAYOUT_VERSION) {
    throw new Error(`sim_host: frame layout version ${at(words, 1)}; expected ${LAYOUT_VERSION}`);
  }
  if (at(words, 2) !== RENDER_FRAME_VERSION) {
    throw new Error(`sim_host: render frame version ${at(words, 2)}; expected ${RENDER_FRAME_VERSION}`);
  }
  if (at(words, 4) !== HEADER_WORDS) {
    throw new Error(`sim_host: header words ${at(words, 4)}; expected ${HEADER_WORDS}`);
  }
  if (at(words, 5) !== SCALAR_FIELD_COUNT) {
    throw new Error(`sim_host: scalar words ${at(words, 5)}; expected ${SCALAR_FIELD_COUNT}`);
  }
  if (at(words, 7) !== PLAYER_FIELD_COUNT) {
    throw new Error(`sim_host: player field count ${at(words, 7)}; expected ${PLAYER_FIELD_COUNT}`);
  }
  if (at(words, 9) !== EVENT_FIELD_COUNT) {
    throw new Error(`sim_host: event field count ${at(words, 9)}; expected ${EVENT_FIELD_COUNT}`);
  }

  const count = at(words, 6);
  const eventCount = at(words, 8);
  const total = frameWords(count, eventCount);
  if (at(words, 3) !== total) {
    throw new Error(`sim_host: total words ${at(words, 3)}; expected ${total}`);
  }

  const scalarsAt = HEADER_WORDS;
  const s = (index: number): number => at(words, scalarsAt + index);

  const field: Field = {
    w: s(0),
    h: s(1),
    crossbar_h: s(2),
    penalty_box_depth: s(3),
    penalty_box_h: s(4),
    goal_home: { x: s(5), y: s(6), w: s(7), h: s(8) },
    goal_away: { x: s(9), y: s(10), w: s(11), h: s(12) },
  };

  const landingValid = decodeBool(s(20));
  const ball: Ball = {
    x: s(13),
    y: s(14),
    z: s(15),
    visible: decodeBool(s(19)),
    ...(landingValid ? { landing_x: s(21), landing_y: s(22) } : {}),
  };

  const passTarget = s(27);
  const chargeKind = optDecode(s(28), chargeKindFromCode, "charge kind");
  const control: Control = {
    controlled: s(26),
    ...(passTarget !== 0 ? { pass_target: passTarget } : {}),
    ...(chargeKind !== undefined ? { charge_kind: chargeKind } : {}),
    charge: s(29),
  };

  const playersAt = scalarsAt + SCALAR_FIELD_COUNT;
  const rawControlled = column(words, playersAt, 8, count);
  const rawDashing = column(words, playersAt, 9, count);
  const rawHolding = column(words, playersAt, 10, count);
  const players: Players = {
    count,
    x: column(words, playersAt, 0, count),
    y: column(words, playersAt, 1, count),
    facing_x: column(words, playersAt, 2, count),
    facing_y: column(words, playersAt, 3, count),
    pose_id: column(words, playersAt, 5, count).map((code) => optDecode(code, poseIdFromCode, "pose id")),
    pose_priority: column(words, playersAt, 6, count),
    pose_source: column(words, playersAt, 7, count).map((code) => optDecode(code, poseSourceFromCode, "pose source")),
    controlled: rawControlled.map(decodeBool),
    dashing: rawDashing.map(decodeBool),
    holding: rawHolding.map(decodeBool),
    dive: column(words, playersAt, 11, count),
    dive_dir_x: column(words, playersAt, 12, count),
    dive_dir_y: column(words, playersAt, 13, count),
    grab: column(words, playersAt, 14, count),
    throw: column(words, playersAt, 15, count),
    windup: column(words, playersAt, 16, count),
    aerial: column(words, playersAt, 17, count),
    aerial_jump: column(words, playersAt, 18, count),
    aerial_style: column(words, playersAt, 19, count).map((code) => optDecode(code, aerialStyleFromCode, "aerial style")),
    aerial_outcome: column(words, playersAt, 20, count).map((code) =>
      optDecode(code, aerialOutcomeFromCode, "aerial outcome"),
    ),
  };

  return { field, ball, control, players };
}

function at2(parts: readonly string[], index: number): string {
  const value = parts[index];
  if (value === undefined) {
    throw new Error(`sim_host: roster string ${index} out of range`);
  }
  return value;
}

function atNum(values: readonly number[], index: number): number {
  const value = values[index];
  if (value === undefined) {
    throw new Error(`sim_host: value ${index} out of range`);
  }
  return value;
}

/** Verbatim-trimmed port of `frame_buffer.ts`'s `decodeRoster`. */
function decodeRosterWords(words: Float64Array, strings: string): RenderFrameRoster {
  if (at(words, 0) !== ROSTER_MAGIC) {
    throw new Error(`sim_host: not a render roster block: magic ${at(words, 0)}`);
  }
  if (at(words, 1) !== LAYOUT_VERSION) {
    throw new Error(`sim_host: roster layout version ${at(words, 1)}; expected ${LAYOUT_VERSION}`);
  }
  if (at(words, 2) !== RENDER_FRAME_VERSION) {
    throw new Error(`sim_host: render frame version ${at(words, 2)}; expected ${RENDER_FRAME_VERSION}`);
  }
  if (at(words, 6) !== ROSTER_FIELD_COUNT) {
    throw new Error(`sim_host: roster field count ${at(words, 6)}; expected ${ROSTER_FIELD_COUNT}`);
  }

  const count = at(words, 5);

  const blob = `${strings}\n`;
  const parts = blob.split("\n");
  parts.pop();
  if (parts.length !== count * 2) {
    throw new Error(`sim_host: roster blob holds ${parts.length} strings; expected ${count * 2}`);
  }

  const ids: string[] = [];
  for (let index = 0; index < count; index += 1) {
    ids.push(at2(parts, index * 2));
  }

  const at0 = ROSTER_HEADER_WORDS;
  const rawIsKeeper = column(words, at0, 1, count);
  const r = column(words, at0, 4, count);
  const g = column(words, at0, 5, count);
  const b = column(words, at0, 6, count);
  const speciesColor: RGB[] = [];
  for (let index = 0; index < count; index += 1) {
    speciesColor.push([atNum(r, index), atNum(g, index), atNum(b, index)]);
  }

  return {
    ids,
    teams: column(words, at0, 0, count).map((code) => {
      const team = optDecode(code, teamFromCode, "team");
      if (team === undefined) {
        throw new Error("sim_host: missing required team (code 0)");
      }
      return team;
    }),
    is_keeper: rawIsKeeper.map(decodeBool),
    radius: column(words, at0, 2, count),
    species_shape: column(words, at0, 3, count).map((code) => {
      const shape = optDecode(code, speciesShapeFromCode, "species shape");
      if (shape === undefined) {
        throw new Error("sim_host: missing required species shape (code 0)");
      }
      return shape;
    }),
    species_color: speciesColor,
  };
}

function toRenderFrame(frame: DecodedFrame, roster: RenderFrameRoster): RenderFrame {
  return {
    field: frame.field,
    roster,
    players: frame.players,
    ball: frame.ball,
    control: frame.control,
  };
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
}

class WasmSimHost implements SimHostPort {
  private readonly host: SimHost;
  private readonly session: SimSession;
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
    this.session = new this.host.Session(homeTeamId, awayTeamId, seed, durationSeconds, maxGoals);
  }

  private assertLive(): void {
    if (this.disposed) {
      throw new Error("sim_host: use after dispose");
    }
  }

  step(sample: InputSample): void {
    this.assertLive();
    inputSample.validateSample(sample);
    const wire = encodeInputFrameWire(this.session.inputTick, this.localSlot, sample);
    this.session.step(wire);
    this.frameCache = undefined;
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
    const decoded = decodeFrameWords(words);
    const frame = toRenderFrame(decoded, this.roster());
    this.frameCache = { tick, frame };
    return frame;
  }

  roster(): RenderFrameRoster {
    this.assertLive();
    if (this.rosterCache === undefined) {
      const numeric = this.session.rosterNumeric();
      const strings = this.session.rosterIdsAndNames();
      this.rosterCache = decodeRosterWords(numeric, strings);
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
