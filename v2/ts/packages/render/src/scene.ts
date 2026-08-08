// New shared infrastructure, not a port of one Lua file (v2/README.md #7 --
// the app-shell/glue milestone this file belongs to, not a game/render/*.lua
// module).
//
// Every other file in this package draws content -- pitch.ts assembles arena
// + players + effects + combat into one `THREE.Group`, match_hud.ts paints
// the HUD into another -- but nothing owned a `THREE.Scene` or a
// `THREE.WebGLRenderer`, and nothing assembled the pieces into an actual
// rendered frame. This file is that missing root.
//
// TESTABILITY BOUNDARY. pitch.ts's `draw`, player_renderer_3d.ts's `draw`
// and bloom.ts's `draw` all take an ALREADY-CONSTRUCTED `THREE.WebGLRenderer`
// as a parameter rather than building one -- each file's header says this is
// deliberate, so the pure content-building half of each module stays
// unit-testable without a GL context. `SceneRoot` preserves that: it takes
// its `THREE.WebGLRenderer` via the constructor (dependency injection, the
// same convention, just one level up the call stack) instead of constructing
// one itself from a `<canvas>`. Actually building that canvas and renderer is
// `game/`'s job -- the app-shell glue v2/README.md #1 explicitly scopes out
// of this milestone ("wasm bindings, the JS<->wasm marshalling layer, asset
// pipeline, bundling, a running app"). From the point a renderer is handed
// in, `SceneRoot` OWNS its lifecycle: stored once, resized on `resize()`,
// passed down to every `draw` call, released on `dispose()` -- never
// reconstructed.
//
// Constructing `THREE.Scene` / `THREE.Group` / `THREE.OrthographicCamera` /
// `Bloom` touches no GL at all (they are plain JS object graphs until
// something calls `renderer.render()`), so assembly order, resize math and
// disposal are exercised headless in scene.spec.ts. `render()` itself is
// checked only for the one thing about it that does NOT need a live GL
// context to verify: that pitch.ts's static-scene memoisation still holds
// across repeated frames through this class (see pitch.ts's own
// `staticSceneBuildCount`). Its actual pixel output is not -- see
// scene.spec.ts's header and this port's report.

import * as THREE from "three";
import { pitch, type PitchDrawOptions, type PitchViewport, type RenderFrame } from "./pitch.ts";
import { Bloom, type BloomConfig } from "./bloom.ts";
import { camera, perspectiveRig, type CameraField } from "./camera.ts";
import { cameraFollow } from "./camera_follow.ts";
import { drawMatchHud, type MatchHudLayout, type MatchHudModel, type MatchHudTheme, type MatchHudViewport } from "./match_hud.ts";
import { disposeObject } from "./draw2d.ts";

/** `{w, h}` in pixels. Shared shape with `pitch.ts`'s `PitchViewport`. */
export type SceneViewport = PitchViewport;

/** The product HUD's already-computed model/layout/theme (see match_hud.ts's
 * header -- none of these are produced by `RenderFrame`, so they are threaded
 * through as an explicit, optional parameter rather than derived here). */
export interface SceneHud {
  readonly model: MatchHudModel;
  readonly layout: MatchHudLayout;
  readonly theme: MatchHudTheme;
}

/** Per-frame input to `SceneRoot.render`, beyond the `RenderFrame` itself. */
export interface SceneRenderOptions {
  readonly pitch: PitchDrawOptions;
  /** Omit to draw no HUD this frame (e.g. a menu behind the pitch). */
  readonly hud?: SceneHud;
  /** Seconds, replacing `love.timer.getTime()` -- see pitch.ts/player_renderer_3d.ts. */
  readonly now?: number;
}

/** Construction-time configuration for `SceneRoot`. */
export interface SceneRootOptions {
  readonly viewport: SceneViewport;
  readonly bloom?: Partial<BloomConfig>;
  readonly pixelRatio?: number;
}

/**
 * A world-space 3D layer (the stadium: bowl, crowd, sky, whatever sits
 * behind/around the flat pitch content) that `SceneRoot` can composite
 * UNDER `pitchGroup`/`hudGroup` when `camera.perspective_mode` is on. Kept
 * deliberately minimal and content-agnostic -- `SceneRoot` never constructs
 * or names anything inside `group`, it only adds/removes the group itself
 * and calls `update`/`dispose` -- so this file has no dependency on whatever
 * builds the actual stadium geometry (a `stadium*.ts` module elsewhere in
 * this package, out of this file's scope).
 */
export interface WorldLayer {
  /** Added to `worldScene` by `setWorldLayer`; never mutated by `SceneRoot` itself. */
  readonly group: THREE.Group;
  /** Called once per `populate()` (i.e. once per frame), before rasterizing. */
  update(now: number): void;
  /** Releases whatever GPU-side resources `group`'s subtree owns. See `setWorldLayer`'s and `dispose`'s own doc comments for exactly when `SceneRoot` calls this. */
  dispose(): void;
}

// HUD paints after (visually on top of) the pitch regardless of the z=0 tie
// every draw2d.ts command shares (see draw2d.ts's `paint` doc comment) --
// `renderOrder` is three.js's own mechanism for that, and does not depend on
// child-array order the way relying on `scene.add` sequence alone would.
const PITCH_RENDER_ORDER = 0;
const HUD_RENDER_ORDER = 1;

// draw2d.ts's `paint` puts every command at z=0; near/far just need to
// bracket that with margin. Exact values do not matter beyond "z=0 is
// visible" since nothing in this package's screen-space content varies in Z.
const CAMERA_NEAR = 0.1;
const CAMERA_FAR = 10;

/**
 * Owns the `THREE.Scene` / `THREE.WebGLRenderer` this package's leaf draw
 * functions take by injection, and assembles them into one rendered frame.
 *
 * ASSEMBLY ORDER, and why:
 *
 *   1. `pitchGroup` -- arena backdrop/frame, the projected floor, markings
 *      and goals, the depth-sorted players (billboard or rigged), effects
 *      (ball trail, bursts) and combat telegraphs. This is `pitch.draw`'s
 *      OWN composition (see pitch.ts's file header): pitch.ts already wires
 *      together arena.ts, player_renderer.ts/player_renderer_3d.ts,
 *      effects.ts and combat.ts in a specific, documented order. `SceneRoot`
 *      therefore calls pitch.draw -- the one existing orchestrator -- rather
 *      than re-deriving that order by calling each of those modules itself.
 *   2. `hudGroup` -- the product HUD (match_hud.ts), added to the SAME
 *      scene but at `HUD_RENDER_ORDER > PITCH_RENDER_ORDER` so it paints on
 *      top of `pitchGroup`.
 *   3. Bloom composites the WHOLE scene (`pitchGroup` + `hudGroup`) in one
 *      pass via `bloom.ts`'s `Bloom.draw`. DELIBERATE SIMPLIFICATION: three.js
 *      has no built-in *selective* bloom (excluding `hudGroup` would need a
 *      second render pass keyed off `object.layers`, whose correctness is
 *      exactly the kind of thing that needs a live GL context to verify --
 *      the same tradeoff pitch.ts's own SCOPE NOTE makes for the rigged-
 *      player composite, see below). `UnrealBloomPass` only blooms pixels at
 *      or above `config.threshold` (0.55 by default), so most HUD panel
 *      fills do not visibly glow -- flagged here as a conscious choice
 *      rather than left as a silent side effect.
 *
 * FIXED HERE (was a known gap, #1): `pitch.draw`'s rigged-player pass used
 * to composite each character by calling `renderer.render()` DIRECTLY and
 * IMMEDIATELY, rather than adding the character to `pitchGroup`'s `Object3D`
 * graph -- `SceneRoot.render`'s own later full-scene render would then clear
 * the canvas and wipe out whatever `pitch.draw` had just painted onto it.
 * Intermediate fix (superseded, see next paragraph): each rigged character
 * rendered to a private OFFSCREEN `THREE.WebGLRenderTarget`, composited as a
 * viewport-sized textured quad added to `pitchGroup` at the right point in
 * painter's-algorithm order. VERIFIED WITH A LIVE GL CONTEXT at the time: the
 * character camera's asymmetric frustum (`player_renderer_3d.ts`'s
 * `characterCameraParams`) landed the character at the right `(sx, sy)`, and
 * texture orientation needed `scale.y = -1` on the quad (`target.texture
 * .flipY` is a no-op for render-target textures).
 *
 * FIXED HERE (#2, superseding #1's offscreen-quad mechanism): that quad was
 * still one FULL-VIEWPORT `renderer.render()` call per rigged character per
 * frame, composited with `depthTest: false` -- correct painter's order via
 * `pitchGroup`'s child array position, but not per-pixel occlusion, and N
 * render passes where the goal was one. Every rigged character now joins
 * this class's own `scene` as an actual depth-tested `THREE.SkinnedMesh`
 * (`player_renderer_3d.ts`'s `characterMesh`, pooled per player, wrapped by
 * `pitch.ts`'s `riggedCharacterObject`), drawn in this class's ONE existing
 * full-scene `render()` call alongside everything else -- no per-character
 * render pass, no render target, at all. `player_renderer_3d.ts`'s old
 * `renderToSprite` (the #1 mechanism) is kept, unchanged, for parity/
 * diagnostics, but is no longer on `pitch.draw`'s call path -- see that
 * file's and pitch.ts's own headers ("ONE PASS, ONE DEPTH BUFFER") for the
 * exact composition (screen-space position/scale, the elevation tilt, and
 * this class's own Y-inverted camera convention -- still needing an
 * equivalent of `scale.y = -1`, now on `riggedCharacterObject`'s wrapper
 * instead of a compositing plane). The one, single render that touches the
 * visible canvas/composite chain is still this class's own `render` below,
 * for a WHOLE frame -- flat content and every rigged character together --
 * rather than N-plus-one.
 *
 * The camera is one shared `THREE.OrthographicCamera` sized to the
 * viewport, `top = 0` / `bottom = viewport.h` -- matching draw2d.ts's
 * `paint` doc comment: every draw2d.ts command is already in screen space
 * with y increasing downward, so the frustum is set up to match that
 * convention directly instead of flipping content.
 *
 * `render` (the full frame, bloom included) is split from `populate` (just
 * the object-graph assembly, no rasterization) precisely so the assembly
 * half can be exercised without a live GL context -- see `populate`'s own
 * doc comment.
 *
 * WORLD LAYER (the stadium). `worldScene`/`worldCamera` are a SECOND,
 * independent scene graph, always constructed (see the constructor) but
 * only ever rendered through when BOTH a `WorldLayer` is attached (via
 * `setWorldLayer`) AND `camera.perspective_mode` is on -- see `populate`'s
 * and `render`'s own doc comments for exactly what each half does. Kept
 * entirely separate from `scene`/`camera` (the existing screen-space
 * ortho scene every draw2d.ts command already lives in) rather than added
 * as a third child group of the SAME scene, because it is not
 * screen-space content at all: it is real, metre-scaled 3D geometry seen
 * through a real `THREE.PerspectiveCamera`, and mixing that into the
 * ortho scene's single shared camera the way `pitchGroup`/`hudGroup` share
 * one would mean every world-layer object also has to be reprojected into
 * screen-pixel coordinates by hand, which is exactly the "camera per
 * subject" complexity `pitch.ts`'s "ONE PASS, ONE DEPTH BUFFER" section
 * fought hard to get away from for rigged characters. `bloom.ts`'s
 * `draw_layers` (not `draw`) is what reconciles the two into one
 * composite when both are active -- see `render`'s doc comment.
 */
export class SceneRoot {
  readonly scene: THREE.Scene;
  readonly camera: THREE.OrthographicCamera;
  readonly pitchGroup: THREE.Group;
  readonly hudGroup: THREE.Group;
  /** See the class doc comment's WORLD LAYER section. */
  readonly worldScene: THREE.Scene;
  /** Synced from `camera.perspectiveRig` every `populate()` call -- see that method. */
  readonly worldCamera: THREE.PerspectiveCamera;

  private readonly renderer: THREE.WebGLRenderer;
  private readonly bloomPass: Bloom;
  private viewport: SceneViewport;
  private worldLayer: WorldLayer | undefined;
  private disposed = false;

  constructor(renderer: THREE.WebGLRenderer, options: SceneRootOptions) {
    this.renderer = renderer;
    this.viewport = options.viewport;

    this.scene = new THREE.Scene();
    this.pitchGroup = new THREE.Group();
    this.pitchGroup.name = "pitch";
    this.pitchGroup.renderOrder = PITCH_RENDER_ORDER;
    this.hudGroup = new THREE.Group();
    this.hudGroup.name = "hud";
    this.hudGroup.renderOrder = HUD_RENDER_ORDER;
    this.scene.add(this.pitchGroup, this.hudGroup);

    this.camera = new THREE.OrthographicCamera(0, options.viewport.w, 0, options.viewport.h, CAMERA_NEAR, CAMERA_FAR);
    this.camera.position.set(0, 0, 1);
    this.camera.lookAt(0, 0, 0);

    // Always constructed -- see the class doc comment's WORLD LAYER section
    // -- but unused (never added as a `draw_layers` layer) until BOTH a
    // `WorldLayer` is attached and `camera.perspective_mode` is on. The
    // initial fov/near/far mirror `camera.PERSPECTIVE`/`perspectiveRig`'s
    // own defaults so this camera is never left in a nonsensical state
    // before the first `populate()` call re-syncs it properly.
    this.worldScene = new THREE.Scene();
    this.worldCamera = new THREE.PerspectiveCamera(camera.PERSPECTIVE.fov, options.viewport.w / options.viewport.h, 1, 8000);

    this.bloomPass = new Bloom(options.bloom ?? {});

    this.renderer.setPixelRatio(options.pixelRatio ?? 1);
    this.renderer.setSize(options.viewport.w, options.viewport.h, false);
  }

  /**
   * Attaches (or, with `undefined`, detaches) a world layer: adds/removes
   * `layer.group` from `worldScene`. OWNERSHIP: `SceneRoot` does NOT call
   * `layer.dispose()` here -- detaching via `setWorldLayer(undefined)` (or
   * swapping to a different layer) hands the OLD layer back to its owner
   * un-disposed, the same way `SceneRoot`'s constructor-injected `renderer`
   * itself is owned by whoever constructed it, not by every class it is
   * threaded through. The one place `SceneRoot` DOES call `dispose()` on a
   * still-attached layer is its own `dispose()` (see that method) -- at
   * teardown there is no later `setWorldLayer` call coming to hand
   * ownership back, so leaving it undisposed there would leak.
   */
  setWorldLayer(layer: WorldLayer | undefined): void {
    this.assertNotDisposed();
    if (this.worldLayer !== undefined) {
      this.worldScene.remove(this.worldLayer.group);
    }
    this.worldLayer = layer;
    if (layer !== undefined) {
      this.worldScene.add(layer.group);
    }
  }

  /** The viewport `resize` last set. Exposed so a test can assert `resize` took effect. */
  getViewport(): SceneViewport {
    return this.viewport;
  }

  /**
   * Resize the owned renderer and recompute the camera frustum(s). Reuses
   * the existing `camera`/`worldCamera`/`scene`/groups/renderer -- none of
   * them are reconstructed. `worldCamera`'s ASPECT is updated here (a
   * window resize changes it regardless of whether a world layer or
   * `perspective_mode` is currently active); its position/fov/near/far stay
   * `populate`'s job, since those depend on the FRAME (the follow-camera
   * view, in particular) rather than the viewport.
   */
  resize(viewport: SceneViewport): void {
    this.assertNotDisposed();
    this.viewport = viewport;
    this.camera.left = 0;
    this.camera.right = viewport.w;
    this.camera.top = 0;
    this.camera.bottom = viewport.h;
    this.camera.updateProjectionMatrix();
    this.worldCamera.aspect = viewport.w / viewport.h;
    this.worldCamera.updateProjectionMatrix();
    this.renderer.setSize(viewport.w, viewport.h, false);
  }

  /**
   * Populate `pitchGroup`/`hudGroup` for one frame from `frame`/`options`,
   * WITHOUT rasterizing anything -- no `Bloom`/composite call happens here.
   *
   * Split out from `render` specifically so assembly is exercisable headless
   * (see scene.spec.ts): unlike `render`, this method never touches
   * `Bloom.draw`, whose `EffectComposer` needs a much larger slice of the
   * real `THREE.WebGLRenderer` surface (clear color, render targets, ...)
   * than anything else in this class does -- exercising that without a live
   * GL context would mean faking a WebGL-shaped renderer, which is the exact
   * thing this milestone's tests are told not to do. `render`'s pixel output
   * is therefore untested; `populate`'s effect on the object graph is.
   *
   * `renderer.autoClear` is restored to whatever the caller had it set to
   * once `pitch.draw` returns; it is only forced `false` for the duration of
   * that call. `pitch.draw`'s rigged pass no longer depends on this the way
   * `player_renderer_3d.ts`'s old direct-render `draw` did (see that file's
   * doc comment and this class's own "FIXED HERE" notes) -- `pitch.draw`
   * doesn't rasterize anything at all anymore (rigged or not): it only
   * builds `pitchGroup`'s object graph (`characterMesh`/`riggedCharacterObject`
   * for rigged players, `paint`/`appendCommands` for everything else), and
   * this class's own later `render()` is the one place that ever calls
   * `renderer.render()`. `autoClear` is still forced off here defensively,
   * to avoid depending on `SceneRoot` being the only caller that ever sets
   * it, even though nothing in `populate` currently rasterizes.
   *
   * WORLD LAYER SYNC. When a `WorldLayer` is attached AND
   * `camera.perspective_mode` is on, this method ALSO (a) calls
   * `layer.update(now)` -- the same `now` `pitch.draw` gets, so both halves
   * of one frame agree on the clock -- and (b) re-derives `worldCamera`'s
   * position/orientation/lens from `camera.perspectiveRig`, using the EXACT
   * SAME `view` (`pitch.follow_camera ? cameraFollow.view(frame.field) :
   * undefined`) `pitch.ts`'s own `pitchProject` computes internally. This is
   * not incidental duplication: importing `camera`/`cameraFollow` directly
   * here (rather than, say, asking `pitch.ts` to hand back whatever view it
   * used) means both this file and pitch.ts derive the view from the SAME
   * two flags (`pitch.follow_camera`, and the `cameraFollow` module's own
   * frame-to-frame state) through the SAME two calls -- there is no third
   * copy of "how do I get the current view" to drift out of sync with
   * either. If `worldCamera`'s eye ever disagreed with the screen-space
   * projection's own implied eye, the world-space stadium would visibly
   * swim relative to the flat pitch/players drawn over it the moment the
   * follow camera moved. Neither half is rasterized here -- see `render`.
   */
  populate(frame: RenderFrame, options: SceneRenderOptions): void {
    this.assertNotDisposed();
    const now = options.now ?? 0;

    const wasAutoClear = this.renderer.autoClear;
    this.renderer.autoClear = false;
    try {
      pitch.draw(this.pitchGroup, frame, this.viewport, options.pitch, this.renderer, now);
    } finally {
      this.renderer.autoClear = wasAutoClear;
    }

    if (options.hud !== undefined) {
      const hudViewport: MatchHudViewport = this.viewport;
      drawMatchHud(this.hudGroup, options.hud.model, options.hud.layout, options.hud.theme, hudViewport);
    } else {
      this.clearGroup(this.hudGroup);
    }

    if (this.worldLayer !== undefined && camera.perspective_mode) {
      this.worldLayer.update(now);
      this.syncWorldCamera(frame.field);
    }
  }

  // See `populate`'s WORLD LAYER SYNC section for why this reads
  // `pitch.follow_camera`/`cameraFollow` directly instead of taking a `view`
  // parameter from the caller.
  private syncWorldCamera(field: CameraField): void {
    const view = pitch.follow_camera ? cameraFollow.view(field) : undefined;
    const rig = perspectiveRig(field, view);
    this.worldCamera.position.set(rig.eye[0], rig.eye[1], rig.eye[2]);
    // `mat4.lookAt` (camera.ts's own hand-rolled path) always builds its
    // basis off a world-up of (0, 1, 0) -- `THREE.Camera.up`'s own default,
    // left untouched here rather than reset explicitly, so both projections
    // agree without either one special-casing a roll. See camera.ts's
    // `projectPerspective` doc comment for the matching note on that side.
    this.worldCamera.lookAt(rig.target[0], rig.target[1], rig.target[2]);
    this.worldCamera.fov = rig.fov;
    this.worldCamera.near = rig.near;
    this.worldCamera.far = rig.far;
    this.worldCamera.aspect = this.viewport.w / this.viewport.h;
    this.worldCamera.updateProjectionMatrix();
  }

  /**
   * Render one frame: `populate` (see above), then rasterize the assembled
   * scene(s) through bloom. Needs a live GL context to run at all -- see
   * `populate`'s doc comment -- so it is not exercised by scene.spec.ts.
   *
   * With an active world layer under `camera.perspective_mode`, this uses
   * `bloom.ts`'s `draw_layers` with the world layer FIRST (so it composites
   * underneath, per that method's clear semantics -- see bloom.ts) and this
   * class's own screen-space `scene`/`camera` second: `[{ scene:
   * worldScene, camera: worldCamera }, { scene, camera }]`. Otherwise --
   * no layer, or `perspective_mode` off -- this is EXACTLY today's
   * single-layer path (`Bloom.draw`), bit-identical to before this world-
   * layer support existed: no behaviour change for the default, flags-off
   * configuration.
   */
  render(frame: RenderFrame, options: SceneRenderOptions): void {
    this.assertNotDisposed();
    this.populate(frame, options);
    if (this.worldLayer !== undefined && camera.perspective_mode) {
      this.bloomPass.draw_layers(
        this.renderer,
        [
          { scene: this.worldScene, camera: this.worldCamera },
          { scene: this.scene, camera: this.camera },
        ],
        this.viewport.w,
        this.viewport.h,
      );
    } else {
      this.bloomPass.draw(this.renderer, this.scene, this.camera, this.viewport.w, this.viewport.h);
    }
  }

  /**
   * Release everything this class owns: group children's geometries/
   * materials/textures/owned render targets (via draw2d.ts's `disposeObject`
   * -- the same function `paint`'s own per-frame cleanup uses, so the two
   * never drift apart the way two independent `instanceof` checks could),
   * the groups themselves, `Bloom`'s `EffectComposer` and its render
   * targets, and the renderer. Idempotent -- a second call is a no-op rather
   * than an error, since callers commonly `dispose()` from more than one
   * teardown path.
   *
   * WORLD LAYER: if one is still attached, this is the one place `SceneRoot`
   * calls `layer.dispose()` itself -- see `setWorldLayer`'s doc comment for
   * why that call does not live there instead. `worldScene` itself (like
   * `scene`) is not disposed -- a `THREE.Scene` holds no GPU resources of
   * its own beyond its children's, which are what actually get released.
   */
  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.clearGroup(this.pitchGroup);
    this.clearGroup(this.hudGroup);
    this.scene.remove(this.pitchGroup, this.hudGroup);
    if (this.worldLayer !== undefined) {
      this.worldScene.remove(this.worldLayer.group);
      this.worldLayer.dispose();
      this.worldLayer = undefined;
    }
    this.bloomPass.dispose();
    this.renderer.dispose();
    this.disposed = true;
  }

  private clearGroup(group: THREE.Group): void {
    for (const child of [...group.children]) {
      group.remove(child);
      disposeObject(child);
    }
  }

  private assertNotDisposed(): void {
    if (this.disposed) {
      throw new Error("scene.ts: SceneRoot used after dispose()");
    }
  }
}
