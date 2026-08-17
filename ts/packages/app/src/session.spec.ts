import { describe, expect, it } from "vitest";
import { session } from "./session.ts";
import { MATCH_CONTRACT_CONTENT, NEBULA } from "./test_support/fixtures.ts";

describe("game session", () => {
  it("builds requests and preserves player choices", () => {
    const state = session.new(NEBULA);
    const ids = ["ozzo", "brakka", "rok_tann", "zyro_vex", "mika_olu"];
    const set = session.setStarters(MATCH_CONTRACT_CONTENT, state, NEBULA, ids);
    expect(set.ok).toBe(true);
    session.setFormation(state, "1-1-2");
    session.setTactic(state, "press_high");
    const requestResult = session.buildRequest(MATCH_CONTRACT_CONTENT, state, 9);
    expect(requestResult.ok).toBe(true);
    if (!requestResult.ok) {
      throw new Error("unreachable");
    }
    const request = requestResult.value;
    expect(request.home_starter_ids[4]).toBe("mika_olu");
    expect(request.formation_id).toBe("1-1-2");
    expect(request.tactic_id).toBe("press_high");
    expect(request.show_onboarding).toBe(true);
    // Combat ships on now; the team sheet's toggle is what turns it off.
    expect(request.combat_enabled).toBe(true);

    session.setCombatEnabled(state, false);
    const withoutCombat = session.buildRequest(MATCH_CONTRACT_CONTENT, state, 10);
    expect(withoutCombat.ok && withoutCombat.value.combat_enabled).toBe(false);

    state.firstMatch = false;
    const afterFirstMatch = session.buildRequest(MATCH_CONTRACT_CONTENT, state, 10);
    expect(afterFirstMatch.ok && afterFirstMatch.value.show_onboarding).toBe(false);
  });

  it("maps result actions without mutating setup", () => {
    const state = session.new(NEBULA);
    const original = state.starterIds[0];
    expect(session.routeForResult("rematch")).toBe("match");
    expect(session.routeForResult("change_plan")).toBe("team_sheet");
    expect(session.routeForResult("main_menu")).toBe("title");
    expect(state.starterIds[0]).toBe(original);
  });
});
