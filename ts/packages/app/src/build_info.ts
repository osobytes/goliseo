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
}

export const buildInfo: BuildInfo = {
  name: "GOLISEO",
  version: "0.1.0-dev",
  channel: "development",
  identity: "goliseo",
};
