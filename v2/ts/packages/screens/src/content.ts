// Structural declarations for content this package reads but must not own.
//
// v2/README.md rule 6.7: "TypeScript never imports content tables — it
// receives them." `data/**` (players, teams, formations, tactics) is
// Rust-owned (`crates/gc-data`); `render/identity.lua`, `game/settings.lua`,
// `game/build_info.lua`, and `game/input/bindings.lua` are TypeScript-owned
// but live in packages this one must not depend on yet (`@gc/render`,
// `@gc/app`, `@gc/input` — none are a declared dependency of `@gc/screens`,
// and this task may not edit package.json). Every shape below is declared
// locally, structurally compatible with its Lua/TS original, and threaded
// through as an explicit parameter to each screen's `newState` — the same
// pattern `@gc/presentation`'s `combat.ts` uses for `CombatPresentationData`
// and `@gc/ui`'s `types.ts` uses for `FocusEvent`. Only the fields a screen
// actually reads are declared, matching `combat.ts`'s `MatchPlayerState`.
//
// Field names stay `snake_case` where they mirror a real cross-language
// content shape (`data/players.lua`'s `PlayerData`, `ProductMatchResult`,
// `GameSettings`'s on-disk keys) — same rule `combat.ts` follows for
// `CombatEvent`. UI-only shapes stay `camelCase`.

import type { Anchor, RgbColor } from "@gc/ui";

// --- data/players.lua ------------------------------------------------------

export type Position = "keeper" | "defender" | "midfielder" | "forward";

export interface StatBlock {
  readonly pace: number;
  readonly strength: number;
  readonly technique: number;
  readonly stamina: number;
  readonly mental: number;
}

/** The slice of `data/players.lua`'s `PlayerData` the screens in this package read. */
export interface PlayerData {
  readonly id: string;
  readonly name: string;
  readonly position: Position;
  readonly stats: StatBlock;
}

// --- render/identity.lua ----------------------------------------------------

/** `render/identity.lua`'s `PlayerPresentationIdentity` (`identity.for_player`'s return shape). */
export interface PlayerPresentationIdentity {
  readonly player_id: string;
  readonly name: string;
  readonly species_name: string;
  readonly tagline: string;
  readonly shape: "round" | "broad" | "angular" | "cluster";
  readonly palette: RgbColor;
}

// --- data/formations.lua ----------------------------------------------------

export type FormationRole = "def" | "mid" | "wide" | "fwd";

export interface OutfieldAnchor extends Anchor {
  readonly role: FormationRole;
}

export interface FormationData {
  readonly id: string;
  readonly name: string;
  readonly strength?: string;
  readonly risk?: string;
  readonly keeper: Anchor;
  /** Exactly 4, in defence -> attack line order. */
  readonly outfield: readonly OutfieldAnchor[];
}

// --- data/tactics.lua --------------------------------------------------------

export interface TacticData {
  readonly id: string;
  readonly name: string;
  readonly strength?: string;
  readonly risk?: string;
}

// --- game/match_contract.lua (-> @gc/app; not this package's to own) --------

export type MatchWinner = "home" | "away" | "draw";

export interface TeamResultStats {
  readonly shots?: number;
  readonly possession?: number;
  readonly saves?: number;
  readonly pass_completion?: number;
}

/** The slice of `ProductMatchResult` `result.lua` renders. */
export interface ProductMatchResult {
  readonly home_score: number;
  readonly away_score: number;
  readonly home_name: string;
  readonly away_name: string;
  readonly winner: MatchWinner;
  readonly mvp_player_id?: string;
  readonly mvp_summary?: string;
  readonly home_stats: TeamResultStats;
  readonly away_stats: TeamResultStats;
}

/** The slice of `ProductMatchRequest` `fake_match.lua` renders. */
export interface ProductMatchRequest {
  readonly formation_id: string;
  readonly tactic_id: string;
}

// --- game/settings.lua (-> @gc/app) ------------------------------------------

/** On-disk field names (`settings.serialize`'s keys) — kept verbatim, not camelCased. */
export interface GameSettings {
  readonly master_volume: number;
  readonly sfx_volume: number;
  readonly crowd_volume: number;
  readonly muted: boolean;
  readonly fullscreen: boolean;
  readonly screen_shake: boolean;
  readonly bloom: boolean;
}

/**
 * What `settings.lua` needs from `game/settings.lua`'s module. Injected for
 * the same reason `@gc/ui`'s `tuning_panel.ts` injects a `TuningSource`:
 * `game/settings.lua` is `game/` root -> `@gc/app`, a package this one must
 * not depend on. `validate` is only needed once, at `newState` — see
 * settings.ts's header comment for why `update` does not need it again.
 */
export interface SettingsSource {
  defaults(): GameSettings;
  validate(input: Partial<GameSettings> | undefined): GameSettings;
}

// --- game/build_info.lua (-> @gc/app) -----------------------------------------

export interface BuildInfo {
  readonly name: string;
  readonly version: string;
  readonly channel: "development" | "release";
  readonly source_url?: string;
}

// --- data/teams.lua ----------------------------------------------------------

/** The slice of `data/teams.lua`'s `TeamData` `real_match.ts`/`match.ts` read. */
export interface TeamData {
  readonly id: string;
  readonly name: string;
  /** `{r, g, b}` in 0..1. */
  readonly color: readonly [number, number, number];
  /** Key into `data/formations.lua`. */
  readonly formation: string;
  /** 5 player ids from `data/players.lua`. */
  readonly roster: readonly string[];
  /** Eligible player ids; defaults to `roster` when absent. */
  readonly squad?: readonly string[];
}

// --- game/input/bindings.lua (-> @gc/input) -----------------------------------

/**
 * Structurally identical to `@gc/input`'s `ControlReferenceRow`
 * (`bindings.ts`). Declared locally rather than imported for the same
 * reason `@gc/ui`'s `types.ts` declares `FocusEvent` locally instead of
 * importing it: `@gc/screens` is not (yet) a declared dependency of
 * `@gc/input`, or vice versa, and this task may not edit package.json.
 */
export interface ControlReferenceRow {
  readonly label: string;
  readonly note?: string;
  readonly footnote: boolean;
  readonly keyboard: string;
  readonly gamepad: string;
}
