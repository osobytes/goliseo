// Coverage for `matchAdapter`'s own control flow.
//
// A broader "real match adapter" describe block would mostly exercise
// `@gc/screens`'s concrete real-match screen, which this package does not
// depend on (see package boundaries) -- those assertions aren't expressible
// here. The one exception, "keeps the fake adapter available for isolated
// product-flow tests", only asserts `match_adapter.fake()`/`.real()`'s
// `.kind`, which needs no screen at all -- covered here as `matchAdapter`'s
// only direct spec coverage.

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
