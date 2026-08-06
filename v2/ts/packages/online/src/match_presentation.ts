import type { Result } from "@gc/core";

// Ported from game/online/match_presentation.lua.
//
// Turns the online driver's raw tick outputs into the stable presentation
// timeline the match renderer already consumes.
//
// `sim.rollback_playable_lab` owns an event timeline because it owns its
// own rollback session. The online driver deliberately does not: it
// publishes authority, corrections, and outputs, and leaves presentation to
// the game layer. This module is that layer, and it is the only new
// presentation machinery the online flow adds -- everything downstream
// (speculative add/revoke/replace, confirmed exactly-once consumers, VFX,
// audio, camera, HUD, replay, statistics) is the shipped #113/#147 path,
// unchanged.
//
// It is pure: no transport, no clock, no DOM. It reads a `MatchDriverBatch`
// and the driver's snapshot lookups, and returns a batch in exactly the
// shape `game/screens/match.lua`'s TypeScript successor already knows how
// to consume.
//
// # Append versus replacement
//
// The driver emits corrected outputs (from its single reconciliation)
// before this step's fresh outputs, both in session-tick space. An output
// at or below the highest tick already applied is therefore the head of a
// correction run that replaces the complete speculative tail; anything
// above it is an ordinary append. `sim.rollback_events` asserts both
// shapes, so a mis-split fails loudly here rather than silently publishing
// a cue twice.
//
// Confirmation runs after the correction run and again at the end of the
// batch, mirroring the laboratory: freeing the oldest event slot before the
// next predicted output is appended is the difference between a supported
// correction and a false unconfirmed-window terminal at a long delay.
//
// ## Boundary note: this is the file v2/README.md calls out by name
//
// `sim.rollback_events`, `sim.rollback_input_history`, and
// `game.online.match_driver` are all Rust-owned (`crates/gc-sim` /
// `crates/gc-netcode`; v2/README.md §2.1) -- this module reads rollback
// state to decide what to *show*, which is presentation, but it calls four
// functions belonging to those modules (`rollback_events.new/apply/confirm
// /diagnostics` and `match_driver.snapshot/diagnostics`) that this port has
// no Rust bridge to reach. `@gc/wasm` now exposes those four as separate
// callable primitives -- `RollbackEventsTimeline`'s `create`/`apply`/
// `confirm`/`diagnosticsJson` (`crates/gc-wasm/src/
// rollback_events_bridge.rs`) and `MatchDriverBridge.snapshotLookup`/
// `diagnosticsJson` (`crates/gc-wasm/src/match_driver_bridge.rs`) -- built
// specifically to satisfy `RollbackEventsPort`/`MatchDriverPort` below, and
// the correction-batch case is no longer skipped on the Rust side either.
// What still blocks a real adapter is that nothing in `@gc/wasm` can build
// the `freezeJson`/`manifestJson` `MatchDriverBridge`'s constructor needs
// in the first place (`match_driver_fixture` is ported in
// `crates/gc-netcode` but has no `wasm-bindgen` binding yet), and that a
// real `RollbackEventsPort.apply` adapter needs more of the driver's raw
// per-tick output than `RollbackTickOutput` below currently carries. See
// `match_presentation.spec.ts`'s header comment for the full, empirically
// checked breakdown. Rather than duplicate rollback scheduling here --
// which the README is explicit is the one thing that must never happen on
// this side of the determinism line -- those four functions are threaded
// through as `RollbackEventsPort`/`MatchDriverPort`, injected the same way
// `@gc/ui/src/tuning_panel.ts` injects `TuningSource`. `MatchSnapshot`,
// `RollbackEventTimeline`, and every wrapped-event payload stay fully
// opaque type parameters: this module never inspects their contents, only
// passes them through.

/** `sim.match_driver`'s (or `sim.rollback_session`'s) `MatchSnapshot` -- opaque here; never inspected. */
export type OpaqueSnapshot = unknown;

/** `game.online.match_driver`'s `RollbackTickOutput` (Rust-owned; only the fields this module reads). */
export interface RollbackTickOutput {
  readonly tick: number;
  readonly end_boundary: number;
}

/** `game.online.match_driver`'s `MatchDriverBatch` (Rust-owned; only the field this module reads). */
export interface MatchDriverBatch {
  readonly outputs: readonly RollbackTickOutput[];
}

/** `sim.rollback_snapshot_history`'s `RollbackSnapshotLookup` (Rust-owned). */
export interface SnapshotLookup<TSnapshot> {
  /** Owned by `gc-sim`'s `rollback_snapshot_history`; only `"present"`/`"retained"` are inspected here. */
  readonly status: string;
  readonly tick: number;
  readonly snapshot?: TSnapshot;
}

/** `game.online.match_driver.snapshot`/`.diagnostics`, injected -- see the module doc comment. */
export interface MatchDriverPort<TDriver, TSnapshot> {
  snapshot(driver: TDriver, boundaryTick: number): SnapshotLookup<TSnapshot>;
  diagnostics(driver: TDriver): { readonly confirmed_output_tick: number };
}

/** `sim.rollback_events`'s `RollbackEventStepInput` (Rust-owned). */
export interface RollbackEventStepInput<TSnapshot> {
  readonly output: RollbackTickOutput;
  /** Canonical post-step boundary. */
  readonly snapshot: TSnapshot;
}

/** `sim.rollback_events`'s `RollbackWrappedEvent` (Rust-owned). The payload is opaque here. */
export interface RollbackWrappedEvent {
  readonly id: string;
  readonly tick: number;
  readonly domain: string;
  readonly ordinal: number;
  readonly payload: unknown;
}

export interface RollbackEventReplacement {
  readonly before: RollbackWrappedEvent;
  readonly after: RollbackWrappedEvent;
}

/** `sim.rollback_events`'s `RollbackEventDiff` (Rust-owned). */
export interface RollbackEventDiff {
  readonly added: readonly RollbackWrappedEvent[];
  readonly revoked: readonly RollbackWrappedEvent[];
  readonly replaced: readonly RollbackEventReplacement[];
}

export type RollbackEventsStatus = "active" | "unconfirmed_window_exceeded";

/** `sim.rollback_events`'s `RollbackEventsDiagnostics` (Rust-owned). */
export interface RollbackEventsDiagnostics {
  readonly status: RollbackEventsStatus;
  readonly confirmed_tick: number;
  readonly confirmed_boundary: number;
  readonly max_unconfirmed_ticks: number;
  readonly retained_step_count: number;
  readonly retained_event_count: number;
  readonly oldest_tick?: number;
  readonly latest_tick?: number;
}

/** `sim.rollback_events`'s `RollbackConfirmedStateView` (Rust-owned). */
export interface RollbackConfirmedStateView {
  readonly score: { readonly home: number; readonly away: number };
  readonly time_left: number;
  readonly finished: boolean;
  readonly owner_id?: string;
  readonly owner_team?: string;
}

/** `sim.rollback_events`'s `RollbackEventStep` (Rust-owned). */
export interface RollbackEventStep {
  readonly tick: number;
  readonly start_boundary: number;
  readonly end_boundary: number;
  readonly state: RollbackConfirmedStateView;
  readonly match_events: readonly RollbackWrappedEvent[];
  readonly combat_events?: readonly RollbackWrappedEvent[];
  readonly lifecycle_events: readonly RollbackWrappedEvent[];
}

export interface RollbackApplyFailure {
  readonly message: string;
  readonly code: "unconfirmed_window_exceeded";
}

export type RollbackApplyResult = Result<RollbackEventDiff, RollbackApplyFailure>;

/** `sim.rollback_events.new/apply/confirm/diagnostics`, injected -- see the module doc comment. */
export interface RollbackEventsPort<TTimeline, TSnapshot> {
  /** Mirrors `rollback_events.new`; renamed because `new` is a reserved word. */
  create(initialSnapshot: TSnapshot, maxUnconfirmedTicks: number): TTimeline;
  apply(
    timeline: TTimeline,
    replacedFromTick: number,
    replacedThroughTick: number,
    steps: readonly RollbackEventStepInput<TSnapshot>[]
  ): RollbackApplyResult;
  confirm(timeline: TTimeline, confirmedOutputTick: number): readonly RollbackEventStep[];
  diagnostics(timeline: TTimeline): RollbackEventsDiagnostics;
}

export type RollbackPlayableLabStatus = "active" | "unconfirmed_window_exceeded";

/** `sim.rollback_playable_lab`'s `RollbackPlayableCorrection` (Rust-owned). */
export interface RollbackPlayableCorrection {
  readonly causal_tick: number;
  readonly restored_boundary: number;
  readonly old_present_boundary: number;
  readonly new_present_boundary: number;
  readonly replaced_from_tick: number;
  readonly replaced_through_tick: number;
  readonly corrected_from_tick: number;
  readonly corrected_through_tick: number;
  readonly old_present_hash: string | null;
  readonly new_present_hash: string | null;
  readonly first_difference: unknown | null;
}

/** `sim.rollback_playable_lab`'s `RollbackPlayableLabBatch` (Rust-owned shape; produced here). */
export interface RollbackPlayableLabBatch {
  readonly outputs: RollbackTickOutput[];
  readonly event_diffs: RollbackEventDiff[];
  readonly confirmed_steps: RollbackEventStep[];
  readonly corrections: RollbackPlayableCorrection[];
  status: RollbackPlayableLabStatus;
}

/**
 * The presentation timeline's own state. Internal bookkeeping only --
 * mutated exclusively by this module's functions, exactly like the Lua
 * table it replaces (`_events`/`_first`/`_applied`/`_status` there).
 */
export interface OnlineMatchPresentation<TTimeline> {
  events: TTimeline;
  readonly first: number;
  applied: number;
  status: RollbackPlayableLabStatus;
}

function newBatch(): RollbackPlayableLabBatch {
  return { outputs: [], event_diffs: [], confirmed_steps: [], corrections: [], status: "active" };
}

export function newOnlineMatchPresentation<TTimeline, TSnapshot>(
  rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>,
  initialSnapshot: TSnapshot,
  firstInputTick: number,
  maxUnconfirmedTicks: number
): OnlineMatchPresentation<TTimeline> {
  return {
    events: rollbackEvents.create(initialSnapshot, maxUnconfirmedTicks),
    first: firstInputTick,
    applied: -1,
    status: "active",
  };
}

function eventStep<TDriver, TSnapshot>(
  presentation: OnlineMatchPresentation<unknown>,
  matchDriver: MatchDriverPort<TDriver, TSnapshot>,
  driver: TDriver,
  output: RollbackTickOutput
): RollbackEventStepInput<TSnapshot> {
  const lookup = matchDriver.snapshot(driver, output.end_boundary + presentation.first);
  if (lookup.status !== "present" && lookup.status !== "retained") {
    throw new Error("the online presentation needs a retained boundary snapshot");
  }
  if (lookup.snapshot === undefined) {
    throw new Error("retained boundary snapshot is missing");
  }
  return { output, snapshot: lookup.snapshot };
}

function applySteps<TTimeline, TSnapshot>(
  presentation: OnlineMatchPresentation<TTimeline>,
  rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>,
  batch: RollbackPlayableLabBatch,
  from: number,
  through: number,
  steps: readonly RollbackEventStepInput<TSnapshot>[]
): boolean {
  const applied = rollbackEvents.apply(presentation.events, from, through, steps);
  if (!applied.ok) {
    if (applied.error.code !== "unconfirmed_window_exceeded") {
      throw new Error(applied.error.message || "online event application failed");
    }
    presentation.status = "unconfirmed_window_exceeded";
    return false;
  }
  batch.event_diffs.push(structuredClone(applied.value));
  return true;
}

function publishConfirmation<TTimeline, TSnapshot>(
  presentation: OnlineMatchPresentation<TTimeline>,
  rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>,
  batch: RollbackPlayableLabBatch,
  confirmedOutputTick: number
): boolean {
  if (rollbackEvents.diagnostics(presentation.events).status !== "active") {
    presentation.status = "unconfirmed_window_exceeded";
    return false;
  }
  // Never confirm past what has been applied. The driver's confirmation
  // ceiling covers the whole batch, but the mid-batch confirmation after a
  // correction runs before this batch's fresh appends exist.
  const sessionTick = Math.min(confirmedOutputTick - presentation.first, presentation.applied);
  for (const step of rollbackEvents.confirm(presentation.events, sessionTick)) {
    batch.confirmed_steps.push(structuredClone(step));
  }
  return true;
}

export interface MatchPresentationPorts<TTimeline, TSnapshot, TDriver> {
  readonly rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>;
  readonly matchDriver: MatchDriverPort<TDriver, TSnapshot>;
}

// Consume one driver batch. `driver` is read only for its retained boundary
// snapshots and its confirmation ceiling; nothing here can move the
// simulation.
export function consume<TTimeline, TSnapshot, TDriver>(
  presentation: OnlineMatchPresentation<TTimeline>,
  ports: MatchPresentationPorts<TTimeline, TSnapshot, TDriver>,
  driver: TDriver,
  driverBatch: MatchDriverBatch
): RollbackPlayableLabBatch {
  const { rollbackEvents, matchDriver } = ports;
  const batch = newBatch();
  if (presentation.status !== "active") {
    batch.status = presentation.status;
    return batch;
  }
  const confirmedOutputTick = matchDriver.diagnostics(driver).confirmed_output_tick;
  const outputs = driverBatch.outputs;
  let index = 0;
  while (index < outputs.length) {
    const output = outputs[index];
    if (output === undefined) {
      break;
    }
    batch.outputs.push(structuredClone(output));
    if (output.tick > presentation.applied) {
      if (
        !applySteps(presentation, rollbackEvents, batch, output.tick, output.tick, [
          eventStep(presentation, matchDriver, driver, output),
        ])
      ) {
        batch.status = presentation.status;
        return batch;
      }
      presentation.applied = output.tick;
      index += 1;
    } else {
      // A correction run: contiguous corrected outputs replacing the whole
      // speculative tail from this tick through everything already
      // applied.
      const from = output.tick;
      const through = presentation.applied;
      const steps: RollbackEventStepInput<TSnapshot>[] = [];
      let expected = from;
      // Bounded by `through`: the corrected run replaces the speculative
      // tail and stops there. A fresh output for the next tick may follow
      // it in the same driver batch, and swallowing that into the
      // replacement would extend it past the interval being replaced.
      while (index < outputs.length && expected <= through) {
        const candidate = outputs[index];
        if (candidate === undefined || candidate.tick !== expected) {
          break;
        }
        if (expected !== from) {
          batch.outputs.push(structuredClone(candidate));
        }
        steps.push(eventStep(presentation, matchDriver, driver, candidate));
        expected += 1;
        index += 1;
      }
      if (!applySteps(presentation, rollbackEvents, batch, from, through, steps)) {
        batch.status = presentation.status;
        return batch;
      }
      // A correction may legally shorten the tail at full time, so the
      // applied ceiling follows the corrected run rather than the replaced
      // one; claiming a tick the timeline no longer holds would break the
      // next append.
      const correctedThrough = from + steps.length - 1;
      presentation.applied = correctedThrough;
      batch.corrections.push({
        causal_tick: from,
        restored_boundary: from,
        old_present_boundary: through + 1,
        new_present_boundary: correctedThrough + 1,
        replaced_from_tick: from,
        replaced_through_tick: through,
        corrected_from_tick: from,
        corrected_through_tick: correctedThrough,
        old_present_hash: null,
        new_present_hash: null,
        first_difference: null,
      });
      if (!publishConfirmation(presentation, rollbackEvents, batch, confirmedOutputTick)) {
        batch.status = presentation.status;
        return batch;
      }
    }
  }
  publishConfirmation(presentation, rollbackEvents, batch, confirmedOutputTick);
  batch.status = presentation.status;
  return batch;
}

export function diagnostics<TTimeline, TSnapshot>(
  presentation: OnlineMatchPresentation<TTimeline>,
  rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>
): RollbackEventsDiagnostics {
  return rollbackEvents.diagnostics(presentation.events);
}

export function status<TTimeline>(presentation: OnlineMatchPresentation<TTimeline>): RollbackPlayableLabStatus {
  return presentation.status;
}
