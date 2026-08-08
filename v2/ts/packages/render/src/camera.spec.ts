// Ported from spec/render/camera_spec.lua.

import { describe, expect, it } from "vitest";
import { camera, type CameraField, type CameraView, type CameraViewport } from "./camera.ts";

function near(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

const field: CameraField = { w: 960, h: 540 };
const vp: CameraViewport = { w: 1280, h: 720 };

describe("camera.project", () => {
  it("places nearer points lower on screen than far points", () => {
    const [, farY] = camera.project(480, 0, field, vp);
    const [, nearY] = camera.project(480, 540, field, vp);
    expect(nearY > farY).toBe(true);
  });

  it("scales nearer points up relative to far points", () => {
    const [, , farScale] = camera.project(480, 0, field, vp);
    const [, , nearScale] = camera.project(480, 540, field, vp);
    expect(nearScale > farScale).toBe(true);
  });

  it("keeps the pitch centre line on the screen centre at any depth", () => {
    const [farX] = camera.project(480, 0, field, vp);
    const [nearX] = camera.project(480, 540, field, vp);
    near(farX, vp.w / 2, 1e-6);
    near(nearX, vp.w / 2, 1e-6);
  });

  it("spreads the near edge wider than the far edge (trapezoid)", () => {
    const [farRight] = camera.project(960, 0, field, vp);
    const [nearRight] = camera.project(960, 540, field, vp);
    expect(nearRight > farRight).toBe(true);
  });
});

describe("camera.view", () => {
  it("zoom 1 leaves the projection untouched", () => {
    const v = camera.view(120, 90, field, 1);
    const [a, b, c] = camera.project(300, 200, field, vp);
    const [x, y, z] = camera.project(300, 200, field, vp, undefined, v);
    near(x, a, 0.001);
    near(y, b, 0.001);
    near(z, c, 0.001);
  });

  it("clamps the focus so a zoomed frame stays over the pitch", () => {
    const v = camera.view(0, 0, field, 2);
    expect(v.x).toBe(240);
    expect(v.y).toBe(135);
  });

  it("puts the focus at the centre of the screen", () => {
    const v = camera.view(300, 200, field, 2);
    const [x, y] = camera.project(v.x, v.y, field, vp, undefined, v);
    near(x, vp.w / 2, 0.001);
    near(y, vp.h / 2, 0.001);
  });

  it("magnifies without adding convergence", () => {
    // The whole point of a lens zoom: the ratio between a far span and a near
    // span is a property of the perspective, so magnifying must not change
    // it. The earlier window remap did, which is what made the pitch look
    // like a funnel.
    const v = camera.view(480, 270, field, 2);
    function span(wy: number, view: CameraView | undefined): number {
      const [a] = camera.project(300, wy, field, vp, undefined, view);
      const [b] = camera.project(660, wy, field, vp, undefined, view);
      return Math.abs(b - a);
    }
    near(span(60, v) / span(480, v), span(60, undefined) / span(480, undefined), 0.0001);
  });
});

// ============================================================================
// VIEWPORT INVARIANCE (#414)
//
// The specs above this block -- and every projection spec ported from Lua --
// only ever ran at one viewport, and Lua only ever ran at `vp == field`. That
// is exactly how a port that translated every module AND every test still
// shipped a projection whose entity sizes carried no viewport factor: the
// tests inherited the degenerate case along with the code. These are the
// properties that were silently false, written so they fail on the old
// formula at any viewport other than the field's own size.
// ============================================================================
describe("camera.project across viewports", () => {
  // The near (widest) edge of the pitch trapezoid, in screen pixels. This is
  // the thing a player's size has to stay in proportion to -- "the players
  // read as ~20px specks on a full-width pitch" is precisely this ratio going
  // wrong.
  function pitchWidth(vp: CameraViewport): number {
    const [left] = camera.project(0, field.h, field, vp);
    const [right] = camera.project(field.w, field.h, field, vp);
    return right - left;
  }

  // Top (far edge) to bottom (near edge) of the same trapezoid.
  function pitchHeight(vp: CameraViewport): number {
    const [, top] = camera.project(field.w / 2, 0, field, vp);
    const [, bottom] = camera.project(field.w / 2, field.h, field, vp);
    return bottom - top;
  }

  // What every entity's drawn size is derived from: `r = radius * scale`
  // (pitch.ts), and from there the rigged characters' pixels-per-metre, the
  // goal frames, the ball, the shadows and the reticles.
  function entityScale(vp: CameraViewport): number {
    const [, , scale] = camera.project(field.w / 2, field.h, field, vp);
    return scale;
  }

  const SAME_ASPECT: readonly CameraViewport[] = [
    { w: 960, h: 540 }, // the LÖVE window, and the only case the old specs covered
    { w: 1280, h: 720 },
    { w: 1920, h: 1080 },
    { w: 2560, h: 1440 },
  ];

  const OTHER_ASPECTS: readonly CameraViewport[] = [
    { w: 3440, h: 1440 }, // ultrawide 21:9 -- the display this was reported from
    { w: 1280, h: 1024 }, // 5:4
    { w: 1024, h: 1366 }, // portrait tablet
  ];

  it("keeps an entity's size in constant proportion to the pitch across viewports", () => {
    const reference = entityScale(SAME_ASPECT[0]!) / pitchWidth(SAME_ASPECT[0]!);
    for (const vp of [...SAME_ASPECT, ...OTHER_ASPECTS]) {
      near(entityScale(vp) / pitchWidth(vp), reference, 1e-12);
    }
  });

  it("scales entity size with the viewport, not just position -- doubling the window doubles both", () => {
    const base = { w: 960, h: 540 };
    const doubled = { w: 1920, h: 1080 };
    near(pitchWidth(doubled) / pitchWidth(base), 2, 1e-9);
    near(entityScale(doubled) / entityScale(base), 2, 1e-9);
  });

  it("scales the pitch rather than stretching it: the trapezoid keeps its aspect at every viewport aspect ratio", () => {
    const reference = pitchWidth(SAME_ASPECT[0]!) / pitchHeight(SAME_ASPECT[0]!);
    for (const vp of [...SAME_ASPECT, ...OTHER_ASPECTS]) {
      near(pitchWidth(vp) / pitchHeight(vp), reference, 1e-9);
    }
  });

  it("fits the whole trapezoid inside every viewport -- the spare space on the long axis is starfield, not a clipped pitch", () => {
    for (const vp of [...SAME_ASPECT, ...OTHER_ASPECTS]) {
      const [nearLeft, nearY] = camera.project(0, field.h, field, vp);
      const [nearRight] = camera.project(field.w, field.h, field, vp);
      const [, farY] = camera.project(field.w / 2, 0, field, vp);
      expect(nearLeft).toBeGreaterThanOrEqual(0);
      expect(nearRight).toBeLessThanOrEqual(vp.w);
      expect(farY).toBeGreaterThanOrEqual(0);
      expect(nearY).toBeLessThanOrEqual(vp.h);
    }
  });

  it("keeps the pitch centred, so the spare space is split evenly instead of pushed to one side", () => {
    for (const vp of [...SAME_ASPECT, ...OTHER_ASPECTS]) {
      const [nearLeft] = camera.project(0, field.h, field, vp);
      const [nearRight] = camera.project(field.w, field.h, field, vp);
      near((nearLeft + nearRight) / 2, vp.w / 2, 1e-9);
    }
  });
});

describe("camera perspective mode", () => {
  function withPerspective(fn: () => void): void {
    const saved = camera.perspective_mode;
    camera.perspective_mode = true;
    try {
      fn();
    } finally {
      camera.perspective_mode = saved;
    }
  }

  it("centres the focus on screen", () => {
    withPerspective(() => {
      const v = camera.view(300, 200, field, 1);
      const [x, y] = camera.project(v.x, v.y, field, vp, undefined, v);
      near(x, vp.w / 2, 0.5);
      near(y, vp.h / 2, 0.5);
    });
  });

  it("converges: a far span is narrower than an equal near span", () => {
    withPerspective(() => {
      const v = camera.view(480, 270, field, 1);
      function span(wy: number): number {
        const [a] = camera.project(300, wy, field, vp, undefined, v);
        const [b] = camera.project(660, wy, field, vp, undefined, v);
        return Math.abs(b - a);
      }
      // +y runs toward the viewer, so a low y is the far touchline.
      expect(span(60) < span(480)).toBe(true);
    });
  });

  it("scales a player with depth, larger when nearer", () => {
    withPerspective(() => {
      const v = camera.view(480, 270, field, 1);
      const [, , far] = camera.project(480, 60, field, vp, undefined, v);
      const [, , near_] = camera.project(480, 480, field, vp, undefined, v);
      expect(near_ > far).toBe(true);
      expect(far > 0).toBe(true);
    });
  });

  it("pushes a point behind the camera out of frame rather than mirroring it", () => {
    withPerspective(() => {
      // Clamping a negative w to a small positive would place the point at a
      // finite but wildly wrong spot on screen; it has to leave frame.
      const v = camera.view(480, 270, field, 1);
      // +y runs toward the viewer and the eye sits beyond the focus on that
      // side, so a large +y is behind the camera. (A large -y is merely very
      // far away, and correctly converges on the vanishing point rather than
      // leaving frame.)
      const [x, y] = camera.project(480, 100000, field, vp, undefined, v);
      expect(x < -1000 || x > vp.w + 1000).toBe(true);
      expect(y < -1000 || y > vp.h + 1000).toBe(true);
    });
  });
});

// ============================================================================
// LUA DIFFERENTIAL -- see v2/tools/render_reference/README.md (the pitch
// differential's own tool directory; this capture used the same headless
// `love .` pattern but needs no love.graphics stub at all, since
// `camera.project` is pure -- no love.* calls anywhere in camera.lua). This
// is the leading hypothesis check named in this task's brief: if v2's camera
// or pixels-per-metre differs from the Lua original, everything on screen
// moves at the wrong apparent speed/distance even with identical sim state.
//
// Captured with a small standalone script (field 960x540, at TWO viewports --
// see below for why two):
//
//   local camera = require("game.render.camera")
//   local sx, sy, scale = camera.project(wx, wy, field, vp, cfg, view)
//   print(string.format("%.17g\t%.17g\t%.17g", sx, sy, scale))
//
// run for the fixed (default) projection, a fixed projection under a 2x
// zoomed follow view, and the perspective-mode projection under a 1x follow
// view -- the three distinct code paths `camera.project` can take.
//
// TWO VIEWPORTS, AND ONE DELIBERATE DIVERGENCE (#414)
//
// 960x540 is the viewport the LÖVE build actually renders at, and the only
// one it can: conf.lua pins a non-resizable 960x540 window and
// sim/env_config.lua's DEFAULT_FIELD is 960x540, so `vp == field` in every
// LÖVE frame ever drawn. (An earlier revision of this comment called 1280x720
// "the product's own dimensions". It never was.) There, v2 must and does
// match Lua on all three returned values.
//
// 1280x720 is a viewport only v2 can be in, and it is kept because it pins
// the one place the two builds now differ ON PURPOSE. Lua's projection puts
// the world-to-pixel factor into screen POSITIONS and leaves the depth
// `scale` -- the only input to every entity size -- a pure ratio, so a bigger
// window grew the pitch and left the players the same number of pixels.
// camera.ts's `projectFixed` folds a single uniform fit factor into both.
// The consequences, both asserted below:
//
//   * POSITIONS are unchanged at any viewport with the field's aspect ratio.
//     This fix reframes nothing; the 1280x720 sx/sy rows below are still
//     matched exactly, character for character with the Lua capture.
//   * `scale` is multiplied by that fit factor -- 1280/960 == 720/540 == 4/3
//     here. That IS the fix, and asserting it against the Lua number times
//     4/3 states the divergence precisely instead of hiding it behind a
//     re-baselined golden.
// ============================================================================

interface CameraReferenceRow {
  readonly wx: number;
  readonly wy: number;
  readonly sx: number;
  readonly sy: number;
  readonly scale: number;
}

// Viewport 960x540 -- `vp == field`, the LÖVE build's only configuration.
// prettier-ignore
const FIXED_REFERENCE_AT_FIELD_SIZE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 235.19999999999999, sy: 129.59999999999999, scale: 0.51000000000000001 },
  { wx: 960, wy: 0, sx: 724.79999999999995, sy: 129.59999999999999, scale: 0.51000000000000001 },
  { wx: 0, wy: 540, sx: 76.800000000000011, sy: 475.20000000000005, scale: 0.83999999999999997 },
  { wx: 960, wy: 540, sx: 883.20000000000005, sy: 475.20000000000005, scale: 0.83999999999999997 },
  { wx: 480, wy: 270, sx: 480, sy: 302.39999999999998, scale: 0.67500000000000004 },
  { wx: 123.456, wy: 78.9, sx: 280.97119680000003, sy: 180.096, scale: 0.55821666666666669 },
  { wx: -50, wy: 600, sx: 15.366666666666674, sy: 513.60000000000002, scale: 0.87666666666666671 },
];

// prettier-ignore
const FIXED_ZOOM_REFERENCE_AT_FIELD_SIZE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 218, sy: 13.999999999999943, scale: 1.02 },
  { wx: 960, wy: 0, sx: 1197.1999999999998, sy: 13.999999999999943, scale: 1.02 },
  { wx: 0, wy: 540, sx: -98.799999999999955, sy: 705.20000000000005, scale: 1.6799999999999999 },
  { wx: 960, wy: 540, sx: 1514, sy: 705.20000000000005, scale: 1.6799999999999999 },
  { wx: 480, wy: 270, sx: 707.60000000000002, sy: 359.59999999999991, scale: 1.3500000000000001 },
  { wx: 123.456, wy: 78.9, sx: 309.54239360000008, sy: 114.99199999999996, scale: 1.1164333333333334 },
  { wx: -50, wy: 600, sx: -221.66666666666663, sy: 782, scale: 1.7533333333333334 },
];

// Viewport 1280x720 -- a viewport only v2 can be in. See the block comment
// above: sx/sy must still match Lua exactly, `scale` must be Lua's times the
// uniform fit factor.
// prettier-ignore
const FIXED_REFERENCE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 313.60000000000002, sy: 172.79999999999998, scale: 0.51000000000000001 },
  { wx: 960, wy: 0, sx: 966.39999999999998, sy: 172.79999999999998, scale: 0.51000000000000001 },
  { wx: 0, wy: 540, sx: 102.40000000000009, sy: 633.60000000000002, scale: 0.83999999999999997 },
  { wx: 960, wy: 540, sx: 1177.5999999999999, sy: 633.60000000000002, scale: 0.83999999999999997 },
  { wx: 480, wy: 270, sx: 640, sy: 403.20000000000005, scale: 0.67500000000000004 },
  { wx: 123.456, wy: 78.9, sx: 374.62826240000004, sy: 240.12799999999999, scale: 0.55821666666666669 },
  { wx: -50, wy: 600, sx: 20.488888888888937, sy: 684.80000000000007, scale: 0.87666666666666671 },
];

// prettier-ignore
const FIXED_ZOOM_REFERENCE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 290.66666666666674, sy: 18.666666666666572, scale: 1.02 },
  { wx: 960, wy: 0, sx: 1596.2666666666667, sy: 18.666666666666572, scale: 1.02 },
  { wx: 0, wy: 540, sx: -131.73333333333312, sy: 940.26666666666665, scale: 1.6799999999999999 },
  { wx: 960, wy: 540, sx: 2018.6666666666665, sy: 940.26666666666665, scale: 1.6799999999999999 },
  { wx: 480, wy: 270, sx: 943.4666666666667, sy: 479.4666666666667, scale: 1.3500000000000001 },
  { wx: 123.456, wy: 78.9, sx: 412.72319146666678, sy: 153.32266666666658, scale: 1.1164333333333334 },
  { wx: -50, wy: 600, sx: -295.55555555555543, sy: 1042.6666666666667, scale: 1.7533333333333334 },
];

// prettier-ignore
const PERSPECTIVE_REFERENCE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: -213.83897478161117, sy: 52.529404792105957, scale: 0.61401333930083757 },
  { wx: 960, wy: 0, sx: 1493.8389747816111, sy: 52.529404792105957, scale: 0.61401333930083757 },
  { wx: 0, wy: 540, sx: -5016.6832079281721, sy: 2396.9926932522976, scale: 4.0678383728680467 },
  { wx: 960, wy: 540, sx: 6296.6832079281721, sy: 2396.9926932522976, scale: 4.0678383728680467 },
  { wx: 480, wy: 270, sx: 640, sy: 360.00000000000006, scale: 1.0669739994407994 },
  { wx: 123.456, wy: 78.9, sx: -84.055450614806375, sy: 111.55831353284962, scale: 0.70097376376832288 },
  { wx: -50, wy: 600, sx: -16015.789445566254, sy: 6999.087296525996, scale: 10.847568994314772 },
];

describe("camera.project differential against the real Lua game.render.camera", () => {
  const bigField: CameraField = { w: 960, h: 540 };
  const bigVp: CameraViewport = { w: 1280, h: 720 };
  const fieldSizedVp: CameraViewport = { w: 960, h: 540 };
  // 1280/960 == 720/540. See the block comment above.
  const FIT_1280x720 = 4 / 3;

  it("matches the Lua reference EXACTLY -- all three returned values -- at the viewport LÖVE actually renders at (vp == field, conf.lua's pinned 960x540 window)", () => {
    for (const row of FIXED_REFERENCE_AT_FIELD_SIZE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, fieldSizedVp);
      near(sx, row.sx, 1e-6);
      near(sy, row.sy, 1e-6);
      near(scale, row.scale, 1e-9);
    }
  });

  it("matches the Lua reference exactly at vp == field under a 2x zoomed follow view", () => {
    const view = camera.view(300, 200, bigField, 2);
    for (const row of FIXED_ZOOM_REFERENCE_AT_FIELD_SIZE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, fieldSizedVp, undefined, view);
      near(sx, row.sx, 1e-6);
      near(sy, row.sy, 1e-6);
      near(scale, row.scale, 1e-9);
    }
  });

  it("keeps Lua's exact screen POSITIONS at a larger same-aspect viewport, and diverges only by folding the uniform fit factor into the depth scale (#414)", () => {
    for (const row of FIXED_REFERENCE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp);
      near(sx, row.sx, 1e-6);
      near(sy, row.sy, 1e-6);
      near(scale, row.scale * FIT_1280x720, 1e-9);
    }
  });

  it("keeps Lua's exact screen positions under a 2x zoomed follow view at that same larger viewport, with the same single scale divergence", () => {
    const view = camera.view(300, 200, bigField, 2);
    for (const row of FIXED_ZOOM_REFERENCE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp, undefined, view);
      near(sx, row.sx, 1e-6);
      near(sy, row.sy, 1e-6);
      near(scale, row.scale * FIT_1280x720, 1e-9);
    }
  });

  it("matches the Lua reference for perspective mode under a 1x follow view", () => {
    const saved = camera.perspective_mode;
    camera.perspective_mode = true;
    try {
      const view = camera.view(480, 270, bigField, 1);
      for (const row of PERSPECTIVE_REFERENCE) {
        const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp, undefined, view);
        // A wider epsilon than the fixed-mode tests: mat4.lookAt/perspective
        // route through trigonometric functions, which v2/README.md's
        // determinism rules explicitly call out as implementation-
        // approximated across runtimes (ECMAScript spec) -- irrelevant to
        // the sim (this is presentation-only, per that same README section),
        // but real enough to need a looser tolerance than a pure arithmetic
        // path here.
        near(sx, row.sx, 1e-3);
        near(sy, row.sy, 1e-3);
        near(scale, row.scale, 1e-6);
      }
    } finally {
      camera.perspective_mode = saved;
    }
  });
});
