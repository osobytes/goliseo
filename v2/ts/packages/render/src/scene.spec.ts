// New tests for scene.ts (new shared infrastructure, not a port of one Lua
// file -- see scene.ts's own header).
//
// TESTABILITY BOUNDARY: constructing a real `THREE.WebGLRenderer` needs a
// live GL context (a real <canvas> in a browser, or a WebGL polyfill), which
// does not exist in this milestone's headless `vitest` environment ("node",
// no DOM -- see v2/ts/vitest.config.ts). This suite therefore never
// constructs one. `stubRenderer()` below is NOT a WebGL mock: it is a plain
// object satisfying the handful of methods `SceneRoot` calls directly on the
// renderer it was handed (`setPixelRatio`, `setSize`, `autoClear`, `dispose`,
// and -- only reachable if `pitch.rigged_players` were left on --
// `render`). None of those touch three.js's actual WebGLRenderer
// implementation or a GL context; they let this suite verify SceneRoot's OWN
// orchestration (what it calls, in what order, on what objects) the same way
// a fake database connection tests a repository's logic without a real
// database. `render()`'s actual pixel output is never asserted here -- that
// needs a live GL context to verify at all (see pitch.ts's SCOPE NOTE and
// scene.ts's own doc comment on the rigged-player compositing gap).
//
// `pitch.rigged_players` is deliberately forced off for every test in this
// file (saved/restored around the suite): it is module-level mutable state
// shared with every other spec importing pitch.ts in the same process, and
// forcing it off keeps this suite's `populate()` calls on the deterministic,
// GL-free billboard path instead of depending on whether
// `player_renderer_3d.available()` happens to succeed in this environment.
//
// This suite calls `SceneRoot.populate` (object-graph assembly), never
// `SceneRoot.render` (populate + `Bloom.draw`). `Bloom.draw`'s
// `EffectComposer` needs a much larger slice of the real
// `THREE.WebGLRenderer` surface (clear color, render targets, ...) than
// anything `stubRenderer()` below provides -- see scene.ts's `populate` doc
// comment for why that split exists.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import * as THREE from "three";
import { pitch, resetStaticSceneCache, staticSceneBuildCount, type PitchDrawOptions, type RenderFrame } from "./pitch.ts";
import { camera, perspectiveRig } from "./camera.ts";
import { SceneRoot, type WorldLayer } from "./scene.ts";

interface StubRenderer {
  autoClear: boolean;
  setSize(w: number, h: number, updateStyle?: boolean): void;
  setPixelRatio(ratio: number): void;
  dispose(): void;
  render(): void;
  readonly setSizeCalls: ReadonlyArray<readonly [number, number]>;
  readonly setPixelRatioCalls: readonly number[];
  disposeCalls: number;
}

function stubRenderer(): StubRenderer {
  const setSizeCalls: [number, number][] = [];
  const setPixelRatioCalls: number[] = [];
  return {
    autoClear: true,
    setSizeCalls,
    setPixelRatioCalls,
    disposeCalls: 0,
    setSize(w, h) {
      setSizeCalls.push([w, h]);
    },
    setPixelRatio(ratio) {
      setPixelRatioCalls.push(ratio);
    },
    dispose() {
      this.disposeCalls += 1;
    },
    render() {
      // no-op: never reached with pitch.rigged_players forced off, kept only
      // so a stray call fails loudly (TypeError) rather than silently, and
      // so the type below is structurally close to THREE.WebGLRenderer.
    },
  };
}

function asRenderer(stub: StubRenderer): THREE.WebGLRenderer {
  return stub as unknown as THREE.WebGLRenderer;
}

// A minimal `WorldLayer` (scene.ts's own interface -- see its doc comment):
// a real `THREE.Group` so `setWorldLayer`'s scene-graph add/remove is
// genuinely exercised, plus counters so `populate`/`dispose`'s calls into
// `update`/`dispose` are observable without needing a live GL context (the
// world layer's OWN content is never rasterized by anything this suite
// calls -- see this file's header on `populate` vs `render`).
interface StubWorldLayer extends WorldLayer {
  readonly updateCalls: number[];
  disposeCalls: number;
}

function stubWorldLayer(): StubWorldLayer {
  const group = new THREE.Group();
  const updateCalls: number[] = [];
  return {
    group,
    updateCalls,
    disposeCalls: 0,
    update(now) {
      updateCalls.push(now);
    },
    dispose() {
      this.disposeCalls += 1;
    },
  };
}

const VIEWPORT = { w: 800, h: 600 };

function frame(playerCount: 0 | 1 | 2 = 2): RenderFrame {
  const ids = ["home_0", "away_0"].slice(0, playerCount);
  return {
    field: {
      w: 900,
      h: 500,
      penalty_box_depth: 60,
      penalty_box_h: 220,
      crossbar_h: 60,
      goal_home: { x: 0, y: 210, w: 12, h: 80 },
      goal_away: { x: 888, y: 210, w: 12, h: 80 },
    },
    roster: {
      radius: ids.map(() => 12),
      teams: ids.map((_, i) => (i % 2 === 0 ? "home" : "away")),
      is_keeper: ids.map(() => false),
      species_shape: ids.map(() => "round"),
      species_color: ids.map(() => [1, 1, 1] as const),
      ids,
    },
    players: {
      count: ids.length,
      x: ids.map((_, i) => 100 + i * 50),
      y: ids.map(() => 250),
      facing_x: ids.map(() => 1),
      facing_y: ids.map(() => 0),
      controlled: ids.map(() => false),
      dashing: ids.map(() => undefined),
      dive: ids.map(() => undefined),
      dive_dir_x: ids.map(() => undefined),
      dive_dir_y: ids.map(() => undefined),
      holding: ids.map(() => undefined),
      grab: ids.map(() => undefined),
      throw: ids.map(() => undefined),
      windup: ids.map(() => undefined),
      aerial: ids.map(() => undefined),
      aerial_style: ids.map(() => undefined),
      aerial_outcome: ids.map(() => undefined),
      aerial_jump: ids.map(() => undefined),
      pose_id: ids.map(() => undefined),
      pose_priority: ids.map(() => undefined),
      pose_source: ids.map(() => undefined),
    },
    ball: { x: 450, y: 250, z: 0, visible: true },
    control: { charge: 0, controlled: 0 },
  };
}

const pitchOptions: PitchDrawOptions = {
  home_color: [0.2, 0.6, 1],
  away_color: [1, 0.4, 0.2],
};

let savedRigged: boolean;

beforeEach(() => {
  savedRigged = pitch.rigged_players;
  pitch.rigged_players = false;
  resetStaticSceneCache();
});

afterEach(() => {
  pitch.rigged_players = savedRigged;
  resetStaticSceneCache();
});

describe("SceneRoot construction", () => {
  it("assembles one scene containing pitchGroup then hudGroup, hud painting after pitch", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    expect(scene.scene.children).toEqual([scene.pitchGroup, scene.hudGroup]);
    expect(scene.hudGroup.renderOrder).toBeGreaterThan(scene.pitchGroup.renderOrder);
  });

  it("sizes the renderer and camera frustum from the constructor viewport", () => {
    const renderer = stubRenderer();
    const scene = new SceneRoot(asRenderer(renderer), { viewport: VIEWPORT, pixelRatio: 2 });
    expect(renderer.setSizeCalls).toEqual([[VIEWPORT.w, VIEWPORT.h]]);
    expect(renderer.setPixelRatioCalls).toEqual([2]);
    expect(scene.camera.right).toBe(VIEWPORT.w);
    expect(scene.camera.bottom).toBe(VIEWPORT.h);
    expect(scene.camera.top).toBe(0);
    expect(scene.camera.left).toBe(0);
  });

  it("always constructs worldScene/worldCamera, even with no world layer attached", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    expect(scene.worldScene).toBeInstanceOf(THREE.Scene);
    expect(scene.worldCamera).toBeInstanceOf(THREE.PerspectiveCamera);
    expect(scene.worldScene.children).toHaveLength(0);
  });
});

describe("SceneRoot.resize", () => {
  it("reuses the existing scene/camera/groups -- none are reconstructed", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const sceneObj = scene.scene;
    const camera = scene.camera;
    const pitchGroup = scene.pitchGroup;
    const hudGroup = scene.hudGroup;

    scene.resize({ w: 1024, h: 768 });

    expect(scene.scene).toBe(sceneObj);
    expect(scene.camera).toBe(camera);
    expect(scene.pitchGroup).toBe(pitchGroup);
    expect(scene.hudGroup).toBe(hudGroup);
    expect(scene.scene.children).toHaveLength(2);
  });

  it("updates the camera frustum and forwards the new size to the renderer", () => {
    const renderer = stubRenderer();
    const scene = new SceneRoot(asRenderer(renderer), { viewport: VIEWPORT });
    scene.resize({ w: 1024, h: 768 });
    expect(scene.camera.right).toBe(1024);
    expect(scene.camera.bottom).toBe(768);
    expect(scene.getViewport()).toEqual({ w: 1024, h: 768 });
    expect(renderer.setSizeCalls.at(-1)).toEqual([1024, 768]);
  });

  it("throws rather than resizing a disposed scene", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.dispose();
    expect(() => scene.resize({ w: 10, h: 10 })).toThrow(/dispose/);
  });

  it("updates worldCamera's aspect too, regardless of whether a world layer is attached", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.resize({ w: 1024, h: 768 });
    expect(scene.worldCamera.aspect).toBeCloseTo(1024 / 768);
  });
});

describe("SceneRoot.populate", () => {
  it("populates pitchGroup and restores renderer.autoClear to what it was before the call", () => {
    const renderer = stubRenderer();
    const scene = new SceneRoot(asRenderer(renderer), { viewport: VIEWPORT });
    renderer.autoClear = true;
    scene.populate(frame(), { pitch: pitchOptions });
    expect(scene.pitchGroup.children.length).toBeGreaterThan(0);
    expect(renderer.autoClear).toBe(true);
  });

  it("does not rebuild pitch.ts's memoised static scene across repeated frames with the same projection", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.populate(frame(), { pitch: pitchOptions });
    expect(staticSceneBuildCount()).toBe(1);
    scene.populate(frame(), { pitch: pitchOptions });
    scene.populate(frame(), { pitch: pitchOptions });
    expect(staticSceneBuildCount()).toBe(1);
  });

  it("rebuilds the static scene when the viewport changes", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.populate(frame(), { pitch: pitchOptions });
    expect(staticSceneBuildCount()).toBe(1);
    scene.resize({ w: 640, h: 480 });
    scene.populate(frame(), { pitch: pitchOptions });
    expect(staticSceneBuildCount()).toBe(2);
  });

  it("does not repopulate hudGroup with content when hud is omitted", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.populate(frame(), { pitch: pitchOptions });
    expect(scene.hudGroup.children).toHaveLength(0);
  });

  // NOT TESTED: actually calling `populate` with `hud` supplied. Every
  // `match_hud.ts` HUD unconditionally draws at least one `dl.text(...)`
  // command (the venue label), and draw2d.ts's `paint` turns each of those
  // into a canvas-texture sprite via `document.createElement("canvas")`
  // (see draw2d.ts's `buildTextSprite`) -- `document` does not exist in
  // this milestone's "node" vitest environment (no DOM, not just no GL; see
  // v2/ts/vitest.config.ts). This is not new to scene.ts: match_hud.ts's own
  // `drawMatchHud` is already documented "Impure ... Untested -- see
  // draw2d.ts" for exactly this reason. Polyfilling `document` here would be
  // the DOM-flavoured version of "mocking a WebGL context into existence" --
  // this suite says so instead. The `hud`-omitted path above (which never
  // reaches `drawMatchHud`) is what stays covered.

  it("throws rather than populating a disposed scene", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.dispose();
    expect(() => scene.populate(frame(), { pitch: pitchOptions })).toThrow(/dispose/);
  });
});

// See scene.ts's WORLD LAYER class doc comment section and `populate`'s own
// WORLD LAYER SYNC doc comment. `camera.perspective_mode` is module-level
// mutable state shared with camera.spec.ts and every other spec importing
// camera.ts, so it is saved/restored around this describe block the same
// way `pitch.rigged_players` is saved/restored above.
describe("SceneRoot world layer", () => {
  let savedPerspective: boolean;

  beforeEach(() => {
    savedPerspective = camera.perspective_mode;
  });

  afterEach(() => {
    camera.perspective_mode = savedPerspective;
  });

  it("setWorldLayer adds the layer's group to worldScene, and removes it when set back to undefined", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const layer = stubWorldLayer();

    scene.setWorldLayer(layer);
    expect(scene.worldScene.children).toEqual([layer.group]);

    scene.setWorldLayer(undefined);
    expect(scene.worldScene.children).toHaveLength(0);
    // Detaching does NOT dispose -- see setWorldLayer's ownership doc comment.
    expect(layer.disposeCalls).toBe(0);
  });

  it("setWorldLayer swaps to a new layer, detaching (not disposing) the old one", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const first = stubWorldLayer();
    const second = stubWorldLayer();

    scene.setWorldLayer(first);
    scene.setWorldLayer(second);

    expect(scene.worldScene.children).toEqual([second.group]);
    expect(first.disposeCalls).toBe(0);
  });

  it("calls the world layer's update(now) during populate when perspective_mode is on", () => {
    camera.perspective_mode = true;
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const layer = stubWorldLayer();
    scene.setWorldLayer(layer);

    scene.populate(frame(), { pitch: pitchOptions, now: 12.5 });

    expect(layer.updateCalls).toEqual([12.5]);
  });

  it("does not update the world layer when perspective_mode is off, even with a layer attached", () => {
    camera.perspective_mode = false;
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const layer = stubWorldLayer();
    scene.setWorldLayer(layer);

    scene.populate(frame(), { pitch: pitchOptions, now: 12.5 });

    expect(layer.updateCalls).toEqual([]);
  });

  it("does not update anything when no world layer is attached, even with perspective_mode on", () => {
    camera.perspective_mode = true;
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    // No setWorldLayer call at all -- populate must not throw or touch worldCamera meaningfully.
    expect(() => scene.populate(frame(), { pitch: pitchOptions })).not.toThrow();
  });

  it("syncs worldCamera position/fov/near/far from camera.perspectiveRig for the frame's field, with aspect from the viewport", () => {
    camera.perspective_mode = true;
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    scene.setWorldLayer(stubWorldLayer());
    const f = frame();

    scene.populate(f, { pitch: pitchOptions });

    // pitch.follow_camera defaults to false (pitch.spec.ts pins this), so
    // this must match perspectiveRig with NO view -- the same call
    // syncWorldCamera makes internally.
    const rig = perspectiveRig(f.field);
    expect(scene.worldCamera.position.x).toBeCloseTo(rig.eye[0]);
    expect(scene.worldCamera.position.y).toBeCloseTo(rig.eye[1]);
    expect(scene.worldCamera.position.z).toBeCloseTo(rig.eye[2]);
    expect(scene.worldCamera.fov).toBeCloseTo(rig.fov);
    expect(scene.worldCamera.near).toBeCloseTo(rig.near);
    expect(scene.worldCamera.far).toBeCloseTo(rig.far);
    expect(scene.worldCamera.aspect).toBeCloseTo(VIEWPORT.w / VIEWPORT.h);
  });
});

describe("SceneRoot.dispose", () => {
  it("empties both groups, calls renderer.dispose exactly once, and is idempotent", () => {
    const renderer = stubRenderer();
    const scene = new SceneRoot(asRenderer(renderer), { viewport: VIEWPORT });
    scene.populate(frame(), { pitch: pitchOptions });
    expect(scene.pitchGroup.children.length).toBeGreaterThan(0);

    scene.dispose();
    expect(scene.pitchGroup.children).toHaveLength(0);
    expect(scene.hudGroup.children).toHaveLength(0);
    expect(renderer.disposeCalls).toBe(1);

    scene.dispose();
    expect(renderer.disposeCalls).toBe(1);
  });

  it("disposes each child mesh's geometry and material", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    let geometryDisposed = false;
    let materialDisposed = false;
    const geometry = new THREE.PlaneGeometry(1, 1);
    geometry.dispose = () => {
      geometryDisposed = true;
    };
    const material = new THREE.MeshBasicMaterial();
    material.dispose = () => {
      materialDisposed = true;
    };
    scene.pitchGroup.add(new THREE.Mesh(geometry, material));

    scene.dispose();

    expect(geometryDisposed).toBe(true);
    expect(materialDisposed).toBe(true);
  });

  it("disposes and detaches a still-attached world layer", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const layer = stubWorldLayer();
    scene.setWorldLayer(layer);

    scene.dispose();

    expect(layer.disposeCalls).toBe(1);
    expect(scene.worldScene.children).toHaveLength(0);
  });

  it("does not throw disposing when no world layer was ever attached", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    expect(() => scene.dispose()).not.toThrow();
  });

  it("does not double-dispose a world layer across two dispose() calls", () => {
    const scene = new SceneRoot(asRenderer(stubRenderer()), { viewport: VIEWPORT });
    const layer = stubWorldLayer();
    scene.setWorldLayer(layer);

    scene.dispose();
    scene.dispose();

    expect(layer.disposeCalls).toBe(1);
  });
});
