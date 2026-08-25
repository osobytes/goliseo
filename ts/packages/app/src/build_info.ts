// `identity` names the namespace persisted data is saved under, corrected
// from a prototype-era name. The browser has no filesystem save-directory
// concept (this package's `settings.ts` header), but it does have an
// analogous one: the key namespace persisted data (e.g. `localStorage`) is
// saved under. `browser_main.ts`'s `localStorageSettings` uses this field
// for exactly that, so the "was the prototype name actually replaced"
// check has a single source of truth instead of a hand-typed "goliseo"
// string literal that could drift out of sync.

export interface BuildInfo {
  readonly name: string;
  readonly version: string;
  readonly channel: "development" | "release";
  readonly source_url?: string;
  /** See this file's header. */
  readonly identity: string;
  /**
   * A short, opaque identity distinguishing one deploy from another --
   * `online_ports.ts` folds this into the session manifest's `build_id`/
   * `source_id` so two tabs loaded from different deploys refuse to pair at
   * handshake (`gc_netcode::coordinator`'s existing, honest `build_mismatch`
   * reason) instead of silently pairing and desyncing mid-match. Without
   * this, every deployed build shared the one literal
   * `gc_netcode::protocol_fixture` hardcodes, so `build_mismatch` could
   * never actually fire across a deploy (#612).
   *
   * `vite_env.d.ts`'s own doc: `VITE_BUILD_SHA` is set by
   * `.github/workflows/deploy.yml` before `vite build`; every other build
   * (local dev, `vitest`, `ci.yml`'s gate) leaves it unset, and this falls
   * back to a fixed literal so two local tabs -- or two peers in the SAME
   * test/CI run -- still agree and can pair.
   */
  readonly build_sha: string;
}

export const buildInfo: BuildInfo = {
  name: "GOLISEO",
  version: "0.1.0-dev",
  channel: "development",
  identity: "goliseo",
  build_sha: import.meta.env.VITE_BUILD_SHA ?? "dev",
};
