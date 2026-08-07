// Wires `@gc/screens`'s real match screen (`MatchScreen` + `real_match.ts`'s
// `RealMatchScreen`/`MatchScreenAsRealMatchScreen`) into a `RealMatchFactory`
// (`match_adapter.ts`'s injected port; see that file's header for why
// `match_adapter.real()` takes one instead of importing
// `game.screens.real_match` directly). This is the "wire the real thing"
// this batch's task brief asked for: `bootstrap.ts`/`app.ts` no longer need
// a throwing placeholder factory once a caller supplies real collaborators.
//
// `createHost`/`renderer` are injected rather than built in here (unlike an
// earlier version of this file, which hard-wired `browser_sim_host.ts` and
// `@gc/render`'s `SceneRoot` directly) specifically so this factory is
// testable with fakes -- see `bootstrap.spec.ts`'s "real match adapter"
// cases, which construct a real `RealMatchScreen` through this same function
// against a hand-written `SimHostPort`, the same way
// `@gc/screens`'s `match_screen.spec.ts` tests `MatchScreen` itself.
// `browser_main.ts` supplies the real, wasm/three.js-backed versions.
//
// KNOWN, DOCUMENTED GAP -- match observation. `real_match.ts`'s
// `RealMatchScreenPort<TState, TStep>` (which `MatchScreenAsRealMatchScreen`
// implements) deliberately narrows `state` to `RealMatchState`
// (`{time_left, score}`) and this milestone's `frameEvents` is always `[]`
// (`match.ts`'s own doc: "Not yet wired"). `match_observer.ts`'s real
// `ObservedMatchState`, by contrast, needs the full player roster, live
// `owner`, and a real per-tick event stream to produce meaningful
// shots/possession/pass-completion stats. None of that is reachable through
// `RealMatchScreenPort`'s narrow contract as it stands today -- extending it
// is `@gc/screens`'s call, not this batch's (that package is out of this
// batch's file ownership). So `observerPort` below adapts `matchObserver`
// faithfully to the seam that DOES exist: it tracks score transitions (goals)
// correctly, every possession/shot/save/pass stat resolves to zero because
// nothing here can observe them yet. `ProductMatchResult` still comes out
// correct and complete -- `home_stats`/`away_stats` are optional fields
// (`match_contract.ts`'s `TeamResultStats`) precisely for cases like this.
// Not silently patched over -- flagged here and in this port's report.

import {
  MatchScreen,
  MatchScreenAsRealMatchScreen,
  RealMatchScreen,
  type MatchContractPort,
  type MatchObserverPort,
  type RealMatchState,
  type RenderPort,
  type SimHostFactory,
} from "@gc/screens";
import type { GamepadState, KeyboardState } from "@gc/input";
import { matchContract } from "./match_contract.ts";
import { matchObserver, type MatchObserver } from "./match_observer.ts";
import type { RealMatchFactory } from "./match_adapter.ts";
import type { ProductMatchRequest } from "./match_contract.ts";
import type { MatchContractContent } from "./content.ts";
import type { Screen } from "./screen_stack.ts";

/** See this file's header -- score-only observation, honestly scoped. */
const observerPort: MatchObserverPort<MatchObserver, RealMatchState, never> = {
  create: () => matchObserver.new({ players: [], events: [], score: { home: 0, away: 0 } }),
  observe(observer, state, dt, events) {
    matchObserver.observe(
      observer,
      { players: [], events: [], score: { home: state.score.home, away: state.score.away } },
      dt,
      // `events` is `RealMatchScreenPort.frameEvents`, always `[]` this
      // milestone (see this file's header) -- an empty array satisfies any
      // element type, so no unsafe read happens here.
      (events ?? []) as never[],
    );
  },
  // `TStep` is `never`: `MatchScreenAsRealMatchScreen.rollbackLab` is always
  // `false` (this milestone's `MatchScreen` has no rollback lab), so
  // `RealMatchScreen` never calls this.
  observeConfirmed: () => false,
  finish: (observer) => matchObserver.finish(observer),
};

function contractPort(content: Pick<MatchContractContent, "teams">): MatchContractPort {
  return {
    newResult: (opts) => matchContract.newResult({ teams: content.teams }, opts),
  };
}

export interface RealMatchFactoryDeps {
  readonly content: Pick<MatchContractContent, "teams">;
  /** Builds a fresh {@link SimHostFactory} closed over one match's request -- the real caller wraps `browser_sim_host.ts`'s `createBrowserSimHost`; a test wraps a hand-written fake. */
  readonly createHost: (request: ProductMatchRequest) => SimHostFactory;
  readonly renderer: RenderPort;
  readonly keyboard: KeyboardState;
  readonly gamepad?: GamepadState;
}

/**
 * Builds a {@link RealMatchFactory} closed over injected collaborators. See
 * this file's header for the match-observation gap, why `createHost`/
 * `renderer` are injected, and why that is safe.
 */
export function createRealMatchFactory(deps: RealMatchFactoryDeps): RealMatchFactory {
  return (request, callbacks) => {
    const matchScreen = new MatchScreen(
      {
        createHost: deps.createHost(request),
        renderer: deps.renderer,
        keyboard: deps.keyboard,
        ...(deps.gamepad !== undefined ? { gamepad: deps.gamepad } : {}),
      },
      // `request.combat_enabled` (`match_contract.ts`'s `ProductMatchRequest`,
      // the "explicit post-showcase request" opt-in) now reaches
      // `MatchScreenOptions.combat_enabled` -- previously dropped here
      // silently. See `match.ts`'s own doc on that option for what this
      // does and does not prove for the BASE (non-rollback) game loop this
      // factory always builds: `deps.createHost` (the real caller wraps
      // `browser_sim_host.ts`, out of this batch's file ownership) is the
      // one that must actually construct its `Session` with a matching
      // `combat_enabled`; this factory has no way to verify it did.
      { profile: "product", combat_enabled: request.combat_enabled },
    );
    const realMatchScreenPort = new MatchScreenAsRealMatchScreen(matchScreen);
    const realMatchScreen = new RealMatchScreen(
      realMatchScreenPort,
      // `request` (`match_contract.ts`'s `ProductMatchRequest`) is
      // structurally identical, field for field, to `real_match.ts`'s own
      // `RealMatchRequest` -- see this file's header note on why that type
      // is not imported by name (`@gc/screens`'s index.ts does not
      // re-export it; not this batch's file to edit).
      request,
      { onFinished: callbacks.on_finished },
      contractPort(deps.content),
      observerPort,
    );
    // Same "cast to the (unparameterized) Screen this package's factories
    // return" pattern `match_adapter.ts`'s own `fake()` uses for `Menu` --
    // see that file's header.
    return realMatchScreen as unknown as Screen;
  };
}
