// Pure 2.5D projection. Maps a point on the flat pitch (world space) to a
// screen point plus a depth scale, through a REAL pinhole camera sitting
// above and behind the focus and looking down at the pitch: the far edge
// (world y = 0) comes out higher and narrower, the near edge (world y =
// field.h) lower and wider, because that is what a camera in that pose does
// to a ground plane -- not because any screen shape was authored. No
// three.js/DOM calls, so the projection is unit-testable headless.
//
// There is ONE projection. An earlier fixed-trapezoid mode interpolated
// scale linearly with depth (which is not what a camera does) and mapped
// whatever region it was handed onto the same authored screen shape (so
// magnifying it just flattened the fake perspective). It was retired once
// the product entry point stopped ever selecting it -- see `camera.PERSPECTIVE`
// for the rig this projection is tuned by, and `PerspectiveRig` for why
// `scene.ts`'s world-layer camera is built from that same derivation rather
// than a second, independently-tuned copy of "the same shot".

import { mat4 } from "@gc/core";

export interface CameraView {
  /** focus world x */
  readonly x: number;
  /** focus world y */
  readonly y: number;
  /** 1 = whole pitch, >1 = magnified about the focus */
  readonly zoom: number;
}

export interface CameraField {
  readonly w: number;
  readonly h: number;
}

export interface CameraViewport {
  readonly w: number;
  readonly h: number;
}

export interface CameraMargin {
  readonly x: number;
  readonly y: number;
}

export interface CameraPerspectiveConfig {
  /** world units above the pitch */
  readonly height: number;
  /** world units back from the focus */
  readonly distance: number;
  /** degrees, vertical */
  readonly fov: number;
}

/** `[screenX, screenY, depthScale]`. */
export type CameraProjection = readonly [number, number, number];

// ---------------------------------------------------------------------------
// The projection
// ---------------------------------------------------------------------------
//
// Convergence is a consequence of where the camera IS, not of a screen shape
// anyone authored, so moving in close strengthens the perspective instead of
// flattening it (`perspectiveRig` pulls the eye toward the focus rather than
// magnifying the image -- see its `zoom` note).
//
// Everything on the pitch draws through camera.project, so the lines, goals,
// players and effects all move together with the rig below.

/**
 * The camera rig `projectPerspective` derives internally: where the eye
 * sits, what it looks at, and the lens. Extracted (rather than left inline)
 * so `scene.ts`'s `SceneRoot` can build a REAL `THREE.PerspectiveCamera` off
 * the exact same numbers this file's own `mat4.lookAt`/`mat4.perspective`
 * path uses -- the world-layer camera and this 2.5D projection's camera are
 * then bit-for-bit the same camera, not two independently-tuned
 * approximations of "the same shot" that drift the moment one gets retuned
 * and the other does not. `aspect` is deliberately NOT part of this rig: it
 * comes from the viewport at USE time (`vp.w / vp.h` here,
 * `viewport.w / viewport.h` in `scene.ts`), because a `PerspectiveRig` is a
 * property of the field/view/PERSPECTIVE config alone, while the viewport
 * can change (a window resize) without moving the camera at all.
 */
export interface PerspectiveRig {
  readonly eye: readonly [number, number, number];
  readonly target: readonly [number, number, number];
  /** degrees, vertical */
  readonly fov: number;
  readonly near: number;
  readonly far: number;
}

const PERSPECTIVE_NEAR = 1;
const PERSPECTIVE_FAR = 8000;

/** Pure eye/target/lens derivation for perspective mode. See `PerspectiveRig`. */
export function perspectiveRig(field: CameraField, view?: CameraView): PerspectiveRig {
  const cfg = camera.PERSPECTIVE;
  const fx = view !== undefined ? view.x : field.w / 2;
  const fy = view !== undefined ? view.y : field.h / 2;

  // Pitch (x, y) becomes world (x, 0, y): y runs from the far edge toward the
  // camera, so the camera sits at +Z beyond the focus.
  // Zoom pulls the camera in rather than magnifying the image, so closing in
  // strengthens the perspective instead of flattening it.
  const zoom = Math.max(0.25, view !== undefined ? view.zoom : 1);
  return {
    eye: [fx, cfg.height / zoom, fy + cfg.distance / zoom],
    target: [fx, 0, fy],
    fov: cfg.fov,
    near: PERSPECTIVE_NEAR,
    far: PERSPECTIVE_FAR,
  };
}

function projectPerspective(
  wx: number,
  wy: number,
  field: CameraField,
  vp: CameraViewport,
  view: CameraView | undefined,
): CameraProjection {
  const rig = perspectiveRig(field, view);
  // `mat4.lookAt` builds its basis from a fixed world-up of (0, 1, 0) (see
  // mat4.ts's own `lookAt` -- `cross(0, 1, 0, ...)`), the SAME default
  // `THREE.Camera`/`THREE.Object3D.up` starts with. That is what keeps this
  // hand-rolled matrix path and a `THREE.PerspectiveCamera` built from the
  // same `rig` (scene.ts's `SceneRoot`) pointed the same way for the same
  // eye/target -- neither side has to special-case a roll.
  const vm = mat4.lookAt(rig.eye, rig.target);
  const pm = mat4.perspective(rig.fov, vp.w / vp.h, rig.near, rig.far);
  const m = mat4.multiply(pm, vm);

  // World Y is always 0 on the flat pitch, so the column-1 (world-Y) terms
  // drop out entirely -- only the world-X (wx) and world-Z (wy) columns
  // contribute.
  const x = m[0] * wx + m[2] * wy + m[3];
  const y = m[4] * wx + m[6] * wy + m[7];
  const w = m[12] * wx + m[14] * wy + m[15];
  // Behind the camera. Clamping a negative w to a small positive would not
  // just avoid the divide -- it would flip the point to a finite but wildly
  // wrong screen position instead of pushing it out of frame.
  if (w < 0.0001) {
    return [-1e6, -1e6, 0];
  }

  // DEPTH SCALE = SCREEN PIXELS PER WORLD UNIT AT THIS DEPTH.
  //
  // Every consumer multiplies an authored WORLD-unit size by this to get a
  // pixel size: pitch.ts's `r = radius * scale` (`radius` is the sim's own
  // `PLAYER_RADIUS_PX`, 12 world units -- `gc-sim`'s match.rs calls it
  // "world units" in as many words), the ball's `5 * scale`, marker rings,
  // health bars, `lineWidth`s. So the factor has to be exactly the
  // world->pixel ratio a perspective camera actually produces:
  //
  //   px_per_world(w) = vp.h / (2 * w * tan(fov / 2))
  //
  // which is just the standard symmetric-perspective vertical mapping: the
  // frustum is `2 * w * tan(fov/2)` world units tall at clip-depth `w`, and
  // that height maps onto `vp.h` pixels. No calibration constant is needed
  // or wanted -- the pitch is 960x540 WORLD units and `@gc/ui`'s virtual
  // canvas (viewport.ts's `baseW`/`baseH`) is 960x540 PIXELS, so one world
  // unit is one virtual pixel by construction and the ratio is absolute.
  //
  // This replaces a `scale_k / w` constant (scale_k = 582), hand-calibrated
  // against the mid-pitch scale of the fixed trapezoid this projection
  // replaced, which carried NEITHER of the two terms above. Both omissions
  // were live bugs:
  //
  //   1. NO `vp.h`. Character pixel size was independent of resolution while
  //      the stadium -- drawn by a REAL `THREE.PerspectiveCamera` off this
  //      same rig (scene.ts's `syncWorldCamera`) -- scaled with `vp.h` as a
  //      perspective camera must. So players did not "grow with the window":
  //      at 540p a player read at the intended ~9% of frame height, and at
  //      1235p at ~4%, shrinking against the pitch as the window grew. The
  //      menu half of the app never had this bug because `viewport.create`
  //      letterboxes the whole virtual canvas; the match path takes raw
  //      pixel `vp` and so has to apply the ratio itself, which it did for
  //      POSITIONS (`* vp.w`, `* vp.h` above) but not for SIZES.
  //   2. NO `fov`. A lens change moved the world geometry without resizing
  //      the characters standing on it, so any `fov` retune silently
  //      desynced the two -- the sprites had to be hand-recalibrated
  //      afterwards to catch up (see this file's own retune history).
  //
  // Deriving both means character size now tracks the resolution and the
  // lens automatically, and stays locked to the stadium under either.
  // Authored visual weight ("how tall is a player, in radii") is
  // player_renderer_3d.ts's `HEIGHT_IN_RADII`, which is where that
  // judgement belongs -- not smuggled into the projection as a constant.
  const halfFovTan = Math.tan((rig.fov * Math.PI) / 360);
  return [(x / w + 1) * 0.5 * vp.w, (1 - y / w) * 0.5 * vp.h, vp.h / (2 * w * halfFovTan)];
}

/** Pure 2.5D projection module. See file header. */
export const camera = {
  PERSPECTIVE: {
    // BROADCAST REFRAME. A true-perspective rig for the coliseum, now
    // matched to a REFERENCE CAMERA TABLE rather than to stills, on the
    // product's 1648x927 field (see camera.spec.ts's differential fixtures).
    //
    // What "match the reference" reduces to, measurably: a player reads at
    // roughly a TENTH of frame height, and the camera FOLLOWS play at a
    // moderate downward tilt instead of holding a fixed establishing shot of
    // the whole stadium. The second half is not this table's job -- see
    // `pitch.follow_camera` / camera_follow.ts, which the product entry
    // switches on (browser_main.ts). This table only sets the shot's
    // geometry.
    //
    // Derivation, kept here rather than left as bare numbers because every
    // input matters if this ever needs retuning again:
    //
    //   1. LENS. `fov: 27` (VERTICAL, degrees). The reference stores 33.4
    //      for its 4:3 mode and 44.3 for its widescreen mode, but those are
    //      NOT vertical angles: its projection helper converts what it is
    //      handed with `2 * atan(tan(in / 2) / aspect)` before building the
    //      matrix, which is the horizontal-to-vertical conversion. Applying
    //      it with the aspect values that code actually uses -- 1.3323944
    //      and 1.666, the second being anamorphic-widescreen compensation
    //      rather than 16/9 -- gives true vertical angles of 25.38 and
    //      27.46 degrees. We take the widescreen one and round to 27.
    //      Reading the stored pair as already-vertical would imply
    //      horizontal angles of 43.6 vs 71.9 between the two display modes,
    //      which no lens design would intend. This is much longer than the
    //      46 this rig used while it was tuned by eye, and the flattened
    //      perspective is most of what separates the reference's look from
    //      ours.
    //   2. TILT. 38 degrees down from horizontal, the reference's own far
    //      preset (its near preset is 25). `tilt = atan(height / distance)`.
    //      Shallower again than the 45 this rig used, and far shallower than
    //      the 50 it used while framing the whole bowl.
    //   3. DISTANCE. Taken from the reference directly: 35.0 of its world
    //      units on its far preset, and its gravity literal pins one of its
    //      units to one metre. At this project's rig scale -- a 1.75 m
    //      player drawn `PLAYER_RADIUS * HEIGHT_IN_RADII * 2` = 72 px tall,
    //      so 24.31 mm per world unit -- that is `35.0 / 0.02431` ~= 1440
    //      world units of straight-line camera-to-focus distance `L`.
    //   4. CHARACTER SIZE, as a check rather than as the input. A
    //      character's drawn height in pixels is `6 * radius * scale`
    //      (pitch.ts's `r = radius * scale`, then player_renderer_3d.ts's
    //      `ppmForRadius` times the mesh's own metre height, which cancels
    //      to `r * HEIGHT_IN_RADII * 2` = `6 * r`), and `scale` at the focus
    //      is `vp.h / (2 * L * tan(fov / 2))` (`projectPerspective`'s DEPTH
    //      SCALE note). `vp.h` cancels, so the FRACTION of frame height a
    //      player occupies is resolution-independent:
    //        player_frac = 6 * PLAYER_RADIUS / (2 * L * tan(fov / 2))
    //                    = 72 / (2 * 1440.5 * tan(13.5 deg)) ~= 10.4%
    //      Close to the ~12% the by-eye retune had solved for, which is the
    //      reassuring part: the lens and the distance both moved a long way,
    //      and they moved in opposite directions, so the players did not
    //      change size. What changed is the perspective.
    //   5. HEIGHT/DISTANCE. Split `L` by the tilt:
    //        height   = L * sin(38 deg) = 1440 * 0.61566 ~= 887
    //        distance = L * cos(38 deg) = 1440 * 0.78801 ~= 1135
    //      Both agree with the reference's own tabulated split of its far
    //      preset -- height 21.548 and ground offset 27.580 of its units,
    //      which at 24.31 mm per unit are 887 and 1135.
    //      Recovered from the rounded pair below: `atan(887 / 1135)` is
    //      38.00 degrees and `sqrt(887^2 + 1135^2)` is 1440.5.
    //
    // NOT ADOPTED from the reference table, deliberately: its near preset
    // (distance 20, tilt 25) and the runtime zoom that interpolates toward
    // it. This table sets one static shot; a dynamic zoom belongs with
    // `pitch.follow_camera` in camera_follow.ts, not here.
    height: 887,
    distance: 1135,
    fov: 27,
  } satisfies CameraPerspectiveConfig as CameraPerspectiveConfig,

  // Clamped focus for a following camera.
  //
  // `margin` is how far, in world units, the focus must stay inside each edge
  // of the pitch. The caller supplies it because the right value depends on
  // the projection: a lens zoom needs the whole frame to stay over the pitch,
  // while a real camera only needs to not fly off the end of it -- and must
  // still be able to reach the goals.
  //
  // Deriving the margin from `field / (2 * zoom)` (the previous behaviour)
  // pins the focus to the exact centre of the pitch at zoom 1, so the camera
  // cannot follow anything at all.
  view(fx: number, fy: number, field: CameraField, zoom = 1, margin?: CameraMargin): CameraView {
    const z = Math.max(0.25, zoom);
    let mx = margin !== undefined ? margin.x : field.w / (2 * z);
    let my = margin !== undefined ? margin.y : field.h / (2 * z);
    mx = Math.min(mx, field.w / 2);
    my = Math.min(my, field.h / 2);
    return {
      x: Math.max(mx, Math.min(field.w - mx, fx)),
      y: Math.max(my, Math.min(field.h - my, fy)),
      zoom: z,
    };
  },

  /**
   * The whole renderer's world-to-screen entry point. `view` is optional:
   * without one the camera frames the pitch centre at zoom 1, which is what
   * every non-following caller (and `SceneRoot`'s own `syncWorldCamera` when
   * `pitch.follow_camera` is off) gets.
   */
  project(
    wx: number,
    wy: number,
    field: CameraField,
    vp: CameraViewport,
    view?: CameraView,
  ): CameraProjection {
    return projectPerspective(wx, wy, field, vp, view);
  },

  /**
   * The perspective rig's downward tilt: the angle below horizontal the
   * camera looks at its focus, `atan(height-above-focus / horizontal
   * distance-to-focus)`. Used by pitch.ts to keep a rigged character's
   * elevation tilt COHERENT with the camera actually looking at it -- see
   * that file's "CHARACTER TILT COHERENCE" section. There is a genuine
   * camera pose to match here, and a character tilted to a DIFFERENT angle
   * than the camera actually looking at it reads as visibly wrong the moment
   * it turns. (The retired fixed trapezoid had no real camera angle to
   * match, which is why `player_renderer_3d.ts`'s `ELEVATION` constant used
   * to stand in for one.)
   *
   * Note this is invariant to `view.zoom`: `perspectiveRig` scales `height`
   * and `distance` by the SAME `1 / zoom` factor (see that function), so
   * their ratio -- and therefore this angle -- does not change as a follow
   * view zooms in or out. Only the eye's DISTANCE from the focus changes;
   * the ANGLE it looks down at does not. `field`/`view` are still threaded
   * through (rather than reading `camera.PERSPECTIVE` directly) so this
   * stays keyed off the same `PerspectiveRig` derivation as everything
   * else, in case that invariant ever stops holding for some future rig
   * shape.
   */
  rigAngleRad(field: CameraField, view?: CameraView): number {
    const rig = perspectiveRig(field, view);
    const heightAboveFocus = rig.eye[1] - rig.target[1];
    const horizontalDistance = Math.hypot(rig.eye[0] - rig.target[0], rig.eye[2] - rig.target[2]);
    return Math.atan2(heightAboveFocus, horizontalDistance);
  },
};
