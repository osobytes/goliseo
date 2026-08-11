// Accumulates flat-shaded triangles and bakes them into a GPU mesh, which is
// exactly what three.js's `BufferGeometry` already does. Vertices are plain
// typed fields (position, normal, palette slot, bone index, material id) --
// there is no hand-packed vertex format to declare, because three.js skins
// with `skinIndex`/`skinWeight` attributes and a bone matrix texture, not a
// hand-packed uniform array, and takes multiple materials per mesh via
// geometry groups rather than a baked per-vertex material id.
//
// This module keeps two things:
//
//   * the flat-shaded triangle/quad accumulation with a computed face normal
//     (`Builder`), and
//   * the procedural shape generators built on top of it: boxes,
//     spheres-as-lathes, a general profile extrusion/loft, a curved panel and
//     a disc.
//
// The merged output of `merge()` below is plain typed vertex data (position,
// normal, a palette slot index, a bone index, a material id) -- everything
// body.ts/equipment.ts/headgear.ts/face.ts need to describe a character.
//
// `build()` (below) takes that data the one more mechanical step into a
// `THREE.BufferGeometry`. `@types/three` is installed in this workspace, and
// constructing a `BufferGeometry`/`BufferAttribute` needs no live WebGL
// context: it is a plain typed-array data container, same as any other
// object three.js exposes, so it stays testable headless. What stays
// genuinely out of scope for this milestone (no live GL context) is a
// `THREE.SkinnedMesh` bound to a posed `THREE.Skeleton` and
// actually rendered -- that is `player_renderer_3d.ts`'s job, not this
// module's, and it builds its own attributes directly from `PartBuilder.verts`
// today rather than calling `build()` (both are valid consumers of the same
// vertex data; nothing here requires unifying them in this milestone).

import * as THREE from "three";
import { mat4, type Mat4 } from "@gc/core";

/** A point or direction in the part's local space. */
export type Point3 = readonly [number, number, number];

/** Shading family, resolved onto a vertex at merge time instead of per draw. */
export const MATERIAL = {
  plain: 0,
  metal: 1,
  emissive: 2,
} as const;

export type MaterialId = (typeof MATERIAL)[keyof typeof MATERIAL];

// `undefined` is the common case (skin, cloth, straps), so it maps to `plain`
// rather than being an error. Takes a bare `string` rather than `keyof typeof
// MATERIAL`: material names arrive as untyped content-authored strings (see
// `Theme`'s loadout-driven callers), so an unknown name has to fail at
// runtime, not just be unrepresentable at the type level.
export function materialIndex(name?: string): MaterialId {
  if (name === undefined) {
    return MATERIAL.plain;
  }
  const index = (MATERIAL as Readonly<Record<string, MaterialId>>)[name];
  if (index === undefined) {
    throw new Error(`unknown rig3d material: ${name}`);
  }
  return index;
}

/** One accumulated vertex: a position, a face normal, and its metadata. */
export interface Vertex {
  readonly position: Point3;
  readonly normal: Point3;
  readonly paletteSlot: number;
  readonly bone: number;
  readonly material: MaterialId;
}

function normalizeVec(x: number, y: number, z: number): Point3 {
  const len = Math.sqrt(x * x + y * y + z * z);
  if (len < 1e-12) {
    return [0, 1, 0];
  }
  return [x / len, y / len, z / len];
}

// Accumulates flat-shaded triangles in a part's own local space.
//
// Every triangle gets a single face normal computed from its own winding, so
// the whole slice is flat-shaded. That is deliberate: it is one line of code
// instead of a smoothing-group pass, and it gives the faceted low-poly arcade
// look the brief asks for.
//
// Points are transformed by a `Mat4` as they are added ("baking"), so a part
// generator can be written in convenient local coordinates and then placed
// wherever it belongs on the rig.
//
// A PART builder holds one part's triangles, `bone`/`material` still at their
// defaults: a part does not know where it rides or how it shades until
// `merge` folds it into a character.
export class PartBuilder {
  readonly verts: Vertex[] = [];

  // True once this builder is `merge()`'s output. A part builder starts
  // false: it is not a draw call on its own (see `merge`'s doc), and
  // `build()` below refuses to run against one.
  merged = false;

  // Adds one triangle. `tf` may be null (identity). Vertices are expected
  // counter-clockwise as seen from outside the solid.
  //
  // `slot` is typed `number`, so a caller passing a colour table or nothing
  // is a compile error in content code -- but content still flows through
  // `unknown` boundaries (test fixtures, future data-driven authoring), so
  // this keeps the runtime guard too rather than relying on the type system
  // alone.
  triangle(tf: Mat4 | null, a: Point3, b: Point3, c: Point3, slot: number): void {
    if (!Number.isFinite(slot)) {
      throw new Error(`triangle slot must be a palette slot index (a finite number), got ${String(slot)}`);
    }
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    let [cx, cy, cz] = c;
    if (tf) {
      [ax, ay, az] = mat4.transformPoint(tf, ax, ay, az);
      [bx, by, bz] = mat4.transformPoint(tf, bx, by, bz);
      [cx, cy, cz] = mat4.transformPoint(tf, cx, cy, cz);
    }

    const ux = bx - ax;
    const uy = by - ay;
    const uz = bz - az;
    const vx = cx - ax;
    const vy = cy - ay;
    const vz = cz - az;
    const nx = uy * vz - uz * vy;
    const ny = uz * vx - ux * vz;
    const nz = ux * vy - uy * vx;
    // Degenerate triangles (sword tip, sphere poles, collapsed extrusion
    // rings) would produce a NaN normal, so drop them instead.
    if (nx * nx + ny * ny + nz * nz < 1e-18) {
      return;
    }
    const normal = normalizeVec(nx, ny, nz);

    this.verts.push({ position: [ax, ay, az], normal, paletteSlot: slot, bone: 0, material: MATERIAL.plain });
    this.verts.push({ position: [bx, by, bz], normal, paletteSlot: slot, bone: 0, material: MATERIAL.plain });
    this.verts.push({ position: [cx, cy, cz], normal, paletteSlot: slot, bone: 0, material: MATERIAL.plain });
  }

  // Adds a planar quad as two triangles, wound a -> b -> c -> d.
  quad(tf: Mat4 | null, a: Point3, b: Point3, c: Point3, d: Point3, slot: number): void {
    this.triangle(tf, a, b, c, slot);
    this.triangle(tf, a, c, d, slot);
  }

  triangleCount(): number {
    return this.verts.length / 3;
  }

  // Bakes the accumulated triangles into a `THREE.BufferGeometry`. Only a
  // merged builder is a draw call, so building a bare part directly would
  // render every part on the root bone (#337 slice 2).
  //
  // Attribute names are this module's own (`paletteSlot`/`boneIndex`/
  // `material`), not a three.js-reserved name like `skinIndex`/`skinWeight`
  // -- resolving those, and binding a `Skeleton`, is the rendering
  // integration's job (`player_renderer_3d.ts`), not this content-only
  // module's. See this file's header for why that split is fine.
  build(): THREE.BufferGeometry {
    if (!this.merged) {
      throw new Error(
        "only a merged builder is a mesh (#337 slice 2): a part is not a draw call, pass it through geometry.merge",
      );
    }
    if (this.verts.length === 0) {
      throw new Error("cannot build an empty mesh");
    }
    const count = this.verts.length;
    const positions = new Float32Array(count * 3);
    const normals = new Float32Array(count * 3);
    const paletteSlots = new Float32Array(count);
    const bones = new Float32Array(count);
    const materials = new Float32Array(count);
    this.verts.forEach((v, i) => {
      positions[i * 3] = v.position[0];
      positions[i * 3 + 1] = v.position[1];
      positions[i * 3 + 2] = v.position[2];
      normals[i * 3] = v.normal[0];
      normals[i * 3 + 1] = v.normal[1];
      normals[i * 3 + 2] = v.normal[2];
      paletteSlots[i] = v.paletteSlot;
      bones[i] = v.bone;
      materials[i] = v.material;
    });
    const geom = new THREE.BufferGeometry();
    geom.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geom.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    geom.setAttribute("paletteSlot", new THREE.BufferAttribute(paletteSlots, 1));
    geom.setAttribute("boneIndex", new THREE.BufferAttribute(bones, 1));
    geom.setAttribute("material", new THREE.BufferAttribute(materials, 1));
    return geom;
  }
}

/** One part waiting to be folded into a character by `merge`. */
export interface Part {
  readonly builder: PartBuilder;
  /** 0-based index into the skeleton's bone order (`skeleton.boneIndex`). */
  readonly bone: number;
  /** A static per-part offset from its bone, baked into the vertex. */
  readonly attach?: Mat4 | undefined;
  readonly material?: string | undefined;
}

// Folds many part builders into the single builder that becomes one
// character's geometry.
//
// Each part contributes its vertices with:
//   * `attach` baked in, so the bone transform alone drives the vertex from
//     here on;
//   * `bone` written to every vertex, the 0-based index into the skeleton's
//     bone order; and
//   * `material` resolved and written to every vertex, so armour and an
//     energy blade can shade differently without splitting the character
//     into multiple draws.
//
// Part order is preserved verbatim, so triangles rasterise in the order the
// separate parts were authored.
export function merge(parts: readonly Part[]): PartBuilder {
  const out = new PartBuilder();
  out.merged = true;
  for (const part of parts) {
    if (!Number.isInteger(part.bone) || part.bone < 0) {
      throw new Error(`merge needs a 0-based bone index per part, got ${String(part.bone)}`);
    }
    const bone = part.bone;
    const material = materialIndex(part.material);
    const tf = part.attach;
    for (const v of part.builder.verts) {
      let [x, y, z] = v.position;
      let [nx, ny, nz] = v.normal;
      if (tf) {
        [x, y, z] = mat4.transformPoint(tf, x, y, z);
        // Every attach is rotation + translation today, so this keeps unit
        // length already; renormalising costs one build-time square root per
        // vertex and keeps downstream shading's normalize-free assumptions
        // true if an attach ever gains a scale.
        [nx, ny, nz] = normalizeVec(...mat4.transformDirection(tf, nx, ny, nz));
      }
      out.verts.push({ position: [x, y, z], normal: [nx, ny, nz], paletteSlot: v.paletteSlot, bone, material });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Procedural primitive generators.
//
// Everything visible in the slice -- sword, shield, helmet, every limb -- is
// built from these functions. The workhorse is `extrude`: a closed 2D
// cross-section swept through a stack of rings, each ring free to move and
// rescale. A tapered blade, a grip cylinder, a pommel sphere and a tapering
// limb are all the same operation with different inputs.
// ---------------------------------------------------------------------------

/** A closed 2D polygon in the XZ plane, CCW from +Y, as (x, z) pairs. */
export type Profile = readonly (readonly [number, number])[];

// A circle, for grips, limbs and spheres.
export function circleProfile(segments: number): Profile {
  const profile: Array<readonly [number, number]> = [];
  for (let i = 0; i < segments; i++) {
    const a = (i / segments) * Math.PI * 2;
    profile.push([Math.cos(a), Math.sin(a)]);
  }
  return profile;
}

// A flattened diamond: the classic edged-blade cross-section, with a central
// ridge running down each face. `thickness` is the half-depth relative to
// the half-width, so 0.2 is a broad, thin blade.
export function diamondProfile(thickness: number): Profile {
  return [
    [1, 0],
    [0, thickness],
    [-1, 0],
    [0, -thickness],
  ];
}

// A rectangle, optionally with the corners cut off, for torsos and limbs
// that should read as blocky rather than round.
export function boxProfile(depth: number, bevel = 0): Profile {
  if (bevel <= 0) {
    return [
      [1, -depth],
      [1, depth],
      [-1, depth],
      [-1, -depth],
    ];
  }
  const b = bevel;
  const d = depth;
  return [
    [1, -d + b * d],
    [1, d - b * d],
    [1 - b, d],
    [-1 + b, d],
    [-1, d - b * d],
    [-1, -d + b * d],
    [-1 + b, -d],
    [1 - b, -d],
  ];
}

/** One ring of an extrusion: height, half-width/depth scale, lateral offset. */
export interface Section {
  readonly y: number;
  readonly w: number;
  readonly d?: number;
  readonly x?: number;
  readonly z?: number;
}

/** A palette slot, or a function of (section index, profile point index). */
export type SlotOrFn = number | ((sectionIndex: number, pointIndex: number) => number);

function at<T>(arr: readonly T[], i: number): T {
  const v = arr[i];
  if (v === undefined) {
    throw new Error(`index ${i} out of range (length ${arr.length})`);
  }
  return v;
}

// Sweeps `profile` through `sections`, bottom to top, and caps both ends.
//
// A section with `w = 0` collapses to a point, which is how a sword tip and
// a sphere pole are produced -- the resulting zero-area triangles are
// dropped by `PartBuilder.triangle`.
export function extrude(mb: PartBuilder, tf: Mat4 | null, profile: Profile, sections: readonly Section[], slot: SlotOrFn): void {
  const n = profile.length;
  const slotFor = (i: number, j: number): number => (typeof slot === "function" ? slot(i, j) : slot);

  const point = (section: Section, j: number): Point3 => {
    const p = at(profile, j);
    const w = section.w;
    const d = section.d ?? w;
    return [(section.x ?? 0) + p[0] * w, section.y, (section.z ?? 0) + p[1] * d];
  };

  for (let i = 0; i < sections.length - 1; i++) {
    const lower = at(sections, i);
    const upper = at(sections, i + 1);
    for (let j = 0; j < n; j++) {
      const k = (j + 1) % n;
      mb.quad(tf, point(lower, j), point(lower, k), point(upper, k), point(upper, j), slotFor(i + 1, j + 1));
    }
  }

  // Triangle-fan caps. Skipped automatically when the end ring is collapsed
  // (n < 3, or every triangle degenerate).
  if (sections.length > 0) {
    const first = at(sections, 0);
    const last = at(sections, sections.length - 1);
    for (let j = 1; j <= n - 2; j++) {
      mb.triangle(tf, point(first, 0), point(first, j + 1), point(first, j), slotFor(0, j));
      mb.triangle(tf, point(last, 0), point(last, j), point(last, j + 1), slotFor(sections.length, j));
    }
  }
}

// An axis-aligned box centred on the local origin.
export function box(mb: PartBuilder, tf: Mat4 | null, w: number, h: number, d: number, slot: number): void {
  const x = w / 2;
  const y = h / 2;
  const z = d / 2;
  const p: readonly Point3[] = [
    [-x, -y, z],
    [x, -y, z],
    [x, y, z],
    [-x, y, z], // front (+Z)
    [-x, -y, -z],
    [x, -y, -z],
    [x, y, -z],
    [-x, y, -z], // back (-Z)
  ];
  mb.quad(tf, at(p, 0), at(p, 1), at(p, 2), at(p, 3), slot); // +Z
  mb.quad(tf, at(p, 5), at(p, 4), at(p, 7), at(p, 6), slot); // -Z
  mb.quad(tf, at(p, 1), at(p, 5), at(p, 6), at(p, 2), slot); // +X
  mb.quad(tf, at(p, 4), at(p, 0), at(p, 3), at(p, 7), slot); // -X
  mb.quad(tf, at(p, 3), at(p, 2), at(p, 6), at(p, 7), slot); // +Y
  mb.quad(tf, at(p, 4), at(p, 5), at(p, 1), at(p, 0), slot); // -Y
}

// A sphere, expressed as a circle profile swept along a sine silhouette. Used
// for sword pommels, shoulder pauldrons and shield bosses.
// `arc`: 1 (default) = full sphere, 0.5 = hemisphere.
export function sphere(
  mb: PartBuilder,
  tf: Mat4 | null,
  radius: number,
  rings: number,
  segments: number,
  slot: number,
  arc = 1,
): void {
  const span = arc;
  const sections: Section[] = [];
  for (let i = 0; i <= rings; i++) {
    // phi runs from the south pole (or the equator, for a dome) to the north.
    const phi = Math.PI * (1 - span) + (i / rings) * Math.PI * span;
    sections.push({ y: -radius * Math.cos(phi), w: radius * Math.sin(phi) });
  }
  extrude(mb, tf, circleProfile(segments), sections, slot);
}

/** Returns a palette slot for a point on a curved panel's face, `u, v` in [0, 1]. */
export type ColorAtFn = (u: number, v: number) => number;

// A rectangle bent around the vertical axis, for curved shield shells. Local
// space: the panel is centred on the origin, curving away from +Z, so the
// painted face looks toward +Z.
export function curvedPanel(
  mb: PartBuilder,
  tf: Mat4 | null,
  w: number,
  h: number,
  thickness: number,
  curve: number,
  segments: number,
  colorAt: ColorAtFn,
  edge: number,
): void {
  const radius = w / curve;
  const halfH = h / 2;

  // front[i] / back[i] are the two surfaces at column i.
  const front: Array<readonly [number, number]> = [];
  const back: Array<readonly [number, number]> = [];
  for (let i = 0; i <= segments; i++) {
    const u = i / segments;
    const theta = (u - 0.5) * curve;
    const s = Math.sin(theta);
    const c = Math.cos(theta);
    front.push([radius * s, radius * c - radius + thickness]);
    back.push([(radius - thickness) * s, (radius - thickness) * c - radius + thickness]);
  }

  for (let i = 0; i < segments; i++) {
    const u0 = i / segments;
    const u1 = (i + 1) / segments;
    const f0 = at(front, i);
    const f1 = at(front, i + 1);
    const b0 = at(back, i);
    const b1 = at(back, i + 1);

    // Painted face, split vertically so the device can vary along v too.
    const rows = 10;
    for (let r = 0; r < rows; r++) {
      const y0 = -halfH + h * (r / rows);
      const y1 = -halfH + h * ((r + 1) / rows);
      const v = (r + 0.5) / rows;
      mb.quad(
        tf,
        [f0[0], y0, f0[1]],
        [f1[0], y0, f1[1]],
        [f1[0], y1, f1[1]],
        [f0[0], y1, f0[1]],
        colorAt((u0 + u1) / 2, v),
      );
    }

    // Inside face and the top/bottom rims.
    mb.quad(tf, [b1[0], -halfH, b1[1]], [b0[0], -halfH, b0[1]], [b0[0], halfH, b0[1]], [b1[0], halfH, b1[1]], edge);
    mb.quad(tf, [f0[0], halfH, f0[1]], [f1[0], halfH, f1[1]], [b1[0], halfH, b1[1]], [b0[0], halfH, b0[1]], edge);
    mb.quad(tf, [b0[0], -halfH, b0[1]], [b1[0], -halfH, b1[1]], [f1[0], -halfH, f1[1]], [f0[0], -halfH, f0[1]], edge);
  }

  // Left and right vertical rims.
  for (const side of [0, segments]) {
    const f = at(front, side);
    const b = at(back, side);
    mb.quad(tf, [f[0], -halfH, f[1]], [b[0], -halfH, b[1]], [b[0], halfH, b[1]], [f[0], halfH, f[1]], edge);
  }
}

// A flat horizontal disc on the XZ plane. The arena floor and the drop shadow.
export function disc(mb: PartBuilder, tf: Mat4 | null, radius: number, rings: number, segments: number, colorAt: ColorAtFn): void {
  for (let r = 0; r < rings; r++) {
    const r0 = radius * (r / rings);
    const r1 = radius * ((r + 1) / rings);
    for (let s = 0; s < segments; s++) {
      const a0 = (s / segments) * Math.PI * 2;
      const a1 = ((s + 1) / segments) * Math.PI * 2;
      const slot = colorAt((r + 0.5) / rings, (s + 0.5) / segments);
      mb.quad(
        tf,
        [r0 * Math.cos(a0), 0, r0 * Math.sin(a0)],
        [r1 * Math.cos(a0), 0, r1 * Math.sin(a0)],
        [r1 * Math.cos(a1), 0, r1 * Math.sin(a1)],
        [r0 * Math.cos(a1), 0, r0 * Math.sin(a1)],
        slot,
      );
    }
  }
}
