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

export type * as cameraTypes from "./camera.ts";
export type * as cameraFollowTypes from "./camera_follow.ts";
export type * as viewStateTypes from "./view_state.ts";
export type * as correctionSmoothingTypes from "./correction_smoothing.ts";
export type * as replayTypes from "./replay.ts";
export type * as releaseFollowTypes from "./release_follow.ts";
export type * as benchmarkTypes from "./benchmark.ts";
