// Fixture note: `gc-data`'s three tactics are transcribed verbatim
// (content.ts's header explains why this package receives rather than
// imports that content), but only the fields `tactic.ts` reads
// (id/name/strength/risk) — `gc-data`'s marking/transition/press tuning
// knobs are sim-only and never reach the screen.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { tactic, type TacticContentData } from "./tactic.ts";
import type { TacticData } from "./content.ts";

const VP = { w: 960, h: 540 };

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

const CONTENT: TacticContentData = { tactics: TACTICS };

function clickOn(layout: ReturnType<typeof tactic.layout>, id: string) {
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

describe("tactic screen", () => {
  it("defaults to balanced and offers all three tactics", () => {
    const s = tactic.newState(VP, CONTENT);
    expect(s.selected).toBe("balanced");
    const layout = tactic.layout(s);
    expect(hit.find(layout, "tactic_balanced")).not.toBeNull();
    expect(hit.find(layout, "tactic_press_high")).not.toBeNull();
    expect(hit.find(layout, "tactic_counter")).not.toBeNull();
  });

  it("selects the clicked tactic", () => {
    const s = tactic.newState(VP, CONTENT);
    const [s2] = tactic.update(s, clickOn(tactic.layout(s), "tactic_press_high"));
    expect(s2.selected).toBe("press_high");
  });

  it("emits a match transition carrying the tactic on Kick Off", () => {
    let s = tactic.newState(VP, CONTENT);
    [s] = tactic.update(s, clickOn(tactic.layout(s), "tactic_counter"));
    const [, action] = tactic.update(s, clickOn(tactic.layout(s), "kickoff"));
    expect(action, "expected a transition action").toBeDefined();
    expect(action?.go).toBe("match");
    expect(action?.tactic).toBe("counter");
  });
});
