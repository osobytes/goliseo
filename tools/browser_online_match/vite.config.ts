import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// Build config for the two-real-peer online match harness (see
// web/online_peer.ts for what it proves). Deliberately separate from
// v2/ts/vite.config.ts -- this task does not own that file, and the app
// shell it builds is a different, much larger thing than this narrow
// harness page.
//
// `web/` is the root (where index.html lives). `@gc/core`/`@gc/transport`
// are resolved straight to their package sources rather than through pnpm
// workspace node_modules symlinks -- this directory is intentionally
// outside the v2/ts pnpm workspace (v2/tools is a separate tree per this
// task's file ownership), so bare-specifier resolution would otherwise
// fail. `@gc/wasm`'s browser artifact is imported by relative path
// directly from its already-built `dist/pkg-web` (this task may read that
// package's build output but not its sources).
const here = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  root: fileURLToPath(new URL("./web", import.meta.url)),
  resolve: {
    alias: {
      "@gc/core": fileURLToPath(new URL("../../ts/packages/core/src/index.ts", import.meta.url)),
      "@gc/transport": fileURLToPath(
        new URL("../../ts/packages/transport/src/index.ts", import.meta.url),
      ),
    },
  },
  server: {
    fs: {
      // The dev server needs to read outside `web/`: the aliased
      // `@gc/core`/`@gc/transport` sources, and `@gc/wasm`'s built
      // `dist/pkg-web` artifact (its `.wasm` binary is resolved via `new
      // URL(..., import.meta.url)`, which Vite serves as a static asset).
      allow: [here, fileURLToPath(new URL("../../ts", import.meta.url))],
    },
  },
  build: {
    target: "es2022",
    outDir: fileURLToPath(new URL("./dist", import.meta.url)),
    emptyOutDir: true,
  },
});
