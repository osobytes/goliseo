// The six prototype equipment presentations, one loadout per theme.
//
// The interesting one is `light_melee`: Tournament Sword, Vector Blade and
// Foam Champion are three completely different objects that share one
// simulation family and one semantic animation verb. The roster calls this
// "the deliberate cross-theme reskin test", so all three are built to hang
// off the same right hand socket with the same attachment transform --
// swapping them changes only what you see.
//
// Local space for every hand item: grip centred on the origin, business end
// running up +Y. The hand attachment in body.ts assumes exactly that.

import { mat4 } from "@gc/core";
import * as geometry from "./geometry.ts";
import { PartBuilder } from "./geometry.ts";
import type { SlotIndex } from "./themes.ts";

// A hexagonal blade cross-section: two cutting edges at x = +/-1 with flat
// faces meeting at a central ridge.
const BLADE_PROFILE: geometry.Profile = [
  [1, 0],
  [0.45, 1],
  [-0.45, 1],
  [-1, 0],
  [-0.45, -1],
  [0.45, -1],
];

// Grip + pommel shared by every melee item, so they all sit in the fist the
// same way regardless of theme.
function hilt(mb: PartBuilder, gripR: number, gripTop: number, pommelR: number, gripColor: number, metal: number): void {
  geometry.sphere(mb, mat4.translation(0, -gripTop - pommelR * 0.6, 0), pommelR, 6, 10, metal);
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(10),
    [
      { y: -gripTop, w: gripR * 1.06 },
      { y: -gripTop * 0.4, w: gripR },
      { y: gripTop * 0.5, w: gripR },
      { y: gripTop, w: gripR * 1.08 },
    ],
    gripColor,
  );
}

// ---------------------------------------------------------------------------
// Medieval Fantasy
// ---------------------------------------------------------------------------

// Tournament Sword: straight arming sword, cruciform guard, wide fuller.
function tournamentSword(c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  hilt(mb, 0.021, 0.085, 0.034, c.strap, c.accent);

  // Cruciform quillons: one long bar across, plus a central block.
  geometry.extrude(
    mb,
    mat4.rotationZ(Math.PI / 2),
    geometry.circleProfile(8),
    [
      { y: -0.115, w: 0.016 },
      { y: -0.085, w: 0.021 },
      { y: 0.085, w: 0.021 },
      { y: 0.115, w: 0.016 },
    ],
    c.accent,
  );
  geometry.box(mb, mat4.translation(0, 0.095, 0), 0.055, 0.045, 0.04, c.accent);

  geometry.extrude(
    mb,
    null,
    BLADE_PROFILE,
    [
      { y: 0.115, w: 0.04, d: 0.0095 },
      { y: 0.2, w: 0.038, d: 0.009 },
      { y: 0.56, w: 0.031, d: 0.007 },
      { y: 0.68, w: 0.024, d: 0.0055 },
      { y: 0.84, w: 0.002, d: 0.0006 },
    ],
    c.plate,
  );
  return [mb, null];
}

// Emberguard Shield: a heater shield -- wide flat top, tapering to a point.
// Built as a stack of curved rings whose width shrinks, so it curves in both
// axes rather than being a flat plate.
function heaterShield(c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  const rows = 12;
  const height = 0.86;
  const topW = 0.62;
  const thickness = 0.03;

  const widthAt = (t: number): number => topW * (0.16 + 0.84 * Math.sin(t * Math.PI * 0.62) ** 0.55);

  let prev: geometry.Point3[] | null = null;
  for (let i = 0; i <= rows; i++) {
    const t = i / rows;
    const w = widthAt(t);
    const y = -height * 0.5 + height * t;
    const radius = w / 1.3;
    const ring: geometry.Point3[] = [];
    for (let j = 0; j <= 6; j++) {
      const a = (j / 6 - 0.5) * 1.3;
      ring.push([radius * Math.sin(a), y, radius * Math.cos(a) - radius + thickness]);
    }
    if (prev) {
      const prevRing = prev;
      for (let j = 0; j < 6; j++) {
        // Face: team-coloured field with a bronze border band.
        const edge = i <= 1 || i >= rows - 1 || j === 0 || j === 5;
        const prevJ = prevRing[j];
        const prevJ1 = prevRing[j + 1];
        const ringJ = ring[j];
        const ringJ1 = ring[j + 1];
        if (!prevJ || !prevJ1 || !ringJ || !ringJ1) {
          continue;
        }
        mb.quad(null, prevJ, prevJ1, ringJ1, ringJ, edge ? c.accent : c.cloth);
        // Back, pushed in by the thickness.
        const back = (p: geometry.Point3): geometry.Point3 => [p[0] * 0.94, p[1], p[2] - thickness];
        mb.quad(null, back(ringJ), back(ringJ1), back(prevJ1), back(prevJ), c.strap);
      }
    }
    prev = ring;
  }

  // Boss and the enarmes on the back.
  geometry.sphere(mb, mat4.multiply(mat4.translation(0, 0.02, thickness), mat4.rotationX(Math.PI / 2)), 0.07, 5, 12, c.accent, 0.55);
  for (const y of [-0.07, 0.09]) {
    geometry.box(mb, mat4.translation(0, y, -0.045), 0.2, 0.035, 0.03, c.strap);
  }
  return [mb, null];
}

// ---------------------------------------------------------------------------
// Galactic Sci-Fi
// ---------------------------------------------------------------------------

// Vector Blade: a machined hilt projecting a hard-light edge. The blade is
// returned as a SECOND mesh tagged emissive, so it ignores the lighting
// model and reads as energy rather than painted metal.
function vectorBlade(c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  hilt(mb, 0.022, 0.09, 0.028, c.plate_dark, c.plate);

  // Emitter housing.
  geometry.extrude(
    mb,
    null,
    geometry.boxProfile(0.72, 0.4),
    [
      { y: 0.09, w: 0.042 },
      { y: 0.128, w: 0.046 },
      { y: 0.165, w: 0.034 },
    ],
    c.plate,
  );
  geometry.box(mb, mat4.translation(0, 0.106, 0.04), 0.05, 0.03, 0.014, c.plate_dark);

  const glow = new PartBuilder();
  // Vent slots in the housing, then the blade itself.
  for (const y of [0.1, 0.122]) {
    geometry.box(glow, mat4.translation(0, y, 0.047), 0.036, 0.008, 0.006, c.seam);
  }
  geometry.extrude(
    glow,
    null,
    BLADE_PROFILE,
    [
      { y: 0.168, w: 0.03, d: 0.0075 },
      { y: 0.3, w: 0.032, d: 0.008 },
      { y: 0.66, w: 0.027, d: 0.0065 },
      { y: 0.82, w: 0.002, d: 0.0006 },
    ],
    c.seam,
  );
  return [mb, glow];
}

// Pulse Blaster: a compact sidearm. Carried holstered on the hip, which also
// proves a non-hand attachment socket.
function pulseBlaster(c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  geometry.extrude(
    mb,
    null,
    geometry.boxProfile(0.55, 0.35),
    [
      { y: -0.075, w: 0.026 },
      { y: 0.0, w: 0.03 },
    ],
    c.plate_dark,
  ); // grip
  geometry.box(mb, mat4.translation(0, 0.032, 0.02), 0.056, 0.07, 0.15, c.plate); // receiver
  geometry.extrude(
    mb,
    mat4.multiply(mat4.translation(0, 0.04, 0.115), mat4.rotationX(Math.PI / 2)),
    geometry.circleProfile(10),
    [
      { y: 0, w: 0.026 },
      { y: 0.07, w: 0.021 },
    ],
    c.plate_dark,
  );

  const glow = new PartBuilder();
  geometry.box(glow, mat4.translation(0, 0.07, 0.01), 0.03, 0.01, 0.09, c.seam);
  geometry.extrude(
    glow,
    mat4.multiply(mat4.translation(0, 0.04, 0.186), mat4.rotationX(Math.PI / 2)),
    geometry.circleProfile(10),
    [
      { y: 0, w: 0.017 },
      { y: 0.01, w: 0.017 },
    ],
    c.seam,
  );
  return [mb, glow];
}

// ---------------------------------------------------------------------------
// Toybox
// ---------------------------------------------------------------------------

// Foam Champion: a fat, soft, obviously harmless sword. Same socket and same
// semantic verb as the Tournament Sword and Vector Blade.
function foamChampion(c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  hilt(mb, 0.026, 0.08, 0.04, c.plate_dark, c.accent);

  // Chunky moulded guard.
  geometry.extrude(
    mb,
    null,
    geometry.boxProfile(0.8, 0.85),
    [
      { y: 0.076, w: 0.052 },
      { y: 0.104, w: 0.078 },
      { y: 0.132, w: 0.05 },
    ],
    c.accent,
  );

  // The foam blade: rounded rectangle, barely tapered, domed tip.
  geometry.extrude(
    mb,
    null,
    geometry.boxProfile(0.44, 0.92),
    [
      { y: 0.13, w: 0.062 },
      { y: 0.165, w: 0.078 },
      { y: 0.43, w: 0.076 },
      { y: 0.52, w: 0.066 },
      { y: 0.565, w: 0.04 },
      { y: 0.585, w: 0.014 },
    ],
    c.plate,
  );
  // A cream stripe so the foam reads as moulded plastic, not a slab.
  geometry.extrude(
    mb,
    null,
    geometry.boxProfile(0.44, 0.92),
    [
      { y: 0.3, w: 0.08 },
      { y: 0.33, w: 0.08 },
    ],
    c.plate_dark,
  );
  return [mb, null];
}

// Spring Glove: an oversized moulded gauntlet with a visible coil.
function springGlove(c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const mb = new PartBuilder();
  geometry.sphere(mb, mat4.translation(0, -0.03, 0), 0.088, 6, 12, c.plate);
  // Cuff.
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(12),
    [
      { y: 0.03, w: 0.086 },
      { y: 0.062, w: 0.094 },
      { y: 0.08, w: 0.08 },
    ],
    c.accent,
  );
  // The spring: a stack of rings stepping forward off the knuckles.
  for (let i = 0; i <= 3; i++) {
    geometry.extrude(
      mb,
      mat4.translation(0, -0.04, 0.078 + i * 0.026),
      geometry.circleProfile(10),
      [
        { y: -0.01, w: 0.05 - i * 0.004 },
        { y: 0.01, w: 0.05 - i * 0.004 },
      ],
      i % 2 === 0 ? c.plate_dark : c.accent,
    );
  }
  return [mb, null];
}

// ---------------------------------------------------------------------------

const BUILDERS: Readonly<Record<string, (c: SlotIndex) => readonly [PartBuilder, PartBuilder | null]>> = {
  tournament_sword: tournamentSword,
  heater_shield: heaterShield,
  vector_blade: vectorBlade,
  pulse_blaster: pulseBlaster,
  foam_champion: foamChampion,
  spring_glove: springGlove,
};

/**
 * Builds one item by id. Returns the solid PART BUILDER plus an optional
 * emissive one (null for everything that does not glow). Neither is a mesh:
 * both are folded into the character's single geometry by body.accumulate
 * (#337 slice 2), which is what lets an emissive blade cost zero extra draws.
 */
export function build(id: string, c: SlotIndex): readonly [PartBuilder, PartBuilder | null] {
  const fn = BUILDERS[id];
  if (!fn) {
    throw new Error(`unknown equipment id: ${id}`);
  }
  return fn(c);
}
