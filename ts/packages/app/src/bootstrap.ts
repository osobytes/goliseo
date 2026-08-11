// Ported from game/bootstrap.lua.
//
// The Lua original wires `match_adapter.real()` unconditionally. This
// port's `match_adapter.real()` needs an injected `RealMatchFactory`
// (`game.screens.real_match.new`, not yet ported to `@gc/screens` -- this
// package's porting report; `match_adapter.ts`'s header), so `bootstrap.new`
// takes one too.

import { App, type AppContent, type OnlinePorts } from "./app.ts";
import { matchAdapter, type RealMatchFactory } from "./match_adapter.ts";
import type { GameSettings, SettingsStorage } from "./settings.ts";

export interface BootstrapOptions {
  readonly applySettings?: (settings: GameSettings) => void;
  readonly requestQuit?: () => void;
  readonly settingsStorage?: SettingsStorage;
  /** playtest: boot straight into a match. */
  readonly quickMatch?: boolean;
  readonly online?: OnlinePorts;
}

function newApp(
  content: AppContent,
  realMatchFactory: RealMatchFactory,
  width: number,
  height: number,
  opts: BootstrapOptions = {},
): App {
  return new App(content, {
    actualW: width,
    actualH: height,
    matchAdapter: matchAdapter.real(realMatchFactory),
    ...(opts.applySettings !== undefined ? { applySettings: opts.applySettings } : {}),
    ...(opts.requestQuit !== undefined ? { requestQuit: opts.requestQuit } : {}),
    ...(opts.settingsStorage !== undefined ? { settingsStorage: opts.settingsStorage } : {}),
    ...(opts.quickMatch !== undefined ? { quickMatch: opts.quickMatch } : {}),
    ...(opts.online !== undefined ? { online: opts.online } : {}),
  });
}

export const bootstrap = { new: newApp };
