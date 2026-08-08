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
  BLOOM_THRESHOLD_KNEE,
  DEFAULT_BLOOM_CONFIG,
  bloomKernelReachPixels,
  blurDirection,
  brightPassFactor,
  buildBlurFragmentShader,
  buildThresholdFragmentShader,
  compositeChannel,
  lowResSize,
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
