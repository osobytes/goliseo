// Ported from spec/render/replay_spec.lua.
//
// Fixture note (mirrors @gc/presentation's combat.spec.ts): the Lua spec
// builds its fixture via `sim.match.new` + `match.step` (both Rust-owned,
// `crates/gc-sim`) and `data.teams` (Rust-owned, `crates/gc-data`). None of
// that exists in TypeScript. `replay`'s module functions are a pure
// transformation of whatever `MatchState`/`CombatMatchState`-shaped frames
// they are handed, so the fixture below is a self-consistent synthetic
// stand-in (ten static players, a ball drifting at a constant velocity) that
// exercises the exact same replay mechanics -- capture, boundary
// replace/truncate, celebration targeting, slow-motion playback, and the
// combat-state passenger -- without needing a running simulation.
//
// One test is intentionally not ported as written: "carries every pose
// input through capture, celebration, and playback" exercises
// `render/player_pose.lua`'s `select`/`PRIORITY`. That module has since
// landed as `crates/gc-render/src/player_pose.rs` (confirmed: pose selection
// -- `PlayerPoseId`, its priority table -- is real Rust there, and its
// output already crosses the wasm boundary baked into the RenderFrame wire,
// see `frame_buffer.rs`'s `pose_id`/`pose_priority`/`pose_source` fields and
// `pose_id_code`). So this is not a pending port -- it is finished, and it
// finished exactly where v2/README.md's file mapping said it would (Rust),
// permanently outside `@gc/render`.
//
// That does not unblock this test, though, and the reason is specific to
// what `replay` buffers: a goal replay captures pre-render `MatchState`/
// `MatchPlayer` snapshots (see this module's own header), not decoded
// RenderFrame wire data -- so a buffered, replayed frame was never run
// through Rust's pose selection in the first place, and there is no
// `@gc/wasm` binding that takes an arbitrary buffered player snapshot and
// returns a selected pose id the way `buildRenderFrame` does for a live
// `SimSession` (see `packages/wasm/src/index.ts`'s `SimHost` -- its only
// pose-shaped surface is the live per-session render frame). Unblocked by
// either a new wasm export exposing `player_pose::select` standalone, or by
// changing what `replay` captures so pose ids are baked in at record time --
// neither of which is this package's call to make alone. It is kept below as
// `it.skip` per v2/README.md §4 ("port it as #[ignore]/it.skip... and report
// it -- never delete it silently"), plus a replacement test that asserts
// directly on the field the selector would have read
// (`keeper_get_up_timer`), which is the property that test was actually
// protecting: "the buffered struct must carry it; the selector must not
// default it."
//
// Re-checked, not just trusted, against this task's landed surface:
// `SimSession.matchStateJson()`/`RollbackPlayableLab.currentMatchStateJson`
// et al. (`@gc/wasm`) add exactly `MatchState`-shaped JSON (`field`,
// `goal_home`, `goal_away`, `score`, `time_left`, `outfield_press`,
// `transition`, `transition_windows`, `controlled?`, `owner?`, `ball`,
// `ball_vel`, `ball_z`, `ball_vz`, `players`, `events`) -- this module's own
// `MatchState`/`MatchPlayer` interfaces, verbatim, per `@gc/wasm`'s own doc
// on those methods.
//
// # Re-audited again: the standalone export landed, but this package cannot
// reach it
//
// The paragraph above is now stale on its own terms: `@gc/wasm` DOES expose
// standalone pose selection now -- `crates/gc-wasm/src/player_pose_bridge.rs`'s
// `playerPoseSelect` takes exactly this module's own `MatchPlayer` JSON shape
// (its doc cross-checks `match_state_bridge.rs`'s encoder field-for-field,
// the same shape this file's own `makePlayers`/`snapshotState` fixture
// already builds) plus the same optional combat/keeper/outfield context
// `player_pose::select` itself takes, and returns `{id, priority, source}` --
// exactly what the Lua original's `player_pose.select(p, nil, ...)` call
// needs. Confirmed reachable and correct in isolation: `node -e` against
// `packages/wasm/dist/pkg/gc_wasm.cjs` resolves `playerPoseSelect` as a
// function.
//
// What is NOT true, checked directly rather than assumed: `@gc/render`'s own
// `package.json` declares no dependency on `@gc/wasm` at all (unlike
// `@gc/screens`, which lists it under `devDependencies` for exactly this
// "specs may import it directly" reason -- v2/README.md's package layout).
// `packages/render/node_modules/@gc/` only symlinks `core`/`presentation`;
// `require.resolve("@gc/wasm", { paths: [...] })` from inside this package
// fails outright (confirmed by running it, not inferred). Reaching
// `playerPoseSelect` from this file therefore needs a `package.json` edit
// (adding `@gc/wasm` to `devDependencies`, mirroring `@gc/screens`) followed
// by a workspace install to materialize the symlink -- both outside a single
// task batch scoped to `replay.ts`/`replay.spec.ts` alone, and the install
// step outside what an automated pass here may run at all. So the blocker
// this test now names is a one-line, low-risk DEPENDENCY-GRAPH gap for
// whoever owns this package's `package.json` and can run the install step --
// not a missing Rust/wasm surface, which is what every prior pass here
// assumed. Left `it.skip`, for this third, narrower, and different reason.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import type { CombatMatchState, CombatPlayerState } from "@gc/presentation";
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

  // pitch.draw hands buffered replay players straight to player_pose.select
  // in the Lua original -- see the file header (the "Re-audited again"
  // section) for why this is still skipped rather than ported as written.
  it.skip(
    "carries every pose input through capture, celebration, and playback " +
      "-- @gc/wasm's playerPoseSelect (crates/gc-wasm/src/player_pose_bridge.rs) " +
      "is now exactly the standalone entry point this case needs, confirmed " +
      "reachable and correct in isolation. The remaining blocker is narrower " +
      "and different: @gc/render's own package.json declares no dependency " +
      "on @gc/wasm at all (unlike @gc/screens, which does, for exactly this " +
      "'specs may import it directly' reason) -- packages/render/node_modules/@gc/ " +
      "has no wasm symlink, and require.resolve('@gc/wasm', ...) fails from " +
      "inside this package (confirmed by running it). Unblocks on adding " +
      "@gc/wasm to this package's devDependencies plus a workspace install " +
      "to materialize the symlink -- a package.json edit and an install step, " +
      "neither of which this file's own port can make.",
    () => {
      // Intentionally not ported; see skip reason above and the file header.
    },
  );

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
