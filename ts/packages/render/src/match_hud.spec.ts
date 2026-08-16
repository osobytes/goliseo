// Tests for match_hud.ts.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  drawMatchHud,
  matchHudCommands,
  type MatchHudLayout,
  type MatchHudModel,
  type MatchHudTheme,
} from "./match_hud.ts";
import type { TextCommand } from "./draw2d.ts";

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
    zenith: [0.01, 0.008, 0.03],
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
    const withEquip = matchHudCommands(
      model({ equipment_label: "SWORD", equipment_state: "READY", equipment_progress: 1 }),
      layout,
      theme,
      viewport,
    );
    const equipmentText = (cs: typeof without) =>
      cs.filter((c) => c.kind === "text" && c.text.includes("EQUIPMENT"));
    expect(equipmentText(without)).toHaveLength(0);
    expect(equipmentText(withEquip).length).toBeGreaterThan(0);
  });

  it("shows combat feedback text instead of the equipment label, in amber", () => {
    const commands = matchHudCommands(
      model({
        equipment_label: "SWORD",
        equipment_state: "READY",
        feedback_text: "CLEAN HIT",
        feedback_glyph: "cross",
      }),
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
    const fullTime = matchHudCommands(
      model({ announcement_title: "FULL TIME", announcement_kind: "full_time" }),
      layout,
      theme,
      viewport,
    );
    const goal = matchHudCommands(
      model({ announcement_title: "GOAL", announcement_kind: "goal" }),
      layout,
      theme,
      viewport,
    );
    const dim = (cs: typeof fullTime) =>
      cs.some((c) => c.kind === "rect" && c.w === viewport.w && c.h === viewport.h);
    expect(dim(fullTime)).toBe(true);
    expect(dim(goal)).toBe(false);
  });

  it("fills the stamina bar proportionally and switches to amber when low", () => {
    // The stamina bar sits at identity.y + 46 (see match_hud.ts's `stamina`
    // helper), well below the scorebug's own cyan/amber possession strip.
    const staminaY = layout.identity.y + 46 * layout.scale;
    const inStaminaBand = (c: { kind: string; y?: number }): boolean =>
      c.kind === "rect" && "y" in c && Math.abs((c.y ?? 0) - staminaY) < 1e-9;

    const healthy = matchHudCommands(model({ stamina: 0.8 }), layout, theme, viewport);
    const low = matchHudCommands(model({ stamina: 0.1 }), layout, theme, viewport);
    const healthyFill = healthy.find(
      (c) => inStaminaBand(c) && c.kind === "rect" && c.color === theme.colors.cyan,
    );
    const lowFill = low.find(
      (c) => inStaminaBand(c) && c.kind === "rect" && c.color === theme.colors.amber,
    );
    expect(healthyFill).toBeDefined();
    expect(lowFill).toBeDefined();
    if (healthyFill?.kind === "rect") {
      // Wider than the low-stamina fill, since 0.8 > 0.1 of the same track width.
      const lowFillWidth = lowFill?.kind === "rect" ? lowFill.w : 0;
      expect(healthyFill.w).toBeGreaterThan(lowFillWidth);
    }
  });
});

// HUD POPULATION (drawMatchHud -> draw2d.ts's `paint`). `matchHudCommands`
// above is pure and needs nothing GPU/DOM-shaped to test; `drawMatchHud`
// additionally has to turn every one of those commands into a real three.js
// object and land it in a `THREE.Group`, and `drawMatchHud` *always* emits at
// least a venue `dl.text(...)` command, whose default build path
// (draw2d.ts's `buildTextSprite`) calls `document.createElement("canvas")` --
// unavailable under this workspace's default `vitest` "node" environment
// (ts/vitest.config.ts has no per-file environment overrides, and no
// jsdom/happy-dom is installed in this workspace to request one via a
// `// @vitest-environment` docblock).
//
// Rather than add a DOM dependency this milestone does not otherwise need,
// this suite uses the seam draw2d.ts's `paint`/`drawMatchHud` already expose
// for exactly this: `PaintOptions.buildText` substitutes the text command's
// three.js object with a plain stand-in, so every OTHER command kind the HUD
// actually emits (rect, polygon, line -- see match_hud.ts, no circle/ellipse/
// arc in this module) goes through draw2d.ts's real, unmodified builders.
// That is real coverage of HUD population, not a mock of it: the only thing
// substituted is the one code path that is DOM-shaped rather than
// GL-shaped, and it is substituted with a real (if trivial) `THREE.Object3D`
// so the group's child count and disposal behavior stay meaningful.
describe("match_hud.drawMatchHud (population)", () => {
  it("adds exactly one three.js object per DrawCommand, in order, without touching document", () => {
    const group = new THREE.Group();
    const m = model();
    const commands = matchHudCommands(m, layout, theme, viewport);
    let textCalls = 0;
    drawMatchHud(group, m, layout, theme, viewport, {
      buildText: (_c: TextCommand) => {
        textCalls += 1;
        return new THREE.Object3D();
      },
    });
    expect(group.children).toHaveLength(commands.length);
    expect(textCalls).toBe(commands.filter((c) => c.kind === "text").length);
  });

  it("repopulating the group clears the previous frame's objects instead of accumulating them", () => {
    const group = new THREE.Group();
    const buildText = (): THREE.Object3D => new THREE.Object3D();
    drawMatchHud(group, model(), layout, theme, viewport, { buildText });
    const first = group.children.length;
    expect(first).toBeGreaterThan(0);

    // A model with the combat panel active emits strictly more commands.
    const withEquip = model({
      equipment_label: "SWORD",
      equipment_state: "READY",
      equipment_progress: 0.5,
    });
    drawMatchHud(group, withEquip, layout, theme, viewport, { buildText });
    const expected = matchHudCommands(withEquip, layout, theme, viewport).length;
    expect(group.children).toHaveLength(expected);
    expect(group.children.length).not.toBe(first + expected); // would be true if paint() had accumulated instead of clearing
  });

  // Materials deliberately NOT disposed here anymore -- they are shared and
  // cached by draw2d.ts (see its SHARED MATERIALS section and #403: disposing
  // them destroyed and recompiled five GL programs per frame). Geometry is
  // still per-frame, and this asserts both halves of that split so a
  // regression in either direction is loud.
  it("disposes each rebuilt frame's mesh geometry, and REUSES its shared material", () => {
    const group = new THREE.Group();
    const buildText = (): THREE.Object3D => new THREE.Object3D();
    drawMatchHud(group, model(), layout, theme, viewport, { buildText });

    const mesh = group.children.find((c): c is THREE.Mesh => c instanceof THREE.Mesh);
    expect(mesh).toBeDefined();
    let geometryDisposed = false;
    let materialDisposed = false;
    let firstMaterial: THREE.Material | undefined;
    if (mesh !== undefined) {
      mesh.geometry.dispose = () => {
        geometryDisposed = true;
      };
      const material = mesh.material;
      if (!Array.isArray(material)) {
        firstMaterial = material;
        material.dispose = () => {
          materialDisposed = true;
        };
      }
    }

    drawMatchHud(group, model(), layout, theme, viewport, { buildText });

    expect(geometryDisposed).toBe(true);
    expect(materialDisposed).toBe(false);
    const next = group.children.find((c): c is THREE.Mesh => c instanceof THREE.Mesh);
    expect(next).toBeDefined();
    expect(next?.material).toBe(firstMaterial);
  });
});
