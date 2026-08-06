// Structural declarations for content this package reads but must not own.
//
// v2/README.md rule 6.7: "TypeScript never imports content tables — it
// receives them." `data/**` (players, teams, formations, tactics, arenas)
// is Rust-owned (`crates/gc-data`). Every shape below is declared locally,
// structurally compatible with its Lua original, and threaded through as an
// explicit parameter — the same pattern `@gc/screens`'s `content.ts` uses.
// Only the fields the modules in this package actually read are declared.

// --- data/players.lua -------------------------------------------------------

export type Position = "keeper" | "defender" | "midfielder" | "forward";

/** The slice of `data/players.lua`'s `PlayerData` this package reads. */
export interface PlayerData {
  readonly id: string;
  readonly name: string;
  readonly position: Position;
}

// --- data/teams.lua ----------------------------------------------------------

/** The slice of `data/teams.lua`'s `TeamData` `match_contract.lua`/`session.lua` read. */
export interface TeamData {
  readonly id: string;
  readonly name: string;
  /** Full eligible pool for a team sheet; falls back to `roster` when absent (`match_contract.lua`). */
  readonly squad?: readonly string[];
  /** The default starting five. */
  readonly roster: readonly string[];
  /** `session.lua`'s default formation for this team. */
  readonly formation: string;
}

// --- data/formations.lua / data/tactics.lua / data/arenas.lua ---------------

/** `match_contract.lua` only checks membership; no fields are read. */
export interface FormationExists {
  readonly id: string;
}

export interface TacticExists {
  readonly id: string;
}

export interface ArenaExists {
  readonly id: string;
}

/** What `game/match_contract.lua` needs from `data/**`. */
export interface MatchContractContent {
  readonly teams: Readonly<Record<string, TeamData>>;
  readonly players: Readonly<Record<string, PlayerData>>;
  readonly formations: Readonly<Record<string, FormationExists>>;
  readonly tactics: Readonly<Record<string, TacticExists>>;
  readonly arenas: Readonly<Record<string, ArenaExists>>;
}
