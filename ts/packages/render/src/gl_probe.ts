// A thin, typed read of `WebGLRenderer.capabilities` (plus the handful of
// `WebGL2RenderingContext` queries it does not surface), for whatever a
// diagnostics/log screen wants to display.
//
// This can stay this small because `WebGLRenderer` already interrogates its
// `WebGL2RenderingContext` at construction and exposes the result
// synchronously as `renderer.capabilities` (with real properties, not a
// stringified marker line) and `renderer.getContext()` for anything not
// surfaced there:
//
//   - There is no need to probe by trial-and-error (creating a canvas of an
//     unsupported format and seeing what happens): reading the capability
//     values directly is enough, and trial-and-error is worth avoiding
//     anyway, since in at least one runtime that kind of probe did not raise
//     a catchable error -- it aborted the whole process (#360).
//   - There is no hand-written shader on this path to bisect for a
//     browser-specific compile/link failure (#391): `bloom.ts` and `rig3d`'s
//     materials use three.js's own, already-portable
//     `MeshStandardMaterial`/`UnrealBloomPass` shaders.
//   - There is no uniform-count budget for content code to compute: three.js
//     skins via a bone matrix texture (or a uniform array it sizes itself
//     against the real `capabilities.maxVertexUniforms`).
//
// Untested (needs a real `WebGLRenderer`), but there is very little left to
// test: this module reads, it does not decide.

import * as THREE from "three";

export interface GlProbeReport {
  readonly isWebGL2: boolean;
  readonly maxTextures: number;
  readonly maxVertexTextures: number;
  readonly maxTextureSize: number;
  readonly maxCubemapSize: number;
  readonly maxAttributes: number;
  readonly maxVertexUniforms: number;
  readonly maxVaryings: number;
  readonly maxFragmentUniforms: number;
  readonly vertexTextures: boolean;
  readonly precision: string;
  readonly logarithmicDepthBuffer: boolean;
  readonly reversedDepthBuffer: boolean;
}

/** Reads `renderer`'s already-resolved capabilities. Untested -- see file header. */
export function report(renderer: THREE.WebGLRenderer): GlProbeReport {
  const c = renderer.capabilities;
  return {
    isWebGL2: c.isWebGL2,
    maxTextures: c.maxTextures,
    maxVertexTextures: c.maxVertexTextures,
    maxTextureSize: c.maxTextureSize,
    maxCubemapSize: c.maxCubemapSize,
    maxAttributes: c.maxAttributes,
    maxVertexUniforms: c.maxVertexUniforms,
    maxVaryings: c.maxVaryings,
    maxFragmentUniforms: c.maxFragmentUniforms,
    vertexTextures: c.vertexTextures,
    precision: c.precision,
    logarithmicDepthBuffer: c.logarithmicDepthBuffer,
    reversedDepthBuffer: c.reversedDepthBuffer,
  };
}
