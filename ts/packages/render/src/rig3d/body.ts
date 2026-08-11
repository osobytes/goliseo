// Builds one themed character presentation on the shared Rig_Medium skeleton.
//
// Nothing in this file knows which theme it is building. It reads shape flags
// and a resolved palette out of themes.ts and proportions out of
// proportions.ts, so all three presentations come from this one code path. If
// a theme ever needs a bespoke generator, that is a signal the theme contract
// is under-specified, not that this file should grow a branch.
//
// Kit rules -- what makes armour read as *equipped* rather than as body
// paint, learned the hard way when v1 buried the greaves inside the shins:
//   1. every piece is authored as its own PART: its own builder, in its own
//      local space, riding its own bone, with its own material;
//   2. it is `gear.over` times wider than the limb it wraps;
//   3. it flares into a rim at each open end;
//   4. a dark strap ring sits in the gap where it meets the body.
//
// Rule 1 used to read "every piece is its own mesh and its own draw call",
// and #337 slice 2 overturned that MECHANISM: the parts are merged into one
// geometry. The RULE survives because the draw call was never what did the
// work. What sells "equipped" is entirely geometry -- the `gear.over`
// standoff of 2, the rim flare of 3, the dark strap ring of 4 -- and geometry
// is authored per part whether or not each part ends up a separate mesh.
// Keeping a piece a distinct part is still load-bearing: it is the unit that
// carries a bone, a material and an attach transform, and the unit whose
// local space lets 2-4 be written in limb-relative numbers instead of
// rig-absolute ones. Merge parts into one BUILDER, never into one shape.

import { mat4, type Mat4 } from "@gc/core";
import * as equipment from "./equipment.ts";
import * as face from "./face.ts";
import * as geometry from "./geometry.ts";
import { PartBuilder } from "./geometry.ts";
import * as headgear from "./headgear.ts";
import type { HeadGeometry } from "./headgear.ts";
import type { RigForm, RigProportions } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";
import * as themes from "./themes.ts";
import type { Figure, Loadout, SlotIndex, Theme } from "./themes.ts";

type AddFn = (bone: string, builder: PartBuilder, attach?: Mat4, material?: string) => void;

const SIDES: readonly { readonly name: "R" | "L"; readonly sign: number }[] = [
  { name: "R", sign: -1 },
  { name: "L", sign: 1 },
]; // his right is -X

// Where each item hangs. The three light_melee items deliberately share one
// entry: same bone, same transform, different geometry. That is the reskin
// test.
//
// Exported since #447 so `presentation_content.spec.ts` can assert that every
// equipment id `gc-data` can name actually has somewhere to hang -- a
// mapping table that resolves to an item with no socket throws at mesh-build
// time, inside a `try` that disables rigged players entirely, which is the
// least diagnosable place for it to happen.
export const SOCKETS: Readonly<Record<string, string>> = {
  tournament_sword: "hand_r",
  vector_blade: "hand_r",
  foam_champion: "hand_r",
  heater_shield: "forearm_l",
  spring_glove: "hand_l",
  pulse_blaster: "hip",
};

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

// A limb: a tube from the joint down -Y with rounded ends.
function limb(
  mb: PartBuilder,
  f: RigForm,
  r0: number,
  r1: number,
  length: number,
  color: number,
): void {
  const c0 = r0 * f.cap * 0.42;
  const c1 = r1 * f.cap * 0.42;
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(f.segments),
    [
      { y: c0 * 0.95, w: r0 * 0.52 },
      { y: c0 * 0.55, w: r0 * 0.88 },
      { y: 0, w: r0 },
      { y: -length, w: r1 },
      { y: -length - c1 * 0.55, w: r1 * 0.88 },
      { y: -length - c1 * 0.95, w: r1 * 0.52 },
    ],
    color,
  );
}

// An armour sleeve standing proud of a limb, flared into a lip at both ends,
// with straps biting into the gap.
function sleeve(
  mb: PartBuilder,
  rig: RigProportions,
  limbR: number,
  y0: number,
  y1: number,
  color: number,
  strap: number,
): void {
  const f = rig.form;
  const r = limbR * rig.gear.over;
  const lip = Math.min((y1 - y0) * 0.16, r * 0.3);
  const profile = geometry.circleProfile(f.segments);

  geometry.extrude(
    mb,
    null,
    profile,
    [
      { y: y0, w: r * 1.13 },
      { y: y0 + lip, w: r },
      { y: y1 - lip, w: r },
      { y: y1, w: r * 1.13 },
    ],
    color,
  );
  for (const y of [y0 + lip * 1.25, y1 - lip * 1.25]) {
    geometry.extrude(
      mb,
      null,
      profile,
      [
        { y: y - lip * 0.22, w: r * 1.04 },
        { y: y + lip * 0.22, w: r * 1.04 },
      ],
      strap,
    );
  }
}

// A thin ring. Emissive seams and plate ridges are both this shape.
function band(
  mb: PartBuilder,
  f: RigForm,
  r: number,
  y: number,
  halfH: number,
  color: number,
): void {
  geometry.extrude(
    mb,
    null,
    geometry.circleProfile(f.segments),
    [
      { y: y - halfH, w: r },
      { y: y + halfH, w: r },
    ],
    color,
  );
}

// ---------------------------------------------------------------------------
// Body
// ---------------------------------------------------------------------------

// Head silhouette per figure style. Returns the extrude sections, the
// profile to sweep, and a function giving the front-surface Z at any height
// -- which is what lets face.ts push features onto any head shape.
function headForm(
  figure: Figure,
  hr: number,
  baseY: number,
): {
  sections: geometry.Section[];
  profile: geometry.Profile;
  hh: number;
  frontAt: (y: number) => number;
} {
  const hh = hr * figure.head_h;
  let sections: geometry.Section[] = [];
  let profile: geometry.Profile;
  let depth: number;

  if (figure.head_shape === "cylinder") {
    // Lego: a straight cylinder, very slightly barrelled at the rim.
    profile = geometry.circleProfile(16);
    depth = 1.0;
    sections = [
      { y: baseY, w: hr * 0.96 },
      { y: baseY + hh * 0.06, w: hr },
      { y: baseY + hh * 0.94, w: hr },
      { y: baseY + hh, w: hr * 0.96 },
    ];
  } else if (figure.head_shape === "boxround") {
    // Funko: a tall rounded box, flat-sided with domed ends.
    profile = geometry.boxProfile(0.86, 0.8);
    depth = 0.86;
    for (let i = 0; i <= 10; i++) {
      const t = i / 10;
      sections.push({ y: baseY + t * hh, w: hr * Math.sin(Math.PI * (0.13 + 0.74 * t)) ** 0.3 });
    }
  } else {
    profile = geometry.boxProfile(0.92, 0.55);
    depth = 0.92;
    for (let i = 0; i <= 9; i++) {
      const t = i / 9;
      sections.push({ y: baseY + t * hh, w: hr * Math.sin(Math.PI * (0.08 + 0.84 * t)) ** 0.62 });
    }
  }

  // Linear search + lerp: the section list is a handful of entries.
  const frontAt = (y: number): number => {
    for (let i = 0; i < sections.length - 1; i++) {
      const a = sections[i];
      const b = sections[i + 1];
      if (!a || !b) {
        continue;
      }
      if (y >= a.y && y <= b.y) {
        const u = (y - a.y) / Math.max(b.y - a.y, 1e-6);
        return (a.w + (b.w - a.w) * u) * depth * 0.94;
      }
    }
    const last = sections[sections.length - 1];
    return (last ? last.w : 0) * depth * 0.94;
  };

  return { sections, profile, hh, frontAt };
}

// A Lego C-clamp hand: a broken ring whose gap faces forward, so a weapon
// shaft passes through it.
function clampHand(mb: PartBuilder, r: number, color: number): void {
  const tube = r * 0.34;
  const steps = 10;
  for (let i = 0; i <= steps; i++) {
    const a = Math.PI * 0.5 + 0.62 + (i / steps) * (Math.PI * 2 - 1.24);
    geometry.sphere(
      mb,
      mat4.translation(r * Math.cos(a), -r * 1.1, r * Math.sin(a)),
      tube,
      4,
      8,
      color,
    );
  }
}

// The skull's measurements, shared by the face and by whatever helm goes on it.
function headGeometry(rig: RigProportions, figure: Figure): HeadGeometry {
  const f = rig.form;
  const hr = f.head_r * figure.head_scale;
  const base = 0.02 + figure.head_drop;
  const hh = hr * figure.head_h;
  return { hr, base, hh, eye: base + hh * f.eye_y };
}

function buildBody(
  add: AddFn,
  rig: RigProportions,
  theme: Theme,
  figure: Figure,
  c: SlotIndex,
): void {
  const s = rig.seg;
  const f = rig.form;
  const shape = theme.shape;

  // Torso.
  const torso = new PartBuilder();
  const bulge = f.torso_r * f.torso_bulge;
  if (figure.torso === "trapezoid") {
    // Minifig: a flat-sided block, wider at the hem than the shoulders.
    geometry.extrude(
      torso,
      null,
      geometry.boxProfile(0.62, 0.18),
      [
        { y: -s.spine - 0.02, w: bulge * 1.04 },
        { y: s.chest * 0.55, w: bulge * 0.94 },
        { y: s.chest + s.neck * 0.34, w: bulge * 0.86 },
        { y: s.chest + s.neck * 0.6, w: bulge * 0.62 },
      ],
      c.cloth,
    );
  } else {
    geometry.extrude(
      torso,
      null,
      geometry.boxProfile(0.66, shape.bevel),
      [
        { y: -s.spine - 0.02, w: f.torso_r * 0.86 },
        { y: 0, w: f.torso_r },
        { y: s.chest * 0.55, w: bulge },
        { y: s.chest + s.neck * 0.3, w: bulge * 0.94 },
        { y: s.chest + s.neck * 0.62, w: f.torso_r * 0.72 },
      ],
      c.cloth,
    );
  }
  add("spine", torso);

  if (figure.neck) {
    const neck = new PartBuilder();
    limb(neck, f, f.neck_r, f.neck_r * 0.94, s.neck * 0.55, c.skin);
    add("neck", neck);
  }

  // Head + face.
  const head = new PartBuilder();
  const g = headGeometry(rig, figure);
  const { sections, profile, frontAt } = headForm(figure, g.hr, g.base);
  geometry.extrude(head, null, profile, sections, c.skin);
  if (figure.stud) {
    geometry.extrude(
      head,
      null,
      geometry.circleProfile(12),
      [
        { y: g.base + g.hh, w: g.hr * 0.4 },
        { y: g.base + g.hh + g.hr * 0.16, w: g.hr * 0.4 },
      ],
      c.skin,
    );
  }
  face.build(head, c, g.hr, figure.face, frontAt, g.eye);
  add("head", head);

  for (const side of SIDES) {
    const n = side.name;
    const tube = figure.limbs === "tube";
    const taper = tube ? 1.0 : f.arm_taper;
    const legTaper = tube ? 1.0 : f.leg_taper;

    const upper = new PartBuilder();
    limb(upper, f, f.arm_r, f.arm_r * taper, s.upperarm, c.limbs);
    add(`upper_arm.${n}`, upper);

    const lower = new PartBuilder();
    limb(lower, f, f.arm_r * taper, f.arm_r * taper ** 2, s.lowerarm, c.limbs);
    add(`forearm.${n}`, lower);

    const hand = new PartBuilder();
    if (figure.hands === "clamp") {
      clampHand(hand, f.hand_r * 1.05, c.limbs);
    } else {
      geometry.sphere(hand, mat4.translation(0, -f.hand_r * 0.95, 0), f.hand_r, 5, 10, c.limbs);
    }
    add(`hand.${n}`, hand);

    const thigh = new PartBuilder();
    limb(thigh, f, f.leg_r, f.leg_r * legTaper, s.upperleg, c.cloth);
    add(`thigh.${n}`, thigh);

    const shin = new PartBuilder();
    limb(shin, f, f.leg_r * legTaper, f.leg_r * legTaper ** 2, s.lowerleg, c.limbs);
    add(`shin.${n}`, shin);

    if (shape.joint_balls) {
      const joints: readonly { readonly bone: string; readonly r: number }[] = [
        { bone: `upper_arm.${n}`, r: f.arm_r * 1.16 },
        { bone: `forearm.${n}`, r: f.arm_r * taper * 1.18 },
        { bone: `thigh.${n}`, r: f.leg_r * 1.12 },
        { bone: `shin.${n}`, r: f.leg_r * legTaper * 1.16 },
      ];
      for (const joint of joints) {
        const ball = new PartBuilder();
        geometry.sphere(ball, null, joint.r, 6, f.segments, c.joint);
        add(joint.bone, ball);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Kit
// ---------------------------------------------------------------------------

function buildKit(add: AddFn, rig: RigProportions, theme: Theme, c: SlotIndex): void {
  const s = rig.seg;
  const f = rig.form;
  const shape = theme.shape;
  const over = rig.gear.over;

  const cuirass = new PartBuilder();
  const cr = f.torso_r * f.torso_bulge * over;
  geometry.extrude(
    cuirass,
    null,
    geometry.boxProfile(0.66, shape.bevel),
    [
      { y: -s.spine * 0.35, w: cr * 1.1 },
      { y: -s.spine * 0.05, w: cr * 0.97 },
      { y: s.chest * 0.55, w: cr },
      { y: s.chest + s.neck * 0.26, w: cr * 0.93 },
      { y: s.chest + s.neck * 0.46, w: cr * 0.74 },
    ],
    c.plate,
  );
  for (let i = 1; i <= shape.plate_layers; i++) {
    const t = shape.plate_layers > 1 ? (i - 1) / (shape.plate_layers - 1) : 0;
    const y = s.chest * (0.08 + 0.3 * t);
    geometry.extrude(
      cuirass,
      null,
      geometry.boxProfile(0.66, shape.bevel),
      [
        { y: y - s.chest * 0.045, w: cr * 1.05 },
        { y: y + s.chest * 0.045, w: cr * 1.05 },
      ],
      c.plate_dark,
    );
  }
  if (shape.heraldic_block) {
    geometry.box(
      cuirass,
      mat4.translation(0, s.chest * 0.66, cr * 0.7),
      cr * 0.6,
      s.chest * 0.48,
      0.03,
      c.cloth,
    );
    geometry.box(
      cuirass,
      mat4.translation(0, s.chest * 0.66, cr * 0.72),
      cr * 0.18,
      s.chest * 0.48,
      0.03,
      c.accent,
    );
  }
  add("chest", cuirass, undefined, "metal");

  // Sci-Fi: luminous seams tracing the panel joins.
  if (shape.seams) {
    const seam = new PartBuilder();
    geometry.extrude(
      seam,
      null,
      geometry.boxProfile(0.66, shape.bevel),
      [
        { y: s.chest * 0.5, w: cr * 1.02 },
        { y: s.chest * 0.545, w: cr * 1.02 },
      ],
      c.seam,
    );
    geometry.box(
      seam,
      mat4.translation(0, s.chest * 0.82, cr * 0.72),
      cr * 0.14,
      s.chest * 0.28,
      0.022,
      c.seam,
    );
    add("chest", seam, undefined, "emissive");
  }

  const belt = new PartBuilder();
  const br = f.torso_r * over * 0.98;
  geometry.extrude(
    belt,
    null,
    geometry.boxProfile(0.66, shape.bevel),
    [
      { y: -0.012, w: br * 1.03 },
      { y: 0.048, w: br * 1.03 },
    ],
    c.strap,
  );
  geometry.box(belt, mat4.translation(0, 0.018, br * 0.74), br * 0.3, 0.052, 0.03, c.accent);
  add("hips", belt, undefined, "metal");

  for (const side of SIDES) {
    const n = side.name;

    const pauldron = new PartBuilder();
    const pr = f.arm_r * over * 1.62;
    geometry.sphere(pauldron, mat4.translation(0, -pr * 0.16, 0), pr, 5, f.segments, c.plate);
    band(pauldron, f, pr * 0.96, -pr * 0.46, pr * 0.13, c.plate_dark);
    add(`shoulder.${n}`, pauldron, undefined, "metal");

    const bracer = new PartBuilder();
    sleeve(
      bracer,
      rig,
      f.arm_r * f.arm_taper,
      -s.lowerarm * 0.88,
      -s.lowerarm * 0.3,
      c.accent,
      c.strap,
    );
    add(`forearm.${n}`, bracer, undefined, "metal");

    const greave = new PartBuilder();
    sleeve(
      greave,
      rig,
      f.leg_r * f.leg_taper,
      -s.lowerleg * 0.86,
      -s.lowerleg * 0.16,
      c.plate,
      c.strap,
    );
    add(`shin.${n}`, greave, undefined, "metal");

    if (shape.seams) {
      const seam = new PartBuilder();
      band(
        seam,
        f,
        f.leg_r * f.leg_taper * over * 1.02,
        -s.lowerleg * 0.42,
        s.lowerleg * 0.02,
        c.seam,
      );
      add(`shin.${n}`, seam, undefined, "emissive");
    }

    // Heel and midfoot ride the ankle...
    const boot = new PartBuilder();
    geometry.extrude(
      boot,
      null,
      geometry.boxProfile(1.15, 0.3),
      [
        { y: -f.foot_w * 0.62, w: f.foot_w * 0.5, z: f.foot_len * 0.06 },
        { y: -f.foot_w * 0.16, w: f.foot_w * 0.48, z: f.foot_len * 0.05 },
        { y: f.foot_w * 0.22, w: f.foot_w * 0.44, z: 0 },
      ],
      c.strap,
    );
    geometry.box(
      boot,
      mat4.translation(0, -f.foot_w * 0.72, f.foot_len * 0.06),
      f.foot_w,
      f.foot_w * 0.22,
      f.foot_len * 0.66,
      c.plate_dark,
    );
    add(`foot.${n}`, boot);

    // ...and the toe box rides the ball of the foot, so a plant, a
    // push-off or a kick contact can actually roll.
    const toe = new PartBuilder();
    geometry.extrude(
      toe,
      null,
      geometry.boxProfile(0.86, 0.45),
      [
        { y: -f.foot_w * 0.26, w: f.foot_w * 0.46, z: f.foot_len * 0.09 },
        { y: f.foot_w * 0.12, w: f.foot_w * 0.42, z: f.foot_len * 0.05 },
      ],
      c.strap,
    );
    geometry.box(
      toe,
      mat4.translation(0, -f.foot_w * 0.36, f.foot_len * 0.09),
      f.foot_w * 0.94,
      f.foot_w * 0.2,
      f.foot_len * 0.36,
      c.plate_dark,
    );
    add(`toe.${n}`, toe);
  }
}

// ---------------------------------------------------------------------------
// Loadout
// ---------------------------------------------------------------------------

// Sockets are BONES now (see skeleton.ts), so most of the old attachment
// matrices are gone: the bone's rest offset and rest rotation carry the
// grip. What is left here is the small per-item adjustment on top of its
// socket.
function socketTransform(socket: string, rig: RigProportions): readonly [string, Mat4 | undefined] {
  const s = rig.seg;
  if (socket === "hand_r" || socket === "hand_l") {
    return [`socket_hand.${socket === "hand_r" ? "R" : "L"}`, undefined];
  } else if (socket === "forearm_l") {
    // Slide the shell down its own axis so it covers hip to shoulder.
    return ["socket_shield.L", mat4.translation(0, -0.1, 0)];
  } else if (socket === "hip") {
    // Holstered on his left hip for a cross-body draw.
    return [
      "hips",
      mat4.chain(
        mat4.translation(s.thigh_x * 1.5, -0.045, -0.02),
        mat4.rotationZ((14 * Math.PI) / 180),
        mat4.rotationX((-8 * Math.PI) / 180),
      ),
    ];
  }
  throw new Error(`unknown socket: ${socket}`);
}

// HEADGEAR IS NOT LOADOUT, EVEN THOUGH IT IS BUILT HERE. `theme.head` is
// part of what a theme's character IS -- a great helm, a visor helm, a
// moulded figure helm -- so it is built from the theme unconditionally and
// is NOT affected by `loadout`. Only the carried items are. A keeper with an
// empty loadout still wears their helmet (#447).
function buildLoadout(
  add: AddFn,
  rig: RigProportions,
  theme: Theme,
  figure: Figure,
  c: SlotIndex,
  loadout: Loadout,
): void {
  const [solid, glow] = headgear.build(theme.head, c, headGeometry(rig, figure));
  add("head", solid, undefined, "metal");
  if (glow) {
    add("head", glow, undefined, "emissive");
  }

  const ids = Object.values(loadout).filter((id): id is string => id !== undefined);
  for (const id of ids) {
    const socket = SOCKETS[id];
    if (!socket) {
      throw new Error(`no socket for ${id}`);
    }
    const [bone, attach] = socketTransform(socket, rig);
    const [item, itemGlow] = equipment.build(id, c);
    add(bone, item, attach, "metal");
    if (itemGlow) {
      add(bone, itemGlow, attach, "emissive");
    }
  }
}

// ---------------------------------------------------------------------------

/** One part on its way into `merge`, plus the diagnostics tests read. */
export interface AccumulatedPart {
  readonly builder: PartBuilder;
  readonly bone_name: string;
  readonly bone: number;
  readonly attach?: Mat4 | undefined;
  readonly material?: string | undefined;
}

// Generates every PART of one themed character and folds them into the
// single builder that becomes its geometry.
//
// Deliberately separate from a future `body.build`: everything here is pure
// TypeScript, so the whole generation path -- bone assignment, material
// assignment, attach baking, the merge itself -- is exercised headless, with
// no renderer required. Turning the result into an actual
// `THREE.SkinnedMesh` is the next, GPU-adjacent step (see this file's
// `geometry.ts` header for why that step is deferred).
//
// Colour is NOT baked in (#337 slice 1): every vertex carries a palette slot
// index, resolved against a team's actual colours later (see
// `themes.resolvedPalette`). That is what makes this build reusable across
// every team -- and every future cosmetic palette -- without rebuilding a
// single mesh.
//
// `loadout` (#447) OVERRIDES `theme.loadout`, and defaults to it. The theme's
// own authored loadout is what a character carries when nobody says
// otherwise -- a standalone preview, a diagnostic render, this package's own
// fixtures. On the product path the caller passes the player's AUTHORED
// loadout instead, resolved from their `loadout_id`, and an empty `{}` for a
// player who carries nothing. That is the whole of the keeper fix at this
// layer: no `is_keeper` branch, just the data the player already has.
export function accumulate(
  rig: RigProportions,
  theme: Theme,
  figure: Figure,
  loadout: Loadout = theme.loadout,
): readonly [PartBuilder, readonly AccumulatedPart[]] {
  const c = themes.SLOT_INDEX;
  const boneIndex = skeleton.boneIndex(rig);
  const parts: AccumulatedPart[] = [];
  const add: AddFn = (bone, builder, attach, material) => {
    const index = boneIndex[bone];
    // A typo'd bone name used to place a part at the identity transform and
    // render it at the character's feet; now it would index off the end of
    // the bone array, which is undefined rather than merely wrong.
    if (index === undefined) {
      throw new Error(`rig3d part refers to a bone the skeleton does not have: ${bone}`);
    }
    parts.push({ builder, bone_name: bone, bone: index, attach, material });
  };

  buildBody(add, rig, theme, figure, c);
  buildKit(add, rig, theme, c);
  buildLoadout(add, rig, theme, figure, c, loadout);

  const mergeParts: geometry.Part[] = parts.map((p) => ({
    builder: p.builder,
    bone: p.bone,
    attach: p.attach,
    material: p.material,
  }));
  return [geometry.merge(mergeParts), parts];
}
