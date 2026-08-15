// Proves #552's room-code lobby path end to end, through this package's
// real production wiring (`online_ports.ts`'s `createOnlinePorts`, its
// `roomSignaling` option) -- mirroring `online_ports.spec.ts`'s two-peer
// manual-signaling handshake (study that file's header first) but driving
// a REAL host and REAL guest through the room-code path instead: "host with
// a room code" -> a code appears -> "join with a room code" -> type the
// code -> connected, with no `copy`/`paste` command ever dispatched by this
// spec. The star transport is the same `fakeStar`/`fakeStarRendezvous`
// `online_ports.spec.ts` already uses (in-process, no real WebRTC); the
// room-code signaling channel is `fakeRoomRendezvous` below, an in-process
// stand-in for `infra/src/room_durable_object.ts`'s relay, built only from
// this issue's own `@gc/online` `room_signaling.ts` types (`RoomSignalingEvent`/
// `RoomSignalingHandle`, re-exported structurally by `@gc/screens`'s
// `online_lobby.ts`) -- not a real WebSocket, the same "no DOM, same
// contract" trade `fakeStar` already makes for the star.
//
// What this proves: `lobby_model.ts`'s room-code commands
// (`room_pick`/`room_key`/`room_submit`/`room_created`/`room_joined`/
// `room_guest_joined`/`room_peer_signal`/...) drive a real session to
// readiness with the offer/answer blobs traveling entirely over the
// room-code channel, auto-sent and auto-imported (`lobby_model.ts`'s own
// header: no `copy`/`paste` click). It does NOT prove a real two-browser
// WebSocket round trip against the deployed Worker -- that is
// `docs/online/hosting_runbook.md`'s `wrangler dev` territory, disclosed as
// a residual risk in this issue's PR rather than claimed here.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar, fakeStarRendezvous, type StarTransportAdapter } from "@gc/transport";
import type { LobbyScreenState, RoomSignalingEvent, RoomSignalingHandle } from "@gc/screens";
import { createOnlinePorts, type OnlinePortsDeps } from "./online_ports.ts";
import type { OnlineWasmHost } from "./online_wasm_host.ts";
import { APP_CONTENT, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";

function nodeWasmHost(): OnlineWasmHost {
  const sim = loadSimHost();
  return sim as unknown as OnlineWasmHost;
}

interface DispatchableLobby {
  dispatch(command: { readonly kind: string; readonly [key: string]: unknown }): void;
  update(dt: number): void;
  readonly state: LobbyScreenState;
}

function newTestOnlinePorts(
  starFactory: OnlinePortsDeps["starFactory"],
  roomSignaling: OnlinePortsDeps["roomSignaling"],
) {
  return createOnlinePorts({
    wasm: nodeWasmHost(),
    starFactory,
    renderer: noopRenderPort,
    keyboard: fakeKeyboard(),
    content: APP_CONTENT.matchContract,
    ...(roomSignaling !== undefined ? { roomSignaling } : {}),
  });
}

function newDispatchableLobby(
  onlinePorts: ReturnType<typeof newTestOnlinePorts>,
  onAction: (action: { readonly go: string }) => void,
): DispatchableLobby {
  return onlinePorts.newLobbyScreen(onAction, {
    template: onlinePorts.matchManifestTemplate,
  }) as unknown as DispatchableLobby;
}

interface Pumpable {
  pump(): void;
}

/** Pumps every star's buffered delivery and `update(dt)`s every lobby --
 * `update(dt)` is also where `OnlineLobby` polls its room-code channel
 * (`online_lobby.ts`'s own `update()`), so one pump loop drives both the
 * star transport and the room-code relay. Mirrors `online_ports.spec.ts`'s
 * identically-named helper. */
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

type TestStar = StarTransportAdapter & Pumpable;

// ---------------------------------------------------------------------------
// `fakeRoomRendezvous`: an in-process stand-in for the room-code Worker
// (`infra/src/room_durable_object.ts`), built only from this issue's own
// `RoomSignalingEvent`/`RoomSignalingHandle` port shapes -- no WebSocket, no
// Durable Object, the same trade `fakeStar` makes for WebRTC. It reproduces
// the two behaviors this spec actually exercises: a host gets a code and
// learns of each guest that "joins" it (`guestId`s in join order); a
// guest's `send` is always addressed to the host (single recipient) and a
// host's `send` must name which guest.
// ---------------------------------------------------------------------------

interface FakeRoom {
  readonly code: string;
  readonly hostQueue: RoomSignalingEvent[];
  readonly guestQueues: Map<string, RoomSignalingEvent[]>;
}

interface FakeRoomRendezvous {
  openHost(): RoomSignalingHandle;
  openGuest(code: string): RoomSignalingHandle;
}

function fakeRoomRendezvous(): FakeRoomRendezvous {
  const rooms = new Map<string, FakeRoom>();
  let roomCounter = 0;
  let guestCounter = 0;

  function openHost(): RoomSignalingHandle {
    roomCounter += 1;
    // Digits only, padded to the composer's fixed 6-character width -- the
    // real alphabet excludes I/L/O/U (`room_signaling.ts`'s own header), and
    // the guest composer below (`typeCode`) can only ever produce a
    // 6-character, alphabet-valid code, so the fake code generated here has
    // to be one too or the round trip through the composer would silently
    // diverge from it.
    const code = String(roomCounter).padStart(6, "0");
    const room: FakeRoom = { code, hostQueue: [{ kind: "created", code }], guestQueues: new Map() };
    rooms.set(code, room);
    let closed = false;
    return {
      poll: () => (closed ? [] : room.hostQueue.splice(0, room.hostQueue.length)),
      send: (effect) => {
        if (closed || effect.to === undefined) {
          return;
        }
        room.guestQueues.get(effect.to)?.push({ kind: "signal", signal: effect.signal });
      },
      close: () => {
        closed = true;
        rooms.delete(code);
      },
    };
  }

  function openGuest(code: string): RoomSignalingHandle {
    const room = rooms.get(code);
    if (room === undefined) {
      // Unknown/expired code: a real handshake against a room that does not
      // exist collapses to a generic failure -- see `room_signaling.ts`'s
      // own doc on why a browser WebSocket cannot report more than that.
      let events: RoomSignalingEvent[] = [{ kind: "failed", reason: "handshake_failed" }];
      return {
        poll: () => {
          const drained = events;
          events = [];
          return drained;
        },
        send: () => {},
        close: () => {},
      };
    }
    guestCounter += 1;
    const guestId = `guest-do-${guestCounter}`;
    const queue: RoomSignalingEvent[] = [{ kind: "joined", code }];
    room.guestQueues.set(guestId, queue);
    room.hostQueue.push({ kind: "guest_joined", guest_id: guestId });
    let closed = false;
    return {
      poll: () => (closed ? [] : queue.splice(0, queue.length)),
      send: (effect) => {
        if (closed) {
          return;
        }
        room.hostQueue.push({ kind: "signal", signal: effect.signal, guest_id: guestId });
      },
      close: () => {
        closed = true;
        room.guestQueues.delete(guestId);
        room.hostQueue.push({ kind: "guest_left", guest_id: guestId });
      },
    };
  }

  return { openHost, openGuest };
}

function typeCode(lobby: DispatchableLobby, code: string): void {
  for (const ch of code) {
    lobby.dispatch({ kind: "room_key", key: ch });
  }
}

interface RoomCodeHandshake {
  readonly host: DispatchableLobby;
  readonly guest: DispatchableLobby;
  readonly stars: readonly TestStar[];
  readonly code: string;
}

/** Drives a real host and a real, independently-constructed guest through
 * the room-code path (never `copy`/`paste`) to a real, ready, two-human
 * 1v1 session -- the room-code counterpart of `online_ports.spec.ts`'s
 * `runTwoPeerHandshake`. */
function runRoomCodeHandshake(): RoomCodeHandshake {
  const starRendezvous = fakeStarRendezvous();
  const stars: TestStar[] = [];
  const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
    const star = fakeStar({ role, rendezvous: starRendezvous, peer_id: peerId });
    if (!star.initialize().ok) {
      return undefined;
    }
    stars.push(star);
    return star;
  };
  const roomRendezvous = fakeRoomRendezvous();
  const hostPorts = newTestOnlinePorts(starFactory, roomRendezvous);
  const guestPorts = newTestOnlinePorts(starFactory, roomRendezvous);
  const host = newDispatchableLobby(hostPorts, () => {});
  const guest = newDispatchableLobby(guestPorts, () => {});

  host.dispatch({ kind: "room_pick", role: "host" });
  pump([host], stars);
  const code = host.state.model.room_code;
  if (code === undefined) {
    throw new Error("the host must have a room code after room_pick(host) + a pump cycle");
  }
  host.dispatch({ kind: "mode", mode: "1v1" });

  guest.dispatch({ kind: "room_pick", role: "guest" });
  typeCode(guest, code);
  guest.dispatch({ kind: "room_submit" });
  // Enough cycles for: guest_joined -> auto-invite -> offer -> auto-send ->
  // guest auto-accepts -> answer -> auto-send -> host auto-accepts ->
  // WebRTC connects (fakeStar) -> admission.
  pump([host, guest], stars, 60);

  host.dispatch({ kind: "lock" });
  pump([host, guest], stars, 40);
  host.dispatch({ kind: "ready", ready: true });
  guest.dispatch({ kind: "ready", ready: true });
  pump([host, guest], stars, 40);

  return { host, guest, stars, code };
}

describe("room_code_lobby: the room-code path (#552), through production OnlinePorts wiring", () => {
  it("connects a real host and guest to a ready 1v1 session with no copy/paste command", () => {
    const { host, guest, code } = runRoomCodeHandshake();

    expect(code).toMatch(/^\d{6}$/);
    expect(host.state.model.room_code).toBe(code);
    expect(host.state.model.role).toBe("host");
    expect(guest.state.model.role).toBe("guest");
    // Room-code mode auto-sends: the manual "outgoing" blob a `copy` click
    // would otherwise hand to a clipboard is never left sitting unsent.
    expect(host.state.model.outgoing).toBeUndefined();
    expect(guest.state.model.outgoing).toBeUndefined();

    expect(host.state.model.coordinator?.role).toBe("host");
    expect(guest.state.model.coordinator?.role).toBe("guest");
    expect(host.state.model.coordinator?.peers.length).toBe(2);
    expect(host.state.model.coordinator?.phase).toBe("ready");
    expect(guest.state.model.coordinator?.phase).toBe("ready");
  });

  it("reaches a synchronized start once the host begins the countdown", () => {
    const { host, guest, stars } = runRoomCodeHandshake();

    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);
    expect(host.state.model.started).toBe(true);
    expect(guest.state.model.started).toBe(true);
  });
});

describe("room_code_lobby: failure states", () => {
  it("surfaces an unknown/expired code as a readable lobby error, not a hang", () => {
    const roomRendezvous = fakeRoomRendezvous();
    const starRendezvous = fakeStarRendezvous();
    const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
      const star = fakeStar({ role, rendezvous: starRendezvous, peer_id: peerId });
      return star.initialize().ok ? star : undefined;
    };
    const guestPorts = newTestOnlinePorts(starFactory, roomRendezvous);
    const guest = newDispatchableLobby(guestPorts, () => {});

    guest.dispatch({ kind: "room_pick", role: "guest" });
    typeCode(guest, "ZZZZZZ"); // never hosted -- no matching FakeRoom
    guest.dispatch({ kind: "room_submit" });
    pump([guest], [], 5);

    expect(guest.state.model.role).toBeUndefined();
    // `room_error`, not `error`: `error` is cleared by the very next `tick`
    // command `pump()`'s `update(dt)` dispatches every frame, so only the
    // dedicated, persistent field still holds the failure by the time this
    // assertion runs (`room_error`'s own doc on `LobbyModel`).
    expect(guest.state.model.room_error).toBe("handshake_failed");
  });

  it("leaves the manual signaling path fully usable alongside the room-code one", () => {
    const starRendezvous = fakeStarRendezvous();
    const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
      const star = fakeStar({ role, rendezvous: starRendezvous, peer_id: peerId });
      return star.initialize().ok ? star : undefined;
    };
    // No roomSignaling port at all -- the same shape a static host with no
    // Worker behind it produces, and the same shape a spec that never
    // exercises the room-code path already omits (`online_ports.spec.ts`).
    const ports = newTestOnlinePorts(starFactory, undefined);
    const lobby = newDispatchableLobby(ports, () => {});

    lobby.dispatch({ kind: "role", role: "host" });
    expect(lobby.state.model.role).toBe("host");
    expect(lobby.state.model.room_active).toBe(false);
  });
});
