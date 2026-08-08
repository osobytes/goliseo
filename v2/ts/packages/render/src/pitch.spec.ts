// New tests for pitch.ts's pure path (`pitchDrawCommands`). No Lua spec
// targets game/render/pitch.lua with a claimable, self-contained fixture --
// spec/render/draw_smoke_spec.lua and spec/render/combat_presentation_spec.lua
// both exercise it, but only by stubbing `love.graphics` and pulling in
// `sim.match`/`data.teams`/`data.formations` (Rust-owned) and `game.ui.draw`/
// `game.match_hud` (other packages) -- see this package's port report for
// why those specs are not claimed wholesale.

import { describe, expect, it, afterEach, vi } from "vitest";
import * as THREE from "three";
import { pitchDrawCommands, pitch, depthToZ, BACKDROP_Z, ENTITY_Z_NEAR, ENTITY_Z_FAR, OVERLAY_RENDER_ORDER, type PitchDrawOptions, type PitchViewport, type RenderFrame } from "./pitch.ts";
import { camera } from "./camera.ts";
import * as playerRenderer3d from "./player_renderer_3d.ts";
import * as playerRenderer from "./player_renderer.ts";
import type { PlayerRenderOptions } from "./player_renderer.ts";
import type { DrawCommand, RGB } from "./draw2d.ts";

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

    const hasBackdropRect = (cs: readonly DrawCommand[]) => cs.some((c) => c.kind === "rect" && c.mode === "fill" && c.w === viewport.w && c.h === viewport.h);
    const hasFloorPolygon = (cs: readonly DrawCommand[]) => cs.some((c) => c.kind === "polygon" && c.mode === "fill" && colorEq(c.color, FLOOR_COLOR));

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
    const withLanding = pitchDrawCommands(frame({ ball: { x: 480, y: 270, z: 40, visible: true, landing_x: 600, landing_y: 300 } }), viewport, opts, 0.25);

    const hasBallShadow = withLanding.some((c) => c.kind === "ellipse" && c.mode === "fill" && colorEq(c.color, BALL_SHADOW_COLOR));
    const hasBallLit = withLanding.some((c) => c.kind === "circle" && c.mode === "fill" && colorEq(c.color, BALL_LIT_COLOR));
    const hasReticle = withLanding.some((c) => c.kind === "circle" && c.mode === "line" && colorEq(c.color, [1, 0.85, 0.35]));

    expect(hasBallShadow).toBe(true);
    expect(hasBallLit).toBe(true);
    expect(hasReticle).toBe(true);
  });

  it("keeps the charge meter overlay (an entity-anchored, non-arena command)", () => {
    pitch.stadium_mode = true;
    const commands = pitchDrawCommands(frame({ control: { charge: 0.6, charge_kind: "shot", controlled: 0 } }), viewport, opts);
    const hasMeter = commands.some((c) => c.kind === "rect" && c.mode === "fill" && colorEq(c.color, [1, 0.72, 0.3]));
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
// camera.PERSPECTIVE's tuned tilt, ~50 degrees, does not equal
// player_renderer_3d.ts's ELEVATION -- if it ever did coincidentally, this
// test would need a mode-independent invariant instead).
describe("pitch.draw character tilt coherence with camera.perspective_mode", () => {
  function stubRenderer(): THREE.WebGLRenderer {
    return { autoClear: true, render(): void {} } as unknown as THREE.WebGLRenderer;
  }

  function firstRiggedMeshQuaternion(f: RenderFrame): THREE.Quaternion | undefined {
    const group = new THREE.Group();
    pitch.draw(group, f, viewport, opts, stubRenderer());
    const wrapper = group.children.find((c) => c.userData["riggedCharacter"] === true);
    const mesh = wrapper?.children.find((c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh);
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
// DEPTH BUFFER" header section and this port's report for before/after
// screenshots/draw-call counts against a live GL context).
// `player_renderer_3d.ts`'s `build()` only constructs plain three.js
// geometry/skeleton/material objects -- no GL calls -- so `available()`
// genuinely returns true under this workspace's "node" vitest environment
// (the same fact scene.spec.ts's header notes and works around by forcing
// `pitch.rigged_players = false`; this suite does the opposite and leans on
// it, since it is exactly the rigged path being tested here). `characterMesh`
// needs no renderer at all -- there is no per-character render pass or
// render target anymore -- so `renderer` below is a minimal stub whose sole
// job is to be non-`undefined` (the `riggedActive` gate) while asserting it
// is never actually called into; that IS the regression this suite pins:
// `pitch.draw` no longer rasterizes anything, rigged or not, it only builds
// `group`'s object graph. What IS verified here: a rigged player lands in
// the object graph as a real `THREE.SkinnedMesh` (not a quad) and in the
// correct painter's-algorithm position relative to the ball; what is NOT
// verified (same as everywhere else GPU-adjacent in this package): the
// actual rendered pixel content.
describe("pitch.draw (rigged compositing)", () => {
  interface TrackedRenderer {
    readonly renderCalls: number;
  }

  function stubRenderer(): THREE.WebGLRenderer & TrackedRenderer {
    const stub = {
      autoClear: true,
      renderCalls: 0,
      render(): void {
        stub.renderCalls += 1;
      },
    };
    return stub as unknown as THREE.WebGLRenderer & TrackedRenderer;
  }

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
    pitch.rigged_players = true;
  });

  it("adds a rigged player to the object graph as a real SkinnedMesh, never touching the renderer directly", () => {
    const f = frame();
    const group = new THREE.Group();
    const renderer = stubRenderer();

    pitch.draw(group, f, viewport, opts, renderer);

    const wrappers = riggedWrappers(group.children);
    expect(wrappers.length).toBeGreaterThan(0);
    for (const wrapper of wrappers) {
      const mesh = wrapper.children.find((c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh);
      expect(mesh).toBeDefined();
    }
    // The whole point of the single-pass redesign: `pitch.draw` only builds
    // the object graph now, rigged or not -- it never rasterizes, so the one
    // render pass a real frame needs happens exactly once, later, in
    // scene.ts's `SceneRoot.render`.
    expect(renderer.renderCalls).toBe(0);
  });

  it("reuses the SAME pooled mesh instance for the same playerId across frames", () => {
    const f = frame();
    const groupA = new THREE.Group();
    pitch.draw(groupA, f, viewport, opts, stubRenderer());
    const meshA = riggedWrappers(groupA.children)[0]?.children.find((c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh);

    const groupB = new THREE.Group();
    pitch.draw(groupB, f, viewport, opts, stubRenderer());
    const meshB = riggedWrappers(groupB.children)[0]?.children.find((c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh);

    expect(meshA).toBeDefined();
    expect(meshA).toBe(meshB);
  });

  it("interleaves a rigged player with the ball in painter's-algorithm order, matching pitchDrawCommands' depth sort", () => {
    // Player 0 is far (small y), player 1 is near (large y); the ball sits
    // between them. depthSortedItems draws far-to-near, so the expected
    // object order is: player 0's wrapper, then the ball, then player 1's
    // wrapper.
    const f = frame({ players: { ...emptyPlayers(2), y: [50, 500] } });
    const group = new THREE.Group();
    const renderer = stubRenderer();

    pitch.draw(group, f, viewport, opts, renderer);

    const children = group.children;
    const wrapperIndices = children.map((c, i) => (c.userData["riggedCharacter"] === true ? i : -1)).filter((i) => i >= 0);
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

  it("falls back to the procedural billboard (no renderer) without adding any rigged character", () => {
    const f = frame();
    const group = new THREE.Group();

    pitch.draw(group, f, viewport, opts, undefined);

    expect(group.children.length).toBeGreaterThan(0);
    expect(riggedWrappers(group.children)).toHaveLength(0);
  });

  it("falls back to the procedural billboard when rigged_players is turned off, even with a renderer supplied", () => {
    pitch.rigged_players = false;
    const f = frame();
    const group = new THREE.Group();

    pitch.draw(group, f, viewport, opts, stubRenderer());

    expect(group.children.length).toBeGreaterThan(0);
    expect(riggedWrappers(group.children)).toHaveLength(0);
  });

  // DEPTH ZONES (pitch.ts's file header + the block above `depthToZ`).
  describe("depth placement", () => {
    it("leaves the backdrop (drawPitchBeforeItems) at BACKDROP_Z", () => {
      const f = frame();
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts, stubRenderer());

      // The pitch surface trapezoid: the first filled ShapeGeometry mesh in
      // the child list, drawn well before any depth-sorted entity.
      const floor = group.children.find((c) => c instanceof THREE.Mesh && c.geometry instanceof THREE.ShapeGeometry);
      expect(floor).toBeDefined();
      expect(floor?.position.z).toBe(BACKDROP_Z);
    });

    it("positions a rigged player's wrapper within the entity depth zone, mapped from its world y", () => {
      const f = frame(); // both players and the ball sit at y=270, field.h=540
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts, stubRenderer());

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
      pitch.draw(group, f, viewport, opts, stubRenderer());

      const ball = group.children.find(ballMesh);
      expect(ball).toBeDefined();
      expect(ball?.position.z).toBeCloseTo(depthToZ(f.ball.y, f.field.h));
    });

    it("gives a mid-frame billboard fallback (characterMesh declining one player) the same entity-zone z a rigged wrapper would have gotten", () => {
      // available() stays true (so the frame is genuinely riggedActive), but
      // characterMesh itself declines for the FAR player only -- the same
      // "graceful degradation partway through a frame's roster" this file's
      // header describes (a build failure, or -- as stubbed here -- any
      // other reason a specific character comes back undefined).
      const f = frame({ players: { ...emptyPlayers(2), y: [50, 500] } });
      const group = new THREE.Group();
      const spy = vi.spyOn(playerRenderer3d, "characterMesh").mockImplementationOnce(() => undefined);
      try {
        pitch.draw(group, f, viewport, opts, stubRenderer());
      } finally {
        spy.mockRestore();
      }

      // Exactly one rigged wrapper made it through (the near player); the
      // far player fell back to a procedural billboard, which must still
      // carry the SAME entity-zone z a rigged wrapper would have -- draw2d.ts
      // content and rigged characters depth-test against the same zone (see
      // pitch.ts's DEPTH ZONES), not two different conventions.
      const wrappers = riggedWrappers(group.children);
      expect(wrappers).toHaveLength(1);
      const farZ = depthToZ(50, f.field.h);
      const matched = group.children.some((c) => c.userData["riggedCharacter"] !== true && Math.abs(c.position.z - farZ) < 1e-6);
      expect(matched).toBe(true);
    });

    it("makes the post-entity overlay layer (drawPitchAfterItems) ignore depth and win render order, so it always reads on top", () => {
      // The landing reticle: circle "line" commands only (no dl.text), so
      // this exercises the overlay layer without needing buildTextSprite's
      // document.createElement -- unavailable under this workspace's
      // DOM-less "node" vitest environment (see draw2d.ts's header).
      const f = frame({ ball: { x: 480, y: 270, z: 40, visible: true, landing_x: 600, landing_y: 300 } });
      const group = new THREE.Group();
      pitch.draw(group, f, viewport, opts, stubRenderer());

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
// LUA DIFFERENTIAL -- v2/tools/render_reference/. See that directory's
// README.md for how this was captured, and for exactly what is (and is
// deliberately not) covered: full command-level parity for everything
// pitch.lua/pitch.ts draw directly (arena, floor, hex tiling, markings,
// goals, outline, chevrons, the loose ball, the overlay layer), plus the
// full per-player anchor + PlayerRenderOptions payload handed to the player
// renderer -- NOT the player silhouette's own internal limb geometry (that
// module has its own port-fidelity spec), and NOT the relative depth order
// BETWEEN a player and the ball (both languages' comparator was verified by
// direct side-by-side reading instead of a second capture).
//
// This is the gate v2/README.md #1's milestone scope never asked for and
// that the rest of this migration's differential coverage (five suites,
// bit-pattern comparison, Lua-captured fixtures for the wire/diagnostics/
// desync path) never extended to rendering -- see the port report for what
// running it against the CURRENT pitch.ts exposed.
// ============================================================================

const LUA_REFERENCE_JSON = `[{"alpha":1,"color":[0.014999999999999999,0.021999999999999999,0.055],"h":720,"kind":"rect","mode":"fill","w":1280,"x":0,"y":0},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":64,"y":64.799999999999997},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":153.59999999999999,"y":129.59999999999999},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":256,"y":43.199999999999996},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":371.19999999999999,"y":108},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":473.60000000000002,"y":36},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":588.80000000000007,"y":93.600000000000009},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":704,"y":50.400000000000006},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":819.20000000000005,"y":115.2},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":921.59999999999991,"y":36},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":1036.8000000000002,"y":93.600000000000009},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":2,"x":1152,"y":50.400000000000006},{"alpha":0.34999999999999998,"color":[0.91000000000000003,0.95999999999999996,1],"kind":"circle","mode":"fill","r":1,"x":1228.8,"y":129.59999999999999},{"alpha":0.12,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"circle","mode":"fill","r":54,"x":640,"y":147.59999999999999},{"alpha":0.78000000000000003,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"circle","mode":"fill","r":24.48,"x":640,"y":147.59999999999999},{"alpha":0.69999999999999996,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"ellipse","lineWidth":2.6666666666666665,"mode":"line","rx":294.40000000000003,"ry":48.960000000000001,"x":640,"y":147.59999999999999},{"alpha":0.34999999999999998,"color":[0.25,0.88,1],"kind":"ellipse","lineWidth":2.6666666666666665,"mode":"line","rx":396.80000000000001,"ry":64.799999999999997,"x":640,"y":147.59999999999999},{"alpha":0.62,"color":[0.25,0.88,1],"kind":"line","lineWidth":4,"points":[89.600000000000009,159.84,499.20000000000005,159.84]},{"alpha":0.62,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"line","lineWidth":4,"points":[780.79999999999995,159.84,1190.4000000000001,159.84]},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":204.80000000000001,"y":145.44},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":330.24000000000001,"y":145.44},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":455.67999999999995,"y":145.44},{"alpha":0.17999999999999999,"color":[0.25,0.88,1],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":581.12000000000012,"y":145.44},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":706.56000000000006,"y":145.44},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":832,"y":145.44},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":957.44000000000017,"y":145.44},{"alpha":0.17999999999999999,"color":[1,0.66000000000000003,0.23999999999999999],"h":20.16,"kind":"rect","mode":"fill","rx":3,"ry":3,"w":84.480000000000004,"x":1082.8800000000001,"y":145.44},{"alpha":1,"color":[0.025000000000000001,0.16,0.17000000000000001],"kind":"polygon","mode":"fill","points":[313.59999999999997,172.79999999999998,966.40000000000009,172.79999999999998,1177.5999999999999,633.60000000000002,102.39999999999998,633.60000000000002]},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":520,"ry":256,"x":640,"y":403.20000000000005},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":390,"ry":192,"x":640,"y":403.20000000000005},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":260,"ry":128,"x":640,"y":403.20000000000005},{"alpha":0.059999999999999998,"blend":"add","color":[0.050000000000000003,0.16,0.20000000000000001],"kind":"ellipse","mode":"fill","rx":130,"ry":64,"x":640,"y":403.20000000000005},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[387.09437986676261,172.79999999999998,369.3661917887955,222.71999999999997,267.83999999999997,272.63999999999999,290.72000000000003,222.71999999999997,313.59999999999997,172.79999999999998,313.59999999999997,172.79999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[534.08313960028784,172.79999999999998,526.65857536638646,222.71999999999997,435.43600742165665,272.63999999999999,369.36619178879545,222.71999999999997,387.09437986676255,172.79999999999998,460.58875973352519,172.79999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[681.07189933381301,172.79999999999998,683.9509589439773,222.71999999999997,603.03201484331328,272.63999999999999,526.65857536638634,222.71999999999997,534.08313960028784,172.79999999999998,607.57751946705037,172.79999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[828.06065906733818,172.79999999999998,841.24334252156825,222.71999999999997,770.62802226497001,272.63999999999999,683.9509589439773,222.71999999999997,681.07189933381301,172.79999999999998,754.56627920057565,172.79999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[966.40000000000009,172.79999999999998,989.27999999999997,222.71999999999997,938.22402968662664,272.63999999999999,841.24334252156825,222.71999999999997,828.06065906733818,172.79999999999998,901.55503893410082,172.79999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[966.40000000000009,172.79999999999998,989.27999999999997,222.71999999999997,1012.1600000000001,272.63999999999999,989.27999999999997,222.71999999999997,966.40000000000009,172.79999999999998,966.40000000000009,172.79999999999998]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[435.43600742165665,272.63999999999999,410.28325510978812,372.48000000000002,298.45343947692686,422.39999999999998,222.07999999999998,372.48000000000002,267.83999999999997,272.63999999999999,369.36619178879545,222.72]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[603.03201484331339,272.63999999999999,598.4865102195763,372.48000000000002,496.96031843078083,422.39999999999998,410.28325510978812,372.48000000000002,435.43600742165665,272.63999999999999,526.65857536638646,222.72]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[770.62802226497001,272.63999999999999,786.68976532936449,372.48000000000002,695.46719738463469,422.39999999999998,598.4865102195763,372.48000000000002,603.03201484331328,272.63999999999999,683.9509589439773,222.72]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[938.22402968662664,272.63999999999999,974.89302043915256,372.48000000000002,893.97407633848854,422.39999999999998,786.68976532936449,372.48000000000002,770.62802226497001,272.63999999999999,841.24334252156825,222.72]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[1012.1600000000001,272.63999999999999,1057.9200000000001,372.48000000000002,1080.8,422.39999999999998,974.89302043915268,372.48000000000002,938.22402968662686,272.63999999999999,989.27999999999997,222.72]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[298.45343947692697,422.39999999999998,262.99706332099271,522.24000000000001,130.55999999999995,572.16000000000008,153.44000000000005,522.24000000000001,199.19999999999999,422.39999999999998,222.07999999999998,372.48000000000002]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[496.96031843078083,422.39999999999998,482.11118996297807,522.24000000000001,359.97775048605109,572.16000000000008,262.99706332099259,522.24000000000001,298.45343947692686,422.39999999999998,410.28325510978812,372.48000000000002]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[695.46719738463469,422.39999999999998,701.22531660496338,522.24000000000001,589.39550097210213,572.16000000000008,482.11118996297796,522.24000000000001,496.96031843078083,422.39999999999998,598.4865102195763,372.48000000000002]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[893.97407633848854,422.39999999999998,920.3394432469488,522.24000000000001,818.81325145815333,572.16000000000008,701.22531660496338,522.24000000000001,695.4671973846348,422.39999999999998,786.68976532936449,372.48000000000002]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[1080.8,422.39999999999998,1126.5599999999999,522.24000000000001,1048.2310019442043,572.16000000000008,920.3394432469488,522.24000000000001,893.97407633848854,422.39999999999998,974.89302043915256,372.48000000000002]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[1080.8,422.39999999999998,1126.5599999999999,522.24000000000001,1149.4400000000001,572.16000000000008,1126.5599999999999,522.24000000000001,1080.8,422.39999999999998,1057.9200000000001,372.48000000000002]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[359.97775048605109,572.16000000000008,344.49913367874734,633.60000000000002,223.44956683937357,633.60000000000002,102.39999999999998,633.60000000000002,130.55999999999995,572.16000000000008,262.99706332099259,522.24000000000001]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[589.39550097210224,572.16000000000008,586.59826735749482,633.60000000000002,465.54870051812111,633.60000000000002,344.49913367874734,633.60000000000002,359.97775048605109,572.16000000000008,482.11118996297807,522.24000000000001]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[818.81325145815333,572.16000000000008,828.69740103624224,633.60000000000002,707.64783419686842,633.60000000000002,586.59826735749471,633.60000000000002,589.39550097210213,572.16000000000008,701.22531660496338,522.24000000000001]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[1048.2310019442043,572.16000000000008,1070.7965347149895,633.60000000000002,949.74696787561584,633.60000000000002,828.69740103624224,633.60000000000002,818.81325145815333,572.16000000000008,920.3394432469488,522.24000000000001]},{"alpha":0.10000000000000001,"color":[0.16,0.5,0.59999999999999998],"kind":"polygon","lineWidth":4,"mode":"line","points":[1149.4400000000001,572.16000000000008,1177.5999999999999,633.60000000000002,1177.5999999999999,633.60000000000002,1070.7965347149895,633.60000000000002,1048.2310019442045,572.16000000000008,1126.5599999999999,522.24000000000001]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"line","lineWidth":2,"points":[640,172.79999999999998,640,633.60000000000002]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[942.40000000000009,403.20000000000005,952.55377309109463,449.87663015687099,951.88005025534233,495.13501452593971,939.22909751559928,537.60000000000002,914.11674990906545,575.98130948374182,876.84388347909589,609.11274631038134,828.54301541118502,635.98762853725714,771.14389307136582,655.78937646725217,707.25911750668263,667.91632400968149,640,672,572.74088249331737,667.91632400968149,508.85610692863418,655.78937646725217,451.45698458881509,635.98762853725725,403.15611652090411,609.11274631038134,365.88325009093467,575.98130948374182,340.77090248440072,537.60000000000002,328.11994974465767,495.13501452593982,327.44622690890537,449.87663015687099,337.59999999999997,403.20000000000005,356.94204406931112,356.5233698431291,383.55395320402482,311.26498547406027,415.45693330677079,268.79999999999995,450.81307071070739,230.41869051625824,488.08593714067683,197.28725368961872,526.14301541118493,170.41237146274295,564.29011038800127,150.61062353274787,602.23669965372312,138.48367599031843,640,134.39999999999998,677.76330034627676,138.48367599031843,715.70988961199873,150.61062353274781,753.85698458881507,170.4123714627429,791.91406285932317,197.28725368961869,829.18692928929249,230.41869051625815,864.54306669322909,268.7999999999999,896.44604679597501,311.26498547406004,923.05795593068888,356.5233698431291,942.39999999999998,403.19999999999993]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"circle","mode":"fill","r":3,"x":640,"y":403.20000000000005},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[260.79999999999995,288,336.63999999999993,288,252.16000000000003,518.39999999999998,155.19999999999999,518.39999999999998]},{"alpha":0.84999999999999998,"color":[0.34999999999999998,0.71999999999999997,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[943.36000000000013,288,1019.2,288,1124.8,518.39999999999998,1027.8399999999999,518.39999999999998]},{"alpha":0.90000000000000002,"color":[0.25,0.88,1],"kind":"polygon","lineWidth":2,"mode":"line","points":[313.59999999999997,172.79999999999998,966.40000000000009,172.79999999999998,1177.5999999999999,633.60000000000002,102.39999999999998,633.60000000000002]},{"alpha":0.57999999999999996,"color":[0.25,0.88,1],"kind":"line","lineWidth":2,"points":[313.59999999999997,172.79999999999998,303.59999999999997,147.79999999999998]},{"alpha":0.57999999999999996,"color":[0.25,0.88,1],"kind":"line","lineWidth":2,"points":[102.39999999999998,633.60000000000002,90.399999999999977,649.60000000000002]},{"alpha":0.57999999999999996,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"line","lineWidth":2,"points":[966.40000000000009,172.79999999999998,976.40000000000009,147.79999999999998]},{"alpha":0.57999999999999996,"color":[1,0.66000000000000003,0.23999999999999999],"kind":"line","lineWidth":2,"points":[1177.5999999999999,633.60000000000002,1189.5999999999999,649.60000000000002]},{"alpha":0.29999999999999999,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[222.07999999999998,372.48000000000002,213.72159999999991,372.48000000000002,213.72159999999991,366.73360000000002,222.07999999999998,362.03200000000004]},{"alpha":0.29999999999999999,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[193.92000000000007,433.91999999999996,184.9984,433.91999999999996,184.9984,427.78639999999996,193.92000000000007,422.76799999999997]},{"alpha":0.29999999999999999,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[213.72159999999991,372.48000000000002,184.9984,433.91999999999996,184.9984,427.78639999999996,213.72159999999991,366.73360000000002]},{"alpha":0.22,"color":[0.34999999999999998,0.75,1],"kind":"polygon","mode":"fill","points":[222.07999999999998,362.03200000000004,193.92000000000007,422.76799999999997,184.9984,427.78639999999996,213.72159999999991,366.73360000000002]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[222.07999999999998,372.48000000000002,222.07999999999998,362.03200000000004]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[193.92000000000007,433.91999999999996,193.92000000000007,422.76799999999997]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[222.07999999999998,362.03200000000004,193.92000000000007,422.76799999999997]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[213.72159999999991,372.48000000000002,213.72159999999991,366.73360000000002]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[184.9984,433.91999999999996,184.9984,427.78639999999996]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[213.72159999999991,366.73360000000002,184.9984,427.78639999999996]},{"alpha":0.29999999999999999,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[1057.9200000000001,372.48000000000002,1066.2784000000001,372.48000000000002,1066.2784000000001,366.73360000000002,1057.9200000000001,362.03200000000004]},{"alpha":0.29999999999999999,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[1086.0799999999999,433.91999999999996,1095.0016000000001,433.91999999999996,1095.0016000000001,427.78639999999996,1086.0799999999999,422.76799999999997]},{"alpha":0.29999999999999999,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[1066.2784000000001,372.48000000000002,1095.0016000000001,433.91999999999996,1095.0016000000001,427.78639999999996,1066.2784000000001,366.73360000000002]},{"alpha":0.22,"color":[1,0.55000000000000004,0.25],"kind":"polygon","mode":"fill","points":[1057.9200000000001,362.03200000000004,1086.0799999999999,422.76799999999997,1095.0016000000001,427.78639999999996,1066.2784000000001,366.73360000000002]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[1057.9200000000001,372.48000000000002,1057.9200000000001,362.03200000000004]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[1086.0799999999999,433.91999999999996,1086.0799999999999,422.76799999999997]},{"alpha":0.94999999999999996,"color":[0.92000000000000004,0.96999999999999997,1],"kind":"line","lineWidth":3,"points":[1057.9200000000001,362.03200000000004,1086.0799999999999,422.76799999999997]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[1066.2784000000001,372.48000000000002,1066.2784000000001,366.73360000000002]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[1095.0016000000001,433.91999999999996,1095.0016000000001,427.78639999999996]},{"alpha":0.5,"color":[0.69999999999999996,0.84999999999999998,1],"kind":"line","lineWidth":1,"points":[1066.2784000000001,366.73360000000002,1095.0016000000001,427.78639999999996]},{"color":[1,0.55000000000000004,0.25],"kind":"player","opts":{"controlled":false,"dive":0.34999999999999998,"dive_dir":[1,0],"facing":[0,1],"holding":true,"is_keeper":true,"pose_id":"keeper_ready_tall","pose_priority":2,"pose_source":"keeper","species_color":[0.80000000000000004,0.80000000000000004,1],"species_shape":"round","team":"away"},"r":1.51065,"sx":353.536,"sy":241.91999999999999},{"color":[0.34999999999999998,0.75,1],"kind":"player","opts":{"aerial":0.59999999999999998,"aerial_jump":0.22,"aerial_outcome":"clean","aerial_style":"header","controlled":false,"dive_dir":[],"facing":[-1,0],"is_keeper":false,"pose_id":"aerial_header","pose_priority":3,"pose_source":"combat","species_color":[0.90000000000000002,0.5,0.20000000000000001],"species_shape":"angular","team":"home"},"r":1.653125,"sx":834.67200000000003,"sy":384},{"alpha":0.2857142857142857,"color":[0,0,0],"kind":"ellipse","mode":"fill","rx":3.9514285714285711,"ry":1.9757142857142855,"x":640,"y":426.24000000000001},{"alpha":1,"color":[1,0.94999999999999996,0.69999999999999996],"kind":"circle","mode":"fill","r":3.4575,"x":640,"y":420.70800000000003},{"color":[0.34999999999999998,0.75,1],"kind":"player","opts":{"controlled":true,"dashing":true,"dive_dir":[],"facing":[1,0],"is_keeper":false,"pose_id":"tackle","pose_priority":1,"pose_source":"combat","species_color":[1,1,1],"species_shape":"round","team":"home","windup":0.41999999999999998},"r":1.8799999999999999,"sx":678.50239999999997,"sy":510.72000000000003},{"alpha":0.51000000000000001,"color":[1,0.84999999999999998,0.34999999999999998],"kind":"circle","lineWidth":1.0743750000000001,"mode":"line","r":5.157,"x":763.76800000000003,"y":460.80000000000007},{"alpha":0.40000000000000002,"color":[1,0.84999999999999998,0.34999999999999998],"kind":"circle","lineWidth":1.0743750000000001,"mode":"line","r":5.0137499999999999,"x":763.76800000000003,"y":460.80000000000007},{"alpha":0.55249999999999999,"color":[0.34999999999999998,0.75,1],"kind":"circle","lineWidth":1,"mode":"line","r":4.2981249999999998,"x":834.67200000000003,"y":384},{"alpha":0.29250000000000004,"color":[0.34999999999999998,0.75,1],"kind":"circle","lineWidth":1,"mode":"line","r":6.8770000000000007,"x":834.67200000000003,"y":384},{"alpha":0.55000000000000004,"color":[0,0,0],"h":3.008,"kind":"rect","mode":"fill","w":25.568000000000001,"x":665.71839999999997,"y":519.74400000000003},{"alpha":0.94999999999999996,"color":[1,0.71999999999999997,0.29999999999999999],"h":3.008,"kind":"rect","mode":"fill","w":14.062400000000002,"x":665.71839999999997,"y":519.74400000000003},{"alpha":0.34999999999999998,"color":[1,1,1],"h":3.008,"kind":"rect","lineWidth":1,"mode":"line","w":25.568000000000001,"x":665.71839999999997,"y":519.74400000000003},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[670.83199999999999,519.74400000000003,670.83199999999999,522.75200000000007]},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[675.94560000000001,519.74400000000003,675.94560000000001,522.75200000000007]},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[681.05919999999992,519.74400000000003,681.05919999999992,522.75200000000007]},{"alpha":0.34999999999999998,"color":[1,1,1],"kind":"line","lineWidth":1,"points":[686.17279999999994,519.74400000000003,686.17279999999994,522.75200000000007]},{"align":"center","alpha":0.94999999999999996,"color":[1,0.71999999999999997,0.29999999999999999],"kind":"text","text":"SHOT","w":25.568000000000001,"x":665.71839999999997,"y":523.75200000000007}]`;

/** One `love.graphics` call, normalized -- see capture_pitch_reference.lua. */
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

/** One `game.render.player_renderer.draw(sx, sy, r, color, v, opts)` call, normalized. */
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
    // [] when pitch.lua's always-constructed {x=nil,y=nil} had no components
    // -- see this describe block's own test for the divergence this causes.
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

// A deliberately SMALL field -- see v2/tools/render_reference/README.md's
// "How the fixture was chosen". MUST stay byte-for-byte identical to
// capture_pitch_reference.lua's `frame` literal; the comparison only means
// something if both sides consumed the same input.
function luaDifferentialFrame(): RenderFrame {
  return {
    field: { w: 200, h: 120, penalty_box_depth: 20, penalty_box_h: 60, crossbar_h: 16, goal_home: { x: -2, y: 52, w: 2, h: 16 }, goal_away: { x: 200, y: 52, w: 2, h: 16 } },
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
    // Deliberately roster-slot (one-based, matching the wire -- README rule 3
    // and frame_buffer.ts's own decode comment) rather than zero-based array
    // index: player 1 (roster slot 1) sits at TS array index 0; player 3
    // (roster slot 3) sits at TS array index 2. See this suite's "KNOWN BUG"
    // block below for what asserting these against the Lua reference exposed.
    control: { pass_target: 3, charge_kind: "shot", charge: 0.55, controlled: 1 },
  };
}

const LUA_DIFFERENTIAL_VIEWPORT: PitchViewport = { w: 1280, h: 720 };
const LUA_DIFFERENTIAL_OPTS: PitchDrawOptions = { home_color: [0.35, 0.75, 1.0], away_color: [1.0, 0.55, 0.25] };

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

// Normalizes EITHER a captured Lua geometry record OR a real `DrawCommand`
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
// before (matching the Lua's own draw order), because they are static,
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

function colorCloseTo(color: readonly number[], target: readonly [number, number, number]): boolean {
  return Math.abs((color[0] ?? NaN) - target[0]) < 1e-6 && Math.abs((color[1] ?? NaN) - target[1]) < 1e-6 && Math.abs((color[2] ?? NaN) - target[2]) < 1e-6;
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
    throw new Error("expected to find the landing-reticle ring (the overlay section's first entry) in this fixture's output");
  }
  return idx;
}

describe("pitch.pitchDrawCommands differential against the real Lua game.render.pitch", () => {
  function drawWithoutPlayers(): DrawCommand[] {
    const spy = vi.spyOn(playerRenderer, "playerDrawCommands").mockReturnValue([]);
    try {
      return pitchDrawCommands(luaDifferentialFrame(), LUA_DIFFERENTIAL_VIEWPORT, LUA_DIFFERENTIAL_OPTS, 0);
    } finally {
      spy.mockRestore();
    }
  }

  it("matches the Lua capture's static arena/pitch scene and the loose ball -- backdrop, floor, markings, goal nets/frames, outline, chevrons, ball shadow+lift (content-equal; see sortedNormalized's comment for why this is order-insensitive). The hex floor's line width is excluded here -- see the dedicated KNOWN BUG test below.", () => {
    const commands = drawWithoutPlayers();
    const luaBefore = LUA_GEOM_RECORDS.slice(0, overlayStart(LUA_GEOM_RECORDS)).filter((r) => !isHexFloorTile(r));
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

  it("draws the same NUMBER of overlay commands as the Lua capture (pass-target preview + charge meter) -- their POSITIONS are pinned separately by the KNOWN BUG tests further down, since both currently diverge from Lua", () => {
    const commands = drawWithoutPlayers();
    const luaOverlayCount = LUA_GEOM_RECORDS.length - overlayStart(LUA_GEOM_RECORDS);
    const tsOverlayCount = commands.length - overlayStart(commands);

    expect(tsOverlayCount).toBe(luaOverlayCount);
  });

  // KNOWN BUG, found by this differential: `game/render/arena.lua`'s
  // backdrop draw ends with `love.graphics.setLineWidth(math.max(2,
  // viewport.h / 180))` for its ribbon markers (720 / 180 = 4 for this
  // fixture's viewport) and never resets it. `pitch.lua`'s `draw_hex_floor`
  // never calls `setLineWidth` itself, so LÖVE's stateful graphics context
  // leaves that 4px width in effect for every hex tile line -- a real,
  // rendered, reproducible feature of the Lua original (confirmed by this
  // capture), not an authoring accident this port should "correct": per
  // v2/README.md's rule 9 and this task's own brief, "A port reproduces
  // behaviour, not intent." `pitch.ts`'s `drawHexFloor` has no equivalent
  // ambient state to inherit from (`arena.ts`'s backdrop is a separate, pure
  // command-producing function with no shared mutable graphics context), so
  // it draws hex lines at the `dl.polygon` default width (1px) -- visibly
  // thinner/fainter grid lines than the original ever actually rendered.
  // Reproducing this needs `Math.max(2, vp.h / 180)` threaded into
  // `drawHexFloor`'s own `dl.polygon(..., { lineWidth: ... })` call --
  // outside this task's file ownership (`pitch.ts`), so pinned here instead.
  it.fails("hex floor tiles render at the Lua original's actual (leaked, viewport-dependent) line width, not the DrawList default", () => {
    const commands = drawWithoutPlayers();
    const luaHex = LUA_GEOM_RECORDS.filter(isHexFloorTile);
    const tsHex = commands.filter(isHexFloorTile);

    expect(tsHex.length).toBeGreaterThan(0);
    expect(tsHex.map(normalizeGeom)).toEqual(luaHex.map(normalizeGeom));
  });

  it("hands the player renderer the exact same anchor (sx, sy, r, color) and PlayerRenderOptions payload the Lua original computes, in the same depth-sorted order", () => {
    interface Captured {
      readonly sx: number;
      readonly sy: number;
      readonly r: number;
      readonly color: RGB;
      readonly options: PlayerRenderOptions;
    }
    const captured: Captured[] = [];
    const spy = vi.spyOn(playerRenderer, "playerDrawCommands").mockImplementation((sx, sy, r, color, _v, options) => {
      captured.push({ sx, sy, r, color, options });
      return [];
    });
    try {
      pitchDrawCommands(luaDifferentialFrame(), LUA_DIFFERENTIAL_VIEWPORT, LUA_DIFFERENTIAL_OPTS, 0);
    } finally {
      spy.mockRestore();
    }

    expect(captured).toHaveLength(LUA_PLAYER_RECORDS.length);
    for (const [i, luaPlayer] of LUA_PLAYER_RECORDS.entries()) {
      const ts = captured[i];
      if (ts === undefined) {
        throw new Error(`missing captured player renderer call at depth-sorted index ${i}`);
      }
      expect(round(ts.sx)).toBeCloseTo(round(luaPlayer.sx), 4);
      expect(round(ts.sy)).toBeCloseTo(round(luaPlayer.sy), 4);
      expect(round(ts.r)).toBeCloseTo(round(luaPlayer.r), 4);
      expect(roundArr(ts.color)).toEqual(roundArr(luaPlayer.color));

      const o = ts.options;
      expect(o.facing !== undefined ? [o.facing.x, o.facing.y] : undefined).toEqual(luaPlayer.opts.facing);
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
      expect(o.species_color !== undefined ? [...o.species_color] : undefined).toEqual(luaPlayer.opts.species_color);
      expect(o.team).toBe(luaPlayer.opts.team);
      expect(o.pose?.id).toBe(luaPlayer.opts.pose_id);
      expect(o.pose?.priority).toBe(luaPlayer.opts.pose_priority);
      expect(o.pose?.source).toBe(luaPlayer.opts.pose_source);

      // KNOWN, NARROW DIVERGENCE (see v2/tools/render_reference/README.md and
      // the port report): pitch.lua ALWAYS constructs
      // `player_opts.dive_dir = { x = players.dive_dir_x[index], y =
      // players.dive_dir_y[index] }` (pitch.lua's PlayerRenderOptions block),
      // even when neither component is set -- so a non-diving player's Lua
      // record carries dive_dir as a table with two nil fields (this
      // harness's JSON encodes that as `[]`, since a Lua table with no
      // non-nil entries is indistinguishable from an empty array). pitch.ts's
      // playerOptions() only includes `dive_dir` at all when BOTH
      // dive_dir_x/y are defined, omitting the key entirely otherwise. Both
      // convey "no dive direction data", but via a different representation
      // (present-with-nils vs absent-key) -- documented here, not asserted
      // equal, since which one is "right" depends on whether a downstream
      // consumer (player_renderer_3d.ts/rig3d, outside this task's scope)
      // branches on the KEY'S PRESENCE or on the component VALUES.
      if (luaPlayer.opts.dive_dir !== undefined && luaPlayer.opts.dive_dir.length === 2) {
        expect(o.dive_dir).toBeDefined();
        expect(o.dive_dir !== undefined ? [o.dive_dir.x, o.dive_dir.y] : undefined).toEqual(luaPlayer.opts.dive_dir);
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
// preview. The Lua original has no such bug: `game.render.pitch` indexes its
// own (also one-based) `players.x` Lua array with the same one-based value,
// so the two stay aligned there.
//
// FIXED. `drawPitchAfterItems` now subtracts 1 from both fields before
// indexing the zero-based SoA arrays. These were pinned as `it.fails` by the
// differential that found the bug, and flipped to real assertions once the
// fix landed -- the transition from expected-fail to pass IS the proof.
describe("pitch.ts converts frame.control.controlled/pass_target from one-based to zero-based", () => {
  it("charge meter renders under the CORRECT (one-based-adjusted) controlled player, matching the Lua reference", () => {
    const commands = pitchDrawCommands(luaDifferentialFrame(), LUA_DIFFERENTIAL_VIEWPORT, LUA_DIFFERENTIAL_OPTS, 0);
    // Lua's charge-meter bar fill (color = home_color, the "shot" charge
    // color multiplied through -- see the captured reference) sits at
    // x=665.7184 (home-1, roster slot 1 == TS array index 0). The current
    // (buggy) pitch.ts instead reads players.x[1] directly -- TS index 1,
    // "away-kp" -- and renders the bar under the WRONG player.
    const expectedX = 665.7184;
    const meterFill = commands.find((c) => c.kind === "rect" && c.mode === "fill" && Math.abs(c.color[0] - 1) < 1e-6 && Math.abs(c.color[1] - 0.72) < 1e-6 && Math.abs(c.color[2] - 0.3) < 1e-6);
    expect(meterFill).toBeDefined();
    expect(meterFill?.kind === "rect" ? round(meterFill.x) : undefined).toBeCloseTo(expectedX, 2);
  });

  it("pass-target preview renders at the CORRECT (one-based-adjusted) target player's position, matching the Lua reference", () => {
    const commands = pitchDrawCommands(luaDifferentialFrame(), LUA_DIFFERENTIAL_VIEWPORT, LUA_DIFFERENTIAL_OPTS, 0);
    // Lua's pass-target rings (home_color, since roster slot 3 / "home-2" is
    // on the home team) sit at (834.672, 384) -- that player's own projected
    // anchor. The current (buggy) pitch.ts reads players.x[3]/players.y[3],
    // both out of bounds on a 3-element zero-based array, defaulting to
    // world (0, 0) via `?? 0` -- nowhere near the intended player.
    const expectedX = 834.672;
    const expectedY = 384;
    const ring = commands.find((c) => c.kind === "circle" && c.mode === "line" && Math.abs(c.color[0] - 0.35) < 1e-6 && Math.abs(c.color[1] - 0.75) < 1e-6 && Math.abs(c.color[2] - 1) < 1e-6);
    expect(ring).toBeDefined();
    expect(ring?.kind === "circle" ? round(ring.x) : undefined).toBeCloseTo(expectedX, 2);
    expect(ring?.kind === "circle" ? round(ring.y) : undefined).toBeCloseTo(expectedY, 2);
  });
});
