// Tests for bloom.ts.
//
// TESTABILITY BOUNDARY, and how #404 moved it. The previous version of this
// file could only test `dispose()`'s no-op path: everything else about bloom
// lived inside `UnrealBloomPass`, so the port's actual behaviour -- what makes
// a pixel glow, how far the glow spreads, how it is added back -- was not
// expressible without a live GL context. That is exactly how a faithful
// NUMBER port shipped an unfaithful LOOK.
//
// The pass is now Lua's own algorithm, and the parts of it that decide the
// look are pure arithmetic pulled out of the shaders: `brightPassFactor`,
// `lowResSize`, `blurDirection`, `compositeChannel`, `BLOOM_BLUR_WEIGHTS`.
// The GLSL is GENERATED from those same constants (see
// `buildBlurFragmentShader`), so asserting on them constrains what the GPU
// actually runs rather than a parallel copy of it. Rasterization itself is
// still not asserted here -- that needs a real `THREE.WebGLRenderer`, the same
// boundary scene.spec.ts's `stubRenderer()` note draws.
//
// Every expectation below is cross-checked against game/render/bloom.lua,
// cited by line, so a future edit to one side fails against the other.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  Bloom,
  BLOOM_BLUR_TAP_RADIUS,
  BLOOM_BLUR_WEIGHTS,
  BLOOM_COMPOSITE_CLAMP,
  BLOOM_THRESHOLD_KNEE,
  DEFAULT_BLOOM_CONFIG,
  bloomKernelReachPixels,
  blurDirection,
  brightPassFactor,
  buildBlurFragmentShader,
  buildCompositeFragmentShader,
  buildThresholdFragmentShader,
  compositeChannel,
  lowResSize,
  sameBloomBufferKey,
  smoothstep,
} from "./bloom.ts";

describe("DEFAULT_BLOOM_CONFIG", () => {
  it("is game/render/bloom.lua's `bloom.config` (lines 70-77), value for value", () => {
    expect(DEFAULT_BLOOM_CONFIG).toEqual({
      enabled: true,
      threshold: 0.55,
      intensity: 1.3,
      passes: 2,
      radius: 2.0,
      downscale: 2,
    });
  });
});

describe("BLOOM_BLUR_WEIGHTS", () => {
  it("is BLUR_SRC's 9 taps (bloom.lua lines 24-32), in order", () => {
    expect(BLOOM_BLUR_WEIGHTS).toEqual([0.05, 0.09, 0.12, 0.15, 0.18, 0.15, 0.12, 0.09, 0.05]);
  });

  it("sums to exactly 1 -- the blur spreads brightness, it never amplifies it", () => {
    const total = BLOOM_BLUR_WEIGHTS.reduce((sum, weight) => sum + weight, 0);
    expect(total).toBeCloseTo(1, 12);
  });

  it("is symmetric about the centre tap, so the blur does not drift the glow", () => {
    const reversed = [...BLOOM_BLUR_WEIGHTS].reverse();
    expect(reversed).toEqual([...BLOOM_BLUR_WEIGHTS]);
  });

  it("reaches 4 taps either side of centre", () => {
    expect(BLOOM_BLUR_TAP_RADIUS).toBe(4);
    expect(BLOOM_BLUR_WEIGHTS.length).toBe(2 * BLOOM_BLUR_TAP_RADIUS + 1);
  });
});

describe("smoothstep", () => {
  it("matches GLSL: 0 below edge0, 1 above edge1, 0.5 at the midpoint", () => {
    expect(smoothstep(0.2, 0.8, 0.1)).toBe(0);
    expect(smoothstep(0.2, 0.8, 0.2)).toBe(0);
    expect(smoothstep(0.2, 0.8, 0.8)).toBe(1);
    expect(smoothstep(0.2, 0.8, 0.9)).toBe(1);
    expect(smoothstep(0.2, 0.8, 0.5)).toBeCloseTo(0.5, 12);
  });

  it("is the cubic 3t^2 - 2t^3, not a straight ramp", () => {
    // A linear ramp would give 0.25 a quarter of the way in; the cubic gives
    // 3*(0.25^2) - 2*(0.25^3) = 0.15625.
    expect(smoothstep(0, 1, 0.25)).toBeCloseTo(0.15625, 12);
  });
});

describe("brightPassFactor -- what actually gets to glow", () => {
  const threshold = DEFAULT_BLOOM_CONFIG.threshold;

  it("rejects everything at or below the threshold outright", () => {
    expect(brightPassFactor(0, 0, 0, threshold)).toBe(0);
    expect(brightPassFactor(0.3, 0.3, 0.3, threshold)).toBe(0);
    expect(brightPassFactor(0.55, 0.55, 0.55, threshold)).toBe(0);
  });

  it("passes everything at or above threshold + knee at full strength", () => {
    expect(brightPassFactor(threshold + BLOOM_THRESHOLD_KNEE, 0, 0, threshold)).toBe(1);
    expect(brightPassFactor(1, 1, 1, threshold)).toBe(1);
  });

  it("ramps smoothly across the 0.15 knee (bloom.lua line 16)", () => {
    expect(BLOOM_THRESHOLD_KNEE).toBe(0.15);
    const mid = brightPassFactor(threshold + BLOOM_THRESHOLD_KNEE / 2, 0, 0, threshold);
    expect(mid).toBeCloseTo(0.5, 12);
    const low = brightPassFactor(threshold + 0.03, 0, 0, threshold);
    expect(low).toBeGreaterThan(0);
    expect(low).toBeLessThan(0.2);
  });

  it("keys off the MAX channel, not luminance -- a saturated single-channel colour still glows", () => {
    // This is mismatch #2 in bloom.ts's header, in one assertion. Rec.709
    // luminance of pure blue at 0.9 is 0.065, so `UnrealBloomPass`'s
    // `LuminosityHighPassShader` rejected it outright; `max(r, g, b)` is 0.9,
    // so Lua blooms it fully. Same constant, different pixels.
    expect(brightPassFactor(0.9, 0.0, 0.0, threshold)).toBe(1);
    expect(brightPassFactor(0.0, 0.0, 0.9, threshold)).toBe(1);
    expect(brightPassFactor(0.5, 0.5, 0.5, threshold)).toBe(0);
  });

  it("treats clamped-to-white emissive (cel_shader's 1.80x) as fully bright", () => {
    // rig3d/cel_shader.ts pushes emissive slots past white on purpose (#404
    // says NOT to retune them). The capture target is 8-bit in BOTH the old
    // and new chains (see bloom.ts's BUFFER PRECISION note), so what the
    // bright pass sees is 1.0, not 1.8 -- same as LOVE's rgba8 canvas. This
    // pins that: the 1.80x is not, and never was, the halo.
    expect(brightPassFactor(1, 1, 1, threshold)).toBe(1);
  });
});

describe("lowResSize -- bloom.lua lines 123-124", () => {
  it("floors w/downscale and h/downscale", () => {
    expect(lowResSize(960, 540, 2)).toEqual({ w: 480, h: 270 });
    expect(lowResSize(1920, 1080, 4)).toEqual({ w: 480, h: 270 });
  });

  it("floors rather than rounds, on odd sizes", () => {
    expect(lowResSize(961, 541, 2)).toEqual({ w: 480, h: 270 });
  });

  it("clamps to at least 1x1, matching math.max(1, ...)", () => {
    expect(lowResSize(1, 1, 8)).toEqual({ w: 1, h: 1 });
    expect(lowResSize(0, 0, 2)).toEqual({ w: 1, h: 1 });
  });

  it("is identity at downscale 1", () => {
    expect(lowResSize(960, 540, 1)).toEqual({ w: 960, h: 540 });
  });
});

describe("blurDirection -- bloom.lua lines 259 and 265", () => {
  const size = { w: 480, h: 270 };

  it("steps `radius` texels horizontally, in normalised UV", () => {
    expect(blurDirection(2.0, size, "horizontal")).toEqual([2.0 / 480, 0]);
  });

  it("steps `radius` texels vertically, in normalised UV", () => {
    expect(blurDirection(2.0, size, "vertical")).toEqual([0, 2.0 / 270]);
  });

  it("never mixes axes -- the blur is separable, one axis per pass", () => {
    expect(blurDirection(3.0, size, "horizontal")[1]).toBe(0);
    expect(blurDirection(3.0, size, "vertical")[0]).toBe(0);
  });

  it("scales with the low-res buffer, so the same `radius` is the same number of texels at any resolution", () => {
    const [smallStep] = blurDirection(2.0, { w: 240, h: 135 }, "horizontal");
    const [bigStep] = blurDirection(2.0, { w: 480, h: 270 }, "horizontal");
    expect(smallStep).toBeCloseTo(bigStep * 2, 12);
  });
});

describe("compositeChannel -- bloom.lua lines 272-276", () => {
  it("is scene + glow * intensity", () => {
    expect(compositeChannel(0.2, 0.5, 1.3)).toBeCloseTo(0.2 + 0.5 * 1.3, 12);
  });

  it("leaves a pixel with no glow completely untouched", () => {
    expect(compositeChannel(0.42, 0, 1.3)).toBeCloseTo(0.42, 12);
    expect(compositeChannel(0, 0, 1.3)).toBe(0);
  });

  it("clamps at 1, like the 8-bit destination framebuffer both builds composite into", () => {
    expect(compositeChannel(0.9, 0.9, 1.3)).toBe(1);
    expect(compositeChannel(1, 1, 1.3)).toBe(1);
  });

  it("is additive, not a blend -- the scene is never darkened by the glow", () => {
    for (const scene of [0, 0.25, 0.5, 0.75, 1]) {
      expect(compositeChannel(scene, 0.3, 1.3)).toBeGreaterThanOrEqual(Math.min(scene, 1));
    }
  });
});

describe("bloomKernelReachPixels -- the #404 regression fence", () => {
  it("is 32 full-res pixels at the shipped defaults", () => {
    // 2 passes * 4 taps * radius 2.0 * downscale 2. A bright pixel simply
    // cannot tint anything further away than this, which is why the Lua build
    // has a highlight on trim and not a halo around a character. For scale:
    // UnrealBloomPass's fifth mip is 1/32 of the pass resolution, so one texel
    // of it covers ~64 full-res pixels and its blur kernel spans several of
    // those -- hundreds of pixels of reach on the same frame.
    expect(bloomKernelReachPixels(DEFAULT_BLOOM_CONFIG)).toBe(32);
  });

  it("stays well under a character's own on-screen height at 960x540", () => {
    // Characters are drawn on the order of 100+ px tall; a reach that exceeded
    // that is what "haloed" looks like.
    expect(bloomKernelReachPixels(DEFAULT_BLOOM_CONFIG)).toBeLessThan(100);
  });

  it("grows linearly in each of passes, radius and downscale", () => {
    const base = bloomKernelReachPixels(DEFAULT_BLOOM_CONFIG);
    expect(bloomKernelReachPixels({ ...DEFAULT_BLOOM_CONFIG, passes: 4 })).toBe(base * 2);
    expect(bloomKernelReachPixels({ ...DEFAULT_BLOOM_CONFIG, radius: 4.0 })).toBe(base * 2);
    expect(bloomKernelReachPixels({ ...DEFAULT_BLOOM_CONFIG, downscale: 4 })).toBe(base * 2);
  });

  it("is zero when passes is 0 -- the bright pass alone spreads nothing", () => {
    expect(bloomKernelReachPixels({ ...DEFAULT_BLOOM_CONFIG, passes: 0 })).toBe(0);
  });
});

describe("the generated GLSL is generated from the constants above", () => {
  it("emits one blur tap per weight, offset -4..+4, with the weight inline", () => {
    const source = buildBlurFragmentShader(BLOOM_BLUR_WEIGHTS);
    const taps = source.split("\n").filter((line) => line.includes("sum +="));
    expect(taps.length).toBe(BLOOM_BLUR_WEIGHTS.length);
    expect(taps[0]).toContain("direction * -4.0");
    expect(taps[0]).toContain("0.0500");
    expect(taps[4]).toContain("direction * 0.0");
    expect(taps[4]).toContain("0.1800");
    expect(taps[8]).toContain("direction * 4.0");
    expect(taps[8]).toContain("0.0500");
  });

  it("tracks the weights it is given, so the kernel cannot drift from the constant", () => {
    const source = buildBlurFragmentShader([0.25, 0.5, 0.25]);
    const taps = source.split("\n").filter((line) => line.includes("sum +="));
    expect(taps.length).toBe(3);
    expect(taps[0]).toContain("direction * -1.0");
    expect(taps[1]).toContain("0.5000");
  });

  it("writes the threshold knee into the smoothstep, from BLOOM_THRESHOLD_KNEE", () => {
    expect(buildThresholdFragmentShader(BLOOM_THRESHOLD_KNEE)).toContain("smoothstep(threshold, threshold + 0.1500, b)");
  });

  it("thresholds on the max channel, matching brightPassFactor", () => {
    expect(buildThresholdFragmentShader(BLOOM_THRESHOLD_KNEE)).toContain("max(c.r, max(c.g, c.b))");
  });

  it("writes alpha as a literal 1.0, NOT the keep factor -- the composite depends on this", () => {
    // COMPOSITE_FRAGMENT_SHADER reads only `.rgb` from the glow buffer, where
    // LOVE's "add" blend composites THROUGH alpha. Those agree only while glow
    // alpha is 1.0 everywhere, which needs BOTH halves: alpha a literal here
    // (not `f`, not `c.a * f`), and a kernel summing to exactly 1 so the blur
    // carries it through. The other half is asserted in the
    // BLOOM_BLUR_WEIGHTS suite above. See COMPOSITE_FRAGMENT_SHADER's comment.
    expect(buildThresholdFragmentShader(BLOOM_THRESHOLD_KNEE)).toContain("gl_FragColor = vec4(c.rgb * f, 1.0);");
  });
});

describe("sameBloomBufferKey -- when the render targets must be rebuilt", () => {
  const base = { w: 960, h: 540, ratio: 1, downscale: 2 };

  it("is true for an identical key -- a steady-state frame reallocates nothing", () => {
    expect(sameBloomBufferKey(base, { ...base })).toBe(true);
  });

  it("is false when the viewport changes", () => {
    expect(sameBloomBufferKey(base, { ...base, w: 1280 })).toBe(false);
    expect(sameBloomBufferKey(base, { ...base, h: 720 })).toBe(false);
  });

  it("is false when the pixel ratio changes at an unchanged viewport", () => {
    // Targets are sized in DEVICE pixels, so this alone must rebuild them.
    expect(sameBloomBufferKey(base, { ...base, ratio: 2 })).toBe(false);
  });

  it("is false when downscale changes at an unchanged viewport and ratio", () => {
    // `downscale` sizes blurA/blurB and nothing else would notice it moving.
    // `BloomConfig`'s fields are mutable and runtime_settings.ts already
    // mutates one of them (`enabled`), so this is keyed rather than assumed
    // constant -- see Bloom.ensure's doc comment.
    expect(sameBloomBufferKey(base, { ...base, downscale: 4 })).toBe(false);
  });
});

// `draw` needs a live GL context for everything except the disabled path, and
// that path is a product lever (`?bloom=0` in tools/browser_match_harness),
// not an implementation detail -- so it is worth a test of its own. The stub
// below is not a WebGL mock: `draw`'s `enabled === false` branch calls exactly
// one method on the renderer and allocates nothing, which is the whole claim.
interface StubRenderer {
  renderCalls: number;
  render(scene: THREE.Object3D, camera: THREE.Camera): void;
  getPixelRatio(): number;
}

function stubRenderer(): StubRenderer {
  return {
    renderCalls: 0,
    render() {
      this.renderCalls += 1;
    },
    getPixelRatio() {
      throw new Error("bloom.ts: the disabled path must not touch the renderer beyond render()");
    },
  };
}

describe("Bloom.draw with enabled: false -- the ?bloom=0 lever", () => {
  it("renders the scene straight to the canvas and allocates no bloom resources", () => {
    const stub = stubRenderer();
    const bloom = new Bloom({ enabled: false });
    const scene = new THREE.Scene();
    const camera = new THREE.Camera();

    bloom.draw(stub as unknown as THREE.WebGLRenderer, scene, camera, 960, 540);

    expect(stub.renderCalls).toBe(1);
    // Nothing was sized, so nothing was allocated -- `getPixelRatio` would
    // have thrown had `ensure()` run at all.
    expect(bloom.getBufferSize()).toEqual({ w: 0, h: 0 });
  });

  it("stays a passthrough across repeated frames", () => {
    const stub = stubRenderer();
    const bloom = new Bloom({ enabled: false });
    const scene = new THREE.Scene();
    const camera = new THREE.Camera();
    for (let i = 0; i < 3; i += 1) {
      bloom.draw(stub as unknown as THREE.WebGLRenderer, scene, camera, 960, 540);
    }
    expect(stub.renderCalls).toBe(3);
  });
});

describe("Bloom.dispose", () => {
  it("is a no-op, and does not throw, when draw() was never called", () => {
    const bloom = new Bloom();
    expect(() => bloom.dispose()).not.toThrow();
  });

  it("is idempotent -- a second call does not throw either", () => {
    const bloom = new Bloom();
    bloom.dispose();
    expect(() => bloom.dispose()).not.toThrow();
  });

  it("leaves config untouched -- dispose releases GPU resources, not configuration", () => {
    const bloom = new Bloom({ threshold: 0.2 });
    bloom.dispose();
    expect(bloom.config).toEqual({ ...DEFAULT_BLOOM_CONFIG, threshold: 0.2 });
  });
});

// ---------------------------------------------------------------------------
// THE ENABLED PATH, i.e. the half that was previously never executed at all.
//
// Everything above proves the pure functions correct. None of it proved they
// were the values actually WIRED INTO the running pass -- that link lived only
// in comments, so an edit that rewired `ensure()`'s constants, or made the
// composite non-additive, left the whole suite green. That is the same shape
// of gap as #404 itself: right pieces, wrong assembly.
//
// `recordingRenderer()` closes it. It is NOT a WebGL mock and does not
// rasterise: `Bloom.draw`'s non-disabled path calls exactly five methods on the
// renderer (`getPixelRatio`, `getRenderTarget`, `setRenderTarget`, `clear`,
// `render`) plus the `autoClear` property, none of which need a GL context to
// stand in for. Because `Quad.render` hands the renderer the quad MESH, the
// recorder can read `mesh.material` off each call and assert on the REAL
// `ShaderMaterial` -- its `fragmentShader` string and its live uniform values
// -- which is what makes "the constants the tests assert on are the constants
// the GPU runs" a checked statement rather than a hopeful one.
//
// `THREE.WebGLRenderTarget` is a plain JS object until something renders to
// it, so constructing them here touches no GL either.

interface RenderRecord {
  readonly target: THREE.WebGLRenderTarget | null;
  readonly material: THREE.ShaderMaterial | undefined;
  readonly isScene: boolean;
}

interface RecordingRenderer {
  autoClear: boolean;
  readonly records: RenderRecord[];
  readonly clears: number;
  getPixelRatio(): number;
  getRenderTarget(): THREE.WebGLRenderTarget | null;
  setRenderTarget(target: THREE.WebGLRenderTarget | null): void;
  clear(color?: boolean, depth?: boolean, stencil?: boolean): void;
  render(object: THREE.Object3D, camera: THREE.Camera): void;
}

function recordingRenderer(pixelRatio = 1): RecordingRenderer {
  const records: RenderRecord[] = [];
  let current: THREE.WebGLRenderTarget | null = null;
  return {
    autoClear: true,
    records,
    clears: 0,
    getPixelRatio: () => pixelRatio,
    getRenderTarget: () => current,
    setRenderTarget(target) {
      current = target;
    },
    clear() {
      (this as { clears: number }).clears += 1;
    },
    render(object) {
      const material = (object as THREE.Mesh).material;
      records.push({
        target: current,
        material: material instanceof THREE.ShaderMaterial ? material : undefined,
        isScene: (object as THREE.Scene).isScene === true,
      });
    },
  };
}

function drawOnce(bloom: Bloom, w = 960, h = 540, pixelRatio = 1): RecordingRenderer {
  const renderer = recordingRenderer(pixelRatio);
  bloom.draw(renderer as unknown as THREE.WebGLRenderer, new THREE.Scene(), new THREE.Camera(), w, h);
  return renderer;
}

describe("Bloom.draw with enabled: true -- the pure functions are the ones actually wired in", () => {
  it("runs scene capture, bright pass, 2 blur passes per axis, and one composite", () => {
    const renderer = drawOnce(new Bloom());
    // 1 scene + 1 threshold + 2*passes blur + 1 composite.
    expect(renderer.records.length).toBe(1 + 1 + 2 * DEFAULT_BLOOM_CONFIG.passes + 1);
    expect(renderer.records[0]?.isScene).toBe(true);
    expect(renderer.records.slice(1).every((r) => r.isScene === false)).toBe(true);
  });

  it("honours `passes` -- the knob UnrealBloomPass had nothing to map to", () => {
    expect(drawOnce(new Bloom({ passes: 0 })).records.length).toBe(3);
    expect(drawOnce(new Bloom({ passes: 1 })).records.length).toBe(5);
    expect(drawOnce(new Bloom({ passes: 4 })).records.length).toBe(11);
  });

  it("composites to whatever render target the caller had bound -- the canvas, in production", () => {
    const renderer = drawOnce(new Bloom());
    expect(renderer.records[renderer.records.length - 1]?.target).toBe(null);
  });

  it("renders the scene into an offscreen target, never straight to the canvas", () => {
    const renderer = drawOnce(new Bloom());
    expect(renderer.records[0]?.target).not.toBe(null);
  });

  it("ping-pongs the blur between two DIFFERENT low-res targets", () => {
    const renderer = drawOnce(new Bloom());
    const blurTargets = renderer.records.slice(2, 6).map((r) => r.target);
    expect(blurTargets[0]).not.toBe(blurTargets[1]);
    expect(blurTargets[0]).toBe(blurTargets[2]);
    expect(blurTargets[1]).toBe(blurTargets[3]);
  });

  it("sizes the bright/blur buffers through lowResSize, in DEVICE pixels", () => {
    const bloom = new Bloom();
    drawOnce(bloom, 960, 540, 2);
    expect(bloom.getBufferSize()).toEqual(lowResSize(1920, 1080, DEFAULT_BLOOM_CONFIG.downscale));
    expect(bloom.getBufferSize()).toEqual({ w: 960, h: 540 });
  });

  it("feeds the threshold shader `config.threshold`, and the composite `config.intensity`", () => {
    const bloom = new Bloom({ threshold: 0.31, intensity: 2.25 });
    const renderer = drawOnce(bloom);
    const threshold = renderer.records[1]?.material;
    const composite = renderer.records[renderer.records.length - 1]?.material;
    expect(threshold?.uniforms.threshold?.value).toBe(0.31);
    expect(composite?.uniforms.intensity?.value).toBe(2.25);
  });

  it("feeds the blur shader the exact vector blurDirection derives", () => {
    const bloom = new Bloom({ radius: 3.0 });
    const renderer = drawOnce(bloom);
    // The last blur of the loop is the vertical one, so that is what the
    // shared uniform holds when drawing ends.
    const blur = renderer.records[5]?.material;
    const expected = blurDirection(3.0, bloom.getBufferSize(), "vertical");
    expect(blur?.uniforms.direction?.value.x).toBeCloseTo(expected[0], 12);
    expect(blur?.uniforms.direction?.value.y).toBeCloseTo(expected[1], 12);
  });

  it("runs the GENERATED shaders, not some other copy of them", () => {
    const renderer = drawOnce(new Bloom());
    expect(renderer.records[1]?.material?.fragmentShader).toBe(buildThresholdFragmentShader(BLOOM_THRESHOLD_KNEE));
    expect(renderer.records[2]?.material?.fragmentShader).toBe(buildBlurFragmentShader(BLOOM_BLUR_WEIGHTS));
    expect(renderer.records[renderer.records.length - 1]?.material?.fragmentShader).toBe(
      buildCompositeFragmentShader(BLOOM_COMPOSITE_CLAMP),
    );
  });

  it("reuses its targets across frames at an unchanged size, and rebuilds them when downscale moves", () => {
    const bloom = new Bloom();
    const first = drawOnce(bloom);
    const sceneTarget = first.records[0]?.target;
    const second = drawOnce(bloom);
    expect(second.records[0]?.target).toBe(sceneTarget);

    bloom.config.downscale = 4;
    const third = drawOnce(bloom);
    expect(third.records[0]?.target).not.toBe(sceneTarget);
    expect(bloom.getBufferSize()).toEqual(lowResSize(960, 540, 4));
  });

  it("restores the caller's autoClear rather than leaving it forced off", () => {
    const renderer = recordingRenderer();
    renderer.autoClear = true;
    new Bloom().draw(renderer as unknown as THREE.WebGLRenderer, new THREE.Scene(), new THREE.Camera(), 960, 540);
    expect(renderer.autoClear).toBe(true);
  });
});

describe("the composite shader is additive -- the property the whole effect rests on", () => {
  const source = buildCompositeFragmentShader(BLOOM_COMPOSITE_CLAMP);

  it("adds the glow to the scene, scaled by intensity", () => {
    expect(source).toContain("scene + glow * intensity");
  });

  it("clamps at BLOOM_COMPOSITE_CLAMP, agreeing with compositeChannel", () => {
    expect(source).toContain("vec3(1.0000)");
    expect(compositeChannel(0.9, 0.9, 1.3)).toBe(BLOOM_COMPOSITE_CLAMP);
  });

  it("applies the output colour-space encode -- without it the bloomed frame is darker than ?bloom=0", () => {
    // Found on hardware, not in a test: three.js encodes automatically only
    // for its own materials, so `renderer.render()` (the ?bloom=0 path) got the
    // sRGB transfer and this hand-written ShaderMaterial did not. See
    // buildCompositeFragmentShader's comment.
    expect(source).toContain("#include <colorspace_fragment>");
  });

  it("tracks the clamp it is given rather than hardcoding one", () => {
    expect(buildCompositeFragmentShader(0.5)).toContain("vec3(0.5000)");
  });
});
