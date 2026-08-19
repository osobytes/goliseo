// `bootstrap.new` takes a `RealMatchFactory` because `match_adapter.real()`
// needs one injected rather than wiring a concrete real-match screen
// directly -- see `match_adapter.ts`'s header for why.

import { App, type AppContent, type OnlinePorts } from "./app.ts";
import { matchAdapter, type RealMatchFactory } from "./match_adapter.ts";
import type { GameSettings, SettingsStorage } from "./settings.ts";

export interface BootstrapOptions {
  readonly applySettings?: (settings: GameSettings) => void;
  readonly requestQuit?: () => void;
  readonly settingsStorage?: SettingsStorage;
  /** `team_settings.ts` storage -- see `app.ts`'s `AppOptions.teamSettingsStorage`. */
  readonly teamSettingsStorage?: SettingsStorage;
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
    ...(opts.teamSettingsStorage !== undefined
      ? { teamSettingsStorage: opts.teamSettingsStorage }
      : {}),
    ...(opts.quickMatch !== undefined ? { quickMatch: opts.quickMatch } : {}),
    ...(opts.online !== undefined ? { online: opts.online } : {}),
  });
}

export const bootstrap = { new: newApp };
