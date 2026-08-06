// Ported from spec/game/match_hud_spec.lua.
//
// The Lua spec drives `hud.model` from `render_frame.hud(match_sim.new(...))`
// -- a real simulation state run through the Rust-owned RenderFrame producer
// (`render/frame.lua` -> `crates/gc-render`; v2/README.md §2) -- and from
// `combat_presentation.model`/`combat_feedback` (`@gc/presentation`, not a
// declared dependency of this package). Neither is available to this port.
//
// `hud.model` itself only transforms an already-built `RenderFrameHud` +
// `MatchHudContext` into a `MatchHudModel`; none of that transformation is
// simulation math. Every case below is therefore ported against hand-built
// `RenderFrameHud`/`CombatPlayerPresentation`/`CombatFeedbackNotice`
// fixtures standing in for what the real Rust/`@gc/presentation` pipeline
// would have produced -- same technique `@gc/screens`'s `content.ts`-based
// fixtures use, and the "supplementary unit tests against the ported
// module's own control flow" the task brief calls for. The specific team
// names/species/notice text below are chosen to exercise `hud.model`'s
// branches, not transcribed from the unreachable Lua fixture values.

import { describe, expect, it } from "vitest";
import { hud, type IdentityPort, type MatchHudContext, type RenderFrameHud } from "./match_hud.ts";

const IDENTITY: IdentityPort = {
  forPlayer: (playerId) =>
    playerId === "missing"
      ? undefined
      : { name: "Zyro Vex", species_name: "Terran", position: "forward" },
};

function context(overrides: Partial<MatchHudContext> = {}): MatchHudContext {
  return {
    home_name: "Nebula FC",
    away_name: "Orion Miners",
    arena_name: "Helios Crown",
    arena_location: "Kairon-9 Orbit",
    tactic_name: "Press High",
    formation_name: "1-2-1",
    ...overrides,
  };
}

function scoreboard(overrides: Partial<RenderFrameHud> = {}): RenderFrameHud {
  return {
    home_score: 0,
    away_score: 0,
    time_left: 5.9,
    controlled_id: "zyro_vex",
    controlled_team: "home",
    controlled_is_keeper: false,
    controlled_owns_ball: true,
    controlled_stamina: 1.4,
    species_shape: "round",
    species_color: [0.5, 0.5, 0.5],
    ...overrides,
  };
}

describe("match HUD model", () => {
  it("formats time, identity, plan, possession, and clamped stamina", () => {
    const filled = hud.model(IDENTITY, scoreboard({ possession_team: "home" }), context());
    expect(filled.clock).toBe("0:05");
    expect(filled.possession).toBe("NEBULA FC POSSESSION");
    expect(filled.possession_marker).toBe("filled");
    expect(filled.player_name).not.toBe("");
    expect(filled.player_detail).toMatch(/TERRAN/);
    expect(filled.plan).toBe("PLAN · PRESS HIGH · 1-2-1");
    expect(filled.stamina).toBe(1);

    const loose = hud.model(IDENTITY, scoreboard(), context());
    expect(loose.possession).toBe("LOOSE BALL");
    expect(loose.possession_marker).toBe("outline");

    const contested = hud.model(
      IDENTITY,
      scoreboard({ possession_team: "away", controlled_owns_ball: false }),
      context(),
    );
    expect(contested.possession).toBe("ORION MINERS POSSESSION");
    expect(contested.player_state).toBe("DEFENDING");
  });

  it("authors kickoff, goal, replay, and full-time broadcast phases", () => {
    const phases = ["kickoff", "goal", "replay", "full_time"] as const;
    for (const phase of phases) {
      const model = hud.model(
        IDENTITY,
        scoreboard(),
        context({ phase, scoring_team: "home" }),
      );
      expect(model.announcement_title, `${phase} needs a title`).toBeDefined();
      expect(model.announcement_detail, `${phase} needs supporting detail`).toBeDefined();
    }
  });

  it("keeps every HUD region inside supported viewports", () => {
    const sizes: readonly [number, number][] = [
      [960, 540],
      [1280, 720],
      [1920, 1080],
      [1280, 800],
    ];
    for (const [w, h] of sizes) {
      const layout = hud.layout({ w, h });
      for (const [id, rect] of Object.entries(layout)) {
        if (typeof rect === "object") {
          expect(rect.x >= 0 && rect.y >= 0, `${id} begins outside the viewport`).toBe(true);
          expect(rect.x + rect.w <= w, `${id} exceeds viewport width`).toBe(true);
          expect(rect.y + rect.h <= h, `${id} exceeds viewport height`).toBe(true);
        }
      }
    }
  });

  it("shows geometry-labeled equipment readiness and confirmed result feedback", () => {
    const model = hud.model(
      IDENTITY,
      scoreboard(),
      context({
        combat_enabled: true,
        combat: { family_name: "spike_launcher", readiness: "committed", cooldown_fraction: 0, phase: "windup", phase_progress: 0.4 },
        combat_notice: { text: "EQUIPMENT COMMITTED", glyph: "chevron" },
      }),
    );
    expect(model.equipment_label).toBeDefined();
    expect(model.equipment_state).toBeDefined();
    expect(model.feedback_text).toBe("EQUIPMENT COMMITTED");
    expect(model.feedback_glyph).toBe("chevron");
    expect(model.equipment_progress >= 0 && model.equipment_progress <= 1).toBe(true);
  });
});
