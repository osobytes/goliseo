// Quaternions, for interpolating and blending rotations.
//
// WHY THIS EXISTS
//
// Clips are authored as Euler angles because that is what a human can read and
// edit. Euler angles are a fine *authoring* format and a bad *interpolation*
// format, and the difference only shows up once you blend:
//
//   * Component-wise Euler lerp does not trace the shortest rotation. Blending
//     yaw -170 to +170 the naive way sweeps 340 degrees the wrong way round
//     instead of stepping 20 degrees across the seam.
//   * Two axes changing at once trace a path that is not a rotation about any
//     fixed axis, so a blend can dip through poses neither keyframe contains --
//     an arm can pass through the torso between two poses that both clear it.
//   * Near gimbal lock the mapping from angles to orientation is degenerate and
//     small angle changes produce large orientation changes.
//
// Playing a single clip mostly hides all three. Layering a strike over a run,
// or crossfading between clips (which #98 requires), does not.
//
// THE FIX, AND WHY IT IS CHEAP
//
// Author in Euler, bake to quaternions once at load, interpolate and blend as
// quaternions, convert to a matrix at the end. Nothing in clips.lua has to
// change for a human. This is the same split every production pipeline uses:
// Blender authors Euler curves, glTF stores quaternions.
//
// Storage is [x, y, z, w].

import type { Mat4 } from "./mat4.ts";

/** A quaternion, stored as [x, y, z, w]. */
export type Quat = readonly [number, number, number, number];

function identity(): Quat {
  return [0, 0, 0, 1];
}

/** @param angle radians */
function axisAngle(ax: number, ay: number, az: number, angle: number): Quat {
  const h = angle * 0.5;
  const s = Math.sin(h);
  return [ax * s, ay * s, az * s, Math.cos(h)];
}

// Returns a * b: the rotation that applies b first, then a.
function multiply(a: Quat, b: Quat): Quat {
  const [ax, ay, az, aw] = a;
  const [bx, by, bz, bw] = b;
  return [
    aw * bx + ax * bw + ay * bz - az * by,
    aw * by - ax * bz + ay * bw + az * bx,
    aw * bz + ax * by - ay * bx + az * bw,
    aw * bw - ax * bx - ay * by - az * bz,
  ];
}

// Euler in the slice's convention: Y * X * Z, so a limb is rolled (Z), then
// pitched (X), then yawed (Y). Matches mat4.euler exactly, which is what lets
// the switch to quaternions be visually invisible.
/**
 * @param rx radians
 * @param ry radians
 * @param rz radians
 */
function fromEuler(rx: number, ry: number, rz: number): Quat {
  return multiply(multiply(axisAngle(0, 1, 0, ry), axisAngle(1, 0, 0, rx)), axisAngle(0, 0, 1, rz));
}

function normalize(q: Quat): Quat {
  const [x, y, z, w] = q;
  const len = Math.sqrt(x * x + y * y + z * z + w * w);
  if (len < 1e-12) {
    return identity();
  }
  return [x / len, y / len, z / len, w / len];
}

// Spherical linear interpolation, always along the SHORT arc.
//
// The sign flip is the part people forget: q and -q are the same orientation,
// so without it a blend can take the 340-degree route. Below a very small angle
// slerp's division becomes unstable, so it falls back to a normalized lerp --
// indistinguishable at that scale.
function slerp(a: Quat, b: Quat, t: number): Quat {
  let [bx, by, bz, bw] = b;
  let dot = a[0] * bx + a[1] * by + a[2] * bz + a[3] * bw;

  if (dot < 0) {
    bx = -bx;
    by = -by;
    bz = -bz;
    bw = -bw;
    dot = -dot;
  }

  if (dot > 0.9995) {
    return normalize([
      a[0] + (bx - a[0]) * t,
      a[1] + (by - a[1]) * t,
      a[2] + (bz - a[2]) * t,
      a[3] + (bw - a[3]) * t,
    ]);
  }

  const theta = Math.acos(dot);
  const sinTheta = Math.sin(theta);
  const s0 = Math.sin((1 - t) * theta) / sinTheta;
  const s1 = Math.sin(t * theta) / sinTheta;
  return [a[0] * s0 + bx * s1, a[1] * s0 + by * s1, a[2] * s0 + bz * s1, a[3] * s0 + bw * s1];
}

// Builds a row-major 4x4 with this rotation and the given translation, matching
// mat4.ts's storage so the two are interchangeable downstream.
function toMat4(q: Quat, tx: number, ty: number, tz: number): Mat4 {
  const [x, y, z, w] = q;
  const x2 = x + x;
  const y2 = y + y;
  const z2 = z + z;
  const xx = x * x2;
  const xy = x * y2;
  const xz = x * z2;
  const yy = y * y2;
  const yz = y * z2;
  const zz = z * z2;
  const wx = w * x2;
  const wy = w * y2;
  const wz = w * z2;

  // prettier-ignore
  return [
    1 - (yy + zz), xy - wz,       xz + wy,       tx,
    xy + wz,       1 - (xx + zz), yz - wx,       ty,
    xz - wy,       yz + wx,       1 - (xx + yy), tz,
    0,             0,             0,             1,
  ];
}

/** Quaternions, for interpolating and blending rotations. */
export const quat = {
  identity,
  multiply,
  fromEuler,
  normalize,
  slerp,
  toMat4,
};
