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
  TeamResultStats,
} from "./content.ts";

export { squad } from "./squad.ts";
export type { SquadAction, SquadContentData, SquadScreenContext, SquadScreenState } from "./squad.ts";

export { formation } from "./formation.ts";
export type {
  FormationAction,
  FormationContentData,
  FormationScreenContext,
  FormationScreenState,
} from "./formation.ts";

export { tactic } from "./tactic.ts";
export type { TacticAction, TacticContentData, TacticScreenContext, TacticScreenState } from "./tactic.ts";

export { result } from "./result.ts";
export type { ResultAction, ResultContentData, ResultScreenContext, ResultScreenState } from "./result.ts";

export { settings } from "./settings.ts";
export type { SettingsAction, SettingsScreenContext, SettingsScreenState } from "./settings.ts";

export { fakeMatch } from "./fake_match.ts";
export type { FakeMatchAction, FakeMatchScreenContext, FakeMatchScreenState } from "./fake_match.ts";

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
