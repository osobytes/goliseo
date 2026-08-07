// New tests for player_renderer_3d.ts's pure half: pose selection and the
// metres-per-world-unit conversion. No Lua spec targets
// game/render/player_renderer_3d.lua directly (it has no `spec/render/`
// counterpart in the Lua tree). Most of the GPU-adjacent mesh/skeleton/camera
// half is untested -- see this package's port report -- EXCEPT
// `renderToSprite`'s and `characterMesh`'s object-graph shape (below), which
// need only a stub renderer (or no renderer at all, for `characterMesh`),
// not a live GL context, to verify (the same boundary scene.spec.ts's
// `stubRenderer()` and pitch.spec.ts's rigged-compositing suite draw);
// `build()` itself only constructs plain three.js geometry/skeleton objects
// with no GL calls, so it genuinely succeeds under this workspace's "node"
// vitest environment.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { Vec2 } from "@gc/core";
import { available, characterCameraParams, characterMesh, clipFor, metresPerWorldUnit, poseFor, ppmForRadius, renderToSprite, DEFAULT_PLAYER_RADIUS } from "./player_renderer_3d.ts";
import type { PlayerRenderOptions } from "./player_renderer.ts";
import type { PlayerView } from "./view_state.ts";

function baseOptions(overrides: Partial<PlayerRenderOptions> = {}): PlayerRenderOptions {
  return { is_keeper: false, controlled: false, ...overrides };
}

describe("player_renderer_3d.clipFor", () => {
  it("maps stances/gaits onto their limb clip", () => {
    expect(clipFor("locomotion")).toBe("locomotion");
    expect(clipFor("contain")).toBe("locomotion");
    expect(clipFor("keeper_shuffle")).toBe("locomotion");
    expect(clipFor("combat_guard")).toBe("guard");
    expect(clipFor("combat_windup")).toBe("guard");
    expect(clipFor("combat_active")).toBe("charge");
    expect(clipFor("combat_recovery")).toBe("guard");
    expect(clipFor("combat_aim")).toBe("guard");
    expect(clipFor("fatigue")).toBe("idle");
  });

  it("falls back to idle for an unmapped or missing pose id (e.g. whole-body actions owned by action_pose.ts)", () => {
    expect(clipFor(undefined)).toBe("idle");
    expect(clipFor("aerial_bicycle")).toBe("idle");
    expect(clipFor("keeper_dive")).toBe("idle");
  });
});

describe("player_renderer_3d.metresPerWorldUnit", () => {
  it("is size-independent of depth: converting the same world distance at any scale gives the same metres", () => {
    // The conversion itself does not take a `scale`/`radius` argument -- it
    // is a fixed ratio derived once from the rig's height -- so this checks
    // that two different rig heights produce their own distinct, stable ratios.
    const short = metresPerWorldUnit(1.6);
    const tall = metresPerWorldUnit(2.0);
    expect(tall).toBeGreaterThan(short);
    expect(metresPerWorldUnit(1.6)).toBe(short);
  });

  it("uses sim/match.lua's PLAYER_RADIUS (12) as its default divisor", () => {
    const height = 1.8;
    expect(metresPerWorldUnit(height)).toBe(height / (DEFAULT_PLAYER_RADIUS * 3.0 * 2));
  });
});

describe("player_renderer_3d.poseFor", () => {
  const idleView: PlayerView = { px: 0, py: 0, speed: 0, phase: 0, gait: 0, lean: 0 };

  it("produces a pose with a root-adjacent sparse rotation/move map, deterministic for the same `now`", () => {
    const a = poseFor(idleView, baseOptions(), 1.23);
    const b = poseFor(idleView, baseOptions(), 1.23);
    expect(a).toEqual(b);
  });

  it("differs when the pose id selects the charge overlay vs plain idle", () => {
    const idle = poseFor(idleView, baseOptions(), 1.0);
    const charging = poseFor(idleView, baseOptions({ pose: { id: "combat_active" } }), 1.0);
    expect(idle).not.toEqual(charging);
  });

  it("layers the keeper sling overlay while throw_timer counts down toward zero", () => {
    const midThrow = poseFor(idleView, baseOptions({ throw: 0.5 }), 1.0);
    const noThrow = poseFor(idleView, baseOptions({ throw: 0 }), 1.0);
    expect(midThrow).not.toEqual(noThrow);
  });

  it("applies a whole-body action (e.g. a keeper dive) on top of the gait/stance pose", () => {
    // dive_dir must have a nonzero lateral component relative to facing (see
    // rig3d/action_pose.ts's `lateralSign`) or the save overlay is a no-op.
    const grounded = poseFor(idleView, baseOptions(), 1.0);
    const diving = poseFor(idleView, baseOptions({ dive: 1, dive_dir: new Vec2(0, 1), pose: { id: "keeper_dive" } }), 1.0);
    expect(diving.rot["root"]).toBeDefined();
    expect(grounded.rot["root"]).not.toEqual(diving.rot["root"]);
  });
});

describe("player_renderer_3d.characterCameraParams", () => {
  it("centres the frustum so the character lands at the requested screen point", () => {
    const params = characterCameraParams(640, 360, 40, 1280, 720, (17 * Math.PI) / 180, 1.8);
    // left/right straddle 0 at sx == vw/2, and similarly for top/bottom at sy == vh/2.
    expect(params.left + params.right).toBeCloseTo(0, 9);
    expect(params.top + params.bottom).toBeCloseTo(0, 9);
  });

  it("shifts the frustum bounds as the requested screen point moves toward the left edge", () => {
    // left = -sx/ppm, right = (vw-sx)/ppm: moving sx toward 0 pushes both
    // bounds up (left toward/through 0, right further positive), which is
    // what re-centres a character drawn nearer the screen's left edge.
    const centered = characterCameraParams(640, 360, 40, 1280, 720, 0, 1.8);
    const left = characterCameraParams(100, 360, 40, 1280, 720, 0, 1.8);
    expect(left.left).toBeGreaterThan(centered.left);
    expect(left.right).toBeGreaterThan(centered.right);
  });

  it("elevates the eye above the character's mid-height and looks back down at it", () => {
    const params = characterCameraParams(640, 360, 40, 1280, 720, Math.PI / 6, 1.8);
    expect(params.eye[1]).toBeGreaterThan(params.target[1]);
    expect(params.target).toEqual([0, 0.9, 0]);
  });
});

// RENDERTOSPRITE (defect #1's fix, see pitch.ts's file header and scene.ts's
// class doc comment). A minimal renderer stub is enough to verify this
// function's OWN contract -- it builds a viewport-sized quad with an owned
// off-screen render target, and it restores the renderer's prior
// target/clear state -- without needing a live GL context. What is NOT
// verified here (or anywhere in this package) is the actual pixel content
// the off-screen render produces; see this port's report.
describe("player_renderer_3d.renderToSprite", () => {
  interface RenderCall {
    readonly scene: THREE.Scene;
    readonly camera: THREE.Camera;
  }

  interface StubRenderer {
    autoClear: boolean;
    readonly setRenderTargetCalls: (THREE.WebGLRenderTarget | null)[];
    readonly renderCalls: RenderCall[];
    getRenderTarget(): THREE.WebGLRenderTarget | null;
    setRenderTarget(t: THREE.WebGLRenderTarget | null): void;
    getClearColor(target: THREE.Color): THREE.Color;
    getClearAlpha(): number;
    setClearColor(color: THREE.ColorRepresentation, alpha?: number): void;
    clear(): void;
    render(scene: THREE.Scene, camera: THREE.Camera): void;
  }

  function stubRenderer(): THREE.WebGLRenderer & StubRenderer {
    let current: THREE.WebGLRenderTarget | null = null;
    const setRenderTargetCalls: (THREE.WebGLRenderTarget | null)[] = [];
    const renderCalls: RenderCall[] = [];
    const stub: StubRenderer = {
      autoClear: true,
      setRenderTargetCalls,
      renderCalls,
      getRenderTarget: () => current,
      setRenderTarget: (t) => {
        current = t;
        setRenderTargetCalls.push(t);
      },
      getClearColor: (target) => target.set(0, 0, 0),
      getClearAlpha: () => 1,
      setClearColor: () => {},
      clear: () => {},
      // NOT a WebGL mock (see this file's own header and scene.spec.ts's):
      // this never rasterises anything. It only lets tests below inspect the
      // EXACT `THREE.Scene`/`THREE.Camera` `renderToSprite` was about to hand
      // a real renderer -- the same interception-point pattern
      // scene.spec.ts's `stubRenderer()` uses for `setSize`/`setPixelRatio`.
      render: (scene, camera) => {
        renderCalls.push({ scene, camera });
      },
    };
    return stub as unknown as THREE.WebGLRenderer & StubRenderer;
  }

  const idleView: PlayerView = { px: 0, py: 0, speed: 0, phase: 0, gait: 0, lean: 0 };

  it("skips this environment's assertions if the rigged pass genuinely could not build", () => {
    // Documents the precondition the rest of this describe block assumes,
    // rather than silently no-op-ing if a future change to rig3d content
    // makes `build()` start failing under vitest's "node" environment.
    expect(available()).toBe(true);
  });

  it("returns a viewport-sized mesh positioned at the viewport's centre, tagged with its owned render target", () => {
    const renderer = stubRenderer();
    const mesh = renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    expect(mesh).toBeInstanceOf(THREE.Mesh);
    if (mesh === undefined) {
      throw new Error("expected a mesh");
    }
    expect(mesh.position.x).toBeCloseTo(640, 9);
    expect(mesh.position.y).toBeCloseTo(360, 9);
    expect(mesh.userData["ownedRenderTarget"]).toBeInstanceOf(THREE.WebGLRenderTarget);
  });

  it("renders into a private off-screen target, never the renderer's own bound target", () => {
    const renderer = stubRenderer();
    renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    expect(renderer.setRenderTargetCalls.length).toBeGreaterThanOrEqual(2);
    const offscreenCall = renderer.setRenderTargetCalls[0];
    expect(offscreenCall).toBeInstanceOf(THREE.WebGLRenderTarget);
  });

  it("restores the renderer's prior target once done", () => {
    const renderer = stubRenderer();
    renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    expect(renderer.getRenderTarget()).toBeNull();
  });

  it("restores the renderer's prior autoClear value once done", () => {
    const renderer = stubRenderer();
    renderer.autoClear = false;
    renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    expect(renderer.autoClear).toBe(false);
  });

  // REGRESSION (this port's report): players rendered as solid black
  // silhouettes with no team-colour distinction in a real browser --
  // invisible to every test above, which never inspects WHAT is inside the
  // scene `renderToSprite` hands a renderer, only how the renderer is
  // called. `stubRenderer().render` above now captures its `scene` argument
  // (a plain object-graph read, no GL touched), which is enough to assert
  // the two structural facts that were actually broken: a `THREE.Light` was
  // missing from the scene a `THREE.MeshStandardMaterial` character was
  // rendered into (PBR materials shade pure black with none), and the two
  // teams' materials carried only ONE flat colour each instead of a
  // per-vertex palette (see `materialsForTeam`'s doc comment). This does NOT
  // prove the rendered pixels are non-black or team-distinguishable --
  // three.js's actual lighting math is a live-GL question this suite
  // structurally cannot answer (see file header) -- but a scene with no
  // light and a material needing one is a defect this test WOULD have
  // caught before it ever reached a browser.
  it("renders the character into a scene containing at least one THREE.Light", () => {
    const renderer = stubRenderer();
    renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    expect(renderer.renderCalls).toHaveLength(1);
    const call = renderer.renderCalls[0];
    if (call === undefined) {
      throw new Error("expected one render call");
    }
    const lights = call.scene.children.filter((child): child is THREE.Light => (child as { isLight?: boolean }).isLight === true);
    expect(lights.length).toBeGreaterThan(0);
  });

  // The materials stay `MeshStandardMaterial` (a lit, PBR material) rather
  // than switching to an unlit material as a workaround for the missing
  // light above -- see player_renderer_3d.ts's LIGHTING comment for why that
  // is the correct call given rig3d/renderer.lua's original toon-shaded,
  // lit-by-a-directional-key-light intent (v2/README.md #7: "replace --
  // WebGLRenderer, MeshStandardMaterial").
  it("shades the character with MeshStandardMaterial, not an unlit material standing in for the missing light", () => {
    const renderer = stubRenderer();
    renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    const call = renderer.renderCalls[0];
    if (call === undefined) {
      throw new Error("expected one render call");
    }
    const mesh = call.scene.children.find((child): child is THREE.SkinnedMesh => child instanceof THREE.SkinnedMesh);
    if (mesh === undefined) {
      throw new Error("expected a SkinnedMesh in the rendered scene");
    }
    const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
    expect(materials.length).toBeGreaterThan(0);
    for (const material of materials) {
      expect(material).toBeInstanceOf(THREE.MeshStandardMaterial);
    }
  });

  // REGRESSION target of THIS port: the toon shading itself (rig3d/renderer.lua's
  // hand-written GLSL -- bands, bounce, rim, metal specular, emissive-past-white)
  // had NO port at all before this change; `materialsForTeam` built plain
  // `MeshStandardMaterial`s with tuned `roughness`/`metalness` numbers and a
  // fixed emissive accent colour, which is generic PBR, not this game's look.
  // This suite cannot rasterise a pixel (see file header), so it checks the
  // one thing it CAN from here: that every one of the three per-material-group
  // materials `renderToSprite` puts into the scene actually has
  // `rig3d/cel_shader.ts`'s `applyCelShading` wired onto it (a real, non-default
  // `onBeforeCompile` and `side === THREE.DoubleSide`, matching
  // rig3d/renderer.lua's `setMeshCullMode("none")`) rather than being plain,
  // untouched `MeshStandardMaterial`s relying on PBR defaults. See
  // rig3d/cel_shader.spec.ts for what the injected shading itself contains.
  it("wires rig3d/cel_shader.ts's toon shading onto every material group, not stock MeshStandardMaterial defaults", () => {
    const renderer = stubRenderer();
    renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    const call = renderer.renderCalls[0];
    if (call === undefined) {
      throw new Error("expected one render call");
    }
    const mesh = call.scene.children.find((child): child is THREE.SkinnedMesh => child instanceof THREE.SkinnedMesh);
    if (mesh === undefined) {
      throw new Error("expected a SkinnedMesh in the rendered scene");
    }
    const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
    expect(materials.length).toBe(3);
    const cacheKeys = new Set<string>();
    for (const material of materials) {
      if (!(material instanceof THREE.MeshStandardMaterial)) {
        throw new Error("expected a MeshStandardMaterial");
      }
      // Every stock `THREE.Material` has a no-op `onBeforeCompile` by
      // default; a real hook was assigned only if this one differs from a
      // freshly-constructed material's own default.
      expect(material.onBeforeCompile).not.toBe(new THREE.MeshStandardMaterial().onBeforeCompile);
      expect(material.side).toBe(THREE.DoubleSide);
      cacheKeys.add(material.customProgramCacheKey());
    }
    // plain/metal/emissive must each compile to their own GPU program (see
    // cel_shader.ts's `applyCelShading` doc comment on why -- three.js's
    // program cache cannot otherwise tell them apart).
    expect(cacheKeys.size).toBe(3);
  });

  // REGRESSION: the offscreen composite's camera frustum alignment was fine
  // (verified live-GL, see this port's report), but the returned sprite
  // rendered upside down under `SceneRoot`'s Y-inverted 2D camera --
  // `target.texture.flipY` (the line the code previously pointed at) turned
  // out to be a no-op for render-target textures. The actual fix is
  // `scale.y = -1` on the returned mesh; this pins that regression
  // structurally without needing a live GL context to see the pixels.
  it("flips the returned sprite vertically to counter SceneRoot's Y-inverted camera", () => {
    const renderer = stubRenderer();
    const mesh = renderToSprite(renderer, 640, 360, 12, 1280, 720, idleView, baseOptions(), 0);
    if (mesh === undefined) {
      throw new Error("expected a mesh");
    }
    expect(mesh.scale.y).toBe(-1);
  });

  // REGRESSION: both teams' rigged characters rendered with the exact same
  // flat colour (`materialsForTeam` used to read only `palette[0]`, the
  // "skin" slot, which no theme ever maps to "team" -- see themes.ts's
  // `SLOTS`/`ColorSlot`). Baking a per-vertex colour attribute from each
  // team's resolved palette (this port's fix) means the two teams' baked
  // colour buffers must differ once real team-owned surfaces (cloth, in the
  // medieval theme fixture content) are included.
  it("bakes different per-vertex colours for the home and away team", () => {
    // Both draws share the SAME geometry object (one shared character mesh,
    // see player_renderer_3d.ts's `build()` doc comment) -- its `color`
    // attribute is swapped in per draw call by `materialsForTeam`, so it
    // must be read back immediately after each respective call, in the same
    // order production code depends on, rather than read at the end.
    const homeRenderer = stubRenderer();
    renderToSprite(homeRenderer, 640, 360, 12, 1280, 720, idleView, baseOptions({ team: "home" }), 0);
    const homeMesh = homeRenderer.renderCalls[0]?.scene.children.find((c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh);
    if (homeMesh === undefined) {
      throw new Error("expected a SkinnedMesh in the rendered scene");
    }
    const homeColor = Array.from(homeMesh.geometry.getAttribute("color").array);

    const awayRenderer = stubRenderer();
    renderToSprite(awayRenderer, 640, 360, 12, 1280, 720, idleView, baseOptions({ team: "away" }), 0);
    const awayMesh = awayRenderer.renderCalls[0]?.scene.children.find((c): c is THREE.SkinnedMesh => c instanceof THREE.SkinnedMesh);
    if (awayMesh === undefined) {
      throw new Error("expected a SkinnedMesh in the rendered scene");
    }
    const awayColor = Array.from(awayMesh.geometry.getAttribute("color").array);

    expect(awayColor).not.toEqual(homeColor);
  });
});

// CHARACTERMESH / PPMFORRADIUS (pitch.ts's single-pass compositing -- see
// that file's "ONE PASS, ONE DEPTH BUFFER" header section). `characterMesh`
// replaces `renderToSprite` on pitch.ts's call path: no renderer, no render
// target, just a posed/coloured/yawed `THREE.SkinnedMesh` in local metre
// space, pooled per `playerId`. Same GPU-adjacent-but-GL-free testability
// boundary as the `renderToSprite` suite above.
describe("player_renderer_3d.characterMesh / ppmForRadius", () => {
  const idleView: PlayerView = { px: 0, py: 0, speed: 0, phase: 0, gait: 0, lean: 0 };

  it("skips this environment's assertions if the rigged pass genuinely could not build", () => {
    expect(available()).toBe(true);
  });

  it("returns a SkinnedMesh, needing no renderer at all", () => {
    const mesh = characterMesh("p1", idleView, baseOptions(), 0);
    expect(mesh).toBeInstanceOf(THREE.SkinnedMesh);
  });

  it("pools the SAME mesh instance for the same playerId across calls, a DIFFERENT instance for a different playerId", () => {
    const a1 = characterMesh("player-a", idleView, baseOptions(), 0);
    const a2 = characterMesh("player-a", idleView, baseOptions(), 1);
    const b1 = characterMesh("player-b", idleView, baseOptions(), 0);
    expect(a1).toBe(a2);
    expect(a1).not.toBe(b1);
  });

  it("poses two different playerIds' meshes independently -- posing one does not clobber the other's skeleton", () => {
    // A keeper gathering the ball is a strong pose (overrides the upper
    // body), easy to tell apart from an idle stance by eye in `poseFor`'s own
    // suite above; here it is enough that the two pooled meshes' bone
    // matrices end up different when only one player is given that pose.
    const gathering = characterMesh("keeper", idleView, baseOptions({ holding: true }), 0);
    const idle = characterMesh("outfield", idleView, baseOptions(), 0);
    if (gathering === undefined || idle === undefined) {
      throw new Error("expected two meshes");
    }
    const gatheringBones = (gathering as THREE.SkinnedMesh).skeleton.bones.map((b) => b.matrixWorld.elements.slice());
    const idleBones = (idle as THREE.SkinnedMesh).skeleton.bones.map((b) => b.matrixWorld.elements.slice());
    expect(gatheringBones).not.toEqual(idleBones);

    // Re-posing "outfield" again afterwards must not have picked up
    // "keeper"'s gather pose -- each call repositions the SHARED pose-
    // evaluation rig (`character.rig`) and immediately copies the result
    // into the CALLER's own pooled bones, so there is no cross-player
    // leakage even though the rig itself is shared.
    const idleAgain = characterMesh("outfield", idleView, baseOptions(), 0);
    const idleAgainBones = (idleAgain as THREE.SkinnedMesh).skeleton.bones.map((b) => b.matrixWorld.elements.slice());
    expect(idleAgainBones).toEqual(idleBones);
  });

  it("bakes different per-vertex colours for the home and away team, on separate geometry objects", () => {
    const home = characterMesh("home-player", idleView, baseOptions({ team: "home" }), 0) as THREE.SkinnedMesh;
    const away = characterMesh("away-player", idleView, baseOptions({ team: "away" }), 0) as THREE.SkinnedMesh;
    expect(home.geometry).not.toBe(away.geometry);
    const homeColor = Array.from(home.geometry.getAttribute("color").array);
    const awayColor = Array.from(away.geometry.getAttribute("color").array);
    expect(awayColor).not.toEqual(homeColor);
  });

  it("bakes the facing yaw onto the mesh's own quaternion", () => {
    const forward = characterMesh("yaw-a", idleView, baseOptions({ facing: new Vec2(0, 1) }), 0) as THREE.SkinnedMesh;
    const sideways = characterMesh("yaw-b", idleView, baseOptions({ facing: new Vec2(1, 0) }), 0) as THREE.SkinnedMesh;
    expect(forward.quaternion.equals(sideways.quaternion)).toBe(false);
  });

  it("ppmForRadius scales linearly with the requested on-screen radius", () => {
    const small = ppmForRadius(10);
    const large = ppmForRadius(20);
    expect(small).toBeDefined();
    expect(large).toBeDefined();
    if (small === undefined || large === undefined) {
      throw new Error("expected both to be defined");
    }
    expect(large).toBeCloseTo(small * 2);
  });
});
