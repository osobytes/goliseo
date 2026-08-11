// The pitch surface: the hero floor. One quad, exactly covering
// `x in [0, field.w], z in [0, field.h]` at `y = 0.5` (half a unit above the
// apron disk -- see stadium_apron.ts's file header -- purely to win the
// z-fighting between two coincident flat meshes, matching this task's own
// spec wording), carrying every marking analytically in its fragment shader
// rather than as separate geometry: the same content pitch.ts's 2D
// `drawMarkings`/`drawHexFloor`/`drawFloorGlow` drew as ~350 individual
// `Line`/`Polygon` draw calls (see pitch.ts's file header on why that split
// existed), collapsed into one shader on one mesh.
//
// UNIFORM CONTRACT (stadium.spec.ts asserts this directly): every marking
// value that depends on the match's own field -- `w`, `h`, the penalty box's
// depth/height -- is read from `field` and written verbatim into a uniform,
// never hand-copied as a literal; the center circle's radius (70) and the
// center spot's radius (3) are CONSTANTS matching pitch.ts's own
// `drawMarkings` (`dl.polygon(..., projectedCircle(project, field.w / 2,
// field.h / 2, 70, 36), ...)`), exposed as uniforms too so a test can pin
// them without reaching into the shader source string.
//
// Draw calls: 1.

import * as THREE from "three";
import type { ArenaColors } from "./arena.ts";
import type { RenderFrameField } from "./pitch.ts";
import type { NumberUniform } from "./stadium_types.ts";

export interface PitchSurfaceBuild {
  readonly group: THREE.Group;
  readonly timeUniforms: readonly NumberUniform[];
}

/** Center circle radius, world units -- matches pitch.ts's `drawMarkings` literal. */
export const CENTER_CIRCLE_RADIUS = 70;
/** Center spot radius, world units -- matches pitch.ts's `drawMarkings` literal. */
export const CENTER_SPOT_RADIUS = 3;
/** Hex tile radius (centre to corner), world units -- matches pitch.ts's `HEX_RADIUS`. */
export const HEX_TILE_RADIUS = 26;

const VERTEX_SHADER = /* glsl */ `
  varying vec2 vWorldXZ;
  void main() {
    vec4 world = modelMatrix * vec4(position, 1.0);
    vWorldXZ = world.xz;
    gl_Position = projectionMatrix * viewMatrix * world;
  }
`;

const FRAGMENT_SHADER = /* glsl */ `
  uniform float uTime;
  uniform vec2 uFieldSize;
  uniform vec2 uPenaltyBox;
  uniform float uCircleRadius;
  uniform float uCenterSpotRadius;
  uniform float uHexRadius;
  uniform vec3 uMarkingColor;
  uniform vec3 uFloorColor;
  varying vec2 vWorldXZ;

  float aaLine(float d, float halfWidth) {
    float aa = fwidth(d) + 0.0005;
    return 1.0 - smoothstep(halfWidth - aa, halfWidth + aa, abs(d));
  }

  // Signed distance to an axis-aligned box outline (0 exactly on the edge,
  // negative inside, positive outside) -- the standard box SDF.
  float boxOutlineDist(vec2 p, vec2 boxCenter, vec2 halfExtent) {
    vec2 d = abs(p - boxCenter) - halfExtent;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
  }

  // Nearest pointy-top hex centre (redblobgames' standard axial pixel<->hex
  // formulas) and the hexagon's own edge distance, derived from first
  // principles: a pointy-top regular hexagon is the intersection of three
  // symmetric slabs with normals at 0/60/120 degrees, apothem = R*sqrt(3)/2 --
  // so max() of the three slab distances IS the hexagon's edge distance.
  vec3 hexEdgeInfo(vec2 p, float R) {
    float q = (0.5773502692 * p.x - 0.3333333333 * p.y) / R;
    float r = (0.6666666667 * p.y) / R;
    float cx = q;
    float cz = r;
    float cy = -cx - cz;
    float rx = floor(cx + 0.5);
    float ry = floor(cy + 0.5);
    float rz = floor(cz + 0.5);
    float dx = abs(rx - cx);
    float dy = abs(ry - cy);
    float dz = abs(rz - cz);
    if (dx > dy && dx > dz) {
      rx = -ry - rz;
    } else if (dy > dz) {
      ry = -rx - rz;
    } else {
      rz = -rx - ry;
    }
    vec2 center = vec2(R * 1.7320508 * (rx + rz * 0.5), R * 1.5 * rz);
    vec2 local = p - center;
    float apothem = R * 0.8660254;
    float slab0 = abs(local.x);
    float slab60 = abs(dot(local, vec2(0.5, 0.8660254)));
    float slab120 = abs(dot(local, vec2(-0.5, 0.8660254)));
    float edgeDist = max(max(slab0, slab60), slab120) - apothem;
    float hexId = rx * 12.9898 + rz * 78.233;
    return vec3(edgeDist, hexId, 0.0);
  }

  void main() {
    vec2 p = vWorldXZ;
    vec3 color = uFloorColor;

    // Soft radial luminance toward the pitch centre.
    vec2 center = uFieldSize * 0.5;
    float distToCenter = length(p - center);
    color += vec3(0.05, 0.16, 0.2) * smoothstep(uFieldSize.x * 0.65, 0.0, distToCenter) * 0.35;

    // Hex floor: faint glowing lines with a slow animated shimmer riding the
    // brightness of each individual hex cell.
    vec3 hexInfo = hexEdgeInfo(p, uHexRadius);
    float hexEdge = -hexInfo.x;
    float hexAA = fwidth(hexEdge) + 0.0005;
    float hexLine = 1.0 - smoothstep(0.0, hexAA * 2.2, hexEdge);
    float shimmer = 0.6 + 0.4 * sin(uTime * 0.35 + hexInfo.y);
    color += uMarkingColor * hexLine * 0.11 * shimmer;

    // Crisp analytic markings, matching pitch.ts's drawMarkings content.
    float markLine = 0.0;
    // Pitch outline.
    float edgeDist = min(min(p.x, uFieldSize.x - p.x), min(p.y, uFieldSize.y - p.y));
    markLine = max(markLine, aaLine(edgeDist, 1.1));
    // Halfway line.
    markLine = max(markLine, aaLine(p.x - uFieldSize.x * 0.5, 1.0));
    // Centre circle + spot.
    float distFromCentre = length(p - center);
    markLine = max(markLine, aaLine(distFromCentre - uCircleRadius, 1.0));
    float spotAA = fwidth(distFromCentre) + 0.0005;
    markLine = max(markLine, 1.0 - smoothstep(uCenterSpotRadius - spotAA, uCenterSpotRadius + spotAA, distFromCentre));
    // Penalty boxes, both ends.
    vec2 boxHalf = vec2(uPenaltyBox.x * 0.5, uPenaltyBox.y * 0.5);
    float homeBox = boxOutlineDist(p, vec2(uPenaltyBox.x * 0.5, center.y), boxHalf);
    float awayBox = boxOutlineDist(p, vec2(uFieldSize.x - uPenaltyBox.x * 0.5, center.y), boxHalf);
    markLine = max(markLine, aaLine(homeBox, 1.0));
    markLine = max(markLine, aaLine(awayBox, 1.0));

    color += uMarkingColor * markLine * 0.9;

    gl_FragColor = vec4(color, 1.0);
  }
`;

function buildPitchQuadGeometry(w: number, h: number): THREE.BufferGeometry {
  const y = 0.5;
  const positions = new Float32Array([0, y, 0, w, y, 0, w, y, h, 0, y, h]);
  const uvs = new Float32Array([0, 0, 1, 0, 1, 1, 0, 1]);
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.BufferAttribute(uvs, 2));
  geometry.setIndex([0, 2, 1, 0, 3, 2]);
  geometry.computeVertexNormals();
  return geometry;
}

/** Builds the "pitch_surface" sub-group: the one hero-floor quad. See file header for the uniform contract. */
export function buildPitchSurface(field: RenderFrameField, arena: ArenaColors): PitchSurfaceBuild {
  const group = new THREE.Group();
  group.name = "pitch_surface";

  const uTime: NumberUniform = { value: 0 };
  const geometry = buildPitchQuadGeometry(field.w, field.h);
  const material = new THREE.ShaderMaterial({
    uniforms: {
      uTime,
      uFieldSize: { value: new THREE.Vector2(field.w, field.h) },
      uPenaltyBox: { value: new THREE.Vector2(field.penalty_box_depth, field.penalty_box_h) },
      uCircleRadius: { value: CENTER_CIRCLE_RADIUS },
      uCenterSpotRadius: { value: CENTER_SPOT_RADIUS },
      uHexRadius: { value: HEX_TILE_RADIUS },
      uMarkingColor: { value: new THREE.Color(0.85, 0.95, 1.0) },
      uFloorColor: { value: new THREE.Color(arena.floor_color[0], arena.floor_color[1], arena.floor_color[2]) },
    },
    vertexShader: VERTEX_SHADER,
    fragmentShader: FRAGMENT_SHADER,
    side: THREE.DoubleSide,
  });
  const mesh = new THREE.Mesh(geometry, material);
  mesh.name = "pitch_surface_quad";
  group.add(mesh);

  return { group, timeUniforms: [uTime] };
}
