// New tests for bloom.ts. Only the parts reachable WITHOUT a live GL context
// are tested here. `draw`'s `ensure()` constructs a real `EffectComposer`,
// whose constructor calls `renderer.getPixelRatio()` synchronously (checked
// against the installed `three/examples/jsm/postprocessing/EffectComposer.js`
// source), and its `render()` needs the full `WebGLRenderer` surface --
// building one against a stub renderer would not be a faithful test of
// anything, the same boundary scene.spec.ts's `stubRenderer()` note draws for
// `SceneRoot.render`. `dispose()`'s no-op-before-first-`draw` path needs no
// renderer at all, so that is what this file covers. `dispose()` actually
// releasing a live composer's render targets is untested here for the same
// reason `draw` itself is -- see this port's report.

import { describe, expect, it } from "vitest";
import { Bloom, DEFAULT_BLOOM_CONFIG } from "./bloom.ts";

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
