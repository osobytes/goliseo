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

// ============================================================================
// VIEWPORT INVARIANCE (#414)
//
// The specs above this block only ever ran at one viewport, matching the
// single viewport the original renderer ever ran at (`vp == field`). That is
// exactly how a projection whose entity sizes carried no viewport factor
// shipped undetected: the tests exercised only the degenerate case, so a
// viewport-invariance property this projection was supposed to have went
// completely unchecked. These are the properties that were silently false,
// written so they fail on the old formula at any viewport other than the
// field's own size.
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
    { w: 960, h: 540 }, // the original renderer's only window size, and the only case the old specs covered
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

  // The depth scale is SCREEN PIXELS PER WORLD UNIT (camera.ts's
  // `projectPerspective` DEPTH SCALE note). Every consumer multiplies an
  // authored world-unit size by it -- pitch.ts's `r = radius * scale` for a
  // character, the ball's `5 * scale`, marker rings, health bars -- so these
  // four tests pin the two terms a previous `scale_k / w` constant omitted.
  // Both omissions were live bugs, and the FIRST one is the reason players
  // read as specks in a large window: character pixel size did not grow with
  // the window while the stadium (drawn by a real `THREE.PerspectiveCamera`
  // off this same rig -- scene.ts's `syncWorldCamera`) did.
  //
  // `PLAYER_FRAME_FRACTION_NUMERATOR` is a character's drawn height in
  // "scale units": `6 * PLAYER_RADIUS` = `6 * 12`, since pitch.ts's
  // `r = radius * scale` feeds player_renderer_3d.ts's `ppmForRadius`, whose
  // mesh-height division cancels to `r * HEIGHT_IN_RADII * 2` = `6 * r`.
  const PLAYER_FRAME_FRACTION_NUMERATOR = 6 * 12;

  it("scales linearly with viewport height, so a character grows with the window", () => {
    withPerspective(() => {
      const v = camera.view(480, 270, field, 1);
      const at = (h: number): number => camera.project(480, 270, field, { w: (h * 16) / 9, h }, undefined, v)[2];
      const base = at(540);
      near(at(1080), base * 2, 1e-9);
      near(at(1350), base * 2.5, 1e-9);
    });
  });

  it("keeps a character's share of frame height resolution-independent", () => {
    withPerspective(() => {
      const v = camera.view(480, 270, field, 1);
      const frac = (w: number, h: number): number =>
        (PLAYER_FRAME_FRACTION_NUMERATOR * camera.project(480, 270, field, { w, h }, undefined, v)[2]) / h;
      // The whole point of deriving the scale rather than calibrating it: the
      // fraction is a property of the RIG (see camera.PERSPECTIVE's step 3),
      // so it must not move with the window at all.
      near(frac(960, 540), frac(1280, 720), 1e-9);
      near(frac(960, 540), frac(2560, 1080), 1e-9);
      near(frac(960, 540), frac(3000, 1235), 1e-9);
      // ...and it must land on the framing that derivation solved for.
      near(frac(1280, 720), 0.121, 5e-4);
    });
  });

  it("is independent of viewport WIDTH: an ultrawide window sees more pitch, it does not resize the players", () => {
    withPerspective(() => {
      // The rig's fov is VERTICAL and `syncWorldCamera` applies aspect to the
      // horizontal axis only, so widening the window must widen the view
      // without touching how big anything standing in it draws.
      const v = camera.view(480, 270, field, 1);
      const narrow = camera.project(480, 270, field, { w: 960, h: 540 }, undefined, v)[2];
      const ultrawide = camera.project(480, 270, field, { w: 2400, h: 540 }, undefined, v)[2];
      near(ultrawide, narrow, 1e-9);
    });
  });

  it("tracks the lens: a wider fov draws the same world smaller", () => {
    withPerspective(() => {
      const saved = camera.PERSPECTIVE;
      try {
        const v = camera.view(480, 270, field, 1);
        const scaleAtFov = (fov: number): number => {
          (camera as { PERSPECTIVE: typeof saved }).PERSPECTIVE = { ...saved, fov };
          return camera.project(480, 270, field, vp, undefined, v)[2];
        };
        // Previously the sprite scale was a constant, so retuning fov moved
        // the stadium and left the characters behind -- every retune in
        // camera.ts's history then needed a matching hand-recalibration.
        expect(scaleAtFov(60)).toBeLessThan(scaleAtFov(30));
      } finally {
        (camera as { PERSPECTIVE: typeof saved }).PERSPECTIVE = saved;
      }
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

  it("rigAngleRad reports a downward tilt of 45 degrees, matching camera.PERSPECTIVE's Strikers framing", () => {
    const deg = (camera.rigAngleRad(field) * 180) / Math.PI;
    near(deg, 45, 0.1);
  });

  it("puts the tilt at 45 degrees by construction: height and distance are equal", () => {
    // camera.PERSPECTIVE's derivation picks the tilt FIRST and splits L by it,
    // and 45 degrees is the one tilt where that split is verifiable by eye.
    expect(camera.PERSPECTIVE.height).toBe(camera.PERSPECTIVE.distance);
  });

  it("rigAngleRad matches atan(height / distance) directly off camera.PERSPECTIVE", () => {
    const expected = Math.atan2(camera.PERSPECTIVE.height, camera.PERSPECTIVE.distance);
    near(camera.rigAngleRad(field), expected, 1e-9);
  });
});

// ============================================================================
// PINNED REFERENCE VALUES -- see tools/render_reference/README.md (the
// pitch differential's own tool directory; this capture used the same
// headless capture pattern, though `camera.project` needed no rendering
// stub at all, since it is pure -- the reference implementation had no
// rendering calls anywhere in its own camera projection). This was the
// leading hypothesis check for a rewrite of this scope: if this camera or
// its pixels-per-metre differs from the reference implementation, everything
// on screen moves at the wrong apparent speed/distance even with identical
// sim state.
//
// Captured with a small standalone script against the reference
// implementation (field 960x540, at TWO viewports -- see below for why
// two), run for the fixed (default) projection, a fixed projection under a
// 2x zoomed follow view, and the perspective-mode projection under a 1x
// follow view -- the three distinct code paths `camera.project` can take.
//
// TWO VIEWPORTS, AND ONE DELIBERATE DIVERGENCE (#414)
//
// 960x540 is the viewport the reference implementation actually rendered
// at, and the only one it could: it pinned a non-resizable 960x540 window
// and a 960x540 default field, so `vp == field` in every frame it ever
// drew. (An earlier revision of this comment called 1280x720 "the
// product's own dimensions". It never was.) There, this renderer must and
// does match the reference on all three returned values.
//
// 1280x720 is a viewport only this renderer can be in, and it is kept
// because it pins the one place the two now differ ON PURPOSE. The
// reference projection puts the world-to-pixel factor into screen
// POSITIONS and leaves the depth `scale` -- the only input to every entity
// size -- a pure ratio, so a bigger window grew the pitch and left the
// players the same number of pixels. camera.ts's `projectFixed` folds a
// single uniform fit factor into both. The consequences, both asserted
// below:
//
//   * POSITIONS are unchanged at any viewport with the field's aspect ratio.
//     This fix reframes nothing; the 1280x720 sx/sy rows below are still
//     matched exactly, character for character with the reference capture.
//   * `scale` is multiplied by that fit factor -- 1280/960 == 720/540 == 4/3
//     here. That IS the fix, and asserting it against the reference number
//     times 4/3 states the divergence precisely instead of hiding it behind
//     a re-baselined golden.
// ============================================================================

interface CameraReferenceRow {
  readonly wx: number;
  readonly wy: number;
  readonly sx: number;
  readonly sy: number;
  readonly scale: number;
}

// Viewport 960x540 -- `vp == field`, the reference implementation's only configuration.
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

// Viewport 1280x720 -- a viewport only this renderer can be in. See the
// block comment above: sx/sy must still match the reference exactly,
// `scale` must be the reference's times the uniform fit factor.
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

// PERSPECTIVE REFERENCE (camera.ts's `camera.PERSPECTIVE` doc comment has the
// full derivation). This table used to be an independent reference capture,
// the same way FIXED_REFERENCE/FIXED_ZOOM_REFERENCE still are -- but
// `camera.PERSPECTIVE` was deliberately retuned away from the reference
// implementation's framing for the true-perspective Strikers camera work,
// so a byte-for-byte reference capture at the OLD tuning is no longer a
// meaningful regression target for the NEW one -- the two cameras are not
// the same shot on purpose. What still needs pinning is that
// `projectPerspective`'s MATH (the mat4 lookAt/perspective/multiply
// pipeline, routed through `perspectiveRig` -- see camera.ts) keeps
// producing exactly what that pipeline computes for the current tuning, so
// a future refactor cannot silently perturb it. These rows are therefore
// recomputed directly from the pipeline at the current tuning (same sample
// points, same view, same field/viewport as the reference capture used)
// rather than re-captured from the reference implementation; the "camera
// perspective mode" describe block above is what still protects the
// CONVERGENCE/FORESHORTENING invariants independent of tuning.
//
// Regenerated for the STRIKERS REFRAME (height 660 -> 495, distance
// 554 -> 495 -- tilt 50 -> 45 degrees, L 862 -> 700; fov 46 unchanged; and
// the `scale_k` constant REMOVED, the `scale` column now being the derived
// `vp.h / (2 * w * tan(fov/2))` pixels-per-world-unit -- see camera.ts's
// `projectPerspective` DEPTH SCALE note for why that constant was a bug),
// with a scratch vitest snippet evaluating this same unchanged pipeline at
// the new PERSPECTIVE tuning, per this comment's own instructions.
//
// Note the `scale` column is now ~1.2 at the focus rather than ~0.68: it is a
// real pixels-per-world-unit ratio at THIS viewport (1280x720), not a
// dimensionless sprite fudge, so it necessarily scales with `bigVp.h`. The
// "scales linearly with viewport height" test below is what pins that
// property independently of these rows.
// prettier-ignore
const PERSPECTIVE_REFERENCE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 183.0841097101677, sy: 178.26281749359623, scale: 0.9519081047704837 },
  { wx: 960, wy: 0, sx: 1096.9158902898323, sy: 178.26281749359623, scale: 0.9519081047704837 },
  { wx: 0, wy: 540, sx: -159.6028080072064, sy: 678.0400693862066, scale: 1.6658391833483466 },
  { wx: 960, wy: 540, sx: 1439.6028080072062, sy: 678.0400693862066, scale: 1.6658391833483466 },
  { wx: 480, wy: 270, sx: 640, sy: 360, scale: 1.2115194060715249 },
  { wx: 123.456, wy: 78.9, sx: 277.9304247166352, sy: 222.77773316466775, scale: 1.0154975971643465 },
  { wx: -50, wy: 600, sx: -323.1579278268623, sy: 784.0534258482754, scale: 1.8172791091072868 },
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

  it("pins projectPerspective's mat4 pipeline against a precomputed reference at the current PERSPECTIVE tuning (no longer a Lua capture -- see PERSPECTIVE_REFERENCE's own comment)", () => {
    const saved = camera.perspective_mode;
    camera.perspective_mode = true;
    try {
      const view = camera.view(480, 270, bigField, 1);
      for (const row of PERSPECTIVE_REFERENCE) {
        const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp, undefined, view);
        // A wider epsilon than the fixed-mode tests: mat4.lookAt/perspective
        // route through trigonometric functions, which ARCHITECTURE.md §1's
        // determinism rules explicitly call out as implementation-
        // approximated across runtimes (ECMAScript spec) -- irrelevant to
        // the sim (this is presentation-only, per that same section),
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
