import { describe, expect, it } from "vitest";
import { matchContract } from "./match_contract.ts";
import { MATCH_CONTRACT_CONTENT, NEBULA } from "./test_support/fixtures.ts";

describe("match contracts", () => {
  it("builds an immutable validated request", () => {
    const source = ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"];
    const requestResult = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: source,
      formation_id: "1-2-1",
      tactic_id: "counter",
      seed: 42,
    });
    expect(requestResult.ok).toBe(true);
    if (!requestResult.ok) {
      throw new Error("unreachable");
    }
    const request = requestResult.value;
    source[0] = "changed";
    expect(request.home_starter_ids[0]).toBe("ozzo");
    expect(request.arena_id).toBe("helios_crown");
    expect(request.show_onboarding).toBe(false);
    expect(request.combat_enabled).toBe(false);
    expect(request.seed).toBe(42);

    const combatRequestResult = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
      formation_id: "1-2-1",
      tactic_id: "counter",
      combat_enabled: true,
    });
    expect(combatRequestResult.ok).toBe(true);
    if (combatRequestResult.ok) {
      expect(combatRequestResult.value.combat_enabled).toBe(true);
    }
  });

  it("rejects unknown arena presentation records", () => {
    const result = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
      formation_id: "1-2-1",
      tactic_id: "counter",
      arena_id: "unfinished_moon",
    });
    expect(result.ok).toBe(false);
    expect(!result.ok && result.error.includes("arena")).toBe(true);
  });

  it("rejects duplicate, ineligible, and keeperless team sheets", () => {
    const duplicate = matchContract.validateStarters(
      MATCH_CONTRACT_CONTENT,
      ["ozzo", "brakka", "brakka", "rok_tann", "zyro_vex"],
      NEBULA,
    );
    expect(duplicate.ok).toBe(false);
    expect(!duplicate.ok && duplicate.error.includes("unique")).toBe(true);

    const ineligible = matchContract.validateStarters(
      MATCH_CONTRACT_CONTENT,
      ["gax_oru", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
      NEBULA,
    );
    expect(ineligible.ok).toBe(false);
    expect(!ineligible.ok && ineligible.error.includes("eligible")).toBe(true);

    const keeperless = matchContract.validateStarters(
      MATCH_CONTRACT_CONTENT,
      ["brakka", "veil_nyx", "rok_tann", "zyro_vex", "mika_olu"],
      NEBULA,
    );
    expect(keeperless.ok).toBe(false);
    expect(!keeperless.ok && keeperless.error.includes("keeper")).toBe(true);
  });

  it("derives winner and accepts unavailable optional metrics", () => {
    const result = matchContract.newResult(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_score: 2,
      away_score: 1,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.winner).toBe("home");
      expect(result.value.home_stats.shots).toBeUndefined();
    }
  });

  it("rejects invalid result ratios", () => {
    const result = matchContract.newResult(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_score: 0,
      away_score: 0,
      home_stats: { possession: 1.2 },
    });
    expect(result.ok).toBe(false);
    expect(!result.ok && result.error.includes("possession")).toBe(true);
  });
});
