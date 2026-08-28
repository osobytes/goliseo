import { describe, expect, it } from "vitest";
import {
  camera,
  perspectiveRig,
  type CameraField,
  type CameraView,
  type CameraViewport,
} from "./camera.ts";

function near(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

const field: CameraField = { w: 960, h: 540 };
const vp: CameraViewport = { w: 1280, h: 720 };

describe("camera.view", () => {
  it("zoom 1 leaves the projection untouched", () => {
    const v = camera.view(120, 90, field, 1);
    const [a, b, c] = camera.project(300, 200, field, vp);
    const [x, y, z] = camera.project(300, 200, field, vp, v);
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
    const [x, y] = camera.project(v.x, v.y, field, vp, v);
    near(x, vp.w / 2, 0.001);
    near(y, vp.h / 2, 0.001);
  });
});

describe("camera.project (the perspective rig)", () => {
  it("centres the focus on screen", () => {
    const v = camera.view(300, 200, field, 1);
    const [x, y] = camera.project(v.x, v.y, field, vp, v);
    near(x, vp.w / 2, 0.5);
    near(y, vp.h / 2, 0.5);
  });

  it("converges: a far span is narrower than an equal near span", () => {
    const v = camera.view(480, 270, field, 1);
    function span(wy: number): number {
      const [a] = camera.project(300, wy, field, vp, v);
      const [b] = camera.project(660, wy, field, vp, v);
      return Math.abs(b - a);
    }
    // +y runs toward the viewer, so a low y is the far touchline.
    expect(span(60) < span(480)).toBe(true);
  });

  it("scales a player with depth, larger when nearer", () => {
    const v = camera.view(480, 270, field, 1);
    const [, , far] = camera.project(480, 60, field, vp, v);
    const [, , near_] = camera.project(480, 480, field, vp, v);
    expect(near_ > far).toBe(true);
    expect(far > 0).toBe(true);
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
    const v = camera.view(480, 270, field, 1);
    const at = (h: number): number => camera.project(480, 270, field, { w: (h * 16) / 9, h }, v)[2];
    const base = at(540);
    near(at(1080), base * 2, 1e-9);
    near(at(1350), base * 2.5, 1e-9);
  });

  it("keeps a character's share of frame height resolution-independent", () => {
    const v = camera.view(480, 270, field, 1);
    const frac = (w: number, h: number): number =>
      (PLAYER_FRAME_FRACTION_NUMERATOR * camera.project(480, 270, field, { w, h }, v)[2]) / h;
    // The whole point of deriving the scale rather than calibrating it: the
    // fraction is a property of the RIG (see camera.PERSPECTIVE's step 3),
    // so it must not move with the window at all.
    near(frac(960, 540), frac(1280, 720), 1e-9);
    near(frac(960, 540), frac(2560, 1080), 1e-9);
    near(frac(960, 540), frac(3000, 1235), 1e-9);
    // ...and it must land on the framing that derivation solved for.
    // 72 / (2 * L * tan(fov/2)) with L = hypot(887, 1135) ~= 1440.48 and
    // fov = 27 degrees (camera.PERSPECTIVE's own doc comment, step 4) ~=
    // 0.1041.
    near(frac(1280, 720), 0.104, 5e-4);
  });

  it("is independent of viewport WIDTH: an ultrawide window sees more pitch, it does not resize the players", () => {
    // The rig's fov is VERTICAL and `syncWorldCamera` applies aspect to the
    // horizontal axis only, so widening the window must widen the view
    // without touching how big anything standing in it draws.
    const v = camera.view(480, 270, field, 1);
    const narrow = camera.project(480, 270, field, { w: 960, h: 540 }, v)[2];
    const ultrawide = camera.project(480, 270, field, { w: 2400, h: 540 }, v)[2];
    near(ultrawide, narrow, 1e-9);
  });

  // Rescued from a block that pinned these two properties on the retired
  // fixed projection, where they were #414's actual defect: that formula
  // fitted the field to viewport WIDTH horizontally while spanning a
  // fraction of viewport HEIGHT vertically, so the pitch stretched at any
  // aspect ratio other than the field's own. They are properties the live
  // rig must hold too -- and holds by construction, its fov being vertical
  // with aspect only widening the view -- so they are asserted here rather
  // than deleted alongside the projection that used to get them wrong.
  const ASPECTS: readonly CameraViewport[] = [
    { w: 960, h: 540 },
    { w: 1280, h: 720 },
    { w: 1920, h: 1080 },
    { w: 3440, h: 1440 }, // ultrawide 21:9 -- the display #414 was reported from
    { w: 1280, h: 1024 }, // 5:4
    { w: 1024, h: 1366 }, // portrait tablet
  ];

  it("scales the pitch rather than stretching it: its projected shape keeps one aspect at every viewport aspect ratio", () => {
    function projectedAspect(at: CameraViewport): number {
      const [left] = camera.project(0, field.h, field, at);
      const [right] = camera.project(field.w, field.h, field, at);
      const [, top] = camera.project(field.w / 2, 0, field, at);
      const [, bottom] = camera.project(field.w / 2, field.h, field, at);
      return (right - left) / (bottom - top);
    }
    const reference = projectedAspect(ASPECTS[0]!);
    for (const at of ASPECTS) {
      near(projectedAspect(at), reference, 1e-9);
    }
  });

  it("keeps the pitch centre line on the screen centre at every depth and every viewport", () => {
    for (const at of ASPECTS) {
      const [farX] = camera.project(field.w / 2, 0, field, at);
      const [nearX] = camera.project(field.w / 2, field.h, field, at);
      near(farX, at.w / 2, 1e-9);
      near(nearX, at.w / 2, 1e-9);
    }
  });

  it("tracks the lens: a wider fov draws the same world smaller", () => {
    const saved = camera.PERSPECTIVE;
    try {
      const v = camera.view(480, 270, field, 1);
      const scaleAtFov = (fov: number): number => {
        (camera as { PERSPECTIVE: typeof saved }).PERSPECTIVE = { ...saved, fov };
        return camera.project(480, 270, field, vp, v)[2];
      };
      // Previously the sprite scale was a constant, so retuning fov moved
      // the stadium and left the characters behind -- every retune in
      // camera.ts's history then needed a matching hand-recalibration.
      expect(scaleAtFov(60)).toBeLessThan(scaleAtFov(30));
    } finally {
      (camera as { PERSPECTIVE: typeof saved }).PERSPECTIVE = saved;
    }
  });

  it("pushes a point behind the camera out of frame rather than mirroring it", () => {
    // Clamping a negative w to a small positive would place the point at a
    // finite but wildly wrong spot on screen; it has to leave frame.
    const v = camera.view(480, 270, field, 1);
    // +y runs toward the viewer and the eye sits beyond the focus on that
    // side, so a large +y is behind the camera. (A large -y is merely very
    // far away, and correctly converges on the vanishing point rather than
    // leaving frame.)
    const [x, y] = camera.project(480, 100000, field, vp, v);
    expect(x < -1000 || x > vp.w + 1000).toBe(true);
    expect(y < -1000 || y > vp.h + 1000).toBe(true);
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
    const distanceTo = (rig: ReturnType<typeof perspectiveRig>): number =>
      Math.hypot(
        rig.eye[0] - rig.target[0],
        rig.eye[1] - rig.target[1],
        rig.eye[2] - rig.target[2],
      );
    expect(distanceTo(zoomedIn)).toBeLessThan(distanceTo(zoomedOut));
    near(
      camera.rigAngleRad(field, { x: 480, y: 270, zoom: 1 }),
      camera.rigAngleRad(field, { x: 480, y: 270, zoom: 2 }),
      1e-9,
    );
  });

  it("carries fov/near/far from camera.PERSPECTIVE, not from any viewport (aspect is applied at use time, not baked into the rig)", () => {
    const rig = perspectiveRig(field);
    expect(rig.fov).toBe(camera.PERSPECTIVE.fov);
    expect(rig.near).toBe(1);
    expect(rig.far).toBe(8000);
  });

  it("rigAngleRad reports a downward tilt of 38 degrees, matching camera.PERSPECTIVE's broadcast framing", () => {
    const deg = (camera.rigAngleRad(field) * 180) / Math.PI;
    near(deg, 38, 0.1);
  });

  it("puts the tilt at 38 degrees, recoverable as atan(height / distance)", () => {
    // camera.PERSPECTIVE's derivation still picks the tilt FIRST and splits
    // L by it (camera.ts's PERSPECTIVE doc comment, step 5) -- but 38
    // degrees, unlike the 45 this rig used before the broadcast reframe, is
    // not the self-verifying case where height and distance land on the
    // same number: height = L * sin(38 deg) and distance = L * cos(38 deg)
    // are two different numbers by construction now. What still holds, and
    // is what this asserts, is that recovering the tilt from that split
    // lands back on 38 degrees.
    const deg =
      (Math.atan2(camera.PERSPECTIVE.height, camera.PERSPECTIVE.distance) * 180) / Math.PI;
    near(deg, 38, 0.1);
  });

  it("rigAngleRad matches atan(height / distance) directly off camera.PERSPECTIVE", () => {
    const expected = Math.atan2(camera.PERSPECTIVE.height, camera.PERSPECTIVE.distance);
    near(camera.rigAngleRad(field), expected, 1e-9);
  });
});

// ============================================================================
// PINNED PIPELINE VALUES
//
// This block used to hold four Lua reference captures as well: the retired
// fixed projection at `vp == field` and at 1280x720, each with and without a
// 2x zoomed follow view. Those were a genuine differential -- captured from
// the LOVE original with a standalone script, and the leading-hypothesis
// check for a rewrite of that scope (if this camera or its pixels-per-metre
// differed from the original's, everything on screen would move at the wrong
// apparent speed and distance even with identical sim state). They went with
// the projection they pinned; the reference tree itself was deleted at #467,
// and docs/render_differential.md records what that work found.
//
// What is left is not a capture of anything external, and says so.
// `camera.PERSPECTIVE` was deliberately retuned away from the original's
// framing, so a byte-for-byte capture at the OLD tuning stopped being a
// meaningful regression target for the NEW one -- the two cameras are not
// the same shot, on purpose. What still needs pinning is that
// `projectPerspective`'s MATH (the mat4 lookAt/perspective/multiply
// pipeline, routed through `perspectiveRig` -- see camera.ts) keeps
// producing exactly what that pipeline computes at the current tuning, so a
// future refactor cannot silently perturb it. The rows below are therefore
// RECOMPUTED from the pipeline whenever the tuning moves, never re-captured;
// the describes above are what protect the convergence, foreshortening and
// viewport invariants independent of any tuning.
// ============================================================================

interface CameraReferenceRow {
  readonly wx: number;
  readonly wy: number;
  readonly sx: number;
  readonly sy: number;
  readonly scale: number;
}

// PERSPECTIVE REFERENCE. camera.ts's `camera.PERSPECTIVE` doc comment has
// the full derivation of the tuning these rows are evaluated at.
//
// Regenerated for the BROADCAST REFRAME (height 660 -> 495, distance
// 554 -> 495 -- tilt 50 -> 45 degrees, L 862 -> 700; fov 46 unchanged; and
// the `scale_k` constant REMOVED, the `scale` column now being the derived
// `vp.h / (2 * w * tan(fov/2))` pixels-per-world-unit -- see camera.ts's
// `projectPerspective` DEPTH SCALE note for why that constant was a bug),
// with a scratch vitest snippet evaluating this same unchanged pipeline at
// the new PERSPECTIVE tuning, per this comment's own instructions.
//
// Regenerated AGAIN, the same way, for the second BROADCAST REFRAME that
// matched this table to a reference camera table
// (height 495 -> 887, distance 495 -> 1135 -- tilt 45 -> 38 degrees,
// L 700 -> 1440.5; fov 46 -> 27, and now VERTICAL rather than an already-
// vertical-looking pair -- see camera.ts's `camera.PERSPECTIVE` doc comment,
// step 1, for why that conversion matters). Same sample points, same view,
// same field/viewport; only the tuning the pipeline is evaluated at moved.
//
// Note the `scale` column is now ~1.04 at the focus rather than ~1.2: it is
// a real pixels-per-world-unit ratio at THIS viewport (1280x720), not a
// dimensionless sprite fudge, so it necessarily scales with `bigVp.h`. The
// "scales linearly with viewport height" test below is what pins that
// property independently of these rows.
// prettier-ignore
const PERSPECTIVE_REFERENCE: readonly CameraReferenceRow[] = [
  { wx: 0, wy: 0, sx: 204.6304513009443, sy: 209.20193332363743, scale: 0.9070198931230327 },
  { wx: 960, wy: 0, sx: 1075.3695486990557, sy: 209.20193332363743, scale: 0.9070198931230327 },
  { wx: 0, wy: 540, sx: 53.75040737913551, sy: 563.0580811662155, scale: 1.2213533179601344 },
  { wx: 960, wy: 540, sx: 1226.2495926208644, sy: 563.0580811662155, scale: 1.2213533179601344 },
  { wx: 480, wy: 270, sx: 640, sy: 359.99999999999994, scale: 1.0409750979320844 },
  { wx: 123.456, wy: 78.9, sx: 303.97152981109514, sy: 249.09814087438846, scale: 0.9424600335131283 },
  { wx: -50, wy: 600, sx: -33.241255607358084, sy: 618.1213862103896, scale: 1.2702665200138832 },
];

describe("projectPerspective's mat4 pipeline, pinned", () => {
  const bigField: CameraField = { w: 960, h: 540 };
  const bigVp: CameraViewport = { w: 1280, h: 720 };

  it("reproduces exactly what the pipeline computes at the current PERSPECTIVE tuning (see PERSPECTIVE_REFERENCE's own comment: recomputed, not captured)", () => {
    const view = camera.view(480, 270, bigField, 1);
    for (const row of PERSPECTIVE_REFERENCE) {
      const [sx, sy, scale] = camera.project(row.wx, row.wy, bigField, bigVp, view);
      // A wider epsilon than a pure arithmetic path would need:
      // mat4.lookAt/perspective route through trigonometric functions, which
      // ARCHITECTURE.md §1's determinism rules explicitly call out as
      // implementation-approximated across runtimes (ECMAScript spec) --
      // irrelevant to the sim (this is presentation-only, per that same
      // section), but real enough to need the tolerance here.
      near(sx, row.sx, 1e-3);
      near(sy, row.sy, 1e-3);
      near(scale, row.scale, 1e-6);
    }
  });
});
