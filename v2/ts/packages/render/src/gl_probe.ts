// Replaces game/render/gl_probe.lua (v2/README.md #7: "gl_probe.lua ->
// replace -- three.js capability detection").
//
// The Lua original's 453 lines exist entirely to answer a question three.js
// answers directly, for reasons specific to love.js:
//
//   - `gl_probe.survey()` asks the runtime what canvas formats/GL limits it
//     supports by READING tables (`love.graphics.getCanvasFormats()`,
//     `getSystemLimits()`), because in love.js *asking by trying* (creating
//     a canvas of an unsupported format) does not raise a catchable Lua
//     error -- it aborts the whole process (#360). `WebGLRenderer` already
//     interrogates its `WebGL2RenderingContext` at construction and exposes
//     the result synchronously as `renderer.capabilities` (with real
//     properties, not a stringified marker line) and `renderer.getContext()`
//     for anything not surfaced there; there is nothing left to probe for.
//   - `gl_probe.attempt()`/`attemptOnce()`/the pipe-delimited `marker()`
//     encoding exist to get that answer OUT of a headless love.js process
//     through its only channel (the browser console), one canvas creation
//     per process because a second attempt could be lost to the same abort.
//     A real browser JS exception is always catchable and `capabilities` is
//     a plain object a test or a log line can read directly -- no transport
//     encoding is needed.
//   - `gl_probe.SHADER_LADDER` bisects why a hand-written LÖVE/GLSL-ES-1.00
//     shader fails to compile/link in a specific browser (#391). `rig3d`'s
//     materials are three.js's own (`MeshStandardMaterial`), already portable.
//     bloom.ts DOES hand-write three small post-process shaders again since
//     #404 -- three.js's `UnrealBloomPass` turned out not to be the algorithm
//     the Lua constants describe -- but they are `THREE.ShaderMaterial`s, so
//     three.js supplies the version directive, precision qualifiers and
//     attribute/uniform preamble that the Lua ladder existed to bisect, and a
//     compile failure surfaces as a normal, catchable browser console error
//     with the full info log. The ladder still has nothing to add.
//   - `gl_probe.budget()`/`MIN_VERTEX_UNIFORM_VECTORS` computed whether
//     rig3d's hand-packed bone-uniform array fit inside WebGL1's guaranteed
//     128-vector floor. three.js skins via a bone matrix texture (or a
//     uniform array it sizes itself against the real
//     `capabilities.maxVertexUniforms`), so there is no budget for content
//     code to compute.
//
// What replaces all of it: a thin, typed read of `WebGLRenderer.capabilities`
// (plus the handful of `WebGL2RenderingContext` queries it does not surface),
// for whatever a diagnostics/log screen wants to display. No survey/attempt
// split, no marker transport, no shader ladder -- untested (needs a real
// `WebGLRenderer`), but there is very little left to test: this module reads,
// it does not decide.

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
