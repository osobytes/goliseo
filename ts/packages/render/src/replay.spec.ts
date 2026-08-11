// Fixture note (mirrors @gc/presentation's combat.spec.ts): `replay`'s
// module functions are a pure transformation of whatever
// `MatchState`/`CombatMatchState`-shaped frames they are handed, so the
// fixture below is a self-consistent synthetic stand-in (ten static
// players, a ball drifting at a constant velocity) that exercises the exact
// same replay mechanics -- capture, boundary replace/truncate, celebration
// targeting, slow-motion playback, and the combat-state passenger -- without
// needing a running simulation.
//
// Pose selection (`PlayerPoseId`, its priority table) lives in Rust at
// `crates/gc-render/src/player_pose.rs`, but a goal replay buffers
// pre-render `MatchState`/`MatchPlayer` snapshots (see this module's own
// header), not decoded RenderFrame wire data, so a buffered, replayed frame
// is never run through Rust's pose selection as part of the ordinary
// per-frame path.
//
// The "carries every pose input through capture, celebration, and
// playback" test below exercises the real selector directly instead:
// `@gc/wasm`'s `playerPoseSelect` -- `crates/gc-wasm/src/player_pose_bridge.rs`'s
// standalone entry point over `gc_render::player_pose::select` -- takes
// exactly this module's own `MatchPlayer` JSON shape (its doc cross-checks
// `match_state_bridge.rs`'s encoder field-for-field, the same shape this
// file's own `makePlayers`/`snapshotState` fixture already builds) plus the
// same optional combat/keeper/outfield context `player_pose::select` itself
// takes, and returns `{id, priority, source}`.
//
// `@gc/render`'s own `package.json` declares a dependency on `@gc/wasm` for
// this: `@gc/wasm` is a `devDependency` here (mirroring `@gc/screens`), the
// workspace symlink at `node_modules/@gc/wasm` and this package's
// `tsconfig.json` project reference are both wired, and `host.playerPoseSelect`
// resolves and executes correctly from inside this spec file. `@gc/wasm`
// stays a devDependency deliberately: this spec may drive the real pose
// selector, but production `replay.ts` must keep receiving everything it
// needs through its existing arguments -- see ARCHITECTURE.md §1 on the
// determinism line, and do not import `@gc/wasm` from a non-spec file in
// this package.
//
// The "preserves a held pose-input field (keeper_get_up_timer)" test stays
// alongside the wasm-backed one: it pins the same underlying property (the
// buffered struct must carry `keeper_get_up_timer`) without a wasm round
// trip, so it is cheap regression coverage the wasm-backed test does not
// make redundant.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import type { CombatMatchState, CombatPlayerState } from "@gc/presentation";
import { loadSimHost } from "@gc/wasm";
import {
  replay,
  type MatchEvent,
  type MatchPlayer,
  type MatchState,
  type OutfieldDecisionState,
  type OutfieldPressState,
  type PossessionTransitionState,
  type Rect,
  type ReplayFrame,
  type TransitionWindows,
} from "./replay.ts";

function must<T>(value: T | undefined): T {
  if (value === undefined) {
    throw new Error("expected a defined value in test fixture");
  }
  return value;
}

const FIELD = { w: 960, h: 540 };
const GOAL_HOME: Rect = { x: -12, y: 210, w: 12, h: 120 };
const GOAL_AWAY: Rect = { x: 960, y: 210, w: 12, h: 120 };

function outfieldDecision(): OutfieldDecisionState {
  return { version: 1, generation: 0, rng_state: 1, remaining: 0, context: "offball", intent: "none" };
}

function pressState(): OutfieldPressState {
  return { version: 1, mode: "inactive", reason: "no_trigger" };
}

function transitionState(): PossessionTransitionState {
  return { version: 1, hold: 0, elapsed: 0 };
}

function windows(): TransitionWindows {
  return { counterpress: 0, counterattack: 0 };
}

const PLAYER_LAYOUT: ReadonlyArray<{
  readonly id: string;
  readonly team: "home" | "away";
  readonly isKeeper: boolean;
  readonly pos: readonly [number, number];
}> = [
  { id: "home_gk", team: "home", isKeeper: true, pos: [20, 270] },
  { id: "home_def", team: "home", isKeeper: false, pos: [150, 200] },
  { id: "home_mid", team: "home", isKeeper: false, pos: [300, 270] },
  { id: "home_fwd", team: "home", isKeeper: false, pos: [450, 300] },
  { id: "home_wing", team: "home", isKeeper: false, pos: [420, 150] },
  { id: "away_gk", team: "away", isKeeper: true, pos: [940, 270] },
  { id: "away_def", team: "away", isKeeper: false, pos: [810, 340] },
  { id: "away_mid", team: "away", isKeeper: false, pos: [660, 270] },
  { id: "away_fwd", team: "away", isKeeper: false, pos: [510, 240] },
  { id: "away_wing", team: "away", isKeeper: false, pos: [540, 390] },
];

/** Ten static players (index 0 and 5 are keepers), evenly split home/away. */
function makePlayers(keeperGetUpTimer = 0): MatchPlayer[] {
  return PLAYER_LAYOUT.map(({ id, team, isKeeper, pos }) => ({
    id,
    team,
    pos: new Vec2(pos[0], pos[1]),
    run_vel: new Vec2(0, 0),
    facing: new Vec2(team === "home" ? 1 : -1, 0),
    radius: 10,
    is_keeper: isKeeper,
    keeper_state: "base",
    keeper_set: 0,
    slide_timer: 0,
    tackle_timer: 0,
    stun_timer: 0,
    settle_timer: 0,
    sprinting: false,
    outfield_decision: outfieldDecision(),
    dive_timer: 0,
    dive_dir: new Vec2(0, 0),
    keeper_get_up_timer: isKeeper ? keeperGetUpTimer : 0,
    grab_timer: 0,
    throw_timer: 0,
    windup_timer: 0,
    aerial_timer: 0,
    aerial_jump: 0,
    sprint_meter: 1,
    jockey_timer: 0,
  }));
}

function snapshotState(
  players: readonly MatchPlayer[],
  ball: Vec2,
  ballVel: Vec2,
  events: readonly MatchEvent[],
): MatchState {
  return {
    field: FIELD,
    goal_home: GOAL_HOME,
    goal_away: GOAL_AWAY,
    score: { home: 0, away: 0 },
    time_left: 300,
    outfield_press: { home: pressState(), away: pressState() },
    transition: transitionState(),
    transition_windows: { home: windows(), away: windows() },
    ball,
    ball_vel: ballVel,
    ball_z: 0,
    ball_vz: 0,
    players,
    events,
  };
}

function advanceBall(ball: Vec2, vel: Vec2): Vec2 {
  return new Vec2(ball.x + vel.x / 60, ball.y + vel.y / 60);
}

function defaultCombatPlayer(): CombatPlayerState {
  return { phase: "ready", phase_ticks: 0, cooldown_ticks: 0, forced_ticks: 0, immunity_ticks: 0 };
}

function makeCombatState(ids: readonly string[]): CombatMatchState {
  return {
    tick: 0,
    player_ids: [...ids],
    players: ids.map(() => defaultCombatPlayer()),
    projectiles: [],
  };
}

const PLAYER_IDS = PLAYER_LAYOUT.map((p) => p.id);

describe("goal replay buffer", () => {
  it("records live frames and plays them back in slow motion", () => {
    replay.reset();
    replay.resetTuning();
    const players = makePlayers();
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    for (let i = 0; i < 90; i += 1) {
      replay.record(snapshotState(players, ball, ballVel, []));
      ball = advanceBall(ball, ballVel);
    }
    expect(replay.start("home")).toBe(true);
    expect(replay.active()).toBe(true);
    expect(replay.celebrating()).toBe(true);

    let frames = 0;
    let last: ReplayFrame | undefined;
    for (let i = 0; i < 2000; i += 1) {
      const st = replay.step(1 / 60);
      if (st === undefined) {
        break;
      }
      frames += 1;
      expect(st.players.length).toBe(10);
      expect(st.ball).toBeDefined();
      last = st;
    }
    expect(replay.active()).toBe(false);
    expect(last).toBeDefined();
    // Slow motion: playing ~90 recorded frames (+ tail) at 0.35x must take
    // meaningfully MORE display frames than were recorded.
    expect(frames > 90 * 2).toBe(true);
  });

  it("can be skipped", () => {
    replay.reset();
    replay.resetTuning();
    const players = makePlayers();
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    for (let i = 0; i < 60; i += 1) {
      replay.record(snapshotState(players, ball, ballVel, []));
      ball = advanceBall(ball, ballVel);
    }
    expect(replay.start("home")).toBe(true);
    replay.stop();
    expect(replay.active()).toBe(false);
    expect(replay.step(1 / 60)).toBeUndefined();
  });

  it("refuses to start without enough footage", () => {
    replay.reset();
    expect(replay.start("home")).toBe(false);
  });

  it("retains the authoritative combat companion in recorded frames", () => {
    replay.reset();
    replay.resetTuning();
    const players = makePlayers();
    const combatState = makeCombatState(PLAYER_IDS);
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    for (let i = 0; i < 60; i += 1) {
      replay.record(snapshotState(players, ball, ballVel, []), combatState);
      ball = advanceBall(ball, ballVel);
    }
    expect(replay.start("home")).toBe(true);
    const frame = must(replay.step(1 / 60));
    expect(frame._combat_state).toBeDefined();
    const combat = must(frame._combat_state);
    expect(combat.player_ids[1]).toBe(frame.players[1]?.id);
  });

  it("retains the corrected combat companion when replacing a rollback tail", () => {
    replay.reset();
    replay.resetTuning();
    const players = makePlayers();
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    let combatState = makeCombatState(PLAYER_IDS);
    for (let boundary = 0; boundary <= 39; boundary += 1) {
      replay.recordBoundary(boundary, snapshotState(players, ball, ballVel, []), combatState);
      ball = advanceBall(ball, ballVel);
    }

    replay.truncateFrom(20);
    const correctedPlayer: CombatPlayerState = {
      ...must(combatState.players[1]),
      phase: "recovery",
      phase_ticks: 9,
      cooldown_ticks: 47,
    };
    combatState = {
      ...combatState,
      players: combatState.players.map((p, i) => (i === 1 ? correctedPlayer : p)),
    };
    for (let boundary = 20; boundary <= 39; boundary += 1) {
      replay.recordBoundary(boundary, snapshotState(players, ball, ballVel, []), combatState);
      ball = advanceBall(ball, ballVel);
    }

    expect(replay.startAt("home", 39)).toBe(true);
    const frame = must(replay.step(1 / 60));
    const corrected = must(frame._combat_state);
    const p = must(corrected.players[1]);
    expect(p.phase).toBe("recovery");
    expect(p.phase_ticks).toBe(9);
    expect(p.cooldown_ticks).toBe(47);
  });

  it("celebrates before cutting to the slow-motion replay", () => {
    replay.reset();
    replay.resetTuning();
    const players = makePlayers();
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    for (let i = 0; i < 90; i += 1) {
      replay.record(snapshotState(players, ball, ballVel, []));
      ball = advanceBall(ball, ballVel);
    }
    expect(replay.start("home")).toBe(true);
    // The celebration runs first (real time), then playback takes over.
    let celebFrames = 0;
    for (let i = 0; i < 2000; i += 1) {
      const st = replay.step(1 / 60);
      if (st === undefined || !replay.celebrating()) {
        break;
      }
      celebFrames += 1;
      expect(st.players.length === 10 && st.ball !== undefined).toBe(true);
    }
    expect(celebFrames > 30).toBe(true);
    expect(replay.active() && !replay.celebrating()).toBe(true);
  });

  // pitch.draw hands buffered replay players straight to pose selection --
  // see the file header for how this reaches the real Rust selector via
  // @gc/wasm's playerPoseSelect.
  it("carries every pose input through capture, celebration, and playback", () => {
    const host = loadSimHost();
    replay.reset();
    replay.resetTuning();
    // Hold the post-dive get-up window open across the whole recording. The
    // buffered struct must carry it; the selector must not default it.
    const players = makePlayers(0.18);
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    for (let i = 0; i < 90; i += 1) {
      replay.record(snapshotState(players, ball, ballVel, []));
      ball = advanceBall(ball, ballVel);
    }

    expect(replay.start("home")).toBe(true);
    const neutralKeeperContext = { near_ball: false, shuffling: false, tip: false };
    let frames = 0;
    let getUpFrames = 0;
    for (let i = 0; i < 4000; i += 1) {
      const st = replay.step(1 / 60);
      if (st === undefined) {
        break;
      }
      frames += 1;
      for (const p of st.players) {
        const input: { readonly player: unknown; readonly keeper_context?: unknown } = {
          player: p,
          ...(p.is_keeper ? { keeper_context: neutralKeeperContext } : {}),
        };
        const selection = JSON.parse(host.playerPoseSelect(JSON.stringify(input))) as {
          readonly id: string;
          readonly priority: number;
          readonly source: string;
        };
        expect(selection.id.length > 0, "unknown replay pose").toBe(true);
        expect(typeof selection.priority).toBe("number");
        expect(["soccer", "combat", "locomotion"].includes(selection.source), "unknown pose source").toBe(true);
        if (p.is_keeper && selection.id === "keeper_get_up") {
          getUpFrames += 1;
        }
      }
    }
    expect(frames > 0, "replay produced no frames").toBe(true);
    expect(getUpFrames > 0, "the get-up window never reached a replay frame").toBe(true);
  });

  it("preserves a held pose-input field (keeper_get_up_timer) through capture, celebration, and playback", () => {
    replay.reset();
    replay.resetTuning();
    const players = makePlayers(0.18);
    const keeperIndex = players.findIndex((p) => p.is_keeper);
    expect(keeperIndex).toBeGreaterThanOrEqual(0);
    const keeperId = must(players[keeperIndex]).id;
    let ball = new Vec2(480, 270);
    const ballVel = new Vec2(120, 0);
    for (let i = 0; i < 90; i += 1) {
      replay.record(snapshotState(players, ball, ballVel, []));
      ball = advanceBall(ball, ballVel);
    }

    expect(replay.start("home")).toBe(true);
    let frames = 0;
    let sawKeeperTimer = false;
    for (let i = 0; i < 4000; i += 1) {
      const st = replay.step(1 / 60);
      if (st === undefined) {
        break;
      }
      frames += 1;
      const keeper = st.players.find((p) => p.id === keeperId);
      expect(keeper).toBeDefined();
      if (keeper !== undefined && keeper.keeper_get_up_timer === 0.18) {
        sawKeeperTimer = true;
      }
    }
    expect(frames > 0).toBe(true);
    expect(sawKeeperTimer).toBe(true);
  });
});
