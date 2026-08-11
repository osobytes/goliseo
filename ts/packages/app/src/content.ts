// Structural declarations for content this package reads but must not own.
//
// ARCHITECTURE.md §4 rule 6: "TypeScript never imports content tables — it
// receives them." `data/**` (players, teams, formations, tactics, arenas)
// is Rust-owned (`crates/gc-data`). Every shape below is declared locally
// and threaded through as an explicit parameter — the same pattern
// `@gc/screens`'s `content.ts` uses. Only the fields the modules in this
// package actually read are declared.

// --- gc-data/src/players.rs -------------------------------------------------

export type Position = "keeper" | "defender" | "midfielder" | "forward";

/** The slice of `players.rs`'s player data this package reads. */
export interface PlayerData {
  readonly id: string;
  readonly name: string;
  readonly position: Position;
}

// --- gc-data/src/teams.rs ----------------------------------------------------

/** The slice of `teams.rs`'s team data `match_contract.ts`/`session.ts` read. */
export interface TeamData {
  readonly id: string;
  readonly name: string;
  /** Full eligible pool for a team sheet; falls back to `roster` when absent (`match_contract.ts`). */
  readonly squad?: readonly string[];
  /** The default starting five. */
  readonly roster: readonly string[];
  /** `session.ts`'s default formation for this team. */
  readonly formation: string;
}

// --- gc-data/src/formations.rs / tactics.rs / arenas.rs ---------------------

/** `match_contract.ts` only checks membership; no fields are read. */
export interface FormationExists {
  readonly id: string;
}

export interface TacticExists {
  readonly id: string;
}

export interface ArenaExists {
  readonly id: string;
}

/** What `match_contract.ts` needs from `data/**`. */
export interface MatchContractContent {
  readonly teams: Readonly<Record<string, TeamData>>;
  readonly players: Readonly<Record<string, PlayerData>>;
  readonly formations: Readonly<Record<string, FormationExists>>;
  readonly tactics: Readonly<Record<string, TacticExists>>;
  readonly arenas: Readonly<Record<string, ArenaExists>>;
}
