// Ported from spec/screens/match_rollback_lab_spec.lua.
//
// STATUS (re-checked for this task; see git history for the prior header
// this replaces): the two blockers that used to block every case here are
// cleared -- `@gc/render` is now a declared dependency of `@gc/screens`, and
// `MatchScreen` (match.ts) now has a `rollback_lab` construction option plus
// a `RollbackHostPort` rollback surface (this file's header used to name
// both as missing; see match.ts's "THE ROLLBACK LABORATORY" section).
//
// What is still genuinely missing, and is NOT solved by this file: a real
// `@gc/wasm` binding for `sim.rollback_playable_lab`
// (`crates/gc-sim/src/rollback_playable_lab.rs`). `@gc/wasm`'s closest
// rollback surface, `MatchDriverBridge`, binds `gc_netcode::match_driver` --
// an OMP-3 TWO-PEER ONLINE match driver (`Coordinator` freeze/manifest,
// `@gc/transport`-shaped envelope queues), not this single-process local dev
// harness. Building a real implementation would mean either a new Rust/wasm
// export (outside this file's ownership -- no Rust crate may be touched) or
// reimplementing a full loopback two-peer online session on top of
// `MatchDriverBridge`/`Coordinator` (a `packages/online`/`packages/
// transport`-sized undertaking, also outside this file's ownership).
//
// So `RollbackHostPort` (match.ts) is this package's OWN design, and every
// case below drives it with a hand-written `FakeRollbackHost` -- the exact
// same "TS-glue-observable analog" pattern `match_screen.spec.ts`'s header
// already establishes for `SimHostPort`/`FakeSimHost`. The fake's clock
// mirrors `gc_sim::fixed_clock::advance` (README v2/README.md §2.1 -- see
// `match_screen.spec.ts`'s identical justification for its own fake clock),
// and its "network profile"/"confirmation"/"correction" bookkeeping is
// deliberately simple: enough to prove `MatchScreen`'s OWN aggregation/
// wiring logic (does it record outputs into `debugRollbackOutputs`? clear
// them on pause? feed `viewState`/`correctionSmoothing`? rebuild cleanly on
// restart?) -- proving `gc_sim::rollback_events`/`rollback_playable_lab`'s
// OWN correctness is `crates/gc-sim/tests`' job, already covered by the
// Rust port those Lua specs became.
//
// Two tier-2 cases remain out of scope in this package, for a REAL,
// still-standing reason each:
//
//   - "keeps actual goal replay gait coherent..." (tier 2) needs the BASE
//     (non-rollback) goal-replay feature -- `SimHostPort`'s `RenderFrame` is
//     a presentation-derived, structure-of-arrays wire shape
//     (`crates/gc-render/src/frame.rs`), not the raw `MatchState` (outfield
//     press/transition/decision state, per-player timers) `@gc/render`'s
//     real `replay.ts` needs to `recordBoundary`. See `match_screen.spec.ts`'s
//     own "goal replay" skip, which names the identical blocker.
//   - "clears smoothing at kickoff, full time, and stack teardown" (tier 2)
//     bundles three sub-cases; two of its three depend on things outside
//     this package structurally: the "kickoff" sub-case needs the same
//     goal-replay capability as above, and the "teardown" sub-case needs
//     `game.screen_stack`'s TS analog, which lives in `@gc/app` --
//     `@gc/screens` cannot depend on `@gc/app` (the dependency runs the
//     other way, v2/README.md §2/§9). Only the middle ("full time") sub-case
//     is independently portable, and a single `it` cannot be two-thirds
//     skipped, so the whole case stays `it.skip`.
//
// The two tier-3 cases (`ScreenStack` driving a real `RealMatch`/`MatchScreen`
// pair) moved to `@gc/app` -- see the comment where they used to sit below.
// They stay skipped there too: `ScreenStack` was necessary but not
// sufficient, since both also need a `sim.rollback_playable_lab` wasm bridge
// this codebase doesn't have. Moving them was still correct -- `@gc/screens`
// structurally cannot host a test that needs `@gc/app` regardless of what
// else blocks it.

import { describe, expect, it } from "vitest";
import type { RollbackEventDiff } from "@gc/presentation";
import { MatchScreen } from "./match.ts";
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
  RollbackLabOptions,
  RollbackLabOutput,
  RollbackLabSnapshotSummary,
  RollbackLabStatus,
  RollbackLabStepResult,
} from "./match.ts";

// --- the fake clock: mirrors gc_sim::fixed_clock::advance -----------------
// (see this file's header, and match_screen.spec.ts's identical fake clock).

const TICK_SECONDS = 1 / 60;
const MAX_TICKS_PER_UPDATE = 8;
const CLOCK_EPSILON = TICK_SECONDS * 1e-9;

class FakeFixedClock {
  tick = 0;
  accumulator = 0;
  dropped_ticks = 0;
  overloads = 0;

  advance(dt: number): number {
    this.accumulator += dt;
    let ticks = 0;
    while (this.accumulator + CLOCK_EPSILON >= TICK_SECONDS && ticks < MAX_TICKS_PER_UPDATE) {
      this.accumulator -= TICK_SECONDS;
      if (this.accumulator < 0) {
        this.accumulator = 0;
      }
      ticks += 1;
      this.tick += 1;
    }
    if (this.accumulator + CLOCK_EPSILON >= TICK_SECONDS) {
      const dropped = Math.floor((this.accumulator + CLOCK_EPSILON) / TICK_SECONDS);
      this.accumulator -= dropped * TICK_SECONDS;
      if (this.accumulator < 0) {
        this.accumulator = 0;
      }
      this.dropped_ticks += dropped;
      this.overloads += 1;
    }
    return ticks;
  }

  stopEarly(): void {
    this.accumulator = 0;
  }
}

// --- FakeRollbackHost -------------------------------------------------------

interface FakeRollbackHostConfig {
  readonly localSlot: number;
  readonly profileName: string;
  /** Simulated network delay, in ticks; 0 (the default, "clean") confirms every tick immediately. */
  readonly delayTicks?: number;
  readonly maxUnconfirmedTicks?: number;
  /** When set, `currentSnapshot()`/`referenceSnapshot()` report a combat companion with these player ids. */
  readonly combatPlayerIds?: readonly string[];
}

const ROSTER_IDS: readonly string[] = [
  "home_1",
  "home_2",
  "home_3",
  "home_4",
  "home_5",
  "away_1",
  "away_2",
  "away_3",
  "away_4",
  "away_5",
];

class FakeRollbackHost implements RollbackHostPort {
  readonly clock = new FakeFixedClock();
  readonly stepCalls: InputSample[] = [];
  disposeCalls = 0;
  readonly hud = { finished: false, controlled_owns_ball: true, home_score: 0, away_score: 0, time_left: 300 };
  private readonly config: FakeRollbackHostConfig;
  private tickCount = 0;
  private status: RollbackLabStatus = "active";
  private readonly pending: Array<{ readonly tick: number }> = [];
  private readonly playerPos = new Map<string, { x: number; y: number }>(
    ROSTER_IDS.map((id, index) => [id, { x: index * 10, y: 0 }]),
  );
  private ball = { x: 0, y: 0 };

  constructor(config: FakeRollbackHostConfig) {
    this.config = config;
  }

  planTicks(dt: number): number {
    return this.clock.advance(dt);
  }

  cancelPlannedTicks(): void {
    this.clock.stopEarly();
  }

  clockDebug(): RollbackLabClockDebug {
    return {
      tick: this.clock.tick,
      dropped_ticks: this.clock.dropped_ticks,
      overloads: this.clock.overloads,
      accumulator: this.clock.accumulator,
    };
  }

  step(sample: InputSample): RollbackLabStepResult {
    this.stepCalls.push(sample);
    // 0-based, matching `gc_sim::fixed_clock::FixedClockState.tick`'s own
    // convention ("next tick to simulate; starts at zero") -- the tick just
    // simulated is the value BEFORE incrementing.
    const tickIndex = this.tickCount;
    this.tickCount += 1;

    const controlledId = ROSTER_IDS[this.config.localSlot - 1] ?? ROSTER_IDS[0]!;
    const previous = this.playerPos.get(controlledId)!;
    this.playerPos.set(controlledId, { x: previous.x + sample.move_x / 20, y: previous.y + sample.move_y / 20 });
    this.ball = { x: this.ball.x + sample.move_x / 40, y: this.ball.y + sample.move_y / 40 };

    const output: RollbackLabOutput = { tick: tickIndex, input: { slots: [{ sample }] } };
    // Unconditional, one per tick -- see RollbackLabStepResult's doc.
    const eventDiffs: RollbackEventDiff[] = [{ added: [], revoked: [], replaced: [] }];
    let confirmedSteps: readonly RollbackEventStep[] = [];
    let correction: RollbackLabCorrectionSample | undefined;

    const delayTicks = this.config.delayTicks ?? 0;
    const maxUnconfirmedTicks = this.config.maxUnconfirmedTicks ?? MAX_TICKS_PER_UPDATE;
    this.pending.push({ tick: tickIndex });
    if (this.pending.length > maxUnconfirmedTicks) {
      // The unconfirmed window (ticks simulated but not yet confirmed) grew
      // past what this profile tolerates -- a synchronization failure, not
      // merely "still waiting". Checked BEFORE the delay-gated confirmation
      // below: a confirmation and a terminal failure are mutually exclusive
      // outcomes for the same tick.
      this.status = "unconfirmed_window_exceeded";
    } else if (this.pending.length > delayTicks) {
      const confirmedEntry = this.pending.shift()!;
      confirmedSteps = [
        {
          tick: confirmedEntry.tick,
          start_boundary: confirmedEntry.tick,
          end_boundary: confirmedEntry.tick + 1,
          state: {
            score: { home: this.hud.home_score, away: this.hud.away_score },
            time_left: this.hud.time_left,
            finished: this.hud.finished,
          },
          match_events: [],
          lifecycle_events: [],
        },
      ];
      correction = {
        tick: confirmedEntry.tick,
        source: { players: [...this.playerPos.entries()].map(([id, pos]) => ({ id, pos })), ball: this.ball },
      };
      this.status = "active";
    }

    return {
      output,
      eventDiffs,
      confirmedSteps,
      ...(correction !== undefined ? { correction } : {}),
      debug: this.debug(),
    };
  }

  debug(): RollbackLabHostDebug {
    return {
      profile: this.config.profileName,
      local_slot: this.config.localSlot,
      transport_tick: this.tickCount,
      reference_tick: this.tickCount,
      current_tick: this.tickCount,
      rollback_count: 0,
      network_pending: this.pending.length,
      status: this.status,
      event_status: "active",
    };
  }

  frame(): RenderFrame {
    return { hud: this.hud, possession: {} };
  }

  roster(): RenderFrameRoster {
    return {};
  }

  tick(): number {
    return this.tickCount;
  }

  displayedPositions(): RollbackLabDisplayedPositions {
    return { players: [...this.playerPos.entries()].map(([id, pos]) => ({ id, pos })), ball: this.ball };
  }

  currentSnapshot(): RollbackLabSnapshotSummary {
    return this.config.combatPlayerIds !== undefined ? { combat: { player_ids: this.config.combatPlayerIds } } : {};
  }

  referenceSnapshot(): RollbackLabSnapshotSummary {
    return this.currentSnapshot();
  }

  dispose(): void {
    this.disposeCalls += 1;
  }
}

function makeRollbackHostFactory(
  config: FakeRollbackHostConfig,
): { readonly factory: RollbackHostFactory; readonly hosts: FakeRollbackHost[] } {
  const hosts: FakeRollbackHost[] = [];
  const factory: RollbackHostFactory = (): RollbackHostPort => {
    const host = new FakeRollbackHost(config);
    hosts.push(host);
    return host;
  };
  return { factory, hosts };
}

// --- trivial EffectsPort/AudioPort/ReplayPort fakes -------------------------
// (this file's fake host never produces a match/combat/lifecycle event, so
// these never need to do more than satisfy the shape -- see FakeRollbackHost
// above; the rollback-consumption seam itself is exercised for real by
// combat_feedback_rollback.spec.ts).

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

class FakeReplay implements ReplayPort {
  private activeFlag = false;
  active(): boolean {
    return this.activeFlag;
  }
  startAt(): boolean {
    return false;
  }
  reset(): void {
    this.activeFlag = false;
  }
}

const noopRenderer: RenderPort = { draw: (): void => {} };

function rollbackPorts(factory: RollbackHostFactory, tuningPause?: { open: boolean }): MatchScreenPorts {
  return {
    createHost: () => {
      throw new Error("base createHost should not be called for a rollback-lab screen");
    },
    renderer: noopRenderer,
    keyboard: { isDown: () => false },
    createRollbackHost: factory,
    effects: noopEffects(),
    audio: noopAudio(),
    replay: new FakeReplay(),
    ...(tuningPause !== undefined ? { tuningPause } : {}),
  };
}

function labOptions(overrides: Partial<RollbackLabOptions> & Pick<RollbackLabOptions, "local_slot" | "profile_name">): RollbackLabOptions {
  return overrides;
}

describe("match screen rollback laboratory (tier 2)", () => {
  it("constructs the combat companion for an explicit rollback playtest", () => {
    const { factory } = makeRollbackHostFactory({
      localSlot: 2,
      profileName: "clean",
      combatPlayerIds: ROSTER_IDS,
    });
    const screen = new MatchScreen(rollbackPorts(factory), {
      combat_enabled: true,
      rollback_lab: labOptions({ local_slot: 2, profile_name: "clean" }),
    });

    expect(screen.debugRollbackActive).toBe(true);
    const snapshot = screen.debugRollbackCurrentSnapshot();
    expect(snapshot?.combat).toBeDefined();
    // slot 2 -> index 1 -- the combat companion's player_ids align with the
    // roster by slot, matching the Lua original's `snapshot.combat.player_ids[2]
    // == screen.state.players[2].id`.
    expect(snapshot?.combat?.player_ids[1]).toBe(ROSTER_IDS[1]);
  });

  it("requires rollback snapshot combat presence to match the explicit opt-in", () => {
    const { factory: combatFactory } = makeRollbackHostFactory({
      localSlot: 1,
      profileName: "clean",
      combatPlayerIds: ROSTER_IDS,
    });
    expect(
      () =>
        new MatchScreen(rollbackPorts(combatFactory), {
          rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
        }),
    ).toThrow("combat-bearing rollback snapshots require combat_enabled = true");

    const { factory: combatFactory2 } = makeRollbackHostFactory({
      localSlot: 1,
      profileName: "clean",
      combatPlayerIds: ROSTER_IDS,
    });
    expect(
      () =>
        new MatchScreen(rollbackPorts(combatFactory2), {
          combat_enabled: true,
          rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
        }),
    ).not.toThrow();

    const { factory: soccerFactory } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    expect(
      () =>
        new MatchScreen(rollbackPorts(soccerFactory), {
          combat_enabled: true,
          rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
        }),
    ).toThrow("combat-enabled matches require a CombatMatchState companion");
  });

  it("is an explicit development-only slot-mode option", () => {
    const { factory: productFactory } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    expect(
      () =>
        new MatchScreen(rollbackPorts(productFactory), {
          profile: "product",
          rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
        }),
    ).toThrow();

    const { factory } = makeRollbackHostFactory({ localSlot: 6, profileName: "clean" });
    const screen = new MatchScreen(rollbackPorts(factory), {
      rollback_lab: labOptions({ local_slot: 6, profile_name: "clean" }),
    });
    expect(screen.debugRollbackActive).toBe(true);
    expect(screen.debugRollbackDebug()?.local_slot).toBe(6);
    expect(screen.debugRollbackDebug()?.status).toBe("active");
  });

  it("retains a zero-tick edge and consumes it exactly once", () => {
    const { factory, hosts } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    const screen = new MatchScreen(rollbackPorts(factory), {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
    });
    hosts[0]!.hud.controlled_owns_ball = false;
    screen.event({ kind: "key", key: "k" });

    screen.update(TICK_SECONDS / 2);
    expect(screen.debugRollbackOutputs.length).toBe(0);
    expect(screen.debugSwitchPending).toBe(true);

    screen.update(TICK_SECONDS / 2);
    expect(screen.debugRollbackOutputs.length).toBe(1);
    const firstSample = screen.debugRollbackOutputs[0]!.input.slots[0]!.sample;
    expect((firstSample.edges & 4) !== 0).toBe(true); // "switch" edge bit
    expect(screen.debugSwitchPending).toBe(false);

    screen.update(TICK_SECONDS);
    expect(screen.debugRollbackOutputs.length).toBe(1);
    const secondSample = screen.debugRollbackOutputs[0]!.input.slots[0]!.sample;
    expect((secondSample.edges & 4) !== 0).toBe(false);
  });

  // `@gc/input`'s `capture_frame.ts` captures equipment by POLLING
  // `bindings.isDown("equipment", ...)` and diffing against the previous
  // `sample()` call (its own header: "direct/poll-diffed, no `carrying`
  // check") -- unlike `switch`/`dash`, it has no discrete press/release
  // EVENT capture, so a tap entirely between two render calls (the Lua
  // original's exact scenario, driven through `Match:event`) cannot be
  // reproduced through this port's real capture layer. The TS-glue-
  // observable analog: hold the bound key ("u") across one render call
  // (produces the held + pressed edge), release it across the next
  // (produces the released edge, no longer held) -- same edge/held bits,
  // spread across the two render calls this capture layer actually needs.
  it("captures a complete equipment tap before the next render update", () => {
    const down: Record<string, boolean> = {};
    const { factory } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    const keyboard = { isDown: (...keys: readonly string[]): boolean => keys.some((k) => down[k] === true) };
    const screen = new MatchScreen({ ...rollbackPorts(factory), keyboard }, {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
    });

    down.u = true;
    screen.update(TICK_SECONDS);
    const pressedSample = screen.debugRollbackOutputs[0]!.input.slots[0]!.sample;
    expect((pressedSample.held & 128) !== 0, "equipment held bit").toBe(true);
    expect((pressedSample.edges & 32) !== 0, "equipment_pressed edge bit").toBe(true);

    down.u = false;
    screen.update(TICK_SECONDS);
    const releasedSample = screen.debugRollbackOutputs[0]!.input.slots[0]!.sample;
    expect((releasedSample.held & 128) !== 0, "equipment held bit clears on release").toBe(false);
    expect((releasedSample.edges & 64) !== 0, "equipment_released edge bit").toBe(true);
  });

  it("uses one fixed clock and aggregates multi-tick edges, holds, and corrections", () => {
    const { factory, hosts } = makeRollbackHostFactory({
      localSlot: 1,
      profileName: "two_tick",
      delayTicks: 2,
    });
    const keyboard = { isDown: (...keys: readonly string[]): boolean => keys.includes("lshift") };
    const screen = new MatchScreen({ ...rollbackPorts(factory), keyboard }, {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "two_tick" }),
    });
    hosts[0]!.hud.controlled_owns_ball = false;
    screen.event({ kind: "key", key: "k" });

    screen.update(3 * TICK_SECONDS);

    expect(screen.debugRollbackClock()?.tick).toBe(3);
    expect(screen.debugRollbackDebug()?.transport_tick).toBe(3);
    expect(screen.debugRollbackDebug()?.reference_tick).toBe(3);
    expect(screen.debugRollbackOutputs.length).toBe(3);
    screen.debugRollbackOutputs.forEach((output, index) => {
      const sample = output.input.slots[0]!.sample;
      expect((sample.held & 4) !== 0).toBe(true); // "sprint" held bit, every tick
      expect((sample.edges & 4) !== 0).toBe(index === 0); // "switch" edge only on the first tick of the batch
    });
    expect(
      screen.debugRollbackCorrections.length,
      "the render update must retain corrections from every simulated tick",
    ).toBeGreaterThan(0);
    expect(screen.debugRollbackEventDiffs.length).toBeGreaterThanOrEqual(screen.debugRollbackOutputs.length);
    expect(screen.debugRollbackFrameEvents.length, "legacy speculative consumers receive no events").toBe(0);
  });

  it("updates live player view state from the displayed rollback client", async () => {
    const { viewState } = await import("@gc/render");
    const { factory } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    const keyboard = {
      isDown: (...keys: readonly string[]): boolean => keys.includes("right") || keys.includes("lshift"),
    };
    const screen = new MatchScreen({ ...rollbackPorts(factory), keyboard }, {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
    });
    screen.update(TICK_SECONDS);
    screen.update(TICK_SECONDS);

    const view = viewState.get(ROSTER_IDS[0]!);
    expect(view, "a moving lab player must produce live gait speed").toBeDefined();
    expect(view!.speed > 0 || view!.phase > 0, "a moving lab player must advance gait speed/phase").toBe(true);
    viewState.reset();
  });

  it("clears rollback handoff batches before paused and terminal early returns", () => {
    const { factory: pausedFactory } = makeRollbackHostFactory({
      localSlot: 1,
      profileName: "pause",
      delayTicks: 2,
    });
    const tuningPause = { open: false };
    const paused = new MatchScreen(rollbackPorts(pausedFactory, tuningPause), {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "pause" }),
    });
    paused.update(3 * TICK_SECONDS);
    expect(paused.debugRollbackOutputs.length).toBeGreaterThan(0);
    tuningPause.open = true;
    paused.update(0);
    tuningPause.open = false;
    expect(paused.debugRollbackOutputs.length).toBe(0);
    expect(paused.debugRollbackEventDiffs.length).toBe(0);
    expect(paused.debugRollbackConfirmedSteps.length).toBe(0);
    expect(paused.debugRollbackCorrections.length).toBe(0);

    const { factory: terminalFactory } = makeRollbackHostFactory({
      localSlot: 1,
      profileName: "terminal",
      delayTicks: 2,
      maxUnconfirmedTicks: 1,
    });
    const terminal = new MatchScreen(rollbackPorts(terminalFactory), {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "terminal" }),
    });
    terminal.update(2 * TICK_SECONDS);
    expect(terminal.debugRollbackDebug()?.status).toBe("unconfirmed_window_exceeded");
    expect(terminal.debugRollbackOutputs.length).toBeGreaterThan(0);
    expect(terminal.debugRollbackDebug()?.active_smoothing_count).toBe(0);
    expect(terminal.debugRollbackDebug()?.correction_magnitude).toBeCloseTo(0);
    // The Lua original also asserts `terminal:broadcast_phase() == nil` here
    // -- an online-coordinator concept this port never built (out of scope,
    // `@gc/online`'s territory). The TS-observable analog: a sync failure
    // must not read as a finished/full-time match either.
    expect(terminal.finished, "synchronization failure must not masquerade as full time").toBe(false);

    terminal.update(0);
    expect(terminal.debugRollbackOutputs.length).toBe(0);
    expect(terminal.debugRollbackEventDiffs.length).toBe(0);
    expect(terminal.debugRollbackConfirmedSteps.length).toBe(0);
    expect(terminal.debugRollbackCorrections.length).toBe(0);
  });

  it("preserves fixed-clock overload dropping and contiguous transport ticks", () => {
    const { factory } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    const screen = new MatchScreen(rollbackPorts(factory), {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
    });
    screen.update((MAX_TICKS_PER_UPDATE + 3.5) * TICK_SECONDS);
    expect(screen.debugRollbackOutputs.length).toBe(MAX_TICKS_PER_UPDATE);
    expect(screen.debugRollbackClock()?.tick).toBe(MAX_TICKS_PER_UPDATE);
    expect(screen.debugRollbackClock()?.dropped_ticks).toBe(3);
    expect(screen.debugRollbackClock()?.overloads).toBe(1);
    expect(screen.debugRollbackDebug()?.transport_tick).toBe(MAX_TICKS_PER_UPDATE);
    expect(screen.debugRollbackDebug()?.reference_tick).toBe(MAX_TICKS_PER_UPDATE);
    expect(screen.debugRollbackClock()?.accumulator).toBeCloseTo(TICK_SECONDS / 2, 9);

    screen.update(TICK_SECONDS / 2);
    expect(screen.debugRollbackOutputs.length).toBe(1);
    expect(screen.debugRollbackOutputs[0]!.tick).toBe(MAX_TICKS_PER_UPDATE);
    expect(screen.debugRollbackClock()?.tick).toBe(MAX_TICKS_PER_UPDATE + 1);
    expect(screen.debugRollbackDebug()?.transport_tick).toBe(MAX_TICKS_PER_UPDATE + 1);
    expect(screen.debugRollbackDebug()?.reference_tick).toBe(MAX_TICKS_PER_UPDATE + 1);
  });

  it("live R replaces all rollback and presentation-owned state", () => {
    const { factory, hosts } = makeRollbackHostFactory({
      localSlot: 3,
      profileName: "restart_profile",
      delayTicks: 2,
    });
    const screen = new MatchScreen(rollbackPorts(factory), {
      profile: "playtest",
      rollback_lab: labOptions({ local_slot: 3, profile_name: "restart_profile" }),
    });
    const oldHost = hosts[0]!;
    screen.update(3 * TICK_SECONDS);
    expect(screen.debugRollbackCorrections.length).toBeGreaterThan(0);

    const consumerBefore = screen.debugRollbackConsumerState()!;
    consumerBefore.last_scoring_team = "home";
    consumerBefore.kickoff_banner = 0;

    // `Match:event`'s rematch handling only routes to `restart()` once the
    // match reads as finished (`match_is_over(self)`) -- true in both the
    // base and rollback branches; the ported spec doesn't set this
    // explicitly but relies on the same gate.
    oldHost.hud.finished = true;
    screen.event({ kind: "key", key: "r" });

    expect(hosts.length, "the same factory rebuilt a fresh host").toBe(2);
    expect(oldHost.disposeCalls).toBe(1);
    const debug = screen.debugRollbackDebug()!;
    expect(debug.profile).toBe("restart_profile");
    expect(debug.local_slot).toBe(3);
    expect(debug.transport_tick).toBe(0);
    expect(debug.reference_tick).toBe(0);
    expect(debug.current_tick).toBe(0);
    expect(debug.rollback_count).toBe(0);
    expect(debug.network_pending).toBe(0);
    expect(debug.event_status).toBe("active");
    expect(debug.active_smoothing_count).toBe(0);
    expect(debug.correction_magnitude).toBeCloseTo(0);
    expect(screen.debugRollbackClock()?.tick).toBe(0);
    expect(screen.debugRollbackClock()?.accumulator).toBe(0);
    expect(screen.debugRollbackFrameEvents.length).toBe(0);
    expect(screen.debugRollbackOutputs.length).toBe(0);
    expect(screen.debugRollbackEventDiffs.length).toBe(0);
    expect(screen.debugRollbackConfirmedSteps.length).toBe(0);
    expect(screen.debugRollbackCorrections.length).toBe(0);
    const consumerAfter = screen.debugRollbackConsumerState()!;
    expect(consumerAfter.last_scoring_team).toBeUndefined();
    expect(consumerAfter.kickoff_banner).toBeGreaterThan(0);
  });

  it("draws only from the cached debug model without mutating either match", () => {
    const { factory } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
    const screen = new MatchScreen(rollbackPorts(factory), {
      rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
    });
    const clientBefore = screen.debugRollbackCurrentSnapshot();
    const referenceBefore = screen.debugRollbackReferenceSnapshot();
    const debugBefore = screen.debugRollbackDebug()!.transport_tick;

    screen.draw();

    expect(screen.debugRollbackCurrentSnapshot()).toEqual(clientBefore);
    expect(screen.debugRollbackReferenceSnapshot()).toEqual(referenceBefore);
    expect(screen.debugRollbackDebug()!.transport_tick).toBe(debugBefore);
  });

  // Both blocked -- see this file's header.
  it.skip(
    "clears smoothing at kickoff, full time, and stack teardown [kickoff sub-case needs base goal replay's raw-MatchState capability; teardown sub-case needs @gc/app's screen_stack.ts, a reverse dependency]",
    () => {},
  );
  it.skip(
    "keeps actual goal replay gait coherent and clears smoothing on both exits [needs the base (non-rollback) goal-replay feature; see match_screen.spec.ts's goal-replay skip]",
    () => {},
  );
});

// "playable rollback ScreenStack flow (tier 3)" (both cases) moved to
// packages/app/src/match_rollback_lab.spec.ts: they need `ScreenStack`
// (`@gc/app`'s screen_stack.ts) driving a real `MatchScreen`/`RealMatch`
// pair, and `@gc/screens` cannot depend on `@gc/app` (the dependency runs
// the other way). They stay `it.skip` in their new home too -- moving them
// was necessary but not sufficient; see that file's header for the
// still-missing `sim.rollback_playable_lab` wasm bridge underneath. Do not
// re-add them here.
