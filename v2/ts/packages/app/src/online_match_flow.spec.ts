// Ported from spec/screens/online_match_flow_spec.lua's "routes the lobby's
// synchronized start into the online match" -- left `describe.skip`/`it.skip`
// in `@gc/screens` (packages/screens/src/online_match_flow.spec.ts) because
// `@gc/app` depends on `@gc/screens`, never the reverse, so `App` (the
// module this case is actually about) is unreachable from that package.
// `@gc/app` is the correct side of that edge; this file is where the case
// now lives.
//
// # What this case can and cannot prove here
//
// The Lua original mounts two REAL `App`s, drives a REAL `OnlineLobby`
// through a real manual offer/answer handshake over a fake star transport
// (`game.transport.fake_star`/`fake_star_rendezvous`), and lets `game.online
// .coordinator` (a ~3,300-line replicated state machine: admission,
// assignment, the pair-preference protocol, readiness, the countdown/freeze
// barrier) run for real to reach a synchronized start.
//
// None of that machinery is available to reproduce for real from this
// package:
//   - `game.online.match_manifest`/`match_session` are Rust-owned
//     (`gc-netcode`) with no `@gc/wasm` bridge -- confirmed by
//     `app.ts`'s own header and by `online_match_flow.spec.ts`
//     (`@gc/screens`) injecting a `MatchSessionPort` fake even for its own
//     real-`Coordinator` cases.
//   - `@gc/screens`'s `lobby_flow.spec.ts` already builds the one hand-written
//     stand-in for `game.online.coordinator`'s assignment/preference/
//     readiness/countdown protocol this codebase has -- roughly 1,600 lines,
//     deliberately scoped to that package because reimplementing the
//     coordinator a second time would create a second source of truth for
//     exactly the kind of state two peers must agree on bit for bit (see
//     that file's own header). Duplicating it here, one layer up, would add
//     no protocol-correctness coverage beyond what that file already gives
//     (Rust owns correctness; `crates/gc-netcode/tests` is where that's
//     proven) -- it would only be a second copy of the same fake.
//   - A real two-peer signaling handshake needs `@gc/transport`'s fake star,
//     which is not a declared dependency of `@gc/app` (package.json) and
//     this port may not edit `package.json`.
//
// So this case is ported at the boundary `App` actually owns: `OnlinePorts`
// (app.ts). What it proves is real and was, before this move, entirely
// unverified: `App.startOnlineMatch` used to be an unconditional stub that
// threw without ever reading the mounted lobby's state (see app.ts's
// header) -- a defect this move exposed and app.ts's port now fixes for
// real. This file drives that fixed code path -- reading
// `lobby.state.model.coordinator`/`lobby.link`, calling
// `requestMatchSession`, constructing the match screen, pushing the
// `online_match` route, and the focus/controller-loss behavior once there
// -- through a hand-written `OnlinePorts`, at the same level of fakery
// `app.spec.ts`'s own fake match adapter already uses for the offline
// flow. It does not, and cannot from here, prove `game.online.coordinator`'s
// own admission/assignment/readiness logic -- that is Rust's job.

import { describe, expect, it } from "vitest";
import { App, type OnlineLobbyCoordinatorState, type OnlineLobbyScreen, type OnlineMatchScreen, type OnlinePorts } from "./app.ts";
import { APP_CONTENT } from "./test_support/fixtures.ts";

interface FakeRequest {
  readonly mode: string;
  readonly owned: readonly string[];
}

function fakeLobbyScreen(coordinator?: OnlineLobbyCoordinatorState, link?: unknown): OnlineLobbyScreen & { teardownCalls: number } {
  return {
    state: { model: coordinator !== undefined ? { coordinator } : {} },
    link,
    teardownCalls: 0,
    teardown(): void {
      this.teardownCalls += 1;
    },
  };
}

function fakeOnlineMatchScreen(request: FakeRequest): OnlineMatchScreen & {
  readonly request: FakeRequest;
  focusLostCalls: number;
  controllerLostCalls: number;
} {
  return {
    request,
    focusLostCalls: 0,
    controllerLostCalls: 0,
    focusLost(): void {
      this.focusLostCalls += 1;
    },
    controllerLost(): void {
      this.controllerLostCalls += 1;
    },
  };
}

// Builds an `OnlinePorts` whose lobby is a fixed fake (settable by the
// caller before dispatching the `online_match` action, standing in for
// "the lobby's coordinator reached a frozen session") and whose match
// session request always succeeds with `matchRequest`, mirroring
// `match_session.request`'s ok path.
function fakeOnlinePorts(
  lobby: OnlineLobbyScreen,
  matchRequest: FakeRequest,
): { readonly ports: OnlinePorts; readonly matchScreens: ReturnType<typeof fakeOnlineMatchScreen>[] } {
  const matchScreens: ReturnType<typeof fakeOnlineMatchScreen>[] = [];
  const ports: OnlinePorts = {
    matchManifestTemplate: undefined,
    requestMatchSession: () => ({ ok: true, value: matchRequest }),
    newLobbyScreen: () => lobby,
    newOnlineMatchScreen: (options) => {
      const screen = fakeOnlineMatchScreen(options.request as FakeRequest);
      matchScreens.push(screen);
      return screen;
    },
  };
  return { ports, matchScreens };
}

describe("online match app routing", () => {
  it("routes the lobby's synchronized start into the online match", () => {
    const lobby = fakeLobbyScreen({ role: "host", peer_id: "host", manifest: { match_mode: "2v2" } }, { star: {} });
    const { ports, matchScreens } = fakeOnlinePorts(lobby, { mode: "2v2", owned: ["home_1", "home_2"] });
    const app = new App(APP_CONTENT, { online: ports });

    app.handleAction({ go: "online_lobby" });
    expect(app.currentRoute()).toBe("lobby");

    app.handleAction({ go: "online_match", freeze: { countdown_id: "countdown.1" } });

    expect(app.currentRoute()).toBe("online_match");
    expect(app.onlineError).toBeUndefined();
    const screen = matchScreens[0]!;
    expect(screen.request.mode).toBe("2v2");
    expect(screen.request.owned.length).toBe(2);

    // Focus loss on the online route must not push a pause screen.
    app.focus(false);
    expect(app.currentRoute()).toBe("online_match");
    expect(screen.focusLostCalls).toBe(1);

    app.controllerRemoved();
    expect(app.currentRoute()).toBe("online_match");
    expect(screen.controllerLostCalls).toBe(1);
  });

  it("reports an online error and stays on the lobby route when the coordinator has no frozen manifest", () => {
    const lobby = fakeLobbyScreen(undefined, undefined);
    const { ports, matchScreens } = fakeOnlinePorts(lobby, { mode: "2v2", owned: [] });
    const app = new App(APP_CONTENT, { online: ports });

    app.handleAction({ go: "online_lobby" });
    app.handleAction({ go: "online_match", freeze: { countdown_id: "countdown.1" } });

    expect(app.currentRoute()).toBe("lobby");
    expect(app.onlineError).toBe("the lobby has no frozen session to play");
    expect(matchScreens.length).toBe(0);
  });
});
