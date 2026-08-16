// App shell: bootstrap, routing, screen stack, settings.

export { App } from "./app.ts";
export type {
  AppAction,
  AppContent,
  AppOptions,
  OnlinePorts,
  OnlineLobbyCoordinatorState,
  OnlineLobbyScreen,
  OnlineMatchScreen,
  Viewport,
} from "./app.ts";

export { bootstrap } from "./bootstrap.ts";
export type { BootstrapOptions } from "./bootstrap.ts";

export { ScreenStack } from "./screen_stack.ts";
export type { Screen } from "./screen_stack.ts";

export { settings } from "./settings.ts";
export type { GameSettings, SettingsStorage } from "./settings.ts";

export { session } from "./session.ts";
export type { GameSession, ResultAction } from "./session.ts";

export { matchContract } from "./match_contract.ts";
export type {
  MatchWinner,
  ProductMatchRequest,
  ProductMatchRequestOptions,
  ProductMatchResult,
  ProductMatchResultOptions,
  TeamResultStats,
} from "./match_contract.ts";

export type {
  ArenaExists,
  FormationExists,
  MatchContractContent,
  PlayerData,
  Position,
  TacticExists,
  TeamData,
} from "./content.ts";

export { matchObserver } from "./match_observer.ts";
export type {
  MatchObserver,
  MatchTeam,
  ObservedConfirmedStep,
  ObservedEventKind,
  ObservedMatchEvent,
  ObservedMatchState,
  ObservedMatchSummary,
  ObservedPlayer,
  RollbackWrappedMatchEvent,
} from "./match_observer.ts";

export { matchAdapter } from "./match_adapter.ts";
export type { MatchAdapter, MatchAdapterCallbacks, RealMatchFactory } from "./match_adapter.ts";

export { fakeResult } from "./fake_result.ts";

export { onboarding } from "./match_onboarding.ts";
export type {
  MatchOnboardingState,
  OnboardingContext,
  OnboardingPrompt,
  OnboardingPromptId,
} from "./match_onboarding.ts";

export { matchInputAdapter } from "./match_input_adapter.ts";
export type {
  MatchInput,
  MatchInputAdapterState,
  MatchInputEdges,
  MatchInputHeld,
} from "./match_input_adapter.ts";

export { hud } from "./match_hud.ts";
export type {
  BroadcastPhase,
  CombatFeedbackNotice,
  CombatPlayerPresentation,
  IdentityPort,
  MatchHudContext,
  MatchHudLayout,
  MatchHudModel,
  PlayerPresentationIdentity as HudPlayerPresentationIdentity,
  RenderFrameHud,
  RenderSpeciesShape,
  RenderTeam,
} from "./match_hud.ts";

export { Audio } from "./audio.ts";
export type {
  AudioBackend,
  AudioMatchState,
  AudioSourceHandle,
  CombatEventLike,
  CombatFeedbackPort,
  GameSettingsAudioSlice,
  RollbackWrappedEvent as AudioRollbackWrappedEvent,
} from "./audio.ts";

export { runtimeSettings } from "./runtime_settings.ts";
export type {
  BloomPort,
  CombatFeedbackDefaultsPort,
  EffectsConfigurePort,
  RuntimeSettingsPorts,
  WindowPort,
} from "./runtime_settings.ts";

export { CompatibilityFlow } from "./compatibility_flow.ts";
export type { CompatibilityFlowApp } from "./compatibility_flow.ts";

export { CompatibilityMetrics, compatibilityMetrics } from "./compatibility_metrics.ts";
export type { CompatibilityPendingInput } from "./compatibility_metrics.ts";

export { MAX_HISTORY, MAX_INPUT_BYTES, VERSION, webrtcProof } from "./webrtc_proof.ts";
export type {
  WebRTCHandshake,
  WebRTCInput,
  WebRTCProofDiagnostics,
  WebRTCProofErrorCode,
  WebRTCProofFailure,
  WebRTCProofRole,
  WebRTCProofSummary,
} from "./webrtc_proof.ts";

export { rollbackValidation } from "./rollback_validation.ts";
export type {
  EffectsPort,
  MatchSnapshotPort,
  ReplayBoundarySample,
  ReplayDiagnosticsSlice,
  ReplayPort,
  RollbackApplyFailure,
  RollbackConfirmedStateView,
  RollbackEventDiff,
  RollbackEventReplacement,
  RollbackEventStep,
  RollbackEventStepInput,
  RollbackEventsPort,
  RollbackValidationAudit,
  RollbackValidationConsumerMetrics,
  RollbackValidationEventMetrics,
  RollbackValidationFinishOptions,
  RollbackValidationPorts,
  RollbackValidationReplaySample,
  RollbackValidationReport,
  RollbackValidationScenario,
  RollbackValidationTraceRow,
  RollbackWrappedEvent as RollbackValidationWrappedEvent,
} from "./rollback_validation.ts";

export { buildInfo } from "./build_info.ts";
export type { BuildInfo } from "./build_info.ts";
