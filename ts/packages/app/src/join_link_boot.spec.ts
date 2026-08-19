// Proves #598's boot-time join-link routing at the level of reality the
// issue itself asks for: a friend clicking `.../?room=<CODE>` must land, on
// its own, in a real mounted `OnlineLobby` with the code already submitted
// -- never the title screen, never a screen where the player has to type
// anything. `join_link.spec.ts` already covers `roomCodeFromSearch`'s own
// parsing rules headlessly; this file starts from an already-parsed code
// (standing in for `browser_main.ts`'s impure `window.location.search`
// read -- there is no `LocationPort` abstraction in this codebase, the
// same gap `ice_config.spec.ts`'s header discloses for `?ice=relay`) and
// drives the REST of the path for real: `App`'s constructor routing
// (`app.ts`'s `presetRoomCode` branch), and a real `OnlineLobby` mounted
// the same way production does (`online_ports.ts`'s `createOnlinePorts`).
//
// Fakery level mirrors `multiplayer_room_flow.spec.ts` exactly:
// `fakeStar`/`fakeStarRendezvous` (`@gc/transport`) for the star transport,
// and a room-code relay fake built only from this package's own
// `RoomSignalingFactory`/`RoomSignalingHandle`/`RoomSignalingEvent` port
// shapes (never a real WebSocket).

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar, fakeStarRendezvous, type StarTransportAdapter } from "@gc/transport";
import {
  lobbyScreenLayout,
  LOBBY_ROOM_FAILURE_TEXT,
  type LobbyScreenState,
  type RoomSignalingEvent,
  type RoomSignalingHandle,
} from "@gc/screens";
import { createOnlinePorts, type OnlinePortsDeps } from "./online_ports.ts";
import type { OnlineWasmHost } from "./online_wasm_host.ts";
import { App } from "./app.ts";
import { hit } from "./ui_bridge.ts";
import { roomCodeFromSearch } from "./join_link.ts";
import { APP_CONTENT, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";

function nodeWasmHost(): OnlineWasmHost {
  const sim = loadSimHost();
  return sim as unknown as OnlineWasmHost;
}

/** Mirrors `multiplayer_room_flow.spec.ts`'s identically-shaped helper. */
interface DispatchableLobby {
  dispatch(command: { readonly kind: string; readonly [key: string]: unknown }): void;
  event(evt: { readonly kind: string; readonly [key: string]: unknown }): void;
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

/** Mirrors `multiplayer_room_flow.spec.ts`'s identically-named helper. */
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

/** A minimal guest-only room-code relay fake: this file never drives the
 * host side (`multiplayer_room_flow.spec.ts`/`room_code_lobby.spec.ts` own
 * that coverage). Confirms the join with the code the caller submitted. */
function fakeGuestOnlyRoomSignaling(): NonNullable<OnlinePortsDeps["roomSignaling"]> {
  return {
    openHost(): RoomSignalingHandle {
      throw new Error("not exercised by this file -- see multiplayer_room_flow.spec.ts");
    },
    openGuest(code: string): RoomSignalingHandle {
      let queue: RoomSignalingEvent[] = [{ kind: "joined", code }];
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
  };
}

/** A relay that fails a guest's join attempt outright -- for the dead/
 * expired/full room a join link can point at. */
function fakeFailingGuestRoomSignaling(
  reason: string,
): NonNullable<OnlinePortsDeps["roomSignaling"]> {
  return {
    openHost(): RoomSignalingHandle {
      throw new Error("not exercised by this file -- see multiplayer_room_flow.spec.ts");
    },
    openGuest(): RoomSignalingHandle {
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
  };
}

function newApp(
  roomSignaling: OnlinePortsDeps["roomSignaling"],
  presetRoomCode: string,
): { app: App; stars: TestStar[] } {
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
  return { app: new App(APP_CONTENT, { online: onlinePorts, presetRoomCode }), stars };
}

describe("join-link boot routing (#598)", () => {
  it("?room=<code> lands boot directly in the lobby, with the code already submitted", () => {
    // Stands in for `browser_main.ts`'s impure `window.location.search`
    // read -- a lowercase code, proving the boot path carries the SAME
    // normalization `join_link.spec.ts` proves headlessly, not a second,
    // untested one.
    const code = roomCodeFromSearch("?room=a3f9k2");
    expect(code).toBe("A3F9K2");
    if (code === undefined) {
      throw new Error("expected a parsed room code");
    }

    const { app, stars } = newApp(fakeGuestOnlyRoomSignaling(), code);

    // No title screen, no multiplayer front door, no manual click at all --
    // boot itself routed straight to the lobby.
    expect(app.currentRoute()).toBe("lobby");
    const lobby = currentLobby(app);
    // The composer is already gone and the join already in flight -- proof
    // the code was not merely pre-filled but genuinely auto-submitted
    // (`room_submit`'s own effect, `room_open_guest`, already ran).
    expect(lobby.state.model.room_entry).toBeUndefined();
    expect(lobby.state.model.role).toBeUndefined();
    expect(lobby.state.model.room_active).toBe(true);
    expect(lobby.state.model.room_status).toBe("connecting");

    // The fake relay confirms the join -- `OnlineLobby.update(dt)` is where
    // that is polled and turned into `lobby_model.ts`'s `room_joined`.
    pump([lobby], stars);
    expect(lobby.state.model.role).toBe("guest");
    expect(lobby.state.model.room_status).toBe("connected");
  });

  // Failure honesty (#598, mirroring #602's own distinct room-failure
  // copy): a dead/expired/full room reached via a link is exactly as
  // reachable-to-retry as one reached by typing a code by hand -- never a
  // blank screen, never a wedged lobby.
  it("a dead room reached via a join link shows the standard failure copy, composer reachable to retry", () => {
    const { app, stars } = newApp(fakeFailingGuestRoomSignaling("room_expired"), "A3F9K2");

    expect(app.currentRoute()).toBe("lobby");
    const lobby = currentLobby(app);
    expect(lobby.state.model.room_active).toBe(true);

    // The fake relay reports the room has expired -- the auto-submitted
    // attempt ends with no role ever resolved.
    pump([lobby], stars);
    expect(lobby.state.model.role).toBeUndefined();
    expect(lobby.state.model.room_active).toBe(false);
    expect(lobby.state.model.room_error).toBe(LOBBY_ROOM_FAILURE_TEXT["room_expired"]);

    // The role screen -- with its "JOIN WITH A ROOM CODE" composer entry --
    // is reachable again, exactly the retry path a mistyped code already
    // leaves (#602).
    const roleScreen = lobbyScreenLayout(lobby.state);
    const retry = hit.find(roleScreen, "room_code_join");
    expect(retry, "the composer must be reachable to retry after a dead-link join").not.toBeNull();
    expect(retry?.data?.disabled).toBe(false);
  });
});
