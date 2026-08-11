import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// Build config for the v2 render-benchmark harness (see web/render_bench.ts
// for what it measures). Mirrors
// v2/tools/browser_online_match/vite.config.ts's structure exactly per this
// task's brief ("mirror this structure rather than inventing a new one") --
// same rationale for aliasing `@gc/*` straight to package sources instead of
// pnpm workspace node_modules symlinks (this directory is intentionally
// outside the v2/ts pnpm workspace), same reason `@gc/wasm`'s browser
// artifact is imported by relative path from its already-built
// `dist/pkg-web` rather than aliased.
//
// This harness additionally needs `@gc/render`, which transitively needs
// `three` -- resolved via ordinary Node module resolution from
// `@gc/render`'s OWN real file location on disk
// (`v2/ts/packages/render/src/...`), which is inside the pnpm workspace and
// already has `three` installed as a direct dependency. Aliasing only
// redirects the SPECIFIER `@gc/render` to that file; resolution of THAT
// file's own bare imports still walks up from its real path, exactly the
// same mechanism the sibling harness relies on for `@gc/core`/`@gc/transport`.
const here = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  root: fileURLToPath(new URL("./web", import.meta.url)),
  resolve: {
    alias: {
      "@gc/render": fileURLToPath(new URL("../../ts/packages/render/src/index.ts", import.meta.url)),
      // This harness's own web/render_bench.ts also imports `three`
      // directly (to construct the `THREE.WebGLRenderer` `SceneRoot`
      // takes by injection -- see scene.ts's header on why it does not
      // build one itself). Unlike `@gc/render`'s own internal imports of
      // `three` (resolved fine via that package's real on-disk location,
      // which has `three` as a pnpm dependency), a file physically living
      // under v2/tools has no such node_modules chain of its own, since
      // this directory is intentionally outside the pnpm workspace (see
      // this file's header). Aliased straight to `@gc/render`'s resolved
      // `three` so both ends of this harness share the exact same
      // installed copy.
      three: fileURLToPath(new URL("../../ts/packages/render/node_modules/three", import.meta.url)),
    },
  },
  server: {
    fs: {
      allow: [here, fileURLToPath(new URL("../../ts", import.meta.url))],
    },
  },
  build: {
    target: "es2022",
    outDir: fileURLToPath(new URL("./dist", import.meta.url)),
    emptyOutDir: true,
  },
});
