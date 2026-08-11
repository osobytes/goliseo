// Ported from game/compatibility_flow.lua.
//
// Drives the product flow through the normal input seam (clicks on the
// current screen's widgets) so a native and a browser build can be exercised
// identically for the OMP compatibility harness. `game.ui.hit`/
// `game.ui.viewport` are `@gc/ui`'s -- not a declared dependency of this
// package (report per the task brief) -- so this module uses `ui_bridge.ts`'s
// structurally-identical `hit.find`/`viewport.toActual`/`menuLayout`.

import { hit, menuLayout, viewport, type ViewportTransform } from "./ui_bridge.ts";

export interface CompatibilityFlowApp {
  readonly transform: ViewportTransform;
  readonly adapter: { readonly kind: string };
  readonly stack: { current(): unknown };
  currentRoute(): string;
  event(evt: { readonly kind: "click"; readonly x: number; readonly y: number; readonly button: number }): void;
}

const ROUTE_WIDGETS: Readonly<Record<string, string>> = {
  title: "play",
  squad: "next",
  formation: "next",
  tactic: "kickoff",
  pause: "resume",
};

function clickWidget(app: CompatibilityFlowApp, id: string, beforeClick?: () => void): boolean {
  const layout = menuLayout(app.stack.current());
  if (!layout) {
    return false;
  }
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    return false;
  }
  const [x, y] = viewport.toActual(app.transform, widget.rect.x + widget.rect.w / 2, widget.rect.y + widget.rect.h / 2);
  if (beforeClick) {
    beforeClick();
  }
  app.event({ kind: "click", x, y, button: 1 });
  return true;
}

export class CompatibilityFlow {
  nextActionAt = 0;
  actionDelay = 0.5;
  finished = false;
  lastRoute: string | undefined;
  private readonly recordInput: ((kind: string) => void) | undefined;

  constructor(recordInput?: (kind: string) => void) {
    this.recordInput = recordInput;
  }

  update(app: CompatibilityFlowApp, now: number): void {
    if (this.finished || now < this.nextActionAt) {
      return;
    }
    const route = app.currentRoute();
    if (route !== this.lastRoute) {
      this.lastRoute = route;
      if (route === "pause") {
        this.nextActionAt = now + this.actionDelay;
        return;
      }
    }
    if (route === "result") {
      this.finished = true;
      return;
    }
    let widget = ROUTE_WIDGETS[route];
    if (route === "match" && app.adapter.kind === "fake") {
      widget = "complete";
    }
    if (
      widget !== undefined &&
      clickWidget(app, widget, () => {
        if (this.recordInput) {
          this.recordInput(`compat_click_${widget}`);
        }
      })
    ) {
      this.nextActionAt = now + this.actionDelay;
    }
  }
}
