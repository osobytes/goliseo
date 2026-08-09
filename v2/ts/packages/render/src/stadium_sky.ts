// The sky: a huge inverted sphere with a fully emissive procedural shader --
// a twilight nebula gradient, two octaves of cheap value noise for drifting
// wisps, and a hashed starfield with a handful of twinkling stars. No
// lighting is involved (BackSide, unlit), and no texture is loaded (three
// values sampled here are entirely computed in-shader), so this needs
// neither a GL context nor a DOM to CONSTRUCT -- only to render, which this
// package's headless test tier never does (see draw2d.ts's file header).
//
// Draw calls: 1 (a single sphere mesh).

import * as THREE from "three";
import type { NumberUniform } from "./stadium_types.ts";

export interface SkyBuild {
  readonly group: THREE.Group;
  readonly timeUniforms: readonly NumberUniform[];
}

const VERTEX_SHADER = /* glsl */ `
  varying vec3 vDir;
  void main() {
    vDir = normalize(position);
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

// Cheap hash-based value noise (no texture): two integer-ish hashes plus a
// smoothstep interpolation give "good enough" wisps and stars for a distant
// backdrop nobody will inspect up close. `hash2` returns [0,1); `valueNoise`
// interpolates a 2D lattice of `hash2` samples.
const FRAGMENT_SHADER = /* glsl */ `
  uniform float uTime;
  varying vec3 vDir;

  float hash2(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
  }

  float valueNoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    float a = hash2(i);
    float b = hash2(i + vec2(1.0, 0.0));
    float c = hash2(i + vec2(0.0, 1.0));
    float d = hash2(i + vec2(1.0, 1.0));
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
  }

  void main() {
    vec3 dir = normalize(vDir);
    // Vertical gradient: deep indigo horizon -> violet -> near-black zenith.
    float h = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 horizon = vec3(0.07, 0.05, 0.16);
    vec3 mid = vec3(0.10, 0.05, 0.20);
    vec3 zenith = vec3(0.01, 0.008, 0.03);
    vec3 sky = mix(horizon, mid, smoothstep(0.0, 0.35, h));
    sky = mix(sky, zenith, smoothstep(0.35, 1.0, h));

    // Nebula wisps: two octaves of drifting value noise, sampled on a
    // spherical-ish projection of the view direction, tinted cyan/magenta.
    vec2 uv = dir.xz / (abs(dir.y) + 0.35) * 0.6;
    float drift = uTime * 0.004;
    float n1 = valueNoise(uv * 1.4 + vec2(drift, drift * 0.6));
    float n2 = valueNoise(uv * 3.1 - vec2(drift * 0.7, drift));
    float wisp = clamp(n1 * 0.65 + n2 * 0.35 - 0.35, 0.0, 1.0);
    vec3 cyanWisp = vec3(0.1, 0.35, 0.4) * wisp * 0.5;
    vec3 magentaWisp = vec3(0.35, 0.08, 0.3) * pow(n2, 2.0) * 0.4;
    sky += (cyanWisp + magentaWisp) * smoothstep(0.0, 0.6, h);

    // Starfield: a sparse hash grid, brightness varies per star, a few
    // percent twinkle with time. Only above the horizon.
    vec2 starUv = dir.xz / (abs(dir.y) + 0.02) * 40.0 + dir.xy * 12.0;
    vec2 cell = floor(starUv);
    float starChance = hash2(cell);
    float star = 0.0;
    if (starChance > 0.986) {
      vec2 cellUv = fract(starUv) - 0.5;
      float d = length(cellUv);
      float brightness = hash2(cell + 7.0);
      float twinkle = starChance > 0.997 ? (0.6 + 0.4 * sin(uTime * 3.0 + brightness * 40.0)) : 1.0;
      star = smoothstep(0.22, 0.0, d) * brightness * twinkle;
    }
    sky += vec3(star) * step(0.0, dir.y + 0.05);

    gl_FragColor = vec4(sky, 1.0);
  }
`;

/** Builds the "sky" sub-group: one huge inverted, unlit, emissive sphere. */
export function buildSky(radius: number): SkyBuild {
  const group = new THREE.Group();
  group.name = "sky";

  const uTime: NumberUniform = { value: 0 };
  const geometry = new THREE.SphereGeometry(radius, 32, 20);
  const material = new THREE.ShaderMaterial({
    uniforms: { uTime },
    vertexShader: VERTEX_SHADER,
    fragmentShader: FRAGMENT_SHADER,
    side: THREE.BackSide,
    depthWrite: false,
    fog: false,
  });
  const mesh = new THREE.Mesh(geometry, material);
  mesh.name = "sky_dome";
  group.add(mesh);

  return { group, timeUniforms: [uTime] };
}
