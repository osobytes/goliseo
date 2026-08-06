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
import type { PlayerRenderOptions } from "./player_renderer.ts";

// Character height maps to roughly this many player-radii on screen. Tuned
// so a rigged player reads at the same visual weight as the billboard it
// replaces.
const HEIGHT_IN_RADII = 3.0;
// Match camera looks down at the pitch; this is the apparent elevation.
const ELEVATION = (17 * Math.PI) / 180;

// Mirrors `sim/match.lua`'s `PLAYER_RADIUS`. See file header.
export const DEFAULT_PLAYER_RADIUS = 12;

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
}

const materialsByTeam = new Map<string, [THREE.MeshStandardMaterial, THREE.MeshStandardMaterial, THREE.MeshStandardMaterial]>();
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
    const rigProportions = proportions.RIG_MEDIUM;
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
    partBuilder.verts.forEach((v, i) => {
      positions[i * 3] = v.position[0];
      positions[i * 3 + 1] = v.position[1];
      positions[i * 3 + 2] = v.position[2];
      normals[i * 3] = v.normal[0];
      normals[i * 3 + 1] = v.normal[1];
      normals[i * 3 + 2] = v.normal[2];
      skinIndices[i * 4] = v.bone;
      skinWeights[i * 4] = 1;
    });

    const geom = new THREE.BufferGeometry();
    geom.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    geom.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    geom.setAttribute("skinIndex", new THREE.BufferAttribute(skinIndices, 4));
    geom.setAttribute("skinWeight", new THREE.BufferAttribute(skinWeights, 4));

    // Material groups: one contiguous run per `geometry.MATERIAL` id, since
    // `geometry.merge` writes vertices part-by-part (each part has one
    // material) rather than interleaving them.
    let groupStart = 0;
    let groupMaterial: number = partBuilder.verts[0]?.material ?? geometry.MATERIAL.plain;
    for (let i = 1; i <= vertCount; i += 1) {
      const v = partBuilder.verts[i];
      const material: number = v?.material ?? -1;
      if (material !== groupMaterial) {
        geom.addGroup(groupStart, i - groupStart, groupMaterial);
        groupStart = i;
        groupMaterial = material;
      }
    }

    const boneDefs = skeleton.bones(rigProportions);
    const bones = boneDefs.map((def) => {
      const bone = new THREE.Bone();
      bone.name = def.name;
      bone.matrixAutoUpdate = false;
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

    built = { rig, bones, mesh, height };
    return built;
  } catch (error) {
    failed = true;
    // eslint-disable-next-line no-console
    console.warn(`rigged 3D players disabled (build failed): ${String(error)}`);
    return undefined;
  }
}

function materialsForTeam(team: "home" | "away"): readonly THREE.MeshStandardMaterial[] {
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
  // A single representative colour per material id (metal/emissive read a
  // fixed accent rather than the per-vertex palette slot -- see this
  // function's header note in the report on why per-vertex colour baking
  // was not implemented in this milestone).
  const plainColor = palette[0] ?? [1, 1, 1, 1];
  const plain = new THREE.MeshStandardMaterial({ color: new THREE.Color(plainColor[0], plainColor[1], plainColor[2]), roughness: 0.8 });
  const metal = new THREE.MeshStandardMaterial({ color: new THREE.Color(0.8, 0.8, 0.85), metalness: 0.6, roughness: 0.35 });
  const emissive = new THREE.MeshStandardMaterial({ color: new THREE.Color(0, 0, 0), emissive: new THREE.Color(0.4, 0.9, 1), emissiveIntensity: 1.2 });
  const materials: [THREE.MeshStandardMaterial, THREE.MeshStandardMaterial, THREE.MeshStandardMaterial] = [plain, metal, emissive];
  materialsByTeam.set(key, materials);
  return materials;
}

/** True when a rigged player can actually be drawn this frame. */
export function available(): boolean {
  return build() !== undefined;
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

// A throwaway scene holding just the shared character mesh, reused every
// draw call (matching the Lua original's one-draw-call-per-character shape).
const scene = new THREE.Scene();

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
  const character = build();
  if (character === undefined) {
    return;
  }
  try {
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

    const materials = materialsForTeam(opts.team ?? "home");
    character.mesh.material = [...materials];

    const ppm = (r * HEIGHT_IN_RADII * 2) / character.height;
    const far = character.height * 4 + 10;
    const params = characterCameraParams(sx, sy, ppm, vw, vh, ELEVATION, character.height);
    const cam = new THREE.OrthographicCamera(params.left, params.right, params.top, params.bottom, 0.01, far);
    cam.position.set(...params.eye);
    cam.lookAt(...params.target);
    cam.updateProjectionMatrix();

    scene.clear();
    scene.add(character.mesh);
    renderer.render(scene, cam);
  } catch (error) {
    failed = true;
    // eslint-disable-next-line no-console
    console.warn(`rigged 3D players disabled (draw failed): ${String(error)}`);
  }
}
