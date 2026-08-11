// Tests for combat.ts's telegraph/projectile geometry.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import type { ActionFamilyId, CombatPlayerPresentation, CombatProjectilePresentation } from "@gc/presentation";
import { drawOverCommands, drawUnderCommands, type CombatDrawFrame } from "./combat.ts";
import type { Project } from "./draw2d.ts";

const identityProject: Project = (wx, wy) => [wx, wy, 1];

function player(overrides: Partial<CombatPlayerPresentation>): CombatPlayerPresentation {
  return {
    player_index: 1,
    player_id: "p1",
    phase: "active",
    phase_progress: 0.5,
    phase_ticks: 1,
    cooldown_ticks: 0,
    cooldown_fraction: 0,
    readiness: "committed",
    forced_ticks: 0,
    immunity_ticks: 0,
    position: new Vec2(100, 100),
    direction: new Vec2(1, 0),
    ...overrides,
  };
}

function frameFor(players: readonly CombatPlayerPresentation[], projectiles: readonly CombatProjectilePresentation[] = []): CombatDrawFrame {
  return {
    combat: { enabled: true, players, projectiles },
    players: { x: [100], y: [100] },
  };
}

describe("combat.drawUnderCommands", () => {
  it("draws nothing when the combat model is absent or disabled", () => {
    expect(drawUnderCommands({ players: { x: [], y: [] } }, identityProject)).toEqual([]);
    expect(drawUnderCommands({ combat: { enabled: false, players: [], projectiles: [] }, players: { x: [], y: [] } }, identityProject)).toEqual([]);
  });

  it("draws an unarmed telegraph as the family's cyan-ish colour", () => {
    const commands = drawUnderCommands(frameFor([player({ family_id: "unarmed", front_arc_degrees: 90, telegraph_kind: "arc" })]), identityProject);
    const arcLines = commands.filter((c) => c.kind === "line" && Math.abs((c.lineWidth ?? 0) - 1.8) < 1e-9);
    expect(arcLines.length).toBeGreaterThan(0);
    const first = arcLines[0];
    if (first === undefined) {
      throw new Error("expected an arc segment");
    }
    expect(first.color).toEqual([0.55, 0.95, 1]);
  });

  it("draws a guard fan with a thicker line than an unarmed/melee arc", () => {
    const commands = drawUnderCommands(frameFor([player({ family_id: "guard", front_arc_degrees: 120, telegraph_kind: "guard_arc" })]), identityProject);
    const guardLines = commands.filter((c) => c.kind === "line" && Math.abs((c.lineWidth ?? 0) - 2.5) < 1e-9);
    expect(guardLines.length).toBeGreaterThan(0);
  });

  it("draws a ranged telegraph as a straight sightline to the projectile range", () => {
    const commands = drawUnderCommands(
      frameFor([player({ family_id: "ranged", telegraph_kind: "line", direction: new Vec2(1, 0), projectile_range_px: 300, phase: "active" })]),
      identityProject,
    );
    const main = commands[0];
    if (main === undefined || main.kind !== "line") {
      throw new Error("expected the ranged sightline first");
    }
    expect(main.points).toEqual([100, 100, 400, 100]);
    expect(main.lineWidth).toBe(3); // active phase widens the line
    expect(main.color).toEqual([1, 0.45, 0.78]);
  });

  it("throws rather than silently misdrawing when a family_id is unknown", () => {
    const badFamilyId = "not_a_real_family" as unknown as ActionFamilyId;
    expect(() => drawUnderCommands(frameFor([player({ family_id: badFamilyId, front_arc_degrees: 90, telegraph_kind: "arc" })]), identityProject)).toThrow();
  });
});

describe("combat.drawOverCommands", () => {
  it("draws a projectile trail, a diamond head, and a dark core, in that order", () => {
    const projectile: CombatProjectilePresentation = {
      family_id: "ranged",
      source_index: 1,
      source_sequence: 1,
      position: new Vec2(50, 50),
      direction: new Vec2(1, 0),
      remaining_ticks: 10,
    };
    const commands = drawOverCommands(frameFor([], [projectile]), identityProject);
    expect(commands.map((c) => c.kind)).toEqual(["line", "polygon", "circle"]);
    const polygon = commands[1];
    if (polygon === undefined || polygon.kind !== "polygon") {
      throw new Error("expected the projectile diamond");
    }
    expect(polygon.mode).toBe("fill");
    expect(polygon.color).toEqual([1, 0.9, 1]);
  });
});
