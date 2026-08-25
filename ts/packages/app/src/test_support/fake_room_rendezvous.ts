// `fakeRoomRendezvous`: an in-process stand-in for the room-code Worker
// (`infra/src/room_durable_object.ts`), built only from this issue's own
// `RoomSignalingEvent`/`RoomSignalingHandle` port shapes -- no WebSocket, no
// Durable Object, the same trade `fakeStar` makes for WebRTC. It reproduces
// the two behaviors specs actually exercise: a host gets a code and learns
// of each guest that "joins" it (`guestId`s in join order); a guest's
// `send` is always addressed to the host (single recipient) and a host's
// `send` must name which guest.
//
// Extracted from `room_code_lobby.spec.ts` (#552) so `multiplayer_room_flow.spec.ts`
// (#610, the front-door-to-countdown host journey) can drive a real guest
// through the same real relay wire functions without a second copy.
//
// #601, round-2 council review: `send` on both sides routes through the
// REAL `@gc/online` `room_signaling.ts` wire functions
// (`encodeHostSignal`/`parseServerFrame`), not a hand-rolled
// `RoomSignalingEvent` passthrough -- a fake that only ever forwarded
// `effect.signal` directly could never have exercised the `v:1` slot
// envelope at all. What this does NOT reproduce is
// `room_signaling_port.ts`'s own WebSocket glue (message listeners,
// `readyState`, `close()` semantics) -- that stays `room_signaling_port.spec.ts`'s
// job, against a `FakeSocket`.

import { encodeHostSignal, parseServerFrame } from "@gc/online";
import type { RoomSignalingEvent, RoomSignalingHandle } from "@gc/screens";

interface FakeRoom {
  readonly code: string;
  readonly hostQueue: RoomSignalingEvent[];
  readonly guestQueues: Map<string, RoomSignalingEvent[]>;
}

export interface FakeRoomRendezvous {
  openHost(): RoomSignalingHandle;
  openGuest(code: string): RoomSignalingHandle;
  /** Total `close()` calls across every host handle this rendezvous has
   * ever produced -- proves a caller closes a stale `roomLink` before
   * replacing it with a fresh one on a retry, rather than leaking it. */
  hostCloseCount(): number;
}

export function fakeRoomRendezvous(): FakeRoomRendezvous {
  const rooms = new Map<string, FakeRoom>();
  let roomCounter = 0;
  let guestCounter = 0;
  let hostCloses = 0;

  function openHost(): RoomSignalingHandle {
    roomCounter += 1;
    // Digits only, padded to the composer's fixed 6-character width -- the
    // real alphabet excludes I/L/O/U (`room_signaling.ts`'s own header), and
    // a guest composer typing this code can only ever produce a
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
        // Host -> DO -> guest, through the real wire functions (this
        // module's own doc). `encodeHostSignal` wraps `effect.slot` in the
        // `v:1` envelope when the host provided one; the DO forwards
        // `body` exactly as sent (`room_durable_object.ts`'s own doc,
        // "NOT re-stringified") into a fresh `{type:"signal", from:"host",
        // body}` frame, which `parseServerFrame` -- the guest's own parser
        // -- decodes back into the signal and, when present, the slot.
        const hostToDo = JSON.parse(encodeHostSignal(effect.to, effect.signal, effect.slot)) as {
          readonly body: unknown;
        };
        const doToGuest = JSON.stringify({ type: "signal", from: "host", body: hostToDo.body });
        const parsed = parseServerFrame(doToGuest);
        if (parsed.ok && parsed.value.type === "signal") {
          room.guestQueues.get(effect.to)?.push({
            kind: "signal",
            signal: parsed.value.body,
            ...(parsed.value.slot !== undefined ? { slot: parsed.value.slot } : {}),
          });
        }
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
        // A guest's own signal is never enveloped (`room_signaling.ts`'s
        // own header, "guest -> host stays raw") -- the DO forwards it as
        // the raw text it received, `body` a plain string, still through
        // the real parser for consistency with the host->guest direction
        // above.
        const doToHost = JSON.stringify({ type: "signal", from: guestId, body: effect.signal });
        const parsed = parseServerFrame(doToHost);
        if (parsed.ok && parsed.value.type === "signal") {
          room.hostQueue.push({ kind: "signal", signal: parsed.value.body, guest_id: guestId });
        }
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
