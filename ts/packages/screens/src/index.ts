// Screens: pure layout/update, no drawing. See AGENTS.md §9.

export type {
  BuildInfo,
  ControlReferenceRow,
  FormationData,
  FormationRole,
  GameSettings,
  MatchWinner,
  OutfieldAnchor,
  PlayerData,
  PlayerPresentationIdentity,
  Position,
  ProductMatchRequest,
  ProductMatchResult,
  SettingsSource,
  StatBlock,
  TacticData,
  TeamData,
  TeamResultStats,
} from "./content.ts";

export { squad } from "./squad.ts";
export type {
  SquadAction,
  SquadContentData,
  SquadScreenContext,
  SquadScreenState,
} from "./squad.ts";

export { formation } from "./formation.ts";
export type {
  FormationAction,
  FormationContentData,
  FormationScreenContext,
  FormationScreenState,
} from "./formation.ts";

export { tactic } from "./tactic.ts";
export type {
  TacticAction,
  TacticContentData,
  TacticScreenContext,
  TacticScreenState,
} from "./tactic.ts";

export { result } from "./result.ts";
export type {
  ResultAction,
  ResultContentData,
  ResultScreenContext,
  ResultScreenState,
} from "./result.ts";

export { settings } from "./settings.ts";
export type { SettingsAction, SettingsScreenContext, SettingsScreenState } from "./settings.ts";

export { fakeMatch } from "./fake_match.ts";
export type {
  FakeMatchAction,
  FakeMatchScreenContext,
  FakeMatchScreenState,
} from "./fake_match.ts";

export { help } from "./help.ts";
export type { HelpAction, HelpScreenState } from "./help.ts";

export { pause } from "./pause.ts";
export type { PauseAction, PauseScreenState } from "./pause.ts";

export { title } from "./title.ts";
export type { TitleAction, TitleScreenState } from "./title.ts";

export { credits } from "./credits.ts";
export type { CreditsAction, CreditsScreenState } from "./credits.ts";

export { Menu } from "./menu.ts";
export type { ScreenDef, Viewport } from "./menu.ts";

// --- lobby / online ----------------------------------------------------

export {
  command as lobbyModelCommand,
  newLobbyModel,
  view as lobbyModelView,
  DEFAULT_MODE as LOBBY_DEFAULT_MODE,
  COUNTDOWN_ID,
  COUNTDOWN_TICKS,
  FIRST_INPUT_TICK,
  MAX_SIGNAL_BYTES,
  GUEST_LINK_PREFIX,
  MODES as LOBBY_MODES,
  TERMINAL_TEXT as LOBBY_TERMINAL_TEXT,
  DEPARTURE_TEXT as LOBBY_DEPARTURE_TEXT,
  PREFERENCE_TEXT as LOBBY_PREFERENCE_TEXT,
} from "./lobby_model.ts";
export type {
  CoordinatorAction,
  CoordinatorDeparture,
  CoordinatorManifestExpectation,
  CoordinatorNewGuestOptions,
  CoordinatorNewHostOptions,
  CoordinatorOutcome,
  CoordinatorPeer,
  CoordinatorPort,
  CoordinatorSlotDriver,
  CoordinatorState,
  CoordinatorTerminal,
  CoordinatorTerminalReason,
  Fnv1a64Port,
  InputFramePort,
  InputSlotId,
  InputTeam,
  LobbyCommand,
  LobbyEffect,
  LobbyIdentityRow,
  LobbyKeeperView,
  LobbyModel,
  LobbyModelOptions,
  LobbyModelPorts,
  LobbyPreferenceView,
  LobbyRole,
  LobbySeatView,
  LobbySignalDirection,
  LobbySignalRecord,
  LobbySlotView,
  LobbyView,
  ProtocolFixturePort,
  ProtocolMessage,
  ProtocolPort as LobbyProtocolPort,
  SessionLifecyclePhase,
  SessionManifest,
  SessionManifestTeam,
  SessionMatchMode,
  SessionMatchModeShape,
  SessionPreference,
  SessionPreferenceRejection,
  SessionPreferenceStatus,
  SessionRosterEntry,
  SessionSlotProducer,
  TransportContractPort,
} from "./lobby_model.ts";

export {
  lobby,
  newState as newLobbyScreenState,
  layout as lobbyScreenLayout,
  update as lobbyScreenUpdate,
} from "./lobby.ts";
export type {
  LobbyAction,
  LobbyScreenContext,
  LobbyScreenEvent,
  LobbyScreenState,
} from "./lobby.ts";

export { OnlineLobby } from "./online_lobby.ts";
export type {
  LobbyClipboard,
  LobbyLinkInstance,
  OnlineLobbyAction,
  OnlineLobbyOptions,
} from "./online_lobby.ts";

export {
  command as onlineMatchModelCommand,
  ended as onlineMatchModelEnded,
  exitRoute as onlineMatchModelExitRoute,
  newOnlineMatchModel,
  ABORT_PROMPT as ONLINE_MATCH_ABORT_PROMPT,
  CONTROLLER_NOTICE as ONLINE_MATCH_CONTROLLER_NOTICE,
  FOCUS_NOTICE as ONLINE_MATCH_FOCUS_NOTICE,
  NOTICE_SECONDS as ONLINE_MATCH_NOTICE_SECONDS,
  PAUSE_NOTICE as ONLINE_MATCH_PAUSE_NOTICE,
} from "./online_match_model.ts";
export type {
  CoordinatorEvent as OnlineMatchCoordinatorEvent,
  CoordinatorPort as OnlineMatchCoordinatorPort,
  CoordinatorStateCore,
  CoordinatorTerminal as OnlineMatchCoordinatorTerminal,
  LifecyclePayload,
  OnlineMatchCommand,
  OnlineMatchEffect,
  OnlineMatchModel,
  OnlineMatchModelPorts,
  OnlineMatchNotice,
  OnlineMatchPhase,
  OnlineMatchRequestCore,
  ProtocolMessage as OnlineMatchProtocolMessage,
  ProtocolPort as OnlineMatchProtocolPort,
  RollbackEventStepCore,
  RollbackWrappedEvent as OnlineMatchRollbackWrappedEvent,
} from "./online_match_model.ts";

export { OnlineMatch } from "./online_match.ts";
export type {
  LobbyFramingPort,
  LobbyFrameBuffer,
  MatchDriverPort,
  MatchLobbyLinkPort,
  MatchPresentationPort,
  MatchSessionPort,
  OnlineMatchDispatchEvent,
  OnlineMatchOptions,
  OnlineMatchRequest,
} from "./online_match.ts";

export { RealMatchScreen } from "./real_match.ts";
export type {
  MatchContractPort,
  MatchContractResultOptions,
  MatchObserverPort,
  ObservedMatchSummary,
  RealMatchCallbacks,
  RealMatchEvent,
  RealMatchEventKind,
  RealMatchInputEvent,
  RealMatchRequest,
  RealMatchRosterEntry,
  RealMatchScreenPort,
  RealMatchState,
  RealMatchTeam,
} from "./real_match.ts";

export {
  consumeConfirmedLifecycle,
  consumeConfirmedStep,
  consumeRollbackEventDiff,
  consumeRollbackPresentation,
  newMatchControlLatches,
  newMatchRollbackConsumerState,
  MatchScreen,
  MatchScreenAsOnlineMatchScreen,
  MatchScreenAsRealMatchScreen,
  stepMatchControlLatches,
} from "./match.ts";
export type {
  AudioPort,
  EffectsPort,
  InputSample,
  MatchControlLatches,
  MatchControlPoll,
  MatchContextualStep,
  MatchRollbackConsumerPorts,
  MatchRollbackConsumerState,
  MatchScreenOptions,
  MatchScreenPorts,
  MatchScreenProfile,
  OnlineHostPort,
  OnlineRenderFrameControl,
  OnlineRenderFramePlayers,
  RealMatchRenderFrameEvents,
  RenderFrame,
  RenderFrameHud,
  RenderFramePossession,
  RenderFrameRoster,
  RenderPort,
  ReplayPort,
  RollbackConfirmedStateView,
  RollbackEventStep,
  RollbackWrappedCombatEvent,
  RollbackWrappedLifecycleEvent,
  RollbackWrappedMatchEvent,
  SimHostFactory,
  SimHostPort,
  RollbackHostFactory,
  RollbackHostPort,
  RollbackLabClockDebug,
  RollbackLabCorrectionSample,
  RollbackLabDebug,
  RollbackLabDisplayedPositions,
  RollbackLabHostDebug,
  RollbackLabInputSlot,
  RollbackLabNetworkProfile,
  RollbackLabOptions,
  RollbackLabOutput,
  RollbackLabPlayerPosition,
  RollbackLabSnapshotSummary,
  RollbackLabStatus,
  RollbackLabStepResult,
} from "./match.ts";
