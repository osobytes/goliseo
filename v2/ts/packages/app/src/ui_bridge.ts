// The app's seam onto `@gc/ui`.
//
// This file used to carry local copies of `viewport.create`/`toVirtual`/
// `toActual` and `hit.find`, because `@gc/ui` was not yet a declared dependency
// of `@gc/app` and porting agents were forbidden from editing `package.json`
// while others were running (a concurrent lockfile write corrupts the
// workspace). That dependency now exists, so the duplicates are gone and this
// module re-exports the real implementations.
//
// What remains here is genuinely app-specific: the `ViewportMapper` adapter
// `@gc/input` asks for, and the `Menu` introspection below.

import { hit, viewport } from "@gc/ui";
import type { Layout, Rect, Widget } from "@gc/ui";
import type { ViewportMapper, ViewportTransform } from "@gc/input";

export { hit, viewport };
export type { ViewportTransform };
export type HitRect = Rect;
export type HitWidget = Widget;

/** Satisfies the `ViewportMapper` that `@gc/input`'s `controller.normalize` injects. */
export const viewportMapper: ViewportMapper = { toVirtual: viewport.toVirtual };

/**
 * Reaches into a `@gc/screens` `Menu` instance's layout, the same way the Lua
 * original's `game/screens/menu.lua` and `game/compatibility_flow.lua` read
 * `screen.def.layout(screen.state)` directly — duck-typed tables have no privacy
 * in Lua.
 *
 * The ported `Menu` class makes `def`/`state` TypeScript-`private`, which hides
 * them from the type checker only; `private` erases at runtime, unlike a real
 * `#private` field, so the properties are still ordinary and readable.
 * `compatibility_flow.lua`'s automated-input harness and `app_flow_spec.lua`'s
 * test helper both depend on exactly this, so it is ported as one explicit,
 * well-contained cast rather than reimplemented ad hoc at every call site.
 */
interface MenuIntrospection {
  readonly def: { layout(state: unknown): Layout };
  readonly state: unknown;
}

export function menuLayout(screen: unknown): Layout | undefined {
  if (typeof screen !== "object" || screen === null || !("def" in screen) || !("state" in screen)) {
    return undefined;
  }
  const menu = screen as unknown as MenuIntrospection;
  if (typeof menu.def !== "object" || menu.def === null || typeof menu.def.layout !== "function") {
    return undefined;
  }
  return menu.def.layout(menu.state);
}
