// Ported from game/render/player_renderer.lua.
//
// Procedural billboard avatar, drawn entirely as flat 2D primitives in
// screen space (no sprite sheets, no 3D mesh -- see `player_renderer_3d.ts`
// for the rigged alternative). The figure stands upright facing the camera;
// body motion lives in screen space (bob, limb pump, lean) while aim/facing
// stays a ground-plane vector. All sizes scale off `r` (the projected body
// radius) so far players shrink with the perspective.
//
// Pure content: `playerDrawCommands` returns a `DrawCommand[]` (draw2d.ts)
// rather than calling `love.graphics`/three.js directly, so every branch --
// every pose, every piece of equipment -- is testable headless. The Lua
// original's `love.graphics.push(); translate(...); rotate(...);
// translate(...)` blocks (dive lunges, aerial rotations, knockback/stagger,
// stumble) become a `PivotTransform` threaded through as a parameter instead
// of a graphics-state stack (see draw2d.ts's `rotateAround`); the Lua
// original's `alpha_mul` module global (used for faded dash afterimages)
// becomes an explicit parameter for the same reason -- no mutable renderer
// state survives the port.
//
// Boundary note (v2/README.md rule 6.7): `PlayerPoseSelection` is
// `render/player_pose.lua`'s (the Rust RenderFrame producer, `render/**` ->
// `crates/gc-render`) -- only the fields this module reads are declared
// locally. `CombatPlayerPresentation` is reused from `@gc/presentation` (a
// declared dependency). `PlayerView` is reused from `view_state.ts` (already
// ported, same package).

import * as THREE from "three";
import type { Vec2 } from "@gc/core";
import type { CombatPlayerPresentation } from "@gc/presentation";
import type { PlayerView } from "./view_state.ts";
import { DrawList, paint, type DrawCommand, type PivotTransform, type RGB } from "./draw2d.ts";

export type AerialStyle = "leg_control" | "chest_control" | "volley" | "header" | "bicycle";
export type AerialOutcome = "clean" | "heavy" | "miss";
export type SpeciesShape = "round" | "broad" | "angular" | "cluster";

/** The slice of `render/player_pose.lua`'s pose selection this module reads. */
export interface PlayerPoseSelection {
  readonly id?: string;
  readonly priority?: number;
  readonly source?: string;
}

export interface PlayerRenderOptions {
  readonly facing?: Vec2;
  readonly is_keeper: boolean;
  readonly controlled: boolean;
  readonly dashing?: boolean;
  readonly dive?: number;
  readonly dive_dir?: Vec2;
  readonly holding?: boolean;
  readonly grab?: number;
  readonly throw?: number;
  readonly windup?: number;
  readonly aerial?: number;
  readonly aerial_style?: AerialStyle;
  readonly aerial_outcome?: AerialOutcome;
  readonly aerial_jump?: number;
  readonly species_shape?: SpeciesShape;
  readonly species_color?: RGB;
  readonly team?: "home" | "away";
  readonly combat?: CombatPlayerPresentation;
  readonly pose?: PlayerPoseSelection;
}

export interface PlayerSilhouetteProfile {
  readonly torso_scale: number;
  readonly limb_scale: number;
  readonly head_kind: SpeciesShape;
}

const SILHOUETTES: Readonly<Record<SpeciesShape, PlayerSilhouetteProfile>> = {
  round: { torso_scale: 1.1, limb_scale: 1, head_kind: "round" },
  broad: { torso_scale: 1.5, limb_scale: 1.22, head_kind: "broad" },
  angular: { torso_scale: 0.82, limb_scale: 0.82, head_kind: "angular" },
  cluster: { torso_scale: 0.76, limb_scale: 1, head_kind: "cluster" },
};

export function silhouette(shape: SpeciesShape): PlayerSilhouetteProfile {
  const profile = SILHOUETTES[shape];
  if (profile === undefined) {
    throw new Error(`unknown player silhouette: ${String(shape)}`);
  }
  return profile;
}

function clamp(x: number, a: number, b: number): number {
  return Math.max(a, Math.min(b, x));
}

// Lighten a colour toward white for joint/visor accents.
function lighten(c: RGB, t: number): RGB {
  return [c[0] + (1 - c[0]) * t, c[1] + (1 - c[1]) * t, c[2] + (1 - c[2]) * t];
}

function mul(alphaMul: number, a?: number): number {
  return (a ?? 1) * alphaMul;
}

function drawEquipment(
  dl: DrawList,
  t: PivotTransform | undefined,
  alphaMul: number,
  cx: number,
  shY: number,
  hipY: number,
  r: number,
  opts: PlayerRenderOptions,
): void {
  const combat = opts.combat;
  if (combat === undefined || combat.equipment_presentation_id === undefined || combat.family_id === undefined) {
    return;
  }
  const fx = opts.facing !== undefined ? opts.facing.x : 1;
  const side = fx >= 0 ? 1 : -1;
  const active = combat.phase === "active";
  const raised = combat.phase === "guard" || combat.phase === "aim";
  const reach = active ? 1.35 : raised ? 1.05 : 0.78;
  const handX = cx + side * r * reach;
  const handY = raised ? shY + r * 0.28 : hipY - r * 0.3;
  const id = combat.equipment_presentation_id;

  if (id === "toy_spring_gloves") {
    const extension = active ? r * 0.9 : r * 0.25;
    const frontX = handX + side * extension;
    dl.line(
      [
        handX - side * r * 0.22,
        handY,
        handX - side * r * 0.08,
        handY - r * 0.12,
        handX + side * r * 0.08,
        handY + r * 0.12,
        frontX,
        handY,
      ],
      [0.92, 0.96, 1],
      { alpha: mul(alphaMul, 0.95), lineWidth: Math.max(1, r * 0.12) },
      t,
    );
    dl.circle("fill", frontX, handY, r * 0.34, [0.4, 0.95, 1], { alpha: mul(alphaMul, 0.98) }, t);
    dl.circle("line", cx - side * r * 0.72, handY + r * 0.12, r * 0.3, [0.4, 0.95, 1], { alpha: mul(alphaMul, 0.98) }, t);
  } else if (id === "medieval_heater_shield") {
    const shieldX = handX + side * r * 0.08;
    const shieldY = raised ? shY + r * 0.42 : hipY - r * 0.18;
    const width = r * (raised ? 0.72 : 0.58);
    const height = r * (raised ? 1.05 : 0.85);
    dl.polygon(
      "fill",
      [
        shieldX - width,
        shieldY - height * 0.45,
        shieldX + width,
        shieldY - height * 0.45,
        shieldX + width * 0.72,
        shieldY + height * 0.45,
        shieldX,
        shieldY + height * 0.72,
        shieldX - width * 0.72,
        shieldY + height * 0.45,
      ],
      [0.92, 0.72, 0.28],
      { alpha: mul(alphaMul, 0.96) },
      t,
    );
    dl.line([shieldX, shieldY - height * 0.35, shieldX, shieldY + height * 0.45], [0.18, 0.12, 0.08], {
      alpha: mul(alphaMul, 0.82),
      lineWidth: Math.max(1, r * 0.12),
    }, t);
    dl.line([shieldX - width * 0.52, shieldY, shieldX + width * 0.52, shieldY], [0.18, 0.12, 0.08], {
      alpha: mul(alphaMul, 0.82),
      lineWidth: Math.max(1, r * 0.12),
    }, t);
  } else if (id === "scifi_pulse_blaster") {
    const muzzleX = handX + side * r * (active ? 1.0 : 0.68);
    const left = Math.min(handX - side * r * 0.18, muzzleX);
    const width = Math.abs(muzzleX - handX) + r * 0.25;
    dl.rect("fill", left, handY - r * 0.22, width, r * 0.44, [0.24, 0.14, 0.32], { alpha: mul(alphaMul, 0.98), rx: r * 0.1, ry: r * 0.1 }, t);
    dl.line([handX, handY, muzzleX, handY], [1, 0.45, 0.78], { alpha: mul(alphaMul, 0.98), lineWidth: Math.max(1, r * 0.13) }, t);
    dl.polygon(
      "line",
      [muzzleX, handY - r * 0.28, muzzleX + side * r * 0.35, handY, muzzleX, handY + r * 0.28],
      [1, 0.45, 0.78],
      { alpha: mul(alphaMul, 0.98) },
      t,
    );
  } else {
    const bladeLength = active ? r * 1.8 : r * 1.35;
    const tipX = handX + side * bladeLength;
    let bladeColor: RGB = [0.92, 0.96, 1];
    let bladeWidth = r * 0.14;
    if (id === "scifi_energy_blade") {
      bladeColor = [0.45, 1, 0.9];
      bladeWidth = r * 0.2;
    } else if (id === "toy_foam_sword") {
      bladeColor = [1, 0.55, 0.28];
      bladeWidth = r * 0.34;
    }
    dl.line([handX - side * r * 0.2, handY, handX + side * r * 0.18, handY], [0.2, 0.14, 0.12], {
      alpha: mul(alphaMul, 0.98),
      lineWidth: Math.max(1.5, r * 0.26),
    }, t);
    dl.line([handX, handY - r * 0.3, handX, handY + r * 0.3], [0.2, 0.14, 0.12], {
      alpha: mul(alphaMul, 0.98),
      lineWidth: Math.max(1.5, r * 0.26),
    }, t);
    dl.line([handX + side * r * 0.15, handY, tipX, handY], bladeColor, {
      alpha: mul(alphaMul, 0.98),
      lineWidth: Math.max(1.5, bladeWidth),
    }, t);
    if (id === "toy_foam_sword") {
      dl.circle("fill", tipX, handY, r * 0.2, bladeColor, { alpha: mul(alphaMul, 0.98) }, t);
    }
  }
}

// Draws just the standing body (legs, arms, torso, helmet) centred on
// screen-x `bx`, feet at `gy`. No shadow / selection ring / facing tick --
// those are drawn once by `playerDrawCommands` so afterimage ghosts don't
// duplicate them.
function figure(
  dl: DrawList,
  t: PivotTransform | undefined,
  alphaMul: number,
  bx: number,
  gy: number,
  r: number,
  color: RGB,
  v: PlayerView | undefined,
  opts: PlayerRenderOptions,
): void {
  const sp = v?.speed ?? 0;
  const ph = v?.phase ?? 0;
  const run = clamp(sp / 90, 0, 1); // 0 idle .. 1 full sprint
  const lean = v?.lean ?? 0;
  const accent = opts.species_color ?? lighten(color, 0.55);
  const shape = opts.species_shape ?? "round";
  const sil = silhouette(shape);
  const poseId = opts.pose?.id;

  const swing = Math.sin(ph); // fore/aft limb phase
  // Whole-body bounce: a gentle idle breath plus a run bob that peaks twice per stride.
  const bounce = run * Math.abs(Math.sin(ph)) * r * 0.16;
  const breath = (1 - run) * Math.sin(ph * 0.5 + bx) * r * 0.04;

  // Wind-up back-swing: lean the whole figure opposite the facing direction.
  let wu = opts.windup ?? 0;
  if (poseId === "combat_windup") {
    wu = Math.max(wu, 1 - (opts.combat?.phase_progress ?? 0) * 0.45);
  }
  const fx = opts.facing?.x ?? 0;
  const windupLean = -fx * wu * r * 0.6; // leans back opposite facing
  const aerial = opts.aerial ?? 0;
  const aerialStyle = opts.aerial_style;
  let actionLean = 0;
  if (aerialStyle === "header") {
    actionLean = fx * aerial * r * 0.45;
  } else if (aerialStyle === "chest_control") {
    actionLean = -fx * aerial * r * 0.25;
  } else if (poseId === "combat_active") {
    actionLean = fx * r * 0.65;
  } else if (poseId === "combat_recovery") {
    actionLean = -fx * r * 0.2;
  } else if (poseId === "keeper_set") {
    actionLean = fx * r * 0.18;
  } else if (poseId === "keeper_get_up") {
    actionLean = -fx * r * 0.25;
  } else if (poseId === "tackle") {
    // A committed challenge throws the mass at the ball.
    actionLean = fx * r * 0.8;
  } else if (poseId === "run_telegraph") {
    actionLean = fx * r * 0.5;
  } else if (poseId === "kick_follow") {
    actionLean = fx * r * 0.28;
  } else if (poseId === "contain") {
    // Contain holds its weight back: it shepherds, it does not commit.
    actionLean = -fx * r * 0.3;
  } else if (poseId === "fatigue") {
    actionLean = -fx * r * 0.12;
  }

  const cx = bx + lean * r * 0.5 + windupLean + actionLean;
  const footY = gy;
  let stanceDrop = 0;
  if (poseId === "keeper_ready_low" || poseId === "keeper_spread") {
    stanceDrop = r * 0.32;
  } else if (poseId === "keeper_set") {
    stanceDrop = r * 0.18;
  } else if (poseId === "keeper_get_up") {
    stanceDrop = r * 0.42;
  } else if (poseId === "settle") {
    stanceDrop = r * 0.3;
  } else if (poseId === "contain") {
    stanceDrop = r * 0.26;
  } else if (poseId === "tackle") {
    stanceDrop = r * 0.2;
  } else if (poseId === "fatigue") {
    stanceDrop = r * 0.16;
  }
  // A slump lowers the shoulders and head without dropping the hips, so a
  // spent player reads as heavy rather than merely crouched.
  const slump = poseId === "fatigue" ? r * 0.24 : 0;
  const hipY = gy - r * 1.35 - bounce - breath + stanceDrop;
  const shY = gy - r * 2.15 - bounce - breath + stanceDrop + slump;
  const headY = gy - r * 2.75 - bounce - breath + stanceDrop + slump * 1.35;

  let stride = run * r * 0.65;
  if (poseId === "keeper_shuffle") {
    stride = r * 0.28;
  } else if (poseId === "run_telegraph") {
    // The first strides of a granted run drive harder than the gait the player has actually reached yet.
    stride = Math.max(stride, r * 0.58);
  } else if (poseId === "fatigue") {
    stride = Math.min(stride, r * 0.22);
  }
  const wideStance = poseId === "keeper_ready_low" || poseId === "keeper_set" || poseId === "keeper_spread" || poseId === "contain";
  let hipDx = r * (wideStance ? 0.52 : 0.34);
  if (poseId === "settle") {
    // Squarest, widest base on the pitch: the player is standing over the ball, not travelling anywhere.
    hipDx = r * 0.74;
  }

  // Legs (pump in opposite phase). Boots are chunky blocks at the feet.
  const limbScale = sil.limb_scale;
  const legAlpha = mul(alphaMul, opts.is_keeper ? 0.8 : 1);
  let lfx = cx - hipDx + swing * stride;
  let rfx = cx + hipDx - swing * stride;
  let lfy = footY;
  let rfy = footY;
  const strikeSign = fx >= 0 ? 1 : -1;
  if (aerialStyle === "volley" || aerialStyle === "leg_control") {
    lfx = cx + strikeSign * r * 1.45 * aerial;
    lfy = footY - r * 0.85 * aerial;
  } else if (poseId === "kick_follow") {
    // The striking leg keeps travelling after the ball has gone.
    lfx = cx + strikeSign * r * 1.5;
    lfy = footY - r * 0.6;
    rfx = cx - strikeSign * r * 0.28;
  } else if (poseId === "tackle") {
    // A directional reach: the near leg extends past any contain stance, the trailing leg stays planted behind the hips.
    lfx = cx + strikeSign * r * 2.0;
    rfx = cx - strikeSign * r * 0.45;
  } else if (poseId === "settle") {
    // Planted and square over the ball: no stride at all.
    lfx = cx - hipDx;
    rfx = cx + hipDx;
  } else if (poseId === "contain") {
    // Side-on: leading foot toward the ball, trailing foot well behind it, both flat.
    lfx = cx + strikeSign * r * 1.15;
    rfx = cx - strikeSign * r * 0.8;
  } else if (poseId === "stumble") {
    // Feet crossed under a body that is already past its balance point.
    lfx = cx - hipDx * 1.7;
    rfx = cx + hipDx * 0.35;
    lfy = footY - r * 0.22;
  }
  dl.line([cx - hipDx, hipY, lfx, lfy], color, { alpha: legAlpha, lineWidth: Math.max(1.5, r * 0.34 * limbScale) }, t);
  dl.line([cx + hipDx, hipY, rfx, rfy], color, { alpha: legAlpha, lineWidth: Math.max(1.5, r * 0.34 * limbScale) }, t);
  const bootAlpha = mul(alphaMul, 1);
  dl.line([lfx - r * 0.12, lfy, lfx + r * 0.18, lfy], accent, { alpha: bootAlpha, lineWidth: Math.max(1.5, r * 0.42) }, t);
  dl.line([rfx - r * 0.12, rfy, rfx + r * 0.18, rfy], accent, { alpha: bootAlpha, lineWidth: Math.max(1.5, r * 0.42) }, t);

  // Arms (opposite the legs, swinging the other way).
  const armAlpha = mul(alphaMul, opts.is_keeper ? 0.8 : 1);
  const armLineWidth = Math.max(1.5, r * 0.26 * limbScale);
  const armLine = (points: readonly number[]): void => dl.line(points, color, { alpha: armAlpha, lineWidth: armLineWidth }, t);
  if (poseId === "keeper_spread") {
    armLine([cx - r * 0.48, shY, cx - r * 1.4, shY + r * 0.28]);
    armLine([cx + r * 0.48, shY, cx + r * 1.4, shY + r * 0.28]);
  } else if (poseId === "keeper_central" || poseId === "keeper_stretch" || poseId === "keeper_tip") {
    const side = fx >= 0 ? 1 : -1;
    const reach = poseId === "keeper_tip" ? 1.75 : poseId === "keeper_stretch" ? 1.45 : 1.08;
    armLine([cx - r * 0.48, shY, cx + side * r * reach, shY - r * 0.2]);
    armLine([cx + r * 0.48, shY, cx + side * r * reach, shY + r * 0.3]);
  } else if (poseId === "keeper_set") {
    armLine([cx - r * 0.5, shY, cx - r * 0.82, hipY - r * 0.08]);
    armLine([cx + r * 0.5, shY, cx + r * 0.82, hipY - r * 0.08]);
  } else if (poseId === "keeper_ready_low") {
    armLine([cx - r * 0.5, shY, cx - r * 0.95, hipY + r * 0.25]);
    armLine([cx + r * 0.5, shY, cx + r * 0.95, hipY + r * 0.25]);
  } else if (poseId === "keeper_shuffle") {
    armLine([cx - r * 0.5, shY, cx - r * 0.8, hipY - r * 0.05]);
    armLine([cx + r * 0.5, shY, cx + r * 0.8, hipY - r * 0.05]);
  } else if (poseId === "keeper_ready_tall") {
    armLine([cx - r * 0.5, shY, cx - r * 0.82, shY + r * 0.48, cx - r * 0.62, hipY + r * 0.05]);
    armLine([cx + r * 0.5, shY, cx + r * 0.82, shY + r * 0.48, cx + r * 0.62, hipY + r * 0.05]);
  } else if (poseId === "keeper_get_up") {
    armLine([cx - r * 0.5, shY, cx - r * 1.0, footY - r * 0.15]);
    armLine([cx + r * 0.5, shY, cx + r * 0.7, hipY + r * 0.15]);
  } else if (poseId === "combat_guard") {
    armLine([cx - r * 0.5, shY, cx + fx * r * 0.75, shY + r * 0.2]);
    armLine([cx + r * 0.5, shY, cx + fx * r * 0.95, shY + r * 0.5]);
  } else if (poseId === "combat_active" || poseId === "combat_aim") {
    armLine([cx - r * 0.5, shY, cx + fx * r * 0.7, shY + r * 0.35]);
    armLine([cx + r * 0.5, shY, cx + fx * r * 1.0, shY + r * 0.15]);
  } else if (poseId === "tackle") {
    const side = fx >= 0 ? 1 : -1;
    armLine([cx - r * 0.5, shY, cx + side * r * 1.6, shY + r * 0.6]);
    armLine([cx + r * 0.5, shY, cx - side * r * 0.9, shY + r * 0.1]);
  } else if (poseId === "contain") {
    armLine([cx - r * 0.5, shY, cx - r * 0.95, hipY + r * 0.55]);
    armLine([cx + r * 0.5, shY, cx + r * 0.95, hipY + r * 0.55]);
  } else if (poseId === "settle") {
    // Both arms out level for balance, forearms angled back in over the ball.
    armLine([cx - r * 0.5, shY, cx - r * 1.35, shY - r * 0.05, cx - r * 1.05, shY + r * 0.5]);
    armLine([cx + r * 0.5, shY, cx + r * 1.35, shY - r * 0.05, cx + r * 1.05, shY + r * 0.5]);
  } else if (poseId === "run_telegraph") {
    // Hard arm drive: the leading arm punches up and across, the trailing arm is thrown all the way back.
    const side = fx >= 0 ? 1 : -1;
    armLine([cx - r * 0.5, shY, cx + side * r * 0.9, shY - r * 0.3]);
    armLine([cx + r * 0.5, shY, cx - side * r * 1.0, hipY + r * 0.1]);
  } else if (poseId === "kick_follow") {
    const side = fx >= 0 ? 1 : -1;
    armLine([cx - r * 0.5, shY, cx - side * r * 1.15, shY + r * 0.08]);
    armLine([cx + r * 0.5, shY, cx + side * r * 0.55, hipY + r * 0.28]);
  } else if (poseId === "stumble") {
    armLine([cx - r * 0.5, shY, cx - r * 1.3, shY - r * 0.5]);
    armLine([cx + r * 0.5, shY, cx + r * 1.0, shY - r * 0.75]);
  } else if (poseId === "fatigue") {
    armLine([cx - r * 0.55, shY, cx - r * 0.66, hipY + r * 0.6]);
    armLine([cx + r * 0.55, shY, cx + r * 0.64, hipY + r * 0.6]);
  } else if (aerialStyle === "chest_control") {
    armLine([cx - r * 0.5, shY, cx - r * (0.55 + 0.55 * aerial), shY + r * 0.2]);
    armLine([cx + r * 0.5, shY, cx + r * (0.55 + 0.55 * aerial), shY + r * 0.2]);
  } else {
    armLine([cx - r * 0.5, shY, cx - r * 0.55 - swing * stride * 0.6, hipY + r * 0.2]);
    armLine([cx + r * 0.5, shY, cx + r * 0.55 + swing * stride * 0.6, hipY + r * 0.2]);
  }

  // Torso (capsule: rounded rect from hips to shoulders).
  const tw = r * sil.torso_scale;
  const torsoAlpha = mul(alphaMul, opts.is_keeper ? 0.7 : 1);
  if (shape === "angular") {
    dl.polygon("fill", [cx - tw * 0.62, shY, cx + tw * 0.62, shY, cx + tw * 0.38, hipY, cx - tw * 0.38, hipY], color, { alpha: torsoAlpha }, t);
  } else {
    const roundness = shape === "broad" ? r * 0.18 : r * 0.45;
    dl.rect("fill", cx - tw / 2, shY, tw, hipY - shY, color, { alpha: torsoAlpha, rx: roundness, ry: roundness }, t);
  }
  // Team joint band across the chest.
  const bandAlpha = mul(alphaMul, 0.9);
  const bandLineWidth = Math.max(1, r * 0.18);
  const bandY = shY + (hipY - shY) * 0.4;
  if (opts.team === "away") {
    dl.line([cx - tw / 2, bandY, cx - tw * 0.12, bandY], accent, { alpha: bandAlpha, lineWidth: bandLineWidth }, t);
    dl.line([cx + tw * 0.12, bandY, cx + tw / 2, bandY], accent, { alpha: bandAlpha, lineWidth: bandLineWidth }, t);
  } else {
    dl.line([cx - tw / 2, bandY, cx + tw / 2, bandY], accent, { alpha: bandAlpha, lineWidth: bandLineWidth }, t);
  }

  // Helmet + visor. The visor sits on the side the player aims toward (its
  // ground-plane x), giving a readable facing cue without rotating the body.
  const hr = r * 0.62;
  const headAlpha = mul(alphaMul, 1);
  if (shape === "broad") {
    dl.rect("fill", cx - hr * 1.15, headY - hr * 0.65, hr * 2.3, hr * 1.3, color, { alpha: headAlpha, rx: hr * 0.25, ry: hr * 0.25 }, t);
  } else if (shape === "angular") {
    dl.polygon("fill", [cx, headY - hr * 1.35, cx + hr * 0.9, headY + hr * 0.7, cx, headY + hr, cx - hr * 0.9, headY + hr * 0.7], color, { alpha: headAlpha }, t);
  } else if (shape === "cluster") {
    dl.circle("fill", cx - hr * 0.7, headY + hr * 0.2, hr * 0.68, color, { alpha: headAlpha }, t);
    dl.circle("fill", cx + hr * 0.7, headY + hr * 0.2, hr * 0.68, color, { alpha: headAlpha }, t);
    dl.circle("fill", cx, headY - hr * 0.55, hr * 0.72, color, { alpha: headAlpha }, t);
  } else {
    dl.circle("fill", cx, headY, hr, color, { alpha: headAlpha }, t);
  }
  const visorAlpha = mul(alphaMul, 0.95);
  if (shape === "round") {
    dl.arc("fill", cx, headY, hr * 0.82, (-40 * Math.PI) / 180 + fx * 0.6, (90 * Math.PI) / 180 + fx * 0.6, accent, { alpha: visorAlpha }, t);
  } else if (shape === "cluster") {
    dl.circle("fill", cx + fx * hr * 0.3, headY, hr * 0.22, accent, { alpha: visorAlpha }, t);
  } else {
    dl.line([cx - hr * 0.45, headY + fx * hr * 0.14, cx + hr * 0.45, headY + fx * hr * 0.14], accent, { alpha: visorAlpha }, t);
  }

  drawEquipment(dl, t, alphaMul, cx, shY, hipY, r, opts);

  // Keeper handling stays inside the figure transform so a dive-catch
  // carries its gloves and ball with the body instead of leaving them at
  // the feet.
  if (opts.holding === true) {
    const gathering = (opts.grab ?? 0) > 0;
    const hy = gathering ? gy - r * 1.6 : gy - r * 2.1;
    const hx = cx + fx * r * 0.35;
    const glow = lighten(color, 0.55);
    dl.line([cx - r * 0.5, shY, hx - r * 0.35, hy], glow, { alpha: mul(alphaMul, 0.95), lineWidth: Math.max(1.5, r * 0.26) }, t);
    dl.line([cx + r * 0.5, shY, hx + r * 0.35, hy], glow, { alpha: mul(alphaMul, 0.95), lineWidth: Math.max(1.5, r * 0.26) }, t);
    dl.circle("fill", hx, hy, r * 0.5, [1, 0.95, 0.7], { alpha: alphaMul }, t);
  } else if ((opts.throw ?? 0) > 0) {
    const throwAmount = opts.throw ?? 0;
    const hx = cx + fx * r * (0.6 + throwAmount * 0.8);
    const glow = lighten(color, 0.55);
    dl.line([cx - r * 0.5, shY, hx, gy - r * 1.9], glow, { alpha: mul(alphaMul, 0.9), lineWidth: Math.max(1.5, r * 0.26) }, t);
    dl.line([cx + r * 0.5, shY, hx, gy - r * 1.9], glow, { alpha: mul(alphaMul, 0.9), lineWidth: Math.max(1.5, r * 0.26) }, t);
  }
}

/**
 * Draws one player.
 * @param sx screen x of the ground point (feet)
 * @param gy screen y of the ground point (feet)
 * @param r projected body radius (px)
 * @param v nil = idle fallback
 */
export function playerDrawCommands(sx: number, gy: number, r: number, color: RGB, v: PlayerView | undefined, opts: PlayerRenderOptions): DrawCommand[] {
  const dl = new DrawList();
  const poseId = opts.pose?.id;

  // Ground shadow (kept here so it tracks the figure).
  dl.ellipse("fill", sx, gy, r * 1.15, r * 0.5, [0, 0, 0], { alpha: 0.35 });

  // Selection is geometry-first: two rings and a downward chevron remain
  // readable in grayscale, during aerial poses, and during keeper dives.
  if (opts.controlled) {
    dl.ellipse("line", sx, gy, r * 1.25, r * 0.6, [1, 1, 1], { alpha: 0.92, lineWidth: Math.max(1, r * 0.12) });
    dl.ellipse("line", sx, gy, r * 1.48, r * 0.72, [1, 1, 1], { alpha: 0.92, lineWidth: Math.max(1, r * 0.12) });
    dl.polygon("fill", [sx, gy - r * 3.75, sx - r * 0.42, gy - r * 4.25, sx + r * 0.42, gy - r * 4.25], [1, 1, 1], { alpha: 0.92 });
  }

  // Keeper saves reuse the same body under bounded transforms. Spread stays
  // compact, central corrects a short distance, stretch holds the
  // full-lunge silhouette, and a one-shot tip reaches just beyond it.
  const keeperSavePose =
    poseId === "keeper_spread" || poseId === "keeper_central" || poseId === "keeper_stretch" || poseId === "keeper_tip" || poseId === "keeper_dive";
  if ((keeperSavePose || (poseId === undefined && (opts.dive ?? 0) > 0)) && opts.dive_dir !== undefined) {
    const d = opts.dive_dir;
    const axis = Math.abs(d.x) > Math.abs(d.y) ? d.x : d.y;
    const sign = axis >= 0 ? 1 : -1;
    let amount = poseId === "keeper_tip" ? 1 : clamp(opts.dive ?? 0, 0, 1);
    let angleDegrees = 72;
    let travel = 1.6;
    if (poseId === "keeper_spread") {
      angleDegrees = 28;
      travel = 0.65;
    } else if (poseId === "keeper_central") {
      angleDegrees = 48;
      travel = 0.95;
    } else if (poseId === "keeper_stretch") {
      angleDegrees = 78;
      travel = 1.9;
      amount = Math.max(amount, 0.82);
    } else if (poseId === "keeper_tip") {
      angleDegrees = 84;
      travel = 2.2;
    }
    const angle = sign * ((angleDegrees * Math.PI) / 180) * amount;
    const t: PivotTransform = { pivotX: sx, pivotY: gy, angle, offsetX: sign * r * travel * amount, offsetY: 0 };
    figure(dl, t, 1, sx, gy, r, color, v, opts);
    // Facing tick still helps read the dive direction.
    return dl.commands;
  }

  // Aerial actions use the ground point for sorting/shadow but lift the
  // billboard. A bicycle rotates the whole figure into a readable overhead
  // silhouette; other styles pose individual limbs in figure().
  if ((poseId === "aerial_bicycle" || poseId === "aerial_action" || (poseId === undefined && (opts.aerial ?? 0) > 0)) && opts.aerial_style !== undefined) {
    const amount = clamp(opts.aerial ?? 0, 0, 1);
    const lift = r * (0.35 + 1.65 * (opts.aerial_jump ?? 0)) * amount;
    if (opts.aerial_style === "bicycle") {
      const fx = opts.facing?.x ?? 1;
      const sign = fx >= 0 ? -1 : 1;
      const t: PivotTransform = {
        pivotX: sx,
        pivotY: gy,
        angle: sign * ((78 * Math.PI) / 180) * amount,
        offsetX: 0,
        offsetY: -lift - r * 0.7,
      };
      figure(dl, t, 1, sx, gy, r, color, v, opts);
    } else {
      figure(dl, undefined, 1, sx, gy - lift, r, color, v, opts);
    }
    return dl.commands;
  }

  if (poseId === "combat_knockback") {
    const t: PivotTransform = { pivotX: sx, pivotY: gy, angle: (68 * Math.PI) / 180, offsetX: 0, offsetY: -r * 0.45 };
    figure(dl, t, 1, sx, gy, r, color, v, opts);
    return dl.commands;
  } else if (poseId === "combat_stagger") {
    const t: PivotTransform = { pivotX: sx, pivotY: gy, angle: (8 * Math.PI) / 180, offsetX: 0, offsetY: r * 0.28 };
    figure(dl, t, 1, sx, gy, r, color, v, opts);
    return dl.commands;
  }

  // A failed challenge tips the figure away from the direction it committed
  // to. Steeper than a combat stagger and pivoted off the trailing heel, so
  // the two never read as the same recovery.
  if (poseId === "stumble") {
    const fx = opts.facing?.x ?? 1;
    const sign = fx >= 0 ? 1 : -1;
    const t: PivotTransform = {
      pivotX: sx,
      pivotY: gy,
      angle: -sign * ((24 * Math.PI) / 180),
      offsetX: -sign * r * 0.35,
      offsetY: r * 0.12,
    };
    figure(dl, t, 1, sx, gy, r, color, v, opts);
    return dl.commands;
  }

  if (poseId === "keeper_get_up") {
    const d = opts.dive_dir;
    const axis = d !== undefined ? (Math.abs(d.x) > Math.abs(d.y) ? d.x : d.y) : 1;
    const sign = axis >= 0 ? 1 : -1;
    const t: PivotTransform = { pivotX: sx, pivotY: gy, angle: sign * ((16 * Math.PI) / 180), offsetX: 0, offsetY: r * 0.18 };
    figure(dl, t, 1, sx, gy, r, color, v, opts);
    return dl.commands;
  }

  // Dash afterimage: faded copies trailing backward along the facing
  // direction (which equals the move direction during a dash). Drawn before
  // the figure so the solid body sits on top of its own smear.
  if (opts.dashing === true && opts.facing !== undefined) {
    const fx = opts.facing.x;
    const fy = opts.facing.y;
    for (let n = 1; n <= 2; n += 1) {
      const k = n * 0.6;
      const alphaMul = 0.24 / n;
      figure(dl, undefined, alphaMul, sx - fx * r * 1.1 * k, gy - fy * r * 0.55 * k, r, color, v, opts);
    }
  }

  figure(dl, undefined, 1, sx, gy, r, color, v, opts);

  // Ground-plane facing tick (kept from the old renderer as a clear aim cue).
  if (opts.facing !== undefined) {
    dl.line([sx, gy, sx + opts.facing.x * r * 1.1, gy + opts.facing.y * r * 0.6], [1, 1, 1], { alpha: 0.7, lineWidth: Math.max(1, r * 0.12) });
  }

  return dl.commands;
}

/** Impure: paints one player into `group`. Untested -- see draw2d.ts. */
export function drawPlayer(group: THREE.Group, sx: number, gy: number, r: number, color: RGB, v: PlayerView | undefined, opts: PlayerRenderOptions): void {
  paint(group, playerDrawCommands(sx, gy, r, color, v, opts));
}
