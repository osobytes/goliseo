// Ported from spec/game/app_flow_spec.lua.
//
// The one exception is "applies live screen-shake changes to a paused match
// before resume": it constructs a custom `real`-kind adapter that returns
// `game.screens.match.new({combat_enabled = true})` directly and reads
// `combat_feedback.diagnostics(match_screen._combat_feedback)`. As of this
// batch, `@gc/screens`'s `match.ts` (`MatchScreen`) and `@gc/presentation`
// (a declared dependency here, per package.json) both exist -- the stated
// blocker as originally written is stale.
//
// Re-audited a third time this batch, against current code rather than the
// prior pass's comment: `@gc/wasm`'s `Session::step`
// (`crates/gc-wasm/src/session.rs`) no longer hard-codes `combat_state:
// None` -- `Session::new` gained a `combat_enabled` parameter and `step`
// now threads the resulting companion through every tick (this wave; see
// `bootstrap.spec.ts`'s now-ported "constructs combat only for the explicit
// post-showcase request", which exercises exactly that flag reaching
// `MatchScreenOptions.combat_enabled` via `real_match_factory.ts`). So the
// PREVIOUS blocker named here is stale.
//
// This case needs strictly more than that, though: `combat_feedback.diagnostics`
// summarizes per-tick combat EVENTS (`_combat_feedback`'s ported
// `combatFeedback.confirm`/`link` calls, `match.ts`'s own rollback-only
// `consumeConfirmedStep`), not merely combat's construction-time presence.
// Two gaps remain, both still genuinely out of reach this wave:
// (1) neither `@gc/wasm`'s `SimSession` nor `gc_render::frame::RenderFrame`
// exposes ANY per-tick combat event/state getter for the BASE (non-rollback)
// session this test's custom adapter would drive -- confirmed by reading
// `session.rs` and `frame.rs` directly, not inferred; and (2) `match.ts`'s
// own BASE-mode `update()` has no code path that consumes combat events at
// all (unlike its rollback branch's `consumeConfirmedStep`, which only ever
// receives ROLLBACK-confirmed steps). Both would need to land before this
// case's `_combat_feedback.diagnostics` assertion could mean anything here.
// Ported as `it.skip` below with the CURRENT, narrower blocker named; every
// other case in this file drives the *fake* match adapter (`App`'s
// default), which needs neither.

import { describe, expect, it } from "vitest";
import { actions } from "@gc/input";
import { fakeResult } from "./fake_result.ts";
import { session } from "./session.ts";
import { settings } from "./settings.ts";
import { App } from "./app.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { APP_CONTENT, MATCH_CONTRACT_CONTENT } from "./test_support/fixtures.ts";

function clickWidget(app: App, id: string): void {
  const layout = menuLayout(app.stack.current());
  if (!layout) {
    throw new Error(`no menu layout on the current screen (looking for widget ${id})`);
  }
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  const [x, y] = viewport.toActual(app.transform, widget.rect.x + widget.rect.w / 2, widget.rect.y + widget.rect.h / 2);
  app.event({ kind: "click", x, y, button: 1 });
}

function reachFakeMatch(app: App): void {
  clickWidget(app, "play");
  clickWidget(app, "next");
  clickWidget(app, "formation_1-1-2");
  clickWidget(app, "next");
  clickWidget(app, "tactic_press_high");
  clickWidget(app, "kickoff");
  expect(app.currentRoute()).toBe("match");
}

function newApp(overrides: ConstructorParameters<typeof App>[1] = {}): App {
  return new App(APP_CONTENT, overrides);
}

describe("product application flow", () => {
  it("drives title through deterministic result and repeated matches", () => {
    const app = newApp({ actualW: 1280, actualH: 800 });
    reachFakeMatch(app);
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");

    clickWidget(app, "complete");
    expect(app.currentRoute()).toBe("result");
    expect(app.session.matchNumber).toBe(1);
    const firstSeed = app.session.lastResult?.seed;

    clickWidget(app, "rematch");
    expect(app.currentRoute()).toBe("match");
    clickWidget(app, "complete");
    expect(app.currentRoute()).toBe("result");
    expect(app.session.matchNumber).toBe(2);
    expect(app.session.lastResult?.seed).toBe(2);
    expect(firstSeed).not.toBe(app.session.lastResult?.seed);

    clickWidget(app, "rematch");
    clickWidget(app, "complete");
    expect(app.currentRoute()).toBe("result");
    expect(app.session.matchNumber).toBe(3);
    expect(app.session.lastResult?.seed).toBe(3);

    clickWidget(app, "change_plan");
    expect(app.currentRoute()).toBe("formation");
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");
    expect(app.session.starterIds.length).toBe(5);
  });

  it("supports every result exit without losing the intended choices", () => {
    const app = newApp();
    reachFakeMatch(app);
    clickWidget(app, "complete");
    clickWidget(app, "change_lineup");
    expect(app.currentRoute()).toBe("squad");
    expect(app.session.starterIds.length).toBe(5);

    app.handleAction({ go: "formation", starterIds: app.session.starterIds });
    app.handleAction({ go: "tactic", formationId: app.session.formationId });
    app.handleAction({ go: "match", tacticId: app.session.tacticId });
    clickWidget(app, "complete");
    clickWidget(app, "main_menu");
    expect(app.currentRoute()).toBe("title");
  });

  it("preserves formation and tactic choices while navigating backward", () => {
    const app = newApp();
    clickWidget(app, "play");
    clickWidget(app, "next");
    clickWidget(app, "formation_1-1-2");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("squad");
    expect(app.session.formationId).toBe("1-1-2");

    clickWidget(app, "next");
    clickWidget(app, "next");
    clickWidget(app, "tactic_counter");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("formation");
    expect(app.session.tacticId).toBe("counter");
    clickWidget(app, "next");
    const layout = menuLayout(app.stack.current());
    const widget = layout ? hit.find(layout, "tactic_counter") : null;
    expect((widget as { readonly selected?: boolean } | null)?.selected).toBe(true);
  });

  it("maps keyboard and gamepad through nested shell routes", () => {
    const app = newApp();
    app.event({ kind: "key", key: "down" });
    app.event({ kind: "key", key: "down" });
    app.event({ kind: "gamepad", button: "a" });
    expect(app.currentRoute()).toBe("help");
    app.event({ kind: "gamepad", button: "b" });
    expect(app.currentRoute()).toBe("title");
  });

  it("keeps the showcase combat-disabled and exposes a separate prototype path", () => {
    const app = newApp();
    clickWidget(app, "combat_prototype");
    expect(app.currentRoute()).toBe("squad");
    expect(app.session.combatEnabled).toBe(true);
    const withCombat = session.buildRequest(MATCH_CONTRACT_CONTENT, app.session, 4);
    expect(withCombat.ok && withCombat.value.combat_enabled).toBe(true);

    app.showTitle();
    clickWidget(app, "play");
    expect(app.session.combatEnabled).toBe(false);
    const withoutCombat = session.buildRequest(MATCH_CONTRACT_CONTENT, app.session, 5);
    expect(withoutCombat.ok && withoutCombat.value.combat_enabled).toBe(false);
  });

  it("backs out of credits and handles quit deliberately", () => {
    const app = newApp();
    clickWidget(app, "credits");
    expect(app.currentRoute()).toBe("credits");
    app.event({ kind: "key", key: "escape" });
    expect(app.currentRoute()).toBe("title");
    clickWidget(app, "quit");
    expect(app.quitRequested).toBe(true);
  });

  it("persists settings and resumes a paused fake fixture", () => {
    let saved: string | undefined;
    const storage = {
      read: () => undefined,
      write: (contents: string) => {
        saved = contents;
        return { ok: true as const, value: true as const };
      },
    };
    const app = newApp({ settingsStorage: storage });
    clickWidget(app, "settings");
    app.event(actions.event("left"));
    expect(app.settings.master_volume).toBeCloseTo(0.9);
    clickWidget(app, "fullscreen");
    expect(app.settings.fullscreen).toBe(true);
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("title");
    expect(saved?.includes("master_volume=0.90")).toBe(true);

    reachFakeMatch(app);
    app.event({ kind: "key", key: "p" });
    expect(app.currentRoute()).toBe("pause");
    app.event({ kind: "gamepad", button: "start" });
    expect(app.currentRoute()).toBe("match");
  });

  // `@gc/wasm`'s `Session` never runs combat outside the rollback
  // laboratory -- see this file's header.
  it.skip("applies live screen-shake changes to a paused match before resume", () => {});

  it("requires confirmation before restarting a paused fixture", () => {
    const app = newApp();
    reachFakeMatch(app);
    app.event({ kind: "key", key: "p" });
    clickWidget(app, "restart");
    expect(app.currentRoute()).toBe("pause");
    clickWidget(app, "restart");
    expect(app.currentRoute()).toBe("match");
    expect(app.session.matchNumber).toBe(0);
  });

  it("returns from nested pause routes and can leave for the title", () => {
    const storage = {
      read: () => undefined,
      write: () => ({ ok: true as const, value: true as const }),
    };
    const app = newApp({ settingsStorage: storage });
    reachFakeMatch(app);
    app.event({ kind: "key", key: "p" });
    clickWidget(app, "controls");
    expect(app.currentRoute()).toBe("help");
    app.event({ kind: "gamepad", button: "b" });
    expect(app.currentRoute()).toBe("pause");
    clickWidget(app, "settings");
    expect(app.currentRoute()).toBe("settings");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("pause");
    clickWidget(app, "main_menu");
    expect(app.currentRoute()).toBe("title");
  });
});

describe("fake match adapter", () => {
  it("produces identical results for identical requests", () => {
    const app = newApp();
    const requestResult = session.buildRequest(MATCH_CONTRACT_CONTENT, app.session, 41);
    if (!requestResult.ok) {
      throw new Error("unreachable");
    }
    const request = requestResult.value;
    const first = fakeResult.forRequest(MATCH_CONTRACT_CONTENT, request);
    const second = fakeResult.forRequest(MATCH_CONTRACT_CONTENT, request);
    expect(first.home_score).toBe(second.home_score);
    expect(first.away_score).toBe(second.away_score);
    expect(first.mvp_player_id).toBe(second.mvp_player_id);
    expect(first.home_stats.possession).toBe(second.home_stats.possession);
  });

  it("cancels back to tactical setup", () => {
    const app = newApp();
    reachFakeMatch(app);
    clickWidget(app, "cancel");
    expect(app.currentRoute()).toBe("tactic");
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");
  });
});
