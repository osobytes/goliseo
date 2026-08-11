// `real()` takes a `RealMatchFactory` port instead of importing a concrete
// real-match screen directly -- the same injection shape `bootstrap.ts` and
// `app.ts` thread through -- because this package cannot depend on the
// concrete real-match implementation (see package boundaries); `fakeMatch`/
// `Menu` are imported directly from `@gc/screens` since no such constraint
// applies to them.

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
  // Quoting the property name keeps this a plain method, matching this
  // codebase's `.new(...)` factory-method convention.
  "new"(request: ProductMatchRequest, callbacks: MatchAdapterCallbacks, viewport: Viewport): Screen;
}

/** A concrete real-match screen constructor, injected -- see this file's header. */
export type RealMatchFactory = (
  request: ProductMatchRequest,
  callbacks: MatchAdapterCallbacks,
) => Screen;

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
      // milestone deliberately does not wire up ("do not
      // build [the browser main loop]"). `Screen.draw` takes no backend
      // argument, and nothing in this package's test coverage calls `draw`;
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
