// Tests for pitch.ts's pure path (`pitchDrawCommands`).

import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import {
  pitchDrawCommands,
  playerAnchors,
  pitch,
  depthToZ,
  resetControlledMarkerPulse,
  BACKDROP_Z,
  ENTITY_Z_NEAR,
  ENTITY_Z_FAR,
  OVERLAY_RENDER_ORDER,
  type PitchDrawOptions,
  type PitchViewport,
  type RenderFrame,
} from "./pitch.ts";
import { camera } from "./camera.ts";
import * as playerRenderer3d from "./player_renderer_3d.ts";
import type { DrawCommand } from "./draw2d.ts";

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
        [1, 1, 1],
        [1, 1, 1],
      ],
      ids: ["home-1", "away-1"],
      // #447: real authored content ids, so this fixture drives the same
      // path a live roster does -- an outfielder carrying a loadout and a
      // player carrying none.
      presentation_ids: ["medieval_rook_emberguard", "scifi_axi"],
      loadout_ids: ["loadout_emberguard_shield", undefined],
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
    const backdropFill = commands.findIndex(
      (c) => c.kind === "rect" && c.w === viewport.w && c.h === viewport.h,
    );
    const pitchFill = commands.findIndex((c) => c.kind === "polygon" && c.mode === "fill");
    expect(backdropFill).toBeGreaterThanOrEqual(0);
    expect(pitchFill).toBeGreaterThan(backdropFill);
  });

  // NOTE ON WHAT MOVED (#415): a test here used to assert that players and the
  // ball were depth-sorted together, by counting the ground-shadow ellipses
  // `pitchDrawCommands` emitted. Players are no longer draw commands at all
  // (see that function's doc comment), so it has no subject in this suite. The
  // property it was reaching for -- a player and the ball interleaving by world
  // y rather than one class always drawing over the other -- is asserted
  // directly, on real objects, by "interleaves a rigged player with the ball in
  // painter's-algorithm order" below, and the depth-sorted ORDER of the anchors
  // themselves is pinned by the reference differential further down.

  it("skips the loose ball draw when it is not visible (e.g. held by a keeper)", () => {
    const visible = pitchDrawCommands(frame(), viewport, opts);
    const hidden = pitchDrawCommands(
      frame({ ball: { x: 480, y: 270, z: 0, visible: false } }),
      viewport,
      opts,
    );
    const ballCircle = (cs: typeof visible) =>
      cs.some(
        (c) => c.kind === "circle" && c.color[0] === 1 && c.color[1] === 0.95 && c.color[2] === 0.7,
      );
    expect(ballCircle(visible)).toBe(true);
    expect(ballCircle(hidden)).toBe(false);
  });

  it("draws a landing reticle only when the payload names a landing spot", () => {
    const without = pitchDrawCommands(frame(), viewport, opts);
    const withLanding = pitchDrawCommands(
      frame({ ball: { x: 480, y: 270, z: 40, visible: true, landing_x: 600, landing_y: 300 } }),
      viewport,
      opts,
      0.25,
    );
    const reticle = (cs: typeof without) =>
      cs.some(
        (c) =>
          c.kind === "circle" &&
          c.mode === "line" &&
          c.color[0] === 1 &&
          c.color[1] === 0.85 &&
          c.color[2] === 0.35,
      );
    expect(reticle(without)).toBe(false);
    expect(reticle(withLanding)).toBe(true);
  });

  it("draws a charge meter under the controlled player, colored by charge kind", () => {
    const shot = pitchDrawCommands(
      frame({ control: { charge: 0.6, charge_kind: "shot", controlled: 0 } }),
      viewport,
      opts,
    );
    const pass = pitchDrawCommands(
      frame({ control: { charge: 0.6, charge_kind: "pass", controlled: 0 } }),
      viewport,
      opts,
    );
    const meter = (cs: typeof shot, color: readonly [number, number, number]) =>
      cs.some(
        (c) =>
          c.kind === "rect" &&
          c.mode === "fill" &&
          c.color[0] === color[0] &&
          c.color[1] === color[1] &&
          c.color[2] === color[2],
      );
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
  it("defaults to the follow camera off, matching the Lua original", () => {
    expect(pitch.follow_camera).toBe(false);
  });

  // #415 removed `pitch.rigged_players`. There is no second player renderer to
  // select between, so a flag that could turn the only one off was a way to
  // ship a frame with no players in it.
  it("exposes no switch that could turn the rigged player pass off", () => {
    expect(Object.keys(pitch)).not.toContain("rigged_players");
  });

  it("defaults stadium_mode to off, so a world-space stadium layer is opt-in", () => {
    expect(pitch.stadium_mode).toBe(false);
  });
});

// STADIUM MODE: a world-space `WorldLayer` (scene.ts) owns the backdrop,
// floor, hex grid, markings, goals and arena chevrons; this screen-space
// path must stop building a second, flat copy of the same content while
// keeping the ball trail/combat/entities/overlay commands, which have no
// world-layer equivalent (see pitch.ts's own "STADIUM MODE" doc comment on
// `drawPitchBeforeItems`). `stadium_mode` is module-level mutable state
// shared with every other spec importing pitch.ts, so it is restored in
// `afterEach` -- the same pattern `pitch.draw (rigged compositing)` below
// uses for `pitch.rigged_players`.
describe("pitch.stadium_mode", () => {
  afterEach(() => {
    pitch.stadium_mode = false;
  });

  const FLOOR_COLOR = [0.025, 0.16, 0.17] as const; // DEFAULT_ARENA.floor_color, mirrored (not exported) -- see the "falls back to DEFAULT_ARENA" test above.
  const BALL_SHADOW_COLOR = [0, 0, 0] as const;
  const BALL_LIT_COLOR = [1, 0.95, 0.7] as const;

  function colorEq(c: readonly number[], target: readonly [number, number, number]): boolean {
    return c[0] === target[0] && c[1] === target[1] && c[2] === target[2];
  }

  it("removes the full-viewport backdrop rect and the pitch-surface fill polygon", () => {
    const off = pitchDrawCommands(frame(), viewport, opts);
    pitch.stadium_mode = true;
    const on = pitchDrawCommands(frame(), viewport, opts);

    const hasBackdropRect = (cs: readonly DrawCommand[]) =>
      cs.some(
        (c) => c.kind === "rect" && c.mode === "fill" && c.w === viewport.w && c.h === viewport.h,
      );
    const hasFloorPolygon = (cs: readonly DrawCommand[]) =>
      cs.some((c) => c.kind === "polygon" && c.mode === "fill" && colorEq(c.color, FLOOR_COLOR));

    expect(hasBackdropRect(off)).toBe(true);
    expect(hasBackdropRect(on)).toBe(false);
    expect(hasFloorPolygon(off)).toBe(true);
    expect(hasFloorPolygon(on)).toBe(false);
  });

  it("drops the overall command count substantially (the static scene + chevrons, not just the backdrop)", () => {
    const off = pitchDrawCommands(frame(), viewport, opts);
    pitch.stadium_mode = true;
    const on = pitchDrawCommands(frame(), viewport, opts);

    // The hex floor alone is on the order of hundreds of line commands (see
    // this file's own "batchStrokes" comment in pitch.ts, which measured 353
    // Line objects on a comparable frame) -- losing the whole static scene
    // should more than halve the command count for this otherwise-identical
    // fixture.
    expect(on.length).toBeLessThan(off.length / 2);
  });

  it("keeps the ball trail/entities/overlay: the loose ball still draws its shadow+lit circle, and the landing reticle still appears", () => {
    pitch.stadium_mode = true;
    const withLanding = pitchDrawCommands(
      frame({ ball: { x: 480, y: 270, z: 40, visible: true, landing_x: 600, landing_y: 300 } }),
      viewport,
      opts,
      0.25,
    );

    const hasBallShadow = withLanding.some(
      (c) => c.kind === "ellipse" && c.mode === "fill" && colorEq(c.color, BALL_SHADOW_COLOR),
    );
    const hasBallLit = withLanding.some(
      (c) => c.kind === "circle" && c.mode === "fill" && colorEq(c.color, BALL_LIT_COLOR),
    );
    const hasReticle = withLanding.some(
      (c) => c.kind === "circle" && c.mode === "line" && colorEq(c.color, [1, 0.85, 0.35]),
    );

    expect(hasBallShadow).toBe(true);
    expect(hasBallLit).toBe(true);
    expect(hasReticle).toBe(true);
  });

  it("keeps the charge meter overlay (an entity-anchored, non-arena command)", () => {
    pitch.stadium_mode = true;
    const commands = pitchDrawCommands(
      frame({ control: { charge: 0.6, charge_kind: "shot", controlled: 0 } }),
      viewport,
      opts,
    );
    const hasMeter = commands.some(
      (c) => c.kind === "rect" && c.mode === "fill" && colorEq(c.color, [1, 0.72, 0.3]),
    );
    expect(hasMeter).toBe(true);
  });
});

// CHARACTER TILT COHERENCE: `pitch.draw`'s rigged pass tilts every character
// by the angle the live camera actually looks down at the pitch from
// (`camera.rigAngleRad`) -- see pitch.ts's `characterTilt`. Both the
// quaternion CONSTRUCTION and `characterTilt` itself are private to pitch.ts,
// so this suite asserts the OBSERVABLE consequence: the wrapper's contained
// mesh carries that camera angle, and specifically NOT
// player_renderer_3d.ts's `ELEVATION`.
//
// `ELEVATION` is what this used to be a choice between: the tilt was that
// fixed constant under the retired fixed trapezoid (which had no real camera
// angle to ask for) and the rig angle otherwise, and this suite asserted the
// two disagreed. With one projection left the constant is no longer a
// candidate tilt at all -- it survives only as rig3d's cel-shader SHADING
// direction -- so what needs pinning is that a character is tilted to the
// camera's angle rather than drifting back to it.
describe("pitch.draw character tilt coherence", () => {
  function firstRiggedMeshQuaternion(f: RenderFrame): THREE.Quaternion {
    const group = new THREE.Group();
    // #415 dropped `pitch.draw`'s renderer parameter -- it rasterizes nothing.
    pitch.draw(group, f, viewport, opts);
    const wrapper = group.children.find((c) => c.userData["riggedCharacter"] === true);
    const mesh = wrapper?.children.find(
      (c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh,
    );
    if (mesh === undefined) {
      throw new Error("expected a rigged character mesh");
    }
    return mesh.quaternion.clone();
  }

  /**
   * The composed quaternion is `tilt * yaw` (see `riggedCharacterObject`'s
   * COMPOSITION ORDER note) and the yaw is private to `characterMesh`, so a
   * test cannot read the tilt off one frame without assuming the yaw. It can
   * read it off TWO frames that differ ONLY in the rig: the yaw is then
   * identical and divides out --
   *
   *   q2 * q1^-1 = (T2 * Y) * (T1 * Y)^-1 = T2 * T1^-1
   *
   * -- leaving exactly the change in tilt. That is what pins the tilt to the
   * camera 1:1 rather than to any constant (a constant would leave the two
   * quaternions equal) or to some scaled or offset function of it.
   */
  function tiltDeltaBetweenRigs(
    a: { height: number; distance: number },
    b: { height: number; distance: number },
  ): { measured: THREE.Quaternion; rigDelta: number } {
    const saved = camera.PERSPECTIVE;
    const set = (cfg: { height: number; distance: number }): void => {
      (camera as { PERSPECTIVE: typeof saved }).PERSPECTIVE = { ...saved, ...cfg };
    };
    try {
      const f = frame();
      set(a);
      const q1 = firstRiggedMeshQuaternion(f);
      const angleA = camera.rigAngleRad(f.field);
      set(b);
      const q2 = firstRiggedMeshQuaternion(f);
      const angleB = camera.rigAngleRad(f.field);
      return { measured: q2.multiply(q1.invert()), rigDelta: angleB - angleA };
    } finally {
      (camera as { PERSPECTIVE: typeof saved }).PERSPECTIVE = saved;
    }
  }

  it("tilts rigged characters by the camera rig's own downward angle, tracking it 1:1", () => {
    // Two deliberately different shots: ~38 degrees (the shipped tuning's
    // ratio) and ~60. If `characterTilt` returned any constant these two
    // would compose identically and the measured delta would be zero.
    const { measured, rigDelta } = tiltDeltaBetweenRigs(
      { height: 887, distance: 1135 },
      { height: 1135, distance: 655 },
    );
    expect(Math.abs(rigDelta)).toBeGreaterThan(0.3); // the two rigs really do differ
    const expected = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), rigDelta);
    expect(measured.angleTo(expected)).toBeLessThan(1e-6);
  });

  it("does not tilt them by player_renderer_3d's ELEVATION, the constant the retired projection stood in with", () => {
    // Belt and braces on the test above: ELEVATION is 17 degrees and the
    // shipped rig looks down at 38, so a character tilted by the constant
    // would sit ~21 degrees off the camera. Asserted as a floor on the gap
    // so this stays meaningful if either number is retuned -- and if they
    // ever DID coincide, the 1:1 test above is the one that still fails.
    const f = frame();
    const gap = Math.abs(camera.rigAngleRad(f.field) - playerRenderer3d.ELEVATION);
    expect(gap).toBeGreaterThan(0.1);
  });
});

describe("depthToZ", () => {
  it("maps the far edge (depth 0) to ENTITY_Z_NEAR and the near edge (depth === fieldH) to ENTITY_Z_FAR", () => {
    expect(depthToZ(0, 540)).toBeCloseTo(ENTITY_Z_NEAR);
    expect(depthToZ(540, 540)).toBeCloseTo(ENTITY_Z_FAR);
  });

  it("is monotonically increasing in depth, matching depthSortedItems' far-to-near sort direction", () => {
    expect(depthToZ(50, 540)).toBeLessThan(depthToZ(270, 540));
    expect(depthToZ(270, 540)).toBeLessThan(depthToZ(500, 540));
  });

  it("always returns a z strictly greater than BACKDROP_Z, so an entity never loses a depth test to the ground", () => {
    expect(depthToZ(0, 540)).toBeGreaterThan(BACKDROP_Z);
    expect(depthToZ(540, 540)).toBeGreaterThan(BACKDROP_Z);
  });

  it("clamps out-of-range depth instead of extrapolating past the entity zone", () => {
    expect(depthToZ(-100, 540)).toBeCloseTo(ENTITY_Z_NEAR);
    expect(depthToZ(10_000, 540)).toBeCloseTo(ENTITY_Z_FAR);
  });
});

// PITCH.DRAW'S RIGGED COMPOSITING (single-pass; see pitch.ts's "ONE PASS, ONE
// DEPTH BUFFER" header section; verified against a live GL context for
// before/after screenshots/draw-call counts).
// `player_renderer_3d.ts`'s `build()` only constructs plain three.js
// geometry/skeleton/material objects -- no GL calls -- so the rigged path
// genuinely runs under this workspace's "node" vitest environment, which is
// what lets every test below exercise the real product path. `pitch.draw`
// takes no `THREE.WebGLRenderer` at all since #415 (it never rasterized; the
// parameter existed only to gate the deleted billboard fallback), so this
// suite no longer needs a call-counting renderer stub to pin "draw builds an
// object graph and nothing else" -- the signature does. What IS verified here:
// a rigged player lands in the object graph as a real `THREE.SkinnedMesh` (not
// a quad) and in the correct painter's-algorithm position relative to the
// ball, and a character that cannot be built fails loudly; what is NOT
// verified (same as everywhere else GPU-adjacent in this package): the
// actual rendered pixel content.
describe("pitch.draw (rigged compositing)", () => {
  // A rigged player is now a `THREE.Group` wrapper (see pitch.ts's
  // `riggedCharacterObject`) marked `userData.riggedCharacter`, holding a
  // real `THREE.SkinnedMesh` -- not the old sprite quad tagged
  // `userData.ownedRenderTarget`.
  function riggedWrappers(children: readonly THREE.Object3D[]): THREE.Object3D[] {
    return children.filter((c) => c.userData["riggedCharacter"] === true);
  }

  const ballMesh = (c: THREE.Object3D): c is THREE.Mesh =>
    c instanceof THREE.Mesh &&
    !Array.isArray(c.material) &&
    c.material instanceof THREE.MeshBasicMaterial &&
    Math.abs(c.material.color.r - 1) < 1e-6 &&
    Math.abs(c.material.color.g - 0.95) < 1e-6 &&
    Math.abs(c.material.color.b - 0.7) < 1e-6;

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("adds a rigged player to the object graph as a real SkinnedMesh", () => {
    const f = frame();
    const group = new THREE.Group();

    pitch.draw(group, f, viewport, opts);

    const wrappers = riggedWrappers(group.children);
    expect(wrappers.length).toBeGreaterThan(0);
    for (const wrapper of wrappers) {
      const mesh = wrapper.children.find(
        (c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh,
      );
      expect(mesh).toBeDefined();
    }
  });

  // The whole point of the single-pass redesign: `pitch.draw` only builds the
  // object graph -- it never rasterizes, so the one render pass a real frame
  // needs happens exactly once, later, in scene.ts's `SceneRoot.render`. This
  // used to be asserted with a call-counting `THREE.WebGLRenderer` stub;
  // since #415 `draw` is not handed a renderer at all, so it is enforced by
  // the signature. Pinned here so re-adding the parameter is a deliberate act.
  it("cannot rasterize: draw takes no renderer at all", () => {
    // (group, frame, viewport, opts, now) -- `now` has a default, so `length`
    // counts the four required parameters.
    expect(pitch.draw.length).toBe(4);
  });

  it("reuses the SAME pooled mesh instance for the same playerId across frames", () => {
    const f = frame();
    const groupA = new THREE.Group();
    pitch.draw(groupA, f, viewport, opts);
    const meshA = riggedWrappers(groupA.children)[0]?.children.find(
      (c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh,
    );

    const groupB = new THREE.Group();
    pitch.draw(groupB, f, viewport, opts);
    const meshB = riggedWrappers(groupB.children)[0]?.children.find(
      (c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh,
    );

    expect(meshA).toBeDefined();
    expect(meshA).toBe(meshB);
  });

  it("interleaves a rigged player with the ball in painter's-algorithm order, matching playerAnchors' depth sort", () => {
    // Player 0 is far (small y), player 1 is near (large y); the ball sits
    // between them. depthSortedItems draws far-to-near, so the expected
    // object order is: player 0's wrapper, then the ball, then player 1's
    // wrapper.
    const f = frame({ players: { ...emptyPlayers(2), y: [50, 500] } });
    const group = new THREE.Group();

    pitch.draw(group, f, viewport, opts);

    const children = group.children;
    const wrapperIndices = children
      .map((c, i) => (c.userData["riggedCharacter"] === true ? i : -1))
      .filter((i) => i >= 0);
    expect(wrapperIndices).toHaveLength(2);
    const [farIndex, nearIndex] = wrapperIndices;
    if (farIndex === undefined || nearIndex === undefined) {
      throw new Error("expected two rigged player wrappers");
    }

    const ballIndex = children.findIndex(ballMesh);
    expect(ballIndex).toBeGreaterThan(-1);
    expect(farIndex).toBeLessThan(ballIndex);
    expect(ballIndex).toBeLessThan(nearIndex);
  });

  // FAIL LOUD ON A FAILED RIG BUILD (#415, AGENTS.md §7).
  //
  // These replace two "falls back to the procedural billboard" cases (one for
  // "no renderer supplied", one for `rigged_players = false`) and a third that
  // asserted a MID-ROSTER fallback got the right depth z. All three encoded
  // the same behaviour: a player the rigged pass declines is quietly drawn some
  // other way, or not at all, and the frame proceeds looking plausible.
  //
  // `characterMesh` declines only when `player_renderer_3d.build()` failed, and
  // `build()` fails on an invalid vertex index or missing rig3d content --
  // programmer errors. AGENTS.md §7: those `assert`, they do not degrade. The
  // cost of the old behaviour is on record: during #403 "the reporter may have
  // been on the fallback without realising" stayed a live hypothesis for hours
  // because the downgrade was invisible from the outside.
  describe("a rigged character that cannot be built", () => {
    it("throws, naming the player, rather than rendering that player as nothing", () => {
      const f = frame();
      const group = new THREE.Group();
      vi.spyOn(playerRenderer3d, "characterMesh").mockReturnValue(undefined);

      expect(() => {
        pitch.draw(group, f, viewport, opts);
      }).toThrow(/home-1/);
    });

    it("throws on a MID-ROSTER failure too -- the case that used to downgrade the rest of the team silently", () => {
      // The far player builds; the near one declines. Before #415 this frame
      // rendered: one rigged character, one billboard, no error, no signal.
      const f = frame({ players: { ...emptyPlayers(2), y: [50, 500] } });
      const group = new THREE.Group();
      const real = playerRenderer3d.characterMesh;
      let calls = 0;
      vi.spyOn(playerRenderer3d, "characterMesh").mockImplementation((...args) => {
        calls += 1;
        return calls === 1 ? real(...args) : undefined;
      });

      expect(() => {
        pitch.draw(group, f, viewport, opts);
      }).toThrow(/away-1/);
      expect(calls).toBe(2);
    });

    it("names player_renderer_3d.build() as the cause, so the failure is actionable without a bisect", () => {
      const f = frame();
      const group = new THREE.Group();
      vi.spyOn(playerRenderer3d, "characterMesh").mockReturnValue(undefined);

      expect(() => {
        pitch.draw(group, f, viewport, opts);
      }).toThrow(/player_renderer_3d\.build\(\) failed/);
    });
  });

  // DEPTH ZONES (pitch.ts's file header + the block above `depthToZ`).
  describe("depth placement", () => {
    it("leaves the backdrop (drawPitchBeforeItems) at BACKDROP_Z", () => {
      const f = frame();
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts);

      // The pitch surface trapezoid: the first filled ShapeGeometry mesh in
      // the child list, drawn well before any depth-sorted entity.
      const floor = group.children.find(
        (c) => c instanceof THREE.Mesh && c.geometry instanceof THREE.ShapeGeometry,
      );
      expect(floor).toBeDefined();
      expect(floor?.position.z).toBe(BACKDROP_Z);
    });

    it("positions a rigged player's wrapper within the entity depth zone, mapped from its world y", () => {
      const f = frame(); // both players and the ball sit at y=270, field.h=540
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts);

      const wrappers = riggedWrappers(group.children);
      expect(wrappers.length).toBeGreaterThan(0);
      const expectedZ = depthToZ(270, f.field.h);
      for (const wrapper of wrappers) {
        expect(wrapper.position.z).toBeCloseTo(expectedZ);
        expect(wrapper.position.z).toBeGreaterThan(BACKDROP_Z);
      }
    });

    it("positions the ball within the entity depth zone too, so it depth-tests against players consistently", () => {
      const f = frame();
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts);

      const ball = group.children.find(ballMesh);
      expect(ball).toBeDefined();
      expect(ball?.position.z).toBeCloseTo(depthToZ(f.ball.y, f.field.h));
    });

    it("makes the post-entity overlay layer (drawPitchAfterItems) ignore depth and win render order, so it always reads on top", () => {
      // The landing reticle: circle "line" commands only (no dl.text), so
      // this exercises the overlay layer without needing buildTextSprite's
      // document.createElement -- unavailable under this workspace's
      // DOM-less "node" vitest environment (see draw2d.ts's header).
      const f = frame({
        ball: { x: 480, y: 270, z: 40, visible: true, landing_x: 600, landing_y: 300 },
      });
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts);

      const isReticleRing = (c: THREE.Object3D): c is THREE.LineLoop =>
        c instanceof THREE.LineLoop &&
        c.material instanceof THREE.LineBasicMaterial &&
        Math.abs(c.material.color.r - 1) < 1e-6 &&
        Math.abs(c.material.color.g - 0.85) < 1e-6 &&
        Math.abs(c.material.color.b - 0.35) < 1e-6;
      const reticle = group.children.filter(isReticleRing);
      expect(reticle.length).toBeGreaterThan(0);
      for (const child of reticle) {
        expect(child.renderOrder).toBe(OVERLAY_RENDER_ORDER);
        expect((child.material as THREE.LineBasicMaterial).depthTest).toBe(false);
      }
    });
  });
});

function colorCloseTo(
  color: readonly number[],
  target: readonly [number, number, number],
): boolean {
  return (
    Math.abs((color[0] ?? NaN) - target[0]) < 1e-6 &&
    Math.abs((color[1] ?? NaN) - target[1]) < 1e-6 &&
    Math.abs((color[2] ?? NaN) - target[2]) < 1e-6
  );
}

// ROSTER SLOTS ARE ONE-BASED ON THE WIRE (ARCHITECTURE.md §4 rule 3, and
// frame_buffer.ts's own decode comment): `control.controlled` / `pass_target`
// name a roster SLOT, and pitch.ts has to subtract one before indexing its
// zero-based player arrays. Reading the raw slot instead draws the charge
// meter under the player standing one place further along the roster, and
// puts the pass-target ring on a player who is not the target -- or, when
// the slot is the last one, past the end of the array entirely, where
// pitch.ts's `?? 0` lands it at world (0, 0).
//
// This pair used to assert pinned pixel coordinates from the LOVE reference
// capture. That capture was taken through the retired fixed projection, so
// it went with it; the conversion is now asserted against `playerAnchors` --
// pitch.ts's own exported anchor seam, through whatever camera is live --
// which is what the pinned numbers were standing in for. Each test names
// both the RIGHT player and the WRONG one the off-by-one would have picked,
// so an off-by-one still fails here rather than merely moving the marker.
describe("pitch.ts converts frame.control.controlled/pass_target from one-based to zero-based", () => {
  /** Three well-separated players, so "right one" and "wrong one" cannot be confused for rounding. */
  function slotFrame(control: RenderFrame["control"]): RenderFrame {
    return frame({
      roster: {
        radius: [12, 12, 12],
        teams: ["home", "home", "away"],
        is_keeper: [false, false, false],
        species_shape: ["round", "round", "round"],
        species_color: [
          [1, 1, 1],
          [1, 1, 1],
          [1, 1, 1],
        ],
        ids: ["home-1", "home-2", "away-1"],
        presentation_ids: ["medieval_rook_emberguard", "scifi_axi", "toy_tock"],
        loadout_ids: ["loadout_emberguard_shield", undefined, undefined],
      },
      players: { ...emptyPlayers(3), x: [120, 480, 840], y: [80, 270, 460] },
      control,
    });
  }

  function anchorAt(f: RenderFrame, index: number): { sx: number; sy: number } {
    const anchor = playerAnchors(f, viewport, opts).find((a) => a.index === index);
    if (anchor === undefined) {
      throw new Error(`expected an anchor for roster index ${index}`);
    }
    return { sx: anchor.sx, sy: anchor.sy };
  }

  it("draws the charge meter under the controlled SLOT's player, not the player at that raw index", () => {
    // Slot 2 is roster index 1 ("home-2", world 480/270). Reading the slot
    // raw would pick index 2 ("away-1", world 840/460).
    const f = slotFrame({ charge_kind: "shot", charge: 0.55, controlled: 2 });
    const commands = pitchDrawCommands(f, viewport, opts, 0);
    const meterFill = commands.find(
      (c) => c.kind === "rect" && c.mode === "fill" && colorCloseTo(c.color, [1, 0.72, 0.3]),
    );
    expect(meterFill).toBeDefined();
    if (meterFill === undefined || meterFill.kind !== "rect") {
      throw new Error("expected the charge meter's fill bar");
    }
    // The bar is drawn from its left edge; its own `w` is the full track, of
    // which the fill shows `charge`. Centre the TRACK, not the fill.
    const track = commands.find(
      (c) => c.kind === "rect" && c.mode === "line" && Math.abs(c.x - meterFill.x) < 1e-9,
    );
    if (track === undefined || track.kind !== "rect") {
      throw new Error("expected the charge meter's outline track");
    }
    const centre = track.x + track.w / 2;
    expect(centre).toBeCloseTo(anchorAt(f, 1).sx, 6);
    expect(Math.abs(centre - anchorAt(f, 2).sx)).toBeGreaterThan(50);
  });

  it("draws the pass-target preview on the target SLOT's player, not the player at that raw index", () => {
    // Slot 3 is roster index 2 ("away-1"). Reading the slot raw would index
    // 3 -- off the end of a three-element array, which pitch.ts's `?? 0`
    // turns into world (0, 0), nowhere near any player.
    const f = slotFrame({ pass_target: 3, charge: 0, controlled: 0 });
    const commands = pitchDrawCommands(f, viewport, opts, 0);
    const rings = commands.filter(
      (c) => c.kind === "circle" && c.mode === "line" && colorCloseTo(c.color, [1, 0.4, 0.3]),
    );
    expect(rings.length).toBeGreaterThan(0);
    const target = anchorAt(f, 2);
    for (const ring of rings) {
      if (ring.kind !== "circle") {
        throw new Error("expected a circle command");
      }
      expect(ring.x).toBeCloseTo(target.sx, 6);
      expect(ring.y).toBeCloseTo(target.sy, 6);
    }
  });
});

describe("pitch.ts's pass-target marker has no memory across independent draws", () => {
  function threePlayerFrame(passTarget: number | undefined): RenderFrame {
    return frame({
      roster: {
        radius: [12, 12, 12],
        teams: ["home", "home", "away"],
        is_keeper: [false, false, false],
        species_shape: ["round", "round", "round"],
        species_color: [
          [1, 1, 1],
          [1, 1, 1],
          [1, 1, 1],
        ],
        ids: ["home-1", "home-2", "away-1"],
        presentation_ids: ["medieval_rook_emberguard", "scifi_axi", "toy_tock"],
        loadout_ids: ["loadout_emberguard_shield", undefined, undefined],
      },
      players: {
        ...emptyPlayers(3),
        x: [100, 700, 400],
        y: [100, 400, 250],
      },
      // `exactOptionalPropertyTypes` forbids an explicit `pass_target:
      // undefined` -- the key must be absent, not present-with-undefined,
      // matching frame_buffer.ts's own decode ("`pass_target !== 0 ? {
      // pass_target: passTarget } : {}`").
      control: {
        ...(passTarget !== undefined ? { pass_target: passTarget } : {}),
        charge: 0,
        controlled: 0,
      },
    });
  }

  const isPassRing = (c: DrawCommand) => c.kind === "circle" && c.mode === "line";

  it("moves the ring to the new winner on the very next call, and back again -- never the previous call's position", () => {
    const atHome1 = pitchDrawCommands(threePlayerFrame(1), viewport, opts);
    const ringAtHome1 = atHome1.find(isPassRing);
    expect(ringAtHome1).toBeDefined();

    const atHome2 = pitchDrawCommands(threePlayerFrame(2), viewport, opts);
    const ringAtHome2 = atHome2.find(isPassRing);
    expect(ringAtHome2).toBeDefined();
    // home-1 (100, 100) and home-2 (700, 400) project to clearly distinct
    // screen points at this viewport -- a real move, not sub-pixel noise.
    expect(
      ringAtHome2?.kind === "circle" && ringAtHome1?.kind === "circle"
        ? Math.hypot(ringAtHome2.x - ringAtHome1.x, ringAtHome2.y - ringAtHome1.y)
        : 0,
    ).toBeGreaterThan(50);

    // A rollback correction can also erase the winner outright.
    const cleared = pitchDrawCommands(threePlayerFrame(undefined), viewport, opts);
    expect(cleared.find(isPassRing)).toBeUndefined();

    // Back to home-1: must land exactly where the FIRST call put it, not
    // wherever the SECOND (home-2) or cleared call left something behind.
    const backToHome1 = pitchDrawCommands(threePlayerFrame(1), viewport, opts);
    const ringBackToHome1 = backToHome1.find(isPassRing);
    expect(ringBackToHome1).toBeDefined();
    expect(ringBackToHome1?.kind === "circle" ? ringBackToHome1.x : undefined).toBeCloseTo(
      ringAtHome1?.kind === "circle" ? ringAtHome1.x : NaN,
      6,
    );
    expect(ringBackToHome1?.kind === "circle" ? ringBackToHome1.y : undefined).toBeCloseTo(
      ringAtHome1?.kind === "circle" ? ringAtHome1.y : NaN,
      6,
    );
  });
});

// PERSISTENT CONTROLLED-PLAYER MARKER (docs/design/broadcast_presentation.md's
// "double-ringed controlled player", never previously implemented -- see
// pitch.ts's own CONTROLLED-PLAYER MARKER comment above `drawControlledMarker`).
// Unlike the pass-target preview above, this marker has no Lua reference to
// diverge from -- it is a genuinely new feature -- so these tests are pinned
// against pitch.ts's own behaviour rather than a captured fixture.
describe("pitch.ts's controlled-player marker", () => {
  // Mirrors pitch.ts's own (not exported) `CONTROLLED_MARKER_COLOR` -- same
  // convention as this file's `FLOOR_COLOR`/`RETICLE_COLOR` local mirrors.
  const CONTROLLED_MARKER_COLOR = [1, 0.92, 0.6] as const;

  function isControlledMarkerRing(c: DrawCommand): boolean {
    return (
      c.kind === "circle" && c.mode === "line" && colorCloseTo(c.color, CONTROLLED_MARKER_COLOR)
    );
  }

  // The marker's pulse is module-level state (see pitch.ts's own comment on
  // why) -- reset before every test in this block so none of them depend on
  // execution order or leak into any other suite in this file.
  beforeEach(() => {
    resetControlledMarkerPulse();
  });
  afterEach(() => {
    resetControlledMarkerPulse();
  });

  it("(a) draws a double ring at the controlled slot's position and nowhere else", () => {
    const f = frame({
      players: { ...emptyPlayers(2), x: [100, 700], y: [100, 400] },
      control: { charge: 0, controlled: 1 }, // one-based: roster index 0 ("home-1")
    });
    const commands = pitchDrawCommands(f, viewport, opts, 0);
    const rings = commands.filter(isControlledMarkerRing);
    expect(rings).toHaveLength(2); // the double ring

    const anchors = playerAnchors(f, viewport, opts);
    const anchor0 = anchors.find((a) => a.index === 0);
    const anchor1 = anchors.find((a) => a.index === 1);
    if (anchor0 === undefined || anchor1 === undefined) {
      throw new Error("expected anchors for both roster slots");
    }
    // home-1 (100, 100) and away-1 (700, 400) project to clearly distinct
    // screen points at this viewport -- a real position check, not sub-pixel
    // noise (same margin the pass-target "no memory" test above uses).
    expect(Math.hypot(anchor0.sx - anchor1.sx, anchor0.sy - anchor1.sy)).toBeGreaterThan(50);

    for (const ring of rings) {
      if (ring.kind !== "circle") {
        throw new Error("expected a circle command");
      }
      expect(ring.x).toBeCloseTo(anchor0.sx, 6);
      expect(ring.y).toBeCloseTo(anchor0.sy, 6);
      expect(Math.hypot(ring.x - anchor1.sx, ring.y - anchor1.sy)).toBeGreaterThan(50);
    }
  });

  it("(a) draws nothing when the controlled slot is out of range (e.g. the test-fixture default of 0, an invalid one-based slot)", () => {
    const f = frame({ control: { charge: 0, controlled: 0 } });
    const commands = pitchDrawCommands(f, viewport, opts, 0);
    expect(commands.some(isControlledMarkerRing)).toBe(false);
  });

  it("(b) persists when no charge is active, unlike the charge meter it sits near", () => {
    const f = frame({ control: { charge: 0, controlled: 1 } }); // no charge_kind at all
    const commands = pitchDrawCommands(f, viewport, opts, 0);

    expect(commands.some(isControlledMarkerRing)).toBe(true);
    const hasChargeMeter = commands.some(
      (c) =>
        c.kind === "rect" &&
        c.mode === "fill" &&
        (colorCloseTo(c.color, [1, 0.72, 0.3]) || colorCloseTo(c.color, [0.45, 0.85, 1])),
    );
    expect(hasChargeMeter).toBe(false);
  });

  it("(c) activates a brief pulse on a controlled-index change and decays back to the steady-state ring", () => {
    const innerRingR = (commands: readonly DrawCommand[]): number => {
      const ring = commands.find(isControlledMarkerRing);
      if (ring === undefined || ring.kind !== "circle") {
        throw new Error("expected a controlled-marker ring");
      }
      return ring.r;
    };

    // Both roster slots sit at the SAME world position in the default
    // fixture (`emptyPlayers`), so the projected `scale` is identical across
    // the switch below -- radius differences below are the pulse, not a
    // change in the player's own screen depth.
    const f1 = frame({ control: { charge: 0, controlled: 1 } }); // index 0
    const atSwitch = innerRingR(pitchDrawCommands(f1, viewport, opts, 0));
    const afterDecay = innerRingR(pitchDrawCommands(f1, viewport, opts, 1.0)); // >> 0.3s later, same index
    expect(atSwitch).toBeGreaterThan(afterDecay);

    const f2 = frame({ control: { charge: 0, controlled: 2 } }); // switch to index 1
    const atNewSwitch = innerRingR(pitchDrawCommands(f2, viewport, opts, 1.0));
    expect(atNewSwitch).toBeGreaterThan(afterDecay);

    const afterNewDecay = innerRingR(pitchDrawCommands(f2, viewport, opts, 1.3)); // +0.3s
    expect(atNewSwitch).toBeGreaterThan(afterNewDecay);
    // Decays back to the SAME steady-state radius, not a new baseline.
    expect(afterNewDecay).toBeCloseTo(afterDecay, 6);
  });
});

// ============================================================================
// VIEWPORT INVARIANCE (#414)
//
// The renderer-level half of camera.spec.ts's "camera.project across
// viewports" block. That one pins the projection's own arithmetic; this one
// pins what a player actually sees, through the real draw list: a player, a
// goal frame and the ball must all keep a constant size RELATIVE to the pitch
// as the window changes. Before the fix they did not -- positions carried the
// viewport factor and sizes did not -- so on any window that was not exactly
// 960x540 the pitch stretched to fill the frame while the entities stayed at
// their 960-wide pixel sizes.
//
// #414 confirmed the defect for players and left goals and the ball as
// "reported but not traced". Both are asserted here: every one of them is
// sized off the projection's third return value, so all three move together.
// ============================================================================
describe("pitch entity sizes stay in proportion to the pitch at any viewport", () => {
  const GOAL_FRAME_COLOR: readonly [number, number, number] = [0.92, 0.97, 1.0];
  const BALL_COLOR: readonly [number, number, number] = [1, 0.95, 0.7];

  /** Near (widest) edge of the pitch trapezoid, from the floor fill itself. */
  function pitchNearWidth(commands: readonly DrawCommand[]): number {
    const floor = commands.find((c) => c.kind === "polygon" && c.mode === "fill");
    if (floor === undefined || floor.kind !== "polygon") {
      throw new Error("expected the floor trapezoid fill");
    }
    // [far-left, far-right, near-right, near-left], x/y interleaved.
    return (floor.points[4] ?? NaN) - (floor.points[6] ?? NaN);
  }

  /**
   * Drawn radius handed to the player renderer -- `radius * scale`, for the
   * first player in depth-sorted order.
   *
   * This was captured by spying on `playerRenderer.playerDrawCommands` and
   * reading its third argument. #415 deleted that module, so it now reads
   * `playerAnchors`' `r` instead. The quantity is unchanged, and deliberately
   * so: both derivations build `project = pitchProject(frame, vp, opts)` from
   * the same three arguments, take `scale` from `project(players.x[i],
   * players.y[i])`, and return `roster.radius[i] * scale` -- the same
   * expression, the same amount of pipeline, for the same player (both walk
   * `depthSortedItems`, so index 0 is the same one the spy saw first). The
   * assertions below therefore still test what #414 fixed. If anything it is
   * a touch stronger: no mock, and `playerAnchors` is the exported seam
   * `pitch.draw` itself derives every rigged character's `ppm` from, so this
   * now reads the size the product actually draws rather than an argument to a
   * renderer the product no longer has.
   */
  function playerRadius(vp: PitchViewport): number {
    const first = playerAnchors(frame(), vp, opts)[0];
    if (first === undefined) {
      throw new Error("expected at least one player anchor");
    }
    return first.r;
  }

  /** Height of a goal post: `crossbar_h * scale`. */
  function goalPostHeight(commands: readonly DrawCommand[]): number {
    const post = commands.find(
      (c) =>
        c.kind === "line" && colorCloseTo(c.color, GOAL_FRAME_COLOR) && c.points[0] === c.points[2],
    );
    if (post === undefined || post.kind !== "line") {
      throw new Error("expected an upright goal post in the goal frame colour");
    }
    return Math.abs((post.points[1] ?? NaN) - (post.points[3] ?? NaN));
  }

  /** The loose ball's drawn radius: `5 * scale`. */
  function ballRadius(commands: readonly DrawCommand[]): number {
    const ball = commands.find(
      (c) => c.kind === "circle" && c.mode === "fill" && colorCloseTo(c.color, BALL_COLOR),
    );
    if (ball === undefined || ball.kind !== "circle") {
      throw new Error("expected the loose ball fill");
    }
    return ball.r;
  }

  const VIEWPORTS: readonly PitchViewport[] = [
    { w: 960, h: 540 }, // the reference implementation's window: the ONLY case the old specs covered
    { w: 1280, h: 720 },
    { w: 1920, h: 1080 },
    { w: 3440, h: 1440 }, // the ultrawide this was reported from
    { w: 1280, h: 1024 }, // a non-16:9 viewport, where the pitch used to stretch
  ];

  it("keeps player, goal-frame and ball sizes at a constant ratio to the pitch width across five viewports", () => {
    const ratios = VIEWPORTS.map((vp) => {
      const commands = pitchDrawCommands(frame(), vp, opts);
      const width = pitchNearWidth(commands);
      return {
        vp,
        player: playerRadius(vp) / width,
        goal: goalPostHeight(commands) / width,
        ball: ballRadius(commands) / width,
      };
    });
    const reference = ratios[0]!;
    for (const r of ratios) {
      expect(r.player, `player at ${r.vp.w}x${r.vp.h}`).toBeCloseTo(reference.player, 12);
      expect(r.goal, `goal at ${r.vp.w}x${r.vp.h}`).toBeCloseTo(reference.goal, 12);
      expect(r.ball, `ball at ${r.vp.w}x${r.vp.h}`).toBeCloseTo(reference.ball, 12);
    }
  });

  it("grows entities with the window: a 2x viewport draws a 2x player, goal and ball", () => {
    const base: PitchViewport = { w: 960, h: 540 };
    const doubled: PitchViewport = { w: 1920, h: 1080 };
    const baseCommands = pitchDrawCommands(frame(), base, opts);
    const doubledCommands = pitchDrawCommands(frame(), doubled, opts);
    expect(pitchNearWidth(doubledCommands) / pitchNearWidth(baseCommands)).toBeCloseTo(2, 9);
    expect(playerRadius(doubled) / playerRadius(base)).toBeCloseTo(2, 9);
    expect(goalPostHeight(doubledCommands) / goalPostHeight(baseCommands)).toBeCloseTo(2, 9);
    expect(ballRadius(doubledCommands) / ballRadius(baseCommands)).toBeCloseTo(2, 9);
  });
});
