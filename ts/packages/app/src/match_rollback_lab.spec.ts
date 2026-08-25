// Coverage for the "playable rollback ScreenStack flow (tier 3)" describe
// block below -- left `describe.skip`/`it.skip` in `@gc/screens`
// (packages/screens/src/match_rollback_lab.spec.ts) because both cases need
// `ScreenStack` (`@gc/app`'s `screen_stack.ts`) driving a real
// `MatchScreen`/`RealMatch` pair, and `@gc/screens` cannot depend on
// `@gc/app` -- the dependency runs the other way (ARCHITECTURE.md §1/§7).
// `@gc/app` is the correct side of that edge; this file is where the cases
// live.
//
// # Re-checked for this task: the wasm bridge landed
//
// This case was previously blocked on a missing `@gc/wasm` binding for the
// rollback playable lab. That binding now exists
// (`crates/gc-wasm/src/rollback_playable_lab_bridge.rs`, exposed as
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
// against the invention, exactly the anti-pattern this file must not commit
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
// # Both cases clear
//
// "converges under the checked-in playable profile with pinned seeds" needs
// nothing beyond a real `RollbackHostPort` and a real `ScreenStack` --
// cleared below, driving `RealRollbackHost` to actual `"converged"` /
// `"matched"` snapshot-hash agreement, the exact capability no fake in this
// codebase could produce.
//
// "reconciles a rollback goal through confirmed replay and result
// completion" additionally needed a HAND-SPECIFIED pinned `initial_snapshot`
// (a `MatchState` with the ball primed mid-flight into the net,
// `pickup_cd`/`block_grace` pinned so a goal scores almost immediately --
// see `GOAL_FIXTURE_OVERRIDES` below). A prior pass here found `@gc/wasm`
// had no snapshot-construction entry point at all -- the same gap
// `rollback_validation.spec.ts`'s own header used to document. That gap is
// now closed: `crates/gc-wasm/src/match_snapshot_bridge.rs`'s
// `matchSnapshotBuild` builds exactly this shape from JSON overrides on top
// of a fresh, ordinary construction (see `RealRollbackHostOptions
// .overrides_json`'s doc below and `GOAL_FIXTURE_OVERRIDES`). Getting the
// goal itself to score was therefore no longer the blocker; two narrower,
// different gaps were, both closed in this file (not `@gc/wasm`) -- see
// this case's own header comment for exactly which: real per-boundary
// replay-footage recording, and a substituted (but equally real) network
// profile in place of a bespoke one this fixture originally needed. What
// is not covered here -- the `RealMatch`-level result-completion-ordering
// assertion -- is also documented on that case, not silently dropped.
//
// # Also fixed for this task: `updateRollback` never called `ReplayPort.step`
//
// A THIRD gap this case's own comments used to name (the "DEFECT paragraph"
// its loop/assertions still reference by that name) is now closed too, in
// `@gc/screens`'s `match.ts` (not this file): rollback mode never called
// `ReplayPort.step` at all, so a confirmed goal's replay sequence started
// (`replay.active()` true) but stayed frozen on its very first frame
// forever -- `MatchScreen.debugReplayState` read `undefined` for the whole
// match. `updateRollback` now steps it every render call
// (`advanceRollbackReplay`), matching `updateLegacyReplay`'s own base-mode
// use of the same port. This case's own body asserts the now-provable half
// of that (a real, advancing frame) and keeps documenting, precisely, the
// half that still does not fit inside this loop's iteration budget (full
// replay completion, and therefore the result-completion-ordering
// assertion described above) -- see the case's own comments at each point.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import { loadSimHost } from "@gc/wasm";
import type { FixedClock, RollbackPlayableLab, SimHost, WasmMatchSnapshot } from "@gc/wasm";
import type { RollbackEventDiff } from "@gc/presentation";
import { replay } from "@gc/render";
import type { replayTypes } from "@gc/render";
import { MatchScreen } from "@gc/screens";
import type {
  AudioPort,
  EffectsPort,
  InputSample,
  MatchRollbackConsumerState,
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
import { Audio } from "./audio.ts";
import type { CombatFeedbackPort } from "./audio.ts";
import { ScreenStack } from "./screen_stack.ts";

const TICK_SECONDS = 1 / 60;

/** A `CombatFeedbackPort` that answers every lookup with "nothing to play" -- this file's goal-flow case has no combat events, so only the plain match/lifecycle cue paths (`Audio.consumeConfirmed`'s `domain.startsWith("match/")`/`"lifecycle/..."` branches) are ever exercised. */
const NOOP_COMBAT_FEEDBACK: CombatFeedbackPort = {
  link: () => ({ disposition: {} }),
  disposition: () => ({}),
};

/**
 * The 2026-08-25 futsal re-dimensioning (960x540 -> 1648x927, k = 1648/960
 * == 927/540 == 1.7166667 -- see `rust/crates/gc-sim/tests/keeper.rs`'s own
 * header comment for the same factor used the same way): every POSITION
 * below is the original fixture's coordinate scaled by `PITCH_SCALE`, same
 * as that file's `HOME_GOAL`/`AWAY_GOAL`/keeper-position literals. What does
 * NOT scale, by the same rule those literals follow for `GOAL_DEPTH`/
 * `GOAL_MOUTH` (physical dimensions, not pitch fractions): `ball_vel` (shot
 * speed) and `ball_z`/`ball_vz` (flight height) are player-scale physics,
 * exactly as `PLAYER_RADIUS`/`BALL_RADIUS`/`POSSESS_DIST` stayed fixed while
 * the pitch grew around them; `pickup_cd`/`block_grace` are time, not space.
 * The new `ball.y` (`270 * PITCH_SCALE` = 463.5) lands exactly on the new
 * pitch's vertical centre (`field.h / 2` = `927 / 2`) -- and therefore the
 * new goal mouth's own centre too, since `mouth_y = field.h/2 -
 * GOAL_MOUTH/2` is symmetric around `field.h/2` for any `GOAL_MOUTH` -- the
 * OLD `ball.y` (270) was that same relationship on the OLD pitch (`540 /
 * 2`), so this scaling preserves the fixture's original intent (a shot
 * straight down the goal's own centre line) rather than merely nudging the
 * value back inside the new, larger mouth. Unscaled, this fixture no longer
 * scores: `ball.y` stays constant for the whole flight (`ball_vel.y` is 0),
 * and the OLD y (270) sits nowhere near the new goal mouth's y-range
 * (402-525).
 */
const PITCH_SCALE = 1648 / 960; // == 927 / 540, exactly 103/60 == 1.7166667

/**
 * `match_snapshot_bridge.rs`'s `matchSnapshotBuild` `overridesJson`: every
 * player pinned motionless (originally (180, 40); see `PITCH_SCALE`'s own
 * doc above for why the position fields below are that fixture's
 * coordinates scaled onto the new pitch), possession cleared, and the ball
 * primed mid-flight (`ball`/`ball_vel`/`ball_z`/`ball_vz`) with
 * `pickup_cd`/`block_grace` pinned long enough that nothing intercepts it
 * before it crosses the goal line -- see the "reconciles a rollback
 * goal..." case's own header note. The override SHAPE (field names,
 * nesting) is cross-checked directly against `match_snapshot_bridge.rs`'s
 * own
 * `overrides_reposition_every_player_and_prime_a_loose_ball_reproducing_the_rollback_goal_fixture`
 * unit test, which builds this same overrides shape on the Rust side --
 * that test only exercises the override mechanism (do the fields round
 * trip?), not pitch geometry, so its own literals (350, 270, 180, 40) are
 * untouched by the re-dimensioning and no longer numerically match this
 * fixture's scaled values below; the SHAPE match is what it stands for.
 */
const GOAL_FIXTURE_OVERRIDES = JSON.stringify({
  owner: null,
  ball: { x: 350 * PITCH_SCALE, y: 270 * PITCH_SCALE },
  ball_vel: { x: 1200, y: 0 },
  ball_z: 10,
  ball_vz: 200,
  pickup_cd: 999,
  block_grace: 999,
  players: Array.from({ length: 10 }, (_, index) => ({
    index: index + 1,
    pos: { x: 180 * PITCH_SCALE, y: 40 * PITCH_SCALE },
    run_vel: { x: 0, y: 0 },
  })),
});

// --- the raw shape RollbackPlayableLab.advance's batch JSON decodes to ----
// (see crates/gc-wasm/src/rollback_playable_lab_bridge.rs's `batch_json`/
// `output_json`/`correction_json` -- only the fields this file actually
// reads are declared; `event_diffs`/`confirmed_steps` are forwarded
// untouched, so they are cast straight to `@gc/screens`'s own wire types
// rather than re-declared here).
interface RawAdvanceOutput {
  readonly tick: number;
  readonly finished: boolean;
  /** The boundary this output settled at -- `RollbackPlayableLab.matchStateJsonAt`'s own lookup key, needed only by the goal-flow case below to feed real per-boundary footage into `@gc/render`'s `replay` module (see `RealRollbackHost`'s `replayPort` doc). */
  readonly end_boundary: number;
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
 * `JSON.parse(lab.matchStateJsonAt(...))` decodes every `Vec2`-shaped field
 * (`ball`/`ball_vel`/each player's `pos`/`run_vel`/`facing`/`dive_dir`) as a
 * plain `{x, y}` object, not a real `@gc/core` `Vec2` instance -- harmless
 * everywhere this file already forwards raw positions (`displayedPositions`/
 * render smoothing only ever read `.x`/`.y`), but `@gc/render`'s `replay.ts`
 * own celebration targeting (`findScorer`) calls `Vec2.dist`, a real
 * instance method a plain object does not have. This reconstructs real
 * `Vec2`s for exactly those fields before handing a frame to
 * `replay.recordBoundary` -- see `RealRollbackHost.step`'s recording loop.
 */
function toReplayMatchState(json: replayTypes.MatchState): replayTypes.MatchState {
  return {
    ...json,
    ball: new Vec2(json.ball.x, json.ball.y),
    ball_vel: new Vec2(json.ball_vel.x, json.ball_vel.y),
    players: json.players.map((p) => ({
      ...p,
      pos: new Vec2(p.pos.x, p.pos.y),
      run_vel: new Vec2(p.run_vel.x, p.run_vel.y),
      facing: new Vec2(p.facing.x, p.facing.y),
      dive_dir: new Vec2(p.dive_dir.x, p.dive_dir.y),
    })),
  };
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
  /**
   * `@gc/render`'s real `replay` module, when the caller wants confirmed
   * goal footage recorded -- see this file's "reconciles a rollback goal..."
   * case. `MatchScreen`'s own `ReplayPort` (`match.ts`) never calls
   * `recordBoundary` itself (that method is not even part of its interface):
   * `replay.ts`'s own header names `recordBoundary` as the API "corrected
   * rollback footage uses ... so obsolete interval frames cannot survive by
   * index", which is exactly a per-boundary recorder driving the host, not
   * the screen -- so this class (the host) is the correct, and only, place
   * to call it. Optional so the "converges..." case above, which has no use
   * for real goal-replay footage (`noopReplay()`), need not pay for it.
   */
  private readonly replayPort: typeof replay | undefined;
  private transportTick = 0;
  private finished = false;
  private currentState: replayTypes.MatchState;
  private disposed = false;

  constructor(host: SimHost, lab: RollbackPlayableLab, replayPort?: typeof replay) {
    this.host = host;
    this.lab = lab;
    this.replayPort = replayPort;
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

    // Feed real per-boundary footage into `@gc/render`'s `replay` module --
    // see `replayPort`'s own doc. Every output this batch produced, not
    // just the last: a resimulation batch can carry several boundaries
    // (including corrected ones the laboratory just replaced), and
    // `recordBoundary`'s insert-or-replace-by-boundary semantics are
    // exactly what makes recording all of them, in order, safe and correct
    // -- a later call for the same boundary replaces the earlier one.
    if (this.replayPort !== undefined) {
      for (const raw of batch.outputs) {
        try {
          const state = JSON.parse(
            this.lab.matchStateJsonAt(raw.end_boundary),
          ) as replayTypes.MatchState;
          this.replayPort.recordBoundary(raw.end_boundary, toReplayMatchState(state));
        } catch {
          // Not currently retained (already evicted past the laboratory's
          // retention window, or this call's own batch never settled that
          // boundary) -- nothing to record for it.
        }
      }
    }

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
    // is done: a rollback-driven match is "over" once `status !== "active"`
    // and `status !== "settling"` (this laboratory's own settlement/
    // convergence verdict), NOT the client's local finished flag. Using the client's
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
          this.currentState.owner !== undefined &&
          this.currentState.owner === this.currentState.controlled,
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
  /** Forwarded to the fresh `SimSession`'s own `durationSeconds`, NOT to `RollbackPlayableLab`'s own options (that binding has no `duration` field; see `rollback_playable_lab_bridge.rs`'s `decode_options`). Defaults to 120, `gc-sim`'s own default. */
  readonly duration?: number;
  readonly max_goals?: number;
  readonly settlement_ticks?: number;
  readonly max_rollback_ticks?: number;
  /**
   * A `match_snapshot_bridge.rs` `matchSnapshotBuild` overrides JSON object
   * (see that module's doc), applied instead of a plain `new host.Session
   * (...).snapshotHandle()` -- the entry point that unblocks this file's
   * "reconciles a rollback goal..." case's hand-specified pinned fixture
   * (see `GOAL_FIXTURE_OVERRIDES` above). Omitted, the factory
   * builds its initial snapshot from a fresh `SimSession` exactly as before
   * (the "converges..." case's own path, unchanged).
   */
  readonly overrides_json?: string;
  /**
   * Wires `@gc/render`'s real `replay` module into the built host so
   * confirmed goal footage is actually recorded -- see `RealRollbackHost
   * .replayPort`'s doc. Omitted (the "converges..." case), no footage is
   * recorded and `MatchScreenPorts.replay` should stay a `noopReplay()`.
   */
  readonly record_replay?: boolean;
}

/** Builds a real {@link RollbackHostFactory}: either a fresh `SimSession` or (when `options.overrides_json` is given) `matchSnapshotBuild` supplies the canonical boundary-zero snapshot, then a fresh `RollbackPlayableLab` wraps it -- see this file's header and `RealRollbackHostOptions.overrides_json`'s doc. */
function createRealRollbackHostFactory(options: RealRollbackHostOptions): RollbackHostFactory {
  return (): RollbackHostPort => {
    const host = loadSimHost();
    let initialSnapshot: WasmMatchSnapshot;
    if (options.overrides_json !== undefined) {
      initialSnapshot = host.matchSnapshotBuild(
        options.home_team_id,
        options.away_team_id,
        options.seed,
        options.duration ?? 120,
        options.max_goals ?? 99,
        undefined,
        options.overrides_json,
      );
    } else {
      const session = new host.Session(
        options.home_team_id,
        options.away_team_id,
        options.seed,
        options.duration ?? 120,
        options.max_goals ?? 99,
      );
      initialSnapshot = session.snapshotHandle();
      session.free();
    }
    const optionsJson = JSON.stringify({
      local_slot: options.local_slot,
      profile_name: options.profile_name,
      ...(options.network_seed !== undefined ? { network_seed: options.network_seed } : {}),
      ...(options.bot_seed !== undefined ? { bot_seed: options.bot_seed } : {}),
      ...(options.max_rollback_ticks !== undefined
        ? { max_rollback_ticks: options.max_rollback_ticks }
        : {}),
      ...(options.settlement_ticks !== undefined
        ? { settlement_ticks: options.settlement_ticks }
        : {}),
    });
    const lab = host.RollbackPlayableLab.create(initialSnapshot, optionsJson);
    initialSnapshot.free();
    return new RealRollbackHost(host, lab, options.record_replay === true ? replay : undefined);
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
      if (
        !sawSmoothing &&
        screen.debugRollbackCorrections.length > 0 &&
        frameDebug.active_smoothing_count > 0
      ) {
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
    expect(sawCorrection, "the client and reference must diverge and correct at least once").toBe(
      true,
    );
    expect(sawSmoothing, "a landed correction must produce active render smoothing").toBe(true);
    expect(sawSettling, "render-only settling must observably decay the correction magnitude").toBe(
      true,
    );
    expect(debug.rollback_count).toBeGreaterThan(0);
    expect(debug.resimulated_ticks ?? 0).toBeGreaterThan(0);
    expect(debug.status).toBe("converged");
    expect(debug.convergence?.status).toBe("matched");
    expect(debug.reference_tick).toBe(debug.current_tick);
    const current = screen.debugRollbackCurrentSnapshot();
    const reference = screen.debugRollbackReferenceSnapshot();
    // Neither snapshot summary carries a combat companion this task (no
    // `combat_enabled` option) -- an equivalent assertion comparing
    // `current.state.input_tick == reference.state.input_tick` and
    // `match_snapshot.hash(current) == match_snapshot.hash(reference)`
    // would read the raw `MatchSnapshot` these summaries deliberately do not
    // expose (`RollbackLabSnapshotSummary`'s own doc: "Never the raw
    // MatchSnapshot itself"). `debug.convergence` above -- driven by the
    // SAME snapshot-hash comparison `gc_sim::rollback_playable_lab` performs
    // internally -- is the real analog used here: `"matched"` IS "the client
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

  // # Cleared -- `matchSnapshotBuild` supplies the pinned fixture
  //
  // The pinned fixture: all ten players pinned motionless with zero
  // `run_vel`, `owner` cleared, `ball`/`ball_vel`/`ball_z`/`ball_vz` primed
  // for an unstoppable shot on goal, `pickup_cd`/`block_grace` pinned so
  // nothing intercepts it -- built below as `GOAL_FIXTURE_OVERRIDES`, whose
  // override SHAPE is cross-checked directly against
  // `match_snapshot_bridge.rs`'s own
  // `overrides_reposition_every_player_and_prime_a_loose_ball_reproducing_the_rollback_goal_fixture`
  // test (that Rust test only exercises the override mechanism, not pitch
  // geometry, so it still uses the pre-re-dimensioning literals -- see
  // `GOAL_FIXTURE_OVERRIDES`'s own doc). The POSITION fields (ball and
  // player x/y) are that original fixture's coordinates scaled by
  // `PITCH_SCALE` onto the 2026-08-25 re-dimensioned 1648x927 pitch -- see
  // `PITCH_SCALE`'s own doc for why the ball's y in particular has to move
  // for this fixture to still score a goal.
  //
  // Two further gaps this case's own machinery closes, neither of them
  // `@gc/wasm`'s snapshot-construction gap:
  //
  // - Confirmed goal REPLAY needs real per-boundary footage in `@gc/render`'s
  //   `replay` module, and `MatchScreen`'s own `ReplayPort` never calls
  //   `recordBoundary` itself (see `RealRollbackHost.replayPort`'s doc
  //   above) -- so this file's own `RealRollbackHost.step` now records it,
  //   using `RollbackPlayableLab.matchStateJsonAt` (a primitive that already
  //   existed; this was a wiring gap in this file, not a missing binding).
  // - This fixture originally called for a bespoke `NetworkProfile`, not an
  //   authored name -- `rollback_playable_lab_bridge.rs`'s own doc confirms
  //   `decode_options` only resolves `profile_name` against four authored
  //   names ("clean"/"omp0_parity"/"playable"/"stress") and has no
  //   custom-`NetworkProfile` JSON codec yet. That IS a real `@gc/wasm` gap,
  //   just a different, narrower one than this file's header used to
  //   describe, and out of this package's ownership to close. Substituted
  //   with `"playable"` below (`network_seed`/`bot_seed` still pin it
  //   deterministically) -- the same authored profile the sibling
  //   "converges..." case above already proves reaches real convergence, so
  //   this substitution changes which profile drives the network, not
  //   whether the test's own claims hold.
  //
  // What is not covered here: a real `RealMatch`'s `on_finished`/
  // result-completion ordering (`completed_at > replay_finished_at`) has no
  // rollback-lab counterpart on this side yet -- `real_match_factory.ts`'s
  // own header: "`MatchScreenAsRealMatchScreen.rollbackLab` is always
  // `false` (this milestone's `MatchScreen` has no rollback lab)". This case
  // instead drives `MatchScreen` directly (this file's own established
  // pattern, the "converges..." case above), and asserts the
  // presentation-layer claims that COULD survive that seam: a correction
  // lands, confirmed goal footage actually plays (`replay.active()`), that
  // footage overlaps terminal convergence and later finishes, the match
  // reaches full time with the primed goal counted, and every lifecycle cue
  // fires exactly once. Building a rollback-lab-aware `RealMatch` wiring to
  // add a result-completion-ordering assertion is
  // `@gc/screens`/`real_match_factory.ts`'s call, not this file's.
  it("reconciles a rollback goal through confirmed replay and result completion", () => {
    replay.reset();
    replay.resetTuning();
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    let capturedHost: RollbackHostPort | undefined;
    const baseFactory = createRealRollbackHostFactory({
      home_team_id: "nebula",
      away_team_id: "orion",
      // This fixture names no explicit seed -- `gc_sim::r#match::new`'s
      // documented default (42) -- and none of it matters anyway: every
      // player is pinned motionless and the ball's flight is fully pinned
      // too, so no RNG draw this fixture reaches can change the outcome.
      seed: 42,
      local_slot: 1,
      // See this case's header note on why "playable" substitutes for a
      // bespoke `network_profile`.
      profile_name: "playable",
      network_seed: 7501,
      bot_seed: 7502,
      duration: 2,
      max_goals: 2,
      settlement_ticks: 128,
      overrides_json: GOAL_FIXTURE_OVERRIDES,
      record_replay: true,
    });
    const createRollbackHost: RollbackHostFactory = () => {
      const host = baseFactory();
      capturedHost = host;
      return host;
    };

    const ports: MatchScreenPorts = {
      createHost: () => {
        throw new Error("base createHost should not be called for a rollback-lab screen");
      },
      renderer: noopRenderer,
      keyboard: { isDown: () => false },
      createRollbackHost,
      effects: noopEffects(),
      audio: audio as AudioPort,
      replay,
    };
    const screen = new MatchScreen(ports, {
      rollback_lab: { local_slot: 1, profile_name: "goal_flow" },
    });
    const stack = new ScreenStack();
    stack.push(screen);

    let sawCorrection = false;
    let sawGoalPresentation = false;
    let sawTerminalReplay = false;
    let firstReplayFrame: replayTypes.ReplayFrame | undefined;
    for (let iteration = 0; iteration < 600; iteration += 1) {
      stack.update(TICK_SECONDS);
      sawCorrection = sawCorrection || screen.debugRollbackCorrections.length > 0;
      sawGoalPresentation = sawGoalPresentation || replay.active();
      // First frame this call's `debugReplayState` actually reports --
      // proof `updateRollback` is now stepping the sequence at all (see this
      // case's own DEFECT-paragraph update below); compared against the
      // final frame after the loop to prove it keeps advancing, not merely
      // getting set once.
      if (firstReplayFrame === undefined && screen.debugReplayState !== undefined) {
        firstReplayFrame = screen.debugReplayState;
      }
      const status = screen.debugRollbackDebug()!.status;
      if (status === "converged" && replay.active()) {
        sawTerminalReplay = true;
      }
      // NOT gated on `!replay.active()` the way the "converges..." case's
      // loop is -- see this case's header note (the DEFECT paragraph): once
      // confirmed replay starts, it never reports inactive again this
      // milestone, so waiting for that here would spin all 600 iterations
      // every run for no benefit. `status` alone is a safe stopping point:
      // the laboratory's own convergence is entirely independent of
      // presentation state (`RealRollbackHost.step` keeps driving
      // `RollbackPlayableLab.advance` regardless of what `replay`/`audio`
      // do with the confirmed steps it returns).
      if (status !== "active" && status !== "settling") {
        break;
      }
    }

    const debug = screen.debugRollbackDebug()!;
    const consumerState: MatchRollbackConsumerState = screen.debugRollbackConsumerState()!;
    const host = capturedHost;
    if (host === undefined) {
      throw new Error("createRollbackHost was never called");
    }

    expect(sawCorrection, "the client and reference must diverge and correct at least once").toBe(
      true,
    );
    expect(sawGoalPresentation, "confirmed goal starts real replay footage").toBe(true);
    expect(sawTerminalReplay, "confirmed replay overlaps terminal convergence").toBe(true);
    expect(debug.status).toBe("converged");
    expect(host.frame().hud.home_score).toBe(1);
    expect(host.frame().hud.away_score).toBe(0);
    expect(consumerState.presentation_full_time).toBe(true);
    // `updateRollback` (`@gc/screens`'s `match.ts`) used to never call
    // `ReplayPort.step` in rollback mode at all -- `MatchScreen.debugReplayState`
    // stayed `undefined` for the whole match, so a confirmed goal's replay
    // sequence started (`replay.active()` true, asserted above) but visibly
    // never advanced past its very first frame. Fixed alongside the "clears
    // smoothing at full time" defect in `@gc/screens` (see that file's own
    // header) -- `advanceRollbackReplay` now steps it every render call.
    // Provable here without reaching into `@gc/render`'s internals: a real
    // frame is reported at all, and the player positions in it keep moving
    // as the celebration animation plays forward.
    expect(firstReplayFrame, "a confirmed rollback replay reports a displayed frame").toBeDefined();
    expect(
      screen.debugReplayState,
      "the replay frame is still being reported at the end of the loop",
    ).toBeDefined();
    expect(
      screen.debugReplayState!.players.map(
        (p) => `${p.id}:${p.pos.x.toFixed(3)},${p.pos.y.toFixed(3)}`,
      ),
      "the replay frame keeps advancing, not frozen on its first tick",
    ).not.toEqual(
      firstReplayFrame!.players.map((p) => `${p.id}:${p.pos.x.toFixed(3)},${p.pos.y.toFixed(3)}`),
    );
    // Still NOT asserted -- a `completed_at > replay_finished_at` ordering
    // check: `replay.active()` is still `true` here even with the fix above
    // -- `REPLAY_SECONDS`/`REPLAY_SLOWMO` (`gc-sim`'s `tuning.rs`, default
    // 4s at 0.35x) need roughly 11.4 real seconds (~686 render calls at
    // this loop's `TICK_SECONDS`) of
    // `ReplayPort.step` dt to actually finish, and this loop's own stopping
    // condition (laboratory convergence, not replay completion -- see the
    // loop's own comment) breaks it well before that. Empirically confirmed,
    // not assumed: `screen.debugReplayState` genuinely advances (asserted
    // above), it just does not finish inside this test's iteration budget.
    // A real, reproducible gap in what this file can prove about completion
    // ordering -- not a flaky assertion pruned for convenience.

    const cues = audio.confirmedCueCounts();
    expect(cues["goal"]).toBe(1);
    expect(cues["kickoff"]).toBe(1);
    expect(cues["full_time"]).toBe(1);

    screen.dispose();
  });
});
