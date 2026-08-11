// Ported from spec/screens/squad_product_spec.lua.
//
// Fixture note: the Lua spec drives the real `data/players.lua` /
// `data/teams.lua` / `render/identity.lua`. Those are content this package
// receives rather than imports (content.ts's header; v2/README.md rule
// 6.7), so the fixture below transcribes the real Nebula FC squad verbatim
// — same ids, names, stats, and derived identities as the production
// tables (`data/players.lua`, `data/teams.lua`, `data/species.lua`,
// `data/showcase_player_compatibility.lua`) — so the assertions exercise
// the exact same values the Lua spec does. Same approach `@gc/presentation`'s
// `combat.spec.ts` takes for `data/action_families.lua` et al.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { squad, type SquadContentData } from "./squad.ts";
import type { PlayerData, PlayerPresentationIdentity } from "./content.ts";

const VP = { w: 960, h: 540 };

const PLAYERS: Readonly<Record<string, PlayerData>> = {
  ozzo: {
    id: "ozzo",
    name: "Ozzo",
    position: "keeper",
    stats: { pace: 4, strength: 5, technique: 4, stamina: 8, mental: 8 },
  },
  brakka: {
    id: "brakka",
    name: "Brakka",
    position: "defender",
    stats: { pace: 4, strength: 8, technique: 3, stamina: 7, mental: 8 },
  },
  veil_nyx: {
    id: "veil_nyx",
    name: "Veil Nyx",
    position: "defender",
    stats: { pace: 5, strength: 6, technique: 4, stamina: 6, mental: 7 },
  },
  rok_tann: {
    id: "rok_tann",
    name: "Rok Tann",
    position: "midfielder",
    stats: { pace: 5, strength: 7, technique: 6, stamina: 7, mental: 5 },
  },
  zyro_vex: {
    id: "zyro_vex",
    name: "Zyro Vex",
    position: "forward",
    stats: { pace: 8, strength: 6, technique: 7, stamina: 5, mental: 2 },
  },
  mika_olu: {
    id: "mika_olu",
    name: "Mika Olu",
    position: "forward",
    stats: { pace: 7, strength: 5, technique: 8, stamina: 6, mental: 3 },
  },
  sela_dwin: {
    id: "sela_dwin",
    name: "Sela Dwin",
    position: "midfielder",
    stats: { pace: 6, strength: 4, technique: 7, stamina: 6, mental: 6 },
  },
  tib_quell: {
    id: "tib_quell",
    name: "Tib Quell",
    position: "midfielder",
    stats: { pace: 6, strength: 6, technique: 6, stamina: 5, mental: 5 },
  },
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

const SQUAD_IDS = [
  "ozzo",
  "brakka",
  "veil_nyx",
  "rok_tann",
  "zyro_vex",
  "mika_olu",
  "sela_dwin",
  "tib_quell",
];
const ROSTER_IDS = ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"];

const CONTENT: SquadContentData = {
  players: PLAYERS,
  identities: IDENTITIES,
  squadIds: SQUAD_IDS,
  defaultStarterIds: ROSTER_IDS,
};

function click(state: Parameters<typeof squad.update>[0], id: string) {
  const widget = hit.find(squad.layout(state), id);
  expect(widget, `missing widget ${id}`).not.toBeNull();
  const rect = widget?.rect;
  expect(rect).toBeDefined();
  return squad.update(state, {
    kind: "click",
    x: (rect?.x ?? 0) + (rect?.w ?? 0) / 2,
    y: (rect?.y ?? 0) + (rect?.h ?? 0) / 2,
    button: 1,
  });
}

describe("product squad picker", () => {
  it("shows eight authored cards and starts with a valid five", () => {
    const state = squad.newState(VP, CONTENT);
    expect(state.roster.length).toBe(8);
    expect(state.selectedIds.length).toBe(5);
    const layout = squad.layout(state);
    for (const player of state.roster) {
      const card = hit.find(layout, `player_${player.id}`);
      expect(card).not.toBeNull();
      expect(card?.kind).toBe("card");
      expect(card?.text).toContain("PAC");
      expect(card?.data?.accent).toBeDefined();
      expect(card?.data?.speciesShape).toBeDefined();
    }
  });

  it("locks the keeper and supports an explicit outfielder replacement", () => {
    let state = squad.newState(VP, CONTENT);
    [state] = click(state, "player_ozzo");
    expect(state.selectedIds.length).toBe(5);
    expect(state.message).toContain("keeper");

    [state] = click(state, "player_brakka");
    expect(state.selectedIds.length).toBe(4);
    [state] = click(state, "player_tib_quell");
    expect(state.selectedIds.length).toBe(5);
    const [, action] = click(state, "next");
    expect(action).toBeDefined();
    expect(action?.go).toBe("formation");
    expect(action && "starterIds" in action ? action.starterIds.length : undefined).toBe(5);
  });

  it("does not activate disabled continuation or static labels", () => {
    let state = squad.newState(VP, CONTENT);
    [state] = click(state, "player_brakka");
    const [unchanged, action] = click(state, "next");
    expect(action).toBeUndefined();
    expect(unchanged.selectedIds.length).toBe(4);

    const message = hit.find(squad.layout(state), "message");
    expect(message?.rect).toBeDefined();
    const [, labelAction] = squad.update(state, {
      kind: "click",
      x: (message?.rect?.x ?? 0) + 2,
      y: (message?.rect?.y ?? 0) + 2,
      button: 1,
    });
    expect(labelAction).toBeUndefined();
  });
});
