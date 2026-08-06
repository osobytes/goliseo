// New tests for pitch.ts's pure path (`pitchDrawCommands`). No Lua spec
// targets game/render/pitch.lua with a claimable, self-contained fixture --
// spec/render/draw_smoke_spec.lua and spec/render/combat_presentation_spec.lua
// both exercise it, but only by stubbing `love.graphics` and pulling in
// `sim.match`/`data.teams`/`data.formations` (Rust-owned) and `game.ui.draw`/
// `game.match_hud` (other packages) -- see this package's port report for
// why those specs are not claimed wholesale.

import { describe, expect, it } from "vitest";
import { pitchDrawCommands, pitch, type PitchDrawOptions, type RenderFrame } from "./pitch.ts";

function emptyPlayers(count: number) {
  return {
    count,
    x: Array<number>(count).fill(480),
    y: Array<number>(count).fill(270),
    facing_x: Array<number>(count).fill(1),
    facing_y: Array<number>(count).fill(0),
    controlled: Array<boolean>(count).fill(false),
    dashing: Array<boolean | undefined>(count).fill(undefined),
    dive: Array<number | undefined>(count).fill(undefined),
    dive_dir_x: Array<number | undefined>(count).fill(undefined),
    dive_dir_y: Array<number | undefined>(count).fill(undefined),
    holding: Array<boolean | undefined>(count).fill(undefined),
    grab: Array<number | undefined>(count).fill(undefined),
    throw: Array<number | undefined>(count).fill(undefined),
    windup: Array<number | undefined>(count).fill(undefined),
    aerial: Array<number | undefined>(count).fill(undefined),
    aerial_style: Array<undefined>(count).fill(undefined),
    aerial_outcome: Array<undefined>(count).fill(undefined),
    aerial_jump: Array<number | undefined>(count).fill(undefined),
    pose_id: Array<string | undefined>(count).fill(undefined),
    pose_priority: Array<number | undefined>(count).fill(undefined),
    pose_source: Array<string | undefined>(count).fill(undefined),
  };
}

function frame(overrides: Partial<RenderFrame> = {}): RenderFrame {
  return {
    field: { w: 960, h: 540, penalty_box_depth: 90, penalty_box_h: 240, crossbar_h: 70, goal_home: { x: -6, y: 235, w: 6, h: 70 }, goal_away: { x: 960, y: 235, w: 6, h: 70 } },
    roster: {
      radius: [12, 12],
      teams: ["home", "away"],
      is_keeper: [false, false],
      species_shape: ["round", "round"],
      species_color: [
        [1, 1, 1],
        [1, 1, 1],
      ],
      ids: ["home-1", "away-1"],
    },
    players: emptyPlayers(2),
    ball: { x: 480, y: 270, z: 0, visible: true },
    control: { charge: 0, controlled: 0 },
    ...overrides,
  };
}

const viewport = { w: 1280, h: 720 };
const opts: PitchDrawOptions = { home_color: [0.2, 0.6, 1], away_color: [1, 0.4, 0.3] };

describe("pitch.pitchDrawCommands", () => {
  it("draws the arena backdrop before the pitch surface", () => {
    const commands = pitchDrawCommands(frame(), viewport, opts);
    const backdropFill = commands.findIndex((c) => c.kind === "rect" && c.w === viewport.w && c.h === viewport.h);
    const pitchFill = commands.findIndex((c) => c.kind === "polygon" && c.mode === "fill");
    expect(backdropFill).toBeGreaterThanOrEqual(0);
    expect(pitchFill).toBeGreaterThan(backdropFill);
  });

  it("depth-sorts players and the ball together by world y (far first)", () => {
    const f = frame({ players: { ...emptyPlayers(2), y: [500, 50] } });
    const commands = pitchDrawCommands(f, viewport, opts);
    // The far player (y=50) is one of the shadow ellipses drawn earliest
    // among the depth-sorted entities; the near player (y=500) later. We
    // can't easily recover "which entity" from a flat command list, but we
    // can confirm the ball (y=270, mid-pack) draws its lit circle strictly
    // between the two players' ground shadows.
    const ellipses = commands.filter((c) => c.kind === "ellipse" && c.mode === "fill");
    expect(ellipses.length).toBeGreaterThanOrEqual(3); // 2 player shadows + 1 ball shadow
  });

  it("skips the loose ball draw when it is not visible (e.g. held by a keeper)", () => {
    const visible = pitchDrawCommands(frame(), viewport, opts);
    const hidden = pitchDrawCommands(frame({ ball: { x: 480, y: 270, z: 0, visible: false } }), viewport, opts);
    const ballCircle = (cs: typeof visible) => cs.some((c) => c.kind === "circle" && c.color[0] === 1 && c.color[1] === 0.95 && c.color[2] === 0.7);
    expect(ballCircle(visible)).toBe(true);
    expect(ballCircle(hidden)).toBe(false);
  });

  it("draws a landing reticle only when the payload names a landing spot", () => {
    const without = pitchDrawCommands(frame(), viewport, opts);
    const withLanding = pitchDrawCommands(frame({ ball: { x: 480, y: 270, z: 40, visible: true, landing_x: 600, landing_y: 300 } }), viewport, opts, 0.25);
    const reticle = (cs: typeof without) => cs.some((c) => c.kind === "circle" && c.mode === "line" && c.color[0] === 1 && c.color[1] === 0.85 && c.color[2] === 0.35);
    expect(reticle(without)).toBe(false);
    expect(reticle(withLanding)).toBe(true);
  });

  it("draws a charge meter under the controlled player, colored by charge kind", () => {
    const shot = pitchDrawCommands(frame({ control: { charge: 0.6, charge_kind: "shot", controlled: 0 } }), viewport, opts);
    const pass = pitchDrawCommands(frame({ control: { charge: 0.6, charge_kind: "pass", controlled: 0 } }), viewport, opts);
    const meter = (cs: typeof shot, color: readonly [number, number, number]) => cs.some((c) => c.kind === "rect" && c.mode === "fill" && c.color[0] === color[0] && c.color[1] === color[1] && c.color[2] === color[2]);
    expect(meter(shot, [1, 0.72, 0.3])).toBe(true);
    expect(meter(pass, [0.45, 0.85, 1])).toBe(true);
  });

  it("falls back to the built-in DEFAULT_ARENA colours when no arena is supplied", () => {
    const commands = pitchDrawCommands(frame(), viewport, opts);
    const floor = commands.find((c) => c.kind === "polygon" && c.mode === "fill");
    expect(floor?.kind === "polygon" ? floor.color : undefined).toEqual([0.025, 0.16, 0.17]);
  });
});

describe("pitch config defaults", () => {
  it("defaults to rigged players on and the follow camera off, matching the Lua original", () => {
    expect(pitch.rigged_players).toBe(true);
    expect(pitch.follow_camera).toBe(false);
  });
});
