#!/usr/bin/env node
/* Unit coverage for the #391 varying hoist.
 *
 * The hoist is a pure string transform applied to EVERY shader the browser
 * build ever compiles, so the cases that must NOT be touched matter at least as
 * much as the one that must: a transform that rewrites a commented-out
 * declaration, or a `varying` inside a disabled `#ifdef`, breaks shaders that
 * work today in every browser.
 *
 * The final line is a machine-readable terminator. scripts/check_shader_hoist.sh
 * requires it AND a zero exit AND a minimum case count, because a runner that
 * prints failures and exits 0 is exactly the shape AGENTS.md section 9 forbids.
 */
"use strict";

const fs = require("fs");
const vm = require("vm");

const source = fs.readFileSync(require.resolve("./browser_shader_hoist.js"), "utf8");
const window = {};
vm.runInNewContext(source, { window });
const hoist = window.GoliseoShaderHoist.hoist;
const install = window.GoliseoShaderHoist.install;
const MARKER = window.GoliseoShaderHoist.marker;

let passed = 0;
const failures = [];

function check(name, body) {
  try {
    body();
    passed += 1;
  } catch (error) {
    failures.push(name + ": " + (error && error.message ? error.message : String(error)));
  }
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertSame(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(message + "\n--- actual ---\n" + actual + "\n--- expected ---\n" + expected);
  }
}

/* Declaration index of `name` in the hoisted output, measured on the
 * comment-free text so the marker cannot be mistaken for a declaration. */
function declarationIndex(text, name) {
  return text.indexOf("varying " + name);
}

function mainIndex(text) {
  return text.search(/\bvoid\s+main\s*\(/);
}

/* The real shape: LÖVE boilerplate, LÖVE's main(), `#line 1`, then user code.
 * Trimmed to the lines that decide the transform; the full 208-line capture
 * from the live love.js page has the same structure. */
const LOVE_STITCHED = [
  "#version 100",
  "#define VERTEX VERTEX",
  "#define extern uniform",
  "attribute vec4 VertexPosition;",
  "varying vec4 VaryingTexCoord;",
  "varying vec4 VaryingColor;",
  "vec4 position(mat4 clipSpaceFromLocal, vec4 localPosition);",
  "void main() {",
  "\tVaryingTexCoord = VertexTexCoord;",
  "\tlove_Position = position(ClipSpaceFromLocal, VertexPosition);",
  "}",
  "#line 1",
  "            varying vec3 v_probe;",
  "        #ifdef VERTEX",
  "            vec4 position(mat4 transform_projection, vec4 vertex_position) {",
  "                v_probe = vec3(0.5);",
  "                return transform_projection * vertex_position;",
  "            }",
  "        #endif",
  "        #ifdef PIXEL",
  "    vec4 effect(vec4 color, Image tex, vec2 tc, vec2 sc) {",
  "        return vec4(v_probe, 1.0);",
  "    }",
  "#endif",
  ""
].join("\n");

check("hoists a user varying above LOVE's main()", () => {
  const out = hoist(LOVE_STITCHED);
  assert(out !== LOVE_STITCHED, "source was left unchanged");
  const decl = declarationIndex(out, "vec3 v_probe");
  assert(decl >= 0, "the declaration disappeared entirely");
  assert(decl < mainIndex(out), "the declaration is still below main()");
});

check("leaves the declaration exactly once", () => {
  const out = hoist(LOVE_STITCHED);
  assert(out.split("varying vec3 v_probe;").length - 1 === 1, "declaration was duplicated");
});

check("preserves every line number", () => {
  const out = hoist(LOVE_STITCHED);
  assertSame(
    out.split("\n").length,
    LOVE_STITCHED.split("\n").length,
    "line count moved, which breaks LOVE's #line 1 error mapping"
  );
  const before = LOVE_STITCHED.split("\n");
  const after = out.split("\n");
  /* Everything below `#line 1` other than the blanked declaration is byte
   * identical, and the declaration's own line becomes empty rather than
   * vanishing. */
  const lineDirective = before.indexOf("#line 1");
  for (let i = lineDirective; i < before.length; i += 1) {
    if (before[i].trim() === "varying vec3 v_probe;") {
      assertSame(after[i], "", "the hoisted declaration line was not blanked");
    } else {
      assertSame(after[i], before[i], "line " + i + " below #line 1 was rewritten");
    }
  }
});

check("marks the source it rewrote", () => {
  assert(hoist(LOVE_STITCHED).indexOf(MARKER) >= 0, "no marker, so a page cannot prove the hoist ran");
});

check("hoists all four rig3d varyings, in order", () => {
  const rig = [
    "varying vec4 VaryingTexCoord;",
    "void main() {",
    "\tlove_Position = position(ClipSpaceFromLocal, VertexPosition);",
    "}",
    "#line 1",
    "    varying vec3 v_normal;",
    "    varying vec3 v_world;",
    "    varying vec4 v_slot_color;",
    "    varying float v_material;",
    "#ifdef VERTEX",
    "    uniform mat4 u_model;",
    "    vec4 position(mat4 tp, vec4 vp) { v_normal = vec3(0.0); return vp; }",
    "#endif",
    ""
  ].join("\n");
  const out = hoist(rig);
  const names = ["vec3 v_normal", "vec3 v_world", "vec4 v_slot_color", "float v_material"];
  let previous = -1;
  names.forEach((name) => {
    const at = declarationIndex(out, name);
    assert(at >= 0, name + " was lost");
    assert(at < mainIndex(out), name + " is still below main()");
    assert(at > previous, "declaration order changed at " + name);
    previous = at;
  });
  assertSame(out.split("\n").length, rig.split("\n").length, "line count moved");
});

/* ---- cases that must not be touched ------------------------------------ */

check("leaves a source with no main() alone", () => {
  const src = "varying vec3 v_probe;\nvec4 effect() { return vec4(v_probe, 1.0); }\n";
  assertSame(hoist(src), src, "a source without main() was rewritten");
});

check("leaves declarations already above main() alone", () => {
  const src = "varying vec3 v_probe;\nvoid main() {\n\tgl_Position = vec4(0.0);\n}\n";
  assertSame(hoist(src), src, "a compliant source was rewritten");
});

check("leaves non-varying globals below main() alone", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "uniform vec4 u_palette[12];",
    "attribute float VertexBoneIndex;",
    "const float k = 0.5;",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "a non-varying global was hoisted");
});

check("leaves a varying inside a line comment alone", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "// varying vec3 v_dead;",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "a commented-out declaration was hoisted into live code");
});

check("leaves a varying inside a block comment alone", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "/*",
    "varying vec3 v_dead;",
    "*/",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "a block-commented declaration was hoisted into live code");
});

check("does not treat a commented-out main() as main()", () => {
  const src = [
    "// void main() { }",
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "varying vec3 v_probe;",
    ""
  ].join("\n");
  /* `mainIndex` cannot be used here -- it would find the commented main() and
   * pass vacuously. The real main() is line 1, so that is the line the
   * declaration must land on. */
  const out = hoist(src).split("\n");
  assertSame(out[0], "// void main() { }", "the comment line was rewritten");
  assert(out[1].indexOf("varying vec3 v_probe;") >= 0, "the declaration did not land on the real main()");
  assert(/\bvoid\s+main\s*\(/.test(out[1]), "the declaration landed on the wrong line");
  assertSame(out[4], "", "the original declaration line was not blanked");
});

check("leaves an identifier that merely starts with varying alone", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "float varyingScale = 1.0;",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "`varyingScale` was mistaken for a declaration");
});

check("leaves a declaration sharing a line with code alone", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "varying vec3 v_probe; float f() { return 1.0; }",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "a shared line was rewritten");
});

check("leaves a local declaration inside a function body alone", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "void helper() {",
    "\tvarying vec3 v_illegal;",
    "}",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "a brace-nested declaration was hoisted out of its scope");
});

check("bails out of a source with unbalanced braces", () => {
  const src = ["void main() {", "\tgl_Position = vec4(0.0);", "varying vec3 v_probe;", ""].join("\n");
  assertSame(hoist(src), src, "an unparseable source was rewritten anyway");
});

check("bails out of a source with a stray #endif", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "#endif",
    "varying vec3 v_probe;",
    ""
  ].join("\n");
  assertSame(hoist(src), src, "an unbalanced conditional was rewritten anyway");
});

check("passes non-strings straight through", () => {
  assertSame(hoist(null), null, "null was transformed");
  assertSame(hoist(undefined), undefined, "undefined was transformed");
  const buffer = {};
  assert(hoist(buffer) === buffer, "a non-string object was transformed");
});

/* ---- preprocessor conditionals keep their guard ------------------------- */

check("replays the #ifdef guard around a conditional declaration", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "#ifdef VERTEX",
    "varying vec3 v_probe;",
    "#endif",
    ""
  ].join("\n");
  const out = hoist(src);
  const decl = declarationIndex(out, "vec3 v_probe");
  assert(decl >= 0 && decl < mainIndex(out), "the guarded declaration was not hoisted");
  const head = out.slice(0, mainIndex(out));
  assert(/#ifdef VERTEX/.test(head), "the guard was dropped, changing which stages declare it");
  assert(/#endif/.test(head), "the guard was left open");
});

check("replays an #else branch as an #else, not as the #if", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "#ifdef VERTEX",
    "varying vec3 v_vertex_only;",
    "#else",
    "varying vec3 v_pixel_only;",
    "#endif",
    ""
  ].join("\n");
  const head = hoist(src).slice(0, hoist(src).search(/\bvoid\s+main\s*\(/));
  const lines = head.split("\n").map((line) => line.trim()).filter((line) => line.length > 0);
  const vertexAt = lines.indexOf("varying vec3 v_vertex_only;");
  const pixelAt = lines.indexOf("varying vec3 v_pixel_only;");
  assert(vertexAt > 0 && pixelAt > 0, "one of the branch declarations was lost");
  assertSame(lines[vertexAt - 1], "#ifdef VERTEX", "the #ifdef branch lost its guard");
  assertSame(lines[pixelAt - 1], "#else", "the #else branch was promoted into the #ifdef branch");
  assertSame(lines[pixelAt - 2], "#ifdef VERTEX", "the #else lost its opening #ifdef");
});

check("keeps nested guards nested", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "#ifdef VERTEX",
    "#ifdef GL_ES",
    "varying vec3 v_probe;",
    "#endif",
    "#endif",
    ""
  ].join("\n");
  const out = hoist(src);
  const head = out.split("\n").slice(0, out.split("\n").findIndex((l) => /\bvoid\s+main\s*\(/.test(l)));
  const lines = head.map((line) => line.trim()).filter((line) => line.length > 0);
  const at = lines.indexOf("varying vec3 v_probe;");
  assert(at >= 2, "the nested declaration was not hoisted with both guards");
  assertSame(lines[at - 1], "#ifdef GL_ES", "the inner guard was lost");
  assertSame(lines[at - 2], "#ifdef VERTEX", "the outer guard was lost");
  assertSame(lines[at + 1], "#endif", "the inner guard was left open");
  assertSame(lines[at + 2], "#endif", "the outer guard was left open");
});

check("shifts nothing below #line 1 even when it must insert lines", () => {
  const src = [
    "void main() {",
    "\tgl_Position = vec4(0.0);",
    "}",
    "#line 1",
    "#ifdef VERTEX",
    "varying vec3 v_probe;",
    "#endif",
    "vec4 position(mat4 tp, vec4 vp) { return vp; }",
    ""
  ].join("\n");
  const out = hoist(src).split("\n");
  const at = out.indexOf("#line 1");
  assert(at >= 0, "#line 1 was lost");
  assertSame(out[at + 1], "#ifdef VERTEX", "user code below #line 1 was rewritten");
  assertSame(out[at + 2], "", "the declaration line was not blanked in place");
  assertSame(out[at + 3], "#endif", "user code below #line 1 was rewritten");
});

/* ---- installation ------------------------------------------------------- */

check("wraps shaderSource on both context prototypes", () => {
  const calls = [];
  function WebGLRenderingContext() {}
  WebGLRenderingContext.prototype.shaderSource = function (shader, text) {
    calls.push(text);
  };
  function WebGL2RenderingContext() {}
  WebGL2RenderingContext.prototype.shaderSource = function (shader, text) {
    calls.push(text);
  };
  const scope = {
    WebGLRenderingContext: WebGLRenderingContext,
    WebGL2RenderingContext: WebGL2RenderingContext
  };
  const wrapped = install(scope);
  assertSame(wrapped.length, 2, "expected both prototypes to be wrapped");
  new WebGLRenderingContext().shaderSource({}, LOVE_STITCHED);
  assertSame(calls.length, 1, "the original shaderSource was not called");
  assert(calls[0].indexOf(MARKER) >= 0, "the driver received the untransformed source");
  assert(declarationIndex(calls[0], "vec3 v_probe") < mainIndex(calls[0]), "no hoist reached WebGL");
});

check("is idempotent, so a double load cannot stack transforms", () => {
  function WebGLRenderingContext() {}
  WebGLRenderingContext.prototype.shaderSource = function () {};
  const scope = { WebGLRenderingContext: WebGLRenderingContext };
  assertSame(install(scope).length, 1, "first install did not wrap");
  assertSame(install(scope).length, 0, "second install wrapped again");
});

check("survives a scope with no WebGL at all", () => {
  assertSame(install({}).length, 0, "install invented a prototype");
});

/* ---- report ------------------------------------------------------------- */

failures.forEach((failure) => {
  console.error("FAIL " + failure);
});
console.log("GC_SHADER_HOIST|pass=" + passed + "|fail=" + failures.length);
if (failures.length > 0) {
  process.exit(1);
}
