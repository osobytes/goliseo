// Dev-only per-match debug event log (owner request, 2026-08-26).
//
// PURPOSE. Play-test reports arrive as prose ("the pass went to the wrong
// man", "I could not get a first touch") and the frame events that would
// settle them are gone by the time anyone looks. When the app runs under
// `vite dev`, this module streams each local match's discrete events and a
// sparse state sample to the dev server, which appends them under
// `ts/.match-debug/<match id>.jsonl` (see `matchDebugSink` in
// `ts/vite.config.ts`) -- a plain file an agent or a human can read after
// the fact and replay in their head.
//
// WHAT IT IS NOT. Not telemetry (nothing leaves the developer's machine:
// the sink only exists on the local dev server), not a replay (the input
// tape and determinism evidence own that), and not part of the simulation
// or presentation path -- everything here READS a decoded `RenderFrame`
// the renderer was getting anyway, and a `vite build` bundle disables
// itself at the `import.meta.env.DEV` gate with the sink absent regardless.
//
// SHAPE. One JSON object per line:
//   {"t":"meta", ...}   once, at match construction: teams, seed, options.
//   {"t":"ev", ...}     one per discrete match event, with the actor's id.
//   {"t":"state", ...}  every SAMPLE_EVERY_TICKS ticks: ball, owner, score.
//   {"t":"end", ...}    once, at dispose.
//
// The pure part (`entriesForFrame`) is separated from the impure sender so
// the entry shapes are testable without a browser -- the same pure-core
// split every screen module uses (AGENTS.md §9).

import { inputSample } from "@gc/input";
import type { inputSampleTypes } from "@gc/input";
import type { frameBufferTypes } from "@gc/render";

type RenderFrame = frameBufferTypes.RenderFrame;

// Every held/edge action name, for decoding a sample's bitmasks via
// `inputSample.packHeld`/`packEdges` -- no duplicate bit table. The
// `satisfies` record forces this list to stay exhaustive when a name is
// added to the union.
const HELD_NAME_SET = {
  shoot: true,
  pass: true,
  sprint: true,
  jockey: true,
  lob: true,
  aerial_strike: true,
  aerial_acrobatic: true,
  equipment: true,
} satisfies Record<inputSampleTypes.HeldActionName, true>;
const HELD_NAMES = Object.keys(HELD_NAME_SET) as readonly inputSampleTypes.HeldActionName[];
const EDGE_NAME_SET = {
  shoot: true,
  pass: true,
  switch: true,
  dash: true,
  dodge: true,
  equipment_pressed: true,
  equipment_released: true,
} satisfies Record<inputSampleTypes.EdgeActionName, true>;
const EDGE_NAMES = Object.keys(EDGE_NAME_SET) as readonly inputSampleTypes.EdgeActionName[];

/** Sparse state-sample cadence, in simulation ticks (30 = twice a second). */
export const SAMPLE_EVERY_TICKS = 30;

/** Flush when the buffer holds this many lines... */
const FLUSH_LINES = 200;
/** ...or when this much wall-clock time has passed since the last flush. */
const FLUSH_INTERVAL_MS = 1500;

/** The facts `begin` records once per match. */
export interface MatchDebugMeta {
  readonly home: string;
  readonly away: string;
  readonly seed: number;
  readonly duration_seconds: number;
  readonly max_goals: number;
  readonly combat: boolean;
  readonly local_slot: number;
}

/** One match's live logging state. */
interface LogState {
  readonly id: string;
  lines: string[];
  lastFlushMs: number;
  lastEventTick: number;
  lastSampleTick: number;
  lastInputKey: string;
  disabled: boolean;
}

let current: LogState | undefined;

function playerId(ids: readonly string[], slot: number | undefined): string | null {
  if (slot === undefined) {
    return null;
  }
  return ids[slot - 1] ?? null;
}

/**
 * The JSONL lines one decoded frame contributes: every discrete event, plus
 * a state sample when `tick` crossed the sampling cadence since
 * `lastSampleTick`. Pure -- the impure shell owns WHEN it is called and
 * where the lines go.
 */
export function entriesForFrame(
  tick: number,
  frame: RenderFrame,
  ids: readonly string[],
  lastSampleTick: number,
): readonly string[] {
  const out: string[] = [];
  const ev = frame.events;
  for (let i = 0; i < ev.count; i += 1) {
    out.push(
      JSON.stringify({
        t: "ev",
        tick,
        kind: ev.kind[i],
        p: playerId(ids, ev.slot[i]),
        x: Math.round(ev.x[i] ?? 0),
        y: Math.round(ev.y[i] ?? 0),
        ...(ev.outcome[i] !== undefined && { outcome: ev.outcome[i] }),
        ...(ev.style[i] !== undefined && { style: ev.style[i] }),
        ...(ev.difficulty[i] !== undefined && {
          difficulty: Number((ev.difficulty[i] ?? 0).toFixed(3)),
        }),
        ...(ev.shot_type[i] !== undefined && { shot_type: ev.shot_type[i] }),
        ...(ev.on_target[i] !== undefined && { on_target: ev.on_target[i] }),
      }),
    );
  }
  if (tick - lastSampleTick >= SAMPLE_EVERY_TICKS) {
    out.push(
      JSON.stringify({
        t: "state",
        tick,
        ball: [Math.round(frame.ball.x), Math.round(frame.ball.y), Math.round(frame.ball.z)],
        owner: playerId(ids, frame.possession.owner),
        controlled: playerId(ids, frame.hud.controlled),
        score: [frame.hud.home_score, frame.hud.away_score],
        time_left: Number(frame.hud.time_left.toFixed(1)),
      }),
    );
  }
  return out;
}

function flush(state: LogState, final: boolean): void {
  if (state.disabled || state.lines.length === 0) {
    return;
  }
  const body = state.lines.join("\n") + "\n";
  state.lines = [];
  state.lastFlushMs = Date.now();
  try {
    void fetch(`/__match_debug?m=${state.id}`, {
      method: "POST",
      body,
      ...(final && { keepalive: true }),
    }).catch(() => {
      // No sink (production preview, tests): stop trying for this match.
      state.disabled = true;
    });
  } catch {
    state.disabled = true;
  }
}

function maybeFlush(state: LogState): void {
  if (state.lines.length >= FLUSH_LINES || Date.now() - state.lastFlushMs >= FLUSH_INTERVAL_MS) {
    flush(state, false);
  }
}

/** Open a fresh per-match log. A previous unfinished log is flushed first. */
function begin(meta: MatchDebugMeta): void {
  if (!import.meta.env.DEV) {
    return;
  }
  if (current !== undefined) {
    end(-1);
  }
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const nonce = Math.floor(Math.random() * 0xffff).toString(16);
  current = {
    id: `${stamp}_${nonce}`,
    lines: [JSON.stringify({ t: "meta", ...meta })],
    lastFlushMs: Date.now(),
    lastEventTick: -1,
    lastSampleTick: -SAMPLE_EVERY_TICKS,
    lastInputKey: "",
    disabled: false,
  };
}

/**
 * Record one decoded frame. Ticks that did not advance are skipped, so a
 * same-tick frame rebuild (presentation mask changes re-decode the frame
 * without stepping the simulation) cannot double-log its events.
 */
function frame(tick: number, decoded: RenderFrame, ids: readonly string[]): void {
  const state = current;
  if (state === undefined || state.disabled || tick <= state.lastEventTick) {
    return;
  }
  state.lastEventTick = tick;
  const entries = entriesForFrame(tick, decoded, ids, state.lastSampleTick);
  if (tick - state.lastSampleTick >= SAMPLE_EVERY_TICKS) {
    state.lastSampleTick = tick;
  }
  state.lines.push(...entries);
  maybeFlush(state);
}

/**
 * The JSONL line one input sample contributes, or `undefined` when the
 * held set, the edges, and the stick's zero-ness all match `prevKey` (the
 * previous line's transition key) -- an analog stick wiggling while
 * nothing else changes must not flood the log. Pure; the impure shell owns
 * the key state.
 */
export function inputEntry(
  tick: number,
  sample: inputSampleTypes.InputSample,
  prevKey: string,
): { readonly key: string; readonly line: string } | undefined {
  const moving = sample.move_x !== 0 || sample.move_y !== 0;
  const key = `${sample.held}|${sample.edges}|${moving ? 1 : 0}`;
  if (key === prevKey) {
    return undefined;
  }
  const held = HELD_NAMES.filter((n) => (sample.held & inputSample.packHeld([n])) !== 0);
  const edges = EDGE_NAMES.filter((n) => (sample.edges & inputSample.packEdges([n])) !== 0);
  const line = JSON.stringify({
    t: "input",
    tick,
    held,
    ...(edges.length > 0 && { edges }),
    move: [sample.move_x, sample.move_y],
  });
  return { key, line };
}

/**
 * Record the local player's input sample for the tick about to step.
 * Logged on TRANSITIONS only (held set, edges, stick zero-ness), so a held
 * button is one line, not sixty a second.
 */
function input(tick: number, sample: inputSampleTypes.InputSample): void {
  const state = current;
  if (state === undefined || state.disabled) {
    return;
  }
  const entry = inputEntry(tick, sample, state.lastInputKey);
  if (entry === undefined) {
    return;
  }
  state.lastInputKey = entry.key;
  state.lines.push(entry.line);
  maybeFlush(state);
}

/** Close the current log and flush what remains. */
function end(tick: number): void {
  const state = current;
  current = undefined;
  if (state === undefined || state.disabled) {
    return;
  }
  state.lines.push(JSON.stringify({ t: "end", tick }));
  flush(state, true);
}

export const matchDebugLog = { begin, frame, input, end };
