// Replaces game/render/bloom.lua (v2/README.md #7: "bloom.lua -> replace --
// UnrealBloomPass").
//
// The Lua original hand-rolls a threshold-extract + separable-Gaussian-blur
// pass pair over raw GLSL and manually managed canvases, entirely because
// LÖVE gives you a GL context and nothing else. `EffectComposer` +
// `UnrealBloomPass` are exactly that pipeline, built in and GPU-profiled by
// three.js itself, so none of `ensure()`'s shader/canvas plumbing is ported.
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
// WHAT IS DROPPED ENTIRELY: `bloom.DEPTH_FORMATS` and the depth-canvas
// fallback ladder. That list exists solely because love.js's WebGL1 backend
// cannot supply a `depth24stencil8`/`depth24` attachment and, per #360,
// *aborts the whole runtime* rather than raising a catchable error when
// asked for an unsupported one -- so the Lua module has to ask
// `getCanvasFormats()` first and never request a format it has not already
// been told is safe. A real browser WebGL2 context (three.js's default,
// `WebGLRenderer`) always provides a depth/stencil renderbuffer through its
// own render-target management, and a JS exception is always catchable --
// neither the fallback ladder nor the "ask first" protocol it existed for
// has anything to check. `bloom.hasDepth()`/`bloom.depthFormat()` are
// dropped with it.

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
  /** blur iterations in the Lua original; not wired to `UnrealBloomPass' fixed mip pyramid, kept for parity */
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

/** Additive bloom post-process, replacing game/render/bloom.lua. See file header. */
export class Bloom {
  readonly config: BloomConfig;
  private composer: EffectComposer | undefined;
  private bloomPass: UnrealBloomPass | undefined;
  private w = 0;
  private h = 0;
  private ratio = 0;

  constructor(config: Partial<BloomConfig> = {}) {
    this.config = { ...DEFAULT_BLOOM_CONFIG, ...config };
  }

  private ensure(renderer: THREE.WebGLRenderer, scene: THREE.Scene, camera: THREE.Camera, w: number, h: number): EffectComposer {
    // The pixel ratio belongs in the cache key: the composer's target is sized
    // in DEVICE pixels below, so a ratio change must rebuild it even when the
    // logical viewport is unchanged.
    const ratio = renderer.getPixelRatio();
    if (this.composer !== undefined && this.w === w && this.h === h && this.ratio === ratio) {
      return this.composer;
    }
    this.w = w;
    this.h = h;
    this.ratio = ratio;
    // `downscale` applies to the BLOOM BUFFERS, not to the scene.
    //
    // It used to size the composer's own render target, which is what the
    // whole frame is rendered into and then blitted to the canvas from. At the
    // default `downscale: 2` that meant drawing the entire game at 480x270 and
    // upscaling it to the display -- reported, accurately, as "it looks like
    // pixel art". This is a porting slip rather than a style choice:
    // `game/render/bloom.lua` downscales its bright/blur buffers, and this
    // config field's own doc comment says "bright/blur buffers at 1/downscale
    // resolution". The intent was right and the wiring was inverted -- the
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
    composer.addPass(new RenderPass(scene, camera));
    const bloomPass = new UnrealBloomPass(
      new THREE.Vector2(Math.max(w / this.config.downscale, 1), Math.max(h / this.config.downscale, 1)),
      this.config.intensity,
      this.config.radius,
      this.config.threshold,
    );
    composer.addPass(bloomPass);
    this.composer = composer;
    this.bloomPass = bloomPass;
    return composer;
  }

  /** Render `scene`/`camera` with bloom applied (or plain, if disabled). Untested -- see draw2d.ts. */
  draw(renderer: THREE.WebGLRenderer, scene: THREE.Scene, camera: THREE.Camera, w: number, h: number): void {
    if (!this.config.enabled) {
      renderer.render(scene, camera);
      return;
    }
    const composer = this.ensure(renderer, scene, camera, w, h);
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
   * `draw` was never called (nothing was allocated yet via `ensure`).
   * Idempotent -- safe to call more than once, matching `SceneRoot.dispose`'s
   * own contract, which is what calls this. Wired up so `SceneRoot.dispose`
   * can actually release this scene's post-processing resources instead of
   * leaking an `EffectComposer` and its render targets on every teardown --
   * see scene.ts's `dispose` doc comment (before this port) for the gap.
   */
  dispose(): void {
    this.bloomPass?.dispose();
    this.composer?.dispose();
    this.composer = undefined;
    this.bloomPass = undefined;
    this.w = 0;
    this.h = 0;
  }
}
