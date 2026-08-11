// Faces.
//
// The v1 head was a blank blob with two dark spheres pushed into it, which is
// why all three themes read as robots regardless of what the theme actually
// was. A face is cheap -- eyes with a real sclera, a brow, and a mouth are
// perhaps 60 triangles -- and it does more for "this is a character" than any
// amount of armour detail.
//
// Three treatments, matching the three figure styles:
//   expressive  sclera + pupil + angled brows + a mouth. Reads as a person.
//   minifig     flat dot eyes, straight brows, a printed smile. Reads as a toy
//               with a PRINTED face, which is the Lego language exactly.
//   vinyl       oversized solid eyes, no mouth, no brows. Reads as Funko: the
//               blankness is the whole point, so resist adding a mouth.
//
// Everything is authored in head-bone space and pushed onto the front of the
// skull by the caller's `front` function, so it works on any head shape.

import { mat4 } from "@gc/core";
import * as geometry from "./geometry.ts";
import type { PartBuilder } from "./geometry.ts";
import type { FaceStyle, SlotIndex } from "./themes.ts";

// A mouth: a row of small blocks following a parabola. Rotating each block to
// follow the tangent looked worse than leaving them axis-aligned at this
// scale, so they are not rotated -- the curve does the work.
function mouth(
  mb: PartBuilder,
  cy: number,
  front: number,
  width: number,
  curve: number,
  thickness: number,
  color: number,
): void {
  const steps = 9;
  for (let i = 0; i < steps; i++) {
    const t = (i + 0.5) / steps;
    const x = (t - 0.5) * width;
    // Zero at the corners, deepest in the middle: a smile.
    const dip = curve * (1 - (2 * t - 1) ** 2);
    geometry.box(
      mb,
      mat4.translation(x, cy - dip, front),
      (width / steps) * 1.2,
      thickness,
      thickness * 0.9,
      color,
    );
  }
}

// Draws a face onto `mb`.
//
// `hr` is the head half-width, `front` gives the front-surface Z at a given
// height, `eyeY` is the world-space Y of the eye line.
export function build(
  mb: PartBuilder,
  c: SlotIndex,
  hr: number,
  style: FaceStyle,
  front: (y: number) => number,
  eyeY: number,
): void {
  // `ink` and `sclera` are constant slots (see themes.ts CONSTANT_COLOR): no
  // theme has ever overridden them, so they are always just `c.ink` /
  // `c.sclera`.
  const ink = c.ink;
  const sclera = c.sclera;

  if (style === "vinyl") {
    // Two big solid eyes and nothing else. Funko's read is the blankness.
    const ew = hr * 0.3;
    const eh = hr * 0.4;
    for (const side of [-1, 1]) {
      const z = front(eyeY);
      geometry.extrude(
        mb,
        mat4.translation(side * hr * 0.4, eyeY, z),
        geometry.circleProfile(12),
        [
          { y: -eh * 0.5, w: ew * 0.62, d: ew * 0.62 },
          { y: -eh * 0.28, w: ew, d: ew },
          { y: eh * 0.28, w: ew, d: ew },
          { y: eh * 0.5, w: ew * 0.62, d: ew * 0.62 },
        ],
        ink,
      );
      // A single catchlight keeps it from going dead.
      geometry.sphere(
        mb,
        mat4.translation(side * hr * 0.4 - side * ew * 0.3, eyeY + eh * 0.22, z + ew * 0.34),
        ew * 0.24,
        4,
        8,
        sclera,
      );
    }
    return;
  }

  if (style === "minifig") {
    // Printed face: flat dots, straight brows, a wide simple smile.
    for (const side of [-1, 1]) {
      const z = front(eyeY);
      geometry.sphere(mb, mat4.translation(side * hr * 0.34, eyeY, z), hr * 0.115, 5, 10, ink);
      geometry.box(
        mb,
        mat4.translation(side * hr * 0.34, eyeY + hr * 0.26, z * 0.99),
        hr * 0.34,
        hr * 0.075,
        hr * 0.06,
        ink,
      );
    }
    mouth(mb, eyeY - hr * 0.44, front(eyeY - hr * 0.44), hr * 0.78, hr * 0.16, hr * 0.085, ink);
    return;
  }

  // Expressive: a real eye -- white sclera with a pupil sitting proud of it --
  // plus angled brows, which is where nearly all of the personality lives.
  for (const side of [-1, 1]) {
    const z = front(eyeY);
    geometry.sphere(mb, mat4.translation(side * hr * 0.36, eyeY, z), hr * 0.165, 5, 10, sclera);
    geometry.sphere(
      mb,
      mat4.translation(side * hr * 0.36 + side * hr * 0.02, eyeY, z + hr * 0.1),
      hr * 0.085,
      5,
      10,
      ink,
    );
    // Brow, tilted down toward the nose: determined rather than surprised.
    geometry.box(
      mb,
      mat4.multiply(
        mat4.translation(side * hr * 0.36, eyeY + hr * 0.3, z * 0.98),
        mat4.rotationZ((side * 11 * Math.PI) / 180),
      ),
      hr * 0.4,
      hr * 0.085,
      hr * 0.07,
      ink,
    );
  }
  // Nose bridge and a small mouth.
  geometry.box(
    mb,
    mat4.translation(0, eyeY - hr * 0.12, front(eyeY) * 1.02),
    hr * 0.14,
    hr * 0.3,
    hr * 0.12,
    c.skin,
  );
  mouth(mb, eyeY - hr * 0.48, front(eyeY - hr * 0.48), hr * 0.52, hr * 0.11, hr * 0.07, ink);
}
