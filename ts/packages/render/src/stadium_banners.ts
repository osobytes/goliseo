// Banners & standards: team-coloured cloth hanging from the inside of the
// top arcade ring, plus a few grand horizontal banners over the four tunnel
// gates. Both share one flutter shader (vertex-shader sideways sine
// displacement, scaled by distance from the anchored/hanging edge, injected
// via `onBeforeCompile` -- same testability shape as stadium_crowd.ts's
// bounce shader: the `uTime` uniform object is created and handed back
// BEFORE it is ever bound into compiled GLSL, so `Stadium.update` can
// exercise it headlessly).
//
// Draw calls: 2 (one InstancedMesh for the ~20 vertical banners, one for the
// 4 horizontal gate banners).

import * as THREE from "three";
import type { RGB } from "./draw2d.ts";
import { ellipsePoint, ellipseYawFacingCenter } from "./stadium_geo.ts";
import type { StadiumLayout } from "./stadium_layout.ts";
import { prngRange, type Prng } from "./stadium_prng.ts";
import type { NumberUniform } from "./stadium_types.ts";

export interface BannersBuild {
  readonly group: THREE.Group;
  readonly timeUniforms: readonly NumberUniform[];
}

const FLUTTER_VERTEX_CHUNK = /* glsl */ `
  // transformed.y is 0 at the hanging (anchored) edge and negative toward
  // the free edge (the plane is built pre-translated so its top sits at
  // local y = 0) -- distance from the anchor is therefore -transformed.y,
  // growing toward the free edge exactly where real cloth swings the most.
  float distFromHang = -transformed.y;
  float sway = sin(uTime * 1.6 + aPhase * 6.28318) * distFromHang * 0.05;
  transformed.x += sway;
`;

function injectFlutterShader(material: THREE.Material, uTime: NumberUniform): void {
  material.onBeforeCompile = (shader: THREE.WebGLProgramParametersWithUniforms) => {
    shader.uniforms["uTime"] = uTime;
    shader.vertexShader = shader.vertexShader
      .replace(
        "#include <common>",
        "#include <common>\nuniform float uTime;\nattribute float aPhase;",
      )
      .replace("#include <begin_vertex>", `#include <begin_vertex>\n${FLUTTER_VERTEX_CHUNK}`)
      // Per-instance emissive tint: `vColor` already carries the instance
      // colour (three defines USE_INSTANCING_COLOR/USE_COLOR automatically
      // for an InstancedMesh with `instanceColor` set -- see stadium_crowd.ts's
      // file header for the same mechanism), so the cloth's OWN emissive glow
      // reads as the same team colour as its diffuse tint instead of a flat
      // uniform warmth.
      .replace(
        "#include <emissivemap_fragment>",
        "#include <emissivemap_fragment>\ntotalEmissiveRadiance *= vColor;",
      );
  };
}

function buildHangingPlane(
  width: number,
  height: number,
  heightSegments: number,
): THREE.BufferGeometry {
  return new THREE.PlaneGeometry(width, height, 1, heightSegments).translate(0, -height / 2, 0);
}

function bannerColor(rng: Prng, home: RGB, away: RGB, isHome: boolean): THREE.Color {
  const base = isHome ? home : away;
  const color = new THREE.Color(base[0], base[1], base[2]);
  color.multiplyScalar(0.85 + rng() * 0.25);
  return color;
}

/** Builds the "banners" sub-group. See file header for the shared flutter shader. */
export function buildBanners(
  layout: StadiumLayout,
  homeColor: RGB,
  awayColor: RGB,
  rng: Prng,
): BannersBuild {
  const group = new THREE.Group();
  group.name = "banners";

  const uTime: NumberUniform = { value: 0 };
  // Same live-found defect as stadium_crowd.ts: `vertexColors: true` reads a
  // nonexistent geometry `color` attribute as (0,0,0) and blacked the cloth
  // out entirely. Per-instance tint comes from `setColorAt` alone.
  const material = new THREE.MeshStandardMaterial({
    roughness: 0.75,
    metalness: 0.0,
    side: THREE.DoubleSide,
    emissive: 0xffffff,
    emissiveIntensity: 0.35,
  });
  injectFlutterShader(material, uTime);

  // Vertical banners, alternating home/away, hanging from just inside the
  // arcade ring.
  const verticalGeometry = buildHangingPlane(8, layout.bannerHeight, 10);
  const verticalPhase = new Float32Array(layout.bannerCount);
  for (let i = 0; i < layout.bannerCount; i += 1) {
    verticalPhase[i] = prngRange(rng, 0, 1);
  }
  verticalGeometry.setAttribute("aPhase", new THREE.InstancedBufferAttribute(verticalPhase, 1));
  const vertical = new THREE.InstancedMesh(verticalGeometry, material, layout.bannerCount);
  vertical.name = "banners_vertical";

  const dummy = new THREE.Object3D();
  for (let i = 0; i < layout.bannerCount; i += 1) {
    const angle = (i / layout.bannerCount) * Math.PI * 2;
    const [x, z] = ellipsePoint(layout.cx, layout.cz, layout.bannerRx, layout.bannerRz, angle);
    const yaw = ellipseYawFacingCenter(angle, layout.bannerRx, layout.bannerRz);
    dummy.position.set(x, layout.bannerTopY, z);
    dummy.rotation.set(0, yaw, 0);
    dummy.scale.setScalar(1);
    dummy.updateMatrix();
    vertical.setMatrixAt(i, dummy.matrix);
    vertical.setColorAt(i, bannerColor(rng, homeColor, awayColor, i % 2 === 0));
  }
  vertical.instanceMatrix.needsUpdate = true;
  if (vertical.instanceColor !== null) {
    vertical.instanceColor.needsUpdate = true;
  }

  // Grand horizontal banners over the four tunnel gates -- wider, shorter,
  // sharing the same flutter material/uniform.
  const gateGeometry = buildHangingPlane(layout.gateWidth * 0.85, 16, 6);
  const gatePhase = new Float32Array(layout.gateAngles.length);
  for (let i = 0; i < layout.gateAngles.length; i += 1) {
    gatePhase[i] = prngRange(rng, 0, 1);
  }
  gateGeometry.setAttribute("aPhase", new THREE.InstancedBufferAttribute(gatePhase, 1));
  const gates = new THREE.InstancedMesh(gateGeometry, material, layout.gateAngles.length);
  gates.name = "banners_gates";
  for (let i = 0; i < layout.gateAngles.length; i += 1) {
    const angle = layout.gateAngles[i] ?? 0;
    const [x, z] = ellipsePoint(
      layout.cx,
      layout.cz,
      layout.apronOuterRx,
      layout.apronOuterRz,
      angle,
    );
    const yaw = ellipseYawFacingCenter(angle, layout.apronOuterRx, layout.apronOuterRz);
    dummy.position.set(x, layout.gateHeight + 12, z);
    dummy.rotation.set(0, yaw, 0);
    dummy.scale.setScalar(1);
    dummy.updateMatrix();
    gates.setMatrixAt(i, dummy.matrix);
    gates.setColorAt(i, bannerColor(rng, homeColor, awayColor, i % 2 === 0));
  }
  gates.instanceMatrix.needsUpdate = true;
  if (gates.instanceColor !== null) {
    gates.instanceColor.needsUpdate = true;
  }

  group.add(vertical, gates);
  return { group, timeUniforms: [uTime] };
}
