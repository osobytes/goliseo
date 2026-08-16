// Fixture note: player, team, formation and tactic data are content this
// package receives rather than imports (content.ts's header;
// ARCHITECTURE.md §4 rule 6), so the fixtures below transcribe the real
// tables verbatim — same ids, names, stats, anchors and derived identities as
// production (`gc-data`'s `players.rs`, `teams.rs`, `formations.rs`,
// `tactics.rs`, `species.rs`, `showcase_player_compatibility.rs`) — so the
// assertions exercise the exact values production does. Tactic fixtures carry
// only the fields the screen reads (id/name/strength/risk); `gc-data`'s
// marking/transition/press tuning knobs are sim-only and never reach a screen.
//
// This file consolidates what squad_product.spec.ts, formation.spec.ts,
// tactical_product.spec.ts and tactic.spec.ts asserted before those three
// screens became one. The last case drives a real `@gc/wasm` session
// directly: `packages/screens/package.json` lists `@gc/wasm` as a
// **devDependency** (test-only; production `team_sheet.ts` never imports it,
// so the dependency direction in AGENTS.md §2 stays intact).

import { describe, expect, it } from "vitest";
import { hit, theme } from "@gc/ui";
import { loadSimHost } from "@gc/wasm";
import { teamSheet, type TeamSheetContentData } from "./team_sheet.ts";
import type {
  FormationData,
  PlayerData,
  PlayerPresentationIdentity,
  TacticData,
} from "./content.ts";

const VP = { w: 960, h: 540 };
const GK = { x: 0.06, y: 0.5 };

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
// game.session.new()'s defaults: teams.nebula.roster / teams.nebula.formation.
const ROSTER_IDS = ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"];

const CONTENT: TeamSheetContentData = {
  players: PLAYERS,
  identities: IDENTITIES,
  squadIds: SQUAD_IDS,
  defaultStarterIds: ROSTER_IDS,
  formations: FORMATIONS,
  tactics: TACTICS,
  teamName: "Nebula FC",
};

type State = Parameters<typeof teamSheet.update>[0];

function click(state: State, id: string) {
  const widget = hit.find(teamSheet.layout(state), id);
  expect(widget, `missing widget ${id}`).not.toBeNull();
  const rect = widget?.rect;
  expect(rect).toBeDefined();
  return teamSheet.update(state, {
    kind: "click",
    x: (rect?.x ?? 0) + (rect?.w ?? 0) / 2,
    y: (rect?.y ?? 0) + (rect?.h ?? 0) / 2,
    button: 1,
  });
}

function offeredIds(
  layout: ReturnType<typeof teamSheet.layout>,
  prefix: string,
): readonly string[] {
  const ids: string[] = [];
  const pattern = new RegExp(`^${prefix}_(.+)$`);
  for (const widget of layout) {
    if (widget.kind === "button") {
      const id = pattern.exec(widget.id)?.[1];
      if (id !== undefined) {
        ids.push(id);
      }
    }
  }
  return ids;
}

describe("team sheet: the five", () => {
  it("shows eight authored cards and starts with a valid five", () => {
    const state = teamSheet.newState(VP, CONTENT);
    expect(state.roster.length).toBe(8);
    expect(state.selectedIds.length).toBe(5);
    const layout = teamSheet.layout(state);
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
    let state = teamSheet.newState(VP, CONTENT);
    [state] = click(state, "player_ozzo");
    expect(state.selectedIds.length).toBe(5);
    expect(state.message).toContain("keeper");

    [state] = click(state, "player_brakka");
    expect(state.selectedIds.length).toBe(4);
    [state] = click(state, "player_tib_quell");
    expect(state.selectedIds.length).toBe(5);
  });

  // Caught in the real browser first: every roster card was 40px tall for
  // 42.5px of text, so the stat line was clipped by three pixels. Nothing in
  // the pure layer measures text, so the invariant is asserted geometrically
  // against the theme's own body size rather than left to a screenshot.
  it("gives a roster card room for both of its lines", () => {
    const CARD_TOP_INSET = 10; // draw.ts's drawCard
    const LINE_HEIGHT = theme.fonts.body * 1.25; // canvas_graphics_backend.ts's printf
    const needed = CARD_TOP_INSET + 2 * LINE_HEIGHT;

    const state = teamSheet.newState(VP, CONTENT);
    for (const player of state.roster) {
      const card = hit.find(teamSheet.layout(state), `player_${player.id}`);
      expect(card?.text, `${player.id} should carry a name line and a stat line`).toContain("\n");
      expect(
        card?.rect?.h ?? 0,
        `player_${player.id} would clip its stat line`,
      ).toBeGreaterThanOrEqual(needed);
    }
  });

  it("does not activate a disabled kick off, or a static label", () => {
    let state = teamSheet.newState(VP, CONTENT);
    [state] = click(state, "player_brakka");
    expect(hit.find(teamSheet.layout(state), "kickoff")?.data?.disabled).toBe(true);

    const [unchanged, action] = click(state, "kickoff");
    expect(action).toBeUndefined();
    expect(unchanged.selectedIds.length).toBe(4);

    const message = hit.find(teamSheet.layout(state), "message");
    expect(message?.rect).toBeDefined();
    const [, labelAction] = teamSheet.update(state, {
      kind: "click",
      x: (message?.rect?.x ?? 0) + 2,
      y: (message?.rect?.y ?? 0) + 2,
      button: 1,
    });
    expect(labelAction).toBeUndefined();
  });
});

describe("team sheet: the shape", () => {
  it("defaults to the balanced 2-1-1 formation", () => {
    const s = teamSheet.newState(VP, CONTENT);
    expect(s.formationId).toBe("2-1-1");
    expect(hit.find(teamSheet.layout(s), "formation_2-1-1")?.selected).toBe(true);
  });

  it("offers every authored formation in stable key order", () => {
    const actual = offeredIds(teamSheet.layout(teamSheet.newState(VP, CONTENT)), "formation");
    const expected = Object.keys(FORMATIONS).sort();

    expect(actual.length).toBe(expected.length);
    expected.forEach((id, i) => {
      expect(actual[i]).toBe(id);
      expect(FORMATIONS[actual[i] ?? ""]).toBeDefined();
    });
  });

  it("discovers a newly authored formation without a screen edit", () => {
    const id = "0-2-2-test";
    const extended: TeamSheetContentData = {
      ...CONTENT,
      formations: {
        ...FORMATIONS,
        [id]: {
          id,
          name: "Test Shape",
          keeper: FORMATIONS["2-1-1"]?.keeper ?? GK,
          outfield: FORMATIONS["2-1-1"]?.outfield ?? [],
        },
      },
    };

    const layout = teamSheet.layout(teamSheet.newState(VP, extended));
    expect(offeredIds(layout, "formation")[0]).toBe(id);
    expect(hit.find(layout, `formation_${id}`)).not.toBeNull();
  });

  it("selects the clicked formation without mutating its input state", () => {
    const s = teamSheet.newState(VP, CONTENT);
    const [s2] = click(s, "formation_1-1-2");
    expect(s2.formationId).toBe("1-1-2");
    expect(s.formationId, "update should not mutate its input state").toBe("2-1-1");
  });

  it("keeps one legible anchor preview, and it follows the selection", () => {
    let s = teamSheet.newState(VP, CONTENT);
    for (const id of Object.keys(FORMATIONS).sort()) {
      [s] = click(s, `formation_${id}`);
      const preview = hit.find(teamSheet.layout(s), `preview_${id}`);
      expect(preview, `missing preview for ${id}`).not.toBeNull();
      expect(preview?.kind).toBe("formation_preview");
      expect(preview?.rect?.w).toBeGreaterThanOrEqual(100);
      expect(preview?.rect?.h).toBeGreaterThanOrEqual(40);
      expect(preview?.data?.keeper).toEqual(FORMATIONS[id]?.keeper);
      expect(preview?.data?.outfield).toEqual(FORMATIONS[id]?.outfield);
      expect(preview?.data?.outfield?.length).toBe(4);
      // Exactly one preview is drawn: the shape actually chosen.
      const previews = teamSheet.layout(s).filter((w) => w.kind === "formation_preview");
      expect(previews.length).toBe(1);
    }
  });

  it("selects a formation when its shape preview is clicked", () => {
    let s = teamSheet.newState(VP, CONTENT);
    [s] = click(s, "formation_1-2-1");
    const [s2] = click(s, "preview_1-2-1");
    expect(s2.formationId).toBe("1-2-1");
  });

  it("carries all five visual identities into the preview markers", () => {
    const layout = teamSheet.layout(
      teamSheet.newState(VP, CONTENT, { starterIds: ROSTER_IDS, formationId: "2-1-1" }),
    );
    const preview = hit.find(layout, "preview_2-1-1");
    expect(preview?.data?.markers?.length).toBe(5);
    expect(preview?.data?.markers?.[0]?.name).toBe("Ozzo");
    expect(preview?.data?.markers?.[0]?.shape).toBe("round");
    expect(preview?.data?.markers?.[0]?.color).toEqual([0.35, 0.75, 1.0]);
  });

  it("states the chosen shape's authored strength and risk", () => {
    let s = teamSheet.newState(VP, CONTENT);
    const notesOf = (state: State) => hit.find(teamSheet.layout(state), "shape_notes")?.text ?? "";
    expect(notesOf(s)).toContain("+");
    expect(notesOf(s)).toContain("−");
    expect(notesOf(s)).toContain("Two defenders protect the middle.");

    [s] = click(s, "formation_1-1-2");
    expect(notesOf(s)).toContain("Large spaces open behind the first press.");
  });

  it("only offers formations accepted by the match simulation", () => {
    const { Session } = loadSimHost();
    const ids = offeredIds(teamSheet.layout(teamSheet.newState(VP, CONTENT)), "formation");
    for (const id of ids) {
      const session = new Session("nebula", "orion", 1, 60, 99, id);
      try {
        // `rosterNumeric()`'s header word 5 is the roster slot count
        // (`gc_render::frame_buffer::encode_roster`'s layout) -- 5 per side
        // for the fixture teams, so the total across both sides is 10.
        expect(session.rosterNumeric()[5]).toBe(10);
      } finally {
        session.free();
      }
    }
  });
});

describe("team sheet: the plan", () => {
  it("defaults to balanced and offers all three tactics", () => {
    const s = teamSheet.newState(VP, CONTENT);
    expect(s.tacticId).toBe("balanced");
    expect(offeredIds(teamSheet.layout(s), "tactic")).toEqual(Object.keys(TACTICS).sort());
    expect(hit.find(teamSheet.layout(s), "tactic_balanced")?.selected).toBe(true);
  });

  it("selects the clicked tactic", () => {
    const s = teamSheet.newState(VP, CONTENT);
    const [s2] = click(s, "tactic_press_high");
    expect(s2.tacticId).toBe("press_high");
    expect(s.tacticId, "update should not mutate its input state").toBe("balanced");
  });

  it("uses authored strengths and risks without exposing tuning values", () => {
    for (const widget of teamSheet.layout(teamSheet.newState(VP, CONTENT))) {
      if (/^tactic_/.exec(widget.id)) {
        expect(widget.text).toContain("+");
        expect(widget.text).toContain("−");
        expect(widget.text).not.toContain("stamina_drain");
      }
    }
  });

  it("ships combat on, as a toggle rather than a second Play button", () => {
    let s = teamSheet.newState(VP, CONTENT);
    expect(s.combatEnabled).toBe(true);
    expect(hit.find(teamSheet.layout(s), "combat")?.text).toContain("ON");

    [s] = click(s, "combat");
    expect(s.combatEnabled).toBe(false);
    expect(hit.find(teamSheet.layout(s), "combat")?.text).toContain("OFF");
  });

  it("honours an injected combat preference", () => {
    const s = teamSheet.newState(VP, CONTENT, { combatEnabled: false });
    expect(s.combatEnabled).toBe(false);
  });
});

describe("team sheet: committing the decision", () => {
  it("restates the whole decision in the footer before it is committed", () => {
    let s = teamSheet.newState(VP, CONTENT);
    [s] = click(s, "tactic_press_high");
    [s] = click(s, "formation_1-1-2");
    const summary = hit.find(teamSheet.layout(s), "summary")?.text ?? "";
    expect(summary).toContain("PRESS HIGH");
    expect(summary).toContain("1-1-2");
    expect(summary).toContain("COMBAT ON");
  });

  it("emits one match transition carrying all three decisions at once", () => {
    let s = teamSheet.newState(VP, CONTENT);
    [s] = click(s, "formation_1-2-1");
    [s] = click(s, "tactic_counter");
    [s] = click(s, "player_brakka");
    [s] = click(s, "player_tib_quell");

    const [, action] = click(s, "kickoff");
    expect(action, "expected a transition action").toBeDefined();
    expect(action?.go).toBe("match");
    if (action === undefined || action.go !== "match") {
      throw new Error("expected a match transition");
    }
    expect(action.formationId).toBe("1-2-1");
    expect(action.tacticId).toBe("counter");
    expect(action.combatEnabled).toBe(true);
    expect(action.starterIds.length).toBe(5);
    expect(action.starterIds).toContain("tib_quell");
    expect(action.starterIds).not.toContain("brakka");
  });

  it("returns to the title on Back and on the back action", () => {
    const s = teamSheet.newState(VP, CONTENT);
    expect(click(s, "back")[1]?.go).toBe("title");
    expect(teamSheet.update(s, { kind: "action", action: "back" })[1]?.go).toBe("title");
  });

  it("does nothing when clicking empty space", () => {
    const s = teamSheet.newState(VP, CONTENT);
    const [, action] = teamSheet.update(s, { kind: "click", x: 5, y: 5 });
    expect(action).toBeUndefined();
  });

  it("keeps every widget inside the virtual canvas", () => {
    for (const widget of teamSheet.layout(teamSheet.newState(VP, CONTENT))) {
      const rect = widget.rect;
      expect(rect, `widget ${widget.id} has no rect`).toBeDefined();
      expect(rect?.x ?? -1, `${widget.id} starts left of the canvas`).toBeGreaterThanOrEqual(0);
      expect(rect?.y ?? -1, `${widget.id} starts above the canvas`).toBeGreaterThanOrEqual(0);
      expect((rect?.x ?? 0) + (rect?.w ?? 0), `${widget.id} overflows right`).toBeLessThanOrEqual(
        VP.w,
      );
      expect((rect?.y ?? 0) + (rect?.h ?? 0), `${widget.id} overflows bottom`).toBeLessThanOrEqual(
        VP.h,
      );
    }
  });
});
