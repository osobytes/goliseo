// Same-origin TURN credential fetch (#550's ICE config path; #248's
// force-relay/hosted-TURN rollout is separate -- this only lands the
// contract both share). Never bundles a credential: this is the ONLY place
// a TURN credential is read, and it is read at runtime, from the page's own
// origin, never compiled into the shipped bundle.
//
// Degrades silently on any failure -- an absent endpoint (the game must
// keep working on a plain static host), a network error, a non-2xx
// response, or a body that is not the `{ iceServers: RTCIceServer[] }`
// shape -- because a TURN outage or a host with no such Worker configured
// must never block the online lobby, only narrow its NAT-traversal odds
// back to STUN-only. `ice_config.ts`'s `mergeIceServers`/`newIceConfig`
// treat `undefined` as exactly that: "no TURN servers available."
//
// This is the impure half of #550's ICE wiring, deliberately kept to one
// `fetch` call and one shape check -- everything else (the STUN defaults,
// the merge, the relay toggle) is `ice_config.ts`'s pure logic, testable
// with no network and no DOM.

const TURN_CREDENTIALS_PATH = "/api/turn-credentials";

function isRtcIceServer(value: unknown): value is RTCIceServer {
  if (typeof value !== "object" || value === null || !("urls" in value)) {
    return false;
  }
  const urls = (value as { readonly urls: unknown }).urls;
  return (
    typeof urls === "string" || (Array.isArray(urls) && urls.every((u) => typeof u === "string"))
  );
}

function isRtcIceServerArray(value: unknown): value is RTCIceServer[] {
  return Array.isArray(value) && value.every(isRtcIceServer);
}

/**
 * Fetches `/api/turn-credentials` (same-origin) and returns its
 * `iceServers`, or `undefined` on any failure -- see this file's header.
 * Never throws and never rejects.
 */
export async function fetchTurnCredentials(): Promise<readonly RTCIceServer[] | undefined> {
  try {
    const response = await fetch(TURN_CREDENTIALS_PATH, { credentials: "same-origin" });
    if (!response.ok) {
      return undefined;
    }
    const body: unknown = await response.json();
    if (typeof body !== "object" || body === null) {
      return undefined;
    }
    const iceServers = (body as { readonly iceServers?: unknown }).iceServers;
    return isRtcIceServerArray(iceServers) ? iceServers : undefined;
  } catch {
    return undefined;
  }
}
