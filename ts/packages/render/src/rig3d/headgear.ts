// One head presentation per theme.
//
// Every helm is positioned from the ACTUAL head geometry it is going onto --
// half-width, base, height and eye line are all passed in -- so the same helm
// fits a natural head, a minifig cylinder and an oversized vinyl skull without
// a per-figure special case.
//
// All three are deliberately OPEN-FACED. The v1 great helm was a closed barrel
// that hid the entire face, which is a large part of why the characters read
// as robots. A helmet that frames a face beats a helmet that replaces one.
//
// Each returns [solid, emissive]. The crest / band / key is wired to the TEAM
// trim colour, per the roster's team-ownership rule.

import { mat4 } from "@gc/core";
import * as geometry from "./geometry.ts";
import { PartBuilder } from "./geometry.ts";
import type { SlotIndex } from "./themes.ts";

/** The skull measurements a helm is fitted to. */
export interface HeadGeometry {
  readonly hr: number; // half-width
  readonly base: number; // base Y
  readonly hh: number; // height
  readonly eye: number; // eye-line Y
}

// A fin of quads following a half-circle silhouette: plumes and crests.
function crest(
  mb: PartBuilder,
  span: number,
  height: number,
  baseY: number,
  thickness: number,
  color: number,
): void {
  const segments = 14;
  for (let i = 0; i < segments; i++) {
    const t0 = i / segments;
    const t1 = (i + 1) / segments;
    const x0 = (t0 - 0.5) * 2 * span;
    const x1 = (t1 - 0.5) * 2 * span;
    const curve = 0.3 / Math.max(span, 1e-6);
    const y0 = baseY - curve * x0 * x0;
    const y1 = baseY - curve * x1 * x1;
    const h0 = height * Math.sin(t0 * Math.PI) ** 0.7;
    const h1 = height * Math.sin(t1 * Math.PI) ** 0.7;
    const d = thickness;
    mb.quad(null, [x0, y0, -d], [x1, y1, -d], [x1, y1 + h1, -d], [x0, y0 + h0, -d], color);
    mb.quad(null, [x1, y1, d], [x0, y0, d], [x0, y0 + h0, d], [x1, y1 + h1, d], color);
    mb.quad(null, [x0, y0 + h0, -d], [x1, y1 + h1, -d], [x1, y1 + h1, d], [x0, y0 + h0, d], color);
  }
}

// Medieval Fantasy: an open barbute. Bowl, brow band, nasal bar, cheek
// plates, transverse plume -- and a clear opening for the eyes and mouth.
function greatHelm(c: SlotIndex, h: HeadGeometry): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  const r = h.hr;

  geometry.sphere(mb, mat4.translation(0, h.eye + r * 0.36, 0), r * 1.14, 6, 14, c.plate, 0.62);
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(14),
    [
      { y: h.eye + r * 0.26, w: r * 1.1, d: r * 1.06 },
      { y: h.eye + r * 0.44, w: r * 1.18, d: r * 1.14 },
      { y: h.eye + r * 0.6, w: r * 1.12, d: r * 1.08 },
    ],
    c.accent,
  );
  // Nasal bar down the centre of the opening.
  geometry.box(
    mb,
    mat4.translation(0, h.eye - r * 0.1, r * 0.92),
    r * 0.15,
    r * 0.8,
    r * 0.16,
    c.plate,
  );
  // Cheek plates, leaving the middle of the face clear.
  for (const side of [-1, 1]) {
    geometry.extrude(
      mb,
      mat4.multiply(
        mat4.translation(side * r * 0.86, 0, r * 0.1),
        mat4.rotationZ((-side * 6 * Math.PI) / 180),
      ),
      geometry.boxProfile(0.8, 0.45),
      [
        { y: h.eye - r * 0.72, w: r * 0.26 },
        { y: h.eye - r * 0.1, w: r * 0.34 },
        { y: h.eye + r * 0.34, w: r * 0.32 },
      ],
      c.plate,
    );
  }
  const top = h.base + h.hh;
  geometry.box(mb, mat4.translation(0, top * 0.99, 0), r * 2.0, r * 0.14, r * 0.34, c.accent);
  crest(mb, r * 0.94, r * 0.8, top * 1.02, r * 0.12, c.crest);
  return [mb, null];
}

// Galactic Sci-Fi: a swept helm whose emissive band sits ABOVE the eyes, so
// the visor reads as tech without blanking the face.
function visorHelm(c: SlotIndex, h: HeadGeometry): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  const r = h.hr;

  geometry.sphere(
    mb,
    mat4.translation(0, h.eye + r * 0.42, -r * 0.04),
    r * 1.12,
    7,
    14,
    c.plate,
    0.6,
  );
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(14),
    [
      { y: h.eye + r * 0.3, w: r * 1.06, d: r * 1.02 },
      { y: h.eye + r * 0.46, w: r * 1.16, d: r * 1.12 },
    ],
    c.plate_dark,
  );
  for (const side of [-1, 1]) {
    geometry.box(
      mb,
      mat4.multiply(
        mat4.translation(side * r * 1.02, h.eye + r * 0.3, -r * 0.22),
        mat4.rotationZ((-side * 14 * Math.PI) / 180),
      ),
      r * 0.18,
      r * 0.66,
      r * 0.92,
      c.accent,
    );
    // Jaw guard under the cheeks, still clear of the mouth.
    geometry.box(
      mb,
      mat4.translation(side * r * 0.84, h.eye - r * 0.42, r * 0.26),
      r * 0.24,
      r * 0.7,
      r * 0.62,
      c.accent,
    );
  }

  const glow = new PartBuilder();
  const segments = 12;
  const ring = (t: number, y: number, rad: number): geometry.Point3 => {
    const a = (t - 0.5) * 2.3;
    return [rad * Math.sin(a), y, rad * 0.98 * Math.cos(a)];
  };
  for (let i = 0; i < segments; i++) {
    const t0 = i / segments;
    const t1 = (i + 1) / segments;
    const lo = h.eye + r * 0.5;
    const hi = h.eye + r * 0.7;
    glow.quad(
      null,
      ring(t0, lo, r * 1.14),
      ring(t1, lo, r * 1.14),
      ring(t1, hi, r * 1.08),
      ring(t0, hi, r * 1.08),
      c.seam,
    );
  }
  return [mb, glow];
}

// Toybox: a moulded action-figure cap plus the wind-up key. Sits high, so
// the whole face shows -- which is the point of the theme.
function figureHelm(c: SlotIndex, h: HeadGeometry): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  const r = h.hr;

  geometry.sphere(
    mb,
    mat4.translation(0, h.eye + r * 0.5, -r * 0.05),
    r * 1.1,
    7,
    14,
    c.plate,
    0.56,
  );
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(14),
    [
      { y: h.eye + r * 0.34, w: r * 1.04, d: r * 1.02 },
      { y: h.eye + r * 0.52, w: r * 1.16, d: r * 1.12 },
      { y: h.eye + r * 0.68, w: r * 1.1, d: r * 1.06 },
    ],
    c.accent,
  );
  for (const side of [-1, 1]) {
    geometry.sphere(
      mb,
      mat4.translation(side * r * 1.02, h.eye + r * 0.36, 0),
      r * 0.3,
      5,
      10,
      c.accent,
    );
  }

  // Wind-up key: the Toybox joke in one shape.
  const keyTf = mat4.multiply(
    mat4.translation(0, h.eye + r * 0.9, -r * 1.0),
    mat4.rotationX((-18 * Math.PI) / 180),
  );
  geometry.extrude(
    mb,
    keyTf,
    geometry.circleProfile(8),
    [
      { y: 0, w: r * 0.12 },
      { y: r * 0.5, w: r * 0.1 },
    ],
    c.joint,
  );
  for (const side of [-1, 1]) {
    geometry.extrude(
      mb,
      mat4.multiply(keyTf, mat4.translation(side * r * 0.26, r * 0.52, 0)),
      geometry.boxProfile(0.3, 0.9),
      [
        { y: -r * 0.19, w: r * 0.27 },
        { y: r * 0.19, w: r * 0.27 },
      ],
      c.joint,
    );
  }
  crest(mb, r * 0.72, r * 0.42, h.base + h.hh * 1.02, r * 0.13, c.crest);
  return [mb, null];
}

const BUILDERS: Readonly<
  Record<string, (c: SlotIndex, h: HeadGeometry) => readonly [PartBuilder, PartBuilder | null]>
> = {
  great_helm: greatHelm,
  visor_helm: visorHelm,
  figure_helm: figureHelm,
};

/**
 * `head` carries the geometry of the skull this helm has to fit.
 * Returns [solid, emissive|null] part builders, not merged parts (#337
 * slice 2's split survives the port: only `body.accumulate` folds these in).
 */
export function build(
  id: string,
  c: SlotIndex,
  head: HeadGeometry,
): readonly [PartBuilder, PartBuilder | null] {
  const fn = BUILDERS[id];
  if (!fn) {
    throw new Error(`unknown headgear id: ${id}`);
  }
  return fn(c, head);
}
