//! `RoomDurableObject`: the impure glue around `room_state.ts`'s pure
//! reducer. One instance per room code -- `getByName(code)` in `index.ts`
//! -- addressed by `ctx.id.name`, which the platform recovers for us
//! because every stub is created via `getByName`, never `newUniqueId`.
//!
//! Hibernation-safe by construction: nothing this class needs to answer a
//! request lives only in JS memory. The room's authoritative state lives
//! in one SQLite row (`loadState`/`saveState`); each live socket's own
//! identity (`connectionId`, `role`) travels with it via
//! `serializeAttachment`, which the runtime preserves across a
//! hibernate/wake cycle even though the class instance itself is
//! discarded and re-constructed.
//!
//! The one thing this file is not allowed to do, per issue #551 and
//! `docs/online/relay_topology_decision.md`'s "dumb relay" principle: read
//! the CONTENTS of a signaling message. `webSocketMessage` below reads
//! exactly one field from a host's message (`to`, the routing envelope)
//! and forwards `body` untouched in both directions -- but "untouched" is
//! NOT the same shape both ways; a client (the #552 issue this note is
//! for) needs to know the wire protocol exactly:
//!
//! - **Guest -> DO**: any text frame, sent as-is -- this is the guest's
//!   own signaling payload (e.g. a JSON-stringified `RTCSessionDescription`
//!   or ICE candidate), and the DO never parses it at all.
//! - **DO -> host** (relaying that guest message):
//!   `{ "type": "signal", "from": "<guestConnectionId>", "body": "<the
//!   guest's exact text> }`. Because `body` is embedded as a JSON STRING
//!   value inside this envelope, and the guest's own text is very likely
//!   already JSON, the host receives `body` **double-encoded**: parse the
//!   envelope first, then `body` is the guest's original string, to parse
//!   again if it was JSON.
//! - **Host -> DO**: `{ "to": "<guestConnectionId>", "body": <anything> }`
//!   -- `body` may be a string OR any other JSON value; the DO only reads
//!   `to`, never `body`.
//! - **DO -> guest** (relaying that host message):
//!   `{ "type": "signal", "from": "host", "body": <exactly what the host
//!   put there> }` -- NOT re-stringified, so a guest's `body` is whatever
//!   type the host sent (commonly a string, to keep both directions
//!   symmetric, but the DO does not enforce that).
//!
//! Binary (`ArrayBuffer`) frames are rejected outright, from either role
//! -- see `webSocketMessage`'s own comment for why forwarding one would be
//! silent data loss rather than a relay.
//!
//! ## Admission failures are in-band
//!
//! A browser `WebSocket` cannot read the HTTP status of a failed upgrade at
//! all (the platform gives a script no access to it -- see `@gc/online`'s
//! `room_signaling.ts`, `RoomSignalingFailureReason`'s own doc), so a
//! pre-upgrade HTTP rejection is invisible to a client past "something went
//! wrong". `fetch`
//! below completes the WebSocket upgrade EVEN ON an admission failure
//! (`claimHost`/`joinGuest` rejecting), sends this file's existing in-band
//! frame shape, `{ "type": "error", "error": "<reason>" }`, then closes with
//! a reason-mapped code (`ADMISSION_CLOSE_CODE`) and never accepts the
//! socket into hibernation (`rejectInBand`'s own comment). The reasons
//! `claimHost`/`joinGuest` (`room_state.ts`) can produce, every one of them
//! now reaching a client this way: `room_not_found` (a guest addressed a
//! code no host has ever claimed -- indistinguishable, at this layer, from a
//! code that was never issued at all), `room_full`, `room_expired`,
//! `room_closed`, `host_already_claimed`, `already_joined`. Two requests
//! stay pre-upgrade HTTP rejections BY DESIGN, both worker-level and both
//! cheap to reject before a Durable Object is even addressed: the per-IP
//! rate limit and a malformed room-code shape (`index.ts`'s
//! `handleHostSignal`/`handleJoinSignal`). `room_not_open` -- returned by
//! `routeSignal`, not `claimHost`/`joinGuest` -- is a DIFFERENT, already
//! in-band case: a signal arriving on an already-admitted socket for a room
//! that is no longer open (`webSocketMessage` below), not an admission
//! failure at all.
//!
//! ## Collision probe (`fetch`'s non-upgrade branch)
//!
//! Every admission failure completing the upgrade (above) removes the ONE
//! signal `index.ts`'s `handleHostSignal` used to read to know a freshly
//! generated room code collided with an existing live room and it should
//! retry with a different one (an HTTP 409, pre-upgrade). A non-websocket
//! request to this same DO -- `fetch`'s `Upgrade` header check failing --
//! is now that signal instead: a cheap, side-effect-free, HTTP 409-if-
//! claimed/200-otherwise check `handleHostSignal` makes BEFORE attempting
//! the real upgrade for each candidate code, so a collision (astronomically
//! unlikely -- `room_code.ts`'s own doc) is caught without ever opening,
//! and immediately tearing down, a real WebSocket for the losing code.
//!
//! ## Sliding TTL
//!
//! `room_state.ts`'s `ROOM_IDLE_TTL_MS`/`ROOM_MAX_LIFETIME_MS`/`touch`/
//! `nextAlarmMs` are the pure half of this; this file's only job is calling
//! `nextAlarmMs` and `ctx.storage.setAlarm` after anything that counts as
//! activity (a claim or join succeeding, in `fetch`; a relayed signal, in
//! `webSocketMessage`) so the cleanup alarm always reflects the room's
//! current idle deadline rather than the one that was true when it was
//! first scheduled. A FAILED admission attempt does not touch the room
//! (`touch`'s own doc) but still re-arms the alarm from the room's
//! unchanged state, so a code nobody ever successfully claims still cleans
//! up on schedule.
//!
//! ## Host departure is an event
//!
//! When the host's socket disconnects and that closes the room
//! (`hostDeparted`, `room_state.ts`), every currently connected guest
//! receives `{ "type": "host_left" }` -- see `disconnect` below -- BEFORE
//! the grace-period alarm (`CLOSE_GRACE_MS`) closes their sockets. This is
//! a `send`, not a `close`, from inside another socket's own
//! `webSocketClose` handler, so the hibernation-timing hazard `disconnect`'s
//! own comment documents for `close()` does not apply to it.

import { DurableObject } from "cloudflare:workers";

import { type FixedWindowState, tryConsume } from "./rate_limiter.ts";
import {
  type ConnectionId,
  type RoomState,
  claimHost,
  closeRoom,
  hostDeparted,
  isExpired,
  joinGuest,
  newRoom,
  nextAlarmMs,
  removeConnection,
  routeSignal,
  touch,
} from "./room_state.ts";

/** At most this many join *attempts* (successful or not) per room per window -- abuse guard. */
const JOIN_RATE_LIMIT = { limit: 20, windowMs: 60_000 };

/** How long a just-closed room's storage lingers before cleanup, to let in-flight sends land. */
const CLOSE_GRACE_MS = 5_000;

/**
 * WebSocket close codes for each admission-failure reason this DO reports
 * in-band (this module's doc, "Admission failures"), in the 4000-4999
 * "private use" range -- RFC 6455 §7.4.2 reserves 3000-3999 for
 * IANA-registered libraries/frameworks. Distinct per reason so anything
 * inspecting the close event alone (not just the `{"type":"error"}` frame
 * that always arrives first) can still tell causes apart. Every reason
 * `claimHost`/`joinGuest` (`room_state.ts`) can actually produce is listed;
 * `DEFAULT_ADMISSION_CLOSE_CODE` is a defensive fallback for anything else.
 */
const ADMISSION_CLOSE_CODE: Readonly<Record<string, number>> = {
  room_not_found: 4404,
  room_full: 4408,
  room_expired: 4409,
  room_closed: 4410,
  host_already_claimed: 4411,
  already_joined: 4412,
};

/** Fallback for `ADMISSION_CLOSE_CODE` -- see its own doc. */
const DEFAULT_ADMISSION_CLOSE_CODE = 4400;

/** Exported for room_durable_object.spec.ts -- AGENTS.md §4: everything a
 * test touches is reachable. The rest of this class's behavior needs a real
 * Workers runtime (`vitest.config.ts`'s own doc); this mapping does not. */
export function closeCodeForAdmissionFailure(reason: string): number {
  return ADMISSION_CLOSE_CODE[reason] ?? DEFAULT_ADMISSION_CLOSE_CODE;
}

type Role = "host" | "guest";

interface SocketAttachment {
  readonly connectionId: ConnectionId;
  readonly role: Role;
}

function isSocketAttachment(value: unknown): value is SocketAttachment {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const v = value as Partial<SocketAttachment>;
  return typeof v.connectionId === "string" && (v.role === "host" || v.role === "guest");
}

interface RoomRow extends Record<string, SqlStorageValue> {
  readonly code: string;
  readonly created_at_ms: number;
  readonly last_activity_ms: number;
  readonly phase: string;
  readonly host_id: string | null;
  readonly guest_ids: string;
  readonly join_window_start_ms: number | null;
  readonly join_window_count: number | null;
}

/** The room-code signaling Durable Object. See this module's doc for the shape. */
export class RoomDurableObject extends DurableObject<Env> {
  public constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // sql.exec is synchronous -- schema setup finishes before the
    // constructor returns, with nothing to gate via blockConcurrencyWhile
    // (which is for holding off requests during genuinely ASYNC init).
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS room (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        code TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        last_activity_ms INTEGER NOT NULL,
        phase TEXT NOT NULL,
        host_id TEXT,
        guest_ids TEXT NOT NULL,
        join_window_start_ms INTEGER,
        join_window_count INTEGER
      )
    `);
    // `CREATE TABLE IF NOT EXISTS` above is a no-op against a room row that
    // was already created before the sliding-TTL revision added this
    // column, so a leftover pre-migration room needs it added explicitly.
    // Backfilling with 0 makes `isExpired` see it as maximally stale --
    // exactly right: a room this old should already be gone, and the next
    // thing that reads it (a request, or this instance's own next alarm)
    // closes it out promptly instead of erroring on a missing column.
    try {
      this.ctx.storage.sql.exec(
        "ALTER TABLE room ADD COLUMN last_activity_ms INTEGER NOT NULL DEFAULT 0",
      );
    } catch {
      // Column already exists -- the common case, for every room created
      // under this schema version, including every brand-new one (the
      // CREATE TABLE above already added it).
    }
  }

  private codeFromId(): string {
    const name = this.ctx.id.name;
    if (name === undefined) {
      // Programmer error, not a client-triggerable one: every stub in this
      // codebase is created via getByName(code) (index.ts), never
      // newUniqueId() or idFromString(). AGENTS.md §7: fail loud.
      throw new Error("RoomDurableObject must be addressed via getByName(code)");
    }
    return name;
  }

  private loadState(nowMs: number): RoomState {
    const row = this.ctx.storage.sql
      .exec<RoomRow>(
        "SELECT code, created_at_ms, last_activity_ms, phase, host_id, guest_ids, join_window_start_ms, join_window_count FROM room WHERE id = 1",
      )
      .toArray()[0];
    if (row === undefined) {
      return newRoom(this.codeFromId(), nowMs);
    }
    return {
      code: row.code,
      createdAtMs: row.created_at_ms,
      lastActivityMs: row.last_activity_ms,
      phase: row.phase as RoomState["phase"],
      hostId: row.host_id,
      guestIds: JSON.parse(row.guest_ids) as ConnectionId[],
    };
  }

  private loadJoinWindow(): FixedWindowState | null {
    const row = this.ctx.storage.sql
      .exec<RoomRow>(
        "SELECT code, created_at_ms, last_activity_ms, phase, host_id, guest_ids, join_window_start_ms, join_window_count FROM room WHERE id = 1",
      )
      .toArray()[0];
    if (row === undefined || row.join_window_start_ms === null || row.join_window_count === null) {
      return null;
    }
    return { windowStartMs: row.join_window_start_ms, count: row.join_window_count };
  }

  private saveState(state: RoomState, joinWindow?: FixedWindowState): void {
    this.ctx.storage.sql.exec(
      `INSERT INTO room (id, code, created_at_ms, last_activity_ms, phase, host_id, guest_ids, join_window_start_ms, join_window_count)
       VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT (id) DO UPDATE SET
         code = excluded.code,
         created_at_ms = excluded.created_at_ms,
         last_activity_ms = excluded.last_activity_ms,
         phase = excluded.phase,
         host_id = excluded.host_id,
         guest_ids = excluded.guest_ids,
         join_window_start_ms = COALESCE(excluded.join_window_start_ms, room.join_window_start_ms),
         join_window_count = COALESCE(excluded.join_window_count, room.join_window_count)`,
      state.code,
      state.createdAtMs,
      state.lastActivityMs,
      state.phase,
      state.hostId,
      JSON.stringify(state.guestIds),
      joinWindow?.windowStartMs ?? null,
      joinWindow?.count ?? null,
    );
  }

  public override async fetch(request: Request): Promise<Response> {
    if (request.headers.get("Upgrade") !== "websocket") {
      // Not a WebSocket upgrade at all: `handleHostSignal`'s own collision
      // probe (index.ts) -- see this module's doc, "Collision probe". The
      // only other caller of a non-upgrade request against this DO.
      return this.probeHostClaim();
    }
    const role = request.headers.get("Room-Role");
    if (role !== "host" && role !== "guest") {
      // Set by index.ts, never by the client directly -- see its own doc.
      return new Response("missing or invalid Room-Role", { status: 400 });
    }

    const nowMs = Date.now();
    const state = this.loadState(nowMs);

    const windowResult = tryConsume(this.loadJoinWindow(), nowMs, JOIN_RATE_LIMIT);
    if (!windowResult.ok) {
      return new Response("too many join attempts for this room", { status: 429 });
    }

    const connectionId = crypto.randomUUID();
    const claimResult =
      role === "host"
        ? claimHost(state, connectionId, nowMs)
        : joinGuest(state, connectionId, nowMs);
    if (!claimResult.ok) {
      // Admission failure: complete the upgrade and refuse IN-BAND instead
      // of rejecting the HTTP request -- see this module's doc, "Admission
      // failures". `state` (not touched -- see `touch`'s own doc) is what
      // gets persisted and what the alarm re-arms from, so a code nobody
      // ever successfully claims still cleans up on schedule.
      this.saveState(state, windowResult.value);
      await this.ctx.storage.setAlarm(nextAlarmMs(state));
      return this.rejectInBand(claimResult.error);
    }

    const nextState = claimResult.value;
    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    const attachment: SocketAttachment = { connectionId, role };
    server.serializeAttachment(attachment);
    this.ctx.acceptWebSocket(server);
    this.saveState(nextState, windowResult.value);
    await this.ctx.storage.setAlarm(nextAlarmMs(nextState));

    server.send(
      JSON.stringify(
        role === "host"
          ? { type: "created", code: state.code }
          : { type: "joined", code: state.code },
      ),
    );
    if (role === "guest") {
      // The host has no other way to learn a guest's connection id --
      // and needs it to address that guest in a signal's `to` field.
      this.sendToHost({ type: "guest_joined", guestId: connectionId });
    }

    return new Response(null, { status: 101, webSocket: client });
  }

  /** Non-upgrade probe -- see this module's doc, "Collision probe". Reads
   * state, saves nothing, never touches a WebSocket. */
  private probeHostClaim(): Response {
    const nowMs = Date.now();
    const state = this.loadState(nowMs);
    const claimed = state.phase !== "closed" && !isExpired(state, nowMs) && state.hostId !== null;
    return new Response(null, { status: claimed ? 409 : 200 });
  }

  /** Completes the WebSocket upgrade for an admission failure instead of
   * rejecting the HTTP request -- see this module's doc, "Admission
   * failures". `server.accept()` (not `ctx.acceptWebSocket`) is deliberate:
   * this socket sends exactly one frame and closes immediately, so it never
   * needs hibernation, and skipping `ctx.acceptWebSocket` keeps it out of
   * `ctx.getWebSockets()` entirely -- nothing here should ever try to relay
   * through, or broadcast to, a socket that was never admitted. */
  private rejectInBand(reason: string): Response {
    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    server.accept();
    server.send(JSON.stringify({ type: "error", error: reason }));
    server.close(closeCodeForAdmissionFailure(reason), reason);
    return new Response(null, { status: 101, webSocket: client });
  }

  /** Send `body` to the room's host socket, if one is currently connected. */
  private sendToHost(body: unknown): void {
    const message = JSON.stringify(body);
    for (const socket of this.ctx.getWebSockets()) {
      const attachment: unknown = socket.deserializeAttachment();
      if (isSocketAttachment(attachment) && attachment.role === "host") {
        socket.send(message);
      }
    }
  }

  /** Send `body` to every connected guest socket -- the opposite direction
   * from `sendToHost`, used only for `host_left` (this module's doc, "Host
   * departure"). */
  private sendToGuests(body: unknown): void {
    const message = JSON.stringify(body);
    for (const socket of this.ctx.getWebSockets()) {
      const attachment: unknown = socket.deserializeAttachment();
      if (isSocketAttachment(attachment) && attachment.role === "guest") {
        socket.send(message);
      }
    }
  }

  public override async webSocketMessage(
    ws: WebSocket,
    message: string | ArrayBuffer,
  ): Promise<void> {
    const attachment: unknown = ws.deserializeAttachment();
    if (!isSocketAttachment(attachment)) {
      ws.close(1011, "unrecognized connection");
      return;
    }

    // Neither role may send a binary frame: a host's message must be JSON
    // (the { to, body } envelope below), and a guest's message is
    // forwarded to the host completely unparsed -- as a STRING, always
    // (see this file's module doc, "wire protocol"). An ArrayBuffer
    // handed to JSON.stringify serializes as "{}", which would silently
    // drop a guest's payload rather than relay it; reject it instead of
    // forwarding garbage.
    if (typeof message !== "string") {
      ws.send(JSON.stringify({ type: "error", error: "binary_not_supported" }));
      return;
    }

    const byteLength = new TextEncoder().encode(message).length;

    // Routing metadata only. `body` is forwarded exactly as received in
    // both directions -- this is the one place this file reads any part of
    // a signaling message, and it stops at `to`.
    let toId: ConnectionId | undefined;
    let outgoingBody: unknown = message;
    if (attachment.role === "host") {
      let envelope: unknown;
      try {
        envelope = JSON.parse(message);
      } catch {
        ws.send(JSON.stringify({ type: "error", error: "invalid_envelope" }));
        return;
      }
      const to = (envelope as { to?: unknown }).to;
      if (typeof to !== "string") {
        ws.send(JSON.stringify({ type: "error", error: "missing_target" }));
        return;
      }
      toId = to;
      outgoingBody = (envelope as { body?: unknown }).body;
    }

    const nowMs = Date.now();
    const state = this.loadState(nowMs);
    const routeResult = routeSignal(state, {
      fromId: attachment.connectionId,
      byteLength,
      ...(toId !== undefined ? { toId } : {}),
    });
    if (!routeResult.ok) {
      ws.send(JSON.stringify({ type: "error", error: routeResult.error }));
      return;
    }

    // A relayed signal is live activity -- re-arm the sliding idle window
    // (this module's doc, "Sliding TTL").
    const touched = touch(state, nowMs);
    this.saveState(touched);
    await this.ctx.storage.setAlarm(nextAlarmMs(touched));

    const outgoing = JSON.stringify(
      attachment.role === "host"
        ? { type: "signal", from: "host", body: outgoingBody }
        : { type: "signal", from: attachment.connectionId, body: outgoingBody },
    );

    for (const socket of this.ctx.getWebSockets()) {
      const other: unknown = socket.deserializeAttachment();
      if (isSocketAttachment(other) && routeResult.value.includes(other.connectionId)) {
        socket.send(outgoing);
      }
    }
  }

  public override async webSocketClose(ws: WebSocket): Promise<void> {
    await this.disconnect(ws);
  }

  public override async webSocketError(ws: WebSocket): Promise<void> {
    await this.disconnect(ws);
  }

  private async disconnect(ws: WebSocket): Promise<void> {
    const attachment: unknown = ws.deserializeAttachment();
    if (!isSocketAttachment(attachment)) {
      return;
    }
    const state = this.loadState(Date.now());
    const nextState = removeConnection(state, attachment.connectionId);
    this.saveState(nextState);

    if (nextState.phase === "closed") {
      if (hostDeparted(state, nextState)) {
        // Tell every connected guest BEFORE the grace-period alarm closes
        // their sockets (this module's doc, "Host departure"). A `send()`
        // to OTHER sockets from inside this socket's own webSocketClose
        // handler is fine -- the hibernation-timing hazard documented right
        // below is specific to `close()`, not `send()`.
        this.sendToGuests({ type: "host_left" });
      }
      // Close the OTHER live sockets from the alarm handler, not here.
      // A hibernatable socket's close() called synchronously from inside a
      // DIFFERENT socket's own webSocketClose handler does not reliably
      // deliver the close frame in every runtime (observed locally under
      // `wrangler dev`); a fresh alarm invocation closing them does. The
      // short grace period this schedules is imperceptible for a
      // handshake-only relay.
      await this.ctx.storage.setAlarm(Date.now() + CLOSE_GRACE_MS);
    } else if (attachment.role === "guest") {
      // A guest left but the room (and the host's connection) is still
      // live -- tell the host so it can tear down that guest's peer
      // connection instead of waiting on a WebRTC-level timeout.
      this.sendToHost({ type: "guest_left", guestId: attachment.connectionId });
    }
  }

  public override alarm(): void {
    const state = this.loadState(Date.now());
    if (state.phase !== "closed") {
      this.saveState(closeRoom(state));
    }
    // Unconditional: this alarm is the cleanup point both for TTL expiry
    // (state was still open above) and for the short grace period
    // disconnect() schedules once a host departure has already closed the
    // room -- either way, any socket still open at this point should not
    // be.
    for (const socket of this.ctx.getWebSockets()) {
      try {
        socket.close(1000, "room closed");
      } catch {
        // Already closing; nothing to do.
      }
    }
    // Deliberately NOT ctx.storage.deleteAll(): closing a socket here
    // schedules its OWN webSocketClose callback asynchronously, which
    // calls disconnect() -> loadState() on this same instance. Dropping
    // the table now would make that later, already-in-flight callback
    // fail with "no such table". A closed room's one-row footprint costs
    // nothing worth racing this for; the row simply stays "closed" and
    // isValidRoomCode/claimHost reject anything further against this code.
  }
}
