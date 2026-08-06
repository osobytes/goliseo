// Ported from spec/game/compatibility_flow_spec.lua.
//
// The second case ("drives the production bootstrap into and out of the
// real match") needs `game.screens.real_match` (via `bootstrap.new`), not
// yet ported to `@gc/screens` -- see this package's porting report and
// bootstrap.spec.ts, which covers what bootstrap.ts itself can prove
// without it.

import { describe, expect, it } from "vitest";
import { App } from "./app.ts";
import { CompatibilityFlow } from "./compatibility_flow.ts";
import { APP_CONTENT } from "./test_support/fixtures.ts";

describe("compatibility flow", () => {
  it("drives the complete fake product flow through the normal input seam", () => {
    const inputs: string[] = [];
    const flow = new CompatibilityFlow((kind) => {
      inputs.push(kind);
    });
    flow.actionDelay = 0;
    const app = new App(APP_CONTENT);

    for (let step = 1; step <= 8; step += 1) {
      flow.update(app, step);
    }

    expect(app.currentRoute()).toBe("result");
    expect(flow.finished).toBe(true);
    const expected = [
      "compat_click_play",
      "compat_click_next",
      "compat_click_next",
      "compat_click_kickoff",
      "compat_click_complete",
    ];
    expect(inputs).toEqual(expected);
  });

  // Needs `game.screens.real_match` via `bootstrap.new` -- see this file's
  // header and bootstrap.spec.ts.
  it.skip("drives the production bootstrap into and out of the real match", () => {});
});
