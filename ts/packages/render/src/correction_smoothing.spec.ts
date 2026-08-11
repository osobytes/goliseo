// Fixture note (mirrors @gc/presentation's combat.spec.ts): a real
// `MatchState` would come from `sim.match`/`data.teams` (both Rust-owned,
// `crates/gc-sim` and `crates/gc-data`), neither of which exists in
// TypeScript. `correction_smoothing`'s functions are a pure transformation
// of whatever `{ players: [{id, pos}], ball: {x, y} }`-shaped source they
// are handed, so the fixture below is a self-consistent synthetic stand-in
// that exercises the exact same code paths. The purity assertions are
// expressed as a structural before/after `JSON.stringify` snapshot of the
// inputs -- there is no TypeScript `match_snapshot` to hash against, but
// "the inputs are untouched" is exactly what that would check, and this is
// the same substitution combat.spec.ts uses.

import { describe, expect, it } from "vitest";
import {
  correctionSmoothing,
  type CorrectionSmoothingSource,
  type CorrectionSmoothingSourcePlayer,
} from "./correction_smoothing.ts";
import { viewState } from "./view_state.ts";

function must<T>(value: T | undefined): T {
  if (value === undefined) {
    throw new Error("expected a defined value in test fixture");
  }
  return value;
}

function near(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

function snapshot(source: CorrectionSmoothingSource): string {
  return JSON.stringify(source);
}

const PLAYER_IDS = [
  "home_gk",
  "home_def_1",
  "home_def_2",
  "home_mid_1",
  "home_fwd_1",
  "away_gk",
  "away_def_1",
  "away_def_2",
  "away_mid_1",
  "away_fwd_1",
];

/** Ten players at distinct positions, matching `sim.match.new`'s player count. */
function newMatch(): CorrectionSmoothingSource {
  const players: CorrectionSmoothingSourcePlayer[] = PLAYER_IDS.map((id, i) => ({
    id,
    pos: { x: 40 + i * 40, y: 100 + i * 10 },
  }));
  return { players, ball: { x: 480, y: 270 } };
}

function correctedMatch(
  state: CorrectionSmoothingSource,
  playerDx: number,
  ballDx: number,
): CorrectionSmoothingSource {
  const players = state.players.map((p, i) =>
    i === 0 ? { id: p.id, pos: { x: p.pos.x + playerDx, y: p.pos.y } } : { id: p.id, pos: { ...p.pos } },
  );
  return { players, ball: { x: state.ball.x + ballDx, y: state.ball.y } };
}

describe("render correction smoothing", () => {
  it("starts at the previous pose and converges monotonically in 100 ms", () => {
    const authoritative = newMatch();
    const sourceSnapshot = snapshot(authoritative);
    const model = correctionSmoothing.new(authoritative);
    const originalPose = correctionSmoothing.pose(model);
    const corrected = correctedMatch(authoritative, 80, 40);
    const correctedSnapshot = snapshot(corrected);

    let smoothing = correctionSmoothing.correct(model, corrected);
    let pose = correctionSmoothing.pose(smoothing);
    const playerId = must(corrected.players[0]).id;
    near(must(pose.players[playerId]).x, must(originalPose.players[playerId]).x);
    near(pose.ball.x, originalPose.ball.x);
    expect(correctionSmoothing.diagnostics(smoothing).active_count).toBe(2);

    let previousDistance = Infinity;
    for (let i = 0; i < 4; i += 1) {
      smoothing = correctionSmoothing.advance(smoothing, corrected, 0.025);
      pose = correctionSmoothing.pose(smoothing);
      const distance = must(corrected.players[0]).pos.x - must(pose.players[playerId]).x;
      expect(distance >= 0).toBe(true);
      expect(distance < previousDistance).toBe(true);
      previousDistance = distance;
    }
    near(must(pose.players[playerId]).x, must(corrected.players[0]).pos.x);
    near(pose.ball.x, corrected.ball.x);
    expect(correctionSmoothing.diagnostics(smoothing).active_count).toBe(0);

    expect(snapshot(authoritative)).toBe(sourceSnapshot);
    expect(snapshot(corrected)).toBe(correctedSnapshot);
    expect(correctionSmoothing.diagnostics(model).active_count).toBe(0);
    near(
      must(correctionSmoothing.pose(model).players[playerId]).x,
      must(originalPose.players[playerId]).x,
    );
  });

  it("composes repeated corrections without a displayed discontinuity", () => {
    const authoritative = newMatch();
    const playerId = must(authoritative.players[0]).id;
    let model = correctionSmoothing.new(authoritative);
    const first = correctedMatch(authoritative, 80, 0);
    model = correctionSmoothing.correct(model, first);
    model = correctionSmoothing.advance(model, first, 0.025);
    const before = must(correctionSmoothing.pose(model).players[playerId]);

    const second = correctedMatch(authoritative, 100, 0);
    model = correctionSmoothing.correct(model, second);
    const after = must(correctionSmoothing.pose(model).players[playerId]);
    near(after.x, before.x);
    near(after.y, before.y);
    const diagnostics = correctionSmoothing.diagnostics(model);
    expect(diagnostics.active_count).toBe(1);
    expect(diagnostics.maximum_magnitude < correctionSmoothing.DEFAULT_HARD_SNAP_DISTANCE).toBe(true);

    model = correctionSmoothing.advance(model, second, 0.1);
    near(
      must(correctionSmoothing.pose(model).players[playerId]).x,
      must(second.players[0]).pos.x,
    );
    expect(correctionSmoothing.diagnostics(model).active_count).toBe(0);
  });

  it("advances player and ball poses on every consecutive correction frame", () => {
    const authoritative = newMatch();
    const authoritativeSnapshot = snapshot(authoritative);
    const playerId = must(authoritative.players[0]).id;
    let model = correctionSmoothing.new(authoritative);
    let previous = correctionSmoothing.pose(model);

    for (let frame = 1; frame <= 5; frame += 1) {
      const corrected = correctedMatch(authoritative, frame * 20, frame * 12);
      const correctedSnapshot = snapshot(corrected);
      model = correctionSmoothing.reconcile(model, corrected, 0.02);
      const pose = correctionSmoothing.pose(model);
      expect(must(pose.players[playerId]).x > must(previous.players[playerId]).x).toBe(true);
      expect(pose.ball.x > previous.ball.x).toBe(true);
      expect(must(pose.players[playerId]).x < must(corrected.players[0]).pos.x).toBe(true);
      expect(pose.ball.x < corrected.ball.x).toBe(true);
      const diagnostics = correctionSmoothing.diagnostics(model);
      expect(diagnostics.active_count).toBe(2);
      expect(diagnostics.maximum_magnitude < correctionSmoothing.DEFAULT_HARD_SNAP_DISTANCE).toBe(true);
      expect(snapshot(corrected)).toBe(correctedSnapshot);
      previous = pose;
    }

    expect(snapshot(authoritative)).toBe(authoritativeSnapshot);
  });

  it("hard-snaps corrections at the 160-world-unit threshold", () => {
    const authoritative = newMatch();
    const corrected = correctedMatch(authoritative, 160, 160);
    const model = correctionSmoothing.correct(correctionSmoothing.new(authoritative), corrected);
    const pose = correctionSmoothing.pose(model);
    const playerId = must(corrected.players[0]).id;
    near(must(pose.players[playerId]).x, must(corrected.players[0]).pos.x);
    near(pose.ball.x, corrected.ball.x);
    expect(correctionSmoothing.diagnostics(model).active_count).toBe(0);
  });

  it("uses render dt consistently across different render rates", () => {
    const authoritative = newMatch();
    const corrected = correctedMatch(authoritative, 90, 0);
    const playerId = must(corrected.players[0]).id;

    function run(dt: number, count: number) {
      let model = correctionSmoothing.correct(correctionSmoothing.new(authoritative), corrected);
      for (let i = 0; i < count; i += 1) {
        model = correctionSmoothing.advance(model, corrected, dt);
      }
      return model;
    }

    let slow = run(0.05, 1);
    let fast = run(0.01, 5);
    near(
      must(correctionSmoothing.pose(slow).players[playerId]).x,
      must(correctionSmoothing.pose(fast).players[playerId]).x,
    );
    near(
      correctionSmoothing.diagnostics(slow).maximum_magnitude,
      correctionSmoothing.diagnostics(fast).maximum_magnitude,
    );

    slow = run(0.05, 2);
    fast = run(0.01, 10);
    near(must(correctionSmoothing.pose(slow).players[playerId]).x, must(corrected.players[0]).pos.x);
    near(must(correctionSmoothing.pose(fast).players[playerId]).x, must(corrected.players[0]).pos.x);
  });

  it("clears offsets immediately at lifecycle discontinuities", () => {
    const authoritative = newMatch();
    const corrected = correctedMatch(authoritative, 70, 30);
    let model = correctionSmoothing.correct(correctionSmoothing.new(authoritative), corrected);
    expect(correctionSmoothing.diagnostics(model).active_count > 0).toBe(true);

    model = correctionSmoothing.clear(model, corrected);
    const pose = correctionSmoothing.pose(model);
    const playerId = must(corrected.players[0]).id;
    expect(correctionSmoothing.diagnostics(model).active_count).toBe(0);
    near(must(pose.players[playerId]).x, must(corrected.players[0]).pos.x);
    near(pose.ball.x, corrected.ball.x);
  });

  it("derives bounded gait and lean from the smoothed trajectory", () => {
    const authoritative = newMatch();
    const corrected = correctedMatch(authoritative, 60, 0);
    const correctedSnapshot = snapshot(corrected);
    const playerId = must(corrected.players[0]).id;
    let model = correctionSmoothing.new(authoritative);
    viewState.reset();
    viewState.update(authoritative.players, 0, correctionSmoothing.pose(model));

    model = correctionSmoothing.correct(model, corrected);
    viewState.update(corrected.players, 1 / 60, correctionSmoothing.pose(model));
    for (let i = 0; i < 7; i += 1) {
      model = correctionSmoothing.advance(model, corrected, 1 / 60);
      viewState.update(corrected.players, 1 / 60, correctionSmoothing.pose(model));
    }

    const view = must(viewState.get(playerId));
    expect(view.speed > 0).toBe(true);
    expect(view.speed <= viewState.MAX_DISPLAY_SPEED).toBe(true);
    expect(view.lean >= -1 && view.lean <= 1).toBe(true);
    expect(view.phase > 0).toBe(true);
    expect(snapshot(corrected)).toBe(correctedSnapshot);
    viewState.reset();
  });
});
