// New tests for pitch.ts's pure path (`pitchDrawCommands`). No Lua spec
// targets game/render/pitch.lua with a claimable, self-contained fixture --
// spec/render/draw_smoke_spec.lua and spec/render/combat_presentation_spec.lua
// both exercise it, but only by stubbing `love.graphics` and pulling in
// `sim.match`/`data.teams`/`data.formations` (Rust-owned) and `game.ui.draw`/
// `game.match_hud` (other packages) -- see this package's port report for
// why those specs are not claimed wholesale.

import { describe, expect, it, afterEach, vi } from "vitest";
import * as THREE from "three";
import { pitchDrawCommands, pitch, depthToZ, BACKDROP_Z, ENTITY_Z_NEAR, ENTITY_Z_FAR, OVERLAY_RENDER_ORDER, type PitchDrawOptions, type RenderFrame } from "./pitch.ts";
import * as playerRenderer3d from "./player_renderer_3d.ts";

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
