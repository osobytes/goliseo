// The case "is reachable from the title and returns to it" is left
// `it.skip` in `@gc/screens` (packages/screens/src/lobby_flow.spec.ts,
// "online lobby screen shell" describe block) because it is actually about
// `@gc/app`'s routing (`App.stack`, `App:current_route`), and `@gc/screens`
// cannot depend on `@gc/app` -- the dependency runs the other way (see that
// file's comment, left in place, for the full reasoning). `@gc/app` is the
// correct side of that edge; this file is where the case lives, alongside
// this package's own `online_match_flow.spec.ts` (the same move, for the
// sibling "routes the lobby's synchronized start..." case).
//
// # What this case proves, and at what level of fakery
//
// This test mounts a single real `App`, clicks the title's real "online
// lobby" widget, and asserts the route becomes `"lobby"`; then presses
// escape and asserts the route returns to `"title"`. It never drives a real
// coordinator, transport, or handshake -- the assertion is entirely about
// `App`'s own shell routing (`showLobby`/`popRoute`), not about online
// lobby session logic. It drives `App`'s real title menu (`@gc/screens`'s
// `title.ts`, via `App.stack`/`clickWidget`, exactly like `app.spec.ts`'s
// own cases) and a hand-written `OnlinePorts.newLobbyScreen` fake -- the
// same level of fakery `online_match_flow.spec.ts` (this package) already
// uses for `App`-level routing cases, and the one `app.spec.ts` itself uses
// for the offline fake match adapter. A real `OnlineLobby`/`lobby_link`
// round trip (open a star, complete a handshake) is exercised for real
// already, in `@gc/screens`' own "online lobby screen shell" describe block
// (the sibling case right above where this one used to sit) -- duplicating
// that here would prove transport/session behaviour a second time under a
// fake `OnlinePorts.requestMatchSession`/`newOnlineMatchScreen`, not this
// case's actual claim.
//
// The fake lobby screen's `event` mirrors the one behaviour this case
// depends on: `@gc/screens`'s real `lobby.ts` turns a `back` action into a
// `{ go: "back" }`-shaped app action once there is no session to preserve
// (an empty/unstarted lobby leaving), which `App.handleAction`'s
// unconditional `action.go === "back"` branch (`app.ts`) pops off the
// stack regardless of route. This fake reproduces exactly that shape
// without reimplementing `lobby.ts`'s model.

import { describe, expect, it } from "vitest";
import { App, type OnlineLobbyScreen, type OnlinePorts } from "./app.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { APP_CONTENT } from "./test_support/fixtures.ts";
import type { ControllerInputEvent } from "@gc/input";

function clickWidget(app: App, id: string): void {
  const layout = menuLayout(app.stack.current());
  if (!layout) {
    throw new Error(`no menu layout on the current screen (looking for widget ${id})`);
  }
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  const [x, y] = viewport.toActual(app.transform, widget.rect.x + widget.rect.w / 2, widget.rect.y + widget.rect.h / 2);
  app.event({ kind: "click", x, y, button: 1 });
}

// A lobby screen with no session to preserve: its only reachable action is
// leaving, which -- mirroring `lobby.ts`'s real `back` handling -- reports
// itself to `App` as `{ go: "back" }`. See this file's header for why a
// fuller fake (or the real `OnlineLobby`) is not needed for this case.
function fakeEmptyLobbyScreen(onAction: (action: { readonly go: string }) => void): OnlineLobbyScreen {
  return {
    state: { model: {} },
    link: undefined,
    event(evt: ControllerInputEvent): void {
      if (evt.kind === "action" && evt.action === "back") {
        onAction({ go: "back" });
      }
    },
  };
}

function fakeOnlinePorts(): OnlinePorts {
  return {
    matchManifestTemplate: undefined,
    requestMatchSession: () => ({ ok: false, error: "not exercised by this case" }),
    newLobbyScreen: (onAction) => fakeEmptyLobbyScreen(onAction),
    newOnlineMatchScreen: () => {
      throw new Error("not exercised by this case");
    },
  };
}

describe("online lobby app routing", () => {
  it("is reachable from the title and returns to it", () => {
    const app = new App(APP_CONTENT, { online: fakeOnlinePorts() });
    expect(app.currentRoute()).toBe("title");

    clickWidget(app, "online_lobby");
    expect(app.currentRoute()).toBe("lobby");

    app.event({ kind: "key", key: "escape" });
    expect(app.currentRoute()).toBe("title");
  });
});
