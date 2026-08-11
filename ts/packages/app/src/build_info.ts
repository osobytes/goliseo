// Ported from game/build_info.lua.
//
// `identity` has no Lua original field -- it mirrors what
// `love.filesystem.setIdentity("goliseo")` (`conf.lua`, asserted by
// `spec/game/branding_spec.lua`'s "uses the corrected technical save
// identity") accomplishes on native LÖVE: naming the namespace persisted
// game data is saved under, corrected from a prototype-era name. The
// browser has no `love.filesystem` save-directory equivalent (this
// package's `settings.ts` header), but it does have an analogous concept --
// the key namespace persisted data (e.g. `localStorage`) is saved under.
// `browser_main.ts`'s `localStorageSettings` uses this field for exactly
// that, so the two "was the prototype name actually replaced" checks (LÖVE's
// save identity; the browser's storage namespace) stay a single source of
// truth instead of two hand-typed "goliseo" string literals silently
// drifting apart.

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
