// The bowl: three stacked stone seating tiers, a dark ambulatory wall behind
// the top rim, and an instanced arcade of arches around that rim -- with a
// contiguous ~15% span swapped for a shorter, broken "ruined" variant so the
// silhouette reads as an ancient monument rather than a modern stadium (see
// stadium.ts's file header, creative brief item 1).
//
// Draw calls: 3 (one merged mesh per tier) + 1 (ambulatory wall) +
// 2 (one InstancedMesh for the intact arches, one for the ruined span) +
// 2 (fascia band, merged across all 3 tiers; fascia trim, ditto) = 8.

import * as THREE from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import type { ArenaColors } from "./arena.ts";
import { buildRevolvedRing, ellipsePoint, ellipseYawFacingCenter } from "./stadium_geo.ts";
import type { StadiumLayout, StadiumQuality } from "./stadium_layout.ts";
import { prngRange, type Prng } from "./stadium_prng.ts";

const COLUMN_RADIUS = 4.5;
const PLINTH_SIZE = 12;

// One intact arch: two columns, a semicircular torus arch bridging their
// tops (a torus's main circle lies in its local XY plane by default, so an
// `arc: Math.PI` slice from angle 0 gives exactly the upper semicircle from
// one column to the other -- no extra rotation needed), and two plinths.
function buildArchGeometry(columnHeight: number): THREE.BufferGeometry {
  const archRadius = columnHeight * 0.55;
  const column = new THREE.CylinderGeometry(COLUMN_RADIUS, COLUMN_RADIUS * 1.15, columnHeight, 8);
  const columnL = column.clone().translate(-archRadius, columnHeight / 2, 0);
  const columnR = column.clone().translate(archRadius, columnHeight / 2, 0);
  column.dispose();
  const arch = new THREE.TorusGeometry(archRadius, COLUMN_RADIUS * 0.9, 6, 24, Math.PI).translate(0, columnHeight, 0);
  const plinth = new THREE.BoxGeometry(PLINTH_SIZE, columnHeight * 0.08, PLINTH_SIZE);
  const plinthL = plinth.clone().translate(-archRadius, columnHeight * 0.04, 0);
  const plinthR = plinth.clone().translate(archRadius, columnHeight * 0.04, 0);
  plinth.dispose();

  const merged = mergeGeometries([columnL, columnR, arch, plinthL, plinthR], false);
  columnL.dispose();
  columnR.dispose();
  arch.dispose();
  plinthL.dispose();
  plinthR.dispose();
  return merged;
}

// A ruined slot: one broken, shorter column and a low rubble block where its
// partner and the arch used to be -- the silhouette gap this creates against
// the intact arches on either side is what reads as "ancient ruin".
function buildRuinedArchGeometry(columnHeight: number): THREE.BufferGeometry {
  const brokenHeight = columnHeight * 0.5;
  const archRadius = columnHeight * 0.55;
  const column = new THREE.CylinderGeometry(COLUMN_RADIUS * 0.85, COLUMN_RADIUS * 1.1, brokenHeight, 8).translate(
    -archRadius * 0.4,
    brokenHeight / 2,
    0,
  );
  const rubble = new THREE.BoxGeometry(PLINTH_SIZE * 1.6, columnHeight * 0.12, PLINTH_SIZE * 1.3).translate(
    archRadius * 0.5,
    columnHeight * 0.06,
    0,
  );
  const merged = mergeGeometries([column, rubble], false);
  column.dispose();
  rubble.dispose();
  return merged;
}

function placeInstance(mesh: THREE.InstancedMesh, index: number, x: number, y: number, z: number, yaw: number, scale: number): void {
  const position = new THREE.Vector3(x, y, z);
  const quaternion = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw);
  const scaleVec = new THREE.Vector3(scale, scale, scale);
  mesh.setMatrixAt(index, new THREE.Matrix4().compose(position, quaternion, scaleVec));
}

function buildArcade(layout: StadiumLayout, rng: Prng): THREE.Group {
  const group = new THREE.Group();
  group.name = "bowl_arcade";

  const material = new THREE.MeshStandardMaterial({ color: 0x8a7a63, roughness: 0.92, metalness: 0.02 });
  const intactGeometry = buildArchGeometry(layout.arcadeHeight);
  const ruinedGeometry = buildRuinedArchGeometry(layout.arcadeHeight);

  const ruinedEnd = layout.ruinedStartIndex + layout.ruinedCount;
  const intactCount = layout.arcadeCount - layout.ruinedCount;
  const intact = new THREE.InstancedMesh(intactGeometry, material, Math.max(1, intactCount));
  const ruined = new THREE.InstancedMesh(ruinedGeometry, material, Math.max(1, layout.ruinedCount));

  let intactIndex = 0;
  let ruinedIndex = 0;
  for (let i = 0; i < layout.arcadeCount; i += 1) {
    const angle = (i / layout.arcadeCount) * Math.PI * 2;
    const [x, z] = ellipsePoint(layout.cx, layout.cz, layout.arcadeRx, layout.arcadeRz, angle);
    const yaw = ellipseYawFacingCenter(angle, layout.arcadeRx, layout.arcadeRz) + prngRange(rng, -0.02, 0.02);
    const scale = prngRange(rng, 0.94, 1.06);
    const isRuined = i >= layout.ruinedStartIndex && i < ruinedEnd;
    if (isRuined) {
      placeInstance(ruined, ruinedIndex, x, layout.bowlTopY, z, yaw, scale);
      ruinedIndex += 1;
    } else {
      placeInstance(intact, intactIndex, x, layout.bowlTopY, z, yaw, scale);
      intactIndex += 1;
    }
  }
  intact.instanceMatrix.needsUpdate = true;
  ruined.instanceMatrix.needsUpdate = true;
  intact.count = intactIndex;
  ruined.count = ruinedIndex;

  group.add(intact, ruined);
  return group;
}

// Fascia band (dark teal, ~#1c2b33) and its amber emissive trim line, both
// sitting directly in front of each tier's own parapet riser -- see
// `buildFasciaBands`'s own header for why these are a SEPARATE pair of
// meshes rather than a per-vertex tint of the riser already baked into each
// tier's own geometry.
const FASCIA_COLOR: readonly [number, number, number] = [0.11, 0.169, 0.2];
const FASCIA_TRIM_COLOR: readonly [number, number, number] = [1.0, 0.66, 0.24];
// Radial inset (world units): pulls the fascia/trim geometry a hair closer
// to the pitch than the tier riser they overlay sits at, so the overlay
// reads as proud of the stone instead of z-fighting with it (both meshes
// would otherwise share the exact same radius at the exact same height).
const FASCIA_INSET = 0.25;
const TRIM_INSET = 0.45;
const TRIM_THICKNESS = 1.4;

// Twilight mood, creative brief item 1: a dark teal band across every tier's
// parapet face, capped with a thin amber emissive line -- the two accent
// colours (cyan-teal stone, amber tech) the rest of the coliseum's
// holographic dressing (braziers, banners, apron seams) already carries,
// applied to the one surface (the parapet riser) that faces the crowd's own
// eye line and therefore reads first. Built as TWO merged meshes (one band
// per tier, one trim line per tier, each merged into a single draw call)
// rather than SIX separate per-tier meshes, to keep the bowl's draw-call
// count from tripling for a detail this thin.
function buildFasciaBands(layout: StadiumLayout, rng: Prng): THREE.Group {
  const group = new THREE.Group();
  group.name = "bowl_fascia";

  const bandGeometries: THREE.BufferGeometry[] = [];
  const trimGeometries: THREE.BufferGeometry[] = [];
  for (let i = 0; i < layout.tierCount; i += 1) {
    const rxInner = layout.tierInnerRx[i] ?? layout.bowlInnerRx;
    const rzInner = layout.tierInnerRz[i] ?? layout.bowlInnerRz;
    const baseY = layout.tierBaseY[i] ?? 0;
    bandGeometries.push(
      buildRevolvedRing({
        cx: layout.cx,
        cz: layout.cz,
        rxInner: rxInner - FASCIA_INSET,
        rzInner: rzInner - FASCIA_INSET,
        width: 0,
        height: layout.parapetHeight,
        baseY,
        steps: 1,
        segments: layout.tierSegments,
        baseColor: FASCIA_COLOR,
        tint: 0.02,
        rng,
      }),
    );
    trimGeometries.push(
      buildRevolvedRing({
        cx: layout.cx,
        cz: layout.cz,
        rxInner: rxInner - TRIM_INSET,
        rzInner: rzInner - TRIM_INSET,
        width: 0,
        height: TRIM_THICKNESS,
        baseY: baseY + layout.parapetHeight - TRIM_THICKNESS,
        steps: 1,
        segments: layout.tierSegments,
        baseColor: FASCIA_TRIM_COLOR,
        tint: 0,
        rng,
      }),
    );
  }

  const bandMerged = mergeGeometries(bandGeometries, false);
  for (const g of bandGeometries) {
    g.dispose();
  }
  const bandMaterial = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.55, metalness: 0.15, side: THREE.DoubleSide });
  const band = new THREE.Mesh(bandMerged, bandMaterial);
  band.name = "bowl_fascia_band";
  group.add(band);

  const trimMerged = mergeGeometries(trimGeometries, false);
  for (const g of trimGeometries) {
    g.dispose();
  }
  // Below the bloom threshold (0.55) per this module's own file-header rule:
  // a thin accent line, not a second light source.
  const trimColor = new THREE.Color(FASCIA_TRIM_COLOR[0], FASCIA_TRIM_COLOR[1], FASCIA_TRIM_COLOR[2]);
  const trimMaterial = new THREE.MeshStandardMaterial({
    color: trimColor,
    emissive: trimColor,
    emissiveIntensity: 0.3,
    roughness: 0.4,
    side: THREE.DoubleSide,
  });
  const trim = new THREE.Mesh(trimMerged, trimMaterial);
  trim.name = "bowl_fascia_trim";
  group.add(trim);

  return group;
}

/**
 * Builds the "bowl" sub-group: three seating tiers, the dark ambulatory wall
 * behind the top rim, and the instanced arcade (intact + ruined). Purely
 * static geometry -- nothing here needs a per-frame uniform, so unlike
 * stadium_crowd.ts/stadium_sky.ts this returns a bare `THREE.Group` rather
 * than a `{ group, timeUniforms }` pair; `stadium.ts` disposes it generically
 * by traversal, so no separate disposal bookkeeping is needed here either.
 */
export function buildBowl(layout: StadiumLayout, arena: ArenaColors, quality: StadiumQuality, rng: Prng): THREE.Group {
  // `quality` does not change the bowl's own instance/tier counts (only the
  // crowd and arcade angular resolution scale down, both already read from
  // `layout`, which is itself quality-derived) -- kept as a parameter for a
  // uniform call signature across stadium_*.ts's builder functions.
  void quality;
  const group = new THREE.Group();
  group.name = "bowl";

  const tierMaterial = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.9, metalness: 0.0, side: THREE.DoubleSide });
  // Twilight mood (creative brief item 1): dark warm basalt, not the washed-
  // out tan sandstone this bowl started with -- ~#5d5348 family, drifting a
  // touch cooler/paler toward the rim so the upper stands still read as
  // further from the pitch's own warm key light.
  const tierBaseColors: Array<readonly [number, number, number]> = [
    [0.33, 0.29, 0.25],
    [0.36, 0.32, 0.28],
    [0.39, 0.35, 0.32],
  ];
  for (let i = 0; i < layout.tierCount; i += 1) {
    const rxInner = layout.tierInnerRx[i] ?? layout.bowlInnerRx;
    const rzInner = layout.tierInnerRz[i] ?? layout.bowlInnerRz;
    const baseY = layout.tierBaseY[i] ?? 0;
    const baseColor = tierBaseColors[i] ?? tierBaseColors[0]!;
    const geometry = buildRevolvedRing({
      cx: layout.cx,
      cz: layout.cz,
      rxInner,
      rzInner,
      width: layout.tierWidth,
      height: layout.tierHeight,
      baseY,
      steps: layout.tierSteps,
      segments: layout.tierSegments,
      parapetDepth: layout.parapetDepth,
      parapetHeight: layout.parapetHeight,
      baseColor,
      tint: 0.035,
      rng,
    });
    const mesh = new THREE.Mesh(geometry, tierMaterial);
    mesh.name = `bowl_tier_${i}`;
    group.add(mesh);
  }

  // Ambulatory wall: a flat vertical ring (a degenerate `buildRevolvedRing`
  // with zero radial width) sitting a little further out and a little lower
  // than the arcade, so the arch openings read as looking through to
  // something behind them instead of straight into the sky.
  // A hint of `arena.rail_color` mixed into the near-black base -- the
  // ambulatory should read as dark depth, not a flat silhouette, and a faint
  // cyan cast ties it to the same holographic-tech accent the braziers,
  // pylons and pitch markings all share.
  const wallColor: readonly [number, number, number] = [
    0.02 + arena.rail_color[0] * 0.05,
    0.02 + arena.rail_color[1] * 0.05,
    0.035 + arena.rail_color[2] * 0.05,
  ];
  const wallGeometry = buildRevolvedRing({
    cx: layout.cx,
    cz: layout.cz,
    rxInner: layout.wallRx,
    rzInner: layout.wallRz,
    width: 0,
    height: layout.wallHeight,
    baseY: layout.wallBaseY,
    steps: 1,
    segments: layout.tierSegments,
    baseColor: wallColor,
    tint: 0,
    rng,
  });
  const wallMaterial = new THREE.MeshBasicMaterial({ vertexColors: true, side: THREE.DoubleSide });
  const wall = new THREE.Mesh(wallGeometry, wallMaterial);
  wall.name = "bowl_ambulatory_wall";
  group.add(wall);

  group.add(buildArcade(layout, rng));
  group.add(buildFasciaBands(layout, rng));

  return group;
}
