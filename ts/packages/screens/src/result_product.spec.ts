// Fixture note: production builds these fixtures via `@gc/app`'s
// `match_contract.ts`'s `newResult` (not this package's to own — see
// content.ts's header). `makeResult` below reproduces just the two things
// that function does that the screen's assertions depend on: filling in
// `home_name`/`away_name` from `gc-data`'s teams (transcribed verbatim) and
// deriving `winner` from the scores — the same defaulting
// `match_contract.ts`'s `newResult` performs.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { result, type ResultContentData } from "./result.ts";
import type { MatchWinner, ProductMatchResult, TeamResultStats } from "./content.ts";

const VP = { w: 960, h: 540 };
const CONTENT: ResultContentData = { players: {} };

function makeResult(
  homeScore: number,
  awayScore: number,
  stats?: TeamResultStats,
): ProductMatchResult {
  const winner: MatchWinner =
    homeScore > awayScore ? "home" : awayScore > homeScore ? "away" : "draw";
  return {
    home_name: "Nebula FC",
    away_name: "Orion Miners",
    home_score: homeScore,
    away_score: awayScore,
    winner,
    home_stats: stats ?? {},
    away_stats: stats ?? {},
  };
}

describe("product result screen", () => {
  it("presents win, loss, and draw outcomes explicitly", () => {
    const cases: readonly [number, number, string][] = [
      [2, 1, "NEBULA FC WIN"],
      [0, 3, "ORION MINERS WIN"],
      [0, 0, "HONORS EVEN"],
    ];
    for (const [homeScore, awayScore, expected] of cases) {
      const state = result.newState(VP, CONTENT, { result: makeResult(homeScore, awayScore) });
      const outcome = hit.find(result.layout(state), "outcome");
      expect(outcome?.text).toBe(expected);
    }
  });

  it("degrades missing metrics and zero-event fixtures without inventing values", () => {
    const missing = result.layout(result.newState(VP, CONTENT, { result: makeResult(0, 0) }));
    const missingStats = hit.find(missing, "stats");
    expect(missingStats?.text).toContain("—");

    const zero = result.layout(
      result.newState(VP, CONTENT, {
        result: makeResult(0, 0, { shots: 0, possession: 0, saves: 0, pass_completion: 0 }),
      }),
    );
    const text = hit.find(zero, "stats")?.text;
    expect(text).toBeDefined();
    expect(text).toMatch(/SHOTS\s+0/);
    expect(text).toContain("0%");
  });
});
