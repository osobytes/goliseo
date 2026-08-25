// Proves the REAL two-click journey #610 promises, at the level of reality
// the issue asks for: the ONLY navigation path a player has -- title ->
// PLAY ONLINE -- must land directly on an already-live hosting screen, and
// START MATCH must be the only other click needed to reach countdown with
// a real, independently-connected guest. This supersedes #597's own
// front-door-card-click premise (#610 round-2 review, blocking finding 1:
// the front door folded into the hosting screen itself, and
// `multiplayer.ts`'s "OPEN A LOBBY"/"USE AN INVITE" cards no longer
// exist) -- kept as the SAME file rather than a new one, since the
// underlying reason it exists is identical: a front door that sends the
// wrong shape of action into a real lobby is exactly how #597 shipped for
// two releases with a fully green suite, and only a real `App` ->
// `handleAction` -> real `OnlineLobby` chain catches that class of defect.
//
// # Why this file exists alongside `lobby_flow.spec.ts` and `room_code_lobby.spec.ts`
//
// `lobby_flow.spec.ts`'s "online lobby app routing" case drives the real
// title but substitutes `fakeEmptyLobbyScreen` for the mounted lobby (see
// that file's own header for why -- it is deliberately about `App`'s
// routing shell, nothing more). `room_code_lobby.spec.ts` drives a real
// `room_pick` handshake to a ready session, but starts from
// `newDispatchableLobby`, which calls `OnlinePorts.newLobbyScreen` directly
// with only a `template` option -- it never goes through `App.handleAction`'s
// routing at all. This file drives every step for real: the title's own
// action on a card click, `App.handleAction`'s routing of it, and a REAL
// `OnlineLobby`, mounted the same way production does (`online_ports.ts`'s
// `createOnlinePorts`).
//
// Fakery level mirrors `room_code_lobby.spec.ts` exactly: `fakeStar`/
// `fakeStarRendezvous` (`@gc/transport`) for the star transport, and a
// room-code relay fake (`test_support/fake_room_rendezvous.ts`, shared with
// that file) built only from this issue's own `RoomSignalingFactory`/
// `RoomSignalingHandle`/`RoomSignalingEvent` port shapes (never a real
// WebSocket) -- see that module's header for the same disclosed gap (no
// real two-browser WebRTC/WebSocket round trip).

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar, fakeStarRendezvous, type StarTransportAdapter } from "@gc/transport";
import {
  lobbyScreenLayout,
  type LobbyScreenState,
  type RoomSignalingEvent,
  type RoomSignalingHandle,
} from "@gc/screens";
import { createOnlinePorts, type OnlinePortsDeps } from "./online_ports.ts";
import type { OnlineWasmHost } from "./online_wasm_host.ts";
import { App } from "./app.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { APP_CONTENT, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";
import { fakeRoomRendezvous } from "./test_support/fake_room_rendezvous.ts";

function nodeWasmHost(): OnlineWasmHost {
  const sim = loadSimHost();
  return sim as unknown as OnlineWasmHost;
}

// Drives a real `App`'s widgets exactly like a player's click would --
// `menuLayout` only reads `Menu`-wrapped screens (title), so this only
// ever targets it, never the mounted `OnlineLobby` itself (which is not
// `Menu`-wrapped -- `online_ports.spec.ts`'s own header notes the same
// thing). Mirrors `online_ports.spec.ts`'s identically-named helper.
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

/** The real `OnlineLobby`'s own read/drive surface, once mounted --
 * `dispatch`/`event`/`update`/`state` are all public on the class itself
 * (`online_lobby.ts`); `app.ts`'s narrower `OnlineLobbyScreen` just does not
 * declare them. Mirrors `room_code_lobby.spec.ts`'s identically-shaped
 * `DispatchableLobby`, plus `event` -- needed below to drive a REAL click
 * (and real keystrokes) on the lobby's own screen, since `OnlineLobby` is
 * not `Menu`-wrapped and so cannot be reached by `clickWidget`'s
 * `menuLayout` helper. */
interface DispatchableLobby {
  dispatch(command: { readonly kind: string; readonly [key: string]: unknown }): void;
  event(evt: { readonly kind: string; readonly [key: string]: unknown }): void;
  update(dt: number): void;
  readonly state: LobbyScreenState;
}

function currentLobby(app: App): DispatchableLobby {
  return app.stack.current() as unknown as DispatchableLobby;
}

/** A real click on one of the mounted `OnlineLobby`'s own widgets --
 * `clickWidget`'s counterpart for a screen `menuLayout` cannot see. */
function clickLobbyWidget(lobby: DispatchableLobby, id: string): void {
  const layout = lobbyScreenLayout(lobby.state);
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    throw new Error(`missing lobby widget ${id}`);
  }
  lobby.event({
    kind: "click",
    x: widget.rect.x + widget.rect.w / 2,
    y: widget.rect.y + widget.rect.h / 2,
    button: 1,
  });
}

interface Pumpable {
  pump(): void;
}
type TestStar = StarTransportAdapter & Pumpable;

/** Mirrors `room_code_lobby.spec.ts`'s identically-named helper. */
function pump(
  lobbies: readonly DispatchableLobby[],
  stars: readonly Pumpable[],
  cycles = 10,
): void {
  for (let i = 0; i < cycles; i += 1) {
    for (const star of stars) {
      star.pump();
    }
    for (const lobby of lobbies) {
      lobby.update(1 / 60);
    }
  }
}

/** A minimal, host-only room-code relay fake: `openHost` alone, confirming
 * a code -- for cases that only need PLAY ONLINE's own auto-hosting
 * attempt to resolve, never a second peer. Structurally
 * `RoomSignalingFactory`/`RoomSignalingHandle`, mirroring
 * `test_support/fake_room_rendezvous.ts`'s fuller two-way fake. */
function fakeHostOnlyRoomSignaling(code: string): {
  openHost(): RoomSignalingHandle;
  openGuest(roomCode: string): RoomSignalingHandle;
} {
  return {
    openHost(): RoomSignalingHandle {
      let queue: RoomSignalingEvent[] = [{ kind: "created", code }];
      return {
        poll: () => {
          const drained = queue;
          queue = [];
          return drained;
        },
        send: () => {},
        close: () => {},
      };
    },
    openGuest(): RoomSignalingHandle {
      throw new Error("not exercised by this file -- see room_code_lobby.spec.ts");
    },
  };
}

/** A relay that fails the host's request outright, instead of confirming a
 * code -- for the case where PLAY ONLINE's auto-hosting attempt never
 * resolves to a role at all. */
function fakeFailingHostRoomSignaling(reason: string): {
  openHost(): RoomSignalingHandle;
  openGuest(roomCode: string): RoomSignalingHandle;
} {
  return {
    openHost(): RoomSignalingHandle {
      let queue: RoomSignalingEvent[] = [{ kind: "failed", reason }];
      return {
        poll: () => {
          const drained = queue;
          queue = [];
          return drained;
        },
        send: () => {},
        close: () => {},
      };
    },
    openGuest(): RoomSignalingHandle {
      throw new Error("not exercised by this file -- see room_code_lobby.spec.ts");
    },
  };
}

function newApp(roomSignaling?: OnlinePortsDeps["roomSignaling"]): {
  app: App;
  stars: TestStar[];
  /** The star rendezvous backing this app's own `starFactory` -- returned
   * so a SEPARATE, independently-constructed lobby (`newIndependentLobby`
   * below) can join the exact same in-process star network, the way a
   * second browser tab shares nothing but the transport. */
  starRendezvous: ReturnType<typeof fakeStarRendezvous>;
} {
  const rendezvous = fakeStarRendezvous();
  const stars: TestStar[] = [];
  const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
    const star = fakeStar({ role, rendezvous, peer_id: peerId });
    if (!star.initialize().ok) {
      return undefined;
    }
    stars.push(star);
    return star;
  };
  const onlinePorts = createOnlinePorts({
    wasm: nodeWasmHost(),
    starFactory,
    renderer: noopRenderPort,
    keyboard: fakeKeyboard(),
    content: APP_CONTENT.matchContract,
    ...(roomSignaling !== undefined ? { roomSignaling } : {}),
  });
  return { app: new App(APP_CONTENT, { online: onlinePorts }), stars, starRendezvous: rendezvous };
}

/** A second, independently-constructed lobby joining the SAME star and
 * room-code rendezvous the app under test uses -- exactly what a friend's
 * separate browser tab is, mirroring `room_code_lobby.spec.ts`'s
 * `newDispatchableLobby`. Generic over role: the caller dispatches
 * `room_pick` itself, so this serves both "a friend's own separately
 * hosted room" and "a friend joining the room under test." */
function newIndependentLobby(
  starRendezvous: ReturnType<typeof fakeStarRendezvous>,
  roomRendezvous: OnlinePortsDeps["roomSignaling"],
  stars: TestStar[],
): DispatchableLobby {
  const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
    const star = fakeStar({ role, rendezvous: starRendezvous, peer_id: peerId });
    if (!star.initialize().ok) {
      return undefined;
    }
    stars.push(star);
    return star;
  };
  const onlinePorts = createOnlinePorts({
    wasm: nodeWasmHost(),
    starFactory,
    renderer: noopRenderPort,
    keyboard: fakeKeyboard(),
    content: APP_CONTENT.matchContract,
    ...(roomRendezvous !== undefined ? { roomSignaling: roomRendezvous } : {}),
  });
  return onlinePorts.newLobbyScreen(() => {}, {
    template: onlinePorts.matchManifestTemplate,
  }) as unknown as DispatchableLobby;
}

describe("PLAY ONLINE: the #610 two-click host journey", () => {
  it("PLAY ONLINE, then START MATCH -- exactly two activations -- reaches countdown with a real relay-connected guest", () => {
    const roomRendezvous = fakeRoomRendezvous();
    const { app, stars, starRendezvous } = newApp(roomRendezvous);
    // 1v1 needs only one connected guest -- set directly (mirroring how a
    // real player's PREVIOUS session already persisted this choice,
    // `team_settings.ts`) rather than a third click on a mode chip, so the
    // two `clickWidget`/`clickLobbyWidget` calls below really are the
    // WHOLE journey, matching #610's "two clicks total" literally.
    app.lastOnlineMode = "1v1";

    // ACTIVATION 1 of 2: PLAY ONLINE, from the title. Lands directly on
    // the already-live hosting screen -- a room code appears with no
    // further click (#598/#603) -- so this one click is the entire "reach
    // the hosting screen" side of the journey; there is no front-door card
    // screen left to click through.
    clickWidget(app, "multiplayer");
    expect(app.currentRoute()).toBe("lobby");

    const lobby = currentLobby(app);
    pump([lobby], stars);
    const code = lobby.state.model.room_code;
    if (code === undefined) {
      throw new Error("the host must have a room code after a pump cycle");
    }
    expect(lobby.state.model.role).toBe("host");
    expect(lobby.state.model.mode).toBe("1v1");

    // A friend joins over the real relay wire -- constructed
    // independently, never through a click on THIS app -- exactly what a
    // second browser tab is. This is not one of the two activations under
    // test: nobody clicked anything on the host's OWN screen to make it
    // happen.
    const guest = newIndependentLobby(starRendezvous, roomRendezvous, stars);
    guest.dispatch({ kind: "room_pick", role: "guest" });
    for (const ch of code) {
      guest.dispatch({ kind: "room_key", key: ch });
    }
    guest.dispatch({ kind: "room_submit" });
    pump([lobby, guest], stars, 60);
    expect(guest.state.model.role).toBe("guest");
    expect(guest.state.model.coordinator?.role).toBe("guest");

    // ACTIVATION 2 of 2: START MATCH. Locks the mode, publishes ownership,
    // marks the host ready, and begins the countdown -- one command, no
    // LOCK MATCH and no READY toggle anywhere in this journey.
    clickLobbyWidget(lobby, "start");
    pump([lobby, guest], stars, 60);

    expect(lobby.state.model.coordinator?.phase).toBe("countdown");
    expect(guest.state.model.coordinator?.phase).toBe("countdown");

    pump([lobby, guest], stars, 250);
    expect(lobby.state.model.started).toBe(true);
    expect(guest.state.model.started).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// #610 round-2 review, blocking finding 1e/1c: the manual-signaling
// fallback (#597's own acceptance criterion) must survive an auto-hosted
// room too, and a player who meant to join, not host, must be able to say
// so from the SAME screen -- neither is a normal forward navigation, so
// both go through `app.ts`'s `restartLobby` (`ScreenStack.replace`, which
// tears the old screen down before mounting a fresh one).
// ---------------------------------------------------------------------------

describe("PLAY ONLINE: CANCEL and the inline guest composer (#610 round-2 review, blocking finding 1)", () => {
  it("CANCEL from the auto-hosted screen restarts to the role screen, where the manual fallback still lives", () => {
    const { app, stars } = newApp(fakeHostOnlyRoomSignaling("7F3K9Q"));

    clickWidget(app, "multiplayer");
    const lobby = currentLobby(app);
    pump([lobby], stars);
    expect(lobby.state.model.role).toBe("host");

    const beforeRestart = app.stack.current();
    clickLobbyWidget(lobby, "cancel_to_role");
    // A genuinely fresh screen -- not the same `OnlineLobby` instance with
    // its role reset in place (`restartLobby`'s own doc on why: reusing a
    // live star transport mid-flight risks exactly the orphaned-connection
    // defect round-2 council review caught for room-code retries).
    expect(app.stack.current()).not.toBe(beforeRestart);

    const restarted = currentLobby(app);
    expect(restarted.state.model.role).toBeUndefined();
    const roleScreen = lobbyScreenLayout(restarted.state);
    const roleHost = hit.find(roleScreen, "role_host");
    expect(roleHost, "the manual role screen must be reachable after CANCEL").not.toBeNull();
    expect(roleHost?.data?.disabled).toBe(false);
  });

  it("typing a friend's code anywhere on the auto-hosted screen switches this player into a guest of it", () => {
    const roomRendezvous = fakeRoomRendezvous();
    const { app, stars, starRendezvous } = newApp(roomRendezvous);

    // A friend's own, separately hosted room -- sharing nothing with this
    // app but the same in-process star/relay, exactly what a second
    // browser tab's host would be.
    const friendHost = newIndependentLobby(starRendezvous, roomRendezvous, stars);
    friendHost.dispatch({ kind: "room_pick", role: "host" });
    pump([friendHost], stars);
    const friendCode = friendHost.state.model.room_code;
    if (friendCode === undefined) {
      throw new Error("the friend's host must have a room code after a pump cycle");
    }

    clickWidget(app, "multiplayer");
    const lobby = currentLobby(app);
    pump([lobby, friendHost], stars);
    expect(lobby.state.model.role).toBe("host");

    // Typing the friend's code -- no prior click on the composer itself --
    // focuses it and feeds every character through (`lobby.ts`'s
    // `update()`, "type anywhere" doc).
    for (const ch of friendCode) {
      lobby.event({ kind: "key", key: ch, pressed: true });
    }
    const beforeRestart = app.stack.current();
    clickLobbyWidget(lobby, "join_code_entry");
    expect(app.stack.current()).not.toBe(beforeRestart);

    const restarted = currentLobby(app);
    pump([restarted, friendHost], stars, 60);
    expect(restarted.state.model.role).toBe("guest");
    expect(restarted.state.model.coordinator?.role).toBe("guest");
  });
});

// Round-2 council review, blocking finding 1 (PR #603, predating #610):
// `OnlineLobby` used to apply the front door's chosen mode ONLY inside its
// `room_created` handling -- so a room-hosting attempt that never reaches
// `room_created` (a relay failure, here) silently dropped it. The player
// still has a way to become host: CANCEL back to the role screen (proven
// above) and pick "HOST A SESSION" manually. That manual pick resolves
// `role === "host"` through a completely different code path
// (`OnlineLobby.event`, a real click, never `dispatch`) -- proving the fix
// has to live wherever host role resolution happens, not just in the one
// command that used to be the only way there.
describe("a failed auto-hosting attempt still falls back to the manual role, mode preserved", () => {
  it("applies the persisted mode once the host picks the manual role after a relay failure", () => {
    const { app, stars } = newApp(fakeFailingHostRoomSignaling("handshake_failed"));
    app.lastOnlineMode = "2v2";

    clickWidget(app, "multiplayer");
    expect(app.currentRoute()).toBe("lobby");

    const lobby = currentLobby(app);
    expect(lobby.state.model.room_active).toBe(true);

    // The fake relay reports failure -- the room-hosting attempt ends with
    // no role ever resolved, and the role screen (with its manual
    // fallback) becomes reachable again.
    pump([lobby], stars);
    expect(lobby.state.model.role).toBeUndefined();
    expect(lobby.state.model.room_active).toBe(false);
    const roleScreen = lobbyScreenLayout(lobby.state);
    const roleHost = hit.find(roleScreen, "role_host");
    expect(roleHost, "the manual role screen must be reachable after a failure").not.toBeNull();
    expect(roleHost?.data?.disabled).toBe(false);

    // The player picks "HOST A SESSION" manually -- a real click, exactly
    // what a player does, going through `OnlineLobby.event`, not
    // `dispatch`.
    clickLobbyWidget(lobby, "role_host");
    expect(lobby.state.model.role).toBe("host");

    // The persisted mode (seeded before "PLAY ONLINE" was ever clicked)
    // must still reach the manifest this host proposes.
    expect(lobby.state.model.mode).toBe("2v2");
    lobby.dispatch({ kind: "bot_fill" });
    lobby.dispatch({ kind: "lock" });
    expect(lobby.state.model.coordinator?.manifest?.match_mode).toBe("2v2");
  });
});
