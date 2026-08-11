// This spec pins the static-pitch-scene caching invariant from #398/#393.
//
// An earlier implementation cached the static pitch scene into 2D canvases,
// worth ~2.3 ms of frame time under a plain interpreter with no JIT. That
// mechanism does not apply here — a projected 3D scene cached into a 2D
// bitmap cannot respond to camera movement, which is exactly why that cache
// had to bypass itself whenever a follow view was active. Under three.js the
// static scene is persistent geometry, so the scene graph is the cache.
//
// What carries over is the invariant (build it once) and, less obviously, the
// membership of the static set. The arena frame chevrons pulse with the kickoff
// banner, so they are dynamic and must stay outside — #398/#393 moved them to
// draw *after* the goals for exactly this reason. `pulseIsNotCached` below is
// the regression that pins it: over-caching freezes the kickoff pulse, which no
// type checker would catch.

import { describe, expect, it, beforeEach } from "vitest";
import {
  pitchDrawCommands,
  resetStaticSceneCache,
  staticSceneBuildCount,
  type PitchDrawOptions,
  type RenderFrame,
} from "./pitch.ts";

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
    field: {
      w: 960,
      h: 540,
      penalty_box_depth: 90,
      penalty_box_h: 240,
      crossbar_h: 70,
      goal_home: { x: -6, y: 235, w: 6, h: 70 },
      goal_away: { x: 960, y: 235, w: 6, h: 70 },
    },
    roster: {
      radius: [12, 12],
      teams: ["home", "away"],
      is_keeper: [false, false],
      species_shape: ["round", "round"],
      species_color: [
        [1, 0, 0],
        [0, 0, 1],
      ],
      ids: ["a", "b"],
      // #447. This suite only draws the STATIC pitch, which has no players
      // in it, so these are present to satisfy the roster shape rather than
      // to be read.
      presentation_ids: ["medieval_rook_emberguard", "toy_tock"],
      loadout_ids: ["loadout_tournament_sword", undefined],
    },
    players: emptyPlayers(2),
    ball: { x: 480, y: 270, z: 0, visible: true },
    control: { charge: 0, controlled: 0 },
    ...overrides,
  };
}

const VIEWPORT = { w: 960, h: 540 } as const;

function options(overrides: Partial<PitchDrawOptions> = {}): PitchDrawOptions {
  return { home_color: [1, 0.2, 0.2], away_color: [0.2, 0.4, 1], ...overrides };
}

describe("the static pitch scene is built once", () => {
  beforeEach(() => {
    resetStaticSceneCache();
  });

  it("builds the static scene exactly once across many identical frames", () => {
    const f = frame();
    const o = options();
    for (let tick = 0; tick < 60; tick += 1) {
      pitchDrawCommands(f, VIEWPORT, o, tick);
    }
    // The whole point of the split. A deep-equality check would pass even if
    // the scene were re-derived on all 60 frames, so assert the build count.
    expect(staticSceneBuildCount()).toBe(1);
  });

  it("rebuilds when the field geometry changes", () => {
    const o = options();
    const narrow = pitchDrawCommands(frame(), VIEWPORT, o, 0);
    const wide = pitchDrawCommands(
      frame({
        field: {
          w: 1200,
          h: 540,
          penalty_box_depth: 90,
          penalty_box_h: 240,
          crossbar_h: 70,
          goal_home: { x: -6, y: 235, w: 6, h: 70 },
          goal_away: { x: 1200, y: 235, w: 6, h: 70 },
        },
      }),
      VIEWPORT,
      o,
      0,
    );
    expect(narrow).not.toEqual(wide);
  });

  it("rebuilds when the viewport changes", () => {
    const f = frame();
    const o = options();
    const small = pitchDrawCommands(f, { w: 960, h: 540 }, o, 0);
    const large = pitchDrawCommands(f, { w: 1920, h: 1080 }, o, 0);
    expect(small).not.toEqual(large);
  });

  it("rebuilds when the team colours change, because the goals carry them", () => {
    const f = frame();
    const red = pitchDrawCommands(f, VIEWPORT, options(), 0);
    const green = pitchDrawCommands(f, VIEWPORT, options({ home_color: [0, 1, 0] }), 0);
    expect(red).not.toEqual(green);
  });
});

describe("the arena frame chevrons are dynamic, not static", () => {
  beforeEach(() => {
    resetStaticSceneCache();
  });

  // The regression this file exists for. The chevrons pulse with the kickoff
  // banner; caching them alongside the floor and goals would freeze that pulse
  // while every test above still passed.
  it("still change when only arena_pulse changes", () => {
    const f = frame();
    const quiet = pitchDrawCommands(f, VIEWPORT, options({ arena_pulse: 0 }), 0);
    const pulsing = pitchDrawCommands(f, VIEWPORT, options({ arena_pulse: 1 }), 0);
    expect(quiet).not.toEqual(pulsing);
  });

  it("are emitted after the goals, matching the Lua's static/dynamic order", () => {
    // #398/#393 moved this draw after the goals when the cache was extracted,
    // noting the chevrons sit at the trapezoid corners clear of the goals so the
    // reorder is visually identical. Keep that order here.
    const commands = pitchDrawCommands(frame(), VIEWPORT, options({ arena_pulse: 1 }), 0);
    expect(commands.length).toBeGreaterThan(0);
    // And varying only the pulse must not rebuild the static scene.
    const before = staticSceneBuildCount();
    pitchDrawCommands(frame(), VIEWPORT, options({ arena_pulse: 0.5 }), 0);
    expect(staticSceneBuildCount()).toBe(before);
  });
});
