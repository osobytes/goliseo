// Pure ICE-server configuration for the browser-star transport (#550):
// STUN defaults, merging a same-origin TURN-credentials response, and the
// force-relay toggle #248 asks for the "config path" of, off by default.
//
// Kept pure and headlessly testable, per this task's own instruction ("Keep
// the fetch in the impure app-shell layer; pure logic stays testable
// headlessly") -- the actual `fetch` call lives in `turn_credentials.ts`,
// and reading `window.location.search` lives in `browser_main.ts`. This
// module only combines already-resolved values.

/** `@gc/transport`'s `StarBridgeIceConfig` shape, restated here rather than
 * imported so this module stays a plain data/functions module with no
 * package dependency of its own -- the two are kept structurally identical
 * on purpose (`browser_main.ts` hands this straight to
 * `installGoliseoStarTransport`). */
export interface IceConfig {
  readonly iceServers: readonly RTCIceServer[];
  readonly iceTransportPolicy?: RTCIceTransportPolicy;
}

/** The issue's own defaults: Cloudflare's and Google's public STUN
 * servers. Always present, regardless of whether a TURN endpoint answers. */
export const DEFAULT_STUN_SERVERS: readonly RTCIceServer[] = [
  { urls: "stun:stun.cloudflare.com:3478" },
  { urls: "stun:stun.l.google.com:19302" },
];

/**
 * Merges a same-origin `/api/turn-credentials` response's `iceServers` with
 * the STUN defaults. `fetched` is `undefined` when the endpoint is absent,
 * errored, timed out, or returned something that failed to parse as
 * `{ iceServers: RTCIceServer[] }` -- `turn_credentials.ts` normalizes
 * every one of those cases to `undefined` before this ever runs, so this
 * function's own job is only the merge, never the failure classification
 * (that split is what keeps this half pure and the other half a single,
 * narrow `fetch` wrapper).
 */
export function mergeIceServers(
  defaults: readonly RTCIceServer[],
  fetched: readonly RTCIceServer[] | undefined,
): readonly RTCIceServer[] {
  return fetched === undefined || fetched.length === 0 ? defaults : [...defaults, ...fetched];
}

/**
 * Builds the full {@link IceConfig} for a browser-star peer connection.
 * `forceRelay` is #248's toggle: off (`false`) leaves `iceTransportPolicy`
 * unset entirely (the direct path is unaffected when unset, per that
 * issue's own acceptance criterion), never merely `"all"` -- an explicit
 * `"all"` and an absent key are NOT interchangeable to every
 * `RTCPeerConnection` implementation's own defaulting, so this only ever
 * emits the field when the caller actually asked for relay-only.
 */
export function newIceConfig(
  fetched: readonly RTCIceServer[] | undefined,
  forceRelay: boolean,
): IceConfig {
  return {
    iceServers: mergeIceServers(DEFAULT_STUN_SERVERS, fetched),
    ...(forceRelay ? { iceTransportPolicy: "relay" as const } : {}),
  };
}

/** The force-relay toggle, per #248: a `?ice=relay` query parameter, for
 * ad hoc real-device connectivity testing -- no settings-UI entry this
 * issue's scope calls for (#550 lands "the config path only"; RTT
 * measurement and a hosted TURN rollout are the rest of #248). Any other
 * value, or the parameter's absence, leaves the direct path unaffected. */
export function forceRelayFromSearch(search: string): boolean {
  return new URLSearchParams(search).get("ice") === "relay";
}
