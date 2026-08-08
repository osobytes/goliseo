// Supplementary coverage for game/match_adapter.lua's own control flow.
//
// spec/game/real_match_spec.lua's "real match adapter" describe block is
// almost entirely about `game.screens.real_match`, which has no TS port yet
// (see this package's porting report) -- ported as `it.skip` in
// match_adapter_realmatch.spec.ts is not needed since none of those
// assertions are expressible without it. The one exception, "keeps the fake
// adapter available for isolated product-flow tests" (real_match_spec.lua
// lines 124-127), only asserts `match_adapter.fake()`/`.real()`'s `.kind`,
// which needs no screen at all -- ported here as `matchAdapter`'s only
// direct spec coverage.

import { describe, expect, it } from "vitest";
import { matchAdapter } from "./match_adapter.ts";
import { MATCH_CONTRACT_CONTENT } from "./test_support/fixtures.ts";

describe("match adapter", () => {
  it("keeps the fake adapter available for isolated product-flow tests", () => {
    expect(matchAdapter.fake(MATCH_CONTRACT_CONTENT).kind).toBe("fake");
    expect(matchAdapter.real(() => {
      throw new Error("not invoked by this test");
    }).kind).toBe("real");
  });
});
