// Ported from game/render/rig3d/renderer.lua's `SHADER_SOURCE` -- specifically
// its `#ifdef PIXEL` / `effect()` half.
//
// v2/README.md #7 marks `rig3d/renderer.lua` "replace -- WebGLRenderer,
// MeshStandardMaterial", and that verdict is correct for roughly 20% of the
// file: the depth pass setup, the draw call, the per-character orthographic
// camera. It is WRONG for the other 80%, the shader itself. Three.js ships a
// renderer; it does not ship this game's look. The file's own header states
// the intent directly: "a directional key light quantised into three bands,
// plus a view-dependent rim so silhouettes separate from the background.
// Enough to make the 3D form read without pretending to be physically
// based." `MeshStandardMaterial` is exactly the physically-based thing that
// sentence is declining. This module ports the shading math itself via
// `MeshStandardMaterial.onBeforeCompile`, which keeps three.js's own
// skinning / morph-target / vertex chunks intact (the characters are
// `THREE.SkinnedMesh`; hand-rolling skinning is how that breaks) while
// replacing only the fragment-stage lighting.
//
// WHAT DID NOT PORT, DELIBERATELY (the README's "replace" verdict still
// applies to these specific mechanisms, even though the shading around them
// did not "replace" cleanly):
//
//   * Bone matrices. The Lua packed bones as THREE-row `mat4`s (dropping the
//     implicit (0,0,0,1) fourth row) to fit `gl_MaxVertexUniformVectors`'s
//     GLSL ES 1.00 guaranteed floor of 128 vec4 alongside a 26-bone rig, a
//     palette array and three view/proj/model matrices -- see the Lua's own
//     `MIN_VERTEX_UNIFORM_VECTORS` comment for the arithmetic. Three.js
//     uploads bone matrices via a `DataTexture` (`Skeleton.boneTexture`);
//     there is no such uniform budget to protect, so there is nothing to
//     port here -- three.js's own skinning chunks are simply used unmodified.
//   * The palette lookup. GLSL ES 1.00 forbids *dynamic* (non-constant)
//     array indexing in the fragment stage, which is why the Lua resolved
//     `u_palette[VertexPaletteSlot]` in the VERTEX stage and carried the
//     result down as a varying. WebGL2 (three.js's default) allows dynamic
//     fragment-stage indexing, but this port does not even need that
//     allowance: `player_renderer_3d.ts`'s `materialsForTeam` bakes the
//     resolved palette straight into a per-vertex `color` `BufferAttribute`
//     ahead of time, so there is no uniform array left to index into at all,
//     in either stage.
// Vertex-carried material family as a float DID eventually port, in the end
// exactly as the Lua wrote it, though not on the first pass through this
// file. The Lua's `VertexMaterial` attribute existed only because GLSL ES
// 1.00 has no integer vertex attributes; this module's first cut instead let
// three.js's geometry *groups* assign one `THREE.Material` per contiguous
// vertex run, so "which shading family a triangle belongs to" was which
// `CelFamily` `applyCelShading` (below) was called with for that group's
// material -- a compile-time, per-material-instance constant, not a
// per-vertex value read at all. That reproduced the LOOK but not the DRAW-CALL
// COUNT: `player_renderer_3d.ts`'s parts interleave families, so groups fired
// on every material transition, not once per family, and even a clean
// three-groups-per-character outcome would still be three draws where the
// Lua's one shader was one. `applyCombinedCelShading` (below, alongside the
// original per-family `applyCelShading`) is the fix: it reads a real
// `materialFamily` vertex attribute (`player_renderer_3d.ts`'s `build()`
// bakes it, `geometry.MATERIAL`'s own numbering) and branches on it at
// RUNTIME in the fragment stage -- precisely rig3d/renderer.lua's own
// mechanism, and precisely why that file's header states "branching on a
// varying in the fragment stage is fine here; only dynamic *array indexing*
// is forbidden." One compiled program, one draw call, matching the Lua.
//
// Only the LOOK is reproduced; the WebGL1 plumbing that produced it is not.

import * as THREE from "three";

/** Which of rig3d/renderer.lua's three shading families a material belongs to. */
export type CelFamily = "plain" | "metal" | "emissive";

// Ported 1:1 from rig3d/renderer.lua's module-level `light_dir` constant:
// `{ -0.42, -0.78, -0.46 }`, "direction the light travels" (world space).
// Also reused by player_renderer_3d.ts to POSITION its (now vestigial, see
// that file's LIGHTING comment) decorative `THREE.DirectionalLight`, so this
// is the one place the constant is declared.
export const LIGHT_DIR = new THREE.Vector3(-0.42, -0.78, -0.46).normalize();

// The broadcast camera's apparent elevation. Declared here, and re-exported by
// player_renderer_3d.ts, for the same reason as `LIGHT_DIR` above: the shading
// frame this module rebuilds (see `shadingFrameVertexChunk`) has to remove
// exactly the tilt pitch.ts composes onto a character, and two copies of that
// angle could drift apart silently -- the symptom would be shading that is
// subtly lit from the wrong height, which nothing else would catch.
export const ELEVATION = (17 * Math.PI) / 180;

// Ported from rig3d/renderer.lua's `SHADING_EYE_DISTANCE = 24`, and its
// comment: "How far away the shader is told the camera is. Only affects rim/
// specular direction, never the projection." The Lua deliberately decouples
// the shading eye from the (orthographic) projection, because -- its words --
// "`dir` is a unit vector, which next to a ~1.8 unit tall figure is close
// enough that the view direction swings wildly between the feet and the head
// and the shading reads inconsistently across one character."
export const SHADING_EYE_DISTANCE = 24;

// rig3d/renderer.lua's `characterCamera`: `dir = { 0, sin(elevation),
// cos(elevation) }`, `eye = dir * SHADING_EYE_DISTANCE`. In the character's own
// Y-up frame (which is what `shadingFrameVertexChunk` rebuilds), so it needs no
// camera state at all -- exactly like the Lua uniform it replaces.
export const SHADING_EYE = new THREE.Vector3(0, Math.sin(ELEVATION), Math.cos(ELEVATION)).multiplyScalar(SHADING_EYE_DISTANCE);

// Ported constants, one per named quantity in rig3d/renderer.lua's PIXEL
// stage (`effect()`). Exported (not inlined as bare numbers in the template
// string below) so a headless spec can assert the actual numbers this port
// runs with, rather than merely that some replacement occurred, and so the
// injected GLSL and the tested numbers cannot drift apart -- `celShadingFor`
// below is *generated* from these, not a hand-duplicated second copy.
//
// "Three flat bands instead of a smooth ramp: the 'toon' part."
export const BAND_HIGH_THRESHOLD = 0.55;
export const BAND_MID_THRESHOLD = 0.12;
export const BAND_HIGH = 1.0;
export const BAND_MID = 0.72;
export const BAND_LOW = 0.42;
// "Cool bounce light from below so shadowed sides do not go dead."
export const BOUNCE_SCALE = 0.18;
// The view-dependent rim: `pow(1 - max(NdotV,0), 3)`, `smoothstep(0.35, 0.95)`,
// tinted, scaled down for non-metal so it stays a silhouette hint rather
// than a highlight.
export const RIM_POWER = 3.0;
export const RIM_SMOOTH_LOW = 0.35;
export const RIM_SMOOTH_HIGH = 0.95;
export const RIM_TINT = new THREE.Vector3(0.42, 0.52, 0.7);
export const RIM_METAL_MIX_LOW = 0.55;
export const RIM_METAL_MIX_HIGH = 1.05;
// Metal-only hard specular: "most of what separates 'kit' from 'body' at a
// glance" per the Lua's own comment. Skin and cloth (the `plain` family) get
// none of this.
export const SPEC_POWER = 24.0;
export const SPEC_SMOOTH_LOW = 0.2;
export const SPEC_SMOOTH_HIGH = 0.42;
export const SPEC_SCALE = 0.6;
export const SPEC_TINT = new THREE.Vector3(1.0, 0.97, 0.88);
// Emissive slots bypass lighting entirely and are pushed PAST WHITE
// (1.25 + 0.55 * facing, facing in [0,1], so up to 1.80x) at the centre of
// their silhouette, so they read as light sources rather than as brightly
// painted metal.
export const EMISSIVE_BASE = 1.25;
export const EMISSIVE_FACING_SCALE = 0.55;

// The four consecutive `#include`s this shading replaces in three.js's
// `meshphysical.glsl.js` fragment template (the shader `MeshStandardMaterial`
// compiles to), in source order. `lights_physical_fragment` builds the
// `PhysicalMaterial` struct (roughness/metalness/etc) this shading never
// reads; `lights_fragment_begin` / `_maps` / `_end` are the light
// accumulation loop (`RE_Direct`, `RE_IndirectDiffuse`, `RE_IndirectSpecular`)
// this shading replaces outright. Blanking all four -- rather than only
// `lights_fragment_begin` -- means no light loop runs at all, so
// `reflectedLight.indirectDiffuse` / `.indirectSpecular` stay at
// `ReflectedLight`'s zero-initialised default and every `THREE.Light` in the
// scene (ambient, hemisphere, directional, ...) is a no-op for this
// material, exactly matching the Lua original: LOVE has no scene-graph
// lights at all, only the two uniforms `renderer.beginPass` sends by hand
// (`u_light_dir`, `u_cam_pos`), which is what this module hardcodes instead.
export const CEL_SHADING_TARGET_INCLUDES: readonly string[] = ["#include <lights_physical_fragment>", "#include <lights_fragment_begin>", "#include <lights_fragment_maps>", "#include <lights_fragment_end>"];

// GLSL requires an explicit decimal point on float literals -- there is no
// implicit int-to-float conversion, so `pow(x, 3)` fails to link ("no
// matching overloaded function") where `pow(x, 3.0)` compiles. Several of
// the ported constants above are whole numbers (`BAND_HIGH = 1.0`,
// `RIM_POWER = 3.0`, `SPEC_POWER = 24.0`, ...), and JS's `Number#toString()`
// drops the trailing `.0` (`String(1.0) === "1"`), so every numeric constant
// interpolated into the template below goes through this rather than being
// interpolated directly.
function glslFloat(n: number): string {
  return Number.isInteger(n) ? `${n}.0` : `${n}`;
}

function glslVec3(v: THREE.Vector3): string {
  return `vec3(${glslFloat(v.x)}, ${glslFloat(v.y)}, ${glslFloat(v.z)})`;
}

// ---------------------------------------------------------------------------
// THE SHADING FRAME, and why this shader may not use three.js's own `normal` /
// `vViewPosition` the way an ordinary material would.
//
// rig3d/renderer.lua shades in ONE specific space: the character's own
// Y-up world, where `u_model` is the character's YAW ONLY (`v_normal =
// mat3(u_model) * bone_normal`, `v_world = (u_model * posed).xyz`), the light
// is the world constant `light_dir`, and the camera elevation lives entirely
// in `u_view` / the shading eye. Every ported number below -- the band
// thresholds, the rim's smoothstep window, the specular exponent -- is
// calibrated against normals and view directions expressed in THAT frame.
//
// pitch.ts's single-pass compositing does not hand us that frame. It draws
// every character through one shared `OrthographicCamera(0, w, 0, h)` whose
// units are SCREEN PIXELS, and places each character with
// `wrapper.scale.set(ppm, -ppm, CHARACTER_DEPTH_SCALE)`. That transform is
// correct for geometry and catastrophic for shading, in two independent ways:
//
//  1. IT DESTROYS THE NORMALS. A normal matrix is the inverse transpose, so
//     `diag(ppm, -ppm, 0.05)` becomes `diag(1/ppm, -1/ppm, 20)` -- the Z
//     component is amplified by ~500x relative to X/Y at a typical ppm of 25.
//     Measured against this exact transform chain: 32 of 32 sample normals
//     spread around a character collapse onto +/-Z, leaving `ndl` with two
//     values instead of a range. Both sit between BAND_MID_THRESHOLD and
//     BAND_HIGH_THRESHOLD, so the whole character renders in two flat tones
//     and the bright band is never reached at all. The "toon" shading had no
//     form left to quantise.
//  2. IT INVALIDATES `vViewPosition`. three.js sets it to `-mvPosition.xyz`,
//     the fragment-to-eye vector -- right for a perspective camera, wrong for
//     an orthographic one, and doubly wrong here because the camera sits at
//     z = 1 in a space where the character is tens of pixels wide. The vector
//     points mostly SIDEWAYS, so `dot(n, viewDir) ~ 0`, `rim` saturates to 1
//     across the entire silhouette, and every fragment gains a flat
//     `RIM_TINT * RIM_METAL_MIX_LOW` wash. That is the near-white,
//     desaturated look docs/render_differential.md recorded.
//
// So this module rebuilds the Lua's frame in the vertex stage instead of
// consuming three.js's. `modelMatrix`'s upper 3x3 is `S * R` (the wrapper
// carries only scale and translation; the mesh carries only rotation), and `R`
// is orthonormal -- so each ROW's length recovers that row's scale factor
// exactly, and dividing it out leaves `R * v`. The Y-mirror's sign comes back
// from the determinant rather than being hardcoded, so a caller that composes
// its transform differently (or not at all -- `modelMatrix` is a plain
// rotation on the per-character-camera path) still gets the right frame. What
// remains is pitch.ts's elevation tilt, which the Lua keeps in its camera and
// not its model; removing it lands us in the Lua's own frame, where
// `LIGHT_DIR` and `SHADING_EYE` can be used verbatim with no camera state.
// ---------------------------------------------------------------------------

/** Varying carrying the character-frame normal; see the section comment above. */
export const SHADING_NORMAL_VARYING = "vGcNormal";
/** Varying carrying the character-frame position, in metres. */
export const SHADING_WORLD_VARYING = "vGcWorld";

// Inserted after `#include <project_vertex>`, which is the first point where
// BOTH of three.js's own working values exist: `objectNormal` (declared by
// `beginnormal_vertex`, final after `skinnormal_vertex`) and `transformed`
// (declared by `begin_vertex`, final after `skinning_vertex`). Reading them
// rather than `position`/`normal` is what makes this work on a SkinnedMesh at
// all -- the rig's bones move both, and shading the bind pose would be a
// silent, subtle wrongness rather than an obvious one.
function shadingFrameVertexBody(elevation: number): string {
  const ce = glslFloat(Math.cos(elevation));
  const se = glslFloat(Math.sin(elevation));
  const negSe = glslFloat(-Math.sin(elevation));
  return `
    // Recover R from modelMatrix = S * R by dividing each row by its own
    // length (R is orthonormal, so every row of S * R has length |S_i|).
    mat3 gcModel = mat3( modelMatrix );
    vec3 gcScale = vec3(
      length( vec3( gcModel[0][0], gcModel[1][0], gcModel[2][0] ) ),
      length( vec3( gcModel[0][1], gcModel[1][1], gcModel[2][1] ) ),
      length( vec3( gcModel[0][2], gcModel[1][2], gcModel[2][2] ) )
    );
    gcScale = max( gcScale, vec3( 1e-6 ) );
    // Row lengths are unsigned, so a mirrored transform would come back
    // unmirrored. pitch.ts mirrors Y (SceneRoot's Y-down screen convention);
    // a negative determinant is that mirror, and restoring its sign here is
    // what keeps the key light coming from above rather than below.
    gcScale.y *= sign( determinant( gcModel ) );

    // Undo the elevation tilt pitch.ts composes onto the MESH, landing in
    // rig3d/renderer.lua's own frame (yaw only, elevation in the camera).
    mat3 gcUntilt = mat3( 1.0, 0.0, 0.0, 0.0, ${ce}, ${negSe}, 0.0, ${se}, ${ce} );

    ${SHADING_NORMAL_VARYING} = gcUntilt * ( ( gcModel * objectNormal ) / gcScale );
    ${SHADING_WORLD_VARYING} = gcUntilt * ( ( gcModel * transformed ) / gcScale );
`;
}

/**
 * The vertex-stage half of this shading: declares the two shading-frame
 * varyings and fills them. Both `applyCelShading` and
 * `applyCombinedCelShading` inject this -- the fragment chunks below read
 * those varyings and nothing else camera-dependent, so the two entry points
 * cannot drift on which space they shade in.
 */
export function shadingFrameVertexChunk(elevation: number = ELEVATION): string {
  return shadingFrameVertexBody(elevation);
}

function withShadingFrameVertex(vertexShader: string, elevation: number): string {
  const declared = `varying vec3 ${SHADING_NORMAL_VARYING};\nvarying vec3 ${SHADING_WORLD_VARYING};\n${vertexShader}`;
  return declared.replace("#include <project_vertex>", `#include <project_vertex>\n${shadingFrameVertexBody(elevation)}`);
}

function shadingFrameFragmentDeclarations(): string {
  return `varying vec3 ${SHADING_NORMAL_VARYING};\nvarying vec3 ${SHADING_WORLD_VARYING};\n`;
}

// The replacement GLSL for one shading family, spliced in where
// `#include <lights_physical_fragment>` was (see `applyCelShading`). Reads
// `diffuseColor.rgb` -- already carrying the per-vertex baked team palette
// colour by this point in `main()` (`#include <color_fragment>` runs earlier
// and multiplies it into `diffuseColor`, see `materialsForTeam`'s `color`
// `BufferAttribute`) -- and `normal` / `vViewPosition`, three.js's own
// view-space fragment normal and view direction, already computed by earlier
// chunks (`normal_fragment_begin` / `_maps`). `viewMatrix` and
// `cameraPosition` are three.js built-in uniforms, declared for every
// fragment shader unconditionally (see `WebGLProgram.js`), so moving the
// world-space `LIGHT_DIR` into the same view space `normal` already uses
// needs no extra uniform plumbing.
// rig3d/renderer.lua keeps back faces ("cull mode 'none' so a generator that
// winds a face the wrong way still shades correctly instead of vanishing") and
// spells the correction as `if (!gl_FrontFacing) n = -n;`.
//
// That exact line does NOT port, and copying it verbatim was wrong when this
// port first tried. `gl_FrontFacing` is a RASTER-STATE fact, not a geometric
// one, and the two hosts disagree about it here: pitch.ts's wrapper mirrors Y,
// which reverses triangle winding, and three.js compensates per object
// (`matrixWorld.determinant() < 0 -> frontFaceCW`, see `WebGLRenderer`) where
// LÖVE -- whose Y flip lives in the PROJECTION matrix, not the model -- does
// not. The same visible surface therefore reports `gl_FrontFacing == true`
// under three.js and `false` under LÖVE, so the ported line left every visible
// fragment holding an INWARD normal. Measured on an RTX 2070 SUPER by writing
// the sign of N.V straight to the framebuffer: negative across the entire
// silhouette, which pins `rim` at 1 and reproduces the exact flooding this
// shading-frame work set out to remove.
//
// What the Lua's rule MEANS, independent of raster convention, is: shade the
// side of the surface the viewer can actually see. For closed geometry that is
// exactly `N.V >= 0`, and for a wrongly-wound face it applies the same
// correction the Lua's own version does -- so this is the host-independent
// statement of that rule rather than a different rule. Bands are unaffected in
// character: `band` is driven by N.L, not N.V, so it still varies over a
// figure exactly as before.
const TWO_SIDED_NORMAL = `
    vec3 gcNormal = normalize( ${SHADING_NORMAL_VARYING} );
    if ( dot( gcNormal, gcViewDir ) < 0.0 ) {
      gcNormal = -gcNormal;
    }`;

function celShadingChunk(family: CelFamily): string {
  if (family === "emissive") {
    // rig3d/renderer.lua: `if (v_material > 1.5) { ... return
    // vec4(v_slot_color.rgb * (1.25 + 0.55 * facing), v_slot_color.a); }`.
    return `
    vec3 gcNormal = normalize( ${SHADING_NORMAL_VARYING} );
    vec3 gcViewDir = normalize( ${glslVec3(SHADING_EYE)} - ${SHADING_WORLD_VARYING} );
    float gcFacing = abs( dot( gcNormal, gcViewDir ) );
    reflectedLight.directDiffuse = diffuseColor.rgb * ( ${glslFloat(EMISSIVE_BASE)} + ${glslFloat(EMISSIVE_FACING_SCALE)} * gcFacing );
    `;
  }

  const metalMix = family === "metal" ? glslFloat(1) : glslFloat(0);
  const specular =
    family === "metal"
      ? `
    // Lua: "Metal gets one hard specular band. Skin and cloth get none, and
    // that difference is most of what separates 'kit' from 'body' at a
    // glance."
    vec3 gcHalfDir = normalize( normalize( -${glslVec3(LIGHT_DIR)} ) + gcViewDir );
    float gcSpec = pow( max( dot( gcNormal, gcHalfDir ), 0.0 ), ${glslFloat(SPEC_POWER)} );
    gcLit += ${glslVec3(SPEC_TINT)} * smoothstep( ${glslFloat(SPEC_SMOOTH_LOW)}, ${glslFloat(SPEC_SMOOTH_HIGH)}, gcSpec ) * ${glslFloat(SPEC_SCALE)};
    `
      : "";

  return `
    // The Lua's \`normalize(u_cam_pos - v_world)\`, with \`u_cam_pos\` the
    // deliberately-distant shading eye. No camera state: SHADING_EYE is fixed
    // in the same character frame the varyings are expressed in.
    vec3 gcViewDir = normalize( ${glslVec3(SHADING_EYE)} - ${SHADING_WORLD_VARYING} );
    ${TWO_SIDED_NORMAL}
    float ndl = dot( gcNormal, normalize( -${glslVec3(LIGHT_DIR)} ) );

    float band = ${glslFloat(BAND_LOW)};
    if ( ndl > ${glslFloat(BAND_HIGH_THRESHOLD)} ) {
      band = ${glslFloat(BAND_HIGH)};
    } else if ( ndl > ${glslFloat(BAND_MID_THRESHOLD)} ) {
      band = ${glslFloat(BAND_MID)};
    }

    float bounce = max( -ndl, 0.0 ) * ${glslFloat(BOUNCE_SCALE)};

    float rim = pow( 1.0 - max( dot( gcNormal, gcViewDir ), 0.0 ), ${glslFloat(RIM_POWER)} );
    rim = smoothstep( ${glslFloat(RIM_SMOOTH_LOW)}, ${glslFloat(RIM_SMOOTH_HIGH)}, rim );

    vec3 gcLit = diffuseColor.rgb * ( band + bounce )
      + ${glslVec3(RIM_TINT)} * rim * mix( ${glslFloat(RIM_METAL_MIX_LOW)}, ${glslFloat(RIM_METAL_MIX_HIGH)}, ${metalMix} );
    ${specular}
    reflectedLight.directDiffuse = gcLit;
    `;
}

/**
 * Wires rig3d/renderer.lua's toon shading into a `THREE.MeshStandardMaterial`
 * via `onBeforeCompile`. Call once per material instance -- see
 * `player_renderer_3d.ts`'s `materialsForTeam`, which builds and caches one
 * material per shading family per team, so this runs once per material, not
 * per frame or per draw call.
 *
 * `material.side` is forced to `THREE.DoubleSide`, matching
 * rig3d/renderer.lua's `love.graphics.setMeshCullMode("none")` (kept there so
 * a part whose winding a content generator gets backwards still shades
 * instead of vanishing) -- see `celShadingChunk`'s comment on why that means
 * this file does not ALSO flip the normal by hand.
 *
 * `material.customProgramCacheKey` is set to the family name.
 * `WebGLPrograms.getProgramCacheKey` does not hash `onBeforeCompile`'s
 * function body in this three.js version -- only `customProgramCacheKey()`
 * feeds the cache key -- and `plain` / `metal` / `emissive` are otherwise
 * indistinguishable to that cache (same defines, same `vertexColors: true`,
 * no maps): metalness/roughness differ only as runtime *uniforms*, which
 * this shading does not even read (see this file's header on what a "shading
 * family" now means, a compile-time choice of which chunk got spliced in,
 * not a material property or a per-vertex value). Without this, three.js
 * could hand one family's compiled program to another's draw call.
 */
export function applyCelShading(material: THREE.MeshStandardMaterial, family: CelFamily, elevation: number = ELEVATION): void {
  material.side = THREE.DoubleSide;
  material.customProgramCacheKey = () => family;
  material.onBeforeCompile = (shader) => {
    shader.vertexShader = withShadingFrameVertex(shader.vertexShader, elevation);
    let fragmentShader = shadingFrameFragmentDeclarations() + shader.fragmentShader;
    fragmentShader = fragmentShader.replace("#include <lights_physical_fragment>", celShadingChunk(family));
    for (const include of CEL_SHADING_TARGET_INCLUDES.slice(1)) {
      fragmentShader = fragmentShader.replace(include, "");
    }
    shader.fragmentShader = fragmentShader;
  };
}

// Exposed for this file's own spec: lets a headless test run the exact same
// `String.replace` `applyCelShading`'s `onBeforeCompile` performs, against
// three.js's OWN unmodified shader template (`THREE.ShaderLib.standard`, no
// WebGL context needed to read a template string), to confirm the four
// target includes still exist verbatim in this three.js version and that the
// generated replacement contains the ported numbers -- not just that some
// `onBeforeCompile` function was assigned.
export function shaderChunkFor(family: CelFamily): string {
  return celShadingChunk(family);
}

// ---------------------------------------------------------------------------
// COMBINED (draw-call fix #2, see player_renderer_3d.ts's `materialsForTeam`
// and this file's header). `applyCelShading`/`celShadingChunk` above compile
// one GLSL program PER shading family, selected at TypeScript-build time by
// which `MeshStandardMaterial` instance a caller applies them to --
// `customProgramCacheKey` deliberately keeps the three apart so three.js
// never hands one family's compiled program to another's draw call. That
// reproduces rig3d/renderer.lua's LOOK but not its DRAW-CALL COUNT: three
// materials still need up to three geometry groups, hence up to three draws
// per character, where the Lua shader -- one program, branching on a
// per-vertex `VertexMaterial` varying -- was always exactly one.
//
// `applyCombinedCelShading` below is that one-program mechanism, ported
// directly rather than reinvented: `combinedCelShadingChunk` wraps
// `celShadingChunk("emissive"|"metal"|"plain")` -- the SAME generated text
// `applyCelShading` uses, not a hand-duplicated second copy of the ported
// bands/bounce/rim/specular math -- in a runtime `if / else if / else` on a
// `vMaterialFamily` GLSL float, exactly mirroring rig3d/renderer.lua's own
// `effect()`: `if (v_material > 1.5) { ... } ... float u_metal = v_material
// > 0.5 ? 1.0 : 0.0;`. Each wrapped chunk keeps its own `{ }` block scope, so
// the three branches' identically-named locals (`gcNormal`, `ndl`, ...) do
// not collide -- ordinary C-style block scoping, valid in GLSL ES 1.00 same
// as GLSL ES 3.00.
// ---------------------------------------------------------------------------

function combinedCelShadingChunk(): string {
  return `
    // Runtime branch on the per-vertex shading family (plain low, metal
    // mid, emissive high -- rig3d/geometry.ts's MATERIAL numbering), read
    // off the \`materialFamily\` vertex attribute player_renderer_3d.ts's build()
    // bakes -- exactly rig3d/renderer.lua's effect(): "branching on a
    // varying in the fragment stage is fine here; only dynamic *array
    // indexing* is forbidden." One compiled program covers every family, so
    // this material never needs a geometry group to pick a shader -- see
    // applyCombinedCelShading's doc comment.
    if ( vMaterialFamily > 1.5 ) {
      ${celShadingChunk("emissive")}
    } else if ( vMaterialFamily > 0.5 ) {
      ${celShadingChunk("metal")}
    } else {
      ${celShadingChunk("plain")}
    }
    `;
}

/**
 * Wires rig3d/renderer.lua's toon shading into a `THREE.MeshStandardMaterial`
 * as ONE compiled program that branches on a per-vertex shading-family
 * attribute at runtime, instead of `applyCelShading`'s one-program-per-family
 * split. Call once per material instance -- see `player_renderer_3d.ts`'s
 * `materialsForTeam`, which now builds and caches exactly ONE material per
 * team (not one per family per team), so this runs once per material, not
 * per frame or per draw call.
 *
 * The geometry this material is used with MUST carry a `materialFamily`
 * `THREE.BufferAttribute` -- one float per vertex, `rig3d/geometry.ts`'s
 * `MATERIAL` numbering (0 plain, 1 metal, 2 emissive) -- see
 * `player_renderer_3d.ts`'s `build()`. Three.js auto-binds a vertex attribute
 * to a shader `attribute` of the same name and type
 * (`WebGLProgram`/`WebGLBindingStates`), so declaring `attribute float
 * materialFamily;` below is enough; nothing here reads or validates the
 * geometry directly.
 *
 * `material.side` is forced to `THREE.DoubleSide`, matching
 * rig3d/renderer.lua's `love.graphics.setMeshCullMode("none")`, same as
 * `applyCelShading` above.
 *
 * `material.customProgramCacheKey` is set to a fixed string rather than a
 * per-family one: unlike `applyCelShading`, every material this function
 * touches compiles to the SAME program (the branch is a runtime value, not a
 * `defines`/uniform-shape difference three.js's cache would otherwise
 * conflate with a genuinely different family) -- see `applyCelShading`'s own
 * doc comment for why THAT function needs a per-family key and this one
 * does not.
 */
export function applyCombinedCelShading(material: THREE.MeshStandardMaterial, elevation: number = ELEVATION): void {
  material.side = THREE.DoubleSide;
  material.customProgramCacheKey = () => "combined";
  material.onBeforeCompile = (shader) => {
    let vertexShader = `attribute float materialFamily;\nvarying float vMaterialFamily;\n${withShadingFrameVertex(shader.vertexShader, elevation)}`;
    // `void main() {` appears exactly once in three.js's vertex template
    // (there is only one entry point) -- inserting the varying assignment
    // as the first statement means every later chunk (skinning, project,
    // ...) runs after it, though nothing downstream in the VERTEX stage
    // actually depends on it; only the fragment stage below reads
    // `vMaterialFamily`.
    vertexShader = vertexShader.replace("void main() {", "void main() {\n\n\tvMaterialFamily = materialFamily;\n");
    shader.vertexShader = vertexShader;

    let fragmentShader = `varying float vMaterialFamily;\n${shadingFrameFragmentDeclarations()}${shader.fragmentShader}`;
    fragmentShader = fragmentShader.replace("#include <lights_physical_fragment>", combinedCelShadingChunk());
    for (const include of CEL_SHADING_TARGET_INCLUDES.slice(1)) {
      fragmentShader = fragmentShader.replace(include, "");
    }
    shader.fragmentShader = fragmentShader;
  };
}

// Exposed for this file's own spec, same reason as `shaderChunkFor` above.
export function shaderChunkForCombined(): string {
  return combinedCelShadingChunk();
}
