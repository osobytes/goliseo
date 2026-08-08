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
function projectPerspective(
  wx: number,
  wy: number,
  field: CameraField,
  vp: CameraViewport,
  view: CameraView | undefined,
): CameraProjection {
  const cfg = camera.PERSPECTIVE;
  const fx = view !== undefined ? view.x : field.w / 2;
  const fy = view !== undefined ? view.y : field.h / 2;

  // Pitch (x, y) becomes world (x, 0, y): y runs from the far edge toward the
  // camera, so the camera sits at +Z beyond the focus.
  // Zoom pulls the camera in rather than magnifying the image, so closing in
  // strengthens the perspective instead of flattening it.
  const zoom = Math.max(0.25, view !== undefined ? view.zoom : 1);
  const eye: readonly [number, number, number] = [fx, cfg.height / zoom, fy + cfg.distance / zoom];
  const vm = mat4.lookAt(eye, [fx, 0, fy]);
  const pm = mat4.perspective(cfg.fov, vp.w / vp.h, 1, 8000);
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
    // Tuned against arcade soccer reference framing: a player near the focus
    // occupies roughly an eighth of the screen height, close enough to read a
    // face while still showing enough pitch to aim a pass.
    height: 180,
    distance: 216,
    fov: 45,
    scale_k: 300,
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
};
