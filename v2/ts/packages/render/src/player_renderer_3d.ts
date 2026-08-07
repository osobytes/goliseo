// Ported from game/render/player_renderer_3d.lua.
//
// Rigged 3D player renderer: a drop-in alternative to
// `player_renderer.playerDrawCommands`. `sx`, `sy` and `r` still come from
// `camera.ts` -- that projection stays the authority on where a player is
// and how large they appear. This module only decides *how* the player is
// drawn at that spot: a rigged, animated, depth-tested character instead of
// a billboard.
//
// PURE AND TESTED: pose selection (`clipFor`/`poseFor`) and the metres-per-
// world-unit conversion. These are content decisions (which clip plays for
// which pose id, how gait/stance/whole-body actions layer) with no GPU
// dependency once `love.timer.getTime()` is threaded through as an
// explicit `now` parameter instead of a global clock read.
//
// GPU-ADJACENT AND UNTESTED: building the `THREE.SkinnedMesh` (the
// "deliberately... deferred" step `rig3d/body.ts`'s and `rig3d/geometry.ts`'s
// own headers point at -- this is that later "rendering integration pass"),
// posing its `THREE.Bone`s each frame, and the per-character camera. See the
// scope note below on the character camera specifically.
//
// SINGLE-PASS COMPOSITING (`characterMesh`, near `available()` below). What
// pitch.ts actually calls today is NOT the per-character-camera path the
// scope note below describes -- that path (`draw`/`renderToSprite`) rendered
// each character through its OWN small camera into its OWN full-viewport
// off-screen target, one `renderer.render()` call per character per frame.
// `characterMesh` instead returns a posed, coloured, yawed `THREE.SkinnedMesh`
// -- pooled per `playerId`, since every character now coexists in ONE scene
// graph for ONE shared render instead of being rendered/composited one at a
// time -- with no camera or render target of its own at all; pitch.ts places
// it directly under the SAME shared camera/scene `SceneRoot` already renders
// everything else through (arena, HUD, the lot). `draw`/`renderToSprite` are
// kept, unchanged, for parity/diagnostics -- see their own doc comments.
//
// SCOPE NOTE on the character camera. The Lua original renders each rigged
// character through its OWN small orthographic camera
// (`rig3d/renderer.lua`'s `characterCamera`), positioned so the character
// lands at the billboard's exact screen coordinates -- a "3D insert into a
// 2D scene" trick, not a single whole-pitch 3D camera. `rig3d/renderer.lua`
// is itself mechanism (v2/README.md #7: "replace -- WebGLRenderer,
// MeshStandardMaterial") and was never ported by anyone in this package (it
// does not appear in the "already ported" rig3d list), so there is no Lua
// TypeScript-adjacent source to diff this against line by line. What is
// ported faithfully below is the CONTENT that came from player_renderer_3d.lua
// itself: `HEIGHT_IN_RADII`, `ELEVATION`, the metres-per-world-unit formula,
// and the pose-selection table. The camera placement math
// (`characterCameraParams`) is new code, written the way three.js expects
// (an `OrthographicCamera`'s position/target/frustum) rather than a
// reconstruction of unseen GLSL.
//
// Boundary note (v2/README.md rule 6.7): `match.PLAYER_RADIUS` is
// `sim/match.lua`'s (Rust `crates/gc-sim`). `DEFAULT_PLAYER_RADIUS` mirrors
// its current value (12) as an injectable default, the same pattern
// `pitch.ts`'s `DEFAULT_ARENA` uses for Rust-owned content.

import * as THREE from "three";
import type { Mat4 } from "@gc/core";
import type { PlayerView } from "./view_state.ts";
import { viewState } from "./view_state.ts";
import * as skeleton from "./rig3d/skeleton.ts";
import * as proportions from "./rig3d/proportions.ts";
import * as clips from "./rig3d/clips.ts";
import * as masks from "./rig3d/masks.ts";
import * as actionPose from "./rig3d/action_pose.ts";
import * as themes from "./rig3d/themes.ts";
import * as body from "./rig3d/body.ts";
import * as geometry from "./rig3d/geometry.ts";
import * as celShader from "./rig3d/cel_shader.ts";
import type { PlayerRenderOptions } from "./player_renderer.ts";

// Character height maps to roughly this many player-radii on screen. Tuned
// so a rigged player reads at the same visual weight as the billboard it
// replaces.
const HEIGHT_IN_RADII = 3.0;
// Match camera looks down at the pitch; this is the apparent elevation.
// Exported: pitch.ts's single-pass compositing (see that file's "ONE PASS,
// ONE DEPTH BUFFER" header section) needs this to compose the same tilt onto
// a character's own transform, now that there is no longer a separate
// per-character camera to hold it.
export const ELEVATION = (17 * Math.PI) / 180;

// LIGHTING. `rig3d/renderer.lua`'s hand-written GLSL (v2/README.md #7 marks
// the file "replace -- WebGLRenderer, MeshStandardMaterial", but that verdict
// covers the mechanism -- the depth pass, the draw call, the WebGL1 bone/
// palette packing -- not the shading itself; see `rig3d/cel_shader.ts`'s file
// header for the full argument) drove its toon shading off one directional
// key light, `light_dir = { -0.42, -0.78, -0.46 }` ("direction the light
// travels"), plus a soft "cool bounce light from below so shadowed sides do
// not go dead", three flat quantised bands, a view-dependent rim, and a
// metal-only hard specular.
//
// That shading is now ported: `rig3d/cel_shader.ts`'s `applyCombinedCelShading`
// splices it into `materialsForTeam`'s ONE `MeshStandardMaterial` via
// `onBeforeCompile`, reading `LIGHT_DIR` as a hardcoded uniform rather than
// any `THREE.Light` in the scene -- exactly like the Lua original, which had
// no scene-graph lights at all, only `renderer.beginPass`'s two hand-sent
// uniforms. `LIGHT_DIR` itself lives in `cel_shader.ts` (re-exported here) so
// the toon shading and this file's decorative light share one constant
// instead of two copies that could drift.
//
// ONE material, not three (draw-call fix, see `materialsForTeam` and
// `build()`'s own comments below): `rig3d/renderer.lua`'s shader always read
// the shading family off a per-vertex `VertexMaterial` attribute and branched
// on it inside ONE fragment shader -- "branching on a varying in the
// fragment stage is fine here; only dynamic *array indexing* is forbidden"
// (that file's own SHADER_SOURCE comment). This port used to reproduce the
// FAMILIES (plain/metal/emissive) but not that mechanism: three separate
// `MeshStandardMaterial`s, one `onBeforeCompile` variant each, selected at
// mesh-build time via `THREE.BufferGeometry` material groups -- which
// resurrected the "one draw per material" cost the Lua shader was written
// specifically to avoid, and worse, `body.ts`'s parts interleave families
// (a plain visor next to a metal band next to an emissive seam), so groups
// fired on every material *transition*, not once per family -- tens of
// draws per character instead of three. `build()` below now bakes the same
// per-vertex family float `rig3d/renderer.lua` used (`materialFamily`), and
// `applyCombinedCelShading` branches on it at runtime in one compiled
// program, matching the Lua mechanism exactly and collapsing the whole
// character back to ONE draw call.
//
// The two `THREE.Light` instances below therefore no longer drive the
// character's own shading -- `cel_shading`'s replaced fragment chunk never
// calls `RE_Direct` / `RE_IndirectDiffuse`, so every scene light is a no-op
// for a `SkinnedMesh` using one of `materialsForTeam`'s materials. They stay
// in place regardless: an earlier fix here was "the private module-level
// scene had no lights" (a real defect against the *previous*, un-toon-shaded
// `MeshStandardMaterial` state), and removing them is not part of this
// port -- see this file's own header note on not undoing that fix. They are
// harmless dead weight for the character mesh today, not a hazard.
const { LIGHT_DIR } = celShader;
const KEY_LIGHT_INTENSITY = 3.2;
// Vestigial alongside `keyLight` above -- see this comment block.
const SKY_COLOR = 0xaebfe0;
const GROUND_COLOR = 0x30251c;
const FILL_LIGHT_INTENSITY = 0.9;

// Mirrors `sim/match.lua`'s `PLAYER_RADIUS`. See file header.
export const DEFAULT_PLAYER_RADIUS = 12;

// The one rig contract every character uses (see rig3d/proportions.ts's own
// header: "there is one rig here"). Hoisted out of `build()` so `pooledCharacter`
// below can derive a fresh bone list for each player from the SAME contract
// `build()` used for the shared geometry, without a second source of truth.
const RIG_PROPORTIONS = proportions.RIG_MEDIUM;

/**
 * The one conversion between the pitch's world units and the rig's metres.
 *
 * Worth stating because it looks depth-dependent and is not. World-to-pixels
 * is the projection's `scale`, and pixels-to-metres is `ppm`, which is built
 * from `r = radius * scale`. The two `scale` terms cancel:
 *
 *     metres = world * height / (PLAYER_RADIUS * HEIGHT_IN_RADII * 2)
 *
 * so anything sized through here is the same size at the halfway line as it
 * is on the goal line.
 */
export function metresPerWorldUnit(height: number, playerRadius = DEFAULT_PLAYER_RADIUS): number {
  return height / (playerRadius * HEIGHT_IN_RADII * 2);
}

// Maps a presentation pose id onto the LIMB clips that exist today.
//
// Three mechanisms cover the pose contract between them, and which one owns
// an id is not arbitrary:
//   * rig3d/action_pose.ts owns the ids the body performs as a whole --
//     dives, aerials, knockback, stagger, stumble, get-up. Those are
//     continuous transforms driven by the simulation's own timers, not clips.
//   * The keeper's hands are driven off possession (`holding`/`throw`)
//     rather than a pose id.
//   * This table owns the rest: stances and gaits that are genuinely limb
//     animation.
const POSE_CLIP: Readonly<Record<string, string>> = {
  locomotion: "locomotion",
  contain: "locomotion",
  run_telegraph: "locomotion",
  fatigue: "idle",
  keeper_shuffle: "locomotion",
  settle: "locomotion",
  kick_follow: "locomotion",
  combat_guard: "guard",
  combat_windup: "guard",
  combat_active: "charge",
  combat_recovery: "guard",
  combat_aim: "guard",
};

export function clipFor(poseId?: string): string {
  return POSE_CLIP[poseId ?? ""] ?? "idle";
}

/** Resolves the pose for one player. `now`: seconds, replacing `love.timer.getTime()`. */
export function poseFor(view: PlayerView | undefined, opts: PlayerRenderOptions, now: number): actionPose.MutablePose {
  const speed = view?.speed ?? 0;
  const idle = clips.ORDER[0];
  const walk = clips.ORDER[1];
  const run = clips.RUN;
  if (idle === undefined || walk === undefined) {
    throw new Error("player_renderer_3d.ts: clips.ORDER is missing idle/walk");
  }

  // A run is not a fast walk, so the two are separate clips blended by speed
  // rather than one clip played quicker.
  const walkMix = Math.min(speed / viewState.WALK_SPEED, 1);
  const runMix = Math.max(0, Math.min((speed - viewState.WALK_SPEED) / (viewState.RUN_SPEED - viewState.WALK_SPEED), 1));

  // Both cycles are two steps with contacts at 0 and 0.5, so one normalised
  // phase drives both and they stay in step through the blend.
  const cycles = view?.gait ?? 0;

  let pose: actionPose.MutablePose = clips.layer(clips.sample(idle, now * 0.35), clips.sample(walk, cycles * walk.duration), masks.FULL_BODY, walkMix);
  if (runMix > 0) {
    pose = clips.layer(pose, clips.sample(run, cycles * run.duration), masks.FULL_BODY, runMix);
  }

  const selected = clipFor(opts.pose?.id);
  if (selected === "guard") {
    pose = clips.layer(pose, clips.sample(clips.GUARD_STANCE, now), masks.UPPER_BODY, 1);
  } else if (selected === "charge") {
    // The charge is a held pose, so it only needs a phase to breathe on;
    // tying it to the stride keeps the sway in step with the legs.
    pose = clips.layer(pose, clips.sample(clips.CHARGE, cycles * clips.CHARGE.duration), masks.UPPER_BODY, 1);
  } else if (selected === "swing") {
    const swingClip = clips.ORDER[2];
    if (swingClip !== undefined) {
      const t = (opts.windup ?? 0) > 0 ? 0.3 : 0.55;
      pose = clips.layer(pose, clips.sample(swingClip, t * swingClip.duration), masks.UPPER_BODY, 1);
    }
  }

  // The keeper's hands follow possession, not the pose id: arms wrapped
  // around a ball outrank whatever else the keeper is doing.
  const throwAmount = opts.throw ?? 0;
  if (throwAmount > 0) {
    // throw_timer counts DOWN, so 1 is the moment of commitment.
    const sling = clips.KEEPER_SLING;
    pose = clips.layer(pose, clips.sample(sling, (1 - Math.min(throwAmount, 1)) * sling.duration), masks.UPPER_BODY, 1);
  } else if (opts.holding === true) {
    pose = clips.layer(pose, clips.sample(clips.KEEPER_GATHER, now), masks.UPPER_BODY, 1);
  }

  // Whole-body actions last: they move the root, so they ride on top of
  // whatever gait and stance resolved instead of competing with them.
  return actionPose.apply(pose, opts);
}

// ---------------------------------------------------------------------------
// GPU-adjacent: mesh construction, skeleton posing, per-character camera.
// Untested in this milestone (no WebGL context, v2/README.md #1).
// ---------------------------------------------------------------------------

interface BuiltCharacter {
  readonly rig: skeleton.Rig;
  readonly bones: readonly THREE.Bone[];
  readonly mesh: THREE.SkinnedMesh;
  readonly height: number;
  // One `themes.SLOTS` index per vertex, kept alongside the mesh so
  // `materialsForTeam` can bake a per-team vertex `color` attribute against
  // the SAME shared geometry (see that function).
  readonly paletteSlots: Float32Array;
}

interface TeamMaterials {
  // ONE material for the whole character -- see `materialsForTeam`'s doc
  // comment and this file's LIGHTING header for why that used to be three.
  readonly material: THREE.MeshStandardMaterial;
  readonly color: THREE.BufferAttribute;
}

const materialsByTeam = new Map<string, TeamMaterials>();
let built: BuiltCharacter | undefined;
let failed = false;

function mat4ToThree(m: Mat4): THREE.Matrix4 {
  const out = new THREE.Matrix4();
  // `.set()` takes row-major arguments; `Mat4` is stored row-major (see
  // @gc/core/mat4.ts), so this is a direct copy, not a transpose.
  out.set(m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11], m[12], m[13], m[14], m[15]);
  return out;
}

// Builds the shared rig, the ONE shared character mesh (colour-free), and
// the per-material-id `MeshStandardMaterial`s a team's resolved palette
// produces. Called once, lazily.
function build(): BuiltCharacter | undefined {
  if (built !== undefined || failed) {
    return built;
  }
  try {
    const rigProportions = RIG_PROPORTIONS;
    const rig = skeleton.newRig(rigProportions);
    const height = proportions.height(rigProportions);
    const theme = themes.LIST[0];
    const figure = themes.FIGURES[0];
    if (theme === undefined || figure === undefined) {
      throw new Error("player_renderer_3d.ts: no rig3d theme/figure content available");
    }
    const [partBuilder] = body.accumulate(rigProportions, theme, figure);

    const vertCount = partBuilder.verts.length;
    const positions = new Float32Array(vertCount * 3);
    const normals = new Float32Array(vertCount * 3);
    const skinIndices = new Float32Array(vertCount * 4);
    const skinWeights = new Float32Array(vertCount * 4);
    const paletteSlots = new Float32Array(vertCount);
    // Draw-call fix #1: reorder so vertices sharing a material family are
    // CONTIGUOUS, instead of `geometry.merge`'s own order (one run per PART
    // -- see that function's header, "Part order is preserved verbatim").
    // `body.ts`'s buildBody/buildKit/buildLoadout interleave plain/metal/
    // emissive PARTS (an armour piece's plain visor next to its metal band
    // next to an emissive seam), so the OLD per-contiguous-run grouping
    // fired a new group on every material TRANSITION, not once per family --
    // dozens of groups for a ~28-part character instead of three.
    //
    // Reordering here (rather than in `geometry.merge` itself) is
    // deliberate: `merge`'s own contract and tests pin "triangles rasterise
    // in submission order" for its own callers, and nothing about that
    // invariant is wrong -- it is simply not what a DEPTH-TESTED, opaque,
    // non-blended mesh needs. Triangle submission order never affects the
    // final image once depth testing decides visibility per pixel (there is
    // no painter's-algorithm blending across these three opaque materials),
    // so grouping by family here is a free lunch: identical picture, at most
    // 3 groups instead of ~50. A STABLE partition (plain, then metal, then
    // emissive, each preserving its own original relative order) rather than
    // a comparator sort, so this is deterministic regardless of engine sort
    // stability.
    const MATERIAL_ORDER = [geometry.MATERIAL.plain, geometry.MATERIAL.metal, geometry.MATERIAL.emissive] as const;
    const order: number[] = [];
    const groupCounts: [number, number, number] = [0, 0, 0];
    for (const wanted of MATERIAL_ORDER) {
      for (let i = 0; i < vertCount; i += 1) {
        const material = partBuilder.verts[i]?.material ?? geometry.MATERIAL.plain;
        if (material === wanted) {
          order.push(i);
          groupCounts[wanted] += 1;
        }
      }
    }

    // Draw-call fix #2: bake the same per-vertex shading-family float
    // `rig3d/renderer.lua`'s `VertexMaterial` attribute carried
    // (`geometry.MATERIAL`'s numbering: 0 plain, 1 metal, 2 emissive), so
    // `cel_shader.ts`'s `applyCombinedCelShading` can branch on it at
    // RUNTIME in one compiled program instead of needing a separate
    // material (and therefore a separate group/draw) per family -- see this
    // file's LIGHTING header. The geometry groups built below still exist
    // (Fix #1, kept for diagnostics -- `player_renderer_3d.spec.ts` pins
    // them at exactly 3), but three.js's `WebGLRenderer` only iterates
    // `geometry.groups` when a mesh's `.material` is an ARRAY
    // (`Array.isArray(material)` in `WebGLRenderer.js`'s `projectObject`);
    // `materialsForTeam` never assigns one, so they are inert for the draw
    // count, not load-bearing for it.
    const materialFamilies = new Float32Array(vertCount);
    order.forEach((srcIndex, i) => {
      const v = partBuilder.verts[srcIndex];
      if (v === undefined) {
        throw new Error("player_renderer_3d.ts: build() reorder produced an invalid vertex index");
      }
      positions[i * 3] = v.position[0];
      positions[i * 3 + 1] = v.position[1];
      positions[i * 3 + 2] = v.position[2];
      normals[i * 3] = v.normal[0];
      normals[i * 3 + 1] = v.normal[1];
      normals[i * 3 + 2] = v.normal[2];
      skinIndices[i * 4] = v.bone;
      skinWeights[i * 4] = 1;
      paletteSlots[i] = v.paletteSlot;
      materialFamilies[i] = v.material;
    });

    const geom = new THREE.BufferGeometry();
    geom.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geom.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    geom.setAttribute("skinIndex", new THREE.BufferAttribute(skinIndices, 4));
    geom.setAttribute("skinWeight", new THREE.BufferAttribute(skinWeights, 4));
    geom.setAttribute("materialFamily", new THREE.BufferAttribute(materialFamilies, 1));

    let groupStart = 0;
    for (const material of MATERIAL_ORDER) {
      const count = groupCounts[material];
      if (count > 0) {
        geom.addGroup(groupStart, count, material);
        groupStart += count;
      }
    }

    const boneDefs = skeleton.bones(rigProportions);
    const bones = boneDefs.map((def) => {
      const bone = new THREE.Bone();
      bone.name = def.name;
      // BOTH flags, not just `matrixAutoUpdate`. `matrixAutoUpdate = false`
      // only stops three.js recomputing `bone.matrix` from position/
      // quaternion/scale (irrelevant here -- those are never set; this file
      // writes `bone.matrixWorld` directly from the posed rig every frame,
      // see `prepareCharacter` below). But `character.mesh` DOES change its
      // own quaternion every frame (`prepareCharacter`'s facing yaw), which
      // marks the mesh's `matrixWorldNeedsUpdate` and makes
      // `Object3D.updateMatrixWorld` cascade `force = true` into every
      // child, bones included. A bone's `matrixWorldAutoUpdate` defaults to
      // `true`, so that forced cascade was recomputing every bone's
      // `matrixWorld` as `mesh.matrixWorld * bone.matrix` (`bone.matrix`
      // always identity, since it too is never set) -- silently discarding
      // the per-vertex pose this file just wrote and replacing it with the
      // rig's unposed rest transform on every `renderer.render()` call. This
      // was the actual cause of the "characters render mis-shapen/rotated"
      // symptom this port's report tracked down live -- not the render
      // target's `flipY` (see `renderToSprite`'s doc comment, which
      // toggling had zero visible effect on, confirming the bug was here,
      // upstream of compositing entirely).
      bone.matrixAutoUpdate = false;
      bone.matrixWorldAutoUpdate = false;
      return bone;
    });
    const skeletonObj = new THREE.Skeleton(bones);

    // three.js's own SkinnedMesh materials come from the geometry groups
    // above; a per-team palette resolves at draw time (see `draw` below),
    // so the mesh itself starts colour-free (matching the Lua original's
    // "one build serves every team" note).
    const mesh = new THREE.SkinnedMesh(geom, new THREE.MeshStandardMaterial());
    mesh.add(bones[0] ?? new THREE.Bone());
    mesh.bind(skeletonObj);

    built = { rig, bones, mesh, height, paletteSlots };
    return built;
  } catch (error) {
    failed = true;
    // eslint-disable-next-line no-console
    console.warn(`rigged 3D players disabled (build failed): ${String(error)}`);
    return undefined;
  }
}

// Bakes the Lua shader's per-vertex `u_palette[VertexPaletteSlot]` lookup
// (rig3d/renderer.lua's GLSL, quoted in this file's LIGHTING comment above)
// into a `THREE.BufferAttribute("color")` instead: three.js's stock
// `MeshStandardMaterial` has no dynamic per-vertex uniform-array indexing to
// port to (only a hand-written shader would, and the README marks that
// mechanism "replace"), but `vertexColors: true` reproduces the visible
// result -- every vertex shaded by its OWN resolved palette colour rather
// than one flat colour for the whole material group. This is what makes
// "skin" read differently from "cloth" (the team's `main` colour) on the
// SAME `plain`-material surface, and what makes the two teams distinguishable
// at all: the previous single `palette[0]` ("skin", never team-linked) used
// the same swatch for both sides regardless of `team`.
//
// `character` is passed in (not read from module state) because
// `paletteSlots` -- needed to bake the attribute -- lives on `BuiltCharacter`,
// produced by `build()`.
function materialsForTeam(character: BuiltCharacter, team: "home" | "away"): TeamMaterials {
  const key = team;
  const cached = materialsByTeam.get(key);
  if (cached !== undefined) {
    return cached;
  }
  const theme = themes.LIST[0];
  const teamData = themes.TEAMS[team === "away" ? 1 : 0];
  if (theme === undefined || teamData === undefined) {
    throw new Error("player_renderer_3d.ts: no rig3d theme/team content available");
  }
  const palette = themes.resolvedPalette(theme, teamData);

  const vertCount = character.paletteSlots.length;
  const colors = new Float32Array(vertCount * 3);
  for (let i = 0; i < vertCount; i += 1) {
    const slotIndex = character.paletteSlots[i] ?? 0;
    const rgba = palette[slotIndex] ?? [1, 1, 1, 1];
    colors[i * 3] = rgba[0];
    colors[i * 3 + 1] = rgba[1];
    colors[i * 3 + 2] = rgba[2];
  }
  const color = new THREE.BufferAttribute(colors, 3);

  // Base `.color` stays white so `vertexColors` passes the baked-per-vertex
  // palette colour through unmodified (three.js multiplies material colour x
  // vertex colour) -- `rig3d/cel_shader.ts`'s injected shading reads that
  // same `diffuseColor.rgb` for every family, metal and emissive included,
  // matching rig3d/renderer.lua's PIXEL stage, which shaded ALL three
  // families from the one `v_slot_color` varying (there was never a fixed
  // accent colour for emissive -- it multiplies the resolved palette colour
  // by a facing-dependent brightness boost instead; see `cel_shader.ts`'s
  // `celShadingChunk`). `roughness`/`metalness` are not set here: the
  // injected shading never reads three.js's `PhysicalMaterial` struct at all
  // (see `applyCombinedCelShading`'s doc comment), so they would be dead
  // uniforms -- the metal/plain/emissive distinction is entirely which
  // branch `vMaterialFamily` takes at runtime, a per-vertex value baked by
  // `build()` above, not a material property.
  //
  // ONE `MeshStandardMaterial`, not three (draw-call fix #2 -- see this
  // file's LIGHTING header and `build()`'s own comment on the
  // `materialFamily` attribute this material's shading reads).
  // `applyCombinedCelShading` is `cel_shader.ts`'s per-vertex-branching
  // sibling of the per-family `applyCelShading` used elsewhere in that
  // module's own test suite: same ported GLSL, selected at runtime off
  // `materialFamily` instead of at TypeScript-build time off which
  // `MeshStandardMaterial` instance this is.
  const material = new THREE.MeshStandardMaterial({ vertexColors: true });
  celShader.applyCombinedCelShading(material);
  const materials: TeamMaterials = { material, color };
  materialsByTeam.set(key, materials);
  return materials;
}

/** True when a rigged player can actually be drawn this frame. */
export function available(): boolean {
  return build() !== undefined;
}

// ---------------------------------------------------------------------------
// SINGLE-PASS COMPOSITING: `characterMesh`, its per-`playerId` pool, and the
// per-team geometry that pool depends on. See this file's header ("THE EXACT
// SIGNATURE THIS FILE IS WRITTEN TO EXPECT") and pitch.ts's header ("ONE
// PASS, ONE DEPTH BUFFER") for the full design this section implements.
// ---------------------------------------------------------------------------

// `materialsForTeam` bakes a per-team vertex `color` BufferAttribute (see
// that function's doc comment), but `build()`'s ONE shared `BufferGeometry`
// can only hold ONE such attribute at a time -- fine for the old design,
// where exactly one character was ever mid-render, but not once every
// character (both teams) coexists in the same scene simultaneously: setting
// the attribute on the shared geometry for team B would silently repaint
// every already-added team-A mesh too, since a `THREE.BufferAttribute` is
// GPU buffer state shared by every `SkinnedMesh` that references the same
// `BufferGeometry` object, not per-mesh.
//
// The fix is one geometry PER TEAM rather than one geometry total: position/
// normal/skinIndex/skinWeight and the material groups are genuinely shared
// (referenced, not copied -- they never change after `build()`), and only
// the `color` attribute differs, so this is one small wrapper object per
// team, not a deep clone of the mesh. Built lazily, cached forever (there are
// only two teams), mirroring `materialsByTeam`'s own cache lifetime.
const teamGeometry = new Map<"home" | "away", THREE.BufferGeometry>();

function geometryForTeam(character: BuiltCharacter, team: "home" | "away"): THREE.BufferGeometry {
  const cached = teamGeometry.get(team);
  if (cached !== undefined) {
    return cached;
  }
  const base = character.mesh.geometry;
  const position = base.getAttribute("position");
  const normal = base.getAttribute("normal");
  const skinIndex = base.getAttribute("skinIndex");
  const skinWeight = base.getAttribute("skinWeight");
  const materialFamily = base.getAttribute("materialFamily");
  if (position === undefined || normal === undefined || skinIndex === undefined || skinWeight === undefined || materialFamily === undefined) {
    throw new Error("player_renderer_3d.ts: shared character geometry is missing a required attribute");
  }
  const geom = new THREE.BufferGeometry();
  geom.setAttribute("position", position);
  geom.setAttribute("normal", normal);
  geom.setAttribute("skinIndex", skinIndex);
  geom.setAttribute("skinWeight", skinWeight);
  // `materialFamily`: read by `applyCombinedCelShading`'s injected shading
  // (draw-call fix #2, see `materialsForTeam`). The groups below are draw-
  // call fix #1's own artifact -- copied along for diagnostic parity with
  // `base` (and so `player_renderer_3d.spec.ts` can pin the group count via
  // this function's own output, the geometry `characterMesh` actually
  // returns) even though `materialsForTeam` never assigns an ARRAY of
  // materials any more, so three.js's `WebGLRenderer` never iterates them
  // (see `build()`'s comment on `Array.isArray(material)`).
  geom.setAttribute("materialFamily", materialFamily);
  for (const g of base.groups) {
    geom.addGroup(g.start, g.count, g.materialIndex ?? 0);
  }
  geom.setAttribute("color", materialsForTeam(character, team).color);
  teamGeometry.set(team, geom);
  return geom;
}

function buildCharacterBones(): THREE.Bone[] {
  return skeleton.bones(RIG_PROPORTIONS).map((def) => {
    const bone = new THREE.Bone();
    bone.name = def.name;
    // See build()'s own comment on BOTH flags, above: `matrixWorld` is
    // written directly from the posed rig every frame (`characterMesh`
    // below), so three.js must never recompute it from position/quaternion/
    // scale, nor cascade a forced recompute down from a moving parent.
    bone.matrixAutoUpdate = false;
    bone.matrixWorldAutoUpdate = false;
    return bone;
  });
}

interface PooledCharacter {
  readonly bones: readonly THREE.Bone[];
  readonly mesh: THREE.SkinnedMesh;
}

// Per-`playerId` rigged character instances, pooled and reused frame-to-frame.
// `build()`'s singleton `built.mesh`/`built.bones` is reused SEQUENTIALLY
// across players within one frame -- correct only because the old design
// rendered and composited one character at a time (see this file's header).
// Once every character coexists simultaneously in one scene graph for one
// shared render, each needs its OWN skeleton/mesh instance so posing one
// player can never clobber another's mid-frame, while still sharing the one
// expensive-to-build `BufferGeometry` (via `geometryForTeam`) and the shared
// `rig`/pose-evaluation machinery (`character.rig`, mutated and immediately
// consumed per player below -- JS is single-threaded, so this is safe the
// same way `prepareCharacter` already relies on it being safe).
//
// Never cleared, matching `built`/`materialsByTeam`'s own lifetime (neither
// of those is torn down on a match/scene teardown either). This is safe from
// `SceneRoot.dispose()`/draw2d.ts's per-frame `paint()` cleanup because
// pitch.ts adds a pooled mesh wrapped in a fresh `THREE.Group` each frame
// (see pitch.ts's header) and `draw2d.ts`'s `disposeObject` does not recurse
// into a `THREE.Group`'s children -- only directly-added
// `Mesh`/`Line`/`Sprite` objects are disposed, so a pooled mesh's geometry/
// material are never released out from under a still-live `playerId`. A
// roster whose player ids churn heavily across matches (unlikely in this
// game -- ids are stable roster slots) would leak pooled entries; not
// addressed here, no different from the module's existing singletons.
const characterPool = new Map<string, PooledCharacter>();

function pooledCharacter(playerId: string, character: BuiltCharacter, team: "home" | "away"): PooledCharacter {
  const cached = characterPool.get(playerId);
  if (cached !== undefined) {
    return cached;
  }
  const bones = buildCharacterBones();
  const skeletonObj = new THREE.Skeleton(bones);
  const mesh = new THREE.SkinnedMesh(geometryForTeam(character, team), new THREE.MeshStandardMaterial());
  mesh.add(bones[0] ?? new THREE.Bone());
  mesh.bind(skeletonObj);
  // DETACHED bind mode -- a real defect this port's report documents finding
  // live (characters rendered nowhere visible, not merely mis-sized): three.js's
  // DEFAULT "attached" bind mode makes `SkinnedMesh.updateMatrixWorld`
  // recompute `bindMatrixInverse = inverse(this.matrixWorld)` on EVERY
  // update, using whatever the mesh's CURRENT world transform happens to be
  // -- not the transform at bind time. `build()`'s singleton mesh (used by
  // `draw`/`renderToSprite`) never noticed: its OWN transform stays identity
  // forever (the OLD per-character CAMERA moved instead of the mesh), so
  // `inverse(identity) === identity` and the skinning math was correct by
  // accident. This pooled mesh is different BY DESIGN -- pitch.ts's
  // `riggedCharacterObject` gives its PARENT a real screen-placement
  // transform (`ppm` scale, `(sx, sy)` position) -- and "attached" mode's
  // dynamic `bindMatrixInverse` EXACTLY CANCELS that transform back out
  // (`matrixWorld * inverse(matrixWorld) * boneMatrix * position === boneMatrix
  // * position`), so the character skinned to its own raw rig-local
  // coordinates (roughly 0-2 units) instead of the screen position -- not
  // absent, just rendering in entirely the wrong place (effectively pinned
  // near the world origin) regardless of what the wrapper's transform said.
  // "detached" mode keeps `bindMatrix`/`bindMatrixInverse` FIXED at whatever
  // they were when `.bind()` ran above -- identity, since the mesh is not yet
  // parented to anything at that point -- so the wrapper's real transform is
  // the only one that ever applies.
  mesh.bindMode = "detached";
  const pooled: PooledCharacter = { bones, mesh };
  characterPool.set(playerId, pooled);
  return pooled;
}

/**
 * Pixels-per-metre uniform scale for a character rendered at on-screen
 * radius `r` (pixels -- matching the billboard fallback's own `r = radius *
 * scale`, see pitch.ts's `playerOptions`/`depthSortedItems` callers).
 * Mirrors `prepareCharacter`'s own `ppm` derivation exactly, so a rigged
 * player reads at the same visual weight it did as an offscreen-rendered
 * sprite. `undefined` when the rigged pass is unavailable (`build()` failed),
 * matching `characterMesh`'s own contract below.
 */
export function ppmForRadius(r: number): number | undefined {
  const character = build();
  if (character === undefined) {
    return undefined;
  }
  return (r * HEIGHT_IN_RADII * 2) / character.height;
}

/**
 * Impure: returns the posed, coloured, YAWED character mesh for `playerId`,
 * in its own local metre space -- exactly `prepareCharacter`'s
 * `character.mesh` as it existed before this port, minus the per-character
 * camera/scene/render-target work `renderToSprite` layers on top (see this
 * file's header, "THE EXACT SIGNATURE THIS FILE IS WRITTEN TO EXPECT", which
 * this function fulfils). Pooled per `playerId` (see `pooledCharacter`
 * above) rather than the shared singleton `draw`/`renderToSprite` still use.
 *
 * Deliberately does NOT apply the elevation tilt, screen-space scale/
 * position, or the Y-inversion -- pitch.ts owns all of that on the object
 * this function returns (see that file's "ONE PASS, ONE DEPTH BUFFER"
 * header section for exactly why the composition order there is not
 * arbitrary), since none of the three.js state involved (a per-entity
 * `THREE.Group`, `depthToZ`'s world z) is this file's to know about.
 *
 * Returns `undefined` when the rigged pass is unavailable (`build()`
 * failed) -- callers fall back to the procedural billboard, same contract
 * as `renderToSprite`.
 */
export function characterMesh(
  playerId: string,
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now: number,
): THREE.Object3D | undefined {
  const character = build();
  if (character === undefined) {
    return undefined;
  }
  const team = opts.team ?? "home";
  const pooled = pooledCharacter(playerId, character, team);

  const pose = poseFor(view, opts, now);
  skeleton.apply(character.rig, pose);
  pooled.bones.forEach((bone, i) => {
    const name = character.rig.order[i];
    const world = name !== undefined ? character.rig.world[name] : undefined;
    if (world !== undefined) {
      bone.matrixWorld.copy(mat4ToThree(world));
    }
  });
  pooled.mesh.skeleton.update();

  const facing = opts.facing;
  const yaw = facing !== undefined ? Math.atan2(facing.x, facing.y) : 0;
  pooled.mesh.quaternion.setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw);

  // A player's team is stable in practice (a stable roster slot, not a
  // per-frame choice), but re-checked every frame rather than cached on the
  // pooled entry -- both `geometryForTeam` and `materialsForTeam` are
  // memoised maps, so this costs a lookup, not a rebuild.
  const geometry = geometryForTeam(character, team);
  if (pooled.mesh.geometry !== geometry) {
    pooled.mesh.geometry = geometry;
  }
  // A single Material (not an array) -- see `materialsForTeam`'s doc comment
  // on why that is what makes this one draw call instead of up to three.
  const teamMaterials = materialsForTeam(character, team);
  pooled.mesh.material = teamMaterials.material;

  return pooled.mesh;
}

/**
 * Character camera placement: a small orthographic camera framing the
 * character so it lands at `(sx, sy)` on screen at `ppm` pixels-per-metre.
 * See this file's header scope note -- new code, not a port of unseen GLSL.
 */
export function characterCameraParams(sx: number, sy: number, ppm: number, vw: number, vh: number, elevation: number, height: number) {
  const halfHeightMetres = height / 2;
  return {
    // Looks down at `elevation` from slightly above and behind the
    // character's mid-height, along -Z (the character's own facing is
    // applied to the mesh's world transform instead of the camera).
    eye: [0, halfHeightMetres + Math.sin(elevation) * height, Math.cos(elevation) * height] as const,
    target: [0, halfHeightMetres, 0] as const,
    ppm,
    // Orthographic frustum in metres, centred so the character lands at
    // (sx, sy) once mapped to the shared viewport.
    left: -sx / ppm,
    right: (vw - sx) / ppm,
    top: sy / ppm,
    bottom: -(vh - sy) / ppm,
  };
}

// A throwaway scene holding just the shared character mesh (plus the two
// lights below), reused every draw call (matching the Lua original's
// one-draw-call-per-character shape). `scene.clear()` in `prepareCharacter`
// removes ALL children each frame, lights included, so the lights are
// re-added there alongside the mesh rather than added once here -- see that
// function.
const scene = new THREE.Scene();
const keyLight = new THREE.DirectionalLight(0xffffff, KEY_LIGHT_INTENSITY);
keyLight.position.copy(LIGHT_DIR).multiplyScalar(-10);
keyLight.target.position.set(0, 0, 0);
const fillLight = new THREE.HemisphereLight(SKY_COLOR, GROUND_COLOR, FILL_LIGHT_INTENSITY);

interface PreparedCharacter {
  readonly character: BuiltCharacter;
  readonly cam: THREE.OrthographicCamera;
}

// Shared setup for `draw` and `renderToSprite` -- both kept for parity/
// diagnostics now that pitch.ts's single-pass compositing calls
// `characterMesh` instead (see this file's header and pitch.ts's "ONE PASS,
// ONE DEPTH BUFFER" section): pose the shared singleton rig/mesh, orient/
// colour it for this player, build this player's OWN small camera, and stage
// the shared `scene` with just that mesh. Split out so the two entry points
// cannot drift on how a character is posed or framed -- only WHERE the
// result ends up differs. `characterMesh` above does NOT go through this --
// it poses a POOLED per-`playerId` mesh instead of the shared singleton, and
// has no camera to build at all (pitch.ts's shared camera handles every
// character), so it duplicates the pose/colour steps rather than reusing
// this function.
function prepareCharacter(
  sx: number,
  sy: number,
  r: number,
  vw: number,
  vh: number,
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now: number,
): PreparedCharacter | undefined {
  const character = build();
  if (character === undefined) {
    return undefined;
  }
  const pose = poseFor(view, opts, now);
  skeleton.apply(character.rig, pose);
  character.bones.forEach((bone, i) => {
    const name = character.rig.order[i];
    const world = name !== undefined ? character.rig.world[name] : undefined;
    if (world !== undefined) {
      bone.matrixWorld.copy(mat4ToThree(world));
    }
  });
  character.mesh.skeleton.update();

  const facing = opts.facing;
  const yaw = facing !== undefined ? Math.atan2(facing.x, facing.y) : 0;
  character.mesh.quaternion.setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw);

  const teamMaterials = materialsForTeam(character, opts.team ?? "home");
  character.mesh.material = teamMaterials.material;
  // The `color` attribute is swapped per draw call, same shared geometry
  // (see `TeamMaterials`'s doc comment) -- there is exactly one character
  // mesh in flight at a time (`scene.clear()` above), so this cannot race
  // between two teams' colours within a frame.
  character.mesh.geometry.setAttribute("color", teamMaterials.color);

  const ppm = (r * HEIGHT_IN_RADII * 2) / character.height;
  const far = character.height * 4 + 10;
  const params = characterCameraParams(sx, sy, ppm, vw, vh, ELEVATION, character.height);
  const cam = new THREE.OrthographicCamera(params.left, params.right, params.top, params.bottom, 0.01, far);
  cam.position.set(...params.eye);
  cam.lookAt(...params.target);
  cam.updateProjectionMatrix();

  scene.clear();
  scene.add(character.mesh, keyLight, keyLight.target, fillLight);
  return { character, cam };
}

/**
 * Impure: draws one rigged player, matching the Lua original's
 * `beginPass(cam, palette) / draw(mesh, world, bone_rows) / endPass()`
 * shape -- one immediate `renderer.render()` call per character, into
 * whatever target `renderer` is currently bound to. The caller is
 * responsible for `renderer.autoClear = false` across a frame's players (so
 * each character composites onto the same target instead of clearing the
 * others) and for restoring it afterward, exactly as `bloom.ts`'s `draw`
 * resets depth/cull/shader state after its own render pass. Untested -- see
 * file header.
 *
 * NOT CALLED BY `pitch.ts` -- kept for parity/diagnostics (a standalone
 * character preview has no "later full-scene render" to collide with). Do
 * not combine a call to this function with a subsequent whole-scene render
 * into the SAME target: that later render will clear what this one just drew
 * (see scene.ts's class doc comment, "FIXED HERE", for the defect this shape
 * caused once `SceneRoot` started doing both in one frame). `characterMesh`
 * above is what `pitch.draw` actually uses now (a posed mesh added to the
 * shared scene graph, not an immediate render); `renderToSprite` below is a
 * second, also-unused-by-pitch.ts alternative that composited via an
 * offscreen render target -- see that function's own doc comment for why it,
 * too, is no longer on pitch.ts's call path.
 */
export function draw(
  renderer: THREE.WebGLRenderer,
  sx: number,
  sy: number,
  r: number,
  vw: number,
  vh: number,
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now = 0,
): void {
  try {
    const prepared = prepareCharacter(sx, sy, r, vw, vh, view, opts, now);
    if (prepared === undefined) {
      return;
    }
    renderer.render(scene, prepared.cam);
  } catch (error) {
    failed = true;
    // eslint-disable-next-line no-console
    console.warn(`rigged 3D players disabled (draw failed): ${String(error)}`);
  }
}

/**
 * NOT CALLED BY `pitch.ts` ANYMORE -- kept for parity/diagnostics, same as
 * `draw` above. This was the fix for defect #1 (a rigged character no longer
 * getting cleared by `SceneRoot`'s later full-scene render -- see scene.ts's
 * "FIXED HERE" note) but it was still one full-viewport `renderer.render()`
 * call per character per frame, composited as a `depthTest: false` quad
 * (painter's order only, no per-pixel occlusion) -- see this file's header
 * and pitch.ts's "ONE PASS, ONE DEPTH BUFFER" section for what replaced it:
 * `characterMesh` above, a real depth-tested `SkinnedMesh` added to the SAME
 * scene `SceneRoot` already renders, with no separate render pass or render
 * target at all. Left in place, and left correct, as a smaller/cheaper
 * fallback shape a future caller could still reach for (e.g. a standalone
 * character-preview widget with no pitch scene to share).
 *
 * Impure: renders one rigged player OFF-SCREEN, into a private
 * `THREE.WebGLRenderTarget` sized to the FULL viewport, and returns the
 * result as a `THREE.Mesh` (a `vw`x`vh` plane at `(vw/2, vh/2)`, matching
 * every other draw2d.ts "fill" shape's own placement convention) ready to be
 * added directly to a `pitchGroup`-shaped `THREE.Group`. `characterCameraParams` is
 * UNCHANGED from `draw`'s: it already frames the character at `(sx, sy)`
 * across an asymmetric frustum spanning the WHOLE viewport (not a tight crop
 * around the character), which is exactly what makes a viewport-sized
 * transparent quad reproduce `draw`'s old direct-render compositing once it
 * is part of the scene graph -- only the render TARGET changes, from
 * whatever framebuffer `renderer` currently has bound to this private one, so
 * this render never touches the visible canvas. Only the later single
 * full-scene render does (see scene.ts's `SceneRoot.render`), with this mesh
 * already part of what it draws, at the correct point in `pitch.draw`'s
 * painter's-algorithm order.
 *
 * The renderer's previous target/clear-color/clear-alpha/autoClear are saved
 * and restored, matching this file's own `draw` contract note on renderer
 * state.
 *
 * The returned mesh's `userData.ownedRenderTarget` records the render
 * target so `draw2d.ts`'s `disposeObject` (used by both `paint`'s per-frame
 * rebuild and `SceneRoot.dispose`'s teardown) releases it -- an offscreen
 * target this heavy must not outlive the frame it was built for.
 *
 * VERIFIED WITH A LIVE GL CONTEXT (this port's report): the returned mesh's
 * `scale.y = -1` (set just before `return`, below) is required for the
 * character to land right-side-up under `SceneRoot`'s shared 2D orthographic
 * camera, which is Y-inverted relative to three.js's own default to match
 * draw2d.ts's screen-space convention. `target.texture.flipY` does NOT
 * affect this -- it is a no-op for `WebGLRenderTarget` textures (the flag
 * only matters for the CPU-upload path a normal image-backed `Texture`
 * goes through) -- see the comment at the `scale.y = -1` assignment for the
 * full explanation and how this was confirmed (reading the render target's
 * own pixels back independently of the plane/composite).
 */
export function renderToSprite(
  renderer: THREE.WebGLRenderer,
  sx: number,
  sy: number,
  r: number,
  vw: number,
  vh: number,
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now = 0,
): THREE.Mesh | undefined {
  try {
    const prepared = prepareCharacter(sx, sy, r, vw, vh, view, opts, now);
    if (prepared === undefined) {
      return undefined;
    }

    const target = new THREE.WebGLRenderTarget(vw, vh, { format: THREE.RGBAFormat });

    const previousTarget = renderer.getRenderTarget();
    const previousClearColor = renderer.getClearColor(new THREE.Color());
    const previousClearAlpha = renderer.getClearAlpha();
    const previousAutoClear = renderer.autoClear;
    try {
      renderer.setRenderTarget(target);
      renderer.setClearColor(0x000000, 0);
      renderer.autoClear = true;
      renderer.clear(true, true, true);
      renderer.render(scene, prepared.cam);
    } finally {
      renderer.setRenderTarget(previousTarget);
      renderer.setClearColor(previousClearColor, previousClearAlpha);
      renderer.autoClear = previousAutoClear;
    }

    const material = new THREE.MeshBasicMaterial({
      map: target.texture,
      transparent: true,
      depthTest: false,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const geometry = new THREE.PlaneGeometry(vw, vh);
    const mesh = new THREE.Mesh(geometry, material);
    mesh.position.set(vw / 2, vh / 2, 0);
    // VERIFIED WITH A LIVE GL CONTEXT (see this port's report): the character
    // itself rendered upright and correctly posed into `target` -- confirmed
    // by reading `target`'s pixels back directly with
    // `renderer.readRenderTargetPixels` and inspecting them independently of
    // this plane/composite. What was NOT correct was this plane's own
    // mapping of that texture: `target.texture.flipY` (previously set here)
    // turned out to be a NO-OP -- three.js's `flipY` only affects the
    // CPU-upload path (`texImage2D`) a normal image-backed `Texture` goes
    // through, and a `WebGLRenderTarget`'s texture is written by the GPU
    // directly (`framebufferTexture2D`), which never goes through that path,
    // so setting it either way rendered identically. The actual mismatch is
    // between this plane's default UV orientation and `SceneRoot`'s shared
    // 2D camera, which is deliberately Y-INVERTED to match draw2d.ts's
    // "y increasing downward" convention (`top = 0`, `bottom = viewport.h`,
    // see scene.ts's `SceneRoot` doc comment) -- one axis flip relative to
    // three.js's own default. A plain, unrotated `PlaneGeometry` samples its
    // texture with v=0 at local -Y and v=1 at local +Y; under this
    // viewport's inverted camera, local +Y renders toward the BOTTOM of the
    // screen, so the texture's v=1 row (the top of what the character camera
    // rendered, i.e. the character's head) was landing at the screen's
    // bottom -- upside down. `scale.y = -1` mirrors the plane's geometry
    // (not the texture) around its own centre, which is exactly enough to
    // cancel that one inversion; `side: THREE.DoubleSide` above already
    // makes the resulting inverted winding a non-issue.
    mesh.scale.y = -1;
    mesh.userData["ownedRenderTarget"] = target;
    return mesh;
  } catch (error) {
    failed = true;
    // eslint-disable-next-line no-console
    console.warn(`rigged 3D players disabled (renderToSprite failed): ${String(error)}`);
    return undefined;
  }
}
