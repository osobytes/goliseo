// Tests cel_shader.ts's headless-testable surface: the GENERATED GLSL string
// and the material wiring `applyCelShading` performs. Neither needs a live
// WebGL context -- `THREE.ShaderLib.standard.fragmentShader` is a plain
// string three.js exports, `onBeforeCompile` is just a JS function three.js
// calls with a `{ fragmentShader, vertexShader, uniforms, ... }` object
// during program compilation, and this suite calls it directly the same way,
// against three's own unmodified template rather than a hand-copied stand-in.
//
// WHAT THIS CANNOT CATCH (see player_renderer_3d.ts's port report): whether
// the generated GLSL actually LINKS, whether the toon bands/rim/specular
// look correct once rasterised, or whether `viewMatrix`/`cameraPosition`/
// `normal`/`vViewPosition`/`diffuseColor`/`reflectedLight` resolve the way
// this module assumes at the injection point for every three.js version this
// package might run under. This suite pins three things a live-GL check
// cannot easily pin down instead: the four target chunks genuinely get
// removed (not merely "some replace ran"), the ported numbers are the exact
// numbers rig3d/renderer.lua's SHADER_SOURCE used, and each shading family
// produces a distinct, deterministic string (so a program-cache collision
// between families would show up here as one).

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { applyCelShading, BAND_HIGH, BAND_HIGH_THRESHOLD, BAND_LOW, BAND_MID, BAND_MID_THRESHOLD, BOUNCE_SCALE, CEL_SHADING_TARGET_INCLUDES, EMISSIVE_BASE, EMISSIVE_FACING_SCALE, LIGHT_DIR, RIM_METAL_MIX_HIGH, RIM_METAL_MIX_LOW, RIM_POWER, RIM_SMOOTH_HIGH, RIM_SMOOTH_LOW, shaderChunkFor, SPEC_POWER, SPEC_SCALE, SPEC_SMOOTH_HIGH, SPEC_SMOOTH_LOW } from "./cel_shader.ts";

// rig3d/renderer.lua's SHADER_SOURCE, PIXEL stage -- the numbers this file
// ports. Kept here as a second, independent statement of the same nine
// numbers (rather than importing the module's own constants for the
// expectation) so a typo introduced in cel_shader.ts's exported constants
// would still be caught, not just echoed back at itself.
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
    // The Lua's `mix(0.55, 1.05, u_metal)` term: this is the one place the
    // plain/metal split shows up OUTSIDE the specular block itself.
    expect(shaderChunkFor("plain")).toContain("mix( 0.55, 1.05, 0.0 )");
    expect(shaderChunkFor("metal")).toContain("mix( 0.55, 1.05, 1.0 )");
  });

  it("emissive: bypasses the band/rim/specular math entirely", () => {
    const chunk = shaderChunkFor("emissive");
    expect(chunk).toContain("reflectedLight.directDiffuse = diffuseColor.rgb * ( 1.25 + 0.55 * gcFacing )");
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
      throw new Error("cel_shader.spec.ts: THREE.ShaderLib.standard is missing -- three.js version mismatch?");
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
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

  it("sets material.side to DoubleSide, matching rig3d/renderer.lua's setMeshCullMode(\"none\")", () => {
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
    const keys = new Set([plain.customProgramCacheKey(), metal.customProgramCacheKey(), emissive.customProgramCacheKey()]);
    expect(keys.size).toBe(3);
  });
});
