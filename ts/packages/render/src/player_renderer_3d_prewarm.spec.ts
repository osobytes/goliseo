// #447 follow-up: NO CHARACTER GEOMETRY IS BUILT INSIDE THE DRAW LOOP.
//
// WHY THIS IS A SEPARATE FILE from `player_renderer_3d.spec.ts`. Everything
// this suite asserts is about a process-global, never-cleared cache
// (`builtByVariant`, `teamGeometry`, `materialsByTeam` -- all documented as
// living for the process, matching the singleton they replaced). Vitest gives
// each spec FILE its own module registry but shares it across the tests
// inside one file, so a build-count assertion has to run in a file where
// nothing else has already warmed the cache. Sharing a file with the existing
// suite would make these pass or fail depending on test order, which is worse
// than a second file.
//
// WHY A COUNT AND NOT A CLOCK. The regression being guarded is a TIME cost --
// ~58 ms of geometry generation landing on the first drawn frame -- but a
// wall-clock bound is flaky on shared CI and says nothing about WHY it went
// slow. The count is deterministic and states the actual property: once a
// match's roster is known, drawing it builds nothing.
//
// WHY THE EXISTING BENCHMARK CANNOT SEE THIS. `benchmark.ts` discards 300
// warm-up frames (`warmup_frames`, 5 s at 60 Hz) before `drawSamples` and
// `over_16_67` start counting, and its own header states the assumption:
// "Shader compilation, mesh generation and the first GC cycle are startup
// costs ... measured separately". That was true when mesh generation was one
// sub-millisecond singleton build. It stopped being true when #447 made it
// scale with roster diversity, and the harness has no way to notice: every
// variant build completes and is thrown away inside the warm-up. In
// production there is no five-second grace period -- frame 1 is what the user
// sees when the pitch appears. This suite is the instrument that harness
// cannot be.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { characterMesh, prewarmCharacters, variantBuildCount } from "./player_renderer_3d.ts";
import type { PlayerRenderOptions } from "./player_render_options.ts";
import type { PlayerView } from "./view_state.ts";

const idleView: PlayerView = { px: 0, py: 0, speed: 0, phase: 0, gait: 0, lean: 0 };

/**
 * The ten players `gc-data`'s `nebula` and `orion` rosters field, in roster
 * order, as the renderer sees them off the wire. Mirrored from
 * `crates/gc-data/src/players.rs` and `teams.rs`; the presentation and
 * loadout ids are pinned against those tables by
 * `scripts/check_presentation_parity.mjs`, so a drift here is a gate failure
 * rather than a silently wrong fixture.
 *
 * Nine distinct `(theme, figure, loadout)` variants across the ten: `brakka`
 * and `drell` are the one shared pair.
 */
const STARTERS = {
  ids: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex", "gax_oru", "drell", "morv", "krag", "tox_vren"],
  presentation_ids: [
    "scifi_axi", // ozzo, keeper
    "medieval_rook_emberguard", // brakka
    "toy_moxie_modular", // veil_nyx
    "scifi_nova_quell", // rok_tann
    "medieval_bramble_quickstep", // zyro_vex
    "toy_tock", // gax_oru, keeper
    "medieval_rook_emberguard", // drell -- shares brakka's variant
    "scifi_axi", // morv
    "toy_moxie_modular", // krag
    "scifi_nova_quell", // tox_vren
  ],
  loadout_ids: [
    undefined, // ozzo carries nothing
    "loadout_emberguard_shield",
    "loadout_tournament_sword",
    "loadout_pulse_blaster",
    "loadout_spring_gloves",
    undefined, // gax_oru carries nothing
    "loadout_emberguard_shield",
    "loadout_vector_blade",
    "loadout_pulse_blaster",
    "loadout_spring_gloves",
  ],
  teams: ["home", "home", "home", "home", "home", "away", "away", "away", "away", "away"],
} as const;

function optionsFor(index: number): PlayerRenderOptions {
  const loadoutId = STARTERS.loadout_ids[index];
  const team = STARTERS.teams[index];
  return {
    is_keeper: index === 0 || index === 5,
    controlled: false,
    presentation_id: STARTERS.presentation_ids[index] ?? "",
    ...(loadoutId !== undefined ? { loadout_id: loadoutId } : {}),
    ...(team !== undefined ? { team } : {}),
  };
}

/**
 * What `pitch.draw`'s depth-sorted loop does to the renderer, minus the
 * three.js scene assembly it wraps around it: one `characterMesh` per
 * rostered player, synchronously, in one call stack. That call is
 * `pitch.ts:1159` verbatim in everything this suite measures.
 */
function drawEveryPlayer(): THREE.Object3D[] {
  const meshes: THREE.Object3D[] = [];
  for (let index = 0; index < STARTERS.ids.length; index += 1) {
    const mesh = characterMesh(STARTERS.ids[index] ?? "", idleView, optionsFor(index), 0);
    if (mesh === undefined) {
      throw new Error(`expected a mesh for roster slot ${String(index)}`);
    }
    meshes.push(mesh);
  }
  return meshes;
}

describe("character pre-warm (#447): a match's geometry is built before its first frame", () => {
  // ORDER MATTERS INSIDE THIS BLOCK and vitest runs `it`s in declaration
  // order within a file. Each test states which cache state it assumes.

  it("builds every distinct variant the roster asks for, and no more", () => {
    const before = variantBuildCount();
    const result = prewarmCharacters(STARTERS);

    // NON-VACUOUS, and this is the assertion the next test rests on: if the
    // pre-warm built nothing, "the draw loop builds nothing" would be true
    // and would mean nothing.
    expect(result.built, "the two fixture teams field nine distinct variants across ten players").toBe(9);
    expect(result.variants, "nine variants, but ten (variant, team) pairs -- brakka and drell share a variant across opposite teams").toBe(10);
    expect(result.pooled, "and one pooled mesh per named player, which is the OTHER thing the first frame used to allocate").toBe(10);
    expect(variantBuildCount() - before, "and the counter agrees with what the call reported").toBe(9);
  });

  it("is idempotent: warming the same roster again builds nothing", () => {
    const before = variantBuildCount();
    const result = prewarmCharacters(STARTERS);
    expect(result.built).toBe(0);
    expect(variantBuildCount()).toBe(before);
  });

  // THE PROPERTY. This is the whole point of the suite: with the roster
  // known, a full frame's worth of `characterMesh` calls builds no geometry
  // at all. Before the pre-warm existed this number was 9 -- every one of
  // them inside `pitch.draw`, on the frame the pitch appears.
  it("draws a full roster without building a single geometry", () => {
    const before = variantBuildCount();
    const meshes = drawEveryPlayer();
    // Non-vacuous in the other direction: the loop really did draw ten
    // players, so "no builds" is not "no work asked for".
    expect(meshes, "every rostered player was drawn").toHaveLength(10);
    for (const mesh of meshes) {
      expect(mesh).toBeInstanceOf(THREE.SkinnedMesh);
    }
    expect(variantBuildCount() - before, "a drawn frame must build no character geometry once the roster is known").toBe(0);
  });

  it("draws every subsequent frame without building either", () => {
    const before = variantBuildCount();
    drawEveryPlayer();
    drawEveryPlayer();
    drawEveryPlayer();
    expect(variantBuildCount() - before).toBe(0);
  });

  // THE MID-MATCH FIRST SIGHTING. `gc-data`'s `nebula` team authors a `squad`
  // wider than its starting roster, so a player whose variant nobody has seen
  // can appear after kickoff. The lazy path is the correctness backstop and
  // must still work -- it just costs a build, during live play, which is
  // exactly what the pre-warm exists to avoid.
  it("still builds a never-before-seen variant on demand, at the cost the pre-warm exists to avoid", () => {
    // `mika_olu`: toybox + foam champion, in `nebula`'s squad and not in its
    // starting roster, so this variant is absent from `STARTERS`.
    const sub: PlayerRenderOptions = {
      is_keeper: false,
      controlled: false,
      team: "home",
      presentation_id: "toy_tock",
      loadout_id: "loadout_foam_champion",
    };
    const before = variantBuildCount();
    expect(characterMesh("mika_olu", idleView, sub, 0)).toBeInstanceOf(THREE.SkinnedMesh);
    expect(variantBuildCount() - before, "the backstop works, and it costs a build inside the draw loop").toBe(1);

    // And having paid it once, it is not paid again -- so the cost is a
    // one-off stutter rather than a permanent one, and pre-warming the wider
    // squad would remove even that.
    const afterFirst = variantBuildCount();
    characterMesh("mika_olu", idleView, sub, 0);
    expect(variantBuildCount()).toBe(afterFirst);
  });

  // The same substitution, pre-warmed instead of discovered: warming a roster
  // that INCLUDES the incoming player costs the build up front and leaves the
  // draw loop at zero. This is what a future squad-aware pre-warm would buy,
  // and it is asserted rather than asserted-about so the mechanism is known
  // to support it.
  it("costs the draw loop nothing when the incoming player was warmed first", () => {
    const roster = {
      presentation_ids: ["scifi_nova_quell"],
      loadout_ids: ["loadout_foam_champion"],
      teams: ["away"] as const,
    };
    expect(prewarmCharacters(roster).built, "a genuinely new variant").toBe(1);
    const before = variantBuildCount();
    const mesh = characterMesh("late-sub", idleView, {
      is_keeper: false,
      controlled: false,
      team: "away",
      presentation_id: "scifi_nova_quell",
      loadout_id: "loadout_foam_champion",
    }, 0);
    expect(mesh).toBeInstanceOf(THREE.SkinnedMesh);
    expect(variantBuildCount() - before, "warmed in advance, the first sighting is free").toBe(0);
  });

  it("is a no-op for a roster that carries no presentation ids at all", () => {
    // The fake hosts in `@gc/screens`' own suites return `{}`, and
    // `MatchScreen` pre-warms unconditionally at construction, so this path
    // runs in a hundred tests that have no content and must not throw.
    const before = variantBuildCount();
    expect(prewarmCharacters({})).toEqual({ variants: 0, built: 0, pooled: 0 });
    expect(prewarmCharacters({ ids: [] })).toEqual({ variants: 0, built: 0, pooled: 0 });
    expect(variantBuildCount()).toBe(before);
  });

  it("fails at pre-warm time, not mid-match, on content the renderer does not know", () => {
    expect(() => prewarmCharacters({ presentation_ids: ["no_such_presentation"] })).toThrow(/no theme for presentation id/);
    expect(() => prewarmCharacters({ presentation_ids: ["scifi_axi"], loadout_ids: ["no_such_loadout"] })).toThrow(/no equipment for loadout id/);
  });

  // The silent-substitution guard, at the last layer before the geometry.
  // `decodeRoster` refuses an empty presentation id and `encode_roster`
  // asserts against one, but a hand-built frame reaches here having passed
  // neither -- and the old behaviour was to read `""` as "nothing was wired"
  // and hand back `themes.LIST[0]`, sword and shield included.
  it("refuses an empty presentation id rather than resolving it to the preview default", () => {
    expect(() =>
      characterMesh("broken", idleView, { is_keeper: true, controlled: false, presentation_id: "" }, 0),
    ).toThrow(/empty presentation_id/);
    expect(() => prewarmCharacters({ presentation_ids: [""] })).toThrow(/empty presentation_id/);
  });
});
