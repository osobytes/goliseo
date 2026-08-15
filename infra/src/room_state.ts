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

/** A room that nobody finishes claiming/joining within this window expires. */
export const ROOM_TTL_MS = 10 * 60 * 1000;

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
  /** Current lifecycle phase. */
  readonly phase: RoomPhase;
  /** The host's connection id, or `null` before a host has claimed the room. */
  readonly hostId: ConnectionId | null;
  /** Connected guests' connection ids, in join order. */
  readonly guestIds: readonly ConnectionId[];
}

/** A freshly created, unclaimed room. */
export function newRoom(code: string, nowMs: number): RoomState {
  return { code, createdAtMs: nowMs, phase: "waiting_for_host", hostId: null, guestIds: [] };
}

/** Whether `state` has outlived `ROOM_TTL_MS` since creation. */
export function isExpired(state: RoomState, nowMs: number): boolean {
  return nowMs - state.createdAtMs > ROOM_TTL_MS;
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
  if (state.hostId !== null) {
    return err("host_already_claimed");
  }
  return ok({ ...state, phase: "open", hostId: connectionId });
}

/** A guest connection joins this room by its code. Fails if not open, full, or expired. */
export function joinGuest(
  state: RoomState,
  connectionId: ConnectionId,
  nowMs: number,
): Result<RoomState> {
  if (state.phase === "closed") {
    return err("room_closed");
  }
  if (state.phase === "waiting_for_host") {
    return err("room_not_open");
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
  return ok({ ...state, guestIds: [...state.guestIds, connectionId] });
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
