// Proves #597's actual fix, at the level of reality the issue itself asks
// for: the ONLY navigation path a player has -- title -> the multiplayer
// front door -> a card click -- must reach the room-code widgets
// `lobby_model.ts`'s `room_pick` path exists to serve (#552).
//
// # Why this file exists alongside `lobby_flow.spec.ts` and `room_code_lobby.spec.ts`
//
// `lobby_flow.spec.ts`'s "online lobby app routing" case drives the real
// front door but substitutes `fakeEmptyLobbyScreen` for the mounted lobby
// (see that file's own header for why -- it is deliberately about `App`'s
// routing shell, nothing more). `room_code_lobby.spec.ts` drives a real
// `room_pick` handshake to a ready session, but starts from
// `newDispatchableLobby`, which calls `OnlinePorts.newLobbyScreen` directly
// with only a `template` option -- it never goes through the multiplayer
// screen or `App.handleAction`'s routing at all. Neither file can catch a
// front door that sends the WRONG shape of action into a real lobby: that
// is exactly how #597 shipped for two releases with a fully green suite
// (`multiplayer.ts` presetting a manual role `lobby_model.ts`'s `room_pick`
// path was never reachable from). This file drives every step for real:
// `multiplayer.update`'s own action on a card click, `App.handleAction`'s
// routing of that action, and a REAL `OnlineLobby`, mounted the same way
// production does (`online_ports.ts`'s `createOnlinePorts`), read back for
// the room-code widgets the front door is supposed to reach.
//
// Fakery level mirrors `room_code_lobby.spec.ts` exactly: `fakeStar`/
// `fakeStarRendezvous` (`@gc/transport`) for the star transport, and a
// room-code relay fake built only from this package's own
// `RoomSignalingFactory`/`RoomSignalingHandle`/`RoomSignalingEvent` port
// shapes (never a real WebSocket) -- see that file's header for the same
// disclosed gap (no real two-browser WebRTC/WebSocket round trip).

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar, fakeStarRendezvous, type StarTransportAdapter } from "@gc/transport";
import {
  lobbyScreenLayout,
  ROOM_CODE_ENTRY_WIDGET,
  type LobbyScreenState,
  type RoomSignalingEvent,
  type RoomSignalingHandle,
} from "@gc/screens";
import { createOnlinePorts, type OnlinePortsDeps } from "./online_ports.ts";
import type { OnlineWasmHost } from "./online_wasm_host.ts";
import { App } from "./app.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { APP_CONTENT, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";

function nodeWasmHost(): OnlineWasmHost {
  const sim = loadSimHost();
  return sim as unknown as OnlineWasmHost;
}

// Drives a real `App`'s widgets exactly like a player's click would --
// `menuLayout` only reads `Menu`-wrapped screens (title, multiplayer), so
// this only ever targets THOSE, never the mounted `OnlineLobby` itself
// (which is not `Menu`-wrapped -- `online_ports.spec.ts`'s own header notes
// the same thing). Mirrors `online_ports.spec.ts`'s identically-named
// helper.
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
 * `dispatch`/`update`/`state` are all public on the class itself
 * (`online_lobby.ts`); `app.ts`'s narrower `OnlineLobbyScreen` just does not
 * declare them. Mirrors `room_code_lobby.spec.ts`'s identically-shaped
 * `DispatchableLobby`. */
interface DispatchableLobby {
  dispatch(command: { readonly kind: string; readonly [key: string]: unknown }): void;
  update(dt: number): void;
  readonly state: LobbyScreenState;
}

function currentLobby(app: App): DispatchableLobby {
  return app.stack.current() as unknown as DispatchableLobby;
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

/** A minimal, host-only room-code relay fake: this file never drives a
 * guest through `room_submit` (that full round trip, both roles, is
 * `room_code_lobby.spec.ts`'s own job), only the host side of "a card click
 * reaches `room_open_host`, and a code comes back." Structurally
 * `RoomSignalingFactory`/`RoomSignalingHandle`, mirroring
 * `room_code_lobby.spec.ts`'s fuller `fakeRoomRendezvous`. */
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

function newApp(roomSignaling?: OnlinePortsDeps["roomSignaling"]): { app: App; stars: TestStar[] } {
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
  return { app: new App(APP_CONTENT, { online: onlinePorts }), stars };
}

describe("multiplayer front door -> a real online lobby (#597)", () => {
  it('"USE AN INVITE" lands on a focused room-code composer, with no extra clicks', () => {
    const { app } = newApp();

    clickWidget(app, "multiplayer");
    expect(app.currentRoute()).toBe("multiplayer");
    clickWidget(app, "join");
    expect(app.currentRoute()).toBe("lobby");

    const lobby = currentLobby(app);
    // The composer claims focus itself -- a player can start typing the
    // six characters immediately, the acceptance criterion #597 states in
    // its own words ("the code composer is on screen and focused").
    expect(lobby.state.focus).toBe(ROOM_CODE_ENTRY_WIDGET);
    const layout = lobbyScreenLayout(lobby.state);
    const composer = hit.find(layout, ROOM_CODE_ENTRY_WIDGET);
    expect(composer, "the room-code composer must be on screen for the join card").not.toBeNull();
    expect(composer?.focused).toBe(true);
    // The manual "JOIN WITH AN OFFER" screen -- the ONLY screen a guest
    // could actually reach before this fix (#597's own defect) -- must not
    // be what renders instead.
    expect(hit.find(layout, "role_guest")).toBeNull();
  });

  it('"OPEN A LOBBY" requests a room code immediately, and the front door\'s chosen mode reaches the proposed manifest', () => {
    const code = "7F3K9Q";
    const { app, stars } = newApp(fakeHostOnlyRoomSignaling(code));

    clickWidget(app, "multiplayer");
    clickWidget(app, "mode_2v2");
    clickWidget(app, "host");
    expect(app.currentRoute()).toBe("lobby");

    const lobby = currentLobby(app);
    // `room_pick`'s host path ran as the lobby's own opening command --
    // `room_open_host` already fired, before any further click.
    expect(lobby.state.model.room_active).toBe(true);
    expect(lobby.state.model.room_status).toBe("connecting");
    expect(lobby.state.model.role).toBeUndefined();

    // The fake relay reports the room created -- `OnlineLobby.update(dt)`
    // is where that's polled and turned into `lobby_model.ts`'s
    // `room_created` command (`online_lobby.ts`'s own `roomCommandFor`).
    pump([lobby], stars);

    expect(lobby.state.model.role).toBe("host");
    expect(lobby.state.model.room_code).toBe(code);
    const layout = lobbyScreenLayout(lobby.state);
    expect(hit.find(layout, "room_code_display")?.text).toContain(code);

    // The mode chosen on the front door BEFORE "OPEN A LOBBY" was clicked
    // must survive into the model...
    expect(lobby.state.model.mode).toBe("2v2");
    // ...and, more to the point, into the manifest the host actually
    // proposes -- proving it was dispatched at a point `lobby_model.ts`'s
    // `setMode` legally accepted it (a coordinator must already exist),
    // not silently dropped the way a naive "apply it up front" attempt
    // would be (`setMode` throws before `chooseRole` has ever run).
    lobby.dispatch({ kind: "bot_fill" });
    lobby.dispatch({ kind: "lock" });
    expect(lobby.state.model.coordinator?.manifest?.match_mode).toBe("2v2");
  });
});
