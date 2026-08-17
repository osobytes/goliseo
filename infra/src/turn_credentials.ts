//! `GET /api/turn-credentials`: short-TTL WebRTC TURN credentials, minted
//! by calling Cloudflare Realtime's TURN key API. The key id and API token
//! live in Worker secrets (`TURN_KEY_ID`, `TURN_API_TOKEN`) and never in
//! the repo -- when they are unset, the route degrades to a 404 (see
//! `index.ts`) rather than reaching this module at all.
//!
//! Split from `index.ts` so the request-shaping, response-validation, and
//! same-origin check are unit-testable without a real fetch or a Worker
//! runtime: `fetchIceServers` takes its `fetch` implementation as a
//! parameter and never reads a global.

import { type Result, err, ok } from "./result.ts";

/** One ICE server entry, matching the browser's `RTCIceServer` shape. */
export interface IceServer {
  readonly urls: string | readonly string[];
  readonly username?: string;
  readonly credential?: string;
}

/** The shape returned to the client: `{ iceServers: RTCIceServer[] }`. */
export interface TurnCredentialsResponse {
  readonly iceServers: readonly IceServer[];
}

/** Credentials are minted short-lived: long enough to establish a connection, no longer. */
export const TURN_CREDENTIAL_TTL_SECONDS = 600;

/** The TURN key identity, read from Worker secrets. Never logged, never committed. */
export interface TurnKeyCredentials {
  readonly keyId: string;
  readonly apiToken: string;
}

function isIceServer(value: unknown): value is IceServer {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const urls = (value as { urls?: unknown }).urls;
  const urlsOk =
    typeof urls === "string" || (Array.isArray(urls) && urls.every((u) => typeof u === "string"));
  if (!urlsOk) {
    return false;
  }
  const username = (value as { username?: unknown }).username;
  if (username !== undefined && typeof username !== "string") {
    return false;
  }
  const credential = (value as { credential?: unknown }).credential;
  if (credential !== undefined && typeof credential !== "string") {
    return false;
  }
  return true;
}

/** Structural validation of Cloudflare Realtime's response body. */
export function isTurnCredentialsResponse(value: unknown): value is TurnCredentialsResponse {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const iceServers = (value as { iceServers?: unknown }).iceServers;
  return Array.isArray(iceServers) && iceServers.every(isIceServer);
}

/**
 * Compare a request's `Origin` header against its own host. `null`/absent
 * `Origin` (same-origin navigations, curl, non-browser callers) is
 * rejected -- browsers always send `Origin` on a cross-origin fetch, and a
 * same-origin page load never hits this API route directly without one
 * either, so requiring it costs legitimate callers nothing.
 */
export function isSameOrigin(requestUrl: string, originHeader: string | null): boolean {
  if (originHeader === null) {
    return false;
  }
  try {
    return new URL(requestUrl).origin === new URL(originHeader).origin;
  } catch {
    return false;
  }
}

/**
 * Call Cloudflare Realtime's generate-ice-servers endpoint and validate the
 * shape of what comes back. `fetchImpl` is injected so tests never make a
 * real network call.
 */
export async function fetchIceServers(
  credentials: TurnKeyCredentials,
  ttlSeconds: number,
  fetchImpl: typeof fetch,
): Promise<Result<TurnCredentialsResponse>> {
  const url = `https://rtc.live.cloudflare.com/v1/turn/keys/${encodeURIComponent(credentials.keyId)}/credentials/generate-ice-servers`;

  let response: Response;
  try {
    response = await fetchImpl(url, {
      method: "POST",
      headers: {
        authorization: `Bearer ${credentials.apiToken}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ ttl: ttlSeconds }),
    });
  } catch {
    return err("turn_api_unreachable");
  }

  if (!response.ok) {
    return err(`turn_api_error_${response.status}`);
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    return err("turn_api_invalid_json");
  }

  if (!isTurnCredentialsResponse(body)) {
    return err("turn_api_unexpected_shape");
  }
  return ok(body);
}
