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
  private composer?: EffectComposer;
  private bloomPass?: UnrealBloomPass;
  private w = 0;
  private h = 0;

  constructor(config: Partial<BloomConfig> = {}) {
    this.config = { ...DEFAULT_BLOOM_CONFIG, ...config };
  }

  private ensure(renderer: THREE.WebGLRenderer, scene: THREE.Scene, camera: THREE.Camera, w: number, h: number): EffectComposer {
    if (this.composer !== undefined && this.w === w && this.h === h) {
      return this.composer;
    }
    this.w = w;
    this.h = h;
    const composer = new EffectComposer(renderer, new THREE.WebGLRenderTarget(w / this.config.downscale, h / this.config.downscale));
    composer.addPass(new RenderPass(scene, camera));
    const bloomPass = new UnrealBloomPass(new THREE.Vector2(w, h), this.config.intensity, this.config.radius, this.config.threshold);
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
}
