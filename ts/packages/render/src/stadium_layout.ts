// Single source of truth for the coliseum's ground-plan numbers.
//
// stadium_bowl.ts, stadium_crowd.ts, stadium_apron.ts, stadium_props.ts
// (braziers + floodlight pylons) and stadium_banners.ts all place things
// relative to the SAME three seating tiers and the SAME top-of-bowl arcade
// ring -- a brazier that doesn't sit on the tier1/tier2 parapet, or crowd
// jittered past the arcade's own radius, reads as a bug, not a style choice.
// Rather than let each module re-derive "where tier 2 ends" from its own
// copy of the tier-width constant, `computeStadiumLayout` computes every
// radius/height once from `field` and `quality`, and every other module
// takes the result as a parameter. Pure and side-effect-free, so it is cheap
// to call once per `Stadium` construction and share by reference.

import type { RenderFrameField } from "./pitch.ts";

export type StadiumQuality = "high" | "low";

export interface StadiumLayout {
  readonly cx: number;
  readonly cz: number;

  // Seating bowl: three stacked tiers, each an additive (radial width,
  // height) step out from the previous one -- see stadium_geo.ts's
  // `buildRevolvedRing` for how a tier's own stair profile is generated.
  readonly bowlInnerRx: number;
  readonly bowlInnerRz: number;
  readonly tierWidth: number;
  readonly tierHeight: number;
  readonly tierSteps: number;
  readonly tierSegments: number;
  readonly parapetDepth: number;
  readonly parapetHeight: number;
  readonly tierCount: number;
  /** Inner-edge radii, one pair per tier, chained (`tier[i+1]` starts where `tier[i]` ends). */
  readonly tierInnerRx: readonly number[];
  readonly tierInnerRz: readonly number[];
  readonly tierBaseY: readonly number[];
  /** Radii/height at the very top of the bowl (top of the last tier). */
  readonly bowlOuterRx: number;
  readonly bowlOuterRz: number;
  readonly bowlTopY: number;

  // Top-rim arcade: one instanced arch every ~(360/count) degrees, plus a
  // contiguous ruined span.
  readonly arcadeRx: number;
  readonly arcadeRz: number;
  readonly arcadeHeight: number;
  readonly arcadeCount: number;
  readonly ruinedCount: number;
  readonly ruinedStartIndex: number;

  // Dark ambulatory wall, a touch further out and a touch lower than the
  // arcade so the arch openings read as looking THROUGH to something behind.
  readonly wallRx: number;
  readonly wallRz: number;
  readonly wallBaseY: number;
  readonly wallHeight: number;

  // Parapet ring between tier 1 and tier 2 -- brazier placement.
  readonly brazierRx: number;
  readonly brazierRz: number;
  readonly brazierY: number;
  readonly brazierCount: number;

  // Four leaning floodlight pylons, one per ellipse diagonal.
  readonly pylonRx: number;
  readonly pylonRz: number;
  readonly pylonCount: number;

  // Banners hang from the inside of the arcade ring.
  readonly bannerRx: number;
  readonly bannerRz: number;
  readonly bannerTopY: number;
  readonly bannerHeight: number;
  readonly bannerCount: number;
  readonly gateBannerCount: number;

  // Ground apron: a flat disk from the pitch out to just inside the bowl.
  readonly apronOuterRx: number;
  readonly apronOuterRz: number;
  readonly apronY: number;
  /** Four tunnel gate structures, aligned with the pitch's two ends and two sides. */
  readonly gateAngles: readonly number[];
  readonly gateWidth: number;
  readonly gateHeight: number;

  readonly skyRadius: number;
}

export function computeStadiumLayout(
  field: RenderFrameField,
  quality: StadiumQuality,
): StadiumLayout {
  const cx = field.w / 2;
  const cz = field.h / 2;

  const bowlInnerRx = field.w / 2 + 190;
  const bowlInnerRz = field.h / 2 + 150;
  const tierWidth = 130;
  // Lowered from 58 (creative brief item 2, "show the sky"): the CROWD
  // (seated on top of each tier, stadium_crowd.ts's own +4.5-and-up offset)
  // was the actual top-of-frame blocker, not the arcade -- lowering only the
  // arcade rim (see `arcadeHeight` below) left the seating bowl itself still
  // tall enough to fill the broadcast camera's frame edge to edge with zero
  // sky gap. A ~24% cut here (58 -> 44, compounding across all 3 tiers)
  // is what actually clears room above the bowl.
  const tierHeight = 36;
  const tierCount = 3;
  const tierSteps = quality === "high" ? 6 : 4;
  const tierSegments = quality === "high" ? 96 : 48;
  const parapetDepth = 22;
  const parapetHeight = 10;

  const tierInnerRx: number[] = [];
  const tierInnerRz: number[] = [];
  const tierBaseY: number[] = [];
  for (let i = 0; i < tierCount; i += 1) {
    tierInnerRx.push(bowlInnerRx + i * tierWidth);
    tierInnerRz.push(bowlInnerRz + i * tierWidth);
    tierBaseY.push(i * tierHeight);
  }
  const bowlOuterRx = bowlInnerRx + tierCount * tierWidth;
  const bowlOuterRz = bowlInnerRz + tierCount * tierWidth;
  const bowlTopY = tierCount * tierHeight;

  const arcadeRx = bowlOuterRx + 15;
  const arcadeRz = bowlOuterRz + 15;
  // Lowered from 72 (creative brief item 2, "show the sky"), on top of the
  // `tierHeight` cut above.
  const arcadeHeight = 44;
  const arcadeCount = quality === "high" ? 56 : 28;
  const ruinedCount = Math.max(2, Math.round(arcadeCount * 0.15));
  const ruinedStartIndex = 0;

  const wallRx = arcadeRx + 22;
  const wallRz = arcadeRz + 22;
  const wallBaseY = bowlTopY - 10;
  const wallHeight = arcadeHeight + 10;

  // Moved from the tier1/tier2 parapet (creative brief item 5): from the
  // broadcast camera's own steep, pulled-back angle (see camera.ts's
  // PERSPECTIVE retune), that parapet position -- and the brief's own
  // fallback of the TOP rim next to the arcade -- both sit almost entirely
  // OUTSIDE the camera's frustum; a live scan projecting all 12 instances'
  // actual world matrices confirmed only 1-2 of 12 ever land inside a
  // comfortable on-screen margin out there, vs. 7-9 of 12 right at the
  // bowl's OWN inner edge (just outside `bowlInnerRx`/`bowlInnerRz`, i.e.
  // the innermost tier's own front wall, elevated clear of the apron/pitch
  // sightline). Braziers lining the pitch-side wall of the lowest tier is
  // exactly as period-appropriate as the parapet position, and is what the
  // broadcast camera can actually see.
  const brazierRx = bowlInnerRx - 10;
  const brazierRz = bowlInnerRz - 10;
  const brazierY = bowlTopY - 8;
  // Creative brief item 5 ("braziers that read"): bumped from 10/8 so the
  // ring of fire reads as a continuous feature at broadcast distance rather
  // than a handful of scattered points.
  const brazierCount = quality === "high" ? 12 : 9;

  const pylonRx = arcadeRx + 150;
  const pylonRz = arcadeRz + 150;
  const pylonCount = 4;

  const bannerRx = arcadeRx - 30;
  const bannerRz = arcadeRz - 30;
  const bannerTopY = bowlTopY + arcadeHeight * 0.55;
  const bannerHeight = 60;
  const bannerCount = 20;
  const gateBannerCount = 4;

  const apronOuterRx = bowlInnerRx - 6;
  const apronOuterRz = bowlInnerRz - 6;
  const apronY = 0;
  const gateAngles = [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2];
  const gateWidth = 90;
  const gateHeight = 50;

  const skyRadius = 5000;

  return {
    cx,
    cz,
    bowlInnerRx,
    bowlInnerRz,
    tierWidth,
    tierHeight,
    tierSteps,
    tierSegments,
    parapetDepth,
    parapetHeight,
    tierCount,
    tierInnerRx,
    tierInnerRz,
    tierBaseY,
    bowlOuterRx,
    bowlOuterRz,
    bowlTopY,
    arcadeRx,
    arcadeRz,
    arcadeHeight,
    arcadeCount,
    ruinedCount,
    ruinedStartIndex,
    wallRx,
    wallRz,
    wallBaseY,
    wallHeight,
    brazierRx,
    brazierRz,
    brazierY,
    brazierCount,
    pylonRx,
    pylonRz,
    pylonCount,
    bannerRx,
    bannerRz,
    bannerTopY,
    bannerHeight,
    bannerCount,
    gateBannerCount,
    apronOuterRx,
    apronOuterRz,
    apronY,
    gateAngles,
    gateWidth,
    gateHeight,
    skyRadius,
  };
}
