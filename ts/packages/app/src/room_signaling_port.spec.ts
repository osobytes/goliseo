// Tier 3-ish coverage for the impure `room_signaling_port.ts` -- driven
// against a minimal fake `WebSocket` (`FakeSocket` below) rather than a
// real connection, so this runs headlessly with no network and no `window`
// (`connect()`'s own header explains why an injected `newSocket` never
// touches `window.location`). `room_signaling.spec.ts` (`@gc/online`)
// already covers the pure parsing/encoding this port calls into; this file
// is about the socket lifecycle glue itself: which events a message/close
// sequence produces, and what `send`/`close` do to the socket.

import { describe, expect, it } from "vitest";
import { connectRoomGuest, connectRoomHost, type MinimalWebSocket } from "./room_signaling_port.ts";

type Listener = (event: { readonly data: unknown }) => void;
type PlainListener = () => void;

class FakeSocket implements MinimalWebSocket {
  readyState = 0; // CONNECTING
  readonly sent: string[] = [];
  closeCalled = 0;
  private readonly messageListeners: Listener[] = [];
  private readonly closeListeners: PlainListener[] = [];
  private readonly errorListeners: PlainListener[] = [];

  addEventListener(type: "message" | "close" | "error", listener: Listener | PlainListener): void {
    if (type === "message") {
      this.messageListeners.push(listener);
    } else if (type === "close") {
      this.closeListeners.push(listener as PlainListener);
    } else {
      this.errorListeners.push(listener as PlainListener);
    }
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.closeCalled += 1;
    this.open(); // no-op if already past CONNECTING
    this.readyState = 3; // CLOSED
    this.emitClose();
  }

  // --- test-only helpers, not part of MinimalWebSocket -------------------

  open(): void {
    this.readyState = 1; // OPEN
  }

  emitMessage(data: unknown): void {
    for (const listener of this.messageListeners) {
      listener({ data });
    }
  }

  emitClose(): void {
    for (const listener of this.closeListeners) {
      listener();
    }
  }

  emitError(): void {
    for (const listener of this.errorListeners) {
      listener();
    }
  }
}

function hostHandle(): {
  readonly socket: FakeSocket;
  readonly handle: ReturnType<typeof connectRoomHost>;
} {
  const socket = new FakeSocket();
  const handle = connectRoomHost(() => socket);
  return { socket, handle };
}

function guestHandle(): {
  readonly socket: FakeSocket;
  readonly handle: ReturnType<typeof connectRoomGuest>;
} {
  const socket = new FakeSocket();
  const handle = connectRoomGuest("A3F9K2", () => socket);
  return { socket, handle };
}

describe("room_signaling_port: host", () => {
  it("reports created once the Worker confirms the room", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage('{"type":"created","code":"A3F9K2"}');
    expect(handle.poll()).toEqual([{ kind: "created", code: "A3F9K2" }]);
    // Drained -- a second poll before any new frame sees nothing.
    expect(handle.poll()).toEqual([]);
  });

  it("reports a guest joining with its Durable-Object-issued id", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage('{"type":"guest_joined","guestId":"g-1"}');
    expect(handle.poll()).toEqual([{ kind: "guest_joined", guest_id: "g-1" }]);
  });

  it("addresses a signal at a specific guest id", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    handle.send({ to: "g-1", signal: "offer-blob" });
    expect(socket.sent).toEqual([JSON.stringify({ to: "g-1", body: "offer-blob" })]);
  });

  it("does not send when no target guest is given (defensive)", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    handle.send({ signal: "offer-blob" });
    expect(socket.sent).toEqual([]);
  });

  it("does not send before the socket is open", () => {
    const { socket, handle } = hostHandle();
    handle.send({ to: "g-1", signal: "offer-blob" });
    expect(socket.sent).toEqual([]);
  });

  it("reports a guest's relayed signal by its guest id", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage(JSON.stringify({ type: "signal", from: "g-1", body: "answer-blob" }));
    expect(handle.poll()).toEqual([{ kind: "signal", signal: "answer-blob", guest_id: "g-1" }]);
  });

  it("closes the socket and stops reporting further events", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    handle.close();
    expect(socket.closeCalled).toBe(1);
    socket.emitMessage('{"type":"guest_joined","guestId":"g-1"}');
    expect(handle.poll()).toEqual([]);
  });
});

describe("room_signaling_port: guest", () => {
  it("connects to /signal/join with the code as a query parameter", () => {
    const socket = new FakeSocket();
    let seenPath: string | undefined;
    connectRoomGuest("A3F9K2", (path) => {
      seenPath = path;
      return socket;
    });
    expect(seenPath).toBe("/signal/join?code=A3F9K2");
  });

  it("reports joined once the Worker confirms", () => {
    const { socket, handle } = guestHandle();
    socket.open();
    socket.emitMessage('{"type":"joined","code":"A3F9K2"}');
    expect(handle.poll()).toEqual([{ kind: "joined", code: "A3F9K2" }]);
  });

  it("sends its raw signal text with no envelope", () => {
    const { socket, handle } = guestHandle();
    socket.open();
    handle.send({ signal: "answer-blob" });
    expect(socket.sent).toEqual(["answer-blob"]);
  });

  it("reports the host's relayed signal with no guest id", () => {
    const { socket, handle } = guestHandle();
    socket.open();
    socket.emitMessage(JSON.stringify({ type: "signal", from: "host", body: "offer-blob" }));
    expect(handle.poll()).toEqual([{ kind: "signal", signal: "offer-blob" }]);
  });
});

describe("room_signaling_port: failure states", () => {
  it("reports a handshake failure when the socket closes before readiness", () => {
    const { socket, handle } = hostHandle();
    socket.emitClose();
    expect(handle.poll()).toEqual([{ kind: "failed", reason: "handshake_failed" }]);
  });

  it("reports a dropped connection when the socket closes after readiness", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage('{"type":"created","code":"A3F9K2"}');
    handle.poll(); // drain the created event
    socket.emitClose();
    expect(handle.poll()).toEqual([{ kind: "dropped", reason: "connection_lost" }]);
  });

  // Round-2 council review, blocking finding 3: every branch below used to
  // report `{kind:"failed"}` without ever closing the socket -- against
  // the rate-limited signaling Worker, a retry after ANY of these piled a
  // fresh connection on top of the still-open old one, with only the
  // room's 10-minute TTL alarm to clean it up eventually.
  it("reports a malformed frame without throwing, and closes the socket", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage("not json");
    expect(handle.poll()).toEqual([{ kind: "failed", reason: "malformed_frame" }]);
    expect(socket.closeCalled).toBe(1);
    // `FakeSocket.close()` synchronously re-fires its own `close` event;
    // this must not ALSO report a redundant `dropped`/`handshake_failed`
    // event for the same failure.
    expect(handle.poll()).toEqual([]);
  });

  it("reports a binary frame as malformed, and closes the socket", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage(new ArrayBuffer(4));
    expect(handle.poll()).toEqual([{ kind: "failed", reason: "malformed_frame" }]);
    expect(socket.closeCalled).toBe(1);
  });

  it("reports the Worker's own error code from a protocol-level error frame, and closes the socket", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage('{"type":"error","error":"room_not_open"}');
    expect(handle.poll()).toEqual([{ kind: "failed", reason: "room_not_open" }]);
    expect(socket.closeCalled).toBe(1);
    expect(handle.poll()).toEqual([]);
  });

  // #599: an admission-failure reason (room_not_found/room_full/
  // room_expired/room_closed/host_already_claimed/already_joined) now
  // arrives the exact same way as any other in-band error -- the Durable
  // Object completes the upgrade instead of rejecting the HTTP request, so
  // this port has no separate code path for it.
  it("reports each in-band admission-failure reason and closes the socket", () => {
    for (const reason of [
      "room_not_found",
      "room_full",
      "room_expired",
      "room_closed",
      "host_already_claimed",
      "already_joined",
    ]) {
      const { socket, handle } = hostHandle();
      socket.open();
      socket.emitMessage(JSON.stringify({ type: "error", error: reason }));
      expect(handle.poll()).toEqual([{ kind: "failed", reason }]);
      expect(socket.closeCalled).toBe(1);
    }
  });

  it("reports a host_left event and closes the socket, without a redundant dropped event", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage('{"type":"created","code":"A3F9K2"}');
    handle.poll(); // drain the created event
    socket.emitMessage('{"type":"host_left"}');
    expect(handle.poll()).toEqual([{ kind: "host_left" }]);
    expect(socket.closeCalled).toBe(1);
    // `FakeSocket.close()` synchronously re-fires its own `close` event;
    // this must not ALSO report a redundant `dropped` event for the same
    // departure -- the same discipline `fail()` already follows.
    expect(handle.poll()).toEqual([]);
  });

  it("does not send after a protocol-level error has closed the socket", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    socket.emitMessage('{"type":"error","error":"room_not_open"}');
    handle.poll();
    handle.send({ to: "g-1", signal: "offer-blob" });
    // `FakeSocket.close()` sets `readyState` to CLOSED, and `send()` only
    // ever writes while `readyState === OPEN`.
    expect(socket.sent).toEqual([]);
  });

  it("does not double-report a close event", () => {
    const { socket, handle } = hostHandle();
    socket.open();
    handle.close();
    socket.emitClose(); // a real socket would not call back twice, but never trust that
    expect(handle.poll()).toEqual([]);
  });

  it("reports a synchronous constructor throw as a handshake failure", () => {
    const handle = connectRoomHost(() => {
      throw new Error("blocked by a browser security policy");
    });
    expect(handle.poll()).toEqual([{ kind: "failed", reason: "handshake_failed" }]);
  });
});
