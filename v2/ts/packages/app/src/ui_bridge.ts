// Local, structurally-identical stand-ins for `@gc/ui`'s `viewport.ts`
// (`create`/`toVirtual`/`toActual`) and `hit.ts`'s `find`.
//
// `@gc/ui` is not a declared dependency of `@gc/app` (this task may not
// edit package.json -- report per the task brief). `@gc/input`'s
// `controller.ts` hit exactly this boundary already and resolved it the
// same way: inject a small structurally-identical port rather than adding
// the dependency (see controller.ts's header comment). This module is that
// composition one layer up -- `app.ts`/`compatibility_flow.ts` both need
// actual-window <-> virtual-canvas coordinate mapping and widget lookup by
// id, and duplicating a ~15-line pure coordinate transform locally is
// cheaper than blocking this port on a workspace `package.json` edit.

import type { ViewportMapper, ViewportTransform } from "@gc/input";

export type { ViewportTransform };

function create(actualW: number, actualH: number, baseW = 960, baseH = 540): ViewportTransform {
  if (!(actualW > 0 && actualH > 0)) {
    throw new Error("viewport dimensions must be positive");
  }
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

function toVirtual(transform: ViewportTransform, x: number, y: number): readonly [number, number] | null {
  const vx = (x - transform.offsetX) / transform.scale;
  const vy = (y - transform.offsetY) / transform.scale;
  if (vx < 0 || vy < 0 || vx > transform.baseW || vy > transform.baseH) {
    return null;
  }
  return [vx, vy];
}

function toActual(transform: ViewportTransform, x: number, y: number): readonly [number, number] {
  return [transform.offsetX + x * transform.scale, transform.offsetY + y * transform.scale];
}

export const viewport = { create, toVirtual, toActual };

/** Satisfies `@gc/input`'s `controller.normalize`'s injected `ViewportMapper`. */
export const viewportMapper: ViewportMapper = { toVirtual };

/** Structurally identical to `@gc/ui`'s `Widget`/`Layout` -- only the `id`/`rect` fields this bridge needs. */
export interface HitRect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface HitWidget {
  readonly id: string;
  readonly rect?: HitRect;
}

function find<T extends HitWidget>(layout: readonly T[], id: string): T | null {
  for (const w of layout) {
    if (w.id === id) {
      return w;
    }
  }
  return null;
}

export const hit = { find };

/**
 * Reaches into a `@gc/screens` `Menu` instance's layout, the same way the
 * Lua original's `game.screens.menu.lua`/`game.compatibility_flow.lua`
 * read `screen.def.layout(screen.state)` directly -- duck-typed tables have
 * no privacy in Lua. The ported `Menu` class (`@gc/screens`'s `menu.ts`)
 * makes `def`/`state` TypeScript-`private`, which only hides them from the
 * *type checker*; the fields are ordinary properties at runtime (TS
 * `private` erases, unlike real `#private` fields). This introspection is
 * exactly what `compatibility_flow.lua`'s automated-input harness and
 * `app_flow_spec.lua`'s test helper both rely on, so it is ported as a
 * shared, explicit, well-contained cast rather than reimplemented ad hoc at
 * every call site.
 */
interface MenuIntrospection {
  readonly def: { layout(state: unknown): readonly HitWidget[] };
  readonly state: unknown;
}

export function menuLayout(screen: unknown): readonly HitWidget[] | undefined {
  if (typeof screen !== "object" || screen === null || !("def" in screen) || !("state" in screen)) {
    return undefined;
  }
  const menu = screen as unknown as MenuIntrospection;
  if (typeof menu.def !== "object" || menu.def === null || typeof menu.def.layout !== "function") {
    return undefined;
  }
  return menu.def.layout(menu.state);
}
