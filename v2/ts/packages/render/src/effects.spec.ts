// Ported from spec/render/combat_feedback_spec.lua, the assertions that
// exercise game/render/effects.lua directly (`effects.configure`, `.reset`,
// `.apply_event_diff`, `.diagnostics`, `.confirm_event`,
// `.readability_observation`). The Lua spec also calls `game.match_hud`
// (the HUD *model*, `game/match_hud.lua` -- a different, unported module
// this package does not own; see match_hud.ts's header) via
// `match_hud.layout(viewport)` for its "preserves ball and HUD clearance at
// every supported fixture size" case. That one assertion is `it.skip`ped
// below with the concrete unblocker: porting `game/match_hud.lua`'s model
// (not `game/render/match_hud.lua`, which this package does own).

import { describe, expect, it } from "vitest";
import type { CombatEvent, RollbackWrappedEvent } from "@gc/presentation";
import { effects, type EffectsSettings } from "./effects.ts";
import type { Project } from "./draw2d.ts";

const settings: EffectsSettings = { screen_shake: true, bloom: true };

function combatEvent(id: string, overrides: Partial<CombatEvent> = {}): RollbackWrappedEvent<CombatEvent> {
  return {
    id,
    tick: 1,
    domain: "combat/contact",
    ordinal: 1,
    payload: {
      kind: "contact",
      tick: 1,
      x: 480,
      y: 270,
      family_id: "unarmed",
      result: "hit",
      source_sequence: 1,
      ...overrides,
    },
  };
}

describe("effects (combat feedback)", () => {
  it("reconciles crowded add, replace, revoke, and confirm by stable ID", () => {
    effects.configure(settings);
    effects.reset();
    const a = combatEvent("combat/a");
    const b = combatEvent("combat/b");
    effects.apply_event_diff({ added: [a, b], revoked: [], replaced: [] });
    const added = effects.diagnostics();
    expect(added.speculative_ids).toEqual([a.id, b.id].sort());
    const particleCount = added.particle_count;
    expect(particleCount).toBeGreaterThan(0);

    // Duplicate add is idempotent.
    effects.apply_event_diff({ added: [a, b], revoked: [], replaced: [] });
    expect(effects.diagnostics().particle_count).toBe(particleCount);

    const immune: RollbackWrappedEvent<CombatEvent> = { ...b, id: `${b.id}/immune`, payload: { ...b.payload, result: "immune" } };
    effects.apply_event_diff({ added: [], revoked: [], replaced: [{ before: b, after: immune }] });
    const replaced = effects.diagnostics();
    expect(replaced.speculative_ids).toEqual([a.id, immune.id].sort());
    expect(replaced.particle_count).toBeLessThan(particleCount);

    expect(effects.confirm_event(immune)).toBe(true);
    const confirmedCount = effects.diagnostics().particle_count;
    expect(effects.confirm_event(immune)).toBe(false);
    expect(effects.diagnostics().particle_count).toBe(confirmedCount);

    effects.apply_event_diff({ added: [], revoked: [a], replaced: [] });
    expect(effects.diagnostics().speculative_ids).toEqual([]);
  });

  it("reports crowded geometry without masking the ball or HUD", () => {
    effects.configure(settings);
    effects.reset();
    const events = [combatEvent("combat/x"), combatEvent("combat/y")];
    effects.apply_event_diff({ added: events, revoked: [], replaced: [] });
    const project: Project = (x, y) => [x, y, 1];
    const ball = { x: 0, y: 0 }; // far from the events, so nothing should mask it
    const observation = effects.readability_observation(project, ball, [], []);
    expect(observation.concurrent_event_count).toBe(events.length);
    expect(observation.active_feedback_count).toBe(events.length);
    expect(observation.ball_masked).toBe(false);
    expect(observation.hud_masked).toBe(false);
    expect(observation.occluded_count).toBe(0);
    expect(observation.non_color_only).toBe(true);
  });

  // Requires game/match_hud.lua's `hud.layout(viewport)`, which this package
  // does not own (see file header). Unblocked by porting that module (to
  // whichever package ends up owning game/match_hud.lua's *model*, not this
  // package's game/render/match_hud.lua) and injecting its layout output
  // here the same way match_hud.ts's own spec would.
  it.skip("preserves ball and HUD clearance at every supported fixture size", () => {
    // Blocked on game/match_hud.lua's hud.layout(viewport) -- see above.
  });

  it("reduces flash density and particle travel through existing settings", () => {
    const reduced: EffectsSettings = { screen_shake: false, bloom: false };
    effects.configure(reduced);
    effects.reset();
    effects.apply_event_diff({ added: [combatEvent("combat/z")], revoked: [], replaced: [] });
    expect(effects.diagnostics().particle_count).toBeLessThanOrEqual(4);
    const project: Project = (x, y) => [x, y, 1];
    const observation = effects.readability_observation(project, { x: 0, y: 0 }, [], []);
    expect(observation.active_feedback_count).toBe(1);
    expect(observation.non_color_only).toBe(true);
    effects.configure(settings);
  });
});

describe("effects.sample_ball / tick", () => {
  it("only trails a fast, owned-by-nobody ball, spaced by distance", () => {
    effects.reset();
    const fastLength = { length: () => 200 };
    effects.sample_ball({ ball: { x: 0, y: 0 }, ball_vel: fastLength, ball_z: 0, events: [] });
    expect(effects.diagnostics().trail_count).toBe(1);
    // A tiny move doesn't clear TRAIL_SPACING, so no new sample.
    effects.sample_ball({ ball: { x: 1, y: 0 }, ball_vel: fastLength, ball_z: 0, events: [] });
    expect(effects.diagnostics().trail_count).toBe(1);
    // A slow ball (owned by nobody) does not trail at all.
    effects.reset();
    effects.sample_ball({ ball: { x: 0, y: 0 }, ball_vel: { length: () => 10 }, ball_z: 0, events: [] });
    expect(effects.diagnostics().trail_count).toBe(0);
  });

  it("ages and removes particles/trail dots as dt accumulates", () => {
    effects.reset();
    effects.consume({ ball: { x: 0, y: 0 }, ball_vel: { length: () => 0 }, ball_z: 0, events: [{ kind: "touch", x: 1, y: 1 }] });
    expect(effects.diagnostics().particle_count).toBeGreaterThan(0);
    effects.tick(10); // touch particles live ~0.28s
    expect(effects.diagnostics().particle_count).toBe(0);
  });
});
