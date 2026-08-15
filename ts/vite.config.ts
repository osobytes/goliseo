import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { defineConfig, type Plugin } from "vite";

import { buildThirdPartyNotices } from "./scripts/generate_third_party_notices.ts";

// The browser app shell's dev/build config -- part of the "a running
// app" milestone. Root is this directory (where index.html lives); the
// entry module it loads is `packages/app/src/browser_main.ts`.
//
// No plugin is needed for the workspace's own `.ts` sources -- Vite's
// esbuild pipeline resolves explicit `.ts` extension relative imports
// (`rewriteRelativeImportExtensions`, ARCHITECTURE.md §4 rule 2) natively, and
// every `@gc/*` package resolves through pnpm's workspace symlinks the same
// way any other npm dependency would.
//
// `@gc/wasm`'s browser artifact (`packages/wasm/dist/pkg-web/gc_wasm.js`,
// built by `packages/wasm/scripts/build_web.mjs`) locates its `.wasm`
// binary via `new URL('gc_wasm_bg.wasm', import.meta.url)` -- a pattern
// Vite recognizes and bundles as a static asset automatically, in both
// `vite dev` and `vite build`, with no wasm-specific plugin required.
//
// The one plugin below (`thirdPartyNotices`) is build-only, for a different
// reason: Vite's minifier strips every `@license`/`/*!` comment, so nothing
// else in this config carries three.js's or the wasm-linked Rust crates'
// notices into the shipped bundle. See THIRD_PARTY.md and
// scripts/generate_third_party_notices.ts.

// Writes THIRD_PARTY_NOTICES.txt into the build output. `closeBundle` runs
// once, after every other asset is already written, so `config.build.outDir`
// is guaranteed to exist by the time this fires.
function thirdPartyNotices(): Plugin {
  let outDir = "dist-app";
  let root = import.meta.dirname;
  return {
    name: "third-party-notices",
    apply: "build",
    configResolved(config) {
      outDir = config.build.outDir;
      root = config.root;
    },
    closeBundle() {
      const targetDir = join(root, outDir);
      mkdirSync(targetDir, { recursive: true });
      const noticesPath = join(targetDir, "THIRD_PARTY_NOTICES.txt");
      writeFileSync(noticesPath, buildThirdPartyNotices());
      console.log(`[third-party-notices] wrote ${noticesPath}`);
    },
  };
}

export default defineConfig({
  plugins: [thirdPartyNotices()],
  server: {
    fs: {
      // Serve from the whole ts workspace (pnpm's workspace symlinks
      // point at sibling package directories, all under this root).
      allow: [import.meta.dirname],
    },
  },
  build: {
    target: "es2022",
    outDir: "dist-app",
  },
});
