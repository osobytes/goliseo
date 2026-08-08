// Test-only fixture data, transcribed verbatim from `data/players.lua`,
// `data/teams.lua`, `data/formations.lua`, `data/tactics.lua`, and
// `data/arenas.lua` — the same ids, names, and squads the Lua specs this
// package ports exercise. Those tables are Rust-owned (v2/README.md rule
// 6.7), so this package receives content as an explicit parameter rather
// than importing it; this fixture is the "explicit parameter" a spec
// supplies. Same approach `@gc/screens`'s `squad_product.spec.ts` takes for
// `data/players.lua`/`data/teams.lua`.
//
// Not exported from `src/index.ts` — this is test infrastructure, not a
// port of production Lua.

import type {
  FormationData,
  InputSample,
  PlayerData as ScreensPlayerData,
  PlayerPresentationIdentity,
  RenderFrame,
  RenderFrameRoster,
  RenderPort,
  SimHostFactory,
  SimHostPort,
  SquadContentData,
  FormationContentData,
  TacticContentData,
  TacticData,
} from "@gc/screens";
import type { KeyboardState } from "@gc/input";
import type { MatchContractContent, PlayerData, TeamData } from "../content.ts";
import type { AppContent } from "../app.ts";

export const NEBULA_PLAYERS: Readonly<Record<string, PlayerData>> = {
  ozzo: { id: "ozzo", name: "Ozzo", position: "keeper" },
  brakka: { id: "brakka", name: "Brakka", position: "defender" },
  veil_nyx: { id: "veil_nyx", name: "Veil Nyx", position: "defender" },
  rok_tann: { id: "rok_tann", name: "Rok Tann", position: "midfielder" },
  zyro_vex: { id: "zyro_vex", name: "Zyro Vex", position: "forward" },
  mika_olu: { id: "mika_olu", name: "Mika Olu", position: "forward" },
  sela_dwin: { id: "sela_dwin", name: "Sela Dwin", position: "midfielder" },
  tib_quell: { id: "tib_quell", name: "Tib Quell", position: "midfielder" },
};

export const ORION_PLAYERS: Readonly<Record<string, PlayerData>> = {
  gax_oru: { id: "gax_oru", name: "Gax Oru", position: "keeper" },
  drell: { id: "drell", name: "Drell", position: "defender" },
  morv: { id: "morv", name: "Morv", position: "midfielder" },
  krag: { id: "krag", name: "Krag", position: "defender" },
  tox_vren: { id: "tox_vren", name: "Tox Vren", position: "forward" },
};

export const PLAYERS: Readonly<Record<string, PlayerData>> = { ...NEBULA_PLAYERS, ...ORION_PLAYERS };

export const NEBULA: TeamData = {
  id: "nebula",
  name: "Nebula FC",
  formation: "2-1-1",
  roster: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
  squad: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex", "mika_olu", "sela_dwin", "tib_quell"],
};

export const ORION: TeamData = {
  id: "orion",
  name: "Orion Miners",
  formation: "1-1-2",
  roster: ["gax_oru", "drell", "morv", "krag", "tox_vren"],
};

export const TEAMS: Readonly<Record<string, TeamData>> = { nebula: NEBULA, orion: ORION };

export const FORMATIONS: Readonly<Record<string, { readonly id: string }>> = {
  "2-1-1": { id: "2-1-1" },
  "1-2-1": { id: "1-2-1" },
  "1-1-2": { id: "1-1-2" },
};

export const TACTICS: Readonly<Record<string, { readonly id: string }>> = {
  balanced: { id: "balanced" },
  press_high: { id: "press_high" },
  counter: { id: "counter" },
};

export const ARENAS: Readonly<Record<string, { readonly id: string }>> = {
  helios_crown: { id: "helios_crown" },
};

export const MATCH_CONTRACT_CONTENT: MatchContractContent = {
  teams: TEAMS,
  players: PLAYERS,
  formations: FORMATIONS,
  tactics: TACTICS,
  arenas: ARENAS,
};

// --- @gc/screens content bundles (Nebula squad, real formations/tactics) ---

export const IDENTITIES: Readonly<Record<string, PlayerPresentationIdentity>> = {
  ozzo: {
    player_id: "ozzo",
    name: "Ozzo",
    species_name: "Terran",
    tagline: "Composed and versatile",
    shape: "round",
    palette: [0.35, 0.75, 1.0],
  },
  brakka: {
    player_id: "brakka",
    name: "Brakka",
    species_name: "Gravling",
    tagline: "Powerful anchor players",
    shape: "broad",
    palette: [1.0, 0.55, 0.25],
  },
  veil_nyx: {
    player_id: "veil_nyx",
    name: "Veil Nyx",
    species_name: "Gravling",
    tagline: "Powerful anchor players",
    shape: "broad",
    palette: [1.0, 0.55, 0.25],
  },
  rok_tann: {
    player_id: "rok_tann",
    name: "Rok Tann",
    species_name: "Terran",
    tagline: "Composed and versatile",
    shape: "round",
    palette: [0.35, 0.75, 1.0],
  },
  zyro_vex: {
    player_id: "zyro_vex",
    name: "Zyro Vex",
    species_name: "Voltari",
    tagline: "Electric breakaway speed",
    shape: "angular",
    palette: [0.9, 0.85, 0.2],
  },
  mika_olu: {
    player_id: "mika_olu",
    name: "Mika Olu",
    species_name: "Myceloid",
    tagline: "Technical collective minds",
    shape: "cluster",
    palette: [0.7, 0.4, 0.95],
  },
  sela_dwin: {
    player_id: "sela_dwin",
    name: "Sela Dwin",
    species_name: "Voltari",
    tagline: "Electric breakaway speed",
    shape: "angular",
    palette: [0.9, 0.85, 0.2],
  },
  tib_quell: {
    player_id: "tib_quell",
    name: "Tib Quell",
    species_name: "Myceloid",
    tagline: "Technical collective minds",
    shape: "cluster",
    palette: [0.7, 0.4, 0.95],
  },
};

const GK = { x: 0.06, y: 0.5 };

export const FULL_FORMATIONS: Readonly<Record<string, FormationData>> = {
  "2-1-1": {
    id: "2-1-1",
    name: "Balanced",
    strength: "Two defenders protect the middle.",
    risk: "The lone forward can become isolated.",
    keeper: GK,
    outfield: [
      { x: 0.28, y: 0.3, role: "def" },
      { x: 0.28, y: 0.7, role: "def" },
      { x: 0.52, y: 0.5, role: "mid" },
      { x: 0.76, y: 0.5, role: "fwd" },
    ],
  },
  "1-2-1": {
    id: "1-2-1",
    name: "Control",
    strength: "Two midfielders create passing angles.",
    risk: "Only one defender guards counterattacks.",
    keeper: GK,
    outfield: [
      { x: 0.26, y: 0.5, role: "def" },
      { x: 0.52, y: 0.3, role: "wide" },
      { x: 0.52, y: 0.7, role: "wide" },
      { x: 0.78, y: 0.5, role: "fwd" },
    ],
  },
  "1-1-2": {
    id: "1-1-2",
    name: "Aggressive",
    strength: "Two forwards keep constant goal pressure.",
    risk: "Large spaces open behind the first press.",
    keeper: GK,
    outfield: [
      { x: 0.26, y: 0.5, role: "def" },
      { x: 0.5, y: 0.5, role: "mid" },
      { x: 0.76, y: 0.3, role: "fwd" },
      { x: 0.76, y: 0.7, role: "fwd" },
    ],
  },
};

export const FULL_TACTICS: Readonly<Record<string, TacticData>> = {
  balanced: {
    id: "balanced",
    name: "Balanced",
    strength: "Keeps one presser and a compact supporting shape.",
    risk: "Creates fewer overloads at either end.",
  },
  press_high: {
    id: "press_high",
    name: "Press High",
    strength: "Wins the ball closer to the opponent goal.",
    risk: "A beaten press exposes space behind it.",
  },
  counter: {
    id: "counter",
    name: "Counter Attack",
    strength: "Drops compact, then attacks open space quickly.",
    risk: "Concedes territory and sustained possession.",
  },
};

// `@gc/screens`'s `SquadContentData` needs the full `PlayerData` (with
// `stats`), unlike this package's own narrower `content.ts` `PlayerData`
// (match_contract.ts/session.ts only ever read `position`). Values
// transcribed from `data/players.lua`, same as `squad_product.spec.ts`.
const SCREENS_NEBULA_PLAYERS: Readonly<Record<string, ScreensPlayerData>> = {
  ozzo: { id: "ozzo", name: "Ozzo", position: "keeper", stats: { pace: 4, strength: 5, technique: 4, stamina: 8, mental: 8 } },
  brakka: { id: "brakka", name: "Brakka", position: "defender", stats: { pace: 4, strength: 8, technique: 3, stamina: 7, mental: 8 } },
  veil_nyx: { id: "veil_nyx", name: "Veil Nyx", position: "defender", stats: { pace: 5, strength: 6, technique: 4, stamina: 6, mental: 7 } },
  rok_tann: { id: "rok_tann", name: "Rok Tann", position: "midfielder", stats: { pace: 5, strength: 7, technique: 6, stamina: 7, mental: 5 } },
  zyro_vex: { id: "zyro_vex", name: "Zyro Vex", position: "forward", stats: { pace: 8, strength: 6, technique: 7, stamina: 5, mental: 2 } },
  mika_olu: { id: "mika_olu", name: "Mika Olu", position: "forward", stats: { pace: 7, strength: 5, technique: 8, stamina: 6, mental: 3 } },
  sela_dwin: { id: "sela_dwin", name: "Sela Dwin", position: "midfielder", stats: { pace: 6, strength: 4, technique: 7, stamina: 6, mental: 6 } },
  tib_quell: { id: "tib_quell", name: "Tib Quell", position: "midfielder", stats: { pace: 6, strength: 6, technique: 6, stamina: 5, mental: 5 } },
};

export const SQUAD_CONTENT: SquadContentData = {
  players: SCREENS_NEBULA_PLAYERS,
  identities: IDENTITIES,
  squadIds: NEBULA.squad ?? NEBULA.roster,
  defaultStarterIds: NEBULA.roster,
};

export const FORMATION_CONTENT: FormationContentData = {
  formations: FULL_FORMATIONS,
  players: NEBULA_PLAYERS,
  identities: IDENTITIES,
};

export const TACTIC_CONTENT: TacticContentData = {
  tactics: FULL_TACTICS,
};

export const APP_CONTENT: AppContent = {
  matchContract: MATCH_CONTRACT_CONTENT,
  homeTeam: NEBULA,
  squad: SQUAD_CONTENT,
  formation: FORMATION_CONTENT,
  tactic: TACTIC_CONTENT,
  buildInfo: { name: "GOLISEO", version: "0.1.0-dev", channel: "development" },
};

// --- real-match test doubles (bootstrap.spec.ts's "real match adapter") ---
//
// A hand-written `SimHostPort` fake, mirroring `@gc/screens`'s own
// `match_screen.spec.ts`'s `FakeSimHost` (not importable across a `.spec.ts`
// file, and not part of that package's public surface either way) --
// `real_match_factory.ts`'s `createRealMatchFactory` is written against the
// exact same injected-`createHost`/`renderer` seam that file's `MatchScreen`
// tests use, specifically so a real `RealMatchScreen` can be driven end to
// end here without a real wasm build or a live GL context. `hud.finished`
// defaults to `false`, matching kickoff.

export function fakeKeyboard(down: Readonly<Record<string, boolean>> = {}): KeyboardState {
  return {
    isDown: (...keys: readonly string[]): boolean => keys.some((key) => down[key] === true),
  };
}

export const noopRenderPort: RenderPort = {
  draw: (): void => {},
};

export class FakeSimHost implements SimHostPort {
  readonly stepCalls: InputSample[] = [];
  disposeCalls = 0;
  readonly hud: {
    finished: boolean;
    controlled_owns_ball: boolean;
    home_score: number;
    away_score: number;
    time_left: number;
  } = {
    finished: false,
    controlled_owns_ball: true,
    home_score: 0,
    away_score: 0,
    time_left: 120,
  };
  private tickCount = 0;

  planTicks(_dt: number): number {
    // One tick per render call is enough to drive `RealMatchScreen`'s own
    // full-time-hold timing (it reads wall-clock `dt` directly, not tick
    // count) -- see this fixture's callers.
    return 1;
  }

  step(sample: InputSample): void {
    this.stepCalls.push(sample);
    this.tickCount += 1;
  }

  cancelPlannedTicks(): void {}

  frame(): RenderFrame {
    return { hud: this.hud, possession: {} };
  }

  roster(): RenderFrameRoster {
    return {};
  }

  tick(): number {
    return this.tickCount;
  }

  dispose(): void {
    this.disposeCalls += 1;
  }
}

/** One {@link FakeSimHost} per constructed match, so a test can reach into whichever one is currently live. */
export function fakeHostFactory(): { readonly createHost: () => SimHostFactory; readonly hosts: FakeSimHost[] } {
  const hosts: FakeSimHost[] = [];
  return {
    createHost: () => (): SimHostPort => {
      const host = new FakeSimHost();
      hosts.push(host);
      return host;
    },
    hosts,
  };
}
