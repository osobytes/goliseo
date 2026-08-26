// The project's declared world-unit-to-metre scale, and the metre-denominated
// claims that rest on it.
//
// This file exists because that scale used to be derived from the rig's own
// authored geometry, which is art-space and about 1.57 m. Every metre figure
// stated anywhere in the codebase was quietly quoted against that, and nothing
// checked it. These assertions are cheap; the confusion was not.
import { describe, expect, it } from "vitest";
import {
  DECLARED_PLAYER_HEIGHT_M,
  METRES_PER_WORLD_UNIT,
  DEFAULT_PLAYER_RADIUS,
  metresPerWorldUnit,
} from "./player_renderer_3d.ts";
import * as proportions from "./rig3d/proportions.ts";

// gc_sim's pitch and ball, in world units. Mirrored deliberately rather than
// imported: @gc/render must not depend on the sim, and a wrong copy here fails
// these tests loudly rather than drifting.
const FIELD_W = 1648;
const FIELD_H = 927;
const BALL_RADIUS = 6;
const GRAVITY = 900;

describe("the declared world-unit scale", () => {
  it("puts one world unit at 24.31 mm", () => {
    expect(METRES_PER_WORLD_UNIT * 1000).toBeCloseTo(24.306, 3);
  });

  it("is the declared height over the 72 px a player actually draws at", () => {
    expect(METRES_PER_WORLD_UNIT).toBe(metresPerWorldUnit(DECLARED_PLAYER_HEIGHT_M));
    expect(DEFAULT_PLAYER_RADIUS * 3.0 * 2).toBe(72);
  });

  it("is NOT the rig's own authored height, which is art-space", () => {
    // The whole point of the declaration. If these ever coincide it is because
    // someone rescaled the rig -- which leaves absolute-metre art behind (see
    // rig3d/equipment.ts, which never reads RigProportions) and is not the way
    // to make the two agree.
    const rigOwn = proportions.height(proportions.RIG_MEDIUM);
    expect(rigOwn).toBeLessThan(DECLARED_PLAYER_HEIGHT_M);
    expect(METRES_PER_WORLD_UNIT).not.toBe(metresPerWorldUnit(rigOwn));
  });
});

describe("what the declared scale makes true", () => {
  it("makes the pitch a regulation futsal court", () => {
    const length = FIELD_W * METRES_PER_WORLD_UNIT;
    const width = FIELD_H * METRES_PER_WORLD_UNIT;
    expect(length).toBeCloseTo(40.1, 1);
    expect(width).toBeCloseTo(22.5, 1);
    // Futsal specs: a regulation international court is 38-42 m
    // by 20-25 m, and the touchline must always exceed the end line.
    expect(length).toBeGreaterThanOrEqual(38);
    expect(length).toBeLessThanOrEqual(42);
    expect(width).toBeGreaterThanOrEqual(20);
    expect(width).toBeLessThanOrEqual(25);
    expect(length).toBeGreaterThan(width);
  });

  it("prices the ball's arcade oversize honestly", () => {
    const diameterCm = 2 * BALL_RADIUS * METRES_PER_WORLD_UNIT * 100;
    expect(diameterCm).toBeCloseTo(29.2, 1);
    // A real futsal ball is 20.5 cm. Ours is deliberately bigger for
    // readability; this pins how much bigger so it stays a choice.
    expect(diameterCm / 20.5).toBeCloseTo(1.42, 2);
  });

  it("prices gravity's arcade juice honestly", () => {
    const g = GRAVITY * METRES_PER_WORLD_UNIT;
    expect(g).toBeCloseTo(21.9, 1);
    // docs/vision.md: "Readable over realistic." Gravity is an arena feel dial
    // inherited from the Lua prototype, never derived from 9.81 -- but a
    // 2.2x Earth ball should be a stated decision, not a discovery.
    expect(g / 9.81).toBeCloseTo(2.23, 2);
  });
});
