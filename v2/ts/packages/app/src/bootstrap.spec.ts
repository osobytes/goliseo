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
// context needed. Three of the four originally-skipped cases below are
// therefore no longer blocked (the combat case joined the other two this
// wave, once `crates/gc-wasm/src/session.rs`'s `Session::new`/`step` grew a
// real combat surface -- see that `it`'s own comment); see each `it`'s own
// comment. The remaining one stays genuinely blocked -- not because
// `real_match.ts`/`match.ts` don't exist, but because the SHAPE they expose
// does not carry what that assertion needs (see its `it.skip` comment for
// exactly what and why).

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
  //
  // Re-audited this batch: `@gc/screens` (`real_match.ts`/`match.ts`) is
  // now IN this batch's file ownership, so "extending that contract is
  // someone else's call" (the previous version of this note) is stale --
  // widening `RealMatchScreenPort.state` was genuinely on the table. It
  // still does not reach this case, for a narrower, confirmed reason:
  // `players[1].id`/`players[2].id` (a starting XI) is derivable without
  // any wasm change at all -- `home_starter_ids` is already static, known
  // content on the request this factory receives -- but `press.home`
  // (`gc_sim::match::MatchState.press`, a `ByTeam<f64>` copied from the
  // authored tactic's `TacticData.press` at kickoff,
  // `crates/gc-sim/src/match.rs`) is exposed NOWHERE reachable from this
  // package: not `@gc/wasm`'s `SimSession` (score/timeLeft/finished/
  // inputTick/snapshotHash/roster only, confirmed by reading `session.rs`),
  // not `gc_render::frame::RenderFrame` (no `press` field, confirmed by
  // reading `frame.rs`), and not this package's own `content.ts`'s
  // `TacticExists` (`{id}` only, no `press`) -- and `content.ts` is NOT in
  // this batch's file ownership, so adding the field is not this task's to
  // do either. Still genuinely blocked, now for the `press` half only.
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

  // Re-audited against current code: `@gc/wasm`'s `Session::step`
  // (`crates/gc-wasm/src/session.rs`) no longer hard-codes `combat_state:
  // None` -- `Session::new` gained a `combat_enabled` parameter and `step`
  // now threads the resulting companion through every tick, so that stated
  // blocker is stale. What was ALSO stale, and is now fixed here:
  // `real_match_factory.ts` never forwarded `ProductMatchRequest.combat_enabled`
  // (the "explicit post-showcase request" opt-in) into
  // `MatchScreenOptions.combat_enabled` at all -- it silently dropped the
  // flag on every request, combat or not. That plumbing gap is now closed,
  // and `MatchScreen.debugCombatEnabled` (reached here through
  // `RealMatchScreen.match`, mirroring how `Lua`'s
  // `screen.match._combat_state` presence check worked) is this port's
  // observable analog.
  //
  // What is STILL genuinely blocked, for a real reason rather than a stale
  // one: neither `@gc/wasm`'s `SimSession` nor `@gc/screens`'s `SimHostPort`
  // exposes any getter for combat presence or per-tick combat state, so
  // this case cannot prove the underlying wasm session this factory's
  // `deps.createHost` closure builds actually carries a matching
  // `combat_enabled` -- only that the REQUEST's opt-in reaches
  // `MatchScreen`'s own construction option correctly. See
  // `match_screen.spec.ts`'s "constructs and drives combat only behind the
  // explicit option" for the same boundary, spelled out fully.
  it("constructs combat only for the explicit post-showcase request", () => {
    const { createHost, hosts } = fakeHostFactory();
    const factory = createRealMatchFactory({
      content: MATCH_CONTRACT_CONTENT,
      createHost,
      renderer: noopRenderPort,
      keyboard: fakeKeyboard(),
    });
    const callbacks = { on_finished: (): void => {}, on_cancelled: (): void => {} };

    const withCombat = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: NEBULA.roster,
      formation_id: "1-2-1",
      tactic_id: "press_high",
      seed: 5,
      combat_enabled: true,
    });
    if (!withCombat.ok) {
      throw new Error(withCombat.error);
    }
    const combatScreen = factory(withCombat.value, callbacks) as unknown as {
      readonly match: { readonly debugCombatEnabled?: boolean };
    };
    expect(
      combatScreen.match.debugCombatEnabled,
      "the explicit post-showcase request's combat_enabled reaches MatchScreenOptions",
    ).toBe(true);

    const withoutCombat = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: NEBULA.roster,
      formation_id: "1-2-1",
      tactic_id: "press_high",
      seed: 5,
    });
    if (!withoutCombat.ok) {
      throw new Error(withoutCombat.error);
    }
    const plainScreen = factory(withoutCombat.value, callbacks) as unknown as {
      readonly match: { readonly debugCombatEnabled?: boolean };
    };
    expect(plainScreen.match.debugCombatEnabled, "an ordinary request never opts into combat").toBe(false);

    expect(hosts.length, "one host per constructed match").toBe(2);
  });

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
