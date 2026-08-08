// Ported from game/match_adapter.lua.
//
// The Lua original requires `game.screens.fake_match`, `game.screens.menu`,
// and `game.screens.real_match`. The first two are ported (`@gc/screens`'s
// `fakeMatch`/`Menu`); `game.screens.real_match` is not yet ported (see this
// package's porting report), so `real()` takes a `RealMatchFactory` port
// instead of importing it -- the same injection shape `bootstrap.ts` and
// `app.ts` thread through.

import { Menu, fakeMatch, type Viewport } from "@gc/screens";
import type { Screen } from "./screen_stack.ts";
import { fakeResult } from "./fake_result.ts";
import type { MatchContractContent } from "./content.ts";
import type { ProductMatchRequest, ProductMatchResult } from "./match_contract.ts";

export interface MatchAdapterCallbacks {
  readonly on_finished: (result: ProductMatchResult) => void;
  readonly on_cancelled: () => void;
}

export interface MatchAdapter {
  readonly kind: "fake" | "real";
  // Quoted: `new(...)` inside an interface body parses as a *construct*
  // signature (`new MatchAdapter(...)`), not a regular method named `new`.
  // Quoting the property name keeps this a plain method, matching the Lua
  // original's `match_adapter.new`/`.new(...)` call shape.
  "new"(request: ProductMatchRequest, callbacks: MatchAdapterCallbacks, viewport: Viewport): Screen;
}

/** `game.screens.real_match.new`, injected -- see this file's header. */
export type RealMatchFactory = (request: ProductMatchRequest, callbacks: MatchAdapterCallbacks) => Screen;

function fake(content: Pick<MatchContractContent, "players" | "teams">): MatchAdapter {
  return {
    kind: "fake",
    new(request, callbacks, viewport) {
      const result = fakeResult.forRequest(content, request);
      const menu = new Menu(
        fakeMatch,
        fakeMatch.newState(viewport, { request, result }),
        (action) => {
          if (action.go === "complete") {
            // `fakeMatch`'s action carries `@gc/screens`' own (narrower)
            // `ProductMatchResult` type, but never recomputes it -- it is
            // the exact `result` value closed over above, already this
            // package's fuller type. Using the closure value instead of
            // `action.result` keeps the callback's type honest without a
            // cast.
            callbacks.on_finished(result);
          } else if (action.go === "cancel") {
            callbacks.on_cancelled();
          }
        },
      );
      // `Menu.draw(backend: GraphicsBackend)` needs a `@gc/ui` backend this
      // milestone deliberately does not wire up (v2/README.md §1 -- "do not
      // build [the browser main loop]"). `Screen.draw` stays call-signature
      // compatible with the Lua original (`fun(self: Screen)`, no backend
      // argument) since nothing in this port's test coverage calls `draw`;
      // the cast documents that gap rather than hiding it.
      return menu as unknown as Screen;
    },
  };
}

function real(factory: RealMatchFactory): MatchAdapter {
  return {
    kind: "real",
    new(request, callbacks) {
      return factory(request, callbacks);
    },
  };
}

export const matchAdapter = { fake, real };
