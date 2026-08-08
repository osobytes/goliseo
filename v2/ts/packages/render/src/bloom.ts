// Port of game/render/bloom.lua (v2/README.md #7).
//
// WHAT CHANGED, AND WHY (#404). The first version of this file mapped Lua's
// bloom CONSTANTS onto three.js's `UnrealBloomPass`: `threshold -> .threshold`,
// `intensity -> .strength`, `radius -> .radius`, `downscale -> the pass
// resolution`, and `passes` wired to nothing at all because the pyramid has no
// such knob. The numbers were exact and the look was wrong -- characters wore a
// bright halo the LOVE build does not have, reading as light sources rather
// than as lit surfaces.
//
// The reason is that the two algorithms are not the same algorithm.
// game/render/bloom.lua (lines 21-36, 254-269) does exactly ONE separable 9-tap
// Gaussian at a single resolution, run `passes` times. Its total reach is
// bounded and small -- see `bloomKernelReachPixels` -- and energy outside that
// reach is exactly zero. `UnrealBloomPass` is a FIVE-LEVEL MIP PYRAMID: it
// blurs at 1/2, 1/4, 1/8, 1/16 and 1/32 of the pass resolution and sums the
// levels, so a bright texel reaches across a large fraction of the screen no
// matter what its parameters say. Three specific mismatches, each checked
// against the installed `three/examples/jsm/postprocessing` source:
//
//   1. `radius -> bloomRadius` is CATASTROPHIC at Lua's value, not merely
//      different. `UnrealBloomPass`'s composite weights its five mips by
//      `lerpBloomFactor(f) = mix(f, 1.2 - f, bloomRadius)` over the fixed
//      factors [1.0, 0.8, 0.6, 0.4, 0.2] -- a mix parameter, documented for
//      0..1. Lua's `radius = 2.0` is a BLUR STEP IN TEXELS, so passing it in
//      extrapolates every weight far past the intended range: the sharpest
//      mip lands at 1.0 + (0.2 - 1.0) * 2 = -0.6, and the widest, 1/32-
//      resolution mip at 0.2 + (1.0 - 0.2) * 2 = 1.8. The pass was
//      SUBTRACTING the tight glow and multiplying the screen-wide one by 1.8,
//      then scaling the sum by `strength = 1.3`. That is the halo, in numbers.
//   2. `threshold` selects a DIFFERENT SET OF PIXELS. `LuminosityHighPassShader`
//      thresholds on Rec.709 `luminance(rgb)`; Lua thresholds on
//      `max(r, g, b)`. A saturated blue emissive weighs 0.0722 as luminance
//      and 1.0 as max-channel -- Lua blooms it fully, the pyramid barely at
//      all. `UnrealBloomPass` also hardcodes `smoothWidth = 0.01` (not
//      exposed), a near-binary cut where Lua ramps over a 0.15 knee.
//   3. `passes` had no knob to map to at all, and was wired to nothing.
//
// So this file now implements LUA'S algorithm rather than re-tuning someone
// else's: threshold -> one separable blur at 1/downscale -> additive
// composite, over hand-managed render targets and an owned full-screen quad,
// which is the same three passes and the same three shaders the Lua module
// compiles. Retuning `UnrealBloomPass` was rejected in #404 for the reason it
// should be: its parameters do not correspond to Lua's, so any values we
// picked would be fitted by eye rather than ported.
//
// BUFFER PRECISION IS UNCHANGED BY THIS PORT, and that is worth stating
// because it looks like it changed. The previous version constructed its own
// `THREE.WebGLRenderTarget` with no `type` option and handed it to
// `EffectComposer` -- three.js's default `UnsignedByteType`, NOT the
// `HalfFloatType` `EffectComposer` picks when it allocates its own. So the
// captured scene was already clamped to 1.0 before the bright pass, exactly as
// LOVE's `rgba8` canvas clamps it, and this file's `sceneTarget` is the same
// 8-bit format. (`UnrealBloomPass`'s five internal mip targets WERE half-float,
// but they only ever received already-clamped input, so nothing above 1.0 ever
// entered them either.) The bright/blur buffers here are 8-bit where the
// pyramid's were half-float, which costs nothing that can be seen: the
// threshold pass multiplies an already-clamped colour by a factor <= 1, and
// `BLOOM_BLUR_WEIGHTS` sums to exactly 1, so no value in those buffers can
// exceed 1.0 by construction -- there is no headroom to preserve, only
// quantisation, and quantising there is what LOVE does.
//
// It matters that this is NOT the fix: rig3d/cel_shader.ts deliberately pushes
// emissive palette slots to 1.80x (an exact port of Lua's, which #404 says must
// not be retuned), and it would be tempting to blame the halo on that value
// surviving into the bright pass. It never did, in either build. Do not
// "upgrade" these targets to half-float on the theory that it would be more
// correct -- it would make the port less faithful, not more.
//
// `LinearFilter` + `ClampToEdgeWrapping` (the three.js defaults, left alone)
// match LOVE's default canvas filter and wrap mode for the same reason.
//
// COLOR MANAGEMENT: the composite pass applies the output sRGB encode via
// `#include <colorspace_fragment>`, and it has to -- see that shader's own
// comment for the full account. Short version: the old chain got the encode
// for free because `UnrealBloomPass`'s final scene blit used a built-in
// `MeshBasicMaterial`; a hand-written `ShaderMaterial` gets nothing
// automatically, and without the include the bloomed frame came out visibly
// DARKER than the same frame with `?bloom=0`, which is impossible for an
// additive effect. An earlier revision of this header claimed the two chains
// were equivalent here. They were not, and a hardware screenshot is what
// showed it.
//
// WHAT IS STILL DROPPED: `bloom.DEPTH_FORMATS` and the depth-canvas fallback
// ladder (Lua lines 38-68, 139-186). That list exists solely because love.js's
// WebGL1 backend cannot supply a `depth24stencil8`/`depth24` attachment and,
// per #360, *aborts the whole runtime* rather than raising a catchable error
// when asked for an unsupported one -- so the Lua module has to ask
// `getCanvasFormats()` first. `THREE.WebGLRenderTarget` allocates its own depth
// buffer (`depthBuffer` defaults to `true`, which is what lets the rigged
// `SkinnedMesh` characters depth-test inside the capture pass), and a JS
// exception is always catchable. `bloom.hasDepth()`/`bloom.depthFormat()` are
// dropped with it.

import * as THREE from "three";

export interface BloomConfig {
  enabled: boolean;
  /** brightness above which pixels glow */
  threshold: number;
  /** additive strength of the glow */
  intensity: number;
  /** blur iterations (more = softer/wider) */
  passes: number;
  /** blur step (texels in low-res space) */
  radius: number;
  /** bright/blur buffers at 1/downscale resolution */
  downscale: number;
}

/** `game/render/bloom.lua` lines 70-77, value for value. */
export const DEFAULT_BLOOM_CONFIG: BloomConfig = {
  enabled: true,
  threshold: 0.55,
  intensity: 1.3,
  passes: 2,
  radius: 2.0,
  downscale: 2,
};

/**
 * Width of the soft knee above `threshold` over which a pixel fades in, i.e.
 * Lua's `smoothstep(threshold, threshold + 0.15, b)` (bloom.lua line 16).
 */
export const BLOOM_THRESHOLD_KNEE = 0.15;

/**
 * Ceiling the composite clamps to, i.e. Lua's implicit one: LOVE's additive
 * draw lands in the 8-bit window framebuffer, and ours in the 8-bit canvas, so
 * both saturate at 1.0. Named and tested rather than inlined so
 * `compositeChannel` and the shader cannot disagree about it.
 */
export const BLOOM_COMPOSITE_CLAMP = 1.0;

/**
 * The separable blur kernel, tap for tap, from `BLUR_SRC` (bloom.lua lines
 * 24-32). Index 0 is the `-4` tap, index 8 the `+4`; the centre tap is index
 * 4. They sum to exactly 1, which is what makes the blur energy-preserving --
 * a pixel's brightness is spread, never amplified. `UnrealBloomPass` summing
 * five mips under extrapolated weights (see the file header: -0.6 on the
 * tightest, 1.8 on the widest, at Lua's `radius = 2.0`) is precisely what broke
 * that.
 */
export const BLOOM_BLUR_WEIGHTS: readonly number[] = [0.05, 0.09, 0.12, 0.15, 0.18, 0.15, 0.12, 0.09, 0.05];

/** How many taps the kernel reaches to either side of centre: 4, for the 9-tap kernel above. */
export const BLOOM_BLUR_TAP_RADIUS = (BLOOM_BLUR_WEIGHTS.length - 1) / 2;

/** `{w, h}` of the bright/blur buffers. */
export interface BloomBufferSize {
  readonly w: number;
  readonly h: number;
}

/**
 * Everything the render-target sizes depend on. `Bloom.ensure` reallocates when
 * and only when this changes, so it has to name EVERY such input -- see
 * `Bloom.ensure`'s doc comment for why `downscale` in particular is keyed
 * rather than assumed constant.
 */
export interface BloomBufferKey {
  readonly w: number;
  readonly h: number;
  readonly ratio: number;
  readonly downscale: number;
}

/**
 * Whether two buffer keys describe the same allocation. Split out of
 * `Bloom.ensure` so the cache-invalidation rule is provable without a GL
 * context -- `ensure` itself needs a real `THREE.WebGLRenderer` to reach.
 */
export function sameBloomBufferKey(a: BloomBufferKey, b: BloomBufferKey): boolean {
  return a.w === b.w && a.h === b.h && a.ratio === b.ratio && a.downscale === b.downscale;
}

/**
 * GLSL `smoothstep`, as the reference for `brightPassFactor` below. Exported
 * because the shader's behaviour is only testable through it.
 */
export function smoothstep(edge0: number, edge1: number, x: number): number {
  if (edge1 <= edge0) {
    return x < edge0 ? 0 : 1;
  }
  const t = Math.min(Math.max((x - edge0) / (edge1 - edge0), 0), 1);
  return t * t * (3 - 2 * t);
}

/**
 * The bright pass's per-pixel keep factor: `smoothstep(threshold, threshold +
 * knee, max(r, g, b))` (bloom.lua lines 14-17). The threshold shader multiplies
 * `c.rgb` by this, so 0 means "contributes no glow at all" -- the property that
 * keeps non-emissive surfaces out of the bloom buffer entirely.
 */
export function brightPassFactor(r: number, g: number, b: number, threshold: number): number {
  return smoothstep(threshold, threshold + BLOOM_THRESHOLD_KNEE, Math.max(r, Math.max(g, b)));
}

/**
 * Size of the bright/blur buffers, `math.max(1, math.floor(w / downscale))`
 * per axis (bloom.lua lines 123-124). `w`/`h` are the buffers' base resolution
 * in DEVICE pixels, not logical ones -- see `Bloom.ensure`.
 */
export function lowResSize(w: number, h: number, downscale: number): BloomBufferSize {
  return {
    w: Math.max(1, Math.floor(w / downscale)),
    h: Math.max(1, Math.floor(h / downscale)),
  };
}

/**
 * The blur shader's `direction` uniform for one axis: `{radius / lw, 0}`
 * horizontally, `{0, radius / lh}` vertically (bloom.lua lines 259, 265). It is
 * in normalised UV, so one unit of `radius` is one texel OF THE LOW-RES BUFFER
 * -- which is why `downscale` widens the blur in screen pixels as well as
 * making it cheaper.
 */
export function blurDirection(radius: number, size: BloomBufferSize, axis: "horizontal" | "vertical"): [number, number] {
  return axis === "horizontal" ? [radius / size.w, 0] : [0, radius / size.h];
}

/**
 * One channel of the final composite: the captured frame plus the glow scaled
 * by `intensity` (bloom.lua lines 272-276 -- `draw(scene)`, then `"add"`
 * blending with `setColor(intensity, intensity, intensity, 1)`). Clamped at 1
 * because LOVE's destination there is the 8-bit window framebuffer, and ours is
 * the 8-bit canvas; the composite shader applies the same `min`.
 */
export function compositeChannel(scene: number, glow: number, intensity: number): number {
  return Math.min(scene + glow * intensity, BLOOM_COMPOSITE_CLAMP);
}

/**
 * How far, in FULL-RESOLUTION pixels, one bright pixel's energy can reach in
 * the finished composite. `passes` sequential blurs of a kernel reaching
 * `BLOOM_BLUR_TAP_RADIUS * radius` low-res texels add their supports, and each
 * low-res texel is `downscale` full-res pixels wide.
 *
 * This exists to be asserted on. It is the number #404 is about: at the
 * defaults it is 32 px, whereas `UnrealBloomPass`'s 1/32 mip spreads a bright
 * texel across hundreds of pixels of a 960x540 frame. Nothing in the pipeline
 * reads it -- it is a diagnostic, and the regression fence for this fix.
 */
export function bloomKernelReachPixels(config: BloomConfig): number {
  return config.passes * BLOOM_BLUR_TAP_RADIUS * config.radius * config.downscale;
}

// Shared by all three post-process materials. `Quad`'s geometry is a 2x2 plane
// centred on the origin, so `position.xy` IS the clip-space coordinate and
// neither the camera nor the model matrix is consulted -- which is what makes
// the quad's camera conventions irrelevant. `uv` runs 0..1 across it in the
// same direction as a render target's texture coordinates, so no flip is
// needed anywhere in the chain.
const VERTEX_SHADER = /* glsl */ `
varying vec2 vUv;
void main() {
    vUv = uv;
    gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

/**
 * `THRESHOLD_SRC` (bloom.lua lines 11-19), with the knee taken from
 * `BLOOM_THRESHOLD_KNEE` rather than written out, so the constant the tests
 * assert on is the constant the GPU runs.
 *
 * Exported only so bloom.spec.ts can check that generation actually happened;
 * nothing outside this file calls it.
 */
export function buildThresholdFragmentShader(knee: number): string {
  return /* glsl */ `
uniform sampler2D tDiffuse;
uniform float threshold;
varying vec2 vUv;
void main() {
    vec4 c = texture2D(tDiffuse, vUv);
    float b = max(c.r, max(c.g, c.b));
    float f = smoothstep(threshold, threshold + ${knee.toFixed(4)}, b);
    gl_FragColor = vec4(c.rgb * f, 1.0);
}
`;
}

/**
 * `BLUR_SRC` (bloom.lua lines 21-36), generated from `BLOOM_BLUR_WEIGHTS` for
 * the same reason: the kernel is a tested constant, not a string literal that
 * can drift away from one.
 *
 * Exported only so bloom.spec.ts can check that generation actually happened;
 * nothing outside this file calls it.
 */
export function buildBlurFragmentShader(weights: readonly number[]): string {
  const tapRadius = (weights.length - 1) / 2;
  const taps = weights
    .map((weight, index) => {
      const offset = index - tapRadius;
      return `    sum += texture2D(tex, vUv + direction * ${offset.toFixed(1)}) * ${weight.toFixed(4)};`;
    })
    .join("\n");
  return /* glsl */ `
uniform sampler2D tex;
uniform vec2 direction;
varying vec2 vUv;
void main() {
    vec4 sum = vec4(0.0);
${taps}
    gl_FragColor = sum;
}
`;
}

/**
 * Lua's step 4 (bloom.lua lines 271-278) as one pass instead of two draws:
 * `draw(scene)` followed by an additive `draw(glow)` tinted by `intensity` is
 * `scene.rgb + glow.rgb * intensity`, clamped by the 8-bit destination. Doing
 * it in the shader avoids depending on three.js blend-state defaults for
 * something this load-bearing. Mirrors `compositeChannel`.
 *
 * IT DROPS THE GLOW BUFFER'S ALPHA, and that is only correct because of an
 * invariant the two shaders above enforce. LOVE's `"add"` blend mode composites
 * THROUGH alpha (`src.rgb * src.a + dst.rgb`), so Lua's step 4 is scaled by the
 * glow canvas's alpha; this reads `.rgb` and ignores it. The two agree today
 * because glow alpha is 1.0 everywhere by construction: the threshold shader
 * writes `vec4(c.rgb * f, 1.0)` -- alpha a literal, NOT multiplied by `f` --
 * and `BLOOM_BLUR_WEIGHTS` sums to exactly 1, so the blur carries that 1.0
 * through unchanged.
 *
 * If either half ever changes -- the threshold pass writing `f` into alpha, or
 * a kernel that does not sum to 1 -- this shader silently stops matching Lua
 * and must multiply by `texture2D(tGlow, vUv).a`. Both halves are pinned by
 * tests in bloom.spec.ts, so the drift fails loudly rather than visually.
 *
 * `#include <colorspace_fragment>` IS LOAD-BEARING, and its absence was a real
 * bug caught from a hardware screenshot rather than from a test. This is the
 * one pass in the chain whose render target is the CANVAS, so it is the one
 * that owes the output an sRGB encode.
 *
 * three.js applies that encode automatically only to its own materials.
 * `renderer.render(scene, camera)` -- the `enabled: false` path, and the old
 * `UnrealBloomPass` chain's final scene blit, which used a built-in
 * `MeshBasicMaterial` -- gets it for free. A hand-written `ShaderMaterial` does
 * not: rendering into a non-XR render target makes three emit
 * `LinearSRGBColorSpace`, so `sceneTarget` holds LINEAR values, and blitting
 * them raw to the canvas produced a **visibly darker frame with bloom on than
 * with `?bloom=0`** -- which is impossible for an additive effect, and would
 * have silently corrupted the `?bloom=0` attribution the browser harness
 * exists for. The include resolves to `linearToOutputTexel`, which three
 * generates from `renderer.outputColorSpace`, so this tracks the renderer
 * rather than hardcoding sRGB.
 *
 * Encoding ONCE, here, on the summed value is also the right place for it: the
 * add happens in linear light and the transfer function is applied to the
 * result. The old chain encoded the scene and then added un-encoded bloom on
 * top, which is not a thing that corresponds to anything.
 *
 * KNOWN, DELIBERATELY NOT CHANGED HERE: this leaves the 0.55 threshold being
 * evaluated against LINEAR values, whereas LOVE's canvas is display-space, so
 * v2 selects a slightly tighter set of pixels than Lua does. That is a
 * pre-existing property of the v2 chain (`LuminosityHighPassShader` thresholded
 * the linear buffer too), it errs toward under-blooming rather than the
 * over-blooming #404 is about, and changing it moves which pixels glow -- a
 * separate decision with its own evidence, not a rider on this fix.
 */
export function buildCompositeFragmentShader(clamp: number): string {
  return /* glsl */ `
uniform sampler2D tScene;
uniform sampler2D tGlow;
uniform float intensity;
varying vec2 vUv;
void main() {
    vec3 scene = texture2D(tScene, vUv).rgb;
    vec3 glow = texture2D(tGlow, vUv).rgb;
    gl_FragColor = vec4(min(scene + glow * intensity, vec3(${clamp.toFixed(4)})), 1.0);
    #include <colorspace_fragment>
}
`;
}

/**
 * A full-screen quad this class OWNS.
 *
 * three.js ships `FullScreenQuad` in `examples/jsm/postprocessing/Pass.js` and
 * it is not used here on purpose: its geometry is a MODULE-LEVEL `const`
 * shared by every instance in the process, so `FullScreenQuad.dispose()`
 * disposes a buffer other passes are still rendering with (checked against the
 * installed source, not assumed). Owning ~10 lines makes `Bloom.dispose`'s
 * contract -- release exactly what this instance allocated -- actually true.
 */
class Quad {
  private readonly mesh: THREE.Mesh;
  private readonly geometry: THREE.PlaneGeometry;
  private readonly placeholder: THREE.Material;
  private readonly camera: THREE.Camera;

  constructor() {
    this.geometry = new THREE.PlaneGeometry(2, 2);
    this.mesh = new THREE.Mesh(this.geometry);
    // `THREE.Mesh` builds a `MeshBasicMaterial` when handed none; `render`
    // replaces it before anything is drawn, but it is still this object's to
    // release.
    this.placeholder = this.mesh.material as THREE.Material;
    this.mesh.frustumCulled = false;
    this.camera = new THREE.Camera();
  }

  render(renderer: THREE.WebGLRenderer, material: THREE.ShaderMaterial): void {
    this.mesh.material = material;
    renderer.render(this.mesh, this.camera);
  }

  dispose(): void {
    this.geometry.dispose();
    this.placeholder.dispose();
  }
}

/**
 * A post-process material bundled with its OWN uniform objects.
 *
 * `ShaderMaterial.uniforms` is typed as an index signature, so reading
 * `material.uniforms.threshold.value` back out is `possibly undefined` under
 * this workspace's `noUncheckedIndexedAccess`. Holding the uniform records that
 * were handed to the constructor keeps every per-frame write statically typed
 * (`{ value: number }`, `{ value: THREE.Texture | null }`) instead of casting
 * the checker away -- three.js mutates the very objects passed in, so these are
 * the same records the GPU reads.
 */
interface PostMaterial<U> {
  readonly material: THREE.ShaderMaterial;
  readonly uniforms: U;
}

function postMaterial<U extends Record<string, THREE.IUniform>>(fragmentShader: string, uniforms: U): PostMaterial<U> {
  return {
    material: new THREE.ShaderMaterial({
      uniforms,
      vertexShader: VERTEX_SHADER,
      fragmentShader,
      depthTest: false,
      depthWrite: false,
      blending: THREE.NoBlending,
    }),
    uniforms,
  };
}

type ThresholdMaterial = PostMaterial<{
  tDiffuse: THREE.IUniform<THREE.Texture | null>;
  threshold: THREE.IUniform<number>;
}>;

type BlurMaterial = PostMaterial<{
  tex: THREE.IUniform<THREE.Texture | null>;
  direction: THREE.IUniform<THREE.Vector2>;
}>;

type CompositeMaterial = PostMaterial<{
  tScene: THREE.IUniform<THREE.Texture | null>;
  tGlow: THREE.IUniform<THREE.Texture | null>;
  intensity: THREE.IUniform<number>;
}>;

/** Additive bloom post-process, porting game/render/bloom.lua. See file header. */
export class Bloom {
  readonly config: BloomConfig;

  private sceneTarget: THREE.WebGLRenderTarget | undefined;
  private blurA: THREE.WebGLRenderTarget | undefined;
  private blurB: THREE.WebGLRenderTarget | undefined;
  private thresholdMaterial: ThresholdMaterial | undefined;
  private blurMaterial: BlurMaterial | undefined;
  private compositeMaterial: CompositeMaterial | undefined;
  private quad: Quad | undefined;
  private key: BloomBufferKey = { w: 0, h: 0, ratio: 0, downscale: 0 };
  private low: BloomBufferSize = { w: 0, h: 0 };

  constructor(config: Partial<BloomConfig> = {}) {
    this.config = { ...DEFAULT_BLOOM_CONFIG, ...config };
  }

  /**
   * The bright/blur buffer size in device pixels, or `{0, 0}` before anything
   * has been allocated. Exposed so a test can tell "allocated" from "not
   * allocated" without a GL context -- which is how the `?bloom=0` passthrough
   * is checked to allocate nothing at all.
   */
  getBufferSize(): BloomBufferSize {
    return this.low;
  }

  /**
   * Allocate (or reallocate) the three render targets and the three materials,
   * mirroring Lua's `ensure` (bloom.lua lines 102-137).
   *
   * THE CACHE KEY IS EVERY INPUT TO THE TARGET SIZES, which is `{w, h, ratio,
   * downscale}` -- not just the viewport.
   *
   * The pixel ratio belongs in it because the targets are sized in DEVICE
   * pixels, so a ratio change must rebuild them even when the logical viewport
   * is unchanged. Device pixels are also the base `downscale` divides -- Lua
   * divides the window resolution it actually renders at, and ours is
   * `w * ratio`, not `w`.
   *
   * `downscale` belongs in it because it sizes `blurA`/`blurB` and nothing else
   * would notice it changing. It is keyed rather than assumed-constant on
   * purpose: `BloomConfig`'s fields are mutable and ARE mutated at runtime --
   * `runtime_settings.ts` writes `bloom.config.enabled` from the settings
   * screen, so "nothing ever mutates this config" is already false for one
   * field, and resting on it for another is the kind of invariant that holds
   * until it quietly does not. Keying costs one comparison per frame; the bug
   * it forecloses is low-res buffers left stale until the next real resize.
   */
  private ensure(renderer: THREE.WebGLRenderer, w: number, h: number): void {
    const next: BloomBufferKey = { w, h, ratio: renderer.getPixelRatio(), downscale: this.config.downscale };
    if (this.sceneTarget !== undefined && sameBloomBufferKey(this.key, next)) {
      return;
    }
    this.disposeTargets();
    this.key = next;
    const { ratio } = next;

    const fullW = Math.max(Math.round(w * ratio), 1);
    const fullH = Math.max(Math.round(h * ratio), 1);
    this.low = lowResSize(fullW, fullH, this.config.downscale);

    // Both defaults are stated rather than inherited because both are
    // load-bearing: `UnsignedByteType` is LOVE's rgba8 clamp, and also what
    // the previous `EffectComposer` target already used -- see the file
    // header's BUFFER PRECISION note. `depthBuffer` is what lets the rigged
    // `SkinnedMesh` characters depth-test inside the capture pass.
    this.sceneTarget = new THREE.WebGLRenderTarget(fullW, fullH, {
      type: THREE.UnsignedByteType,
      depthBuffer: true,
    });
    // The bright/blur buffers never host geometry, only full-screen quads, so
    // no depth. 8-bit for the reason in the header: nothing that reaches them
    // can exceed 1.0, so there is no range to preserve.
    this.blurA = new THREE.WebGLRenderTarget(this.low.w, this.low.h, {
      type: THREE.UnsignedByteType,
      depthBuffer: false,
    });
    this.blurB = new THREE.WebGLRenderTarget(this.low.w, this.low.h, {
      type: THREE.UnsignedByteType,
      depthBuffer: false,
    });

    // Shaders and quad geometry survive a resize -- only the targets are
    // resolution-dependent, and recompiling three identical programs on every
    // window drag would be pure waste.
    if (this.quad === undefined) {
      const threshold: ThresholdMaterial = postMaterial(buildThresholdFragmentShader(BLOOM_THRESHOLD_KNEE), {
        tDiffuse: { value: null },
        threshold: { value: this.config.threshold },
      });
      const blur: BlurMaterial = postMaterial(buildBlurFragmentShader(BLOOM_BLUR_WEIGHTS), {
        tex: { value: null },
        direction: { value: new THREE.Vector2(0, 0) },
      });
      const composite: CompositeMaterial = postMaterial(buildCompositeFragmentShader(BLOOM_COMPOSITE_CLAMP), {
        tScene: { value: null },
        tGlow: { value: null },
        intensity: { value: this.config.intensity },
      });
      this.thresholdMaterial = threshold;
      this.blurMaterial = blur;
      this.compositeMaterial = composite;
      this.quad = new Quad();
    }
  }

  /**
   * Render `scene`/`camera` with bloom applied (or plain, if disabled).
   *
   * `config.enabled === false` must stay a pure passthrough that allocates
   * nothing: it is the `?bloom=0` lever the browser harness attributes frame
   * cost with (tools/browser_match_harness), so anything this method does
   * before that check shows up in the "bloom off" number.
   *
   * Steps below are numbered to match bloom.lua's own comments (lines 228-278).
   */
  draw(renderer: THREE.WebGLRenderer, scene: THREE.Scene, camera: THREE.Camera, w: number, h: number): void {
    if (!this.config.enabled) {
      renderer.render(scene, camera);
      return;
    }
    this.ensure(renderer, w, h);
    const sceneTarget = this.sceneTarget;
    const blurA = this.blurA;
    const blurB = this.blurB;
    const quad = this.quad;
    const thresholdMaterial = this.thresholdMaterial;
    const blurMaterial = this.blurMaterial;
    const compositeMaterial = this.compositeMaterial;
    // Unreachable: `ensure()` above assigned all seven. This narrows the
    // optional fields for the checker, and fails loudly rather than silently
    // skipping a pass if that ever stops being true.
    if (
      sceneTarget === undefined ||
      blurA === undefined ||
      blurB === undefined ||
      quad === undefined ||
      thresholdMaterial === undefined ||
      blurMaterial === undefined ||
      compositeMaterial === undefined
    ) {
      throw new Error("bloom.ts: ensure() did not allocate");
    }

    const previousTarget = renderer.getRenderTarget();
    const previousAutoClear = renderer.autoClear;
    renderer.autoClear = false;

    // 1. Capture the frame, depth buffer attached so a 3D phase inside the
    //    scene can depth-test; 2D content ignores it.
    renderer.setRenderTarget(sceneTarget);
    renderer.clear(true, true, true);
    renderer.render(scene, camera);

    // 2. Bright-pass into the low-res buffer.
    thresholdMaterial.uniforms.tDiffuse.value = sceneTarget.texture;
    thresholdMaterial.uniforms.threshold.value = this.config.threshold;
    renderer.setRenderTarget(blurA);
    renderer.clear(true, false, false);
    quad.render(renderer, thresholdMaterial.material);

    // 3. Separable Gaussian blur, ping-ponging blurA <-> blurB.
    const direction = blurMaterial.uniforms.direction.value;
    for (let pass = 0; pass < this.config.passes; pass += 1) {
      const [hx, hy] = blurDirection(this.config.radius, this.low, "horizontal");
      blurMaterial.uniforms.tex.value = blurA.texture;
      direction.set(hx, hy);
      renderer.setRenderTarget(blurB);
      renderer.clear(true, false, false);
      quad.render(renderer, blurMaterial.material);

      const [vx, vy] = blurDirection(this.config.radius, this.low, "vertical");
      blurMaterial.uniforms.tex.value = blurB.texture;
      direction.set(vx, vy);
      renderer.setRenderTarget(blurA);
      renderer.clear(true, false, false);
      quad.render(renderer, blurMaterial.material);
    }

    // 4. Composite: captured frame + additive glow, straight to `previousTarget`
    //    (the canvas, in production).
    compositeMaterial.uniforms.tScene.value = sceneTarget.texture;
    compositeMaterial.uniforms.tGlow.value = blurA.texture;
    compositeMaterial.uniforms.intensity.value = this.config.intensity;
    renderer.setRenderTarget(previousTarget);
    quad.render(renderer, compositeMaterial.material);

    renderer.autoClear = previousAutoClear;
  }

  /**
   * Releases every GPU resource this instance owns: the three
   * `THREE.WebGLRenderTarget`s, the three `THREE.ShaderMaterial`s and the
   * full-screen quad's own geometry (`Quad`, which exists precisely so that
   * geometry is not shared with anything else -- see its doc comment). A no-op
   * if `draw` was never called (nothing was allocated yet via `ensure`).
   * Idempotent --
   * safe to call more than once, matching `SceneRoot.dispose`'s own contract,
   * which is what calls this.
   */
  dispose(): void {
    this.disposeTargets();
    this.thresholdMaterial?.material.dispose();
    this.blurMaterial?.material.dispose();
    this.compositeMaterial?.material.dispose();
    this.quad?.dispose();
    this.thresholdMaterial = undefined;
    this.blurMaterial = undefined;
    this.compositeMaterial = undefined;
    this.quad = undefined;
  }

  private disposeTargets(): void {
    this.sceneTarget?.dispose();
    this.blurA?.dispose();
    this.blurB?.dispose();
    this.sceneTarget = undefined;
    this.blurA = undefined;
    this.blurB = undefined;
    this.key = { w: 0, h: 0, ratio: 0, downscale: 0 };
    this.low = { w: 0, h: 0 };
  }
}
