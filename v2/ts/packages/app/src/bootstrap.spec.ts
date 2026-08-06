// Supplementary coverage for game/bootstrap.lua's own control flow.
//
// spec/game/compatibility_flow_spec.lua's "drives the production bootstrap
// into and out of the real match" and spec/game/real_match_spec.lua's
// "is the adapter selected by the default bootstrap" both need
// `game.screens.real_match`, not yet ported to `@gc/screens` (see this
// package's porting report) -- ported as `it.skip` in
// compatibility_flow.spec.ts with that unblocker named. This test covers
// what bootstrap.ts itself is responsible for: wiring a "real"-kind
// `MatchAdapter` (via the injected `RealMatchFactory`) into a fresh `App`,
// which needs no real match screen to verify.

import { describe, expect, it } from "vitest";
import { bootstrap } from "./bootstrap.ts";
import { APP_CONTENT } from "./test_support/fixtures.ts";

describe("bootstrap", () => {
  it("wires a real-kind match adapter into a fresh App", () => {
    const app = bootstrap.new(
      APP_CONTENT,
      () => {
        throw new Error("not invoked by this test");
      },
      960,
      540,
      {
        settingsStorage: { read: () => undefined, write: () => ({ ok: true, value: true }) },
      },
    );
    expect(app.adapter.kind).toBe("real");
    expect(app.currentRoute()).toBe("title");
  });
});

// spec/game/real_match_spec.lua's "real match adapter" describe block is
// primarily an integration test of `game.screens.real_match` (bootstrapped
// through `bootstrap.new` + bare `App` route transitions into it) -- a
// screens-package file, not this package's to claim (v2/README.md's file
// table; the task brief's "leave those and name them in your report").
// This package only claims the one assertion in that block that needs
// neither `RealMatch` nor `Match` (`match_adapter.fake()/.real().kind` --
// ported in match_adapter.spec.ts). The remaining four are named here so
// they are not silently dropped from the accounting, even though the
// block as a whole belongs to whoever ports `game/screens/real_match.lua`.
describe.skip("real match adapter (needs game.screens.real_match)", () => {
  it("applies request roster, formation, tactic, and seed", () => {});
  it("is the adapter selected by the default bootstrap", () => {});
  it("constructs combat only for the explicit post-showcase request", () => {});
  it("allows confirmation to advance the full-time hold after its safety beat", () => {});
});
