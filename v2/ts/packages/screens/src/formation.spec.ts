// Ported from spec/screens/formation_spec.lua.
//
// Fixture note: `data/formations.lua`'s three formations are transcribed
// verbatim below (content.ts's header explains why this package receives
// rather than imports `data/**`). The Lua spec's final case,
// "only offers formations accepted by the match simulation", drives
// `sim.match.new` directly — `sim/**` is Rust-owned (`crates/gc-sim`).
// `@gc/wasm` now bridges `gc-sim` into TypeScript (v2/README.md §1), but
// `@gc/screens` (this package, "pure layout/update, no drawing" per its own
// description) does not declare `@gc/wasm` as a dependency and should not
// gain one just to reach a sim constructor — that dependency direction
// belongs to a screen's host/assembly layer (`@gc/app`), not to a layout
// module. Still ported as `it.skip`; see that test for what would unblock
// it.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { formation, type FormationContentData } from "./formation.ts";
import type { FormationData } from "./content.ts";

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

const CONTENT: FormationContentData = { formations: FORMATIONS, players: {}, identities: {} };

function sortedDataIds(formations: Readonly<Record<string, FormationData>>): readonly string[] {
  return Object.keys(formations).sort();
}

function offeredIds(layout: ReturnType<typeof formation.layout>): readonly string[] {
  const ids: string[] = [];
  for (const widget of layout) {
    if (widget.kind === "button") {
      const id = /^formation_(.+)$/.exec(widget.id)?.[1];
      if (id !== undefined) {
        ids.push(id);
      }
    }
  }
  return ids;
}

function clickOn(layout: ReturnType<typeof formation.layout>, id: string) {
  const w = hit.find(layout, id);
  expect(w, `missing widget ${id}`).not.toBeNull();
  const rect = w?.rect;
  expect(rect).toBeDefined();
  return {
    kind: "click" as const,
    x: (rect?.x ?? 0) + (rect?.w ?? 0) / 2,
    y: (rect?.y ?? 0) + (rect?.h ?? 0) / 2,
  };
}

describe("formation screen", () => {
  it("defaults to the balanced 2-1-1 formation", () => {
    const s = formation.newState(VP, CONTENT);
    expect(s.selected).toBe("2-1-1");
    const selected = hit.find(formation.layout(s), "formation_2-1-1");
    expect(selected?.selected).toBe(true);
  });

  it("offers every authored formation in stable key order", () => {
    const actual = offeredIds(formation.layout(formation.newState(VP, CONTENT)));
    const expected = sortedDataIds(FORMATIONS);

    expect(actual.length).toBe(expected.length);
    expected.forEach((id, i) => {
      expect(actual[i]).toBe(id);
      expect(FORMATIONS[actual[i] ?? ""]).toBeDefined();
    });
  });

  it("discovers a newly authored formation without a screen edit", () => {
    const id = "0-2-2-test";
    const extended: FormationContentData = {
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

    const layout = formation.layout(formation.newState(VP, extended));
    const actual = offeredIds(layout);
    expect(actual[0]).toBe(id);
    expect(hit.find(layout, `formation_${id}`)).not.toBeNull();
    expect(hit.find(layout, `preview_${id}`)).not.toBeNull();
  });

  it("includes a legible anchor preview for every option", () => {
    const layout = formation.layout(formation.newState(VP, CONTENT));
    for (const id of sortedDataIds(FORMATIONS)) {
      const preview = hit.find(layout, `preview_${id}`);
      expect(preview, `missing preview for ${id}`).not.toBeNull();
      expect(preview?.kind).toBe("formation_preview");
      expect(preview?.rect?.w).toBeGreaterThanOrEqual(100);
      expect(preview?.rect?.h).toBeGreaterThanOrEqual(40);
      expect(preview?.data?.keeper).toEqual(FORMATIONS[id]?.keeper);
      expect(preview?.data?.outfield).toEqual(FORMATIONS[id]?.outfield);
      expect(preview?.data?.outfield?.length).toBe(4);
    }
  });

  it("selects the clicked formation", () => {
    const s = formation.newState(VP, CONTENT);
    const [s2] = formation.update(s, clickOn(formation.layout(s), "formation_1-1-2"));
    expect(s2.selected).toBe("1-1-2");
    expect(s.selected, "update should not mutate its input state").toBe("2-1-1");
  });

  it("selects a formation when its shape preview is clicked", () => {
    const s = formation.newState(VP, CONTENT);
    const [s2] = formation.update(s, clickOn(formation.layout(s), "preview_1-2-1"));
    expect(s2.selected).toBe("1-2-1");
  });

  it("emits a tactic transition carrying the selection on Next", () => {
    let s = formation.newState(VP, CONTENT);
    [s] = formation.update(s, clickOn(formation.layout(s), "formation_1-2-1"));
    const [, action] = formation.update(s, clickOn(formation.layout(s), "next"));
    expect(action, "expected a transition action").toBeDefined();
    expect(action?.go).toBe("tactic");
    expect(action?.formation).toBe("1-2-1");
  });

  // Needs `sim.match.new` (Rust-owned, `crates/gc-sim`; no TypeScript bridge
  // exists in this milestone — v2/README.md §1). Re-port once the sim is
  // reachable from TypeScript, e.g. via a wasm binding or a differential
  // fixture generated from the Rust port.
  it.skip("only offers formations accepted by the match simulation", () => {
    // Needs a TypeScript- or wasm-reachable sim.match.new.
  });

  it("does nothing when clicking empty space", () => {
    const s = formation.newState(VP, CONTENT);
    const [, action] = formation.update(s, { kind: "click", x: 5, y: 5 });
    expect(action).toBeUndefined();
  });
});
