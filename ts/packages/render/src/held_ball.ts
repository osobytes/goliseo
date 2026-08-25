// The match ball while a keeper is holding it, and the two numbers that keep
// it the same object as the ball on the grass.
//
// WHY THIS EXISTS. `gc_render::frame::build` publishes `ball.visible =
// !keeper_holds`, and `pitch.ts` honours it: the moment a keeper gathers the
// ball, the ground ball stops being drawn "on the understanding that the
// keeper avatar draws it in their hands". Nothing drew it. A keeper caught
// the ball, adopted the gather pose, and the ball simply vanished until the
// throw put it back on the pitch.
//
// That is a PORT REGRESSION, not a missing feature: the Lua tree fixed exactly
// this (commit `5a68c68`, "draw the keeper's held ball") by riding the ball on
// the `socket_ball` bone `rig3d/skeleton.ts` still declares for it, and the
// Rust/TypeScript port (`2c0d449`) carried the socket and the `visible` flag
// across but not the draw.
//
// SIZE AND COLOUR ARE SHARED WITH THE LOOSE BALL, not restated. `RADIUS` and
// `COLOR` below are the numbers `pitch.ts`'s `drawLooseBallCommands` draws the
// ground ball with, imported from here rather than written twice, because "the
// ball in the hands is the ball that was on the grass" is the whole claim and
// two literals cannot make it. The screen sizes agree exactly, not
// approximately: `pitch.ts` draws the loose ball at `RADIUS * scale` pixels,
// and this ball is `RADIUS` world units converted to metres, drawn under a
// wrapper scaled `ppm = r * HEIGHT_IN_RADII * 2 / height` pixels per metre
// with `r = PLAYER_RADIUS * scale` -- both `scale` and `height` cancel, so a
// held ball is `RADIUS * scale` pixels too, anywhere on the pitch.
//
// `MeshBasicMaterial`, deliberately: the loose ball is a flat filled circle in
// exactly this colour (`draw2d.ts`'s `fillMaterial`, also a
// `MeshBasicMaterial`), so an unlit sphere is what makes the two read as one
// object. `pitch.ts` also gives every character wrapper a deliberately tiny
// world-Z scale (`CHARACTER_DEPTH_SCALE`), which flattens this sphere to a
// disc facing the camera -- the same silhouette, still depth-tested against
// the hands that hold it.

import * as THREE from "three";
import type { RGB } from "./draw2d.ts";
import * as skeleton from "./rig3d/skeleton.ts";

/**
 * The bone the held ball rides. `rig3d/skeleton.ts` declares it on the chest
 * precisely so a torso-driven clip carries the ball; `rig3d/clips.ts`'s
 * `keeper_gather` wraps the hands around this point.
 */
export const SOCKET = "socket_ball";

/**
 * `THREE.Object3D.name` on the ball mesh itself.
 *
 * Deliberately NOT `SOCKET`. The bone of that name is a descendant of the same
 * `THREE.SkinnedMesh` (`bones[0]` is parented to it), so naming the mesh after
 * its socket would make `mesh.getObjectByName("socket_ball")` ambiguous -- and
 * it resolves to the BONE, since depth-first traversal reaches the skeleton
 * first. Two things, two names.
 */
export const NAME = "held_ball";

/** The ball's drawn radius, in pitch world units. Shared with `pitch.ts`'s loose ball. */
export const RADIUS = 5;

/** The ball's colour. Shared with `pitch.ts`'s loose ball. */
export const COLOR: RGB = [1, 0.95, 0.7];

// Segment counts: the silhouette is what is seen (see the header on the
// world-Z flattening), so this matches `draw2d.ts`'s `CIRCLE_SEGMENTS` around
// the equator and spends far less on the rings that flatten away.
const WIDTH_SEGMENTS = 32;
const HEIGHT_SEGMENTS = 12;

/** The held ball's radius in rig metres, for a rig whose world unit is `metresPerWorldUnit`. */
export function radiusMetres(metresPerWorldUnit: number): number {
  return RADIUS * metresPerWorldUnit;
}

/**
 * Impure: builds a held-ball mesh of `radius` metres and parents it to
 * `character`, hidden until `place` is told a keeper is holding.
 *
 * Parented to the character's own mesh rather than added beside it, so the
 * facing yaw, the elevation tilt and the screen placement `pitch.ts` composes
 * onto that mesh all reach the ball for free -- and so `draw2d.ts`'s per-frame
 * `disposeObject` cannot reach it (it never recurses past a `THREE.Group`'s
 * direct children, which is the same reason the pooled character survives).
 *
 * Geometry and material are shared across every character: one ball is the
 * same size and colour as another, and only ever one is visible at a time
 * (the simulation has exactly one ball).
 */
export function attach(character: THREE.Object3D, radius: number): THREE.Mesh {
  const ball = new THREE.Mesh(geometryFor(radius), material());
  ball.name = NAME;
  ball.visible = false;
  character.add(ball);
  return ball;
}

/**
 * Places `ball` at the posed rig's ball socket, or hides it when this player
 * is not holding.
 *
 * POSITION ONLY, and that is not a shortcut: a sphere has no visible
 * orientation, so the socket's rotation is the one part of its transform
 * nothing can observe. Reading just the translation keeps this on three.js's
 * ordinary `position`/`matrixAutoUpdate` path instead of hand-writing a matrix
 * and having to hand-manage the dirty flags that go with it.
 */
export function place(ball: THREE.Mesh, rig: skeleton.Rig, holding: boolean): void {
  ball.visible = holding;
  if (!holding) {
    return;
  }
  const [x, y, z] = skeleton.jointPosition(rig, SOCKET);
  ball.position.set(x, y, z);
}

let sharedGeometry: THREE.SphereGeometry | undefined;
let sharedMaterial: THREE.MeshBasicMaterial | undefined;

// One radius serves every character (there is one rig contract -- see
// `rig3d/proportions.ts`), so the cache is a single slot rather than a map;
// a second radius rebuilds rather than accumulating entries for a case that
// does not arise on the product path.
function geometryFor(radius: number): THREE.SphereGeometry {
  if (sharedGeometry === undefined || sharedGeometry.parameters.radius !== radius) {
    sharedGeometry = new THREE.SphereGeometry(radius, WIDTH_SEGMENTS, HEIGHT_SEGMENTS);
  }
  return sharedGeometry;
}

function material(): THREE.MeshBasicMaterial {
  sharedMaterial ??= new THREE.MeshBasicMaterial({
    color: new THREE.Color(COLOR[0], COLOR[1], COLOR[2]),
  });
  return sharedMaterial;
}
