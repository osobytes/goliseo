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
import { ok, type Result } from "@gc/core";
import { actions } from "@gc/input";
import { fakeResult } from "./fake_result.ts";
import { session } from "./session.ts";
import { App, type OnlinePorts } from "./app.ts";
import { teamSettings } from "./team_settings.ts";
import type { SettingsStorage } from "./settings.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { APP_CONTENT, MATCH_CONTRACT_CONTENT, NEBULA } from "./test_support/fixtures.ts";

function memoryStorage(seed?: string): SettingsStorage & { contents: string | undefined } {
  const box: { contents: string | undefined } = { contents: seed };
  return {
    get contents() {
      return box.contents;
    },
    read: () => box.contents,
    write: (value): Result<true, string> => {
      box.contents = value;
      return ok(true);
    },
  };
}

function clickWidget(app: App, id: string): void {
  const layout = menuLayout(app.stack.current());
  if (!layout) {
    throw new Error(`no menu layout on the current screen (looking for widget ${id})`);
  }
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  const [x, y] = viewport.toActual(
    app.transform,
    widget.rect.x + widget.rect.w / 2,
    widget.rect.y + widget.rect.h / 2,
  );
  app.event({ kind: "click", x, y, button: 1 });
}

// Title -> team sheet -> match. This used to be six clicks across three
// routes (squad -> formation -> tactic); it is one screen and one commit now.
function reachFakeMatch(app: App): void {
  clickWidget(app, "play");
  expect(app.currentRoute()).toBe("team_sheet");
  clickWidget(app, "formation_1-1-2");
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
    expect(app.currentRoute()).toBe("team_sheet");
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");
    expect(app.session.starterIds.length).toBe(5);
  });

  it("supports every result exit without losing the intended choices", () => {
    const app = newApp();
    reachFakeMatch(app);
    clickWidget(app, "complete");
    clickWidget(app, "change_plan");
    expect(app.currentRoute()).toBe("team_sheet");
    expect(app.session.starterIds.length).toBe(5);

    clickWidget(app, "kickoff");
    expect(app.currentRoute()).toBe("match");
    clickWidget(app, "complete");
    clickWidget(app, "main_menu");
    expect(app.currentRoute()).toBe("title");
  });

  it("carries every choice back into the team sheet it was made on, and BACK commits it too", () => {
    const app = newApp();
    clickWidget(app, "play");
    clickWidget(app, "formation_1-1-2");
    clickWidget(app, "tactic_counter");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("title");

    // BACK commits the draft too now (#600's team-persistence fix), not
    // only a kickoff -- a standalone visit that ends in BACK still sticks.
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("counter");

    clickWidget(app, "play");
    expect(app.currentRoute()).toBe("team_sheet");
    const reopened = menuLayout(app.stack.current());
    const reopenedShape = reopened ? hit.find(reopened, "formation_1-1-2") : null;
    expect((reopenedShape as { readonly selected?: boolean } | null)?.selected).toBe(true);
    const reopenedTactic = reopened ? hit.find(reopened, "tactic_counter") : null;
    expect((reopenedTactic as { readonly selected?: boolean } | null)?.selected).toBe(true);

    clickWidget(app, "kickoff");
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("counter");

    clickWidget(app, "complete");
    clickWidget(app, "change_plan");
    const layout = menuLayout(app.stack.current());
    const selected = layout ? hit.find(layout, "tactic_counter") : null;
    expect((selected as { readonly selected?: boolean } | null)?.selected).toBe(true);
    const shape = layout ? hit.find(layout, "formation_1-1-2") : null;
    expect((shape as { readonly selected?: boolean } | null)?.selected).toBe(true);
  });

  it("maps keyboard and gamepad through nested shell routes", () => {
    const app = newApp();
    // PLAY -> TEAM -> MULTIPLAYER -> HELP: one more "down" than before
    // TEAM (#600) joined the title menu.
    app.event({ kind: "key", key: "down" });
    app.event({ kind: "key", key: "down" });
    app.event({ kind: "key", key: "down" });
    app.event({ kind: "gamepad", button: "a" });
    expect(app.currentRoute()).toBe("help");
    app.event({ kind: "gamepad", button: "b" });
    expect(app.currentRoute()).toBe("title");
  });

  it("ships combat on, and lets the team sheet's toggle turn it off", () => {
    const app = newApp();
    reachFakeMatch(app);
    expect(app.session.combatEnabled).toBe(true);
    const withCombat = session.buildRequest(MATCH_CONTRACT_CONTENT, app.session, 4);
    expect(withCombat.ok && withCombat.value.combat_enabled).toBe(true);

    clickWidget(app, "cancel");
    expect(app.currentRoute()).toBe("team_sheet");
    clickWidget(app, "combat");
    clickWidget(app, "kickoff");
    expect(app.session.combatEnabled).toBe(false);
    const withoutCombat = session.buildRequest(MATCH_CONTRACT_CONTENT, app.session, 5);
    expect(withoutCombat.ok && withoutCombat.value.combat_enabled).toBe(false);
  });

  it("reaches credits through Settings -> About, not from the front door", () => {
    const app = newApp();
    const titleLayout = menuLayout(app.stack.current());
    expect(titleLayout ? hit.find(titleLayout, "credits") : null).toBeNull();

    clickWidget(app, "settings");
    clickWidget(app, "credits");
    expect(app.currentRoute()).toBe("credits");
    app.event({ kind: "key", key: "escape" });
    expect(app.currentRoute()).toBe("settings");
  });

  it("has no Quit button, and still honours the window-close gesture", () => {
    const app = newApp();
    const titleLayout = menuLayout(app.stack.current());
    expect(titleLayout ? hit.find(titleLayout, "quit") : null).toBeNull();
    app.event({ kind: "key", key: "escape" });
    expect(app.quitRequested).toBe(true);
  });

  it("opens the multiplayer front door instead of throwing at a dev lobby", () => {
    const app = newApp();
    clickWidget(app, "multiplayer");
    expect(app.currentRoute()).toBe("multiplayer");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("title");
  });

  it("shows a dead online session its reason, instead of dropping to the title", () => {
    const app = newApp();
    app.routes = ["online_match"];
    app.handleAction({ go: "online_ended", terminal: { reason: "host_left" } });
    expect(app.currentRoute()).toBe("session_ended");
    const layout = menuLayout(app.stack.current());
    const headline = layout ? hit.find(layout, "headline") : null;
    expect((headline as { readonly text?: string } | null)?.text).toContain("host left");
    const detail = layout ? hit.find(layout, "detail") : null;
    expect((detail as { readonly text?: string } | null)?.text).toContain("host_left");

    clickWidget(app, "main_menu");
    expect(app.currentRoute()).toBe("title");
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

  it("cancels back to the team sheet", () => {
    const app = newApp();
    reachFakeMatch(app);
    clickWidget(app, "cancel");
    expect(app.currentRoute()).toBe("team_sheet");
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");
  });
});

describe("team persistence (#600)", () => {
  it("is reachable from the title menu on its own, and leaving returns to the title", () => {
    const app = newApp();
    const titleLayout = menuLayout(app.stack.current());
    expect(titleLayout ? hit.find(titleLayout, "team") : null).not.toBeNull();

    clickWidget(app, "team");
    expect(app.currentRoute()).toBe("team_sheet");

    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("title");
  });

  it("keeps an edit made on a standalone TEAM visit after BACK, and shows it on the next visit", () => {
    const app = newApp();
    clickWidget(app, "team");
    expect(app.currentRoute()).toBe("team_sheet");
    clickWidget(app, "formation_1-1-2");
    clickWidget(app, "tactic_press_high");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("title");

    // BACK committed the edit -- it was never near "kickoff".
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");

    clickWidget(app, "team");
    expect(app.currentRoute()).toBe("team_sheet");
    const layout = menuLayout(app.stack.current());
    const formation = layout ? hit.find(layout, "formation_1-1-2") : null;
    expect((formation as { readonly selected?: boolean } | null)?.selected).toBe(true);
    const tactic = layout ? hit.find(layout, "tactic_press_high") : null;
    expect((tactic as { readonly selected?: boolean } | null)?.selected).toBe(true);
  });

  it("persists a standalone TEAM edit to storage on BACK, not only on kickoff", () => {
    const storage = memoryStorage();
    const app = newApp({ teamSettingsStorage: storage });
    clickWidget(app, "team");
    clickWidget(app, "formation_1-2-1");
    clickWidget(app, "tactic_counter");
    clickWidget(app, "combat");
    clickWidget(app, "back");
    expect(app.currentRoute()).toBe("title");

    const saved = teamSettings.load(storage);
    expect(saved.formationId).toBe("1-2-1");
    expect(saved.tacticId).toBe("counter");
    expect(saved.combatEnabled).toBe(false);
  });

  it("boots the session seeded from persisted, content-valid team preferences", () => {
    const storage = memoryStorage(
      teamSettings.serialize({
        starterIds: ["ozzo", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin"],
        formationId: "1-1-2",
        tacticId: "press_high",
        combatEnabled: false,
        lastOnlineMode: "2v2",
        lastBotFill: true,
      }),
    );
    const app = new App(APP_CONTENT, { teamSettingsStorage: storage });
    expect(app.session.starterIds).toEqual([
      "ozzo",
      "veil_nyx",
      "rok_tann",
      "mika_olu",
      "sela_dwin",
    ]);
    expect(app.session.formationId).toBe("1-1-2");
    expect(app.session.tacticId).toBe("press_high");
    expect(app.session.combatEnabled).toBe(false);
    expect(app.lastOnlineMode).toBe("2v2");
    expect(app.lastBotFill).toBe(true);
  });

  it("falls back to the home team's own defaults when a stored id no longer exists in content", () => {
    const storage = memoryStorage(
      teamSettings.serialize({
        starterIds: ["ozzo", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin"],
        formationId: "a-shape-nobody-authors-any-more",
        tacticId: "a-plan-nobody-authors-any-more",
        combatEnabled: true,
        lastOnlineMode: "not-a-real-mode",
        lastBotFill: false,
      }),
    );
    const app = new App(APP_CONTENT, { teamSettingsStorage: storage });
    expect(app.session.formationId).toBe(NEBULA.formation);
    expect(app.session.tacticId).toBe("balanced");
    // Starters were valid and still take effect independently of the
    // formation/tactic fallback beside them.
    expect(app.session.starterIds).toEqual([
      "ozzo",
      "veil_nyx",
      "rok_tann",
      "mika_olu",
      "sela_dwin",
    ]);
    expect(app.lastOnlineMode).toBe("4v4"); // LOBBY_DEFAULT_MODE
  });

  it("boots from a missing/corrupt storage port without throwing, at the home team's defaults", () => {
    const corrupt: SettingsStorage = {
      read: () => "\0garbage\0not the wire format",
      write: () => ok(true),
    };
    expect(() => new App(APP_CONTENT, { teamSettingsStorage: corrupt })).not.toThrow();
    const app = new App(APP_CONTENT, { teamSettingsStorage: corrupt });
    expect(app.session.starterIds).toEqual([...NEBULA.roster]);
    expect(app.session.formationId).toBe(NEBULA.formation);

    const noStorage = new App(APP_CONTENT);
    expect(noStorage.session.starterIds).toEqual([...NEBULA.roster]);
  });

  it("saves the team sheet's committed choices, ready to be re-validated as content next boot", () => {
    const storage = memoryStorage();
    const app = newApp({ teamSettingsStorage: storage });
    clickWidget(app, "play");
    clickWidget(app, "formation_1-1-2");
    clickWidget(app, "tactic_press_high");
    clickWidget(app, "combat");
    clickWidget(app, "kickoff");
    expect(app.currentRoute()).toBe("match");

    const saved = teamSettings.load(storage);
    expect(saved.formationId).toBe("1-1-2");
    expect(saved.tacticId).toBe("press_high");
    expect(saved.combatEnabled).toBe(false);
    expect(saved.starterIds.length).toBe(5);

    // A fresh boot against the same storage reproduces the exact session
    // the player left with, once resolved against content.
    const rebooted = new App(APP_CONTENT, { teamSettingsStorage: storage });
    expect(rebooted.session.formationId).toBe("1-1-2");
    expect(rebooted.session.tacticId).toBe("press_high");
    expect(rebooted.session.combatEnabled).toBe(false);
  });

  it("seeds the multiplayer mode picker from the persisted last online mode, and re-saves it on a host commit", () => {
    const storage = memoryStorage(
      teamSettings.serialize({ ...teamSettings.defaults(), lastOnlineMode: "2v2" }),
    );
    const onlinePorts: OnlinePorts = {
      matchManifestTemplate: undefined,
      requestMatchSession: () => ({ ok: false, error: "not exercised by this case" }),
      newLobbyScreen: () => ({ state: { model: {} }, link: undefined }),
      newOnlineMatchScreen: () => {
        throw new Error("not exercised by this case");
      },
    };
    const app = new App(APP_CONTENT, { teamSettingsStorage: storage, online: onlinePorts });
    clickWidget(app, "multiplayer");
    const layout = menuLayout(app.stack.current());
    const modeWidget = layout ? hit.find(layout, "mode_2v2") : null;
    expect(
      (modeWidget as { readonly selected?: boolean } | null)?.selected,
      "the persisted mode is pre-selected on the multiplayer front door",
    ).toBe(true);

    clickWidget(app, "mode_4v4");
    clickWidget(app, "host");
    expect(app.currentRoute()).toBe("lobby");
    expect(app.lastOnlineMode).toBe("4v4");
    expect(teamSettings.load(storage).lastOnlineMode).toBe("4v4");
  });

  it("seeds a hosted lobby's bot fill from persisted preferences, and saves the lobby's own choice back on leaving", () => {
    const storage = memoryStorage(
      teamSettings.serialize({
        ...teamSettings.defaults(),
        lastOnlineMode: "2v2",
        lastBotFill: true,
      }),
    );
    let capturedOptions:
      { readonly role?: string; readonly mode?: string; readonly botFill?: boolean } | undefined;
    const modelBox = { bot_fill: true };
    const onlinePorts: OnlinePorts = {
      matchManifestTemplate: undefined,
      requestMatchSession: () => ({ ok: false, error: "not exercised by this case" }),
      newLobbyScreen: (_onAction, options) => {
        capturedOptions = options as typeof capturedOptions;
        return { state: { model: modelBox }, link: undefined };
      },
      newOnlineMatchScreen: () => {
        throw new Error("not exercised by this case");
      },
    };
    const app = new App(APP_CONTENT, { teamSettingsStorage: storage, online: onlinePorts });

    clickWidget(app, "multiplayer");
    clickWidget(app, "host");
    expect(app.currentRoute()).toBe("lobby");
    expect(capturedOptions?.role).toBe("host");
    expect(capturedOptions?.botFill).toBe(true);

    // The host turns bot fill back off inside the lobby -- `App` has no
    // `AppAction` for that toggle (`lobby_model.ts`'s own command stays
    // internal to the screen), so leaving reads it straight off the
    // departing screen's model instead (`OnlineLobbyScreen.state.model.bot_fill`).
    modelBox.bot_fill = false;
    app.handleAction({ go: "main_menu" });
    expect(app.currentRoute()).toBe("title");

    const saved = teamSettings.load(storage);
    expect(saved.lastBotFill).toBe(false);
    expect(saved.lastOnlineMode).toBe("2v2");
  });

  it("never lets a guest's always-false bot fill overwrite a host's real preference", () => {
    const storage = memoryStorage(
      teamSettings.serialize({ ...teamSettings.defaults(), lastBotFill: true }),
    );
    const onlinePorts: OnlinePorts = {
      matchManifestTemplate: undefined,
      requestMatchSession: () => ({ ok: false, error: "not exercised by this case" }),
      newLobbyScreen: () => ({ state: { model: { bot_fill: false } }, link: undefined }),
      newOnlineMatchScreen: () => {
        throw new Error("not exercised by this case");
      },
    };
    const app = new App(APP_CONTENT, { teamSettingsStorage: storage, online: onlinePorts });
    clickWidget(app, "multiplayer");
    clickWidget(app, "join"); // role: guest
    expect(app.currentRoute()).toBe("lobby");
    app.handleAction({ go: "main_menu" });
    expect(teamSettings.load(storage).lastBotFill).toBe(true);
  });
});
