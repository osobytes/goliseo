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
import {
  LOBBY_ROOM_FAILURE_TEXT,
  type LobbyScreenState,
  type RoomSignalingEvent,
  type RoomSignalingHandle,
} from "@gc/screens";
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
  /** Total `close()` calls across every host handle this rendezvous has
   * ever produced -- round-2 council review, blocking finding 3: proves
   * `online_lobby.ts` closes a stale `roomLink` before replacing it with a
   * fresh one on a retry, rather than leaking it. */
  hostCloseCount(): number;
}

function fakeRoomRendezvous(): FakeRoomRendezvous {
  const rooms = new Map<string, FakeRoom>();
  let roomCounter = 0;
  let guestCounter = 0;
  let hostCloses = 0;

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
        if (!closed) {
          hostCloses += 1;
        }
        closed = true;
        rooms.delete(code);
      },
    };
  }

  function openGuest(code: string): RoomSignalingHandle {
    const room = rooms.get(code);
    if (room === undefined) {
      // Unknown/expired code: as of #599 this completes the WebSocket
      // upgrade and reports the reason in-band instead of failing the
      // handshake itself -- `room_durable_object.ts`'s own doc, "Admission
      // failures". A code no host has ever claimed reads as `room_not_found`
      // (`room_state.ts`'s `joinGuest`, same reasoning this fake's own
      // `rooms` map already encodes: an unclaimed code and a nonexistent one
      // are indistinguishable here).
      let events: RoomSignalingEvent[] = [{ kind: "failed", reason: "room_not_found" }];
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

  return { openHost, openGuest, hostCloseCount: () => hostCloses };
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
    expect(guest.state.model.room_error).toBe(LOBBY_ROOM_FAILURE_TEXT["room_not_found"]);
    // A failed room-code attempt must not leave `room_active` stuck `true`
    // -- that would hide the manual copy/paste controls forever
    // (`lobby.ts`'s layout gates them on `!room_active`), making the
    // "manual signaling still works" fallback unreachable after exactly
    // the failure this test drives. Round-2 council review, blocking
    // finding 1.
    expect(guest.state.model.room_active).toBe(false);

    // The manual path is genuinely usable from here: picking a manual
    // role now must not be refused, and must reach a normal, working
    // manual lobby (not one wedged mid-connection).
    guest.dispatch({ kind: "role", role: "guest" });
    expect(guest.state.model.role).toBe("guest");
    expect(guest.state.model.error).toBeUndefined();
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

  // Round-2 council review, blocking finding 2: a manual role pick (or a
  // second room-code attempt) used to stay reachable during the async
  // window after `room_pick`/`room_submit` -- `room_active` already `true`,
  // `room_status: "connecting"`, but no `coordinator` yet. Activating one
  // then wedged the lobby: the LATE `room_created`/`room_joined` would
  // no-op against the already-chosen role but still leave `room_active`
  // stuck `true` forever, with no way to reach either signaling path again
  // except LEAVE.
  it("refuses a manual role pick raced against an in-flight room-code connection", () => {
    const roomRendezvous = fakeRoomRendezvous();
    const ports = newTestOnlinePorts(() => undefined, roomRendezvous);
    const host = newDispatchableLobby(ports, () => {});

    // `room_open_host` is processed synchronously by `dispatch()` -- the
    // fake's `created` event is queued but not yet polled, so this is
    // exactly the async window: `room_active` is `true`, `coordinator` is
    // still `undefined`.
    host.dispatch({ kind: "room_pick", role: "host" });
    expect(host.state.model.room_active).toBe(true);
    expect(host.state.model.coordinator).toBeUndefined();

    // The race: a manual role pick during that window is refused, not
    // silently accepted and then abandoned.
    host.dispatch({ kind: "role", role: "host" });
    expect(host.state.model.coordinator).toBeUndefined();
    expect(host.state.model.error).toBeDefined();

    // A second room-code attempt during the SAME window is refused too.
    host.dispatch({ kind: "room_pick", role: "guest" });
    expect(host.state.model.room_entry).toBeUndefined();
    expect(host.state.model.error).toBeDefined();

    // The original room-code attempt itself is unaffected by either
    // refused race and still completes normally.
    pump([host], []);
    expect(host.state.model.role).toBe("host");
    expect(host.state.model.coordinator?.role).toBe("host");
    expect(host.state.model.room_code).toBeDefined();
  });

  // Round-2 council review, blocking finding 3: a room-code retry used to
  // leave the PREVIOUS attempt's socket open (`room_signaling_port.ts`
  // never closed it on a protocol-level failure, and `online_lobby.ts`
  // overwrote `this.roomLink` without closing the old one first) --
  // against the rate-limited signaling Worker, every retry orphaned
  // another live connection.
  it("closes the previous room-code connection before a retry opens a new one", () => {
    const roomRendezvous = fakeRoomRendezvous();
    const ports = newTestOnlinePorts(() => undefined, roomRendezvous);
    const host = newDispatchableLobby(ports, () => {});

    // Never pumped: the fake's `created` event for THIS attempt sits
    // unpolled, so `chooseRole` never runs and `coordinator` stays
    // `undefined` -- exactly a connection that fails before ever
    // succeeding (a Worker outage, a network drop mid-handshake), not a
    // connection that succeeded and later died. `roomPick`'s own
    // "already chosen" guard would otherwise block a legitimate retry, so
    // this is the scenario where one actually has to work.
    host.dispatch({ kind: "room_pick", role: "host" });
    expect(host.state.model.role).toBeUndefined();
    expect(roomRendezvous.hostCloseCount()).toBe(0);

    // A failure -- exactly what `room_signaling_port.ts`'s own `fail()`
    // helper now reports after closing its own socket (blocking finding
    // 3's other half) -- resets `room_active` (blocking finding 1)
    // without the pure model ever pushing a `room_close` effect itself,
    // so `online_lobby.ts`'s `roomLink` is a stale, not-yet-closed
    // reference until the next `room_open_*` effect closes it.
    host.dispatch({ kind: "room_failed", reason: "handshake_failed" });
    expect(host.state.model.room_active).toBe(false);
    expect(host.state.model.role).toBeUndefined();

    // Retrying as host opens a genuinely new connection, and closes the
    // stale one first rather than leaking it.
    host.dispatch({ kind: "room_pick", role: "host" });
    expect(roomRendezvous.hostCloseCount()).toBe(1);
    pump([host], []);
    expect(host.state.model.role).toBe("host");
    expect(host.state.model.room_code).toBeDefined();
  });
});
