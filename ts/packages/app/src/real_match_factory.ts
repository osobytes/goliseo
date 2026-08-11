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
// MATCH OBSERVATION -- previously a documented gap, now closed. This header
// used to record that `RealMatchScreenPort` narrowed `state` to
// `{time_left, score}` with `frameEvents` always `[]`, so every shot, save,
// possession and pass-completion figure resolved to zero while
// `ProductMatchResult` still came out looking complete. Goals were tracked;
// nothing else was.
//
// `RealMatchScreenPort.state` now also carries `roster` (id, team, is_keeper
// per player) and `owner` (the ball carrier), and `frameEvents` carries the
// ticks observed since the last call -- all of it already present on the real
// `RenderFrame`, so no Rust change was needed. `observerPort` below consumes
// them.
//
// Worth knowing why this survived so long: nothing tested it. The stats were
// optional fields on `TeamResultStats`, so zeros were indistinguishable from
// a match where nothing happened, and no test asserted otherwise. The
// coverage now lives in `match_observer.spec.ts`'s "real match screen port,
// widened" block, which drives a scripted match through the real screen, the
// real adapter and the real observer, and asserts non-zero stats on both
// sides plus an evidence-backed MVP.

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
import {
  matchObserver,
  type MatchObserver,
  type ObservedMatchEvent,
} from "./match_observer.ts";
import type { RealMatchFactory } from "./match_adapter.ts";
import type { ProductMatchRequest } from "./match_contract.ts";
import type { MatchContractContent } from "./content.ts";
import type { Screen } from "./screen_stack.ts";

/**
 * Feeds `matchObserver` the real roster, carrier and per-tick events that
 * `RealMatchScreenPort` now carries.
 *
 * This used to pass `{ players: [], events: [], score }`, which meant every
 * shot, save, possession and pass-completion figure resolved to zero while
 * looking like a complete result -- goals were tracked, nothing else was. The
 * port was widened for exactly this, and wiring it here is the last link:
 * `state.roster`/`state.owner` come off `SimHostPort.roster()` and the frame's
 * possession, and `frameEvents` accumulates inside the tick loop rather than
 * after it, because a catch-up render call steps several ticks and the frame
 * only ever reports the last one's events.
 */
const observerPort: MatchObserverPort<MatchObserver, RealMatchState, never> = {
  create: () => matchObserver.new({ players: [], events: [], score: { home: 0, away: 0 } }),
  observe(observer, state, dt, events) {
    matchObserver.observe(
      observer,
      {
        players: (state.roster ?? []).map((entry) => ({
          id: entry.id,
          team: entry.team,
          is_keeper: entry.is_keeper,
        })),
        ...(state.owner !== undefined ? { owner: state.owner } : {}),
        events: [],
        score: { home: state.score.home, away: state.score.away },
      },
      dt,
      // `RealMatchScreenPort.frameEvents` is `readonly unknown[]` by design:
      // `@gc/screens` cannot depend on `@gc/app`, so it cannot name
      // `ObservedMatchEvent` and restates the shape as `RealMatchEvent`
      // instead. The dependency runs the other way here, so this is the layer
      // that can narrow it.
      //
      // Forwarded unfiltered, and the cast is why: the wire carries about
      // twenty-five event kinds while `ObservedEventKind` names only the
      // subset the observer acts on. `match_observer.ts`'s own `observe`
      // iterates the whole raw list and switches on `kind`, ignoring the
      // rest, so handing it everything is what it actually receives.
      // Filtering first would be behaviourally identical today and would
      // quietly diverge the moment the observer starts counting what it was
      // given rather than only what it recognises.
      (events ?? []) as readonly ObservedMatchEvent[],
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
