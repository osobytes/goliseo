import { defineConfig } from "vite";

// The browser app shell's dev/build config -- part of the "a running
// app" milestone. Root is this directory (where index.html lives); the
// entry module it loads is `packages/app/src/browser_main.ts`.
//
// No plugins are needed for the workspace's own `.ts` sources -- Vite's
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
export default defineConfig({
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
