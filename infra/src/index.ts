//! The Worker entry point. `wrangler.jsonc`'s `assets.run_worker_first`
//! routes only `/api/*` and `/signal/*` here -- every other path (the app
//! shell, its JS bundle, its `.wasm`) is served directly from
//! `ts/dist-app/` by the platform's static assets layer without this file
//! running at all.
//!
//! Two routes:
//! - `/signal/host` and `/signal/join` upgrade to a WebSocket and hand off
//!   to a `RoomDurableObject`, one per room code (`room_durable_object.ts`).
//!   `Room-Role` is set HERE, from which route matched -- never read from
//!   anything the client sent, so a client cannot self-declare "host".
//! - `/api/turn-credentials` mints short-TTL TURN credentials (`turn_credentials.ts`).

import { generateRoomCode, isValidRoomCode } from "./room_code.ts";
import {
  TURN_CREDENTIAL_TTL_SECONDS,
  fetchIceServers,
  isSameOrigin,
  type TurnKeyCredentials,
} from "./turn_credentials.ts";

export { RoomDurableObject } from "./room_durable_object.ts";

/** Bounded retries against a code collision (or a still-live prior room on that code). */
const MAX_CODE_ATTEMPTS = 5;

/** The client-IP key every rate limiter binding in this Worker keys on. */
function rateLimitKeyFor(request: Request): string {
  return request.headers.get("CF-Connecting-IP") ?? "unknown";
}

/** Exported for index.spec.ts -- AGENTS.md §4: everything a test touches is reachable. */
export async function handleHostSignal(request: Request, env: Env): Promise<Response> {
  // Checked BEFORE anything touches env.ROOM.getByName(): each attempt
  // below provisions or addresses a Durable Object, which is billed, and
  // room_state.ts's own per-room JOIN_RATE_LIMIT cannot help here -- it
  // only throttles repeated attempts against ONE already-known code, and
  // a fresh code (hence a fresh DO) is generated on every call to this
  // handler.
  const { success } = await env.SIGNAL_RATE_LIMITER.limit({ key: rateLimitKeyFor(request) });
  if (!success) {
    return new Response("too many requests", { status: 429 });
  }

  const forwarded = new Headers(request.headers);
  forwarded.set("Room-Role", "host");

  for (let attempt = 0; attempt < MAX_CODE_ATTEMPTS; attempt += 1) {
    const randomBytes = crypto.getRandomValues(new Uint8Array(6));
    const code = generateRoomCode(randomBytes);
    const stub = env.ROOM.getByName(code);
    // room_durable_object.ts's admission failures now complete the
    // WebSocket upgrade instead of returning an HTTP status this loop can
    // read (its own module doc, "Admission failures") -- so a collision
    // with an existing live room is no longer visible from the real
    // upgrade attempt's response. This non-upgrade probe (that module doc,
    // "Collision probe") is the replacement signal: cheap, and it never
    // opens (and immediately tears down) a real WebSocket for the losing
    // code.
    const probe = await stub.fetch(new Request(request.url));
    if (probe.status === 409) {
      continue; // This freshly generated code already names a live room --
      // astronomically unlikely (see room_code.ts's doc) but cheap to
      // retry rather than assume can't happen.
    }
    return stub.fetch(new Request(request.url, { headers: forwarded }));
  }
  return new Response("could not allocate a room code, try again", { status: 503 });
}

/** Exported for index.spec.ts -- AGENTS.md §4: everything a test touches is reachable. */
export async function handleJoinSignal(request: Request, env: Env): Promise<Response> {
  // Same reasoning as handleHostSignal: this addresses (and, via the DO's
  // own claim logic, may create the SQLite row for) a Durable Object
  // before any per-room check runs.
  const { success } = await env.SIGNAL_RATE_LIMITER.limit({ key: rateLimitKeyFor(request) });
  if (!success) {
    return new Response("too many requests", { status: 429 });
  }

  const url = new URL(request.url);
  const code = (url.searchParams.get("code") ?? "").toUpperCase();
  if (!isValidRoomCode(code)) {
    return new Response("invalid room code", { status: 400 });
  }
  const forwarded = new Headers(request.headers);
  forwarded.set("Room-Role", "guest");
  const stub = env.ROOM.getByName(code);
  return stub.fetch(new Request(request.url, { headers: forwarded }));
}

async function handleTurnCredentials(request: Request, env: Env): Promise<Response> {
  // Same-origin only -- this route mints credentials that cost money.
  if (!isSameOrigin(request.url, request.headers.get("Origin"))) {
    return new Response("forbidden", { status: 403 });
  }

  const { success } = await env.TURN_RATE_LIMITER.limit({ key: rateLimitKeyFor(request) });
  if (!success) {
    return new Response("too many requests", { status: 429 });
  }

  // Secrets unset (owner hasn't finished account setup, or local dev with
  // no .dev.vars) -- 404 so the client degrades to STUN-only. This exact
  // contract is depended on by the client-side issue in this milestone;
  // do not change it to a different status without updating that side too.
  if (!env.TURN_KEY_ID || !env.TURN_API_TOKEN) {
    return new Response(null, { status: 404 });
  }

  const credentials: TurnKeyCredentials = { keyId: env.TURN_KEY_ID, apiToken: env.TURN_API_TOKEN };
  const result = await fetchIceServers(credentials, TURN_CREDENTIAL_TTL_SECONDS, fetch);
  if (!result.ok) {
    // Never log credentials.keyId/apiToken; result.error is a fixed
    // stringly-typed code (turn_credentials.ts), never response content.
    console.error("turn-credentials: upstream call failed", result.error);
    return new Response("upstream TURN provider error", { status: 502 });
  }

  return Response.json(result.value, {
    headers: { "cache-control": "no-store" },
  });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === "/signal/host") {
      return handleHostSignal(request, env);
    }
    if (url.pathname === "/signal/join") {
      return handleJoinSignal(request, env);
    }
    if (url.pathname === "/api/turn-credentials" && request.method === "GET") {
      return handleTurnCredentials(request, env);
    }

    return new Response("not found", { status: 404 });
  },
};
