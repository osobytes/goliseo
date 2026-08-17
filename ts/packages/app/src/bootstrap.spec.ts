// Coverage for `bootstrap.ts`'s own control flow, and for the "real match
// adapter" describe block below.
//
// `@gc/screens`'s `real_match.ts`/`match.ts` (`RealMatchScreen`/
// `MatchScreen`/`MatchScreenAsRealMatchScreen`) exist, and this batch's
// `real_match_factory.ts` wires them into a `RealMatchFactory` against an
// INJECTED `createHost`/`renderer` seam specifically so a real
// `RealMatchScreen` can be driven end to end here, against a hand-written
// `FakeSimHost` (`test_support/fixtures.ts`, mirroring `@gc/screens`'s own
// `match_screen.spec.ts` `FakeSimHost`), with no real wasm build or live GL
// context needed. All four cases below that were originally skipped are now
// unblocked (the combat case joined two others in an earlier wave once
// `crates/gc-wasm/src/session.rs`'s `Session::new`/`step` grew a real combat
// surface; the roster/formation/tactic/seed case is unblocked this wave, now
// that the same constructor also grew `tactic`/`away_tactic`/
// `home_starter_ids` -- see that `it`'s own comment for what it proves and
// at which layer, since `RealMatchScreenPort.state` itself still cannot
// carry `players`/`press`); see each `it`'s own comment.

import { describe, expect, it } from "vitest";
import { bootstrap } from "./bootstrap.ts";
import { createRealMatchFactory } from "./real_match_factory.ts";
import { matchContract } from "./match_contract.ts";
import { createSimHost } from "./sim_host.ts";
import {
  APP_CONTENT,
  MATCH_CONTRACT_CONTENT,
  NEBULA,
  fakeHostFactory,
  fakeKeyboard,
  noopRenderPort,
} from "./test_support/fixtures.ts";

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

// This block covers the "real match adapter" describe block.
// `RealMatchScreenPort.state` (`real_match.ts`) is a narrow, explicitly
// declared, READ-ONLY interface (`{time_left, score}`) by design (this
// package's `real_match_factory.ts` header) -- there is no live
// match-state table to reach into at all (the sim itself lives in Rust;
// `MatchScreenAsRealMatchScreen` only ever reads its host's decoded HUD).
// So this suite drives the real classes through the seams this package's
// contract actually exposes: a fake `SimHostPort`'s mutable `hud`, and
// `RealMatchScreen`'s own `update`/`event`.
describe("real match adapter", () => {
  // "keeps the fake adapter available for isolated product-flow tests"
  // (`match_adapter.fake()/.real().kind`, no real match needed at all) is
  // covered in match_adapter.spec.ts, not duplicated here -- see that
  // file's header.

  // Re-audited a third time this wave, against current code rather than the
  // prior pass's comment. `Session::new`'s wasm binding
  // (`crates/gc-wasm/src/session.rs`) now takes `home_formation`/`tactic`/
  // `away_tactic`/`home_starter_ids` as its sixth/eighth/ninth/tenth
  // optional parameters (confirmed by reading the regenerated
  // `dist/pkg/gc_wasm.d.cts` after rebuilding, not assumed from a stale
  // artifact -- `types.ts`'s `SimSessionConstructor` matches it field for
  // field), and `sim_host.ts`'s `SimHostOptions` (this package's own file)
  // now threads all four through to the real constructor. That closes the
  // stated blocker: a request's `tactic_id`/`home_starter_ids` DO now reach
  // a real simulated match's `press`/roster.
  //
  // What is STILL true, and still shapes how this case is written:
  // `RealMatchScreenPort.state` (`real_match.ts`, `@gc/screens`, out of
  // this batch's file ownership) stays deliberately narrowed to
  // `{time_left, score}` -- there is no route from an actual constructed
  // `RealMatchScreen` to `players`/`press`. So this proves the claim at
  // the layer this package DOES own and control end to end: the exact
  // parameters `real_match_factory.ts`'s injected `deps.createHost`
  // closure receives from a `ProductMatchRequest` -- the same shape
  // `browser_main.ts`'s real call site now forwards (see that file's
  // `createHost` closure; it used to silently drop `formation_id`/
  // `tactic_id`/`home_starter_ids` entirely, forwarding only
  // `combat_enabled`) -- reach a real `createSimHost`-built session's
  // `matchStateJson()`/`snapshotHash()` correctly. `players[0]`/`players[1]`
  // and `press.home` use zero-based array indexing; `snapshotHash` stands
  // in for a direct `result.seed` check (that one is downstream of
  // `@gc/screens`'s own result-building, out of scope here) by proving
  // `seed` reaches the session deterministically.
  it("applies request roster, formation, tactic, and seed", () => {
    const STARTERS = ["ozzo", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin"];
    const requested = matchContract.newRequest(MATCH_CONTRACT_CONTENT, {
      home_team_id: "nebula",
      away_team_id: "orion",
      home_starter_ids: STARTERS,
      formation_id: "1-2-1",
      tactic_id: "press_high",
      seed: 77,
    });
    if (!requested.ok) {
      throw new Error(requested.error);
    }
    const request = requested.value;

    // The exact parameters `real_match_factory.ts`'s injected `createHost`
    // closure receives from `request`, threaded the same way
    // `browser_main.ts`'s real call site now does.
    const host = createSimHost(
      request.home_team_id,
      request.away_team_id,
      request.seed ?? 0,
      20,
      3,
      {
        homeFormation: request.formation_id,
        tactic: request.tactic_id,
        homeStarterIds: request.home_starter_ids,
      },
    );
    try {
      const raw = host.matchStateJson?.();
      if (raw === undefined) {
        throw new Error("expected matchStateJson() on a WasmSimHost");
      }
      const state = JSON.parse(raw) as {
        readonly players: readonly { readonly id: string }[];
        readonly press: { readonly home: number; readonly away: number };
      };
      expect(state.players[0]?.id).toBe("ozzo");
      expect(state.players[1]?.id).toBe("veil_nyx");
      expect(state.press.home).toBe(2); // press_high

      const sameSeed = createSimHost(
        request.home_team_id,
        request.away_team_id,
        request.seed ?? 0,
        20,
        3,
        {
          homeFormation: request.formation_id,
          tactic: request.tactic_id,
          homeStarterIds: request.home_starter_ids,
        },
      );
      try {
        expect(sameSeed.snapshotHash?.()).toBe(host.snapshotHash?.());
      } finally {
        sameSeed.dispose();
      }

      const differentSeed = createSimHost(
        request.home_team_id,
        request.away_team_id,
        (request.seed ?? 0) + 1,
        20,
        3,
        {
          homeFormation: request.formation_id,
          tactic: request.tactic_id,
          homeStarterIds: request.home_starter_ids,
        },
      );
      try {
        expect(differentSeed.snapshotHash?.()).not.toBe(host.snapshotHash?.());
      } finally {
        differentSeed.dispose();
      }
    } finally {
      host.dispose();
    }
  });

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
    app.handleAction({
      go: "match",
      starterIds: NEBULA.roster,
      formationId: "1-2-1",
      tacticId: "press_high",
      combatEnabled: true,
    });
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
  // `RealMatchScreen.match`) is the observable analog used here.
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
    expect(plainScreen.match.debugCombatEnabled, "an ordinary request never opts into combat").toBe(
      false,
    );

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
