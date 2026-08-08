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
  readonly scale_k: number;
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
  const cfg = camera.PERSPECTIVE;
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

  return [(x / w + 1) * 0.5 * vp.w, (1 - y / w) * 0.5 * vp.h, cfg.scale_k / w];
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
    // Retuned for whole-pitch Strikers-style broadcast framing on the
    // product's actual 960x540 field at a 16:9 viewport (v2/README.md's own
    // reference dimensions -- see camera.spec.ts's differential fixtures).
    // Previously (height 180 / distance 216 / fov 45) this rig framed a
    // CLOSE-IN shot -- right for reading one player's face, wrong for a
    // broadcast angle that has to show the whole pitch. Derivation, redone
    // here rather than left as bare numbers because every input matters if
    // this ever needs retuning again:
    //
    //   1. TILT. Chosen first, not derived: 50 degrees down from horizontal
    //      is the Strikers/broadcast reference angle this task specifies --
    //      steep enough to read depth/occlusion between players, shallow
    //      enough that the far touchline does not collapse to a sliver.
    //      `tilt = atan(height / distance)`, so height/distance below are
    //      picked to land on 50 degrees, not the other way around.
    //   2. LENS. `fov: 42` (vertical, degrees) is the other free choice --
    //      picked, like the tilt, for the reference framing rather than
    //      derived from anything else. Horizontal FOV follows from it and
    //      the 16:9 aspect: `hFov = 2 * atan(tan(fov/2) * aspect)`, giving
    //      hFov ~= 68.6 degrees (half-angle ~= 34.3 degrees).
    //   3. DISTANCE. Fit so the pitch width, padded 18% for margin (matching
    //      DEFAULTS' own inset framing above), stays within that horizontal
    //      FOV at the eye-to-focus (pitch centre) distance L:
    //      `L = (field.w / 2 * 1.18) / tan(hFov / 2)`
    //        = (480 * 1.18) / tan(34.3 deg) ~= 830 world units.
    //      This is a deliberately simple fit -- it calibrates the
    //      horizontal budget at the FOCUS point's distance, not at the
    //      pitch's nearest (and therefore widest-appearing) corners, which
    //      sit closer to the eye and so eat slightly more of the frame than
    //      this budget assumes. Checked against the tuned numbers below:
    //      the near corners land about 19px outside a 1280-wide viewport
    //      (~1.5%), i.e. barely clipped at the very edge -- an accepted,
    //      documented trade-off for a broadcast angle, not an error,
    //      matching how real sports broadcast cameras routinely crop the
    //      two nearest corners.
    //   4. HEIGHT/DISTANCE. Split `L` by the tilt: `height = L * sin(tilt)`,
    //      `distance = L * cos(tilt)`. Rounded to whole world units below
    //      (656 / 551) -- their ratio still lands tilt at ~49.97 degrees,
    //      well inside "~50" for a hand-tuned broadcast angle.
    //
    //   VISUAL POLISH RETUNE (stadium art-direction pass, creative brief item
    //   2 -- "show the sky"): the framing above put the far bowl rim right at
    //   (or past) the top screen edge, leaving no sky above it. Pulled the
    //   whole rig back ~10% along the SAME tilt ratio (both height and
    //   distance scaled by the same 1.1 factor, so the 50-degree tilt angle
    //   -- camera.rigAngleRad's own invariant -- is unchanged; only the
    //   eye's distance from the focus grows): 656 -> 722, 551 -> 606. Distance
    //   alone was not enough on its own -- a live scan of the rendered frame
    //   (a scratch script projecting the bowl's own tier/rim world points
    //   through this exact rig) showed the pullback moves the WHOLE bowl
    //   toward screen centre (a dolly-back converges off-axis geometry
    //   inward, it does not shrink it out of frame), so `fov` also went
    //   42 -> 50 -- a genuinely wider lens is what actually buys vertical
    //   headroom above the bowl, paired with a shorter bowl itself
    //   (stadium_layout.ts's `tierHeight`/`arcadeHeight`, cut for the same
    //   reason). Confirmed live: the nebula and arcade silhouette are now
    //   visible along both top corners of the broadcast frame.
    //   GAMEPLAY REFRAME (after the polish pass): fov 50 at L ~= 943 showed
    //   the WHOLE bowl -- a beautiful establishing shot, but the pitch shrank
    //   to ~55% of frame height, which is not the Strikers reference (pitch
    //   dominant, crowd as a band along the top). Tightened to fov 46 at
    //   L ~= 862 (height/distance re-split on the same 50-degree tilt):
    //   the pitch regains the frame while the shorter bowl (the polish
    //   pass's tierHeight/arcadeHeight cut) keeps the far arcade + nebula
    //   sky visible in the top corners. Verified live against screenshots
    //   of both tunings side by side.
    height: 660,
    distance: 554,
    fov: 46,
    // SCALE_K. `projectPerspective`'s returned scale is `scale_k / w_clip`
    // (`w_clip` being the clip-space w at the projected point -- see that
    // function's return). Retuned so a player standing at the pitch centre
    // projects at roughly the SAME on-screen radius the fixed trapezoid
    // path already gives there today, so switching `perspective_mode` on
    // does not also make every sprite jump to a wildly different size:
    //   fixed-path scale at centre ~= (far_scale + near_scale) / 2
    //                              = (0.51 + 0.84) / 2 = 0.675
    //   w_clip at the pitch centre, under THIS rig (height 722 / distance
    //   606, 16:9 viewport, zoom 1, no follow view) ~= 942.6 -- note this
    //   does NOT depend on fov (a standard symmetric perspective matrix's
    //   clip-space w is -view-space-z, which fov does not touch; only the
    //   x/y clip components scale with fov), so the fov 42 -> 50 visual
    //   polish retune above did not require rederiving this number.
    //   scale_k = 0.675 * w_clip; at the GAMEPLAY REFRAME's rig (height 660 /
    //   distance 554) w_clip at the pitch centre is L = sqrt(660^2 + 554^2)
    //   ~= 861.6, so scale_k = 0.675 * 861.6 ~= 581.6, rounded: 582
    //     (recovered centre scale: 582 / 861.6 ~= 0.6755, ~0.07% off
    //     target -- well within "roughly the same radius")
    //   Recomputed the same way on every retune (a scratch vitest snippet
    //   evaluating this same unchanged pipeline, per this file's own
    //   convention -- see camera.spec.ts's PERSPECTIVE_REFERENCE comment).
    scale_k: 582,
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
