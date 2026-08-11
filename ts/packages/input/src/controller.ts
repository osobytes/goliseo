// Normalizes a raw input event (key press, gamepad button, click, or an
// already-normalized action) into the ActionEvent/click shape screens
// consume.
//
// The click branch needs @gc/ui's actual-window -> virtual-canvas
// coordinate mapping (viewport.ts). @gc/ui owns that, but @gc/input's
// package.json cannot gain a dependency on @gc/ui here: other workspace
// packages are being edited concurrently and share the same
// pnpm-lock.yaml, so editing a workspace package.json risks corrupting
// that lockfile mid-flight (this is a needed `@gc/ui` dependency deliberately
// not added — the "needed dependency, report rather than add it" case the
// task brief calls out). `ViewportMapper` is injected instead: the real
// implementation is `@gc/ui`'s `viewport.toVirtual`/`ViewportTransform`,
// which are structurally identical to the types below, so
// `{ toVirtual: viewport.toVirtual }` from `@gc/ui` satisfies this
// interface without a static import. Wiring that composition is a later
// milestone's job (`@gc/screens`, which will depend on both packages).

import { actions } from "./actions.ts";
import type { ActionEvent } from "./actions.ts";

/** Structurally identical to `@gc/ui`'s `ViewportTransform` — see this file's header comment. */
export interface ViewportTransform {
  readonly baseW: number;
  readonly baseH: number;
  readonly actualW: number;
  readonly actualH: number;
  readonly scale: number;
  readonly offsetX: number;
  readonly offsetY: number;
}

/** The one piece of `@gc/ui`'s `viewport` module this file needs. */
export interface ViewportMapper {
  toVirtual(
    transform: ViewportTransform,
    x: number,
    y: number,
  ): readonly [x: number, y: number] | null;
}

export interface RawGamepadEvent {
  readonly kind: "gamepad";
  readonly button: string;
  readonly pressed?: boolean;
}

export type KeyEvent = { readonly kind: "key"; readonly key: string; readonly pressed?: boolean };
export type ClickEvent = {
  readonly kind: "click";
  readonly x: number;
  readonly y: number;
  readonly button?: number;
};

export type ControllerInputEvent = ActionEvent | KeyEvent | ClickEvent | RawGamepadEvent;

function normalize(
  event: ControllerInputEvent,
  transform: ViewportTransform,
  viewportMapper: ViewportMapper,
): ActionEvent | ClickEvent | null {
  let normalized: ActionEvent | null = null;
  if (event.kind === "action") {
    normalized = event;
  } else if (event.kind === "key") {
    normalized = actions.fromKey(event.key, event.pressed);
  } else if (event.kind === "gamepad") {
    normalized = actions.fromGamepad(event.button, event.pressed);
  } else if (event.kind === "click") {
    const virtual = viewportMapper.toVirtual(transform, event.x, event.y);
    if (virtual === null) {
      return null;
    }
    const [x, y] = virtual;
    return { kind: "click", x, y, button: event.button ?? 1 };
  }
  if (
    normalized !== null &&
    normalized.kind === "action" &&
    normalized.pressed === false &&
    normalized.action !== "equipment"
  ) {
    return null;
  }
  return normalized;
}

export const controller = { normalize };
