// The coliseum: GOLISEO's stadium module. An ancient Roman coliseum drifting
// in space, retrofitted with holographic-era tech -- sandstone-and-marble
// bones (the bowl, stadium_bowl.ts) with cyan/amber plasma technology
// grafted on (braziers, floodlights, banners, the pitch's own marking
// glow), under a twilight nebula sky, all under an `UnrealBloomPass`
// (bloom.ts, threshold 0.55) that this module deliberately courts: every
// emissive material here either sits comfortably under that threshold (the
// stone, the crowd, the apron's base tile) or is pushed intentionally over
// it (brazier cores, goal trim, lamp banks, the pitch markings) so the glow
// reads as art direction, not an accident.
//
// MODULE MAP. Each sub-piece of the brief lives in its own `stadium_*.ts`
// file, all orchestrated here:
//   - stadium_layout.ts   -- the one shared ground-plan (every radius/height
//                            every other module places things against).
//   - stadium_geo.ts       -- the shared revolved-ring geometry primitive.
//   - stadium_prng.ts      -- the seeded RNG (mulberry32) that keeps this
//                            whole module deterministic (see below).
//   - stadium_bowl.ts      -- the "bowl" sub-group: 3 seating tiers, the
//                            ambulatory wall, the instanced arcade.
//   - stadium_crowd.ts     -- the "crowd" sub-group: thousands of vertex-
//                            shader-animated spectator blobs.
//   - stadium_sky.ts       -- the "sky" sub-group: the nebula dome.
//   - stadium_props.ts     -- the "braziers" and "floodlights" sub-groups.
//   - stadium_banners.ts   -- the "banners" sub-group.
//   - stadium_apron.ts     -- the "apron" sub-group: ground disk + gates.
//   - stadium_pitch_surface.ts -- the "pitch_surface" sub-group: the hero floor.
//   - stadium_goals.ts     -- `buildGoal`, used twice for the "goals" sub-group.
//   - stadium_types.ts     -- the shared `NumberUniform` shape.
//
// DETERMINISM. Every builder above that needs placement/colour variety
// (crowd jitter, arcade tint, banner phase, brazier angle jitter) draws from
// ONE `mulberry32` stream seeded with a fixed literal (`STADIUM_SEED`),
// threaded through as an explicit `Prng` parameter -- never `Math.random()`.
// Two `new Stadium(options)` calls with the same options therefore produce
// byte-identical instance matrices (stadium.spec.ts asserts this directly),
// which matters for the same reason it matters everywhere else in this
// codebase per this task's brief: a headless test has no way to eyeball a
// frame, so bit-for-bit reproducibility IS the verification.
//
// PERF / DRAW-CALL INVENTORY (high quality; low quality only reduces
// instance/segment COUNTS, never the number of draw calls below):
//   bowl:          3 tiers + 1 ambulatory wall + 2 arcade (intact/ruined)  = 6
//   crowd:         2 InstancedMesh (near/far tiers)                        = 2  (1 in "low")
//   sky:           1 dome mesh                                             = 1
//   braziers:      1 bowls InstancedMesh + 1 flames InstancedMesh          = 2
//   floodlights:   1 pylon bodies + 1 lamp bank + 1 light cones            = 3
//   banners:       1 vertical InstancedMesh + 1 gate InstancedMesh         = 2
//   apron:         1 ground disk + 1 gate frames + 1 gate glow             = 3
//   pitch_surface: 1 hero-floor quad                                       = 1
//   goals:         (1 frame + 1 trim + 1 net) x 2 goals                    = 6
//   lighting:      0 (lights are not draw calls)
//   TOTAL (high):  26   (well under the ~40 budget; "low" drops to 25)
//
// Construction is plain object-graph assembly (BufferGeometry vertex math,
// ShaderMaterial source strings, InstancedMesh matrix writes) -- nothing
// here touches a `WebGLRenderer`, so it constructs and is fully inspectable
// under vitest's default "node" environment, no GL context or DOM required
// (mirrors draw2d.ts's own PURE-command / impure-interpreter split, taken
// one step further here since three.js objects themselves are the "pure"
// output -- no `paint()`-style separate interpreter is needed because
// nothing is drawn immediately, only constructed).

import * as THREE from "three";
import type { ArenaColors } from "./arena.ts";
import type { RGB } from "./draw2d.ts";
import type { RenderFrameField } from "./pitch.ts";
import { buildApron } from "./stadium_apron.ts";
import { buildBanners } from "./stadium_banners.ts";
import { buildBowl } from "./stadium_bowl.ts";
import { buildCrowd } from "./stadium_crowd.ts";
import { buildGoal } from "./stadium_goals.ts";
import { computeStadiumLayout, type StadiumQuality } from "./stadium_layout.ts";
import { buildPitchSurface } from "./stadium_pitch_surface.ts";
import { buildBraziers, buildFloodlights } from "./stadium_props.ts";
import { buildSky } from "./stadium_sky.ts";
import { mulberry32 } from "./stadium_prng.ts";
import type { NumberUniform } from "./stadium_types.ts";

export interface StadiumOptions {
  readonly field: RenderFrameField;
  readonly home_color: RGB;
  readonly away_color: RGB;
  readonly arena?: ArenaColors;
  readonly quality?: "high" | "low";
}

// Mirrors pitch.ts's own (unexported) `DEFAULT_ARENA`, which in turn mirrors
// Rust `crates/gc-data`'s only current entry (`helios_crown`) -- see that
// file's header for why this package cannot import the Rust-owned table
// directly.
const DEFAULT_ARENA: ArenaColors = {
  floor_color: [0.025, 0.16, 0.17],
  rail_color: [0.25, 0.88, 1.0],
  highlight_color: [1.0, 0.66, 0.24],
};

// Fixed, literal -- see file header's DETERMINISM note. Any 32-bit value
// works; this one has no significance beyond "not zero, not a round number
// someone might mistake for a placeholder".
const STADIUM_SEED = 0x9e_2f_71_c3;

function buildLighting(field: RenderFrameField): THREE.Group {
  const group = new THREE.Group();
  group.name = "lighting";
  const cx = field.w / 2;
  const cz = field.h / 2;

  // Twilight sky colour above, deep indigo-violet bounce below (creative
  // brief item 1: shadowed stone should fall toward blue-violet, not the
  // warm dust-brown this used to bounce) -- ambient fill so unlit-shader
  // surfaces (sky, apron, pitch) don't clash with lit ones (bowl stone, goal
  // metal, crowd) sitting right next to them. Dimmer than before (0.8 -> 0.55)
  // so the directional key/rim pair below reads as the dominant contrast,
  // not the flat ambient wash.
  const hemisphere = new THREE.HemisphereLight(new THREE.Color(0.24, 0.22, 0.42), new THREE.Color(0.08, 0.07, 0.17), 0.55);
  hemisphere.name = "hemisphere";
  group.add(hemisphere);

  const key = new THREE.DirectionalLight(new THREE.Color(1.0, 0.78, 0.48), 1.1);
  key.name = "key_amber";
  key.position.set(cx - field.w * 0.4, 520, cz - field.h * 0.55);
  key.target.position.set(cx, 0, cz);
  group.add(key, key.target);

  const rim = new THREE.DirectionalLight(new THREE.Color(0.42, 0.85, 1.0), 0.5);
  rim.name = "rim_cyan";
  rim.position.set(cx + field.w * 0.55, 380, cz + field.h * 0.65);
  rim.target.position.set(cx, 0, cz);
  group.add(rim, rim.target);

  return group;
}

// Generic disposal by traversal: every geometry/material (and any texture a
// material happens to hold) under `root` is released, regardless of which
// stadium_*.ts builder created it. This is deliberately simpler than
// draw2d.ts's `disposeObject` needing a `PaintOptions`-aware caller -- there
// is no repaint/rebuild cycle here (the stadium's object graph is built once
// and lives for the module's lifetime), so `Stadium.dispose()` only ever
// needs to run this once, mirroring the SAME `instanceof` dispatch draw2d.ts
// already uses. `InstancedMesh` additionally gets its own `.dispose()` call
// (that is separate from its geometry's) so the instance matrix/colour
// attributes it owns directly are released too, not only the shared
// geometry/material.
function disposeObjectTree(root: THREE.Object3D): void {
  root.traverse((obj) => {
    if (obj instanceof THREE.Mesh || obj instanceof THREE.Line || obj instanceof THREE.Points) {
      // See draw2d.ts's `disposeObject`: `instanceof` on three.js's generic
      // classes narrows their type arguments to `any`. Runtime check unchanged.
      const owner: THREE.Mesh | THREE.Line | THREE.Points = obj;
      owner.geometry.dispose();
      disposeMaterial(owner.material);
      if (obj instanceof THREE.InstancedMesh) {
        obj.dispose();
      }
    } else if (obj instanceof THREE.Sprite) {
      disposeMaterial(obj.material);
    }
  });
}

function disposeMaterial(material: THREE.Material | THREE.Material[]): void {
  const materials = Array.isArray(material) ? material : [material];
  for (const m of materials) {
    if ("map" in m && m.map instanceof THREE.Texture) {
      m.map.dispose();
    }
    m.dispose();
  }
}

/**
 * GOLISEO's coliseum stadium: an ancient-Roman bowl retrofitted with
 * holographic-era tech, built as a full 360-degree object graph (the camera
 * can move freely; nothing here is a screen-space backdrop). See this file's
 * header for the module map, the determinism guarantee and the draw-call
 * inventory.
 */
export class Stadium {
  readonly group: THREE.Group;
  private readonly timeUniforms: readonly NumberUniform[];
  private readonly excitementUniforms: readonly NumberUniform[];

  constructor(options: StadiumOptions) {
    const quality: StadiumQuality = options.quality ?? "high";
    const arena = options.arena ?? DEFAULT_ARENA;
    const field = options.field;
    const layout = computeStadiumLayout(field, quality);
    const rng = mulberry32(STADIUM_SEED);

    const group = new THREE.Group();
    group.name = "stadium";

    const timeUniforms: NumberUniform[] = [];
    const excitementUniforms: NumberUniform[] = [];

    const bowl = buildBowl(layout, arena, quality, rng);
    group.add(bowl);

    const crowd = buildCrowd(layout, options.home_color, options.away_color, quality, rng);
    group.add(crowd.group);
    timeUniforms.push(...crowd.timeUniforms);
    excitementUniforms.push(...crowd.excitementUniforms);

    const sky = buildSky(layout.skyRadius);
    group.add(sky.group);
    timeUniforms.push(...sky.timeUniforms);

    const braziers = buildBraziers(layout, rng);
    group.add(braziers.group);
    timeUniforms.push(...braziers.timeUniforms);

    const floodlights = buildFloodlights(layout);
    group.add(floodlights.group);

    const banners = buildBanners(layout, options.home_color, options.away_color, rng);
    group.add(banners.group);
    timeUniforms.push(...banners.timeUniforms);

    const apron = buildApron(layout, arena, rng);
    group.add(apron.group);
    timeUniforms.push(...apron.timeUniforms);

    const pitchSurface = buildPitchSurface(field, arena);
    group.add(pitchSurface.group);
    timeUniforms.push(...pitchSurface.timeUniforms);

    const goalsGroup = new THREE.Group();
    goalsGroup.name = "goals";
    // Mouth lines are the coordinate contract's OWN literals (world x = 0 for
    // the home end, x = field.w for the away end), not re-derived from the
    // goal rects -- see stadium_goals.ts's file header on why `buildGoal`
    // only needs `mouthLineX`, not a second explicit back-line parameter.
    const homeGoal = buildGoal(field.goal_home, field.crossbar_h, 0, options.home_color);
    homeGoal.userData["stadiumGoal"] = "home";
    goalsGroup.add(homeGoal);
    const awayGoal = buildGoal(field.goal_away, field.crossbar_h, field.w, options.away_color);
    awayGoal.userData["stadiumGoal"] = "away";
    goalsGroup.add(awayGoal);
    group.add(goalsGroup);

    group.add(buildLighting(field));

    // Exposes the SAME uniform objects `update`/`setExcitement` mutate, for
    // stadium.spec.ts. This is the only way a headless test can observe that
    // `update(now)` actually reaches the crowd/banner uTime uniforms:
    // crowd.ts's and banners.ts's `onBeforeCompile` never runs without a
    // real GL compile (see stadium_crowd.ts's file header), so those two
    // materials have no `.uniforms` map to traverse to at all -- only the
    // OBJECTS this constructor already holds a reference to (`timeUniforms`/
    // `excitementUniforms` below) prove the wiring. `userData` is ordinary
    // `THREE.Object3D` metadata, not a change to this class's stated public
    // shape (`group`/constructor/`update`/`setExcitement`/`dispose`).
    group.userData["stadiumTimeUniforms"] = timeUniforms;
    group.userData["stadiumExcitementUniforms"] = excitementUniforms;

    this.group = group;
    this.timeUniforms = timeUniforms;
    this.excitementUniforms = excitementUniforms;
  }

  /** Advances every animation uniform (sky drift, crowd bounce/wave, flame flicker, banner flutter, ...) to `now` (seconds). */
  update(now: number): void {
    for (const uniform of this.timeUniforms) {
      uniform.value = now;
    }
  }

  /** Crowd hype, `[0, 1]` (clamped): scales the crowd's bounce/wave amplitude. Used for goal celebrations. */
  setExcitement(level: number): void {
    const clamped = Math.min(1, Math.max(0, level));
    for (const uniform of this.excitementUniforms) {
      uniform.value = clamped;
    }
  }

  /** Disposes every geometry/material/texture this module created. See `disposeObjectTree`. */
  dispose(): void {
    disposeObjectTree(this.group);
  }
}
