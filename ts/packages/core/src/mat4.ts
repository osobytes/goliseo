// Minimal 4x4 matrix math for the slice's 3D pipeline.
//
// Storage is a flat 16-element, 0-based array in ROW-MAJOR order:
//     m[0]  m[1]  m[2]  m[3]      <- row 0
//     m[4]  m[5]  m[6]  m[7]      <- row 1
//     m[8]  m[9]  m[10] m[11]     <- row 2
//     m[12] m[13] m[14] m[15]     <- row 3
// so a translation lives in m[3], m[7], m[11].
//
// mat4 is presentation-only (not on the determinism path), but the row-major
// layout is still a real constraint: it's preserved so quat.toMat4 stays
// interchangeable with it.

/**
 * A 4x4 matrix, stored flat and row-major. Fixed-length so element access
 * stays statically known under `noUncheckedIndexedAccess` — no `undefined`
 * to narrow away, no non-null assertions.
 */
export type Mat4 = readonly [
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
  number,
];

/** A plain (x, y, z) triple, used only for the `lookAt` eye/target inputs. */
type Vec3Tuple = readonly [number, number, number];

function identity(): Mat4 {
  return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}

// Returns a * b, i.e. "apply b first, then a" when acting on a column vector.
// The 4x4 case is unrolled so every term uses a literal index, which is what
// keeps this free of `| undefined` without asserting. Each output element's
// sum is written in left-to-right, k = 0..3 order.
function multiply(a: Mat4, b: Mat4): Mat4 {
  return [
    a[0] * b[0] + a[1] * b[4] + a[2] * b[8] + a[3] * b[12],
    a[0] * b[1] + a[1] * b[5] + a[2] * b[9] + a[3] * b[13],
    a[0] * b[2] + a[1] * b[6] + a[2] * b[10] + a[3] * b[14],
    a[0] * b[3] + a[1] * b[7] + a[2] * b[11] + a[3] * b[15],

    a[4] * b[0] + a[5] * b[4] + a[6] * b[8] + a[7] * b[12],
    a[4] * b[1] + a[5] * b[5] + a[6] * b[9] + a[7] * b[13],
    a[4] * b[2] + a[5] * b[6] + a[6] * b[10] + a[7] * b[14],
    a[4] * b[3] + a[5] * b[7] + a[6] * b[11] + a[7] * b[15],

    a[8] * b[0] + a[9] * b[4] + a[10] * b[8] + a[11] * b[12],
    a[8] * b[1] + a[9] * b[5] + a[10] * b[9] + a[11] * b[13],
    a[8] * b[2] + a[9] * b[6] + a[10] * b[10] + a[11] * b[14],
    a[8] * b[3] + a[9] * b[7] + a[10] * b[11] + a[11] * b[15],

    a[12] * b[0] + a[13] * b[4] + a[14] * b[8] + a[15] * b[12],
    a[12] * b[1] + a[13] * b[5] + a[14] * b[9] + a[15] * b[13],
    a[12] * b[2] + a[13] * b[6] + a[14] * b[10] + a[15] * b[14],
    a[12] * b[3] + a[13] * b[7] + a[14] * b[11] + a[15] * b[15],
  ];
}

// Convenience for the common "parent * child * local" chains. Requires at
// least one matrix; the type signature makes the empty-call state
// unreachable.
function chain(first: Mat4, ...rest: readonly Mat4[]): Mat4 {
  let out = first;
  for (const m of rest) {
    out = multiply(out, m);
  }
  return out;
}

function translation(x: number, y: number, z: number): Mat4 {
  return [1, 0, 0, x, 0, 1, 0, y, 0, 0, 1, z, 0, 0, 0, 1];
}

/**
 * @param s uniform scale only: keeps transforms rigid-ish so that mat3(model)
 *   stays a valid normal matrix in the shader.
 */
function uniformScale(s: number): Mat4 {
  return [s, 0, 0, 0, 0, s, 0, 0, 0, 0, s, 0, 0, 0, 0, 1];
}

/** @param a radians */
function rotationX(a: number): Mat4 {
  const c = Math.cos(a);
  const s = Math.sin(a);
  return [1, 0, 0, 0, 0, c, -s, 0, 0, s, c, 0, 0, 0, 0, 1];
}

/** @param a radians */
function rotationY(a: number): Mat4 {
  const c = Math.cos(a);
  const s = Math.sin(a);
  return [c, 0, s, 0, 0, 1, 0, 0, -s, 0, c, 0, 0, 0, 0, 1];
}

/** @param a radians */
function rotationZ(a: number): Mat4 {
  const c = Math.cos(a);
  const s = Math.sin(a);
  return [c, -s, 0, 0, s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}

// Euler rotation used by every bone and every baked part transform.
// Composition order is Y * X * Z, so a vector is rolled (Z), then pitched (X),
// then yawed (Y). In practice: Z splays a limb outward, X swings it
// forward/back, Y twists it.
/**
 * @param rx radians
 * @param ry radians
 * @param rz radians
 */
function euler(rx: number, ry: number, rz: number): Mat4 {
  return chain(rotationY(ry), rotationX(rx), rotationZ(rz));
}

// A bone/part local transform: move to `offset`, then rotate in place.
function transform(x: number, y: number, z: number, rx = 0, ry = 0, rz = 0): Mat4 {
  return multiply(translation(x, y, z), euler(rx, ry, rz));
}

function perspective(fovDeg: number, aspect: number, near: number, far: number): Mat4 {
  const f = 1 / Math.tan(((fovDeg * Math.PI) / 180 / 2));
  const nf = 1 / (near - far);
  // prettier-ignore
  return [
    f / aspect, 0, 0,                 0,
    0,          f, 0,                 0,
    0,          0, (far + near) * nf, (2 * far * near) * nf,
    0,          0, -1,                0,
  ];
}

function normalize(x: number, y: number, z: number): Vec3Tuple {
  const len = Math.sqrt(x * x + y * y + z * z);
  if (len < 1e-9) {
    return [0, 0, 0];
  }
  return [x / len, y / len, z / len];
}

function cross(
  ax: number,
  ay: number,
  az: number,
  bx: number,
  by: number,
  bz: number,
): Vec3Tuple {
  return [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx];
}

/**
 * @param eye {x, y, z}
 * @param target {x, y, z}
 */
function lookAt(eye: Vec3Tuple, target: Vec3Tuple): Mat4 {
  // z points *backwards* out of the screen, the usual right-handed view basis.
  const [zx, zy, zz] = normalize(eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]);
  const [xx, xy, xz] = normalize(...cross(0, 1, 0, zx, zy, zz));
  const [yx, yy, yz] = cross(zx, zy, zz, xx, xy, xz);
  const dot = (ax: number, ay: number, az: number): number =>
    ax * eye[0] + ay * eye[1] + az * eye[2];
  // prettier-ignore
  return [
    xx, xy, xz, -dot(xx, xy, xz),
    yx, yy, yz, -dot(yx, yy, yz),
    zx, zy, zz, -dot(zx, zy, zz),
    0,  0,  0,  1,
  ];
}

// Transforms a point (w = 1). Used to place shadow blobs and debug markers.
function transformPoint(m: Mat4, x: number, y: number, z: number): Vec3Tuple {
  return [
    m[0] * x + m[1] * y + m[2] * z + m[3],
    m[4] * x + m[5] * y + m[6] * z + m[7],
    m[8] * x + m[9] * y + m[10] * z + m[11],
  ];
}

// Transforms a direction (w = 0), ignoring translation.
function transformDirection(m: Mat4, x: number, y: number, z: number): Vec3Tuple {
  return [
    m[0] * x + m[1] * y + m[2] * z,
    m[4] * x + m[5] * y + m[6] * z,
    m[8] * x + m[9] * y + m[10] * z,
  ];
}

/** Minimal 4x4 matrix math for the slice's 3D pipeline. */
export const mat4 = {
  identity,
  multiply,
  chain,
  translation,
  uniformScale,
  rotationX,
  rotationY,
  rotationZ,
  euler,
  transform,
  perspective,
  lookAt,
  transformPoint,
  transformDirection,
};
