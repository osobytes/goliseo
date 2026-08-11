// Shared, headless-safe geometry helpers for stadium.ts's helper modules.
//
// Every static structure in the coliseum -- the three seating tiers, the dark
// ambulatory wall behind the arcade, and the apron disk -- is the same shape
// viewed at different scales: a cross-section profile (radial offset, height
// offset) REVOLVED around an ellipse. A flat vertical wall is a degenerate
// one-step profile with zero radial width; a flat ground disk is a
// degenerate one-step profile with zero height. Rather than three near-
// duplicate mesh builders, `buildRevolvedRing` below is the one primitive,
// parameterised down to each of those shapes -- see stadium_bowl.ts and
// stadium_apron.ts for how each caller collapses the general case.
//
// Positions are built as a plain, non-indexed triangle soup and finished with
// `computeVertexNormals()`: with no shared indices, three.js assigns each
// triangle's own face normal to its three vertices (there is nothing to
// average against), which is exactly the faceted, "cut stone" look these
// surfaces want -- and it means this module never has to reason about
// winding-consistent index buffers by hand.

import * as THREE from "three";
import type { Prng } from "./stadium_prng.ts";

/** A point on an ellipse centred at `(cx, cz)` with radii `(rx, rz)`, in the XZ ground plane. */
export function ellipsePoint(
  cx: number,
  cz: number,
  rx: number,
  rz: number,
  angle: number,
): readonly [number, number] {
  return [cx + rx * Math.cos(angle), cz + rz * Math.sin(angle)];
}

/**
 * The yaw (radians, three.js convention: 0 faces +Z, increasing yaw rotates
 * toward +X) that turns an object sitting on the ellipse at `angle` to face
 * the ellipse's centre -- used to aim each arcade arch, brazier and banner
 * inward at the pitch instead of outward into space.
 */
export function ellipseYawFacingCenter(angle: number, rx: number, rz: number): number {
  const dirX = -rx * Math.cos(angle);
  const dirZ = -rz * Math.sin(angle);
  return Math.atan2(dirX, dirZ);
}

export interface RevolvedRingOptions {
  /** Ellipse centre, ground plane. */
  readonly cx: number;
  readonly cz: number;
  /** Radii of the profile's INNER edge (radial offset 0). */
  readonly rxInner: number;
  readonly rzInner: number;
  /** Total radial extent of the profile, added to `rxInner`/`rzInner` at its outer edge. */
  readonly width: number;
  /** Total height of the profile, added to `baseY` at its outer/top edge. */
  readonly height: number;
  readonly baseY: number;
  /** Stair-step count. 1 collapses the profile to a single flat/vertical span (a wall or a disk). */
  readonly steps: number;
  /** Angular subdivisions around the ellipse. */
  readonly segments: number;
  /** A flat walkway inserted at the profile's inner edge, before the steps begin. */
  readonly parapetDepth?: number;
  readonly parapetHeight?: number;
  readonly startAngle?: number;
  readonly endAngle?: number;
  readonly baseColor: readonly [number, number, number];
  /** Per-vertex colour jitter amplitude, applied via `rng`. 0 disables variation. */
  readonly tint: number;
  readonly rng: Prng;
}

function jitteredColor(
  base: readonly [number, number, number],
  tint: number,
  rng: Prng,
): readonly [number, number, number] {
  if (tint <= 0) {
    return base;
  }
  const d = (rng() - 0.5) * tint;
  return [clamp01(base[0] + d), clamp01(base[1] + d), clamp01(base[2] + d)];
}

function clamp01(v: number): number {
  return Math.min(1, Math.max(0, v));
}

function pushTri(
  positions: number[],
  colors: number[],
  a: readonly [number, number, number],
  b: readonly [number, number, number],
  c: readonly [number, number, number],
  baseColor: readonly [number, number, number],
  tint: number,
  rng: Prng,
): void {
  for (const p of [a, b, c]) {
    positions.push(p[0], p[1], p[2]);
    const col = jitteredColor(baseColor, tint, rng);
    colors.push(col[0], col[1], col[2]);
  }
}

/**
 * Revolves a stair-step cross-section (see file header) around an ellipse
 * into a non-indexed, faceted `BufferGeometry` with `position` and `color`
 * attributes. Degenerate uses:
 *   - `steps: 1, width: 0` -> a flat vertical wall ring (stadium_bowl.ts's ambulatory wall).
 *   - `steps: 1, height: 0` -> a flat ground disk/ring (stadium_apron.ts's apron).
 */
export function buildRevolvedRing(opts: RevolvedRingOptions): THREE.BufferGeometry {
  const {
    cx,
    cz,
    rxInner,
    rzInner,
    width,
    height,
    baseY,
    steps,
    segments,
    parapetDepth = 0,
    parapetHeight = 0,
    startAngle = 0,
    endAngle = Math.PI * 2,
    baseColor,
    tint,
    rng,
  } = opts;

  // Stair-step cross-section: a flat list of [radial offset, height offset].
  const profile: Array<readonly [number, number]> = [[0, 0]];
  let tCur = 0;
  let yCur = 0;
  if (parapetDepth > 0) {
    yCur = parapetHeight;
    profile.push([tCur, yCur]);
    tCur = parapetDepth;
    profile.push([tCur, yCur]);
  }
  const remainingWidth = Math.max(0, width - tCur);
  const remainingHeight = Math.max(0, height - yCur);
  const stepCount = Math.max(1, steps);
  const stepW = remainingWidth / stepCount;
  const stepH = remainingHeight / stepCount;
  for (let i = 0; i < stepCount; i += 1) {
    yCur += stepH;
    profile.push([tCur, yCur]);
    tCur += stepW;
    profile.push([tCur, yCur]);
  }

  // Revolve: one slice of the profile per angular subdivision.
  const angleSpan = endAngle - startAngle;
  const segCount = Math.max(1, segments);
  const slices: Array<ReadonlyArray<readonly [number, number, number]>> = [];
  for (let i = 0; i <= segCount; i += 1) {
    const angle = startAngle + (angleSpan * i) / segCount;
    const cosA = Math.cos(angle);
    const sinA = Math.sin(angle);
    const slice: Array<readonly [number, number, number]> = [];
    for (const point of profile) {
      const t = point[0];
      const y = point[1];
      const px = cx + (rxInner + t) * cosA;
      const pz = cz + (rzInner + t) * sinA;
      slice.push([px, baseY + y, pz]);
    }
    slices.push(slice);
  }

  const positions: number[] = [];
  const colors: number[] = [];
  for (let i = 0; i < segCount; i += 1) {
    const sliceA = slices[i];
    const sliceB = slices[i + 1];
    if (sliceA === undefined || sliceB === undefined) {
      continue;
    }
    for (let j = 0; j + 1 < profile.length; j += 1) {
      const a = sliceA[j];
      const b = sliceA[j + 1];
      const c = sliceB[j];
      const d = sliceB[j + 1];
      if (a === undefined || b === undefined || c === undefined || d === undefined) {
        continue;
      }
      pushTri(positions, colors, a, c, b, baseColor, tint, rng);
      pushTri(positions, colors, b, c, d, baseColor, tint, rng);
    }
  }

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  geometry.computeVertexNormals();
  return geometry;
}
