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
// "clears smoothing at kickoff, full time, and stack teardown" (tier 2) used
// to be one bundled `it.skip` for three sub-cases with three different
// blockers. Split below (in its own nested `describe`, right after "draws
// only from the cached debug model...") so each is its own case with its own
// individually re-verified status, rather than one skip whose reason ages
// out from under two-thirds of what it actually covers the moment only one
// sub-case's blocker clears:
//
//   - kickoff needs a `kickoff_hold`-equivalent presentation timer. Confirmed
//     absent end to end: not on `MatchScreen`, not on `RenderFrameHud`, not
//     produced by `crates/gc-render`'s frame builder -- a genuinely unported
//     piece of `game/screens/match.lua`. Still `it.skip`.
//   - full time WAS blocked: seeding a real correction on a rollback-mode
//     `MatchScreen`, marking the host finished, and calling `update(0)` --
//     the literal translation of the Lua original -- used to leave
//     `active_smoothing_count` unchanged, not 0. Root cause, fixed in
//     `match.ts`: `updateRollback` opened with `if (this.finished) { return;
//     }` -- an unconditional no-op the moment `hud.finished` already read
//     true going in, before any smoothing bookkeeping ran. The Lua original
//     never hits an equivalent guard here: `match_is_over(self)`, for a
//     rollback-mode `Match`, tests `self._source:terminal()` (`lab_source
//     (lab).terminal`, derived from `rollback_playable_lab.debug_model(lab)
//     .status`) -- NOT `self.state.finished`. So a clean-profile lab never
//     reads as `match_is_over`, and execution falls through to the bottom of
//     `Match:update`, where `lifecycle_reset = ... or self.state.finished or
//     ...` is still true and drives `update_render_smoothing`'s
//     `clear_render_smoothing` unconditionally. `updateRollback` now gates
//     its early return on `rollbackTerminal(rollbackHost.debug().status)`
//     instead (mirroring `lab_source.terminal` exactly) and separately reads
//     `hud.finished` at the tail to drive the same smoothing clear the Lua
//     fallthrough does -- see `match.ts`'s own doc on both. Below, no longer
//     `it.skip`.
//   - teardown needs `game.screen_stack`'s TS analog, which lives in
//     `@gc/app` -- `@gc/screens` cannot depend on `@gc/app` (the dependency
//     runs the other way, v2/README.md §2/§9). Still `it.skip`.
//
// The two tier-3 cases (`ScreenStack` driving a real `RealMatch`/`MatchScreen`
// pair) moved to `@gc/app` -- see the comment where they used to sit below.
// They stay skipped there too: `ScreenStack` was necessary but not
// sufficient, since both also need a `sim.rollback_playable_lab` wasm bridge
// this codebase doesn't have. Moving them was still correct -- `@gc/screens`
// structurally cannot host a test that needs `@gc/app` regardless of what
// else blocks it.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import type { RollbackEventDiff } from "@gc/presentation";
import { replay, viewState } from "@gc/render";
import type { replayTypes } from "@gc/render";
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
  SimHostFactory,
  SimHostPort,
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

// --- base-mode fixtures, for "keeps actual goal replay gait coherent..." --
// (below) -- a minimal `SimHostPort` fake distinct from `FakeRollbackHost`
// above: that one drives `RollbackHostPort`, this one drives the BASE
// (non-rollback) `SimHostPort` branch this file's own case needs. Duplicates
// `match_screen.spec.ts`'s own `FakeSimHost`/fixed-clock pattern rather than
// importing it -- two independent spec files, the same "TS-glue-observable
// analog" duplication this file's header already accepts for its own fixed
// clock.

const BASE_TICK_SECONDS = 1 / 60;

class FakeBaseSimHost implements SimHostPort {
  readonly hud = { finished: false, controlled_owns_ball: true, home_score: 0, away_score: 0, time_left: 300 };
  private accumulator = 0;
  private tickCount = 0;

  planTicks(dt: number): number {
    this.accumulator += dt;
    let ticks = 0;
    while (this.accumulator + 1e-9 >= BASE_TICK_SECONDS) {
      this.accumulator -= BASE_TICK_SECONDS;
      ticks += 1;
    }
    return ticks;
  }

  step(): void {
    this.tickCount += 1;
  }

  cancelPlannedTicks(): void {
    this.accumulator = 0;
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

  dispose(): void {}
}

function makeBaseHostFactory(): { readonly factory: SimHostFactory; readonly hosts: FakeBaseSimHost[] } {
  const hosts: FakeBaseSimHost[] = [];
  const factory: SimHostFactory = (): SimHostPort => {
    const host = new FakeBaseSimHost();
    hosts.push(host);
    return host;
  };
  return { factory, hosts };
}

function fixtureBasePlayer(id: string, pos: Vec2): replayTypes.MatchPlayer {
  return {
    id,
    team: "home",
    pos,
    run_vel: new Vec2(0, 0),
    facing: new Vec2(1, 0),
    radius: 10,
    is_keeper: false,
    keeper_state: "base",
    keeper_set: 0,
    slide_timer: 0,
    tackle_timer: 0,
    stun_timer: 0,
    settle_timer: 0,
    sprinting: false,
    outfield_decision: { version: 1, generation: 0, rng_state: 1, remaining: 0, context: "offball", intent: "none" },
    dive_timer: 0,
    dive_dir: new Vec2(0, 0),
    keeper_get_up_timer: 0,
    grab_timer: 0,
    throw_timer: 0,
    windup_timer: 0,
    aerial_timer: 0,
    aerial_jump: 0,
    sprint_meter: 1,
    jockey_timer: 0,
  };
}

/** A `MatchScreenPorts.matchState` fake, for the base-mode goal-replay case below: one drifting player so the replay buffer holds observable motion; see `match_screen.spec.ts`'s identical fixture (`fixtureMatchStateSource`) for why this takes a THUNK, not a `FakeBaseSimHost` directly. */
function fixtureBaseMatchState(getHost: () => FakeBaseSimHost): () => replayTypes.MatchState {
  let ballX = 480;
  return (): replayTypes.MatchState => {
    const host = getHost();
    ballX += 3;
    return {
      field: { w: 960, h: 540 },
      goal_home: { x: -12, y: 210, w: 12, h: 120 },
      goal_away: { x: 960, y: 210, w: 12, h: 120 },
      score: { home: host.hud.home_score, away: host.hud.away_score },
      time_left: host.hud.time_left,
      outfield_press: {
        home: { version: 1, mode: "inactive", reason: "no_trigger" },
        away: { version: 1, mode: "inactive", reason: "no_trigger" },
      },
      transition: { version: 1, hold: 0, elapsed: 0 },
      transition_windows: { home: { counterpress: 0, counterattack: 0 }, away: { counterpress: 0, counterattack: 0 } },
      ball: new Vec2(ballX, 270),
      ball_vel: new Vec2(180, 0),
      ball_z: 0,
      ball_vz: 0,
      players: [fixtureBasePlayer("home_1", new Vec2(ballX - 20, 270))],
      events: [],
    };
  };
}

/** Mirrors the ported spec's `start_actual_goal_replay` helper: build up footage, force a home goal edge, and confirm the replay started but has not displayed a frame yet. */
function startActualGoalReplay(host: FakeBaseSimHost, screen: MatchScreen): void {
  for (let i = 0; i < 40; i += 1) {
    screen.update(BASE_TICK_SECONDS);
  }
  host.hud.home_score += 1;
  screen.update(BASE_TICK_SECONDS);
  expect(host.hud.home_score).toBe(1);
  expect(replay.active(), "the replay rolls after a goal").toBe(true);
  expect(screen.debugReplayState).toBeUndefined();
}

/** Mirrors the ported spec's `seed_render_correction` helper: seed a bounded, decaying correction offset so the smoothing-diagnostics assertions below exercise something real. `matchState` is the SAME source function the screen itself was constructed with -- see `MatchScreen.debugSeedRenderCorrection`'s doc for why a "previous" pose is built from it rather than reaching into screen-private state. */
function seedBaseRenderCorrection(screen: MatchScreen, matchState: () => replayTypes.MatchState): void {
  const current = matchState();
  const player = current.players[0]!;
  screen.debugSeedRenderCorrection({
    players: [{ id: player.id, pos: new Vec2(player.pos.x - 40, player.pos.y) }],
    ball: new Vec2(current.ball.x - 20, current.ball.y),
  });
  expect(screen.debugRenderSmoothingDiagnostics?.active_count ?? 0).toBeGreaterThan(0);
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

  // Split from the single bundled "clears smoothing at kickoff, full time,
  // and stack teardown" case -- three sub-cases, three different blockers.
  // See this file's header for the two that are now cleared elsewhere in
  // this file ("@gc/render" dependency, `rollback_lab` construction option)
  // and stay cleared here; what follows is each sub-case's OWN, individually
  // re-verified status.
  describe("clears smoothing at kickoff, full time, and stack teardown (split)", () => {
    // Needs a `kickoff_hold`-equivalent presentation timer. The Lua original
    // reads `kickoff.state.kickoff_hold > 0` straight off `sim.match`'s own
    // snapshot field (`crates/gc-sim/src/match_snapshot.rs`'s `kickoff_hold`,
    // set to `KICKOFF_HOLD` in `match.rs` on a kickoff/restart and ticked
    // down in `sim_match::step`). Confirmed absent end to end on this side of
    // the port: no `kickoff_hold` anywhere under `v2/ts/` (grepped the whole
    // tree), not on `MatchScreen`'s own state, not on `RenderFrameHud`, and
    // not produced by `crates/gc-render`'s frame builder either -- the value
    // exists only inside `gc-sim`'s snapshot and is never surfaced across the
    // wasm boundary. A genuinely unported piece of `game/screens/match.lua`,
    // not a dependency-graph or fixture gap this package can work around.
    it.skip(
      "kickoff [needs a kickoff_hold-equivalent presentation timer, absent from MatchScreen/RenderFrameHud/crates/gc-render's frame -- not this package's call to add]",
      () => {},
    );

    // Literal translation of the Lua original's `full_time.state.finished =
    // true; full_time:update(0)`: seed a real, nonzero smoothing correction
    // on a rollback-mode `MatchScreen` (`screen.debugSeedRenderCorrection`),
    // confirm `active_smoothing_count > 0`, then set `host.hud.finished =
    // true` and call `screen.update(0)`.
    //
    // This used to fail: `updateRollback` opened with `if (this.finished) {
    // return; }`, an unconditional no-op the moment `hud.finished` already
    // read true going in, before any smoothing bookkeeping ran. Fixed in
    // `match.ts` -- see this file's header for the root-cause trace and how
    // the fix maps onto the Lua original's two separate checks
    // (`rollbackTerminal`'s early-return gate vs. `updateRollback`'s own
    // `hudFinished` tail branch).
    it("clears smoothing at full time", () => {
      const { factory, hosts } = makeRollbackHostFactory({ localSlot: 1, profileName: "clean" });
      const screen = new MatchScreen(rollbackPorts(factory), {
        rollback_lab: labOptions({ local_slot: 1, profile_name: "clean" }),
      });
      const host = hosts[0]!;
      const current = host.displayedPositions();
      const player = current.players[0]!;
      screen.debugSeedRenderCorrection({
        players: [{ id: player.id, pos: { x: player.pos.x - 40, y: player.pos.y } }],
        ball: { x: current.ball.x - 20, y: current.ball.y },
      });
      expect(screen.debugRenderSmoothingDiagnostics?.active_count ?? 0).toBeGreaterThan(0);

      host.hud.finished = true;
      screen.update(0);

      expect(screen.debugRollbackDebug()?.active_smoothing_count).toBe(0);
      expect(screen.debugRollbackDebug()?.correction_magnitude).toBeCloseTo(0);
    });

    // Needs `@gc/app`'s `screen_stack.ts` -- `@gc/screens` cannot depend on
    // `@gc/app` (v2/README.md §2/§9; confirmed `packages/screens/package.json`
    // declares no such dependency, and the direction is structural, not an
    // oversight). Unrelated to either blocker above.
    it.skip(
      "stack teardown [needs @gc/app's screen_stack.ts, a reverse dependency @gc/screens structurally cannot take]",
      () => {},
    );
  });

  it("keeps actual goal replay gait coherent and clears smoothing on both exits", () => {
    replay.reset();
    replay.resetTuning();
    viewState.reset();

    const { factory: naturalFactory, hosts: naturalHosts } = makeBaseHostFactory();
    const naturalMatchState = fixtureBaseMatchState(() => naturalHosts[0]!);
    const natural = new MatchScreen({
      createHost: naturalFactory,
      renderer: noopRenderer,
      keyboard: { isDown: () => false },
      replay,
      matchState: naturalMatchState,
    });
    startActualGoalReplay(naturalHosts[0]!, natural);

    natural.update(1 / 60);
    const replayState = natural.debugReplayState;
    expect(replayState).toBeDefined();
    for (const player of replayState!.players) {
      const view = viewState.get(player.id);
      expect(view).toBeDefined();
      expect(view!.speed).toBeCloseTo(0);
      expect(view!.phase).toBeCloseTo(0);
      expect(view!.lean).toBeCloseTo(0);
    }
    for (let i = 0; i < 12; i += 1) {
      natural.update(1 / 60);
    }
    let sawGait = false;
    let sawLean = false;
    for (const player of natural.debugReplayState!.players) {
      const view = viewState.get(player.id)!;
      sawGait = sawGait || (view.speed > 0 && view.phase > 0);
      sawLean = sawLean || Math.abs(view.lean) > 0;
    }
    expect(sawGait, "active replay frames must preserve gait progression").toBe(true);
    expect(sawLean, "active replay frames must preserve lean progression").toBe(true);
    seedBaseRenderCorrection(natural, naturalMatchState);

    natural.update(2);
    natural.update(100);
    expect(replay.active()).toBe(false);
    expect(natural.debugReplayState).toBeUndefined();
    const naturalLive = viewState.get("home_1")!;
    expect(naturalLive.speed).toBeCloseTo(0);
    expect(naturalLive.phase).toBeCloseTo(0);
    expect(naturalLive.lean).toBeCloseTo(0);
    expect(natural.debugRenderSmoothingDiagnostics?.active_count ?? 0).toBe(0);

    replay.reset();
    viewState.reset();
    const { factory: skippedFactory, hosts: skippedHosts } = makeBaseHostFactory();
    const skippedMatchState = fixtureBaseMatchState(() => skippedHosts[0]!);
    const skipped = new MatchScreen({
      createHost: skippedFactory,
      renderer: noopRenderer,
      keyboard: { isDown: () => false },
      replay,
      matchState: skippedMatchState,
    });
    startActualGoalReplay(skippedHosts[0]!, skipped);
    skipped.update(1 / 60);
    seedBaseRenderCorrection(skipped, skippedMatchState);
    skipped.event({ kind: "key", key: "space" });
    expect(replay.active()).toBe(false);
    expect(skipped.debugReplayState).toBeUndefined();
    const skippedLive = viewState.get("home_1")!;
    expect(skippedLive.speed).toBeCloseTo(0);
    expect(skippedLive.phase).toBeCloseTo(0);
    expect(skippedLive.lean).toBeCloseTo(0);
    expect(skipped.debugRenderSmoothingDiagnostics?.active_count ?? 0).toBe(0);
  });
});

// "playable rollback ScreenStack flow (tier 3)" (both cases) moved to
// packages/app/src/match_rollback_lab.spec.ts: they need `ScreenStack`
// (`@gc/app`'s screen_stack.ts) driving a real `MatchScreen`/`RealMatch`
// pair, and `@gc/screens` cannot depend on `@gc/app` (the dependency runs
// the other way). They stay `it.skip` in their new home too -- moving them
// was necessary but not sufficient; see that file's header for the
// still-missing `sim.rollback_playable_lab` wasm bridge underneath. Do not
// re-add them here.
