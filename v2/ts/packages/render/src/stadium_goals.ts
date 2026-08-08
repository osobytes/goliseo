// Mario-Strikers-style goals: thick metallic posts/crossbar with a bloom-
// catching emissive team-colour trim, a back frame, and a diamond-grid net
// with a static catenary sag baked into its geometry at construction time
// (no per-frame animation -- nothing here needs a `uTime` uniform, so unlike
// most of stadium.ts's other helper modules `buildGoal` returns a bare
// `THREE.Group`).
//
// COORDINATE CONTRACT (see stadium.ts's own header for the full contract):
// the mouth spans `rect` on the goal line at world x = `mouthLineX`; the net
// box extends behind it to `rect`'s OTHER x edge (whichever of `rect.x`/
// `rect.x + rect.w` is not `mouthLineX`) -- this module derives that "back"
// x itself, since the caller only passes `mouthLineX`, not a second explicit
// back-line parameter (contrast pitch.ts's `drawGoal`, ported from the 2D
// original, which took both `lineX` and `backX` directly).
//
// Draw calls per goal: 3 (frame -- front + back posts/crossbar merged into
// one mesh/material; emissive trim rings; net -- four panels merged into
// one mesh). Two goals -> 6 total.

import * as THREE from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import type { RGB } from "./draw2d.ts";
import type { Rect } from "./pitch.ts";

/** Back frame height as a fraction of the crossbar -- matches pitch.ts's `NET_BACK_FRAC`. */
const NET_BACK_FRAC = 0.55;
// Chunky Strikers goals (creative brief item 4): posts thick enough to read
// as a landmark at broadcast-camera distance, not a wireframe.
const POST_RADIUS = 3.5;
const BACK_POST_RADIUS = 2.2;
const TRIM_HEIGHT_FRAC = 0.82;
const NET_SEGMENTS = 8;
const NET_SAG = 5;

function resolveBackX(rect: Rect, mouthLineX: number): number {
  const nearEdge = rect.x;
  const farEdge = rect.x + rect.w;
  return Math.abs(nearEdge - mouthLineX) <= Math.abs(farEdge - mouthLineX) ? farEdge : nearEdge;
}

// Two posts + a crossbar, built once and reused (translated to the right x)
// for both the bright front frame and the thinner back frame.
function buildFrameSegment(z0: number, z1: number, x: number, radius: number, barHeight: number): THREE.BufferGeometry {
  const postHeight = barHeight;
  const post = new THREE.CylinderGeometry(radius, radius, postHeight, 10);
  const postA = post.clone().translate(x, postHeight / 2, z0);
  const postB = post.clone().translate(x, postHeight / 2, z1);
  post.dispose();

  const barLength = Math.abs(z1 - z0);
  const bar = new THREE.CylinderGeometry(radius, radius, barLength, 10)
    .rotateX(Math.PI / 2)
    .translate(x, postHeight, (z0 + z1) / 2);

  const merged = mergeGeometries([postA, postB, bar], false);
  postA.dispose();
  postB.dispose();
  bar.dispose();
  return merged;
}

// Emissive trim rings near the top of the two FRONT posts -- the detail the
// bloom pass (threshold 0.55) is meant to ignite (creative brief item 9).
function buildTrimGeometry(z0: number, z1: number, x: number, crossbarH: number): THREE.BufferGeometry {
  const ringY = crossbarH * TRIM_HEIGHT_FRAC;
  const ring = new THREE.TorusGeometry(POST_RADIUS + 1.1, 0.9, 8, 16).rotateX(Math.PI / 2);
  const ringA = ring.clone().translate(x, ringY, z0);
  const ringB = ring.clone().translate(x, ringY, z1);
  ring.dispose();
  const merged = mergeGeometries([ringA, ringB], false);
  ringA.dispose();
  ringB.dispose();
  return merged;
}

interface Vec3Like {
  readonly x: number;
  readonly y: number;
  readonly z: number;
}

// A bilinear grid between 4 corners (letting the top edge taper from the
// crossbar's height down to the back frame's, exactly like pitch.ts's own
// net polygons), with a `sin(pi u) * sin(pi v)` bulge along `sagDir` --
// zero at every edge, peaking in the middle of the panel, i.e. a static
// catenary-shaped sag baked directly into the mesh.
function buildNetPanel(p00: Vec3Like, p10: Vec3Like, p01: Vec3Like, p11: Vec3Like, sagDir: THREE.Vector3): THREE.BufferGeometry {
  const positions: number[] = [];
  const uvs: number[] = [];
  const rows = NET_SEGMENTS;
  const cols = NET_SEGMENTS;
  for (let j = 0; j <= rows; j += 1) {
    const v = j / rows;
    for (let i = 0; i <= cols; i += 1) {
      const u = i / cols;
      const top = { x: lerp(p00.x, p10.x, u), y: lerp(p00.y, p10.y, u), z: lerp(p00.z, p10.z, u) };
      const bottom = { x: lerp(p01.x, p11.x, u), y: lerp(p01.y, p11.y, u), z: lerp(p01.z, p11.z, u) };
      const base = { x: lerp(top.x, bottom.x, v), y: lerp(top.y, bottom.y, v), z: lerp(top.z, bottom.z, v) };
      const sag = NET_SAG * Math.sin(Math.PI * u) * Math.sin(Math.PI * v);
      positions.push(base.x + sagDir.x * sag, base.y + sagDir.y * sag, base.z + sagDir.z * sag);
      uvs.push(u, v);
    }
  }
  const indices: number[] = [];
  const stride = cols + 1;
  for (let j = 0; j < rows; j += 1) {
    for (let i = 0; i < cols; i += 1) {
      const a = j * stride + i;
      const b = a + 1;
      const c = a + stride;
      const d = c + 1;
      indices.push(a, c, b, b, c, d);
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uvs, 2));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

const NET_VERTEX = /* glsl */ `
  varying vec2 vUv;
  void main() {
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

// Analytic AA diamond-grid net pattern: two families of diagonal lines
// (`u + v` and `u - v`), transparent everywhere except close to a strand.
const NET_FRAGMENT = /* glsl */ `
  uniform vec3 uTeamColor;
  varying vec2 vUv;
  void main() {
    float scale = 16.0;
    float ga = (vUv.x + vUv.y) * scale;
    float gb = (vUv.x - vUv.y) * scale;
    float da = abs(ga - floor(ga + 0.5));
    float db = abs(gb - floor(gb + 0.5));
    float strand = min(da, db);
    float aa = fwidth(strand) + 0.001;
    float alpha = 1.0 - smoothstep(0.05, 0.05 + aa * 2.0, strand);
    // Slightly brighter strands (creative brief item 4) so the net reads
    // clearly against the dark apron/pitch behind it instead of near-
    // vanishing at broadcast-camera distance.
    vec3 netColor = mix(vec3(0.95, 0.97, 1.0), uTeamColor, 0.22);
    gl_FragColor = vec4(netColor, alpha * 0.92);
  }
`;

/**
 * Builds one goal: mouth spans `rect` at world x = `mouthLineX`, crossbar at
 * `crossbarH`, net box extending to `rect`'s other x edge. See file header
 * for the draw-call inventory and the coordinate-contract note on deriving
 * the back x. Caller sets `group.userData["stadiumGoal"]`.
 */
export function buildGoal(rect: Rect, crossbarH: number, mouthLineX: number, teamColor: RGB): THREE.Group {
  const group = new THREE.Group();
  group.name = "goal";

  const backX = resolveBackX(rect, mouthLineX);
  const backH = crossbarH * NET_BACK_FRAC;
  const zNear = rect.y;
  const zFar = rect.y + rect.h;

  const frontFrame = buildFrameSegment(zNear, zFar, mouthLineX, POST_RADIUS, crossbarH);
  const backFrame = buildFrameSegment(zNear, zFar, backX, BACK_POST_RADIUS, backH);
  const frameGeometry = mergeGeometries([frontFrame, backFrame], false);
  frontFrame.dispose();
  backFrame.dispose();
  // Near-white metal with its own emissive glow (creative brief item 4): the
  // frame itself should visibly bloom, the way the old 2D goals' outline
  // glow read, not just the trim rings riding on top of a flat-lit post.
  const frameMaterial = new THREE.MeshStandardMaterial({
    color: 0xdfe8f2,
    roughness: 0.35,
    metalness: 0.85,
    emissive: new THREE.Color(0.9, 0.95, 1.0),
    emissiveIntensity: 0.9,
  });
  const frame = new THREE.Mesh(frameGeometry, frameMaterial);
  frame.name = "goal_frame";
  group.add(frame);

  const trimGeometry = buildTrimGeometry(zNear, zFar, mouthLineX, crossbarH);
  const trimColor = new THREE.Color(teamColor[0], teamColor[1], teamColor[2]);
  const trimMaterial = new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 1.2, roughness: 0.4 });
  const trim = new THREE.Mesh(trimGeometry, trimMaterial);
  trim.name = "goal_trim";
  group.add(trim);

  const sideNear = buildNetPanel(
    { x: mouthLineX, y: crossbarH, z: zNear },
    { x: backX, y: backH, z: zNear },
    { x: mouthLineX, y: 0, z: zNear },
    { x: backX, y: 0, z: zNear },
    new THREE.Vector3(0, 0, mouthLineX < backX ? 1 : -1),
  );
  const sideFar = buildNetPanel(
    { x: mouthLineX, y: crossbarH, z: zFar },
    { x: backX, y: backH, z: zFar },
    { x: mouthLineX, y: 0, z: zFar },
    { x: backX, y: 0, z: zFar },
    new THREE.Vector3(0, 0, mouthLineX < backX ? -1 : 1),
  );
  const back = buildNetPanel(
    { x: backX, y: backH, z: zNear },
    { x: backX, y: backH, z: zFar },
    { x: backX, y: 0, z: zNear },
    { x: backX, y: 0, z: zFar },
    new THREE.Vector3(mouthLineX < backX ? 1 : -1, 0, 0),
  );
  const roof = buildNetPanel(
    { x: mouthLineX, y: crossbarH, z: zNear },
    { x: mouthLineX, y: crossbarH, z: zFar },
    { x: backX, y: backH, z: zNear },
    { x: backX, y: backH, z: zFar },
    new THREE.Vector3(0, -1, 0),
  );
  const netGeometry = mergeGeometries([sideNear, sideFar, back, roof], false);
  for (const g of [sideNear, sideFar, back, roof]) {
    g.dispose();
  }
  const netMaterial = new THREE.ShaderMaterial({
    uniforms: { uTeamColor: { value: trimColor } },
    vertexShader: NET_VERTEX,
    fragmentShader: NET_FRAGMENT,
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const net = new THREE.Mesh(netGeometry, netMaterial);
  net.name = "goal_net";
  group.add(net);

  return group;
}
