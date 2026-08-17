import { describe, expect, it } from "vitest";

import {
  MAX_GUESTS,
  ROOM_TTL_MS,
  claimHost,
  closeRoom,
  isExpired,
  joinGuest,
  newRoom,
  removeConnection,
  routeSignal,
} from "./room_state.ts";

const T0 = 1_000_000;

describe("newRoom", () => {
  it("starts unclaimed and waiting for a host", () => {
    const room = newRoom("ABC123", T0);
    expect(room.phase).toBe("waiting_for_host");
    expect(room.hostId).toBeNull();
    expect(room.guestIds).toEqual([]);
  });
});

describe("isExpired", () => {
  it("is false right at creation and false just under the TTL", () => {
    const room = newRoom("ABC123", T0);
    expect(isExpired(room, T0)).toBe(false);
    expect(isExpired(room, T0 + ROOM_TTL_MS)).toBe(false);
  });

  it("is true once the TTL has elapsed", () => {
    const room = newRoom("ABC123", T0);
    expect(isExpired(room, T0 + ROOM_TTL_MS + 1)).toBe(true);
  });
});

describe("claimHost", () => {
  it("moves an unclaimed room to open", () => {
    const room = newRoom("ABC123", T0);
    const result = claimHost(room, "host-1", T0);
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.phase).toBe("open");
      expect(result.value.hostId).toBe("host-1");
    }
  });

  it("rejects a second claim", () => {
    const room = newRoom("ABC123", T0);
    const claimed = claimHost(room, "host-1", T0);
    expect(claimed.ok).toBe(true);
    if (!claimed.ok) return;
    const second = claimHost(claimed.value, "host-2", T0);
    expect(second).toEqual({ ok: false, error: "host_already_claimed" });
  });

  it("rejects an expired room", () => {
    const room = newRoom("ABC123", T0);
    const result = claimHost(room, "host-1", T0 + ROOM_TTL_MS + 1);
    expect(result).toEqual({ ok: false, error: "room_expired" });
  });

  it("rejects a closed room", () => {
    const room = closeRoom(newRoom("ABC123", T0));
    const result = claimHost(room, "host-1", T0);
    expect(result).toEqual({ ok: false, error: "room_closed" });
  });
});

function openRoom(nowMs: number, hostId = "host-1") {
  const claimed = claimHost(newRoom("ABC123", nowMs), hostId, nowMs);
  if (!claimed.ok) throw new Error("test setup: claimHost failed");
  return claimed.value;
}

describe("joinGuest", () => {
  it("rejects joining before a host has claimed the room", () => {
    const room = newRoom("ABC123", T0);
    const result = joinGuest(room, "guest-1", T0);
    expect(result).toEqual({ ok: false, error: "room_not_open" });
  });

  it("adds a guest to an open room", () => {
    const room = openRoom(T0);
    const result = joinGuest(room, "guest-1", T0);
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.guestIds).toEqual(["guest-1"]);
    }
  });

  it("rejects the same connection id joining twice", () => {
    const room = openRoom(T0);
    const joined = joinGuest(room, "guest-1", T0);
    if (!joined.ok) throw new Error("test setup failed");
    const second = joinGuest(joined.value, "guest-1", T0);
    expect(second).toEqual({ ok: false, error: "already_joined" });
  });

  it(`admits up to MAX_GUESTS (${MAX_GUESTS}) and rejects the next`, () => {
    let room = openRoom(T0);
    for (let i = 0; i < MAX_GUESTS; i += 1) {
      const result = joinGuest(room, `guest-${i}`, T0);
      expect(result.ok).toBe(true);
      if (result.ok) room = result.value;
    }
    expect(room.guestIds).toHaveLength(MAX_GUESTS);
    const overflow = joinGuest(room, "guest-overflow", T0);
    expect(overflow).toEqual({ ok: false, error: "room_full" });
  });

  it("rejects joining an expired room", () => {
    const room = openRoom(T0);
    const result = joinGuest(room, "guest-1", T0 + ROOM_TTL_MS + 1);
    expect(result).toEqual({ ok: false, error: "room_expired" });
  });

  it("rejects joining a closed room", () => {
    const room = closeRoom(openRoom(T0));
    const result = joinGuest(room, "guest-1", T0);
    expect(result).toEqual({ ok: false, error: "room_closed" });
  });
});

describe("removeConnection", () => {
  it("closes the whole room when the host disconnects", () => {
    let room = openRoom(T0);
    const joined = joinGuest(room, "guest-1", T0);
    if (!joined.ok) throw new Error("test setup failed");
    room = joined.value;

    const afterHostLeaves = removeConnection(room, "host-1");
    expect(afterHostLeaves.phase).toBe("closed");
    expect(afterHostLeaves.hostId).toBeNull();
    expect(afterHostLeaves.guestIds).toEqual([]);
  });

  it("frees a guest's slot without closing the room", () => {
    let room = openRoom(T0);
    for (const id of ["guest-1", "guest-2"]) {
      const result = joinGuest(room, id, T0);
      if (!result.ok) throw new Error("test setup failed");
      room = result.value;
    }
    const afterOneLeaves = removeConnection(room, "guest-1");
    expect(afterOneLeaves.phase).toBe("open");
    expect(afterOneLeaves.guestIds).toEqual(["guest-2"]);
  });

  it("is a no-op for an unknown connection id", () => {
    const room = openRoom(T0);
    expect(removeConnection(room, "nobody")).toEqual(room);
  });

  it("is a no-op once the room is already closed", () => {
    const room = closeRoom(openRoom(T0));
    expect(removeConnection(room, "host-1")).toEqual(room);
  });
});

describe("routeSignal", () => {
  function roomWithGuests() {
    let room = openRoom(T0);
    for (const id of ["guest-1", "guest-2"]) {
      const result = joinGuest(room, id, T0);
      if (!result.ok) throw new Error("test setup failed");
      room = result.value;
    }
    return room;
  }

  it("routes a host message to the addressed guest only", () => {
    const room = roomWithGuests();
    const result = routeSignal(room, { fromId: "host-1", toId: "guest-2", byteLength: 100 });
    expect(result).toEqual({ ok: true, value: ["guest-2"] });
  });

  it("routes a guest message to the host, ignoring any toId", () => {
    const room = roomWithGuests();
    const result = routeSignal(room, { fromId: "guest-1", byteLength: 100 });
    expect(result).toEqual({ ok: true, value: ["host-1"] });
  });

  it("rejects a host message with no target", () => {
    const room = roomWithGuests();
    const result = routeSignal(room, { fromId: "host-1", byteLength: 100 });
    expect(result).toEqual({ ok: false, error: "missing_target" });
  });

  it("rejects a host message addressed to a non-existent guest", () => {
    const room = roomWithGuests();
    const result = routeSignal(room, { fromId: "host-1", toId: "ghost", byteLength: 100 });
    expect(result).toEqual({ ok: false, error: "unknown_target" });
  });

  it("rejects a message from a connection that is not in the room", () => {
    const room = roomWithGuests();
    const result = routeSignal(room, { fromId: "stranger", byteLength: 100 });
    expect(result).toEqual({ ok: false, error: "unknown_sender" });
  });

  it("rejects an oversized message", () => {
    const room = roomWithGuests();
    const result = routeSignal(room, {
      fromId: "guest-1",
      byteLength: 16 * 1024 + 1,
    });
    expect(result).toEqual({ ok: false, error: "message_too_large" });
  });

  it("rejects relaying in a room that is not open", () => {
    const room = newRoom("ABC123", T0);
    const result = routeSignal(room, { fromId: "host-1", toId: "guest-1", byteLength: 10 });
    expect(result).toEqual({ ok: false, error: "room_not_open" });
  });
});
