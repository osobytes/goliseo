// The app's seam onto `@gc/ui`.
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
 * Reaches into a `@gc/screens` `Menu` instance's layout, reading
 * `screen.def.layout(screen.state)` directly.
 *
 * The `Menu` class makes `def`/`state` TypeScript-`private`, which hides
 * them from the type checker only; `private` erases at runtime, unlike a
 * real `#private` field, so the properties are still ordinary and
 * readable. `compatibility_flow.ts`'s automated-input harness and
 * `app.spec.ts`'s test helper both depend on exactly this, so it is
 * implemented as one explicit, well-contained cast rather than
 * reimplemented ad hoc at every call site.
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
