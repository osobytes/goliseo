// Match manifest/session types (Rust-owned `gc-netcode`; ARCHITECTURE.md
// §1.1) are injected below since neither has a wasm bridge this milestone
// (no `@gc/wasm` export builds a `SessionManifest` or a match-session
// request; see online_match_flow.spec.ts, this package's own spec, for the
// same gap from the test side). The online lobby/match screens, though,
// ARE real (`@gc/screens`'s `OnlineLobby`/`OnlineMatch`). They stay behind
// `OnlinePorts` regardless: this file is decoupled from how a lobby/match
// screen gets built (a test can supply lighter fakes than the real
// classes), matching `match_adapter.ts`'s injection pattern for the
// offline match screen.
//
// `startOnlineMatch` used to be an unconditional stub -- it never even read
// the mounted lobby's coordinator state, on the theory that `OnlinePorts`
// had no way to expose it. That was the actual bug, not a structural
// blocker: `OnlinePorts.requestMatchSession`/`newOnlineMatchScreen` were
// already the right shape, and `OnlineLobbyScreen` below gives the one
// missing piece -- a read on `state.model.coordinator`/`link`. See
// online_match_flow.spec.ts (this package) for the case this was added to
// unblock.
//
// `viewport`/`controller` compose the same way `ui_bridge.ts`'s header
// explains; `controller` itself IS a declared dependency (`@gc/input`) and
// is used directly.

import { bindings, controller, type ControllerInputEvent } from "@gc/input";
import {
  Menu,
  credits,
  help,
  multiplayer,
  pause,
  result,
  sessionEnded,
  settings as settingsScreen,
  teamSheet,
  title,
  LOBBY_DEFAULT_MODE,
  LOBBY_MODES,
  LOBBY_TERMINAL_TEXT,
  type BuildInfo as ScreensBuildInfo,
  type SessionMatchMode,
  type TeamSheetContentData,
} from "@gc/screens";
import { matchAdapter, type MatchAdapter, type MatchAdapterCallbacks } from "./match_adapter.ts";
import { ScreenStack, type Screen } from "./screen_stack.ts";
import { session, type GameSession } from "./session.ts";
import { settings as settingsModule, type GameSettings, type SettingsStorage } from "./settings.ts";
import { teamSettings } from "./team_settings.ts";
import { viewport, viewportMapper, type ViewportTransform } from "./ui_bridge.ts";
import type { MatchContractContent, TeamData } from "./content.ts";
import type { ProductMatchResult } from "./match_contract.ts";

/** Only a value `LOBBY_MODES` actually carries survives -- a stored id from
 * an older build (or hand-edited storage) falls back to the protocol's own
 * default, the same "content may drift" discipline `team_settings.ts` uses
 * for formations/tactics (AGENTS.md §8). Match modes are a fixed protocol
 * enum rather than Rust-owned content, so this check lives here rather than
 * in `team_settings.ts`, which deliberately keeps no `@gc/screens` dependency. */
function validatedOnlineMode(stored: string): SessionMatchMode {
  return (LOBBY_MODES as readonly string[]).includes(stored)
    ? (stored as SessionMatchMode)
    : LOBBY_DEFAULT_MODE;
}

export interface Viewport {
  readonly w: number;
  readonly h: number;
}

/** The online coordinator's state as read by `App.startOnlineMatch` -- `state.role`/`state.peer_id`/`state.manifest`. Kept structural (no `@gc/screens` import) rather than named after `@gc/screens`'s `CoordinatorState` so a test can supply a lighter fake than the real one. */
export interface OnlineLobbyCoordinatorState {
  readonly role: unknown;
  readonly peer_id: unknown;
  readonly manifest?: unknown;
}

/**
 * Minimal read surface `startOnlineMatch` needs off the mounted lobby
 * screen: `lobby.state.model.coordinator`/`lobby.link`. See this file's
 * header for why this exists (it did not, before).
 */
export interface OnlineLobbyScreen extends Screen<ControllerInputEvent, GameSettings> {
  readonly state: {
    readonly model: {
      readonly coordinator?: OnlineLobbyCoordinatorState;
      /** `lobby_model.ts`'s own `LobbyModel.bot_fill` -- read on leaving a
       * hosted lobby so the choice survives to the next one
       * (`handleAction`'s `main_menu` branch). */
      readonly bot_fill?: boolean;
    };
  };
  readonly link: unknown;
}

/** Match manifest/session types (Rust-owned, no wasm bridge), and the real (but injected) online lobby/match screens -- see this file's header. */
export interface OnlinePorts {
  readonly matchManifestTemplate: unknown;
  requestMatchSession(options: {
    readonly role: unknown;
    readonly peerId: unknown;
    readonly manifest: unknown;
    readonly freeze: unknown;
  }):
    { readonly ok: true; readonly value: unknown } | { readonly ok: false; readonly error: string };
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
  readonly teamSheet: TeamSheetContentData;
  readonly buildInfo: ScreensBuildInfo;
}

export interface AppOptions {
  readonly actualW?: number;
  readonly actualH?: number;
  readonly settings?: GameSettings;
  readonly settingsStorage?: SettingsStorage;
  /** `team_settings.ts` storage for the team sheet + last online lobby
   * choices -- a separate port from `settingsStorage` (a separate storage
   * key at the browser edge, `browser_main.ts`), mirroring how the two
   * modules are separate files with a shared discipline rather than one. */
  readonly teamSettingsStorage?: SettingsStorage;
  readonly matchAdapter?: MatchAdapter;
  readonly applySettings?: (settings: GameSettings) => void;
  readonly requestQuit?: () => void;
  /** Playtest convenience: boot straight into a match. */
  readonly quickMatch?: boolean;
  readonly online?: OnlinePorts;
  /**
   * A one-click join link's room code (#598), already parsed and validated
   * at boot (`browser_main.ts`'s `roomCodeFromSearch`, against the SAME
   * alphabet/length `@gc/online`'s `room_signaling.ts` exports -- never
   * re-derived). When present, boot routes directly into the lobby as a
   * guest with this code pre-filled and auto-submitted
   * (`showLobby`'s own `presetRoomCode`), instead of the title screen --
   * see the constructor below for the one place this is read.
   */
  readonly presetRoomCode?: string;
}

function asMenu<State extends { readonly viewport: Viewport }, Action>(
  menu: Menu<State, Action>,
): Screen<ControllerInputEvent, GameSettings> {
  // `Menu.draw` needs a `@gc/ui` `GraphicsBackend` this milestone
  // deliberately does not wire up. See match_adapter.ts's
  // header for the identical cast and why it is safe here: nothing in this
  // package's test coverage calls `draw`.
  return menu as unknown as Screen<ControllerInputEvent, GameSettings>;
}

export class App {
  readonly stack = new ScreenStack<ControllerInputEvent, GameSettings>();
  session: GameSession;
  settings: GameSettings;
  readonly settingsStorage: SettingsStorage | undefined;
  readonly teamSettingsStorage: SettingsStorage | undefined;
  /** Seeds `showMultiplayer`'s mode picker and, on a host commit, is
   * re-saved (`team_settings.ts`'s "last online mode"). */
  lastOnlineMode: SessionMatchMode;
  /** Seeds a hosted lobby's `bot_fill` and, on leaving one hosted, is
   * re-saved (`team_settings.ts`'s "last bot-fill choice"). */
  lastBotFill: boolean;
  /** The `intent` this app entered the CURRENT lobby route with, if any
   * (`showLobby`'s own `intent` option -- #597's room-flow role, not
   * necessarily the coordinator's eventual resolved role) --
   * `bot_fill` is host-only (`lobby_model.ts`'s own rule), so only a host
   * intent's departure re-saves it; a guest's `bot_fill` stays permanently
   * `false` and must never overwrite a real preference. Cleared once the
   * lobby route is left. */
  private currentLobbyRole: "host" | "guest" | undefined;
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
    this.teamSettingsStorage = opts.teamSettingsStorage;
    const storedPreferences = teamSettings.load(opts.teamSettingsStorage);
    const preferences = teamSettings.validateAgainstContent(
      content.matchContract,
      content.homeTeam,
      storedPreferences,
    );
    this.session = session.new(content.homeTeam, preferences);
    this.lastOnlineMode = validatedOnlineMode(storedPreferences.lastOnlineMode);
    this.lastBotFill = storedPreferences.lastBotFill;
    this.settingsStorage = opts.settingsStorage;
    this.settings = opts.settings
      ? settingsModule.validate(opts.settings)
      : settingsModule.load(opts.settingsStorage);
    this.transform = viewport.create(opts.actualW ?? 960, opts.actualH ?? 540);
    this.adapter = opts.matchAdapter ?? matchAdapter.fake(content.matchContract);
    this.applySettingsCallback = opts.applySettings;
    this.requestQuit = opts.requestQuit;

    if (opts.quickMatch) {
      this.startMatch();
    } else if (opts.presetRoomCode !== undefined && this.online) {
      // #598: a join link lands here with a room code already validated at
      // the boot boundary -- straight into the lobby as a guest, code
      // pre-filled and auto-submitted, never the title screen. `this.online`
      // is checked defensively (`showLobby` throws without it) rather than
      // asserted: a preset code with no online ports injected falls back to
      // the title screen instead of crashing boot.
      this.showLobby({ intent: "guest", presetRoomCode: opts.presetRoomCode });
    } else {
      this.showTitle();
    }
  }

  /** The impure write half of `team_settings.ts`'s discipline -- the pure
   * screens never call this (AGENTS.md §9); every commit point below does. */
  private saveTeamPreferences(): void {
    teamSettings.save(
      {
        starterIds: this.session.starterIds,
        formationId: this.session.formationId,
        tacticId: this.session.tacticId,
        combatEnabled: this.session.combatEnabled,
        lastOnlineMode: this.lastOnlineMode,
        lastBotFill: this.lastBotFill,
      },
      this.teamSettingsStorage,
    );
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

  // The whole pre-match decision. It replaced three routes -- squad,
  // formation, tactic -- that were three views of it.
  showTeamSheet(): void {
    const menu = new Menu(
      teamSheet,
      teamSheet.newState(this.viewport, this.content.teamSheet, {
        starterIds: this.session.starterIds,
        formationId: this.session.formationId,
        tacticId: this.session.tacticId,
        combatEnabled: this.session.combatEnabled,
      }),
      this.onAction(),
    );
    this.replaceRoute("team_sheet", asMenu(menu));
  }

  showMultiplayer(): void {
    const menu = new Menu(
      multiplayer,
      multiplayer.newState(this.viewport, { mode: this.lastOnlineMode }),
      this.onAction(),
    );
    this.replaceRoute("multiplayer", asMenu(menu));
  }

  /**
   * One result route for both contexts. It used to be two -- `result` and
   * `online_result` -- purely so the offline rematch, which replays a local
   * session that an online match does not have, was unreachable from an
   * online one. The screen takes a flag for that now.
   */
  showResult(matchResult?: ProductMatchResult): void {
    const online = matchResult !== undefined;
    const shown = matchResult ?? this.session.lastResult;
    if (!shown) {
      throw new Error("result route needs a match result");
    }
    if (online) {
      this.onlineError = undefined;
    }
    const menu = new Menu(
      result,
      result.newState(
        this.viewport,
        { players: this.content.matchContract.players },
        { result: shown, online },
      ),
      this.onAction(),
    );
    this.replaceRoute("result", asMenu(menu));
  }

  /**
   * Where a dead online session goes. The coordinator has always emitted a
   * typed reason; until now it was stored on `onlineError` and rendered
   * nowhere, and the player was dropped at the title screen without being
   * told anything.
   */
  showSessionEnded(reason: string, detail?: string): void {
    // `reason` arrives as a plain string off an `AppAction`, so the lookup is
    // widened rather than asserted: a reason the model gains without a thought
    // for this screen must still render, and the screen has its own fallback
    // for exactly that case.
    const table: Readonly<Record<string, string | undefined>> = LOBBY_TERMINAL_TEXT;
    const text = table[reason];
    this.onlineError = detail ?? text ?? reason;
    const menu = new Menu(
      sessionEnded,
      sessionEnded.newState(this.viewport, {
        reason,
        ...(text !== undefined ? { text } : {}),
        ...(detail !== undefined ? { detail } : {}),
      }),
      this.onAction(),
    );
    this.replaceRoute("session_ended", asMenu(menu));
  }

  startMatch(): void {
    const requested = session.buildRequest(
      this.content.matchContract,
      this.session,
      this.session.matchNumber + 1,
    );
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
        this.showTeamSheet();
      },
    };
    const screen = this.adapter.new(request, callbacks, this.viewport);
    if (screen.applySettings) {
      screen.applySettings(this.settings);
    }
    this.replaceRoute("match", screen);
  }

  // The online route -- see this file's header for why the lobby screen is
  // injected rather than imported. `intent`/`mode` are the multiplayer
  // front door's decision (#597: a room-flow intent, not a preset manual
  // role -- see `multiplayer.ts`'s `MultiplayerAction` doc), forwarded so
  // the player does not choose Host/Join twice.
  showLobby(options?: {
    readonly modelOptions?: Record<string, unknown>;
    readonly intent?: "host" | "guest";
    readonly mode?: string;
    /** #598: a join link's room code, pre-filled and auto-submitted the
     * moment a `"guest"` intent's composer is revealed. See
     * `AppOptions.presetRoomCode`'s own doc for where this originates. */
    readonly presetRoomCode?: string;
  }): void {
    if (!this.online) {
      throw new Error("no online ports were injected into this App");
    }
    this.currentLobbyRole = options?.intent;
    const modelOptions = options?.modelOptions ?? {};
    const resolved = {
      ...modelOptions,
      template: modelOptions.template ?? this.online.matchManifestTemplate,
      ...(options?.intent !== undefined ? { intent: options.intent } : {}),
      ...(options?.mode !== undefined ? { mode: options.mode } : {}),
      // Host-only, mirroring `mode` above -- see `OnlineLobbyOptions.botFill`'s
      // own doc for why this is safe to pass unconditionally (a non-host
      // value is simply never dispatched, and only applied once this peer
      // actually resolves to host -- `pendingBotFill`, not synchronously).
      ...(options?.intent === "host" ? { botFill: this.lastBotFill } : {}),
      ...(options?.presetRoomCode !== undefined ? { presetRoomCode: options.presetRoomCode } : {}),
    };
    const screen = this.online.newLobbyScreen(this.onAction(), resolved);
    this.pushRoute("lobby", screen);
  }

  // Route the lobby's synchronized start into the real online match. The
  // lobby keeps its link: the match borrows the same star, because the
  // session's control channel and the match's input channel are the same
  // transport.
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
      this.showTeamSheet();
    } else if (route === "title" && action.go === "team") {
      // Reached from the menu directly, rather than through Play -- see
      // `title.ts`'s header. Same destination either way: `team_sheet`'s
      // `back` now carries the same draft `match` does (`TeamSheetDraft`),
      // so BOTH the `title` branch below and the `match` branch commit and
      // persist it -- "visit, edit, leave" saves exactly like "on the way
      // to a match" does, instead of a standalone visit silently discarding
      // an edit on BACK.
      this.showTeamSheet();
    } else if (route === "title" && action.go === "multiplayer") {
      this.showMultiplayer();
    } else if (route === "multiplayer" && action.go === "title") {
      this.showTitle();
    } else if (route === "multiplayer" && action.go === "lobby") {
      if (action.intent === "host" && action.mode !== undefined) {
        this.lastOnlineMode = action.mode as SessionMatchMode;
        this.saveTeamPreferences();
      }
      this.showLobby({
        ...(action.intent !== undefined ? { intent: action.intent as "host" | "guest" } : {}),
        ...(action.mode !== undefined ? { mode: action.mode as string } : {}),
        // The front door's own inline code composer (#610) -- reuses the
        // SAME pre-fill-and-auto-submit path #598's join links already go
        // through (`showLobby`'s own `presetRoomCode` doc), not a parallel
        // one.
        ...(action.code !== undefined ? { presetRoomCode: action.code as string } : {}),
      });
    } else if (route === "lobby" && action.go === "online_match") {
      if (action.freeze === undefined) {
        throw new Error("an online start needs its freeze");
      }
      this.startOnlineMatch(action.freeze);
    } else if (route === "online_match" && action.go === "online_result") {
      if (action.result === undefined) {
        throw new Error("an online result needs its record");
      }
      this.showResult(action.result as ProductMatchResult);
    } else if (route === "online_match" && action.go === "online_ended") {
      const terminal = action.terminal as { readonly reason?: string } | undefined;
      this.showSessionEnded(
        terminal?.reason ?? "transport_lost",
        action.detail as string | undefined,
      );
    } else if (
      route === "session_ended" &&
      (action.go === "multiplayer" || action.go === "main_menu")
    ) {
      // Both exits go through the title, so a dead session leaves nothing
      // mounted behind it; Multiplayer then reopens the front door.
      this.showTitle();
      if (action.go === "multiplayer") {
        this.showMultiplayer();
      }
    } else if (route === "result" && action.go === "back_to_lobby") {
      this.showTitle();
      this.showMultiplayer();
    } else if (route === "title" && action.go === "help") {
      const menu = new Menu(
        help,
        help.newState(this.viewport, bindings.reference("match")),
        this.onAction(),
      );
      this.pushRoute("help", asMenu(menu));
    } else if (action.go === "credits") {
      // Reached from Settings -> About. It is no longer a front-door entry:
      // build info and attribution belong behind settings, not beside Play.
      const menu = new Menu(
        credits,
        credits.newState(this.viewport, this.content.buildInfo),
        this.onAction(),
      );
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
    } else if (route === "team_sheet" && action.go === "title") {
      // BACK carries the same draft `match` does (`TeamSheetDraft`) -- an
      // incomplete or currently-illegal five is fine here (unlike the
      // `match` branch below, nothing is about to kick off with it), and
      // gets caught again on the next boot regardless
      // (`team_settings.ts`'s `validateAgainstContent`), so this commits
      // unconditionally via the unchecked `setDraftStarters` rather than
      // the strict, `Result`-returning `setStarters`.
      session.setDraftStarters(this.session, action.starterIds as readonly string[]);
      session.setFormation(this.session, action.formationId as string);
      session.setTactic(this.session, action.tacticId as string);
      session.setCombatEnabled(this.session, action.combatEnabled === true);
      this.saveTeamPreferences();
      this.showTitle();
    } else if (route === "team_sheet" && action.go === "match") {
      // One commit, three decisions. Starters are validated first: an invalid
      // five is a programmer error by the time it reaches here, since the
      // screen refuses to emit `match` at any other count.
      const set = session.setStarters(
        this.content.matchContract,
        this.session,
        this.content.homeTeam,
        action.starterIds as readonly string[],
      );
      if (!set.ok) {
        throw new Error(set.error);
      }
      session.setFormation(this.session, action.formationId as string);
      session.setTactic(this.session, action.tacticId as string);
      session.setCombatEnabled(this.session, action.combatEnabled === true);
      this.saveTeamPreferences();
      this.startMatch();
    } else if (route === "result" && action.go === "rematch") {
      this.startMatch();
    } else if (route === "result" && action.go === "change_plan") {
      this.showTeamSheet();
    } else if (action.go === "main_menu") {
      // A hosted lobby's `bot_fill` has no commit point of its own (it is
      // toggled inside the lobby, not chosen before entering it, unlike
      // `mode` above) -- so it is read off the departing screen here,
      // instead. Host-only: `lobby_model.ts` refuses a non-host toggle, so a
      // guest's `bot_fill` is always `false` and must never overwrite a real
      // host preference (`currentLobbyRole`'s own doc).
      if (route === "lobby" && this.currentLobbyRole === "host") {
        const lobbyScreen = this.stack.current() as OnlineLobbyScreen;
        this.lastBotFill = lobbyScreen.state.model.bot_fill === true;
        this.saveTeamPreferences();
      }
      this.currentLobbyRole = undefined;
      this.showTitle();
    } else if (route === "pause" && action.go === "resume") {
      this.popRoute();
    } else if (route === "pause" && action.go === "controls") {
      const menu = new Menu(
        help,
        help.newState(this.viewport, bindings.reference("match")),
        this.onAction(),
      );
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
  // online one.
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
