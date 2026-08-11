// The crowd: thousands of low-poly spectator blobs seated across the three
// tiers, animated entirely in the VERTEX SHADER so `Stadium.update` stays a
// couple of uniform writes regardless of instance count (per this task's
// perf budget: "update(now) < 0.2ms CPU, uniform writes only").
//
// TESTABILITY NOTE. `onBeforeCompile` only ever runs when three.js actually
// compiles a `WebGLProgram`, which needs a real GL context this workspace's
// headless vitest run never has (see draw2d.ts's file header on the same
// boundary). So the uniform OBJECTS this module hands the material
// (`uTime`/`uExcitement` below) are created up front and returned to the
// caller BEFORE they are ever bound into a shader -- `stadium.ts`'s
// `update`/`setExcitement` mutate `.value` on those same objects directly,
// which is fully testable without a renderer; only the GLSL string-splicing
// itself (untestable without one) lives inside `onBeforeCompile`.
//
// Draw calls: 2 in "high" quality (a higher-poly InstancedMesh for the tier
// closest to the pitch, a cheaper one for the two tiers behind it), 1 in
// "low" quality (a single low-poly InstancedMesh spanning all three tiers).

import * as THREE from "three";
import type { RGB } from "./draw2d.ts";
import type { StadiumLayout, StadiumQuality } from "./stadium_layout.ts";
import { prngRange, type Prng } from "./stadium_prng.ts";
import type { NumberUniform } from "./stadium_types.ts";

export interface CrowdBuild {
  readonly group: THREE.Group;
  readonly timeUniforms: readonly NumberUniform[];
  readonly excitementUniforms: readonly NumberUniform[];
}

// GLSL injected after `#include <begin_vertex>` (which has just set
// `transformed = vec3(position)`, in the blob's own local space, before the
// instance/model/view matrices are applied -- see project_vertex.glsl.js).
// Offsetting `transformed.y` here is therefore a LOCAL vertical lift, which
// is what "bounce" means for a seated blob: every vertex of one instance
// moves together, so the capsule keeps its shape and just hops.
const BOUNCE_VERTEX_CHUNK = /* glsl */ `
  vec3 iWorldPos = (modelMatrix * instanceMatrix * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
  vec2 fromCenter = vec2((iWorldPos.x - uCenter.x) / uRadii.x, (iWorldPos.z - uCenter.y) / uRadii.y);
  float seatAngle = atan(fromCenter.y, fromCenter.x);
  float seatHash = fract(sin(dot(iWorldPos.xz, vec2(12.9898, 78.233))) * 43758.5453);
  float phase = seatHash * 6.28318 + seatAngle * 3.0;
  float amp = 0.6 + uExcitement * 2.5;
  // A slow "mexican wave" ring travelling around the ellipse angle: a bright
  // band of extra amplitude circles the bowl over time, most visible once
  // uExcitement has already raised the base amplitude.
  float waveEnvelope = 0.6 + 0.4 * sin(seatAngle * 1.0 - uTime * 1.2);
  float bounce = abs(sin(uTime * (2.2 + seatHash * 0.6) + phase)) * amp * waveEnvelope;
  transformed.y += bounce;
`;

function injectBounceShader(material: THREE.Material, uTime: NumberUniform, uExcitement: NumberUniform, layout: StadiumLayout): void {
  material.onBeforeCompile = (shader: THREE.WebGLProgramParametersWithUniforms) => {
    shader.uniforms["uTime"] = uTime;
    shader.uniforms["uExcitement"] = uExcitement;
    shader.uniforms["uCenter"] = { value: new THREE.Vector2(layout.cx, layout.cz) };
    shader.uniforms["uRadii"] = { value: new THREE.Vector2(layout.bowlOuterRx, layout.bowlOuterRz) };
    shader.vertexShader = shader.vertexShader
      .replace(
        "#include <common>",
        "#include <common>\nuniform float uTime;\nuniform float uExcitement;\nuniform vec2 uCenter;\nuniform vec2 uRadii;",
      )
      .replace("#include <begin_vertex>", `#include <begin_vertex>\n${BOUNCE_VERTEX_CHUNK}`);
  };
}

// A lively mix, roughly: 8% bright accent "sparkle" seats, then the
// remainder split ~40% home-tinted / 40% away-tinted / 20% neutral pastel
// (spec's own split), each with independent brightness jitter so no two
// seats in the same bucket are identical. The white-lerp on the team buckets
// dropped 0.3 -> 0.15 (creative brief item 7): the old 0.3 washed the tint
// most of the way to white, reading as a flat tan bowl instead of saturated
// team colour at dusk.
function seatColor(rng: Prng, home: RGB, away: RGB): THREE.Color {
  const accentRoll = rng();
  if (accentRoll < 0.08) {
    // A hot, saturated sparkle seat -- deliberately pushed above 1.0 so a
    // handful of seats catch the light and read as glints across the bowl,
    // per the brief's "stands sparkle a little at dusk".
    const accent = new THREE.Color().setHSL(rng(), 0.7, 0.8);
    accent.multiplyScalar(1.15 + rng() * 0.35);
    return accent;
  }
  const bucket = rng();
  let color: THREE.Color;
  if (bucket < 0.4) {
    color = new THREE.Color(home[0], home[1], home[2]).lerp(new THREE.Color(1, 1, 1), 0.15);
  } else if (bucket < 0.8) {
    color = new THREE.Color(away[0], away[1], away[2]).lerp(new THREE.Color(1, 1, 1), 0.15);
  } else {
    color = new THREE.Color().setHSL(rng(), 0.22, 0.58 + rng() * 0.18);
  }
  color.multiplyScalar(0.72 + rng() * 0.4);
  return color;
}

// One seated position on `tierIndex`, jittered but held safely inside
// `[tier inner edge, bowl outer rim]` -- see stadium.spec.ts's ellipse/rect
// containment assertions, which this placement is designed to always satisfy:
// the minimum radial offset from the tier's own inner edge (`u >= 0.1`) is
// large enough that the +/-2.5 unit position jitter can never push a seat
// back across the inner boundary (and therefore never into the pitch rect,
// which sits entirely inside that boundary -- see stadium_layout.ts), and the
// explicit clamp below is the belt-and-suspenders check against the outer rim.
function placeSeat(layout: StadiumLayout, tierIndex: number, rng: Prng): { readonly x: number; readonly y: number; readonly z: number } {
  const rxInner = layout.tierInnerRx[tierIndex] ?? layout.bowlInnerRx;
  const rzInner = layout.tierInnerRz[tierIndex] ?? layout.bowlInnerRz;
  const baseY = layout.tierBaseY[tierIndex] ?? 0;
  const u = prngRange(rng, 0.1, 0.9);
  const radialT = layout.parapetDepth + u * (layout.tierWidth - layout.parapetDepth);
  const heightT = layout.parapetHeight + u * (layout.tierHeight - layout.parapetHeight);
  const angle = prngRange(rng, 0, Math.PI * 2);
  const rx = rxInner + radialT;
  const rz = rzInner + radialT;
  let x = layout.cx + rx * Math.cos(angle) + prngRange(rng, -2.5, 2.5);
  let z = layout.cz + rz * Math.sin(angle) + prngRange(rng, -2.5, 2.5);
  const y = baseY + heightT + 4.5;

  const nx = (x - layout.cx) / layout.bowlOuterRx;
  const nz = (z - layout.cz) / layout.bowlOuterRz;
  const norm = Math.sqrt(nx * nx + nz * nz);
  if (norm > 0.995) {
    const s = 0.995 / norm;
    x = layout.cx + nx * layout.bowlOuterRx * s;
    z = layout.cz + nz * layout.bowlOuterRz * s;
  }
  return { x, y, z };
}

function populateMesh(mesh: THREE.InstancedMesh, count: number, tiers: readonly number[], layout: StadiumLayout, home: RGB, away: RGB, rng: Prng): void {
  const dummy = new THREE.Object3D();
  for (let i = 0; i < count; i += 1) {
    const tierIndex = tiers[i % tiers.length] ?? 0;
    const seat = placeSeat(layout, tierIndex, rng);
    const scale = prngRange(rng, 0.85, 1.2);
    dummy.position.set(seat.x, seat.y, seat.z);
    dummy.rotation.set(0, prngRange(rng, 0, Math.PI * 2), 0);
    dummy.scale.set(scale, scale, scale);
    dummy.updateMatrix();
    mesh.setMatrixAt(i, dummy.matrix);
    mesh.setColorAt(i, seatColor(rng, home, away));
  }
  mesh.instanceMatrix.needsUpdate = true;
  if (mesh.instanceColor !== null) {
    mesh.instanceColor.needsUpdate = true;
  }
}

function buildBlobGeometry(capSegments: number, radialSegments: number): THREE.BufferGeometry {
  // Roughly 9-10 units tall (spec: "8-12 units"), radius 2.4.
  return new THREE.CapsuleGeometry(2.4, 4.6, capSegments, radialSegments);
}

/** Builds the "crowd" sub-group. See file header for the animation/testability design. */
export function buildCrowd(layout: StadiumLayout, homeColor: RGB, awayColor: RGB, quality: StadiumQuality, rng: Prng): CrowdBuild {
  const group = new THREE.Group();
  group.name = "crowd";

  const uTime: NumberUniform = { value: 0 };
  const uExcitement: NumberUniform = { value: 0 };

  // NO `vertexColors: true` here: that define makes the shader multiply by the
  // geometry's `color` ATTRIBUTE, which capsules don't have -- an unbound
  // attribute reads (0,0,0) and every blob went black (found live, first
  // stadium screenshot). `setColorAt` alone is what enables the per-instance
  // tint (`USE_INSTANCING_COLOR`, keyed off `mesh.instanceColor`, not off this
  // material flag).
  const material = new THREE.MeshLambertMaterial();
  injectBounceShader(material, uTime, uExcitement, layout);

  if (quality === "high") {
    const nearCount = 4200;
    const farCount = 8200;
    const nearGeometry = buildBlobGeometry(4, 10);
    const farGeometry = buildBlobGeometry(2, 6);
    const near = new THREE.InstancedMesh(nearGeometry, material, nearCount);
    near.name = "crowd_near";
    const far = new THREE.InstancedMesh(farGeometry, material, farCount);
    far.name = "crowd_far";
    populateMesh(near, nearCount, [0], layout, homeColor, awayColor, rng);
    populateMesh(far, farCount, [1, 2], layout, homeColor, awayColor, rng);
    group.add(near, far);
  } else {
    const count = 2500;
    const geometry = buildBlobGeometry(2, 5);
    const all = new THREE.InstancedMesh(geometry, material, count);
    all.name = "crowd_all";
    populateMesh(all, count, [0, 1, 2], layout, homeColor, awayColor, rng);
    group.add(all);
  }

  return { group, timeUniforms: [uTime], excitementUniforms: [uExcitement] };
}
