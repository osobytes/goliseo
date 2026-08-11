// The bloom pass is `EffectComposer` + `UnrealBloomPass` (see ARCHITECTURE.md
// §5): a threshold-extract + separable-Gaussian-blur pass pair, built in and
// GPU-profiled by three.js itself rather than hand-rolled GLSL over manually
// managed canvases.
//
// WHAT IS KEPT: `bloom.config`'s tunables are content (the game's actual
// glow look), not mechanism, so `DEFAULT_BLOOM_CONFIG` preserves the exact
// values (`threshold = 0.55`, `intensity = 1.3`, `passes = 2`, `radius =
// 2.0`, `downscale = 2`) as the default `Bloom` construction. `intensity` ->
// `UnrealBloomPass.strength`, `radius` -> `UnrealBloomPass.radius`,
// `threshold` -> `UnrealBloomPass.threshold`; `downscale` maps to the
// composer's render-target resolution. `passes` (blur iterations) has no
// direct `UnrealBloomPass` knob -- it runs a fixed 5-mip pyramid internally
// -- so it is kept on `BloomConfig` for parity/diagnostics but is not wired
// to anything; noted here rather than silently dropped.
//
// WHY THERE IS NO DEPTH-FORMAT FALLBACK LADDER: an earlier WebGL1-only
// runtime could not supply a `depth24stencil8`/`depth24` attachment and, per
// #360, *aborted the whole runtime* rather than raising a catchable error
// when asked for an unsupported one -- so a caller had to ask which formats
// were safe before requesting one. A real browser WebGL2 context (three.js's
// default, `WebGLRenderer`) always provides a depth/stencil renderbuffer
// through its own render-target management, and a JS exception is always
// catchable, so neither a fallback ladder nor an "ask first" protocol has
// anything to check here.

import * as THREE from "three";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";

export interface BloomConfig {
  enabled: boolean;
  /** brightness above which pixels glow */
  threshold: number;
  /** additive strength of the glow (`UnrealBloomPass.strength`) */
  intensity: number;
  /** blur iteration count; not wired to `UnrealBloomPass`'s fixed mip pyramid, kept for parity */
  passes: number;
  /** blur step (`UnrealBloomPass.radius`) */
  radius: number;
  /** bright/blur buffers at 1/downscale resolution */
  downscale: number;
}

export const DEFAULT_BLOOM_CONFIG: BloomConfig = {
  enabled: true,
  threshold: 0.55,
  intensity: 1.3,
  passes: 2,
  radius: 2.0,
  downscale: 2,
};

/**
 * One renderable layer for `Bloom.draw_layers`: its own scene/camera pair.
 * `scene.ts`'s `SceneRoot` uses this to composite a world-space 3D stadium
 * layer (drawn through a real `THREE.PerspectiveCamera`) UNDER the existing
 * screen-space pitch/HUD content (drawn through `SceneRoot`'s own
 * orthographic camera) in one bloom pass, instead of two independently
 * bloomed images stacked on top of each other -- see `draw_layers`' own doc
 * comment for why that distinction (one composite pass vs. two) matters.
 */
export interface BloomLayer {
  readonly scene: THREE.Scene;
  readonly camera: THREE.Camera;
}

/** Additive bloom post-process. See file header. */
export class Bloom {
  readonly config: BloomConfig;
  private composer: EffectComposer | undefined;
  private bloomPass: UnrealBloomPass | undefined;
  private layers: ReadonlyArray<BloomLayer> | undefined;
  private w = 0;
  private h = 0;
  private ratio = 0;

  constructor(config: Partial<BloomConfig> = {}) {
    this.config = { ...DEFAULT_BLOOM_CONFIG, ...config };
  }

  // Shared by `draw` (single layer) and `draw_layers` (N layers): `draw`
  // calls this with a fresh one-element array built at its own call site, so
  // the cache key below compares LAYER IDENTITY (each layer's `scene`/
  // `camera` object references, plus count), never the wrapping array's own
  // identity -- a new array every call is fine as long as what is inside it
  // is the same.
  private ensureComposer(
    renderer: THREE.WebGLRenderer,
    layers: ReadonlyArray<BloomLayer>,
    w: number,
    h: number,
  ): EffectComposer {
    // The pixel ratio belongs in the cache key: the composer's target is sized
    // in DEVICE pixels below, so a ratio change must rebuild it even when the
    // logical viewport is unchanged. The layer list joins it for the same
    // reason `RenderPass`es are baked into the composer's own pass array at
    // construction time (three.js has no "replace this pass's scene/camera"
    // call) -- switching from a 1-layer to a 2-layer composite (or back)
    // needs a different set of passes, not just different uniforms on the
    // existing ones.
    const ratio = renderer.getPixelRatio();
    if (
      this.composer !== undefined &&
      this.w === w &&
      this.h === h &&
      this.ratio === ratio &&
      this.sameLayers(layers)
    ) {
      return this.composer;
    }
    this.w = w;
    this.h = h;
    this.ratio = ratio;
    this.layers = layers;
    // `downscale` applies to the BLOOM BUFFERS, not to the scene.
    //
    // It used to size the composer's own render target, which is what the
    // whole frame is rendered into and then blitted to the canvas from. At the
    // default `downscale: 2` that meant drawing the entire game at 480x270 and
    // upscaling it to the display -- reported, accurately, as "it looks like
    // pixel art". This was a bug rather than a style choice: this config
    // field's own doc comment says "bright/blur buffers at 1/downscale
    // resolution", and an earlier version had the wiring inverted -- the
    // scene got the downscale and `UnrealBloomPass` got the full size.
    //
    // So the composer renders at full DEVICE resolution (`w`/`h` are logical;
    // the canvas may be backed by more pixels than that -- see `SceneRoot`'s
    // `pixelRatio`), and the blur pyramid takes the downscale. Blurring at low
    // resolution is invisible by construction, which is why that is where the
    // saving is meant to come from.
    const target = new THREE.WebGLRenderTarget(
      Math.max(Math.round(w * ratio), 1),
      Math.max(Math.round(h * ratio), 1),
    );
    const composer = new EffectComposer(renderer, target);
    // Without this the composer re-derives its own size from the renderer on
    // resize and silently drops back to a ratio of 1.
    composer.setPixelRatio(ratio);
    composer.setSize(w, h);
    // One RenderPass per layer, in order -- painter's algorithm across
    // layers, same as `pitchGroup`/`hudGroup`'s own child-order convention
    // in scene.ts. The FIRST pass uses `RenderPass`'s own defaults (`clear =
    // true`, i.e. it clears colour/depth/stencil -- this is the one place
    // in the whole composite chain the canvas actually gets cleared, see
    // `pitch.ts`'s stadium-mode doc comment for why that matters once there
    // is no full-viewport backdrop rect to rely on instead). EVERY
    // SUBSEQUENT layer must NOT clear colour (that would erase the previous
    // layer's pixels, defeating the whole point of compositing more than
    // one) but MUST clear depth: each layer's scene/camera pair is an
    // unrelated 3D space (a world-space stadium camera vs. this package's
    // own screen-space orthographic camera, in `SceneRoot`'s case), so
    // depth-testing a later layer against an earlier, unrelated layer's
    // leftover depth buffer would occlude/be-occluded by geometry that was
    // never meant to interact with it at all.
    layers.forEach((layer, i) => {
      const pass = new RenderPass(layer.scene, layer.camera);
      if (i > 0) {
        pass.clear = false;
        pass.clearDepth = true;
      }
      composer.addPass(pass);
    });
    const bloomPass = new UnrealBloomPass(
      new THREE.Vector2(
        Math.max(w / this.config.downscale, 1),
        Math.max(h / this.config.downscale, 1),
      ),
      this.config.intensity,
      this.config.radius,
      this.config.threshold,
    );
    composer.addPass(bloomPass);
    this.composer = composer;
    this.bloomPass = bloomPass;
    return composer;
  }

  private sameLayers(layers: ReadonlyArray<BloomLayer>): boolean {
    const prev = this.layers;
    if (prev === undefined || prev.length !== layers.length) {
      return false;
    }
    return prev.every((layer, i) => {
      const next = layers[i];
      return next !== undefined && layer.scene === next.scene && layer.camera === next.camera;
    });
  }

  /** Render `scene`/`camera` with bloom applied (or plain, if disabled). Untested -- see draw2d.ts. */
  draw(
    renderer: THREE.WebGLRenderer,
    scene: THREE.Scene,
    camera: THREE.Camera,
    w: number,
    h: number,
  ): void {
    if (!this.config.enabled) {
      renderer.render(scene, camera);
      return;
    }
    const composer = this.ensureComposer(renderer, [{ scene, camera }], w, h);
    const pass = this.bloomPass;
    if (pass !== undefined) {
      pass.strength = this.config.intensity;
      pass.radius = this.config.radius;
      pass.threshold = this.config.threshold;
    }
    composer.render();
  }

  /**
   * Multi-layer composite: renders each of `layers` in order into ONE
   * shared composite (see `ensureComposer`'s clear-semantics comment above),
   * then applies ONE `UnrealBloomPass` over the result -- not one bloom pass
   * per layer. That distinction is deliberate: bloom reads brightness off
   * the RASTERIZED composite, so a glowing element in an earlier layer can
   * bloom onto pixels a later layer drew over it (and vice versa) exactly
   * as it would if everything had been one scene all along, matching how a
   * real camera's lens bloom does not know or care how many render passes
   * produced the light hitting it. `scene.ts`'s `SceneRoot` is today's only
   * caller: `[worldLayer, pitchLayer]`, world-space stadium geometry under
   * the existing screen-space pitch/HUD content.
   *
   * `enabled: false`: plain sequential `renderer.render()` calls (no
   * composer at all, matching `draw`'s own disabled path), with `autoClear`
   * managed by hand to reproduce the SAME clear semantics the composer path
   * above uses -- the first layer clears, every later layer clears depth
   * only. `renderer.autoClear` is restored to whatever the caller had it
   * set to once this returns, the same convention `scene.ts`'s `populate`
   * uses around `pitch.draw`.
   *
   * Untested beyond what bloom.spec.ts's header already explains for `draw`:
   * `ensureComposer` constructs a real `EffectComposer`, which needs the
   * full `THREE.WebGLRenderer` surface no stub in this milestone's headless
   * suite can stand in for.
   */
  draw_layers(
    renderer: THREE.WebGLRenderer,
    layers: ReadonlyArray<BloomLayer>,
    w: number,
    h: number,
  ): void {
    if (!this.config.enabled) {
      const wasAutoClear = renderer.autoClear;
      try {
        layers.forEach((layer, i) => {
          renderer.autoClear = i === 0;
          if (i > 0) {
            renderer.clearDepth();
          }
          renderer.render(layer.scene, layer.camera);
        });
      } finally {
        renderer.autoClear = wasAutoClear;
      }
      return;
    }
    const composer = this.ensureComposer(renderer, layers, w, h);
    const pass = this.bloomPass;
    if (pass !== undefined) {
      pass.strength = this.config.intensity;
      pass.radius = this.config.radius;
      pass.threshold = this.config.threshold;
    }
    composer.render();
  }

  /**
   * Releases every `THREE.WebGLRenderTarget` this instance owns: the
   * `EffectComposer`'s own two ping-pong buffers (`EffectComposer.dispose`)
   * plus `UnrealBloomPass`'s five-mip blur/bright render targets
   * (`UnrealBloomPass.dispose`, which `EffectComposer.dispose` does NOT call
   * on its own passes -- checked against the installed
   * `three/examples/jsm/postprocessing` source, not assumed). A no-op if
   * `draw`/`draw_layers` was never called (nothing was allocated yet via
   * `ensureComposer`). Idempotent -- safe to call more than once, matching
   * `SceneRoot.dispose`'s own contract, which is what calls this. Wired up
   * so `SceneRoot.dispose` can actually release this scene's post-processing
   * resources instead of leaking an `EffectComposer` and its render targets
   * on every teardown -- see scene.ts's `dispose` doc comment for the gap.
   */
  dispose(): void {
    this.bloomPass?.dispose();
    this.composer?.dispose();
    this.composer = undefined;
    this.bloomPass = undefined;
    this.layers = undefined;
    this.w = 0;
    this.h = 0;
  }
}
