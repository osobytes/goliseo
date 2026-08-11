// Ported from spec/game/combat_presentation_spec.lua.
//
// The Lua spec file has two `describe` blocks. Only "combat presentation
// projection" is ported here — it is the one that actually exercises
// `game/presentation/combat.lua`. The second block, "shared player pose
// priority", exercises `render/player_pose.lua`, which the v2 file mapping
// (v2/README.md §2, the `render/**` row) assigns to Rust (`crates/gc-render`),
// not `@gc/presentation`. There is no TypeScript home for that module in
// this milestone, so that block is intentionally left unported here; it
// belongs with whoever ports `render/player_pose.lua`. See the report back
// to the orchestrator for the same note.
//
// Fixture note: the Lua spec builds its `MatchState`/`CombatMatchState`
// fixture via `sim.match.new` + `sim.combat.new_state` (both Rust-owned,
// `crates/gc-sim`), and reads `data.teams`/`data.loadouts`/etc (Rust-owned,
// `crates/gc-data`). None of that exists in TypeScript. `combat.model` is a
// pure projection of whatever `MatchState`/`CombatMatchState` it is handed,
// so the fixture below is a self-consistent synthetic stand-in that
// exercises the exact same code paths — real `data/action_families.lua` and
// `data/loadouts.lua`/`data/equipment_presentations.lua` values are used
// verbatim (see those files) so numeric assertions (e.g. projectile range)
// match the real content. The purity assertion, which used
// `sim.match_snapshot.hash`, is expressed instead as a structural
// before/after snapshot of the inputs — there is no TypeScript
// `match_snapshot` to hash against, but "the inputs are untouched" is
// exactly what that assertion was checking.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import {
  combat,
  type ActionFamilyId,
  type CombatMatchState,
  type CombatPlayerState,
  type CombatPresentationData,
  type MatchState,
} from "./combat.ts";

const ACTION_FAMILIES: CombatPresentationData["action_families"] = {
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
  guard: {
    id: "guard",
    name: "Guard",
    windup_ticks: 6,
    recovery_ticks: 9,
    cooldown_ticks: 0,
    front_arc_degrees: 120,
  },
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
};

const EQUIPMENT_PRESENTATIONS: CombatPresentationData["equipment_presentations"] = {
  toy_spring_gloves: {
    id: "toy_spring_gloves",
    name: "Spring Gloves",
    family_id: "unarmed",
    attachment: "both_hands",
  },
  medieval_heater_shield: {
    id: "medieval_heater_shield",
    name: "Emberguard Shield",
    family_id: "guard",
    attachment: "left_hand",
  },
  medieval_tournament_sword: {
    id: "medieval_tournament_sword",
    name: "Tournament Sword",
    family_id: "light_melee",
    attachment: "right_hand",
  },
  scifi_pulse_blaster: {
    id: "scifi_pulse_blaster",
    name: "Pulse Blaster",
    family_id: "ranged",
    attachment: "right_hand",
  },
};

const LOADOUTS: CombatPresentationData["loadouts"] = {
  loadout_spring_gloves: {
    id: "loadout_spring_gloves",
    family_id: "unarmed",
    equipment_presentation_id: "toy_spring_gloves",
  },
  loadout_emberguard_shield: {
    id: "loadout_emberguard_shield",
    family_id: "guard",
    equipment_presentation_id: "medieval_heater_shield",
  },
  loadout_tournament_sword: {
    id: "loadout_tournament_sword",
    family_id: "light_melee",
    equipment_presentation_id: "medieval_tournament_sword",
  },
  loadout_pulse_blaster: {
    id: "loadout_pulse_blaster",
    family_id: "ranged",
    equipment_presentation_id: "scifi_pulse_blaster",
  },
};

const DATA: CombatPresentationData = {
  action_families: ACTION_FAMILIES,
  equipment_presentations: EQUIPMENT_PRESENTATIONS,
  loadouts: LOADOUTS,
};

function defaultRuntime(familyId: ActionFamilyId | undefined, loadoutId: string | undefined): CombatPlayerState {
  return {
    phase: "ready",
    phase_ticks: 0,
    cooldown_ticks: 0,
    forced_ticks: 0,
    immunity_ticks: 0,
    ...(familyId !== undefined ? { family_id: familyId } : {}),
    ...(loadoutId !== undefined ? { loadout_id: loadoutId } : {}),
  };
}

/** Ten players (index 1 a keeper with no loadout), covering all four families. */
function fixture(): { state: MatchState; combatState: CombatMatchState } {
  const ids = [
    "home_gk",
    "home_def_1",
    "home_def_2",
    "home_mid_1",
    "home_fwd_1",
    "away_gk",
    "away_def_1",
    "away_def_2",
    "away_mid_1",
    "away_fwd_1",
  ];
  const families: (ActionFamilyId | undefined)[] = [
    undefined,
    "unarmed",
    "guard",
    "light_melee",
    "ranged",
    undefined,
    "unarmed",
    "guard",
    "light_melee",
    "ranged",
  ];
  const loadoutByFamily: Record<ActionFamilyId, string> = {
    unarmed: "loadout_spring_gloves",
    guard: "loadout_emberguard_shield",
    light_melee: "loadout_tournament_sword",
    ranged: "loadout_pulse_blaster",
  };

  const state: MatchState = {
    players: ids.map((id, index) => ({
      id,
      pos: new Vec2(index * 40, 100 + index * 10),
      facing: new Vec2(index < 5 ? 1 : -1, 0),
    })),
  };
  const combatState: CombatMatchState = {
    tick: 0,
    player_ids: ids,
    players: families.map((familyId) =>
      defaultRuntime(familyId, familyId !== undefined ? loadoutByFamily[familyId] : undefined)
    ),
    projectiles: [],
  };
  return { state, combatState };
}

function findFamilyIndices(combatState: CombatMatchState): Partial<Record<ActionFamilyId, number>> {
  const indices: Partial<Record<ActionFamilyId, number>> = {};
  combatState.players.forEach((runtime, i) => {
    if (runtime.family_id !== undefined && indices[runtime.family_id] === undefined) {
      indices[runtime.family_id] = i + 1;
    }
  });
  return indices;
}

describe("combat presentation projection", () => {
  it("projects fixed loadouts, phases, readiness, forced state, and projectiles", () => {
    const { state, combatState: base } = fixture();
    const familyIndices = findFamilyIndices(base);
    const unarmedIndex = familyIndices.unarmed;
    const guardIndex = familyIndices.guard;
    const meleeIndex = familyIndices.light_melee;
    const rangedIndex = familyIndices.ranged;
    expect(unarmedIndex).toBeDefined();
    expect(guardIndex).toBeDefined();
    expect(meleeIndex).toBeDefined();
    expect(rangedIndex).toBeDefined();
    const unarmedI = unarmedIndex as number;
    const guardI = guardIndex as number;
    const meleeI = meleeIndex as number;
    const rangedI = rangedIndex as number;

    const players = base.players.map((runtime, i) => {
      const playerIndex = i + 1;
      if (playerIndex === unarmedI) {
        return { ...runtime, phase: "windup" as const, phase_ticks: 3, cooldown_ticks: 21 };
      }
      if (playerIndex === guardI) {
        return { ...runtime, phase: "guard" as const };
      }
      if (playerIndex === meleeI) {
        return { ...runtime, forced_state: "knockback" as const, forced_ticks: 7 };
      }
      if (playerIndex === rangedI) {
        return { ...runtime, phase: "aim" as const, cooldown_ticks: 41 };
      }
      return runtime;
    });
    const combatState: CombatMatchState = {
      ...base,
      players,
      projectiles: [
        {
          family_id: "ranged",
          source_index: rangedI,
          source_sequence: 12,
          pos: new Vec2(400, 260),
          dir: new Vec2(1, 0),
          remaining_ticks: 44,
        },
      ],
    };

    const model = combat.model(state, combatState, DATA);
    expect(model.enabled).toBe(true);
    expect(model.tick).toBe(0);
    expect(model.players).toHaveLength(10);
    expect(model.projectiles).toHaveLength(1);
    expect(model.players[unarmedI - 1]?.telegraph_kind).toBe("arc");
    expect(model.players[guardI - 1]?.telegraph_kind).toBe("guard_arc");
    expect(model.players[rangedI - 1]?.telegraph_kind).toBe("line");
    expect(model.players[meleeI - 1]?.readiness).toBe("forced");
    expect(model.players[meleeI - 1]?.forced_state).toBe("knockback");
    expect(model.projectiles[0]?.source_sequence).toBe(12);
    expect(model.players[rangedI - 1]?.projectile_range_px).toBeCloseTo(300, 9);
    expect(model.players[0]?.readiness).toBe("unavailable");
    expect(model.players[0]?.equipment_presentation_id).toBeUndefined();
  });

  it("is pure and keeps presentation identity outside simulation hashes", () => {
    const { state, combatState } = fixture();
    const before = JSON.stringify({ state, combatState });
    const first = combat.model(state, combatState, DATA);
    const second = combat.model(state, combatState, DATA);
    const after = JSON.stringify({ state, combatState });
    expect(after).toBe(before);
    expect(first.players[1]?.equipment_presentation_id).toBe(second.players[1]?.equipment_presentation_id);
    expect(combat.model(state, null, DATA).enabled).toBe(false);
  });

  it("maps authoritative event records to stable semantic presentation ids", () => {
    const contact = combat.event(
      {
        kind: "contact",
        tick: 8,
        family_id: "guard",
        source_index: 2,
        target_index: 7,
        source_sequence: 4,
        result: "guarded",
        x: 300,
        y: 240,
      },
      "combat/8/4/contact"
    );
    expect(contact.semantic_id).toBe("combat.contact.guarded");
    expect(contact.stable_id).toBe("combat/8/4/contact");

    const spawn = combat.event({
      kind: "projectile_spawn",
      tick: 9,
      family_id: "ranged",
      source_index: 4,
      source_sequence: 5,
      x: 320,
      y: 240,
    });
    expect(spawn.semantic_id).toBe("combat.projectile.spawn");
  });
});
