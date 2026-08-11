// Fixture note: production seeds this state from `@gc/app`'s `session.ts`
// (not this package's to own — see content.ts's header) purely to get
// Nebula FC's default starters and formation. Those defaults (`gc-data`'s
// `teams.nebula.roster` / `teams.nebula.formation`) are transcribed
// verbatim below instead. Player names/identities are the same Nebula FC
// fixture squad_product.spec.ts uses (`gc-data`'s players, `gc-render`'s
// identity), duplicated per-file to keep each spec self-contained, matching
// @gc/presentation's combat.spec.ts.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { formation, type FormationContentData } from "./formation.ts";
import { tactic, type TacticContentData } from "./tactic.ts";
import type { FormationData, PlayerPresentationIdentity, TacticData } from "./content.ts";

const VP = { w: 960, h: 540 };
const GK = { x: 0.06, y: 0.5 };

const FORMATIONS: Readonly<Record<string, FormationData>> = {
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

const TACTICS: Readonly<Record<string, TacticData>> = {
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

const PLAYERS: Readonly<Record<string, { readonly name: string }>> = {
  ozzo: { name: "Ozzo" },
  brakka: { name: "Brakka" },
  veil_nyx: { name: "Veil Nyx" },
  rok_tann: { name: "Rok Tann" },
  zyro_vex: { name: "Zyro Vex" },
};

const IDENTITIES: Readonly<Record<string, PlayerPresentationIdentity>> = {
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
};

// game.session.new()'s defaults: teams.nebula.roster / teams.nebula.formation.
const SESSION_STARTER_IDS = ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"];
const SESSION_FORMATION_ID = "2-1-1";

const FORMATION_CONTENT: FormationContentData = {
  formations: FORMATIONS,
  players: PLAYERS,
  identities: IDENTITIES,
};
const TACTIC_CONTENT: TacticContentData = { tactics: TACTICS };

describe("product tactical setup", () => {
  it("carries all five visual identities into every formation preview", () => {
    const layout = formation.layout(
      formation.newState(VP, FORMATION_CONTENT, {
        selected: SESSION_FORMATION_ID,
        starterIds: SESSION_STARTER_IDS,
      }),
    );
    for (const id of ["1-1-2", "1-2-1", "2-1-1"]) {
      const preview = hit.find(layout, `preview_${id}`);
      expect(preview).not.toBeNull();
      expect(preview?.data?.markers?.length).toBe(5);
      expect(preview?.data?.markers?.[0]?.name).toBeDefined();
      expect(preview?.data?.markers?.[0]?.shape).toBeDefined();
    }
    const lineup = hit.find(layout, "lineup");
    expect(lineup?.text).toContain("OZZO");
  });

  it("uses authored strengths and risks without exposing tuning values", () => {
    const formationState = formation.newState(VP, FORMATION_CONTENT);
    for (const widget of formation.layout(formationState)) {
      if (/^formation_/.exec(widget.id)) {
        expect(widget.text).toContain("+");
        expect(widget.text).toContain("−");
      }
    }

    const tacticState = tactic.newState(VP, TACTIC_CONTENT);
    for (const widget of tactic.layout(tacticState)) {
      if (/^tactic_/.exec(widget.id)) {
        expect(widget.text).toContain("+");
        expect(widget.text).toContain("−");
        expect(widget.text).not.toContain("stamina_drain");
      }
    }
  });
});
