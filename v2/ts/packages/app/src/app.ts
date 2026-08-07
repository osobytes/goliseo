// Ported from game/app.lua.
//
// The Lua original requires `game.online.match_manifest`/`match_session`
// (Rust-owned `gc-netcode`; v2/README.md §2.1) -- still true and still
// injected below, since neither has a wasm bridge this milestone (no
// `@gc/wasm` export builds a `SessionManifest` or a match-session request;
// see online_match_flow.spec.ts, this package's own spec, for the same
// gap from the test side). `game.screens.online_lobby`/`online_match`,
// though, ARE now real, ported screens (`@gc/screens`'s `OnlineLobby`/
// `OnlineMatch`) -- that half of this note was stale. They stay behind
// `OnlinePorts` regardless: this file is decoupled from how a lobby/match
// screen gets built (a test can supply lighter fakes than the real
// classes), matching `match_adapter.ts`'s injection pattern for the
// offline match screen.
//
// `start_online_match` used to be an unconditional stub -- it never even
// read the mounted lobby's coordinator state, on the theory that
// `OnlinePorts` had no way to expose it. That was the actual bug, not a
// structural blocker: `OnlinePorts.requestMatchSession`/`newOnlineMatchScreen`
// were already the right shape (mirroring `match_session.request`/
// `OnlineMatch.new`), and `OnlineLobbyScreen` below (added by this port)
// gives the one missing piece -- a read on `state.model.coordinator`/
// `link`, exactly what the Lua original's `---@cast lobby OnlineLobby`
// exposes. See online_match_flow.spec.ts (this package) for the case this
// was ported to unblock.
//
// `game.ui.viewport`/`game.input.controller` compose the same way
// `ui_bridge.ts`'s header explains; `controller` itself IS a declared
// dependency (`@gc/input`) and is used directly.

import { bindings, controller, type ControllerInputEvent } from "@gc/input";
import {
  Menu,
  credits,
  formation,
  help,
  pause,
  result,
  settings as settingsScreen,
  squad,
  tactic,
  title,
  type BuildInfo as ScreensBuildInfo,
  type FormationContentData,
  type SquadContentData,
  type TacticContentData,
} from "@gc/screens";
import { matchAdapter, type MatchAdapter, type MatchAdapterCallbacks } from "./match_adapter.ts";
import { ScreenStack, type Screen } from "./screen_stack.ts";
import { session, type GameSession } from "./session.ts";
import { settings as settingsModule, type GameSettings, type SettingsStorage } from "./settings.ts";
import { viewport, viewportMapper, type ViewportTransform } from "./ui_bridge.ts";
import type { MatchContractContent, TeamData } from "./content.ts";
import type { ProductMatchResult } from "./match_contract.ts";

export interface Viewport {
  readonly w: number;
  readonly h: number;
}

/** `game.online.coordinator`'s state as read by `App:start_online_match` -- `state.role`/`state.peer_id`/`state.manifest`. Kept structural (no `@gc/screens` import) rather than named after `@gc/screens`'s `CoordinatorState` so a test can supply a lighter fake than the real one. */
export interface OnlineLobbyCoordinatorState {
  readonly role: unknown;
  readonly peer_id: unknown;
  readonly manifest?: unknown;
}

/**
 * Minimal read surface `start_online_match` needs off the mounted lobby
 * screen -- mirrors the Lua original's `---@cast lobby OnlineLobby` before
 * reading `lobby.state.model.coordinator`/`lobby.link`. See this file's
 * header for why this exists (it did not, before).
 */
export interface OnlineLobbyScreen extends Screen<ControllerInputEvent, GameSettings> {
  readonly state: { readonly model: { readonly coordinator?: OnlineLobbyCoordinatorState } };
  readonly link: unknown;
}

/** `game.online.match_manifest`/`match_session` (Rust-owned, no wasm bridge), and the real (but injected) online lobby/match screens -- see this file's header. */
export interface OnlinePorts {
  readonly matchManifestTemplate: unknown;
  requestMatchSession(options: {
    readonly role: unknown;
    readonly peerId: unknown;
    readonly manifest: unknown;
    readonly freeze: unknown;
  }): { readonly ok: true; readonly value: unknown } | { readonly ok: false; readonly error: string };
  newLobbyScreen(onAction: (action: AppAction) => void, options: unknown): OnlineLobbyScreen;
  newOnlineMatchScreen(options: {
    readonly request: unknown;
    readonly coordinator: unknown;
    readonly link: unknown;
    readonly onAction: (action: AppAction) => void;
  }): OnlineMatchScreen;
}

export interface OnlineMatchScreen extends Screen<ControllerInputEvent, GameSettings> {
  focusLost(): void;
  controllerLost(): void;
}

export type AppAction = { readonly go: string } & Record<string, unknown>;

export interface AppContent {
  readonly matchContract: MatchContractContent;
  readonly homeTeam: TeamData;
  readonly squad: SquadContentData;
  readonly formation: FormationContentData;
  readonly tactic: TacticContentData;
  readonly buildInfo: ScreensBuildInfo;
}

export interface AppOptions {
  readonly actualW?: number;
  readonly actualH?: number;
  readonly settings?: GameSettings;
  readonly settingsStorage?: SettingsStorage;
  readonly matchAdapter?: MatchAdapter;
  readonly applySettings?: (settings: GameSettings) => void;
  readonly requestQuit?: () => void;
  /** Playtest convenience: boot straight into a match. */
  readonly quickMatch?: boolean;
  readonly online?: OnlinePorts;
}

function asMenu<State extends { readonly viewport: Viewport }, Action>(
  menu: Menu<State, Action>,
): Screen<ControllerInputEvent, GameSettings> {
  // `Menu.draw` needs a `@gc/ui` `GraphicsBackend` this milestone
  // deliberately does not wire up (v2/README.md §1). See match_adapter.ts's
  // header for the identical cast and why it is safe here: nothing in this
  // port's test coverage calls `draw`.
  return menu as unknown as Screen<ControllerInputEvent, GameSettings>;
}

export class App {
  readonly stack = new ScreenStack<ControllerInputEvent, GameSettings>();
  session: GameSession;
  settings: GameSettings;
  readonly settingsStorage: SettingsStorage | undefined;
  readonly viewport: Viewport = { w: 960, h: 540 };
  transform: ViewportTransform;
  adapter: MatchAdapter;
  routes: string[] = [];
  quitRequested = false;
  readonly applySettingsCallback: ((settings: GameSettings) => void) | undefined;
  readonly requestQuit: (() => void) | undefined;
  /** Why the last online session ended, if it did not complete. */
  onlineError: string | undefined;

  private readonly content: AppContent;
  private readonly online: OnlinePorts | undefined;

  constructor(content: AppContent, opts: AppOptions = {}) {
    this.content = content;
    this.online = opts.online;
    this.session = session.new(content.homeTeam);
    this.settingsStorage = opts.settingsStorage;
    this.settings = opts.settings ? settingsModule.validate(opts.settings) : settingsModule.load(opts.settingsStorage);
    this.transform = viewport.create(opts.actualW ?? 960, opts.actualH ?? 540);
    this.adapter = opts.matchAdapter ?? matchAdapter.fake(content.matchContract);
    this.applySettingsCallback = opts.applySettings;
    this.requestQuit = opts.requestQuit;

    if (opts.quickMatch) {
      this.startMatch();
    } else {
      this.showTitle();
    }
  }

  private replaceRoute(route: string, screen: Screen<ControllerInputEvent, GameSettings>): void {
    this.stack.clear();
    this.stack.push(screen);
    this.routes = [route];
  }

  private pushRoute(route: string, screen: Screen<ControllerInputEvent, GameSettings>): void {
    this.stack.push(screen);
    this.routes.push(route);
  }

  private popRoute(): void {
    if (this.routes.length > 1) {
      this.stack.pop();
      this.routes.pop();
    }
  }

  currentRoute(): string {
    const route = this.routes[this.routes.length - 1];
    if (route === undefined) {
      throw new Error("app has no active route");
    }
    return route;
  }

  private onAction(): (action: AppAction) => void {
    return (action) => this.handleAction(action);
  }

  showTitle(): void {
    const menu = new Menu(title, title.newState(this.viewport), this.onAction());
    this.replaceRoute("title", asMenu(menu));
  }

  showSquad(): void {
    this.session.setupStep = "squad";
    const menu = new Menu(
      squad,
      squad.newState(this.viewport, this.content.squad, { starterIds: this.session.starterIds }),
      this.onAction(),
    );
    this.replaceRoute("squad", asMenu(menu));
  }

  showFormation(): void {
    this.session.setupStep = "formation";
    const menu = new Menu(
      formation,
      formation.newState(this.viewport, this.content.formation, {
        selected: this.session.formationId,
        starterIds: this.session.starterIds,
      }),
      this.onAction(),
    );
    this.replaceRoute("formation", asMenu(menu));
  }

  showTactic(): void {
    this.session.setupStep = "tactic";
    const menu = new Menu(
      tactic,
      tactic.newState(this.viewport, this.content.tactic, {
        selected: this.session.tacticId,
        formationId: this.session.formationId,
      }),
      this.onAction(),
    );
    this.replaceRoute("tactic", asMenu(menu));
  }

  showResult(): void {
    const lastResult = this.session.lastResult;
    if (!lastResult) {
      throw new Error("result route needs a match result");
    }
    const menu = new Menu(
      result,
      result.newState(this.viewport, { players: this.content.matchContract.players }, { result: lastResult }),
      this.onAction(),
    );
    this.replaceRoute("result", asMenu(menu));
  }

  // The online result screen is the same product screen, on its own route so
  // the offline rematch (which replays the local session) cannot be reached
  // from a session that no longer exists.
  showOnlineResult(matchResult: ProductMatchResult): void {
    this.onlineError = undefined;
    const menu = new Menu(
      result,
      result.newState(this.viewport, { players: this.content.matchContract.players }, { result: matchResult }),
      this.onAction(),
    );
    this.replaceRoute("online_result", asMenu(menu));
  }

  startMatch(): void {
    const requested = session.buildRequest(this.content.matchContract, this.session, this.session.matchNumber + 1);
    if (!requested.ok) {
      throw new Error(requested.error);
    }
    const request = requested.value;
    const callbacks: MatchAdapterCallbacks = {
      on_finished: (matchResult) => {
        session.recordResult(this.session, matchResult);
        this.showResult();
      },
      on_cancelled: () => {
        this.showTactic();
      },
    };
    const screen = this.adapter.new(request, callbacks, this.viewport);
    if (screen.applySettings) {
      screen.applySettings(this.settings);
    }
    this.replaceRoute("match", screen);
  }

  // The developer online route -- see this file's header for why it is
  // injected rather than imported.
  showLobby(options?: { readonly modelOptions?: Record<string, unknown> }): void {
    if (!this.online) {
      throw new Error("no online ports were injected into this App");
    }
    const modelOptions = options?.modelOptions ?? {};
    const resolved = { ...modelOptions, template: modelOptions.template ?? this.online.matchManifestTemplate };
    const screen = this.online.newLobbyScreen(this.onAction(), resolved);
    this.pushRoute("lobby", screen);
  }

  // Route the lobby's synchronized start into the real online match. The
  // lobby keeps its link: the match borrows the same star, because the
  // session's control channel and the match's input channel are the same
  // transport (game/app.lua's own comment on this method).
  startOnlineMatch(freeze: unknown): void {
    if (!this.online) {
      throw new Error("no online ports were injected into this App");
    }
    const lobby = this.stack.current() as OnlineLobbyScreen;
    const state = lobby.state.model.coordinator;
    const link = lobby.link;
    if (!state || state.manifest === undefined || link === undefined) {
      this.onlineError = "the lobby has no frozen session to play";
      return;
    }
    const requested = this.online.requestMatchSession({
      role: state.role,
      peerId: state.peer_id,
      manifest: state.manifest,
      freeze,
    });
    if (!requested.ok) {
      this.onlineError = requested.error;
      return;
    }
    this.onlineError = undefined;
    const screen = this.online.newOnlineMatchScreen({
      request: requested.value,
      coordinator: state,
      link,
      onAction: this.onAction(),
    });
    if (screen.applySettings) {
      screen.applySettings(this.settings);
    }
    this.pushRoute("online_match", screen);
  }

  showPause(): void {
    const menu = new Menu(pause, pause.newState(this.viewport), this.onAction());
    this.pushRoute("pause", asMenu(menu));
  }

  private setSettings(value: GameSettings, persist?: boolean): void {
    this.settings = settingsModule.validate(value);
    if (this.applySettingsCallback) {
      this.applySettingsCallback(this.settings);
    }
    for (const screen of this.stack.screens) {
      if (screen.applySettings) {
        screen.applySettings(this.settings);
      }
    }
    if (persist) {
      settingsModule.save(this.settings, this.settingsStorage);
    }
  }

  handleAction(action: AppAction): void {
    const route = this.currentRoute();
    if (action.go === "quit") {
      this.quitRequested = true;
      if (this.requestQuit) {
        this.requestQuit();
      }
    } else if (route === "title" && action.go === "play") {
      session.setCombatEnabled(this.session, false);
      this.showSquad();
    } else if (route === "title" && action.go === "combat_prototype") {
      session.setCombatEnabled(this.session, true);
      this.showSquad();
    } else if (route === "title" && action.go === "online_lobby") {
      this.showLobby();
    } else if (route === "lobby" && action.go === "online_match") {
      if (action.freeze === undefined) {
        throw new Error("an online start needs its freeze");
      }
      this.startOnlineMatch(action.freeze);
    } else if (route === "online_match" && action.go === "online_result") {
      if (action.result === undefined) {
        throw new Error("an online result needs its record");
      }
      this.showOnlineResult(action.result as ProductMatchResult);
    } else if (route === "online_match" && action.go === "online_ended") {
      const terminal = action.terminal as { readonly reason?: string } | undefined;
      this.onlineError = (action.detail as string | undefined) ?? terminal?.reason ?? "the online session ended";
      this.showTitle();
    } else if (
      route === "online_result" &&
      (action.go === "rematch" || action.go === "change_lineup" || action.go === "change_plan")
    ) {
      this.showTitle();
      this.showLobby();
    } else if (route === "title" && action.go === "help") {
      const menu = new Menu(help, help.newState(this.viewport, bindings.reference("match")), this.onAction());
      this.pushRoute("help", asMenu(menu));
    } else if (route === "title" && action.go === "credits") {
      const menu = new Menu(credits, credits.newState(this.viewport, this.content.buildInfo), this.onAction());
      this.pushRoute("credits", asMenu(menu));
    } else if (action.go === "settings") {
      const menu = new Menu(
        settingsScreen,
        settingsScreen.newState(this.viewport, settingsModule, { settings: this.settings }),
        this.onAction(),
      );
      this.pushRoute("settings", asMenu(menu));
    } else if (action.go === "settings_changed") {
      this.setSettings(action.settings as GameSettings);
    } else if (action.go === "back") {
      if (action.settings !== undefined) {
        this.setSettings(action.settings as GameSettings, true);
      }
      this.popRoute();
    } else if (route === "squad" && action.go === "formation") {
      const set = session.setStarters(
        this.content.matchContract,
        this.session,
        this.content.homeTeam,
        action.starterIds as readonly string[],
      );
      if (!set.ok) {
        throw new Error(set.error);
      }
      this.showFormation();
    } else if (route === "squad" && action.go === "title") {
      this.showTitle();
    } else if (route === "formation" && action.go === "squad") {
      if (action.formationId !== undefined) {
        session.setFormation(this.session, action.formationId as string);
      }
      this.showSquad();
    } else if (route === "formation" && action.go === "tactic") {
      session.setFormation(this.session, action.formationId as string);
      this.showTactic();
    } else if (route === "tactic" && action.go === "formation") {
      if (action.tacticId !== undefined) {
        session.setTactic(this.session, action.tacticId as string);
      }
      this.showFormation();
    } else if (route === "tactic" && action.go === "match") {
      session.setTactic(this.session, action.tacticId as string);
      this.startMatch();
    } else if (route === "result" && action.go === "rematch") {
      this.startMatch();
    } else if (route === "result" && action.go === "change_plan") {
      this.showFormation();
    } else if (route === "result" && action.go === "change_lineup") {
      this.showSquad();
    } else if (action.go === "main_menu") {
      this.showTitle();
    } else if (route === "pause" && action.go === "resume") {
      this.popRoute();
    } else if (route === "pause" && action.go === "controls") {
      const menu = new Menu(help, help.newState(this.viewport, bindings.reference("match")), this.onAction());
      this.pushRoute("help", asMenu(menu));
    } else if (route === "pause" && action.go === "restart") {
      this.startMatch();
    }
  }

  resize(width: number, height: number): void {
    this.transform = viewport.create(width, height);
  }

  pauseMatch(): void {
    if (this.currentRoute() === "match") {
      this.showPause();
    }
  }

  private onlineMatch(): OnlineMatchScreen | undefined {
    if (this.currentRoute() !== "online_match") {
      return undefined;
    }
    return this.stack.current() as OnlineMatchScreen;
  }

  // Focus loss pauses an offline match and deliberately does not pause an
  // online one -- see the Lua original's comment (game/app.lua).
  focus(focused: boolean): void {
    if (focused) {
      return;
    }
    const online = this.onlineMatch();
    if (online) {
      online.focusLost();
      return;
    }
    this.pauseMatch();
  }

  controllerRemoved(): void {
    const online = this.onlineMatch();
    if (online) {
      online.controllerLost();
      return;
    }
    this.pauseMatch();
  }

  event(evt: ControllerInputEvent): void {
    const route = this.currentRoute();
    const normalized = controller.normalize(evt, this.transform, viewportMapper);
    if (normalized === null) {
      return;
    }
    if (normalized.kind === "action" && normalized.action === "toggle_mute") {
      const value = settingsModule.validate(this.settings);
      this.setSettings({ ...value, muted: !value.muted }, true);
      return;
    } else if (normalized.kind === "action" && normalized.action === "toggle_fullscreen") {
      const value = settingsModule.validate(this.settings);
      this.setSettings({ ...value, fullscreen: !value.fullscreen }, true);
      return;
    }
    if (
      normalized.kind === "action" &&
      route === "match" &&
      (normalized.action === "pause" || normalized.action === "back")
    ) {
      this.showPause();
      return;
    }
    this.stack.event(normalized);
  }

  update(dt: number): void {
    this.stack.update(dt);
  }

  draw(): void {
    this.stack.draw();
  }
}
