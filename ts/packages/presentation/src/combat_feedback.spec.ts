// Fixture note: `combat_feedback`'s functions never take a `MatchState`/
// `CombatMatchState` as an argument in the first place — they operate only
// on detached `CombatEvent` records and this module's own
// `CombatFeedbackState`. So "never mutates simulation truth" is checked
// with an explicit structural before/after snapshot of a synthetic
// sim-shaped fixture, rather than a hash comparison.
//
// The "same-family presentation swap" test passes two different data
// objects rather than mutating one `CombatPresentationData` in place and
// restoring it afterward. Since `combat.model` takes its data tables as an
// explicit parameter (see combat.ts's boundary note) rather than a
// captured global, this makes "a cosmetic swap cannot touch sim state" true
// by construction rather than by convention.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import {
  combat,
  type CombatContactResult,
  type CombatEvent,
  type CombatEventKind,
  type CombatMatchState,
  type CombatPresentationData,
  type MatchState,
} from "./combat.ts";
import { combatFeedback } from "./combat_feedback.ts";

function event(kind: CombatEventKind, result?: CombatContactResult): CombatEvent {
  return {
    kind,
    tick: 12,
    family_id: kind === "guard_recoil" ? "guard" : "light_melee",
    source_index: 2,
    // `target_index` is intentionally omitted here for "commit" and
    // "projectile_spawn": no assertion in this spec reads it for those
    // kinds, so this fixture uses the clearly-intended value (absent)
    // rather than a stray placeholder in a `number?` field.
    ...(kind !== "commit" && kind !== "projectile_spawn" ? { target_index: 7 } : {}),
    source_sequence: 4,
    ...(result !== undefined ? { result } : {}),
    x: 320,
    y: 240,
  };
}

describe("combat feedback presentation contract", () => {
  it("gives every supported authoritative result a deliberate multimodal disposition", () => {
    const fixtures: CombatEvent[] = [
      event("commit"),
      event("projectile_spawn"),
      event("projectile_expire"),
      event("contact", "hit"),
      event("contact", "extended"),
      event("contact", "guarded"),
      event("contact", "immune"),
      event("contact", "superseded"),
      event("ball_spill"),
      event("forced"),
      event("guard_recoil"),
    ];
    const semanticIds = new Set<string>();
    fixtures.forEach((value, index) => {
      const link = combatFeedback.accepted(value, `accepted/${index + 1}`);
      expect(link.link_kind).toBe("accepted_encounter");
      expect(link.source_sequence).toBe(4);
      expect(link.feedback_event_id).toBe(`accepted/${index + 1}`);
      expect(link.disposition.vfx).not.toBe("none");
      expect(link.disposition.audio).not.toBeUndefined();
      expect(link.disposition.glyph).not.toBe("");
      semanticIds.add(link.semantic_id);
    });
    expect(fixtures).toHaveLength(11);
    expect(semanticIds.size).toBe(11);
  });

  it("keeps accepted, rejected, and missing-feedback linkage disjoint", () => {
    const accepted = combatFeedback.accepted(event("contact", "hit"), "accepted/contact");
    const rejected = combatFeedback.rejectedRequest("request/rejected", 14, 2, "soccer_commitment");
    const missing = combatFeedback.missingFeedback("request/missing", 15, 2, "missing_press_edge");

    expect(accepted.link_kind).toBe("accepted_encounter");
    expect(accepted.source_sequence).toBe(4);
    expect(rejected.link_kind).toBe("rejected_request");
    expect(rejected.source_sequence).toBeUndefined();
    expect(rejected.reason).toBe("soccer_commitment");
    expect(rejected.feedback_event_id).toBe("request/rejected");
    expect(rejected.disposition.vfx).toBe("rejected");
    expect(missing.link_kind).toBe("missing_feedback");
    expect(missing.source_sequence).toBeUndefined();
    expect(missing.feedback_event_id).toBeUndefined();
    expect(missing.disposition.vfx).toBe("none");
    expect(missing.disposition.audio).toBeUndefined();
  });

  it("deduplicates confirmed HUD/camera feedback and bounds reduced-motion behavior", () => {
    const state = combatFeedback.new();
    const link = combatFeedback.accepted(event("contact", "hit"), "confirmed/hit");
    expect(combatFeedback.confirm(state, link)).toBe(true);
    expect(combatFeedback.confirm(state, link)).toBe(false);
    expect(combatFeedback.diagnostics(state).confirmed_count).toBe(1);
    expect(combatFeedback.notice(state, 2)?.text).toBe("CLEAN HIT");
    expect(combatFeedback.notice(state, 5)).toBeNull();
    const offset = combatFeedback.cameraOffset(state);
    expect(Math.abs(offset.x)).toBeLessThanOrEqual(4);
    expect(Math.abs(offset.y)).toBeLessThanOrEqual(4);

    let observation = combatFeedback.observation(state, link, "confirmed");
    expect(observation.camera_enabled).toBe(true);
    expect(observation.cue_state).toBe("confirmed");

    combatFeedback.configure(state, true, true);
    const reducedOffset = combatFeedback.cameraOffset(state);
    expect(reducedOffset.x).toBe(0);
    expect(reducedOffset.y).toBe(0);
    observation = combatFeedback.observation(state, link, "confirmed");
    expect(observation.camera_enabled).toBe(false);
    expect(observation.reduced_motion).toBe(true);
    expect(observation.reduced_flash).toBe(true);

    combatFeedback.tick(state, 1);
    expect(combatFeedback.notice(state, 2)).toBeNull();
  });

  it("never mutates simulation truth while projecting feedback", () => {
    // See file header: this module's functions never take sim state as an
    // argument, so a structural snapshot substitutes for a hash comparison
    // here.
    const state: MatchState = {
      players: [{ id: "home_mid_1", pos: new Vec2(100, 200), facing: new Vec2(1, 0) }],
    };
    const combatState: CombatMatchState = {
      tick: 0,
      player_ids: ["home_mid_1"],
      players: [
        {
          phase: "ready",
          phase_ticks: 0,
          cooldown_ticks: 0,
          forced_ticks: 0,
          immunity_ticks: 0,
          family_id: "light_melee",
          loadout_id: "loadout_tournament_sword",
        },
      ],
      projectiles: [],
    };
    const before = JSON.stringify({ state, combatState });

    const link = combatFeedback.accepted(event("contact", "guarded"), "pure/guarded");
    const feedbackState = combatFeedback.new();
    combatFeedback.confirm(feedbackState, link);
    combatFeedback.tick(feedbackState, 1 / 60);
    combatFeedback.observation(feedbackState, link, "confirmed");

    const after = JSON.stringify({ state, combatState });
    expect(after).toBe(before);
  });

  it("keeps simulation identity stable across a same-family presentation swap", () => {
    const state: MatchState = {
      players: [{ id: "home_mid_1", pos: new Vec2(100, 200), facing: new Vec2(1, 0) }],
    };
    const combatState: CombatMatchState = {
      tick: 0,
      player_ids: ["home_mid_1"],
      players: [
        {
          phase: "ready",
          phase_ticks: 0,
          cooldown_ticks: 0,
          forced_ticks: 0,
          immunity_ticks: 0,
          family_id: "light_melee",
          loadout_id: "loadout_light_melee_test",
        },
      ],
      projectiles: [],
    };
    const before = JSON.stringify({ state, combatState });

    function dataWith(equipmentPresentationId: string): CombatPresentationData {
      return {
        action_families: {
          unarmed: {
            id: "unarmed",
            name: "Unarmed",
            windup_ticks: 6,
            active_ticks: 4,
            recovery_ticks: 12,
            cooldown_ticks: 24,
            reach_px: 30,
            front_arc_degrees: 100,
          },
          guard: { id: "guard", name: "Guard", windup_ticks: 6, recovery_ticks: 9, cooldown_ticks: 0, front_arc_degrees: 120 },
          light_melee: {
            id: "light_melee",
            name: "Light Melee",
            windup_ticks: 12,
            active_ticks: 5,
            recovery_ticks: 21,
            cooldown_ticks: 42,
            reach_px: 42,
            front_arc_degrees: 75,
          },
          ranged: {
            id: "ranged",
            name: "Ranged",
            windup_ticks: 18,
            active_ticks: 1,
            recovery_ticks: 27,
            cooldown_ticks: 60,
            projectile_speed_px_per_second: 300,
            projectile_lifetime_ticks: 60,
            front_arc_degrees: 20,
          },
        },
        equipment_presentations: {
          toy_foam_sword: { id: "toy_foam_sword", name: "Foam Champion", family_id: "light_melee", attachment: "right_hand" },
          scifi_energy_blade: {
            id: "scifi_energy_blade",
            name: "Vector Blade",
            family_id: "light_melee",
            attachment: "right_hand",
          },
        },
        loadouts: {
          loadout_light_melee_test: {
            id: "loadout_light_melee_test",
            family_id: "light_melee",
            equipment_presentation_id: equipmentPresentationId,
          },
        },
      };
    }

    const beforeName = combat.model(state, combatState, dataWith("toy_foam_sword")).players[0]?.equipment_name;
    expect(beforeName).toBeDefined();
    const swappedName = combat.model(state, combatState, dataWith("scifi_energy_blade")).players[0]?.equipment_name;
    expect(swappedName).toBeDefined();

    const after = JSON.stringify({ state, combatState });
    expect(swappedName).not.toBe(beforeName);
    expect(after).toBe(before);
  });
});
