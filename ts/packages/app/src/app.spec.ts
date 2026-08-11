// The one exception is "applies live screen-shake changes to a paused match
// before resume": it would construct a custom `real`-kind adapter that
// returns a `MatchScreen` built with `combat_enabled: true` and reads its
// combat-feedback diagnostics. As of this batch, `@gc/screens`'s `match.ts`
// (`MatchScreen`) and `@gc/presentation` (a declared dependency here, per
// package.json) both exist -- the stated blocker as originally written is
// stale.
//
// Re-audited a fourth time this wave, against `@gc/presentation`'s
// `combatFeedback` module directly rather than trusting previous passes'
// framing of what this case needs. That framing was wrong in a way worth
// naming plainly: it assumed `combat_feedback.diagnostics(...).reduced_motion`
// summarizes per-tick combat EVENTS, and therefore that a per-tick combat
// event surface (`SimSession.combatEventsJson`, which landed this very wave
// -- `crates/gc-wasm/src/session.rs`, mirrored in
// `packages/wasm/src/types.ts`, and now threaded through this package's own
// `sim_host.ts`) was the missing piece.
//
// Reading `@gc/presentation`'s `combatFeedback` module directly shows that
// is not what `reduced_motion` is. `reduced_motion` is set by
// `feedback.configure(state, reduced_motion, ...)`, driven by the
// `screen_shake` SETTING (`default_reduced_motion = not settings.screen_shake`)
// -- not by anything event-derived at all. The intended case never steps
// the match or feeds it a single combat event; it pauses immediately after
// kickoff, flips the `screen_shake` settings toggle, and checks that
// `_combat_feedback.reduced_motion` flipped with it. So `combatEventsJson`
// is orthogonal to this test's real requirement -- consuming it here would
// not move this case an inch closer to passing.
//
// The actual, current blocker, confirmed by reading `@gc/screens`'s
// `match.ts` directly: `MatchScreenAsRealMatchScreen.applySettings`/
// `MatchScreenAsOnlineMatchScreen.applySettings` are both explicit no-ops
// ("Settings ... are not wired into `MatchScreen` this milestone -- no
// ported module owns them yet"), and the BASE (non-rollback, non-online)
// `MatchScreen` never constructs or stores a `combat_feedback` state at all
// outside rollback mode (`combat_feedback`/`combatFeedback.new()` only
// exist on `MatchRollbackConsumerState`, built by
// `newMatchRollbackConsumerState`, which a `"playtest"`/`"product"`-profile
// screen never touches). So there is no route from a settings change to any
// combat-feedback diagnostic on the screen this test's custom adapter would
// construct, full stop -- not a narrower "per-tick event" gap, a
// "the settings-to-presentation wire does not exist yet" one. That is
// `@gc/screens`'s `match.ts` to build (out of this batch's file ownership;
// `app.ts`, which would call `applySettings`, is also out of this batch's
// file ownership). Still genuinely blocked, now for the reason this case
// actually needs rather than the one a prior pass's comment guessed at.
// Every other case in this file drives the *fake* match adapter (`App`'s
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
