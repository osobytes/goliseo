// Ported from spec/render/camera_spec.lua.

import { describe, expect, it } from "vitest";
import { camera, perspectiveRig, type CameraField, type CameraView, type CameraViewport } from "./camera.ts";

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

// New coverage for the eye/target/lens extraction (scene.ts's `SceneRoot`
// builds a `THREE.PerspectiveCamera` off this same rig -- see camera.ts's
// `PerspectiveRig` doc comment) and the tilt angle pitch.ts uses to keep a
// rigged character's elevation coherent with whatever camera is actually
// looking at it (see `camera.rigAngleRad`'s own doc comment).
describe("camera.perspectiveRig / camera.rigAngleRad", () => {
  it("looks straight at the default (no-view) focus: the pitch centre, at ground level", () => {
    const rig = perspectiveRig(field);
    expect(rig.target).toEqual([field.w / 2, 0, field.h / 2]);
    expect(rig.eye[0]).toBe(field.w / 2);
    expect(rig.eye[2]).toBeGreaterThan(field.h / 2); // eye sits beyond the focus on +Z
  });

  it("centres eye/target on a supplied view's focus instead of the pitch centre", () => {
    const view: CameraView = { x: 300, y: 200, zoom: 1 };
    const rig = perspectiveRig(field, view);
    expect(rig.eye[0]).toBe(300);
    expect(rig.target).toEqual([300, 0, 200]);
  });

  it("pulls the eye toward the focus as zoom increases, without changing the tilt angle", () => {
    const zoomedOut = perspectiveRig(field, { x: 480, y: 270, zoom: 1 });
    const zoomedIn = perspectiveRig(field, { x: 480, y: 270, zoom: 2 });
    const distanceTo = (rig: ReturnType<typeof perspectiveRig>): number => Math.hypot(rig.eye[0] - rig.target[0], rig.eye[1] - rig.target[1], rig.eye[2] - rig.target[2]);
    expect(distanceTo(zoomedIn)).toBeLessThan(distanceTo(zoomedOut));
    near(camera.rigAngleRad(field, { x: 480, y: 270, zoom: 1 }), camera.rigAngleRad(field, { x: 480, y: 270, zoom: 2 }), 1e-9);
  });

  it("carries fov/near/far from camera.PERSPECTIVE, not from any viewport (aspect is applied at use time, not baked into the rig)", () => {
    const rig = perspectiveRig(field);
    expect(rig.fov).toBe(camera.PERSPECTIVE.fov);
    expect(rig.near).toBe(1);
    expect(rig.far).toBe(8000);
  });

  it("rigAngleRad reports a downward tilt of roughly 50 degrees, matching camera.PERSPECTIVE's tuned framing", () => {
    const deg = (camera.rigAngleRad(field) * 180) / Math.PI;
    near(deg, 50, 0.1);
  });

  it("rigAngleRad matches atan(height / distance) directly off camera.PERSPECTIVE", () => {
    const expected = Math.atan2(camera.PERSPECTIVE.height, camera.PERSPECTIVE.distance);
    near(camera.rigAngleRad(field), expected, 1e-9);
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
// Captured with a small standalone script (field 960x540, viewport 1280x720
// -- the product's own dimensions, unlike pitch.spec.ts's shrunk fixture,
// since camera.project draws nothing and has no hex-floor-style blowup):
//
//   local camera = require("game.render.camera")
//   local sx, sy, scale = camera.project(wx, wy, field, vp, cfg, view)
//   print(string.format("%.17g\t%.17g\t%.17g", sx, sy, scale))
//
// run for the fixed (default) projection, a fixed projection under a 2x
// zoomed follow view, and the perspective-mode projection under a 1x follow
// view -- the three distinct code paths `camera.project` can take.
// ============================================================================

interface CameraReferenceRow {
  readonly wx: number;
  readonly wy: number;
  readonly sx: number;
  readonly sy: number;
  readonly scale: number;
}

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

// PERSPECTIVE RETUNE (camera.ts's `camera.PERSPECTIVE` doc comment has the
// full derivation). This table used to be an independent Lua capture, the
// same way FIXED_REFERENCE/FIXED_ZOOM_REFERENCE still are -- but
// `camera.PERSPECTIVE` was deliberately retuned away from the Lua original's
// close-in framing (height 180/distance 216/fov 45) to a whole-pitch
// broadcast angle for the true-perspective Strikers camera work, so a byte-
// for-byte Lua capture at the OLD tuning is no longer a meaningful
// regression target for the NEW one -- the two cameras are not the same shot
// on purpose. What still needs pinning is that `projectPerspective`'s MATH
// (the mat4 lookAt/perspective/multiply pipeline, now routed through
// `perspectiveRig` -- see camera.ts) keeps producing exactly what that
// pipeline computes for the current tuning, so a future refactor cannot
// silently perturb it. These rows are therefore recomputed directly from the
// pipeline at the new tuning (same sample points, same view, same
// field/viewport as before) rather than re-captured from Lua; the "camera
// perspective mode" describe block above is what still protects the
// CONVERGENCE/FORESHORTENING invariants independent of tuning.
//
// Regenerated for the GAMEPLAY REFRAME (height 722->660, distance 606->554,
// same 50-degree tilt ratio; fov 50->46; scale_k 636->582 -- the polish
// pass's fov-50 full-bowl view was a beautiful establishing shot but shrank
// the pitch to ~55% of frame height, which is not the Strikers reference;
// see camera.ts's own PERSPECTIVE comment for the framing decision) with a
// scratch vitest snippet evaluating this same unchanged pipeline at the new
// PERSPECTIVE tuning, per this comment's own instructions.
// prettier-ignore
const PERSPECTIVE_REFERENCE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 246.7822395897798, sy: 190.58702093460187, scale: 0.5621656440384241 },
  { wx: 960, wy: 0, sx: 1033.2177604102203, sy: 190.58702093460187, scale: 0.5621656440384241 },
  { wx: 0, wy: 540, sx: 48.388103959085385, sy: 614.8886236833745, scale: 0.8458007649798662 },
  { wx: 960, wy: 540, sx: 1231.6118960409146, sy: 614.8886236833745, scale: 0.8458007649798662 },
  { wx: 480, wy: 270, sx: 640, sy: 359.99999999999994, scale: 0.6754140279591309 },
  { wx: 123.456, wy: 78.9, sx: 332.86916081227673, sy: 233.91541064110814, scale: 0.5911296002784843 },
  { wx: -50, wy: 600, sx: -52.033544888599295, sy: 690.0321465572539, scale: 0.896032350390394 },
];

describe("camera.project differential against the real Lua game.render.camera", () => {
  const bigField: CameraField = { w: 960, h: 540 };
  const bigVp: CameraViewport = { w: 1280, h: 720 };

  it("matches the Lua reference for the fixed (default) projection -- the product's actual field/viewport, not a shrunk test fixture", () => {
    for (const row of FIXED_REFERENCE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp);
      near(sx, row.sx, 1e-6);
      near(sy, row.sy, 1e-6);
      near(scale, row.scale, 1e-9);
    }
  });

  it("matches the Lua reference for the fixed projection under a 2x zoomed follow view", () => {
    const view = camera.view(300, 200, bigField, 2);
    for (const row of FIXED_ZOOM_REFERENCE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp, undefined, view);
      near(sx, row.sx, 1e-6);
      near(sy, row.sy, 1e-6);
      near(scale, row.scale, 1e-9);
    }
  });

  it("pins projectPerspective's mat4 pipeline against a precomputed reference at the current PERSPECTIVE tuning (no longer a Lua capture -- see PERSPECTIVE_REFERENCE's own comment)", () => {
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
