// The apron: the ground ring between the pitch and the bowl's inner wall,
// plus the four tunnel gates that break it up.
//
// GEOMETRY DEVIATION FROM THE LITERAL BRIEF, noted as this file's header
// asks: the brief offers two shapes -- "a large flat ring (or plane with the
// pitch rectangle excluded via shader)". This module builds neither a
// washer-shaped ring nor a shader `discard` over the pitch rectangle; it
// builds a full elliptical DISK from (near) the pitch centre out to the
// bowl's inner wall, at `y = 0`. stadium.ts's pitch surface (stadium_pitch_surface.ts)
// sits at `y = 0.5` and exactly covers the pitch rectangle, so the disk
// underneath it is simply occluded there -- geometrically simpler than
// carving a rectangular hole out of an elliptical mesh, and behaviourally
// identical to the shader-`discard` option (nothing the apron draws under
// the pitch is ever visible), without the fragment-shader branch that option
// would need.
//
// Draw calls: 1 (apron disk) + 1 (end-gate frames, instanced x2) + 1
// (end-gate interior glow, instanced x2) + 1 (side-gate scoreboard bezels,
// instanced x2) + 1 (side-gate scoreboard faces, instanced x2) = 5.

import * as THREE from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import type { ArenaColors } from "./arena.ts";
import { buildRevolvedRing, ellipsePoint, ellipseYawFacingCenter } from "./stadium_geo.ts";
import type { StadiumLayout } from "./stadium_layout.ts";
import type { Prng } from "./stadium_prng.ts";
import type { NumberUniform } from "./stadium_types.ts";

export interface ApronBuild {
  readonly group: THREE.Group;
  readonly timeUniforms: readonly NumberUniform[];
}

const APRON_VERTEX = /* glsl */ `
  uniform vec2 uCenter;
  uniform vec2 uOuterRadii;
  varying vec2 vXZ;
  varying float vDistNorm;
  void main() {
    vec4 world = modelMatrix * vec4(position, 1.0);
    vXZ = world.xz;
    vDistNorm = length(vec2((world.x - uCenter.x) / uOuterRadii.x, (world.z - uCenter.y) / uOuterRadii.y));
    gl_Position = projectionMatrix * viewMatrix * world;
  }
`;

// Half-size basalt tile (creative brief item 3: "much darker base, tile size
// ~half, seams at ~1/3 the current brightness") with thin, quiet cyan energy
// seams pulsing slowly, plus an amber AO-ish darkening toward the bowl wall
// (`vDistNorm` -> 1 at the wall, unchanged from the original -- the brief
// asks to KEEP this part). The pitch surface (stadium_pitch_surface.ts) must
// stay the brightest ground plane in frame; this apron is a quiet dark frame
// around it, not a competing pattern.
const APRON_FRAGMENT = /* glsl */ `
  uniform float uTime;
  uniform vec3 uRailColor;
  uniform vec3 uHighlightColor;
  varying vec2 vXZ;
  varying float vDistNorm;
  void main() {
    vec3 basalt = vec3(0.063, 0.075, 0.094);
    float tile = 30.0;
    vec2 g = mod(vXZ, tile);
    vec2 gridDist = min(g, tile - g);
    float seam = clamp(smoothstep(1.3, 0.0, gridDist.x) + smoothstep(1.3, 0.0, gridDist.y), 0.0, 1.0);
    float pulse = 0.55 + 0.45 * sin(uTime * 0.5);
    vec3 seamColor = uRailColor * (0.5 + 0.5 * pulse) * 0.45;
    vec3 color = basalt + seamColor * seam * 0.85;
    float ao = smoothstep(0.35, 1.0, vDistNorm);
    color = mix(color, color * (vec3(1.0) - uHighlightColor * 0.4), ao * 0.6);
    gl_FragColor = vec4(color, 1.0);
  }
`;

function buildGateFrameGeometry(width: number, height: number): THREE.BufferGeometry {
  const pillarWidth = 8;
  const pillarL = new THREE.BoxGeometry(pillarWidth, height, pillarWidth).translate(-width / 2, height / 2, 0);
  const pillarR = new THREE.BoxGeometry(pillarWidth, height, pillarWidth).translate(width / 2, height / 2, 0);
  const lintel = new THREE.BoxGeometry(width + pillarWidth, height * 0.22, pillarWidth).translate(0, height + height * 0.11, 0);
  const merged = mergeGeometries([pillarL, pillarR, lintel], false);
  pillarL.dispose();
  pillarR.dispose();
  lintel.dispose();
  return merged;
}

// `layout.gateAngles` is `[0, PI/2, PI, 3*PI/2]` (see stadium_layout.ts): the
// 0/PI pair sits on the ellipse's x-extremes, aligned with the pitch's own
// goal-line ends (the END gates, directly behind each goal); the PI/2/
// 3*PI/2 pair sits on the z-extremes, aligned with the touchlines (the SIDE
// gates). Read off `cos`/`sin` magnitude rather than hard-coding indices, so
// this keeps matching `gateAngles` even if that array's order or count ever
// changes.
function isEndGateAngle(angle: number): boolean {
  return Math.abs(Math.cos(angle)) > Math.abs(Math.sin(angle));
}

// Creative brief item 6: the far SIDE gate reads as a floating tan billboard
// over the stands from the broadcast camera. `scoreboardQuaternion` composes
// the same "yaw to face the pitch, then tilt" order stadium_props.ts's
// `pylonQuaternion` documents, so the scoreboard leans its top edge down
// toward the pitch instead of standing bolt upright like a gate post.
function scoreboardQuaternion(yaw: number, tiltRadians: number): THREE.Quaternion {
  const yawQuat = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), yaw);
  const tiltQuat = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), tiltRadians);
  return yawQuat.multiply(tiltQuat);
}

const SCOREBOARD_TILT = -0.1;

function buildScoreboardBezelGeometry(width: number, height: number): THREE.BufferGeometry {
  const depth = 6;
  return new THREE.BoxGeometry(width, height, depth).translate(0, height / 2, -depth / 2 - 0.4);
}

function buildScoreboardFaceGeometry(width: number, height: number): THREE.BufferGeometry {
  return new THREE.PlaneGeometry(width, height).translate(0, height / 2, 0.3);
}

// Holographic face: mostly a dim, unlit-reading cyan wash (subtle horizontal
// scanlines, a slow diagonal shimmer) with one brighter border line pushed
// over the bloom threshold (0.55) so the screen edge visibly glows -- "no
// text needed", per the brief, just enough motion and glow to read as an
// active holo-scoreboard instead of a painted panel.
const SCOREBOARD_VERTEX = /* glsl */ `
  varying vec2 vUv;
  void main() {
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0);
  }
`;
const SCOREBOARD_FRAGMENT = /* glsl */ `
  uniform float uTime;
  varying vec2 vUv;
  void main() {
    vec3 base = vec3(0.015, 0.05, 0.065);
    vec3 cyan = vec3(0.15, 0.65, 0.85);
    float scanline = smoothstep(0.7, 1.0, sin(vUv.y * 90.0 - uTime * 2.2) * 0.5 + 0.5) * 0.12;
    float shimmer = 0.5 + 0.5 * sin(uTime * 0.7 + vUv.x * 5.0 - vUv.y * 3.0);
    vec3 color = base + cyan * (0.16 + scanline + shimmer * 0.08);
    float border = step(0.955, max(abs(vUv.x - 0.5) * 2.0, abs(vUv.y - 0.5) * 2.0));
    color = mix(color, cyan * 1.4, border);
    gl_FragColor = vec4(color, 1.0);
  }
`;

/** Builds the "apron" sub-group: the ground disk, the two end-gate tunnels (unchanged tan-glow look), and the two side-gate holo-scoreboards. */
export function buildApron(layout: StadiumLayout, arena: ArenaColors, rng: Prng): ApronBuild {
  const group = new THREE.Group();
  group.name = "apron";

  const uTime: NumberUniform = { value: 0 };
  const diskGeometry = buildRevolvedRing({
    cx: layout.cx,
    cz: layout.cz,
    rxInner: 1,
    rzInner: 1,
    width: layout.apronOuterRx - 1,
    height: 0,
    baseY: layout.apronY,
    steps: 1,
    segments: layout.tierSegments,
    baseColor: [0, 0, 0],
    tint: 0,
    rng,
  });
  const diskMaterial = new THREE.ShaderMaterial({
    uniforms: {
      uTime,
      uCenter: { value: new THREE.Vector2(layout.cx, layout.cz) },
      uOuterRadii: { value: new THREE.Vector2(layout.apronOuterRx, layout.apronOuterRz) },
      uRailColor: { value: new THREE.Color(arena.rail_color[0], arena.rail_color[1], arena.rail_color[2]) },
      uHighlightColor: { value: new THREE.Color(arena.highlight_color[0], arena.highlight_color[1], arena.highlight_color[2]) },
    },
    vertexShader: APRON_VERTEX,
    fragmentShader: APRON_FRAGMENT,
  });
  const disk = new THREE.Mesh(diskGeometry, diskMaterial);
  disk.name = "apron_disk";
  group.add(disk);

  const endAngles = layout.gateAngles.filter((a) => isEndGateAngle(a));
  const sideAngles = layout.gateAngles.filter((a) => !isEndGateAngle(a));

  const gateFrameMaterial = new THREE.MeshStandardMaterial({ color: 0x1a1712, roughness: 0.95, metalness: 0.05 });
  const gateFrameGeometry = buildGateFrameGeometry(layout.gateWidth, layout.gateHeight);
  const gateFrames = new THREE.InstancedMesh(gateFrameGeometry, gateFrameMaterial, Math.max(1, endAngles.length));
  gateFrames.name = "apron_gate_frames";

  // A dim ember interior, not a beacon: at 1.4 the interior plane rendered as
  // a blazing yellow rectangle that out-shone the pitch (found live, first
  // stadium screenshot). 0.38 keeps it under the bloom threshold (0.55) --
  // warm depth behind the arch, no glow halo -- and the darker multiply on
  // the base color keeps the lit face from reading as flat paint.
  const glowColor = new THREE.Color(arena.highlight_color[0], arena.highlight_color[1], arena.highlight_color[2]);
  const gateGlowMaterial = new THREE.MeshStandardMaterial({
    color: glowColor.clone().multiplyScalar(0.35),
    emissive: glowColor,
    emissiveIntensity: 0.38,
    roughness: 0.5,
  });
  const gateGlowGeometry = new THREE.PlaneGeometry(layout.gateWidth - 12, layout.gateHeight * 0.85).translate(0, (layout.gateHeight * 0.85) / 2, 0.1);
  const gateGlows = new THREE.InstancedMesh(gateGlowGeometry, gateGlowMaterial, Math.max(1, endAngles.length));
  gateGlows.name = "apron_gate_glow";

  const dummy = new THREE.Object3D();
  for (let i = 0; i < endAngles.length; i += 1) {
    const angle = endAngles[i] ?? 0;
    const [x, z] = ellipsePoint(layout.cx, layout.cz, layout.apronOuterRx, layout.apronOuterRz, angle);
    const yaw = ellipseYawFacingCenter(angle, layout.apronOuterRx, layout.apronOuterRz);
    dummy.position.set(x, 0, z);
    dummy.rotation.set(0, yaw, 0);
    dummy.scale.setScalar(1);
    dummy.updateMatrix();
    gateFrames.setMatrixAt(i, dummy.matrix);
    gateGlows.setMatrixAt(i, dummy.matrix);
  }
  gateFrames.instanceMatrix.needsUpdate = true;
  gateGlows.instanceMatrix.needsUpdate = true;

  group.add(gateFrames, gateGlows);

  // Side-gate holo-scoreboards (creative brief item 6). Dark bezel first
  // (opaque, ordinary lit material) then the holo face on top, both sharing
  // the apron's own `uTime` so the shimmer/pulse and the apron seam pulse
  // stay on one clock.
  const boardWidth = layout.gateWidth * 0.85;
  const boardHeight = layout.gateHeight * 0.9;
  const bezelMaterial = new THREE.MeshStandardMaterial({ color: 0x0a0c10, roughness: 0.6, metalness: 0.2 });
  const bezelGeometry = buildScoreboardBezelGeometry(boardWidth + 10, boardHeight + 10);
  const bezels = new THREE.InstancedMesh(bezelGeometry, bezelMaterial, Math.max(1, sideAngles.length));
  bezels.name = "apron_scoreboard_bezels";

  const faceMaterial = new THREE.ShaderMaterial({
    uniforms: { uTime },
    vertexShader: SCOREBOARD_VERTEX,
    fragmentShader: SCOREBOARD_FRAGMENT,
  });
  const faceGeometry = buildScoreboardFaceGeometry(boardWidth, boardHeight);
  const faces = new THREE.InstancedMesh(faceGeometry, faceMaterial, Math.max(1, sideAngles.length));
  faces.name = "apron_scoreboard_faces";

  for (let i = 0; i < sideAngles.length; i += 1) {
    const angle = sideAngles[i] ?? 0;
    const [x, z] = ellipsePoint(layout.cx, layout.cz, layout.apronOuterRx, layout.apronOuterRz, angle);
    const yaw = ellipseYawFacingCenter(angle, layout.apronOuterRx, layout.apronOuterRz);
    dummy.position.set(x, 0, z);
    dummy.quaternion.copy(scoreboardQuaternion(yaw, SCOREBOARD_TILT));
    dummy.scale.setScalar(1);
    dummy.updateMatrix();
    bezels.setMatrixAt(i, dummy.matrix);
    faces.setMatrixAt(i, dummy.matrix);
  }
  bezels.instanceMatrix.needsUpdate = true;
  faces.instanceMatrix.needsUpdate = true;

  group.add(bezels, faces);

  return { group, timeUniforms: [uTime] };
}
