// Rigged 3D player renderer, and since #415 the ONLY one: the procedural 2D
// billboard this was once "a drop-in alternative to" is deleted, along with
// every path that could substitute it silently. `sx`, `sy` and `r` still come
// from `camera.ts` -- that projection stays the authority on where a player is
// and how large they appear (see pitch.ts's `playerAnchor`). This module only
// decides *how* the player is drawn at that spot: a rigged, animated,
// depth-tested character.
//
// PURE AND TESTED: pose selection (`clipFor`/`poseFor`) and the metres-per-
// world-unit conversion. These are content decisions (which clip plays for
// which pose id, how gait/stance/whole-body actions layer) with no GPU
// dependency now that `now` is threaded through as an explicit parameter
// instead of read from a global clock.
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
// SCOPE NOTE on the character camera. `draw`/`renderToSprite` render each
// rigged character through its OWN small orthographic camera, positioned so
// the character lands at the projection's exact screen coordinates -- a "3D
// insert into a 2D scene" trick, not a single whole-pitch 3D camera. The
// load-bearing values below are `HEIGHT_IN_RADII`, `ELEVATION`, the
// metres-per-world-unit formula, and the pose-selection table. The camera
// placement math (`characterCameraParams`) is new code, written the way
// three.js expects (an `OrthographicCamera`'s position/target/frustum).
//
// ANIMATION PLAYBACK (#425). What actually poses a character on pitch.ts's
// call path is `rig3d/animator.ts`, a `THREE.AnimationMixer` over the same
// `rig3d/clips.ts` content, driven by `rig3d/pose_table.ts`'s explicit
// 32-entry `PlayerPoseId -> action` table. `poseFor` and `clipFor` below are
// the PREVIOUS, procedural sampler, kept -- unchanged in behaviour apart from
// one dead branch removed -- as the A/B parity reference the two paths are
// pinned against in `player_renderer_3d.spec.ts`. Removing them is signed off
// separately (#425's last task), once the mixer path has been through a
// visual pass.
//
// The mixer does NOT change this file's skinning path, which is the one thing
// that looked like it would have to change: an `AnimationMixer` drives LOCAL
// bone transforms and needs three.js to derive world matrices, whereas the
// `THREE.Bone`s built below deliberately bypass that pipeline entirely
// (`matrixAutoUpdate = false`, `matrixWorldAutoUpdate = false`, `bindMode =
// "detached"`, `bone.matrixWorld` written directly) -- three separate fixes
// for three separate defects, each documented at its own assignment below.
// `rig3d/mixer.ts` resolves that by never touching a `THREE.Bone`: it drives a
// private graph of plain `THREE.Object3D` pose carriers and hands back the
// same sparse `{rot, move}` pose `clips.sample`/`clips.layer` always produced,
// so everything from `skeleton.apply` downward is untouched. See that file's
// header for the full argument.
//
// Boundary note (ARCHITECTURE.md §4 rule 6): `match.PLAYER_RADIUS` comes from
// Rust `crates/gc-sim`. `DEFAULT_PLAYER_RADIUS` mirrors its current value
// (12) as an injectable default, the same pattern `pitch.ts`'s
// `DEFAULT_ARENA` uses for Rust-owned content.

import * as THREE from "three";
import { quat } from "@gc/core";
import type { Mat4 } from "@gc/core";
import type { PlayerView } from "./view_state.ts";
import { viewState } from "./view_state.ts";
import * as skeleton from "./rig3d/skeleton.ts";
import * as proportions from "./rig3d/proportions.ts";
import * as clips from "./rig3d/clips.ts";
import * as masks from "./rig3d/masks.ts";
import * as crouch from "./rig3d/crouch.ts";
import * as actionPose from "./rig3d/action_pose.ts";
import * as animator from "./rig3d/animator.ts";
import * as themes from "./rig3d/themes.ts";
import * as presentationContent from "./rig3d/presentation_content.ts";
import * as body from "./rig3d/body.ts";
import * as geometry from "./rig3d/geometry.ts";
import * as ground from "./rig3d/ground.ts";
import * as celShader from "./rig3d/cel_shader.ts";
import type { PlayerRenderOptions } from "./player_render_options.ts";

// Character height maps to roughly this many player-radii on screen. Tuned
// so a rigged player reads at the same visual weight as the billboard it
// replaced (#415 deleted that renderer; this calibration outlived it).
const HEIGHT_IN_RADII = 3.0;
// Match camera looks down at the pitch; this is the apparent elevation.
// Exported: pitch.ts's single-pass compositing (see that file's "ONE PASS,
// ONE DEPTH BUFFER" header section) needs this to compose the same tilt onto
// a character's own transform, now that there is no longer a separate
// per-character camera to hold it.
//
// Declared in `cel_shader.ts` and re-exported here, same as `LIGHT_DIR` below
// and for a sharper reason: the cel shader has to REMOVE exactly the tilt
// pitch.ts composes on, so the two must be the same number by construction
// rather than by coincidence (see that file's THE SHADING FRAME section).
export const ELEVATION = celShader.ELEVATION;

// LIGHTING. Toon shading (ARCHITECTURE.md §5 marks the mechanism itself --
// the depth pass, the draw call, the WebGL1 bone/palette packing -- as
// "Rendering and materials -- WebGLRenderer, MeshStandardMaterial.", but that
// verdict does not cover the shading; see `rig3d/cel_shader.ts`'s file header for the
// full argument) is driven off one directional key light,
// `light_dir = { -0.42, -0.78, -0.46 }` ("direction the light travels"),
// plus a soft "cool bounce light from below so shadowed sides do not go
// dead", three flat quantised bands, a view-dependent rim, and a
// metal-only hard specular.
//
// `rig3d/cel_shader.ts`'s `applyCombinedCelShading` splices that shading
// into `materialsForTeam`'s ONE `MeshStandardMaterial` via
// `onBeforeCompile`, reading `LIGHT_DIR` as a hardcoded uniform rather than
// any `THREE.Light` in the scene -- there are no scene-graph lights driving
// it, only this hand-set uniform. `LIGHT_DIR` itself lives in
// `cel_shader.ts` (re-exported here) so the toon shading and this file's
// decorative light share one constant instead of two copies that could
// drift.
//
// ONE material, not three (draw-call fix, see `materialsForTeam` and
// `build()`'s own comments below): the shading family is read off a
// per-vertex `VertexMaterial` attribute and branched on inside ONE fragment
// shader -- "branching on a varying in the fragment stage is fine here;
// only dynamic *array indexing* is forbidden" (`cel_shader.ts`'s own
// SHADER_SOURCE comment). An earlier version of this reproduced the
// FAMILIES (plain/metal/emissive) but not that mechanism: three separate
// `MeshStandardMaterial`s, one `onBeforeCompile` variant each, selected at
// mesh-build time via `THREE.BufferGeometry` material groups -- which
// resurrected the "one draw per material" cost this design specifically
// avoids, and worse, `body.ts`'s parts interleave families (a plain visor
// next to a metal band next to an emissive seam), so groups fired on every
// material *transition*, not once per family -- tens of draws per character
// instead of three. `build()` below bakes a per-vertex family float
// (`materialFamily`), and `applyCombinedCelShading` branches on it at
// runtime in one compiled program, collapsing the whole character back to
// ONE draw call.
//
// The two `THREE.Light` instances below therefore no longer drive the
// character's own shading -- `cel_shading`'s replaced fragment chunk never
// calls `RE_Direct` / `RE_IndirectDiffuse`, so every scene light is a no-op
// for a `SkinnedMesh` using one of `materialsForTeam`'s materials. They stay
// in place regardless: an earlier fix here was "the private module-level
// scene had no lights" (a real defect against the *previous*, un-toon-shaded
// `MeshStandardMaterial` state), and removing them now would undo that fix
// for no benefit. They are harmless dead weight for the character mesh
// today, not a hazard.
const { LIGHT_DIR } = celShader;
const KEY_LIGHT_INTENSITY = 3.2;
// Vestigial alongside `keyLight` above -- see this comment block.
const SKY_COLOR = 0xaebfe0;
const GROUND_COLOR = 0x30251c;
const FILL_LIGHT_INTENSITY = 0.9;

// Mirrors Rust `crates/gc-sim`'s `PLAYER_RADIUS`. See file header.
export const DEFAULT_PLAYER_RADIUS = 12;

// The one rig contract every character uses (see rig3d/proportions.ts's own
// header: "there is one rig here"). Hoisted out of `build()` so `pooledCharacter`
// below can derive a fresh bone list for each player from the SAME contract
// `build()` used for the shared geometry, without a second source of truth.
const RIG_PROPORTIONS = proportions.RIG_MEDIUM;

/**
 * The height this project declares a player to be. THE SCALE DECLARATION:
 * this number, and not the rig's own geometry, is what ties a world unit to a
 * metre everywhere in the codebase.
 *
 * The distinction matters because it was conflated for a long time and the
 * conflation was load-bearing in the wrong direction. `proportions.height()`
 * measures the authored mesh in the rig's OWN space, where the figure happens
 * to sum to about 1.57 m. That is an art-space number: the rig is a stylised
 * ~5.4-head silhouette, its equipment is authored in the same space
 * (`rig3d/equipment.ts` never reads `RigProportions` at all), and nothing about
 * it is a claim that a footballer is 1.57 m tall.
 *
 * Deriving the project's metre scale from that number made every metre-
 * denominated statement elsewhere quietly wrong -- the pitch, the ball, the
 * gravity constant. Rescaling the rig to fix it was tried and abandoned: it
 * leaves every absolute-metre art asset behind (equipment came out 11.8%
 * small), so it trades a labelling problem for a real visual one.
 *
 * So the scale is declared here instead, once, and the art is left alone.
 */
export const DECLARED_PLAYER_HEIGHT_M = 1.75;

/**
 * Metres per world unit, at this project's declared scale. Derived from
 * [`DECLARED_PLAYER_HEIGHT_M`] over the pixel height a player actually draws
 * at (`PLAYER_RADIUS * HEIGHT_IN_RADII * 2` = 72), which is 24.31 mm.
 *
 * What this makes true, and checkable: the 1648x927 pitch is 40.1 x 22.5 m --
 * squarely inside regulation futsal bounds -- the ball is 29 cm
 * across, and `gc_sim::ball_flight`'s 900 px/s^2 gravity is 21.9 m/s^2, about
 * 2.2x Earth. That last one is a deliberate arcade dial (docs/vision.md:
 * "Readable over realistic"), not a bug, but it is worth being able to state
 * it as a number rather than discovering it.
 */
export const METRES_PER_WORLD_UNIT =
  DECLARED_PLAYER_HEIGHT_M / (DEFAULT_PLAYER_RADIUS * HEIGHT_IN_RADII * 2);

/**
 * Metres per world unit for an arbitrary declared player height. Prefer the
 * [`METRES_PER_WORLD_UNIT`] constant above; this is the general form, kept for
 * callers exploring a different declared scale.
 *
 * Note the argument is a DECLARED height, not `proportions.height()` -- see
 * [`DECLARED_PLAYER_HEIGHT_M`] for why those are different things.
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

// SUPERSEDED BY `rig3d/pose_table.ts` (#425), kept as the A/B parity
// reference -- see this file's ANIMATION PLAYBACK header note. This table maps
// 12 of the 32 `PlayerPoseId`s onto a clip name and silently returns `"idle"`
// for the other 20, which is the defect `POSE_ACTIONS` exists to fix; nothing
// on pitch.ts's call path reads it any more.
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

// ---------------------------------------------------------------------------
// TORSO LEAN (`view_state.ts`'s `PlayerView.lean`)
// ---------------------------------------------------------------------------
//
// `viewState` derives one `lean` per player per frame -- `clamp(vx / 120, -1,
// 1)`, exponentially smoothed, from WORLD-X velocity alone. Nothing in this
// package read it until this function did, so the rigged character never
// leaned into a sprint or a turn; and since #418 deleted the procedural
// billboard, this file is the ONLY renderer, so "nobody consumes `lean`" and
// "no player on the pitch leans" were the same statement.
//
// WHERE THE MAGNITUDE COMES FROM. The retired billboard renderer sized this
// signal against the real game, and that tuning is the only empirical anchor
// the constant has, so it is what the rig is calibrated back to. Its `figure`
// did `cx = bx + lean * r * 0.5 + ...` -- one on-screen player-radius `r`
// being the same unit the projection already speaks in. That source is gone;
// read it at `git show 0333065^:v2/ts/packages/render/src/player_renderer.ts`
// if the derivation below ever needs re-checking. Everything the derivation
// USES is live code: `ppmForRadius` here, `proportions.RIG_MEDIUM` and
// `proportions.height` in rig3d/proportions.ts.
//
// TRANSLATION THERE, ROTATION HERE. `cx` was a pure horizontal SHIFT of the
// whole billboard -- hips, feet and head moving together -- which a billboard
// can afford because it has no ground contact to violate. The rig does: its
// feet are placed by the skeleton and there is no IK to put a slid foot back
// (skeleton.ts, "No IK here"), so the same displacement has to arrive as a
// tilt. That substitution is the reason the mapping needs an argument at all:
// a translation moves every point by the same distance, while a rotation
// moves each point in proportion to its height above the pivot, so "the same
// amount of lean" is only well defined once you name WHICH point.
//
// DEGREES PER UNIT LEAN:
//
//   1. The target displacement, exactly. At |lean| = 1 the billboard moved by
//      0.5 * r pixels, and `k * r` pixels is `k * height / 6` metres for any
//      k: `ppmForRadius` sets ppm = r * HEIGHT_IN_RADII * 2 / height, so the
//      `r` cancels and this conversion is depth-independent, not a fit.
//      `proportions.height(RIG_MEDIUM)` is 1.5676 m, so 0.5 * r is 0.1306 m.
//
//   2. The reference point, approximately. The head is what the eye tracks in
//      a lean, and on this rig the head spans y = 1.23 m (the `head` bone) to
//      y = 1.5676 m (the crown `height()` accounts for). This step is a
//      judgement about where to read the displacement, not a second exact
//      conversion -- the drawn billboard's own pixel extent was 3.2-3.6r
//      depending on `species_shape`, so there was never a single exact
//      percentage-of-height to recover from it either.
//
//   3. Tilting the torso chain -- spine at y = 0.89 m, chest at y = 1.08 m
//      (`proportions.RIG_MEDIUM`) -- by t1 then t2 displaces a point at
//      height h by 0.19 * sin(t1) + (h - 1.08) * sin(t1 + t2). At t1 = 9 deg,
//      t2 = 7 deg that is 0.071 m at the head bone and 0.164 m at the crown,
//      straddling step 1's 0.1306 m and matching it at y ~ 1.45 m, mid-skull.
//
// So: 16 degrees of torso tilt per unit lean, spread 9 on the spine and 7 on
// the chest. Spread rather than concentrated because skeleton.ts's own design
// rules say so ("the arch is spread across hips + spine + chest + neck") --
// a single 16-degree hinge at one joint reads as a broken back.
//
// The tests below the constant pin the SHAPE of this (which bones move, the
// axis decomposition, the sign, the action-pose interaction), not the two
// numbers: 9 and 7 are a tuning, and re-tuning them by eye at match-camera
// scale is a legitimate follow-up, not a regression.
const SPINE_LEAN_DEGREES = 9;
const CHEST_LEAN_DEGREES = 7;

// WHY THE SPINE AND CHEST, NOT THE ROOT.
//
// `actionPose.apply` owns `root`, and since #430 it writes there with TWO
// different semantics -- which changes nothing about the conclusion below but
// must not be misread as one rule:
//
//   * an ACTION (a dive, an aerial, a knockback: whatever `forOptions`
//     returns) ASSIGNS `root` outright, `pose.rot[bone] = quat.fromEuler(...)`;
//   * a held ATTITUDE (`attitudeFor`: a contain, a crouch, a follow-through)
//     COMPOSES onto `root` instead -- pre-multiplied rotation, added
//     translation -- so it rides the gait rather than overwriting its bob.
//
// The reason lean stays off `root` is the ACTION case, and it is unaffected:
// a lean written to `root` BEFORE the overlay is silently discarded the
// instant a keeper dives, and a lean composed onto `root` AFTER it would
// stack 16 degrees of balance tilt onto a 72-degree committed dive about a
// neighbouring axis -- the "keeper dive and lean fighting each other" case.
// Keeping lean off `root` entirely means there is no ordering question to get
// wrong and `root` keeps exactly one owner.
//
// The attitude case does not reopen it either, from the other direction: an
// attitude is deliberately NOT part of `forOptions`, so the gate below does
// not fire for one, and lean and attitude coexist on different bones (torso
// vs root) rather than competing for the same one. That is the whole design
// -- a defender containing at speed still leans -- and it is pinned in this
// file's spec. See action_pose.ts's TWO KINDS OF ROOT TRANSFORM note.
//
// It is also the anatomically right joint: a torso lean leaves the feet where
// the locomotion clip planted them, which matters because this rig has no IK
// (skeleton.ts: "No IK here") and nothing would put a slid foot back.
//
// The tilt is MULTIPLIED onto whatever the locomotion blend already resolved
// for those two bones rather than replacing it, so a leaning player keeps
// their stride's own torso motion. Pre-multiplying applies the lean in the
// PARENT's frame (`skeleton.apply` evaluates `rot * rest` under the parent),
// which is what makes it a tilt of the torso relative to the pelvis.
//
// Derived, never stored: `poseFor` is called fresh every frame and this reads
// `view.lean`, which `view_state.ts` owns.

/**
 * The torso tilt one player's `view.lean` asks for, in DEGREES, decomposed
 * into the character's OWN axes (`x` tips forward onto the face, `z` tips
 * toward their own right -- action_pose.ts's orientation note).
 *
 * `lean` is a world-X signal but the rig is yawed to `facing`, so the world-X
 * direction has to be resolved into the character's local frame or a player
 * running "up" the pitch would lean sideways when they should be leaning
 * forward. Local +Z is `(facing.x, facing.y)` in pitch coordinates and local
 * +X is `(facing.y, -facing.x)`, so world +X projects onto them as `facing.y`
 * and `facing.x` respectively. With no facing the rig is drawn unyawed
 * (`characterMesh`'s `yaw = 0`), whose local +X IS pitch +x -- so the
 * no-facing fallback is a pure roll, matching what is actually drawn.
 *
 * Returns `undefined` when there is no meaningful lean to apply.
 */
export function leanTilt(
  lean: number,
  facing?: actionPose.XY,
): { readonly x: number; readonly z: number } | undefined {
  if (!Number.isFinite(lean) || Math.abs(lean) < 1e-4) {
    return undefined;
  }
  const amount = Math.max(-1, Math.min(1, lean));
  const fx = facing?.x ?? 0;
  const fy = facing?.y ?? 1;
  const len = Math.hypot(fx, fy);
  // An all-but-zero facing carries no orientation, so fall back to the
  // unyawed frame the renderer would actually draw.
  const nx = len > 1e-6 ? fx / len : 0;
  const ny = len > 1e-6 ? fy / len : 1;
  // Positive x tips FORWARD (toward local +Z); toward local +X needs a
  // NEGATIVE z (see action_pose.ts's orientation note).
  return { x: amount * nx, z: -amount * ny };
}

// Composes `leanTilt`'s tilt onto the torso chain, mutating and returning the
// pose. See the LEAN section above for the mapping and the bone choice.
function applyLean(
  pose: actionPose.MutablePose,
  lean: number,
  facing?: actionPose.XY,
): actionPose.MutablePose {
  const tilt = leanTilt(lean, facing);
  if (tilt === undefined) {
    return pose;
  }
  const toRadians = Math.PI / 180;
  for (const [bone, degrees] of [
    ["spine", SPINE_LEAN_DEGREES],
    ["chest", CHEST_LEAN_DEGREES],
  ] as const) {
    const q = quat.fromEuler(tilt.x * degrees * toRadians, 0, tilt.z * degrees * toRadians);
    const existing = pose.rot[bone];
    pose.rot[bone] = existing !== undefined ? quat.multiply(q, existing) : q;
  }
  return pose;
}

/**
 * Resolves the pose for one player with the PROCEDURAL sampler. `now`:
 * seconds, passed in explicitly rather than read from a global clock.
 *
 * Superseded on pitch.ts's call path by `mixerPoseFor` below; kept as the A/B
 * parity reference the mixer path is pinned against (see this file's
 * ANIMATION PLAYBACK header note and `player_renderer_3d.spec.ts`'s parity
 * suite). Behaviour is unchanged from before #425 except for the removal of an
 * unreachable `"swing"` branch -- no `POSE_CLIP` entry has ever returned
 * `"swing"`, so that code had never run; `rig3d/pose_table.ts` is where the
 * SWING clip finally gets a caller.
 */
export function poseFor(
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now: number,
): actionPose.MutablePose {
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
  const runMix = Math.max(
    0,
    Math.min((speed - viewState.WALK_SPEED) / (viewState.RUN_SPEED - viewState.WALK_SPEED), 1),
  );

  // Both cycles are two steps with contacts at 0 and 0.5, so one normalised
  // phase drives both and they stay in step through the blend.
  const cycles = view?.gait ?? 0;

  // Idle rate READ FROM THE ANIMATOR, not restated. This function is the A/B
  // parity reference for the mixer path (see this file's ANIMATION PLAYBACK
  // header note), so a hardcoded copy here is not a duplicate constant, it is a
  // way for the reference to silently stop referencing. It held 0.35 while
  // #574 retuned the breath to 0.8, and the parity spec caught it.
  let pose: actionPose.MutablePose = clips.layer(
    clips.sample(idle, now * animator.IDLE_RATE),
    clips.sample(walk, cycles * walk.duration),
    masks.FULL_BODY,
    walkMix,
  );
  if (runMix > 0) {
    pose = clips.layer(pose, clips.sample(run, cycles * run.duration), masks.FULL_BODY, runMix);
  }

  const selected = clipFor(opts.pose?.id);
  if (selected === "guard") {
    pose = clips.layer(pose, clips.sample(clips.GUARD_STANCE, now), masks.UPPER_BODY, 1);
  } else if (selected === "charge") {
    // The charge is a held pose, so it only needs a phase to breathe on;
    // tying it to the stride keeps the sway in step with the legs.
    pose = clips.layer(
      pose,
      clips.sample(clips.CHARGE, cycles * clips.CHARGE.duration),
      masks.UPPER_BODY,
      1,
    );
  }

  // The keeper's hands follow possession, not the pose id: arms wrapped
  // around a ball outrank whatever else the keeper is doing.
  const throwAmount = opts.throw ?? 0;
  if (throwAmount > 0) {
    // throw_timer counts DOWN, so 1 is the moment of commitment.
    const sling = clips.KEEPER_SLING;
    pose = clips.layer(
      pose,
      clips.sample(sling, (1 - Math.min(throwAmount, 1)) * sling.duration),
      masks.UPPER_BODY,
      1,
    );
  } else if (opts.holding === true) {
    pose = clips.layer(pose, clips.sample(clips.KEEPER_GATHER, now), masks.UPPER_BODY, 1);
  }

  // The knee bend under a settling body (#445), on the same seam and in the
  // same order the mixer path applies it. Stateless here, where the mixer path
  // eases the depth in: this sampler has no notion of a previous frame (it
  // cannot crossfade either), so what it produces is what the mixer produces on
  // a character's FIRST frame -- which is what the parity suite compares.
  const settling = crouch.poseFor(actionPose.attitudeDrop(opts));
  if (settling !== null) {
    pose = clips.compose(pose, settling);
  }

  // Whole-body actions last: they move the root, so they ride on top of
  // whatever gait and stance resolved instead of competing with them.
  const posed = actionPose.apply(pose, opts);

  // Balance lean, on the torso only -- see the TORSO LEAN section above for
  // the mapping and for why this never touches `root`. Suppressed outright
  // while a whole-body action owns the body: a dive/aerial/knockback IS the
  // displacement of the mass, and the velocity-derived lean it produces is a
  // second reading of the same motion, not an extra cue. `forOptions` is the
  // same pure predicate `apply` just consulted -- cheap enough to ask twice
  // rather than widening action_pose.ts's interface for this.
  if (actionPose.forOptions(opts) === null) {
    applyLean(posed, view?.lean ?? 0, opts.facing);
  }
  return posed;
}

/**
 * Resolves the pose for one player with the `THREE.AnimationMixer` path
 * (#425). This is what pitch.ts's call path uses.
 *
 * Same inputs as `poseFor` plus `playerId`, which keys the per-character
 * crossfade state `rig3d/animator.ts` keeps (`poseFor` has no notion of a
 * previous frame, so it cannot crossfade at all). Everything the mixer path
 * adds over `poseFor` is listed in `rig3d/pose_table.ts`'s `POSE_ACTIONS`.
 *
 * The torso lean is NOT one of those differences: both paths call the same
 * `applyLean` under the same `forOptions` gate, so `leanTilt`'s tuning is
 * shared rather than duplicated and the A/B parity suite can compare the two
 * at a non-zero `lean`.
 */
export function mixerPoseFor(
  playerId: string,
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now: number,
): actionPose.MutablePose {
  const posed = animator.poseFor(playerId, view, opts, now);
  // The SAME torso lean `poseFor` applies, from the same `applyLean`, under
  // the same `forOptions` gate and in the same position in the order (after
  // the whole-body root overlay, which `animator.poseFor` applies as its last
  // step). One implementation, not two tunings that have to agree -- see the
  // TORSO LEAN section above, whose derivation rests on `ppmForRadius` and
  // `HEIGHT_IN_RADII` and therefore belongs to this file rather than to
  // rig3d.
  if (actionPose.forOptions(opts) === null) {
    applyLean(posed, view?.lean ?? 0, opts.facing);
  }
  return posed;
}

/**
 * Drops every character's animation state -- the per-`playerId` stance
 * crossfade, the mask each in-flight action was engaged with, and the
 * timestamp the frame delta is derived from.
 *
 * WIRED AT EVERY `viewState.reset()` SITE (`screens/match.ts`, and
 * `benchmark.ts` in this package). That is a settled decision, not a default:
 * #425 introduced genuinely new per-player state where `poseFor` had been a
 * pure function of `(view, opts, now)`, so it introduced a new thing that can
 * be stale, and leaving that to self-correct would be a judgement call every
 * future reader has to re-make.
 *
 * Why exactly those sites and not a subset: every one of them already resets
 * `viewState` AND `cameraFollow` together, because each is a point where the
 * scene stops being continuous with the frame before it -- a fresh host, a
 * replay jumping the scene back in time, a rollback re-seed, a match going
 * terminal. `match.ts`'s own comment at the goal-replay site puts it exactly
 * right: "the scene jumps back in time, but view state must not carry the
 * post-goal kickoff pose into it." A stance crossfade is that same kind of
 * state -- derived presentation continuity, meaningless across the cut -- so
 * it resets with the other two rather than being reasoned about separately.
 *
 * Not resetting would have been SURVIVABLE: a stale stance fades out within
 * its own crossfade (0.24s at the longest) and a stale timestamp is capped by
 * the animator's frame clamp, so nothing gets stuck. Survivable is not the bar
 * for state that is trivially resettable at a boundary that already exists.
 */
export function resetAnimation(): void {
  animator.reset();
}

// The player id `draw`/`renderToSprite` animate under. Those two are
// parity/diagnostic entry points with no roster behind them (see their doc
// comments), but the animator still needs a key to hang a crossfade off, and
// giving them a fixed one keeps a standalone character preview animating
// continuously instead of re-snapping its stance every call.
const PREVIEW_PLAYER_ID = "__preview";

// ---------------------------------------------------------------------------
// GPU-adjacent: mesh construction, skeleton posing, per-character camera.
// Untested in this milestone (no WebGL context).
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
  // Ground-contact probes (#446), built from the SAME `body.accumulate`
  // output the mesh above is uploaded from, so the geometry that is grounded
  // and the geometry that is drawn cannot describe different figures.
  readonly groundProbes: ground.GroundProbes;
}

interface TeamMaterials {
  // ONE material for the whole character, and since #447 one for every
  // character too -- see `sharedMaterial` and this file's LIGHTING header for
  // why this used to be three per character, then two per process, and is now
  // one.
  readonly material: THREE.MeshStandardMaterial;
  readonly color: THREE.BufferAttribute;
}

// ---------------------------------------------------------------------------
// CHARACTER VARIANTS (#447)
// ---------------------------------------------------------------------------
//
// WHAT WAS WRONG. `build()` took no arguments, read `themes.LIST[0]` and
// `themes.FIGURES[0]`, and memoised the result in a module-level `built`. One
// geometry, for the whole process, for every player -- so `characterMesh`
// could not vary a character no matter what the per-frame path handed it.
// Medieval Fantasy's authored loadout is `{ right: "tournament_sword", left:
// "heater_shield" }`, so EVERY player on the pitch carried a sword and a
// shield, keepers included, while `gc-data` said in a comment AND enforced in
// a test that keepers carry nothing.
//
// WHAT REPLACES IT. A cache keyed by the VISUAL VARIANT -- `(theme, figure,
// loadout)` -- built lazily, on demand, one entry per distinct combination
// the roster actually contains. Nothing is built eagerly: a match that fields
// no toybox player never builds toybox geometry.
//
// EACH VARIANT NEEDS ITS OWN GROUND PROBES, and that is not incidental.
// `rig3d/ground.ts`'s probes are derived FROM the vertices (`probesFrom`
// groups every vertex by bone with no filtering), so a variant carrying a
// shield has probes that include the shield and a variant carrying nothing
// does not. Sharing one probe set across variants would ground a keeper
// against equipment they are not rendering -- which is #446's measurement
// taken over the wrong figure, and the precise reason `ground_contact.spec.ts`
// keeps `body` and `props` as separate columns.
//
// THE SKELETON STAYS SHARED, because the content says so: all six authored
// presentations are `RigId::RigMedium`, and `themes.ts`'s own figure note
// says the skeleton is "byte-identical across all three -- same bone names,
// same bone lengths, and the same clips drive all of them". So every variant
// is built from `RIG_PROPORTIONS`, every variant's bone list comes from
// `skeleton.bones(RIG_PROPORTIONS)`, and `rig3d/clips.ts` and
// `rig3d/animator.ts` are untouched by any of this.
//
// COST, STATED RATHER THAN ASSUMED. Across the thirteen authored players
// there are twelve distinct variants; across the ten who take the pitch in
// the two fixture teams, nine. So this trades one geometry for up to nine
// live ones -- see the PR for the measured per-variant vertex count and build
// time. `presentation_content.spec.ts` pins the count so it cannot grow
// silently.

/** One distinct character geometry: a theme, a figure, and a loadout. */
interface CharacterVariant {
  /** `theme|figure|loadout`, the cache key. */
  readonly key: string;
  readonly theme: themes.Theme;
  readonly figure: themes.Figure;
  readonly loadout: themes.Loadout;
}

/**
 * The variant a `PlayerRenderOptions` asks for.
 *
 * THE PREVIEW DEFAULT IS NAMED, NOT INFERRED. With no `presentation_id` the
 * caller is not on the product path at all -- `draw`/`renderToSprite`, a
 * fixture, a bench -- and gets `themes.LIST[0]` carrying its OWN authored
 * loadout, which is exactly what every caller got before #447. That keeps the
 * diagnostic entry points and this package's own suites rendering the
 * character they were written against.
 *
 * WITH a `presentation_id` the authored content decides everything, and an
 * absent `loadout_id` means the empty loadout -- the keeper rule. The two
 * cases are separated on `presentation_id` precisely so a keeper (authored to
 * carry nothing) can never be confused with an unwired caller (nothing
 * authored at all); collapsing them is how the defect would come back.
 *
 * AN EMPTY STRING IS NOT "UNWIRED", AND IS REFUSED. `""` reaching here would
 * take the preview branch and hand the player `themes.LIST[0]`'s full
 * sword-and-shield loadout -- a keeper silently armed again, which is the
 * exact defect #447 exists to remove and the exact silent substitution this
 * file's header and #415 forbid. `encode_roster` asserts the id non-empty and
 * `decodeRoster` refuses an empty one, so this is the third of three
 * independent checks rather than the only one; it is here because it is the
 * last place before the geometry, and the one a hand-built frame that never
 * crossed the wire still passes through.
 */
function variantFor(opts: PlayerRenderOptions): CharacterVariant {
  const figure = themes.FIGURES[0];
  if (figure === undefined) {
    throw new Error("player_renderer_3d.ts: no rig3d figure content available");
  }
  const presentationId = opts.presentation_id;
  if (presentationId === "") {
    throw new Error(
      "player_renderer_3d.ts: empty presentation_id -- an empty id is a broken roster, not an " +
        "unwired caller, and must not resolve to the preview default's sword and shield (#447)",
    );
  }
  const theme =
    presentationId === undefined ? themes.LIST[0] : presentationContent.themeFor(presentationId);
  if (theme === undefined) {
    throw new Error("player_renderer_3d.ts: no rig3d theme content available");
  }
  const loadout =
    presentationId === undefined ? theme.loadout : presentationContent.loadoutFor(opts.loadout_id);
  return {
    key: `${theme.key}|${figure.key}|${presentationContent.loadoutKey(loadout)}`,
    theme,
    figure,
    loadout,
  };
}

// The variant `available()`, `ppmForRadius` and anything else with no player
// in hand resolves to. Built from an empty `PlayerRenderOptions`, so it is
// the same preview default `variantFor` documents above rather than a second
// definition of it.
function defaultVariant(): CharacterVariant {
  return variantFor({ is_keeper: false, controlled: false });
}

// Keyed `${variant.key}|${team}`: the baked `color` attribute depends on the
// variant's own vertex count AND on the team's resolved palette, so one entry
// per pair. See `materialsForTeam`.
const materialsByTeam = new Map<string, TeamMaterials>();

// ONE `MeshStandardMaterial` FOR EVERY CHARACTER, not one per team and not
// one per variant (#447).
//
// It used to be one per team, which was already one more than necessary: both
// were `new MeshStandardMaterial({ vertexColors: true })` with the same
// `applyCombinedCelShading` hook and no other property set, because
// everything that differs between two characters -- palette, shading family,
// theme -- lives in per-vertex attributes rather than on the material (see
// `materialsForTeam` and `build()`'s `materialFamily` note). Keying the
// materials cache by variant as well as team would have turned two instances
// into up to eighteen, all still identical, purely as a side effect of a
// change about GEOMETRY. Hoisting the material out instead makes the count
// one and independent of how many variants a match fields.
//
// Lazily created rather than constructed at module load, matching every other
// GPU-adjacent object here: constructing three.js materials at import time
// runs in every test file that transitively imports this module.
let sharedMaterialInstance: THREE.MeshStandardMaterial | undefined;

function sharedMaterial(): THREE.MeshStandardMaterial {
  if (sharedMaterialInstance === undefined) {
    // Base `.color` stays white so `vertexColors` passes the baked-per-vertex
    // palette colour through unmodified (three.js multiplies material colour
    // x vertex colour) -- `rig3d/cel_shader.ts`'s injected shading reads that
    // same `diffuseColor.rgb` for every family, metal and emissive included,
    // shading ALL three families from the one palette-colour varying (there
    // is no fixed accent colour for emissive -- it multiplies the resolved
    // palette colour by a facing-dependent brightness boost instead; see
    // `cel_shader.ts`'s `celShadingChunk`). `roughness`/`metalness` are not
    // set here: the injected shading never reads three.js's
    // `PhysicalMaterial` struct at all (see `applyCombinedCelShading`'s doc
    // comment), so they would be dead uniforms -- the metal/plain/emissive
    // distinction is entirely which branch `vMaterialFamily` takes at
    // runtime, a per-vertex value baked by `build()` above, not a material
    // property.
    //
    // ONE `MeshStandardMaterial`, not three per character either (draw-call
    // fix #2 -- see this file's LIGHTING header and `build()`'s own comment
    // on the `materialFamily` attribute this material's shading reads).
    // `applyCombinedCelShading` is `cel_shader.ts`'s per-vertex-branching
    // sibling of the per-family `applyCelShading` used elsewhere in that
    // module's own test suite: the same GLSL, selected at runtime off
    // `materialFamily` instead of at TypeScript-build time off which
    // `MeshStandardMaterial` instance this is.
    const material = new THREE.MeshStandardMaterial({ vertexColors: true });
    celShader.applyCombinedCelShading(material);
    sharedMaterialInstance = material;
  }
  return sharedMaterialInstance;
}
// Keyed by `CharacterVariant.key`. Never cleared, matching the singleton it
// replaces -- see `characterPool`'s note on lifetime.
const builtByVariant = new Map<string, BuiltCharacter>();
let failed = false;

// How many variant geometries this process has actually BUILT (not looked
// up). Monotonic, never reset.
//
// EXPORTED AS A GATE INSTRUMENT, not as a diagnostic. The cost this counts is
// invisible to `benchmark.ts`: that harness discards 300 warm-up frames
// (`warmup_frames`, 5 s at 60 Hz) and its header states outright that "mesh
// generation ... [is a] startup cost" measured separately -- an assumption
// that was correct when mesh generation was ONE sub-millisecond singleton
// build and stopped being correct the moment it started scaling with roster
// diversity. In production there is no five-second grace period: frame 1 is
// what the user sees when the pitch appears. A wall-clock bound would be
// flaky on CI; a COUNT is deterministic, and "no variant is built inside the
// draw loop once the roster is known" is the property that actually matters.
// See `player_renderer_3d_prewarm.spec.ts`.
let variantBuilds = 0;

/** How many distinct character geometries this process has built. Monotonic. */
export function variantBuildCount(): number {
  return variantBuilds;
}

function mat4ToThree(m: Mat4): THREE.Matrix4 {
  const out = new THREE.Matrix4();
  // `.set()` takes row-major arguments; `Mat4` is stored row-major (see
  // @gc/core/mat4.ts), so this is a direct copy, not a transpose.
  out.set(
    m[0],
    m[1],
    m[2],
    m[3],
    m[4],
    m[5],
    m[6],
    m[7],
    m[8],
    m[9],
    m[10],
    m[11],
    m[12],
    m[13],
    m[14],
    m[15],
  );
  return out;
}

// Builds the shared rig and ONE colour-free character mesh for ONE VARIANT
// (#447 -- see the CHARACTER VARIANTS section above). Called lazily, once per
// distinct `(theme, figure, loadout)` the roster actually asks for.
function build(variant: CharacterVariant = defaultVariant()): BuiltCharacter | undefined {
  const cached = builtByVariant.get(variant.key);
  if (cached !== undefined || failed) {
    return cached;
  }
  try {
    const rigProportions = RIG_PROPORTIONS;
    const height = proportions.height(rigProportions);
    const theme = variant.theme;
    const figure = variant.figure;
    const [partBuilder] = body.accumulate(rigProportions, theme, figure, variant.loadout);

    // GROUND CONTACT (#446), resolved before the rig this file poses exists.
    // `skeleton.bones(style)` IS the bone index order the vertices were baked
    // against (`skeleton.ts`'s BONE INDEX CONTRACT), so the probes resolve the
    // same vertex-to-bone mapping the skinning does. `restLift` is the rest
    // pose's own penetration -- 1.2 mm, a constant of the geometry -- and the
    // rig is raised by it once here rather than re-measured every frame; see
    // `rig3d/ground.ts`'s REST POSE section.
    const groundProbes = ground.probesFrom(
      rigProportions,
      partBuilder.verts,
      skeleton.bones(rigProportions).map((bone) => bone.name),
    );
    const rig = skeleton.raised(skeleton.newRig(rigProportions), groundProbes.restLift);

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
    const MATERIAL_ORDER = [
      geometry.MATERIAL.plain,
      geometry.MATERIAL.metal,
      geometry.MATERIAL.emissive,
    ] as const;
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

    // Draw-call fix #2: bake a per-vertex shading-family float
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
      // symptom -- not the render target's `flipY` (see `renderToSprite`'s
      // doc comment, which toggling had zero visible effect on, confirming
      // the bug was here, upstream of compositing entirely).
      bone.matrixAutoUpdate = false;
      bone.matrixWorldAutoUpdate = false;
      return bone;
    });
    const skeletonObj = new THREE.Skeleton(bones);

    // three.js's own SkinnedMesh materials come from the geometry groups
    // above; a per-team palette resolves at draw time (see `draw` below),
    // so the mesh itself starts colour-free (one build serves every team).
    const mesh = new THREE.SkinnedMesh(geom, new THREE.MeshStandardMaterial());
    mesh.add(bones[0] ?? new THREE.Bone());
    mesh.bind(skeletonObj);

    const character: BuiltCharacter = {
      rig,
      bones,
      mesh,
      height,
      paletteSlots,
      groundProbes,
    };
    builtByVariant.set(variant.key, character);
    variantBuilds += 1;
    return character;
  } catch (error) {
    failed = true;

    console.warn(
      `rigged 3D players disabled (build failed, variant ${variant.key}): ${String(error)}`,
    );
    return undefined;
  }
}

// Bakes a per-vertex palette-slot colour lookup into a
// `THREE.BufferAttribute("color")`: three.js's stock `MeshStandardMaterial`
// has no dynamic per-vertex uniform-array indexing (only a hand-written
// shader would, and the README marks that mechanism "replace"), but
// `vertexColors: true` reproduces the visible result -- every vertex shaded
// by its OWN resolved palette colour rather than one flat colour for the
// whole material group. This is what makes
// "skin" read differently from "cloth" (the team's `main` colour) on the
// SAME `plain`-material surface, and what makes the two teams distinguishable
// at all: the previous single `palette[0]` ("skin", never team-linked) used
// the same swatch for both sides regardless of `team`.
//
// `character` is passed in (not read from module state) because
// `paletteSlots` -- needed to bake the attribute -- lives on `BuiltCharacter`,
// produced by `build()`.
//
// KEYED BY VARIANT AS WELL AS TEAM SINCE #447, for two independent reasons,
// either of which alone would require it: the baked buffer is one colour per
// VERTEX and variants have different vertex counts, and the palette is
// resolved against the VARIANT'S OWN THEME (`themes.resolvedPalette(theme,
// team)`), which is what makes a sci-fi player read as sci-fi rather than as
// a medieval one wearing the same colours. This function used to read
// `themes.LIST[0]` directly -- a second hardcoded theme reference beside
// `build()`'s, and one that would have kept every character medieval even
// after `build()` learned to vary.
function materialsForTeam(
  character: BuiltCharacter,
  variant: CharacterVariant,
  team: "home" | "away",
): TeamMaterials {
  const key = `${variant.key}|${team}`;
  const cached = materialsByTeam.get(key);
  if (cached !== undefined) {
    return cached;
  }
  const theme = variant.theme;
  const teamData = themes.TEAMS[team === "away" ? 1 : 0];
  if (teamData === undefined) {
    throw new Error("player_renderer_3d.ts: no rig3d team content available");
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

  // The COLOUR is per (variant, team); the MATERIAL is not per anything --
  // see `sharedMaterial` for what used to be constructed here and why it
  // moved.
  const materials: TeamMaterials = { material: sharedMaterial(), color };
  materialsByTeam.set(key, materials);
  return materials;
}

/**
 * True when a rigged player can actually be drawn this frame.
 *
 * Answered against the preview default variant (see `variantFor`): the
 * question is whether the rigged pass works at all, and every variant shares
 * the rig, the clips and the geometry builder, so one build answers it.
 */
export function available(): boolean {
  return build() !== undefined;
}

// ---------------------------------------------------------------------------
// PRE-WARM (#447 follow-up)
// ---------------------------------------------------------------------------
//
// WHY THIS EXISTS. `build()` is lazy, and laziness alone is not a schedule.
// `pitch.draw` calls `characterMesh` synchronously for every rostered player
// on every rendered frame, and `characterMesh` calls `build(variant)` inline,
// so the FIRST frame of a match discovers all of the starting roster's
// distinct variants back to back in one synchronous call stack. Measured on
// the two fixture teams that is nine builds and ~58 ms -- about 3.5x the
// 60 fps budget and 1.7x `benchmark.ts`'s own `slow_frame_ms: 33` -- landing
// on the frame the user sees when the pitch appears.
//
// WHY PRE-WARMING AND NOT STAGGERING. The roster crosses the wire ONCE PER
// MATCH, not per frame (`crates/gc-render/src/frame_buffer.rs`'s
// `encode_roster` and `gc-wasm`'s `rosterNumeric`/`rosterIdsAndNames`
// accessors both say so, and both sim hosts memoise the decode). So the full
// set of distinct variants is knowable the moment a match's host exists,
// before any frame is drawn. Capping builds at one per frame would only
// spread the stutter over nine frames instead of removing it, and would leave
// a later first-sighting landing during live play regardless. Building them
// where the match is set up puts the cost in the screen transition, where a
// pause is expected and invisible.
//
// THE LAZY PATH STAYS, as the correctness backstop: a variant this never saw
// still builds on demand rather than failing to draw. The gate is that the
// common case never reaches it -- see
// `player_renderer_3d_prewarm.spec.ts`, which pins zero builds inside the
// draw loop once the roster is known.

/**
 * The slice of a decoded roster this needs. Declared structurally rather than
 * imported from `pitch.ts`, which imports THIS module -- and every field is
 * optional so a caller holding a roster that carries none of them (the fake
 * hosts in `@gc/screens`' own suites return `{}`) is a no-op rather than an
 * error.
 */
export interface PrewarmRoster {
  readonly ids?: readonly (string | undefined)[];
  readonly presentation_ids?: readonly (string | undefined)[];
  readonly loadout_ids?: readonly (string | undefined)[];
  readonly teams?: readonly ("home" | "away")[];
}

/** What one `prewarmCharacters` call did, for tests and diagnostics. */
export interface PrewarmResult {
  /** Distinct `(variant, team)` pairs the roster asked for. */
  readonly variants: number;
  /** How many variant geometries had to be built (the rest were cached). */
  readonly built: number;
  /** How many per-player pooled meshes were created. */
  readonly pooled: number;
}

/**
 * Impure: builds every distinct character variant a roster will ask for, plus
 * each one's per-team geometry and baked palette, so the first drawn frame
 * finds all of them cached.
 *
 * Call this when a match's roster becomes known and BEFORE the first
 * `pitch.draw` — `@gc/screens`' `MatchScreen` does it at construction and at
 * every restart. Idempotent and cheap to repeat: everything it touches is
 * memoised, so a second call on the same roster builds nothing.
 *
 * Throws on content ids the renderer does not know, exactly as the draw path
 * would — better here, at match setup, than mid-match on the frame that
 * player first appears.
 */
export function prewarmCharacters(roster: PrewarmRoster): PrewarmResult {
  const presentationIds = roster.presentation_ids;
  // `Array.isArray`, not just an `undefined` check: `@gc/screens`' own
  // `RenderFrameRoster` is `Readonly<Record<string, unknown>>` so every fake
  // host in that package satisfies it, and `MatchScreen` pre-warms
  // unconditionally at construction. A roster that carries this key as
  // something other than an array is a fixture, not a match, and must be a
  // no-op rather than a crash in a hundred unrelated tests.
  if (!Array.isArray(presentationIds)) {
    return { variants: 0, built: 0, pooled: 0 };
  }
  // `Array.isArray` is typed `(arg: any) => arg is any[]`, so narrowing
  // through it discards the declared element type and everything read out of
  // it below becomes `any`. Re-bind at the declared type; the runtime check
  // above is the same one, unchanged.
  const ids: readonly (string | undefined)[] = presentationIds;
  const before = variantBuilds;
  const seen = new Set<string>();
  let pooled = 0;
  for (let index = 0; index < ids.length; index += 1) {
    const presentationId = ids[index];
    if (presentationId === undefined) {
      continue;
    }
    const team = roster.teams?.[index] ?? "home";
    const loadoutId = roster.loadout_ids?.[index];
    const variant = variantFor({
      is_keeper: false,
      controlled: false,
      presentation_id: presentationId,
      ...(loadoutId !== undefined ? { loadout_id: loadoutId } : {}),
    });
    seen.add(`${variant.key}|${team}`);
    const character = build(variant);
    if (character === undefined) {
      // `build` already logged and latched `failed`; there is nothing to warm
      // and `characterMesh` will decline for the same reason.
      break;
    }
    // The per-(variant, team) colour bake is the OTHER cost the first frame
    // used to pay — one `Float32Array(vertCount * 3)` filled per slot — so it
    // is warmed here too rather than left for `characterMesh` to discover.
    geometryForTeam(character, variant, team);
    // AND THE PER-PLAYER MESH, which is the rest of what the first frame
    // used to pay for. `pooledCharacter` allocates a fresh `THREE.Bone` list,
    // a `THREE.Skeleton` and a `SkinnedMesh` wrapper per player id — cheap
    // next to a geometry build, but ten of them still measured ~19 ms on the
    // first frame once the geometry was warm, which is most of a 60 Hz
    // budget on its own. Warmed only when the roster names its players; a
    // roster without `ids` still gets its geometry warmed and pays this on
    // first sight.
    const playerId = roster.ids?.[index];
    if (playerId !== undefined && playerId !== "") {
      pooledCharacter(playerId, character, variant, team);
      pooled += 1;
    }
  }
  return { variants: seen.size, built: variantBuilds - before, pooled };
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
// team, not a deep clone of the mesh. Built lazily, cached forever, mirroring
// `materialsByTeam`'s own cache lifetime.
//
// KEYED `${variant.key}|${team}` SINCE #447: the shared attributes it
// references belong to ONE variant's geometry, so a single per-team entry
// would hand a sci-fi player the medieval positions of whichever variant got
// there first. The cache is now bounded by (variants on the pitch x 2)
// instead of by 2 -- nine and eighteen respectively for the two fixture
// teams.
const teamGeometry = new Map<string, THREE.BufferGeometry>();

function geometryForTeam(
  character: BuiltCharacter,
  variant: CharacterVariant,
  team: "home" | "away",
): THREE.BufferGeometry {
  const cacheKey = `${variant.key}|${team}`;
  const cached = teamGeometry.get(cacheKey);
  if (cached !== undefined) {
    return cached;
  }
  const base = character.mesh.geometry;
  const position = base.getAttribute("position");
  const normal = base.getAttribute("normal");
  const skinIndex = base.getAttribute("skinIndex");
  const skinWeight = base.getAttribute("skinWeight");
  const materialFamily = base.getAttribute("materialFamily");
  if (
    position === undefined ||
    normal === undefined ||
    skinIndex === undefined ||
    skinWeight === undefined ||
    materialFamily === undefined
  ) {
    throw new Error(
      "player_renderer_3d.ts: shared character geometry is missing a required attribute",
    );
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
  geom.setAttribute("color", materialsForTeam(character, variant, team).color);
  teamGeometry.set(cacheKey, geom);
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
//
// KEYED `${playerId}|${variant.key}` SINCE #447, not by `playerId` alone. The
// pooled `SkinnedMesh` wraps ONE variant's geometry, so a player id that ever
// resolved to two variants would keep being handed the first one's mesh --
// which is the original defect in miniature. A player's variant is constant
// for a match in practice (presentation and loadout are authored, not
// per-frame), so this adds a cache entry only where something really did
// change; the cost of getting it wrong is a character silently frozen in the
// wrong kit, which is not a cost worth taking to save a map entry.
const characterPool = new Map<string, PooledCharacter>();

function pooledCharacter(
  playerId: string,
  character: BuiltCharacter,
  variant: CharacterVariant,
  team: "home" | "away",
): PooledCharacter {
  const poolKey = `${playerId}|${variant.key}`;
  const cached = characterPool.get(poolKey);
  if (cached !== undefined) {
    return cached;
  }
  const bones = buildCharacterBones();
  const skeletonObj = new THREE.Skeleton(bones);
  const mesh = new THREE.SkinnedMesh(
    geometryForTeam(character, variant, team),
    new THREE.MeshStandardMaterial(),
  );
  mesh.add(bones[0] ?? new THREE.Bone());
  mesh.bind(skeletonObj);
  // DETACHED bind mode -- a real defect found live (characters rendered
  // nowhere visible, not merely mis-sized): three.js's
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
  characterPool.set(poolKey, pooled);
  return pooled;
}

/**
 * Pixels-per-metre uniform scale for a character rendered at on-screen
 * radius `r` (pixels -- `r = radius * scale`, see pitch.ts's `playerAnchor`).
 * Mirrors `prepareCharacter`'s own `ppm` derivation exactly, so a rigged
 * player reads at the same visual weight it did as an offscreen-rendered
 * sprite. `undefined` when the rigged pass is unavailable (`build()` failed),
 * matching `characterMesh`'s own contract below -- which pitch.ts now turns
 * into a thrown error rather than a quiet downgrade (#415).
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
 * `character.mesh` as it existed before single-pass compositing, minus the
 * per-character camera/scene/render-target work `renderToSprite` layers on
 * top (see this file's header, "THE EXACT SIGNATURE THIS FILE IS WRITTEN TO EXPECT", which
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
 * Returns `undefined` when the rigged pass is unavailable (`build()` failed),
 * same contract as `renderToSprite`. The ONE caller on the product path
 * (`pitch.draw`) treats that as fatal and throws (#415, AGENTS.md §7): the
 * failures `build()` catches are programmer errors, and there is no second
 * renderer left to fall back to.
 */
export function characterMesh(
  playerId: string,
  view: PlayerView | undefined,
  opts: PlayerRenderOptions,
  now: number,
): THREE.Object3D | undefined {
  const variant = variantFor(opts);
  const character = build(variant);
  if (character === undefined) {
    return undefined;
  }
  const team = opts.team ?? "home";
  const pooled = pooledCharacter(playerId, character, variant, team);

  const pose = mixerPoseFor(playerId, view, opts, now);
  // `ground.poseAndGround` rather than `skeleton.apply`: same evaluation, plus
  // the lift a root rotation needs so no limb ends under the turf (#446).
  ground.poseAndGround(character.rig, pose, character.groundProbes);
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
  const geometry = geometryForTeam(character, variant, team);
  if (pooled.mesh.geometry !== geometry) {
    pooled.mesh.geometry = geometry;
  }
  // A single Material (not an array) -- see `materialsForTeam`'s doc comment
  // on why that is what makes this one draw call instead of up to three.
  const teamMaterials = materialsForTeam(character, variant, team);
  pooled.mesh.material = teamMaterials.material;

  return pooled.mesh;
}

/**
 * Character camera placement: a small orthographic camera framing the
 * character so it lands at `(sx, sy)` on screen at `ppm` pixels-per-metre.
 * See this file's header scope note -- new code, not a port of unseen GLSL.
 */
export function characterCameraParams(
  sx: number,
  sy: number,
  ppm: number,
  vw: number,
  vh: number,
  elevation: number,
  height: number,
) {
  const halfHeightMetres = height / 2;
  return {
    // Looks down at `elevation` from slightly above and behind the
    // character's mid-height, along -Z (the character's own facing is
    // applied to the mesh's world transform instead of the camera).
    eye: [
      0,
      halfHeightMetres + Math.sin(elevation) * height,
      Math.cos(elevation) * height,
    ] as const,
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
// lights below), reused every draw call (one draw call per character).
// `scene.clear()` in `prepareCharacter`
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
  const variant = variantFor(opts);
  const character = build(variant);
  if (character === undefined) {
    return undefined;
  }
  const pose = mixerPoseFor(PREVIEW_PLAYER_ID, view, opts, now);
  // Grounded exactly as `characterMesh` above -- the two paths must not
  // disagree about where the pitch is (#446).
  ground.poseAndGround(character.rig, pose, character.groundProbes);
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

  const teamMaterials = materialsForTeam(character, variant, opts.team ?? "home");
  character.mesh.material = teamMaterials.material;
  // The `color` attribute is swapped per draw call, on this VARIANT's own
  // geometry (see `TeamMaterials`'s doc comment) -- there is exactly one
  // character mesh in flight at a time (`scene.clear()` above), so this
  // cannot race between two teams' colours within a frame.
  character.mesh.geometry.setAttribute("color", teamMaterials.color);

  const ppm = (r * HEIGHT_IN_RADII * 2) / character.height;
  const far = character.height * 4 + 10;
  const params = characterCameraParams(sx, sy, ppm, vw, vh, ELEVATION, character.height);
  const cam = new THREE.OrthographicCamera(
    params.left,
    params.right,
    params.top,
    params.bottom,
    0.01,
    far,
  );
  cam.position.set(...params.eye);
  cam.lookAt(...params.target);
  cam.updateProjectionMatrix();

  scene.clear();
  scene.add(character.mesh, keyLight, keyLight.target, fillLight);
  return { character, cam };
}

/**
 * Impure: draws one rigged player -- one immediate `renderer.render()` call
 * per character, into whatever target `renderer` is currently bound to. The
 * caller is responsible for `renderer.autoClear = false` across a frame's players (so
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
 * VERIFIED WITH A LIVE GL CONTEXT: the returned mesh's
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
    // VERIFIED WITH A LIVE GL CONTEXT: the character
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

    console.warn(`rigged 3D players disabled (renderToSprite failed): ${String(error)}`);
    return undefined;
  }
}
