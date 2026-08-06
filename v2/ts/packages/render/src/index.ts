// three.js renderer: rigs, pitch, effects, HUD, camera.
//
// Values are re-exported by name. Types are namespaced per module rather than
// flattened, because several modules legitimately declare shapes with the same
// generic names — `replay.ts` and `@gc/presentation` both have a `MatchState`,
// and `Rect` appears in three places. Flattening them with `export *` would
// either collide or, worse, silently resolve to whichever module the bundler
// reached first.

export * as rig3d from "./rig3d/index.ts";

export { camera } from "./camera.ts";
export { cameraFollow } from "./camera_follow.ts";
export { viewState } from "./view_state.ts";
export { correctionSmoothing } from "./correction_smoothing.ts";
export { replay } from "./replay.ts";
export { releaseFollow } from "./release_follow.ts";
export { Benchmark, GATES, DELTA_BUDGET, evaluate, emit } from "./benchmark.ts";

// The scene-assembly surface: pitch.ts already composes arena + players +
// effects + combat (see its own file header), match_hud.ts draws the HUD,
// bloom.ts post-processes, and scene.ts (this wave's addition) is the root
// that owns the THREE.Scene/WebGLRenderer and assembles all of it into a
// frame -- see scene.ts's header for the assembly order and its known gaps.
export { pitch, pitchDrawCommands, resetStaticSceneCache, staticSceneBuildCount } from "./pitch.ts";
export { backdropCommands, frameCommands, drawBackdrop, drawFrame } from "./arena.ts";
export {
  DEFAULT_PLAYER_RADIUS,
  metresPerWorldUnit,
  clipFor,
  poseFor,
  characterCameraParams,
  available as player3dAvailable,
  draw as player3dDraw,
} from "./player_renderer_3d.ts";
export { effects } from "./effects.ts";
export { Bloom, DEFAULT_BLOOM_CONFIG } from "./bloom.ts";
export { drawUnderCommands, drawOverCommands, drawUnder, drawOver } from "./combat.ts";
export { matchHudCommands, drawMatchHud } from "./match_hud.ts";
export { SceneRoot } from "./scene.ts";

export type * as cameraTypes from "./camera.ts";
export type * as cameraFollowTypes from "./camera_follow.ts";
export type * as viewStateTypes from "./view_state.ts";
export type * as correctionSmoothingTypes from "./correction_smoothing.ts";
export type * as replayTypes from "./replay.ts";
export type * as releaseFollowTypes from "./release_follow.ts";
export type * as benchmarkTypes from "./benchmark.ts";
export type * as pitchTypes from "./pitch.ts";
export type * as arenaTypes from "./arena.ts";
export type * as player3dTypes from "./player_renderer_3d.ts";
export type * as effectsTypes from "./effects.ts";
export type * as bloomTypes from "./bloom.ts";
export type * as combatTypes from "./combat.ts";
export type * as matchHudTypes from "./match_hud.ts";
export type * as sceneTypes from "./scene.ts";

// NOT YET ON THE PACKAGE SURFACE: draw2d.ts (DrawCommand/DrawList/paint --
// internal plumbing every module above already re-exposes the parts of it
// callers need, e.g. RGB via each module's own types) and gl_probe.ts
// (capability reporting, not part of this wave's "assemble a frame" scope).
// Left out deliberately rather than silently; see this port's report.
