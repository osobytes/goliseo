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
 * FIXED HERE (was a known gap): `pitch.draw`'s rigged-player pass used to
 * composite each character by calling `renderer.render()` DIRECTLY and
 * IMMEDIATELY, rather than adding the character to `pitchGroup`'s `Object3D`
 * graph -- `SceneRoot.render`'s own later full-scene render would then clear
 * the canvas and wipe out whatever `pitch.draw` had just painted onto it (see
 * pitch.ts's SCOPE NOTE and player_renderer_3d.ts's `renderToSprite` for the
 * fix: each rigged character now renders to a private OFFSCREEN
 * `THREE.WebGLRenderTarget` during `populate`, and the result is wrapped in a
 * viewport-sized textured object added to `pitchGroup` at the exact point in
 * the painter's-algorithm depth order its procedural billboard would have
 * occupied -- interleaved with the ball and the other players, not appended
 * after them). The one, single render that touches the visible
 * canvas/composite chain is still this class's own `render` below, and it is
 * now genuinely the only one that does, which is what closes the gap.
 *
 * VERIFIED WITH A LIVE GL CONTEXT (this port's report): the offscreen
 * composite's camera frustum alignment is correct -- the character camera's
 * asymmetric frustum (`player_renderer_3d.ts`'s `characterCameraParams`)
 * lands the character at the right `(sx, sy)` on screen. Texture orientation
 * was NOT correct as first written: see `player_renderer_3d.ts`'s
 * `renderToSprite`, whose returned mesh now carries `scale.y = -1` to
 * correct for `SceneRoot`'s Y-inverted camera (below) -- `target.texture
 * .flipY` turned out to be a no-op for render-target textures, not the
 * fix.
 *
 * STILL N PASSES, NOT ONE (the next step, not yet landed). The offscreen-
 * render-target approach above closed the "characters get cleared" defect,
 * but it is still one FULL-VIEWPORT `renderer.render()` call per rigged
 * character per frame (`renderToSprite`), composited as a transparent quad
 * with `depthTest: false` -- correct painter's order via `pitchGroup`'s
 * child array position, but not per-pixel occlusion: two overlapping
 * characters cannot correctly interpenetrate, and neither can a character
 * and the ball. Closing THAT gap means characters joining this class's
 * `scene` as actual depth-tested `THREE.SkinnedMesh` objects, drawn in this
 * class's one existing full-scene `render()` call, rather than as quads
 * holding a picture of a mesh -- pitch.ts's "DEPTH ZONES" section and its
 * file-header "ONE PASS, ONE DEPTH BUFFER" note lay out the reconciliation
 * (screen-space position/scale, the elevation tilt, and this class's own
 * Y-inverted camera convention -- yes, still needing an equivalent of
 * `scale.y = -1`, just applied to the character's own transform instead of
 * a compositing plane) and the exact `player_renderer_3d.ts` signature this
 * package is written to expect once it exists. That file is owned by a
 * concurrent rewrite this milestone (its character SHADING), so the actual
 * mesh-per-character wiring is not implemented here -- `pitch.draw` still
 * calls `renderToSprite` exactly as before, unchanged in behaviour. What
 * pitch.ts/draw2d.ts DO add now, ready for that wiring: every depth-sorted
 * entity (billboard fallback or ball) already carries a real, depth-tested
 * world z (`pitch.ts`'s `depthToZ`), and `draw2d.ts`'s `appendCommands`
 * gained the `z`/`renderOrder`/`depthTest` options that make that possible
 * without touching `buildOne`'s existing screen-space vocabulary.
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
 */
export class SceneRoot {
  readonly scene: THREE.Scene;
  readonly camera: THREE.OrthographicCamera;
  readonly pitchGroup: THREE.Group;
  readonly hudGroup: THREE.Group;

  private readonly renderer: THREE.WebGLRenderer;
  private readonly bloomPass: Bloom;
  private viewport: SceneViewport;
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

    this.bloomPass = new Bloom(options.bloom ?? {});

    this.renderer.setPixelRatio(options.pixelRatio ?? 1);
    this.renderer.setSize(options.viewport.w, options.viewport.h, false);
  }

  /** The viewport `resize` last set. Exposed so a test can assert `resize` took effect. */
  getViewport(): SceneViewport {
    return this.viewport;
  }

  /**
   * Resize the owned renderer and recompute the camera frustum. Reuses the
   * existing `camera`/`scene`/groups/renderer -- none of them are
   * reconstructed.
   */
  resize(viewport: SceneViewport): void {
    this.assertNotDisposed();
    this.viewport = viewport;
    this.camera.left = 0;
    this.camera.right = viewport.w;
    this.camera.top = 0;
    this.camera.bottom = viewport.h;
    this.camera.updateProjectionMatrix();
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
   * doc comment and this class's own "FIXED HERE" note) -- `renderToSprite`
   * renders each character into its own private off-screen target, so
   * nothing it does can be clobbered by another player's draw sharing the
   * same target. `autoClear` is still forced off here because `pitch.draw`'s
   * non-rigged fallback path and the procedural billboards interleaved with
   * rigged sprites both paint into `pitchGroup` via `paint`/`appendCommands`,
   * which build an object graph rather than rasterize, so this setting is
   * defensive rather than load-bearing today; kept to avoid depending on
   * `SceneRoot` being the only caller that ever sets it.
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
  }

  /**
   * Render one frame: `populate` (see above), then rasterize the assembled
   * scene through bloom. Needs a live GL context to run at all -- see
   * `populate`'s doc comment -- so it is not exercised by scene.spec.ts.
   */
  render(frame: RenderFrame, options: SceneRenderOptions): void {
    this.assertNotDisposed();
    this.populate(frame, options);
    this.bloomPass.draw(this.renderer, this.scene, this.camera, this.viewport.w, this.viewport.h);
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
   */
  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.clearGroup(this.pitchGroup);
    this.clearGroup(this.hudGroup);
    this.scene.remove(this.pitchGroup, this.hudGroup);
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
