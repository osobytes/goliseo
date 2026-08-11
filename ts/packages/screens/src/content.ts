// Structural declarations for content this package reads but must not own.
//
// ARCHITECTURE.md §4 rule 6: "TypeScript never imports content tables — it
// receives them." Players, teams, formations, and tactics are Rust-owned
// (`crates/gc-data`); presentation identity (`gc-render`'s `identity.rs`),
// settings, build info, and input bindings are TypeScript-owned but live in
// packages this one must not depend on yet (`@gc/render`, `@gc/app`,
// `@gc/input` — none are a declared dependency of `@gc/screens`, and this
// task may not edit package.json). Every shape below is declared locally,
// structurally compatible with the owning crate/package's real shape, and
// threaded through as an explicit parameter to each screen's `newState` —
// the same pattern `@gc/presentation`'s `combat.ts` uses for
// `CombatPresentationData` and `@gc/ui`'s `types.ts` uses for `FocusEvent`.
// Only the fields a screen actually reads are declared, matching
// `combat.ts`'s `MatchPlayerState`.
//
// Field names stay `snake_case` where they mirror a real cross-language
// content shape (`gc-data`'s player `PlayerData`, `ProductMatchResult`,
// `GameSettings`'s on-disk keys) — same rule `combat.ts` follows for
// `CombatEvent`. UI-only shapes stay `camelCase`.

import type { Anchor, RgbColor } from "@gc/ui";

// --- players (crates/gc-data) ------------------------------------------------------

export type Position = "keeper" | "defender" | "midfielder" | "forward";

export interface StatBlock {
  readonly pace: number;
  readonly strength: number;
  readonly technique: number;
  readonly stamina: number;
  readonly mental: number;
}

/** The slice of `gc-data`'s player `PlayerData` the screens in this package read. */
export interface PlayerData {
  readonly id: string;
  readonly name: string;
  readonly position: Position;
  readonly stats: StatBlock;
}

// --- presentation identity (gc-render) ----------------------------------------------------

/** `gc-render`'s `identity.rs`'s `PlayerPresentationIdentity` (`for_player`'s return shape). */
export interface PlayerPresentationIdentity {
  readonly player_id: string;
  readonly name: string;
  readonly species_name: string;
  readonly tagline: string;
  readonly shape: "round" | "broad" | "angular" | "cluster";
  readonly palette: RgbColor;
}

// --- formations (crates/gc-data) ----------------------------------------------------

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

// --- tactics (crates/gc-data) --------------------------------------------------------

export interface TacticData {
  readonly id: string;
  readonly name: string;
  readonly strength?: string;
  readonly risk?: string;
}

// --- @gc/app's match_contract.ts (not this package's to own) --------

export type MatchWinner = "home" | "away" | "draw";

export interface TeamResultStats {
  readonly shots?: number;
  readonly possession?: number;
  readonly saves?: number;
  readonly pass_completion?: number;
}

/** The slice of `ProductMatchResult` `result.ts` renders. */
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

/** The slice of `ProductMatchRequest` `fake_match.ts` renders. */
export interface ProductMatchRequest {
  readonly formation_id: string;
  readonly tactic_id: string;
}

// --- @gc/app's settings.ts ------------------------------------------

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
 * What `settings.ts` (this package's screen) needs from `@gc/app`'s
 * `settings.ts` module. Injected for the same reason `@gc/ui`'s
 * `tuning_panel.ts` injects a `TuningSource`: `@gc/app` is a package this
 * one must not depend on. `validate` is only needed once, at `newState` —
 * see this package's settings.ts's header comment for why `update` does
 * not need it again.
 */
export interface SettingsSource {
  defaults(): GameSettings;
  validate(input: Partial<GameSettings> | undefined): GameSettings;
}

// --- @gc/app's build_info.ts -----------------------------------------

export interface BuildInfo {
  readonly name: string;
  readonly version: string;
  readonly channel: "development" | "release";
  readonly source_url?: string;
}

// --- teams (crates/gc-data) ----------------------------------------------------

/** The slice of `gc-data`'s `TeamData` `real_match.ts`/`match.ts` read. */
export interface TeamData {
  readonly id: string;
  readonly name: string;
  /** `{r, g, b}` in 0..1. */
  readonly color: readonly [number, number, number];
  /** Key into `gc-data`'s formations. */
  readonly formation: string;
  /** 5 player ids from `gc-data`'s players. */
  readonly roster: readonly string[];
  /** Eligible player ids; defaults to `roster` when absent. */
  readonly squad?: readonly string[];
}

// --- @gc/input's bindings.ts -----------------------------------

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
