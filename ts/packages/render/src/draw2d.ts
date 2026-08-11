// Shared infrastructure used by every rendering module (pitch, arena,
// match_hud, player_renderer, combat, effects).
//
// None of those modules mutates a `THREE.Object3D` scene graph directly --
// three.js is retained-mode, and ad hoc mutation at each call site cannot be
// unit-tested without a renderer. Instead, every one of those modules is
// split here into:
//
//   1. a PURE function that produces a `DrawCommand[]` -- the exact sequence
//      of shapes, colors, positions and text to draw, in order. This is the
//      content, and it is what specs assert against (mirroring how each
//      module's own spec records calls -- see this file's `DrawList`).
//   2. a THIN, impure interpreter (`paint` below) that walks a `DrawCommand[]`
//      and builds/updates three.js objects in a provided `THREE.Group`. This
//      is mechanism -- three.js already has `Line`, `Mesh`, `Sprite` -- so it
//      stays intentionally dumb and is not unit-tested (no WebGL context in
//      this milestone).
//
// Coordinates are screen/pixel space (the output of `camera.project`), so the
// natural three.js host is an orthographic scene sized to the viewport -- see
// `paint`'s doc comment.

import * as THREE from "three";
import type { CameraProjection } from "./camera.ts";

/** An RGB triple in [0, 1]. */
export type RGB = readonly [number, number, number];

/**
 * A world-to-screen projection, matching `camera.project`'s
 * `(wx, wy) -> sx, sy, scale` signature (or a closure over it that adds a
 * camera-shake offset -- see `pitch.ts`).
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

/** A closed polygon, flat `[x0, y0, x1, y1, ...]` pairs. */
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
 * A rotate-then-offset transform around a pivot, used for dive lunges, aerial
 * rotations, combat knockback/stagger, and stumble: every point drawn while
 * the transform is active is `Rotate(B) * (p - P) + A`, where `A = P +
 * offset`. Kept as a pure coordinate transform (not a graphics-state stack)
 * so the callers stay ordinary functions returning `DrawCommand[]`.
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

function transformPoints(
  t: PivotTransform | undefined,
  points: readonly number[],
): readonly number[] {
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
 * Accumulates `DrawCommand`s in the order they are issued. Every method
 * optionally applies a `PivotTransform` (see above), which is how a call site
 * represents a rotated figure pose without a graphics-state stack.
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
// exists in this milestone, so this half is untested. Kept
// deliberately dumb: it interprets the command vocabulary generically rather
// than special-casing content, so the tested half above is where the game's
// actual look lives.
// ---------------------------------------------------------------------------

const CIRCLE_SEGMENTS = 32;

function colorOf(c: DrawCommand): THREE.Color {
  return new THREE.Color(c.color[0], c.color[1], c.color[2]);
}

// ---------------------------------------------------------------------------
// SHARED MATERIALS -- why this cache exists (#403).
//
// `paint` rebuilds the whole group every frame, and used to build a FRESH
// `THREE.LineBasicMaterial`/`MeshBasicMaterial` per command and dispose it
// again on the next frame's clear. Materials are cheap JS objects, so that
// reads harmless. It is not, because of what three.js hangs off a material's
// lifetime: `WebGLPrograms.acquireProgram` refcounts the compiled GL program
// a material resolves to (`usedTimes`), and `releaseProgram` -- reached from
// `material.dispose()` -- calls `program.destroy()` the moment that count
// hits zero. `paint` disposes EVERY old child BEFORE building any new one, so
// the count for each program variant reliably passed through zero once per
// frame. Every frame therefore re-ran `createShader`/`shaderSource`/
// `compileShader`/`attachShader`/`linkProgram` for the same handful of
// programs, and then `WebGLProgram`'s `onFirstUse` link-error check
// (`renderer.debug.checkShaderErrors`, ON by default) called
// `getProgramInfoLog` -- which BLOCKS until the driver has finished linking.
//
// Measured headful on a 74.98 Hz panel, 8 s sample, live match, by wrapping
// every `WebGL2RenderingContext` method with a counter and a timer:
//
//     shaderSource            x10.0/frame
//     getProgramInfoLog        x5.0/frame     6.29 ms/frame
//     getShaderInfoLog        x10.0/frame     0.70 ms/frame
//     getProgramParameter     x15.0/frame     0.32 ms/frame
//
// Five programs recompiled and synchronously re-validated per frame, for
// ~7.8 ms of a 13.33 ms budget -- while the whole frame measured ~12.9 ms.
// That is the "renders at exactly half the display refresh rate" of #403: the
// frame sat at ~97% of the vsync deadline, so it tipped onto every second
// refresh whenever anything else touched the GPU. It also explains why every
// earlier hypothesis missed -- this cost is identical at 960x540 and at
// 2193x1234, with bloom on or off, and it is not a draw call.
//
// The fix is to stop destroying the programs, which means to stop disposing
// the materials, which means to share them. Colour, opacity and blending are
// uniforms and GL state, NOT part of three.js's program cache key, so the
// number of distinct PROGRAMS behind this cache stays the same handful it
// always was; the cache only stops their refcounts reaching zero.
//
// SAFE TO SHARE because nothing mutates a material after `buildOne` returns:
// `applyDepthPlacement` is the only writer, and `depthTest` is part of the
// key below (see that function). A shared material outliving one
// `THREE.WebGLRenderer` is fine too -- three.js keeps per-renderer GL state in
// a renderer-owned `WebGLProperties` WeakMap, so the same material picked up
// by a second renderer simply gets fresh programs there.
//
// BOUNDED: LRU, evicting (and disposing) past `MAX_CACHED_MATERIALS`. Alpha
// is continuous in fades, so the key space is not finite in principle; in
// practice a match frame's distinct keys are in the low hundreds. An eviction
// costs at worst one program recompile on the next frame that needs it, which
// is the old behaviour for one material rather than for all of them.
// ---------------------------------------------------------------------------

const MAX_CACHED_MATERIALS = 512;

/** Marks a material as owned by `materialCache`, so `disposeMaterial` leaves it alone. */
const SHARED_MATERIAL_FLAG = "draw2dSharedMaterial";

const materialCache = new Map<string, THREE.Material>();

function sharedMaterial<T extends THREE.Material>(key: string, build: () => T): T {
  const hit = materialCache.get(key);
  if (hit !== undefined) {
    // Re-insert to move it to the end: `Map` iterates in insertion order, so
    // the front of the map is the least recently used entry.
    materialCache.delete(key);
    materialCache.set(key, hit);
    return hit as T;
  }
  const material = build();
  material.userData[SHARED_MATERIAL_FLAG] = true;
  materialCache.set(key, material);
  while (materialCache.size > MAX_CACHED_MATERIALS) {
    const oldest = materialCache.keys().next();
    if (oldest.done === true) {
      break;
    }
    release(oldest.value);
  }
  return material;
}

/**
 * Drops one entry, HANDING OWNERSHIP BACK rather than disposing.
 *
 * THE CACHE NEVER DISPOSES ANYTHING. This is the single rule that makes
 * sharing safe, and it is worth stating as a rule rather than leaving as an
 * implementation detail, because the obvious alternative — evict and
 * `dispose()` — is a genuine use-after-dispose. Eviction happens on INSERT,
 * and inserts happen while a frame is being built: `pitch.draw` issues one
 * `paint` followed by several `appendCommands`, so a frame with more than
 * `MAX_CACHED_MATERIALS` distinct keys could evict a material that an object
 * built EARLIER IN THE SAME FRAME is still holding, and then dispose it out
 * from under that object.
 *
 * That is the same family of bug as three.js's own `FullScreenQuad`, which
 * shares geometry at module level and frees it on any single instance's
 * `dispose()` — module-level state destroyed by one participant while others
 * still reference it. Here the ownership arrow points the other way and never
 * reverses: the cache is the only thing that may hand a material out, and it
 * is never the thing that frees one. Clearing the flag returns the material to
 * the ordinary per-object path, so the next `paint()` that clears an object
 * holding it disposes it exactly as it would have before this cache existed.
 *
 * The cost of never disposing is bounded and, here, actually desirable: a
 * material dropped while nothing references it is garbage collected without
 * `dispose()`, which leaves three.js's refcount on its compiled program
 * permanently incremented. That keeps the program alive — which is the entire
 * point of this file's SHARED MATERIALS section — and programs are shared by
 * three.js's own cache key, so the number of them stays the same handful
 * (measured: 15, flat across a 120 s soak) no matter how many materials pass
 * through.
 */
function release(key: string): void {
  const material = materialCache.get(key);
  materialCache.delete(key);
  if (material !== undefined) {
    material.userData[SHARED_MATERIAL_FLAG] = false;
  }
}

/** How many materials `materialCache` currently holds. For tests -- same
 * convention as pitch.ts's `staticSceneBuildCount`. */
export function materialCacheSize(): number {
  return materialCache.size;
}

/**
 * Empties the cache, handing every entry back to the ordinary per-object
 * disposal path (see `release` -- this does NOT dispose them, for the same
 * reason eviction does not). For tests, and for a caller that wants to stop
 * sharing across two `SceneRoot`s. Same convention as pitch.ts's
 * `resetStaticSceneCache`.
 */
export function resetMaterialCache(): void {
  for (const key of [...materialCache.keys()]) {
    release(key);
  }
}

// `alpha` is distinguished from `alpha: 1` on purpose: only the former leaves
// `transparent` false, and the two render differently.
function alphaKey(c: DrawCommand): string {
  return c.alpha === undefined ? "-" : String(c.alpha);
}

// The EXACT float triple, deliberately not `THREE.Color.getHexString()`. Hex
// would quantise to 8 bits per channel and hand two commands whose colours
// differ below 1/255 the same material -- imperceptible in the framebuffer,
// but this scene runs a bloom pass with a luminance THRESHOLD (bloom.ts), and
// a threshold is exactly where a sub-1/255 difference stops being invisible.
// Sharing must never change a pixel; a slightly larger key space is the
// cheaper side of that trade, and the LRU bounds it anyway.
function colorKey(c: DrawCommand): string {
  return `${c.color[0]},${c.color[1]},${c.color[2]}`;
}

// `applyDepthPlacement` writes `depthTest` onto the built object's material,
// so it has to be part of the key or two callers passing different
// `PaintOptions.depthTest` would fight over one shared instance. Resolved
// here to the value that function would have written (three.js's own default
// is `true`), and set at construction instead.
function depthTestOf(opts?: PaintOptions): boolean {
  return opts?.depthTest ?? true;
}

function lineMaterial(c: DrawCommand, opts?: PaintOptions): THREE.LineBasicMaterial {
  const color = colorOf(c);
  const depthTest = depthTestOf(opts);
  const additive = c.blend === "add";
  return sharedMaterial(
    `line|${colorKey(c)}|${alphaKey(c)}|${additive ? "add" : "alpha"}|${depthTest ? "z" : "-"}`,
    () =>
      new THREE.LineBasicMaterial({
        color,
        transparent: c.alpha !== undefined,
        opacity: c.alpha ?? 1,
        blending: additive ? THREE.AdditiveBlending : THREE.NormalBlending,
        depthTest,
      }),
  );
}

function fillMaterial(c: DrawCommand, opts?: PaintOptions): THREE.MeshBasicMaterial {
  const color = colorOf(c);
  const depthTest = depthTestOf(opts);
  const additive = c.blend === "add";
  return sharedMaterial(
    `fill|${colorKey(c)}|${alphaKey(c)}|${additive ? "add" : "alpha"}|${depthTest ? "z" : "-"}`,
    () =>
      new THREE.MeshBasicMaterial({
        color,
        transparent: c.alpha !== undefined,
        opacity: c.alpha ?? 1,
        blending: additive ? THREE.AdditiveBlending : THREE.NormalBlending,
        side: THREE.DoubleSide,
        depthTest,
      }),
  );
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

// The world-space polyline a stroke command draws, or `undefined` when the
// command is not a batchable stroke (any `mode: "fill"`, and `text`).
//
// Points are ABSOLUTE. `buildOne` is free to emit geometry around the origin
// and then translate the object, because it owns that object; a batch has no
// per-object transform to hang that on, so everything must be baked in here.
//
// Closed shapes are returned already closed (last point == first) rather than
// as `LineLoop`s, which is why merging them into a strip loses nothing:
// `ringPoints` and `EllipseCurve.getPoints` both already repeat the start
// point at the end for a full revolution.
function strokePolyline(c: DrawCommand): THREE.Vector3[] | undefined {
  switch (c.kind) {
    case "rect":
      if (c.mode === "fill") {
        return undefined;
      }
      return [
        new THREE.Vector3(c.x, c.y, 0),
        new THREE.Vector3(c.x + c.w, c.y, 0),
        new THREE.Vector3(c.x + c.w, c.y + c.h, 0),
        new THREE.Vector3(c.x, c.y + c.h, 0),
        new THREE.Vector3(c.x, c.y, 0),
      ];
    case "circle":
      return c.mode === "fill" ? undefined : ringPoints(c.x, c.y, c.r);
    case "ellipse": {
      if (c.mode === "fill") {
        return undefined;
      }
      const curve = new THREE.EllipseCurve(c.x, c.y, c.rx, c.ry, 0, 2 * Math.PI, false, 0);
      return curve.getPoints(CIRCLE_SEGMENTS).map((p) => new THREE.Vector3(p.x, p.y, 0));
    }
    case "polygon": {
      if (c.mode === "fill") {
        return undefined;
      }
      const points = polylinePoints(c.points);
      const first = points[0];
      return first === undefined ? points : [...points, first];
    }
    case "line":
      return polylinePoints(c.points);
    case "arc": {
      if (c.mode === "fill") {
        return undefined;
      }
      const curve = new THREE.EllipseCurve(c.x, c.y, c.r, c.r, c.angle1, c.angle2, false, 0);
      return curve.getPoints(CIRCLE_SEGMENTS).map((p) => new THREE.Vector3(p.x, p.y, 0));
    }
    case "text":
      return undefined;
  }
}

// Two stroke commands may share one draw call only if every piece of state a
// `LineBasicMaterial` carries agrees. Colour is the exception -- it moves to a
// per-vertex attribute -- which is what makes the hex tiling collapsible at
// all, since those tiles differ in colour and in nothing else.
function strokeBatchKey(c: DrawCommand): string {
  return `${c.blend ?? "normal"}|${c.alpha ?? 1}`;
}

// One draw call for a run of same-state strokes.
//
// Each polyline is expanded into explicit segment pairs rather than kept as a
// strip: a strip cannot represent two disjoint runs in one buffer without a
// degenerate connecting segment, and a stray hairline across the pitch is
// exactly the artifact that would be blamed on something else for an hour.
// The cost is duplicated interior vertices, which is a vertex-buffer trade
// against a per-object draw call -- at 353 objects that is not close.
function batchStrokeRun(
  run: readonly { readonly command: DrawCommand; readonly points: readonly THREE.Vector3[] }[],
  opts?: PaintOptions,
): THREE.LineSegments {
  const positions: number[] = [];
  const colors: number[] = [];
  for (const { command, points } of run) {
    const color = colorOf(command);
    for (let i = 0; i + 1 < points.length; i += 1) {
      const a = points[i];
      const b = points[i + 1];
      if (a === undefined || b === undefined) {
        continue;
      }
      positions.push(a.x, a.y, a.z, b.x, b.y, b.z);
      colors.push(color.r, color.g, color.b, color.r, color.g, color.b);
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  const head = run[0]?.command;
  // Per-vertex colour, so unlike `lineMaterial` the command's own colour is
  // NOT part of this material's identity -- only the run head's alpha and
  // blend are (`strokeBatchKey` already guarantees every command in a run
  // agrees on both).
  const depthTest = depthTestOf(opts);
  const additive = head?.blend === "add";
  const material = sharedMaterial(
    `batch|${head?.alpha === undefined ? "-" : String(head.alpha)}|${additive ? "add" : "alpha"}|${depthTest ? "z" : "-"}`,
    () =>
      new THREE.LineBasicMaterial({
        vertexColors: true,
        transparent: head?.alpha !== undefined,
        opacity: head?.alpha ?? 1,
        blending: additive ? THREE.AdditiveBlending : THREE.NormalBlending,
        depthTest,
      }),
  );
  return new THREE.LineSegments(geometry, material);
}

function buildOne(c: DrawCommand, opts?: PaintOptions): THREE.Object3D {
  switch (c.kind) {
    case "rect": {
      if (c.mode === "fill") {
        const geometry = new THREE.PlaneGeometry(c.w, c.h);
        const mesh = new THREE.Mesh(geometry, fillMaterial(c, opts));
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
      return new THREE.Line(
        new THREE.BufferGeometry().setFromPoints(points),
        lineMaterial(c, opts),
      );
    }
    case "circle": {
      if (c.mode === "fill") {
        const geometry = new THREE.CircleGeometry(c.r, CIRCLE_SEGMENTS);
        const mesh = new THREE.Mesh(geometry, fillMaterial(c, opts));
        mesh.position.set(c.x, c.y, 0);
        return mesh;
      }
      return new THREE.LineLoop(
        new THREE.BufferGeometry().setFromPoints(ringPoints(0, 0, c.r)),
        lineMaterial(c, opts),
      )
        .translateX(c.x)
        .translateY(c.y);
    }
    case "ellipse": {
      const curve = new THREE.EllipseCurve(0, 0, c.rx, c.ry, 0, 2 * Math.PI, false, 0);
      const points = curve.getPoints(CIRCLE_SEGMENTS).map((p) => new THREE.Vector3(p.x, p.y, 0));
      if (c.mode === "fill") {
        const shape = new THREE.Shape(curve.getPoints(CIRCLE_SEGMENTS));
        const mesh = new THREE.Mesh(new THREE.ShapeGeometry(shape), fillMaterial(c, opts));
        mesh.position.set(c.x, c.y, 0);
        return mesh;
      }
      const line = new THREE.LineLoop(
        new THREE.BufferGeometry().setFromPoints(points),
        lineMaterial(c, opts),
      );
      line.position.set(c.x, c.y, 0);
      return line;
    }
    case "polygon": {
      const points = polylinePoints(c.points);
      if (c.mode === "fill") {
        const shapePoints = points.map((p) => new THREE.Vector2(p.x, p.y));
        const mesh = new THREE.Mesh(
          new THREE.ShapeGeometry(new THREE.Shape(shapePoints)),
          fillMaterial(c, opts),
        );
        return mesh;
      }
      const closed = [...points, points[0]].filter((p): p is THREE.Vector3 => p !== undefined);
      return new THREE.Line(
        new THREE.BufferGeometry().setFromPoints(closed),
        lineMaterial(c, opts),
      );
    }
    case "line": {
      const points = polylinePoints(c.points);
      return new THREE.Line(
        new THREE.BufferGeometry().setFromPoints(points),
        lineMaterial(c, opts),
      );
    }
    case "arc": {
      const curve = new THREE.EllipseCurve(0, 0, c.r, c.r, c.angle1, c.angle2, false, 0);
      const points = curve.getPoints(CIRCLE_SEGMENTS).map((p) => new THREE.Vector3(p.x, p.y, 0));
      if (c.mode === "fill") {
        const shapePoints = [
          new THREE.Vector2(0, 0),
          ...points.map((p) => new THREE.Vector2(p.x, p.y)),
        ];
        const mesh = new THREE.Mesh(
          new THREE.ShapeGeometry(new THREE.Shape(shapePoints)),
          fillMaterial(c, opts),
        );
        mesh.position.set(c.x, c.y, 0);
        return mesh;
      }
      const line = new THREE.Line(
        new THREE.BufferGeometry().setFromPoints(points),
        lineMaterial(c, opts),
      );
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
 * (ts/vitest.config.ts). Passing `buildText` lets a "node" spec exercise
 * everything else `paint`/`appendCommands` do -- clearing, rebuilding,
 * disposal, non-text object construction -- without a DOM polyfill. See
 * match_hud.spec.ts's population suite for why this was chosen over a
 * jsdom-scoped spec file.
 *
 * DEPTH PLACEMENT (`z`/`renderOrder`/`depthTest`). Every object `buildOne`
 * constructs sits at local z=0 -- draw2d.ts's whole vocabulary is screen-space
 * 2D, and painter's-algorithm ordering among tied-z siblings falls out of
 * `group.children` insertion order for free (three.js's default depth
 * function is `LessEqualDepth`, so a later draw at an EQUAL depth still wins).
 * That stops being sufficient the moment real 3D geometry with its own depth
 * extent shares the same `THREE.Scene` -- pitch.ts's rigged characters, see
 * that file's "DEPTH ZONES" section and scene.ts's `SceneRoot` doc comment.
 * These three options let a caller opt PART of a `DrawCommand[]` batch into
 * that shared depth space without `buildOne` itself needing to know why:
 *
 *   - `z`: added to every built object's `position.z` (not set outright, so
 *     it composes with whatever `buildOne` already wrote, which is always 0
 *     today -- see above).
 *   - `renderOrder`: forwarded to every built object's own `renderOrder`.
 *     three.js honors this ahead of its internal opaque-list sort, the same
 *     mechanism scene.ts already uses to keep `hudGroup` above `pitchGroup`.
 *   - `depthTest`: forwarded to every built object's material(s). Content
 *     that must stay visually on top (or behind) regardless of a shared
 *     depth buffer's contents -- pitch.ts's post-entity overlay layer is the
 *     one caller today -- sets this `false` instead of trying to out-z
 *     everything else.
 *
 * All three default to leaving `buildOne`'s output exactly as it already was
 * (z=0, three.js's own default `renderOrder` of 0, `depthTest` untouched), so
 * every existing caller (match_hud.ts, bloom.ts, the non-rigged half of
 * pitch.ts) is unaffected.
 */
export interface PaintOptions {
  readonly buildText?: (c: TextCommand) => THREE.Object3D;
  readonly z?: number;
  readonly renderOrder?: number;
  readonly depthTest?: boolean;
  /**
   * Merge consecutive runs of stroke commands that share render state into
   * one `THREE.LineSegments` each, instead of one `THREE.Line` per command.
   *
   * OFF by default, and deliberately so: `pitch.spec.ts`'s differential
   * against a recorded reference draw stream inspects `group.children` per
   * command, and batching is exactly the transformation that would make that
   * comparison meaningless. Callers that want the draw-call win opt in.
   *
   * Measured on the static pitch pass, which is what motivated this: 353
   * `Line` objects -- one draw call each, most of them hex floor tiles --
   * collapsed into a handful. See `batchStrokeRun`.
   */
  readonly batchStrokes?: boolean;
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
    // Re-bound at the default instantiation on purpose. `instanceof` against
    // three.js's GENERIC classes narrows to `Mesh<any, any, any>` /
    // `Line<any, any>`, which switches type-checking off for `.geometry` and
    // `.material` below. The runtime check is unchanged.
    const owner: THREE.Mesh | THREE.Line = child;
    owner.geometry.dispose();
    disposeMaterial(owner.material);
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
    // Owned by `materialCache`, not by the object being removed -- disposing
    // it here is exactly the per-frame program destruction #403 turned out to
    // be (see the SHARED MATERIALS section above). The cache never disposes
    // either: eviction and `resetMaterialCache` clear this flag and hand the
    // material back, so the NEXT clear of an object still holding it takes
    // this branch and disposes it normally. See `release`.
    if (m.userData[SHARED_MATERIAL_FLAG] === true) {
      continue;
    }
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
 * at specific points in the child order -- pitch.ts's depth-sorted entity
 * pass is the one today (see its file header and `pitch.draw`): the
 * painter's-algorithm depth sort requires the ball and the rigged player
 * meshes to land in `group.children` interleaved by
 * world-depth, not as one `paint()` replacing everything and a second
 * wiping it out.
 */
export function appendCommands(
  group: THREE.Group,
  commands: readonly DrawCommand[],
  opts?: PaintOptions,
): void {
  if (opts?.batchStrokes !== true) {
    for (const c of commands) {
      const obj = buildOne(c, opts);
      applyDepthPlacement(obj, opts);
      group.add(obj);
    }
    return;
  }

  // Only CONSECUTIVE strokes merge. Painter's-algorithm order is the whole
  // correctness argument for this group's child list, so a run stops at the
  // first command that is not a same-state stroke -- reordering to gather
  // more into a batch would be a rendering change dressed up as an
  // optimisation.
  let run: { readonly command: DrawCommand; readonly points: readonly THREE.Vector3[] }[] = [];
  let key: string | undefined;
  const flush = (): void => {
    if (run.length === 0) {
      return;
    }
    // A run of one gains nothing and would needlessly change the object type
    // a caller sees, so it takes the ordinary path.
    const obj = run.length === 1 ? buildOne(run[0]!.command, opts) : batchStrokeRun(run, opts);
    applyDepthPlacement(obj, opts);
    group.add(obj);
    run = [];
    key = undefined;
  };

  for (const c of commands) {
    const points = strokePolyline(c);
    if (points === undefined) {
      flush();
      const obj = buildOne(c, opts);
      applyDepthPlacement(obj, opts);
      group.add(obj);
      continue;
    }
    const commandKey = strokeBatchKey(c);
    if (key !== undefined && commandKey !== key) {
      flush();
    }
    key = commandKey;
    run.push({ command: c, points });
  }
  flush();
}

// See PaintOptions' doc comment. Applied once per built object, after
// `buildOne` -- kept out of `buildOne` itself so that function stays a pure
// command -> object mapping with no knowledge of why a caller might want its
// output repositioned in depth.
function applyDepthPlacement(obj: THREE.Object3D, opts?: PaintOptions): void {
  if (opts === undefined) {
    return;
  }
  if (opts.z !== undefined) {
    obj.position.z += opts.z;
  }
  if (opts.renderOrder !== undefined) {
    obj.renderOrder = opts.renderOrder;
  }
  if (opts.depthTest !== undefined) {
    // Annotated for the same reason as `disposeObject` above: the `instanceof`
    // narrowing of three.js's generic classes is `any` in its type arguments.
    const materialOwner: THREE.Mesh | THREE.Line | THREE.Sprite | undefined =
      obj instanceof THREE.Mesh || obj instanceof THREE.Line || obj instanceof THREE.Sprite
        ? obj
        : undefined;
    if (materialOwner !== undefined) {
      const materials = Array.isArray(materialOwner.material)
        ? materialOwner.material
        : [materialOwner.material];
      for (const m of materials) {
        // A cached material was already CONSTRUCTED with this `depthTest`
        // (it is part of `materialCache`'s key -- see `depthTestOf`), so
        // writing it here would be a no-op at best. Skipped explicitly so
        // that stays true by construction rather than by coincidence: a
        // shared instance must never be mutated per-object, or two callers
        // with different `PaintOptions` would fight over one material.
        if (m.userData[SHARED_MATERIAL_FLAG] === true) {
          continue;
        }
        m.depthTest = opts.depthTest;
      }
    }
  }
}

/**
 * Rebuilds `group`'s children from `commands`, in order. `group` is expected
 * to sit under an orthographic camera whose view spans `[0, vp.w] x [0,
 * vp.h]` with y increasing downward (screen convention) -- the same space
 * `camera.project`'s output already lives in, so nothing here re-derives a
 * projection.
 *
 * Rebuilding every call is the simplest correct thing (a fully-immediate-mode
 * redraw each frame) and OBJECTS and GEOMETRIES are still rebuilt that way:
 * measured, that costs ~0.3 ms/frame
 * (177 `bufferData` + 177 buffer create/delete + 88 VAO create/delete), which
 * is not what #403 was.
 *
 * MATERIALS are the exception, and are no longer rebuilt or disposed here:
 * disposing one drops three.js's refcount on the compiled GL program behind
 * it, and since this loop disposes every old child BEFORE building any new
 * one, that count passed through zero once per frame and destroyed five
 * programs -- which then had to be recompiled and, worse, SYNCHRONOUSLY
 * re-validated, for ~7.8 ms of a 13.33 ms frame budget. See the SHARED
 * MATERIALS section above for the measurement and the mechanism.
 */
export function paint(
  group: THREE.Group,
  commands: readonly DrawCommand[],
  opts?: PaintOptions,
): void {
  for (const child of [...group.children]) {
    group.remove(child);
    disposeObject(child);
  }
  appendCommands(group, commands, opts);
}
