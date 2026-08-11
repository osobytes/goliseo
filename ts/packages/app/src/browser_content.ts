// Placeholder production content for the browser app shell.
//
// The real content pipeline (`data/**` -- players, teams, formations,
// tactics, arenas; Rust-owned, v2/README.md rule 6.7) has no
// TypeScript-reachable export yet: `@gc/wasm`'s surface (`src/types.ts`)
// exposes match session lifecycle and the determinism/protocol surface, not
// a way to list authored teams or players. Building that marshalling layer
// is an asset/content pipeline, explicitly out of this milestone's scope
// (v2/README.md §1: "the glue that makes a playable browser build... asset
// pipeline... is a separate milestone").
//
// Until it exists, the app shell boots with the same authored Nebula-vs-
// Orion content this package's own specs already exercise
// (`test_support/fixtures.ts`, itself transcribed verbatim from
// `data/players.lua`/`data/teams.lua`/`data/formations.lua`/
// `data/tactics.lua`/`data/arenas.lua` -- see that file's header). Re-
// exported under this package's real (non-`test_support`) surface so
// `browser_main.ts` does not have to reach into test infrastructure by a
// relative path to boot.

export { APP_CONTENT } from "./test_support/fixtures.ts";
