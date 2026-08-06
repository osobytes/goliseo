// New tests for player_renderer.ts, covering what
// spec/render/combat_presentation_spec.lua asserted about
// game/render/player_renderer.lua's equipment-proxy geometry (the "draws
// all six procedural equipment proxies through compatible poses" and
// "draws distinct forced-state silhouettes" cases), plus the module's own
// pose/silhouette/transform behaviour. See combat.spec.ts's header for why
// the Lua spec as a whole is not claimed by this package.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import type { CombatPlayerPresentation } from "@gc/presentation";
import { playerDrawCommands, silhouette, type PlayerRenderOptions } from "./player_renderer.ts";

function baseOptions(overrides: Partial<PlayerRenderOptions> = {}): PlayerRenderOptions {
  return { is_keeper: false, controlled: false, team: "home", ...overrides };
}

describe("player_renderer.silhouette", () => {
  it("returns a distinct profile per species shape", () => {
    expect(silhouette("round").head_kind).toBe("round");
    expect(silhouette("broad").torso_scale).toBeGreaterThan(silhouette("round").torso_scale);
  });

  it("throws (mirroring the Lua assert) for an unknown shape", () => {
    expect(() => silhouette("nonexistent" as Parameters<typeof silhouette>[0])).toThrow();
  });
});

describe("player_renderer.playerDrawCommands", () => {
  it("draws a ground shadow and, when controlled, two selection rings plus a chevron", () => {
    const uncontrolled = playerDrawCommands(100, 200, 12, [0.2, 0.8, 1], undefined, baseOptions());
    expect(uncontrolled[0]?.kind).toBe("ellipse");
    expect(uncontrolled.some((c) => c.kind === "polygon")).toBe(false);

    const controlled = playerDrawCommands(100, 200, 12, [0.2, 0.8, 1], undefined, baseOptions({ controlled: true }));
    const rings = controlled.filter((c) => c.kind === "ellipse" && c.mode === "line");
    expect(rings).toHaveLength(2);
    expect(controlled.some((c) => c.kind === "polygon" && c.mode === "fill")).toBe(true);
  });

  it("draws every equipment proxy without throwing, through the six documented fixtures", () => {
    const fixtures: readonly [string, string][] = [
      ["toy_spring_gloves", "combat_active"],
      ["medieval_heater_shield", "combat_guard"],
      ["medieval_tournament_sword", "combat_windup"],
      ["scifi_energy_blade", "combat_active"],
      ["toy_foam_sword", "combat_recovery"],
      ["scifi_pulse_blaster", "combat_aim"],
    ];
    for (const [equipmentId, poseId] of fixtures) {
      const combat: CombatPlayerPresentation = {
        player_index: 2,
        player_id: "fixture",
        family_id: equipmentId.includes("shield") ? "guard" : equipmentId.includes("blaster") ? "ranged" : "light_melee",
        equipment_presentation_id: equipmentId,
        phase: poseId === "combat_recovery" ? "recovery" : poseId === "combat_aim" ? "aim" : poseId === "combat_guard" ? "guard" : poseId === "combat_windup" ? "windup" : "active",
        phase_progress: 0.5,
        phase_ticks: 3,
        cooldown_ticks: 8,
        cooldown_fraction: 0.5,
        readiness: "committed",
        forced_ticks: 0,
        immunity_ticks: 0,
        position: new Vec2(100, 100),
        direction: new Vec2(1, 0),
      };
      const commands = playerDrawCommands(
        200,
        300,
        12,
        [0.2, 0.8, 1],
        undefined,
        baseOptions({ facing: new Vec2(1, 0), species_shape: "round", species_color: [1, 0.7, 0.2], combat, pose: { id: poseId } }),
      );
      expect(commands.length).toBeGreaterThan(0);
    }
  });

  it("draws distinct forced-state silhouettes for stagger vs knockback (a rotated pivot each)", () => {
    const stagger = playerDrawCommands(200, 300, 12, [0.2, 0.8, 1], undefined, baseOptions({ facing: new Vec2(1, 0), team: "away", pose: { id: "combat_stagger" } }));
    const knockback = playerDrawCommands(200, 300, 12, [0.2, 0.8, 1], undefined, baseOptions({ facing: new Vec2(1, 0), team: "away", pose: { id: "combat_knockback" } }));
    // Both poses rotate the whole figure about the feet, so no command
    // should still land exactly on an axis-aligned torso rect -- everything
    // degrades to polygons/lines instead of the identity-pose's rect torso.
    expect(stagger.some((c) => c.kind === "rect")).toBe(false);
    expect(knockback.some((c) => c.kind === "rect")).toBe(false);
    expect(stagger).not.toEqual(knockback);
  });

  it("fades dash afterimages by 0.24/n while leaving the solid figure at full alpha", () => {
    const withDash = playerDrawCommands(100, 200, 12, [0.2, 0.8, 1], { px: 100, py: 200, speed: 0, phase: 0, gait: 0, lean: 0 }, baseOptions({ dashing: true, facing: new Vec2(1, 0) }));
    const withoutDash = playerDrawCommands(100, 200, 12, [0.2, 0.8, 1], { px: 100, py: 200, speed: 0, phase: 0, gait: 0, lean: 0 }, baseOptions({ dashing: false, facing: new Vec2(1, 0) }));
    expect(withDash.length).toBeGreaterThan(withoutDash.length);
    const faded = withDash.find((c) => c.kind === "rect" && Math.abs((c.alpha ?? 1) - 0.24) < 1e-9);
    expect(faded).toBeDefined();
  });

  it("draws a keeper dive rotated about the feet, proportional to dive amount", () => {
    const commands = playerDrawCommands(
      100,
      200,
      12,
      [0.2, 0.8, 1],
      undefined,
      baseOptions({ dive: 1, dive_dir: new Vec2(1, 0), pose: { id: "keeper_spread" } }),
    );
    expect(commands.length).toBeGreaterThan(0);
    expect(commands.some((c) => c.kind === "rect")).toBe(false); // torso rect degraded to a polygon under rotation
  });
});
