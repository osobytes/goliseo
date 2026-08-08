// Ported from game/render/camera.lua.
//
// Pure 2.5D projection. Maps a point on the flat pitch (world space) to a
// screen point plus a depth scale, producing a perspective trapezoid: the far
// edge (world y = 0) is higher and narrower, the near edge (world y = field.h)
// is lower and wider. No three.js/DOM calls, so the projection is
// unit-testable headless.

import { mat4 } from "@gc/core";

export interface CameraConfig {
  /** sprite/spread scale at the far edge */
  readonly far_scale: number;
  /** sprite/spread scale at the near edge */
  readonly near_scale: number;
  /** screen-height fraction where the far edge sits */
  readonly horizon_frac: number;
  /** screen-height fraction where the near edge sits */
  readonly bottom_frac: number;
}

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

/** `[screenX, screenY, depthScale]`, matching the Lua original's 3 return values. */
export type CameraProjection = readonly [number, number, number];

// ---------------------------------------------------------------------------
// Perspective mode
// ---------------------------------------------------------------------------
//
// A real pinhole camera above and behind the focus, looking down at the pitch.
//
// The fixed trapezoid interpolates scale LINEARLY with depth, which is not what
// a camera does, and it maps whatever region it is given onto the same screen
// shape. Neither survives moving in close: magnifying a fake perspective just
// flattens it. This mode projects the ground plane properly, so convergence is
// a consequence of where the camera is rather than a fixed screen shape, and
// moving closer increases perspective the way it should.
//
// Everything on the pitch draws through camera.project, so switching modes
// moves lines, goals, players and effects together.

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
  // contribute. Lua's 1-based m[1], m[3], m[4], ... become 0-based m[0], m[2],
  // m[3], ... below.
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
  // This replaces a `scale_k / w` constant (scale_k = 582), which was
  // calibrated to match the fixed trapezoid's ~0.675 mid-pitch scale and
  // carried NEITHER of the two terms above. Both omissions were live bugs:
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

// The fixed whole-pitch projection: world point -> screen point + depth scale.
function projectFixed(
  wx: number,
  wy: number,
  field: CameraField,
  vp: CameraViewport,
  cfg: CameraConfig,
): CameraProjection {
  const t = wy / field.h; // 0 = far, 1 = near
  const scale = cfg.far_scale + (cfg.near_scale - cfg.far_scale) * t;
  const horizon = vp.h * cfg.horizon_frac;
  const bottom = vp.h * cfg.bottom_frac;
  const sy = horizon + (bottom - horizon) * t;
  const sx = vp.w / 2 + (wx - field.w / 2) * scale * (vp.w / field.w);
  return [sx, sy, scale];
}

/** Pure 2.5D projection module. See file header. */
export const camera = {
  // Tuned so the pitch is inset within the viewport (margins on all sides)
  // rather than filling the screen -- the arena floats in space, like a
  // broadcast frame. Keep near_scale < 1 so even the widest (near) edge stays
  // off the screen edges.
  /** Opt-in: use the real perspective camera instead of the fixed trapezoid. */
  perspective_mode: false,

  DEFAULTS: {
    far_scale: 0.51, // wide far edge (less of a sharp wedge), but inset
    near_scale: 0.84, // < 1: near edge sits ~8% in from each side
    horizon_frac: 0.24, // space/HUD band above the pitch
    bottom_frac: 0.88, // margin below the pitch
  } satisfies CameraConfig as CameraConfig,

  PERSPECTIVE: {
    // STRIKERS REFRAME. A true-perspective broadcast rig for the coliseum,
    // tuned against Mario Strikers reference stills, on the product's actual
    // 960x540 field (v2/README.md's own reference dimensions -- see
    // camera.spec.ts's differential fixtures).
    //
    // What "match Strikers" reduces to, measurably: a player reads at
    // roughly a TENTH of frame height, and the camera FOLLOWS play at a
    // moderate downward tilt instead of holding a fixed establishing shot of
    // the whole stadium. The second half is not this table's job -- see
    // `pitch.follow_camera` / camera_follow.ts, which the product entry now
    // switches on alongside `perspective_mode` (browser_main.ts). This table
    // only sets the shot's geometry.
    //
    // Derivation, kept here rather than left as bare numbers because every
    // input matters if this ever needs retuning again:
    //
    //   1. TILT. Chosen, not derived: 45 degrees down from horizontal.
    //      `tilt = atan(height / distance)`, so 45 degrees is exactly
    //      `height == distance` -- which is why both numbers below are the
    //      same, and why the ratio is verifiable by eye. Shallower than the
    //      50 degrees this rig used while it framed the whole bowl: 50 looks
    //      down ONTO the pitch (a stadium establishing angle), 45 reads more
    //      along it, which is the reference's flatter, more side-on look.
    //   2. LENS. `fov: 46` (vertical, degrees) -- unchanged by this retune.
    //   3. DISTANCE. Derived from the CHARACTER SIZE target rather than from
    //      a whole-pitch fit; that inversion is the point of this retune. A
    //      character's drawn height in pixels is `6 * radius * scale`
    //      (pitch.ts's `r = radius * scale`, then player_renderer_3d.ts's
    //      `ppmForRadius` times the mesh's own metre height, which cancels
    //      to `r * HEIGHT_IN_RADII * 2` = `6 * r`), and `scale` at the focus
    //      is `vp.h / (2 * L * tan(fov / 2))` (`projectPerspective`'s DEPTH
    //      SCALE note). `vp.h` cancels, so the FRACTION of frame height a
    //      player occupies is resolution-independent and a property of the
    //      rig alone:
    //        player_frac = 6 * PLAYER_RADIUS / (2 * L * tan(fov / 2))
    //                    = 72 / (2 * L * tan(23 deg))
    //      Solving for the reference's ~12%:
    //        L = 72 / (0.121 * 2 * 0.42447) ~= 700 world units.
    //      (`PLAYER_RADIUS` is 12 world units -- gc-sim's match.rs, exported
    //      to the renderer as `PLAYER_RADIUS_PX`.)
    //   4. HEIGHT/DISTANCE. Split `L` by the tilt: at 45 degrees both are
    //      `L / sqrt(2)` ~= 495. Recovered from the rounded pair below:
    //      tilt exactly 45 degrees, `L = sqrt(2) * 495` ~= 700.0,
    //      `player_frac` ~= 12.1%.
    //
    // Note what is deliberately NOT here anymore: a `scale_k` sprite-size
    // calibration constant. Character size is now derived from this rig plus
    // the viewport (`projectPerspective`'s DEPTH SCALE note explains why the
    // old constant was two live bugs), so retuning `fov`/`height`/`distance`
    // no longer needs a matching hand-recalibration to keep players the
    // right size relative to the stadium they stand in -- which is what made
    // every earlier retune in this file's history a two-step dance.
    height: 495,
    distance: 495,
    fov: 46,
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
  view(
    fx: number,
    fy: number,
    field: CameraField,
    zoom = 1,
    margin?: CameraMargin,
  ): CameraView {
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

  project(
    wx: number,
    wy: number,
    field: CameraField,
    vp: CameraViewport,
    cfg?: CameraConfig,
    view?: CameraView,
  ): CameraProjection {
    const c = cfg ?? camera.DEFAULTS;
    if (camera.perspective_mode) {
      return projectPerspective(wx, wy, field, vp, view);
    }
    const [sx, sy, scale] = projectFixed(wx, wy, field, vp, c);
    if (view === undefined || view.zoom <= 1) {
      return [sx, sy, scale];
    }

    // Magnify in SCREEN space about the focus, which is what a longer lens
    // does. The earlier attempt re-mapped a sub-rectangle of the pitch onto
    // the same fixed trapezoid, which forced full convergence onto a region
    // that should look almost rectangular -- the pitch came out as a funnel.
    //
    // Scaling the already-projected offsets keeps the perspective structure
    // the fixed view establishes: parallel lines stay as straight as they
    // were, the hex grid stays even, and only the framing changes.
    const z = view.zoom;
    const [fx, fy] = projectFixed(view.x, view.y, field, vp, c);
    return [vp.w * 0.5 + (sx - fx) * z, vp.h * 0.5 + (sy - fy) * z, scale * z];
  },

  /**
   * The perspective rig's downward tilt: the angle below horizontal the
   * camera looks at its focus, `atan(height-above-focus / horizontal
   * distance-to-focus)`. Used by pitch.ts to keep a rigged character's
   * elevation tilt COHERENT with whatever camera is actually looking at it
   * -- see that file's "CHARACTER TILT COHERENCE" section: under the fixed
   * trapezoid there is no real camera angle to match (the old constant,
   * `player_renderer_3d.ts`'s `ELEVATION`, approximates one), but under
   * `perspective_mode` there is a genuine camera pose, and a character
   * tilted to a DIFFERENT angle than the camera that is actually looking at
   * it would read as visibly wrong the moment it turns.
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
