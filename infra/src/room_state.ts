//! The room-code signaling state machine. Pure -- no I/O, no clock reads,
//! no WebSocket, no Durable Object glue. `room_durable_object.ts` is the
//! only caller and owns everything impure (storage, sockets, hibernation).
//!
//! A room is a single-use coordination point: one host, up to
//! `MAX_GUESTS` guests, relaying opaque signaling blobs between the host
//! and each guest so they can establish their own WebRTC connections. The
//! DO never inspects the blob contents -- only the routing envelope
//! (`RelayEnvelope.toId`) is read, and that is addressing metadata, not
//! game or signaling payload.

import { type Result, err, ok } from "./result.ts";

/** A room holds at most one host plus this many guests (host-star, up to 4v4). */
export const MAX_GUESTS = 7;

/** Total occupant cap: one host plus `MAX_GUESTS` guests. */
export const MAX_OCCUPANTS = MAX_GUESTS + 1;

/**
 * A room with no live activity (a host connecting, a guest joining, or a
 * relayed signal -- see `touch`) for this long expires. Originally measured
 * from `createdAtMs` alone; that punished an occupied room for outliving a
 * fixed window even while its host was still sitting in it. Sliding instead
 * -- measured from `lastActivityMs` -- restores the ORIGINAL intent ("nobody
 * finishes claiming/joining within this window") for a room nobody ever
 * touches, while a room people are actually using stays alive as long as
 * they keep using it, bounded only by `ROOM_MAX_LIFETIME_MS`.
 */
export const ROOM_IDLE_TTL_MS = 10 * 60 * 1000;

/**
 * Hard cap on a room's total lifetime, measured from `createdAtMs`,
 * regardless of how recently it saw activity -- so a room that keeps
 * trickling activity (an abandoned tab left open, a socket the platform
 * never told this Durable Object had closed) cannot pin it alive forever.
 */
export const ROOM_MAX_LIFETIME_MS = 2 * 60 * 60 * 1000;

/** Signaling blobs (SDP offers/answers, ICE candidates) are small; cap generously. */
export const MAX_SIGNAL_BYTES = 16 * 1024;

/** Identifies one WebSocket connection within a room. Opaque to callers. */
export type ConnectionId = string;

/** The room's lifecycle phase. */
export type RoomPhase = "waiting_for_host" | "open" | "closed";

/** The room's whole state, as a plain, immutable, serializable value. */
export interface RoomState {
  /** The room code this state belongs to (also the Durable Object's name). */
  readonly code: string;
  /** When the room was created, in epoch milliseconds. */
  readonly createdAtMs: number;
  /** When the room last saw live activity, in epoch milliseconds -- see `touch`. */
  readonly lastActivityMs: number;
  /** Current lifecycle phase. */
  readonly phase: RoomPhase;
  /** The host's connection id, or `null` before a host has claimed the room. */
  readonly hostId: ConnectionId | null;
  /** Connected guests' connection ids, in join order. */
  readonly guestIds: readonly ConnectionId[];
}

/** A freshly created, unclaimed room. */
export function newRoom(code: string, nowMs: number): RoomState {
  return {
    code,
    createdAtMs: nowMs,
    lastActivityMs: nowMs,
    phase: "waiting_for_host",
    hostId: null,
    guestIds: [],
  };
}

/**
 * Marks the room as having live activity right now -- re-arms the sliding
 * idle window (`ROOM_IDLE_TTL_MS`). Called only for events that mean someone
 * is genuinely still using the room: a host claiming it, a guest joining it,
 * or a signal being relayed through it -- never for a FAILED admission
 * attempt (a probe against a stale code must not keep it alive) and never
 * for a disconnect (there is nothing left to keep alive).
 */
export function touch(state: RoomState, nowMs: number): RoomState {
  return { ...state, lastActivityMs: nowMs };
}

/** Whether `state` has outlived `ROOM_MAX_LIFETIME_MS` since creation, or
 * `ROOM_IDLE_TTL_MS` since its last live activity -- whichever comes first. */
export function isExpired(state: RoomState, nowMs: number): boolean {
  if (nowMs - state.createdAtMs > ROOM_MAX_LIFETIME_MS) {
    return true;
  }
  return nowMs - state.lastActivityMs > ROOM_IDLE_TTL_MS;
}

/** The epoch millisecond this room's cleanup alarm should next fire at:
 * whichever comes first of the sliding idle deadline (from `lastActivityMs`)
 * or the hard-cap deadline (from `createdAtMs`). The Durable Object calls
 * this after every event that touches the room (`claimHost`/`joinGuest`
 * succeeding, a relayed signal, a failed admission attempt) so the alarm
 * always reflects the room's CURRENT state rather than the window that was
 * true when it was first scheduled. */
export function nextAlarmMs(state: RoomState): number {
  return Math.min(
    state.lastActivityMs + ROOM_IDLE_TTL_MS,
    state.createdAtMs + ROOM_MAX_LIFETIME_MS,
  );
}

/**
 * Whether this room currently has a live, unexpired host claim. Exactly the
 * condition `claimHost` below fails on with `host_already_claimed` once its
 * own `closed`/`expired` checks have already passed -- exported as its own
 * function so the Durable Object's collision-probe RPC
 * (`room_durable_object.ts`'s `isClaimedByHost`) derives the same answer
 * `claimHost` does from one place, rather than an inline re-derivation
 * drifting out of sync with it (round-2 council review, blocking finding 1).
 */
export function isRoomClaimedByHost(state: RoomState, nowMs: number): boolean {
  return state.phase !== "closed" && !isExpired(state, nowMs) && state.hostId !== null;
}

/** A host connection claims this room. Fails if already claimed, closed, or expired. */
export function claimHost(
  state: RoomState,
  connectionId: ConnectionId,
  nowMs: number,
): Result<RoomState> {
  if (state.phase === "closed") {
    return err("room_closed");
  }
  if (isExpired(state, nowMs)) {
    return err("room_expired");
  }
  // Equivalent to `isRoomClaimedByHost(state, nowMs)` at this point --
  // `closed`/`expired` are already ruled out above, so it reduces to
  // exactly `state.hostId !== null` -- written via the shared predicate
  // rather than repeating the bare check, so the two never drift apart.
  if (isRoomClaimedByHost(state, nowMs)) {
    return err("host_already_claimed");
  }
  return ok(touch({ ...state, phase: "open", hostId: connectionId }, nowMs));
}

/** A guest connection joins this room by its code. Fails if unclaimed, full, or expired. */
export function joinGuest(
  state: RoomState,
  connectionId: ConnectionId,
  nowMs: number,
): Result<RoomState> {
  if (state.phase === "closed") {
    return err("room_closed");
  }
  if (state.phase === "waiting_for_host") {
    // No host has ever claimed this code -- from a guest's point of view
    // (and the DO's: a fresh, never-claimed room and a genuinely unknown
    // code are indistinguishable at this layer) this reads as "no such
    // room", not "the room exists but is not accepting guests yet".
    return err("room_not_found");
  }
  if (isExpired(state, nowMs)) {
    return err("room_expired");
  }
  if (state.guestIds.includes(connectionId)) {
    return err("already_joined");
  }
  if (state.guestIds.length >= MAX_GUESTS) {
    return err("room_full");
  }
  return ok(touch({ ...state, guestIds: [...state.guestIds, connectionId] }, nowMs));
}

/**
 * A connection (host or guest) disconnects. The host leaving ends the room
 * for everyone -- there is no one left to relay signaling toward, and the
 * code is single-use, so the room closes rather than waiting for a new
 * host. A guest leaving just frees its slot.
 */
export function removeConnection(state: RoomState, connectionId: ConnectionId): RoomState {
  if (state.phase === "closed") {
    return state;
  }
  if (state.hostId === connectionId) {
    return closeRoom(state);
  }
  if (state.guestIds.includes(connectionId)) {
    return { ...state, guestIds: state.guestIds.filter((id) => id !== connectionId) };
  }
  return state;
}

/** Close the room unconditionally (TTL expiry, host departure, explicit shutdown). */
export function closeRoom(state: RoomState): RoomState {
  if (state.phase === "closed") {
    return state;
  }
  return { ...state, phase: "closed", hostId: null, guestIds: [] };
}

/**
 * Whether the transition from `prevState` to `nextState` (a `removeConnection`
 * call's before/after) was specifically the host leaving an open room -- the
 * one case that should announce `{"type":"host_left"}` to every connected
 * guest before the grace-period alarm closes their sockets. A room that was
 * ALREADY closed (idle/hard-cap expiry, or a repeat disconnect after the
 * host has already left) reports `false`: there is no fresh departure to
 * announce, only a room that was already gone.
 */
export function hostDeparted(prevState: RoomState, nextState: RoomState): boolean {
  return prevState.phase !== "closed" && prevState.hostId !== null && nextState.phase === "closed";
}

/** One signaling message to be relayed, described without touching its payload. */
export interface RelayEnvelope {
  /** The connection id the message came from. */
  readonly fromId: ConnectionId;
  /**
   * The connection id the message is addressed to. Required when `fromId`
   * is the host (who must pick which guest to address); ignored for a
   * guest, whose only possible recipient is the host.
   */
  readonly toId?: ConnectionId;
  /** Size of the raw message, for the `MAX_SIGNAL_BYTES` cap. */
  readonly byteLength: number;
}

/**
 * Resolve who a signaling message should be relayed to. Returns the list
 * of recipient connection ids (always exactly one, today) or an error.
 * Reads only `fromId`/`toId` -- routing metadata -- never the message body.
 */
export function routeSignal(
  state: RoomState,
  envelope: RelayEnvelope,
): Result<readonly ConnectionId[]> {
  if (state.phase !== "open") {
    return err("room_not_open");
  }
  if (envelope.byteLength > MAX_SIGNAL_BYTES) {
    return err("message_too_large");
  }
  if (envelope.fromId === state.hostId) {
    if (envelope.toId === undefined) {
      return err("missing_target");
    }
    if (!state.guestIds.includes(envelope.toId)) {
      return err("unknown_target");
    }
    return ok([envelope.toId]);
  }
  if (state.guestIds.includes(envelope.fromId)) {
    if (state.hostId === null) {
      return err("no_host");
    }
    return ok([state.hostId]);
  }
  return err("unknown_sender");
}
