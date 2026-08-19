// Pure protocol logic for the room-code signaling Worker
// (`infra/src/index.ts`'s `/signal/host`/`/signal/join` routes,
// `infra/src/room_durable_object.ts`'s Durable Object). No `WebSocket` here
// -- see `@gc/app`'s `room_signaling_port.ts` for the impure half that owns
// the actual socket, per AGENTS.md §9 ("the WebSocket itself lives behind a
// port in the app shell").
//
// This module replaces nothing about `lobby_link.ts`'s existing manual
// signaling effects (`open_peer`/`request_offer`/`accept_offer`/
// `accept_answer`) -- those still carry the WebRTC offer/answer blobs into
// the star transport exactly as they do for the manual copy/paste flow.
// What this module adds is a second way for that SAME blob to travel
// between two peers: over the room-code Worker's relay instead of a human's
// clipboard. `@gc/screens`'s `lobby_model.ts` is what wires the two
// together (its own header explains where).
//
// ## Boundary note: the room-code alphabet/length
//
// `ROOM_CODE_ALPHABET`/`ROOM_CODE_LENGTH` mirror `infra/src/room_code.ts`'s
// own constants exactly (Crockford base32, 6 characters). `infra/` is a
// separate Cloudflare Worker deploy target that must stay outside the
// game's Rust/TypeScript dependency graph (AGENTS.md §8/§11), so this is a
// deliberate duplication, not an import -- if the Worker's alphabet or
// length ever changes, this needs a matching update. It exists here only
// for defensive, local shape checks (`isRoomCodeShaped`); the Worker itself
// remains the sole authority on whether a code names a live room.
//
// ## The host/guest body-encoding asymmetry
//
// Read `infra/src/room_durable_object.ts`'s own module doc first -- it is
// the source of truth for the wire, including why a guest's relayed
// message is "double-encoded" from the Durable Object's own point of view.
// Restated for this module's two functions: a HOST addresses its outgoing
// signal with a `{ to, body }` envelope (`encodeHostSignal`); a GUEST has
// exactly one possible recipient (the host) and sends its raw signal text
// with no envelope at all -- there is no `encodeGuestSignal` here because
// there is nothing to encode; the caller just sends the string.
//
// Both roles receive an incoming peer signal the same way: `parseServerFrame`
// returns a `"signal"` frame whose `body` is already the peer's original
// signal string after exactly ONE `JSON.parse` of the wire text (this
// module's own job). Callers must never `JSON.parse` `body` a second time --
// the transport treats a signal as an opaque string end to end, the same
// way `lobby_link.ts`'s `accept_offer`/`accept_answer` effects already
// do (neither ever parses the blob it carries).
//
// ## The slot envelope (#601)
//
// A room-code guest has no way to learn which invitation slot (`guest_N`)
// the host opened for it -- `@gc/screens`'s `lobby_model.ts` (its own
// header, "A guest learns its own slot from the host") needs that value
// carried alongside the offer, and this is the one hop of the round trip
// that is this module's job. The Durable Object's own doc says a host's
// `body` "may be a string OR any other JSON value" and is forwarded to the
// guest "NOT re-stringified" -- so a host addressing a specific slot
// encodes `body` as a small JSON OBJECT instead of a bare string:
// `{ v: 1, slot, payload }`, where `payload` is exactly the string
// `encodeHostSignal`'s old two-argument form used to put there. No DO
// change is needed; the DO never reads `body` at all.
//
// `parseServerFrame` accepts BOTH shapes for a "signal" frame's `body`: a
// plain string (a guest's own signal, always; or a host's, when no slot was
// given), or a well-formed `v: 1` envelope object, in which case the
// returned frame's `body` is the envelope's `payload` and `slot` is set
// alongside it when the envelope carries one. A `v: 1` envelope missing
// `slot` still parses successfully with `slot` simply absent -- see
// `roomPeerSignal`'s own doc in `lobby_model.ts` for why that degrades
// gracefully instead of failing the connection (deploy-window skew: a
// mixed old-host/new-guest pairing keeps working as a single-guest room,
// exactly as it did before #601). Anything else shaped as an object -- not
// a recognized envelope at all -- is still `err("malformed_frame")`, same
// as any other non-string body always was; this module does not go looking
// for a signal inside an unrecognized shape. A GUEST never encodes an
// envelope (guest -> host stays exactly the raw string it always was), so
// this widened acceptance only ever matters for a host's own outgoing
// signal.
//
// The reverse compatibility direction -- an OLD, already-deployed guest
// bundle (predating this envelope) receiving a NEW host's enveloped
// offer -- cannot be patched retroactively, but degrades safely rather than
// hanging: the old `parseServerFrame`'s own body type-check
// (`typeof body !== "string"`) rejects ANY object-shaped body outright,
// envelope or not (this file's own `room_signaling.spec.ts`, "rejects a
// signal frame whose body is not a string", already pins exactly that
// check against an unrelated object shape). The old guest's connection
// therefore ends with a visible `"malformed_frame"` failure -- the room-code
// screen's existing `ROOM_FAILURE_TEXT` fallback path (`lobby_model.ts`) --
// not a silent hang. A stale tab from mid-deploy sees a readable error and
// can retry once it reloads the new bundle.

import { type Result, err, ok } from "@gc/core";

/** The 32-symbol alphabet a room code is drawn from -- see this file's header. */
export const ROOM_CODE_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** Fixed length of a room code -- see this file's header. */
export const ROOM_CODE_LENGTH = 6;

/** The slot envelope's own version tag (#601, this file's header) -- bumped
 * only if the envelope's shape ever needs to change incompatibly. */
const SLOT_ENVELOPE_VERSION = 1;

/** Whether `candidate` has the shape of a code the Worker could have issued.
 * A purely local, defensive check -- it says nothing about whether a room
 * by that code still exists. */
export function isRoomCodeShaped(candidate: string): boolean {
  if (candidate.length !== ROOM_CODE_LENGTH) {
    return false;
  }
  for (const ch of candidate) {
    if (!ROOM_CODE_ALPHABET.includes(ch)) {
      return false;
    }
  }
  return true;
}

/** `infra/src/index.ts`'s host-signaling route. */
export const ROOM_SIGNAL_HOST_PATH = "/signal/host";

/** `infra/src/index.ts`'s join-signaling route, for a given room code. */
export function roomSignalJoinPath(code: string): string {
  return `/signal/join?code=${encodeURIComponent(code)}`;
}

/** One frame the Worker can send over the room-code WebSocket, decoded and
 * validated -- mirrors `infra/src/room_durable_object.ts`'s own module doc
 * exactly (`{type:"created"|"joined", code}`, `{type:"guest_joined"|
 * "guest_left", guestId}`, `{type:"signal", from, body}`, `{type:"error",
 * error}`, `{type:"host_left"}`). `body` is always resolved to a `string`
 * here (see this file's header) regardless of whether the wire carried it
 * bare or inside a `v: 1` slot envelope (#601) -- a caller never sees the
 * envelope shape, only `body` and the optional `slot` recovered from it.
 * Anything that is neither a bare string nor a recognized envelope is a
 * protocol violation, not coerced.
 *
 * `error` is a `string`, not a closed union, because that DO reports it
 * verbatim -- see this module's doc, "why the client is blind" -- and every
 * caller past this module (`room_signaling_port.ts`, `lobby_model.ts`'s
 * `ROOM_FAILURE_TEXT`) already falls back to the raw token for a reason it
 * does not recognize. As of #599 every admission-time reason arrives this
 * way instead of a pre-upgrade HTTP rejection: `room_not_found` (a guest
 * addressed a code no host has ever claimed), `room_full`, `room_expired`,
 * `room_closed`, `host_already_claimed`, `already_joined`. `room_not_open`
 * is a DIFFERENT, already-in-band case -- a signal arriving on an
 * already-admitted socket for a room that is no longer open, not an
 * admission failure. */
export type RoomServerFrame =
  | { readonly type: "created"; readonly code: string }
  | { readonly type: "joined"; readonly code: string }
  | { readonly type: "guest_joined"; readonly guestId: string }
  | { readonly type: "guest_left"; readonly guestId: string }
  // `slot` (#601, this file's header) is present only when the wire's
  // `body` was a `v: 1` envelope that carried one -- a guest's own signal
  // (never enveloped) and a host's slotless signal both omit it.
  | {
      readonly type: "signal";
      readonly from: string;
      readonly body: string;
      readonly slot?: string;
    }
  | { readonly type: "error"; readonly error: string }
  | { readonly type: "host_left" };

/** Recovers `{ body, slot }` from a "signal" frame's raw `body` field --
 * either it is already the signal string (a guest's own, always; or a
 * host's own, when no slot applies), or it is a `v: 1` slot envelope
 * (#601, this file's header) and `payload` is the signal string. `slot` is
 * only ever present in the second case, and only when the envelope itself
 * carried one -- an envelope missing it still resolves, `slot` absent (see
 * this file's header on why that degrades gracefully rather than failing
 * the frame). Anything else -- a non-string, non-envelope value -- is not
 * this function's problem to guess at; the caller treats `undefined` as
 * "not a signal body at all". */
function resolveSignalBody(
  raw: unknown,
): { readonly body: string; readonly slot?: string } | undefined {
  if (typeof raw === "string") {
    return { body: raw };
  }
  if (typeof raw !== "object" || raw === null) {
    return undefined;
  }
  const envelope = raw as Record<string, unknown>;
  const payload = envelope["payload"];
  if (envelope["v"] !== SLOT_ENVELOPE_VERSION || typeof payload !== "string") {
    return undefined;
  }
  const slot = envelope["slot"];
  return typeof slot === "string" ? { body: payload, slot } : { body: payload };
}

/** Parses and validates one raw WebSocket text frame from the room-code
 * Worker. Every failure -- invalid JSON, an unrecognized `type`, a missing
 * or mistyped field -- collapses to `err("malformed_frame")`: a client has
 * no useful way to act on WHICH field was wrong, only that the frame cannot
 * be trusted. */
export function parseServerFrame(raw: string): Result<RoomServerFrame, "malformed_frame"> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return err("malformed_frame");
  }
  if (typeof parsed !== "object" || parsed === null) {
    return err("malformed_frame");
  }
  const value = parsed as Record<string, unknown>;
  const type = value["type"];
  if (type === "created" || type === "joined") {
    const code = value["code"];
    return typeof code === "string" ? ok({ type, code }) : err("malformed_frame");
  }
  if (type === "guest_joined" || type === "guest_left") {
    const guestId = value["guestId"];
    return typeof guestId === "string" ? ok({ type, guestId }) : err("malformed_frame");
  }
  if (type === "signal") {
    const from = value["from"];
    if (typeof from !== "string") {
      return err("malformed_frame");
    }
    const resolved = resolveSignalBody(value["body"]);
    if (resolved === undefined) {
      return err("malformed_frame");
    }
    return ok({ type: "signal", from, ...resolved });
  }
  if (type === "error") {
    const error = value["error"];
    return typeof error === "string" ? ok({ type: "error", error }) : err("malformed_frame");
  }
  if (type === "host_left") {
    return ok({ type: "host_left" });
  }
  return err("malformed_frame");
}

/** Encodes a host's outgoing signal for a specific guest connection id, per
 * `infra/src/room_durable_object.ts`'s "Host -> DO" wire shape. There is no
 * guest-side counterpart -- see this file's header. `slot` (#601, this
 * file's header) wraps `signal` in a `v: 1` envelope when given; omitted,
 * `body` stays the bare string it always was (a stale host build, or a
 * caller with no slot to report -- `lobby_model.ts`'s `onSignal` only ever
 * omits it for a guest's own outgoing answer, never for a host's offer). */
export function encodeHostSignal(toGuestId: string, signal: string, slot?: string): string {
  const body: unknown =
    slot !== undefined ? { v: SLOT_ENVELOPE_VERSION, slot, payload: signal } : signal;
  return JSON.stringify({ to: toGuestId, body });
}

/** Why a room-code connection failed or ended, as a typed state rather than
 * a thrown exception (this issue's own acceptance criterion). A browser
 * `WebSocket` cannot read the HTTP status of a failed upgrade (the platform
 * gives a script no access to it), so a rejection that genuinely happens
 * BEFORE the upgrade completes collapses to `"handshake_failed"`; that is a
 * real, disclosed limitation of the browser WebSocket API, not a gap in
 * this module. Exactly three cases stay pre-upgrade -- this list is
 * exhaustive, kept in sync with `infra/src/room_durable_object.ts`'s own
 * (its doc is the source of truth if the two ever disagree): a per-IP rate
 * limit, a malformed code shape, and the room's own per-code join-attempt
 * limit (`JOIN_RATE_LIMIT`, distinct from the per-IP one -- both surface
 * identically here, as this browser API cannot tell HTTP statuses apart
 * either); a genuine network error collapses here too, for the same
 * platform-visibility reason. As of #599 that is a SMALLER set than it used
 * to be: a bad/expired/full/closed code, or a host-claim collision, now
 * completes the upgrade and arrives as an in-band `{type:"error"}` frame
 * instead (`RoomServerFrame`'s own doc) -- `classifyClose` below never
 * produces those; `room_signaling_port.ts` reports them straight from the
 * frame's `error` field. `"protocol_error"` and `"malformed_frame"` are
 * more specific because they arrive AFTER a successful handshake, over
 * frames this module itself parses. */
export type RoomSignalingFailureReason =
  "handshake_failed" | "malformed_frame" | "protocol_error" | "connection_lost";

export interface RoomSignalingFailure {
  readonly reason: RoomSignalingFailureReason;
  /** Present only for `"protocol_error"`: the Worker's own error code
   * (`infra/src/room_durable_object.ts`'s `{type:"error", error}` frame,
   * e.g. `"message_too_large"`). */
  readonly detail?: string;
}

/** Classifies a socket closing (or erroring) into a `RoomSignalingFailure`.
 * `wasReady` is whatever the caller already knows locally: whether a
 * `"created"`/`"joined"` frame had already arrived before the socket
 * dropped. A close before that point is indistinguishable, from inside a
 * browser, from every other handshake failure (see this type's own doc);
 * a close after it is a genuine mid-session drop. */
export function classifyClose(wasReady: boolean): RoomSignalingFailure {
  return wasReady ? { reason: "connection_lost" } : { reason: "handshake_failed" };
}
