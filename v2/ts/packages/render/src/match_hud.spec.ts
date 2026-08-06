// New tests for match_hud.ts. No Lua spec targets game/render/match_hud.lua
// directly with a claimable, self-contained fixture -- see this package's
// port report and effects.spec.ts's header for the related
// combat_feedback_spec.lua case this module is also blocked on.

import { describe, expect, it } from "vitest";
import { matchHudCommands, type MatchHudLayout, type MatchHudModel, type MatchHudTheme } from "./match_hud.ts";

const rect = (x: number, y: number, w: number, h: number) => ({ x, y, w, h });

const layout: MatchHudLayout = {
  venue: rect(230, 7, 500, 14),
  scorebug: rect(230, 24, 500, 48),
  clock: rect(744, 32, 86, 32),
  status: rect(300, 76, 360, 20),
  identity: rect(24, 452, 340, 64),
  plan: rect(696, 468, 240, 44),
  prompt: rect(270, 402, 420, 52),
  announcement: rect(180, 214, 600, 104),
  combat: rect(696, 436, 240, 28),
  scale: 1,
};

const theme: MatchHudTheme = {
  colors: {
    void: [0.015, 0.022, 0.055],
    cyan: [0.25, 0.88, 1.0],
    amber: [1.0, 0.66, 0.24],
    text: [0.91, 0.96, 1.0],
    text_muted: [0.55, 0.67, 0.78],
    panel: [0.065, 0.095, 0.17],
    panel_raised: [0.09, 0.135, 0.23],
    border: [0.18, 0.43, 0.62],
    border_soft: [0.12, 0.27, 0.4],
  },
  radius: 6,
  border_width: 1,
  fonts: { body: 13, eyebrow: 11, title: 24, hero: 38 },
};

const viewport = { w: 1280, h: 720 };

function model(overrides: Partial<MatchHudModel> = {}): MatchHudModel {
  return {
    home_name: "NEBULA",
    away_name: "ORION",
    home_score: 1,
    away_score: 0,
    clock: "4:12",
    venue: "HELIOS CROWN · KAIRON-9 ORBIT",
    possession: "NEBULA POSSESSION",
    possession_marker: "filled",
    player_name: "ZYRO VEX",
    player_detail: "MEDIEVAL FORWARD",
    player_state: "ON BALL",
    species_shape: "round",
    species_color: [1, 0.7, 0.2],
    stamina: 0.8,
    plan: "PLAN · PRESS HIGH · 1-2-1",
    equipment_progress: 0,
    ...overrides,
  };
}

describe("match_hud.matchHudCommands", () => {
  it("draws the score text with an em-dash separator", () => {
    const commands = matchHudCommands(model(), layout, theme, viewport);
    const scoreText = commands.find((c) => c.kind === "text" && c.text.includes("—"));
    expect(scoreText).toBeDefined();
    if (scoreText?.kind === "text") {
      expect(scoreText.text).toBe("1  —  0");
    }
  });

  it("only draws the combat panel when equipment_label is set", () => {
    const without = matchHudCommands(model(), layout, theme, viewport);
    const withEquip = matchHudCommands(model({ equipment_label: "SWORD", equipment_state: "READY", equipment_progress: 1 }), layout, theme, viewport);
    const equipmentText = (cs: typeof without) => cs.filter((c) => c.kind === "text" && c.text.includes("EQUIPMENT"));
    expect(equipmentText(without)).toHaveLength(0);
    expect(equipmentText(withEquip).length).toBeGreaterThan(0);
  });

  it("shows combat feedback text instead of the equipment label, in amber", () => {
    const commands = matchHudCommands(
      model({ equipment_label: "SWORD", equipment_state: "READY", feedback_text: "CLEAN HIT", feedback_glyph: "cross" }),
      layout,
      theme,
      viewport,
    );
    const feedback = commands.find((c) => c.kind === "text" && c.text.startsWith("[CROSS]"));
    expect(feedback).toBeDefined();
    if (feedback?.kind === "text") {
      expect(feedback.color).toEqual(theme.colors.amber);
    }
  });

  it("dims the whole viewport only for a full_time announcement, not a goal/replay one", () => {
    const fullTime = matchHudCommands(model({ announcement_title: "FULL TIME", announcement_kind: "full_time" }), layout, theme, viewport);
    const goal = matchHudCommands(model({ announcement_title: "GOAL", announcement_kind: "goal" }), layout, theme, viewport);
    const dim = (cs: typeof fullTime) => cs.some((c) => c.kind === "rect" && c.w === viewport.w && c.h === viewport.h);
    expect(dim(fullTime)).toBe(true);
    expect(dim(goal)).toBe(false);
  });

  it("fills the stamina bar proportionally and switches to amber when low", () => {
    // The stamina bar sits at identity.y + 46 (see match_hud.ts's `stamina`
    // helper), well below the scorebug's own cyan/amber possession strip.
    const staminaY = layout.identity.y + 46 * layout.scale;
    const inStaminaBand = (c: { kind: string; y?: number }): boolean => c.kind === "rect" && "y" in c && Math.abs((c.y ?? 0) - staminaY) < 1e-9;

    const healthy = matchHudCommands(model({ stamina: 0.8 }), layout, theme, viewport);
    const low = matchHudCommands(model({ stamina: 0.1 }), layout, theme, viewport);
    const healthyFill = healthy.find((c) => inStaminaBand(c) && c.kind === "rect" && c.color === theme.colors.cyan);
    const lowFill = low.find((c) => inStaminaBand(c) && c.kind === "rect" && c.color === theme.colors.amber);
    expect(healthyFill).toBeDefined();
    expect(lowFill).toBeDefined();
    if (healthyFill?.kind === "rect") {
      // Wider than the low-stamina fill, since 0.8 > 0.1 of the same track width.
      const lowFillWidth = lowFill?.kind === "rect" ? lowFill.w : 0;
      expect(healthyFill.w).toBeGreaterThan(lowFillWidth);
    }
  });
});
