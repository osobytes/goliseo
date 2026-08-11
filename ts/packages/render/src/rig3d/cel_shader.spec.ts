// Tests cel_shader.ts's headless-testable surface: the GENERATED GLSL string
// and the material wiring `applyCelShading` performs. Neither needs a live
// WebGL context -- `THREE.ShaderLib.standard.fragmentShader` is a plain
// string three.js exports, `onBeforeCompile` is just a JS function three.js
// calls with a `{ fragmentShader, vertexShader, uniforms, ... }` object
// during program compilation, and this suite calls it directly the same way,
// against three's own unmodified template rather than a hand-copied stand-in.
//
// WHAT THIS CANNOT CATCH: whether the generated GLSL actually LINKS, whether
// the toon bands/rim/specular look correct once rasterised, or whether
// `viewMatrix`/`cameraPosition`/`normal`/`vViewPosition`/`diffuseColor`/
// `reflectedLight` resolve the way this module assumes at the injection
// point for every three.js version this package might run under. This suite
// pins three things a live-GL check cannot easily pin down instead: the four
// target chunks genuinely get removed (not merely "some replace ran"), the
// constants below are the exact numbers this shading uses, and each shading
// family produces a distinct, deterministic string (so a program-cache
// collision between families would show up here as one).

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  applyCelShading,
  applyCombinedCelShading,
  ELEVATION,
  SHADING_EYE,
  SHADING_EYE_DISTANCE,
  SHADING_NORMAL_VARYING,
  SHADING_WORLD_VARYING,
  shadingFrameVertexChunk,
  BAND_HIGH,
  BAND_HIGH_THRESHOLD,
  BAND_LOW,
  BAND_MID,
  BAND_MID_THRESHOLD,
  BOUNCE_SCALE,
  CEL_SHADING_TARGET_INCLUDES,
  EMISSIVE_BASE,
  EMISSIVE_FACING_SCALE,
  LIGHT_DIR,
  RIM_METAL_MIX_HIGH,
  RIM_METAL_MIX_LOW,
  RIM_POWER,
  RIM_SMOOTH_HIGH,
  RIM_SMOOTH_LOW,
  shaderChunkFor,
  shaderChunkForCombined,
  SPEC_POWER,
  SPEC_SCALE,
  SPEC_SMOOTH_HIGH,
  SPEC_SMOOTH_LOW,
} from "./cel_shader.ts";

// The reference numbers this file's shading constants must match. Kept here
// as a second, independent statement of the same nine numbers (rather than
// importing the module's own constants for the expectation) so a typo
// introduced in cel_shader.ts's exported constants would still be caught,
// not just echoed back at itself.
describe("cel_shader ported constants match rig3d/renderer.lua's SHADER_SOURCE", () => {
  it("bands: ndl > 0.55 -> 1.0, ndl > 0.12 -> 0.72, else 0.42", () => {
    expect(BAND_HIGH_THRESHOLD).toBe(0.55);
    expect(BAND_MID_THRESHOLD).toBe(0.12);
    expect(BAND_HIGH).toBe(1.0);
    expect(BAND_MID).toBe(0.72);
    expect(BAND_LOW).toBe(0.42);
  });

  it("bounce: max(-ndl, 0) * 0.18", () => {
    expect(BOUNCE_SCALE).toBe(0.18);
  });

  it("rim: pow(.., 3), smoothstep(0.35, 0.95), tint (0.42, 0.52, 0.70), metal mix (0.55, 1.05)", () => {
    expect(RIM_POWER).toBe(3.0);
    expect(RIM_SMOOTH_LOW).toBe(0.35);
    expect(RIM_SMOOTH_HIGH).toBe(0.95);
    expect(RIM_METAL_MIX_LOW).toBe(0.55);
    expect(RIM_METAL_MIX_HIGH).toBe(1.05);
  });

  it("metal specular: pow(NdotH, 24), smoothstep(0.20, 0.42) * 0.60", () => {
    expect(SPEC_POWER).toBe(24.0);
    expect(SPEC_SMOOTH_LOW).toBe(0.2);
    expect(SPEC_SMOOTH_HIGH).toBe(0.42);
    expect(SPEC_SCALE).toBe(0.6);
  });

  it("emissive: 1.25 + 0.55 * facing", () => {
    expect(EMISSIVE_BASE).toBe(1.25);
    expect(EMISSIVE_FACING_SCALE).toBe(0.55);
  });

  it("light_dir: { -0.42, -0.78, -0.46 }, normalised", () => {
    const raw = new THREE.Vector3(-0.42, -0.78, -0.46);
    const normalised = raw.clone().normalize();
    expect(LIGHT_DIR.x).toBeCloseTo(normalised.x, 12);
    expect(LIGHT_DIR.y).toBeCloseTo(normalised.y, 12);
    expect(LIGHT_DIR.z).toBeCloseTo(normalised.z, 12);
  });
});

describe("cel_shader.shaderChunkFor", () => {
  it("emits every ported number as a valid GLSL float literal (a decimal point present, never a bare integer)", () => {
    // GLSL has no implicit int-to-float conversion: `pow(x, 3)` fails to
    // compile where `pow(x, 3.0)` links fine. Whole-number constants
    // (BAND_HIGH, RIM_POWER, SPEC_POWER, the 1.0/0.0 metal mix factor, the
    // 1.0 in SPEC_TINT) are exactly where JS's `Number#toString()` silently
    // drops the trailing `.0` if this module ever stopped routing them
    // through `glslFloat`.
    for (const family of ["plain", "metal", "emissive"] as const) {
      const chunk = shaderChunkFor(family);
      const bareIntLiterals = chunk.match(/[^.\w](\d+)(?![.\d\w])/g) ?? [];
      expect(bareIntLiterals, `family=${family} chunk:\n${chunk}`).toEqual([]);
    }
  });

  it("plain: bands + bounce + rim, no metal specular", () => {
    const chunk = shaderChunkFor("plain");
    expect(chunk).toContain("reflectedLight.directDiffuse = gcLit");
    expect(chunk).toContain("smoothstep( 0.35, 0.95, rim )");
    expect(chunk).not.toContain("gcHalfDir");
    expect(chunk).not.toContain("gcSpec");
  });

  it("metal: plain's bands/bounce/rim, PLUS the hard specular band", () => {
    const chunk = shaderChunkFor("metal");
    expect(chunk).toContain("smoothstep( 0.35, 0.95, rim )");
    expect(chunk).toContain("gcHalfDir");
    expect(chunk).toContain("pow( max( dot( gcNormal, gcHalfDir ), 0.0 ), 24.0 )");
    expect(chunk).toContain("smoothstep( 0.2, 0.42, gcSpec ) * 0.6");
  });

  it("metal's rim reads mix(0.55, 1.05, 1.0) -- plain's reads mix(0.55, 1.05, 0.0)", () => {
    // The `mix(0.55, 1.05, u_metal)` term: this is the one place the
    // plain/metal split shows up OUTSIDE the specular block itself.
    expect(shaderChunkFor("plain")).toContain("mix( 0.55, 1.05, 0.0 )");
    expect(shaderChunkFor("metal")).toContain("mix( 0.55, 1.05, 1.0 )");
  });

  it("emissive: bypasses the band/rim/specular math entirely", () => {
    const chunk = shaderChunkFor("emissive");
    expect(chunk).toContain(
      "reflectedLight.directDiffuse = diffuseColor.rgb * ( 1.25 + 0.55 * gcFacing )",
    );
    expect(chunk).not.toContain("band");
    expect(chunk).not.toContain("rim");
    expect(chunk).not.toContain("gcSpec");
  });

  it("plain and metal differ only by the specular addition and the rim's metal-mix factor", () => {
    expect(shaderChunkFor("plain")).not.toEqual(shaderChunkFor("metal"));
  });
});

describe("cel_shader.applyCelShading", () => {
  // Runs `onBeforeCompile` against three.js's OWN unmodified
  // `MeshStandardMaterial` shader template -- proof the four target
  // `#include`s this module depends on still exist, verbatim, in the
  // installed three.js version, and that `String.replace` actually finds and
  // removes all four rather than silently matching zero of them (a
  // `.replace()` call against a string that no longer contains its target is
  // not an error in JS -- it is a silent no-op, which is exactly the failure
  // mode a version bump could introduce).
  function compileAgainstRealTemplate(family: "plain" | "metal" | "emissive") {
    const material = new THREE.MeshStandardMaterial({ vertexColors: true });
    applyCelShading(material, family);
    const shader = {
      fragmentShader: THREE.ShaderLib["standard"]?.fragmentShader,
      vertexShader: THREE.ShaderLib["standard"]?.vertexShader,
      uniforms: {},
      defines: {},
    };
    if (shader.fragmentShader === undefined || shader.vertexShader === undefined) {
      throw new Error(
        "cel_shader.spec.ts: THREE.ShaderLib.standard is missing -- three.js version mismatch?",
      );
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- three.js's `onBeforeCompile(shader, renderer)` takes its own internal shader object and a live WebGLRenderer; this test drives it with a synthetic shader and no renderer on purpose, and neither has a public type to name.
    material.onBeforeCompile(shader as any, undefined as any);
    return { material, fragmentShader: shader.fragmentShader };
  }

  it("removes all four target includes from the real MeshStandardMaterial template", () => {
    const { fragmentShader } = compileAgainstRealTemplate("plain");
    for (const include of CEL_SHADING_TARGET_INCLUDES) {
      expect(fragmentShader).not.toContain(include);
    }
  });

  it("splices the ported shading in where lights_physical_fragment was", () => {
    const { fragmentShader } = compileAgainstRealTemplate("metal");
    expect(fragmentShader).toContain("reflectedLight.directDiffuse = gcLit");
    expect(fragmentShader).toContain("gcHalfDir");
  });

  it('sets material.side to DoubleSide, matching rig3d/renderer.lua\'s setMeshCullMode("none")', () => {
    const { material } = compileAgainstRealTemplate("plain");
    expect(material.side).toBe(THREE.DoubleSide);
  });

  it("gives each shading family its own customProgramCacheKey so three.js cannot share one family's compiled program with another's draw call", () => {
    const plain = new THREE.MeshStandardMaterial({ vertexColors: true });
    applyCelShading(plain, "plain");
    const metal = new THREE.MeshStandardMaterial({ vertexColors: true });
    applyCelShading(metal, "metal");
    const emissive = new THREE.MeshStandardMaterial({ vertexColors: true });
    applyCelShading(emissive, "emissive");
    const keys = new Set([
      plain.customProgramCacheKey(),
      metal.customProgramCacheKey(),
      emissive.customProgramCacheKey(),
    ]);
    expect(keys.size).toBe(3);
  });
});

// COMBINED (draw-call fix #2, see player_renderer_3d.ts's `materialsForTeam`
// and this file's header). `applyCombinedCelShading` is the one-program,
// runtime-branching sibling of `applyCelShading` above: same ported numbers,
// selected off a `materialFamily` vertex attribute instead of which
// `MeshStandardMaterial` instance a caller applied the shading to.
describe("cel_shader.shaderChunkForCombined", () => {
  it("wraps all three per-family chunks (celShadingChunk's own generated text) in a runtime branch on vMaterialFamily", () => {
    const combined = shaderChunkForCombined();
    expect(combined).toContain("if ( vMaterialFamily > 1.5 )");
    expect(combined).toContain("else if ( vMaterialFamily > 0.5 )");
    // Every family's own distinguishing content must be present verbatim --
    // this is NOT a hand-duplicated second copy of the ported math, so a
    // future change to celShadingChunk's numbers propagates here for free
    // and this assertion keeps proving that propagation actually happens.
    expect(combined).toContain(
      "reflectedLight.directDiffuse = diffuseColor.rgb * ( 1.25 + 0.55 * gcFacing )",
    ); // emissive
    expect(combined).toContain("pow( max( dot( gcNormal, gcHalfDir ), 0.0 ), 24.0 )"); // metal specular
    expect(combined).toContain("mix( 0.55, 1.05, 0.0 )"); // plain's rim metal-mix factor
  });

  it("emits every ported number as a valid GLSL float literal, same as the per-family chunks", () => {
    const combined = shaderChunkForCombined();
    const bareIntLiterals = combined.match(/[^.\w](\d+)(?![.\d\w])/g) ?? [];
    expect(bareIntLiterals, combined).toEqual([]);
  });
});

describe("cel_shader.applyCombinedCelShading", () => {
  function compileCombinedAgainstRealTemplate() {
    const material = new THREE.MeshStandardMaterial({ vertexColors: true });
    applyCombinedCelShading(material);
    const shader = {
      fragmentShader: THREE.ShaderLib["standard"]?.fragmentShader,
      vertexShader: THREE.ShaderLib["standard"]?.vertexShader,
      uniforms: {},
      defines: {},
    };
    if (shader.fragmentShader === undefined || shader.vertexShader === undefined) {
      throw new Error(
        "cel_shader.spec.ts: THREE.ShaderLib.standard is missing -- three.js version mismatch?",
      );
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- three.js's `onBeforeCompile(shader, renderer)` takes its own internal shader object and a live WebGLRenderer; this test drives it with a synthetic shader and no renderer on purpose, and neither has a public type to name.
    material.onBeforeCompile(shader as any, undefined as any);
    return { material, fragmentShader: shader.fragmentShader, vertexShader: shader.vertexShader };
  }

  it("removes all four target includes from the real MeshStandardMaterial fragment template", () => {
    const { fragmentShader } = compileCombinedAgainstRealTemplate();
    for (const include of CEL_SHADING_TARGET_INCLUDES) {
      expect(fragmentShader).not.toContain(include);
    }
  });

  it("declares the materialFamily attribute and vMaterialFamily varying in both stages, and assigns the varying in main()", () => {
    const { vertexShader, fragmentShader } = compileCombinedAgainstRealTemplate();
    expect(vertexShader).toContain("attribute float materialFamily;");
    expect(vertexShader).toContain("varying float vMaterialFamily;");
    expect(vertexShader).toContain("vMaterialFamily = materialFamily;");
    expect(fragmentShader).toContain("varying float vMaterialFamily;");
  });

  it("splices the combined runtime-branching shading in where lights_physical_fragment was", () => {
    const { fragmentShader } = compileCombinedAgainstRealTemplate();
    expect(fragmentShader).toContain("if ( vMaterialFamily > 1.5 )");
    expect(fragmentShader).toContain("reflectedLight.directDiffuse = gcLit");
  });

  it('sets material.side to DoubleSide, matching rig3d/renderer.lua\'s setMeshCullMode("none")', () => {
    const { material } = compileCombinedAgainstRealTemplate();
    expect(material.side).toBe(THREE.DoubleSide);
  });

  it("wires a real, non-default onBeforeCompile rather than leaving PBR defaults", () => {
    const material = new THREE.MeshStandardMaterial({ vertexColors: true });
    applyCombinedCelShading(material);
    expect(material.onBeforeCompile).not.toBe(new THREE.MeshStandardMaterial().onBeforeCompile);
  });
});

// ---------------------------------------------------------------------------
// THE SHADING FRAME. Everything above this line passed both before and after
// the defect docs/render_differential.md recorded (characters rendering pale
// and near-white) was fixed -- it pins the ported NUMBERS, and the defect was
// in which SPACE those numbers were evaluated in. These tests pin the space.
//
// Two of them are string assertions, which is the weaker kind: they say the
// generated GLSL still reads the varyings this module builds rather than
// three.js's camera-dependent `normal`/`vViewPosition`. The last one is not a
// string assertion at all -- it reimplements the vertex-stage recovery in TS
// and checks it against a directly-composed ground truth, so it fails on a
// wrong sign or a dropped term rather than on a changed spelling.
// ---------------------------------------------------------------------------

describe("cel_shader shading frame", () => {
  it("reads the rebuilt varyings, never three.js's camera-dependent normal/vViewPosition", () => {
    for (const chunk of [
      shaderChunkFor("plain"),
      shaderChunkFor("metal"),
      shaderChunkFor("emissive"),
      shaderChunkForCombined(),
    ]) {
      expect(chunk).toContain(SHADING_NORMAL_VARYING);
      expect(chunk).toContain(SHADING_WORLD_VARYING);
      // `vViewPosition` is the fragment-to-eye vector for a PERSPECTIVE
      // camera; pitch.ts draws through an orthographic one sitting at z = 1 in
      // a pixel-space scene, where it points mostly sideways and floods `rim`.
      expect(chunk).not.toContain("vViewPosition");
      // three.js's `normal` carries the inverse-transpose of pitch.ts's
      // `(ppm, -ppm, 0.05)` wrapper scale, which collapses every normal onto
      // +/-Z. See THE SHADING FRAME in cel_shader.ts.
      expect(chunk).not.toMatch(/normalize\(\s*normal\s*\)/);
      expect(chunk).not.toContain("viewMatrix");
    }
  });

  it("flips two-sided normals by N.V, not by gl_FrontFacing (which the two hosts disagree about)", () => {
    const chunk = shaderChunkFor("plain");
    expect(chunk).toContain("if ( dot( gcNormal, gcViewDir ) < 0.0 )");
    // The original renderer's own spelling does not carry over unmodified:
    // three.js flips the raster front-face convention for pitch.ts's
    // mirrored wrapper and the original renderer's convention did not, so
    // copying this line leaves every visible fragment inward-facing.
    expect(chunk).not.toContain("gl_FrontFacing");
  });

  it("places the shading eye where rig3d/renderer.lua does: dir * 24, dir = (0, sin elevation, cos elevation)", () => {
    expect(SHADING_EYE_DISTANCE).toBe(24);
    expect(SHADING_EYE.x).toBeCloseTo(0, 12);
    expect(SHADING_EYE.y).toBeCloseTo(Math.sin(ELEVATION) * 24, 12);
    expect(SHADING_EYE.z).toBeCloseTo(Math.cos(ELEVATION) * 24, 12);
    // Distant on purpose: a unit-length eye next to a ~1.8 unit figure
    // swings the view direction between the feet and the head.
    expect(SHADING_EYE.length()).toBeGreaterThan(10);
  });

  it("injects the frame after project_vertex, where BOTH objectNormal and transformed are final", () => {
    for (const apply of [
      (m: THREE.MeshStandardMaterial) => applyCombinedCelShading(m),
      (m: THREE.MeshStandardMaterial) => applyCelShading(m, "plain"),
    ]) {
      const material = new THREE.MeshStandardMaterial({ vertexColors: true });
      apply(material);
      const shader = {
        fragmentShader: THREE.ShaderLib["standard"]?.fragmentShader ?? "",
        vertexShader: THREE.ShaderLib["standard"]?.vertexShader ?? "",
        uniforms: {},
        defines: {},
      };
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- three.js's `onBeforeCompile(shader, renderer)` takes its own internal shader object and a live WebGLRenderer; this test drives it with a synthetic shader and no renderer on purpose, and neither has a public type to name.
      material.onBeforeCompile(shader as any, undefined as any);
      expect(shader.vertexShader).toContain(`varying vec3 ${SHADING_NORMAL_VARYING};`);
      expect(shader.vertexShader).toContain(`varying vec3 ${SHADING_WORLD_VARYING};`);
      expect(shader.fragmentShader).toContain(`varying vec3 ${SHADING_NORMAL_VARYING};`);
      // Reading `objectNormal`/`transformed` rather than `normal`/`position`
      // is what makes this correct on a SkinnedMesh: the rig moves both, and
      // shading the bind pose would be silently wrong rather than obviously so.
      const at = shader.vertexShader.indexOf("#include <project_vertex>");
      const assigned = shader.vertexShader.indexOf(`${SHADING_NORMAL_VARYING} = `);
      expect(at).toBeGreaterThan(0);
      expect(assigned).toBeGreaterThan(at);
      expect(shader.vertexShader).toContain("objectNormal");
      expect(shader.vertexShader).toContain("gcModel * transformed");
    }
  });

  it("recovers R from modelMatrix = S * R for pitch.ts's own (ppm, -ppm, 0.05) wrapper", () => {
    // The GLSL in `shadingFrameVertexChunk` reimplemented in TS. Kept as a
    // reimplementation rather than a parse of the generated string so this
    // fails on the MATH being wrong, which a string assertion cannot see.
    function recover(modelMatrix: THREE.Matrix4, objectNormal: THREE.Vector3): THREE.Vector3 {
      const m = new THREE.Matrix3().setFromMatrix4(modelMatrix);
      const e = m.elements; // column-major: e[c * 3 + r]
      const row = (r: number) => new THREE.Vector3(e[r] ?? 0, e[3 + r] ?? 0, e[6 + r] ?? 0);
      const scale = new THREE.Vector3(row(0).length(), row(1).length(), row(2).length());
      scale.y *= Math.sign(modelMatrix.determinant());
      const v = objectNormal.clone().applyMatrix3(m);
      v.set(v.x / scale.x, v.y / scale.y, v.z / scale.z);
      const untilt = new THREE.Matrix4().makeRotationX(-ELEVATION);
      return v.applyMatrix4(untilt);
    }

    const ppm = 25;
    const yaw = 0.9;
    // Exactly pitch.ts's composition: wrapper carries scale + position, the
    // mesh carries ELEVATION_TILT premultiplied onto its own yaw.
    const rotation = new THREE.Quaternion()
      .setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw)
      .premultiply(new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), ELEVATION));
    const modelMatrix = new THREE.Matrix4()
      .compose(
        new THREE.Vector3(640, 400, 0.5),
        new THREE.Quaternion(),
        new THREE.Vector3(ppm, -ppm, 0.05),
      )
      .multiply(new THREE.Matrix4().makeRotationFromQuaternion(rotation));

    // Ground truth: the yaw alone, which is the shading frame's model.
    const yawOnly = new THREE.Matrix4().makeRotationY(yaw);

    let crushed = 0;
    for (let i = 0; i < 24; i += 1) {
      const a = (i / 24) * Math.PI * 2;
      const n = new THREE.Vector3(Math.cos(a), 0.35, Math.sin(a)).normalize();
      const got = recover(modelMatrix, n).normalize();
      const want = n.clone().applyMatrix4(yawOnly).normalize();
      expect(got.x).toBeCloseTo(want.x, 6);
      expect(got.y).toBeCloseTo(want.y, 6);
      expect(got.z).toBeCloseTo(want.z, 6);
      if (Math.abs(got.z) > 0.999) {
        crushed += 1;
      }
    }
    // The defect this replaces: three.js's own normal matrix -- the inverse
    // transpose of that same wrapper scale -- turns all 24 of these into
    // +/-Z, leaving `ndl` two values and the character two flat tones.
    expect(crushed).toBe(0);
  });

  it("emits the untilt as valid GLSL floats, and reduces to identity at zero elevation", () => {
    const chunk = shadingFrameVertexChunk(0);
    expect(chunk).toContain("mat3 gcUntilt = mat3( 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0 )");
    expect(shadingFrameVertexChunk()).toContain(`${Math.cos(ELEVATION)}`);
    expect(shadingFrameVertexChunk()).toContain("sign( determinant( gcModel ) )");
  });
});
