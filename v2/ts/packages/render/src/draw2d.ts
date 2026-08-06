// New shared infrastructure, not a port of one Lua file.
//
// Every Lua module under game/render/ (pitch, arena, match_hud, player_renderer,
// combat, effects) draws by issuing `love.graphics.*` calls directly: an
// immediate-mode API with no equivalent in three.js, which is a retained-mode
// scene graph. Rather than hand-translate each call site into ad hoc
// `THREE.Object3D` mutation (which cannot be unit-tested without a renderer),
// every one of those modules is split here into:
//
//   1. a PURE function that produces a `DrawCommand[]` -- the exact sequence
//      of shapes, colors, positions and text the Lua original would have
//      issued, in the same order. This is the content, and it is what the
//      ported specs assert against (mirroring how the Lua test suite itself
//      stubs `love.graphics` and records calls -- see
//      spec/render/combat_presentation_spec.lua's `RecordedGeometryCall`).
//   2. a THIN, impure interpreter (`paint` below) that walks a `DrawCommand[]`
//      and builds/updates three.js objects in a provided `THREE.Group`. This
//      is mechanism -- three.js already has `Line`, `Mesh`, `Sprite` -- so it
//      stays intentionally dumb and is not unit-tested (no WebGL context in
//      this milestone; see v2/README.md #1 and this package's port report).
//
// Coordinates are screen/pixel space (the output of `camera.project`), so the
// natural three.js host is an orthographic scene sized to the viewport -- see
// `paint`'s doc comment.

import * as THREE from "three";
import type { CameraProjection } from "./camera.ts";

/** An RGB triple in [0, 1], LÖVE's color convention. */
export type RGB = readonly [number, number, number];

/**
 * A world-to-screen projection, matching every Lua drawing function's
 * `project(wx, wy) -> sx, sy, scale` parameter (`camera.project`, or a
 * closure over it that adds a camera-shake offset -- see `pitch.ts`).
 */
export type Project = (wx: number, wy: number) => CameraProjection;

export type DrawMode = "fill" | "line";
export type TextAlign = "left" | "center" | "right";
export type BlendMode = "alpha" | "add";

interface CommandBase {
  readonly color: RGB;
  readonly alpha?: number;
  readonly blend?: BlendMode;
}

export interface RectCommand extends CommandBase {
  readonly kind: "rect";
  readonly mode: DrawMode;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
  readonly rx?: number;
  readonly ry?: number;
  readonly lineWidth?: number;
}

export interface CircleCommand extends CommandBase {
  readonly kind: "circle";
  readonly mode: DrawMode;
  readonly x: number;
  readonly y: number;
  readonly r: number;
  readonly lineWidth?: number;
}

export interface EllipseCommand extends CommandBase {
  readonly kind: "ellipse";
  readonly mode: DrawMode;
  readonly x: number;
  readonly y: number;
  readonly rx: number;
  readonly ry: number;
  readonly lineWidth?: number;
}

/** A closed polygon, flat `[x0, y0, x1, y1, ...]` pairs (LÖVE's own convention). */
export interface PolygonCommand extends CommandBase {
  readonly kind: "polygon";
  readonly mode: DrawMode;
  readonly points: readonly number[];
  readonly lineWidth?: number;
}

/** An open polyline, flat `[x0, y0, x1, y1, ...]` pairs. */
export interface LineCommand extends CommandBase {
  readonly kind: "line";
  readonly points: readonly number[];
  readonly lineWidth?: number;
}

export interface ArcCommand extends CommandBase {
  readonly kind: "arc";
  readonly mode: DrawMode;
  readonly x: number;
  readonly y: number;
  readonly r: number;
  /** radians */
  readonly angle1: number;
  /** radians */
  readonly angle2: number;
  readonly lineWidth?: number;
}

export interface TextCommand extends CommandBase {
  readonly kind: "text";
  readonly text: string;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly align: TextAlign;
  /** Pixel font size. Computed by the caller (theme font kind * UI scale). */
  readonly fontPx?: number;
}

export type DrawCommand =
  | RectCommand
  | CircleCommand
  | EllipseCommand
  | PolygonCommand
  | LineCommand
  | ArcCommand
  | TextCommand;

/**
 * A rotate-then-offset transform around a pivot, matching the composed
 * effect of the Lua originals' `push(); translate(A); rotate(B);
 * translate(-P); ...; pop()` pattern (dive lunges, aerial rotations, combat
 * knockback/stagger, stumble): every point drawn while the block is active is
 * `Rotate(B) * (p - P) + A`, where `A = P + offset`. Kept as a pure
 * coordinate transform (not a graphics-state stack) so the callers stay
 * ordinary functions returning `DrawCommand[]`.
 */
export interface PivotTransform {
  readonly pivotX: number;
  readonly pivotY: number;
  /** radians */
  readonly angle: number;
  readonly offsetX: number;
  readonly offsetY: number;
}

export function rotateAround(t: PivotTransform, x: number, y: number): readonly [number, number] {
  const dx = x - t.pivotX;
  const dy = y - t.pivotY;
  const c = Math.cos(t.angle);
  const s = Math.sin(t.angle);
  const rx = dx * c - dy * s;
  const ry = dx * s + dy * c;
  return [t.pivotX + t.offsetX + rx, t.pivotY + t.offsetY + ry];
}

function transformPoints(t: PivotTransform | undefined, points: readonly number[]): readonly number[] {
  if (t === undefined) {
    return points;
  }
  const out: number[] = [];
  for (let i = 0; i + 1 < points.length; i += 2) {
    const x = points[i];
    const y = points[i + 1];
    if (x === undefined || y === undefined) {
      continue;
    }
    const [tx, ty] = rotateAround(t, x, y);
    out.push(tx, ty);
  }
  return out;
}

/**
 * Accumulates `DrawCommand`s in the order they are issued, exactly mirroring
 * a sequence of `love.graphics.*` calls. Every method optionally applies a
 * `PivotTransform` (see above), which is how the ported `figure()` (in
 * `player_renderer.ts`) represents the Lua original's rotated push/pop
 * blocks without a graphics-state stack.
 */
export class DrawList {
  readonly commands: DrawCommand[] = [];

  rect(
    mode: DrawMode,
    x: number,
    y: number,
    w: number,
    h: number,
    color: RGB,
    opts?: { alpha?: number; rx?: number; ry?: number; lineWidth?: number; blend?: BlendMode },
    t?: PivotTransform,
  ): void {
    if (t === undefined) {
      this.commands.push({
        kind: "rect",
        mode,
        x,
        y,
        w,
        h,
        color,
        ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
        ...(opts?.rx !== undefined ? { rx: opts.rx } : {}),
        ...(opts?.ry !== undefined ? { ry: opts.ry } : {}),
        ...(opts?.lineWidth !== undefined ? { lineWidth: opts.lineWidth } : {}),
        ...(opts?.blend !== undefined ? { blend: opts.blend } : {}),
      });
      return;
    }
    // A rotated rect can't stay an axis-aligned rect command, so it is
    // expressed as its four corners instead (rounded corners are lost under
    // rotation -- a minor cosmetic simplification, and only reachable from
    // the rare knockback/stagger/dive/stumble figure poses).
    const corners = [x, y, x + w, y, x + w, y + h, x, y + h];
    this.polygon(mode, corners, color, opts, t);
  }

  circle(
    mode: DrawMode,
    x: number,
    y: number,
    r: number,
    color: RGB,
    opts?: { alpha?: number; lineWidth?: number; blend?: BlendMode },
    t?: PivotTransform,
  ): void {
    const [cx, cy] = t === undefined ? [x, y] : rotateAround(t, x, y);
    this.commands.push({
      kind: "circle",
      mode,
      x: cx,
      y: cy,
      r,
      color,
      ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
      ...(opts?.lineWidth !== undefined ? { lineWidth: opts.lineWidth } : {}),
      ...(opts?.blend !== undefined ? { blend: opts.blend } : {}),
    });
  }

  ellipse(
    mode: DrawMode,
    x: number,
    y: number,
    rx: number,
    ry: number,
    color: RGB,
    opts?: { alpha?: number; lineWidth?: number; blend?: BlendMode },
  ): void {
    this.commands.push({
      kind: "ellipse",
      mode,
      x,
      y,
      rx,
      ry,
      color,
      ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
      ...(opts?.lineWidth !== undefined ? { lineWidth: opts.lineWidth } : {}),
      ...(opts?.blend !== undefined ? { blend: opts.blend } : {}),
    });
  }

  polygon(
    mode: DrawMode,
    points: readonly number[],
    color: RGB,
    opts?: { alpha?: number; lineWidth?: number; blend?: BlendMode },
    t?: PivotTransform,
  ): void {
    this.commands.push({
      kind: "polygon",
      mode,
      points: transformPoints(t, points),
      color,
      ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
      ...(opts?.lineWidth !== undefined ? { lineWidth: opts.lineWidth } : {}),
      ...(opts?.blend !== undefined ? { blend: opts.blend } : {}),
    });
  }

  line(
    points: readonly number[],
    color: RGB,
    opts?: { alpha?: number; lineWidth?: number; blend?: BlendMode },
    t?: PivotTransform,
  ): void {
    this.commands.push({
      kind: "line",
      points: transformPoints(t, points),
      color,
      ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
      ...(opts?.lineWidth !== undefined ? { lineWidth: opts.lineWidth } : {}),
      ...(opts?.blend !== undefined ? { blend: opts.blend } : {}),
    });
  }

  arc(
    mode: DrawMode,
    x: number,
    y: number,
    r: number,
    angle1: number,
    angle2: number,
    color: RGB,
    opts?: { alpha?: number; lineWidth?: number; blend?: BlendMode },
    t?: PivotTransform,
  ): void {
    const [cx, cy] = t === undefined ? [x, y] : rotateAround(t, x, y);
    const a1 = t === undefined ? angle1 : angle1 + t.angle;
    const a2 = t === undefined ? angle2 : angle2 + t.angle;
    this.commands.push({
      kind: "arc",
      mode,
      x: cx,
      y: cy,
      r,
      angle1: a1,
      angle2: a2,
      color,
      ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
      ...(opts?.lineWidth !== undefined ? { lineWidth: opts.lineWidth } : {}),
      ...(opts?.blend !== undefined ? { blend: opts.blend } : {}),
    });
  }

  text(
    text: string,
    x: number,
    y: number,
    w: number,
    align: TextAlign,
    color: RGB,
    opts?: { alpha?: number; fontPx?: number },
  ): void {
    this.commands.push({
      kind: "text",
      text,
      x,
      y,
      w,
      align,
      color,
      ...(opts?.alpha !== undefined ? { alpha: opts.alpha } : {}),
      ...(opts?.fontPx !== undefined ? { fontPx: opts.fontPx } : {}),
    });
  }

  /** Appends another list's commands in order (concatenation, not merging). */
  extend(commands: readonly DrawCommand[]): void {
    for (const c of commands) {
      this.commands.push(c);
    }
  }
}

// ---------------------------------------------------------------------------
// Impure: DrawCommand[] -> three.js objects. GPU-adjacent; no WebGL context
// exists in this milestone, so this half is untested (v2/README.md #1). Kept
// deliberately dumb: it interprets the command vocabulary generically rather
// than special-casing content, so the tested half above is where the game's
// actual look lives.
// ---------------------------------------------------------------------------

const CIRCLE_SEGMENTS = 32;

function colorOf(c: DrawCommand): THREE.Color {
  return new THREE.Color(c.color[0], c.color[1], c.color[2]);
}

function lineMaterial(c: DrawCommand): THREE.LineBasicMaterial {
  return new THREE.LineBasicMaterial({
    color: colorOf(c),
    transparent: c.alpha !== undefined,
    opacity: c.alpha ?? 1,
    blending: c.blend === "add" ? THREE.AdditiveBlending : THREE.NormalBlending,
  });
}

function fillMaterial(c: DrawCommand): THREE.MeshBasicMaterial {
  return new THREE.MeshBasicMaterial({
    color: colorOf(c),
    transparent: c.alpha !== undefined,
    opacity: c.alpha ?? 1,
    blending: c.blend === "add" ? THREE.AdditiveBlending : THREE.NormalBlending,
    side: THREE.DoubleSide,
  });
}

function ringPoints(x: number, y: number, r: number, segments = CIRCLE_SEGMENTS): THREE.Vector3[] {
  const points: THREE.Vector3[] = [];
  for (let i = 0; i <= segments; i += 1) {
    const a = (i / segments) * Math.PI * 2;
    points.push(new THREE.Vector3(x + r * Math.cos(a), y + r * Math.sin(a), 0));
  }
  return points;
}

function polylinePoints(points: readonly number[]): THREE.Vector3[] {
  const out: THREE.Vector3[] = [];
  for (let i = 0; i + 1 < points.length; i += 2) {
    const x = points[i];
    const y = points[i + 1];
    if (x === undefined || y === undefined) {
      continue;
    }
    out.push(new THREE.Vector3(x, y, 0));
  }
  return out;
}

/** A canvas-texture sprite for one text command. Cheap, avoids TextGeometry. */
function buildTextSprite(c: TextCommand): THREE.Sprite {
  const canvas = document.createElement("canvas");
  const scale = 4; // supersample for crisper text
  const fontPx = c.fontPx ?? 13;
  canvas.width = Math.max(1, Math.round(c.w * scale));
  canvas.height = Math.round(fontPx * 1.6 * scale);
  const ctx = canvas.getContext("2d");
  if (ctx !== null) {
    ctx.scale(scale, scale);
    ctx.font = `${fontPx}px sans-serif`;
    ctx.fillStyle = `rgba(${Math.round(c.color[0] * 255)}, ${Math.round(c.color[1] * 255)}, ${Math.round(
      c.color[2] * 255,
    )}, ${c.alpha ?? 1})`;
    ctx.textBaseline = "top";
    ctx.textAlign = c.align;
    const tx = c.align === "center" ? c.w / 2 : c.align === "right" ? c.w : 0;
    ctx.fillText(c.text, tx, 0);
  }
  const texture = new THREE.CanvasTexture(canvas);
  const material = new THREE.SpriteMaterial({ map: texture, transparent: true, depthTest: false });
  const sprite = new THREE.Sprite(material);
  sprite.scale.set(c.w, canvas.height / scale, 1);
  sprite.position.set(c.x + c.w / 2, c.y + canvas.height / scale / 2, 0);
  sprite.center.set(0.5, 0.5);
  return sprite;
}

function buildOne(c: DrawCommand, opts?: PaintOptions): THREE.Object3D {
  switch (c.kind) {
    case "rect": {
      if (c.mode === "fill") {
        const geometry = new THREE.PlaneGeometry(c.w, c.h);
        const mesh = new THREE.Mesh(geometry, fillMaterial(c));
        mesh.position.set(c.x + c.w / 2, c.y + c.h / 2, 0);
        return mesh;
      }
      const points = [
        new THREE.Vector3(c.x, c.y, 0),
        new THREE.Vector3(c.x + c.w, c.y, 0),
        new THREE.Vector3(c.x + c.w, c.y + c.h, 0),
        new THREE.Vector3(c.x, c.y + c.h, 0),
        new THREE.Vector3(c.x, c.y, 0),
      ];
      return new THREE.Line(new THREE.BufferGeometry().setFromPoints(points), lineMaterial(c));
    }
    case "circle": {
      if (c.mode === "fill") {
        const geometry = new THREE.CircleGeometry(c.r, CIRCLE_SEGMENTS);
        const mesh = new THREE.Mesh(geometry, fillMaterial(c));
        mesh.position.set(c.x, c.y, 0);
        return mesh;
      }
      return new THREE.LineLoop(
        new THREE.BufferGeometry().setFromPoints(ringPoints(0, 0, c.r)),
        lineMaterial(c),
      ).translateX(c.x).translateY(c.y);
    }
    case "ellipse": {
      const curve = new THREE.EllipseCurve(0, 0, c.rx, c.ry, 0, 2 * Math.PI, false, 0);
      const points = curve.getPoints(CIRCLE_SEGMENTS).map((p) => new THREE.Vector3(p.x, p.y, 0));
      if (c.mode === "fill") {
        const shape = new THREE.Shape(curve.getPoints(CIRCLE_SEGMENTS));
        const mesh = new THREE.Mesh(new THREE.ShapeGeometry(shape), fillMaterial(c));
        mesh.position.set(c.x, c.y, 0);
        return mesh;
      }
      const line = new THREE.LineLoop(new THREE.BufferGeometry().setFromPoints(points), lineMaterial(c));
      line.position.set(c.x, c.y, 0);
      return line;
    }
    case "polygon": {
      const points = polylinePoints(c.points);
      if (c.mode === "fill") {
        const shapePoints = points.map((p) => new THREE.Vector2(p.x, p.y));
        const mesh = new THREE.Mesh(new THREE.ShapeGeometry(new THREE.Shape(shapePoints)), fillMaterial(c));
        return mesh;
      }
      const closed = [...points, points[0]].filter((p): p is THREE.Vector3 => p !== undefined);
      return new THREE.Line(new THREE.BufferGeometry().setFromPoints(closed), lineMaterial(c));
    }
    case "line": {
      const points = polylinePoints(c.points);
      return new THREE.Line(new THREE.BufferGeometry().setFromPoints(points), lineMaterial(c));
    }
    case "arc": {
      const curve = new THREE.EllipseCurve(0, 0, c.r, c.r, c.angle1, c.angle2, false, 0);
      const points = curve.getPoints(CIRCLE_SEGMENTS).map((p) => new THREE.Vector3(p.x, p.y, 0));
      if (c.mode === "fill") {
        const shapePoints = [new THREE.Vector2(0, 0), ...points.map((p) => new THREE.Vector2(p.x, p.y))];
        const mesh = new THREE.Mesh(new THREE.ShapeGeometry(new THREE.Shape(shapePoints)), fillMaterial(c));
        mesh.position.set(c.x, c.y, 0);
        return mesh;
      }
      const line = new THREE.Line(new THREE.BufferGeometry().setFromPoints(points), lineMaterial(c));
      line.position.set(c.x, c.y, 0);
      return line;
    }
    case "text":
      return opts?.buildText !== undefined ? opts.buildText(c) : buildTextSprite(c);
  }
}

/**
 * Substitutes part of `buildOne`'s output. The only case with a substitute
 * today is `text`: `buildTextSprite` calls `document.createElement("canvas")`,
 * which needs a real DOM (a browser, or a jsdom/happy-dom-scoped spec) and
 * does not exist under this workspace's default `vitest` "node" environment
 * (v2/ts/vitest.config.ts). Passing `buildText` lets a "node" spec exercise
 * everything else `paint`/`appendCommands` do -- clearing, rebuilding,
 * disposal, non-text object construction -- without a DOM polyfill. See
 * match_hud.spec.ts's population suite for why this was chosen over a
 * jsdom-scoped spec file.
 */
export interface PaintOptions {
  readonly buildText?: (c: TextCommand) => THREE.Object3D;
}

/**
 * Releases one paint-built object's own GPU-side resources: geometry,
 * material(s), and any texture a material holds via `.map` (this reaches
 * `buildTextSprite`'s `CanvasTexture`, which `paint`'s cleanup did not used
 * to dispose). Also releases `userData.ownedRenderTarget` when present -- a
 * `THREE.WebGLRenderTarget` some other module in this package attached to an
 * object it built and added to the group itself (bypassing `buildOne`), the
 * same way pitch.ts's rigged-player sprites do (see player_renderer_3d.ts's
 * `renderToSprite`). Exported so scene.ts's `SceneRoot` can apply the exact
 * same cleanup at teardown as `paint` applies every frame, instead of two
 * copies of the same `instanceof` check drifting apart.
 */
export function disposeObject(child: THREE.Object3D): void {
  if (child instanceof THREE.Mesh || child instanceof THREE.Line) {
    child.geometry.dispose();
    disposeMaterial(child.material);
  } else if (child instanceof THREE.Sprite) {
    disposeMaterial(child.material);
  }
  const owned: unknown = child.userData["ownedRenderTarget"];
  if (owned instanceof THREE.WebGLRenderTarget) {
    owned.dispose();
  }
}

function disposeMaterial(material: THREE.Material | THREE.Material[]): void {
  const materials = Array.isArray(material) ? material : [material];
  for (const m of materials) {
    if ("map" in m && m.map instanceof THREE.Texture) {
      m.map.dispose();
    }
    m.dispose();
  }
}

/**
 * Appends `commands` to `group` as new children, in order, WITHOUT clearing
 * whatever is already there. `paint` (below) is `appendCommands` preceded by
 * a clear; this half is exposed separately for callers that need to
 * interleave `DrawCommand`-derived objects with content built some other way
 * at specific points in the child order -- pitch.ts's mixed procedural/rigged
 * player pass is the one today (see its file header and `pitch.draw`): the
 * painter's-algorithm depth sort requires the ball, procedural billboards
 * and rigged-player sprites to land in `group.children` interleaved by
 * world-depth, not as one `paint()` replacing everything and a second
 * wiping it out.
 */
export function appendCommands(group: THREE.Group, commands: readonly DrawCommand[], opts?: PaintOptions): void {
  for (const c of commands) {
    group.add(buildOne(c, opts));
  }
}

/**
 * Rebuilds `group`'s children from `commands`, in order. `group` is expected
 * to sit under an orthographic camera whose view spans `[0, vp.w] x [0,
 * vp.h]` with y increasing downward (screen convention) -- the same space
 * `camera.project`'s output already lives in, so nothing here re-derives a
 * projection.
 *
 * Rebuilding every call is the simplest correct thing (matching the Lua
 * original's fully-immediate-mode redraw each frame) and is left that way
 * deliberately: this milestone has no renderer to profile against, so
 * optimizing object reuse here would be tuning against nothing.
 */
export function paint(group: THREE.Group, commands: readonly DrawCommand[], opts?: PaintOptions): void {
  for (const child of [...group.children]) {
    group.remove(child);
    disposeObject(child);
  }
  appendCommands(group, commands, opts);
}
