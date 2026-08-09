// Braziers and floodlight pylons: the coliseum's practical light sources,
// both deliberately overdriven past the bloom threshold (0.55, see
// stadium.ts's file header) so they read as the brightest things on screen.
//
// Draw calls: braziers = 2 (one InstancedMesh for the stone bowls, one for
// the flames). Floodlights = 3 (one InstancedMesh for the four leaning
// pylon bodies, one for the 4x6 lamp bank quads, one for the four light
// cones).

import * as THREE from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { ellipsePoint, ellipseYawFacingCenter } from "./stadium_geo.ts";
import type { StadiumLayout } from "./stadium_layout.ts";
import { prngRange, type Prng } from "./stadium_prng.ts";
import type { NumberUniform } from "./stadium_types.ts";

// Formats a JS number as a syntactically valid GLSL float literal (always a
// decimal point) -- used to bake a couple of layout-derived constants
// (`coneHeight`) into the floodlight cone shader source below.
function glslFloat(n: number): string {
  return Number.isInteger(n) ? `${n}.0` : `${n}`;
}

export interface BraziersBuild {
  readonly group: THREE.Group;
  readonly timeUniforms: readonly NumberUniform[];
}

const FLAME_VERTEX = /* glsl */ `
  attribute float aPhase;
  varying vec2 vUv;
  varying float vPhase;
  void main() {
    vUv = uv;
    vPhase = aPhase;
    vec4 mvPosition = modelViewMatrix * instanceMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mvPosition;
  }
`;

// Cyan core (>1.0 so the bloom pass ignites it), amber tip, alpha shaped by
// a vertical taper (wide base, pinched tip) and a noise-scrolled flicker.
const FLAME_FRAGMENT = /* glsl */ `
  uniform float uTime;
  varying vec2 vUv;
  varying float vPhase;
  float hash1(float n) { return fract(sin(n) * 43758.5453); }
  void main() {
    float t = uTime * 2.6 + vPhase * 6.28318;
    float flicker = 0.7 + 0.3 * sin(t * 3.3 + hash1(vPhase) * 12.0);
    // GLSL's smoothstep is only DEFINED for edge0 < edge1 (spec: "results are
    // undefined" otherwise); this shader used to call it with reversed edges
    // (smoothstep(1.0, 0.1, ...) / smoothstep(0.5, 0.0, ...)) to get a
    // DECREASING curve (1 near the base/centre, 0 toward the tip/side).
    // Found live: that was the actual reason the braziers read as invisible
    // (creative brief item 5) -- both reversed calls evaluated to a constant
    // 0 under this environment's headless SwiftShader renderer, zeroing
    // alpha everywhere. Rewritten below with plain linear clamp() ramps
    // instead of smoothstep at all -- fewer moving parts to get wrong, and
    // no edge-order pitfall to reintroduce on a future edit.
    float taper = clamp(1.0 - vUv.y, 0.0, 1.0);
    float pinch = 0.35 + 0.65 * (1.0 - vUv.y);
    float edge = clamp(1.0 - abs(vUv.x - 0.5) * 2.0 / max(pinch, 0.05), 0.0, 1.0);
    float noise = 0.5 + 0.5 * sin(vUv.y * 9.0 - uTime * 5.0 + vPhase * 5.0);
    float alpha = clamp(taper * edge * flicker * (0.55 + 0.45 * noise), 0.0, 1.0);
    vec3 core = vec3(0.35, 1.5, 1.7);
    vec3 tip = vec3(1.6, 0.95, 0.28);
    vec3 color = mix(core, tip, clamp(vUv.y * 1.15, 0.0, 1.0));
    gl_FragColor = vec4(color * alpha, alpha);
  }
`;

// Scaled 1.5x overall (creative brief item 5: "braziers that read") -- a
// uniform post-build `.scale()` rather than re-deriving every radius/height
// literal, since the proportions that already read as "brazier" should not
// change, only the size.
const BRAZIER_BOWL_SCALE = 1.5;
// The stone bowl's own cup rim, in the SAME local space `buildBraziers`
// anchors both the bowl and flame InstancedMesh instances to (ground level,
// y=0): the bowl cylinder (see `buildBrazierBowlGeometry`) is translated to
// y=26 with height 9, so its top face sits at 26+9/2=30.5 before the 1.5x
// scale. THE ACTUAL BUG this constant fixes: a flame quad built by
// `buildFlameGeometry` alone spans local y=[0, height] -- i.e. starting at
// GROUND LEVEL, entirely BELOW this cup top, so the flame used to render
// fully inside/behind the opaque bowl geometry and lose the WebGL depth
// test every single fragment. Confirmed live (creative brief item 5): with
// the flame material swapped for a same-instance-data debug
// `MeshBasicMaterial`, every brazier was clearly visible at its correct
// position -- placement, instancing and culling were never the problem,
// only this missing vertical offset. `buildFlameGeometry` below takes this
// as an explicit `baseY` so the anchor is never silently dropped again.
const BOWL_CUP_TOP = 30.5 * BRAZIER_BOWL_SCALE;

function buildFlameGeometry(width: number, height: number, baseY: number): THREE.BufferGeometry {
  const plane = new THREE.PlaneGeometry(width, height, 1, 4).translate(0, baseY + height / 2, 0);
  const cross = plane.clone().rotateY(Math.PI / 2);
  const merged = mergeGeometries([plane, cross], false);
  plane.dispose();
  cross.dispose();
  return merged;
}

function buildBrazierBowlGeometry(): THREE.BufferGeometry {
  const bowl = new THREE.CylinderGeometry(7, 5, 9, 10, 1, true).translate(0, 26, 0);
  const stem = new THREE.CylinderGeometry(2.2, 3, 22, 8).translate(0, 11, 0);
  const base = new THREE.CylinderGeometry(5, 5.5, 4, 10).translate(0, 2, 0);
  const merged = mergeGeometries([bowl, stem, base], false);
  bowl.dispose();
  stem.dispose();
  base.dispose();
  merged.scale(BRAZIER_BOWL_SCALE, BRAZIER_BOWL_SCALE, BRAZIER_BOWL_SCALE);
  return merged;
}

/** Builds the "braziers" sub-group: stone bowls on the tier1/tier2 parapet, each holding a plasma flame. */
export function buildBraziers(layout: StadiumLayout, rng: Prng): BraziersBuild {
  const group = new THREE.Group();
  group.name = "braziers";

  const uTime: NumberUniform = { value: 0 };

  const bowlGeometry = buildBrazierBowlGeometry();
  const bowlMaterial = new THREE.MeshStandardMaterial({ color: 0x6a5c4c, roughness: 0.9, metalness: 0.05 });
  const bowls = new THREE.InstancedMesh(bowlGeometry, bowlMaterial, layout.brazierCount);
  bowls.name = "brazier_bowls";

  // Wide/tall enough to actually silhouette above the parapet at broadcast
  // distance (creative brief item 5) -- the old 9x20 quads were reading as
  // invisible slivers. Anchored at BOWL_CUP_TOP (see that constant's own
  // comment for the depth-test bug this fixes), not ground level.
  const flameGeometry = buildFlameGeometry(16, 34, BOWL_CUP_TOP);
  const flameMaterial = new THREE.ShaderMaterial({
    uniforms: { uTime },
    vertexShader: FLAME_VERTEX,
    fragmentShader: FLAME_FRAGMENT,
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    blending: THREE.AdditiveBlending,
  });
  const flames = new THREE.InstancedMesh(flameGeometry, flameMaterial, layout.brazierCount);
  flames.name = "brazier_flames";
  const aPhase = new Float32Array(layout.brazierCount);
  for (let i = 0; i < layout.brazierCount; i += 1) {
    aPhase[i] = prngRange(rng, 0, 1);
  }
  flameGeometry.setAttribute("aPhase", new THREE.InstancedBufferAttribute(aPhase, 1));

  const dummy = new THREE.Object3D();
  for (let i = 0; i < layout.brazierCount; i += 1) {
    const angle = (i / layout.brazierCount) * Math.PI * 2 + prngRange(rng, -0.02, 0.02);
    const [x, z] = ellipsePoint(layout.cx, layout.cz, layout.brazierRx, layout.brazierRz, angle);
    const yaw = ellipseYawFacingCenter(angle, layout.brazierRx, layout.brazierRz);
    dummy.position.set(x, layout.brazierY, z);
    dummy.rotation.set(0, yaw, 0);
    dummy.scale.setScalar(prngRange(rng, 0.9, 1.1));
    dummy.updateMatrix();
    bowls.setMatrixAt(i, dummy.matrix);
    flames.setMatrixAt(i, dummy.matrix);
  }
  bowls.instanceMatrix.needsUpdate = true;
  flames.instanceMatrix.needsUpdate = true;

  group.add(bowls, flames);
  return { group, timeUniforms: [uTime] };
}

export interface FloodlightsBuild {
  readonly group: THREE.Group;
}

function buildPylonBodyGeometry(height: number): THREE.BufferGeometry {
  const mast = new THREE.BoxGeometry(9, height, 9).translate(0, height / 2, 0);
  const footing = new THREE.BoxGeometry(18, height * 0.03, 18).translate(0, height * 0.015, 0);
  const merged = mergeGeometries([mast, footing], false);
  mast.dispose();
  footing.dispose();
  return merged;
}

// A leaning-tower placement: yaw faces the pitch centre, then an additional
// tilt around the (already-yawed) local X axis leans the top of the pylon
// in over the bowl -- the same "rotate local, then reorient by yaw" order
// pitch.ts's `riggedCharacterObject` documents for its own tilt+yaw
// composition (its file header's COMPOSITION ORDER note).
function pylonQuaternion(yaw: number, leanRadians: number): THREE.Quaternion {
  const yawQuat = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw);
  const leanQuat = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), leanRadians);
  return yawQuat.multiply(leanQuat);
}

/** Builds the "floodlights" sub-group: four leaning pylons, each with a lamp bank and a soft light cone. */
export function buildFloodlights(layout: StadiumLayout): FloodlightsBuild {
  const group = new THREE.Group();
  group.name = "floodlights";

  const pylonHeight = 300;
  const leanRadians = 0.32;

  const pylonMaterial = new THREE.MeshStandardMaterial({ color: 0x2a2f38, roughness: 0.6, metalness: 0.6 });
  const pylonGeometry = buildPylonBodyGeometry(pylonHeight);
  const pylons = new THREE.InstancedMesh(pylonGeometry, pylonMaterial, layout.pylonCount);
  pylons.name = "floodlight_pylons";

  const lampMaterial = new THREE.MeshStandardMaterial({ color: 0xfff2d0, emissive: 0xfff2d0, emissiveIntensity: 1.5, roughness: 0.4 });
  const lampGeometry = new THREE.PlaneGeometry(7, 4);
  const lampsPerPylon = 6;
  const lamps = new THREE.InstancedMesh(lampGeometry, lampMaterial, layout.pylonCount * lampsPerPylon);
  lamps.name = "floodlight_lamps";

  const coneHeight = pylonHeight * 0.4;
  const coneMaterial = new THREE.ShaderMaterial({
    uniforms: {},
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    blending: THREE.AdditiveBlending,
    vertexShader: /* glsl */ `
      varying float vT;
      void main() {
        // ConeGeometry's apex sits at local y = +height/2 (radiusTop 0),
        // its base at y = -height/2. Remap to [0, 1] with 1 AT the apex
        // (where the lamp bank is) so the fragment shader can fade OUT
        // toward the base without a second flip.
        vT = (position.y + ${glslFloat(coneHeight / 2)}) / ${glslFloat(coneHeight)};
        vec4 mvPosition = modelViewMatrix * instanceMatrix * vec4(position, 1.0);
        gl_Position = projectionMatrix * mvPosition;
      }
    `,
    fragmentShader: /* glsl */ `
      varying float vT;
      void main() {
        // Fades out from the apex toward the base -- a soft light cone, not
        // a hard-edged shape. Kept well under the bloom threshold (~0.06 max
        // alpha, spec's own number) so it reads as haze, not glow.
        float fade = clamp(vT, 0.0, 1.0);
        gl_FragColor = vec4(vec3(0.9, 0.95, 1.0) * fade, fade * 0.06);
      }
    `,
  });
  const coneGeometry = new THREE.ConeGeometry(coneHeight * 0.55, coneHeight, 12, 1, true);
  const cones = new THREE.InstancedMesh(coneGeometry, coneMaterial, layout.pylonCount);
  cones.name = "floodlight_cones";

  const pylonDummy = new THREE.Object3D();
  const lampDummy = new THREE.Object3D();
  const coneDummy = new THREE.Object3D();
  const pitchCentre = new THREE.Vector3(layout.cx, 0, layout.cz);

  for (let i = 0; i < layout.pylonCount; i += 1) {
    const angle = (Math.PI / 4) + i * (Math.PI / 2);
    const [x, z] = ellipsePoint(layout.cx, layout.cz, layout.pylonRx, layout.pylonRz, angle);
    const yaw = ellipseYawFacingCenter(angle, layout.pylonRx, layout.pylonRz);
    const quat = pylonQuaternion(yaw, leanRadians);

    pylonDummy.position.set(x, 0, z);
    pylonDummy.quaternion.copy(quat);
    pylonDummy.scale.setScalar(1);
    pylonDummy.updateMatrix();
    pylons.setMatrixAt(i, pylonDummy.matrix);

    // Lamp bank near the top of the (already leaned) mast: offsets are in
    // the pylon's own local frame, so they inherit the same lean/yaw via
    // `quat` rather than being computed against world axes.
    const lampBankLocalY = pylonHeight * 0.92;
    for (let l = 0; l < lampsPerPylon; l += 1) {
      const row = Math.floor(l / 3);
      const col = l % 3;
      const localOffset = new THREE.Vector3((col - 1) * 8, lampBankLocalY + (row === 0 ? 6 : -6), 5).applyQuaternion(quat);
      const lampPos = new THREE.Vector3(x, 0, z).add(localOffset);
      lampDummy.position.copy(lampPos);
      // Lamps face straight at the pitch centre from wherever they end up,
      // independent of the mast's own lean -- floodlight panels are aimed,
      // not welded flush to the tower.
      const lampYaw = Math.atan2(pitchCentre.x - lampPos.x, pitchCentre.z - lampPos.z);
      lampDummy.rotation.set(0, lampYaw, 0);
      lampDummy.updateMatrix();
      lamps.setMatrixAt(i * lampsPerPylon + l, lampDummy.matrix);
    }

    // Light cone: apex at the lamp bank, opening toward the pitch centre at
    // ground level. `ConeGeometry` centres its apex at local +height/2, so
    // the mesh origin sits `height/2` back from the apex along the cone's
    // own pointing direction once rotated.
    const apexWorld = new THREE.Vector3(x, 0, z).add(new THREE.Vector3(0, lampBankLocalY, 0).applyQuaternion(quat));
    const aimTarget = new THREE.Vector3(layout.cx, 0, layout.cz);
    const aimDir = aimTarget.clone().sub(apexWorld).normalize();
    const coneQuat = new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, -1, 0), aimDir);
    const apexLocalOffset = new THREE.Vector3(0, coneHeight / 2, 0).applyQuaternion(coneQuat);
    coneDummy.position.copy(apexWorld).sub(apexLocalOffset);
    coneDummy.quaternion.copy(coneQuat);
    coneDummy.updateMatrix();
    cones.setMatrixAt(i, coneDummy.matrix);
  }

  pylons.instanceMatrix.needsUpdate = true;
  lamps.instanceMatrix.needsUpdate = true;
  cones.instanceMatrix.needsUpdate = true;

  group.add(pylons, lamps, cones);
  return { group };
}
