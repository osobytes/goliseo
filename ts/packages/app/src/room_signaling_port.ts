// Impure WebSocket port for the room-code signaling Worker
// (`infra/src/index.ts`'s `/signal/host`/`/signal/join` routes;
// `infra/src/room_durable_object.ts` is the wire contract). Pure parsing,
// encoding, and failure classification live in `@gc/online`'s
// `room_signaling.ts`; this file owns the one thing that package must not
// (AGENTS.md §9): a real `WebSocket`. It is injected into `@gc/screens`'s
// `OnlineLobby` through its own structurally-typed `roomSignaling` option
// (`online_lobby.ts`'s header), the same pattern `starFactory`/`newLink`
// already establish for the star transport (`browser_main.ts`'s wiring).
//
// Same-origin by default: `wss://<page host>/signal/...` -- the deployed
// Worker only ever answers on the SAME origin the page itself is served
// from (`infra/src/index.ts`'s own header: only `/api/*` and `/signal/*`
// reach the Worker; everything else is static assets). A static host with
// no such Worker simply fails every connection attempt, which surfaces as
// an ordinary `"handshake_failed"` event -- the lobby screen shows that as
// a readable failure and the player can still choose the manual-signaling
// path (`docs`/this issue's acceptance criteria: "Manual signaling fallback
// still works").
//
// `newSocket` is an injected `WebSocket` constructor (defaulting to the
// global one) purely for testability: this port's own spec drives it
// against a minimal fake rather than opening a real connection.

import {
  ROOM_SIGNAL_HOST_PATH,
  classifyClose,
  encodeHostSignal,
  parseServerFrame,
  roomSignalJoinPath,
  type RoomServerFrame,
} from "@gc/online";
import type { RoomSignalingEvent, RoomSignalingHandle } from "@gc/screens";

export type { RoomSignalingEvent, RoomSignalingHandle } from "@gc/screens";

/** The minimal subset of the DOM `WebSocket` this port needs -- narrow on
 * purpose so a test can supply a fake without implementing the full API. */
export interface MinimalWebSocket {
  readonly readyState: number;
  send(data: string): void;
  close(): void;
  addEventListener(type: "message", listener: (event: { readonly data: unknown }) => void): void;
  addEventListener(type: "close" | "error", listener: () => void): void;
}

const OPEN_STATE = 1;

function wsUrl(path: string): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}${path}`;
}

// `window.location` is only ever touched here, inside the DEFAULT factory --
// `connect()` itself hands `newSocket` the bare `/signal/...` path, never a
// resolved URL, so an injected fake (this file's own spec) never needs a
// `window` global at all. A production caller that wants the real
// same-origin translation gets it for free by not overriding `newSocket`.
function defaultSocketFactory(path: string): MinimalWebSocket {
  return new WebSocket(wsUrl(path));
}

function connect(
  path: string,
  role: "host" | "guest",
  newSocket: (path: string) => MinimalWebSocket,
): RoomSignalingHandle {
  const events: RoomSignalingEvent[] = [];
  let ready = false;
  let closed = false;
  let socket: MinimalWebSocket | undefined;

  try {
    socket = newSocket(path);
  } catch {
    // Constructing a WebSocket can throw synchronously (an invalid URL, a
    // browser security policy) -- treated identically to an async
    // handshake failure, since a caller cannot tell the two apart either
    // way (see `room_signaling.ts`'s own doc on this limitation).
    events.push({ kind: "failed", reason: "handshake_failed" });
    closed = true;
  }

  if (socket !== undefined) {
    const live = socket;
    // A `"failed"` event always means this connection is over -- every
    // caller of this (the binary-frame branch, the unparseable-frame
    // branch, and the Worker's own in-band `{type:"error"}` frame below)
    // used to leave the socket open, so a retry (a fresh `connect()` call)
    // piled a new socket on top of the old one instead of replacing it.
    // Against the rate-limited signaling Worker, that is an orphaned
    // connection per retry with only the room's 10-minute TTL alarm to
    // clean it up (round-2 council review, blocking finding 3). `closed`
    // is set BEFORE `live.close()` so the `"close"` listener below sees it
    // already true and does not ALSO report a redundant `"dropped"`/
    // `"handshake_failed"` event for the same failure.
    function fail(reason: string): void {
      events.push({ kind: "failed", reason });
      closed = true;
      live.close();
    }
    live.addEventListener("message", (event: { readonly data: unknown }) => {
      if (closed) {
        return;
      }
      if (typeof event.data !== "string") {
        // The Worker only ever sends text frames (`room_durable_object.ts`'s
        // own contract) -- a binary frame here is a protocol violation, not
        // something to silently ignore.
        fail("malformed_frame");
        return;
      }
      const parsed = parseServerFrame(event.data);
      if (!parsed.ok) {
        fail("malformed_frame");
        return;
      }
      const frame: RoomServerFrame = parsed.value;
      if (frame.type === "created" || frame.type === "joined") {
        ready = true;
        events.push({ kind: frame.type, code: frame.code });
      } else if (frame.type === "guest_joined" || frame.type === "guest_left") {
        events.push({ kind: frame.type, guest_id: frame.guestId });
      } else if (frame.type === "signal") {
        events.push({
          kind: "signal",
          signal: frame.body,
          // A guest has exactly one possible sender (the host); its own
          // signal events carry no guest id at all -- `lobby_model.ts`'s
          // `room_peer_signal` command treats an absent `guest_id` that way.
          ...(role === "host" ? { guest_id: frame.from } : {}),
        });
      } else {
        // `frame.type === "error"`: the Worker's own error code
        // (`room_durable_object.ts`'s `{type:"error", error}` frame, e.g.
        // `"room_not_open"`, `"message_too_large"`) is more informative
        // than a generic bucket, so it travels straight through as the
        // reason -- `"protocol_error"` (`room_signaling.ts`'s own type) is
        // only the fallback for the degenerate case of an empty code.
        //
        // Closed here too, deliberately, even though the Durable Object
        // itself leaves the socket open after sending one (`webSocketMessage`
        // just `return`s -- no `ws.close()`): nothing on THIS side has any
        // recovery path for a `missing_target`/`unknown_target`/
        // `message_too_large`/... reply, so continuing to poll a socket in
        // a state this client cannot progress past would just reproduce
        // round-2 council review's blocking finding 2 (a lobby wedged with
        // no way forward but LEAVE) one layer down. Treating it as
        // connection-ending here, and surfacing a clean `"failed"` event, is
        // what actually gives the player a working retry.
        fail(frame.error !== "" ? frame.error : "protocol_error");
      }
    });
    live.addEventListener("close", () => {
      if (closed) {
        return;
      }
      closed = true;
      const failure = classifyClose(ready);
      events.push({ kind: ready ? "dropped" : "failed", reason: failure.reason });
    });
    live.addEventListener("error", () => {
      // A WebSocket always follows an 'error' with a 'close' -- nothing
      // more to report here than the 'close' handler already will.
    });
  }

  return {
    poll(): readonly RoomSignalingEvent[] {
      return events.splice(0, events.length);
    },
    send(effect: { readonly to?: string; readonly signal: string }): void {
      if (socket === undefined || socket.readyState !== OPEN_STATE) {
        return;
      }
      if (role === "host") {
        if (effect.to === undefined) {
          return; // Defensive: a host always addresses a specific guest.
        }
        socket.send(encodeHostSignal(effect.to, effect.signal));
      } else {
        // A guest has exactly one possible recipient and sends its raw
        // signal text with no envelope -- see `room_signaling.ts`'s header.
        socket.send(effect.signal);
      }
    },
    close(): void {
      closed = true;
      socket?.close();
    },
  };
}

export function connectRoomHost(
  newSocket: (path: string) => MinimalWebSocket = defaultSocketFactory,
): RoomSignalingHandle {
  return connect(ROOM_SIGNAL_HOST_PATH, "host", newSocket);
}

export function connectRoomGuest(
  code: string,
  newSocket: (path: string) => MinimalWebSocket = defaultSocketFactory,
): RoomSignalingHandle {
  return connect(roomSignalJoinPath(code), "guest", newSocket);
}
