// Ported from game/render/combat.lua.
//
// Combat telegraphs (arcs, guard fans, ranged sightlines) and projectile
// trails, drawn on the ground under the players (`drawUnderCommands`) and
// on top of everything (`drawOverCommands`). Pure content: every function
// here returns a `DrawCommand[]` (draw2d.ts) rather than calling
// `love.graphics`/three.js directly, so it is fully testable headless.
//
// Boundary note (v2/README.md rule 6.7): `CombatPlayerPresentation` and
// `CombatProjectilePresentation` are `@gc/presentation`'s (already a
// declared dependency), reused rather than redeclared. `RenderFrame` is the
// (Rust) `render/frame.lua` producer's output (`render/**` -> Rust
// `crates/gc-render`, v2/README.md #2) -- only the slice this module reads
// is declared locally. `player_index` on a `CombatPlayerPresentation` is
// 1-based (an identifier that survives the wasm boundary, matching
// `@gc/presentation`'s own convention), so it is adjusted by one when
// indexing this module's 0-based `RenderFrame.players` arrays.

import type { Vec2 } from "@gc/core";
import type { CombatPlayerPresentation, CombatProjectilePresentation } from "@gc/presentation";
import { DrawList, paint, type DrawCommand, type Project, type RGB } from "./draw2d.ts";
import * as THREE from "three";

/** The slice of `RenderFrame` this module reads. */
export interface CombatDrawFrame {
  readonly combat?: {
    readonly enabled: boolean;
    readonly players: readonly CombatPlayerPresentation[];
    readonly projectiles: readonly CombatProjectilePresentation[];
  };
  readonly players: {
    readonly x: readonly number[];
    readonly y: readonly number[];
  };
}

const FAMILY_COLORS: Readonly<Record<string, RGB>> = {
  unarmed: [0.55, 0.95, 1],
  guard: [0.75, 0.9, 1],
  light_melee: [1, 0.72, 0.28],
  ranged: [1, 0.45, 0.78],
};

function familyColor(familyId: string | undefined): RGB {
  const color = familyId !== undefined ? FAMILY_COLORS[familyId] : undefined;
  if (color === undefined) {
    throw new Error(`combat.ts: unknown or missing family_id: ${String(familyId)}`);
  }
  return color;
}

function unit(direction: Vec2): readonly [number, number] {
  const length = Math.sqrt(direction.x * direction.x + direction.y * direction.y);
  if (length < 1e-9) {
    return [1, 0];
  }
  return [direction.x / length, direction.y / length];
}

function telegraphAlpha(sample: CombatPlayerPresentation): number {
  if (sample.phase === "active") {
    return 0.95;
  } else if (sample.phase === "guard" || sample.phase === "aim") {
    return 0.72;
  }
  return 0.35 + sample.phase_progress * 0.35;
}

function drawArc(dl: DrawList, project: Project, sample: CombatPlayerPresentation, position: { x: number; y: number }): void {
  const color = familyColor(sample.family_id);
  const [directionX, directionY] = unit(sample.direction);
  const heading = Math.atan2(directionY, directionX);
  const frontArcDegrees = sample.front_arc_degrees;
  if (frontArcDegrees === undefined) {
    throw new Error("combat.ts: draw_arc requires front_arc_degrees");
  }
  const halfArc = (frontArcDegrees / 2) * (Math.PI / 180);
  const radius = sample.reach_px ?? 34;
  const alpha = telegraphAlpha(sample);
  const [centerX, centerY] = project(position.x, position.y);
  const lineWidth = sample.telegraph_kind === "guard_arc" ? 2.5 : 1.8;

  let priorX: number | undefined;
  let priorY: number | undefined;
  let firstX: number | undefined;
  let firstY: number | undefined;
  let lastX: number | undefined;
  let lastY: number | undefined;
  for (let index = 0; index <= 14; index += 1) {
    const angle = heading - halfArc + (2 * halfArc * index) / 14;
    const worldX = position.x + Math.cos(angle) * radius;
    const worldY = position.y + Math.sin(angle) * radius;
    const [screenX, screenY] = project(worldX, worldY);
    if (priorX !== undefined && priorY !== undefined) {
      dl.line([priorX, priorY, screenX, screenY], color, { alpha, lineWidth });
    } else {
      firstX = screenX;
      firstY = screenY;
    }
    priorX = screenX;
    priorY = screenY;
    lastX = screenX;
    lastY = screenY;
  }

  if (sample.telegraph_kind === "guard_arc") {
    let innerPriorX: number | undefined;
    let innerPriorY: number | undefined;
    for (let index = 0; index <= 10; index += 1) {
      const angle = heading - halfArc + (2 * halfArc * index) / 10;
      const [screenX, screenY] = project(position.x + Math.cos(angle) * radius * 0.7, position.y + Math.sin(angle) * radius * 0.7);
      if (innerPriorX !== undefined && innerPriorY !== undefined) {
        dl.line([innerPriorX, innerPriorY, screenX, screenY], color, { alpha, lineWidth });
      }
      innerPriorX = screenX;
      innerPriorY = screenY;
    }
    if (firstX === undefined || firstY === undefined || lastX === undefined || lastY === undefined) {
      throw new Error("combat.ts: draw_arc guard fan requires at least one arc sample");
    }
    dl.line([centerX, centerY, firstX, firstY], color, { alpha, lineWidth });
    dl.line([centerX, centerY, lastX, lastY], color, { alpha, lineWidth });
  } else {
    const [tipX, tipY] = project(position.x + directionX * radius, position.y + directionY * radius);
    dl.line([centerX, centerY, tipX, tipY], color, { alpha, lineWidth });
    if (sample.family_id === "unarmed") {
      dl.circle("line", tipX, tipY, 4, color, { alpha, lineWidth });
    } else {
      const normalX = -directionY;
      const normalY = directionX;
      const [leftX, leftY] = project(position.x + directionX * radius + normalX * 7, position.y + directionY * radius + normalY * 7);
      const [rightX, rightY] = project(position.x + directionX * radius - normalX * 7, position.y + directionY * radius - normalY * 7);
      dl.line([leftX, leftY, rightX, rightY], color, { alpha, lineWidth });
    }
  }
}

function drawLine(dl: DrawList, project: Project, sample: CombatPlayerPresentation, position: { x: number; y: number }): void {
  const color = familyColor("ranged");
  const [directionX, directionY] = unit(sample.direction);
  const range = sample.projectile_range_px ?? 300;
  const alpha = telegraphAlpha(sample);
  const [startX, startY] = project(position.x, position.y);
  const [endX, endY] = project(position.x + directionX * range, position.y + directionY * range);
  const lineWidth = sample.phase === "active" ? 3 : 1.5;
  dl.line([startX, startY, endX, endY], color, { alpha, lineWidth });

  const normalX = -directionY;
  const normalY = directionX;
  for (let step = 1; step <= 4; step += 1) {
    const distance = (range * step) / 5;
    const [leftX, leftY] = project(position.x + directionX * distance + normalX * 5, position.y + directionY * distance + normalY * 5);
    const [rightX, rightY] = project(position.x + directionX * distance - normalX * 5, position.y + directionY * distance - normalY * 5);
    dl.line([leftX, leftY, rightX, rightY], color, { alpha, lineWidth });
  }
}

/** Telegraphs on the ground under the players. See file header. */
export function drawUnderCommands(frame: CombatDrawFrame, project: Project): DrawCommand[] {
  const dl = new DrawList();
  const model = frame.combat;
  if (model === undefined || !model.enabled) {
    return dl.commands;
  }
  for (const sample of model.players) {
    const slot = sample.player_index - 1; // 1-based content id -> 0-based array index
    const x = frame.players.x[slot];
    const y = frame.players.y[slot];
    if (x === undefined || y === undefined) {
      continue;
    }
    const position = { x, y };
    if (sample.telegraph_kind === "line") {
      drawLine(dl, project, sample, position);
    } else if (sample.telegraph_kind !== undefined) {
      drawArc(dl, project, sample, position);
    }
  }
  return dl.commands;
}

/** Projectile trails, drawn on top of everything. See file header. */
export function drawOverCommands(frame: CombatDrawFrame, project: Project): DrawCommand[] {
  const dl = new DrawList();
  const model = frame.combat;
  if (model === undefined || !model.enabled) {
    return dl.commands;
  }
  for (const projectile of model.projectiles) {
    const [directionX, directionY] = unit(projectile.direction);
    const [tailX, tailY] = project(projectile.position.x - directionX * 18, projectile.position.y - directionY * 18);
    const [x, y, scale] = project(projectile.position.x, projectile.position.y);
    const radius = Math.max(3, 5 * scale);
    dl.line([tailX, tailY, x, y], [1, 0.45, 0.78], { alpha: 0.58, lineWidth: Math.max(1, 3 * scale) });
    dl.polygon("fill", [x, y - radius, x + radius, y, x, y + radius, x - radius, y], [1, 0.9, 1], { alpha: 0.98 });
    dl.circle("fill", x, y, radius * 0.32, [0.15, 0.03, 0.18], { alpha: 0.9 });
  }
  return dl.commands;
}

/** Impure: paints the under-players telegraphs into `group`. Untested -- see draw2d.ts. */
export function drawUnder(group: THREE.Group, frame: CombatDrawFrame, project: Project): void {
  paint(group, drawUnderCommands(frame, project));
}

/** Impure: paints the over-everything projectile trails into `group`. Untested -- see draw2d.ts. */
export function drawOver(group: THREE.Group, frame: CombatDrawFrame, project: Project): void {
  paint(group, drawOverCommands(frame, project));
}
