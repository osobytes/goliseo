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
// by `characterTilt`'s return -- `ELEVATION_TILT` (a fixed quaternion) under
// the flat trapezoid, or `camera.rigAngleRad`'s real camera angle under
// `camera.perspective_mode`. Both quaternion CONSTRUCTION and the module-
// level `camera.perspective_mode`/`ELEVATION` values are private to pitch.ts
// (`characterTilt`/`ELEVATION_TILT` are not exported), so this suite asserts
// the OBSERVABLE consequence instead: the wrapper's contained mesh ends up
// with a DIFFERENT quaternion depending on the mode, and specifically the
// perspective-mode one should differ from the fixed one (since
// camera.PERSPECTIVE's tuned tilt, 45 degrees, does not equal
// player_renderer_3d.ts's ELEVATION -- if it ever did coincidentally, this
// test would need a mode-independent invariant instead).
describe("pitch.draw character tilt coherence with camera.perspective_mode", () => {
  function firstRiggedMeshQuaternion(f: RenderFrame): THREE.Quaternion | undefined {
    const group = new THREE.Group();
    // #415 dropped `pitch.draw`'s renderer parameter -- it rasterizes nothing.
    pitch.draw(group, f, viewport, opts);
    const wrapper = group.children.find((c) => c.userData["riggedCharacter"] === true);
    const mesh = wrapper?.children.find(
      (c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh,
    );
    return mesh?.quaternion.clone();
  }

  afterEach(() => {
    camera.perspective_mode = false;
  });

  it("tilts rigged characters differently under perspective_mode than under the fixed trapezoid", () => {
    const f = frame();

    camera.perspective_mode = false;
    const fixedQuat = firstRiggedMeshQuaternion(f);

    camera.perspective_mode = true;
    const perspectiveQuat = firstRiggedMeshQuaternion(f);

    expect(fixedQuat).toBeDefined();
    expect(perspectiveQuat).toBeDefined();
    if (fixedQuat === undefined || perspectiveQuat === undefined) {
      throw new Error("expected a rigged character mesh in both modes");
    }
    expect(fixedQuat.equals(perspectiveQuat)).toBe(false);
  });
});

// DEPTH ZONES (see pitch.ts's file header "ONE PASS, ONE DEPTH BUFFER" and
// the "DEPTH ZONES" block above `depthToZ`). `depthToZ` is the pure mapping
// from a depth-sort key to a real, depth-testable world z; tested directly
// here since it needs no GL context at all.
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

// ============================================================================
// PINNED REFERENCE DIFFERENTIAL -- tools/render_reference/. See that
// directory's README.md for how this was captured, and for exactly what is
// (and is deliberately not) covered: full command-level parity for
// everything pitch.ts draws directly (arena, floor, hex tiling, markings,
// goals, outline, chevrons, the loose ball, the overlay layer), plus the
// full per-player anchor + PlayerRenderOptions payload handed to the player
// renderer -- NOT the drawn character's own internal geometry (the rig has its
// own suites under rig3d/), and NOT the relative depth order
// BETWEEN a player and the ball (the comparator was verified by direct
// side-by-side reading instead of a second capture).
//
// TWO HALVES SINCE #415. The captured reference is one flat draw list
// interleaving geometry with player-draw calls. TypeScript now produces
// those in two pure functions rather than one: `pitchDrawCommands`
// (everything that is not a player) and `playerAnchors` (the per-player
// anchor + options payload, in the same depth-sorted order). The comparison
// is unchanged in substance -- `LUA_GEOM_RECORDS` was always compared
// against a command list with the player draws mocked out to `[]`, and
// `LUA_PLAYER_RECORDS` against the captured per-player calls -- but neither
// side needs a `vi.spyOn` on a renderer module to separate them anymore. The
// one assertion genuinely lost is the per-player `color`: it was the
// original billboard renderer's team tint, and the rigged path resolves
// colour from rig3d's own team palette instead, so nothing consumes it. The
// team it was derived from is still asserted, via `options.team`.
//
// This is the gate that the rest of this codebase's differential coverage
// (five suites, bit-pattern comparison, pinned reference fixtures for the
// wire/diagnostics/desync path) never extended to rendering, until this
// suite.
// ============================================================================

const LUA_REFERENCE_JSON = `[{"alpha":1,"color":[0.014999999999999999,0.021999999999999999,0.055],"h":120,"kind":"rect","mode":"fill","w":200,"x":0,"y":0},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":10,"y":10.799999999999999},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":24,"y":21.599999999999998},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":40,"y":7.1999999999999993},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":57.999999999999993,"y":18},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":74,"y":6},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":92,"y":15.600000000000001},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":110.00000000000001,"y":8.4000000000000004},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":128,"y":19.199999999999999},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":144,"y":6},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":162,"y":15.600000000000001},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":180,"y":8.4000000000000004},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":192,"y":21.599999999999998},{"alpha":0.12,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"circle","mode":"fill","r":9,"x":100,"y":24.599999999999998},{"alpha":0.78000000000000003,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"circle","mode":"fill","r":4.0800000000000001,"x":100,"y":24.599999999999998},{"alpha":0.69999999999999996,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"ellipse","lineWidth":1,"mode":"line","rx":46,"ry":8.1600000000000001,"x":100,"y":24.599999999999998},{"alpha":0.34999999999999998,"color":[0.25,0.88,1],"kind":"ellipse","lineWidth":1,"mode":"line","rx":62,"ry":10.799999999999999,"x":100,"y":24.599999999999998},{"alpha":0.62,"color":[0.25,0.88,1],"kind":"line","lineWidth":2,"points":[14.000000000000002,26.640000000000001,78,26.640000000000001]},{"alpha":0.62,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"line","lineWidth":2,"points":[122,26.640000000000001,186,26.640000000000001]},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":32,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":51.600000000000001,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":71.200000000000003,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":90.800000000000011,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":110.40000000000001,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":130,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":149.60000000000002,"y":24.240000000000002},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":3.3599999999999999,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":13.200000000000001,"x":169.20000000000002,"y":24.240000000000002},{"alpha":1,"color":[0.025000000000000001,0.16,0.17000000000000001],"kind":"polygon","mode":"fill","points":[49,28.799999999999997,151,28.799999999999997,184,105.59999999999999,16,105.59999999999999]},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":520,"ry":256,"x":100,"y":67.199999999999989},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":390,"ry":192,"x":100,"y":67.199999999999989},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":260,"ry":128,"x":100,"y":67.199999999999989},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":130,"ry":64,"x":100,"y":67.199999999999989},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[60.483496854181659,28.799999999999997,57.713467466999298,37.119999999999997,41.850000000000001,45.439999999999998,45.425000000000004,37.119999999999997,49,28.799999999999997,49,28.799999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[83.450490562544971,28.799999999999997,82.290402400997877,37.119999999999997,68.036876159633849,45.439999999999998,57.71346746699929,37.119999999999997,60.483496854181652,28.799999999999997,71.966993708363304,28.799999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[106.41748427090828,28.799999999999997,106.86733733499646,37.119999999999997,94.223752319267703,45.439999999999998,82.290402400997863,37.119999999999997,83.450490562544971,28.799999999999997,94.933987416726623,28.799999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[129.38447797927159,28.799999999999997,131.44427226899504,37.119999999999997,120.41062847890157,45.439999999999998,106.86733733499646,37.119999999999997,106.41748427090829,28.799999999999997,117.90098112508994,28.799999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[151,28.799999999999997,154.57499999999999,37.119999999999997,146.59750463853541,45.439999999999998,131.44427226899504,37.119999999999997,129.38447797927159,28.799999999999997,140.86797483345325,28.799999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[151,28.799999999999997,154.57499999999999,37.119999999999997,158.15000000000001,45.439999999999998,154.57499999999999,37.119999999999997,151,28.799999999999997,151,28.799999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[68.036876159633849,45.439999999999998,64.106758610904393,62.079999999999998,46.633349918269829,70.399999999999991,34.700000000000003,62.079999999999998,41.850000000000001,45.439999999999998,57.71346746699929,37.119999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[94.223752319267717,45.439999999999998,93.513517221808797,62.079999999999998,77.650049754809515,70.399999999999991,64.106758610904393,62.079999999999998,68.036876159633863,45.439999999999998,82.290402400997877,37.119999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[120.41062847890157,45.439999999999998,122.9202758327132,62.079999999999998,108.66674959134917,70.399999999999991,93.513517221808797,62.079999999999998,94.223752319267703,45.439999999999998,106.86733733499646,37.119999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[146.59750463853541,45.439999999999998,152.32703444361758,62.079999999999998,139.68344942788883,70.399999999999991,122.9202758327132,62.079999999999998,120.41062847890157,45.439999999999998,131.44427226899504,37.119999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[158.15000000000001,45.439999999999998,165.30000000000001,62.079999999999998,168.875,70.399999999999991,152.32703444361761,62.079999999999998,146.59750463853544,45.439999999999998,154.57499999999999,37.119999999999997]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[46.633349918269843,70.399999999999991,41.093291143905113,87.039999999999992,20.399999999999991,95.359999999999999,23.975000000000009,87.039999999999992,31.125,70.399999999999991,34.700000000000003,62.079999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[77.650049754809515,70.399999999999991,75.329873431715328,87.039999999999992,56.246523513445482,95.359999999999999,41.093291143905098,87.039999999999992,46.633349918269829,70.399999999999991,64.106758610904393,62.079999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[108.66674959134917,70.399999999999991,109.56645571952554,87.039999999999992,92.093047026890957,95.359999999999999,75.329873431715313,87.039999999999992,77.650049754809515,70.399999999999991,93.513517221808797,62.079999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[139.68344942788883,70.399999999999991,143.80303800733574,87.039999999999992,127.93957054033646,95.359999999999999,109.56645571952554,87.039999999999992,108.66674959134919,70.399999999999991,122.9202758327132,62.079999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[168.875,70.399999999999991,176.02499999999998,87.039999999999992,163.78609405378194,95.359999999999999,143.80303800733574,87.039999999999992,139.68344942788883,70.399999999999991,152.32703444361758,62.079999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[168.875,70.399999999999991,176.02499999999998,87.039999999999992,179.60000000000002,95.359999999999999,176.02499999999998,87.039999999999992,168.875,70.399999999999991,165.30000000000001,62.079999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[56.246523513445482,95.359999999999999,53.827989637304277,105.59999999999999,34.913994818652128,105.59999999999999,16,105.59999999999999,20.399999999999991,95.359999999999999,41.093291143905098,87.039999999999992]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[92.093047026890972,95.359999999999999,91.655979274608569,105.59999999999999,72.741984455956427,105.59999999999999,53.827989637304277,105.59999999999999,56.246523513445489,95.359999999999999,75.329873431715328,87.039999999999992]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[127.93957054033646,95.359999999999999,129.48396891191285,105.59999999999999,110.5699740932607,105.59999999999999,91.655979274608555,105.59999999999999,92.093047026890957,95.359999999999999,109.56645571952554,87.039999999999992]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[163.78609405378194,95.359999999999999,167.31195854921711,105.59999999999999,148.39796373056498,105.59999999999999,129.48396891191285,105.59999999999999,127.93957054033646,95.359999999999999,143.80303800733574,87.039999999999992]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":2,"mode":"line","points":[179.60000000000002,95.359999999999999,184,105.59999999999999,184,105.59999999999999,167.31195854921714,105.59999999999999,163.78609405378194,95.359999999999999,176.02499999999998,87.039999999999992]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"line","lineWidth":2,"points":[100,28.799999999999997,100,105.59999999999999]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[147.25,67.199999999999989,148.83652704548354,74.979438359478479,148.73125785239722,82.522502420989952,146.75454648681239,89.599999999999994,142.83074217329147,95.996884913956961,137.00685679360873,101.5187910517302,129.45984615799765,105.99793808954284,120.49123329240092,109.29822941120869,110.50923711041915,111.31938733494691,100,111.99999999999999,89.49076288958085,111.31938733494691,79.508766707599094,109.29822941120869,70.540153842002354,105.99793808954286,62.993143206391267,101.5187910517302,57.169257826708545,95.996884913956961,53.245453513187613,89.599999999999994,51.268742147602765,82.522502420989966,51.163472954516472,74.979438359478507,52.75,67.200000000000003,55.772194385829863,59.420561640521512,59.930305188128884,51.87749757901004,64.915145829182933,44.799999999999997,70.439542298548034,38.403115086043037,76.263427678230755,32.881208948269787,82.209846157997646,28.402061910457157,88.170329748125184,25.101770588791311,94.099484320894234,23.080612665053074,100,22.399999999999999,105.90051567910575,23.080612665053074,111.8296702518748,25.101770588791304,117.79015384200235,28.402061910457149,123.73657232176923,32.88120894826978,129.56045770145195,38.403115086043023,135.08485417081704,44.799999999999976,140.0696948118711,51.877497579010011,144.22780561417014,59.420561640521512,147.25,67.199999999999989]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"circle","mode":"fill","r":3,"x":100,"y":67.199999999999989},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[40.75,48,52.599999999999994,48,39.400000000000006,86.399999999999991,24.25,86.399999999999991]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[147.40000000000001,48,159.25,48,175.75,86.399999999999991,160.59999999999999,86.399999999999991]},{"alpha":0.90000000000000002,"color":[0.25,0.88,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[49,28.799999999999997,151,28.799999999999997,184,105.59999999999999,16,105.59999999999999]},{"alpha":0.29999999999999999,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[34.700000000000003,62.079999999999998,33.393999999999991,62.079999999999998,33.393999999999991,56.333599999999997,34.700000000000003,51.631999999999998]},{"alpha":0.29999999999999999,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[30.300000000000011,72.319999999999993,28.906000000000006,72.319999999999993,28.906000000000006,66.186399999999992,30.300000000000011,61.167999999999992]},{"alpha":0.29999999999999999,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[33.393999999999991,62.079999999999998,28.906000000000006,72.319999999999993,28.906000000000006,66.186399999999992,33.393999999999991,56.333599999999997]},{"alpha":0.22,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[34.700000000000003,51.631999999999998,30.300000000000011,61.167999999999992,28.906000000000006,66.186399999999992,33.393999999999991,56.333599999999997]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[34.700000000000003,62.079999999999998,34.700000000000003,51.631999999999998]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[30.300000000000011,72.319999999999993,30.300000000000011,61.167999999999992]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[34.700000000000003,51.631999999999998,30.300000000000011,61.167999999999992]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[33.393999999999991,62.079999999999998,33.393999999999991,56.333599999999997]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[28.906000000000006,72.319999999999993,28.906000000000006,66.186399999999992]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[33.393999999999991,56.333599999999997,28.906000000000006,66.186399999999992]},{"alpha":0.29999999999999999,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[165.30000000000001,62.079999999999998,166.60599999999999,62.079999999999998,166.60599999999999,56.333599999999997,165.30000000000001,51.631999999999998]},{"alpha":0.29999999999999999,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[169.69999999999999,72.319999999999993,171.09399999999999,72.319999999999993,171.09399999999999,66.186399999999992,169.69999999999999,61.167999999999992]},{"alpha":0.29999999999999999,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[166.60599999999999,62.079999999999998,171.09399999999999,72.319999999999993,171.09399999999999,66.186399999999992,166.60599999999999,56.333599999999997]},{"alpha":0.22,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[165.30000000000001,51.631999999999998,169.69999999999999,61.167999999999992,171.09399999999999,66.186399999999992,166.60599999999999,56.333599999999997]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[165.30000000000001,62.079999999999998,165.30000000000001,51.631999999999998]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[169.69999999999999,72.319999999999993,169.69999999999999,61.167999999999992]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[165.30000000000001,51.631999999999998,169.69999999999999,61.167999999999992]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[166.60599999999999,62.079999999999998,166.60599999999999,56.333599999999997]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[171.09399999999999,72.319999999999993,171.09399999999999,66.186399999999992]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[166.60599999999999,56.333599999999997,171.09399999999999,66.186399999999992]},{"alpha":0.57999999999999996,"color":[0.25,0.88,1],"kind":"line","lineWidth":2,"points":[49,28.799999999999997,39,3.7999999999999972]},{"alpha":0.57999999999999996,"color":[0.25,0.88,1],"kind":"line","lineWidth":2,"points":[16,105.59999999999999,4,121.59999999999999]},{"alpha":0.57999999999999996,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"line","lineWidth":2,"points":[151,28.799999999999997,161,3.7999999999999972]},{"alpha":0.57999999999999996,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"line","lineWidth":2,"points":[184,105.59999999999999,196,121.59999999999999]},{"color":[1,0.55000000000000004,0.25],"kind":"player","opts":{"controlled":false,"dive":0.34999999999999998,"dive_dir":[1,0],"facing":[0,1],"holding":true,"is_keeper":true,"pose_id":"keeper_ready_tall","pose_priority":2,"pose_source":"keeper","species_color":[0.80000000000000004,0.80000000000000004,1],"species_shape":"round","team":"away"},"r":1.51065,"sx":55.240000000000002,"sy":40.319999999999993},{"color":[0.34999999999999998,0.75,1],"kind":"player","opts":{"aerial":0.59999999999999998,"aerial_jump":0.22,"aerial_outcome":"clean","aerial_style":"header","controlled":false,"dive_dir":[],"facing":[-1,0],"is_keeper":false,"pose_id":"aerial_header","pose_priority":3,"pose_source":"combat","species_color":[0.90000000000000002,0.5,0.20000000000000001],"species_shape":"angular","team":"home"},"r":1.653125,"sx":130.41749999999999,"sy":63.999999999999993},{"alpha":0.2857142857142857,"color":[0,0,0],"kind":"ellipse","mode":"fill","rx":3.9514285714285711,"ry":1.9757142857142855,"x":100,"y":71.039999999999992},{"alpha":1,"color":[1,0.94999999999999996,0.69999999999999996],"kind":"circle","mode":"fill","r":3.4575,"x":100,"y":65.507999999999996},{"color":[0.34999999999999998,0.75,1],"kind":"player","opts":{"controlled":true,"dashing":true,"dive_dir":[],"facing":[1,0],"is_keeper":false,"pose_id":"tackle","pose_priority":1,"pose_source":"combat","species_color":[1,1,1],"species_shape":"round","team":"home","windup":0.41999999999999998},"r":1.8799999999999999,"sx":106.01600000000001,"sy":85.11999999999999},{"alpha":0.51000000000000001,"color":[1,0.84999999999999998,0.34999999999999998],"kind":"circle","lineWidth":1.0743750000000001,"mode":"line","r":5.157,"x":119.33875,"y":76.799999999999997},{"alpha":0.40000000000000002,"color":[1,0.84999999999999998,0.34999999999999998],"kind":"circle","lineWidth":1.0743750000000001,"mode":"line","r":5.0137499999999999,"x":119.33875,"y":76.799999999999997},{"alpha":0.55249999999999999,"color":[0.34999999999999998,0.75,1],"kind":"circle","lineWidth":1,"mode":"line","r":4.2981249999999998,"x":130.41749999999999,"y":63.999999999999993},{"alpha":0.29250000000000004,"color":[0.34999999999999998,0.75,1],"kind":"circle","lineWidth":1,"mode":"line","r":6.8770000000000007,"x":130.41749999999999,"y":63.999999999999993},{"alpha":0.55000000000000004,"color":[0,0,0],"h":3.008,"kind":"rect","mode":"fill","w":25.568000000000001,"x":93.231999999999999,"y":94.143999999999991},{"alpha":0.94999999999999996,"color":[1,0.71999999999999997,0.29999999999999999],"h":3.008,"kind":"rect","mode":"fill","w":14.062400000000002,"x":93.231999999999999,"y":94.143999999999991},{"alpha":0.34999999999999998,"color":[1,1,1],"h":3.008,"kind":"rect","lineWidth":1,"mode":"line","w":25.568000000000001,"x":93.231999999999999,"y":94.143999999999991},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[98.345600000000005,94.143999999999991,98.345600000000005,97.151999999999987]},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[103.4592,94.143999999999991,103.4592,97.151999999999987]},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[108.5728,94.143999999999991,108.5728,97.151999999999987]},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[113.68639999999999,94.143999999999991,113.68639999999999,97.151999999999987]},{"align":"center","alpha":0.94999999999999996,"color":[1,0.71999999999999997,0.29999999999999999],"kind":"text","text":"SHOT","w":25.568000000000001,"x":93.231999999999999,"y":98.151999999999987}]`;

/** One captured draw call, normalized -- see capture_pitch_reference.lua. */
interface LuaGeomRecord {
  readonly kind: string;
  readonly mode?: string;
  readonly x?: number;
  readonly y?: number;
  readonly r?: number;
  readonly rx?: number;
  readonly ry?: number;
  readonly w?: number;
  readonly h?: number;
  readonly points?: readonly number[];
  readonly text?: string;
  readonly align?: string;
  readonly color: readonly number[];
  readonly alpha: number;
  readonly blend?: string;
  readonly lineWidth?: number;
}

/**
 * One `player_renderer.draw(sx, sy, r, color, v, opts)` call from the pinned
 * reference capture, normalized. The reference implementation's billboard
 * renderer is what this fixture is a recording of -- so the record shape is
 * unchanged; what the TypeScript side compares against it is now
 * `playerAnchors` (see #415).
 */
interface LuaPlayerRecord {
  readonly kind: "player";
  readonly sx: number;
  readonly sy: number;
  readonly r: number;
  readonly color: readonly number[];
  readonly opts: {
    readonly facing: readonly number[];
    readonly is_keeper: boolean;
    readonly controlled: boolean;
    readonly dashing?: boolean;
    readonly dive?: number;
    // [] when the reference implementation's always-constructed {x=nil,y=nil}
    // had no components -- see this describe block's own test for the
    // divergence this causes.
    readonly dive_dir?: readonly number[];
    readonly holding?: boolean;
    readonly grab?: number;
    readonly throw?: number;
    readonly windup?: number;
    readonly aerial?: number;
    readonly aerial_style?: string;
    readonly aerial_outcome?: string;
    readonly aerial_jump?: number;
    readonly species_shape?: string;
    readonly species_color?: readonly number[];
    readonly team?: string;
    readonly pose_id?: string;
    readonly pose_priority?: number;
    readonly pose_source?: string;
  };
}

type LuaRecord = LuaGeomRecord | LuaPlayerRecord;

function isLuaPlayerRecord(r: LuaRecord): r is LuaPlayerRecord {
  return r.kind === "player";
}

function isLuaGeomRecord(r: LuaRecord): r is LuaGeomRecord {
  return r.kind !== "player";
}

const LUA_RECORDS = JSON.parse(LUA_REFERENCE_JSON) as readonly LuaRecord[];
const LUA_GEOM_RECORDS: readonly LuaGeomRecord[] = LUA_RECORDS.filter(isLuaGeomRecord);
const LUA_PLAYER_RECORDS: readonly LuaPlayerRecord[] = LUA_RECORDS.filter(isLuaPlayerRecord);

// A deliberately SMALL field -- see tools/render_reference/README.md's
// "How the fixture was chosen". MUST stay byte-for-byte identical to
// capture_pitch_reference.lua's `frame` literal (the source of the pinned
// capture below); the comparison only means something if both sides
// consumed the same input.
function luaDifferentialFrame(): RenderFrame {
  return {
    field: {
      w: 200,
      h: 120,
      penalty_box_depth: 20,
      penalty_box_h: 60,
      crossbar_h: 16,
      goal_home: { x: -2, y: 52, w: 2, h: 16 },
      goal_away: { x: 200, y: 52, w: 2, h: 16 },
    },
    roster: {
      radius: [2.5, 2.7, 2.5],
      teams: ["home", "away", "home"],
      is_keeper: [false, true, false],
      species_shape: ["round", "round", "angular"],
      species_color: [
        [1, 1, 1],
        [0.8, 0.8, 1],
        [0.9, 0.5, 0.2],
      ],
      ids: ["home-1", "away-kp", "home-2"],
      // #447, AND THE ONE PLACE THE REFERENCE DIVERGENCE SURFACES IN A TEST.
      // The reference implementation's roster payload has no presentation or
      // loadout columns at all -- that divergence is deliberate: #447's whole
      // point is presentation/loadout data that only exists here, not in the
      // reference implementation -- so the pinned capture this frame is
      // compared against could not have consumed these two fields, and the
      // assertion loop below does not compare them. Every field it
      // DOES compare is unaffected by what is written here.
      //
      // Real ids rather than empty strings, because an empty
      // `presentation_id` is now a hard error at three separate layers
      // (`encode_roster`, `decodeRoster`, `variantFor`), precisely so it can
      // never resolve to the preview default's sword and shield. A fixture
      // relying on the old "empty means unwired" behaviour would be encoding
      // the defect into the differential.
      presentation_ids: ["medieval_rook_emberguard", "scifi_axi", "toy_tock"],
      loadout_ids: ["loadout_tournament_sword", undefined, "loadout_pulse_blaster"],
    },
    players: {
      count: 3,
      x: [108, 20, 146],
      y: [88, 18, 55],
      facing_x: [1, 0, -1],
      facing_y: [0, 1, 0],
      controlled: [true, false, false],
      dashing: [true, undefined, undefined],
      dive: [undefined, 0.35, undefined],
      dive_dir_x: [undefined, 1, undefined],
      dive_dir_y: [undefined, 0, undefined],
      holding: [undefined, true, undefined],
      grab: [undefined, undefined, undefined],
      throw: [undefined, undefined, undefined],
      windup: [0.42, undefined, undefined],
      aerial: [undefined, undefined, 0.6],
      aerial_style: [undefined, undefined, "header"],
      aerial_outcome: [undefined, undefined, "clean"],
      aerial_jump: [undefined, undefined, 0.22],
      pose_id: ["tackle", "keeper_ready_tall", "aerial_header"],
      pose_priority: [1, 2, 3],
      pose_source: ["combat", "keeper", "combat"],
    },
    ball: { x: 100, y: 66, z: 4, visible: true, landing_x: 127, landing_y: 75 },
    // Deliberately roster-slot (one-based, matching the wire -- ARCHITECTURE.md
    // §4 rule 3 and frame_buffer.ts's own decode comment) rather than zero-based array
    // index: player 1 (roster slot 1) sits at TS array index 0; player 3
    // (roster slot 3) sits at TS array index 2. See this suite's "KNOWN BUG"
    // block below for what asserting these against the pinned reference exposed.
    control: { pass_target: 3, charge_kind: "shot", charge: 0.55, controlled: 1 },
  };
}

// The viewport is the FIELD's own size (#414) -- see
// capture_pitch_reference.lua's comment above its own `vp` for the full
// reasoning. Short version: camera.ts's `projectFixed` now carries one
// uniform world-to-pixel factor into positions AND sizes, while the
// reference implementation carried it into positions only; the two
// therefore agree exactly when that factor is 1, i.e. at `vp == field`.
// Capturing there keeps this a plain equality differential over all 101
// records instead of one that has to characterise a divergence.
//
// This fixture's field is a synthetic 200x120 (shrunk to keep the hex-tile
// count embeddable -- pre-existing, unrelated to #414), which is 5:3, while
// the old capture viewport was 16:9. So away from `vp == field` the divergence
// here would be TWO-part -- positions and sizes both -- rather than the
// single-constant `scale` divergence camera.spec.ts characterises for its
// same-aspect 1280x720 rows. That, not "this is the shipping configuration",
// is why the capture moved: the reference implementation shipped at 960x540,
// never at 200x120.
//
// The switch does NOT weaken the fixture's coverage of entity SIZES: the
// reference implementation's `scale` had no viewport term at all, so every
// size term below (player `r`, goal frame heights, ball radius, meter
// dimensions) is bit-identical to the previous 1280x720 capture. Only
// screen POSITIONS moved.
//
// WHAT THIS DIFFERENTIAL DOES NOT GUARD. It is not the regression test for
// #414 and never could have been: at `vp == field` the fit factor is 1 and the
// fixed formula is byte-identical to the buggy one, so reverting camera.ts to
// its pre-fix form leaves every record in this block passing (verified). That
// was equally true before this PR, when `projectFixed` was a line-for-line
// mirror of the reference implementation. Viewport-safety regression
// coverage lives in two other places, both viewport-varying by construction:
//
//   * `camera.spec.ts`'s kept 1280x720 differential rows -- reverting
//     camera.ts fails them, off by exactly the predicted fit factor
//     (0.51*(1/3) = 0.17 and 1.02*(1/3) = 0.34 on `scale`);
//   * this file's "pitch entity sizes stay in proportion to the pitch at any
//     viewport" block below, which is reference-independent and asserts the
//     invariant across five viewports including non-16:9 ones.
const LUA_DIFFERENTIAL_VIEWPORT: PitchViewport = { w: 200, h: 120 };
const LUA_DIFFERENTIAL_OPTS: PitchDrawOptions = {
  home_color: [0.35, 0.75, 1.0],
  away_color: [1.0, 0.55, 0.25],
};

function round(n: number): number {
  return Math.round(n * 1e6) / 1e6;
}

function roundArr(a: readonly number[]): number[] {
  return a.map(round);
}

function numField(obj: Record<string, unknown>, key: string): number | undefined {
  const v = obj[key];
  return typeof v === "number" ? v : undefined;
}

function strField(obj: Record<string, unknown>, key: string): string | undefined {
  const v = obj[key];
  return typeof v === "string" ? v : undefined;
}

function numArrField(obj: Record<string, unknown>, key: string): readonly number[] | undefined {
  const v = obj[key];
  return Array.isArray(v) ? (v as number[]) : undefined;
}

// Normalizes EITHER a captured reference geometry record OR a real `DrawCommand`
// (draw2d.ts) into the same plain, rounded shape, so `toEqual` gives a
// readable diff regardless of which side produced it. `value` is read
// defensively field-by-field (never `as` cast to a specific command variant)
// because the two input shapes are only STRUCTURALLY compatible, not the
// same TypeScript type.
function normalizeGeom(value: unknown): Record<string, unknown> {
  const obj = value as Record<string, unknown>;
  const out: Record<string, unknown> = { kind: strField(obj, "kind") };
  const mode = strField(obj, "mode");
  if (mode !== undefined) out.mode = mode;
  for (const key of ["x", "y", "r", "rx", "ry", "w", "h"]) {
    const v = numField(obj, key);
    if (v !== undefined) out[key] = round(v);
  }
  const points = numArrField(obj, "points");
  if (points !== undefined) out.points = roundArr(points);
  const text = strField(obj, "text");
  if (text !== undefined) out.text = text;
  const align = strField(obj, "align");
  if (align !== undefined) out.align = align;
  const color = numArrField(obj, "color");
  out.color = color !== undefined ? roundArr(color) : undefined;
  const alpha = numField(obj, "alpha");
  out.alpha = round(alpha ?? 1);
  const blend = strField(obj, "blend");
  if (blend !== undefined) out.blend = blend;
  const lineWidth = numField(obj, "lineWidth");
  if (lineWidth !== undefined) out.lineWidth = round(lineWidth);
  return out;
}

// A record's normalized-then-stringified form, used to compare two draw
// lists as MULTISETS (content, not sequence). Only used for the "before
// items" section below -- see that test's own comment for why order-
// insensitivity there is a deliberate, documented choice and not a
// weakening of the gate: pitch.ts's own `drawPitchBeforeItems` comment
// explains it draws the arena frame chevrons after the goals rather than
// before (matching the reference implementation's own draw order), because they are static,
// non-overlapping, screen-space content where painter's-algorithm order
// between the two groups cannot be visible -- a real, deliberate,
// already-justified divergence this harness should not flag as a defect.
// A genuine content bug (wrong color, wrong position, a missing/extra
// shape) still fails a multiset comparison exactly as it would an ordered
// one; only a PURE reordering with zero content difference is tolerated.
function sortKey(value: unknown): string {
  return JSON.stringify(normalizeGeom(value));
}

function sortedNormalized(list: readonly unknown[]): Record<string, unknown>[] {
  return [...list].map(normalizeGeom).sort((a, b) => sortKey(a).localeCompare(sortKey(b)));
}

const RETICLE_COLOR = [1, 0.85, 0.35] as const;
const HEX_FLOOR_COLOR = [0.16, 0.5, 0.6] as const;

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

interface RecordLike {
  readonly kind: string;
  readonly mode?: string;
  readonly color: readonly number[];
}

function isReticleRing(r: RecordLike): boolean {
  return r.kind === "circle" && r.mode === "line" && colorCloseTo(r.color, RETICLE_COLOR);
}

function isHexFloorTile(r: RecordLike): boolean {
  return r.kind === "polygon" && r.mode === "line" && colorCloseTo(r.color, HEX_FLOOR_COLOR);
}

/** Index of the first landing-reticle ring: everything before it is the
 * static arena/pitch scene + the loose ball; the reticle marks where the
 * per-frame overlay (reticle, pass-target preview, charge meter) begins. */
function overlayStart(records: readonly RecordLike[]): number {
  const idx = records.findIndex(isReticleRing);
  if (idx < 0) {
    throw new Error(
      "expected to find the landing-reticle ring (the overlay section's first entry) in this fixture's output",
    );
  }
  return idx;
}

describe("pitch.pitchDrawCommands differential against the real Lua game.render.pitch", () => {
  // Since #415 `pitchDrawCommands` emits no player content at all, so this is
  // a plain call. It used to mock `playerDrawCommands` to `[]` to get the same
  // list, which is why every comparison below is unaffected by the removal.
  function drawWithoutPlayers(): DrawCommand[] {
    return pitchDrawCommands(
      luaDifferentialFrame(),
      LUA_DIFFERENTIAL_VIEWPORT,
      LUA_DIFFERENTIAL_OPTS,
      0,
    );
  }

  it("matches the Lua capture's static arena/pitch scene and the loose ball -- backdrop, floor, markings, goal nets/frames, outline, chevrons, ball shadow+lift (content-equal; see sortedNormalized's comment for why this is order-insensitive). The hex floor's line width is excluded here -- see the dedicated KNOWN BUG test below.", () => {
    const commands = drawWithoutPlayers();
    const luaBefore = LUA_GEOM_RECORDS.slice(0, overlayStart(LUA_GEOM_RECORDS)).filter(
      (r) => !isHexFloorTile(r),
    );
    const tsBefore = commands.slice(0, overlayStart(commands)).filter((r) => !isHexFloorTile(r));

    expect(sortedNormalized(tsBefore)).toEqual(sortedNormalized(luaBefore));
  });

  it("matches the Lua capture's landing reticle exactly (unaffected by the KNOWN BUG below -- landing_x/landing_y are plain ball-payload fields, not a roster-slot index)", () => {
    const commands = drawWithoutPlayers();
    const luaReticle = LUA_GEOM_RECORDS.filter(isReticleRing);
    const tsReticle = commands.filter(isReticleRing);

    expect(tsReticle.map(normalizeGeom)).toEqual(luaReticle.map(normalizeGeom));
    expect(tsReticle.length).toBeGreaterThan(0);
  });

  it("draws the same NUMBER of overlay commands as the Lua capture (pass-target preview + charge meter), plus the persistent controlled-player marker the Lua reference never had -- their POSITIONS are pinned separately by the KNOWN BUG tests further down, since both currently diverge from Lua", () => {
    const commands = drawWithoutPlayers();
    const luaOverlayCount = LUA_GEOM_RECORDS.length - overlayStart(LUA_GEOM_RECORDS);
    const tsOverlayCount = commands.length - overlayStart(commands);

    // +2: the controlled-player double ring (`drawControlledMarker`), a
    // genuinely new feature with no Lua-reference equivalent to diverge
    // from -- `luaDifferentialFrame()`'s `control.controlled: 1` is a valid
    // one-based slot (roster index 0, "home-1"), so the marker draws on
    // every call against this fixture.
    expect(tsOverlayCount).toBe(luaOverlayCount + 2);
  });

  // KNOWN BUG, found by this differential: the original arena renderer's
  // backdrop draw ends with a call setting line width to `max(2,
  // viewport.h / 180)` for its ribbon markers (720 / 180 = 4 for this
  // fixture's viewport) and never resets it. The original hex-floor draw
  // never set its own line width, so the graphics context's stateful line
  // width leaked that 4px width into every hex tile line -- a real,
  // rendered, reproducible feature of the original renderer (confirmed by
  // this capture), not an authoring accident this renderer should "correct":
  // per ARCHITECTURE.md §3 rule 7 and this task's own brief, "reproduce
  // behaviour, not intent." `pitch.ts`'s `drawHexFloor` has no equivalent
  // ambient state to inherit from (`arena.ts`'s backdrop is a separate, pure
  // command-producing function with no shared mutable graphics context), so
  // it draws hex lines at the `dl.polygon` default width (1px) -- visibly
  // thinner/fainter grid lines than the original ever actually rendered.
  // Reproducing this needs `Math.max(2, vp.h / 180)` threaded into
  // `drawHexFloor`'s own `dl.polygon(..., { lineWidth: ... })` call --
  // outside this task's file ownership (`pitch.ts`), so pinned here instead.
  it.fails(
    "hex floor tiles render at the Lua original's actual (leaked, viewport-dependent) line width, not the DrawList default",
    () => {
      const commands = drawWithoutPlayers();
      const luaHex = LUA_GEOM_RECORDS.filter(isHexFloorTile);
      const tsHex = commands.filter(isHexFloorTile);

      expect(tsHex.length).toBeGreaterThan(0);
      expect(tsHex.map(normalizeGeom)).toEqual(luaHex.map(normalizeGeom));
    },
  );

  it("derives the exact same per-player anchor (sx, sy, r) and PlayerRenderOptions payload the Lua original computes, in the same depth-sorted order", () => {
    const captured = playerAnchors(
      luaDifferentialFrame(),
      LUA_DIFFERENTIAL_VIEWPORT,
      LUA_DIFFERENTIAL_OPTS,
    );

    expect(captured).toHaveLength(LUA_PLAYER_RECORDS.length);
    for (const [i, luaPlayer] of LUA_PLAYER_RECORDS.entries()) {
      const ts = captured[i];
      if (ts === undefined) {
        throw new Error(`missing player anchor at depth-sorted index ${i}`);
      }
      expect(round(ts.sx)).toBeCloseTo(round(luaPlayer.sx), 4);
      expect(round(ts.sy)).toBeCloseTo(round(luaPlayer.sy), 4);
      expect(round(ts.r)).toBeCloseTo(round(luaPlayer.r), 4);

      const o = ts.options;
      expect(o.facing !== undefined ? [o.facing.x, o.facing.y] : undefined).toEqual(
        luaPlayer.opts.facing,
      );
      expect(o.is_keeper).toBe(luaPlayer.opts.is_keeper);
      expect(o.controlled).toBe(luaPlayer.opts.controlled);
      expect(o.dashing).toBe(luaPlayer.opts.dashing);
      expect(o.dive).toBe(luaPlayer.opts.dive);
      expect(o.holding).toBe(luaPlayer.opts.holding);
      expect(o.grab).toBe(luaPlayer.opts.grab);
      expect(o.throw).toBe(luaPlayer.opts.throw);
      expect(o.windup).toBe(luaPlayer.opts.windup);
      expect(o.aerial).toBe(luaPlayer.opts.aerial);
      expect(o.aerial_style).toBe(luaPlayer.opts.aerial_style);
      expect(o.aerial_outcome).toBe(luaPlayer.opts.aerial_outcome);
      expect(o.aerial_jump).toBe(luaPlayer.opts.aerial_jump);
      expect(o.species_shape).toBe(luaPlayer.opts.species_shape);
      expect(o.species_color !== undefined ? [...o.species_color] : undefined).toEqual(
        luaPlayer.opts.species_color,
      );
      expect(o.team).toBe(luaPlayer.opts.team);
      expect(o.pose?.id).toBe(luaPlayer.opts.pose_id);
      expect(o.pose?.priority).toBe(luaPlayer.opts.pose_priority);
      expect(o.pose?.source).toBe(luaPlayer.opts.pose_source);

      // KNOWN, NARROW DIVERGENCE (see tools/render_reference/README.md):
      // the reference implementation ALWAYS constructs
      // `player_opts.dive_dir = { x = players.dive_dir_x[index], y =
      // players.dive_dir_y[index] }` (its PlayerRenderOptions block), even
      // when neither component is set -- so a non-diving player's reference
      // record carries dive_dir as a table with two nil fields (this
      // harness's JSON encodes that as `[]`, since a table with no non-nil
      // entries is indistinguishable from an empty array). pitch.ts's
      // playerOptions() only includes `dive_dir` at all when BOTH
      // dive_dir_x/y are defined, omitting the key entirely otherwise. Both
      // convey "no dive direction data", but via a different representation
      // (present-with-nils vs absent-key) -- documented here, not asserted
      // equal, since which one is "right" depends on whether a downstream
      // consumer (player_renderer_3d.ts/rig3d, outside this task's scope)
      // branches on the KEY'S PRESENCE or on the component VALUES.
      if (luaPlayer.opts.dive_dir !== undefined && luaPlayer.opts.dive_dir.length === 2) {
        expect(o.dive_dir).toBeDefined();
        expect(o.dive_dir !== undefined ? [o.dive_dir.x, o.dive_dir.y] : undefined).toEqual(
          luaPlayer.opts.dive_dir,
        );
      }
    }
  });
});

// KNOWN BUG, found by the differential above: `frame.control.controlled` and
// `frame.control.pass_target` are ONE-BASED roster slots on the wire --
// `frame_buffer.ts`'s own decoder comment ("Roster slot, one-based ...
// recover it as `roster.ids[hud.controlled - 1]`"), the Rust producer
// (`crates/gc-render/src/frame.rs`'s `RenderFrameControl.controlled:
// state.controlled`, copied verbatim from gc-sim's own one-based
// `MatchState.controlled`), and `packages/screens/src/match.ts`'s OWN
// handling of the identical field two lines away ("hud.controlled is
// one-based ... `const fallbackControlled = (frame.hud.controlled ?? 1) -
// 1;`") all agree on this. `pitch.ts`'s `drawPitchAfterItems` reads BOTH
// fields straight into `players.x[...]`/`players.y[...]` -- a ZERO-based
// array -- with no `-1` anywhere, for the charge meter and the pass-target
// preview. The reference implementation has no such bug: its own player
// position array is also one-based, indexed by the same one-based value,
// so the two stay aligned there.
//
// FIXED. `drawPitchAfterItems` now subtracts 1 from both fields before
// indexing the zero-based SoA arrays. These were pinned as `it.fails` by the
// differential that found the bug, and flipped to real assertions once the
// fix landed -- the transition from expected-fail to pass IS the proof.
describe("pitch.ts converts frame.control.controlled/pass_target from one-based to zero-based", () => {
  it("charge meter renders under the CORRECT (one-based-adjusted) controlled player, matching the Lua reference", () => {
    const commands = pitchDrawCommands(
      luaDifferentialFrame(),
      LUA_DIFFERENTIAL_VIEWPORT,
      LUA_DIFFERENTIAL_OPTS,
      0,
    );
    // The reference's charge-meter bar fill (color = home_color, the "shot"
    // charge color multiplied through -- see the captured reference) sits at
    // x=93.232 (home-1, roster slot 1 == TS array index 0). The current
    // (buggy) pitch.ts instead reads players.x[1] directly -- TS index 1,
    // "away-kp" -- and renders the bar under the WRONG player.
    // (Was 665.7184 when this fixture was captured at a 1280x720 viewport;
    // the capture now runs at the field's own size, see
    // LUA_DIFFERENTIAL_VIEWPORT. Same reference, same bug, same player --
    // only the pixel scale of the whole capture changed.)
    const expectedX = 93.232;
    const meterFill = commands.find(
      (c) =>
        c.kind === "rect" &&
        c.mode === "fill" &&
        Math.abs(c.color[0] - 1) < 1e-6 &&
        Math.abs(c.color[1] - 0.72) < 1e-6 &&
        Math.abs(c.color[2] - 0.3) < 1e-6,
    );
    expect(meterFill).toBeDefined();
    expect(meterFill?.kind === "rect" ? round(meterFill.x) : undefined).toBeCloseTo(expectedX, 2);
  });

  it("pass-target preview renders at the CORRECT (one-based-adjusted) target player's position, matching the Lua reference", () => {
    const commands = pitchDrawCommands(
      luaDifferentialFrame(),
      LUA_DIFFERENTIAL_VIEWPORT,
      LUA_DIFFERENTIAL_OPTS,
      0,
    );
    // The reference's pass-target rings (home_color, since roster slot 3 /
    // "home-2" is on the home team) sit at (130.4175, 64) -- that player's own projected
    // anchor. The current (buggy) pitch.ts reads players.x[3]/players.y[3],
    // both out of bounds on a 3-element zero-based array, defaulting to
    // world (0, 0) via `?? 0` -- nowhere near the intended player.
    // (Was (834.672, 384) at the fixture's previous 1280x720 viewport; see
    // LUA_DIFFERENTIAL_VIEWPORT for why the capture now runs at field size.)
    const expectedX = 130.4175;
    const expectedY = 64;
    const ring = commands.find(
      (c) =>
        c.kind === "circle" &&
        c.mode === "line" &&
        Math.abs(c.color[0] - 0.35) < 1e-6 &&
        Math.abs(c.color[1] - 0.75) < 1e-6 &&
        Math.abs(c.color[2] - 1) < 1e-6,
    );
    expect(ring).toBeDefined();
    expect(ring?.kind === "circle" ? round(ring.x) : undefined).toBeCloseTo(expectedX, 2);
    expect(ring?.kind === "circle" ? round(ring.y) : undefined).toBeCloseTo(expectedY, 2);
  });
});

// #491's no-latching requirement, at the actual drawing layer rather than
// the payload shape: `pitchDrawCommands` is a pure function of the ONE
// `RenderFrame` it is handed (this file's own top comment: "Tests for
// pitch.ts's pure path"), so nothing here can carry a marker position
// between two independent calls -- there is no per-call cache for
// `control.pass_target` the way `staticSceneCommands` deliberately caches
// arena geometry (that cache is keyed on field/camera/theme, never on
// `pass_target`, so it cannot be the mechanism either). These tests make
// that a runtime assertion, not just a reading of the source: a hand-off
// tween or an animation state machine that held the last shown target
// across frames -- exactly the failure #491 names -- would make the
// "must NOT still be at the old ring position" assertion below fail.
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
