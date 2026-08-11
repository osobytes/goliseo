// Letterboxing an actual window into a fixed virtual canvas, and mapping
// points between the two spaces.

import { invariant } from "./assert.ts";

export interface ViewportTransform {
  readonly baseW: number;
  readonly baseH: number;
  readonly actualW: number;
  readonly actualH: number;
  readonly scale: number;
  readonly offsetX: number;
  readonly offsetY: number;
}

function create(actualW: number, actualH: number, baseW = 960, baseH = 540): ViewportTransform {
  invariant(actualW > 0 && actualH > 0, "viewport dimensions must be positive");
  const scale = Math.min(actualW / baseW, actualH / baseH);
  return {
    baseW,
    baseH,
    actualW,
    actualH,
    scale,
    offsetX: (actualW - baseW * scale) / 2,
    offsetY: (actualH - baseH * scale) / 2,
  };
}

/** Maps an actual-window point into virtual space, or `null` when it falls outside the canvas. */
function toVirtual(
  transform: ViewportTransform,
  x: number,
  y: number,
): readonly [x: number, y: number] | null {
  const vx = (x - transform.offsetX) / transform.scale;
  const vy = (y - transform.offsetY) / transform.scale;
  if (vx < 0 || vy < 0 || vx > transform.baseW || vy > transform.baseH) {
    return null;
  }
  return [vx, vy];
}

function toActual(
  transform: ViewportTransform,
  x: number,
  y: number,
): readonly [x: number, y: number] {
  return [transform.offsetX + x * transform.scale, transform.offsetY + y * transform.scale];
}

export const viewport = { create, toVirtual, toActual };
