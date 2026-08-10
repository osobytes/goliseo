// The rigged-player presentation: skeleton, clips, whole-body action poses,
// theming, and per-part character content. Ported from game/render/rig3d/
// (see v2/README.md #7 for the mechanism/content split this followed).
//
// Each submodule is re-exported under its own namespace, mirroring the one
// Lua-file-per-TS-file mapping (v2/README.md #4) and avoiding name collisions
// -- `build` exists on three different submodules here, exactly as
// `headgear.build` / `equipment.build` / (a future) `body.build` did in Lua.

export * as palette from "./palette.ts";
export * as proportions from "./proportions.ts";
export * as masks from "./masks.ts";
export * as themes from "./themes.ts";
export * as skeleton from "./skeleton.ts";
export * as clips from "./clips.ts";
export * as actionPose from "./action_pose.ts";
// The #425 playback layer: the explicit `PlayerPoseId -> action` table
// (`poseTable`), the `THREE.AnimationMixer` wrapper that plays it (`mixer`),
// and the per-character composer that drives both (`animator`).
export * as poseTable from "./pose_table.ts";
export * as mixer from "./mixer.ts";
export * as animator from "./animator.ts";
export * as speciesPresentation from "./species_presentation.ts";
export * as geometry from "./geometry.ts";
export * as ground from "./ground.ts";
export * as face from "./face.ts";
export * as headgear from "./headgear.ts";
export * as equipment from "./equipment.ts";
export * as body from "./body.ts";
export * as poseLod from "./pose_lod.ts";
export * as celShader from "./cel_shader.ts";
