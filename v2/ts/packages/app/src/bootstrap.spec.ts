// Supplementary coverage for game/bootstrap.lua's own control flow, and for
// spec/game/real_match_spec.lua's "real match adapter" describe block.
//
// `game.screens.real_match`/`game.screens.match` are now ported
// (`@gc/screens`'s `real_match.ts`/`match.ts`, `RealMatchScreen`/
// `MatchScreen`/`MatchScreenAsRealMatchScreen`), and this batch's
// `real_match_factory.ts` wires them into a `RealMatchFactory` against an
// INJECTED `createHost`/`renderer` seam specifically so a real
// `RealMatchScreen` can be driven end to end here, against a hand-written
// `FakeSimHost` (`test_support/fixtures.ts`, mirroring `@gc/screens`'s own
// `match_screen.spec.ts` `FakeSimHost`), with no real wasm build or live GL
// context needed. Two of the four originally-skipped cases below are
// therefore no longer blocked; see each `it`'s own comment. The other two
// remain genuinely blocked -- not because `real_match.ts`/`match.ts` don't
// exist, but because the SHAPE they expose does not carry what those two
// assertions need (see their `it.skip` comments for exactly what and why).

import { describe, expect, it } from "vitest";
import { bootstrap } from "./bootstrap.ts";
import { createRealMatchFactory } from "./real_match_factory.ts";
import { matchContract } from "./match_contract.ts";
import { APP_CONTENT, MATCH_CONTRACT_CONTENT, NEBULA, fakeHostFactory, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";

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

// spec/game/real_match_spec.lua's "real match adapter" describe block. The
// Lua original's `RealMatch.new(request, callbacks)` builds its own
// `sim.match` state and hands the spec direct read/write access to it
// (`screen.match.state.players[1].id`, `screen.match.state.score.home = 2`,
// ...) -- duck typing with no privacy, per that language. This port's
// `RealMatchScreenPort.state` (`real_match.ts`) is a narrow, explicitly
// declared, READ-ONLY interface (`{time_left, score}`) by design (this
// package's `real_match_factory.ts` header) -- there is no live `sim.match`
// table to reach into at all, on either language's side of this milestone
// (the sim itself lives in Rust; `MatchScreenAsRealMatchScreen` only ever
// reads its host's decoded HUD). So this suite drives the same real classes
// the Lua spec drove, through the seams THIS port's contract actually
// exposes: a fake `SimHostPort`'s mutable `hud`, and `RealMatchScreen`'s own
// `update`/`event`.
describe("real match adapter", () => {
  // "keeps the fake adapter available for isolated product-flow tests"
  // (`match_adapter.fake()/.real().kind`, no real match needed at all) is
  // ported in match_adapter.spec.ts, not duplicated here -- see that file's
  // header.

  // Needs `screen.match.state.players[1].id`/`.press.home` -- a live
  // `sim.match` state table the Lua original reaches into directly.
  // `RealMatchScreenPort.state` (`real_match.ts`) is deliberately narrowed
  // to `{time_left, score}` with no `players`/`press` fields at all (this
  // package's `real_match_factory.ts` header) -- there is no way to observe
  // a starting XI or a formation's press setting through the contract this
  // milestone's `MatchScreen`/`RealMatchScreenPort` actually expose.
  // Extending that contract is `@gc/screens`'s call (out of this batch's
  // file ownership), not a stale blocker -- still genuinely blocked.
  it.skip("applies request roster, formation, tactic, and seed", () => {});

  it("is the adapter selected by the default bootstrap, and routes a completed match to result", () => {
    const { createHost, hosts } = fakeHostFactory();
    const factory = createRealMatchFactory({
      content: MATCH_CONTRACT_CONTENT,
      createHost,
      renderer: noopRenderPort,
      keyboard: fakeKeyboard(),
    });
    const app = bootstrap.new(APP_CONTENT, factory, 960, 540, {
      settingsStorage: { read: () => undefined, write: () => ({ ok: true, value: true }) },
    });
    expect(app.adapter.kind).toBe("real");
    expect(app.currentRoute()).toBe("title");

    app.handleAction({ go: "play" });
    app.handleAction({ go: "formation", starterIds: NEBULA.roster });
    app.handleAction({ go: "tactic", formationId: "1-2-1" });
    app.handleAction({ go: "match", tacticId: "press_high" });
    expect(app.currentRoute()).toBe("match");

    const host = hosts[hosts.length - 1];
    if (!host) {
      throw new Error("expected a fake sim host to have been constructed");
    }
    host.hud.home_score = 3;
    host.hud.finished = true;
    app.update(0.9); // >= real_match.ts's FULL_TIME_HOLD (0.9s)

    expect(app.currentRoute()).toBe("result");
    expect(app.session.lastResult?.home_score).toBe(3);
  });

  // Needs `screen.match._combat_state` -- an internal `sim.match` field the
  // Lua original reaches into directly to prove combat is constructed only
  // for `combat_enabled` requests. This milestone's `MatchScreen` has no
  // combat state at all (`match.ts`'s own header: "combat... [is] out of
  // scope this milestone" -- `MatchScreenAsRealMatchScreen.frameEvents` is
  // unconditionally `[]`), so there is nothing to assert on either branch of
  // this test yet. Still genuinely blocked, not stale.
  it.skip("constructs combat only for the explicit post-showcase request", () => {});

  it("allows confirmation to advance the full-time hold after its safety beat", () => {
    const { createHost, hosts } = fakeHostFactory();
    const factory = createRealMatchFactory({
      content: MATCH_CONTRACT_CONTENT,
      createHost,
      renderer: noopRenderPort,
      keyboard: fakeKeyboard(),
    });
    const requested = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: NEBULA.roster,
      formation_id: "1-2-1",
      tactic_id: "press_high",
      seed: 91,
    });
    if (!requested.ok) {
      throw new Error(requested.error);
    }
    let completed = false;
    const screen = factory(requested.value, {
      on_finished: () => {
        completed = true;
      },
      on_cancelled: () => {},
    });
    const host = hosts[hosts.length - 1];
    if (!host) {
      throw new Error("expected a fake sim host to have been constructed");
    }
    host.hud.finished = true;

    screen.update?.(0.24);
    screen.event?.({ kind: "action", action: "confirm" });
    expect(completed).toBe(false); // confirmation cannot erase the full-time beat immediately

    screen.update?.(0.02); // fullTimeElapsed now 0.26, past real_match.ts's FULL_TIME_SKIP_DELAY (0.25)
    screen.event?.({ kind: "action", action: "confirm" });
    expect(completed).toBe(true);
  });
});
