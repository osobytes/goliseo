// three.js renderer: rigs, pitch, effects, HUD, camera.
//
// Values are re-exported by name. Types are namespaced per module rather than
// flattened, because several modules legitimately declare shapes with the same
// generic names — `replay.ts` and `@gc/presentation` both have a `MatchState`,
// and `Rect` appears in three places. Flattening them with `export *` would
// either collide or, worse, silently resolve to whichever module the bundler
// reached first.

export * as rig3d from "./rig3d/index.ts";

// The consumer half of the RenderFrame wire format -- the words the Rust
// producer writes into wasm linear memory arrive here. Namespaced rather than
// flattened: `decode`, `LAYOUT_VERSION` and `MAGIC` are far too generic to sit
// at a package root. Anything driving the sim needs these to read a frame at
// all, so leaving them off the surface makes the package unusable from
// outside; that was an oversight, not a deliberate exclusion.
export * as frameBuffer from "./frame_buffer.ts";
export type * as frameBufferTypes from "./frame_buffer.ts";

export { camera } from "./camera.ts";
export { cameraFollow } from "./camera_follow.ts";
export { viewState } from "./view_state.ts";
export { correctionSmoothing } from "./correction_smoothing.ts";
export { replay } from "./replay.ts";
export { releaseFollow } from "./release_follow.ts";
export { dispossessionFlinch } from "./dispossession_flinch.ts";
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
  DECLARED_PLAYER_HEIGHT_M,
  METRES_PER_WORLD_UNIT,
  ELEVATION as player3dElevation,
  metresPerWorldUnit,
  clipFor,
  poseFor,
  mixerPoseFor as player3dMixerPoseFor,
  resetAnimation as player3dResetAnimation,
  // #447. `@gc/screens`' `MatchScreen` calls the pre-warm when a match's host
  // is created, so the distinct character variants a roster asks for are
  // built during the screen transition rather than all at once inside the
  // first drawn frame. `player3dVariantBuildCount` is the gate instrument
  // that pins it -- the same shape as `staticSceneBuildCount` above, and for
  // the same reason: a count is deterministic where a wall clock is flaky.
  prewarmCharacters as player3dPrewarmCharacters,
  variantBuildCount as player3dVariantBuildCount,
  characterCameraParams,
  ppmForRadius as player3dPpmForRadius,
  characterMesh as player3dCharacterMesh,
  available as player3dAvailable,
  draw as player3dDraw,
  renderToSprite as player3dRenderToSprite,
} from "./player_renderer_3d.ts";
export { effects } from "./effects.ts";
export { Bloom, DEFAULT_BLOOM_CONFIG } from "./bloom.ts";
export { drawUnderCommands, drawOverCommands, drawUnder, drawOver } from "./combat.ts";
export { matchHudCommands, drawMatchHud } from "./match_hud.ts";
export { SceneRoot } from "./scene.ts";
// The world-space coliseum stadium (bowl, crowd, sky, goals) rendered as
// `SceneRoot`'s world layer -- see scene.ts's `setWorldLayer` and stadium.ts's
// own header.
export { Stadium } from "./stadium.ts";

export type * as cameraTypes from "./camera.ts";
export type * as cameraFollowTypes from "./camera_follow.ts";
export type * as viewStateTypes from "./view_state.ts";
export type * as correctionSmoothingTypes from "./correction_smoothing.ts";
export type * as replayTypes from "./replay.ts";
export type * as releaseFollowTypes from "./release_follow.ts";
export type * as dispossessionFlinchTypes from "./dispossession_flinch.ts";
export type * as benchmarkTypes from "./benchmark.ts";
export type * as pitchTypes from "./pitch.ts";
export type * as arenaTypes from "./arena.ts";
export type * as player3dTypes from "./player_renderer_3d.ts";
export type * as effectsTypes from "./effects.ts";
export type * as bloomTypes from "./bloom.ts";
export type * as combatTypes from "./combat.ts";
export type * as matchHudTypes from "./match_hud.ts";
export type * as sceneTypes from "./scene.ts";
export type * as stadiumTypes from "./stadium.ts";

// NOT YET ON THE PACKAGE SURFACE: draw2d.ts (DrawCommand/DrawList/paint --
// internal plumbing every module above already re-exposes the parts of it
// callers need, e.g. RGB via each module's own types) and gl_probe.ts
// (capability reporting, not part of this wave's "assemble a frame" scope).
// Left out deliberately rather than silently.
