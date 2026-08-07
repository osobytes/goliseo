// Ported from spec/screens/match_rollback_lab_spec.lua's "playable rollback
// ScreenStack flow (tier 3)" describe block -- left `describe.skip`/
// `it.skip` in `@gc/screens` (packages/screens/src/match_rollback_lab.spec.ts)
// because both cases need `ScreenStack` (`@gc/app`'s `screen_stack.ts`)
// driving a real `MatchScreen`/`RealMatch` pair, and `@gc/screens` cannot
// depend on `@gc/app` -- the dependency runs the other way (v2/README.md
// §2/§9). `@gc/app` is the correct side of that edge; this file is where
// the cases live.
//
// # Re-checked for this task: the wasm bridge landed
//
// The header this replaces said both cases stayed blocked on a missing
// `@gc/wasm` binding for `sim.rollback_playable_lab`. That binding now
// exists (`crates/gc-wasm/src/rollback_playable_lab_bridge.rs`, exposed as
// `RollbackPlayableLab` off `loadSimHost()` -- `packages/wasm/src/types.ts`).
// `RealRollbackHost` below is a real `RollbackHostPort` (`@gc/screens`'s
// `match.ts`) implementation wrapping it: a fresh `SimSession` supplies the
// canonical slot-mode boundary-zero snapshot `RollbackPlayableLab.create`
// needs, `RollbackHostPort.step` drives `RollbackPlayableLab.advance` one
// transport tick at a time (encoding the local sample via
// `inputFrameNewSample`, gated by `needsLocalSample`), and
// `RollbackPlayableLab.currentMatchStateJson`/`debugModelJson` supply the
// raw `MatchState`/debug fields `RollbackHostPort`'s other methods need.
// This is the SAME "TS-glue-observable analog" pattern
// `match_rollback_lab.spec.ts` (`@gc/screens`) already establishes for
// `SimHostPort`/`FakeSimHost` -- except here the thing being wrapped is the
// real algorithm, not a fake, because a fake cannot produce a genuine
// convergence verdict (`_rollback_debug.status` reaching `"converged"`,
// matched snapshot hashes, `rollback_count`/`resimulated_ticks` > 0) --
// that would mean inventing convergence detection locally and asserting
// against the invention, exactly the anti-pattern this port must not commit
// (the same principle `online_match_flow.spec.ts`, this package, names for
// its own still-skipped cases).
//
// `RollbackHostFactory`/`RollbackHostPort` (`match.ts`) are this package's
// OWN design, not a fixed contract shared with anything else, so
// `RealRollbackHost` living entirely in this spec file (rather than a
// production module) is consistent with how `@gc/wasm` is scoped here: rule
// 4 of this task ("`@gc/wasm` is a devDependency of `@gc/screens` -- specs
// may import it directly, but production code must keep receiving these
// through injected ports") applies the same way to `@gc/app`, which also
// only takes `@gc/wasm` as a real build dependency for its OWN production
// `sim_host.ts` (a separate file, not owned by this batch) -- this spec
// importing it directly to build a real `RollbackHostPort` test double is
// exactly the sanctioned use.
//
// # The first case clears; the second stays blocked, for a DIFFERENT reason
//
// "converges under the checked-in playable profile with pinned seeds" needs
// nothing beyond a real `RollbackHostPort` and a real `ScreenStack` --
// cleared below, driving `RealRollbackHost` to actual `"converged"` /
// `"matched"` snapshot-hash agreement, the exact capability no fake in this
// codebase could produce.
//
// "reconciles a rollback goal through confirmed replay and result
// completion" additionally needs a HAND-SPECIFIED pinned `initial_snapshot`
// (`rollback_goal_fixture()` in the Lua original: a `MatchState` with the
// ball primed mid-flight into the net, `pickup_cd`/`block_grace` pinned so a
// goal scores almost immediately -- the fixture exists purely to make the
// goal deterministic and fast instead of waiting on bot-driven play). This
// is the EXACT gap `rollback_validation.spec.ts`'s own remaining skip
// documents in this same package: "`@gc/wasm` has no snapshot-construction
// entry point at all -- the only ways to obtain a `WasmMatchSnapshot` are
// `SimSession.snapshotHandle()` (whatever state a real, actually-stepped
// session reached) ... neither exposes anything resembling 'build a
// snapshot from these fields.'" Re-confirmed here by reading
// `session.rs`/`rollback_playable_lab_bridge.rs` end to end again for this
// task: still true. Driving REAL, undirected bot-vs-bot play for long
// enough to guarantee a goal within a bounded, fast test loop -- rather
// than a pinned fixture -- would be exactly the kind of "guessing at it
// risks a flaky, non-representative test even if it happens to pass once"
// `rollback_validation.spec.ts`'s header already warns against. So this
// case stays `it.skip`, for this narrower and different reason than the
// missing-wasm-bridge one this file's header used to give: it needs a
// snapshot-construction entry point `@gc/wasm` does not have, not a
// rollback-lab host implementation, which this file now has.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import type { FixedClock, RollbackPlayableLab, SimHost, WasmMatchSnapshot } from "@gc/wasm";
import type { RollbackEventDiff } from "@gc/presentation";
import type { replayTypes } from "@gc/render";
import { MatchScreen } from "@gc/screens";
import type {
  AudioPort,
  EffectsPort,
  InputSample,
  MatchScreenPorts,
  RenderFrame,
  RenderFrameRoster,
  RenderPort,
  ReplayPort,
  RollbackEventStep,
  RollbackHostFactory,
  RollbackHostPort,
  RollbackLabClockDebug,
  RollbackLabCorrectionSample,
  RollbackLabDisplayedPositions,
  RollbackLabHostDebug,
  RollbackLabOutput,
  RollbackLabSnapshotSummary,
} from "@gc/screens";
import { ScreenStack } from "./screen_stack.ts";

const TICK_SECONDS = 1 / 60;

// --- the raw shape RollbackPlayableLab.advance's batch JSON decodes to ----
// (see crates/gc-wasm/src/rollback_playable_lab_bridge.rs's `batch_json`/
// `output_json`/`correction_json` -- only the fields this file actually
// reads are declared; `event_diffs`/`confirmed_steps` are forwarded
// untouched, so they are cast straight to `@gc/screens`'s own wire types
// rather than re-declared here).
interface RawAdvanceOutput {
  readonly tick: number;
  readonly finished: boolean;
}
interface RawAdvanceCorrection {
  readonly causal_tick: number;
}
interface RawAdvanceBatch {
  readonly outputs: readonly RawAdvanceOutput[];
  readonly event_diffs: unknown;
  readonly confirmed_steps: unknown;
  readonly corrections: readonly RawAdvanceCorrection[];
  readonly status: string;
}

/**
 * A real {@link RollbackHostPort} over `@gc/wasm`'s `RollbackPlayableLab` --
 * see this file's header. Test-only: `RollbackHostPort` is this package's
 * own design (not a fixed production contract), so there is no reason for
 * this class to live outside the spec that needs it.
 */
class RealRollbackHost implements RollbackHostPort {
  private readonly host: SimHost;
  private readonly lab: RollbackPlayableLab;
  private readonly clock: FixedClock;
  private transportTick = 0;
  private finished = false;
  private currentState: replayTypes.MatchState;
  private disposed = false;

  constructor(host: SimHost, lab: RollbackPlayableLab) {
    this.host = host;
    this.lab = lab;
    this.clock = new host.FixedClock();
    this.currentState = JSON.parse(lab.currentMatchStateJson()) as replayTypes.MatchState;
  }

  planTicks(dt: number): number {
    return this.clock.advance(dt);
  }

  cancelPlannedTicks(): void {
    this.clock.stopEarly();
  }

  clockDebug(): RollbackLabClockDebug {
    // `@gc/wasm`'s real `FixedClock` only exposes `tick` -- see its own
    // doc (`packages/wasm/src/types.ts`); `dropped_ticks`/`overloads`/
    // `accumulator` have no readback on the real binding, unlike
    // `@gc/screens`'s own `FakeFixedClock` test double, which tracks them
    // itself. Not reintroducing that accumulator here (rule 5 of this
    // task) -- these three just report zero rather than being recomputed
    // in TypeScript.
    return { tick: this.clock.tick, dropped_ticks: 0, overloads: 0, accumulator: 0 };
  }

  step(sample: InputSample): RollbackLabStepResultLike {
    const sampleWire = this.lab.needsLocalSample()
      ? this.host.inputFrameNewSample(sample.move_x, sample.move_y, sample.held, sample.edges)
      : undefined;
    const batch = JSON.parse(this.lab.advance(this.transportTick, sampleWire)) as RawAdvanceBatch;
    this.transportTick += 1;
    this.currentState = JSON.parse(this.lab.currentMatchStateJson()) as replayTypes.MatchState;

    // `outputs` can legitimately be EMPTY: once the predicted client's own
    // simulation is done but the laboratory is still draining network
    // settlement (`status === "settling"`), an `advance` call may do
    // nothing but process/await network bookkeeping with no new
    // `RollbackTickOutput` to report -- not an error, and not something
    // `RollbackHostPort.step` (one required `output` per call) can leave
    // unfilled, so this falls back to the transport tick just requested.
    const lastOutput = batch.outputs[batch.outputs.length - 1];
    // NOT `lastOutput.finished` -- that is the PREDICTED client's own
    // match-clock reaching zero, which happens well before the laboratory
    // is done: `game/screens/match.lua`'s `match_is_over`/`lab_source.terminal`
    // define "over" for a rollback-driven match as `status ~= "active" and
    // status ~= "settling"` (this laboratory's own settlement/convergence
    // verdict), NOT the client's local finished flag. Using the client's
    // flag here was this file's own first-draft bug: `MatchScreen.finished`
    // (and this class's own `updateRollback` early return) would go true
    // the moment gameplay ended and PERMANENTLY stop calling `step`/
    // `advance` -- so the laboratory's settlement phase (which needs
    // several dozen MORE transport ticks after gameplay ends, see this
    // file's header) could never actually run, and `status` would stay
    // `"settling"` forever. `RollbackHostPort`/`RollbackLabHostDebug` are
    // this package's own design (not a fixed contract) precisely so this
    // is this file's call to get right.
    this.finished = batch.status !== "active" && batch.status !== "settling";

    // One slot, the local sample just supplied -- mirrors
    // `match_rollback_lab.spec.ts`'s (`@gc/screens`) own `FakeRollbackHost`
    // convention; the real batch's `input.slots` covers all eight canonical
    // slots (`RollbackInputSlotRecord`, wire-encoded), which nothing this
    // task's target tests read.
    const output: RollbackLabOutput = {
      tick: lastOutput?.tick ?? this.transportTick - 1,
      input: { slots: [{ sample }] },
    };

    let correction: RollbackLabCorrectionSample | undefined;
    const lastCorrection = batch.corrections[batch.corrections.length - 1];
    if (lastCorrection !== undefined) {
      correction = {
        tick: lastCorrection.causal_tick,
        source: {
          players: this.currentState.players.map((p) => ({ id: p.id, pos: p.pos })),
          ball: this.currentState.ball,
        },
      };
    }

    return {
      output,
      eventDiffs: batch.event_diffs as readonly RollbackEventDiff[],
      confirmedSteps: batch.confirmed_steps as readonly RollbackEventStep[],
      ...(correction !== undefined ? { correction } : {}),
      debug: this.debug(),
    };
  }

  debug(): RollbackLabHostDebug {
    return JSON.parse(this.lab.debugModelJson()) as RollbackLabHostDebug;
  }

  frame(): RenderFrame {
    return {
      hud: {
        finished: this.finished,
        controlled_owns_ball:
          this.currentState.owner !== undefined && this.currentState.owner === this.currentState.controlled,
        home_score: this.currentState.score.home,
        away_score: this.currentState.score.away,
        time_left: this.currentState.time_left,
      },
      possession: {},
    };
  }

  roster(): RenderFrameRoster {
    return {};
  }

  tick(): number {
    return this.transportTick;
  }

  displayedPositions(): RollbackLabDisplayedPositions {
    return {
      players: this.currentState.players.map((p) => ({ id: p.id, pos: p.pos })),
      ball: this.currentState.ball,
    };
  }

  // No combat companion this task -- every case below constructs its
  // `RollbackPlayableLab` from a plain `SimSession.snapshotHandle()`
  // (`combat_enabled` omitted, defaulting `false`), so there is never a
  // combat-bearing snapshot for this host to report.
  currentSnapshot(): RollbackLabSnapshotSummary {
    return {};
  }

  referenceSnapshot(): RollbackLabSnapshotSummary {
    return {};
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.lab.free();
  }
}

// `RollbackHostPort.step`'s real return shape -- named locally so
// `RealRollbackHost.step`'s signature does not have to re-import
// `RollbackLabStepResult` merely to widen it (`eventDiffs`/`confirmedSteps`
// above are read off parsed JSON, `unknown` until cast, so this file's own
// alias is clearer than fighting the imported type at the call site).
interface RollbackLabStepResultLike {
  readonly output: RollbackLabOutput;
  readonly eventDiffs: readonly RollbackEventDiff[];
  readonly confirmedSteps: readonly RollbackEventStep[];
  readonly correction?: RollbackLabCorrectionSample;
  readonly debug: RollbackLabHostDebug;
}

interface RealRollbackHostOptions {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly seed: number;
  readonly local_slot: number;
  readonly profile_name: string;
  readonly network_seed?: number;
  readonly bot_seed?: number;
  /** Forwarded to the fresh `SimSession`'s own `durationSeconds` -- mirrors `game/screens/match.lua`'s `Match:restart` passing `rollback_options.duration` straight to `sim_match.new`, NOT to `RollbackPlayableLab`'s own options (that binding has no `duration` field; see `rollback_playable_lab_bridge.rs`'s `decode_options`). Defaults to 120, `sim/match.lua`'s own default. */
  readonly duration?: number;
  readonly max_goals?: number;
  readonly settlement_ticks?: number;
  readonly max_rollback_ticks?: number;
}

/** Builds a real {@link RollbackHostFactory}: a fresh `SimSession` supplies the canonical boundary-zero snapshot, then a fresh `RollbackPlayableLab` wraps it -- see this file's header. */
function createRealRollbackHostFactory(options: RealRollbackHostOptions): RollbackHostFactory {
  return (): RollbackHostPort => {
    const host = loadSimHost();
    const session = new host.Session(
      options.home_team_id,
      options.away_team_id,
      options.seed,
      options.duration ?? 120,
      options.max_goals ?? 99,
    );
    const initialSnapshot: WasmMatchSnapshot = session.snapshotHandle();
    session.free();
    const optionsJson = JSON.stringify({
      local_slot: options.local_slot,
      profile_name: options.profile_name,
      ...(options.network_seed !== undefined ? { network_seed: options.network_seed } : {}),
      ...(options.bot_seed !== undefined ? { bot_seed: options.bot_seed } : {}),
      ...(options.max_rollback_ticks !== undefined ? { max_rollback_ticks: options.max_rollback_ticks } : {}),
      ...(options.settlement_ticks !== undefined ? { settlement_ticks: options.settlement_ticks } : {}),
    });
    const lab = host.RollbackPlayableLab.create(initialSnapshot, optionsJson);
    initialSnapshot.free();
    return new RealRollbackHost(host, lab);
  };
}

function noopEffects(): EffectsPort {
  return {
    reset(): void {},
    resetVisuals(): void {},
    resetTrail(): void {},
    applyEventDiff(): void {},
    discardEventDiff(): void {},
    confirmEvent(): void {},
  };
}

function noopAudio(): AudioPort {
  return { consumeConfirmed: (): boolean => true };
}

function noopReplay(): ReplayPort {
  return { active: (): boolean => false, startAt: (): boolean => false };
}

const noopRenderer: RenderPort = { draw: (): void => {} };

describe("playable rollback ScreenStack flow (tier 3)", () => {
  it("converges under the checked-in playable profile with pinned seeds", () => {
    const ports: MatchScreenPorts = {
      createHost: () => {
        throw new Error("base createHost should not be called for a rollback-lab screen");
      },
      renderer: noopRenderer,
      keyboard: { isDown: () => false },
      createRollbackHost: createRealRollbackHostFactory({
        home_team_id: "nebula",
        away_team_id: "orion",
        seed: 1,
        local_slot: 1,
        profile_name: "playable",
        network_seed: 7302,
        bot_seed: 7400,
        duration: 12 * TICK_SECONDS,
        settlement_ticks: 128,
      }),
      effects: noopEffects(),
      audio: noopAudio(),
      replay: noopReplay(),
    };
    const screen = new MatchScreen(ports, {
      rollback_lab: { local_slot: 1, profile_name: "playable" },
    });
    const stack = new ScreenStack();
    stack.push(screen);

    let sawCorrection = false;
    let sawSmoothing = false;
    let sawSettling = false;
    for (let i = 0; i < 160; i += 1) {
      stack.update(TICK_SECONDS);
      sawCorrection = sawCorrection || screen.debugRollbackCorrections.length > 0;
      const frameDebug = screen.debugRollbackDebug()!;
      if (!sawSmoothing && screen.debugRollbackCorrections.length > 0 && frameDebug.active_smoothing_count > 0) {
        sawSmoothing = true;
        const magnitudeBefore = frameDebug.correction_magnitude;
        stack.update(TICK_SECONDS / 4);
        const settledDebug = screen.debugRollbackDebug()!;
        expect(screen.debugRollbackCorrections.length).toBe(0);
        expect(settledDebug.correction_magnitude).toBeLessThan(magnitudeBefore);
        sawSettling = true;
      }
      const status = screen.debugRollbackDebug()!.status;
      if (status !== "active" && status !== "settling") {
        break;
      }
    }

    const debug = screen.debugRollbackDebug()!;
    expect(sawCorrection, "the client and reference must diverge and correct at least once").toBe(true);
    expect(sawSmoothing, "a landed correction must produce active render smoothing").toBe(true);
    expect(sawSettling, "render-only settling must observably decay the correction magnitude").toBe(true);
    expect(debug.rollback_count).toBeGreaterThan(0);
    expect(debug.resimulated_ticks ?? 0).toBeGreaterThan(0);
    expect(debug.status).toBe("converged");
    expect(debug.convergence?.status).toBe("matched");
    expect(debug.reference_tick).toBe(debug.current_tick);
    const current = screen.debugRollbackCurrentSnapshot();
    const reference = screen.debugRollbackReferenceSnapshot();
    // Neither snapshot summary carries a combat companion this task (no
    // `combat_enabled` option) -- the ported spec's own equivalent
    // assertion (`current.state.input_tick == reference.state.input_tick`,
    // `match_snapshot.hash(current) == match_snapshot.hash(reference)`)
    // reads the raw `MatchSnapshot` these summaries deliberately do not
    // expose (`RollbackLabSnapshotSummary`'s own doc: "Never the raw
    // MatchSnapshot itself"). `debug.convergence` above -- driven by the
    // SAME snapshot-hash comparison `gc_sim::rollback_playable_lab` performs
    // internally -- is this port's real analog: `"matched"` IS "the client
    // and reference boundary hashes agree".
    expect(current).toEqual(reference);
    expect(debug.convergence?.actual_hash).toBe(debug.convergence?.expected_hash);
    expect(debug.confirmed_input_tick).toBe(debug.reference_tick - 1);
    expect(debug.confirmed_output_tick).toBe(debug.reference_tick - 1);
    expect(debug.network_pending).toBe(0);
    expect(debug.active_smoothing_count).toBe(0);
    expect(debug.correction_magnitude).toBeCloseTo(0);

    screen.dispose();
  });

  // Stays skipped -- needs a hand-specified pinned `initial_snapshot`
  // `@gc/wasm` has no entry point to build. See this file's header for the
  // full re-check, and `rollback_validation.spec.ts`'s own remaining skip
  // for the identical, already-documented gap.
  it.skip(
    "reconciles a rollback goal through confirmed replay and result completion [needs a hand-specified pinned WasmMatchSnapshot -- @gc/wasm has no snapshot-construction entry point (SimSession.snapshotHandle/MatchDriverBridge.initialSnapshotHandle only capture whatever state a real, already-stepped session/driver reached); see this file's header and rollback_validation.spec.ts's identical, already-documented gap]",
    () => {},
  );
});
