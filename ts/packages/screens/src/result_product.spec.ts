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

  // One screen, two contexts. This replaced a second route (`online_result`)
  // that existed only to make one button unreachable.
  it("offers an offline rematch and a route back into the pre-match screen", () => {
    const layout = result.layout(result.newState(VP, CONTENT, { result: makeResult(3, 2) }));
    expect(hit.find(layout, "rematch")).not.toBeNull();
    expect(hit.find(layout, "change_plan")).not.toBeNull();
    expect(hit.find(layout, "main_menu")).not.toBeNull();
    expect(hit.find(layout, "back_to_lobby"), "offline has no lobby to return to").toBeNull();
  });

  it("swaps that route for the lobby when the session was online", () => {
    const layout = result.layout(
      result.newState(VP, CONTENT, { result: makeResult(3, 2), online: true }),
    );
    expect(hit.find(layout, "back_to_lobby")).not.toBeNull();
    expect(
      hit.find(layout, "change_plan"),
      "an online session has no local pre-match state to change",
    ).toBeNull();
    expect(hit.find(layout, "rematch")).not.toBeNull();
  });

  it("emits whichever action was actually offered", () => {
    const online = result.newState(VP, CONTENT, { result: makeResult(1, 1), online: true });
    const target = hit.find(result.layout(online), "back_to_lobby")?.rect;
    expect(target).toBeDefined();
    const [, action] = result.update(online, {
      kind: "click",
      x: (target?.x ?? 0) + (target?.w ?? 0) / 2,
      y: (target?.y ?? 0) + (target?.h ?? 0) / 2,
      button: 1,
    });
    expect(action?.go).toBe("back_to_lobby");
  });

  it("keeps every footer button inside the virtual canvas in both contexts", () => {
    for (const online of [false, true]) {
      const layout = result.layout(
        result.newState(VP, CONTENT, { result: makeResult(1, 0), online }),
      );
      for (const widget of layout) {
        const rect = widget.rect;
        expect(rect, `widget ${widget.id} has no rect`).toBeDefined();
        expect(rect?.x ?? -1).toBeGreaterThanOrEqual(0);
        expect((rect?.x ?? 0) + (rect?.w ?? 0)).toBeLessThanOrEqual(VP.w);
        expect((rect?.y ?? 0) + (rect?.h ?? 0)).toBeLessThanOrEqual(VP.h);
      }
    }
  });
});
