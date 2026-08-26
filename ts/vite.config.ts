import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
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

// Dev-only sink for `@gc/app`'s `match_debug_log` (owner request,
// 2026-08-26): the running app POSTs one JSONL batch at a time to
// `/__match_debug?m=<match id>`, and this appends it under
// `ts/.match-debug/<match id>.jsonl` (gitignored) -- a plain on-disk record
// of what actually happened in a play-test match, readable after the fact.
// `apply: "serve"` keeps it out of `vite build` entirely; a production
// bundle has no sink, and the logger disables itself on the first failed
// POST anyway.
function matchDebugSink(): Plugin {
  return {
    name: "match-debug-sink",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__match_debug", (req: IncomingMessage, res: ServerResponse) => {
        if (req.method !== "POST") {
          res.statusCode = 405;
          res.end();
          return;
        }
        const url = new URL(req.url ?? "", "http://localhost");
        const id = (url.searchParams.get("m") ?? "").replace(/[^a-zA-Z0-9_-]/g, "") || "match";
        const chunks: Buffer[] = [];
        req.on("data", (chunk: Buffer) => chunks.push(chunk));
        req.on("end", () => {
          const dir = join(import.meta.dirname, ".match-debug");
          mkdirSync(dir, { recursive: true });
          appendFileSync(join(dir, `${id}.jsonl`), Buffer.concat(chunks));
          res.statusCode = 204;
          res.end();
        });
      });
    },
  };
}

export default defineConfig({
  plugins: [thirdPartyNotices(), matchDebugSink()],
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
